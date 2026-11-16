//! mozc-readable の共有メモリの読み（ADR-265 段階 1 の影の読み + ADR-268 の belief への接続）。
//!
//! フォーカスを持つ窓のスレッドの共有メモリを、既存の IME リフレッシュの tick（約 500ms、`ir_execute`）で読む。
//!
//! - **belief への接続（ADR-268、常に動く・自動判定）**: 受け入れた読みを `ImeStateHub::report_mozc_shm` へ渡す
//!   （High・BeliefOnly。外から変わったときは既存の追随の 3 手。IME へは何も書かない）。共有メモリが無い窓
//!   （mozc-readable を使っていない利用者のほぼ全員）では `NotFound` を覚えて、フォーカスが変わるまで読まない。
//! - **診断の記録（段階 1、`[diagnostics] mozc_shm_shadow = true` のときだけ）**: フォーカスが変わった tick と、
//!   前回の記録から結論が変わった tick にだけ journal（`JournalEntry::MozcShm`）と awase.log に残す。belief との
//!   一致率・ずれを実ジャーナルで測るため。反映は再起動後（`app/bootstrap.rs` が起動時に 1 度だけ渡す）。

use crate::state::mozc_shm::{
    outcome_of, to_record, wants_timer, Outcome, ReadFailure, RecordContext, ShmUse, Trigger,
};

/// 受け入れた読みの生の値(`effective_mode`, `update_tick_ms`)。`mozc_shm_on_tick` の戻り値と同じ形。
type ModeRead = Option<(crate::state::mozc_shm::CompositionMode, u64)>;

/// 影の読みの状態。無効のときは何もしない。
pub(super) struct MozcShmShadow {
    /// 診断の記録（journal と awase.log）を出すか（`[diagnostics] mozc_shm_shadow`）。
    /// **belief への接続（ADR-268）はこれと無関係に常に動く**（自動判定。共有メモリが無い窓では何も起きない）。
    enabled: bool,
    /// 前回記録した（スレッド、プロセス、結論）。同じなら記録しない
    last: Option<(u32, u32, Outcome)>,
    /// 共有メモリが開けなかった（`NotFound`）（スレッド、プロセス）。フォーカスが変わるまで読まない
    /// （mozc-readable を使わない利用者に、毎 tick の Win32 呼び出しを課さない。ADR-268 D1）。
    absent_for: Option<(u32, u32)>,
    /// 直前の読みで、この窓のスレッドに共有メモリのブロックがあった（専用のタイマーを回し続ける。ADR-268 D5）。
    polling: bool,
    /// 直前の読みの時刻（`GetTickCount64`）。間隔が TTL を超えたことを診断のログに出すため。
    last_read_ms: Option<u64>,
    /// 直前の読みが受け入れ（`Outcome::Accepted`、`active == 1` を含む）だった（スレッド、プロセス）。それ以外の読みで
    /// 消える。ADR-292 P の始める条件 (b)（round4 U5: `polling` は `Rejected` でも真なので使わない）。
    accepted_for: Option<(u32, u32)>,
    /// ADR-292 P の打鍵の保持中(短周期の読みが代わりに読む)は、通常の専用タイマーを張らない。
    timer_suspended: bool,
    /// ADR-293 10節step5: 最後に`[mozc-direct] ack ...`をログ出力した`last_cmd_nonce`。
    /// 同じ値を毎tick出力しない(値が変わったときだけ1回。拒否率計測用、5-3節の着手判断に使う)。
    last_logged_direct_cmd_nonce: Option<u32>,
}

impl MozcShmShadow {
    /// 直前の読みで、この窓のスレッドに共有メモリのブロックがあったか（字形を belief に使わない窓の判定。ADR-273 R1）。
    pub(super) const fn block_present(&self) -> bool {
        self.polling
    }

    /// 直前の読みが、このスレッド・プロセスで受け入れだったか（ADR-292 P の始める条件 (b)）。
    pub(super) fn accepted_for(&self, tid: u32, pid: u32) -> bool {
        self.accepted_for == Some((tid, pid))
    }

