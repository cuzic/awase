//! IME リフレッシュのタイマー調停を担う `RefreshScheduler`。
//!
//! タイマーの実体（`TIMER_IME_REFRESH`）は `WindowsPlatform::timer` が保持し、
//! スケジューリングポリシー（遅延値・条件）の判断を `Runtime` がここに委譲する。
//!
//! 現フェーズではタイマー状態そのものは `WindowsPlatform::timer` が SSOT であるため
//! 本 struct が直接状態を持つ必要はない。`Runtime` の Facade メソッド
//! (`schedule_ime_refresh` / `reschedule_ime_refresh` / `spawn_ime_refresh`) が
//! このコンポーネントの担う責務を体現する。

/// IME リフレッシュスケジューリングの論理コンポーネント。
///
/// タイマー状態は `WindowsPlatform::timer` が SSOT として保持する。
/// 本 struct は IME リフレッシュスケジューリング責務の境界を明示するための構造体。
#[derive(Debug, Default)]
pub(crate) struct RefreshScheduler;
