//! MS-IME confirm-then-transmit コルーチン実装。
//!
//! # 背景（BUG-13: MS-IME cold start「を」→「wお」）
//!
//! `MsImeStrategy` は「MS-IME の TSF context は常にウォーム」という前提で
//! `is_warm()=true` / `needs_f2_probe()=false` を固定しており、cold-start 保護が
//! 一切なかった。しかし IME OFF→ON 遷移直後は OS 側の準備に実測 ~130-300ms かかり
//! （2026-07-06 WT×MS-IME: 遷移 +122ms で conv=0x00000000 のまま "wo" を送信 →
//! 'w' がリテラル化して「wお」）、この窓に VK を送ると先頭文字が化ける。
//!
//! # 方式: 固定待ちではなく IMC 観測で「準備完了」を確信してから送信する
//!
//! GJI の F2 probe（プロセス I/O 観測）と同型の confirm-then-transmit。MS-IME には
//! 変換専用プロセスがないため、観測シグナルには `IMC_GETCONVERSIONMODE`
//! （フォーカス先の conversion mode。準備完了で NATIVE ビットが立つ）を使う。
//! Chrome 経路の `send_chrome_gji_reinit_and_poll` で運用実績のあるシグナル。
//!
//! この観測シグナル自体は injection mode に依存しないため、`InjectionMode::Vk`
//! （Chrome/Edge/Electron、既定設定の TsfNative アプリ）にも同じゲートを展開している
//! （`Output::ms_ime_gate_defer` 参照）。`target: TransmitTarget` で Phase 2 の送信先を
//! Tsf/Chrome のどちらにするか切り替える。
//!
//! ```text
//! send_romaji_as_tsf / send_romaji_batched（MS-IME + ImeModeFsm 未確認）
//!   ├─ romaji を保持して MsImeReadyCoro を pending_tsf に設置
//!   ├─ Output::start_ms_ime_ready_poll が IMC ポーリング開始（10ms 間隔、async）
//!   │    └─ ImeModeFsm を on_conversion_mode_read で確定させる
//!   └─ コルーチン: env.ime_mode が NATIVE 確認されるまで tick 待機
//!        ├─[確認]────────► Transmit(target) → deferred VK flush → Done
//!        └─[期限切れ]────► 強制 Transmit（安全弁。give-up latch はポーリング側が設定。
//!                          未確認のまま送るため IMC が読めない環境では先頭文字が
//!                          リテラル化する可能性が残る）
//! ```
//!
//! probe 中に届いた後続キーは既存の deferred VK 機構
//! （`defer_if_probe_in_flight` / `defer_vk_if_probe_in_flight`）に積まれ、
//! dispatcher の `Transmit` アームが送信直後に flush する（順序保証）。
//!
//! # `OutputActiveGuard` は Phase 1 では確保しない（BUG-58）
//!
//! Phase 1（NATIVE 確認待ち）は `IMC_GETCONVERSIONMODE` の読み取り結果を待つだけで
//! 一切 SendInput を行わない。にも関わらず旧実装は `MsImeReadyCoro::new()` の時点で
//! `OutputActiveGuard` を確保し、Phase 1 の間ずっと保持していた。これは
//! `OUTPUT_GATE.active` を通じて `app/mod.rs` の物理キー分配
//! （`handle_wm_key_from_hook` 呼び出し）自体を止めてしまう。小指シフト面の
//! チョード（例: Shift+1=「！」）では、conv を Off→NATIVE に戻せる唯一の経路
//! （`kp_shift_conv_guard_key_up`、物理 Shift KeyUp 契機）がまさにこのブロックの
//! 対象になり、「NATIVE を待つゲート」と「NATIVE に戻す処理の起動」が互いを
//! 塞ぎ合う循環待ちに陥って `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（5000ms）の
//! 満了まで毎回フリーズしていた（docs/known-bugs.md BUG-58）。
//!
//! 現在の実装は `OutputActiveGuard` を Phase 2（Transmit 直前）でのみ確保する。
//! Phase 1 は無出力のまま物理キー分配を妨げないため、Shift KeyUp が実時間で
//! 処理され復元が正常に走り、循環が構造的に解消される。

use std::rc::Rc;

use crate::state::event_origin::Generation;
use crate::tsf::ime_mode_fsm::ImeModeState;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TransmitPlan, TransmitTarget, TsfEnvSnapshot};
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};

