//! フォーカス復帰直後の resync（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）の純粋判定。
//!
//! Win32 API を一切呼ばないため Linux でも `cargo test -p awase-windows` で検証できる。
//! 実際の配線（arm/trigger/drain）は `focus_resync.rs`・`runtime/focus_tracking.rs`・
//! `app/mod.rs`・`runtime/key_pipeline.rs`・`runtime/message_handlers.rs` を参照。

/// フォーカス変更時に resync を armed 状態にしてよいか。
///
/// - `is_tsf_native`: フォーカスアプリが TsfNative プロファイルかどうか（GJI/ImmCross は
///   既存の focus-time プローブ（`kp_stage_focus_probe`）や 500ms ポーリングで別途カバー
///   されるため対象外）。
/// - `is_japanese_ime`: アクティブ IME が日本語 IME かどうか。
/// - `send_health_ok`: `send_health::blocking_allowed()`。false のとき arm しない
///   （`get_ime_conversion_mode_raw_timeout` は BUG-34 のとおり数秒ブロックしうるため、
///   ブレーカが落ちている間に新規 resync を armed にしても安全に完了できない）。
/// - `conv_read_in_flight`: 既存の idle-conv-check の conv 読み取りが in-flight 中か。
///   多重 spawn を避けるため in-flight 中は arm しない。
#[must_use]
pub const fn should_arm_focus_resync(
    is_tsf_native: bool,
    is_japanese_ime: bool,
    send_health_ok: bool,
    conv_read_in_flight: bool,
) -> bool {
    is_tsf_native && is_japanese_ime && send_health_ok && !conv_read_in_flight
}

/// resync のハード期限に到達したか。
///
/// `now_ms < armed_at_ms`（クロックの巻き戻り・オーバーフロー相当）は安全側に倒し、
/// 期限到達済み（`true`）として扱う——defer されたキーを無期限に止めないため。
#[must_use]
pub const fn resync_deadline_elapsed(armed_at_ms: u64, now_ms: u64, deadline_ms: u64) -> bool {
    match now_ms.checked_sub(armed_at_ms) {
        Some(elapsed) => elapsed >= deadline_ms,
        None => true,
    }
}

/// resync gate を開けるとき、drain queue の post を自分で行うべきか。
///
/// `OUTPUT_GATE` が active なら post しない——`OutputActiveGuard::drop` が
/// drain を実行する RAII 契約に委ねる（「最後に閉じたゲートが drain する」）。
/// これを守らないと、resync 完了と awase 自身の出力（force-ON 等）が交錯した際に
/// `OUTPUT_GATE` active 中に defer 済みキーが replay され、BUG-02/BUG-70 系の
/// リテラル漏れ経路を新たに開いてしまう。
#[must_use]
pub const fn should_post_drain(output_gate_active: bool) -> bool {
    !output_gate_active
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── should_arm_focus_resync: 4引数全数(16通り) ──

    #[test]
    fn should_arm_focus_resync_all_16_combinations() {
        for is_tsf_native in [false, true] {
            for is_japanese_ime in [false, true] {
                for send_health_ok in [false, true] {
                    for conv_read_in_flight in [false, true] {
                        let expected = is_tsf_native
                            && is_japanese_ime
                            && send_health_ok
                            && !conv_read_in_flight;
                        assert_eq!(
                            should_arm_focus_resync(
                                is_tsf_native,
                                is_japanese_ime,
                                send_health_ok,
                                conv_read_in_flight,
                            ),
                            expected,
                            "is_tsf_native={is_tsf_native} is_japanese_ime={is_japanese_ime} \
                             send_health_ok={send_health_ok} conv_read_in_flight={conv_read_in_flight}"
                        );
                    }
                }
            }
        }
    }

    // ── resync_deadline_elapsed ──

    #[test]
    fn resync_deadline_elapsed_boundary() {
        assert!(!resync_deadline_elapsed(0, 99, 100));
        assert!(resync_deadline_elapsed(0, 100, 100));
        assert!(resync_deadline_elapsed(0, 101, 100));
    }

    #[test]
    fn resync_deadline_elapsed_clock_rewind_is_treated_as_elapsed() {
        // now_ms < armed_at_ms（巻き戻り相当）は安全側に倒し true を返す。
        assert!(resync_deadline_elapsed(1_000, 500, 100));
    }

    #[test]
    fn resync_deadline_elapsed_zero_deadline_always_elapsed() {
        assert!(resync_deadline_elapsed(0, 0, 0));
    }

    // ── should_post_drain: OUTPUT_GATE への委譲 ──

    #[test]
    fn should_post_drain_defers_to_output_gate() {
        assert!(!should_post_drain(true));
        assert!(should_post_drain(false));
    }
}
