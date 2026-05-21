use std::collections::HashMap;

use awase::config::OutputMode;
use awase::types::{AppKind, KeyAction, SpecialKey};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

pub use crate::tsf::output::ColdReason;
pub use crate::tsf::output::{INJECTED_MARKER, TSF_MARKER};
use crate::tsf::output::{kana_for_romaji_static, make_key_input_ex, make_tsf_key_input};

/// 出力セッションを RAII で管理するガード。
///
/// `begin()` で `OUTPUT_ACTIVE=true` をセット。
/// Drop 時に `OUTPUT_ACTIVE=false` にリセットし、`post_drain_output_queue()` を呼ぶ。
#[derive(Debug)]
pub(crate) struct OutputActiveGuard;

impl OutputActiveGuard {
    pub(crate) fn begin() -> Self {
        crate::OUTPUT_ACTIVE.store(true, std::sync::atomic::Ordering::Release);
        Self
    }
}

impl Drop for OutputActiveGuard {
    fn drop(&mut self) {
        crate::OUTPUT_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
        crate::post_drain_output_queue();
    }
}

/// モード別出力ディスパッチのトレイト。
///
/// `send_keys()` が `InjectionMode` ごとに match を繰り返す代わりに、
/// このトレイトで一本化する。
trait InjectionSender {
    fn send_char(&self, ch: char);
    fn send_romaji(&self, romaji: &str);
    fn send_key_sequence(&self, s: &str) {
        for ch in s.chars() {
            self.send_char(ch);
        }
    }
    fn mode_label(&self) -> &'static str;
}

struct UnicodeSender<'a>(&'a Output);
struct VkSender<'a>(&'a Output);
struct TsfSender<'a>(&'a Output);

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

/// `send_keys()` 1回分の出力セッション。
///
/// - `begin()` で `InjectionMode` を解決し `OutputActiveGuard` を取得する
/// - `sender()` で `InjectionSender` の動的ディスパッチオブジェクトを返す
/// - Drop 時に Guard が `OUTPUT_ACTIVE=false` + drain を自動実行する
struct OutputSession<'a> {
    output: &'a Output,
    mode: InjectionMode,
    _guard: OutputActiveGuard,
}

impl<'a> OutputSession<'a> {
    fn begin(output: &'a Output) -> Self {
        let mode = resolve_injection_mode();
        let _guard = OutputActiveGuard::begin();
        Self { output, mode, _guard }
    }

    fn sender(&self) -> Box<dyn InjectionSender + '_> {
        match self.mode {
            InjectionMode::Unicode => Box::new(UnicodeSender(self.output)),
            InjectionMode::Vk     => Box::new(VkSender(self.output)),
            InjectionMode::Tsf    => Box::new(TsfSender(self.output)),
        }
    }

    fn is_vk_mode(&self) -> bool {
        self.mode != InjectionMode::Unicode
    }
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

/// `u64::MAX` は「未送信」を意味するセンチネル値。ログ表示用に "∞" に変換する。
fn fmt_ms(ms: u64) -> String {
    if ms == u64::MAX { "∞".to_owned() } else { ms.to_string() }
}



