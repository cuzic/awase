//! ADR-261 段階 1: 入力インジケーターの字形の**影の取り込み・照合・収集**（挙動を変えない観測）。
//!
//! 前面窓が変わったとき（`observer::indicator_glyph` の `EVENT_SYSTEM_FOREGROUND` の WinEvent が検知）、字形が実状態に追随する
//! のを待ってから（0.9 秒後と 2.5 秒後の 2 回。字形の遅れは中央 0.2 秒・最大 0.68 秒、ADR-261）インジケーターの矩形を取り込み、
//! 前処理まで進める。その特徴を、見本の表（矩形の大きさ別。内蔵の共通の初期見本＝GJI の全 6 状態・ライト/ダークで始め、
//! 同じ環境の見本を足していく。**起動ごとに作る**。永続化は次の段階）に**照合**し、結果
//! （大きさ・前景の点数・所要・失敗の理由・照合の読みと belief の粗いラベルの一致）を journal（`JournalEntry::IndicatorGlyph`）に残す。
//!
//! 表は**受け身に集める**: 読める窓（`standard`。belief が直接の読みに追随している）で、同じ窓の 2 回の取り込みの belief のラベルが
//! 同じ（確認された状態）で字形が落ち着いているときだけ、そのラベルの見本として足す（ADR-261 決定 2。belief の入力モードは
//! ひらがな/カタカナ・半角/全角英数を区別しないので、ラベルは閉・かな・英数の 3 つ。`state::indicator_glyph::BeliefLabel`）。
//!
//! **belief には入れない・IME へ何も書かない・打鍵は変えない・画素は保存しない**。belief は読み取り（`effective_open()`・
//! `input_mode()`）だけで、ラベルの作成と記録に使う。目的は、読める窓での一致率と、観測の無い窓での belief とのずれを、実ジャーナルで
//! 確かめること。
//!
//! 有効なのは `[indicator_evidence] glyph` の実効の段階が `observe` 以上（既定は off のオプトイン。ADR-271）のときだけ。反映は再起動後
//! （フックと読み取りスレッドは起動時に 1 度だけ作る。`app/bootstrap.rs`）。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use windows::Win32::Foundation::HWND;

use crate::focus::class_names::AppImeProfile;
use crate::observer::indicator_glyph::GlyphReadRequest;
use crate::state::indicator_glyph::{
    apply_match, belief_label, collect_label, match_feature, AddOutcome, BeliefLabel, Capture,
    GlyphFeature, GlyphSample, GlyphTable, ProbeOutcome, SampleOrigin, Thresholds,
};
use crate::state::indicator_glyph_store::{EnvFingerprint, GlyphSampleFile};

/// 見本の保存ファイル名（exe と同じディレクトリ。`cache.toml` とは別ファイル。ADR-261 決定 2）。
const SAMPLES_FILENAME: &str = "glyph_samples.json";

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

/// 同じ窓の前回（待ちが短い方）の取り込み。収集の判断（字形が落ち着いているか）に使う。
struct LastCapture {
    foreground: isize,
    feature: GlyphFeature,
    label: Option<BeliefLabel>,
}

/// 影の取り込みの状態。読み取りワーカーへの送信口が無ければ（無効のとき）何もしない。
pub(super) struct IndicatorGlyphShadow {
    tx: Option<Sender<GlyphReadRequest>>,
    /// ワーカーが返す前処理後の特徴（結果のメッセージと依頼の順に 1 対 1 で対応する）
    rx_feature: Option<Receiver<Option<GlyphFeature>>>,
    pending: VecDeque<Pending>,
    last_trigger: Option<Instant>,
    last_capture: Option<LastCapture>,
    /// 見本の表（環境指紋別。矩形の大きさ〔DPI に比例〕・背景の明暗・IME の種類・取り込み方式の版）。内蔵の共通の見本で始め、
    /// 保存済みの同じ環境の見本を足す
    tables: HashMap<EnvFingerprint, GlyphTable>,
    /// 保存ファイルの内容（同じ環境で集めた見本。画素は持たない）と、その保存先（無効・保存先が無いときは `None`）
    store: GlyphSampleFile,
    store_path: Option<PathBuf>,
    /// 取り込みの件数（不具合報告の要約用。画像・画素は持たない。ADR-271 D5）
    counters: crate::indicator_evidence::IndicatorGlyphCounters,
}

impl IndicatorGlyphShadow {
    pub(super) fn disabled() -> Self {
        Self {
            tx: None,
            rx_feature: None,
            pending: VecDeque::new(),
            last_trigger: None,
            last_capture: None,
            tables: HashMap::new(),
            store: GlyphSampleFile::default(),
            store_path: None,
            counters: crate::indicator_evidence::IndicatorGlyphCounters::new(),
        }
    }
}

