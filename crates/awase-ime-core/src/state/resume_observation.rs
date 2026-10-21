//! ADR-245 追補 2 R1: MS-IME 本体の戻り待ちの復元(`FromResume`)の書き込み直後に、A の開閉を読み直して記録する。
//!
//! 観測ストアは観測元ごとに 1 枠で、後から着いたものが勝つ。復元の書き込みの**前**に始まった ImmCrossProbe の読み
//! (フォーカス時の読み、最初の打鍵の FocusProbe が起動する読み)が R1 の記録より後に着くと、閉・英数で上書きして
//! Engine が止まる。それを防ぐため、R1 は最初の `Written` の時刻を floor として残し、floor 以前に始まった読みを棄却する。
//! 基準を Resume の起案時刻にしないのは、FocusProbe 起点の読みの `read_started` が起案より必ず後になるため
//! (docs/adr/245 追補 2 決定 2、Opus round 3 M-1')。
//!
//! 時刻は引数で受ける(`hook` に依存しない)。floor は解除しない: 後から始まる読みは必ず floor より後に
//! `read_started` を取るので、古い floor が後の正当な読みを棄却し続けることは無い。

use crate::state::TickMs;

/// 読みの開始時刻 `read_started` が、復元の書き込みの完了時刻 `floor` 以前か(= 書き込み前の状態を見うる読みか)。
///
/// floor が無ければ(R1 を予約しなかった)棄却しない。比較は ms のまま `<=`: R1 自身の記録は同じ ms になりうる
/// ので、R1 の記録はこの判定を通さない(共有メソッドを直接呼ぶ)。
#[must_use]
pub const fn read_predates_restore_write(read_started: TickMs, floor: Option<TickMs>) -> bool {
    match floor {
        Some(floor) => read_started.0 <= floor.0,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn t(ms: u64) -> TickMs {
        TickMs(ms)
    }

    /// floor が無ければ何も棄却しない(R1 を予約していない通常の ImmCrossProbe は今までどおり)。
    #[test]
    fn no_floor_never_rejects() {
        assert!(!read_predates_restore_write(t(0), None));
        assert!(!read_predates_restore_write(t(1_000), None));
    }

    /// floor より前に始まった読みは棄却する(書き込み前の状態を見うる)。
    #[test]
    fn read_before_floor_is_rejected() {
        assert!(read_predates_restore_write(t(99), Some(t(100))));
    }

    /// 同じ ms は棄却する(`<=`。書き込みの完了と同じ ms に始まった読みは、書き込み前を見うる)。
    #[test]
    fn read_in_same_ms_as_floor_is_rejected() {
        assert!(read_predates_restore_write(t(100), Some(t(100))));
    }

    /// floor より後に始まった読みは通す(書き込みの完了後に始まった正当な読み)。
    #[test]
    fn read_after_floor_is_accepted() {
        assert!(!read_predates_restore_write(t(101), Some(t(100))));
    }

    /// (ii) FocusProbe 起点の読みは、Resume の起案より後・書き込みの完了より前に始まる。
    /// 起案を基準にすると通ってしまい(Opus round 3 M-1')、書き込みの完了を基準にすると棄却される。
    #[test]
    fn focus_probe_read_between_issue_and_write_done_is_rejected() {
        let issued = t(100);
        let write_done = t(105);
        let focus_probe_read_started = t(101);
        assert!(
            !read_predates_restore_write(focus_probe_read_started, Some(issued)),
            "起案時刻を floor にすると、起案より後に始まる FocusProbe 起点の読みは通ってしまう"
        );
        assert!(read_predates_restore_write(
            focus_probe_read_started,
            Some(write_done)
        ));
    }

    /// floor は解除しなくてよい: 古い floor(100)のあとに別の Resume で始まった読み(1_000)は通る。
    #[test]
    fn stale_floor_does_not_reject_later_reads() {
        assert!(!read_predates_restore_write(t(1_000), Some(t(100))));
    }
}
