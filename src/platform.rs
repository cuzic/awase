//! Platform abstraction traits for future cross-platform support.
//!
//! This module defines platform-independent traits and types that
//! abstract over OS-specific keyboard hook, key injection, and
//! IME detection mechanisms.

// ─── IME Types (platform-independent) ────────────────────────

// 旧 EffectOrigin（EngineIntent / ObservationSync）は 2026-07-06 の到達不能パス
// 監査 B6 で撤去 — ObservationSync の生成元（DecisionOrigin::Unknown）が存在せず
// 恒に EngineIntent だったため、区別ごと畳んだ。

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImeOpenOutcome {
    /// 実 `SendInput`（VK送信）を伴って確実に設定できた（`GjiDirectStrategy`/
    /// `MsImeDirectStrategy`）。
    Applied,
    /// `ImmSetOpenStatus`（クロスプロセスIMM32 API）のみで設定できた。VK は
    /// 一切送っていない（ADR-167）。`ImmCrossProcessStrategy`（`Standard`
    /// プロファイル限定）専用。旧実装ではこのケースも`Applied`に潰していたが、
    /// 「`Applied` == 実SendInputを伴う」という
    /// [`should_send_accompanying_warmup`] の前提が`ImmCrossProcessStrategy`
    /// には成立しないため、この専用variantに分離した（ADR-149の随伴warmup
    /// ゲートがStandardプロファイル全体を無条件例外にしていたことで、
    /// `ImmCrossProcessStrategy`が`Failed`を返し`GjiDirectStrategy`へ
    /// フォールスルーした場合に随伴warmupが実送信の直後へ重複する欠陥が
    /// あった）。
    AppliedWithoutSendInput,
    /// shadow が既に目標状態のためスキップ
    AlreadyMatched,
    /// 設定に失敗（非日本語環境など）
    Failed,
    /// トグル操作が unsafe のため送信しなかった（shadow 信頼度不足・focus 直後等）
    ///
    /// F21/F22 や IMM32 SetOpen は冪等だが VK_KANJI は冪等でない。
    /// shadow が stale な状態でトグルすると意図と逆方向に反転する恐れがある。
    /// このケースでは apply は行われていないため applied_snapshot / state は更新しない。
    UnsafeToToggle,
    /// このフォーカス先は awase が IME actuation を所有しないため、
    /// 機構を一切試行しなかった（issue #136 / BUG-90、`AppImeProfile::InputRelay`）。
    /// `UnsafeToToggle` と同じく「送っていない」ので applied / belief を書かない。
    NotOwned,
    /// `issue_open_warrant()` が授権を発行しなかったため、機構を一切試行
    /// しなかった（ADR-090 §2.A A-2、根拠軸）。`NotOwned`（InputRelay、
    /// プロファイル所有権軸）とは別の理由なので別 variant にする——同じ
    /// 「送っていない」結果でも診断ログ・journal を読む側が「なぜ」を
    /// 区別できなくなる（ADR-106決定5が戒める「別目的の値を1つの機構に
    /// 混ぜる」の逆、ここでは逆に「別目的の結果を1つのvariantに混ぜない」）。
    /// `UnsafeToToggle`/`NotOwned`と同じく送っていないため applied / belief
    /// を書かない。
    Unwarranted,
}

impl ImeOpenOutcome {
    /// この outcome が「実際に何らかの機構で open 軸へ書き込んだ」ことを
    /// 意味するか（`Applied`/`AppliedWithoutSendInput`の2つ、
    /// ADR-167）。網羅 `match` で書くことで、将来 variant を追加した際に
    /// このヘルパーの呼び出し元全てがコンパイルエラーで追随を強制される
    /// （非網羅な `matches!` の書き直しを1箇所に集約する狙い）。
    #[must_use]
    pub const fn wrote_open_state(self) -> bool {
        match self {
            Self::Applied | Self::AppliedWithoutSendInput => true,
            Self::AlreadyMatched
            | Self::Failed
            | Self::UnsafeToToggle
            | Self::NotOwned
            | Self::Unwarranted => false,
        }
    }
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
    /// このメソッド自体は `awase-windows` の `runtime/ime_refresh.rs`
    /// （focus change 強制 OFF・drift correction の ImmCross 経路）で実際に
    /// 呼ばれている（2026-08-10、ADR-087 §5 Phase 3 item14 実 actuation 入口棚卸しで判明。
    /// ADR-158 TA1でこのメソッドをラップするだけだった`apply_ime_open`デフォルト実装を
    /// 削除したため、この段落の「以下のapply_ime_open」という言及も併せて削除した）。
    fn set_ime_open(&mut self, open: bool) -> bool;

