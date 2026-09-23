//! 隠れ状態を最小のMealy機械として求める(ADR-195段階5、partition refinement)。
//!
//! 巡回学習(T1)は既に全キー×全到達可能状態を訪問しているため、L*のような能動的クエリ生成は
//! 不要。代わりに分割の反復(partition refinement)を不動点まで回す:
//!
//! 1. 初期分割として、有限個の識別プローブへの応答(`(押下後のOutcome)`)が一致する状態どうしを
//!    同一クラスにまとめる。
//! 2. 各キーで遷移した先のクラスが同じ状態どうしをさらに同一クラスにまとめる。
//! 3. 1・2を分割に変化が無くなるまで繰り返す。
//!
//! **1パスの識別だけで確定させてはいけない**——深さ2以上でしか区別できない状態(あるキーで
//! 遷移した先の状態自身が、さらに別のキーで初めて区別できる場合)を過剰併合してしまう
//! (round3の既知の誤り)。`minimize`は反復のたびに「クラス数が増えたか」を見て不動点を判定する
//! (分割は反復ごとに単調に細かくなるため、クラス数が変わらなければそれ以上分割できない)。

use std::collections::HashMap;

use crate::model::{Machine, Outcome, Status};

pub type ClassId = usize;

/// 分割結果。`class[i]` が状態 `i` の属するクラス(到達不能な状態は `None`)。
#[derive(Debug, Clone)]
pub struct Partition {
    pub class: Vec<Option<ClassId>>,
    pub num_classes: usize,
}

impl Partition {
    /// 状態 `i` のクラス(到達不能なら `None`)。
    pub fn class_of(&self, i: usize) -> Option<ClassId> {
        self.class.get(i).copied().flatten()
    }
}

/// 状態 `state` からキー `key_idx` を押したときの決定的な結果。
/// 非決定的(分岐が複数、または確率1未満)なら `None`——このアルゴリズムは、既に十分な回数
/// 観測して決定的と分かっているセルの上で最小化することを前提とする。
fn deterministic_outcome(m: &Machine, state: usize, key_idx: usize) -> Option<Outcome> {
    let branches = m.outcomes(state, key_idx);
    if branches.len() == 1 && branches[0].0 >= 1.0 - 1e-9 {
        Some(branches[0].1)
    } else {
        None
    }
}

/// 状態 `state` からキー `key_idx` を押したときの決定的な遷移先(状態の添字)。
fn deterministic_target(m: &Machine, state: usize, key_idx: usize) -> Option<usize> {
    let br = &m.states[state].trans[key_idx];
    if br.len() == 1 && br[0].p >= 1.0 - 1e-9 {
        Some(br[0].next)
    } else {
        None
    }
}

