//! IME ON/OFF 状態の確信度モデル。
//!
//! `shadow_ime_on: Cell<bool>` の代替。
//! 推測値（Intended）と実測値（Confirmed）を型で区別し、不明状態（Unknown）も表現する。

/// IME ON/OFF の確信度付き状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeOpenBelief {
    /// IMM 経由の実測値または GJI 観測で確認された状態
    Confirmed(bool),
    /// `apply_ime_open` で送信した意図（OS 到達は不確定）
    Intended(bool),
    /// 不明（起動直後・フォーカス変更直後など）
    Unknown,
}

impl ImeOpenBelief {
    /// 確信度にかかわらず bool 値を返す。Unknown は `None`。
    pub fn assume(self) -> Option<bool> {
        match self {
            Self::Confirmed(v) | Self::Intended(v) => Some(v),
            Self::Unknown => None,
        }
    }
}

/// IME ON/OFF の確信度付き状態を管理するストア。
#[derive(Debug)]
pub struct ImeBeliefStore {
    belief: std::cell::Cell<ImeOpenBelief>,
}

impl ImeBeliefStore {
    pub fn new() -> Self {
        Self { belief: std::cell::Cell::new(ImeOpenBelief::Unknown) }
    }

    /// IMM 経由の実測値または GJI 観測で確認された状態を記録する。
    pub fn record_observation(&self, value: bool) {
        log::debug!("[ime-belief] Confirmed({value})");
        self.belief.set(ImeOpenBelief::Confirmed(value));
    }

    /// `apply_ime_open` 後の意図（OS 側での処理は不確定）を記録する。
    pub fn record_intent(&self, value: bool) {
        log::debug!("[ime-belief] Intended({value})");
        self.belief.set(ImeOpenBelief::Intended(value));
    }

    /// フォーカス変更時に状態を Unknown に降格させる。
    ///
    /// 降格後に `record_observation` が呼ばれることで再確定する。
    pub fn invalidate_on_focus_change(&self) {
        log::debug!("[ime-belief] Unknown (focus changed)");
        self.belief.set(ImeOpenBelief::Unknown);
    }

    /// 現在の確信度付き状態を返す。
    pub fn current(&self) -> ImeOpenBelief {
        self.belief.get()
    }

    /// bool 値を返す。Unknown の場合は `false`（IME OFF）を仮定する。
    ///
    /// 既存の `shadow_ime_on() -> bool` との互換性のために提供。
    pub fn assume_or_false(&self) -> bool {
        self.belief.get().assume().unwrap_or(false)
    }
}
