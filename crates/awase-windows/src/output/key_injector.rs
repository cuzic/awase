//! SendInput / Unicode / VK 送信責務を束ねる `KeyInjector`。
//!
//! `Output` は Facade として本構造体を保持し、低レベルのキー注入操作を委譲する。

use super::resolve::CharResolution;
use awase::kana_table::KanaTable;
use awase::types::VkCode;
use crate::tsf::output::{INJECTED_MARKER, TSF_MARKER, make_key_input_ex};
use crate::vk::{VK_DBE_HIRAGANA, VK_DBE_KATAKANA, VK_DBE_SBCSCHAR, VK_LSHIFT};
use itertools::Itertools as _;
use std::collections::HashMap;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

/// VK INPUT に使うマーカー種別。
///
/// - `Injected`: Chrome/VK モード（INJECTED_MARKER）
/// - `Tsf`:      WezTerm TSF モード（TSF_MARKER）
///
/// LSHIFT は常に INJECTED_MARKER のため、このマーカーは VK 本体にのみ適用する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VkMarker {
    Injected,
    Tsf,
}

impl VkMarker {
    pub(crate) fn make_input(self, vk: VkCode, is_keyup: bool) -> INPUT {
        match self {
            Self::Injected => make_key_input(vk, is_keyup),
            Self::Tsf => crate::tsf::output::make_tsf_key_input(vk, is_keyup),
        }
    }
}

/// INPUT 構造体を作成するヘルパー（INJECTED_MARKER 固定）
#[must_use]
pub(crate) const fn make_key_input(vk: VkCode, is_keyup: bool) -> INPUT {
    make_key_input_ex(vk, is_keyup, INJECTED_MARKER)
}

/// SendInput / Unicode / VK 送信責務を束ねるコンポーネント。
///
/// `Output` は Facade として本構造体を保持し、
/// 低レベルのキー注入操作を `KeyInjector` へ委譲する。
pub(crate) struct KeyInjector {
    /// ローマ字↔かな双方向テーブル（Unicode モード・Chrome VK モード両用）
    pub(super) kana_table: KanaTable,
    /// Chrome VK モード用: 記号→VK コードマッピング
    pub(super) symbol_to_vk: HashMap<char, (VkCode, bool)>,
    /// Unicode cold-start warmup: `send_unicode_char()` の送信を遅延させるフラグ
    pub(super) unicode_cold_defer: std::sync::atomic::AtomicBool,
    /// `unicode_cold_defer=true` 中に蓄積した Unicode 文字バッファ
    pub(super) unicode_cold_deferred: std::cell::RefCell<Vec<char>>,
}

impl KeyInjector {
    pub(crate) fn new() -> Self {
        Self {
            kana_table: KanaTable::build(),
            symbol_to_vk: crate::vk::build_symbol_to_vk(),
            unicode_cold_defer: std::sync::atomic::AtomicBool::new(false),
            unicode_cold_deferred: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// `send_unicode_char()` の遅延モードを ON/OFF する。
    pub(crate) fn set_unicode_cold_defer(&self, defer: bool) {
        self.unicode_cold_defer
            .store(defer, std::sync::atomic::Ordering::Relaxed);
    }

    /// 蓄積した Unicode deferred 文字を取り出してバッファをクリアする。
    pub(crate) fn take_unicode_cold_deferred(&self) -> Vec<char> {
        std::mem::take(&mut *self.unicode_cold_deferred.borrow_mut())
    }

    // ── 文字解決 ───────────────────────────────────────────────────────────────

    /// 文字の送信方法をルックアップテーブルで解決する。
    ///
    /// `send_char_as_tsf` / `send_char_as_vk` が共通で使う 3 段ルックアップ。
    #[must_use]
    pub(super) fn resolve_char(&self, ch: char) -> CharResolution<'_> {
        if let Some(romaji) = self.kana_table.romaji_for_kana(ch) {
            return CharResolution::Romaji(romaji);
        }
        if let Some(&(vk, shift)) = self.symbol_to_vk.get(&ch) {
            return CharResolution::Vk(vk, shift);
        }
        CharResolution::Unicode(ch)
    }

    // ── インスタンスメソッド ───────────────────────────────────────────────────

    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    #[expect(clippy::unused_self)]
    pub(super) fn send_key(&self, vk: VkCode, is_keyup: bool) {
        let input = make_key_input(vk, is_keyup);
        let _ = crate::win32::send_input_safe(&[input]);
    }

    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
    ///
    /// `unicode_cold_defer` フラグが立っている場合は実送信せず `unicode_cold_deferred` に蓄積する。
    pub(super) fn send_unicode_char(&self, ch: char) {
        if self.unicode_cold_defer.load(std::sync::atomic::Ordering::Relaxed) {
            self.unicode_cold_deferred.borrow_mut().push(ch);
            return;
        }
        let mut inputs = Vec::with_capacity(4);
        Self::push_unicode_char_inputs(&mut inputs, ch, INJECTED_MARKER);
        let _ = crate::win32::send_input_safe(&inputs);
    }

    /// Unicode char を直接送信する（defer モードを無視して即送信）。
    ///
    /// `FlushDeferredUnicodeChars` ハンドラが deferred chars を送信するために使う。
    pub(super) fn send_unicode_char_direct(&self, ch: char) {
        // FSM tick 時は unicode_cold_defer=false のため、通常の send_unicode_char で直接送信できる。
        self.send_unicode_char(ch);
    }

    /// PerKey モード: 1文字ずつ個別の SendInput 呼び出し
    #[expect(clippy::unused_self)]
    pub(super) fn send_romaji_per_key(&self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = crate::vk::ascii_to_vk(ch) {
                Self::send_vk_pair(vk, needs_shift, VkMarker::Injected);
            }
        }
    }

