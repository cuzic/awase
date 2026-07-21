use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::scanmap::KeyboardModel;
use crate::types::VkCode;

// NOTE: かつて存在した設定項目（2026-07-06 撤去、旧 config.toml のキーは
// #[serde(default)] + 未知フィールド無視により残っていても無害）:
// - HookMode (hook_mode): Relay に一本化。Filter はリレー系機能（relay-defer/
//   INPUT_DEFER 対称性/NonText パススルー等）の登場以降テストされておらず撤去。
// - OutputMode (output_mode): per-window の InjectionMode（injection_hint + AppKind
//   から自動決定）に完全置換済みで、フィールドは書き込みのみの死に設定だった。
//
// keyboard_model は 2026-07-06 に「レイアウトパースが KeyboardModel::Jis 固定で
// 一度も配線されなかった」として撤去されたが、2026-07-08 に US 配列対応
// (scanmap の JIS/US テーブル分離・layout/nicola_us.yab 追加) と合わせて
// 実際に配線した上で再導入した。旧 config.toml の "jis"/"us" はそのまま解釈される。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmMode {
    /// 待機モード: タイムアウトまで出力を保留
    #[default]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // 設定ファイルの各トグル項目を1:1で表現
pub struct GeneralConfig {
    /// 同時打鍵の判定閾値（ミリ秒）
    pub simultaneous_threshold_ms: u32,
    /// 左親指キーのキー名
    pub left_thumb_key: String,
    /// 右親指キーの仮想キーコード名
    pub right_thumb_key: String,
    /// 有効/無効切り替えホットキー
    pub engine_toggle_hotkey: Option<String>,
    /// 配列定義ファイルの格納ディレクトリ
    pub layouts_dir: String,
    /// デフォルトの .yab レイアウトファイル名
    pub default_layout: String,
    /// n-gram コーパスファイル（オプション）
    pub ngram_file: Option<String>,
    /// n-gram 閾値調整幅（ミリ秒、デフォルト 20ms）
    pub ngram_adjustment_range_ms: u32,
    /// n-gram 適応閾値の下限（ミリ秒、デフォルト 30ms）
    pub ngram_min_threshold_ms: u32,
    /// n-gram 適応閾値の上限（ミリ秒、デフォルト 120ms）
    pub ngram_max_threshold_ms: u32,
    /// 確定モード（デフォルト: wait）
    pub confirm_mode: ConfirmMode,
    /// 投機出力までの待機時間（ミリ秒、TwoPhase/AdaptiveTiming と
    /// NgramPredictive のフォールバック/投機待機で使用）
    pub speculative_delay_ms: u32,
    /// フォーカス遷移デバウンス時間（ミリ秒）。
    /// Alt-Tab 等でフォーカスが連続変更される際に IME 状態の誤検知を防ぐ。
    pub focus_debounce_ms: u32,
    /// IME 状態ポーリング間隔（ミリ秒）。
    /// イベント駆動の IME 検出を補完する安全ネット。
    pub ime_poll_interval_ms: u32,
    /// 自動起動の設定（"ask" = 初回起動時に確認, "enabled" = 有効, "disabled" = 無効）
    pub auto_start: String,
    /// Linux 入力バックエンド ("evdev", "x11", "libinput")
    pub linux_input_backend: String,
    /// evdev バックエンド: キーボードデバイスパス（None = 自動検出）
    pub linux_evdev_device: Option<String>,
    /// キーボードの物理レイアウトモデル（"jis" または "us"）。
    ///
    /// .yab のパース時の列数上限チェックと、プラットフォーム層の
    /// スキャンコード⇔物理位置変換テーブルの選択に使う。
    ///
    /// "us" を指定する場合、既定の `left_thumb_key`/`right_thumb_key`
    /// （無変換/変換）や `[keys]` の既定ホットキーは US キーボードに
    /// 物理キーが存在しないため、明示的に上書きすること
    /// （上書きを忘れると `AppConfig::validate` が警告を返す）。
    ///
    /// 上書き先の VK 選定には注意が必要:
    ///
    /// - **`VK_LMENU`/`VK_RMENU`（Alt）を `left_thumb_key`/`right_thumb_key` に
    ///   直接指定することはできない。** `ModifierState::is_os_modifier_held()` で
    ///   「OS 予約修飾キー」とみなされ、`bypass_reason` がそのキーの KeyDown を
    ///   即座に `OsModifierHeld` として素通しするため、`PendingThumb` に一切入らず
    ///   同時打鍵検出そのものが機能しない（`engine/tests.rs` の
    ///   `test_ctrl_alt_win_thumb_key_never_enters_pending_due_to_os_modifier_bypass`
    ///   で確認済み）。**Alt を使いたい場合は下記の `"Left Alt"`/`"Right Alt"`
    ///   という特殊な値を使うこと**（VK 名を直接指定するのではなく、なりすまし
    ///   機構経由で同じ問題を回避する）。
    /// - **`VK_LCONTROL`/`VK_RCONTROL`（Ctrl）・`VK_LWIN`/`VK_RWIN`（Win）は
    ///   使用不可。** 上記と同じ理由（`is_os_modifier_held()`）で同時打鍵検出が
    ///   機能しない。Alt と異なり、なりすまし機構は用意していない
    ///   （`ModifierState` の左右別トラッキングという設計変更が要る未実装機能）。
    /// - `VK_LSHIFT`/`VK_RSHIFT` は `is_os_modifier_held()` の対象外のため
    ///   `PendingThumb` には到達できるが、左Shift単独タップによる「IME-ON 半角英数」
    ///   持続トグル（`kp_stage_shift_conv_guard`、Windows platform 層）は
    ///   `VK_LSHIFT`/`VK_RSHIFT` を直接見て判定するため、これらを親指キーへ
    ///   割り当てる運用と衝突しないかは未検証。
    /// - 親指キーは「同時打鍵が不成立の単独タップ」時に生の VK を `SendInput` で
    ///   そのまま OS に送る設計（`nicola_fsm.rs` の `timeout_pending_thumb`）。
    ///   無変換/変換は JIS キーボードでは OS 的に無害だからこそ安全に機能している。
    /// - 現実的な代替は、プログラマブルキーボード側で予備キーを無変換/変換や
    ///   F13-F24 等の無害な VK に物理リマップした上で JIS 既定値のまま使うか、
    ///   `VK_SPACE`（単独タップ時に空白が誤挿入され得る）を使うこと。
    ///
    /// `default_layout` も、既定の `layout/nicola.yab`（JIS 版）ではなく
    /// `layout/nicola_us.yab`（US 版、列数が少ない）を指すよう変更が必要。
    ///
    /// `left_thumb_key`/`right_thumb_key` に特殊な値 `"Left Alt"`/`"Right Alt"` を
    /// 指定すると、物理 Left/Right Alt キーをエンジン ON 時に限り親指キーとして扱う
    /// 「なりすまし」機構が有効になる（Platform 層の実装は `hook.rs` の
    /// `resolve_thumb_key`/`apply_alt_impersonation` 参照）。独立したチェックボックス
    /// ではなくこの2つの候補として `left_thumb_key`/`right_thumb_key` の選択肢に
    /// 統合することで、値が一箇所（この2フィールド）だけに存在し、設定 GUI の
    /// 表示条件と実際の有効状態がズレる余地を無くしている。
    ///
    /// - US 配列にはスペースキーの両隣に無変換/変換キーが無いため、コミュニティでは
    ///   PowerToys 等の OS レベルのキーリマップツールで左右の Alt キーを無変換/変換
    ///   相当に置き換える運用が一般的（スペースの両隣という物理位置が JIS の
    ///   無変換/スペース/変換と一致するため）。この機能は同等のことを awase 単体で
    ///   完結させる。
    /// - **エンジン ON 時のみ発動する**: Alt キーの KeyDown/KeyUp が Platform 層の
    ///   フック（`hook.rs` の `hook_callback`、`classify_key`/Ctrl 消費追跡等より前）で
    ///   無変換/変換相当の VK に書き換えられてから以降の全パイプラインに流れる。
    ///   これにより `ModifierState::is_os_modifier_held()` の OS 予約修飾キー bypass
    ///   にも一切引っかからず、PowerToys 等の外部リマップと本質的に同じ効果を得る。
    /// - **エンジン OFF 時は通常の Alt として機能する**（Alt+Tab 等の OS
    ///   ショートカットを損なわない）。押下中に ON/OFF が切り替わっても、
    ///   新規押下時点の判定を離すまで保持するため、なりすまし状態が押下中に
    ///   ズレて Alt が stuck modifier になる事故は起きない（`hook.rs` 参照）。
    /// - 任意のキーを任意の VK に対応させる汎用リマップ機能ではない。Left/Right Alt
    ///   専用。それ以上の自由なリマップをしたい場合は PowerToys 等の外部ツールを使うこと。
    pub keyboard_model: KeyboardModel,
    /// `left_thumb_key`/`right_thumb_key` に `VK_SPACE`（Space）を割り当てている
    /// 場合に限り効く設定。無変換/変換など他の VK には一切影響しない。
    ///
    /// 単独タップ（同時打鍵が不成立）確定時、IME の変換候補ウィンドウ表示中
    /// （`composing`）でも構わず生 VK_SPACE を送出するか。
    ///
    /// `composing` ガードはもともと無変換/変換の誤爆（かな/カタカナ切替・
    /// 再変換）防止用に入れたものだが、Space の場合は composing 中に
    /// 生 VK_SPACE を送ることは MS-IME/Google 日本語入力とも「変換候補送り」
    /// という正規機能であり、無変換/変換と同じガードを適用すると通常の
    /// 変換操作そのものが壊れる。そのため既定値は `true`（常時送出）。
    ///
    /// この設定が `true` でも、フォーカス変更等コンテキスト境界を跨ぐフラッシュ
    /// （`ComposingHint::Unknown`、`nicola_fsm.rs` 参照）では常に suppress される。
    /// 別ウィンドウへの生 VK_SPACE 誤注入を防ぐための安全策で、ユーザーが設定できる
    /// 範囲ではない。
    pub space_thumb_ignore_composing_guard: bool,
    /// `left_thumb_key`/`right_thumb_key` に `VK_SPACE`（Space）を割り当てている
    /// 場合に限り効く設定。無変換/変換など他の VK には一切影響しない。
    ///
    /// Shift を同時に押しながら Space 親指キーを押した場合、同時打鍵判定を
    /// 一切試みず、`PendingThumb` にも入らず即座にリテラルなスペースとして
    /// 送出するか（NICOLA の小指シフト面は Shift 単独系で thumb-shift とは
    /// 組み合わせない設計のため、Shift 押下中は安全に即時パススルーできる）。
    pub space_thumb_shift_literal: bool,
    /// `left_thumb_key`/`right_thumb_key` に無変換(`VK_NONCONVERT`)を割り当てている
    /// 場合に限り効く設定。変換キーや Space 等他の VK には一切影響しない。
    ///
    /// 単独タップ（同時打鍵が不成立）確定時、IME の変換候補ウィンドウ表示中
    /// （`composing`）でも構わず生 VK_NONCONVERT を送出するか。
    ///
    /// composing 中のガードはもともと MS-IME のかな/カタカナ切替・再変換の
    /// 誤爆を防ぐための安全策として入れているため（`docs/known-bugs.md` BUG-25
    /// 参照）、既定値は `false`（従来通り composing 中は suppress）。単独タップで
    /// 無変換キー本来の機能（かな変換の取り消し等）を使いたい場合のみ `true` にする。
    ///
    /// この設定が `true` でも、フォーカス変更等コンテキスト境界を跨ぐフラッシュ
    /// （`ComposingHint::Unknown`、`nicola_fsm.rs` 参照）では常に suppress される。
    /// 別ウィンドウへの生 VK 誤注入を防ぐための安全策で、ユーザーが設定できる
    /// 範囲ではない。
    pub muhenkan_solo_tap_ignore_composing_guard: bool,
    /// `left_thumb_key`/`right_thumb_key` に変換(`VK_CONVERT`)を割り当てている
    /// 場合に限り効く設定。無変換キーや Space 等他の VK には一切影響しない。
    ///
    /// 単独タップ（同時打鍵が不成立）確定時、IME の変換候補ウィンドウ表示中
    /// （`composing`）でも構わず生 VK_CONVERT を送出するか。既定値・注意点は
    /// `muhenkan_solo_tap_ignore_composing_guard` と同様。
    pub henkan_solo_tap_ignore_composing_guard: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            simultaneous_threshold_ms: 100,
            left_thumb_key: "無変換".to_string(),
            right_thumb_key: "変換".to_string(),
            engine_toggle_hotkey: None,
            layouts_dir: "config".to_string(),
            default_layout: "nicola.yab".to_string(),
            ngram_file: Some("data/ngram_hiragana.csv.gz".to_string()),
            ngram_adjustment_range_ms: 20,
            ngram_min_threshold_ms: 30,
            ngram_max_threshold_ms: 120,
            confirm_mode: ConfirmMode::Wait,
            speculative_delay_ms: 30,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            auto_start: "ask".to_string(),
            linux_input_backend: "evdev".to_string(),
            linux_evdev_device: None,
            keyboard_model: KeyboardModel::Jis,
            space_thumb_ignore_composing_guard: true,
            space_thumb_shift_literal: true,
            muhenkan_solo_tap_ignore_composing_guard: false,
            henkan_solo_tap_ignore_composing_guard: false,
        }
    }
}