/// env の IME mode が「かな VK を受け付けられる」状態として確認済みか。
///
/// Hiragana / Katakana はどちらも NATIVE ビット確認済みで romaji VK が compose される。
/// 純粋関数（`ImeModeFsm::is_native_ready` の env 版）。
fn env_native_ready(env: TsfEnvSnapshot) -> bool {
    env.ime_mode_confirmed
        && matches!(
            env.ime_mode,
            ImeModeState::Hiragana | ImeModeState::Katakana
        )
}

// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計
// による意図的な制約（crates/timed-fsm/src/coro.rs::yield_step 参照）。
#[expect(clippy::future_not_send)]
async fn ms_ime_ready_coro_body(
    ch: Rc<Channel<TsfEnvSnapshot, Vec<ProbeAction>>>,
    cold_seq: Generation,
    romaji: String,
    deadline_ms: u64,
    target: TransmitTarget,
) {
    // ── Phase 1: ImeModeFsm の NATIVE 確認待ち ─────────────────────────────
    // 確認の実体は Output::start_ms_ime_ready_poll の async IMC ポーリング。
    // ここでは env 経由で結果を観測するだけ（tick = TIMER_TSF_PROBE 10ms 間隔）。
    let start_ms = crate::hook::current_tick_ms();
    loop {
        let env = yield_step(ch.clone(), vec![]).await;
        if env_native_ready(env) {
            log::info!(
                "[msime-ready] cold={cold_seq} IME mode NATIVE 確認 (+{}ms) → 送信 {romaji:?}",
                crate::hook::current_tick_ms().saturating_sub(start_ms),
                cold_seq = cold_seq.value(),
            );
            break;
        }
        // ADR-084（BUG-49 追補2、pass-5 レビュー反映）: `shift-conv-guard` の
        // hold 中は `Output::confirm_gate_deadline_override_ms` が非 0 になり、
        // 元の `deadline_ms`（送信試行時点起点、BUG-13 の cold-start 保護）を
        // 実効的に押し出す。`0`（shift-conv-guard と無関係、または hold 外）
        // のときは `.max(0)` で無変化 = 従来どおり `deadline_ms` のみが効く。
        // hold 中は override が `current_tick_ms() + SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`
        // （有限キャップ、真の無期限ではない）になる。hold 終了（Shift 解放/復元）
        // で override は「復元時点 + SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS」という
        // フレッシュな猶予に差し替わり（`kp_shift_conv_guard_key_up` 参照）、
        // 続く `kp_restore_kana_from_half_width` のリトライループが
        // `shift_conv_guard_gen` が一致する限り毎試行ごとに同じ幅で押し出し続ける。
        let effective_deadline_ms = deadline_ms.max(env.confirm_gate_deadline_override_ms);
        if crate::hook::current_tick_ms() >= effective_deadline_ms {
            // 安全弁: IMC が読めない環境でタイピングを止めない。
            // give-up latch（連続発動の抑止）は start_ms_ime_ready_poll 側が設定する。
            log::warn!(
                "[msime-ready] cold={cold_seq} 期限切れ (mode={:?} confirmed={} \
                 deadline=0x{effective_deadline_ms:X}) → 強制送信 {romaji:?}",
                env.ime_mode,
                env.ime_mode_confirmed,
                cold_seq = cold_seq.value(),
            );
            break;
        }
    }

    // ── Phase 2: Transmit → Done ──────────────────────────────────────────
    // dispatcher が romaji 送信 → deferred VK flush → warm マークまで行う。
    // F2 前置は不要（MS-IME は VK_DBE_HIRAGANA warmup を必要としない）。
    // LiteralDetect は GJI 観測（candidate window / write_bytes）前提のため使わない。
    //
    // BUG-58: `OutputActiveGuard` は実際に SendInput を伴う Phase 2 に入る
    // 直前でのみ確保する（Phase 1 は IMC 観測を待つだけで無出力）。
    // Phase 1 の間も保持していた旧実装は、`OUTPUT_GATE.active` が
    // `app/mod.rs` の物理キー分配（`handle_wm_key_from_hook`）そのものを
    // 止めてしまうことを見落としていた。conv を Off→NATIVE に戻せる唯一の
    // 経路（`kp_shift_conv_guard_key_up` は物理 Shift KeyUp が
    // `handle_wm_key_from_hook` に届いて初めて起動する）がこの間ブロックされ、
    // 「NATIVE を待つゲート」と「NATIVE に戻す処理の起動」が互いを塞ぎ合う
    // 循環待ちになり、`SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（5000ms）の
    // 安全弁満了まで毎回フリーズしていた（詳細: docs/known-bugs.md BUG-58）。
    // Phase 1 を無出力のまま `OutputActiveGuard` なしで待たせることで、
    // 物理 Shift KeyUp が実時間で処理され復元が正常に走るようになり、
    // 循環が構造的に解消される。後続キーの出力順序は本モジュール冒頭の
    // module doc のとおり `defer_if_probe_in_flight`/`defer_vk_if_probe_in_flight`
    // （engine 経由の romaji）と `run_passthrough_pipeline` の
    // `has_pending_tsf_work()` チェック（PassThrough 全般、BUG-58 で追加）
    // が別途保証するため、`OutputActiveGuard` を Phase 1 で持つ必要はない。
    let _guard = OutputActiveGuard::begin();
    yield_step(
        ch,
        vec![
            ProbeAction::Transmit {
                cold_seq,
                plan: TransmitPlan {
                    used_eager_path: false,
                    needs_literal: false,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                romaji,
                target,
            },
            ProbeAction::Done,
        ],
    )
    .await;
}

/// MS-IME confirm-then-transmit コルーチン。
///
/// [`TickableFsm`] を実装し `pending_tsf` に格納される。
/// 設置は `Output::ms_ime_gate_defer`（`send_romaji_as_tsf` / `send_romaji_batched`
/// のゲート）。
pub(crate) struct MsImeReadyCoro {
    coro: StepCoro<TsfEnvSnapshot, Vec<ProbeAction>>,
    cold_seq: Generation,
    // BUG-58: `OutputActiveGuard` はここでは確保しない（Phase 1 は無出力）。
    // `ms_ime_ready_coro_body` の Phase 2 直前でローカルに確保し、コルーチンの
    // 完了（Done yield 後の future 完了）まで保持する。詳細は Phase 2 直前の
    // コメント参照。
}

impl MsImeReadyCoro {
    pub(crate) fn new(
        romaji: &str,
        cold_seq: Generation,
        deadline_ms: u64,
        target: TransmitTarget,
    ) -> Self {
        let romaji = romaji.to_string();
        let mut coro = StepCoro::new(async move |ch| {
            ms_ime_ready_coro_body(ch, cold_seq, romaji, deadline_ms, target).await;
        });
        // pending_tsf に格納して外部から本物の tick を受け取り始める前に prime() で
        // 消費しておく（詳細は `GjiWarmupCoro::new` のコメント参照）。
        let primed = coro.prime();
        debug_assert!(
            matches!(&primed, CoroStep::Yielded(actions) if actions.is_empty()),
            "MsImeReadyCoro prime() は空の ProbeAction を yield するはず: {primed:?}"
        );
        Self { coro, cold_seq }
    }
}

impl TickableFsm for MsImeReadyCoro {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        match self.coro.step(env) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    fn cold_seq_hint(&self) -> Generation {
        self.cold_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::warmup::probe_fsm::TsfEnvSnapshot;

    fn env(mode: ImeModeState, confirmed: bool) -> TsfEnvSnapshot {
        TsfEnvSnapshot {
            ime_mode: mode,
            ime_mode_confirmed: confirmed,
            ..Default::default()
        }
    }

    fn env_with_override(mode: ImeModeState, confirmed: bool, override_ms: u64) -> TsfEnvSnapshot {
        TsfEnvSnapshot {
            confirm_gate_deadline_override_ms: override_ms,
            ..env(mode, confirmed)
        }
    }

    /// BUG-58 レビュー指摘: `OUTPUT_GATE`（`tsf/probe_bridge.rs`）はプロセス全体で
    /// 共有される static。Phase 2（Transmit yield 直前）に到達するテストは
    /// `OutputActiveGuard::begin()` を実際に呼ぶため、`cargo test` の既定の
    /// マルチスレッド実行下では、このファイル内で Phase 2 に到達する複数テストが
    /// 互いの `depth`/`active` を汚染しうる（同ファイル外の
    /// `GjiWarmupCoro`/`ChromeProbe`/`LiteralDetectFsm` 等も同じ static を触るため、
    /// クレート全体での競合は完全には排除できないが、少なくともこのファイル内の
    /// テスト同士は直列化する）。Phase 2 に到達する全テストの先頭でこのロックを
    /// 取ること。
    static GATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn native_ready_requires_confirmation() {
        // belief だけ（unconfirmed）では準備完了と見なさない — BUG-13 はまさに
        // belief=ON のまま OS 未準備の窓で送信したことが原因。
        assert!(!env_native_ready(env(ImeModeState::Hiragana, false)));
        assert!(env_native_ready(env(ImeModeState::Hiragana, true)));
    }

    #[test]
    fn katakana_counts_as_ready() {
        // ユーザーが意図的にカタカナモードの場合も NATIVE 確認済みなら送信してよい
        // （MsImeDirectStrategy の KATAKANA スキップと同じ扱い）。
        assert!(env_native_ready(env(ImeModeState::Katakana, true)));
    }

    #[test]
    fn off_and_unknown_are_not_ready() {
        assert!(!env_native_ready(env(ImeModeState::Off, true)));
        assert!(!env_native_ready(env(ImeModeState::Unknown, true)));
        assert!(!env_native_ready(env(ImeModeState::Unknown, false)));
    }

    #[test]
    fn coro_waits_until_confirmed_then_transmits() {
        let _lk = GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = crate::hook::current_tick_ms() + 60_000;
        let mut coro = MsImeReadyCoro::new("wo", Generation::new(7), deadline, TransmitTarget::Tsf);

        // 未確認の間は待機（アクションなし）
        for _ in 0..3 {
            let actions = coro.tick(env(ImeModeState::Hiragana, false));
            assert!(actions.is_empty(), "未確認中は待機するはず: {actions:?}");
        }

        // NATIVE 確認 → Transmit + Done
        let actions = coro.tick(env(ImeModeState::Hiragana, true));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            ProbeAction::Transmit { romaji, target: TransmitTarget::Tsf, plan, .. }
                if romaji == "wo" && !plan.needs_literal
        ));
        assert!(matches!(actions[1], ProbeAction::Done));
    }

    /// Vk 注入モード（Chrome/Edge/Electron 等）向け: ゲートが `TransmitTarget::Chrome` で
    /// 設置された場合、Phase 2 の Transmit も Chrome ターゲットで発行されること。
    /// `TransmitTarget::Tsf` へのハードコードが復活する退行を防ぐための固定テスト。
    #[test]
    fn coro_transmits_via_chrome_target_when_installed_for_vk_mode() {
        let _lk = GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = crate::hook::current_tick_ms() + 60_000;
        let mut coro =
            MsImeReadyCoro::new("ka", Generation::new(9), deadline, TransmitTarget::Chrome);

        let actions = coro.tick(env(ImeModeState::Hiragana, false));
        assert!(actions.is_empty(), "未確認中は待機するはず: {actions:?}");

        let actions = coro.tick(env(ImeModeState::Hiragana, true));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            ProbeAction::Transmit { romaji, target: TransmitTarget::Chrome, plan, .. }
                if romaji == "ka" && !plan.needs_literal
        ));
        assert!(matches!(actions[1], ProbeAction::Done));
    }

    #[test]
    fn coro_transmits_on_deadline_even_without_confirmation() {
        let _lk = GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // 安全弁: IMC が読めない環境でも期限でタイピングを止めない。
        let deadline = crate::hook::current_tick_ms(); // 即座に期限切れ
        let mut coro = MsImeReadyCoro::new("ka", Generation::new(8), deadline, TransmitTarget::Tsf);

        let actions = coro.tick(env(ImeModeState::Unknown, false));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            ProbeAction::Transmit { romaji, .. } if romaji == "ka"
        ));
        assert!(matches!(actions[1], ProbeAction::Done));
    }

    /// ADR-084（BUG-49 追補2）の核心: `confirm_gate_deadline_override_ms` が
    /// 大きい（十分未来の）値のとき、元の `deadline_ms` がとっくに過ぎていても
    /// 強制送信しない。ここでは `.max()` のふるまいを極端値で固定するために
    /// `u64::MAX` を使うが、これはテスト専用の値であり、本番コードが実際に
    /// セットする値ではない（本番は `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`
    /// による有限キャップ、`kp_shift_conv_guard_key_down` 参照 — pass-5
    /// レビューで `u64::MAX` の真の無期限は「Shift KeyUp がフックに届かない
    /// 場合に安全弁が永久disableされる」懸念により撤回された）。この test が
    /// 第一・第二の実装（deadline の起点を `unconfirmed_since` から素朴に
    /// 計算する版）では存在せず、実機で「期限切れ→半角化」の回帰を後から
    /// 発見する結果になった — 今後同じ回帰を作らないための直接固定。
    #[test]
    fn coro_does_not_expire_while_confirm_gate_is_overridden_to_max() {
        let _lk = GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = crate::hook::current_tick_ms(); // 元の期限は即座に切れる
        let mut coro = MsImeReadyCoro::new("!", Generation::new(10), deadline, TransmitTarget::Tsf);

        for _ in 0..5 {
            let actions = coro.tick(env_with_override(ImeModeState::Off, true, u64::MAX));
            assert!(
                actions.is_empty(),
                "override=MAX の間は元の deadline_ms が過ぎていても待機し続けるはず: {actions:?}"
            );
        }

        // override が外れて（0 = 上書きなし）元の（とっくに過ぎた）期限が復活すると、
        // 次の tick で強制送信する。
        let actions = coro.tick(env(ImeModeState::Off, true));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            ProbeAction::Transmit { romaji, .. } if romaji == "!"
        ));
    }

    /// `kp_shift_conv_guard_key_up`／`kp_restore_kana_from_half_width` が
    /// hold 終了時点・各リトライ試行の冒頭で書き込む有限の override
    /// （「今から `SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS` 後」を模した未来の値）は、
    /// たとえ元の `deadline_ms`（defer/送信試行時点起点、ここでは既に過ぎている）より
    /// 大きくても、それが優先されて期限切れにならない。`.max()` の実装が
    /// 「大きい方 = より遅い方 = より長く待つ方」を正しく選んでいることを
    /// 固定する（逆にしてしまうと却下済みの実装と同じ「期限が早まる」
    /// バグになる）。
    #[test]
    fn coro_prefers_the_later_of_deadline_ms_and_finite_override() {
        let already_passed_deadline = crate::hook::current_tick_ms(); // 即座に期限切れのはずの値
        let mut coro = MsImeReadyCoro::new(
            "de",
            Generation::new(11),
            already_passed_deadline,
            TransmitTarget::Tsf,
        );

        let future_override = crate::hook::current_tick_ms() + 10_000;
        let actions = coro.tick(env_with_override(ImeModeState::Off, true, future_override));
        assert!(
            actions.is_empty(),
            "deadline_ms は既に過ぎているが、より遅い override が優先されて \
             待機し続けるはず: {actions:?}"
        );
    }

    /// BUG-58 の直接固定: Phase 1（NATIVE 未確認で待機中）は `OUTPUT_GATE.active`
    /// を立てない。これが立ったままだと、conv を Off→NATIVE に戻す唯一の経路
    /// （物理 Shift KeyUp 契機の `kp_shift_conv_guard_key_up`）が
    /// `app/mod.rs::handle_wm_key_from_hook` への到達自体をブロックされ、
    /// 循環待ちで `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（5000ms）まで
    /// フリーズする（docs/known-bugs.md BUG-58）。`OUTPUT_GATE` はプロセス全体で
    /// 共有される static のため、他の並行テストの影響を避けるべく「このテスト内での
    /// 遷移」だけを確認する（テスト開始時点で depth==0、Phase 1 中は変化なし、
    /// Transmit と同時に active になる、を確認）。
    #[test]
    fn phase1_does_not_hold_output_gate_only_phase2_does() {
        use crate::tsf::probe_bridge::OUTPUT_GATE;

        let _lk = GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active_before = OUTPUT_GATE.is_active();
        let deadline = crate::hook::current_tick_ms() + 60_000;
        let mut coro = MsImeReadyCoro::new("!", Generation::new(12), deadline, TransmitTarget::Tsf);

        // Phase 1: 未確認の間、tick を重ねても OUTPUT_GATE は動かない。
        for _ in 0..3 {
            let actions = coro.tick(env(ImeModeState::Off, true));
            assert!(actions.is_empty(), "未確認中は待機するはず: {actions:?}");
            assert_eq!(
                OUTPUT_GATE.is_active(),
                active_before,
                "Phase 1（無出力の待機）は OUTPUT_GATE.active を変化させてはならない \
                 （変化すると物理キー分配がブロックされ BUG-58 の循環待ちが再発する）"
            );
        }

        // Phase 2: NATIVE 確認 → Transmit と同時に OUTPUT_GATE.active になる。
        let actions = coro.tick(env(ImeModeState::Hiragana, true));
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[0], ProbeAction::Transmit { romaji, .. } if romaji == "!"));
        assert!(
            OUTPUT_GATE.is_active(),
            "Phase 2（Transmit 直前）で OUTPUT_GATE.active になっているはず"
        );
    }
}
