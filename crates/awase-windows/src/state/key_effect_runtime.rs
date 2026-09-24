//! ADR-195 段階4: 予測器（`key_effect_predictor.rs`）の実行時読込。
//!
//! `awase-keymap-learn::persist`（段階3の永続化フォーマット）が書き出した学習済み表を、
//! コンパイル時埋め込みの同梱表（`key_effect_table.rs`）の代わりに使う。読み込んだ表は
//! `KeyEffectPredicted`（belief更新）にのみ使い、actuationの判定（ADR-189の固定セット・
//! ユーザー明示config）には一切使わない——本モジュールは`predict_in_table`が引く`Cell`の
//! 一覧を用意するだけで、actuationのどの合流点も呼ばない。
//!
//! 安全側に倒す3つの経路（本タスクB-1 Blockerの核心）:
//! 1. **破損ファイルへの縮退**: サイズ上限超過・パース失敗・スキーマ版不一致は、
//!    無条件で同梱表へフォールバック（`RuntimeTableCache::get`が`None`を返す）。
//! 2. **縮退率チェック**: 変換できたセル（VK・変換モードが表現可能で、かつ予測ありの
//!    セル）の割合が[`MIN_COVERAGE_RATIO`]未満なら不採用。
//! 3. **同梱表とのセル突き合わせ**: 検出したキーマップがカスタム上書き無し（同梱3種の
//!    いずれかとそのまま一致する構成）のときだけ、学習表と同梱表をセル単位で突き合わせ、
//!    不一致率が[`MAX_MISMATCH_RATIO`]を超えたら不採用（カスタム構成では学習表が同梱表と
//!    食い違うのが正常なので、この判定はしない）。
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
use awase_keymap_learn::persist::{self, LoadError, PersistedCell, PersistedTable};

use super::key_effect_predictor::{
    bundled_table, cell as make_cell, Cell, Conv, Disp, KeymapPreset, Stage, TableKey,
};

/// 読み込むファイルの上限サイズ。壊れた/異常に巨大なファイルを丸ごとメモリに載せない
/// （B-1 Blockerの「ファイルサイズに上限を設ける」）。
pub const MAX_TABLE_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// 変換できた（＝実際に使える）セルの割合がこれ未満なら不採用（縮退率チェックの裏返し。
/// ADR本文の「縮退率20%」＝カバレッジ80%を暫定既定値とする）。
pub const MIN_COVERAGE_RATIO: f64 = 0.80;

