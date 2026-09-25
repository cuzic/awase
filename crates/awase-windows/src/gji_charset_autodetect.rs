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
//! - **継続的なポーリングはしない**（ADR-091決定3項目2）。呼び出し側（較正結果の保存、
//!   bug report〈ADR-148〉）が必要なときに1回だけ読む。ADR-191でGJI検出時の自動同期
//!   （`sync_gji_charset_autodetect`、ラッチ付き）は撤去した。
//! - **config1.db未存在（GJI未インストール等）はエラーではない**。読めなければ
//!   静かに何もしない。`awase-gji-config`crate自体の「パース失敗は常に
//!   空の結果に静かにフォールバック」という既存方針を踏襲する。

/// GJIが無変換/変換キーに割り当てているIME意味論の分類（BUG-115）。
/// `session_keymap`/`custom_keymap_table`/`overlay_keymaps`のどれ由来でも
/// 同じ3値に潰す。この分類は較正結果の保存とbug report（ADR-148）の診断表示にだけ使う
/// （ADR-191: awaseがこの結果からIMEの開閉を代行・上書きすることはない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImeToggleKind {
    /// このキー単独でIMEをONにする。
    On,
    /// このキー単独でIMEをOFFにする。
    Off,
    /// 現在のIME開閉状態に応じて反転する（`ctx.ime_on`依存、非冪等）。
    Toggle,
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
/// ADR-191: この分類は、bug report（ADR-148）の診断表示と較正結果の保存にだけ使う。awaseが
/// この結果からIMEの開閉を代行・上書きすることはない（`gji_thumb_key_ime_toggle`設定と採用機構は撤去済み）。
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
/// 無変換(`Muhenkan`)・変換(`Henkan`)だけを扱う（`ImeKeyKind::from_vk`に含まれないキー）。
/// ADR-191で、この分類結果を awase が自動採用する機構（`*_delegate_to_open_axis`、
/// shadow_action override）は撤去した。分類は bug report の診断（[`classify_mode_key_ime_action`]）に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ModeKeyCandidate {
    Henkan,
    Muhenkan,
}

