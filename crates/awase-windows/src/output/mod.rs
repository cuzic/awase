use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use awase::config::OutputMode;
use awase::types::{AppKind, KeyAction, SpecialKey};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

pub use crate::tsf::output::ColdReason;
pub use crate::tsf::output::{INJECTED_MARKER, TSF_MARKER};
use crate::tsf::output::{kana_for_romaji_static, make_key_input_ex, make_tsf_key_input};

pub(crate) mod sender;
pub(crate) use sender::{InjectionMode, OutputSession};

/// 出力セッションを RAII で管理するガード（参照カウント方式）。
///
/// `begin()` で深度をインクリメントし、深度 0→1 のとき `OUTPUT_ACTIVE=true` をセット。
/// Drop 時に深度をデクリメントし、深度 1→0 のとき `OUTPUT_ACTIVE=false` + drain。
///
/// TSF probe 延期中は `TsfProbeData` がガードを保持し続けることで、
/// `OutputSession` が drop しても `OUTPUT_ACTIVE` が維持される。
#[derive(Debug)]
pub(crate) struct OutputActiveGuard;

static OUTPUT_ACTIVE_DEPTH: AtomicU32 = AtomicU32::new(0);

impl OutputActiveGuard {
    pub(crate) fn begin() -> Self {
        let prev = OUTPUT_ACTIVE_DEPTH.fetch_add(1, Ordering::AcqRel);
        if prev == 0 {
            crate::OUTPUT_ACTIVE.store(true, Ordering::Release);
        }
        Self
    }
}

impl Drop for OutputActiveGuard {
    fn drop(&mut self) {
        let prev = OUTPUT_ACTIVE_DEPTH.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            crate::OUTPUT_ACTIVE.store(false, Ordering::Release);
            crate::post_drain_output_queue();
        }
    }
}

/// VK_LSHIFT の仮想キーコード
const VK_LSHIFT: u16 = 0xA0;

/// `u64::MAX` は「未送信」を意味するセンチネル値。ログ表示用に "∞" に変換する。
pub(crate) fn fmt_ms(ms: u64) -> String {
    if ms == u64::MAX { "∞".to_owned() } else { ms.to_string() }
}

/// VK/TSF モードでの最終 SendInput 時刻（`mark_vk_output` が書き込む）。
///
/// `execute_one` → `send_keys` のコールスタックから `with_app` を呼ぶと再入 UB になるため、
/// `last_hook_activity_ms` の代替として atomic に書き込む。
/// 読み取り側では `last_hook_activity_ms.max(LAST_VK_OUTPUT_MS)` を使う。
pub static LAST_VK_OUTPUT_MS: AtomicU64 = AtomicU64::new(0);



/// `resolve_injection_mode_from` に渡すコンテキスト。
///
/// APP グローバルから必要なフィールドだけを切り出すことで、
/// ロジック部（`resolve_injection_mode_from`）を純粋関数としてテスト可能にする。
pub(crate) struct InjectionModeContext<'a> {
    /// 直前のフォーカス情報 (process_id, window_class)。未取得なら None。
    pub focus_info: Option<(u32, &'a str)>,
    /// config の app_overrides（force_tsf / force_vk / force_text / force_bypass）
    pub overrides: &'a awase::config::AppOverrides,
    /// 現在フォーカス中のアプリの種別（TSF ネイティブかどうか）
    pub app_kind: AppKind,
}

/// `InjectionModeContext` から注入モードを決定する純粋関数。
///
/// APP グローバルへの直アクセスを持たないため、ユニットテストで直接呼び出せる。
///
/// 優先順位:
///   1. config の `app_overrides.force_tsf` にマッチ → Tsf
///   2. config の `app_overrides.force_vk` にマッチ → Vk
///   3. AppKind::TsfNative → Vk
///   4. それ以外 (Win32 / Uwp) → Unicode
fn resolve_injection_mode_from(ctx: &InjectionModeContext<'_>) -> InjectionMode {
    if let Some((pid, class)) = ctx.focus_info {
        if crate::runtime::is_force_tsf(ctx.overrides, pid, class) {
            return InjectionMode::Tsf;
        }
        if crate::runtime::is_force_vk(ctx.overrides, pid, class) {
            return InjectionMode::Vk;
        }
    }
    if ctx.app_kind == AppKind::TsfNative {
        InjectionMode::Vk
    } else {
        InjectionMode::Unicode
    }
}

