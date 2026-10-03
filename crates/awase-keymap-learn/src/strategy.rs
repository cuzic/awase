//! 戦略 S0〜S9(文献調査の(c))の**定義の正**(READMEはここを参照する)。どれも同じ `Executor` と事前モデル(`Prior`)を使い、各セルを `k` 回測る(S7〜S9は矛盾したセルなどを増やす)。
//!
//! - S0 現状: 試行ごとにリセット→キー列で状態を作る→押す(作る間の押下は記録しない)。
//! - S1 ランダムウォーク: 確率 `restart` でリセット。
//! - S2 貪欲: 最寄りの未測定セルへ最短経路で移動。
//! - S3 有向CPP: 全セル `k` 回を覆う最小コストの巡回(計画が外れたら、残りで計画し直す)。
//! - S4 rural CPP: 1周目(k=1)の後、2周目は必須辺だけを別の順序で(経路を変えて)巡る。
//! - S5 status付きtour: S3 + 各statusを初めて見たときだけ2経路で読む(R2)。
//! - S6 部分1-switch: 直前キーが疑わしいキー集合のときだけ、文脈つきの拡大グラフで巡る。
//! - S7 適応: S5を別経路で2周し、矛盾したセルだけ `adaptive_n` 回まで増やす。
//! - S8 S6+適応: 文脈つきグラフでS6の巡回をした後、矛盾したセルだけ `adaptive_n` 回まで増やす。
//! - S9 全セルn回: 1周ごとに巡回の順序を変えて(別経路)、全セルの観測回数を1回ずつ増やし `adaptive_n` 回まで。

use crate::cost::CostModel;
use crate::exec::{Executor, ImeDriver, PressInfo, ReadPolicy};
use crate::graph::{cpp_plan, EdgeKind, Graph, Prior};
use crate::model::Status;
use crate::rng::Rng;
use crate::verify::{classify_robust, DEFAULT_MIN_MINORITY};

/// 戦略。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Strategy {
    S0,
    S1 { restart: f64 },
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
    S9,
}

impl Strategy {
    pub fn name(self) -> String {
        match self {
            Self::S0 => "S0 現状(毎回リセット)".into(),
            Self::S1 { restart } => format!("S1 ランダム(p={restart})"),
            Self::S2 => "S2 貪欲".into(),
            Self::S3 => "S3 有向CPP".into(),
            Self::S4 => "S4 rural CPP".into(),
            Self::S5 => "S5 status付きtour".into(),
            Self::S6 => "S6 部分1-switch".into(),
            Self::S7 => "S7 適応(S5+矛盾セル)".into(),
            Self::S8 => "S8 S6+適応".into(),
            Self::S9 => "S9 全セルn回(別経路)".into(),
        }
    }
}

/// 要求。
#[derive(Debug, Clone, Copy)]
pub struct Req {
    /// 各セルの目標観測回数。
    pub k: u32,
    /// 打ち切りの予算(ms)。
    pub budget_ms: f64,
    /// 押下数の上限(暴走防止)。
    pub max_presses: u32,
    /// S7が矛盾したセルに対して目指す観測回数。
    pub adaptive_n: u32,
}

impl Default for Req {
    fn default() -> Self {
        Self {
            k: 2,
            budget_ms: 60.0 * 60.0 * 1000.0,
            max_presses: 20_000,
            adaptive_n: 12,
        }
    }
}

fn over<D: ImeDriver>(exec: &Executor<D>, req: &Req) -> bool {
    // opus-adversarial-consult round2 N3対応: セッション監視が既に失敗と判定した
    // 後は、予算（時間・押下数）を使い切るまで注入し続けるのはUX・実行時間の無駄
    // なので、ここで早期終了する（安全上の問題ではない、汚染された観測は既に
    // `Executor::press`が記録を見送っている）。
    exec.elapsed_ms() > req.budget_ms
        || exec.stats.presses >= req.max_presses
        || exec.driver.should_abort()
}

