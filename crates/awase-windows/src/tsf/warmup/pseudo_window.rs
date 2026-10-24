//! 擬似窓: 窓の種類ごとの「GJI の書き込みの出方」を模して、per-VK の確認と回収の予約を通しで動かすテスト用の窓。
//!
//! # 何を模すか
//!
//! per-VK の確認(`probe_fsm::run_per_vk_confirm`)は、1 キーを送るたびに GJI 変換サーバーの書き込みバイト
//! (`TSF_OBS.gji_write_bytes`)が閾値(`LiteralDetector::COMPOSITION_BYTES_THRESHOLD`、350B)以上増えるかで
//! 「IME に届いた」かを決める。その書き込み量は窓によって、また入力中(未確定)の文字数によって大きく変わる。
//! 擬似窓は **打鍵ごとの書き込みバイト** と **最初の書き込みが見えるまでの遅れ** を窓ごとの表で持ち、
//! 仮想時計(`clock::freeze`)の上で `TSF_OBS` に書き込みを起こす。本物の `TsfProbeCoro`・`LiteralDetector`・
//! `dispatch_probe_actions` をそのまま動かし、結果の文字列と Backspace・再送の回数を検査する。
//!
//! # 真値の出所と、仮定
//!
//! 表の値は `docs/tasks/write-profile-2026-10-24.md` の実測(Zoom は dragonflyg4 実機、メモ帳と Windows Terminal は
//! GitHub の windows-latest、いずれも GJI)。**測った入力中の文字数(0・12・30)の間は直線で結ぶ**(仮定)。
//! 測っていない窓(Chrome など)は、閾値を超える値を置いた「届く窓」として使う。
//!
//! - 「IME ON 直後の最初の 1 文字が本物のリテラルになる」のは Zoom の実測(「kいききき」)だけ。メモ帳・Windows Terminal は
//!   観測していないので `false` にしている(未観測であって、起きないという意味ではない)。
//! - 連続リテラル回数(`consecutive`)は、確認が取れたとき(`reset_consecutive_count`)だけ 0 に戻す。実機のログでは
//!   3 秒空けても戻らず 13 まで増えた。焦点移動・別の cold 理由での戻りは模さない。
//! - 文字の変換は、子音 + 母音の小さな表だけ(`か行`・`あ行`)。
//!
//! 窓の型を増やすときは [`Profile`] を足し、実測の出所をここに書く。

use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering::SeqCst;

use crate::output::probe_io::{dispatch_probe_actions, DispatchResult, ProbeIo};
use crate::output::VkMarker;
use crate::state::event_origin::Generation;
use crate::tsf::literal_facts::LiteralDetectTrace;
use crate::tsf::observer::TSF_OBS;
use crate::tsf::probe::TsfReadinessProbe;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{DeferredVk, TsfEnvSnapshot, TsfProbeCoro};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use awase::types::VkCode;

/// 暴走ガード。1 語の入力で、回収の予約 → 再送を繰り返してよい回数の上限(これを超えたら暴走とみなす)。
const MAX_ROUNDS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KeyKind {
    Consonant,
    Vowel,
}

/// 窓の種類ごとの「証拠の出方」。
#[derive(Clone, Copy, Debug)]
pub(super) struct Profile {
    pub name: &'static str,
    /// 本番のウィンドウクラス名。`class_names::is_later_vk_literal_check_unreliable_window` の入力に使う。
    pub class: &'static str,
    /// 打鍵から最初の書き込みが見えるまでの遅れ(ms)。
    first_write_ms: u64,
    /// 子音の書き込みバイト: `(入力中の文字数, バイト)` の測定点。間は直線で結ぶ。
    consonant: &'static [(usize, u64)],
    /// 母音(子音と合わせて 1 文字が完成する打鍵)の書き込みバイト。
    vowel: &'static [(usize, u64)],
    /// IME ON 直後の最初の 1 打鍵が、IME に届かず ASCII のまま出る(本物のリテラル化)。
    first_key_literal: bool,
}

