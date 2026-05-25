use awase::gate::{HoldingGate, SyncKeyGateEvent, SyncKeyGateMachine};
use awase::types::RawKeyEvent;

/// フックルーティング状態（キーペア追跡・再入ガード）
#[derive(Debug)]
pub struct HookRoutingState {
    /// Engine に送った KeyDown を記録するビットセット（VK 0-255）
    pub(crate) sent_to_engine: [u64; 4],
    /// TrackOnly で送った KeyDown を記録するビットセット
    pub(crate) track_only_keys: [u64; 4],
    /// キーマップでインターセプト消費済みの VK を記録するビットセット（KeyUp も消費するため）
    pub(crate) intercept_consumed: [u64; 4],
    /// 再入ガード
    pub(crate) in_callback: bool,
    /// IME 制御コンボ直後の Ctrl バイパス抑制フラグ。
    /// Ctrl+Henkan/Muhenkan 消費後、Ctrl がまだ押されている間の文字キーを
    /// ショートカットとして Bypass しない。Ctrl KeyUp で解除。
    pub(crate) ctrl_bypass_hold: bool,
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

    /// VK を Engine 送信済みビットセットに記録する。
    pub const fn mark_engine_sent(&mut self, vk: u16) {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx < 4 {
            self.sent_to_engine[idx] |= bit;
        }
    }

    /// VK を Engine 送信済みおよび TrackOnly 両ビットセットに記録する。
    pub const fn mark_track_only_sent(&mut self, vk: u16) {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx < 4 {
            self.sent_to_engine[idx] |= bit;
            self.track_only_keys[idx] |= bit;
        }
    }

    /// VK を Engine 送信済み・TrackOnly 両ビットセットから削除する。
    pub const fn clear_engine_sent(&mut self, vk: u16) {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx < 4 {
            self.sent_to_engine[idx] &= !bit;
            self.track_only_keys[idx] &= !bit;
        }
    }

    /// VK が Engine 送信済みかどうかを返す。
    #[must_use] 
    pub const fn is_engine_sent(&self, vk: u16) -> bool {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx >= 4 {
            return false;
        }
        (self.sent_to_engine[idx] & bit) != 0
    }

    /// VK が TrackOnly で記録されているかどうかを返す。
    #[must_use] 
    pub const fn is_track_only(&self, vk: u16) -> bool {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx >= 4 {
            return false;
        }
        (self.track_only_keys[idx] & bit) != 0
    }

    /// キーマップでインターセプト済みの VK を記録する。
    pub const fn mark_intercept_consumed(&mut self, vk: u16) {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx < 4 { self.intercept_consumed[idx] |= bit; }
    }

    /// VK がインターセプト消費済みかどうかを返す。
    #[must_use]
    pub const fn is_intercept_consumed(&self, vk: u16) -> bool {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        idx < 4 && (self.intercept_consumed[idx] & bit) != 0
    }

    /// インターセプト消費フラグをクリアする（KeyUp 処理後）。
    pub const fn clear_intercept_consumed(&mut self, vk: u16) {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx < 4 { self.intercept_consumed[idx] &= !bit; }
    }

    /// ルーティングビットセットを全クリアする（panic_reset 用）。
    pub const fn reset_routing(&mut self) {
        self.sent_to_engine = [0u64; 4];
        self.track_only_keys = [0u64; 4];
        self.intercept_consumed = [0u64; 4];
    }

    /// コールバック再入ガードを立てる。
    pub const fn enter_callback(&mut self) {
        self.in_callback = true;
    }

    /// コールバック再入ガードを下ろす。
    pub const fn leave_callback(&mut self) {
        self.in_callback = false;
    }

    /// コールバック再入ガードが立っているかどうかを返す。
    #[must_use] 
    pub const fn is_in_callback(&self) -> bool {
        self.in_callback
    }