    pub(super) const fn disabled() -> Self {
        Self {
            enabled: false,
            last: None,
            absent_for: None,
            polling: false,
            last_read_ms: None,
            accepted_for: None,
            timer_suspended: false,
            last_logged_direct_cmd_nonce: None,
        }
    }
}

impl super::Runtime {
    /// 起動時に、有効にするかを渡す。
    pub(crate) fn init_mozc_shm(&mut self, enabled: bool) {
        self.mozc_shm = MozcShmShadow {
            enabled,
            last: None,
            absent_for: None,
            polling: false,
            last_read_ms: None,
            accepted_for: None,
            timer_suspended: false,
            last_logged_direct_cmd_nonce: None,
        };
    }

    /// IME リフレッシュの 1 tick から呼ぶ（`ir_execute`、`app_disabled` の打ち切りの後）。IME へは何も書かない。
    ///
    /// 対象は、現在のフォーカス（`platform.focus.current`）の窓のスレッド。読む手順と受け入れ条件は
    /// `observer::mozc_shm::read` と `state::mozc_shm::outcome_of`。
    ///
    /// 戻り値（ADR-290/BUG-230、Opusレビュー後のD1″）: 受け入れた読みがあれば`(effective_mode,
    /// update_tick_ms)`。半角英数トグルを手放すべきかの判断はここでは一切行わない(belief も書かない)
    /// ——`gate.half_width_alnum`に届く呼び出し元(`ir_abandon_half_width_alnum_from_mozc_shm_if`)が、
    /// トグルのエントリ tick(`entered_at_tick_ms`)と合わせて edge-trigger の判断をする。このファイルは
    /// `gate`に一切触れない設計を保つ（`mozc_shm_reader_touches_belief_only_through_report_mozc_shm`）。
    #[must_use]
    pub(super) fn mozc_shm_on_tick(
        &mut self,
        focus_changed: bool,
    ) -> Option<(crate::state::mozc_shm::CompositionMode, u64)> {
        let (mode_read, _) = self.mozc_shm_read_once(focus_changed, None);
        self.mozc_shm_ensure_timer();
        mode_read
    }

    /// 専用のタイマー（`TIMER_MOZC_SHM`、ADR-268 D5）の満了。ハンドラ冒頭で kill 済み。
    ///
    /// refresh の tick は、明示意図があるとき・効果的に TSF ネイティブなときに止まる（他の stage を走らせないため）。
    /// 共有メモリの読み・追随だけを切り離して、ブロックがある間、約 `MOZC_SHM_TIMER_MS` ごとに回す。無効アプリに
    /// フォーカスがあるときは読まない（refresh の tick と同じ）。
    ///
    /// 引数・戻り値は `mozc_shm_on_tick` と同じ。
    #[must_use]
    pub(super) fn mozc_shm_on_timer(
        &mut self,
    ) -> Option<(crate::state::mozc_shm::CompositionMode, u64)> {
        if self.platform_state.focus.app_disabled {
            self.mozc_shm.polling = false;
            return None;
        }
        let (mode_read, _) = self.mozc_shm_read_once(false, None);
        self.mozc_shm_ensure_timer();
        mode_read
    }

    /// ADR-292 P(BUG-233): 打鍵の保持の解放判定のための読み（短い周期、`runtime/key_effect_open_hold.rs` から）。
    ///
    /// 読み方・belief への入口（`report_mozc_shm`）は通常の読みと同じ 1 か所で、違うのは `decide_use` の一致する読みの
    /// 起点（`baseline_ms`）だけ: 保持（抑止）した打鍵でも進むローリング値（`last_operation_tick_ms`）ではなく、呼び出し側が
    /// 作った `hold_baseline_ms`（予測の時刻と S1 の送信打刻の大きい方、ADR-292 round3 T1）を使う。食い違う読みの起点は
    /// 通常どおり `report_mozc_shm` が合成する（G3）。戻り値は受け入れた読みの判断（`None` = 読めなかった・受け入れなかった）。
    /// 無効アプリの判定は呼び出し側が行う（ここは `platform_state` に触れる箇所を増やさない）。
    pub(super) fn mozc_shm_read_for_hold(&mut self, hold_baseline_ms: u64) -> Option<ShmUse> {
        let (_, used) = self.mozc_shm_read_once(false, Some(hold_baseline_ms));
        used
    }

