//! journal のダンプ → 擬似 Mozc のシナリオ（段階 2）。
//!
//! 実機の journal（`UnifiedJournal::to_json`、封筒 `{seq, elapsed_ms, entry}` の配列）から、Mozc の確認が出した
//! `LiteralDetect`（`facts.evidence.mozc_key_seq` を持つもの）を取り出し、擬似 Mozc で同じ結末になるかを
//! 確かめられる形（[`Case`]）にする。`JournalEntry` は `Serialize` だけなので、`serde_json::Value` で読む。
//!
//! 推定の仕方（journal が記録する経路から、擬似物の宣言を決める）:
//! - 回収（`romaji` を持つ SuspectedLiteral）だけを対象にする。健全な確認は journal に畳まれて残らない。
//! - `escape_composition == true`: ADR-284 の部分リテラル回収（バッチ送信、`key_seq` の進みが足りない）。
//!   `romaji` の文字数 − `key_seq` の進み = `backs`。
//! - `escape_composition == false` で `backs == 1` かつ `romaji` が 2 文字以上: ADR-285 の per-VK（最初の 1 キー）。
//! - どちらも、最初のキーを TIP が見なかった（`drop_first_key`）ことを意味する。
//!
//! 限界（正直に）: 経路を結末から推定しているので、この変換が検証するのは「journal の記録と擬似物の結末が
//! 矛盾しないこと」であって、擬似物の独立な予言ではない。擬似物の予言力は、`drop_first_key` が起きる条件
//! （cold の長さ・アプリ）を実測の分布で与えたときに初めて測れる（段階 3）。

use serde_json::Value;

use super::pseudo_mozc::{Awase, PseudoMozc, Run, TraceEvent};

/// journal から取り出した、Mozc の確認の回収 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub seq: u64,
    pub elapsed_ms: u64,
    /// 直前の `FocusTransition` の窓のクラス（無ければ `None`）。
    pub class_name: Option<String>,
    pub romaji: String,
    pub backs: usize,
    pub escape: bool,
    pub key_seq_before: u32,
    pub key_seq_after: u32,
}

impl Case {
    /// journal の記録から推定した awase の側の設定。
    pub fn inferred_config(&self) -> Awase {
        let per_vk = !self.escape && self.backs == 1 && self.romaji.chars().count() >= 2;
        Awase {
            inverted_window_rule: true,
            settle: self.escape,
            per_vk,
        }
    }

    /// journal の記録と矛盾しない擬似 Mozc（cold の最初の 1 キーを TIP が見ない）。
    pub fn pseudo(&self) -> PseudoMozc {
        let mut pm = PseudoMozc::cold_dropping_first_key();
        pm.key_seq = self.key_seq_before;
        pm
    }

    /// 擬似物で再生する。
    pub fn replay(&self) -> Run {
        let class = self.class_name.as_deref().unwrap_or("");
        super::pseudo_mozc::run(
            self.inferred_config(),
            class,
            &mut self.pseudo(),
            &self.romaji,
        )
    }

    /// journal の記録が言う回収。擬似物の `TraceEvent::Recovered` と比べる。
    pub fn recorded_recovery(&self) -> TraceEvent {
        TraceEvent::Recovered {
            backs: self.backs,
            escape: self.escape,
            resent: self.romaji.clone(),
        }
    }
}

/// ダンプ（封筒の配列）から、Mozc の確認の回収を seq 順に取り出す。
pub fn extract_cases(dump: &Value) -> Vec<Case> {
    let Some(envelopes) = dump.as_array() else {
        return Vec::new();
    };
    let mut sorted: Vec<&Value> = envelopes.iter().collect();
    sorted.sort_by_key(|e| e["seq"].as_u64().unwrap_or(0));

    let mut class_name: Option<String> = None;
    let mut cases = Vec::new();
    for env in sorted {
        let entry = &env["entry"];
        if let Some(to) = entry.get("FocusTransition").map(|f| &f["to"]) {
            class_name = to["class_name"].as_str().map(str::to_string);
            continue;
        }
        let Some(ld) = entry.get("LiteralDetect") else {
            continue;
        };
        let rec = &ld["record"];
        let seq_ev = &rec["facts"]["evidence"]["mozc_key_seq"];
        let (Some(before), Some(after)) = (seq_ev["before"].as_u64(), seq_ev["after"].as_u64())
        else {
            continue; // Mozc の確認の verdict でない
        };
        let Some(romaji) = rec["romaji"].as_str() else {
            continue; // 回収でない（romaji を持たない verdict）
        };
        if rec["facts"]["verdict"] != "SuspectedLiteral" {
            continue;
        }
        cases.push(Case {
            seq: env["seq"].as_u64().unwrap_or(0),
            elapsed_ms: env["elapsed_ms"].as_u64().unwrap_or(0),
            class_name: class_name.clone(),
            romaji: romaji.to_string(),
            backs: usize::try_from(rec["backs"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX),
            escape: rec["escape_composition"].as_bool().unwrap_or(false),
            key_seq_before: u32::try_from(before).unwrap_or(u32::MAX),
            key_seq_after: u32::try_from(after).unwrap_or(u32::MAX),
        });
    }
    cases
}
