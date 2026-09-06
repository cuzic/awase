//! IME 入力モード belief FSM（Off / Hiragana / Katakana）。
//!
//! [`GjiFsm`] が warm/cold（起動状態）を管理するのに対し、
//! 本 FSM は入力モード（OFF / ひらがな / カタカナ）の belief を管理する。
//!
//! ## 更新ソース
//!
//! - VK_IME_OFF 送信 → `Off`（即時 belief）
//! - VK_IME_ON 送信 → `Hiragana`（即時 belief）
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
    /// 最後に VK_IME_ON/OFF を送信した時刻 (ms)。0 = 未送信。
    ///
    /// VK_IME_ON/OFF 送信が Chrome の FocusChange を誘発するため、送信後 100ms 以内の
    /// FocusChange では state を Unknown にリセットせず confirmed のみクリアする。
    last_vk_send_ms: u64,
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
            last_vk_send_ms: 0,
        }
    }

    pub(crate) fn state(&self) -> ImeModeState {
        self.state
    }

    pub(crate) fn is_confirmed(&self) -> bool {
        self.confirmed
    }

    /// かな VK 入力を受け付けられる状態が OS 確認済みか。
    ///
    /// Hiragana / Katakana はどちらも NATIVE ビットが立っており romaji VK が
    /// compose されるため「準備完了」。MS-IME confirm-then-transmit ゲート
    /// (`Output::ms_ime_gate_defer`) の判定に使う。
    pub(crate) fn is_native_ready(&self) -> bool {
        self.confirmed && matches!(self.state, ImeModeState::Hiragana | ImeModeState::Katakana)
    }

    /// `ImeEffect::SetOpen` の適用完了時に呼ぶ（機構は ImmCross / MsImeDirect /
    /// GjiDirect / KanjiToggle のいずれでもよい）。belief を即時更新し unconfirmed にする。
    ///
    /// MsImeDirect（VK_DBE_HIRAGANA / VK_IME_OFF を `send_ime_mode_key` で送る）は
    /// `on_f21_sent` / `on_f22_sent` を経由しないため、ここが唯一の invalidate 点になる。
    /// IME ON/OFF 遷移直後は OS 側の準備に実測 ~130-300ms かかる（2026-07-06 WT×MS-IME）ので、
    /// unconfirmed 化により次の送信が IMC 確認を待つ。
    pub(crate) fn on_set_open_applied(&mut self, open: bool) {
        let new_state = if open {
            ImeModeState::Hiragana
        } else {
            ImeModeState::Off
        };
        tracing::debug!("[ime-mode] SetOpen({open}) applied → {new_state:?} (belief, unconfirmed)");
        self.state = new_state;
        self.confirmed = false;
        self.last_vk_send_ms = crate::hook::current_tick_ms();
    }

    /// 外部要因で conv が変わった可能性があるとき、belief の state は保ったまま
    /// unconfirmed 化する。次の送信は msime-ready ゲートが IMC を確認してから行う。
    ///
    /// 用途: Shift 解放時（MS-IME が Shift 単独タップと誤認して英数へ切り替える
    /// 可能性があるタイミング）等、awase の送信起点ではない conv 変化の疑い。
    pub(crate) fn unconfirm(&mut self, reason: &str) {
        if self.confirmed {
            tracing::debug!(
                "[ime-mode] unconfirm ({reason}): {:?} → IMC 確認待ち",
                self.state
            );
        }
        self.confirmed = false;
    }

    /// VK_IME_OFF 送信時に呼ぶ。Off belief に即時移行する。
    pub(crate) fn on_f22_sent(&mut self) {
        tracing::debug!("[ime-mode] VK_IME_OFF 送信 → Off (belief, unconfirmed)");
        self.state = ImeModeState::Off;
        self.confirmed = false;
        self.last_vk_send_ms = crate::hook::current_tick_ms();
    }

    /// VK_IME_ON 送信時に呼ぶ。Hiragana belief に即時移行する。
    pub(crate) fn on_f21_sent(&mut self) {
        tracing::debug!("[ime-mode] VK_IME_ON 送信 → Hiragana (belief, unconfirmed)");
        self.state = ImeModeState::Hiragana;
        self.confirmed = false;
        self.last_vk_send_ms = crate::hook::current_tick_ms();
    }

    /// `IMC_GETCONVERSIONMODE` の結果を反映する。
    ///
    /// `None` = タイムアウト / IME ウィンドウなし（belief は変更しない）。
    pub(crate) fn on_conversion_mode_read(&mut self, mode: Option<u32>) {
        let Some(mode) = mode else {
            tracing::debug!("[ime-mode] IMC_GETCONVERSIONMODE: None (timeout / no IME window)");
            return;
        };
        let new_state = ImeModeState::from_conversion_mode(mode);
        match (self.state, new_state == self.state) {
            (ImeModeState::Unknown, _) => {
                tracing::debug!("[ime-mode] initial confirm: {new_state:?} (conv=0x{mode:08X})");
            }
            (_, false) => {
                tracing::warn!(
                    "[ime-mode] drift detected: belief={:?} → actual={:?} (conv=0x{:08X})",
                    self.state,
                    new_state,
                    mode
                );
            }
            (_, true) => {
                tracing::debug!("[ime-mode] confirmed: {new_state:?} (conv=0x{mode:08X})");
            }
        }
        self.state = new_state;
        self.confirmed = true;
    }

    /// `IMC_GETCONVERSIONMODE` の結果を「参考値」として反映する（BUG-59）。
    ///
    /// `on_conversion_mode_read` との違い: `state` は更新するが `confirmed` は
    /// 変更しない。呼び出し元は「これは実際に compose 可能かの確認ではなく、
    /// cold 判定を早めるためのヒント」という前提で呼ぶこと。
    ///
    /// 用途: `platform.rs::gji_on_focus_change` が FocusChange 直後に投げる
    /// 1回限りの参考ポーリング。この結果で `confirmed=true` を立てると、
    /// `is_native_ready()` を見る `Output::ms_ime_gate_defer`（BUG-13 の
    /// confirm-then-transmit ゲート）が「安全に送信してよい」と誤認し、
    /// フォーカス変更直後の TSF compose sink 未準備な状態へ romaji を
    /// 即送信してしまい先頭文字がリテラル化した（BUG-59）。`state` だけ
    /// 更新すれば、他の消費者（`mod.rs` の Unicode cold-start 観測ゲート等）
    /// にはヒントとして反映されつつ、`is_native_ready()` は
    /// `start_ms_ime_ready_poll`（`on_conversion_mode_read` を呼ぶ真の
    /// confirm-then-transmit ポーリング）が実際に確認するまで `false` のまま
    /// になる。
    pub(crate) fn on_conversion_mode_hint(&mut self, mode: Option<u32>) {
        let Some(mode) = mode else {
            return;
        };
        let new_state = ImeModeState::from_conversion_mode(mode);
        if new_state != self.state {
            tracing::debug!(
                "[ime-mode] cold hint: {:?} → {new_state:?} (conv=0x{mode:08X}, unconfirmed)",
                self.state
            );
            self.state = new_state;
        }
    }

    /// フォーカス変更時に呼ぶ。
    ///
    /// VK_IME_ON/OFF 送信から 100ms 以内の場合は Chrome の副作用 FocusChange と判断し
    /// state を維持したまま `confirmed` のみクリアする。
    /// それ以外は Unknown に戻して次の `IMC_GETCONVERSIONMODE` で再確認する。
    pub(crate) fn on_focus_changed(&mut self, now_ms: u64) {
        let since_vk_ms = now_ms.saturating_sub(self.last_vk_send_ms);
        if self.last_vk_send_ms > 0 && since_vk_ms < 100 {
            tracing::debug!(
                "[ime-mode] FocusChange ({}ms after VK send) → confirmed=false のみ（state={:?} 維持）",
                since_vk_ms, self.state
            );
            self.confirmed = false;
            return;
        }
        tracing::debug!("[ime-mode] FocusChange → Unknown");
        self.state = ImeModeState::Unknown;
        self.confirmed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{ImeModeFsm, ImeModeState};

    /// ADR-084 P1（BUG-49）: `unconfirm()` は `state` を変えないまま
    /// `is_native_ready()` を false に落とす。`kp_shift_conv_guard_key_down`
    /// （`runtime/key_pipeline.rs`）が MS-IME 経路で conv=0x0000 を先回り書き込む
    /// 直前に同期的に `unconfirm("shift-conv-guard entry")` を呼ぶことで、
    /// `Output::ms_ime_gate_defer` が stale な `is_native_ready()==true` を
    /// 信じて素通しするのを防ぐ（BUG-49 の直接の原因）。この不変条件が壊れたら
    /// key_pipeline.rs 側の修正は無意味になるため、ここで直接固定する。
    #[test]
    fn unconfirm_makes_native_ready_false_without_changing_state() {
        let mut fsm = ImeModeFsm::new();
        fsm.on_conversion_mode_read(Some(
            crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_ROMAN,
        ));
        assert_eq!(fsm.state(), ImeModeState::Hiragana);
        assert!(fsm.is_native_ready(), "confirmed かつ Hiragana なら ready");

        fsm.unconfirm("shift-conv-guard entry");
        assert_eq!(
            fsm.state(),
            ImeModeState::Hiragana,
            "unconfirm は state を変えない（conv が実際に半角英数へ変わったことを \
             belief に先読みで反映するわけではなく、あくまで「確認が必要」を示すだけ）"
        );
        assert!(
            !fsm.is_native_ready(),
            "unconfirm 後は state が Hiragana のままでも ready ではない"
        );
    }

    /// `unconfirm()` は複数回呼んでも安全（冪等）。エントリ時とその後の
    /// 再確認失敗時に重ねて呼ばれても panic せず、常に unconfirmed のままになる。
    #[test]
    fn unconfirm_is_idempotent() {
        let mut fsm = ImeModeFsm::new();
        fsm.unconfirm("first");
        fsm.unconfirm("second");
        assert!(!fsm.is_confirmed());
    }

    #[test]
    fn on_conversion_mode_read_confirms_native_ready_again() {
        let mut fsm = ImeModeFsm::new();
        fsm.on_conversion_mode_read(Some(
            crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_ROMAN,
        ));
        fsm.unconfirm("test");
        assert!(!fsm.is_native_ready());

        // IMC ポーリングが実際に NATIVE を確認し直すと ready に戻る。
        fsm.on_conversion_mode_read(Some(
            crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_ROMAN,
        ));
        assert!(fsm.is_native_ready());
    }

    /// BUG-59: FocusChange 直後の cold 判定用ポーリング（`on_conversion_mode_hint`）は
    /// `state` を更新しても `confirmed` を立てない。これが崩れると
    /// `Output::ms_ime_gate_defer`（BUG-13 の confirm-then-transmit ゲート）が
    /// 「安全に送信してよい」と誤認し、フォーカス変更直後の未準備な TSF
    /// compose sink へ romaji を即送信して先頭文字がリテラル化する
    /// （実機ログで2件確認: フォーカス変更 69 秒後 / 975ms 後の双方で再現）。
    #[test]
    fn conversion_mode_hint_updates_state_without_confirming() {
        let mut fsm = ImeModeFsm::new();
        assert_eq!(fsm.state(), ImeModeState::Unknown);
        assert!(!fsm.is_native_ready());

        fsm.on_conversion_mode_hint(Some(
            crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_ROMAN,
        ));
        assert_eq!(
            fsm.state(),
            ImeModeState::Hiragana,
            "ヒントでも state は更新される（Unicode cold-start 観測ゲート等の消費者向け）"
        );
        assert!(
            !fsm.is_native_ready(),
            "ヒントだけでは is_native_ready() が true になってはいけない — \
             ms_ime_gate_defer が誤って即送信を許可してしまう（BUG-59）"
        );

        // 真の confirm-then-transmit ポーリング（on_conversion_mode_read）が
        // 実際に確認して初めて ready になる。
        fsm.on_conversion_mode_read(Some(
            crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_ROMAN,
        ));
        assert!(fsm.is_native_ready());
    }

    /// ヒントは `None`（IMC 読み取り失敗）でも panic せず、state を変えない。
    #[test]
    fn conversion_mode_hint_ignores_none() {
        let mut fsm = ImeModeFsm::new();
        fsm.on_conversion_mode_hint(None);
        assert_eq!(fsm.state(), ImeModeState::Unknown);
        assert!(!fsm.is_native_ready());
    }
}
