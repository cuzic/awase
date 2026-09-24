#![allow(unsafe_code)]
//! `WM_IME_NOTIFY`（`IMN_SETOPENSTATUS`/`IMN_SETCONVERSIONMODE`）を監視し、
//! 学習プロセスが自分で注入していない期間にこれらが届いたら外部からの
//! 書き込みとして扱う（ADR-196決定1b項目2）。
//!
//! TSF compartment変更通知そのもの（COMの`ITfCompartmentEventSink`）は実装
//! せず、既存の`pump_for`のメッセージループが受け取る`WM_IME_NOTIFY`
//! （IMM32互換レイヤー経由、TSF下でも発行される）を代わりに使う——専用EDIT窓
//! （`create_window`）は素のWin32窓であり、独自のCOM sinkを実装しなくても
//! この通知は届く。フルのTSF advise sinkは将来の拡張として残す。

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::MSG;

pub const WM_IME_NOTIFY: u32 = 0x0282;
const IMN_SETOPENSTATUS: usize = 0x0008;
const IMN_SETCONVERSIONMODE: usize = 0x0006;

thread_local! {
    /// ウィンドウプロシージャ（EDITのサブクラスと親窓）が受け取った
    /// `WM_IME_NOTIFY`の`(wParam, 到着時刻)`。`WM_IME_NOTIFY`は`SendMessage`で
    /// 配送されるため`PeekMessageW`がMSGとして返すことはなく、`pump_for`の
    /// ループでは観測できない（B-1）。ウィンドウプロシージャ側で積み、
    /// `drain_queued_into`で監視状態へ渡す。到着時刻を積む時点で記録するので、
    /// 取り出しが遅れても猶予窓の判定は到着時点で行われる。
    static QUEUED: RefCell<Vec<(usize, Instant)>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// 診断用: 到着した`WM_IME_NOTIFY`の全wParam（開閉・変換モード以外も含む）の履歴。
    static ARRIVAL_LOG: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// 診断用（B-1の実機検証）: これまでに届いた全`WM_IME_NOTIFY`のwParam履歴。
#[must_use]
pub fn arrival_log() -> Vec<usize> {
    ARRIVAL_LOG.with(|l| l.borrow().clone())
}

/// ウィンドウプロシージャから呼ぶ: `WM_IME_NOTIFY`を到着時刻付きで積む。
pub fn queue_notify(wparam: usize) {
    ARRIVAL_LOG.with(|l| {
        let mut l = l.borrow_mut();
        if l.len() < 256 {
            l.push(wparam);
        }
    });
    QUEUED.with(|q| {
        let mut q = q.borrow_mut();
        // 取り出されないまま溜まり続けないよう上限を置く（通常は`pump_for`が即座に取り出す）。
        if q.len() < 1024 {
            q.push((wparam, Instant::now()));
        }
    });
}

/// 積まれた通知を全て`monitor`へ渡す。`pump_for`と`mark_expected_notify`の
/// 直前（猶予窓の付け替え前に届いた通知を取りこぼさない・新しい窓へ
/// 混入させないため）に呼ぶ。
pub fn drain_queued_into(monitor: &ImeNotifyMonitor) {
    let drained = QUEUED.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for (wparam, at) in drained {
        monitor.observe_notify_at(wparam, at);
    }
}

/// `WM_IME_NOTIFY`の監視状態。専用EDIT窓の`pump_for`ループから
/// `observe_message`を毎回呼んでもらう前提（このモニタ自体はメッセージを
/// 消費しない、`DispatchMessageW`はそのまま呼び出し側が行う）。
#[derive(Debug, Default)]
pub struct ImeNotifyMonitor {
    /// 自分の書き込みが原因で通知が届く可能性がある猶予の締切。
    expect_until: Cell<Option<Instant>>,
    /// 猶予期間の外で届いた通知（＝外部からの書き込みの証拠）の累計件数。
    external_count: Cell<u32>,
    /// 直近の`mark_expected_notify`以降に届いた通知の件数
    /// （残余リスク判定・生存確認に使う）。
    notify_since_mark: Cell<u32>,
    /// 直近の`mark_expected_notify`以降に1件でも通知が届いたか（生存確認）。
    notify_observed_since_mark: Cell<bool>,
}

impl ImeNotifyMonitor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 開閉・変換モードに関する`WM_IME_NOTIFY`かどうかを判定する。
    const fn is_open_or_conv_notify(msg: &MSG) -> bool {
        msg.message == WM_IME_NOTIFY
            && matches!(msg.wParam.0, IMN_SETOPENSTATUS | IMN_SETCONVERSIONMODE)
    }

    /// `pump_for`のメッセージループから1件ずつ渡す。
    pub fn observe_message(&self, msg: &MSG) {
        if Self::is_open_or_conv_notify(msg) {
            self.observe_notify_at(msg.wParam.0, Instant::now());
        }
    }

    /// `WM_IME_NOTIFY`の`wParam`と到着時刻から観測する（開閉・変換モード以外は無視）。
    pub fn observe_notify_at(&self, wparam: usize, at: Instant) {
        if !matches!(wparam, IMN_SETOPENSTATUS | IMN_SETCONVERSIONMODE) {
            return;
        }
        self.notify_since_mark.set(self.notify_since_mark.get() + 1);
        self.notify_observed_since_mark.set(true);
        let outside_window = self.expect_until.get().is_none_or(|deadline| at > deadline);
        if outside_window {
            self.external_count.set(self.external_count.get() + 1);
        }
    }

    /// 自分の書き込みが原因で、この先`window`以内に通知が届く可能性がある
    /// ことを申告する。生存確認・残余リスク判定のカウントをリセットする。
    pub fn mark_expected_notify(&self, window: Duration) {
        self.expect_until.set(Some(Instant::now() + window));
        self.notify_since_mark.set(0);
        self.notify_observed_since_mark.set(false);
    }

    /// 生存確認（round5 S-1対応）: `read_status`で開閉・変換モードの変化が
    /// 観測されたのに、対応する通知が1件も届かなかった場合は経路停止の
    /// 疑いが強い。呼び出し側は自分の注入で実際に状態が変わったかを渡す。
    #[must_use]
    pub fn is_alive_given_status_changed(&self, status_changed: bool) -> bool {
        !status_changed || self.notify_observed_since_mark.get()
    }

    /// 直近の`mark_expected_notify`以降に届いた通知の件数
    /// （残余リスク判定、決定1b項目6の入力の一部。**向きの逆転は未実装**——
    /// `WM_IME_NOTIFY`はメッセージの種別しか運ばず、値そのものは別途
    /// `observe_imm`/`observe_tsf`で読む必要があるため、本実装では件数のみを
    /// 見る。2回以上で疑わしいと判定する閾値自体は残余リスクの緩和として
    /// 機能する）。
    #[must_use]
    pub fn notify_count_since_mark(&self) -> u32 {
        self.notify_since_mark.get()
    }

    /// 猶予期間の外で届いた通知（＝外部の証拠）の累計件数。quiet window判定・
    /// セッション中の監視（差分は呼び出し側で計算する）に使う。
    #[must_use]
    pub fn external_count(&self) -> u32 {
        self.external_count.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};

    fn notify_msg(wparam: usize) -> MSG {
        MSG {
            hwnd: HWND(std::ptr::null_mut()),
            message: WM_IME_NOTIFY,
            wParam: WPARAM(wparam),
            lParam: LPARAM(0),
            time: 0,
            pt: POINT { x: 0, y: 0 },
        }
    }

    #[test]
    fn irrelevant_messages_are_ignored() {
        let monitor = ImeNotifyMonitor::new();
        monitor.observe_message(&MSG {
            hwnd: HWND(std::ptr::null_mut()),
            message: 0x0001, // WM_CREATE等、無関係なメッセージ
            wParam: WPARAM(0),
            lParam: LPARAM(0),
            time: 0,
            pt: POINT { x: 0, y: 0 },
        });
        assert_eq!(monitor.external_count(), 0);
        assert_eq!(monitor.notify_count_since_mark(), 0);
    }

    #[test]
    fn notify_outside_expected_window_counts_as_external() {
        let monitor = ImeNotifyMonitor::new();
        // mark_expected_notifyを一度も呼んでいない状態（expect_until=None）は
        // 常に「猶予の外」として扱う。
        monitor.observe_message(&notify_msg(IMN_SETOPENSTATUS));
        assert_eq!(monitor.external_count(), 1);
        assert_eq!(monitor.notify_count_since_mark(), 1);
    }

    #[test]
    fn notify_inside_expected_window_does_not_count_as_external() {
        let monitor = ImeNotifyMonitor::new();
        monitor.mark_expected_notify(Duration::from_millis(500));
        monitor.observe_message(&notify_msg(IMN_SETCONVERSIONMODE));
        assert_eq!(monitor.external_count(), 0);
        assert_eq!(monitor.notify_count_since_mark(), 1);
        assert!(monitor.is_alive_given_status_changed(true));
    }

    #[test]
    fn liveness_fails_when_status_changed_but_no_notify_arrived() {
        let monitor = ImeNotifyMonitor::new();
        monitor.mark_expected_notify(Duration::from_millis(500));
        // 通知が一切届かないまま、状態は変わったと申告する。
        assert!(!monitor.is_alive_given_status_changed(true));
        // 状態が変わっていなければ、通知が無くても経路の生死は判定できない
        // （疑わしいとはみなさない）。
        assert!(monitor.is_alive_given_status_changed(false));
    }

    #[test]
    fn queued_notify_is_judged_by_arrival_time_not_drain_time() {
        let monitor = ImeNotifyMonitor::new();
        monitor.mark_expected_notify(Duration::from_millis(500));
        // 猶予窓の内側で到着した通知は、取り出しが窓の終了後でも外部扱いにならない。
        queue_notify(IMN_SETOPENSTATUS);
        std::thread::sleep(Duration::from_millis(1));
        drain_queued_into(&monitor);
        assert_eq!(monitor.external_count(), 0);
        assert_eq!(monitor.notify_count_since_mark(), 1);
        // 窓を張っていない状態で届いた通知は外部扱い。
        let outside = ImeNotifyMonitor::new();
        queue_notify(IMN_SETCONVERSIONMODE);
        drain_queued_into(&outside);
        assert_eq!(outside.external_count(), 1);
    }

    #[test]
    fn unrelated_wparam_is_not_open_or_conv_notify() {
        let monitor = ImeNotifyMonitor::new();
        // IMN_SETOPENSTATUS/IMN_SETCONVERSIONMODE以外（例: IMN_SETCANDIDATEPOS=0x0009）
        // は対象外。
        monitor.observe_message(&notify_msg(0x0009));
        assert_eq!(monitor.external_count(), 0);
        assert_eq!(monitor.notify_count_since_mark(), 0);
    }
}