    /// ADR-292 P の打鍵の保持の間、通常の専用タイマー(`TIMER_MOZC_SHM`、500ms)を止める(`true`)・戻す(`false`)。
    /// 保持中は短周期の読み(`mozc_shm_read_for_hold`)が代わりに読むので、同じ窓を二重に読んで判定しない。戻すときは
    /// ブロックがあれば(`polling`)すぐ張り直す。refresh の tick の読み(`mozc_shm_on_tick`)は止めない。
    pub(super) fn mozc_shm_suspend_timer(&mut self, suspend: bool) {
        self.mozc_shm.timer_suspended = suspend;
        if suspend {
            self.platform.timer.kill(crate::TIMER_MOZC_SHM);
        } else {
            self.mozc_shm_ensure_timer();
        }
    }

    /// 現在のフォーカスの窓のスレッドで、直前の読みが受け入れだったか（ADR-292 P の始める条件 (b)）。
    pub(super) fn mozc_shm_accepted_for_focus(&self) -> bool {
        // ADR-293(BUG-234、round2レビュー指摘L4): スタンプ側(`WindowsPlatform::mozc_target()`)
        // と同じ式を使う——項目0の(tid,pid)一致判定は読み側とスタンプ側の式の一致に依存する。
        self.platform
            .mozc_target()
            .is_some_and(|(tid, pid)| self.mozc_shm.accepted_for(tid, pid))
    }

    /// ブロックがある間は専用のタイマーを張る（既に張ってあれば触らない）。
    fn mozc_shm_ensure_timer(&mut self) {
        if self.mozc_shm.polling
            && !self.mozc_shm.timer_suspended
            && !self.platform.timer.is_active(crate::TIMER_MOZC_SHM)
        {
            self.platform.timer.set(
                crate::TIMER_MOZC_SHM,
                std::time::Duration::from_millis(crate::tuning::MOZC_SHM_TIMER_MS),
            );
        }
    }

