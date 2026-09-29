use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

use crate::key_text::{combo_main_identity, key_identity, split_combo};
use crate::scanmap::KeyboardModel;
use crate::types::VkCode;

// NOTE: かつて存在した設定項目（2026-07-06 撤去、旧 config.toml のキーは
// #[serde(default)] + 未知フィールド無視により残っていても無害）:
// - HookMode (hook_mode): Relay に一本化。Filter はリレー系機能（relay-defer/
//   INPUT_DEFER 対称性/NonText パススルー等）の登場以降テストされておらず撤去。
// - OutputMode (output_mode): per-window の InjectionMode（injection_hint + AppKind
//   から自動決定）に完全置換済みで、フィールドは書き込みのみの死に設定だった。
// - ConvModePolicy (conv_mode_policy): 2026-08-17、ADR-094 で charset 軸
//   （ひらがな/カタカナ×全角/半角の追跡）自体を撤去したのに伴い削除。
//   `Observe`/`Force` という2値のうち `Force`（conv モードの能動的な強制
//   書き戻し）は charset 軸の存在が前提のため道連れになった。ADR-091 決定3
//   §D3.4「かな⇔英数の2値境界は別軸として残す」の対象（`is_eisu()`）は
//   config 設定なしで引き続き機能する。
//
// keyboard_model は 2026-07-06 に「レイアウトパースが KeyboardModel::Jis 固定で
// 一度も配線されなかった」として撤去されたが、2026-07-08 に US 配列対応
// (scanmap の JIS/US テーブル分離・layout/nicola_us.yab 追加) と合わせて
// 実際に配線した上で再導入した。旧 config.toml の "jis"/"us" はそのまま解釈される。

/// 打鍵列機能（`.yab` の `CtrlChord`/`InlineSequence`/`MacroRef`）を有効化するか。
///
/// ADR-115 決定8は既定 `Off` だったが、2026-09-13 に既定 `On` へ変更した
/// （経緯は ADR-115 決定8追補・ADR-109 参照）。
/// `CV`+16進数2桁（`CtrlChord`）・セル内 `+` 区切り（`InlineSequence`）・
/// `@`+マクロ名（`MacroRef`）はいずれも偶然一致しうるほど一般的な文字列ではなく、
/// 既存のやまぶき派生レイアウトでこの語彙を使うユーザー（Issue #118 報告者）に
/// とっては「意図しない暴発」ではなく素の目的（`layout/nicola_kakutei.yab` の
/// 句読点確定を含む）そのものである。`.yab` パーサ自体は常に新構文を認識するが、
/// `Off` にすると解決パス（`resolve_keystroke_syntax`）が
/// `CtrlChord`/`InlineSequence`/`MacroRef` を保持している元のセル
/// 生テキストから `Literal` へ差し替え、この機能導入前の挙動に戻す
/// （Ctrl+チョード等の解釈自体を望まないユーザー向けの明示的オプトアウト）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeystrokeSequencePolicy {
    Off,
    #[default]
    On,
}

/// 左Shift単独タップによる「IME-ON 半角英数」持続トグルをどの IME で許可するか。
///
/// 既定値 `MsImeOnly` は従来動作そのもの。設定GUIからは `Off`/`All` の
/// 二択チェックボックスとして操作する（`MsImeOnly` はGUIからは選べない、
/// config.toml に残っている場合のみ有効な中間値）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HalfWidthAlnumTogglePolicy {
    /// MS-IME / GJI ともに機能全体を止める。
    Off,
    /// 従来どおり MS-IME のみ許可する。
    #[default]
    MsImeOnly,
    /// MS-IME と Google 日本語入力の両方で許可する。
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmMode {
    /// 待機モード: タイムアウトまで出力を保留
    #[default]
    Wait,
    /// n-gram 予測で投機/待機を動的切替
    NgramPredictive,
}

// 旧値 `speculative` / `two_phase` / `adaptive_timing` は v2 で廃止（A2）。
// 既存 config.toml を読めるよう `Wait` として受ける（`#[serde(alias)]` は
// キー名用の KEY_ALIASES ガードに数えられるので手書きにしている）。
// 警告は `AppConfig::from_toml_str` が `load_warnings` に積む。
impl<'de> Deserialize<'de> for ConfirmMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "wait" | "speculative" | "two_phase" | "adaptive_timing" => Ok(Self::Wait),
            "ngram_predictive" => Ok(Self::NgramPredictive),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["wait", "ngram_predictive"],
            )),
        }
    }
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
    /// 3キー仲裁のタイミングマージン（%、デフォルト30）。char1→thumb→char2の
    /// 3キーが来た場合、d1(thumb-char1)とd2(char2-thumb)の差がこの割合を
    /// 超えればタイミングだけで確定し、n-gramタイブレークを行わない。
    pub timing_margin_percent: u32,
    /// 重なり不足判定のマージン（%、デフォルト0）。thumb押下からchar1解放までの
    /// 物理的な重なり時間が閾値のこの割合未満なら「重なり不足」とみなし、
    /// n-gramタイブレークに回す（無ければ単独打鍵扱い）。既定値0は
    /// ADR-112決定1（`docs/adr/112-keyup-lifecycle-fsm-delivery.md`）に合わせて
    /// 意図的に「常に重なり十分」＝この判定を実質無効化した値。数日の実機ソーク
    /// （KeyUpがFSMへ実際に届くようになった影響の確認）で不具合報告は無かった
    /// が、引き締めに必要な実測データ（重なり時間msの分布等）は未取得のため、
    /// ADR-112決定3は見送り、**この0を恒久的な既定値として確定した**
    /// （2026-08-31）。将来引き上げを検討する場合は実測データの収集から。
    /// 上級者設定から手動で上げることはできる。
    pub min_overlap_margin_percent: u32,
    /// 確定モード（デフォルト: wait）
    pub confirm_mode: ConfirmMode,
    /// 投機出力までの待機時間（ミリ秒、NgramPredictive のフォールバック/
    /// 投機待機で使用）
    pub speculative_delay_ms: u32,
    /// フォーカス遷移デバウンス時間（ミリ秒）。
    /// Alt-Tab 等でフォーカスが連続変更される際に IME 状態の誤検知を防ぐ。
    pub focus_debounce_ms: u32,
    /// IME 状態ポーリング間隔（ミリ秒）。
    /// イベント駆動の IME 検出を補完する安全ネット。
    pub ime_poll_interval_ms: u32,
    /// 自動起動の設定（"enabled" = 有効, "disabled" = 無効）
    pub auto_start: String,
    /// タスクトレイから右クリックした際に最新バージョンを確認する。
    pub update_check: bool,
    /// 状態依存のIMEモードキーを検出したときに警告する（ADR-192）。
    pub warn_state_dependent_mode_keys: bool,
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
    /// （`ThumbRawVkEmission::Denied`、`nicola_fsm.rs` 参照）では常に suppress される。
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
    /// （`ThumbRawVkEmission::Denied`、`nicola_fsm.rs` 参照）では常に suppress される。
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
    /// `VkCode::from_name` が受理するキー名（例: `"VK_F21"`、`"F21"`。`VK_` は任意、
    /// 大文字小文字は問わない。ADR-201）を指定する。`validate_dedicated_fn_key` が
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
    /// 左Shift単独タップによる「IME-ON 半角英数」持続トグルの許可範囲。
    ///
    /// 既定 `ms_ime_only` は従来動作を維持する。設定GUI（上級者向け設定）
    /// からは `off`/`all` の二択チェックボックスとして操作できる（実機ソーク
    /// 完了、2026-08-27）——`ms_ime_only` はGUIからは選べない中間値で、
    /// 既存ユーザーの config.toml に残っている場合のみ意味を持つ
    /// （チェックボックスを一切操作しなければ値は変わらない）。
    pub half_width_alnum_toggle: HalfWidthAlnumTogglePolicy,
    /// 打鍵列機能（ADR-115）の有効化。既定 `On`（2026-09-13〜、ADR-115 決定8
    /// 追補）。設定GUI（上級者向け設定）から `off`/`on` の二択チェックボックス
    /// として操作できる。`off` はこの構文（`.yab` の `CtrlChord`/
    /// `InlineSequence`/`MacroRef`）の解釈自体を望まないユーザー向けの
    /// 明示的オプトアウト。
    pub keystroke_sequence: KeystrokeSequencePolicy,
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
    /// （`ThumbRawVkEmission::Denied`、`nicola_fsm.rs` 参照）では常に suppress される。
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

    /// **非推奨（ADR-206、読み込み専用）**: 無変換単独タップ確定時の IME ON/OFF/Toggle（旧 ADR-153 決定1、隠し設定）。
    ///
    /// 値は読み込むが、エンジンには直接渡さない。そのキーが親指キー（`left_thumb_key`/`right_thumb_key`）に
    /// 割り当てられているときだけ、読込時にメモリ上で `keys.ime_on/off/toggle` に bare で書いたのと同じ扱いへ
    /// 移し（`GeneralConfig::legacy_thumb_solo_tap_actions`）、`validate` が非推奨の警告を出す。親指キーでなければ
    /// 効かない（IME が自分で処理する）。config.toml は書き換えない。
    ///
    /// 単独タップの扱いは ADR-206 の規則に従う: 開閉の役割（bare `keys.ime_*`、または GJI の CUSTOM 表で無変換/変換が
    /// トグル）があれば生キーを抑止して awase が絶対指定の ON/OFF を1回書き、なければ `ModeKeyConfig` の
    /// Suppress/Passthrough に従う。
    #[serde(default)]
    pub muhenkan_solo_tap_ime_action: Option<ShadowImeActionConfig>,
    /// `muhenkan_solo_tap_ime_action` と対称（変換キー用、非推奨）。
    #[serde(default)]
    pub henkan_solo_tap_ime_action: Option<ShadowImeActionConfig>,
    /// ADR-195段階4: `<config dir>/keymap-learn-table.json`（段階3永続化）が存在し
    /// 検証を通れば、それを`key_effect_predictor`が引く表として同梱表の代わりに使う。
    /// `false`にすると学習済み表があっても常に同梱表を使う（opt-out、M-b）。
    /// 現状は`config.toml`を直接編集する以外の切り替え手段は無い
    /// （awase-settingsのUIチェックボックスは未実装、フォローアップが必要）。
    #[serde(default = "default_use_learned_keymap_table")]
    pub use_learned_keymap_table: bool,
}

