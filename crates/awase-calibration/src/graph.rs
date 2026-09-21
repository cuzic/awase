//! プランナ用のグラフ(事前モデル=前回の表から作る)と、有向中国人郵便配達(CPP)による巡回計画。
//!
//! - 節点は `(status, 文脈)`。文脈は「直前に押したキーが `ctx_keys` に含まれるか、どれか」(S6の部分1-switch)。`ctx_keys` が空なら文脈なし。
//! - 辺は「そのキーを押す」(`Press`)と「リセット」(`Reset`: 任意の節点→初期節点、コスト=リセット時間)。
//! - 巡回計画: 必須辺(まだ測っていないセル)に最小費用流で足りない辺を足して次数を釣り合わせ、オイラー閉路を作る。
//!   必須辺が複数の連結成分に分かれるとき(rural CPP)は、最短の往復経路で成分をつなぐヒューリスティックを使う。

use std::collections::{HashMap, VecDeque};

use crate::cost::CostModel;
use crate::model::{Machine, Outcome, Status};
use crate::rng::Rng;

/// 事前モデル(前回の表)。セル→結果。
#[derive(Debug, Clone)]
pub struct Prior {
    pub statuses: Vec<Status>,
    pub n_keys: usize,
    pub outcomes: HashMap<(usize, usize), Outcome>,
    pub initial: usize,
}

impl Prior {
    /// 真のモデルから、前回の表に当たるものを作る。到達可能な状態にわたる、確率で重み付けした多数派の結果。
    /// `err` の確率で、セルの結果を別のstatusにすり替える(前回の表の誤り)。
    pub fn from_machine(m: &Machine, err: f64, rng: &mut Rng) -> Self {
        let statuses = m.statuses();
        let reach = m.reachable();
        let n_keys = m.keys.len();
        let mut outcomes = HashMap::new();
        for (si, st) in statuses.iter().enumerate() {
            for k in 0..n_keys {
                let mut weights: Vec<(Outcome, f64)> = Vec::new();
                for (i, s) in m.states.iter().enumerate() {
                    if !reach[i] || s.status != *st {
                        continue;
                    }
                    for (p, o) in m.outcomes(i, k) {
                        if let Some(e) = weights.iter_mut().find(|(x, _)| *x == o) {
                            e.1 += p;
                        } else {
                            weights.push((o, p));
                        }
                    }
                }
                let Some((mut best, _)) =
                    weights
                        .iter()
                        .copied()
                        .reduce(|a, b| if b.1 > a.1 { b } else { a })
                else {
                    continue;
                };
                if rng.chance(err) && statuses.len() > 1 {
                    let other = statuses[rng.below(statuses.len())];
                    best = Outcome {
                        status: other,
                        disp: best.disp,
                    };
                }
                outcomes.insert((si, k), best);
            }
        }
        let initial = statuses
            .iter()
            .position(|s| *s == m.initial_status())
            .unwrap_or(0);
        Self {
            statuses,
            n_keys,
            outcomes,
            initial,
        }
    }
}

/// 辺の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Press { node: usize, key: usize },
    Reset { from: usize },
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    to: usize,
    cost: i64,
}

const INF: i64 = i64::MAX / 4;

/// プランナ用グラフ。
#[derive(Debug, Clone)]
pub struct Graph {
    pub statuses: Vec<Status>,
    pub n_keys: usize,
    pub ctx_keys: Vec<usize>,
    pub n_ctx: usize,
    pub n_nodes: usize,
    pub initial_node: usize,
    edges: Vec<Option<Edge>>,
    reset_cost: i64,
    cost_model: CostModel,
    dist: Vec<Vec<i64>>,
    first: Vec<Vec<Option<EdgeKind>>>,
}

impl Graph {
    pub fn build(prior: &Prior, ctx_keys: &[usize], cost: &CostModel) -> Self {
        let n_ctx = ctx_keys.len() + 1;
        let n_status = prior.statuses.len();
        let n_nodes = n_status * n_ctx;
        let n_keys = prior.n_keys;
        let ctx_of = |k: usize| ctx_keys.iter().position(|c| *c == k).map_or(0, |p| p + 1);
        let mut edges: Vec<Option<Edge>> = vec![None; n_nodes * n_keys];
        for s in 0..n_status {
            for c in 0..n_ctx {
                let node = s * n_ctx + c;
                for k in 0..n_keys {
                    let Some(o) = prior.outcomes.get(&(s, k)) else {
                        continue;
                    };
                    let Some(to_s) = prior.statuses.iter().position(|x| *x == o.status) else {
                        continue;
                    };
                    let changed = to_s != s || o.disp != crate::model::Disposition::None;
                    let ms = cost.expected_press_ms(changed, 60.0) + cost.read_ms;
                    edges[node * n_keys + k] = Some(Edge {
                        to: to_s * n_ctx + ctx_of(k),
                        cost: ms.round() as i64,
                    });
                }
            }
        }
        let initial_node = prior.initial * n_ctx;
        let reset_cost = (cost.reset_ms + cost.read_ms).round() as i64;
        let mut g = Self {
            statuses: prior.statuses.clone(),
            n_keys,
            ctx_keys: ctx_keys.to_vec(),
            n_ctx,
            n_nodes,
            initial_node,
            edges,
            reset_cost,
            cost_model: *cost,
            dist: Vec::new(),
            first: Vec::new(),
        };
        g.compute_paths();
        g
    }

