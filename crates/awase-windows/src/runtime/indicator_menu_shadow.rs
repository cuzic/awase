//! ADR-264 段階 1: 入力インジケーターの右クリックメニューの**影の読み**（挙動を変えない観測）。
//!
//! 利用者がタスクバーの上で右クリックしてメニューを開いたとき（`observer::indicator_menu` の WinEvent が検知）、
//! メニューの現在のモードを読み取りワーカーで読み、結果を journal（`JournalEntry::IndicatorMenu`）に残す。
//! **belief には入れない・IME へ何も書かない・打鍵は変えない**。目的は、読める窓で直接の読みとの一致率を、観測の無い窓で
//! belief とのずれを、実ジャーナルで確かめること（ADR-264 決定 5。belief への配線は ADR-261 の段階 2 と一本化して別レビュー）。
//!
//! 有効なのは `[indicator_evidence] menu` の実効の段階が `observe` 以上（既定は off のオプトイン。ADR-271）のときだけ。反映は再起動後
//! （フックと読み取りスレッドは起動時に 1 度だけ作る。`app/bootstrap.rs`）。

use std::sync::mpsc::Sender;
use std::time::Instant;

use windows::Win32::Foundation::HWND;

use crate::observer::indicator_menu::{cursor_over_taskbar, is_popup_menu_window, MenuReadRequest};
use crate::state::indicator_menu::{may_request_read, menu_ime_for, MenuIme, MenuOutcome};
use crate::win32::SendableHwnd;

/// 結果待ちの依頼に付けておく、メニューを開いた時点の文脈（結果が返ったら journal の記録に添える）。
struct Pending {
    ime: MenuIme,
    /// メニューを開いた時点の窓の IME プロファイル
    profile: &'static str,
    /// メニューを開いた時点の belief（IME が開いているか）
    belief_open: Option<bool>,
    at: Instant,
}

/// 影の読みの状態。読み取りワーカーへの送信口が無ければ（無効のとき）何もしない。
pub(super) struct IndicatorMenuShadow {
    tx: Option<Sender<MenuReadRequest>>,
    pending: Option<Pending>,
    last_request: Option<Instant>,
    /// 読みの件数（不具合報告の要約用。内容は持たない。ADR-271 D5）
    counters: crate::indicator_evidence::IndicatorMenuCounters,
    /// 起動時に固定した実効の段階（`min(設定値, ui_max_level())`）。無効のときも持つ（報告は `off` でも段階を出す）
    effective: Option<crate::indicator_evidence::EffectiveEvidence>,
}

impl IndicatorMenuShadow {
    pub(super) const fn disabled() -> Self {
        Self {
            tx: None,
            pending: None,
            last_request: None,
            counters: crate::indicator_evidence::IndicatorMenuCounters::new(),
            effective: None,
        }
    }
}

impl super::Runtime {
    /// 起動時に、読み取りワーカーへの送信口を渡して有効にする（`None` なら無効のまま）。
    pub(crate) fn init_indicator_menu(&mut self, tx: Option<Sender<MenuReadRequest>>) {
        self.indicator_menu = IndicatorMenuShadow {
            tx,
            pending: None,
            last_request: None,
            counters: crate::indicator_evidence::IndicatorMenuCounters::new(),
            effective: self.indicator_menu.effective,
        };
    }

    /// 起動時に実効の段階を固定する（有効・無効にかかわらず呼ぶ。不具合報告が `off` でも段階を出すため。ADR-271 D5）。
    pub(crate) fn set_indicator_effective_evidence(
        &mut self,
        effective: crate::indicator_evidence::EffectiveEvidence,
    ) {
        self.indicator_menu.effective = Some(effective);
    }

