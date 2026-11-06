//! Chrome×Mozc の調査（2026-11-06、所有者からの依頼を受けた研究エージェントの報告）で出た「A1(TIP にまだ届いて
//! いないだけで、いずれ届く)なら、Backspace+再送は送った順に処理されるので、最終的に正しい文字列になる」という
//! 主張を、擬似 Mozc（`PseudoMozc::first_key_delay_ms`、A1 の模型）で固定する。
//!
//! 背景: `is_mozc_compose_check_window`（`crates/awase-windows-base/src/focus/class_names.rs`）が Chrome 系を
//! 確認の対象から外している理由は「Chrome の TSF composition context の再初期化が確認の期限(300ms)を越えうるので、
//! 期限超過を Literal と誤判定して Backspace を送ると、もうすぐ正しく入る文字を壊しうる」。しかし研究報告は、
//! awase 自身が確認中は後続キーを保留する（`pending_tsf`）ため、Backspace+再送は必ず最初のキーより後に同じ
//! スレッドの入力キューへ入り、処理順も保たれるので、最初のキーが（a）後から正しく組成されても（b）本当に
//! 素通りしていても、どちらでも最終的に正しい文字列になる、と結論した。実機の `session/session.cc` の
//! `Session::Backspace`（入力中の文字がある状態のBackspaceはサーバーが消費し文書に届かない）と `Session::Revert`
//! （無い状態はアプリへ素通り）も確認済み。
//!
//! ここではこの (a) の場合——最初のキーが遅れて届くが、いずれ届く——を `first_key_delay_ms` で模し、
//! Backspace+再送のあとに最終的に正しい文字列になることを固定する。(b) の場合（本当に素通り）は
//! `PseudoMozc::cold_dropping_first_key()` を使う既存の `pseudo_mozc_repro.rs` が固定済み。
//!
//! ADR-288 で `is_mozc_compose_check_window` から Chrome 除外を外す本番コード変更を実施済み（2026-11-06）。
//! まだ実機未確認（ADR-288「残っている懸念」参照）。実機の Chrome×Mozc で `key_eaten_count`/`key_passed_count`/
//! `edit_session_fail_count` と bit0 を集め、ここで書いた予測（期限超過の例は「スレッド無応答(キュー)」か
//! 「`key_seq` 不進(本物の素通り)」のどちらかだけ）を確かめること。

mod support;

use support::pseudo_mozc::{run, Awase, PseudoMozc, TraceEvent, WINDOWS_TERMINAL};

const DEADLINE: u64 = awase_windows::tuning::MOZC_COMPOSE_CHECK_MS;

/// 遅れが期限より短ければ、Backspace は一切送らず正常に組成される（対照）。
#[test]
fn delay_under_the_deadline_composes_normally_without_any_recovery() {
    let mut pm = PseudoMozc::cold_queuing_first_key(50);
    pm.doc = "あ".to_string();
    let r = run(Awase::CURRENT, WINDOWS_TERMINAL, &mut pm, "ko");
    assert_eq!(r.text, "あこ");
    assert_eq!(pm.backspaces_issued, 0);
}

/// 本題: 遅れが期限を越えても（Chrome の再初期化を想定した模型）、Backspace ×1 + 全体の再送で最終的に
/// 正しい文字列になる。直前の文字（`あ`）も消えない。
#[test]
fn delay_over_the_deadline_still_recovers_to_the_correct_text() {
    for delay in [DEADLINE + 1, 320, 326, 400, 500] {
        let mut pm = PseudoMozc::cold_queuing_first_key(delay);
        pm.doc = "あ".to_string();
        let r = run(Awase::CURRENT, WINDOWS_TERMINAL, &mut pm, "ko");
        assert_eq!(r.text, "あこ", "delay={delay}");
        assert_eq!(pm.backspaces_issued, 1, "delay={delay}");
        // 素通りした文字が文書に残っていないこと（Backspace だけで直り、ESCAPE は要らない）。
        assert!(!r.text.contains('k'), "delay={delay}, text={}", r.text);
    }
}

/// 回収の経路が実際に「Literal 判定 → Backspace(1, escape=false) → 再送」であることを trace で確かめる
/// （escape_composition が false のままであることが、実機調査の「Chrome では ESCAPE を足すほうが危険」の
/// 前提——ESC を送っていないことをここで固定する）。
#[test]
fn delay_over_the_deadline_recovers_without_escape() {
    let mut pm = PseudoMozc::cold_queuing_first_key(326);
    let r = run(Awase::CURRENT, WINDOWS_TERMINAL, &mut pm, "ko");
    let recovered = r.events().into_iter().find_map(|e| match e {
        TraceEvent::Recovered {
            backs,
            escape,
            resent,
        } => Some((*backs, *escape, resent.clone())),
        _ => None,
    });
    assert_eq!(recovered, Some((1, false, "ko".to_string())));
}

/// 遅れが無いとき（`first_key_delay_ms` 未設定）と比べて、キュー処理の経路を足したことによる回帰がないこと。
#[test]
fn no_delay_is_unchanged_from_the_plain_healthy_case() {
    let mut pm_plain = PseudoMozc::healthy();
    let mut pm_queued = PseudoMozc::cold_queuing_first_key(0);
    let r1 = run(Awase::CURRENT, WINDOWS_TERMINAL, &mut pm_plain, "ko");
    let r2 = run(Awase::CURRENT, WINDOWS_TERMINAL, &mut pm_queued, "ko");
    assert_eq!(r1.text, r2.text);
    assert_eq!(pm_plain.backspaces_issued, pm_queued.backspaces_issued);
}
