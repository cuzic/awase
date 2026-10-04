//! Pure policy helpers for journal lane classification, byte-budget selection,
//! and diagnostic suppression decisions.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaneKind {
    State,
    Timing,
    Actuation,
    KeyInput,
}

impl LaneKind {
    #[must_use]
    /// 不具合報告に載せる打鍵（KeyInput）の範囲（ADR-222 / Opus round2 B-E1）。
    ///
    /// 打鍵 ring は「最大頻度で 10 分が溢れない」容量（8,192 件）で、通常の頻度では
    /// 10 分をはるかに超えて何時間ぶんも溜まる。所有者が許容したのは「直近 10 分」の
    /// 全打鍵なので、ダンプ時に KeyInput だけをこの窓に絞る。
    pub const REPORT_KEY_INPUT_WINDOW_MS: u64 = 10 * 60 * 1000;

    /// 打鍵 entry が、ダンプ時点 `now_ms` から `window_ms` 以内か。
    ///
    /// 畳み込まれた自動リピート（ADR-169）は、envelope の `elapsed_ms` が最初の押下のまま
    /// `last_elapsed_ms` だけが進む。11 分前に押し始めて今も押している打鍵を落とさないよう、
    /// 新しい方（`max`）で判定する。
    #[must_use]
    pub const fn key_input_in_report_window(
        elapsed_ms: u64,
        last_elapsed_ms: u64,
        now_ms: u64,
        window_ms: u64,
    ) -> bool {
        let newest = if last_elapsed_ms > elapsed_ms {
            last_elapsed_ms
        } else {
            elapsed_ms
        };
        newest >= now_ms.saturating_sub(window_ms)
    }

