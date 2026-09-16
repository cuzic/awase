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
}
