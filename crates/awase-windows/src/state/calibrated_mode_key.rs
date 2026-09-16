//! [ADR-176](../../../../docs/adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
//! 決定6: モードキー較正結果1件を表す純粋データ構造。
//!
//! `176-T1`。プラットフォーム非依存（`windows`crateに依存しない）な純粋
//! データ構造として、Linux上でも定義・テストできる場所に置く
//! （`state/ime_kind.rs`と同じ ungated パターン）。この構造体自体は
//! `ImeModel`のbeliefではない——`.claude/rules/ime-belief-architecture.md`の
//! Observe→`classify_*`→`reduce()`規律が対象とするIME belief状態
//! （ON/OFF・input_mode）とは別の、静的config分類を補完する較正記録
//! である。

use crate::gji_charset_autodetect::ImeToggleKind;
use crate::state::ime_kind::ImeKindId;
use crate::vk::VkCodeExt as _;
use awase::config::ImeDetectConfig;
use awase::types::VkCode;

/// GJI/MS-IMEの`config1.db`/レジストリの、較正時点でのフィンガープリント。
///
/// `is_stale`がこれを現在値と比較し、食い違えば較正結果を無効化する
/// （BUG-143の既知の限界——GUI実装のクリア漏れによる`custom_keymap_table`
/// 残留——を検出する手段としても機能する、ADR-176決定6）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigFingerprint {
    Gji {
        /// `session_keymap`フィールドの値（`awase-gji-config`のraw値）。
        session_keymap: Option<i64>,
        /// 較正対象キーに関連する`custom_keymap_table`の該当行
        /// （無ければ`None`）。行の生テキストをそのまま保持し、
        /// 意味解釈はここでは行わない。
        relevant_row: Option<String>,
    },
    MsIme {
        /// 較正に関連するレジストリ値のハッシュ。
        registry_value_hash: u64,
    },
}

/// 較正結果1件。対象VK・GJI/MS-IMEどちらの環境で較正したか・較正時点の
/// config指紋・確定時刻を持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CalibratedModeKey {
    pub(crate) vk: VkCode,
    /// v8時点のスコープ（OFF→ON方向のみ）では`ImeToggleKind::On`のみが
    /// 実際に保存される想定だが、型としては3値のまま持つ
    /// （decision4「変化なし/Toggle判別不能は保存しない」の判定は
    /// この型の外、確定ロジック側で行う）。
    pub(crate) result: ImeToggleKind,
    pub(crate) active_ime_kind: ImeKindId,
    pub(crate) config_fingerprint: ConfigFingerprint,
    /// 較正が確定した時刻（`TickMs`ではなく永続化を跨ぐため、
    /// プロセス再起動をまたいでも意味を持つUnix epoch msで保持する）。
    pub(crate) confirmed_at_epoch_ms: u64,
}

/// `record`の`config_fingerprint`が`current`と食い違っていれば`true`
/// （stale、静的分類へフォールバックすべき）。
#[must_use]
pub(crate) fn is_stale(record: &CalibratedModeKey, current: &ConfigFingerprint) -> bool {
    record.config_fingerprint != *current
}

/// `176-T2`（ADR-176決定5）: `gate_thumb_key_ime_actions`が返す
/// `wiring.henkan`/`wiring.muhenkan`（`static_result`）を、確定済み較正結果
/// （`calibrated`）があればそれで差し替える純粋関数。
///
/// `calibrated`はstale判定済みの値を渡すこと（stale/未較正なら呼び出し側で
/// `None`にしてから渡す——このモジュールの`is_stale`を使う）。
/// この関数自体は「較正結果があれば最優先」という単純な優先順位だけを持ち、
/// staleかどうかの判断はここでは行わない。
#[must_use]
pub(crate) fn apply_calibration_override(
    static_result: Option<ImeToggleKind>,
    calibrated: Option<&CalibratedModeKey>,
) -> Option<ImeToggleKind> {
    calibrated.map(|c| c.result).or(static_result)
}