/// 現在のフォーカス先から出力注入モードを決定する。
///
/// 優先順位:
///   1. config の `app_overrides.force_tsf` にマッチ → Tsf
///   2. config の `app_overrides.force_vk` にマッチ → Vk
///   3. AppKind::TsfNative → Vk
///   4. それ以外 (Win32 / Uwp) → Unicode
fn resolve_injection_mode() -> InjectionMode {
    // SAFETY: APP is a SingleThreadCell accessed only from the main message-loop thread.
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
        if app.platform_state.app_kind == AppKind::TsfNative {
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
    /// TSF composition context の warm/cold 状態管理。
    ///
    /// warm/cold epoch、last_send_ms、eager_warmup_sent_ms 等を集約する。
    /// 詳細は [`crate::tsf::probe::CompositionState`] を参照。
    pub composition: crate::tsf::probe::CompositionState,
}

/// `ensure_tsf_warm` の戻り値。warmup フローの結果を表す。
struct WarmupOutcome {
    /// F2 ウォームアップバッチが前置きされたか
    prepend_f2_warmup: bool,
    /// eager warmup パス（既存の F2 経由）を通ったか（Unicode 送信判定に使用）
    used_eager_path: bool,
    /// cold start シーケンス番号（ログ相関用）
    cold_n: u32,
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
            composition: crate::tsf::probe::CompositionState::new(),
        }
    }

    /// eager warmup F2 を送信した時刻（ms）を返す。0 = 未送信。
    /// WinEvent 観察コールバックが warmup からの経過時間をログするために使う。
    #[must_use]
    pub fn eager_warmup_sent_ms(&self) -> u64 {
        self.composition.eager_warmup_sent_ms()
    }

    /// シャドウ IME ON 状態を返す。
    /// FocusChange / notify_ime_open() で更新される。
    #[must_use]
    pub fn shadow_ime_on(&self) -> bool {
        self.composition.shadow_ime_on()
    }

    /// 最後の `send_keys` 完了からの経過時間（ms）。
    /// 一度も送信していない場合は `u64::MAX` を返す（= 永久に in-flight でない）。
    #[must_use]
    pub fn ms_since_last_send(&self) -> u64 {
        self.composition.ms_since_last_send()
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
        self.composition.mark_composition_cold(reason);
    }

    /// IME composition context をウォーム状態にマークする。
    ///
    /// 直前の NICOLA 出力バッチで warmup F2 が正常に送信され、
    /// TSF composition context が初期化済みであると分かっている場合に呼ぶ。
    pub fn mark_composition_warm(&self) {
        self.composition.mark_composition_warm();
    }

    /// 現在の composition_warm フラグを返す。
    ///
    /// `focus_epoch` が変化していれば前ウィンドウのウォーム状態は自動無効化される。
    pub fn is_composition_warm(&self) -> bool {
        self.composition.is_composition_warm()
    }

    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// `focus_epoch` をインクリメントし、前ウィンドウのウォーム状態を自動無効化する。
    /// 従来の `mark_composition_cold()` 呼び出しの代わりに使う（明示的なコールド化も同時に行う）。
    pub fn on_focus_changed(&self) {
        self.composition.on_focus_changed();
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
        self.composition.notify_ime_open(open);
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
        if !self.composition.shadow_ime_on() || !self.is_tsf_mode() {
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
        // SAFETY: warmup_inputs is a valid slice of INPUT structs for the duration of the call.
        unsafe {
            SendInput(
                &warmup_inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
        let ms = crate::hook::current_tick_ms();
        self.composition.set_eager_warmup_sent_ms(ms);
        log::debug!("[tsf-eager-warmup] VK_DBE_HIRAGANA 送信, eager_warmup_sent_ms={ms}ms");
    }

    /// `send_keys` 完了時刻を記録する内部ヘルパー。
    fn mark_send(&self) {
        self.composition.update_last_send_ms();
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
        // SAFETY: APP is a SingleThreadCell; this is only called from the main message-loop thread.
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
        // モード解決 + OutputActiveGuard 取得をセッションオブジェクトに委譲
        let session = OutputSession::begin(self);

        // mark_send() より前に elapsed を読む。mark_send() は last_send_ms を上書きするため、
        // 内部の send_romaji_as_tsf 等での ms_since_last_send() は常に ~0ms を返す。
        // 真の「前回送信からの経過時間」はここで記録する。
        let prev_elapsed_ms = self.ms_since_last_send();
        log::debug!(
            "send_keys: mode={:?} actions={actions:?} prev_elapsed={}ms",
            session.mode,
            fmt_ms(prev_elapsed_ms)
        );

        // NOTE: ImeDiagnosticSnapshot::capture("send_keys_pre") をここに置いてはいけない。
        // capture() は内部で GetGUIThreadInfo(100ms) + SendMessageTimeoutW(50ms×2) を
        // 呼ぶため、send_keys の中でメッセージポンプが走り Space 等の WH_KEYBOARD_LL
        // コールバックが SendInput より前に発火して "境界dえ" 等の race を起こす。

        // output in-flight guard の基準点を SendInput より前に設定する。
        self.mark_send();

        let sender = session.sender();
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
                    log::debug!("  → Char('{ch}') via {}", sender.mode_label());
                    sender.send_char(*ch);
                }
                KeyAction::Suppress => {
                    log::debug!("  → Suppress");
                }
                KeyAction::Romaji(s) => {
                    log::debug!("  → Romaji(\"{s}\") via {}", sender.mode_label());
                    sender.send_romaji(s);
                }
                KeyAction::KeySequence(s) => {
                    log::debug!("  → KeySequence(\"{s}\") via {}", sender.mode_label());
                    sender.send_key_sequence(s);
                }
            }
        }

        // VK/TSF モードで出力した場合、直後の IME ポーリングをガードするため
        // タイムスタンプを記録する（母音落ち「て→tえ」防止）。
        if session.is_vk_mode() {
            Self::mark_vk_output();
        }

        // executor が「output in-flight」判定に使う送信時刻を記録する。
        self.mark_send();
        // session ここで Drop → OutputActiveGuard::drop() → OUTPUT_ACTIVE=false + drain
    }

    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    #[allow(clippy::unused_self)]
    fn send_key(&self, vk: u16, is_keyup: bool) {
        let input = make_key_input(vk, is_keyup);
        // SAFETY: &[input] is a valid single-element slice for the duration of the call.
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
        // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
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
                // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
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
        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired = warm && elapsed < u64::MAX && elapsed > crate::timing::COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        log::debug!(
            "[vk-send] romaji={romaji:?} warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        if prepend_f2_warmup {
            if session_expired {
                log::debug!("[vk-warmup] session expired ({elapsed}ms) → F2-only先行バッチ (案A)");
            } else {
                log::debug!("[vk-warmup] cold → F2-only先行バッチ (案A)");
            }
            // SAFETY: IMM32 API; uses the foreground thread's IME context, valid during message loop.
            let conv_pre = unsafe { crate::ime::get_ime_conversion_mode_raw() };
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.map_or(false, |v| v & 0x0001 != 0),
                conv_pre.map_or(false, |v| v & 0x0010 != 0),
                conv_pre.map_or(false, |v| v & 0x0002 != 0),
            );
            // SAFETY: IMM32 API; sets conversion mode on the foreground window's IME context.
            // IMM32 経由で同期的にローマ字モードへ切り替え。
            unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

            let cold_n = self.composition.increment_cold_start_count();

            // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!("[h1-window] cold={cold_n} class={win_class}");

            // F2 を SendMessageTimeout で wndproc に直接届ける。
            // SendInput は OS 入力キューを経由するため、その後の probe（SendMessageTimeout）
            // より低優先度で処理され（QS_SENDMESSAGE > QS_INPUT）、probe が F2 処理前に
            // 完了してしまう競合が起きていた。
            // SendMessageTimeout は return 後に Chrome が WM_KEYDOWN を処理済みであることを保証する。
            log::debug!("[h1-run] cold={cold_n} F2 via SendMessageTimeout");
            let f2_sent_ms = crate::hook::current_tick_ms();
            // SAFETY: sends WM_KEYDOWN/WM_KEYUP to the foreground window via SendMessageTimeout; HWND validity checked internally.
            let f2_ok = unsafe { crate::ime::send_f2_via_sendmessage() };
            log::debug!("[h1-run] cold={cold_n} F2 SendMessageTimeout delivered={f2_ok}");

            // Chrome は F2 を wndproc で受け取った後、IPC でレンダラーに転送する。
            // SendInput のローマ字もレンダラー IPC キューに積まれるため、
            // レンダラーが F2 の IME 初期化を完了する前に 1 文字目が届くことがある。
            // GJI I/O 静止を待つことで、IME がレンダラー側で ready になってから送る。
            {
                // min_ms: IPC 1往復分の余裕 (Chrome レンダラーは通常 20ms 以内)
                // total_max_ms: GJI が応答しない場合でも 120ms で打ち切る
                const CHROME_PROBE_MIN_MS: u64 = 20;
                const CHROME_PROBE_MAX_MS: u64 = 120;
                let probe = crate::tsf::probe::TsfReadinessProbe::new(
                    f2_sent_ms,
                    cold_n,
                    CHROME_PROBE_MIN_MS,
                );
                probe.wait_until_ready(CHROME_PROBE_MAX_MS);
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
        // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
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
    fn ensure_tsf_warm(&self) -> WarmupOutcome {
        // composition_warm が false（コールド）のとき VK_DBE_HIRAGANA 先行バッチを送信する。
        // タイムアウト: 前回送信から COMPOSITION_TIMEOUT_MS 以上経過した場合も warm 扱いしない。
        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired = warm && elapsed < u64::MAX && elapsed > crate::timing::COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        log::debug!(
            "[tsf-send] warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        let cold_n = if prepend_f2_warmup {
            self.execute_cold_warmup(session_expired, elapsed)
        } else {
            self.composition.cold_start_count()
        };

        let used_eager_path = self.composition.eager_warmup_sent_ms() != 0;
        WarmupOutcome { prepend_f2_warmup, used_eager_path, cold_n }
    }

    /// TSF cold-start 時のウォームアップシーケンスを実行して cold-start シーケンス番号を返す。
    ///
    /// `ensure_tsf_warm` の `prepend_f2_warmup` ブランチを切り出したもの。
    fn execute_cold_warmup(&self, session_expired: bool, elapsed_ms: u64) -> u32 {
        use std::sync::atomic::Ordering::Relaxed;

        const VK_DBE_HIRAGANA: u16 = 0xF2;
        // cold 発生前の idle 時間が長い場合（ナビゲーション等）、GJI が TSF セッションを
        // リセットしている可能性があり、再初期化に FocusChange 相当の時間が必要。
        // 閾値は 10s: 2-9s 程度の「考える・少し読む」では GJI セッションが生存しており
        // I/O が発火せず probe が 1500ms タイムアウトしてしまうため、低すぎる閾値は NG。
        // 10s 以上の長期 idle（矢印キーナビゲーション等）では GJI セッションリセットが確実。
        if session_expired {
            log::debug!("[tsf-warmup] session expired ({elapsed_ms}ms) → F2-only先行バッチ (案A)");
        } else {
            log::debug!("[tsf-warmup] cold → F2-only先行バッチ (案A)");
        }
        // H4/H5 判定: pre-send で ROMAN=true なら IMM32 は正しいが TSF が無視している。
        // SAFETY: IMM32 API; uses the foreground thread's IME context, valid during message loop.
        let conv_pre = unsafe { crate::ime::get_ime_conversion_mode_raw() };
        log::debug!(
            "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
            conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
            conv_pre.map_or(false, |v| v & 0x0001 != 0),
            conv_pre.map_or(false, |v| v & 0x0010 != 0),
            conv_pre.map_or(false, |v| v & 0x0002 != 0),
        );
        // SAFETY: IMM32 API; sets conversion mode on the foreground window's IME context.
        // IMM32 経由で同期的にローマ字モードへ切り替え。
        unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

        let cold_n = self.composition.increment_cold_start_count();

        // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
        let win_class = unsafe { crate::ime::get_foreground_window_class() };
        log::debug!("[h1-window] cold={cold_n} class={win_class}");

        let long_idle = self.composition.idle_ms_at_last_cold() > crate::timing::LONG_IDLE_MS;
        // ColdReason に応じてウォームアップ待機時間を決定:
        //   FocusChange / SetOpenTrue / NativeF2Consumed:
        //     awase が物理キーを消費して VK_DBE_HIRAGANA を代わりに送るため、
        //     GJI から見ると FocusChange 相当の TSF 再初期化が発生しうる。
        //     実測で候補窓出現まで 1031ms かかることがあるため 1500ms を上限とする。
        //   PassthroughConfirmKey / ReinjectConfirmKey + long_idle:
        //     Enter/Space/Escape 後でも長期 idle 後は GJI セッションがリセットされ、
        //     500ms のバジェットでは不足する（kおのじしょう バグ）。1500ms に拡張する。
        //   その他（Enter/Space/記号等）: composition 再突入のみ → 500ms
        let cold_reason = self.composition.last_cold_reason();
        if cold_reason.is_confirm_key() && long_idle {
            log::debug!(
                "[h1-warmup] cold={cold_n} PassthroughConfirmKey/ReinjectConfirmKey + long idle \
                 ({}ms) → eager_settle_ms=1500ms",
                self.composition.idle_ms_at_last_cold()
            );
        }
        let eager_settle_ms: u64 = cold_reason.eager_settle_ms(long_idle);
        // ColdReason に応じた probe 最小待機時間（warmup_sent_ms 起点）:
        //   VK_DBE_HIRAGANA がキューに入ってから GJI が最初の I/O を開始するまでの
        //   実測下限。この時間内は GJI I/O 監視結果を信頼しない。
        let probe_min_ms: u64 = cold_reason.probe_min_ms(long_idle);
        log::debug!(
            "[h1-warmup] cold={cold_n} eager_settle_ms={eager_settle_ms}ms probe_min_ms={probe_min_ms}ms \
             reason={:?} long_idle={long_idle} idle_at_cold={}ms",
            self.composition.last_cold_reason(),
            self.composition.idle_ms_at_last_cold()
        );

        // session_expired: 2秒以上放置後は TSF composition context がリセット済みの可能性大。
        // 古い eager_warmup_sent_ms を使って「elapsed >= 500ms → スリープなし」にすると、
        // TSF が cold なまま 'd' 等が literal になる（dえーた バグ）。
        // fresh な VK_DBE_HIRAGANA を送信して eager_warmup_sent_ms を更新し、500ms 待機を強制する。
        if session_expired {
            log::debug!("[h1-warmup] cold={cold_n} session expired → fresh VK_DBE_HIRAGANA 送信 (500ms待機を強制)");
            self.send_eager_tsf_warmup();
        }

        let eager_ms = self.composition.eager_warmup_sent_ms();
        let now_ms = crate::hook::current_tick_ms();
        let eager_elapsed =
            if eager_ms != 0 { now_ms.saturating_sub(eager_ms) } else { u64::MAX };
        let use_eager = eager_ms != 0;

        // どのパスを通るかを明示的にログ（根本原因判別用）
        log::debug!(
            "[h1-warmup] cold={cold_n} path={} eager_ms={eager_ms} now_ms={now_ms} elapsed={}ms",
            if use_eager { "eager" } else { "non-eager" },
            fmt_ms(eager_elapsed),
        );

        if use_eager {
            let remaining = eager_settle_ms.saturating_sub(eager_elapsed);
            if remaining == 0 {
                // eager_settle_ms 以上経過しているが、GJI は WM_SETFOCUS の遅延処理
                // (メッセージキュー滞留 500-900ms) で TSF context を再初期化することがある。
                // FocusChange / SetOpenTrue / NativeF2Consumed の場合はこの再初期化レースが
                // 発生しやすいため、新規 VK_DBE_HIRAGANA を送って再び 500ms 待機する。
                // PassthroughConfirmKey 等の composition-only reset では不要。
                let needs_re_warmup = cold_reason.requires_settle();
                if needs_re_warmup {
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager: {eager_elapsed}ms 経過 → 再warmup (GJI 再初期化レース対策)",
                    );
                    let refresh_inputs = [
                        make_tsf_key_input(VK_DBE_HIRAGANA, false),
                        make_tsf_key_input(VK_DBE_HIRAGANA, true),
                    ];
                    // SAFETY: refresh_inputs is a valid array of INPUT structs for the duration of the call.
                    unsafe {
                        SendInput(
                            &refresh_inputs,
                            i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                        );
                    }
                    const RE_WARMUP_MS: u64 = 500;
                    let re_warmup_sent_ms = crate::hook::current_tick_ms();
                    crate::tsf::probe::TsfReadinessProbe::new(
                        re_warmup_sent_ms, cold_n, probe_min_ms,
                    )
                    .wait_until_ready(RE_WARMUP_MS);
                    let actual_wait = crate::hook::current_tick_ms().saturating_sub(re_warmup_sent_ms);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} 再warmup probe完了={actual_wait}ms",
                    );
                } else {
                    // eager F2 から時間が経過（elapsed >= eager_settle_ms）しており、
                    // gji_idle が大きい状態で即送信すると raw-tsf-literal false positive が発生する。
                    // （GJI candidate SHOW が 300ms を超えるため raw-tsf-literal がタイムアウト）
                    // → fresh F2 を送って TsfReadinessProbe で composition context を
                    //   再確認してから送信する。追加 ~140ms だが false positive を防げる。
                    let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
                    let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager: {eager_elapsed}ms 経過 (gji_idle={gji_idle}ms) \
                         → fresh F2 + probe (raw-tsf-literal false positive 防止)",
                    );
                    let refresh_inputs = [
                        make_tsf_key_input(VK_DBE_HIRAGANA, false),
                        make_tsf_key_input(VK_DBE_HIRAGANA, true),
                    ];
                    let fresh_f2_ms = crate::hook::current_tick_ms();
                    // SAFETY: refresh_inputs is a valid array of INPUT structs for the duration of the call.
                    unsafe {
                        SendInput(
                            &refresh_inputs,
                            i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                        );
                    }
                    crate::tsf::probe::TsfReadinessProbe::new(
                        fresh_f2_ms, cold_n, probe_min_ms,
                    )
                    .wait_until_ready(eager_settle_ms);
                    let actual_wait = crate::hook::current_tick_ms().saturating_sub(fresh_f2_ms);
                    log::debug!(
                        "[h1-warmup] cold={cold_n} eager→fresh probe完了={actual_wait}ms",
                    );
                }
            } else {
                log::debug!(
                    "[h1-warmup] cold={cold_n} eager: elapsed={eager_elapsed}ms → probe (budget={eager_settle_ms}ms from warmup)",
                );
                // total_max_ms は warmup_sent_ms 起点の合計予算（remaining ではない）。
                // probe 内で max_deadline = eager_ms + eager_settle_ms が計算される。
                crate::tsf::probe::TsfReadinessProbe::new(
                    eager_ms, cold_n, probe_min_ms,
                )
                .wait_until_ready(eager_settle_ms);
                let total_elapsed = crate::hook::current_tick_ms().saturating_sub(eager_ms);
                log::debug!(
                    "[h1-warmup] cold={cold_n} probe完了 warmup経過={total_elapsed}ms",
                );

                // probe が GJI 活動なしでタイムアウトした場合、または NativeF2Consumed /
                // SetOpenTrue の cold start では、GJI idle だけでは WezTerm の TSF
                // composition context が ready かを保証できない。
                //
                // 理由: probe は std::thread::sleep でブロックするため、その間に
                // WezTerm が発行した OBJ_NAMECHANGE WinEvent がキューに溜まるが
                // 処理されない。probe 完了後に即ローマ字送信すると、WezTerm の TSF が
                // まだ活性化処理中の場合、先頭の 1 文字（例: 't'）が PTY に素通りし、
                // 次の文字（例: 'o'）が IME に捕捉されて "tお" になる。
                //
                // 修正: fresh F2 を送り wait_for_tsf_cold_settle でメッセージをポンプする。
                // probe sleep 中に溜まった pending NAMECHANGE を即処理するため、
                // NativeF2Consumed/SetOpenTrue では追加遅延はほぼ 0ms で済む。
                let gji_last = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
                let probe_settled = gji_last >= eager_ms;
                let gji_monitor_ok = crate::tsf::observer::OBS_GJI_MONITOR_OK.load(Relaxed);

                let is_ime_init_cold = cold_reason.requires_settle();

                if (!probe_settled || is_ime_init_cold) && gji_monitor_ok {
                    // GJI probe timeout (no activity) または IME ON 初期化 cold start:
                    // TSF context が stale / 未確定の可能性あり → fresh F2 + settle。
                    //
                    // wait_for_tsf_cold_settle で OBJ_NAMECHANGE を reactive に待つ（上限 300ms）。
                    const SETTLE_TIMEOUT_MS: u32 = 300;
                    let nc_baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed);
                    let settle_reason = if !probe_settled {
                        "probe timeout (no GJI activity)"
                    } else {
                        "NativeF2Consumed/SetOpenTrue (GJI settled だが NAMECHANGE 未処理の可能性)"
                    };
                    log::debug!(
                        "[h1-warmup] cold={cold_n} {settle_reason} \
                        → fresh F2 + tsf_cold_settle (up to {SETTLE_TIMEOUT_MS}ms, nc_seq={nc_baseline})",
                    );
                    let refresh_inputs = [
                        make_tsf_key_input(VK_DBE_HIRAGANA, false),
                        make_tsf_key_input(VK_DBE_HIRAGANA, true),
                    ];
                    let fresh_f2_ms = crate::hook::current_tick_ms();
                    // SAFETY: refresh_inputs is a valid array of INPUT structs for the duration of the call.
                    unsafe {
                        SendInput(
                            &refresh_inputs,
                            i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                        );
                    }
                    let settled = wait_for_tsf_cold_settle(nc_baseline, SETTLE_TIMEOUT_MS);

                    // OBJ_NAMECHANGE 確認かつ GJI 活動なし（probe_settled=false）の場合、
                    // OBJ_NAMECHANGE は「WezTerm が F2 を処理した」シグナルだが
                    // GJI composition session 初期化の完了を意味しない。
                    // 直後にローマ字を送ると GJI がまだ初期化中で literal になる（raw TSF literal バグ）。
                    // → fresh F2 タイムスタンプ起点で GJI I/O 静止を待つ二次プローブを実施。
                    if settled && !probe_settled {
                        const GJI_POST_NAMECHANGE_MS: u64 = 300;
                        log::debug!(
                            "[h1-warmup] cold={cold_n} OBJ_NAMECHANGE後 GJI 二次プローブ (max {GJI_POST_NAMECHANGE_MS}ms)",
                        );
                        crate::tsf::probe::TsfReadinessProbe::new(
                            fresh_f2_ms, cold_n, 0,
                        )
                        .wait_until_ready(GJI_POST_NAMECHANGE_MS);
                        log::debug!("[h1-warmup] cold={cold_n} GJI 二次プローブ完了");
                    }
                }
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
            // SAFETY: ime_on_probe is a valid array of INPUT structs for the duration of the call.
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
            crate::tsf::probe::TsfReadinessProbe::new(
                probe_sent_ms, cold_n, probe_min_ms,
            )
            .wait_until_ready(eager_settle_ms);
            log::debug!("[h1-warmup] cold={cold_n} non-eager probe完了");
        }

        cold_n
    }

    /// VK run 分割送信: 同一 VK 連続境界でバッチを分割して IME のオートリピート誤検出を回避する。
    fn send_vk_runs(&self, chars: &[(u16, bool)], cold_n: u32) {
        use std::sync::atomic::Ordering::Relaxed;

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
            let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS
                .load(Relaxed);
            let run_gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            let vks: Vec<String> = run.iter().map(|&(v, s)| {
                if s { format!("S{v:02X}") } else { format!("{v:02X}") }
            }).collect();
            log::debug!(
                "[h1-run] cold={cold_n} run={run_idx}/{total_runs} gji={run_gji_idle}ms vks=[{}]",
                vks.join(","),
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
            // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
        }
    }

    fn send_romaji_as_tsf(&self, romaji: &str) {
        TsfSendPipeline::new(self).run(romaji);
    }

    /// 文字の送信方法をルックアップテーブルで解決する。
    ///
    /// `send_char_as_tsf` / `send_char_as_vk` が共通で使う 3 段ルックアップ。
    fn resolve_char(&self, ch: char) -> CharResolution<'_> {
        if let Some(romaji) = self.kana_to_romaji.get(&ch) {
            return CharResolution::Romaji(romaji);
        }
        if let Some(&(vk, shift)) = self.symbol_to_vk.get(&ch) {
            return CharResolution::Vk(vk, shift);
        }
        CharResolution::Unicode(ch)
    }

    /// 文字を TSF Sequential VK キーストロークとして送信する（WezTerm TSF モード用）
    ///
    /// かな文字はローマ字に逆変換してから `send_romaji_as_tsf` で送信する。
    /// 記号は symbol_to_vk テーブルで直接 VK コードに変換する。
    /// マッチしない場合は Unicode 直接出力にフォールバックする。
    fn send_char_as_tsf(&self, ch: char) {
        match self.resolve_char(ch) {
            CharResolution::Romaji(romaji) => {
                log::debug!("    send_char_as_tsf: '{ch}' → romaji \"{romaji}\"");
                self.send_romaji_as_tsf(romaji);
            }
            CharResolution::Vk(vk, needs_shift) => {
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
                // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
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
            }
            CharResolution::Unicode(ch) => {
                log::debug!("    send_char_as_tsf: '{ch}' (U+{:04X}) → fallback Unicode", ch as u32);
                self.send_unicode_char(ch);
            }
        }
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
        match self.resolve_char(ch) {
            CharResolution::Romaji(romaji) => {
                log::debug!("    send_char_as_vk: '{ch}' → romaji \"{romaji}\"");
                // Batched (1回の SendInput) を使うことで、後続キー（Enter reinject 等）との
                // 競合を防ぐ。per_key では K↓K↑ と A↓A↑ が別 SendInput になり、
                // 間に Enter が割り込むと "kあ" のような出力破壊が起きる。
                self.send_romaji_batched(romaji);
            }
            CharResolution::Vk(vk, needs_shift) => {
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
                // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
                unsafe {
                    SendInput(
                        &inputs,
                        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                    );
                }
            }
            CharResolution::Unicode(ch) => {
                log::debug!("    send_char_as_vk: '{ch}' (U+{:04X}) → fallback Unicode", ch as u32);
                self.send_unicode_char(ch);
            }
        }
    }

}

