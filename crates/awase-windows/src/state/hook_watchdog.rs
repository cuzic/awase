//! フックwatchdog（issue #165、hook_starved）の自己修復トリガー判定の純粋関数。
//!
//! Win32 API を一切呼ばないため Linux でも `cargo test -p awase-windows` で検証できる。
//! 実際の配線（`GetLastInputInfo`/昇格判定/セッションロック/secure desktop判定/
//! relayプロセス検出/設定読み込み）は `runtime/message_handlers.rs`
//! （`TIMER_HOOK_WATCHDOG`分岐）・`runtime/mod.rs`
//! （`Runtime::reinstall_keyboard_hook_for_watchdog`）を参照。
//!
//! opus-adversarial-consult round1（2026-09-28、fix/hook-self-heal-v2起票時）の
//! ブロッカー/主要指摘（F1〜F3）への対応:
//! - F1: マウスのみの通常利用でもトリガー条件が成立しうるため、hook_starved episode
//!   （＝一度検知してから hook が実際に生き返るまでの連続区間）ごとに1回だけ
//!   再インストールを試みる（`already_attempted_this_episode`）ラッチに加え、
//!   長期スパンのレート上限（`THRASH_WINDOW_MS`/`THRASH_LIMIT`）も課す。
//! - F2: 昇格ウィンドウへのタイピング・セッションロック/secure desktop 中は
//!   何をしても効果が無いか、実害の方が大きいためスキップする。
//! - F3: 入力中継/リマップソフト（Mouse Without Borders・mstsc.exe 等）が
//!   フォアグラウンドの間は、意図的なキーボード奪取を壊しかねないためスキップし、
//!   `[diagnostics] hook_self_heal` でのビルド無しキルスイッチも用意する。

/// hook_starved 検知 tick で watchdog が取るべきアクション。
///
/// `Reinstall` 以外は全て「自己修復を行わない」を意味し、バリアントの違いは
/// ログでスキップ理由を区別するためだけに存在する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookWatchdogAction {
    /// フックを再インストールする。
    Reinstall,
    /// `[diagnostics] hook_self_heal = false`（キルスイッチ）。
    SkipDisabled,
    /// フォアグラウンドが昇格プロセスで自分（awase）は非昇格（UIPI）。
    SkipElevatedForeground,
    /// セッションロック中（Win+L 等）。
    SkipSessionLocked,
    /// secure desktop（UAC 昇格プロンプト等）がアクティブ。
    SkipSecureDesktop,
    /// `disable_apps`/`input_relay_apps` 一致、または既知の入力中継/リマップ
    /// ソフトがフォアグラウンド。
    SkipRelayOrRemapForeground,
    /// 現在の hook_starved episode で既に1回再インストールを試行済み。
    SkipAlreadyAttemptedThisEpisode,
    /// 直近 `THRASH_WINDOW_MS` 以内の再インストール回数が `THRASH_LIMIT` に到達。
    SkipThrashLimit,
}

/// レート上限のウィンドウ幅（ミリ秒）。1時間。
///
/// `tuning.rs`（実測 ms に基づくタイミング定数）の対象外
/// （[tuning-constants](../../../../.claude/rules/tuning-constants.md) 参照、あちらは
/// 「probe が実際に何 ms 必要か」等の実測値を扱う。こちらは IME/hook 実機タイミングの
/// モデル化ではなく、異常な連続再インストールを止める安全弁の閾値であり、実測根拠は
/// 存在しない・不要）。値は「1時間に数回を超える再インストールは環境側の異常」という
/// 保守的な目安として選んだもので、実機フィードバックに基づき今後調整してよい。
pub const THRASH_WINDOW_MS: u64 = 60 * 60 * 1000;

/// `THRASH_WINDOW_MS` 内で許容する最大再インストール回数。
pub const THRASH_LIMIT: u32 = 5;

/// hook_starved 検知 tick で自己修復（フック再インストール）を行ってよいか判定する。
///
/// 引数:
/// - `self_heal_enabled`: `[diagnostics] hook_self_heal`（既定 true）。
/// - `is_elevated_foreground`: フォアグラウンドが昇格プロセスで自分は非昇格か（F2）。
/// - `is_session_locked`: セッションロック中か（F2）。
/// - `is_secure_desktop`: secure desktop がアクティブか（F2）。
/// - `is_relay_or_remap_foreground`: `disable_apps`/`input_relay_apps`一致、または
///   既知の入力中継/リマップソフトがフォアグラウンドか（F3）。
/// - `already_attempted_this_episode`: 現在の hook_starved episode で既に
///   再インストールを試行済みか（F1a、エピソードラッチ）。
/// - `reinstalls_in_thrash_window`: 直近 `THRASH_WINDOW_MS` 以内の再インストール回数。
/// - `thrash_limit`: レート上限（通常 [`THRASH_LIMIT`] を渡す、テスト用に引数化）。
#[must_use]
#[allow(clippy::too_many_arguments)]
pub const fn decide(
    self_heal_enabled: bool,
    is_elevated_foreground: bool,
    is_session_locked: bool,
    is_secure_desktop: bool,
    is_relay_or_remap_foreground: bool,
    already_attempted_this_episode: bool,
    reinstalls_in_thrash_window: u32,
    thrash_limit: u32,
) -> HookWatchdogAction {
    if !self_heal_enabled {
        return HookWatchdogAction::SkipDisabled;
    }
    if is_elevated_foreground {
        return HookWatchdogAction::SkipElevatedForeground;
    }
    if is_session_locked {
        return HookWatchdogAction::SkipSessionLocked;
    }
    if is_secure_desktop {
        return HookWatchdogAction::SkipSecureDesktop;
    }
    if is_relay_or_remap_foreground {
        return HookWatchdogAction::SkipRelayOrRemapForeground;
    }
    if already_attempted_this_episode {
        return HookWatchdogAction::SkipAlreadyAttemptedThisEpisode;
    }
    if reinstalls_in_thrash_window >= thrash_limit {
        return HookWatchdogAction::SkipThrashLimit;
    }
    HookWatchdogAction::Reinstall
}

