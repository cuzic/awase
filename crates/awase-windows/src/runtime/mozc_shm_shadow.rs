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
    outcome_of, to_record, wants_timer, Outcome, ReadFailure, RecordContext, Trigger,
};

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
}

impl MozcShmShadow {
    /// 直前の読みで、この窓のスレッドに共有メモリのブロックがあったか（字形を belief に使わない窓の判定。ADR-273 R1）。
    pub(super) const fn block_present(&self) -> bool {
        self.polling
    }

    pub(super) const fn disabled() -> Self {
        Self {
            enabled: false,
            last: None,
            absent_for: None,
            polling: false,
            last_read_ms: None,
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
        };
    }

    /// IME リフレッシュの 1 tick から呼ぶ（`ir_execute`、`app_disabled` の打ち切りの後）。何も変えない。
    ///
    /// 対象は、現在のフォーカス（`platform.focus.current`）の窓のスレッド。読む手順と受け入れ条件は
    /// `observer::mozc_shm::read` と `state::mozc_shm::outcome_of`。
    pub(super) fn mozc_shm_on_tick(&mut self, focus_changed: bool) {
        self.mozc_shm_read_once(focus_changed);
        self.mozc_shm_ensure_timer();
    }

    /// 専用のタイマー（`TIMER_MOZC_SHM`、ADR-268 D5）の満了。ハンドラ冒頭で kill 済み。
    ///
    /// refresh の tick は、明示意図があるとき・効果的に TSF ネイティブなときに止まる（他の stage を走らせないため）。
    /// 共有メモリの読み・追随だけを切り離して、ブロックがある間、約 `MOZC_SHM_TIMER_MS` ごとに回す。無効アプリに
    /// フォーカスがあるときは読まない（refresh の tick と同じ）。
    pub(super) fn mozc_shm_on_timer(&mut self) {
        if self.platform_state.focus.app_disabled {
            self.mozc_shm.polling = false;
            return;
        }
        self.mozc_shm_read_once(false);
        self.mozc_shm_ensure_timer();
    }

    /// ブロックがある間は専用のタイマーを張る（既に張ってあれば触らない）。
    fn mozc_shm_ensure_timer(&mut self) {
        if self.mozc_shm.polling && !self.platform.timer.is_active(crate::TIMER_MOZC_SHM) {
            self.platform.timer.set(
                crate::TIMER_MOZC_SHM,
                std::time::Duration::from_millis(crate::tuning::MOZC_SHM_TIMER_MS),
            );
        }
    }

    /// 現在のフォーカスの窓のスレッドの共有メモリを 1 回読んで、belief に使う（診断の記録を含む）。
    fn mozc_shm_read_once(&mut self, focus_changed: bool) {
        self.mozc_shm.polling = false;
        let hwnd_addr = self.platform.focus.current.hwnd;
        let pid = self.platform.focus.current.pid;
        if hwnd_addr == 0 || pid == 0 {
            return;
        }
        let tid = crate::observer::mozc_shm::thread_of_window(hwnd_addr);
        if tid == 0 {
            return;
        }

        // 開けなかった窓は、フォーカスが変わるまで読まない（ADR-268 D1）。
        if !focus_changed && self.mozc_shm.absent_for == Some((tid, pid)) {
            return;
        }
        let (reading, read_us) = crate::observer::mozc_shm::read(tid, pid);
        let outcome = outcome_of(&reading, pid);
        self.mozc_shm.absent_for =
            matches!(outcome, Outcome::Failed(ReadFailure::NotFound)).then_some((tid, pid));
        self.mozc_shm.polling = wants_timer(&outcome);
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
        if let (Outcome::Accepted(shm), Ok(raw)) = (&outcome, &reading.block) {
            let now_ms = crate::hook::current_tick_ms();
            let baseline_ms = crate::hook::last_operation_tick_ms();
            self.platform_state.ime.report_mozc_shm(
                *shm,
                raw.update_tick_ms,
                now_ms,
                baseline_ms,
                crate::state::TickMs(now_ms),
            );
        }

        // 以降は診断の記録（`[diagnostics] mozc_shm_shadow = true` のときだけ）。
        if !self.mozc_shm.enabled {
            return;
        }
        let key = (tid, pid, outcome);
        let changed = self.mozc_shm.last != Some(key);
        if !focus_changed && !changed {
            return;
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
    }
}