/// `176-T5`（ADR-176決定7、B4対応）: 較正対象VKが明示configに既に
/// 登録されているかを判定し、登録されていれば較正UI（`176-T10`）が
/// 表示すべき警告理由を返す。`None`なら較正してよい。
///
/// BUG-140（`docs/known-bugs/BUG-140.md`）と同じ「優先順位ではなく
/// 構造的除外」の方針を取る: `apply_calibration_override`は較正結果を
/// 無条件に最優先採用する（`176-T2`）ため、既に明示config済みのVKを
/// 較正すると、ユーザーが意図して書いた設定を較正UIが無断で上書きする
/// ことになる。値の優劣や後勝ちで解決するのではなく、較正の実行自体を
/// 拒否して構造的に衝突を起こさせない。
///
/// `awase-settings`から呼び出せるよう`pub`（`awase_windows`クレートの
/// 公開関数）。Windows APIには依存しない純粋関数のため、Linux上で
/// ユニットテストできる。
#[must_use]
pub fn explicit_config_conflict_reason(
    vk: VkCode,
    ime_detect: &ImeDetectConfig,
    ime_on: &[String],
    ime_off: &[String],
    ime_toggle: &[String],
) -> Option<&'static str> {
    let in_ime_detect = [&ime_detect.toggle, &ime_detect.on, &ime_detect.off]
        .into_iter()
        .flatten()
        .filter_map(|s| VkCode::from_name(s))
        .any(|registered| registered == vk);
    if in_ime_detect {
        return Some("keys.ime_detect（IME検出用シャドウ追跡キー）に既に登録されているキーです");
    }

    let bare_vk_registered = [ime_on, ime_off, ime_toggle]
        .into_iter()
        .flatten()
        .filter_map(|s| crate::vk::parse_key_combo(s))
        .any(|combo| combo.vk == vk && !combo.ctrl && !combo.shift && !combo.alt);
    if bare_vk_registered {
        return Some("keys.ime_on/ime_off/ime_toggle に修飾キー無しで既に登録されているキーです");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(fingerprint: ConfigFingerprint) -> CalibratedModeKey {
        CalibratedModeKey {
            vk: VkCode::from(0x1C),
            result: ImeToggleKind::On,
            active_ime_kind: ImeKindId::Gji,
            config_fingerprint: fingerprint,
            confirmed_at_epoch_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn matching_fingerprint_is_not_stale() {
        let fp = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOn".to_string()),
        };
        let record = sample(fp.clone());
        assert!(!is_stale(&record, &fp));
    }

    #[test]
    fn differing_session_keymap_is_stale() {
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let current = ConfigFingerprint::Gji {
            session_keymap: Some(2),
            relevant_row: None,
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    #[test]
    fn differing_relevant_row_is_stale_even_with_same_session_keymap() {
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOn".to_string()),
        };
        let current = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOff".to_string()),
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    #[test]
    fn ms_ime_hash_mismatch_is_stale() {
        let recorded = ConfigFingerprint::MsIme {
            registry_value_hash: 111,
        };
        let current = ConfigFingerprint::MsIme {
            registry_value_hash: 222,
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    #[test]
    fn override_falls_back_to_static_when_no_calibration() {
        let result = apply_calibration_override(Some(ImeToggleKind::Off), None);
        assert_eq!(result, Some(ImeToggleKind::Off));
    }

    #[test]
    fn override_falls_back_to_static_when_calibration_absent_and_static_none() {
        let result = apply_calibration_override(None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn override_prefers_calibration_over_static() {
        let fp = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let calibrated = sample(fp);
        // static_resultが較正結果と異なっていても、較正結果が優先される。
        let result = apply_calibration_override(Some(ImeToggleKind::Off), Some(&calibrated));
        assert_eq!(result, Some(ImeToggleKind::On));
    }

    #[test]
    fn override_prefers_calibration_even_without_static_result() {
        let fp = ConfigFingerprint::MsIme {
            registry_value_hash: 1,
        };
        let calibrated = sample(fp);
        let result = apply_calibration_override(None, Some(&calibrated));
        assert_eq!(result, Some(ImeToggleKind::On));
    }

    #[test]
    fn mismatched_ime_kind_variant_is_stale() {
        // GJIで較正したレコードを、MS-IMEのfingerprintと突き合わせる
        // （較正時と反映時でactive_ime_kindが変わった異常系）。
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let current = ConfigFingerprint::MsIme {
            registry_value_hash: 0,
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    // ── 176-T5: explicit_config_conflict_reason ─────────────────────────

    fn muhenkan() -> VkCode {
        VkCode::from_name("無変換").expect("VK_NONCONVERT should parse")
    }

    fn empty_ime_detect() -> ImeDetectConfig {
        ImeDetectConfig {
            toggle: vec![],
            on: vec![],
            off: vec![],
        }
    }

    #[test]
    fn no_conflict_when_vk_absent_from_all_lists() {
        let ime_detect = ImeDetectConfig {
            toggle: vec![],
            on: vec!["IMEオン".to_string()],
            off: vec!["IMEオフ".to_string()],
        };
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &[]);
        assert_eq!(result, None);
    }

    #[test]
    fn conflicts_when_vk_registered_in_ime_detect_on() {
        let ime_detect = ImeDetectConfig {
            toggle: vec![],
            on: vec!["IMEオン".to_string(), "無変換".to_string()],
            off: vec![],
        };
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &[]);
        assert!(result.is_some(), "keys.ime_detect.on との衝突を検出すべき");
    }

    #[test]
    fn conflicts_when_vk_registered_in_ime_detect_toggle() {
        let ime_detect = ImeDetectConfig {
            toggle: vec!["無変換".to_string()],
            on: vec![],
            off: vec![],
        };
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &[]);
        assert!(
            result.is_some(),
            "keys.ime_detect.toggle との衝突を検出すべき"
        );
    }

    #[test]
    fn conflicts_when_vk_registered_bare_in_ime_on() {
        let ime_detect = empty_ime_detect();
        let ime_on = vec!["無変換".to_string()];
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &ime_on, &[], &[]);
        assert!(
            result.is_some(),
            "keys.ime_on への修飾キー無し登録との衝突を検出すべき（src/config.rs:601-609の実害と同型）"
        );
    }

    #[test]
    fn conflicts_when_vk_registered_bare_in_ime_toggle() {
        let ime_detect = empty_ime_detect();
        let ime_toggle = vec!["無変換".to_string()];
        let result =
            explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &ime_toggle);
        assert!(result.is_some());
    }

    #[test]
    fn no_conflict_when_vk_registered_with_modifier_in_ime_off() {
        // 修飾キー付き（例: Ctrl+無変換）は Phase 1 で無条件消費しないため、
        // BUG-140/実害報告と同種の衝突ではない。構造的除外の対象外。
        let ime_detect = empty_ime_detect();
        let ime_off = vec!["Ctrl+無変換".to_string()];
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &ime_off, &[]);
        assert_eq!(
            result, None,
            "修飾キー付き登録は構造的除外の対象外（優先順位の問題であり本判定の対象外）"
        );
    }

    #[test]
    fn no_conflict_when_different_vk_registered() {
        let ime_detect = empty_ime_detect();
        let ime_on = vec!["変換".to_string()];
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &ime_on, &[], &[]);
        assert_eq!(result, None);
    }
}
