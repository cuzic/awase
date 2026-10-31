//! 擬似 Zoom チャット窓による BUG-217 の再現（Linux で走る）。
//!
//! 「注入キーの↓の間隔が近いと、Zoom のチャット入力欄が母音などを捨てる」ことを、修正前の送信計画（全キーを
//! 一度に）で再現し、修正後の送信計画（`PacedQueue` で 1 キーずつ、`ZOOM_KEY_HOLD_MS`）で直ることを示す。
//! 模型と出典は `support/pseudo_zoom.rs`。
//!
//! 精度の測り方: 模型の点は `key_seq` の実測 3 点（と間接の 1 点）に合わせたもの。**文の再生の一致回数**
//! （修正前 0/8・0/24、出荷後 24/24・24/24・23/24）は、模型の点に使っていない独立の観測で、二項分布として矛盾しないかを見る。

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

mod support;

use support::pseudo_zoom::{
    binomial_consistent, plan_batch, plan_for, plan_paced, ZoomModel, TIMER_GRANULARITY_MS,
    ZOOM_CHAT_POPUP, ZOOM_CHAT_TAB,
};

/// `kakuninnwoonegaisimasu.`（かくにんをおねがいします。）のキー数。
const KEYS: usize = 23;
const ROMAJI: &str = "kakuninnwoonegaisimasu.";

/// 模型は、点に使った `key_seq` の実測を再現する（これは当てはめの確認で、独立な検証ではない）。
#[test]
fn model_reproduces_the_key_seq_fit_points() {
    let m = ZoomModel::MEASURED;
    // 80 キーを間隔 g で送ったとき、届く数の期待値（実測の 2 回の平均）
    let reached = |g: f64| 80.0 * (1.0 - m.p_drop(g));
    assert!((reached(0.0) - 54.5).abs() < 1.0, "{}", reached(0.0));
    assert!((reached(2.0) - 53.0).abs() < 1.0, "{}", reached(2.0));
    assert!((reached(12.0) - 79.5).abs() < 1.0, "{}", reached(12.0));
}

/// 修正前: 全キーの↓が同時（間隔 0）で、文が全部届く確率はほぼ 0。
#[test]
fn before_the_fix_the_whole_sentence_almost_never_arrives() {
    let plan = plan_batch(KEYS);
    assert!(plan.gaps().iter().all(|g| *g == 0.0));
    assert!(plan.pass_probability(&ZoomModel::MEASURED) < 1e-3);
}

/// 修正後: `PacedQueue` が出す↓の間隔は、タイマーの粒度（約 15.6ms）に切り上がり、落ちない間隔（12ms）を超える。
#[test]
fn after_the_fix_the_paced_downs_are_spaced_by_the_timer_granularity() {
    let plan = plan_paced(KEYS, 10, TIMER_GRANULARITY_MS);
    assert_eq!(plan.downs.len(), KEYS);
    for g in plan.gaps() {
        assert!((g - TIMER_GRANULARITY_MS).abs() < 1e-9, "gap {g}");
        assert!(g >= 12.0);
    }
    assert!(plan.pass_probability(&ZoomModel::MEASURED) > 0.95);
}

/// 窓の判定: Zoom のチャットの 2 クラスだけが paced になり、ほかの窓は変わらない。修正前は全部 batch。
#[test]
fn only_the_zoom_chat_classes_are_paced_after_the_fix() {
    for class in [ZOOM_CHAT_POPUP, ZOOM_CHAT_TAB] {
        assert_eq!(
            plan_for(class, KEYS, false),
            plan_batch(KEYS),
            "{class} 修正前"
        );
        assert_ne!(
            plan_for(class, KEYS, true),
            plan_batch(KEYS),
            "{class} 修正後"
        );
    }
    for class in ["Notepad", "Chrome_WidgetWin_1", "Qt663QWindowIcon", ""] {
        assert_eq!(plan_for(class, KEYS, true), plan_batch(KEYS), "{class}");
    }
}

/// 独立の観測（文の再生の一致回数）が、模型の予測と二項分布として矛盾しない。
#[test]
fn sentence_level_observations_are_consistent_with_the_model() {
    let m = ZoomModel::MEASURED;
    let p_old = plan_for(ZOOM_CHAT_TAB, KEYS, false).pass_probability(&m);
    let p_new = plan_for(ZOOM_CHAT_TAB, KEYS, true).pass_probability(&m);
    // (出典, 一致, 試行) — BUG-217 本文
    let old: [(&str, u32, u32); 2] = [
        ("従来 8 回(2026-10-24)", 0, 8),
        ("従来 条件を交互に混ぜた 3 周×8", 0, 24),
    ];
    let new: [(&str, u32, u32); 3] = [
        ("出荷ビルド 条件を交互に混ぜた 3 周×8", 24, 24),
        ("非ブロック版 key_pacer 3×8", 24, 24),
        ("出荷ビルド 会議中の再生", 23, 24),
    ];
    for (src, k, n) in old {
        assert!(binomial_consistent(k, n, p_old), "{src}: p={p_old}");
    }
    for (src, k, n) in new {
        assert!(binomial_consistent(k, n, p_new), "{src}: p={p_new}");
    }
}

/// 決定的な乱数（xorshift64）。
struct Rng(u64);
impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1_u64 << 53) as f64
    }
}

/// 1 回の打鍵: 模型の確率でキーを捨てた結果の文字列。
fn typed(plan: &support::pseudo_zoom::Plan, rng: &mut Rng) -> String {
    let m = ZoomModel::MEASURED;
    let mut out = String::new();
    for (i, c) in ROMAJI.chars().enumerate() {
        let p = if i == 0 {
            m.background
        } else {
            m.p_drop(plan.downs[i] - plan.downs[i - 1])
        };
        if rng.next_f64() >= p {
            out.push(c);
        }
    }
    out
}

/// 症状の再現: 修正前は文字が欠けた文字列になり、修正後はほぼ毎回そのまま届く（100 回、固定の乱数列）。
#[test]
fn sampled_typing_shows_the_symptom_before_and_not_after() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let before = plan_for(ZOOM_CHAT_TAB, KEYS, false);
    let after = plan_for(ZOOM_CHAT_TAB, KEYS, true);
    let exact = |plan: &support::pseudo_zoom::Plan, rng: &mut Rng| {
        (0..100).filter(|_| typed(plan, rng) == ROMAJI).count()
    };
    let (b, a) = (exact(&before, &mut rng), exact(&after, &mut rng));
    assert!(b <= 3, "修正前に {b}/100 回そのまま届いた");
    assert!(a >= 90, "修正後に {a}/100 回しかそのまま届かなかった");
    // 欠けた文字列は元の文字の部分列（順序は保たれる）で、毎回違う場所が欠ける（落ちる場所が毎回違う症状）。
    let samples: Vec<String> = (0..20).map(|_| typed(&before, &mut rng)).collect();
    assert!(samples.iter().all(|s| s.len() < ROMAJI.len()));
    assert!(
        samples
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 10
    );
}
