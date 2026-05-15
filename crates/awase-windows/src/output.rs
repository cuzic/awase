use std::collections::HashMap;

use awase::config::OutputMode;
use awase::types::{AppKind, KeyAction, SpecialKey};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};

/// 自己注入マーカー（"KEYM" = 0x4B45_594D）
pub const INJECTED_MARKER: usize = 0x4B45_594D;

/// TSF 向け注入マーカー（"KEYF" = 0x4B45_5946）
///
/// hook では INJECTED_MARKER と同様に再処理をスキップするが、
/// dwExtraInfo の値が異なることで TSF Sequential モードの識別に使う。
pub const TSF_MARKER: usize = 0x4B45_5946;

/// 出力注入モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionMode {
    /// Unicode 直接注入（Win32/UWP デフォルト）
    Unicode,
    /// VK Batched 注入（Chrome/Edge/Electron — IME composition 経由）
    Vk,
    /// VK Sequential 注入（WezTerm — TSF 直結アプリ向け）
    Tsf,
}

/// VK_LSHIFT の仮想キーコード
const VK_LSHIFT: u16 = 0xA0;

/// 現在のフォーカス先から出力注入モードを決定する。
///
/// 優先順位:
///   1. config の `focus_overrides.force_tsf` にマッチ → Tsf
///   2. config の `focus_overrides.force_vk` にマッチ → Vk
///   3. AppKind::Chrome → Vk
///   4. それ以外 (Win32 / Uwp) → Unicode
fn resolve_injection_mode() -> InjectionMode {
    unsafe {
        let Some(app) = crate::APP.get_ref() else {
            return InjectionMode::Unicode;
        };
        if let Some((pid, class)) = app.executor.platform.focus.last_focus_info.as_ref() {
            if crate::runtime::is_force_tsf(
                &app.executor.platform.focus.overrides,
                *pid,
                class,
            ) {
                return InjectionMode::Tsf;
            }
            if crate::runtime::is_force_vk(
                &app.executor.platform.focus.overrides,
                *pid,
                class,
            ) {
                return InjectionMode::Vk;
            }
        }
        if app.platform_state.app_kind == AppKind::Chrome {
            InjectionMode::Vk
        } else {
            InjectionMode::Unicode
        }
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
    /// 最後の `send_keys` 完了時刻（ms）。
    ///
    /// awase が SendInput でキーを注入した直後は、OS 入力キュー → WezTerm/Chrome
    /// → IME の composition pipeline で出力イベントが処理中。この間に user の
    /// passthrough キー (Enter / Ctrl / Backspace 等) が届くと、IME が pending
    /// composition を cancel して "タスク → タスk" 等の race condition が起きる。
    ///
    /// executor がこの値を読んで「output in-flight 期間」を判定し、当該期間内に
    /// 来た passthrough を deferr/wait することで race を解消する。
    last_send_ms: std::cell::Cell<u64>,
    /// TSF composition context の readiness フラグ。
    ///
    /// # 設計メモ（2026-05-14）
    ///
    /// このフラグは概念的に2つの独立した状態を1つに統合している:
    ///   - `ime_enabled`: IME がオープン（ひらがなモード）かどうか
    ///   - `composition_ready`: TSF composition session が awase の
    ///     SendInput バッチを受け付ける状態にあるかどうか
    ///
    /// 今後問題が複雑化する場合は `ImeState { ime_enabled, composition_ready }`
    /// への分離を検討すること。
    ///
    /// # cold になるタイミング（`false`）
    ///   - 起動時（初期値 false）
    ///   - フォーカス変更（別ウィンドウ = 新しい TSF context）
    ///   - Space / Enter / Escape の passthrough / reinject（composition 確定・キャンセル）
    ///   - エンジン OFF → ON（IME 状態リセット）
    ///   - F2 (VK_DBE_HIRAGANA) 検出（TSF context が未初期化になる可能性）
    ///
    /// # 重要: 物理 F2 と warmup F2 の違い
    ///
    /// 物理 F2（CallNextHookEx 経由）は OS レベルで IME をひらがなモードに
    /// 切り替えるが、awase の SendInput バッチに対する TSF composition context を
    /// 初期化しない（別のメッセージポンプサイクルで処理されるため）。
    ///
    /// TSF context の初期化には warmup F2 が SendInput バッチの先頭に含まれる
    /// 必要がある（[F2↓F2↑, romaji...] の形で同一バッチ内に同居すること）。
    ///
    /// そのため TSF モードでは物理 F2 を awase が Consume し、
    /// 次の NICOLA 出力バッチの warmup F2 で一本化する設計を取る。
    ///
    /// `send_romaji_batched` / `send_romaji_as_tsf` が `false` を検出すると
    /// VK_DBE_HIRAGANA ウォームアップを先頭に挿入し、送信後に `true` にセットする。
    composition_warm: std::cell::Cell<bool>,
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
            last_send_ms: std::cell::Cell::new(0),
            composition_warm: std::cell::Cell::new(false),
        }
    }

    /// 最後の `send_keys` 完了からの経過時間（ms）。
    /// 一度も送信していない場合は `u64::MAX` を返す（= 永久に in-flight でない）。
    #[must_use]
    pub fn ms_since_last_send(&self) -> u64 {
        let last = self.last_send_ms.get();
        if last == 0 {
            return u64::MAX;
        }
        crate::hook::current_tick_ms().saturating_sub(last)
    }

    /// IME composition context をコールド状態にマークする。
    ///
    /// 次の VK / TSF composition 送信時に VK_DBE_HIRAGANA ウォームアップを
    /// 先行送信させる。フォーカス変更・Space/Enter/Escape passthrough・
    /// エンジン toggle 等のタイミングで呼ぶ。
    pub fn mark_composition_cold(&self) {
        log::debug!("[composition] marked cold → next VK/TSF output will send VK_DBE_HIRAGANA warmup");
        self.composition_warm.set(false);
    }

    /// IME composition context をウォーム状態にマークする。
    ///
    /// 直前の NICOLA 出力バッチで warmup F2 が正常に送信され、
    /// TSF composition context が初期化済みであると分かっている場合に呼ぶ。
    pub fn mark_composition_warm(&self) {
        log::debug!("[composition] marked warm → next VK/TSF output will NOT send VK_DBE_HIRAGANA warmup");
        self.composition_warm.set(true);
    }

    /// 現在の composition_warm フラグを返す（ログ・診断用）。
    pub fn is_composition_warm(&self) -> bool {
        self.composition_warm.get()
    }

    /// 現在のフォーカス先が TSF 注入モードかどうかを返す。
    ///
    /// TSF モード（WezTerm 等）では物理 F2 の扱いが特殊なため、
    /// executor がこのメソッドで判定してキー処理を切り替える。
    pub fn is_tsf_mode(&self) -> bool {
        resolve_injection_mode() == InjectionMode::Tsf
    }

    /// `send_keys` 完了時刻を記録する内部ヘルパー。
    fn mark_send(&self) {
        let ms = crate::hook::current_tick_ms();
        log::debug!("[mark-send] last_send_ms={ms}");
        self.last_send_ms.set(ms);
    }

    /// 出力モードを変更する
    pub fn set_mode(&mut self, mode: OutputMode) {
        self.mode = mode;
        if mode == OutputMode::Unicode {
            self.romaji_to_kana
                .get_or_insert_with(awase::kana_table::build_romaji_to_kana);
        }
    }

    /// VK/TSF 出力後に「最終キー活動時刻」を同期更新する。
    ///
    /// SendInput 後の hook 通知はメッセージループで非同期処理されるため、
    /// 直後に IME ポーリングが走ると `last_hook_activity_ms` が更新前のまま
    /// アイドル判定を通過してしまう。送信直後に同期更新することで
    /// アイドルタイマーが正しくリセットされる。
    fn mark_vk_output() {
        unsafe {
            if let Some(app) = crate::APP.get_mut() {
                app.platform_state.last_hook_activity_ms = crate::hook::current_tick_ms();
            }
        }
    }

    /// アクション列を順に実行する
    ///
    /// 注入モードは `resolve_injection_mode()` で決定:
    /// - Unicode: Win32/UWP デフォルト。Unicode 直接注入で IME をバイパス。
    /// - Vk: Chrome/Edge/Electron。Batched VK で IME composition。
    /// - Tsf: WezTerm 等。Sequential VK で TSF/IME に composition させる。
    pub fn send_keys(&self, actions: &[KeyAction]) {
        let mode = resolve_injection_mode();

        log::debug!("send_keys: mode={mode:?} actions={actions:?}");

        // NOTE: ImeDiagnosticSnapshot::capture("send_keys_pre") をここに置いてはいけない。
        // capture() は内部で GetGUIThreadInfo(100ms) + SendMessageTimeoutW(50ms×2) を
        // 呼ぶため、send_keys の中でメッセージポンプが走り Space 等の WH_KEYBOARD_LL
        // コールバックが SendInput より前に発火して "境界dえ" 等の race を起こす。

        // output in-flight guard の基準点を SendInput より前に設定する。
        // 呼び出し元が何らかのブロッキング処理を send_keys 内に追加した場合でも
        // guard が有効になるよう、ループ前に記録しておく。
        // ループ後の mark_send() も残す（reinject wait の正確な基準点のため）。
        self.mark_send();

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
                KeyAction::Char(ch) => match mode {
                    InjectionMode::Vk => {
                        log::debug!("  → Char('{ch}') via VK Batched (Chrome mode)");
                        self.send_char_as_vk(*ch);
                    }
                    InjectionMode::Tsf => {
                        log::debug!("  → Char('{ch}') via VK Sequential (TSF mode)");
                        self.send_char_as_tsf(*ch);
                    }
                    InjectionMode::Unicode => {
                        log::debug!("  → Char('{ch}') via Unicode");
                        self.send_unicode_char(*ch);
                    }
                },
                KeyAction::Suppress => {
                    log::debug!("  → Suppress");
                }
                KeyAction::Romaji(s) => {
                    log::debug!("  → Romaji(\"{s}\") mode={mode:?}");
                    self.send_romaji(s);
                }
                KeyAction::KeySequence(s) => match mode {
                    InjectionMode::Vk => {
                        log::debug!("  → KeySequence(\"{s}\") via VK Batched (Chrome)");
                        for ch in s.chars() {
                            self.send_char_as_vk(ch);
                        }
                    }
                    InjectionMode::Tsf => {
                        log::debug!("  → KeySequence(\"{s}\") via VK Sequential (TSF)");
                        for ch in s.chars() {
                            self.send_char_as_tsf(ch);
                        }
                    }
                    InjectionMode::Unicode => {
                        log::debug!("  → KeySequence(\"{s}\") via Unicode");
                        for ch in s.chars() {
                            self.send_unicode_char(ch);
                        }
                    }
                },
            }
        }

        // VK/TSF モードで出力した場合、直後の IME ポーリングをガードするため
        // タイムスタンプを記録する（母音落ち「て→tえ」防止）。
        if mode != InjectionMode::Unicode {
            Self::mark_vk_output();
        }

        // executor が「output in-flight」判定に使う送信時刻を記録する。
        // user passthrough キー (Enter / Ctrl 等) が race するのを防ぐための基準点。
        // Unicode モードでも記録しておく（race の理論的可能性は残るため）。
        self.mark_send();
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
        match resolve_injection_mode() {
            InjectionMode::Vk => {
                log::debug!("  send_romaji: → VK Batched (Chrome)");
                self.send_romaji_batched(romaji);
            }
            InjectionMode::Tsf => {
                log::debug!("  send_romaji: → VK Sequential (TSF)");
                self.send_romaji_as_tsf(romaji);
            }
            InjectionMode::Unicode => {
                log::debug!("  send_romaji: → Unicode");
                self.send_romaji_as_unicode(romaji);
            }
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

    /// Batched モード: 全文字を1回の SendInput にまとめて送信（重畳押し順）
    ///
    /// 送信順序: 全文字の KeyDown を先に送り、その後全文字の KeyUp を送る
    /// （重畳順: 例 "mu" → M↓ U↓ M↑ U↑）
    ///
    /// WezTerm 等は WM_KEYUP 受信時にコンポジションをコミットする。
    /// 逐次順（M↓ M↑ U↓ U↑）だと M↑ 到着時点でまだ U が来ていないため
    /// 'm' が単独確定し "mう" になる。
    /// 重畳順では U↓ が M↑ より先に処理されるため IME が 'mu' を一組として
    /// 受け取り "む" に正しく変換される。
    /// 全文字を1回の SendInput にまとめることで、後続キー（Enter reinject 等）が
    /// 文字キーの間に割り込むのを防ぐ（per_key との違い）。
    fn send_romaji_batched(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();

        if chars.is_empty() {
            return;
        }

        // composition_warm が false（コールド）のとき VK_DBE_HIRAGANA ウォームアップを先行送信する。
        // コールドになるタイミング: 起動時・フォーカス変更・Space/Enter/Escape passthrough・
        // エンジン toggle・F2 passthrough（executor / runtime が mark_composition_cold() を呼ぶ）。
        // タイムアウト: 前回送信から COMPOSITION_TIMEOUT_MS 以上経過した場合も warm 扱いしない。
        // IME composition context は長い沈黙の後に期限切れになる可能性があるため。
        const COMPOSITION_TIMEOUT_MS: u64 = 2000;
        let warm = self.composition_warm.get();
        let elapsed = self.ms_since_last_send();
        let session_expired = warm && elapsed < u64::MAX && elapsed > COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        if prepend_f2_warmup {
            if session_expired {
                log::debug!("[vk-warmup] session expired ({elapsed}ms) → prepending VK_DBE_HIRAGANA warmup");
            } else {
                log::debug!("[vk-warmup] cold → prepending VK_DBE_HIRAGANA warmup");
            }
        }

        let warmup_slots = if prepend_f2_warmup { 2 } else { 0 };
        let mut inputs = Vec::with_capacity(chars.len() * 4 + warmup_slots);

        if prepend_f2_warmup {
            const VK_DBE_HIRAGANA: u16 = 0xF2;
            inputs.push(make_key_input(VK_DBE_HIRAGANA, false));
            inputs.push(make_key_input(VK_DBE_HIRAGANA, true));
        }

        for &(vk, needs_shift) in &chars {
            if needs_shift {
                inputs.push(make_key_input(VK_LSHIFT, false));
            }
            inputs.push(make_key_input(vk, false));
        }
        for &(vk, needs_shift) in &chars {
            inputs.push(make_key_input(vk, true));
            if needs_shift {
                inputs.push(make_key_input(VK_LSHIFT, true));
            }
        }

        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        self.composition_warm.set(true);
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

    /// TSF Batched モード: 全文字を1回の SendInput にまとめて送信（TSF_MARKER 付き）
    ///
    /// WezTerm 等 TSF 直結アプリ向け。送信順序は Chrome Batched と同じく
    /// 全文字の KeyDown を先に送り、その後全文字の KeyUp を送る
    /// （重畳順: 例 "mu" → M↓ U↓ M↑ U↑）。
    ///
    /// Sequential（M↓M↑ then U↓U↑）では M↑ 到着時点で IME が 'm' を単独確定し
    /// "mう" になる。重畳順では U↓ が M↑ より先に到達するため IME が "mu" を
    /// ひとまとまりとして受け取り "む" に変換する。
    /// TSF_MARKER を使うことで WezTerm の IME バイパスを回避する（INJECTED_MARKER
    /// を使うと WezTerm が IME をスキップして PTY に直送してしまう）。
    fn send_romaji_as_tsf(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();

        if chars.is_empty() {
            return;
        }

        // composition_warm が false（コールド）のとき VK_DBE_HIRAGANA ウォームアップを先行送信する。
        // WezTerm の TSF context は F2 passthrough・フォーカス変更・Space/Enter/Escape 後に
        // コールド状態になる。同一 SendInput に含める必要がある（別 SendInput にすると
        // WezTerm が TSF 初期化完了前に次のキーを受け取り効果がないため）。
        // タイムアウト: 前回送信から COMPOSITION_TIMEOUT_MS 以上経過した場合も warm 扱いしない。
        // IME composition context は長い沈黙の後に期限切れになる可能性があるため。
        const COMPOSITION_TIMEOUT_MS: u64 = 2000;
        let warm = self.composition_warm.get();
        let elapsed = self.ms_since_last_send();
        let session_expired = warm && elapsed < u64::MAX && elapsed > COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        if prepend_f2_warmup {
            if session_expired {
                log::debug!("[tsf-warmup] session expired ({elapsed}ms) → prepending VK_DBE_HIRAGANA to first SendInput");
            } else {
                log::debug!("[tsf-warmup] cold → prepending VK_DBE_HIRAGANA to first SendInput");
            }
        }

        // 同一 VK が連続する箇所（例 "nn"）でバッチに N↓N↓N↑N↑ を含めると、IME が
        // 2 つ目の N↓ をオートリピートと判定して破棄してしまう。結果 "n"+次の母音 →
        // "ne" 等に合成され、例えば "かんえい" → "かねい" になる。
        // 同一 VK が連続する境界で run を分割し、各 run を別の SendInput で送る
        // ことで、N↑ が次の N↓ より先に届き auto-repeat 判定を回避できる。
        // 異なる VK 同士は重畳順を保つため同一バッチに含める（"mu" の崩れ防止）。
        let mut runs: Vec<&[(u16, bool)]> = Vec::new();
        let mut start = 0;
        for i in 1..chars.len() {
            if chars[i].0 == chars[i - 1].0 {
                runs.push(&chars[start..i]);
                start = i;
            }
        }
        runs.push(&chars[start..]);

        for (run_idx, run) in runs.iter().enumerate() {
            let warmup_slots = if run_idx == 0 && prepend_f2_warmup { 2 } else { 0 };
            let mut inputs = Vec::with_capacity(run.len() * 4 + warmup_slots);
            if run_idx == 0 && prepend_f2_warmup {
                const VK_DBE_HIRAGANA: u16 = 0xF2;
                inputs.push(make_tsf_key_input(VK_DBE_HIRAGANA, false));
                inputs.push(make_tsf_key_input(VK_DBE_HIRAGANA, true));
            }
            for &(vk, needs_shift) in *run {
                if needs_shift {
                    inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
                }
                inputs.push(make_tsf_key_input(vk, false));
            }
            for &(vk, needs_shift) in *run {
                inputs.push(make_tsf_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
                }
            }
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
        }
        self.composition_warm.set(true);
    }

    /// 文字を TSF Sequential VK キーストロークとして送信する（WezTerm TSF モード用）
    ///
    /// かな文字はローマ字に逆変換してから `send_romaji_as_tsf` で送信する。
    /// 記号は symbol_to_vk テーブルで直接 VK コードに変換する。
    /// マッチしない場合は Unicode 直接出力にフォールバックする。
    fn send_char_as_tsf(&self, ch: char) {
        if let Some(romaji) = self.kana_to_romaji.get(&ch) {
            log::debug!("    send_char_as_tsf: '{ch}' → romaji \"{romaji}\"");
            self.send_romaji_as_tsf(romaji);
            return;
        }
        if let Some(&(vk, needs_shift)) = self.symbol_to_vk.get(&ch) {
            log::debug!("    send_char_as_tsf: '{ch}' → VK 0x{vk:02X} shift={needs_shift}");
            let mut inputs = Vec::with_capacity(4);
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
            }
            inputs.push(make_tsf_key_input(vk, false));
            inputs.push(make_tsf_key_input(vk, true));
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
            }
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
            // シンボル VK 送信後、WezTerm TSF は現在の composition を commit して
            // context をリセットする可能性がある（例: 'ー' 後の composition リセット）。
            // 次の romaji 出力で F2 warmup を prepend させるため cold にマーク。
            self.mark_composition_cold();
            return;
        }
        log::debug!("    send_char_as_tsf: '{ch}' (U+{:04X}) → fallback Unicode", ch as u32);
        self.send_unicode_char(ch);
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
            // Batched (1回の SendInput) を使うことで、後続キー（Enter reinject 等）との
            // 競合を防ぐ。per_key では K↓K↑ と A↓A↑ が別 SendInput になり、
            // 間に Enter が割り込むと "kあ" のような出力破壊が起きる。
            self.send_romaji_batched(romaji);
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

/// INPUT 構造体を作成するヘルパー（INJECTED_MARKER 固定）
const fn make_key_input(vk: u16, is_keyup: bool) -> INPUT {
    make_key_input_ex(vk, is_keyup, INJECTED_MARKER)
}

/// TSF モード用 INPUT 構造体を作成するヘルパー（TSF_MARKER 付き）
///
/// `wVk` を保持したまま `MapVirtualKeyW` で算出した `wScan` も設定する。
/// `KEYEVENTF_SCANCODE` は付加しない（付加すると WezTerm が LLKHF_SCANCODE フラグ付き
/// キーとして検出し IME をバイパスしてしまうため）。
fn make_tsf_key_input(vk: u16, is_keyup: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) as u16 };
    let flags = if is_keyup { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: TSF_MARKER,
            },
        },
    }
}

/// INPUT 構造体を作成するヘルパー（dwExtraInfo 指定版）
const fn make_key_input_ex(vk: u16, is_keyup: bool, extra_info: usize) -> INPUT {
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
                dwExtraInfo: extra_info,
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
