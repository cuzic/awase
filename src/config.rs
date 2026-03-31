use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::types::VkCode;

/// フックの動作モード
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HookMode {
    /// フィルター型: PassThrough キーは OS にそのまま通す。
    /// フック内で SendInput を呼ぶため、レイヤー分離は不完全だがレイテンシが低い。
    #[default]
    Filter,
    /// リレー型: 全キーを Consume し、メッセージループで再注入する。
    /// フック内で OS API を呼ばず、キー順序を FIFO で保証する。
    /// わずかな入力遅延が生じる可能性がある。
    Relay,
}

/// ローマ字出力の送信方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// 1文字ずつ個別にキーイベントを送信（他のフックとの互換性重視）
    PerKey,
    /// 全文字を1回にまとめて送信（高速、アトミック）
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
    /// キーボードの物理レイアウトモデル（"jis" or "us"）
    #[serde(default = "default_keyboard_model")]
    pub keyboard_model: String,

    /// 同時打鍵の判定閾値（ミリ秒）
    #[serde(default = "default_threshold")]
    pub simultaneous_threshold_ms: u32,

    /// 左親指キーのキー名
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

    /// フックの動作モード（デフォルト: filter）
    #[serde(default)]
    pub hook_mode: HookMode,

    /// Engine ON keys (multiple combos allowed)
    #[serde(default = "default_engine_on_keys")]
    pub engine_on_keys: Vec<String>,

    /// Engine OFF keys (multiple combos allowed)
    #[serde(default = "default_engine_off_keys")]
    pub engine_off_keys: Vec<String>,

    /// IME ON keys — IME を ON にするキーコンボ
    #[serde(default = "default_ime_control_on_keys")]
    pub ime_on_keys: Vec<String>,

    /// IME OFF keys — IME を OFF にするキーコンボ
    #[serde(default = "default_ime_control_off_keys")]
    pub ime_off_keys: Vec<String>,

    /// フォーカス遷移デバウンス時間（ミリ秒）。
    /// Alt-Tab 等でフォーカスが連続変更される際に IME 状態の誤検知を防ぐ。
    #[serde(default = "default_focus_debounce_ms")]
    pub focus_debounce_ms: u32,

    /// IME 状態ポーリング間隔（ミリ秒）。
    /// イベント駆動の IME 検出を補完する安全ネット。
    #[serde(default = "default_ime_poll_interval_ms")]
    pub ime_poll_interval_ms: u32,

    /// Linux 入力バックエンド ("evdev", "x11", "libinput")
    #[serde(default = "default_linux_input_backend")]
    pub linux_input_backend: String,

    /// evdev バックエンド: キーボードデバイスパス（None = 自動検出）
    #[serde(default)]
    pub linux_evdev_device: Option<String>,
}

fn default_keyboard_model() -> String {
    "jis".to_string()
}

/// NICOLA 規格の標準的な同時打鍵判定閾値（100ms）
const fn default_threshold() -> u32 {
    100
}

fn default_left_thumb() -> String {
    "Nonconvert".to_string()
}

fn default_right_thumb() -> String {
    "Convert".to_string()
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
    vec!["Ctrl+Shift+Convert".to_string()]
}

fn default_engine_off_keys() -> Vec<String> {
    vec!["Ctrl+Shift+Nonconvert".to_string()]
}

fn default_ime_control_on_keys() -> Vec<String> {
    vec!["Ctrl+Convert".to_string()]
}

fn default_ime_control_off_keys() -> Vec<String> {
    vec!["Ctrl+Nonconvert".to_string()]
}

const fn default_focus_debounce_ms() -> u32 {
    50
}

const fn default_ime_poll_interval_ms() -> u32 {
    500
}

fn default_linux_input_backend() -> String {
    "evdev".to_string()
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
    vec!["Kanji".to_string()]
}

fn default_ime_on_keys() -> Vec<String> {
    vec!["ImeOn".to_string()]
}

