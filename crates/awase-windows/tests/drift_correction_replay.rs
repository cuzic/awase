//! ADR-082「第一歩」2./3.: BUG-43（drift correction 無限再送）の journal リプレイ回帰。
//!
//! `docs/known-bugs.md` BUG-43 の実機ログ（675ms の間に `apply_ime_open(false)` を
//! 16 回連続送信）を `DriftCorrectionFixture`（`state/ime_actuation.rs`）として固定化し、
//! ADR-080 Phase1 で実装済みの `decide_actuation_action` が同じ 16 回の drift 検知に
//! 対して試行回数を有界に打ち切る（`FeedbackPolicy::Blind::max_attempts` 到達後は
//! `GiveUp` のまま `Send` に戻らない）ことを回帰テストとして固定する。
//!
//! `tests/journal_replay.rs`（`ConvClassifyFixture` 専用、`tests/journals/*.json` を
//! フラットに読む）とは別ファイルに分離した。理由: `journal_replay.rs` は
//! `tests/journals/` 直下の全 `*.json` を無条件に `ConvClassifyFixture` としてパースする
//! ため、`DriftCorrectionFixture` 形式の JSON を同じ階層に置くとパース失敗で衝突する。
//! 本テストのフィクスチャは衝突を避けるため `tests/journals/drift_correction/`
//! サブディレクトリに置く（`journal_replay.rs` の `read_dir` は非再帰のため、
//! サブディレクトリ内のファイルはそちらから見えない）。
//!
//! `state::ime_actuation` は `#[cfg(windows)]` でゲートされていないため、
//! `conv_classify` と同様このテストは Linux ホストでもそのまま実行できる。
//!
//! # ADR-082 Phase 0.5: 「新 variant 経由」の意味
//!
//! `journal.rs::JournalEntry::ImeActuation` 自体は `journal` モジュールが
//! `#[cfg(windows)]` のため Linux からは参照できない。そこで variant のペイロード型
//! `ActuationRecord`（`state/ime_actuation.rs`、プラットフォーム非依存）を Linux でも
//! 共有し、リプレイはこの `ActuationRecord`（= journal に積まれるのと同一の構造化
//! レコード）を `ActuationRecord::new` で構築して照合する。これにより「出所（常に
//! `SelfActuated`）・世代（`epoch`）・判定（`action`）が型として正しく積まれるか」まで
//! 含めて回帰させる（従来は `decide_actuation_action` の `action` だけを見ていた）。

use awase_windows::state::app_ime_policy::AppImePolicy;
use awase_windows::state::event_origin::EventSource;
use awase_windows::state::ime_actuation::{
    actuation_origin, ActuationAction, ActuationRecord, DriftCorrectionFixture, FeedbackPolicy,
};
use awase_windows::state::ime_event::ImePolicyProfile;

/// BUG-43 の drift correction は `apply_ime_open(false)`（IME を OFF に落とす）だった。
/// fixture は試行回数の有界化検証が主眼で target を保持しないため、ここで固定する。
const BUG43_TARGET: bool = false;

/// 1 tick 分を journal の `JournalEntry::ImeActuation` に積まれるのと同一の
/// `ActuationRecord`（新 variant のペイロード）として組み立てる単一経路。
fn record_for_tick(
    fixture: &DriftCorrectionFixture,
    tick: &awase_windows::state::ime_actuation::DriftCorrectionTick,
) -> ActuationRecord {
    let origin = actuation_origin(fixture.policy, tick.epoch);
    ActuationRecord::new(origin, BUG43_TARGET, fixture.policy, tick.attempts)
}

fn load_fixtures(path: &std::path::Path) -> Vec<DriftCorrectionFixture> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("フィクスチャ読み込み失敗 {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("フィクスチャのJSONパース失敗 {}: {e}", path.display()))
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/drift_correction")
}

