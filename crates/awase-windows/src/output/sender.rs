use crate::tsf::probe_bridge::OutputActiveGuard;

/// 文字・ローマ字の送信口（Batched VK）。
///
/// 以前は注入モード（`InjectionMode::Vk`/`Tsf`）ごとの送信口をトレイトで切り替えていたが、
/// Tsf 注入経路の撤去（2026-10-22、ADR-0004 追記）で送信口は Batched VK の 1 つになった。
pub(crate) struct VkSender<'a>(pub &'a super::Output);

impl VkSender<'_> {
    pub(crate) fn send_char(&self, ch: char) {
        self.0.send_char_as_vk(ch);
    }
    pub(crate) fn send_romaji(&self, romaji: &str) {
        self.0.send_romaji_batched(romaji);
    }
    pub(crate) fn send_key_sequence(&self, s: &str) {
        for ch in s.chars() {
            self.send_char(ch);
        }
    }
}

/// `send_keys()` 1回分の出力セッション。
///
/// - `begin()` で `OutputActiveGuard` を取得する
/// - `sender()` で送信口を返す
/// - Drop 時に Guard が `OUTPUT_GATE.active=false` + drain を自動実行する
pub(crate) struct OutputSession<'a> {
    pub output: &'a super::Output,
    #[allow(dead_code)] // RAII: Drop が OUTPUT_GATE.active=false を実行する
    pub guard: OutputActiveGuard,
}

impl<'a> OutputSession<'a> {
    pub(crate) fn begin(output: &'a super::Output) -> Self {
        let guard = OutputActiveGuard::begin();
        Self { output, guard }
    }

    pub(crate) const fn sender(&self) -> VkSender<'_> {
        VkSender(self.output)
    }
}
