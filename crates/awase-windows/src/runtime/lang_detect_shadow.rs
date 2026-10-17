//! ADR-263: 窓切替直後・長い無操作のあとのエピソード先頭の打鍵を判定する殻。
//!
//! - `mode = observe`（PR 2a）: 打鍵は止めず、判定器を影として走らせて結果を journal（`JournalEntry::LangDetect`）に残すだけ。
//! - `mode = ime_on`（PR 2b）: 観測の無い窓（実測学習で降格された窓）で belief が ON のとき、エピソードの打鍵を最大
//!   `max_keys` 打鍵・`max_ms` ミリ秒だけ**取り込み口で保持**し、日本語入力と判定できたときだけ、ひらがなキー
//!   （`VK_DBE_HIRAGANA`）を 1 回送ってから、保持した打鍵を順序どおりに流す（`INPUT_DEFER.replay_later`）。
//!   英数・決まらなかった・打ち切りのときは、保持した打鍵を今までと同じ流れに戻すだけ（IME に書かない・素通しにもしない）。
//!   保持中に前面窓が変わったら、保持した打鍵は捨てる（元の窓の打鍵が別の窓へ入る誤送信を避ける）。
//!
//! 判断（保持・解放・判定）は root crate の `awase::lang_detect::HoldController`（純粋、Linux でテスト）。ここは
//! `RawKeyEvent` の分類、IME への書き込みの前提条件の確認、journal、タイマー、再生の接続だけを持つ。
//! 記録に**キー位置の列は含めない**。

use std::time::Duration;

use awase::config::{AutoDetectConfig, AutoDetectMode};
use awase::lang_detect::{
    DetectParams, HoldController, HoldStep, JapaneseEvidence, KanaModel, KeyRole, ObservedKey,
    ThumbOverlap, TrackerConfig, Verdict,
};
use awase::ngram::NgramModel;
use awase::types::{KeyClassification, KeyEventType, RawKeyEvent};

use crate::focus::class_names::AppImeProfile;
use crate::focus::FocusKind;

/// 取り込み口への指示
pub(crate) enum LangDetectIntake {
    /// この打鍵は通常どおり処理を続ける
    Pass,
    /// この打鍵は保持した（または解放の再生に含めた）ので、取り込み口はここで終わる
    Consumed,
}

/// 判定の状態。`mode = off`、または n-gram 表を読めなかったときは何もしない。
pub(super) struct LangDetectShadow {
    mode: AutoDetectMode,
    controller: HoldController<RawKeyEvent>,
    /// 仮名 n-gram の生の表（配列の切替でモデルを作り直すために持つ）
    ngram: Option<NgramModel>,
    model: Option<KanaModel>,
    /// `model` を作った配列の名前
    model_layout: String,
    /// 設定が保持と両立するか（Space を親指にした構成・ADR-255 の遅いルールがあると偽。起動時に決める）
    hold_config_ok: bool,
    include_apps: Vec<String>,
    exclude_apps: Vec<String>,
    max_ms: u32,
    /// 直近の打鍵の時刻（ミリ秒。タイマーでエピソードを閉じるときの時刻に使う）
    last_key_ms: u64,
}

