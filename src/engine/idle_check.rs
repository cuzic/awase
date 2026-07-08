//! idle 中の conv mode チェック実行可否を判定する純粋関数。

/// idle 中の conv mode チェック（`kp_stage_idle_conv_check`）を実行すべきか判定する。
///
/// 4 つのガード条件をまとめた純粋関数。Win32 API を呼ばないため Linux でもテスト可能。
///
/// # 引数
/// - `is_key_down`: KeyDown イベントかどうか（KeyUp はスキップ）
/// - `is_tsf_native`: フォーカスアプリが TsfNative プロファイルかどうか
/// - `in_flight_ms`: `output_in_flight_ms()` の値（`u64::MAX` = cold start）
/// - `explicit_age_ms`: `explicit_ime_action_age_ms()` の値（`u64::MAX` = 操作なし）
/// - `typing_idle_ms`: タイピング停止とみなす閾値（`TYPING_IDLE_MS`、通常 500ms）
/// - `explicit_suppress_ms`: 明示的 IME 操作後の抑制窓（`EXPLICIT_IME_SUPPRESS_MS`、通常 1500ms）
#[must_use]
pub const fn should_run_idle_conv_check(
    is_key_down: bool,
    is_tsf_native: bool,
    in_flight_ms: u64,
    explicit_age_ms: u64,
    typing_idle_ms: u64,
    explicit_suppress_ms: u64,
) -> bool {
    // ガード 1: KeyDown イベントのみ対象
    if !is_key_down {
        return false;
    }
    // ガード 2: TsfNative アプリのみ（WezTerm 等）
    if !is_tsf_native {
        return false;
    }
    // ガード 3: タイピング停止後のみ（in_flight_ms > typing_idle_ms）
    // u64::MAX（cold start）は typing_idle_ms より大きいため通過する
    if in_flight_ms <= typing_idle_ms {
        return false;
    }
    // ガード 4: 明示的 IME 操作直後はスキップ
    // Ctrl+変換/無変換 後に GJI probe が ROMAN ビットを確立する前に
    // check が走って belief を誤上書きするのを防ぐ
    if explicit_age_ms < explicit_suppress_ms {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE_MS: u64 = 500; // TYPING_IDLE_MS
    const SUPPRESS_MS: u64 = 1500; // EXPLICIT_IME_SUPPRESS_MS

    fn run_ok(in_flight: u64, explicit_age: u64) -> bool {
        should_run_idle_conv_check(true, true, in_flight, explicit_age, IDLE_MS, SUPPRESS_MS)
    }

    // ── ガード 1: KeyDown のみ ──
    #[test]
    fn guard1_key_up_skips() {
        assert!(!should_run_idle_conv_check(
            false,
            true,
            u64::MAX,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS
        ));
    }

    #[test]
    fn guard1_key_down_passes() {
        assert!(should_run_idle_conv_check(
            true,
            true,
            u64::MAX,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS
        ));
    }

    // ── ガード 2: TsfNative のみ ──
    #[test]
    fn guard2_non_tsf_native_skips() {
        assert!(!should_run_idle_conv_check(
            true,
            false,
            u64::MAX,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS
        ));
    }

    // ── ガード 3: タイピング停止後のみ ──
    #[test]
    fn guard3_typing_in_progress_skips() {
        // in_flight が IDLE_MS 以下 → タイピング中 → スキップ
        assert!(!run_ok(IDLE_MS, u64::MAX));
        assert!(!run_ok(0, u64::MAX));
        assert!(!run_ok(1, u64::MAX));
    }

    #[test]
    fn guard3_just_above_idle_threshold_passes() {
        // IDLE_MS + 1ms → 停止とみなす
        assert!(run_ok(IDLE_MS + 1, u64::MAX));
    }

    #[test]
    fn guard3_cold_start_passes() {
        // u64::MAX（cold start）は IDLE_MS より大きい → 通過
        assert!(run_ok(u64::MAX, u64::MAX));
    }

    // ── ガード 4: 明示的 IME 操作直後の抑制窓 ──
    #[test]
    fn guard4_within_suppress_window_skips() {
        assert!(!run_ok(u64::MAX, 0));
        assert!(!run_ok(u64::MAX, SUPPRESS_MS - 1));
    }

    #[test]
    fn guard4_at_suppress_boundary_passes() {
        // explicit_age == SUPPRESS_MS → `<` 条件が成立しない → 通過
        assert!(run_ok(u64::MAX, SUPPRESS_MS));
    }

    #[test]
    fn guard4_no_explicit_action_passes() {
        // u64::MAX（操作なし）→ SUPPRESS_MS 以上 → 通過
        assert!(run_ok(u64::MAX, u64::MAX));
    }

    // ── 全条件通過（通常の idle チェック）──
    #[test]
    fn all_guards_pass_for_normal_idle() {
        // 600ms 停止後、2000ms 前に IME 操作 → 通過
        assert!(run_ok(IDLE_MS + 100, SUPPRESS_MS + 500));
    }

    // ── 複合スキップ ──
    #[test]
    fn multiple_guards_fail_still_skips() {
        // KeyUp かつ typing 中 → どちらもスキップ条件
        assert!(!should_run_idle_conv_check(
            false,
            true,
            IDLE_MS,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS
        ));
    }
}
