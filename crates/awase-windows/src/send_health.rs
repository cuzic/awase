//! IMM32 クロスプロセス呼び出し(`imm::send_ime_control`、`SendMessageTimeoutW` ベース)の
//! 実測レイテンシを追跡し、直近が遅かった間は同期呼び出しサイトへ発行を見送らせる
//! サーキットブレーカ(BUG-34 横展開 Step0-c)。
//!
//! # 背景
//! `SMTO_ABORTIFHUNG` は呼び出し時点で相手スレッドが既にハングとマークされている
//! 場合のみ即座に打ち切る。呼び出し中に相手が応答不能になり始めた場合、Windows が
//! ハングと判定するまで(既定 `HungAppTimeout` ~5000ms)呼び出し元は普通にブロック
//! し続け、呼び出し側が指定した小さな `timeout_ms` はこの一次ブロックには効かない
//! (`docs/known-bugs.md` BUG-34)。
//!
//! # 限界(重要)
//! このブレーカは**初回の ~5s ブロックを防げない**(再発だけを止める)。真にブロックを
//! 消すには当該呼び出しを `win32_async::offload` でエンジンスレッドから追い出す必要が
//! ある。ここでの計測は tuning-constants.md が要求する実測根拠(BUG-34 実測の
//! WezTerm 5741ms 以外に正常時の p50/p99 が記録されていない)を作る作業も兼ねる。
//!
//! # スレッド安全性
//! `imm::send_ime_control` はエンジンスレッドから直接呼ばれる場合と、
//! `win32_async::offload` 経由でワーカースレッドから呼ばれる場合の両方があるため、
//! ここは `SingleThreadCell` ではなく `Atomic*` で実装する。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// この ms 以上かかった呼び出しを「遅い」とみなす。
///
/// 【暫定値、実測前】BUG-34 実測(WezTerm, ~5741ms)以外に正常時のレイテンシ分布が
/// 無いため、実機ログ(`[send-health]` ログ)を集めてから見直すこと
/// (`.claude/rules/tuning-constants.md` の実測義務)。宣言タイムアウトが
/// 5/10/20/50/150ms とサイトごとにばらついている実態を踏まえ、まずは
/// どの宣言タイムアウトよりも大きい値を暫定的に置く。
const SLOW_THRESHOLD_MS: u64 = 100;

/// slow 判定後、同期サイトへの新規発行を見送る期間。
///
/// 【暫定値、実測前】上記と同じ理由で実機ログ確定後に見直す。
const COOLDOWN_MS: u64 = 2000;

/// この回数だけ連続で `SLOW_THRESHOLD_MS` を超えて初めてブレーカを作動させる。
///
/// BUG-34 横展開レビュー指摘: 当初は1回の slow 判定だけで即座にブレーカを
/// 作動させていたが、一時的な GC 停止等の単発スパイク1回で
/// A（ime_refresh.rs）・C（apply_focus_probe）・E（romaji_pre_write）の
/// 3サイトが2秒間まとめて degrade してしまう。2回連続を要求することで、
/// 「相手が本当にハングし始めている」兆候（連続して遅い）と単発スパイクを
/// 区別する。
const TRIP_AFTER_CONSECUTIVE_SLOW: u32 = 2;

struct SendHealth {
    /// 直近の呼び出しの実測ms(診断ログ用、安全条件には使わない)。
    last_elapsed_ms: AtomicU64,
    /// 連続して `SLOW_THRESHOLD_MS` を超えた回数。
    consecutive_slow: AtomicU32,
    /// 直近の slow 判定が発生した絶対時刻(`hook::current_tick_ms()` 基準)。0 = 未発生。
    last_slow_at_ms: AtomicU64,
}

static SEND_HEALTH: SendHealth = SendHealth {
    last_elapsed_ms: AtomicU64::new(0),
    consecutive_slow: AtomicU32::new(0),
    last_slow_at_ms: AtomicU64::new(0),
};

