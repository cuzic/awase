//! `output/` の TSF 送信パイプライン（`assess_warmth`・`send_romaji_as_tsf_warm`）が下す「composition は温かいか」
//! 「GJI の応答待ちか」「LiteralDetect を仕掛けるか」の判断の核（FCIS F6b）。
//!
//! 元の分岐を変えずに、判断だけを純粋関数へ出した。時刻は `crate::hook::current_tick_ms()` の `u64`（ms）で既に持たれており
//! `Instant` は使われていない（`output/`・`tsf/` の本番コードに `Instant` は無い。`tsf/probe.rs` のテスト内の 3 箇所だけ）ので、事実は `u64` のまま渡す
//! （V4 の前提は満たされている。番兵値〈`elapsed_ms == u64::MAX`・`last_unicode_ms == 0`〉は元の表現を保っている。`TickMs`/`HubClock` への置き換えは本 PR では要らない）。値の読み取り
//! （`ms_since_last_send`・`gji_last_io_ms`・`tsf_gate.state()` など。いずれも Cell/atomic/RefCell の副作用の無い読み）と
//! 実行（ログ・`spawn_local`・`install_pending_tsf`）は殻（`Output`）に残る。
//!
//! ADR-156 の窓口: `assess_warmth` の呼び出し元は `send_romaji_batched_gated`・`send_romaji_as_tsf_gated` の 2 つで、
//! どちらも drain-before-send（`deferred_gate_plan`）の**後**に呼ぶ順序（e7）を保つ。本モジュールはその順序に触れない。

/// `assess_warmth` の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WarmthFacts {
    pub warm: bool,
    /// 直近の送信からの経過 ms。一度も送信していなければ `u64::MAX`。
    pub elapsed_ms: u64,
    /// GJI 戦略（F2 probe を持つ）か。
    pub needs_f2_probe: bool,
    /// `tuning::COMPOSITION_TIMEOUT_MS`。
    pub composition_timeout_ms: u64,
}

/// `assess_warmth` の決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WarmthPlan {
    pub session_expired: bool,
    pub prepend_f2_warmup: bool,
    pub reason: WarmthReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarmthReason {
    /// warm で、期限内（または一度も送信していない）。
    WarmFresh,
    /// warm だが最後の送信から `composition_timeout_ms` を超えた。
    WarmSessionExpired,
    /// cold。
    Cold,
}

#[must_use]
pub(crate) const fn plan_warmth(facts: WarmthFacts) -> WarmthPlan {
    let session_expired = facts.warm
        && facts.elapsed_ms < u64::MAX
        && facts.elapsed_ms > facts.composition_timeout_ms;
    let reason = if !facts.warm {
        WarmthReason::Cold
    } else if session_expired {
        WarmthReason::WarmSessionExpired
    } else {
        WarmthReason::WarmFresh
    };
    WarmthPlan {
        session_expired,
        prepend_f2_warmup: (!facts.warm || session_expired) && facts.needs_f2_probe,
        reason,
    }
}

/// `is_post_unicode_pending` の入力（名前付きで渡し、同じ型の `u64` の取り違えを防ぐ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostUnicodeFacts {
    /// 最後に unicode 送信した時刻。0 = 一度も送信していない。
    pub last_unicode_ms: u64,
    pub gji_last_io_ms: u64,
}

/// unicode 送信の後、GJI がまだ I/O 応答していないか（`PendingGjiConfirm`）。
/// 真なら次のキーも unicode で強制送信する（先頭 VK のリテラル化を避ける）。
#[must_use]
pub(crate) const fn is_post_unicode_pending(facts: PostUnicodeFacts) -> bool {
    facts.last_unicode_ms != 0 && facts.gji_last_io_ms <= facts.last_unicode_ms
}

/// `is_long_idle` の入力（名前付きで渡す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LongIdleFacts {
    pub now_ms: u64,
    pub gji_last_io_ms: u64,
    pub threshold_ms: u64,
}

/// 最後の GJI I/O から `threshold_ms` 以上経っているか。
#[must_use]
pub(crate) const fn is_long_idle(facts: LongIdleFacts) -> bool {
    facts.now_ms.saturating_sub(facts.gji_last_io_ms) >= facts.threshold_ms
}

/// `plan_literal_detect` の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiteralDetectFacts {
    /// `tsf_gate.state() == Probing`。
    pub gate_probing: bool,
    /// `gji_is_active_ime()`。
    pub gji_active: bool,
    pub long_idle: bool,
    pub is_tsf_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralDetectPlan {
    Install,
    Skip(LiteralDetectSkip),
}