/// `send_char_as_tsf` / `send_char_as_vk` 共通の文字解決結果。
enum CharResolution<'a> {
    /// かな → ローマ字（VK / TSF 経由で IME に渡す）
    Romaji(&'a str),
    /// 記号 → (VK コード, Shift 要否)
    Vk(u16, bool),
    /// フォールバック（Unicode 直接出力）
    Unicode(char),
}

/// TSF 送信の 3 フェーズを管理するパイプライン。
///
/// - Phase 1 `warm_up`:  TSF composition context の初期化待ち（最大 1500ms）
/// - Phase 2 `transmit`: VK または Unicode kana で romaji を WezTerm に送信
/// - Phase 3 `verify`:   raw TSF literal 漏れを検出してリカバリをスケジュール
///
/// `send_romaji_as_tsf()` はこのパイプラインを呼び出す薄いラッパー。
struct TsfSendPipeline<'a> {
    output: &'a Output,
}

impl<'a> TsfSendPipeline<'a> {
    fn new(output: &'a Output) -> Self {
        Self { output }
    }

    fn run(&self, romaji: &str) {
        use std::sync::atomic::Ordering::Relaxed;

        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        // Phase 1: warmup
        let outcome = self.output.ensure_tsf_warm();

        // warmup 完了 → ローマ字送信開始 (GJI idle・IME conv 状態を記録)
        {
            let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            // SAFETY: IMM32 API; uses the foreground thread's IME context, valid during message loop.
            let conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) };
            log::debug!(
                "[h1-send] cold={} romaji={romaji:?} chars={} gji_idle={gji_idle}ms \
                 conv={} ROMAN={} NATIVE={}",
                outcome.cold_n,
                chars.len(),
                conv.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv.map_or(false, |v| v & 0x0010 != 0),
                conv.map_or(false, |v| v & 0x0001 != 0),
            );
        }

        // Phase 2: transmit
        let detector = crate::tsf::probe::LiteralDetector::new();
        let ze_bs_count = self.transmit(romaji, &chars, &outcome);
        self.output.mark_composition_warm();

        // Phase 3: verify
        self.verify(romaji, &outcome, detector, ze_bs_count);
    }

    /// Phase 2: VK run または Unicode kana を送信し、バックスペース数を返す。
    fn transmit(&self, romaji: &str, chars: &[(u16, bool)], outcome: &WarmupOutcome) -> usize {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            KEYEVENTF_UNICODE, KEYEVENTF_KEYUP,
        };
        use crate::tsf::output::TSF_MARKER;

        let unicode_kana: Option<char> = if outcome.prepend_f2_warmup && outcome.used_eager_path {
            kana_for_romaji_static(romaji)
        } else {
            None
        };

        if let Some(kana) = unicode_kana {
            let mut utf16_buf = [0u16; 2];
            let utf16 = kana.encode_utf16(&mut utf16_buf);
            log::debug!(
                "[h1-run] cold={} unicode TSF: {romaji:?} → '{}' (U+{:04X})",
                outcome.cold_n, kana, kana as u32,
            );
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
                            dwExtraInfo: TSF_MARKER,
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
                            dwExtraInfo: TSF_MARKER,
                        },
                    },
                });
            }
            // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
            unsafe {
                SendInput(
                    &inputs,
                    i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                );
            }
            1
        } else {
            self.output.send_vk_runs(chars, outcome.cold_n);
            chars.len()
        }
    }

    /// Phase 3: raw TSF literal 検出と回収スケジュール。
    fn verify(
        &self,
        romaji: &str,
        outcome: &WarmupOutcome,
        detector: crate::tsf::probe::LiteralDetector,
        ze_bs_count: usize,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        use crate::tsf::probe::DetectionResult;

        let gji_active = crate::tsf::observer::OBS_GJI_MONITOR_OK.load(Relaxed);
        if !outcome.prepend_f2_warmup || !gji_active {
            return;
        }

        let t_send = crate::hook::current_tick_ms();
        let detection = detector.detect(crate::timing::RAW_TSF_LITERAL_DETECT_MS);
        let elapsed_ms = crate::hook::current_tick_ms().saturating_sub(t_send);

        match detection {
            DetectionResult::CompositionConfirmed => {
                log::debug!(
                    "[raw-tsf-literal] cold={} composition confirmed ({elapsed_ms}ms)",
                    outcome.cold_n
                );
            }
            DetectionResult::SuspectedLiteral => {
                let consecutive = self.output.composition.consecutive_count();
                if consecutive == 0 {
                    log::warn!(
                        "[raw-tsf-literal] cold={} raw TSF literal suspected \
                        ({elapsed_ms}ms) \
                        → backspace ×{ze_bs_count} + re-send {romaji:?} scheduled + mark cold",
                        outcome.cold_n,
                    );
                    crate::RAW_TSF_LITERAL.backs.store(ze_bs_count, Relaxed);
                    *crate::RAW_TSF_LITERAL.romaji
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = romaji.to_string();
                } else {
                    log::warn!(
                        "[raw-tsf-literal] cold={} consecutive raw-tsf-literal fire \
                        (count={}) → likely false positive, giving up without backspace",
                        outcome.cold_n,
                        consecutive + 1,
                    );
                }
                self.output.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
            }
        }
    }
}

