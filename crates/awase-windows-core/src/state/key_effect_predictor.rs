//! ADR-191 決定3・4: 打鍵の時点で、(状態, キー)→効果の表を引いてbeliefを予測する**予測器**
//! （`predict()`、隠れ状態の追跡規則`KeyTrack`、キーマップ差の判定）。純粋関数だけで、状態は持たない。
//!
//! **このファイルは表のデータを持たない**（ファイル名は`_table`だが実体は予測器）。表のデータは
//! `key_effect_table.rs`（生成物）にある。
//!
//! awaseはIMEへ書かない。生キーはそのままIMEへ通り、ここでは**その結果を先取りして**beliefへ
//! 反映するための予測だけを返す（観測は後から確認・訂正する。`ime_model.rs`の`KeyEffectPredicted`）。
//!
//! # 出所（学習結果だけ）
//!
//! 表の中身は**手で書かない**。`tools/e2e/ime_key_matrix/gen_key_effect_table.py`が、CI実機の
//! `--grid`学習（awaseを完全にバイパスした注入。`grid-tables/{atok,msime}.json`）から生成する
//! `key_effect_table.rs`だけがデータ源である（ADR-191の3段階ラウンド: 設定の読み取り→学習→検証）。
//! MS-IMEは「GJIのMS-IMEプリセット」の表で、Microsoft IME本体ではない。
//!
//! # 状態
//!
//! `(開閉, 変換モード, 入力中の段階)`。変換モードはプリセットごとにキーで到達できる2値だけ、閉(OFF)状態では追わない。入力中の段階のうち**変換中（`Conversion`）は観測できない
//! 隠れ状態**なので、打鍵履歴から`KeyTrack`が追跡する（変換/無変換/Spaceで入り、Esc/Enter/文字入力等で出る）。
//! 変換モードは`Conv`（ROMANビットを除いたconvの生値）。
//!
//! **カスタムキーマップ・overlayが対象キーの行を上書きしている場合は予測しない**（`None`、観測に任せる）。
//!
//! # 予測しないもの
//!
//! - 表に無い・非決定のセル（生成時に除外）は`None`（観測が唯一の信号になる）。
//! - ADR-189の固定セット（半角/全角0xF3/0xF4・漢字0x19）: 呼び出し側が`shadow_action.is_some()`で
//!   除外する（二重に効かせない）。ただし GJI の採用学習表が半角/全角を開閉トグルでないと示す場合は
//!   `shadow_action`が外れるので、半角/全角はこの予測で追随する（ADR-195追記）。0x16/0x1A（`VK_IME_ON`/`OFF`）は呼び出し側が追随の対象にしない。

use awase::engine::{AssumedReason, InputModeState};

/// MS-IME本体の旧UIのキーテンプレート(`HKCU\\Software\\Microsoft\\IME\\15.0\\IMEJP\\MSIME\\keystyle`)の
/// 読み取り結果(ADR-254)。OS 依存の読み取りは殻(`msime_legacy_keymap`)が行い、ここには結果だけを渡す。
///
/// 実機(新エンジン=互換 OFF、旧エンジン=互換 ON)とCIの測定で、`keystyle`が既定でないときは
/// 同梱表`MSIME_NATIVE`(NATURAL相当)の予測が外れることが分かった(ATOK等の名前付きスタイルは
/// 新旧エンジンとも別の内蔵表で動く。`Custom`は互換 ON のときだけレジストリの表が効き、互換 OFF では
/// 無視されて NATURAL と同じ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsImeKeystyle {
    /// 値が無い(旧UIの設定画面を一度も開いていない既定)。NATURAL と同じ扱い。
    Absent,
    /// `NATURAL`(Microsoft IME の既定)。
    Natural,
    /// `Custom`(旧UIの「ユーザー定義」)。表が効くのは互換 ON のときだけ。
    Custom,
    /// 名前付きの他のスタイル(`ATOK`・`VJE`・`WX`・`MS-IME2000`〈IME_Standard〉)。
    Named,
    /// 未知の名前、または読み取りに失敗した。安全側(予測しない)。
    Unknown,
}

impl MsImeKeystyle {
    /// このテンプレートと互換モードの組み合わせで、同梱表`MSIME_NATIVE`の予測を止めるか(ADR-254 決定1)。
    /// - `Absent`/`Natural`: 止めない。
    /// - `Named`/`Unknown`: 止める(互換 ON/OFF とも。新旧エンジンの内蔵表は`MSIME_NATIVE`と違う)。
    /// - `Custom`: 互換 OFF(`Some(false)`)だけ止めない(新エンジンは`Custom`を読まない)。互換 ON と
    ///   読めない(`None`)は止める。
    #[must_use]
    pub const fn disables_native_prediction(self, compat_mode: Option<bool>) -> bool {
        match self {
            Self::Absent | Self::Natural => false,
            Self::Named | Self::Unknown => true,
            Self::Custom => !matches!(compat_mode, Some(false)),
        }
    }

    /// 指紋・版スタンプに混ぜる種別タグ(`Absent`/`Natural`は同じ挙動なので同じ値。指紋には混ぜない)。
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Absent | Self::Natural => 0,
            Self::Custom => 1,
            Self::Named => 2,
            Self::Unknown => 3,
        }
    }
}

/// 予測に使うキーマップの系統。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapPreset {
    Atok,
    /// GJIのMS-IMEプリセット（Microsoft IME本体ではない）。
    MsIme,
    /// Microsoft IME本体（`ActiveImeKind::MicrosoftIme`を明示検出したときだけ。ADR-191、CI `cal-notify-msimenative-*`）。
    MsImeNative,
    /// `session_keymap`がATOK/MSIME以外（CUSTOM・MOBILE等）で、基準となる同梱表が無い構成
    /// （ADR-195段階4 B3対応）。`bundled_table`は空を返す——`is_unmodified_bundled_config`が
    /// 常に`false`を返すため、この空表がセル突き合わせ判定で実際に引かれることはない。
    /// 学習済み表(ADR-195段階4)があれば、この構成でも`predict_with_override`経由で使える。
    Custom,
}

/// 変換モード（`conv`の生値からROMANビットを除いた、キーで到達できる3種）。
///
/// 格子第2版（変換モードをキーで到達）の実測: IME単独のキーで入れる変換モードは、ATOKで`C19`・`C10`、
/// MS-IMEプリセットで`C19`・`C1B`の2つだけ（半角カタカナ0x13・全角英数0x18は到達不能）。
/// 表現できない値（0x13/0x18等）は追わない（`from_raw`が`None`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Conv {
    /// 半角英数
    C10,
    /// ひらがな
    C19,
    /// 全角カタカナ
    C1B,
}

impl Conv {
    /// `conv`の生値（IMEのconversion mode）から。NATIVE(1)・KATAKANA(2)・FULLSHAPE(8)だけを見る
    /// （ROMAN(0x10)はGJIが報告しない）。3種以外の組み合わせは`None`（追わない）。
    #[must_use]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw & 0x0B {
            0x00 => Some(Self::C10),
            0x09 => Some(Self::C19),
            0x0B => Some(Self::C1B),
            _ => None,
        }
    }

    /// かな入力系（NATIVEビットあり）か。EngineはこのときだけNICOLAを有効にする。
    #[must_use]
    pub const fn is_native(self) -> bool {
        matches!(self, Self::C19 | Self::C1B)
    }
}

/// 入力中の段階。`None`は入力中でない。`Typing`と変換中3種のうち、変換中は打鍵履歴からの追跡（隠れ状態）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub enum Stage {
    #[default]
    None,
    /// 未確定文字列がある（変換前）。
    Typing,
    /// Spaceで変換中（候補選択）。
    ConvSpace,
    /// 変換キーで変換中。
    ConvHenkan,
    /// 無変換で入る英数変換中（`ToggleAlphanumericMode`の変換系状態）。
    ConvMuhenkan,
}

/// 表が持つキー（学習した13種）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKey {
    Bs,
    Eisu,
    Enter,
    Esc,
    HankakuZenkaku,
    Henkan,
    Hiragana,
    ImeOff,
    ImeOn,
    Kanji,
    Katakana,
    Muhenkan,
    Space,
}

impl TableKey {
    /// VKから。表に無いキー（文字キー等）は`None`。
    #[must_use]
    pub const fn from_vk(vk: u16) -> Option<Self> {
        Some(match vk {
            0x08 => Self::Bs,
            0xF0 => Self::Eisu,
            0x0D => Self::Enter,
            0x1B => Self::Esc,
            0xF3 | 0xF4 => Self::HankakuZenkaku,
            0x1C => Self::Henkan,
            0xF2 => Self::Hiragana,
            0x1A => Self::ImeOff,
            0x16 => Self::ImeOn,
            0x19 => Self::Kanji,
            0xF1 => Self::Katakana,
            0x1D => Self::Muhenkan,
            0x20 => Self::Space,
            _ => return None,
        })
    }
}

/// 入力中の文字列の行方（押下後）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disp {
    /// 入力中でなかった（行方なし）。
    None,
    /// 保持（入力中/変換中のまま）。
    Kept,
    /// 破棄。
    Discarded,
    /// 確定。
    Committed,
}

/// 学習した1セル: 押下前の状態とキー → 押下後の開閉・変換モード・入力中の行方。
///
/// `conv`が`None`のセルは、変換モードを問わない（閉(OFF)状態のセル。閉状態の変換モードの読み取りは不安定なため、
/// 開閉だけを予測する）。`after_conv`が`None`のセルは、押下後の変換モードが不明（閉になる/開く遷移、
/// 表現できないモード）で、追跡を捨てる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub(super) open: bool,
    conv: Option<Conv>,
    stage: Stage,
    pub(super) key: TableKey,
    pub(super) after_open: bool,
    after_conv: Option<Conv>,
    pub(super) disp: Disp,
}

/// `key_effect_table.rs`（生成物）が使うセル構築子。
#[must_use]
pub const fn cell(
    open: bool,
    conv: Option<Conv>,
    stage: Stage,
    key: TableKey,
    after_open: bool,
    after_conv: Option<Conv>,
    disp: Disp,
) -> Cell {
    Cell {
        open,
        conv,
        stage,
        key,
        after_open,
        after_conv,
        disp,
    }
}

impl Cell {
    /// [`crate::state::key_effect_runtime`]が、学習表と同梱表のセルを突き合わせるためのアクセサ。
    #[must_use]
    pub const fn open(&self) -> bool {
        self.open
    }
    #[must_use]
    pub const fn conv(&self) -> Option<Conv> {
        self.conv
    }
    #[must_use]
    pub const fn stage(&self) -> Stage {
        self.stage
    }
    #[must_use]
    pub const fn key(&self) -> TableKey {
        self.key
    }
    #[must_use]
    pub const fn after_open(&self) -> bool {
        self.after_open
    }
    #[must_use]
    pub const fn after_conv(&self) -> Option<Conv> {
        self.after_conv
    }
    #[must_use]
    pub const fn disp(&self) -> Disp {
        self.disp
    }

    /// 同じ`(open, conv, stage, key)`のセルか（[`crate::state::key_effect_runtime`]が学習表と
    /// 同梱表のセルを突き合わせるための同一性判定。`open`と`conv`は常に連動する——閉セルは
    /// `conv: None`固定、開セルは常に`Some`——ので単純な等値比較でよい）。
    #[must_use]
    pub fn matches_lookup_key(
        &self,
        open: bool,
        conv: Option<Conv>,
        stage: Stage,
        key: TableKey,
    ) -> bool {
        self.open == open && self.conv == conv && self.stage == stage && self.key == key
    }
}

fn find_in(table: &[Cell], open: bool, conv: Conv, stage: Stage, key: TableKey) -> Option<&Cell> {
    table.iter().find(|c| {
        c.open == open && c.conv.is_none_or(|cv| cv == conv) && c.stage == stage && c.key == key
    })
}

const fn table_of(preset: KeymapPreset) -> &'static [Cell] {
    match preset {
        KeymapPreset::Atok => super::key_effect_table::ATOK,
        KeymapPreset::MsIme => super::key_effect_table::MSIME,
        KeymapPreset::MsImeNative => super::key_effect_table::MSIME_NATIVE,
        KeymapPreset::Custom => &[],
    }
}

/// 同梱（コンパイル時埋め込み）の表。[`crate::state::key_effect_runtime`]が、実行時に読み込んだ
/// 表との突き合わせ（ADR-195段階4の縮退率・セル不一致率チェック）に使う。
#[must_use]
pub(crate) const fn bundled_table(preset: KeymapPreset) -> &'static [Cell] {
    table_of(preset)
}

/// 打鍵履歴から追跡する隠れ状態（`ImeModel`が`KeyEffectPredicted`で持つ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct KeyTrack {
    /// 直近の予測が示した変換モード。`None`なら観測（`prev_conversion_mode`）か既定から引く。
    pub conv: Option<Conv>,
    /// 入力中の段階（入力中でないときは無視され、`None`扱い）。
    pub stage: Stage,
}

impl KeyTrack {
    /// 左 Shift 単独タップの持続半角英数トグル中は、追跡した変換モード（`conv`）を捨てる（ADR-244 M-2）。
    ///
    /// トグル開始は awase 自身が `conv=0x0000` を書く（`InputModeApplied`）が、reducer は `input_mode` だけを書いて
    /// 追跡（`key_track`）には触れない。予測付きのモードキー（ひらがな 0xF2 等）の後にトグルを開始すると、追跡は
    /// `Some(C19)` のまま残り、トグル中の 無変換・ひらがな に予測が付いて belief を「開・ローマ字」へ動かす——実 IME は
    /// 閉じる／半角英数のままなので Engine だけが ON になり、入力が空になる（BUG-192）。さらに予測が付いた打鍵では
    /// 直接観測（`follow_direct_read_in_scope`）が見送られ、持続トグルも手放されない。
    ///
    /// 追跡を捨てれば、入力モードの `ObservedEisu` から変換モードを `C10` と見なし、予測なしになる（追跡が空の場合と
    /// 同じ。S1 で 8/8、CI の `sc-bug186-msime-shift-toggle-*`）。入力中の段階（`stage`）は残す。
    ///
    /// 注意: これは予測の**入力**だけを替えるが、トグル中の文字キーなどの予測は `track.conv = None` を返すので、
    /// `KeyEffectPredicted` として保存される追跡の `conv` もそこで消える。トグルを抜けた後の予測は、`conv_raw`、
    /// 次いで入力モードから変換モードを決める（解除後の実状態はかななので方向は正しい）。
    #[must_use]
    pub const fn without_conv_while_half_width_alnum(self, toggle_active: bool) -> Self {
        if toggle_active {
            Self {
                conv: None,
                stage: self.stage,
            }
        } else {
            self
        }
    }
}

/// 表から予測した、beliefへの反映内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PredictedEffect {
    /// 予測される開閉。`None` = 変えない。
    pub open: Option<bool>,
    /// 予測される入力モード。`None` = 変えない。
    pub mode: Option<InputModeState>,
}

impl PredictedEffect {
    /// 何も変わらない予測（beliefを書き換えない）。
    #[must_use]
    pub const fn is_noop(&self) -> bool {
        self.open.is_none() && self.mode.is_none()
    }

    /// 続けて押した打鍵の効果を重ねる（後の打鍵が値を持つ軸はそれで上書きし、持たない軸は前の値を残す）。
    #[must_use]
    fn overlaid_by(self, later: Self) -> Self {
        Self {
            open: later.open.or(self.open),
            mode: later.mode.or(self.mode),
        }
    }
}

/// 予測結果: beliefへの反映と、更新後の追跡状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prediction {
    pub effect: PredictedEffect,
    pub track: KeyTrack,
}

/// 予測の入力（打鍵前の、awaseが知っている状態）。
#[derive(Debug, Clone, Copy)]
pub struct PredictInput {
    /// 打鍵前のbeliefの開閉。
    pub open: bool,
    /// 打鍵前のbeliefの入力モード。`Unknown`のときは既定（ひらがな）を種にして予測を始める。
    pub mode: InputModeState,
    /// 直近に観測した`conv`の生値（読めるアプリのみ）。
    pub conv_raw: Option<u32>,
    /// 入力中（未確定文字列あり）か（TSFの観測）。
    pub composing: bool,
    /// 追跡中の隠れ状態。
    pub track: KeyTrack,
    /// 前面窓が「IME の実状態を読めない」種類（`cannot_verify_real_ime_state`かつ`InputRelay`以外）で、
    /// 窓別の規則（ADR-209）を使ってよいか。設定で止められる（止めるとき`false`）。
    pub unreadable: bool,
    /// 表に無い受動のキー（プリセットで閉状態から開く F13、ADR-211）の規則を当ててよい打鍵か。イベント側の条件
    /// （KeyDown・自動リピートでない・非 injected・`shadow_action`/`sync_direction` が無い・エンジンが消費していない・修飾なし）を
    /// `kp_stage_key_effect_track` が計算して渡す。規則は窓の種類にも ADR-209 の設定にも依らない。
    pub passive_rule_eligible: bool,
}

impl PredictInput {
    /// この状態で `step` の打鍵が IME に届いた後の状態（ADR-188 案2、[`KeyEffectKeymap::predict_after_resent`]）。
    /// 開閉・入力モードは予測の値（`None` は変えない）、追跡は予測のもの。`composing`（打鍵前の TSF 観測）は
    /// 打鍵後には当てにならないので偽にし、入力中の段階は追跡（`track.stage`）だけで決める。
    fn after(self, step: Prediction) -> Self {
        Self {
            open: step.effect.open.unwrap_or(self.open),
            mode: step.effect.mode.unwrap_or(self.mode),
            composing: false,
            track: step.track,
            ..self
        }
    }
}

const fn kana_mode() -> InputModeState {
    InputModeState::AssumedRomaji {
        reason: AssumedReason::KeyEffectPrediction,
    }
}

/// 入力モードのbelief（Eisuか否か）が、変換モード`conv`と食い違うときだけ、反映すべき値を返す。
/// `Unknown`は既定の種として必ず返す（読めない/不明のときも、予測を始められるように）。
fn mode_effect(current: InputModeState, conv: Conv) -> Option<InputModeState> {
    let target = if conv.is_native() {
        kana_mode()
    } else {
        InputModeState::ObservedEisu
    };
    let current_native = !matches!(current, InputModeState::ObservedEisu);
    if matches!(current, InputModeState::Unknown) || current_native != conv.is_native() {
        Some(target)
    } else {
        None
    }
}

/// 入力中の段階の遷移（キー種別の小さな規則）。表が持つのは「保持/破棄/確定」だけ。
const fn next_stage(prev: Stage, key: TableKey, disp: Disp, open_after: bool) -> Stage {
    if !open_after {
        return Stage::None;
    }
    match disp {
        Disp::None | Disp::Discarded | Disp::Committed => Stage::None,
        Disp::Kept => match key {
            TableKey::Henkan => Stage::ConvHenkan,
            TableKey::Muhenkan => Stage::ConvMuhenkan,
            TableKey::Space => Stage::ConvSpace,
            TableKey::Esc => Stage::None,
            _ => prev,
        },
    }
}

/// 文字を入力するキー（英数字・記号）か。開いている間に押すと未確定文字列ができる（入力中になる）。
const fn is_char_vk(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A | 0xBA..=0xC0 | 0xDB..=0xDF)
}

/// 表を引いて予測を返す。予測できない（表に無い・非決定・プリセット外）ときは`None`。
///
/// 表に無いキー（文字キー等）は、開閉・入力モードを変えないが、変換中の段階だけは`Typing`へ戻す
/// （変換中に文字を打つと確定して新しい入力中になる）。
#[must_use]
pub fn predict(preset: KeymapPreset, vk: u16, input: &PredictInput) -> Option<Prediction> {
    predict_in_table(table_of(preset), vk, input)
}

