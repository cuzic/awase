use std::collections::HashMap;

use awase::config::OutputMode;
use awase::types::{AppKind, KeyAction, SpecialKey};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

/// 自己注入マーカー（"KEYM" = 0x4B45_594D）
pub const INJECTED_MARKER: usize = 0x4B45_594D;

/// VK_LSHIFT の仮想キーコード
const VK_LSHIFT: u16 = 0xA0;

/// 出力で VK 経路を使うべきかを現在のフォーカスから判定する。
///
/// 優先順位:
///   1. config の `focus_overrides.force_vk` にマッチ → VK
///   2. AppKind::Chrome → VK
///   3. それ以外 (Win32 / Uwp) → Unicode
fn resolve_use_vk() -> bool {
    unsafe {
        let Some(app) = crate::APP.get_ref() else {
            return false;
        };
        if let Some((pid, class)) = app.executor.platform.focus.last_focus_info.as_ref() {
            if crate::runtime::is_force_vk(
                &app.executor.platform.focus.overrides,
                *pid,
                class,
            ) {
                return true;
            }
        }
        app.platform_state.app_kind == AppKind::Chrome
    }
}

/// ASCII 文字を対応する VK コードに変換する。
const fn ascii_to_vk(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => Some((0x41 + (ch as u16 - 'a' as u16), false)),
        'A'..='Z' => Some((0x41 + (ch as u16 - 'A' as u16), true)),
        '0'..='9' => Some((0x30 + (ch as u16 - '0' as u16), false)),
        '-' => Some((0xBD, false)),
        '.' => Some((0xBE, false)),
        ',' => Some((0xBC, false)),
        '/' => Some((0xBF, false)),
        _ => None,
    }
}

/// 半角 ASCII 文字をキーシーケンス用に VK コードに変換する。
/// `ascii_to_vk` より広い範囲の記号を対応する。JIS キーボード前提。
#[allow(dead_code)]
const fn ascii_to_vk_extended(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => Some((0x41 + (ch as u16 - 'a' as u16), false)),
        'A'..='Z' => Some((0x41 + (ch as u16 - 'A' as u16), true)),
        '0' => Some((0x30, false)),
        '1' => Some((0x31, false)),
        '2' => Some((0x32, false)),
        '3' => Some((0x33, false)),
        '4' => Some((0x34, false)),
        '5' => Some((0x35, false)),
        '6' => Some((0x36, false)),
        '7' => Some((0x37, false)),
        '8' => Some((0x38, false)),
        '9' => Some((0x39, false)),
        // Shifted digits (JIS keyboard)
        '!' => Some((0x31, true)),  // Shift+1
        '"' => Some((0x32, true)),  // Shift+2
        '#' => Some((0x33, true)),  // Shift+3
        '$' => Some((0x34, true)),  // Shift+4
        '%' => Some((0x35, true)),  // Shift+5
        '&' => Some((0x36, true)),  // Shift+6
        '\'' => Some((0x37, true)), // Shift+7
        '(' => Some((0x38, true)),  // Shift+8
        ')' => Some((0x39, true)),  // Shift+9
        // Symbols (JIS keyboard)
        '-' => Some((0xBD, false)),  // VK_OEM_MINUS
        '=' => Some((0xBD, true)),   // Shift+- (JIS: =)
        '^' => Some((0xDE, false)),  // VK_OEM_7 (JIS: ^)
        '~' => Some((0xDE, true)),   // Shift+^ (JIS: ~)
        '\\' => Some((0xE2, false)), // VK_OEM_102 (JIS: ＼)
        '|' => Some((0xDC, true)),   // Shift+¥ (JIS: |)
        '@' => Some((0xC0, false)),  // VK_OEM_3 (JIS: @)
        '`' => Some((0xC0, true)),   // Shift+@ (JIS: `)
        '[' => Some((0xDB, false)),  // VK_OEM_4
        '{' => Some((0xDB, true)),   // Shift+[
        ']' => Some((0xDD, false)),  // VK_OEM_6
        '}' => Some((0xDD, true)),   // Shift+]
        ';' => Some((0xBB, false)),  // VK_OEM_PLUS (JIS: ;)
        '+' => Some((0xBB, true)),   // Shift+; (JIS: +)
        ':' => Some((0xBA, false)),  // VK_OEM_1 (JIS: :)
        '*' => Some((0xBA, true)),   // Shift+: (JIS: *)
        ',' => Some((0xBC, false)),  // VK_OEM_COMMA
        '<' => Some((0xBC, true)),   // Shift+,
        '.' => Some((0xBE, false)),  // VK_OEM_PERIOD
        '>' => Some((0xBE, true)),   // Shift+.
        '/' => Some((0xBF, false)),  // VK_OEM_2
        '?' => Some((0xBF, true)),   // Shift+/
        '_' => Some((0xE2, true)),   // Shift+＼ (JIS: _)
        _ => None,
    }
}

