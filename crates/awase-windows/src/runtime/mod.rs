mod ime_refresh;
use ime_refresh::ImeRefreshPipeline;

use awase::engine::{Engine, EngineCommand, InputContext};
use awase::types::{ContextChange, FocusKind, RawKeyEvent, ShadowImeAction, VkCode};

use crate::focus::cache::DetectionSource;
use crate::win32::HwndExt as _;
use crate::ImeBelief;

pub use crate::focus::classifier::{AppKindClassifier, ImmCapability, InjectionHint};

/// `ImeBelief` と修飾キースナップショットから `InputContext` を構築する。
///
/// `modifiers` はフック時点でキャプチャした `ModifierState` を渡すこと。
/// タイマー等のイベント非同期パスでは呼び出し元が `read_os_modifiers()` で取得する。
///
/// Phase 3d: `ime_on` は呼び出し元が `platform_state.ime_on()` (shadow_model.effective_open) を
/// 評価して渡す。`belief` からは入力モード等の追加情報のみ取得する。
#[must_use]
pub const fn build_input_context(
    ime_on: bool,
    belief: &ImeBelief,
    modifiers: &awase::engine::ModifierState,
) -> InputContext {
    InputContext {
        ime_on,
        input_mode: belief.input_mode(),
        is_japanese_ime: belief.is_japanese_ime(),
        modifiers: *modifiers,
        left_thumb_down: None,
        right_thumb_down: None,
    }
}
use awase::yab::YabLayout;

use crate::executor::DecisionExecutor;
use crate::hook::CallbackResult;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[derive(Debug)]
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
}

/// アプリケーションランタイム。
///
/// Engine (判断) と DecisionExecutor (実行) を保持し、配線する。
/// OS イベントの受け取り → Observer → Engine → Executor のパイプラインを駆動する。
///
/// 注意: 判断ロジックを追加しないこと。判断は Engine が担う。
pub struct Runtime {
    pub engine: Engine,
    pub executor: DecisionExecutor,
    pub layouts: Vec<LayoutEntry>,
    /// IME 同期キー（イベント事前分類用）
    pub sync_toggle_keys: Vec<VkCode>,
    pub sync_on_keys: Vec<VkCode>,
    pub sync_off_keys: Vec<VkCode>,
    /// Platform 層の全状態
    pub platform_state: crate::PlatformState,
    /// 全キーマップルール（アプリフィルタ前）
    pub all_keymaps: crate::keymap::KeymapTable,
    /// Ctrl+無変換 IME-OFF 救済窓中に保留している event。
    ///
    /// `TIMER_IME_OFF_RESCUE` 満了で IME-OFF 発火、Ctrl↑ 到達で ctrl=false に書き換えて発火。
    /// `Some` 中に他のキーが到着したら救済中止して原 event を engine に渡す。
    pub pending_ime_off_rescue: Option<RawKeyEvent>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

/// `apply_focus_probe_result` 内部で使うフォーカス分類結果。
/// このファイル内のみで使用する。
struct ClassifiedFocus {
    hwnd: windows::Win32::Foundation::HWND,
    process_id: u32,
    class_name: String,
    kind: FocusKind,
    #[allow(dead_code)]
    app_kind: awase::types::AppKind,
}

impl Runtime {
    fn build_ctx(&self) -> InputContext {
        // SAFETY: `read_os_modifiers` は Win32 `GetKeyState` を呼ぶのみで副作用はない。
        //         メインスレッドから呼ばれるため、スレッド要件を満たしている。
        let modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
        build_input_context(
            self.platform_state.ime_on(),
            self.platform_state.belief(),
            &modifiers,
        )
    }

    /// output 層が注入モードを決定するために呼ぶ公開 API。
    ///
    /// focus の `injection_hint()` と `platform_state.focus.app_kind` を組み合わせて
    /// `InjectionHint` を返す。output 層はこのメソッドのみを呼び、
    /// focus/classify の内部型に直接アクセスしない。
    #[must_use]
    pub fn injection_hint(&self) -> (InjectionHint, awase::types::AppKind) {
        (
            self.executor.platform.focus.injection_hint(),
            self.platform_state.focus.app_kind,
        )
    }