/// APP グローバルから `InjectionModeContext` を構築して注入モードを返す薄いシム。
///
/// APP への直アクセスはこの関数にのみ集約する。ロジック本体は
/// `resolve_injection_mode_from` を参照のこと。
fn resolve_injection_mode() -> InjectionMode {
    crate::with_app_ref(|app| {
        let focus_info = app
            .executor
            .platform
            .focus
            .last_focus_info
            .as_ref()
            .map(|(pid, class)| (*pid, class.as_str()));
        let ctx = InjectionModeContext {
            focus_info,
            overrides: &app.executor.platform.focus.overrides,
            app_kind: app.platform_state.app_kind,
        };
        resolve_injection_mode_from(&ctx)
    })
    .unwrap_or(InjectionMode::Unicode)
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
    /// TIMER_TSF_PROBE で処理中の保留 TSF/VK probe データ。
    ///
    /// `send_romaji_as_tsf` / `send_romaji_batched` が cold start 時に設定し、
    /// `advance_tsf_probe` がタイマーごとに状態を進める。
    /// `_guard` により OUTPUT_ACTIVE が保留期間中維持される。
    pub(crate) pending_tsf: std::cell::RefCell<Option<TsfProbeData>>,
    /// フォーカス変更直後の TSF モード確定前にキーを一時保留するゲート。
    ///
    /// PendingWarmup 状態中のみキーを保留し、run_with_prefetched 完了後に
    /// Probing または Bypass に遷移して保留キーを再処理する。
    pub tsf_gate: crate::tsf::gate::TsfGate,
    /// pending_tsf プローブ進行中に受信した直接 VK を順序保証のため保留するキュー。
    ///
    /// `send_char_as_tsf` が Vk ブランチを通り `pending_tsf` が Some の場合に積み上げ、
    /// `do_transmit_tsf` / `ChromeProbe` 完了後に romaji の直後に送出する。
    /// これにより「F2 → ー → ba」→「F2 → ba → ー」の順序逆転バグを防ぐ。
    deferred_vks_during_probe: std::cell::RefCell<Vec<(u16, bool)>>,
}

/// TSF/VK probe の現在フェーズ
pub(crate) enum TsfProbePhase {
    /// GJI 静止待ち（TSF warmup 後または Chrome F2 後）
    GjiProbe {
        probe: crate::tsf::probe::TsfReadinessProbe,
        total_max_ms: u64,
        /// プローブ完了後に NAMECHANGE 確認が必要か（eager_probe_with_settle パスのみ true）
        needs_settle_check: bool,
        cold_reason: ColdReason,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
    },
    /// OBJ_NAMECHANGE 待ち（GJI probe + needs_settle_check の次フェーズ）
    NameChangeWait {
        nc_baseline: u32,
        deadline_ms: u64,
        fresh_f2_ms: u64,
        probe_settled: bool,
        cold_reason: ColdReason,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
    },
    /// GJI 二次プローブ（OBJ_NAMECHANGE 後）
    SecondaryGjiProbe {
        probe: crate::tsf::probe::TsfReadinessProbe,
        total_max_ms: u64,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
    },
    /// raw TSF literal 検出待ち（TSF 送信後の verify フェーズ）
    LiteralDetect {
        detector: crate::tsf::probe::LiteralDetector,
        ze_bs_count: usize,
        deadline_ms: u64,
    },
    /// Chrome/VK probe（F2 後の GJI 静止待ち）
    ChromeProbe {
        probe: crate::tsf::probe::TsfReadinessProbe,
        total_max_ms: u64,
    },
}