/// IME 検出設定（シャドウ IME 状態追跡用キー定義）
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ImeDetectConfig {
    /// Toggle keys (direction unknown, flip shadow state)
    pub toggle: Vec<String>,
    /// ON keys (IME is now ON / zenkaku)
    pub on: Vec<String>,
    /// OFF keys (IME is now OFF / hankaku)
    pub off: Vec<String>,
}

impl Default for ImeDetectConfig {
    fn default() -> Self {
        Self {
            toggle: vec!["漢字".to_string()],
            on: vec!["IMEオン".to_string()],
            off: vec!["IMEオフ".to_string()],
        }
    }
}

/// キーバインディング設定
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct KeysConfig {
    /// Engine ON keys (multiple combos allowed)
    pub engine_on: Vec<String>,
    /// Engine OFF keys (multiple combos allowed)
    pub engine_off: Vec<String>,
    /// IME ON keys — IME を ON にするキーコンボ
    pub ime_on: Vec<String>,
    /// IME OFF keys — IME を OFF にするキーコンボ
    pub ime_off: Vec<String>,
    /// IME 検出設定
    pub ime_detect: ImeDetectConfig,
    /// ソロ5連打でエンジン OFF するキー（None または空文字列で無効）
    ///
    /// モディファイア不要のキー名を1つ指定する（"VK_NONCONVERT" 等）。
    /// Ctrl スタック等でホットキーが効かなくなった場合の緊急回復用。
    /// 必要連打回数は `SOLO_OFF_TRIGGER_COUNT`（`src/engine/nicola_fsm.rs`）。
    pub engine_off_solo_triple: Option<String>,
    /// Engine ON 時に送信する IME モード切り替えキー（None で無効）
    ///
    /// エンジンが有効になったとき、このキーを `SendInput` で送信して
    /// IME を全角/ひらがなモードに切り替える。
    /// デフォルト: `"VK_DBE_DBCSCHAR"` (0xF4 = 全角モード)
    pub engine_on_ime_key: Option<String>,
    /// Engine OFF 時に送信する IME モード切り替えキー（None で無効）
    ///
    /// エンジンが無効になったとき、このキーを `SendInput` で送信して
    /// IME を半角/直接入力モードに切り替える。
    /// デフォルト: `"VK_DBE_SBCSCHAR"` (0xF3 = 半角モード)
    pub engine_off_ime_key: Option<String>,
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            engine_on: vec!["Ctrl+Shift+変換".to_string()],
            engine_off: vec!["Ctrl+Shift+無変換".to_string()],
            ime_on: vec!["Ctrl+変換".to_string()],
            ime_off: vec!["Ctrl+無変換".to_string()],
            ime_detect: ImeDetectConfig::default(),
            engine_off_solo_triple: Some("VK_NONCONVERT".to_string()),
            engine_on_ime_key: Some("VK_DBE_DBCSCHAR".to_string()),
            engine_off_ime_key: Some("VK_DBE_SBCSCHAR".to_string()),
        }
    }
}

