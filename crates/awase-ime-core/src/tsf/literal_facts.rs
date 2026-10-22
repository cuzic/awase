//! literal-detect の判定結果を journal へ持ち上げるための純粋データ型。

use crate::state::event_origin::Generation;
use crate::state::focus_gen::FocusGen;

#[derive(strum::IntoStaticStr, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LiteralVerdict {
    CompositionConfirmed,
    SuspectedLiteral,
    StaleConfirm,
    VetoExpired,
    SessionSkip,
    PlanSkippedLiteral,
    AbortedNoVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectRoute {
    CheckNow,
    VisibleFencing,
    SessionFlag,
    PlanDecision,
    ProbeEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectPath {
    PerVk,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectTarget {
    Chrome,
    Tsf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct DetectEvidence {
    pub show_changed: bool,
    pub candidate_visible: bool,
    pub write_delta: u64,
    pub evidence_fresh: bool,

    // ── ここから診断専用フィールド（BUG-75、2026-08-25 追加）──
    //
    // BUG-75（GJI StaleConfirm 回収がromaji全体を再送し「っつかって」のように
    // 促音が増える不具合）の対話設計（Sonnet + Opus 2体、6ラウンド）で検討した
    // 複数の恒久対策案が、それぞれ以下のような実機データを必要とすると判明した。
    // suffix再送方式（一度実装しdevelopにマージしたが致命的欠陥が見つかり
    // revertした、docs/known-bugs.md BUG-75 追補参照）のように「証拠のない
    // 仮定」で実装してから壊れることを繰り返さないため、まずはこのフィールド
    // 群を判定ロジックに一切使わず記録するだけに留め、タスクトレイ不具合報告
    // （ADR-095）経由で実機データが集まってから設計判断する。
    //
    // 各フィールドがどの案の判断材料かは以下の通り:
    /// `GetProcessIoCounters`（既存の public Win32 API、`gji_monitor.rs` が
    /// 既に10msごとにサンプリングしている）の `WriteOperationCount` 差分
    /// （送信前ベースラインから、この verdict 確定時点まで）。
    ///
    /// `write_delta`（バイト量、350B閾値で cold/warm を区別する既存の確認
    /// シグナル）と違い、書き込み"回数"は量に依存しない。子音単体の
    /// per-VK confirm は write_delta が閾値に届かないことがある（BUG-27
    /// 追補5）ため、より粒度の細かい確認シグナルとして機能しうるか実機で
    /// 検証する（write_ops活用案）。
    pub write_ops_delta: u64,
    /// 同 `ReadOperationCount` 差分。cold-start の辞書再読込等の補助情報。
    pub read_ops_delta: u64,
    /// 同 `OtherOperationCount` 差分（パイプ・セクション経由 IPC 等が計上される）。
    pub other_ops_delta: u64,
    /// `gji_last_write_ms()` の生値（verdict 確定時点）。0 = 未観測。
    /// `epoch_send_ms`/`deadline_ms` と突き合わせることで、grace延長案の
    /// 判断材料（実際どれだけの遅延で write evidence が追いついたか、
    /// または追いつかなかったか）になる。
    pub last_write_ms: u64,
    /// この detector の送信時刻（`LiteralDetector::epoch_send_ms`）。
    pub epoch_send_ms: u64,
    /// この VK/word の literal-detect deadline（`plan.literal_detect_ms` 由来）。
    /// grace（`LiteralDetector::EPOCH_FENCE_GRACE_MS`）が deadline に先んじて
    /// verdict を確定させたか、deadline 到達で確定したかを区別する判断材料。
    pub deadline_ms: u64,
    /// SHOW-only confirm の猶予（`EPOCH_FENCE_GRACE_MS`）を実際にどれだけ
    /// 保持してから verdict が確定したか（ms）。`None` = 猶予自体に入らな
    /// かった（write_confirmed 等で即断、または `check_now`/
    /// `visible_fencing_verdict` を経由しない verdict）。
    ///
    /// grace延長案（`EPOCH_FENCE_GRACE_MS` を実測ベースで延ばす）の判断材料。
    /// 現行値20msに対し実際どれだけ待てば`evidence_fresh`になっていたかを
    /// 複数の実機報告から集計できる。
    pub grace_hold_ms: Option<u64>,
    /// この verdict 確定時点で、同一 `cold_seq`（コンポジションセッション）内の
    /// 別のモーラが既に confirm 済みだったか
    /// （`crate::tsf::observer::literal_session_confirmed`）。
    ///
    /// 「session内で最初のモーラだけ ESC 先行が安全」という案（session状態
    /// ベース判断案）の判断材料。BUG-39 により、フォーカス変更等をまたいで
    /// stale になりうる既知の不正確さがあることに注意。
    pub literal_session_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectFacts {
    pub verdict: LiteralVerdict,
    pub route: DetectRoute,
    pub path: DetectPath,
    pub target: DetectTarget,
    pub vk: Option<u16>,
    pub idx: u16,
    pub last_idx: u16,
    pub evidence: DetectEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectRecord {
    pub cold_seq: Generation,
    pub facts: LiteralDetectFacts,
    pub consecutive_before: u32,
    pub gave_up: bool,
    pub backs: usize,
    pub escape_composition: bool,
    pub session_marked: bool,
    /// BUG-74/ADR-100 決定3 案L: `RawTsfLiteralRecovery`（初回疑い・give-up 双方）で
    /// 送信対象だった romaji。`None` はこの verdict が romaji を持たない（`Composition
    /// Confirmed`/旧 `LiteralDetectNote`/`PlanSkippedLiteral`/`AbortedNoVerdict`）ことを表す
    /// — 空文字列との混同（「記録し忘れ」なのか「そもそも romaji を持たない verdict」
    /// なのか区別できなくなる）を避けるため、`String::new()` ではなく `Option` にする。
    ///
    /// give-up（`gave_up=true`）で romaji が失われる（backspace のみ、再送なし）場合
    /// でも、この記録には**送信予定だった元の romaji**を残す。ADR-100 決定3 が
    /// 「give-up 分岐に reinit 完了確認後の retry を追加する」提案2 を却下した代わりに
    /// 採用した対策（完了通知経路が存在しない・focus 世代照合が未整備 (F6) 等、
    /// 却下理由の詳細は ADR-100 参照）。次に同種の文字消失が報告されたとき、
    /// journal からどの romaji が失われたかを機械可読に復元できるようにする。
    pub romaji: Option<String>,
}

/// give-up の証拠(ADR-227): 外部から実 IME が閉じられたと疑う根拠。runtime が読み直し(`follow_external_change`)の
/// きっかけにするだけで、閉の観測としては書かない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GiveUpEvidence {
    pub cold_seq: u64,
    /// 一連の literal 疑いの**最初の VK 送信時**の `Output::ime_mode_focus_gen`。取り出し時の世代と一致しなければ捨てる。
    pub focus_gen: FocusGen,
}

/// 一連の literal 疑いの**1回目**(`SuspectedLiteral`、まだ give-up でない)の証拠(ADR-258 段階0)。
/// runtime が「この時点で arm したら、読みは何だったか」を journal に**記録するだけ**(arm しない・belief を動かさない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstLiteralEvidence {
    pub cold_seq: u64,
    /// 一連の literal 疑いの最初の VK 送信時の `Output::ime_mode_focus_gen`(`GiveUpEvidence` と同じ)。
    pub focus_gen: FocusGen,
    /// 記録時点の `consecutive_count`(0 でなければ、外部クローズへの追随の後など、前の連鎖の数え残りがある)。
    /// 「連鎖の1回目」は tracker の連鎖(`suspected == 0`)で決め、`consecutive_count` は集計側で分けられるよう記録に残す。
    pub consecutive_before: u32,
}

/// `LiteralDetectRecord` の列から give-up の証拠を判定する純粋な状態機械(ADR-227)。
///
/// **途切れずに続いた** `SuspectedLiteral` が 2 回以上あり、最新の記録が give-up(`gave_up`)のときだけ証拠を返す。
/// `CompositionConfirmed` と `StaleConfirm` はどちらも連鎖を切る(リセットする)。StaleConfirm を「以後ずっと拒否」の
/// ラッチにすると、無関係な過去の StaleConfirm(CI の setup で出た)が以後の連鎖をすべて拒否した(ADR-227 の検証で判明)。
/// `consecutive` は StaleConfirm でも増えるので使わない(ADR-200 決定1 の否定的証拠と同じ数え方)。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GiveUpTracker {
    suspected: u32,
    focus_gen: Option<FocusGen>,
}

impl GiveUpTracker {
    /// VK を送ったとき(`LiteralDetectTraceItem::VkSent` の取り込み時)に呼ぶ。最初の送信時の世代だけ覚える。
    ///
    /// 保持している世代と違う世代で送られたら(フォーカスが変わった)連鎖を切って数え直す。古い窓の世代と数えかけの
    /// 回数を新しい窓へ持ち越すと、新しい窓での最初の追随が世代の不一致で捨てられ 2 打鍵遅れる(PR #480 Opus r1 M1)。
    pub fn note_vk_sent(&mut self, focus_gen: FocusGen) {
        if self.focus_gen.is_some_and(|g| g != focus_gen) {
            *self = Self::default();
        }
        self.focus_gen.get_or_insert(focus_gen);
    }

    /// 1回目の literal 疑い(ADR-258 段階0)の証拠を返す。**内部状態を変えない**(`note_record` の前に呼ぶ)。
    ///
    /// 条件: verdict が `SuspectedLiteral`、まだ give-up でない(`gave_up == false`)、この tracker の連鎖で最初(`suspected == 0`)、
    /// かつ `note_vk_sent` で最初の送信時の世代を覚えている。`consecutive_before`(`consecutive_count`)は条件に使わない:
    /// 外部クローズへの追随の後に IME を戻した場合などに、前の連鎖の数え残りで 1 以上になるが、それも「1回目」として記録したい
    /// (ADR-258 の Opus レビュー M1)。give-up の証拠(`note_record` の戻り値)とは独立で、同じ連鎖の2回目の記録では返さない。
    #[must_use]
    pub fn first_literal_evidence(
        &self,
        record: &LiteralDetectRecord,
    ) -> Option<FirstLiteralEvidence> {
        if record.facts.verdict != LiteralVerdict::SuspectedLiteral
            || record.gave_up
            || self.suspected != 0
        {
            return None;
        }
        Some(FirstLiteralEvidence {
            cold_seq: record.cold_seq.value(),
            focus_gen: self.focus_gen?,
            consecutive_before: record.consecutive_before,
        })
    }

    /// verdict の記録を 1 件取り込む。条件を満たす give-up なら証拠を返し、内部状態を空に戻す。
    pub fn note_record(&mut self, record: &LiteralDetectRecord) -> Option<GiveUpEvidence> {
        match record.facts.verdict {
            // 連鎖を切る: 本物の確定、誤検出の疑い(StaleConfirm)、literal セッションの確定(SessionSkip)、
            // literal 判定のスキップ(PlanSkippedLiteral)。
            LiteralVerdict::CompositionConfirmed
            | LiteralVerdict::StaleConfirm
            | LiteralVerdict::SessionSkip
            | LiteralVerdict::PlanSkippedLiteral => {
                *self = Self::default();
                None
            }
            // 無視する(切りも数えもしない): 候補窓 veto の期限切れ、判定が出なかった中断(これらは literal の有無を言わない)。
            LiteralVerdict::VetoExpired | LiteralVerdict::AbortedNoVerdict => None,
            LiteralVerdict::SuspectedLiteral => {
                self.suspected += 1;
                if !(record.gave_up && self.suspected >= 2) {
                    return None;
                }
                let focus_gen = self.focus_gen?;
                *self = Self::default();
                Some(GiveUpEvidence {
                    cold_seq: record.cold_seq.value(),
                    focus_gen,
                })
            }
        }
    }
}

/// give-up 証拠を受けたときの runtime の判断(純関数。Windows 専用コードから切り出して Linux でテストする)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveUpFollowDecision {
    /// GJI × Imm32Unavailable の窓でない。
    NotApplicable,
    /// プローブ開始時から focus 世代が変わった。
    StaleFocus,
    /// 明示意図が ON でない。
    NoExplicitIntent,
    /// 監視窓を arm して読み直す。
    Arm,
}

impl GiveUpFollowDecision {
    #[must_use]
    pub const fn outcome(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::StaleFocus => "stale_focus",
            Self::NoExplicitIntent => "no_explicit_intent",
            Self::Arm => "armed",
        }
    }

    /// 1回目の literal(ADR-258 段階0)で記録する outcome。`Arm` は実際には arm していない(仮定)ので別の名前にする
    /// (`GiveUpFollow` の `outcome` と混ぜないため)。
    #[must_use]
    pub const fn first_literal_outcome(self) -> &'static str {
        match self {
            Self::NotApplicable => "first_literal_not_applicable",
            Self::StaleFocus => "first_literal_stale_focus",
            Self::NoExplicitIntent => "first_literal_no_explicit_intent",
            Self::Arm => "first_literal_would_arm",
        }
    }
}

