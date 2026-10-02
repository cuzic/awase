//! 連続打鍵では作れない「実利用に近い状況」を試行に差し込む摂動。入力先・IME・打鍵種別とは独立で、
//! すべて既定オフ(指定しなければ従来どおり)。フラグは起動時に 1 度だけ解釈して [`Perturbation`] に持つ。
//!
//! | フラグ                              | 効果                                                              |
//! |-------------------------------------|-------------------------------------------------------------------|
//! | `--cold`                            | 準備確認(`ime_ready`)を省き、最初の本試行を窓への最初の確定入力にする |
//! | `--pause-after=N --pause-ms=MS`     | N 文字目の直後に MS だけ打鍵を止めてから再開する                   |
//! | `--idle=MS`                         | 各試行の前に MS だけ何もせず待つ                                   |
//! | `--switch-focus`                    | 各試行の前に別窓へ前面を渡し、入力先へ戻す                         |
//! | `--start-delay=MS`                  | 入力欄を空にしてから打鍵を始めるまでの待ち(既定 300)              |
//! | `--interrupt=off_on\|off\|f2\|none`  | 打鍵直後(未確定)に IME 制御キーを送る(未確定文字が消えるかの対照) |
//! | `--settle-read`                     | 内容が 800ms 変わらなくなるまで読み直す(取りこぼしと遅延の切り分け) |
//! | `--off-after=N`                     | N 文字目の直後(打鍵列の途中)に明示 IME OFF(`VK_IME_OFF`、マーカー付き=awase には物理キー)を差し込む。cold の probe 実行中の ImeOff 競合(CI 検証スパイク) |

use serde_json::json;

use crate::target::InputTarget;
use crate::{
    arg_value, front_distractor, has_flag, press, rec, sleep_ms, Ev, VK_DBE_HIRAGANA, VK_IME_OFF,
    VK_IME_ON,
};

/// 打鍵の直後(未確定)に送る IME 制御キー列。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Interrupt {
    /// `VK_IME_OFF` → `VK_IME_ON`(awase の chrome-reinit と同じ列)。
    OffOn,
    Off,
    /// `VK_DBE_HIRAGANA`(F2 相当)。
    F2,
    /// 何も送らない対照(待ち時間と `interrupt` レコードは他と揃える)。
    None,
}

impl Interrupt {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "off_on" => Some(Self::OffOn),
            "off" => Some(Self::Off),
            "f2" => Some(Self::F2),
            "none" => Some(Self::None),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::OffOn => "off_on",
            Self::Off => "off",
            Self::F2 => "f2",
            Self::None => "none",
        }
    }
}

pub(crate) struct Perturbation {
    pub(crate) cold: bool,
    pause_after: usize,
    pause_ms: u64,
    idle_ms: u64,
    switch_focus: bool,
    pub(crate) start_delay_ms: u64,
    interrupt: Option<Interrupt>,
    pub(crate) settle_read: bool,
    off_after: usize,
}

fn num<T: std::str::FromStr>(key: &str) -> Option<T> {
    arg_value(key).and_then(|v| v.parse().ok())
}

impl Perturbation {
    pub(crate) fn from_args() -> Self {
        Self {
            cold: has_flag("--cold"),
            pause_after: num("--pause-after=").unwrap_or(0),
            pause_ms: num("--pause-ms=").unwrap_or(0),
            idle_ms: num("--idle=").unwrap_or(0),
            switch_focus: has_flag("--switch-focus"),
            start_delay_ms: num("--start-delay=").unwrap_or(300),
            interrupt: arg_value("--interrupt=").map(|v| {
                Interrupt::parse(&v).unwrap_or_else(|| {
                    crate::log(&format!(
                        "[FATAL] 引数エラー: --interrupt={v}(off_on|off|f2|none)"
                    ));
                    std::process::exit(2);
                })
            }),
            settle_read: has_flag("--settle-read"),
            off_after: num("--off-after=").unwrap_or(0),
        }
    }

