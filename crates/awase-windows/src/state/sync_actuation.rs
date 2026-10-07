//! 同期経路の IME actuation の判断（FCIS の核、ADR-241 決定2）。
//!
//! 元は殻（`#[cfg(windows)]`）の 2 か所にあった判断を、挙動を変えずにここへ移した。
//!
//! | 元の場所 | ここ |
//! |---|---|
//! | `ime_controller.rs::ImeController::apply` の本体（gate → 授権 → 機構チェーン → 記録の組み立て） | [`apply_sync`] |
//! | `ime_controller.rs::SyncChainWriter::write`（`decide_attempt` → 送信 → 試行の記録） | [`CoreSyncWriter`] |
//! | `ime_controller.rs` の機構の適用可否・`imm_cross_is_first_applicable` | [`mechanism_applicable`]・[`imm_cross_is_first_applicable`] |
//! | `runtime/executor.rs::dispatch_ime_set_open` の判断（D1 の未知化 → gate → claim → plan → 同期チェーン） | [`dispatch_set_open`] |
//!
//! **差し替え口は [`CommandSink`] の 1 つだけ**（決まった `MechanismCommand` を送って outcome を返す）。
//! 何を送るか（`attempts[].command`）は核が決め、殻（`ime_controller.rs::ShellCommandSink`）と再生の側では作らない。
//! `run_chain`（走査規則・型状態、ADR-089/090）は書き換えていない。汎用の Effect/Handler 基盤でもない（ADR-229 で却下済み）。
//!
//! # 同期経路の合流点（ADR-119 の更新、ADR-241）
//!
//! 同期経路の唯一の合流点は [`apply_sync`] である（旧 `ImeController::apply`）。呼び出し元は
//! `ImeController::apply`（drift correction・shadow toggle の殻）と [`dispatch_set_open`] の 2 つで、
//! InputRelay の gate（issue #136・BUG-90 決定4）は [`apply_sync`] の先頭で必ず効く。
//! [`dispatch_set_open`] は executor 入口の早期 gate（`DecisionSite::DispatchImeSetOpen` の記録を積む独立した判定点）を
//! 別に持つ。非同期経路（`runtime/open_chain.rs` の 3 関数が各自 gate を再検出する設計、ADR-180 の INV-45）には触れない。
//! `with_app`・HWND・OS の読み取りはここに無い（`ImeStateHub` は呼び出し元から `&mut` で受ける）。

use awase::platform::ImeOpenOutcome;

use crate::state::actuation_chain::{
    ActuationOrder, MechanismWriter, VerifiedTarget, WriteMechanism,
};
use crate::state::actuation_decision_record::{
    ActuationDecisionRecord, ActuationOrderRecord, AttemptRecord, MAX_WRITE_MECHANISMS,
};
use crate::state::ime_actuation_decision::{
    decide_attempt, decide_chain, decide_gate, engine_press_unknowns_applied,
    explicit_press_applied_pair, DecisionInputs, DecisionSite, GateResult, MechanismCommand,
};
use crate::state::ime_set_open_plan::{plan_set_open, SetOpenFacts, SetOpenPlan};
use crate::state::key_sequence_policy;
use crate::state::platform_state::ImeStateHub;
use crate::state::press_ledger::{outcome_sent_nothing, PressSource};

/// 決まった command を 1 機構ぶん送る口（同期経路の I/O の唯一の差し替え口）。
///
/// 本番の実装は `ime_controller.rs::ShellCommandSink`（`apply_mechanism`）。`command` は
/// [`CoreSyncWriter`] が `decide_attempt` で決めたもので、実装は再計算しない（`None` = already-matched で送らない）。
/// 呼んでよいのは [`CoreSyncWriter`] の `write`（= `run_chain` が駆動する write ステップ）だけ
/// （`tests/architecture_guard.rs::raw_mechanism_write_sites_are_confined_to_chain_writers`）。
pub(crate) trait CommandSink {
    fn send_command(
        &mut self,
        mechanism: WriteMechanism,
        command: Option<MechanismCommand>,
        open: bool,
    ) -> ImeOpenOutcome;
}