/// 見送り理由。元の `&&` の評価順で最初に偽になった項。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralDetectSkip {
    GateNotProbing,
    GjiNotActive,
    /// 長期静止では LiteralDetector が常にタイムアウトして誤検出になる。
    LongIdle,
    TsfMode,
}

#[must_use]
pub(crate) const fn plan_literal_detect(facts: LiteralDetectFacts) -> LiteralDetectPlan {
    if !facts.gate_probing {
        LiteralDetectPlan::Skip(LiteralDetectSkip::GateNotProbing)
    } else if !facts.gji_active {
        LiteralDetectPlan::Skip(LiteralDetectSkip::GjiNotActive)
    } else if facts.long_idle {
        LiteralDetectPlan::Skip(LiteralDetectSkip::LongIdle)
    } else if facts.is_tsf_mode {
        LiteralDetectPlan::Skip(LiteralDetectSkip::TsfMode)
    } else {
        LiteralDetectPlan::Install
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 5000;

    fn w(warm: bool, elapsed_ms: u64, f2: bool) -> WarmthPlan {
        plan_warmth(WarmthFacts {
            warm,
            elapsed_ms,
            needs_f2_probe: f2,
            composition_timeout_ms: T,
        })
    }

    #[test]
    fn plan_warmth_exhaustive() {
        use WarmthReason::*;
        // (warm, elapsed, f2, session_expired, prepend, reason)
        let table = [
            (false, 0, false, false, false, Cold),
            (false, 0, true, false, true, Cold),
            (false, u64::MAX, true, false, true, Cold),
            (true, u64::MAX, true, false, false, WarmFresh), // 未送信は期限切れではない
            (true, 0, true, false, false, WarmFresh),
            (true, T, true, false, false, WarmFresh), // 境界: `>` なので期限内
            (true, T + 1, true, true, true, WarmSessionExpired),
            (true, T + 1, false, true, false, WarmSessionExpired), // MS-IME は F2 を足さない
            (true, u64::MAX - 1, true, true, true, WarmSessionExpired),
        ];
        for (warm, el, f2, expired, prepend, reason) in table {
            assert_eq!(
                w(warm, el, f2),
                WarmthPlan {
                    session_expired: expired,
                    prepend_f2_warmup: prepend,
                    reason
                },
                "warm={warm} elapsed={el} f2={f2}"
            );
        }
    }

    fn pending(last_unicode_ms: u64, gji_last_io_ms: u64) -> bool {
        is_post_unicode_pending(PostUnicodeFacts {
            last_unicode_ms,
            gji_last_io_ms,
        })
    }

    fn idle(now_ms: u64, gji_last_io_ms: u64, threshold_ms: u64) -> bool {
        is_long_idle(LongIdleFacts {
            now_ms,
            gji_last_io_ms,
            threshold_ms,
        })
    }

    #[test]
    fn post_unicode_pending_boundaries() {
        assert!(!pending(0, 0)); // 一度も unicode 送信していない
        assert!(!pending(0, 100));
        assert!(pending(100, 99));
        assert!(pending(100, 100)); // `<=` なので同時刻は応答待ち
        assert!(!pending(100, 101));
    }

    #[test]
    fn long_idle_boundaries() {
        assert!(!idle(1099, 100, 1000));
        assert!(idle(1100, 100, 1000)); // `>=`
        assert!(idle(1101, 100, 1000));
        assert!(!idle(50, 100, 1000)); // saturating（過去の値は 0 扱い）
        assert!(idle(50, 100, 0));
    }

    #[test]
    fn plan_literal_detect_exhaustive() {
        use LiteralDetectPlan::*;
        use LiteralDetectSkip::*;
        for bits in 0..16_u8 {
            let f = LiteralDetectFacts {
                gate_probing: bits & 1 != 0,
                gji_active: bits & 2 != 0,
                long_idle: bits & 4 != 0,
                is_tsf_mode: bits & 8 != 0,
            };
            // 元の `probing && gji_active && !long_idle && !is_tsf_mode`
            let original = f.gate_probing && f.gji_active && !f.long_idle && !f.is_tsf_mode;
            let plan = plan_literal_detect(f);
            assert_eq!(plan == Install, original, "{f:?}");
            // 理由は `&&` の評価順で最初に偽になった項（手書きの表。実装の if 連鎖の写しにしない）。
            let want = match (f.gate_probing, f.gji_active, f.long_idle, f.is_tsf_mode) {
                (false, _, _, _) => Skip(GateNotProbing),
                (true, false, _, _) => Skip(GjiNotActive),
                (true, true, true, _) => Skip(LongIdle),
                (true, true, false, true) => Skip(TsfMode),
                (true, true, false, false) => Install,
            };
            assert_eq!(plan, want, "{f:?}");
        }
    }
}
