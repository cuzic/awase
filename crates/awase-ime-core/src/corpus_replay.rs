//! ADR-278 段階 1(試験運転): 実機 CI のジャーナル(コーパス)を読み、記録が入力と出力の両方を持つ種類だけを、
//! 純粋な決定関数に通し直して記録と比べる(決定の差分の再生)。
//!
//! - 入力: 1 行 1 envelope の JSON(`journal::emit_json_line` が `awase::journal_json` の TRACE 行に出す形)。
//!   `awase.log` の行のまま(`... awase::journal_json: {json}`)でも、切り出した JSON だけでも読む。
//! - 読む場所: リポジトリの小さな代表 `tests/journals/corpus/*.jsonl` と、環境変数 `AWASE_CORPUS_DIR` の下の
//!   `*.jsonl`(再帰。CI の artifact `result-*/journal.jsonl` を展開した置き場)。
//! - 再生する種類: `ConvClassifyCall`(`classify_conv_transition`)と `ActuationDecision`
//!   (`actuation_decision_record::tests::replay_record`、gate・chain・command・チェーン走査)。
//!   ほかの種類は数えるだけ(どの記録がコーパスにあるかの一覧。カバレッジの地図の材料)。
//! - 2 コミットの比較(ADR-278 現状調査 (4)): 基準版で取ったコーパスを候補版のコードで再生し、記録と食い違う件を数える。
//!   食い違いは「どのファイル・何行目・seq で、記録と再計算がどう違うか」を全件出す。
//!
//! **限界(誤読しないこと)**: 一致は「再生した決定関数の出力が記録と同じ」ことだけを言う。殻の配線(その関数を呼ぶか)の変更、
//! 記録されない決定、実 IME の結果(文字)は判定しない(ADR-278「限界」)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use awase::engine::{ConvMode, InputModeState};
use serde_json::Value;

use crate::journal::JOURNAL_JSON_TARGET;
use crate::state::actuation_decision_record::ActuationDecisionRecord;
use crate::state::conv_classify::{classify_conv_transition, ConvTransition};

/// 1 行ぶんの記録(出所つき)。
pub(crate) struct CorpusLine {
    pub(crate) source: String,
    pub(crate) line_no: usize,
    pub(crate) envelope: Value,
}

/// 読み込みと再生の集計。
#[derive(Debug, Default)]
pub(crate) struct CorpusReport {
    pub(crate) files: usize,
    pub(crate) lines: usize,
    pub(crate) parse_errors: usize,
    /// `entry.type` ごとの件数(再生しない種類も数える)。
    pub(crate) by_type: BTreeMap<String, usize>,
    /// 再生した件数(種類ごと)。
    pub(crate) replayed: BTreeMap<&'static str, usize>,
    /// 食い違いの全件(「出所:行 seq=N 種類: 理由」)。
    pub(crate) mismatches: Vec<String>,
}

impl CorpusReport {
    pub(crate) fn summary(&self) -> String {
        let mut s = format!(
            "corpus: files={} lines={} parse_errors={} replayed={:?} mismatches={}\n",
            self.files,
            self.lines,
            self.parse_errors,
            self.replayed,
            self.mismatches.len()
        );
        for (ty, n) in &self.by_type {
            s.push_str(&format!("  type {ty}: {n}\n"));
        }
        for m in &self.mismatches {
            s.push_str(&format!("  MISMATCH {m}\n"));
        }
        s
    }
}

