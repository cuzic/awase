#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! P1: ジャーナル・リプレイ回帰基盤。
//!
//! # 目的
//! 過去の再発バグ（fc18cc7 / 109b4c9 / 1544d3f / ea3da7f 等）は、実機でしか観測できない
//! conv ビットの組合せが原因だった。`tests/journals/conv_classify/*.json` に「実際に観測された
//! `classify_conv_transition` の入力＋期待出力」をフィクスチャとして蓄積し、この
//! テストが毎回再実行して一致を確認する。**実機でしか観測できない事象を、観測した
//! 瞬間に固定化する**のが狙い。
//!
//! `state::conv_classify` モジュールは `#[cfg(windows)]` でゲートされていない
//! （唯一の呼び出し元 `runtime/key_pipeline.rs` が Windows 専用なだけ）ため、
//! このテストは Linux ホストでもそのまま実行できる。
//!
//! # フィクスチャの追加手順
//! 詳細は `docs/journal-replay-guide.md` を参照。要点:
//! 1. 実機でバグに気づいたら、**修正する前に** ホットキー（Alt+変換→Alt+無変換 を
//!    2 回連続）でジャーナルをダンプする（`journal.rs`）。
//! 2. ダンプ JSON から該当する `ConvClassifyCall` エントリを見つけ、このディレクトリに
//!    `ConvClassifyFixture` 形式（`name`/`note`/`conv`/`current`/`is_cold`/
//!    `effective_open`/`conv_mode_changed`/`is_roman_reliable`/`expected`）で転記する。
//! 3. 転記した直後の `expected` は「実際に起きたバグの出力」なので、**必ず** 手で
//!    「あるべき出力」に書き換えてからコミットする（そうしないとこのテストはバグを
//!    固定化してしまう）。
//! 4. 修正を実装し、このテストが通ることを確認する。

use awase::engine::ConvMode;
use awase_windows::state::conv_classify::{classify_conv_transition, ConvClassifyFixture};
use awase_windows::state::ime_event::{
    EventTime, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile,
};
use awase_windows::state::ime_model::{AppliedImeState, ImeModel};
use awase_windows::state::ApplyGeneration;

#[test]
fn replay_all_journal_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/conv_classify");
    // classify_conv_transition は ConvModeMgr::get() のデバウンス確定値を受け取る
    // （BUG-19）。フィクスチャは実機観測の生 conv を保存しているだけなので、
    // ここで ConvMode に変換する（このリプレイ基盤は conv ビット解釈ロジック
    // 自体の回帰検出が目的で、デバウンスとの相互作用は対象外 — 詳細は
    // `ConvClassifyFixture` のドキュメントコメント参照）。
    awase_replay::replay_dir::<ConvClassifyFixture>(&dir, |fixture| {
        let actual = classify_conv_transition(
            ConvMode::from_u32(fixture.conv),
            fixture.current,
            fixture.is_cold,
            fixture.effective_open,
            fixture.conv_mode_changed,
            fixture.is_roman_reliable,
        );
        if actual == fixture.expected {
            Ok(())
        } else {
            Err(format!(
                "{} ({}):\n  expected: {:?}\n  actual:   {:?}",
                fixture.name, fixture.note, fixture.expected, actual,
            ))
        }
    })
    .assert_ok();
}

#[derive(Debug, serde::Deserialize)]
struct ImeEventReplayFixture {
    name: String,
    note: String,
    events: Vec<ImeEventReplayStep>,
    expected_applied: ExpectedApplied,
    expected_pending_generation: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ImeEventReplayStep {
    at_ms: u64,
    event: ImeReplayEvent,
}

#[derive(Debug, Clone, serde::Deserialize)]
enum ImeReplayEvent {
    ImeApplyRequested {
        target: bool,
        generation: u64,
        ctrl_held: bool,
    },
    ImeApplySucceeded {
        target: bool,
        generation: u64,
    },
    FocusChanged {
        to: usize,
        focus_epoch: u64,
        profile: String,
    },
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
enum ExpectedApplied {
    Unknown,
    Optimistic(bool),
    Confirmed { open: bool, at_ms: u64 },
}

impl ExpectedApplied {
    const fn to_state(self) -> AppliedImeState {
        match self {
            Self::Unknown => AppliedImeState::Unknown,
            Self::Optimistic(open) => AppliedImeState::Optimistic(open),
            Self::Confirmed { open, at_ms } => AppliedImeState::Confirmed { open, at_ms },
        }
    }
}

fn apply_ime_replay_event(
    model: &mut ImeModel,
    seq: u64,
    base: std::time::Instant,
    step: ImeEventReplayStep,
) {
    let event = match step.event {
        ImeReplayEvent::ImeApplyRequested {
            target,
            generation,
            ctrl_held,
        } => ImeEvent::ImeApplyRequested {
            target,
            generation: ApplyGeneration::new(generation)
                .unwrap_or_else(|| panic!("invalid generation {generation}")),
            ctrl_held,
        },
        ImeReplayEvent::ImeApplySucceeded { target, generation } => ImeEvent::ImeApplySucceeded {
            target,
            generation: ApplyGeneration::new(generation)
                .unwrap_or_else(|| panic!("invalid generation {generation}")),
        },
        ImeReplayEvent::FocusChanged {
            to,
            focus_epoch,
            profile,
        } => ImeEvent::FocusChanged {
            from: None,
            to: HwndId(to),
            profile: parse_ime_policy_profile(&profile),
            focus_epoch,
        },
    };
    model.reduce(&ImeEventEnvelope {
        time: EventTime {
            seq,
            monotonic: base + std::time::Duration::from_millis(step.at_ms),
            tick_ms: step.at_ms,
        },
        event,
    });
}

fn parse_ime_policy_profile(value: &str) -> ImePolicyProfile {
    match value {
        "ImmCross" => ImePolicyProfile::ImmCross,
        "Imm32Unavailable" => ImePolicyProfile::Imm32Unavailable,
        "TsfNative" => ImePolicyProfile::TsfNative,
        "Plain" => ImePolicyProfile::Plain,
        "Unknown" => ImePolicyProfile::Unknown,
        other => panic!("unknown ImePolicyProfile fixture value: {other}"),
    }
}

#[test]
fn replay_ime_apply_focus_epoch_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/ime_apply");
    // 1 ディレクトリ = 1 形式のため、ケースは (fixture ごとの) 2 つの照合をまとめて 1 件として扱う。
    // `replay_dir` は fixture を借用で渡すため、消費型の `apply_ime_replay_event` には clone して渡す。
    awase_replay::replay_dir::<ImeEventReplayFixture>(&dir, |fixture| {
        let mut model = ImeModel::new();
        let base = std::time::Instant::now();
        for (i, step) in fixture.events.iter().cloned().enumerate() {
            apply_ime_replay_event(&mut model, i as u64 + 1, base, step);
        }
        let mut errors = Vec::new();
        if model.applied_state() != fixture.expected_applied.to_state() {
            errors.push(format!(
                "applied_state: expected {:?}, actual {:?}",
                fixture.expected_applied.to_state(),
                model.applied_state()
            ));
        }
        let pending = model.pending_generation().map(ApplyGeneration::get);
        if pending != fixture.expected_pending_generation {
            errors.push(format!(
                "pending_generation: expected {:?}, actual {pending:?}",
                fixture.expected_pending_generation
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{} ({}): {}",
                fixture.name,
                fixture.note,
                errors.join("; ")
            ))
        }
    })
    .assert_ok();
}