    /// IME 関連の事前分類情報を sync key 設定で補完する
    pub fn enrich_ime_relevance(&self, event: &mut RawKeyEvent) {
        let vk = event.vk_code;
        let rel = &mut event.ime_relevance;

        if self.sync_toggle_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::Toggle);
            rel.may_change_ime = true;
        } else if self.sync_on_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOn);
            rel.may_change_ime = true;
        } else if self.sync_off_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOff);
            rel.may_change_ime = true;
        }
    }

    /// Decision の副作用を実行する（メッセージループ用）。
    pub fn execute_decision(&mut self, decision: awase::engine::Decision) -> CallbackResult {
        let result = self.executor.execute_from_loop(decision);
        self.flush_pending_apply_events();
        result
    }

    /// `DecisionExecutor` の sync path pending apply events を排出し、
    /// `shadow_model.pending` の generation に一致するもののみ
    /// `ImeApplySucceeded` / `ImeApplyFailed` を dispatch する。
    ///
    /// async path (ImmCross) は spawn_local 内で直接 dispatch するためここを経由しない。
    /// generation 照合は [[feedback_generation_check_for_async_apply]] 参照。
    pub fn flush_pending_apply_events(&mut self) {
        use crate::state::ime_event::ImeEvent;
        let events = self.executor.drain_pending_apply_events();
        for pending in events {
            let Some(generation) = self
                .platform_state
                .ime
                .shadow_model
                .pending
                .as_ref()
                .map(|p| p.generation)
            else {
                // pending がない経路 (toggle_engine / refresh_ime_state_cache /
                // process_deferred_keys 等) は ImeApplyRequested を出していないため
                // Succeeded/Failed も出さない。
                continue;
            };
            let event =
                ImeEvent::from_apply_outcome(pending.target, pending.outcome, generation);
            self.platform_state.ime.dispatch_event(event);
        }
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub fn toggle_engine(&mut self) {
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
        self.execute_decision(decision);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let ctx = self.build_ctx();
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason), &ctx);
        self.execute_decision(decision);
    }

    /// IME 状態とフォーカス状態を一括で再観測し、Engine に通知する。
    ///
    /// フォーカスデバウンス後・500ms ポーリング・may_change_ime 後など、
    /// 全ての IME/フォーカス更新がこのメソッドに集約される（ADR 028）。
    ///
    /// 処理フロー:
    /// 1. 現在のフォーカス先を取得・分類（focus_kind, app_kind 更新）
    /// 2. 前面プロセスが変わった場合は Engine に FocusChanged（flush あり）
    /// 3. IME 状態を再取得して Preconditions を更新
    /// 4. Engine に RefreshState（active 状態の遷移検知）
    /// 5. 次回ポーリングを自動スケジュール
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn refresh_ime_state_cache(&mut self) {
        ImeRefreshPipeline::new(self).run();
    }

    /// フォーカスプローブ結果を適用する（blocking なし、with_app 内で呼ぶ）。
    /// detect_and_update_focus の fetch 部分を除いた apply のみ。
    /// async drain 後に with_app 内で呼ぶ用途に使う。
    pub fn apply_focus_probe_result(
        &mut self,
        probe: Option<crate::focus::probe::FocusSnapshot>,
    ) -> bool {
        let Some(classified) = self.classify_focus_probe(probe) else {
            return false;
        };
        let (process_changed, prev_pid) = self.advance_focus_tracking(&classified);
        // injection_mode を push — advance_focus_tracking() で last_focus_info が更新された後に
        // 呼ぶことで injection_hint() が新ウィンドウ (WezTerm 等) を正しく参照できる。
        // classify_focus_probe() 内では last_focus_info が前ウィンドウのままであり、
        // ForceTsf を取得できず injection_mode=Vk になって send_eager_tsf_warmup() が
        // can_warmup()=false のまま失敗するバグを修正する。
        {
            let hint = self.executor.platform.focus.injection_hint();
            let new_mode = crate::output::types::InjectionMode::from(
                (hint, self.platform_state.focus.app_kind),
            );
            self.executor.platform.output.update_injection_mode(new_mode);
        }
        if process_changed {
            self.on_focus_process_changed(&classified, prev_pid);
        } else if classified.kind == FocusKind::Undetermined {
            if let Some(sender) = &self.executor.platform.focus.uia_sender {
                let _ = sender.send(crate::focus::uia::SendableHwnd(classified.hwnd));
            }
        }
        process_changed
    }

    /// プローブ結果を検証・分類し、platform_state (app_kind / focus_kind) を更新する。
    ///
    /// injection_mode の更新は `apply_focus_probe_result` が `advance_focus_tracking` 後に行う。
    /// None を返した場合は呼び出し元が early return すること。
    fn classify_focus_probe(
        &mut self,
        probe: Option<crate::focus::probe::FocusSnapshot>,
    ) -> Option<ClassifiedFocus> {
        use crate::focus::imm_learning;
        use crate::focus::kind_classifier;

        let Some(probe) = probe else {
            log::warn!("Focus probe timed out — skipping update this cycle");
            return None;
        };
        if probe.process_id == 0 {
            return None;
        }

        let hwnd = probe.hwnd();
        let process_id = probe.process_id;
        let class_name = probe.class_name;

        // app_kind を更新
        let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);

        // ── Phase 2: IMM 能力キャッシュの初期学習 ──
        // SAFETY: `learn_imm_capability_on_focus` は Win32 IMM API を呼ぶ unsafe fn。
        //         `hwnd` は `probe` から得た有効なウィンドウハンドルであり、
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        unsafe {
            imm_learning::learn_imm_capability_on_focus(
                &mut self.executor.platform.focus,
                hwnd,
                &class_name,
                new_app_kind,
            );
        }

        if self.platform_state.focus.app_kind != new_app_kind {
            log::info!(
                "AppKind changed: {:?} → {:?} (class={class_name})",
                self.platform_state.focus.app_kind,
                new_app_kind
            );
            self.platform_state.focus.app_kind = new_app_kind;
        }

        // ── Phase 3: focus_kind を決定 ──
        // SAFETY: `resolve_focus_kind` は Win32 API で HWND を問い合わせる unsafe fn。
        //         `hwnd` と `process_id` はフォーカスプローブで確認済みの有効な値。
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        let resolution = unsafe {
            kind_classifier::resolve_focus_kind(
                &self.executor.platform.focus,
                &self.executor,
                process_id,
                &class_name,
                hwnd,
            )
        };
        let kind = resolution.kind;
        let reason = resolution.reason;
        let overridden = resolution.overridden;

        // focus_kind を更新
        if self.platform_state.focus.focus_kind != kind {
            log::debug!(
                "Focus kind changed: {:?} → {kind:?} (reason={reason})",
                self.platform_state.focus.focus_kind
            );
            self.platform_state.focus.focus_kind = kind;
        }

        // キャッシュ格納（オーバーライドでない場合のみ）
        if !overridden {
            self.executor.platform.focus.cache.insert(
                process_id,
                class_name.clone(),
                kind,
                DetectionSource::Automatic,
            );
        }

        Some(ClassifiedFocus {
            hwnd,
            process_id,
            class_name,
            kind,
            app_kind: new_app_kind,
        })
    }

    /// last_focus_info を更新し、(process_changed, prev_pid) を返す。
    ///
    /// process_changed な場合は事前に `hwnd_ime_cache.save()` を呼び出す。
    /// prev_pid は process_changed 時のみ Some になる（ログ用）。
    fn advance_focus_tracking(&mut self, classified: &ClassifiedFocus) -> (bool, Option<u32>) {
        let last_pid = self
            .executor
            .platform
            .focus
            .last_focus_info
            .as_ref()
            .map(|(pid, _)| *pid);
        let process_changed = last_pid.is_some_and(|last| last != classified.process_id);

        // フォーカス離脱: 現在の belief を per-HWND キャッシュに保存
        if process_changed {
            if let Some((old_pid, old_class)) =
                self.executor.platform.focus.last_focus_info.clone()
            {
                self.executor.platform.focus.hwnd_ime_cache.save(
                    old_pid,
                    old_class,
                    self.platform_state.ime_on(),
                    self.platform_state.input_mode(),
                );
            }
        }

        // last_focus_info と AppImeProfile キャッシュをアトミックに更新（IMM 制御の SSOT）。
        self.executor
            .platform
            .focus
            .update_focus_info(classified.process_id, classified.class_name.clone());

        // prev_conversion_mode をリセット
        self.platform_state.set_prev_conversion_mode(None);

        (process_changed, if process_changed { last_pid } else { None })
    }

    /// プロセス変更時の後処理（ログ・タイムスタンプ・output 通知・ime_observations
    /// クリア・hwnd_cache 復元・force_on_guard リセット・UIA フォールバック）。
    /// `prev_pid` は `advance_focus_tracking` が返した直前のプロセス ID（ログ用）。
    fn on_focus_process_changed(&mut self, classified: &ClassifiedFocus, prev_pid: Option<u32>) {
        log::info!(
            "FocusChange [{}→{}] {}: stale ime_on={} intent={:?} mode={:?} japanese={}",
            prev_pid.map_or_else(|| "?".to_string(), |p| p.to_string()),
            classified.process_id,
            classified.class_name,
            self.platform_state.ime_on(),
            self.platform_state.explicit_intent(),
            self.platform_state.input_mode(),
            self.platform_state.is_japanese_ime(),
        );

        self.platform_state.focus.last_focus_change_ms = crate::hook::current_tick_ms();
        self.executor.platform.output.on_focus_changed();
        // Step 1.5: FocusChanged event を dispatch して shadow_model の AppImePolicy を更新する。
        // 順序: policy 確定 → observation clear → 以降の observation は新 policy で評価される。
        let new_profile = self.executor.platform.focus.current_app_profile();
        let new_hwnd = crate::state::ime_event::HwndId(classified.hwnd.0 as usize);
        self.platform_state
            .ime
            .dispatch_event(crate::state::ime_event::ImeEvent::FocusChanged {
                from: None, // 旧 hwnd は別途追跡可能だが Step 1.5 では None
                to: new_hwnd,
                profile: new_profile,
            });
        self.platform_state.clear_ime_observations_on_focus_change();

        {
            let process_name = &self.executor.platform.focus.current_process_name;
            self.platform_state.active_keymaps =
                self.all_keymaps.filter_active(process_name);
            log::debug!("[keymap] active rules updated: {} rule(s) for process={:?}",
                self.platform_state.active_keymaps.len(), process_name);
        }

        {
            let cache_hit = self.executor.platform.focus.hwnd_ime_cache.restore(
                classified.process_id,
                &classified.class_name,
            );
            let cache_miss = cache_hit.is_none();
            self.platform_state.apply_hwnd_cache_restore(cache_hit);

            // TsfNative プロファイル（Windows Terminal 等）への cache miss 入場では、
            // 前ウィンドウの ime_on=false が carry over したまま IMM/poll で復旧できず
            // Engine が活性化不能になる。stale を true へ寄せ直して trap を解く。
            //
            // ただし直近 10 秒以内に物理 IME キー / sync キーで明示的に IME OFF にしていた
            // 場合は、ユーザーの意図を尊重してリセットをスキップする。
            // Edge 等で Chrome_Widget → TsfNative サブウィンドウへフォーカスが移った際に
            // stale reset が IME ON へ戻してしまいフォーム送信が起きるバグを防ぐ。
            if cache_miss
                && matches!(
                    self.executor.platform.focus.current_app_profile(),
                    crate::focus::classify::AppImeProfile::TsfNative,
                )
            {
                let now_ms = crate::hook::current_tick_ms();
                let last_off_ms = self.platform_state.last_explicit_ime_off_ms;
                let elapsed = now_ms.saturating_sub(last_off_ms);
                if last_off_ms > 0 && elapsed < 10_000 {
                    log::debug!(
                        "[focus] TsfNative cache-miss: skip reset_stale — explicit IME-OFF {}ms ago",
                        elapsed
                    );
                } else {
                    self.platform_state.reset_stale_ime_on_for_tsf_native();
                }
            }
        }

        // TsfNative アプリ（LINE / Windows Terminal 等）では直後に呼ばれる
        // notify_focus_changed でエンジンが SetOpen(true, EngineIntent) を発行する。
        // on_focus_changed() がラッチを無効化（false）したままだと、
        // dispatch_ime_set_open の override-latch 判定が spurious VK_KANJI を LINE に
        // 送信してしまい IME がトグルされる（フォーム送信誤動作等）。
        //
        // desired=true (IME ON): hard pre-sync (applied_ms=now) で SetOpen(true) を抑止。
        //   300ms 後に override が再度可能になり、Ctrl+変換 で再試行できる。
        // desired=false (IME OFF): soft pre-sync (applied_ms=0 維持) で、
        //   フォーカス先の IME が ON だった場合に初回 Ctrl+無変換 で確実に VK_KANJI を送れる。
        //   実 apply 後は applied_ms > 0 となり「確認済み OFF」として永続スキップ。
        if matches!(
            self.executor.platform.focus.current_app_profile(),
            crate::focus::classify::AppImeProfile::TsfNative,
        ) {
            let ime_on_now = self.platform_state.ime_on();
            if ime_on_now {
                self.platform_state.ime.mirror_applied_open(true);
                log::debug!(
                    "[focus] TsfNative hard pre-sync applied=true (prevent spurious VK_KANJI from SetOpen(true))"
                );
            } else {
                // soft presync: applied_open = None のまま（applied_at_ms = 0 = 不確定）
                // → 初回 Ctrl+無変換 で latch 強制が発火できるようにする
                log::debug!(
                    "[focus] TsfNative soft pre-sync: applied_open=None (allow override on first Ctrl+無変換)"
                );
            }
        }

        if self.platform_state.is_force_on_guard_active()
            || self.platform_state.ime_detect_miss_count() > 0
        {
            log::debug!(
                "Focus changed: clearing force_on_guard and detect_miss_count \
                 (new window may have different IME state)"
            );
            self.platform_state.reset_ime_detect_state();
        }

        if classified.kind == FocusKind::Undetermined {
            if let Some(sender) = &self.executor.platform.focus.uia_sender {
                let _ = sender.send(crate::focus::uia::SendableHwnd(classified.hwnd));
            }
        }
    }

    /// 現在のフォーカス先を検出し、focus_kind / app_kind を更新する。
    ///
    /// 前面プロセスが前回と異なる場合は `true` を返す（flush が必要）。
    /// 同一プロセス内のフォーカス移動では `false` を返す（flush 不要）。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    unsafe fn detect_and_update_focus(&mut self) -> bool {
        // フォーカス検出全体をワーカースレッドでタイムアウト付き実行する。
        // 詳細は focus::probe::read_focus_snapshot() を参照。
        // SAFETY: `read_focus_snapshot` は Win32 `GetForegroundWindow` 等を呼ぶ unsafe fn。
        //         この関数自体が unsafe として宣言されており、メインスレッド呼び出しが前提。
        let probe = unsafe { crate::focus::probe::read_focus_snapshot() };
        self.apply_focus_probe_result(probe)
    }

    /// IME リフレッシュを非同期タスクとしてスポーン。
    /// with_app の外でフェッチを行い、完了後に with_app で適用する。
    pub fn spawn_ime_refresh(&mut self) {
        self.executor.platform.timer.kill(crate::TIMER_IME_REFRESH);

        // NOTE: ここで send_eager_tsf_warmup() を呼ばない。
        // focus_transition_pending=true の時点では injection_mode が前ウィンドウ（WezTerm 等）
        // の stale な Tsf のままであり、新しいウィンドウが Chrome/Edge の場合に誤って
        // VK_DBE_HIRAGANA を送信して Chrome の IME を ON にしてしまうバグがあった。
        // eager warmup は post_focus_change_snapshot (run_with_prefetched 内) で injection_mode
        // 確定後に正しく送信される。

        win32_async::spawn_local(async {
            let focus = crate::focus::probe::run_focus_probe_async().await;
            let snap = crate::ime::read_ime_state_full_async().await;
            let _ = crate::with_app(|app| {
                ImeRefreshPipeline::new(app).run_with_prefetched(focus, &snap);

                // run_with_prefetched 完了後: last_focus_info が更新済みのため
                // injection_hint を読んで TsfGate を遷移させる。
                // PendingWarmup 以外（Probing/Ready/Bypass）なら空 Vec が返る。
                let is_tsf = matches!(
                    app.executor.platform.focus.injection_hint(),
                    InjectionHint::ForceTsf
                );
                let held = if is_tsf {
                    app.executor.platform.output.confirm_tsf()
                } else {
                    // BypassConfirmed（非TSFウィンドウ確定）: warmup_grace を無視して
                    // ime_on を false に強制する。
                    //
                    // apply_focus_probe が WARMUP_GRACE_MS(300ms) の抑制で ime_on=true を
                    // 保持したまま bypass_tsf() に到達した場合、以降のキーが NICOLA 変換
                    // されてしまい Win+X メニュー等の1文字ショートカットが届かなくなる。
                    // 非TSFウィンドウには日本語IMEが存在しないため grace 抑制は不要であり、
                    // ここで ime_on=false を確定させる。
                    let ms = crate::hook::current_tick_ms();
                    let user_enabled = app.engine.is_user_enabled();
                    app.platform_state.write_focus_probe(false, ms, user_enabled);
                    app.executor.platform.output.bypass_tsf()
                };
                // confirm_tsf は PendingWarmup/Bypass → Probing、bypass_tsf は PendingWarmup/Probing → Bypass。
                // Bypass から confirm_tsf を呼ぶのは WarmupTimeout 後に async タスクが遅れて完了した回復パス。
                // すでに Probing/Ready なら空 Vec が返るため、500ms ポーリング等での再呼び出しも safe。
                app.executor.platform.timer.kill(crate::TIMER_TSF_GATE);
                if !held.is_empty() {
                    log::debug!("[tsf-gate] draining {} held keys via INPUT_DEFER", held.len());
                    crate::INPUT_DEFER.replay_later(held);
                }
            });
        });
    }

    /// 統合 IME リフレッシュタイマーをスケジュール（リセット）する。
    ///
    /// 既存のタイマーをキャンセルして `delay_ms` 後に再設定する。
    /// フォーカス変更(50ms) / ポーリング(500ms) / 即時(0ms) を統一的に扱う。
    pub fn schedule_ime_refresh(&mut self, delay_ms: u64) {
        self.executor.platform.timer.set(
            crate::TIMER_IME_REFRESH,
            std::time::Duration::from_millis(delay_ms),
        );
    }

    /// 配列を動的に切り替える
    pub fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self
            .engine
            .on_command(EngineCommand::SwapLayout(entry.layout.clone()), &self.build_ctx());
        self.execute_decision(decision);

        self.executor.platform.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// 手動アプリオーバーライドのトグル処理
    pub fn toggle_app_override(&mut self) {
        let current = self.platform_state.focus.focus_kind;
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        self.platform_state.focus.focus_kind = new_kind;

        // Update learning cache
        if let Some((pid, cls)) = self.executor.platform.focus.last_focus_info.as_ref() {
            self.executor.platform.focus.cache.insert(
                *pid,
                cls.clone(),
                new_kind,
                DetectionSource::UserOverride,
            );
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        // バルーン通知を表示
        self.executor.platform.tray.show_balloon(
            "awase",
            if new_kind == FocusKind::TextInput {
                "テキスト入力モードに切り替えました"
            } else {
                "バイパスモードに切り替えました"
            },
        );

        let mode_str = if new_kind == FocusKind::TextInput {
            "TextInput (engine enabled)"
        } else {
            "NonText (engine bypassed)"
        };
        log::info!("Manual focus override: → {mode_str}");
    }

    /// Sync key 後に遅延されたキーを再処理する。
    ///
    /// sync key で guard が起動された後、KeyUp で OS が IME を切り替えてから呼ばれる。
    /// guard 解除 → IME 状態 refresh → Engine 通知 → バッファキー再処理。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn process_deferred_keys(&mut self) {
        // Guard を解除し、保留キーを回収
        let keys = self.platform_state.sync_key_gate.deactivate();
        log::debug!("IME guard OFF (process_deferred_keys)");

        // Refresh IME state (Observer → ImeObservations → Preconditions)
        // SAFETY: `poll_and_classify_ime` は Win32 IMM API（`ImmGetContext` 等）を呼ぶ unsafe fn。
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        let observer_out = unsafe {
            crate::observer::ime_observer::poll_and_classify_ime(
                self.platform_state.ime_on(),
                self.platform_state.is_force_on_guard_active(),
                self.platform_state.input_mode(),
                self.platform_state.prev_conversion_mode(),
            )
        };
        self.platform_state.apply_ime_update(&observer_out, self.engine.is_user_enabled());

        // LastAppliedImeState を OS 観測値に同期する。
        // 物理 Kanji キー（sync key）は apply_ime_open を経由しないため last_applied が更新されない。
        // last_applied が stale なまま Engine が activate → SetOpen(true) → KanjiToggleStrategy が
        // last_applied(false) != desired(true) と判定して VK_KANJI を余分に送信し、
        // Chrome では IME が逆転するバグを防ぐ。
        let observed_ime_on = self.platform_state.ime_on();
        self.platform_state.ime.mirror_applied_open(observed_ime_on);
        log::debug!("[process-deferred] applied_open → {observed_ime_on} (sync with OS poll)");

        // Engine に IME 状態変化を即通知する（deferred keys の有無にかかわらず）。
        // suppress_engine_state_key = true: sync key（Kanji 等）がすでに IME を正しい状態に
        // 設定しているため、engine_on/off_ime_key（VK_DBE_DBCSCHAR 等）を追加送信しない。
        // 送ると IME モードが ひらがな→全角英数 等に意図せず変わる可能性がある。
        {
            let ctx = self.build_ctx();
            let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
            self.executor.platform.suppress_engine_state_key = true;
            self.execute_decision(decision);
            self.executor.platform.suppress_engine_state_key = false;
        }

        if keys.is_empty() {
            return;
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        for (event, _phys) in keys {
            // Build fresh context with updated preconditions
            let ctx = self.build_ctx();
            let decision = self.engine.on_input(event, &ctx);
            self.execute_decision(decision);
        }
    }

    /// パニックリセット: IME 関連キー連打で発動する緊急リセット。
    ///
    /// エンジン状態・IME・修飾キー・フック・キャッシュをすべて初期状態に戻す。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn panic_reset(&mut self) {
        log::warn!("Panic reset triggered!");

        // 1. エンジンの保留状態をフラッシュ
        self.invalidate_engine_context(ContextChange::InputLanguageChanged);

        // 2. IME 未確定文字列をキャンセル → OFF → ON
        // SAFETY: `cancel_ime_composition` は Win32 IMM API を呼ぶ unsafe fn。
        //         `panic_reset` はメッセージループ上（メインスレッド）から呼ばれるため安全。
        unsafe { cancel_ime_composition() };
        // OFF → ON を順序保証付きで実行する。`WindowsPlatform::set_ime_open` は
        // 内部で spawn_local して fire-and-forget するため、2 連発で呼ぶと async race で
        // 順序が逆転しうる (true→false の終端で IME OFF のまま残るリスク)。単一の
        // spawn_local タスク内で 2 回 await する形にして OFF → ON を直列化する。
        if self
            .executor
            .platform
            .focus
            .current_app_profile()
            .can_use_imm32_cross_process()
        {
            win32_async::spawn_local(async {
                let _ = crate::ime::set_ime_open_cross_process_async(false).await;
                let _ = crate::ime::set_ime_open_cross_process_async(true).await;
            });
        }

        // 3. 全修飾キーの KeyUp を送信（スタック解消）
        send_all_modifier_key_ups();

        // 4. PlatformState を全面リセット
        // panic_reset 直後に refresh_ime_state_cache() が走ると、ここで書いた
        // ime_on=true を stale な observe() 結果が即座に上書きしてしまう。
        // force_on_guard で 1 サイクルだけ保護し、次の検出成功時に自然に解除する。
        self.platform_state.apply_panic_reset();
        // Step 4: chord barrier も clear (旧 ctrl_bypass_hold 相当)
        self.platform_state.ime.shadow_model.input_barrier = None;
        self.platform_state.sync_key_gate.clear();

        // 6. IME 状態を再取得
        self.refresh_ime_state_cache();

        // 7. バルーン通知
        self.executor
            .platform
            .tray
            .show_balloon("awase", "状態をリセットしました");
    }
}

