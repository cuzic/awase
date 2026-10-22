//! `output/` の TSF 送信パイプライン（`assess_warmth`・`send_romaji_as_tsf_warm`）が下す「composition は温かいか」
//! 「LiteralDetect を仕掛けるか」の判断の核（FCIS F6b）。
//!
//! 元の分岐を変えずに、判断だけを純粋関数へ出した。時刻は `crate::hook::current_tick_ms()` の `u64`（ms）で既に持たれており
//! `Instant` は使われていない（`output/`・`tsf/` の本番コードに `Instant` は無い。`tsf/probe.rs` のテスト内の 3 箇所だけ）ので、事実は `u64` のまま渡す
//! （全数表で固定できるので V4〈事実の型に `Instant` を入れない前提チェック〉は適用しない。`TickMs`/`HubClock` への置き換えは本 PR では要らない）。値の読み取り
//! （`ms_since_last_send`・`gji_last_io_ms`・`tsf_gate.state()` など。いずれも Cell/atomic/RefCell の副作用の無い読み）と
//! 実行（ログ・`spawn_local`・`install_pending_tsf`）は殻（`Output`）に残る。
//!
//! ADR-156 の窓口: `assess_warmth` の呼び出し元は `send_romaji_batched_gated`・`send_romaji_as_tsf_gated` の 2 つで、
//! どちらも drain-before-send（`deferred_gate_plan`）の**後**に呼ぶ順序（e7）を保つ。本モジュールはその順序に触れない。

/// `assess_warmth` の入力。
///
/// かつては「warm でも最後の送信から `COMPOSITION_TIMEOUT_MS`(2000ms)を超えたら期限切れ」とする
/// `session_expired` があったが、`Output::send_keys` が `assess_warmth` より前に `mark_send()` で
/// 最終送信時刻を更新するため経過時間は常にほぼ 0ms で、成立しなかった(raw recovery の再送は
/// `send_keys` を通らないが、同時に `mark_cold_raw_tsf` で cold にするので warm にならない)。
/// 経過時間を直すと 2 秒空いた後の送信がすべて cold になり挙動が大きく変わるため、直さずに撤去した(MB-05)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarmthFacts {
    pub warm: bool,
    /// GJI 戦略（F2 probe を持つ）か。
    pub needs_f2_probe: bool,
}

/// `assess_warmth` の決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarmthPlan {
    pub prepend_f2_warmup: bool,
    pub reason: WarmthReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmthReason {
    /// warm。
    WarmFresh,
    /// cold。
    Cold,
}

#[must_use]
pub const fn plan_warmth(facts: WarmthFacts) -> WarmthPlan {
    let reason = if facts.warm {
        WarmthReason::WarmFresh
    } else {
        WarmthReason::Cold
    };
    WarmthPlan {
        prepend_f2_warmup: !facts.warm && facts.needs_f2_probe,
        reason,
    }
}

/// `is_long_idle` の入力（名前付きで渡す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongIdleFacts {
    pub now_ms: u64,
    pub gji_last_io_ms: u64,
    /// しきい値（ms）。
    pub threshold: u64,
}

/// 最後の GJI I/O から `threshold_ms` 以上経っているか。
#[must_use]
pub const fn is_long_idle(facts: LongIdleFacts) -> bool {
    facts.now_ms.saturating_sub(facts.gji_last_io_ms) >= facts.threshold
}

/// `plan_literal_detect` の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiteralDetectFacts {
    /// `tsf_gate.state() == Probing`。
    pub gate_probing: bool,
    /// `gji_is_active_ime()`。
    pub gji_active: bool,
    pub long_idle: bool,
    pub is_tsf_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralDetectPlan {
    Install,
    Skip(LiteralDetectSkip),
}

/// 見送り理由。元の `&&` の評価順で最初に偽になった項。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralDetectSkip {
    GateNotProbing,
    GjiNotActive,
    /// 長期静止では LiteralDetector が常にタイムアウトして誤検出になる。
    LongIdle,
    TsfMode,
}

#[must_use]
pub const fn plan_literal_detect(facts: LiteralDetectFacts) -> LiteralDetectPlan {
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

    fn w(warm: bool, f2: bool) -> WarmthPlan {
        plan_warmth(WarmthFacts {
            warm,
            needs_f2_probe: f2,
        })
    }

    #[test]
    fn plan_warmth_exhaustive() {
        use WarmthReason::*;
        // (warm, f2, prepend, reason)
        let table = [
            (false, false, false, Cold), // MS-IME は F2 を足さない
            (false, true, true, Cold),
            (true, false, false, WarmFresh),
            (true, true, false, WarmFresh),
        ];
        for (warm, f2, prepend, reason) in table {
            assert_eq!(
                w(warm, f2),
                WarmthPlan {
                    prepend_f2_warmup: prepend,
                    reason
                },
                "warm={warm} f2={f2}"
            );
        }
    }

    fn idle(now_ms: u64, gji_last_io_ms: u64, threshold_ms: u64) -> bool {
        is_long_idle(LongIdleFacts {
            now_ms,
            gji_last_io_ms,
            threshold: threshold_ms,
        })
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
