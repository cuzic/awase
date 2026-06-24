//! IME 入力モード belief FSM（Off / Hiragana / Katakana）。
//!
//! [`GjiFsm`] が warm/cold（起動状態）を管理するのに対し、
//! 本 FSM は入力モード（OFF / ひらがな / カタカナ）の belief を管理する。
//!
//! ## 更新ソース
//!
//! - F22 送信 → `Off`（即時 belief）
//! - F21 送信 → `Hiragana`（即時 belief）
//! - `IMC_GETCONVERSIONMODE` async ポーリング → 確定（`confirmed=true`）
//! - フォーカス変更 → `Unknown`（再確認が必要）
//!
//! ## 利用方法
//!
//! `Output` が `RefCell<ImeModeFsm>` として保持する。
//! `TsfEnvSnapshot.ime_mode` / `ime_mode_confirmed` を通じて各 TickableFsm に公開する。

/// IME 入力モードの belief 値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ImeModeState {
    /// 不明（起動直後 / フォーカス変更直後）。
    #[default]
    Unknown,
    /// IME OFF（ROMAN モード、NATIVE=0）。
    Off,
    /// ひらがな入力（NATIVE=1, KATAKANA=0）。
    Hiragana,
    /// カタカナ入力（NATIVE=1, KATAKANA=1）。
    Katakana,
}

impl ImeModeState {
    pub(crate) fn from_conversion_mode(mode: u32) -> Self {
        if mode & crate::imm::IME_CMODE_NATIVE == 0 {
            Self::Off
        } else if mode & crate::imm::IME_CMODE_KATAKANA != 0 {
            Self::Katakana
        } else {
            Self::Hiragana
        }
    }

    pub(crate) const fn is_hiragana(self) -> bool {
        matches!(self, Self::Hiragana)
    }
}

/// IME 入力モード belief を管理する状態機械。
///
/// `Output` が `RefCell<ImeModeFsm>` として保持する。
pub(crate) struct ImeModeFsm {
    state: ImeModeState,
    /// `true` = `IMC_GETCONVERSIONMODE` で OS から確認済み（belief ではなく ground truth）。
    confirmed: bool,
}

impl Default for ImeModeFsm {
    fn default() -> Self {
        Self::new()
    }
}

impl ImeModeFsm {
    pub(crate) fn new() -> Self {
        Self {
            state: ImeModeState::Unknown,
            confirmed: false,
        }
    }

    pub(crate) fn state(&self) -> ImeModeState {
        self.state
    }

    pub(crate) fn is_confirmed(&self) -> bool {
        self.confirmed
    }

    /// F22（IME OFF）送信時に呼ぶ。Off belief に即時移行する。
    pub(crate) fn on_f22_sent(&mut self) {
        log::debug!("[ime-mode] F22 送信 → Off (belief, unconfirmed)");
        self.state = ImeModeState::Off;
        self.confirmed = false;
    }

    /// F21（IME ON / ひらがな）送信時に呼ぶ。Hiragana belief に即時移行する。
    pub(crate) fn on_f21_sent(&mut self) {
        log::debug!("[ime-mode] F21 送信 → Hiragana (belief, unconfirmed)");
        self.state = ImeModeState::Hiragana;
        self.confirmed = false;
    }

    /// `IMC_GETCONVERSIONMODE` の結果を反映する。
    ///
    /// `None` = タイムアウト / IME ウィンドウなし（belief は変更しない）。
    pub(crate) fn on_conversion_mode_read(&mut self, mode: Option<u32>) {
        let Some(mode) = mode else {
            log::debug!("[ime-mode] IMC_GETCONVERSIONMODE: None (timeout / no IME window)");
            return;
        };
        let new_state = ImeModeState::from_conversion_mode(mode);
        if new_state != self.state {
            log::warn!(
                "[ime-mode] drift detected: belief={:?} → actual={:?} (conv=0x{:08X})",
                self.state, new_state, mode
            );
        } else {
            log::debug!(
                "[ime-mode] confirmed: {:?} (conv=0x{:08X})",
                new_state, mode
            );
        }
        self.state = new_state;
        self.confirmed = true;
    }

    /// フォーカス変更時に呼ぶ。Unknown に戻す（次の `IMC_GETCONVERSIONMODE` で再確認）。
    pub(crate) fn on_focus_changed(&mut self) {
        log::debug!("[ime-mode] FocusChange → Unknown");
        self.state = ImeModeState::Unknown;
        self.confirmed = false;
    }
}
