#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! IME 状態の観測 — `read_ime_state_full()` を呼び出して観測スナップショットを返す。
//!
//! ## 設計方針
//!
//! observer は観測値を `ImeUpdate` として返すのみで、`Preconditions` を直接
//! 変更しない。状態への反映は `PlatformState::apply_ime_update()` に一元化。
//!
//! ## 更新ポリシー
//!
//! `ImeSnapshot` の 3 フィールドはすべて `Option<bool>` で 3 値意味論を持つ:
//! - `Some(v)` = 検出成功 → 呼び出し元が `Preconditions` を更新する
//! - `None`    = 不明（タイムアウト等） → **前回キャッシュ値を維持する**
//!
//! `None` を「偽」として扱ってはならない。

use awase::engine::InputModeState;

/// Observer が返す単一観測 (値 + タイムスタンプ)。
#[derive(Debug, Clone, Copy)]
pub struct ImeObs {
    pub value: bool,
    pub ms: u64,
}

/// `classify_ime_snapshot()` が返す状態更新命令。
///
/// 副作用なし・純粋変換の結果を表す。
/// 呼び出し元（`PlatformState::apply_ime_update()`）が状態に反映する。
#[derive(Debug)]
pub struct ImeUpdate {
    /// 検出された is_japanese_ime（`Some` のときのみ更新すべき）
    pub is_japanese_ime: Option<bool>,
    /// `observer_poll` スロットに書くべき値（`Some` のときのみ書く）
    pub observer_poll: Option<ImeObs>,
    /// miss_count を 1 インクリメントすべきか
    pub increment_miss_count: bool,
    /// `force_on_broken_app_bootstrap` フラグをリセットすべきか（検出成功時）
    pub clear_force_on_broken_app_bootstrap: bool,
    /// `force_on_panic_reset` フラグと miss_count をリセットすべきか（検出成功時）
    pub clear_force_on_panic_reset: bool,
    /// `input_mode` に適用すべき新しい値（`Some` のときのみ更新すべき）
    pub new_input_mode: Option<InputModeState>,
    /// `prev_conversion_mode` に書くべき値（`Some` のときのみ更新すべき）
    pub new_prev_conversion_mode: Option<u32>,
}

/// observer_poll/miss_count/force_on_guard の更新方針
struct PollOutcome {
    observer_poll: Option<ImeObs>,
    increment_miss_count: bool,
    clear_force_on_broken_app_bootstrap: bool,
    clear_force_on_panic_reset: bool,
}

impl crate::ime::ImeSnapshot {
    fn classify_poll_outcome(
        &self,
        now_ms: u64,
        current_ime_on: bool,
        guard_active: bool,
    ) -> PollOutcome {
        let known_not_japanese = self.is_japanese_ime == Some(false);
        if known_not_japanese {
            PollOutcome {
                observer_poll: Some(ImeObs {
                    value: false,
                    ms: now_ms,
                }),
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: true,
                clear_force_on_panic_reset: true,
            }
        } else if let Some(on) = self.ime_on {
            PollOutcome {
                observer_poll: Some(ImeObs {
                    value: on,
                    ms: now_ms,
                }),
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: true,
                clear_force_on_panic_reset: true,
            }
        } else if self.is_tsf_native {
            tracing::debug!(
                "IME detection skipped (TSF-native window), preserving ime_on={current_ime_on}"
            );
            PollOutcome {
                observer_poll: None,
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: false,
                clear_force_on_panic_reset: false,
            }
        } else if !crate::state::force_guard::read_miss_is_imm_evidence(self.probe_timed_out) {
            // 時間切れ（遅い応答）は「IMMが使えない」証拠ではない。missに数えず、belief を保つ
            // （`imm-learning`が3回連続で`Unavailable`を学習してしまう。BUG-158追補）。
            tracing::debug!(
                "IME detection timed out (slow response, not IMM-unavailable evidence), \
                 preserving ime_on={current_ime_on}"
            );
            PollOutcome {
                observer_poll: None,
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: false,
                clear_force_on_panic_reset: false,
            }
        } else if guard_active {
            tracing::debug!(
                "IME detection failed but force_on_guard active, preserving ime_on={current_ime_on}"
            );
            PollOutcome {
                observer_poll: None,
                increment_miss_count: false,
                clear_force_on_broken_app_bootstrap: false,
                clear_force_on_panic_reset: false,
            }
        } else {
            PollOutcome {
                observer_poll: None,
                increment_miss_count: true,
                clear_force_on_broken_app_bootstrap: false,
                clear_force_on_panic_reset: false,
            }
        }
    }

