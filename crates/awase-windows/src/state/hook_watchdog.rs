//! フックwatchdog（issue #165、hook_starved）の自己修復トリガー判定の純粋関数。
//!
//! Win32 API を一切呼ばないため Linux でも `cargo test -p awase-windows` で検証できる。
//! 実際の配線（`GetLastInputInfo`/昇格判定/セッションロック/secure desktop判定/
//! relayプロセス検出/設定読み込み）は `runtime/message_handlers.rs`
//! （`TIMER_HOOK_WATCHDOG`分岐）・`runtime/mod.rs`
//! （`Runtime::evaluate_hook_watchdog`/`send_hook_watchdog_canary`/
//! `confirm_hook_watchdog_canary`/`reinstall_keyboard_hook_for_watchdog`）を参照。
//!
//! ## 設計の経緯（round1 → round2）
//!
//! opus-adversarial-consult round1（2026-09-28、fix/hook-self-heal-v2起票時）は
//! F1〜F11を指摘し、「エピソードごとに1回だけ再インストールする」ラッチ
//! （`already_attempted_this_episode`）+ レート上限（`THRASH_WINDOW_MS`/
//! `THRASH_LIMIT`）で対応した。
//!
//! round2レビュー（`fix/hook-self-heal-v2` HEAD `ea24ec3f`対象）は、この設計に
//! **Blocker B1** を発見した: ラッチが解除される唯一の経路は「awase自身の
//! フックコールバックが直近5秒以内に呼ばれたこと」だが、hook_starvedとは
//! まさに「awaseのフックが呼ばれない状態」なので、**starvationが続く限り
//! ラッチは解除されない**。マウスのみの通常利用で先にラッチが立つと、その後に
//! 来た本物のstarvationが永久に直らなくなる（`203af106`の旧実装＝毎tick
//! 再インストールより後退）。
//!
//! 本モジュールはB1の推奨修正(i)+(ii)を実装する:
//! - **(i) カナリア**（[`HookWatchdogAction::SendCanary`]）: 全ガード通過時、
//!   即座に`Reinstall`せず、まず無害な自己注入キー（`hook::send_hook_watchdog_canary`）
//!   を送る。[`CANARY_CONFIRM_MS`]後に`hook::hook_alive_tick_ms()`が送信時刻より
//!   進んだかを[`canary_confirmed_starved`]で判定し、進んでいれば
//!   「マウスのみのアイドル」という誤検知と分かりepisodeラッチ/thrash履歴を
//!   一切消費せずスキップできる。進んでいなければ本物のstarvationと確定して
//!   初めて再インストールする（`Runtime::reinstall_keyboard_hook_for_watchdog`は
//!   カナリア確認後にしか呼ばれない設計にした）。
//! - **(ii) バックオフ**（[`next_retry_at_ms`]/[`backoff_delay_ms`]）:
//!   「エピソードにつき1回だけ」という硬い上限を、
//!   [`BACKOFF_SCHEDULE_MS`]（0s/30s/5min/30min）による繰り返しリトライへ
//!   置き換えた。本物のstarvationがカナリア確認済みの再インストール後も続く
//!   場合、一定間隔で確認・再試行し続けられる（`hook_alive_tick_ms`の
//!   自然回復を待つだけの旧設計とは異なり、starvationが続いていても停止しない）。
//!
//! さらに round2 M5（`install_hook()`失敗時にラッチだけ立って二度と
//! リトライされない）に対応するため、`hook_guard_present=false`
//! （フックが1つも存在しない）の間はバックオフ待機・thrash上限の両方を
//! バイパスする（[`decide`]参照）——フックが無い状態は「誤検知の上限」の
//! 対象ではなく、常に最優先で復旧を試みるべきだからである。

/// hook_starved 検知 tick で watchdog が取るべきアクション。
///
/// `SendCanary` 以外は全て「自己修復を行わない」を意味し、バリアントの違いは
/// ログでスキップ理由を区別するためだけに存在する。`SendCanary` も即座の
/// 再インストールではなく、カナリア確認（[`canary_confirmed_starved`]）を
/// 経てから初めて実際の再インストールに進む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookWatchdogAction {
    /// 全ガード通過。カナリアを送り、確認後に再インストールするかを決める。
    SendCanary,
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
    /// バックオフ待機中（`retry_at_ms`まで次のカナリア送信を控える）。
    /// `hook_guard_present=false`の間は発生しない（M5）。
    SkipBackoffPending {
        /// このtick_msに到達するまで待つ。
        retry_at_ms: u64,
    },
    /// 直近 `THRASH_WINDOW_MS` 以内の再インストール回数が `THRASH_LIMIT` に到達。
    /// `hook_guard_present=false`の間は発生しない（M5）。
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
/// round2でカナリア(F1c)が誤検知を弾くため、この上限は誤検知と本物を区別しない
/// round1時点の懸念（opus round2 M1）が解消され、本来の「安全弁」として機能する。
pub const THRASH_WINDOW_MS: u64 = 60 * 60 * 1000;

