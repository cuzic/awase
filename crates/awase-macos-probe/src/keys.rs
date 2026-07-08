//! 生キーイベント（keyDown/keyUp/flagsChanged）の取得・分類・レコード化。
//! Phase M0 の検証項目は project_macos_port_strategy.md の「推奨実装順序（Phase M0〜M6）」
//! を参照。
//!
//! # このモジュールの責務
//!
//! CGEventTap の callback（[`crate::tap`]）は C 境界・run loop スレッド・objc2 FFI に
//! 縛られていて Linux 上で単体テストできない。そこで「observ した生値を診断レコードへ
//! 変換・解釈する純粋ロジック」だけをここへ切り出す。[`crate::tap`] の callback は
//! CGEvent から生フィールドを読み ([`RawKeyEvent`]) 、周辺状態を集めて
//! ([`CaptureContext`])、[`build_event_record`] を呼ぶだけにする。これにより:
//!
//! - キャプチャ／解釈ロジック（イベント種別分類・修飾キー分解・左右/Caps 判定）が
//!   objc2 なしの純粋 std になり、任意ホストで `#[test]` できる（tap.rs 本体は不可）。
//! - `tap.rs` は FFI 読み出しと run loop 管理に専念する。
//!
//! JIS/US 差・Karabiner 併用・左右 modifier・Caps Lock の **実機確認** は #19（実機検証
//! ゲート）で行う。ここで完成させるのは解釈ロジックとその単体テストのみ。
//!
//! 生の数値（`event_type` / `flags`）は [`crate::report::EventRecord`] にそのまま焼き、
//! 人間可読な解釈（[`EventKind`] / [`ModifierState`]）は診断トレースやオフライン解析で
//! 使う。`EventRecord` のスキーマ（`report.rs` 所有）は変更しない。

use crate::report::EventRecord;

/// `CGEventType` の生値（Apple の `CGEventTypes.h` 由来。objc2 に依存せず持つことで
/// この解釈ロジックを非 macOS ホストでもコンパイル・テストできる）。
pub mod event_type_raw {
    /// `kCGEventKeyDown`。
    pub const KEY_DOWN: u32 = 10;
    /// `kCGEventKeyUp`。
    pub const KEY_UP: u32 = 11;
    /// `kCGEventFlagsChanged`。
    pub const FLAGS_CHANGED: u32 = 12;
}

/// キーイベントの種別。生の `CGEventType` を診断に必要な粒度へ畳む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// キー押下（`kCGEventKeyDown`）。
    KeyDown,
    /// キー解放（`kCGEventKeyUp`）。
    KeyUp,
    /// 修飾キーの状態変化（`kCGEventFlagsChanged`）。keycode は変化した修飾キー。
    FlagsChanged,
    /// 上記以外。tap の event mask は KeyDown/KeyUp/FlagsChanged のみなので通常は起きない
    /// フォールバック。生の種別値は [`EventRecord::event_type`] に保持されるため捨てる。
    Other,
}

impl EventKind {
    /// `CGEventType` 生値から分類する。
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            event_type_raw::KEY_DOWN => Self::KeyDown,
            event_type_raw::KEY_UP => Self::KeyUp,
            event_type_raw::FLAGS_CHANGED => Self::FlagsChanged,
            _ => Self::Other,
        }
    }

    /// 診断トレース用の短い名前。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeyDown => "keyDown",
            Self::KeyUp => "keyUp",
            Self::FlagsChanged => "flagsChanged",
            Self::Other => "other",
        }
    }
}