/// INPUT 構造体を作成するヘルパー（INJECTED_MARKER 固定）
const fn make_key_input(vk: u16, is_keyup: bool) -> INPUT {
    make_key_input_ex(vk, is_keyup, INJECTED_MARKER)
}

impl awase::platform::CompositionOutput for Output {
    fn send_romaji(&self, romaji: &str) {
        // モード判定は resolve_injection_mode() が行う。
        // 現状は TSF / VK Batched / Unicode の3モードを自動選択する。
        match resolve_injection_mode() {
            InjectionMode::Vk => self.send_romaji_batched(romaji),
            InjectionMode::Tsf => self.send_romaji_as_tsf(romaji),
            InjectionMode::Unicode => self.send_romaji_as_unicode(romaji),
        }
    }

    fn send_kana_char(&self, ch: char) {
        self.send_char_as_tsf(ch);
    }

    fn is_composition_warm(&self) -> bool {
        self.is_composition_warm()
    }

    fn mark_cold_focus_change(&self) {
        self.mark_composition_cold(ColdReason::FocusChange);
    }

    fn mark_cold_confirm_key(&self) {
        self.mark_composition_cold(ColdReason::PassthroughConfirmKey);
    }

    fn mark_cold_ime_toggle(&self) {
        self.mark_composition_cold(ColdReason::SetOpenTrue);
    }