/// `history`（過去の再インストール試行時刻、tick_ms）のうち、`now_ms` から
/// `window_ms` 以内に収まる件数を数える。
///
/// `now_ms < t`（クロックの巻き戻り相当）は`saturating_sub`で0扱いになり
/// window内としてカウントされる——安全側（カウントを増やす＝再インストールを
/// 抑制する方向）に倒れるため許容する。
#[must_use]
pub fn count_within_window(history: &[u64], now_ms: u64, window_ms: u64) -> u32 {
    u32::try_from(
        history
            .iter()
            .filter(|&&t| now_ms.saturating_sub(t) < window_ms)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── decide: 各ガードが単独で Skip を返すこと（優先順位どおり） ──

    #[test]
    fn decide_reinstalls_when_all_clear() {
        assert_eq!(
            decide(true, false, false, false, false, false, 0, THRASH_LIMIT),
            HookWatchdogAction::Reinstall
        );
    }

    #[test]
    fn decide_skips_when_disabled() {
        assert_eq!(
            decide(false, false, false, false, false, false, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipDisabled
        );
    }

    #[test]
    fn decide_skips_when_elevated_foreground() {
        assert_eq!(
            decide(true, true, false, false, false, false, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipElevatedForeground
        );
    }

    #[test]
    fn decide_skips_when_session_locked() {
        assert_eq!(
            decide(true, false, true, false, false, false, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipSessionLocked
        );
    }

    #[test]
    fn decide_skips_when_secure_desktop() {
        assert_eq!(
            decide(true, false, false, true, false, false, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipSecureDesktop
        );
    }

    #[test]
    fn decide_skips_when_relay_or_remap_foreground() {
        assert_eq!(
            decide(true, false, false, false, true, false, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipRelayOrRemapForeground
        );
    }

    #[test]
    fn decide_skips_when_already_attempted_this_episode() {
        // マウスのみで stale_ms>5000 が成立し続ける通常利用のケース（F1）:
        // 同じ episode 内で 2 回目以降は再インストールしない。
        assert_eq!(
            decide(true, false, false, false, false, true, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipAlreadyAttemptedThisEpisode
        );
    }

    #[test]
    fn decide_skips_when_thrash_limit_reached() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                false,
                THRASH_LIMIT,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipThrashLimit
        );
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                false,
                THRASH_LIMIT + 1,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipThrashLimit
        );
    }

    #[test]
    fn decide_reinstalls_just_below_thrash_limit() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                false,
                THRASH_LIMIT - 1,
                THRASH_LIMIT
            ),
            HookWatchdogAction::Reinstall
        );
    }

    #[test]
    fn decide_disabled_kill_switch_wins_over_all_other_guards() {
        // 優先順位の先頭であることを確認: 他の全条件が Reinstall 方向でも
        // self_heal_enabled=false なら必ず SkipDisabled。
        assert_eq!(
            decide(false, false, false, false, false, false, 0, THRASH_LIMIT),
            HookWatchdogAction::SkipDisabled
        );
    }

    // ── count_within_window ──

    #[test]
    fn count_within_window_counts_only_recent_entries() {
        // now=3_600_500, window=3_600_000 → 差が 3_600_000ms 未満なのは
        // 1_000(差3_599_500)・30_000(差3_570_500)・3_600_000(差500) の3件。
        // 0(差3_600_500)だけが window 外。
        let history = [0u64, 1_000, 30_000, 3_600_000];
        assert_eq!(count_within_window(&history, 3_600_500, 3_600_000), 3);
    }

    #[test]
    fn count_within_window_empty_history_is_zero() {
        assert_eq!(count_within_window(&[], 1_000, THRASH_WINDOW_MS), 0);
    }

    #[test]
    fn count_within_window_boundary_is_exclusive() {
        // elapsed == window_ms ちょうどは「window外」（`<` 判定、`<=` ではない）。
        assert_eq!(count_within_window(&[0], 1_000, 1_000), 0);
        assert_eq!(count_within_window(&[0], 999, 1_000), 1);
    }

    #[test]
    fn count_within_window_clock_rewind_counts_as_within_window() {
        // now_ms < t（巻き戻り相当）は saturating_sub で 0 になり window 内扱い。
        assert_eq!(count_within_window(&[10_000], 0, 1_000), 1);
    }
}
