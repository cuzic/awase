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
//! **IME へ何も書かない・打鍵は変えない・画素は保存しない**。belief は読み取り（`effective_open()`・`input_mode()`）で
//! ラベルの作成と記録に使う。**旧モデルへの報告（ADR-273）は、実効の段階が `unobserved` 以上の対象の窓でだけ**（`report_glyph_to_belief`。
//! 現状は `ui_max_level` が `Observe` なので動かない）。目的は、読める窓での一致率と、観測の無い窓での belief とのずれを、実ジャーナルで
//! 確かめること。
//!
//! **ADR-283 G2(観測のみ、2026-10-28): 候補 Z([`crate::state::scope_belief::ScopeBelief`]、本番の `engine_belief()` の
//! 既定の入力源)へ、見本の表に照合してラベルが付いた読み(`LabelReading::Hit`)を今入れたら、という試算を
//! `preview_glyph_for_z` が journal に記録する(`ImeStateHub::scope_shadow_glyph_preview`、唯一の入口。副作用なし。
//! 本番の `scope_shadow` 自体には `with_glyph` を付けない。[`crate::state::scope_belief::GlyphPolicy`] を付けたクローンで
//! 試算するだけ)**。当初(2026-10-28)は直接 `scope_shadow` へ書く案で実装したが、Opus レビューで複数の正しさの問題
//! (フォーカス追跡が新しい窓に追いついたかの柵が無い・`observe` 設定の「belief には入れない」という約束が崩れる等)が
//! 見つかり、観測のみの案(当初「別インスタンスが要って重い」と判断していたが、実際はクローンで十分軽い)へ戻した。
//! D4(テキストの欄の読みか、`FocusKind::TextInput` 限定)・D5(窓の信頼、Z 自身の `scope_shadow_facts()` を読む)・
//! D3-b(MS-IME本体か)を殻側で判定し、旧経路と同じフォーカスの同一性の柵(`glyph_read_fence`)を通った読みだけ試算する。
//! トリガーは今のところ前面窓の変化(T1)だけ(ADR-283のT2〜T4は未実装)。`StateChange`(欄の移動・モード系打鍵)は
//! まだ核へ渡していない(ADR-283 D2-1、今後の課題)。
//!
//! 有効なのは `[indicator_evidence] glyph` の実効の段階が `observe` 以上（既定は off のオプトイン。ADR-271）のときだけ。反映は再起動後
//! （フックと読み取りスレッドは起動時に 1 度だけ作る。`app/bootstrap.rs`）。候補 Z への試算(上記)もこの設定に従う:
//! 取り込みワーカー・WinEvent フック自体がこの設定で起動するかどうかを決めるので、既定(off)のままなら
//! `preview_glyph_for_z` は一度も呼ばれない。既定を変えて `observe` 以上にした利用者だけ、journal に試算が残る
//! (belief への書き込みは、observe の間は一切無い)。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use windows::Win32::Foundation::HWND;

use crate::focus::class_names::AppImeProfile;
use crate::observer::indicator_glyph::GlyphReadRequest;
use crate::state::indicator_glyph::{
    apply_match, belief_label, classify_label, collect_label, first_read_fence, glyph_scope_label,
    glyph_window_in_scope, match_feature, AddOutcome, BeliefLabel, Capture, GlyphFeature,
    GlyphMiss, GlyphSample, GlyphScopeFacts, GlyphTable, ProbeOutcome, SampleOrigin, Thresholds,
};
use crate::state::indicator_glyph_store::{EnvFingerprint, GlyphSampleFile};
use crate::state::probe_admission::FocusFence;

/// 見本の保存ファイル名（exe と同じディレクトリ。`cache.toml` とは別ファイル。ADR-261 決定 2）。
const SAMPLES_FILENAME: &str = "glyph_samples.json";

