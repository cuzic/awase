//! ADR-263 PR 2a: 窓切替直後・長い無操作のあとのエピソードの**影の判定**（挙動を変えない観測）。
//!
//! 取り込み口（`app/mod.rs::handle_hook_key_event`、フックの打鍵が届いた順）で、打鍵を `awase::lang_detect::EpisodeTracker`
//! に渡し、エピソードが閉じたら結果を journal（`JournalEntry::LangDetect`）に残す。打鍵は止めない・書き換えない・
//! IME に何も書かない。目的は、実ジャーナルで判定の誤り率・閾値・無操作の長さを確かめること（PR 2b の前提）。
//!
//! 記録に**キー位置の列は含めない**（打鍵ごとの尤度比の増分の列と、親指との重なりの有無だけ）。

use awase::config::{AutoDetectConfig, AutoDetectMode};
use awase::lang_detect::{
    DetectParams, EpisodeOutcome, EpisodeTracker, JapaneseEvidence, KanaModel, KeyRole,
    ObservedKey, ThumbOverlap, TrackerConfig, Verdict,
};
use awase::ngram::NgramModel;
use awase::types::{KeyClassification, KeyEventType, RawKeyEvent};

use crate::focus::class_names::AppImeProfile;

/// 影の判定の状態。`mode = off`、または n-gram 表を読めなかったときは何もしない。
pub(super) struct LangDetectShadow {
    mode: AutoDetectMode,
    tracker: EpisodeTracker,
    /// 仮名 n-gram の生の表（配列の切替でモデルを作り直すために持つ）
    ngram: Option<NgramModel>,
    model: Option<KanaModel>,
    /// `model` を作った配列の名前
    model_layout: String,
}

impl LangDetectShadow {
    pub(super) fn disabled() -> Self {
        Self::new(&AutoDetectConfig::default(), None, AutoDetectMode::Off)
    }

    /// `ngram` が `None`（表を読めなかった）ときは `mode` を問わず何もしない。
    pub(super) fn new(
        cfg: &AutoDetectConfig,
        ngram: Option<NgramModel>,
        mode: AutoDetectMode,
    ) -> Self {
        let tracker_cfg = TrackerConfig {
            params: DetectParams {
                ja_threshold: cfg.ja_threshold,
                en_threshold: cfg.en_threshold,
                max_keys: usize::try_from(cfg.max_keys).unwrap_or(7),
            },
            // 影の判定では、Space を親指にした構成の除外をしない（誤判定率を実測するため。記録で見分ける）
            thumb_overlap: ThumbOverlap::Enabled,
            idle_threshold_ms: u64::from(cfg.idle_ms),
            max_ms: u64::from(cfg.max_ms),
        };
        let mode = if ngram.is_some() {
            mode
        } else {
            AutoDetectMode::Off
        };
        Self {
            mode,
            tracker: EpisodeTracker::new(tracker_cfg),
            ngram,
            model: None,
            model_layout: String::new(),
        }
    }

    const fn is_active(&self) -> bool {
        matches!(self.mode, AutoDetectMode::Observe)
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

    fn on_key(&mut self, key: ObservedKey, scope: u64) -> Option<EpisodeOutcome> {
        let model = self.model.as_ref()?;
        self.tracker.on_key(model, key, scope)
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

pub(super) const fn profile_label(profile: AppImeProfile) -> &'static str {
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
    /// 取り込み口から、フックの打鍵 1 つにつき 1 回呼ぶ（KeyDown/KeyUp とも）。何も変えない。
    pub(crate) fn lang_detect_on_key(&mut self, event: &RawKeyEvent) {
        if !self.lang_detect.is_active() {
            return;
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
        let Some(outcome) = self.lang_detect.on_key(key, scope) else {
            return;
        };

        let (verdict, evidence) = verdict_labels(outcome.verdict);
        #[allow(clippy::cast_possible_truncation)] // ログ用。f32 の精度で足りる
        let llr = outcome.llr as f32;
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
        };
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::LangDetect { record });
    }

    /// 起動時・設定リロード時に、`[auto_detect]` と n-gram の表で影の判定を作り直す。
    pub(crate) fn init_lang_detect(&mut self, cfg: &AutoDetectConfig, ngram: Option<NgramModel>) {
        self.lang_detect = LangDetectShadow::new(cfg, ngram, cfg.mode);
    }
}