    /// IME 状態キャッシュの非同期リフレッシュを要求する
    fn post_ime_refresh(&mut self);

    // ── トレイ ──

    /// エンジン有効/無効に応じてトレイアイコンを更新する
    fn update_tray(&mut self, enabled: bool);

    /// バルーン通知を表示する
    fn show_balloon(&mut self, title: &str, message: &str);

    /// 配列名をトレイに表示する
    fn set_tray_layout_name(&mut self, name: &str);
}

/// TSF / IMM composition 特有の platform フック（Windows 固有の意味論）。
///
/// `PlatformRuntime` のコア（キー出力・タイマー・トレイ・IME open/close 制御）とは分離し、
/// TSF composition warmup 特有の判定・フック（warm/cold 判定、passthrough / reinject 時の
/// composition 状態更新等）をまとめる。macOS/Linux 実装者は `PlatformRuntime` のコアだけを
/// 実装すればよく、本トレイトは全メソッドがデフォルト実装（no-op / `false` / `None` /
/// `u64::MAX`）を持つため、composition 機構が不要なら実装を省略できる。
///
/// Windows では `WindowsPlatform` が本トレイトを override し、`tsf` サブシステムに委譲する。
pub trait TsfComposition {
    /// composition 出力コンポーネントへの参照を返す。
    /// `None` の場合は composition 不要（macOS のシンプルモード等）。
    fn composition_output(&self) -> Option<&dyn CompositionOutput> {
        None
    }

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

    /// キー再注入時の composition 状態更新フック。
    ///
    /// confirm キー KeyDown の reinject 時に cold 化する（`VK_IME_ON` の eager warmup は送らない）。
    fn on_reinject_key(&mut self, _vk: crate::types::VkCode, _is_keydown: bool) {}
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
    fn wrote_open_state_distinguishes_real_send_from_immcross_only() {
        // ADR-167: 「何か書き込んだか」（wrote_open_state）は3つとも true、
        // 「実SendInputを伴ったか」（should_send_accompanying_warmupの否定）は
        // AppliedWithoutSendInputだけ異なる、という非対称性を固定する。
        assert!(ImeOpenOutcome::Applied.wrote_open_state());
        assert!(ImeOpenOutcome::AppliedWithoutSendInput.wrote_open_state());
        assert!(!ImeOpenOutcome::AlreadyMatched.wrote_open_state());
        assert!(!ImeOpenOutcome::Failed.wrote_open_state());
        assert!(!ImeOpenOutcome::UnsafeToToggle.wrote_open_state());
        assert!(!ImeOpenOutcome::NotOwned.wrote_open_state());
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

    // ── ImeDetector::is_active (デフォルト実装) ──
    //
    // 具体的な実装（awase-windows 等）を持たないため、最小のテスト用実装で
    // デフォルトメソッドのロジックそのものを検証する。

    struct FakeImeDetector(ImeMode);

    impl ImeDetector for FakeImeDetector {
        fn get_mode(&self) -> ImeMode {
            self.0
        }
        fn is_composing(&self) -> bool {
            false
        }
    }

    #[test]
    fn is_active_true_for_kana_modes() {
        assert!(FakeImeDetector(ImeMode::Hiragana).is_active());
        assert!(FakeImeDetector(ImeMode::Katakana).is_active());
        assert!(FakeImeDetector(ImeMode::HalfKatakana).is_active());
    }

    #[test]
    fn is_active_false_for_off_and_alphanumeric() {
        // is_active の本体が無条件 true に置換される、または
        // `!matches!(...)` の `!` が消えると、Off/Alphanumeric でも
        // true になってしまう。
        assert!(!FakeImeDetector(ImeMode::Off).is_active());
        assert!(!FakeImeDetector(ImeMode::Alphanumeric).is_active());
    }
}
