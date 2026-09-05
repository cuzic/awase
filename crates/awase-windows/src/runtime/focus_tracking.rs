#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! フォーカス追跡ロジック（`Runtime` の `impl` 分割）。
//!
//! ウィンドウフォーカス変化の検出・分類・後処理を担う。
//! 親モジュール（`runtime/mod.rs`）のフィールドに `self.*` でアクセスできる。

use crate::focus::cache::DetectionSource;
use crate::focus::current::FocusIdentity;
use crate::focus::FocusKind;
use windows::Win32::Foundation::HWND;

use super::Runtime;
use win32_async;

const EXPLICIT_OFF_CACHE_SUPPRESS_MS: u64 = 10_000;

/// `apply_focus_probe_result` 内部で使うフォーカス分類結果。
pub(super) struct ClassifiedFocus {
    pub hwnd: HWND,
    pub process_id: u32,
    pub process_name: Option<String>,
    pub class_name: String,
    pub kind: FocusKind,
}

impl Runtime {
    fn focus_identity_snapshot(&self) -> FocusIdentity {
        FocusIdentity {
            hwnd: self.platform.focus.current.hwnd,
            pid: self.platform.focus.current.pid,
            class_name: self.platform.focus.current.class_name.clone(),
            process_name: self.platform.focus.current.process_name.clone(),
            app_profile: self.platform.focus.current.app_profile,
            app_kind: self.platform_state.focus.app_kind,
            focus_kind: self.platform_state.focus.focus_kind,
        }
    }

