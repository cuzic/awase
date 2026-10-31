//! 擬似 Zoom チャット窓（BUG-217）と、awase の送信計画。
//!
//! 症状: Zoom のチャット入力欄（会議中）は、注入キーの↓が近すぎると一部（主に母音）を TSF に渡す前に捨てる。
//! 捨てる確率は「直前のキー↓からの間隔」で決まる（[`ZoomModel`]）。awase の側は、前景窓が
//! `needs_key_hold_window` の窓なら、キーを 1 つずつ「↓ → `ZOOM_KEY_HOLD_MS` 待つ → ↑」で `key_pacer` に出す。
//!
//! この模型が**本物**を使う所:
//! - 送信の間隔を決める状態機械 `state::paced_queue::PacedQueue`（`key_pacer.rs` の殻と同じ使い方を、仮想時計と
//!   タイマーの粒度〈`SetTimer` は約 15.6ms 単位〉で動かす）。
//! - 窓の判定 `class_names::needs_key_hold_window` と待ちの長さ `tuning::ZOOM_KEY_HOLD_MS`。
//!
//! **写し**（本物ではない）: 殻（`key_pacer.rs`、`cfg(windows)`）の `run_pacing`/`advance` のループ。
//!
//! 模型の出典は [`ZoomModel::MEASURED`] の各点に書いてある。出典のない値は置かない。

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use awase_windows::focus::class_names::needs_key_hold_window;
use awase_windows::state::paced_queue::{Next, PacedQueue, Submit};
use awase_windows::tuning::ZOOM_KEY_HOLD_MS;

/// `SetTimer` の粒度（ms）。BUG-217 の本文「約 15.6ms 単位」。
pub const TIMER_GRANULARITY_MS: f64 = 15.625;

/// 直前の↓からの間隔 → 捨てる確率。
#[derive(Debug, Clone)]
pub struct ZoomModel {
    /// (間隔 ms, 捨てる確率) を間隔の昇順に並べた点。間にある間隔は線形補間、最後の点より大きい間隔は最後の点。
    pub points: &'static [(f64, f64)],
    /// 間隔と無関係に捨てる確率（背景）。
    pub background: f64,
}

impl ZoomModel {
    /// 実測の点（dragonflyg4、GJI、Zoom のメイン会議ウィンドウ内のチャット、Mozc Readable v5、2026-10-24）。
    ///
    /// - (0, 0.319) / (2, 0.3375) / (12, 0.0): `k`→`a` の 40 組（80 キー）を順に送り、TIP の `key_seq` で届いた数を
    ///   数えた。押下 0ms は 53・56、2ms は 48・58、12ms は 80・79（BUG-217 本文）。捨てた割合 = 1 − 届いた/160
    ///   （0ms: 0.319、2ms: 0.3375、12ms: 0.006）。12ms は「約 12ms 以上離れていれば落ちない」（BUG-217 の整理）に
    ///   従い 0 とする（1/160 は背景に入れる）。
    /// - (5, 0.058): **間接**。キーを順に 5ms 押しで送った文の再生は 2/8 一致。文は約 23 キーなので、
    ///   一致確率 0.25 から 1 − 0.25^(1/23) ≈ 0.058。
    pub const MEASURED: Self = Self {
        points: &[(0.0, 0.319), (2.0, 0.3375), (5.0, 0.058), (12.0, 0.0)],
        // 出荷ビルドの文の再生 71/72 一致（約 1,650 キーで 1 回、IME OFF の試行で `a` が 1 つ落ちた）→ 約 0.0006。
        background: 0.0006,
    };

    pub fn p_drop(&self, gap_ms: f64) -> f64 {
        let p = match self.points {
            [] => 0.0,
            [(_, p)] => *p,
            pts => {
                if gap_ms <= pts[0].0 {
                    pts[0].1
                } else if gap_ms >= pts[pts.len() - 1].0 {
                    pts[pts.len() - 1].1
                } else {
                    let i = pts.windows(2).position(|w| gap_ms <= w[1].0).unwrap();
                    let ((x0, y0), (x1, y1)) = (pts[i], pts[i + 1]);
                    y0 + (y1 - y0) * (gap_ms - x0) / (x1 - x0)
                }
            }
        };
        1.0 - (1.0 - p) * (1.0 - self.background)
    }
}