    fn notify_ime_open(&self, open: bool) {
        self.notify_ime_open(open);
    }

    fn on_focus_changed(&self) {
        self.on_focus_changed();
    }
}

/// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼ぶ。`flush_raw_tsf_literal_backspaces` の後に呼ぶこと。
///
/// `RAW_TSF_LITERAL.romaji` に退避されたローマ字を読み取り、`send_romaji_as_tsf` で再送する。
/// cold 状態（RawTsfLiteralRecovery）で呼ばれるため warmup probe が走り正しく compose される。
/// drain キーの前に呼ぶことで「backspace → raw TSF literal char → drain keys」の順を保証する。
impl Output {
    pub fn flush_raw_tsf_literal_romaji(&self) {
        let romaji = {
            let mut guard = crate::RAW_TSF_LITERAL.romaji
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        if romaji.is_empty() {
            return;
        }
        log::debug!("[raw-tsf-literal] re-sending raw TSF literal romaji={romaji:?}");
        self.send_romaji_as_tsf(&romaji);
    }

    /// raw TSF literal 回収を一括実行: backspace 送信 → romaji 再送。
    ///
    /// WM_DRAIN_OUTPUT_QUEUE ハンドラから呼ぶ。drain keys より前に実行すること。
    pub fn flush_raw_tsf_literal_recovery(&self) {
        flush_raw_tsf_literal_backspaces();
        self.flush_raw_tsf_literal_romaji();
    }
}

pub use crate::tsf::output::flush_raw_tsf_literal_backspaces;

/// TSF cold-start 後の composition context 初期化完了を reactive に待つ。
///
/// fresh F2 送信直後に呼ぶ。OBJ_NAMECHANGE WinEvent か タイムアウトまで待機する。
///
/// WezTerm は TSF composition ウィンドウ名をひらがなモード切替時に更新する (~125ms)。
/// このイベントで早期終了する。発火しない場合は timeout_ms まで待つ。
///
/// # Re-entrancy safety
/// OUTPUT_ACTIVE=true（send_keys スコープ）でメッセージループを動かしながら OBJ_NAMECHANGE を待つ。
/// `MsgWaitForMultipleObjects` を廃止し、`win32_async::block_on` + `sleep_ms` で実装。
///
/// Returns `true` = OBJ_NAMECHANGE 検出、`false` = タイムアウト
fn wait_for_tsf_cold_settle(nc_baseline: u32, timeout_ms: u32) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let settled = win32_async::block_on(settle_async(nc_baseline, timeout_ms));
    // drain は OutputActiveGuard::drop が行うため、ここでは呼ばない。

