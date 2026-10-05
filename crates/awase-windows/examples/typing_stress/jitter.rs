//! 打鍵の揺らぎ(既定オフ)。固定 `--interval` の等間隔打鍵では、同時打鍵の判定窓(BUG-126/075 系)や
//! 通常打鍵の取りこぼしが出る条件に届かないため、間隔と親指キーのずれに分布を持たせる。
//!
//! フラグ(`--seed` と試行ごとの種で再現可能):
//! - `--jit-iv=SIGMA`: 1文字ごとの間隔に、対数正規の倍率 `exp(SIGMA*z - SIGMA^2/2)`(平均 1、0.3〜4 倍に丸める)を掛ける。
//! - `--jit-thumb=MIN_MS:MAX_MS`: 親指シフトの「親指↔文字の押下間隔」を MIN〜MAX の対数一様分布から引く。
//!   未指定なら従来どおり間隔の 25%。引いた間隔の分だけ、その文字の枠が伸びる(連打と重ならないようにする)。
//! - `--jit-char-first=PCT`: 親指シフトのうち、文字を先に押す(文字先押し)割合(%)。既定 0=親指先押しのみ(従来)。
//!
//! 分布の根拠(2026-10 時点): 形は**仮置き**。範囲だけ docs/adr/182 の実機ジャーナル実測に合わせる。
//! 文字→親指(文字先押し)の押下間隔は成功 2.8〜70.3ms・失敗 80.7〜92.6ms、親指先押しは 0.3〜81.7ms(一部 750ms まで)。
//! 分布そのもの(中央値・裾)の実測はリポジトリに無い。3 つとも未指定なら `nicola_events` と同一のイベント列になる。
use super::{arg_value, Cell, Ev, Face, SCAN_HENKAN, SCAN_MUHENKAN, VK_HENKAN, VK_MUHENKAN};

pub(crate) struct Jitter {
    iv_sigma: f64,
    thumb_gap_ms: Option<(f64, f64)>,
    char_first_pct: u64,
}

impl Jitter {
    pub(crate) fn from_args() -> Self {
        let iv_sigma = arg_value("--jit-iv=")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v >= 0.0)
            .unwrap_or(0.0);
        let thumb_gap_ms = arg_value("--jit-thumb=").and_then(|v| {
            let (a, b) = v.split_once(':')?;
            let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
            (a > 0.0 && b >= a).then_some((a, b))
        });
        let char_first_pct = arg_value("--jit-char-first=")
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(0, |v| v.min(100));
        Self {
            iv_sigma,
            thumb_gap_ms,
            char_first_pct,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.iv_sigma > 0.0 || self.thumb_gap_ms.is_some() || self.char_first_pct > 0
    }

    /// `config` レコードに載せる。
    pub(crate) fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.is_active(),
            "iv_sigma": self.iv_sigma,
            "thumb_gap_ms": self.thumb_gap_ms.map(|(a, b)| [a, b]),
            "char_first_pct": self.char_first_pct,
        })
    }
}

struct Xs(u64);
impl Xs {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// (0, 1) の一様乱数。
    fn unit(&mut self) -> f64 {
        ((self.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    /// 標準正規乱数(Box-Muller)。
    fn normal(&mut self) -> f64 {
        let (u1, u2) = (self.unit(), self.unit());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

/// `nicola_events` の揺らぎ版。既定値のとき(`!is_active()`)は `nicola_events` と同じ時刻になる。
pub(crate) fn events(seq: &[Cell], iv_us: u64, j: &Jitter, seed: u64) -> Vec<Ev> {
    let mut rng = Xs(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..8 {
        rng.next();
    }
    let mut evs = Vec::new();
    let mut t: u64 = 0;
    for c in seq {
        let mult = if j.iv_sigma > 0.0 {
            (j.iv_sigma * rng.normal() - j.iv_sigma * j.iv_sigma / 2.0)
                .exp()
                .clamp(0.3, 4.0)
        } else {
            1.0
        };
        let slot = (iv_us as f64 * mult) as u64;
        let mut push = |at: u64, vk: u32, scan: u16, down: bool| {
            evs.push(Ev {
                t_us: at,
                vk,
                scan,
                down,
            });
        };
        let mut extra = 0;
        match c.face {
            Face::Single => {
                push(t, c.vk, c.scan, true);
                push(t + slot / 2, c.vk, c.scan, false);
            }
            Face::Left | Face::Right => {
                let (tvk, tscan) = if c.face == Face::Left {
                    (VK_MUHENKAN, SCAN_MUHENKAN)
                } else {
                    (VK_HENKAN, SCAN_HENKAN)
                };
                let gap = match j.thumb_gap_ms {
                    Some((lo, hi)) => {
                        // 対数一様: ln(gap) を [ln lo, ln hi] の一様分布から引く。
                        let g = (lo.ln() + rng.unit() * (hi.ln() - lo.ln())).exp();
                        extra = (g * 1000.0) as u64;
                        extra
                    }
                    None => slot * 25 / 100,
                };
                let char_first = j.char_first_pct > 0 && rng.next() % 100 < j.char_first_pct;
                if char_first {
                    push(t, c.vk, c.scan, true);
                    push(t + gap, tvk, tscan, true);
                    push(t + gap + slot / 10, c.vk, c.scan, false);
                    push(t + gap + slot / 5, tvk, tscan, false);
                } else {
                    let hold = slot * 35 / 100;
                    push(t, tvk, tscan, true);
                    push(t + gap, c.vk, c.scan, true);
                    push(t + gap + hold, c.vk, c.scan, false);
                    push(t + gap + hold + slot / 10, tvk, tscan, false);
                }
            }
        }
        t += slot + extra;
    }
    evs.sort_by_key(|e| e.t_us);
    evs
}
