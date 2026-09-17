//! ADR-176 176-T7: awase.exe ⇔ awase-settings 間の較正モードIPCメッセージの
//! ペイロードpack/unpackと、送信元プロセスの検証。
//!
//! 生の`|`/`<<`を両プロセスに散らすと、後でフィールドを1つ足したときに
//! 片方だけ直す事故になる（opus-adversarial-consultレビューround7 S3
//! 指摘）ため、この1箇所に集約する。Windows APIには依存しない純粋関数
//! のためLinux上でユニットテスト可能。

use awase::types::VkCode;

/// `WM_CALIBRATION_START`/`WM_CALIBRATION_END`共通のwparamペイロード。
/// 対象VK（下位16bit）+ 送信元awase-settingsのPID（次の32bit）。
/// `WM_CALIBRATION_END`は`vk`を使わない（`VkCode::from(0)`で埋める）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationIpcPayload {
    pub vk: VkCode,
    pub pid: u32,
}

/// `CalibrationIpcPayload`をwparamへエンコードする。lparamは未使用
/// （0固定、`WM_CALIBRATION_START`/`END`ともHWNDを運ばない——round7 S1、
/// 較正の測定はそもそも「awase-settingsにフォーカスがある間」しか
/// 成立しないため、awase.exe自身のライブなフォーカス追跡で足りる）。
#[must_use]
pub const fn pack(payload: CalibrationIpcPayload) -> usize {
    (payload.vk.0 as usize) | ((payload.pid as usize) << 16)
}

/// `pack`の逆変換。
#[must_use]
pub const fn unpack(wparam: usize) -> CalibrationIpcPayload {
    CalibrationIpcPayload {
        vk: VkCode((wparam & 0xFFFF) as u16),
        pid: ((wparam >> 16) & 0xFFFF_FFFF) as u32,
    }
}

/// PIDから取得したプロセス名（`get_process_name`等で解決済みの文字列）が
/// `awase-settings.exe`（大文字小文字・`.exe`有無を無視）と一致するか。
/// `runtime/message_handlers.rs`の`sender_is_awase_settings`
/// （Windows専用、PIDから実際のプロセス名を取得した上でこれを呼ぶ）から
/// 使う純粋判定部分。
#[must_use]
pub fn is_awase_settings_process_name(name: &str) -> bool {
    crate::state::app_suppression::normalize_process_name(name)
        == crate::state::app_suppression::normalize_process_name("awase-settings.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_round_trips() {
        let payload = CalibrationIpcPayload {
            vk: VkCode(0x1C),
            pid: 123_456,
        };
        let wparam = pack(payload);
        assert_eq!(unpack(wparam), payload);
    }

    #[test]
    fn pack_unpack_round_trips_with_max_pid() {
        let payload = CalibrationIpcPayload {
            vk: VkCode(0xFFFF),
            pid: u32::MAX,
        };
        let wparam = pack(payload);
        assert_eq!(unpack(wparam), payload);
    }

    #[test]
    fn process_name_matches_case_insensitive_and_with_or_without_exe() {
        assert!(is_awase_settings_process_name("awase-settings.exe"));
        assert!(is_awase_settings_process_name("AWASE-SETTINGS.EXE"));
        assert!(is_awase_settings_process_name("awase-settings"));
    }

    #[test]
    fn process_name_rejects_other_processes() {
        assert!(!is_awase_settings_process_name("awase.exe"));
        assert!(!is_awase_settings_process_name(""));
        assert!(!is_awase_settings_process_name("evil.exe"));
    }
}
