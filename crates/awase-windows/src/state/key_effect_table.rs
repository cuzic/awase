//! ADR-191 決定3・4: 打鍵の時点で、(状態, キー)→効果の表を引いてbeliefを予測する**予測器**
//! （`predict()`、隠れ状態の追跡規則`KeyTrack`、キーマップ差の判定）。純粋関数だけで、状態は持たない。
//!
//! **このファイルは表のデータを持たない**（ファイル名は`_table`だが実体は予測器）。表のデータは
//! `key_effect_data.rs`（生成物）にある。
//!
//! awaseはIMEへ書かない。生キーはそのままIMEへ通り、ここでは**その結果を先取りして**beliefへ
//! 反映するための予測だけを返す（観測は後から確認・訂正する。`ime_model.rs`の`KeyEffectPredicted`）。
//!
//! # 出所（学習結果だけ）
//!
//! 表の中身は**手で書かない**。`tools/e2e/ime_key_matrix/gen_key_effect_table.py`が、CI実機の
//! `--grid`学習（awaseを完全にバイパスした注入。`grid-tables/{atok,msime}.json`）から生成する
//! `key_effect_data.rs`だけがデータ源である（ADR-191の3段階ラウンド: 設定の読み取り→学習→検証）。
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
//!   除外する（二重に効かせない）。0x16/0x1A（`VK_IME_ON`/`OFF`）は呼び出し側が追随の対象にしない。

use awase::engine::{AssumedReason, InputModeState};

/// 予測に使うキーマップの系統。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapPreset {
    Atok,
    /// GJIのMS-IMEプリセット（Microsoft IME本体ではない）。
    MsIme,
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
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    open: bool,
    conv: Option<Conv>,
    stage: Stage,
    key: TableKey,
    after_open: bool,
    after_conv: Option<Conv>,
    disp: Disp,
}

/// `key_effect_data.rs`（生成物）が使うセル構築子。
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

fn find(
    preset: KeymapPreset,
    open: bool,
    conv: Conv,
    stage: Stage,
    key: TableKey,
) -> Option<&'static Cell> {
    table_of(preset).iter().find(|c| {
        c.open == open && c.conv.is_none_or(|cv| cv == conv) && c.stage == stage && c.key == key
    })
}

const fn table_of(preset: KeymapPreset) -> &'static [Cell] {
    match preset {
        KeymapPreset::Atok => super::key_effect_data::ATOK,
        KeymapPreset::MsIme => super::key_effect_data::MSIME,
    }
}

/// 打鍵履歴から追跡する隠れ状態（`ImeModel`が`KeyEffectPredicted`で持つ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct KeyTrack {
    /// 直近の予測が示した変換モード。`None`なら観測（`prev_conversion_mode`）か既定から引く。
    pub conv: Option<Conv>,
    /// 入力中の段階（入力中でないときは無視され、`None`扱い）。
    pub stage: Stage,
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
        table_of(preset)
            .iter()
            .any(|c| c.open == input.open && c.conv.is_none_or(|cv| cv == conv) && c.stage == st)
    };
    let c = find(preset, input.open, conv, stage, key).or_else(|| {
        (matches!(
            stage,
            Stage::ConvSpace | Stage::ConvHenkan | Stage::ConvMuhenkan
        ) && !stage_modeled(stage))
        .then(|| find(preset, input.open, conv, Stage::Typing, key))
        .flatten()
    })?;
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

/// `config1.db`から得た、予測に使うキーマップ（プリセット+カスタム上書きの検出材料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEffectKeymap {
    preset: KeymapPreset,
    custom_table: Option<String>,
    has_overlay: bool,
}

/// Mozc `SessionKeymap`: `NONE=-1, CUSTOM=0, ATOK=1, MSIME=2`（`awase-gji-config`の定数と同じ値）。
const SESSION_KEYMAP_NONE: i64 = -1;
const SESSION_KEYMAP_ATOK: i64 = 1;
const SESSION_KEYMAP_MSIME: i64 = 2;

