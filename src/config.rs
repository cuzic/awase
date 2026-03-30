use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// ローマ字出力の送信方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// 1文字ずつ個別の SendInput 呼び出し（他のフックとの互換性重視）
    PerKey,
    /// 全文字を1回の SendInput にまとめて送信（高速、アトミック）
    Batched,
    /// ローマ字→ひらがなに変換して Unicode 文字として直接送信（IME 不要）
    #[default]
    Unicode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmMode {
    /// 待機モード: タイムアウトまで出力を保留
    Wait,
    /// 先行確定モード: 即座に出力、同時打鍵時に BS で差し替え
    Speculative,
    /// 二段タイマー: 短い待機→投機出力→差し替え
    TwoPhase,
    /// 連続中は待機、途切れたら投機
    AdaptiveTiming,
    /// n-gram 予測で投機/待機を動的切替
    NgramPredictive,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GeneralConfig {
    /// 同時打鍵の判定閾値（ミリ秒）
    #[serde(default = "default_threshold")]
    pub simultaneous_threshold_ms: u32,

    /// 左親指キーの仮想キーコード名
    #[serde(default = "default_left_thumb")]
    pub left_thumb_key: String,

    /// 右親指キーの仮想キーコード名
    #[serde(default = "default_right_thumb")]
    pub right_thumb_key: String,

    /// 有効/無効切り替えホットキー
    #[serde(default)]
    pub toggle_hotkey: Option<String>,

    /// 配列定義ファイルの格納ディレクトリ
    #[serde(default = "default_layouts_dir")]
    pub layouts_dir: String,

    /// デフォルトの .yab レイアウトファイル名
    #[serde(default = "default_layout")]
    pub default_layout: String,

    /// n-gram コーパスファイル（オプション）
    #[serde(default)]
    pub ngram_file: Option<String>,

    /// n-gram 閾値調整幅（ミリ秒、デフォルト 20ms）
    #[serde(default = "default_ngram_adjustment")]
    pub ngram_adjustment_range_ms: u32,

    /// n-gram 適応閾値の下限（ミリ秒、デフォルト 30ms）
    #[serde(default = "default_ngram_min_threshold")]
    pub ngram_min_threshold_ms: u32,

    /// n-gram 適応閾値の上限（ミリ秒、デフォルト 120ms）
    #[serde(default = "default_ngram_max_threshold")]
    pub ngram_max_threshold_ms: u32,

    /// 確定モード（デフォルト: wait）
    #[serde(default = "default_confirm_mode")]
    pub confirm_mode: ConfirmMode,

    /// 投機出力までの待機時間（ミリ秒、TwoPhase/AdaptiveTiming で使用）
    #[serde(default = "default_speculative_delay")]
    pub speculative_delay_ms: u32,

    /// ローマ字出力の送信方式（デフォルト: per_key）
    #[serde(default)]
    pub output_mode: OutputMode,

    /// Engine ON keys (multiple combos allowed)
    #[serde(default = "default_engine_on_keys")]
    pub engine_on_keys: Vec<String>,

    /// Engine OFF keys (multiple combos allowed)
    #[serde(default = "default_engine_off_keys")]
    pub engine_off_keys: Vec<String>,

    /// Engine ON 時に送信する IME 制御キー（VK コード名、デフォルト: VK_OEM_ENLW=0xF4）
    /// IME をひらがなモードに切り替える
    #[serde(default = "default_ime_on_key")]
    pub ime_on_vk: String,

    /// Engine OFF 時に送信する IME 制御キー（VK コード名、デフォルト: VK_OEM_AUTO=0xF3）
    /// IME を直接入力モードに切り替える
    #[serde(default = "default_ime_off_key")]
    pub ime_off_vk: String,

    /// キーリマップ: Ctrl+変換 → 指定 VK を送信（デフォルト: VK_OEM_ENLW=0xF4）
    /// エンジン状態に関係なく、Ctrl+変換 を押すと IME ON に切り替える
    #[serde(default = "default_ime_on_key")]
    pub ctrl_convert_remap_vk: String,

    /// キーリマップ: Ctrl+無変換 → 指定 VK を送信（デフォルト: VK_OEM_AUTO=0xF3）
    /// エンジン状態に関係なく、Ctrl+無変換 を押すと IME OFF に切り替える
    #[serde(default = "default_ime_off_key")]
    pub ctrl_nonconvert_remap_vk: String,
}

/// NICOLA 規格の標準的な同時打鍵判定閾値（100ms）
const fn default_threshold() -> u32 {
    100
}

fn default_left_thumb() -> String {
    "VK_NONCONVERT".to_string()
}

fn default_right_thumb() -> String {
    "VK_CONVERT".to_string()
}

fn default_layouts_dir() -> String {
    "config".to_string()
}

fn default_layout() -> String {
    "nicola.yab".to_string()
}

const fn default_ngram_adjustment() -> u32 {
    20
}

const fn default_ngram_min_threshold() -> u32 {
    30
}

const fn default_ngram_max_threshold() -> u32 {
    120
}

const fn default_confirm_mode() -> ConfirmMode {
    ConfirmMode::Wait
}

/// TwoPhase/AdaptiveTiming の投機出力待機時間（30ms: Phase 1 を短く保つ）
const fn default_speculative_delay() -> u32 {
    30
}

fn default_engine_on_keys() -> Vec<String> {
    vec!["Ctrl+Shift+VK_CONVERT".to_string()]
}

fn default_engine_off_keys() -> Vec<String> {
    vec!["Ctrl+Shift+VK_NONCONVERT".to_string()]
}

fn default_ime_on_key() -> String {
    "VK_OEM_ENLW".to_string() // 0xF4
}

fn default_ime_off_key() -> String {
    "VK_OEM_AUTO".to_string() // 0xF3
}

/// IME 同期設定（シャドウ IME 状態追跡用キー定義）
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ImeSyncConfig {
    /// Toggle keys (direction unknown, flip shadow state)
    #[serde(default = "default_ime_toggle_keys")]
    pub toggle_keys: Vec<String>,

    /// ON keys (IME is now ON / zenkaku)
    #[serde(default = "default_ime_on_keys")]
    pub on_keys: Vec<String>,

    /// OFF keys (IME is now OFF / hankaku)
    #[serde(default = "default_ime_off_keys")]
    pub off_keys: Vec<String>,
}

fn default_ime_toggle_keys() -> Vec<String> {
    vec!["VK_KANJI".to_string()]
}

fn default_ime_on_keys() -> Vec<String> {
    vec!["VK_DBE_DBCSCHAR".to_string(), "VK_IME_ON".to_string()]
}

fn default_ime_off_keys() -> Vec<String> {
    vec!["VK_DBE_SBCSCHAR".to_string(), "VK_IME_OFF".to_string()]
}

/// フォーカスオーバーライドのエントリ（プロセス名とクラス名の組み合わせ）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FocusOverrideEntry {
    pub process: String,
    pub class: String,
}