impl ModeKeyCandidate {
    /// `awase::types::VkCode::from_name`が受理するVK名。
    const fn vk_name(self) -> &'static str {
        match self {
            Self::Henkan => "VK_CONVERT",
            Self::Muhenkan => "VK_NONCONVERT",
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
/// ADR-191: この分類は、bug report（ADR-148）の診断表示と較正結果の保存にだけ使う。awaseが
/// この結果からIMEの開閉を代行・上書きすることはない（`gji_thumb_key_ime_toggle`設定と採用機構は撤去済み）。
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
        Some(v) if v == awase_gji_config::SESSION_KEYMAP_ATOK => Some(ImeToggleKind::Toggle),
        // MSIME/MOBILE/フィールド不在(実質MSIME相当)/KOTOERI/CHROMEOS/未知の値: 無変換/変換は静的には決めない。
        Some(_) | None => None,
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

/// ADR196-T2「1e前半」: 学習プロセス（`awase-keymap-learn-win`、別クレート）が
/// 開始時・終了時の`config1.db`比較（opus-adversarial-consult 2026-09-23 B-3）と
/// 既知構成判定に直接呼べるよう`pub`で再エクスポートする。
#[cfg(windows)]
pub use windows_impl::{bundled_preset_for_adjudication, read_config1_db, BundledPresetLookup};
#[cfg(windows)]
pub(crate) use windows_impl::{config1_db_stamp, is_configured_thumb_key, read_key_effect_keymap};

#[cfg(windows)]
mod windows_impl {
    use awase::types::VkCode;

    /// `vk`が現在`left_thumb_key`/`right_thumb_key`のいずれかに設定されて
    /// いるか（BUG-115）。`crate::hook::thumb_vk_codes()`
    /// （`apply_config_update`/起動時に更新される、常に最新の親指キー
    /// ペア）と比較する汎用ヘルパー——無変換/変換に限らず任意のVKに使える。
    pub(crate) fn is_configured_thumb_key(vk: VkCode) -> bool {
        let (left, right) = crate::hook::thumb_vk_codes();
        vk == left || vk == right
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
    /// 報告生成時点の内容を都度読み直すためにも使う（Runtime側にキャッシュされた
    /// `GjiRawConfig`は存在しないため）。
    #[must_use]
    pub fn read_config1_db() -> Option<Vec<u8>> {
        let path = config1_db_path()?;
        std::fs::read(&path).ok()
    }

    /// `config1.db`の版（更新時刻のナノ秒+長さ）。読めなければ`None`。`KeymapCache`が
    /// 打鍵ごとに全体を読み直さないための判定材料（statだけ）。
    pub(crate) fn config1_db_stamp() -> Option<(u64, u64)> {
        let meta = std::fs::metadata(config1_db_path()?).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some((u64::try_from(modified.as_nanos()).ok()?, meta.len()))
    }

    /// ADR-191 決定3: `config1.db`から、打鍵時点の予測（`key_effect_predictor`）に使うキーマップを読む。
    /// 呼び出しは`KeymapCache`が版の変化時だけに絞る（打鍵ごとに読まない）。パスが解決できない・
    /// `config1.db`が読めない・パースできない場合は`None`。ファイルが**無い**ときはMozcと同じく
    /// 既定のキーマップ（MS-IMEプリセット相当）を返す（ADR-199 決定6-3・決定8 (ii)、
    /// `KeyEffectKeymap::from_config1_db_read`）。`session_keymap`がATOK/MSIME以外
    /// （CUSTOM・MOBILE等）でも、ADR-195段階4 B3対応により`KeymapPreset::Custom`として`Some`を返す
    /// （同梱表による予測は無いが、学習済み表があれば`predict_with_override`経由で使える）。
    pub(crate) fn read_key_effect_keymap(
    ) -> Option<crate::state::key_effect_predictor::KeyEffectKeymap> {
        let path = config1_db_path()?;
        crate::state::key_effect_predictor::KeyEffectKeymap::from_config1_db_read(std::fs::read(
            &path,
        ))
    }

    /// [`bundled_preset_for_adjudication`]の戻り値（ADR196-T2「1e前半」決定1c）。
    ///
    /// `NotKnown`と`ConfigUnreadable`を区別する——前者は「既知構成でない」という
    /// 確定した判定結果、後者は「判定できなかった」という不確実性そのものを表す
    /// （opus-adversarial-consult 2026-09-23 C-5: 「既知でない」と「読めなかった」を
    /// 同じ値に潰すと、不具合報告で原因を切り分けられなくなる）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BundledPresetLookup {
        /// 内蔵表との突き合わせに使うプリセットが確定した。
        Known(crate::state::key_effect_predictor::KeymapPreset),
        /// GJIだが既知構成でない、またはGJI以外（Microsoft IME本体・その他のTIP）で
        /// 既知構成判定が未実装（T2タスク1c「未着手のうちは既知構成と判定しない」）。
        NotKnown,
        /// `TipIdentity::Gji`なのに`config1.db`が読めない・パースできない
        /// （GJI未インストールの矛盾等、異常系）。
        ConfigUnreadable,
    }

    /// ADR196-T2決定1c: 学習対象のIME（学習窓で同定した`TipIdentity`）と、現在の
    /// `config1.db`（GJIのときのみ）から、内蔵表との突き合わせ（[`super::super::state::
    /// key_effect_runtime::diff_against_bundled`]）に使うプリセットを決める。
    ///
    /// `config1.db`は学習対象がGJIでなくても読めてしまう（GJIがインストールされて
    /// いれば常に存在するファイル）ため、`TipIdentity::Gji`のときだけ読む
    /// （opus-adversarial-consult 2026-09-23 B-2: ゲート無しだと、GJIを「ATOKプリセット」に
    /// 設定したままATOK本体やMicrosoft IME本体で学習したセッションが、誤ってGJIのATOK同梱表と
    /// 突き合わされ、偽の不一致になる）。
    #[must_use]
    pub fn bundled_preset_for_adjudication(
        tip: crate::state::ime_kind::TipIdentity,
    ) -> BundledPresetLookup {
        use crate::state::ime_kind::TipIdentity;
        use crate::state::key_effect_predictor::KeymapPreset;
        use awase_gji_config::known_keymap::{classify_known_gji_keymap, KnownGjiKeymap};

        if tip != TipIdentity::Gji {
            // MsImeNative: T2タスク1cのMicrosoft IME本体側判定はADR196-T5待ち(未着手)。
            // Other: 内蔵表を持たない構成(ATOK本体・Japanist等)。
            return BundledPresetLookup::NotKnown;
        }
        let Some(bytes) = read_config1_db() else {
            return BundledPresetLookup::ConfigUnreadable;
        };
        let Some(raw) = awase_gji_config::wire::parse_top_level(&bytes) else {
            return BundledPresetLookup::ConfigUnreadable;
        };
        match classify_known_gji_keymap(
            raw.session_keymap,
            &raw.overlay_keymaps,
            raw.custom_keymap_table.as_deref(),
        ) {
            Some(KnownGjiKeymap::Atok) => BundledPresetLookup::Known(KeymapPreset::Atok),
            Some(KnownGjiKeymap::MsIme) => BundledPresetLookup::Known(KeymapPreset::MsIme),
            None => BundledPresetLookup::NotKnown,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::state::ime_kind::TipIdentity;

        /// B-2回帰テスト(opus-adversarial-consult 2026-09-23): GJI以外のTIPでは
        /// config1.dbを一切読まず(読めても)常に`NotKnown`を返す。`MsImeNative`は
        /// T2タスク1cのMicrosoft IME本体側判定が未実装のため、`Other`は内蔵表を
        /// 持たない構成のため、どちらも既知構成の対象外。
        /// `#[cfg(windows)]`配下(Win32型`TipIdentity`比較を含む)のため、Windows
        /// ターゲットでのみ実行される(`cargo check --target x86_64-pc-windows-msvc`
        /// で存在確認、実行はwindows-build CI)。
        #[test]
        fn non_gji_tip_never_reads_config1_db() {
            assert_eq!(
                bundled_preset_for_adjudication(TipIdentity::MsImeNative),
                BundledPresetLookup::NotKnown
            );
            assert_eq!(
                bundled_preset_for_adjudication(TipIdentity::Other),
                BundledPresetLookup::NotKnown
            );
        }
    }
}

#[cfg(test)]
mod tests {
    // ── classify_thumb_key_ime_actions (BUG-115) ──

    use super::{
        classify_mode_key_ime_action, classify_thumb_key_ime_actions, ImeToggleKind,
        ModeKeyCandidate,
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
}
