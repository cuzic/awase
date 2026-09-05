//! BUG-113 診断専用スパイク・第3弾（一時的、恒久機能ではない）。
//!
//! docs/known-bugs.md BUG-113・docs/experiments.md エントリ21参照。
//! 第2弾（`ime_controller.rs`の`diag_bug113_dedup_gji_off_actuation`・
//! `runtime/key_pipeline.rs`の`diag_bug113_skip_idle_conv_probe`）は
//! 手動でconfig.tomlを書き換えてawaseを再起動する形で1条件ずつ検証した
//! ——dedup単体を有効化した実機テストで「@」が54エピソード連続で0件に
//! なるという強い結果が出た。本モジュールは同じ2条件（dedup・probe
//! skip）を **1回のテストセッション内で自動ローテーション** できるように
//! する、`ime_controller.rs`と`runtime/key_pipeline.rs`の両方から参照
//! される共有状態。
//!
//! 物理キーイベント1回（`kp_run_inner`1呼び出し、KeyDownのみ）につき
//! 1回だけ次のコンボを払い出す。同じイベント処理内で probe 側
//! （`kp_stage_idle_conv_check`、イベント処理の前半）・actuation側
//! （`GjiDirectStrategy::apply`、イベント処理の後半）の両方が同じ
//! コンボを参照できるよう、`select_for_new_event`で1回だけ更新し、
//! それ以降は`current_combo_flags`で読むだけにする（単一スレッドの
//! 同期処理内でのみ呼ばれるため追加の同期は不要）。
//!
//! 調査終了後は本ファイル一式・`lib.rs`の宣言・呼び出し元
//! （`key_pipeline.rs`・`ime_controller.rs`・`app/bootstrap.rs`・
//! `runtime/mod.rs`・`config.rs`）を削除すること。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// 4条件を自動ローテーションするか。
static COMBO_CYCLE_ENABLED: AtomicBool = AtomicBool::new(false);
/// 巡回カウンタ（次に選ぶコンボ番号を決める、単調増加）。
static COMBO_COUNTER: AtomicU32 = AtomicU32::new(0);
/// 現在処理中の物理キーイベントに対して選択済みのコンボ番号
/// （0=baseline, 1=dedupのみ, 2=probe skipのみ, 3=両方）。
static CURRENT_COMBO: AtomicU32 = AtomicU32::new(0);
/// 巡回するコンボ数。
const COMBO_COUNT: u32 = 4;

/// `config.toml`の`diag_bug113_combo_cycle_enabled`に応じて直接呼ぶ。
pub(crate) fn set_combo_cycle_enabled(enabled: bool) {
    if enabled != COMBO_CYCLE_ENABLED.swap(enabled, Ordering::Relaxed) {
        log::info!(
            "[bug113-diag3] combo cycle: {}",
            if enabled { "有効化" } else { "無効化" }
        );
    }
}

pub(crate) fn combo_cycle_enabled() -> bool {
    COMBO_CYCLE_ENABLED.load(Ordering::Relaxed)
}

/// 新しい物理キーイベントの処理開始時（`kp_run_inner`冒頭、KeyDownのみ）に
/// 1回だけ呼ぶ。巡回中の次のコンボを選び、以後このイベント処理が終わる
/// まで`current_combo_flags`はこの値を返す。無効時は何もしない。
pub(crate) fn select_for_new_event() {
    if !combo_cycle_enabled() {
        return;
    }
    let combo = COMBO_COUNTER.fetch_add(1, Ordering::Relaxed) % COMBO_COUNT;
    CURRENT_COMBO.store(combo, Ordering::Relaxed);
}

/// 現在のコンボ番号に応じた `(dedup有効か, probe skipか)` を返す。
/// 巡回無効時は常に `(false, false)`（呼び出し元は個別の hidden config
/// トグルにフォールバックすること）。
pub(crate) fn current_combo_flags() -> (bool, bool) {
    if !combo_cycle_enabled() {
        return (false, false);
    }
    match CURRENT_COMBO.load(Ordering::Relaxed) {
        1 => (true, false),
        2 => (false, true),
        3 => (true, true),
        _ => (false, false),
    }
}

/// ログ用にコンボの組み合わせを1つのラベル文字列へ変換する。
pub(crate) fn combo_label(dedup: bool, skip_probe: bool) -> &'static str {
    match (dedup, skip_probe) {
        (false, false) => "0(baseline)",
        (true, false) => "1(dedup)",
        (false, true) => "2(skip-probe)",
        (true, true) => "3(dedup+skip-probe)",
    }
}
