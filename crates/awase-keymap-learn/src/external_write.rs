//! 学習窓への「自分以外からの書き込み」を直接観測する判定ロジック（ADR-196決定1b）。
//!
//! round1で提案した「固定トグルキーを1回注入して開閉の反転を見る」自己診断は、
//! リリースビルドのawaseが注入キーに一切反応しない（BUG-14ガード）ため原理的に
//! 検出できないとround2で判明し撤回された。ここでは代わりに、学習窓へ届く
//! 「自分以外からの書き込み」を直接観測する判定を、Win32 APIを持たない純粋な形で
//! 実装する（実際のフック・COM呼び出しは`awase-keymap-learn-win`側が担う）。

/// 観測した注入イベント1件の出所（ADR-196決定1b項目1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionOrigin {
    /// `LLKHF_INJECTED`が立っていない、ユーザーの物理入力。
    Physical,
    /// 注入されていて、学習プロセス自身の目印が付いている。
    SelfInjected,
    /// それ以外の注入（目印が無い、または別の目印）。awase自身によるものかは問わない。
    External,
}

/// 注入イベント1件を分類する（決定1b項目1）。
///
/// 「自分の目印を列挙して探す」規則ではなく「自分の目印が無ければ外部」という規則に
/// すること——awase本体は`INJECTED_MARKER`以外にも`TSF_MARKER`（warmup）・
/// `IME_KANJI_MARKER`（漢字キーactuation）を使い分けており、前者だけを探す規則では
/// 後2つを見落とす。
#[must_use]
pub const fn classify_injection(
    is_injected: bool,
    extra_info: usize,
    self_marker: usize,
) -> InjectionOrigin {
    if !is_injected {
        InjectionOrigin::Physical
    } else if extra_info == self_marker {
        InjectionOrigin::SelfInjected
    } else {
        InjectionOrigin::External
    }
}

/// フック（またはTSF通知経路）の生存確認（決定1b項目4・項目2）。
///
/// 「検出が無いことは外部の書き込みが無いことと区別できない」ため、経路が
/// 黙って停止していないかを確認する。学習プロセスが自分で注入した回数
/// （`sent`）と、その経路で実際に自分の注入として観測できた回数（`observed`）を
/// 比較し、1件でも観測漏れがあれば経路が停止しているとみなす。
#[derive(Debug, Clone, Copy, Default)]
pub struct LivenessCounter {
    sent: u32,
    observed: u32,
}

impl LivenessCounter {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sent: 0,
            observed: 0,
        }
    }

    /// 学習プロセスが自分の注入を1件送ったことを記録する。
    pub const fn mark_sent(&mut self) {
        self.sent += 1;
    }

    /// その経路で自分の注入が1件観測できたことを記録する。
    pub const fn mark_observed(&mut self) {
        self.observed += 1;
    }

    /// 送った数だけ観測できていれば経路は生きている。
    #[must_use]
    pub const fn is_alive(&self) -> bool {
        self.observed >= self.sent
    }

    #[must_use]
    pub const fn sent(&self) -> u32 {
        self.sent
    }

    #[must_use]
    pub const fn observed(&self) -> u32 {
        self.observed
    }
}

/// 残余リスクの緩和（決定1b項目6）。
///
/// IMMを直接呼ぶ外部書き込みが測定の窓の中に入った場合は、項目1・2（測定と測定の
/// **間**を対象とする）では検出できない。1回の注入に対して、開閉・変換モードの
/// compartment変更通知が2回以上、または向きが逆転して届いたら、その試行を無効に
/// する（系統的バグの検出ではなく、あくまで緩和策）。
#[must_use]
pub const fn is_measurement_suspicious(notification_count: u32, direction_reversed: bool) -> bool {
    notification_count >= 2 || direction_reversed
}

/// 測定区間の汚染判定（決定1b項目5・[ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)
/// 項目2）。
///
/// 外部からの書き込み（フック＋IME通知経由）・ユーザーの物理入力
/// （`LLKHF_INJECTED`無し）・学習窓からのフォーカス喪失のいずれか1つでも
/// 測定区間内に観測されたら、その試行は汚染されたとみなす（無効化して
/// `SessionMonitor::record_invalidated_trial`へ記録する）。3種のうちどれが
/// 原因かは呼び出し側（Win32依存のカウンタ差分・`GetFocus`比較）が判定し、
/// このブール値だけをここへ渡す。
#[must_use]
pub const fn trial_contaminated(
    external_changed: bool,
    physical_changed: bool,
    focus_lost: bool,
) -> bool {
    external_changed || physical_changed || focus_lost
}

/// [ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)項目2
/// （opus-adversarial-consult round1 m2対応）: `trial_contaminated`の3入力の
/// うち、累計カウンタ由来の2つ（外部からの書き込み・物理入力）が「前回の
/// 呼び出し以降に変化したか」を判定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub external: bool,
    pub physical: bool,
    pub focus_lost: bool,
}