/// アプリオーバーライドのエントリ（プロセス名とクラス名の組み合わせ）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppOverrideEntry {
    pub process: String,
    pub class: String,
}

/// `[[keymap]]` ショートカットインターセプトルール
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct KeymapRule {
    /// プロセス名（省略=全アプリ、大文字小文字無視）
    #[serde(default)]
    pub app: Option<String>,
    /// インターセプトするキーコンボ（例: "Ctrl+I"）
    pub from: String,
    /// 再注入するキー（例: "F7"）、省略=消費のみ
    #[serde(default)]
    pub to: Option<String>,
}

/// アプリ別の永続オーバーライド設定
///
/// - `force_text`: 常にテキスト入力として扱う (process, class) の組
/// - `force_bypass`: 常に非テキストとしてバイパスする組
/// - `force_vk`: ローマ字出力を VK キーストローク Batched モードで送る組（Chrome/Edge/Electron 等）
/// - `force_tsf`: ローマ字出力を VK キーストローク Sequential モードで送る組（WezTerm 等 TSF 直結アプリ）
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AppOverrides {
    #[serde(default)]
    pub force_text: Vec<AppOverrideEntry>,
    #[serde(default)]
    pub force_bypass: Vec<AppOverrideEntry>,
    #[serde(default)]
    pub force_vk: Vec<AppOverrideEntry>,
    #[serde(default)]
    pub force_tsf: Vec<AppOverrideEntry>,
}