/// 戦略を実行する。`suspects` は履歴依存が疑われるキーの添字(S6・S7の部分アルファベット)。
pub fn run<D: ImeDriver>(
    strategy: Strategy,
    exec: &mut Executor<D>,
    prior: &Prior,
    cost: &CostModel,
    suspects: &[usize],
    req: &Req,
    rng: &mut Rng,
) {
    match strategy {
        Strategy::S0 => {
            let g = Graph::build(prior, &[], cost);
            s0(exec, &g, req);
        }
        Strategy::S1 { restart } => {
            let g = Graph::build(prior, &[], cost);
            s1(exec, &g, req, rng, restart);
        }
        Strategy::S2 => {
            let mut g = Graph::build(prior, &[], cost);
            s2(exec, &mut g, req);
        }
        Strategy::S3 => {
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, req.k));
        }
        Strategy::S4 => {
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, 1));
            tour(exec, &mut g, req, rng, true, |e, g| need_full(e, g, req.k));
        }
        Strategy::S5 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, req.k));
        }
        Strategy::S6 => {
            let mut g = Graph::build(prior, suspects, cost);
            tour(exec, &mut g, req, rng, false, |e, g| {
                need_ctx(e, g, req.k, suspects)
            });
        }
        Strategy::S7 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, &[], cost);
            tour(exec, &mut g, req, rng, false, |e, g| need_full(e, g, 1));
            tour(exec, &mut g, req, rng, true, |e, g| need_full(e, g, req.k));
            let n = req.adaptive_n;
            tour(exec, &mut g, req, rng, true, move |e, g| {
                need_adaptive(e, g, n)
            });
        }
        Strategy::S9 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, &[], cost);
            for pass in 1..=req.adaptive_n {
                tour(exec, &mut g, req, rng, pass > 1, move |e, g| {
                    need_full(e, g, pass)
                });
            }
        }
        Strategy::S8 => {
            exec.set_read_policy(ReadPolicy::DoubleFirst);
            let mut g = Graph::build(prior, suspects, cost);
            tour(exec, &mut g, req, rng, false, |e, g| {
                need_ctx(e, g, req.k, suspects)
            });
            let n = req.adaptive_n;
            tour(exec, &mut g, req, rng, true, move |e, g| {
                need_adaptive(e, g, n)
            });
        }
    }
}

/// セル(status, キー)を満たす節点。文脈つきグラフでは、初期節点から到達できる最寄りの文脈の節点。
fn pick_node(g: &Graph, si: usize, key: usize) -> Option<usize> {
    (0..g.n_ctx)
        .map(|c| si * g.n_ctx + c)
        .filter(|n| g.edge_exists(*n, key) && g.reachable_from_initial(*n))
        .min_by_key(|n| g.dist(g.initial_node, *n))
}

fn edge_cells(g: &Graph) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    for si in 0..g.statuses.len() {
        for key in 0..g.n_keys {
            if let Some(node) = pick_node(g, si, key) {
                v.push((node, key));
            }
        }
    }
    v
}

fn need_full<D: ImeDriver>(exec: &Executor<D>, g: &Graph, k: u32) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for (node, key) in edge_cells(g) {
        let c = exec.table.count(g.status_of_node(node), key) as u32;
        need[node * g.n_keys + key] = k.saturating_sub(c);
    }
    need
}

/// やり直し用: 非決定と判定されたセルだけを、観測数が`target`に達するまで再訪する。
/// 判定は誤りに強い分類(`classify_robust`)で行い、[`Table::class`]の厳密一致による
/// `need_adaptive`(S7・S8用)とは別。既に`target`以上観測したセルは十分な証拠があると
/// みなして再訪しない。
fn need_revisit<D: ImeDriver>(exec: &Executor<D>, g: &Graph, target: u32) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for (node, key) in edge_cells(g) {
        let s = g.status_of_node(node);
        if classify_robust(&exec.table, s, key, DEFAULT_MIN_MINORITY).declared_not_det() {
            let c = exec.table.count(s, key) as u32;
            need[node * g.n_keys + key] = target.saturating_sub(c);
        }
    }
    need
}

/// 表に既にある観測を、グラフの辺と節点に反映する(やり直しで、事前モデルからグラフを
/// 作り直したときに、1回目で学んだ遷移・見つけた状態を引き継ぐため)。
fn seed_graph_from_table<D: ImeDriver>(exec: &Executor<D>, g: &mut Graph) {
    // 先に結果として現れた未知の状態を節点に加え、その後で辺を反映する
    // (加える前だと`learn_edge`が未知の遷移先を捨てるため)。
    let cells: Vec<(Status, usize)> = exec.table.cells().map(|(&(s, k), _)| (s, k)).collect();
    for &(status, key) in &cells {
        if let Some(maj) = exec.table.majority(status, key) {
            if g.status_index(maj.status).is_none()
                && exec.table.outcome_status_count(maj.status) >= DISCOVER_MIN_OBS
            {
                let _ = g.add_status(maj.status);
            }
        }
    }
    for (status, key) in cells {
        if let Some(maj) = exec.table.majority(status, key) {
            g.learn_edge(status, key, maj);
        }
    }
}

