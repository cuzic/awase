//! ADR-261 段階 1: 入力インジケーターの字形の**影の取り込み**（挙動を変えない観測）。
//!
//! 前面窓が変わったとき（`observer::indicator_glyph` の `EVENT_SYSTEM_FOREGROUND` の WinEvent が検知）、字形が実状態に追随する
//! のを待ってから（0.9 秒後と 2.5 秒後の 2 回。字形の遅れは中央 0.2 秒・最大 0.68 秒、ADR-261）インジケーターの矩形を取り込み、
//! 前処理まで進めた結果（大きさ・前景の点数・所要・失敗の理由）を journal（`JournalEntry::IndicatorGlyph`）に残す。
//! **belief には入れない・IME へ何も書かない・打鍵は変えない・画素は保存しない**。目的は、実機で「取れる環境・取れない環境」と
//! 失敗の理由の分布を実ジャーナルで確かめること（見本の表への照合と一致率は次の段階）。
//!
//! 有効なのは `[indicator_evidence] glyph` の実効の段階が `observe` 以上（既定は off のオプトイン。ADR-271）のときだけ。反映は再起動後
//! （フックと読み取りスレッドは起動時に 1 度だけ作る。`app/bootstrap.rs`）。

use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::time::Instant;

use windows::Win32::Foundation::HWND;

use crate::observer::indicator_glyph::GlyphReadRequest;
use crate::state::indicator_glyph::ProbeOutcome;

/// 前面窓の変化から取り込むまでの待ち（ミリ秒）。字形の遅れ（最大 0.68 秒、ADR-261）より長い 2 点。
const READ_DELAYS_MS: [u16; 2] = [900, 2500];
/// 前面窓の変化の受け付け間隔の下限（Alt+Tab の連打で依頼が溜まらないように）。
const MIN_TRIGGER_INTERVAL_MS: u64 = 3000;
/// 結果待ちの依頼の上限（これを超えて溜まっていたら新しい依頼は出さない）。
const MAX_PENDING: usize = 4;

/// 結果待ちの依頼に付けておく文脈（結果が返ったら journal の記録に添える）。
struct Pending {
    /// 依頼したときの前面窓（結果が返ったときに前面窓が変わっていれば古いので捨てる）
    foreground: isize,
    delay_ms: u32,
}

/// 影の取り込みの状態。読み取りワーカーへの送信口が無ければ（無効のとき）何もしない。
pub(super) struct IndicatorGlyphShadow {
    tx: Option<Sender<GlyphReadRequest>>,
    pending: VecDeque<Pending>,
    last_trigger: Option<Instant>,
    /// 取り込みの件数（不具合報告の要約用。画像・画素は持たない。ADR-271 D5）
    counters: crate::indicator_evidence::IndicatorGlyphCounters,
}

impl IndicatorGlyphShadow {
    pub(super) const fn disabled() -> Self {
        Self {
            tx: None,
            pending: VecDeque::new(),
            last_trigger: None,
            counters: crate::indicator_evidence::IndicatorGlyphCounters::new(),
        }
    }
}

/// 前面窓の変化のうち、取り込みの対象にしない窓のクラス（タスクバー自身・メニュー。インジケーターを操作している最中）。
fn is_excluded_foreground_class(class: &str) -> bool {
    matches!(
        class,
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "TopLevelWindowForOverflowXamlIsland"
    ) || crate::focus::class_names::is_popup_menu_window_class(class)
}

impl super::Runtime {
    /// 起動時に、読み取りワーカーへの送信口を渡して有効にする（`None` なら無効のまま）。
    pub(crate) fn init_indicator_glyph(&mut self, tx: Option<Sender<GlyphReadRequest>>) {
        self.indicator_glyph = IndicatorGlyphShadow {
            tx,
            pending: VecDeque::new(),
            last_trigger: None,
            counters: crate::indicator_evidence::IndicatorGlyphCounters::new(),
        };
    }

    /// 不具合報告の要約用に、字形の取り込みの件数を返す（読み取りだけ）。
    pub(crate) const fn indicator_glyph_counters(
        &self,
    ) -> &crate::indicator_evidence::IndicatorGlyphCounters {
        &self.indicator_glyph.counters
    }

    /// `EVENT_SYSTEM_FOREGROUND`（`observer::indicator_glyph` の WinEvent）から呼ぶ。
    ///
    /// 前置きのフィルタ: 有効・タスクバーやメニューが前面になっただけではない・受け付け間隔・結果待ちが溜まっていない。
    /// 通ったときだけ、待ち時間つきの取り込みの依頼を読み取りワーカーへ送る。何も変えない。
    pub(crate) fn indicator_glyph_on_foreground(&mut self, hwnd: HWND) {
        let Some(tx) = self.indicator_glyph.tx.clone() else {
            return;
        };
        let now = Instant::now();
        let since_last_ms = self
            .indicator_glyph
            .last_trigger
            .map(|t| u64::try_from(now.duration_since(t).as_millis()).unwrap_or(u64::MAX));
        if since_last_ms.is_some_and(|ms| ms < MIN_TRIGGER_INTERVAL_MS)
            || self.indicator_glyph.pending.len() + READ_DELAYS_MS.len() > MAX_PENDING
        {
            return;
        }
        let class = crate::focus::classify::get_class_name_string(hwnd);
        if is_excluded_foreground_class(&class) {
            return;
        }
        let foreground = hwnd.0 as isize;
        for delay in READ_DELAYS_MS {
            if tx.send(GlyphReadRequest { delay_ms: delay }).is_err() {
                // ワーカーが終了している。以後は何もしない。
                tracing::warn!("[indicator-glyph] worker is gone; shadow disabled");
                self.indicator_glyph.tx = None;
                self.indicator_glyph.pending.clear();
                return;
            }
            self.indicator_glyph.pending.push_back(Pending {
                foreground,
                delay_ms: u32::from(delay),
            });
        }
        self.indicator_glyph.last_trigger = Some(now);
        tracing::debug!("[indicator-glyph] foreground changed → glyph reads requested");
    }

    /// 読み取りワーカーの結果（`WM_INDICATOR_GLYPH_READ`）を受け、journal に残す。何も変えない。
    ///
    /// 結果は依頼の順に返る。結果が返った時点で前面窓が依頼時と変わっていれば、字形は別の窓のものなので捨てる。
    /// 窓の IME プロファイルと belief は、**結果が返った時点**（フォーカス追跡が落ち着いた後）のものを添える。
    pub(crate) fn indicator_glyph_on_result(&mut self, wparam: usize, lparam: isize) {
        let Some(outcome) = ProbeOutcome::unpack(wparam, lparam) else {
            tracing::warn!(
                "[indicator-glyph] corrupt result message (wparam={wparam:#x}, lparam={lparam:#x})"
            );
            return;
        };
        let Some(pending) = self.indicator_glyph.pending.pop_front() else {
            // 失われたとみなした依頼の遅れた結果、または二重配送。文脈が無いので記録しない。
            tracing::debug!("[indicator-glyph] result without a pending request; dropped");
            return;
        };
        let now_foreground = crate::observer::indicator_glyph::foreground_window_value();
        if now_foreground != pending.foreground {
            tracing::debug!("[indicator-glyph] foreground changed during the read; result dropped");
            return;
        }
        let profile = super::lang_detect_shadow::profile_label(self.platform.current_app_profile());
        let belief_open = Some(self.platform_state.ime.effective_open());
        let record = outcome.to_record(profile, belief_open, pending.delay_ms);
        self.indicator_glyph.counters.record(&record);
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::IndicatorGlyph { record });
    }
}