/// TSF/VK probe の保留データ。
///
/// `Output::pending_tsf` に格納し、TIMER_TSF_PROBE ハンドラが
/// `advance_tsf_probe` を呼んで状態を進める。
/// `_guard` によって OUTPUT_ACTIVE が維持される。
pub(crate) struct TsfProbeData {
    pub romaji: String,
    pub cold_n: u32,
    pub phase: TsfProbePhase,
    pub _guard: OutputActiveGuard,
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
            pending_tsf: std::cell::RefCell::new(None),
            tsf_gate: crate::tsf::gate::TsfGate::new(),
            deferred_vks_during_probe: std::cell::RefCell::new(Vec::new()),
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

    /// `composition_warm_epoch` のみ 0 にリセットする（`eager_warmup_sent_ms` は保持）。
    ///
    /// フォーカス遷移直後の最初のキーで呼ぶ。
    pub fn suppress_warm_epoch(&self) {
        self.composition.suppress_warm_epoch();
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
        self.deferred_vks_during_probe.borrow_mut().clear();
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
        crate::OBS_FOCUS_NAMECHANGE_SEQ.store(0, Ordering::Relaxed);
        // VK_DBE_HIRAGANA (F2) を送信: VK_IME_ON (0x16) は IME ON 状態をセットするだけで
        // TSF composition context の初期化をトリガーしない。WezTerm は物理 F2 受信時に
        // TSF composition を初期化するため、同等の VK_DBE_HIRAGANA を送る必要がある。
        // SAFETY: SendInput via send_vk_dbe_hiragana_pair; called from message-loop thread.
        let ms = unsafe { crate::tsf::send::send_vk_dbe_hiragana_pair() };
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
    ///
    /// `with_app` は `execute_one` からの再入 UB を避けるため使用不可。
    /// グローバル atomic に書き込み、読み取り側で `last_hook_activity_ms` と max を取る。
    fn mark_vk_output() {
        LAST_VK_OUTPUT_MS.store(crate::hook::current_tick_ms(), Ordering::Relaxed);
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
    /// cold 時は F2 を先行送信してから GJI プローブを開始し（ノンブロッキング）、
    /// TIMER_TSF_PROBE が `ChromeProbe` フェーズを進めてローマ字を送信する。
    fn send_romaji_batched(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired =
            warm && elapsed < u64::MAX && elapsed > crate::timing::COMPOSITION_TIMEOUT_MS;
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
            // SAFETY: IMM32 API; uses the foreground thread's IME context.
            let conv_pre = unsafe { crate::ime::get_ime_conversion_mode_raw() };
            log::debug!(
                "[cold-diag] pre-send conv={} NATIVE={} ROMAN={} KATAKANA={}",
                conv_pre.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv_pre.map_or(false, |v| v & 0x0001 != 0),
                conv_pre.map_or(false, |v| v & 0x0010 != 0),
                conv_pre.map_or(false, |v| v & 0x0002 != 0),
            );
            // SAFETY: IMM32 API; sets conversion mode on the foreground window's IME context.
            unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

            let cold_n = self.composition.increment_cold_start_count();
            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!("[h1-window] cold={cold_n} class={win_class}");

            log::debug!("[h1-run] cold={cold_n} F2 via SendMessageTimeout");
            let f2_sent_ms = crate::hook::current_tick_ms();
            // SAFETY: sends WM_KEYDOWN/WM_KEYUP to the foreground window via SendMessageTimeout.
            let f2_ok = unsafe { crate::ime::send_f2_via_sendmessage() };
            log::debug!("[h1-run] cold={cold_n} F2 SendMessageTimeout delivered={f2_ok}");

            // ノンブロッキング Chrome プローブを開始
            const CHROME_PROBE_MIN_MS: u64 = 20;
            const CHROME_PROBE_MAX_MS: u64 = 120;
            let probe = crate::tsf::probe::TsfReadinessProbe::new(
                f2_sent_ms,
                cold_n,
                CHROME_PROBE_MIN_MS,
            );
            let guard = OutputActiveGuard::begin();
            *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                romaji: romaji.to_string(),
                cold_n,
                phase: TsfProbePhase::ChromeProbe { probe, total_max_ms: CHROME_PROBE_MAX_MS },
                _guard: guard,
            });
            // WindowsPlatform::send_keys が pending_tsf を見て TIMER_TSF_PROBE をセットする
            return;
        }

