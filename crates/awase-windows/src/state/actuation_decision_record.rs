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
pub const MAX_WRITE_MECHANISMS: usize = 4;

/// `Option<Option<bool>>` の3値（未記録／記録済みだが値不明／記録済みで既知）を
/// `{"recorded":bool,"value":Option<bool>}` という常に固定サイズのオブジェクトへ
/// 展開する代わりに、`null`／`"unknown"`／素の`bool`という自己記述的な最小表現へ
/// 直接写す（ADR-163 Part D N-1対応、2026-09-11）。この値は`docs/journal-replay-guide.md`
/// 「ActuationDecisionコーパスの扱い」が明記するとおり人間がbug reportを直接読んで
/// 根本原因特定に使うためのものなので、数値コード（例: `0`/`1`/`2`）ではなく文字列
/// `"unknown"`を選び、圧縮と可読性を両立させる。
mod nested_optional_bool {
    use serde::de::{Error, Unexpected, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    #[allow(
        clippy::option_option,
        clippy::ref_option,
        clippy::trivially_copy_pass_by_ref
    )]
    pub(super) fn serialize<S>(
        value: &Option<Option<bool>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            None => serializer.serialize_none(),
            Some(None) => serializer.serialize_str("unknown"),
            Some(Some(b)) => serializer.serialize_bool(*b),
        }
    }

    struct TriStateVisitor;

    impl Visitor<'_> for TriStateVisitor {
        type Value = Option<Option<bool>>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("null, \"unknown\", or a bool")
        }

        fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bool<E: Error>(self, v: bool) -> Result<Self::Value, E> {
            Ok(Some(Some(v)))
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
            if v == "unknown" {
                Ok(Some(None))
            } else {
                Err(E::invalid_value(Unexpected::Str(v), &self))
            }
        }
    }

    #[allow(clippy::option_option)]
    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Option<bool>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(TriStateVisitor)
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
pub enum EventSourceKind {
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
pub struct EventOriginRecord {
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
pub struct ActuationOrderRecord {
    pub open: bool,
    /// A-1 shadow authorization の測定値（`log_shadow_warrant`が使う値と同一）。
    pub would_have_blocked: bool,
    pub origin: EventOriginRecord,
}

impl From<&crate::state::actuation_chain::ActuationOrder> for ActuationOrderRecord {
    /// `ime_controller.rs`と`runtime/open_chain.rs`が独立に持っていた同一実装の
    /// `order_record`関数を統合した（/code-review指摘、PR #201）。
    fn from(order: &crate::state::actuation_chain::ActuationOrder) -> Self {
        Self {
            open: order.open(),
            would_have_blocked: order.would_have_blocked(),
            origin: EventOriginRecord::from(order.origin()),
        }
    }
}

/// 1機構への1回のwrite判断の記録（ADR-163 Part B「スキーマはsite単位ではなく
/// attempt単位」節）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttemptRecord {
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
    #[serde(with = "nested_optional_bool")]
    pub shadow_on_before_bug113_override: Option<Option<bool>>,
    /// `ActuationOutcome::Failed`後の`read_ime_state_fast()`再観測結果。
    /// 「未取得」（外側`None`）と「取得してfalse」（`Some(Some(false))`）を
    /// 区別する（BUG-113と同型の罠、round2 T3。`Option<bool>`に潰さないこと）。
    // `runtime/ime_refresh.rs::ir_stage_focus`と同じ理由でネストする
    // `Option`が必須（`clippy::option_option`は意図的に無視する）。
    #[serde(with = "nested_optional_bool")]
    pub post_failed_reobservation: Option<Option<bool>>,
}

/// actuation合流点1呼び出し分の決定点ジャーナルレコード
/// （ADR-163 Part B、`ActuationDecisionRecord`）。
///
/// # ワイヤ表現は固定長配列をそのまま出さない（ADR-163 Part D N-1対応）
///
/// `chain`/`attempts`はホットパス（`ImeController::apply`等）でヒープ確保を
/// 避けるため固定長`[Option<_>; MAX_WRITE_MECHANISMS]`で持つ（ADR-163 round1 B4）
/// が、この型そのものに`#[derive(Serialize, Deserialize)]`を付けると、未使用
/// スロットの`null`と`chain_len`/`attempts_len`の冗長フィールドがJSON表現に
/// そのまま出て1レコード約631バイトに膨らみ（`journal.rs`のActuation lane予約
/// （全体の20%）を既存の`ImeActuation`等と奪い合う——実測はこのファイルの
/// `actuation_decision_record_json_byte_size_is_measured`参照）、実ユーザーの
/// bug report経由コーパスが溜まるほどlaneの実効容量を圧迫する。
///
/// メモリ上の表現（固定長・`Copy`）とワイヤ表現（可変長・ヒープ確保あり）を
/// 分離するため、`Serialize`/`Deserialize`は`derive`せず[`ActuationDecisionRecordWire`]
/// を介した手書き実装にしている。ワイヤ側の変換自体はI/O層（journalダンプ時、
/// ホットパスではない）でのみ発生するため、B4が守ろうとした制約（構築時に
/// ヒープ確保しない）とは抵触しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActuationDecisionRecord {
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
    /// このレコードを実際に記録した呼び出し元（provenance）。`site`とは意味が
    /// 異なるフィールドとして分離している（/code-review指摘 B-2、PR #201）。
    ///
    /// `site`は「`decide_gate`/`decide_chain`/`decide_attempt`にどの
    /// `DecisionSite`を渡して決定を計算したか」を表し、`replay_record`の
    /// chain再導出（siteがSyncのときのみ）・ImmCross command再計算スキップ
    /// 判定（siteがSync以外のときスキップ）が直接この値を見る。`caller`は
    /// これとは独立に「実際にどの関数がこのレコードを作ったか」という
    /// 診断ラベルで、再生の一致検証には一切使わない。
    ///
    /// 当初`site`自体を呼び出し元ラベルへ事後上書きしていたが、それだと
    /// `ImeController::apply`経由（常に`site=Sync`で`decide_attempt`を
    /// 呼ぶ）のレコードのうち`reassert_explicit_physical_key`/
    /// `force_on_and_correct_romaji`由来の分だけ`site`が`Sync`でなくなり、
    /// `replay_record`のchain再導出とImmCross command再計算がその分
    /// スキップされ、実際には`Sync`で計算された正当な値の検証が
    /// 無効化されていた（同期記録点6箇所中3箇所、`dispatch_ime_set_open`の
    /// 主経路を含む）。`caller`に分離することでこの穴を塞ぐ。
    pub caller: Option<DecisionSite>,
}

