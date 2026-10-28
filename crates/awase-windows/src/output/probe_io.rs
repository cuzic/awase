//! ProbeIo トレイト — Win32 副作用を抽象化し `dispatch_probe_actions` をテスト可能にする。
//!
//! `Output` が本番実装。`#[cfg(test)]` ブロック内の `FakeProbeIo` がテスト実装。
//! `dispatch_probe_actions` は `ProbeIo` を受け取り、Win32 呼び出しを直接行わない。

use crate::output::{KeyInjector, Output, VkMarker, VkSequence};
use crate::state::event_origin::Generation;
use crate::tsf::gji_fsm::StageEndReason;
use crate::tsf::literal_facts::{
    DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralDetectFacts, LiteralDetectRecord,
    LiteralDetectTrace, LiteralDetectTraceItem, LiteralVerdict,
};
use crate::tsf::output::ColdReason;
use crate::tsf::warmup::mozc_compose_check_fsm::MozcComposeTarget;
use crate::tsf::warmup::probe_fsm::DeferredVk;
use awase::types::VkCode;
use win32_async;

/// `dispatch_probe_actions` が要求する Win32 / 状態ミューテーション操作の抽象。
///
/// - `Output` が本番実装（Win32 SendInput・グローバル原子値の操作）
/// - `FakeProbeIo` がテスト実装（状態変化をフラグで記録し、返値を制御）
pub(crate) trait ProbeIo {
    /// Chrome バッチ送信を実行する。
    fn transmit_chrome(&self, romaji: &str, chars: &[(VkCode, bool)]);
    /// per-VK confirm 専用: 1 VK の
    /// DOWN+UP を `VkMarker::InjectedWithScan`（scan code 付き）で単独送信する。
    fn send_single_chrome_vk(&self, vk: VkCode, needs_shift: bool);
    /// deferred VKs を送信する。段末（`Output::flush_pending_deferred_vks`）専用。
    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker);
    /// 連続 raw TSF literal 回数を返す。
    fn consecutive_count(&self) -> u32;
    /// 連続カウントをリセットする（`DetectionResult::CompositionConfirmed` 確認時、BUG-27 追補4）。
    fn reset_consecutive_count(&self);
    /// `RAW_TSF_LITERAL` グローバルを設定する（`consecutive == 0` のときのみ呼ばれる）。
    ///
    /// `escape_composition`: partial literal（candidate 表示中に一部だけ literal 化）回収時に
    /// `true`。`flush_raw_tsf_literal_backspaces` がバックスペース前に `VK_ESCAPE` を送る。
    fn set_raw_literal(&self, backs: usize, romaji: String, escape_composition: bool);
    /// composition を `RawTsfLiteralRecovery` で cold にマークする。
    fn mark_cold_raw_tsf(&self);
    /// ADR-270: Mozc で、この送信の後に入力中の文字の確認を張るなら、その対象を返す。**送信の直前**に呼ぶ
    /// （まだ入力中の文字が無いことを確かめるため）。既定は張らない（テスト用の stub と、Mozc 系でない環境）。
    fn mozc_compose_precheck(
        &self,
        _romaji: &str,
        _cold_seq: Generation,
    ) -> Option<MozcComposeTarget> {
        None
    }
    /// ADR-270: 段が終わったら確認を張る予約（`Output::step_probe` の段末で取り出す）。既定は何もしない。
    fn request_mozc_compose_check(&self, _target: MozcComposeTarget) {}
}

impl ProbeIo for Output {
    fn transmit_chrome(&self, romaji: &str, chars: &[(VkCode, bool)]) {
        self.warmup_coord.note_stage_injection();
        Self::send_romaji_batch_immediate(romaji, chars);
    }

    fn send_single_chrome_vk(&self, vk: VkCode, needs_shift: bool) {
        self.warmup_coord.note_stage_injection();
        // scan code 付き（VkMarker::InjectedWithScan）、key_injector.rs の
        // send_romaji_batch_immediate と同じ恒久仕様。
        KeyInjector::send_vk_pair(vk, needs_shift, VkMarker::InjectedWithScan);
    }

    fn send_deferred_vks(&self, vks: &[DeferredVk], marker: VkMarker) {
        let pairs: Vec<(VkCode, bool)> = vks.iter().map(|d| (d.vk, d.needs_shift)).collect();
        Self::send_deferred_probe_vks_from(&pairs, marker);
    }

    fn consecutive_count(&self) -> u32 {
        self.composition.consecutive_count()
    }

    fn reset_consecutive_count(&self) {
        self.composition.reset_consecutive_count();
    }

    fn set_raw_literal(&self, backs: usize, romaji: String, escape_composition: bool) {
        self.record_raw_tsf_literal(backs, romaji, escape_composition);
    }

    fn mark_cold_raw_tsf(&self) {
        self.mark_composition_cold(ColdReason::RawTsfLiteralRecovery);
        self.warmup_coord.note_stage_recovery();
        self.warmup_coord.mark_composition_reset();
    }

    #[allow(unsafe_code)] // 前景窓のクラス名の読み取り(Win32)
    fn mozc_compose_precheck(
        &self,
        romaji: &str,
        cold_seq: Generation,
    ) -> Option<MozcComposeTarget> {
        // GJI の証拠が無いと確定している（Mozc 系、ADR-269 D1、`gji_evidence_absent`）かつ、旧来の範囲
        // (前景窓が Windows Terminal・WezTerm)のときだけ(PR #809 レビュー M3・再レビュー N2、`mozc_compose_check_applies`)。
        let mozc = crate::tsf::observer::gji_evidence_absent();
        if !mozc {
            return None;
        }
        // SAFETY: Win32 GetForegroundWindow + GetClassName; returns empty string on failure.
        let fg_class = unsafe { crate::ime::get_foreground_window_class() };
        if !crate::focus::class_names::mozc_compose_check_applies(mozc, &fg_class) {
            return None;
        }
        let (hwnd_addr, thread_id, process_id) =
            crate::observer::mozc_shm::focus_thread_and_process()?;
        let pre = crate::observer::mozc_shm::read_compose(thread_id, process_id);
        let crate::state::mozc_shm::ComposeReading::Block(before) = pre else {
            return None;
        };
        crate::state::mozc_shm::should_check_compose(pre, romaji).then(|| MozcComposeTarget {
            cold_seq,
            romaji: romaji.to_string(),
            hwnd_addr,
            thread_id,
            process_id,
            key_seq_before: before.key_seq,
            rest: String::new(),
        })
    }

