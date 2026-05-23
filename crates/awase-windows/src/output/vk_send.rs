use std::mem::size_of;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};
use crate::tsf::output::{INJECTED_MARKER, make_key_input_ex, make_tsf_key_input, kana_for_romaji_static};
use crate::tsf::output::ColdReason;
use crate::tsf::output::TSF_MARKER;
use crate::tsf::probe_bridge::OutputActiveGuard;
use super::Output;
use super::{WarmthContext, WarmupOutcome, TsfProbeData, TsfProbePhase, fmt_ms};
use super::resolve::{ascii_to_vk, CharResolution};

/// VK_LSHIFT の仮想キーコード
const VK_LSHIFT: u16 = 0xA0;

/// INPUT 構造体を作成するヘルパー（INJECTED_MARKER 固定）
#[must_use]
pub(super) const fn make_key_input(vk: u16, is_keyup: bool) -> INPUT {
    make_key_input_ex(vk, is_keyup, INJECTED_MARKER)
}

/// TSF 送信パイプライン（transmit フェーズのみ）。
///
/// - `transmit`: VK または Unicode kana で romaji を WezTerm に送信
///
/// warm パス（`send_romaji_as_tsf` の non-cold ブランチ）と
/// `do_transmit_tsf`（タイマー FSM からの遅延送信）が使用する。
pub(super) struct TsfSendPipeline;

impl TsfSendPipeline {
    /// VK run または Unicode kana を送信し、バックスペース数を返す。
    pub(super) fn transmit(romaji: &str, chars: &[(u16, bool)], outcome: &WarmupOutcome) -> usize {
        let unicode_kana: Option<char> = if outcome.prepend_f2_warmup && outcome.used_eager_path {
            kana_for_romaji_static(romaji)
        } else {
            None
        };

        unicode_kana.map_or_else(|| {
            Output::send_vk_runs(chars, outcome.cold_seq);
            chars.len()
        }, |kana| {
            let mut utf16_buf = [0u16; 2];
            let utf16 = kana.encode_utf16(&mut utf16_buf);
            log::debug!(
                "[h1-run] cold={} unicode TSF: {romaji:?} → '{}' (U+{:04X})",
                outcome.cold_seq, kana, kana as u32,
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
        })
    }
}

impl Output {
    /// 仮想キーコードを使って即座に KeyDown/KeyUp を送信する
    #[allow(clippy::unused_self)]
    pub(super) fn send_key(&self, vk: u16, is_keyup: bool) {
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
    pub(super) fn send_unicode_char(&self, ch: char) {
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
    pub(super) fn send_romaji_per_key(&self, romaji: &str) {
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
    pub(super) fn send_romaji_batched(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
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
                conv_pre.is_some_and(|v| v & 0x0001 != 0),
                conv_pre.is_some_and(|v| v & 0x0010 != 0),
                conv_pre.is_some_and(|v| v & 0x0002 != 0),
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

            // ノンブロッキング Chrome プローブを開始
            let probe = crate::tsf::probe::TsfReadinessProbe::new(
                f2_sent_ms,
                cold_seq,
                crate::tuning::CHROME_PROBE_MIN_MS,
            );
            let guard = OutputActiveGuard::begin();
            *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                romaji: romaji.to_string(),
                cold_seq,
                deferred_vks: Vec::new(),
                phase: TsfProbePhase::ChromeProbe { probe, total_max_ms: crate::tuning::CHROME_PROBE_MAX_MS },
                _guard: guard,
            });
            // WindowsPlatform::send_keys が pending_tsf を見て TIMER_TSF_PROBE をセットする
            return;
        }

        // warm パス: 即座にバッチ送信
        Self::send_romaji_batch_immediate(romaji, &chars);
        self.mark_composition_warm();
    }