/// `CGEventFlags` のビットマスク（`CGEventTypes.h`）。修飾キーの論理状態を表す上位ビット群。
mod flag_mask {
    /// `kCGEventFlagMaskAlphaShift`（Caps Lock）。
    pub const ALPHA_SHIFT: u64 = 0x0001_0000;
    /// `kCGEventFlagMaskShift`。
    pub const SHIFT: u64 = 0x0002_0000;
    /// `kCGEventFlagMaskControl`。
    pub const CONTROL: u64 = 0x0004_0000;
    /// `kCGEventFlagMaskAlternate`（Option）。
    pub const ALTERNATE: u64 = 0x0008_0000;
    /// `kCGEventFlagMaskCommand`。
    pub const COMMAND: u64 = 0x0010_0000;
    /// `kCGEventFlagMaskNumericPad`。
    pub const NUMERIC_PAD: u64 = 0x0020_0000;
    /// `kCGEventFlagMaskHelp`。
    pub const HELP: u64 = 0x0040_0000;
    /// `kCGEventFlagMaskSecondaryFn`（fn）。
    pub const SECONDARY_FN: u64 = 0x0080_0000;
}

/// device-dependent な左右別 modifier ビット（IOKit `IOLLEvent.h` の `NX_DEVICE*` 群）。
/// CGEventFlags の下位ビットに現れ、左右どちらの修飾キーかを区別できる。
mod device_mask {
    pub const LCTRL: u64 = 0x0000_0001;
    pub const LSHIFT: u64 = 0x0000_0002;
    pub const RSHIFT: u64 = 0x0000_0004;
    pub const LCOMMAND: u64 = 0x0000_0008;
    pub const RCOMMAND: u64 = 0x0000_0010;
    pub const LALT: u64 = 0x0000_0020;
    pub const RALT: u64 = 0x0000_0040;
    pub const RCTRL: u64 = 0x0000_2000;
}

/// `CGEventFlags` を包んで、修飾キーの状態を名前付きで問い合わせられるようにする。
///
/// bool を多数フィールドに展開せず生 `flags` を保持し、アクセサでビットを見る
/// （`clippy::struct_excessive_bools` 回避 かつ 元値を保つ）。左右別（`left_*`/`right_*`）は
/// device-dependent ビットが立っているときのみ真になる。論理状態（[`Self::shift`] 等）は
/// 上位マスクで、左右いずれかが押されていれば真。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ModifierState {
    flags: u64,
}

impl ModifierState {
    /// 生の `CGEventFlags` 値から作る。
    #[must_use]
    pub const fn from_flags(flags: u64) -> Self {
        Self { flags }
    }

    const fn has(self, mask: u64) -> bool {
        self.flags & mask != 0
    }

    /// Caps Lock が有効か（`kCGEventFlagMaskAlphaShift`）。
    #[must_use]
    pub const fn caps_lock(self) -> bool {
        self.has(flag_mask::ALPHA_SHIFT)
    }
    /// Shift（左右いずれか）が押されているか。
    #[must_use]
    pub const fn shift(self) -> bool {
        self.has(flag_mask::SHIFT)
    }
    /// Control（左右いずれか）が押されているか。
    #[must_use]
    pub const fn control(self) -> bool {
        self.has(flag_mask::CONTROL)
    }
    /// Option/Alt（左右いずれか）が押されているか。
    #[must_use]
    pub const fn option(self) -> bool {
        self.has(flag_mask::ALTERNATE)
    }
    /// Command（左右いずれか）が押されているか。
    #[must_use]
    pub const fn command(self) -> bool {
        self.has(flag_mask::COMMAND)
    }
    /// fn（secondary function）が押されているか。
    #[must_use]
    pub const fn function(self) -> bool {
        self.has(flag_mask::SECONDARY_FN)
    }
    /// テンキー由来のイベントか（`kCGEventFlagMaskNumericPad`）。
    #[must_use]
    pub const fn numeric_pad(self) -> bool {
        self.has(flag_mask::NUMERIC_PAD)
    }
    /// Help 修飾（`kCGEventFlagMaskHelp`）。
    #[must_use]
    pub const fn help(self) -> bool {
        self.has(flag_mask::HELP)
    }

