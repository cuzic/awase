use awase::gate::{HoldingGate, SyncKeyGateEvent, SyncKeyGateMachine};
use awase::types::{RawKeyEvent, VkCode};

/// 直近の Ctrl+key ショートカット Bypass からの「stale Ctrl」しきい値（マイクロ秒）。
/// この時間内に親指キー (Henkan/Muhenkan) が到着した場合、Ctrl 修飾は
/// Ctrl release 遅延によるタイミング干渉とみなして無視する。
pub const CTRL_STALE_THRESHOLD_US: u64 = 50_000;

/// フックルーティング状態
#[derive(Debug)]
pub struct HookRoutingState {
    /// キーマップでインターセプト消費済みの VK を記録するビットセット（KeyUp も消費するため）
    pub(crate) intercept_consumed: [u64; 4],
    /// IME 制御コンボ直後の Ctrl バイパス抑制フラグ。
    /// Ctrl+Henkan/Muhenkan 消費後、Ctrl がまだ押されている間の文字キーを
    /// ショートカットとして Bypass しない。Ctrl KeyUp で解除。
    pub(crate) ctrl_bypass_hold: bool,
    /// 直近の Ctrl+key ショートカット Bypass の KeyDown タイムスタンプ（マイクロ秒）。
    /// Ctrl+I 等のショートカット直後に Ctrl release より先に親指キーが入った場合、
    /// Engine 側の Ctrl+無変換 → IME-OFF 誤マッチを抑制するための観測点。
    /// Ctrl KeyUp でクリア。
    pub(crate) last_ctrl_bypass_keydown_us: Option<u64>,
}

impl HookRoutingState {
    /// `ctrl_bypass_hold` フラグを設定する。
    ///
    /// IME 制御コンボ消費後に `true` をセットし、Ctrl KeyUp 時に `false` にリセットする。
    pub const fn set_ctrl_bypass_hold(&mut self, value: bool) {
        self.ctrl_bypass_hold = value;
    }

    /// `ctrl_bypass_hold` フラグを読み取る。
    #[must_use]
    pub const fn ctrl_bypass_hold(&self) -> bool {
        self.ctrl_bypass_hold
    }

    /// 直近の Ctrl+key Bypass KeyDown タイムスタンプを記録する。
    pub const fn record_ctrl_bypass_keydown(&mut self, ts_us: u64) {
        self.last_ctrl_bypass_keydown_us = Some(ts_us);
    }

    /// 直近の Ctrl+key Bypass KeyDown タイムスタンプをクリアする（Ctrl KeyUp 時）。
    pub const fn clear_ctrl_bypass_keydown(&mut self) {
        self.last_ctrl_bypass_keydown_us = None;
    }

    /// 現在時刻 `now_us` がしきい値以内に Ctrl Bypass KeyDown を観測しているか。
    /// 真なら、続いて来た親指キーの Ctrl 修飾は stale とみなして無視すべき。
    #[must_use]
    pub const fn is_ctrl_stale(&self, now_us: u64) -> bool {
        match self.last_ctrl_bypass_keydown_us {
            Some(ts) => now_us.saturating_sub(ts) < CTRL_STALE_THRESHOLD_US,
            None => false,
        }
    }

    /// キーマップでインターセプト済みの VK を記録する。
    pub const fn mark_intercept_consumed(&mut self, vk: VkCode) {
        let idx = (vk.0 as usize) / 64;
        let bit = 1u64 << ((vk.0 as usize) % 64);
        if idx < 4 { self.intercept_consumed[idx] |= bit; }
    }

    /// VK がインターセプト消費済みかどうかを返す。
    #[must_use]
    pub const fn is_intercept_consumed(&self, vk: VkCode) -> bool {
        let idx = (vk.0 as usize) / 64;
        let bit = 1u64 << ((vk.0 as usize) % 64);
        idx < 4 && (self.intercept_consumed[idx] & bit) != 0
    }

