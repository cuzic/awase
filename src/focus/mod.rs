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

use std::sync::atomic::Ordering;

use awase::types::{ContextChange, FocusKind};
use awase::vk;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetOpenStatus};
use windows::Win32::UI::WindowsAndMessaging::KillTimer;

use crate::focus::cache::DetectionSource;
use crate::focus::uia::SendableHwnd;

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// 指定ウィンドウの IME を OFF にする。
pub(crate) unsafe fn set_ime_off(hwnd: HWND) {
    let himc = ImmGetContext(hwnd);
    if !himc.is_invalid() {
        let _ = ImmSetOpenStatus(himc, false);
        ImmReleaseContext(hwnd, himc);
        log::debug!("IME auto-OFF for hwnd={:?}", hwnd);
    }
}

/// 指定ウィンドウの IME を ON にする。
pub(crate) unsafe fn set_ime_on(hwnd: HWND) {
    let himc = ImmGetContext(hwnd);
    if !himc.is_invalid() {
        let _ = ImmSetOpenStatus(himc, true);
        ImmReleaseContext(hwnd, himc);
        log::debug!("IME auto-ON for hwnd={:?}", hwnd);
    }
}

/// フォーカス変更イベントフックを登録する
///
/// `WINEVENT_OUTOFCONTEXT` を使用するため、コールバックはメッセージループ上で実行される。
/// これにより `classify_focus` が非同期（キーイベントとは別タイミング）で呼ばれる。
pub(crate) fn install_focus_hook() {
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

    // Step 0: プロセスID・クラス名を取得し、キャッシュを検索
    let process_id = classify::get_window_process_id(hwnd);
    let class_name = classify::get_class_name_string(hwnd);

    // UIA 非同期結果のキャッシュ更新用に保存
    if let Some(last) = crate::LAST_FOCUS_INFO.get_mut() {
        *last = (process_id, class_name.clone());
    }

    // フォーカス変更時にパターントラッカーと記憶バッファをリセット
    if let Some(tracker) = crate::KEY_PATTERN_TRACKER.get_mut() {
        tracker.clear();
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
    if let Some(overrides) = crate::FOCUS_OVERRIDES.get_ref() {
        if !overrides.force_text.is_empty() || !overrides.force_bypass.is_empty() {
            let process_name = classify::get_process_name(process_id);
            for entry in &overrides.force_text {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_text ({}, {})",
                        process_name,
                        class_name
                    );
                    crate::FOCUS_KIND.store(FocusKind::TextInput as u8, Ordering::Release);
                    return;
                }
            }
            for entry in &overrides.force_bypass {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_bypass ({}, {})",
                        process_name,
                        class_name
                    );
                    crate::FOCUS_KIND.store(FocusKind::NonText as u8, Ordering::Release);
                    crate::invalidate_engine_context(ContextChange::FocusChanged);
                    return;
                }
            }
        }
    }

    // キャッシュヒット → 即座に結果を適用
    if let Some(cached) = crate::FOCUS_CACHE
        .get_ref()
        .and_then(|c| c.get(process_id, &class_name))
    {
        log::trace!(
            "classify_focus: cache hit ({}, {}) → {:?}",
            process_id,
            class_name,
            cached
        );
        crate::FOCUS_KIND.store(cached as u8, Ordering::Release);
        if cached == FocusKind::NonText {
            crate::invalidate_engine_context(ContextChange::FocusChanged);
        }
        return;
    }

    // Step 1: 評価中は安全側（Undetermined）に設定
    crate::FOCUS_KIND.store(FocusKind::Undetermined as u8, Ordering::Release);

    // Step 2: バイパス状態を判定
    let state = classify::classify_focus(hwnd);

    // Step 3: キャッシュに格納し、FOCUS_KIND を更新
    if let Some(cache) = crate::FOCUS_CACHE.get_mut() {
        cache.insert(process_id, class_name.clone(), state, DetectionSource::Automatic);
    }
    crate::FOCUS_KIND.store(state as u8, Ordering::Release);

    // Step 4: NonText ならエンジンの保留状態をフラッシュ
    if state == FocusKind::NonText {
        crate::invalidate_engine_context(ContextChange::FocusChanged);
    }

    // Step 5: Phase 1-2 で判定不能なら UIA 非同期判定をリクエスト
    if state == FocusKind::Undetermined {
        if let Some(tx) = crate::UIA_SENDER.get_ref() {
            let _ = tx.send(SendableHwnd(hwnd));
        }

        // Step 6: Undetermined + 非ブラウザ系 → IME OFF にして安全側に倒す
        // ブラウザ/Electron 系は UIA Phase 3 で正確に判定できるため、IME を維持する。
        // ゲーム/gvim 等の非ブラウザ系は UIA でも判定不能なため、IME OFF で保護する。
        // UIA が後から TextInput を返した場合は IME ON に復帰する（WM_FOCUS_KIND_UPDATE）。
        if !vk::is_browser_or_electron_class(&class_name) {
            set_ime_off(hwnd);
            crate::invalidate_engine_context(ContextChange::FocusChanged);
        }
    }

    log::debug!(
        "Focus changed: hwnd={:?} class={} → {:?}{}",
        hwnd,
        class_name,
        state,
        if state == FocusKind::Undetermined && !vk::is_browser_or_electron_class(&class_name) {
            " (IME auto-OFF)"
        } else {
            ""
        }
    );
}

/// 手動フォーカスオーバーライドのトグル処理
///
/// 現在の `FocusKind` を反転し、学習キャッシュに `UserOverride` で記録する。
/// `NonText` への降格時はエンジンコンテキストを無効化し、バッファもクリアする。
///
/// Safety: シングルスレッドからのみ呼び出すこと
pub(crate) unsafe fn toggle_focus_override() {
    let current = crate::FOCUS_KIND.load(Ordering::Acquire);
    let new_kind = if current == FocusKind::TextInput as u8 {
        FocusKind::NonText
    } else {
        FocusKind::TextInput
    };

    crate::FOCUS_KIND.store(new_kind as u8, Ordering::Release);

    // Update learning cache
    if let Some((pid, cls)) = crate::LAST_FOCUS_INFO.get_ref() {
        if let Some(cache) = crate::FOCUS_CACHE.get_mut() {
            cache.insert(*pid, cls.clone(), new_kind, DetectionSource::UserOverride);
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

    let mode_str = if new_kind == FocusKind::TextInput {
        "TextInput (engine enabled)"
    } else {
        "NonText (engine bypassed)"
    };
    log::info!("Manual focus override: → {}", mode_str);
}
