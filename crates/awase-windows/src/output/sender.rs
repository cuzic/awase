/// モード別出力ディスパッチのトレイト。
///
/// `send_keys()` が `InjectionMode` ごとに match を繰り返す代わりに、
/// このトレイトで一本化する。
pub(crate) trait InjectionSender {
    fn send_char(&self, ch: char);
    fn send_romaji(&self, romaji: &str);
    fn send_key_sequence(&self, s: &str) {
        for ch in s.chars() {
            self.send_char(ch);
        }
    }
    fn mode_label(&self) -> &'static str;
}

pub(crate) struct UnicodeSender<'a>(pub(crate) &'a super::Output);
pub(crate) struct VkSender<'a>(pub(crate) &'a super::Output);
pub(crate) struct TsfSender<'a>(pub(crate) &'a super::Output);

impl InjectionSender for UnicodeSender<'_> {
    fn send_char(&self, ch: char) { self.0.send_unicode_char(ch); }
    fn send_romaji(&self, romaji: &str) { self.0.send_romaji_as_unicode(romaji); }
    fn mode_label(&self) -> &'static str { "Unicode" }
}

impl InjectionSender for VkSender<'_> {
    fn send_char(&self, ch: char) { self.0.send_char_as_vk(ch); }
    fn send_romaji(&self, romaji: &str) { self.0.send_romaji_batched(romaji); }
    fn mode_label(&self) -> &'static str { "VK Batched (Chrome)" }
}

impl InjectionSender for TsfSender<'_> {
    fn send_char(&self, ch: char) { self.0.send_char_as_tsf(ch); }
    fn send_romaji(&self, romaji: &str) { self.0.send_romaji_as_tsf(romaji); }
    fn mode_label(&self) -> &'static str { "VK Sequential (TSF)" }
}

/// 出力注入モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InjectionMode {
    /// Unicode 直接注入（Win32/UWP デフォルト）
    Unicode,
    /// VK Batched 注入（Chrome/Edge/Electron — IME composition 経由）
    Vk,
    /// VK Sequential 注入（WezTerm — TSF 直結アプリ向け）
    Tsf,
}

/// `send_keys()` 1回分の出力セッション。
///
/// - `begin()` で `InjectionMode` を解決し `OutputActiveGuard` を取得する
/// - `sender()` で `InjectionSender` の動的ディスパッチオブジェクトを返す
/// - Drop 時に Guard が `OUTPUT_ACTIVE=false` + drain を自動実行する
pub(crate) struct OutputSession<'a> {
    pub(crate) output: &'a super::Output,
    pub(crate) mode: InjectionMode,
    pub(crate) _guard: super::OutputActiveGuard,
}

impl<'a> OutputSession<'a> {
    pub(crate) fn begin(output: &'a super::Output) -> Self {
        let mode = super::resolve_injection_mode();
        let _guard = super::OutputActiveGuard::begin();
        Self { output, mode, _guard }
    }

    pub(crate) fn sender(&self) -> Box<dyn InjectionSender + '_> {
        match self.mode {
            InjectionMode::Unicode => Box::new(UnicodeSender(self.output)),
            InjectionMode::Vk     => Box::new(VkSender(self.output)),
            InjectionMode::Tsf    => Box::new(TsfSender(self.output)),
        }
    }

    pub(crate) fn is_vk_mode(&self) -> bool {
        self.mode != InjectionMode::Unicode
    }
}
