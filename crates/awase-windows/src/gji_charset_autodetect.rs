//! GJI検出時、`config1.db`の`custom_keymap_table`から専用Fnキー変換モード
//! （ADR-091 §D3.2）を自動判定する（同§D3.1項目1）。
//!
//! # 設計方針
//!
//! - **新しいbeliefは持たない**（ADR-091の中心方針）。ここでの判定は
//!   `config1.db`という外部ファイルの現在の中身を毎回そのまま読むだけで、
//!   awase側で過去の観測を蓄積・推測することはしない。
//! - **継続的なポーリングはしない**（ADR-091決定3項目2）。GJIが継続して
//!   アクティブな間は一度判定したら再読み込みしない（[`sync_gji_charset_autodetect`]
//!   のラッチ参照）。GJI以外へ遷移したら自動検出は解除する（F21等の専用Fnキーが
//!   他IMEへ漏れるのを防ぐ）。
//! - **手動設定が常に優先**。`GeneralConfig::muhenkan_solo_tap_dedicated_fn_key`
//!   が明示設定されている間は、この自動判定は一切介入しない
//!   （`Runtime::muhenkan_dedicated_fn_key_is_manual`/`set_muhenkan_dedicated_fn_key_auto`
//!   参照）。
//! - **config1.db未存在（GJI未インストール等）はエラーではない**。読めなければ
//!   静かに何もしない（既定の「抑止」のまま）。`awase-gji-config`crate自体の
//!   「パース失敗は常に空の結果に静かにフォールバック」という既存方針を踏襲する。

use awase::config::ParsedKeyCombo;
use awase::types::VkCode;

use crate::vk::VkCodeExt as _;

/// 専用Fnキー変換として自動採用してよいVK名の範囲。
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

/// `config1.db`の`custom_keymap_table`（TSV文字列）から、専用Fnキー変換として
/// 自動的に有効化すべき`VkCode`を判定する（ADR-091 §D3.1項目1）。
///
/// 安全範囲内（[`is_in_safe_autodetect_range`]）で`SwitchKanaType`
/// （`ToggleKanaType`に分類される）に割り当てられているキーがちょうど1つ
/// なら、それを採用する。0個、または複数（どれを使うべきか一意に定まらない）
/// なら`None`（安全側、既定の「抑止」のまま変更しない）。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn detect_dedicated_fn_key(custom_keymap_table: &str) -> Option<VkCode> {
    let mode_keys = awase_gji_config::keymap::extract_mode_keys(custom_keymap_table);
    let mut candidates = mode_keys
        .toggle_kana_type
        .iter()
        .filter(|vk_name| is_in_safe_autodetect_range(vk_name));
    let only = candidates.next()?;
    if candidates.next().is_some() {
        log::warn!(
            "[gji-charset-autodetect] 安全範囲内に複数の SwitchKanaType キーが \
             見つかったため自動判定を見送りました（一意に定まらない）"
        );
        return None;
    }
    VkCode::from_name(only)
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
/// GJIが無変換/変換等の親指キーにIME ON/OFFを割り当てるケースは、そもそも
/// `mozc_key_to_vk_name`の出力範囲（F1-F24と上記4エイリアスのみ）に無変換/
/// 変換のVK名が含まれないため発生しない（Step4bの無変換/変換
/// delegate-to-open-axisとは競合し得ない）。
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

#[cfg(windows)]
pub(crate) use windows_impl::sync_gji_charset_autodetect;

#[cfg(windows)]
mod windows_impl {
    use std::sync::atomic::{AtomicU8, Ordering};

    use crate::runtime::Runtime;

    use super::{detect_dedicated_fn_key, extract_ime_on_off_toggle_combos};

    const NOT_GJI: u8 = 0;
    const GJI_CHECKED: u8 = 1;

    /// GJIが継続してアクティブな「区間」ごとに一度だけ判定するためのラッチ。
    /// `NOT_GJI`（GJI以外、または未判定）/`GJI_CHECKED`（この区間で判定済み）
    /// の2値。`sync_gji_charset_autodetect`以外から触らない。
    static LAST_GJI_STREAK_CHECKED: AtomicU8 = AtomicU8::new(NOT_GJI);

