//! GJI検出時、`config1.db`の`custom_keymap_table`からIME ON/OFF/トグルキー
//! （ADR-092 決定D Step4c）を自動判定する。
//!
//! 専用Fnキー変換（ADR-091 §D3.2）の自動判定・設定支援ポップアップ・
//! config1.db書き込みは、実験的機能のまま撤去し忘れて出荷され、実機で
//! ユーザーの混乱を招いた（GJIのキー設定が実際にはカスタムなのに
//! 「カスタム以外」と誤診断されるなど）ため2026-09-02に全撤去した
//! （`gji_charset_popup.rs`/`gji_charset_write.rs`ごと削除）。
//! `GeneralConfig::muhenkan_solo_tap_dedicated_fn_key`による手動設定
//! （config.toml）経由の内部配線（`nicola_fsm.rs`の専用Fnキー送出）は
//! そのまま残っている。
//!
//! # 設計方針
//!
//! - **新しいbeliefは持たない**（ADR-091の中心方針）。ここでの判定は
//!   `config1.db`という外部ファイルの現在の中身を毎回そのまま読むだけで、
//!   awase側で過去の観測を蓄積・推測することはしない。
//! - **継続的なポーリングはしない**（ADR-091決定3項目2）。GJIが継続して
//!   アクティブな間は一度判定したら再読み込みしない（[`sync_gji_charset_autodetect`]
//!   のラッチ参照）。
//! - **config1.db未存在（GJI未インストール等）はエラーではない**。読めなければ
//!   静かに何もしない。`awase-gji-config`crate自体の「パース失敗は常に
//!   空の結果に静かにフォールバック」という既存方針を踏襲する。

use awase::config::ParsedKeyCombo;
use awase::types::VkCode;

use crate::vk::VkCodeExt as _;

/// IME ON/OFF/トグルキーとして自動採用してよいVK名の範囲。
///
/// `src/config.rs::validate_dedicated_fn_key`と同じ基準（`VK_F15`-`VK_F24`の
/// うち`VK_F13`/`VK_F14`を除く）。`VK_F13`/`VK_F14`はターミナルエスケープ
/// シーケンス漏れが実機確認済み（ADR-057）のため、config1.dbにこれらへの
/// バインドが見つかっても絶対に採用しない。
#[cfg_attr(not(windows), allow(dead_code))]
fn is_in_safe_autodetect_range(vk_name: &str) -> bool {
    matches!(
        vk_name,
        "VK_F15"
            | "VK_F16"
            | "VK_F17"
            | "VK_F18"
            | "VK_F19"
            | "VK_F20"
            | "VK_F21"
            | "VK_F22"
            | "VK_F23"
            | "VK_F24"
    )
}

/// `config1.db`の`custom_keymap_table`から、IME ON/OFF/トグルの自動検出用
/// `ParsedKeyCombo`リストを判定する（ADR-092 決定D Step4c）。戻り値は
/// `(on, off, toggle)`で、それぞれ`Engine::set_ime_on_auto_keys`等へ渡す。
///
/// `awase_gji_config::keymap::extract_ime_keys`が返すVK名は`VK_KANJI`
/// （Hankaku/Zenkakuも同一）・`VK_IME_ON`・`VK_IME_OFF`・`VK_DBE_ALPHANUMERIC`
/// （Eisu）を含みうる。これらはBUG-14で確認済みの「MS-IME/CTFが注入する
/// 合成イベントと衝突しうるキー」であり、`SpecialKeyCombos::match_event`が
/// `event.injected`を見ずにマッチするため、そのまま採用するとBUG-14と同種の
/// 誤トグルを招く。[`is_in_safe_autodetect_range`]（専用FnキーF15-F24のみ）
/// で必ず絞り込み、上記4エイリアスを一律除外する。
///
/// GJIが無変換/変換にIME ON/OFF/トグルを割り当てているケースは、本関数
/// （F15-F24限定の安全範囲フィルタ）の対象外——`Henkan`/`Muhenkan`は
/// `mozc_key_to_vk_name`で`VK_CONVERT`/`VK_NONCONVERT`に変換されうるが
/// （BUG-115で追加）、F15-F24範囲外なので[`is_in_safe_autodetect_range`]で
/// 必ず除外される。この2キーは
/// [`classify_thumb_key_ime_actions`]・[`gate_thumb_key_ime_actions`]が
/// 別途分類・opt-inゲートし、`windows_impl::route_thumb_key_action`が
/// `is_thumb_key`に応じてStep4b delegate-to-open-axisまたは
/// `ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`へ振り分ける。詳細は
/// [docs/known-bugs.md BUG-115](../../../../docs/known-bugs.md) 参照。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
fn extract_ime_on_off_toggle_combos(
    custom_keymap_table: &str,
) -> (
    Vec<ParsedKeyCombo>,
    Vec<ParsedKeyCombo>,
    Vec<ParsedKeyCombo>,
) {
    let keys = awase_gji_config::keymap::extract_ime_keys(custom_keymap_table);
    let to_combos = |names: &[String]| -> Vec<ParsedKeyCombo> {
        names
            .iter()
            .filter(|name| is_in_safe_autodetect_range(name))
            .filter_map(|name| VkCode::from_name(name))
            .map(|vk| ParsedKeyCombo {
                ctrl: false,
                shift: false,
                alt: false,
                vk,
            })
            .collect()
    };
    (
        to_combos(&keys.on),
        to_combos(&keys.off),
        to_combos(&keys.toggle),
    )
}

/// GJIが無変換/変換キーに割り当てているIME意味論の分類（BUG-115）。
/// `session_keymap`/`custom_keymap_table`/`overlay_keymaps`のどれ由来でも
/// 同じ3値に潰す——awase側の反応（delegate-to-open-axisか
/// `ime_on_auto`/`off_auto`/`toggle_auto`か）は`On`/`Off`なら常に安全
/// （冪等）、`Toggle`のときだけopt-inゲートの対象になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImeToggleKind {
    /// このキー単独でIMEをONにする。
    On,
    /// このキー単独でIMEをOFFにする。
    Off,
    /// 現在のIME開閉状態に応じて反転する（`ctx.ime_on`依存、非冪等）。
    Toggle,
}

/// [`classify_thumb_key_ime_actions`]の判定結果に付随する、ユーザーへの
/// 通知要否（BUG-115）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(windows, repr(u8))]
pub(crate) enum ThumbKeyImeWarning {
    /// 通知不要（矛盾なし、または`On`/`Off`のみで冪等、あるいは
    /// overlayにより既に解決済み）。
    #[default]
    None,
    /// 無変換/変換に状態依存トグル（`Toggle`）を検出したが、
    /// `gji_thumb_key_ime_toggle`が`false`（既定）のため反映しなかった。
    /// 対処法を案内する`log::warn!`が必要。
    ToggleDeclined,
    /// 状態依存トグルを、ユーザーのopt-in設定によりベストエフォートで
    /// 反映した。`log::info!`で通知する。
    ToggleHonored,
}

