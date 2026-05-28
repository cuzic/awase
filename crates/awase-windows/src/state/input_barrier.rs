//! 入力 transaction (Barrier) (Step 4)
//!
//! `ctrl_bypass_hold: bool` のような boolean ガードを「入力 chord transaction」に
//! 置き換える。Ctrl+IME chord 中の二次 SetOpen を boolean フラグで skip するのではなく、
//! 「今は CtrlImeChord transaction の中である」と表現する。
//!
//! ## 設計原則
//!
//! - boolean フラグではなく "transaction" として表現する
//! - transaction は明示的な start / end イベントで区切る (chord_started / chord_ended)
//! - chord 中の Ctrl KeyUp は「状態変更イベント」ではなく「transaction 終了イベント」

use std::time::Instant;

use super::ime_event::{ChordKind, HwndId};

/// 入力 transaction の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputBarrier {
    /// Ctrl+IME chord (Ctrl+無変換 / Ctrl+変換 等)
    ///
    /// この transaction 中は二次 SetOpen 要求 (Ctrl KeyUp 由来等) を filter する。
    /// `target` は chord の目的 (false=IME OFF / true=IME ON)。
    CtrlImeChord {
        target: bool,
        kind: ChordKind,
        started_seq: u64,
        started_at: Instant,
    },

    /// フォーカス変更直後の遷移 transaction (Step 5)。
    ///
    /// この期間中、外部観測 (focus_probe / observer_poll) は信頼度を下げて扱う。
    /// `focus_settle_ms` (AppImePolicy 由来) 経過 or 最初のキー入力で解除。
    FocusTransition {
        to_hwnd: HwndId,
        started_seq: u64,
        started_at: Instant,
        settle_until: Instant,
    },
}

impl InputBarrier {
    /// この barrier が CtrlImeChord であるかを返す。
    #[must_use]
    pub const fn is_ctrl_ime_chord(&self) -> bool {
        matches!(self, Self::CtrlImeChord { .. })
    }

    /// この barrier が FocusTransition であるかを返す。
    #[must_use]
    pub const fn is_focus_transition(&self) -> bool {
        matches!(self, Self::FocusTransition { .. })
    }

    /// この barrier の chord kind を返す (CtrlImeChord 以外は None)。
    #[must_use]
    pub const fn chord_kind(&self) -> Option<ChordKind> {
        match self {
            Self::CtrlImeChord { kind, .. } => Some(*kind),
            Self::FocusTransition { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_ime_chord_detection() {
        let b = InputBarrier::CtrlImeChord {
            target: false,
            kind: ChordKind::CtrlMuhenkanImeOff,
            started_seq: 100,
            started_at: Instant::now(),
        };
        assert!(b.is_ctrl_ime_chord());
        assert_eq!(b.chord_kind(), Some(ChordKind::CtrlMuhenkanImeOff));
    }
}