const fn default_use_learned_keymap_table() -> bool {
    true
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
            timing_margin_percent: 30,
            min_overlap_margin_percent: 0,
            confirm_mode: ConfirmMode::Wait,
            speculative_delay_ms: 30,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            auto_start: "enabled".to_string(),
            update_check: true,
            warn_state_dependent_mode_keys: true,
            linux_input_backend: "evdev".to_string(),
            linux_evdev_device: None,
            keyboard_model: KeyboardModel::Jis,
            space_thumb_ignore_composing_guard: true,
            space_thumb_shift_literal: true,
            muhenkan_solo_tap_ignore_composing_guard: false,
            muhenkan_solo_tap_always_suppress: true,
            muhenkan_solo_tap_dedicated_fn_key: None,
            half_width_alnum_toggle: HalfWidthAlnumTogglePolicy::MsImeOnly,
            keystroke_sequence: KeystrokeSequencePolicy::On,
            henkan_solo_tap_ignore_composing_guard: false,
            henkan_solo_tap_always_suppress: true,
            enter_thumb_ignore_composing_guard: true,
            enter_thumb_shift_literal: true,
            swallow_alt_kana_input_method_switch: true,
            muhenkan_solo_tap_ime_action: None,
            henkan_solo_tap_ime_action: None,
            use_learned_keymap_table: true,
        }
    }
}

impl GeneralConfig {
    /// ADR-206 決定4: 非推奨の `*_solo_tap_ime_action` のうち、そのキーが親指キー
    /// （`left_thumb_key`/`right_thumb_key`）に割り当てられているものだけを `(無変換, 変換)` で返す。
    /// 呼び出し側（Platform 層）が、該当キーの bare コンボを `keys.ime_on/off/toggle` に相当する形で
    /// メモリ上でだけ追加する。親指キーでないキーの旧設定は返さない（読み捨てて警告する）。
    #[must_use]
    pub fn legacy_thumb_solo_tap_actions(
        &self,
    ) -> (
        Option<crate::types::ShadowImeAction>,
        Option<crate::types::ShadowImeAction>,
    ) {
        let is_thumb = |canonical: &str| {
            [self.left_thumb_key.as_str(), self.right_thumb_key.as_str()]
                .into_iter()
                .any(|k| key_identity(k) == canonical)
        };
        (
            self.muhenkan_solo_tap_ime_action
                .filter(|_| is_thumb("NONCONVERT"))
                .map(ShadowImeActionConfig::to_core),
            self.henkan_solo_tap_ime_action
                .filter(|_| is_thumb("CONVERT"))
                .map(ShadowImeActionConfig::to_core),
        )
    }
}

/// `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` のTOML表現
/// （`"on"`/`"off"`/`"toggle"`、ADR-153 決定1）。
///
/// `awase::types::ShadowImeAction` という**プラットフォーム非依存コア型**への
/// 変換は、ここ（config 側の薄い層）に置く——`ADR-019` の層境界を守るため、
/// core 型に serde を直接付けない（`deserialize_keymap_to` と同じ様式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShadowImeActionConfig {
    On,
    Off,
    Toggle,
}

impl ShadowImeActionConfig {
    /// `awase::types::ShadowImeAction`（core 型）へ変換する。
    #[must_use]
    pub const fn to_core(self) -> crate::types::ShadowImeAction {
        match self {
            Self::On => crate::types::ShadowImeAction::TurnOn,
            Self::Off => crate::types::ShadowImeAction::TurnOff,
            Self::Toggle => crate::types::ShadowImeAction::Toggle,
        }
    }
}

/// 診断・自己修復系のキルスイッチ設定（issue #165）。
///
/// `[general]`（`GeneralConfig`）ではなく独立したセクションにしているのは、
/// ここに置く項目が「ユーザーの好み」ではなく「不具合発生時にビルド無しで
/// 無効化できる安全弁」という性質のものだけだから（`awase-settings` GUI には
/// 当面出さない、上級者向け `config.toml` 直接編集専用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct DiagnosticsConfig {
    /// hook watchdog（`TIMER_HOOK_WATCHDOG`）が hook_starved（issue #165、他プロセスの
    /// `WH_KEYBOARD_LL`が`CallNextHookEx`を呼ばず握りつぶす）を検知した際、キーボード
    /// フックを自己修復（`UnhookWindowsHookEx`→`SetWindowsHookExW`で再インストール）
    /// するかどうか。既定で有効。誤検知や環境固有の副作用が疑われる場合、この値を
    /// `false`にすることでビルド無しで無効化できる（自己修復以前の診断ログ出力自体は
    /// この設定に関係なく継続する）。
    pub hook_self_heal: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            hook_self_heal: true,
        }
    }
}

// 既定はすべて空（`#[derive(Default)]`）。経緯:
// - 2026-08-16: 「漢字」（VK_KANJI）を `toggle` の既定から外した。当時 `keys.ime_toggle` が同じ
//   VK_KANJI を既定で持っており、同一の物理キー押下に対して `kp_stage_shadow_ime_toggle`（このフィールド由来、
//   belief を反転）と `Engine::apply_special_key_match`（`keys.ime_toggle` 由来、反転後の belief を読んで
//   逆方向へ再反転しキーを consume）が二重に働き、「押しても IME が動かない」キーになっていた。
//   （2026-09-29 追記: `keys.ime_toggle` の既定も空になったので、既定同士の衝突は起きない。）
// - 2026-09-29（ADR-207、所有者決定）: `on`/`off` の既定（`IMEオン`/`IMEオフ` = VK_IME_ON/OFF）も空にした。
//   hook の静的 `shadow_action` が `kp_stage_shadow_ime_toggle` で `is_japanese_ime()` を問わず採用され
//   （`vk::is_static_idempotent_open_key`）、同じ追随を担うため冗長だった。明示した値はそのまま尊重される。

/// IME 検出設定（シャドウ IME 状態追跡用キー定義）
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ImeDetectConfig {
    /// Toggle keys (direction unknown, flip shadow state)
    pub toggle: Vec<String>,
    /// ON keys (IME is now ON / zenkaku)
    pub on: Vec<String>,
    /// OFF keys (IME is now OFF / hankaku)
    pub off: Vec<String>,
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
    /// モディファイア不要のキー名を1つ指定する（"VK_INSERT" 等）。
    /// Ctrl スタック等でホットキーが効かなくなった場合の緊急回復用。
    /// 必要連打回数は `SOLO_OFF_TRIGGER_COUNT`（`src/engine/nicola_fsm.rs`）。
    ///
    /// `left_thumb_key`/`right_thumb_key` と同じ VK にも、それ以外の任意の
    /// VK にも設定できる（`NicolaFsm::handle_bypass` が `KeyClass::Passthrough`
    /// 経路で独立にカウントするため、通常のキー動作を変えずに済む）。既定値は
    /// 2026-08-25 に `VK_NONCONVERT`（無変換）から `VK_INSERT` へ変更した——
    /// 無変換は既定で `left_thumb_key` でもあるため、ユーザーが独自に
    /// `keys.ime_on`/`ime_off` へ同じ無変換キーを追加設定すると、そちらが
    /// Phase 1（ホットキー層）で先に無条件消費してしまい、`muhenkan_solo_tap_*`
    /// もこのソロ連打判定も一切発火しなくなる実例が確認された（`docs/bug-reports-triage.md`
    /// report `01M0VC3B1NG9JCDWMJTNNK6YAK`）。`VK_INSERT` はどの既定キー割当てとも
    /// 重複せず、通常のタイピングで連打されることもない。
    ///
    /// フィールド名は 2026-08-25 に `engine_off_solo_triple` から改称した
    /// （実際の必要連打回数は 2026-07-08 の追補で 3→5 に変わっていたのに
    /// 名前だけ "triple" のまま取り残されていたため。ADR-055 追補参照）。
    /// `config.toml` に旧キー名で保存済みの既存ユーザーとの互換のため
    /// `serde(alias)` で旧名も引き続き受け付ける。
    #[serde(alias = "engine_off_solo_triple")]
    pub engine_off_solo_repeat: Option<String>,
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            engine_on: vec!["Ctrl+Shift+変換".to_string()],
            engine_off: vec!["Ctrl+Shift+無変換".to_string()],
            ime_on: vec!["Ctrl+変換".to_string()],
            ime_off: vec!["Ctrl+無変換".to_string()],
            // 既定は空（ADR-199 決定15、2026-09-29 所有者決定で確定）。「IME の設定に従う」
            // 原則のため、awase 自身の設定としては漢字キー（VK_KANJI）を能動的に
            // 消費しない。物理の 0x19 は JIS 配列で Alt+半角/全角として届くので、
            // 無修飾の `VK_KANJI` は Engine の照合（修飾の完全一致）には元々一致せず、
            // 一致するのはリマッパー等が出す無修飾の 0x19 だけだった。Alt+半角/全角は
            // `hook.rs` の静的 `Toggle`（GJI は `Hankaku/Zenkaku` 行から役割判定、
            // ADR-202）が担い続ける。既定に `VK_KANJI` があると、GJI では役割判定が
            // `explicit_overlap`（`has_bare_ime_combo`）で常に無効化されていた。
            ime_toggle: Vec::new(),
            ime_detect: ImeDetectConfig::default(),
            engine_off_solo_repeat: Some("VK_INSERT".to_string()),
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
///
/// `from`/`to` に指定できない vk がある（ADR-114 決定5、`KeymapTable::new` が
/// `tracing::warn!` して該当ルールを skip する）: 親指キー・IME 制御系 VK・Alt 系
/// VK（`from` の修飾子としての Alt を含む）・Win 系 VK・`VK_CAPITAL`・
/// Shift を `from` の主キーにすること。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct KeymapRule {
    /// プロセス名（省略=全アプリ）。大文字小文字を無視し、末尾の `.exe` の
    /// 有無どちらでも一致する完全一致（前方一致はしない）。
    #[serde(default)]
    pub app: Option<String>,
    /// インターセプトするキーコンボ（例: "Ctrl+VK_I"）。主キーは `VK_` 接頭辞
    /// 付きの名前が必要（`crate::vk::VkCodeExt::from_name` が解決できる形式）。
    pub from: String,
    /// 再注入するキー列（例: `["F7", "F8"]`）。空、または省略=消費のみ。
    ///
    /// ADR-130 決定1: deserialize は旧形式 `to = "F7"` と新形式
    /// `to = ["F7", "F8"]` の両方を受ける。serialize は設定全体の保存時に
    /// 常に配列形式へ正規化される。
    #[serde(default, deserialize_with = "deserialize_keymap_to")]
    pub to: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum KeymapToCompat {
    Single(String),
    Many(Vec<String>),
}

fn deserialize_keymap_to<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<KeymapToCompat>::deserialize(deserializer)? {
        Some(KeymapToCompat::Single(to)) => vec![to],
        Some(KeymapToCompat::Many(to)) => to,
        None => Vec::new(),
    })
}