/// フォーカス判定の永続オーバーライド設定
///
/// `force_text` に指定した (process, class) の組み合わせは常にテキスト入力として扱い、
/// `force_bypass` に指定した組み合わせは常に非テキストとしてバイパスする。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FocusOverrides {
    #[serde(default)]
    pub force_text: Vec<FocusOverrideEntry>,
    #[serde(default)]
    pub force_bypass: Vec<FocusOverrideEntry>,
}

/// アプリケーション設定ファイル (config.toml) のトップレベル構造
///
/// レイアウト定義は .yab ファイルから読み込むため、
/// このファイルにはアプリ全体の設定のみを含む。
#[derive(Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub general: GeneralConfig,
    #[serde(default)]
    pub focus_overrides: FocusOverrides,
    #[serde(default)]
    pub ime_sync: ImeSyncConfig,
}

impl AppConfig {
    /// config.toml を読み込んでパースする
    ///
    /// # Errors
    ///
    /// ファイルの読み込みまたはパースに失敗した場合にエラーを返す。
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(config)
    }

    /// 設定を TOML 形式でファイルに保存する
    ///
    /// # Errors
    ///
    /// シリアライズまたはファイル書き込みに失敗した場合にエラーを返す。
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}

/// 検証済み設定（全値が妥当であることが保証される）
#[derive(Debug)]
pub struct ValidatedConfig {
    /// 検証済みの一般設定
    pub general: GeneralConfig,
    /// 検証済みのフォーカスオーバーライド
    pub focus_overrides: FocusOverrides,
    /// 検証済みの IME 同期設定
    pub ime_sync: ImeSyncConfig,
}