    /// インターセプト消費フラグをクリアする（KeyUp 処理後）。
    pub const fn clear_intercept_consumed(&mut self, vk: VkCode) {
        let idx = (vk.0 as usize) / 64;
        let bit = 1u64 << ((vk.0 as usize) % 64);
        if idx < 4 { self.intercept_consumed[idx] &= !bit; }
    }

    /// intercept_consumed ビットセットを全クリアする（panic_reset 用）。
    pub const fn reset_routing(&mut self) {
        self.intercept_consumed = [0u64; 4];
    }
}

/// フック設定（親指キー VK コード）
#[derive(Debug, Copy, Clone)]
pub struct HookConfig {
    pub left_thumb_vk: VkCode,
    pub right_thumb_vk: VkCode,
}

/// IME 同期キー（Henkan/Muhenkan/Kanji 等）押下後のキー保留バッファ。
///
/// # 役割
///
/// ユーザーが sync key を押した直後、OS が IME 状態を切り替える間に到着する後続キーを
/// 一時的にバッファする。sync key KeyUp 後に `process_deferred_keys()` で
/// IME 状態を再観測してから、バッファされたキーを新しい IME 状態で再処理する。
///
/// この処理はフックコールバック（`hook::apply_ime_gate`）で起動される。
///
/// # `TsfGate` との違い
///
/// `awase::tsf::TsfGate` はフォーカス変更後の TSF probe ウォームアップ用のステートマシンで、
/// 別目的・別タイミング・別レイヤー（Engine 出力側）で動作する:
///
/// | | `SyncKeyGate`（本構造体） | [`TsfGate`](awase::tsf::TsfGate) |
/// |--|--|--|
/// | トリガー | sync key（IME ON/OFF キー）KeyDown | フォーカス変更 |
/// | レイヤー | Platform 層（フックコールバック） | Output 層（TSF 注入直前） |
/// | 解除タイミング | sync key KeyUp + IME 再観測完了 | TSF/Bypass モード確定 or 500ms タイムアウト |
/// | 保留対象 | sync key 後に到着した全キー | フォーカス直後に到着した全キー |
///
/// 両者は独立に動作する（同時に active になることもある）。
type SyncKeyItem = (RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState);

/// `SyncKeyGate` 内のバッファ最大件数。
const SYNC_KEY_CAPACITY: usize = 10;

#[derive(Debug)]
pub struct SyncKeyGate {
    inner: HoldingGate<SyncKeyGateMachine, SyncKeyItem>,
}

impl SyncKeyGate {
    /// 初期状態（Inactive）でゲートを生成する。
    #[must_use]
    pub fn new() -> Self {
        Self { inner: HoldingGate::new(SyncKeyGateMachine::new(), SYNC_KEY_CAPACITY) }
    }

    /// ゲートをアクティブにする（sync key KeyDown 検出時）。
    pub fn activate(&mut self) {
        let _ = self.inner.on_event(SyncKeyGateEvent::Activate);
    }

    /// ゲートを非アクティブにし、保留していたキーを返す（sync key KeyUp / IME 再観測後）。
    pub fn deactivate(&mut self) -> Vec<SyncKeyItem> {
        self.inner.on_event(SyncKeyGateEvent::Deactivate).1
    }

    /// ゲートがアクティブかどうかを返す。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.inner.is_holding()
    }

    /// キーをバッファに追加する。`SYNC_KEY_CAPACITY` 件キャップ超過時は `false` を返しガードを解除する。
    ///
    /// `false` が返った場合、呼び出し元はガードを強制解除すること。
    pub fn try_push(
        &mut self,
        event: RawKeyEvent,
        phys: awase::engine::input_tracker::PhysicalKeyState,
    ) -> bool {
        self.inner.try_hold((event, phys))
    }

    /// バッファにキーが残っているかどうかを返す。
    #[must_use]
    pub fn has_deferred_keys(&self) -> bool {
        !self.inner.is_empty()
    }

    /// バッファをクリアする（`panic_reset` 用）。
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

impl Default for SyncKeyGate {
    fn default() -> Self {
        Self::new()
    }
}