/// 全角文字を半角に変換する。
/// 全角 ASCII 範囲 (U+FF01..U+FF5E) に該当する場合、対応する半角文字を返す。
#[allow(dead_code)]
const fn fullwidth_to_halfwidth(ch: char) -> Option<char> {
    let cp = ch as u32;
    // 全角 ASCII: U+FF01 ('！') .. U+FF5E ('～')
    // 対応する半角: U+0021 ('!') .. U+007E ('~')
    if cp >= 0xFF01 && cp <= 0xFF5E {
        // const fn では char::from_u32 が使えないため直接変換
        let half_cp = cp - 0xFEE0;
        if half_cp <= 0x7F {
            Some(half_cp as u8 as char)
        } else {
            None
        }
    } else {
        None
    }
}

/// 文字をキーシーケンス用の VK コードに変換する。
/// 全角文字は半角に変換してから `ascii_to_vk_extended` で対応する。
#[allow(dead_code)]
fn char_to_key_sequence(ch: char) -> Option<(u16, bool)> {
    // まず全角→半角変換を試みる
    let half = fullwidth_to_halfwidth(ch).unwrap_or(ch);
    ascii_to_vk_extended(half)
}

/// SpecialKey を Windows VK コードに変換する
const fn special_key_to_vk(sk: SpecialKey) -> u16 {
    match sk {
        SpecialKey::Backspace => 0x08, // VK_BACK
        SpecialKey::Escape => 0x1B,    // VK_ESCAPE
        SpecialKey::Enter => 0x0D,     // VK_RETURN
        SpecialKey::Space => 0x20,     // VK_SPACE
        SpecialKey::Delete => 0x2E,    // VK_DELETE
    }
}