    fn input_mode_from_romaji_flag(
        &self,
        current_input_mode: InputModeState,
    ) -> Option<InputModeState> {
        let romaji = self.is_romaji?;
        let prev = current_input_mode.is_romaji_capable();
        if prev != romaji {
            tracing::info!(
                "IME input method changed: {} → {} (focused_class={:?})",
                if prev { "romaji" } else { "kana" },
                if romaji { "romaji" } else { "kana" },
                self.focused_class,
            );
        }
        Some(if romaji {
            InputModeState::ObservedRomaji
        } else {
            InputModeState::ObservedKana
        })
    }

    fn input_mode_from_conversion(
        &self,
        current_prev_conversion_mode: Option<u32>,
        current_input_mode: InputModeState,
    ) -> Option<InputModeState> {
        let curr_conv = self.conversion_mode?;
        let prev_conv = current_prev_conversion_mode?;
        let result = awase::engine::ConvMode::from_u32(curr_conv).classify_transition(
            awase::engine::ConvMode::from_u32(prev_conv),
            current_input_mode,
        );
        if let Some(new_mode) = result {
            tracing::info!(
                "IME input method changed: conv=0x{prev_conv:08X}→0x{curr_conv:08X}, belief {current_input_mode:?}→{new_mode:?}"
            );
        }
        result
    }
}

/// `ImeSnapshot` と現在の `Preconditions` の読み取り専用ビューから更新命令を計算する。
///
/// `Preconditions` への書き込みを一切行わない純粋関数。
/// `poll_and_classify_ime()` と `classify_fetched_snapshot()` の共通ロジックを集約。
#[must_use]
pub fn classify_ime_snapshot(
    snap: &crate::ime::ImeSnapshot,
    now_ms: u64,
    // Preconditions の読み取り専用フィールド
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
    // 呼び出し元が `!focus::class_names::is_own_ui_window(snap.focused_class, process_name)`
    // で算出する。false のとき input_mode 軸の観測を一切採用しない（BUG-106追補3・4:
    // awase自身のトレイ/設定画面へ一瞬フォーカスが移った際の観測を、ユーザーが
    // 編集中のアプリの入力方式としてbeliefに書き込んでしまっていた）。
    trust_input_mode: bool,
) -> ImeUpdate {
    let guard_active = current_force_on_guard_active;
    let poll = snap.classify_poll_outcome(now_ms, current_ime_on, guard_active);

    let new_input_mode = if !trust_input_mode {
        // 採用しない: 前回値を維持する（ImeSnapshot の doc が言う「None は偽ではなく
        // 不明」のまま扱う）。new_prev_conversion_mode も下で揃えて None にすること
        // （でないと次の信頼できる poll で偽の conv 遷移を作ってしまう）。
        None
    } else if guard_active && snap.is_romaji.is_none() {
        None
    } else if awase::engine::ConvMode::is_eisu_evidence(snap.ime_on, snap.conversion_mode)
        == Some(true)
    {
        // 英数モードは romaji フラグより優先して ObservedEisu を返す。
        // input_mode_from_romaji_flag は romaji=false を ObservedKana と判定するため
        // 英数モードを誤って ObservedKana にしてしまう問題をここで遮断する。
        (!matches!(current_input_mode, InputModeState::ObservedEisu))
            .then_some(InputModeState::ObservedEisu)
    } else {
        snap.input_mode_from_romaji_flag(current_input_mode)
            .or_else(|| {
                snap.input_mode_from_conversion(current_prev_conversion_mode, current_input_mode)
            })
            .or_else(|| {
                // ObservedEisu が stale の場合の回復。
                // GJI 等 ROMAN bit 不使用 IME では英数→ひらがな切替で conv が変化しても
                // ROMAN bit は両方 false のまま → classify_transition が None を返し
                // belief が ObservedEisu に固まる。
                // TsfNative は conversion_mode=None のため is_some_and が false → 不適用。
                if matches!(current_input_mode, InputModeState::ObservedEisu)
                    && snap
                        .conversion_mode
                        .is_some_and(|c| !awase::engine::ConvMode::from_u32(c).is_eisu())
                {
                    let conv = snap.conversion_mode.unwrap_or(0);
                    tracing::info!(
                        "IME input method changed: ObservedEisu → AssumedRomaji \
                         (conv=0x{conv:08X}, GJI/ImmCross stale recovery)"
                    );
                    Some(InputModeState::AssumedRomaji {
                        reason: awase::engine::AssumedReason::AppKindExcluded,
                    })
                } else {
                    None
                }
            })
    };

    tracing::debug!(
        "IME snapshot: japanese={:?} ime_on={:?} romaji={:?} conv={:?} guard={}",
        snap.is_japanese_ime,
        snap.ime_on,
        snap.is_romaji,
        snap.conversion_mode.map(|v| format!("0x{v:08X}")),
        guard_active,
    );

    ImeUpdate {
        is_japanese_ime: snap.is_japanese_ime,
        observer_poll: poll.observer_poll,
        increment_miss_count: poll.increment_miss_count,
        clear_force_on_broken_app_bootstrap: poll.clear_force_on_broken_app_bootstrap,
        clear_force_on_panic_reset: poll.clear_force_on_panic_reset,
        new_input_mode,
        new_prev_conversion_mode: if trust_input_mode {
            snap.conversion_mode
        } else {
            None
        },
    }
}