impl Profile {
    /// Zoom のチャット(メイン会議ウィンドウ内)。実機 dragonflyg4、2026-10-24。
    pub(super) const ZOOM: Self = Self {
        name: "zoom",
        class: "ConfMultiTabContentWndClass",
        first_write_ms: 45,
        consonant: &[(0, 407), (12, 280)],
        vowel: &[(0, 535), (12, 324)],
        first_key_literal: true,
    };
    /// メモ帳。windows-latest、2026-10-24(run 40065722169)。
    pub(super) const NOTEPAD: Self = Self {
        name: "notepad",
        class: "Notepad",
        first_write_ms: 30,
        consonant: &[(0, 170), (12, 556), (30, 992)],
        vowel: &[(0, 268), (12, 606), (30, 1098)],
        first_key_literal: false,
    };
    /// Windows Terminal(cmd)。windows-latest、2026-10-24(run 40065722169)。
    pub(super) const WINDOWS_TERMINAL: Self = Self {
        name: "windows-terminal",
        class: "CASCADIA_HOSTING_WINDOW_CLASS",
        first_write_ms: 32,
        consonant: &[(0, 157), (12, 281), (30, 499)],
        vowel: &[(0, 111), (12, 325), (30, 599)],
        first_key_literal: false,
    };
    /// 閾値を常に超える窓(測定ではなく、対照用)。Chrome の「warm な composition は +400B」(`probe.rs` のコメント)を元にした。
    pub(super) const ALWAYS_REACHES: Self = Self {
        name: "always-reaches",
        class: "Chrome_WidgetWin_1",
        first_write_ms: 30,
        consonant: &[(0, 450)],
        vowel: &[(0, 450)],
        first_key_literal: false,
    };
    /// 子音は届くが、母音の書き込みが閾値に届かない窓(対照用。Zoom の「2 番目のキーの疑い」を単独で再現する)。
    /// クラス名は Zoom ではない(= 本番では 2 番目以降の疑いを信用する窓)。
    pub(super) const SECOND_KEY_WEAK: Self = Self {
        name: "second-key-weak",
        class: "SomeOtherWndClass",
        first_write_ms: 30,
        consonant: &[(0, 450)],
        vowel: &[(0, 150)],
        first_key_literal: false,
    };
}

fn lerp(points: &[(usize, u64)], len: usize) -> u64 {
    let Some(&(x0, y0)) = points.first() else {
        return 0;
    };
    if len <= x0 {
        return y0;
    }
    for w in points.windows(2) {
        let ((xa, ya), (xb, yb)) = (w[0], w[1]);
        if len <= xb {
            let step = u64::try_from(len - xa).unwrap_or(0);
            let span = u64::try_from(xb - xa).unwrap_or(1).max(1);
            return if yb >= ya {
                ya + (yb - ya) * step / span
            } else {
                ya - (ya - yb) * step / span
            };
        }
    }
    // 測定点より後ろは、最後の測定値のまま(仮定)。
    points.last().map_or(0, |&(_, y)| y)
}

fn kana(consonant: char, vowel: char) -> char {
    const VOWELS: [char; 5] = ['a', 'i', 'u', 'e', 'o'];
    let v = VOWELS.iter().position(|&c| c == vowel).unwrap_or(0);
    let base = match consonant {
        'k' => ['か', 'き', 'く', 'け', 'こ'],
        't' => ['た', 'ち', 'つ', 'て', 'と'],
        _ => ['あ', 'い', 'う', 'え', 'お'],
    };
    base[v]
}

/// 回収の予約(`ProbeIo::set_raw_literal` に渡される値)。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reservation {
    backs: usize,
    romaji: String,
    escape: bool,
}

/// 1 回の入力で起きたことの集計。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Stats {
    /// 回収で送った Backspace の合計。
    pub backspaces: usize,
    /// 回収の再送(ローマ字を送り直した)回数。
    pub resends: usize,
    /// 回収の ESC(入力中の文字の破棄)回数。
    pub escapes: usize,
    /// `MAX_ROUNDS` を超えて予約が続いた(暴走)か。
    pub runaway: bool,
}

pub(super) struct PseudoWindow {
    profile: Profile,
    committed: RefCell<String>,
    composing: RefCell<Vec<char>>,
    pending_consonant: Cell<Option<char>>,
    /// 次の 1 打鍵が IME に届かず ASCII のまま出る(IME ON 直後の最初の 1 文字)。
    literal_next: Cell<bool>,
    /// `(見えるようになる時刻 ms, バイト)`。
    writes: RefCell<Vec<(u64, u64)>>,
    consecutive: Cell<u32>,
    reservation: RefCell<Option<Reservation>>,
    cold_seq: Cell<u64>,
    stats: Cell<Stats>,
}