impl AppConfig {
    /// 設定値を検証し、`ValidatedConfig` を返す。
    ///
    /// 不正な値がある場合は警告メッセージのリストと共に返す（厳密なエラーではなくデフォルト値にフォールバック）。
    #[must_use]
    pub fn validate(self) -> (ValidatedConfig, Vec<String>) {
        let mut warnings = Vec::new();
        let mut general = self.general;

        // threshold: 10..=500
        if general.simultaneous_threshold_ms < 10 || general.simultaneous_threshold_ms > 500 {
            warnings.push(format!(
                "simultaneous_threshold_ms ({}) は 10-500 の範囲外です。100 にリセットします",
                general.simultaneous_threshold_ms
            ));
            general.simultaneous_threshold_ms = 100;
        }

        // speculative_delay: 0..=threshold
        if general.speculative_delay_ms > general.simultaneous_threshold_ms {
            warnings.push(format!(
                "speculative_delay_ms ({}) が threshold ({}) を超えています。30 にリセットします",
                general.speculative_delay_ms, general.simultaneous_threshold_ms
            ));
            general.speculative_delay_ms = 30;
        }

        // layouts_dir: no path traversal
        if general.layouts_dir.contains("..") {
            warnings.push(format!(
                "layouts_dir に '..' が含まれています: {}",
                general.layouts_dir
            ));
            general.layouts_dir = "layout".to_string();
        }

        // default_layout: must end with .yab
        if !general
            .default_layout
            .to_ascii_lowercase()
            .ends_with(".yab")
        {
            warnings.push(format!(
                "default_layout は .yab で終わる必要があります: {}",
                general.default_layout
            ));
        }

        // focus_overrides: process names not empty
        let focus_overrides = self.focus_overrides;
        for entry in &focus_overrides.force_text {
            if entry.process.is_empty() || entry.class.is_empty() {
                warnings.push("focus_overrides.force_text に空のエントリがあります".to_string());
            }
        }
        for entry in &focus_overrides.force_bypass {
            if entry.process.is_empty() || entry.class.is_empty() {
                warnings.push("focus_overrides.force_bypass に空のエントリがあります".to_string());
            }
        }

        (
            ValidatedConfig {
                general,
                focus_overrides,
                ime_sync: self.ime_sync,
            },
            warnings,
        )
    }
}

