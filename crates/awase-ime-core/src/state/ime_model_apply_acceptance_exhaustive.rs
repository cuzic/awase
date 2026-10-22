//! DD2-05(`docs/tasks/mech-ecrs-2026-10-21/dd2-stale.md`): apply 完了の受理判定の全列挙。
//!
//! 問い: `completion_can_update_applied` の `pending.focus_epoch == current_epoch` を、
//! `generation >= focus_generation_watermark` に統合(`ImeTransition.focus_epoch` を撤去)して、
//! `ImeApplyAcceptance` は変わらないか。
//!
//! 方法: 実物の `ImeModel::reduce` へイベント列(Req / Succ / Focus / Init / Expire)を全列挙で流し、各状態で
//! 完了(generation × outcome × open)の受理を「旧判定(本番の写し)」と「新判定の候補」で比べる。
//! 旧判定の写しは本番の `classify_apply_completion` とも毎回突き合わせる(写しの腐敗を検出する)。
//!
//! 新判定の候補(`epoch` の照合を `pending.generation >= wm` に置き換える):
//! - A: 厳密一致の腕と Superseded の腕の**両方**で `pending.generation >= wm`。
//! - B: 厳密一致の腕だけ `pending.generation >= wm`。Superseded の腕は完了側の `generation >= wm` だけ(epoch 照合を消す)。
//!
//! `wm` は 2 通り: `Real` = 本番の `focus_generation_watermark`(`FocusChanged` でだけ進む)、
//! `Shadow` = `InitialFocusScopeEstablished` でも進めた場合(調査の例外 (i) を閉じる案)。
//!
//! 現実の前提: generation は払い出し順に単調増加し(`Req` は `last_seen` より大きい)、完了は払い出し済みの
//! generation に対してだけ届く(`probe_gen <= last_seen`、`loose` で外す)。`InitialFocusScopeEstablished` は
//! 本番では起動時の 1 度きりで最初の apply 要求より前(`init_before_first_req`、外すと「どこでも」)。

// 本番コード走査のガード(`production_code_only`)が `#[cfg(test)] mod tests` から先を落とすので、中身はここに置く。
#[cfg(test)]
mod tests {