/// GJIの現在の設定（`config1.db`）が、無変換/変換キー単体にどのIME意味論
/// （[`ImeToggleKind`]）を割り当てているかを判定する（BUG-115）。
/// `config1.db`の3つの独立した情報源を、優先順位付きで1つの結論に
/// まとめる純粋関数。Linux上でもテスト可能（Windows APIに依存しない）。
///
/// 優先順位（Mozcがoverlayをbase keymapの上に重ね掛けする実装、
/// `session.cc`/`keymap.cc::ApplyOverlaySessionKeymap`と対応させてある）:
///
/// 1. **`overlay_keymaps`に`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`(100)が
///    含まれる**: `session_keymap`の値に関わらず最優先（ATOKや後述の
///    CUSTOMトークンと同時に該当していても、overlayが勝つ）。
///    Henkan→`On`・Muhenkan→`Off`（`overlay_henkan_muhenkan_to_ime_on_off.tsv`
///    で2026-09-05確認済み: 状態非依存で一貫しているため`Toggle`にはならず
///    警告不要）。
/// 2. **`session_keymap == CUSTOM`（overlay無し）**: `custom_keymap_table`
///    （field 42）に、ユーザーが（例えばATOKベースからカスタムを作った
///    場合など）literal に`Henkan`/`Muhenkan`トークンを含めていることが
///    ある（BUG-115で判明。`awase-gji-config::keymap::extract_ime_keys`が
///    これらのトークンを認識し、`STATUSES_WHEN_IME_OFF`/
///    `STATUSES_WHEN_IME_ON`に基づき`On`/`Off`/`Toggle`へ分類する——ATOK
///    プリセット由来の行をそのままコピーした場合は3.と同じ`Toggle`に
///    classifyされる）。Henkan/Muhenkanそれぞれ独立に判定する
///    （一方だけ設定されている場合もある）。
/// 3. **`session_keymap == ATOK`（overlay無し、custom無し）**:
///    `google/mozc`の`src/data/keymap/atok.tsv`（2026-09-05取得）は、
///    Henkan/Muhenkan双方を`DirectInput`状態で`IMEOn`、`Precomposition`
///    状態で`CancelAndIMEOff`に割り当てている——`ctx.ime_on`の値に応じて
///    反転する割当てだが、`ShadowImeAction::Toggle`
///    （`Engine::apply_ime_open_request`の`Toggle => !ctx.ime_on`）で
///    **正確に表現できる**（「表現不能」ではない）。
/// 4. **それ以外**（`MSIME`/`MOBILE`/`KOTOERI`/`CHROMEOS`/フィールド不在/
///    未知の値、または`CUSTOM`だがHenkan/Muhenkanトークンが無い）:
///    割り当てなし。`ms-ime.tsv`/`mobile.tsv`はHenkanが`Reconvert`
///    （IME開閉と無関係）でMuhenkanは該当行自体が無く、`kotoeri.tsv`/
///    `chromeos.tsv`はHenkan/Muhenkan関連行が無い（いずれも2026-09-05
///    取得して確認済み）。フィールド不在/`NONE`もここに落ちるが、
///    Windows版GJIでは`ConfigHandler::GetDefaultKeyMap()`
///    （`config_handler.cc`で確認済み）により実質MSIME相当なので、この
///    fail-closedな既定は実際のGJI挙動とも一致する。
///
/// **`Toggle`は「表現不能」ではなく「opt-inで提供する」設計判断**である
/// ことに注意。既定でopt-inしない理由は
/// `GeneralConfig::gji_thumb_key_ime_toggle`のdocコメント、および
/// [docs/known-bugs.md BUG-115](../../../../docs/known-bugs.md)参照
/// （要旨: `Toggle`の非冪等性・親指キー2本への露出倍増・非opt-in・GJIが
/// Mozcのフォークである不確実性の4点）。将来この関数を見て「Toggleで
/// 書けるから既定ONにできるのでは」と再検討する場合は、必ず上記
/// ドキュメントの「なぜ既定OFFにしたか」を先に読むこと。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn classify_thumb_key_ime_actions(
    raw: &awase_gji_config::wire::GjiRawConfig,
) -> (Option<ImeToggleKind>, Option<ImeToggleKind>) {
    (
        classify_mode_key_ime_action(ModeKeyCandidate::Henkan, raw),
        classify_mode_key_ime_action(ModeKeyCandidate::Muhenkan, raw),
    )
}

/// GJIがIME on/off意味論を割り当てうる候補キー（BUG-115）。
///
/// **注意（2026-09-05訂正、[ADR-135](../../../../docs/adr/135-generic-thumb-key-ime-toggle-delegate.md)
/// 「Phase 2の撤回」参照）**: 以前このdocコメントには「Hiragana/Katakana
/// にはdelegate-to-open-axis相当の安全な自動反映手段が存在しない」と
/// 書かれていたが、これは誤りだった。実際には
/// `crate::vk::ImeKeyKind::from_vk`→`hook.rs`の`shadow_action`→
/// `runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle`という別の
/// 既存機構が既にHiragana/Katakana/Eisu/Kanji等の追従を担当している。
/// `Henkan`/`Muhenkan`（`VK_CONVERT`/`VK_NONCONVERT`）だけが
/// `ImeKeyKind::from_vk`に含まれず、それゆえStep4b
/// delegate-to-open-axis（`henkan_delegate_to_open_axis`/
/// `muhenkan_delegate_to_open_axis`、`src/engine/nicola_fsm.rs`の専用
/// フィールド2つ）が必要だった。`Hiragana`/`Katakana`バリアントと
/// [`classify_mode_key_ime_action`]自体はここに残してあるが、
/// `windows_impl::sync_gji_charset_autodetect`からの実際の配線
/// （actuation-autoへ載せる処理）はPhase 1ホリスティックレビューで
/// 既存のshadow-toggle機構との二重actuationが発覚し撤去済み
/// （2026-09-05）。GJI検出値をshadow_actionへ正しく反映する設計は
/// ADR-135「Phase 2（再設計）」で扱う——実装時にこの分類関数と
/// テストを再利用する想定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ModeKeyCandidate {
    Henkan,
    Muhenkan,
    // Phase 2（ADR-135「Phase 2（再設計）」）実装まで本番コードから未使用。
    // classify_mode_key_ime_action と回帰テストのために維持している。
    #[allow(dead_code)]
    Hiragana,
    #[allow(dead_code)]
    Katakana,
}

