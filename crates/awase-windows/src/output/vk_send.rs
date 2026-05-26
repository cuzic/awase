use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};
use awase::types::VkCode;
use crate::tsf::output::{INJECTED_MARKER, make_key_input_ex, make_tsf_key_input, kana_for_romaji_static};
use crate::tsf::output::ColdReason;
use crate::tsf::output::TSF_MARKER;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::TsfProbeMachine;
use crate::vk::{VK_DBE_HIRAGANA, VK_LSHIFT, VK_OEM_MINUS};
use super::{Output, VkSequence};
use super::{WarmthContext, WarmupOutcome, fmt_ms};
use super::resolve::{ascii_to_vk, CharResolution};

/// INPUT 構造体を作成するヘルパー（INJECTED_MARKER 固定）
#[must_use]
pub(super) const fn make_key_input(vk: VkCode, is_keyup: bool) -> INPUT {
    make_key_input_ex(vk, is_keyup, INJECTED_MARKER)
}

/// VK の DOWN+UP ペアを（オプション shift 付きで）1回の SendInput で送信する。
fn send_vk_pair(vk: VkCode, needs_shift: bool, use_tsf_marker: bool) {
    let mut inputs = Vec::with_capacity(4);
    if needs_shift {
        inputs.push(make_key_input(VK_LSHIFT, false));
    }
    if use_tsf_marker {
        inputs.push(make_tsf_key_input(vk, false));
        inputs.push(make_tsf_key_input(vk, true));
    } else {
        inputs.push(make_key_input(vk, false));
        inputs.push(make_key_input(vk, true));
    }
    if needs_shift {
        inputs.push(make_key_input(VK_LSHIFT, true));
    }
    let _ = crate::win32::send_input_safe(&inputs);
}

/// `ch` を UTF-16 エンコードし、down/up ペアを `inputs` に追加する。
///
/// `marker` は `INJECTED_MARKER`（Unicode モード）または `TSF_MARKER`（TSF モード）。
fn push_unicode_char_inputs(inputs: &mut Vec<INPUT>, ch: char, marker: usize) {
    let mut buf = [0u16; 2];
    let utf16 = ch.encode_utf16(&mut buf);
    for &cu in utf16.iter() {
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0), wScan: cu,
                dwFlags: KEYEVENTF_UNICODE, time: 0, dwExtraInfo: marker,
            }},
        });
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0), wScan: cu,
                dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, time: 0, dwExtraInfo: marker,
            }},
        });
    }
}

