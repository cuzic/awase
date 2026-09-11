//! ADR-163 Part B（TH1c）: attempt単位の決定点ジャーナルスキーマと
//! crate内`#[cfg(test)]`再生ハーネス。
//!
//! `state::ime_actuation_decision`（TH1b/Part A）が提供する
//! `decide_gate`/`decide_chain`/`decide_attempt`は「何を送るか」を決める
//! 純粋関数だが、これまで記録済みの決定列を再生して回帰させる仕組みが
//! 無かった（ADR-163背景節）。本モジュールは
//!
//! 1. 1回の actuation 合流点呼び出し（`ImeController::apply`/
//!    `run_open_chain_async`/`dispatch_ime_set_open`）を1レコードとする
//!    attempt単位の決定点ジャーナルスキーマ（[`ActuationDecisionRecord`]/
//!    [`AttemptRecord`]）、
//! 2. `tests/journals/actuation_decision/*.json`（TH1dで実機ダンプから
//!    投入予定、本タスク時点では未投入）を読み、記録済みの
//!    `decide_gate`/`decide_chain`/`decide_attempt`の入力から
//!    同じ関数を再度呼んで記録済みの判定・chain・commandと一致するかを
//!    確認するcrate内再生ハーネス（[`tests`]モジュール）
//!
//! を提供する。
//!
//! # 可視性: crate内`#[cfg(test)]`として置く（ADR-163 round1 M9）
//!
//! `DecisionInputs`/`DecisionSite`/`MechanismCommand`（`state::
//! ime_actuation_decision`）はいずれも`pub(crate)`で、外部crate扱いの
//! `tests/*.rs` integration testからは参照できない。`tests/
//! drift_correction_replay.rs`のような外部テストにはできず、
//! `state::actuation_chain::tests`と同じ`#[cfg(test)] mod`パターンを
//! 踏襲する。
//!
//! # スコープ外（誤読防止）
//!
//! - **journal.rsへの実配線はしない**（ADR-163「コーパスの置き場所と運用」節）。
//!   `ActuationDecisionRecord`はここに定義するfixture専用型であり、
//!   `journal.rs::JournalEntry`とは独立に持つ。実機での記録・凍結コーパスの
//!   初回投入はTH1dのスコープ。
//! - **ImmCross×非Sync siteのcommand再計算はしない**。`decide_attempt`は
//!   この組合せに対して常に`None`を返す設計（呼び出し元が`ImmCrossOp`を
//!   別途組み立てる、`ime_actuation_decision.rs`のdoc参照）であり、
//!   `open_chain.rs`自体もまだ`decide_attempt`経由に統合されていない
//!   （その統合はTH1eのスコープ）。このためこの組合せの`AttemptRecord`は
//!   記録済み`command`をそのまま信用し、再計算による一致確認はスキップする
//!   （[`tests::replay_record`]のコメント参照）。
//! - **`ImmCrossWrite`/`RunOpenChainAsync`/`DispatchImeSetOpen`のImmCross
//!   attemptはTH1e完了まで自動差分証明の対象外**。これらは記録済み
//!   `command`を人間の診断材料として保持するが、現時点の再生ハーネスでは
//!   command再計算による一致確認を行わない。

use awase::platform::ImeOpenOutcome;
use std::mem::size_of;

use super::actuation_chain::WriteMechanism;
use super::event_origin::{EventOrigin, EventSource, Generation};
use super::ime_actuation_decision::{DecisionInputs, DecisionSite, MechanismCommand};

/// ADR-163 D2: `WriteMechanism::ALL`と同じ最大attempt数。
pub(crate) const MAX_WRITE_MECHANISMS: usize = 4;

mod nested_optional_bool {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Encoded {
        recorded: bool,
        value: Option<bool>,
    }

    pub(super) fn serialize<S>(
        value: &Option<Option<bool>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Encoded {
            recorded: value.is_some(),
            value: value.unwrap_or(None),
        }
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Option<bool>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = Encoded::deserialize(deserializer)?;
        Ok(if encoded.recorded {
            Some(encoded.value)
        } else {
            None
        })
    }
}