/// 仮想キーコード名（"VK_A" 等）を実際の u16 値に変換する
#[must_use]
pub fn vk_name_to_code(name: &str) -> Option<u16> {
    match name {
        // アルファベットキー
        "VK_A" => Some(0x41),
        "VK_B" => Some(0x42),
        "VK_C" => Some(0x43),
        "VK_D" => Some(0x44),
        "VK_E" => Some(0x45),
        "VK_F" => Some(0x46),
        "VK_G" => Some(0x47),
        "VK_H" => Some(0x48),
        "VK_I" => Some(0x49),
        "VK_J" => Some(0x4A),
        "VK_K" => Some(0x4B),
        "VK_L" => Some(0x4C),
        "VK_M" => Some(0x4D),
        "VK_N" => Some(0x4E),
        "VK_O" => Some(0x4F),
        "VK_P" => Some(0x50),
        "VK_Q" => Some(0x51),
        "VK_R" => Some(0x52),
        "VK_S" => Some(0x53),
        "VK_T" => Some(0x54),
        "VK_U" => Some(0x55),
        "VK_V" => Some(0x56),
        "VK_W" => Some(0x57),
        "VK_X" => Some(0x58),
        "VK_Y" => Some(0x59),
        "VK_Z" => Some(0x5A),

        // 数字キー
        "VK_0" => Some(0x30),
        "VK_1" => Some(0x31),
        "VK_2" => Some(0x32),
        "VK_3" => Some(0x33),
        "VK_4" => Some(0x34),
        "VK_5" => Some(0x35),
        "VK_6" => Some(0x36),
        "VK_7" => Some(0x37),
        "VK_8" => Some(0x38),
        "VK_9" => Some(0x39),

        // OEM キー
        "VK_OEM_PLUS" => Some(0xBB),
        "VK_OEM_COMMA" => Some(0xBC),
        "VK_OEM_MINUS" => Some(0xBD),
        "VK_OEM_PERIOD" => Some(0xBE),
        "VK_OEM_2" => Some(0xBF),   // /
        "VK_OEM_1" => Some(0xBA),   // ;
        "VK_OEM_3" => Some(0xC0),   // `
        "VK_OEM_4" => Some(0xDB),   // [
        "VK_OEM_5" => Some(0xDC),   // \
        "VK_OEM_6" => Some(0xDD),   // ]
        "VK_OEM_7" => Some(0xDE),   // '
        "VK_OEM_102" => Some(0xE2), // <> (日本語キーボードの＼)

        // 特殊キー
        "VK_SPACE" => Some(0x20),
        "VK_RETURN" => Some(0x0D),
        "VK_TAB" => Some(0x09),
        "VK_BACK" => Some(0x08),
        "VK_ESCAPE" => Some(0x1B),
        "VK_DELETE" => Some(0x2E),

        // 日本語入力関連
        "VK_CONVERT" => Some(0x1C), // 変換
        #[allow(clippy::match_same_arms)] // 意図的なエイリアス
        "VK_NONCONVERT" | "VK_MUHENKAN" => Some(0x1D), // 無変換
        "VK_KANA" => Some(0x15),    // かな
        "VK_KANJI" => Some(0x19),   // 半角/全角
        "VK_IME_ON" => Some(0x16),
        "VK_IME_OFF" => Some(0x1A),
        "VK_DBE_ALPHANUMERIC" => Some(0xF0),
        "VK_DBE_KATAKANA" => Some(0xF1),
        "VK_DBE_HIRAGANA" => Some(0xF2),
        "VK_DBE_SBCSCHAR" | "VK_OEM_AUTO" => Some(0xF3), // 半角モード
        "VK_DBE_DBCSCHAR" | "VK_OEM_ENLW" => Some(0xF4), // 全角モード

        // 修飾キー
        "VK_SHIFT" => Some(0x10),
        "VK_CONTROL" => Some(0x11),
        "VK_MENU" => Some(0x12), // Alt
        "VK_LSHIFT" => Some(0xA0),
        "VK_RSHIFT" => Some(0xA1),
        "VK_LCONTROL" => Some(0xA2),
        "VK_RCONTROL" => Some(0xA3),
        "VK_LMENU" => Some(0xA4),
        "VK_RMENU" => Some(0xA5),

        // ファンクションキー
        "VK_F1" => Some(0x70),
        "VK_F2" => Some(0x71),
        "VK_F3" => Some(0x72),
        "VK_F4" => Some(0x73),
        "VK_F5" => Some(0x74),
        "VK_F6" => Some(0x75),
        "VK_F7" => Some(0x76),
        "VK_F8" => Some(0x77),
        "VK_F9" => Some(0x78),
        "VK_F10" => Some(0x79),
        "VK_F11" => Some(0x7A),
        "VK_F12" => Some(0x7B),

        _ => None,
    }
}

/// ホットキー文字列をパースして修飾キーフラグと仮想キーコードに変換する。
///
/// 例: `"Ctrl+Shift+F12"` → `Some((0x0006, 0x7B))` (MOD\_CONTROL | MOD\_SHIFT, VK\_F12)
///
/// 修飾キー: Ctrl (`MOD_CONTROL` = 0x0002), Shift (`MOD_SHIFT` = 0x0004), Alt (`MOD_ALT` = 0x0001)
///
/// 最後のトークンがメインキーとして `vk_name_to_code` で解決される。
#[must_use]
pub fn parse_hotkey(s: &str) -> Option<(u32, u16)> {
    const MOD_ALT: u32 = 0x0001;
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_SHIFT: u32 = 0x0004;

    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers: u32 = 0;
    for &part in &parts[..parts.len() - 1] {
        match part {
            "Ctrl" | "Control" => modifiers |= MOD_CONTROL,
            "Shift" => modifiers |= MOD_SHIFT,
            "Alt" => modifiers |= MOD_ALT,
            _ => return None,
        }
    }

    let key_name = format!("VK_{}", parts.last()?);
    let vk = vk_name_to_code(&key_name)?;

    Some((modifiers, vk))
}