/// 全修飾キーの KeyUp を `SendInput` で送信する。
///
/// Shift, Ctrl, Alt, Win の左右それぞれに対して KeyUp を送り、
/// スタックした修飾キー状態を解消する。
fn send_all_modifier_key_ups() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    // VK_SHIFT(0x10), VK_CONTROL(0x11), VK_MENU(0x12),
    // VK_LWIN(0x5B), VK_RWIN(0x5C),
    // VK_LSHIFT(0xA0), VK_RSHIFT(0xA1),
    // VK_LCONTROL(0xA2), VK_RCONTROL(0xA3),
    // VK_LMENU(0xA4), VK_RMENU(0xA5)
    use crate::vk::{VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU};
    const MODIFIER_VKS: [VkCode; 11] = [
        VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN,
        VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU,
    ];

    let inputs: Vec<INPUT> = MODIFIER_VKS
        .iter()
        .map(|&vk| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk.0),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: crate::output::INJECTED_MARKER,
                },
            },
        })
        .collect();

    // OutputActiveGuard: SendInput 実行中にユーザーキーが届いた場合、
    // フックが RUNTIME 借用中（panic_reset の with_app 内）で再入しないよう
    // OUTPUT_GATE.active=true で INPUT_DEFER に退避する。
    let _guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
    let _ = crate::win32::send_input_safe(&inputs);
    log::debug!("Sent KeyUp for all modifier keys");
}

