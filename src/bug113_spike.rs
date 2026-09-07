//! BUG-113 残置症状（無変換キー単独タップで「@」再発）のスパイク検証専用。
//!
//! 案α（`AlreadyMatched` も随伴 warmup を skip+latch にする）と
//! 案β（delegate の no-op 強制再アサーションを抑止する）の効果を、
//! 1ビルドで A/B/A+B/baseline の4モードとして実機検証するための
//! 一時的な診断コード。**検証完了後は削除する**（本番ロジックではない）。
//!
//! `awase-windows` 側（`kp_stage_shadow_ime_toggle`）が TurnOn 方向の
//! 物理 IME キー単独タップを検出するたびに [`next_mode`] でモードを
//! 巡回し、コア（`engine.rs::apply_ime_open_request`）と
//! プラットフォーム層（`platform.rs::on_ime_applied`）の両方が
//! [`current_mode`] で同じ値を読んで挙動を切り替える。

use std::sync::atomic::{AtomicU8, Ordering};

static MODE: AtomicU8 = AtomicU8::new(0);
static COUNTER: AtomicU8 = AtomicU8::new(0);

/// bit0 = 案α（`on_ime_applied` の `AlreadyMatched` も送信せず latch のみにする）
pub const MODE_BIT_ALPHA: u8 = 0b01;
/// bit1 = 案β（delegate 経由の `ime_set_open_effects` no-op 強制再アサーションを抑止）
pub const MODE_BIT_BETA: u8 = 0b10;

/// 新しいエピソード（TurnOn 方向の物理 IME キー単独タップ）ごとに呼ぶ。
/// 0→1→2→3→0… と巡回し、選んだモードを返す。
pub fn next_mode() -> u8 {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mode = n % 4;
    MODE.store(mode, Ordering::Relaxed);
    mode
}

/// 直近の [`next_mode`] が選んだモードを読む（同一エピソード内で
/// core/platform 双方から参照する）。
pub fn current_mode() -> u8 {
    MODE.load(Ordering::Relaxed)
}

pub fn alpha_active() -> bool {
    current_mode() & MODE_BIT_ALPHA != 0
}

pub fn beta_active() -> bool {
    current_mode() & MODE_BIT_BETA != 0
}