        // warm パス: 即座にバッチ送信
        self.send_romaji_batch_immediate(romaji, &chars);
        self.mark_composition_warm();
    }

    /// ローマ字を即座にバッチ送信する（重畳順）。
    /// warm パスおよび `advance_tsf_probe` の ChromeProbe 完了時に呼ぶ。
    fn send_romaji_batch_immediate(&self, romaji: &str, chars: &[(u16, bool)]) {
        let mut inputs = Vec::with_capacity(chars.len() * 4);
        for &(vk, needs_shift) in chars {
            if needs_shift {
                inputs.push(make_key_input(VK_LSHIFT, false));
            }
            inputs.push(make_key_input(vk, false));
        }
        for &(vk, needs_shift) in chars {
            inputs.push(make_key_input(vk, true));
            if needs_shift {
                inputs.push(make_key_input(VK_LSHIFT, true));
            }
        }
        log::debug!("[vk-send] romaji={romaji:?} batch {} inputs", inputs.len());
        // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of the call.
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
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
        use std::sync::atomic::Ordering::Relaxed;

        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        let warm = self.is_composition_warm();
        let elapsed = self.ms_since_last_send();
        let session_expired =
            warm && elapsed < u64::MAX && elapsed > crate::timing::COMPOSITION_TIMEOUT_MS;
        let prepend_f2_warmup = !warm || session_expired;
        let used_eager_path = self.composition.eager_warmup_sent_ms() != 0;

        log::debug!(
            "[tsf-send] warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        if prepend_f2_warmup {
            // ノンブロッキング warmup を開始して pending_tsf に保留
            let started = crate::tsf::cold_warmup::ColdWarmupSequence::new(self)
                .run_start(session_expired, elapsed);
            let cold_n = started.probe.cold_n;
            let guard = OutputActiveGuard::begin();
            *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                romaji: romaji.to_string(),
                cold_n,
                phase: TsfProbePhase::GjiProbe {
                    probe: started.probe,
                    total_max_ms: started.total_max_ms,
                    needs_settle_check: started.needs_settle_check,
                    cold_reason: started.cold_reason,
                    prepend_f2_warmup,
                    used_eager_path,
                },
                _guard: guard,
            });
            // WindowsPlatform::send_keys が pending_tsf を見て TIMER_TSF_PROBE をセットする
            return;
        }

        // warm パス: 即座に送信
        let cold_n = self.composition.cold_start_count();
        let outcome = WarmupOutcome { prepend_f2_warmup: false, used_eager_path, cold_n };

        {
            let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            let conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) };
            log::debug!(
                "[h1-send] cold={cold_n} romaji={romaji:?} chars={} gji_idle={gji_idle}ms \
                 conv={} ROMAN={} NATIVE={}",
                chars.len(),
                conv.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv.map_or(false, |v| v & 0x0010 != 0),
                conv.map_or(false, |v| v & 0x0001 != 0),
            );
        }

        let detector = crate::tsf::probe::LiteralDetector::new();
        let ze_bs_count = TsfSendPipeline::new(self).transmit(romaji, &chars, &outcome);
        self.mark_composition_warm();

        // warm パスは prepend_f2_warmup=false なので literal detect は不要
        let _ = (detector, ze_bs_count);
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
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する。
                // 例: ば(probe中) + ー(VK0xBD) の場合、先に ba VKs を送ってから ー を送る。
                // probe なしで直接送ると「F2 → ー → ba」→「ーば」の順序逆転が起きる。
                if self.pending_tsf.borrow().is_some() {
                    log::debug!("    send_char_as_tsf: VK 0x{vk:02X} deferred (probe in flight)");
                    self.deferred_vks_during_probe.borrow_mut().push((vk, needs_shift));
                    return;
                }
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

    /// TIMER_TSF_PROBE ハンドラから呼ぶ。pending_tsf の現フェーズを1ステップ進める。
    ///
    /// 戻り値: `true` = 完了（タイマーを kill すべき）、`false` = まだ継続中。
    pub(crate) fn advance_tsf_probe(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        use crate::tsf::probe::DetectionResult;
        use crate::tsf::observer::{OBS_GJI_LAST_IO_MS, OBS_GJI_MONITOR_OK};

        let Some(data) = self.pending_tsf.borrow_mut().take() else {
            return true;
        };
        let TsfProbeData { romaji, cold_n, phase, _guard: guard } = data;

        match phase {
            TsfProbePhase::GjiProbe {
                probe, total_max_ms, needs_settle_check, cold_reason,
                prepend_f2_warmup, used_eager_path,
            } => {
                if !probe.check_now(total_max_ms) {
                    *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                        romaji, cold_n,
                        phase: TsfProbePhase::GjiProbe {
                            probe, total_max_ms, needs_settle_check, cold_reason,
                            prepend_f2_warmup, used_eager_path,
                        },
                        _guard: guard,
                    });
                    return false;
                }

                let elapsed = crate::hook::current_tick_ms().saturating_sub(probe.warmup_sent_ms);
                log::debug!("[tsf-probe] cold={cold_n} GjiProbe 完了 ({elapsed}ms)");

                if needs_settle_check {
                    let gji_last = OBS_GJI_LAST_IO_MS.load(Relaxed);
                    let probe_settled = gji_last >= probe.warmup_sent_ms;
                    let gji_monitor_ok = OBS_GJI_MONITOR_OK.load(Relaxed);
                    let is_ime_init_cold = cold_reason.requires_settle();
                    if (!probe_settled || is_ime_init_cold) && gji_monitor_ok {
                        const SETTLE_TIMEOUT_MS: u64 = 300;
                        let nc_baseline = crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed);
                        let settle_reason = if !probe_settled {
                            "probe timeout"
                        } else {
                            "NativeF2Consumed/SetOpenTrue"
                        };
                        log::debug!(
                            "[tsf-probe] cold={cold_n} {settle_reason} \
                             → fresh F2 + NameChangeWait (nc_seq={nc_baseline})"
                        );
                        const VK_DBE_HIRAGANA: u16 = 0xF2;
                        let refresh = [
                            make_tsf_key_input(VK_DBE_HIRAGANA, false),
                            make_tsf_key_input(VK_DBE_HIRAGANA, true),
                        ];
                        let fresh_f2_ms = crate::hook::current_tick_ms();
                        // SAFETY: refresh is a valid fixed-size array of INPUT.
                        unsafe {
                            SendInput(
                                &refresh,
                                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
                            );
                        }
                        let deadline_ms = fresh_f2_ms + SETTLE_TIMEOUT_MS;
                        *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                            romaji, cold_n,
                            phase: TsfProbePhase::NameChangeWait {
                                nc_baseline, deadline_ms, fresh_f2_ms, probe_settled,
                                cold_reason, prepend_f2_warmup, used_eager_path,
                            },
                            _guard: guard,
                        });
                        return false;
                    }
                }

                self.do_transmit_tsf(romaji, cold_n, prepend_f2_warmup, used_eager_path, guard)
            }

            TsfProbePhase::NameChangeWait {
                nc_baseline, deadline_ms, fresh_f2_ms, probe_settled,
                cold_reason,
                prepend_f2_warmup, used_eager_path,
            } => {
                let now = crate::hook::current_tick_ms();
                let nc_fired = crate::OBS_FOCUS_NAMECHANGE_SEQ.load(Relaxed) != nc_baseline;
                let timed_out = now >= deadline_ms;

                if !nc_fired && !timed_out {
                    *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                        romaji, cold_n,
                        phase: TsfProbePhase::NameChangeWait {
                            nc_baseline, deadline_ms, fresh_f2_ms, probe_settled,
                            cold_reason,
                            prepend_f2_warmup, used_eager_path,
                        },
                        _guard: guard,
                    });
                    return false;
                }

                let elapsed = now.saturating_sub(fresh_f2_ms);
                log::debug!(
                    "[tsf-probe] cold={cold_n} NameChangeWait → \
                     nc_fired={nc_fired} timed_out={timed_out} ({elapsed}ms)"
                );

                if nc_fired && !probe_settled {
                    const GJI_POST_NAMECHANGE_MS: u64 = 300;
                    log::debug!(
                        "[tsf-probe] cold={cold_n} \
                         OBJ_NAMECHANGE後 GJI 二次プローブ (max {GJI_POST_NAMECHANGE_MS}ms)"
                    );
                    let probe =
                        crate::tsf::probe::TsfReadinessProbe::new(fresh_f2_ms, cold_n, 0);
                    *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                        romaji, cold_n,
                        phase: TsfProbePhase::SecondaryGjiProbe {
                            probe,
                            total_max_ms: GJI_POST_NAMECHANGE_MS,
                            prepend_f2_warmup,
                            used_eager_path,
                        },
                        _guard: guard,
                    });
                    return false;
                }

                self.do_transmit_tsf(romaji, cold_n, prepend_f2_warmup, used_eager_path, guard)
            }

            TsfProbePhase::SecondaryGjiProbe {
                probe, total_max_ms, prepend_f2_warmup, used_eager_path,
            } => {
                if !probe.check_now(total_max_ms) {
                    *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                        romaji, cold_n,
                        phase: TsfProbePhase::SecondaryGjiProbe {
                            probe, total_max_ms, prepend_f2_warmup, used_eager_path,
                        },
                        _guard: guard,
                    });
                    return false;
                }
                let elapsed = crate::hook::current_tick_ms().saturating_sub(probe.warmup_sent_ms);
                log::debug!("[tsf-probe] cold={cold_n} SecondaryGjiProbe 完了 ({elapsed}ms)");
                self.do_transmit_tsf(romaji, cold_n, prepend_f2_warmup, used_eager_path, guard)
            }

            TsfProbePhase::LiteralDetect { detector, ze_bs_count, deadline_ms } => {
                let Some(detection) = detector.check_now(deadline_ms) else {
                    *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                        romaji, cold_n,
                        phase: TsfProbePhase::LiteralDetect {
                            detector, ze_bs_count, deadline_ms,
                        },
                        _guard: guard,
                    });
                    return false;
                };
                match detection {
                    DetectionResult::CompositionConfirmed => {
                        log::debug!("[raw-tsf-literal] cold={cold_n} composition confirmed");
                    }
                    DetectionResult::SuspectedLiteral => {
                        let consecutive = self.composition.consecutive_count();
                        if consecutive == 0 {
                            log::warn!(
                                "[raw-tsf-literal] cold={cold_n} raw TSF literal suspected \
                                → backspace ×{ze_bs_count} + re-send {romaji:?} scheduled \
                                + mark cold"
                            );
                            crate::RAW_TSF_LITERAL.backs.store(ze_bs_count, Relaxed);
                            *crate::RAW_TSF_LITERAL.romaji
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = romaji;
                        } else {
                            log::warn!(
                                "[raw-tsf-literal] cold={cold_n} consecutive raw-tsf-literal \
                                (count={}) → likely false positive, giving up",
                                consecutive + 1,
                            );
                        }
                        self.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
                    }
                }
                true
            }

            TsfProbePhase::ChromeProbe { probe, total_max_ms } => {
                if !probe.check_now(total_max_ms) {
                    *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                        romaji, cold_n,
                        phase: TsfProbePhase::ChromeProbe { probe, total_max_ms },
                        _guard: guard,
                    });
                    return false;
                }
                log::debug!("[tsf-probe] cold={cold_n} ChromeProbe 完了 → batched 送信");
                let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
                self.send_romaji_batch_immediate(&romaji, &chars);
                self.send_deferred_probe_vks();
                self.mark_composition_warm();
                true
            }
        }
    }

    /// probe 完了後に deferred_vks_during_probe を romaji の直後に送出する。
    fn send_deferred_probe_vks(&self) {
        let vks = std::mem::take(&mut *self.deferred_vks_during_probe.borrow_mut());
        if vks.is_empty() {
            return;
        }
        log::debug!("[tsf-probe] deferred {} VK(s) を romaji 直後に送出", vks.len());
        let mut inputs: Vec<INPUT> = Vec::with_capacity(vks.len() * 4);
        for &(vk, needs_shift) in &vks {
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
            }
            inputs.push(make_tsf_key_input(vk, false));
            inputs.push(make_tsf_key_input(vk, true));
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
            }
        }
        // SAFETY: inputs is a valid Vec<INPUT> whose contents live for the duration of this call.
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }

    /// GjiProbe / NameChangeWait / SecondaryGjiProbe 完了後の TSF 送信を実行する。
    ///
    /// LiteralDetect フェーズが必要なら pending_tsf にセットして `false` を返す。
    /// 不要なら `true` を返す（guard がここで drop → OUTPUT_ACTIVE=false + drain）。
    fn do_transmit_tsf(
        &self,
        romaji: String,
        cold_n: u32,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
        guard: OutputActiveGuard,
    ) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        use crate::tsf::observer::OBS_GJI_MONITOR_OK;

        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return true;
        }

        let outcome = WarmupOutcome { prepend_f2_warmup, used_eager_path, cold_n };

        {
            let last_io = crate::tsf::observer::OBS_GJI_LAST_IO_MS.load(Relaxed);
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            // SAFETY: IMM32 API; called from message-loop thread.
            let conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) };
            log::debug!(
                "[h1-send] cold={cold_n} romaji={romaji:?} chars={} gji_idle={gji_idle}ms \
                 conv={} ROMAN={} NATIVE={}",
                chars.len(),
                conv.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv.map_or(false, |v| v & 0x0010 != 0),
                conv.map_or(false, |v| v & 0x0001 != 0),
            );
        }

        let detector = crate::tsf::probe::LiteralDetector::new();
        let ze_bs_count = TsfSendPipeline::new(self).transmit(&romaji, &chars, &outcome);
        self.send_deferred_probe_vks();
        self.mark_composition_warm();

        let gji_active = OBS_GJI_MONITOR_OK.load(Relaxed);
        if prepend_f2_warmup && gji_active {
            let deadline_ms = crate::hook::current_tick_ms()
                + crate::timing::RAW_TSF_LITERAL_DETECT_MS;
            *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                romaji,
                cold_n,
                phase: TsfProbePhase::LiteralDetect { detector, ze_bs_count, deadline_ms },
                _guard: guard,
            });
            return false;
        }
        // guard drops here → OUTPUT_ACTIVE=false + drain
        true
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

/// TSF 送信パイプライン（transmit フェーズのみ）。
///
/// - `transmit`: VK または Unicode kana で romaji を WezTerm に送信
///
/// warm パス（`send_romaji_as_tsf` の non-cold ブランチ）と
/// `do_transmit_tsf`（タイマー FSM からの遅延送信）が使用する。
struct TsfSendPipeline<'a> {
    output: &'a Output,
}

impl<'a> TsfSendPipeline<'a> {
    fn new(output: &'a Output) -> Self {
        Self { output }
    }

    /// VK run または Unicode kana を送信し、バックスペース数を返す。
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


#[cfg(test)]
mod tests {
    use super::*;

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