impl PseudoWindow {
    /// 擬似窓を作る。**仮想時計(`clock::freeze`)と `TSF_OBS_TEST_LOCK` を呼び出し側が持っていること。**
    pub(super) fn new(profile: Profile) -> Self {
        crate::tsf::observer::reset_literal_session_confirmed();
        TSF_OBS.gji_monitor_ok.store(true, SeqCst);
        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_write_bytes.store(1_000_000, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
        Self {
            profile,
            committed: RefCell::new(String::new()),
            composing: RefCell::new(Vec::new()),
            pending_consonant: Cell::new(None),
            literal_next: Cell::new(profile.first_key_literal),
            writes: RefCell::new(Vec::new()),
            consecutive: Cell::new(0),
            reservation: RefCell::new(None),
            cold_seq: Cell::new(0),
            stats: Cell::new(Stats::default()),
        }
    }

    /// 窓に見えている文字列(確定済み + 入力中 + 子音だけ打った状態の ASCII)。
    pub(super) fn text(&self) -> String {
        let mut s = self.committed.borrow().clone();
        s.extend(self.composing.borrow().iter());
        if let Some(c) = self.pending_consonant.get() {
            s.push(c);
        }
        s
    }

    pub(super) fn stats(&self) -> Stats {
        self.stats.get()
    }

    fn on_key(&self, ch: char) {
        let kind = if "aiueo".contains(ch) {
            KeyKind::Vowel
        } else {
            KeyKind::Consonant
        };
        if self.literal_next.replace(false) {
            // IME に届かなかった: ASCII のまま出る。GJI の書き込みは起きない。
            self.committed.borrow_mut().push(ch);
            return;
        }
        let len = self.composing.borrow().len();
        let table = match kind {
            KeyKind::Consonant => self.profile.consonant,
            KeyKind::Vowel => self.profile.vowel,
        };
        let bytes = lerp(table, len);
        self.writes.borrow_mut().push((
            crate::clock::current_tick_ms() + self.profile.first_write_ms,
            bytes,
        ));
        match kind {
            KeyKind::Vowel => {
                let c = self.pending_consonant.take();
                let k = c.map_or_else(|| kana('\0', ch), |c| kana(c, ch));
                self.composing.borrow_mut().push(k);
            }
            KeyKind::Consonant => {
                if self.pending_consonant.get() == Some(ch) {
                    self.composing.borrow_mut().push('っ');
                } else {
                    self.pending_consonant.set(Some(ch));
                }
            }
        }
    }

    /// 見えるようになった書き込みを `TSF_OBS` へ反映する(仮想時計の現在時刻まで)。
    fn apply_due_writes(&self) {
        let now = crate::clock::current_tick_ms();
        let mut writes = self.writes.borrow_mut();
        writes.retain(|&(due, bytes)| {
            if due <= now {
                TSF_OBS.gji_write_bytes.fetch_add(bytes, SeqCst);
                TSF_OBS.gji_write_ops.fetch_add(1, SeqCst);
                TSF_OBS.gji_last_write_ms.store(due, SeqCst);
                false
            } else {
                true
            }
        });
    }

    fn delete_last(&self) {
        if self.pending_consonant.take().is_some() {
            return;
        }
        if self.composing.borrow_mut().pop().is_some() {
            return;
        }
        self.committed.borrow_mut().pop();
    }

    fn bump_stats(&self, f: impl FnOnce(&mut Stats)) {
        let mut s = self.stats.get();
        f(&mut s);
        self.stats.set(s);
    }

    /// cold 状態で `romaji` を 1 語入力する(本番の `send_romaji_batched` の cold パスに相当)。
    /// 回収の予約が出たら、`Output::flush_raw_tsf_literal_recovery` と同じ順(ESC → Backspace → 再送)で適用する。
    pub(super) fn type_cold(&self, romaji: &str) {
        let mut pending = romaji.to_string();
        for round in 0.. {
            if round >= MAX_ROUNDS {
                self.bump_stats(|s| s.runaway = true);
                return;
            }
            self.run_probe(&pending);
            let Some(r) = self.reservation.borrow_mut().take() else {
                return;
            };
            if r.escape {
                self.composing.borrow_mut().clear();
                self.pending_consonant.set(None);
                self.bump_stats(|s| s.escapes += 1);
            }
            for _ in 0..r.backs {
                self.delete_last();
            }
            self.bump_stats(|s| s.backspaces += r.backs);
            if r.romaji.is_empty() {
                return;
            }
            self.bump_stats(|s| s.resends += 1);
            pending = r.romaji;
        }
    }

    fn run_probe(&self, romaji: &str) {
        let cold_seq = Generation::new(self.cold_seq.get() + 1);
        self.cold_seq.set(cold_seq.value());
        let probe = TsfReadinessProbe::new(0, cold_seq);
        let mut machine =
            TsfProbeCoro::new_chrome(romaji, cold_seq, probe, OutputActiveGuard::noop_for_test());
        let mut trace = LiteralDetectTrace::default();
        let unreliable = crate::focus::class_names::is_later_vk_literal_check_unreliable_window(
            self.profile.class,
        );
        for _ in 0..400 {
            self.apply_due_writes();
            let env = TsfEnvSnapshot {
                gji_active: true,
                later_vk_literal_check_unreliable: unreliable,
                literal_session_confirmed_gen:
                    crate::tsf::observer::literal_session_confirmed_gen_snapshot(),
                gji_candidate_visible_now: false,
                ..Default::default()
            };
            let actions = TickableFsm::tick(&mut machine, env);
            if let DispatchResult::Ended(_) =
                dispatch_probe_actions(&mut machine, actions, self, &mut trace)
            {
                return;
            }
            crate::clock::advance(10);
        }
        panic!("擬似窓: probe が 4 秒(仮想時間)たっても終わらない: {romaji:?}");
    }
}

impl ProbeIo for PseudoWindow {
    fn transmit_chrome(&self, _romaji: &str, chars: &[(VkCode, bool)]) {
        for &(vk, _) in chars {
            if let Some(c) = vk_to_char(vk) {
                self.on_key(c);
            }
        }
    }