/// [`EventOrigin`]の出所を、`&'static str`を含まない判別子だけで表したもの
/// （ADR-163 Part B「`ActuationOrderRecord`の借用問題」節）。
///
/// `EventSource::Injected { reason }`/`SelfActuated { strategy }`の
/// `&'static str`ペイロードは保存しない——`event_origin.rs`が明記する
/// とおり、任意入力から`&'static str`を復元するDeserializeは型として
/// 表現できない。判別子だけで十分な理由: 再生ハーネスが検証したいのは
/// 「どの経路からの起案か（物理/注入/自己駆動）」という分岐であって、
/// 注入理由や戦略名の文字列そのものではない
/// （`state::ime_actuation::ActuationRecord`の回避策と同型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum EventSourceKind {
    Physical,
    Injected,
    SelfActuated,
}

impl From<EventSource> for EventSourceKind {
    fn from(source: EventSource) -> Self {
        match source {
            EventSource::Physical => Self::Physical,
            EventSource::Injected { .. } => Self::Injected,
            EventSource::SelfActuated { .. } => Self::SelfActuated,
        }
    }
}

/// [`EventOrigin`]のfixture専用ミラー（`&'static str`を含まないためDeserialize可能）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct EventOriginRecord {
    pub source: EventSourceKind,
    pub epoch: Generation,
}

impl From<EventOrigin> for EventOriginRecord {
    fn from(origin: EventOrigin) -> Self {
        Self {
            source: EventSourceKind::from(origin.source),
            epoch: origin.epoch,
        }
    }
}

/// `ActuationOrder`のfixture専用ミラー（ADR-163 Part B「`ActuationOrderRecord`の
/// 借用問題」節）。`ActuationOrder`自体はprivateフィールドのみでDeserializeを
/// 導出できないため、公開アクセサ（`open()`/`would_have_blocked()`/`origin()`）
/// が返す値だけをここに集める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ActuationOrderRecord {
    pub open: bool,
    /// A-1 shadow authorization の測定値（`log_shadow_warrant`が使う値と同一）。
    pub would_have_blocked: bool,
    pub origin: EventOriginRecord,
}

/// 1機構への1回のwrite判断の記録（ADR-163 Part B「スキーマはsite単位ではなく
/// attempt単位」節）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AttemptRecord {
    /// このattempt時点で再サンプリングされた決定入力。
    pub inputs: DecisionInputs,
    /// `with_app(...).unwrap_or(false)`のfail-open結果（決定ロジックからは
    /// 導出不能な外部入力、ADR-163 round1 S3・S6）。
    pub with_app_available: bool,
    pub mechanism: WriteMechanism,
    /// `None` = already-matchedで送信しなかった。
    pub command: Option<MechanismCommand>,
    /// 実`ImeOpenOutcome`。外部入力として記録し、再計算しない。
    pub outcome: ImeOpenOutcome,
    /// BUG-113追補で`view.control.shadow_on = None`へ上書きする直前の値。
    /// 「上書きなし」（外側`None`）と「上書き前の値が未知」（`Some(None)`）を
    /// 区別するため、`post_failed_reobservation`と同じ二重`Option`で保持する。
    #[expect(clippy::option_option)]
    #[serde(with = "nested_optional_bool")]
    pub shadow_on_before_bug113_override: Option<Option<bool>>,
    /// `ActuationOutcome::Failed`後の`read_ime_state_fast()`再観測結果。
    /// 「未取得」（外側`None`）と「取得してfalse」（`Some(Some(false))`）を
    /// 区別する（BUG-113と同型の罠、round2 T3。`Option<bool>`に潰さないこと）。
    // `runtime/ime_refresh.rs::ir_stage_focus`と同じ理由でネストする
    // `Option`が必須（`clippy::option_option`は意図的に無視する）。
    #[expect(clippy::option_option)]
    #[serde(with = "nested_optional_bool")]
    pub post_failed_reobservation: Option<Option<bool>>,
}

