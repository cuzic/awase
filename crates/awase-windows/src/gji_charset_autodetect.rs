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

use awase::types::{ShadowImeAction, VkCode};

use crate::vk::VkCodeExt as _;

/// GJIが無変換/変換キーに割り当てているIME意味論の分類（BUG-115）。
/// `session_keymap`/`custom_keymap_table`/`overlay_keymaps`のどれ由来でも
/// 同じ3値に潰す——awase側の反応（`shadow_action` override経由の
/// follow-only、ADR-179）は`On`/`Off`なら常に安全（冪等）、`Toggle`のときだけ
/// opt-inゲートの対象になる。
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
    /// 対処法を案内する`tracing::warn!`が必要。
    ToggleDeclined,
    /// 状態依存トグルを、ユーザーのopt-in設定によりベストエフォートで
    /// 反映した。`tracing::info!`で通知する。
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
///    プリセット由来の行をそのままコピーした場合は4.と同じ`Toggle`に
///    classifyされる）。Henkan/Muhenkanそれぞれ独立に判定する
///    （一方だけ設定されている場合もある）。
/// 3. **`session_keymap`がCUSTOM以外でも`custom_keymap_table`に該当行が
///    ある場合はそれを優先する**（ADR-174実機検証、2026-09-15）:
///    `session_keymap`がプリセット値（実機でMSIME=2を確認）のままでも、
///    `custom_keymap_table`にユーザーが個別上書きした行
///    （`DirectInput\tHenkan\tIMEOn`等、F15-F19のSetMode割り当てと
///    共存する形で実機確認済み）が残っていることがある。テーブルに
///    該当行が無ければ4.のプリセット静的知識へフォールスルーする。
/// 4. **`session_keymap == ATOK`（overlay無し、custom無し）**:
///    `google/mozc`の`src/data/keymap/atok.tsv`（2026-09-05取得）は、
///    Henkan/Muhenkan双方を`DirectInput`状態で`IMEOn`、`Precomposition`
///    状態で`CancelAndIMEOff`に割り当てている——`ctx.ime_on`の値に応じて
///    反転する割当てだが、`ShadowImeAction::Toggle`
///    （`Engine::apply_ime_open_request`の`Toggle => !ctx.ime_on`）で
///    **正確に表現できる**（「表現不能」ではない）。
/// 5. **それ以外**（`MSIME`/`MOBILE`/`KOTOERI`/`CHROMEOS`/フィールド不在/
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
/// [`classify_mode_key_ime_action`]は、無変換/変換向けの既存delegateに加え、
/// ADR-135 Phase 2/3でHiragana/Katakanaのshadow_action overrideと
/// delegate-to-open-axisにも使う。Hiragana/Katakanaをactuation-autoへ
/// 載せる処理は、既存shadow-toggleとの二重actuationを作るため採用しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ModeKeyCandidate {
    Henkan,
    Muhenkan,
    Hiragana,
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

    /// ADR-176決定6（176-T12）: 現在の`config1.db`内容から、このキーの
    /// 較正フィンガープリントを構築する。
    /// `state::calibrated_mode_key::fresh_or_none`によるstale判定専用
    /// ——値の意味解釈は[`classify_mode_key_ime_action`]が別途行う。
    #[cfg_attr(not(windows), allow(dead_code))]
    fn current_fingerprint(
        self,
        raw: &awase_gji_config::wire::GjiRawConfig,
    ) -> crate::state::calibrated_mode_key::ConfigFingerprint {
        let relevant_row = raw.custom_keymap_table.as_deref().and_then(|table| {
            awase_gji_config::keymap::relevant_rows_for_vk(table, self.vk_name())
        });
        crate::state::calibrated_mode_key::ConfigFingerprint::Gji {
            session_keymap: raw.session_keymap,
            relevant_row,
        }
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
/// 3. **`session_keymap`がCUSTOM以外でも`custom_keymap_table`に該当行が
///    ある場合はそれを優先する**（ADR-174実機検証、2026-09-15）:
///    `session_keymap`がプリセット値（実機でMSIME=2を確認）のままでも、
///    `custom_keymap_table`にユーザーが個別上書きした行
///    （`DirectInput\tHenkan\tIMEOn`等、F15-F19のSetMode割り当てと
///    共存する形で実機確認済み）が残っていることがある。テーブルに
///    該当行が無ければ4.のプリセット静的知識へフォールスルーする。
/// 4. **`session_keymap`がプリセット（overlay/custom無し）**:
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
/// 5. **それ以外**（フィールド不在/未知の値、または`CUSTOM`だが該当
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
    // ADR-174実機検証（2026-09-15）: `session_keymap`が`CUSTOM`以外の値
    // （実機でMSIME=2を確認）でも、`custom_keymap_table`にこのキーの
    // 明示的な行（実機で`DirectInput\tHenkan\tIMEOn`を確認）が実在する
    // ことがある——旧実装はこの場合`custom_keymap_table`を一切参照せず
    // 下記プリセット静的知識（Henkan/Muhenkanは`None`）へ落ち、実際に
    // GJIがIMEを開いてもawaseのbeliefが追従しなかった。`session_keymap`
    // がプリセット値のままでも`custom_keymap_table`にユーザーが個別に
    // 上書きした行が残る実例（F15-F19のSetMode等と共存）が実機で確認
    // 済みのため、テーブルに該当行があればプリセットの静的知識より
    // 優先する。テーブルに該当行が無ければ下記のプリセット分岐へ
    // フォールスルーする（`session_keymap == CUSTOM`の場合はこの
    // フォールスルーを行わない——真にCUSTOM選択時は「テーブルに無い
    // ＝割り当てなし」がGJIの実際の意味論であり、他プリセットの静的
    // 知識を借用する根拠が無いため、上のCUSTOM専用分岐のまま`None`を
    // 返す）。
    // ADR-186(実機スパイク、2026-09-20): `session_keymap == ATOK`では、`config1.db`に残る
    // 古い`custom_keymap_table`は**GJIに使われない**。実機(ATOK)で、表に
    // `DirectInput\tHenkan\tIMEOn`/`Precomposition\tHenkan\tCompositionModeHiragana`が残って
    // いても、変換は`atok.tsv`どおり開閉トグル(ON中→OFF)として動いた(`docs/adr/186-measurements/`)。
    // 表を優先するとHenkanが`On`(冪等・belief追随のみ・生キー素通し)と誤分類され、GJIの実トグル
    // とbeliefが逆になる(Muhenkanは表に行が無くATOKの`Toggle`になり非対称)。ATOKでは表を読まず
    // 下のプリセット分岐へ進む。MSIME等はADR-174の実機根拠があるため従来どおり表を優先する。
    if raw.session_keymap != Some(awase_gji_config::SESSION_KEYMAP_ATOK) {
        if let Some(table) = &raw.custom_keymap_table {
            let keys = awase_gji_config::keymap::extract_ime_keys(table);
            if let Some(found) = classify_vk_in_ime_keys(&keys, key.vk_name()) {
                return Some(found);
            }
        }
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

#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const fn ime_toggle_kind_to_shadow_action(
    kind: ImeToggleKind,
    opt_in: bool,
) -> Option<ShadowImeAction> {
    match kind {
        ImeToggleKind::On => Some(ShadowImeAction::TurnOn),
        ImeToggleKind::Off => Some(ShadowImeAction::TurnOff),
        ImeToggleKind::Toggle if opt_in => Some(ShadowImeAction::Toggle),
        ImeToggleKind::Toggle => None,
    }
}

/// `ImeToggleKind`と`ShadowImeAction`は同型（On/Off/Toggleの3値）だが、
/// GJI由来の分類（`ImeToggleKind`）とMS-IMEレジストリ由来の分類
/// （`ShadowImeAction`、`msime_key_assignment.rs`）で別の型として扱われて
/// いる。ADR-176（較正結果の適用、176-T4）はどちらの経路でも同じ
/// `CalibratedModeKey::result: ImeToggleKind`を使うため、opt-in条件を
/// 挟まない直接の相互変換をここに用意する（`ime_toggle_kind_to_shadow_action`
/// と違い`Toggle`を無条件に変換する——opt-inによる`Toggle`抑制は
/// 呼び出し元がこの関数を使う前/後に別途行う）。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const fn shadow_action_to_ime_toggle_kind(action: ShadowImeAction) -> ImeToggleKind {
    match action {
        ShadowImeAction::TurnOn => ImeToggleKind::On,
        ShadowImeAction::TurnOff => ImeToggleKind::Off,
        ShadowImeAction::Toggle => ImeToggleKind::Toggle,
    }
}

