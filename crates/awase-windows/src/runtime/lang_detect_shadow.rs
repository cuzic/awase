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

use awase::config::{AutoDetectConfig, AutoDetectMode, AutoDetectScope};
use awase::lang_detect::{
    eisu_write_skip_reason, hiragana_write_skip_reason, DetectParams, HoldController, HoldStep,
    JapaneseEvidence, KanaModel, KeyRole, ObservedKey, ThumbOverlap, TrackerConfig, Verdict,
    WriteFacts,
};
use awase::ngram::NgramModel;
use awase::platform::{ImeOpenOutcome, TsfComposition as _};

use crate::state::half_width_alnum::HalfWidthAlnumEffect;
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
    /// `ime_on` の対象にする窓の範囲（既定は観測の無い窓だけ。`all` は利用者の責任）
    scope: AutoDetectScope,
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
                // `ime_on_eisu` は『日本語でない』の確信度で 半角英数へ入るので、より強い確信を要求する
                // （その分、英数側の保持が長くなる）。他のモードは早く流すための小さめの値。
                en_threshold: if cfg.mode == AutoDetectMode::ImeOnEisu {
                    cfg.eisu_threshold.max(cfg.en_threshold)
                } else {
                    cfg.en_threshold
                },
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
            scope: cfg.scope,
        }
    }

    const fn is_active(&self) -> bool {
        !matches!(self.mode, AutoDetectMode::Off)
    }

    /// 打鍵を保持して書き込みまで行うモードか（`ime_on`・`ime_on_eisu`）
    const fn is_hold_mode(&self) -> bool {
        matches!(self.mode, AutoDetectMode::ImeOn | AutoDetectMode::ImeOnEisu)
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
        Some(self.controller.on_key(model, key, event, scope, may_hold))
    }
}

/// IME のキー（IME 制御・モードキー・同期キー・IME 設定由来の役割を持つキー）か。
fn is_ime_key(event: &RawKeyEvent) -> bool {
    let ime = &event.ime_relevance;
    ime.is_ime_control || ime.is_ime_mode_key || ime.is_sync_key || ime.shadow_action.is_some()
}