impl KeyEffectKeymap {
    /// `config1.db`の`session_keymap`（不在=`None`）・`custom_keymap_table`・`overlay_keymaps`から作る。
    /// プリセットがATOK/MSIME（不在/NONEはWindows版GJIの既定でMSIME相当）以外（CUSTOM・MOBILE等）は
    /// 基準の表が無いので`None`（予測しない）。
    #[must_use]
    pub fn from_config(
        session_keymap: Option<i64>,
        custom_table: Option<String>,
        overlay_keymaps: &[i64],
    ) -> Option<Self> {
        let preset = match session_keymap {
            Some(SESSION_KEYMAP_ATOK) => KeymapPreset::Atok,
            None | Some(SESSION_KEYMAP_NONE | SESSION_KEYMAP_MSIME) => KeymapPreset::MsIme,
            Some(_) => return None,
        };
        Some(Self {
            preset,
            custom_table,
            has_overlay: !overlay_keymaps.is_empty(),
        })
    }

    /// このキーマップでの、`vk`の打鍵の予測。カスタム表がそのキーの行を持つ、または overlay がある
    /// （無変換/変換は overlay `HENKAN_MUHENKAN_TO_IME_ON_OFF` が上書きしうる）ときは`None`。
    #[must_use]
    pub fn predict(&self, vk: u16, input: &PredictInput) -> Option<Prediction> {
        if self
            .custom_table
            .as_deref()
            .is_some_and(|t| custom_table_overrides(t, vk))
        {
            return None;
        }
        if self.has_overlay && matches!(vk, 0x1C | 0x1D) {
            return None;
        }
        predict(self.preset, vk, input)
    }
}

/// このVKのMozcキーイベント名（カスタムキーマップに同名の行があるかの判定用）。
/// 別名を含む（`key_parser.cc`）。
const fn mozc_tokens(vk: u16) -> &'static [&'static str] {
    match vk {
        0xF0 => &["eisu"],
        0xF1 => &["katakana"],
        0xF2 => &["kana", "hiragana"],
        0xF3 | 0xF4 => &["hankaku", "zenkaku", "hankaku/zenkaku"],
        0x1C => &["henkan"],
        0x1D => &["muhenkan"],
        0x08 => &["backspace"],
        0x0D => &["enter"],
        0x1B => &["escape"],
        0x20 => &["space"],
        _ => &[],
    }
}

/// カスタムキーマップTSV（`custom_keymap_table`）が、このVKのキーイベントの行を持つか。
#[must_use]
pub fn custom_table_overrides(custom_table: &str, vk: u16) -> bool {
    let tokens = mozc_tokens(vk);
    custom_table.lines().any(|line| {
        let mut cols = line.split('\t');
        let (_status, Some(key)) = (cols.next(), cols.next()) else {
            return false;
        };
        let key = key.trim().to_ascii_lowercase();
        tokens.contains(&key.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(open: bool, mode: InputModeState, composing: bool, track: KeyTrack) -> PredictInput {
        PredictInput {
            open,
            mode,
            conv_raw: None,
            composing,
            track,
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
        // CUSTOM・MOBILE 等は基準の表が無い。
        assert!(KeyEffectKeymap::from_config(Some(0), None, &[]).is_none());
        assert!(KeyEffectKeymap::from_config(Some(4), None, &[]).is_none());
        // ATOK + カスタム表が無変換の行を持つ → 無変換だけ予測しない。
        let table = "Precomposition\tMuhenkan\tIMEOn\n".to_string();
        let custom = KeyEffectKeymap::from_config(Some(1), Some(table), &[]).unwrap();
        assert_eq!(custom.predict(0x1D, &base), None);
        assert!(custom.predict(0xF2, &base).is_some());
        // overlay があると 無変換/変換 だけ予測しない。
        let ov = KeyEffectKeymap::from_config(Some(1), None, &[100]).unwrap();
        assert_eq!(ov.predict(0x1C, &base), None);
        assert!(ov.predict(0xF2, &base).is_some());
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
    }
}
