use awase::types::RawKeyEvent;

/// フックルーティング状態（キーペア追跡・再入ガード）
#[derive(Debug)]
pub struct HookRoutingState {
    /// Engine に送った KeyDown を記録するビットセット（VK 0-255）
    pub sent_to_engine: [u64; 4],
    /// TrackOnly で送った KeyDown を記録するビットセット
    pub track_only_keys: [u64; 4],
    /// 再入ガード
    pub in_callback: bool,
    /// IME 制御コンボ直後の Ctrl バイパス抑制フラグ。
    /// Ctrl+Henkan/Muhenkan 消費後、Ctrl がまだ押されている間の文字キーを
    /// ショートカットとして Bypass しない。Ctrl KeyUp で解除。
    pub(super) suppress_ctrl_bypass: bool,
}

impl HookRoutingState {
    /// `suppress_ctrl_bypass` フラグを設定する。
    ///
    /// IME 制御コンボ消費後に `true` をセットし、Ctrl KeyUp 時に `false` にリセットする。
    pub fn set_suppress_ctrl_bypass(&mut self, value: bool) {
        self.suppress_ctrl_bypass = value;
    }

    /// `suppress_ctrl_bypass` フラグを読み取る。
    pub fn suppress_ctrl_bypass(&self) -> bool {
        self.suppress_ctrl_bypass
    }
}

/// フック設定（親指キー VK コード）
#[derive(Debug, Copy, Clone)]
pub struct HookConfig {
    pub left_thumb_vk: u16,
    pub right_thumb_vk: u16,
}

/// IME 遷移ガード状態（IME トグルキー押下中のキーバッファリング）
#[derive(Debug)]
pub struct ImeGuardState {
    pub active: bool,
    pub deferred_keys: Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)>,
}