const _: () = assert!(size_of::<ActuationDecisionRecord>() <= 184);
const _: () = {
    const fn assert_copy<T: Copy>() {}
    assert_copy::<AttemptRecord>();
    assert_copy::<ActuationDecisionRecord>();
};

/// [`ActuationDecisionRecord`]のワイヤ専用ミラー（ADR-163 Part D N-1対応）。
///
/// `chain`/`attempts`を固定長`[Option<_>; MAX_WRITE_MECHANISMS]`のまま
/// シリアライズすると未使用スロットの`null`がそのまま出力される。ここでは
/// 実際に埋まっている`chain_len`/`attempts_len`件分だけを`Vec`として持ち、
/// 長さそのものを`chain_len`/`attempts_len`フィールドの代わりに使う。
#[derive(serde::Serialize, serde::Deserialize)]
struct ActuationDecisionRecordWire {
    site: DecisionSite,
    gate_inputs: DecisionInputs,
    order: ActuationOrderRecord,
    chain: Vec<WriteMechanism>,
    attempts: Vec<AttemptRecord>,
    caller: Option<DecisionSite>,
}

impl From<&ActuationDecisionRecord> for ActuationDecisionRecordWire {
    fn from(record: &ActuationDecisionRecord) -> Self {
        Self {
            site: record.site,
            gate_inputs: record.gate_inputs,
            order: record.order,
            chain: record.chain[..record.chain_len]
                .iter()
                .copied()
                .flatten()
                .collect(),
            attempts: record.attempts[..record.attempts_len]
                .iter()
                .copied()
                .flatten()
                .collect(),
            caller: record.caller,
        }
    }
}

/// ワイヤの`Vec`長が[`MAX_WRITE_MECHANISMS`]を超えていた場合のエラー
/// （改ざんされた、または将来`MAX_WRITE_MECHANISMS`が縮小されたフィクスチャの
/// デシリアライズ時のみ発生しうる）。
#[derive(Debug)]
struct WireLenOverflow {
    field: &'static str,
    len: usize,
}

impl std::fmt::Display for WireLenOverflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} has {} entries, exceeding MAX_WRITE_MECHANISMS={MAX_WRITE_MECHANISMS}",
            self.field, self.len
        )
    }
}

fn fixed_array_from_vec<T: Copy>(
    field: &'static str,
    values: Vec<T>,
) -> Result<([Option<T>; MAX_WRITE_MECHANISMS], usize), WireLenOverflow> {
    if values.len() > MAX_WRITE_MECHANISMS {
        return Err(WireLenOverflow {
            field,
            len: values.len(),
        });
    }
    let len = values.len();
    let mut array = [None; MAX_WRITE_MECHANISMS];
    for (slot, value) in array.iter_mut().zip(values) {
        *slot = Some(value);
    }
    Ok((array, len))
}

