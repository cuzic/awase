//! DD2-05(`docs/tasks/mech-ecrs-2026-10-21/dd2-stale.md`): apply 完了の受理判定の全列挙(回帰テスト)。
//!
//! `completion_can_update_applied` は、かつて `pending.focus_epoch == current_epoch`(fence の epoch)で「この apply は
//! フォーカス変更より前に払い出したものか」を見ていた。ECRS DD2-05 で `ImeTransition.focus_epoch` を撤去し、
//! `pending.generation >= focus_generation_watermark` の 1 本にした。このテストは、撤去前の判定(epoch 版)を**旧判定の
//! oracle としてテスト内に残し**、実物の `ImeModel::reduce` へイベント列(Req / Succ / Focus / Init / Expire)を全列挙で流して、
//! 各状態の完了(generation × outcome × open)で、本番の `classify_apply_completion` が oracle と一致することを示す。
//! oracle は pending を張ったときの fence の epoch を、テスト側(`St::req_epochs`)で覚える。
//!
//! 現実の前提: generation は払い出し順に単調増加し(`Req` は `last_seen` より大きい)、`InitialFocusScopeEstablished` は
//! 起動時の 1 度きりで最初の apply 要求より前。後者は本番コードが `debug_assert` で表明する
//! (後に来ると fence の epoch だけが進み watermark が進まず、古い pending の完了を受理しうる。全列挙で見つけた唯一の反例)。
//! 完了の generation が未払い出しでも一致することも確かめる(`loose`)。

// 本番コード走査のガード(`production_code_only`)が `#[cfg(test)] mod tests` から先を落とすので、中身はここに置く。
#[cfg(test)]
mod tests {

    use super::super::*;
    use crate::state::exhaustive_seq::explore;
    use crate::state::probe_admission::FocusFence;
    use awase::platform::ImeOpenOutcome;
    use std::time::Duration;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Ev {
        /// `ImeApplyRequested { generation, target }`
        Req(u64, bool),
        /// `ImeApplySucceeded { generation, target }`
        Succ(u64, bool),
        /// `FocusChanged`(epoch を 1 進める)
        Focus,
        /// `InitialFocusScopeEstablished`(fence 未確立のときだけ)
        Init,
        /// pending のタイムアウトを越えて時計を進める(無害な `ChordEnded` で purge を起こす)
        Expire,
    }

    #[derive(Clone)]
    struct St {
        model: ImeModel,
        base: Instant,
        clock_ms: u64,
        seq: u64,
        epoch: u64,
        /// oracle 用: apply 要求(`Req`)を張ったときの fence の epoch(generation → epoch)。
        req_epochs: Vec<(u64, u64)>,
    }

    impl St {
        fn new() -> Self {
            Self {
                model: ImeModel::new(),
                base: Instant::now(),
                clock_ms: 0,
                seq: 0,
                epoch: 0,
                req_epochs: Vec::new(),
            }
        }

        fn dispatch(&mut self, event: ImeEvent) {
            self.seq += 1;
            let env = ImeEventEnvelope {
                time: EventTime {
                    seq: self.seq,
                    monotonic: self.base + Duration::from_millis(self.clock_ms),
                    tick_ms: self.clock_ms,
                },
                event,
            };
            self.model.reduce(&env);
        }
    }

    fn gn(n: u64) -> ApplyGeneration {
        ApplyGeneration::new(n).expect("nonzero")
    }

    fn issued(st: &St, g: u64) -> bool {
        st.model.last_seen_generation.is_some_and(|l| gn(g) <= l)
    }

    fn step(st: &St, ev: Ev) -> Option<St> {
        let mut s = st.clone();
        match ev {
            Ev::Req(g, target) => {
                // 払い出し順に単調増加する(本番の採番)。
                if issued(st, g) {
                    return None;
                }
                let fence_epoch = s.model.observations.current_fence().epoch;
                s.req_epochs.push((g, fence_epoch));
                s.dispatch(ImeEvent::ImeApplyRequested {
                    target,
                    generation: gn(g),
                    ctrl_held: false,
                });
            }
            Ev::Succ(g, target) => {
                // 完了は払い出し済みの generation にだけ届く。
                if !issued(st, g) {
                    return None;
                }
                s.dispatch(ImeEvent::ImeApplySucceeded {
                    target,
                    generation: gn(g),
                });
            }
            Ev::Focus => {
                s.epoch += 1;
                let to = HwndId(0x100 + s.epoch as usize);
                let epoch = s.epoch;
                s.dispatch(ImeEvent::FocusChanged {
                    from: None,
                    to,
                    profile: ImePolicyProfile::ImmCross,
                    focus_epoch: epoch,
                });
            }
            Ev::Init => {
                // `establish_initial_fence` の前提(fence 未確立)。本番は起動時に 1 度だけ。
                if st.model.observations.current_fence() != FocusFence::default() {
                    return None;
                }
                if st.model.last_seen_generation.is_some() {
                    return None;
                }
                s.epoch += 1;
                let to = HwndId(0x100 + s.epoch as usize);
                let epoch = s.epoch;
                s.dispatch(ImeEvent::InitialFocusScopeEstablished {
                    to,
                    profile: ImePolicyProfile::ImmCross,
                    focus_epoch: epoch,
                });
            }
            Ev::Expire => {
                s.clock_ms += crate::tuning::IME_APPLY_PENDING_TIMEOUT_MS + 1;
                s.dispatch(ImeEvent::ChordEnded {
                    kind: ChordKind::CtrlMuhenkanImeOff,
                });
            }
        }
        Some(s)
    }

