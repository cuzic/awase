//! ADR-265 段階 1: mozc-readable の共有メモリの**影の読み**（挙動を変えない観測）。
//!
//! フォーカスを持つ窓のスレッドの共有メモリを、既存の IME リフレッシュの tick（約 500ms、`ir_execute`）で読み、
//! フォーカスが変わった tick と、前回の記録から結論が変わった tick にだけ journal（`JournalEntry::MozcShm`）に残す。
//! **belief には入れない・IME へ何も書かない・打鍵は変えない**。目的は、読める窓で belief との一致率を、観測の無い窓で
//! belief とのずれを、実ジャーナルで測ること（ADR-265 段階 1。belief への配線は段階 2 で別レビュー、信頼度は Low から）。
//!
//! 有効なのは `[diagnostics] mozc_shm_shadow = true`（既定は無効のオプトイン）のときだけ。反映は再起動後
//! （`app/bootstrap.rs` が起動時に 1 度だけ渡す）。

use crate::state::mozc_shm::{outcome_of, to_record, Outcome, RecordContext, Trigger};

/// 影の読みの状態。無効のときは何もしない。
pub(super) struct MozcShmShadow {
    enabled: bool,
    /// 前回記録した（スレッド、プロセス、結論）。同じなら記録しない
    last: Option<(u32, u32, Outcome)>,
}

impl MozcShmShadow {
    pub(super) const fn disabled() -> Self {
        Self {
            enabled: false,
            last: None,
        }
    }
}

impl super::Runtime {
    /// 起動時に、有効にするかを渡す。
    pub(crate) fn init_mozc_shm(&mut self, enabled: bool) {
        self.mozc_shm = MozcShmShadow {
            enabled,
            last: None,
        };
    }

    /// IME リフレッシュの 1 tick から呼ぶ（`ir_execute`、`app_disabled` の打ち切りの後）。何も変えない。
    ///
    /// 対象は、現在のフォーカス（`platform.focus.current`）の窓のスレッド。読む手順と受け入れ条件は
    /// `observer::mozc_shm::read` と `state::mozc_shm::outcome_of`。
    pub(super) fn mozc_shm_on_tick(&mut self, focus_changed: bool) {
        if !self.mozc_shm.enabled {
            return;
        }
        let hwnd_addr = self.platform.focus.current.hwnd;
        let pid = self.platform.focus.current.pid;
        if hwnd_addr == 0 || pid == 0 {
            return;
        }
        let tid = crate::observer::mozc_shm::thread_of_window(hwnd_addr);
        if tid == 0 {
            return;
        }

        let (reading, read_us) = crate::observer::mozc_shm::read(tid, pid);
        let outcome = outcome_of(&reading, pid);
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