    /// 左 Shift（device-dependent ビット）。
    #[must_use]
    pub const fn left_shift(self) -> bool {
        self.has(device_mask::LSHIFT)
    }
    /// 右 Shift。
    #[must_use]
    pub const fn right_shift(self) -> bool {
        self.has(device_mask::RSHIFT)
    }
    /// 左 Control。
    #[must_use]
    pub const fn left_control(self) -> bool {
        self.has(device_mask::LCTRL)
    }
    /// 右 Control。
    #[must_use]
    pub const fn right_control(self) -> bool {
        self.has(device_mask::RCTRL)
    }
    /// 左 Option/Alt。
    #[must_use]
    pub const fn left_option(self) -> bool {
        self.has(device_mask::LALT)
    }
    /// 右 Option/Alt。
    #[must_use]
    pub const fn right_option(self) -> bool {
        self.has(device_mask::RALT)
    }
    /// 左 Command。
    #[must_use]
    pub const fn left_command(self) -> bool {
        self.has(device_mask::LCOMMAND)
    }
    /// 右 Command。
    #[must_use]
    pub const fn right_command(self) -> bool {
        self.has(device_mask::RCOMMAND)
    }
}

impl core::fmt::Debug for ModifierState {
    /// アクティブな修飾キー名だけを並べる（`flagsChanged` の観測を読みやすくする）。
    /// 左右を区別できる修飾キーには `(L)`/`(R)`/`(LR)` を付す。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut list = f.debug_list();
        for (active, left, right, name) in [
            (self.shift(), self.left_shift(), self.right_shift(), "shift"),
            (
                self.control(),
                self.left_control(),
                self.right_control(),
                "control",
            ),
            (
                self.option(),
                self.left_option(),
                self.right_option(),
                "option",
            ),
            (
                self.command(),
                self.left_command(),
                self.right_command(),
                "command",
            ),
        ] {
            if active {
                let side = match (left, right) {
                    (true, true) => "(LR)",
                    (true, false) => "(L)",
                    (false, true) => "(R)",
                    (false, false) => "",
                };
                list.entry(&format!("{name}{side}"));
            }
        }
        for (active, name) in [
            (self.caps_lock(), "caps"),
            (self.function(), "fn"),
            (self.numeric_pad(), "numpad"),
            (self.help(), "help"),
        ] {
            if active {
                list.entry(&name);
            }
        }
        list.finish()
    }
}

/// CGEvent から読み出した 1 イベント分の生フィールド。tap callback が FFI で埋める。
#[derive(Debug, Clone, Copy)]
pub struct RawKeyEvent {
    /// `CGEventType` 生値。
    pub event_type: u32,
    /// 仮想キーコード（`kCGKeyboardEventKeycode`）。
    pub keycode: u16,
    /// 修飾フラグ（`CGEventFlags` 生値）。
    pub flags: u64,
    /// オートリピート（`kCGKeyboardEventAutorepeat`）か。
    pub autorepeat: bool,
    /// CGEvent 自身のタイムスタンプ（`CGEventGetTimestamp` 生値）。
    pub cg_event_timestamp: u64,
    /// `kCGEventSourceUserData` 生値（synthetic 照合に使う）。
    pub source_user_data: i64,
}

impl RawKeyEvent {
    /// このイベントの種別。
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        EventKind::from_raw(self.event_type)
    }

    /// このイベントの修飾キー状態。
    #[must_use]
    pub const fn modifiers(&self) -> ModifierState {
        ModifierState::from_flags(self.flags)
    }
}