/// やり直し(ADR-195段階2): 1回目の学習の後、誤りに強い分類でも決定的と言えなかった
/// セルを、観測数が`req.adaptive_n`に達するまで再訪し、全セルも最低`req.k`回観測させる。1回目で学んだ遷移と
/// 見つけた状態は、表から引き継ぐ。
///
/// 旧実装は、非決定セルの最大観測数に2を足した`k`を全セルへ一律に課して全体を巡回し直して
/// いた。巡回の通過点として踏まれただけで観測数が数十に達するセルがあると`k`が跳ね上がり、
/// 全セルへ数十回ずつを要求して数千押下を使った(GJIのMS-IMEプリセットで、約6600押下)。
pub fn revisit_nondeterministic<D: ImeDriver>(
    exec: &mut Executor<D>,
    prior: &Prior,
    cost: &CostModel,
    suspects: &[usize],
    req: &Req,
    rng: &mut Rng,
) {
    let mut g = Graph::build(prior, suspects, cost);
    seed_graph_from_table(exec, &mut g);
    let target = req.adaptive_n;
    let k = req.k;
    // 非決定と判定されたセルの再訪(`target`まで)に加え、全セルを最低`k`回観測させる。
    // 1回目で2回しか観測されず決定的と判定されたセルの中に、隠れ状態で結果が割れるものが
    // 混じる(検証ウォークの誤答の大半がこの種のセル)ため。
    tour(exec, &mut g, req, rng, true, move |e, g| {
        let mut need = need_ctx(e, g, k, suspects);
        for (n, r) in need.iter_mut().zip(need_revisit(e, g, target)) {
            *n = (*n).max(r);
        }
        need
    });
}

fn need_adaptive<D: ImeDriver>(exec: &Executor<D>, g: &Graph, n: u32) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for (node, key) in edge_cells(g) {
        let s = g.status_of_node(node);
        if exec.table.class(s, key).declared_not_det() {
            let c = exec.table.count(s, key) as u32;
            need[node * g.n_keys + key] = n.saturating_sub(c);
        }
    }
    need
}

/// S6: 疑わしいキー(`suspects`)のセルは、文脈(直前キーが疑わしいキーのどれか/どれでもない)ごとに1回。他のセルは `k` 回
/// (文脈のうち、初期節点から到達できる最寄りの節点で満たす)。
fn need_ctx<D: ImeDriver>(exec: &Executor<D>, g: &Graph, k: u32, suspects: &[usize]) -> Vec<u32> {
    let mut need = vec![0u32; g.n_nodes * g.n_keys];
    for si in 0..g.statuses.len() {
        let s = g.statuses[si];
        for key in 0..g.n_keys {
            if suspects.contains(&key) {
                for c in 0..g.n_ctx {
                    let node = si * g.n_ctx + c;
                    if !g.edge_exists(node, key) || !g.reachable_from_initial(node) {
                        continue;
                    }
                    let have = exec
                        .table
                        .observations(s, key)
                        .iter()
                        .filter(|o| ctx_id(g, o.ctx) == c)
                        .count() as u32;
                    need[node * g.n_keys + key] = 1u32.saturating_sub(have);
                }
            } else {
                let best = pick_node(g, si, key);
                if let Some(node) = best {
                    let have = exec.table.count(s, key) as u32;
                    need[node * g.n_keys + key] = k.saturating_sub(have);
                }
            }
        }
    }
    need
}

fn ctx_id(g: &Graph, last_key: Option<usize>) -> usize {
    last_key
        .and_then(|k| g.ctx_keys.iter().position(|c| *c == k))
        .map_or(0, |p| p + 1)
}

/// 観測の多数派の結果でグラフの辺を直す。
fn learn<D: ImeDriver>(exec: &Executor<D>, g: &mut Graph, info: PressInfo, key: usize) {
    if let Some(maj) = exec.table.majority(info.before, key) {
        // 事前モデルに無い状態へ遷移したら、観測で2回確かめられた時点で節点に加える
        // (観測誤りによる1回きりの状態でグラフを膨らませない)。
        if g.status_index(maj.status).is_none()
            && exec.table.outcome_status_count(maj.status) >= DISCOVER_MIN_OBS
        {
            let _ = g.add_status(maj.status);
        }
        g.learn_edge(info.before, key, maj);
    }
}