/// Ctrl+key バイパス直後に次キーを NICOLA スキップするルール
///
/// `key` に指定した Ctrl+key が PassThrough になった直後、
/// 次の non-Ctrl 非修飾キー 1 つを NICOLA エンジンをスキップして
/// 直接 passthrough させる。
///
/// 例: tmux の prefix (Ctrl+J) → コマンドキー (n/p) で
/// NICOLA が n/p を横取りするのを防ぐ。
///
/// ```toml
/// [[post_bypass]]
/// key = "Ctrl+J"
/// process = "WindowsTerminal"   # wt.exe（省略=全アプリ）
/// class = ""                    # ウィンドウクラス（省略=全クラス）
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PostBypassRule {
    /// バイパストリガーキー（例: "Ctrl+J"）
    pub key: String,
    /// プロセス名フィルタ（省略=全アプリ、大文字小文字無視）
    #[serde(default)]
    pub process: String,
    /// ウィンドウクラスフィルタ（省略=全クラス、大文字小文字無視）
    #[serde(default)]
    pub class: String,
}

/// アプリケーション設定ファイル (config.toml) のトップレベル構造
///
/// レイアウト定義は .yab ファイルから読み込むため、
/// このファイルにはアプリ全体の設定のみを含む。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    pub general: GeneralConfig,
    #[serde(default)]
    pub keys: KeysConfig,
    #[serde(default)]
    pub app_overrides: AppOverrides,
    #[serde(default)]
    pub keymaps: Vec<KeymapRule>,
    /// Ctrl+key バイパス後に次キーを NICOLA スキップするルール一覧
    #[serde(default)]
    pub post_bypass: Vec<PostBypassRule>,
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
    /// 検証済みのキーバインディング設定
    pub keys: KeysConfig,
    /// 検証済みのアプリ別オーバーライド
    pub app_overrides: AppOverrides,
    /// キーマップインターセプトルール
    pub keymaps: Vec<KeymapRule>,
    /// Ctrl+key バイパス後に次キーを NICOLA スキップするルール
    pub post_bypass: Vec<PostBypassRule>,
}

