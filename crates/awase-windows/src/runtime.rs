use awase::engine::{Engine, EngineCommand, InputContext};
use awase::types::{ContextChange, FocusKind, ImeCacheState, RawKeyEvent, ShadowImeAction, VkCode};
use awase::yab::YabLayout;

use crate::executor::DecisionExecutor;
use crate::focus::cache::DetectionSource;
use crate::hook::CallbackResult;
use crate::ime::HybridProvider;
use crate::{FOCUS_KIND, IME_RELIABILITY, IME_STATE_CACHE};

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[allow(dead_code)] // left/right_thumb_vk はレイアウト切替時に使用予定
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
    pub left_thumb_vk: VkCode,
    pub right_thumb_vk: VkCode,
}

// ── FocusDetector（フォーカス検出状態）──

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
pub struct FocusDetector {
    pub cache: crate::focus::cache::FocusCache,
    pub overrides: awase::config::FocusOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub uia_sender: Option<std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>>,
}

impl FocusDetector {
    pub fn new(overrides: awase::config::FocusOverrides) -> Self {
        Self {
            cache: crate::focus::cache::FocusCache::new(),
            overrides,
            last_focus_info: None,
            uia_sender: None,
        }
    }

    pub fn set_uia_sender(
        &mut self,
        sender: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.uia_sender = Some(sender);
    }
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
    #[allow(dead_code)] // IME プロバイダは将来のモード検出で使用予定
    pub ime: HybridProvider,
    pub layouts: Vec<LayoutEntry>,
    /// IME 同期キー（イベント事前分類用）
    pub sync_toggle_keys: Vec<VkCode>,
    pub sync_on_keys: Vec<VkCode>,
    pub sync_off_keys: Vec<VkCode>,
}

impl Runtime {
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
        self.executor.execute_from_loop(decision)
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub fn toggle_engine(&mut self) {
        let decision = self.engine.on_command(EngineCommand::ToggleEngine);
        self.executor.execute_from_loop(decision);
        // ウィンドウごとのエンジン状態をキャッシュに保存
        if let Some((pid, cls)) = self.executor.platform.focus.last_focus_info.as_ref() {
            self.executor.platform.focus.cache.set_engine_state(
                *pid,
                cls.clone(),
                self.engine.is_fsm_enabled(),
            );
        }
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason));
        self.executor.execute_from_loop(decision);
    }

    /// IME ON/OFF 状態をキャッシュに書き込む。
    ///
    /// Observer → Engine → Runtime の 3 層パイプラインで処理する。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn refresh_ime_state_cache(&mut self) {
        // Observer: OS 観測 → ImeObservation
        let obs = unsafe { crate::observer::ime_observer::observe(&IME_RELIABILITY) };

        // Engine: 判断 → Decision
        let decision = self.engine.on_command(EngineCommand::ImeObserved(obs));

        // Runtime: 副作用実行
        self.executor.execute_from_loop(decision);
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
            .on_command(EngineCommand::SwapLayout(entry.layout.clone()));
        self.executor.execute_from_loop(decision);

        self.executor.platform.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// 手動フォーカスオーバーライドのトグル処理
    pub fn toggle_focus_override(&mut self) {
        let current = FocusKind::load(&FOCUS_KIND);
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        new_kind.store(&FOCUS_KIND);

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

        // Clear any active buffers
        self.engine.on_command(EngineCommand::ClearDeferredKeys);
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

    /// IME 制御キー後に遅延されたキーを再処理する。
    pub fn process_deferred_keys(&mut self) {
        // IME 状態キャッシュを更新（メッセージループ上なのでブロッキング OK）
        self.refresh_ime_state_cache();

        let ctx = InputContext {
            ime_cache: ImeCacheState::load(&IME_STATE_CACHE),
        };
        let decisions = self.engine.process_deferred_keys(&ctx);
        for decision in decisions {
            // メッセージループ上なので即座に drain して Effects を実行する
            self.execute_decision(decision);
        }
    }
}
