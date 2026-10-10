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
//! | `--dictate=paste\|unicode`          | 各試行の打鍵の前に、音声入力ソフトの挿入を模擬する(paste=クリップボード+Ctrl+V、unicode=`KEYEVENTF_UNICODE`)。期待文字列は挿入文+打鍵 |
//! | `--dictate-text=TEXT`               | 挿入する文(既定「音声入力テスト」)                                 |
//! | `--dictate-n=N`                     | 挿入を N 回続ける(既定 1)                                          |
//! | `--foreign-ctrl=s1\|s2\|s3\|s4`      | ADR-252: 他アプリの注入 Ctrl↓(Up なし)を打鍵の前に送る。s1=そのまま本試行を打つ(期限内の物理相当) / s2=VK_LCONTROL+注入の A / s3=VK_LCONTROL+TTL 超過後に注入の A / s4=VK 0x11+注入の A。最後に必ず注入 Ctrl↑ で解放 |

use serde_json::json;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

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
    dictate: Option<Dictate>,
    foreign_ctrl: Option<ForeignCtrl>,
    /// 注入 Ctrl↓ を送って Up をまだ送っていない VK(解放が要る)。
    foreign_ctrl_held: std::cell::Cell<Option<u32>>,
}

/// ADR-252 のケース。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ForeignCtrl {
    S1,
    S2,
    S3,
    S4,
}

const VK_LCONTROL_U32: u32 = 0xA2;
const VK_A_U32: u32 = 0x41;
/// `tuning.rs::FOREIGN_CTRL_TTL_MS`(1000ms)を確実に過ぎる待ち。TTL の値を決める根拠ではない。
const FOREIGN_CTRL_PAST_TTL_MS: u64 = 1500;

/// 音声入力ソフトの「挿入」の模擬。キーは `dwExtraInfo=0`(他アプリの注入、`LLKHF_INJECTED` 付き)で送る。
/// 本物の Spokenly の挿入方式は未確認のため、考えられる 2 方式を別々に試せるようにしてある。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DictateMode {
    /// クリップボードへ文を置き、Ctrl+V を送る。
    Paste,
    /// 文字ごとに `KEYEVENTF_UNICODE` の SendInput。
    Unicode,
}

struct Dictate {
    mode: DictateMode,
    text: String,
    n: usize,
}

const VK_CONTROL_U32: u32 = 0x11;
const VK_V_U32: u32 = 0x56;

/// 他アプリの注入キー(`dwExtraInfo=0`)。`typing_stress` 自身の注入(`MARKER`、物理扱い)とは別系統。
fn foreign_key(vk: u32, scan: u16, flags: u32, down: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(flags) | if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], size_of::<INPUT>() as i32);
    }
}