/// 機構がこの決定入力で適用可能か（旧 `ImmCrossProcessStrategy`/`GjiDirectStrategy`/`MsImeDirectStrategy` の
/// `is_applicable`）。ImmCross は IMM-bridge が使えるプロファイル、GjiDirect は GJI（全プロファイル）、
/// MsImeDirect は MS-IME（CLSID ベースの判定）。
#[must_use]
pub(crate) fn mechanism_applicable(mechanism: WriteMechanism, inputs: DecisionInputs) -> bool {
    match mechanism {
        WriteMechanism::ImmCross => key_sequence_policy::imm_cross_applicable(inputs.profile),
        WriteMechanism::GjiDirect => key_sequence_policy::gji_direct_applicable(inputs.kind),
        WriteMechanism::MsImeDirect => key_sequence_policy::ms_ime_direct_applicable(inputs.kind),
    }
}

/// ImmCross が機構チェーンの先頭で、かつ最初に適用可能か（async/sync の分岐）。
///
/// **`chain[0]` の同一性チェックが要る**——`Imm32Unavailable` / `TsfNative` の chain は `[GjiDirect]` /
/// `[MsImeDirect]` なので、「chain 中で最初に適用可能な要素の index が 0 か」だけを見ると真になってしまい、
/// GJI 経路が誤って async 分岐へ流れる。
#[must_use]
pub(crate) fn imm_cross_is_first_applicable(inputs: DecisionInputs) -> bool {
    let chain = decide_chain(inputs);
    chain.first() == Some(&WriteMechanism::ImmCross)
        && chain.iter().position(|m| mechanism_applicable(*m, inputs)) == Some(0)
}

/// この送信が GJI の候補ウィンドウ再表示という desync 証拠（`candidate_was_seen`）を消費したか（ADR-171）。
///
/// GjiDirect の OFF 方向で `SendVk` を送り、`SendInput` の発行に成功した（`Applied`）ときだけ真。
/// 殻は真のとき `tsf::observer::reset_candidate_was_seen()` を呼ぶ（次回 apply が同じ証拠を再度読む BUG-113 型の
/// 二重送信を防ぐ）。GJI が実際に候補ウィンドウを閉じたことの確認は待たない（1 回の証拠につき再送は 1 回、
/// ADR-171 round4 M4）。
#[must_use]
pub(crate) const fn consumes_candidate_evidence(
    mechanism: WriteMechanism,
    command: Option<MechanismCommand>,
    open: bool,
    outcome: ImeOpenOutcome,
) -> bool {
    matches!(mechanism, WriteMechanism::GjiDirect)
        && !open
        && matches!(command, Some(MechanismCommand::SendVk(_)))
        && matches!(outcome, ImeOpenOutcome::Applied)
}

/// 同期チェーンの writer（旧 `ime_controller.rs::SyncChainWriter`）。
///
/// sync 経路では全 attempt が同じ決定入力を使う（`ActuationDecisionRecord::gate_inputs` と一致）。
struct CoreSyncWriter<'s, S: ?Sized> {
    inputs: DecisionInputs,
    sink: &'s mut S,
    attempts: [Option<AttemptRecord>; MAX_WRITE_MECHANISMS],
    attempts_len: usize,
}

impl<'s, S: CommandSink + ?Sized> CoreSyncWriter<'s, S> {
    fn new(inputs: DecisionInputs, sink: &'s mut S) -> Self {
        Self {
            inputs,
            sink,
            attempts: [None; MAX_WRITE_MECHANISMS],
            attempts_len: 0,
        }
    }
}

impl<S: CommandSink + ?Sized> MechanismWriter for CoreSyncWriter<'_, S> {
    fn is_applicable(&self, mechanism: WriteMechanism) -> bool {
        mechanism_applicable(mechanism, self.inputs)
    }

    fn write(&mut self, mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome {
        let (_, command) = decide_attempt(self.inputs, DecisionSite::Sync, mechanism, open);
        let outcome = self.sink.send_command(mechanism, command, open);
        if self.attempts_len < MAX_WRITE_MECHANISMS {
            self.attempts[self.attempts_len] = Some(AttemptRecord {
                inputs: self.inputs,
                with_app_available: true,
                mechanism,
                command,
                outcome,
                shadow_on_before_bug113_override: None,
                post_failed_reobservation: None,
            });
            self.attempts_len += 1;
        }
        outcome
    }
}

fn chain_record(
    chain: &[WriteMechanism],
) -> ([Option<WriteMechanism>; MAX_WRITE_MECHANISMS], usize) {
    debug_assert!(chain.len() <= MAX_WRITE_MECHANISMS);
    let mut record = [None; MAX_WRITE_MECHANISMS];
    for (index, mechanism) in chain.iter().copied().enumerate() {
        record[index] = Some(mechanism);
    }
    (record, chain.len())
}

