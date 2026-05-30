//! Platform abstraction traits for future cross-platform support.
//!
//! This module defines platform-independent traits and types that
//! abstract over OS-specific keyboard hook, key injection, and
//! IME detection mechanisms.

// ─── IME Types (platform-independent) ────────────────────────

/// `ImeEffect` の発生源を示す。
///
/// Platform 側は `origin` を見て「Engine がキー入力を処理した結果として
/// 意図的に IME 状態を変更する」のか「IME 観測値との同期（VK_KANJI 等の
/// フォールバック送信は不要）」なのかを区別できる。
///
/// `suppress_engine_state_key` のような手続き的フラグを将来的に廃止する
/// ための布石として導入される。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectOrigin {
    /// Engine がキー入力を処理した結果として意図的に IME 状態を変更する
    EngineIntent,
    /// IME 観測値との同期（エンジン内部の状態同期、VK_KANJI 等は不要）
    ObservationSync,
}

/// IME の変換モード（プラットフォーム非依存）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeMode {
    /// IME OFF（直接入力）
    Off,
    /// ひらがなモード
    Hiragana,
    /// カタカナモード
    Katakana,
    /// 半角カタカナモード
    HalfKatakana,
    /// 英数モード
    Alphanumeric,
}

impl ImeMode {
    /// かな入力モード（ひらがな / カタカナ / 半角カタカナ）かどうかを返す
    #[must_use]
    pub const fn is_kana_input(self) -> bool {
        matches!(self, Self::Hiragana | Self::Katakana | Self::HalfKatakana)
    }
}

// ─── Platform Abstraction Traits ─────────────────────────────

/// Abstraction for keyboard hook installation.
///
/// Platform implementations:
/// - Windows: `WH_KEYBOARD_LL` via `SetWindowsHookExW`
/// - macOS: `CGEventTap` (future)
/// - Linux: libinput / evdev (future)
pub trait KeyboardHook {
    /// Raw key event from the platform hook
    type RawEvent;

    /// Install the hook. The callback returns `true` if the event was consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform hook installation fails.
    fn install(&mut self, callback: Box<dyn FnMut(Self::RawEvent) -> bool>) -> anyhow::Result<()>;

    /// Uninstall the hook.
    fn uninstall(&mut self);
}

/// Abstraction for key injection.
///
/// Platform implementations:
/// - Windows: `SendInput`
/// - macOS: `CGEventPost` (future)
/// - Linux: uinput (future)
pub trait KeySender {
    /// Send a sequence of VK code key events (for IME romaji input)
    fn send_romaji(&self, romaji: &str);

    /// Send a Unicode character directly (for kana input mode)
    fn send_unicode(&self, ch: char);

    /// Send a literal string (each char as Unicode)
    fn send_literal(&self, s: &str);

    /// Send a virtual key code (`KeyDown` + `KeyUp`)
    fn send_vk(&self, vk: crate::types::VkCode);

    /// Send a virtual key event (`KeyDown` or `KeyUp`)
    fn send_vk_event(&self, vk: crate::types::VkCode, is_keyup: bool);
}

/// Abstraction for IME state detection.
///
/// Platform implementations:
/// - Windows: TSF + IMM32 hybrid
/// - macOS: `TISGetInputSourceProperty` (future)
/// - Linux: IBus / Fcitx D-Bus (future)
pub trait ImeDetector {
    /// Get the current IME mode
    fn get_mode(&self) -> ImeMode;

    /// Check if IME is active (Japanese input mode)
    fn is_active(&self) -> bool {
        let mode = self.get_mode();
        !matches!(mode, ImeMode::Off | ImeMode::Alphanumeric)
    }

    /// IME が未確定文字列を持っているか（変換中か）
    fn is_composing(&self) -> bool;
}

// ─── CompositionOutput Trait ─────────────────────────────────

/// composition context が cold になる理由（プラットフォーム非依存）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformColdReason {
    /// フォーカス変更
    FocusChange,
    /// Enter/Space/Escape（composition 確定・キャンセル）
    ConfirmKey,
    /// IME ON/OFF 操作
    ImeToggle,
}

