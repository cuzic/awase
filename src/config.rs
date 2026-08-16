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

/// IME 変換モード（英数/ひらがな/カタカナ × 半角/全角）を awase が積極的に
/// 強制するかどうかのポリシー。
///
/// `IME ON/OFF`（`ImeModel::desired_open`）とは独立した別軸の設定。conv
/// モードは 2026-08 時点で物理キーの意図しない漏洩（`docs/known-bugs.md`
/// BUG-52）等により実 IME 側と乖離することがあるため、`Force` はこの乖離を
/// cold 転換のたびに強制的に正すためのオプトイン機能。
///
/// `Observe`（デフォルト）は従来通り、awase は conv モードを観測するのみで
/// 能動的な強制書き込みは行わない。カタカナ/英数への追従警告は BUG-19 で
/// 自己増幅ループを起こした前例があるため、デフォルトは安全側（観測のみ）
/// のままにしている。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConvModePolicy {
    /// 観測のみ。awase は conv モードを能動的に書き換えない（従来動作）。
    #[default]
    Observe,
    /// 強制。cold 転換のたびに、トレイで選択した目標モードへ冪等に書き戻す。
    Force,
}

/// BUG-52 の「DBE レンジ」キーをパススルーしてよいかどうか（隠し設定、上級者向け）。
///
/// 物理 Hiragana/Katakana/Eisu キー等が生成する `VK_DBE_ALPHANUMERIC` /
/// `VK_DBE_KATAKANA` / `VK_DBE_SBCSCHAR` / `VK_DBE_DBCSCHAR`
/// （`crates/awase-windows/src/runtime/transport.rs`）が対象。
///
/// `VK_DBE_HIRAGANA`（かな入力キー本来の VK、F2 warmup 関連）はこの設定の
/// 対象外（別分岐で処理される、`transport.rs` 参照）。
///
/// 素のパススルーは、MS-IME の既定キー割当て（無変換単独打鍵→かな切替相当）や
/// OS 側キーボードレイアウト変換層の状態依存トグル（物理「IME ON」キーが
/// `VK_DBE_HIRAGANA` の代わりに `VK_DBE_KATAKANA` を生成することがある）に
/// 横取りされ、awase の管理外で IME モードが切り替わるリスクがある
/// （2026-08-05 実機、`docs/known-bugs.md` BUG-52）。既定値は `Suppress`
/// （常に抑制、現状維持）。
///
/// **`Passthrough` が実際に緩めるのは限定的**: `shadow_toggle` が発火した
/// KeyDown（awase 自身が意図した切替）と全 KeyUp は `Passthrough` でも
/// 引き続き Suppress される（`transport.rs::plan` 参照）。緩むのは
/// `shadow_toggle` 不発の KeyDown（＝ IME が既に目的の状態にあるのに OS が
/// 状態依存で `VK_DBE_*` を誤生成したケース、BUG-52 の再現条件そのもの）に
/// 限られる。また `ImmCross` プロファイル（LINE/Qt 等）では `plan` が
/// この判定に到達する前に別分岐で Suppress を決定するため、この設定は
/// そもそも無視される。[ADR-091](../docs/adr/091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
/// §D3.6 参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DbeModeKeyPolicy {
    /// 常に抑制する（OS に一切送出しない、従来動作）。
    #[default]
    Suppress,
    /// 素の VK をパススルーする（BUG-52 のリスクを引き受ける、上級者向け）。
    Passthrough,
}

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
    /// conv モード（英数/ひらがな/カタカナ × 半角/全角）強制ポリシー
    /// （デフォルト: observe＝従来通り観測のみ）
    pub conv_mode_policy: ConvModePolicy,
    /// 投機出力までの待機時間（ミリ秒、TwoPhase/AdaptiveTiming と
    /// NgramPredictive のフォールバック/投機待機で使用）
    pub speculative_delay_ms: u32,
    /// フォーカス遷移デバウンス時間（ミリ秒）。
    /// Alt-Tab 等でフォーカスが連続変更される際に IME 状態の誤検知を防ぐ。
    pub focus_debounce_ms: u32,
    /// IME 状態ポーリング間隔（ミリ秒）。
    /// イベント駆動の IME 検出を補完する安全ネット。
    pub ime_poll_interval_ms: u32,
    /// 自動起動の設定（"enabled" = 有効, "disabled" = 無効）
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
    /// `left_thumb_key`/`right_thumb_key` に無変換(`VK_NONCONVERT`)を割り当てている
    /// 場合に限り効く設定。変換キーや Space 等他の VK には一切影響しない。
    ///
    /// 無変換キー単独タップを、composing 中かどうかに関わらず常に完全に抑制する
    /// （OS に一切送出しない）。
    ///
    /// MS-IME は「キーとタッチのカスタマイズ」で無変換キー単独打鍵に既定で
    /// 「かな切替」（IME オン相当）を割り当てている。awase が composing して
    /// いない場面で無変換の生 VK を素通しすると、この既定割当てに横取りされて
    /// awase の管理外で IME モードが切り替わる（2026-08-07 実機: composing=false
    /// の無変換単独タップ直後に `VK_DBE_ALPHANUMERIC`→`VK_DBE_HIRAGANA` が非注入で
    /// 観測され、shadow toggle が IME を ON にした）。既定値は `true`
    /// （無変換単独タップは常に無視する）。無変換キー本来の機能（かな変換の
    /// 取り消し等）を Windows 全般で使いたい場合のみ `false` にする。
    pub muhenkan_solo_tap_always_suppress: bool,
    /// 無変換単独タップを、素の `VK_NONCONVERT` の代わりに専用 Fn キーへ
    /// 変換して送出する（隠し設定、上級者向け）。`None`（既定）なら無効で、
    /// `muhenkan_solo_tap_always_suppress`/`muhenkan_solo_tap_ignore_composing_guard`
    /// による従来の抑制/パススルー判定がそのまま適用される。
    ///
    /// `VkCode::from_name` が受理する完全な VK 名（例: `"VK_F21"`、`"F21"` の
    /// ような短縮形は不可）を指定する。`validate_dedicated_fn_key` が
    /// `VK_F15`-`VK_F24`（`VK_F13`/`VK_F14` を除く、物理キー非存在で安全、
    /// ADR-057）の範囲外を警告する（`VK_NONCONVERT`/`VK_IME_ON`/`VK_KANJI` 等の
    /// 危険なキー、およびターミナルエスケープシーケンス漏れが実機確認済みの
    /// `VK_F13`/`VK_F14` を避けるため）。`VK_F21`/`VK_F22` は BUG-64 の
    /// config1.db 残骸バインドと同番号のため、GJI 側の既存キー設定と
    /// 衝突していないか確認してから使うこと。
    ///
    /// 有効な場合は既存の抑制/パススルー判定より**手前**で分岐し、composing の
    /// 有無や `always_suppress` の値に関わらず常にこの Fn キーを送出する
    /// （Google 日本語入力の `config1.db` にこの Fn キーを Composition/
    /// Conversion 時の `SwitchKanaType` としてバインドしておくことで、GJI が
    /// 自身の内部状態を見てかな形状をトグルする。awase 側は belief を持たず、
    /// GJI 未対応の場面では単に何も起きない安全域のキーを送るだけ）。
    ///
    /// [ADR-091](../docs/adr/091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
    /// §D3.2 参照。
    pub muhenkan_solo_tap_dedicated_fn_key: Option<String>,
    /// BUG-52 の DBE レンジ Suppress（`VK_DBE_ALPHANUMERIC`/`KATAKANA`/
    /// `SBCSCHAR`/`DBCSCHAR`）を無条件抑制のままにするか、パススルーを
    /// 許すか（隠し設定、上級者向け）。既定値・リスクは [`DbeModeKeyPolicy`] 参照。
    pub dbe_mode_key_policy: DbeModeKeyPolicy,
    /// `left_thumb_key`/`right_thumb_key` に変換(`VK_CONVERT`)を割り当てている
    /// 場合に限り効く設定。無変換キーや Space 等他の VK には一切影響しない。
    ///
    /// 単独タップ（同時打鍵が不成立）確定時、IME の変換候補ウィンドウ表示中
    /// （`composing`）でも構わず生 VK_CONVERT を送出するか。既定値・注意点は
    /// `muhenkan_solo_tap_ignore_composing_guard` と同様。
    pub henkan_solo_tap_ignore_composing_guard: bool,
    /// `left_thumb_key`/`right_thumb_key` に変換(`VK_CONVERT`)を割り当てている
    /// 場合に限り効く設定。無変換キーや Space 等他の VK には一切影響しない。
    ///
    /// 変換キー単独タップを、composing 中かどうかに関わらず常に完全に抑制する
    /// （OS に一切送出しない）。既定値・注意点は `muhenkan_solo_tap_always_suppress`
    /// と同様（BUG-58 関連調査で判明: 従来 `henkan_solo_tap_ignore_composing_guard`
    /// は composing 中の挙動しか制御できず、composing していない場面では常に
    /// 生 VK_CONVERT が送出されていた。無変換と対称になるよう新設）。既定値は
    /// `true`（変換単独タップは常に無視する）。
    pub henkan_solo_tap_always_suppress: bool,
    /// `left_thumb_key`/`right_thumb_key` に Enter (`VK_RETURN`) を割り当てている
    /// 場合に限り効く設定。無変換/変換や Space 等他の VK には一切影響しない。
    ///
    /// 単独タップ（同時打鍵が不成立）確定時、IME の変換候補ウィンドウ表示中
    /// （`composing`）でも構わず生 VK_RETURN を送出するか。
    ///
    /// Enter は IME 変換候補の確定という正規機能を持つため、`space_thumb_ignore_composing_guard`
    /// と同じ理由で既定値は `true`（常時送出）。無変換/変換と同じ既定 `false` にすると、
    /// 変換候補ウィンドウ表示中の Enter 単独タップが丸ごと抑制され、通常の変換確定
    /// 操作そのものができなくなってしまう。
    ///
    /// この設定が `true` でも、フォーカス変更等コンテキスト境界を跨ぐフラッシュ
    /// （`ComposingHint::Unknown`、`nicola_fsm.rs` 参照）では常に suppress される。
    pub enter_thumb_ignore_composing_guard: bool,
    /// `left_thumb_key`/`right_thumb_key` に Enter (`VK_RETURN`) を割り当てている
    /// 場合に限り効く設定。無変換/変換や Space 等他の VK には一切影響しない。
    ///
    /// Shift を同時に押しながら Enter 親指キーを押した場合、同時打鍵判定を
    /// 一切試みず、`PendingThumb` にも入らず即座にリテラルな Enter（Shift+Enter の
    /// ソフト改行）として送出するか。既定値・注意点は `space_thumb_shift_literal`
    /// と同様（NICOLA の小指シフト面は Shift 単独系で thumb-shift とは組み合わせない
    /// 設計のため、Shift 押下中は安全に即時パススルーできる）。
    pub enter_thumb_shift_literal: bool,
    /// 物理 Alt を押しながら「かな」キー（`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`）を
    /// 押した際、MS-IME の「ローマ字入力 ⇔ JIS かな直接入力」切替ショートカット
    /// を OS へ渡さず未然に無効化するか（Windows 固有、`hook.rs` 参照）。
    ///
    /// JIS かな直接入力に切り替わると、awase が常時送出しているローマ字綴りの
    /// VK 列が MS-IME に誤読され、以後の日本語入力が壊れる（BUG-61: 一度
    /// 切り替わると awase 側から元に戻す公式 API が存在せず復旧不能、BUG-62
    /// 参照）。既定値は `true`（常に無効化）。JIS かな直接入力を意図的に
    /// 使いたい場合（= awase の Engine を OFF にして使う想定）のみ `false` にする。
    pub swallow_alt_kana_input_method_switch: bool,
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
            conv_mode_policy: ConvModePolicy::Observe,
            speculative_delay_ms: 30,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            auto_start: "enabled".to_string(),
            linux_input_backend: "evdev".to_string(),
            linux_evdev_device: None,
            keyboard_model: KeyboardModel::Jis,
            space_thumb_ignore_composing_guard: true,
            space_thumb_shift_literal: true,
            muhenkan_solo_tap_ignore_composing_guard: false,
            muhenkan_solo_tap_always_suppress: true,
            muhenkan_solo_tap_dedicated_fn_key: None,
            dbe_mode_key_policy: DbeModeKeyPolicy::Suppress,
            henkan_solo_tap_ignore_composing_guard: false,
            henkan_solo_tap_always_suppress: true,
            enter_thumb_ignore_composing_guard: true,
            enter_thumb_shift_literal: true,
            swallow_alt_kana_input_method_switch: true,
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
    /// IME トグル keys — IME の ON/OFF を反転するキーコンボ（ADR-092 決定D Step4a）
    ///
    /// `ime_on`/`ime_off`（方向固定）とは異なり、押した時点の実際の IME
    /// 状態（`InputContext::ime_on`、belief）を見て反転方向を決める。
    /// MS-IME の「キーとタッチのカスタマイズ」で Ctrl+Space/Shift+Space に
    /// 「IME ON/OFF」（トグル）を割り当てた場合の自動反映先。
    pub ime_toggle: Vec<String>,
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
    /// IME を全角/ひらがなモードに強制する。open 軸（IME の開閉）と
    /// charset 軸（全角/半角モード強制）を1つのキーで束ねる複合副作用キー
    /// であり、ADR-091 決定1（open 軸は `VK_IME_ON`/`VK_IME_OFF` で決着済み）
    /// より前の機構の残骸。既定 `None`（ADR-092 決定D Step1、2026-08-15）。
    /// 上級者が明示的に設定した場合のみ有効化される。
    pub engine_on_ime_key: Option<String>,
    /// Engine OFF 時に送信する IME モード切り替えキー（None で無効）
    ///
    /// エンジンが無効になったとき、このキーを `SendInput` で送信して
    /// IME を半角/直接入力モードに強制する。`engine_on_ime_key` と同種の
    /// 複合副作用キーの残骸。既定 `None`（ADR-092 決定D Step1）。
    pub engine_off_ime_key: Option<String>,
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            engine_on: vec!["Ctrl+Shift+変換".to_string()],
            engine_off: vec!["Ctrl+Shift+無変換".to_string()],
            ime_on: vec!["Ctrl+変換".to_string()],
            ime_off: vec!["Ctrl+無変換".to_string()],
            ime_toggle: vec!["VK_KANJI".to_string()],
            ime_detect: ImeDetectConfig::default(),
            engine_off_solo_triple: Some("VK_NONCONVERT".to_string()),
            engine_on_ime_key: None,
            engine_off_ime_key: None,
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

    /// 専用 Fn キー変換（ADR-091 §D3.2）の設定値が安全な範囲か検証する。
    ///
    /// 範囲を絞らないと `VK_NONCONVERT`（`muhenkan_solo_tap_always_suppress` を
    /// 迂回して素の無変換キーが常時飛ぶ、2026-08-07 実機の再発）や `VK_IME_ON`/
    /// `VK_KANJI`（belief を経ない open 軸 actuation が engine 層に生える）を
    /// 指定できてしまう。`VK_F13`/`VK_F14` は ADR-057 が実機（WezTerm/xterm）で
    /// ターミナルエスケープシーケンス漏れ・DirectInput ゲームとの競合を確認済みの
    /// 物理キーであり、config1.db の状態に関係なく危険なため除外する。
    ///
    /// `VK_F15`-`VK_F24`（F13/F14 を除く）は ADR-057 が WezTerm 実機で
    /// エスケープシーケンスを生成しないことを確認済みの Windows 予約 VK
    /// （物理キーボード対応なし）で、いずれも許可する。`VK_F21`/`VK_F22` は
    /// `docs/known-bugs.md` BUG-64 が記録する旧 ADR-057 設計の config1.db
    /// 残骸バインドと同じ番号だが、この残骸は 2026-08-13 に実機で確認・削除済み
    /// であり VK 自体が危険なわけではない。`awase-gji-config` の衝突検出機能
    /// （ADR-091 §4 Phase1-3、未実装）が入るまでは、GJI 側の既存キー設定に
    /// 同じ番号が使われていないかをユーザー自身が確認すること。
    fn validate_dedicated_fn_key(g: &GeneralConfig, w: &mut Vec<String>) {
        const SAFE_RANGE: &[&str] = &[
            "VK_F15", "VK_F16", "VK_F17", "VK_F18", "VK_F19", "VK_F20", "VK_F21", "VK_F22",
            "VK_F23", "VK_F24",
        ];
        if let Some(name) = &g.muhenkan_solo_tap_dedicated_fn_key {
            if !SAFE_RANGE.contains(&name.as_str()) {
                w.push(format!(
                    "muhenkan_solo_tap_dedicated_fn_key = {name:?} は安全な範囲外です \
                     （VK_F15〜VK_F24 のうち VK_F13/VK_F14 を除く番号のみ許可、\
                     ADR-091 §D3.2）。VK_NONCONVERT 等の危険なキーは指定しないこと。\
                     VK_F13/VK_F14 はターミナルエスケープシーケンス漏れの実機確認が \
                     あり常に避けること。VK_F21/VK_F22 を使う場合は、GJI 側の既存 \
                     キー設定（config1.db）で既に別の意味に割り当てられていないか \
                     確認すること（BUG-64 参照）。"
                ));
            }
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
        Self::validate_dedicated_fn_key(&general, &mut warnings);
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

    /// ADR-092 決定D Step1: engine_on_ime_key/engine_off_ime_key の既定値は
    /// 複合副作用キー（open + charset強制を1発で行う）の残骸であり、
    /// 既定 None に固定する（2026-08-15、実装時に既定値を変更）。
    #[test]
    fn test_keys_config_default_has_no_engine_ime_mode_keys() {
        let keys = KeysConfig::default();
        assert_eq!(keys.engine_on_ime_key, None);
        assert_eq!(keys.engine_off_ime_key, None);
    }

    /// 新規インストール（config.toml に該当キー未指定）は None のまま
    /// パースされる。既存 config.toml に明示値がある場合は
    /// `AppConfig::save` が全フィールドを明示出力する仕様上、この
    /// デフォルト変更は新規/未保存ユーザーにのみ効く（ADR-092 決定D Step1）。
    #[test]
    fn test_parse_app_config_engine_ime_keys_default_to_none() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.keys.engine_on_ime_key, None);
        assert_eq!(config.keys.engine_off_ime_key, None);
    }

    /// `keys.ime_toggle` の既定値は漢字キー（`VK_KANJI`）（2026-08-16
    /// ユーザー要望）。`VK_KANJI` は ADR-091 §1.2 で「Imm32Unavailable
    /// プロファイル向けの真のトグル」として既に確立済みの冪等な IME
    /// ON/OFF トグルキーであり、新設の GUI「IME ON/OFF トグル」欄の
    /// 既定候補として妥当（`msime_key_assignment.rs`のドキュメント参照）。
    #[test]
    fn test_keys_config_default_ime_toggle_is_kanji_key() {
        let keys = KeysConfig::default();
        assert_eq!(keys.ime_toggle, vec!["VK_KANJI".to_string()]);
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

    /// ADR-091 §D3.2: `VK_F15`-`VK_F24`（`VK_F13`/`VK_F14` を除く）は
    /// `validate_dedicated_fn_key` の安全範囲内で警告なし。`VK_F21`/`VK_F22` は
    /// BUG-64 の config1.db 残骸バインドと同番号だが、VK 自体は ADR-057 で
    /// ターミナル安全と確認済みのため許可範囲に含む（GJI 側の既存設定との
    /// 衝突確認はユーザーの責務、警告文に明記）。
    #[test]
    fn test_validate_dedicated_fn_key_safe_range_no_warning() {
        for vk in [
            "VK_F15", "VK_F16", "VK_F17", "VK_F18", "VK_F19", "VK_F20", "VK_F21", "VK_F22",
            "VK_F23", "VK_F24",
        ] {
            let mut general = GeneralConfig::default();
            general.muhenkan_solo_tap_dedicated_fn_key = Some(vk.to_string());
            let mut warnings = Vec::new();
            AppConfig::validate_dedicated_fn_key(&general, &mut warnings);
            assert!(warnings.is_empty(), "{vk} は警告なしで許可されるべき");
        }
    }

    /// `VK_F13`/`VK_F14`（ターミナルエスケープシーケンス漏れ実機確認済み、
    /// ADR-057）と、危険な VK（`VK_NONCONVERT` 等）は安全範囲外として警告する。
    #[test]
    fn test_validate_dedicated_fn_key_rejects_dangerous_and_terminal_unsafe_keys() {
        for vk in ["VK_F13", "VK_F14", "VK_NONCONVERT", "VK_IME_ON", "VK_KANJI"] {
            let mut general = GeneralConfig::default();
            general.muhenkan_solo_tap_dedicated_fn_key = Some(vk.to_string());
            let mut warnings = Vec::new();
            AppConfig::validate_dedicated_fn_key(&general, &mut warnings);
            assert_eq!(warnings.len(), 1, "{vk} は安全範囲外として警告されるべき");
        }
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

    // ── AppConfig::save (395): 実際にシリアライズしてファイルへ書き込むこと ──

    #[test]
    fn save_writes_full_toml_content_and_round_trips() {
        // save() 本体が `Ok(()) を返すだけの no-op` に置換されると、ファイルには
        // 何も書き込まれない（あるいは元の内容が残ったまま）になる。
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 123
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let path =
            std::env::temp_dir().join(format!("awase_test_save_{}.toml", std::process::id()));

        config.save(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let reloaded = AppConfig::load(&path);
        let _ = std::fs::remove_file(&path);

        assert!(
            content.contains("simultaneous_threshold_ms"),
            "save() must actually serialize the config to the file, got: {content}"
        );
        assert_eq!(reloaded.unwrap().general.simultaneous_threshold_ms, 123);
    }

    // ── validate_thresholds (426): speculative_delay_ms == threshold は境界内 ──

    #[test]
    fn test_validate_speculative_delay_equal_to_threshold_is_not_reset() {
        // `speculative_delay_ms > threshold` の `>` が `>=` に壊れると、ちょうど
        // 等しい場合まで誤ってリセットされてしまう。
        let toml_str = r#"
[general]
simultaneous_threshold_ms = 50
speculative_delay_ms = 50
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(
            validated.general.speculative_delay_ms, 50,
            "equal to threshold must not be reset"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("speculative_delay_ms")),
            "unexpected warning: {warnings:?}"
        );
    }

    // ── validate_thumb_keys (452-455): 4条件の `||` を個別に検証 ──

    #[test]
    fn test_validate_thumb_keys_warns_on_left_kana() {
        let toml_str = r#"
[general]
left_thumb_key = "Kana"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("ロック型")));
    }

    #[test]
    fn test_validate_thumb_keys_warns_on_left_vk_kana() {
        let toml_str = r#"
[general]
left_thumb_key = "VK_KANA"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("ロック型")));
    }

    #[test]
    fn test_validate_thumb_keys_warns_on_right_kana() {
        let toml_str = r#"
[general]
right_thumb_key = "Kana"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("ロック型")));
    }

    #[test]
    fn test_validate_thumb_keys_warns_on_right_vk_kana() {
        let toml_str = r#"
[general]
right_thumb_key = "VK_KANA"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("ロック型")));
    }

    #[test]
    fn test_validate_thumb_keys_no_warning_for_defaults() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(
            !warnings.iter().any(|w| w.contains("ロック型")),
            "default thumb keys must not warn, got: {warnings:?}"
        );
    }
}