/// `dir` の下の `*.jsonl` を集める(`recursive` なら再帰)。名前順。
fn jsonl_paths(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if recursive {
                jsonl_paths(&path, true, out);
            }
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

/// 1 行から envelope の JSON を取り出す。`awase.log` の行なら target の後ろを使う。
fn parse_line(line: &str) -> Option<Value> {
    let marker = format!("{JOURNAL_JSON_TARGET}: ");
    let json = line
        .find(&marker)
        .map_or(line, |i| &line[i + marker.len()..]);
    let json = json.trim();
    if !json.starts_with('{') {
        return None;
    }
    serde_json::from_str(json).ok()
}

/// `paths` の各ファイルを読み、行ごとの envelope にする。
pub(crate) fn load(paths: &[PathBuf], report: &mut CorpusReport) -> Vec<CorpusLine> {
    let mut lines = Vec::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            report.parse_errors += 1;
            continue;
        };
        report.files += 1;
        let source = path.display().to_string();
        for (i, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            report.lines += 1;
            match parse_line(raw) {
                Some(envelope) => lines.push(CorpusLine {
                    source: source.clone(),
                    line_no: i + 1,
                    envelope,
                }),
                None => report.parse_errors += 1,
            }
        }
    }
    lines
}

/// `ConvClassifyCall` の記録(`JournalEntry::ConvClassifyCall` の項目と同じ名前)。
#[derive(serde::Deserialize)]
struct ConvClassifyRecorded {
    conv: u32,
    current: InputModeState,
    is_cold: bool,
    effective_open: bool,
    conv_mode_changed: bool,
    is_roman_reliable: bool,
    result: ConvTransition,
}

fn replay_conv_classify(entry: &Value) -> Result<(), String> {
    let r: ConvClassifyRecorded =
        serde_json::from_value(entry.clone()).map_err(|e| format!("読めない: {e}"))?;
    let actual = classify_conv_transition(
        ConvMode::from_u32(r.conv),
        r.current,
        r.is_cold,
        r.effective_open,
        r.conv_mode_changed,
        r.is_roman_reliable,
    );
    if actual == r.result {
        Ok(())
    } else {
        Err(format!("recorded {:?} != replayed {actual:?}", r.result))
    }
}

fn replay_actuation_decision(entry: &Value) -> Result<(), String> {
    let record_value = entry.get("record").ok_or("record が無い")?;
    let record: ActuationDecisionRecord =
        serde_json::from_value(record_value.clone()).map_err(|e| format!("読めない: {e}"))?;
    let failures = crate::state::actuation_decision_record::replay_record_for_corpus(&record);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(" / "))
    }
}

/// 再生できる種類なら再生して `Some(結果)`、数えるだけの種類なら `None`。
fn replay_entry(ty: &str, entry: &Value) -> Option<(&'static str, Result<(), String>)> {
    match ty {
        "ConvClassifyCall" => Some(("ConvClassifyCall", replay_conv_classify(entry))),
        "ActuationDecision" => Some(("ActuationDecision", replay_actuation_decision(entry))),
        _ => None,
    }
}

/// 全行を種類ごとに数え、再生できる種類を再生する。
pub(crate) fn replay(lines: &[CorpusLine], report: &mut CorpusReport) {
    for line in lines {
        let Some(entry) = line.envelope.get("entry") else {
            report.parse_errors += 1;
            continue;
        };
        let ty = entry.get("type").and_then(Value::as_str).unwrap_or("?");
        *report.by_type.entry(ty.to_owned()).or_default() += 1;
        let Some((kind, result)) = replay_entry(ty, entry) else {
            continue;
        };
        *report.replayed.entry(kind).or_default() += 1;
        if let Err(reason) = result {
            let seq = line
                .envelope
                .get("seq")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            report.mismatches.push(format!(
                "{}:{} seq={seq} {kind}: {reason}",
                line.source, line.line_no
            ));
        }
    }
}

/// リポジトリに入れた小さな代表の置き場。
fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/corpus")
}