impl AppConfig {
    fn validate_thresholds(g: &mut GeneralConfig, w: &mut Vec<String>) {
        if g.simultaneous_threshold_ms < 10 || g.simultaneous_threshold_ms > 500 {
            w.push(format!(
                "simultaneous_threshold_ms ({}) は 10-500 の範囲外です。100 にリセットします",
                g.simultaneous_threshold_ms
            ));
            g.simultaneous_threshold_ms = 100;
        }
        if g.speculative_delay_ms > g.simultaneous_threshold_ms {
            w.push(format!(
                "speculative_delay_ms ({}) が threshold ({}) を超えています。30 にリセットします",
                g.speculative_delay_ms, g.simultaneous_threshold_ms
            ));
            g.speculative_delay_ms = 30;
        }
    }

    fn validate_layouts(g: &mut GeneralConfig, w: &mut Vec<String>) {
        if g.layouts_dir.contains("..") {
            w.push(format!(
                "layouts_dir に '..' が含まれています: {}",
                g.layouts_dir
            ));
            g.layouts_dir = "layout".to_string();
        }
        if !g.default_layout.to_ascii_lowercase().ends_with(".yab") {
            w.push(format!(
                "default_layout は .yab で終わる必要があります: {}",
                g.default_layout
            ));
        }
    }