/// 事前モデルに無いstatusを節点に加えるのに要る、結果としての観測回数。
const DISCOVER_MIN_OBS: usize = 2;

fn cur_node<D: ImeDriver>(exec: &Executor<D>, g: &Graph) -> Option<usize> {
    g.node_of(exec.current()?, exec.last_key())
}

enum Step {
    Done,
    Mismatch,
}

/// 計画を実行する。期待と違う状態に出た・キーが届かない・強制リセットが要る、のいずれかで `Mismatch`(計画し直す)。
fn execute<D: ImeDriver>(
    exec: &mut Executor<D>,
    g: &mut Graph,
    plan: &[EdgeKind],
    req: &Req,
) -> Step {
    for k in plan {
        if over(exec, req) {
            return Step::Done;
        }
        match k {
            EdgeKind::Press { .. } => {
                exec.stats.plan_presses_left = exec.stats.plan_presses_left.saturating_sub(1);
            }
            EdgeKind::Reset { .. } => {
                exec.stats.plan_resets_left = exec.stats.plan_resets_left.saturating_sub(1);
            }
        }
        if exec.should_reset() && matches!(k, EdgeKind::Press { .. }) {
            exec.note_forced_reset();
            exec.reset();
            return Step::Mismatch;
        }
        match *k {
            EdgeKind::Reset { .. } => exec.reset(),
            EdgeKind::Press { node, key } => {
                if cur_node(exec, g) != Some(node) {
                    diag_mismatch(exec, g, "pre", node, key, None);
                    exec.note_sync_loss();
                    return Step::Mismatch;
                }
                let Some(info) = exec.press(key) else {
                    return Step::Mismatch;
                };
                learn(exec, g, info, key);
                let after = g.node_of(info.outcome.status, exec.last_key());
                if after != Some(g.kind_to(*k)) {
                    diag_mismatch(exec, g, "post", node, key, Some(info.outcome.status));
                    exec.note_sync_loss();
                    return Step::Mismatch;
                }
            }
        }
    }
    Step::Done
}

/// 診断出力(`KEYMAP_LEARN_DEBUG_TOUR`が設定されているときだけ)を出すか。`s0`の
/// `KEYMAP_LEARN_DEBUG_CELLS`と同じ流儀で、OS非依存クレートなので環境変数のreadだけに留める。
fn debug_tour() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KEYMAP_LEARN_DEBUG_TOUR").is_ok())
}

/// 診断(MS-IMEプリセットの非収束の調査で使った出力): 同期喪失の内容を、最初の60件と
/// 以降500件ごとに標準エラーへ出す。`observed`が`None`なら計画の始点に居なかった(押す前の不一致)。
fn diag_mismatch<D: ImeDriver>(
    exec: &Executor<D>,
    g: &Graph,
    phase: &str,
    node: usize,
    key: usize,
    observed: Option<Status>,
) {
    if !debug_tour() {
        return;
    }
    let n = exec.stats.sync_losses;
    if n > 60 && !n.is_multiple_of(500) {
        return;
    }
    eprintln!(
        "[mismatch] n={n} phase={phase} from={:?} key={key} cur={:?} observed={observed:?}",
        g.status_of_node(node),
        exec.current()
    );
}

/// 診断: 巡回が終わったとき、満たされていない必要セルを一覧する(`why`は終わった理由)。
fn diag_unmet<D: ImeDriver>(exec: &Executor<D>, g: &Graph, need: &[u32], why: &str) {
    if !debug_tour() {
        return;
    }
    let mut cells: Vec<String> = Vec::new();
    for node in 0..g.n_nodes {
        for key in 0..g.n_keys {
            if need[node * g.n_keys + key] > 0 {
                cells.push(format!("{:?}/key{key}", g.status_of_node(node)));
            }
        }
    }
    eprintln!(
        "[unmet] why={why} presses={} n={} cells={cells:?}",
        exec.stats.presses,
        cells.len()
    );
}