impl Verdict {
    #[must_use]
    pub const fn contaminated(&self) -> bool {
        trial_contaminated(self.external, self.physical, self.focus_lost)
    }
}

/// [ADR195-T7](../../../../docs/tasks/adr195-t7-safety-measures.md)項目2
/// （round1 m2対応）: 汚染判定に使う3種の累計カウンタのbaseline管理を1箇所に
/// まとめる。以前は`RealImeDriver`がquiet window判定・
/// `check_session_interference`の両方で同じ差分ロジックを重複して書いていて
/// （baselineの取り違え・設定漏れのリスクがあり、しかもその結線部分自体は
/// Win32依存のためLinux上でテストできなかった）、この構造体へ切り出すことで
/// 呼び出し側（`RealImeDriver`）は値を取ってきて`observe`へ渡すだけにする。
///
/// `external`/`physical`/`focus_events`は呼び出し側で単調増加する累計カウンタ
/// （フック等からの生の合計値）を渡す前提。`focus_intact_now`は瞬時の
/// フォーカス確認結果（`GetFocus`/`GetForegroundWindow`の比較）を毎回渡す
/// 前提（baselineの対象外——「今」の状態しか意味を持たないため差分を取らない）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterferenceTracker {
    external: u32,
    physical: u32,
    focus_events: u32,
}

impl InterferenceTracker {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            external: 0,
            physical: 0,
            focus_events: 0,
        }
    }

    /// 現在の3種の累計値・瞬時のフォーカス確認結果から汚染を判定し、判定に
    /// 使ったのと同じ値でbaselineを前進させる（round1 m1対応: 判定後に改めて
    /// 読み直した値をbaselineにすると、その間に来たイベントを取りこぼす）。
    pub const fn observe(
        &mut self,
        external_total: u32,
        physical_total: u32,
        focus_events_total: u32,
        focus_intact_now: bool,
    ) -> Verdict {
        let verdict = Verdict {
            external: external_total != self.external,
            physical: physical_total != self.physical,
            focus_lost: focus_events_total != self.focus_events || !focus_intact_now,
        };
        self.external = external_total;
        self.physical = physical_total;
        self.focus_events = focus_events_total;
        verdict
    }
}

/// セッション中の監視（決定1b項目5）。
///
/// 測定と測定の間の待ち時間に外部からの書き込みが検出されたら、その試行を
/// 無効化する。無効化がN回を超えたら、セッション全体を失敗として終了し、
/// 表を書き出さない。Nの実測値は`.claude/rules/tuning-constants.md`に従って
/// 別途確定する（暫定値は呼び出し側が`SessionMonitor::new`へ渡す）。
#[derive(Debug, Clone, Copy)]
pub struct SessionMonitor {
    invalidation_limit: u32,
    invalidated_trials: u32,
}

impl SessionMonitor {
    #[must_use]
    pub const fn new(invalidation_limit: u32) -> Self {
        Self {
            invalidation_limit,
            invalidated_trials: 0,
        }
    }

    /// 1回分の試行が外部からの書き込みで無効化されたことを記録する。
    /// セッション全体を失敗にすべきなら`true`を返す。
    pub const fn record_invalidated_trial(&mut self) -> bool {
        self.invalidated_trials += 1;
        self.invalidated_trials > self.invalidation_limit
    }

