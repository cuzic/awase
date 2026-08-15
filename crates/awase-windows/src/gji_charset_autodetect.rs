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

use awase::types::VkCode;

use crate::vk::VkCodeExt as _;

/// 専用Fnキー変換として自動採用してよいVK名の範囲。
///
/// `src/config.rs::validate_dedicated_fn_key`と同じ基準（`VK_F15`-`VK_F24`の
/// うち`VK_F13`/`VK_F14`を除く）。`VK_F13`/`VK_F14`はターミナルエスケープ
/// シーケンス漏れが実機確認済み（ADR-057）のため、config1.dbにこれらへの
/// バインドが見つかっても絶対に採用しない。
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

/// `write_dedicated_fn_key_set`（ADR-091 §D3.2、2026-08-15拡張）が書き込む
/// 専用Fnキー一式のうち、無変換単独タップで実際に送信する「主役」キー。
/// F22-24 も同じ状態（Composition系）で`SwitchKanaType`を持つため、
/// [`detect_dedicated_fn_key`]は複数候補の中からこのキーを優先する。
const PRIMARY_GJI_KEY_VK_NAME: &str = "VK_F21";

/// `config1.db`の`custom_keymap_table`（TSV文字列）から、専用Fnキー変換として
/// 自動的に有効化すべき`VkCode`を判定する（ADR-091 §D3.1項目1）。
///
/// 安全範囲内（[`is_in_safe_autodetect_range`]）で`SwitchKanaType`
/// （`ToggleKanaType`に分類される）に割り当てられているキーの中に
/// [`PRIMARY_GJI_KEY_VK_NAME`]（F21）が含まれていれば、他に何個候補があっても
/// 常にそれを採用する——`write_dedicated_fn_key_set`はF21-24全てに
/// `SwitchKanaType`を書き込むため、F21は「主役」として複数候補の中でも
/// 一意に定まる（F22-24は将来awase側が判断して送信する予約キーであり、
/// 無変換単独タップの送信先としては使わない）。F21が候補に無い場合は、
/// 従来通り「安全範囲内でちょうど1つ」の場合のみ採用する（ユーザー自身が
/// 手動で別のFnキーを設定したケースとの後方互換）。0個、またはF21不在で
/// 複数（どれを使うべきか一意に定まらない）なら`None`（安全側）。
#[must_use]
pub(crate) fn detect_dedicated_fn_key(custom_keymap_table: &str) -> Option<VkCode> {
    let mode_keys = awase_gji_config::keymap::extract_mode_keys(custom_keymap_table);
    let mut candidates = mode_keys
        .toggle_kana_type
        .iter()
        .filter(|vk_name| is_in_safe_autodetect_range(vk_name));
    if mode_keys
        .toggle_kana_type
        .iter()
        .any(|vk_name| vk_name == PRIMARY_GJI_KEY_VK_NAME)
    {
        return VkCode::from_name(PRIMARY_GJI_KEY_VK_NAME);
    }
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

#[cfg(windows)]
pub(crate) use windows_impl::sync_gji_charset_autodetect;

#[cfg(windows)]
mod windows_impl {
    use std::sync::atomic::{AtomicU8, Ordering};

    use crate::runtime::Runtime;

    use super::detect_dedicated_fn_key;

    const NOT_GJI: u8 = 0;
    const GJI_CHECKED: u8 = 1;

    /// GJIが継続してアクティブな「区間」ごとに一度だけ判定するためのラッチ。
    /// `NOT_GJI`（GJI以外、または未判定）/`GJI_CHECKED`（この区間で判定済み）
    /// の2値。`sync_gji_charset_autodetect`以外から触らない。
    static LAST_GJI_STREAK_CHECKED: AtomicU8 = AtomicU8::new(NOT_GJI);

    /// `runtime::message_handlers::sync_ime_kind_from_observation`から呼ぶ、
    /// GJI検出/離脱の唯一の合流点（`msime_key_assignment::check_and_warn`と対）。
    ///
    /// - **GJI以外への遷移**: 直前がGJI継続区間だった場合のみ、自動検出して
    ///   いた専用Fnキー変換を解除する。
    /// - **GJIへの新規遷移**: `config1.db`を読み、安全範囲内のFnキーバインドが
    ///   ちょうど1つ見つかれば自動的に有効化する。既にこのGJI継続区間で
    ///   チェック済みなら（ラッチが`GJI_CHECKED`のまま）何もしない
    ///   ——継続的なポーリングをしないため（ADR-091決定3項目2）。
    ///
    /// いずれも `app.muhenkan_dedicated_fn_key_is_manual()` が `true`
    /// （ユーザーが `muhenkan_solo_tap_dedicated_fn_key` を明示設定済み）の間は
    /// 一切介入しない（`Runtime::set_muhenkan_dedicated_fn_key_auto` が
    /// 内部で同じガードを持つため、ここでの事前チェックは主にログ・ファイル
    /// I/O の無駄な実行を避けるための早期return）。
    pub(crate) fn sync_gji_charset_autodetect(app: &mut Runtime, is_gji: bool) {
        if !is_gji {
            if LAST_GJI_STREAK_CHECKED.swap(NOT_GJI, Ordering::Relaxed) == GJI_CHECKED {
                log::info!(
                    "[gji-charset-autodetect] GJI から離脱: 自動検出した専用Fnキー変換を解除"
                );
                app.set_muhenkan_dedicated_fn_key_auto(None);
            }
            return;
        }
        if LAST_GJI_STREAK_CHECKED.swap(GJI_CHECKED, Ordering::Relaxed) == GJI_CHECKED {
            return;
        }
        if app.muhenkan_dedicated_fn_key_is_manual() {
            log::debug!(
                "[gji-charset-autodetect] muhenkan_solo_tap_dedicated_fn_key が \
                 手動設定済みのため自動判定をスキップ"
            );
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
    use super::detect_dedicated_fn_key;
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
    /// F21 が候補に含まれない、真に一意に定まらないケース（F21 不在）。
    #[test]
    fn multiple_safe_range_candidates_without_f21_yields_none() {
        let table = "status\tkey\tcommand\n\
                      Composition\tF22\tSwitchKanaType\n\
                      Composition\tF23\tSwitchKanaType\n";
        assert_eq!(detect_dedicated_fn_key(table), None);
    }

    /// `write_dedicated_fn_key_set`（ADR-091 §D3.2拡張、2026-08-15）はF21-24の
    /// 全てにComposition系`SwitchKanaType`を書き込むため、複数候補が見える。
    /// F21が「主役」として一意に定まる（F22-24は将来の予約キーであり、
    /// 無変換単独タップの送信先には使わない）。
    #[test]
    fn f21_is_preferred_among_multiple_candidates_including_f21() {
        let table = "status\tkey\tcommand\n\
                      Composition\tF21\tSwitchKanaType\n\
                      Composition\tF22\tSwitchKanaType\n\
                      Composition\tF23\tSwitchKanaType\n\
                      Composition\tF24\tSwitchKanaType\n";
        assert_eq!(detect_dedicated_fn_key(table), VkCode::from_name("VK_F21"));
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
}