/// actuation合流点1呼び出し分の決定点ジャーナルレコード
/// （ADR-163 Part B、`ActuationDecisionRecord`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ActuationDecisionRecord {
    pub site: DecisionSite,
    /// `decide_gate`/`decide_chain`（siteがSyncの場合のみ再導出、round2 T2）を
    /// 評価する際に使った決定入力。sync経路では全attemptがこのviewを共有する
    /// ため`attempts[0].inputs`と一致するが、async経路（`open_chain.rs`が
    /// attemptごとに`shadow_ime_control_view()`を作り直す、round1 M3）では
    /// 各attemptの`inputs`と異なりうる——ゲート判定時点の1回だけのサンプルを
    /// 独立して保持する。
    pub gate_inputs: DecisionInputs,
    pub order: ActuationOrderRecord,
    /// 使用したchain。syncは`decide_chain(gate_inputs)`との一致を再生時に
    /// assertする。asyncは`WriteMechanism::ALL`固定（ADR-159の理由により
    /// 変更しない、round2 T2）ため記録値をそのまま使う。
    pub chain: [Option<WriteMechanism>; MAX_WRITE_MECHANISMS],
    pub chain_len: usize,
    /// `GateResult::NotOwned`だった場合は空（chainの走査自体が起きない）。
    pub attempts: [Option<AttemptRecord>; MAX_WRITE_MECHANISMS],
    pub attempts_len: usize,
}