/// クリップボードへ文字列を置く(Win32 を直接呼ぶ。PowerShell を起動すると前面を奪い、
/// 入力先が `focus_ok=False` になって試行が無効になる)。
fn set_clipboard(text: &str) {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * size_of::<u16>();
    unsafe {
        // 他プロセスが一時的に開いていることがあるので数回やり直す。
        let mut opened = false;
        for _ in 0..20 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            sleep_ms(10);
        }
        if !opened {
            crate::log("[WARN] OpenClipboard に失敗");
            return;
        }
        let _ = EmptyClipboard();
        let h: Result<HGLOBAL, _> = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if let Ok(h) = h {
            let p = GlobalLock(h).cast::<u16>();
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(h);
                if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0))).is_err() {
                    crate::log("[WARN] SetClipboardData に失敗");
                }
            }
        }
        let _ = CloseClipboard();
    }
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
            dictate: arg_value("--dictate=").map(|v| Dictate {
                mode: match v.as_str() {
                    "paste" => DictateMode::Paste,
                    "unicode" => DictateMode::Unicode,
                    _ => {
                        crate::log(&format!("[FATAL] 引数エラー: --dictate={v}(paste|unicode)"));
                        std::process::exit(2);
                    }
                },
                text: arg_value("--dictate-text=").unwrap_or_else(|| "音声入力テスト".to_string()),
                n: num("--dictate-n=").unwrap_or(1),
            }),
            foreign_ctrl: arg_value("--foreign-ctrl=").map(|v| match v.as_str() {
                "s1" => ForeignCtrl::S1,
                "s2" => ForeignCtrl::S2,
                "s3" => ForeignCtrl::S3,
                "s4" => ForeignCtrl::S4,
                _ => {
                    crate::log(&format!("[FATAL] 引数エラー: --foreign-ctrl={v}(s1|s2|s3|s4)"));
                    std::process::exit(2);
                }
            }),
            foreign_ctrl_held: std::cell::Cell::new(None),
        }
    }

    /// 挿入で入力欄に入るはずの文(期待文字列の先頭に足す)。
    pub(crate) fn dictate_prefix(&self) -> String {
        self.dictate
            .as_ref()
            .map_or_else(String::new, |d| d.text.repeat(d.n))
    }

    /// 打鍵の前に、音声入力の挿入を模擬する。
    pub(crate) fn dictate_before_typing(&self) {
        let Some(d) = &self.dictate else { return };
        for _ in 0..d.n {
            match d.mode {
                DictateMode::Paste => {
                    set_clipboard(&d.text);
                    foreign_key(VK_CONTROL_U32, 0x1D, 0, true);
                    sleep_ms(15);
                    foreign_key(VK_V_U32, 0x2F, 0, true);
                    sleep_ms(15);
                    foreign_key(VK_V_U32, 0x2F, 0, false);
                    sleep_ms(15);
                    foreign_key(VK_CONTROL_U32, 0x1D, 0, false);
                }
                DictateMode::Unicode => {
                    for u in d.text.encode_utf16() {
                        foreign_key(0, u, KEYEVENTF_UNICODE.0, true);
                        foreign_key(0, u, KEYEVENTF_UNICODE.0, false);
                        sleep_ms(2);
                    }
                }
            }
            // 挿入が入力先へ届いてから次へ(本物のソフトも挿入の後に少し間がある想定)。
            sleep_ms(500);
        }
    }

    /// ADR-252: 注入 Ctrl↓(`dwExtraInfo=0`、Up なし)を送る。s2〜s4 はそのあと注入の `A` を送って入力欄を読み、
    /// 注入 Ctrl↑ で解放して観測レコードを返す(呼び出し側が入力欄を空にし直す)。s1 は Down だけ送り、
    /// 解放は [`Self::foreign_ctrl_release`](本試行の打鍵の後)。
    pub(crate) fn foreign_ctrl_before_typing(
        &self,
        read: &dyn Fn() -> String,
    ) -> Option<serde_json::Value> {
        let case = self.foreign_ctrl?;
        let ctrl_vk = if case == ForeignCtrl::S4 { VK_CONTROL_U32 } else { VK_LCONTROL_U32 };
        if let Ok(mut g) = crate::FOREIGN_LOG.lock() {
            g.clear();
        }
        foreign_key(ctrl_vk, 0x1D, 0, true);
        self.foreign_ctrl_held.set(Some(ctrl_vk));
        sleep_ms(50);
        if case == ForeignCtrl::S1 {
            return None;
        }
        if case == ForeignCtrl::S3 {
            sleep_ms(FOREIGN_CTRL_PAST_TTL_MS);
        }
        foreign_key(VK_A_U32, 0x1E, 0, true);
        sleep_ms(15);
        foreign_key(VK_A_U32, 0x1E, 0, false);
        sleep_ms(400);
        let text_after = read();
        self.foreign_ctrl_release();
        let reached: Vec<serde_json::Value> = crate::FOREIGN_LOG
            .lock()
            .map(|g| g.iter().map(|(vk, sc, d)| json!({"vk":format!("0x{vk:02X}"),"scan":sc,"down":d})).collect())
            .unwrap_or_default();
        Some(json!({"type":"foreign_ctrl","case":format!("{case:?}"),"sent_ctrl_vk":format!("0x{ctrl_vk:02X}"),
                    "text_after_injected_a":text_after,"hook_reached":reached}))
    }

    /// 注入 Ctrl↓ の Up を送る(残っていれば)。KeyUp 欠落そのものの再現なので、解放しないと次のケースを汚染する。
    pub(crate) fn foreign_ctrl_release(&self) {
        if let Some(vk) = self.foreign_ctrl_held.take() {
            foreign_key(vk, 0x1D, 0, false);
            sleep_ms(100);
            if self.foreign_ctrl == Some(ForeignCtrl::S1) {
                let reached: Vec<serde_json::Value> = crate::FOREIGN_LOG
                    .lock()
                    .map(|g| g.iter().map(|(vk, sc, d)| json!({"vk":format!("0x{vk:02X}"),"scan":sc,"down":d})).collect())
                    .unwrap_or_default();
                rec(&json!({"type":"foreign_ctrl","case":"S1","sent_ctrl_vk":format!("0x{vk:02X}"),"hook_reached":reached}));
            }
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
               "dictate":self.dictate.as_ref().map(|d| format!("{:?}", d.mode)),
               "dictate_text":self.dictate.as_ref().map(|d| d.text.clone()),
               "dictate_n":self.dictate.as_ref().map(|d| d.n),
               "foreign_ctrl":self.foreign_ctrl.map(|c| format!("{c:?}"))})
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
