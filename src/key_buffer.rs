//! IME ガード + 遅延キー処理 + ハイブリッドバッファリング
//!
//! IME 制御キー直後のガード、Undetermined 時のバッファリング、
//! IME OFF 時の PassThrough 記憶を一元管理する。
//!
//! このモジュールは純粋なデータ構造のみを提供する。
//! OS 副作用（SendInput, SetTimer 等）や `crate::APP` アクセスは
//! `AppState` のメソッドに移動済み。

use std::collections::VecDeque;

use awase::engine::input_tracker::PhysicalKeyState;
use awase::types::RawKeyEvent;

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