    /// 観測した結果を辺に反映する(前回の表が誤っていたとき、以後の計画が同じ誤りを繰り返さないように)。
    /// 変わったら経路を計算し直す。観測が既知のstatusでないとき(観測誤りなど)は何もしない。
    pub fn learn_edge(&mut self, from: Status, key: usize, outcome: Outcome) {
        let (Some(si), Some(ti)) = (self.status_index(from), self.status_index(outcome.status))
        else {
            return;
        };
        let ctx_to = self
            .ctx_keys
            .iter()
            .position(|c| *c == key)
            .map_or(0, |p| p + 1);
        let want_to = ti * self.n_ctx + ctx_to;
        let mut changed = false;
        let ms = self.cost_model.expected_press_ms(
            ti != si || outcome.disp != crate::model::Disposition::None,
            60.0,
        ) + self.cost_model.read_ms;
        for c in 0..self.n_ctx {
            let node = si * self.n_ctx + c;
            let e = &mut self.edges[node * self.n_keys + key];
            if e.is_none_or(|x| x.to != want_to) {
                *e = Some(Edge {
                    to: want_to,
                    cost: ms.round() as i64,
                });
                changed = true;
            }
        }
        if changed {
            self.compute_paths();
        }
    }

    pub fn status_index(&self, s: Status) -> Option<usize> {
        self.statuses.iter().position(|x| *x == s)
    }

    /// 観測した status と直前に押したキーから、節点を決める。
    pub fn node_of(&self, s: Status, last_key: Option<usize>) -> Option<usize> {
        let si = self.status_index(s)?;
        let ctx = last_key
            .and_then(|k| self.ctx_keys.iter().position(|c| *c == k))
            .map_or(0, |p| p + 1);
        Some(si * self.n_ctx + ctx)
    }

    pub fn status_of_node(&self, node: usize) -> Status {
        self.statuses[node / self.n_ctx]
    }

    pub const fn ctx_of_node(&self, node: usize) -> usize {
        node % self.n_ctx
    }

    pub fn edge_exists(&self, node: usize, key: usize) -> bool {
        self.edges[node * self.n_keys + key].is_some()
    }

    pub fn press_to(&self, node: usize, key: usize) -> Option<usize> {
        self.edges[node * self.n_keys + key].map(|e| e.to)
    }

    pub fn kind_to(&self, k: EdgeKind) -> usize {
        match k {
            EdgeKind::Press { node, key } => {
                self.edges[node * self.n_keys + key].expect("辺が無い").to
            }
            EdgeKind::Reset { .. } => self.initial_node,
        }
    }

    pub fn kind_from(&self, k: EdgeKind) -> usize {
        match k {
            EdgeKind::Press { node, .. } => node,
            EdgeKind::Reset { from } => from,
        }
    }

    pub fn kind_cost(&self, k: EdgeKind) -> i64 {
        match k {
            EdgeKind::Press { node, key } => {
                self.edges[node * self.n_keys + key].expect("辺が無い").cost
            }
            EdgeKind::Reset { .. } => self.reset_cost,
        }
    }

    fn out_kinds(&self, node: usize) -> Vec<EdgeKind> {
        let mut v: Vec<EdgeKind> = (0..self.n_keys)
            .filter(|k| self.edges[node * self.n_keys + k].is_some())
            .map(|key| EdgeKind::Press { node, key })
            .collect();
        if node != self.initial_node {
            v.push(EdgeKind::Reset { from: node });
        }
        v
    }