/// `ConvClassifyFixture` リプレイ（`tests/journal_replay.rs`）と同じ形の per-tick 照合:
/// フィクスチャに記録された `policy`/`attempts` の組で `decide_actuation_action` を
/// 再実行し、`expected` と一致するかを確認する。
#[test]
fn replay_all_drift_correction_fixtures() {
    let dir = fixture_dir();
    let mut failures = Vec::new();
    let mut total = 0usize;

    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} が読めない: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry read failed").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for path in &paths {
        for fixture in load_fixtures(path) {
            for tick in &fixture.ticks {
                total += 1;
                let record = record_for_tick(&fixture, tick);

                // (1) 判定（action）の照合。
                if record.action != tick.expected {
                    failures.push(format!(
                        "[{}] {} attempts={} observed_at_ms={:?}:\n  expected action: {:?}\n  actual action:   {:?}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        fixture.name,
                        tick.attempts,
                        tick.observed_at_ms,
                        tick.expected,
                        record.action,
                    ));
                }

                // (2) 出所の照合: actuation は常に SelfActuated（物理でも外部注入でもない）。
                let expected_source = EventSource::SelfActuated {
                    strategy: awase_windows::state::ime_actuation::actuation_strategy(
                        fixture.policy,
                    ),
                };
                if record.origin.source != expected_source {
                    failures.push(format!(
                        "[{}] {} attempts={}: origin.source が SelfActuated でない: {:?}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        fixture.name,
                        tick.attempts,
                        record.origin.source,
                    ));
                }

                // (3) 世代の配線: epoch は attempts と歩調を合わせて積まれる
                //     （Actuation::advance_epoch）。fixture の epoch と record の epoch、
                //     さらに attempts との一致を固定し、EventOrigin 配線の退行を検知する。
                if record.origin.epoch != tick.epoch
                    || record.origin.epoch.value() != u64::from(tick.attempts)
                {
                    failures.push(format!(
                        "[{}] {} attempts={}: epoch 配線が壊れている: tick.epoch={:?} record.epoch={:?}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        fixture.name,
                        tick.attempts,
                        tick.epoch,
                        record.origin.epoch,
                    ));
                }
            }
        }
    }

    assert!(total > 0, "{} にフィクスチャが1件もない", dir.display());
    assert!(
        failures.is_empty(),
        "{} 件のドリフト補正リプレイ不一致（実機で観測済みの入力に対する退行）:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// BUG-43 固有の意味論的アサーション: 675ms の間に観測された 16 回の drift 検知
/// すべてを実際に送信していた旧実装（journal フィードバック欠如、修正前）に対し、
/// ADR-080 の `Blind` ポリシーは `max_attempts` 到達後 `Send` に戻らないことを、
/// tick 単位の一致確認だけでなく「有界終端化そのもの」として明示的に確認する。
///
/// このテストが `replay_all_drift_correction_fixtures` と重複しているように見えても
/// 意図的に残す: フィクスチャの `expected` を書き間違えて全部 `"Send"` にしてしまう
/// ような回帰（=BUG-43 を固定化してしまう間違い）は前者だけでは検知できないため、
/// フィクスチャの値に依存しない不変条件をここで独立に検証する。
#[test]
fn bug43_tight_loop_is_bounded_not_infinite() {
    let dir = fixture_dir();
    let path = dir.join("bug-43-drift-correction-tight-loop.json");
    let fixtures = load_fixtures(&path);
    assert_eq!(fixtures.len(), 1, "BUG-43 フィクスチャは1件のはず");
    let fixture = &fixtures[0];

    let FeedbackPolicy::Blind { max_attempts, .. } = fixture.policy else {
        panic!("BUG-43 フィクスチャの policy は Blind のはず（TsfNative/Blacklist パス）");
    };

    // フィクスチャの policy を本番設定（AppImePolicy::from_profile）にアンカーする。
    // これが無いと、ADR-080 の有界化が将来本番側で緩められて（例: max_attempts 引き上げ、
    // FeedbackPolicy::Read への変更）も、フィクスチャに書かれた固定値のおかげでこの
    // 「BUG-43 回帰テスト」は無関係に green のまま通り続け、退行を検知できなくなる。
    assert_eq!(
        fixture.policy,
        AppImePolicy::from_profile(ImePolicyProfile::TsfNative).default_feedback,
        "BUG-43 フィクスチャの policy が本番の AppImePolicy::from_profile(TsfNative) と \
         乖離している（本番の有界化設定が変わったのにフィクスチャが追従していない）"
    );

    // BUG-43 実機ログの回数(16)が max_attempts(5) を上回っていることが前提条件。
    // これが崩れると「有界にした」ことの検証にならない（max_attempts 未到達のまま
    // 終わってしまうテストは何も証明しない）。
    let tick_count =
        u32::try_from(fixture.ticks.len()).expect("フィクスチャの tick 数は u32 に収まるはず");
    assert!(
        tick_count > max_attempts,
        "BUG-43 の観測回数({tick_count})が max_attempts({max_attempts}) を上回っていないと有界終端の証明にならない"
    );

    // 新 variant のペイロード `ActuationRecord` 経由で action 列を得る（journal に
    // 積まれるのと同一経路）。
    let actions: Vec<ActuationAction> = fixture
        .ticks
        .iter()
        .map(|tick| record_for_tick(fixture, tick).action)
        .collect();

    let send_count = actions
        .iter()
        .filter(|a| **a == ActuationAction::Send)
        .count();
    assert_eq!(
        send_count, max_attempts as usize,
        "16回の drift 検知のうち実際に送信されるのは max_attempts({max_attempts})回だけのはず \
         （BUG-43 の無限再送は再発していない）"
    );

    // 一度 GiveUp に達したら、この tick 列の範囲内では二度と Send に戻らない
    // （ADR-080 不変条件: Blind は境界を越えても Send に戻らない）。
    let first_give_up = actions
        .iter()
        .position(|a| *a == ActuationAction::GiveUp)
        .expect("16回の観測に対し max_attempts=5 なら GiveUp が発生するはず");
    assert!(
        actions[first_give_up..]
            .iter()
            .all(|a| *a == ActuationAction::GiveUp),
        "GiveUp 到達後に Send へ戻る tick があってはならない（BUG-43 と同型の再発）"
    );
    assert_eq!(
        actions.last(),
        Some(&ActuationAction::GiveUp),
        "16 tick分リプレイした最後まで有界打ち切りが維持されているはず"
    );
}
