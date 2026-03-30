//! フォーカス検出モジュール
//!
//! ウィンドウのフォーカス変更を監視し、テキスト入力コントロールかどうかを判定する。
//! Phase 1-2（同期）+ Phase 3（UIA 非同期）の多段判定を行い、
//! タイピングパターンによる推定も併用する。

pub mod cache;
pub mod classify;
pub mod msaa;
pub mod pattern;
pub mod uia;

use awase::types::{ContextChange, FocusKind};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::KillTimer;

use std::sync::mpsc;

use crate::focus::cache::{DetectionSource, FocusCache};
use crate::focus::pattern::KeyPatternTracker;
use crate::focus::uia::SendableHwnd;

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
///
/// `FOCUS_KIND`（`AtomicU8`）は UIA ワーカースレッドからもアクセスされるため、
/// スレッド安全性のために別の static として保持する。
pub struct FocusDetector {
    pub cache: FocusCache,
    pub overrides: awase::config::FocusOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub pattern_tracker: KeyPatternTracker,
    pub uia_sender: Option<mpsc::Sender<SendableHwnd>>,
}

impl FocusDetector {
    pub fn new(overrides: awase::config::FocusOverrides) -> Self {
        Self {
            cache: FocusCache::new(),
            overrides,
            last_focus_info: None,
            pattern_tracker: KeyPatternTracker::new(),
            uia_sender: None,
        }
    }

    pub fn set_uia_sender(&mut self, sender: mpsc::Sender<SendableHwnd>) {
        self.uia_sender = Some(sender);
    }

    /// フォーカス変更時などに単一スレッド側の状態をリセットする
    #[allow(dead_code)] // フォーカスリセットの将来拡張用に保持
    pub fn clear(&mut self) {
        self.cache = FocusCache::new();
        self.last_focus_info = None;
        self.pattern_tracker.clear();
    }
}

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// フォーカス変更イベントフックを登録する
///
/// `WINEVENT_OUTOFCONTEXT` を使用するため、コールバックはメッセージループ上で実行される。
/// これにより `classify_focus` が非同期（キーイベントとは別タイミング）で呼ばれる。
pub fn install_focus_hook() {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            log::warn!("Failed to install focus event hook");
        } else {
            log::info!("Focus event hook installed");
        }
    }
}