/// `[[keystroke_macro]]` 名前付き打鍵列マクロ（ADR-115 決定2b）。
///
/// 複数キーで再利用する列、または将来ステップ種別が増える列を定義する。
/// 単発・局所的な列（句読点確定等）は `.yab` セル内 `+` 区切り
/// （`InlineSequence`）で書く——両者は排他ではなく、`+` 区切りの1セグメント
/// として `@name` を書くこともできる。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct KeystrokeMacro {
    /// マクロ名（`.yab` セルから `@name` で参照する）
    pub name: String,
    /// 順序付きの出力ステップ列。非ネスト（`@` 参照はマクロ内で禁止）。
    /// 各要素は `.yab` の1トークンと**全く同じ文字列表記**
    /// （`"'（'"`/`"CV4D"`/`"左"` 等）。空リスト、または全要素が許可リスト
    /// （`Literal`/`KeySequence`/`Special`/`CtrlChord`）で拒否された場合は
    /// バリデーションエラーにせず、そのマクロは `YabValue::None`
    /// （明示的な無出力）に解決される。
    #[serde(default)]
    pub steps: Vec<String>,
}

/// アプリ別の永続オーバーライド設定
///
/// - `force_text`: 常にテキスト入力として扱う (process, class) の組
/// - `force_bypass`: 常に非テキストとしてバイパスする組
/// - `force_vk`: ローマ字出力を VK キーストローク Batched モードで送る組（Chrome/Edge/Electron 等）
/// - `force_tsf`: ローマ字出力を VK キーストローク Sequential モードで送る組（WezTerm 等 TSF 直結アプリ）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppOverrides {
    #[serde(default)]
    pub force_text: Vec<AppOverrideEntry>,
    #[serde(default)]
    pub force_bypass: Vec<AppOverrideEntry>,
    #[serde(default)]
    pub force_vk: Vec<AppOverrideEntry>,
    #[serde(default)]
    pub force_tsf: Vec<AppOverrideEntry>,
    /// フォーカス中このプロセス名（大文字小文字無視、`.exe` 有無どちらでも一致）
    /// にマッチしたら awase を丸ごと無効化する（force_bypass と異なり class 指定不要、
    /// フックレベルで生キーをそのまま OS に通す）。
    ///
    /// 既定値に `mstsc.exe` を含む: リモートデスクトップ接続中にローカル側の
    /// awase が Ctrl キーの押しっぱなし状態を起こす既知の問題への対策
    /// （`docs/known-bugs.md` BUG-78）。空にすれば無効化できる。
    #[serde(default = "default_disable_apps")]
    pub disable_apps: Vec<String>,
    /// 入力中継ツールのプロセス名（大文字小文字無視、`.exe` 有無どちらでも一致）。
    ///
    /// マッチしたフォーカス先では awase は IME actuation を所有せず、文字変換自体は
    /// 通常どおり継続する。
    #[serde(default = "default_input_relay_apps")]
    pub input_relay_apps: Vec<String>,
}

impl Default for AppOverrides {
    fn default() -> Self {
        Self {
            force_text: Vec::new(),
            force_bypass: Vec::new(),
            force_vk: Vec::new(),
            force_tsf: Vec::new(),
            disable_apps: default_disable_apps(),
            input_relay_apps: default_input_relay_apps(),
        }
    }
}

/// `AppOverrides::disable_apps` の既定値。
fn default_disable_apps() -> Vec<String> {
    vec!["mstsc.exe".to_string()]
}

/// `AppOverrides::input_relay_apps` の既定値。
const fn default_input_relay_apps() -> Vec<String> {
    Vec::new()
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
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub keys: KeysConfig,
    #[serde(default)]
    pub app_overrides: AppOverrides,
    /// 診断・自己修復系のキルスイッチ（issue #165）。
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
    #[serde(default)]
    pub keymaps: Vec<KeymapRule>,
    /// Ctrl+key バイパス後に次キーを NICOLA スキップするルール一覧
    #[serde(default)]
    pub post_bypass: Vec<PostBypassRule>,
    /// 名前付き打鍵列マクロ一覧（ADR-115 決定2b）。
    #[serde(default)]
    pub keystroke_macro: Vec<KeystrokeMacro>,
    /// 旧表記 `[[keymap]]`（ADR-201 決定5）。`alias` にすると `[[keymap]]` と `[[keymaps]]` が
    /// 両方あるとき serde が読み込み全体を失敗させるので、別のフィールドで受けて
    /// [`AppConfig::from_toml_str`] が `keymaps` へ合流させる（合流後は空）。保存はしない。
    #[serde(default, rename = "keymap", skip_serializing)]
    legacy_keymap: Vec<KeymapRule>,
    /// 読み込み時に集めた診断（未知のキー・`[[keymap]]` の合流）。`validate()` が警告に加える。
    /// 設定ファイルの項目ではない（保存しない）。
    #[serde(skip)]
    load_warnings: Vec<String>,
    /// 読み込んだ config.toml に撤去済みで**効果があった**キー（`REMOVED_WITH_NOTICE`）が残っていたときの通知
    /// （ADR-207）。`load_warnings`（ログだけ）と違い、`validate()` が**警告**として返しトレイに出る。
    /// 設定の保存（`config_save::save_edit`）が該当キーをファイルから消すので、保存後は
    /// [`AppConfig::clear_removed_notices`] で落とす。保存しない。
    #[serde(skip)]
    removed_notices: Vec<String>,
}

/// `AppConfig::load` の失敗を UI 側の扱い分けができる粒度に分類した結果
/// （ADR-099 決定4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoadState {
    /// 正常に読み込めた。
    Loaded,
    /// ファイルが存在しない（初回起動など、警告不要の正常系）。
    NotFound,
    /// `NotFound` 以外の全ての失敗（parse error・`PermissionDenied`・共有
    /// 違反等）。危険側のデフォルトとして扱い、呼び出し元は警告表示・
    /// 保存前バックアップ・保存前確認を必須にすること。
    Dangerous(String),
}

/// `AppConfig::load` のエラーを `ConfigLoadState` に分類する。
///
/// 分類ルールは「`io::ErrorKind::NotFound` と確認できた場合のみ
/// `NotFound`、それ以外は種別を問わず `Dangerous`」の一つだけ。
/// `NotFound` 以外の I/O エラー（`PermissionDenied` 等）を誤って
/// `NotFound` 扱いにすると、危険な失敗が静かにデフォルト値へフォール
/// バックしてしまう（ADR-099 F4、round2 指摘 MF-2）。
#[must_use]
pub fn classify_load_error(e: &anyhow::Error) -> ConfigLoadState {
    let is_not_found = e
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound);
    if is_not_found {
        ConfigLoadState::NotFound
    } else {
        ConfigLoadState::Dangerous(e.to_string())
    }
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
        let config = Self::from_toml_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(config)
    }

    /// 設定テキストを読み込む**唯一の入口**（ADR-201 決定2・5）。
    ///
    /// `AppConfig::load`・不具合報告・テストはこれを使う（`toml::from_str` を直接呼ぶと
    /// `[[keymap]]` の合流も未知キーの検出も通らない）。
    /// - 未知のキー（`serde_ignored`）は `load_warnings` に入れ、`validate()` が警告に加える。
    ///   撤去済みのキー（[`crate::config_load_diag::is_removed_key`]）は警告しない。
    /// - 旧表記 `[[keymap]]` は `keymaps` の後ろへ連結して合流させる（両方あれば連結）。
    ///
    /// # Errors
    ///
    /// TOML として、または型として読めない場合にエラーを返す。
    pub fn from_toml_str(text: &str) -> Result<Self, toml::de::Error> {
        let mut ignored: Vec<String> = Vec::new();
        let mut config: Self =
            serde_ignored::deserialize(toml::de::Deserializer::new(text), |p| {
                ignored.push(p.to_string());
            })?;
        let default_table = toml::Table::try_from(Self::default()).unwrap_or_default();
        for path in ignored {
            // 効果があった撤去キーは専用の通知（未知キーの提案文より先に判定する。
            // 接頭辞ルールが `engine_on_ime_key` に `keys.engine_on` を提案してしまうため）。
            if let Some(msg) = crate::config_load_diag::removed_notice(&path) {
                config.removed_notices.push(msg.to_string());
                continue;
            }
            if crate::config_load_diag::is_removed_key(&path) {
                continue;
            }
            let (parent, _) = path.rsplit_once('.').unwrap_or(("", &path));
            let siblings = Self::known_keys_under(&default_table, parent);
            config
                .load_warnings
                .push(crate::config_load_diag::unknown_key_message(
                    &path, &siblings,
                ));
        }
        // 廃止済みの confirm_mode（A2）: serde alias で `Wait` として読まれているので、
        // 元の文字列を見て警告だけ積む。
        if let Some(old) = toml::from_str::<toml::Table>(text)
            .ok()
            .and_then(|t| {
                t.get("general")?
                    .get("confirm_mode")?
                    .as_str()
                    .map(str::to_owned)
            })
            .filter(|v| matches!(v.as_str(), "speculative" | "two_phase" | "adaptive_timing"))
        {
            config.load_warnings.push(format!(
                "confirm_mode \"{old}\" は廃止されました。wait として扱います\
                 （使える値は wait / ngram_predictive）"
            ));
        }
        if !config.legacy_keymap.is_empty() {
            let n = config.legacy_keymap.len();
            let both = !config.keymaps.is_empty();
            config.keymaps.append(&mut config.legacy_keymap);
            config.load_warnings.push(if both {
                format!(
                    "[[keymap]] {n} 件を [[keymaps]] と連結して読みました\
                     （[[keymap]] は旧表記です。[[keymaps]] にまとめてください）"
                )
            } else {
                format!(
                    "[[keymap]] {n} 件を [[keymaps]] として読みました\
                     （[[keymap]] は旧表記です）"
                )
            });
        }
        Ok(config)
    }

    /// 既定値を TOML の表にしたものから、`parent`（`""` は最上位、`"general"` 等）の
    /// 直下の既知のキー名を返す。`None` の項目は表に出ないので、提案の候補が少し減るだけ。
    fn known_keys_under(default_table: &toml::Table, parent: &str) -> Vec<String> {
        let mut cur = default_table;
        if !parent.is_empty() {
            for seg in parent.split('.') {
                match cur.get(seg).and_then(toml::Value::as_table) {
                    Some(t) => cur = t,
                    None => return Vec::new(),
                }
            }
        }
        cur.keys().cloned().collect()
    }

    /// 読み込み時の診断（未知のキー・`[[keymap]]` の合流）。`validate()` の警告にも含まれる。
    #[must_use]
    pub fn load_warnings(&self) -> &[String] {
        &self.load_warnings
    }

    /// 撤去済みで効果があったキーが config.toml に残っていた通知（ADR-207）。`validate()` の警告にも含まれる。
    #[must_use]
    pub fn removed_notices(&self) -> &[String] {
        &self.removed_notices
    }

    /// 設定の保存で撤去キーがファイルから消えた後に、通知を落とす。
    pub fn clear_removed_notices(&mut self) {
        self.removed_notices.clear();
    }

    /// 設定を TOML 形式でファイルに保存する
    ///
    /// 一時ファイルへ書き込み・fsync してから `rename` するアトミック書き込み
    /// （ADR-099 決定3、実体は [`crate::fs_atomic::write_atomic`]）。書き込み中の
    /// クラッシュ・強制終了・ディスクフルで `path` が不完全な内容のまま残ることを
    /// 構造的に防ぐ。`path` がシンボリックリンクなら実体側へ書き込み、既存
    /// ファイルのパーミッションを引き継ぐ。Windows の `rename` は宛先が他
    /// プロセス（AV スキャナ・OneDrive 等）に開かれていると失敗しうるため、
    /// 短いリトライ（50ms×最大4回＝最大200msブロック、初回試行と合わせて
    /// 最大5回試行）で緩和する。宛先が読み取り専用の場合はリトライしても
    /// 成功しないためリトライを省略し即座にエラーを返す。
    ///
    /// # Errors
    ///
    /// シリアライズ・一時ファイルへの書き込み・`rename` のいずれかに
    /// 失敗した場合にエラーを返す。
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        crate::fs_atomic::write_atomic(path, content.as_bytes())
    }

    /// キーが無いときに serde が読む既定値（`from_toml_str("")` の結果、ADR-201 決定3）。
    /// `GeneralConfig::default()` や同梱の `config.toml` の値とは食い違いうる。
    #[must_use]
    pub fn default_from_empty() -> Self {
        Self::from_toml_str("").unwrap_or_default()
    }

    /// `config.toml` を再読み込みし、`general.auto_start` だけを書き換えて保存する。
    ///
    /// 自動起動のON/OFFはトレイメニュー（`awase.exe`）と設定画面
    /// （`awase-settings.exe`、別プロセス）の両方から独立に切り替えられる。
    /// どちらも「フォームで編集中の他の未保存の変更」を巻き込まないよう、
    /// in-memory の `AppConfig` をそのまま保存するのではなく、この関数を
    /// 通して都度ディスクから読み直す（Opus敵対的レビュー指摘 Minor 11、2026-09-07）。
    ///
    /// ADR-201 決定3: 保存は `toml_edit`（[`crate::config_save::save_edit`]）で、
    /// 読み直した値から `auto_start` だけを変えた差分だけを書く（コメント・未知キー・
    /// 他の項目は触らない。`validate()` の正規化もここでは書かない）。
    ///
    /// 戻り値: `Some(warnings)` は保存に成功したことを示す（`warnings` は
    /// `validate()` が検出した他フィールドの警告、空なら警告なし）。`None`
    /// は読み込みまたは保存自体が失敗したことを示す（空の `Vec` と区別する
    /// ため `Option` にしてある — 呼び出し元は「警告0件で成功」と
    /// 「保存自体が失敗」を混同してはならない）。
    pub fn save_auto_start(path: &Path, value: &str) -> Option<Vec<String>> {
        let base = match Self::load(path) {
            Ok(config) => config,
            Err(e) => {
                tracing::error!("Failed to load config for saving auto_start: {e}");
                return None;
            }
        };
        let mut to_save = base.clone();
        to_save.general.auto_start = value.to_string();
        let (_, warnings) = to_save.clone().validate();
        for w in &warnings {
            tracing::warn!("Config validation warning while saving auto_start: {w}");
        }
        if let Err(e) = crate::config_save::save_edit(&to_save, &base, path) {
            tracing::error!("Failed to save auto_start config: {e}");
            return None;
        }
        Some(warnings)
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
    /// 診断・自己修復系のキルスイッチ（issue #165）。検証は行わない（bool のみ）。
    pub diagnostics: DiagnosticsConfig,
    /// キーマップインターセプトルール
    pub keymaps: Vec<KeymapRule>,
    /// Ctrl+key バイパス後に次キーを NICOLA スキップするルール
    pub post_bypass: Vec<PostBypassRule>,
    /// 名前付き打鍵列マクロ一覧（ADR-115 決定2b）。`AppConfig` から単純に
    /// 転送するのみで検証は行わない（`steps` の中身の妥当性は
    /// `resolve_keystroke_syntax` が読み込み時に判定し警告する、決定3）。
    pub keystroke_macro: Vec<KeystrokeMacro>,
}