/// 前面窓の変化から取り込むまでの待ち（ミリ秒）。字形の遅れ（最大 0.68 秒、ADR-261）より長い 2 点。
const READ_DELAYS_MS: [u16; 2] = [900, 2500];
/// 前面窓の変化の受け付け間隔の下限（Alt+Tab の連打で依頼が溜まらないように）。
const MIN_TRIGGER_INTERVAL_MS: u64 = 3000;
/// 結果待ちの依頼の上限（これを超えて溜まっていたら新しい依頼は出さない）。
const MAX_PENDING: usize = 4;
/// 見本に無い字形の形をログに出す上限（起動ごと。同じ形は 1 度だけ）。
const MAX_MISS_SHAPES: usize = 24;

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
    /// 1 回目（待ちが短い方）の取り込みを処理した時点の（前面窓、フォーカスの同一性〔epoch + hwnd〕）。2 回目の取り込みの報告の基準
    /// （前面窓の変化の通知はフォーカス追跡より先に届くので、依頼の時点ではなく、追跡が追いついた後の 1 回目の時点を基準にする）。
    /// 新しい依頼のたびに消す（1 回目が取り込めなかったときは基準が無く、2 回目は報告しない）
    baseline: Option<(isize, FocusFence)>,
    /// 診断: 見本に無い字形（`TooFar`/`Ambiguous`）の形をログに出した指紋（同じ形は 1 度だけ。上限あり）
    miss_shapes: std::collections::HashSet<u64>,
    /// 見本の表（環境指紋別。矩形の大きさ〔DPI に比例〕・背景の明暗・IME の種類・取り込み方式の版）。内蔵の共通の見本で始め、
    /// 保存済みの同じ環境の見本を足す
    tables: HashMap<EnvFingerprint, GlyphTable>,
    /// 保存ファイルの内容（同じ環境で集めた見本。画素は持たない）と、その保存先（無効・保存先が無いときは `None`）
    store: GlyphSampleFile,
    store_path: Option<PathBuf>,
    /// 取り込みの件数（不具合報告の要約用。画像・画素は持たない。ADR-271 D5）
    counters: crate::indicator_evidence::IndicatorGlyphCounters,
    /// 起動時に固定した実効の段階（`min(設定値, ui_max_level())`）。無効のときも持つ（報告は `off` でも段階を出す）
    effective: Option<crate::indicator_evidence::EffectiveEvidence>,
}

impl IndicatorGlyphShadow {
    pub(super) fn disabled() -> Self {
        Self {
            tx: None,
            rx_feature: None,
            pending: VecDeque::new(),
            last_trigger: None,
            last_capture: None,
            baseline: None,
            miss_shapes: std::collections::HashSet::new(),
            tables: HashMap::new(),
            store: GlyphSampleFile::default(),
            store_path: None,
            counters: crate::indicator_evidence::IndicatorGlyphCounters::new(),
            effective: None,
        }
    }
}