/// [`shadow_action_to_ime_toggle_kind`]の逆変換。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const fn ime_toggle_kind_to_shadow_action_direct(
    kind: ImeToggleKind,
) -> ShadowImeAction {
    match kind {
        ImeToggleKind::On => ShadowImeAction::TurnOn,
        ImeToggleKind::Off => ShadowImeAction::TurnOff,
        ImeToggleKind::Toggle => ShadowImeAction::Toggle,
    }
}

#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_gji_mode_key_shadow_overrides(
    is_gji: bool,
    raw: Option<&awase_gji_config::wire::GjiRawConfig>,
    opt_in: bool,
) -> (Option<ShadowImeAction>, Option<ShadowImeAction>) {
    if !is_gji {
        return (None, None);
    }
    let Some(raw) = raw else {
        return (None, None);
    };
    let hiragana = classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, raw)
        .and_then(|kind| ime_toggle_kind_to_shadow_action(kind, opt_in));
    let katakana = classify_mode_key_ime_action(ModeKeyCandidate::Katakana, raw)
        .and_then(|kind| ime_toggle_kind_to_shadow_action(kind, opt_in));
    (hiragana, katakana)
}

#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_mode_key_shadow_override_for_event(
    vk: VkCode,
    hiragana_override: Option<ShadowImeAction>,
    katakana_override: Option<ShadowImeAction>,
    thumb_pair: (VkCode, VkCode),
) -> Option<ShadowImeAction> {
    if vk == thumb_pair.0 || vk == thumb_pair.1 {
        return None;
    }
    if vk == ModeKeyCandidate::Hiragana.vk() {
        hiragana_override
    } else if vk == ModeKeyCandidate::Katakana.vk() {
        katakana_override
    } else {
        None
    }
}

/// この関数が見ていない「`resolve_pending_thumb_as_single`側でdelegateが
/// 実際には発火しない条件」は現在2つある: (1) 専用Fnキー設定
/// （`muhenkan_dedicated_fn_key_configured`引数で対処済み）、(2) ユーザーの
/// 単独タップ「パススルー」設定（BUG-119/ADR-147、`TurnOn`方向のみ辞退。
/// `TurnOn`分類は`crates/awase-gji-config/src/keymap.rs::classify_and_push`
/// の構造上、全状態でOFFにならないことが保証されるため配線不要——詳細は
/// ADR-147「消費点と所有権のマトリクス」参照）。3つ目の「黙って辞退する
/// 条件」を`resolve_pending_thumb_as_single`側に足す場合は、この関数側も
/// 対称に配線が必要かどうか（=belief ON中に実際に状態を反転させうる方向
/// かどうか）を必ず検討すること。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn delegate_owns_mode_key_shadow_toggle(
    vk: VkCode,
    is_configured_thumb_key: bool,
    hiragana_delegate: Option<ShadowImeAction>,
    katakana_delegate: Option<ShadowImeAction>,
    henkan_delegate: Option<ShadowImeAction>,
    muhenkan_delegate: Option<ShadowImeAction>,
    muhenkan_dedicated_fn_key_configured: bool,
) -> bool {
    is_configured_thumb_key
        && ((vk == ModeKeyCandidate::Hiragana.vk() && hiragana_delegate.is_some())
            || (vk == ModeKeyCandidate::Katakana.vk() && katakana_delegate.is_some())
            || (vk == ModeKeyCandidate::Henkan.vk() && henkan_delegate.is_some())
            // ADR-141実装レビュー(/code-review指摘): `muhenkan_solo_tap_
            // dedicated_fn_key`が設定済みだと、`resolve_pending_thumb_
            // as_single`の優先順位（専用Fnキー > delegate）でdelegateが
            // 実際には発火しない（BUG-115「専用Fnキーとの非対称」節）。
            // それを見ずにここが true を返すと、shadow-toggleが「delegate
            // が処理する」と誤信して身を引き、delegateも発火しないため
            // 「誰も何もしない」C2と同型の穴が専用Fnキー設定時に再発する。
            || (vk == ModeKeyCandidate::Muhenkan.vk()
                && muhenkan_delegate.is_some()
                && !muhenkan_dedicated_fn_key_configured))
}

/// [`resolve_mode_key_shadow_override_for_event`]の無変換/変換専用版
/// （ADR-141、C2対策）。Hiragana/Katakana版と異なり「親指キーならNone」
/// の早期returnを行わない——無変換/変換には`ImeKeyKind::from_vk`由来の
/// 守るべき静的`shadow_action`が存在しないため、親指キーとして設定
/// されている場合でもoverrideを差してよい（`&& effective_open()`
/// ゲートが`delegate_owns_mode_key_shadow_toggle`側で排他的に切り替える）。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_henkan_muhenkan_shadow_override_for_event(
    vk: VkCode,
    henkan_override: Option<ShadowImeAction>,
    muhenkan_override: Option<ShadowImeAction>,
) -> Option<ShadowImeAction> {
    if vk == ModeKeyCandidate::Henkan.vk() {
        henkan_override
    } else if vk == ModeKeyCandidate::Muhenkan.vk() {
        muhenkan_override
    } else {
        None
    }
}

/// ADR-188: GJIでは半角/全角のVK(0xF3/0xF4)はどちらも開閉トグル。
/// GJIがアクティブなときToggleを返す。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_hankaku_zenkaku_shadow_override_for_event(
    vk: VkCode,
    gji_active: bool,
) -> Option<ShadowImeAction> {
    if gji_active && (vk == crate::vk::VK_DBE_SBCSCHAR || vk == crate::vk::VK_DBE_DBCSCHAR) {
        Some(ShadowImeAction::Toggle)
    } else {
        None
    }
}