    /// ADR-222: 不具合報告は ring の中身を全部ダンプする（旧: バイト配分で絞っていた）。
    ///
    /// 打鍵は最大頻度（実測 1 分最大 475 件、`awase.log.old` の `journal: key input`
    /// 行の件数）で 10 分ぶん（約 4,750 件）が溢れない 8,192 件にする。
    /// 他レーンは、打鍵の多い時間帯に 10 分前後を保てるよう旧値の 2〜4 倍にする
    /// （常用時の実測では 15.6 時間の稼働で追い出し件数が state 2,676 /
    /// timing 6,626 / actuation 2,200 で、頻度は打鍵の 1/2〜1/8 程度）。
    pub const fn capacity(self) -> usize {
        match self {
            Self::State | Self::Timing => 2048,
            Self::Actuation => 1024,
            Self::KeyInput => 8192,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeTickFacts {
    pub state_changed: bool,
    pub needs_composition_reset: bool,
    pub has_gji_response: bool,
    pub learned_tsf: bool,
    pub completed: bool,
    pub terminal_timer: bool,
    pub is_first_tick: bool,
}

impl ProbeTickFacts {
    #[must_use]
    pub const fn is_notable(self) -> bool {
        self.state_changed
            || self.needs_composition_reset
            || self.has_gji_response
            || self.learned_tsf
            || self.completed
            || self.terminal_timer
            || self.is_first_tick
    }
}

#[must_use]
pub fn literal_detect_is_notable(record: &crate::tsf::literal_facts::LiteralDetectRecord) -> bool {
    use crate::tsf::literal_facts::LiteralVerdict;
    match record.facts.verdict {
        LiteralVerdict::SuspectedLiteral
        | LiteralVerdict::StaleConfirm
        | LiteralVerdict::VetoExpired
        | LiteralVerdict::PlanSkippedLiteral
        | LiteralVerdict::AbortedNoVerdict => true,
        LiteralVerdict::CompositionConfirmed if record.session_marked => true,
        LiteralVerdict::CompositionConfirmed => record.consecutive_before > 0 || record.gave_up,
        LiteralVerdict::SessionSkip => record.consecutive_before > 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredRecoveryFlushFacts {
    /// `raw_recovery` / `drain_before_send` のどちらの trigger でも、
    /// 実際に VK が flush された場合のみ notable とする。
    Flushed {
        vk_count: usize,
    },
    DiscardedStale,
    SkippedWhilePolling,
}

#[must_use]
pub const fn deferred_recovery_flush_is_notable(f: DeferredRecoveryFlushFacts) -> bool {
    match f {
        DeferredRecoveryFlushFacts::Flushed { vk_count } => vk_count > 0,
        DeferredRecoveryFlushFacts::DiscardedStale
        | DeferredRecoveryFlushFacts::SkippedWhilePolling => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderViolation {
    pub index: usize,
    pub previous: u64,
    pub current: u64,
}

/// `tokens` を先頭から走査し、単調増加が破れた最初の箇所を返す。
///
/// `&[u64]` ではなく `impl IntoIterator<Item = u64>` を取る（2026-09-03
/// code review指摘で変更）: 呼び出し元（`output/mod.rs::
/// flush_pending_deferred_vks`）は元々 `DeferredVk` の列から `order_token`
/// だけを `Vec<u64>` へ collect してから渡していたが、これは
/// flush のたびに（違反が無い共通ケースでも）ヒープ確保が発生していた。
/// イテレータを直接受けることで、呼び出し元は `vks.iter().map(|vk|
/// vk.order_token)` をそのまま渡せ、中間 `Vec` を経由しない。
#[must_use]
pub fn order_violation(tokens: impl IntoIterator<Item = u64>) -> Option<OrderViolation> {
    let mut iter = tokens.into_iter().enumerate();
    let (_, mut previous) = iter.next()?;
    for (index, current) in iter {
        if current <= previous {
            return Some(OrderViolation {
                index,
                previous,
                current,
            });
        }
        previous = current;
    }
    None
}

// ── KeyInput auto-repeat 畳み込み（ADR-169） ────────────────────────────────
//
// `journal.rs::DecisionKind`/`PhysicalDispositionSummary` は `#[cfg(windows)]`
// 配下（`journal` モジュール自体がゲートされている）のため、Windows非依存で
// あるべき本モジュールから直接参照できない。判定に必要な形だけをここに
// 局所的に再定義し、`journal.rs` 側で実際の型からこちらへ変換して渡す。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInputDecisionShape {
    PassThrough,
    PassThroughWith { effect_count: usize },
    Consume { effect_count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInputPhysicalShape {
    Allow,
    Suppress { reason: &'static str },
}

/// auto-repeat 畳み込み判定に必要な、1件の `KeyInput` エントリの識別情報。
///
/// `state_before`/`state_after` は `&str` で借用する（全打鍵が通るホットパス
/// `key_pipeline.rs` での比較のためだけに `String` を clone しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInputIdentity<'a> {
    pub vk_code: u16,
    pub scan_code: u32,
    /// `true` = KeyDown、`false` = KeyUp。
    ///
    /// **畳み込み判定に必須。** `was_down`（`hook.rs::HOOK_STATE.
    /// physical_key_state` の `swap` 由来）は KeyDown/KeyUp 両方のイベントで
    /// 「このイベント直前の物理押下状態」を返す——ごく普通の1回のタップ
    /// （KeyDown→KeyUp）でも、KeyUp 時点では直前は「押されていた」ので
    /// `was_down: true` になる。これは auto-repeat（KeyDown が連続する）の
    /// 検出とは全く別の事実であり、`is_down` を識別情報に含めて
    /// KeyDown同士でなければ絶対に畳み込まないようにしないと、通常の
    /// 1タップが「押しっぱなしで一度も離されていない」という誤った
    /// journal 記録に化ける（`coalesce_key_input` 側の追加ガードと二重に
    /// 防御する）。
    pub is_down: bool,
    /// **畳み込み判定に必須。** `coalesce_key_input` は `next`（これから
    /// 記録するイベント）が非 injected であることは引数 `next_injected`
    /// で確認するが、`KeyInputIdentity` にこのフィールドが無いと**直前
    /// （`prev`）のエントリが injected かどうか**を一切確認できない。
    /// foreign-injected な KeyDown（BUG-90/issue #136、PowerToys Mouse
    /// Without Borders 等）が偶然レーン末尾に居るとき、その直後に届いた
    /// **本物の**物理 auto-repeat（`next_injected: false`）が、他フィールド
    /// 一致だけで injected エントリへ誤って畳み込まれてしまう
    /// （`injected` フラグの信頼性が診断上の生命線である BUG-90 系の
    /// 保護を破る）。`PartialEq` 比較に含めることで、`prev.injected`
    /// と `next.injected`（常に `false`）の不一致により自動的に
    /// 畳み込み対象外になる。
    pub injected: bool,
    pub key_class: &'static str,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub state_before: &'a str,
    pub state_after: &'a str,
    pub decision: KeyInputDecisionShape,
    pub physical: KeyInputPhysicalShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoalesceOutcome {
    NewEntry,
    MergeIntoPrevious,
}

/// 直近に記録した `KeyInput`（`prev`）と、これから記録しようとしている
/// `KeyInput`（`next`）を比較し、OS auto-repeat として1エントリへ畳み込んで
/// よいかを判定する（ADR-169）。
///
/// 畳み込む条件は次のすべてを満たす場合のみ:
/// - `prev` が存在する（レーン先頭ではない）
/// - `next_injected` が false（foreign-injected な連続 down を誤って
///   auto-repeat とみなさない。BUG-90/issue #136 対策）
/// - `next_was_down` が true（`hook.rs::HOOK_STATE.physical_key_state` の
///   `swap` で得た、このイベント直前の物理押下状態。同一 vk の押下が
///   間に key-up を挟まず連続することは、物理的に OS auto-repeat 以外では
///   起こり得ない）
/// - `prev` と `next` の識別情報（vk/scan/key_class/修飾キー/NICOLA状態/
///   decision/physical）が完全一致（1つでも異なれば「auto-repeatだが
///   診断上意味のある変化点」として畳み込まない）
#[must_use]
pub fn coalesce_key_input(
    prev: Option<&KeyInputIdentity>,
    next: &KeyInputIdentity,
    next_was_down: bool,
    next_injected: bool,
) -> CoalesceOutcome {
    // OS auto-repeat は KeyDown が連続するだけであり、KeyUp が連続すること
    // はない。`next.is_down == false`（KeyUp）を無条件に除外する——`was_down`
    // は KeyUp イベントでも「直前は押されていた」を意味するだけの通常の
    // 事実であり、auto-repeat の証拠にはならない（`KeyInputIdentity::
    // is_down` のdoc参照）。この明示ガードは、万一 `KeyInputIdentity` の
    // `PartialEq` 比較だけに頼った場合に起こりうる事故（`is_down` 以外の
    // 全フィールドが一致してしまうケースの見落とし）に対する二重の防御。
    if next_injected || !next_was_down || !next.is_down {
        return CoalesceOutcome::NewEntry;
    }
    match prev {
        // `prev.injected` の明示チェックは、foreign-injected な KeyDown
        // （BUG-90/issue #136）が偶然レーン末尾に居るときに、直後の本物の
        // auto-repeat がそれへ誤って畳み込まれるのを防ぐ（`injected` を
        // `KeyInputIdentity::PartialEq` に含めたことで `prev == next` でも
        // 実質同じ効果は得られるが、`is_down` と同じ理由で明示的に二重防御
        // する）。
        Some(prev) if prev.is_down && !prev.injected && prev == next => {
            CoalesceOutcome::MergeIntoPrevious
        }
        _ => CoalesceOutcome::NewEntry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_input_window_keeps_recent_and_drops_old_entries() {
        let w = REPORT_KEY_INPUT_WINDOW_MS;
        let now = 3_600_000;
        assert!(key_input_in_report_window(now - w, 0, now, w));
        assert!(!key_input_in_report_window(now - w - 1, 0, now, w));
        assert!(key_input_in_report_window(now, 0, now, w));
        // 起動直後（now < window）は全件が窓内。
        assert!(key_input_in_report_window(0, 0, 1_000, w));
    }

    #[test]
    fn key_input_window_uses_last_elapsed_for_coalesced_repeats() {
        // 11 分前に押し始め、今も押し続けている打鍵（elapsed は最初の押下のまま）。
        let w = REPORT_KEY_INPUT_WINDOW_MS;
        let now = 3_600_000;
        assert!(!key_input_in_report_window(now - w - 60_000, 0, now, w));
        assert!(key_input_in_report_window(
            now - w - 60_000,
            now - 1_000,
            now,
            w
        ));
    }

    #[test]
    fn probe_tick_is_notable_for_each_fact() {
        assert!(!ProbeTickFacts::default().is_notable());
        assert!(ProbeTickFacts {
            state_changed: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
        assert!(ProbeTickFacts {
            needs_composition_reset: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
        assert!(ProbeTickFacts {
            has_gji_response: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
        assert!(ProbeTickFacts {
            learned_tsf: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
        assert!(ProbeTickFacts {
            completed: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
        assert!(ProbeTickFacts {
            terminal_timer: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
        assert!(ProbeTickFacts {
            is_first_tick: true,
            ..ProbeTickFacts::default()
        }
        .is_notable());
    }

    #[test]
    fn deferred_recovery_flush_is_notable_for_informative_outcomes() {
        use DeferredRecoveryFlushFacts::{DiscardedStale, Flushed, SkippedWhilePolling};

        let cases = [
            (Flushed { vk_count: 0 }, false),
            (Flushed { vk_count: 1 }, true),
            (Flushed { vk_count: 3 }, true),
            (DiscardedStale, true),
            (SkippedWhilePolling, true),
        ];

        for (facts, expected) in cases {
            assert_eq!(
                deferred_recovery_flush_is_notable(facts),
                expected,
                "{facts:?}"
            );
        }
    }

    fn literal_record(
        verdict: crate::tsf::literal_facts::LiteralVerdict,
    ) -> crate::tsf::literal_facts::LiteralDetectRecord {
        use crate::tsf::literal_facts::{
            DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralDetectFacts,
        };
        crate::tsf::literal_facts::LiteralDetectRecord {
            cold_seq: crate::state::event_origin::Generation::INITIAL,
            facts: LiteralDetectFacts {
                verdict,
                route: DetectRoute::CheckNow,
                path: DetectPath::Word,
                target: DetectTarget::Tsf,
                vk: None,
                idx: 0,
                last_idx: 0,
                evidence: DetectEvidence::default(),
            },
            consecutive_before: 0,
            gave_up: false,
            backs: 0,
            escape_composition: false,
            session_marked: false,
            romaji: None,
        }
    }

    #[test]
    fn literal_detect_is_notable_for_failure_and_absence_verdicts() {
        use crate::tsf::literal_facts::LiteralVerdict::{
            AbortedNoVerdict, PlanSkippedLiteral, StaleConfirm, SuspectedLiteral, VetoExpired,
        };
        for verdict in [
            SuspectedLiteral,
            StaleConfirm,
            VetoExpired,
            PlanSkippedLiteral,
            AbortedNoVerdict,
        ] {
            assert!(literal_detect_is_notable(&literal_record(verdict)));
        }
    }

    #[test]
    fn literal_detect_suppresses_healthy_confirm_repetition() {
        use crate::tsf::literal_facts::LiteralVerdict;
        assert!(!literal_detect_is_notable(&literal_record(
            LiteralVerdict::CompositionConfirmed
        )));

        let mut session_marked = literal_record(LiteralVerdict::CompositionConfirmed);
        session_marked.session_marked = true;
        assert!(literal_detect_is_notable(&session_marked));

        let mut recovering = literal_record(LiteralVerdict::CompositionConfirmed);
        recovering.consecutive_before = 1;
        assert!(literal_detect_is_notable(&recovering));

        let mut gave_up = literal_record(LiteralVerdict::CompositionConfirmed);
        gave_up.gave_up = true;
        assert!(literal_detect_is_notable(&gave_up));
    }

    #[test]
    fn literal_detect_session_skip_only_records_during_failure_chain() {
        use crate::tsf::literal_facts::LiteralVerdict;
        assert!(!literal_detect_is_notable(&literal_record(
            LiteralVerdict::SessionSkip
        )));

        let mut recovering = literal_record(LiteralVerdict::SessionSkip);
        recovering.consecutive_before = 1;
        assert!(literal_detect_is_notable(&recovering));
    }

    #[test]
    fn order_violation_accepts_empty_singleton_and_increasing_sequences() {
        for tokens in [vec![], vec![1], vec![1, 2, 3, 4]] {
            assert_eq!(order_violation(tokens), None);
        }
    }

    #[test]
    fn order_violation_detects_non_increasing_sequences() {
        let cases = [
            (
                vec![1, 2, 4, 3],
                OrderViolation {
                    index: 3,
                    previous: 4,
                    current: 3,
                },
            ),
            (
                vec![1, 3, 2],
                OrderViolation {
                    index: 2,
                    previous: 3,
                    current: 2,
                },
            ),
            (
                vec![1, 2, 2],
                OrderViolation {
                    index: 2,
                    previous: 2,
                    current: 2,
                },
            ),
        ];

        for (tokens, expected) in cases {
            assert_eq!(order_violation(tokens), Some(expected));
        }
    }

    // ── coalesce_key_input（ADR-169） ──

    fn ctrl_hold_identity() -> KeyInputIdentity<'static> {
        KeyInputIdentity {
            vk_code: 162, // VK_LCONTROL
            scan_code: 29,
            is_down: true,
            injected: false,
            key_class: "Passthrough",
            alt: false,
            ctrl: true,
            shift: false,
            state_before: "Idle",
            state_after: "Idle",
            decision: KeyInputDecisionShape::PassThrough,
            physical: KeyInputPhysicalShape::Allow,
        }
    }

    #[test]
    fn coalesce_merges_identical_repeat_when_was_down_and_not_injected() {
        let identity = ctrl_hold_identity();
        assert_eq!(
            coalesce_key_input(Some(&identity), &identity, true, false),
            CoalesceOutcome::MergeIntoPrevious
        );
    }

    #[test]
    fn coalesce_keeps_new_entry_when_injected_even_if_was_down() {
        let identity = ctrl_hold_identity();
        assert_eq!(
            coalesce_key_input(Some(&identity), &identity, true, true),
            CoalesceOutcome::NewEntry
        );
    }

    #[test]
    fn coalesce_keeps_new_entry_when_not_was_down() {
        let identity = ctrl_hold_identity();
        assert_eq!(
            coalesce_key_input(Some(&identity), &identity, false, false),
            CoalesceOutcome::NewEntry
        );
    }

    #[test]
    fn coalesce_keeps_new_entry_when_lane_is_empty() {
        let identity = ctrl_hold_identity();
        assert_eq!(
            coalesce_key_input(None, &identity, true, false),
            CoalesceOutcome::NewEntry
        );
    }

    #[test]
    fn coalesce_keeps_new_entry_when_modifier_changes_mid_hold() {
        let prev = ctrl_hold_identity();
        let mut next = prev;
        next.shift = true; // 例: 押しっぱなしの途中で Shift を追加で押す
        assert_eq!(
            coalesce_key_input(Some(&prev), &next, true, false),
            CoalesceOutcome::NewEntry
        );
    }

    #[test]
    fn coalesce_keeps_new_entry_when_decision_changes_mid_hold() {
        let prev = ctrl_hold_identity();
        let mut next = prev;
        next.decision = KeyInputDecisionShape::Consume { effect_count: 1 };
        assert_eq!(
            coalesce_key_input(Some(&prev), &next, true, false),
            CoalesceOutcome::NewEntry
        );
    }

    #[test]
    fn coalesce_keeps_new_entry_when_fsm_state_changes_mid_hold() {
        let prev = ctrl_hold_identity();
        let mut next = prev;
        next.state_after = "PendingChar(vk=0x41)";
        assert_eq!(
            coalesce_key_input(Some(&prev), &next, true, false),
            CoalesceOutcome::NewEntry
        );
    }

    /// 回帰テスト: 通常の1タップ（KeyDown→KeyUp）が「押しっぱなしで一度も
    /// 離されていない」という誤った記録に化けないこと。
    ///
    /// `hook.rs::HOOK_STATE.physical_key_state` の `swap` は KeyDown/KeyUp
    /// 両方のイベントで「直前の物理押下状態」を返すため、ごく普通の
    /// タップの KeyUp 時点でも `was_down: true` になる（直前は押されて
    /// いたので当然。auto-repeat の証拠ではない）。`KeyInputIdentity` に
    /// `is_down` が無かった旧実装では、他フィールドが一致するだけで
    /// KeyDown を KeyUp が「畳み込んで」しまい、実質すべての単発タップで
    /// 発火する回帰だった（opus-adversarial-consult コードレビューで発見）。
    #[test]
    fn coalesce_never_merges_keyup_into_preceding_keydown_even_if_was_down() {
        let keydown = ctrl_hold_identity(); // is_down: true
        let mut keyup = keydown;
        keyup.is_down = false;
        // KeyUp 時点では `was_down`（直前の物理状態）は必ず true になる
        // （直前の KeyDown で slot が true になっているため）。この
        // `next_was_down: true` を渡しても NewEntry のままであること。
        assert_eq!(
            coalesce_key_input(Some(&keydown), &keyup, true, false),
            CoalesceOutcome::NewEntry
        );
    }

    /// 上記と対称の回帰テスト: 直前が KeyUp（例: 他キーとの入れ替わり）の
    /// 場合、次が KeyDown で `was_down: true` になっていても畳み込まない
    /// （`is_down` 不一致で `PartialEq` 自体が false になるほか、
    /// `prev.is_down` ガードでも二重に防ぐ）。
    #[test]
    fn coalesce_never_merges_keydown_into_preceding_keyup() {
        let mut keyup = ctrl_hold_identity();
        keyup.is_down = false;
        let keydown = ctrl_hold_identity(); // is_down: true
        assert_eq!(
            coalesce_key_input(Some(&keyup), &keydown, true, false),
            CoalesceOutcome::NewEntry
        );
    }

    /// 回帰テスト（`/code-review opus` round3 で発見）: 直前のレーン末尾が
    /// foreign-injected な KeyDown（BUG-90/issue #136、PowerToys Mouse
    /// Without Borders 等）だった場合、その直後に届いた**本物の**物理
    /// auto-repeat（`next_injected: false`）が、他フィールド一致だけで
    /// injected エントリへ誤って畳み込まれないこと。畳み込むと
    /// `injected` フラグの信頼性（BUG-90系診断の生命線）が壊れる。
    #[test]
    fn coalesce_never_merges_real_repeat_into_preceding_injected_entry() {
        let mut injected_prev = ctrl_hold_identity();
        injected_prev.injected = true;
        let real_next = ctrl_hold_identity(); // injected: false
        assert_eq!(
            coalesce_key_input(Some(&injected_prev), &real_next, true, false),
            CoalesceOutcome::NewEntry
        );
    }
}
