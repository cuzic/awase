//! ADR-195 段階4: 予測器（`key_effect_predictor.rs`）の実行時読込。
//!
//! `awase-keymap-learn::persist`（段階3の永続化フォーマット）が書き出した学習済み表を、
//! コンパイル時埋め込みの同梱表（`key_effect_table.rs`）の代わりに使う。読み込んだ表は
//! `KeyEffectPredicted`（belief更新）に使う。actuationの判定には原則使わない（ユーザー明示config・
//! 漢字0x19の固定Toggleは不変）。**唯一の例外**: GJIの採用学習表が半角/全角を開閉トグルでないと示すとき
//! （ADR-195追記、`hankaku_zenkaku_non_toggle`）だけ、ADR-189の固定セットの`shadow_action=Toggle`を外す。
//! 本モジュールは`Cell`の一覧と、その前計算フラグを用意するだけで、actuationのどの合流点も呼ばない。
//!
//! 安全側に倒す2つの経路（本タスクB-1 Blockerの核心）:
//! 1. **破損ファイルへの縮退**: サイズ上限超過・パース失敗・スキーマ版不一致は、
//!    無条件で同梱表へフォールバック（`RuntimeTableCache::get`が`None`を返す）。
//! 2. **縮退率チェック**: 変換対象になりえたセル（VK・変換モードが表現可能なセルの
//!    検索キー数。閉状態のセルは変換モード違いを1つの枠に畳む）のうち、実際に使える
//!    セル（予測ありで、畳んでも矛盾しないもの）の割合が[`MIN_COVERAGE_RATIO`]未満なら不採用。
//!
//! 同梱表とのセル突き合わせ（旧・不一致率5%判定）は行わない（ADR-196決定1e:
//! 内蔵表を審査官にしない。突き合わせ・再測定は学習プロセスの仕事）。
//!
//! # 表現の粒度の制約（既知の限界、フォローアップが必要）
//!
//! `awase-keymap-learn::model::Status.composing`は`bool`（入力中か否か）だが、
//! `key_effect_predictor::Stage`は`Typing`/`ConvSpace`/`ConvHenkan`/`ConvMuhenkan`の
//! 4種の入力中サブ状態を区別する（ADR195-T5がMealy機械で置き換える対象そのもの、
//! 隠れ状態）。本モジュールは`composing == true`を一律`Stage::Typing`に写像する
//! （表に無い場合に「入力中」の行で代用する、既存の`predict_in_table`のフォールバック
//! 規則と同じ精度への意図的な劣化——新しい未検証の推測ではない）。これにより
//! `ConvSpace`/`ConvHenkan`/`ConvMuhenkan`固有のセルは学習表からは再現できず、
//! 常に同梱表側の値に頼る（学習表側にそのセルが無いのと同義）。段階5（Mealy機械最小化）
//! が隠れ状態を含む表現へ永続化スキーマを拡張すれば解消する見込み（[`awase_keymap_learn::persist::CURRENT_SCHEMA_VERSION`]の
//! 版上げが必要）。

use std::fs;
use std::path::Path;

use awase_keymap_learn::model::{Disposition, Outcome};
use awase_keymap_learn::persist::{self, Fingerprint, LoadError, PersistedCell, PersistedTable};
use awase_keymap_learn::staleness::{self, FingerprintProbe, Staleness};

use super::key_effect_predictor::{
    bundled_table, cell as make_cell, Cell, Conv, Disp, KeymapPreset, Stage, TableKey,
};
use super::key_effect_table::{toggle_contradiction, ToggleContradiction, NARROWABLE_KEYS};

/// 読み込むファイルの上限サイズ。壊れた/異常に巨大なファイルを丸ごとメモリに載せない
/// （B-1 Blockerの「ファイルサイズに上限を設ける」）。
pub const MAX_TABLE_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// 変換できた（＝実際に使える）セルの割合がこれ未満なら不採用（縮退率チェックの裏返し。
/// ADR本文の「縮退率20%」＝カバレッジ80%を暫定既定値とする）。分母は
/// [`coverage_slot_count`]（畳んだ後に変換対象になりえた検索キー数）。
pub const MIN_COVERAGE_RATIO: f64 = 0.80;

/// [`load_runtime_table`]が採用しなかった理由（ログ・診断用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RejectReason {
    /// ファイルが存在しない（未学習、正常系）。
    NotFound,
    /// ファイルは存在するが読み取りに失敗した（権限・排他ロック等、異常系）。
    Io,
    TooLarge,
    Parse,
    SchemaVersionMismatch,
    /// `(status, key)`の重複エントリがある（[`persist::LoadError::DuplicateCell`]）。
    DuplicateCell,
    /// 変換できたセルの割合が[`MIN_COVERAGE_RATIO`]未満。
    CoverageTooLow {
        coverage: f64,
    },
    /// 書き手（学習プロセス）の採否判定（[`awase_keymap_learn::judgement::TableJudgement`]）が
    /// `Accepted`でない（`Rejected`・`NeedsConfirmation`のいずれか）、または判定フィールド
    /// 自体が無い（`judgement: None`、1e前半以前に書かれたv2ファイル）（ADR196-T2決定1e、
    /// opus-adversarial-consult 2026-09-23 C-3: 判定を書いても読み手が読まなければ、
    /// 書いたのに効かない状態になる）。
    NotAccepted {
        judgement: Option<awase_keymap_learn::judgement::TableJudgement>,
    },
    /// 学習時点のキーマップ指紋が現在のキーマップと一致しない/確認できない（[`staleness::check`]が
    /// `Fresh`以外）。判定は書き手の採否判定のやり直しではなく、「表が測った構成と今の構成が
    /// 同じか」の照合（ADR-196決定1eの「判定をやり直さない」は正答率等の採否判定を指す）。
    Stale(Staleness),
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "ファイル未学習（存在しない）"),
            Self::Io => write!(f, "読み取り失敗"),
            Self::TooLarge => write!(f, "ファイルサイズが上限({MAX_TABLE_FILE_BYTES}バイト)超過"),
            Self::Parse => write!(f, "パース失敗"),
            Self::SchemaVersionMismatch => write!(f, "スキーマ版不一致"),
            Self::DuplicateCell => write!(f, "(status, key)の重複エントリ"),
            Self::CoverageTooLow { coverage } => {
                write!(f, "変換できたセルの割合が低すぎる(coverage={coverage:.2})")
            }
            Self::NotAccepted { judgement } => {
                write!(
                    f,
                    "書き手の採否判定がAcceptedでない(judgement={judgement:?})"
                )
            }
            Self::Stale(staleness) => write!(
                f,
                "学習時のキーマップ構成と現在の構成が一致しない(staleness={staleness:?})"
            ),
        }
    }
}

/// `PersistedCell`一覧を`key_effect_predictor::Cell`一覧へ変換する。表現できないセル
/// （VKが表に無い・変換モードが表現不能・`prediction: None`＝未測定または非決定と判定済み）は
/// 結果に含めない（「予測なし」に読み替える、B-1 Blockerの縮退経路）。
///
/// 閉セルは`conv: None`（ワイルドカード）へ変換されるため、閉状態の変換モードだけが違う
/// 複数セル（例: mode 0x09 と 0x00）が同じ検索キー`(stage, key)`に潰れる。潰れた先を
/// 反復順（`HashMap`由来の書き出し順）で先勝ちにすると採用セルが実行ごとに変わる（B-2）ため、
/// [`merge_closed_cells`]で入力順に依存しない形へ畳む。
pub(crate) fn convert_cells(cells: &[PersistedCell]) -> Vec<Cell> {
    let mut out: Vec<Cell> = Vec::new();
    // 検索キーごとの閉セル群（キー出現順は結果の並びにだけ影響し、内容には影響しない）。
    let mut closed_groups: Vec<Vec<Cell>> = Vec::new();
    for c in cells.iter().filter_map(convert_cell) {
        if c.open() {
            out.push(c);
        } else if let Some(g) = closed_groups
            .iter_mut()
            .find(|g| g[0].stage() == c.stage() && g[0].key() == c.key())
        {
            g.push(c);
        } else {
            closed_groups.push(vec![c]);
        }
    }
    out.extend(closed_groups.iter().filter_map(|g| merge_closed_cells(g)));
    out
}

/// 同じ`(stage, key)`の閉セル群を1セルへ畳む。開閉（`after_open`）と行方（`disp`）が全セルで
/// 一致するときだけ採用し、押下後の変換モードだけが食い違うなら`after_conv: None`
/// （「押下後の変換が不明」＝追跡を捨てる、既存の意味）にする。`after_open`/`disp`が食い違う
/// なら閉状態の隠れたモードに依存して予測できないので、セルごと落とす（「予測なし」）。
fn merge_closed_cells(group: &[Cell]) -> Option<Cell> {
    let first = group.first()?;
    if group
        .iter()
        .any(|c| c.after_open() != first.after_open() || c.disp() != first.disp())
    {
        return None;
    }
    let after_conv = if group.iter().all(|c| c.after_conv() == first.after_conv()) {
        first.after_conv()
    } else {
        None
    };
    Some(make_cell(
        false,
        None,
        first.stage(),
        first.key(),
        first.after_open(),
        after_conv,
        first.disp(),
    ))
}