/// 設定のプロセス名の正規化（小文字化し、末尾の `.exe` を落とす）
fn normalize_app(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

impl LangDetectShadow {
    pub(super) fn disabled() -> Self {
        Self::new(&AutoDetectConfig::default(), None, true)
    }

    /// `ngram` が `None`（表を読めなかった）ときは `mode` を問わず何もしない。
    pub(super) fn new(
        cfg: &AutoDetectConfig,
        ngram: Option<NgramModel>,
        hold_config_ok: bool,
    ) -> Self {
        let tracker_cfg = TrackerConfig {
            params: DetectParams {
                ja_threshold: cfg.ja_threshold,
                en_threshold: cfg.en_threshold,
                max_keys: usize::try_from(cfg.max_keys).unwrap_or(7),
            },
            // 重なりを証拠にする。Space を親指にした構成では保持自体をしない（`hold_config_ok`）ので、
            // 観測では見分けられるようにそのままにする（記録の `evidence` と `profile` で後から見分ける）。
            thumb_overlap: ThumbOverlap::Enabled,
            idle_threshold_ms: u64::from(cfg.idle_ms),
            max_ms: u64::from(cfg.max_ms),
        };
        let mode = if ngram.is_some() {
            cfg.mode
        } else {
            AutoDetectMode::Off
        };
        Self {
            mode,
            controller: HoldController::new(tracker_cfg),
            ngram,
            model: None,
            model_layout: String::new(),
            hold_config_ok,
            include_apps: cfg.include_apps.iter().map(|a| normalize_app(a)).collect(),
            exclude_apps: cfg.exclude_apps.iter().map(|a| normalize_app(a)).collect(),
            max_ms: cfg.max_ms,
            last_key_ms: 0,
        }
    }

    const fn is_active(&self) -> bool {
        !matches!(self.mode, AutoDetectMode::Off)
    }

    /// 配列が変わった（または初回）なら、未シフト面でモデルを作り直す。
    fn ensure_model(&mut self, layout_name: &str, normal: Option<&awase::yab::YabFace>) {
        if self.model.is_some() && self.model_layout == layout_name {
            return;
        }
        let (Some(ngram), Some(normal)) = (self.ngram.as_ref(), normal) else {
            return;
        };
        self.model = Some(KanaModel::new(ngram, normal));
        self.model_layout = layout_name.to_string();
    }

    /// プロセス名が対象か（`include_apps` が空でなければその窓だけ、`exclude_apps` は除く）
    fn app_allowed(&self, process_name: &str) -> bool {
        let name = normalize_app(process_name);
        if self.exclude_apps.contains(&name) {
            return false;
        }
        self.include_apps.is_empty() || self.include_apps.contains(&name)
    }

    fn on_key(
        &mut self,
        key: ObservedKey,
        event: RawKeyEvent,
        scope: u64,
        may_hold: bool,
    ) -> Option<HoldStep<RawKeyEvent>> {
        let model = self.model.as_ref()?;
        self.last_key_ms = key.timestamp_ms;
        Some(self.controller.on_key(model, key, event, scope, may_hold))
    }
}

/// `RawKeyEvent` から追跡器への 1 打鍵を作る（純粋）。
pub(super) fn observed_key(event: &RawKeyEvent) -> ObservedKey {
    let down = matches!(event.event_type, KeyEventType::KeyDown);
    let m = event.modifier_snapshot;
    let ime = &event.ime_relevance;
    // Alt なりすましの親指（`"Left Alt"`）は Alt が立ちうるので、`impersonated` なら除外しない（ADR-263 m6）。
    let alt = m.alt && !event.impersonated;
    let excluded = event.injected
        || m.ctrl
        || alt
        || m.win
        || ime.is_ime_control
        || ime.is_ime_mode_key
        || ime.is_sync_key
        || ime.shadow_action.is_some();
    let role = match event.key_classification {
        KeyClassification::Char => event.physical_pos.map_or(KeyRole::Other, KeyRole::Char),
        KeyClassification::LeftThumb => KeyRole::Thumb(awase::engine::fsm_types::ThumbSide::Left),
        KeyClassification::RightThumb => KeyRole::Thumb(awase::engine::fsm_types::ThumbSide::Right),
        KeyClassification::Passthrough => {
            if event.modifier_key.is_some() {
                KeyRole::Modifier
            } else {
                KeyRole::Other
            }
        }
    };
    ObservedKey {
        role,
        down,
        excluded,
        timestamp_ms: event.timestamp / 1000,
    }
}

/// 前面窓の識別子（窓が変わったかの比較にだけ使う）。
fn scope_id(scope: crate::win32::ForegroundScope) -> u64 {
    let hwnd = u64::try_from(scope.hwnd.unsigned_abs()).unwrap_or(0);
    u64::from(scope.pid).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ hwnd
}

const fn profile_label(profile: AppImeProfile) -> &'static str {
    match profile {
        AppImeProfile::Standard => "standard",
        AppImeProfile::Imm32Unavailable => "imm32-unavailable",
        AppImeProfile::TsfNative => "tsf-native",
        AppImeProfile::InputRelay => "input-relay",
    }
}

fn verdict_labels(verdict: Verdict) -> (&'static str, Option<&'static str>) {
    match verdict {
        Verdict::Japanese(JapaneseEvidence::ThumbOverlap) => ("japanese", Some("thumb-overlap")),
        Verdict::Japanese(JapaneseEvidence::Likelihood) => ("japanese", Some("likelihood")),
        Verdict::NotJapanese => ("not-japanese", None),
        Verdict::Pending | Verdict::Undecided => ("undecided", None),
    }
}

impl super::Runtime {
    /// 保持してよい窓・状態か（エピソードが**始まる**打鍵でだけ効く。進行中のエピソードの保持は変わらない）。
    ///
    /// 観測の無い窓（実測学習で降格された窓）で、belief が ON、NonText でなく、IME の変換中でないこと
    /// （変換中は未確定文字列に打鍵が足されるので保持しない）。
    fn lang_detect_may_hold(&self) -> bool {
        self.lang_detect.mode == AutoDetectMode::ImeOn
            && self.lang_detect.hold_config_ok
            && self.platform_state.ime.effective_open()
            && self.platform.focus.is_demoted_by_learning()
            && self.platform_state.focus.focus_kind != FocusKind::NonText
            && !crate::tsf::observer::ime_composition_active_now()
            && self
                .lang_detect
                .app_allowed(&self.platform.focus.current.process_name)
    }