/// 送信計画: キーの↓を入れる時刻（ms）。
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub downs: Vec<f64>,
}

impl Plan {
    /// 各キー（2 番目以降）の、直前の↓からの間隔。
    pub fn gaps(&self) -> Vec<f64> {
        self.downs.windows(2).map(|w| w[1] - w[0]).collect()
    }

    /// 全キーが届く確率（模型の下で）。最初のキーは直前が無いので背景のみ。
    pub fn pass_probability(&self, model: &ZoomModel) -> f64 {
        let first = 1.0 - model.background;
        self.gaps()
            .iter()
            .fold(first, |acc, g| acc * (1.0 - model.p_drop(*g)))
    }
}

/// 従来: 全キーの↓を 1 回の `SendInput` にまとめて送る（間隔 0）。
pub fn plan_batch(keys: usize) -> Plan {
    Plan {
        downs: vec![0.0; keys],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Down,
    Up,
}

/// 修正後: `key_pacer` と同じ使い方で `PacedQueue` に「↓（送ったあと `hold_ms` 待つ）→ ↑」を出し、
/// タイマー（`granularity_ms` 単位に切り上げ）で進める。
pub fn plan_paced(keys: usize, hold_ms: u32, granularity_ms: f64) -> Plan {
    let mut q: PacedQueue<Item> = PacedQueue::new(1_000);
    let mut downs = Vec::with_capacity(keys);
    let now = 0_u64;
    let mut first_arm = None;
    // 全キーを同時（t=0）に提出する（`send_vk_run_batch` が run を一度に出すのと同じ）。
    for _ in 0..keys {
        for (item, delay) in [(Item::Down, hold_ms), (Item::Up, 0)] {
            match q.submit(item, delay, now) {
                Submit::SendNow { item, arm } => {
                    if item == Item::Down {
                        downs.push(0.0);
                    }
                    if arm.is_some() {
                        first_arm = arm;
                    }
                }
                Submit::Queued => {}
                Submit::Recovered { .. } => unreachable!("t=0 の提出で復帰は起きない"),
            }
        }
    }
    // タスク: 待つ → 進める、をキューが空になるまで。
    let Some(arm) = first_arm else {
        return Plan { downs };
    };
    let mut t = 0.0_f64;
    let mut wait = f64::from(arm.wait_ms);
    loop {
        t = ((t + wait) / granularity_ms).ceil() * granularity_ms;
        let mut next_wait = None;
        loop {
            match q.wait_elapsed(arm.ticket, t as u64) {
                Next::Send { item, wait_ms } => {
                    if item == Item::Down {
                        downs.push(t);
                    }
                    if wait_ms > 0 {
                        next_wait = Some(f64::from(wait_ms));
                        break;
                    }
                }
                Next::Idle | Next::Superseded => return Plan { downs },
            }
        }
        wait = next_wait.expect("待ちがあるときだけここへ来る");
    }
}

/// 前景窓のクラスに対する awase の送信計画。`fixed=false` は BUG-217 の修正前（窓の判定を無視する）。
pub fn plan_for(class: &str, keys: usize, fixed: bool) -> Plan {
    if fixed && needs_key_hold_window(class) {
        plan_paced(
            keys,
            u32::try_from(ZOOM_KEY_HOLD_MS).unwrap(),
            TIMER_GRANULARITY_MS,
        )
    } else {
        plan_batch(keys)
    }
}

/// 試行 `n` 回のうち `k` 回の全キー到達が、成功確率 `p` の二項分布で起こりうるか（両側 2.5%）。
pub fn binomial_consistent(k: u32, n: u32, p: f64) -> bool {
    let pmf = |i: u32| -> f64 {
        let mut c = 1.0_f64;
        for j in 0..i {
            c *= f64::from(n - j) / f64::from(j + 1);
        }
        c * p.powi(i32::try_from(i).unwrap()) * (1.0 - p).powi(i32::try_from(n - i).unwrap())
    };
    let lower: f64 = (0..=k).map(pmf).sum();
    let upper: f64 = (k..=n).map(pmf).sum();
    lower >= 0.025 && upper >= 0.025
}

pub const ZOOM_CHAT_POPUP: &str = "ZConfChatPopupContainerWndClass";
pub const ZOOM_CHAT_TAB: &str = "ConfMultiTabContentWndClass";