/// 記号の VK マッピング（文字 → (VK コード, Shift 必要)）
///
/// JIS キーボード + IME ひらがなモード前提。
/// IME が有効な状態でこれらのキーストロークを送ると、
/// 対応する全角記号が入力される。
fn build_symbol_to_vk() -> HashMap<char, (u16, bool)> {
    let entries: &[(char, u16, bool)] = &[
        // 句読点・括弧
        ('、', 0xBC, false),  // , (VK_OEM_COMMA)
        ('。', 0xBE, false),  // . (VK_OEM_PERIOD)
        ('・', 0xBF, false),  // / (VK_OEM_2)
        ('「', 0xDB, false),  // [ (VK_OEM_4)
        ('」', 0xDD, false),  // ] (VK_OEM_6)
        // 長音・記号
        ('ー', 0xBD, false),  // - (VK_OEM_MINUS)
        ('～', 0xDE, true),   // Shift+^ (VK_OEM_7, JIS)
        // 全角 ASCII 記号
        ('？', 0xBF, true),   // Shift+/
        ('！', 0x31, true),   // Shift+1
        ('＃', 0x33, true),   // Shift+3
        ('＄', 0x34, true),   // Shift+4
        ('％', 0x35, true),   // Shift+5
        ('＆', 0x36, true),   // Shift+6
        ('（', 0x38, true),   // Shift+8
        ('）', 0x39, true),   // Shift+9
        ('＝', 0xBD, true),   // Shift+- (JIS: =)
        ('＋', 0xBB, true),   // Shift+; (VK_OEM_PLUS, JIS: +)
        ('＊', 0xBA, true),   // Shift+: (VK_OEM_1, JIS: *)
        ('＜', 0xBC, true),   // Shift+,
        ('＞', 0xBE, true),   // Shift+.
        ('＠', 0xC0, false),  // @ (VK_OEM_3, JIS)
        ('｛', 0xDB, true),   // Shift+[
        ('｝', 0xDD, true),   // Shift+]
        ('＿', 0xE2, true),   // Shift+＼ (JIS: _)
        ('｜', 0xDC, true),   // Shift+¥ (JIS: |)
        ('"', 0x32, true),    // Shift+2 (JIS: ")
        ('＂', 0x32, true),   // 全角" → Shift+2
        ('；', 0xBB, false),  // ; (VK_OEM_PLUS, JIS: ;)
        ('：', 0xBA, false),  // : (VK_OEM_1, JIS: :)
        ('－', 0xBD, false),  // - (VK_OEM_MINUS) 全角ハイフンマイナス
        ('／', 0xBF, false),  // / (VK_OEM_2)
        ('＾', 0xDE, false),  // ^ (VK_OEM_7, JIS)
        ('｀', 0xC0, true),   // Shift+@ (JIS: `)
        ('＇', 0x37, true),   // Shift+7 (JIS: ')
        ('＼', 0xE2, false),  // ＼ (VK_OEM_102, JIS)
        // 全角数字
        ('０', 0x30, false),
        ('１', 0x31, false),
        ('２', 0x32, false),
        ('３', 0x33, false),
        ('４', 0x34, false),
        ('５', 0x35, false),
        ('６', 0x36, false),
        ('７', 0x37, false),
        ('８', 0x38, false),
        ('９', 0x39, false),
        // 半角数字
        ('0', 0x30, false),
        ('1', 0x31, false),
        ('2', 0x32, false),
        ('3', 0x33, false),
        ('4', 0x34, false),
        ('5', 0x35, false),
        ('6', 0x36, false),
        ('7', 0x37, false),
        ('8', 0x38, false),
        ('9', 0x39, false),
        // 半角 ASCII 記号
        ('!', 0x31, true),   // Shift+1
        ('"', 0x32, true),   // Shift+2 (JIS)
        ('#', 0x33, true),   // Shift+3
        ('$', 0x34, true),   // Shift+4
        ('%', 0x35, true),   // Shift+5
        ('&', 0x36, true),   // Shift+6
        ('\'', 0x37, true),  // Shift+7 (JIS)
        ('(', 0x38, true),   // Shift+8
        (')', 0x39, true),   // Shift+9
        ('?', 0xBF, true),   // Shift+/
        ('-', 0xBD, false),
        ('=', 0xBD, true),   // Shift+- (JIS)
        ('.', 0xBE, false),
        (',', 0xBC, false),
        ('/', 0xBF, false),
        ('[', 0xDB, false),
        (']', 0xDD, false),
        (';', 0xBB, false),  // JIS: ;
        (':', 0xBA, false),  // JIS: :
        ('+', 0xBB, true),   // Shift+; (JIS)
        ('*', 0xBA, true),   // Shift+: (JIS)
        ('<', 0xBC, true),   // Shift+,
        ('>', 0xBE, true),   // Shift+.
        ('@', 0xC0, false),  // JIS: @
        ('^', 0xDE, false),  // JIS: ^
        ('_', 0xE2, true),   // Shift+＼ (JIS)
        ('{', 0xDB, true),   // Shift+[
        ('}', 0xDD, true),   // Shift+]
        ('|', 0xDC, true),   // Shift+¥ (JIS)
        ('~', 0xDE, true),   // Shift+^ (JIS)
        ('`', 0xC0, true),   // Shift+@ (JIS)
        ('\\', 0xE2, false), // JIS: ＼
    ];
    entries.iter().map(|&(ch, vk, shift)| (ch, (vk, shift))).collect()
}

/// SendInput によるキー注入を行うモジュール
#[allow(missing_debug_implementations)]
pub struct Output {
    mode: OutputMode,
    /// Unicode モード用: ローマ字→ひらがな変換テーブル
    romaji_to_kana: Option<HashMap<String, char>>,
    /// Chrome VK モード用: かな→ローマ字逆引きテーブル
    kana_to_romaji: HashMap<char, String>,
    /// Chrome VK モード用: 記号→VK コードマッピング
    symbol_to_vk: HashMap<char, (u16, bool)>,
}

impl Output {
    pub fn new(mode: OutputMode) -> Self {
        // romaji_to_kana テーブルは UWP アプリ向けの Unicode フォールバックで
        // 常に必要なので、設定に関わらず構築する。
        let romaji_to_kana = Some(awase::kana_table::build_romaji_to_kana());
        Self {
            mode,
            romaji_to_kana,
            kana_to_romaji: awase::kana_table::build_kana_to_romaji(),
            symbol_to_vk: build_symbol_to_vk(),
        }
    }

    /// 出力モードを変更する
    pub fn set_mode(&mut self, mode: OutputMode) {
        self.mode = mode;
        if mode == OutputMode::Unicode {
            self.romaji_to_kana
                .get_or_insert_with(awase::kana_table::build_romaji_to_kana);
        }
    }