impl TryFrom<ActuationDecisionRecordWire> for ActuationDecisionRecord {
    type Error = WireLenOverflow;

    fn try_from(wire: ActuationDecisionRecordWire) -> Result<Self, Self::Error> {
        let (chain, chain_len) = fixed_array_from_vec("chain", wire.chain)?;
        let (attempts, attempts_len) = fixed_array_from_vec("attempts", wire.attempts)?;
        Ok(Self {
            site: wire.site,
            gate_inputs: wire.gate_inputs,
            order: wire.order,
            chain,
            chain_len,
            attempts,
            attempts_len,
            caller: wire.caller,
        })
    }
}

impl serde::Serialize for ActuationDecisionRecord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ActuationDecisionRecordWire::from(self).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ActuationDecisionRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ActuationDecisionRecordWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

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
        //
        // /code-review指摘（S-6、PR #201）: 本PR自身が上記の「先頭要素が
        // 必ず適用可能」という前提から外れた具体的な経路を作った——
        // `run_open_chain_async`の冒頭gateは`with_app`成功だが（gate自体は
        // Proceed）、その後`imm_cross_write`/`fallback_write`内側の
        // `with_app`が全機構でNoneを返すfail-openケース（`runtime/
        // open_chain.rs`のB-1修正参照）では、`attempts_len == 0`のまま
        // `Proceed`なレコードが記録されうる。この場合`expected_gate`は
        // `NotOwned`（誤り）になり`gate mismatch`が報告される——実装の
        // バグではなく、この逆算ロジックの既知の誤検知パターンである。
        // 163-T8でfixtureを投入する際は、この組合せ（gate=Proceed、
        // attempts_len=0）を「既知の誤検知」として扱うこと。
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
                     != recorded {recorded_chain:?}"
                ));
            }
        }

        for (i, attempt) in used_attempts(record).iter().enumerate() {
            let Some(attempt) = attempt else {
                failures.push(format!("attempt[{i}] is empty within attempts_len"));
                continue;
            };
            // `shadow_on_before_bug113_override`の値そのものは`outcome`と同じ
            // 「外部入力として記録し、再計算しない」フィールドであり、上書き前の
            // 値が何だったかを独立に再導出する手段は無い。ただし
            // `Some(_)`（＝上書きが発生した）ときは、上書き後に組み立てられた
            // `attempt.inputs.shadow_on`が必ず`None`になるという構造的な
            // 事実は再生時に検証できる（`runtime/open_chain.rs::fallback_write`が
            // `view.control.shadow_on = None;`の**後**に`inputs`を組み立てる
            // ため）。
            //
            // /code-review指摘（PR #201）: 当初はここで
            // `before_override != attempt.inputs.shadow_on`という一致確認を
            // 行っていたが構造的に誤りだった——`attempt.inputs.shadow_on`は
            // 上書き後の値（常に`None`）であり、上書き**前**の値
            // `before_override`と比較すると、上書き前の値が既知
            // （`Some(true)`/`Some(false)`）だった実機コーパスの全件が
            // 「不一致」と誤検出されていた。上記の正しい不変条件に置き換えた。
            if attempt.shadow_on_before_bug113_override.is_some()
                && attempt.inputs.shadow_on.is_some()
            {
                failures.push(format!(
                    "attempt[{i}] shadow_on_before_bug113_override is Some \
                     (override happened) but attempt.inputs.shadow_on is \
                     {:?} instead of None (fallback_write always overrides \
                     shadow_on to None before building inputs)",
                    attempt.inputs.shadow_on
                ));
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
            caller: None,
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
            caller: None,
        };

        let json = serde_json::to_string(&record).expect("serialize");
        let back: ActuationDecisionRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    // N-1対応（`ActuationDecisionRecordWire`導入）で新設したバリデーション。
    // 改ざん・または将来`MAX_WRITE_MECHANISMS`が縮小されたフィクスチャで
    // `chain`/`attempts`が上限を超えていた場合、固定長配列への詰め直しで
    // 静かに切り詰めるのではなくデシリアライズ自体をエラーにする。
    #[test]
    fn deserialize_rejects_chain_longer_than_max_write_mechanisms() {
        let json = r#"{
            "site": "Sync",
            "gate_inputs": {"profile":"Standard","kind":"Gji","shadow_on":null,"belief_input_mode":"Unknown"},
            "order": {"open":true,"would_have_blocked":false,"origin":{"source":"Physical","epoch":0}},
            "chain": ["ImmCross","GjiDirect","MsImeDirect","KanjiToggle","ImmCross"],
            "attempts": [],
            "caller": null
        }"#;
        let result: Result<ActuationDecisionRecord, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "chainがMAX_WRITE_MECHANISMSを超える場合はデシリアライズが失敗するはず"
        );
    }

    // /code-review指摘（S-4、PR #201）: 「LaneKind::Actuationへの相乗りが
    // 既存ImeActuation/DriftGiveUpDiagnostic/ConvClassifyCallエントリを
    // 押し出すペースを悪化させないか」の実測は、windows-build CIや実機
    // ダンプが無くてもLinux上のJSONバイト数計測で今すぐ着手できる
    // （指摘のとおり「実測はCI待ち」は不要な先送りだった）。1 actuationに
    // つき既存`ImeActuation`と合わせ同一laneに2エントリ積まれる点は本テスト
    // の範囲外（実際のlane圧迫の実測はwindows-build CI/実機ダンプでの前後
    // 比較が別途必要、163-T1dの受け入れ基準に残したまま）。
    //
    // N-1対応（2026-09-11、docs/adr/163-implementation-tasks.md）:
    // 当初631バイトだった実測値を、ワイヤ表現の圧縮（`chain`/`attempts`を
    // `null`パディング済み固定長配列のまま出さず、埋まっている分だけの`Vec`
    // として直列化し`chain_len`/`attempts_len`を廃止する
    // [`ActuationDecisionRecordWire`]、`nested_optional_bool`を
    // `{"recorded":..,"value":..}`オブジェクトから`null`/`"unknown"`/素の
    // `bool`へ圧縮）で523バイトへ縮小した。
    #[test]
    fn actuation_decision_record_json_byte_size_is_measured() {
        let gate_inputs = inputs(
            AppImeProfile::Standard,
            ImeKindId::Gji,
            Some(true),
            InputModeState::Unknown,
        );
        let (chain, chain_len) = chain_from_slice(decide_chain(gate_inputs));
        // 実運用で最頻出と見込む構成: attemptsは1件のみ埋まり残り3スロットは
        // null（GjiDirect/MsImeDirectはchain中1機構だけで already-matched/
        // 送信が決まることが多い）。
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
                shadow_on_before_bug113_override: Some(Some(true)),
                post_failed_reobservation: Some(Some(true)),
            }])
            .0,
            attempts_len: 1,
            caller: None,
        };
        let json = serde_json::to_string(&record).expect("serialize");
        // 実測値（2026-09-11時点、フィールド構成が変わったら更新すること）:
        // attempt 1件の構成で523バイト（N-1対応前は631バイト、上記コメント
        // 参照）。journal.rsの`select_tail_within_budget`はlane予約20%
        // （Actuation）の中で既存`ImeActuation`（固定サイズ数十バイト）と
        // 奪い合うため、1 actuationあたりのlane消費バイト数は本エントリの
        // 追加でおよそ7〜8倍規模になる——この数値をwindows-build CI/実機
        // ダンプでの前後比較（163-T1d受け入れ基準）の基準値として使うこと。
        assert!(
            json.len() < 560,
            "ActuationDecisionRecordのJSON表現が想定より大きい: {} bytes ({json})",
            json.len()
        );
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
            caller: None,
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
            caller: None,
        };
        assert!(
            !replay_record(&record).is_empty(),
            "改ざんしたcommandはreplay_recordが不一致として検出するはず"
        );
    }

    #[test]
    // /code-review指摘（S-2、PR #201）: F1修正前は「上書き前の値の改ざん」を
    // 検出するテストのつもりだったが、F1修正後の不変条件（shadow_on_before_
    // bug113_overrideがSomeなら、attempt.inputs.shadow_onは必ずNoneのはず）
    // のもとでは、実際に検出しているのは「値の改ざん」ではなく「上書きが
    // 発生したはずなのにinputs.shadow_onがNoneでない」という構造的な矛盾
    // （fallback_writeでは起こり得ない組合せ）である。テスト名を実態に
    // 合わせて訂正した（163-T0の受け入れ基準「上書き前の値の改ざんを検出」も
    // 同じ理由で原理的に達成不可能——上書き前の値を独立に再導出する手段が
    // 無いため——docs/adr/163-implementation-tasks.mdに記録済み）。
    fn replay_detects_bug113_override_recorded_but_inputs_shadow_on_not_none() {
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
                // fallback_writeでは上書きが発生したattemptのinputs.shadow_on
                // は必ずNoneになる（open_chain.rs::fallback_write参照）。
                // ここでは意図的にinputs.shadow_on（gate_inputs由来のSome(true)）
                // と矛盾させている。
                shadow_on_before_bug113_override: Some(Some(false)),
                post_failed_reobservation: None,
            }])
            .0,
            attempts_len: 1,
            caller: None,
        };
        assert!(
            !replay_record(&record).is_empty(),
            "shadow_on_before_bug113_overrideがSomeなのにinputs.shadow_onが\
             Noneでない矛盾はreplay_recordが検出するはず"
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
            caller: None,
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