/// [`predict`]と同じ規則だが、表を`preset`ではなく直接指定する。ADR-195段階4（実行時読込）が
/// 検証済みの学習済み表（[`crate::state::key_effect_runtime`]）を同梱表の代わりに引くための入口。
#[must_use]
pub fn predict_in_table(table: &[Cell], vk: u16, input: &PredictInput) -> Option<Prediction> {
    let seeded = matches!(input.mode, InputModeState::Unknown);
    let conv = input
        .track
        .conv
        .or_else(|| input.conv_raw.and_then(Conv::from_raw))
        .unwrap_or(if matches!(input.mode, InputModeState::ObservedEisu) {
            Conv::C10
        } else {
            Conv::C19
        });
    // 入力中（未確定文字列あり）か。観測（TSF）が読めるアプリでは`composing`が真になる。読めないアプリでは
    // 観測が無いので、追跡した段階（文字キーで`Typing`、変換系キーで変換中）を正とする（開いている間だけ）。
    let stage = if input.open {
        match input.track.stage {
            Stage::None if input.composing => Stage::Typing,
            s => s,
        }
    } else {
        Stage::None
    };
    let Some(key) = TableKey::from_vk(vk) else {
        // 文字キー等: 開いていれば入力中（`Typing`）になる（変換中に打てば確定して新しい入力中）。閉なら段階なし。
        // 種（Unknown）は必ず反映する。
        let new_stage = if input.open && is_char_vk(vk) {
            Stage::Typing
        } else if matches!(input.track.stage, Stage::None) {
            Stage::None
        } else {
            Stage::Typing
        };
        let track = KeyTrack {
            conv: input.track.conv,
            stage: new_stage,
        };
        let effect = PredictedEffect {
            open: None,
            mode: if seeded { Some(kana_mode()) } else { None },
        };
        return (!effect.is_noop() || track != input.track).then_some(Prediction { effect, track });
    };
    // 追跡した変換中の段階が、この表に**行として全く無い**とき（例: ATOKに`ConvMuhenkan`の行は無い）は、
    // 「入力中」の行で代用する。代用しないと予測が返らず、追跡した段階が古いまま残って以後の打鍵の予測が
    // 全て外れる（CI blind: ATOKで入力中の無変換の後、Enter/半角全角が予測なしのまま OFF/ON がずれ続けた）。
    // 段階の行はあるが、そのキーのセルだけが非決定で除外されている場合は代用しない（予測なしのまま）。
    let stage_modeled = |st: Stage| {
        table
            .iter()
            .any(|c| c.open == input.open && c.conv.is_none_or(|cv| cv == conv) && c.stage == st)
    };
    let Some(c) = find_in(table, input.open, conv, stage, key).or_else(|| {
        (matches!(
            stage,
            Stage::ConvSpace | Stage::ConvHenkan | Stage::ConvMuhenkan
        ) && !stage_modeled(stage))
        .then(|| find_in(table, input.open, conv, Stage::Typing, key))
        .flatten()
    }) else {
        // BUG-162: 表に該当セルが無い（ATOK の入力中/変換中の Esc は保持/破棄が割れるので生成時に除外される）
        // ときは、開閉・入力モードは予測しない（観測に委ねる）が、**入力中の段階の追跡は捨てる**。
        // 入力中（`Typing`）の Esc は未確定文字列を破棄するので、セルが無くても段階への効果は
        // `next_stage` の Esc の規則と同じ（`Stage::None`）。変換中（`Conv*`）の Esc は保持/破棄が実際に
        // 割れる（元の状態へ戻る）ので、ここでは触らない（追跡は従来どおり）。追跡を捨てないと、`k`→Esc の後も段階が `Typing` のまま残り、次の無変換が「入力中の無変換
        // （かなのまま）」と誤予測され、Engine が活性化して `SetOpen(true)` で IME を書き戻す（読めない窓では
        // 観測が追跡を直せない）。
        if matches!(key, TableKey::Esc) && input.track.stage == Stage::Typing {
            return Some(Prediction {
                effect: PredictedEffect {
                    open: None,
                    mode: seeded.then(kana_mode),
                },
                track: KeyTrack {
                    conv: input.track.conv,
                    stage: Stage::None,
                },
            });
        }
        return None;
    };
    // 押下後の変換モードが不明（閉になる/開く遷移など）のときは、追跡を捨てる。入力モードは種（Unknown）だけ反映する。
    let effect = PredictedEffect {
        open: (c.after_open != input.open).then_some(c.after_open),
        mode: c
            .after_conv
            .and_then(|cv| mode_effect(input.mode, cv))
            .or_else(|| seeded.then(kana_mode)),
    };
    let track = KeyTrack {
        conv: c.after_conv,
        stage: next_stage(stage, key, c.disp, c.after_open),
    };
    Some(Prediction { effect, track })
}

/// 旧UIの`StyleList\\Custom`の表(`S4key`・`key`)の、無変換/変換(無修飾)の行から、打鍵時予測のセルを作る
/// (ADR-254 第二段)。**効果が実測で確かめられたコードだけ**セルにし、それ以外(未知のコード・行が無い・
/// 表が読めない・変化なしのコード)はセルを作らない=予測しない(観測に任せる。変化なしのキーは、予測しなければ
/// beliefも変わらないので正しい)。
///
/// 実測(CI windows-latest と実機 dragonflyg4〈互換 ON〉、各セル n=3〜5、全試行一致。ADR-254 実測5・13・16):
/// - 閉じた状態(`S4key`の1列目): `87`・`CE`→IME が開く(変換モードは`C19`のまま)。
/// - 開・入力なし(`key`の1列目、ひらがな`C19`のとき): `97`・`C9`→`C1B`(全角カタカナ)、`CD`・`B3`→IME が閉じる、
///   `A4`→閉じて半角英数(`C10`)。`D5`は ND(直接入力モードを使用しない)で向きが逆になるので除く。
/// - 入力中・変換中の列(`key`の2〜6列目)と、開・入力なしのひらがな以外のモードは、実測が無いので読まない。
///
/// 対象は無変換(`0x1D`)と変換(`0x1C`)だけ。呼び出し側は、互換 ON かつ `keystyle=Custom` のときだけ使う。
#[must_use]
pub fn legacy_custom_cells(s4key: Option<&[u8]>, key: Option<&[u8]>) -> Vec<Cell> {
    // 行のラベル(Shift-JIS)。修飾付き(`Ctrl+`など)は別のラベルになるので、完全一致だけを見る。
    const MUHENKAN: &[u8] = &[0x96, 0xB3, 0x95, 0xCF, 0x8A, 0xB7];
    const HENKAN: &[u8] = &[0x95, 0xCF, 0x8A, 0xB7];
    let mut cells = Vec::new();
    for (label, table_key) in [(MUHENKAN, TableKey::Muhenkan), (HENKAN, TableKey::Henkan)] {
        if let Some(code) = s4key.and_then(|t| first_column_code(t, label)) {
            if matches!(code, 0x87 | 0xCE) {
                cells.push(cell(
                    false,
                    None,
                    Stage::None,
                    table_key,
                    true,
                    None,
                    Disp::None,
                ));
            }
        }
        if let Some(code) = key.and_then(|t| first_column_code(t, label)) {
            let after = match code {
                0x97 | 0xC9 => Some((true, Some(Conv::C1B))),
                0xCD | 0xB3 => Some((false, None)),
                0xA4 => Some((false, Some(Conv::C10))),
                _ => None,
            };
            if let Some((after_open, after_conv)) = after {
                cells.push(cell(
                    true,
                    Some(Conv::C19),
                    Stage::None,
                    table_key,
                    after_open,
                    after_conv,
                    Disp::None,
                ));
            }
        }
    }
    cells
}

/// 表(`<ラベル>=XX XX XX XX XX XX`をNULで区切ったShift-JISテキスト)から、`label`と完全に一致する行の
/// 1列目のコード(16進)を返す。行が無い・形式が違う・複数あって食い違うときは`None`(安全側)。
fn first_column_code(table: &[u8], label: &[u8]) -> Option<u8> {
    let mut found: Option<u8> = None;
    for record in table.split(|&b| b == 0).filter(|r| !r.is_empty()) {
        let Some(eq) = record.iter().position(|&b| b == b'=') else {
            continue;
        };
        if &record[..eq] != label {
            continue;
        }
        let value = std::str::from_utf8(&record[eq + 1..]).ok()?;
        let first = value.split_whitespace().next()?;
        let code = u8::from_str_radix(first, 16).ok()?;
        match found {
            None => found = Some(code),
            Some(prev) if prev == code => {}
            // 同じ行が食い違って複数ある: 優先順位が未確認なので使わない。
            Some(_) => return None,
        }
    }
    found
}

/// `config1.db`から得た、予測に使うキーマップ（プリセット+カスタム上書きの検出材料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEffectKeymap {
    pub(super) preset: KeymapPreset,
    /// `config1.db`の生の`session_keymap`（不在=`None`）。`preset`はATOK/MSIME以外を`Custom`に
    /// まとめるので、役割判定（ADR-199 決定4）のプリセット判別にはこちらを使う。指紋はハッシュで
    /// 復元できないので別に持つ（決定8 (i)）。Microsoft IME本体のキーマップでは使わない（`None`）。
    session_keymap: Option<i64>,
    pub(super) custom_table: Option<String>,
    /// `config1.db`の生の`overlay_keymaps`（役割判定は種類で受動にするキーが違う、ADR-199 決定4）。
    overlay_keymaps: Vec<i64>,
    /// Microsoft IME本体のキー割り当て（レジストリ`KeyAssignmentHenkan`/`Muhenkan`）に明示値がある
    /// （`IsKeyAssignmentEnabled=1`かつ値が存在する）。その変換/無変換の打鍵は予測しない（GJIのoverlay/
    /// カスタム上書きと同じ安全側）。ADR-199 T12（2026-09-26実機確認）で値0=IME-オン・3=既定
    /// 〈かな切替/再変換〉と確定し、「0=既定」という以前の前提が逆転した。値0/3の実機的意味が
    /// 確認できるまでは明示値なら一律に予測しない（決定C R3・M5、推測で値を決めない）。
    henkan_reassigned: bool,
    muhenkan_reassigned: bool,
    /// 同じ明示値が**2（IME-オン/オフのトグル）**（`IsKeyAssignmentEnabled=1`かつ値==2）。ADR-199 T17 Phase 4:
    /// [`Self::msime_native_key_role`]が無変換/変換に`ImeToggle`を返す根拠。指紋は生の値を含むのでここは指紋に混ぜない。
    henkan_toggle: bool,
    muhenkan_toggle: bool,
    /// MS-IME本体の「以前のバージョンのMicrosoft IMEを使う」互換モード（ADR-197決定4、
    /// [`crate::msime_legacy_keymap::read_legacy_compat_mode_enabled`]）。`Some(true)`=ON・
    /// `Some(false)`=OFF・`None`=読めない（決定17により「新しい版」として扱う）。GJIのキーマップ
    /// では使わない（`None`）。役割判定（[`Self::msime_native_key_role`]）だけが参照し、指紋には
    /// 混ぜない（ADR196-T5の`env_version`が別途担当、M3）。
    msime_compat_mode: Option<bool>,
    /// 旧UIのキーテンプレート(`keystyle`)と互換モードの組み合わせで、`MSIME_NATIVE`の予測が当たらない
    /// 構成(ADR-254 決定1)。`true`なら[`Self::predict_with_override`]は学習表の参照の後で`None`を返す。
    /// GJIのキーマップでは常に`false`。
    legacy_table_unknown: bool,
    /// 予測を止める構成のうち、互換 ON の`Custom`の表(`S4key`・`key`)から作った、無変換/変換のセル
    /// (ADR-254 第二段、[`legacy_custom_cells`])。空なら、止める構成は何も予測しない(第一段のまま)。
    legacy_cells: Vec<Cell>,
    /// 止める構成が「互換 ON の`Custom`」(旧UIの表が効く構成)か。セルを受け付けるのはこのときだけ
    /// (名前付きスタイル・未知は表の中身を読まない)。
    legacy_custom_on: bool,
    /// このキーマップの生の入力（GJI: session/custom/overlay、Microsoft IME本体: 3 DWORD）から
    /// 作った指紋（`awase_keymap_learn::fingerprint`）。上の真偽値は overlay の中身や
    /// 再割り当て値を潰すので、学習表の陳腐化検出（`key_effect_runtime`）にはこちらを使う。
    /// キーマップと同時に計算するので「キーマップは取れたのに指紋だけ取れない」状態は無い。
    fingerprint: awase_keymap_learn::persist::Fingerprint,
}

/// 修飾キーを押したままの打鍵では予測・追跡をしない。
///
/// **限界（round2 A-N10）**: 表のキー（Space/Esc/Enter/BS等）を修飾付きで押すと`Stage`の追跡も更新されないので、
/// 変換中の Shift+Enter 等で実 IME が変換を抜けても追跡は`ConvSpace`等のまま残りうる（読めない窓では観測で直らない）。
/// 誤りの種類を「修飾付きを素のキーとして予測」から「追跡の取り残し」へ入れ替えたもので、実害は未確認。
///
/// レビュー指摘A-B3。旧`enrich_ime_relevance`の修飾キーガード〈ADR-186 残る問題2: Shift+変換はATOKで
/// 開閉トグルではない〉の置き換え。表が持つキー（モードキー・Space/Esc/Enter/BS）はShift/Ctrl/Alt/Winのどれかで抑止する
/// （Shift+Spaceなど表のセルは「素のキー」の結果）。表に無い文字キーはShift（大文字入力）を許し、
/// Ctrl/Alt/Win（ショートカット。入力中にならない）だけ抑止する。
#[must_use]
pub const fn modifiers_suppress_prediction(
    in_table: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
) -> bool {
    if in_table {
        ctrl || alt || shift || win
    } else {
        ctrl || alt || win
    }
}

/// [`plan_key_effect_track`] が読む打鍵の事実（殻 `kp_stage_key_effect_track` が `RawKeyEvent`/`Decision` から写す）。
#[derive(Debug, Clone, Copy)]
pub struct KeyTrackFacts {
    pub vk: u16,
    /// 注入でない物理キーの KeyDown か（KeyUp・自己注入は追跡しない）。
    pub is_physical_key_down: bool,
    /// 修飾キー単体（Shift/Ctrl/Alt/Win）か。
    pub is_modifier_key: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// 自動リピート（直前もキーが押されていた）か。
    pub was_down: bool,
    /// エンジンが消費したか。
    pub consumed: bool,
    /// ADR-189 の固定セット（`shadow_action`）を持つか。従来の経路に任せる。
    pub has_shadow_action: bool,
    /// 同期キー（`sync_direction`）か。従来の経路に任せる。
    pub has_sync_direction: bool,
}

/// 打鍵ごとの予測・追跡を行うか。`None`=行わない、`Some(passive_rule_eligible)`=行う。
///
/// - 表が持つキー（モードキー、Space/Esc/Enter/BS等）: エンジンが消費せず IME へ通したときだけ。
///   `shadow_action`・`sync_direction` 付きは従来の経路に任せる。
/// - 表に無いキー（文字キー）: エンジンが消費しても（ローマ字を IME へ再注入して入力中にするため）
///   変換中の段階を戻す追跡だけを更新する。
/// - 修飾付きは [`modifiers_suppress_prediction`] で抑止する。
///
/// `passive_rule_eligible`（ADR-211 決定2）は、表に無い受動のキー（プリセットの F13）の規則を当ててよい打鍵。
/// 表のキーの除外は `in_table` のときだけなので、表に無いキーではここで明示する。自動リピート・エンジンが
/// 消費した打鍵・`shadow_action`/`sync_direction` 付き・修飾付き（Shift も）は当てない。
#[must_use]
pub fn plan_key_effect_track(f: &KeyTrackFacts) -> Option<bool> {
    if !f.is_physical_key_down || f.is_modifier_key {
        return None;
    }
    let in_table = TableKey::from_vk(f.vk).is_some();
    if modifiers_suppress_prediction(in_table, f.ctrl, f.alt, f.shift, f.win) {
        // Shift+変換（ATOKで開閉トグルではない）やCtrl+文字（ショートカット）は「素のキー」の結果と違う。
        return None;
    }
    let delegated = f.consumed || f.has_shadow_action || f.has_sync_direction;
    if in_table && delegated {
        return None;
    }
    let any_modifier = f.ctrl || f.alt || f.shift || f.win;
    Some(!in_table && !f.was_down && !delegated && !any_modifier)
}

/// ADR-188 案2: FSM が打鍵の前に IME へ送り直すモードキー（保留中の親指の単独タップ）を、送る順に返す。
///
/// 対象は打鍵を通す決定（`PassThroughWith`）の `SendKeys` にある `KeyAction::Key` のうち、
/// 通過マーク・直接観測の窓を開く対象（[`crate::vk::is_followed_mode_key`]）——executor が送出時に窓を開き直すキー
/// （`runtime/executor.rs::dispatch_effect`）と同じ判定。executor は `effects` を送ってから打鍵を再注入するので、
/// IME には ここで返すキー → 打鍵 の順に届く。
///
/// 打鍵を消費する決定（`Consume`）は対象にしない: 表のキーはそもそも予測しない（`plan_key_effect_track`）、
/// 文字キーは段階の追跡だけで、送り直したキーは従来どおり観測に任せる（予測を付けると c353bcbb のガードで観測を捨てる）。
#[must_use]
pub fn fsm_resent_mode_keys(decision: &awase::engine::Decision) -> Vec<u16> {
    use awase::engine::{Decision, Effect, InputEffect};
    use awase::types::KeyAction;
    let Decision::PassThroughWith { effects } = decision else {
        return Vec::new();
    };
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Input(InputEffect::SendKeys(actions)) => Some(actions),
            _ => None,
        })
        .flatten()
        .filter_map(|action| match action {
            KeyAction::Key(vk) if crate::vk::is_followed_mode_key(*vk) => Some(vk.0),
            _ => None,
        })
        .collect()
}

/// `config1.db`から作ったキーマップのキャッシュ（打鍵ごとの同期fs読み取り+パースを避ける。
/// レビュー指摘A-B1）。`RECHECK_MS`ごとに、ファイルの版（更新時刻+長さ）だけを問い合わせ、
/// 変わったときだけ読み直す。判定は純関数で、fs/時計は呼び出し側が渡す。
#[derive(Debug, Default)]
pub struct KeymapCache {
    checked_at_ms: Option<u64>,
    stamp: Option<(u64, u64)>,
    keymap: Option<KeyEffectKeymap>,
}

impl KeymapCache {
    /// 版の再確認の間隔。キーマップの変更（GJI設定画面）は打鍵より遅い操作なので数秒遅れてよい。
    pub const RECHECK_MS: u64 = 2000;

    /// 再確認・読み直しをせず、キャッシュ済みのキーマップだけを返す（診断用）。
    #[must_use]
    pub const fn peek(&self) -> Option<&KeyEffectKeymap> {
        self.keymap.as_ref()
    }

    /// キャッシュしたキーマップを返す。再確認の時刻なら`stamp`（更新時刻+長さ。読めなければ`None`）で
    /// 版を確かめ、初回または版が変わったときだけ`load`で読み直す。`load`は`None`（GJI未導入・
    /// 未対応プリセット）も正常系として保持する。
    pub fn get(
        &mut self,
        now_ms: u64,
        stamp: impl FnOnce() -> Option<(u64, u64)>,
        load: impl FnOnce() -> Option<KeyEffectKeymap>,
    ) -> Option<&KeyEffectKeymap> {
        let first = self.checked_at_ms.is_none();
        let due = self
            .checked_at_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= Self::RECHECK_MS);
        if due {
            self.checked_at_ms = Some(now_ms);
            let now_stamp = stamp();
            if first || now_stamp != self.stamp {
                self.stamp = now_stamp;
                self.keymap = load();
            }
        }
        self.keymap.as_ref()
    }
}

/// Mozc `SessionKeymap`: `NONE=-1, CUSTOM=0, ATOK=1, MSIME=2`（`awase-gji-config`の定数と同じ値）。
const SESSION_KEYMAP_NONE: i64 = -1;
const SESSION_KEYMAP_ATOK: i64 = 1;
const SESSION_KEYMAP_MSIME: i64 = 2;