/// 見本の保存先へアトミックに書く。失敗しても何も変えない（見本は次の起動で集め直せる）。
fn save_samples(path: &Path, store: &GlyphSampleFile) {
    if let Err(e) = awase::fs_atomic::write_atomic(path, store.to_json().as_bytes()) {
        tracing::warn!(
            "[indicator-glyph] failed to save samples to {}: {e}",
            path.display()
        );
    }
}

/// 環境指紋に使う IME の種類（`gji` / `ms-ime`。まだ同定できていなければ空）。
fn ime_label() -> &'static str {
    match crate::tsf::observer::tsf_obs().table_ime_kind() {
        Some(crate::state::ime_kind::ImeKindId::Gji) => "gji",
        Some(crate::state::ime_kind::ImeKindId::MsIme) => "ms-ime",
        None => "",
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
    /// 起動時に、読み取りワーカーへの送信口と特徴の受信口を渡して有効にする（`None` なら無効のまま）。
    pub(crate) fn init_indicator_glyph(
        &mut self,
        tx: Option<Sender<GlyphReadRequest>>,
        rx_feature: Option<Receiver<Option<GlyphFeature>>>,
        base_dir: &Path,
    ) {
        // 保存済みの見本（同じ環境で集めたもの）を読む。壊れている・版が違う・無いときは空（集め直す）。
        let store_path = base_dir.join(SAMPLES_FILENAME);
        let store = std::fs::read_to_string(&store_path)
            .map(|json| GlyphSampleFile::from_json(&json))
            .unwrap_or_default();
        tracing::info!(
            "[indicator-glyph] stored sample environments: {} ({})",
            store.env_count(),
            store_path.display()
        );
        self.indicator_glyph = IndicatorGlyphShadow {
            tx,
            rx_feature,
            store,
            store_path: Some(store_path),
            ..IndicatorGlyphShadow::disabled()
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
        self.indicator_glyph.last_capture = None;
        tracing::debug!("[indicator-glyph] foreground changed → glyph reads requested");
    }

    /// 読み取りワーカーの結果（`WM_INDICATOR_GLYPH_READ`）を受け、照合・収集して journal に残す。何も変えない。
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
        // 特徴は結果と 1 対 1 に対応する。捨てる結果でも必ず 1 つ取り出して、対応を保つ。
        let feature = self
            .indicator_glyph
            .rx_feature
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
            .flatten();
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
        let profile_kind = self.platform.current_app_profile();
        let profile = super::lang_detect_shadow::profile_label(profile_kind);
        let belief_open = self.platform_state.ime.effective_open();
        let label = belief_label(belief_open, self.platform_state.ime.input_mode());
        let mut record = outcome.to_record(profile, Some(belief_open), pending.delay_ms);

        if let Some(f) = feature {
            // 環境指紋: 矩形の大きさ（DPI）・背景の明暗・IME の種類・取り込み方式の版。違う環境の表は使わない。
            let fingerprint =
                EnvFingerprint::new(outcome.width, outcome.height, outcome.dark, ime_label());
            let store = &self.indicator_glyph.store;
            let table = self
                .indicator_glyph
                .tables
                .entry(fingerprint.clone())
                .or_insert_with(|| {
                    // 内蔵の共通の見本で始め、保存済みの同じ環境の見本を足す。
                    let mut t = crate::state::indicator_glyph_seeds::seeded_table();
                    for s in store.samples_for(&fingerprint) {
                        let _ = t.add(s);
                    }
                    t
                });
            // 照合は、収集する前の表で行う（自分自身を見本にして一致させない）。
            let m = match_feature(table, &Thresholds::default(), &f);
            // 収集: 2 回目（待ちが長い方）の取り込みで、同じ窓の 1 回目と比べて落ち着いていれば足す。
            let is_second = pending.delay_ms == u32::from(READ_DELAYS_MS[1]);
            let prev = self
                .indicator_glyph
                .last_capture
                .as_ref()
                .filter(|p| p.foreground == pending.foreground && is_second)
                .map(|p| Capture {
                    feature: &p.feature,
                    label: p.label,
                });
            let readable = profile_kind == AppImeProfile::Standard;
            let collected = collect_label(prev, Capture { feature: &f, label }, readable)
                .is_some_and(|l| {
                    table.add(GlyphSample {
                        class: l.class(),
                        feature: f.clone(),
                        origin: SampleOrigin::SameEnv,
                    }) == AddOutcome::Added
                });
            apply_match(&mut record, &m, label, collected, table.len());
            if collected {
                // 見本が増えたときだけ保存する（まれ。アトミックに書く）。
                self.indicator_glyph.store.set_env(&fingerprint, table);
                if let Some(path) = &self.indicator_glyph.store_path {
                    save_samples(path, &self.indicator_glyph.store);
                }
            }
            self.indicator_glyph.last_capture = if is_second {
                None
            } else {
                Some(LastCapture {
                    foreground: pending.foreground,
                    feature: f,
                    label,
                })
            };
        }
        self.indicator_glyph.counters.record(&record);
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::IndicatorGlyph { record });
    }
}