/// IME composition context を管理する抽象インターフェース。
///
/// 各プラットフォーム（Windows TSF / macOS InputMethod / Linux IBus 等）が
/// このトレイトを実装することで、awase エンジンが OS に依存せず
/// composition 状態を操作できる。
///
/// # cold になるタイミング
/// awase エンジンは `mark_cold(reason)` で cold 化を通知する:
/// - `PlatformColdReason::FocusChange`: フォーカス変更
/// - `PlatformColdReason::ConfirmKey`: Enter/Space/Escape
/// - `PlatformColdReason::ImeToggle`: IME ON/OFF 操作
///
/// Windows TSF 実装は各 reason を `ColdReason::*` にマップする。
/// macOS/Linux 実装は自身のセマンティクスに従って実装する。
pub trait CompositionOutput {
    /// ローマ字文字列を composition 経由で送信する。
    fn send_romaji(&self, romaji: &str);

    /// かな文字を composition 経由で送信する。
    fn send_kana_char(&self, ch: char);

    /// composition context が warm（受け付け可能）かどうかを返す。
    fn is_composition_warm(&self) -> bool;

    /// composition context を cold 化する。
    fn mark_cold(&self, reason: PlatformColdReason);

    /// フォーカス変更を通知する（epoch インクリメント）。
    fn on_focus_changed(&self);
}

// ─── PlatformRuntime Trait ──────────────────────────────────

use std::time::Duration;

use crate::types::{KeyAction, RawKeyEvent};

/// `apply_ime_open` の実行結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeOpenOutcome {
    /// IMM 経由で確実に設定できた
    Applied,
    /// フォールバック（VK_KANJI 等）を送信済み。OS 処理完了まで不確定
    FallbackSent,
    /// shadow が既に目標状態のためスキップ
    AlreadyMatched,
    /// 設定に失敗（非日本語環境など）
    Failed,
    /// トグル操作が unsafe のため送信しなかった（shadow 信頼度不足・focus 直後等）
    ///
    /// F13/F14 や IMM32 SetOpen は冪等だが VK_KANJI は冪等でない。
    /// shadow が stale な状態でトグルすると意図と逆方向に反転する恐れがある。
    /// このケースでは apply は行われていないため applied_snapshot / state は更新しない。
    UnsafeToToggle,
}

/// フォアグラウンドウィンドウ情報（プラットフォーム非依存）
#[derive(Debug, Clone)]
pub struct ForegroundInfo {
    pub process_id: u32,
    pub class_name: String,
}

/// プラットフォーム固有の副作用実行インターフェース。
///
/// `DecisionExecutor` がこのトレイトを通じて OS 操作を行う。
/// Windows/macOS/Linux でそれぞれ実装を提供する。
pub trait PlatformRuntime {
    // ── キー出力 ──

    /// `KeyAction` のスライスを順に実行する
    fn send_keys(&mut self, actions: &[KeyAction]);

    /// 元のキーイベントを再注入する（IME OFF 時の遅延キー再生用）
    fn reinject_key(&mut self, event: &RawKeyEvent);

    // ── タイマー ──

    /// 指定 ID のタイマーを開始する
    fn set_timer(&mut self, id: usize, duration: Duration);

    /// 指定 ID のタイマーを停止する
    fn kill_timer(&mut self, id: usize);

    // ── IME 制御 ──

    /// IME の ON/OFF を設定する。成功時 true を返す。
    ///
    /// 廃止予定: `apply_ime_open` を使うこと。
    fn set_ime_open(&mut self, open: bool) -> bool;

    /// IME の ON/OFF を設定し、実行結果を返す。
    ///
    /// デフォルト実装は `set_ime_open` をラップする。
    /// プラットフォーム実装はオーバーライドしてフォールバック戦略を組み込める。
    fn apply_ime_open(&mut self, open: bool) -> ImeOpenOutcome {
        if self.set_ime_open(open) {
            ImeOpenOutcome::Applied
        } else {
            ImeOpenOutcome::Failed
        }
    }

    /// IME 状態キャッシュの非同期リフレッシュを要求する
    fn post_ime_refresh(&mut self);