/// キーコンボ文字列をパースして (ctrl, shift, alt, vk_code) を返す。
///
/// `parse_hotkey` と異なり、キー名は `VK_` プレフィックス付きで指定する。
/// 例: `"Ctrl+VK_NONCONVERT"` → `Some(ParsedKeyCombo { ctrl: true, shift: false, alt: false, vk: 0x1D })`
///      `"VK_CONVERT"` → `Some(ParsedKeyCombo { ctrl: false, shift: false, alt: false, vk: 0x1C })`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedKeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: u16,
}

#[must_use]
pub fn parse_key_combo(s: &str) -> Option<ParsedKeyCombo> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    for &part in &parts[..parts.len() - 1] {
        match part {
            "Ctrl" | "Control" => ctrl = true,
            "Shift" => shift = true,
            "Alt" => alt = true,
            _ => return None,
        }
    }

    let key_name = *parts.last()?;
    let vk = vk_name_to_code(key_name)?;

    Some(ParsedKeyCombo {
        ctrl,
        shift,
        alt,
        vk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── vk_name_to_code テスト ──

    #[test]
    fn test_alphabet_keys() {
        assert_eq!(vk_name_to_code("VK_A"), Some(0x41));
        assert_eq!(vk_name_to_code("VK_Z"), Some(0x5A));
    }

    #[test]
    fn test_number_keys() {
        assert_eq!(vk_name_to_code("VK_0"), Some(0x30));
        assert_eq!(vk_name_to_code("VK_9"), Some(0x39));
    }

    #[test]
    fn test_oem_keys() {
        assert_eq!(vk_name_to_code("VK_OEM_PLUS"), Some(0xBB));
        assert_eq!(vk_name_to_code("VK_OEM_COMMA"), Some(0xBC));
        assert_eq!(vk_name_to_code("VK_OEM_MINUS"), Some(0xBD));
        assert_eq!(vk_name_to_code("VK_OEM_PERIOD"), Some(0xBE));
    }

    #[test]
    fn test_japanese_input_keys() {
        assert_eq!(vk_name_to_code("VK_CONVERT"), Some(0x1C));
        assert_eq!(vk_name_to_code("VK_NONCONVERT"), Some(0x1D));
        // エイリアス
        assert_eq!(vk_name_to_code("VK_MUHENKAN"), Some(0x1D));
        // NONCONVERT と MUHENKAN は同じ値
        assert_eq!(
            vk_name_to_code("VK_NONCONVERT"),
            vk_name_to_code("VK_MUHENKAN")
        );
    }

    #[test]
    fn test_unknown_key_returns_none() {
        assert_eq!(vk_name_to_code("VK_UNKNOWN"), None);
        assert_eq!(vk_name_to_code(""), None);
        assert_eq!(vk_name_to_code("INVALID"), None);
    }

    // ── parse_hotkey テスト ──

    #[test]
    fn test_parse_hotkey_ctrl_shift_f12() {
        let result = parse_hotkey("Ctrl+Shift+F12");
        assert_eq!(result, Some((0x0002 | 0x0004, 0x7B)));
    }

    #[test]
    fn test_parse_hotkey_ctrl_f1() {
        let result = parse_hotkey("Ctrl+F1");
        assert_eq!(result, Some((0x0002, 0x70)));
    }

    #[test]
    fn test_parse_hotkey_alt_shift_a() {
        let result = parse_hotkey("Alt+Shift+A");
        assert_eq!(result, Some((0x0001 | 0x0004, 0x41)));
    }

    #[test]
    fn test_parse_hotkey_single_key() {
        let result = parse_hotkey("F12");
        assert_eq!(result, Some((0, 0x7B)));
    }

    #[test]
    fn test_parse_hotkey_invalid_modifier() {
        assert!(parse_hotkey("Win+F12").is_none());
    }

    #[test]
    fn test_parse_hotkey_invalid_key() {
        assert!(parse_hotkey("Ctrl+UNKNOWN").is_none());
    }

    #[test]
    fn test_parse_hotkey_empty() {
        assert!(parse_hotkey("").is_none());
    }

    // ── AppConfig パーステスト ──

    #[test]
    fn test_parse_app_config() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 100
toggle_hotkey = "Ctrl+Shift+F12"
layouts_dir = "layout"
default_layout = "nicola.yab"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.simultaneous_threshold_ms, 100);
        assert_eq!(config.general.layouts_dir, "layout");
        assert_eq!(config.general.default_layout, "nicola.yab");
        assert_eq!(
            config.general.toggle_hotkey,
            Some("Ctrl+Shift+F12".to_string())
        );
    }

    #[test]
    fn test_parse_app_config_defaults() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.simultaneous_threshold_ms, 100);
        assert_eq!(config.general.left_thumb_key, "VK_NONCONVERT");
        assert_eq!(config.general.right_thumb_key, "VK_CONVERT");
        assert_eq!(config.general.default_layout, "nicola.yab");
        assert_eq!(config.general.layouts_dir, "config");
    }

    #[test]
    fn test_confirm_mode_deserialize() {
        let toml_str = r#"
[general]
confirm_mode = "two_phase"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.confirm_mode, ConfirmMode::TwoPhase);
    }

    #[test]
    fn test_confirm_mode_all_variants() {
        for (input, expected) in [
            ("wait", ConfirmMode::Wait),
            ("speculative", ConfirmMode::Speculative),
            ("two_phase", ConfirmMode::TwoPhase),
            ("adaptive_timing", ConfirmMode::AdaptiveTiming),
            ("ngram_predictive", ConfirmMode::NgramPredictive),
        ] {
            let toml_str = format!("[general]\nconfirm_mode = \"{input}\"");
            let config: AppConfig = toml::from_str(&toml_str).unwrap();
            assert_eq!(config.general.confirm_mode, expected);
        }
    }

    #[test]
    fn test_confirm_mode_default() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.confirm_mode, ConfirmMode::Wait);
        assert_eq!(config.general.speculative_delay_ms, 30);
    }

    #[test]
    fn test_load_app_config_file() {
        let path = Path::new("config.toml");
        if !path.exists() {
            return;
        }
        let config = AppConfig::load(path).unwrap();
        assert_eq!(config.general.default_layout, "nicola.yab");
        assert_eq!(config.general.layouts_dir, "layout");
    }

    // ── FocusOverrides テスト ──

    #[test]
    fn test_focus_overrides_default_empty() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.focus_overrides.force_text.is_empty());
        assert!(config.focus_overrides.force_bypass.is_empty());
    }

    #[test]
    fn test_focus_overrides_parse() {
        let toml_str = r#"
[general]

[focus_overrides]
force_text = [
    { process = "chrome.exe", class = "Chrome_RenderWidgetHostHWND" },
    { process = "firefox.exe", class = "MozillaWindowClass" },
]
force_bypass = [
    { process = "explorer.exe", class = "CabinetWClass" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.focus_overrides.force_text.len(), 2);
        assert_eq!(config.focus_overrides.force_text[0].process, "chrome.exe");
        assert_eq!(
            config.focus_overrides.force_text[0].class,
            "Chrome_RenderWidgetHostHWND"
        );
        assert_eq!(config.focus_overrides.force_text[1].process, "firefox.exe");
        assert_eq!(config.focus_overrides.force_bypass.len(), 1);
        assert_eq!(
            config.focus_overrides.force_bypass[0].process,
            "explorer.exe"
        );
        assert_eq!(
            config.focus_overrides.force_bypass[0].class,
            "CabinetWClass"
        );
    }

    #[test]
    fn test_focus_overrides_partial() {
        let toml_str = r#"
[general]

[focus_overrides]
force_text = [
    { process = "notepad.exe", class = "Edit" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.focus_overrides.force_text.len(), 1);
        assert!(config.focus_overrides.force_bypass.is_empty());
    }

    // ── validate テスト ──

    #[test]
    fn test_validate_threshold_out_of_range() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 1000
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.simultaneous_threshold_ms, 100);
        assert!(warnings
            .iter()
            .any(|w| w.contains("simultaneous_threshold_ms")));
    }

    #[test]
    fn test_validate_threshold_too_low() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 5
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.simultaneous_threshold_ms, 100);
        assert!(warnings
            .iter()
            .any(|w| w.contains("simultaneous_threshold_ms")));
    }

    #[test]
    fn test_validate_speculative_delay_exceeds_threshold() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 50