    /// `runtime::message_handlers::sync_ime_kind_from_observation`から呼ぶ、
    /// GJI検出/離脱の唯一の合流点（`msime_key_assignment::check_and_warn`と対）。
    /// 専用Fnキー変換（ADR-091）とIME ON/OFF/トグルキー（ADR-092 Step4c）の
    /// 2つの独立した自動判定を行う。
    ///
    /// - **GJI以外への遷移**: 直前がGJI継続区間だった場合のみ、自動検出して
    ///   いた専用Fnキー変換・`ime_on_auto`/`ime_off_auto`を解除する
    ///   （`ime_toggle_auto`は意図的に対象外、関数内コメント参照）。
    /// - **GJIへの新規遷移**: `config1.db`を読み、(1)安全範囲内のFnキー
    ///   バインドがちょうど1つ見つかれば専用Fnキー変換として自動的に有効化し、
    ///   (2)宣言されているIME ON/OFF/トグルキー（安全範囲内のみ）を
    ///   `Engine`の自動検出リストへ反映する。既にこのGJI継続区間でチェック
    ///   済みなら（ラッチが`GJI_CHECKED`のまま）何もしない——継続的な
    ///   ポーリングをしないため（ADR-091決定3項目2）。
    ///
    /// `app.muhenkan_dedicated_fn_key_is_manual()`（ユーザーが
    /// `muhenkan_solo_tap_dedicated_fn_key`を明示設定済み）による手動優先
    /// ガードは**専用Fnキー変換の自動検出にのみ**適用され、IME ON/OFF/
    /// トグルキーの自動検出（Step4c）には適用されない——Step4c側の手動優先
    /// は`Engine::match_ime_on_off_auto`/`match_ime_toggle_auto`が
    /// `special_keys`側で独立に行う（Opus コードレビュー指摘: 以前は
    /// この関数全体が同じガードで早期returnしており、専用Fnキーを手動
    /// 設定しているユーザーがStep4cの機能を丸ごと失っていた）。
    pub(crate) fn sync_gji_charset_autodetect(app: &mut Runtime, is_gji: bool) {
        if !is_gji {
            if LAST_GJI_STREAK_CHECKED.swap(NOT_GJI, Ordering::Relaxed) == GJI_CHECKED {
                log::info!(
                    "[gji-charset-autodetect] GJI から離脱: 自動検出した専用Fnキー変換・\
                     IME ON/OFFキーを解除"
                );
                app.set_muhenkan_dedicated_fn_key_auto(None);
                // ime_on_auto/ime_off_auto/ime_toggle_auto を全て解除する
                // （さもないと GJI 離脱後も別アプリ/別IMEの文脈にF15-F24の
                // バインドが残留してしまう）。ime_toggle_auto は MS-IME 側
                // （sync_ime_toggle_auto_detect）とも共有するフィールドだが、
                // 呼び出し元（message_handlers::sync_ime_kind_from_observation）
                // が GJI 側の同期を MS-IME 側より**先に**呼ぶ順序になっている
                // ため、GJI→MS-IME遷移ではここで解除した直後に MS-IME 側が
                // 新しい値で上書きし、破綻しない（Opus コードレビュー指摘:
                // 以前は逆順だったため、ここで解除すると MS-IME 側の値を
                // 上書き消去してしまう懸念からime_toggle_autoを対象外にして
                // いたが、その代わりの前提「GJI側は次回アクティブ化時に
                // 必ず自分の現在値で上書きする」も
                // `muhenkan_dedicated_fn_key_is_manual()`等の早期returnで
                // 成立しない場合があり誤りだった）。
                app.clear_gji_ime_on_off_auto_keys();
            }
            return;
        }
        if LAST_GJI_STREAK_CHECKED.swap(GJI_CHECKED, Ordering::Relaxed) == GJI_CHECKED {
            return;
        }
        let Some(bytes) = read_config1_db() else {
            log::debug!(
                "[gji-charset-autodetect] config1.db を読めませんでした \
                 （GJI 未インストール、または初回起動でまだ作成されていない等）"
            );
            return;
        };
        let Some(raw) = awase_gji_config::wire::parse_top_level(&bytes) else {
            return;
        };
        if raw.session_keymap != Some(awase_gji_config::SESSION_KEYMAP_CUSTOM) {
            // session_keymap が CUSTOM でなければ（ATOK/MS-IME 等のプリセット
            // 選択中）、custom_keymap_table に何が残っていても GJI はそれを
            // 参照しない。古いカスタムテーブルの残骸を誤って有効と判定しない
            // ための必須ガード（Opus レビュー指摘）。
            log::debug!(
                "[gji-charset-autodetect] session_keymap が CUSTOM ではないため \
                 自動判定をスキップ: {:?}",
                raw.session_keymap
            );
            return;
        }
        let Some(table) = raw.custom_keymap_table else {
            return;
        };

        // ADR-092 決定D Step4c: IME ON/OFF/トグルキーの自動検出は専用Fnキー
        // 変換（ToggleKanaType）の検出可否・`muhenkan_dedicated_fn_key_is_manual`
        // に依存しない、独立した判定のため、下の専用Fnキー検出（手動設定
        // ガード付き）より前に行う（Opus コードレビュー指摘: 以前はこの
        // ブロックが `muhenkan_dedicated_fn_key_is_manual()` の早期return
        // より後にあり、専用Fnキーを手動設定しているユーザーは本Stepの
        // 機能を丸ごと失っていた——doc の「独立した判定」という主張と矛盾
        // していた）。2026-08-16 ユーザー判断: `Engine::match_ime_on_off_auto`/
        // `match_ime_toggle_auto`は`special_keys.ime_on/ime_off/ime_toggle`が
        // 非空でも自動リストを併用する（明示 ∪ 自動、手動排他ではない）ため、
        // ここでの事前チェックは元々不要（Step4aと同じ規約）。
        let (on, off, toggle) = extract_ime_on_off_toggle_combos(&table);
        if !on.is_empty() || !off.is_empty() || !toggle.is_empty() {
            log::info!(
                "[gji-charset-autodetect] config1.db から IME ON/OFF/トグルキーを \
                 自動検出しました: on={on:?} off={off:?} toggle={toggle:?}"
            );
        }
        app.set_gji_ime_on_off_toggle_auto_keys(on, off, toggle);

        // 専用Fnキー変換の自動検出は、Step4cと異なり手動設定
        // （`muhenkan_solo_tap_dedicated_fn_key`）が優先される
        // （`Runtime::set_muhenkan_dedicated_fn_key_auto` が内部で同じ
        // ガードを持つため、ここでの事前チェックは主にログ・処理の無駄な
        // 実行を避けるための早期return）。
        if app.muhenkan_dedicated_fn_key_is_manual() {
            log::debug!(
                "[gji-charset-autodetect] muhenkan_solo_tap_dedicated_fn_key が \
                 手動設定済みのため専用Fnキーの自動判定をスキップ"
            );
            return;
        }
        let Some(vk) = detect_dedicated_fn_key(&table) else {
            return;
        };
        log::info!(
            "[gji-charset-autodetect] config1.db から専用Fnキー変換を自動検出しました: {vk:?}"
        );
        app.set_muhenkan_dedicated_fn_key_auto(Some(vk));
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
    use super::{detect_dedicated_fn_key, extract_ime_on_off_toggle_combos};
    use crate::vk::VkCodeExt as _;
    use awase::types::VkCode;

    fn table_with_toggle_kana_type(vk_key: &str) -> String {
        format!(
            "status\tkey\tcommand\n\
             Composition\t{vk_key}\tSwitchKanaType\n\
             Conversion\t{vk_key}\tSwitchKanaType\n"
        )
    }

    #[test]
    fn detects_single_safe_range_key() {
        let table = table_with_toggle_kana_type("F21");
        assert_eq!(detect_dedicated_fn_key(&table), VkCode::from_name("VK_F21"));
    }

    #[test]
    fn detects_key_at_edge_of_safe_range() {
        for key in ["F15", "F20", "F23", "F24"] {
            let table = table_with_toggle_kana_type(key);
            assert!(
                detect_dedicated_fn_key(&table).is_some(),
                "{key} は安全範囲内のはず"
            );
        }
    }

    /// ADR-057: F13/F14はターミナルエスケープシーケンス漏れが実機確認済みの
    /// ため、config1.dbにバインドが見つかっても絶対に自動採用しない。
    #[test]
    fn never_adopts_f13_or_f14() {
        for key in ["F13", "F14"] {
            let table = table_with_toggle_kana_type(key);
            assert_eq!(
                detect_dedicated_fn_key(&table),
                None,
                "{key} は安全範囲外のため自動採用してはならない"
            );
        }
    }

    #[test]
    fn no_binding_yields_none() {
        assert_eq!(detect_dedicated_fn_key(""), None);
    }

    #[test]
    fn out_of_range_key_yields_none() {
        // F1 はToggleKanaTypeに割り当てられていても安全範囲外。
        let table = table_with_toggle_kana_type("F1");
        assert_eq!(detect_dedicated_fn_key(&table), None);
    }

    /// 安全範囲内に複数のToggleKanaTypeキーが存在する場合、どちらを使うべきか
    /// 一意に定まらないため採用しない（安全側）。
    #[test]
    fn multiple_safe_range_candidates_yields_none() {
        let table = "status\tkey\tcommand\n\
                      Composition\tF21\tSwitchKanaType\n\
                      Composition\tF22\tSwitchKanaType\n";
        assert_eq!(detect_dedicated_fn_key(table), None);
    }

    /// BUG-64のconfig1.db残骸バインド（F21→IMEOn）が残っていても、この関数は
    /// `IMEOn`コマンドの行を`ToggleKanaType`には分類しない（extract_mode_keysが
    /// classify_commandで区別する）ため誤検出しない。
    #[test]
    fn ime_on_binding_is_not_confused_with_switch_kana_type() {
        let table = "status\tkey\tcommand\nDirectInput\tF21\tIMEOn\n";
        assert_eq!(detect_dedicated_fn_key(table), None);
    }

    /// ADR-091 §D3.2の推奨構成そのもの: Precomposition/DirectInputは未バインド、
    /// Composition/Conversion/Prediction/SuggestionにSwitchKanaType。
    #[test]
    fn detects_full_d32_recommended_binding_set() {
        let table = "status\tkey\tcommand\n\
                      Composition\tF21\tSwitchKanaType\n\
                      Conversion\tF21\tSwitchKanaType\n\
                      Prediction\tF21\tSwitchKanaType\n\
                      Suggestion\tF21\tSwitchKanaType\n";
        assert_eq!(detect_dedicated_fn_key(table), VkCode::from_name("VK_F21"));
    }

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
}
