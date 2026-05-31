//! フォーカス追跡ロジック（`Runtime` の `impl` 分割）。
//!
//! ウィンドウフォーカス変化の検出・分類・後処理を担う。
//! 親モジュール（`runtime/mod.rs`）のフィールドに `self.*` でアクセスできる。

use crate::focus::cache::DetectionSource;
use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;

use super::Runtime;

/// `apply_focus_probe_result` 内部で使うフォーカス分類結果。
pub(super) struct ClassifiedFocus {
    pub hwnd: HWND,
    pub process_id: u32,
    pub class_name: String,
    pub kind: FocusKind,
}

impl Runtime {
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
        {
            let hint = self.platform.injection_hint();
            let new_mode =
                crate::output::types::InjectionMode::from((hint, self.platform_state.app_kind));
            self.platform.update_injection_mode(new_mode);
        }
        if process_changed {
            self.on_focus_process_changed(&classified, prev_pid);
        } else if classified.kind == FocusKind::Undetermined {
            self.platform
                .focus
                .try_send_uia(crate::focus::uia::SendableHwnd(classified.hwnd));
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

        let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);

        // SAFETY: `learn_imm_capability_on_focus` は Win32 IMM API を呼ぶ unsafe fn。
        //         `hwnd` は `probe` から得た有効なウィンドウハンドルであり、
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        unsafe {
            imm_learning::learn_imm_capability_on_focus(
                &mut self.platform,
                hwnd,
                &class_name,
                new_app_kind,
            );
        }

        if self.platform_state.app_kind != new_app_kind {
            log::info!(
                "AppKind changed: {:?} → {:?} (class={class_name})",
                self.platform_state.app_kind,
                new_app_kind
            );
            self.platform_state.app_kind = new_app_kind;
        }

        // SAFETY: `resolve_focus_kind` は Win32 API で HWND を問い合わせる unsafe fn。
        //         `hwnd` と `process_id` はフォーカスプローブで確認済みの有効な値。
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        let resolution = unsafe {
            kind_classifier::resolve_focus_kind(&self.platform, process_id, &class_name, hwnd)
        };
        let kind = resolution.kind;
        let reason = resolution.reason;
        let overridden = resolution.overridden;

        if self.platform_state.focus_kind != kind {
            log::debug!(
                "Focus kind changed: {:?} → {kind:?} (reason={reason})",
                self.platform_state.focus_kind
            );
            self.platform_state.focus_kind = kind;
        }

        if !overridden {
            self.platform.focus.cache_insert(
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
        })
    }

    /// last_focus_info を更新し、(process_changed, prev_pid) を返す。
    ///
    /// process_changed な場合は事前に `hwnd_ime_cache.save()` を呼び出す。
    fn advance_focus_tracking(&mut self, classified: &ClassifiedFocus) -> (bool, Option<u32>) {
        let last_pid = if self.platform.focus.is_focused() {
            Some(self.platform.focus.pid())
        } else {
            None
        };
        let process_changed = last_pid.is_some_and(|last| last != classified.process_id);

        if process_changed {
            self.platform.focus.save_ime_state(
                self.platform_state.ime_on(),
                self.platform_state.input_mode(),
            );
        }

        self.platform
            .update_focus_info(classified.process_id, classified.class_name.clone());

        self.platform_state.set_prev_conversion_mode(None);

        (
            process_changed,
            if process_changed { last_pid } else { None },
        )
    }

    /// プロセス変更時の後処理（ログ・タイムスタンプ・output 通知・IME キャッシュ復元等）。
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

        self.platform_state.last_focus_change_ms = crate::hook::current_tick_ms();
        self.platform.notify_focus_changed();
        let new_profile = self.platform.current_app_profile();
        let new_hwnd = crate::state::ime_event::HwndId(classified.hwnd.0 as usize);
        self.platform_state
            .ime
            .dispatch_event(crate::state::ime_event::ImeEvent::FocusChanged {
                from: None,
                to: new_hwnd,
                profile: new_profile,
            });

        {
            let process_name = self.platform.focus.process_name().to_owned();
            self.platform_state.active_keymaps = self.all_keymaps.filter_active(&process_name);
            log::debug!(
                "[keymap] active rules updated: {} rule(s) for process={:?}",
                self.platform_state.active_keymaps.len(),
                process_name
            );
        }

        {
            let cache_hit = self.platform.focus.restore_ime_state();
            let cache_miss = cache_hit.is_none();
            self.platform_state.apply_hwnd_cache_restore(cache_hit);

            // TsfNative プロファイルへの cache miss 入場では、前ウィンドウの ime_on=false が
            // carry over したまま IMM/poll で復旧できず Engine が活性化不能になる。
            // stale を true へ寄せ直して trap を解く。
            // ただし直近 10 秒以内に明示的 IME OFF にしていた場合はユーザーの意図を尊重する。
            if cache_miss
                && matches!(
                    self.platform.current_app_profile(),
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

        if matches!(
            self.platform.current_app_profile(),
            crate::focus::classify::AppImeProfile::TsfNative,
        ) {
            let ime_on_now = self.platform_state.ime_on();
            if ime_on_now {
                self.platform_state.ime.mirror_applied_open(true);
                log::debug!(
                    "[focus] TsfNative hard pre-sync applied=true (prevent spurious VK_KANJI from SetOpen(true))"
                );
            } else {
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
            self.platform
                .focus
                .try_send_uia(crate::focus::uia::SendableHwnd(classified.hwnd));
        }
    }

    /// 現在のフォーカス先を検出し、focus_kind / app_kind を更新する。
    ///
    /// 前面プロセスが前回と異なる場合は `true` を返す（flush が必要）。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    pub(super) unsafe fn detect_and_update_focus(&mut self) -> bool {
        let probe = unsafe { crate::focus::probe::read_focus_snapshot() };
        self.apply_focus_probe_result(probe)
    }
}