    fn record_focus_transition_if_changed(
        &mut self,
        prev: &FocusIdentity,
        next: &FocusIdentity,
        prev_started_ms: u64,
    ) {
        if next.hwnd == 0 {
            return;
        }
        let changed = prev.changed_axes(next);
        if !changed.any() {
            return;
        }
        let now_ms = crate::hook::current_tick_ms();
        let dwell_ms = if prev.hwnd == 0 {
            0
        } else {
            now_ms.saturating_sub(prev_started_ms)
        };
        self.platform_state.focus.last_focus_transition_ms = now_ms;
        let profile = crate::state::ime_event::ImePolicyProfile::from(next.app_profile);
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::FocusTransition {
                changed,
                from: (prev.hwnd != 0).then(|| focus_endpoint(prev)),
                to: focus_endpoint(next),
                dwell_ms,
                profile: format!("{profile:?}"),
            });
    }

    /// フォーカスプローブ結果を適用する（blocking なし、with_app 内で呼ぶ）。
    /// detect_and_update_focus の fetch 部分を除いた apply のみ。
    /// async drain 後に with_app 内で呼ぶ用途に使う。
    pub fn apply_focus_probe_result(
        &mut self,
        probe: Option<crate::focus::probe::FocusSnapshot>,
    ) -> bool {
        let prev = self.focus_identity_snapshot();
        let prev_started_ms = self.platform_state.focus.last_focus_transition_ms;
        let Some(classified) = self.classify_focus_probe(probe) else {
            return false;
        };
        let (process_changed, prev_pid) = self.advance_focus_tracking(&classified, false);
        let next = self.focus_identity_snapshot();
        self.record_focus_transition_if_changed(&prev, &next, prev_started_ms);
        // injection_mode を push — advance_focus_tracking() で last_focus_info が更新された後に
        // 呼ぶことで injection_hint() が新ウィンドウ (WezTerm 等) を正しく参照できる。
        {
            let hint = self.platform.injection_hint();
            let new_mode = crate::output::types::InjectionMode::from((
                hint,
                self.platform_state.focus.app_kind,
            ));
            self.platform.update_injection_mode(new_mode);
        }
        if process_changed {
            self.on_focus_process_changed(&classified, prev_pid, &prev);
        } else if classified.kind == FocusKind::Undetermined {
            self.platform
                .focus
                .try_send_uia(crate::focus::uia::SendableHwnd(classified.hwnd));
        }
        process_changed
    }

    pub(crate) fn establish_initial_focus_scope(&mut self) {
        let prev = self.focus_identity_snapshot();
        let prev_started_ms = self.platform_state.focus.last_focus_transition_ms;
        let probe = unsafe { crate::focus::probe::read_focus_snapshot() };
        let Some(classified) = self.classify_focus_probe(probe) else {
            return;
        };
        // `is_bootstrap=true`: これが最初のフォーカス確立であり、まだ一度も IME を
        // 観測していない。`apply_app_disable_transition` の `invalidate_engine_context`
        // 呼び出し（Enter エッジのみ）は engine の pending 状態を安全にflushする決定実行
        // であり、bootstrap時点では engine に何もpendingが無いため意味を持たない一方、
        // ADR-102 決定3-b が「最初のIME観測より前にbeliefを書き換えない」ことを
        // 構造的に保証しようとしている以上、この経路自体をbootstrapでは通さない
        // （`apply_app_disable_transition` 側で `is_bootstrap` を見てskipする）。
        let _ = self.advance_focus_tracking(&classified, true);
        let next = self.focus_identity_snapshot();
        self.record_focus_transition_if_changed(&prev, &next, prev_started_ms);

        let tick_ms = self.enter_focus_scope(&classified);
        // BUG-102: `enter_focus_scope` の直後（epoch インクリメント済み・
        // `update_focus_info` 済み）に、live 側フェンスを `ObservationStore` 側へ
        // 同期する。この 1 行が無いと、起動時にフォーカスされていたアプリの
        // `ImmCrossProbe`（High）観測が次のプロセス変更まで `derive_*` から
        // 外れ続ける。
        //
        // 上の early return（`classify_focus_probe` が `None`、= probe タイム
        // アウトや pid 取得失敗）を通った場合はここまで来ないため同期も走らないが、
        // その場合は `enter_focus_scope` も走っておらず live 側 epoch も 0 のまま
        // なので、両側は既定値で一致したままになる（BUG-102 の desync は起きない）。
        self.sync_initial_focus_fence(tick_ms);
        // BUG-114 根本原因1（ADR-134 D1c）: `advance_focus_tracking` 済み
        // （`self.platform.focus.current.app_profile` 確定済み）の**後**に
        // 呼ぶこと。これより前だと `current_app_profile()` がまだ正しい
        // 値を返さない。
        self.sync_initial_app_policy(tick_ms);

        // injection_mode の再計算は呼び出し元に残す（指摘9: `on_focus_process_changed`
        // とは呼び出し順序が異なるため `enter_focus_scope` には含めない）。
        let hint = self.platform.injection_hint();
        let new_mode =
            crate::output::types::InjectionMode::from((hint, self.platform_state.focus.app_kind));
        self.platform.update_injection_mode(new_mode);
    }

    /// フォーカス確定処理の共通シーケンス（`last_focus_change_ms`/`focus_epoch`
    /// 更新 + `notify_focus_changed()` + `active_keymaps` フィルタ）を1箇所に
    /// まとめる（コードレビュー指摘9）。
    ///
    /// `establish_initial_focus_scope`（bootstrap、まだ一度も IME を観測していない）
    /// と `on_focus_process_changed`（定常経路）が、この処理列を独立に手動コピー
    /// していた。
    ///
    /// **`injection_mode` の再計算はここに含めない**（各呼び出し元に残す）。
    /// 2箇所で呼び出し順序が異なる（`establish_initial_focus_scope` はこの直後、
    /// `on_focus_process_changed` は IME belief dispatch・cache restore の後）ため、
    /// 統合すると挙動が変わるリスクがある。
    ///
    /// 戻り値の `TickMs` は、呼び出し元がこの後の処理（`dispatch_event` 等）で
    /// 同じ時刻を使い回すために返す。
    fn enter_focus_scope(&mut self, classified: &ClassifiedFocus) -> crate::state::TickMs {
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state.focus.last_focus_change_ms = tick_ms.0;
        // フォーカスエポックをインクリメント。このフォーカスで spawn された probe が
        // 次のフォーカス変更後に完了しても、epoch 不一致で棄却される。
        self.platform_state.focus.focus_epoch =
            self.platform_state.focus.focus_epoch.wrapping_add(1);
        self.platform.notify_focus_changed();

        self.recompute_active_keymaps();
        log::debug!(
            "[keymap] active rules updated on focus change: {} rule(s) \
             (hwnd={:?} kind={:?} focus_epoch={})",
            self.platform_state.keymap.active_keymaps.len(),
            classified.hwnd,
            classified.kind,
            self.platform_state.focus.focus_epoch,
        );
        tick_ms
    }

    /// bootstrap で確立した最初のフォーカススコープの同一性（epoch + hwnd）を
    /// `ObservationStore::current_fence` へ同期する（BUG-102）。
    ///
    /// **必ず `enter_focus_scope`（epoch インクリメント）と `advance_focus_tracking`
    /// （`update_focus_info` による hwnd 更新）の後に呼ぶこと** ——
    /// `focus_fence()` が live 側の確定値を返している必要がある。
    ///
    /// `notify_focus_hwnd_updated_if_needed` と同じ理由で独立した関数として切り出して
    /// いる: `dispatch_event(` を直接テキストとして含む関数は
    /// `establish_initial_focus_scope_does_not_write_ime_belief`
    /// （`architecture_guard.rs`）の対象リストに直接載っているため、
    /// `establish_initial_focus_scope` の本体に置くと静的テキスト検査で機械的に落ちる。
    ///
    /// **このイベントが belief を書かないこと**（ADR-102 決定3-b の不変条件）は、
    /// 運ぶ値が「観測の新鮮さを判定するための識別子」だけであることと、reducer 側の
    /// アームが `ObservationStore::establish_initial_fence()` しか呼ばないことの
    /// 2点で担保する。後者は `initial_focus_fence_event_only_touches_the_fence`
    /// （`architecture_guard.rs`）と
    /// `state::ime_model::tests::initial_focus_fence_established_touches_only_the_fence`
    /// が固定する。
    ///
    /// **bootstrap で1度しか呼ばれない**（唯一の呼び出し元
    /// `establish_initial_focus_scope` 自体が `app/bootstrap.rs::run_all` から
    /// 1度だけ呼ばれる）ことは、静的には
    /// `initial_focus_fence_event_only_touches_the_fence` が、実行時には
    /// `ObservationStore::establish_initial_fence()` 側の `debug_assert!` が
    /// 固定する。2度目以降の呼び出しは「initial」ではなく、観測プールを持った
    /// まま fence だけ差し替える危険な操作になる（その用途は
    /// `clear_on_focus_change()` が担当する）。
    fn sync_initial_focus_fence(&mut self, tick_ms: crate::state::TickMs) {
        let fence = self.focus_fence();
        log::debug!("[focus-fence] bootstrap initial fence: {fence:?}");
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::InitialFocusFenceEstablished { fence },
            tick_ms,
        );
    }

    /// BUG-114 根本原因1（ADR-134 D1c）: 起動直後の初回フォーカス確立時に
    /// `app_policy` を live 側の profile 分類で初期化する。
    ///
    /// これが無いと `app_policy`（`ImeModel::app_policy`）は既定値
    /// `AppImePolicy::standard()`（`ImmCross` 固定、`default_feedback=Read`）
    /// のまま、最初のプロセス切替（`FocusChanged`）まで固定される。ユーザーが
    /// 起動後 1 つのアプリ（Windows Terminal 等）に留まり続けるだけの
    /// 自然な使い方でこの窓に入り、TsfNative/Imm32Unavailable では読み戻し
    /// 不能なため `Read` が無条件に再送し続ける（実機確認済み、
    /// `docs/known-bugs.md` BUG-114）。
    fn sync_initial_app_policy(&mut self, tick_ms: crate::state::TickMs) {
        let profile: crate::state::ime_event::ImePolicyProfile =
            self.platform.current_app_profile().into();
        log::debug!("[app-policy] bootstrap initial app_policy: profile={profile:?}");
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::InitialAppPolicyEstablished { profile },
            tick_ms,
        );
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
        let mut process_name = None;
        unsafe {
            imm_learning::learn_imm_capability_on_focus(
                &mut self.platform,
                hwnd,
                || {
                    let name = crate::focus::classify::get_process_name(process_id).to_lowercase();
                    // 取得失敗（空文字列）でも Some に包んで返す。CurrentFocus::
                    // update_with_process_name 側は Some(..) をそのまま採用するため、
                    // 失敗結果まで含めて再利用でき、同一 pid への get_process_name
                    // の再呼び出し（/code-review 指摘: 失敗時だけ二重取得が残っていた）
                    // を防げる。
                    process_name = Some(name.clone());
                    name
                },
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

        // SAFETY: `resolve_focus_kind` は Win32 API で HWND を問い合わせる unsafe fn。
        //         `hwnd` と `process_id` はフォーカスプローブで確認済みの有効な値。
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        let resolution = unsafe {
            kind_classifier::resolve_focus_kind(&self.platform, process_id, &class_name, hwnd)
        };
        let kind = resolution.kind;
        let reason = resolution.reason;
        let overridden = resolution.overridden;

        if self.platform_state.focus.focus_kind != kind {
            log::debug!(
                "Focus kind changed: {:?} → {kind:?} (reason={reason})",
                self.platform_state.focus.focus_kind
            );
            self.platform_state.focus.focus_kind = kind;
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
            process_name,
            class_name,
            kind,
        })
    }

    /// last_focus_info を更新し、(process_changed, prev_pid) を返す。
    ///
    /// process_changed な場合は事前に `hwnd_ime_cache.save()` を呼び出す。
    ///
    /// `is_bootstrap`: `establish_initial_focus_scope`（起動時、まだ一度も IME を
    /// 観測していない）からの呼び出しかどうか。`apply_app_disable_transition` へ
    /// そのまま伝播する。
    fn advance_focus_tracking(
        &mut self,
        classified: &ClassifiedFocus,
        is_bootstrap: bool,
    ) -> (bool, Option<u32>) {
        let last_pid = if self.platform.focus.is_focused() {
            Some(self.platform.focus.pid())
        } else {
            None
        };
        let process_changed = last_pid.is_some_and(|last| last != classified.process_id);
        // `focus_hwnd()` は `self.platform.focus.current.hwnd` を都度読む薄いラッパー
        // （`Runtime::focus_fence().hwnd`、ADR-106 決定3）。`prev_hwnd`/`new_hwnd` を
        // 同一の生成元（このメソッド）から取ることで、`update_focus_info` 呼び出しの
        // 前後で同じ式を2通りに書く重複を無くす（PR 109 コードレビュー指摘6）。
        // この時点ではまだ epoch は進んでいない（`on_focus_process_changed` は
        // 後続の `apply_focus_probe_result` 内で呼ばれる）ため、両軸を同時に運ぶ
        // `focus_fence()`（フル）ではなく hwnd 単独の `focus_hwnd()` を使う——ここで
        // フルフェンスを使うと「epoch が古いフェンス」を意図的に組み立てる最初の
        // 前例になってしまう。
        let prev_hwnd = self.focus_hwnd();

        if process_changed {
            let ime_on = self.platform_state.ime.effective_open();

            // 滞在時間が短すぎる（通知ポップアップ等の瞬間フォーカス）場合はキャッシュを
            // 上書きしない。last_focus_change_ms は前回の on_focus_process_changed で記録済み。
            let focus_start_ms = self.platform_state.focus.last_focus_change_ms;
            let now_ms = crate::hook::current_tick_ms();
            let focus_duration_ms = now_ms.saturating_sub(focus_start_ms);
            let should_save = focus_duration_ms >= crate::tuning::MIN_FOCUS_DURATION_MS;

            if should_save {
                // ime_on=false のとき、それがユーザーの明示操作（SyncKey 等）由来かを記録する。
                // Imm32Unavailable アプリでは IME を awase が制御できないためキャッシュが stale に
                // なりやすく、入場時に信頼できる OFF か否かをこのフラグで区別する。
                let from_explicit_off_intent = !ime_on && {
                    use crate::state::ime_event::UserIntentSource;
                    matches!(
                        self.platform_state.ime.model().last_intent.as_ref(),
                        Some(i) if !i.target
                            && matches!(
                                i.source,
                                UserIntentSource::SyncKey
                                    | UserIntentSource::PhysicalImeKey
                                    | UserIntentSource::Command
                            )
                    )
                };
                self.platform.focus.save_ime_state(
                    ime_on,
                    self.platform_state.ime.input_mode(),
                    from_explicit_off_intent,
                );
            } else {
                log::debug!(
                    "[focus] focus duration {focus_duration_ms}ms < MIN_FOCUS_DURATION_MS={} — cache save スキップ",
                    crate::tuning::MIN_FOCUS_DURATION_MS,
                );
            }
        }

        self.platform.update_focus_info_with_process_name(
            classified.process_id,
            classified.class_name.clone(),
            classified.hwnd.0 as usize,
            classified.process_name.clone(),
        );

        // `update_focus_info` 直後のため `self.focus_hwnd()` は `classified.hwnd` と
        // 構築上必ず一致する（同じ値を2通りの式で書く重複を無くす、PR 109
        // コードレビュー指摘6）。hwnd 追従の詳細は
        // `notify_focus_hwnd_updated_if_needed` のドキュメントを参照。
        let new_hwnd = self.focus_hwnd();
        self.notify_focus_hwnd_updated_if_needed(
            is_bootstrap,
            process_changed,
            prev_hwnd,
            new_hwnd,
        );

        self.apply_app_disable_transition(classified.process_id, is_bootstrap);

        self.platform_state.ime.set_prev_conversion_mode(None);

        (
            process_changed,
            if process_changed { last_pid } else { None },
        )
    }

    /// 同一プロセス内で hwnd だけが変わった場合、`ObservationStore` 側の
    /// `current_fence.hwnd` を追従させる（ADR-106 決定3、code review 2026-08-26
    /// で発見された退行の修正）。プロセス変更（`process_changed`）は後続の
    /// `on_focus_process_changed` が `FocusChanged`（epoch インクリメント +
    /// 観測プールクリア）で hwnd も一緒に更新するため、ここでは扱わない。
    ///
    /// **bootstrap では dispatch しない。** bootstrap 時点では
    /// `platform.focus.current.hwnd` がまだ 0 のため「hwnd だけが変わった」判定が
    /// 必ず成立してしまうが、初回フォーカスは同一プロセス内のウィンドウ移動では
    /// なく「最初のスコープ確立」であり、扱うべきは hwnd 片側ではなく epoch を
    /// 含む両軸である（bootstrap では `enter_focus_scope` が epoch も 0→1 に
    /// 進める）。この同期は `establish_initial_focus_scope` が
    /// `sync_initial_focus_fence`（`ImeEvent::InitialFocusFenceEstablished`）で
    /// 行う——ここから hwnd だけ先に dispatch すると、epoch が食い違ったままの
    /// 中途半端な fence を1度作ることになる（BUG-102）。この関数を
    /// `advance_focus_tracking` の
    /// 本体から切り出しているのは、`dispatch_event(` を直接テキストとして含む
    /// 関数が `establish_initial_focus_scope_does_not_write_ime_belief`
    /// （`architecture_guard.rs`）の対象リストに直接含まれるため——本体に
    /// `dispatch_event(` を残したまま `is_bootstrap` で実行時にガードしても、
    /// あのテストは静的テキスト検査のため機械的に落ちる。この関数の
    /// `!is_bootstrap` ガードは
    /// `focus_hwnd_updated_dispatch_is_skipped_during_bootstrap`
    /// （`architecture_guard.rs`）で固定する。
    fn notify_focus_hwnd_updated_if_needed(
        &mut self,
        is_bootstrap: bool,
        process_changed: bool,
        prev_hwnd: crate::state::ime_event::HwndId,
        new_hwnd: crate::state::ime_event::HwndId,
    ) {
        if is_bootstrap || process_changed || new_hwnd == prev_hwnd {
            return;
        }
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::FocusHwndUpdated { hwnd: new_hwnd },
            tick_ms,
        );
    }

    /// `config.app_overrides.disable_apps` への出入りを検出し、フックへ伝達する
    /// （BUG-78: リモートデスクトップ接続中にローカル側の awase が Ctrl キーの
    /// 押しっぱなし状態を起こす問題への対策）。
    ///
    /// `advance_focus_tracking` の `update_focus_info` 直後、フォーカス変更のたびに
    /// 呼ぶ想定。`process_changed` を流用せず自前でエッジ判定する — `process_changed`
    /// は初回フォーカス（`last_pid == None`）で `false` になるため、無効化対象アプリが
    /// フォーカスを持ったまま awase が起動したケースを取りこぼす。
    ///
    /// `is_bootstrap`（`establish_initial_focus_scope` からの呼び出し、まだ一度も IME
    /// を観測していない）の場合は `invalidate_engine_context` を呼ばない。この呼び出しは
    /// 「engine に pending だったチョードを安全にflushする」ための decision 実行だが、
    /// bootstrap時点では engine 生成直後で pending は存在しえず意味を持たない。ADR-102
    /// 決定3-b が「最初のIME観測より前にbeliefを書き換えない」ことを構造的に保証しよう
    /// としている以上、"今は何も起きないはず" という前提に頼らず、経路自体を通さない
    /// （Opus敵対的レビューで、この経路がbelief書き込みに繋がりうる非推移的な穴として
    /// 指摘され是正、2026-08-26）。`app_disabled` フラグ・フックラッチのクリアは
    /// belief を一切書かないため bootstrap でも通常どおり行う。
    pub(super) fn apply_app_disable_transition(&mut self, process_id: u32, is_bootstrap: bool) {
        use crate::state::app_suppression::{edge, SuppressionEdge};
        use awase::types::ContextChange;

        let was_disabled = self.platform_state.focus.app_disabled;
        let is_disabled = self.platform.focus.is_app_disabled();
        let transition = edge(was_disabled, is_disabled);
        if matches!(transition, SuppressionEdge::None) {
            return;
        }

        self.platform_state.focus.app_disabled = is_disabled;
        crate::hook::set_focus_app_disabled(is_disabled);
        crate::hook::clear_hook_latches_for_app_disable(transition);
        // [[keymap]] latch もフック側と同じタイミングで解放する（ADR-114 決定4
        // 「latch 漏れ対策」経路3）。FOCUS_APP_DISABLED 遷移中はフックが
        // deliver_key_event に一切イベントを渡さないため、latch が残っていても
        // 対応する KeyUp が永遠に届かない。
        self.platform_state.keymap.keymap_latch.release_all();

        if matches!(transition, SuppressionEdge::Enter) && !is_bootstrap {
            // 無効アプリに入った瞬間、pending だったチョードをタイマー満了に任せず
            // 確定させる。放置すると、Enter 後はフックが生キーを素通しするため
            // TIMER_PENDING を解決すべき後続キーイベントが engine に届かなくなり、
            // タイマー満了時に無効化先アプリへ謎の文字が飛ぶ（`toggle_app_override`
            // の NonText 降格時と同じ対処、enabled フラグ自体は変更しない）。
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        log::info!("[app-disable] {transition:?}: process_id={process_id} disabled={is_disabled}");
    }

    /// プロセス変更時の後処理（ログ・タイムスタンプ・output 通知・IME キャッシュ復元等）。
    #[expect(clippy::cognitive_complexity)]
    fn on_focus_process_changed(
        &mut self,
        classified: &ClassifiedFocus,
        prev_pid: Option<u32>,
        prev: &FocusIdentity,
    ) {
        log::info!(
            "FocusChange [{}→{}] {}: stale ime_on={} intent={:?} mode={:?} japanese={}",
            prev_pid.map_or_else(|| "?".to_string(), |p| p.to_string()),
            classified.process_id,
            classified.class_name,
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.explicit_intent(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
        );
        if let Some(started_at) = self.drift_giveup_started_at.take() {
            self.platform_state.ime.journal.record(
                crate::journal::JournalEntry::DriftGiveUpIntervalEnded {
                    reason: "FocusChanged",
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                },
            );
        }

        // 前ウィンドウの candidate_was_seen をキャリーオーバーしない。
        // 他プロセス窓で候補ウィンドウが表示された履歴が新窓の dispatch-ime に影響すると
        // effective_open が誤って true になり VK_KANJI を誤送信する（shadow desync 偽陽性）。
        crate::tsf::observer::reset_candidate_was_seen();
        let tick_ms = self.enter_focus_scope(classified);
        let new_profile = self.platform.current_app_profile();
        let new_hwnd = crate::state::ime_event::HwndId(classified.hwnd.0 as usize);
        // persistent_explicit_off_ms() を使う: FocusChanged が last_intent を
        // クリアしても、複数の rapid focus 変化（仮想デスクトップ切替等）で
        // 2 回目以降の guard が機能し続けるよう ImeStateHub 側で永続保持している。
        let pre_focus_explicit_off_ms = self.platform_state.ime.persistent_explicit_off_ms();
        let ime_profile = crate::state::ime_event::ImePolicyProfile::from(new_profile);
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::FocusChanged {
                from: (prev.hwnd != 0).then_some(crate::state::ime_event::HwndId(prev.hwnd)),
                to: new_hwnd,
                profile: ime_profile,
                focus_epoch: self.platform_state.focus.focus_epoch,
            },
            tick_ms,
        );

        {
            let cache_hit = self.platform.focus.restore_ime_state();
            let profile = self.platform.current_app_profile();
            let is_imm_broken = matches!(
                profile,
                crate::focus::classify::AppImeProfile::Imm32Unavailable,
            );
            // CASCADIA_HOSTING_WINDOW_CLASS 等は profile が Imm32Unavailable になるため
            // `matches!(profile, TsfNative)` では取りこぼす。`class_names.rs` 参照。
            let is_effectively_tsf = crate::focus::class_names::is_effectively_tsf_native(
                profile,
                &classified.class_name,
            );

            if is_effectively_tsf {
                // ── TsfNative SSOT ──────────────────────────────────────────────
                // awase が TSF 経由で完全制御できるため awase が SSOT として機能する。
                // 通常: desired_open を前窓の値のまま保持（push model）。
                //
                // 例外: Imm32Unavailable (Chrome 等) での明示 IME-OFF が
                // desired_open=false をグローバルに書いた後に TsfNative 窓へ戻る場合。
                // キャッシュが ime_on=true ならキャッシュ復元し TsfNative の最後の状態を回復する。
                // (desired_open がどのコンテキストで設定されたかではなく
                //  「キャッシュとの不一致」で Imm32Unavailable 汚染を検出する。)
                // 仮想デスクトップ transient bug (29a39b9) への影響なし:
                //  transient UWP 窓のキャッシュが false (explicit/non-explicit) の場合は
                //  cache_says_on=false → 復元しない → 従来の SSOT 継続。
                let desired_open = self.platform_state.ime.model().desired_open();
                let cache_says_on = matches!(&cache_hit, Some(snap) if snap.ime_on);
                if cache_says_on && !desired_open {
                    // Imm32Unavailable コンテキストで desired_open が false に汚染された可能性。
                    // キャッシュの true を復元して TsfNative 窓の状態を回復する。
                    self.platform_state
                        .ime
                        .apply_hwnd_cache_restore(cache_hit, tick_ms);
                    log::debug!(
                        "[focus] TsfNative: cache restore \
                         (desired_open=false だが cache=true — Imm32Unavailable 汚染を修正)"
                    );
                } else {
                    // SSOT: desired_open を前窓の値のまま維持。
                    // FocusChanged が applied=Unknown を設定済みのため、最初のキー入力で
                    // dispatch_ime が desired_open を窓へ apply する。
                    log::debug!(
                        "[focus] TsfNative/SSOT: cache restore スキップ — \
                         最初のキー入力で dispatch_ime が apply"
                    );
                }
            } else {
                // ── 純粋な Imm32Unavailable (Chrome/Edge 等) ────────────────────
                // awase が IME 状態を直接制御できないため、キャッシュが唯一の根拠。
                // 「ユーザー明示の OFF」由来でない false は stale とみなして破棄する。
                let discard_cache = should_discard_imm_broken_cache(
                    cache_hit,
                    is_imm_broken,
                    tick_ms.0,
                    pre_focus_explicit_off_ms,
                );
                if discard_cache {
                    log::debug!(
                        "[focus] Imm32Unavailable cache discarded \
                         (stale false or explicit IME OFF is newer) — treating as cache miss"
                    );
                }
                let effective_cache = if discard_cache { None } else { cache_hit };
                let effective_cache_miss = effective_cache.is_none();
                self.platform_state
                    .ime
                    .apply_hwnd_cache_restore(effective_cache, tick_ms);

                if effective_cache_miss {
                    let last_off_ms = pre_focus_explicit_off_ms;
                    let elapsed = tick_ms.saturating_sub(last_off_ms);
                    if last_off_ms > 0 && elapsed < EXPLICIT_OFF_CACHE_SUPPRESS_MS {
                        log::debug!(
                            "[focus] Imm32Unavailable cache-miss: skip reset_stale \
                             — explicit IME OFF {elapsed}ms ago",
                        );
                    } else {
                        self.platform_state.ime.reset_stale_ime_on_for_imm_broken(
                            crate::state::ime_event::ImePolicyProfile::Imm32Unavailable,
                            tick_ms,
                        );
                    }
                }
            }
        }

        // 非 TsfNative（Standard/ImmCross/Plain/Unknown）: VK_KANJI はトグルのため、
        // desired=true でキャッシュが ON なら applied=true に先同期して冗長な
        // VK_KANJI を防ぐ（ADR-098 決定5: 旧コメント「Imm32Unavailable (Chrome 等)
        // のみ」は実際のガード条件 `!is_effectively_tsf_native` と食い違っていた
        // ため訂正——`Standard`+MS-IME の `CHAIN_IMM_CROSS_THEN_KANJI` が今も
        // `KanjiToggle` を含むため、この pre-sync は Standard でも引き続き必要）。
        // TsfNative は SSOT model: applied=Unknown のまま維持し、最初のキーで
        // SetOpen が VK_DBE_HIRAGANA/ALPHANUMERIC (SET、トグルでない) を発行する。
        //
        // この後の focus-resync arm 判定（本関数末尾）でも同じ問い合わせが必要なため
        // ここで一度だけ計算して使い回す（BUG-77 code review 追補: 同一引数での
        // 重複計算の指摘）。
        let is_effectively_tsf_native_now = crate::focus::class_names::is_effectively_tsf_native(
            self.platform.current_app_profile(),
            self.platform.focus.class_name(),
        );
        if !is_effectively_tsf_native_now {
            let ime_on_now = self.platform_state.ime.effective_open();
            if ime_on_now {
                self.platform_state.ime.record_confirmed(true, tick_ms.0);
                log::debug!(
                    "[focus] Imm32Unavailable hard pre-sync applied=true \
                     (prevent spurious VK_KANJI on first character key)"
                );
                // BUG-18: HwndCacheRestored / mirror_applied_open は belief 層
                // (ImeModel) だけを ON に戻し、GjiFsm には一切通知しない。
                // 無操作中の AppKind 往復 (TsfNative⇔Uwp) で本経路を繰り返し通ると、
                // 直前の実 IME-OFF で GjiFsm::OffCold に入ったまま belief だけが
                // ON に戻り、再開後の StartComposition が OffCold で握りつぶされて
                // 最初の数文字が欠落する。sync_ime_kind_from_observation
                // (runtime/message_handlers.rs) と同じ「belief=ON なら GjiFsm へも
                // ImeOn を通知する」パターンをここにも適用して揃える。GjiFsm が
                // 既に ON なら ImeOn ハンドラ側で no-op になる (gji_fsm.rs 558-565)。
                if matches!(
                    crate::tsf::observer::tsf_obs().active_ime_kind(),
                    crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
                ) {
                    let mode = self.platform.output.injection_mode;
                    self.platform.gji_on_ime_on(mode);
                    for entry in self.platform.drain_journal_entries() {
                        self.platform_state.ime.journal.absorb(entry);
                    }
                }
            }
        }

        // ImmCross アプリ（Qt/LINE 等）: FocusChanged 直後に child hwnd の正確な IME 状態を
        // 非同期読み取りする。FocusProbe（first-key トリガー）より早く確定させることで
        // 最初のキー入力から正しい belief で engine が動作する。
        // FocusChanged が observations をクリアした後のため、この probe が最初の High conf 観測になる。
        //
        // エポック照合: spawn 後にフォーカスが変わった場合（仮想デスクトップ切替中の経由ウィンドウ等）
        // は棄却する。時間ベースのシャドウグレースより正確で、誤って High confidence false を
        // 書き込む Engine OFF カスケードを構造的に防ぐ。
        if matches!(
            self.platform.current_app_profile(),
            crate::focus::classify::AppImeProfile::Standard,
        ) && self.platform_state.ime.belief.is_japanese_ime()
        {
            let ticket = crate::state::probe_admission::ImmLikeTicket {
                fence: self.focus_fence(),
            };
            win32_async::spawn_local(async move {
                let snap = crate::ime::read_ime_state_full_async().await;
                if let Some(open) = snap.ime_on {
                    let _ = crate::with_app(|app| {
                        crate::state::probe_admission::admit_epoch_in_app(
                            app,
                            ticket,
                            "[ImmCrossProbe/focus] epoch rejected \
                             (transient window — focus changed since probe spawn)",
                            |app, accepted| {
                                let now_tick = crate::state::TickMs(crate::hook::current_tick_ms());
                                app.platform_state
                                    .ime
                                    .write_imm_cross_probe(open, now_tick, accepted);
                                log::debug!(
                                    "[ImmCrossProbe/focus] child-hwnd IME={open} → \
                                     High confidence 観測記録"
                                );
                            },
                        );
                    });
                }
            });
        }

        if self.platform_state.ime.is_force_on_guard_active()
            || self.platform_state.ime.detect_miss_count() > 0
        {
            log::debug!(
                "Focus changed: clearing force_on_guard and detect_miss_count \
                 (new window may have different IME state)"
            );
            self.platform_state.ime.reset_detect_state();
        }

        if classified.kind == FocusKind::Undetermined {
            self.platform
                .focus
                .try_send_uia(crate::focus::uia::SendableHwnd(classified.hwnd));
        }

        // フォーカス復帰後 resync（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）: TsfNative は
        // ImmCross と違い上記の focus-time probe が無く、周期ポーリングも
        // `reschedule_ime_refresh` が早期 return するため構造的に走らない
        // （`runtime/mod.rs::reschedule_ime_refresh` 参照）。復帰後の最初の
        // resync 対象キー（`RawKeyEvent::starts_focus_resync()`）を defer して
        // resync を待たせるための armed フラグをここで立てる。タイマーは張らない
        // （ユーザーがいつ打つか分からないため。有効期限も付けない——
        // `state/focus_resync_policy.rs` の doc 参照）。
        if crate::state::focus_resync_policy::should_arm_focus_resync(
            is_effectively_tsf_native_now,
            self.platform_state.ime.belief.is_japanese_ime(),
            crate::send_health::blocking_allowed(tick_ms.0),
            self.platform_state
                .gate
                .idle_conv_check_in_flight_since_ms
                .is_some(),
        ) {
            crate::focus_resync::FOCUS_RESYNC.arm(tick_ms.0);
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

fn focus_endpoint(identity: &FocusIdentity) -> crate::journal::FocusEndpoint {
    crate::journal::FocusEndpoint {
        hwnd: crate::state::ime_event::HwndId(identity.hwnd),
        pid: identity.pid,
        process_name: identity.process_name.clone(),
        class_name: identity.class_name.clone(),
        app_kind: format!("{:?}", identity.app_kind),
        focus_kind: format!("{:?}", identity.focus_kind),
    }
}

fn should_discard_imm_broken_cache(
    cache_hit: Option<crate::focus::hwnd_cache::HwndImeSnapshot>,
    is_imm_broken: bool,
    now_ms: u64,
    pre_focus_explicit_off_ms: u64,
) -> bool {
    if !is_imm_broken {
        return false;
    }
    let Some(snap) = cache_hit else {
        return false;
    };
    if !snap.ime_on {
        return !snap.from_explicit_off_intent;
    }
    if pre_focus_explicit_off_ms == 0 {
        return false;
    }
    now_ms.saturating_sub(pre_focus_explicit_off_ms) < EXPLICIT_OFF_CACHE_SUPPRESS_MS
}

#[cfg(test)]
mod tests {
    use awase::engine::InputModeState;

    use super::*;

    fn snap(
        ime_on: bool,
        from_explicit_off_intent: bool,
    ) -> crate::focus::hwnd_cache::HwndImeSnapshot {
        crate::focus::hwnd_cache::HwndImeSnapshot {
            ime_on,
            input_mode: InputModeState::ObservedRomaji,
            recorded_ms: 0,
            from_explicit_off_intent,
        }
    }

    #[test]
    fn imm_broken_true_cache_is_discarded_right_after_explicit_off() {
        assert!(should_discard_imm_broken_cache(
            Some(snap(true, false)),
            true,
            20_000,
            19_000,
        ));
    }

    #[test]
    fn imm_broken_true_cache_is_kept_after_explicit_off_window_expires() {
        assert!(!should_discard_imm_broken_cache(
            Some(snap(true, false)),
            true,
            30_001,
            20_000,
        ));
    }

    #[test]
    fn imm_broken_false_cache_is_kept_when_it_came_from_explicit_off() {
        assert!(!should_discard_imm_broken_cache(
            Some(snap(false, true)),
            true,
            20_000,
            19_000,
        ));
    }
}