// /code-review指摘（S-5、PR #201）: `order: &ActuationOrder`ではなく
// `ActuationOrderRecord`（Copy、記録に必要な3値のみ）を受け取る——
// `ActuationOrder`はINV-47のアフィン値であり、記録のためだけに`.clone()`で
// warrantを複製しない（`runtime/open_chain.rs::async_record`と同じ理由）。
fn sync_record(
    gate_inputs: DecisionInputs,
    order_record: ActuationOrderRecord,
    chain: &[WriteMechanism],
    attempts: [Option<AttemptRecord>; MAX_WRITE_MECHANISMS],
    attempts_len: usize,
) -> ActuationDecisionRecord {
    let (chain, chain_len) = chain_record(chain);
    ActuationDecisionRecord {
        site: DecisionSite::Sync,
        gate_inputs,
        order: order_record,
        chain,
        chain_len,
        attempts,
        attempts_len,
        caller: None,
    }
}

/// A-1 shadow の測定点（ADR-090 §2.A 設計案 2、§6 ステップ 5 item 21）。
///
/// 「実機で実際にどの入口が何回 warrant を取れないか」を測るための唯一のログ点。
/// **差分オラクル（`open_warrant.rs`）は 240 通りの組合せを測っているが、
/// 実機でどの組合せが実際に起きるかは測っていない。** A-2（強制）の対象入口は
/// このログがゼロだった入口から順に決める。
///
/// # 「ゼロだったから安全」と「そもそも発火していない」を混同しないこと
///
/// ADR-090 §7-1 が指摘するとおり、撤去済みの `try_force_on_bootstrap` の発火条件
/// （`IME_DETECT_MISS_THRESHOLD` 回連続の検出失敗）は稀であり、1 日の通常利用
/// では一度も踏まない可能性が高い。そのため**授権が下りた場合も 1 行出す**
/// ——入口が発火したこと自体をログに残さないと、`would_have_blocked` の
/// ゼロが「安全」なのか「未測定」なのか区別できない。
pub(crate) fn log_shadow_warrant(chain: &str, order: &ActuationOrder) {
    if order.would_have_blocked() {
        tracing::info!(
            "[warrant-shadow] chain={chain} open={} origin={:?} would_have_blocked=true \
             (A-1 shadow: 書き込みは止めない。A-2 で強制する際の判断材料)",
            order.open(),
            order.origin(),
        );
    } else {
        tracing::debug!(
            "[warrant-shadow] chain={chain} open={} origin={:?} warranted",
            order.open(),
            order.origin(),
        );
    }
}

/// 機構チェーンを走査して IME を設定する（同期経路の唯一の合流点、旧 `ImeController::apply` の本体）。
///
/// 機構が `Failed` を返した場合（例: ImmCross の `SendMessageTimeout` タイムアウト）、次の適用可能な機構へ
/// フォールスルーする（走査規則は `Actuation::<Verified>::run_chain` が SSOT）。
/// `class_name` は全機構が失敗したときのログ専用（決定入力には入れない。`DecisionInputs` の doc の D8）。
pub(crate) fn apply_sync<S: CommandSink + ?Sized>(
    order: ActuationOrder,
    inputs: DecisionInputs,
    class_name: &str,
    sink: &mut S,
) -> (ImeOpenOutcome, ActuationDecisionRecord) {
    // issue #136 / BUG-90 決定4: この窓は awase が IME actuation を所有しない
    // （InputRelay）。ここが同期経路（shadow toggle・drift correction・executor の
    // Engine の SetOpen）の唯一の合流点であり、`dispatch_set_open` の早期 gate を
    // バイパスする経路を含めてここで確実に止める。ADR-119 参照（gate をここ1点に
    // 集約できなかった経緯）。
    if matches!(decide_gate(inputs), GateResult::NotOwned) {
        let record = sync_record(
            inputs,
            ActuationOrderRecord::from(&order),
            &[],
            [None; MAX_WRITE_MECHANISMS],
            0,
        );
        return (ImeOpenOutcome::NotOwned, record);
    }
    // ADR-090 §2.A A-2: 授権は入口側（`ImeStateHub::issue_actuation_order`）で発行済み。
    // 警告なし（`None`）の場合は`Unwarranted`を返し、機構チェーンを一切試行しない
    // （`NotOwned`と同じ「送っていない」扱い、別variant）。
    log_shadow_warrant("sync", &order);
    let chain = decide_chain(inputs);
    let order_record = ActuationOrderRecord::from(&order);
    let Some(actuation) = order.into_actuation() else {
        let record = sync_record(inputs, order_record, &[], [None; MAX_WRITE_MECHANISMS], 0);
        return (ImeOpenOutcome::Unwarranted, record);
    };
    // 宛先: VK 送信機構（GjiDirect / MsImeDirect）は SendInput が
    // フォアグラウンドのフォーカスへ配送するため、hwnd を捕獲する余地が
    // 構造的に無い（`SendInput` は宛先引数を取らない）。したがって
    // `FocusImplicit` はこの経路では「未移行」ではなく**機構固有の性質**で
    // ある（ADR-089 §9-19 の訂正）。同期経路で hwnd を持つ唯一の write は
    // ROMAN 補完であり、そちらは殻の `apply_mechanism` が
    // `ActuationTarget::capture_blocking` で捕獲する（Phase C item 12）。
    let actuation = actuation.verify(VerifiedTarget::FocusImplicit);
    let mut writer = CoreSyncWriter::new(inputs, sink);
    let outcome = actuation.run_chain(chain, &mut writer);
    if outcome == ImeOpenOutcome::Failed {
        tracing::warn!("[apply-ime] all strategies failed for class={class_name}");
    }
    let record = sync_record(
        inputs,
        order_record,
        chain,
        writer.attempts,
        writer.attempts_len,
    );
    (outcome, record)
}

