//! IME ガード + 遅延キー処理 + ハイブリッドバッファリング
//!
//! IME 制御キー直後のガード、Undetermined 時のバッファリング、
//! IME OFF 時の PassThrough 記憶を一元管理する。

use std::collections::VecDeque;

use awase::engine::input_tracker::PhysicalKeyState;
use awase::types::{KeyAction, KeyEventType, RawKeyEvent};
use timed_fsm::dispatch;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::focus::cache::DetectionSource;
use crate::ime::ImeProvider;

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// ガード中に遅延されたキーイベント + 物理キー状態のバッファ
    pub deferred_keys: Vec<(RawKeyEvent, PhysicalKeyState)>,
    /// IME OFF 時の記憶バッファ（PassThrough 済みキー）
    pub passthrough_memory: VecDeque<RawKeyEvent>,
    /// Undetermined + IME ON 時のバッファリング中フラグ
    pub undetermined_buffering: bool,
}

impl KeyBuffer {
    pub fn new() -> Self {
        Self {
            ime_transition_guard: false,
            deferred_keys: Vec::new(),
            passthrough_memory: VecDeque::new(),
            undetermined_buffering: false,
        }
    }

    /// ガードが有効かどうか
    pub const fn is_guarded(&self) -> bool {
        self.ime_transition_guard
    }

    /// ガードを設定/解除する
    pub const fn set_guard(&mut self, on: bool) {
        self.ime_transition_guard = on;
    }

    /// 遅延キーを追加する（物理キー状態のスナップショットも一緒に保存）
    pub fn push_deferred(&mut self, event: RawKeyEvent, phys: PhysicalKeyState) {
        self.deferred_keys.push((event, phys));
    }

    /// PassThrough 記憶にキーを追加する（上限 20）
    #[allow(dead_code)] // 将来のパターン検出再有効化で使用予定
    pub fn push_passthrough(&mut self, event: RawKeyEvent) {
        self.passthrough_memory.push_back(event);
        if self.passthrough_memory.len() > 20 {
            self.passthrough_memory.pop_front();
        }
    }

    /// 遅延キーを全て取り出す
    pub fn drain_deferred(&mut self) -> Vec<(RawKeyEvent, PhysicalKeyState)> {
        std::mem::take(&mut self.deferred_keys)
    }

    /// PassThrough 記憶を全て取り出す
    pub fn drain_passthrough(&mut self) -> Vec<RawKeyEvent> {
        std::mem::take(&mut self.passthrough_memory).into()
    }

    /// バッファリング中かどうか
    #[allow(dead_code)] // 将来拡張用に保持
    pub const fn is_buffering(&self) -> bool {
        self.undetermined_buffering
    }

    /// バッファリング状態を設定する
    #[allow(dead_code)] // 将来拡張用に保持
    pub const fn set_buffering(&mut self, on: bool) {
        self.undetermined_buffering = on;
    }

    /// 全状態をクリアする
    #[allow(dead_code)] // 将来拡張用に保持
    pub fn clear(&mut self) {
        self.ime_transition_guard = false;
        self.deferred_keys.clear();
        self.passthrough_memory.clear();
        self.undetermined_buffering = false;
    }
}

/// PassThrough 済みキーを BS で取り消し、エンジンで再処理する。
///
/// IME OFF + Undetermined 状態で PassThrough したキーを、
/// TextInput に昇格した後に正しく処理し直すために使用する。
pub unsafe fn retract_passthrough_memory() {
    let Some(app) = crate::APP.get_mut() else {
        return;
    };

    let keys = app.key_buffer.drain_passthrough();

    if keys.is_empty() {
        return;
    }

    log::debug!(
        "Retracting {} passthrough key(s) with BS + re-process",
        keys.len()
    );

    // BS を送信して PassThrough 済みの文字を取り消す
    {
        let mut bs_actions: Vec<KeyAction> = Vec::new();
        for _ in 0..keys.len() {
            bs_actions.push(KeyAction::Key(0x08)); // VK_BACK down
            bs_actions.push(KeyAction::KeyUp(0x08)); // VK_BACK up
        }
        app.output.send_keys(&bs_actions);
    }

    // エンジンで再処理
    for event in keys {
        let ime_active = app.ime.is_active() && app.ime.get_mode().is_kana_input();

        if ime_active {
            let phys = app.tracker.process(&event);
            let response = app.engine.on_event(event, &phys);
            let mut timer_runtime = crate::Win32TimerRuntime;
            let mut action_executor = crate::SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
        }
        // IME OFF のままなら再注入（元々 PassThrough だったので同じ結果）
        // この場合は BS 分が余計だが、IME OFF → パターン検出 → 昇格の流れでは
        // IME が ON になっていることが前提なので通常は engine 経由になる
    }
}