/// 診断: 見本に無い字形（`TooFar`/`Ambiguous`）の形（前処理後の 2 値画像。元の画素ではない）を、同じ形は 1 度だけログに出す
/// （起動ごとに最大 `MAX_MISS_SHAPES` 件）。実機で曖昧になる窓の字形を後から確かめ、見本に足すか決める材料。
fn log_miss_shape(
    shapes: &mut std::collections::HashSet<u64>,
    feature: &GlyphFeature,
    miss: Option<GlyphMiss>,
    profile: &str,
    outcome: &ProbeOutcome,
    table_len: usize,
) {
    let Some(miss @ (GlyphMiss::TooFar | GlyphMiss::Ambiguous)) = miss else {
        return;
    };
    if shapes.len() >= MAX_MISS_SHAPES || !shapes.insert(feature.mask_fingerprint()) {
        return;
    }
    tracing::info!(
        target: "awase::glyph_miss",
        "[indicator-glyph] miss-shape {} miss={} profile={profile} size={}x{} dark={} table={table_len}",
        feature.mask_art(),
        miss.label(),
        outcome.width,
        outcome.height,
        outcome.dark,
    );
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

/// ADR-283 D3-b: 今の窓が MS-IME 本体と同定済みか(`focus_tracking.rs::scope_shadow_window_facts` の
/// `inherit_open_on_entry` の前半 2 項〔WT 判定〕と同じ判定。`inherit_open_on_entry` 自身は後半に `imm_trusted` も
/// 含むが、ここでは含めない)。
fn glyph_ms_ime_native() -> bool {
    use crate::tsf::observer::{tsf_obs, ActiveImeKind};
    matches!(tsf_obs().active_ime_kind(), ActiveImeKind::MicrosoftIme)
        && tsf_obs().ms_ime_native_identified()
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
            effective: self.indicator_glyph.effective,
            ..IndicatorGlyphShadow::disabled()
        };
    }

    /// 起動時に実効の段階を固定する（有効・無効にかかわらず呼ぶ。不具合報告が `off` でも段階を出すため。ADR-271 D5）。
    pub(crate) fn set_indicator_effective_evidence(
        &mut self,
        effective: crate::indicator_evidence::EffectiveEvidence,
    ) {
        self.indicator_glyph.effective = Some(effective);
    }

    /// 字形の実効の段階（起動時に固定したもの。まだ渡されていなければ `None`）。
    fn effective_glyph_level(&self) -> Option<awase::config::EvidenceLevel> {
        self.indicator_glyph.effective.map(|e| e.glyph)
    }

    /// 不具合報告の要約用に、実効の段階と取り込みの件数を返す（読み取りだけ）。
    pub(crate) fn indicator_evidence_summary(
        &self,
    ) -> crate::bug_report::BugReportIndicatorEvidence {
        let eff = self.indicator_glyph.effective.unwrap_or(
            crate::indicator_evidence::EffectiveEvidence {
                glyph: awase::config::EvidenceLevel::Off,
            },
        );
        crate::bug_report::BugReportIndicatorEvidence::new(eff, &self.indicator_glyph.counters)
    }

    /// 窓の種類の事実（D3・Q2・R1。判定は核の純粋関数 `glyph_window_in_scope`・`glyph_scope_label`）。
    fn glyph_scope_facts(&self) -> GlyphScopeFacts {
        let profile = self.platform.current_app_profile();
        GlyphScopeFacts {
            // 実効のプロファイル（学習で降格した後の値。降格した窓は `demoted_by_learning` の側で拾う）
            effective_standard: profile == AppImeProfile::Standard,
            demoted_by_learning: self.platform.focus.is_demoted_by_learning(),
            tsf_native: profile.is_effectively_tsf_native(self.platform.focus.class_name()),
            input_relay: matches!(profile, AppImeProfile::InputRelay),
            mozc_block: self.mozc_shm.block_present(),
        }
    }

    /// 字形を belief に使ってよい窓・設定か（ADR-273 D3・Q2・R1）。設定の実効の段階が `unobserved` 以上のときだけ（現状は
    /// `EvidenceSource::ui_max_level()` が `Observe` なので実効の段階は上がらず、ここは常に偽）。窓の判定は核の純粋関数。
    fn glyph_belief_in_scope(&self) -> bool {
        let level = self
            .effective_glyph_level()
            .unwrap_or(awase::config::EvidenceLevel::Off);
        glyph_window_in_scope(level, self.glyph_scope_facts())
    }

    /// ADR-283 D5: この字形の読みを候補 Z([`crate::state::scope_belief::ScopeBelief`])の試算に使ってよい窓か。
    ///
    /// `imm_trusted` は Z 自身が入場時に固定した値([`crate::state::platform_state::ImeStateHub::scope_shadow_facts`]、
    /// `runtime/focus_tracking.rs::scope_shadow_window_facts` が作る)を読む(読みの時点で再計算すると、入場後に学習で
    /// 降格された場合に Z の判定とずれる。Opus レビュー指摘 M4、2026-10-28)。未入場なら信頼できない側(`false`)。
    fn glyph_z_window_ok(&self) -> bool {
        use crate::state::scope_belief::glyph_window_ok;
        let imm_trusted = self
            .platform_state
            .ime
            .scope_shadow_facts()
            .is_some_and(|f| f.imm_trusted);
        let profile = self.platform.current_app_profile();
        let input_relay = matches!(profile, AppImeProfile::InputRelay);
        glyph_window_ok(imm_trusted, input_relay, self.mozc_shm.block_present())
    }

    /// ADR-283 G2(観測のみ、2026-10-28 Opus レビューを受けて直接配線から戻した): この字形の読みを今候補 Z へ
    /// 入れたら、という試算を journal に記録する(**副作用なし。belief には一切書かない**)。呼び出し元
    /// ([`Self::indicator_glyph_on_result`])が、フォーカス追跡が新しい窓に追いついている読みだけに絞る(B2)。
    ///
    /// D4(テキストの欄の読みか、`FocusKind::TextInput` を要求。`Undetermined` は通さない。Opus レビュー指摘 B3)・
    /// D5(`glyph_z_window_ok`)・D3-b(MS-IME 本体か)をここで決め、柵・写像・2回一致の要否は核
    /// ([`crate::state::scope_belief::ScopeBelief::evolve`])が判断する。`captured_ms` は取り込みの所要
    /// (`read_ms`)を引いた近似値(Opus レビュー指摘 M3: 処理した「今」のままだと柵の区間が実際の取り込みより
    /// 後ろへずれ、緩む方向に働く)。
    fn preview_glyph_for_z(
        &self,
        reading: crate::state::indicator_glyph::LabelReading,
        read_ms: u16,
        record: &mut crate::journal::IndicatorGlyphRecord,
    ) {
        use crate::state::indicator_glyph::LabelReading;
        let LabelReading::Hit(hit) = reading else {
            return;
        };
        let is_text_field =
            self.platform_state.focus.focus_kind == crate::focus::FocusKind::TextInput;
        let eligible = is_text_field && self.glyph_z_window_ok();
        let ms_ime_native = glyph_ms_ime_native();
        let captured_ms = crate::hook::current_tick_ms().saturating_sub(u64::from(read_ms));
        let flip = self.platform_state.ime.scope_shadow_glyph_preview(
            crate::state::scope_belief::ScopeEvent::Glyph {
                label: hit.label,
                captured_ms,
                eligible,
                ms_ime_native,
            },
        );
        record.would_decide_z = Some(flip.is_some());
        record.would_flip_z = flip;
    }

    /// 報告の基準となる同一性（ADR-273 D1 の M8、Opus RE1）。1 回目は**処理した時点**の同一性（追跡が追いついているときだけ。
    /// 前面窓の変化の通知はフォーカス追跡より先に届くので、依頼の時点の値は切り替える前の窓のものになりうる）、2 回目は
    /// 1 回目の基準（同じ前面窓のときだけ）。無ければ報告しない。
    fn glyph_read_fence(&self, pending: &Pending, is_second: bool) -> Option<FocusFence> {
        if is_second {
            self.indicator_glyph
                .baseline
                .filter(|(f, _)| *f == pending.foreground)
                .map(|(_, fence)| fence)
        } else {
            first_read_fence(
                self.platform_state.ime.focus_fence(),
                self.lang_detect_focus_in_sync(),
            )
        }
    }

    /// 字形の読みを belief に報告する（ADR-273 D1・D4）。**設定の実効の段階が `unobserved` 以上で、対象の窓のときだけ**
    /// （`glyph_belief_in_scope`。現状は `ui_max_level` が `Observe` なので動かない）。判断・柵・記録は `ImeStateHub` 側。
    fn report_glyph_to_belief(
        &mut self,
        reading: crate::state::indicator_glyph::LabelReading,
        read_fence: FocusFence,
    ) {
        if !self.glyph_belief_in_scope() {
            return;
        }
        let outcome = self.platform_state.ime.report_indicator_glyph(
            reading,
            read_fence,
            self.lang_detect_focus_in_sync(),
            crate::state::TickMs(crate::hook::current_tick_ms()),
        );
        tracing::debug!("[indicator-glyph] belief report: {outcome:?}");
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
        self.indicator_glyph.baseline = None;
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
        // R2: 字形が決めた belief で字形の見本にラベルを付けない（誤読が見本に永続化して自分を強化する・一致の集計が同語反復になる）。
        // 判定は hub の層の最終的な理由（`IntentStore` の上書きを字形由来と数えない。R6）。
        let decided_by_glyph = self
            .platform_state
            .ime
            .belief_is_decided_by_glyph(crate::state::TickMs(crate::hook::current_tick_ms()));
        let label = if decided_by_glyph {
            None
        } else {
            belief_label(belief_open, self.platform_state.ime.input_mode())
        };
        let mut record = outcome.to_record(profile, Some(belief_open), pending.delay_ms);
        let mut glyph_reading = None;

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
            glyph_reading = Some(classify_label(table, &Thresholds::default(), &f));
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
            log_miss_shape(
                &mut self.indicator_glyph.miss_shapes,
                &f,
                m.miss,
                profile,
                &outcome,
                table.len(),
            );
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
        if let Some(reading) = glyph_reading {
            let is_second = pending.delay_ms == u32::from(READ_DELAYS_MS[1]);
            let read_fence = self.glyph_read_fence(&pending, is_second);
            // ADR-283 G2(観測のみ): 旧経路と同じ柵(フォーカス追跡が新しい窓に追いついている読みだけ、B2)を通った
            // ときだけ、候補 Z への試算を記録する(belief には書かない)。
            if read_fence.is_some() {
                self.preview_glyph_for_z(reading, outcome.read_ms, &mut record);
            }
            // 設定が observe のままでも、対象の窓だったとして柵を通るかを記録する（belief には入れない）。段階 2 の前提の件数。
            // 基準が作れなかった読み（追跡のずれ）も分母に入れるため、窓の種類は基準の有無によらず出す。
            let scope = glyph_scope_label(self.glyph_scope_facts());
            record.would_scope = Some(scope);
            match read_fence {
                Some(read_fence) => {
                    if scope != "none" {
                        let preview = self.platform_state.ime.preview_indicator_glyph(
                            reading,
                            read_fence,
                            self.lang_detect_focus_in_sync(),
                            crate::state::TickMs(crate::hook::current_tick_ms()),
                        );
                        record.would_report = Some(preview.report.label());
                        if preview.report.is_reported() {
                            record.would_decide = Some(preview.decides.is_some());
                            record.would_flip = preview.decides;
                        }
                    }
                    self.report_glyph_to_belief(reading, read_fence);
                }
                None if scope != "none" => record.would_report = Some("fenced:no-baseline"),
                None => {}
            }
            if !is_second {
                // 2 回目の基準: 1 回目を処理した時点の同一性（追跡が追いついていなければ作らない。2 回目は報告しない）。
                self.indicator_glyph.baseline = read_fence.map(|fence| (pending.foreground, fence));
            }
        }
        self.indicator_glyph.counters.record(&record);
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::IndicatorGlyph { record });
    }
}
