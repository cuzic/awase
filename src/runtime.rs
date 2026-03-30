use awase::engine::{Engine, EngineCommand, InputContext};
use awase::types::{ContextChange, FocusKind, ImeCacheState, VkCode};
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
}

impl Runtime {
    /// Decision の副作用を実行する — executor に委譲
    pub(crate) fn execute_decision(&mut self, decision: awase::engine::Decision) -> CallbackResult {
        self.executor.execute(decision)
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub(crate) fn toggle_engine(&mut self) {
        let decision = self.engine.on_command(EngineCommand::ToggleEngine);
        self.executor.execute(decision);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub(crate) fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason));
        self.executor.execute(decision);
    }

    /// IME ON/OFF 状態をキャッシュに書き込む。
    ///
    /// Observer → Engine → Runtime の 3 層パイプラインで処理する。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub(crate) fn refresh_ime_state_cache(&mut self) {
        // Observer: OS 観測 → ImeObservation
        let obs = unsafe { crate::observer::ime_observer::observe(&IME_RELIABILITY) };

        // Engine: 判断 → Decision
        let decision = self.engine.on_command(EngineCommand::ImeObserved(obs));

        // Runtime: 副作用実行
        self.executor.execute(decision);
    }

    /// 配列を動的に切り替える
    pub(crate) fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self
            .engine
            .on_command(EngineCommand::SwapLayout(entry.layout.clone()));
        self.executor.execute(decision);

        self.executor.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// 手動フォーカスオーバーライドのトグル処理
    pub(crate) fn toggle_focus_override(&mut self) {
        let current = FocusKind::load(&FOCUS_KIND);
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        new_kind.store(&FOCUS_KIND);

        // Update learning cache
        if let Some((pid, cls)) = self.executor.focus.last_focus_info.as_ref() {
            self.executor.focus.cache.insert(
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
        self.executor.tray.show_balloon(
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
    pub(crate) fn process_deferred_keys(&mut self) {
        // IME 状態キャッシュを更新（メッセージループ上なのでブロッキング OK）
        self.refresh_ime_state_cache();

        let ctx = InputContext {
            ime_cache: ImeCacheState::load(&IME_STATE_CACHE),
        };
        let decisions = self.engine.process_deferred_keys(&ctx);
        for decision in decisions {
            self.executor.execute(decision);
        }
    }
}