/// 同梱表とのセル不一致率がこれを超えたら不採用（カスタム構成無しのときだけ判定）。
pub const MAX_MISMATCH_RATIO: f64 = 0.05;

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
    /// 同梱表とのセル不一致率が[`MAX_MISMATCH_RATIO`]を超えた。
    MismatchesBundledTooMuch {
        mismatch_ratio: f64,
    },
    /// 書き手（学習プロセス）の採否判定（[`awase_keymap_learn::judgement::TableJudgement`]）が
    /// `Accepted`でない（`Rejected`・`NeedsConfirmation`のいずれか）、または判定フィールド
    /// 自体が無い（`judgement: None`、1e前半以前に書かれたv2ファイル）（ADR196-T2決定1e、
    /// opus-adversarial-consult 2026-09-23 C-3: 判定を書いても読み手が読まなければ、
    /// 書いたのに効かない状態になる）。
    NotAccepted {
        judgement: Option<awase_keymap_learn::judgement::TableJudgement>,
    },
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
            Self::MismatchesBundledTooMuch { mismatch_ratio } => write!(
                f,
                "同梱表とのセル不一致率が高すぎる(mismatch_ratio={mismatch_ratio:.2})"
            ),
            Self::NotAccepted { judgement } => {
                write!(
                    f,
                    "書き手の採否判定がAcceptedでない(judgement={judgement:?})"
                )
            }
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
fn convert_cells(cells: &[PersistedCell]) -> Vec<Cell> {
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

fn convert_cell(pc: &PersistedCell) -> Option<Cell> {
    let key = TableKey::from_vk(pc.key.0)?;
    let outcome: Outcome = pc.prediction?;
    // 開状態のセルだけ変換モードを持つ（閉セルは`conv: None`がワイルドカード、Cellの既存の約束）。
    // 開状態で変換モードが表現できないときはセルごと除外する（`open`かつ変換不能は無効な組み合わせ）。
    let conv = if pc.status.open {
        Some(Conv::from_raw(u32::from(pc.status.mode))?)
    } else {
        None
    };
    let after_open = outcome.status.open;
    let after_conv = if after_open {
        // 変換不能なモードへ遷移した場合は「押下後の変換が不明」として追跡を捨てる
        // （既存の`predict_in_table`が`after_conv: None`を「開く/閉じる遷移で不明」として扱うのと同じ扱い）。
        Conv::from_raw(u32::from(outcome.status.mode))
    } else {
        None
    };
    let stage = if pc.status.composing {
        Stage::Typing
    } else {
        Stage::None
    };
    let disp = match outcome.disp {
        Disposition::None => Disp::None,
        Disposition::Kept => Disp::Kept,
        Disposition::Discarded => Disp::Discarded,
        Disposition::Committed => Disp::Committed,
    };
    Some(make_cell(
        pc.status.open,
        conv,
        stage,
        key,
        after_open,
        after_conv,
        disp,
    ))
}

/// 変換できたセルの割合（B-1 Blockerの縮退率チェック）。
fn coverage_ratio(raw: &[PersistedCell], converted_len: usize) -> f64 {
    if raw.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = converted_len as f64 / raw.len() as f64;
    ratio
}

/// 学習表と同梱表のセル不一致率。同梱表の各セルについて、学習表に同じ
/// `(open, conv, stage, key)`のセルがあり、かつ`after_open`/`after_conv`/`disp`のいずれかが
/// 食い違うものを数える(`after_conv`は[`after_conv_conflicts`]、片方が`None`＝不明なら矛盾でない)（学習表に無いセル＝単に未学習は不一致に数えない、突き合わせの対象は
/// 「同梱表にあるセルのうち学習表でも答えが出ているもの」だけ）。
fn mismatch_ratio(learned: &[Cell], bundled: &[Cell]) -> f64 {
    let mut compared = 0usize;
    let mut mismatched = 0usize;
    for b in bundled {
        let Some(l) = learned
            .iter()
            .find(|c| c.matches_lookup_key(b.open(), b.conv(), b.stage(), b.key()))
        else {
            continue;
        };
        compared += 1;
        if l.after_open() != b.after_open()
            || after_conv_conflicts(l.after_conv(), b.after_conv())
            || l.disp() != b.disp()
        {
            mismatched += 1;
        }
    }
    if compared == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = mismatched as f64 / compared as f64;
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
pub(crate) fn load_and_log(preset: KeymapPreset, check_against_bundled: bool) -> Option<Vec<Cell>> {
    let path = table_file_path()?;
    match load_runtime_table(&path, preset, check_against_bundled) {
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

/// `std::io::Error`を、ファイル不在(正常系)とそれ以外の読み取り失敗(異常系、ログすべき)
/// とを区別できる[`RejectReason`]へ変換する。
fn io_reject_reason(e: &std::io::Error) -> RejectReason {
    if e.kind() == std::io::ErrorKind::NotFound {
        RejectReason::NotFound
    } else {
        RejectReason::Io
    }
}

/// ファイルを読み、パース・スキーマ検証・縮退率チェック・（該当すれば）同梱表とのセル突き合わせまで
/// 行う。`check_against_bundled`は[`super::key_effect_predictor::KeyEffectKeymap::is_unmodified_bundled_config`]の
/// 結果を渡す（カスタム構成では突き合わせをしない）。
///
/// # Errors
/// 採用できない理由を[`RejectReason`]で返す。
pub fn load_runtime_table(
    path: &Path,
    preset: KeymapPreset,
    check_against_bundled: bool,
) -> Result<Vec<Cell>, RejectReason> {
    let table = read_persisted_table(path)?;
    validate_and_convert(&table, preset, check_against_bundled)
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
    preset: KeymapPreset,
    check_against_bundled: bool,
) -> Result<Vec<Cell>, RejectReason> {
    if table.judgement != Some(awase_keymap_learn::judgement::TableJudgement::Accepted) {
        return Err(RejectReason::NotAccepted {
            judgement: table.judgement,
        });
    }
    let converted = convert_cells(&table.cells);
    let coverage = coverage_ratio(&table.cells, converted.len());
    if coverage < MIN_COVERAGE_RATIO {
        return Err(RejectReason::CoverageTooLow { coverage });
    }
    if check_against_bundled {
        let ratio = mismatch_ratio(&converted, bundled_table(preset));
        if ratio > MAX_MISMATCH_RATIO {
            return Err(RejectReason::MismatchesBundledTooMuch {
                mismatch_ratio: ratio,
            });
        }
    }
    Ok(converted)
}

/// `config1.db`スタンプ（[`super::key_effect_predictor::KeymapCache`]）と同じ方式のfsキャッシュ。
/// `RECHECK_MS`ごとにファイルの版（更新時刻+長さ）だけを問い合わせ、変わったときだけ読み直す。
/// 判定は純関数で、fs/時計は呼び出し側が渡す（テスト容易性のため`KeymapCache`と同じ形にする）。
///
/// ファイル自身のスタンプに加えて`(KeymapPreset, check_against_bundled)`も版の一部として
/// 比較する——学習済み表ファイル自体は変わっていなくても、GJIのプリセット切替
/// （`session_keymap`）やカスタム構成の有無が変わると、`validate_and_convert`が
/// 検証に使う`preset`/`check_against_bundled`が変わり、以前キャッシュしたセルは
/// 新しい構成に対して未検証のまま（かつVK/モードの意味が構成ごとに違いうる）になる。
/// ファイルスタンプだけで比較すると、プリセットを切り替えても再学習していない限り
/// 古いプリセット向けに検証済みのセルを黙って使い続けてしまう。
#[derive(Debug, Default)]
pub struct RuntimeTableCache {
    checked_at_ms: Option<u64>,
    stamp: Option<(u64, u64, KeymapPreset, bool)>,
    cells: Option<Vec<Cell>>,
}

impl RuntimeTableCache {
    /// 直近の`get`で学習済み表が採用されている（＝予測に使われている）か。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.cells.is_some()
    }

    /// 直近の読込で使った`(preset, check_against_bundled)`（診断用。学習表ファイルが
    /// 無い/読めなかった場合は`None`）。
    #[must_use]
    pub fn last_validation_key(&self) -> Option<(KeymapPreset, bool)> {
        self.stamp.map(|(_, _, preset, check)| (preset, check))
    }

    pub const RECHECK_MS: u64 = super::key_effect_predictor::KeymapCache::RECHECK_MS;

    /// キャッシュした学習済み表を返す（採用できなかった/未学習なら`None`＝呼び出し側は同梱表を使う）。
    ///
    /// `validation_key`は`(preset, check_against_bundled)`——呼び出し側が`load`に渡すのと
    /// 同じ値を渡すこと（版の一部として比較され、変わればファイルスタンプが同じでも読み直す）。
    pub fn get(
        &mut self,
        now_ms: u64,
        validation_key: (KeymapPreset, bool),
        stamp: impl FnOnce() -> Option<(u64, u64)>,
        load: impl FnOnce() -> Option<Vec<Cell>>,
    ) -> Option<&[Cell]> {
        let first = self.checked_at_ms.is_none();
        let due = self
            .checked_at_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= Self::RECHECK_MS);
        // code-review指摘: validation_key(preset/check_against_bundled)の変化は、
        // RECHECK_MS(fsアクセスの間引き)とは独立に毎回チェックする。プリセット切替は
        // フォーカス移動というユーザー操作でRECHECK_MSの窓の途中でも起こりうり、
        // dueがfalseのまま素通りすると古いプリセット向けに検証済みのセルを
        // 新しいプリセットの予測にそのまま使い続けてしまう(セルの意味がプリセットごとに
        // 違いうるため、これは黙って誤った予測を返す事故になる)。
        let validation_key_changed = !first
            && self
                .stamp
                .is_some_and(|(_, _, preset, check)| (preset, check) != validation_key);
        if due || validation_key_changed {
            self.checked_at_ms = Some(now_ms);
            let now_stamp =
                stamp().map(|(mtime, len)| (mtime, len, validation_key.0, validation_key.1));
            if first || now_stamp != self.stamp {
                self.stamp = now_stamp;
                self.cells = load();
            }
        }
        self.cells.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let err = validate_and_convert(&table, KeymapPreset::Atok, false).unwrap_err();
        assert!(matches!(err, RejectReason::CoverageTooLow { .. }));
    }

    #[test]
    fn high_coverage_without_bundled_check_is_accepted() {
        let cells: Vec<_> = (0..10)
            .map(|_| pcell(true, 0x09, false, 0xF2, Some((true, 0x00))))
            .collect();
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        let out = validate_and_convert(&table, KeymapPreset::Atok, false).unwrap();
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn mismatch_against_bundled_is_rejected_when_checked() {
        // 同梱ATOK表にある、ひらがな(0xF2)を開いた状態で押した結果は「閉じない」実測（コード中の
        // `atok_hiragana_is_a_pure_toggle_between_hiragana_and_halfwidth_alnum`テスト参照）。
        // ここではわざと「閉じる」という誤った学習結果を大量に混ぜ、突き合わせで不採用になることを固定する。
        let cells: Vec<_> = (0..20)
            .map(|_| pcell(true, 0x09, false, 0xF2, Some((false, 0x09))))
            .collect();
        let table = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        let rejected_when_checked =
            validate_and_convert(&table, KeymapPreset::Atok, true).unwrap_err();
        assert!(matches!(
            rejected_when_checked,
            RejectReason::MismatchesBundledTooMuch { .. }
        ));
        // カスタム構成(突き合わせなし)なら同じ表でも採用される。
        assert!(validate_and_convert(&table, KeymapPreset::Atok, false).is_ok());
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
            validate_and_convert(&no_judgement, KeymapPreset::Atok, false).unwrap_err(),
            RejectReason::NotAccepted { judgement: None }
        );

        let rejected = PersistedTable::new(cells.clone()).with_judgement(TableJudgement::Rejected(
            awase_keymap_learn::judgement::RejectedReason::LowAccuracy,
        ));
        assert!(matches!(
            validate_and_convert(&rejected, KeymapPreset::Atok, false).unwrap_err(),
            RejectReason::NotAccepted {
                judgement: Some(TableJudgement::Rejected(_))
            }
        ));

        let needs_confirmation =
            PersistedTable::new(cells).with_judgement(TableJudgement::NeedsConfirmation(
                awase_keymap_learn::judgement::NeedsConfirmationReason::UnverifiedMsImeNative,
            ));
        assert!(matches!(
            validate_and_convert(&needs_confirmation, KeymapPreset::Atok, false).unwrap_err(),
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

    /// 実行時の不採用判定`mismatch_ratio`も同じ扱い(実機CI実測: 偽不一致10/73≒13.7%が
    /// 上限`MAX_MISMATCH_RATIO`(5%)を超え、未改造GJI ATOKの学習表が不採用になりうる型)。
    #[test]
    fn mismatch_ratio_treats_unknown_bundled_after_conv_as_no_claim() {
        let mut pc = pcell(false, 0x00, false, 0xF2, Some((true, 0x09)));
        pc.prediction.as_mut().unwrap().disp = Disposition::None;
        let learned = convert_cells(&[pc]);
        assert_eq!(
            mismatch_ratio(&learned, &none_after_conv_bundled_table()),
            0.0
        );
        // 既知同士の矛盾は従来どおり不一致に数える。
        let conflicting = convert_cells(&[pcell(true, 0x09, false, 0xF2, Some((true, 0x09)))]);
        assert_eq!(mismatch_ratio(&conflicting, &one_cell_bundled_table()), 1.0);
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
        assert_eq!(mismatch_ratio(&learned, &one_cell_bundled_table()), 1.0);
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
        let key = (KeymapPreset::Atok, true);
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
            .get(0, (KeymapPreset::Atok, true), || Some((1, 10)), load)
            .is_some());
        assert_eq!(loads.get(), 1);
        // ファイルスタンプは同じ(1, 10)のまま、プリセットだけがMsImeへ変わった。
        assert!(cache
            .get(
                RuntimeTableCache::RECHECK_MS,
                (KeymapPreset::MsIme, true),
                || Some((1, 10)),
                load
            )
            .is_some());
        assert_eq!(loads.get(), 2, "プリセット変更で読み直すべき");
        // check_against_bundledだけが変わった場合も読み直す(カスタム構成の有無で検証内容が違う)。
        assert!(cache
            .get(
                RuntimeTableCache::RECHECK_MS * 2,
                (KeymapPreset::MsIme, false),
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
            .get(0, (KeymapPreset::Atok, true), || Some((1, 10)), load)
            .is_some());
        assert_eq!(loads.get(), 1);
        // RECHECK_MSの窓の途中(now_msを1msしか進めない、due=false)でプリセットが変わった。
        assert!(cache
            .get(1, (KeymapPreset::MsIme, true), || Some((1, 10)), load)
            .is_some());
        assert_eq!(
            loads.get(),
            2,
            "RECHECK_MSの窓の途中でもプリセット変更は読み直すべき"
        );
    }

    /// CI専用(`--ignored`、環境変数`KL_TABLE_PATH`): 実機の学習プロセスが書いた
    /// `keymap-learn-table.json`を、未改造GJI ATOKとして実行時の採否判定
    /// (`load_runtime_table`、`mismatch_ratio`込み)に通す。修正前(`after_conv`の単純`==`比較)の
    /// 不一致率も併せて出力し、偽不一致が実際に採否を分けたかを実測で確認できるようにする。
    #[test]
    #[ignore = "CI用: 実機の学習表(KL_TABLE_PATH)が要る"]
    fn ci_real_learned_table_is_adopted_for_unmodified_atok() {
        let path = std::env::var("KL_TABLE_PATH").expect("KL_TABLE_PATH");
        let path = std::path::Path::new(&path);
        let persisted = read_persisted_table(path).expect("読める");
        let learned = convert_cells(&persisted.cells);
        let preset = match std::env::var("KL_PRESET").as_deref() {
            Ok("msime-native") => KeymapPreset::MsImeNative,
            Ok("msime") => KeymapPreset::MsIme,
            _ => KeymapPreset::Atok,
        };
        let bundled = bundled_table(preset);
        let (mut compared, mut strict_mismatch) = (0usize, 0usize);
        for b in bundled {
            if let Some(l) = learned
                .iter()
                .find(|c| c.matches_lookup_key(b.open(), b.conv(), b.stage(), b.key()))
            {
                compared += 1;
                if l.after_open() != b.after_open()
                    || l.after_conv() != b.after_conv()
                    || l.disp() != b.disp()
                {
                    strict_mismatch += 1;
                }
            }
        }
        let pre_merge: Vec<Cell> = persisted.cells.iter().filter_map(convert_cell).collect();
        let closed_pre: Vec<&Cell> = pre_merge.iter().filter(|c| !c.open()).collect();
        let mut closed_modes: Vec<String> = closed_pre.iter().map(|c| format!("{:?}", c.conv())).collect();
        closed_modes.sort_unstable();
        closed_modes.dedup();
        println!(
            "CI-RESULT preset={preset:?} judgement={:?} raw_cells={} convertible_pre_merge={} closed_pre_merge={} open_pre_merge={} closed_distinct_conv_modes={} converted_after_merge={} coverage={:.3} min_coverage={MIN_COVERAGE_RATIO} bundled_cells={}",
            persisted.judgement,
            persisted.cells.len(),
            pre_merge.len(),
            closed_pre.len(),
            pre_merge.len() - closed_pre.len(),
            closed_modes.len(),
            learned.len(),
            coverage_ratio(&persisted.cells, learned.len()),
            bundled.len(),
        );
        println!(
            "CI-RESULT compared={compared} old_strict_mismatch={strict_mismatch} old_ratio={:.3} new_ratio={:.3} limit={MAX_MISMATCH_RATIO}",
            strict_mismatch as f64 / compared.max(1) as f64,
            mismatch_ratio(&learned, bundled),
        );
        let result = load_runtime_table(path, preset, true);
        println!(
            "CI-RESULT load_runtime_table={:?}",
            result.as_ref().map(Vec::len)
        );
        println!("CI-RESULT adopted={}", result.is_ok());
    }

    #[test]
    fn load_runtime_table_distinguishes_not_found_from_real_io_errors() {
        // ファイル不在(正常系、ログしない)と、存在するが読み取れない(異常系、ログすべき)を
        // 混同しない。存在しないパスはNotFound。
        let missing = std::env::temp_dir().join("awase_keymap_learn_table_does_not_exist.json");
        let _ = fs::remove_file(&missing);
        assert_eq!(
            load_runtime_table(&missing, KeymapPreset::Atok, true),
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
        let result = load_runtime_table(&dir, KeymapPreset::Atok, true);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result, Err(RejectReason::Io));
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
            .get(0, (KeymapPreset::Atok, true), || Some((1, 10)), || None)
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
        let cells = validate_and_convert(&table, KeymapPreset::Atok, false)
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
}
