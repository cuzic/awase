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
}
