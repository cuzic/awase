//! 擬似 Mozc の条件の掃引と、実測の目録（段階 3）。
//!
//! 1. 実測の目録（`support/cold_drop.rs`）の各観測を、現行の設定で擬似物が再現する。
//! 2. 目録の数では「起きる割合」を確率として使えないことを、区間で固定する（観測が増えたら外す）。
//! 3. 条件（素通りの有無 × 入力中の文字ができるまでの遅れ × 2 打目の数えの遅れ × 窓）を総当たりで掃引し、
//!    「期限(`MOZC_COMPOSE_CHECK_MS`)より前に入力中の文字ができるなら、どの条件でも正しい文字になり、
//!    直前の文字を消さない」ことを固定する。期限以降の遅れでは直前の文字を消しうる境界が実在することも固定する
//!    （Chrome 系を張らない理由、PR #809 M3 と同じ型の危険）。

mod support;

use support::cold_drop::{overall, wilson, OBSERVATIONS};
use support::pseudo_mozc::{run, Awase, PseudoMozc, LINE, WINDOWS_TERMINAL};

const DEADLINE: u64 = awase_windows::tuning::MOZC_COMPOSE_CHECK_MS;

fn class_of(window: &str) -> &str {
    match window {
        "windows-terminal-family" | "unknown" => WINDOWS_TERMINAL,
        other => other,
    }
}

/// 目録の各観測を、現行の設定で擬似物が再現する（起きたものは回収され、起きなかったものは何も消さない）。
#[test]
fn every_observation_is_handled_by_the_current_config() {
    for o in OBSERVATIONS {
        let mut pm = if o.dropped {
            PseudoMozc::cold_dropping_first_key()
        } else {
            PseudoMozc::healthy()
        };
        let r = run(Awase::CURRENT, class_of(o.window), &mut pm, "ko");
        assert_eq!(r.text, "こ", "{}", o.id);
        assert_eq!(pm.backspaces_issued, usize::from(o.dropped), "{}", o.id);
    }
}

/// 観測が少なすぎて、起きる割合を確率として使えない。観測が増えて区間が狭まったら、この試験を直す合図。
#[test]
fn the_observations_are_too_few_to_use_the_rate_as_a_probability() {
    let r = overall();
    assert_eq!((r.dropped, r.n), (2, 3));
    assert!(!r.is_conclusive(), "{r:?}");
    assert!(r.low < 0.3 && r.high > 0.9, "{r:?}");
}

#[test]
fn wilson_interval_narrows_with_more_observations() {
    let few = wilson(2, 3);
    let many = wilson(20, 30);
    assert!(many.high - many.low < few.high - few.low);
    assert!(many.is_conclusive() || many.high - many.low < 0.4);
    // n=0 は何も言えない
    let none = wilson(0, 0);
    assert!(!none.is_conclusive() && none.low == 0.0 && none.high == 1.0);
}

struct Case {
    class: &'static str,
    cfg: Awase,
    drop: bool,
    delay: u64,
    lag: u64,
}

fn sweep() -> Vec<Case> {
    let mut v = Vec::new();
    for class in [WINDOWS_TERMINAL, LINE, "Notepad"] {
        for cfg in [Awase::CURRENT, Awase::BEFORE_BUG226] {
            for drop in [false, true] {
                for delay in [20, 60, 200, 290, 400] {
                    for lag in [0, 30, 100] {
                        v.push(Case {
                            class,
                            cfg,
                            drop,
                            delay,
                            lag,
                        });
                    }
                }
            }
        }
    }
    v
}

/// 直前の文字（`あ`）を置いた文書に cold の最初の `ko` を送る。画面と、消された Backspace の数を返す。
fn run_case(c: &Case) -> (String, usize) {
    // BEFORE_BUG226 は許可リスト（WT のみ）なので、他の窓では確認が張られない。ここでは張られる窓だけを対象にする。
    let mut pm = if c.drop {
        PseudoMozc::cold_dropping_first_key()
    } else {
        PseudoMozc::healthy()
    };
    pm.compose_delay_ms = c.delay;
    pm.second_key_lag_ms = c.lag;
    pm.doc = "あ".to_string();
    let r = run(c.cfg, c.class, &mut pm, "ko");
    (r.text, pm.backspaces_issued)
}

fn armed(c: &Case) -> bool {
    if c.cfg.inverted_window_rule {
        true // 現行は Chrome 系と空以外すべて。掃引の窓はどれも張られる。
    } else {
        c.class == WINDOWS_TERMINAL
    }
}

/// 期限より前に入力中の文字ができるなら、どの条件でも正しい文字になり、直前の文字を消さない。
#[test]
fn below_the_deadline_every_condition_types_correctly_and_keeps_the_previous_char() {
    let mut checked = 0;
    for c in sweep().iter().filter(|c| armed(c) && c.delay < DEADLINE) {
        let (text, _) = run_case(c);
        assert_eq!(
            text, "あこ",
            "class={} drop={} delay={} lag={} cfg={:?}",
            c.class, c.drop, c.delay, c.lag, c.cfg
        );
        checked += 1;
    }
    assert!(checked > 50, "掃引が空に近い: {checked}");
}

/// 期限以降に入力中の文字ができる遅い TIP では、直前の文字を消す誤判定が実在する（Chrome 系を張らない理由と同じ型）。
/// 境界が消えたら（修正されたら）この試験を直す。
#[test]
fn at_or_after_the_deadline_a_slow_tip_can_make_the_recovery_delete_the_previous_char() {
    let violations = sweep()
        .iter()
        .filter(|c| armed(c) && c.delay >= DEADLINE)
        .filter(|c| run_case(c).0 != "あこ")
        .count();
    assert!(violations > 0, "期限以降の遅れで誤判定が出なくなった");
}