    fn request_mozc_compose_check(&self, target: MozcComposeTarget) {
        *self.mozc_compose_request.borrow_mut() = Some(target);
    }
}

/// [`Output::start_ms_ime_ready_poll`] の `with_app` クロージャ戻り値。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MsImePollStatus {
    /// NATIVE 確認済み → ポーリング終了。
    Ready,
    /// 未確認だが期限内 → 継続。
    Pending,
    /// 未確認のまま期限切れ → give-up latch を立ててポーリング終了。
    Expired,
    /// フォーカス世代不一致 / with_app 失敗 → 黙って終了。
    Stale,
}

impl Output {
    /// `send_chrome_gji_reinit_and_poll` / [`Output::start_ms_ime_ready_poll`] の
    /// `with_app` クロージャ内で1 tickごとに独立してコピーされていた「focus_gen
    /// 照合 → 不一致なら stale → `update_ime_mode_from_imc(conv)` で IMC を反映」を
    /// 共通化する。この2行より外側（ループの周期・終了条件・give-up latch・
    /// write_bytes 観測ログ・完了通知の有無）は両者で意味が異なるため、あえて
    /// 呼び出し元にそのまま残す（無理に1つのポーリングループへ統合しない）。
    ///
    /// `false`（focus_gen不一致）なら呼び出し元は自分の stale 値を返すこと。
    /// `true`なら`update_ime_mode_from_imc`済みなので、続けて終端判定を行える。
    ///
    /// 2026-09-10、自由関数からメソッドへ変更した（`&Output`を引数に取り続けて
    /// いたが、同じファイル内に既存の`impl Output`ブロックがあった）。
    /// 挙動は変更していない。
    fn refresh_ime_mode_if_focus_matches(
        &self,
        expected_focus_gen: crate::state::focus_gen::FocusGen,
        conv: Option<u32>,
    ) -> bool {
        if self.ime_mode_focus_gen.get() != expected_focus_gen {
            return false;
        }
        self.update_ime_mode_from_imc(conv);
        true
    }

    /// [`Output::start_ms_ime_ready_poll`] の期限判定（ADR-140 Step1b 指摘S1対応）。
    ///
    /// conv を読めた・読めなかった（GJI actuation フェンスで abandon した）に
    /// 関わらず、**このループが必ず終了する**ことを保証するために、conv の
    /// 有無と無関係に毎tick呼べる形で分離した。呼ばなければ「abandon が連続する
    /// 限りタスクが不死になり `ms_ime_gate_give_up` を一度も立てない」という
    /// 終了保証の喪失を招く（実装レビュー指摘S1、BUG-114のような actuation
    /// ストーム下で顕在化しうる）。
    ///
    /// 2026-09-10、自由関数からメソッドへ変更した（同上）。挙動は変更していない。
    fn ms_ime_ready_poll_check_deadline(&self, deadline_ms: u64, cold_seq: Generation) -> bool {
        // ADR-084（BUG-49 追補2）: `shift-conv-guard` の hold 中は
        // `confirm_gate_deadline_override_ms` が元の `deadline_ms`
        // （送信試行時点起点）を押し出す。詳細は `MsImeReadyCoro` の
        // 同型コメント参照。
        let effective_deadline_ms = deadline_ms.max(self.confirm_gate_deadline_override_ms.get());
        if crate::hook::current_tick_ms() >= effective_deadline_ms {
            self.ms_ime_gate_give_up.set(true);
            tracing::warn!(
                "[msime-ready] cold={cold_seq} IMC 未確認のまま期限切れ \
                 (deadline=0x{effective_deadline_ms:X}) → give-up latch 設定 \
                 （フォーカス変更 / 次の IME ON / 次の conv actuation まで gate 停止）",
                cold_seq = cold_seq.value(),
            );
            true
        } else {
            false
        }
    }

