//! IME 変換モードの管理コンポーネント。
//!
//! `Charset` と `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）のみを定義する。

pub(crate) use awase::engine::ConvMode;
#[cfg(windows)]
pub(crate) use awase::engine::Charset;

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
    /// フォーカス変更が最後に発生した時刻（`current_tick_ms` 値）。
    ///
    /// TsfNative ウィンドウ（WezTerm 等）はフォーカス直後の IMM conv 読み取りが
    /// TSF 実態を反映しないため、直後の idle-conv-check による conv_mode 更新を抑制するために使う。
    #[cfg(windows)]
    focus_changed_ms: std::cell::Cell<u64>,
}

impl Default for ConvModeMgr {
    fn default() -> Self {
        Self {
            mode: std::cell::Cell::new(None),
            #[cfg(windows)]
            focus_changed_ms: std::cell::Cell::new(0),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ConvModeMgr {
    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// 以後 1500ms 以内の HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する。
    #[cfg(windows)]
    pub(crate) fn on_focus_changed(&self) {
        self.focus_changed_ms.set(crate::hook::current_tick_ms());
    }

    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力する。
    /// フォーカス変更後 1500ms 以内に HankakuKatakana → ZenkakuKatakana へダウングレードしようと
    /// した場合は更新を抑制する（TsfNative ウィンドウの IMM/TSF 乖離対策）。
    pub(crate) fn update_from_conv(&self, conv: u32) {
        let new = ConvMode::from_u32(conv);
        let old = self.mode.get();
        if old != Some(new) {
            // TsfNative (WezTerm 等) はフォーカス直後に IMM conv が TSF mode を反映しない。
            // idle-conv-check が ZenKata を誤読して HanKata → ZenKata に書き換えると、
            // 次の send_eager_tsf_warmup が F1 のみ（ZenKata 用）を送信して TSF を破壊する。
            // フォーカス変更後 1500ms 以内のダウングレードは無視する。
            #[cfg(windows)]
            if old.map_or(false, |m| m.charset == Charset::HankakuKatakana)
                && new.charset == Charset::ZenkakuKatakana
            {
                let elapsed = crate::hook::current_tick_ms()
                    .saturating_sub(self.focus_changed_ms.get());
                if elapsed < 1500 {
                    log::debug!(
                        "[conv-mode] focus 後 {elapsed}ms: HanKata→ZenKata ダウングレード抑制 \
                         (conv=0x{conv:08X})"
                    );
                    return;
                }
            }
            log::info!(
                "[conv-mode] {} → {} (conv=0x{conv:08X})",
                old.map_or_else(|| "None".to_string(), |m| m.to_string()),
                new,
            );
            self.mode.set(Some(new));
        }
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }
}