    /// アクション列を順に実行する
    ///
    /// 出力方式は `AppKind::Chrome` または config の `focus_overrides.force_vk` に
    /// マッチする場合に VK 経路、それ以外は Unicode 直接送信:
    /// - Chrome: VK キーストローク（独自 TSF text store 経由で IME composition）
    /// - Win32/Uwp: Unicode 直接送信
    pub fn send_keys(&self, actions: &[KeyAction]) {
        let use_vk = resolve_use_vk();

        log::debug!(
            "send_keys: use_vk={use_vk} actions={actions:?}",
        );

        for action in actions {
            match action {
                KeyAction::SpecialKey(sk) => {
                    log::debug!("  → SpecialKey({sk:?}) vk=0x{:02X}", special_key_to_vk(*sk));
                    self.send_key(special_key_to_vk(*sk), false);
                }
                KeyAction::Key(vk) => {
                    log::debug!("  → Key(0x{:04X})", vk.0);
                    self.send_key(vk.0, false);
                }
                KeyAction::KeyUp(vk) => {
                    log::debug!("  → KeyUp(0x{:04X})", vk.0);
                    self.send_key(vk.0, true);
                }
                KeyAction::Char(ch) => {
                    if use_vk {
                        log::debug!("  → Char('{ch}') via VK (Chrome mode)");
                        self.send_char_as_vk(*ch);
                    } else {
                        log::debug!("  → Char('{ch}') via Unicode");
                        self.send_unicode_char(*ch);
                    }
                }
                KeyAction::Suppress => {
                    log::debug!("  → Suppress");
                }
                KeyAction::Romaji(s) => {
                    log::debug!("  → Romaji(\"{s}\") (mode auto-selected by AppKind)");
                    self.send_romaji(s);
                }
                KeyAction::KeySequence(s) => {
                    if use_vk {
                        log::debug!("  → KeySequence(\"{s}\") via VK (Chrome)");
                        for ch in s.chars() {
                            self.send_char_as_vk(ch);
                        }
                    } else {
                        log::debug!("  → KeySequence(\"{s}\") via Unicode");
                        for ch in s.chars() {
                            self.send_unicode_char(ch);
                        }
                    }
                }
            }
        }
    }

    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    #[allow(clippy::unused_self)]
    fn send_key(&self, vk: u16, is_keyup: bool) {
        let input = make_key_input(vk, is_keyup);
        unsafe {
            SendInput(
                &[input],
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
    #[allow(clippy::unused_self)]
    fn send_unicode_char(&self, ch: char) {
        let mut utf16_buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut utf16_buf);

        let mut inputs = Vec::with_capacity(utf16.len() * 2);
        for &code_unit in utf16.iter() {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code_unit,
                        dwFlags: KEYEVENTF_UNICODE,
                        time: 0,
                        dwExtraInfo: INJECTED_MARKER,
                    },
                },
            });
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code_unit,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: INJECTED_MARKER,
                    },
                },
            });
        }
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    /// ローマ字文字列を送信する（モードに応じて方式を切り替え）
    fn send_romaji(&self, romaji: &str) {
        // AppKind 自動判定:
        // - Chrome (Chrome, Edge, Electron, VS Code 等):
        //   Batched で IME composition を経由。独自 TSF text store を持つアプリでは
        //   VK キーストロークを IME に渡して composition させる必要がある。
        //   Batched (1回の SendInput) を使うことで、flush 時の出力と後続キー
        //   (Enter 等) の reinject が競合するのを防ぐ。
        // - Win32 / UWP: Unicode 直接送信（IME をバイパス）。
        //   Win32 クラシックアプリは Unicode 注入が最も安定。
        //   UWP は TSF が VK キーストロークを composition できないため Unicode 必須。
        //
        // wezterm など「IMM query は通るが composition は独自 TSF」系アプリは
        // config の `focus_overrides.force_vk` で VK 側へ明示的に倒す。
        if resolve_use_vk() {
            log::debug!("  send_romaji: → Batched (VK composition)");
            self.send_romaji_batched(romaji);
        } else {
            log::debug!("  send_romaji: → Unicode");
            self.send_romaji_as_unicode(romaji);
        }
    }

    /// PerKey モード: 1文字ずつ個別の SendInput 呼び出し
    ///
    /// 各文字の KeyDown+KeyUp は1回の SendInput にまとめるが、
    /// 文字間は別の SendInput 呼び出しに分離する。
    /// 他のキーボードフックに処理時間を与える。
    #[allow(clippy::unused_self)]
    fn send_romaji_per_key(&self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                let mut inputs = Vec::with_capacity(4);
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, false));
                }
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, true));
                }
                unsafe {
                    SendInput(
                        &inputs,
                        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                    );
                }
            }
        }
    }

    /// Batched モード: 全文字を1回の SendInput にまとめて送信
    ///
    /// 最も高速。SendInput のアトミック性により他の入力が割り込めない。
    /// Chrome/wezterm 等の AppKind::Chrome アプリで使用。
    /// flush 時の出力と後続キー (Enter reinject 等) の競合を防ぐ。
    #[allow(clippy::unused_self)]
    fn send_romaji_batched(&self, romaji: &str) {
        let mut inputs = Vec::with_capacity(romaji.len() * 4);
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, false));
                }
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, true));
                }
            }
        }
        if !inputs.is_empty() {
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
        }
    }

    /// Unicode モード: ローマ字→ひらがなに変換して Unicode 文字として直接送信
    ///
    /// IME を経由せず、ひらがなを直接テキストフィールドに挿入する。
    /// 変換テーブルにないローマ字は PerKey モードでフォールバック送信する。
    fn send_romaji_as_unicode(&self, romaji: &str) {
        if let Some(&kana) = self.romaji_to_kana.as_ref().and_then(|t| t.get(romaji)) {
            self.send_unicode_char(kana);
            return;
        }
        // テーブルにない場合はフォールバック
        self.send_romaji_per_key(romaji);
    }

    /// 文字を VK キーストロークとして送信する（Chrome モード用）
    ///
    /// かな文字はローマ字に逆変換してからキーストロークとして送信する。
    /// ASCII 記号は対応する VK コードで直接送信する。
    /// いずれにもマッチしない場合は Unicode 直接出力にフォールバックする。
    /// 文字を Chrome モード用に送信する。
    ///
    /// 1. かな → ローマ字 VK（IME 経由で変換）
    /// 2. 記号 → マッピングテーブルの VK コード（IME が全角変換）
    /// 3. フォールバック → Unicode 直接出力
    fn send_char_as_vk(&self, ch: char) {
        // 1. かな→ローマ字逆引き（か → "ka" → VK(k), VK(a) → IME が変換）
        if let Some(romaji) = self.kana_to_romaji.get(&ch) {
            log::debug!("    send_char_as_vk: '{ch}' → romaji \"{romaji}\"");
            self.send_romaji_per_key(romaji);
            return;
        }
        // 2. 記号→VK コード（？ → Shift+/ → IME が全角？に変換）
        if let Some(&(vk, needs_shift)) = self.symbol_to_vk.get(&ch) {
            log::debug!("    send_char_as_vk: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
            let mut inputs = Vec::with_capacity(4);
            if needs_shift {
                inputs.push(make_key_input(VK_LSHIFT, false));
            }
            inputs.push(make_key_input(vk, false));
            inputs.push(make_key_input(vk, true));
            if needs_shift {
                inputs.push(make_key_input(VK_LSHIFT, true));
            }
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
            return;
        }
        // 3. フォールバック: Unicode 直接出力
        log::debug!("    send_char_as_vk: '{ch}' (U+{:04X}) → fallback Unicode", ch as u32);
        self.send_unicode_char(ch);
    }

    /// キーシーケンスを送信する（IME がキーストロークを変換する）
    ///
    /// 各文字について対応するキーストローク（VK コード + Shift）を送信する。
    /// マッピングが見つからない文字は Unicode 直接出力でフォールバックする。
    #[allow(dead_code)]
    fn send_key_sequence(&self, s: &str) {
        for ch in s.chars() {
            if let Some((vk, needs_shift)) = char_to_key_sequence(ch) {
                let mut inputs = Vec::with_capacity(4);
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, false));
                }
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input(VK_LSHIFT, true));
                }
                unsafe {
                    SendInput(
                        &inputs,
                        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                    );
                }
            } else {
                // マッピングが見つからない場合は Unicode 直接出力
                self.send_unicode_char(ch);
            }
        }
    }
}

/// INPUT 構造体を作成するヘルパー
const fn make_key_input(vk: u16, is_keyup: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_to_vk_lowercase() {
        assert_eq!(ascii_to_vk('a'), Some((0x41, false)));
        assert_eq!(ascii_to_vk('z'), Some((0x5A, false)));
    }

    #[test]
    fn test_ascii_to_vk_uppercase() {
        assert_eq!(ascii_to_vk('A'), Some((0x41, true)));
    }

    #[test]
    fn test_ascii_to_vk_digits() {
        assert_eq!(ascii_to_vk('0'), Some((0x30, false)));
        assert_eq!(ascii_to_vk('9'), Some((0x39, false)));
    }

    #[test]
    fn test_ascii_to_vk_unknown() {
        assert_eq!(ascii_to_vk('\u{3042}'), None); // 'あ'
    }
}