    /// MS-IME confirm-then-transmit ゲート（BUG-13）の IMC 確認ポーリングを開始する。
    ///
    /// `MS_IME_READY_POLL_INTERVAL_MS` 間隔で `IMC_GETCONVERSIONMODE` を読み、
    /// `ImeModeFsm` に反映する。NATIVE 確認（Hiragana/Katakana confirmed）で終了。
    /// `deadline_ms` までに一度も確認できなければ `ms_ime_gate_give_up` を立てて
    /// 以後のゲート発動をフォーカス変更 / 次の `SetOpen(true)` まで抑止する。
    ///
    /// `ime_mode_focus_gen` の世代照合により、ポーリング中にフォーカスが変わった場合は
    /// stale 結果で `ImeModeFsm` / latch を汚染せず黙って終了する。
    /// 待機側は `MsImeReadyCoro`（`pending_tsf`）が env 経由で確認を観測する。
    ///
    /// ADR-140 Step1b（`/code-review max`指摘）: `kp_stage_idle_conv_check_inner`
    /// と全く同型の spawn→クロスプロセス conv 読み取り→`with_app` の形でありながら
    /// フェンス対象外だった。この読み取りは `confirmed=true` を実際に立てて
    /// `Output::ms_ime_gate_defer`（BUG-13）の送信可否を決めるため、GJI/actuation
    /// との交錯を見逃すと未準備な IME への早期送信を招きうる
    /// （`crate::probe_actuation_fence` module doc 参照）。issue前
    /// （checkpoint1/2）に加え read 完了直後（checkpoint3）でもフェンスを
    /// 再比較する——ループ2箇所は `fence_at_call` を await 直前で取るため
    /// checkpoint1 の窓が実質ゼロで、最も起こりやすい「`SendMessageTimeoutW`
    /// in-flight 中の actuation」は checkpoint3 でしか捕捉できない
    /// （実装レビュー指摘S2）。conv が信用できない（abandon/checkpoint3
    /// 不一致のいずれか）場合も `ms_ime_ready_poll_check_deadline` による
    /// 期限判定だけは必ず行う——これを省略すると actuation が連続する限り
    /// このタスクが不死になり `ms_ime_gate_give_up` を一度も立てなくなる
    /// （実装レビュー指摘S1）。
    ///
    /// **`/code-review max`指摘（S1実装のフォローアップ）**: 上記 abandon 分岐は
    /// 元々 `ime_mode_focus_gen` の世代照合を欠いていた——good-read 分岐は
    /// `refresh_ime_mode_if_focus_matches` 経由で必ず世代照合してから
    /// `Output` へ触れるのに対し、abandon 分岐は照合なしで
    /// `ms_ime_ready_poll_check_deadline` を呼んでいたため、フォーカスが
    /// 切り替わった後も生き続ける旧世代のこのタスクが、（グローバル1本の
    /// フェンスのため新フォーカス側の通常の IME 操作でも起こりうる）
    /// actuation との交錯でこの分岐に落ち続けると、旧世代の `deadline_ms`
    /// 期限切れを検知した瞬間に**現在の（新世代の）** `ms_ime_gate_give_up`
    /// を誤って立ててしまう——新世代は一度もタイムアウトしていないのに
    /// BUG-13 のゲートが無効化される。abandon 分岐にも世代照合を追加し、
    /// 世代が一致しない限り `Output` へ触れない（`Stale` で終了する）よう
    /// good-read 分岐と揃えた。
    pub(crate) fn start_ms_ime_ready_poll(&self, cold_seq: Generation, deadline_ms: u64) {
        let gen = self.ime_mode_focus_gen.get();
        win32_async::spawn_local(async move {
            loop {
                let fence_at_call = crate::probe_actuation_fence::current();
                let outcome =
                    crate::ime::get_ime_conversion_mode_fenced_async(10, fence_at_call).await;
                // ADR-140 Step1b 指摘S2: checkpoint1/2（issue前）だけでは、probe の
                // `SendMessageTimeoutW` が in-flight の間に発行された actuation
                // （最も起こりやすい交錯）を捕捉できない。read 完了直後にもう一度
                // フェンスを比較し（checkpoint3）、この窓も塞ぐ。issue前
                // abandon（checkpoint1/2）と合わせて `outcome` へのガード付き
                // `match` 1つに集約する（実装レビュー指摘S3: `Option` を経由して
                // `unreachable!` で取り出す形は awase のようなキーボードフック
                // プロセスでは避けたい——`outcome` を直接 match するガード条件
                // なら型システムだけでパニック不能な形にできる）。
                let status = match outcome {
                    crate::probe_actuation_fence::FencedProbeOutcome::Read(conv)
                        if crate::probe_actuation_fence::current() == fence_at_call =>
                    {
                        crate::with_app(|runtime| {
                            let out = &runtime.platform.output;
                            if !out.refresh_ime_mode_if_focus_matches(gen, conv) {
                                return MsImePollStatus::Stale;
                            }
                            if out.ime_mode_fsm.borrow().is_native_ready() {
                                return MsImePollStatus::Ready;
                            }
                            if out.ms_ime_ready_poll_check_deadline(deadline_ms, cold_seq) {
                                MsImePollStatus::Expired
                            } else {
                                MsImePollStatus::Pending
                            }
                        })
                        .unwrap_or(MsImePollStatus::Stale)
                    }
                    // Abandoned（issue前の交錯）、または Read だが read 後の
                    // checkpoint3 でフェンス不一致を検知した場合。
                    _ => {
                        tracing::debug!(
                            "[msime-ready] cold={cold_seq} GJI actuation との交錯を検知 \
                             (issue前 or read後) → このtickの読み取りを破棄（次tickへ継続）",
                            cold_seq = cold_seq.value(),
                        );
                        // `/code-review max`指摘: 指摘S1で「conv が信用できなくても
                        // deadline判定だけは行う」形にした際、good-read側の分岐が
                        // `refresh_ime_mode_if_focus_matches` 経由で必ず行っていた
                        // `ime_mode_focus_gen` 世代照合を、この分岐にだけ移植し忘れて
                        // いた。世代照合を欠くと、フォーカスが切り替わった後も生き
                        // 続ける旧世代（`gen`）のこのタスクが、GJI actuationとの交錯
                        // （このタスクが生きている限りいつでも起こりうる。フェンス
                        // グローバル1本のため、新フォーカス側の通常のIME操作でも
                        // bumpされる）でこの分岐に落ち続け、旧世代の`deadline_ms`
                        // が期限切れ（実時間は既に経過済みのため高確率で真）になった
                        // 瞬間、**現在の（新世代の）** `Output.ms_ime_gate_give_up`
                        // を誤って立ててしまう——新世代側は一度もタイムアウトして
                        // いないのに、BUG-13のconfirm-then-transmitゲートが黙って
                        // 無効化され、未準備なIMEへ早期送信しうる。good-read側と
                        // 同じ「世代が一致しない限りOutputへ触れない」規律に揃える。
                        crate::with_app(|runtime| {
                            let out = &runtime.platform.output;
                            if out.ime_mode_focus_gen.get() != gen {
                                return MsImePollStatus::Stale;
                            }
                            if out.ms_ime_ready_poll_check_deadline(deadline_ms, cold_seq) {
                                MsImePollStatus::Expired
                            } else {
                                MsImePollStatus::Pending
                            }
                        })
                        .unwrap_or(MsImePollStatus::Stale)
                    }
                };

                match status {
                    MsImePollStatus::Ready => {
                        tracing::debug!(
                            "[msime-ready] cold={cold_seq} IMC ポーリング: NATIVE 確認 → 終了",
                            cold_seq = cold_seq.value(),
                        );
                        return;
                    }
                    MsImePollStatus::Stale | MsImePollStatus::Expired => return,
                    MsImePollStatus::Pending => {}
                }
                win32_async::sleep_ms(crate::tuning::MS_IME_READY_POLL_INTERVAL_MS as u32).await;
            }
        });
    }
}