/// `THRASH_WINDOW_MS` 内で許容する最大再インストール回数。
pub const THRASH_LIMIT: u32 = 5;

/// カナリア注入（`hook::send_hook_watchdog_canary`）から確認タイマー
/// （`TIMER_HOOK_WATCHDOG_CANARY_CHECK`）発火までの待機時間。
///
/// `tuning.rs`の対象外（上記`THRASH_WINDOW_MS`と同じ理由）。性質としては
/// 「自己注入キーが`SendInput`→OSのフックチェーン→自分の`WH_KEYBOARD_LL`
/// コールバックへ往復するのに正常時どれだけかかるか」を見積もる値でtuning.rs
/// に近いが、**Windows実機での実測は未実施**（このブランチはLinux上でのみ
/// 開発・検証している）。同一プロセス内のOSフックチェーン通過は通常
/// サブミリ秒〜数msで完結すると見込み、実機環境差（他フックの処理時間・
/// システム負荷）へのマージンを含めて保守的に大きめの値を選んだ。実機ソーク時に
/// この値の妥当性を確認し、実測が取れ次第 tuning.rs 相当の実測コメントに
/// 置き換えること。
pub const CANARY_CONFIRM_MS: u64 = 200;

/// カナリア確認後に本物のstarvationと確定し再インストールした回数
/// （エピソード内、`hook::hook_alive_tick_ms()`の自然回復でリセット）に応じた
/// 次回リトライまでのバックオフ（ミリ秒）。
///
/// `[0, 30s, 5min, 30min]`: 最初の1回は即座、その後は指数的にではなく
/// 「まず短く様子を見て、それでも直らないなら間隔を大きく空ける」段階的な
/// スケジュールにした。5回目以降は最後の間隔（30分）を繰り返す
/// （[`backoff_delay_ms`]参照）。実機ソーク未実施の暫定値
/// （`THRASH_WINDOW_MS`と同種、実測根拠なしの安全弁）。
pub const BACKOFF_SCHEDULE_MS: [u64; 4] = [0, 30_000, 5 * 60_000, 30 * 60_000];

/// `confirmed_attempt_count`（このエピソードでカナリア確認済みの再インストールを
/// 何回試みたか、0始まり）に対応するバックオフ遅延を返す。
/// `BACKOFF_SCHEDULE_MS`の範囲を超えたら最後の値（30分）を繰り返す。
#[must_use]
pub const fn backoff_delay_ms(confirmed_attempt_count: u32) -> u64 {
    let last_idx = BACKOFF_SCHEDULE_MS.len() - 1;
    #[allow(clippy::cast_possible_truncation)] // last_idxは配列長由来の小さい値
    let idx = if (confirmed_attempt_count as usize) < last_idx {
        confirmed_attempt_count as usize
    } else {
        last_idx
    };
    BACKOFF_SCHEDULE_MS[idx]
}

/// hook_starved 検知 tick で自己修復（カナリア送信）を行ってよいか判定する。
///
/// 引数:
/// - `self_heal_enabled`: `[diagnostics] hook_self_heal`（既定 true）。
/// - `is_elevated_foreground`: フォアグラウンドが昇格プロセスで自分は非昇格か（F2）。
/// - `is_session_locked`: セッションロック中か（F2）。
/// - `is_secure_desktop`: secure desktop がアクティブか（F2）。
/// - `is_relay_or_remap_foreground`: `disable_apps`/`input_relay_apps`一致、または
///   既知の入力中継/リマップソフトがフォアグラウンドか（F3）。
/// - `hook_guard_present`: 現在フックが1つでもインストールされているか
///   （`Runtime.hook_guard.is_some()`）。`false`の間はバックオフ・thrash上限を
///   バイパスする（round2 M5: install失敗時は最優先でリトライする）。
/// - `now_ms`: 現在の`hook::current_tick_ms()`。
/// - `next_retry_at_ms`: 前回のカナリア確認済み再インストールが設定した、
///   次に試みてよい時刻（`None`なら即座に試みてよい）。
/// - `reinstalls_in_thrash_window`: 直近 `THRASH_WINDOW_MS` 以内の
///   カナリア確認済み再インストール回数。
/// - `thrash_limit`: レート上限（通常 [`THRASH_LIMIT`] を渡す、テスト用に引数化）。
#[must_use]
#[allow(clippy::too_many_arguments)]
pub const fn decide(
    self_heal_enabled: bool,
    is_elevated_foreground: bool,
    is_session_locked: bool,
    is_secure_desktop: bool,
    is_relay_or_remap_foreground: bool,
    hook_guard_present: bool,
    now_ms: u64,
    next_retry_at_ms: Option<u64>,
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
    if hook_guard_present {
        if let Some(retry_at_ms) = next_retry_at_ms {
            if now_ms < retry_at_ms {
                return HookWatchdogAction::SkipBackoffPending { retry_at_ms };
            }
        }
        if reinstalls_in_thrash_window >= thrash_limit {
            return HookWatchdogAction::SkipThrashLimit;
        }
    }
    HookWatchdogAction::SendCanary
}