/// `dir` を読んで再生した集計。
pub(crate) fn replay_dir(dir: &Path, recursive: bool) -> CorpusReport {
    let mut paths = Vec::new();
    jsonl_paths(dir, recursive, &mut paths);
    let mut report = CorpusReport::default();
    let lines = load(&paths, &mut report);
    replay(&lines, &mut report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// リポジトリの代表のコーパスを HEAD で再生して、記録と全件一致すること。ファイル 0 件・再生 0 件は失敗にする
    /// (置き場の消失・形式の変化で黙って素通りしない)。
    #[test]
    fn committed_corpus_replays_without_mismatch() {
        let report = replay_dir(&committed_dir(), false);
        println!("{}", report.summary());
        assert!(report.files > 0, "tests/journals/corpus/*.jsonl が無い");
        assert!(
            report.replayed.values().sum::<usize>() > 0,
            "再生できる記録が 1 件も無い:\n{}",
            report.summary()
        );
        assert_eq!(report.parse_errors, 0, "{}", report.summary());
        assert!(report.mismatches.is_empty(), "{}", report.summary());
    }

    /// `AWASE_CORPUS_DIR` が指す置き場(CI の artifact を展開したもの)を再生する。未設定なら何もしない
    /// (e2e-ime の `corpus-replay` ジョブだけが設定して `--nocapture` で呼ぶ)。
    #[test]
    fn env_corpus_replays_without_mismatch() {
        let Some(dir) = std::env::var_os("AWASE_CORPUS_DIR") else {
            return;
        };
        let report = replay_dir(Path::new(&dir), true);
        println!("{}", report.summary());
        assert!(report.files > 0, "AWASE_CORPUS_DIR に *.jsonl が無い");
        assert!(report.mismatches.is_empty(), "{}", report.summary());
    }

    #[test]
    fn parse_line_accepts_log_lines_and_bare_json() {
        let bare =
            r#"{"seq":1,"elapsed_ms":2,"entry":{"type":"ClockAnchor","tick_ms":3,"hook_us":4}}"#;
        assert!(parse_line(bare).is_some());
        let log =
            format!("2026-10-22T00:00:00.000Z TRACE span{{a=1}}: {JOURNAL_JSON_TARGET}: {bare}");
        assert_eq!(parse_line(&log), parse_line(bare));
        assert!(
            parse_line("2026-10-22T00:00:00.000Z DEBUG awase::journal: key input seq=1").is_none()
        );
    }

    /// 記録と再計算が食い違う行を、食い違いとして報告すること(素通りしない)。
    #[test]
    fn tampered_conv_classify_result_is_reported() {
        let current = serde_json::to_value(InputModeState::ObservedKana).unwrap();
        let actual = classify_conv_transition(
            ConvMode::from_u32(9),
            InputModeState::ObservedKana,
            false,
            true,
            true,
            false,
        );
        let good = serde_json::json!({"seq": 7, "elapsed_ms": 0, "entry": {
            "type": "ConvClassifyCall", "conv": 9, "current": current, "is_cold": false,
            "effective_open": true, "conv_mode_changed": true, "is_roman_reliable": false,
            "result": serde_json::to_value(actual).unwrap()}});
        let mut bad = good.clone();
        // 再計算と必ず違う値に書き換える(`ObservedKana` なら `None`、それ以外なら `ObservedKana`)。
        let tampered = if actual.input_mode_update == Some(InputModeState::ObservedKana) {
            None
        } else {
            Some(InputModeState::ObservedKana)
        };
        bad["entry"]["result"]["input_mode_update"] = serde_json::to_value(tampered).unwrap();
        let lines: Vec<CorpusLine> = [good, bad]
            .into_iter()
            .enumerate()
            .map(|(i, envelope)| CorpusLine {
                source: "t".into(),
                line_no: i + 1,
                envelope,
            })
            .collect();
        let mut report = CorpusReport::default();
        replay(&lines, &mut report);
        assert_eq!(report.replayed.get("ConvClassifyCall"), Some(&2));
        assert_eq!(report.mismatches.len(), 1, "{}", report.summary());
        assert!(report.mismatches[0].starts_with("t:2 seq=7 ConvClassifyCall"));
    }
}