/// `RawKeyEvent` から追跡器への 1 打鍵を作る（純粋）。
pub(super) fn observed_key(event: &RawKeyEvent) -> ObservedKey {
    let down = matches!(event.event_type, KeyEventType::KeyDown);
    let m = event.modifier_snapshot;
    // Alt なりすましの親指（`"Left Alt"`）は Alt が立ちうるので、`impersonated` なら除外しない（ADR-263 m6）。
    let alt = m.alt && !event.impersonated;
    let excluded = event.injected || m.ctrl || alt || m.win || is_ime_key(event);
    let role = match event.key_classification {
        KeyClassification::Char => event.physical_pos.map_or(KeyRole::Other, KeyRole::Char),
        KeyClassification::LeftThumb => KeyRole::Thumb(awase::engine::fsm_types::ThumbSide::Left),
        KeyClassification::RightThumb => KeyRole::Thumb(awase::engine::fsm_types::ThumbSide::Right),
        KeyClassification::Passthrough => {
            // 修飾キー自身と、語の区切りの Space（親指でないとき）は、エピソードに影響しない（保持はして順序を保つ）。
            // Space で打ち切ると、英文は最初の語（3〜5 文字）で終わり、`eisu_threshold` の確信に届かない。評価（Space を除いた
            // キー列）と同じ条件にそろえるため、Space は透明に扱う。
            if event.modifier_key.is_some() || event.vk_code == crate::vk::VK_SPACE {
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
    /// 前面窓（OS）と awase のフォーカス追跡が同じ窓を指しているか（プロセスと、ルート窓が分かっていればその hwnd）。
    ///
    /// Alt+Tab 直後の打鍵では、フォーカスの検出（WinEvent・500ms のリフレッシュ）が追いついておらず、
    /// `demoted_by_learning`・belief・`focus_kind` が**前の窓**の値のことがある。そのまま使うと、たとえば
    /// 一太郎から Chrome へ移った直後に、一太郎の値で Chrome の打鍵を保持し、Chrome へひらがなキーを送ってしまう。
    /// 同じプロセスの別の窓（一太郎の文書の窓、Chrome の窓間）の切替も見逃さないよう、hwnd も比べる。
    fn lang_detect_focus_in_sync(&self) -> bool {
        let foreground = crate::win32::foreground_scope();
        if !foreground.is_valid() || foreground.pid != self.platform.focus.pid() {
            return false;
        }
        let tracked_root = self.platform.focus.current.root_hwnd;
        tracked_root == 0 || tracked_root == foreground.hwnd.unsigned_abs()
    }

    /// `ime_on` の対象の窓か。既定は観測の無い窓（実測学習で降格された窓）だけ。`scope = all` なら読める窓も含める。
    /// 実質 TSF ネイティブの窓（プロファイルが `Imm32Unavailable` でも `is_effectively_tsf_native` が真の Windows Terminal 等。
    /// `FOCUS_RESYNC` が最初の打鍵を取り込み口で扱い、単発の `VK_IME_OFF` は BUG-124 型の「@」の条件になる）と、
    /// 入力中継ツールの窓は常に対象外。
    fn lang_detect_window_in_scope(&self) -> bool {
        let profile = self.platform.current_app_profile();
        if matches!(profile, AppImeProfile::InputRelay)
            || profile.is_effectively_tsf_native(self.platform.focus.class_name())
        {
            return false;
        }
        self.lang_detect.scope == AutoDetectScope::All
            || self.platform.focus.is_demoted_by_learning()
    }

    /// awase の Engine が、今の belief で活性か（IME ON で、入力モードがローマ字入力と見られる）。
    ///
    /// 保持して判定する前提は、IME が開いているだけでは足りない。入力モードが英数系の belief のとき Engine は非活性で、
    /// ひらがなキーを送ると IME はひらがなになるのに、保持した打鍵は素のキーのまま IME に入る（ローマ字が仮名に化ける）。
    fn lang_detect_engine_active(&self) -> bool {
        let ctx = self.build_ctx();
        self.engine.compute_active(&ctx)
    }

    /// 保持してよい窓・状態か（エピソードが**始まる**打鍵でだけ効く。進行中のエピソードの保持は変わらない）。
    ///
    /// 対象の窓で、Engine が活性、NonText でなく、IME の変換中でない（変換中は未確定文字列に打鍵が足されるので保持しない）。
    /// フォーカス追跡が前面窓に追いついていること。
    fn lang_detect_may_hold(&self) -> bool {
        self.lang_detect.is_hold_mode()
            && self.lang_detect.hold_config_ok
            && self.lang_detect_focus_in_sync()
            && self.lang_detect_engine_active()
            && self.lang_detect_window_in_scope()
            && self.platform_state.focus.focus_kind != FocusKind::NonText
            && !crate::tsf::observer::ime_composition_active_now()
            && self
                .lang_detect
                .app_allowed(&self.platform.focus.current.process_name)
    }

    /// ひらがなキー/IME OFF キーを送ってよいかの事実を集める（判断は `awase::lang_detect` の純粋関数）。
    /// `belief_open` は「awase の Engine が活性」（IME ON かつローマ字入力と見られる）。
    fn lang_detect_write_facts(&self, target_open: bool) -> WriteFacts {
        WriteFacts {
            belief_open: self.lang_detect_engine_active(),
            unobserved_window: self.lang_detect_window_in_scope(),
            non_text: self.platform_state.focus.focus_kind == FocusKind::NonText,
            focus_in_sync: self.lang_detect_focus_in_sync(),
            explicit_opposite_intent: self.platform_state.ime.explicit_intent()
                == Some(!target_open),
            modifier_held: crate::hook::ime_mode_key_injection_blocked_by_modifier(),
        }
    }

    /// 日本語と判定したとき、ひらがなキーを 1 回送る。送る前提を満たさなければ送らず、理由を返す。
    ///
    /// 前提は送信直前に確かめ直す（保持の間にフォーカス追跡や belief が変わりうる）。Alt/Win 中は送らない（Alt 中の合成かな
    /// キーは JIS かな入力への切替に化ける。BUG-008、`kp_restore_kana_from_half_width` と同じ判定）。利用者が明示的に
    /// IME OFF を選んでいるときも送らない（その意図を尊重する。ADR-263 D5 の授権の代わり）。Ctrl/Shift は送信口が一時解放する。
    ///
    /// 送った**あと**に、`on_ime_applied`（composition を cold に・MS-IME のモード FSM の未確認化）と GjiFsm の
    /// OFF→ON（`Reopen`）を行う。これを欠くと、無操作のエピソードで GjiFsm が warm のまま、開いた直後の最初の
    /// ローマ字を送ってリテラル化する（BUG-002 系）。`applied` は書き換えない（ADR-262 D3）。
    ///
    /// 限界: `send_ime_mode_key` は scan=0 で送る。GJI（mozc 系）は VK だけで判定するので効く見込みだが、MS-IME(TSF) は
    /// scan 無しのモードキーを処理しないという実機の記録がある（`kp_restore_kana_from_half_width` 付近のコメント）。
    /// scan 付きは実 IME が OFF の窓でかなロック等を汚すハザードがある（BUG-015 追補7）ため使わない。
    /// これは BUG-015 追補7（『実 IME が確実に ON でない限りモードキーを注入しない』）への、所有者判断（2026-10-17、利用者が設定で
    /// 選んだときだけ）による例外。Alt/Win を確かめ、scan 無しで送ることでハザードを避ける。
    fn lang_detect_write_hiragana(&mut self) -> &'static str {
        if !self.lang_detect.is_hold_mode() {
            return "none";
        }
        if let Some(reason) = hiragana_write_skip_reason(self.lang_detect_write_facts(true)) {
            return reason;
        }
        if !crate::ime::send_hiragana_key() {
            return "skipped-send-failed";
        }
        self.platform.on_ime_applied(true, ImeOpenOutcome::Applied);
        self.kp_reopen_gji_fsm(crate::state::gji_direct_mechanism::ReopenSource::LangDetect);
        "hiragana-sent"
    }

    /// 英数入力と判定したとき（`ime_on_eisu`）、**半角英数トグルに入る**（IME は ON のまま）。送れない前提なら理由を返す。
    ///
    /// 左Shift単独タップの半角英数トグル（ADR-107・ADR-245）と同じ入口・同じ状態を使う: GJI は `VK_DBE_ALPHANUMERIC`、
    /// MS-IME は IMC への conv 書き込み。入ったあとは `input_mode = ObservedEisu` の belief で Engine が非活性になり、
    /// 保持した打鍵は素通しで流れる。開閉の belief（`desired_open`）は動かさない。戻り方も同じ（左Shift の再タップ・
    /// 右Shift・IME のキー・窓を離れたときの戻り待ちと復帰時の復元）。日本語を英数と誤判定しても Shift タップで戻れる。
    ///
    /// 前提: 半角英数トグルの設定が entry を許すこと・Engine が活性・対象の窓・Alt/Win が押されていない・フォーカス追跡が
    /// 前面窓に追いついている・利用者が直近に明示的に IME ON を選んでいない（`eisu_write_skip_reason`）。
    fn lang_detect_enter_half_width_alnum(&mut self) -> &'static str {
        if self.lang_detect.mode != AutoDetectMode::ImeOnEisu {
            return "none";
        }
        if let Some(reason) = eisu_write_skip_reason(self.lang_detect_write_facts(false)) {
            return reason;
        }
        let uses_imc_conv_write = crate::tsf::observer::tsf_obs().active_ime_kind()
            == crate::tsf::observer::ActiveImeKind::MicrosoftIme;
        let entry_ime_ok = self.platform_state.ime.effective_open()
            && self.platform_state.ime.belief.is_japanese_ime()
            && self.engine.is_user_enabled();
        let pending_for_scope = self
            .platform_state
            .gate
            .half_width_alnum
            .find_return_pending(crate::win32::foreground_scope())
            .is_some();
        match self
            .platform_state
            .gate
            .half_width_alnum
            .plan_entry_for_language_detect(entry_ime_ok, uses_imc_conv_write, pending_for_scope)
        {
            HalfWidthAlnumEffect::EnterViaImcWrite => {
                self.kp_enter_half_width_alnum_imc(
                    crate::state::ConvMutationReason::LangDetectEnglish,
                    "打鍵列から英数と判定",
                );
                "half-width-alnum-entered"
            }
            HalfWidthAlnumEffect::EnterViaGjiSendInput => {
                if self.kp_enter_half_width_alnum_gji(false, "打鍵列から英数と判定") {
                    "half-width-alnum-entered"
                } else {
                    "skipped-send-failed"
                }
            }
            _ => "skipped-toggle-unavailable",
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
    ///
    /// 前面窓が保持の開始時と違えば、保持した打鍵は捨てる（マウスで窓を変えたあとに次の打鍵が来なかった場合）。
    pub(crate) fn lang_detect_on_timer(&mut self) {
        let scope = scope_id(crate::win32::foreground_scope());
        let step = self.lang_detect.controller.on_timer(scope);
        let _ = self.lang_detect_apply(step);
    }

    /// `HoldStep` を実行する: journal、IME への書き込み、保持した打鍵の再生、タイマー。
    fn lang_detect_apply(&mut self, step: HoldStep<RawKeyEvent>) -> LangDetectIntake {
        // 判定時点の belief（書き込みの**前**に読む）。
        let belief_before = self.platform_state.ime.effective_open();
        let action = if step.japanese {
            self.lang_detect_write_hiragana()
        } else if step.english {
            self.lang_detect_enter_half_width_alnum()
        } else {
            "none"
        };
        let discarded = u32::try_from(step.discarded).unwrap_or(u32::MAX);
        let step_japanese = step.japanese;
        let step_english = step.english;
        for outcome in step.outcomes {
            let (verdict, evidence) = verdict_labels(outcome.verdict);
            #[allow(clippy::cast_possible_truncation)] // ログ用。f32 の精度で足りる
            let llr = outcome.llr as f32;
            let japanese = matches!(outcome.verdict, Verdict::Japanese(_));
            let not_japanese = outcome.verdict == Verdict::NotJapanese;
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
                belief_open: Some(belief_before),
                // 日本語と判定したが保持していなかった（観測だけのエピソード）は `observe-only`。
                action: if japanese {
                    if step_japanese {
                        action
                    } else {
                        "observe-only"
                    }
                } else if not_japanese && step_english {
                    action
                } else {
                    "none"
                },
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
        // 英数と判定して半角英数トグルに入った場合は、`input_mode = ObservedEisu` で Engine が非活性なので、
        // 再生された打鍵は Engine を通らず、そのまま OS へ流れる（順序どおり）。
        if !step.release.is_empty() {
            crate::INPUT_DEFER.replay_later(step.release);
        }

        // 保持が続くなら、始まったときにだけ期限のタイマーを置く。保持が終わったなら、同じ打鍵で始まって確定した
        // 場合を含めて必ず外す（残すと、次の（保持しない）エピソードをタイマーが途中で閉じてしまう）。
        if step.holding {
            if step.started_holding {
                self.platform.timer.set(
                    crate::TIMER_LANG_HOLD,
                    Duration::from_millis(u64::from(self.lang_detect.max_ms)),
                );
            }
        } else {
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
        // 設定リロードで作り直す前に、保持中の打鍵を解放する（作り直しで黙って消えないように）。
        let scope = scope_id(crate::win32::foreground_scope());
        let step = self.lang_detect.controller.on_timer(scope);
        let _ = self.lang_detect_apply(step);
        self.platform.timer.kill(crate::TIMER_LANG_HOLD);
        self.lang_detect = LangDetectShadow::new(cfg, ngram, hold_config_ok);
    }
}