/// Undetermined + IME ON バッファリングのタイムアウトを開始する（初回バッファ時のみ）。
#[allow(dead_code)] // 将来の Undetermined バッファリング再有効化で使用予定
pub unsafe fn start_buffer_timeout_if_needed() {
    if let Some(app) = crate::APP.get_mut() {
        if !app.key_buffer.undetermined_buffering {
            app.key_buffer.undetermined_buffering = true;
            let _ = SetTimer(HWND::default(), crate::TIMER_UNDETERMINED_BUFFER, 300, None);
        }
    }
}

/// Undetermined + IME ON バッファリングのタイムアウト処理。
///
/// 300ms 以内にパターン検出されなかった場合、バッファされたキーを
/// エンジンで処理する（安全側: TextInput として扱う）。
pub unsafe fn handle_buffer_timeout() {
    let _ = KillTimer(HWND::default(), crate::TIMER_UNDETERMINED_BUFFER);
    let keys = if let Some(app) = crate::APP.get_mut() {
        app.key_buffer.undetermined_buffering = false;
        app.key_buffer.drain_deferred()
    } else {
        Vec::new()
    };

    if keys.is_empty() {
        return;
    }

    log::debug!(
        "Buffer timeout: promoting to TextInput and processing {} buffered key(s)",
        keys.len()
    );

    // タイムアウト → TextInput に昇格してエンジンで処理
    if let Some(app) = crate::APP.get_mut() {
        app.promote_to_text_input(
            DetectionSource::TypingPatternInferred,
            "buffer timeout (IME ON + Undetermined)",
        );
    }

    for (event, phys) in keys {
        if let Some(app) = crate::APP.get_mut() {
            let response = app.engine.on_event(event, &phys);
            let mut timer_runtime = crate::Win32TimerRuntime;
            let mut action_executor = crate::SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
        }
    }
}

/// IME 制御キー後に遅延されたキーを再処理する。
///
/// メッセージループから呼ばれるため、この時点で IME 制御キーは OS/IME に
/// 渡し済みで、IME 状態は最新に更新されている。
///
/// クロスプロセス API で実際の IME 状態を確認し、shadow state も同期する。
///
/// Safety: シングルスレッドからのみ呼び出すこと
pub unsafe fn process_deferred_keys() {
    // ガード解除 + バッファからキーを取り出す
    let keys = crate::APP.get_mut().map_or_else(Vec::new, |app| {
        app.key_buffer.set_guard(false);
        app.key_buffer.drain_deferred()
    });

    if keys.is_empty() {
        return;
    }

    log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

    // IME 状態キャッシュを更新（メッセージループ上なのでブロッキング OK）
    crate::APP.get_ref().map(|app| app.refresh_ime_state_cache());

    // キャッシュから IME 状態を取得
    let cached = crate::IME_STATE_CACHE.load(std::sync::atomic::Ordering::Acquire);
    let ime_on = match cached {
        0 => false,
        1 => true,
        _ => crate::APP.get_ref().map_or(true, |app| app.shadow_ime_on),
    };

    for (event, phys) in keys {
        if ime_on {
            // IME ON → エンジンで処理（push_deferred 時に保存した phys を使用）
            if let Some(app) = crate::APP.get_mut() {
                let response = app.engine.on_event(event, &phys);
                let mut timer_runtime = crate::Win32TimerRuntime;
                let mut action_executor = crate::SendInputExecutor;
                dispatch(&response, &mut timer_runtime, &mut action_executor);
            }
        } else {
            // IME OFF → キーをそのまま再注入（INJECTED_MARKER 付き）
            reinject_key(&event);
        }
    }
}

/// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）
///
/// INJECTED_MARKER 付きなのでフックに再捕捉されない。
pub unsafe fn reinject_key(event: &RawKeyEvent) {
    use crate::output::INJECTED_MARKER;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        VIRTUAL_KEY,
    };

    let is_keyup = matches!(
        event.event_type,
        KeyEventType::KeyUp | KeyEventType::SysKeyUp
    );

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code.0),
                wScan: event.scan_code.0 as u16,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE
                } else {
                    KEYEVENTF_SCANCODE
                },
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    };
    crate::win32::send_input_safe(&[input]);
}