    #[must_use]
    pub const fn invalidated_trials(&self) -> u32 {
        self.invalidated_trials
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELF_MARKER: usize = 0x4C52_4E4D;
    const OTHER_MARKER: usize = 0x4B45_594D; // awase本体のINJECTED_MARKER相当

    #[test]
    fn physical_input_has_no_injected_flag() {
        assert_eq!(
            classify_injection(false, 0, SELF_MARKER),
            InjectionOrigin::Physical
        );
        // 注入フラグが立っていなければ、たとえ自分の目印と同じ値が偶然extra_infoに
        // 入っていても物理入力として扱う（injectedフラグが優先）。
        assert_eq!(
            classify_injection(false, SELF_MARKER, SELF_MARKER),
            InjectionOrigin::Physical
        );
    }

    #[test]
    fn self_injected_only_when_marker_matches() {
        assert_eq!(
            classify_injection(true, SELF_MARKER, SELF_MARKER),
            InjectionOrigin::SelfInjected
        );
    }

    #[test]
    fn other_awase_markers_are_external_not_self() {
        // awase本体のINJECTED_MARKER/TSF_MARKER/IME_KANJI_MARKERは、学習プロセス
        // 自身の目印と異なるため、いずれも「外部」に分類されること
        // （「awaseの目印を列挙して探す」規則の禁止、round1 M-A対応）。
        assert_eq!(
            classify_injection(true, OTHER_MARKER, SELF_MARKER),
            InjectionOrigin::External
        );
        assert_eq!(
            classify_injection(true, 0x4B45_5946, SELF_MARKER),
            InjectionOrigin::External
        );
        assert_eq!(
            classify_injection(true, 0x4B45_594A, SELF_MARKER),
            InjectionOrigin::External
        );
    }

    #[test]
    fn injected_with_no_marker_is_external() {
        assert_eq!(
            classify_injection(true, 0, SELF_MARKER),
            InjectionOrigin::External
        );
    }

    #[test]
    fn liveness_counter_detects_missed_observation() {
        let mut counter = LivenessCounter::new();
        assert!(counter.is_alive()); // 何も送っていなければ生きている扱い
        counter.mark_sent();
        assert!(!counter.is_alive()); // 送ったがまだ観測されていない
        counter.mark_observed();
        assert!(counter.is_alive());
        counter.mark_sent();
        counter.mark_sent();
        counter.mark_observed();
        // 2件送って1件しか観測されていない → 経路停止の疑い
        assert!(!counter.is_alive());
    }

    #[test]
    fn suspicious_measurement_flags_double_or_reversed_notifications() {
        assert!(!is_measurement_suspicious(0, false));
        assert!(!is_measurement_suspicious(1, false));
        assert!(is_measurement_suspicious(2, false));
        assert!(is_measurement_suspicious(1, true));
        assert!(is_measurement_suspicious(0, true));
    }

    #[test]
    fn session_monitor_fails_session_after_limit_exceeded() {
        let mut monitor = SessionMonitor::new(2);
        assert!(!monitor.record_invalidated_trial()); // 1件目、まだ範囲内
        assert!(!monitor.record_invalidated_trial()); // 2件目、まだ範囲内（上限ちょうど）
        assert!(monitor.record_invalidated_trial()); // 3件目、上限超過でセッション失敗
        assert_eq!(monitor.invalidated_trials(), 3);
    }

    #[test]
    fn session_monitor_zero_limit_fails_on_first_invalidation() {
        let mut monitor = SessionMonitor::new(0);
        assert!(monitor.record_invalidated_trial());
    }

    #[test]
    fn trial_not_contaminated_when_nothing_changed() {
        assert!(!trial_contaminated(false, false, false));
    }

    #[test]
    fn trial_contaminated_by_external_write_alone() {
        assert!(trial_contaminated(true, false, false));
    }

    #[test]
    fn trial_contaminated_by_physical_input_alone() {
        // ADR195-T7項目2: 自分の注入以外の物理キーが混入したら、外部からの
        // 書き込みが無くても汚染とみなす。
        assert!(trial_contaminated(false, true, false));
    }

    #[test]
    fn trial_contaminated_by_focus_loss_alone() {
        // ADR195-T7項目2: フォーカスが学習窓から外れただけでも(キー入力が
        // 無くても)汚染とみなす。
        assert!(trial_contaminated(false, false, true));
    }

    #[test]
    fn trial_contaminated_by_all_three_causes_together() {
        assert!(trial_contaminated(true, true, true));
    }

    #[test]
    fn tracker_reports_no_contamination_when_nothing_changes_across_two_observations() {
        // (i) round1 m2対応: 変化なしの2回連続は非汚染。
        let mut tracker = InterferenceTracker::new();
        let v1 = tracker.observe(0, 0, 0, true);
        assert!(!v1.contaminated());
        let v2 = tracker.observe(0, 0, 0, true);
        assert!(!v2.contaminated());
    }

    #[test]
    fn tracker_baseline_advances_even_when_contaminated_so_next_observation_is_clean() {
        // (ii) round1 m2対応: 汚染と判定した回もbaselineは前進するので、
        // 直後にもう一度同じ値で観測すると非汚染に戻る。
        let mut tracker = InterferenceTracker::new();
        let v1 = tracker.observe(1, 0, 0, true);
        assert!(v1.contaminated());
        assert!(v1.external);
        let v2 = tracker.observe(1, 0, 0, true);
        assert!(
            !v2.contaminated(),
            "baselineが前進していれば同じ値の再観測は非汚染のはず"
        );
    }

    #[test]
    fn tracker_flags_each_cause_independently() {
        let mut external_only = InterferenceTracker::new();
        assert!(external_only.observe(1, 0, 0, true).contaminated());

        let mut physical_only = InterferenceTracker::new();
        assert!(physical_only.observe(0, 1, 0, true).contaminated());

        let mut focus_events_only = InterferenceTracker::new();
        assert!(focus_events_only.observe(0, 0, 1, true).contaminated());

        let mut focus_intact_false_only = InterferenceTracker::new();
        assert!(focus_intact_false_only
            .observe(0, 0, 0, false)
            .contaminated());
    }

    #[test]
    fn tracker_focus_intact_now_is_not_baseline_tracked() {
        // `focus_intact_now`は瞬時の値であり、baselineの対象外。前回`false`
        // だった後でも、今回`true`ならフォーカス起因の汚染にはならない
        // （他の原因が無ければ非汚染）。
        let mut tracker = InterferenceTracker::new();
        assert!(tracker.observe(0, 0, 0, false).contaminated());
        let v = tracker.observe(0, 0, 0, true);
        assert!(!v.contaminated());
    }
}