#[must_use]
pub fn giveup_follow_decision(
    applies: bool,
    gen_at_probe: FocusGen,
    gen_now: FocusGen,
    explicit_intent: Option<bool>,
) -> GiveUpFollowDecision {
    if !applies {
        GiveUpFollowDecision::NotApplicable
    } else if gen_at_probe != gen_now {
        GiveUpFollowDecision::StaleFocus
    } else if explicit_intent != Some(true) {
        GiveUpFollowDecision::NoExplicitIntent
    } else {
        GiveUpFollowDecision::Arm
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum LiteralDetectTraceItem {
    VkSent {
        cold_seq: u64,
        vk: u16,
        idx: u16,
        last_idx: u16,
        target: DetectTarget,
    },
    Verdict(LiteralDetectRecord),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectTrace(pub Vec<LiteralDetectTraceItem>);

#[cfg(test)]
mod giveup_tracker_tests {
    use super::*;

    fn rec(verdict: LiteralVerdict, gave_up: bool) -> LiteralDetectRecord {
        LiteralDetectRecord {
            cold_seq: Generation::new(7),
            facts: LiteralDetectFacts {
                verdict,
                route: DetectRoute::CheckNow,
                path: DetectPath::PerVk,
                target: DetectTarget::Tsf,
                vk: Some(0x4B),
                idx: 0,
                last_idx: 0,
                evidence: DetectEvidence::default(),
            },
            consecutive_before: u32::from(gave_up),
            gave_up,
            backs: 1,
            escape_composition: false,
            session_marked: false,
            romaji: Some("ka".into()),
        }
    }

    #[test]
    fn first_literal_evidence_is_only_for_the_first_ungave_suspected_of_a_chain_and_does_not_mutate(
    ) {
        let mut t = GiveUpTracker::default();
        // 送信を覚える前は返さない(世代が無い)。
        assert_eq!(
            t.first_literal_evidence(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        t.note_vk_sent(FocusGen::new(5));
        let before = t.clone();
        assert_eq!(
            t.first_literal_evidence(&rec(LiteralVerdict::SuspectedLiteral, false)),
            Some(FirstLiteralEvidence {
                cold_seq: 7,
                focus_gen: FocusGen::new(5),
                consecutive_before: 0
            })
        );
        assert_eq!(t, before, "内部状態を変えない");
        // 確定・StaleConfirm・give-up は返さない。
        for v in [
            LiteralVerdict::CompositionConfirmed,
            LiteralVerdict::StaleConfirm,
            LiteralVerdict::SessionSkip,
            LiteralVerdict::PlanSkippedLiteral,
            LiteralVerdict::VetoExpired,
            LiteralVerdict::AbortedNoVerdict,
        ] {
            assert_eq!(t.first_literal_evidence(&rec(v, false)), None, "{v:?}");
        }
        assert_eq!(
            t.first_literal_evidence(&rec(LiteralVerdict::SuspectedLiteral, true)),
            None,
            "give-up の記録(consecutive_before が 1)は2回目"
        );
        // 1回目を数えた後(note_record)は、同じ連鎖では返さない。
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        assert_eq!(
            t.first_literal_evidence(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None,
            "連鎖の途中(数え済み)では返さない"
        );
    }

    /// Opus レビュー M1/M4: `consecutive_before > 0` でも(外部クローズへの追随の後に IME を戻した場合の数え残り)、
    /// tracker の連鎖の1回目(`suspected == 0`)なら返し、`consecutive_before` を証拠に載せる。連鎖の途中(数え済み)では返さない。
    #[test]
    fn first_literal_evidence_uses_the_trackers_chain_not_consecutive_before_and_carries_it() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(5));
        let mut r = rec(LiteralVerdict::SuspectedLiteral, false);
        r.consecutive_before = 3; // 前の連鎖の数え残り(gave_up は偽)
        assert_eq!(
            t.first_literal_evidence(&r),
            Some(FirstLiteralEvidence {
                cold_seq: 7,
                focus_gen: FocusGen::new(5),
                consecutive_before: 3
            })
        );
        // tracker が既に数えた連鎖の途中(suspected > 0)なら、consecutive_before が 0 でも返さない。
        assert_eq!(t.note_record(&r), None);
        r.consecutive_before = 0;
        assert_eq!(t.first_literal_evidence(&r), None);
    }

    /// 配線の取り違え対策(Opus レビュー M4): 「送信 → 1回目 → 再送 → give-up」の列で、1回目だけが first-literal の証拠になり、
    /// give-up の証拠は従来どおり2回目で出る(`note_record` の入力・戻り値は変わらない)。
    #[test]
    fn a_full_chain_yields_first_literal_evidence_once_and_giveup_evidence_unchanged() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(5));
        let first = rec(LiteralVerdict::SuspectedLiteral, false);
        let fl = t.first_literal_evidence(&first);
        assert_eq!(fl.map(|e| e.cold_seq), Some(7));
        assert_eq!(t.note_record(&first), None);
        t.note_vk_sent(FocusGen::new(5));
        let second = rec(LiteralVerdict::SuspectedLiteral, true);
        assert_eq!(
            t.first_literal_evidence(&second),
            None,
            "2回目は証拠にならない"
        );
        assert_eq!(
            t.note_record(&second),
            Some(GiveUpEvidence {
                cold_seq: 7,
                focus_gen: FocusGen::new(5)
            }),
            "give-up の証拠は従来どおり"
        );
    }

    #[test]
    fn first_literal_outcome_names_are_distinct_from_giveup_outcomes() {
        use GiveUpFollowDecision::*;
        for d in [NotApplicable, StaleFocus, NoExplicitIntent, Arm] {
            assert!(d.first_literal_outcome().starts_with("first_literal_"));
            assert_ne!(d.first_literal_outcome(), d.outcome());
        }
        assert_eq!(Arm.first_literal_outcome(), "first_literal_would_arm");
    }

    #[test]
    fn two_suspected_literals_ending_in_give_up_yield_evidence_with_the_send_focus_gen() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(5));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        t.note_vk_sent(FocusGen::new(5)); // 再送(同じ窓)
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            Some(GiveUpEvidence {
                cold_seq: 7,
                focus_gen: FocusGen::new(5)
            })
        );
        // 証拠を返したら空に戻る
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            None
        );
    }

    #[test]
    fn a_single_suspected_literal_is_not_enough() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            None
        );
    }

    /// 連鎖の途中に StaleConfirm が入ったら連鎖は切れる(ADR-200 の高速打鍵の誤検出型)。
    #[test]
    fn stale_confirm_in_the_middle_of_the_chain_blocks_evidence() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::StaleConfirm, false)),
            None
        );
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            None
        );
    }

    /// 連鎖が始まる前の(無関係な)StaleConfirm は以後をラッチしない(CI の setup で出た StaleConfirm が全試行を拒否した)。
    #[test]
    fn stale_confirm_before_the_chain_does_not_latch() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::StaleConfirm, false)),
            None
        );
        t.note_vk_sent(FocusGen::new(2));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            Some(GiveUpEvidence {
                cold_seq: 7,
                focus_gen: FocusGen::new(2)
            })
        );
    }

    #[test]
    fn composition_confirmed_resets_the_chain() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::CompositionConfirmed, false)),
            None
        );
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            None
        );
    }

    #[test]
    fn without_a_recorded_send_there_is_no_focus_gen_so_no_evidence() {
        let mut t = GiveUpTracker::default();
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            None
        );
    }

    /// フォーカスが変わった(世代が違う)VK 送信で、古い窓の数えかけを持ち越さない(PR #480 Opus r1 M1)。
    #[test]
    fn a_new_focus_generation_restarts_the_chain() {
        let mut t = GiveUpTracker::default();
        t.note_vk_sent(FocusGen::new(1));
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        t.note_vk_sent(FocusGen::new(2)); // 窓が変わった
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
            None
        );
        assert_eq!(
            t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
            Some(GiveUpEvidence {
                cold_seq: 7,
                focus_gen: FocusGen::new(2)
            })
        );
    }

    #[test]
    fn session_skip_and_plan_skipped_literal_cut_the_chain() {
        for v in [
            LiteralVerdict::SessionSkip,
            LiteralVerdict::PlanSkippedLiteral,
        ] {
            let mut t = GiveUpTracker::default();
            t.note_vk_sent(FocusGen::new(1));
            assert_eq!(
                t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
                None
            );
            assert_eq!(t.note_record(&rec(v, false)), None);
            t.note_vk_sent(FocusGen::new(1));
            assert_eq!(
                t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true)),
                None,
                "{v:?}"
            );
        }
    }

    #[test]
    fn veto_expired_and_aborted_are_neutral() {
        for v in [
            LiteralVerdict::VetoExpired,
            LiteralVerdict::AbortedNoVerdict,
        ] {
            let mut t = GiveUpTracker::default();
            t.note_vk_sent(FocusGen::new(1));
            assert_eq!(
                t.note_record(&rec(LiteralVerdict::SuspectedLiteral, false)),
                None
            );
            assert_eq!(t.note_record(&rec(v, false)), None);
            assert!(
                t.note_record(&rec(LiteralVerdict::SuspectedLiteral, true))
                    .is_some(),
                "{v:?}"
            );
        }
    }

    #[test]
    fn follow_decision_checks_applicability_then_focus_then_intent() {
        use GiveUpFollowDecision::*;
        assert_eq!(
            giveup_follow_decision(false, FocusGen::new(1), FocusGen::new(1), Some(true)),
            NotApplicable
        );
        assert_eq!(
            giveup_follow_decision(true, FocusGen::new(1), FocusGen::new(2), Some(true)),
            StaleFocus
        );
        assert_eq!(
            giveup_follow_decision(true, FocusGen::new(1), FocusGen::new(1), None),
            NoExplicitIntent
        );
        assert_eq!(
            giveup_follow_decision(true, FocusGen::new(1), FocusGen::new(1), Some(false)),
            NoExplicitIntent
        );
        assert_eq!(
            giveup_follow_decision(true, FocusGen::new(1), FocusGen::new(1), Some(true)),
            Arm
        );
        assert_eq!(Arm.outcome(), "armed");
    }
}
