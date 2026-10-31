//! 擬似 Mozc による BUG-225 / BUG-226 の再現（Linux で走る）。
//!
//! 目的: 「実機で起きたバグを、修正前の設定で再現し、修正後の設定で直ることを示す」型を作る。
//! 擬似物の精度は「過去のバグを修正前に再現できるか」で測る（この 2 件が最初の検証）。
//! 擬似 Mozc と確認の流れは `support/pseudo_mozc.rs`（限界もそこに書いてある）。

mod support;

use support::pseudo_mozc::{run, Awase, PseudoMozc, TraceEvent, CHROME, LINE, WINDOWS_TERMINAL};

// ───────────────────────── シナリオ ─────────────────────────

/// 対照: cold でも最初のキーを落とさない Mozc なら、どの設定でも `こ`。
#[test]
fn healthy_cold_types_correctly_in_every_config() {
    for cfg in [Awase::BEFORE_BUG225, Awase::BEFORE_BUG226, Awase::CURRENT] {
        let mut pm = PseudoMozc::healthy();
        assert_eq!(
            run(cfg, WINDOWS_TERMINAL, &mut pm, "ko").text,
            "こ",
            "{cfg:?}"
        );
        assert_eq!(pm.backspaces_issued, 0, "正常なときは何も消さない: {cfg:?}");
    }
}

/// BUG-225: 修正前は `k` が素通りして `kお` になる。
#[test]
fn bug225_reproduces_before_the_fix() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(
        run(Awase::BEFORE_BUG225, WINDOWS_TERMINAL, &mut pm, "ko").text,
        "kお"
    );
}

/// BUG-225: ADR-284（`key_seq` の進みによる回収）で直る。per-VK 化は無くても直る。
#[test]
fn bug225_is_fixed_by_adr284_settle() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(
        run(Awase::BEFORE_BUG226, WINDOWS_TERMINAL, &mut pm, "ko").text,
        "こ"
    );
    assert_eq!(pm.backspaces_issued, 1, "素通りした k の 1 文字だけ消す");
}

/// BUG-226: LINE では、ADR-284 を含む設定でも許可リストの外なので確認が張られず `kお` のまま。
#[test]
fn bug226_reproduces_in_line_with_the_allow_list() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(Awase::BEFORE_BUG226, LINE, &mut pm, "ko").text, "kお");
}

/// BUG-226: ADR-285（除外リスト + per-VK）で LINE も直る。
#[test]
fn bug226_is_fixed_by_adr285_in_line() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(Awase::CURRENT, LINE, &mut pm, "ko").text, "こ");
    assert_eq!(pm.backspaces_issued, 1);
}

/// ADR-285 の per-VK 化は、ADR-284 の settle が無くても単独で直す（素通りは最初の 1 キーだけと決まるため）。
#[test]
fn per_vk_alone_fixes_the_first_key_literal() {
    let cfg = Awase {
        settle: false,
        ..Awase::CURRENT
    };
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(cfg, LINE, &mut pm, "ko").text, "こ");
    assert_eq!(pm.backspaces_issued, 1);
}

/// Chrome 系は確認を張らない（誤判定の Backspace が正しい文字を消すおそれがあるため、実測まで外す）。
#[test]
fn chrome_is_left_alone_so_the_leak_is_not_recovered() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    assert_eq!(run(Awase::CURRENT, CHROME, &mut pm, "ko").text, "kお");
    assert_eq!(pm.backspaces_issued, 0);
}

/// 安全側: Mozc サーバーが応答できないときはキーが握りつぶされている可能性があるので、何も消さない。
#[test]
fn swallowed_keys_never_cause_a_backspace() {
    let mut pm = PseudoMozc::new(false, false);
    let _ = run(Awase::CURRENT, LINE, &mut pm, "ko");
    assert_eq!(pm.backspaces_issued, 0);
}

// ───────────────────── トレース・journal 形式の出力（段階 1） ─────────────────────

/// 経過が中立の形で残る: BUG-225（バッチ送信 + settle）は「確認 → 読み直し → 回収」の順になる。
#[test]
fn trace_of_bug225_recovery_is_confirm_then_settle_then_recover() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    let r = run(Awase::BEFORE_BUG226, WINDOWS_TERMINAL, &mut pm, "ko");
    assert_eq!(
        r.events(),
        vec![
            &TraceEvent::Sent {
                romaji: "ko".into()
            },
            &TraceEvent::Settling { key_seq_delta: 1 },
            &TraceEvent::Recovered {
                backs: 1,
                escape: true,
                resent: "ko".into()
            },
        ]
    );
    // 回収は期限(MOZC_COMPOSE_CHECK_MS)に達した tick で起きる。
    let at = r.trace.last().unwrap().0;
    assert!(at >= awase_windows::tuning::MOZC_COMPOSE_CHECK_MS);
}

/// per-VK(ADR-285)では、最初の 1 キーだけ送って回収し、残りは再送に含まれる(二重に送らない)。
#[test]
fn trace_of_per_vk_recovery_has_no_rest_sent_after_recovery() {
    let mut pm = PseudoMozc::cold_dropping_first_key();
    let r = run(Awase::CURRENT, LINE, &mut pm, "ko");
    assert_eq!(
        r.events(),
        vec![
            &TraceEvent::Sent { romaji: "k".into() },
            &TraceEvent::Recovered {
                backs: 1,
                escape: false,
                resent: "ko".into()
            },
        ]
    );
}

/// 正常なときは、確認 → 残りの順で、回収はない。
#[test]
fn trace_of_healthy_per_vk_is_confirm_then_rest() {
    let mut pm = PseudoMozc::healthy();
    let r = run(Awase::CURRENT, LINE, &mut pm, "ko");
    assert_eq!(
        r.events(),
        vec![
            &TraceEvent::Sent { romaji: "k".into() },
            &TraceEvent::Confirmed { key_seq_delta: 1 },
            &TraceEvent::SentRest { romaji: "o".into() },
        ]
    );
}

/// 擬似物の読みが、実機と同じ変換で `MozcShmRecord`(journal のエントリ)になる。
#[test]
fn pseudo_read_becomes_the_same_journal_record_as_the_real_one() {
    use awase_windows::state::mozc_shm::Trigger;
    let pm = PseudoMozc::healthy();
    let rec = pm.shm_record(Trigger::Focus, 100, 200);
    assert_eq!(rec.trigger, "focus");
    assert_eq!(rec.verdict, "accepted");
    assert_eq!(rec.effective_open, Some(true));
    assert_eq!(rec.thread_id, 100);
    assert_eq!(rec.process_id, 200);
    // journal へ直列化できる(実機のダンプと同じ解析器に載せるため)。
    let json = serde_json::to_value(&rec).expect("MozcShmRecord serializes");
    assert_eq!(json["verdict"], "accepted");
}
