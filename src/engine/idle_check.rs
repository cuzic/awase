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
/// - `is_first_key_after_focus`: フォーカス復帰後の resync 対象キー（`RawKeyEvent::
///   starts_focus_resync()`）かどうか。true のときガード3（タイピング停止判定）
///   のみバイパスする（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`: フォーカス復帰直後は
///   `output_in_flight_ms` が意味を持たないため）。ガード1・2・4・5は
///   `is_first_key_after_focus` でも必ず効く——特にガード4（明示的 IME 操作直後の
///   抑制窓）を緩めると、フォーカス復帰直後にユーザーが意図的に IME 操作した
///   直後の conv 誤読で belief を押し付ける経路が生まれる。
/// - `is_ime_mode_key`: この打鍵自身が IME のモードを動かしうるキーか
///   （`ImeRelevance::is_ime_mode_key`、`is_ime_mode_key_for_ime()`）。
///   true なら conv 読み取りをスキップする（ガード5、BUG-113残置課題）。
///   ガード4の doc が挙げる「Ctrl+変換/無変換」は`explicit_age_ms`経由で
///   守られているのに、素の 変換/無変換（`note_explicit_ime_action`を呼ばない
///   passthroughのモードキー）が無防備だった穴を塞ぐ。GJI既定キーマップでは
///   無変換=直接入力/変換=ひらがなであり、その打鍵直後のconvはGJI側で遷移中
///   のため、この読み取りとそれに続くdrift correctionのactuation（SendInput）
///   が時間的に近接し、GJIのTSF composition追跡を乱して「@」を生む
///   （実機A/Bで確定済みの独立した十分条件、docs/known-bugs.md BUG-113参照）。
#[must_use]
#[allow(clippy::fn_params_excessive_bools)] // 各ガード条件を独立の引数として明示（enum化は呼び出し元の可読性を下げる）
pub const fn should_run_idle_conv_check(
    is_key_down: bool,
    is_tsf_native: bool,
    in_flight_ms: u64,
    explicit_age_ms: u64,
    typing_idle_ms: u64,
    explicit_suppress_ms: u64,
    is_first_key_after_focus: bool,
    is_ime_mode_key: bool,
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
    // u64::MAX（cold start）は typing_idle_ms より大きいため通過する。
    // フォーカス復帰直後の resync 対象キーのみこのガードをバイパスする。
    if in_flight_ms <= typing_idle_ms && !is_first_key_after_focus {
        return false;
    }
    // ガード 4: 明示的 IME 操作直後はスキップ
    // Ctrl+変換/無変換 後に GJI probe が ROMAN ビットを確立する前に
    // check が走って belief を誤上書きするのを防ぐ
    if explicit_age_ms < explicit_suppress_ms {
        return false;
    }
    // ガード 5: この打鍵自身が IME モードを動かすキー → conv は遷移中で
    // 信用できない。加えて、この打鍵で probe を発行しないことが BUG-113
    // 「読み取りと書き込みの時間的近接」の最大のトリガーを根元から消す。
    if is_ime_mode_key {
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
        should_run_idle_conv_check(
            true,
            true,
            in_flight,
            explicit_age,
            IDLE_MS,
            SUPPRESS_MS,
            false,
            false,
        )
    }

    fn run_ok_first_key(in_flight: u64, explicit_age: u64) -> bool {
        should_run_idle_conv_check(
            true,
            true,
            in_flight,
            explicit_age,
            IDLE_MS,
            SUPPRESS_MS,
            true,
            false,
        )
    }

    fn run_ok_mode_key(is_ime_mode_key: bool) -> bool {
        should_run_idle_conv_check(
            true,
            true,
            u64::MAX,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS,
            false,
            is_ime_mode_key,
        )
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
            SUPPRESS_MS,
            false,
            false
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
            SUPPRESS_MS,
            false,
            false
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
            SUPPRESS_MS,
            false,
            false
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
            SUPPRESS_MS,
            false,
            false
        ));
    }

    // ── is_first_key_after_focus: ガード3のみバイパス ──

    #[test]
    fn first_key_after_focus_bypasses_typing_idle_guard() {
        // in_flight=0（タイピング中相当）でも first key なら通過する
        assert!(run_ok_first_key(0, u64::MAX));
        assert!(run_ok_first_key(IDLE_MS, u64::MAX));
    }

    #[test]
    fn first_key_after_focus_still_respects_explicit_suppress() {
        // ガード4（明示的 IME 操作直後の抑制窓）は first key でも必ず効く
        assert!(!run_ok_first_key(0, 0));
        assert!(!run_ok_first_key(u64::MAX, SUPPRESS_MS - 1));
        assert!(run_ok_first_key(0, SUPPRESS_MS));
    }

    #[test]
    fn first_key_after_focus_still_requires_key_down() {
        assert!(!should_run_idle_conv_check(
            false,
            true,
            0,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS,
            true,
            false,
        ));
    }

    #[test]
    fn first_key_after_focus_still_requires_tsf_native() {
        assert!(!should_run_idle_conv_check(
            true,
            false,
            0,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS,
            true,
            false,
        ));
    }

    #[test]
    fn is_first_key_after_focus_false_preserves_legacy_behavior() {
        // 既存の全ケースが is_first_key_after_focus=false で従来どおりであること
        assert!(!run_ok(IDLE_MS, u64::MAX));
        assert!(run_ok(IDLE_MS + 1, u64::MAX));
        assert!(run_ok(u64::MAX, u64::MAX));
        assert!(!run_ok(u64::MAX, 0));
    }

    // ── ガード 5: IME モードキー自身の打鍵はスキップ（BUG-113残置課題）──

    #[test]
    fn guard5_ime_mode_key_skips() {
        // ガード1〜4を全通過する条件でも is_ime_mode_key=true なら false
        assert!(!run_ok_mode_key(true));
    }

    #[test]
    fn guard5_is_not_bypassed_by_first_key_after_focus() {
        assert!(!should_run_idle_conv_check(
            true,
            true,
            0,
            u64::MAX,
            IDLE_MS,
            SUPPRESS_MS,
            true,
            true,
        ));
    }

    #[test]
    fn guard5_false_preserves_legacy_behavior() {
        // 既存の代表4ケースが is_ime_mode_key=false で従来どおりであること
        assert!(run_ok_mode_key(false));
        assert!(!run_ok(IDLE_MS, u64::MAX));
        assert!(run_ok(IDLE_MS + 1, u64::MAX));
        assert!(run_ok(u64::MAX, u64::MAX));
        assert!(!run_ok(u64::MAX, 0));
    }
}