    /// 不具合報告の要約用に、実効の段階と読みの件数を返す（読み取りだけ）。
    pub(crate) fn indicator_evidence_summary(
        &self,
    ) -> crate::bug_report::BugReportIndicatorEvidence {
        let eff =
            self.indicator_menu
                .effective
                .unwrap_or(crate::indicator_evidence::EffectiveEvidence {
                    glyph: awase::config::EvidenceLevel::Off,
                    menu: awase::config::EvidenceLevel::Off,
                });
        crate::bug_report::BugReportIndicatorEvidence::new(
            eff,
            &self.indicator_menu.counters,
            self.indicator_glyph_counters(),
        )
    }

    /// `EVENT_SYSTEM_MENUPOPUPSTART`（`observer::indicator_menu::menu_popup_event_proc`）から呼ぶ。
    ///
    /// 前置きのフィルタ（ADR-264 決定 1）: 有効・間隔・メニュー窓のクラス・カーソルがタスクバーの上・IME が同定済み。
    /// 通ったときだけ、メニュー窓の hwnd を読み取りワーカーへ依頼する。何も変えない。
    pub(crate) fn indicator_menu_on_popup_start(&mut self, menu_hwnd: HWND) {
        let Some(tx) = self.indicator_menu.tx.clone() else {
            return;
        };
        let now = Instant::now();
        let since_last_ms = self
            .indicator_menu
            .last_request
            .map(|t| u64::try_from(now.duration_since(t).as_millis()).unwrap_or(u64::MAX));
        let pending_age_ms = self
            .indicator_menu
            .pending
            .as_ref()
            .map(|p| u64::try_from(now.duration_since(p.at).as_millis()).unwrap_or(u64::MAX));
        if !may_request_read(since_last_ms, pending_age_ms) {
            return;
        }
        // `#32768` は誰のメニューでも出るので、まずクラス、次にカーソルがタスクバーの上であること（他のアプリの
        // 右クリックメニューを UI Automation で読まない）。
        if !is_popup_menu_window(menu_hwnd) || !cursor_over_taskbar() {
            return;
        }
        let Some(ime) = menu_ime_for(crate::tsf::observer::tsf_obs().table_ime_kind()) else {
            tracing::debug!(
                "[indicator-menu] popup on taskbar, but the IME is not GJI/native MS-IME; skipped"
            );
            return;
        };
        let profile = super::lang_detect_shadow::profile_label(self.platform.current_app_profile());
        let belief_open = Some(self.platform_state.ime.effective_open());
        let req = MenuReadRequest {
            menu_hwnd: SendableHwnd(menu_hwnd),
            ime,
        };
        if tx.send(req).is_err() {
            // ワーカーが終了している。以後は何もしない。
            tracing::warn!("[indicator-menu] worker is gone; shadow disabled");
            self.indicator_menu.tx = None;
            self.indicator_menu.pending = None;
            return;
        }
        self.indicator_menu.pending = Some(Pending {
            ime,
            profile,
            belief_open,
            at: now,
        });
        self.indicator_menu.last_request = Some(now);
        tracing::debug!(
            "[indicator-menu] popup on taskbar → read requested (ime={}, profile={profile}, belief_open={belief_open:?})",
            ime.label()
        );
    }

    /// 読み取りワーカーの結果（`WM_INDICATOR_MENU_READ`）を受け、journal に残す。何も変えない。
    pub(crate) fn indicator_menu_on_result(&mut self, wparam: usize, lparam: isize) {
        let Some(outcome) = MenuOutcome::unpack(wparam, lparam) else {
            tracing::warn!(
                "[indicator-menu] corrupt result message (wparam={wparam:#x}, lparam={lparam:#x})"
            );
            return;
        };
        let Some(pending) = self.indicator_menu.pending.take() else {
            // 失われたとみなした依頼の遅れた結果、または二重配送。文脈が無いので記録しない。
            tracing::debug!("[indicator-menu] result without a pending request; dropped");
            return;
        };
        if pending.ime != outcome.ime {
            tracing::debug!("[indicator-menu] result for another IME table; dropped");
            return;
        }
        let record = outcome.to_record(pending.profile, pending.belief_open);
        self.indicator_menu.counters.record(&record);
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::IndicatorMenu { record });
    }
}