/// 同一 VK が連続する境界でランを分割する。
///
/// `send_vk_runs` と `send_deferred_probe_vks_from` 共通のヘルパー。
/// IME のオートリピート誤検出を防ぐため、同一 VK が連続する箇所で区切る。
fn split_vk_runs(vks: &[(VkCode, bool)]) -> Vec<&[(VkCode, bool)]> {
    if vks.is_empty() { return vec![]; }
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

/// TSF 送信パイプライン（transmit フェーズのみ）。
///
/// - `transmit`: VK または Unicode kana で romaji を WezTerm に送信
///
/// warm パス（`send_romaji_as_tsf` の non-cold ブランチ）と
/// `do_transmit_tsf`（タイマー FSM からの遅延送信）が使用する。
pub(crate) struct TsfSendPipeline;

impl TsfSendPipeline {
    /// VK run または Unicode kana を送信し、バックスペース数を返す。
    pub(crate) fn transmit(romaji: &str, chars: &[(VkCode, bool)], outcome: &WarmupOutcome) -> usize {
        let unicode_kana: Option<char> = if outcome.prepend_f2_warmup && outcome.used_eager_path {
            kana_for_romaji_static(romaji)
        } else {
            None
        };

        unicode_kana.map_or_else(|| {
            Output::send_vk_runs(chars, outcome.cold_seq);
            chars.len()
        }, |kana| {
            log::debug!(
                "[h1-run] cold={} unicode TSF: {romaji:?} → '{}' (U+{:04X})",
                outcome.cold_seq, kana, kana as u32,
            );
            let mut inputs = Vec::with_capacity(4);
            push_unicode_char_inputs(&mut inputs, kana, TSF_MARKER);
            let _ = crate::win32::send_input_safe(&inputs);
            1
        })
    }
}

impl Output {
    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    #[allow(clippy::unused_self)] // Output の impl に所属させ API の一貫性を保つ
    pub(super) fn send_key(&self, vk: VkCode, is_keyup: bool) {
        let input = make_key_input(vk, is_keyup);
        let _ = crate::win32::send_input_safe(&[input]);
    }

    /// Unicode 文字を直接送信する（`KEYEVENTF_UNICODE`）
    #[allow(clippy::unused_self)] // Output の impl に所属させ API の一貫性を保つ
    pub(super) fn send_unicode_char(&self, ch: char) {
        let mut inputs = Vec::with_capacity(4);
        push_unicode_char_inputs(&mut inputs, ch, INJECTED_MARKER);
        let _ = crate::win32::send_input_safe(&inputs);
    }

    /// PerKey モード: 1文字ずつ個別の SendInput 呼び出し
    ///
    /// 各文字の KeyDown+KeyUp は1回の SendInput にまとめるが、
    /// 文字間は別の SendInput 呼び出しに分離する。
    /// 他のキーボードフックに処理時間を与える。
    #[allow(clippy::unused_self)] // Output の impl に所属させ API の一貫性を保つ
    pub(super) fn send_romaji_per_key(&self, romaji: &str) {
        for ch in romaji.chars() {
            if let Some((vk, needs_shift)) = ascii_to_vk(ch) {
                send_vk_pair(vk, needs_shift, false);
            }
        }
    }

    /// Batched モード: 全文字を1回の SendInput にまとめて送信（重畳押し順）
    ///
    /// cold 時は F2 を先行送信してから GJI プローブを開始し（ノンブロッキング）、
    /// TIMER_TSF_PROBE が `ChromeProbe` フェーズを進めてローマ字を送信する。
    pub(super) fn send_romaji_batched(&self, romaji: &str) {
        let chars: VkSequence = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        let WarmthContext { warm, elapsed, session_expired, prepend_f2_warmup } =
            self.assess_warmth();
        log::debug!(
            "[vk-send] romaji={romaji:?} warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        if prepend_f2_warmup {
            if self.defer_if_probe_in_flight(romaji) { return; }

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
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                conv_pre.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_KATAKANA)),
            );
            // SAFETY: IMM32 API; sets conversion mode on the foreground window's IME context.
            unsafe { let _ = crate::ime::set_ime_romaji_mode(); }

            let cold_seq = self.composition.increment_cold_start_count();
            let win_class = unsafe { crate::ime::get_foreground_window_class() };
            log::debug!("[h1-window] cold={cold_seq} class={win_class}");

            log::debug!("[h1-run] cold={cold_seq} F2 via SendMessageTimeout");
            let f2_sent_ms = crate::hook::current_tick_ms();
            // SAFETY: sends WM_KEYDOWN/WM_KEYUP to the foreground window via SendMessageTimeout.
            let f2_ok = unsafe { crate::ime::send_f2_via_sendmessage() };
            log::debug!("[h1-run] cold={cold_seq} F2 SendMessageTimeout delivered={f2_ok}");

            // SendMessageTimeout はウィンドウの wndproc に直接届くが TSF のキーストローク
            // マネージャーを経由しないため、Chrome の composition context が初期化されない。
            // SendInput 経由でも F2 を送り TSF に composition context を初期化させる。
            // INJECTED_MARKER 付きなので awase 自身のフックは即座に素通しする（mark_cold 不要）。
            let f2_via_sendinput = [make_key_input(VK_DBE_HIRAGANA, false), make_key_input(VK_DBE_HIRAGANA, true)];
            let _ = crate::win32::send_input_safe(&f2_via_sendinput);
            log::debug!("[h1-run] cold={cold_seq} F2 via SendInput (TSF composition context init)");

            // ノンブロッキング Chrome プローブを開始。
            // 長期 idle 後の cold start では GJI が reinit に要する時間が長いため
            // min/max を延長する（120ms では GJI が settle する前に timeout して literal
            // 出力される回帰を抑制）。
            let long_idle = self.composition.idle_ms_at_last_cold()
                > crate::tuning::LONG_IDLE_MS;
            let (probe_min_ms, probe_max_ms) = if long_idle {
                (
                    crate::tuning::CHROME_PROBE_LONG_IDLE_MIN_MS,
                    crate::tuning::CHROME_PROBE_LONG_IDLE_MAX_MS,
                )
            } else {
                (
                    crate::tuning::CHROME_PROBE_MIN_MS,
                    crate::tuning::CHROME_PROBE_MAX_MS,
                )
            };
            log::debug!(
                "[h1-probe] cold={cold_seq} long_idle={long_idle} idle_at_cold={}ms min={probe_min_ms}ms max={probe_max_ms}ms",
                self.composition.idle_ms_at_last_cold(),
            );
            let probe = crate::tsf::probe::TsfReadinessProbe::new(
                f2_sent_ms,
                cold_seq,
                probe_min_ms,
            );
            let guard = OutputActiveGuard::begin();
            self.install_pending_tsf(TsfProbeMachine::new_chrome(
                romaji,
                cold_seq,
                probe,
                probe_max_ms,
                guard,
            ));
            // WindowsPlatform::send_keys が pending_tsf を見て TIMER_TSF_PROBE をセットする
            return;
        }

        // warm パス: 即座にバッチ送信
        Self::send_romaji_batch_immediate(romaji, &chars);
        self.mark_composition_warm();
    }

    /// ローマ字を即座にバッチ送信する（重畳順）。
    /// warm パスおよび `advance_tsf_probe` の ChromeProbe 完了時に呼ぶ。
    pub(crate) fn send_romaji_batch_immediate(romaji: &str, chars: &[(VkCode, bool)]) {
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
        let _ = crate::win32::send_input_safe(&inputs);
    }

    /// Unicode モード: ローマ字→ひらがなに変換して Unicode 文字として直接送信
    ///
    /// IME を経由せず、ひらがなを直接テキストフィールドに挿入する。
    /// 変換テーブルにないローマ字は PerKey モードでフォールバック送信する。
    pub(super) fn send_romaji_as_unicode(&self, romaji: &str) {
        if let Some(&kana) = self.romaji_to_kana.as_ref().and_then(|t| t.get(romaji)) {
            self.send_unicode_char(kana);
            return;
        }
        // テーブルにない場合はフォールバック
        self.send_romaji_per_key(romaji);
    }

    /// VK run 分割送信: 同一 VK 連続境界でバッチを分割して IME のオートリピート誤検出を回避する。
    pub(super) fn send_vk_runs(chars: &[(VkCode, bool)], cold_seq: u32) {
        // 同一 VK が連続する箇所（例 "nn"）でバッチに N↓N↓N↑N↑ を含めると、IME が
        // 2 つ目の N↓ をオートリピートと判定して破棄してしまう。
        // 同一 VK が連続する境界で run を分割し、各 run を別の SendInput で送る。
        let runs = split_vk_runs(chars);
        let total_runs = runs.len();

        for (run_idx, run) in runs.into_iter().enumerate() {
            let last_io = crate::tsf::observer::gji_last_io_ms();
            let run_gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            log::debug!(
                "[h1-run] cold={cold_seq} run={run_idx}/{total_runs} gji={run_gji_idle}ms vks=[{}]",
                run.iter()
                    .map(|&(v, s)| if s { format!("S{v:02X}") } else { format!("{v:02X}") })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            let mut inputs = Vec::with_capacity(run.len() * 4);
            for &(vk, needs_shift) in run {
                if needs_shift {
                    inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
                }
                inputs.push(make_tsf_key_input(vk, false));
            }
            for &(vk, needs_shift) in run {
                inputs.push(make_tsf_key_input(vk, true));
                if needs_shift {
                    inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
                }
            }
            let _ = crate::win32::send_input_safe(&inputs);
        }
    }

    pub(super) fn send_romaji_as_tsf(&self, romaji: &str) {
        let chars: VkSequence = romaji.chars().filter_map(ascii_to_vk).collect();
        if chars.is_empty() {
            return;
        }

        let WarmthContext { warm, elapsed, session_expired, prepend_f2_warmup } =
            self.assess_warmth();
        let used_eager_path = self.composition.eager_warmup_sent_ms() != 0;

        log::debug!(
            "[tsf-send] warm={warm} elapsed={}ms session_expired={session_expired} prepend_f2_warmup={prepend_f2_warmup}",
            fmt_ms(elapsed)
        );

        if prepend_f2_warmup {
            if self.defer_if_probe_in_flight(romaji) { return; }

            // ノンブロッキング warmup を開始して pending_tsf に保留
            let started = crate::tsf::cold_warmup::ColdWarmupSequence::new(self)
                .run_start(session_expired, elapsed);
            let cold_seq = started.probe.cold_seq;
            let guard = OutputActiveGuard::begin();
            self.install_pending_tsf(TsfProbeMachine::new_gji(
                romaji,
                cold_seq,
                started.probe,
                started.total_max_ms,
                started.needs_settle_check,
                started.cold_reason,
                prepend_f2_warmup,
                used_eager_path,
                guard,
            ));
            // WindowsPlatform::send_keys が pending_tsf を見て TIMER_TSF_PROBE をセットする
            return;
        }

        // warm パス: 即座に送信
        self.send_romaji_as_tsf_warm(romaji, &chars, used_eager_path);
    }

    fn send_romaji_as_tsf_warm(&self, romaji: &str, chars: &VkSequence, used_eager_path: bool) {
        let cold_seq = self.composition.cold_start_count();
        let outcome = WarmupOutcome { prepend_f2_warmup: false, used_eager_path, cold_seq };

        {
            let last_io = crate::tsf::observer::gji_last_io_ms();
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            let conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) };
            log::debug!(
                "[h1-send] cold={cold_seq} romaji={romaji:?} chars={} gji_idle={gji_idle}ms \
                 conv={} ROMAN={} NATIVE={}",
                chars.len(),
                conv.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_ROMAN)),
                conv.is_some_and(|v| crate::imm::cmode_has(v, crate::imm::IME_CMODE_NATIVE)),
            );
        }

        let detector = crate::tsf::probe::LiteralDetector::new();
        let ze_bs_count = TsfSendPipeline::transmit(romaji, chars, &outcome);
        self.mark_composition_warm();

        let gji_active = crate::tsf::observer::gji_monitor_healthy();
        if self.tsf_gate.state() == crate::tsf::TsfGateState::Probing && gji_active {
            let deadline_ms = crate::hook::current_tick_ms()
                + crate::tuning::RAW_TSF_LITERAL_DETECT_MS;
            let guard = OutputActiveGuard::begin();
            self.install_pending_tsf(TsfProbeMachine::new_literal_detect(
                romaji,
                cold_seq,
                detector,
                ze_bs_count,
                deadline_ms,
                guard,
            ));
        } else {
            // detector と ze_bs_count は Probing+GJI 健全パスでのみ使う。
            // 他パスでは warm マーク済みで LiteralDetect 不要。
            let _ = (detector, ze_bs_count);
        }
    }

    /// 文字を TSF Sequential VK キーストロークとして送信する（WezTerm TSF モード用）
    ///
    /// かな文字はローマ字に逆変換してから `send_romaji_as_tsf` で送信する。
    /// 記号は symbol_to_vk テーブルで直接 VK コードに変換する。
    /// マッチしない場合は Unicode 直接出力にフォールバックする。
    pub(super) fn send_char_as_tsf(&self, ch: char) {
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
                if self.defer_vk_if_probe_in_flight(vk, needs_shift) {
                    log::debug!("    send_char_as_tsf: VK 0x{vk:02X} deferred (probe in flight)");
                    return;
                }
                send_vk_pair(vk, needs_shift, true);
                // VK_OEM_MINUS (0xBD, no-shift) = '-' は GJI ローマ字モードで「ー」として
                // composition に取り込まれる（composition context はリセットされない）。
                // これらは warm 状態を維持し、次の romaji を warmup sleep なしで即送信する。
                // その他の記号（句読点など）は composition を commit する可能性があるため cold にマーク。
                let keeps_composition = vk == VK_OEM_MINUS && !needs_shift;
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
    pub(super) fn send_char_as_vk(&self, ch: char) {
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
                // probe 進行中は VK を後回しにして romaji との送信順序を保証する。
                // 例: ば(ChromeProbe中) + ー(VK0xBD) の場合、先に ba VKs を送ってから ー を送る。
                if self.defer_vk_if_probe_in_flight(vk, needs_shift) {
                    log::debug!("    send_char_as_vk: VK 0x{vk:02X} deferred (probe in flight)");
                    return;
                }
                send_vk_pair(vk, needs_shift, false);
            }
            CharResolution::Unicode(ch) => {
                log::debug!("    send_char_as_vk: '{ch}' (U+{:04X}) → fallback Unicode", ch as u32);
                self.send_unicode_char(ch);
            }
        }
    }

    /// probe 完了後に deferred_vks を romaji の直後に送出する。
    /// vks が空なら no-op。
    /// `use_tsf_marker` = true → TSF_MARKER（WezTerm TSF モード）
    ///                    false → INJECTED_MARKER（Chrome/VK モード）
    ///
    /// # 送信順序（コード順）
    ///
    /// `send_vk_runs` と同様に「全↓→全↑」のコード順で送信する。
    /// シーケンシャル順（R↓R↑A↓A↑）では KEYEVENTF_UNICODE 送信直後に
    /// 不完全ローマ字がリテラルコミットされる問題があるため、
    /// コード順（R↓A↓R↑A↑）を使うことで IME が romaji ペアを正しく結合する。
    ///
    /// 同一 VK が連続する箇所（例 "nn"）ではオートリピート誤検出を避けるため
    /// `send_vk_runs` と同様にランごとに分割して別 SendInput を使う。
    pub(crate) fn send_deferred_probe_vks_from(vks: &[(VkCode, bool)], use_tsf_marker: bool) {
        if vks.is_empty() {
            return;
        }
        log::debug!("[tsf-probe] deferred {} VK(s) を romaji 直後に送出 (tsf_marker={use_tsf_marker})", vks.len());

        for run in split_vk_runs(vks) {
            let mut inputs: Vec<INPUT> = Vec::with_capacity(run.len() * 4);
            // 全↓（コード順前半）
            for &(vk, needs_shift) in run {
                if needs_shift {
                    inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
                }
                if use_tsf_marker {
                    inputs.push(make_tsf_key_input(vk, false));
                } else {
                    inputs.push(make_key_input(vk, false));
                }
            }
            // 全↑（コード順後半）
            for &(vk, needs_shift) in run {
                if use_tsf_marker {
                    inputs.push(make_tsf_key_input(vk, true));
                } else {
                    inputs.push(make_key_input(vk, true));
                }
                if needs_shift {
                    inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
                }
            }
            let _ = crate::win32::send_input_safe(&inputs);
        }
    }
}