fn plan_skipped_record(cold_seq: Generation, consecutive_before: u32) -> LiteralDetectRecord {
    LiteralDetectRecord {
        cold_seq,
        facts: LiteralDetectFacts {
            verdict: LiteralVerdict::PlanSkippedLiteral,
            route: DetectRoute::PlanDecision,
            path: DetectPath::Word,
            target: DetectTarget::Chrome,
            vk: None,
            idx: 0,
            last_idx: 0,
            evidence: DetectEvidence::default(),
        },
        consecutive_before,
        gave_up: false,
        backs: 0,
        escape_composition: false,
        session_marked: false,
        romaji: None,
    }
}

/// `dispatch_probe_actions` の結果。
#[derive(Debug, Clone, Copy)]
pub(crate) struct StageEnd {
    pub(crate) reason: StageEndReason,
}

#[derive(Debug)]
pub(crate) enum DispatchResult {
    /// probe 継続（次回 tick を待つ）。
    Continue,
    /// 段は終わった。呼び出し元が段末処理を行う。
    Ended(StageEnd),
}

impl DispatchResult {
    #[cfg(test)]
    pub(crate) const fn is_done(&self) -> bool {
        matches!(self, Self::Ended(_))
    }
}

/// `RawTsfLiteralRecovery` の予約の入力(判断済みの値)。
struct RawLiteralPlan {
    /// これまでの連続 literal 回数。0 のときだけ Backspace + 再送を予約する。
    consecutive: u32,
    backs: usize,
    escape_composition: bool,
    /// 疑いが出た VK より後ろの、まだ送っていないローマ字。諦める分岐でも、これだけは送る(BUG-215)。
    rest: String,
}

/// ADR-285: Mozc の確認を張る送信は、最初の 1 キーだけ送り、残りを確認の対象に持たせる(per-VK)。
///
/// リテラルなら素通りしているのは最初の 1 キーだけなので、Backspace は 1 回に決まる。送るローマ字の長さ(バイト数 =
/// 文字数、英小文字のみ)を返す。確認を張らない、または分けられない(1 文字)ときは `None`(全体を送る)。
fn split_for_mozc_check(romaji: &str, target: Option<&mut MozcComposeTarget>) -> Option<usize> {
    let target = target?;
    let (first, rest) = crate::state::mozc_shm::split_first_key(romaji)?;
    target.romaji = first.to_string();
    target.rest = rest.to_string();
    Some(first.len())
}

/// `Transmit` の送信。Mozc の確認を張るなら最初の 1 キーだけ、そうでなければ全体を送る(ADR-285)。
fn transmit_first_or_whole<I: ProbeIo>(
    io: &I,
    romaji: &str,
    chars: &[(VkCode, bool)],
    target: Option<&mut MozcComposeTarget>,
) {
    match split_for_mozc_check(romaji, target) {
        Some(n) => io.transmit_chrome(&romaji[..n], &chars[..n]),
        None => io.transmit_chrome(romaji, chars),
    }
}

/// `TransmitRest` の送信(ADR-285)。確認は張り直さない。段を終える理由を返す。
fn transmit_rest<I: ProbeIo>(io: &I, romaji: &str) -> StageEndReason {
    let chars: VkSequence = romaji
        .chars()
        .filter_map(crate::output::resolve_ascii_to_vk)
        .collect();
    if chars.is_empty() {
        return StageEndReason::NoResolvableVk;
    }
    io.transmit_chrome(romaji, &chars);
    StageEndReason::ProbeDone
}

/// 疑いが出た VK(`failed_idx`、0 始まり)より後ろの、まだ送っていないローマ字を返す。
fn romaji_after(romaji: &str, failed_idx: usize) -> String {
    romaji
        .chars()
        .filter(|c| crate::vk::ascii_to_vk(*c).is_some())
        .skip(failed_idx + 1)
        .collect()
}

/// `RawTsfLiteralRecovery` の Backspace・再送の予約を決めて `io` に書く。
fn reserve_raw_literal_recovery<I: ProbeIo>(
    io: &I,
    cold_seq: Generation,
    plan: RawLiteralPlan,
    romaji: String,
) {
    let RawLiteralPlan {
        consecutive,
        backs,
        escape_composition,
        rest,
    } = plan;
    if consecutive == 0 {
        tracing::warn!(
            "[raw-tsf-literal] cold={cold_seq} raw TSF literal suspected \
             → backspace ×{backs} + re-send {romaji:?} scheduled + mark cold",
            cold_seq = cold_seq.value(),
        );
        io.set_raw_literal(backs, romaji, escape_composition);
    } else {
        tracing::warn!(
            "[raw-tsf-literal] cold={cold_seq} consecutive raw-tsf-literal \
             (count={}) → giving up, no backspace, re-send only the rest {rest:?}",
            consecutive + 1,
            cold_seq = cold_seq.value(),
        );
        // 以前はここで VK_IME_OFF→VK_IME_ON の reinit を予約していた(BUG-33/36/168)が、
        // 実 Chrome×GJI で 0/10 と効かず、入力中文字を消す副作用もあったので撤去した(ADR-212 P3)。
        // 見た目の掃除(BS)も、正しく入った文字を消すので送らない(Zoom 実測、BUG-215)。
        // 疑いが出た VK で per-VK の確認は終わり、後ろの VK は送られていない。「ki」の `i` が届かず「k」だけが
        // 出るのを避けるため、後ろの分だけは送る(`k` を消して `ki` を送り直すことはしない)。
        // ESC も送らない(BUG-216): 諦める時点で入力中の文字は正しく入っていることが多く、破棄すると消える。
        io.set_raw_literal(0, rest, false);
    }
}