/// セルの検索キー`(open, conv, stage, key)`。VKまたは（開状態の）変換モードが表に表現できない
/// セルは`None`（そのセルは、予測の有無にかかわらず変換対象になりえない）。
fn lookup_slot(pc: &PersistedCell) -> Option<(bool, Option<Conv>, Stage, TableKey)> {
    let key = TableKey::from_vk(pc.key.0)?;
    // 開状態のセルだけ変換モードを持つ（閉セルは`conv: None`がワイルドカード、Cellの既存の約束）。
    // 開状態で変換モードが表現できないときはセルごと除外する（`open`かつ変換不能は無効な組み合わせ）。
    let conv = if pc.status.open {
        Some(Conv::from_raw(u32::from(pc.status.mode))?)
    } else {
        None
    };
    let stage = if pc.status.composing {
        Stage::Typing
    } else {
        Stage::None
    };
    Some((pc.status.open, conv, stage, key))
}

fn convert_cell(pc: &PersistedCell) -> Option<Cell> {
    let (open, conv, stage, key) = lookup_slot(pc)?;
    let outcome: Outcome = pc.prediction?;
    let after_open = outcome.status.open;
    let after_conv = if after_open {
        // 変換不能なモードへ遷移した場合は「押下後の変換が不明」として追跡を捨てる
        // （既存の`predict_in_table`が`after_conv: None`を「開く/閉じる遷移で不明」として扱うのと同じ扱い）。
        Conv::from_raw(u32::from(outcome.status.mode))
    } else {
        None
    };
    let disp = match outcome.disp {
        Disposition::None => Disp::None,
        Disposition::Kept => Disp::Kept,
        Disposition::Discarded => Disp::Discarded,
        Disposition::Committed => Disp::Committed,
    };
    Some(make_cell(
        open, conv, stage, key, after_open, after_conv, disp,
    ))
}

/// 縮退率チェックの分母: 畳んだ後に変換対象になりえたセル数（検索キーの異なり数）。
///
/// 生セル数を分母にすると、閉状態の変換モード違いのセル（[`merge_closed_cells`]で1枠に畳まれる）や、
/// 変換モードを`Conv`で表せない開状態のセル（MS-IME本体など、そもそも表に入らない）まで
/// 「変換できなかった」と数えてしまい、学習が成功していてもカバレッジが基準を下回る
/// （実機CI実測: GJI+ATOK 0.782、MS-IME本体 0.52〜0.53。いずれも`mismatch`は0.000）。
/// 分母に数えないのは変換モード側の表現限界だけ。次は枠に数え、縮退の証拠としてカバレッジを下げる:
/// - 予測なし（`prediction: None`＝非決定と判定済み・未測定）のセル（学習側が決められなかった）。
/// - 表に無いVKのセル（生の表が想定外のキーを含む＝破損・別物の疑い。セルごとに1枠）。
fn coverage_slot_count(raw: &[PersistedCell]) -> usize {
    let mut slots: Vec<(Stage, TableKey)> = Vec::new();
    let mut open_slots: Vec<(Conv, Stage, TableKey)> = Vec::new();
    let unknown_vk = raw
        .iter()
        .filter(|pc| TableKey::from_vk(pc.key.0).is_none())
        .count();
    for (open, conv, stage, key) in raw.iter().filter_map(lookup_slot) {
        match (open, conv) {
            (true, Some(conv)) => {
                if !open_slots.contains(&(conv, stage, key)) {
                    open_slots.push((conv, stage, key));
                }
            }
            _ => {
                // 閉状態は変換モード違いを1枠に畳む（`convert_cells`の畳み込みと同じ単位）。
                if !slots.contains(&(stage, key)) {
                    slots.push((stage, key));
                }
            }
        }
    }
    slots.len() + open_slots.len() + unknown_vk
}

/// 変換できたセルの割合（B-1 Blockerの縮退率チェック）。分母は[`coverage_slot_count`]。
fn coverage_ratio(raw: &[PersistedCell], converted_len: usize) -> f64 {
    let slots = coverage_slot_count(raw);
    if slots == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = converted_len as f64 / slots as f64;
    ratio
}

/// `(status, key)`ペアで不一致セルを識別する（[`awase_keymap_learn::model::Status`]/
/// [`awase_keymap_learn::model::KeyId`]をそのまま使い、`PersistedCell`と同じ識別方式にする）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MismatchedCell {
    pub status: awase_keymap_learn::model::Status,
    pub key: awase_keymap_learn::model::KeyId,
}

/// 学習表と同梱表の第一段階の突き合わせ結果（[ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md)決定1b項目7〜9）。
///
/// 再測定（実際にIMEを再度叩いて元の学習値が再現するか確認する）は含まない
/// ——ここでの`mismatched`は「再測定が必要な候補」であり、呼び出し側（学習プロセス）が
/// 再測定した結果を[`awase_keymap_learn::judgement::CellReconciliation`]で確定させる。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundledDiff {
    /// 両方の表にあり、結果も一致したセル数。
    pub matched: u32,
    /// 両方の表にあるが結果が食い違うセル（再測定対象、順序は学習表側の入力順）。
    pub mismatched: Vec<MismatchedCell>,
    /// 学習表・同梱表の片方にしか無いセル数（突き合わせの分母に含めない）。
    pub only_in_one_table: u32,
}

/// 学習表を同梱表と突き合わせる（[ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md)決定1e第1項:「学習プロセスが内蔵表を参照できるようにする」の実装本体）。
///
/// `preset`は[`super::key_effect_predictor::KeyEffectKeymap::is_unmodified_bundled_config`]相当の判定
/// （呼び出し側の[`awase_gji_config::known_keymap`]等）で「既知構成である」と確認できた場合のみ
/// 渡すこと——カスタム構成では突き合わせに意味が無い（`mismatch_ratio`と同じ前提）。
///
/// 変換できない学習セル（`convert_cell`が`None`を返すもの: VK非対応・変換モード非表現・
/// `prediction: None`）は、同梱表側から見て「学習表に無い」＝`only_in_one_table`に数える
/// （このセルは学習表からは何も主張していないので、突き合わせの母数からは除きつつ、
/// 「表にのみ存在」の記録には残す）。
#[must_use]
pub fn diff_against_bundled(persisted: &[PersistedCell], preset: KeymapPreset) -> BundledDiff {
    diff_against_bundled_cells(persisted, bundled_table(preset))
}

/// 診断用: `pc`と同じ`(open, conv, stage, key)`の同梱表セルの結果を文字列にする
/// （不一致セルで、同梱表側が何と言っているかをログに残すため。突き合わせ判定には使わない）。
/// 変換できないセル・同梱表に無いセルは`None`。
#[must_use]
pub fn describe_bundled_cell(pc: &PersistedCell, preset: KeymapPreset) -> Option<String> {
    let converted = convert_cell(pc)?;
    let b = bundled_table(preset).iter().find(|b| {
        b.matches_lookup_key(
            converted.open(),
            converted.conv(),
            converted.stage(),
            converted.key(),
        )
    })?;
    Some(format!(
        "after_open={:?} after_conv={:?} disp={:?}",
        b.after_open(),
        b.after_conv(),
        b.disp()
    ))
}

/// 押下後の変換モードが**矛盾**するか。同梱表側の`None`は「不明（追跡を捨てた）」であって
/// 「変換モードが変わらない」等の主張ではないため、同梱表が`None`なら矛盾とみなさない
/// （実測: GJI+ATOKで閉→開のセルは、同梱表が`after_conv: None`、学習側が実測モード`Some(x)`を
/// 持ち、旧・単純な`==`比較だと10セルが偽の不一致になった）。
///
/// 逆向き（学習側が`None`で同梱表が`Some`）は矛盾として数え続ける: 学習表が同梱表より
/// 情報を落としている（変換モードの追跡が途切れる）ことを、採否ゲートが検出できなくなるため
/// （code-review 2026-09-24 指摘）。
fn after_conv_conflicts(learned: Option<Conv>, bundled: Option<Conv>) -> bool {
    match (learned, bundled) {
        (_, None) => false,
        (Some(l), Some(b)) => l != b,
        (None, Some(_)) => true,
    }
}

/// [`diff_against_bundled`]の本体。テストで同梱表全体ではなく小さな合成`Cell`列を渡せるように
/// 分離している（`mismatch_ratio`と同じ理由）。
fn diff_against_bundled_cells(persisted: &[PersistedCell], bundled: &[Cell]) -> BundledDiff {
    let mut matched_bundled = vec![false; bundled.len()];
    let mut diff = BundledDiff::default();

    for pc in persisted {
        let Some(converted) = convert_cell(pc) else {
            diff.only_in_one_table += 1;
            continue;
        };
        let found = bundled.iter().enumerate().find(|(_, b)| {
            b.matches_lookup_key(
                converted.open(),
                converted.conv(),
                converted.stage(),
                converted.key(),
            )
        });
        match found {
            Some((idx, b)) => {
                matched_bundled[idx] = true;
                if converted.after_open() == b.after_open()
                    && !after_conv_conflicts(converted.after_conv(), b.after_conv())
                    && converted.disp() == b.disp()
                {
                    diff.matched += 1;
                } else {
                    diff.mismatched.push(MismatchedCell {
                        status: pc.status,
                        key: pc.key,
                    });
                }
            }
            None => diff.only_in_one_table += 1,
        }
    }
    diff.only_in_one_table +=
        u32::try_from(matched_bundled.iter().filter(|m| !**m).count()).unwrap_or(u32::MAX);

    diff
}