/// `left_thumb_key`/`right_thumb_key`のうちHiragana/Katakanaに一致する方の
/// VKを解決する（`NicolaFsm::set_hiragana_katakana_thumb_key_config`へ渡す
/// 値）。起動時（`app/bootstrap.rs`）とreload時
/// （`runtime/mod.rs::apply_config_update`）の両方から呼び、同じ導出
/// ロジックを2箇所で重複させない（/code-review指摘——`space_is_thumb_key`/
/// `muhenkan_dedicated_fn_key`が過去に同種のboot/reload重複から実際に
/// 乖離した前例があるため、共有ヘルパーへ揃える）。
#[must_use]
pub(crate) fn resolve_hiragana_katakana_thumb_vks(
    left: VkCode,
    right: VkCode,
) -> (Option<VkCode>, Option<VkCode>) {
    let hiragana_vk = [left, right]
        .into_iter()
        .find(|&vk| vk == crate::vk::VK_DBE_HIRAGANA);
    let katakana_vk = [left, right]
        .into_iter()
        .find(|&vk| vk == crate::vk::VK_DBE_KATAKANA);
    (hiragana_vk, katakana_vk)
}

/// ADR-176: `awase-settings`（別クレート）から直接呼べるよう`pub`で
/// 再エクスポートする（`build_confirmed_calibration_entry`のdoc参照）。
#[cfg(windows)]
pub use windows_impl::build_confirmed_calibration_entry;
#[cfg(windows)]
pub(crate) use windows_impl::{
    is_configured_thumb_key, read_config1_db, reset_streak_latch_for_reload,
    sync_gji_charset_autodetect,
};

#[cfg(windows)]
mod windows_impl {
    use awase::types::VkCode;

    use crate::runtime::Runtime;

    use super::{
        classify_mode_key_ime_action, classify_thumb_key_ime_actions, gate_thumb_key_ime_actions,
        ime_toggle_kind_to_shadow_action, resolve_gji_mode_key_shadow_overrides, ImeToggleKind,
        ModeKeyCandidate, ThumbKeyImeWarning,
    };

    /// `app/mod.rs::reload_config`から、GJI利用中の設定リロード時に呼ぶ
    /// （BUG-115 F4）。MS-IME側の`sync_ime_toggle_auto_detect`が設定
    /// リロードのたびに無条件で再読みするのと対称に、GJI側もラッチを
    /// リセットしてから`sync_gji_charset_autodetect`を呼び直すことで、
    /// `gji_thumb_key_ime_toggle`をユーザーが設定画面で変更した際に
    /// 次のGJIストリークまで（＝再起動するまで）反映されない、という
    /// stale化を防ぐ。ADR-091の「継続的ポーリングをしない」とは矛盾しない
    /// （reloadはユーザー起点の離散イベントであり、ポーリングではない）。
    pub(crate) fn reset_streak_latch_for_reload(app: &mut Runtime) {
        app.reset_gji_charset_streak_checked();
        sync_gji_charset_autodetect(app, true);
    }

    /// `runtime::message_handlers::sync_ime_kind_from_observation`から呼ぶ、
    /// GJI検出/離脱の唯一の合流点（`msime_key_assignment::check_and_warn`と対）。
    /// 無変換/変換キーのIME on/off/toggle意味論（BUG-115、ADR-179）の
    /// 自動判定を行う。
    ///
    /// - **GJI以外への遷移**: 直前がGJI継続区間だった場合のみ、自動検出して
    ///   いたshadow_action override・delegate-to-open-axisを解除する。
    /// - **GJIへの新規遷移**: `config1.db`を読み、無変換/変換キーの
    ///   割当てを`Engine`へ反映する。既にこのGJI継続区間でチェック済み
    ///   なら（ラッチが`GJI_CHECKED`のまま）何もしない——継続的な
    ///   ポーリングをしないため。
    pub(crate) fn sync_gji_charset_autodetect(app: &mut Runtime, is_gji: bool) {
        if !is_gji {
            if app.swap_gji_charset_streak_checked(false) {
                tracing::info!(
                    "[gji-charset-autodetect] GJI から離脱: 自動検出したIME関連キーを解除"
                );
                app.set_gji_mode_key_shadow_overrides(None, None);
                app.set_gji_mode_key_delegate_to_open_axis(None, None);
                // ADR-141: 無変換/変換のshadow_action overrideも同様に解除
                // する。忘れると、GJI由来のstale overrideが非GJI文脈へ
                // 無期限に残留する——直後の`set_gji_thumb_key_delegate_to_
                // open_axis(None, None)`の行が過去に抜けていて同種のバグを
                // 踏んだ経緯と同じ（Opus再レビュー Must-fix指摘）。
                app.set_thumb_key_shadow_overrides(None, None);
                // 無変換/変換側のGJI由来delegateも同様に解除する。従来は
                // 「GJI→非GJI遷移では必ずMS-IME側のsync_ime_toggle_auto_detect
                // が無条件で上書きするため冗長」としてここでは解除していな
                // かったが、それは`kind==MicrosoftIme`検出成功時にしか
                // 成立しない前提だった（/code-review指摘）。IME種別が
                // 未検出のままGJI以外へ遷移するケース（例: TSF/IME種別検出が
                // 失敗する非対応アプリへのフォーカス移動）ではMS-IME側の
                // 同期が走らず、GJI由来のstaleなdelegateが無期限に残留し、
                // 無関係なアプリでの単独タップがIME状態を静かに反転させる。
                app.set_gji_thumb_key_delegate_to_open_axis(None, None);
            }
            return;
        }
        if app.swap_gji_charset_streak_checked(true) {
            return;
        }

        let bytes = read_config1_db();
        let raw = bytes
            .as_deref()
            .and_then(awase_gji_config::wire::parse_top_level);
        let (hiragana_shadow_override, katakana_shadow_override) =
            resolve_gji_mode_key_shadow_overrides(
                true,
                raw.as_ref(),
                app.gji_thumb_key_ime_toggle_opt_in(),
            );
        app.set_gji_mode_key_shadow_overrides(hiragana_shadow_override, katakana_shadow_override);
        app.set_gji_mode_key_delegate_to_open_axis(
            hiragana_shadow_override,
            katakana_shadow_override,
        );
        warn_mode_key_thumb_key_unsupported_if_needed(app, raw.as_ref());

        // BUG-115（F2、must-fix）: 無変換/変換のIME意味論は、config1.dbが
        // 読めない場合も無条件で計算・反映する。理由はMozcのoverlay_keymaps/
        // session_keymap意味論のためだけでなく、MS-IME→GJI遷移時に
        // MS-IMEレジストリ由来の値（`sync_ime_toggle_auto_detect`が
        // セットしたもの）を上書きする**唯一の書き込み点**がここだから
        // （MS-IME側の同期は`kind == MicrosoftIme`でしか走らない、
        // `message_handlers.rs`参照）。
        let default_raw = awase_gji_config::wire::GjiRawConfig::default();
        let (henkan_kind, muhenkan_kind) =
            classify_thumb_key_ime_actions(raw.as_ref().unwrap_or(&default_raw));
        let mut wiring = gate_thumb_key_ime_actions(
            henkan_kind,
            muhenkan_kind,
            app.gji_thumb_key_ime_toggle_opt_in(),
        );
        // ADR-176決定5（176-T3）: 確定済み較正結果があれば、
        // gate_thumb_key_ime_actionsの出力そのものを差し替える。
        // mask_auto_detect_for_explicit_config等は変更せずそのまま効く。
        // 176-T12: 現在のconfig1.db内容に対してstaleな較正結果は
        // fresh_or_noneで「較正結果なし」に落とし、静的分類へ
        // フォールバックさせる。
        let raw_for_fingerprint = raw.as_ref().unwrap_or(&default_raw);
        wiring.henkan = crate::state::calibrated_mode_key::apply_calibration_override(
            wiring.henkan,
            crate::state::calibrated_mode_key::fresh_or_none(
                app.calibrated_mode_key_for(ModeKeyCandidate::Henkan.vk()),
                &ModeKeyCandidate::Henkan.current_fingerprint(raw_for_fingerprint),
            ),
        );
        wiring.muhenkan = crate::state::calibrated_mode_key::apply_calibration_override(
            wiring.muhenkan,
            crate::state::calibrated_mode_key::fresh_or_none(
                app.calibrated_mode_key_for(ModeKeyCandidate::Muhenkan.vk()),
                &ModeKeyCandidate::Muhenkan.current_fingerprint(raw_for_fingerprint),
            ),
        );
        warn_thumb_key_toggle_if_needed(app, wiring.warning, wiring.muhenkan);

        // ADR-179 決定1: 無変換/変換が親指シフトのチョードキーとして
        // 設定されているかどうかに関わらず、分類結果（On/Off/Toggle）は
        // 常に`ime_toggle_kind_to_shadow_action`経由で`henkan_shadow_
        // override`/`muhenkan_shadow_override`へ渡す。かつては非親指キー
        // 配置時だけ別の能動actuation経路（`ime_on_auto`/`ime_off_auto`
        // へのVec push）を使っていたが、これは無変換/変換向けに既に実装・
        // 検証済みのfollow-only経路（`shadow_action` override）を使わず
        // 別経路を再発明していただけだったと判明したため撤去した
        // （`ModeKeyActuationOwner`が実際の所有権判定を担う、
        // `runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle`参照）。
        let henkan_shadow = wiring
            .henkan
            .and_then(|a| ime_toggle_kind_to_shadow_action(a, true));
        let muhenkan_shadow = wiring
            .muhenkan
            .and_then(|a| ime_toggle_kind_to_shadow_action(a, true));
        // ADR-153 決定1 M15対策: ユーザーが明示config
        // （`henkan_solo_tap_ime_action`/`muhenkan_solo_tap_ime_action`）を
        // 設定しているキーについては、GJI自動検出由来のdelegate/
        // shadow_overrideの両方をarmedにしない——`kp_stage_shadow_ime_toggle`
        // のケース2/3（belief OFF側）とcase1（belief ON側、
        // `resolve_pending_thumb_as_single`）が明示configを直接扱うため、
        // 自動検出由来の値が同時にarmedのままだと、明示config対象キーの
        // 「@」対策（生キーを一切GJIに渡さない）が自動検出結果に依存して
        // しまう（本ADRの動機そのものを崩す）。書き込み点は2系統4箇所
        // （GJI側=ここ、MS-IME側=`message_handlers.rs::
        // sync_ime_toggle_auto_detect`）、両方に同じ無効化を適用する
        // （ADR-119の教訓「gateを1箇所に置いて満足しない」）。
        let henkan_shadow = crate::runtime::mask_auto_detect_for_explicit_config(
            henkan_shadow,
            app.henkan_solo_tap_ime_action(),
        );
        let muhenkan_shadow = crate::runtime::mask_auto_detect_for_explicit_config(
            muhenkan_shadow,
            app.muhenkan_solo_tap_ime_action(),
        );
        // `set_gji_thumb_key_delegate_to_open_axis`は非親指キー配置時にも
        // 同じ値を渡すが無害——`resolve_pending_thumb_as_single`自体が
        // `muhenkan_vk == Some(vk)`（＝親指キー設定）を要求するため、
        // 非親指キーのdelegate値はどのみち消費されない（ADR-179参照）。
        app.set_gji_thumb_key_delegate_to_open_axis(henkan_shadow, muhenkan_shadow);
        // ADR-141（C2対策）: delegateと同じ値をshadow_action overrideにも
        // 常時反映する。親指キーの場合、delegateとoverrideの両方に
        // 同じ値が登録され、`&& effective_open()`ゲート
        // （`mode_key_delegate_owns_shadow_toggle`）が実行時に排他的に
        // 切り替える——belief ON中はdelegateが、belief OFF中は
        // shadow-toggle（belief追随のみ）が処理する。上記M15対策の
        // 上書き後の値をそのまま使うため、明示config対象キーはここでも
        // Noneのまま。
        app.set_thumb_key_shadow_overrides(henkan_shadow, muhenkan_shadow);

        // BUG-115 Phase 2/3: Hiragana/Katakana は actuation-auto には載せない。
        // 非親指キーでは Runtime::enrich_ime_relevance の shadow_action
        // override、親指キーでは resolve_pending_thumb_as_single の
        // delegate-to-open-axis が担当する。
    }