    use super::super::*;
    use crate::state::exhaustive_seq::explore;
    use crate::state::probe_admission::FocusFence;
    use awase::platform::ImeOpenOutcome;
    use std::fmt::Write as _;
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
        /// `InitialFocusScopeEstablished` でも前進させた場合の watermark(例外 (i) を閉じる案)。
        shadow_wm: ApplyGeneration,
    }

    impl St {
        fn new() -> Self {
            Self {
                model: ImeModel::new(),
                base: Instant::now(),
                clock_ms: 0,
                seq: 0,
                epoch: 0,
                shadow_wm: ApplyGeneration::MIN,
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

        fn advance_shadow_wm(&mut self) {
            if let Some(next) = self
                .model
                .last_seen_generation
                .and_then(ApplyGeneration::checked_next)
            {
                self.shadow_wm = next;
            }
        }
    }

    fn gn(n: u64) -> ApplyGeneration {
        ApplyGeneration::new(n).expect("nonzero")
    }

    fn issued(st: &St, g: u64) -> bool {
        st.model.last_seen_generation.is_some_and(|l| gn(g) <= l)
    }

    fn step(st: &St, ev: Ev, init_before_first_req: bool) -> Option<St> {
        let mut s = st.clone();
        match ev {
            Ev::Req(g, target) => {
                // 払い出し順に単調増加する(本番の採番)。
                if issued(st, g) {
                    return None;
                }
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
                s.advance_shadow_wm();
            }
            Ev::Init => {
                // `establish_initial_fence` の前提(fence 未確立)。本番は起動時に 1 度だけ。
                if st.model.observations.current_fence() != FocusFence::default() {
                    return None;
                }
                if init_before_first_req && st.model.last_seen_generation.is_some() {
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
                s.advance_shadow_wm();
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

    /// 本番 `completion_can_update_applied` の写し(2026-10-22 の origin/develop)。
    fn old_accept(
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
        if pending.generation == generation {
            if pending.focus_epoch == current_epoch {
                ImeApplyAcceptance::Accepted
            } else {
                ImeApplyAcceptance::Stale
            }
        } else if (outcome.wrote_open_state() || outcome == ImeOpenOutcome::AlreadyMatched)
            && generation >= m.focus_generation_watermark
            && pending.focus_epoch == current_epoch
            && pending.target == open
            && m.applied.applied_open() != Some(open)
        {
            ImeApplyAcceptance::Superseded
        } else {
            ImeApplyAcceptance::Stale
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Cand {
        A,
        B,
    }

    /// 新判定の候補。`epoch` の照合を `pending.generation >= wm` に置き換える。`ImeTransition.focus_epoch` は読まない。
    fn new_accept(
        cand: Cand,
        m: &ImeModel,
        wm: ApplyGeneration,
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
        let pending_in_scope = pending.generation >= wm;
        if pending.generation == generation {
            if pending_in_scope {
                ImeApplyAcceptance::Accepted
            } else {
                ImeApplyAcceptance::Stale
            }
        } else if (outcome.wrote_open_state() || outcome == ImeOpenOutcome::AlreadyMatched)
            && generation >= wm
            && (cand == Cand::B || pending_in_scope)
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

    #[derive(Default, Clone)]
    struct Tally {
        mismatches: u64,
        /// 最短の反例(列と、そのときの完了 `(generation, outcome, open)` と旧/新の判定)。
        shortest: Option<(Vec<Ev>, String)>,
        all_contain_init: bool,
    }

    impl Tally {
        fn new() -> Self {
            Self {
                all_contain_init: true,
                ..Self::default()
            }
        }

        fn record(&mut self, trail: &[Ev], detail: String) {
            self.mismatches += 1;
            if !trail.contains(&Ev::Init) {
                self.all_contain_init = false;
            }
            let shorter = match &self.shortest {
                Some((t, _)) => trail.len() < t.len(),
                None => true,
            };
            if shorter {
                self.shortest = Some((trail.to_vec(), detail));
            }
        }

        fn shortest_len(&self) -> Option<usize> {
            self.shortest.as_ref().map(|(t, _)| t.len())
        }
    }

    struct Report {
        nodes: u64,
        probes: u64,
        /// 順に A/Real, B/Real, A/Shadow, B/Shadow。
        tallies: [Tally; 4],
    }

    fn run(gens: &[u64], max_len: usize, loose: bool, init_before_first_req: bool) -> Report {
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
        let mut tallies = [Tally::new(), Tally::new(), Tally::new(), Tally::new()];
        let mut probes = 0u64;
        let nodes = explore(
            &St::new(),
            &vocab,
            max_len,
            &|s, e| step(s, e, init_before_first_req),
            &mut |st, trail| {
                for &pg in &probe_gens {
                    if !loose && !issued(st, pg) {
                        continue;
                    }
                    let g = gn(pg);
                    for open in [true, false] {
                        for outcome in OUTCOMES {
                            probes += 1;
                            let old = old_accept(&st.model, open, outcome, g);
                            // 写しの腐敗検出: 本番と一致していること。
                            assert_eq!(
                            old,
                            st.model.classify_apply_completion(open, outcome, g),
                            "旧判定の写しが本番とずれた: {trail:?} probe=(gen {pg}, {outcome:?}, open={open})"
                        );
                            let wms = [st.model.focus_generation_watermark, st.shadow_wm];
                            let cases = [(Cand::A, 0), (Cand::B, 0), (Cand::A, 1), (Cand::B, 1)];
                            for (i, (cand, w)) in cases.into_iter().enumerate() {
                                let new = new_accept(cand, &st.model, wms[w], open, outcome, g);
                                if new != old {
                                    tallies[i].record(
                                    trail,
                                    format!(
                                        "probe=(gen {pg}, {outcome:?}, open={open}) old={old:?} new={new:?}"
                                    ),
                                );
                                }
                            }
                        }
                    }
                }
            },
        );
        Report {
            nodes,
            probes,
            tallies,
        }
    }

    fn describe(r: &Report) -> String {
        let names = ["A/Real", "B/Real", "A/Shadow", "B/Shadow"];
        let mut out = format!("nodes={} probes={}\n", r.nodes, r.probes);
        for (n, t) in names.iter().zip(&r.tallies) {
            let _ = writeln!(
                out,
                "  {n}: mismatches={} all_contain_init={} shortest={:?}",
                t.mismatches, t.all_contain_init, t.shortest
            );
        }
        out
    }

    /// 本番の前提(`InitialFocusScopeEstablished` は最初の apply 要求より前): epoch の照合を `pending.generation >= wm` に
    /// 置き換えても(A も B も、Real の watermark のままで)、全列・全完了で受理は旧判定と一致する。
    fn assert_equivalent_under_production_precondition(r: &Report) {
        let msg = describe(r);
        for (i, t) in r.tallies.iter().enumerate() {
            assert_eq!(t.mismatches, 0, "候補 {i} に反例がある\n{msg}");
        }
    }

    #[test]
    fn epoch_is_redundant_given_init_precedes_first_request_gens_1_2_len_6() {
        let r = run(&[1, 2], 6, false, true);
        println!("{}", describe(&r));
        assert_equivalent_under_production_precondition(&r);
    }

    #[test]
    fn epoch_is_redundant_given_init_precedes_first_request_gens_1_2_3_len_5() {
        let r = run(&[1, 2, 3], 5, false, true);
        println!("{}", describe(&r));
        assert_equivalent_under_production_precondition(&r);
    }

    /// 例外 (i): `InitialFocusScopeEstablished` が `ImeApplyRequested` の後に来ると、fence の epoch だけが進み watermark は
    /// 進まない。epoch を撤去するとその pending の完了が受理される(最短の反例は `[Req(1,_), Init]` の 2 イベント)。
    /// 反例はすべて `Init` を含む。`Init` でも watermark を進める案(Shadow)は、`Init` の前に払い出した generation の
    /// Superseded を旧判定より多く落とすので、これも一致しない(`[Req(1,_), Init, Req(2,_)]`)。
    #[test]
    fn counterexamples_exist_only_when_init_follows_a_request() {
        let r = run(&[1, 2], 6, false, false);
        let msg = describe(&r);
        println!("{msg}");
        for i in [0, 1] {
            let t = &r.tallies[i];
            assert!(t.mismatches > 0, "Real の候補に反例が無い\n{msg}");
            assert_eq!(t.shortest_len(), Some(2), "{msg}");
        }
        for t in &r.tallies {
            assert!(t.all_contain_init, "Init を含まない反例がある\n{msg}");
        }
        for i in [2, 3] {
            assert!(
                r.tallies[i].mismatches > 0,
                "Shadow の候補に反例が無い\n{msg}"
            );
        }
    }

    /// 完了の generation が払い出し済みであるという前提を外すと、B(Superseded の腕の epoch 照合を消す案)は旧判定とずれる
    /// (`Req(1)`, `Focus`, 完了 gen 2)。A(`pending.generation >= wm` を両方の腕に残す案)は前提なしでも一致する。
    #[test]
    fn candidate_b_needs_completions_to_be_for_issued_generations() {
        let r = run(&[1], 4, true, true);
        let msg = describe(&r);
        println!("{msg}");
        assert_eq!(r.tallies[0].mismatches, 0, "A/Real\n{msg}");
        assert!(r.tallies[1].mismatches > 0, "B/Real に反例が無い\n{msg}");
    }
}