/// `imm::send_ime_control` の呼び出し前後で計測した実測msを記録する。
///
/// エンジンスレッド・ワーカースレッドいずれからも呼ばれうる。全呼び出しについて
/// 無条件に記録する(計測はブレーカの発行可否に関わらず常に行う)。
pub(crate) fn record(elapsed_ms: u64, now_ms: u64) {
    SEND_HEALTH
        .last_elapsed_ms
        .store(elapsed_ms, Ordering::Relaxed);
    if elapsed_ms >= SLOW_THRESHOLD_MS {
        let n = SEND_HEALTH.consecutive_slow.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= TRIP_AFTER_CONSECUTIVE_SLOW {
            SEND_HEALTH.last_slow_at_ms.store(now_ms, Ordering::Relaxed);
            log::warn!(
                "[send-health] slow IMM call: {elapsed_ms}ms (連続{n}回目) — 以後 {COOLDOWN_MS}ms は同期サイトの発行を見送る"
            );
        } else {
            log::debug!(
                "[send-health] slow IMM call: {elapsed_ms}ms (連続{n}回目、{TRIP_AFTER_CONSECUTIVE_SLOW}回連続でブレーカ作動)"
            );
        }
    } else {
        SEND_HEALTH.consecutive_slow.store(0, Ordering::Relaxed);
    }
}

/// 直近の `imm::send_ime_control` 呼び出しの実測ms。
///
/// バグ報告の内部状態スナップショットに載せる診断用（BUG-34 の切り分け）。
/// 呼び出しが一度も無ければ 0。
pub(crate) fn last_elapsed_ms() -> u64 {
    SEND_HEALTH.last_elapsed_ms.load(Ordering::Relaxed)
}

/// `SLOW_THRESHOLD_MS` 以上かかった呼び出しの連続回数。
///
/// バグ報告の内部状態スナップショットに載せる診断用。`TRIP_AFTER_CONSECUTIVE_SLOW`
/// 未満でもブレーカ作動の予兆として意味がある。
pub(crate) fn consecutive_slow() -> u32 {
    SEND_HEALTH.consecutive_slow.load(Ordering::Relaxed)
}

/// 同期サイトがこの瞬間に IMM 読み書きを発行してよいか。
///
/// 偽が返った場合、呼び出し元は読み書きを発行せず degrade(例: `None` を渡す/
/// 書き込みを見送る)へ倒すこと。再武装(このブレーカが再び true を返すようになる
/// タイミング)の確認は offload 済み経路からのみ行い、同期サイト自身はここでは
/// 待たない。
///
/// 【限界】直近に slow が無ければ true を返す。つまり初回の ~5s ブロックはこの
/// 関数では防げない — 防ぐには当該呼び出し自体を offload する必要がある。
pub(crate) fn blocking_allowed(now_ms: u64) -> bool {
    is_blocking_allowed(SEND_HEALTH.last_slow_at_ms.load(Ordering::Relaxed), now_ms)
}

/// [`blocking_allowed`] の純粋ロジック(static を触らないユニットテスト用に分離)。
const fn is_blocking_allowed(last_slow_at_ms: u64, now_ms: u64) -> bool {
    last_slow_at_ms == 0 || now_ms.saturating_sub(last_slow_at_ms) >= COOLDOWN_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_when_never_slow() {
        assert!(is_blocking_allowed(0, 0));
        assert!(is_blocking_allowed(0, 999_999));
    }

    #[test]
    fn blocked_immediately_after_slow_call() {
        assert!(!is_blocking_allowed(1_000, 1_000));
        assert!(!is_blocking_allowed(1_000, 1_000 + COOLDOWN_MS - 1));
    }

    #[test]
    fn allowed_again_after_cooldown_elapses() {
        assert!(is_blocking_allowed(1_000, 1_000 + COOLDOWN_MS));
        assert!(is_blocking_allowed(1_000, 1_000 + COOLDOWN_MS + 1));
    }

    #[test]
    fn now_before_last_slow_does_not_panic_or_allow() {
        // クロックの巻き戻り等で now_ms < last_slow_at_ms でも saturating_sub で
        // パニックせず、cooldown 未経過として扱われる(= 発行を見送る)。
        assert!(!is_blocking_allowed(1_000_000, 0));
    }
}
