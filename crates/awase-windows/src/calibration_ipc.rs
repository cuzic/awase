//! ADR-176 176-T7: awase.exe ⇔ awase-settings 間の較正モードIPCメッセージの
//! ペイロードpack/unpackと、送信元プロセスの検証。
//!
//! 生の`|`/`<<`を両プロセスに散らすと、後でフィールドを1つ足したときに
//! 片方だけ直す事故になる（opus-adversarial-consultレビューround7 S3
//! 指摘）ため、この1箇所に集約する。Windows APIには依存しない純粋関数
//! のためLinux上でユニットテスト可能。

use crate::state::ime_kind::ImeKindId;
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

/// ADR-176 176-T9b: awase-settings側の較正結果受信用メッセージ専用
/// ウィンドウの固定クラス名。
///
/// winitの既定クラス名（`"Window Class"`）はプロセス間で衝突する
/// （決定2、`focus/imm_learning.rs`のBUG-107文脈参照）ため、
/// awase.exe側の`awase_tray_window`（`tray.rs`）と同型の専用クラス名を
/// 新設し、awase.exe側が`FindWindowW`でこれを探して`WM_CALIBRATION_
/// RESULT`を送る（round7 S1・round9 N5の「HWNDはIPCで運ばず固定クラス名
/// ルックアップに統一する」方針をawase.exe→awase-settings方向にも適用）。
///
/// この定数をawase-windows側に置き両プロセスから参照することで、
/// `WM_RELOAD_CONFIG`が過去に踏んだ「クラス名を両側で独立に書き、
/// 片方だけ古いままになる」事故（`main.rs`の該当コメント参照、
/// opus-adversarial-consultレビューround7 S3指摘）を構造的に防ぐ。
pub const CALIBRATION_RESULT_WINDOW_CLASS_NAME: &str = "awase_settings_calibration_result_window";

/// ADR-176 176-T9b: `WM_CALIBRATION_RESULT`のペイロード（awase.exe→
/// awase-settings）。対象VK（下位16bit）+ 結果種別（次の16bit）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationResultPayload {
    pub vk: VkCode,
    pub kind: CalibrationResultKind,
    /// ADR-176（T9a確定結果のconfig.toml永続化）: 確定時点でawase.exeが
    /// 観測していたIME種別。`awase-settings`が`ConfirmedOn`を受けて
    /// `config1.db`かレジストリのどちらを読み直すか決めるために必要
    /// （`gji_charset_autodetect::build_confirmed_calibration_entry`参照）。
    /// `Rejected`では未使用だが、ペイロード形状を`kind`で分岐させない
    /// ために常に含める。
    pub active_ime_kind: ImeKindId,
}

/// 較正結果の種別。`Undetermined`（未確定）は送信しない
/// （`runtime/focus_tracking.rs`の較正probeループ参照）ため、ここには
/// 含めない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationResultKind {
    /// `ImeToggleKind::On`として確定した。
    ConfirmedOn,
    /// `Toggle`の決定的証拠が観測されたため確定できず却下した。
    Rejected,
}

impl CalibrationResultKind {
    const fn to_bits(self) -> usize {
        match self {
            Self::ConfirmedOn => 1,
            Self::Rejected => 2,
        }
    }

    const fn from_bits(bits: usize) -> Option<Self> {
        match bits {
            1 => Some(Self::ConfirmedOn),
            2 => Some(Self::Rejected),
            _ => None,
        }
    }
}

const fn ime_kind_id_to_bits(kind: ImeKindId) -> usize {
    match kind {
        ImeKindId::Gji => 1,
        ImeKindId::MsIme => 2,
    }
}

const fn ime_kind_id_from_bits(bits: usize) -> Option<ImeKindId> {
    match bits {
        1 => Some(ImeKindId::Gji),
        2 => Some(ImeKindId::MsIme),
        _ => None,
    }
}

/// `CalibrationResultPayload`をwparamへエンコードする。lparamは未使用
/// （0固定、`pack`/`unpack`と同じ理由）。vk（下位16bit）+ 結果種別
/// （次の16bit）+ `active_ime_kind`（さらに次の16bit）。
#[must_use]
pub const fn pack_result(payload: CalibrationResultPayload) -> usize {
    (payload.vk.0 as usize)
        | (payload.kind.to_bits() << 16)
        | (ime_kind_id_to_bits(payload.active_ime_kind) << 32)
}

