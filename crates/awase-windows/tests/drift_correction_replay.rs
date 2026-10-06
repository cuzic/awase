#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! ADR-082「第一歩」2./3.: BUG-43（drift correction 無限再送）の journal リプレイ回帰。
//!
//! `docs/known-bugs.md` BUG-43 の実機ログ（675ms の間に `apply_ime_open(false)` を
//! 16 回連続送信）を `DriftCorrectionFixture`（`state/ime_actuation.rs`）として固定化し、
//! ADR-080 Phase1 で実装済みの `FeedbackPolicy::decide_action` が同じ 16 回の drift 検知に
//! 対して試行回数を有界に打ち切る（`FeedbackPolicy::Blind::max_attempts` 到達後は
//! `GiveUp` のまま `Send` に戻らない）ことを回帰テストとして固定する。
//!
//! `tests/journals/` は「1 ディレクトリ = 1 形式」で、`DriftCorrectionFixture` 形式の JSON は
//! `tests/journals/drift_correction/` に置く（`tests/journal_replay.rs` の `ConvClassifyFixture` は
//! `tests/journals/conv_classify/`）。読み込みと件数の集計は `awase_replay::replay_dir`。
//!
//! `state::ime_actuation` は `#[cfg(windows)]` でゲートされていないため、
//! `conv_classify` と同様このテストは Linux ホストでもそのまま実行できる。
//!
//! # ADR-082 Phase 0.5: 「新 variant 経由」の意味
//!
//! `journal.rs::JournalEntry::ImeActuation` は（ADR-229 T4 で gate 解除済みのため）
//! Linux からも参照できるが、このテストは variant のペイロード型
//! `ActuationRecord`（`state/ime_actuation.rs`、プラットフォーム非依存）を Linux でも
//! 共有し、リプレイはこの `ActuationRecord`（= journal に積まれるのと同一の構造化
//! レコード）を `ActuationRecord::new` で構築して照合する。これにより「出所（常に
//! `SelfActuated`）・世代（`epoch`）・判定（`action`）が型として正しく積まれるか」まで
//! 含めて回帰させる（従来は `FeedbackPolicy::decide_action` の `action` だけを見ていた）。

use awase_windows::state::app_ime_policy::AppImePolicy;
use awase_windows::state::event_origin::EventSource;
use awase_windows::state::ime_actuation::{
    ActuationAction, ActuationRecord, DriftCorrectionFixture, FeedbackPolicy,
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
    let origin = fixture.policy.origin(tick.epoch);
    ActuationRecord::new(origin, BUG43_TARGET, fixture.policy, tick.attempts)
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/drift_correction")
}

/// `ConvClassifyFixture` リプレイ（`tests/journal_replay.rs`）と同じ形の per-tick 照合:
/// フィクスチャに記録された `policy`/`attempts` の組で `FeedbackPolicy::decide_action` を
/// 再実行し、`expected` と一致するかを確認する。ケース 1 件 = フィクスチャ 1 件（tick は件内で全件照合）。
#[test]
fn replay_all_drift_correction_fixtures() {
    awase_replay::replay_dir::<DriftCorrectionFixture>(&fixture_dir(), check_fixture).assert_ok();
}

fn check_fixture(fixture: &DriftCorrectionFixture) -> Result<(), String> {
    let mut errors = Vec::new();
    // tick が 0 件の fixture は何も検査しないまま通るので失敗にする（件の単位を fixture にした際の
    // カバレッジ後退を戻す。以前は tick の総数 0 で落ちた）。
    if fixture.ticks.is_empty() {
        errors.push(format!(
            "{}: ticks が 0 件（何も検査していない）",
            fixture.name
        ));
    }
    for tick in &fixture.ticks {
        let record = record_for_tick(fixture, tick);

        // (1) 判定（action）の照合。
        if record.action != tick.expected {
            errors.push(format!(
                "{} attempts={} observed_at_ms={:?}:\n  expected action: {:?}\n  actual action:   {:?}",
                fixture.name, tick.attempts, tick.observed_at_ms, tick.expected, record.action,
            ));
        }

        // (2) 出所の照合: actuation は常に SelfActuated（物理でも外部注入でもない）。
        let expected_source = EventSource::SelfActuated {
            strategy: fixture.policy.strategy(),
        };
        if record.origin.source != expected_source {
            errors.push(format!(
                "{} attempts={}: origin.source が SelfActuated でない: {:?}",
                fixture.name, tick.attempts, record.origin.source,
            ));
        }

        // (3) 世代の配線: epoch は attempts と歩調を合わせて積まれる
        //     （Actuation::advance_epoch）。fixture の epoch と record の epoch、
        //     さらに attempts との一致を固定し、EventOrigin 配線の退行を検知する。
        if record.origin.epoch != tick.epoch
            || record.origin.epoch.value() != u64::from(tick.attempts)
        {
            errors.push(format!(
                "{} attempts={}: epoch 配線が壊れている: tick.epoch={:?} record.epoch={:?}",
                fixture.name, tick.attempts, tick.epoch, record.origin.epoch,
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
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
    let fixtures =
        awase_replay::load_file::<DriftCorrectionFixture>(&path).unwrap_or_else(|e| panic!("{e}"));
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

/// `ticks: []` の fixture が `replay_dir` 経由で失敗として報告されること（素通りの後退を固定する）。
#[test]
fn fixture_without_ticks_is_reported_as_failure() {
    let dir = std::env::temp_dir().join(format!("awase-drift-empty-ticks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let real = fixture_dir().join("bug-43-drift-correction-tight-loop.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(real).unwrap()).unwrap();
    for fixture in value.as_array_mut().unwrap() {
        fixture["ticks"] = serde_json::json!([]);
    }
    let fixtures = value.as_array().unwrap().len();
    std::fs::write(dir.join("empty.json"), value.to_string()).unwrap();
    let report = awase_replay::replay_dir::<DriftCorrectionFixture>(&dir, check_fixture);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(report.cases, fixtures);
    assert_eq!(report.failures.len(), fixtures, "{:?}", report.failures);
    assert!(report.failures.iter().all(|f| f.contains("ticks が 0 件")));
}