impl KeyEffectKeymap {
    /// `config1.db`の`session_keymap`（不在=`None`）・`custom_keymap_table`・`overlay_keymaps`から作る。
    /// プリセットがATOK/MSIME（不在/NONEはWindows版GJIの既定でMSIME相当）以外（CUSTOM・MOBILE等）は
    /// 基準の同梱表が無いため`KeymapPreset::Custom`にする（ADR-195段階4 B3対応、以前は`None`を
    /// 返して予測自体を諦めていたが、学習済み表があればこの構成でも`predict_with_override`経由で
    /// 使えるようにするため、常に`Some`を返すようにした）。
    #[must_use]
    pub fn from_config(
        session_keymap: Option<i64>,
        custom_table: Option<String>,
        overlay_keymaps: &[i64],
    ) -> Option<Self> {
        let preset = match session_keymap {
            Some(SESSION_KEYMAP_ATOK) => KeymapPreset::Atok,
            None | Some(SESSION_KEYMAP_NONE | SESSION_KEYMAP_MSIME) => KeymapPreset::MsIme,
            Some(_) => KeymapPreset::Custom,
        };
        let fingerprint = awase_keymap_learn::fingerprint::gji_keymap_fingerprint(
            session_keymap,
            custom_table.as_deref(),
            overlay_keymaps,
        );
        Some(Self {
            preset,
            session_keymap,
            custom_table,
            overlay_keymaps: overlay_keymaps.to_vec(),
            henkan_reassigned: false,
            muhenkan_reassigned: false,
            henkan_toggle: false,
            muhenkan_toggle: false,
            msime_compat_mode: None,
            legacy_table_unknown: false,
            legacy_cells: Vec::new(),
            legacy_custom_on: false,
            fingerprint,
        })
    }

    /// `config1.db`の読み取り結果から作る（ADR-199 決定6-3・決定8 (ii)）。ファイルが**無い**
    /// （`NotFound`）ときは、Mozcがファイル不在を既定設定（Windowsでは`session_keymap = MSIME`）と
    /// して扱うのに揃えて既定のキーマップ（`from_config(None, None, &[])`）を返す。読めない・
    /// パースできないときは`None`（不明。役割を能動側へ倒さない）。
    #[must_use]
    pub fn from_config1_db_read(read: std::io::Result<Vec<u8>>) -> Option<Self> {
        match read {
            Ok(bytes) => {
                let raw = awase_gji_config::wire::parse_top_level(&bytes)?;
                Self::from_config(
                    raw.session_keymap,
                    raw.custom_keymap_table,
                    &raw.overlay_keymaps,
                )
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Self::from_config(None, None, &[])
            }
            Err(_) => None,
        }
    }

    /// このGJIのキーマップで、`vk`（無修飾の打鍵）が持つ役割（ADR-199 決定4）。GJIの設定から
    /// 逆算するだけで、学習表による狭め（決定6-2）・明示configとの重なり（決定8）は呼び出し側。
    /// Microsoft IME本体のキーマップ（`MsImeNative`）では`None`（[`Self::msime_native_key_role`]が別の規則）。
    #[must_use]
    pub fn gji_key_role(&self, vk: u16) -> Option<awase_gji_config::role::KeyRole> {
        use crate::vk::VkCodeExt;
        if matches!(self.preset, KeymapPreset::MsImeNative) {
            return None;
        }
        // ADR-202 決定1: 0x19（Alt+半角/全角）は GJI の TSF 経路で `Hankaku/Zenkaku` 行に従い `Kanji` 行は見ない
        // （実機確認、T1(b)）ので、半角/全角（0xF4）と同じ行から求める。
        let lookup_vk = if vk == 0x19 { 0xF4 } else { vk };
        let vk_name = awase_gji_config::role::ROLE_CANDIDATE_VK_NAMES
            .iter()
            .find(|name| awase::types::VkCode::from_name(name).is_some_and(|v| v.0 == lookup_vk))?;
        awase_gji_config::role::key_role(
            self.session_keymap,
            self.custom_table.as_deref(),
            &self.overlay_keymaps,
            vk_name,
        )
    }

    /// Microsoft IME本体のキーマップで、`vk`（無修飾の打鍵）が持つ役割（[`Self::gji_key_role`]のMS-IME本体版、
    /// ADR-199 T17）。GJIのキーマップ（`MsImeNative`以外）では`None`。
    ///
    /// - 半角/全角（0xF3/0xF4）: 仕様固定トグル（決定6-4）。互換モード（[`Self::msime_compat_mode`]相当）が
    ///   `Some(true)`なら受動（決定17・T13）。
    /// - 無変換/変換（0x1C/0x1D）: マスタースイッチ有効かつ値==2（トグル、T12）のときだけ`Some(ImeToggle)`（ADR-199 T17 Phase 4、決定16）。
    ///   互換モード`Some(true)`は値が効かない（T12）ので受動。値0/1/3・値なしは受動。入力中・変換中・候補窓でも除外しない
    ///   （所有者決定 2026-09-29: 未確定文字列を捨ててよい）。実際の発火は ADR-206 の role_open_action（単独タップが Passthrough のときだけ。Suppress は IME を動かさない）。
    /// - F13〜F24・その他: 常に`None`（受動）。
    #[must_use]
    pub fn msime_native_key_role(&self, vk: u16) -> Option<awase_gji_config::role::KeyRole> {
        use crate::vk::{VK_DBE_DBCSCHAR, VK_DBE_SBCSCHAR};
        use awase_gji_config::role::KeyRole;
        if !matches!(self.preset, KeymapPreset::MsImeNative) {
            return None;
        }
        if vk == VK_DBE_SBCSCHAR.0 || vk == VK_DBE_DBCSCHAR.0 {
            return (self.msime_compat_mode != Some(true)).then_some(KeyRole::ImeToggle);
        }
        let thumb_toggle = if vk == 0x1D {
            self.muhenkan_toggle
        } else if vk == 0x1C {
            self.henkan_toggle
        } else {
            false
        };
        (thumb_toggle && self.msime_compat_mode != Some(true)).then_some(KeyRole::ImeToggle)
    }

    /// Microsoft IME本体のキーマップ。`assignment_enabled`は`IsKeyAssignmentEnabled == 1`、`henkan`/`muhenkan`は
    /// `KeyAssignmentHenkan`/`KeyAssignmentMuhenkan`の値。ADR-199 T12（2026-09-26実機確認）で
    /// 0=IME-オン・1=IME-オフ・2=トグル・3=既定〈無変換=かな切替/変換=再変換〉と確定した
    /// （不在は未設定=`None`）。値0/3の実機的意味（予測への影響）はまだ確認できていないため、
    /// `assignment_enabled`かつ明示値があれば一律に予測しない（安全側、M5・決定C R3）。
    /// `compat_mode`は`msime_legacy_keymap::read_legacy_compat_mode_enabled()`の結果をそのまま渡す
    /// （決定17・T13）。マスタースイッチが無効なら割り当ては効かない（既定のキー設定）。
    /// Ctrl+Space/Shift+Spaceは修飾キー付きなので`modifiers_suppress_prediction`が抑止する。
    #[must_use]
    pub fn for_msime_native(
        assignment_enabled: bool,
        henkan: Option<u32>,
        muhenkan: Option<u32>,
        compat_mode: Option<bool>,
    ) -> Self {
        Self::for_msime_native_with_legacy(
            assignment_enabled,
            henkan,
            muhenkan,
            compat_mode,
            MsImeKeystyle::Absent,
            None,
        )
    }

    /// [`Self::for_msime_native`]に、旧UIのキーテンプレート(`keystyle`)の読み取り結果を足したもの
    /// (ADR-254)。`keystyle`が`Absent`/`Natural`(大多数の利用者)なら[`Self::for_msime_native`]と
    /// 完全に同じ(予測も指紋も)。`legacy_hash`は、`Custom`ならレジストリの表(`key`・`S*key`)の
    /// ハッシュ、`Named`ならテンプレート名のハッシュ(ATOK と VJE を指紋で区別するため)。
    /// 読めなければ`None`。
    #[must_use]
    pub fn for_msime_native_with_legacy(
        assignment_enabled: bool,
        henkan: Option<u32>,
        muhenkan: Option<u32>,
        compat_mode: Option<bool>,
        keystyle: MsImeKeystyle,
        legacy_hash: Option<u64>,
    ) -> Self {
        let reassigned = |v: Option<u32>| assignment_enabled && v.is_some();
        let legacy_table_unknown = keystyle.disables_native_prediction(compat_mode);
        Self {
            preset: KeymapPreset::MsImeNative,
            session_keymap: None,
            custom_table: None,
            overlay_keymaps: Vec::new(),
            henkan_reassigned: reassigned(henkan),
            muhenkan_reassigned: reassigned(muhenkan),
            henkan_toggle: assignment_enabled && henkan == Some(2),
            muhenkan_toggle: assignment_enabled && muhenkan == Some(2),
            msime_compat_mode: compat_mode,
            legacy_table_unknown,
            legacy_cells: Vec::new(),
            legacy_custom_on: matches!(keystyle, MsImeKeystyle::Custom)
                && compat_mode == Some(true),
            // 指紋に混ぜるのは、予測を止める構成のときだけ(NATURAL・不在・互換 OFF の Custom は
            // 従来と同じ指紋のまま。互換 OFF の Custom の表は新エンジンが読まないので、編集しても
            // 学習表を失効させない)。
            fingerprint:
                awase_keymap_learn::fingerprint::msime_native_keymap_fingerprint_with_legacy(
                    assignment_enabled,
                    henkan,
                    muhenkan,
                    legacy_table_unknown.then(|| (keystyle.tag(), legacy_hash)),
                ),
        }
    }

    /// 止める構成のうち「互換 ON の`Custom`」に、`Custom`の表から作った無変換/変換のセルを渡す(ADR-254 第二段)。
    /// それ以外の構成(名前付き・未知・止めない構成)では何もしない(セルは捨てる)。セルは指紋に影響しない
    /// (指紋は表のハッシュを既に含む)。
    #[must_use]
    pub fn with_legacy_custom_cells(mut self, cells: Vec<Cell>) -> Self {
        if self.legacy_custom_on {
            self.legacy_cells = cells;
        }
        self
    }

    /// 旧UIのキーテンプレートと互換モードの組み合わせで`MSIME_NATIVE`の予測を止めているか
    /// (ADR-254。不具合報告・診断用)。
    #[must_use]
    pub const fn legacy_table_unknown(&self) -> bool {
        self.legacy_table_unknown
    }

    /// overlay（`overlay_keymaps`）が1つでもあるか。無変換/変換は overlay
    /// `HENKAN_MUHENKAN_TO_IME_ON_OFF` が上書きしうるので予測しない。
    #[must_use]
    pub const fn has_overlay(&self) -> bool {
        !self.overlay_keymaps.is_empty()
    }

    /// 学習表の陳腐化検出に使う、このキーマップの指紋（生の入力から作ったもの）。
    #[must_use]
    pub const fn fingerprint(&self) -> awase_keymap_learn::persist::Fingerprint {
        self.fingerprint
    }

    /// このキーマップでの、`vk`の打鍵の予測。カスタム表がそのキーの行を持つ、または overlay がある
    /// （無変換/変換は overlay `HENKAN_MUHENKAN_TO_IME_ON_OFF` が上書きしうる）ときは`None`。
    #[must_use]
    pub fn predict(&self, vk: u16, input: &PredictInput) -> Option<Prediction> {
        // ガード(custom_table/overlay/レジストリ再割り当ての除外)は`predict_with_override`と
        // 完全に同じでなければならない。2箇所に手書きすると片方だけ直る事故が起きうるため
        // (`.claude/rules/fix-requires-evidence.md`の「キー選択」再発ファミリー)、こちらへ委譲する。
        self.predict_with_override(vk, input, None)
    }

    /// [`Self::predict`]と同じだが、`override_table`が`Some`なら同梱表の代わりにそれを引く
    /// （ADR-195段階4。検証済みの学習済み表、[`crate::state::key_effect_runtime`]が用意する）。
    #[must_use]
    pub fn predict_with_override(
        &self,
        vk: u16,
        input: &PredictInput,
        override_table: Option<&[Cell]>,
    ) -> Option<Prediction> {
        // ADR-209 決定1・3: 読めない窓（TSF）では、GJI は MS-IME プリセットの閉状態の変換で IME を開く
        // （実機/CIで確認）。学習は素の EDIT で測るので、窓別の規則を学習表より先に引く（ADR-196の例外）。
        if let Some(prediction) = self.unreadable_window_prediction(vk, input) {
            return Some(prediction);
        }
        // ADR-211 決定1: 表に無い受動のキー（プリセットの F13）は、閉状態から開く。学習表・打ち切りより先（ADR-209 と同じ理由）。
        if let Some(prediction) = self.passive_open_key_prediction(vk, input) {
            return Some(prediction);
        }
        // ADR-247: CUSTOM の表にある F13〜F24 の行（半角英数への SET など）は、学習表にセルが無いので表の行から予測する。
        if let Some(prediction) = self.custom_f_key_prediction(vk, input) {
            return Some(prediction);
        }
        // ADR-195段階4 B3対応: 学習済み表にこのキー・状態の答えがあれば、custom_table/overlay/
        // レジストリ再割り当てのガードより先にそれを使う。これらのガードは「同梱表はユーザーの
        // 独自割り当てを知らないので予測しない」という安全策であり、学習済み表はまさにその
        // 独自割り当てを実測したものなので、ガードの理由が最初から当てはまらない。ガードを先に
        // 通すと、ユーザーが学習させたかったキー（カスタムキーマップで上書きしたキーそのもの）
        // だけが黙って予測対象から外れてしまう。
        if let Some(table) = override_table {
            if let Some(prediction) = predict_in_table(table, vk, input) {
                return Some(prediction);
            }
        }
        // ADR-254 決定1: 旧UIのキーテンプレート(`keystyle`)が既定でなく、同梱表`MSIME_NATIVE`の予測が
        // 当たらない構成(互換 ON の Custom・名前付きスタイル・未知)では予測しない。**学習表の参照の後**に
        // 置く(学習表があればそれを使う、ADR-196)。窓別の規則(上の2つ)は MS-IME 本体では元から`None`。
        if self.legacy_table_unknown {
            // ADR-254 第二段: 互換 ON の Custom は、無変換/変換だけ、表から作ったセルで予測する。
            // それ以外のキー(文字キーなど)は予測しない(`predict_in_table`は文字キーの追跡や、読めない窓での
            // 種の反映をするので、止める構成では呼ばない)。
            return if matches!(vk, 0x1C | 0x1D) && !self.legacy_cells.is_empty() {
                predict_in_table(&self.legacy_cells, vk, input)
            } else {
                None
            };
        }
        // ADR-209 決定4: GJI はプリセット（ATOK/MS-IME/不在/NONE）のとき`custom_keymap_table`を読まない
        // （ADR-186 決定2(c)、実機X1）ので、古い表の行を理由に打ち切らない。CUSTOM等のときだけ従来どおり。
        if matches!(self.preset, KeymapPreset::Custom)
            && self
                .custom_table
                .as_deref()
                .is_some_and(|t| custom_table_overrides(t, vk))
        {
            return None;
        }
        if (self.has_overlay() && matches!(vk, 0x1C | 0x1D))
            || (self.henkan_reassigned && vk == 0x1C)
            || (self.muhenkan_reassigned && vk == 0x1D)
        {
            return None;
        }
        predict(self.preset, vk, input)
    }

    /// ADR-188 案2: この打鍵の前に FSM が IME へ送り直すモードキー（`resent`、[`fsm_resent_mode_keys`]）があるとき、
    /// その効果を先に重ねてから `vk` を予測する。`resent` が空なら [`Self::predict_with_override`] と同じ。
    ///
    /// 素通し設定で無変換（親指）を押したまま別のキーを押すと、FSM は保留中の親指を単独タップとして送り直してから
    /// そのキーを通す（IME には 無変換→キー の順に届く）。打鍵前の belief だけで予測すると、送り直した無変換の効果が
    /// 抜けて予測が外れる（CI run 38065774507: ATOK の変換は「かな→閉」と予測されたが、実際は無変換で閉じてから
    /// 変換で開く。MS-IME プリセットの英数は「かな→半角英数」と予測されたが、実際は無変換で全角カタカナになってから
    /// 英数でひらがなに戻る）。予測が付いた打鍵は窓内の読みで覆さない（c353bcbb）ので、外れた予測がそのまま残っていた。
    ///
    /// 効果の合成: 開閉・入力モードは後の打鍵の値が前の値を上書きする（`Some` が勝つ）。追跡は最後の打鍵のもの。
    /// 送り直したキー、または `vk` のどれかが予測できないときは `None`（途中の状態が分からないので観測に任せる）。
    /// 送り直したキーの後に開いていて変換モードが分からない（セルの押下後の変換モードが不明）ときも `None`。
    /// 送り直したキーには表に無い受動キーの規則（`passive_rule_eligible`、ADR-211）を当てない（安全側）。
    #[must_use]
    pub fn predict_after_resent(
        &self,
        resent: &[u16],
        vk: u16,
        input: &PredictInput,
        override_table: Option<&[Cell]>,
    ) -> Option<Prediction> {
        let mut state = *input;
        let mut effect = PredictedEffect {
            open: None,
            mode: None,
        };
        for &resent_vk in resent {
            let step_input = PredictInput {
                passive_rule_eligible: false,
                ..state
            };
            let step = self.predict_with_override(resent_vk, &step_input, override_table)?;
            effect = effect.overlaid_by(step.effect);
            state = step_input.after(step);
            // 送り直したキーの後に開いていて変換モードが分からない（表のセルの押下後の変換モードが不明）ときは、
            // 次の打鍵を打鍵前の観測（`conv_raw`）や入力モードから引くと推測になるので予測しない。
            // 実測(CI run 38094299141 sc-armc-gji-msimepreset-passthru): 全角カタカナで無変換(押下後不明)→英数を、
            // 打鍵前のかなで引いて「半角英数」と予測し外れた(実際はかな系のまま)。
            if state.open && state.track.conv.is_none() {
                return None;
            }
        }
        let last = self.predict_with_override(vk, &state, override_table)?;
        Some(Prediction {
            effect: effect.overlaid_by(last.effect),
            track: last.track,
        })
    }

    /// ADR-209 決定1〜3: 読めない窓で、GJI の MS-IME プリセット（`session_keymap`が不在/NONE/MSIME）の
    /// 閉状態の無修飾の変換（0x1C）は IME を開く。開閉だけを予測し、モード・段階は変えない
    /// （`Unknown`のときだけ既存の種を使う）。overlay・レジストリ再割り当てがあるときは当てない。
    fn unreadable_window_prediction(&self, vk: u16, input: &PredictInput) -> Option<Prediction> {
        // Microsoft IME 本体のキーマップも`session_keymap: None`を持つが、GJI の MS-IME プリセットの規則ではない（ADR-211 N1）。
        if matches!(self.preset, KeymapPreset::MsImeNative) {
            return None;
        }
        let msime_like = matches!(
            self.session_keymap,
            None | Some(SESSION_KEYMAP_NONE | SESSION_KEYMAP_MSIME)
        );
        if !input.unreadable
            || !msime_like
            || vk != 0x1C
            || input.open
            || self.has_overlay()
            || self.henkan_reassigned
        {
            return None;
        }
        let mode = matches!(input.mode, InputModeState::Unknown).then(kana_mode);
        Some(Prediction {
            effect: PredictedEffect {
                open: Some(true),
                mode,
            },
            track: input.track,
        })
    }

    /// ADR-211 決定1・2: GJI の MS-IME/MOBILE プリセット（`role::passive_open_vk_names_outside_table`）で、表に無い
    /// 受動のキー（F13）の閉状態の無修飾の打鍵は IME を開く。開閉だけを予測し、モード・段階は変えない
    /// （`Unknown`のときだけ既存の種）。Microsoft IME 本体・overlay ありでは当てない。
    fn passive_open_key_prediction(&self, vk: u16, input: &PredictInput) -> Option<Prediction> {
        use crate::vk::VkCodeExt;
        if !input.passive_rule_eligible
            || input.open
            || matches!(self.preset, KeymapPreset::MsImeNative)
            || self.has_overlay()
        {
            return None;
        }
        let names = awase_gji_config::role::passive_open_vk_names_outside_table(
            self.session_keymap,
            self.custom_table.as_deref(),
        );
        if !names
            .iter()
            .any(|n| awase::types::VkCode::from_name(n).is_some_and(|v| v.0 == vk))
        {
            return None;
        }
        let mode = matches!(input.mode, InputModeState::Unknown).then(kana_mode);
        Some(Prediction {
            effect: PredictedEffect {
                open: Some(true),
                mode,
            },
            track: input.track,
        })
    }

