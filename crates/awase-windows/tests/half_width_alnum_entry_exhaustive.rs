//! ADR-280 D2: `HalfWidthAlnumState::plan_entry_for_language_detect` の全列挙（現状の挙動の固定。挙動は変えない）。
//!
//! 入力は `entry_policy`(Off/MsImeOnly/All) 3 × `toggle_held` 2 × bool 3（`entry_ime_ok`・`uses_imc_conv_write`・
//! `pending_for_scope`）= 48 通り。期待値は実装の `if` の連なりを写さず、関数の doc の仕様
//! （「左 Shift 単独タップの entry と同じ条件で 2 経路のどちらかを選ぶ。すでに持続トグルが有効、または戻り待ちが
//! あるときは何もしない」）から、方針の表として別の形で書く。

use awase::config::HalfWidthAlnumTogglePolicy;
use awase_windows::state::foreground_scope::ForegroundScope;
use awase_windows::state::half_width_alnum::{HalfWidthAlnumEffect, HalfWidthAlnumState};

const POLICIES: [HalfWidthAlnumTogglePolicy; 3] = [
    HalfWidthAlnumTogglePolicy::Off,
    HalfWidthAlnumTogglePolicy::MsImeOnly,
    HalfWidthAlnumTogglePolicy::All,
];

/// 方針ごとに、入りを許す経路の表: (IMC write の経路で入れるか, GJI SendInput の経路で入れるか)。
const fn policy_allows(policy: HalfWidthAlnumTogglePolicy) -> (bool, bool) {
    match policy {
        HalfWidthAlnumTogglePolicy::Off => (false, false),
        HalfWidthAlnumTogglePolicy::MsImeOnly => (true, false),
        HalfWidthAlnumTogglePolicy::All => (true, true),
    }
}

/// 仕様からの期待値。
fn expected(
    policy: HalfWidthAlnumTogglePolicy,
    toggle_held: bool,
    entry_ime_ok: bool,
    uses_imc_conv_write: bool,
    pending_for_scope: bool,
) -> HalfWidthAlnumEffect {
    // 二重に入らない: 持続トグルが有効、または戻り待ちが積まれている。
    if toggle_held || pending_for_scope {
        return HalfWidthAlnumEffect::Nothing;
    }
    // 入る意味のある IME でない。
    if !entry_ime_ok {
        return HalfWidthAlnumEffect::Nothing;
    }
    let (imc, gji) = policy_allows(policy);
    if uses_imc_conv_write {
        if imc {
            HalfWidthAlnumEffect::EnterViaImcWrite
        } else {
            HalfWidthAlnumEffect::Nothing
        }
    } else if gji {
        HalfWidthAlnumEffect::EnterViaGjiSendInput
    } else {
        HalfWidthAlnumEffect::Nothing
    }
}

fn state(policy: HalfWidthAlnumTogglePolicy, toggle_held: bool) -> HalfWidthAlnumState {
    let mut st = HalfWidthAlnumState::default();
    st.set_policy(policy);
    if toggle_held {
        st.commit_enter_imc(ForegroundScope { pid: 1, hwnd: 1 });
    }
    st
}

#[test]
fn plan_entry_for_language_detect_matches_the_spec_table_for_all_48_inputs() {
    let mut cases = 0;
    let mut enter_imc = 0;
    let mut enter_gji = 0;
    for policy in POLICIES {
        for toggle_held in [false, true] {
            for entry_ime_ok in [false, true] {
                for uses_imc_conv_write in [false, true] {
                    for pending_for_scope in [false, true] {
                        cases += 1;
                        let st = state(policy, toggle_held);
                        let got = st.plan_entry_for_language_detect(
                            entry_ime_ok,
                            uses_imc_conv_write,
                            pending_for_scope,
                        );
                        let want = expected(
                            policy,
                            toggle_held,
                            entry_ime_ok,
                            uses_imc_conv_write,
                            pending_for_scope,
                        );
                        assert_eq!(
                            got, want,
                            "policy={policy:?} toggle_held={toggle_held} entry_ime_ok={entry_ime_ok} \
                             uses_imc_conv_write={uses_imc_conv_write} pending_for_scope={pending_for_scope}"
                        );
                        match got {
                            HalfWidthAlnumEffect::EnterViaImcWrite => enter_imc += 1,
                            HalfWidthAlnumEffect::EnterViaGjiSendInput => enter_gji += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    // フィールド・方針の追加で列挙が黙って欠けないように、件数を固定する。
    assert_eq!(cases, 48);
    // 入りになるのは、トグル無し・戻り待ち無し・entry_ime_ok の 3 条件が揃った 3 方針 × 2 経路のうち許される組
    // (Off: 0、MsImeOnly: IMC のみ 1、All: 2) = 3。
    assert_eq!(enter_imc, 2, "IMC write の経路で入る組");
    assert_eq!(enter_gji, 1, "GJI SendInput の経路で入る組");
}

#[test]
fn state_helper_really_sets_toggle_held() {
    // `toggle_held` を立てる手順（commit_enter_imc）が効いていること（全列挙が空振りしないための確認）。
    let st = state(HalfWidthAlnumTogglePolicy::All, true);
    assert_eq!(
        st.plan_entry_for_language_detect(true, true, false),
        HalfWidthAlnumEffect::Nothing
    );
    let st = state(HalfWidthAlnumTogglePolicy::All, false);
    assert_eq!(
        st.plan_entry_for_language_detect(true, true, false),
        HalfWidthAlnumEffect::EnterViaImcWrite
    );
}