    fn send_single_chrome_vk(&self, vk: VkCode, _needs_shift: bool) {
        if let Some(c) = vk_to_char(vk) {
            self.on_key(c);
        }
    }

    fn send_deferred_vks(&self, _vks: &[DeferredVk], _marker: VkMarker) {}

    fn consecutive_count(&self) -> u32 {
        self.consecutive.get()
    }

    fn reset_consecutive_count(&self) {
        self.consecutive.set(0);
    }

    fn set_raw_literal(&self, backs: usize, romaji: String, escape_composition: bool) {
        *self.reservation.borrow_mut() = Some(Reservation {
            backs,
            romaji,
            escape: escape_composition,
        });
    }

    fn mark_cold_raw_tsf(&self) {
        self.consecutive.set(self.consecutive.get() + 1);
    }
}

fn vk_to_char(vk: VkCode) -> Option<char> {
    let v = u8::try_from(vk.0).ok()?;
    v.is_ascii_uppercase()
        .then(|| char::from(v.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仮想時計と `TSF_OBS_TEST_LOCK` を持って擬似窓のテストを走らせる。
    fn with_window(profile: Profile, f: impl FnOnce(&PseudoWindow)) {
        let _g = crate::tsf::observer::TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _clock = crate::clock::freeze();
        let w = PseudoWindow::new(profile);
        f(&w);
    }

    #[test]
    fn lerp_interpolates_between_measured_points_and_clamps_outside() {
        let pts = [(0, 400), (10, 200)];
        assert_eq!(lerp(&pts, 0), 400);
        assert_eq!(lerp(&pts, 5), 300);
        assert_eq!(lerp(&pts, 10), 200);
        assert_eq!(lerp(&pts, 99), 200, "測定点より後ろは最後の測定値のまま");
    }

    /// 対照: 書き込みが閾値を常に超える窓では、回収は一切起きない。
    #[test]
    fn a_window_that_always_reaches_the_threshold_needs_no_recovery() {
        with_window(Profile::ALWAYS_REACHES, |w| {
            for _ in 0..4 {
                w.type_cold("ki");
            }
            assert_eq!(w.text(), "きききき");
            assert_eq!(w.stats(), Stats::default());
        });
    }

    /// BUG-215: Zoom のチャットで、IME ON 直後の最初の 1 文字が本物のリテラルになり、その後は正しく入る。
    /// 実機の入力欄は、最初の修正前は「っっっｋkkk」、per-VK を外した版は「kいききき」、現行は「きききき」だった。
    #[test]
    fn zoom_first_key_literal_is_recovered_and_the_rest_types_correctly() {
        with_window(Profile::ZOOM, |w| {
            for _ in 0..4 {
                w.type_cold("ki");
            }
            assert_eq!(w.text(), "きききき", "{:?}", w.stats());
            assert!(!w.stats().runaway, "{:?}", w.stats());
            assert!(
                w.stats().backspaces <= 1,
                "本物のリテラル(最初の 1 文字)の Backspace だけ。正しく入った文字を消さない: {:?}",
                w.stats()
            );
        });
    }

    /// BUG-215: 入力中の文字がたまると Zoom の書き込みが閾値を下回る。長く打っても文字が欠けたり余ったりしない。
    #[test]
    fn zoom_long_typing_stays_correct_even_when_writes_fall_below_the_threshold() {
        with_window(Profile::ZOOM, |w| {
            let n = 20;
            for _ in 0..n {
                w.type_cold("ki");
            }
            assert_eq!(w.text(), "き".repeat(n), "{:?}", w.stats());
            assert!(!w.stats().runaway, "{:?}", w.stats());
            let s = w.stats();
            assert!(
                s.resends <= 2 * n,
                "1 語あたり再送は 2 回まで(1 打鍵で ki が 6 回再送された暴走の再発防止): {s:?}"
            );
        });
    }

    /// BUG-216(既知の問題、現状の挙動を固定): 2 番目のキーの書き込みだけが閾値に届かない窓で、本番が
    /// 「2 番目以降の疑いを信用する」窓(Zoom 以外)だと、2 番目の疑いが ESC + 全体の再送を起こす。再送した
    /// 1 番目のキーは確認できて連続カウントが 0 に戻るため、同じことが止まらず、ESC が入力中の文字を毎回
    /// 破棄して文字が全部消える。Zoom の実機(修正前)で `ki` が 6 回再送されたのと同じ型。
    /// 直したら、`runaway` と空文字の期待を `text() == "き"` と `!runaway` に反転する。
    #[test]
    fn known_bug_216_weak_second_key_in_an_untrusted_window_loops_and_loses_text() {
        with_window(Profile::SECOND_KEY_WEAK, |w| {
            w.type_cold("ki");
            let s = w.stats();
            assert!(
                s.runaway,
                "BUG-216 が直ったなら、このテストを反転する: {s:?}"
            );
            assert!(s.escapes >= MAX_ROUNDS - 1, "{s:?}");
            assert_eq!(w.text(), "", "ESC が入力中の文字を毎回破棄する: {s:?}");
        });
    }

    /// 窓の型を表にして、同じ入力(ki を 12 回)を通す。どの窓でも、文字は崩れず、暴走せず、
    /// 回収の Backspace は打鍵数を超えない。(`SECOND_KEY_WEAK` は BUG-216 の既知の問題なので、上の専用テストで扱う。)
    #[test]
    fn every_profile_types_twelve_words_without_corruption_or_runaway() {
        for profile in [
            Profile::ZOOM,
            Profile::NOTEPAD,
            Profile::WINDOWS_TERMINAL,
            Profile::ALWAYS_REACHES,
        ] {
            with_window(profile, |w| {
                let n = 12;
                for _ in 0..n {
                    w.type_cold("ki");
                }
                let s = w.stats();
                assert_eq!(
                    w.text(),
                    "き".repeat(n),
                    "{}: 文字が崩れた {s:?}",
                    profile.name
                );
                assert!(!s.runaway, "{}: 暴走 {s:?}", profile.name);
                assert!(
                    s.backspaces <= n,
                    "{}: Backspace が多い {s:?}",
                    profile.name
                );
            });
        }
    }
}