    fn compute_paths(&mut self) {
        let n = self.n_nodes;
        let mut dist = vec![vec![INF; n]; n];
        let mut first: Vec<Vec<Option<EdgeKind>>> = vec![vec![None; n]; n];
        for u in 0..n {
            dist[u][u] = 0;
            for k in self.out_kinds(u) {
                let v = self.kind_to(k);
                let c = self.kind_cost(k);
                if c < dist[u][v] {
                    dist[u][v] = c;
                    first[u][v] = Some(k);
                }
            }
        }
        for m in 0..n {
            for u in 0..n {
                if dist[u][m] >= INF {
                    continue;
                }
                for v in 0..n {
                    if dist[m][v] >= INF {
                        continue;
                    }
                    let d = dist[u][m] + dist[m][v];
                    if d < dist[u][v] {
                        dist[u][v] = d;
                        first[u][v] = first[u][m];
                    }
                }
            }
        }
        self.dist = dist;
        self.first = first;
    }

    /// 初期節点から到達できるか。
    pub fn reachable_from_initial(&self, node: usize) -> bool {
        self.dist[self.initial_node][node] < INF
    }

    pub fn dist(&self, u: usize, v: usize) -> i64 {
        self.dist[u][v]
    }

    /// `u` から `v` への最短経路(辺の列)。到達できなければ空。
    pub fn path(&self, u: usize, v: usize) -> Vec<EdgeKind> {
        let mut out = Vec::new();
        let mut cur = u;
        while cur != v {
            let Some(k) = self.first[cur][v] else {
                return Vec::new();
            };
            out.push(k);
            cur = self.kind_to(k);
            if out.len() > self.n_nodes + 2 {
                return Vec::new();
            }
        }
        out
    }
}

// ---- 最小費用流 ----

struct Mcmf {
    g: Vec<Vec<usize>>,
    to: Vec<usize>,
    cap: Vec<i64>,
    cost: Vec<i64>,
}

impl Mcmf {
    fn new(n: usize) -> Self {
        Self {
            g: vec![Vec::new(); n],
            to: Vec::new(),
            cap: Vec::new(),
            cost: Vec::new(),
        }
    }

    fn add(&mut self, u: usize, v: usize, cap: i64, cost: i64) -> usize {
        let id = self.to.len();
        self.g[u].push(id);
        self.to.push(v);
        self.cap.push(cap);
        self.cost.push(cost);
        self.g[v].push(id + 1);
        self.to.push(u);
        self.cap.push(0);
        self.cost.push(-cost);
        id
    }

    /// `s` から `t` へ、流せるだけ流す(1単位ずつ最短路を増やす)。
    fn run(&mut self, s: usize, t: usize) -> i64 {
        let n = self.g.len();
        let mut flow = 0;
        loop {
            let mut dist = vec![INF; n];
            let mut in_q = vec![false; n];
            let mut prev_e = vec![usize::MAX; n];
            dist[s] = 0;
            let mut q = VecDeque::new();
            q.push_back(s);
            while let Some(u) = q.pop_front() {
                in_q[u] = false;
                for &e in &self.g[u] {
                    if self.cap[e] > 0 && dist[u] + self.cost[e] < dist[self.to[e]] {
                        dist[self.to[e]] = dist[u] + self.cost[e];
                        prev_e[self.to[e]] = e;
                        if !in_q[self.to[e]] {
                            in_q[self.to[e]] = true;
                            q.push_back(self.to[e]);
                        }
                    }
                }
            }
            if dist[t] >= INF {
                break;
            }
            // 経路上の最小容量。
            let mut f = i64::MAX;
            let mut v = t;
            while v != s {
                let e = prev_e[v];
                f = f.min(self.cap[e]);
                v = self.to[e ^ 1];
            }
            let mut v = t;
            while v != s {
                let e = prev_e[v];
                self.cap[e] -= f;
                self.cap[e ^ 1] += f;
                v = self.to[e ^ 1];
            }
            flow += f;
        }
        flow
    }
}

fn find(uf: &mut [usize], x: usize) -> usize {
    let mut r = x;
    while uf[r] != r {
        r = uf[r];
    }
    let mut c = x;
    while uf[c] != r {
        let n = uf[c];
        uf[c] = r;
        c = n;
    }
    r
}

