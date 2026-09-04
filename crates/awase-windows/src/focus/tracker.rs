//! フォーカス追跡の状態を一箇所に集約する構造体。
//!
//! `WindowsPlatform` が散在して持っていた 6 つのフォーカス関連フィールドを
//! `FocusTracker` に移動し、意味のある操作単位で API を提供する。

use std::sync::mpsc::Sender;

use crate::focus::FocusKind;
use awase::engine::InputModeState;

use crate::focus::cache::{DetectionSource, FocusCache};
use crate::focus::classifier::{
    ForceOverrides, ImmCapability, ImmCapabilityStore, InjectionHint, InjectionModeStore,
};
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
    injection_mode_store: InjectionModeStore,
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
        injection_mode_store: InjectionModeStore,
    ) -> Self {
        Self {
            current: CurrentFocus::unfocused(),
            cache,
            overrides,
            uia_sender: None,
            imm_learning,
            injection_mode_store,
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

    /// `app_overrides.input_relay_apps`（issue #136 / BUG-90 決定4）。
    ///
    /// 正規ルート。`focus/classifier.rs` のプロセスグローバル
    /// `input_relay_apps_snapshot()` は `self` を持たない `pub unsafe fn`
    /// （`ime.rs::read_ime_state_fast`）専用であり、`FocusTracker` に到達
    /// できる呼び出し元はこちらを使うこと。
    pub(crate) fn input_relay_apps(&self) -> &[String] {
        self.overrides.input_relay_apps()
    }

    pub(crate) fn injection_hint(&self) -> InjectionHint {
        if !self.current.is_focused() {
            return InjectionHint::Default;
        }
        let hint = self
            .overrides
            .injection_hint(self.current.pid, &self.current.class_name);
        if hint != InjectionHint::Default {
            return hint;
        }
        if self.injection_mode_store.has_tsf(&self.current.class_name) {
            return InjectionHint::ForceTsf;
        }
        InjectionHint::Default
    }

    /// 指定した pid/class に対する injection_hint を返す（フォーカス変更直後の stale 回避用）。
    /// `self.current` が更新される前に新ウィンドウの hint を引くために使う。
    pub(crate) fn injection_hint_for(&self, pid: u32, class_name: &str) -> InjectionHint {
        let hint = self.overrides.injection_hint(pid, class_name);
        if hint != InjectionHint::Default {
            return hint;
        }
        if self.injection_mode_store.has_tsf(class_name) {
            return InjectionHint::ForceTsf;
        }
        InjectionHint::Default
    }

    // ── フォーカス更新 ──────────────────────────────────────────────────────

    /// フォーカス情報を更新する。`app_profile` は `class_name` から自動導出したうえで、
    /// 実測学習（`ImmCapabilityStore`）による降格を適用する。
    pub(crate) fn update(&mut self, pid: u32, class_name: String, hwnd: usize) {
        self.update_with_process_name(pid, class_name, hwnd, None);
    }

    pub(crate) fn update_with_process_name(
        &mut self,
        pid: u32,
        class_name: String,
        hwnd: usize,
        process_name: Option<String>,
    ) {
        // BUG-111: このメソッドは実フォーカス変更時だけでなく、`run_ime_refresh` の
        // 500ms 周期リフレッシュ（`runtime/ime_refresh.rs::ir_stage_focus`）からも
        // 同一ウィンドウのまま毎ティック呼ばれる（自己修復のため store 再照会自体は
        // 必要、doc コメント参照）。`self.current.update_with_process_name` は
        // `app_profile` を毎回いったん静的分類にリセットするため、以前は
        // 「リセット直後の静的値」対「学習済み降格値」を比較しており、既に
        // 降格済みで何も変わっていないティックでも INFO ログが再発火し続けていた
        // （実機で同一ウィンドウ在留中に 500ms おき・42 回連続再発火を確認）。
        // 直前 tick の実効値（同一 pid/class のときだけ意味を持つ）と比較し、
        // 本当に遷移した時だけログする。
        let prev_pid = self.current.pid;
        let prev_class_name_matches = self.current.class_name == class_name;
        let prev_profile = self.current.app_profile;

        self.current.update_with_process_name(
            pid,
            class_name,
            hwnd,
            self.overrides.input_relay_apps(),
            process_name,
        );
        let learned = self
            .imm_learning
            .get(&self.current.process_name, &self.current.class_name);
        let overridden = Self::apply_learned_imm_capability(self.current.app_profile, learned);
        if overridden != self.current.app_profile {
            let same_window = prev_pid == pid && prev_class_name_matches;
            if Self::should_log_demotion(same_window, prev_profile, overridden) {
                log::info!(
                    "[imm-learning] profile 降格: process={:?} class={:?} {:?} → {:?} \
                     (実測学習 ImmCapability::Unavailable。誤学習なら cache.toml の \
                     [imm_capability] から該当 process/class を削除)",
                    self.current.process_name,
                    self.current.class_name,
                    self.current.app_profile,
                    overridden,
                );
            }
            self.current.app_profile = overridden;
        }
    }

    /// BUG-111: `[imm-learning] profile 降格` ログを「本当に遷移した時だけ」出す
    /// 純粋判定。同一ウィンドウ（`same_window`）に在留したまま前回と同じ
    /// `overridden` 値が再計算されただけなら false（既に一度ログ済み・何も新しい
    /// 情報が無い）。ウィンドウが変わった、または降格結果自体が変わった場合は true。
    fn should_log_demotion(
        same_window: bool,
        prev_profile: crate::focus::class_names::AppImeProfile,
        overridden: crate::focus::class_names::AppImeProfile,
    ) -> bool {
        !(same_window && prev_profile == overridden)
    }

    /// 学習済み IMM 能力による profile 降格の純粋判定。
    ///
    /// 静的分類が `Standard` かつ実測学習が `Unavailable` のときだけ
    /// `Imm32Unavailable` に降格する。昇格方向（`Works` による
    /// `Imm32Unavailable`/`TsfNative` → `Standard`）は行わない — 静的リストの
    /// Imm32 不可・TSF ネイティブ知識は実測 1 回の「読めた」より強いため。
    ///
    /// これにより静的リストに載っていない IMM-broken アプリ（`ImmGetDefaultIMEWnd`
    /// が NULL / IME 検出ミスが閾値超え）でも、ImmCross の無駄な
    /// `SendMessageTimeoutW` を踏まずに MsImeDirect / GjiDirect / KanjiToggle 系へ
    /// 直行できる。学習の書き手は `focus/imm_learning.rs`（フォーカス時の
    /// `ImmGetDefaultIMEWnd` 判定）と `Runtime::learn_imm_capability_from_miss`
    /// （IME 検出ミス数の閾値超え/回復）。`Works` 回復学習で store が更新されれば
    /// 次のフォーカス更新から降格は解除される（自己修復）。
    pub(crate) fn apply_learned_imm_capability(
        static_profile: crate::focus::class_names::AppImeProfile,
        learned: Option<ImmCapability>,
    ) -> crate::focus::class_names::AppImeProfile {
        use crate::focus::class_names::AppImeProfile;
        match (static_profile, learned) {
            (AppImeProfile::Standard, Some(ImmCapability::Unavailable)) => {
                AppImeProfile::Imm32Unavailable
            }
            (p, _) => p,
        }
    }

    // ── フォーカスキャッシュ ────────────────────────────────────────────────

    pub(crate) fn cache_get(&self, pid: u32, class_name: &str) -> Option<FocusKind> {
        self.cache.get(pid, class_name)
    }

    pub(crate) fn override_check(&self, pid: u32, class_name: &str) -> Option<FocusKind> {
        self.overrides.check_app_override(pid, class_name)
    }

    /// 現在フォーカス中のプロセスが `disable_apps` にマッチしているか
    /// （BUG-78 対策、`runtime/focus_tracking.rs` から呼ぶ）。
    pub(crate) fn is_app_disabled(&self) -> bool {
        self.overrides.is_app_disabled(&self.current.process_name)
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

    pub(crate) fn imm_capability(
        &self,
        process_name: &str,
        class_name: &str,
    ) -> Option<ImmCapability> {
        self.imm_learning.get(process_name, class_name)
    }

    pub(crate) fn learn_imm_capability(
        &mut self,
        process_name: String,
        class_name: String,
        cap: ImmCapability,
    ) {
        self.imm_learning.learn(process_name, class_name, cap);
    }

    pub(crate) fn record_imm_null_probe(&mut self, process_name: String, class_name: String) {
        self.imm_learning
            .record_null_probe(process_name, class_name);
    }

    pub(crate) fn clear_imm_pending_unavailable(&mut self, process_name: &str, class_name: &str) {
        self.imm_learning
            .clear_pending_unavailable(process_name, class_name);
    }

    // ── Injection モード学習 ────────────────────────────────────────────────

    /// class_name が Tsf モード必要と学習済みかどうか。
    pub(crate) fn has_learned_injection_mode_tsf(&self, class_name: &str) -> bool {
        self.injection_mode_store.has_tsf(class_name)
    }

    /// GJI write 観測で判明した「Tsf 必要」クラスを永続化する（事後昇格）。
    pub(crate) fn learn_injection_mode_tsf(&mut self, class_name: String) {
        self.injection_mode_store.learn_tsf(class_name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::class_names::AppImeProfile;

    // ── apply_learned_imm_capability（B5 配線の純粋判定）────────────────────

    #[test]
    fn standard_with_learned_unavailable_downgrades() {
        assert_eq!(
            FocusTracker::apply_learned_imm_capability(
                AppImeProfile::Standard,
                Some(ImmCapability::Unavailable)
            ),
            AppImeProfile::Imm32Unavailable,
            "実測で IMM 不可と学習済みの Standard クラスは降格する"
        );
    }

    #[test]
    fn standard_with_learned_works_or_unlearned_stays() {
        assert_eq!(
            FocusTracker::apply_learned_imm_capability(
                AppImeProfile::Standard,
                Some(ImmCapability::Works)
            ),
            AppImeProfile::Standard,
        );
        assert_eq!(
            FocusTracker::apply_learned_imm_capability(AppImeProfile::Standard, None),
            AppImeProfile::Standard,
        );
    }

    // ── should_log_demotion（BUG-111: 500ms リフレッシュ毎の再発火防止）───────

    #[test]
    fn should_log_demotion_suppresses_repeat_on_same_window() {
        // 同一ウィンドウに在留したまま、前回と同じ降格結果が再計算されただけなら
        // ログしない（`run_ime_refresh` の 500ms 周期ティックで無条件に再発火して
        // いた実機バグの再発防止）。
        assert!(!FocusTracker::should_log_demotion(
            true,
            AppImeProfile::Imm32Unavailable,
            AppImeProfile::Imm32Unavailable,
        ));
    }

    #[test]
    fn should_log_demotion_fires_on_first_transition() {
        // 別ウィンドウから遷移してきた直後（same_window=false）は、たとえ
        // 降格結果の値が偶然同じでも「新しい情報」としてログする。
        assert!(FocusTracker::should_log_demotion(
            false,
            AppImeProfile::Standard,
            AppImeProfile::Imm32Unavailable,
        ));
    }

    #[test]
    fn should_log_demotion_fires_when_result_changes_within_same_window() {
        // 同一ウィンドウでも、自己修復（`Works` 学習）等で結果自体が変われば
        // 変化イベントとしてログする。
        assert!(FocusTracker::should_log_demotion(
            true,
            AppImeProfile::Imm32Unavailable,
            AppImeProfile::Standard,
        ));
    }

    #[test]
    fn static_classification_is_never_upgraded() {
        // 静的な Imm32Unavailable / TsfNative 知識は実測 Works より強い（昇格しない）。
        assert_eq!(
            FocusTracker::apply_learned_imm_capability(
                AppImeProfile::Imm32Unavailable,
                Some(ImmCapability::Works)
            ),
            AppImeProfile::Imm32Unavailable,
        );
        assert_eq!(
            FocusTracker::apply_learned_imm_capability(
                AppImeProfile::TsfNative,
                Some(ImmCapability::Unavailable)
            ),
            AppImeProfile::TsfNative,
            "TsfNative は Imm32Unavailable と物理キー扱いが異なるため学習で動かさない"
        );
    }
}
