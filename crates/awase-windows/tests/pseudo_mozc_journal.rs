//! journal のダンプ → 擬似 Mozc の再生（段階 2）。
//!
//! 実機の journal（`LiteralDetect` の `facts.evidence.mozc_key_seq`）から Mozc の確認の回収を取り出し、
//! 擬似 Mozc で再生して、journal の記録と同じ結末になることを確かめる。変換の推定と限界は
//! `support/journal_scenario.rs`。
//!
//! フィクスチャ `journals/pseudo_mozc/hand-made-bug225-bug226.json` は**手書き**（実機の報告ではない）。
//! BUG-225（WT、`ko`、`key_seq` 330→331）と BUG-226（LINE、cold=184、per-VK）の報告に書かれた値から、
//! ダンプと同じ形に起こした。新しい版の実機の報告 journal が手に入ったら、同じ形で足す。

mod support;

use serde_json::Value;
use support::journal_scenario::extract_cases;
use support::pseudo_mozc::TraceEvent;

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/journals/pseudo_mozc/hand-made-bug225-bug226.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("valid json")
}

#[test]
fn extracts_the_recoveries_with_the_window_class_of_the_preceding_focus() {
    let cases = extract_cases(&fixture());
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].class_name.as_deref(),
        Some("CASCADIA_HOSTING_WINDOW_CLASS")
    );
    assert_eq!(cases[1].class_name.as_deref(), Some("Qt663QWindowIcon"));
    assert_eq!(
        (cases[0].key_seq_before, cases[0].key_seq_after),
        (330, 331)
    );
}

#[test]
fn infers_the_path_from_the_recorded_recovery() {
    let cases = extract_cases(&fixture());
    // escape あり = ADR-284 の部分リテラル回収（バッチ、settle）
    let c = cases[0].inferred_config();
    assert!(c.settle && !c.per_vk);
    // escape なし・backs 1・2 文字以上 = ADR-285 の per-VK
    let c = cases[1].inferred_config();
    assert!(!c.settle && c.per_vk);
}

/// 擬似物が、journal の記録と同じ回収（backs・escape・再送）を出す。
#[test]
fn pseudo_replay_ends_with_the_recorded_recovery() {
    for case in extract_cases(&fixture()) {
        let run = case.replay();
        let last = run.events().last().cloned().cloned();
        assert_eq!(last, Some(case.recorded_recovery()), "seq {}", case.seq);
        // 回収の結果、画面は正しい文字になる。
        assert_eq!(run.text, "こ", "seq {}", case.seq);
    }
}

/// 擬似物が TIP に見せた `key_seq` の進みが、journal の記録（after − before）と一致する。
#[test]
fn pseudo_key_seq_progress_matches_the_journal() {
    let cases = extract_cases(&fixture());
    // バッチ送信: 2 文字送って TIP は 1 つ見た（進み 1）。擬似物も Settling で進み 1 を見る。
    let run = cases[0].replay();
    assert!(run.events().contains(&&TraceEvent::Settling {
        key_seq_delta: cases[0].key_seq_after - cases[0].key_seq_before
    }));
    // per-VK: 最初の 1 キーを TIP が見ない（進み 0）。擬似物は確認も settle も出さずに回収する。
    assert_eq!(cases[1].key_seq_after, cases[1].key_seq_before);
    let run = cases[1].replay();
    assert!(!run.events().iter().any(|e| matches!(
        e,
        TraceEvent::Confirmed { .. } | TraceEvent::Settling { .. }
    )));
}

/// Mozc の確認でない verdict（`mozc_key_seq` なし）や、回収でない verdict は拾わない。
#[test]
fn ignores_entries_that_are_not_mozc_recoveries() {
    let dump: Value = serde_json::from_str(
        r#"[
          {"seq":1,"elapsed_ms":1,"entry":{"LiteralDetect":{"record":{"cold_seq":1,
            "facts":{"verdict":"SuspectedLiteral","evidence":{}},"romaji":"ko","backs":1,
            "escape_composition":false}}}},
          {"seq":2,"elapsed_ms":2,"entry":{"LiteralDetect":{"record":{"cold_seq":1,
            "facts":{"verdict":"CompositionConfirmed","evidence":{"mozc_key_seq":{"before":1,"after":2}}},
            "romaji":null,"backs":0,"escape_composition":false}}}}
        ]"#,
    )
    .unwrap();
    assert!(extract_cases(&dump).is_empty());
}
