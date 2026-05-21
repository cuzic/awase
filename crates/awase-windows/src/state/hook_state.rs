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
    pub suppress_ctrl_bypass: bool,
}

/// フック設定（親指キー VK コード）
#[derive(Debug)]
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
