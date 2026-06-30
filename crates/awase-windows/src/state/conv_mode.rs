//! IME 変換モードの管理コンポーネント。
//!
//! `Charset` と `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）のみを定義する。

pub(crate) use awase::engine::{Charset, ConvMode};

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug, Default)]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
}

impl ConvModeMgr {
    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力する。
    pub(crate) fn update_from_conv(&self, conv: u32) {
        let new = ConvMode::from_u32(conv);
        let old = self.mode.get();
        if old != Some(new) {
            log::info!(
                "[conv-mode] {:?} → {:?} (conv=0x{conv:08X})",
                old.as_ref().map(|m| format!("{:?}/{}", m.charset, if m.romaji { "roma" } else { "kana" })),
                format!("{:?}/{}", new.charset, if new.romaji { "roma" } else { "kana" }),
            );
            self.mode.set(Some(new));
        }
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }
}