    /// ADR-247: GJI の CUSTOM 表にある F13〜F24（無修飾）の行から、打鍵の効果（開閉と入力モード）を予測する。
    /// 学習表・同梱表は F13〜F24 のセルを持たないので、表が無いと予測が付かず、読めない窓では belief がずれたまま残る
    /// （英数=F16 で GJI が半角英数になっても Engine が ON のまま）。変換中（`Conversion`）は段階の追跡に依存するので
    /// 予測しない。行が無い・相対トグル系・未知のコマンドは `None`（観測に委ねる）。
    fn custom_f_key_prediction(&self, vk: u16, input: &PredictInput) -> Option<Prediction> {
        use awase_gji_config::command::GjiCompositionMode as Mode;
        if !input.passive_rule_eligible
            || !(0x7C..=0x87).contains(&vk)
            || !matches!(self.preset, KeymapPreset::Custom)
            || self.has_overlay()
            || matches!(
                input.track.stage,
                Stage::ConvSpace | Stage::ConvHenkan | Stage::ConvMuhenkan
            )
        {
            return None;
        }
        let name = format!("VK_F{}", vk - 0x6F);
        let effect = awase_gji_config::role::custom_key_mode_effect(
            self.session_keymap,
            self.custom_table.as_deref(),
            &[],
            &name,
            input.open,
            input.composing,
        )?;
        let conv = effect.mode.and_then(|m| match m {
            Mode::HalfAlphanumeric => Some(Conv::C10),
            Mode::Hiragana => Some(Conv::C19),
            Mode::FullKatakana => Some(Conv::C1B),
            // 全角英数・半角カナは `Conv` が表せない（追わない）。
            Mode::FullAlphanumeric | Mode::HalfKatakana => None,
        });
        let open_after = effect.open.unwrap_or(input.open);
        if effect.open.is_none() && conv.is_none() {
            return None;
        }
        // 閉じる効果は段階・変換モードの追跡を捨てる。モードを SET する効果は追跡する変換モードを更新する。
        let track = if open_after {
            KeyTrack {
                conv: conv.or(input.track.conv),
                stage: input.track.stage,
            }
        } else {
            KeyTrack {
                conv: None,
                stage: Stage::None,
            }
        };
        let mode = conv
            .and_then(|cv| mode_effect(input.mode, cv))
            .or_else(|| matches!(input.mode, InputModeState::Unknown).then(kana_mode));
        Some(Prediction {
            effect: PredictedEffect {
                open: (open_after != input.open).then_some(open_after),
                mode,
            },
            track,
        })
    }

    /// この構成の`preset`。[`crate::state::key_effect_runtime`]が同梱表との突き合わせに使う。
    #[must_use]
    pub const fn preset(&self) -> KeymapPreset {
        self.preset
    }

    /// カスタム表・overlay・レジストリ再割り当てのいずれも無い、同梱3種のいずれかとそのまま一致する
    /// 構成か。ADR-195段階4の「同梱表と同じ構成ならセル突き合わせで縮退検出」判定に使う——
    /// カスタム構成では学習表が同梱表と食い違うのが正常なので、この判定が`false`のときは
    /// セル突き合わせ自体を行わない。
    #[must_use]
    pub const fn is_unmodified_bundled_config(&self) -> bool {
        // KeymapPreset::Customはそもそも基準となる同梱表が無い構成なので、custom_table等が
        // たまたま空でも「同梱表そのまま」とは判定しない(ADR-195段階4 B3対応)。
        !matches!(self.preset, KeymapPreset::Custom)
            && self.custom_table.is_none()
            && !self.has_overlay()
            && !self.henkan_reassigned
            && !self.muhenkan_reassigned
    }
}

/// カスタムキーマップTSV（`custom_keymap_table`）が、このVKのキーイベントの行を持つか。
/// キー名→VK の写像は`awase_gji_config::keymap::mozc_key_vk_names`（ADR-199 T2 で一本化）。
#[must_use]
pub fn custom_table_overrides(custom_table: &str, vk: u16) -> bool {
    use crate::vk::VkCodeExt;
    custom_table.lines().any(|line| {
        let mut cols = line.split('\t');
        let (_status, Some(key)) = (cols.next(), cols.next()) else {
            return false;
        };
        awase_gji_config::keymap::mozc_key_vk_names(key)
            .iter()
            .any(|name| awase::types::VkCode::from_name(name).is_some_and(|v| v.0 == vk))
    })
}

/// ひらがなキー（F2 / `VK_DBE_HIRAGANA`）が、開いた IME を**かなへ SET する**キーか（ADR-245 決定8・10）。
///
/// 半角英数トグルの戻り待ちを戻った窓で復元するとき、GJI のプリセットによっては F2 が純粋な**トグル**
/// （ATOK: 0x19→0x10、0x10→0x19）で、離れている間に窓の状態が変わっていると復元の F2 が逆に働く。
/// 送ってよいかを予測器に問う。2 問: (i) 開・半角英数(conv 0x10)で押したらかなになるか、
/// (ii) 開・ひらがな(conv 0x19)で押してもかなのままか。
///
/// - 両方かな → `Some(true)`（MS-IME プリセット。どの状態でも F2 はかなへ SET）。
/// - (ii) が英数になる（トグル） → `Some(false)`。
/// - 予測なし（Custom・MsImeNative 等の CannotPredict、表に無いセル）、または (i) が英数のまま
///   （想定外） → `None`。
///
/// `PredictInput` の `composing = false`（復元の時点で入力中の文字列は無い）と `passive_rule_eligible = false`
/// （F2 は表のキー）はここで決める。`unreadable` と `learned`（検証済みの学習済み表）は呼び出し側の事実。
#[must_use]
pub fn hiragana_key_is_set(
    keymap: &KeyEffectKeymap,
    learned: Option<&[Cell]>,
    unreadable: bool,
) -> Option<bool> {
    const VK_HIRAGANA: u16 = 0xF2;
    // 押した後に変換モードがかな(NATIVE)か。追跡(`track.conv`)が捨てられたセルは、効果の入力モードで見る。
    let ask = |mode: InputModeState, conv: Conv| -> Option<bool> {
        let input = PredictInput {
            open: true,
            mode,
            conv_raw: None,
            composing: false,
            track: KeyTrack {
                conv: Some(conv),
                stage: Stage::None,
            },
            unreadable,
            passive_rule_eligible: false,
        };
        let p = keymap.predict_with_override(VK_HIRAGANA, &input, learned)?;
        Some(match (p.track.conv, p.effect.mode) {
            (Some(after), _) => after.is_native(),
            (None, Some(InputModeState::ObservedEisu)) => false,
            (None, Some(_)) => true,
            (None, None) => conv.is_native(),
        })
    };
    let after_from_eisu = ask(InputModeState::ObservedEisu, Conv::C10)?;
    let after_from_hiragana = ask(kana_mode(), Conv::C19)?;
    match (after_from_eisu, after_from_hiragana) {
        (true, true) => Some(true),
        (_, false) => Some(false),
        (false, true) => None,
    }
}

#[cfg(test)]
mod tests {
    /// 通過マーク（`kp_stage_mode_key_follow`、BUG-157の`desired_open`の揃え）を立てるのは`is_followed_mode_key`のキーだけ。
    /// 予測が開閉を動かすキーがそれと食い違うと、そのキーで動いた`open`は`desired_open`へ揃わず、BUG-157が黙って退行する。
    /// 表のうち通過マークの対象でないキー（Space/Esc/Enter/BS）が開閉を変えるセルを持たないことを、全表で固定する
    /// （将来の格子で「入力中のEscで閉じる」等が学習されたら、ここで気付く。round2 A-N9）。
    /// レビュー指摘(design-patterns-review.md D4/C4): `find()`は`.find(..)`＝**最初の一致**を返すので、
    /// 生成物に矛盾するセル（同じ(open, conv, stage, key)に複数行）が混ざっても黙って先勝ちする。
    /// `key_effect_table_matches_generator`（architecture_guard）は「生成器の出力と一致するか」だけを見ており、
    /// キーの一意性そのものは見ていない。全表でキーが一意であることをここで固定する
    /// （closed セルは`conv`が常に`None`＝ワイルドカードなので、キーは`(open, stage, key)`で比較する。
    /// `conv`フィールドの意味は`Cell`のdoc参照）。
    #[test]
    fn table_cell_keys_are_unique_in_every_preset() {
        for preset in [
            KeymapPreset::Atok,
            KeymapPreset::MsIme,
            KeymapPreset::MsImeNative,
        ] {
            // TableKey/Stage/Conv は Hash を持たないので Vec + 線形探索で十分（表は最大でも数百行）。
            let mut seen: Vec<(bool, Option<Conv>, Stage, TableKey)> = Vec::new();
            for c in table_of(preset) {
                assert!(
                    c.open || c.conv.is_none(),
                    "{preset:?}: 閉セル{:?}のconvはNone(ワイルドカード)のはず、実際={:?}",
                    c.key,
                    c.conv
                );
                let dedup_key = (c.open, c.open.then_some(c.conv).flatten(), c.stage, c.key);
                assert!(
                    !seen.contains(&dedup_key),
                    "{preset:?}: キー{dedup_key:?}が複数行ある(findは先勝ちで矛盾を黙って通す)"
                );
                seen.push(dedup_key);
            }
        }
    }

    #[test]
    fn table_keys_outside_the_followed_mode_keys_never_change_open() {
        for preset in [
            KeymapPreset::Atok,
            KeymapPreset::MsIme,
            KeymapPreset::MsImeNative,
        ] {
            for c in table_of(preset) {
                if matches!(
                    c.key,
                    TableKey::Space | TableKey::Esc | TableKey::Enter | TableKey::Bs
                ) {
                    assert_eq!(
                        c.after_open, c.open,
                        "{preset:?}: {:?} が開閉を変える（通過マークの対象外のキーなので BUG-157 の揃えが効かない）",
                        c.key
                    );
                }
            }
        }
    }

    fn track_facts(vk: u16) -> KeyTrackFacts {
        KeyTrackFacts {
            vk,
            is_physical_key_down: true,
            is_modifier_key: false,
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            was_down: false,
            consumed: false,
            has_shadow_action: false,
            has_sync_direction: false,
        }
    }

    /// 表に無い文字キー（'A'）と、表が持つキー（Space）。
    const VK_CHAR: u16 = 0x41;
    const VK_TABLE: u16 = 0x20;

    #[test]
    fn plan_key_effect_track_skips_non_physical_down_and_modifier_keys() {
        for vk in [VK_CHAR, VK_TABLE] {
            let mut f = track_facts(vk);
            f.is_physical_key_down = false;
            assert_eq!(plan_key_effect_track(&f), None, "KeyUp/注入は追跡しない");
            let mut f = track_facts(vk);
            f.is_modifier_key = true;
            assert_eq!(plan_key_effect_track(&f), None, "修飾キー単体は追跡しない");
        }
    }

    #[test]
    fn plan_key_effect_track_table_key_only_when_passed_through_unmodified() {
        assert_eq!(plan_key_effect_track(&track_facts(VK_TABLE)), Some(false));
        let mut f = track_facts(VK_TABLE);
        f.consumed = true;
        assert_eq!(plan_key_effect_track(&f), None);
        let mut f = track_facts(VK_TABLE);
        f.has_shadow_action = true;
        assert_eq!(plan_key_effect_track(&f), None);
        let mut f = track_facts(VK_TABLE);
        f.has_sync_direction = true;
        assert_eq!(plan_key_effect_track(&f), None);
        let mut f = track_facts(VK_TABLE);
        f.shift = true;
        assert_eq!(
            plan_key_effect_track(&f),
            None,
            "表のキーは Shift 付きで追跡しない"
        );
    }