    let nc_fired = crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed) != nc_baseline;
    log::debug!(
        "[tsf-settle] → {} (nc_fired={nc_fired})",
        if settled { "OBJ_NAMECHANGE" } else { "timeout" },
    );
    settled
}

/// OBJ_NAMECHANGE または タイムアウト まで非ブロッキングで待つ。
/// block_on の内部ループがメッセージをポンプするため WinEvent コールバックが発火し、
/// `OBS_FOCUS_NAMECHANGE_SEQ` が更新される。
async fn settle_async(nc_baseline: u32, timeout_ms: u32) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    const POLL_MS: u32 = 5;

    let deadline_ms = crate::hook::current_tick_ms() + u64::from(timeout_ms);

    loop {
        if crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed) != nc_baseline {
            return true;
        }
        let now = crate::hook::current_tick_ms();
        if now >= deadline_ms {
            return false;
        }
        let remaining = u32::try_from(deadline_ms.saturating_sub(now)).unwrap_or(u32::MAX);
        win32_async::sleep_ms(remaining.min(POLL_MS)).await;
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    // ── settle_async テスト（Windows のみ）──────────────────────────────────────

    /// テスト間でグローバル OBS_FOCUS_NAMECHANGE_SEQ が競合しないようシリアライズ
    #[cfg(windows)]
    static SETTLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// baseline がすでに変化していれば即 true を返す
    #[test]
    #[cfg(windows)]
    fn settle_returns_true_when_already_changed() {
        let _g = SETTLE_TEST_LOCK.lock().unwrap();
        // seq をインクリメントして baseline とずらす
        let baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = win32_async::block_on(settle_async(baseline, 500));
        assert!(result, "seq already != baseline → should return true");
    }

    /// NAMECHANGE が来ない場合は timeout_ms 後に false を返す
    #[test]
    #[cfg(windows)]
    fn settle_times_out_when_no_namechange() {
        let _g = SETTLE_TEST_LOCK.lock().unwrap();
        let current = crate::OBS_FOCUS_NAMECHANGE_SEQ
            .load(std::sync::atomic::Ordering::SeqCst);

        let start = std::time::Instant::now();
        let result = win32_async::block_on(settle_async(current, 100));
        let elapsed = start.elapsed().as_millis();

        assert!(!result, "no NAMECHANGE → should timeout with false");
        assert!(elapsed >= 60, "timed out too early: {elapsed}ms");
        assert!(elapsed < 500, "timed out too late: {elapsed}ms");
    }

    /// 待機中に別タスクから seq が変化したら true を返す
    #[test]
    #[cfg(windows)]
    fn settle_returns_true_on_namechange_during_wait() {
        let _g = SETTLE_TEST_LOCK.lock().unwrap();
        let baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ
            .load(std::sync::atomic::Ordering::SeqCst);

        let result = win32_async::block_on(async {
            // 30ms 後に seq を変化させる spawn_local タスク
            win32_async::spawn_local(async {
                win32_async::sleep_ms(30).await;
                crate::OBS_FOCUS_NAMECHANGE_SEQ
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // settle_async は 5ms ポーリングなので次のポーリングで検出
            });

            settle_async(baseline, 500).await
        });

        assert!(result, "NAMECHANGE fired during wait → should return true");
    }

    // ── ColdReason impl メソッドテスト ────────────────────────────────────────

    #[test]
    fn cold_reason_eager_settle_ms_short_idle() {
        assert_eq!(ColdReason::FocusChange.eager_settle_ms(false), 1500);
        assert_eq!(ColdReason::NativeF2Consumed.eager_settle_ms(false), 1500);
        assert_eq!(ColdReason::SetOpenTrue.eager_settle_ms(false), 1500);
        assert_eq!(ColdReason::PassthroughConfirmKey.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::ReinjectConfirmKey.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::SessionExpired.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::SymbolVkSent.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::F2NonTsf.eager_settle_ms(false), 500);
        assert_eq!(ColdReason::RawTsfLiteralRecovery.eager_settle_ms(false), 500);
    }

    #[test]
    fn cold_reason_eager_settle_ms_long_idle() {
        // long_idle=true → ConfirmKey 系のみ延長
        assert_eq!(ColdReason::PassthroughConfirmKey.eager_settle_ms(true), 1500);
        assert_eq!(ColdReason::ReinjectConfirmKey.eager_settle_ms(true), 1500);
        // 他は不変
        assert_eq!(ColdReason::SessionExpired.eager_settle_ms(true), 500);
        assert_eq!(ColdReason::SymbolVkSent.eager_settle_ms(true), 500);
    }

    #[test]
    fn cold_reason_probe_min_ms() {
        assert_eq!(ColdReason::FocusChange.probe_min_ms(false), 300);
        assert_eq!(ColdReason::NativeF2Consumed.probe_min_ms(false), 300);
        assert_eq!(ColdReason::SetOpenTrue.probe_min_ms(false), 300);
        assert_eq!(ColdReason::SessionExpired.probe_min_ms(false), 200);
        assert_eq!(ColdReason::PassthroughConfirmKey.probe_min_ms(false), 50);
        assert_eq!(ColdReason::ReinjectConfirmKey.probe_min_ms(false), 50);
        assert_eq!(ColdReason::PassthroughConfirmKey.probe_min_ms(true), 300);
        assert_eq!(ColdReason::SymbolVkSent.probe_min_ms(false), 30);
        assert_eq!(ColdReason::F2NonTsf.probe_min_ms(false), 100);
        assert_eq!(ColdReason::RawTsfLiteralRecovery.probe_min_ms(false), 100);
    }

    #[test]
    fn cold_reason_is_confirm_key() {
        assert!(ColdReason::PassthroughConfirmKey.is_confirm_key());
        assert!(ColdReason::ReinjectConfirmKey.is_confirm_key());
        assert!(!ColdReason::FocusChange.is_confirm_key());
        assert!(!ColdReason::SessionExpired.is_confirm_key());
        assert!(!ColdReason::RawTsfLiteralRecovery.is_confirm_key());
    }

    #[test]
    fn cold_reason_requires_settle() {
        assert!(ColdReason::FocusChange.requires_settle());
        assert!(ColdReason::NativeF2Consumed.requires_settle());
        assert!(ColdReason::SetOpenTrue.requires_settle());
        assert!(!ColdReason::PassthroughConfirmKey.requires_settle());
        assert!(!ColdReason::SessionExpired.requires_settle());
        assert!(!ColdReason::RawTsfLiteralRecovery.requires_settle());
    }

    // ── Output 状態管理テスト ───────────────────────────────────────────────────

    fn make_output() -> Output {
        Output::new(OutputMode::Unicode)
    }

    #[test]
    fn output_starts_cold() {
        let o = make_output();
        assert!(!o.is_composition_warm(), "Output should start cold");
    }

    #[test]
    fn output_mark_warm_then_cold() {
        let o = make_output();
        o.mark_composition_warm();
        assert!(o.is_composition_warm(), "should be warm after mark_composition_warm");
        o.mark_composition_cold(ColdReason::FocusChange);
        assert!(!o.is_composition_warm(), "should be cold after mark_composition_cold");
    }

    #[test]
    fn output_focus_change_invalidates_warm() {
        let o = make_output();
        o.mark_composition_warm();
        assert!(o.is_composition_warm());
        o.on_focus_changed();
        assert!(!o.is_composition_warm(), "focus change should invalidate warm state");
    }

    #[test]
    fn output_rewarm_after_focus_change() {
        let o = make_output();
        o.mark_composition_warm();
        o.on_focus_changed();
        o.mark_composition_warm();
        assert!(o.is_composition_warm(), "can warm again after focus change + re-warm");
    }

    #[test]
    fn output_consecutive_count_increments_on_raw_tsf_literal_recovery() {
        let o = make_output();
        assert_eq!(o.composition.consecutive_count(), 0);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 1);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 2);
    }

    #[test]
    fn output_consecutive_count_resets_on_other_cold_reason() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 2);
        o.mark_composition_cold(ColdReason::FocusChange);
        assert_eq!(o.composition.consecutive_count(), 0, "non-recovery cold should reset count");
    }

    #[test]
    fn output_consecutive_count_resets_on_warm() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 2);
        o.mark_composition_warm();
        assert_eq!(o.composition.consecutive_count(), 0, "warm should reset consecutive count");
    }

    #[test]
    fn output_consecutive_count_resets_on_focus_change() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.consecutive_count(), 1);
        o.on_focus_changed();
        assert_eq!(o.composition.consecutive_count(), 0, "focus change should reset consecutive count");
    }

    #[test]
    fn output_last_cold_reason_tracks_latest() {
        let o = make_output();
        o.mark_composition_cold(ColdReason::SessionExpired);
        assert_eq!(o.composition.last_cold_reason(), ColdReason::SessionExpired);
        o.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        assert_eq!(o.composition.last_cold_reason(), ColdReason::RawTsfLiteralRecovery);
    }

    // ── RAW_TSF_LITERAL グローバル構造体テスト ──────────────────────────────────

    #[test]
    fn raw_tsf_literal_backs_roundtrip() {
        use std::sync::atomic::Ordering::Relaxed;
        crate::RAW_TSF_LITERAL.backs.store(3, Relaxed);
        let n = crate::RAW_TSF_LITERAL.backs.swap(0, Relaxed);
        assert_eq!(n, 3);
        assert_eq!(crate::RAW_TSF_LITERAL.backs.load(Relaxed), 0);
    }

    #[test]
    fn raw_tsf_literal_romaji_roundtrip() {
        {
            let mut guard = crate::RAW_TSF_LITERAL.romaji.lock().unwrap();
            *guard = "konnichiwa".to_string();
        }
        let taken = {
            let mut guard = crate::RAW_TSF_LITERAL.romaji.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        assert_eq!(taken, "konnichiwa");
        let now_empty = crate::RAW_TSF_LITERAL.romaji.lock().unwrap().clone();
        assert!(now_empty.is_empty());
    }

    // ── 既存テスト ─────────────────────────────────────────────────────────────

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