/// Win32 API を使って IME 状態を観測し、`ImeUpdate` を返す。
///
/// `Preconditions` を直接変更しない。呼び出し元が
/// `PlatformState::apply_ime_update()` で状態に反映すること。
///
/// `focus_process_name` は `FocusTracker::process_name()`（小文字）を渡すこと。
/// 観測対象ウィンドウが awase 自身のUI（トレイ／設定画面）かどうかの判定
/// （`focus::class_names::is_own_ui_window`）に使う（BUG-106追補3・4）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn poll_and_classify_ime(
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
    focus_process_name: &str,
) -> ImeUpdate {
    // read_ime_state_full は複数のブロッキング IMM32 API を連鎖呼び出しするため、
    // ワーカースレッドでタイムアウト付き実行する（メッセージループハング防止）。
    let snap = crate::ime::read_ime_state_full_with_timeout(std::time::Duration::from_millis(300));
    let now_ms = crate::hook::current_tick_ms();
    let trust_input_mode = !crate::focus::class_names::is_own_ui_window(
        snap.focused_class.as_deref().unwrap_or(""),
        focus_process_name,
    );
    classify_ime_snapshot(
        &snap,
        now_ms,
        current_ime_on,
        current_force_on_guard_active,
        current_input_mode,
        current_prev_conversion_mode,
        trust_input_mode,
    )
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::ime::ImeSnapshot;
    use awase::engine::InputModeState;

    fn default_snap() -> ImeSnapshot {
        ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: None,
            is_romaji: None,
            conversion_mode: None,
            is_tsf_native: false,
            focused_class: None,
            probe_timed_out: false,
        }
    }

    /// BUG-158追補: `ime_on`が読めなかった理由が時間切れなら missに数えない（`imm-learning`が誤降格しない）。
    /// 即時の拒否（時間切れでない）は従来どおり数える（本当にIMM不可のアプリを降格できる）。
    #[test]
    fn timed_out_read_does_not_increment_miss_count_but_refusal_does() {
        let mut snap = default_snap();
        snap.probe_timed_out = true;
        let out = snap.classify_poll_outcome(0, true, false);
        assert!(!out.increment_miss_count, "時間切れはmissに数えない");
        assert!(
            out.observer_poll.is_none(),
            "beliefを保つ（観測は書かない）"
        );
        snap.probe_timed_out = false;
        let out = snap.classify_poll_outcome(0, true, false);
        assert!(out.increment_miss_count, "即時の拒否は従来どおり数える");
    }

    /// ケース 1: 日本語 IME + IME ON → observer_poll に Some(true) が記録される
    #[test]
    fn classify_returns_observer_poll_true_for_japanese_ime_on() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: Some(true),
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            false, // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
            true, // trust_input_mode
        );
        assert!(update.observer_poll.is_some());
        assert!(update.observer_poll.unwrap().value);
    }

    /// ケース 2: 日本語 IME + IME OFF → observer_poll に Some(false)
    #[test]
    fn classify_returns_observer_poll_false_for_japanese_ime_off() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: Some(false),
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
            true, // trust_input_mode
        );
        assert!(update.observer_poll.is_some());
        assert!(!update.observer_poll.unwrap().value);
    }

    /// ケース 3: 非日本語 IME → observer_poll に Some(false)（non-Japanese → IME 不活性）
    #[test]
    fn classify_returns_observer_poll_false_for_non_japanese_ime() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(false),
            ime_on: None,
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
            true, // trust_input_mode
        );
        // known_not_japanese → (Some(false), false, true, true)
        assert!(update.observer_poll.is_some());
        assert!(!update.observer_poll.unwrap().value);
        assert!(!update.increment_miss_count);
        assert!(update.clear_force_on_broken_app_bootstrap);
        assert!(update.clear_force_on_panic_reset);
    }

    /// ケース 4: IME 検出失敗（is_japanese_ime: None, not tsf, no guard) → miss_count インクリメント
    #[test]
    fn classify_increments_miss_count_on_detection_failure() {
        let snap = ImeSnapshot {
            is_japanese_ime: None,
            ime_on: None,
            is_romaji: None,
            conversion_mode: None,
            is_tsf_native: false,
            focused_class: None,
            probe_timed_out: false,
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            false, // current_ime_on
            false, // current_force_on_guard_active（ガードなし）
            InputModeState::Unknown,
            None,
            true, // trust_input_mode
        );
        assert!(update.increment_miss_count);
        assert!(update.observer_poll.is_none());
    }

    /// ケース 5: force_on_guard アクティブ時 → observer_poll が None、miss_count も増えない
    #[test]
    fn classify_skips_observer_poll_when_force_on_guard_active() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: None, // 検出失敗
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true, // current_ime_on
            true, // current_force_on_guard_active = true
            InputModeState::Unknown,
            None,
            true, // trust_input_mode
        );
        assert!(update.observer_poll.is_none());
        assert!(!update.increment_miss_count);
        assert!(!update.clear_force_on_broken_app_bootstrap);
        assert!(!update.clear_force_on_panic_reset);
    }

    /// ケース 6: TSF ネイティブウィンドウ → observer_poll が None、miss_count も増えない
    #[test]
    fn classify_skips_observer_poll_for_tsf_native_window() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: None, // TSF なので取得不能
            is_tsf_native: true,
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,  // current_ime_on
            false, // current_force_on_guard_active
            InputModeState::Unknown,
            None,
            true, // trust_input_mode
        );
        assert!(update.observer_poll.is_none());
        assert!(!update.increment_miss_count);
        assert!(!update.clear_force_on_broken_app_bootstrap);
        assert!(!update.clear_force_on_panic_reset);
    }

    /// ケース 7（BUG-106追補4）: trust_input_mode=false → is_romaji が
    /// romaji→kana の変化を示していても input_mode/prev_conversion_mode の
    /// どちらも更新しない(前回値を維持)。awase自身のトレイ/設定画面から
    /// 読んだ観測を、ユーザーの入力方式としてbeliefに採用しないための回帰テスト。
    #[test]
    fn classify_ignores_input_mode_and_conv_when_not_trusted() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: Some(true),
            is_romaji: Some(false), // romaji → kana を示す観測
            conversion_mode: Some(0x0000_0000),
            focused_class: Some("awase_tray_window".to_string()),
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true, // current_ime_on
            false,
            InputModeState::ObservedRomaji,
            Some(0x0000_0009), // 直前に信頼できたconv値
            false,             // trust_input_mode = false（信頼しない）
        );
        assert_eq!(update.new_input_mode, None);
        assert_eq!(update.new_prev_conversion_mode, None);
        // open軸（observer_poll）は今回のスコープ外で従来どおり動く
        assert!(update.observer_poll.is_some());
        assert!(update.observer_poll.unwrap().value);
    }

    /// ケース 8（BUG-106追補4）: trust_input_mode=true（既定）なら従来どおり
    /// romaji→kana の変化を input_mode として採用する（回帰確認）。
    #[test]
    fn classify_adopts_input_mode_when_trusted() {
        let snap = ImeSnapshot {
            is_japanese_ime: Some(true),
            ime_on: Some(true),
            is_romaji: Some(false),
            focused_class: Some("Chrome_WidgetWin_1".to_string()),
            ..default_snap()
        };
        let update = classify_ime_snapshot(
            &snap,
            1000,
            true,
            false,
            InputModeState::ObservedRomaji,
            None,
            true, // trust_input_mode
        );
        assert_eq!(update.new_input_mode, Some(InputModeState::ObservedKana));
    }
}

/// IME スナップショットを `ImeUpdate` に変換する（純粋 sync）。
///
/// `poll_and_classify_ime()` から blocking fetch 部分を分離したもの。async drain 後に with_app 内で呼ぶ。
/// `Preconditions` を直接変更しない。
#[must_use]
///
/// `focus_process_name` は `poll_and_classify_ime` と同じ（BUG-106追補3・4）。
pub fn classify_fetched_snapshot(
    snap: &crate::ime::ImeSnapshot,
    now_ms: u64,
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
    focus_process_name: &str,
) -> ImeUpdate {
    let trust_input_mode = !crate::focus::class_names::is_own_ui_window(
        snap.focused_class.as_deref().unwrap_or(""),
        focus_process_name,
    );
    classify_ime_snapshot(
        snap,
        now_ms,
        current_ime_on,
        current_force_on_guard_active,
        current_input_mode,
        current_prev_conversion_mode,
        trust_input_mode,
    )
}