fn default_ime_off_keys() -> Vec<String> {
    vec!["ImeOff".to_string()]
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

        // Kana キーはロック型キーで KeyUp が発生しないため親指キーとして非推奨
        if general.left_thumb_key == "Kana"
            || general.left_thumb_key == "VK_KANA"
            || general.right_thumb_key == "Kana"
            || general.right_thumb_key == "VK_KANA"
        {
            warnings.push(
                "Kana キーはロック型キーで KeyUp イベントが発生しません。\
                 親指キーとしての使用は推奨しません。"
                    .to_string(),
            );
        }

        // linux_input_backend: must be one of "evdev", "x11", "libinput"
        if !["evdev", "x11", "libinput"].contains(&general.linux_input_backend.as_str()) {
            warnings.push(format!(
                "linux_input_backend \"{}\" は不正です。evdev/x11/libinput のいずれかを指定してください。evdev にリセットします",
                general.linux_input_backend
            ));
            general.linux_input_backend = "evdev".to_string();
        }

        // linux_evdev_device: if specified, must start with "/dev/"
        if let Some(ref dev) = general.linux_evdev_device {
            if !dev.starts_with("/dev/") {
                warnings.push(format!(
                    "linux_evdev_device \"{dev}\" は /dev/ で始まる必要があります。自動検出にリセットします"
                ));
                general.linux_evdev_device = None;
            }
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

/// キーコンボ（修飾キー + メインキー）のパース済みデータ。
///
/// プラットフォーム層が `vk_name_to_code` 等で解決して構築する。
/// Engine はこの構造体の VkCode を等値比較するのみ（値の検査はしない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedKeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: VkCode,
}

#[cfg(test)]
mod tests {
    use super::*;

    // vk_name_to_code / parse_hotkey / parse_key_combo テストは awase-windows に移動済み

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
        assert_eq!(config.general.left_thumb_key, "Nonconvert");
        assert_eq!(config.general.right_thumb_key, "Convert");
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
    { process = "browser", class = "WebContent" },
    { process = "editor", class = "TextArea" },
]
force_bypass = [
    { process = "launcher", class = "SearchBox" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.focus_overrides.force_text.len(), 2);
        assert_eq!(config.focus_overrides.force_text[0].process, "browser");
        assert_eq!(config.focus_overrides.force_text[0].class, "WebContent");
        assert_eq!(config.focus_overrides.force_text[1].process, "editor");
        assert_eq!(config.focus_overrides.force_bypass.len(), 1);
        assert_eq!(config.focus_overrides.force_bypass[0].process, "launcher");
        assert_eq!(config.focus_overrides.force_bypass[0].class, "SearchBox");
    }

    #[test]
    fn test_focus_overrides_partial() {
        let toml_str = r#"
[general]

[focus_overrides]
force_text = [
    { process = "editor", class = "TextInput" },
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

    // parse_key_combo テストは awase-windows に移動済み

    // ── engine_on/off_keys デフォルトテスト ──

    #[test]
    fn test_engine_toggle_key_defaults() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.general.engine_off_keys,
            vec!["Ctrl+Shift+Nonconvert"]
        );
        assert_eq!(config.general.engine_on_keys, vec!["Ctrl+Shift+Convert"]);
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

    // ── Linux 設定テスト ──

    #[test]
    fn test_linux_defaults() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.linux_input_backend, "evdev");
        assert_eq!(config.general.linux_evdev_device, None);
    }

    #[test]
    fn test_linux_custom_values() {
        let toml_str = r#"
[general]
linux_input_backend = "x11"
linux_evdev_device = "/dev/input/event3"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.linux_input_backend, "x11");
        assert_eq!(
            config.general.linux_evdev_device,
            Some("/dev/input/event3".to_string())
        );
    }

    #[test]
    fn test_linux_libinput_backend() {
        let toml_str = r#"
[general]
linux_input_backend = "libinput"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert!(warnings.iter().all(|w| !w.contains("linux_input_backend")));
        assert_eq!(validated.general.linux_input_backend, "libinput");
    }

    #[test]
    fn test_linux_invalid_backend_produces_warning() {
        let toml_str = r#"
[general]
linux_input_backend = "wayland"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("linux_input_backend")));
        assert_eq!(validated.general.linux_input_backend, "evdev");
    }

    #[test]
    fn test_linux_invalid_evdev_device_produces_warning() {
        let toml_str = r#"
[general]
linux_evdev_device = "not/a/dev/path"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("linux_evdev_device")));
        assert_eq!(validated.general.linux_evdev_device, None);
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