/// 必須辺(`need[node*n_keys+key] > 0` の回数ぶん)を覆う巡回(辺の列)を、`start` から始まる形で作る。
/// 必須辺が無ければ `None`。`shuffle` なら、オイラー閉路の辺の順序を乱数で変える(経路の多様性のため)。
pub fn cpp_plan(
    g: &Graph,
    need: &[u32],
    start: usize,
    rng: &mut Rng,
    shuffle: bool,
) -> Option<Vec<EdgeKind>> {
    let n = g.n_nodes;
    let mut es: Vec<EdgeKind> = Vec::new();
    for node in 0..n {
        for key in 0..g.n_keys {
            let m = need[node * g.n_keys + key];
            if m > 0 && g.edge_exists(node, key) {
                for _ in 0..m {
                    es.push(EdgeKind::Press { node, key });
                }
            }
        }
    }
    if es.is_empty() {
        return None;
    }
    // 連結成分(弱連結)を、開始節点を含む成分に、最短の往復経路でつなぐ。
    let mut uf: Vec<usize> = (0..n).collect();
    let mut touched = vec![false; n];
    touched[start] = true;
    for k in &es {
        let (a, b) = (g.kind_from(*k), g.kind_to(*k));
        touched[a] = true;
        touched[b] = true;
        let (ra, rb) = (find(&mut uf, a), find(&mut uf, b));
        uf[ra] = rb;
    }
    loop {
        let main = find(&mut uf, start);
        let mut best: Option<(i64, usize, usize)> = None;
        for (u, &tu) in touched.iter().enumerate() {
            if !tu || find(&mut uf, u) != main {
                continue;
            }
            for (v, &tv) in touched.iter().enumerate() {
                if !tv || find(&mut uf, v) == main {
                    continue;
                }
                let d = g.dist(u, v).saturating_add(g.dist(v, u));
                if d < INF && best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, u, v));
                }
            }
        }
        let Some((_, u, v)) = best else { break };
        es.extend(g.path(u, v));
        es.extend(g.path(v, u));
        let (ru, rv) = (find(&mut uf, u), find(&mut uf, v));
        uf[ru] = rv;
    }
    // 到達できず、つながらなかった成分の必須辺は落とす。
    let main = find(&mut uf, start);
    es.retain(|k| find(&mut uf, g.kind_from(*k)) == main);
    if es.is_empty() {
        return None;
    }
    // 次数を釣り合わせる。
    let mut indeg = vec![0i64; n];
    let mut outdeg = vec![0i64; n];
    for k in &es {
        outdeg[g.kind_from(*k)] += 1;
        indeg[g.kind_to(*k)] += 1;
    }
    let (s_node, t_node) = (n, n + 1);
    let mut f = Mcmf::new(n + 2);
    for u in 0..n {
        if indeg[u] > outdeg[u] {
            f.add(s_node, u, indeg[u] - outdeg[u], 0);
        } else if outdeg[u] > indeg[u] {
            f.add(u, t_node, outdeg[u] - indeg[u], 0);
        }
    }
    let mut pair_edges: Vec<(usize, EdgeKind)> = Vec::new();
    for u in 0..n {
        // (u, v) ごとに最安の辺を1つ選ぶ。
        let mut cheapest: HashMap<usize, EdgeKind> = HashMap::new();
        for k in g.out_kinds(u) {
            let v = g.kind_to(k);
            if v == u {
                continue;
            }
            let better = cheapest
                .get(&v)
                .is_none_or(|c| g.kind_cost(k) < g.kind_cost(*c));
            if better {
                cheapest.insert(v, k);
            }
        }
        let mut vs: Vec<usize> = cheapest.keys().copied().collect();
        vs.sort_unstable();
        for v in vs {
            let k = cheapest[&v];
            let id = f.add(u, v, i64::MAX / 8, g.kind_cost(k));
            pair_edges.push((id, k));
        }
    }
    let _ = f.run(s_node, t_node);
    // 流量のある辺を足す。
    let mut extra: Vec<EdgeKind> = Vec::new();
    for (id, k) in &pair_edges {
        let used = f.cap[*id ^ 1];
        for _ in 0..used {
            extra.push(*k);
        }
    }
    es.extend(extra);
    euler(g, &es, start, rng, shuffle)
}