    #[test]
    fn plan_key_effect_track_char_key_tracks_even_if_consumed_but_rule_gate_is_strict() {
        assert_eq!(plan_key_effect_track(&track_facts(VK_CHAR)), Some(true));
        let mut f = track_facts(VK_CHAR);
        f.consumed = true;
        assert_eq!(
            plan_key_effect_track(&f),
            Some(false),
            "消費しても追跡は更新、規則は当てない"
        );
        let mut f = track_facts(VK_CHAR);
        f.was_down = true;
        assert_eq!(
            plan_key_effect_track(&f),
            Some(false),
            "自動リピートには規則を当てない"
        );
        let mut f = track_facts(VK_CHAR);
        f.has_shadow_action = true;
        assert_eq!(plan_key_effect_track(&f), Some(false));
        let mut f = track_facts(VK_CHAR);
        f.has_sync_direction = true;
        assert_eq!(plan_key_effect_track(&f), Some(false));
        let mut f = track_facts(VK_CHAR);
        f.shift = true;
        assert_eq!(
            plan_key_effect_track(&f),
            Some(false),
            "Shift（大文字）は追跡するが規則は当てない"
        );
        for (c, a, w) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut f = track_facts(VK_CHAR);
            f.ctrl = c;
            f.alt = a;
            f.win = w;
            assert_eq!(
                plan_key_effect_track(&f),
                None,
                "ショートカットは追跡しない"
            );
        }
    }

    #[test]
    fn modifiers_suppress_prediction_for_table_keys_and_shortcuts() {
        // 表のキー: 修飾なしだけ予測する。
        assert!(!modifiers_suppress_prediction(
            true, false, false, false, false
        ));
        for (c, a, sh, w) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert!(
                modifiers_suppress_prediction(true, c, a, sh, w),
                "表のキーは修飾付きで予測しない"
            );
        }
        // 文字キー: Shift（大文字）は追跡する、Ctrl/Alt/Win（ショートカット）は追跡しない。
        assert!(!modifiers_suppress_prediction(
            false, false, false, false, false
        ));
        assert!(!modifiers_suppress_prediction(
            false, false, false, true, false
        ));
        assert!(modifiers_suppress_prediction(
            false, true, false, false, false
        ));
        assert!(modifiers_suppress_prediction(
            false, false, true, false, false
        ));
        assert!(modifiers_suppress_prediction(
            false, false, false, false, true
        ));
    }

    #[test]
    fn keymap_cache_loads_once_and_rechecks_by_stamp() {
        use std::cell::Cell;
        let loads = Cell::new(0u32);
        let stamps = Cell::new(0u32);
        let mut cache = KeymapCache::default();
        let load = || {
            loads.set(loads.get() + 1);
            KeyEffectKeymap::from_config(Some(1), None, &[])
        };
        // 初回は読む。
        assert!(cache
            .get(
                0,
                || {
                    stamps.set(stamps.get() + 1);
                    Some((1, 10))
                },
                load
            )
            .is_some());
        assert_eq!((loads.get(), stamps.get()), (1, 1));
        // 再確認の間隔内（文字キーの連打）はfsに触れない（stampもloadも呼ばない）。
        for t in [1, 500, 1999] {
            assert!(cache
                .get(
                    t,
                    || {
                        stamps.set(stamps.get() + 1);
                        Some((1, 10))
                    },
                    load
                )
                .is_some());
        }
        assert_eq!((loads.get(), stamps.get()), (1, 1), "間隔内はfsを読まない");
        // 間隔を過ぎても版が同じなら読み直さない（statだけ）。
        assert!(cache
            .get(
                2000,
                || {
                    stamps.set(stamps.get() + 1);
                    Some((1, 10))
                },
                load
            )
            .is_some());
        assert_eq!((loads.get(), stamps.get()), (1, 2));
        // 版が変わったら読み直す。
        assert!(cache
            .get(
                4000,
                || {
                    stamps.set(stamps.get() + 1);
                    Some((2, 10))
                },
                load
            )
            .is_some());
        assert_eq!((loads.get(), stamps.get()), (2, 3));
    }

    #[test]
    fn keymap_cache_keeps_absent_keymap_without_rereading() {
        use std::cell::Cell;
        let loads = Cell::new(0u32);
        let mut cache = KeymapCache::default();
        for t in [0, 100, 2500] {
            assert!(cache
                .get(
                    t,
                    || None,
                    || {
                        loads.set(loads.get() + 1);
                        None
                    }
                )
                .is_none());
        }
        assert_eq!(
            loads.get(),
            1,
            "GJI未導入（stamp=None）は版が変わらない限り読み直さない"
        );
    }

    use super::*;

    fn input(open: bool, mode: InputModeState, composing: bool, track: KeyTrack) -> PredictInput {
        PredictInput {
            open,
            mode,
            conv_raw: None,
            composing,
            track,
            unreadable: false,
            passive_rule_eligible: false,
        }
    }

    const ROMAJI: InputModeState = InputModeState::ObservedRomaji;
    const NOTRACK: KeyTrack = KeyTrack {
        conv: None,
        stage: Stage::None,
    };

    /// 読めないアプリ（観測の`composing`が常に偽）でも、文字キーで入力中を追跡し、その後の無変換/Escが
    /// 「入力中」のセルを引く（CI blind: `k`のあとの無変換が「入力中でない」セルを引いてOFFと予測していた）。
    #[test]
    fn typed_char_tracks_typing_without_composing_observation() {
        let eisu = KeyTrack {
            conv: Some(Conv::C10),
            stage: Stage::None,
        };
        // 'k'(0x4B)を開いた状態で打つ。観測(composing)は偽のまま。
        let p = predict(
            KeymapPreset::Atok,
            0x4B,
            &input(true, InputModeState::ObservedEisu, false, eisu),
        )
        .expect("文字キーで追跡が更新される");
        assert_eq!(p.track.stage, Stage::Typing);
        assert_eq!(p.effect.open, None);

        // 続く無変換は「入力中」のセル（ATOKでは入力中の無変換はOFFにならない）を引く。
        let m = predict(
            KeymapPreset::Atok,
            0x1D,
            &input(true, InputModeState::ObservedEisu, false, p.track),
        );
        assert!(
            m.is_none_or(|m| m.effect.open != Some(false)),
            "入力中の無変換は開閉を閉じない（追跡した入力中で引く）: {m:?}"
        );
    }

    /// ATOKの表に`ConvMuhenkan`の行は無い。入力中の無変換で入った変換中の段階の後も、Enter・半角/全角は
    /// 「入力中」の行で予測し、追跡した段階を更新する（予測なしで古い段階が残らない）。
    /// BUG-162 回帰: 入力中の Esc は ATOK 表にセルが無い（保持/破棄が割れて生成時に除外）が、追跡した
    /// 段階（`Typing`）は捨てなければならない。捨てないと、`k`→Esc の後の無変換が「入力中の無変換」と
    /// 誤予測される（実際は入力中でないので IME OFF）。
    #[test]
    fn typing_esc_without_a_table_cell_drops_the_tracked_stage() {
        const EISU: InputModeState = InputModeState::ObservedEisu;
        let mut base = input(true, EISU, false, NOTRACK);
        base.conv_raw = Some(0x10); // 半角英数（実測 baseline の手順2の状態）
                                    // 1. 文字キー（k）: 入力中（Typing）になる。
        let k = predict(KeymapPreset::Atok, 0x4B, &base).expect("文字キーは段階を Typing にする");
        assert_eq!(k.track.stage, Stage::Typing);
        // 2. Esc: 表にセルは無く、開閉・モードは予測しないが、追跡した段階は捨てる。
        let mut after_k = base;
        after_k.track = k.track;
        assert_eq!(
            find_in(
                table_of(KeymapPreset::Atok),
                true,
                Conv::C10,
                Stage::Typing,
                TableKey::Esc
            ),
            None,
            "前提: ATOK 表に（半角英数・入力中・Esc）のセルは無い"
        );
        let esc = predict(KeymapPreset::Atok, 0x1B, &after_k)
            .expect("段階が変わるので追跡の更新だけ返す");
        assert!(esc.effect.is_noop(), "開閉・モードは予測しない: {esc:?}");
        assert_eq!(esc.track.stage, Stage::None);
        // 3. 無変換: 入力中でない無変換として引く（古い Typing の行を引かない）。
        let mut after_esc = base;
        after_esc.track = esc.track;
        let muhenkan = predict(KeymapPreset::Atok, 0x1D, &after_esc);
        let stale = predict(KeymapPreset::Atok, 0x1D, &after_k);
        assert_ne!(
            muhenkan.map(|p| p.effect.open),
            stale.map(|p| p.effect.open),
            "Esc で段階を捨てた後の予測は、古い Typing のままの予測と違う（BUG-162）"
        );
    }

    /// 変換中（`Conv*`）の Esc は保持/破棄が実際に割れる（元の状態へ戻る）ので、追跡は変えない（従来どおり予測なし）。
    #[test]
    fn conversion_stage_esc_without_a_table_cell_keeps_the_previous_behaviour() {
        let conv_space = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::ConvSpace,
        };
        assert!(predict(
            KeymapPreset::Atok,
            0x1B,
            &input(true, ROMAJI, true, conv_space)
        )
        .is_none());
    }

    #[test]
    fn atok_conv_muhenkan_stage_falls_back_to_typing_row() {
        let conv = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::ConvMuhenkan,
        };
        let enter = predict(KeymapPreset::Atok, 0x0D, &input(true, ROMAJI, false, conv))
            .expect("ConvMuhenkanの行が無くても入力中の行で予測する");
        assert_eq!(enter.track.stage, Stage::None, "確定して入力中でなくなる");
        let hz = predict(KeymapPreset::Atok, 0xF3, &input(true, ROMAJI, false, conv))
            .expect("半角/全角も予測する");
        assert_eq!(hz.effect.open, Some(false));
    }

    /// 閉じているときの文字キーは入力中にならない。
    #[test]
    fn typed_char_while_closed_does_not_start_typing() {
        let p = predict(
            KeymapPreset::Atok,
            0x4B,
            &input(false, InputModeState::ObservedRomaji, false, NOTRACK),
        );
        assert!(
            p.is_none_or(|p| p.track.stage == Stage::None),
            "閉のときの文字キーで入力中の段階を作らない: {p:?}"
        );
    }

    #[test]
    fn atok_hiragana_returns_to_hiragana_from_key_entered_halfwidth_alnum() {
        // 実測(grid第2版、変換モードをキーで到達): ATOK ひらがな(0xF2)。キーで入った半角英数(0x10)→ひらがな(0x19)。
        let eisu = KeyTrack {
            conv: Some(Conv::C10),
            stage: Stage::None,
        };
        let p = predict(
            KeymapPreset::Atok,
            0xF2,
            &input(true, InputModeState::ObservedEisu, false, eisu),
        )
        .unwrap();
        assert_eq!(p.effect.open, None);
        assert_eq!(p.effect.mode, Some(kana_mode()));
        assert_eq!(p.track.conv, Some(Conv::C19));
    }

    #[test]
    fn atok_hiragana_is_a_pure_toggle_between_hiragana_and_halfwidth_alnum() {
        // 実測(grid第3版、全状態をキーだけで作る): ATOK ひらがな(0xF2)は 0x19→0x10、0x10→0x19 の純粋なトグル
        // (独立walkの 0x19→0x10 20/20、0x10→0x19 19/19と一致)。第2版はIMMで作った0x19から「不変」と誤っていた。
        let hira = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::None,
        };
        let p = predict(KeymapPreset::Atok, 0xF2, &input(true, ROMAJI, false, hira)).unwrap();
        assert_eq!(p.track.conv, Some(Conv::C10));
        assert_eq!(p.effect.mode, Some(InputModeState::ObservedEisu));
        let eisu = KeyTrack {
            conv: Some(Conv::C10),
            stage: Stage::None,
        };
        let p = predict(
            KeymapPreset::Atok,
            0xF2,
            &input(true, InputModeState::ObservedEisu, false, eisu),
        )
        .unwrap();
        assert_eq!(p.track.conv, Some(Conv::C19));
    }

    #[test]
    fn msime_katakana_from_hiragana_goes_to_fullwidth_katakana() {
        // 実測(grid第3版、MS-IMEプリセット、全状態をキーだけで作る): カタカナ(0xF1)は 0x19→0x1B、ひらがな(0xF2)は 0x1B→0x19。
        // 第2版はIMMで作った0x19から「不変」と誤っていた。
        let hira = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::None,
        };
        let p = predict(KeymapPreset::MsIme, 0xF1, &input(true, ROMAJI, false, hira)).unwrap();
        assert_eq!(p.track.conv, Some(Conv::C1B));
        let kata = KeyTrack {
            conv: Some(Conv::C1B),
            stage: Stage::None,
        };
        let p = predict(KeymapPreset::MsIme, 0xF2, &input(true, ROMAJI, false, kata)).unwrap();
        assert_eq!(p.track.conv, Some(Conv::C19));
    }

    #[test]
    fn atok_hiragana_in_direct_input_changes_nothing() {
        // 実測: IME OFFでひらがなを押しても開かない・conv不変。
        let p = predict(
            KeymapPreset::Atok,
            0xF2,
            &input(false, ROMAJI, false, NOTRACK),
        )
        .unwrap();
        assert!(p.effect.is_noop());
    }

    #[test]
    fn atok_muhenkan_and_henkan_close_when_open_and_idle_and_open_when_closed() {
        for vk in [0x1D, 0x1C] {
            let closed = predict(
                KeymapPreset::Atok,
                vk,
                &input(false, ROMAJI, false, NOTRACK),
            )
            .unwrap();
            assert_eq!(closed.effect.open, Some(true), "vk=0x{vk:02X}");
            let open =
                predict(KeymapPreset::Atok, vk, &input(true, ROMAJI, false, NOTRACK)).unwrap();
            assert_eq!(open.effect.open, Some(false), "vk=0x{vk:02X}");
        }
    }

    #[test]
    fn conversion_stage_is_tracked_from_key_history_and_changes_what_esc_does() {
        // 変換(入力中)→変換中(ConvHenkan)へ入る。その後のEscは入力中に戻るだけ(保持)。
        let typing = input(true, ROMAJI, true, NOTRACK);
        let p1 = predict(KeymapPreset::Atok, 0x1C, &typing).unwrap();
        assert_eq!(p1.track.stage, Stage::ConvHenkan);
        let p2 = predict(
            KeymapPreset::Atok,
            0x1B,
            &input(true, ROMAJI, true, p1.track),
        )
        .unwrap();
        assert_eq!(
            p2.track.stage,
            Stage::None,
            "変換中のEscは入力中(Typing)に戻る"
        );
        // 入力中(Typing)のEscは破棄(入力中でなくなる)。
        let p3 = predict(KeymapPreset::Atok, 0x1B, &typing).unwrap();
        assert_eq!(p3.track.stage, Stage::None);
    }

    #[test]
    fn typing_a_character_leaves_the_conversion_stage() {
        let conv = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::ConvSpace,
        };
        let p = predict(KeymapPreset::Atok, 0x41, &input(true, ROMAJI, true, conv)).unwrap();
        assert_eq!(p.track.stage, Stage::Typing);
        assert!(p.effect.is_noop());
        // 追跡が段階なしでも、開いている間の文字キーは入力中(Typing)を追跡する(読めないアプリでは観測が無い)。
        let p = predict(
            KeymapPreset::Atok,
            0x41,
            &input(true, ROMAJI, true, NOTRACK),
        )
        .unwrap();
        assert_eq!(p.track.stage, Stage::Typing);
        assert!(p.effect.is_noop());
        // 追跡が既に入力中なら何も更新しない(予測なし)。
        let typing = KeyTrack {
            conv: None,
            stage: Stage::Typing,
        };
        assert_eq!(
            predict(KeymapPreset::Atok, 0x41, &input(true, ROMAJI, true, typing)),
            None
        );
    }

    #[test]
    fn unknown_mode_is_seeded_with_kana_so_prediction_can_start() {
        // 入力モード不明(読めない窓の起動直後)でも、既定のひらがなを種にして予測を始める。
        let p = predict(
            KeymapPreset::Atok,
            0x1C,
            &input(false, InputModeState::Unknown, false, NOTRACK),
        )
        .unwrap();
        assert_eq!(p.effect.open, Some(true));
        assert_eq!(p.effect.mode, Some(kana_mode()));
        // 表に無いキー(文字)でも種は反映する。
        let p = predict(
            KeymapPreset::Atok,
            0x41,
            &input(true, InputModeState::Unknown, false, NOTRACK),
        )
        .unwrap();
        assert_eq!(p.effect.mode, Some(kana_mode()));
    }

    #[test]
    fn observed_conv_raw_selects_the_katakana_state_only_where_the_preset_reaches_it() {
        // 観測したconv(0x1B=全角カタカナ)が追跡に無いときの初期値になる。MS-IMEプリセットはキーで0x1Bに入れる。
        let mut i = input(true, ROMAJI, false, NOTRACK);
        i.conv_raw = Some(0x1B);
        let p = predict(KeymapPreset::MsIme, 0xF1, &i).unwrap();
        assert_eq!(p.track.conv, Some(Conv::C1B));
        // ATOKは0x1Bにキーで入れない(表に行が無い): 予測しない。
        assert_eq!(predict(KeymapPreset::Atok, 0xF1, &i), None);
        assert_eq!(Conv::from_raw(0x19), Some(Conv::C19));
        assert_eq!(Conv::from_raw(0x00), Some(Conv::C10));
        assert_eq!(
            Conv::from_raw(0x03),
            None,
            "半角カタカナは到達不能で追わない"
        );
        assert_eq!(Conv::from_raw(0x08), None, "全角英数は到達不能で追わない");
    }

    #[test]
    fn closed_state_predicts_open_close_only_and_drops_the_conv_track() {
        // 閉(OFF)状態の変換モードは読み取りが不安定なので、閉のセルは変換モードを問わず、押下後の追跡も捨てる。
        let tracked = KeyTrack {
            conv: Some(Conv::C10),
            stage: Stage::None,
        };
        for conv in [Conv::C10, Conv::C19] {
            let t = KeyTrack {
                conv: Some(conv),
                stage: Stage::None,
            };
            let p = predict(KeymapPreset::Atok, 0xF3, &input(false, ROMAJI, false, t)).unwrap();
            assert_eq!(p.effect.open, Some(true));
            assert_eq!(p.track.conv, None, "開く遷移の押下後convは不明");
        }
        // 開→閉でも追跡を捨てる。
        let p = predict(
            KeymapPreset::Atok,
            0xF3,
            &input(true, ROMAJI, false, tracked),
        )
        .unwrap();
        assert_eq!(p.effect.open, Some(false));
        assert_eq!(p.track.conv, None);
    }

    #[test]
    fn grid_facts_are_reproduced_by_the_generated_table() {
        // 実測(CI --grid): ATOK 入力中の無変換は ToggleAlphanumericMode の変換系の段階へ入り、
        // 入力中を保持したまま半角英数(0x10)になる。
        let typing = input(true, ROMAJI, true, NOTRACK);
        let p1 = predict(KeymapPreset::Atok, 0x1D, &typing).unwrap();
        assert_eq!(p1.track.stage, Stage::ConvMuhenkan);
        assert_eq!(p1.track.conv, Some(Conv::C10));
        assert_eq!(p1.effect.mode, Some(InputModeState::ObservedEisu));
        // 半角/全角: 開なら閉じて入力中は破棄、閉なら開く。
        let hz = predict(KeymapPreset::Atok, 0xF3, &typing).unwrap();
        assert_eq!(hz.effect.open, Some(false));
        assert_eq!(hz.track.stage, Stage::None);
        let opened = predict(
            KeymapPreset::Atok,
            0xF4,
            &input(false, ROMAJI, false, NOTRACK),
        )
        .unwrap();
        assert_eq!(opened.effect.open, Some(true));
        // 非決定セル（ATOK: 変換中のEsc〈保持/破棄〉、入力中のBS〈保持/破棄〉）は生成時に除外され、予測しない。
        let conv_space = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::ConvSpace,
        };
        assert_eq!(
            predict(
                KeymapPreset::Atok,
                0x1B,
                &input(true, ROMAJI, true, conv_space)
            ),
            None
        );
        assert_eq!(predict(KeymapPreset::Atok, 0x08, &typing), None);
    }

    #[test]
    fn msime_preset_uses_its_own_table() {
        // MS-IMEプリセット: 開・ひらがなでひらがな(0xF2)は0x19のまま/カタカナ(0xF1)は0x1Bへ、など
        // atokと別の表であること(同一入力で結果が食い違うセルがある)。
        let differs = [0xF0, 0xF1, 0xF2, 0x1C, 0x1D].iter().any(|&vk| {
            let a = predict(KeymapPreset::Atok, vk, &input(true, ROMAJI, false, NOTRACK));
            let m = predict(
                KeymapPreset::MsIme,
                vk,
                &input(true, ROMAJI, false, NOTRACK),
            );
            a != m
        });
        assert!(differs);
    }

    #[test]
    fn keymap_from_config_selects_preset_and_respects_overrides() {
        let base = input(true, ROMAJI, false, NOTRACK);
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        assert!(atok.predict(0xF2, &base).is_some());
        // CUSTOM・MOBILE 等は基準の同梱表が無い(KeymapPreset::Custom)が、ADR-195段階4 B3対応で
        // Noneは返さない(学習済み表があればpredict_with_override経由で使えるようにするため)。
        // 同梱表(predict())は無いので常にNoneを返す。
        let custom_preset = KeyEffectKeymap::from_config(Some(0), None, &[]).unwrap();
        assert_eq!(custom_preset.preset(), KeymapPreset::Custom);
        assert_eq!(custom_preset.predict(0xF2, &base), None);
        assert!(!custom_preset.is_unmodified_bundled_config());
        assert_eq!(
            KeyEffectKeymap::from_config(Some(4), None, &[])
                .unwrap()
                .preset(),
            KeymapPreset::Custom
        );
        // ATOK + 古いカスタム表: GJI はプリセットのとき表を読まないので、表の行を理由に打ち切らない
        // （ADR-209 決定4）。CUSTOM のときは表が無変換の行を持てば予測しない（従来どおり）。
        let table = "Precomposition\tMuhenkan\tIMEOn\n".to_string();
        let stale = KeyEffectKeymap::from_config(Some(1), Some(table.clone()), &[]).unwrap();
        assert!(stale.predict(0x1D, &base).is_some());
        let custom = KeyEffectKeymap::from_config(Some(0), Some(table), &[]).unwrap();
        assert_eq!(custom.predict(0x1D, &base), None);
        assert!(stale.predict(0xF2, &base).is_some());
        // overlay があると 無変換/変換 だけ予測しない。
        let ov = KeyEffectKeymap::from_config(Some(1), None, &[100]).unwrap();
        assert_eq!(ov.predict(0x1C, &base), None);
        assert!(ov.predict(0xF2, &base).is_some());
    }

    /// ADR-195段階4 B3対応の回帰テスト: 学習済み表(override_table)にこのキー・状態の答えが
    /// あれば、custom_table上書きガードより先にそれが使われる。ちょうどユーザーが
    /// カスタムキーマップで上書きし、学習させたかったキーそのものが、ガードによって
    /// 黙って予測対象から外れてしまう事故を防ぐ。
    #[test]
    fn predict_with_override_bypasses_custom_table_guard_for_the_customized_key() {
        // 閉状態から始める: セルの遷移(閉→開)がbeliefへの実変化になるようにするため
        // (`effect.open`は`after_open != input.open`のときだけ`Some`を返す)。
        let base = input(false, ROMAJI, false, NOTRACK);
        // CUSTOM + カスタム表が無変換(0x1D)の行を持つ → 通常のpredict()は無変換だけ予測しない
        // (観測に委ねる、まさにユーザーが学習させたいキー)。
        let table = "Precomposition\tMuhenkan\tIMEOn\n".to_string();
        let custom = KeyEffectKeymap::from_config(Some(0), Some(table), &[]).unwrap();
        assert_eq!(custom.predict(0x1D, &base), None);

        let learned = [cell(
            false,
            None,
            Stage::None,
            TableKey::Muhenkan,
            true,
            Some(Conv::C19),
            Disp::None,
        )];
        let prediction = custom
            .predict_with_override(0x1D, &base, Some(&learned))
            .expect("学習済み表に答えがあれば、ガードより先にそれを使うはず");
        assert_eq!(prediction.effect.open, Some(true));

        // 学習済み表にこのキーの答えが無ければ、従来どおりガードが効く。
        assert_eq!(custom.predict_with_override(0x1D, &base, Some(&[])), None);
    }

    /// ADR-195段階4 B3対応の回帰テスト: `session_keymap`がCUSTOM/MOBILE等
    /// (`KeymapPreset::Custom`)でも、学習済み表があれば`predict_with_override`経由で
    /// 予測が返る(以前は`from_config`が`None`を返し、この構成では予測自体が始まらなかった)。
    #[test]
    fn predict_with_override_works_for_custom_preset_when_learned_table_has_the_cell() {
        let base = input(true, ROMAJI, false, NOTRACK);
        let custom_preset = KeyEffectKeymap::from_config(Some(0), None, &[]).unwrap();
        assert_eq!(custom_preset.predict(0xF2, &base), None);

        let learned = [cell(
            true,
            Some(Conv::C19),
            Stage::None,
            TableKey::Hiragana,
            true,
            Some(Conv::C19),
            Disp::None,
        )];
        assert!(custom_preset
            .predict_with_override(0xF2, &base, Some(&learned))
            .is_some());
    }

    /// 実機(ADR-191 実機検証、GJI + MS-IMEプリセット `session_keymap=2` + 既存の `custom_keymap_table` 175行)の
    /// キーマップの代表行。GJIは`CUSTOM`以外ではこの表を使わずプリセットで動く（ADR-186 決定2(c)、X1）ので、
    /// 古い表の行を理由に予測を打ち切らない（ADR-209 決定4）。素の EDIT（読める窓）では変換は開かない
    /// （同梱表どおり）。読める/読めないの区別は`unreadable`（TSF の窓では変換で開く、決定1）。
    #[test]
    fn realdev_msime_preset_with_stale_custom_table() {
        let table = "status\tkey\tcommand\nDirectInput\tEisu\tIMEOn\nDirectInput\tHenkan\tIMEOn\n\
                     Precomposition\tEisu\tToggleAlphanumericMode\nPrecomposition\tHenkan\tCompositionModeHiragana\n\
                     Precomposition\tF15\tCompositionModeHiragana\nComposition\tShift Henkan\tCompositionModeFullKatakana\n"
            .to_string();
        let km = KeyEffectKeymap::from_config(Some(2), Some(table), &[]).unwrap();
        let closed = input(false, ROMAJI, false, NOTRACK);
        // 変換（読める窓）: 表の行は無視し、同梱表（MS-IME プリセット、EDIT で測定）は開かないので予測は変えない。
        assert!(km
            .predict(0x1C, &closed)
            .is_none_or(|p| p.effect.open != Some(true)));
        // ひらがな(0xF2): 閉から開く(実機の awase 無し実測: open 0→1、conv 0x09→0x19)。
        let hira = km.predict(0xF2, &closed).expect("ひらがなは予測する");
        assert_eq!(hira.effect.open, Some(true));
        // 英数(0xF0): 古い表の行を理由に打ち切らない。同梱表の予測が引かれる。
        assert!(km.predict(0xF0, &closed).is_some());
    }

    fn unreadable_input(open: bool, mode: InputModeState) -> PredictInput {
        PredictInput {
            unreadable: true,
            ..input(open, mode, false, NOTRACK)
        }
    }

    /// ADR-209 決定1・2: 読めない窓・閉状態の無修飾の変換は開閉だけ予測する（モードは変えない）。
    #[test]
    fn adr209_unreadable_window_henkan_opens_without_touching_mode() {
        for session in [None, Some(-1), Some(2)] {
            let km = KeyEffectKeymap::from_config(session, None, &[]).unwrap();
            let p = km
                .predict(0x1C, &unreadable_input(false, ROMAJI))
                .expect("読めない窓の変換は開くと予測する");
            assert_eq!(p.effect.open, Some(true), "session={session:?}");
            assert_eq!(p.effect.mode, None);
            // 半角英数のまま開く（X7）: モードは書かない。
            let p = km
                .predict(0x1C, &unreadable_input(false, InputModeState::ObservedEisu))
                .unwrap();
            assert_eq!(p.effect.mode, None);
            // belief が Unknown のときだけ既定の種。
            let p = km
                .predict(0x1C, &unreadable_input(false, InputModeState::Unknown))
                .unwrap();
            assert_eq!(p.effect.mode, Some(kana_mode()));
        }
    }

    /// 規則が当たらない条件: 読める窓・開状態・他のキー・他のプリセット・overlay・レジストリ再割り当て。
    #[test]
    fn adr209_rule_does_not_apply_outside_its_conditions() {
        let km = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        let opens = |p: Option<Prediction>| p.is_some_and(|p| p.effect.open == Some(true));
        assert!(
            !opens(km.predict(0x1C, &input(false, ROMAJI, false, NOTRACK))),
            "読める窓"
        );
        assert!(
            !opens(km.predict(0x1C, &unreadable_input(true, ROMAJI))),
            "開状態"
        );
        assert!(
            !opens(km.predict(0x1D, &unreadable_input(false, ROMAJI))),
            "無変換"
        );
        let custom = KeyEffectKeymap::from_config(Some(0), None, &[]).unwrap();
        assert!(
            !opens(custom.predict(0x1C, &unreadable_input(false, ROMAJI))),
            "CUSTOM"
        );
        let overlay = KeyEffectKeymap::from_config(Some(2), None, &[100]).unwrap();
        assert!(
            !opens(overlay.predict(0x1C, &unreadable_input(false, ROMAJI))),
            "overlay"
        );
    }

    /// ADR-209 決定3: 学習表（素の EDIT で測ったため`off|henkan=OFF`）より窓別の規則が先。
    #[test]
    fn adr209_unreadable_rule_beats_learned_table() {
        let km = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        let learned = vec![cell(
            false,
            None,
            Stage::None,
            TableKey::Henkan,
            false,
            None,
            Disp::None,
        )];
        let p = km
            .predict_with_override(0x1C, &unreadable_input(false, ROMAJI), Some(&learned))
            .unwrap();
        assert_eq!(p.effect.open, Some(true));
        // 読める窓では学習表が引かれる（従来どおり）。
        let p =
            km.predict_with_override(0x1C, &input(false, ROMAJI, false, NOTRACK), Some(&learned));
        assert!(p.is_none_or(|p| p.effect.open != Some(true)));
    }

    /// ADR-209 決定4: CUSTOM のときは従来どおり、表が行を持つキーは予測しない。
    #[test]
    fn adr209_custom_preset_keeps_custom_table_cutoff() {
        let table = "DirectInput\tHenkan\tIMEOn\n".to_string();
        let km = KeyEffectKeymap::from_config(Some(0), Some(table), &[]).unwrap();
        assert_eq!(
            km.predict(0x1C, &input(false, ROMAJI, false, NOTRACK)),
            None
        );
    }

    /// ADR-247: CUSTOM の表の F16（半角英数へ SET）は、開状態で押すと入力モードが英数になる（開閉は変えない）。
    #[test]
    fn adr247_custom_f16_sets_half_alphanumeric() {
        let table =
            "DirectInput\tON\tIMEOn\nPrecomposition\tF16\tInputModeHalfAlphanumeric\n".to_string();
        let km = KeyEffectKeymap::from_config(Some(0), Some(table), &[]).unwrap();
        let p = km
            .predict(0x7F, &eligible_input(true, ROMAJI))
            .expect("F16 の行から予測される");
        assert_eq!(p.effect.open, None);
        assert_eq!(p.effect.mode, Some(InputModeState::ObservedEisu));
        assert_eq!(p.track.conv, Some(Conv::C10));
        // すでに英数なら効果なし（モードの予測は付かない）。
        let p = km
            .predict(0x7F, &eligible_input(true, InputModeState::ObservedEisu))
            .expect("追跡する変換モードは更新される");
        assert_eq!(p.effect.mode, None);
        assert_eq!(p.track.conv, Some(Conv::C10));
    }

    /// ADR-247: 閉状態の F17 が `InputModeHiragana`（DirectInput）なら開いてかなになる。表に行が無い状態・F キーは予測しない。
    #[test]
    fn adr247_custom_f_key_opens_and_respects_conditions() {
        let table =
            "DirectInput\tF17\tInputModeHiragana\nPrecomposition\tF18\tToggleAlphanumericMode\n"
                .to_string();
        let km = KeyEffectKeymap::from_config(Some(0), Some(table), &[]).unwrap();
        let p = km.predict(0x80, &eligible_input(false, ROMAJI)).unwrap();
        assert_eq!(p.effect.open, Some(true));
        // 条件を満たさない: ゲート偽（修飾付き・リピート等）・相対トグル系・行の無い F キー・変換中。
        assert_eq!(
            km.predict(0x80, &input(false, ROMAJI, false, NOTRACK)),
            None
        );
        assert_eq!(km.predict(0x81, &eligible_input(true, ROMAJI)), None);
        assert_eq!(km.predict(0x82, &eligible_input(true, ROMAJI)), None);
        let converting = PredictInput {
            track: KeyTrack {
                conv: None,
                stage: Stage::ConvSpace,
            },
            ..eligible_input(false, ROMAJI)
        };
        assert_eq!(km.predict(0x80, &converting), None);
        // プリセット（ATOK）は CUSTOM の表を評価しない。
        let atok = KeyEffectKeymap::from_config(
            Some(1),
            Some("DirectInput\tF17\tIMEOn\n".to_string()),
            &[],
        )
        .unwrap();
        assert_eq!(atok.predict(0x80, &eligible_input(false, ROMAJI)), None);
    }

    fn eligible_input(open: bool, mode: InputModeState) -> PredictInput {
        PredictInput {
            passive_rule_eligible: true,
            ..input(open, mode, false, NOTRACK)
        }
    }

    const F13: u16 = 0x7C;

    /// ADR-211 決定1: プリセットの F13 は閉状態から開く（開閉だけ。モードは変えない）。
    #[test]
    fn adr211_f13_opens_when_closed_in_ms_ime_like_presets() {
        for (session, table) in [
            (Some(2), None),
            (Some(4), None),
            (None, None),
            (Some(-1), None),
            (Some(0), None),
            (Some(0), Some("".to_string())),
            // 古い表が残っていても、プリセット(2)の GJI は表を読まない（ADR-186 決定2(c)）。
            (Some(2), Some("DirectInput\tF13\tIMEOff\n".to_string())),
        ] {
            let km = KeyEffectKeymap::from_config(session, table.clone(), &[]).unwrap();
            let p = km
                .predict(F13, &eligible_input(false, ROMAJI))
                .unwrap_or_else(|| panic!("session={session:?} table={table:?}"));
            assert_eq!(p.effect.open, Some(true), "session={session:?}");
            assert_eq!(p.effect.mode, None);
            let p = km
                .predict(F13, &eligible_input(false, InputModeState::Unknown))
                .unwrap();
            assert_eq!(p.effect.mode, Some(kana_mode()));
        }
    }

    /// 規則が当たらない条件（決定1・2）。
    #[test]
    fn adr211_f13_rule_does_not_apply_outside_its_conditions() {
        let opens = |p: Option<Prediction>| p.is_some_and(|p| p.effect.open == Some(true));
        let ms = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        assert!(
            !opens(ms.predict(F13, &input(false, ROMAJI, false, NOTRACK))),
            "ゲート(eligible)が偽"
        );
        assert!(
            !opens(ms.predict(F13, &eligible_input(true, ROMAJI))),
            "開状態"
        );
        assert!(
            !opens(ms.predict(0x7D, &eligible_input(false, ROMAJI))),
            "F14"
        );
        for (session, name) in [(Some(1), "ATOK"), (Some(3), "KOTOERI")] {
            let km = KeyEffectKeymap::from_config(session, None, &[]).unwrap();
            assert!(
                !opens(km.predict(F13, &eligible_input(false, ROMAJI))),
                "{name}"
            );
        }
        let custom =
            KeyEffectKeymap::from_config(Some(0), Some("DirectInput\tF13\tIMEOn\n".into()), &[])
                .unwrap();
        // ADR-211 の規則(プリセットの F13)は CUSTOM(表あり)に当てない。CUSTOM の表の行は ADR-247 の規則が別に扱う
        // (`custom_f_key_prediction`、`adr247_*` のテスト)ので、`predict` の結果ではなく ADR-211 の規則そのものを見る。
        assert!(
            custom
                .passive_open_key_prediction(F13, &eligible_input(false, ROMAJI))
                .is_none(),
            "CUSTOM(表あり)"
        );
        let overlay = KeyEffectKeymap::from_config(Some(2), None, &[100]).unwrap();
        assert!(
            !opens(overlay.predict(F13, &eligible_input(false, ROMAJI))),
            "overlay"
        );
        // Microsoft IME 本体（session_keymap は None だが GJI の規則ではない）。
        let native = KeyEffectKeymap::for_msime_native(false, None, None, None);
        assert!(
            !opens(native.predict(F13, &eligible_input(false, ROMAJI))),
            "MS-IME 本体"
        );
        // ADR-209 の規則そのもの（本体の同梱表は閉状態の変換を元々「開く」と予測するので、結果でなく規則の有無を見る）。
        assert!(
            native
                .unreadable_window_prediction(0x1C, &unreadable_input(false, ROMAJI))
                .is_none(),
            "MS-IME 本体には ADR-209 の規則を当てない(N1)"
        );
        let gji = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        assert!(gji
            .unreadable_window_prediction(0x1C, &unreadable_input(false, ROMAJI))
            .is_some());
    }

    /// 学習表・追跡の段階・種があっても、規則は先頭で当たる（B1: 表に無いキーで `predict_in_table` が先に抜ける分岐を通らない）。
    #[test]
    fn adr211_f13_rule_precedes_learned_table_and_stage() {
        let km = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        let learned = vec![cell(
            false,
            None,
            Stage::None,
            TableKey::Henkan,
            false,
            None,
            Disp::None,
        )];
        let mut typing = eligible_input(false, ROMAJI);
        typing.track = KeyTrack {
            conv: None,
            stage: Stage::Typing,
        };
        assert_eq!(
            km.predict_with_override(F13, &typing, Some(&learned))
                .unwrap()
                .effect
                .open,
            Some(true)
        );
    }

    /// 定数表の Mozc 名（`role.rs` のテストが使う `TABLE_KEY_MOZC_NAMES`）が、予測の表のキーと一致していること。
    #[test]
    fn adr211_table_key_names_used_by_the_gji_config_test_match_table_keys() {
        for name in [
            "Eisu",
            "Hankaku/Zenkaku",
            "Henkan",
            "Hiragana",
            "Kanji",
            "Katakana",
            "Muhenkan",
            "ON",
        ] {
            let vks = awase_gji_config::keymap::mozc_key_vk_names(name);
            // `Kanji`(0x19)は`mozc_key_vk_names`が VK に写さない（ADR-199 T2）ので、ここでは確認できない。
            assert!(!vks.is_empty() || name == "Kanji", "{name}");
            for vk_name in vks {
                use crate::vk::VkCodeExt;
                let vk =
                    awase::types::VkCode::from_name(vk_name).unwrap_or_else(|| panic!("{vk_name}"));
                assert!(
                    TableKey::from_vk(vk.0).is_some(),
                    "{name} -> {vk_name} は表のキー"
                );
            }
        }
        assert!(TableKey::from_vk(F13).is_none(), "F13 は表のキーではない");
    }

    #[test]
    fn custom_table_rows_for_the_key_disable_prediction() {
        let table =
            "status\tkey\tcommand\nDirectInput\tF15\tIMEOn\nPrecomposition\tMuhenkan\tIMEOff\n";
        assert!(custom_table_overrides(table, 0x1D));
        assert!(!custom_table_overrides(table, 0x1C));
        let alias = "Composition\tHiragana\tCancel\n";
        assert!(custom_table_overrides(alias, 0xF2));
        assert!(!custom_table_overrides(alias, 0xF0));
        assert!(custom_table_overrides(
            "Composition\tEscape\tCancel\n",
            0x1B
        ));
        // ADR-199 T2: キー名→VK を awase-gji-config と一本化。プリセット TSV の綴り`ESC`も拾い、
        // 半角/全角は 0xF3/0xF4 の両方、`Kanji`行は 0x19 に写さない。
        assert!(custom_table_overrides("Composition\tESC\tCancel\n", 0x1B));
        let hz = "Composition\tHankaku/Zenkaku\tIMEOff\nComposition\tKanji\tIMEOff\n";
        assert!(custom_table_overrides(hz, 0xF3));
        assert!(custom_table_overrides(hz, 0xF4));
        assert!(!custom_table_overrides(hz, 0x19));
        // 修飾付きの行はそのキーの行として数えない（従来どおり）。
        assert!(!custom_table_overrides(
            "Composition\tShift Space\tConvert\n",
            0x20
        ));
    }

    // ---- ADR-199 T2: config1.db の読み取り結果・役割判定 ----

    #[test]
    fn missing_config1_db_is_the_default_msime_keymap() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let km = KeyEffectKeymap::from_config1_db_read(Err(missing)).expect("不在は既定の keymap");
        assert_eq!(km, KeyEffectKeymap::from_config(None, None, &[]).unwrap());
        assert_eq!(km.preset(), KeymapPreset::MsIme);
        // 予測も MS-IME プリセットで動く（決定8 の副作用、ADR-199）。
        let closed = input(false, ROMAJI, false, NOTRACK);
        assert!(km.predict(0xF3, &closed).is_some());
        // 半角/全角はトグルの役割を持つ。
        assert_eq!(
            km.gji_key_role(0xF3),
            Some(awase_gji_config::role::KeyRole::ImeToggle)
        );
    }

    #[test]
    fn unreadable_or_unparsable_config1_db_is_unknown() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(KeyEffectKeymap::from_config1_db_read(Err(denied)), None);
        // ファイルはあるがパースできない（`parse_top_level`が`None`、空のバイト列）→ 不明。
        assert_eq!(KeyEffectKeymap::from_config1_db_read(Ok(Vec::new())), None);
    }

    #[test]
    fn gji_key_role_follows_the_raw_session_keymap() {
        use awase_gji_config::role::KeyRole;
        // CUSTOM で半角/全角を別機能（開かない相対トグル。`CompositionMode*` は決定13 で Open に数える）にした表 → 受動。
        let custom = "status\tkey\tcommand\nDirectInput\tHankaku/Zenkaku\tToggleAlphanumericMode\n\
                      Precomposition\tHankaku/Zenkaku\tIMEOff\nComposition\tHankaku/Zenkaku\tIMEOff\n\
                      Conversion\tHankaku/Zenkaku\tIMEOff\nDirectInput\tON\tIMEOn\n\
                      Precomposition\tOFF\tIMEOff\nComposition\tOFF\tIMEOff\nConversion\tOFF\tIMEOff\n"
            .to_string();
        let km = KeyEffectKeymap::from_config(Some(0), Some(custom.clone()), &[]).unwrap();
        assert_eq!(km.gji_key_role(0xF3), None);
        assert_eq!(km.gji_key_role(0xF4), None);
        // KOTOERI(3) は preset=Custom にまとめられるが、役割は生の値で判別し、古い custom 表は読まない。
        let kotoeri = KeyEffectKeymap::from_config(Some(3), Some(custom), &[]).unwrap();
        assert_eq!(kotoeri.preset(), KeymapPreset::Custom);
        assert_eq!(kotoeri.gji_key_role(0xF3), Some(KeyRole::ImeToggle));
        // 候補外のキー（0xF2）は役割を持たない。0x19（Alt+半角/全角）は半角/全角と同じ `Hankaku/Zenkaku` 行に従う
        // （ADR-202 決定1、実機確認 T1(b)）ので、プリセット（KOTOERI）ではトグル。
        assert_eq!(kotoeri.gji_key_role(0xF2), None);
        assert_eq!(kotoeri.gji_key_role(0x19), Some(KeyRole::ImeToggle));
        // CUSTOM で半角/全角を別機能にした表（上の `custom`）では 0x19 も受動。
        assert_eq!(km.gji_key_role(0x19), None);
        // `Hankaku/Zenkaku` 行がトグルなら 0x19 もトグル。`Kanji` 行だけがトグルで `Hankaku/Zenkaku` 行が無い表では
        // 受動（実機: 0x19 は `Kanji` 行を見ない、run 36242940343）。
        let toggle_rows = |key: &str| {
            format!(
                "status\tkey\tcommand\nDirectInput\t{key}\tIMEOn\nPrecomposition\t{key}\tIMEOff\n\
                 Composition\t{key}\tIMEOff\nConversion\t{key}\tIMEOff\n\
                 DirectInput\tON\tIMEOn\nPrecomposition\tOFF\tIMEOff\nComposition\tOFF\tIMEOff\n\
                 Conversion\tOFF\tIMEOff\n"
            )
        };
        let hz = KeyEffectKeymap::from_config(Some(0), Some(toggle_rows("Hankaku/Zenkaku")), &[])
            .unwrap();
        assert_eq!(hz.gji_key_role(0x19), Some(KeyRole::ImeToggle));
        let kanji_only =
            KeyEffectKeymap::from_config(Some(0), Some(toggle_rows("Kanji")), &[]).unwrap();
        assert_eq!(kanji_only.gji_key_role(0x19), None);
        // 候補キーの VK 名は全て`from_name`で解決でき、F13〜F24・変換/無変換も VK 値から引ける。
        use crate::vk::VkCodeExt;
        for name in awase_gji_config::role::ROLE_CANDIDATE_VK_NAMES {
            assert!(awase::types::VkCode::from_name(name).is_some(), "{name}");
        }
        let rows = |key: &str| {
            format!(
                "DirectInput\t{key}\tIMEOn\nPrecomposition\t{key}\tIMEOff\n\
                 Composition\t{key}\tIMEOff\nConversion\t{key}\tIMEOff\n"
            )
        };
        let f_and_henkan = format!(
            "status\tkey\tcommand\n{}{}{}{}",
            rows("F13"),
            rows("F24"),
            rows("Henkan"),
            "DirectInput\tON\tIMEOn\nPrecomposition\tOFF\tIMEOff\n\
             Composition\tOFF\tIMEOff\nConversion\tOFF\tIMEOff\n"
        );
        let km = KeyEffectKeymap::from_config(Some(0), Some(f_and_henkan.clone()), &[]).unwrap();
        for vk in [0x7C, 0x87, 0x1C] {
            assert_eq!(km.gji_key_role(vk), Some(KeyRole::ImeToggle), "{vk:#x}");
        }
        assert_eq!(km.gji_key_role(0x1D), None);
        // overlay 100 が変換/無変換を書き換える構成では、変換は受動（F キーはトグルのまま）。
        let overlaid = KeyEffectKeymap::from_config(Some(0), Some(f_and_henkan), &[100]).unwrap();
        assert_eq!(overlaid.gji_key_role(0x1C), None);
        assert_eq!(overlaid.gji_key_role(0x7C), Some(KeyRole::ImeToggle));
        // Microsoft IME 本体のキーマップでは GJI の役割判定をしない。
        let native = KeyEffectKeymap::for_msime_native(false, None, None, None);
        assert_eq!(native.gji_key_role(0xF3), None);
    }

    // ---- Microsoft IME 本体(ImeKind=MicrosoftIme。CI cal-notify-msimenative-s{1..4}、独立walkで一段予測 99.1%) ----

    #[test]
    fn msime_native_hiragana_opens_from_direct_input() {
        // 実測(grid、--msime、awase なし): 閉(直接入力)でひらがな(0xF2)を押すと開く。撤去版CIで最初のF2が Engine に
        // 反映されなかった実害(msime-native/sc-* が3/3 FAIL)の予測側の対処。
        let km = KeyEffectKeymap::for_msime_native(false, None, None, None);
        let p = km
            .predict(0xF2, &input(false, ROMAJI, false, NOTRACK))
            .expect("MS-IME本体のひらがなは予測する");
        assert_eq!(p.effect.open, Some(true));
    }

    #[test]
    fn msime_native_eisu_closes_ime_and_hankaku_zenkaku_toggles() {
        // 実測: MS-IME本体の英数(0xF0)は開いていて入力中でなければ IME オフ(0x10)、半角/全角は開→閉・閉→開。
        let km = KeyEffectKeymap::for_msime_native(false, None, None, None);
        let hira = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::None,
        };
        let eisu = km.predict(0xF0, &input(true, ROMAJI, false, hira)).unwrap();
        assert_eq!(eisu.effect.open, Some(false));
        let hz_open = km.predict(0xF3, &input(true, ROMAJI, false, hira)).unwrap();
        assert_eq!(hz_open.effect.open, Some(false));
        let hz_closed = km
            .predict(0xF3, &input(false, ROMAJI, false, NOTRACK))
            .unwrap();
        assert_eq!(hz_closed.effect.open, Some(true));
    }

    #[test]
    fn msime_native_muhenkan_rotates_conversion_mode() {
        // 実測: 無変換(既定=かな切替)は 0x19→0x1B。
        let km = KeyEffectKeymap::for_msime_native(false, None, None, None);
        let hira = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::None,
        };
        let p = km.predict(0x1D, &input(true, ROMAJI, false, hira)).unwrap();
        assert_eq!(p.track.conv, Some(Conv::C1B));
    }

    #[test]
    fn msime_native_henkan_none_and_typing_esc_are_not_predicted() {
        // 独立walkで入力欄の中身/候補ウィンドウ依存と判明したセルは「予測なし」(再変換は入力欄に確定済み文字列があると入力中になる)。
        let km = KeyEffectKeymap::for_msime_native(false, None, None, None);
        let hira = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::None,
        };
        assert_eq!(km.predict(0x1C, &input(true, ROMAJI, false, hira)), None);
        let typing = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::Typing,
        };
        // 入力中の Esc: 開閉・入力モードは予測しない（候補ウィンドウ依存）。ただし入力中の段階の追跡は捨てる
        // （BUG-162。捨てないと Esc の後の打鍵が古い「入力中」の行で誤予測される。候補ウィンドウだけが閉じて
        // 入力中が残る場合の取りこぼしは、読める窓では観測（composing）が直す。読めない窓では未解決の限界）。
        let esc = km
            .predict(0x1B, &input(true, ROMAJI, true, typing))
            .expect("追跡の更新だけ返す");
        assert!(esc.effect.is_noop());
        assert_eq!(esc.track.stage, Stage::None);
    }

    #[test]
    fn msime_native_reassigned_keys_are_not_predicted() {
        // レジストリのキー割り当て(IsKeyAssignmentEnabled=1)で変換/無変換に明示値があれば、その打鍵は予測しない
        // (安全側。値0/3の実機的意味はADR-199 T12で未確認、決定C R3・M5)。
        let closed = input(false, ROMAJI, false, NOTRACK);
        let km = KeyEffectKeymap::for_msime_native(true, Some(1), Some(1), None);
        assert_eq!(km.predict(0x1C, &closed), None);
        assert_eq!(km.predict(0x1D, &closed), None);
        assert!(km.predict(0xF2, &closed).is_some(), "他のキーは予測する");
        // マスタースイッチが無効なら割り当ては効かない(既定のキー設定)。
        let km = KeyEffectKeymap::for_msime_native(false, Some(1), Some(1), None);
        assert!(km.predict(0x1D, &closed).is_some());
        // 値0もADR-199 T12でIME-オンと確定した明示値であり、既定ではない(以前の「0=既定」の
        // 前提が逆転した、M5)。安全側として予測しない。
        let km = KeyEffectKeymap::for_msime_native(true, Some(0), Some(0), None);
        assert_eq!(km.predict(0x1D, &closed), None);
    }

    // ── ADR-199 T17: `msime_native_key_role`（半角/全角トグル・互換モードでの受動化、無変換/変換は値2のときだけトグル） ──

    #[test]
    fn msime_native_key_role_hz_is_toggle_unless_compat_mode() {
        use awase_gji_config::role::KeyRole::ImeToggle;
        let default = KeyEffectKeymap::for_msime_native(false, None, None, None);
        assert_eq!(default.msime_native_key_role(0xF3), Some(ImeToggle));
        assert_eq!(default.msime_native_key_role(0xF4), Some(ImeToggle));
        let compat_off = KeyEffectKeymap::for_msime_native(false, None, None, Some(false));
        assert_eq!(compat_off.msime_native_key_role(0xF3), Some(ImeToggle));
        // 互換モードON(NoTsf3Override2=1)では半角/全角も受動(決定17・T13)。
        let compat_on = KeyEffectKeymap::for_msime_native(false, None, None, Some(true));
        assert_eq!(compat_on.msime_native_key_role(0xF3), None);
        assert_eq!(compat_on.msime_native_key_role(0xF4), None);
    }

    #[test]
    fn msime_native_key_role_thumb_keys_toggle_only_for_value_2() {
        use awase_gji_config::role::KeyRole::ImeToggle;
        // ADR-199 T17 Phase 4: マスタースイッチ{ON,OFF} x 値{なし,0,1,2,3} x 互換{None,Some(false),Some(true)} x キー{変換0x1C,無変換0x1D}。
        // Some(ImeToggle)になるのは「ON・そのキーの値==2・互換!=Some(true)」だけ。取り違え防止に無変換と変換で別の値を与える。
        let values = [None, Some(0), Some(1), Some(2), Some(3)];
        for enabled in [true, false] {
            for h in values {
                for m in values {
                    for compat in [None, Some(false), Some(true)] {
                        let km = KeyEffectKeymap::for_msime_native(enabled, h, m, compat);
                        let want = |v: Option<u32>| {
                            (enabled && v == Some(2) && compat != Some(true)).then_some(ImeToggle)
                        };
                        assert_eq!(
                            km.msime_native_key_role(0x1C),
                            want(h),
                            "henkan {enabled}/{h:?}/{m:?}/{compat:?}"
                        );
                        assert_eq!(
                            km.msime_native_key_role(0x1D),
                            want(m),
                            "muhenkan {enabled}/{h:?}/{m:?}/{compat:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn msime_native_key_role_fkeys_and_other_vks_are_passive() {
        let km = KeyEffectKeymap::for_msime_native(false, None, None, None);
        assert_eq!(km.msime_native_key_role(0x7C), None, "F13");
        assert_eq!(km.msime_native_key_role(0x41), None, "'A'");
    }

    /// opusレビュー指摘: docコメントは「GJIのキーマップ(MsImeNative以外)ではNone」と約束して
    /// いるが、以前の実装はpresetを見ておらずGJIのキーマップでも半角/全角にSome(ImeToggle)を
    /// 返していた(実害は無い——呼び出し元は常にMS-IME本体のキーマップだけを渡すため——が、
    /// 将来の誤用を防ぐガードを追加した)。
    #[test]
    fn msime_native_key_role_is_none_for_gji_keymap() {
        let gji = KeyEffectKeymap::from_config(None, None, &[]).unwrap();
        assert_eq!(gji.msime_native_key_role(0xF3), None);
        assert_eq!(gji.msime_native_key_role(0xF4), None);
    }

    /// ADR-244 M-2: 持続半角英数トグル中だけ、追跡した変換モードを捨てる。入力中の段階は残す。トグル外は何も変えない。
    #[test]
    fn key_track_forgets_conv_only_while_half_width_alnum_toggle_is_active() {
        let track = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::Typing,
        };
        assert_eq!(track.without_conv_while_half_width_alnum(false), track);
        assert_eq!(
            track.without_conv_while_half_width_alnum(true),
            KeyTrack {
                conv: None,
                stage: Stage::Typing
            }
        );
    }

    /// ADR-244 M-2: 追跡を捨てたトグル中は、MS-IME 本体の 変換・無変換・英数・ひらがな が「予測なし」になる
    /// （ひらがな等を先に押して追跡が `Some(C19)` でも、直接観測が働く）。追跡を捨てないと予測が付く。
    #[test]
    fn msime_native_toggle_keys_have_no_prediction_once_stale_conv_is_dropped() {
        let stale = KeyTrack {
            conv: Some(Conv::C19),
            stage: Stage::None,
        };
        let input = |track: KeyTrack| PredictInput {
            open: true,
            mode: InputModeState::ObservedEisu,
            conv_raw: None,
            composing: false,
            track,
            unreadable: true,
            passive_rule_eligible: false,
        };
        // 前提: 古い追跡 C19 のままだと、無変換・ひらがなには予測が付く（変換・英数は表のセルの有無で付かない）。
        for vk in [0x1D_u16, 0xF2] {
            assert!(
                predict_in_table(table_of(KeymapPreset::MsImeNative), vk, &input(stale)).is_some(),
                "前提: 古い追跡 C19 のままだと vk=0x{vk:02X} に予測が付く"
            );
        }
        // 追跡を捨てれば、変換・無変換・英数・ひらがな のどれも予測なし（D5 のテストと同じ 4 キー）。
        for vk in [0x1C_u16, 0x1D, 0xF0, 0xF2] {
            assert!(
                predict_in_table(
                    table_of(KeymapPreset::MsImeNative),
                    vk,
                    &input(stale.without_conv_while_half_width_alnum(true))
                )
                .is_none(),
                "追跡を捨てれば vk=0x{vk:02X} は予測なし"
            );
        }
    }

    /// ADR-245 決定10: ATOK は F2 がひらがな↔半角英数の純粋なトグルなので `Some(false)`、
    /// GJI の MS-IME プリセットはどの状態でもかなへ SET なので `Some(true)`、Custom は予測なしで `None`。
    #[test]
    fn hiragana_key_is_set_distinguishes_toggle_from_set_by_preset() {
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        assert_eq!(atok.preset(), KeymapPreset::Atok);
        assert_eq!(hiragana_key_is_set(&atok, None, false), Some(false));
        let msime = KeyEffectKeymap::from_config(None, None, &[]).unwrap();
        assert_eq!(msime.preset(), KeymapPreset::MsIme);
        assert_eq!(hiragana_key_is_set(&msime, None, false), Some(true));
        let custom = KeyEffectKeymap::from_config(Some(0), None, &[]).unwrap();
        assert_eq!(custom.preset(), KeymapPreset::Custom);
        assert_eq!(hiragana_key_is_set(&custom, None, false), None);
    }

    /// 窓の読めなさ(`unreadable`)は F2 の答えを変えない(窓別の規則は変換 0x1C だけ)。
    #[test]
    fn hiragana_key_is_set_is_independent_of_unreadable() {
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        let msime = KeyEffectKeymap::from_config(None, None, &[]).unwrap();
        assert_eq!(hiragana_key_is_set(&atok, None, true), Some(false));
        assert_eq!(hiragana_key_is_set(&msime, None, true), Some(true));
    }

    /// 検証済みの学習済み表(ADR-195 段階4)があれば、Custom 構成でも答えが出る。トグルを学習したら `Some(false)`、
    /// SET を学習したら `Some(true)`。片方のセルしか無ければ `None`。
    #[test]
    fn hiragana_key_is_set_uses_the_learned_table_for_custom() {
        let custom = KeyEffectKeymap::from_config(Some(0), None, &[]).unwrap();
        let to_kana = cell(
            true,
            Some(Conv::C10),
            Stage::None,
            TableKey::Hiragana,
            true,
            Some(Conv::C19),
            Disp::None,
        );
        let toggle_back = cell(
            true,
            Some(Conv::C19),
            Stage::None,
            TableKey::Hiragana,
            true,
            Some(Conv::C10),
            Disp::None,
        );
        let stay_kana = cell(
            true,
            Some(Conv::C19),
            Stage::None,
            TableKey::Hiragana,
            true,
            Some(Conv::C19),
            Disp::None,
        );
        assert_eq!(
            hiragana_key_is_set(&custom, Some(&[to_kana, toggle_back]), false),
            Some(false)
        );
        assert_eq!(
            hiragana_key_is_set(&custom, Some(&[to_kana, stay_kana]), false),
            Some(true)
        );
        assert_eq!(hiragana_key_is_set(&custom, Some(&[to_kana]), false), None);
    }

    // ── ADR-254 第一段: 旧UIのキーテンプレート(keystyle)が既定でないとき、MSIME_NATIVE の予測を止める ──

    /// `MSIME_NATIVE`にセルがある全`TableKey`のVK（無修飾）。表から作るので、将来キーが増えても
    /// 取りこぼさない（「止めるキーを列挙すると漏れる」、Opus r2 M2-3）。
    fn msime_native_table_vks() -> Vec<u16> {
        let mut vks: Vec<u16> = Vec::new();
        for c in bundled_table(KeymapPreset::MsImeNative) {
            for vk in 0u16..=0xFF {
                if TableKey::from_vk(vk) == Some(c.key) && !vks.contains(&vk) {
                    vks.push(vk);
                }
            }
        }
        vks
    }

    /// 開/閉 × 段階 の全入力（読める窓・読めない窓の両方）。
    fn all_inputs() -> Vec<PredictInput> {
        let mut v = Vec::new();
        for open in [false, true] {
            for unreadable in [false, true] {
                for composing in [false, true] {
                    for stage in [Stage::None, Stage::Typing, Stage::ConvSpace] {
                        v.push(PredictInput {
                            unreadable,
                            passive_rule_eligible: true,
                            ..input(open, ROMAJI, composing, KeyTrack { conv: None, stage })
                        });
                    }
                }
            }
        }
        v
    }

    fn msime_native(
        keystyle: MsImeKeystyle,
        compat: Option<bool>,
        hash: Option<u64>,
    ) -> KeyEffectKeymap {
        KeyEffectKeymap::for_msime_native_with_legacy(false, None, None, compat, keystyle, hash)
    }

    /// (1) 互換 ON の Custom・名前付き・未知は、`MSIME_NATIVE`にセルがある全キー × 全入力で予測しない。
    #[test]
    fn adr254_stopped_configs_predict_nothing_for_every_table_key() {
        let vks = msime_native_table_vks();
        assert!(vks.len() >= 6, "表のキーが取れていない: {vks:?}");
        // テストが空振りしない保証: 既定の構成(止めない)では、同じ全組み合わせの多くで予測が付く。
        // (止める構成が全部 None になるのは、そもそも予測が付かない入力だから、ではない。)
        let default_km = KeyEffectKeymap::for_msime_native(false, None, None, None);
        let predicted = vks
            .iter()
            .flat_map(|&vk| all_inputs().into_iter().map(move |inp| (vk, inp)))
            .filter(|(vk, inp)| default_km.predict(*vk, inp).is_some())
            .count();
        assert!(
            predicted >= 10,
            "既定の構成で予測が付く組み合わせが少なすぎる: {predicted}"
        );
        for (style, compat) in [
            (MsImeKeystyle::Custom, Some(true)),
            (MsImeKeystyle::Named, Some(true)),
            (MsImeKeystyle::Named, Some(false)),
            (MsImeKeystyle::Named, None),
            (MsImeKeystyle::Unknown, Some(true)),
            (MsImeKeystyle::Unknown, Some(false)),
        ] {
            let km = msime_native(style, compat, Some(1));
            assert!(km.legacy_table_unknown(), "{style:?} {compat:?}");
            for &vk in &vks {
                for inp in all_inputs() {
                    assert_eq!(
                        km.predict(vk, &inp),
                        None,
                        "{style:?} compat={compat:?} vk={vk:#x} {inp:?}"
                    );
                }
            }
        }
    }

    /// (2) keystyle が不在・NATURAL・互換 OFF の Custom は、今の既定(`for_msime_native(false, None, None, None)`)と
    /// 全キー × 全入力で予測が一致する(大多数の利用者の挙動が変わらない)。
    #[test]
    fn adr254_default_configs_predict_like_the_bundled_table() {
        // 基準は、同梱表をそのまま引く自由関数(`for_msime_native_with_legacy`を経由しない)。
        let reference = |vk: u16, inp: &PredictInput| predict(KeymapPreset::MsImeNative, vk, inp);
        let vks = msime_native_table_vks();
        for (style, compat) in [
            (MsImeKeystyle::Absent, None),
            (MsImeKeystyle::Absent, Some(true)),
            (MsImeKeystyle::Absent, Some(false)),
            (MsImeKeystyle::Natural, None),
            (MsImeKeystyle::Natural, Some(true)),
            (MsImeKeystyle::Natural, Some(false)),
            // 互換 OFF の Custom は新エンジンが読まないので NATURAL と同じ(実機: dragonflyg4、実測15)。
            (MsImeKeystyle::Custom, Some(false)),
        ] {
            let km = msime_native(style, compat, Some(9));
            assert!(!km.legacy_table_unknown(), "{style:?} {compat:?}");
            for &vk in &vks {
                for inp in all_inputs() {
                    // 互換モードは予測に影響しない(役割判定だけが参照する)ので、同じ予測になる。
                    assert_eq!(
                        km.predict(vk, &inp),
                        reference(vk, &inp),
                        "{style:?} compat={compat:?} vk={vk:#x}"
                    );
                }
            }
        }
        // 互換モードが読めない(None)Custom は安全側で止める(新エンジンか旧エンジンか分からない)。
        assert!(msime_native(MsImeKeystyle::Custom, None, None).legacy_table_unknown());
    }

    /// (3) 止める構成でも、学習表(`override_table`)に答えがあればそれを使う(ADR-196。学習で戻せる)。
    #[test]
    fn adr254_stopped_config_still_uses_the_learned_table() {
        let km = msime_native(MsImeKeystyle::Custom, Some(true), Some(1));
        let base = input(false, ROMAJI, false, NOTRACK);
        assert_eq!(km.predict(0x1C, &base), None);
        let learned = [cell(
            false,
            None,
            Stage::None,
            TableKey::Henkan,
            true,
            Some(Conv::C19),
            Disp::None,
        )];
        let p = km
            .predict_with_override(0x1C, &base, Some(&learned))
            .expect("学習表に答えがあれば使う");
        assert_eq!(p.effect.open, Some(true));
        // 学習表に答えが無ければ、止める(同梱表へ落ちない)。
        assert_eq!(km.predict_with_override(0x1D, &base, Some(&learned)), None);
    }

    /// (5) 互換フラグ(Some(true)/Some(false)/None)× 名前付き・未知は、フラグに関係なく止まる
    /// (Custom だけがフラグで変わる)。
    #[test]
    fn adr254_named_and_unknown_stop_regardless_of_compat_flag() {
        for compat in [Some(true), Some(false), None] {
            for style in [MsImeKeystyle::Named, MsImeKeystyle::Unknown] {
                assert!(
                    style.disables_native_prediction(compat),
                    "{style:?} {compat:?}"
                );
            }
        }
        assert!(MsImeKeystyle::Custom.disables_native_prediction(Some(true)));
        assert!(MsImeKeystyle::Custom.disables_native_prediction(None));
        assert!(!MsImeKeystyle::Custom.disables_native_prediction(Some(false)));
        for compat in [Some(true), Some(false), None] {
            assert!(!MsImeKeystyle::Absent.disables_native_prediction(compat));
            assert!(!MsImeKeystyle::Natural.disables_native_prediction(compat));
        }
    }

    /// (6) 指紋: 既定の構成(不在・NATURAL・互換 OFF の Custom)は従来と同じ、止める構成は別、
    /// Custom の表のハッシュ違いと ATOK/VJE(名前のハッシュ違い)は区別される。
    #[test]
    fn adr254_fingerprint_is_unchanged_for_defaults_and_differs_for_stopped_configs() {
        let base = KeyEffectKeymap::for_msime_native(false, None, None, None).fingerprint();
        for (style, compat) in [
            (MsImeKeystyle::Absent, None),
            (MsImeKeystyle::Natural, Some(true)),
            (MsImeKeystyle::Custom, Some(false)),
        ] {
            assert_eq!(msime_native(style, compat, Some(77)).fingerprint(), base);
        }
        let custom_a = msime_native(MsImeKeystyle::Custom, Some(true), Some(1)).fingerprint();
        let custom_b = msime_native(MsImeKeystyle::Custom, Some(true), Some(2)).fingerprint();
        let atok = msime_native(MsImeKeystyle::Named, Some(true), Some(10)).fingerprint();
        let vje = msime_native(MsImeKeystyle::Named, Some(true), Some(20)).fingerprint();
        assert_ne!(custom_a, base);
        assert_ne!(custom_a, custom_b);
        assert_ne!(atok, vje);
        // 互換 ON/OFF/不明は、止める構成の中では指紋に混ぜない(`msime_compat_mode`と同じ方針、役割判定だけが参照)。
        assert_eq!(
            msime_native(MsImeKeystyle::Named, Some(true), Some(10)).fingerprint(),
            msime_native(MsImeKeystyle::Named, Some(false), Some(10)).fingerprint()
        );
    }

    /// (7) 役割(ADR-199)は`keystyle`では変わらない(第一段は役割に触れない)。
    #[test]
    fn adr254_key_role_is_unchanged_by_keystyle() {
        for vk in [0x1Cu16, 0x1D, 0xF3, 0xF4, 0x19] {
            for compat in [Some(true), Some(false), None] {
                let reference = KeyEffectKeymap::for_msime_native(true, Some(2), Some(2), compat);
                for style in [
                    MsImeKeystyle::Absent,
                    MsImeKeystyle::Natural,
                    MsImeKeystyle::Custom,
                    MsImeKeystyle::Named,
                    MsImeKeystyle::Unknown,
                ] {
                    let km = KeyEffectKeymap::for_msime_native_with_legacy(
                        true,
                        Some(2),
                        Some(2),
                        compat,
                        style,
                        Some(5),
                    );
                    assert_eq!(
                        km.msime_native_key_role(vk),
                        reference.msime_native_key_role(vk),
                        "vk={vk:#x} compat={compat:?} {style:?}"
                    );
                }
            }
        }
    }

    /// (8) 読めない窓(`unreadable`)でも、止める構成は`None`。窓別の規則・受動キーの規則は MS-IME 本体では
    /// 元から`None`だが、順序を変えても崩れないよう固定する。
    #[test]
    fn adr254_unreadable_window_is_also_stopped() {
        let km = msime_native(MsImeKeystyle::Custom, Some(true), Some(1));
        for open in [false, true] {
            let inp = PredictInput {
                passive_rule_eligible: true,
                ..unreadable_input(open, ROMAJI)
            };
            for vk in [0x1Cu16, 0x1D, 0xF2, 0xF3, 0xF4, 0x19, 0x7C] {
                assert_eq!(km.predict(vk, &inp), None, "vk={vk:#x} open={open}");
            }
        }
    }

    // ── ADR-254 第二段: 互換 ON の Custom の表(S4key・key)から、無変換/変換の予測セルを作る ──

    const MUHENKAN_SJIS: &[u8] = &[0x96, 0xB3, 0x95, 0xCF, 0x8A, 0xB7];
    const HENKAN_SJIS: &[u8] = &[0x95, 0xCF, 0x8A, 0xB7];

    /// `<ラベル>=<コード>`をNUL区切りで並べた表のバイト列(終端は NUL が 1 つ多い)。
    fn table_bytes(rows: &[(&[u8], &str)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (label, codes) in rows {
            v.extend_from_slice(label);
            v.push(b'=');
            v.extend_from_slice(codes.as_bytes());
            v.push(0);
        }
        v.push(0);
        v
    }

    /// 互換 ON の Custom のキーマップ(止める構成)に、表から作ったセルを渡したもの。
    fn custom_on_with(s4key: Option<&[u8]>, key: Option<&[u8]>) -> KeyEffectKeymap {
        msime_native(MsImeKeystyle::Custom, Some(true), Some(1))
            .with_legacy_custom_cells(legacy_custom_cells(s4key, key))
    }

    fn effect_of(km: &KeyEffectKeymap, vk: u16, inp: &PredictInput) -> Option<PredictedEffect> {
        km.predict(vk, inp).map(|p| p.effect)
    }

    /// dragonflyg4 の実機の表(2026-10-10 の読み取り。UI の「IME オン/オフ」の割り当てが書く形):
    /// S4key の無変換・変換=`CE 00…`、key の無変換・変換=`CE CD CD CD CD CD`。実機の測定(互換 ON・Custom)で
    /// 閉・無変換が開く・開・入力なしでは何もしない、と確認した挙動を再現する。
    #[test]
    fn adr254_stage2_dragonflyg4_table_opens_when_closed_and_does_nothing_when_idle() {
        let s4 = table_bytes(&[
            (MUHENKAN_SJIS, "CE 00 00 00 00 00"),
            (HENKAN_SJIS, "CE 00 00 00 00 00"),
        ]);
        let key = table_bytes(&[
            (MUHENKAN_SJIS, "CE CD CD CD CD CD"),
            (HENKAN_SJIS, "CE CD CD CD CD CD"),
        ]);
        let km = custom_on_with(Some(&s4), Some(&key));
        let closed = input(false, ROMAJI, false, NOTRACK);
        for vk in [0x1Du16, 0x1C] {
            // 閉: 開く(実機: o0→o1)。
            assert_eq!(
                effect_of(&km, vk, &closed).and_then(|e| e.open),
                Some(true),
                "vk={vk:#x}"
            );
            // 開・入力なし: CE は何もしない(実機: 変化なし)=予測しない。
            let idle = input(true, ROMAJI, false, NOTRACK);
            assert_eq!(km.predict(vk, &idle), None, "vk={vk:#x}");
            // 入力中: 読まない(実測が無い列)。
            let typing = input(
                true,
                ROMAJI,
                true,
                KeyTrack {
                    conv: None,
                    stage: Stage::Typing,
                },
            );
            assert_eq!(km.predict(vk, &typing), None, "vk={vk:#x}");
        }
    }

    /// key の1列目のコード別の効果(CI と実機の測定): 97・C9→全角カタカナ、CD・B3→閉じる、A4→閉じて半角英数。
    #[test]
    fn adr254_stage2_key_first_column_codes() {
        let idle = input(true, ROMAJI, false, NOTRACK);
        let cases: [(&str, Option<bool>, Option<Conv>); 5] = [
            ("97 28 28 28 28 28", None, Some(Conv::C1B)),
            ("C9 C9 C9 C9 C9 C9", None, Some(Conv::C1B)),
            ("CD B3 B3 B3 B3 B3", Some(false), None),
            ("B3 B3 B3 B3 B3 B3", Some(false), None),
            ("A4 A4 A4 A4 A4 A4", Some(false), Some(Conv::C10)),
        ];
        for (codes, open, conv) in cases {
            let key = table_bytes(&[(MUHENKAN_SJIS, codes)]);
            let km = custom_on_with(None, Some(&key));
            let p = km
                .predict(0x1D, &idle)
                .unwrap_or_else(|| panic!("{codes}: 予測が付く"));
            assert_eq!(p.effect.open, open, "{codes}");
            assert_eq!(p.track.conv, conv, "{codes}");
        }
        // 変化なし・効果が未確認のコード(D5 は ND で向きが逆、CA/CE/00/80/FF は変化なし、未知)は予測しない。
        for codes in [
            "D5 00 00 00 00 00",
            "CA CA CA CA CA CA",
            "CE 00 00 00 00 00",
            "00 00 00 00 00 00",
            "80 00 00 00 00 00",
            "FF 00 00 00 00 00",
            "F5 00 00 00 00 00",
        ] {
            let key = table_bytes(&[(MUHENKAN_SJIS, codes)]);
            let km = custom_on_with(None, Some(&key));
            assert_eq!(km.predict(0x1D, &idle), None, "{codes}");
        }
    }

    /// S4key(閉じた状態)の1列目: 87・CE→開く。それ以外(00・80・FF・未知・行が無い・表が読めない)は予測しない。
    #[test]
    fn adr254_stage2_s4key_first_column_codes() {
        let closed = input(false, ROMAJI, false, NOTRACK);
        for codes in ["87 00 00 00 00 00", "CE 00 00 00 00 00"] {
            let s4 = table_bytes(&[(HENKAN_SJIS, codes)]);
            let km = custom_on_with(Some(&s4), None);
            assert_eq!(
                effect_of(&km, 0x1C, &closed).and_then(|e| e.open),
                Some(true),
                "{codes}"
            );
            // 行の無い無変換は予測しない。
            assert_eq!(km.predict(0x1D, &closed), None, "{codes}");
        }
        for codes in [
            "00 00 00 00 00 00",
            "80 00 00 00 00 00",
            "FF 00 00 00 00 00",
            "C9 00 00 00 00 00",
        ] {
            let s4 = table_bytes(&[(HENKAN_SJIS, codes)]);
            assert_eq!(
                custom_on_with(Some(&s4), None).predict(0x1C, &closed),
                None,
                "{codes}"
            );
        }
        // 表が読めない(None)なら、閉の予測も無い。
        assert_eq!(custom_on_with(None, None).predict(0x1C, &closed), None);
    }

    /// 行のラベルは完全一致だけ: `Ctrl+変換`(修飾付き)や`無変換`は、`変換`の行として読まない。食い違う重複行も使わない。
    #[test]
    fn adr254_stage2_label_matching_is_exact_and_conflicts_are_ignored() {
        let closed = input(false, ROMAJI, false, NOTRACK);
        let mut ctrl_henkan: Vec<u8> = b"Ctrl+".to_vec();
        ctrl_henkan.extend_from_slice(HENKAN_SJIS);
        let s4 = table_bytes(&[
            (&ctrl_henkan, "CE 00 00 00 00 00"),
            (MUHENKAN_SJIS, "CE 00 00 00 00 00"),
        ]);
        let km = custom_on_with(Some(&s4), None);
        assert_eq!(
            km.predict(0x1C, &closed),
            None,
            "修飾付きの行を変換の行と読まない"
        );
        assert!(km.predict(0x1D, &closed).is_some());
        // 同じ行が食い違って2つ: 優先順位が未確認なので使わない。同じなら使う。
        let conflict = table_bytes(&[
            (HENKAN_SJIS, "CE 00 00 00 00 00"),
            (HENKAN_SJIS, "00 00 00 00 00 00"),
        ]);
        assert_eq!(
            custom_on_with(Some(&conflict), None).predict(0x1C, &closed),
            None
        );
        let same = table_bytes(&[
            (HENKAN_SJIS, "CE 00 00 00 00 00"),
            (HENKAN_SJIS, "CE 00 00 00 00 00"),
        ]);
        assert!(custom_on_with(Some(&same), None)
            .predict(0x1C, &closed)
            .is_some());
        // 形式が壊れた行(= が無い・16進でない)は無視する。
        let broken = [b"junk\0".as_slice(), HENKAN_SJIS, b"=ZZ\0\0"].concat();
        assert_eq!(
            custom_on_with(Some(&broken), None).predict(0x1C, &closed),
            None
        );
    }

    /// 無変換/変換以外のキー(文字キー・半角/全角など)は、セルがあっても予測しない
    /// (`predict_in_table`が文字キーの追跡や種の反映をするので、止める構成では呼ばない)。
    #[test]
    fn adr254_stage2_other_keys_are_not_predicted() {
        let s4 = table_bytes(&[
            (MUHENKAN_SJIS, "CE 00 00 00 00 00"),
            (HENKAN_SJIS, "CE 00 00 00 00 00"),
        ]);
        let key = table_bytes(&[(MUHENKAN_SJIS, "97 28 28 28 28 28")]);
        let km = custom_on_with(Some(&s4), Some(&key));
        for inp in all_inputs() {
            for vk in [
                0x41u16, 0x4B, 0x20, 0x0D, 0x1B, 0xF3, 0xF4, 0x19, 0xF0, 0xF2, 0x7C,
            ] {
                assert_eq!(km.predict(vk, &inp), None, "vk={vk:#x} {inp:?}");
            }
        }
    }

    /// セルを受け付けるのは「互換 ON の Custom」だけ。名前付き・未知・互換 OFF/不明の Custom・既定の構成では、
    /// 渡されても捨てる(名前付き・未知は表の中身を読まない、止めない構成は同梱表で予測する)。
    #[test]
    fn adr254_stage2_cells_are_accepted_only_for_custom_with_compat_on() {
        let s4 = table_bytes(&[(HENKAN_SJIS, "CE 00 00 00 00 00")]);
        let cells = legacy_custom_cells(Some(&s4), None);
        assert!(!cells.is_empty());
        let closed = input(false, ROMAJI, false, NOTRACK);
        // 互換 ON の Custom: 受け付ける。
        let on = msime_native(MsImeKeystyle::Custom, Some(true), Some(1))
            .with_legacy_custom_cells(cells.clone());
        assert!(on.predict(0x1C, &closed).is_some());
        // 互換が読めない(None)の Custom: 止める構成だが表は読まない(新エンジンか旧エンジンか分からない)。
        let unknown = msime_native(MsImeKeystyle::Custom, None, Some(1))
            .with_legacy_custom_cells(cells.clone());
        assert!(unknown.legacy_table_unknown());
        assert_eq!(unknown.predict(0x1C, &closed), None);
        // 名前付き・未知: 渡されても捨てる。
        for style in [MsImeKeystyle::Named, MsImeKeystyle::Unknown] {
            let km =
                msime_native(style, Some(true), Some(1)).with_legacy_custom_cells(cells.clone());
            assert_eq!(km.predict(0x1C, &closed), None, "{style:?}");
        }
        // 止めない構成(互換 OFF の Custom・不在): 同梱表で予測する(セルは捨てる=結果が変わらない)。
        let reference = KeyEffectKeymap::for_msime_native(false, None, None, None);
        for (style, compat) in [
            (MsImeKeystyle::Custom, Some(false)),
            (MsImeKeystyle::Absent, None),
        ] {
            let km = msime_native(style, compat, None).with_legacy_custom_cells(cells.clone());
            assert_eq!(
                km.predict(0x1C, &closed),
                reference.predict(0x1C, &closed),
                "{style:?}"
            );
        }
    }

    /// 指紋はセルに影響しない(止める構成の指紋は表のハッシュで決まる。セルは表から作るので二重に混ぜない)。
    #[test]
    fn adr254_stage2_cells_do_not_change_the_fingerprint() {
        let s4 = table_bytes(&[(HENKAN_SJIS, "CE 00 00 00 00 00")]);
        let plain = msime_native(MsImeKeystyle::Custom, Some(true), Some(5));
        let with = plain
            .clone()
            .with_legacy_custom_cells(legacy_custom_cells(Some(&s4), None));
        assert_eq!(plain.fingerprint(), with.fingerprint());
    }

    // ── ADR-188 案2: FSM が送り直す親指の効果を予測に重ねる ──

    fn resend_decision(vks: &[u16]) -> awase::engine::Decision {
        use awase::engine::{Decision, Effect, InputEffect, TimerEffect};
        use awase::types::{KeyAction, VkCode};
        let mut d = Decision::pass_through();
        d.push_effect(Effect::Timer(TimerEffect::Kill(1)));
        d.push_effect(Effect::Input(InputEffect::SendKeys(
            vks.iter().map(|&v| KeyAction::Key(VkCode(v))).collect(),
        )));
        d
    }

    /// 送り直す親指（無変換）は、打鍵を通す決定（`PassThroughWith`）の `SendKeys` からだけ拾う。文字キー・Timer・
    /// 打鍵を消費する決定・素の `PassThrough` は空（executor が窓を開き直す判定〈`is_followed_mode_key`〉と同じ）。
    #[test]
    fn fsm_resent_mode_keys_picks_followed_mode_keys_of_pass_through_with_only() {
        use awase::engine::{Decision, Effect, InputEffect};
        use awase::types::{KeyAction, VkCode};
        assert_eq!(fsm_resent_mode_keys(&resend_decision(&[0x1D])), vec![0x1D]);
        assert_eq!(fsm_resent_mode_keys(&resend_decision(&[0x1C])), vec![0x1C]);
        assert!(
            fsm_resent_mode_keys(&resend_decision(&[0x41])).is_empty(),
            "文字キーは対象外"
        );
        assert!(
            fsm_resent_mode_keys(&resend_decision(&[0x20])).is_empty(),
            "Space は対象外"
        );
        assert!(fsm_resent_mode_keys(&Decision::pass_through()).is_empty());
        let mut consumed = Decision::consumed();
        consumed.push_effect(Effect::Input(InputEffect::SendKeys(vec![KeyAction::Key(
            VkCode(0x1D),
        )])));
        assert!(
            fsm_resent_mode_keys(&consumed).is_empty(),
            "打鍵を消費する決定は対象外（従来どおり観測に任せる）"
        );
    }

    /// CI run 38065774507（sc-armc-gji-atok-passthru）の外れ: ATOK でかなのとき、無変換を押したまま変換を押すと、
    /// FSM が無変換を送り直す（かな→閉）ので、変換は閉から開く。送り直しを重ねない予測は「かな→閉」で外れていた。
    #[test]
    fn atok_henkan_after_resent_muhenkan_predicts_open() {
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        let kana = input(true, ROMAJI, false, NOTRACK);
        // 送り直しが無い（従来）: かなの変換は閉じる。
        assert_eq!(atok.predict(0x1C, &kana).unwrap().effect.open, Some(false));
        let p = atok
            .predict_after_resent(&[0x1D], 0x1C, &kana, None)
            .expect("無変換(かな→閉)→変換(閉→開)は表にある");
        assert_eq!(
            p.effect.open,
            Some(true),
            "送り直した無変換で閉じてから変換で開く"
        );
        assert_eq!(p.effect.mode, None);
    }

    /// CI run 38065774507（sc-armc-gji-msimepreset-passthru）の外れ: MS-IME プリセットでかなのとき、無変換を押したまま
    /// 英数を押すと、送り直した無変換で全角カタカナになってから英数でひらがなに戻る。重ねない予測は半角英数（外れ）。
    #[test]
    fn msime_preset_eisu_after_resent_muhenkan_stays_kana() {
        let msime = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        let kana = input(true, ROMAJI, false, NOTRACK);
        assert_eq!(
            msime.predict(0xF0, &kana).unwrap().effect.mode,
            Some(InputModeState::ObservedEisu),
            "送り直しが無い（従来）: かなの英数は半角英数"
        );
        let p = msime
            .predict_after_resent(&[0x1D], 0xF0, &kana, None)
            .expect("無変換(C19→C1B)→英数(C1B→C19)は表にある");
        assert_eq!(p.effect.open, None);
        assert_eq!(
            p.effect.mode, None,
            "かな系のまま（belief を英数へ動かさない）"
        );
        assert_eq!(p.track.conv, Some(Conv::C19));
    }

    /// CI run 38094299141（sc-armc-gji-msimepreset-passthru、修正後も英数 15/16 FAIL）: 前のケースで全角カタカナに
    /// なったまま、無変換を押したまま英数を押した。全角カタカナの無変換は押下後の変換モードが表で不明なので、
    /// 英数を打鍵前のかなから引くと「半角英数」と外れる（実際はかな系）。途中の変換モードが分からないなら予測しない。
    #[test]
    fn predict_after_resent_gives_up_when_the_resent_key_leaves_an_unknown_conv() {
        let msime = KeyEffectKeymap::from_config(Some(2), None, &[]).unwrap();
        let katakana = input(
            true,
            ROMAJI,
            false,
            KeyTrack {
                conv: Some(Conv::C1B),
                stage: Stage::None,
            },
        );
        let step = msime
            .predict(0x1D, &katakana)
            .expect("全角カタカナの無変換は表にある");
        assert!(step.track.conv.is_none(), "押下後の変換モードは表で不明");
        assert!(
            msime.predict(0xF0, &katakana.after(step)).is_some(),
            "推測なら引けてしまう"
        );
        assert_eq!(
            msime.predict_after_resent(&[0x1D], 0xF0, &katakana, None),
            None
        );
    }

    /// 送り直しが無ければ従来の予測と同じ。送り直したキーが予測できない（例: overlay で無変換が上書きされうる）ときは、
    /// 途中の状態が分からないので予測しない（観測に任せる）。
    #[test]
    fn predict_after_resent_is_identity_without_resend_and_none_when_a_resent_key_is_unknown() {
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        for open in [true, false] {
            let i = input(open, ROMAJI, false, NOTRACK);
            for vk in [0x1C, 0x1D, 0xF0, 0xF2, 0x20, 0x41] {
                assert_eq!(
                    atok.predict_after_resent(&[], vk, &i, None),
                    atok.predict_with_override(vk, &i, None),
                    "open={open} vk=0x{vk:02X}"
                );
            }
        }
        let overlay = KeyEffectKeymap::from_config(Some(1), None, &[1]).unwrap();
        let kana = input(true, ROMAJI, false, NOTRACK);
        assert!(
            overlay.predict(0xF0, &kana).is_some(),
            "英数そのものは予測できる"
        );
        assert_eq!(
            overlay.predict_after_resent(&[0x1D], 0xF0, &kana, None),
            None,
            "overlay ありの無変換は予測しない → 重ねられないので全体を予測しない"
        );
    }

    /// 入力中（TSF の composing）に送り直した無変換が入力中の段階を進めたら、次の打鍵は追跡の段階で引く
    /// （打鍵前の composing で「入力中」に戻さない）。
    #[test]
    fn predict_after_resent_uses_tracked_stage_not_stale_composing() {
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        let typing = input(true, ROMAJI, true, NOTRACK);
        let step = atok.predict(0x1D, &typing);
        let Some(step) = step else {
            return; // ATOK の表に入力中の無変換が無い構成では、この性質は検査しない。
        };
        let after = typing.after(step);
        assert!(!after.composing);
        assert_eq!(after.track, step.track);
        let last = atok.predict(0x0D, &after).expect("送り直し後の Enter");
        let got = atok
            .predict_after_resent(&[0x1D], 0x0D, &typing, None)
            .expect("重ねた予測");
        assert_eq!(
            got.track, last.track,
            "追跡は最後の打鍵（送り直し後の状態で引いた Enter）のもの"
        );
        assert_eq!(got.effect.open, last.effect.open.or(step.effect.open));
        assert_eq!(
            got.effect.mode,
            last.effect.mode.or(step.effect.mode),
            "送り直した無変換の入力モードの効果は、後の打鍵が上書きしない限り残る"
        );
    }
}