    /// 現在のフォーカスの窓のスレッドの共有メモリを 1 回読んで、belief に使う（診断の記録を含む）。
    ///
    /// `focus_changed` と 1 つ目の戻り値は `mozc_shm_on_tick` と同じ。`hold_baseline_ms` が `Some` なら、`decide_use` の
    /// 一致する読みの起点をローリング値の代わりにそれにする（ADR-292 P、`mozc_shm_read_for_hold`）。2 つ目の戻り値は
    /// 受け入れた読みの判断（`report_mozc_shm` の戻り値。読めなかった・受け入れなかったなら `None`）。
    fn mozc_shm_read_once(
        &mut self,
        focus_changed: bool,
        hold_baseline_ms: Option<u64>,
    ) -> (ModeRead, Option<ShmUse>) {
        self.mozc_shm.polling = false;
        self.mozc_shm.accepted_for = None;
        // ADR-293(BUG-234、round2レビュー指摘L4): スタンプ側(`WindowsPlatform::mozc_target()`)
        // と同じ式を使う(項目0の(tid,pid)一致判定は読み側とスタンプ側の式の一致に依存する)。
        let Some((tid, pid)) = self.platform.mozc_target() else {
            return (None, None);
        };

        // 開けなかった窓は、フォーカスが変わるまで読まない（ADR-268 D1）。
        if !focus_changed && self.mozc_shm.absent_for == Some((tid, pid)) {
            return (None, None);
        }
        let (reading, direct_ack, read_us) = crate::observer::mozc_shm::read(tid, pid);
        let outcome = outcome_of(&reading, pid);
        self.mozc_shm.absent_for =
            matches!(outcome, Outcome::Failed(ReadFailure::NotFound)).then_some((tid, pid));
        self.mozc_shm.accepted_for = matches!(outcome, Outcome::Accepted(_)).then_some((tid, pid));
        self.mozc_shm.polling = wants_timer(&outcome);
        // ADR-293 10節step4/5: direct actuationの宛先(`cmd_window_handle`)を更新する。
        // 受け入れでない読み・version 6未満・まだ窓が無い(`cmd_window_handle == 0`)ときは
        // `None`——`mozc_direct_target()`が無条件で上書きするので、ここも無条件で呼ぶ
        // (古い値を残さない、`polling`等と同じ作法)。
        self.platform.set_mozc_direct_readiness(
            matches!(outcome, Outcome::Accepted(_))
                .then(|| direct_ack)
                .flatten()
                .filter(|a| a.cmd_window_handle != 0)
                .map(|a| (tid, pid, a.cmd_window_handle)),
        );
        // 読みの間隔が TTL（`MOZC_SHM_OBSERVATION_TTL_MS`）を超えたら記録する（TTL の見直しの材料。ADR-268）。
        let stamp_ms = crate::hook::current_tick_ms();
        if let Some(prev) = self.mozc_shm.last_read_ms {
            let gap = stamp_ms.saturating_sub(prev);
            if self.mozc_shm.polling && gap > crate::tuning::MOZC_SHM_OBSERVATION_TTL_MS {
                tracing::info!(target: "awase::mozc_shm", "[mozc-shm] read gap {gap}ms exceeds the observation TTL");
            }
        }
        self.mozc_shm.last_read_ms = Some(stamp_ms);

        // ADR-268: 受け入れた読みを belief に使う（IME へは何も書かない）。1 tick に 1 回、変化の有無によらず
        // 報告する（`MozcShm` の観測は TTL を持ち、報告が止まると失効するため）。基準の時刻は、awase の最後の操作
        // （全キー + ImmCross の発行）。`now_ms`・`update_tick_ms` は共に `GetTickCount64`。
        //
        // ADR-290/BUG-230(D1″): `effective_mode`・`update_tick_ms`はbeliefを経由せず、ここで読んだ
        // 生の値をそのまま呼び出し元まで持ち出す(`mode_read`)。トグルを手放すべきかの判断
        // (edge-trigger、エントリ tick との比較)は`gate`に届く呼び出し元の仕事。
        let (mode_read, used) = if let (Outcome::Accepted(shm), Ok(raw)) =
            (&outcome, &reading.block)
        {
            let now_ms = crate::hook::current_tick_ms();
            // ADR-292 P(round3 T1): 打鍵の保持の解放判定の読みでは、保持した打鍵でも進むローリング値を使わない。
            let baseline_ms = hold_baseline_ms.unwrap_or_else(crate::hook::last_operation_tick_ms);
            // ADR-292 D1b/G3(BUG-233): belief と食い違う読みのsettle起点は、全フックコールバックで
            // 進む baseline_ms ではなく、IMEの開閉/モードを動かす目的の送信だけに限定する
            // (platform_state.rs::report_mozc_shmのdocコメント参照。計算自体はそちら側で行う、
            // ここでは生の時刻だけを渡す)。
            // ADR-293 10節step5: direct actuationの発行時刻もこのbaselineに合流させる
            // (own_send自体は消費・supersedeされて消えるため、消えた後も「最近送った」
            // 事実を残す別経路が要る、`hook::note_mozc_direct_post`のdoc参照)。
            let ime_actuation_baseline_ms = crate::hook::imm_cross_write_tick_ms()
                .max(crate::hook::ime_actuation_send_tick_ms())
                .max(crate::hook::mozc_direct_post_tick_ms());
            // ADR-293(BUG-234): own_sendは単一スロットのグローバルなスタンプ(`hook`)。
            // `report_mozc_shm`はSplitの早期returnより前にこれを分類し(5-2節冒頭)、
            // 消すべきと判定したら2つ目の戻り値で伝え返す(`platform_state.rs`は`hook`を
            // 直接呼べないため)。
            let own_send = crate::hook::own_send();
            // ADR-293 10節step3: `direct_ack`は`observer::mozc_shm::read`が`shm`/`raw`と
            // 同じ2段目のseqlock読みから取った、version 6のdirect actuation確認結果
            // (生の解釈)。`classify_own_request`が期待する既に解釈済みの`OwnSendAck`へ
            // ここで変換する(nonceの一致判定自体は`classify_own_request`項目1の仕事、
            // ここは型の変換だけ)。`last_cmd_nonce == 0`("まだ1件も処理していない")は
            // `None`のまま渡す。
            let ack = direct_ack.filter(|a| a.last_cmd_nonce != 0).map(|a| {
                crate::state::mozc_shm::OwnSendAck {
                    nonce: a.last_cmd_nonce,
                    applied: matches!(
                        a.last_cmd_result,
                        crate::state::mozc_shm::DirectActuationResult::Applied
                    ),
                    update_tick_ms: raw.update_tick_ms,
                }
            });
            // ADR-293 10節step5: 拒否率計測用のログ(5-3節の着手判断に使う)。新しいnonceを
            // 観測したときだけ1回出す(同じ値を毎tick出力しない)。`own_send`が同じnonceを
            // 持っていればそこから送信時刻を取り、latencyを添える(消費済みで無ければ無し)。
            if let Some(a) = direct_ack.filter(|a| a.last_cmd_nonce != 0) {
                if self.mozc_shm.last_logged_direct_cmd_nonce != Some(a.last_cmd_nonce) {
                    self.mozc_shm.last_logged_direct_cmd_nonce = Some(a.last_cmd_nonce);
                    let latency_ms = own_send
                        .filter(|s| s.nonce == Some(a.last_cmd_nonce))
                        .map(|s| raw.update_tick_ms.saturating_sub(s.tick_ms));
                    tracing::info!(
                        target: "awase::mozc_shm",
                        "[mozc-direct] ack nonce={} result={:?} latency_ms={:?}",
                        a.last_cmd_nonce,
                        a.last_cmd_result,
                        latency_ms
                    );
                }
            }
            let (used, clear_own_send) = self.platform_state.ime.report_mozc_shm(
                *shm,
                raw.update_tick_ms,
                now_ms,
                baseline_ms,
                ime_actuation_baseline_ms,
                own_send,
                ack,
                (tid, pid),
                crate::state::TickMs(now_ms),
            );
            if clear_own_send {
                crate::hook::clear_own_send();
            }
            (
                shm.effective_mode.map(|mode| (mode, raw.update_tick_ms)),
                Some(used),
            )
        } else {
            (None, None)
        };

        // 以降は診断の記録（`[diagnostics] mozc_shm_shadow = true` のときだけ）。
        if !self.mozc_shm.enabled {
            return (mode_read, used);
        }
        let key = (tid, pid, outcome);
        let changed = self.mozc_shm.last != Some(key);
        if !focus_changed && !changed {
            return (mode_read, used);
        }
        self.mozc_shm.last = Some(key);

        let trigger = if focus_changed {
            Trigger::Focus
        } else {
            Trigger::Change
        };
        let profile = super::lang_detect_shadow::profile_label(self.platform.current_app_profile());
        let belief_open = Some(self.platform_state.ime.effective_open());
        let belief_mode = Some(format!("{:?}", self.platform_state.ime.input_mode()));
        let record = to_record(
            &outcome,
            RecordContext {
                trigger,
                route: reading.route,
                read_us,
                profile,
                thread_id: tid,
                process_id: pid,
                belief_open,
                belief_mode,
            },
        );
        // 実機の測定用に、記録の全項目を 1 行の JSON で awase.log にも出す(info。専用の target なので `RUST_LOG` の指定は要らない)。
        // 時刻は awase.log の行頭の壁時計(UTC)。journal 本体は `elapsed_ms`(journal 作成からの経過)で、壁時計ではない。
        // 記録 1 件につき 1 行(フォーカス変更時と結論が変わったときだけ)で、打鍵の内容は含まない。
        match serde_json::to_string(&record) {
            Ok(json) => {
                tracing::info!(target: "awase::mozc_shm", "[mozc-shm] {json}");
            }
            Err(e) => {
                tracing::warn!(target: "awase::mozc_shm", "[mozc-shm] cannot serialize the record: {e}");
            }
        }
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::MozcShm { record });
        (mode_read, used)
    }
}