/// `probes`(識別プローブのキー添字)で初期分割し、全キーの遷移先クラスが安定するまで反復する。
///
/// `probes` は空でもよい(その場合、初期分割は `Status` のみで行われ、遷移だけで区別する)。
pub fn minimize(m: &Machine, probes: &[usize]) -> Partition {
    let reach = m.reachable();
    let n = m.states.len();

    // 段階1: 初期分割(自身のstatus + probesへの応答)。
    let mut class: Vec<Option<ClassId>> = vec![None; n];
    {
        let mut sig_to_class: HashMap<(Status, Vec<Option<Outcome>>), ClassId> = HashMap::new();
        for (i, reachable) in reach.iter().enumerate().take(n) {
            if !reachable {
                continue;
            }
            let sig: Vec<Option<Outcome>> = probes
                .iter()
                .map(|&k| deterministic_outcome(m, i, k))
                .collect();
            let key = (m.states[i].status, sig);
            let next_id = sig_to_class.len();
            let id = *sig_to_class.entry(key).or_insert(next_id);
            class[i] = Some(id);
        }
        let mut num_classes = sig_to_class.len();
        drop(sig_to_class);

        // 段階2〜3: 遷移先クラスが安定するまで反復(クラス数が増えなくなったら不動点)。
        loop {
            let mut sig_to_class: HashMap<Vec<Option<ClassId>>, ClassId> = HashMap::new();
            let mut new_class: Vec<Option<ClassId>> = vec![None; n];
            for (i, reachable) in reach.iter().enumerate().take(n) {
                if !reachable {
                    continue;
                }
                let mut sig: Vec<Option<ClassId>> = Vec::with_capacity(m.keys.len() + 1);
                sig.push(class[i]);
                for k in 0..m.keys.len() {
                    let target_class = deterministic_target(m, i, k).and_then(|t| class[t]);
                    sig.push(target_class);
                }
                let next_id = sig_to_class.len();
                let id = *sig_to_class.entry(sig).or_insert(next_id);
                new_class[i] = Some(id);
            }
            let new_num_classes = sig_to_class.len();
            class = new_class;
            if new_num_classes == num_classes {
                num_classes = new_num_classes;
                break;
            }
            num_classes = new_num_classes;
        }
        Partition { class, num_classes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Branch, Disposition, KeyId, TrueState};

    fn st(open: bool, mode: u8, composing: bool) -> Status {
        Status {
            open,
            mode,
            composing,
        }
    }

    fn det(next: usize) -> Vec<Branch> {
        vec![Branch {
            p: 1.0,
            next,
            disp: Disposition::None,
        }]
    }

    /// 2状態(state1, state2)が完全に等価(status・全キーの遷移が同一)な、3状態の機械。
    /// 最小化すると2クラス(state0 と、{state1, state2}の合併)になるべき。
    fn redundant_states_machine() -> Machine {
        Machine {
            states: vec![
                TrueState {
                    status: st(true, 0, false),
                    trans: vec![det(1), det(2)],
                },
                TrueState {
                    status: st(true, 1, false),
                    trans: vec![det(0), det(0)],
                },
                TrueState {
                    status: st(true, 1, false),
                    trans: vec![det(0), det(0)],
                },
            ],
            keys: vec![KeyId(0), KeyId(1)],
            initial: 0,
            history_suspects: vec![],
        }
    }

    #[test]
    fn merges_truly_equivalent_states_into_one_class() {
        let m = redundant_states_machine();
        let p = minimize(&m, &[0, 1]);
        assert_eq!(p.num_classes, 2);
        assert_eq!(p.class_of(1), p.class_of(2));
        assert_ne!(p.class_of(0), p.class_of(1));
    }

    /// A/Bは深さ2(2手先)でしか区別できない7状態の機械(R=根 + A,B,X,Y,P,Q)。
    ///
    /// - R(根): key2でA、key3でBへ遷移する(A・Bを同じ`initial`から両方到達可能にするための根)。
    /// - A, B: 同じstatus・同じprobe(key1)応答(自己ループ)を持つため、初期分割(段階1)だけでは
    ///   区別できない。
    /// - A --key0--> X, B --key0--> Y。X, Y も同じstatus・同じprobe応答を持ち、段階1だけでは
    ///   区別できない。
    /// - X --key0--> P, Y --key0--> Q。P, Qは互いに異なるstatusを持つため段階1で最初から別クラス。
    ///
    /// したがって「key0を2回押す」という深さ2の系列で初めてA/Bの違いが観測できる
    /// (A→X→P, B→Y→Q, Pのstatus≠Qのstatus)。1回の初期分割だけで確定させる実装だと
    /// A,BとX,Yを誤って併合したままになる(round3の既知の誤り)。反復適用なら7状態すべてが
    /// 別クラスに分かれるはず。key2/key3はA,B,X,Y,P,Qでは自己ループにして
    /// (対になる状態どうしで同じ形なので)判定に影響させない。
    fn depth2_distinguishable_machine() -> Machine {
        let s_r = st(false, 9, false);
        let s_ab = st(true, 0, false);
        let s_xy = st(true, 1, false);
        let s_p = st(true, 2, false);
        let s_q = st(true, 3, false);
        let leaf = |self_idx: usize, dive: usize| {
            vec![det(dive), det(self_idx), det(self_idx), det(self_idx)]
        };
        Machine {
            states: vec![
                // 0: R(根) -> key0/key1: 自己ループ、key2: A(1)、key3: B(2)
                TrueState {
                    status: s_r,
                    trans: vec![det(0), det(0), det(1), det(2)],
                },
                // 1: A -> key0: X(3)、key1: 自己ループ、key2/key3: 自己ループ(A,Bで対称)
                TrueState {
                    status: s_ab,
                    trans: leaf(1, 3),
                },
                // 2: B -> key0: Y(4)、key1: 自己ループ、key2/key3: 自己ループ
                TrueState {
                    status: s_ab,
                    trans: leaf(2, 4),
                },
                // 3: X -> key0: P(5)、key1: 自己ループ
                TrueState {
                    status: s_xy,
                    trans: leaf(3, 5),
                },
                // 4: Y -> key0: Q(6)、key1: 自己ループ
                TrueState {
                    status: s_xy,
                    trans: leaf(4, 6),
                },
                // 5: P -> 自己ループのみ(status s_p、Qと区別可能)
                TrueState {
                    status: s_p,
                    trans: leaf(5, 5),
                },
                // 6: Q -> 自己ループのみ(status s_q、Pと区別可能)
                TrueState {
                    status: s_q,
                    trans: leaf(6, 6),
                },
            ],
            keys: vec![KeyId(0), KeyId(1), KeyId(2), KeyId(3)],
            initial: 0,
            history_suspects: vec![],
        }
    }

    #[test]
    fn iterative_refinement_separates_states_distinguishable_only_at_depth_two() {
        let m = depth2_distinguishable_machine();
        // probes = [key1] だけ(key0は「深さ2」を作るための遷移にのみ使う、初期分割には含めない)。
        let p = minimize(&m, &[1]);

        // 全7状態(R,A,B,X,Y,P,Q)が正しく別クラスに分かれる(過剰併合が起きていない)。
        assert_eq!(p.num_classes, 7);
        let classes: Vec<ClassId> = (0..7).map(|i| p.class_of(i).unwrap()).collect();
        let mut sorted = classes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 7, "全状態が別クラスであるべき: {classes:?}");

        // 特に、A(1)とB(2)は深さ2でしか区別できないが、正しく分離されている。
        assert_ne!(p.class_of(1), p.class_of(2));
    }

    #[test]
    fn single_pass_initial_partition_alone_would_over_merge() {
        // 回帰ガード: 段階1(初期分割)だけを取り出すと、A/BとX/Yが誤って併合されたままに
        // なることを固定する(反復適用の必要性を裏付ける)。
        let m = depth2_distinguishable_machine();
        let reach = m.reachable();
        let mut sig_to_class: HashMap<(Status, Vec<Option<Outcome>>), ClassId> = HashMap::new();
        let mut class: Vec<Option<ClassId>> = vec![None; m.states.len()];
        for (i, reachable) in reach.iter().enumerate() {
            if !reachable {
                continue;
            }
            let sig: Vec<Option<Outcome>> = [1usize]
                .iter()
                .map(|&k| deterministic_outcome(&m, i, k))
                .collect();
            let key = (m.states[i].status, sig);
            let next_id = sig_to_class.len();
            let id = *sig_to_class.entry(key).or_insert(next_id);
            class[i] = Some(id);
        }
        // A(1)とB(2)は初期分割だけでは同じクラスのまま(誤併合)。
        assert_eq!(class[1], class[2]);
        assert_eq!(class[3], class[4]);
        // 反復適用する`minimize`ならこれが正しく分かれる(上のテストで確認済み)。
    }
}