const _: () = assert!(size_of::<ActuationDecisionRecord>() <= 176);
const _: () = {
    const fn assert_copy<T: Copy>() {}
    assert_copy::<AttemptRecord>();
    assert_copy::<ActuationDecisionRecord>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::class_names::AppImeProfile;
    use crate::state::conv_after_open::ConvAfterOpenId;
    use crate::state::ime_actuation_decision::{
        decide_attempt, decide_chain, decide_gate, GateResult,
    };
    use crate::state::ime_kind::ImeKindId;
    use awase::engine::InputModeState;
    use awase::types::VkCode;

    // ── EventSourceKind / EventOriginRecord ─────────────────────────────────

    #[test]
    fn event_source_kind_discards_payload_but_keeps_variant() {
        assert_eq!(
            EventSourceKind::from(EventSource::Physical),
            EventSourceKind::Physical
        );
        assert_eq!(
            EventSourceKind::from(EventSource::Injected { reason: "x" }),
            EventSourceKind::Injected
        );
        assert_eq!(
            EventSourceKind::from(EventSource::SelfActuated { strategy: "y" }),
            EventSourceKind::SelfActuated
        );
    }

    #[test]
    fn event_origin_record_round_trips_via_json() {
        let origin = EventOrigin::new(
            EventSource::SelfActuated {
                strategy: "drift_correction_blind",
            },
            Generation::new(3),
        );
        let record = EventOriginRecord::from(origin);
        let json = serde_json::to_string(&record).expect("serialize");
        let back: EventOriginRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
        assert_eq!(back.source, EventSourceKind::SelfActuated);
        assert_eq!(back.epoch, Generation::new(3));
    }

    // ── 再生ドライバ ─────────────────────────────────────────────────────────

    fn used_chain(record: &ActuationDecisionRecord) -> &[Option<WriteMechanism>] {
        assert!(record.chain_len <= MAX_WRITE_MECHANISMS);
        &record.chain[..record.chain_len]
    }

    fn used_attempts(record: &ActuationDecisionRecord) -> &[Option<AttemptRecord>] {
        assert!(record.attempts_len <= MAX_WRITE_MECHANISMS);
        &record.attempts[..record.attempts_len]
    }

    fn chain<const N: usize>(
        mechanisms: [WriteMechanism; N],
    ) -> ([Option<WriteMechanism>; MAX_WRITE_MECHANISMS], usize) {
        assert!(N <= MAX_WRITE_MECHANISMS);
        let mut chain = [None; MAX_WRITE_MECHANISMS];
        for (index, mechanism) in mechanisms.into_iter().enumerate() {
            chain[index] = Some(mechanism);
        }
        (chain, N)
    }

    fn chain_from_slice(
        mechanisms: &[WriteMechanism],
    ) -> ([Option<WriteMechanism>; MAX_WRITE_MECHANISMS], usize) {
        assert!(mechanisms.len() <= MAX_WRITE_MECHANISMS);
        let mut chain = [None; MAX_WRITE_MECHANISMS];
        for (index, mechanism) in mechanisms.iter().copied().enumerate() {
            chain[index] = Some(mechanism);
        }
        (chain, mechanisms.len())
    }

    fn attempts<const N: usize>(
        records: [AttemptRecord; N],
    ) -> ([Option<AttemptRecord>; MAX_WRITE_MECHANISMS], usize) {
        assert!(N <= MAX_WRITE_MECHANISMS);
        let mut attempts = [None; MAX_WRITE_MECHANISMS];
        for (index, record) in records.into_iter().enumerate() {
            attempts[index] = Some(record);
        }
        (attempts, N)
    }

    /// 1レコード分の再生。`decide_gate`/`decide_chain`/`decide_attempt`を
    /// 記録済み入力へ再度通し、記録済みの判定・chain・commandと不一致な点を
    /// 文字列のVecとして返す（空なら全一致）。
    fn replay_record(record: &ActuationDecisionRecord) -> Vec<String> {
        let mut failures = Vec::new();

        let gate = decide_gate(record.gate_inputs);
        // `attempts.is_empty()` から`GateResult`を逆算できるのは、現行コードで
        // `is_applicable`（`ImmCrossProcessStrategy`/`GjiDirectStrategy`/
        // `MsImeDirectStrategy`の3実装）が`profile`/`kind`——いずれも
        // `DecisionInputs`に含まれる——にしか依存せず、`caps(profile, kind)`
        // （sync）・`WriteMechanism::ALL`（async）のいずれのchainも先頭要素が
        // 必ず適用可能になるよう構成されているためである。この結合が将来
        // 崩れる（`is_applicable`がDecisionInputs外の値に依存するようになる、
        // または`caps`が非適用要素を含むchainを返すようになる）と、
        // 「chainはあるが全機構が非適用で1件もwriteしない」という正当な
        // `Proceed`かつ空`attempts`のレコードが本チェックで誤検知されうる。
        // TH1dで実機ダンプを投入した際にこの理由でgate mismatchが出た場合は、
        // この逆算そのものを見直すこと（この分岐を無条件に信用しない）。
        let expected_gate = if record.attempts_len == 0 {
            GateResult::NotOwned
        } else {
            GateResult::Proceed
        };
        if gate != expected_gate {
            failures.push(format!(
                "gate mismatch: decide_gate(gate_inputs)={gate:?}, \
                 attempts.is_empty()={} から期待される値は{expected_gate:?}",
                record.attempts_len == 0
            ));
        }

        if record.site == DecisionSite::Sync && gate == GateResult::Proceed {
            let recomputed = decide_chain(record.gate_inputs);
            let recorded_chain: Vec<WriteMechanism> =
                used_chain(record).iter().filter_map(|m| *m).collect();
            if recomputed != recorded_chain.as_slice() {
                failures.push(format!(
                    "chain mismatch (Sync): decide_chain(gate_inputs)={recomputed:?} \
                     != recorded {:?}",
                    recorded_chain
                ));
            }
        }

        for (i, attempt) in used_attempts(record).iter().enumerate() {
            let Some(attempt) = attempt else {
                failures.push(format!("attempt[{i}] is empty within attempts_len"));
                continue;
            };
            if let Some(before_override) = attempt.shadow_on_before_bug113_override {
                if before_override != attempt.inputs.shadow_on {
                    failures.push(format!(
                        "attempt[{i}] shadow_on_before_bug113_override mismatch: \
                         recorded before-override value {before_override:?} \
                         != attempt.inputs.shadow_on {:?}",
                        attempt.inputs.shadow_on
                    ));
                }
            }
            if attempt.mechanism == WriteMechanism::ImmCross && record.site != DecisionSite::Sync {
                // モジュールdoc「スコープ外」節参照: この組合せはdecide_attemptの
                // 責務外（常にNoneを返す設計）であり、再計算による一致確認は
                // まだできない。
                continue;
            }
            let (_, command) = decide_attempt(
                attempt.inputs,
                record.site,
                attempt.mechanism,
                record.order.open,
            );
            if command != attempt.command {
                failures.push(format!(
                    "attempt[{i}] command mismatch: decide_attempt(..)={command:?} \
                     != recorded {:?}",
                    attempt.command
                ));
            }
        }

        failures
    }

    fn inputs(
        profile: AppImeProfile,
        kind: ImeKindId,
        shadow_on: Option<bool>,
        belief_input_mode: InputModeState,
    ) -> DecisionInputs {
        DecisionInputs {
            profile,
            kind,
            shadow_on,
            belief_input_mode,
        }
    }

    fn order(open: bool) -> ActuationOrderRecord {
        ActuationOrderRecord {
            open,
            would_have_blocked: false,
            origin: EventOriginRecord::from(EventOrigin::new(
                EventSource::Physical,
                Generation::INITIAL,
            )),
        }
    }

    #[test]
    fn replay_accepts_a_hand_built_sync_gji_direct_record() {
        let gate_inputs = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        let (chain, chain_len) = chain_from_slice(decide_chain(gate_inputs));
        let record = ActuationDecisionRecord {
            site: DecisionSite::Sync,
            gate_inputs,
            order: order(true),
            chain,
            chain_len,
            attempts: attempts([AttemptRecord {
                inputs: gate_inputs,
                with_app_available: true,
                mechanism: WriteMechanism::GjiDirect,
                command: decide_attempt(
                    gate_inputs,
                    DecisionSite::Sync,
                    WriteMechanism::GjiDirect,
                    true,
                )
                .1,
                outcome: ImeOpenOutcome::Applied,
                shadow_on_before_bug113_override: None,
                post_failed_reobservation: None,
            }])
            .0,
            attempts_len: 1,
        };
        assert_eq!(
            replay_record(&record),
            Vec::<String>::new(),
            "手で組み立てた自己無矛盾なレコードは再生で一致するはず"
        );
    }

    #[test]
    fn actuation_decision_record_round_trips_via_json() {
        let gate_inputs = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        let (chain, chain_len) = chain_from_slice(decide_chain(gate_inputs));
        let record = ActuationDecisionRecord {
            site: DecisionSite::Sync,
            gate_inputs,
            order: order(true),
            chain,
            chain_len,
            attempts: attempts([AttemptRecord {
                inputs: gate_inputs,
                with_app_available: true,
                mechanism: WriteMechanism::GjiDirect,
                command: Some(MechanismCommand::SendVk(VkCode(0x16))),
                outcome: ImeOpenOutcome::Applied,
                shadow_on_before_bug113_override: Some(None),
                post_failed_reobservation: Some(Some(true)),
            }])
            .0,
            attempts_len: 1,
        };

        let json = serde_json::to_string(&record).expect("serialize");
        let back: ActuationDecisionRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    #[test]
    fn replay_accepts_a_not_owned_gate_record_with_empty_attempts() {
        let gate_inputs = inputs(
            AppImeProfile::InputRelay,
            ImeKindId::Gji,
            None,
            InputModeState::Unknown,
        );
        let record = ActuationDecisionRecord {
            site: DecisionSite::Sync,
            gate_inputs,
            order: order(true),
            chain: [None; MAX_WRITE_MECHANISMS],
            chain_len: 0,
            attempts: [None; MAX_WRITE_MECHANISMS],
            attempts_len: 0,
        };
        assert_eq!(replay_record(&record), Vec::<String>::new());
    }

    #[test]
    fn replay_detects_a_tampered_command() {
        let gate_inputs = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(true), // already matches open=true → 本来 command は None
            InputModeState::Unknown,
        );
        let (chain, chain_len) = chain_from_slice(decide_chain(gate_inputs));
        let record = ActuationDecisionRecord {
            site: DecisionSite::Sync,
            gate_inputs,
            order: order(true),
            chain,
            chain_len,
            attempts: attempts([AttemptRecord {
                inputs: gate_inputs,
                with_app_available: true,
                mechanism: WriteMechanism::GjiDirect,
                // 意図的に誤った記録値（本来はNoneのはず）。
                command: Some(MechanismCommand::SendVk(VkCode(0x16))),
                outcome: ImeOpenOutcome::Applied,
                shadow_on_before_bug113_override: None,
                post_failed_reobservation: None,
            }])
            .0,
            attempts_len: 1,
        };
        assert!(
            !replay_record(&record).is_empty(),
            "改ざんしたcommandはreplay_recordが不一致として検出するはず"
        );
    }

    #[test]
    fn replay_detects_a_tampered_bug113_before_override_value() {
        let gate_inputs = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(true),
            InputModeState::Unknown,
        );
        let record = ActuationDecisionRecord {
            site: DecisionSite::FallbackWrite,
            gate_inputs,
            order: order(true),
            chain: chain(WriteMechanism::ALL).0,
            chain_len: WriteMechanism::ALL.len(),
            attempts: attempts([AttemptRecord {
                inputs: gate_inputs,
                with_app_available: true,
                mechanism: WriteMechanism::GjiDirect,
                command: decide_attempt(
                    gate_inputs,
                    DecisionSite::FallbackWrite,
                    WriteMechanism::GjiDirect,
                    true,
                )
                .1,
                outcome: ImeOpenOutcome::Applied,
                // 意図的に誤った記録値（attempt.inputs.shadow_onはSome(true)）。
                shadow_on_before_bug113_override: Some(Some(false)),
                post_failed_reobservation: None,
            }])
            .0,
            attempts_len: 1,
        };
        assert!(
            !replay_record(&record).is_empty(),
            "改ざんしたBUG-113上書き前値はreplay_recordが不一致として検出するはず"
        );
    }

    #[test]
    fn replay_skips_command_recheck_for_imm_cross_at_non_sync_sites() {
        // モジュールdoc「スコープ外」節: この組合せはdecide_attemptの責務外の
        // ためスキップされ、記録済みcommandがどんな値でも再生は失敗しない。
        let gate_inputs = inputs(
            AppImeProfile::Standard,
            ImeKindId::MsIme,
            None,
            InputModeState::Unknown,
        );
        let record = ActuationDecisionRecord {
            site: DecisionSite::RunOpenChainAsync,
            gate_inputs,
            order: order(true),
            chain: chain(WriteMechanism::ALL).0,
            chain_len: WriteMechanism::ALL.len(),
            attempts: attempts([AttemptRecord {
                inputs: gate_inputs,
                with_app_available: true,
                mechanism: WriteMechanism::ImmCross,
                command: Some(MechanismCommand::SetOpenThenConvForTarget {
                    open: true,
                    conv_after_open: ConvAfterOpenId::Write(None),
                }),
                outcome: ImeOpenOutcome::Applied,
                shadow_on_before_bug113_override: None,
                post_failed_reobservation: None,
            }])
            .0,
            attempts_len: 1,
        };
        assert_eq!(replay_record(&record), Vec::<String>::new());
    }

    // ── ディレクトリ走査（TH1d投入後に効き始める）───────────────────────────

    fn fixture_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/actuation_decision")
    }

    fn load_fixtures(path: &std::path::Path) -> Vec<ActuationDecisionRecord> {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("フィクスチャ読み込み失敗 {}: {e}", path.display()));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("フィクスチャのJSONパース失敗 {}: {e}", path.display()))
    }

    /// `tests/journals/actuation_decision/*.json`（TH1dで投入予定）を再生する。
    /// ディレクトリが存在しない間（TH1c時点）はTH1d未着手として黙って通す
    /// ——フィクスチャが実在するようになった時点で「1件もない」ことを拒否する
    /// ガード（`assert!(total > 0)`相当）を足すのはTH1dの作業とする。
    #[test]
    fn replay_all_actuation_decision_fixtures() {
        let dir = fixture_dir();
        if !dir.exists() {
            return;
        }
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{} が読めない: {e}", dir.display()))
            .map(|entry| entry.expect("dir entry read failed").path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect();
        paths.sort();

        let mut failures = Vec::new();
        for path in &paths {
            for record in load_fixtures(path) {
                for failure in replay_record(&record) {
                    failures.push(format!(
                        "[{}] {failure}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} 件のactuation決定リプレイ不一致:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