/// tap callback が観測時点に集める周辺状態（CGEvent 自体には無い情報）。
#[derive(Debug, Clone, Copy)]
pub struct CaptureContext {
    /// プロセス基準の単調増加クロック（ns）。
    pub monotonic_nanos: u64,
    /// UNIX epoch からの壁時計時刻（ns）。
    pub wall_clock_nanos: u64,
    /// callback を実行しているスレッド識別子。
    pub thread_id: u64,
    /// このイベントを観測した入力ストリーム世代（`TapHealth::generation`）。
    pub tap_generation: u64,
    /// 観測時点の focus epoch。
    pub focus_epoch: u64,
    /// active bundle の side-table index（未解決は [`EventRecord::NO_BUNDLE`]）。
    pub bundle_index: u32,
    /// 自己生成イベントと判定されたか（`SyntheticEventOrigin::is_self_event` の結果）。
    pub is_synthetic: bool,
}

/// 生イベント + 周辺状態を、ロガーへ push する [`EventRecord`] に組み立てる純粋関数。
///
/// tap callback はこれを呼んで結果を [`crate::report::EventLogger::try_push`] に渡す。
/// ここは I/O もロックもせず、`Copy` な値の組み替えだけを行う（ホットパス安全）。
#[must_use]
pub const fn build_event_record(raw: RawKeyEvent, ctx: CaptureContext) -> EventRecord {
    EventRecord {
        monotonic_nanos: ctx.monotonic_nanos,
        wall_clock_nanos: ctx.wall_clock_nanos,
        cg_event_timestamp: raw.cg_event_timestamp,
        thread_id: ctx.thread_id,
        tap_generation: ctx.tap_generation,
        focus_epoch: ctx.focus_epoch,
        source_user_data: raw.source_user_data,
        event_type: raw.event_type,
        keycode: raw.keycode,
        flags: raw.flags,
        bundle_index: ctx.bundle_index,
        autorepeat: raw.autorepeat,
        is_synthetic: ctx.is_synthetic,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_event_record, device_mask, flag_mask, CaptureContext, EventKind, ModifierState,
        RawKeyEvent,
    };
    use crate::report::EventRecord;

    #[test]
    fn event_kind_classifies_key_and_flags() {
        assert_eq!(EventKind::from_raw(10), EventKind::KeyDown);
        assert_eq!(EventKind::from_raw(11), EventKind::KeyUp);
        assert_eq!(EventKind::from_raw(12), EventKind::FlagsChanged);
        assert_eq!(EventKind::from_raw(1), EventKind::Other);
        assert_eq!(EventKind::from_raw(999), EventKind::Other);
        assert_eq!(EventKind::KeyDown.as_str(), "keyDown");
        assert_eq!(EventKind::FlagsChanged.as_str(), "flagsChanged");
        assert_eq!(EventKind::Other.as_str(), "other");
    }

    #[test]
    fn modifiers_decode_logical_masks() {
        let m = ModifierState::from_flags(flag_mask::SHIFT | flag_mask::COMMAND);
        assert!(m.shift());
        assert!(m.command());
        assert!(!m.control());
        assert!(!m.option());
        assert!(!m.caps_lock());

        let caps = ModifierState::from_flags(flag_mask::ALPHA_SHIFT);
        assert!(caps.caps_lock());
        assert!(!caps.shift());

        let fnkey = ModifierState::from_flags(flag_mask::SECONDARY_FN | flag_mask::NUMERIC_PAD);
        assert!(fnkey.function());
        assert!(fnkey.numeric_pad());
    }

    #[test]
    fn modifiers_distinguish_left_and_right() {
        // 論理 Shift + device-dependent 左 Shift のみ。
        let left = ModifierState::from_flags(flag_mask::SHIFT | device_mask::LSHIFT);
        assert!(left.shift());
        assert!(left.left_shift());
        assert!(!left.right_shift());

        let right = ModifierState::from_flags(flag_mask::CONTROL | device_mask::RCTRL);
        assert!(right.control());
        assert!(right.right_control());
        assert!(!right.left_control());

        // 左右 Command を同時に。
        let both = ModifierState::from_flags(
            flag_mask::COMMAND | device_mask::LCOMMAND | device_mask::RCOMMAND,
        );
        assert!(both.left_command() && both.right_command());
    }

    #[test]
    fn modifier_debug_lists_only_active() {
        // device-dependent ビットが無ければ side 注記なし。
        let m = ModifierState::from_flags(flag_mask::SHIFT | flag_mask::CONTROL);
        assert_eq!(format!("{m:?}"), "[\"shift\", \"control\"]");
        let none = ModifierState::from_flags(0);
        assert_eq!(format!("{none:?}"), "[]");
        // caps/fn は左右注記の対象外。
        let caps_fn = ModifierState::from_flags(flag_mask::ALPHA_SHIFT | flag_mask::SECONDARY_FN);
        assert_eq!(format!("{caps_fn:?}"), "[\"caps\", \"fn\"]");
    }

    #[test]
    fn modifier_debug_annotates_side() {
        let left = ModifierState::from_flags(flag_mask::SHIFT | device_mask::LSHIFT);
        assert_eq!(format!("{left:?}"), "[\"shift(L)\"]");
        let right = ModifierState::from_flags(flag_mask::CONTROL | device_mask::RCTRL);
        assert_eq!(format!("{right:?}"), "[\"control(R)\"]");
        let both = ModifierState::from_flags(
            flag_mask::COMMAND | device_mask::LCOMMAND | device_mask::RCOMMAND,
        );
        assert_eq!(format!("{both:?}"), "[\"command(LR)\"]");
    }

    #[test]
    fn raw_event_exposes_kind_and_modifiers() {
        let raw = sample_raw(12, 55, flag_mask::COMMAND | device_mask::LCOMMAND);
        assert_eq!(raw.kind(), EventKind::FlagsChanged);
        assert!(raw.modifiers().command());
        assert!(raw.modifiers().left_command());
    }

    #[test]
    fn build_record_copies_all_fields() {
        let raw = RawKeyEvent {
            event_type: 10,
            keycode: 0x20,
            flags: flag_mask::SHIFT,
            autorepeat: true,
            cg_event_timestamp: 999,
            source_user_data: -42,
        };
        let ctx = CaptureContext {
            monotonic_nanos: 111,
            wall_clock_nanos: 222,
            thread_id: 7,
            tap_generation: 3,
            focus_epoch: 9,
            bundle_index: 4,
            is_synthetic: true,
        };
        let rec = build_event_record(raw, ctx);
        assert_eq!(rec.event_type, 10);
        assert_eq!(rec.keycode, 0x20);
        assert_eq!(rec.flags, flag_mask::SHIFT);
        assert!(rec.autorepeat);
        assert_eq!(rec.cg_event_timestamp, 999);
        assert_eq!(rec.source_user_data, -42);
        assert_eq!(rec.monotonic_nanos, 111);
        assert_eq!(rec.wall_clock_nanos, 222);
        assert_eq!(rec.thread_id, 7);
        assert_eq!(rec.tap_generation, 3);
        assert_eq!(rec.focus_epoch, 9);
        assert_eq!(rec.bundle_index, 4);
        assert!(rec.is_synthetic);
    }

    #[test]
    fn build_record_preserves_no_bundle_sentinel() {
        let raw = sample_raw(11, 1, 0);
        let ctx = CaptureContext {
            monotonic_nanos: 0,
            wall_clock_nanos: 0,
            thread_id: 0,
            tap_generation: 0,
            focus_epoch: 0,
            bundle_index: EventRecord::NO_BUNDLE,
            is_synthetic: false,
        };
        let rec = build_event_record(raw, ctx);
        assert_eq!(rec.bundle_index, EventRecord::NO_BUNDLE);
        assert!(!rec.is_synthetic);
    }

    fn sample_raw(event_type: u32, keycode: u16, flags: u64) -> RawKeyEvent {
        RawKeyEvent {
            event_type,
            keycode,
            flags,
            autorepeat: false,
            cg_event_timestamp: 0,
            source_user_data: 0,
        }
    }
}