    /// `vk`が現在`left_thumb_key`/`right_thumb_key`のいずれかに設定されて
    /// いるか（BUG-115）。`crate::hook::thumb_vk_codes()`
    /// （`apply_config_update`/起動時に更新される、常に最新の親指キー
    /// ペア）と比較する汎用ヘルパー——無変換/変換に限らず任意のVKに使える。
    pub(crate) fn is_configured_thumb_key(vk: VkCode) -> bool {
        let (left, right) = crate::hook::thumb_vk_codes();
        vk == left || vk == right
    }

    /// BUG-115（N8）: 無変換/変換のIME意味論判定結果をユーザーへ通知する。
    /// 同一内容の警告はプロセス内で一度だけ
    /// （`msime_key_assignment::check_and_warn`と同型のデデュープ）。
    fn warn_thumb_key_toggle_if_needed(
        app: &mut Runtime,
        warning: ThumbKeyImeWarning,
        muhenkan: Option<ImeToggleKind>,
    ) {
        if app.swap_gji_toggle_warning(warning) == Some(warning) {
            return; // 同じ内容で通知済み
        }
        match warning {
            ThumbKeyImeWarning::None => {}
            ThumbKeyImeWarning::ToggleDeclined => {
                tracing::warn!(
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
                tracing::info!(
                    "[gji-charset-autodetect] gji_thumb_key_ime_toggle=true \
                     設定により、無変換/変換キーの状態依存トグルをベストエフォートで \
                     反映しました（docs/known-bugs.md BUG-115参照）。"
                );
            }
        }
        if is_configured_thumb_key(ModeKeyCandidate::Muhenkan.vk())
            && app.muhenkan_dedicated_fn_key_configured()
            && muhenkan.is_some()
        {
            // F5: 専用Fnキー（muhenkan_solo_tap_dedicated_fn_key）が優先
            // されるため、無変換が親指キーの場合、そのdelegate-to-open-axis
            // は黙って無効化される（`resolve_pending_thumb_as_single`の
            // 優先順位、henkan側には専用Fnキーの概念自体が無い非対称）。
            // 無変換が親指キーでない場合はactuation-auto経由になり
            // dedicated_fn_keyとは無関係なので、この警告は不要。
            // ゲート条件は`warning != None`（Toggle検出時のみ）ではなく
            // `muhenkan.is_some()`（On/Off/Toggleいずれでもマスクされる）
            // を使う——このマスキングはToggle分類とは無関係に、
            // `wiring.muhenkan`がSomeでありさえすれば発生するため。
            // ユーザーがToggleDeclined案内に従いキーマップをOn/Off限定に
            // 直しても`warning`はNoneに戻るが、マスキング自体は解消しない
            // （/code-review指摘）。
            tracing::warn!(
                "[gji-charset-autodetect] muhenkan_solo_tap_dedicated_fn_keyが設定済みの \
                 ため、無変換キーのIME open軸への追従は無効化されます（変換キー側のみ \
                 有効）。詳細はdocs/known-bugs.md BUG-115参照。"
            );
        }
    }

    fn warn_mode_key_thumb_key_unsupported_if_needed(
        app: &mut Runtime,
        raw: Option<&awase_gji_config::wire::GjiRawConfig>,
    ) {
        let Some(raw) = raw else {
            app.reset_gji_mode_key_thumb_warning_declined();
            return;
        };
        let declined = [ModeKeyCandidate::Hiragana, ModeKeyCandidate::Katakana]
            .into_iter()
            .any(|key| {
                is_configured_thumb_key(key.vk())
                    && matches!(
                        classify_mode_key_ime_action(key, raw),
                        Some(ImeToggleKind::Toggle)
                    )
                    && !app.gji_thumb_key_ime_toggle_opt_in()
            });
        if !declined {
            app.reset_gji_mode_key_thumb_warning_declined();
            return;
        }
        if app.swap_gji_mode_key_thumb_warning_declined(true) {
            return;
        }
        tracing::warn!(
            "[gji-charset-autodetect] GJIの設定が親指キーとして設定された \
             Hiragana/Katakana に状態依存のIME ON/OFFトグルを割り当てていますが、\
             gji_thumb_key_ime_toggle=false のため自動反映しません。挙動を理解した \
             上で必要なら config.toml で gji_thumb_key_ime_toggle=true を設定してください \
             （docs/known-bugs.md BUG-115参照）。"
        );
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
    /// GJI未インストール環境を正常系として扱う）。ADR-148（bug report）が
    /// `sync_gji_charset_autodetect`とは独立に、報告生成時点の内容を
    /// 都度読み直すためにも使う（Runtime側にキャッシュされた
    /// `GjiRawConfig`は存在しないため）。
    pub(crate) fn read_config1_db() -> Option<Vec<u8>> {
        let path = config1_db_path()?;
        std::fs::read(&path).ok()
    }

    /// ADR-176（T9a確定結果のconfig.toml永続化、最終配線）:
    /// `awase-settings`が`WM_CALIBRATION_RESULT`（`ConfirmedOn`）を受けて
    /// config.tomlへ書き込む際に呼ぶ。
    ///
    /// awase-settings自身はGJI/レジストリ読み取りロジックを持たないため、
    /// この関数（`awase-windows`クレート内、awase-settingsからも呼べる
    /// `pub`関数）が代わりに`config1.db`/レジストリを読み直して
    /// フィンガープリントを構築する——結果が確定した直後に呼ばれる想定
    /// のため、確定に使われた値と同じ内容が読めるはずである。
    /// `VK_NONCONVERT`/`VK_CONVERT`以外（`apply_calibration_override`が
    /// 消費するのはこの2キーのみ）、またはGJI選択時に`config1.db`が
    /// 読めない場合は`None`（呼び出し元は保存をスキップし警告すること）。
    #[must_use]
    pub fn build_confirmed_calibration_entry(
        vk: VkCode,
        active_ime_kind: crate::state::ime_kind::ImeKindId,
    ) -> Option<awase::config::CalibrationEntry> {
        use crate::state::ime_kind::ImeKindId;

        let candidate = if vk == crate::vk::VK_CONVERT {
            ModeKeyCandidate::Henkan
        } else if vk == crate::vk::VK_NONCONVERT {
            ModeKeyCandidate::Muhenkan
        } else {
            return None;
        };
        let config_fingerprint = match active_ime_kind {
            ImeKindId::Gji => {
                let bytes = read_config1_db()?;
                let raw = awase_gji_config::wire::parse_top_level(&bytes)?;
                candidate.current_fingerprint(&raw)
            }
            ImeKindId::MsIme => crate::state::calibrated_mode_key::ConfigFingerprint::MsIme {
                registry_value_hash: crate::msime_key_assignment::current_registry_fingerprint_hash(
                    vk,
                ),
            },
        };
        let confirmed_at_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        Some(
            crate::state::calibrated_mode_key::CalibratedModeKey {
                vk,
                result: ImeToggleKind::On,
                active_ime_kind,
                config_fingerprint,
                confirmed_at_epoch_ms,
            }
            .to_config_entry(),
        )
    }
}

#[cfg(test)]
mod tests {
    // ── classify_thumb_key_ime_actions / gate_thumb_key_ime_actions (BUG-115) ──

    use super::{
        classify_mode_key_ime_action, classify_thumb_key_ime_actions,
        delegate_owns_mode_key_shadow_toggle, gate_thumb_key_ime_actions,
        ime_toggle_kind_to_shadow_action, resolve_gji_mode_key_shadow_overrides,
        resolve_hankaku_zenkaku_shadow_override_for_event,
        resolve_henkan_muhenkan_shadow_override_for_event,
        resolve_mode_key_shadow_override_for_event, ImeToggleKind, ModeKeyCandidate,
        ThumbKeyImeWarning,
    };
    use awase::types::{ShadowImeAction, VkCode};
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

    /// ADR-174実機検証（2026-09-15、dragonflyg4）: `session_keymap`が
    /// `MSIME`（実機で値2を確認）のままでも、`custom_keymap_table`に
    /// `DirectInput\tHenkan\tIMEOn`という実際のユーザー上書きが残って
    /// いれば、プリセット静的知識（Henkan/Muhenkanは`None`）より
    /// 優先されるべき。旧実装ではこの場合`custom_keymap_table`を一切
    /// 参照せず`None`を返し、GJIが実際にIMEを開いてもawaseのbeliefが
    /// 追従しなかった（ユーザー報告、実機ログで`[shadow-toggle]`行が
    /// 一切出力されないことを確認済み）。実機の`config1.db`はこれ以外にも
    /// `Composition Henkan CompositionModeHiragana`等の行やF15-F19の
    /// SetMode割り当てを含む（本テストはHenkan/Muhenkan分類に関係する
    /// 部分のみ再現）。
    #[test]
    fn classify_msime_session_keymap_with_populated_custom_table_prefers_table() {
        let table = "status\tkey\tcommand\n\
            DirectInput\tHenkan\tIMEOn\n\
            Composition\tHenkan\tCompositionModeHiragana\n";
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_MSIME),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Henkan, &raw),
            Some(ImeToggleKind::On)
        );
        // Muhenkanはテーブルに該当行が無いため、プリセット静的知識
        // （MSIMEはMuhenkanに割り当てなし）へフォールスルーする。
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Muhenkan, &raw),
            None
        );
        // Hiragana/Katakanaはテーブルに該当行が無いため、MSIMEプリセット
        // 静的知識（`On`）へフォールスルーする——テーブルの存在が
        // 無関係なキーの判定を壊さないことの固定。
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Hiragana, &raw),
            Some(ImeToggleKind::On)
        );
    }

    /// ADR-186(実機スパイク、2026-09-20): ATOKプリセットでは、`config1.db`に残る古い
    /// `custom_keymap_table`(実機に実在した`DirectInput\tHenkan\tIMEOn`等)を読まない。
    /// 変換・無変換ともATOKの`Toggle`(開閉トグル)になる。MSIME(ADR-174)は表を優先するまま。
    #[test]
    fn classify_atok_session_keymap_ignores_stale_custom_table() {
        let table = "status\tkey\tcommand\n\
            DirectInput\tHenkan\tIMEOn\n\
            Precomposition\tHenkan\tCompositionModeHiragana\n\
            Composition\tHenkan\tCompositionModeHiragana\n";
        let atok = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_ATOK),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Henkan, &atok),
            Some(ImeToggleKind::Toggle)
        );
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Muhenkan, &atok),
            Some(ImeToggleKind::Toggle)
        );
        // 同じ表でもMSIMEでは従来どおり表を優先する（ADR-174の回帰防止）。
        let msime = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_MSIME),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(
            classify_mode_key_ime_action(ModeKeyCandidate::Henkan, &msime),
            Some(ImeToggleKind::On)
        );
    }

    /// `custom_keymap_table`が存在してもHenkan/Muhenkanに該当する行が
    /// 無ければ、MSIMEプリセットの静的知識（`None`）へフォールスルーする
    /// （2.5節が3節を上書きしないことの固定）。
    #[test]
    fn classify_msime_session_keymap_with_table_lacking_henkan_falls_back_to_preset() {
        let table = "status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n";
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_MSIME),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(classify_thumb_key_ime_actions(&raw), (None, None));
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

    #[test]
    fn mode_key_shadow_override_skips_when_raw_is_unavailable() {
        assert_eq!(
            resolve_gji_mode_key_shadow_overrides(true, None, true),
            (None, None)
        );
    }

    #[test]
    fn mode_key_shadow_override_skips_when_classifier_returns_none() {
        let raw = raw_with_session_keymap(awase_gji_config::SESSION_KEYMAP_ATOK);
        assert_eq!(
            resolve_gji_mode_key_shadow_overrides(true, Some(&raw), true),
            (None, None)
        );
    }

    #[test]
    fn mode_key_shadow_override_gates_toggle_by_opt_in() {
        let table = "status\tkey\tcommand\nDirectInput\tHiragana\tIMEOn\nPrecomposition\tHiragana\tIMEOff\n";
        let raw = GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_CUSTOM),
            custom_keymap_table: Some(table.to_string()),
            ..GjiRawConfig::default()
        };
        assert_eq!(
            resolve_gji_mode_key_shadow_overrides(true, Some(&raw), false),
            (None, None)
        );
        assert_eq!(
            resolve_gji_mode_key_shadow_overrides(true, Some(&raw), true),
            (Some(ShadowImeAction::Toggle), None)
        );
    }

    #[test]
    fn mode_key_shadow_override_clears_when_not_gji() {
        let raw = raw_with_session_keymap(awase_gji_config::SESSION_KEYMAP_MSIME);
        assert_eq!(
            resolve_gji_mode_key_shadow_overrides(false, Some(&raw), true),
            (None, None)
        );
    }

    #[test]
    fn mode_key_shadow_override_applies_only_to_non_thumb_keys() {
        let hiragana = ModeKeyCandidate::Hiragana.vk();
        let katakana = ModeKeyCandidate::Katakana.vk();
        let other_left = VkCode(0x41);
        let other_right = VkCode(0x42);
        assert_eq!(
            resolve_mode_key_shadow_override_for_event(
                hiragana,
                Some(ShadowImeAction::TurnOff),
                Some(ShadowImeAction::Toggle),
                (other_left, other_right),
            ),
            Some(ShadowImeAction::TurnOff)
        );
        assert_eq!(
            resolve_mode_key_shadow_override_for_event(
                hiragana,
                Some(ShadowImeAction::TurnOff),
                Some(ShadowImeAction::Toggle),
                (hiragana, other_right),
            ),
            None
        );
        assert_eq!(
            resolve_mode_key_shadow_override_for_event(
                katakana,
                None,
                Some(ShadowImeAction::Toggle),
                (other_left, other_right),
            ),
            Some(ShadowImeAction::Toggle)
        );
    }

    #[test]
    fn hankaku_zenkaku_shadow_override_toggles_only_for_gji_dbe_width_keys() {
        assert_eq!(
            resolve_hankaku_zenkaku_shadow_override_for_event(crate::vk::VK_DBE_SBCSCHAR, true),
            Some(ShadowImeAction::Toggle)
        );
        assert_eq!(
            resolve_hankaku_zenkaku_shadow_override_for_event(crate::vk::VK_DBE_DBCSCHAR, true),
            Some(ShadowImeAction::Toggle)
        );
        assert_eq!(
            resolve_hankaku_zenkaku_shadow_override_for_event(crate::vk::VK_DBE_SBCSCHAR, false),
            None
        );
        for vk in [
            crate::vk::VK_KANJI,
            crate::vk::VK_DBE_HIRAGANA,
            ModeKeyCandidate::Henkan.vk(),
        ] {
            assert_eq!(
                resolve_hankaku_zenkaku_shadow_override_for_event(vk, true),
                None,
                "{vk:?}はADR-188の半角/全角override対象外"
            );
        }
    }

    #[test]
    fn mode_key_delegate_ownership_is_thumb_key_times_delegate_armed() {
        let hiragana = ModeKeyCandidate::Hiragana.vk();
        assert!(!delegate_owns_mode_key_shadow_toggle(
            hiragana,
            false,
            Some(ShadowImeAction::TurnOn),
            None,
            None,
            None,
            false,
        ));
        assert!(!delegate_owns_mode_key_shadow_toggle(
            hiragana, true, None, None, None, None, false,
        ));
        assert!(delegate_owns_mode_key_shadow_toggle(
            hiragana,
            true,
            Some(ShadowImeAction::TurnOn),
            None,
            None,
            None,
            false,
        ));
        // ADR-141: Henkan/Muhenkanも同じ関数で判定される。
        let henkan = ModeKeyCandidate::Henkan.vk();
        assert!(!delegate_owns_mode_key_shadow_toggle(
            henkan,
            false,
            None,
            None,
            Some(ShadowImeAction::TurnOn),
            None,
            false,
        ));
        assert!(delegate_owns_mode_key_shadow_toggle(
            henkan,
            true,
            None,
            None,
            Some(ShadowImeAction::TurnOn),
            None,
            false,
        ));
    }

    /// /code-review指摘（実装レビューで発見）: `muhenkan_solo_tap_dedicated_
    /// fn_key`が設定済みだと`resolve_pending_thumb_as_single`の優先順位で
    /// delegateが実際には発火しない（専用Fnキーが勝つ）ため、delegateが
    /// armedでもownershipはfalseを返すべき（さもないとshadow-toggleが
    /// 「delegateが処理する」と誤信して身を引き、どちらも処理しない
    /// C2型の穴が再発する）。Henkanには専用Fnキーの概念自体が無いため
    /// 対象外（既存の非対称、BUG-115「専用Fnキーとの非対称」節）。
    #[test]
    fn muhenkan_dedicated_fn_key_configured_blocks_delegate_ownership() {
        let muhenkan = ModeKeyCandidate::Muhenkan.vk();
        assert!(delegate_owns_mode_key_shadow_toggle(
            muhenkan,
            true,
            None,
            None,
            None,
            Some(ShadowImeAction::TurnOff),
            false,
        ));
        assert!(!delegate_owns_mode_key_shadow_toggle(
            muhenkan,
            true,
            None,
            None,
            None,
            Some(ShadowImeAction::TurnOff),
            true,
        ));
    }

    // ── GJI検出→反映の全体パイプライン decision table（ユーザー依頼、
    // 2026-09-05。ADR-179決定1でHenkan/Muhenkanのactuation-auto撤去に
    // 伴い2026-09-18更新）──
    //
    // Phase 1〜3を通じて何度も見落とされてきた「Toggleだけopt-inで
    // ゲートする」という規則を、4キー（Henkan/Muhenkan/Hiragana/
    // Katakana）×GJI判定値（None/On/Off/Toggle）×opt_in×is_thumbの
    // 全組み合わせ（4×4×2×2=64通り）に対して、実際の本番用純粋関数
    // （`classify_mode_key_ime_action`/`classify_thumb_key_ime_actions`/
    // `gate_thumb_key_ime_actions`/`ime_toggle_kind_to_shadow_action`/
    // `resolve_mode_key_shadow_override_for_event`/
    // `delegate_owns_mode_key_shadow_toggle`）を呼び出して検証する。
    //
    // ADR-179決定1により、Henkan/Muhenkanは「非親指キーはactuation-auto、
    // 親指キーはdelegate-to-open-axis」という振り分けを撤去し、
    // is_thumbに関わらず常にdelegate-to-open-axisとshadow_action
    // overrideの両方に同じ値を反映するようになった（実際にどちらが
    // 発火するかは`ModeKeyActuationOwner`が打鍵時に判定する、
    // `runtime/key_pipeline.rs`参照——本decision tableは
    // `sync_gji_charset_autodetect`が計算する値までを検証対象とする）。
    //
    // 64通りは全数（exhaustive）であり、その部分集合として任意の2軸間の
    // 全組（pairwise）も自動的に網羅される——4値軸(Key)×4値軸(Classify)
    // だけで16通りの「全組」が必要になるため、2値軸(opt_in/is_thumb)を
    // 含めても全数を取るのが最も単純かつ取りこぼしが無い。

    /// 4キーそれぞれが対応するMozcキートークン（CUSTOM keymap table用）。
    fn mozc_token(key: ModeKeyCandidate) -> &'static str {
        match key {
            ModeKeyCandidate::Henkan => "Henkan",
            ModeKeyCandidate::Muhenkan => "Muhenkan",
            ModeKeyCandidate::Hiragana => "Hiragana",
            ModeKeyCandidate::Katakana => "Katakana",
        }
    }

    /// 指定したキーに対して`classify_mode_key_ime_action`が指定の判定値を
    /// 返すような`GjiRawConfig`（CUSTOM keymap table方式）を構築する。
    /// `awase-gji-config::keymap::group_ime_rows_by_key`の分類規則
    /// （on_statuses/off_statusesの非空・空の組み合わせのみで決まり、
    /// 具体的な状態名自体は問わない）に基づく最小構成。
    fn custom_raw_yielding(key: ModeKeyCandidate, desired: Option<ImeToggleKind>) -> GjiRawConfig {
        let tok = mozc_token(key);
        let table = match desired {
            None => "status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n".to_string(),
            Some(ImeToggleKind::On) => format!("status\tkey\tcommand\nDirectInput\t{tok}\tIMEOn\n"),
            Some(ImeToggleKind::Off) => {
                format!("status\tkey\tcommand\nPrecomposition\t{tok}\tIMEOff\n")
            }
            Some(ImeToggleKind::Toggle) => format!(
                "status\tkey\tcommand\nDirectInput\t{tok}\tIMEOn\nPrecomposition\t{tok}\tIMEOff\n"
            ),
        };
        GjiRawConfig {
            session_keymap: Some(awase_gji_config::SESSION_KEYMAP_CUSTOM),
            custom_keymap_table: Some(table),
            ..GjiRawConfig::default()
        }
    }

    /// パイプラインの最終的な帰結。4キー共通の型で表す
    /// （ADR-179決定1により、Henkan/Muhenkanはis_thumbに関わらず常に
    /// `DelegateAndShadowOverride`、Hiragana/Katakanaは非親指キー側だけ
    /// `ShadowOverride`——キー族によって消費先の機構自体が異なるため
    /// 区別する）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PipelineOutcome {
        /// GJI判定がNone、またはToggleでopt-in未設定のため何も反映されない。
        Nothing,
        /// 親指キー（Hiragana/Katakana）: delegate-to-open-axisへ反映される
        /// （単独タップ確定時にこの値でIME open軸を操作する）。静的
        /// `shadow_action`を守るため、shadow-toggle側のoverrideは適用
        /// されない。
        Delegate(ShadowImeAction),
        /// 親指キーのHenkan/Muhenkan（ADR-141）: delegate-to-open-axisと
        /// shadow_action overrideの**両方**に同じ値が反映される。
        /// `&& effective_open()`ゲートが実行時に排他的に切り替える
        /// （belief ON中はdelegate、belief OFF中はshadow-toggle）ため、
        /// 静的`shadow_action`を持たないHenkan/MuhenkanはHiragana/
        /// Katakanaと異なりこの二重登録が必要（C2対策）。
        DelegateAndShadowOverride(ShadowImeAction),
        /// 非親指キーのHiragana/Katakana: 静的`shadow_action`を
        /// オーバーライドする。
        ShadowOverride(ShadowImeAction),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum KeyFamily {
        HenkanMuhenkan,
        HiraganaKatakana,
    }

    fn key_family(key: ModeKeyCandidate) -> KeyFamily {
        match key {
            ModeKeyCandidate::Henkan | ModeKeyCandidate::Muhenkan => KeyFamily::HenkanMuhenkan,
            ModeKeyCandidate::Hiragana | ModeKeyCandidate::Katakana => KeyFamily::HiraganaKatakana,
        }
    }

    /// **仕様（この関数自体が decision table の正解列）**: GJI判定値・opt_in・
    /// is_thumbから期待される帰結を、本番実装から独立して導出する。
    /// 本番の`gate_thumb_key_ime_actions`/`ime_toggle_kind_to_shadow_action`
    /// と**同じ規則を意図的に再実装**している——ここが本番コードの
    /// コピーになってしまうと「実装のバグをテストごと固定する」だけに
    /// なるため、規則を素直に書き下す（Toggleのみopt-inでゲート、
    /// 親指キーならdelegate、非親指キーならキー族ごとの機構へ）。
    fn expected_outcome(
        classify: Option<ImeToggleKind>,
        opt_in: bool,
        is_thumb: bool,
        family: KeyFamily,
    ) -> PipelineOutcome {
        let Some(kind) = classify else {
            return PipelineOutcome::Nothing;
        };
        let is_toggle = matches!(kind, ImeToggleKind::Toggle);
        if is_toggle && !opt_in {
            return PipelineOutcome::Nothing;
        }
        let action = match kind {
            ImeToggleKind::On => ShadowImeAction::TurnOn,
            ImeToggleKind::Off => ShadowImeAction::TurnOff,
            ImeToggleKind::Toggle => ShadowImeAction::Toggle,
        };
        match family {
            // ADR-179決定1: Henkan/Muhenkanはis_thumbに関わらず常にdelegateと
            // shadow_action overrideの両方に同じ値が登録される
            // （`&& effective_open()`ゲートが実行時に排他的に切り替える）。
            KeyFamily::HenkanMuhenkan => PipelineOutcome::DelegateAndShadowOverride(action),
            KeyFamily::HiraganaKatakana => {
                // Hiragana/Katakanaは静的shadow_actionを守るため、親指キー
                // 側はdelegateのみ（overrideは「親指キーならNone」で適用
                // されない、Opus再レビューMust-fix #2で明確化）。この軸は
                // ADR-179のスコープ外で変更していない。
                if is_thumb {
                    PipelineOutcome::Delegate(action)
                } else {
                    PipelineOutcome::ShadowOverride(action)
                }
            }
        }
    }

    /// Henkan/Muhenkan側の実際のパイプラインを本番関数だけで辿って
    /// 帰結を得る（`classify_thumb_key_ime_actions` →
    /// `gate_thumb_key_ime_actions` → `ime_toggle_kind_to_shadow_action`、
    /// ADR-179決定1）。`is_thumb`はもはや`sync_gji_charset_autodetect`の
    /// 計算に影響しない（打鍵時の`ModeKeyActuationOwner`判定でのみ使う）
    /// ため引数から外した——呼び出し元のdecision tableループは両方の
    /// is_thumb値で同じ`expected`と突き合わせることで、この軸が
    /// 無関係になったこと自体を検証する。
    fn actual_outcome_henkan_muhenkan(
        target: ModeKeyCandidate,
        raw: &GjiRawConfig,
        opt_in: bool,
    ) -> PipelineOutcome {
        let (henkan_c, muhenkan_c) = classify_thumb_key_ime_actions(raw);
        let wiring = gate_thumb_key_ime_actions(henkan_c, muhenkan_c, opt_in);
        let gated = match target {
            ModeKeyCandidate::Henkan => wiring.henkan,
            ModeKeyCandidate::Muhenkan => wiring.muhenkan,
            _ => unreachable!("this helper is Henkan/Muhenkan専用"),
        };
        let Some(action) = gated.and_then(|k| ime_toggle_kind_to_shadow_action(k, true)) else {
            return PipelineOutcome::Nothing;
        };
        // ADR-141: 本番の`sync_gji_charset_autodetect`はdelegateと
        // 同じ値を`set_thumb_key_shadow_overrides`にも渡す
        // （`henkan_shadow`/`muhenkan_shadow`をそのまま再利用）。
        // ここでは対象キー自身のoverrideスロットにのみ値を入れて
        // `resolve_henkan_muhenkan_shadow_override_for_event`を呼び、
        // 本番配線を再現する（他方のキーの値は本ヘルパーの対象外なので
        // Noneのままでよい——vk一致判定にしか影響しない）。
        let (henkan_override, muhenkan_override) = match target {
            ModeKeyCandidate::Henkan => (Some(action), None),
            ModeKeyCandidate::Muhenkan => (None, Some(action)),
            _ => unreachable!("this helper is Henkan/Muhenkan専用"),
        };
        let shadow_override = resolve_henkan_muhenkan_shadow_override_for_event(
            target.vk(),
            henkan_override,
            muhenkan_override,
        );
        assert_eq!(
            shadow_override,
            Some(action),
            "delegateがSomeならshadow_action overrideも同じ値でSomeになるはず \
             （両方に登録する設計、ADR-141）"
        );
        PipelineOutcome::DelegateAndShadowOverride(action)
    }

    /// Hiragana/Katakana側の実際のパイプラインを本番関数だけで辿って
    /// 帰結を得る（`classify_mode_key_ime_action` →
    /// `ime_toggle_kind_to_shadow_action` →
    /// `resolve_mode_key_shadow_override_for_event`/
    /// `delegate_owns_mode_key_shadow_toggle`）。C1で修正した
    /// 「delegate armed単独ではなく親指キー×armedの積」という不変条件を
    /// `delegate_owns_mode_key_shadow_toggle`経由で検証する
    /// （belief ON条件は`Runtime`層にありここでは検証対象外、ADR-135
    /// 「実装レビューで発覚したC1」節参照）。
    fn actual_outcome_hiragana_katakana(
        target: ModeKeyCandidate,
        raw: &GjiRawConfig,
        opt_in: bool,
        is_thumb: bool,
    ) -> PipelineOutcome {
        let classify = classify_mode_key_ime_action(target, raw);
        let armed = classify.and_then(|k| ime_toggle_kind_to_shadow_action(k, opt_in));
        let vk = target.vk();
        // vkと一致しない番兵VK。is_thumbがtrueのときだけ対象キーを
        // 親指ペアに含める。
        let sentinel = VkCode(0xEE);
        let thumb_pair = if is_thumb {
            (vk, sentinel)
        } else {
            (sentinel, sentinel)
        };
        let (hiragana_armed, katakana_armed) = match target {
            ModeKeyCandidate::Hiragana => (armed, None),
            ModeKeyCandidate::Katakana => (None, armed),
            _ => unreachable!("this helper is Hiragana/Katakana専用"),
        };
        if delegate_owns_mode_key_shadow_toggle(
            vk,
            is_thumb,
            hiragana_armed,
            katakana_armed,
            None,
            None,
            false,
        ) {
            return PipelineOutcome::Delegate(
                armed.expect("armedのはず(delegate_owns==trueの前提)"),
            );
        }
        if let Some(action) = resolve_mode_key_shadow_override_for_event(
            vk,
            hiragana_armed,
            katakana_armed,
            thumb_pair,
        ) {
            return PipelineOutcome::ShadowOverride(action);
        }
        PipelineOutcome::Nothing
    }

    #[test]
    fn gji_detection_to_application_pipeline_decision_table() {
        const KEYS: [ModeKeyCandidate; 4] = [
            ModeKeyCandidate::Henkan,
            ModeKeyCandidate::Muhenkan,
            ModeKeyCandidate::Hiragana,
            ModeKeyCandidate::Katakana,
        ];
        const CLASSIFY_VALUES: [Option<ImeToggleKind>; 4] = [
            None,
            Some(ImeToggleKind::On),
            Some(ImeToggleKind::Off),
            Some(ImeToggleKind::Toggle),
        ];
        const BOOLS: [bool; 2] = [false, true];

        let mut checked = 0usize;
        for key in KEYS {
            for classify in CLASSIFY_VALUES {
                // classify_mode_key_ime_action自体が意図どおりの値を
                // 返すことを自己点検する（raw構築ヘルパーの正しさの検証、
                // decision table本体のノイズにならないよう別途assertする）。
                let raw = custom_raw_yielding(key, classify);
                assert_eq!(
                    classify_mode_key_ime_action(key, &raw),
                    classify,
                    "custom_raw_yieldingの構築自体が壊れている: key={key:?} desired={classify:?}"
                );

                for opt_in in BOOLS {
                    for is_thumb in BOOLS {
                        let family = key_family(key);
                        let expected = expected_outcome(classify, opt_in, is_thumb, family);
                        let actual = match family {
                            KeyFamily::HenkanMuhenkan => {
                                actual_outcome_henkan_muhenkan(key, &raw, opt_in)
                            }
                            KeyFamily::HiraganaKatakana => {
                                actual_outcome_hiragana_katakana(key, &raw, opt_in, is_thumb)
                            }
                        };
                        assert_eq!(
                            actual, expected,
                            "key={key:?} classify={classify:?} opt_in={opt_in} \
                             is_thumb={is_thumb}: 期待={expected:?} 実際={actual:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(
            checked,
            KEYS.len() * CLASSIFY_VALUES.len() * BOOLS.len() * BOOLS.len(),
            "4x4x2x2=64通りを全数網羅したことの自己点検"
        );
    }
}
