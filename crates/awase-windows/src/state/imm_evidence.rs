//! IME読み取り失敗の分類と、`imm-learning`（`IMM32`が使えないことの学習）への証拠判定。
//!
//! 以前は `state/force_guard.rs`（force-on ガードの状態のためのモジュール）に同居しており、
//! `observer/ime_observer.rs` が `force_guard::read_miss_is_imm_evidence(..)` を呼ぶという、
//! 「observer が force guard を参照する」読んで意味の取れない依存を生んでいた
//! （design-patterns-review.md 提案3、A1）。移動のみで判定ロジックは変えていない。

/// 今回のOS読み取りが**新しい観測失敗を数えなかった**か（`consecutive_miss_count`が増えていない）。
///
/// 通過マークの追随（`ir_stage_observe`の`OsPoll`後、意図の破棄と60ms間隔の読み直し）を続けてよい条件。
/// 以前は`miss_after == miss_before`だったが、直前の読み取りが失敗（`ime_on=None`、カウント1）していて
/// 今回**成功**（カウントが0へリセット）すると等しくなくなり、追随が黙って止まっていた。すると
/// 最初の読み取りがfence（`KEY_EFFECT_SETTLE_MS`）内で無視された予測は、その後の打鍵中
/// （typing-idleガード）に訂正の機会を失い、約12秒Engineが固まる（実機cold、`removal-cold-2`）。
/// カウントが**減った**（成功で復帰した）ときも追随を続けるので`<=`とする。
#[must_use]
pub(crate) const fn poll_counted_no_new_miss(miss_before: u32, miss_after: u32) -> bool {
    miss_after <= miss_before
}

/// `SendMessageTimeoutW`が失敗（戻り値0）したとき、それが**時間切れ**か**即時の拒否**かを分類する純関数。
///
/// - `ERROR_TIMEOUT`(1460)、または宣言したタイムアウト以上かかっている: 時間切れ（遅い応答。負荷・忙しいIME）。
/// - それ以外（`ERROR_ACCESS_DENIED`=昇格プロセスへのUIPI拒否、即時の失敗）: 拒否（IMMが使えない証拠になりうる）。
///
/// 実測（CI、MS-IME本体、awase.logの`[ime-io]`）: 成功は 0〜20ms、時間切れは 50〜100ms（宣言50ms+スケジューリング）の
/// 二峰性で、`elapsed_us >= timeout_ms*1000`で時間切れと判別できる。
///
/// **`ERROR_ACCESS_DENIED`は先に判定して拒否として数える**（レビュー round3 A-NEW-2）: 高負荷でスケジューリング
/// 遅延が起き、本当に IMM 不可の証拠である `ERROR_ACCESS_DENIED`（昇格プロセスへの UIPI 拒否）が偶然50ms を
/// 跨ぐと、elapsed だけで判定した場合は「時間切れ」に落ちて証拠から外れてしまう。エラーコードが確実に取れる
/// （`send_ime_control_raw`が`ok.0==0`の直後に`GetLastError()`を読む）ので、経過時間より優先する。
#[must_use]
pub(crate) const fn send_failure_is_timeout(
    last_error: u32,
    elapsed_us: u64,
    timeout_ms: u32,
) -> bool {
    const ERROR_TIMEOUT: u32 = 1460;
    const ERROR_ACCESS_DENIED: u32 = 5;
    if last_error == ERROR_ACCESS_DENIED {
        return false;
    }
    last_error == ERROR_TIMEOUT || elapsed_us >= (timeout_ms as u64) * 1000
}

/// IME状態の読み取りの空振り（`ime_on`が`None`）を、`imm-learning`の「IMMが使えない」証拠（miss）に数えるか。
///
/// **個別の`SendMessageTimeout`の時間切れ**（遅い応答。負荷・忙しいIME・CIの遅いランナー、(b)）は証拠ではない
/// （判定保留=数えない）。
/// **即時の拒否**（`ERROR_ACCESS_DENIED`、IME窓なし=`ImmGetDefaultIMEWnd`=NULL、即時の失敗）と、
/// **読み取り全体のワーカータイムアウト（300ms、(d)）**は従来どおり数える（後者は本当に応答しない窓＝hung を
/// 降格させる唯一の経路で、外すと探索が止まらない）。
/// `IME_DETECT_MISS_THRESHOLD`（連続3回で`Unavailable`を学習）の値は変えない（tuning-constants: 盲目的な引き上げをしない）。
#[must_use]
pub(crate) const fn read_miss_is_imm_evidence(probe_timed_out: bool) -> bool {
    !probe_timed_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_counted_no_new_miss_continues_follow_after_recovery() {
        assert!(poll_counted_no_new_miss(0, 0), "失敗なし");
        assert!(
            poll_counted_no_new_miss(1, 0),
            "直前の失敗から成功で復帰(リセット)しても追随は続ける"
        );
        assert!(poll_counted_no_new_miss(2, 1));
        assert!(
            !poll_counted_no_new_miss(0, 1),
            "今回新しく失敗したら追随しない"
        );
        assert!(!poll_counted_no_new_miss(1, 2));
    }

    /// `imm-learning`の入口の意味: 時間切れは「IMMが使えない」証拠に数えない。本当に使えないパターン
    /// （即時の拒否・IME窓なしが連続）は従来どおり閾値で降格する。
    #[test]
    fn timeouts_are_not_imm_evidence_but_immediate_refusals_are() {
        // 分類: 実測(CI、MS-IME本体)の二峰性。成功は0〜20ms、時間切れは50〜100ms。
        assert!(send_failure_is_timeout(1460, 51_000, 50));
        assert!(
            send_failure_is_timeout(0, 50_953, 50),
            "GetLastErrorが0でも50ms以上なら時間切れ"
        );
        assert!(
            send_failure_is_timeout(1460, 300, 50),
            "ERROR_TIMEOUTなら短くても時間切れ"
        );
        assert!(
            !send_failure_is_timeout(5, 120, 50),
            "ERROR_ACCESS_DENIED(昇格プロセスのUIPI拒否)は時間切れではない"
        );
        assert!(
            !send_failure_is_timeout(0, 800, 50),
            "即時の失敗は時間切れではない"
        );
        // レビュー round3 A-NEW-2: 高負荷でスケジューリング遅延が起き、ERROR_ACCESS_DENIED が50ms(宣言timeout)を
        // 跨いでも、elapsed だけでなくエラーコードを先に見て「時間切れではない(=拒否として数える)」にする。
        assert!(
            !send_failure_is_timeout(5, 60_000, 50),
            "ERROR_ACCESS_DENIEDは、高負荷でelapsedが50msを跨いでも時間切れにしない"
        );
        // 数え方: 3連続のシミュレーション。時間切れ3連続は数えず(降格しない)、即時の拒否3連続は数える(降格する)。
        let count = |seq: &[bool]| -> u32 {
            // seq の各要素 = そのreadが時間切れか。時間切れは数えない(カウントも変えない)。
            seq.iter()
                .filter(|&&timed_out| read_miss_is_imm_evidence(timed_out))
                .count() as u32
        };
        assert_eq!(
            count(&[true, true, true]),
            0,
            "時間切れの連続は降格の材料にならない(CI MS-IME本体)"
        );
        assert_eq!(
            count(&[false, false, false]),
            3,
            "即時の拒否が3連続なら閾値(3)に届く(本当にIMM不可のアプリ)"
        );
        assert_eq!(
            count(&[true, false, true, false]),
            2,
            "混在は即時の拒否だけを数える"
        );
    }
}