impl ModeKeyCandidate {
    /// `awase::types::VkCode::from_name`が受理するVK名。
    const fn vk_name(self) -> &'static str {
        match self {
            Self::Henkan => "VK_CONVERT",
            Self::Muhenkan => "VK_NONCONVERT",
            Self::Hiragana => "VK_DBE_HIRAGANA",
            Self::Katakana => "VK_DBE_KATAKANA",
        }
    }

    /// [`Self::vk_name`]が指すVK値そのもの。全バリアントの文字列は
    /// `VkCode::from_name`が受理する静的に既知の値のみなので`unreachable!`
    /// に到達しない（`tests::mode_key_candidate_vk_resolves_for_all_variants`
    /// が全バリアントを網羅して固定）。
    #[cfg_attr(not(windows), allow(dead_code))]
    fn vk(self) -> VkCode {
        VkCode::from_name(self.vk_name())
            .unwrap_or_else(|| unreachable!("ModeKeyCandidate::vk_name always resolves"))
    }
}

/// [`ModeKeyCandidate`]の現在のGJI設定によるIME意味論を判定する（BUG-115）。
/// `config1.db`の3つの独立した情報源を、優先順位付きで1つの結論に
/// まとめる純粋関数。Linux上でもテスト可能（Windows APIに依存しない）。
///
/// 優先順位（Mozcがoverlayをbase keymapの上に重ね掛けする実装、
/// `session.cc`/`keymap.cc::ApplyOverlaySessionKeymap`と対応させてある）:
///
/// 1. **`overlay_keymaps`に`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`(100)が
///    含まれる**: `session_keymap`の値に関わらず最優先。Henkan→`On`・
///    Muhenkan→`Off`（`overlay_henkan_muhenkan_to_ime_on_off.tsv`で
///    2026-09-05確認済み: 状態非依存で一貫しているため`Toggle`にはならず
///    警告不要）。Hiragana/Katakanaはこのoverlayの対象外——このソースでは
///    次のソースへフォールスルーする。
/// 2. **`session_keymap == CUSTOM`（overlayが対象外、または無し）**:
///    `custom_keymap_table`（field 42）に、ユーザーが（例えばATOK/MSIME
///    ベースからカスタムを作った場合など）literal に該当キーのトークンを
///    含めていることがある（BUG-115で判明。
///    `awase-gji-config::keymap::extract_ime_keys`がこれらのトークンを
///    認識し、`STATUSES_WHEN_IME_OFF`/`STATUSES_WHEN_IME_ON`に基づき
///    `On`/`Off`/`Toggle`へ分類する）。
/// 3. **`session_keymap`がプリセット（overlay/custom無し）**:
///    `google/mozc`の各プリセットtsv（2026-09-05取得）の静的知識。
///    - `ATOK`: Henkan/Muhenkan双方を`DirectInput`状態で`IMEOn`、
///      `Precomposition`状態で`CancelAndIMEOff`に割り当てている——
///      `ctx.ime_on`の値に応じて反転する割当てだが、`ShadowImeAction::Toggle`
///      （`Engine::apply_ime_open_request`の`Toggle => !ctx.ime_on`）で
///      **正確に表現できる**（「表現不能」ではない）。Hiragana/Katakanaへの
///      割当ては無い。
///    - `MSIME`/`MOBILE`: Hiragana/Katakana双方を`DirectInput`状態で
///      `IMEOn`に割り当てている（`Precomposition`状態の
///      `CompositionModeHiragana`/`CompositionModeFullKatakana`はIME
///      開閉と無関係の絶対モード設定なので矛盾しない、単純に`On`）。
///      Henkan/Muhenkanへの割当ては`Reconvert`のみでIME開閉と無関係。
///    - `KOTOERI`/`CHROMEOS`: 該当行なし。
/// 4. **それ以外**（フィールド不在/未知の値、または`CUSTOM`だが該当
///    トークンが無い）: 割り当てなし。フィールド不在/`NONE`は、Windows版
///    GJIでは`ConfigHandler::GetDefaultKeyMap()`（`config_handler.cc`で
///    確認済み）により実質MSIME相当なので、`MSIME`の分岐へ委ねる
///    fail-closedな既定が実際のGJI挙動とも一致する。
///
/// **`Toggle`は「表現不能」ではなく「opt-inで提供する」設計判断**である
/// ことに注意。既定でopt-inしない理由は
/// `GeneralConfig::gji_thumb_key_ime_toggle`のdocコメント、および
/// [docs/known-bugs.md BUG-115](../../../../docs/known-bugs.md)参照
/// （要旨: `Toggle`の非冪等性・親指キー2本への露出倍増・非opt-in・GJIが
/// Mozcのフォークである不確実性の4点）。将来この関数を見て「Toggleで
/// 書けるから既定ONにできるのでは」と再検討する場合は、必ず上記
/// ドキュメントの「なぜ既定OFFにしたか」を先に読むこと。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn classify_mode_key_ime_action(
    key: ModeKeyCandidate,
    raw: &awase_gji_config::wire::GjiRawConfig,
) -> Option<ImeToggleKind> {
    if raw
        .overlay_keymaps
        .contains(&awase_gji_config::SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF)
    {
        match key {
            ModeKeyCandidate::Henkan => return Some(ImeToggleKind::On),
            ModeKeyCandidate::Muhenkan => return Some(ImeToggleKind::Off),
            ModeKeyCandidate::Hiragana | ModeKeyCandidate::Katakana => {
                // overlayはHenkan/Muhenkanのみ対象。次のソースへ。
            }
        }
    }
    if raw.session_keymap == Some(awase_gji_config::SESSION_KEYMAP_CUSTOM) {
        let Some(table) = &raw.custom_keymap_table else {
            return None;
        };
        let keys = awase_gji_config::keymap::extract_ime_keys(table);
        return classify_vk_in_ime_keys(&keys, key.vk_name());
    }
    match raw.session_keymap {
        Some(v) if v == awase_gji_config::SESSION_KEYMAP_ATOK => match key {
            ModeKeyCandidate::Henkan | ModeKeyCandidate::Muhenkan => Some(ImeToggleKind::Toggle),
            ModeKeyCandidate::Hiragana | ModeKeyCandidate::Katakana => None,
        },
        Some(v)
            if v == awase_gji_config::SESSION_KEYMAP_MSIME
                || v == awase_gji_config::SESSION_KEYMAP_MOBILE =>
        {
            match key {
                ModeKeyCandidate::Hiragana | ModeKeyCandidate::Katakana => Some(ImeToggleKind::On),
                ModeKeyCandidate::Henkan | ModeKeyCandidate::Muhenkan => None,
            }
        }
        // フィールド不在/NONE はWindows版GJIでは実質MSIME相当
        // （`config_handler.cc::GetDefaultKeyMap()`）なので、MSIMEと同じ
        // 結論（Hiragana/Katakanaのみ`On`）にfail-closedで倒す。
        None => match key {
            ModeKeyCandidate::Hiragana | ModeKeyCandidate::Katakana => Some(ImeToggleKind::On),
            ModeKeyCandidate::Henkan | ModeKeyCandidate::Muhenkan => None,
        },
        Some(_) => None, // KOTOERI/CHROMEOS/未知の値
    }
}

