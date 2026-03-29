//! IME ガード + 遅延キー処理 + ハイブリッドバッファリング
//!
//! IME 制御キー直後のガード、Undetermined 時のバッファリング、
//! IME OFF 時の PassThrough 記憶を一元管理する。

use awase::types::RawKeyEvent;

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// ガード中に遅延されたキーイベントのバッファ
    pub deferred_keys: Vec<RawKeyEvent>,
    /// IME OFF 時の記憶バッファ（PassThrough 済みキー）
    pub passthrough_memory: Vec<RawKeyEvent>,
    /// Undetermined + IME ON 時のバッファリング中フラグ
    pub undetermined_buffering: bool,
}

impl KeyBuffer {
    pub fn new() -> Self {
        Self {
            ime_transition_guard: false,
            deferred_keys: Vec::new(),
            passthrough_memory: Vec::new(),
            undetermined_buffering: false,
        }
    }

    /// ガードが有効かどうか
    pub fn is_guarded(&self) -> bool {
        self.ime_transition_guard
    }

    /// ガードを設定/解除する
    pub fn set_guard(&mut self, on: bool) {
        self.ime_transition_guard = on;
    }

    /// 遅延キーを追加する
    pub fn push_deferred(&mut self, event: RawKeyEvent) {
        self.deferred_keys.push(event);
    }

    /// PassThrough 記憶にキーを追加する（上限 20）
    pub fn push_passthrough(&mut self, event: RawKeyEvent) {
        self.passthrough_memory.push(event);
        if self.passthrough_memory.len() > 20 {
            self.passthrough_memory.remove(0);
        }
    }

    /// 遅延キーを全て取り出す
    pub fn drain_deferred(&mut self) -> Vec<RawKeyEvent> {
        std::mem::take(&mut self.deferred_keys)
    }

    /// PassThrough 記憶を全て取り出す
    pub fn drain_passthrough(&mut self) -> Vec<RawKeyEvent> {
        std::mem::take(&mut self.passthrough_memory)
    }

    /// バッファリング中かどうか
    pub fn is_buffering(&self) -> bool {
        self.undetermined_buffering
    }

    /// バッファリング状態を設定する
    pub fn set_buffering(&mut self, on: bool) {
        self.undetermined_buffering = on;
    }

    /// 全状態をクリアする
    pub fn clear(&mut self) {
        self.ime_transition_guard = false;
        self.deferred_keys.clear();
        self.passthrough_memory.clear();
        self.undetermined_buffering = false;
    }
}