fn tour<D: ImeDriver>(
    exec: &mut Executor<D>,
    g: &mut Graph,
    req: &Req,
    rng: &mut Rng,
    shuffle: bool,
    need_fn: impl Fn(&Executor<D>, &Graph) -> Vec<u32>,
) {
    if exec.current().is_none() {
        exec.reset();
    }
    for _ in 0..3000 {
        if over(exec, req) {
            let need = need_fn(exec, g);
            diag_unmet(exec, g, &need, "over");
            clear_plan_left(exec);
            return;
        }
        let need = need_fn(exec, g);
        if need.iter().all(|n| *n == 0) {
            clear_plan_left(exec);
            return;
        }
        let mut start = cur_node(exec, g).unwrap_or(g.initial_node);
        if !g.reachable_from_initial(start) {
            // 事前モデルに無い節点(履歴依存で実際にだけ現れる状態)に居る。リセットして初期から計画する。
            exec.reset();
            start = g.initial_node;
        }
        let Some(plan) = cpp_plan(g, &need, start, rng, shuffle) else {
            diag_unmet(exec, g, &need, "no_plan");
            clear_plan_left(exec);
            return;
        };
        set_plan_left(exec, &plan, &need);
        if matches!(execute(exec, g, &plan, req), Step::Done) && need_fn(exec, g) == need {
            diag_unmet(exec, g, &need, "no_progress");
            clear_plan_left(exec);
            return; // 進まなかった(必須辺に到達できない等)
        }
    }
    let need = need_fn(exec, g);
    diag_unmet(exec, g, &need, "loop_end");
    clear_plan_left(exec);
}

/// 計画を立てたとき、残りの打鍵数・リセット数・必要観測数を`Stats`へ出す(進捗の見積り用)。
fn set_plan_left<D: ImeDriver>(exec: &mut Executor<D>, plan: &[EdgeKind], need: &[u32]) {
    let presses = plan
        .iter()
        .filter(|k| matches!(k, EdgeKind::Press { .. }))
        .count();
    exec.stats.plan_presses_left = u32::try_from(presses).unwrap_or(u32::MAX);
    exec.stats.plan_resets_left = u32::try_from(plan.len() - presses).unwrap_or(u32::MAX);
    exec.stats.need_left = need.iter().sum();
}

fn clear_plan_left<D: ImeDriver>(exec: &mut Executor<D>) {
    exec.stats.plan_presses_left = 0;
    exec.stats.plan_resets_left = 0;
    exec.stats.need_left = 0;
}

fn s0<D: ImeDriver>(exec: &mut Executor<D>, g: &Graph, req: &Req) {
    // ADR195-T10究明用の一時的な診断(`KEYMAP_LEARN_DEBUG_CELLS`が設定されているときだけ、
    // 最初の数件のst!=s不一致の詳細をeprintlnする。OS非依存クレートなので環境変数read以外の
    // 依存は増やさない)。presses=0の原因切り分け(status_changesは非0なのに、
    // どのセルも目標状態と一致しない)のための計測で、S0固有の問題(無変換キー1回で
    // IMEがopenになるという想定とATOK実機挙動の不一致疑い)の特定に使った。既定では無効。
    let debug_cells = std::env::var("KEYMAP_LEARN_DEBUG_CELLS").is_ok();
    let mut debug_printed = 0u32;
    for (node, key) in edge_cells(g) {
        let s = g.status_of_node(node);
        let mut attempts = 0;
        while (exec.table.count(s, key) as u32) < req.k && attempts < req.k + 3 {
            if over(exec, req) {
                return;
            }
            attempts += 1;
            exec.reset();
            for kind in g.path(g.initial_node, node) {
                if let EdgeKind::Press { key, .. } = kind {
                    exec.press_setup(key);
                }
            }
            let st = exec.settle_setup();
            if st != s {
                if debug_cells && debug_printed < 20 {
                    debug_printed += 1;
                    let path_keys: Vec<usize> = g
                        .path(g.initial_node, node)
                        .into_iter()
                        .filter_map(|kind| match kind {
                            EdgeKind::Press { key, .. } => Some(key),
                            EdgeKind::Reset { .. } => None,
                        })
                        .collect();
                    eprintln!(
                        "[s0-debug] node={node} key={key} expected={s:?} observed={st:?} \
                         path_keys={path_keys:?} (ADR195-T10)"
                    );
                }
                exec.note_sync_loss();
                continue;
            }
            exec.set_recording(true);
            let _ = exec.press(key);
        }
    }
}

fn s1<D: ImeDriver>(exec: &mut Executor<D>, g: &Graph, req: &Req, rng: &mut Rng, restart: f64) {
    exec.reset();
    for _ in 0..req.max_presses {
        if over(exec, req) {
            return;
        }
        let done = edge_cells(g)
            .iter()
            .all(|(n, k)| exec.table.count(g.status_of_node(*n), *k) as u32 >= req.k);
        if done {
            return;
        }
        if rng.chance(restart) || exec.should_reset() {
            exec.reset();
            continue;
        }
        let key = rng.below(g.n_keys);
        let _ = exec.press(key);
    }
}