/// カナリア確認タイマー（`TIMER_HOOK_WATCHDOG_CANARY_CHECK`）発火時に、
/// 実際にフックを再インストールすべきかを判定する。
///
/// - `hook_alive_tick_ms_after`: 確認時点の `hook::hook_alive_tick_ms()`。
/// - `canary_sent_at_ms`: カナリア注入時点の tick（両者とも同じ
///   `hook::current_tick_ms()` クロック上の値）。
///
/// `tick_hook_alive()` は自己注入キーも含め `hook_callback` の呼び出し毎に
/// 無条件で更新される（`hook.rs`参照）ため、カナリア送信後に値が進んでいれば
/// 「送ったカナリアがコールバックへ届いた」＝フックは生きている（誤検知）と
/// 判定できる。進んでいなければ、他プロセスのフックが `CallNextHookEx` を
/// 呼ばず自己注入キーごと握りつぶしている＝真の hook_starved。
#[must_use]
pub const fn canary_confirmed_starved(
    hook_alive_tick_ms_after: u64,
    canary_sent_at_ms: u64,
) -> bool {
    hook_alive_tick_ms_after <= canary_sent_at_ms
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

    const NO_RETRY_PENDING: Option<u64> = None;

    // ── decide: 各ガードが単独で Skip を返すこと（優先順位どおり） ──

    #[test]
    fn decide_sends_canary_when_all_clear() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SendCanary
        );
    }

    #[test]
    fn decide_skips_when_disabled() {
        assert_eq!(
            decide(
                false,
                false,
                false,
                false,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipDisabled
        );
    }

    #[test]
    fn decide_skips_when_elevated_foreground() {
        assert_eq!(
            decide(
                true,
                true,
                false,
                false,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipElevatedForeground
        );
    }

    #[test]
    fn decide_skips_when_session_locked() {
        assert_eq!(
            decide(
                true,
                false,
                true,
                false,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipSessionLocked
        );
    }

    #[test]
    fn decide_skips_when_secure_desktop() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                true,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipSecureDesktop
        );
    }

    #[test]
    fn decide_skips_when_relay_or_remap_foreground() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                true,
                true,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipRelayOrRemapForeground
        );
    }

    #[test]
    fn decide_disabled_kill_switch_wins_over_all_other_guards() {
        // 優先順位の先頭であることを確認: 他の全条件がSendCanary方向でも
        // self_heal_enabled=false なら必ず SkipDisabled。
        assert_eq!(
            decide(
                false,
                true,
                true,
                true,
                true,
                true,
                10_000,
                Some(999_999),
                THRASH_LIMIT,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipDisabled
        );
    }

    // ── decide: バックオフ（B1 (ii)） ──

    #[test]
    fn decide_skips_when_backoff_pending() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                true,
                1_000,
                Some(5_000),
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipBackoffPending { retry_at_ms: 5_000 }
        );
    }

    #[test]
    fn decide_sends_canary_when_backoff_deadline_reached() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                true,
                5_000,
                Some(5_000),
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SendCanary
        );
    }

    #[test]
    fn decide_thrash_limit_reached() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                THRASH_LIMIT,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipThrashLimit
        );
    }

    #[test]
    fn decide_sends_canary_just_below_thrash_limit() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                true,
                10_000,
                NO_RETRY_PENDING,
                THRASH_LIMIT - 1,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SendCanary
        );
    }

    // ── decide: M5（フック不在時はバックオフ/thrash上限をバイパス） ──

    #[test]
    fn decide_bypasses_backoff_when_hook_guard_absent() {
        // install_hook()が失敗して0個のフックしか無い間は、バックオフ待機中でも
        // 即座にリトライを許す（round2 M5）。
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                false, // hook_guard_present
                1_000,
                Some(999_999_999), // 遠い未来のバックオフ期限でも無視される
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SendCanary
        );
    }

    #[test]
    fn decide_bypasses_thrash_limit_when_hook_guard_absent() {
        assert_eq!(
            decide(
                true,
                false,
                false,
                false,
                false,
                false, // hook_guard_present
                10_000,
                NO_RETRY_PENDING,
                THRASH_LIMIT + 10, // 上限を大幅に超えていても無視される
                THRASH_LIMIT
            ),
            HookWatchdogAction::SendCanary
        );
    }

    #[test]
    fn decide_environmental_guards_still_apply_when_hook_guard_absent() {
        // フックが無くても、昇格/ロック/secure desktop/relayの環境ガードは
        // 引き続き適用される（意図的にreinstallを控える理由は変わらないため）。
        assert_eq!(
            decide(
                true,
                false,
                true,
                false,
                false,
                false,
                10_000,
                NO_RETRY_PENDING,
                0,
                THRASH_LIMIT
            ),
            HookWatchdogAction::SkipSessionLocked
        );
    }

    // ── backoff_delay_ms ──

    #[test]
    fn backoff_delay_ms_follows_schedule() {
        assert_eq!(backoff_delay_ms(0), 0);
        assert_eq!(backoff_delay_ms(1), 30_000);
        assert_eq!(backoff_delay_ms(2), 5 * 60_000);
        assert_eq!(backoff_delay_ms(3), 30 * 60_000);
    }

    #[test]
    fn backoff_delay_ms_repeats_last_entry_beyond_schedule() {
        assert_eq!(backoff_delay_ms(4), 30 * 60_000);
        assert_eq!(backoff_delay_ms(100), 30 * 60_000);
    }

    // ── canary_confirmed_starved ──

    #[test]
    fn canary_confirmed_starved_when_tick_did_not_advance() {
        // カナリア送信後もコールバックが一度も呼ばれず、値が変わらない＝真の starved。
        assert!(canary_confirmed_starved(1_000, 1_000));
    }

    #[test]
    fn canary_confirmed_starved_when_tick_went_backwards() {
        // 巻き戻り相当も安全側（starved扱い＝再インストールする）に倒れる。
        assert!(canary_confirmed_starved(500, 1_000));
    }

    #[test]
    fn canary_not_starved_when_tick_advanced_after_send() {
        // カナリア送信後にコールバックが呼ばれた＝フックは生きている（誤検知）。
        assert!(!canary_confirmed_starved(1_050, 1_000));
    }

    // ── B1 の具体的な失敗シナリオが解消されていることを固定するシナリオテスト ──

    #[test]
    fn scenario_startup_false_positive_does_not_block_later_real_starvation() {
        // B1シナリオ1（起動直後）: hook_alive_tick_ms=0から始まり、起動直後の
        // tickでstale_ms>5000が成立してもSendCanaryになるだけで、即座に
        // episodeラッチ/thrash枠を消費しない（カナリアが「誤検知」と判定すれば
        // 何も消費されない）。
        let action_at_startup = decide(
            true,
            false,
            false,
            false,
            false,
            true, // hook_guard_present（install_hook()成功）
            3_000,
            NO_RETRY_PENDING,
            0,
            THRASH_LIMIT,
        );
        assert_eq!(action_at_startup, HookWatchdogAction::SendCanary);

        // カナリアが誤検知（フックは生きている）と判定された場合、
        // next_retry_at_ms/thrash履歴は一切更新されない（呼び出し元
        // `Runtime::confirm_hook_watchdog_canary`がreinstallを呼ばないため）。
        // そのため、後から本当にstarvationが始まったtickでも、
        // 依然としてSendCanaryが返り続ける（B1が修正前は
        // SkipAlreadyAttemptedThisEpisodeに固定されて二度と反応しなかった）。
        let action_when_real_starvation_begins = decide(
            true,
            false,
            false,
            false,
            false,
            true,
            60_000, // ずっと後のtick
            NO_RETRY_PENDING,
            0,
            THRASH_LIMIT,
        );
        assert_eq!(
            action_when_real_starvation_begins,
            HookWatchdogAction::SendCanary
        );
    }

    #[test]
    fn scenario_confirmed_reinstall_then_persisting_starvation_retries_after_backoff() {
        // B1シナリオ2: カナリア確認済みの本物のstarvationでreinstallした後も
        // starvationが続く場合、backoff_delay_ms(0)=0msなのでバックオフ無しに
        // 次のtickでも即座にSendCanaryへ戻れる（1回目の確認済み再インストール）。
        let now_ms = 10_000;
        let next_retry_at_ms = Some(now_ms + backoff_delay_ms(0));
        let still_starved_next_tick = decide(
            true,
            false,
            false,
            false,
            false,
            true,
            now_ms + 3_000, // 3秒後のtick
            next_retry_at_ms,
            1, // 直前の確認済み再インストールがthrash履歴に1件
            THRASH_LIMIT,
        );
        // backoff_delay_ms(0)==0なので、next_retry_at_ms<=now_ms+3_000は常に真
        assert_eq!(still_starved_next_tick, HookWatchdogAction::SendCanary);
    }
}