///
/// `platform.rs` の `dispatch_probe_actions` を置き換える。
/// `io: &impl ProbeIo` で Win32 副作用を注入することでテスト可能。
#[expect(clippy::too_many_lines)]
#[tracing::instrument(level = "debug", skip_all)]
pub(crate) fn dispatch_probe_actions<M, I>(
    machine: &mut M,
    initial_actions: Vec<crate::tsf::warmup::probe_fsm::ProbeAction>,
    io: &I,
    trace: &mut LiteralDetectTrace,
) -> DispatchResult
where
    M: crate::tsf::warmup::tickable_fsm::TickableFsm + ?Sized,
    I: ProbeIo,
{
    use crate::tsf::warmup::probe_fsm::ProbeAction;
    use std::collections::VecDeque;

    let mut queue: VecDeque<ProbeAction> = initial_actions.into();

    let ended: Option<StageEndReason> = 'stage: {
        while let Some(action) = queue.pop_front() {
            match action {
                ProbeAction::Done => break 'stage Some(StageEndReason::ProbeDone),

                ProbeAction::Transmit {
                    cold_seq,
                    plan,
                    romaji,
                } => {
                    let chars: VkSequence = romaji
                        .chars()
                        .filter_map(crate::output::resolve_ascii_to_vk)
                        .collect();
                    if chars.is_empty() {
                        break 'stage Some(StageEndReason::NoResolvableVk);
                    }
                    // ADR-270: Mozc の最初の送信の後に、入力中の文字ができたかを確かめる。
                    // まだ入力中の文字が無いことを確かめるため、送信の直前に対象を決める。
                    // (Tsf 注入経路の撤去〈2026-10-22〉までは Tsf の送信だけがこれを呼んでいた。)
                    let mut mozc_check = io.mozc_compose_precheck(&romaji, cold_seq);
                    // plan.needs_literal は tsf_probe_coro_body が env.gji_active で確定済み。
                    // ADR-285: Mozc の確認を張るときは最初の 1 キーだけを送る(per-VK)。残りは確認の FSM が送る。
                    transmit_first_or_whole(io, &romaji, &chars, mozc_check.as_mut());
                    // 注入の記録は io.transmit_chrome 自身が note_stage_injection で行う。
                    // deferred VK の解放は段末（finish_probe_stage）へ一元化した（決定4-b）。
                    if !plan.needs_literal {
                        trace
                            .0
                            .push(LiteralDetectTraceItem::Verdict(plan_skipped_record(
                                cold_seq,
                                io.consecutive_count(),
                            )));
                    }
                    // `needs_literal=true` はここへ来ない(per-VK confirm が先に取る。ECRS MB-01)。
                    if let Some(target) = mozc_check {
                        io.request_mozc_compose_check(target);
                    }
                    break 'stage Some(StageEndReason::ProbeDone);
                }

                ProbeAction::TransmitRest { romaji } => {
                    // ADR-285: 最初の 1 キーの確認のあとの残り。確認は張り直さない。
                    break 'stage Some(transmit_rest(io, &romaji));
                }

                ProbeAction::TransmitSingleVk {
                    cold_seq,
                    vk,
                    needs_shift,
                    timeout_ms,
                    is_last,
                    idx,
                    last_idx,
                } => {
                    debug_assert_eq!(is_last, idx == last_idx);
                    // ベースラインは SendInput **前**に取得する（送信中の SHOW/I-O 変化を見逃さないため）。
                    // TSF/Chrome 共通で write-bytes 閾値 + 候補ウィンドウ SHOW の OR 判定を使う
                    // （BUG-30 で target 分岐を撤去して統一）。
                    //
                    // 候補可視による backspace の veto（旧 `LiteralDetectCore::veto_decision`）は
                    // per-VK 経路には無い（ADR-122 round 2）。語単位の検出コアは ECRS MB-02 で撤去した。
                    let detector = crate::tsf::probe::LiteralDetector::new_with_pre_send_baseline(
                        crate::tsf::observer::gji_write_bytes(),
                    );
                    // 注入の記録は send_single_chrome_vk 自身が note_stage_injection で行う。
                    // deferred VK の解放は段末（finish_probe_stage）へ一元化した（決定4-b）。
                    io.send_single_chrome_vk(vk, needs_shift);
                    let deadline_ms = crate::hook::current_tick_ms() + timeout_ms;
                    trace.0.push(LiteralDetectTraceItem::VkSent {
                        cold_seq: cold_seq.value(),
                        vk: vk.0,
                        idx,
                        last_idx,
                        target: DetectTarget::Chrome,
                    });
                    machine.apply_vk_sent(detector, deadline_ms);
                }

                ProbeAction::RawTsfLiteralRecovery {
                    cold_seq,
                    backs,
                    romaji,
                    escape_composition,
                    facts,
                } => {
                    // emit_recovery_actions は常にこのアクションを emit する（捨て駒キー
                    // には倒れない、2026-07-16 撤去）。consecutive==0 なら backspace + romaji
                    // 再送を scheduled し、次の cold パス（per-VK confirm）へ自然に委ねる。
                    let consecutive = io.consecutive_count();
                    let failed_idx = usize::from(facts.idx);
                    // 諦める分岐(連続 2 回目以降)は再送しないので、Backspace も送らない(BUG-215)。
                    // 文字が実際に入っていたとき、正しく入った文字を毎回 1 つ消してしまうため。
                    // (判定を信用できない窓は、そもそもここへ来ない。`Output::step_probe` が per-VK の確認を外す。)
                    let gave_up = consecutive != 0;
                    let backs = if gave_up { 0 } else { backs };
                    trace
                        .0
                        .push(LiteralDetectTraceItem::Verdict(LiteralDetectRecord {
                            cold_seq,
                            facts,
                            consecutive_before: consecutive,
                            gave_up,
                            backs,
                            escape_composition,
                            session_marked: false,
                            // BUG-74/ADR-100 決定3 案L: give-up で romaji 自体は再送されず
                            // 失われるが、journal には「何が失われたか」を残す。push は
                            // romaji が move される前（consecutive==0 分岐が io.set_raw_literal
                            // へ move する／give-up 分岐は String::new() を渡し元の romaji を
                            // 破棄する、いずれも下の if/else より前）に行うため clone が要る。
                            romaji: Some(romaji.clone()),
                        }));
                    reserve_raw_literal_recovery(
                        io,
                        cold_seq,
                        RawLiteralPlan {
                            consecutive,
                            backs,
                            escape_composition,
                            rest: romaji_after(&romaji, failed_idx),
                        },
                        romaji,
                    );
                    io.mark_cold_raw_tsf();
                }

                ProbeAction::CompositionConfirmed {
                    cold_seq,
                    mark_literal_session,
                    facts,
                } => {
                    let consecutive = io.consecutive_count();
                    trace
                        .0
                        .push(LiteralDetectTraceItem::Verdict(LiteralDetectRecord {
                            cold_seq,
                            facts,
                            consecutive_before: consecutive,
                            gave_up: false,
                            backs: 0,
                            escape_composition: false,
                            session_marked: mark_literal_session,
                            romaji: None,
                        }));
                    // BUG-27 追補4: consecutive_count は「連続失敗」の抑止用カウンタ。
                    // 語の最後の VK まで本物の CompositionConfirmed が取れたとき(= `mark_literal_session`)だけ、連続では
                    // なくなるのでリセットする。途中の VK の確認ではリセットしない(BUG-216): 2 番目のキーの疑いで
                    // ESC + 全体を再送した時、再送した 1 番目のキーの確認でカウントが 0 に戻ると、2 番目でまた疑われて
                    // 同じ回収に入り、ESC が入力中の文字を毎回破棄して文字が消えた(擬似窓で 8 回連鎖)。
                    // (従来の「セッション中に一度でも literal 化すると以後ずっと give-up に固定される」regression は、
                    // 語を最後まで確認できればリセットされるので再発しない。)
                    if mark_literal_session {
                        io.reset_consecutive_count();
                    }
                    if mark_literal_session {
                        crate::tsf::observer::mark_literal_session_confirmed(cold_seq);
                    }
                }
            }
        }
        None
    };
    ended.map_or(DispatchResult::Continue, |reason| {
        DispatchResult::Ended(StageEnd { reason })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;
    use crate::tsf::warmup::probe_fsm::{ProbeAction, TransmitPlan};
    use std::cell::Cell;

    // ── gji_reinit_poll_tick_outcome テスト（ADR-101/BUG-74 コードレビュー指摘:
    // with_app 再入(None)を Stale ではなく Continue（未観測、次tickへ継続）扱いに
    // すること）──────────────────────────────────────────────────────────────

    /// テスト用フェイク ProbeIo。Win32 副作用を no-op にし、呼び出しをフラグで記録する。
    struct FakeProbeIo {
        consecutive: u32,
        transmit_chrome_called: Cell<bool>,
        send_single_chrome_vk_call_count: Cell<u32>,
        deferred_vks_called: Cell<bool>,
        set_raw_literal_called: Cell<bool>,
        mark_cold_raw_tsf_called: Cell<bool>,
        reset_consecutive_called: Cell<bool>,
        /// `set_raw_literal` に渡された `(backs, romaji, escape_composition)`。
        raw_literal_args: std::cell::RefCell<Option<(usize, String, bool)>>,
        /// `mozc_compose_precheck` が返す対象(`None` = 張らない)。
        mozc_target: Option<MozcComposeTarget>,
        /// precheck・送信・request の呼び出し順(ADR-270 の配線の固定用)。
        calls: std::cell::RefCell<Vec<&'static str>>,
        /// `transmit_chrome` に渡された romaji(送った順)。
        transmitted: std::cell::RefCell<Vec<String>>,
        /// `request_mozc_compose_check` に渡された対象。
        requested: std::cell::RefCell<Option<MozcComposeTarget>>,
    }

    impl Default for FakeProbeIo {
        fn default() -> Self {
            Self {
                consecutive: 0,
                transmit_chrome_called: Cell::new(false),
                send_single_chrome_vk_call_count: Cell::new(0),
                deferred_vks_called: Cell::new(false),
                set_raw_literal_called: Cell::new(false),
                mark_cold_raw_tsf_called: Cell::new(false),
                reset_consecutive_called: Cell::new(false),
                raw_literal_args: std::cell::RefCell::new(None),
                mozc_target: None,
                calls: std::cell::RefCell::new(Vec::new()),
                transmitted: std::cell::RefCell::new(Vec::new()),
                requested: std::cell::RefCell::new(None),
            }
        }
    }

    impl ProbeIo for FakeProbeIo {
        fn transmit_chrome(&self, romaji: &str, _chars: &[(VkCode, bool)]) {
            self.transmit_chrome_called.set(true);
            self.transmitted.borrow_mut().push(romaji.to_string());
            self.calls.borrow_mut().push("transmit");
        }
        fn mozc_compose_precheck(
            &self,
            _romaji: &str,
            _cold_seq: Generation,
        ) -> Option<MozcComposeTarget> {
            self.calls.borrow_mut().push("precheck");
            self.mozc_target.clone()
        }
        fn request_mozc_compose_check(&self, target: MozcComposeTarget) {
            *self.requested.borrow_mut() = Some(target);
            self.calls.borrow_mut().push("request");
        }
        fn send_single_chrome_vk(&self, _vk: VkCode, _needs_shift: bool) {
            self.send_single_chrome_vk_call_count
                .set(self.send_single_chrome_vk_call_count.get() + 1);
        }
        fn send_deferred_vks(&self, _vks: &[DeferredVk], _marker: VkMarker) {
            self.deferred_vks_called.set(true);
        }
        fn consecutive_count(&self) -> u32 {
            self.consecutive
        }
        fn reset_consecutive_count(&self) {
            self.reset_consecutive_called.set(true);
        }
        fn set_raw_literal(&self, backs: usize, romaji: String, escape_composition: bool) {
            self.set_raw_literal_called.set(true);
            *self.raw_literal_args.borrow_mut() = Some((backs, romaji, escape_composition));
        }
        fn mark_cold_raw_tsf(&self) {
            self.mark_cold_raw_tsf_called.set(true);
        }
    }

    fn make_chrome_machine() -> crate::tsf::warmup::probe_fsm::TsfProbeCoro {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = crate::tsf::probe::TsfReadinessProbe::new(0, Generation::INITIAL);
        crate::tsf::warmup::probe_fsm::TsfProbeCoro::new_chrome(
            "ka",
            Generation::INITIAL,
            probe,
            guard,
        )
    }

    fn dispatch_for_test<M>(
        machine: &mut M,
        actions: Vec<ProbeAction>,
        io: &FakeProbeIo,
    ) -> DispatchResult
    where
        M: crate::tsf::warmup::tickable_fsm::TickableFsm + ?Sized,
    {
        let mut trace = LiteralDetectTrace::default();
        dispatch_probe_actions(machine, actions, io, &mut trace)
    }

    fn test_facts(verdict: LiteralVerdict) -> LiteralDetectFacts {
        LiteralDetectFacts {
            verdict,
            route: DetectRoute::CheckNow,
            path: DetectPath::Word,
            target: DetectTarget::Chrome,
            vk: None,
            idx: 0,
            last_idx: 0,
            evidence: DetectEvidence::default(),
        }
    }

    #[test]
    fn done_action_returns_true_without_side_effects() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(&mut machine, vec![ProbeAction::Done], &io);
        assert!(result.is_done());
        assert!(!io.transmit_chrome_called.get());
    }

    #[test]
    fn chrome_transmit_calls_transmit_chrome_and_mark_warm() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: Generation::INITIAL,
            plan: TransmitPlan {
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            romaji: "ka".to_string(),
        }];
        let result = dispatch_for_test(&mut machine, actions, &io);
        assert!(result.is_done());
        assert!(io.transmit_chrome_called.get());
    }

    // ── ADR-270: Mozc の確認の配線(Tsf 注入経路の撤去で Batched の Transmit へ移した。PR #809 レビュー m1) ──

    fn mozc_target() -> MozcComposeTarget {
        MozcComposeTarget {
            cold_seq: Generation::INITIAL,
            romaji: "wo".to_string(),
            hwnd_addr: 0x1234,
            thread_id: 1,
            process_id: 2,
            key_seq_before: 0,
            rest: String::new(),
        }
    }

    fn transmit(romaji: &str) -> Vec<ProbeAction> {
        vec![ProbeAction::Transmit {
            cold_seq: Generation::INITIAL,
            plan: TransmitPlan {
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            romaji: romaji.to_string(),
        }]
    }

    /// precheck は送信の**前**(まだ入力中の文字が無いことを確かめるため)、request は送信の後。
    #[test]
    fn transmit_prechecks_mozc_before_sending_and_requests_after() {
        let io = FakeProbeIo {
            mozc_target: Some(mozc_target()),
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(&mut machine, transmit("wo"), &io);
        assert!(result.is_done());
        assert_eq!(*io.calls.borrow(), ["precheck", "transmit", "request"]);
    }

    /// ADR-285: Mozc の確認を張るときは、最初の 1 キーだけを送り、残りは確認の対象に持たせる(per-VK)。
    #[test]
    fn mozc_check_sends_only_the_first_key_and_keeps_the_rest() {
        let io = FakeProbeIo {
            mozc_target: Some(mozc_target()),
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(&mut machine, transmit("ko"), &io);
        assert!(result.is_done());
        assert_eq!(*io.transmitted.borrow(), ["k"]);
        let requested = io.requested.borrow();
        let target = requested.as_ref().expect("確認が張られる");
        assert_eq!((target.romaji.as_str(), target.rest.as_str()), ("k", "o"));
    }

    /// 1 文字のローマ字は分けない。確認を張らないときは全体を一度に送る(従来どおり)。
    #[test]
    fn single_char_romaji_and_no_check_are_sent_whole() {
        let io = FakeProbeIo {
            mozc_target: Some(mozc_target()),
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let _ = dispatch_for_test(&mut machine, transmit("a"), &io);
        assert_eq!(*io.transmitted.borrow(), ["a"]);

        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let _ = dispatch_for_test(&mut machine, transmit("ko"), &io);
        assert_eq!(*io.transmitted.borrow(), ["ko"]);
        assert!(io.requested.borrow().is_none());
    }

    /// 残りの送信は確認を張り直さない(precheck を呼ばない)。
    #[test]
    fn transmit_rest_sends_without_a_new_check() {
        let io = FakeProbeIo {
            mozc_target: Some(mozc_target()),
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(
            &mut machine,
            vec![ProbeAction::TransmitRest {
                romaji: "o".to_string(),
            }],
            &io,
        );
        assert!(result.is_done());
        assert_eq!(*io.transmitted.borrow(), ["o"]);
        assert_eq!(*io.calls.borrow(), ["transmit"]);
    }

    /// precheck が対象を返さない(Mozc 系でない・窓が対象外など)ときは request しない。
    #[test]
    fn transmit_does_not_request_mozc_check_without_target() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(&mut machine, transmit("wo"), &io);
        assert!(result.is_done());
        assert_eq!(*io.calls.borrow(), ["precheck", "transmit"]);
    }

    /// 送るキーが無い(NoResolvableVk)ときは precheck も request もしない。
    #[test]
    fn no_resolvable_vk_skips_mozc_precheck_and_request() {
        let io = FakeProbeIo {
            mozc_target: Some(mozc_target()),
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(&mut machine, transmit(""), &io);
        assert!(matches!(
            result,
            DispatchResult::Ended(StageEnd {
                reason: StageEndReason::NoResolvableVk
            })
        ));
        assert!(io.calls.borrow().is_empty(), "{:?}", io.calls.borrow());
    }

    // ── ADR-103 決定4: dispatch_probe_actions の唯一の出口（StageEndReason）──

    #[test]
    fn no_resolvable_vk_ends_stage_without_transmit() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::Transmit {
            cold_seq: Generation::INITIAL,
            plan: TransmitPlan {
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            romaji: String::new(),
        }];
        let result = dispatch_for_test(&mut machine, actions, &io);
        assert!(
            matches!(
                result,
                DispatchResult::Ended(StageEnd {
                    reason: StageEndReason::NoResolvableVk
                })
            ),
            "romaji が空(chars 解決不能)の Transmit は NoResolvableVk で終わるはず: {result:?}"
        );
        assert!(!io.transmit_chrome_called.get());
    }

    #[test]
    fn probe_action_done_ends_stage_with_probe_done_reason() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let result = dispatch_for_test(&mut machine, vec![ProbeAction::Done], &io);
        assert!(
            matches!(
                result,
                DispatchResult::Ended(StageEnd {
                    reason: StageEndReason::ProbeDone
                })
            ),
            "ProbeAction::Done は ProbeDone で終わるはず: {result:?}"
        );
    }

    /// per-VK 列は段を終わらせず、1 VK を送って次 tick を待つ。
    #[test]
    fn per_vk_sends_one_vk_and_continues() {
        let io = FakeProbeIo::default();
        let mut machine = make_chrome_machine();
        let actions = vec![ProbeAction::TransmitSingleVk {
            cold_seq: Generation::INITIAL,
            vk: VkCode(0x41),
            needs_shift: false,
            timeout_ms: 100,
            is_last: false,
            idx: 1,
            last_idx: 2,
        }];
        let result = dispatch_for_test(&mut machine, actions, &io);
        assert!(
            matches!(result, DispatchResult::Continue),
            "per-VK は段を終わらせず列を継続するはず: {result:?}"
        );
        assert_eq!(io.send_single_chrome_vk_call_count.get(), 1);
    }

    // NOTE: `raw_tsf_literal_recovery_skips_set_literal_when_consecutive`（consecutive>0 で
    // set_raw_literal を呼ばない、という旧設計を検証していたテスト）は 2026-07-10 に削除した。
    // 2026-05-25 (9aa7e29) 時点の「諦めたら set_raw_literal を呼ばずスキップする」設計を
    // テストしていたが、2026-06-18 (84e6942, BUG-13 修正) で「諦めても set_raw_literal は
    // 呼び、romaji を空にして BS のみ送る（terminal に 'k'(literal)+composition が残ると
    // 文字化けするため）」という設計に意図的に変更された。この変更時に古いテストが
    // 削除されず、単一の生産コード経路に対して直下の
    // `raw_tsf_literal_recovery_consecutive_gives_up_with_cold_mark`（set_raw_literal を
    // 呼ぶことを期待）と正反対の期待値を持つ矛盾したテストペアが残っていた。
    // 現在の意図（84e6942）と一致する後者のみを残す。

    // 旧 gji_resumed_skips_literal_detect_to_prevent_false_positive テストは
    // ProbeObservations.gji_resumed 撤去（BUG-24 追補9）に伴い削除した。この signal は
    // 本番では常に false だったため（唯一の producer である gji_warmup_coro.rs の
    // 'initial ループが2分岐とも gji_resumed=false を返していた）、実際には一度も
    // 発火していなかった。当時の実機報告（WezTerm long_idle(120s) 後の 'と' 部分
    // リテラル誤判定、2026-06-20）は per-VK confirm（BUG-24 本体）が別の仕組みで
    // 解決済み。詳細は docs/known-bugs.md BUG-24 追補9参照。

    // ---- ADR-200 決定1: reinit は SuspectedLiteral の否定的証拠が累計2回そろったときだけ ----

    #[test]
    fn raw_tsf_literal_recovery_sets_literal_and_marks_cold_when_first_time() {
        let io = FakeProbeIo::default(); // consecutive == 0
        let mut machine = make_chrome_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: Generation::INITIAL,
                backs: 2,
                romaji: "ka".to_string(),
                escape_composition: false,
                facts: test_facts(LiteralVerdict::SuspectedLiteral),
            },
            ProbeAction::Done,
        ];
        let mut trace = LiteralDetectTrace::default();
        let result = dispatch_probe_actions(&mut machine, actions, &io, &mut trace);
        assert!(result.is_done());
        assert!(io.set_raw_literal_called.get());
        assert!(io.mark_cold_raw_tsf_called.get());
        assert!(
            trace.0.iter().any(|item| matches!(
                item,
                LiteralDetectTraceItem::Verdict(record)
                    if record.romaji.as_deref() == Some("ka")
            )),
            "BUG-74/ADR-100決定3案L: 初回疑いでも romaji を journal 記録に残すべき: {trace:?}"
        );
    }

    #[test]
    fn raw_tsf_literal_recovery_consecutive_gives_up_with_cold_mark() {
        // consecutive > 0 のときは諦める。
        // BUG-215 以降は Backspace を送らず、後ろのキーだけ送る(旧: BS のみ送っていたが、正しく入った文字を消し、後ろのキーも落ちた)。
        let io = FakeProbeIo {
            consecutive: 1, // already attempted once
            ..Default::default()
        };
        let mut machine = make_chrome_machine();
        let actions = vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq: Generation::INITIAL,
                backs: 2,
                romaji: "ko".to_string(),
                escape_composition: false,
                facts: test_facts(LiteralVerdict::SuspectedLiteral),
            },
            ProbeAction::Done,
        ];
        let mut trace = LiteralDetectTrace::default();
        let result = dispatch_probe_actions(&mut machine, actions, &io, &mut trace);
        assert!(result.is_done());
        assert!(
            trace.0.iter().any(|item| matches!(
                item,
                LiteralDetectTraceItem::Verdict(record)
                    if record.gave_up
                        && record.consecutive_before == 1
                        && record.facts.verdict == LiteralVerdict::SuspectedLiteral
            )),
            "give-up 分岐では gave_up=true の Verdict が trace に残るべき: {trace:?}"
        );
        assert!(
            trace.0.iter().any(|item| matches!(
                item,
                LiteralDetectTraceItem::Verdict(record)
                    if record.gave_up && record.romaji.as_deref() == Some("ko")
            )),
            "BUG-74/ADR-100決定3案L: give-up で実際には再送されない romaji も、\
             journal 記録には残すべき（次に同種の文字消失が報告されたとき、何が \
             失われたかを機械可読に復元できるようにするため）: {trace:?}"
        );
        assert!(
            io.set_raw_literal_called.get(),
            "consecutive > 0: 予約(直後の deferred の flush を駆動する)のため set_raw_literal を呼ぶべき"
        );
        assert_eq!(
            io.raw_literal_args.borrow().clone(),
            Some((0, "o".to_string(), false)),
            "BUG-215: 諦める分岐は Backspace を送らず、疑いが出た VK より後ろ(ko の o)だけ送る"
        );
        assert!(
            io.mark_cold_raw_tsf_called.get(),
            "consecutive > 0: mark_cold_raw_tsf で cold に戻すべき"
        );
    }

    #[test]
    fn romaji_after_returns_the_vks_after_the_failed_one() {
        assert_eq!(romaji_after("ki", 0), "i");
        assert_eq!(romaji_after("kya", 0), "ya");
        assert_eq!(romaji_after("kya", 1), "a");
        assert_eq!(
            romaji_after("ki", 1),
            "",
            "最後の VK で疑いが出たなら、送るものは無い"
        );
        assert_eq!(romaji_after("", 0), "");
    }
}