    fn validate_thumb_keys(g: &GeneralConfig, w: &mut Vec<String>) {
        if g.left_thumb_key == "Kana"
            || g.left_thumb_key == "VK_KANA"
            || g.right_thumb_key == "Kana"
            || g.right_thumb_key == "VK_KANA"
        {
            w.push(
                "Kana キーはロック型キーで KeyUp イベントが発生しません。\
                 親指キーとしての使用は推奨しません。"
                    .to_string(),
            );
        }
    }

    /// `keyboard_model = "us"` のとき、無変換/変換キー前提のデフォルト値が
    /// 残っていないか確認する。US キーボードにはこれらの物理キーが存在しない。
    fn validate_keyboard_model(g: &GeneralConfig, keys: &KeysConfig, w: &mut Vec<String>) {
        const JIS_ONLY_NEEDLES: &[&str] = &["無変換", "変換", "VK_NONCONVERT", "VK_CONVERT"];

        if g.keyboard_model != KeyboardModel::Us {
            return;
        }

        if g.default_layout.trim_end_matches(".yab") == "nicola" {
            w.push(
                "keyboard_model = \"us\" ですが default_layout が JIS 版の \"nicola.yab\" \
                 のままです。JIS 版は列数が US の上限を超えるためパースに失敗します。\
                 \"nicola_us.yab\" を指定してください。"
                    .to_string(),
            );
        }

        let mentions_jis_only = |s: &str| JIS_ONLY_NEEDLES.iter().any(|n| s.contains(n));

        let mut offending_fields: Vec<&str> = Vec::new();
        if mentions_jis_only(&g.left_thumb_key) {
            offending_fields.push("general.left_thumb_key");
        }
        if mentions_jis_only(&g.right_thumb_key) {
            offending_fields.push("general.right_thumb_key");
        }
        if keys.engine_on.iter().any(|s| mentions_jis_only(s)) {
            offending_fields.push("keys.engine_on");
        }
        if keys.engine_off.iter().any(|s| mentions_jis_only(s)) {
            offending_fields.push("keys.engine_off");
        }
        if keys.ime_on.iter().any(|s| mentions_jis_only(s)) {
            offending_fields.push("keys.ime_on");
        }
        if keys.ime_off.iter().any(|s| mentions_jis_only(s)) {
            offending_fields.push("keys.ime_off");
        }
        if keys
            .engine_off_solo_triple
            .as_deref()
            .is_some_and(mentions_jis_only)
        {
            offending_fields.push("keys.engine_off_solo_triple");
        }

        if !offending_fields.is_empty() {
            w.push(format!(
                "keyboard_model = \"us\" ですが、無変換/変換キー前提の既定値が \
                 次の項目に残っています: {}。US キーボードにはこれらの物理キーが \
                 存在しないため、config.toml で明示的に上書きしてください。\
                 注意: VK_LMENU/VK_RMENU（Alt）・VK_LCONTROL/VK_RCONTROL（Ctrl）・ \
                 VK_LWIN/VK_RWIN（Win）は使用不可（OS 予約修飾キーとして即座に \
                 素通しされ、同時打鍵検出が機能しない）。プログラマブルキーボードで \
                 無変換/変換や F13-F24 に物理リマップするか、VK_SPACE を検討してください。",
                offending_fields.join(", ")
            ));
        }
    }

