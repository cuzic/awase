//! 公開 API 型定義: Decision, Effect, InputContext, EngineCommand

use std::time::Duration;

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, ImeCacheState, KeyAction, RawKeyEvent, VkCode};
use crate::yab::YabLayout;

use super::input_tracker::PhysicalKeyState;

// ── 副作用モデル（Effect / Decision / InputContext）──

/// 入力・出力に関する副作用
#[derive(Debug, Clone)]
pub enum InputEffect {
    /// キーアクションを SendInput で出力する
    SendKeys(Vec<KeyAction>),
    /// キーをそのまま再注入する（IME OFF 時の deferred key 用）
    ReinjectKey(RawKeyEvent),
}

/// タイマーに関する副作用
#[derive(Debug, Clone)]
pub enum TimerEffect {
    /// タイマーを設定する
    Set { id: usize, duration: Duration },
    /// タイマーをキャンセルする
    Kill(usize),
}

/// IME 制御に関する副作用
#[derive(Debug, Clone)]
pub enum ImeEffect {
    /// IME の ON/OFF を設定する (ImmSetOpenStatus)
    SetOpen(bool),
    /// IME キャッシュ更新を要求する (PostMessageW)
    RequestCacheRefresh,
}

/// UI に関する副作用
#[derive(Debug, Clone)]
pub enum UiEffect {
    /// エンジンの有効/無効が変わった
    EngineStateChanged { enabled: bool },
}

/// アプリケーション全体の副作用を表す宣言型。
/// Engine は Effect を返すだけで、実行は呼び出し側が行う。
#[derive(Debug, Clone)]
pub enum Effect {
    Input(InputEffect),
    Timer(TimerEffect),
    Ime(ImeEffect),
    Ui(UiEffect),
}

/// Engine の判断結果（副作用なし、値で消費される）。
///
/// `consumed: bool` ではなく enum で意味を固定する。
/// `PassThrough` なのに `SendKeys` が入る、といった不整合を型で防ぐ。
#[derive(Debug)]
pub enum Decision {
    /// キーを素通しする（副作用なし）
    PassThrough,
    /// キーを素通しするが副作用を伴う（例: IME トグルキーの pass-through + キャッシュ更新要求）
    PassThroughWith { effects: Vec<Effect> },
    /// キーを消費する（副作用あり or なし）
    Consume { effects: Vec<Effect> },
}

impl Decision {
    #[must_use]
    pub const fn pass_through() -> Self {
        Self::PassThrough
    }

    #[must_use]
    pub const fn pass_through_with(effects: Vec<Effect>) -> Self {
        Self::PassThroughWith { effects }
    }

    #[must_use]
    pub const fn consumed() -> Self {
        Self::Consume {
            effects: Vec::new(),
        }
    }

    #[must_use]
    pub const fn consumed_with(effects: Vec<Effect>) -> Self {
        Self::Consume { effects }
    }

    /// effects に追加する。PassThrough なら Consume に昇格。
    pub fn push_effect(&mut self, effect: Effect) {
        match self {
            Self::Consume { effects } | Self::PassThroughWith { effects } => {
                effects.push(effect);
            }
            Self::PassThrough => {
                *self = Self::Consume {
                    effects: vec![effect],
                };
            }
        }
    }

    /// effects への可変参照。PassThrough なら Consume に昇格して空 Vec を返す。
    #[must_use]
    pub fn effects_mut(&mut self) -> &mut Vec<Effect> {
        match self {
            Self::Consume { effects } | Self::PassThroughWith { effects } => effects,
            Self::PassThrough => {
                *self = Self::Consume {
                    effects: Vec::new(),
                };
                let Self::Consume { effects } = self else {
                    unreachable!()
                };
                effects
            }
        }
    }
}

/// Engine が判断に使う外部コンテキスト（読み取り専用）。
///
/// # 設計ルール
/// - OS 由来の「瞬間値」のみを含む（ポーリングで変わる可能性のある値）
/// - Engine 内部で保持できる永続状態は Engine 側に寄せる
/// - 副作用結果を反映したい場合は Effect 経由で表現する
/// - このフィールドを増やす前に、Engine 内部状態で代替できないか検討すること
#[derive(Debug)]
pub struct InputContext {
    /// メッセージループで更新される IME ON/OFF キャッシュ
    pub ime_cache: ImeCacheState,
}

/// IME 同期キー（トグル・ON・OFF）を集約する構造体
#[derive(Debug)]
pub struct ImeSyncKeys {
    pub toggle: Vec<VkCode>,
    pub on: Vec<VkCode>,
    pub off: Vec<VkCode>,
}

/// エンジン切替・IME 制御の特殊キーコンボを集約する構造体。
#[derive(Debug)]
pub struct SpecialKeyCombos {
    pub engine_on: Vec<ParsedKeyCombo>,
    pub engine_off: Vec<ParsedKeyCombo>,
    pub ime_on: Vec<ParsedKeyCombo>,
    pub ime_off: Vec<ParsedKeyCombo>,
}

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
/// OS 副作用は持たず、Engine メソッドがオーケストレーションを行う。
#[derive(Debug)]
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// ガード中に遅延されたキーイベント + 物理キー状態のバッファ
    pub deferred_keys: Vec<(RawKeyEvent, PhysicalKeyState)>,
}

impl Default for KeyBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyBuffer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ime_transition_guard: false,
            deferred_keys: Vec::new(),
        }
    }

    #[must_use]
    pub const fn is_guarded(&self) -> bool {
        self.ime_transition_guard
    }

    pub const fn set_guard(&mut self, on: bool) {
        self.ime_transition_guard = on;
    }

    pub fn push_deferred(&mut self, event: RawKeyEvent, phys: PhysicalKeyState) {
        self.deferred_keys.push((event, phys));
    }

    pub fn drain_deferred(&mut self) -> Vec<(RawKeyEvent, PhysicalKeyState)> {
        std::mem::take(&mut self.deferred_keys)
    }
}

/// Engine への外部コマンド
#[derive(Debug)]
pub enum EngineCommand {
    /// エンジンの有効/無効を切り替える
    ToggleEngine,
    /// 外部コンテキスト喪失（IME OFF、言語切替等）
    InvalidateContext(ContextChange),
    /// 配列を切り替える
    SwapLayout(YabLayout),
    /// IME 状態に追随する
    SyncImeState { ime_on: bool },
    /// IME ガードを設定する
    SetGuard(bool),
    /// 遅延キーをクリアする
    ClearDeferredKeys,
    /// 設定を再読み込みする
    ReloadKeys {
        special: SpecialKeyCombos,
        sync: ImeSyncKeys,
    },
    /// FSM パラメータを更新する
    UpdateFsmParams {
        threshold_ms: u32,
        confirm_mode: crate::config::ConfirmMode,
        speculative_delay_ms: u32,
    },
    /// n-gram モデルを設定する
    SetNgramModel(crate::ngram::NgramModel),
}