speculative_delay_ms = 80
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.speculative_delay_ms, 30);
        assert!(warnings.iter().any(|w| w.contains("speculative_delay_ms")));
    }

    #[test]
    fn test_validate_path_traversal() {
        let toml_str = r#"
[general]
layouts_dir = "../../../etc"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.layouts_dir, "layout");
        assert!(warnings.iter().any(|w| w.contains("..")));
    }

    #[test]
    fn test_validate_default_layout_no_yab() {
        let toml_str = r#"
[general]
default_layout = "nicola.txt"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains(".yab")));
    }

    #[test]
    fn test_validate_empty_focus_override_entry() {
        let toml_str = r#"
[general]

[focus_overrides]
force_text = [
    { process = "", class = "Edit" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("force_text")));
    }

    #[test]
    fn test_validate_threshold_boundary_low() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 9
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.simultaneous_threshold_ms, 100);
        assert!(warnings
            .iter()
            .any(|w| w.contains("simultaneous_threshold_ms")));
    }

    #[test]
    fn test_validate_threshold_boundary_exact_low() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 10
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.simultaneous_threshold_ms, 10);
        assert!(!warnings
            .iter()
            .any(|w| w.contains("simultaneous_threshold_ms")));
    }

    #[test]
    fn test_validate_threshold_boundary_exact_high() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 500
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.simultaneous_threshold_ms, 500);
        assert!(!warnings
            .iter()
            .any(|w| w.contains("simultaneous_threshold_ms")));
    }

    #[test]
    fn test_validate_threshold_boundary_high() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 501
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.simultaneous_threshold_ms, 100);
        assert!(warnings
            .iter()
            .any(|w| w.contains("simultaneous_threshold_ms")));
    }

    #[test]
    fn test_validate_valid_config() {
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 100
speculative_delay_ms = 30
layouts_dir = "layout"
default_layout = "nicola.yab"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert!(warnings.is_empty());
        assert_eq!(validated.general.simultaneous_threshold_ms, 100);
        assert_eq!(validated.general.speculative_delay_ms, 30);
        assert_eq!(validated.general.layouts_dir, "layout");
        assert_eq!(validated.general.default_layout, "nicola.yab");
    }

    // ── parse_key_combo テスト ──

    #[test]
    fn test_parse_key_combo_ctrl_nonconvert() {
        let result = parse_key_combo("Ctrl+VK_NONCONVERT");
        assert_eq!(
            result,
            Some(ParsedKeyCombo {
                ctrl: true,
                shift: false,
                alt: false,
                vk: 0x1D,
            })
        );
    }

    #[test]
    fn test_parse_key_combo_convert_alone() {
        let result = parse_key_combo("VK_CONVERT");
        assert_eq!(
            result,
            Some(ParsedKeyCombo {
                ctrl: false,
                shift: false,
                alt: false,
                vk: 0x1C,
            })
        );
    }

    #[test]
    fn test_parse_key_combo_ctrl_shift_f12() {
        let result = parse_key_combo("Ctrl+Shift+VK_F12");
        assert_eq!(
            result,
            Some(ParsedKeyCombo {
                ctrl: true,
                shift: true,
                alt: false,
                vk: 0x7B,
            })
        );
    }

    #[test]
    fn test_parse_key_combo_invalid() {
        assert!(parse_key_combo("Ctrl+UNKNOWN").is_none());
        assert!(parse_key_combo("").is_none());
        assert!(parse_key_combo("Win+VK_A").is_none());
    }

    // ── engine_on/off_keys デフォルトテスト ──

    #[test]
    fn test_engine_toggle_key_defaults() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.engine_off_keys, vec!["Ctrl+VK_NONCONVERT"]);
        assert_eq!(config.general.engine_on_keys, vec!["VK_CONVERT"]);
    }

    #[test]
    fn test_engine_toggle_key_custom() {
        let toml_str = r#"
[general]
engine_off_keys = ["Ctrl+Shift+VK_F10"]
engine_on_keys = ["Ctrl+VK_F10"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.engine_off_keys, vec!["Ctrl+Shift+VK_F10"]);
        assert_eq!(config.general.engine_on_keys, vec!["Ctrl+VK_F10"]);
    }

    #[test]
    fn test_multiple_engine_keys() {
        let toml_str = r#"
[general]
engine_on_keys = ["VK_CONVERT", "Ctrl+VK_CONVERT"]
engine_off_keys = ["Ctrl+VK_NONCONVERT", "VK_NONCONVERT"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.engine_on_keys.len(), 2);
        assert_eq!(config.general.engine_off_keys.len(), 2);
    }
}
