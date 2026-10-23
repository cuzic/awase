//! `output/` の TSF 送信パイプライン（`assess_warmth`）が下す「composition は温かいか」
//! の判断の核（FCIS F6b）。warm パスの「LiteralDetect を仕掛けるか」の判断（`plan_literal_detect`）は、設置の条件が
//! CI で 1 度も成立しなかったため撤去した（ECRS MB-02）。
//!
//! 元の分岐を変えずに、判断だけを純粋関数へ出した。時刻は `crate::hook::current_tick_ms()` の `u64`（ms）で既に持たれており
//! `Instant` は使われていない（`output/`・`tsf/` の本番コードに `Instant` は無い。`tsf/probe.rs` のテスト内の 3 箇所だけ）ので、事実は `u64` のまま渡す
//! （全数表で固定できるので V4〈事実の型に `Instant` を入れない前提チェック〉は適用しない。`TickMs`/`HubClock` への置き換えは本 PR では要らない）。値の読み取り
//! （`ms_since_last_send`・`gji_last_io_ms` など。いずれも Cell/atomic/RefCell の副作用の無い読み）と
//! 実行（ログ・`spawn_local`・`install_pending_tsf`）は殻（`Output`）に残る。
//!
//! ADR-156 の窓口: `assess_warmth` の呼び出し元は `send_romaji_batched_gated` の 1 つ（Tsf 側の `send_romaji_as_tsf_gated` は 2026-10-22 に撤去）で、
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

// 決定表（warm × needs_f2_probe の全 4 通り）は `relay_plan.rs` の ME-3 golden
// （`tests/golden/relay_warm_plan_table.txt`）が固定する。