    // ── トレイ ──

    /// エンジン有効/無効に応じてトレイアイコンを更新する
    fn update_tray(&mut self, enabled: bool);

    /// バルーン通知を表示する
    fn show_balloon(&mut self, title: &str, message: &str);

    /// 配列名をトレイに表示する
    fn set_tray_layout_name(&mut self, name: &str);

    /// composition 出力コンポーネントへの参照を返す。
    /// `None` の場合は composition 不要（macOS のシンプルモード等）。
    fn composition_output(&self) -> Option<&dyn CompositionOutput> {
        None
    }

    // ── Engine 状態変化時 IME モードキー送信 ──

    /// Engine ON/OFF 時に IME 制御キーを送信する。
    ///
    /// `applied` は直前に apply された IME 開閉状態（executor の `applied_snapshot` から渡す）。
    /// `Some(v)` で `v == enabled` なら apply_ime_open 済みとして mode key 送信をスキップできる。
    /// プラットフォームが IME モードキー送信をサポートしない場合は何もしない。
    fn send_engine_state_ime_key(&self, _enabled: bool, _applied: Option<bool>) {}

    // ── composition state クエリ / フック ──

    /// 最後のキー出力からの経過時間 (ms) を返す。一度も送信していなければ `u64::MAX`。
    fn output_in_flight_ms(&self) -> u64 {
        u64::MAX
    }

    /// TSF composition context が warm 状態かどうかを返す。
    fn is_composition_warm(&self) -> bool {
        false
    }

    /// 現在のフォーカスウィンドウが TSF 注入モードかどうかを返す。
    fn is_tsf_mode(&self) -> bool {
        false
    }

    /// IME apply 完了後の platform 状態更新フック。
    ///
    /// `applied_snapshot` 更新・latch・mark_cold・eager warmup を platform 内で処理する。
    /// executor は outcome を受け取ったら必ずこのメソッドを呼ぶこと。
    fn on_ime_applied(&mut self, _open: bool, _outcome: ImeOpenOutcome) {}

    /// キー通過（パススルー）時の composition 状態更新フック。
    ///
    /// F2+TSF mark_cold、confirm キー KeyDown の mark_cold を処理する。
    /// executor がキーを OS に通す直前（late path — output_guard_defer チェック後）に呼ぶ。
    ///
    /// 戻り値: `true` なら KeyUp タイミングで eager warmup を送るべき（warmup deferred）。
    /// `applied_ime_on`: executor が保持する最後の apply 済み IME 状態。eager warmup 判定に使う。
    fn on_passthrough_key(
        &mut self,
        _vk: crate::types::VkCode,
        _is_keydown: bool,
        _applied_ime_on: Option<bool>,
    ) -> bool {
        false
    }

    /// キー再注入時の composition 状態更新フック。
    ///
    /// F2-TSF deferred / confirm キー reinject の mark_cold + eager warmup を処理する。
    ///
    /// `applied_ime_on`: executor が保持する最後の apply 済み IME 状態。eager warmup 判定に使う。
    fn on_reinject_key(
        &mut self,
        _vk: crate::types::VkCode,
        _is_keydown: bool,
        _applied_ime_on: Option<bool>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_mode_is_kana_input() {
        assert!(!ImeMode::Off.is_kana_input());
        assert!(ImeMode::Hiragana.is_kana_input());
        assert!(ImeMode::Katakana.is_kana_input());
        assert!(ImeMode::HalfKatakana.is_kana_input());
        assert!(!ImeMode::Alphanumeric.is_kana_input());
    }

    #[test]
    fn ime_mode_debug_and_clone() {
        let mode = ImeMode::Hiragana;
        let cloned = mode;
        assert_eq!(mode, cloned);
        // Verify Debug is implemented
        let _debug = format!("{:?}", mode);
    }

    #[test]
    fn foreground_info_fields() {
        let info = ForegroundInfo {
            process_id: 42,
            class_name: "Notepad".to_string(),
        };
        assert_eq!(info.process_id, 42);
        assert_eq!(info.class_name, "Notepad");
    }
}
