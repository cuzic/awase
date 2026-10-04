//! ADR-223 段階 0: 打鍵の時点で、フォーカス窓のスレッドの入力言語を読んで**記録する**(belief は変えない)。
//!
//! - 読む窓は、既存の非同期のフォーカス解決(`GetGUIThreadInfo` 経由、`focus_hwnd()`)が確定した「実際のフォーカス窓」。
//!   `EVENT_OBJECT_FOCUS` の WinEvent の hwnd は使わない(最後に届いたイベントが実際のフォーカスとは限らない。
//!   ADR-223 段階 0 の測定で、別プロセスの `InputSite` 窓の遅れて届いたイベントが最後になり、英語のスレッドを読んで誤検知した)。
//!   tid は保存せず、打鍵ごとに hwnd から引く。UWP のフレーム窓は子の `CoreWindow` のスレッドを読む(observer 側)。
//! - 窓が無い・自プロセスの窓(トレイ・ダイアログ)・tid や HKL が取れないときは「不明」(`None`)。
//!   awase 自身のスレッドの言語は読まない(ADR-223 D0・R4-M2)。
//! - ログ: 値(読み取り・belief)が変わったときだけ `[lang-check]`(info)、打鍵ごとは `[lang-check:key]`(debug、CI 用)。

use crate::observer::layout_observer::read_thread_language;
use crate::state::ime_event::HwndId;

#[derive(Default)]
pub(super) struct LangCheck {
    last_logged: Option<(Option<bool>, bool)>,
    keydowns: u64,
    mismatches: u64,
    unknown: u64,
}

impl LangCheck {
    fn observe_keydown(&mut self, vk: u16, belief_japanese: bool, hwnd: Option<HwndId>) {
        self.keydowns += 1;
        let crate::observer::layout_observer::ThreadLanguage {
            japanese: read,
            tid,
            lang_id: lang,
        } = read_thread_language(hwnd);
        match read {
            None => self.unknown += 1,
            Some(japanese) if japanese != belief_japanese => self.mismatches += 1,
            Some(_) => {}
        }
        tracing::debug!(
            "[lang-check:key] vk=0x{vk:02X} read={read:?} belief={belief_japanese} tid={tid} lang_id=0x{lang:04X}"
        );
        let state = (read, belief_japanese);
        if self.last_logged != Some(state) {
            self.last_logged = Some(state);
            tracing::info!(
                "[lang-check] read={read:?} belief={belief_japanese} tid={tid} lang_id=0x{lang:04X} \
                 (keydowns={} mismatches={} unknown={})",
                self.keydowns,
                self.mismatches,
                self.unknown
            );
        }
    }
}

impl super::Runtime {
    /// 取り込み口(`handle_hook_key_event` の冒頭)から、文字キー・親指キーの KeyDown で呼ぶ。記録のみ。
    ///
    /// Ctrl/Alt/Win を押している間のキーは読まない(PassThrough でエンジンは変換しない。ADR-223 D1)。
    pub(crate) fn lang_check_on_keydown(&mut self, event: &awase::types::RawKeyEvent) {
        if !matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
            return;
        }
        let m = event.modifier_snapshot;
        if m.ctrl || m.alt || m.win {
            return;
        }
        let belief = self.platform_state.ime.belief.is_japanese_ime();
        // 既存の非同期のフォーカス解決が確定した実際のフォーカス窓。まだ無い(0)ときは「不明」。
        let hwnd = self.focus_hwnd();
        let hwnd = (hwnd.0 != 0).then_some(hwnd);
        self.lang_check
            .observe_keydown(event.vk_code.0, belief, hwnd);
    }
}
