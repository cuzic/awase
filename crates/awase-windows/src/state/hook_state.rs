use awase::gate::{HoldingGate, SyncKeyGateEvent, SyncKeyGateMachine};
use awase::scanmap::KeyboardModel;
use awase::types::{RawKeyEvent, VkCode};

/// フック設定（親指キー VK コード）
#[derive(Debug, Copy, Clone)]
pub struct HookConfig {
    pub left_thumb_vk: VkCode,
    pub right_thumb_vk: VkCode,
    /// 物理キーボードモデル。`scan_to_pos` のテーブル選択に使う。
    pub keyboard_model: KeyboardModel,
    /// `GeneralConfig::left_alt_impersonates_thumb_key`。true かつエンジン ON 中は
    /// Left Alt を `left_thumb_vk` になりすまさせる。
    pub left_alt_impersonates_thumb_key: bool,
    /// `GeneralConfig::right_alt_impersonates_thumb_key`。true かつエンジン ON 中は
    /// Right Alt を `right_thumb_vk` になりすまさせる。
    pub right_alt_impersonates_thumb_key: bool,
}

/// IME 同期キー（変換/無変換/漢字 等）押下後のキー保留バッファ。
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
/// `crate::tsf::TsfGate` はフォーカス変更後の TSF probe ウォームアップ用のステートマシンで、
/// 別目的・別タイミング・別レイヤー（Engine 出力側）で動作する:
///
/// | | `SyncKeyGate`（本構造体） | [`TsfGate`](crate::tsf::TsfGate) |
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
    pub const fn new() -> Self {
        Self {
            inner: HoldingGate::new(SyncKeyGateMachine::new(), SYNC_KEY_CAPACITY),
        }
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