fn s2<D: ImeDriver>(exec: &mut Executor<D>, g: &mut Graph, req: &Req) {
    exec.reset();
    for _ in 0..req.max_presses {
        if over(exec, req) {
            return;
        }
        let need = need_full(exec, g, req.k);
        let Some(cur) = cur_node(exec, g) else {
            exec.reset();
            continue;
        };
        // 未測定セルを持つ最寄りの節点。
        let mut best: Option<(i64, usize)> = None;
        for node in 0..g.n_nodes {
            if (0..g.n_keys).any(|k| need[node * g.n_keys + k] > 0) {
                let d = g.dist(cur, node);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, node));
                }
            }
        }
        let Some((_, target)) = best else { return };
        for kind in g.path(cur, target) {
            match kind {
                EdgeKind::Reset { .. } => exec.reset(),
                EdgeKind::Press { key, .. } => {
                    let Some(info) = exec.press(key) else {
                        break;
                    };
                    learn(exec, g, info, key);
                }
            }
            if over(exec, req) {
                return;
            }
        }
        if cur_node(exec, g) == Some(target) {
            if let Some(key) = (0..g.n_keys).find(|k| need[target * g.n_keys + k] > 0) {
                if let Some(info) = exec.press(key) {
                    learn(exec, g, info, key);
                }
            }
        } else {
            exec.note_sync_loss();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anomaly::AnomalyPolicy;
    use crate::anomaly::ResetLevel;
    use crate::metrics::evaluate;
    use crate::model::Status;
    use crate::sample_models::atok_like;
    use crate::sim::{PressReport, SimConfig, SimIme};

    /// [ADR195-T7](../../../docs/tasks/adr195-t7-safety-measures.md)項目2
    /// （opus-adversarial-consult round2 N3対応）のテスト専用ドライバ:
    /// `SimIme`をそのまま包み、`should_abort()`だけ固定値を返す。
    struct AbortingDriver<D> {
        inner: D,
        abort: bool,
    }

    impl<D: ImeDriver> ImeDriver for AbortingDriver<D> {
        fn press(&mut self, key: usize) -> PressReport {
            self.inner.press(key)
        }
        fn press_setup(&mut self, key: usize) {
            self.inner.press_setup(key);
        }
        fn read_primary(&mut self) -> Status {
            self.inner.read_primary()
        }
        fn read_secondary(&mut self) -> Status {
            self.inner.read_secondary()
        }
        fn reread_status(&mut self) -> Status {
            self.inner.reread_status()
        }
        fn settle_setup(&mut self) -> Status {
            self.inner.settle_setup()
        }
        fn reset(&mut self, level: ResetLevel) -> bool {
            self.inner.reset(level)
        }
        fn elapsed_ms(&self) -> f64 {
            self.inner.elapsed_ms()
        }
        fn machine_initial_status(&self) -> Status {
            self.inner.machine_initial_status()
        }
        fn should_abort(&self) -> bool {
            self.abort
        }
    }

    #[test]
    fn over_respects_driver_should_abort_even_within_budget() {
        // round2 N3対応: 予算(時間・押下数)を全く使い切っていなくても、
        // ドライバが`should_abort()==true`を返したら`over()`は即座に真になり、
        // 戦略は1件も押下せず終了するはず。
        let m = atok_like();
        let mut rng = Rng::new(11);
        let prior = Prior::from_machine(&m, 0.0, &mut rng);
        let suspects = m.history_suspects.clone();
        let cost = CostModel::event();
        let sim = SimIme::new(m.clone(), SimConfig::default(), cost);
        let driver = AbortingDriver {
            inner: sim,
            abort: true,
        };
        let mut exec = Executor::new(driver, AnomalyPolicy::default(), ReadPolicy::Single);
        run(
            Strategy::S0,
            &mut exec,
            &prior,
            &cost,
            &suspects,
            &Req::default(),
            &mut rng,
        );
        assert_eq!(
            exec.stats.presses, 0,
            "should_abort()==trueなら予算に関わらず即座に終了するはず"
        );
    }

    fn run_strategy(
        s: Strategy,
        cfg: SimConfig,
        cost: CostModel,
    ) -> (Executor, crate::metrics::Metrics) {
        let m = atok_like();
        let mut rng = Rng::new(11);
        let prior = Prior::from_machine(&m, 0.0, &mut rng);
        let suspects = m.history_suspects.clone();
        let sim = SimIme::new(m.clone(), cfg, cost);
        let mut exec = Executor::new(sim, AnomalyPolicy::default(), ReadPolicy::Single);
        run(
            s,
            &mut exec,
            &prior,
            &cost,
            &suspects,
            &Req::default(),
            &mut rng,
        );
        let met = evaluate(&exec, &m);
        (exec, met)
    }

    #[test]
    fn every_strategy_covers_all_cells_without_noise() {
        let strategies = [
            Strategy::S0,
            Strategy::S1 { restart: 0.05 },
            Strategy::S2,
            Strategy::S3,
            Strategy::S4,
            Strategy::S5,
            Strategy::S6,
            Strategy::S7,
            Strategy::S8,
            Strategy::S9,
        ];
        for s in strategies {
            let (_e, m) = run_strategy(s, SimConfig::default(), CostModel::event());
            assert!(
                m.cov1 >= 0.999,
                "{}: cov1={} presses={}",
                s.name(),
                m.cov1,
                m.presses
            );
        }
    }

    /// 事前モデルに無い状態(GJI MS-IMEプリセットで、F1がカタカナ0x0Bへ遷移する等)が
    /// 実機にあっても、観測で見つけた状態を節点に加えて巡回が収束すること。
    /// 真のモデルは5モード、事前モデルは2モードだけ(3モードぶんの状態を知らない)。
    fn run_with_prior_lacking_modes(
        s: Strategy,
        prior_modes: u8,
    ) -> (Executor, crate::metrics::Metrics) {
        use crate::sample_models::atok_like_with_modes;
        let truth = atok_like_with_modes(5);
        let prior_model = atok_like_with_modes(prior_modes);
        let mut rng = Rng::new(11);
        let prior = Prior::from_machine(&prior_model, 0.0, &mut rng);
        let suspects = truth.history_suspects.clone();
        let cost = CostModel::event();
        let sim = SimIme::new(truth.clone(), SimConfig::default(), cost);
        let mut exec = Executor::new(sim, AnomalyPolicy::default(), ReadPolicy::Single);
        run(
            s,
            &mut exec,
            &prior,
            &cost,
            &suspects,
            &Req::default(),
            &mut rng,
        );
        let met = evaluate(&exec, &truth);
        (exec, met)
    }

    #[test]
    fn statuses_missing_from_the_prior_are_discovered_and_covered() {
        let (_e, known) = run_with_prior_lacking_modes(Strategy::S6, 5);
        let (_e, lacking) = run_with_prior_lacking_modes(Strategy::S6, 2);
        assert!(
            lacking.cov1 >= known.cov1 - 0.05,
            "事前モデルに無い状態を見つけられていない: 2モード事前={} 5モード事前={}",
            lacking.cov1,
            known.cov1
        );
        assert!(
            lacking.presses < 3000.0,
            "事前モデルに無い状態があっても押下数が暴走しない: {}",
            lacking.presses
        );
    }

    /// やり直しの前提となる、S6を1回走らせた後の実行器。
    fn after_first_pass() -> (Executor, Prior, Vec<usize>, CostModel) {
        let m = atok_like();
        let mut rng = Rng::new(11);
        let prior = Prior::from_machine(&m, 0.0, &mut rng);
        let suspects = m.history_suspects.clone();
        let cost = CostModel::event();
        let sim = SimIme::new(m, SimConfig::default(), cost);
        let mut exec = Executor::new(sim, AnomalyPolicy::default(), ReadPolicy::Single);
        run(
            Strategy::S6,
            &mut exec,
            &prior,
            &cost,
            &suspects,
            &Req::default(),
            &mut rng,
        );
        (exec, prior, suspects, cost)
    }

    fn flagged_cells(exec: &Executor) -> Vec<(Status, usize)> {
        exec.table
            .cells()
            .filter(|(&(s, k), _)| {
                classify_robust(&exec.table, s, k, DEFAULT_MIN_MINORITY).declared_not_det()
            })
            .map(|(&(s, k), _)| (s, k))
            .collect()
    }

    #[test]
    fn revisit_measures_only_flagged_cells_up_to_the_target() {
        let (mut exec, prior, suspects, cost) = after_first_pass();
        let flagged = flagged_cells(&exec);
        assert!(!flagged.is_empty(), "前提: 非決定と判定されるセルがある");
        let before = exec.stats.presses;
        let counts_before: Vec<usize> = flagged
            .iter()
            .map(|&(s, k)| exec.table.count(s, k))
            .collect();
        let mut rng = Rng::new(12);
        let req = Req::default();
        revisit_nondeterministic(&mut exec, &prior, &cost, &suspects, &req, &mut rng);
        let added = exec.stats.presses - before;
        for (&(s, k), &b) in flagged.iter().zip(&counts_before) {
            let now = exec.table.count(s, k);
            let still_flagged =
                classify_robust(&exec.table, s, k, DEFAULT_MIN_MINORITY).declared_not_det();
            // 目標まで測るか、測る途中で決定的と分かって再訪が要らなくなるか。
            assert!(
                now >= req.adaptive_n as usize || !still_flagged,
                "非決定セル({s:?},{k})が目標まで測られていない: {b}->{now}"
            );
            assert!(
                now > b || b >= req.adaptive_n as usize,
                "観測が増えていない"
            );
        }
        eprintln!("revisit: flagged={} added_presses={added}", flagged.len());
        assert!(
            added < 1500,
            "非決定セルだけの再訪が{added}押下かかった(flagged={})",
            flagged.len()
        );
    }

    #[test]
    fn revisit_also_raises_every_non_suspect_cell_to_the_base_k() {
        let (mut exec, prior, suspects, cost) = after_first_pass();
        let mut rng = Rng::new(12);
        let req = Req {
            k: 6,
            ..Req::default()
        };
        revisit_nondeterministic(&mut exec, &prior, &cost, &suspects, &req, &mut rng);
        for (&(s, key), obs) in exec.table.cells() {
            if suspects.contains(&key) {
                continue;
            }
            assert!(
                obs.len() >= 6,
                "全セルが最低k回観測されていない: ({s:?},{key}) = {}",
                obs.len()
            );
        }
    }

    #[test]
    fn revisit_uses_far_fewer_presses_than_the_old_uniform_k_rerun() {
        // 旧実装: 非決定セルの最大観測数+2を全セルへ一律に課してS6を再実行。
        let (mut old, prior, suspects, cost) = after_first_pass();
        let max_flagged = old
            .table
            .cells()
            .filter(|(&(s, k), _)| {
                classify_robust(&old.table, s, k, DEFAULT_MIN_MINORITY).declared_not_det()
            })
            .map(|(_, obs)| obs.len())
            .max()
            .expect("前提: 非決定セルがある");
        let old_before = old.stats.presses;
        let mut rng = Rng::new(12);
        let uniform = Req {
            k: u32::try_from(max_flagged).unwrap() + 2,
            ..Req::default()
        };
        run(
            Strategy::S6,
            &mut old,
            &prior,
            &cost,
            &suspects,
            &uniform,
            &mut rng,
        );
        let old_added = old.stats.presses - old_before;

        let (mut new, prior, suspects, cost) = after_first_pass();
        let new_before = new.stats.presses;
        let mut rng = Rng::new(12);
        revisit_nondeterministic(
            &mut new,
            &prior,
            &cost,
            &suspects,
            &Req::default(),
            &mut rng,
        );
        let new_added = new.stats.presses - new_before;
        eprintln!(
            "uniform k={} added={old_added} / targeted added={new_added}",
            uniform.k
        );
        assert!(
            new_added < old_added,
            "狙い撃ち{new_added}押下 vs 一律{old_added}押下"
        );
    }

    #[test]
    fn tour_uses_far_fewer_presses_than_the_current_method() {
        let (_, s0) = run_strategy(Strategy::S0, SimConfig::default(), CostModel::event());
        let (_, s3) = run_strategy(Strategy::S3, SimConfig::default(), CostModel::event());
        assert!(
            s3.time_ms < s0.time_ms,
            "s3={} s0={}",
            s3.time_ms,
            s0.time_ms
        );
    }

    #[test]
    fn tour_survives_anomalies_and_still_covers() {
        let cfg = SimConfig {
            key_drop_prob: 0.05,
            obs_noise: 0.02,
            drift_hazard: 0.01,
            reset_fail_prob: 0.1,
            seed: 4,
            ..SimConfig::default()
        };
        let (e, m) = run_strategy(Strategy::S7, cfg, CostModel::event());
        assert!(m.cov1 > 0.95, "cov1={}", m.cov1);
        assert!(e.stats.sync_losses > 0 || !e.stats.anomalies.is_empty());
    }
}