/// `pack_result`の逆変換。不正な結果種別/IME種別ビットが渡された場合は
/// `None`（通信路の破損・将来のバージョン不一致を安全に無視する）。
#[must_use]
pub const fn unpack_result(wparam: usize) -> Option<CalibrationResultPayload> {
    let vk = VkCode((wparam & 0xFFFF) as u16);
    let kind = CalibrationResultKind::from_bits((wparam >> 16) & 0xFFFF);
    let active_ime_kind = ime_kind_id_from_bits((wparam >> 32) & 0xFFFF);
    match (kind, active_ime_kind) {
        (Some(kind), Some(active_ime_kind)) => Some(CalibrationResultPayload {
            vk,
            kind,
            active_ime_kind,
        }),
        _ => None,
    }
}

/// `WM_CALIBRATION_SET_IME_OPEN`のwparamペイロード。送信元awase-settingsの
/// PID + 設定したいIME open状態。ADR-176ガイド付きウィザードの「IMEを
/// ON/OFFにする」ボタン専用（`lib.rs`の`WM_CALIBRATION_SET_IME_OPEN`
/// doc参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationSetImeOpenPayload {
    pub pid: u32,
    pub open: bool,
}

/// `CalibrationSetImeOpenPayload`をwparamへエンコードする（最下位1bit=
/// open、次の32bit=pid）。
#[must_use]
pub const fn pack_set_ime_open(payload: CalibrationSetImeOpenPayload) -> usize {
    (payload.open as usize) | ((payload.pid as usize) << 1)
}

/// `pack_set_ime_open`の逆変換。
#[must_use]
pub const fn unpack_set_ime_open(wparam: usize) -> CalibrationSetImeOpenPayload {
    CalibrationSetImeOpenPayload {
        open: (wparam & 1) != 0,
        pid: ((wparam >> 1) & 0xFFFF_FFFF) as u32,
    }
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

    // ── 176-T9b: pack_result/unpack_result ───────────────────────────────

    #[test]
    fn pack_unpack_result_round_trips_confirmed_on() {
        let payload = CalibrationResultPayload {
            vk: VkCode(0x1D),
            kind: CalibrationResultKind::ConfirmedOn,
            active_ime_kind: ImeKindId::Gji,
        };
        let wparam = pack_result(payload);
        assert_eq!(unpack_result(wparam), Some(payload));
    }

    #[test]
    fn pack_unpack_result_round_trips_rejected() {
        let payload = CalibrationResultPayload {
            vk: VkCode(0x1C),
            kind: CalibrationResultKind::Rejected,
            active_ime_kind: ImeKindId::MsIme,
        };
        let wparam = pack_result(payload);
        assert_eq!(unpack_result(wparam), Some(payload));
    }

    #[test]
    fn unpack_result_rejects_unknown_kind_bits() {
        // kind bits = 0 は未定義（ConfirmedOn=1, Rejected=2 のみ有効）。
        let wparam = usize::from(VkCode(0x1D).0);
        assert_eq!(unpack_result(wparam), None);
    }

    #[test]
    fn unpack_result_rejects_unknown_active_ime_kind_bits() {
        // active_ime_kind bits = 0 は未定義（Gji=1, MsIme=2 のみ有効）。
        let wparam =
            usize::from(VkCode(0x1D).0) | (CalibrationResultKind::ConfirmedOn.to_bits() << 16);
        assert_eq!(unpack_result(wparam), None);
    }

    // ── ADR-176: pack_set_ime_open/unpack_set_ime_open ───────────────────

    #[test]
    fn pack_unpack_set_ime_open_round_trips_open() {
        let payload = CalibrationSetImeOpenPayload {
            pid: 123_456,
            open: true,
        };
        let wparam = pack_set_ime_open(payload);
        assert_eq!(unpack_set_ime_open(wparam), payload);
    }

    #[test]
    fn pack_unpack_set_ime_open_round_trips_closed_with_max_pid() {
        let payload = CalibrationSetImeOpenPayload {
            pid: u32::MAX,
            open: false,
        };
        let wparam = pack_set_ime_open(payload);
        assert_eq!(unpack_set_ime_open(wparam), payload);
    }
}
