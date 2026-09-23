//! シミュレーション用のモデル: 合成ランダムモデルと、ATOK風モデル。
//!
//! **ATOK風モデルの仮定**(実機の全体ではなく、実測を元にした近似):
//! 観測層(開閉×変換モード0x19/0x10、モードキーの遷移)は、GJI ATOKプリセットの格子(全状態をキーだけで作った第3版)の実測に合わせた。
//! 一方、**入力中の段階(入力中/変換中〈Space〉/変換中〈変換キー〉/変換中〈無変換〉)の遷移規則は、
//! 撤去ブランチの `key_track`(打鍵履歴から追跡する規則)を元にした仮定**であり、実機で全てを確かめたものではない。
//! 非決定(入力中のBS、変換中〈Space〉のEscなど)も、実測の割れ方を元にした近似の確率を与えている。

use crate::model::{Branch, Disposition, KeyId, Machine, Status, TrueState};
use crate::rng::Rng;

/// ATOK風モデルのキー添字。
pub mod atok_keys {
    pub const MUHENKAN: usize = 0;
    pub const HENKAN: usize = 1;
    pub const HIRAGANA: usize = 2;
    pub const KATAKANA: usize = 3;
    pub const EISU: usize = 4;
    pub const HANKAKU: usize = 5;
    pub const KANJI: usize = 6;
    pub const IME_ON: usize = 7;
    pub const IME_OFF: usize = 8;
    pub const ESC: usize = 9;
    pub const ENTER: usize = 10;
    pub const SPACE: usize = 11;
    pub const BS: usize = 12;
    pub const CHAR: usize = 13;
    pub const COUNT: usize = 14;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Typing,
    ConvSpace,
    ConvHenkan,
    ConvMuhenkan,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum A {
    Idle { open: bool, m: u8 },
    Comp { stage: Stage, m: u8 },
}

const STAGES: [Stage; 4] = [
    Stage::Typing,
    Stage::ConvSpace,
    Stage::ConvHenkan,
    Stage::ConvMuhenkan,
];

fn all_states() -> Vec<A> {
    let mut v = Vec::new();
    for open in [true, false] {
        for m in 0..2u8 {
            v.push(A::Idle { open, m });
        }
    }
    for m in 0..2u8 {
        for stage in STAGES {
            v.push(A::Comp { stage, m });
        }
    }
    v
}

fn status_of(a: A) -> Status {
    match a {
        A::Idle { open, m } => Status {
            open,
            mode: m,
            composing: false,
        },
        A::Comp { m, .. } => Status {
            open: true,
            mode: m,
            composing: true,
        },
    }
}

fn br(p: f64, to: A, disp: Disposition, idx: &[A]) -> Branch {
    Branch {
        p,
        next: idx.iter().position(|x| *x == to).expect("状態が無い"),
        disp,
    }
}

fn one(to: A, disp: Disposition, idx: &[A]) -> Vec<Branch> {
    vec![br(1.0, to, disp, idx)]
}

fn atok_trans(from: A, key: usize, idx: &[A]) -> Vec<Branch> {
    use atok_keys::{
        BS, CHAR, EISU, ENTER, ESC, HANKAKU, HENKAN, HIRAGANA, IME_OFF, IME_ON, KANJI, MUHENKAN,
        SPACE,
    };
    use Disposition::{Committed, Discarded, Kept, None as N};
    let flip = |m: u8| 1 - m;
    match from {
        A::Idle { open: true, m } => {
            let on = |m| A::Idle { open: true, m };
            let off = A::Idle { open: false, m };
            match key {
                MUHENKAN | HENKAN | HANKAKU | KANJI | IME_OFF => one(off, N, idx),
                HIRAGANA | EISU => one(on(flip(m)), N, idx),
                CHAR => one(
                    A::Comp {
                        stage: Stage::Typing,
                        m,
                    },
                    N,
                    idx,
                ),
                _ => one(on(m), N, idx),
            }
        }
        A::Idle { open: false, m } => match key {
            MUHENKAN | HENKAN | KANJI | HANKAKU | IME_ON => one(A::Idle { open: true, m }, N, idx),
            _ => one(A::Idle { open: false, m }, N, idx),
        },
        A::Comp { stage, m } => {
            let idle_on = A::Idle { open: true, m };
            let closed = A::Idle { open: false, m };
            let comp = |stage, m| A::Comp { stage, m };
            match key {
                HANKAKU | KANJI | IME_OFF => one(closed, Discarded, idx),
                ENTER => one(idle_on, Committed, idx),
                HIRAGANA | EISU => one(comp(stage, flip(m)), Kept, idx),
                SPACE => one(comp(Stage::ConvSpace, m), Kept, idx),
                HENKAN => one(comp(Stage::ConvHenkan, m), Kept, idx),
                MUHENKAN if stage == Stage::ConvMuhenkan => {
                    one(comp(Stage::ConvMuhenkan, m), Kept, idx)
                }
                MUHENKAN => one(comp(Stage::ConvMuhenkan, 0), Kept, idx),
                CHAR if stage == Stage::Typing => one(comp(Stage::Typing, m), Kept, idx),
                CHAR => one(comp(Stage::Typing, m), Committed, idx),
                BS if stage == Stage::Typing => vec![
                    br(0.75, comp(Stage::Typing, m), Kept, idx),
                    br(0.25, idle_on, Discarded, idx),
                ],
                BS => one(comp(Stage::Typing, m), Kept, idx),
                ESC => match stage {
                    Stage::Typing | Stage::ConvMuhenkan => one(idle_on, Discarded, idx),
                    Stage::ConvSpace => vec![
                        br(0.75, idle_on, Discarded, idx),
                        br(0.25, comp(Stage::Typing, m), Kept, idx),
                    ],
                    Stage::ConvHenkan => one(comp(Stage::Typing, m), Kept, idx),
                },
                _ => one(comp(stage, m), Kept, idx),
            }
        }
    }
}

/// ATOK風モデル(12状態、14キー)。仮定はこのモジュールの説明を参照。
pub fn atok_like() -> Machine {
    let idx = all_states();
    let states: Vec<TrueState> = idx
        .iter()
        .map(|a| TrueState {
            status: status_of(*a),
            trans: (0..atok_keys::COUNT)
                .map(|k| atok_trans(*a, k, &idx))
                .collect(),
        })
        .collect();
    let initial = idx
        .iter()
        .position(|a| *a == A::Idle { open: true, m: 0 })
        .expect("初期状態");
    use atok_keys::{ESC, HENKAN, MUHENKAN, SPACE};
    Machine {
        states,
        keys: (0..atok_keys::COUNT as u16).map(KeyId).collect(),
        initial,
        history_suspects: vec![ESC, SPACE, HENKAN, MUHENKAN],
    }
}

/// 合成ランダムモデル。`hidden` は「入力中」のstatusごとの隠れ状態の数(1なら履歴依存なし)、
/// `nondet_frac` は非決定にするセルの割合。seed固定で再現する。
pub fn synthetic(seed: u64, n_keys: usize, hidden: usize, nondet_frac: f64) -> Machine {
    let mut rng = Rng::new(seed);
    let mut statuses: Vec<Status> = Vec::new();
    for open in [true, false] {
        for m in 0..2u8 {
            statuses.push(Status {
                open,
                mode: m,
                composing: false,
            });
        }
    }
    let idle = statuses.len();
    let mut kinds: Vec<Status> = statuses.clone();
    for m in 0..2u8 {
        for _ in 0..hidden.max(1) {
            kinds.push(Status {
                open: true,
                mode: m,
                composing: true,
            });
        }
    }
    let n = kinds.len();
    let mut states = Vec::new();
    for (i, st) in kinds.iter().enumerate() {
        let mut trans = Vec::new();
        for k in 0..n_keys {
            let pick = |rng: &mut Rng| {
                if rng.chance(0.4) {
                    i
                } else {
                    rng.below(n)
                }
            };
            let mut next = pick(&mut rng);
            // キー0は「入力を始めるキー」: 開いたidleから入力中へ。キー1は「Esc相当」: 入力中→idle(開)。
            if k == 0 && st.open && !st.composing {
                next = idle + rng.below(n - idle);
            }
            if k == 1 && st.composing {
                next = 0;
            }
            let disp_of = |rng: &mut Rng| {
                if st.composing {
                    match rng.below(3) {
                        0 => Disposition::Kept,
                        1 => Disposition::Discarded,
                        _ => Disposition::Committed,
                    }
                } else {
                    Disposition::None
                }
            };
            let d = disp_of(&mut rng);
            let branches = if rng.chance(nondet_frac) {
                let alt = pick(&mut rng);
                let alt_d = disp_of(&mut rng);
                if alt == next && alt_d == d {
                    vec![Branch {
                        p: 1.0,
                        next,
                        disp: d,
                    }]
                } else {
                    vec![
                        Branch {
                            p: 0.7,
                            next,
                            disp: d,
                        },
                        Branch {
                            p: 0.3,
                            next: alt,
                            disp: alt_d,
                        },
                    ]
                }
            } else {
                vec![Branch {
                    p: 1.0,
                    next,
                    disp: d,
                }]
            };
            trans.push(branches);
        }
        states.push(TrueState { status: *st, trans });
    }
    let suspects: Vec<usize> = (0..n_keys.min(4)).collect();
    Machine {
        states,
        keys: (0..n_keys as u16).map(KeyId).collect(),
        initial: 0,
        history_suspects: suspects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CellTruth;

    #[test]
    fn atok_like_is_fully_reachable_and_has_history_dependence() {
        let m = atok_like();
        assert!(m.reachable().iter().all(|r| *r), "全状態に到達できる");
        // 入力中のEscは、変換中(Space/変換)かどうかで結果が違う(履歴依存または非決定)。
        let comp = Status {
            open: true,
            mode: 0,
            composing: true,
        };
        assert_ne!(
            m.truth(comp, atok_keys::ESC),
            Some(CellTruth::Deterministic(
                m.outcomes(m.initial, atok_keys::ESC)[0].1
            ))
        );
    }

    #[test]
    fn atok_hiragana_toggles_the_mode_when_open() {
        let m = atok_like();
        let s0 = m.initial;
        let o = m.outcomes(s0, atok_keys::HIRAGANA)[0].1;
        assert!(o.status.open);
        assert_eq!(o.status.mode, 1);
    }

    #[test]
    fn synthetic_is_reproducible_and_reaches_states() {
        let a = synthetic(3, 13, 3, 0.1);
        let b = synthetic(3, 13, 3, 0.1);
        assert_eq!(a.states.len(), b.states.len());
        for (x, y) in a.states.iter().zip(&b.states) {
            assert_eq!(x.status, y.status);
            assert_eq!(x.trans.len(), y.trans.len());
        }
        assert!(a.reachable().iter().filter(|r| **r).count() >= 2);
    }
}