    fn validate_linux_backend(g: &mut GeneralConfig, w: &mut Vec<String>) {
        if !["evdev", "x11", "libinput"].contains(&g.linux_input_backend.as_str()) {
            w.push(format!(
                "linux_input_backend \"{}\" は不正です。evdev/x11/libinput のいずれかを指定してください。evdev にリセットします",
                g.linux_input_backend
            ));
            g.linux_input_backend = "evdev".to_string();
        }
        if let Some(ref dev) = g.linux_evdev_device {
            if !dev.starts_with("/dev/") {
                w.push(format!(
                    "linux_evdev_device \"{dev}\" は /dev/ で始まる必要があります。自動検出にリセットします"
                ));
                g.linux_evdev_device = None;
            }
        }
    }

    fn validate_app_override_entries(overrides: &AppOverrides, w: &mut Vec<String>) {
        Self::check_override_list(&overrides.force_text, "force_text", w);
        Self::check_override_list(&overrides.force_bypass, "force_bypass", w);
        Self::check_override_list(&overrides.force_vk, "force_vk", w);
        Self::check_override_list(&overrides.force_tsf, "force_tsf", w);
    }

    fn check_override_list(list: &[AppOverrideEntry], list_name: &str, w: &mut Vec<String>) {
        for entry in list {
            if entry.process.is_empty() || entry.class.is_empty() {
                w.push(format!(
                    "app_overrides.{list_name} に空のエントリがあります"
                ));
            }
        }
    }

