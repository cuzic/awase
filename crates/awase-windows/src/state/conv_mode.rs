//! IME 変換モードの管理コンポーネント。
//!
//! `Charset` と `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）と `ConvModeAuthority`（所有権管理）を定義する。

pub(crate) use awase::engine::ConvMode;
#[cfg(windows)]
pub(crate) use awase::engine::Charset;

use super::TickMs;

// ─── 変換モード所有権 ────────────────────────────────────────────────────────────

/// IME 変換モードに対する awase の所有権状態。
///
/// awase エンジン ON/OFF・warmup 開始/終了など、conv mode 制御権の移譲時に更新される。
/// `executor` が `EngineStateChanged` で `WindowsPlatform::set_conv_mode_authority` を呼び、
/// `allows_conv_mutation()` の bool を conv mutation ゲートの唯一の実体である
/// `Output::conv_mutation_allowed`（Cell<bool>）へ push する。この enum 自体は状態を保持せず、
/// bool を導出するための分類器として使われる。
///
/// | 状態               | 説明                                                              |
/// |--------------------|-------------------------------------------------------------------|
/// | `Unknown`          | 初期状態。まだ所有権が確定していない。                            |
/// | `AwaseOwned`       | awase エンジン ON 中。conv mode を RomajiHiragana に lock する。  |
/// | `UserOwned`        | awase エンジン OFF / 非活性中。conv mode に一切触らない。         |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConvModeAuthority {
    /// 初期状態。所有権が誰にあるか不明（起動直後、エンジン状態未受信）。
    #[default]
    Unknown,
    /// awase エンジン ON 中。VK_DBE_HIRAGANA 等の conv mutation を許可する。
    AwaseOwned,
    /// awase 無効・エンジン OFF。IME conv mode に一切触らない。
    UserOwned,
    // 旧 TemporarilyUnowned（warmup 中の一時的な制御権返上）は 2026-07-06 の
    // 到達不能パス監査で撤去 — 構築サイトが一度も配線されなかった。
    // 必要になったら set_conv_mode_authority の呼び出し元とセットで再導入すること。
}

impl ConvModeAuthority {
    /// conv mode を変更する操作（VK_DBE_HIRAGANA 等・ImmSetConversionStatus）を許可するか。
    ///
    /// `AwaseOwned` のときのみ true。
    #[must_use]
    pub const fn allows_conv_mutation(self) -> bool {
        matches!(self, Self::AwaseOwned)
    }
}

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
    /// HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する期限（`current_tick_ms` 値）。
    ///
    /// 0 = 抑制なし。以下の契機で更新される:
    ///
    /// - フォーカス変更後 1500ms: TsfNative でフォーカス直後に IMM conv が TSF を反映しない
    /// - HanKata warmup (F1+F3) 送信後 500ms: TsfNative では F3 が IMM conv の FULLSHAPE ビットを
    ///   変更しないため、F1+F3 後の IMM 読み取りが ZenKata (0x0B) を返す副作用を遮断する
    #[cfg(windows)]
    suppress_zenkata_until_ms: std::cell::Cell<u64>,
}

impl Default for ConvModeMgr {
    fn default() -> Self {
        Self {
            mode: std::cell::Cell::new(None),
            #[cfg(windows)]
            suppress_zenkata_until_ms: std::cell::Cell::new(0),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code, unused_variables))]
impl ConvModeMgr {
    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// 以後 1500ms 以内の HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する。
    /// TsfNative ウィンドウはフォーカス直後に IMM conv が TSF mode を反映しないため。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    #[cfg(windows)]
    pub(crate) fn on_focus_changed(&self, tick_ms: TickMs) {
        let until = tick_ms.0 + 1500;
        if until > self.suppress_zenkata_until_ms.get() {
            self.suppress_zenkata_until_ms.set(until);
        }
    }

    /// HankakuKatakana 用 warmup VK (F1+F3) を送信したことを通知する。
    ///
    /// 以後 500ms 以内の HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する。
    /// TsfNative ウィンドウでは F3 (VK_DBE_SBCSCHAR) が IMM conv の FULLSHAPE ビットを変更しない
    /// ため、F1+F3 送信後の IMM 読み取りが ZenKata (0x0B) を返す副作用を遮断する。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    #[cfg(windows)]
    pub(crate) fn on_hankata_warmup_sent(&self, tick_ms: TickMs) {
        let until = tick_ms.0 + 500;
        if until > self.suppress_zenkata_until_ms.get() {
            self.suppress_zenkata_until_ms.set(until);
        }
    }

    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力し `true` を返す。
    /// HankakuKatakana → ZenkakuKatakana のダウングレードは `suppress_zenkata_until_ms` 期限内
    /// であれば無視する（フォーカス後 1500ms または HanKata warmup 後 500ms）。
    ///
    /// `now_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn update_from_conv(&self, conv: u32, now_ms: TickMs) -> bool {
        let new = ConvMode::from_u32(conv);
        let old = self.mode.get();
        if old != Some(new) {
            #[cfg(windows)]
            if old.map_or(false, |m| m.charset == Charset::HankakuKatakana)
                && new.charset == Charset::ZenkakuKatakana
            {
                let now = now_ms.0;
                let until = self.suppress_zenkata_until_ms.get();
                if now < until {
                    log::debug!(
                        "[conv-mode] HanKata→ZenKata ダウングレード抑制 \
                         (残り{}ms, conv=0x{conv:08X})",
                        until.saturating_sub(now)
                    );
                    return false;
                }
            }
            log::info!(
                "[conv-mode] {} → {} (conv=0x{conv:08X})",
                old.map_or_else(|| "None".to_string(), |m| m.to_string()),
                new,
            );
            self.mode.set(Some(new));
            true
        } else {
            false
        }
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }
}