    /// `GetAsyncKeyState` で OS のキー状態と同期し、
    /// 実際に離されているのにビットが残っているキーをクリアする。
    ///
    /// # Safety
    /// `GetAsyncKeyState` を呼び出す。メインスレッドから呼ぶこと。
    pub unsafe fn sync_with_os_key_state(&mut self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

        let mut cleared = 0u32;
        for idx in 0..4 {
            if self.sent_to_engine[idx] == 0 {
                continue;
            }
            let mut remaining = self.sent_to_engine[idx];
            while remaining != 0 {
                let bit_pos = remaining.trailing_zeros() as usize;
                let vk = (idx * 64 + bit_pos) as i32;
                let bit = 1u64 << bit_pos;
                if (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) == 0 {
                    self.sent_to_engine[idx] &= !bit;
                    self.track_only_keys[idx] &= !bit;
                    cleared += 1;
                }
                remaining &= remaining - 1;
            }
        }
        if cleared > 0 {
            log::debug!("sync_sent_to_engine: cleared {cleared} stale bit(s)");
        }
    }
}

/// フック設定（親指キー VK コード）
#[derive(Debug, Copy, Clone)]
pub struct HookConfig {
    pub left_thumb_vk: u16,
    pub right_thumb_vk: u16,
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
    /// `deactivate()` 時にドレインされたキーを `drain_all()` まで保持する。
    ///
    /// `deactivate()` は `HoldingGate` に `Deactivate` イベントを送り保留キーをドレインするが、
    /// 旧 API では `deactivate()` と `drain_all()` が別タイミングで呼ばれていた。
    /// その call site を維持するため、ドレイン結果は一度ここに stash しておき
    /// `drain_all()` で取り出す。
    pending_drain: Vec<SyncKeyItem>,
}

impl SyncKeyGate {
    /// 初期状態（Inactive）でゲートを生成する。
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: HoldingGate::new(SyncKeyGateMachine::new(), SYNC_KEY_CAPACITY),
            pending_drain: Vec::new(),
        }
    }

    /// ゲートをアクティブにする（sync key KeyDown 検出時）。
    pub fn activate(&mut self) {
        let _ = self.inner.on_event(SyncKeyGateEvent::Activate);
    }

    /// ゲートを非アクティブにする（sync key KeyUp 検出時 / IME 再観測後）。
    ///
    /// `HoldingGate` がドレインした保留キーは `pending_drain` に stash され、
    /// 次の `drain_all()` で取り出される。
    pub fn deactivate(&mut self) {
        let (_, drained) = self.inner.on_event(SyncKeyGateEvent::Deactivate);
        if !drained.is_empty() {
            // すでに pending_drain にキーがある場合は連結（通常は起きないが安全のため）。
            self.pending_drain.extend(drained);
        }
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

    /// バッファされた全キーを取り出して返す。
    ///
    /// `deactivate()` 経由でドレイン済みのキー（`pending_drain`）と、
    /// まだゲート内に残っているキー（`is_active()=true` のまま呼ばれた場合）を
    /// すべて結合して返す。
    pub fn drain_all(&mut self) -> Vec<SyncKeyItem> {
        let mut result = std::mem::take(&mut self.pending_drain);
        // ゲートが active のままなら、Deactivate イベントを送って残りもドレインする。
        if self.inner.is_holding() {
            let (_, drained) = self.inner.on_event(SyncKeyGateEvent::Deactivate);
            result.extend(drained);
        }
        result
    }

    /// バッファにキーが残っているかどうかを返す。
    #[must_use]
    pub fn has_deferred_keys(&self) -> bool {
        !self.pending_drain.is_empty() || !self.inner.is_empty()
    }

    /// バッファをクリアする（`panic_reset` 用）。
    pub fn clear(&mut self) {
        self.inner.clear();
        self.pending_drain.clear();
    }
}

impl Default for SyncKeyGate {
    fn default() -> Self {
        Self::new()
    }
}