    /// Unicode モード: ローマ字→ひらがなに変換して Unicode 文字として直接送信
    pub(super) fn send_romaji_as_unicode(&self, romaji: &str) {
        if let Some(kana) = self.kana_table.kana_for_romaji(romaji) {
            self.send_unicode_char(kana);
            return;
        }
        // テーブルにない場合はフォールバック
        self.send_romaji_per_key(romaji);
    }

    // ── 静的ヘルパー（SendInput 操作） ────────────────────────────────────────

    /// `ch` を UTF-16 エンコードし、down/up ペアを `inputs` に追加する。
    pub(crate) fn push_unicode_char_inputs(inputs: &mut Vec<INPUT>, ch: char, marker: usize) {
        let mut buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut buf);
        for &cu in utf16.iter() {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: cu,
                        dwFlags: KEYEVENTF_UNICODE,
                        time: 0,
                        dwExtraInfo: marker,
                    },
                },
            });
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: cu,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: marker,
                    },
                },
            });
        }
    }

    /// VK の DOWN+UP ペアを（オプション shift 付きで）1回の SendInput で送信する。
    pub(crate) fn send_vk_pair(vk: VkCode, needs_shift: bool, marker: VkMarker) {
        let mut inputs = Vec::with_capacity(4);
        if needs_shift {
            inputs.push(make_key_input(VK_LSHIFT, false));
        }
        inputs.push(marker.make_input(vk, false));
        inputs.push(marker.make_input(vk, true));
        if needs_shift {
            inputs.push(make_key_input(VK_LSHIFT, true));
        }
        let _ = crate::win32::send_input_safe(&inputs);
    }

    /// 1 ラン分の INPUT を構築して送信し、送信した INPUT 数を返す。
    pub(crate) fn send_vk_run_batch(run: &[(VkCode, bool)], marker: VkMarker) -> usize {
        let mut inputs = Vec::with_capacity(run.len() * 4);
        for &(vk, needs_shift) in run {
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
            }
            inputs.push(marker.make_input(vk, false));
        }
        for &(vk, needs_shift) in run {
            inputs.push(marker.make_input(vk, true));
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
            }
        }
        let n = inputs.len();
        let _ = crate::win32::send_input_safe(&inputs);
        n
    }

    /// 同一 VK が連続する境界でランを分割する。
    pub(crate) fn split_vk_runs(vks: &[(VkCode, bool)]) -> Vec<&[(VkCode, bool)]> {
        if vks.is_empty() {
            return vec![];
        }
        let mut runs = Vec::new();
        let mut start = 0;
        for (i, w) in vks.windows(2).enumerate() {
            if w[0].0 == w[1].0 {
                runs.push(&vks[start..=i]);
                start = i + 1;
            }
        }
        runs.push(&vks[start..]);
        runs
    }

    /// ローマ字を即座にバッチ送信する（重畳順・VK ラン分割）。
    pub(crate) fn send_romaji_batch_immediate(romaji: &str, chars: &[(VkCode, bool)]) {
        for run in Self::split_vk_runs(chars) {
            let n = Self::send_vk_run_batch(run, VkMarker::Injected);
            log::debug!("[vk-send] romaji={romaji:?} batch {n} inputs");
        }
    }

    /// VK run 分割送信: 同一 VK 連続境界でバッチを分割して IME のオートリピート誤検出を回避する。
    pub(crate) fn send_vk_runs(chars: &[(VkCode, bool)], cold_seq: u32) {
        let runs = Self::split_vk_runs(chars);
        let total_runs = runs.len();

        for (run_idx, run) in runs.into_iter().enumerate() {
            let last_io = crate::tsf::observer::gji_last_io_ms();
            let run_gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            log::debug!(
                "[h1-run] cold={cold_seq} run={run_idx}/{total_runs} gji={run_gji_idle}ms vks=[{}]",
                run.iter()
                    .map(|&(v, s)| if s {
                        format!("S{v:02X}")
                    } else {
                        format!("{v:02X}")
                    })
                    .join(","),
            );
            Self::send_vk_run_batch(run, VkMarker::Tsf);
        }
    }

    /// VK run 分割送信（F2 leading）: F2 を先頭に付加して `send_vk_runs` と同様に送信する。
    pub(crate) fn send_vk_runs_with_leading_f2(chars: &[(VkCode, bool)], cold_seq: u32) {
        let mut f2_plus_chars = Vec::with_capacity(chars.len() + 1);
        f2_plus_chars.push((VK_DBE_HIRAGANA, false));
        f2_plus_chars.extend_from_slice(chars);
        let runs = Self::split_vk_runs(&f2_plus_chars);
        let total_runs = runs.len();
        for (run_idx, run) in runs.into_iter().enumerate() {
            let last_io = crate::tsf::observer::gji_last_io_ms();
            let run_gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            log::debug!(
                "[h1-run] cold={cold_seq} run={run_idx}/{total_runs} gji={run_gji_idle}ms vks=[{}] (f2-leading)",
                run.iter()
                    .map(|&(v, s)| if s {
                        format!("S{v:02X}")
                    } else {
                        format!("{v:02X}")
                    })
                    .join(","),
            );
            Self::send_vk_run_batch(run, VkMarker::Tsf);
        }
    }

    /// VK run 分割送信（カタカナ warmup 選択）: hint に応じた先頭ウォームアップ VK を使う。
    pub(crate) fn send_vk_runs_with_leading_warmup(
        chars: &[(VkCode, bool)],
        cold_seq: u32,
        charset: crate::state::Charset,
    ) {
        use crate::state::Charset;

        let (leading_label, leading_vks): (&str, Vec<(VkCode, bool)>) = match charset {
            Charset::ZenkakuKatakana => ("f1-leading", vec![(VK_DBE_KATAKANA, false)]),
            Charset::HankakuKatakana => (
                "f1+f3-leading",
                vec![(VK_DBE_KATAKANA, false), (VK_DBE_SBCSCHAR, false)],
            ),
            _ => ("f2-leading", vec![(VK_DBE_HIRAGANA, false)]),
        };

        let mut warmup_plus_chars = Vec::with_capacity(chars.len() + leading_vks.len());
        warmup_plus_chars.extend_from_slice(&leading_vks);
        warmup_plus_chars.extend_from_slice(chars);
        let runs = Self::split_vk_runs(&warmup_plus_chars);
        let total_runs = runs.len();

        for (run_idx, run) in runs.into_iter().enumerate() {
            let last_io = crate::tsf::observer::gji_last_io_ms();
            let run_gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            log::debug!(
                "[h1-run] cold={cold_seq} run={run_idx}/{total_runs} gji={run_gji_idle}ms vks=[{}] ({leading_label})",
                run.iter()
                    .map(|&(v, s)| if s {
                        format!("S{v:02X}")
                    } else {
                        format!("{v:02X}")
                    })
                    .join(","),
            );
            Self::send_vk_run_batch(run, VkMarker::Tsf);
        }
    }

    /// probe 完了後に deferred_vks を romaji の直後に送出する。
    pub(crate) fn send_deferred_probe_vks_from(vks: &[(VkCode, bool)], marker: VkMarker) {
        if vks.is_empty() {
            return;
        }
        log::debug!(
            "[tsf-probe] deferred {} VK(s) を romaji 直後に送出 ({marker:?})",
            vks.len()
        );
        for run in Self::split_vk_runs(vks) {
            Self::send_vk_run_batch(run, marker);
        }
    }
}