/// フォーカス変更イベントのコールバック
///
/// `WINEVENT_OUTOFCONTEXT` により、メッセージループのコンテキストで呼ばれる。
/// フォーカスが移動するたびにバイパス判定を更新し、キャッシュに書き込む。
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_OBJECT_FOCUS {
        return;
    }

    // Step 0: プロセスID・クラス名を取得
    let process_id = classify::get_window_process_id(hwnd);
    let class_name = classify::get_class_name_string(hwnd);

    // 同一フォアグラウンドウィンドウ内での TextInput → Undetermined 降格を防止。
    // Windows 11 では XAML インフラ等がフォーカスイベントを連続発火するが、
    // 同一ウィンドウ内の別サブコンポーネントが Undetermined でも、
    // 先に TextInput が確認されていれば維持する（OR 条件）。
    {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let fg = GetForegroundWindow();
        let current_kind = FocusKind::load(&crate::FOCUS_KIND);
        if current_kind == FocusKind::TextInput {
            // 既に TextInput — フォアグラウンドが変わっていなければ維持
            if let Some(f) = crate::FOCUS.get_ref() {
                if let Some((prev_pid, _)) = &f.last_focus_info {
                    let fg_pid = classify::get_window_process_id(fg);
                    if fg_pid == *prev_pid {
                        log::trace!(
                            "Keeping TextInput (same process {fg_pid}): class={class_name}"
                        );
                        return;
                    }
                }
            }
        }
    }

    // UIA 非同期結果のキャッシュ更新用に保存 + パターントラッカーをリセット
    if let Some(f) = crate::FOCUS.get_mut() {
        f.last_focus_info = Some((process_id, class_name.clone()));
        f.pattern_tracker.clear();
    }
    if let Some(kb) = crate::KEY_BUFFER.get_mut() {
        kb.passthrough_memory.clear();
        // Undetermined バッファリング中ならキャンセル
        if kb.undetermined_buffering {
            kb.undetermined_buffering = false;
            let _ = KillTimer(HWND::default(), crate::TIMER_UNDETERMINED_BUFFER);
            // バッファされたキーは破棄（フォーカスが変わったので無意味）
            kb.deferred_keys.clear();
        }
    }

    // Config オーバーライド（最高優先度、キャッシュより先に判定）
    if let Some(overrides) = crate::FOCUS.get_ref().map(|f| &f.overrides) {
        if !overrides.force_text.is_empty() || !overrides.force_bypass.is_empty() {
            let process_name = classify::get_process_name(process_id);
            for entry in &overrides.force_text {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_text ({process_name}, {class_name})",
                    );
                    FocusKind::TextInput.store(&crate::FOCUS_KIND);
                    return;
                }
            }
            for entry in &overrides.force_bypass {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_bypass ({process_name}, {class_name})",
                    );
                    FocusKind::NonText.store(&crate::FOCUS_KIND);
                    crate::invalidate_engine_context(ContextChange::FocusChanged);
                    return;
                }
            }
        }
    }

    // キャッシュヒット → 即座に結果を適用
    if let Some(cached) = crate::FOCUS
        .get_ref()
        .and_then(|f| f.cache.get(process_id, &class_name))
    {
        log::trace!("classify_focus: cache hit ({process_id}, {class_name}) → {cached:?}",);
        cached.store(&crate::FOCUS_KIND);
        if cached == FocusKind::NonText {
            crate::invalidate_engine_context(ContextChange::FocusChanged);
        }
        return;
    }

    // Step 1: 評価中は安全側（Undetermined）に設定
    FocusKind::Undetermined.store(&crate::FOCUS_KIND);

    // Step 2: バイパス状態を判定
    let result = classify::classify_focus(hwnd);
    let state = result.kind;

    // Step 3: キャッシュに格納し、FOCUS_KIND を更新
    if let Some(f) = crate::FOCUS.get_mut() {
        f.cache.insert(
            process_id,
            class_name.clone(),
            state,
            DetectionSource::Automatic,
        );
    }
    state.store(&crate::FOCUS_KIND);

    // Step 4: NonText ならエンジンの保留状態をフラッシュ
    if state == FocusKind::NonText {
        crate::invalidate_engine_context(ContextChange::FocusChanged);
    }

    // Step 5: Phase 1-2 で判定不能なら UIA 非同期判定をリクエスト
    if state == FocusKind::Undetermined {
        if let Some(tx) = crate::FOCUS.get_ref().and_then(|f| f.uia_sender.as_ref()) {
            let _ = tx.send(SendableHwnd(hwnd));
        }
        // auto-IME-OFF は行わない。Windows 11 では XAML インフラウィンドウが
        // 通常のウィンドウ切替時にも Undetermined フォーカスイベントを発火するた���、
        // auto-IME-OFF は正常なテキスト入力を阻害する。
        // ゲーム/gvim 保護は config.toml の force_bypass で対応する。
    }

    log::debug!(
        "Focus changed: hwnd={:?} class={} reason={} → {:?}",
        hwnd,
        class_name,
        result.reason,
        state,
    );
}

/// 手動フォーカスオーバーライドのトグル処理
///
/// 現在の `FocusKind` を反転し、学習キャッシュに `UserOverride` で記録する。
/// `NonText` への降格時はエンジンコンテキストを無効化し、バッファもクリアする。
///
/// Safety: シングルスレッドからのみ呼び出すこと
pub unsafe fn toggle_focus_override() {
    let current = FocusKind::load(&crate::FOCUS_KIND);
    let new_kind = if current == FocusKind::TextInput {
        FocusKind::NonText
    } else {
        FocusKind::TextInput
    };

    new_kind.store(&crate::FOCUS_KIND);

    // Update learning cache
    if let Some(f) = crate::FOCUS.get_mut() {
        if let Some((pid, cls)) = f.last_focus_info.as_ref() {
            f.cache
                .insert(*pid, cls.clone(), new_kind, DetectionSource::UserOverride);
        }
    }

    // If demoted to NonText, flush engine pending
    if new_kind == FocusKind::NonText {
        crate::invalidate_engine_context(ContextChange::FocusChanged);
    }

    // Clear any active buffers
    if let Some(kb) = crate::KEY_BUFFER.get_mut() {
        kb.deferred_keys.clear();
        kb.passthrough_memory.clear();
        kb.undetermined_buffering = false;
    }

    // バルーン通知を表示
    if let Some(tray) = crate::TRAY.get_mut() {
        tray.show_balloon(
            "awase",
            if new_kind == FocusKind::TextInput {
                "テキスト入力モードに切り替えました"
            } else {
                "バイパスモードに切り替えました"
            },
        );
    }

    let mode_str = if new_kind == FocusKind::TextInput {
        "TextInput (engine enabled)"
    } else {
        "NonText (engine bypassed)"
    };
    log::info!("Manual focus override: → {mode_str}");
}