/// `<config dir>/keymap-learn-table.json`のパス。`config dir`は`config.toml`の親ディレクトリ
/// （`crate::app::find_config_path()`と同じ解決順、見つからなければ`None`＝未学習として扱う）。
///
/// `crate::app`（実行ファイルの位置に基づく解決）は`#[cfg(windows)]`のため、この関数もそれに合わせる
/// （`state/`は原則OS非依存だが、このファイルパス解決だけはWindows固有の起動時パス規則に依存する）。
#[cfg(windows)]
pub(crate) fn table_file_path() -> Option<std::path::PathBuf> {
    let config_path = crate::app::find_config_path().ok()?;
    Some(config_path.parent()?.join("keymap-learn-table.json"))
}

/// 学習プロセスが不採用/要確認の結果を退避する`keymap-learn-last-attempt.json`のパス
/// （`awase-keymap-learn-win`の`--last-attempt-path`既定と同じ場所）。
#[cfg(windows)]
pub(crate) fn last_attempt_file_path() -> Option<std::path::PathBuf> {
    let config_path = crate::app::find_config_path().ok()?;
    Some(config_path.parent()?.join("keymap-learn-last-attempt.json"))
}

/// [`RuntimeTableCache::get`]の`stamp`引数（更新時刻+長さ）。ファイルが無い/読めなければ`None`
/// （`KeymapCache`の「GJI未導入」と同じ規則: 版が変わらない限り読み直さない）。
#[cfg(windows)]
pub(crate) fn table_file_stamp() -> Option<(u64, u64)> {
    let path = table_file_path()?;
    let meta = fs::metadata(&path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((mtime, meta.len()))
}

/// [`RuntimeTableCache::get`]の`load`引数。採用できなければ理由をログに残して`None`を返す
/// （呼び出し側は同梱表へフォールバックする）。
#[cfg(windows)]
pub(crate) fn load_and_log(fingerprint: Fingerprint) -> Option<Vec<Cell>> {
    let path = table_file_path()?;
    match load_runtime_table(&path, FingerprintProbe::Computed(fingerprint)) {
        Ok(cells) => {
            tracing::info!(
                "[key-effect-runtime] 学習済み表を採用: {} セル (path={})",
                cells.len(),
                path.display()
            );
            Some(cells)
        }
        Err(RejectReason::NotFound) => {
            // ファイル未学習（存在しない）は正常系、ログしない。
            None
        }
        Err(reason) => {
            tracing::warn!(
                "[key-effect-runtime] 学習済み表を不採用、同梱表へフォールバック: {reason} (path={})",
                path.display()
            );
            None
        }
    }
}

/// 学習プロセスが使う「今のキーマップの指紋」。
///
/// `awase-keymap-learn-win`が、学習時点の指紋を書き込む/再検証で照合するために使う。awase.exeの読込（`kp_predict_key_effect`）が
/// 使う指紋と同じ関数（`KeyEffectKeymap::fingerprint`）から作るので、書き手と読み手で
/// 計算方式がずれない。GJI/Microsoft IME本体以外（`Other`）は指紋方式が無い（`NotSupported`、
/// 実行時もその構成では予測しない）。GJIで`config1.db`が読めない/解析できないときは`Unavailable`。
#[cfg(windows)]
#[must_use]
pub fn current_fingerprint_probe(tip: super::ime_kind::TipIdentity) -> FingerprintProbe {
    use super::ime_kind::TipIdentity;
    match tip {
        TipIdentity::Gji => crate::gji_charset_autodetect::read_key_effect_keymap()
            .map_or(FingerprintProbe::Unavailable, |k| {
                FingerprintProbe::Computed(k.fingerprint())
            }),
        TipIdentity::MsImeNative => FingerprintProbe::Computed(
            crate::msime_key_assignment::read_key_effect_keymap_native().fingerprint(),
        ),
        TipIdentity::Other => FingerprintProbe::NotSupported,
    }
}

/// `std::io::Error`を、ファイル不在(正常系)とそれ以外の読み取り失敗(異常系、ログすべき)
/// とを区別できる[`RejectReason`]へ変換する。
fn io_reject_reason(e: &std::io::Error) -> RejectReason {
    if e.kind() == std::io::ErrorKind::NotFound {
        RejectReason::NotFound
    } else {
        RejectReason::Io
    }
}

/// ファイルを読み、パース・スキーマ検証・採否判定・指紋照合・縮退率チェックまで行う
/// （同梱表との突き合わせはしない。ADR-196決定1e）。
///
/// # Errors
/// 採用できない理由を[`RejectReason`]で返す。
pub fn load_runtime_table(
    path: &Path,
    current_fingerprint: FingerprintProbe,
) -> Result<Vec<Cell>, RejectReason> {
    let table = read_persisted_table(path)?;
    validate_and_convert(&table, current_fingerprint)
}

/// ファイルを読んで`PersistedTable`へパースするところまで（採否判定・変換はしない）。
/// [`load_runtime_table`]と不具合報告（`bug_report::BugReportKeymapLearnSummary`）が共有する。
///
/// # Errors
/// 読めなかった/パースできなかった理由を[`RejectReason`]で返す（採否判定由来の理由は返さない）。
pub fn read_persisted_table(path: &Path) -> Result<PersistedTable, RejectReason> {
    let meta = fs::metadata(path).map_err(|e| io_reject_reason(&e))?;
    if meta.len() > MAX_TABLE_FILE_BYTES {
        return Err(RejectReason::TooLarge);
    }
    // 0バイトは未学習と同じ扱い（scoopのpersistは、まだ存在しない永続化対象のファイルを
    // 空ファイルとして作ることがあり、壊れたファイル扱い＝パース失敗の警告にしないため）。
    // （Windowsではディレクトリの`len`も0なので、通常ファイルに限る。ディレクトリは読み取り失敗のまま）
    if meta.is_file() && meta.len() == 0 {
        return Err(RejectReason::NotFound);
    }
    let text = fs::read_to_string(path).map_err(|e| io_reject_reason(&e))?;
    persist::from_json(&text).map_err(|e| match e {
        LoadError::Parse(_) => RejectReason::Parse,
        LoadError::SchemaVersionMismatch { .. } => RejectReason::SchemaVersionMismatch,
        LoadError::DuplicateCell { .. } => RejectReason::DuplicateCell,
    })
}

/// [`load_runtime_table`]のfsを伴わない部分（テスト・CI検証双方から呼べるように分離）。
///
/// # Errors
/// 採用できない理由を[`RejectReason`]で返す。
pub fn validate_and_convert(
    table: &PersistedTable,
    current_fingerprint: FingerprintProbe,
) -> Result<Vec<Cell>, RejectReason> {
    if table.judgement != Some(awase_keymap_learn::judgement::TableJudgement::Accepted) {
        return Err(RejectReason::NotAccepted {
            judgement: table.judgement,
        });
    }
    // 学習時点のキーマップ指紋と現在の指紋を照合する（ADR-195段階8）。指紋を持たない表
    // （指紋配線前に書かれたもの）は`check`が`Fresh`にする＝従来どおり保護しない。
    let staleness = staleness::check(table, current_fingerprint);
    if staleness.is_stale() {
        return Err(RejectReason::Stale(staleness));
    }
    let converted = convert_cells(&table.cells);
    let coverage = coverage_ratio(&table.cells, converted.len());
    if coverage < MIN_COVERAGE_RATIO {
        return Err(RejectReason::CoverageTooLow { coverage });
    }
    Ok(converted)
}

/// 学習表由来で`shadow_action=Toggle`を外す（受動に狭める）か（ADR-195追記・ADR-199決定6-2/6-4）の、
/// 分岐部分の純関数。
///
/// `use_learned_keymap_table`（opt-out）と表の前計算結果（[`RuntimeTableCache::toggle_contradiction`]）の
/// 両方が真のときだけ外す。学習表が無い・棄却・未採用・opt-out は常に`false`（呼び出し側の役割を維持）。
/// IME種別による限定はしない: 学習表があるのは GJI と MS-IME 本体だけで、どちらも狭めは同様に適用する
/// （決定6-4。呼び出し側は`table_ime_kind()`が`None`の窓を先に除く）。
#[must_use]
pub const fn hz_omit_verdict(use_learned: bool, table_flag: bool) -> bool {
    hz_omit_may_apply(use_learned) && table_flag
}

/// [`hz_omit_verdict`]の前段（表を読む前に分かる条件）。偽なら表の同期読込を省ける。
#[must_use]
pub const fn hz_omit_may_apply(use_learned: bool) -> bool {
    use_learned
}

/// `enrich_key_role`が判定に届く前に早期returnする（修飾付き・IME種別不明）とき、古いラッチを
/// 捨てるべきか。候補キー（`is_candidate`）の非injected KeyDownだけ捨てる（捨てないと次のKeyUpが前回押下の
/// 判定を使い、Down=Allow・Up=Suppress の非対称になる。Opus round2 N2）。
#[must_use]
pub const fn should_clear_latch_on_early_return(
    is_candidate: bool,
    is_key_down: bool,
    injected: bool,
) -> bool {
    is_candidate && is_key_down && !injected
}

/// 候補キーの「この打鍵の最終的な`shadow_action`」のラッチ（`(scan_code, 判定)`）を進める純関数
/// （ADR-195追記の半角/全角ラッチを、ADR-199決定18(i)で候補キー全体の打鍵ごとのラッチに一般化した。
/// 値の意味はキーの種類で変えない: 判定は常に「付ける`shadow_action`（付けないなら`None`）」）。
///
/// - `reuse`（KeyUp、またはオートリピートの`was_down`なKeyDown）で、ラッチの scan_code（非0）が一致すれば
///   ラッチの判定を使う（`fresh`は呼ばない）。
/// - それ以外は`fresh`で判定を求める。非injectedのKeyDown（`fresh_down`）はその結果でラッチを**上書き**する。
///   KeyUp では消さない（ドレイン経路・救済窓の再入で同じ打鍵が2回 enrich されても Down→Up→Down→Up で
///   同じ結果になる）。
/// - 識別は vk でなく scan_code（`VK_DBE_*`はDown/Upでvkが変わりうる、BUG-131/132）。拡張ビットは照合しない:
///   半角/全角（0xF3/0xF4）・F13〜F24には、Left/Right Alt のような raw scan 同一で拡張ビットだけが違う双子キーが無い。
/// - `fresh`は表の同期読込（`RuntimeTableCache::get`）を含みうるので、`enrich_key_role`内で同期I/Oが走りうる
///   （スタンプ変化時のみ）。
///
/// 戻り値は`(判定, 新しいラッチ)`。
#[must_use]
pub fn latch_step<T: Copy>(
    latch: Option<(awase::types::ScanCode, T)>,
    reuse: bool,
    fresh_down: bool,
    scan: awase::types::ScanCode,
    fresh: impl FnOnce() -> T,
) -> (T, Option<(awase::types::ScanCode, T)>) {
    if reuse {
        if let Some((s, verdict)) = latch {
            if scan.0 != 0 && s == scan {
                return (verdict, latch);
            }
        }
    }
    let verdict = fresh();
    (
        verdict,
        if fresh_down {
            Some((scan, verdict))
        } else {
            latch
        },
    )
}

/// `config1.db`スタンプ（[`super::key_effect_predictor::KeymapCache`]）と同じ方式のfsキャッシュ。
/// `RECHECK_MS`ごとにファイルの版（更新時刻+長さ）だけを問い合わせ、変わったときだけ読み直す。
/// 判定は純関数で、fs/時計は呼び出し側が渡す（テスト容易性のため`KeymapCache`と同じ形にする）。
///
/// ファイル自身のスタンプに加えて`(KeymapPreset, check_against_bundled, キーマップ指紋)`も版の一部として
/// 比較する——学習済み表ファイル自体は変わっていなくても、GJIのプリセット切替
/// （`session_keymap`）やカスタム構成の有無が変わると、以前キャッシュしたセルは
/// 新しい構成に対して未検証のまま（かつVK/モードの意味が構成ごとに違いうる）になる。
/// 採否判定に効くのは指紋（`staleness::check`）だけだが、`preset`/`check_against_bundled`は
/// 不具合報告の同梱表突き合わせ診断（[`Self::last_validation_key`]）が「予測時に実際に使った値」
/// を報告するために、変化のたびに読み直して最新に保つ。
#[derive(Debug, Default)]
pub struct RuntimeTableCache {
    checked_at_ms: Option<u64>,
    stamp: Option<(u64, u64, KeymapPreset, bool, Fingerprint)>,
    cells: Option<Vec<Cell>>,
    /// `cells`から読込時に前計算した、[`NARROWABLE_KEYS`]ごとの「トグルと矛盾する」判定
    /// （[`toggle_contradiction`]、ADR-199決定6-2）。`cells`と同じ場所で更新するので別々に古くならない。
    contradictions: [Option<ToggleContradiction>; NARROWABLE_KEYS.len()],
}

impl RuntimeTableCache {
    /// 採用中の学習表が`key`を開閉トグルと矛盾すると示しているか（読込時に前計算）。
    /// `key`が[`NARROWABLE_KEYS`]に無い・学習表が無い・棄却・未採用なら`None`。
    /// **`use_learned_keymap_table`は見ない**ので、呼び出し側が`get`を呼んだ直後にだけ使うこと。
    #[must_use]
    pub fn toggle_contradiction(&self, key: TableKey) -> Option<ToggleContradiction> {
        let i = NARROWABLE_KEYS.iter().position(|k| *k == key)?;
        self.contradictions[i]
    }

    /// [`Self::get`]を、予測器・警告・(B)判定で共通の検証キー（`preset`・同梱表そのままか・指紋）で呼ぶ。
    #[cfg(windows)]
    pub fn get_for_keymap(
        &mut self,
        now_ms: u64,
        keymap: &super::key_effect_predictor::KeyEffectKeymap,
    ) -> Option<&[Cell]> {
        let fingerprint = keymap.fingerprint();
        self.get(
            now_ms,
            (
                keymap.preset(),
                keymap.is_unmodified_bundled_config(),
                fingerprint,
            ),
            table_file_stamp,
            || load_and_log(fingerprint),
        )
    }

    /// 直近の`get`で学習済み表が採用されている（＝予測に使われている）か。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.cells.is_some()
    }

    /// 直近の読込で使った`(preset, check_against_bundled)`（不具合報告の診断用で、採否判定には
    /// 使わない。学習表ファイルが無い/読めなかった場合は`None`）。
    #[must_use]
    pub fn last_validation_key(&self) -> Option<(KeymapPreset, bool)> {
        self.stamp.map(|(_, _, preset, check, _)| (preset, check))
    }

    pub const RECHECK_MS: u64 = super::key_effect_predictor::KeymapCache::RECHECK_MS;

    /// キャッシュした学習済み表を返す（採用できなかった/未学習なら`None`＝呼び出し側は同梱表を使う）。
    ///
    /// `validation_key`は`(preset, check_against_bundled, 現在のキーマップ指紋)`——呼び出し側が
    /// `load`に渡すのと同じ値を渡すこと。指紋を含めるのは、GJIのカスタムキーマップ/overlayや
    /// MS-IME本体の再割り当てが変わっても`preset`・ファイルスタンプが同じままだと、読み直し
    /// すら起きず`staleness::check`が二度と呼ばれないため（版の一部として比較され、変わればファイルスタンプが同じでも読み直す）。
    pub fn get(
        &mut self,
        now_ms: u64,
        validation_key: (KeymapPreset, bool, Fingerprint),
        stamp: impl FnOnce() -> Option<(u64, u64)>,
        load: impl FnOnce() -> Option<Vec<Cell>>,
    ) -> Option<&[Cell]> {
        let first = self.checked_at_ms.is_none();
        let due = self
            .checked_at_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= Self::RECHECK_MS);
        // code-review指摘: validation_key(preset/指紋)の変化は、
        // RECHECK_MS(fsアクセスの間引き)とは独立に毎回チェックする。プリセット切替は
        // フォーカス移動というユーザー操作でRECHECK_MSの窓の途中でも起こりうり、
        // dueがfalseのまま素通りすると古いプリセット向けに検証済みのセルを
        // 新しいプリセットの予測にそのまま使い続けてしまう(セルの意味がプリセットごとに
        // 違いうるため、これは黙って誤った予測を返す事故になる)。
        let validation_key_changed = !first
            && self
                .stamp
                .is_some_and(|(_, _, preset, check, fp)| (preset, check, fp) != validation_key);
        if due || validation_key_changed {
            self.checked_at_ms = Some(now_ms);
            let now_stamp = stamp().map(|(mtime, len)| {
                (
                    mtime,
                    len,
                    validation_key.0,
                    validation_key.1,
                    validation_key.2,
                )
            });
            if first || now_stamp != self.stamp {
                self.stamp = now_stamp;
                self.cells = load();
                self.contradictions = NARROWABLE_KEYS.map(|key| {
                    self.cells
                        .as_deref()
                        .and_then(|c| toggle_contradiction(c, key))
                });
                // 食い違いの記録（決定6-2）。読込（再計算）時に1回だけ。実際に受動へ狭めるのは
                // 呼び出し側（enrich）で、そのキーが役割を持つときだけ。
                for (key, found) in NARROWABLE_KEYS.iter().zip(self.contradictions) {
                    if let Some(found) = found {
                        tracing::warn!(
                            "[learned-narrow] 採用中の学習表が{key:?}を開閉トグルでないと示す（{}）。役割を持つ場合は受動へ狭める",
                            found.label()
                        );
                    }
                }
            }
        }
        self.cells.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 指紋を持たない表（旧形式）向けの「現在の指紋」: 比較対象が無いので照合しない。
    const NS: FingerprintProbe = FingerprintProbe::NotSupported;
    const FP: Fingerprint = Fingerprint(1, 2);
    use super::super::key_effect_predictor::KeyEffectKeymap;
    use awase_keymap_learn::judgement::TableJudgement;
    use awase_keymap_learn::model::{KeyId, Status};

    fn pcell(
        open: bool,
        mode: u8,
        composing: bool,
        key: u16,
        pred: Option<(bool, u8)>,
    ) -> PersistedCell {
        PersistedCell {
            status: Status {
                open,
                mode,
                composing,
            },
            key: KeyId(key),
            prediction: pred.map(|(o, m)| Outcome {
                status: Status {
                    open: o,
                    mode: m,
                    composing: false,
                },
                disp: Disposition::Kept,
            }),
        }
    }

    #[test]
    fn converts_a_simple_open_to_close_cell() {
        // ひらがな(0xF2)開→英数(0x00)相当、閉じないケース: open=true mode=0x09(ひらがな) -> after open=true mode=0x00(英数)。
        let pc = pcell(true, 0x09, false, 0xF2, Some((true, 0x00)));
        let c = convert_cell(&pc).expect("変換できるはず");
        assert!(c.open());
        assert_eq!(c.conv(), Some(Conv::C19));
        assert_eq!(c.stage(), Stage::None);
        assert_eq!(c.key(), TableKey::Hiragana);
        assert!(c.after_open());
        assert_eq!(c.after_conv(), Some(Conv::C10));
    }

    #[test]
    fn skips_cells_with_no_prediction() {
        let pc = pcell(true, 0x09, false, 0xF2, None);
        assert!(
            convert_cell(&pc).is_none(),
            "未測定/非決定は予測なしに読み替える"
        );
    }

    #[test]
    fn skips_cells_with_unrepresentable_vk_or_mode() {
        // 表に無いVK(適当な値0x99)。
        let unknown_vk = pcell(true, 0x09, false, 0x99, Some((true, 0x00)));
        assert!(convert_cell(&unknown_vk).is_none());
        // 表現できない変換モード(0x03=半角カタカナ、到達不能)。
        let unknown_mode = pcell(true, 0x03, false, 0xF2, Some((true, 0x00)));
        assert!(convert_cell(&unknown_mode).is_none());
    }

    #[test]
    fn composing_true_maps_to_typing_stage() {
        let pc = pcell(true, 0x09, true, 0x0D, Some((true, 0x09)));
        let c = convert_cell(&pc).expect("変換できるはず");
        assert_eq!(
            c.stage(),
            Stage::Typing,
            "入力中サブ状態は一律Typingへ縮退する"
        );
    }

    #[test]
    fn coverage_too_low_is_rejected() {
        // 10セット中2セットだけ予測あり(20%) < MIN_COVERAGE_RATIO(80%)。
        let mut cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x00)))];
        cells.push(pcell(true, 0x00, false, 0xF2, Some((true, 0x09))));
        for _ in 0..8 {
            cells.push(pcell(true, 0x09, false, 0x99, None)); // 表に無いVK: 常に変換不能
        }
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        let err = validate_and_convert(&table, NS).unwrap_err();
        assert!(matches!(err, RejectReason::CoverageTooLow { .. }));
    }

    #[test]
    fn high_coverage_without_bundled_check_is_accepted() {
        let cells: Vec<_> = (0..10)
            .map(|_| pcell(true, 0x09, false, 0xF2, Some((true, 0x00))))
            .collect();
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        let out = validate_and_convert(&table, NS).unwrap();
        assert_eq!(out.len(), 10);
    }

    /// ADR-196決定1e: 内蔵表を審査官にしない。学習表が同梱表と食い違っていても（学習の目的そのもの）、
    /// 採否判定・指紋・カバレッジを通っていれば採用する。
    #[test]
    fn table_disagreeing_with_bundled_is_adopted() {
        // 同梱ATOK表では、ひらがな(0xF2)を開いた状態で押した結果は「閉じない」実測。
        // ここではわざと「閉じる」という学習結果を大量に混ぜても、突き合わせで棄却しない。
        let cells: Vec<_> = (0..20)
            .map(|_| pcell(true, 0x09, false, 0xF2, Some((false, 0x09))))
            .collect();
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        assert!(validate_and_convert(&table, NS).is_ok());
    }

    /// 閉状態の変換モード違いのセルは`convert_cells`で1枠に畳まれる。分母を生セル数にすると、
    /// 畳んだだけで（学習は全セル成功でも）カバレッジが下がる（実機CI: GJI+ATOK 0.782）。
    #[test]
    fn closed_cells_folded_by_mode_do_not_lower_coverage() {
        // 閉状態でモード違い(0x00/0x09/0x11)の同キー3セル→1枠（結果は一致するので採用される）。
        let mk = |mode: u8| {
            let mut pc = pcell(false, mode, false, 0xF2, Some((true, 0x09)));
            pc.prediction.as_mut().unwrap().disp = Disposition::None;
            pc
        };
        let mut cells = vec![mk(0x00), mk(0x09), mk(0x11)];
        // 開状態の別キーセル。
        cells.push(pcell(true, 0x09, false, 0x0D, Some((true, 0x09))));
        assert_eq!(coverage_slot_count(&cells), 2);
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        assert_eq!(validate_and_convert(&table, NS).unwrap().len(), 2);
    }

    /// 開状態で変換モードを`Conv`で表せないセル（MS-IME本体など）は、そもそも変換対象になりえない
    /// ので分母に数えない（実機CI: MS-IME本体 生143セル→畳んだ後74〜76、0.52〜0.53だった）。
    #[test]
    fn unrepresentable_open_mode_cells_are_not_counted_in_the_denominator() {
        let mut cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x00)))];
        // 表現できない変換モード(0x03=半角カタカナ)の開状態セルを大量に混ぜる。
        for key in [0xF2, 0x0D, 0x1B, 0x20] {
            cells.push(pcell(true, 0x03, false, key, Some((true, 0x00))));
        }
        assert_eq!(coverage_slot_count(&cells), 1);
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        assert!(validate_and_convert(&table, NS).is_ok());
    }

    /// 予測なし（非決定・未測定）のセルは枠に数える。縮退した表（予測なしだらけ）は引き続き棄却される。
    #[test]
    fn cells_without_prediction_still_count_against_coverage() {
        let mut cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x00)))];
        for key in [0x0D, 0x1B, 0x08, 0x20] {
            cells.push(pcell(true, 0x09, false, key, None));
        }
        assert_eq!(coverage_slot_count(&cells), 5);
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        assert!(matches!(
            validate_and_convert(&table, NS).unwrap_err(),
            RejectReason::CoverageTooLow { .. }
        ));
    }

    /// C-3回帰テスト(opus-adversarial-consult 2026-09-23): 書き手の採否判定が
    /// `Accepted`でなければ、カバレッジ・同梱表突き合わせがどちらも問題無くても
    /// 不採用にする。`judgement: None`（1e前半以前に書かれたv2ファイル、または
    /// 判定フィールド自体が無い）も同様に不採用へ倒す（判定不明を安全側=不採用に
    /// 倒す、C-5と同じ向き）。
    #[test]
    fn judgement_not_accepted_is_rejected_regardless_of_coverage_or_mismatch() {
        let cells: Vec<_> = (0..10)
            .map(|_| pcell(true, 0x09, false, 0xF2, Some((true, 0x00))))
            .collect();

        let no_judgement = PersistedTable::new(cells.clone());
        assert_eq!(
            validate_and_convert(&no_judgement, NS).unwrap_err(),
            RejectReason::NotAccepted { judgement: None }
        );

        let rejected = PersistedTable::new(cells.clone()).with_judgement(TableJudgement::Rejected(
            awase_keymap_learn::judgement::RejectedReason::LowAccuracy,
        ));
        assert!(matches!(
            validate_and_convert(&rejected, NS).unwrap_err(),
            RejectReason::NotAccepted {
                judgement: Some(TableJudgement::Rejected(_))
            }
        ));

        let needs_confirmation =
            PersistedTable::new(cells).with_judgement(TableJudgement::NeedsConfirmation(
                awase_keymap_learn::judgement::NeedsConfirmationReason::UnverifiedMsImeNative,
            ));
        assert!(matches!(
            validate_and_convert(&needs_confirmation, NS).unwrap_err(),
            RejectReason::NotAccepted {
                judgement: Some(TableJudgement::NeedsConfirmation(_))
            }
        ));
    }

    /// `diff_against_bundled_cells`用の1セルだけの合成同梱表。ひらがな(0xF2)開で押すと
    /// 閉じない(`after_open: true`)という、上の`mismatch_against_bundled_is_rejected_when_checked`
    /// と同じ実測ベースの値を使う。
    fn one_cell_bundled_table() -> Vec<Cell> {
        vec![make_cell(
            true,
            Some(Conv::C19),
            Stage::None,
            TableKey::Hiragana,
            true,
            Some(Conv::C10),
            Disp::Kept,
        )]
    }

    /// 閉→開のセル(同梱表は`after_conv: None`＝不明)。学習側は実測モード(`Some`)を持つが、
    /// 「不明」との違いは矛盾ではない(実機CI実測、run 35933929391の偽不一致10セルの型)。
    fn none_after_conv_bundled_table() -> Vec<Cell> {
        vec![make_cell(
            false,
            None,
            Stage::None,
            TableKey::Hiragana,
            true,
            None,
            Disp::None,
        )]
    }

    /// B-2: 閉状態の変換モードだけが違う複数セルは、入力順に依存しない1セルへ畳まれる。
    #[test]
    fn closed_cells_differing_only_by_hidden_mode_collapse_order_independently() {
        let mk = |mode: u8, after_mode: u8| {
            let mut pc = pcell(false, mode, false, 0xF2, Some((true, after_mode)));
            pc.prediction.as_mut().unwrap().disp = Disposition::None;
            pc
        };
        let a = mk(0x00, 0x00);
        let b = mk(0x09, 0x09);
        let fwd = convert_cells(&[a.clone(), b.clone()]);
        let rev = convert_cells(&[b, a]);
        assert_eq!(fwd, rev);
        assert_eq!(fwd.len(), 1);
        assert!(fwd[0].after_open());
        assert_eq!(
            fwd[0].after_conv(),
            None,
            "保持モードが割れるので追跡を捨てる"
        );
    }

    /// B-2: 閉状態のモードで開閉結果そのものが割れるセルは予測なしにする（順序非依存）。
    #[test]
    fn closed_cells_disagreeing_on_after_open_are_dropped() {
        let a = pcell(false, 0x00, false, 0xF2, Some((true, 0x09)));
        let b = pcell(false, 0x09, false, 0xF2, Some((false, 0x00)));
        assert!(convert_cells(&[a.clone(), b.clone()]).is_empty());
        assert!(convert_cells(&[b, a]).is_empty());
    }

    #[test]
    fn diff_against_bundled_treats_unknown_bundled_after_conv_as_no_claim() {
        let mut cells = vec![pcell(false, 0x00, false, 0xF2, Some((true, 0x09)))];
        cells[0].prediction.as_mut().unwrap().disp = Disposition::None;
        let diff = diff_against_bundled_cells(&cells, &none_after_conv_bundled_table());
        assert_eq!(diff.matched, 1, "{diff:?}");
        assert!(diff.mismatched.is_empty());
    }

    #[test]
    fn diff_against_bundled_still_flags_after_open_difference_when_after_conv_unknown() {
        let mut cells = vec![pcell(false, 0x00, false, 0xF2, Some((false, 0x00)))];
        cells[0].prediction.as_mut().unwrap().disp = Disposition::None;
        let diff = diff_against_bundled_cells(&cells, &none_after_conv_bundled_table());
        assert_eq!(diff.mismatched.len(), 1);
    }

    /// 逆向き: 学習側が`after_conv: None`(追跡を捨てた)で同梱表が`Some`なら、学習表の方が
    /// 情報を落としているので不一致に数える(採否ゲートが弱くならない)。
    #[test]
    fn learned_unknown_after_conv_against_known_bundled_is_still_a_mismatch() {
        // 学習側: 開→開でモード0x01(raw & 0x0B==0x01でConv表現不能→after_conv None)。同梱表: Some(C10)。
        let cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x01)))];
        let learned = convert_cells(&cells);
        assert_eq!(learned.len(), 1, "変換できるセルであること");
        assert_eq!(learned[0].after_conv(), None);
        let diff = diff_against_bundled_cells(&cells, &one_cell_bundled_table());
        assert_eq!(diff.mismatched.len(), 1);
    }

    #[test]
    fn diff_against_bundled_flags_conflicting_known_after_conv() {
        // 同梱表 after_conv=Some(C10)、学習側は別モード(0x09→C19)へ遷移: 本物の矛盾。
        let cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x09)))];
        let diff = diff_against_bundled_cells(&cells, &one_cell_bundled_table());
        assert_eq!(diff.mismatched.len(), 1);
    }

    #[test]
    fn diff_against_bundled_counts_matching_cell_as_matched() {
        let cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x00)))];
        let diff = diff_against_bundled_cells(&cells, &one_cell_bundled_table());
        assert_eq!(diff.matched, 1);
        assert!(diff.mismatched.is_empty());
        assert_eq!(diff.only_in_one_table, 0);
    }

    #[test]
    fn diff_against_bundled_reports_mismatched_cell_identity() {
        // 上記`mismatch_against_bundled_is_rejected_when_checked`と同じ「わざと閉じる」誤り。
        let cells = vec![pcell(true, 0x09, false, 0xF2, Some((false, 0x09)))];
        let diff = diff_against_bundled_cells(&cells, &one_cell_bundled_table());
        assert_eq!(diff.matched, 0);
        assert_eq!(
            diff.mismatched,
            vec![MismatchedCell {
                status: Status {
                    open: true,
                    mode: 0x09,
                    composing: false,
                },
                key: KeyId(0xF2),
            }]
        );
        assert_eq!(diff.only_in_one_table, 0);
    }

    #[test]
    fn diff_against_bundled_counts_unconvertible_cell_as_only_in_one_table() {
        // 表に無いVK(0x99)は変換できない=「学習表にのみ存在」扱い(突き合わせの分母外)。
        // 同梱表側の1セルも学習表からは一致しないので、あわせて2件になる。
        let cells = vec![pcell(true, 0x09, false, 0x99, Some((true, 0x00)))];
        let diff = diff_against_bundled_cells(&cells, &one_cell_bundled_table());
        assert_eq!(diff.matched, 0);
        assert!(diff.mismatched.is_empty());
        assert_eq!(diff.only_in_one_table, 2);
    }

    #[test]
    fn diff_against_bundled_counts_uncovered_bundled_cells_as_only_in_one_table() {
        // 学習表が空なら、同梱表の全セルが「同梱表にのみ存在」になる。
        let diff = diff_against_bundled_cells(&[], &one_cell_bundled_table());
        assert_eq!(diff.matched, 0);
        assert!(diff.mismatched.is_empty());
        assert_eq!(diff.only_in_one_table, 1);
    }

    #[test]
    fn diff_against_bundled_uses_the_real_bundled_table_for_the_given_preset() {
        // 実際の同梱ATOK表全体を使う統合テスト。細かい一致・不一致の判定は上の合成表テストで
        // 検証済みなので、ここでは実表への配線(件数の集計が壊れていないこと)だけ確認する。
        let cells = vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x00)))];
        let diff = diff_against_bundled(&cells, KeymapPreset::Atok);
        assert_eq!(
            u64::from(diff.matched) + diff.mismatched.len() as u64,
            1,
            "唯一の学習セルは、一致か不一致のいずれかとして数えられるはず"
        );
        assert!(
            diff.only_in_one_table > 0,
            "同梱ATOK表は1セルよりずっと多いはず"
        );
    }

    #[test]
    fn schema_version_mismatch_is_rejected() {
        let mut table =
            PersistedTable::new(vec![pcell(true, 0x09, false, 0xF2, Some((true, 0x00)))]);
        table.schema_version = persist::CURRENT_SCHEMA_VERSION + 1;
        let json = table.to_json().unwrap();
        let err = persist::from_json(&json).unwrap_err();
        assert!(matches!(err, LoadError::SchemaVersionMismatch { .. }));
    }

    #[test]
    fn runtime_table_cache_loads_once_and_rechecks_by_stamp() {
        use std::cell::Cell as StdCell;
        let loads = StdCell::new(0u32);
        let mut cache = RuntimeTableCache::default();
        let load = || {
            loads.set(loads.get() + 1);
            Some(vec![])
        };
        let key = (KeymapPreset::Atok, true, FP);
        assert!(cache.get(0, key, || Some((1, 10)), load).is_some());
        assert_eq!(loads.get(), 1);
        // 間隔内はfsを読まない。
        assert!(cache.get(500, key, || Some((1, 10)), load).is_some());
        assert_eq!(loads.get(), 1);
        // 間隔を過ぎても版が同じなら読み直さない。
        assert!(cache
            .get(RuntimeTableCache::RECHECK_MS, key, || Some((1, 10)), load)
            .is_some());
        assert_eq!(loads.get(), 1);
        // 版が変わったら読み直す。
        assert!(cache
            .get(
                RuntimeTableCache::RECHECK_MS * 2,
                key,
                || Some((2, 10)),
                load
            )
            .is_some());
        assert_eq!(loads.get(), 2);
    }

    #[test]
    fn runtime_table_cache_reloads_when_preset_changes_even_if_file_stamp_is_unchanged() {
        // GJIのプリセット切替(session_keymap変更)は学習済み表ファイル自体を書き換えない。
        // ファイルスタンプだけで版を比較すると、切り替え後もATOK向けに検証済みだった
        // 古いセルを黙って使い続けてしまう(旧プリセットの構成に対してのみ妥当な
        // 縮退率/突き合わせ判定を経たセルを、新プリセットへそのまま流用する事故)。
        use std::cell::Cell as StdCell;
        let loads = StdCell::new(0u32);
        let mut cache = RuntimeTableCache::default();
        let load = || {
            loads.set(loads.get() + 1);
            Some(vec![])
        };
        assert!(cache
            .get(0, (KeymapPreset::Atok, true, FP), || Some((1, 10)), load)
            .is_some());
        assert_eq!(loads.get(), 1);
        // ファイルスタンプは同じ(1, 10)のまま、プリセットだけがMsImeへ変わった。
        assert!(cache
            .get(
                RuntimeTableCache::RECHECK_MS,
                (KeymapPreset::MsIme, true, FP),
                || Some((1, 10)),
                load
            )
            .is_some());
        assert_eq!(loads.get(), 2, "プリセット変更で読み直すべき");
        // check_against_bundledだけが変わった場合も読み直す(カスタム構成の有無で検証内容が違う)。
        assert!(cache
            .get(
                RuntimeTableCache::RECHECK_MS * 2,
                (KeymapPreset::MsIme, false, FP),
                || Some((1, 10)),
                load
            )
            .is_some());
        assert_eq!(loads.get(), 3, "check_against_bundled変更でも読み直すべき");
    }

    #[test]
    fn runtime_table_cache_reloads_on_preset_change_even_within_the_recheck_window() {
        // code-review指摘: 前のテストはいずれもnow_msをRECHECK_MSの倍数にしており、
        // 「dueがtrueの場合にvalidation_keyの変化を検出できるか」しか確認していなかった。
        // フォーカス移動によるプリセット切替はRECHECK_MSの間引き窓の途中でも起こりうるため、
        // dueがfalseのままでも(fsを問い合わせ直さずとも)古いプリセット向けのセルを
        // 新しいプリセットへ流用してはいけない。
        use std::cell::Cell as StdCell;
        let loads = StdCell::new(0u32);
        let mut cache = RuntimeTableCache::default();
        let load = || {
            loads.set(loads.get() + 1);
            Some(vec![])
        };
        assert!(cache
            .get(0, (KeymapPreset::Atok, true, FP), || Some((1, 10)), load)
            .is_some());
        assert_eq!(loads.get(), 1);
        // RECHECK_MSの窓の途中(now_msを1msしか進めない、due=false)でプリセットが変わった。
        assert!(cache
            .get(1, (KeymapPreset::MsIme, true, FP), || Some((1, 10)), load)
            .is_some());
        assert_eq!(
            loads.get(),
            2,
            "RECHECK_MSの窓の途中でもプリセット変更は読み直すべき"
        );
    }

    // ---- 陳腐化検出(学習表の指紋照合、ADR-195段階8の実行時配線) ----

    fn accepted_table_with(fingerprint: Option<Fingerprint>) -> PersistedTable {
        let cells: Vec<_> = (0..10)
            .map(|_| pcell(true, 0x09, false, 0xF2, Some((true, 0x00))))
            .collect();
        let mut t = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        t.fingerprint = fingerprint;
        t
    }

    fn gji_fp(session: i64, custom: Option<&str>, overlay: &[i64]) -> Fingerprint {
        awase_keymap_learn::fingerprint::gji_keymap_fingerprint(Some(session), custom, overlay)
    }

    fn adopt(table: &PersistedTable, current: FingerprintProbe) -> Result<Vec<Cell>, RejectReason> {
        validate_and_convert(table, current)
    }

    /// (a) 指紋が違う表は棄却される。一致すれば採用される。
    #[test]
    fn table_with_different_fingerprint_is_rejected_as_stale() {
        let table = accepted_table_with(Some(gji_fp(1, None, &[])));
        let err = adopt(&table, FingerprintProbe::Computed(gji_fp(2, None, &[]))).unwrap_err();
        assert_eq!(err, RejectReason::Stale(Staleness::FingerprintMismatch));
        assert!(adopt(&table, FingerprintProbe::Computed(gji_fp(1, None, &[]))).is_ok());
    }

    /// (b) シナリオ2: GJIの指紋の表を、Microsoft IME本体の指紋(`MsImeNative`)の下で読むと棄却。
    #[test]
    fn gji_table_is_rejected_under_ms_ime_native_fingerprint() {
        let table = accepted_table_with(Some(gji_fp(1, None, &[])));
        let native = KeyEffectKeymap::for_msime_native(false, None, None);
        let err = validate_and_convert(&table, FingerprintProbe::Computed(native.fingerprint()))
            .unwrap_err();
        assert_eq!(err, RejectReason::Stale(Staleness::FingerprintMismatch));
    }

    /// (c) 指紋を持つ表 + 現在が指紋方式なし(NotSupported) → 棄却。
    #[test]
    fn stored_fingerprint_against_not_supported_is_rejected() {
        let table = accepted_table_with(Some(gji_fp(1, None, &[])));
        let err = adopt(&table, FingerprintProbe::NotSupported).unwrap_err();
        assert_eq!(err, RejectReason::Stale(Staleness::FingerprintNotSupported));
    }

    /// 現在の指紋を計算できなかった(Unavailable)なら、指紋を持つ表は安全側で棄却。
    #[test]
    fn stored_fingerprint_against_unavailable_is_rejected() {
        let table = accepted_table_with(Some(gji_fp(1, None, &[])));
        let err = adopt(&table, FingerprintProbe::Unavailable).unwrap_err();
        assert_eq!(err, RejectReason::Stale(Staleness::FingerprintUnavailable));
    }

    /// 指紋を持たない旧形式の表は従来どおり(保護されないが、棄却もされない)。
    #[test]
    fn table_without_fingerprint_keeps_legacy_behaviour() {
        let table = accepted_table_with(None);
        assert!(adopt(&table, FingerprintProbe::Computed(gji_fp(1, None, &[]))).is_ok());
        assert!(adopt(&table, FingerprintProbe::NotSupported).is_ok());
    }

    /// (d) overlayの中身だけが違う(どちらも`has_overlay=true`)GJI構成を、`KeyEffectKeymap`経由の
    /// 指紋で区別して棄却する。真偽値から作る旧案では見逃していたケース。
    #[test]
    fn overlay_content_change_is_detected_through_the_keymap_fingerprint() {
        let learned_under = KeyEffectKeymap::from_config(Some(1), None, &[100]).unwrap();
        let now = KeyEffectKeymap::from_config(Some(1), None, &[101]).unwrap();
        assert_eq!(
            learned_under.is_unmodified_bundled_config(),
            now.is_unmodified_bundled_config()
        );
        let table = accepted_table_with(Some(learned_under.fingerprint()));
        let err = adopt(&table, FingerprintProbe::Computed(now.fingerprint())).unwrap_err();
        assert_eq!(err, RejectReason::Stale(Staleness::FingerprintMismatch));
    }

    /// (e) MS-IME本体の再割り当て値が「0以外→別の0以外」に変わったケース
    /// (`henkan_reassigned`はどちらもtrueで、旧案の真偽値では区別できない)。
    #[test]
    fn msime_native_reassignment_value_change_is_detected_through_the_keymap_fingerprint() {
        let learned_under = KeyEffectKeymap::for_msime_native(true, Some(1), Some(1));
        let now = KeyEffectKeymap::for_msime_native(true, Some(1), Some(2));
        let table = accepted_table_with(Some(learned_under.fingerprint()));
        let err = adopt(&table, FingerprintProbe::Computed(now.fingerprint())).unwrap_err();
        assert_eq!(err, RejectReason::Stale(Staleness::FingerprintMismatch));
        assert!(adopt(
            &table,
            FingerprintProbe::Computed(learned_under.fingerprint())
        )
        .is_ok());
    }

    /// キーマップ指紋は`KeyEffectKeymap`構築時に生の入力から作られ、同じ入力なら同じ値。
    #[test]
    fn keymap_fingerprint_is_deterministic_and_input_sensitive() {
        let a = KeyEffectKeymap::from_config(Some(1), Some("t".into()), &[1]).unwrap();
        let b = KeyEffectKeymap::from_config(Some(1), Some("t".into()), &[1]).unwrap();
        let c = KeyEffectKeymap::from_config(Some(1), Some("u".into()), &[1]).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    /// (i) シナリオ1: presetもファイルスタンプも同じまま指紋だけ変わったら、読み直して棄却する
    /// (`RECHECK_MS`の窓の途中でも)。キャッシュキーに指紋が無いと`staleness::check`は二度と呼ばれない。
    #[test]
    fn runtime_table_cache_reloads_and_rejects_when_only_the_fingerprint_changes() {
        let table = accepted_table_with(Some(gji_fp(1, Some("A"), &[])));
        let fp_a = gji_fp(1, Some("A"), &[]);
        let fp_b = gji_fp(1, Some("B"), &[]);
        let mut cache = RuntimeTableCache::default();
        let load_for = |fp: Fingerprint| {
            let table = table.clone();
            move || validate_and_convert(&table, FingerprintProbe::Computed(fp)).ok()
        };
        assert!(cache
            .get(
                0,
                (KeymapPreset::Custom, false, fp_a),
                || Some((1, 10)),
                load_for(fp_a)
            )
            .is_some());
        // 同じpreset・同じファイルスタンプ・窓の途中。指紋だけが変わった。
        assert!(cache
            .get(
                1,
                (KeymapPreset::Custom, false, fp_b),
                || Some((1, 10)),
                load_for(fp_b)
            )
            .is_none());
        assert!(!cache.is_active());
        // 元の構成へ戻せば再び採用される。
        assert!(cache
            .get(
                2,
                (KeymapPreset::Custom, false, fp_a),
                || Some((1, 10)),
                load_for(fp_a)
            )
            .is_some());
    }

    /// CI専用(`--ignored`、環境変数`KL_TABLE_PATH`): 実機の学習プロセスが書いた
    /// `keymap-learn-table.json`を実行時の採否判定(`load_runtime_table`)に通し、
    /// 生セル数・変換できたセル数・カバレッジの分母（畳んだ後に変換対象になりえた枠数）と
    /// 採否を出力する。学習表が同梱表と違っていても棄却されない（ADR-196決定1e）ことの実機確認用。
    #[test]
    #[ignore = "CI用: 実機の学習表(KL_TABLE_PATH)が要る"]
    fn ci_real_learned_table_is_adopted() {
        let path = std::env::var("KL_TABLE_PATH").expect("KL_TABLE_PATH");
        let path = Path::new(&path);
        let persisted = read_persisted_table(path).expect("読める");
        let converted = convert_cells(&persisted.cells);
        println!(
            "CI-RESULT raw={} converted={} slots={} coverage={:.3} limit={MIN_COVERAGE_RATIO}",
            persisted.cells.len(),
            converted.len(),
            coverage_slot_count(&persisted.cells),
            coverage_ratio(&persisted.cells, converted.len()),
        );
        let result = load_runtime_table(path, NS);
        println!(
            "CI-RESULT load_runtime_table={:?}",
            result.as_ref().map(Vec::len)
        );
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn load_runtime_table_distinguishes_not_found_from_real_io_errors() {
        // ファイル不在(正常系、ログしない)と、存在するが読み取れない(異常系、ログすべき)を
        // 混同しない。存在しないパスはNotFound。
        let missing = std::env::temp_dir().join("awase_keymap_learn_table_does_not_exist.json");
        let _ = fs::remove_file(&missing);
        assert_eq!(
            load_runtime_table(&missing, NS),
            Err(RejectReason::NotFound)
        );

        // ディレクトリをファイルとして開こうとすると(存在はするが読めない)、
        // NotFoundではなくIoになる。
        let dir = std::env::temp_dir().join(format!(
            "awase_keymap_learn_table_dir_{}",
            unique_test_suffix()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).expect("create test dir");
        let result = load_runtime_table(&dir, NS);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result, Err(RejectReason::Io));
    }

    #[test]
    fn empty_file_is_treated_as_not_learned() {
        let path = std::env::temp_dir().join(format!(
            "awase_keymap_learn_table_empty_{}.json",
            unique_test_suffix()
        ));
        fs::write(&path, b"").expect("write empty file");
        let result = load_runtime_table(&path, NS);
        let _ = fs::remove_file(&path);
        assert_eq!(result, Err(RejectReason::NotFound));
    }

    fn unique_test_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    }

    #[test]
    fn runtime_table_cache_falls_back_to_none_when_load_rejects() {
        let mut cache = RuntimeTableCache::default();
        assert!(cache
            .get(0, (KeymapPreset::Atok, true, FP), || Some((1, 10)), || None)
            .is_none());
    }

    /// B-1 Blockerの安全性の核（本来はCI実機blind格子`ci/e2e-ime.yml`で検証すべき項目の、
    /// ユニットテストでの代替——実機検証は本タスクの残課題として別途フォローアップが必要、
    /// [`docs/tasks/adr195-t4-runtime-loading.md`]参照）: **合成の`custom_keymap_table`**
    /// （同梱3種のいずれとも一致しないカスタムキーマップ）から作った学習表を
    /// `predict_in_table`に通したとき、(1) 学習表に無いセルは`None`（予測なし、observation
    /// 側に委ねる安全側）を返すこと、(2) 学習表にあるセルは学習した通りの結果を返すこと、
    /// の両方を固定する。「間違った予測を返す」ことだけが実害（読めない窓でのblind誤り）になる
    /// ため、「予測なし」に倒れる経路が壊れていないことが安全性の core。
    #[test]
    fn custom_table_predictions_are_faithful_or_absent_never_silently_wrong() {
        use super::super::key_effect_predictor::{KeyTrack, PredictInput};
        use awase::engine::InputModeState;

        // カスタムキーマップ学習: ひらがな(0xF2)を押すと開閉トグルする、という(同梱3種のいずれとも
        // 違う)独自の挙動を1セルだけ学習した表。
        let learned = vec![pcell(false, 0x00, false, 0xF2, Some((true, 0x09)))]; // 閉→開
        let table = PersistedTable::new(learned).with_judgement(TableJudgement::Accepted);
        let cells = validate_and_convert(&table, NS)
            .expect("カスタム構成は突き合わせをしないので採用される");

        let closed = PredictInput {
            open: false,
            mode: InputModeState::ObservedRomaji,
            conv_raw: None,
            composing: false,
            track: KeyTrack::default(),
        };
        // (2) 学習した通りの結果。
        let p = super::super::key_effect_predictor::predict_in_table(&cells, 0xF2, &closed)
            .expect("学習したセルは予測する");
        assert_eq!(p.effect.open, Some(true));

        // (1) 学習していないキー(Enter)は「予測なし」——間違った値を捏造しない。
        assert!(
            super::super::key_effect_predictor::predict_in_table(&cells, 0x0D, &closed).is_none()
        );
    }

    /// ADR-195追記: 読込時に「半角/全角が開閉トグルでない」判定を前計算し、`cells`と同時に更新・失効する。
    #[test]
    fn hankaku_zenkaku_non_toggle_is_precomputed_with_cells() {
        use super::super::key_effect_predictor::{cell, Disp};
        let k = TableKey::HankakuZenkaku;
        // IMEOn割当: 閉→開、開→開（C19/C10）。
        let non_toggle = vec![
            cell(false, None, Stage::None, k, true, None, Disp::None),
            cell(
                true,
                Some(Conv::C19),
                Stage::None,
                k,
                true,
                Some(Conv::C19),
                Disp::None,
            ),
            cell(
                true,
                Some(Conv::C10),
                Stage::None,
                k,
                true,
                Some(Conv::C10),
                Disp::None,
            ),
        ];
        let key = (KeymapPreset::Custom, false, FP);
        let mut cache = RuntimeTableCache::default();
        assert_eq!(cache.toggle_contradiction(k), None);
        cache.get(0, key, || Some((1, 1)), || Some(non_toggle));
        assert_eq!(
            cache.toggle_contradiction(k),
            Some(ToggleContradiction::IdleOpenStaysOpen)
        );
        // 対象キー以外・対象外のキーは常に`None`。
        assert_eq!(cache.toggle_contradiction(TableKey::Henkan), None);
        assert_eq!(cache.toggle_contradiction(TableKey::Space), None);
        // 学習表が消えたら（棄却・ファイル削除）判定も戻る。
        cache.get(RuntimeTableCache::RECHECK_MS, key, || Some((2, 1)), || None);
        assert_eq!(cache.toggle_contradiction(k), None);
    }

    /// ADR-195追記: KeyDownで確定した判定がKeyUpへ持ち越され、途中で表が変わっても揃う。
    #[test]
    fn latch_carries_down_verdict_to_up() {
        use awase::types::ScanCode;
        let scan = ScanCode(0x29);
        let (v, latch) = latch_step(None, false, true, scan, || false);
        assert!(!v);
        let (v, latch2) = latch_step(latch, true, false, scan, || panic!("fresh must not run"));
        assert!(!v);
        assert_eq!(latch2, latch, "Upでラッチを消さない");
        let (v, latch3) = latch_step(latch2, false, true, scan, || false);
        assert!(!v);
        let (v, _) = latch_step(latch3, true, false, scan, || true);
        assert!(!v);
    }

    /// オートリピート（`was_down`のKeyDown。reuse=true）はラッチを再利用し、freshを呼ばない。
    #[test]
    fn latch_reused_on_autorepeat_down() {
        use awase::types::ScanCode;
        let latch = Some((ScanCode(0x29), true));
        let (v, l) = latch_step(latch, true, true, ScanCode(0x29), || panic!("no fresh"));
        assert!(v);
        assert_eq!(l, latch);
    }

    /// scan違い/scan 0/ラッチ無しのreuseはその場で判定する。injectedのDownはラッチを更新しない。
    #[test]
    fn latch_ignores_other_scan_and_injected() {
        use awase::types::ScanCode;
        let latch = Some((ScanCode(0x29), false));
        let (v, l) = latch_step(latch, true, false, ScanCode(0x1E), || true);
        assert!(v);
        assert_eq!(l, latch);
        let (v, _) = latch_step(Some((ScanCode(0), false)), true, false, ScanCode(0), || {
            true
        });
        assert!(v);
        let (v, _) = latch_step(None, true, false, ScanCode(0x29), || true);
        assert!(v);
        let (v, l) = latch_step(latch, false, false, ScanCode(0x29), || true);
        assert!(v);
        assert_eq!(l, latch);
    }

    /// 分岐: 学習表使用・表フラグ真の2つが揃うときだけ外す（opt-out・学習表なしは外さない）。
    #[test]
    fn hz_omit_verdict_truth_table() {
        for use_learned in [false, true] {
            for flag in [false, true] {
                assert_eq!(hz_omit_verdict(use_learned, flag), use_learned && flag);
            }
        }
        assert!(hz_omit_may_apply(true));
        assert!(!hz_omit_may_apply(false));
    }

    /// 早期returnでラッチを捨てるのは、半角/全角の非injected KeyDownだけ。
    #[test]
    fn early_return_clears_latch_only_for_fresh_hz_down() {
        assert!(should_clear_latch_on_early_return(true, true, false));
        assert!(
            !should_clear_latch_on_early_return(true, false, false),
            "KeyUpは消さない"
        );
        assert!(
            !should_clear_latch_on_early_return(true, true, true),
            "injectedは消さない"
        );
        assert!(
            !should_clear_latch_on_early_return(false, true, false),
            "他キーは触らない"
        );
    }
}