    /// 設定値を検証し、`ValidatedConfig` を返す。
    ///
    /// 不正な値がある場合は警告メッセージのリストと共に返す（厳密なエラーではなくデフォルト値にフォールバック）。
    #[must_use]
    pub fn validate(self) -> (ValidatedConfig, Vec<String>) {
        let mut warnings = Vec::new();
        let mut general = self.general;
        let app_overrides = self.app_overrides;

        Self::validate_thresholds(&mut general, &mut warnings);
        Self::validate_layouts(&mut general, &mut warnings);
        Self::validate_thumb_keys(&general, &mut warnings);
        Self::validate_keyboard_model(&general, &self.keys, &mut warnings);
        Self::validate_linux_backend(&mut general, &mut warnings);
        Self::validate_app_override_entries(&app_overrides, &mut warnings);

        (
            ValidatedConfig {
                general,
                keys: self.keys,
                app_overrides,
                keymaps: self.keymaps,
                post_bypass: self.post_bypass,
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
engine_toggle_hotkey = "Ctrl+Shift+F12"
layouts_dir = "layout"
default_layout = "nicola.yab"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.simultaneous_threshold_ms, 100);
        assert_eq!(config.general.layouts_dir, "layout");
        assert_eq!(config.general.default_layout, "nicola.yab");
        assert_eq!(
            config.general.engine_toggle_hotkey,
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
        assert_eq!(config.general.left_thumb_key, "無変換");
        assert_eq!(config.general.right_thumb_key, "変換");
        assert_eq!(config.general.default_layout, "nicola.yab");
        assert_eq!(config.general.layouts_dir, "config");
    }

    /// 撤去済みフィールド（output_mode / hook_mode）が
    /// 旧 config.toml に残っていてもパースが失敗しない（後方互換）。
    #[test]
    fn test_removed_fields_are_tolerated() {
        let toml_str = r#"
[general]
output_mode = "batched"
hook_mode = "filter"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.speculative_delay_ms, 30);
    }

    // ── keyboard_model テスト ──

    #[test]
    fn test_keyboard_model_defaults_to_jis() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.keyboard_model, KeyboardModel::Jis);
    }

    #[test]
    fn test_keyboard_model_us_parses() {
        let toml_str = r#"
[general]
keyboard_model = "us"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.keyboard_model, KeyboardModel::Us);
    }

    #[test]
    fn test_validate_us_keyboard_with_default_thumb_keys_warns() {
        let toml_str = r#"
[general]
keyboard_model = "us"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("left_thumb_key")));
        assert!(warnings.iter().any(|w| w.contains("engine_on")));
    }

    #[test]
    fn test_validate_us_keyboard_with_default_layout_warns() {
        let toml_str = r#"
[general]
keyboard_model = "us"
left_thumb_key = "VK_F16"
right_thumb_key = "VK_F17"

[keys]
engine_on = ["Ctrl+Shift+VK_F13"]
engine_off = ["Ctrl+Shift+VK_F14"]
ime_on = ["Ctrl+VK_F13"]
ime_off = ["Ctrl+VK_F14"]
engine_off_solo_triple = "VK_F15"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("nicola_us.yab")));
    }

    #[test]
    fn test_validate_us_keyboard_with_overridden_thumb_keys_is_clean() {
        let toml_str = r#"
[general]
keyboard_model = "us"
left_thumb_key = "VK_F16"
right_thumb_key = "VK_F17"
default_layout = "nicola_us.yab"

[keys]
engine_on = ["Ctrl+Shift+VK_F13"]
engine_off = ["Ctrl+Shift+VK_F14"]
ime_on = ["Ctrl+VK_F13"]
ime_off = ["Ctrl+VK_F14"]
engine_off_solo_triple = "VK_F15"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn test_validate_jis_keyboard_default_thumb_keys_is_clean() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
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
    fn test_load_app_config_file() {
        let path = Path::new("config.toml");
        if !path.exists() {
            return;
        }
        let config = AppConfig::load(path).unwrap();
        assert_eq!(config.general.default_layout, "nicola.yab");
        assert_eq!(config.general.layouts_dir, "layout");
    }

    // ── AppOverrides テスト ──

    #[test]
    fn test_app_overrides_default_empty() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.app_overrides.force_text.is_empty());
        assert!(config.app_overrides.force_bypass.is_empty());
        assert!(config.app_overrides.force_vk.is_empty());
    }

    #[test]
    fn test_app_overrides_force_vk_parse() {
        let toml_str = r#"
[general]

[app_overrides]
force_vk = [
    { process = "wezterm-gui.exe", class = "org.wezfurlong.wezterm" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.app_overrides.force_vk.len(), 1);
        assert_eq!(config.app_overrides.force_vk[0].process, "wezterm-gui.exe");
        assert_eq!(
            config.app_overrides.force_vk[0].class,
            "org.wezfurlong.wezterm"
        );
    }

    #[test]
    fn test_app_overrides_parse() {
        let toml_str = r#"
[general]

[app_overrides]
force_text = [
    { process = "browser", class = "WebContent" },
    { process = "editor", class = "TextArea" },
]
force_bypass = [
    { process = "launcher", class = "SearchBox" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.app_overrides.force_text.len(), 2);
        assert_eq!(config.app_overrides.force_text[0].process, "browser");
        assert_eq!(config.app_overrides.force_text[0].class, "WebContent");
        assert_eq!(config.app_overrides.force_text[1].process, "editor");
        assert_eq!(config.app_overrides.force_bypass.len(), 1);
        assert_eq!(config.app_overrides.force_bypass[0].process, "launcher");
        assert_eq!(config.app_overrides.force_bypass[0].class, "SearchBox");
    }

    #[test]
    fn test_app_overrides_partial() {
        let toml_str = r#"
[general]

[app_overrides]
force_text = [
    { process = "editor", class = "TextInput" },
]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.app_overrides.force_text.len(), 1);
        assert!(config.app_overrides.force_bypass.is_empty());
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

[app_overrides]
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
        assert_eq!(config.keys.engine_off, vec!["Ctrl+Shift+無変換"]);
        assert_eq!(config.keys.engine_on, vec!["Ctrl+Shift+変換"]);
    }

    #[test]
    fn test_engine_toggle_key_custom() {
        let toml_str = r#"
[general]

[keys]
engine_off = ["Ctrl+Shift+VK_F10"]
engine_on = ["Ctrl+VK_F10"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.keys.engine_off, vec!["Ctrl+Shift+VK_F10"]);
        assert_eq!(config.keys.engine_on, vec!["Ctrl+VK_F10"]);
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

[keys]
engine_on = ["VK_CONVERT", "Ctrl+VK_CONVERT"]
engine_off = ["Ctrl+VK_NONCONVERT", "VK_NONCONVERT"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.keys.engine_on.len(), 2);
        assert_eq!(config.keys.engine_off.len(), 2);
    }
}