impl From<ValidatedConfig> for AppConfig {
    /// 検証済み設定を保存・再表示可能な `AppConfig` へ戻す。
    ///
    /// `validate()` が行った正規化（例: `confirm_mode = "speculative"` →
    /// `two_phase` + `speculative_delay_ms=0`）を、保存先やUIの表示に
    /// 反映したい呼び出し元向け（/code-review指摘: `awase-settings` の
    /// `apply_confirmed()` が以前は警告文の生成にしか `validate()` の
    /// 戻り値を使わず、保存対象は未検証の生設定のままだった）。
    fn from(v: ValidatedConfig) -> Self {
        Self {
            general: v.general,
            keys: v.keys,
            app_overrides: v.app_overrides,
            diagnostics: v.diagnostics,
            keymaps: v.keymaps,
            post_bypass: v.post_bypass,
            keystroke_macro: v.keystroke_macro,
            legacy_keymap: Vec::new(),
            load_warnings: Vec::new(),
            removed_notices: Vec::new(),
        }
    }
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
        // リセット先は GeneralConfig::default() の値そのものを参照する
        // （/code-review指摘、PR #127、8回目: ここにハードコードした
        // リテラルとdefault()の値が別々に管理されると、決定3で
        // min_overlap_margin_percentの既定値を引き締める際に片方だけ
        // 更新し忘れ、範囲外値が古い既定へリセットされ続ける事故になる）。
        let defaults = GeneralConfig::default();
        Self::validate_percent_field(
            "timing_margin_percent",
            &mut g.timing_margin_percent,
            defaults.timing_margin_percent,
            w,
        );
        Self::validate_percent_field(
            "min_overlap_margin_percent",
            &mut g.min_overlap_margin_percent,
            defaults.min_overlap_margin_percent,
            w,
        );
    }

    /// `0..=100` の範囲外なら警告を積んで `default` にリセットする
    /// （/code-review指摘、PR #127: `timing_margin_percent`/
    /// `min_overlap_margin_percent` で同一形の検証がコピペされていた）。
    fn validate_percent_field(name: &str, value: &mut u32, default: u32, w: &mut Vec<String>) {
        if *value > 100 {
            w.push(format!(
                "{name} ({value}) は 0-100 の範囲外です。{default} にリセットします"
            ));
            *value = default;
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
        // `canonical_key_text` を通した完全一致（`from_name` と規則を揃える。ADR-201 決定1）。
        const SAFE_RANGE: &[&str] = &[
            "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
        ];
        if let Some(name) = &g.muhenkan_solo_tap_dedicated_fn_key {
            if !SAFE_RANGE.contains(&key_identity(name).as_str()) {
                w.push(format!(
                    "muhenkan_solo_tap_dedicated_fn_key = {name:?} は指定できない値です。\
                     指定できるのは F15〜F24（F13・F14 を除く）のいずれかです \
                     （例: \"VK_F15\"）。無変換キーなど、他の操作にすでに使われている \
                     キーは指定できません。F13・F14 は一部のターミナルソフトで別の \
                     文字として誤認識されることがあるため使用できません。F21・F22 \
                     を使う場合は、Google 日本語入力（GJI）側の既存のキー設定で、\
                     同じキーがすでに別の操作に割り当てられていないか確認してください。"
                ));
            }
        }
    }

    fn validate_thumb_keys(g: &GeneralConfig, w: &mut Vec<String>) {
        // `Kana`/`VK_KANA`/`かな`/`カナ`（大文字小文字・空白は問わない）はすべて同じ VK。
        if key_identity(&g.left_thumb_key) == "KANA" || key_identity(&g.right_thumb_key) == "KANA" {
            w.push(
                "Kana キーはロック型キーで KeyUp イベントが発生しません。\
                 親指キーとしての使用は推奨しません。"
                    .to_string(),
            );
        }
    }

    fn validate_thumb_key_in_ime_combos(g: &GeneralConfig, keys: &KeysConfig, w: &mut Vec<String>) {
        fn is_bare_same_key(combo: &str, thumb_key: &str) -> bool {
            // 修飾キーなし（`+` で区切って主キーだけ）で、主キーが親指キーと同じ組。
            let (mods, main) = split_combo(combo);
            mods.is_empty() && key_identity(main) == key_identity(thumb_key)
        }

        fn warn_for_field(field: &str, combos: &[String], thumb_key: &str, w: &mut Vec<String>) {
            if combos
                .iter()
                .any(|combo| is_bare_same_key(combo, thumb_key))
            {
                let canonical = key_identity(thumb_key);
                let is_supported = canonical == "NONCONVERT" || canonical == "CONVERT";
                let detail = if is_supported {
                    "このキーは同時打鍵かどうかの判定後、単独タップ確定時に強制ON/OFFが発火します。composing中も発火し、未確定文字列が破棄されるか確定されるかはIME実装に依存します。"
                } else if field == "keys.ime_on" {
                    "このキーは同時打鍵（親指シフト入力）にも使うキーなので、IME が \
                     ON になっている間は、まず同時打鍵かどうかの判定が優先されます。\
                     そのため、IME が ON の状態でこのキーだけを押しても IME は \
                     ON のままで変化しません（実害はありません。ただし設定した \
                     つもりの動作にはなりません）。IME が OFF の状態でこのキーだけを \
                     押した場合は、これまでどおり IME を ON にします（本来の主な \
                     用途はこちらです）。IME が ON の間もこのキー単体で操作したい \
                     場合は、他のキーに変更するか、Shift などと組み合わせて \
                     （例: Shift+このキー）設定し直してください。"
                } else {
                    "このキーは同時打鍵（親指シフト入力）にも使うキーなので、IME が \
                     ON になっている間は、まず同時打鍵かどうかの判定が優先されます。\
                     そのため、IME が ON の間はこの設定が働かず、このキーだけを \
                     押しても IME の OFF・切り替えはできません（実害はありません。\
                     ただし設定した意味がありません）。他のキーに変更するか、\
                     Shift などと組み合わせて（例: Shift+このキー）設定し直して \
                     ください。"
                };
                w.push(format!(
                    "{field} に、同時打鍵で使う親指キー（{thumb_key}）が、他のキーとの \
                     組み合わせなしでそのまま設定されています。{detail}"
                ));
            }
        }

        for thumb_key in [g.left_thumb_key.as_str(), g.right_thumb_key.as_str()] {
            warn_for_field("keys.ime_on", &keys.ime_on, thumb_key, w);
            warn_for_field("keys.ime_off", &keys.ime_off, thumb_key, w);
            warn_for_field("keys.ime_toggle", &keys.ime_toggle, thumb_key, w);
        }
    }

    /// 非推奨の `*_solo_tap_ime_action` が残っているときの警告（ADR-206 決定4）。
    fn validate_legacy_solo_tap_action(g: &GeneralConfig, keys: &KeysConfig, w: &mut Vec<String>) {
        // 同じキーの bare が `keys.ime_*` に既にあれば、旧設定は移行されず bare が優先される（ADR-206 決定4）。
        let has_bare = |canonical: &str| {
            [&keys.ime_on, &keys.ime_off, &keys.ime_toggle]
                .into_iter()
                .flatten()
                .any(|combo| {
                    let (mods, main) = split_combo(combo);
                    mods.is_empty() && key_identity(main) == canonical
                })
        };
        let (muhenkan_migrated, henkan_migrated) = g.legacy_thumb_solo_tap_actions();
        for (field, set, migrated, key_name, bare_present) in [
            (
                "muhenkan_solo_tap_ime_action",
                g.muhenkan_solo_tap_ime_action.is_some(),
                muhenkan_migrated.is_some(),
                "無変換",
                has_bare("NONCONVERT"),
            ),
            (
                "henkan_solo_tap_ime_action",
                g.henkan_solo_tap_ime_action.is_some(),
                henkan_migrated.is_some(),
                "変換",
                has_bare("CONVERT"),
            ),
        ] {
            if !set {
                continue;
            }
            if migrated && bare_present {
                w.push(format!(
                    "general.{field} は非推奨で、`keys.ime_on`/`ime_off`/`ime_toggle` に「{key_name}」が既にあるため無視されます（そちらが優先されます）。削除してください。"
                ));
            } else if migrated {
                w.push(format!(
                    "general.{field} は非推奨です。`keys.ime_on`/`ime_off`/`ime_toggle` に「{key_name}」を単独で書くか、削除してください。\
                     現在は、そこに単独で書いたのと同じ扱いで動いています。GJI の CUSTOM 表で{key_name}がトグルなら、設定なしで動きます。"
                ));
            } else {
                w.push(format!(
                    "general.{field} は非推奨で、{key_name}が親指キーに割り当てられていないため今後は効きません（IME が自分で処理します）。\
                     半角状態で「@」が出る場合は、{key_name}を親指キーに割り当ててください。"
                ));
            }
        }
    }

    /// `keyboard_model = "us"` のとき、無変換/変換キー前提のデフォルト値が
    /// 残っていないか確認する。US キーボードにはこれらの物理キーが存在しない。
    fn validate_keyboard_model(g: &GeneralConfig, keys: &KeysConfig, w: &mut Vec<String>) {
        if g.keyboard_model != KeyboardModel::Us {
            return;
        }

        let jis_only_default = matches!(
            g.default_layout.trim_end_matches(".yab"),
            "nicola" | "nicola_keytop" | "nicola_f" | "nicola_kb232" | "nicola_kakutei"
        );
        if jis_only_default {
            w.push(format!(
                "keyboard_model = \"us\" ですが default_layout が JIS配列専用の \
                 レイアウト \"{}\" のままです。このレイアウトは US キーボードでは \
                 正しく読み込めません。\"nicola_us.yab\" を指定してください。",
                g.default_layout
            ));
        }

        // 組み合わせは `split_combo` で主キーを取り出して完全一致（`contains` は使わない。
        // ADR-201 R3-3）。
        let mentions_jis_only =
            |s: &str| matches!(combo_main_identity(s).as_str(), "NONCONVERT" | "CONVERT");

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
            .engine_off_solo_repeat
            .as_deref()
            .is_some_and(mentions_jis_only)
        {
            offending_fields.push("keys.engine_off_solo_repeat");
        }

        if !offending_fields.is_empty() {
            w.push(format!(
                "keyboard_model = \"us\" ですが、無変換/変換キー前提の初期設定が \
                 次の項目に残っています: {}。US キーボードにはこれらの物理キーが \
                 存在しないため、config.toml で別のキーに変更してください。\
                 注意: Alt・Ctrl・Win（左右とも）は OS がすでに予約しているキーの \
                 ため、親指キーとしては使用できません（awase が同時打鍵として \
                 検出するより先に OS 側の機能として使われてしまいます）。物理的に \
                 キーを配置し直せるキーボードをお使いであれば無変換/変換や \
                 F13〜F24 の位置に割り当てる方法もありますが、そうでなければ \
                 スペースキーを親指キーにする設定を検討してください。",
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
        Self::check_disable_apps_list(&overrides.disable_apps, w);
        Self::check_input_relay_apps_list(&overrides.input_relay_apps, w);
    }

    /// `disable_apps` の空文字列エントリを警告する。
    ///
    /// 空文字列は `app_suppression::matches_disabled_app` が常に不一致として扱う
    /// ため実害はないが、設定ミスの手がかりとして警告だけ出す。
    fn check_disable_apps_list(list: &[String], w: &mut Vec<String>) {
        if list.iter().any(String::is_empty) {
            w.push("app_overrides.disable_apps に空のエントリがあります".to_string());
        }
    }

    /// `input_relay_apps` の空文字列エントリを警告する。
    fn check_input_relay_apps_list(list: &[String], w: &mut Vec<String>) {
        if list.iter().any(String::is_empty) {
            w.push("app_overrides.input_relay_apps に空のエントリがあります".to_string());
        }
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
        // 読み込み時の診断（未知のキー・`[[keymap]]` の合流）を先頭に置く。
        let mut warnings = self.load_warnings;
        // 撤去済みで効果があったキーの通知は、ログだけの `load_warnings` とは別に**警告**として返す（ADR-207）。
        warnings.extend(self.removed_notices);
        let mut general = self.general;
        let app_overrides = self.app_overrides;

        Self::validate_thresholds(&mut general, &mut warnings);
        Self::validate_layouts(&mut general, &mut warnings);
        Self::validate_thumb_keys(&general, &mut warnings);
        Self::validate_dedicated_fn_key(&general, &mut warnings);
        Self::validate_thumb_key_in_ime_combos(&general, &self.keys, &mut warnings);
        Self::validate_legacy_solo_tap_action(&general, &self.keys, &mut warnings);
        Self::validate_keyboard_model(&general, &self.keys, &mut warnings);
        Self::validate_linux_backend(&mut general, &mut warnings);
        Self::validate_app_override_entries(&app_overrides, &mut warnings);

        (
            ValidatedConfig {
                general,
                keys: self.keys,
                app_overrides,
                diagnostics: self.diagnostics,
                keymaps: self.keymaps,
                post_bypass: self.post_bypass,
                keystroke_macro: self.keystroke_macro,
            },
            warnings,
        )
    }
}

/// コア`awase`クレートに埋め込んだ、出荷時の`config.toml`（ADR-178 決定3）。
/// `GeneralConfig::default()`のserializeは代用しない
/// （`GeneralConfig::default()`は`layouts_dir: "config"`だが出荷時は
/// `"layout"`であり、項目が食い違う）。
const EMBEDDED_CONFIG_TOML: &str = include_str!("../config.toml");

/// コア`awase`クレートに埋め込んだ、同梱6ファイルの`.yab`（ADR-178 決定3）。
const EMBEDDED_LAYOUTS: &[(&str, &str)] = &[
    ("nicola.yab", include_str!("../layout/nicola.yab")),
    (
        "nicola_keytop.yab",
        include_str!("../layout/nicola_keytop.yab"),
    ),
    ("nicola_us.yab", include_str!("../layout/nicola_us.yab")),
    ("nicola_f.yab", include_str!("../layout/nicola_f.yab")),
    (
        "nicola_kb232.yab",
        include_str!("../layout/nicola_kb232.yab"),
    ),
    (
        "nicola_kakutei.yab",
        include_str!("../layout/nicola_kakutei.yab"),
    ),
];

/// `config_path`が存在しなければ、埋め込み既定値（[`EMBEDDED_CONFIG_TOML`]）
/// から生成する（ADR-178 決定2）。既に存在する場合は内容を一切比較・上書き
/// せず、何もしない——これが「バックアップと実ファイルの整合を取る」という
/// 問題自体を発生させない設計の核心（v2〜v13の複雑さの原因だった問題を
/// 構造的に回避する）。
///
/// # Errors
///
/// 書き込みに失敗した場合にエラーを返す。呼び出し元は失敗してもpanicせず、
/// 既存のエラー経路（`find_config_path`の`bail!`等）に委ねること。
pub fn ensure_config_exists(config_path: &Path) -> Result<()> {
    if config_path.exists() {
        return Ok(());
    }
    crate::fs_atomic::write_atomic(config_path, EMBEDDED_CONFIG_TOML.as_bytes())
}

/// `layouts_dir`に`.yab`拡張子のファイルが1本も無い場合、同梱6ファイルを
/// 埋め込み既定値（[`EMBEDDED_LAYOUTS`]）から生成する（ADR-178 決定2）。
///
/// 1本でも存在すれば何もしない——ユーザーが同梱配列の一部を削除して整理した
/// 状態を復活させないため。中身の妥当性（パース可能かどうか）は判定しない
/// （シンプルさを優先、v13が持っていた`KeyboardModel`全バリアント試行の
/// ような複雑な検証は行わない）。
///
/// 途中（3本目等）で書き込みが失敗した場合、**それまでに書いた分を削除して
/// エラーを返す**（`/code-review`指摘、v14 opusレビューMajor M5対応）——
/// 中途半端な本数のまま抜けると、次回起動時に`has_any_yab`が`true`になり
/// 「1本でもあれば何もしない」判定で永久に残り4本が生成されなくなる。
/// 全滅させて0本に戻すことで、次回起動時に全6本の生成を再試行できる。
///
/// # Errors
///
/// ディレクトリ作成・書き込みに失敗した場合にエラーを返す。呼び出し元は
/// 失敗してもpanicせず、既存のエラー経路（`show_no_layouts_dialog`等）に
/// 委ねること。
pub fn ensure_layouts_exist(layouts_dir: &Path) -> Result<()> {
    let has_any_yab = std::fs::read_dir(layouts_dir).is_ok_and(|entries| {
        entries.filter_map(std::result::Result::ok).any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("yab"))
        })
    });
    if has_any_yab {
        return Ok(());
    }
    std::fs::create_dir_all(layouts_dir)
        .with_context(|| format!("Failed to create {}", layouts_dir.display()))?;
    let mut written = Vec::with_capacity(EMBEDDED_LAYOUTS.len());
    for (name, content) in EMBEDDED_LAYOUTS {
        let path = layouts_dir.join(name);
        if let Err(e) = crate::fs_atomic::write_atomic(&path, content.as_bytes()) {
            for p in &written {
                let _ = std::fs::remove_file(p);
            }
            return Err(e);
        }
        written.push(path);
    }
    Ok(())
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

    /// ADR-099 決定6: `[general]` セクション自体を完全に欠く config.toml が
    /// parse error にならず `GeneralConfig::default()` で埋まることを固定する。
    /// `test_parse_app_config_defaults` は `[general]` セクションはあるが
    /// 中身が空のケースであり、セクション自体が無いケースは別（`AppConfig::general`
    /// に `#[serde(default)]` が無いと後者だけ parse error になる）。
    #[test]
    fn test_parse_app_config_missing_general_section_uses_defaults() {
        let toml_str = "";
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.simultaneous_threshold_ms, 100);
        assert_eq!(config.general.left_thumb_key, "無変換");
        assert_eq!(config.general.right_thumb_key, "変換");
        assert_eq!(config.general.default_layout, "nicola.yab");
    }

    // ── classify_load_error (ADR-099 決定4a) ──

    #[test]
    fn classify_load_error_maps_not_found_io_error_to_not_found() {
        let path = std::env::temp_dir().join(format!(
            "awase_test_classify_missing_{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let err = AppConfig::load(&path).unwrap_err();
        assert_eq!(classify_load_error(&err), ConfigLoadState::NotFound);
    }

    #[test]
    fn classify_load_error_maps_parse_error_to_dangerous() {
        let path = std::env::temp_dir().join(format!(
            "awase_test_classify_broken_{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "this is not valid toml [[[").unwrap();

        let err = AppConfig::load(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(classify_load_error(&err), ConfigLoadState::Dangerous(_)),
            "parse errors must never be classified as NotFound (round2 指摘 MF-2)"
        );
    }

    /// `PermissionDenied` のような NotFound 以外の I/O エラーも `Dangerous`
    /// に倒れること（`ErrorKind::NotFound` 以外を安全側に丸めてはならない）。
    #[cfg(unix)]
    #[test]
    fn classify_load_error_maps_permission_denied_to_dangerous() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "awase_test_classify_denied_{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[general]\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let err = AppConfig::load(&path).unwrap_err();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(classify_load_error(&err), ConfigLoadState::Dangerous(_)),
            "PermissionDenied must not be misclassified as NotFound"
        );
    }

    /// `keys.ime_detect.{toggle,on,off}` の既定はすべて空（ADR-207、2026-09-29 所有者決定）。
    /// 旧既定の `IMEオン`/`IMEオフ`（VK_IME_ON/OFF）の追随は hook の静的 `shadow_action` が担う。
    /// 明示した値は既定と無関係に尊重される（一部の項目だけ書いた場合、残りは空）。
    #[test]
    fn test_ime_detect_defaults_are_empty_and_explicit_values_are_respected() {
        let d = ImeDetectConfig::default();
        assert!(d.toggle.is_empty() && d.on.is_empty() && d.off.is_empty());
        let config: AppConfig = toml::from_str("[general]\n").unwrap();
        assert!(config.keys.ime_detect.on.is_empty() && config.keys.ime_detect.off.is_empty());

        let config: AppConfig =
            toml::from_str("[general]\n[keys.ime_detect]\non = [\"IMEオン\", \"VK_F16\"]\n")
                .unwrap();
        assert_eq!(config.keys.ime_detect.on, vec!["IMEオン", "VK_F16"]);
        assert!(config.keys.ime_detect.off.is_empty());
    }

    /// ADR-207: 撤去した `keys.engine_on_ime_key`/`engine_off_ime_key` が旧 config.toml に残っていても
    /// 読み込みエラーにならず、他の設定が読める。値は無視され、専用の通知（`removed_notices`）だけが積まれる
    /// （未知キー警告〈`load_warnings`、ログだけ〉にも、近い名前の提案〈`keys.engine_on`〉にもならない）。
    #[test]
    fn test_removed_engine_ime_keys_are_ignored_with_a_notice() {
        let c = AppConfig::from_toml_str(
            "[general]\nleft_thumb_key = \"無変換\"\n[keys]\n\
             engine_on_ime_key = \"VK_DBE_DBCSCHAR\"\nengine_off_ime_key = \"VK_DBE_SBCSCHAR\"\n\
             engine_on = [\"Ctrl+A\"]\n",
        )
        .expect("撤去したキーが残っていても読める");
        assert_eq!(c.general.left_thumb_key, "無変換");
        assert_eq!(c.keys.engine_on, vec!["Ctrl+A"]);
        assert!(c.load_warnings().is_empty(), "{:?}", c.load_warnings());
        assert_eq!(c.removed_notices().len(), 2, "{:?}", c.removed_notices());
        assert!(c
            .removed_notices()
            .iter()
            .all(|m| m.contains("撤去") && !m.contains("間違い")));
        let (_v, warnings) = c.validate();
        assert_eq!(
            warnings.iter().filter(|w| w.contains("撤去")).count(),
            2,
            "{warnings:?}"
        );
        // 撤去キーが無ければ通知は出ない。
        let c = AppConfig::from_toml_str("[general]\n").unwrap();
        assert!(c.removed_notices().is_empty());
    }

    /// `keys.ime_toggle` の既定値は空（ADR-199 決定15、2026-09-29 所有者決定）。
    /// 「IME の設定に従う」原則のため、awase 自身は漢字キー（`VK_KANJI`）を能動的に消費しない。
    /// 0x19（Alt+半角/全角）の開閉は `hook.rs` の静的 `Toggle`／GJI の役割判定（ADR-202）が担う。
    /// `ime_on`/`ime_off`（awase 自身が actuate する設定）の既定は変えない。
    #[test]
    fn test_keys_config_default_ime_toggle_is_empty() {
        let keys = KeysConfig::default();
        assert!(keys.ime_toggle.is_empty());
        assert_eq!(keys.ime_on, vec!["Ctrl+変換".to_string()]);
        assert_eq!(keys.ime_off, vec!["Ctrl+無変換".to_string()]);
    }

    /// 既存ユーザーの config.toml に残る明示の `ime_toggle = ["VK_KANJI"]`
    /// （旧既定値を GUI の `AppConfig::save` が書き出したもの）は、読込時に消さず尊重する
    /// （既定値の変更は明示値に影響しない）。
    #[test]
    fn test_explicit_ime_toggle_vk_kanji_is_preserved_on_load() {
        let config: AppConfig = toml::from_str("[keys]\nime_toggle = [\"VK_KANJI\"]\n").unwrap();
        assert_eq!(config.keys.ime_toggle, vec!["VK_KANJI".to_string()]);
        // [keys] を書いても ime_toggle を省略すれば既定（空）。
        let config: AppConfig = toml::from_str("[keys]\nime_on = [\"Ctrl+変換\"]\n").unwrap();
        assert!(config.keys.ime_toggle.is_empty());
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
engine_off_solo_repeat = "VK_F15"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("nicola_us.yab")));
    }

    #[test]
    fn test_validate_us_keyboard_with_nicola_kb232_default_layout_warns() {
        // /code-review指摘（PR #132）: nicola_kb232.yab追加時にJIS専用一覧への
        // 追記が漏れていた。nicola_f.yabと同様、keyboard_model="us"では
        // 列数超過でパースに失敗するため警告対象。
        let toml_str = r#"
[general]
keyboard_model = "us"
default_layout = "nicola_kb232.yab"
left_thumb_key = "VK_F16"
right_thumb_key = "VK_F17"

[keys]
engine_on = ["Ctrl+Shift+VK_F13"]
engine_off = ["Ctrl+Shift+VK_F14"]
ime_on = ["Ctrl+VK_F13"]
ime_off = ["Ctrl+VK_F14"]
engine_off_solo_repeat = "VK_F15"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| w.contains("nicola_us.yab")));
    }

    #[test]
    fn test_validate_us_keyboard_with_nicola_kakutei_default_layout_warns() {
        // /code-review指摘（PR #217）: nicola_kb232.yab追加時に一度発生した
        // 「JIS専用一覧への追記漏れ」（PR #132）と同型の見落としを、
        // nicola_kakutei.yab追加時にも繰り返しかけていた。
        let toml_str = r#"
[general]
keyboard_model = "us"
default_layout = "nicola_kakutei.yab"
left_thumb_key = "VK_F16"
right_thumb_key = "VK_F17"

[keys]
engine_on = ["Ctrl+Shift+VK_F13"]
engine_off = ["Ctrl+Shift+VK_F14"]
ime_on = ["Ctrl+VK_F13"]
ime_off = ["Ctrl+VK_F14"]
engine_off_solo_repeat = "VK_F15"
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
engine_off_solo_repeat = "VK_F15"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn test_engine_off_solo_repeat_accepts_legacy_triple_key_name_via_alias() {
        // 2026-08-25 に engine_off_solo_triple → engine_off_solo_repeat へ改称。
        // 旧キー名で保存済みの既存 config.toml が壊れないことを確認する。
        let toml_str = r#"
[keys]
engine_off_solo_triple = "VK_NONCONVERT"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.keys.engine_off_solo_repeat.as_deref(),
            Some("VK_NONCONVERT")
        );
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
        assert_eq!(config.general.default_layout, "nicola_keytop.yab");
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
    fn test_disable_apps_defaults_to_mstsc() {
        // [app_overrides] を含め設定ファイルに一切キーが無い場合でも、
        // BUG-78 対策として mstsc.exe が既定で無効化リストに入る。
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.app_overrides.disable_apps, vec!["mstsc.exe"]);
    }

    #[test]
    fn test_hook_self_heal_defaults_to_enabled() {
        // [diagnostics] を含め設定ファイルに一切キーが無い場合でも、既定で有効。
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.diagnostics.hook_self_heal);
    }

    #[test]
    fn test_hook_self_heal_can_be_disabled() {
        let toml_str = r#"
[diagnostics]
hook_self_heal = false
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.diagnostics.hook_self_heal);
    }

    #[test]
    fn test_disable_apps_can_be_explicitly_emptied() {
        let toml_str = r#"
[general]

[app_overrides]
disable_apps = []
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.app_overrides.disable_apps.is_empty());
    }

    #[test]
    fn test_disable_apps_custom_list_parse() {
        let toml_str = r#"
[general]

[app_overrides]
disable_apps = ["mstsc.exe", "SomeGame.exe"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.app_overrides.disable_apps,
            vec!["mstsc.exe", "SomeGame.exe"]
        );
    }

    #[test]
    fn test_input_relay_apps_defaults_to_empty() {
        let toml_str = r#"
[general]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.app_overrides.input_relay_apps.is_empty());
    }

    #[test]
    fn test_input_relay_apps_can_be_added_explicitly() {
        let toml_str = r#"
[general]

[app_overrides]
input_relay_apps = ["powertoys.mousewithoutbordershelper.exe"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.app_overrides.input_relay_apps,
            vec!["powertoys.mousewithoutbordershelper.exe"]
        );
    }

    #[test]
    fn test_input_relay_apps_custom_list_parse() {
        let toml_str = r#"
[general]

[app_overrides]
input_relay_apps = ["relay.exe", "SomeRelayHelper.exe"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.app_overrides.input_relay_apps,
            vec!["relay.exe", "SomeRelayHelper.exe"]
        );
    }

    #[test]
    fn test_input_relay_apps_empty_entry_warns() {
        let toml_str = r#"
[general]

[app_overrides]
input_relay_apps = ["relay.exe", ""]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("app_overrides.input_relay_apps")),
            "expected a warning about input_relay_apps, got: {warnings:?}"
        );
    }

    #[test]
    fn test_disable_apps_empty_entry_warns() {
        let toml_str = r#"
[general]

[app_overrides]
disable_apps = ["mstsc.exe", ""]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("app_overrides.disable_apps")),
            "expected a warning about disable_apps, got: {warnings:?}"
        );
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
    fn test_legacy_confirm_mode_values_load_as_wait_with_warning() {
        for old in ["speculative", "two_phase", "adaptive_timing"] {
            let c = AppConfig::from_toml_str(&format!(
                "[general]\nconfirm_mode = \"{old}\"\nspeculative_delay_ms = 30\n"
            ))
            .unwrap();
            assert_eq!(c.general.confirm_mode, ConfirmMode::Wait, "{old}");
            let (validated, warnings) = c.validate();
            assert_eq!(validated.general.confirm_mode, ConfirmMode::Wait, "{old}");
            assert!(
                warnings
                    .iter()
                    .any(|w| w.contains(old) && w.contains("廃止")),
                "{old}: {warnings:?}"
            );
        }
    }

    #[test]
    fn test_current_confirm_mode_values_load_without_warning() {
        for v in ["wait", "ngram_predictive"] {
            let c =
                AppConfig::from_toml_str(&format!("[general]\nconfirm_mode = \"{v}\"\n")).unwrap();
            let (_, warnings) = c.validate();
            assert!(warnings.is_empty(), "{v}: {warnings:?}");
        }
    }

    #[test]
    fn test_validate_confirm_mode_wait_is_untouched() {
        let toml_str = r#"
[general]
confirm_mode = "wait"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (validated, warnings) = config.validate();
        assert_eq!(validated.general.confirm_mode, ConfirmMode::Wait);
        assert!(warnings.is_empty());
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

    /// ADR-201 決定1: `F18` のような `VK_` 無し・小文字の表記も、`from_name` と同じ規則で
    /// 安全範囲として扱う（以前は `"VK_F18"` の完全一致のみで、`F18` は警告された）。
    #[test]
    fn test_validate_dedicated_fn_key_is_lenient_about_notation() {
        for name in ["F18", "f18", "vk_f18", " VK_F18 ", "F15", "F24"] {
            let mut general = GeneralConfig::default();
            general.muhenkan_solo_tap_dedicated_fn_key = Some(name.to_string());
            let mut warnings = Vec::new();
            AppConfig::validate_dedicated_fn_key(&general, &mut warnings);
            assert!(warnings.is_empty(), "{name:?}: {warnings:?}");
        }
        for name in ["F14", "f13", "Ctrl+F18", "VK_F25", "無変換"] {
            let mut general = GeneralConfig::default();
            general.muhenkan_solo_tap_dedicated_fn_key = Some(name.to_string());
            let mut warnings = Vec::new();
            AppConfig::validate_dedicated_fn_key(&general, &mut warnings);
            assert_eq!(warnings.len(), 1, "{name:?}");
        }
    }

    /// ADR-201 決定1: かなキーは `カナ`/`かな`/小文字でも同じ VK（0x15）なので警告する。
    #[test]
    fn test_validate_thumb_keys_warns_on_kana_spellings() {
        for name in ["カナ", "かな", "kana", "vk_kana", " Kana "] {
            let mut general = GeneralConfig::default();
            general.left_thumb_key = name.to_string();
            let mut warnings = Vec::new();
            AppConfig::validate_thumb_keys(&general, &mut warnings);
            assert_eq!(warnings.len(), 1, "{name:?}");
        }
    }

    /// ADR-201 R3-3: 組み合わせは主キーの完全一致で見る（`contains` ではない）。
    /// 修飾付きの `Ctrl+変換` も US 配列では JIS 専用キーとして検出し、無関係な名前の
    /// 一部に「変換」が含まれるだけでは誤検出しない。
    #[test]
    fn test_validate_keyboard_model_us_matches_combo_main_key_exactly() {
        let check = |engine_on: &str| {
            let mut general = GeneralConfig::default();
            general.keyboard_model = KeyboardModel::Us;
            general.left_thumb_key = "VK_SPACE".to_string();
            general.right_thumb_key = "VK_SPACE".to_string();
            let mut keys = KeysConfig::default();
            keys.engine_on = vec![engine_on.to_string()];
            keys.engine_off = vec![];
            keys.ime_on = vec![];
            keys.ime_off = vec![];
            keys.engine_off_solo_repeat = None;
            let mut w = Vec::new();
            AppConfig::validate_keyboard_model(&general, &keys, &mut w);
            w.iter().any(|m| m.contains("keys.engine_on"))
        };
        assert!(check("Ctrl+変換"));
        assert!(check("ctrl+vk_nonconvert"));
        assert!(check("Nonconvert"));
        assert!(!check("Ctrl+VK_F12"));
        // 名前の一部に「変換」を含むだけの別の名前は誤検出しない。
        assert!(!check("Ctrl+再変換"));
    }

    /// T-16: IME コンボに bare 親指キーを設定した場合だけ警告する。
    /// Ctrl+無変換のような修飾付きコンボは従来どおり許容する。
    #[test]
    fn test_validate_warns_for_bare_thumb_key_in_ime_combo_only() {
        let toml_str = r#"
[general]
left_thumb_key = "無変換"

[keys]
ime_on = ["VK_NONCONVERT"]
ime_off = ["Ctrl+VK_NONCONVERT"]
ime_toggle = []
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("keys.ime_on")
                    && w.contains("単独タップ確定時に強制ON/OFFが発火")),
            "bare thumb key in keys.ime_on should warn, got: {warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("keys.ime_off")),
            "Ctrl+無変換 must not warn, got: {warnings:?}"
        );
    }

    #[test]
    fn test_legacy_solo_tap_action_migrates_only_for_thumb_keys_and_warns() {
        let toml_str = r#"
[general]
left_thumb_key = "無変換"
right_thumb_key = "Space"
muhenkan_solo_tap_ime_action = "off"
henkan_solo_tap_ime_action = "on"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        // 無変換だけが親指キー。変換は親指に割り当てられていないので移行しない。
        assert_eq!(
            config.general.legacy_thumb_solo_tap_actions(),
            (Some(crate::types::ShadowImeAction::TurnOff), None)
        );
        let (_validated, warnings) = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("general.muhenkan_solo_tap_ime_action")
                    && w.contains("非推奨です")
                    && w.contains("同じ扱いで動いています")),
            "親指キーの旧設定は移行の警告、got: {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("general.henkan_solo_tap_ime_action")
                    && w.contains("今後は効きません")),
            "親指でないキーの旧設定は読み捨ての警告、got: {warnings:?}"
        );
    }

    #[test]
    fn test_legacy_solo_tap_action_is_ignored_when_bare_combo_exists() {
        let toml_str = r#"
[general]
left_thumb_key = "無変換"
muhenkan_solo_tap_ime_action = "toggle"

[keys]
ime_off = ["無変換"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        let (_validated, warnings) = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("general.muhenkan_solo_tap_ime_action")
                    && w.contains("無視されます")
                    && w.contains("優先されます")),
            "bare が既にあれば旧設定は無視される旨を警告する、got: {warnings:?}"
        );
    }

    #[test]
    fn test_no_legacy_warning_without_solo_tap_action() {
        let (_validated, warnings) = AppConfig::default().validate();
        assert!(
            !warnings.iter().any(|w| w.contains("_solo_tap_ime_action")),
            "旧設定が無ければ警告しない、got: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_keeps_legacy_warning_for_non_convert_thumb_key() {
        let mut config = AppConfig::default();
        config.general.left_thumb_key = "VK_SPACE".to_string();
        config.keys.ime_off = vec!["VK_SPACE".to_string()];
        let (_validated, warnings) = config.validate();
        assert!(warnings.iter().any(|w| {
            w.contains("keys.ime_off")
                && w.contains("他のキーに変更するか")
                && w.contains("Shift などと組み合わせて")
        }));
    }

    // parse_key_combo テストは awase-windows に移動済み

    // ── engine_on/off_keys デフォルトテスト ──

    /// ADR-191で`apply_calibrated_mode_keys`設定を撤去した。既存の`config.toml`に古いキーが残っていても、
    /// 起動時に読み込みエラーにならず、無視されて他の設定が読める（`deny_unknown_fields`を付けていない）。
    #[test]
    fn test_removed_apply_calibrated_mode_keys_key_is_ignored_on_load() {
        let toml_str = r#"
[general]
apply_calibrated_mode_keys = true
left_thumb_key = "無変換"
"#;
        let config: AppConfig = toml::from_str(toml_str).expect("旧キーが残っていても読める");
        assert_eq!(config.general.left_thumb_key, "無変換");
    }

    /// 手動較正(ADR-176)の撤去で`AppConfig::calibration`（`[[calibration]]`）を削除した。
    /// 旧版が書いた`[[calibration]]`が`config.toml`に残っていても、読み込みエラーにならず、
    /// 無視されて他の設定が読める（`deny_unknown_fields`を付けていない）。
    #[test]
    fn test_removed_calibration_section_is_ignored_on_load() {
        let toml_str = r#"
[general]
left_thumb_key = "無変換"

[[calibration]]
vk = 29
result = "On"
active_ime_kind = "Gji"
fingerprint_kind = "Gji"
gji_session_keymap = 3
gji_relevant_row = "DirectInput\tMuhenkan\tIMEOn"
confirmed_at_epoch_ms = 1758000000000
"#;
        let config: AppConfig =
            toml::from_str(toml_str).expect("旧[[calibration]]が残っていても読める");
        assert_eq!(config.general.left_thumb_key, "無変換");
    }

    /// ADR-191で`dbe_mode_key_policy`（BUG-52のDBEキー Suppress を外す隠し設定）と
    /// `gji_thumb_key_ime_toggle`を撤去した（レビュー指摘B-M3）。旧`config.toml`にキーが残っていても、
    /// 読み込みエラーにも警告にもならず、無視されて他の設定が読める。
    #[test]
    fn test_removed_dbe_mode_key_policy_and_gji_thumb_key_ime_toggle_are_ignored_on_load() {
        let toml_str = r#"
[general]
dbe_mode_key_policy = "passthrough"
gji_thumb_key_ime_toggle = true
left_thumb_key = "無変換"
"#;
        let config: AppConfig = toml::from_str(toml_str).expect("旧キーが残っていても読める");
        assert_eq!(config.general.left_thumb_key, "無変換");
    }

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

    // ── AppConfig::save (ADR-099 決定3): アトミック書き込み（tmp+rename）──
    // 詳細な機構（rename経由の置換・リトライ・パーミッション引き継ぎ・
    // シンボリックリンク追従等）は実体である `crate::fs_atomic::write_atomic`
    // 側のテストで検証する（`src/fs_atomic.rs`）。ここでは `save()` が
    // TOML へシリアライズしてから委譲することのみ確認する。

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

    // ── ADR-115: 打鍵列機能 ──

    #[test]
    fn test_keystroke_sequence_defaults_to_on() {
        // 2026-09-13 に既定 Off → On へ変更（ADR-115 決定8追補）。
        let config = AppConfig::default();
        assert_eq!(
            config.general.keystroke_sequence,
            KeystrokeSequencePolicy::On
        );
    }

    #[test]
    fn test_parse_keystroke_macro() {
        let toml_str = r#"
[general]
keystroke_sequence = "on"

[[keystroke_macro]]
name = "bracket_paren"
steps = ["'（'", "CV4D", "'）'", "CV4D", "左"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.general.keystroke_sequence,
            KeystrokeSequencePolicy::On
        );
        assert_eq!(config.keystroke_macro.len(), 1);
        assert_eq!(config.keystroke_macro[0].name, "bracket_paren");
        assert_eq!(
            config.keystroke_macro[0].steps,
            vec!["'（'", "CV4D", "'）'", "CV4D", "左"]
        );
    }

    #[test]
    fn test_keystroke_macro_survives_save_load_round_trip() {
        let mut config = AppConfig::default();
        config.general.keystroke_sequence = KeystrokeSequencePolicy::On;
        config.keystroke_macro.push(KeystrokeMacro {
            name: "bracket_paren".to_string(),
            steps: vec![
                "'（'".to_string(),
                "CV4D".to_string(),
                "'）'".to_string(),
                "CV4D".to_string(),
                "左".to_string(),
            ],
        });

        let serialized = toml::to_string_pretty(&config).unwrap();
        let round_tripped: AppConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(
            round_tripped.general.keystroke_sequence,
            KeystrokeSequencePolicy::On
        );
        assert_eq!(round_tripped.keystroke_macro.len(), 1);
        assert_eq!(round_tripped.keystroke_macro[0].name, "bracket_paren");
        assert_eq!(
            round_tripped.keystroke_macro[0].steps,
            config.keystroke_macro[0].steps
        );
    }

    #[test]
    fn test_validated_config_preserves_keystroke_macro() {
        // ValidatedConfig と AppConfig は別構造体で validate() が手で
        // 詰め替えているため、keystroke_macro が伝播することを回帰で
        // 固定する（実装タスクレビュー指摘 C1）。
        let mut config = AppConfig::default();
        config.keystroke_macro.push(KeystrokeMacro {
            name: "confirm".to_string(),
            steps: vec!["CV4D".to_string()],
        });

        let (validated, _warnings) = config.validate();
        assert_eq!(validated.keystroke_macro.len(), 1);
        assert_eq!(validated.keystroke_macro[0].name, "confirm");

        let round_tripped: AppConfig = validated.into();
        assert_eq!(round_tripped.keystroke_macro.len(), 1);
        assert_eq!(round_tripped.keystroke_macro[0].name, "confirm");
    }

    // ── ensure_config_exists / ensure_layouts_exist（ADR-178 決定2・決定5）──

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "awase_ensure_user_data_test_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_config_exists_creates_file_when_missing() {
        let dir = unique_temp_dir("config_missing");
        let path = dir.join("config.toml");
        assert!(!path.exists());

        ensure_config_exists(&path).unwrap();

        assert!(path.exists());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            EMBEDDED_CONFIG_TOML
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_config_exists_never_touches_existing_file() {
        let dir = unique_temp_dir("config_existing");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[general]\nsimultaneous_threshold_ms = 777\n").unwrap();

        ensure_config_exists(&path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[general]\nsimultaneous_threshold_ms = 777\n",
            "既存ファイルの内容が変わってはならない（ADR-178 決定2: 比較も上書きもしない）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_layouts_exist_creates_all_bundled_files_when_dir_missing() {
        let dir = unique_temp_dir("layouts_missing");
        let layouts_dir = dir.join("layout");
        assert!(!layouts_dir.exists());

        ensure_layouts_exist(&layouts_dir).unwrap();

        for (name, content) in EMBEDDED_LAYOUTS {
            let path = layouts_dir.join(name);
            assert!(path.exists(), "{name} が生成されていない");
            assert_eq!(&std::fs::read_to_string(&path).unwrap(), content);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_layouts_exist_does_nothing_when_one_yab_already_present() {
        let dir = unique_temp_dir("layouts_one_present");
        let layouts_dir = dir.join("layout");
        std::fs::create_dir_all(&layouts_dir).unwrap();
        std::fs::write(layouts_dir.join("custom.yab"), "user data").unwrap();

        ensure_layouts_exist(&layouts_dir).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&layouts_dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "1本でも.yabが存在するなら同梱6ファイルを生成してはならない（ADR-178 決定2）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_layouts_exist_cleans_up_partial_writes_on_failure() {
        // /code-review指摘（v14 opusレビューMajor M5対応）: 3本目の書き込みを
        // write_atomicの内部で使う一時ファイル名（<target>.tmp.<pid>）を狙って
        // 失敗させる。同名のディレクトリを事前に置くと`File::create`が
        // 失敗する。（3本目**そのもの**の名前にディレクトリを置く方式だと
        // has_any_yabの拡張子判定に引っかかり「1本でもある」扱いで
        // ensure_layouts_existが即Ok(())で返ってしまうため使えない。）
        // 途中まで書いた分（1・2本目）が削除され、次回呼び出しで再度0本から
        // 全6本の生成を試みられる状態に戻ることを確認する。
        let dir = unique_temp_dir("layouts_partial_failure");
        let layouts_dir = dir.join("layout");
        std::fs::create_dir_all(&layouts_dir).unwrap();
        let third_name = EMBEDDED_LAYOUTS[2].0;
        let blocked_tmp = layouts_dir.join(format!("{third_name}.tmp.{}", std::process::id()));
        std::fs::create_dir_all(&blocked_tmp).unwrap();

        let result = ensure_layouts_exist(&layouts_dir);
        assert!(
            result.is_err(),
            "3本目の一時ファイル名がディレクトリで塞がれているので失敗するはず"
        );

        for (i, (name, _)) in EMBEDDED_LAYOUTS.iter().enumerate() {
            if i < 2 {
                assert!(
                    !layouts_dir.join(name).exists(),
                    "{name}（{i}本目）は途中失敗時に片付けられているべき（ADR-178 v14 M5）"
                );
            }
        }
        assert!(
            !layouts_dir.join(third_name).exists(),
            "3本目自体はFile::create段階で失敗しているので書き込まれていないはず"
        );

        // 次回呼び出しで「0本」から全6本の再生成を試みられることを確認する
        // （塞いでいた一時ファイル名のディレクトリを除去してから再実行）。
        std::fs::remove_dir_all(&blocked_tmp).unwrap();
        ensure_layouts_exist(&layouts_dir).unwrap();
        for (name, content) in EMBEDDED_LAYOUTS {
            assert_eq!(
                std::fs::read_to_string(layouts_dir.join(name)).unwrap(),
                *content
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── ADR-201 段階2: from_toml_str（未知キー・[[keymap]] の合流）──

    #[test]
    fn from_toml_str_warns_unknown_keys_and_suggests() {
        let c = AppConfig::from_toml_str(
            "[general]\nsimultaneous_threshold_msx = 80\n[keys]\nime_onn = []\n[futuresection]\na = 1\n",
        )
        .unwrap();
        let w = c.load_warnings().join("\n");
        assert!(w.contains("general.simultaneous_threshold_msx"), "{w}");
        assert!(
            w.contains("simultaneous_threshold_ms\""),
            "近い名前を示す: {w}"
        );
        assert!(w.contains("keys.ime_onn") && w.contains("ime_on\""), "{w}");
        assert!(w.contains("futuresection"), "{w}");
        // validate() が警告に加える
        let (_v, warnings) = c.validate();
        assert!(warnings.iter().any(|x| x.contains("futuresection")));
    }

    #[test]
    fn from_toml_str_does_not_warn_for_removed_keys_or_alias() {
        let c = AppConfig::from_toml_str(
            "[general]\napply_calibrated_mode_keys = true\ndbe_mode_key_policy = \"passthrough\"\n\
             output_mode = \"batched\"\n[keys]\nengine_off_solo_triple = \"VK_INSERT\"\n\
             [[calibration]]\nvk = 29\n",
        )
        .unwrap();
        assert!(c.load_warnings().is_empty(), "{:?}", c.load_warnings());
        assert_eq!(c.keys.engine_off_solo_repeat.as_deref(), Some("VK_INSERT"));
    }

    #[test]
    fn from_toml_str_merges_legacy_keymap_into_keymaps() {
        let only_legacy = "[[keymap]]\nfrom = \"Ctrl+VK_I\"\nto = [\"VK_TAB\"]\n";
        let c = AppConfig::from_toml_str(only_legacy).unwrap();
        assert_eq!(c.keymaps.len(), 1);
        assert!(c.load_warnings().iter().any(|w| w.contains("[[keymap]]")));

        // 両方あっても読み込みは失敗せず、連結して警告する（alias にしたときの Dangerous を避ける）
        let both = "[[keymap]]\nfrom = \"Ctrl+VK_I\"\nto = [\"VK_TAB\"]\n\
                    [[keymaps]]\nfrom = \"Ctrl+VK_J\"\nto = [\"VK_TAB\"]\n";
        let c = AppConfig::from_toml_str(both).unwrap();
        assert_eq!(c.keymaps.len(), 2);
        assert!(c.load_warnings().iter().any(|w| w.contains("連結")));
    }

    /// `[[keymap]]` だけのファイル → 読み込み → 保存 → 再読み込みで規則の数が変わらない
    /// （合流後は `keymaps` に一本化されて書かれ、`keymap` は書かれない）。
    #[test]
    fn legacy_keymap_survives_save_and_reload_without_doubling() {
        let text = "[[keymap]]\nfrom = \"Ctrl+VK_I\"\nto = [\"VK_TAB\"]\n";
        let c = AppConfig::from_toml_str(text).unwrap();
        let dir = std::env::temp_dir().join(format!("awase-adr201-s2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        c.save(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("[[keymap]]"), "{saved}");
        let r = AppConfig::load(&path).unwrap();
        assert_eq!(r.keymaps.len(), 1);
        r.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap().keymaps.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 既定値から書き出した設定は、未知キーの警告を出さない（`Option` の `None` などで誤報しない）。
    #[test]
    fn from_toml_str_default_roundtrip_has_no_warnings() {
        let text = toml::to_string_pretty(&AppConfig::default()).unwrap();
        let c = AppConfig::from_toml_str(&text).unwrap();
        assert!(c.load_warnings().is_empty(), "{:?}", c.load_warnings());
        let bundled = AppConfig::from_toml_str(include_str!("../config.toml")).unwrap();
        assert!(
            bundled.load_warnings().is_empty(),
            "{:?}",
            bundled.load_warnings()
        );
    }
}