/// `GjiImeKeys`（`awase-gji-config::keymap::extract_ime_keys`の戻り値）から
/// 特定のVK名がon/off/toggleのどれに分類されているかを引く。
#[cfg_attr(not(windows), allow(dead_code))]
fn classify_vk_in_ime_keys(
    keys: &awase_gji_config::keymap::GjiImeKeys,
    vk_name: &str,
) -> Option<ImeToggleKind> {
    if keys.toggle.iter().any(|v| v == vk_name) {
        Some(ImeToggleKind::Toggle)
    } else if keys.on.iter().any(|v| v == vk_name) {
        Some(ImeToggleKind::On)
    } else if keys.off.iter().any(|v| v == vk_name) {
        Some(ImeToggleKind::Off)
    } else {
        None
    }
}

/// [`classify_thumb_key_ime_actions`]の結果にopt-inゲートを適用した最終判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ThumbKeyImeWiring {
    pub henkan: Option<ImeToggleKind>,
    pub muhenkan: Option<ImeToggleKind>,
    pub warning: ThumbKeyImeWarning,
}

/// `atok_opt_in`（`GeneralConfig::gji_thumb_key_ime_toggle`）を
/// [`classify_thumb_key_ime_actions`]の結果へ適用する（BUG-115）。
/// `Toggle`（非冪等）のみゲート対象——`On`/`Off`は常にそのまま反映してよい。
/// Henkan/Muhenkanそれぞれ独立にゲートする（一方だけ`Toggle`の場合もある）。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn gate_thumb_key_ime_actions(
    henkan: Option<ImeToggleKind>,
    muhenkan: Option<ImeToggleKind>,
    opt_in: bool,
) -> ThumbKeyImeWiring {
    let henkan_is_toggle = matches!(henkan, Some(ImeToggleKind::Toggle));
    let muhenkan_is_toggle = matches!(muhenkan, Some(ImeToggleKind::Toggle));
    let any_toggle = henkan_is_toggle || muhenkan_is_toggle;

    let warning = if !any_toggle {
        ThumbKeyImeWarning::None
    } else if opt_in {
        ThumbKeyImeWarning::ToggleHonored
    } else {
        ThumbKeyImeWarning::ToggleDeclined
    };

    let gate = |action: Option<ImeToggleKind>, is_toggle: bool| {
        if is_toggle && !opt_in {
            None
        } else {
            action
        }
    };
    ThumbKeyImeWiring {
        henkan: gate(henkan, henkan_is_toggle),
        muhenkan: gate(muhenkan, muhenkan_is_toggle),
        warning,
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::{reset_streak_latch_for_reload, sync_gji_charset_autodetect};

#[cfg(windows)]
mod windows_impl {
    use std::sync::atomic::{AtomicU8, Ordering};

    use awase::config::ParsedKeyCombo;
    use awase::types::{ShadowImeAction, VkCode};

    use crate::runtime::Runtime;

    use super::{
        classify_thumb_key_ime_actions, extract_ime_on_off_toggle_combos,
        gate_thumb_key_ime_actions, ImeToggleKind, ModeKeyCandidate, ThumbKeyImeWarning,
    };

    const NOT_GJI: u8 = 0;
    const GJI_CHECKED: u8 = 1;

    /// GJIが継続してアクティブな「区間」ごとに一度だけ判定するためのラッチ。
    /// `NOT_GJI`（GJI以外、または未判定）/`GJI_CHECKED`（この区間で判定済み）
    /// の2値。`sync_gji_charset_autodetect`と`reset_streak_latch_for_reload`
    /// 以外から触らない。
    static LAST_GJI_STREAK_CHECKED: AtomicU8 = AtomicU8::new(NOT_GJI);

    /// BUG-115: 直前にトグル関連の警告を出したかどうかのデデュープ
    /// （`session_keymap`/`custom_keymap_table`/`overlay_keymaps`の内容が
    /// 変わらない限り連呼しない）。`msime_key_assignment::LAST_WARNED`と
    /// 同型。`NOT_WARNED`は未警告、それ以外は`ThumbKeyImeWarning`を
    /// `u8`化した値。GJI離脱ではリセットしない（Q3方針:
    /// GJI⇔MS-IME往復のたびに再警告すると煩わしいため、内容が変わった
    /// ときだけ再警告する）。
    static LAST_TOGGLE_WARNING: AtomicU8 = AtomicU8::new(NOT_WARNED);
    const NOT_WARNED: u8 = 0xFF;

    /// `app/mod.rs::reload_config`から、GJI利用中の設定リロード時に呼ぶ
    /// （BUG-115 F4）。MS-IME側の`sync_ime_toggle_auto_detect`が設定
    /// リロードのたびに無条件で再読みするのと対称に、GJI側もラッチを
    /// リセットしてから`sync_gji_charset_autodetect`を呼び直すことで、
    /// `gji_thumb_key_ime_toggle`をユーザーが設定画面で変更した際に
    /// 次のGJIストリークまで（＝再起動するまで）反映されない、という
    /// stale化を防ぐ。ADR-091の「継続的ポーリングをしない」とは矛盾しない
    /// （reloadはユーザー起点の離散イベントであり、ポーリングではない）。
    pub(crate) fn reset_streak_latch_for_reload(app: &mut Runtime) {
        LAST_GJI_STREAK_CHECKED.store(NOT_GJI, Ordering::Relaxed);
        sync_gji_charset_autodetect(app, true);
    }

    /// `runtime::message_handlers::sync_ime_kind_from_observation`から呼ぶ、
    /// GJI検出/離脱の唯一の合流点（`msime_key_assignment::check_and_warn`と対）。
    /// IME ON/OFF/トグルキー（ADR-092 Step4c）と、無変換/変換キーの
    /// IME on/off/toggle意味論（BUG-115）の自動判定を行う。
    ///
    /// - **GJI以外への遷移**: 直前がGJI継続区間だった場合のみ、自動検出して
    ///   いた`ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`を解除する
    ///   （delegate-to-open-axisは解除しない——GJI以外への遷移では必ず
    ///   MS-IME側の`sync_ime_toggle_auto_detect`が無条件で上書きするため
    ///   冪等で、この関数で解除するのは冗長。ただし`ActiveImeKind`が
    ///   将来3値以上に増えた場合はこの前提が崩れる点に注意）。
    /// - **GJIへの新規遷移**: `config1.db`を読み、宣言されているIME ON/OFF/
    ///   トグルキー（安全範囲内のみ）と、無変換/変換キーの割当てを
    ///   `Engine`へ反映する。既にこのGJI継続区間でチェック済みなら（ラッチが
    ///   `GJI_CHECKED`のまま）何もしない——継続的なポーリングをしないため。
    pub(crate) fn sync_gji_charset_autodetect(app: &mut Runtime, is_gji: bool) {
        if !is_gji {
            if LAST_GJI_STREAK_CHECKED.swap(NOT_GJI, Ordering::Relaxed) == GJI_CHECKED {
                log::info!(
                    "[gji-charset-autodetect] GJI から離脱: 自動検出したIME ON/OFFキーを解除"
                );
                // ime_on_auto/ime_off_auto/ime_toggle_auto を全て解除する
                // （さもないと GJI 離脱後も別アプリ/別IMEの文脈にF15-F24の
                // バインドや、無変換/変換の非親指キー扱い分（BUG-115）が
                // 残留してしまう）。ime_toggle_auto は MS-IME 側
                // （sync_ime_toggle_auto_detect）とも共有するフィールドだが、
                // 呼び出し元（message_handlers::sync_ime_kind_from_observation）
                // が GJI 側の同期を MS-IME 側より**先に**呼ぶ順序になっている
                // ため、GJI→MS-IME遷移ではここで解除した直後に MS-IME 側が
                // 新しい値で上書きし、破綻しない（Opus コードレビュー指摘）。
                app.clear_gji_ime_on_off_auto_keys();
            }
            return;
        }
        if LAST_GJI_STREAK_CHECKED.swap(GJI_CHECKED, Ordering::Relaxed) == GJI_CHECKED {
            return;
        }

        let bytes = read_config1_db();
        let raw = bytes
            .as_deref()
            .and_then(awase_gji_config::wire::parse_top_level);

        // BUG-115（F2、must-fix）: 無変換/変換のIME意味論は、あらゆる早期
        // return（config1.dbが読めない・session_keymap!=CUSTOMゲート）より
        // **前**に、無条件で計算・反映する。理由はMozcのoverlay_keymaps/
        // session_keymap意味論のためだけでなく、MS-IME→GJI遷移時に
        // MS-IMEレジストリ由来の値（`sync_ime_toggle_auto_detect`が
        // セットしたもの）を上書きする**唯一の書き込み点**がここだから
        // （MS-IME側の同期は`kind == MicrosoftIme`でしか走らない、
        // `message_handlers.rs`参照）。ここより後ろに動かすと、
        // 「MS-IMEでKeyAssignmentMuhenkan=1設定→GJIへ切替→config1.dbが
        // 読めない」という経路で無変換がMS-IME由来のTurnOffのまま
        // GJI上で発火する回帰が復活する。
        let default_raw = awase_gji_config::wire::GjiRawConfig::default();
        let (henkan_kind, muhenkan_kind) =
            classify_thumb_key_ime_actions(raw.as_ref().unwrap_or(&default_raw));
        let wiring = gate_thumb_key_ime_actions(
            henkan_kind,
            muhenkan_kind,
            app.gji_thumb_key_ime_toggle_opt_in(),
        );
        warn_thumb_key_toggle_if_needed(app, wiring.warning);

        // BUG-115（F7）: 無変換/変換が親指シフトのチョードキーとして
        // 設定されている場合のみ、Step4bと同じdelegate-to-open-axis
        // （単独タップ確定経路のみで発火、チョード打鍵とは物理的に
        // 衝突しない）に載せる。設定されていない場合、その物理キーは
        // NICOLAの保留状態機械を一切通らない素のキーなので、代わりに
        // Step4cと同じ`ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`
        // （押されたら常にactuationしてよい、既存のBUG-14
        // injected除外もそのまま効く）に載せる。
        let mut on = Vec::new();
        let mut off = Vec::new();
        let mut toggle = Vec::new();
        let henkan_delegate = route_thumb_key_action(
            wiring.henkan,
            is_configured_thumb_key(ModeKeyCandidate::Henkan.vk()),
            ModeKeyCandidate::Henkan.vk(),
            &mut on,
            &mut off,
            &mut toggle,
        );
        let muhenkan_delegate = route_thumb_key_action(
            wiring.muhenkan,
            is_configured_thumb_key(ModeKeyCandidate::Muhenkan.vk()),
            ModeKeyCandidate::Muhenkan.vk(),
            &mut on,
            &mut off,
            &mut toggle,
        );
        app.set_gji_thumb_key_delegate_to_open_axis(henkan_delegate, muhenkan_delegate);

        // BUG-115（Phase 1ホリスティックレビューで撤去、2026-09-05）:
        // 「ひらがな/カタカナ」キーをここでactuation-auto（on/off/toggle）に
        // 載せる処理がかつて存在したが、VK_DBE_HIRAGANA/VK_DBE_KATAKANAは
        // `vk.rs::ImeKeyKind::from_vk`の静的マップで既にshadow_action
        // （TurnOn固定）を持ち、`key_pipeline.rs::kp_stage_shadow_ime_toggle`
        // が毎打鍵belief更新とactuationの両方を行う。ここで同じキーを
        // actuation-autoにも載せると、撤回されたADR-135 Phase 2 v1
        // （nicola_fsm汎用化案）と同型の二重actuationになる
        // （`Engine::ime_set_open_effects`はstateが変化しなくても
        // `Effect::Ime(SetOpen)`を無条件に積む、`engine.rs:810-829`）。
        // GJI側の意味論（Toggleかもしれない等）をshadow_action側へ正しく
        // 反映する設計はADR-135「Phase 2（再設計）」（shadow_actionの
        // オーバーレイ）で扱う。`classify_mode_key_ime_action`自体と
        // そのHiragana/Katakana向けテストは、Phase 2実装時に再利用する
        // ためこのファイルに残してある（呼び出しは削除済み）。

        let Some(raw) = raw else {
            log::debug!(
                "[gji-charset-autodetect] config1.db を読めませんでした \
                 （GJI 未インストール、または初回起動でまだ作成されていない等）"
            );
            app.set_gji_ime_on_off_toggle_auto_keys(on, off, toggle);
            return;
        };
        if raw.session_keymap != Some(awase_gji_config::SESSION_KEYMAP_CUSTOM) {
            // session_keymap が CUSTOM でなければ（ATOK/MS-IME 等のプリセット
            // 選択中）、custom_keymap_table に何が残っていても GJI はそれを
            // 参照しない。古いカスタムテーブルの残骸を誤って有効と判定しない
            // ための必須ガード（Opus レビュー指摘）。上の無変換/変換の
            // IME意味論判定はこのreturnより前に既に完了しているため、
            // ここでの early returnはF15-F24自動検出だけをスキップする
            // （BUG-115、Step4c fix#4と同型の再発条件——配置順は
            // 回帰テストで固定）。
            log::debug!(
                "[gji-charset-autodetect] session_keymap が CUSTOM ではないため \
                 F15-F24自動判定をスキップ: {:?}",
                raw.session_keymap
            );
            app.set_gji_ime_on_off_toggle_auto_keys(on, off, toggle);
            return;
        }
        let Some(table) = raw.custom_keymap_table else {
            app.set_gji_ime_on_off_toggle_auto_keys(on, off, toggle);
            return;
        };

        // ADR-092 決定D Step4c: 2026-08-16 ユーザー判断:
        // `Engine::match_ime_on_off_auto`/`match_ime_toggle_auto`は
        // `special_keys.ime_on/ime_off/ime_toggle`が非空でも自動リストを
        // 併用する（明示 ∪ 自動、手動排他ではない）ため、ここでの事前
        // チェックは元々不要（Step4aと同じ規約）。
        let (f_on, f_off, f_toggle) = extract_ime_on_off_toggle_combos(&table);
        on.extend(f_on);
        off.extend(f_off);
        toggle.extend(f_toggle);
        if !on.is_empty() || !off.is_empty() || !toggle.is_empty() {
            log::info!(
                "[gji-charset-autodetect] config1.db から IME ON/OFF/トグルキーを \
                 自動検出しました: on={on:?} off={off:?} toggle={toggle:?}"
            );
        }
        app.set_gji_ime_on_off_toggle_auto_keys(on, off, toggle);
    }

    /// `vk`が現在`left_thumb_key`/`right_thumb_key`のいずれかに設定されて
    /// いるか（BUG-115）。`crate::hook::thumb_vk_codes()`
    /// （`apply_config_update`/起動時に更新される、常に最新の親指キー
    /// ペア）と比較する汎用ヘルパー——無変換/変換に限らず任意のVKに使える。
    fn is_configured_thumb_key(vk: VkCode) -> bool {
        let (left, right) = crate::hook::thumb_vk_codes();
        vk == left || vk == right
    }

    /// [`ImeToggleKind`]の判定結果を、`is_thumb_key`に応じてStep4b
    /// delegate-to-open-axis（`Some(ShadowImeAction)`を返す）か、
    /// Step4cと同じactuation-auto（`on`/`off`/`toggle`へ`vk`を積む）
    /// のどちらかへ振り分ける（BUG-115 F7）。
    fn route_thumb_key_action(
        action: Option<ImeToggleKind>,
        is_thumb_key: bool,
        vk: VkCode,
        on: &mut Vec<ParsedKeyCombo>,
        off: &mut Vec<ParsedKeyCombo>,
        toggle: &mut Vec<ParsedKeyCombo>,
    ) -> Option<ShadowImeAction> {
        let action = action?;
        if is_thumb_key {
            return Some(match action {
                ImeToggleKind::On => ShadowImeAction::TurnOn,
                ImeToggleKind::Off => ShadowImeAction::TurnOff,
                ImeToggleKind::Toggle => ShadowImeAction::Toggle,
            });
        }
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk,
        };
        match action {
            ImeToggleKind::On => on.push(combo),
            ImeToggleKind::Off => off.push(combo),
            ImeToggleKind::Toggle => toggle.push(combo),
        }
        None
    }

    /// BUG-115（N8）: 無変換/変換のIME意味論判定結果をユーザーへ通知する。
    /// 同一内容の警告はプロセス内で一度だけ
    /// （`msime_key_assignment::check_and_warn`と同型のデデュープ）。
    fn warn_thumb_key_toggle_if_needed(app: &Runtime, warning: ThumbKeyImeWarning) {
        let packed = warning as u8;
        if LAST_TOGGLE_WARNING.swap(packed, Ordering::Relaxed) == packed {
            return; // 同じ内容で通知済み
        }
        match warning {
            ThumbKeyImeWarning::None => {}
            ThumbKeyImeWarning::ToggleDeclined => {
                log::warn!(
                    "[gji-charset-autodetect] GJIの設定（ATOKプリセット、またはカスタム\
                     キーマップ）が無変換/変換キー単体に状態依存のIME ON/OFFトグルを\
                     割り当てており、awaseの想定と衝突する可能性があります。対処法: \
                     (1) GJIの設定でキーマップをカスタム(矛盾のない割当て)またはMS-IME等\
                     へ変更する、(2) GJIの設定でオーバーレイ「無変換キーをIMEオフ、変換\
                     キーをIMEオンに割り当てる」を有効にする（awaseは既に対応済み）、\
                     (3) 挙動を理解した上でawaseのconfig.tomlで\
                     `gji_thumb_key_ime_toggle = true`を設定し、ベストエフォートで\
                     追従させる（自己責任、詳細はdocs/known-bugs.md BUG-115参照）。"
                );
            }
            ThumbKeyImeWarning::ToggleHonored => {
                log::info!(
                    "[gji-charset-autodetect] gji_thumb_key_ime_toggle=true \
                     設定により、無変換/変換キーの状態依存トグルをベストエフォートで \
                     反映しました（docs/known-bugs.md BUG-115参照）。"
                );
            }
        }
        if is_configured_thumb_key(ModeKeyCandidate::Muhenkan.vk())
            && app.muhenkan_dedicated_fn_key_configured()
            && !matches!(warning, ThumbKeyImeWarning::None)
        {
            // F5: 専用Fnキー（muhenkan_solo_tap_dedicated_fn_key）が優先
            // されるため、無変換が親指キーの場合、そのdelegate-to-open-axis
            // は黙って無効化される（`resolve_pending_thumb_as_single`の
            // 優先順位、henkan側には専用Fnキーの概念自体が無い非対称）。
            // 無変換が親指キーでない場合はactuation-auto経由になり
            // dedicated_fn_keyとは無関係なので、この警告は不要。
            log::warn!(
                "[gji-charset-autodetect] muhenkan_solo_tap_dedicated_fn_keyが設定済みの \
                 ため、無変換キーのIME open軸への追従は無効化されます（変換キー側のみ \
                 有効）。詳細はdocs/known-bugs.md BUG-115参照。"
            );
        }
    }

    /// `config1.db`のパス。`%USERPROFILE%\AppData\LocalLow\Google\Google Japanese Input\config1.db`
    /// （実機確認済み、Google 日本語入力はIMEとして低整合性レベルのプロセスから
    /// も読める必要があるため`LocalLow`配下に置かれる）。
    fn config1_db_path() -> Option<std::path::PathBuf> {
        let profile = std::env::var_os("USERPROFILE")?;
        let mut path = std::path::PathBuf::from(profile);
        path.push("AppData");
        path.push("LocalLow");
        path.push("Google");
        path.push("Google Japanese Input");
        path.push("config1.db");
        Some(path)
    }

    /// `config1.db`を読む。存在しない・読めない場合は`None`（エラーにしない、
    /// GJI未インストール環境を正常系として扱う）。
    fn read_config1_db() -> Option<Vec<u8>> {
        let path = config1_db_path()?;
        std::fs::read(&path).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::extract_ime_on_off_toggle_combos;
    use crate::vk::VkCodeExt as _;
    use awase::types::VkCode;

    // ── extract_ime_on_off_toggle_combos (ADR-092 決定D Step4c) ──

    fn combo(vk_name: &str) -> awase::config::ParsedKeyCombo {
        awase::config::ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VkCode::from_name(vk_name).unwrap(),
        }
    }

    #[test]
    fn no_bindings_yields_all_empty() {
        let (on, off, toggle) = extract_ime_on_off_toggle_combos("");
        assert_eq!(on, vec![]);
        assert_eq!(off, vec![]);
        assert_eq!(toggle, vec![]);
    }

    /// `awase-gji-config::keymap`側の`extracts_toggle_on_off_from_fixture`と
    /// 同じフィクスチャ。安全範囲外（F13/`VK_DBE_ALPHANUMERIC`/`VK_IME_ON`/
    /// `VK_IME_OFF`/`VK_KANJI`）は全て除外され、F15-F24範囲内のF21/F22だけが
    /// 残ることを確認する（BUG-14 注入イベント衝突リスクの回避、Opusレビュー
    /// 指摘の反映）。
    #[test]
    fn filters_out_bug14_risky_aliases_and_out_of_range_fn_keys() {
        let table = "status\tkey\tcommand
Composition\tHankaku/Zenkaku\tIMEOff
Conversion\tHankaku/Zenkaku\tIMEOff
DirectInput\tHankaku/Zenkaku\tIMEOn
Precomposition\tHankaku/Zenkaku\tIMEOff
Composition\tKanji\tIMEOff
Conversion\tKanji\tIMEOff
DirectInput\tKanji\tIMEOn
Precomposition\tKanji\tIMEOff
DirectInput\tF13\tIMEOn
DirectInput\tF21\tIMEOn
Precomposition\tF21\tIMEOn
Composition\tF21\tIMEOn
Conversion\tF21\tIMEOn
Precomposition\tF22\tIMEOff
Composition\tF22\tIMEOff
Conversion\tF22\tIMEOff
Composition\tON\tIMEOn
Composition\tOFF\tIMEOff
Conversion\tON\tIMEOn
Conversion\tOFF\tIMEOff
DirectInput\tON\tIMEOn
Precomposition\tON\tIMEOn
Precomposition\tOFF\tIMEOff
DirectInput\tEisu\tIMEOn
Composition\tEisu\tToggleAlphanumericMode
Conversion\tEisu\tToggleAlphanumericMode
Precomposition\tEisu\tToggleAlphanumericMode
";
        let (on, off, toggle) = extract_ime_on_off_toggle_combos(table);
        // 生の GjiImeKeys.on は [VK_DBE_ALPHANUMERIC, VK_F13, VK_F21, VK_IME_ON]
        // だが、安全範囲(F15-F24)外を全て除外すると VK_F21 のみ残る。
        assert_eq!(on, vec![combo("VK_F21")]);
        // 生の GjiImeKeys.off は [VK_F22, VK_IME_OFF] だが VK_IME_OFF は除外。
        assert_eq!(off, vec![combo("VK_F22")]);
        // 生の GjiImeKeys.toggle は [VK_KANJI] のみで、安全範囲外のため全除外。
        assert_eq!(toggle, vec![]);
    }

    /// 安全範囲内（F15-F24）のトグルキーはそのまま採用される。
    #[test]
    fn safe_range_toggle_key_is_kept() {
        let table = "status\tkey\tcommand\nDirectInput\tF20\tIMEOn\nPrediction\tF20\tIMEOff\n";
        let (on, off, toggle) = extract_ime_on_off_toggle_combos(table);
        assert_eq!(on, vec![]);
        assert_eq!(off, vec![]);
        assert_eq!(toggle, vec![combo("VK_F20")]);
    }

    // ── classify_thumb_key_ime_actions / gate_thumb_key_ime_actions (BUG-115) ──

    use super::{
        classify_mode_key_ime_action, classify_thumb_key_ime_actions, gate_thumb_key_ime_actions,
        ImeToggleKind, ModeKeyCandidate, ThumbKeyImeWarning,
    };
    use awase_gji_config::wire::GjiRawConfig;

    fn raw_with_overlay() -> GjiRawConfig {
        GjiRawConfig {
            overlay_keymaps: vec![
                awase_gji_config::SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF,
            ],
            ..GjiRawConfig::default()
        }
    }

    fn raw_with_session_keymap(value: i64) -> GjiRawConfig {
        GjiRawConfig {
            session_keymap: Some(value),
            ..GjiRawConfig::default()
        }
    }

    #[test]
    fn classify_overlay_yields_on_off_unconditionally() {
        let (henkan, muhenkan) = classify_thumb_key_ime_actions(&raw_with_overlay());
        assert_eq!(henkan, Some(ImeToggleKind::On));
        assert_eq!(muhenkan, Some(ImeToggleKind::Off));
    }

    /// overlayはsession_keymapに関わらず最優先（Mozcがoverlayをbase
    /// keymapの上に重ね掛けする実装と対応）。ATOKと同時に該当していても
    /// overlayが勝つ。
    #[test]
    fn classify_overlay_wins_over_atok_session_keymap() {
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_ATOK),
            overlay_keymaps: vec![
                awase_gji_config::SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF,
            ],
            ..GjiRawConfig::default()
        };
        let (henkan, muhenkan) = classify_thumb_key_ime_actions(&raw);
        assert_eq!(henkan, Some(ImeToggleKind::On));
        assert_eq!(muhenkan, Some(ImeToggleKind::Off));
    }

    #[test]
    fn classify_atok_preset_yields_toggle_for_both_keys() {
        let raw = raw_with_session_keymap(awase_gji_config::SESSION_KEYMAP_ATOK);
        let (henkan, muhenkan) = classify_thumb_key_ime_actions(&raw);
        assert_eq!(henkan, Some(ImeToggleKind::Toggle));
        assert_eq!(muhenkan, Some(ImeToggleKind::Toggle));
    }

    /// MSIME/MOBILE/KOTOERI/CHROMEOS、フィールド不在はいずれも
    /// 割り当てなし（本家の各tsvにHenkan/Muhenkanの開閉意味論が無いことを
    /// 2026-09-05に確認済み）。
    #[test]
    fn classify_other_presets_and_absent_yield_none() {
        for value in [2, 4, 3, 5] {
            let raw = raw_with_session_keymap(value);
            assert_eq!(
                classify_thumb_key_ime_actions(&raw),
                (None, None),
                "session_keymap={value}"
            );
        }
        assert_eq!(
            classify_thumb_key_ime_actions(&GjiRawConfig::default()),
            (None, None)
        );
    }

    /// BUG-115: CUSTOMキーマップにliteralなHenkan/Muhenkanトークンが
    /// 含まれる場合、`extract_ime_keys`経由で分類される。
    #[test]
    fn classify_custom_keymap_with_literal_henkan_muhenkan_tokens() {
        let table = "status\tkey\tcommand\nDirectInput\tHenkan\tIMEOn\n";
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_CUSTOM),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        let (henkan, muhenkan) = classify_thumb_key_ime_actions(&raw);
        assert_eq!(henkan, Some(ImeToggleKind::On));
        assert_eq!(muhenkan, None);
    }

    /// CUSTOMだがHenkan/Muhenkanトークンが無いテーブルは割り当てなし。
    #[test]
    fn classify_custom_keymap_without_henkan_muhenkan_yields_none() {
        let table = "status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n";
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_CUSTOM),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(classify_thumb_key_ime_actions(&raw), (None, None));
    }

    // ── classify_mode_key_ime_action: Hiragana/Katakana (BUG-115、ひらがな
    // キーを親指シフトキーに設定しているユーザー向けエッジケース) ──

    /// MSIME/MOBILEプリセットは`DirectInput`状態でHiragana/Katakana双方を
    /// `IMEOn`に割り当てている（本家tsv、2026-09-05確認済み）。
    /// Henkan/Muhenkanはこれらのプリセットでは無関係のまま(`None`)。
    #[test]
    fn classify_msime_mobile_preset_yields_on_for_hiragana_katakana() {
        for value in [
            awase_gji_config::SESSION_KEYMAP_MSIME,
            awase_gji_config::SESSION_KEYMAP_MOBILE,
        ] {
            let raw = raw_with_session_keymap(value);
            assert_eq!(
                classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
                Some(ImeToggleKind::On),
                "session_keymap={value}"
            );
            assert_eq!(
                classify_mode_key_ime_action(ModeKeyCandidate::Katakana, &raw),
                Some(ImeToggleKind::On),
                "session_keymap={value}"
            );
            assert_eq!(
                classify_mode_key_ime_action(ModeKeyCandidate::Henkan, &raw),
                None
            );
            assert_eq!(
                classify_mode_key_ime_action(ModeKeyCandidate::Muhenkan, &raw),
                None
            );
        }
    }

    /// フィールド不在はWindows版GJIの実質既定(MSIME)に倣い、
    /// Hiragana/Katakanaは`On`にfail-closedで倒す
    /// （`config_handler.cc::GetDefaultKeyMap()`で確認済み）。
    #[test]
    fn classify_absent_session_keymap_yields_on_for_hiragana_katakana() {
        let raw = GjiRawConfig::default();
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
            Some(ImeToggleKind::On)
        );
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Katakana, &raw),
            Some(ImeToggleKind::On)
        );
    }

    /// ATOKプリセットにはHiragana/Katakanaへの割当てが無い（本家tsvに
    /// 該当行なし、2026-09-05確認済み）。
    #[test]
    fn classify_atok_preset_yields_none_for_hiragana_katakana() {
        let raw = raw_with_session_keymap(awase_gji_config::SESSION_KEYMAP_ATOK);
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
            None
        );
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Katakana, &raw),
            None
        );
    }

    /// KOTOERI/CHROMEOSはHiragana/Katakana関連行が無い。
    #[test]
    fn classify_kotoeri_chromeos_yield_none_for_hiragana_katakana() {
        for value in [3, 5] {
            let raw = raw_with_session_keymap(value);
            assert_eq!(
                classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
                None,
                "session_keymap={value}"
            );
            assert_eq!(
                classify_mode_key_ime_action(ModeKeyCandidate::Katakana, &raw),
                None,
                "session_keymap={value}"
            );
        }
    }

    /// BUG-115: CUSTOMキーマップにliteralなHiraganaトークンが含まれる場合も
    /// `extract_ime_keys`経由で分類される（Henkan/Muhenkanと同じ経路）。
    #[test]
    fn classify_custom_keymap_with_literal_hiragana_token() {
        let table = "status\tkey\tcommand\nDirectInput\tHiragana\tIMEOn\n";
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_CUSTOM),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
            Some(ImeToggleKind::On)
        );
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Katakana, &raw),
            None
        );
    }

    /// overlay(Henkan/Muhenkan専用)はHiragana/Katakanaには効かない——
    /// overlay該当時でも次のソース(session_keymap)へフォールスルーする。
    #[test]
    fn classify_overlay_does_not_affect_hiragana_katakana() {
        let raw = raw_with_overlay(); // session_keymap不在 + overlay=100
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
            Some(ImeToggleKind::On), // overlayではなく、フィールド不在→MSIME既定経由
        );
    }

    /// `ModeKeyCandidate::vk()`が全バリアントで`VkCode::from_name`の
    /// 解決に失敗しない（`unreachable!`に到達しない）ことを固定する。
    #[test]
    fn mode_key_candidate_vk_resolves_for_all_variants() {
        use crate::vk::VkCodeExt as _;
        use awase::types::VkCode;
        for key in [
            ModeKeyCandidate::Henkan,
            ModeKeyCandidate::Muhenkan,
            ModeKeyCandidate::Hiragana,
            ModeKeyCandidate::Katakana,
        ] {
            assert_eq!(Some(key.vk()), VkCode::from_name(key.vk_name()), "{key:?}");
        }
    }

    #[test]
    fn gate_on_off_is_never_declined_regardless_of_opt_in() {
        for opt_in in [false, true] {
            let wiring = gate_thumb_key_ime_actions(
                Some(ImeToggleKind::On),
                Some(ImeToggleKind::Off),
                opt_in,
            );
            assert_eq!(wiring.henkan, Some(ImeToggleKind::On));
            assert_eq!(wiring.muhenkan, Some(ImeToggleKind::Off));
            assert_eq!(wiring.warning, ThumbKeyImeWarning::None);
        }
    }

    #[test]
    fn gate_toggle_without_opt_in_is_declined() {
        let wiring = gate_thumb_key_ime_actions(
            Some(ImeToggleKind::Toggle),
            Some(ImeToggleKind::Toggle),
            false,
        );
        assert_eq!(wiring.henkan, None);
        assert_eq!(wiring.muhenkan, None);
        assert_eq!(wiring.warning, ThumbKeyImeWarning::ToggleDeclined);
    }

    #[test]
    fn gate_toggle_with_opt_in_is_honored() {
        let wiring = gate_thumb_key_ime_actions(
            Some(ImeToggleKind::Toggle),
            Some(ImeToggleKind::Toggle),
            true,
        );
        assert_eq!(wiring.henkan, Some(ImeToggleKind::Toggle));
        assert_eq!(wiring.muhenkan, Some(ImeToggleKind::Toggle));
        assert_eq!(wiring.warning, ThumbKeyImeWarning::ToggleHonored);
    }

    /// 片方だけToggleの場合も、opt-inゲートは独立にキーごとへ適用される。
    #[test]
    fn gate_applies_independently_per_key() {
        let wiring = gate_thumb_key_ime_actions(
            Some(ImeToggleKind::Toggle),
            Some(ImeToggleKind::Off),
            false,
        );
        assert_eq!(wiring.henkan, None); // Toggleはopt-inなしで却下
        assert_eq!(wiring.muhenkan, Some(ImeToggleKind::Off)); // Offはそのまま反映
        assert_eq!(wiring.warning, ThumbKeyImeWarning::ToggleDeclined);
    }
}