/// [`dispatch_set_open`] に渡す、殻が集めた事実。
#[derive(Debug, Clone, Copy)]
pub(crate) struct SetOpenRequest {
    pub open: bool,
    /// この `SetOpen` を起こしたユーザー打鍵（非リピート KeyDown）の押下 ID（ADR-208 決定2 D1）。
    pub press: Option<awase::types::PressId>,
    /// 決定入力。`shadow_on` は executor の `applied_snapshot` の素の値（D1 の未知化の前）。
    pub inputs: DecisionInputs,
    /// `AppImeProfile::is_effectively_tsf_native(class_name)`（Engine 経路の D1 を止める窓か）。
    pub effectively_tsf_native: bool,
}

/// [`dispatch_set_open`] の結果。元の `dispatch_ime_set_open` の分岐と 1 対 1。
#[derive(Debug)]
pub(crate) enum SetOpenDispatch {
    /// InputRelay（gate の拒否。`site=DispatchImeSetOpen` の記録は journal に積んだ）。
    NotOwned,
    /// 同じ押下で既に同じ向きを書いた（BUG-113 の二重送信防止）。書いていない。
    SkipAlreadyClaimed,
    /// ImmCross が先頭で適用可能: 殻が `spawn_local` で非同期チェーンを走らせる。order は押下つきで発行済み。
    AsyncImmCross { order: ActuationOrder },
    /// 同期チェーンを走らせた（予約の解除・記録は済み）。
    SyncChain { outcome: ImeOpenOutcome },
}