    /// ローマ字を即座にバッチ送信する（重畳順）。
    /// warm パスおよび `advance_tsf_probe` の ChromeProbe 完了時に呼ぶ。
    pub(super) fn send_romaji_batch_immediate(romaji: &str, chars: &[(u16, bool)]) {
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
    pub(super) fn send_romaji_as_unicode(&self, romaji: &str) {
        if let Some(&kana) = self.romaji_to_kana.as_ref().and_then(|t| t.get(romaji)) {
            self.send_unicode_char(kana);
            return;
        }
        // テーブルにない場合はフォールバック
        self.send_romaji_per_key(romaji);
    }

    /// VK run 分割送信: 同一 VK 連続境界でバッチを分割して IME のオートリピート誤検出を回避する。
    pub(super) fn send_vk_runs(chars: &[(u16, bool)], cold_seq: u32) {
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
            let last_io = crate::tsf::observer::with_tsf_obs(super::super::tsf::observer::TsfObservations::gji_last_io_ms);
            let run_gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            let vks: Vec<String> = run.iter().map(|&(v, s)| {
                if s { format!("S{v:02X}") } else { format!("{v:02X}") }
            }).collect();
            log::debug!(
                "[h1-run] cold={cold_seq} run={run_idx}/{total_runs} gji={run_gji_idle}ms vks=[{}]",
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

    pub(super) fn send_romaji_as_tsf(&self, romaji: &str) {
        let chars: Vec<(u16, bool)> = romaji.chars().filter_map(ascii_to_vk).collect();
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
            *self.pending_tsf.borrow_mut() = Some(TsfProbeData {
                romaji: romaji.to_string(),
                cold_seq,
                deferred_vks: Vec::new(),
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
        let cold_seq = self.composition.cold_start_count();
        let outcome = WarmupOutcome { prepend_f2_warmup: false, used_eager_path, cold_seq };

        {
            let last_io = crate::tsf::observer::with_tsf_obs(super::super::tsf::observer::TsfObservations::gji_last_io_ms);
            let gji_idle = crate::hook::current_tick_ms().saturating_sub(last_io);
            let conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) };
            log::debug!(
                "[h1-send] cold={cold_seq} romaji={romaji:?} chars={} gji_idle={gji_idle}ms \
                 conv={} ROMAN={} NATIVE={}",
                chars.len(),
                conv.map_or_else(|| "none".to_string(), |v| format!("0x{v:08X}")),
                conv.is_some_and(|v| v & 0x0010 != 0),
                conv.is_some_and(|v| v & 0x0001 != 0),
            );
        }

        let detector = crate::tsf::probe::LiteralDetector::new();
        let ze_bs_count = TsfSendPipeline::transmit(romaji, &chars, &outcome);
        self.mark_composition_warm();

        // Probing 状態の warm 投機送信: GJI 監視が有効なら LiteralDetect で検証する。
        // (1) raw TSF literal 検出と回復, (2) advance_tsf_probe が on_ready() を呼んで
        //     ゲートを Ready に進める、という 2 つの目的を兼ねる。
        let gji_active = crate::tsf::observer::with_tsf_obs(super::super::tsf::observer::TsfObservations::gji_monitor_ok);
        if self.tsf_gate.state() == crate::tsf::TsfGateState::Probing && gji_active {
            let deadline_ms = crate::hook::current_tick_ms()
                + crate::tuning::RAW_TSF_LITERAL_DETECT_MS;
            let guard = OutputActiveGuard::begin();
            self.put_back_probe(
                romaji.to_string(),
                cold_seq,
                Vec::new(),
                TsfProbePhase::LiteralDetect { detector, ze_bs_count, deadline_ms },
                guard,
            );
        } else {
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
                if let Some(data) = self.pending_tsf.borrow_mut().as_mut() {
                    log::debug!("    send_char_as_tsf: VK 0x{vk:02X} deferred (probe in flight)");
                    data.deferred_vks.push((vk, needs_shift));
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
                if let Some(data) = self.pending_tsf.borrow_mut().as_mut() {
                    log::debug!("    send_char_as_vk: VK 0x{vk:02X} deferred (probe in flight)");
                    data.deferred_vks.push((vk, needs_shift));
                    return;
                }
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

    /// probe 完了後に deferred_vks を romaji の直後に送出する。
    /// vks が空なら no-op。
    /// `use_tsf_marker` = true → TSF_MARKER（WezTerm TSF モード）
    ///                    false → INJECTED_MARKER（Chrome/VK モード）
    pub(super) fn send_deferred_probe_vks_from(vks: &[(u16, bool)], use_tsf_marker: bool) {
        if vks.is_empty() {
            return;
        }
        log::debug!("[tsf-probe] deferred {} VK(s) を romaji 直後に送出 (tsf_marker={use_tsf_marker})", vks.len());
        let mut inputs: Vec<INPUT> = Vec::with_capacity(vks.len() * 4);
        for &(vk, needs_shift) in vks {
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, false, INJECTED_MARKER));
            }
            if use_tsf_marker {
                inputs.push(make_tsf_key_input(vk, false));
                inputs.push(make_tsf_key_input(vk, true));
            } else {
                inputs.push(make_key_input(vk, false));
                inputs.push(make_key_input(vk, true));
            }
            if needs_shift {
                inputs.push(make_key_input_ex(VK_LSHIFT, true, INJECTED_MARKER));
            }
        }
        // SAFETY: inputs is a valid Vec<INPUT> whose contents live for this call.
        unsafe {
            SendInput(
                &inputs,
                i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
            );
        }
    }
}
