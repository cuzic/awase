//! フォーカス追跡の状態を一箇所に集約する構造体。
//!
//! `WindowsPlatform` が散在して持っていた 6 つのフォーカス関連フィールドを
//! `FocusTracker` に移動し、意味のある操作単位で API を提供する。

use std::sync::mpsc::Sender;

use awase::engine::InputModeState;
use awase::types::FocusKind;

use crate::focus::cache::{DetectionSource, FocusCache};
use crate::focus::classifier::{ForceOverrides, ImmCapability, ImmCapabilityStore, InjectionHint};
use crate::focus::current::CurrentFocus;
use crate::focus::hwnd_cache::{HwndImeCache, HwndImeSnapshot};
use crate::focus::uia::SendableHwnd;

/// フォーカス追跡に関わる全状態を集約する構造体。
///
/// `CurrentFocus`（ウィンドウ情報）、判定キャッシュ、IME キャッシュ、
/// IMM 能力学習ストア、UIA 送信チャネルを一括で保持する。
pub(crate) struct FocusTracker {
    /// 現在フォーカス中のウィンドウ情報（pid / class_name / app_profile / process_name）
    pub(crate) current: CurrentFocus,
    cache: FocusCache,
    overrides: ForceOverrides,
    uia_sender: Option<Sender<SendableHwnd>>,
    imm_learning: ImmCapabilityStore,
    hwnd_ime_cache: HwndImeCache,
}

impl std::fmt::Debug for FocusTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusTracker").finish_non_exhaustive()
    }
}

impl FocusTracker {
    pub(crate) fn new(
        cache: FocusCache,
        overrides: ForceOverrides,
        imm_learning: ImmCapabilityStore,
    ) -> Self {
        Self {
            current: CurrentFocus::unfocused(),
            cache,
            overrides,
            uia_sender: None,
            imm_learning,
            hwnd_ime_cache: HwndImeCache::new(),
        }
    }

    // ── クエリ ──────────────────────────────────────────────────────────────

    pub(crate) const fn is_focused(&self) -> bool {
        self.current.is_focused()
    }

    pub(crate) const fn pid(&self) -> u32 {
        self.current.pid
    }

    pub(crate) fn class_name(&self) -> &str {
        &self.current.class_name
    }

    pub(crate) fn process_name(&self) -> &str {
        &self.current.process_name
    }

    pub(crate) const fn current_profile(&self) -> crate::focus::class_names::AppImeProfile {
        self.current.app_profile
    }

    pub(crate) fn injection_hint(&self) -> InjectionHint {
        if !self.current.is_focused() {
            return InjectionHint::Default;
        }
        self.overrides
            .injection_hint(self.current.pid, &self.current.class_name)
    }

    // ── フォーカス更新 ──────────────────────────────────────────────────────

    /// フォーカス情報を更新する。`app_profile` は `class_name` から自動導出。
    pub(crate) fn update(&mut self, pid: u32, class_name: String) {
        self.current.update(pid, class_name);
    }

    // ── フォーカスキャッシュ ────────────────────────────────────────────────

    pub(crate) fn cache_get(&self, pid: u32, class_name: &str) -> Option<FocusKind> {
        self.cache.get(pid, class_name)
    }

    pub(crate) fn override_check(&self, pid: u32, class_name: &str) -> Option<FocusKind> {
        self.overrides.check_app_override(pid, class_name)
    }

    pub(crate) fn cache_insert(
        &mut self,
        pid: u32,
        class_name: String,
        kind: FocusKind,
        source: DetectionSource,
    ) {
        self.cache.insert(pid, class_name, kind, source);
    }

    /// キャッシュを空の状態にリセットする（設定リロード時）。
    pub(crate) fn cache_reset(&mut self) {
        self.cache = FocusCache::new();
    }

    /// アプリオーバーライド設定を差し替える（設定リロード時）。
    pub(crate) fn reset_overrides(&mut self, overrides: ForceOverrides) {
        self.overrides = overrides;
    }

    // ── IME 状態の保存/復元 ─────────────────────────────────────────────────

    /// フォーカス離脱前に現ウィンドウの IME 状態を保存する。
    ///
    /// `self.current` の pid / class_name を使うため、`update()` の前に呼ぶこと。
    /// フォーカスが確立していない場合は何もしない。
    pub(crate) fn save_ime_state(
        &mut self,
        ime_on: bool,
        input_mode: InputModeState,
        from_explicit_off_intent: bool,
    ) {
        if !self.current.is_focused() {
            return;
        }
        self.hwnd_ime_cache.save(
            self.current.pid,
            self.current.class_name.clone(),
            ime_on,
            input_mode,
            from_explicit_off_intent,
        );
    }

    /// フォーカス入場時に新ウィンドウの IME 状態キャッシュを復元する。
    ///
    /// `self.current` の pid / class_name を使うため、`update()` の後に呼ぶこと。
    pub(crate) fn restore_ime_state(&self) -> Option<HwndImeSnapshot> {
        self.hwnd_ime_cache
            .restore(self.current.pid, &self.current.class_name)
    }

    // ── IMM 能力学習 ─────────────────────────────────────────────────────────

    pub(crate) fn imm_capability(&self, class_name: &str) -> Option<ImmCapability> {
        self.imm_learning.get(class_name)
    }

    pub(crate) fn learn_imm_capability(&mut self, class_name: String, cap: ImmCapability) {
        self.imm_learning.learn(class_name, cap);
    }

    /// IMM 能力キャッシュを全クリアし、削除件数を返す（診断コマンド用）。
    pub(crate) fn clear_imm_learning(&mut self) -> usize {
        self.imm_learning.clear()
    }

    // ── UIA ─────────────────────────────────────────────────────────────────

    pub(crate) fn set_uia_sender(&mut self, tx: Sender<SendableHwnd>) {
        self.uia_sender = Some(tx);
    }

    /// UIA ワーカーに hwnd を送る。チャネル未設定または送信失敗は黙って無視する。
    pub(crate) fn try_send_uia(&self, hwnd: SendableHwnd) {
        if let Some(sender) = &self.uia_sender {
            let _ = sender.send(hwnd);
        }
    }
}