/// IME の未確定文字列をキャンセルする。
///
/// # Safety
/// Win32 IMM API (`ImmGetContext`, `ImmNotifyIME`, `ImmReleaseContext`) を呼び出す。
/// メインスレッドから呼ぶこと。
unsafe fn cancel_ime_composition() {
    use windows::Win32::UI::Input::Ime::{ImmNotifyIME, NOTIFY_IME_ACTION, NOTIFY_IME_INDEX};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    // SAFETY: `GetForegroundWindow` は呼び出し時点のフォアグラウンドウィンドウを返す安全なクエリ。
    //         NULL の場合は `non_null()` が None を返すため early return する。
    let Some(hwnd) = GetForegroundWindow().non_null() else {
        return;
    };
    // SAFETY: `hwnd` は直上で NULL でないことを確認済み。
    //         `ImmContextGuard` は RAII で `ImmReleaseContext` を呼ぶため、
    //         コンテキストリークは発生しない。
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        return;
    };
    // NI_COMPOSITIONSTR = 0x15, CPS_CANCEL = 0x04
    // SAFETY: `ctx.himc()` は `ImmContextGuard` が保持する有効な HIMC。
    //         `NI_COMPOSITIONSTR`/`CPS_CANCEL` は未確定文字列キャンセルの標準的な呼び出し。
    let _ = unsafe { ImmNotifyIME(ctx.himc(), NOTIFY_IME_ACTION(0x15), NOTIFY_IME_INDEX(0x04), 0) };
    log::debug!("Cancelled IME composition");
}