fn euler(
    g: &Graph,
    es: &[EdgeKind],
    start: usize,
    rng: &mut Rng,
    shuffle: bool,
) -> Option<Vec<EdgeKind>> {
    let n = g.n_nodes;
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, k) in es.iter().enumerate() {
        adj[g.kind_from(*k)].push(i);
    }
    if shuffle {
        for a in &mut adj {
            rng.shuffle(a);
        }
    }
    let mut stack: Vec<(usize, Option<usize>)> = vec![(start, None)];
    let mut circuit: Vec<usize> = Vec::new();
    while let Some(&(v, e)) = stack.last() {
        if let Some(ne) = adj[v].pop() {
            stack.push((g.kind_to(es[ne]), Some(ne)));
        } else {
            stack.pop();
            if let Some(e) = e {
                circuit.push(e);
            }
        }
    }
    circuit.reverse();
    if circuit.is_empty() {
        return None;
    }
    Some(circuit.into_iter().map(|i| es[i]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::atok_like;

    fn graph() -> (Graph, Prior) {
        let m = atok_like();
        let mut rng = Rng::new(1);
        let prior = Prior::from_machine(&m, 0.0, &mut rng);
        (Graph::build(&prior, &[], &CostModel::event()), prior)
    }

    #[test]
    fn plan_covers_every_required_edge_and_is_a_walk() {
        let (g, _) = graph();
        let mut need = vec![0u32; g.n_nodes * g.n_keys];
        for node in 0..g.n_nodes {
            for key in 0..g.n_keys {
                if g.edge_exists(node, key) {
                    need[node * g.n_keys + key] = 2;
                }
            }
        }
        let mut rng = Rng::new(2);
        let plan = cpp_plan(&g, &need, g.initial_node, &mut rng, false).expect("計画");
        // 連続した歩き(各辺の始点が直前の辺の終点)。
        let mut cur = g.initial_node;
        let mut count: HashMap<EdgeKind, u32> = HashMap::new();
        for k in &plan {
            assert_eq!(g.kind_from(*k), cur);
            cur = g.kind_to(*k);
            *count.entry(*k).or_default() += 1;
        }
        // 閉路(始点に戻る)。
        assert_eq!(cur, g.initial_node);
        for node in 0..g.n_nodes {
            for key in 0..g.n_keys {
                if g.edge_exists(node, key) {
                    assert!(
                        count
                            .get(&EdgeKind::Press { node, key })
                            .copied()
                            .unwrap_or(0)
                            >= 2
                    );
                }
            }
        }
    }

    #[test]
    fn plan_is_no_longer_than_walking_each_edge_from_reset() {
        let (g, _) = graph();
        let mut need = vec![0u32; g.n_nodes * g.n_keys];
        let mut required = 0;
        for node in 0..g.n_nodes {
            for key in 0..g.n_keys {
                if g.edge_exists(node, key) {
                    need[node * g.n_keys + key] = 1;
                    required += 1;
                }
            }
        }
        let mut rng = Rng::new(2);
        let plan = cpp_plan(&g, &need, g.initial_node, &mut rng, false).unwrap();
        let plan_cost: i64 = plan.iter().map(|k| g.kind_cost(*k)).sum();
        // 毎回リセットして経路で歩く場合(S0相当)のコストの下界: 各セルにつき「リセット+最短経路+押下」。
        let per_cell: i64 = (0..g.n_nodes)
            .flat_map(|n| (0..g.n_keys).map(move |k| (n, k)))
            .filter(|(n, k)| g.edge_exists(*n, *k))
            .map(|(n, k)| {
                g.kind_cost(EdgeKind::Reset { from: n })
                    + g.dist(g.initial_node, n)
                    + g.kind_cost(EdgeKind::Press { node: n, key: k })
            })
            .sum();
        assert!(required > 0);
        assert!(plan_cost < per_cell, "{plan_cost} < {per_cell}");
    }

    #[test]
    fn optimal_on_a_tiny_cycle() {
        // 状態2つ・キー1つで A→B→A の閉路: 必須辺2本を各1回、最小コストは辺2本の合計(余計な辺なし)。
        use crate::model::{Branch, Disposition, KeyId, Machine, TrueState};
        let st = |o| Status {
            open: o,
            mode: 0,
            composing: false,
        };
        let m = Machine {
            states: vec![
                TrueState {
                    status: st(true),
                    trans: vec![vec![Branch {
                        p: 1.0,
                        next: 1,
                        disp: Disposition::None,
                    }]],
                },
                TrueState {
                    status: st(false),
                    trans: vec![vec![Branch {
                        p: 1.0,
                        next: 0,
                        disp: Disposition::None,
                    }]],
                },
            ],
            keys: vec![KeyId(0)],
            initial: 0,
            history_suspects: vec![],
        };
        let mut rng = Rng::new(1);
        let prior = Prior::from_machine(&m, 0.0, &mut rng);
        let g = Graph::build(&prior, &[], &CostModel::event());
        let need = vec![1u32; g.n_nodes * g.n_keys];
        let plan = cpp_plan(&g, &need, g.initial_node, &mut rng, false).unwrap();
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn path_reconstructs_a_shortest_path() {
        let (g, _) = graph();
        for v in 0..g.n_nodes {
            let p = g.path(g.initial_node, v);
            let mut cur = g.initial_node;
            let mut c = 0;
            for k in &p {
                assert_eq!(g.kind_from(*k), cur);
                cur = g.kind_to(*k);
                c += g.kind_cost(*k);
            }
            assert_eq!(cur, v);
            assert_eq!(c, g.dist(g.initial_node, v));
        }
    }
}
