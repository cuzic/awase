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

use crate::state::mozc_shm::{outcome_of, to_record, Outcome, ReadFailure, RecordContext, Trigger};

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
}

impl MozcShmShadow {
    pub(super) const fn disabled() -> Self {
        Self {
            enabled: false,
            last: None,
            absent_for: None,
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
        };
    }

    /// IME リフレッシュの 1 tick から呼ぶ（`ir_execute`、`app_disabled` の打ち切りの後）。何も変えない。
    ///
    /// 対象は、現在のフォーカス（`platform.focus.current`）の窓のスレッド。読む手順と受け入れ条件は
    /// `observer::mozc_shm::read` と `state::mozc_shm::outcome_of`。
    pub(super) fn mozc_shm_on_tick(&mut self, focus_changed: bool) {
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
