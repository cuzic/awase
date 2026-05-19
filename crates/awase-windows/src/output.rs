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

/// IME composition context がコールド状態になった理由（診断ログ用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColdReason {
    /// フォーカス変更
    #[default]
    FocusChange,
    /// `ImeEffect::SetOpen(true)` 実行後
    SetOpenTrue,
    /// 物理 F2 (VK_DBE_HIRAGANA) をフックで Consume（TSF モード）
    NativeF2Consumed,
    /// Space/Enter/Escape のパススルー
    PassthroughConfirmKey,
    /// Space/Enter/Escape の reinject
    ReinjectConfirmKey,
    /// 記号 VK 送信後（TSF context リセット可能性あり）
    SymbolVkSent,
    /// F2 non-TSF mode passthrough
    F2NonTsf,
    /// session_expired（前回送信から 2s 超過）
    SessionExpired,
}

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
    /// Cold-start 発生回数カウンタ（H1 診断ログのセッション識別用）。
    ///
    /// warmup バッチを送るたびにインクリメントされる。
    /// `[h1-probe]` ログの `cold=N` フィールドがこの値に対応する。
    cold_start_count: std::cell::Cell<u32>,
    /// NativeF2Consumed 時に即送信した eager warmup F2 の送信時刻（ms）。
    ///
    /// 0 = 有効な eager warmup なし。
    /// `send_romaji_as_tsf` がこれを参照して重複 F2 送信を避け、
    /// 経過時間に応じた最小 sleep のみで済むようにする。
    eager_warmup_sent_ms: std::cell::Cell<u64>,
    /// TSF composition context の readiness フラグ（epoch 付き）。
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
    /// # cold になるタイミング（0）
    ///   - 起動時（初期値 0 = cold）
    ///   - フォーカス変更（`on_focus_changed()` が focus_epoch をインクリメント → 自動無効化）
    ///   - Space / Enter / Escape の passthrough / reinject（composition 確定・キャンセル）
    ///   - エンジン OFF → ON（IME 状態リセット）
    ///   - F2 (VK_DBE_HIRAGANA) 検出（TSF context が未初期化になる可能性）
    ///
    /// # epoch による自動無効化
    ///
    /// `composition_warm_epoch` はウォームになった時点の `focus_epoch` を記録する。
    /// `is_composition_warm()` は両者が一致するときのみ true を返すため、
    /// フォーカス変更で `focus_epoch` が進むと前ウィンドウのウォーム状態は
    /// 自動的に無効化される（`mark_composition_cold()` 呼び出し漏れに対する安全ネット）。
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
    /// `send_romaji_batched` / `send_romaji_as_tsf` が cold を検出すると
    /// VK_DBE_HIRAGANA ウォームアップを先頭に挿入し、送信後に `mark_composition_warm()` を呼ぶ。
    composition_warm_epoch: std::cell::Cell<u32>,
    /// 現在のフォーカスウィンドウのエポック番号。
    ///
    /// `on_focus_changed()` が呼ばれるたびにインクリメントされる。
    /// `composition_warm_epoch` と照合することで、前ウィンドウのウォーム状態を
    /// 自動無効化する。
    focus_epoch: std::cell::Cell<u32>,
    /// IME ON/OFF のシャドウ状態。
    ///
    /// `notify_ime_open()` で更新される。`send_eager_tsf_warmup()` が
    /// IME OFF 時に F2 を誤送信しないためのガード。
    shadow_ime_on: std::cell::Cell<bool>,
    /// 最後に cold にマークされた理由。
    /// `send_romaji_as_tsf` が eager_settle_ms を動的に決定するために使用。
    last_cold_reason: std::cell::Cell<ColdReason>,
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
            cold_start_count: std::cell::Cell::new(0),
            eager_warmup_sent_ms: std::cell::Cell::new(0),
            composition_warm_epoch: std::cell::Cell::new(0), // 0 = cold
            focus_epoch: std::cell::Cell::new(1),            // 1 = initial window
            shadow_ime_on: std::cell::Cell::new(false),
            last_cold_reason: std::cell::Cell::new(ColdReason::FocusChange),
        }
    }

    /// eager warmup F2 を送信した時刻（ms）を返す。0 = 未送信。
    /// WinEvent 観察コールバックが warmup からの経過時間をログするために使う。
    #[must_use]
    pub fn eager_warmup_sent_ms(&self) -> u64 {
        self.eager_warmup_sent_ms.get()
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
    /// 先行送信させる。Space/Enter/Escape passthrough・エンジン toggle 等のタイミングで呼ぶ。
    /// フォーカス変更は `on_focus_changed()` を使うこと（epoch も更新される）。
    ///
    /// # NativeF2Consumed でも eager_warmup_sent_ms をリセットする理由
    ///
    /// 物理 F2 が押された = WezTerm に新しい F2 が届く = TSF 初期化が再トリガーされる。
    /// FocusChange のタイムスタンプを保持すると「古い F2 からの経過時間」を elapsed として
    /// 計算してしまい、sleep がスキップされる（"hoんらい" 化け: BUG-06 の派生形）。
    ///
    /// 例: FocusChange warmup(T=0) → 物理F2(T=2265ms) → ほ送信(T=2562ms)
    ///   旧: elapsed=2562ms→即送信、新F2からは297ms→TSF未初期化→"ho"リテラル
    ///   新: elapsed=297ms→sleep203ms→新F2から500ms待機→TSF初期化済み→"ほ" ✓
    ///
    /// 直後に send_eager_tsf_warmup() が新しいタイムスタンプをセットする。
    pub fn mark_composition_cold(&self, reason: ColdReason) {
        log::debug!("[composition] marked cold reason={reason:?} → next VK/TSF output will send VK_DBE_HIRAGANA warmup");
        self.composition_warm_epoch.set(0);
        self.eager_warmup_sent_ms.set(0);
        self.last_cold_reason.set(reason);
        crate::OBS_WARMUP_SENT_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// IME composition context をウォーム状態にマークする。
    ///
    /// 直前の NICOLA 出力バッチで warmup F2 が正常に送信され、
    /// TSF composition context が初期化済みであると分かっている場合に呼ぶ。
    pub fn mark_composition_warm(&self) {
        let epoch = self.focus_epoch.get();
        log::debug!("[composition] marked warm (epoch={epoch}) → next VK/TSF output will NOT send VK_DBE_HIRAGANA warmup");
        self.composition_warm_epoch.set(epoch);
    }

    /// 現在の composition_warm フラグを返す。
    ///
    /// `focus_epoch` が変化していれば前ウィンドウのウォーム状態は自動無効化される。
    pub fn is_composition_warm(&self) -> bool {
        let epoch = self.focus_epoch.get();
        self.composition_warm_epoch.get() == epoch && epoch != 0
    }

    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// `focus_epoch` をインクリメントし、前ウィンドウのウォーム状態を自動無効化する。
    /// 従来の `mark_composition_cold()` 呼び出しの代わりに使う（明示的なコールド化も同時に行う）。
    pub fn on_focus_changed(&self) {
        let new_epoch = self.focus_epoch.get().wrapping_add(1).max(1); // 0 は cold の番兵値なので skip
        self.focus_epoch.set(new_epoch);
        self.composition_warm_epoch.set(0);
        self.eager_warmup_sent_ms.set(0);
        crate::OBS_WARMUP_SENT_MS.store(0, std::sync::atomic::Ordering::Relaxed);
        log::debug!("[composition] focus changed → epoch={new_epoch}, marked cold");
    }

    /// 現在のフォーカス先が TSF 注入モードかどうかを返す。
    ///
    /// TSF モード（WezTerm 等）では物理 F2 の扱いが特殊なため、
    /// executor がこのメソッドで判定してキー処理を切り替える。
    pub fn is_tsf_mode(&self) -> bool {
        resolve_injection_mode() == InjectionMode::Tsf
    }

    /// IME ON/OFF のシャドウ状態を更新する。
    ///
    /// `ImeEffect::SetOpen` 実行時および FocusChange 時に呼ぶ。
    /// `send_eager_tsf_warmup()` が IME OFF 時に F2 を誤送信しないためのガード。
    pub fn notify_ime_open(&self, open: bool) {
        log::debug!("[composition] shadow_ime_on → {open}");
        self.shadow_ime_on.set(open);
    }

    /// TSF composition context の事前ウォームアップ F2 を送信する。
    ///
    /// 以下のタイミングで呼ぶ:
    /// - FocusChange 直後: WezTerm に TSF 初期化の先行時間を与える
    /// - NativeF2Consumed 直後: 物理 F2 の代替として送信（二重 F2 防止）
    /// - PassthroughConfirmKey / ReinjectConfirmKey 直後: Enter/Escape 後の次打鍵を warmup
    ///
    /// `shadow_ime_on` が false（IME OFF）または TSF モード以外では何もしない。
    ///
    /// `eager_warmup_sent_ms` は既に設定済みの場合は更新しない（FocusChange 側のより古い
    /// タイムスタンプを優先する）。これにより NativeF2Consumed 後も FocusChange 時刻から
    /// の経過時間で wait 計算ができ、長期アイドル後の TSF 初期化問題を解消する。
    pub fn send_eager_tsf_warmup(&self) {
        if !self.shadow_ime_on.get() || !self.is_tsf_mode() {
            return;
        }
        // OBJ_NAMECHANGE 連番をリセット（warmup 後のイベント順序追跡用）
        crate::OBS_FOCUS_NAMECHANGE_SEQ.store(0, std::sync::atomic::Ordering::Relaxed);
        // VK_DBE_HIRAGANA (F2) を送信: VK_IME_ON (0x16) は IME ON 状態をセットするだけで
        // TSF composition context の初期化をトリガーしない。WezTerm は物理 F2 受信時に
        // TSF composition を初期化するため、同等の VK_DBE_HIRAGANA を送る必要がある。
        const VK_DBE_HIRAGANA: u16 = 0xF2;
        let warmup_inputs = [
            make_tsf_key_input(VK_DBE_HIRAGANA, false),
            make_tsf_key_input(VK_DBE_HIRAGANA, true),
        ];
        unsafe {
            SendInput(
                &warmup_inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        let ms = crate::hook::current_tick_ms();
        // mark_composition_cold が常に eager_warmup_sent_ms=0 にリセットするため、
        // ここは常に新しいタイムスタンプをセットする。
        self.eager_warmup_sent_ms.set(ms);
        // UIA コールバックスレッドからも参照できるよう AtomicU64 に同期する。
        crate::OBS_WARMUP_SENT_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
        log::debug!("[tsf-eager-warmup] VK_DBE_HIRAGANA 送信, eager_warmup_sent_ms={ms}ms");
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
    /// WM_KEYUP 受信時に IME がコンポジションをコミットするアプリ（Chrome 等）では、
    /// 逐次順（M↓M↑ then U↓U↑）だと M↑ 到着時点で IME が 'm' を単独確定し "mう" になる。
    /// 重畳順では U↓ が M↑ より先に処理されるため IME が "mu" を一組として受け取る。
    ///
    /// # H1 cold-start 修正（案A: F2-only 先行バッチ）
    ///
    /// Chrome も WezTerm と同様に F2 受信後の IME 初期化が非同期のため、
    /// [F2↓F2↑ K↓I↓ K↑I↑] を同一バッチで送ると初期化完了前に romaji が処理され
    /// ASCII として出力される（probe が attempt=0 で timeout → Chrome は >10ms 処理）。
    ///
    /// 案A: F2-only バッチを先行送信し、probe loop で Chrome が応答するまで待ってから
    /// romaji バッチを送ることで初期化済み状態での文字受信を保証する。
    fn send_romaji_batched(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();

        if chars.is_empty() {
            return;
        }

        // composition_warm が false（コールド）のとき VK_DBE_HIRAGANA 先行バッチを送信する。
        // タイムアウト: 前回送信から COMPOSITION_TIMEOUT_MS 以上経過した場合も warm 扱いしない。
        const COMPOSITION_TIMEOUT_MS: u64 = 2000;
        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired = warm && elapsed < u64::MAX && elapsed > COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        log::debug!(
            "[vk-send] romaji={romaji:?} warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            if elapsed == u64::MAX { "∞".to_string() } else { elapsed.to_string() }
        );

        if prepend_f2_warmup {
            if session_expired {
                log::debug!("[vk-warmup] session expired ({elapsed}ms) → F2-only先行バッチ (案A)");
            } else {
                log::debug!("[vk-warmup] cold → F2-only先行バッチ (案A)");
            }
            let conv_pre = unsafe { crate::ime::get_ime_conversion_mode_raw() };
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.map_or(false, |v| v & 0x0001 != 0),
                conv_pre.map_or(false, |v| v & 0x0010 != 0),
                conv_pre.map_or(false, |v| v & 0x0002 != 0),
            );
            // IMM32 経由で同期的にローマ字モードへ切り替え。
            unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

            let cold_n = self.cold_start_count.get() + 1;
            self.cold_start_count.set(cold_n);

            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!("[h1-window] cold={cold_n} class={win_class}");

            // 案A: F2-only バッチを先行送信し、Chrome が IME を初期化するのを待つ。
            const VK_DBE_HIRAGANA: u16 = 0xF2;
            let f2_inputs = [
                make_key_input(VK_DBE_HIRAGANA, false),
                make_key_input(VK_DBE_HIRAGANA, true),
            ];
            log::debug!("[h1-run] cold={cold_n} F2-only batch sending");
            unsafe {
                SendInput(
                    &f2_inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }

            // F2-only バッチ後のプローブ（ガード）:
            //   responsive=true → Chrome が F2 を処理済み（IME 初期化完了期待）
            //   responsive=false → Chrome がまだ処理中（timeout まで待機）
            let probe_start = std::time::Instant::now();
            const H1_PROBE_TIMEOUT_MS: u32 = 10;
            const H1_PROBE_MAX: u8 = 15;
            for i in 0..H1_PROBE_MAX {
                let elapsed_ms = probe_start.elapsed().as_millis();
                let conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(H1_PROBE_TIMEOUT_MS) };
                let responsive = conv.is_some();
                let roman = conv.map_or(false, |v| v & 0x0010 != 0);
                log::debug!(
                    "[h1-probe] cold={cold_n} vk attempt={i} elapsed={}ms responsive={responsive} roman={roman} conv={}",
                    elapsed_ms,
                    conv.map_or_else(|| "none/timeout".to_string(), |v| format!("0x{v:08X}")),
                );
                if responsive { break; }
            }
        }

        // ローマ字バッチ送信（重畳順: 全 KeyDown → 全 KeyUp）
        let mut inputs = Vec::with_capacity(chars.len() * 4);
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
        self.mark_composition_warm();
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
    ///
    /// # H1 cold-start 修正（案A: F2-only 先行バッチ）
    ///
    /// 旧実装では [F2↓F2↑ i↓i↑] を同一 SendInput バッチで送っていた。
    /// WezTerm は TSF context の初期化を F2 受信後に非同期で行うため、
    /// 同じバッチ内の 'i' が初期化完了前に処理され ASCII 'i' として出力される。
    ///
    /// 案A では F2-only バッチを先行送信し、WezTerm が F2 を処理して
    /// TSF context を初期化するまで待機（probe loop）してからローマ字バッチを送る。
    fn send_romaji_as_tsf(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();

        if chars.is_empty() {
            return;
        }

        // composition_warm が false（コールド）のとき VK_DBE_HIRAGANA 先行バッチを送信する。
        // タイムアウト: 前回送信から COMPOSITION_TIMEOUT_MS 以上経過した場合も warm 扱いしない。
        const COMPOSITION_TIMEOUT_MS: u64 = 2000;
        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired = warm && elapsed < u64::MAX && elapsed > COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        log::debug!(
            "[tsf-send] romaji={romaji:?} warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            if elapsed == u64::MAX { "∞".to_string() } else { elapsed.to_string() }
        );

        let cold_n;
        if prepend_f2_warmup {
            if session_expired {
                log::debug!("[tsf-warmup] session expired ({elapsed}ms) → F2-only先行バッチ (案A)");
            } else {
                log::debug!("[tsf-warmup] cold → F2-only先行バッチ (案A)");
            }
            // H4/H5 判定: pre-send で ROMAN=true なら IMM32 は正しいが TSF が無視している。
            let conv_pre = unsafe { crate::ime::get_ime_conversion_mode_raw() };
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.map_or(false, |v| v & 0x0001 != 0),
                conv_pre.map_or(false, |v| v & 0x0010 != 0),
                conv_pre.map_or(false, |v| v & 0x0002 != 0),
            );
            // IMM32 経由で同期的にローマ字モードへ切り替え。
            unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

            cold_n = self.cold_start_count.get() + 1;
            self.cold_start_count.set(cold_n);

            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!("[h1-window] cold={cold_n} class={win_class}");

            // TSF cold-start: WezTerm は TSF native app のため ImmGetCompositionStringW で
            // composition state を検出できない（IMM32 HIMC に propagate されない）。
            // 固定 sleep を使用するが、eager warmup から十分時間が経過している場合は
            // TSF が確実に初期化済みのため即送信する。
            // FocusChange 等で eager VK_DBE_HIRAGANA を送信済み + eager_settle_ms 以上経過 → 即送信
            // eager 送信済みだが未到達 → 残り時間だけ sleep
            // eager なし → VK_DBE_HIRAGANA warmup + probe (2連送) で TSF 初期化を同期
            const VK_DBE_HIRAGANA: u16 = 0xF2;
            // ColdReason に応じてウォームアップ待機時間を決定:
            //   FocusChange / SetOpenTrue / NativeF2Consumed:
            //     awase が物理キーを消費して VK_DBE_HIRAGANA を代わりに送るため、
            //     GJI から見ると FocusChange 相当の TSF 再初期化が発生しうる。
            //     実測で候補窓出現まで 1031ms かかることがあるため 1500ms を上限とする。
            //     session.ipc モニターが動作すれば実際の待機時間は大幅に短縮される。
            //   その他（Enter/Space/記号等）: composition 再突入のみ → 500ms
            let eager_settle_ms: u64 = match self.last_cold_reason.get() {
                ColdReason::FocusChange
                | ColdReason::SetOpenTrue
                | ColdReason::NativeF2Consumed => 1500,
                _ => 500,
            };
            // ColdReason に応じた probe 最小待機時間（warmup_sent_ms 起点）:
            //   VK_DBE_HIRAGANA がキューに入ってから GJI が最初の I/O を開始するまでの
            //   実測下限。この時間内は GJI I/O 監視結果を信頼しない。
            let probe_min_ms: u64 = match self.last_cold_reason.get() {
                ColdReason::FocusChange
                | ColdReason::SetOpenTrue
                | ColdReason::NativeF2Consumed => 300,
                ColdReason::SessionExpired => 200,
                ColdReason::PassthroughConfirmKey | ColdReason::ReinjectConfirmKey => 50,
                ColdReason::SymbolVkSent => 30,
                _ => 100,
            };
            log::debug!(
                "[h1-warmup] cold={cold_n} eager_settle_ms={eager_settle_ms}ms probe_min_ms={probe_min_ms}ms reason={:?}",
                self.last_cold_reason.get()
            );

            // session_expired: 2秒以上放置後は TSF composition context がリセット済みの可能性大。
            // 古い eager_warmup_sent_ms を使って「elapsed >= 500ms → スリープなし」にすると、
            // TSF が cold なまま 'd' 等が literal になる（dえーた バグ）。
            // fresh な VK_DBE_HIRAGANA を送信して eager_warmup_sent_ms を更新し、500ms 待機を強制する。
            if session_expired {
                log::debug!("[h1-warmup] session expired → fresh VK_DBE_HIRAGANA 送信 (500ms待機を強制)");
                self.send_eager_tsf_warmup();
            }

            let eager_ms = self.eager_warmup_sent_ms.get();
            let now_ms = crate::hook::current_tick_ms();
            let eager_elapsed =
                if eager_ms != 0 { now_ms.saturating_sub(eager_ms) } else { u64::MAX };
            let use_eager = eager_ms != 0;

            // どのパスを通るかを明示的にログ（根本原因判別用）
            log::debug!(
                "[h1-warmup] cold={cold_n} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
                if use_eager { "eager" } else { "non-eager" },
                if eager_elapsed == u64::MAX { "∞".to_owned() } else { eager_elapsed.to_string() },
            );

            if use_eager {
                let remaining = eager_settle_ms.saturating_sub(eager_elapsed);
                if remaining == 0 {
                    // eager_settle_ms 以上経過しているが、GJI は WM_SETFOCUS の遅延処理
                    // (メッセージキュー滞留 500-900ms) で TSF context を再初期化することがある。
                    // FocusChange / SetOpenTrue の場合はこの再初期化レースが発生しやすいため、
                    // 新規 VK_DBE_HIRAGANA を送って再び 500ms 待機する。
                    // PassthroughConfirmKey 等の composition-only reset では不要。
                    let needs_re_warmup = matches!(
                        self.last_cold_reason.get(),
                        ColdReason::FocusChange | ColdReason::SetOpenTrue
                    );
                    if needs_re_warmup {
                        log::debug!(
                            "[h1-warmup] cold={cold_n} eager: {eager_elapsed}ms 経過 → 再warmup (GJI 再初期化レース対策)",
                        );
                        let refresh_inputs = [
                            make_tsf_key_input(VK_DBE_HIRAGANA, false),
                            make_tsf_key_input(VK_DBE_HIRAGANA, true),
                        ];
                        unsafe {
                            SendInput(
                                &refresh_inputs,
                                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                            );
                        }
                        const RE_WARMUP_MS: u64 = 500;
                        let re_warmup_sent_ms = crate::hook::current_tick_ms();
                        crate::tsf_observations::TsfReadinessProbe::new(
                            re_warmup_sent_ms, cold_n, probe_min_ms,
                        )
                        .wait_until_ready(RE_WARMUP_MS);
                        let actual_wait = crate::hook::current_tick_ms().saturating_sub(re_warmup_sent_ms);
                        log::debug!(
                            "[h1-warmup] cold={cold_n} 再warmup probe完了={actual_wait}ms",
                        );
                    } else {
                        let last_io = crate::tsf_observations::OBS_GJI_LAST_IO_MS
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let gji_idle =
                            crate::hook::current_tick_ms().saturating_sub(last_io);
                        log::debug!(
                            "[h1-warmup] cold={cold_n} eager: {eager_elapsed}ms 経過 → 即送信 (gji_idle={gji_idle}ms)",
                        );
                    }
                } else {
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager: elapsed={eager_elapsed}ms → probe (budget={eager_settle_ms}ms from warmup)",
                    );
                    // total_max_ms は warmup_sent_ms 起点の合計予算（remaining ではない）。
                    // probe 内で max_deadline = eager_ms + eager_settle_ms が計算される。
                    crate::tsf_observations::TsfReadinessProbe::new(
                        eager_ms, cold_n, probe_min_ms,
                    )
                    .wait_until_ready(eager_settle_ms);
                    let total_elapsed = crate::hook::current_tick_ms().saturating_sub(eager_ms);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} probe完了 warmup経過={total_elapsed}ms",
                    );
                }
            } else {
                // 投機的プローブ: VK_DBE_HIRAGANA (F2相当) を2連送する。
                // 1回目 (warmup): TSF composition context 初期化をトリガー（VK_IME_ON では不足）
                // 2回目 (probe):  WezTerm が 1回目を処理済みであることを FIFO で保証
                // VK_DBE_HIRAGANA はひらがなモードへの切替のため、既にひらがななら実質冪等。
                log::debug!("[h1-warmup] cold={cold_n} non-eager: VK_DBE_HIRAGANA warmup+probe 送信");
                let ime_on_probe = [
                    make_tsf_key_input(VK_DBE_HIRAGANA, false),
                    make_tsf_key_input(VK_DBE_HIRAGANA, true),
                    make_tsf_key_input(VK_DBE_HIRAGANA, false),
                    make_tsf_key_input(VK_DBE_HIRAGANA, true),
                ];
                let t_pre = crate::hook::current_tick_ms();
                unsafe {
                    SendInput(
                        &ime_on_probe,
                        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                    );
                }
                let elapsed = crate::hook::current_tick_ms().saturating_sub(t_pre);
                log::debug!("[h1-warmup] cold={cold_n} non-eager probe 完了 ({elapsed}ms)");
                // VK_DBE_HIRAGANA 単独では SendInput 完了後でも TSF 初期化に時間がかかる（実測: 40ms では不足）。
                // GJI I/O モニターが利用可能なら静止検出、なければ固定 sleep。
                let probe_sent_ms = crate::hook::current_tick_ms();
                crate::tsf_observations::TsfReadinessProbe::new(
                    probe_sent_ms, cold_n, probe_min_ms,
                )
                .wait_until_ready(eager_settle_ms);
                log::debug!("[h1-warmup] cold={cold_n} non-eager probe完了");
            }

        } else {
            cold_n = self.cold_start_count.get();
        }

        // warmup 完了 → ローマ字送信開始 (GJI idle 状態を記録)
        {
            let last_io = crate::tsf_observations::OBS_GJI_LAST_IO_MS
                .load(std::sync::atomic::Ordering::Relaxed);
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            log::debug!("[h1-send] cold={cold_n} sending romaji ({} chars), gji_idle={gji_idle}ms", chars.len());
        }

        // 同一 VK が連続する箇所（例 "nn"）でバッチに N↓N↓N↑N↑ を含めると、IME が
        // 2 つ目の N↓ をオートリピートと判定して破棄してしまう。
        // 同一 VK が連続する境界で run を分割し、各 run を別の SendInput で送る。
        let mut runs: Vec<&[(u16, bool)]> = Vec::new();
        let mut start = 0;
        for i in 1..chars.len() {
            if chars[i].0 == chars[i - 1].0 {
                runs.push(&chars[start..i]);
                start = i;
            }
        }
        runs.push(&chars[start..]);

        let total_runs = runs.len();

        for (run_idx, run) in runs.iter().enumerate() {
            log::debug!(
                "[h1-run] cold={cold_n} run={run_idx}/{total_runs} chars={}",
                run.len(),
            );
            let mut inputs = Vec::with_capacity(run.len() * 4);
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

        self.mark_composition_warm();
        // [観察用] 送信完了時刻を記録（GoogleJapaneseInputCandidateWindow OBJ_SHOW との delta 計算用）
        crate::OBS_LAST_SEND_MS.store(
            crate::hook::current_tick_ms(),
            std::sync::atomic::Ordering::Relaxed,
        );
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
            // VK_OEM_MINUS (0xBD, no-shift) = '-' は GJI ローマ字モードで「ー」として
            // composition に取り込まれる（composition context はリセットされない）。
            // これらは warm 状態を維持し、次の romaji を warmup sleep なしで即送信する。
            // その他の記号（句読点など）は composition を commit する可能性があるため cold にマーク。
            let keeps_composition = vk == 0xBD && !needs_shift;
            if keeps_composition {
                log::debug!("    send_char_as_tsf: VK 0x{vk:02X} は composition 継続 (ー系) → warm 維持");
            } else {
                self.mark_composition_cold(ColdReason::SymbolVkSent);
                self.send_eager_tsf_warmup();
            }
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