    /// 日本語と判定したとき、ひらがなキーを 1 回送る。送る前提を満たさなければ送らず、理由を返す。
    ///
    /// 前提: belief が ON、観測の無い窓、NonText でない、Alt/Win が押されていない（Alt 中の合成かなキーは
    /// JIS かな入力への切替に化ける。`kp_restore_kana_from_half_width` と同じ判定）。Ctrl/Shift は送信口が一時解放する。
    /// 解放時点でもう一度確かめる（保持の間にフォーカス追跡や belief が変わりうる）。
    fn lang_detect_write_hiragana(&mut self) -> &'static str {
        if self.lang_detect.mode != AutoDetectMode::ImeOn {
            return "none";
        }
        if !self.platform_state.ime.effective_open() {
            return "skipped-belief-off";
        }
        if !self.platform.focus.is_demoted_by_learning()
            || self.platform_state.focus.focus_kind == FocusKind::NonText
        {
            return "skipped-observed-window";
        }
        if crate::hook::ime_mode_key_injection_blocked_by_modifier() {
            return "skipped-modifier-held";
        }
        if crate::ime::send_hiragana_key() {
            "hiragana-sent"
        } else {
            "skipped-send-failed"
        }
    }

    /// 取り込み口から、フックの打鍵 1 つにつき 1 回呼ぶ（KeyDown/KeyUp とも）。
    ///
    /// `observe` では何も変えない（常に `Pass`）。`ime_on` では、保持した打鍵は `Consumed`（解放はタイマーか判定で、
    /// `INPUT_DEFER` 経由の再生で流れる）。
    pub(crate) fn lang_detect_on_key(&mut self, event: &RawKeyEvent) -> LangDetectIntake {
        if !self.lang_detect.is_active() {
            return LangDetectIntake::Pass;
        }
        let layout_name = self.platform.tray.current_layout_name().to_string();
        let normal = self
            .layouts
            .iter()
            .find(|entry| entry.name == layout_name)
            .map(|entry| &entry.layout.normal);
        self.lang_detect.ensure_model(&layout_name, normal);

        let key = observed_key(event);
        let scope = scope_id(crate::win32::foreground_scope());
        let may_hold = self.lang_detect_may_hold();
        let Some(step) = self.lang_detect.on_key(key, *event, scope, may_hold) else {
            return LangDetectIntake::Pass;
        };
        self.lang_detect_apply(step)
    }

    /// 保持の期限のタイマー（`TIMER_LANG_HOLD`）。保持中のエピソードを閉じて、保持した打鍵を解放する。
    pub(crate) fn lang_detect_on_timer(&mut self) {
        let now = self.lang_detect.last_key_ms;
        let step = self.lang_detect.controller.on_timer(now);
        let _ = self.lang_detect_apply(step);
    }

    /// `HoldStep` を実行する: journal、IME への書き込み、保持した打鍵の再生、タイマー。
    fn lang_detect_apply(&mut self, step: HoldStep<RawKeyEvent>) -> LangDetectIntake {
        let action = if step.japanese {
            self.lang_detect_write_hiragana()
        } else {
            "none"
        };
        let discarded = u32::try_from(step.discarded).unwrap_or(u32::MAX);
        for outcome in step.outcomes {
            let (verdict, evidence) = verdict_labels(outcome.verdict);
            #[allow(clippy::cast_possible_truncation)] // ログ用。f32 の精度で足りる
            let llr = outcome.llr as f32;
            let japanese = matches!(outcome.verdict, Verdict::Japanese(_));
            let record = crate::journal::LangDetectRecord {
                trigger: outcome.trigger.label(),
                verdict,
                evidence,
                stop: outcome.stop.label(),
                presses: u32::try_from(outcome.presses).unwrap_or(u32::MAX),
                llr,
                llr_trace: outcome.llr_trace,
                idle_ms: outcome.idle_ms,
                duration_ms: outcome.duration_ms,
                profile: profile_label(self.platform.current_app_profile()),
                belief_open: Some(self.platform_state.ime.effective_open()),
                action: if japanese { action } else { "none" },
                discarded,
            };
            self.platform_state
                .ime
                .journal
                .record(crate::journal::JournalEntry::LangDetect { record });
        }
        if step.discarded > 0 {
            tracing::debug!(
                "[lang-detect] 保持中に前面窓が変わったので {} 打鍵を捨てた",
                step.discarded
            );
        }

        // 保持した打鍵は、書き込みのあとに順序どおりに流す（先頭に書き込みの合成キー、続いて元の打鍵）。
        if !step.release.is_empty() {
            crate::INPUT_DEFER.replay_later(step.release);
        }

        if step.started_holding {
            self.platform.timer.set(
                crate::TIMER_LANG_HOLD,
                Duration::from_millis(u64::from(self.lang_detect.max_ms)),
            );
        } else if !step.holding {
            self.platform.timer.kill(crate::TIMER_LANG_HOLD);
        }

        if step.pass.is_some() {
            LangDetectIntake::Pass
        } else {
            LangDetectIntake::Consumed
        }
    }

    /// 起動時・設定リロード時に、`[auto_detect]` と n-gram の表で判定を作り直す。
    /// `hold_config_ok` は、設定が保持と両立するか（Space を親指にした構成・ADR-255 の遅いルールがあると偽）。
    pub(crate) fn init_lang_detect(
        &mut self,
        cfg: &AutoDetectConfig,
        ngram: Option<NgramModel>,
        hold_config_ok: bool,
    ) {
        self.platform.timer.kill(crate::TIMER_LANG_HOLD);
        self.lang_detect = LangDetectShadow::new(cfg, ngram, hold_config_ok);
    }
}