    /// `--switch-focus` のために、別窓をメインスレッドで作っておく必要があるか。
    pub(crate) fn needs_distractor(&self) -> bool {
        self.switch_focus
    }

    /// `config` レコードに載せる(チェッカーや後日の読み手が、どの摂動で走らせたか分かるように)。
    pub(crate) fn describe(&self) -> serde_json::Value {
        json!({"cold":self.cold,"pause_after":self.pause_after,"pause_ms":self.pause_ms,
               "idle_ms":self.idle_ms,"switch_focus":self.switch_focus,
               "start_delay_ms":self.start_delay_ms,
               "interrupt":self.interrupt.map(Interrupt::name),"settle_read":self.settle_read,
               "off_after":self.off_after})
    }

    /// 打鍵列の `pause_after` 文字目の直後に `pause_ms` の間を空ける(その後は詰めて続ける)。
    /// `nicola_events`/`raw_events` は文字 `i` の各イベントを `t_us = i*iv_us + offset`(`offset < iv_us`)
    /// で生成するため、`t_us / iv_us` から文字境界を逆算できる。
    pub(crate) fn apply_pause(&self, evs: &mut [Ev], iv_us: u64) {
        if self.pause_after == 0 || self.pause_ms == 0 || iv_us == 0 {
            return;
        }
        for e in evs.iter_mut() {
            if usize::try_from(e.t_us / iv_us).unwrap_or(usize::MAX) >= self.pause_after {
                e.t_us += self.pause_ms * 1000;
            }
        }
    }

    /// `--off-after=N`: `N` 文字目と `N+1` 文字目の境界に、明示 IME OFF(`VK_IME_OFF` の down/up)を打鍵列へ差し込む。
    /// 文字 `i` のイベントは `t_us = i*iv_us + (0〜70% of iv_us)` に収まるため、境界 `N*iv_us` の少し手前
    /// (80%/90% 位置)に置けば、直前の文字のイベントより後・次の文字より前になる。
    /// `after_inject` の `--interrupt` と違い、打鍵の最中(cold の probe 実行中)に OFF が入る。
    pub(crate) fn apply_off_after(&self, evs: &mut Vec<Ev>, iv_us: u64) {
        if self.off_after == 0 || iv_us == 0 {
            return;
        }
        let boundary = self.off_after as u64 * iv_us;
        for (back_pct, down) in [(20, true), (10, false)] {
            evs.push(Ev {
                t_us: boundary.saturating_sub(iv_us * back_pct / 100),
                vk: VK_IME_OFF,
                scan: 0x70,
                down,
            });
        }
        evs.sort_by_key(|e| e.t_us);
    }

    /// 各試行の入力欄クリアの前に呼ぶ: アイドル → 別窓へ切替 → 入力先へ復帰。
    pub(crate) fn before_trial(&self, target: &dyn InputTarget) {
        if self.idle_ms > 0 {
            sleep_ms(self.idle_ms);
        }
        if self.switch_focus && front_distractor() {
            sleep_ms(500);
            target.refocus();
        }
    }

    /// 打鍵の直後(確定前)に呼ぶ: `--interrupt` の IME 制御キー列を送る。
    pub(crate) fn after_inject(&self, kind: &str, n: usize) {
        let Some(mode) = self.interrupt else { return };
        sleep_ms(300);
        match mode {
            Interrupt::OffOn => {
                press(VK_IME_OFF, 0x70, 50);
                sleep_ms(100);
                press(VK_IME_ON, 0x70, 50);
                sleep_ms(1500);
            }
            Interrupt::Off => {
                press(VK_IME_OFF, 0x70, 50);
                sleep_ms(1000);
            }
            Interrupt::F2 => {
                press(VK_DBE_HIRAGANA, 0x70, 50);
                sleep_ms(1000);
            }
            // 何も送らない対照。他の mode がキー送信後に待つ 1000ms 相当を揃える。
            Interrupt::None => sleep_ms(1000),
        }
        rec(&json!({"type":"interrupt","mode":mode.name(),"n":n,"kind":kind}));
    }
}