/// `ImeEffect::SetOpen` の dispatch の判断（旧 `runtime/executor.rs::dispatch_ime_set_open` の判断部分）。
///
/// 順序は元のまま: D1 の未知化 → gate（拒否なら記録して返す）→ 押下の予約（claim）→ `plan_set_open` →
/// 同期なら [`apply_sync`]、何も送らなければ予約を解く → `caller` を付けて記録。
/// **claim は `ImeStateHub` の台帳を書き換える**ため、gate が `NotOwned` のときは呼ばない
/// （書かない窓では予約しない）。非同期の場合は order を返し、`spawn_local` 以降は殻に残す。
pub(crate) fn dispatch_set_open<S: CommandSink + ?Sized>(
    hub: &mut ImeStateHub,
    request: SetOpenRequest,
    class_name: &str,
    sink: &mut S,
) -> SetOpenDispatch {
    let SetOpenRequest {
        open,
        press,
        mut inputs,
        effectively_tsf_native,
    } = request;
    // D1: 押下の書き込みは `applied` を省略の根拠にしない（`applied` 自体は書き換えない）。ただし TsfNative の窓は
    // BUG-124 型の「@」の実機 A/B（ADR-208 L3'）が済むまで従来のまま（`engine_press_unknowns_applied`）。
    let unknowns_applied = press.is_some() && engine_press_unknowns_applied(effectively_tsf_native);
    inputs.shadow_on = explicit_press_applied_pair(inputs.shadow_on, open, unknowns_applied);
    if matches!(decide_gate(inputs), GateResult::NotOwned) {
        // /code-review指摘（B-3、PR #201）: ADR-163がDecisionSite::
        // DispatchImeSetOpenを新設した理由は、この早期gate（下のimm_first
        // 判定・sync path双方より前の、executor側だけが持つ独立した
        // 判定点）を「Syncに畳むと回帰が記録上区別できなくなる」ため
        // 区別する必要があったから。まだ`ActuationOrder`は
        // 発行されていない（両分岐が自分の理由文字列で個別に発行する）ため、
        // この記録専用に使い捨てのorderを発行する——`ActuationOrder::issue`
        // はA-1（shadow）段階の純粋な読み取りで、発行して`chain`に通さず
        // 破棄しても既存の警告(warrant)会計に副作用は無い
        // （`state/platform_state.rs::issue_actuation_order`のdoc参照）。
        let gate_reject_order =
            hub.issue_self_actuation_order(open, "dispatch_ime_set_open_gate_not_owned");
        let record = ActuationDecisionRecord {
            site: DecisionSite::DispatchImeSetOpen,
            gate_inputs: inputs,
            order: ActuationOrderRecord::from(&gate_reject_order),
            chain: [None; MAX_WRITE_MECHANISMS],
            chain_len: 0,
            attempts: [None; MAX_WRITE_MECHANISMS],
            attempts_len: 0,
            caller: None,
        };
        hub.journal
            .record(crate::journal::JournalEntry::ActuationDecision { record });
        return SetOpenDispatch::NotOwned;
    }
    // ADR-208 D1: この押下で既に書いた（同じ向き）なら書かない。order の発行直前に予約する
    // （ImmCross の async は完了が WM 経由で後から届くため、完了時の記録では同じ打鍵の二重送信を防げない）。
    // 非同期（ImmCross 先頭の窓）は完了が後から届くので、書けなくても予約は解かない（次の押下で直る）。
    // 同期は何も送らなかったときだけ下で解く（`release_press_write`）。
    let claim = hub.claim_press_write(press, open, PressSource::Engine);
    let plan = plan_set_open(SetOpenFacts {
        claim,
        imm_first: imm_cross_is_first_applicable(inputs),
    });
    match plan {
        SetOpenPlan::SkipAlreadyClaimed => {
            tracing::debug!(
                "[dispatch-ime] 同じ押下で既に書いた（{}）→ 書かない press={press:?} open={open}",
                claim.label()
            );
            SetOpenDispatch::SkipAlreadyClaimed
        }
        SetOpenPlan::AsyncImmCross => {
            // ADR-090 §2.A A-1（shadow）: 起案は spawn_local の**外**で行う
            // ——future の中では `with_app` 再入で `ImeStateHub` に届かない（ADR-090 §4.2）。
            let order = hub
                .issue_self_actuation_order(open, "engine_decision_async")
                .with_press(press);
            SetOpenDispatch::AsyncImmCross { order }
        }
        SetOpenPlan::SyncChain => {
            // ── sync path (Chrome / GJI 経路 / TsfNative 経路) ──
            // ADR-090 §2.A A-1（shadow）。
            let order = hub
                .issue_self_actuation_order(open, "engine_decision_sync")
                .with_press(press);
            let (outcome, mut record) = apply_sync(order, inputs, class_name, sink);
            tracing::debug!("[apply-ime] open={open} → outcome={outcome:?}");
            // 同期の書き込みが何も送らなかったなら予約を解く（同じ押下の次の経路が書ける。async は解けない）。
            if outcome_sent_nothing(outcome) {
                hub.release_press_write(press, open);
            }
            // /code-review指摘（B-2、PR #201）: `site`は上書きしない——
            // `decide_attempt`は常に`Sync`で呼ばれておりrecord.siteもSyncの
            // ままである。呼び出し元の識別は独立の`caller`フィールドに記録する。
            record.caller = Some(DecisionSite::DispatchImeSetOpen);
            hub.journal
                .record(crate::journal::JournalEntry::ActuationDecision { record });
            if outcome == ImeOpenOutcome::Failed {
                tracing::warn!("apply_ime_open({open}) failed");
            }
            SetOpenDispatch::SyncChain { outcome }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::class_names::AppImeProfile;
    use crate::state::hub_clock::HubClock;
    use crate::state::ime_kind::ImeKindId;
    use crate::vk::{VK_IME_OFF, VK_IME_ON};
    use awase::engine::InputModeState;
    use awase::types::PressId;

    /// 送られた command を記録し、`outcomes` の順に outcome を返す（尽きたら `Applied`。`None` は `AlreadyMatched`）。
    #[derive(Default)]
    struct FakeSink {
        sent: Vec<(WriteMechanism, Option<MechanismCommand>, bool)>,
        outcomes: Vec<ImeOpenOutcome>,
    }

    impl CommandSink for FakeSink {
        fn send_command(
            &mut self,
            mechanism: WriteMechanism,
            command: Option<MechanismCommand>,
            open: bool,
        ) -> ImeOpenOutcome {
            self.sent.push((mechanism, command, open));
            if command.is_none() {
                return ImeOpenOutcome::AlreadyMatched;
            }
            if self.outcomes.is_empty() {
                ImeOpenOutcome::Applied
            } else {
                self.outcomes.remove(0)
            }
        }
    }

    fn inputs(profile: AppImeProfile, kind: ImeKindId, shadow_on: Option<bool>) -> DecisionInputs {
        DecisionInputs::from_facts(profile, kind, shadow_on, InputModeState::Unknown, false)
    }

    fn hub() -> ImeStateHub {
        ImeStateHub::with_clock(HubClock::manual(1_000_000))
    }

    /// 押下つきの order（押下の授権は IntentStore・`is_japanese_ime` を問わない、ADR-208 D2/D3）。
    fn press_order(hub: &ImeStateHub, open: bool) -> ActuationOrder {
        hub.issue_self_actuation_order(open, "test")
            .with_press(Some(PressId::new(1)))
    }

    fn sync_commands(record: &ActuationDecisionRecord) -> Vec<Option<MechanismCommand>> {
        record.attempts[..record.attempts_len]
            .iter()
            .map(|a| a.expect("attempt").command)
            .collect()
    }

    // ── CoreSyncWriter ──────────────────────────────────────────────

    /// writer は `decide_attempt` の command を sink に渡し、試行の記録に同じ command と sink の outcome を残す。
    /// 候補ウィンドウの証拠（ADR-171）があると、確認済み OFF でも OFF を送る。
    #[test]
    fn core_writer_sends_decided_command_and_records_it() {
        for (candidate, expected) in [
            (false, None),
            (true, Some(MechanismCommand::SendVk(VK_IME_OFF))),
        ] {
            let mut i = inputs(AppImeProfile::TsfNative, ImeKindId::Gji, Some(false));
            i.candidate_was_seen = candidate;
            let mut sink = FakeSink::default();
            let mut w = CoreSyncWriter::new(i, &mut sink);
            let outcome = w.write(WriteMechanism::GjiDirect, false);
            let attempt = w.attempts[0].expect("attempt");
            assert_eq!(w.attempts_len, 1);
            assert_eq!(attempt.command, expected, "candidate={candidate}");
            assert_eq!(attempt.outcome, outcome);
            assert_eq!(attempt.inputs, i);
            assert_eq!(
                sink.sent,
                vec![(WriteMechanism::GjiDirect, expected, false)]
            );
        }
    }

    #[test]
    fn core_writer_applicability_follows_profile_and_kind() {
        let mut sink = FakeSink::default();
        let w = CoreSyncWriter::new(
            inputs(AppImeProfile::Imm32Unavailable, ImeKindId::Gji, None),
            &mut sink,
        );
        assert!(!w.is_applicable(WriteMechanism::ImmCross));
        assert!(w.is_applicable(WriteMechanism::GjiDirect));
        assert!(!w.is_applicable(WriteMechanism::MsImeDirect));
        let i = inputs(AppImeProfile::Standard, ImeKindId::MsIme, None);
        assert!(mechanism_applicable(WriteMechanism::ImmCross, i));
        assert!(!mechanism_applicable(WriteMechanism::GjiDirect, i));
        assert!(mechanism_applicable(WriteMechanism::MsImeDirect, i));
    }

    #[test]
    fn imm_cross_first_only_when_chain_starts_with_applicable_imm_cross() {
        let table = [
            (AppImeProfile::Standard, ImeKindId::Gji, true),
            (AppImeProfile::Standard, ImeKindId::MsIme, true),
            (AppImeProfile::Imm32Unavailable, ImeKindId::Gji, false),
            (AppImeProfile::Imm32Unavailable, ImeKindId::MsIme, false),
            (AppImeProfile::TsfNative, ImeKindId::Gji, false),
            (AppImeProfile::TsfNative, ImeKindId::MsIme, false),
        ];
        for (profile, kind, expected) in table {
            assert_eq!(
                imm_cross_is_first_applicable(inputs(profile, kind, None)),
                expected,
                "{profile:?} × {kind:?}"
            );
        }
    }

    #[test]
    fn candidate_evidence_is_consumed_only_by_a_sent_gji_off() {
        let off = Some(MechanismCommand::SendVk(VK_IME_OFF));
        let gji = WriteMechanism::GjiDirect;
        assert!(consumes_candidate_evidence(
            gji,
            off,
            false,
            ImeOpenOutcome::Applied
        ));
        assert!(!consumes_candidate_evidence(
            gji,
            off,
            false,
            ImeOpenOutcome::UnsafeToToggle
        ));
        assert!(!consumes_candidate_evidence(
            gji,
            None,
            false,
            ImeOpenOutcome::AlreadyMatched
        ));
        let on = Some(MechanismCommand::SendVk(VK_IME_ON));
        assert!(!consumes_candidate_evidence(
            gji,
            on,
            true,
            ImeOpenOutcome::Applied
        ));
        let ms = WriteMechanism::MsImeDirect;
        assert!(!consumes_candidate_evidence(
            ms,
            off,
            false,
            ImeOpenOutcome::Applied
        ));
    }

    // ── apply_sync ──────────────────────────────────────────────────

    /// InputRelay（issue #136 / BUG-90 決定4）: どの機構も試さず `NotOwned`。
    #[test]
    fn apply_sync_input_relay_is_not_owned_without_any_attempt() {
        let h = hub();
        let mut sink = FakeSink::default();
        let i = inputs(AppImeProfile::InputRelay, ImeKindId::Gji, None);
        let (outcome, record) = apply_sync(press_order(&h, false), i, "", &mut sink);
        assert_eq!(outcome, ImeOpenOutcome::NotOwned);
        assert_eq!((record.attempts_len, record.chain_len), (0, 0));
        assert!(sink.sent.is_empty());
    }

    /// 授権の無い order は送らず `Unwarranted`。既定のハブ（IntentStore・観測が空、`ImmCross` の Read 方針で
    /// OwnSsot に落ちない）では、押下なしの order に warrant が下りない。
    #[test]
    fn apply_sync_unwarranted_order_attempts_nothing() {
        let h = hub();
        let mut sink = FakeSink::default();
        let order = h.issue_self_actuation_order(false, "test");
        let i = inputs(AppImeProfile::TsfNative, ImeKindId::Gji, None);
        let (outcome, record) = apply_sync(order, i, "", &mut sink);
        assert_eq!(outcome, ImeOpenOutcome::Unwarranted);
        assert_eq!(record.attempts_len, 0);
        assert!(sink.sent.is_empty());
    }

    /// ImmCross が `Failed` なら次の機構へ進み、試行を 2 件記録する（chain は `caps` の表どおり）。
    #[test]
    fn apply_sync_falls_through_failed_imm_cross_and_records_both_attempts() {
        let h = hub();
        let mut sink = FakeSink {
            outcomes: vec![ImeOpenOutcome::Failed],
            ..FakeSink::default()
        };
        let i = inputs(AppImeProfile::Standard, ImeKindId::MsIme, None);
        let (outcome, record) = apply_sync(press_order(&h, true), i, "", &mut sink);
        assert_eq!(outcome, ImeOpenOutcome::Applied);
        assert_eq!(record.site, DecisionSite::Sync);
        assert_eq!(record.chain_len, 2);
        assert_eq!(
            sync_commands(&record),
            vec![
                Some(MechanismCommand::SetOpenCrossProcessSync(true)),
                Some(MechanismCommand::SendVk(VK_IME_ON)),
            ]
        );
    }

    // ── dispatch_set_open ───────────────────────────────────────────

    fn request(
        profile: AppImeProfile,
        applied: Option<bool>,
        press: Option<u64>,
    ) -> SetOpenRequest {
        SetOpenRequest {
            open: false,
            press: press.map(PressId::new),
            inputs: inputs(profile, ImeKindId::Gji, applied),
            effectively_tsf_native: matches!(profile, AppImeProfile::TsfNative),
        }
    }

    fn sync_outcome(d: SetOpenDispatch) -> ImeOpenOutcome {
        match d {
            SetOpenDispatch::SyncChain { outcome } => outcome,
            other => panic!("SyncChain を期待: {other:?}"),
        }
    }

    /// ADR-208 D1: Chrome（Imm32Unavailable）× GJI で `applied` が既に OFF でも、押下ごとに OFF を送る。
    /// 同じ押下の 2 回目は書かない（BUG-113）。
    #[test]
    fn dispatch_sends_off_for_each_press_even_when_applied_is_already_off() {
        let mut h = hub();
        for press in [1, 2] {
            let mut sink = FakeSink::default();
            let req = request(AppImeProfile::Imm32Unavailable, Some(false), Some(press));
            let outcome = sync_outcome(dispatch_set_open(&mut h, req, "", &mut sink));
            assert_eq!(outcome, ImeOpenOutcome::Applied, "press={press}");
            let off = Some(MechanismCommand::SendVk(VK_IME_OFF));
            assert_eq!(sink.sent, vec![(WriteMechanism::GjiDirect, off, false)]);
        }
        let mut sink = FakeSink::default();
        let req = request(AppImeProfile::Imm32Unavailable, Some(false), Some(2));
        let d = dispatch_set_open(&mut h, req, "", &mut sink);
        assert!(matches!(d, SetOpenDispatch::SkipAlreadyClaimed), "{d:?}");
        assert!(sink.sent.is_empty());
    }

    /// TsfNative では Engine 経路の D1 を止める（ADR-208 L3' 待ち）。確認済み OFF なら送らない。
    #[test]
    fn dispatch_keeps_already_matched_in_tsf_native_and_releases_the_claim() {
        let mut h = hub();
        let mut sink = FakeSink::default();
        let req = request(AppImeProfile::TsfNative, Some(false), Some(1));
        let outcome = sync_outcome(dispatch_set_open(&mut h, req, "", &mut sink));
        assert_eq!(outcome, ImeOpenOutcome::AlreadyMatched);
        assert_eq!(sink.sent, vec![(WriteMechanism::GjiDirect, None, false)]);
        // AlreadyMatched は「送っていない」ではないので予約は残る（同じ押下は 2 回目を書かない）。
        let d = dispatch_set_open(&mut h, req, "", &mut FakeSink::default());
        assert!(matches!(d, SetOpenDispatch::SkipAlreadyClaimed), "{d:?}");
    }

    /// 何も送らなかった（`UnsafeToToggle`）同期の書き込みは予約を解き、同じ押下で書き直せる。
    #[test]
    fn dispatch_releases_the_claim_when_the_sync_write_sent_nothing() {
        let mut h = hub();
        let mut sink = FakeSink {
            outcomes: vec![ImeOpenOutcome::UnsafeToToggle],
            ..FakeSink::default()
        };
        let req = request(AppImeProfile::Imm32Unavailable, None, Some(7));
        let outcome = sync_outcome(dispatch_set_open(&mut h, req, "", &mut sink));
        assert_eq!(outcome, ImeOpenOutcome::UnsafeToToggle);
        let again = dispatch_set_open(&mut h, req, "", &mut FakeSink::default());
        assert_eq!(sync_outcome(again), ImeOpenOutcome::Applied);
    }

    /// gate の拒否は予約に触れず、ImmCross が先頭の窓は押下つきの order を返して同期チェーンを走らせない。
    #[test]
    fn dispatch_gate_and_async_branches() {
        let mut h = hub();
        let mut sink = FakeSink::default();
        let relay = request(AppImeProfile::InputRelay, None, Some(1));
        let d = dispatch_set_open(&mut h, relay, "", &mut sink);
        assert!(matches!(d, SetOpenDispatch::NotOwned), "{d:?}");
        let standard = request(AppImeProfile::Standard, None, Some(1));
        match dispatch_set_open(&mut h, standard, "", &mut sink) {
            SetOpenDispatch::AsyncImmCross { order } => {
                assert_eq!(order.press(), Some(PressId::new(1)));
                assert!(!order.open());
            }
            other => panic!("AsyncImmCross を期待（gate は予約しない）: {other:?}"),
        }
        assert!(sink.sent.is_empty());
    }
}