    /// 撤去前の `completion_can_update_applied`(epoch 版)の写し。pending の epoch は `St::req_epochs` から引く。
    fn old_accept(
        st: &St,
        m: &ImeModel,
        open: bool,
        outcome: ImeOpenOutcome,
        generation: ApplyGeneration,
    ) -> ImeApplyAcceptance {
        if matches!(
            outcome,
            ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned | ImeOpenOutcome::Unwarranted
        ) {
            return ImeApplyAcceptance::NotSent;
        }
        let Some(pending) = m.pending.as_ref() else {
            return ImeApplyAcceptance::Stale;
        };
        let current_epoch = m.observations.current_fence().epoch;
        let pending_epoch = st
            .req_epochs
            .iter()
            .rev()
            .find(|(g, _)| gn(*g) == pending.generation)
            .map(|(_, e)| *e);
        let pending_in_epoch = pending_epoch == Some(current_epoch);
        if pending.generation == generation {
            if pending_in_epoch {
                ImeApplyAcceptance::Accepted
            } else {
                ImeApplyAcceptance::Stale
            }
        } else if (outcome.wrote_open_state() || outcome == ImeOpenOutcome::AlreadyMatched)
            && generation >= m.focus_generation_watermark
            && pending_in_epoch
            && pending.target == open
            && m.applied.applied_open() != Some(open)
        {
            ImeApplyAcceptance::Superseded
        } else {
            ImeApplyAcceptance::Stale
        }
    }

    const OUTCOMES: [ImeOpenOutcome; 5] = [
        ImeOpenOutcome::Applied,
        ImeOpenOutcome::AppliedWithoutSendInput,
        ImeOpenOutcome::AlreadyMatched,
        ImeOpenOutcome::Failed,
        ImeOpenOutcome::UnsafeToToggle,
    ];

    /// 全列挙して、各状態・各完了で本番 `classify_apply_completion` が旧判定(epoch 版)の oracle と一致するかを数える。
    /// 返り値は (訪れた節の数, 比べた数, 不一致の数, 最短の反例)。
    fn run(
        gens: &[u64],
        max_len: usize,
        loose: bool,
    ) -> (u64, u64, u64, Option<(Vec<Ev>, String)>) {
        let vocab: Vec<Ev> = gens
            .iter()
            .flat_map(|&g| {
                [
                    Ev::Req(g, true),
                    Ev::Req(g, false),
                    Ev::Succ(g, true),
                    Ev::Succ(g, false),
                ]
            })
            .chain([Ev::Focus, Ev::Init, Ev::Expire])
            .collect();
        let probe_gens: Vec<u64> = if loose { vec![1, 2, 3] } else { gens.to_vec() };
        let mut probes = 0u64;
        let mut mismatches = 0u64;
        let mut shortest: Option<(Vec<Ev>, String)> = None;
        let nodes = explore(
            &St::new(),
            &vocab,
            max_len,
            // `Init` は本番の前提(最初の apply 要求より前)を満たす列だけ流す。後に来る列は本番が `debug_assert` で拒む。
            &|s, e| step(s, e),
            &mut |st, trail| {
                for &pg in &probe_gens {
                    if !loose && !issued(st, pg) {
                        continue;
                    }
                    let g = gn(pg);
                    for open in [true, false] {
                        for outcome in OUTCOMES {
                            probes += 1;
                            let old = old_accept(st, &st.model, open, outcome, g);
                            let new = st.model.classify_apply_completion(open, outcome, g);
                            if new != old {
                                mismatches += 1;
                                if shortest.as_ref().is_none_or(|(t, _)| trail.len() < t.len()) {
                                    shortest = Some((
                                        trail.to_vec(),
                                        format!(
                                            "probe=(gen {pg}, {outcome:?}, open={open}) old={old:?} new={new:?}"
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            },
        );
        (nodes, probes, mismatches, shortest)
    }

    #[test]
    fn production_matches_the_epoch_oracle_gens_1_2_len_6() {
        let (nodes, probes, mismatches, shortest) = run(&[1, 2], 6, false);
        println!("nodes={nodes} probes={probes}");
        assert_eq!(
            mismatches, 0,
            "旧判定(epoch 版)と食い違う列がある: {shortest:?}"
        );
    }

    #[test]
    fn production_matches_the_epoch_oracle_gens_1_2_3_len_5() {
        let (nodes, probes, mismatches, shortest) = run(&[1, 2, 3], 5, false);
        println!("nodes={nodes} probes={probes}");
        assert_eq!(
            mismatches, 0,
            "旧判定(epoch 版)と食い違う列がある: {shortest:?}"
        );
    }

    /// 完了の generation が払い出し済みという前提を外しても(未払い出しの generation の完了も流す)、旧判定と一致する。
    #[test]
    fn production_matches_the_epoch_oracle_even_for_unissued_completions() {
        let (nodes, probes, mismatches, shortest) = run(&[1], 4, true);
        println!("nodes={nodes} probes={probes}");
        assert_eq!(
            mismatches, 0,
            "旧判定(epoch 版)と食い違う列がある: {shortest:?}"
        );
    }

    /// 本番の前提の表明: `InitialFocusScopeEstablished` が最初の `ImeApplyRequested` の後に来ると、debug ビルドでは
    /// `debug_assert` が落ちる(全列挙が見つけた唯一の反例の形。`[Req(1,_), Init]`)。
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "起動時のフォーカススコープ確立は最初の ImeApplyRequested より前")]
    fn init_after_the_first_request_is_rejected() {
        let mut st = St::new();
        st.dispatch(ImeEvent::ImeApplyRequested {
            target: true,
            generation: gn(1),
            ctrl_held: false,
        });
        st.dispatch(ImeEvent::InitialFocusScopeEstablished {
            to: HwndId(0x101),
            profile: ImePolicyProfile::ImmCross,
            focus_epoch: 1,
        });
    }
}
