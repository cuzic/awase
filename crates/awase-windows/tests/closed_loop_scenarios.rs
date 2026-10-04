#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! 擬似 IME を使った閉ループのシナリオテスト（Linux ホストで動く）。
//!
//! BUG-162（予測器が古い「入力中」段階を持ち越し→誤予測→Engine 活性化→warrant が下りて IME を
//! 書き戻す）と BUG-163（起動直後の `desired_open=true` 初期値と観測の鮮度窓で、明示意図なしの
//! 補正が発火）は、部品ごとの単体テストは通るのに**合成（状態の持ち越し・初期条件・時間）で**
//! 壊れた。ここでは IME を模した擬似 IME（`support/pseudo_ime.rs`）に、キー・観測・フォーカス・
//! 時刻の進行を流し込み、awase の純粋な状態遷移層を実際に呼んで、出てくる書き込み命令を集め、
//! ADR-191 の不変条件（`support/invariants.rs` の P1〜P3）を検査する。
//!
//! # host（Linux）から見える層 / 見えない層
//!
//! | 層 | host から | このテストでの扱い |
//! |---|---|---|
//! | `state/ime_model.rs`（`ImeModel::reduce`・`resolve_open_at`） | 見える | そのまま呼ぶ |
//! | `state/intent_store.rs`・`state/open_warrant.rs`（`issue_open_warrant`） | 見える | そのまま呼ぶ |
//! | `state/key_effect_predictor.rs`（`KeyEffectKeymap::predict`、ATOK 同梱表） | 見える | そのまま呼ぶ |
//! | `state/drift_correction.rs`（`check_drift_correction`） | 見える（本 PR で `platform_state.rs` から本体を移した） | そのまま呼ぶ |
//! | `awase::engine::Engine`（活性遷移・`SetOpen`） | 見える | `RefreshState`/`FocusChanged` だけ呼ぶ（`on_input` は呼ばない） |
//! | `state/platform_state.rs`（`ImeStateHub`） | **見えない**（`state/mod.rs` で `#[cfg(windows)]`） | 数行の配線を `support/harness.rs` に写した（写し元は同ファイル冒頭） |
//! | `runtime/`（`key_pipeline.rs::kp_predict_key_effect`・`ime_refresh.rs::ir_apply_drift_correction`・`executor.rs::dispatch_ime_set_open`） | **見えない**（`runtime/mod.rs` ごと `#[cfg(windows)]`） | 呼び出し順だけを写した。Blind/Read の再送打ち切り・settle 待ち・通過マーク（ADR-187）は写していない |
//! | `state/mode_key_pass.rs`（`ModeKeyPassLatch`） | 型は見えるが `pub(crate)` | 使わない（通過マークの揃えは起きない） |
//! | observer/・tsf/・output/（実 Win32/TSF） | 見えない | 擬似 IME が代わる |
//!
//! # 擬似 IME の真値
//!
//! `tools/e2e/ime_key_matrix/grid-tables/atok.json`（GJI ATOK プリセットの CI 実機格子学習の生データ）。
//! 予測器の表（`state/key_effect_table.rs::ATOK`）はここから非決定セルを除いた部分集合なので、
//! 「予測器は答えを持たないが実 IME は動く」セルを擬似 IME は動かせる。詳細と仮定は
//! `support/pseudo_ime.rs` の doc。
//!
//! # シナリオの足し方（手引き）
//!
//! 1. `Harness::start(Setup::imm_cross(TrueState { open, conv, stage }))` で起動する。
//! 2. `key(vk)` / `observe(Source::ImmCross | Source::Poll)` / `observe_value(source, open)` /
//!    `advance_ms(n)` / `focus_change()` / `user_set_open(open)` / `block_writes(bool)` /
//!    `external_set_open(open)`（awase の見ていない経路での開閉の変更）を並べる。
//!    各ステップの後、Engine の再評価と drift 判定が1回ずつ走り、書き込み命令が `h.writes` に、
//!    drift の発火が `h.drift_fires` に、打鍵の予測が `h.predictions` に積まれる。
//! 3. `assert_ok(&h, p1_...(&h))` 等で不変条件を検査する（予測の検査は `predictions_agree_with_truth`
//!    も併用する）。失敗時は経過（`h.trace()`）が出る。
//! 4. 擬似 IME は実測の無い状態からの押下で panic する。実測の範囲（atok.json のキー）に収めること。
//! 5. 既知の未修正バグを再現するシナリオは `#[ignore = "BUG-NNN: 未修正。修正後に外す"]` を付けて残し、
//!    `cargo test -p awase-windows --test closed_loop_scenarios -- --ignored` で失敗することを確かめる。

mod support;

use awase_windows::state::open_warrant::WarrantBasis;
use support::harness::{Harness, Setup, Source, WriteOrigin};
use support::invariants::{
    assert_ok, belief_matches_truth_at_end, p1_no_warranted_write_without_intent,
    p2_no_stale_stage_after_unpredicted_key, p3_startup_aligns_desired_without_drift,
    predictions_agree_with_truth,
};
use support::pseudo_ime::{Grid, TrueStage, TrueState, CONV_ALNUM, CONV_HIRAGANA};

const VK_K: u16 = 0x4B;
const VK_ESC: u16 = 0x1B;
const VK_ENTER: u16 = 0x0D;
const VK_MUHENKAN: u16 = 0x1D;
const VK_HIRAGANA: u16 = 0xF2;
const VK_SPACE: u16 = 0x20;

const fn state(open: bool, conv: u32) -> TrueState {
    TrueState {
        open,
        conv,
        stage: TrueStage::None,
    }
}

/// BUG-162 の連鎖の入力列: IME 開・半角英数（ATOK）で `k`（入力中）→ Esc → 無変換。
///
/// 実 IME（格子）: Esc で入力中を破棄（`on-c10-typing|esc` → `ON/0x10/破棄`）、続く無変換は
/// 入力中でない半角英数から IME を閉じる（`on-c10-none|muhenkan` → `OFF/0x10`）。
/// 予測器: `on-c10-typing|esc` は非決定セルとして表から除外されている。`5476cdaa`（BUG-162 A の1段目）より前は
/// Esc が「予測なし」→ 記録した段階 `Typing` が残り、無変換を「入力中の無変換」（開のまま・ひらがな C19）と
/// 誤予測 → Engine 活性化 → `SetOpen(true)` に（直前 300ms 以内の ImmCross/High「開」を根拠に）`DirectRead` の
/// warrant が下り、ユーザーが無変換で閉じた IME を awase が書き戻していた。`d3a05d25`（修正前の develop）で
/// 下の2本が P2・P1 違反で失敗することを確認済み（PR 本文に失敗メッセージ）。
fn bug162_sequence(h: &mut Harness) {
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_K)
        .advance_ms(80)
        .key(VK_ESC)
        .advance_ms(80)
        .key(VK_MUHENKAN)
        .advance_ms(20);
}

/// BUG-162 A-1（P2）の回帰: Esc の後、古い `Typing` が無変換の予測を狂わせない。
#[test]
fn bug162_stale_typing_stage_after_esc_does_not_mispredict_muhenkan() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    bug162_sequence(&mut h);
    assert_ok(&h, p2_no_stale_stage_after_unpredicted_key(&h));
    // P2 は「予測なしの直後」だけを見る。1段目の修正後は Esc が追跡だけの予測を返すので、一般形も併用する。
    assert_ok(&h, predictions_agree_with_truth(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
}

/// BUG-162 の帰結（P1）の回帰: 誤予測で Engine が活性化し、明示意図なしに IME を開け直さない。
#[test]
fn bug162_misprediction_does_not_write_ime_back_open() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    bug162_sequence(&mut h);
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert!(!h.ime.state().open, "無変換で閉じたまま\n{}", h.trace());
}

/// BUG-162 A の2段目（Engine 活性化に伴う `SetOpen` を IME に書かない。v2.0.0 で再現しなくなった〈2026-10-04、`--ignored` 実行で PASS。ActivationSync 撤去、ADR-213 P2c〉ため #[ignore] を外して回帰テストにした）: 予測が正しくても、
/// belief が古い（awase の見ていない経路で IME が閉じた直後、High 観測はまだ鮮度窓 3 秒の内）と、
/// ひらがなキーの予測で Engine が活性化し、`SetOpen(true)` に `DirectRead` の warrant が下りて IME を開ける。
/// 実 IME（格子）: 閉のひらがなキーは閉のまま（`off-c10-none|hiragana` → `OFF/0x10`）。
#[test]
fn bug162_stage2_activation_set_open_does_not_open_ime_closed_outside_awase() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .external_set_open(false)
        .advance_ms(50)
        .key(VK_HIRAGANA)
        .advance_ms(20);
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
}

/// 未起票（BUG-162 A の1段目の範囲外）: 変換中（Space）の Esc は ATOK 表にセルが無く（保持/破棄が割れる）、
/// 1段目の修正は「従来どおり予測なし」で追跡した段階 `ConvSpace` を残す。実 IME（格子第3版）は
/// `on-c19-conv-space|esc` → `ON/0x19/保持`。**擬似 IME の仮定**（変換中の Esc で「保持」なら入力中＝読みへ戻る、
/// `support/pseudo_ime.rs`）の下では、続く無変換は入力中の無変換（`on-c19-typing|muhenkan` → 半角英数 `0x10`）
/// だが、予測器は古い `ConvSpace` の行（ひらがなのまま）を引く。仮定を実機で確かめるまで未起票のまま残す。
#[test]
#[ignore = "未起票: 変換中の Esc の後の古い段階（擬似 IME の仮定に依存）。起票・修正後に外す"]
fn conversion_esc_does_not_leave_stale_conv_stage() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_K)
        .advance_ms(80)
        .key(VK_SPACE)
        .advance_ms(80)
        .key(VK_ESC)
        .advance_ms(80)
        .key(VK_MUHENKAN)
        .advance_ms(20);
    assert_ok(&h, p2_no_stale_stage_after_unpredicted_key(&h));
}

/// BUG-163（P3）の入力列: IME を閉じた状態で起動し、500ms ごとに観測する。
fn bug163_run(source: Source) -> Harness {
    let mut h = Harness::start(Setup::imm_cross(state(false, CONV_HIRAGANA)));
    h.advance_ms(500)
        .observe(source)
        .advance_ms(500)
        .observe(source)
        .advance_ms(500)
        .observe(source);
    h
}

/// BUG-163（P3）: 最初の成功観測が「閉」なら desired がそれに揃い、drift 補正が発火しない。
/// 観測源は現在の CI（ImmCrossProbe/High。`f2a875cd` の run で起動0.5秒後・約1秒後に発火）。
///
/// 1段目の修正（`b6ab8980`、warrant の下りない ImmCross の補正を検知の手前で見送る）と、代案A（起動時の初期値のままの
/// desired を最初の成功観測へ揃える、`ImeStateHub::align_placeholder_desired`）で緑になった。
#[test]
fn bug163_startup_closed_imm_cross_observation_aligns_desired_without_drift_correction() {
    let h = bug163_run(Source::ImmCross);
    assert_ok(&h, p3_startup_aligns_desired_without_drift(&h));
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
}

/// BUG-163（P3）の旧 CI の形（観測源 ObserverPoll/Medium、run 35620809258）。
#[test]
fn bug163_startup_closed_poll_observation_aligns_desired_without_drift_correction() {
    let h = bug163_run(Source::Poll);
    assert_ok(&h, p3_startup_aligns_desired_without_drift(&h));
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
}

// ── 正常系（ignore なしで緑） ─────────────────────────────────────────────

/// 起動時に IME が開いていれば、初期値 `desired_open=true` と観測が一致し、何も書かない。
#[test]
fn startup_open_observation_needs_no_correction() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    h.advance_ms(500)
        .observe(Source::ImmCross)
        .advance_ms(500)
        .observe(Source::Poll)
        .advance_ms(500);
    assert_ok(&h, p3_startup_aligns_desired_without_drift(&h));
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
    assert!(h.drift_fires.is_empty(), "{}", h.trace());
}

/// ユーザーがひらがなキー（0xF2）でかなへ切り替え、打鍵して Enter で確定する。
/// 予測が打鍵の時点で Engine を活性化し、書き込みは実状態を変えない echo だけ。
#[test]
fn hiragana_key_then_typing_and_commit() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_HIRAGANA)
        .advance_ms(30)
        .key(VK_K)
        .advance_ms(80)
        .key(VK_ENTER)
        .advance_ms(300)
        .observe(Source::ImmCross)
        .advance_ms(100);
    assert_eq!(
        h.ime.state(),
        state(true, CONV_HIRAGANA),
        "擬似 IME: 半角英数→ひらがな、入力中は確定で抜ける\n{}",
        h.trace()
    );
    assert!(
        h.predictions.iter().all(|p| p.prediction.is_some()),
        "この列は全キーが予測表にある\n{}",
        h.trace()
    );
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, p2_no_stale_stage_after_unpredicted_key(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
    assert!(h.drift_fires.is_empty(), "{}", h.trace());
}

/// フォーカス変更: 観測・明示意図・予測は新しい窓の文脈で捨てられ、新しい観測で追随し直す。
/// その間、明示意図なしに実状態を変える書き込みは出ない。
#[test]
fn focus_change_then_reobserve_keeps_invariants() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(300)
        .focus_change()
        .advance_ms(50)
        .observe(Source::ImmCross)
        .advance_ms(500)
        .observe(Source::Poll)
        .advance_ms(100);
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
    assert!(h.drift_fires.is_empty(), "{}", h.trace());
}

/// 明示意図のある正常な drift: ユーザーが IME OFF を指示したが書き込みが効かず IME が開いたまま
/// → 観測が desired と食い違う → 明示意図があるので即時に補正が発火し、warrant も明示意図に基づく。
/// 書き込みが効くようになると補正で閉じ、以後は発火しない。
#[test]
fn explicit_intent_drift_correction_fires_with_warrant() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .block_writes(true)
        .user_set_open(false)
        .advance_ms(50)
        .observe(Source::ImmCross);
    assert!(
        h.ime.state().open,
        "書き込みが効かないので IME は開いたまま"
    );
    let fire = h
        .drift_fires
        .last()
        .unwrap_or_else(|| panic!("明示意図のある drift は発火する\n{}", h.trace()));
    assert!(!fire.drift.desired && fire.drift.observed, "{fire:?}");
    assert!(fire.explicit_intent);
    let write = h
        .writes
        .iter()
        .rev()
        .find(|w| w.origin == WriteOrigin::DriftCorrection)
        .expect("drift 補正の書き込み命令");
    assert!(
        matches!(
            write.warrant.as_ref().map(|w| &w.basis),
            Some(WarrantBasis::ExplicitUserIntent(_))
        ),
        "明示意図に基づく warrant が下りる: {write:?}"
    );

    let fires_before = h.drift_fires.len();
    h.block_writes(false)
        .advance_ms(50)
        .observe(Source::ImmCross)
        .advance_ms(50)
        .observe(Source::ImmCross)
        .advance_ms(500)
        .observe(Source::ImmCross);
    assert!(!h.ime.state().open, "補正で閉じた\n{}", h.trace());
    assert_eq!(
        h.drift_fires.len(),
        fires_before + 1,
        "書き込みが効いた後は、閉じた観測で drift が解消して再発火しない\n{}",
        h.trace()
    );
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
}

/// BUG-173 追補2（Opus 発火削減 D4）: conv ビットからの推測（`ConvOpenInference`）だけを根拠にした
/// 「開」の観測は、明示意図（ユーザーの IME OFF）と食い違っても drift correction を発火させない。
/// GJI×TsfNative では IME を閉じても conv の NATIVE が残るので、推測は `VK_IME_OFF` を何度送っても
/// 収束せず、ユーザー自身の OFF の直後に同じ OFF を重ねて送っていた（不具合報告 01M3NJ784NKMH120HM6QGKF7W7）。
/// 実 API の読み取り（`ImmCross`）由来の乖離は従来どおり発火する（`explicit_intent_drift_correction_fires_with_warrant`）。
#[test]
fn conv_inference_alone_does_not_fire_drift_correction_despite_explicit_off() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    // 実 API の観測は無い（読めない窓を想定）。conv 推測しか開閉の手掛かりが無い。
    h.advance_ms(300)
        .user_set_open(false)
        .advance_ms(50)
        .observe_value(Source::ConvInference, true)
        .advance_ms(50)
        .observe_value(Source::ConvInference, true);
    assert!(
        h.drift_fires.is_empty(),
        "conv 推測だけの「開」では drift 補正を発火しない\n{}",
        h.trace()
    );
}

/// Opus round2 R2-2: conv 推測を根拠から外すのは「選ぶ前に除外」する形でなければならない。選んだ後に捨てる形だと、
/// 同じ Medium の `ObserverPoll` が検出した正当な drift まで、後から来た conv 推測が「最新の信頼できる観測」になって覆い隠す。
#[test]
fn conv_inference_does_not_mask_a_poll_drift() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    h.advance_ms(300)
        .block_writes(true)
        .user_set_open(false)
        .advance_ms(50)
        .observe(Source::Poll);
    assert!(
        !h.drift_fires.is_empty(),
        "実 API（Poll）の観測は drift を検出する\n{}",
        h.trace()
    );
    // 時間の経過だけでも drift 判定が走るので、conv 推測を記録する直前の件数を取ってから観測する。
    h.advance_ms(50);
    let fires_before_conv = h.drift_fires.len();
    h.observe_value(Source::ConvInference, true);
    assert!(
        h.drift_fires.len() > fires_before_conv,
        "後から conv 推測が来ても、Poll の drift は隠れない\n{}",
        h.trace()
    );
}

/// ADR-212 P6: drift correction は**ユーザーの明示操作の書き込みが届かなかったときの再試行だけ**を残す。
/// 明示意図が無い（フォーカス変更で意図がクリアされた後）まま、実 IME が awase の知らない経路で開いた
/// （`desired_open` は古い OFF のまま）ときは、awase が自分の推測（古い desired）を実 IME へ書き戻さない。
/// （以前は閾値 `DRIFT_CORRECTION_THRESHOLD_MS` を過ぎると `Poll` の観測を根拠に OFF を書いていた。
/// 実機の過去ログでは drift 補正の書き込み 169 件が全て「IME を OFF にする」方向だった。）
#[test]
fn adr212_p6_drift_without_explicit_intent_does_not_write() {
    let mut h = Harness::start(Setup::imm_cross(state(false, CONV_HIRAGANA)));
    h.advance_ms(300)
        .user_set_open(false)
        .advance_ms(100)
        .focus_change()
        .advance_ms(100)
        .external_set_open(true)
        .advance_ms(600)
        .observe(Source::Poll)
        .advance_ms(1500)
        .observe(Source::Poll)
        .advance_ms(1500)
        .observe(Source::Poll);
    assert!(
        h.drift_fires.is_empty(),
        "明示意図が無いときは drift 補正で書かない\n{}",
        h.trace()
    );
    assert!(
        !h.writes
            .iter()
            .any(|w| w.origin == WriteOrigin::DriftCorrection),
        "drift 補正の書き込みが無い\n{}",
        h.trace()
    );
}

// ── クセ Q3: 読み戻し遅延（実 IME の変化が IMM 再読に現れるまで数十 ms かかる）──────────────
//
// 実測（`tuning.rs` の `MODE_KEY_PASS_REREAD_MS` の注記、`tools/e2e/ime_key_matrix/mode_key_pass_timeline.py`）:
// ATOK プリセットのモードキー通過後、変化が再読に現れるまで min21 / median33 / max62ms。
// 11ms 後の 1 回は古い状態を読んだ。ここでは中央値の 33ms を使う。擬似 IME の真の状態は
// 変わっていて、観測だけが遅れる（`PseudoIme::set_readback_lag_ms`）。

/// クセ Q3 の遅延（ms、実測の中央値）。
const READBACK_LAG_MS: u64 = 33;
/// 押下の何 ms 後に読むか（実測で「古い状態を読んだ」例の 11ms）。
const STALE_READ_AT_MS: u64 = 11;

/// 擬似 IME 自体の確認: 状態変更の直後は変更前を読み、遅延が過ぎると真の状態を読む。
#[test]
fn q3_pseudo_ime_readback_is_stale_until_lag_elapses() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.ime.set_readback_lag_ms(Some(READBACK_LAG_MS));
    h.advance_ms(100).key(VK_MUHENKAN); // 半角英数・入力中でない → 閉（格子 `on-c10-none|muhenkan`）
    assert!(!h.ime.state().open, "真の状態は閉\n{}", h.trace());
    assert!(
        h.ime.read_state().open,
        "直後の読み取りは古い（開）\n{}",
        h.trace()
    );
    h.advance_ms(STALE_READ_AT_MS);
    assert!(h.ime.read_state().open, "11ms 後もまだ古い\n{}", h.trace());
    h.advance_ms(READBACK_LAG_MS - STALE_READ_AT_MS);
    assert!(
        !h.ime.read_state().open,
        "遅延が過ぎたら真の状態\n{}",
        h.trace()
    );
}

/// クセ Q3 × 無変換で閉じた直後の Poll: 古い「開」を読んでも、明示意図が無いので awase は
/// IME を書き換えず（P1・drift 補正の書き込みなし）、遅延後の観測で belief が真の状態へ収束する。
#[test]
fn q3_stale_poll_right_after_muhenkan_close_does_not_write_and_converges() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.ime.set_readback_lag_ms(Some(READBACK_LAG_MS));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_MUHENKAN)
        .advance_ms(STALE_READ_AT_MS)
        .observe(Source::Poll) // 古い「開」を読む
        .advance_ms(READBACK_LAG_MS * 3)
        .observe(Source::ImmCross) // 遅延後の読み取りは閉
        .advance_ms(100);
    assert!(!h.ime.state().open, "無変換で閉じたまま\n{}", h.trace());
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert!(
        !h.writes
            .iter()
            .any(|w| w.origin == WriteOrigin::DriftCorrection),
        "古い読み取りで drift 補正の書き込みが出ない\n{}",
        h.trace()
    );
    assert_ok(&h, belief_matches_truth_at_end(&h));
}

/// 対照: 遅延が無ければ（従来の擬似 IME）、同じ列で最初の Poll から真の状態を読む。
#[test]
fn q3_control_without_lag_reads_truth_immediately() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_MUHENKAN)
        .advance_ms(STALE_READ_AT_MS)
        .observe(Source::Poll)
        .advance_ms(100);
    assert!(!h.ime.read_state().open);
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
}

// ── ハーネスの忠実度(ADR-209 が約束した MS-IME の格子、ADR-223 の is_japanese_ime) ──────────────

/// MS-IME プリセット（GJI、`msime.json`）の格子で、ATOK と同じ「ひらがなキー→打鍵→Enter 確定」を通す。
/// 格子（`on-c10-none|hiragana`=`ON/0x19`、`on-c19-typing|enter`=`ON/0x19/確定`）と、awase の MSIME 同梱表で予測が一致し続けること。
#[test]
fn msime_grid_hiragana_key_then_typing_and_commit() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)).with_grid(Grid::GjiMsime));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_HIRAGANA)
        .advance_ms(30)
        .key(VK_K)
        .advance_ms(80)
        .key(VK_ENTER)
        .advance_ms(300)
        .observe(Source::ImmCross)
        .advance_ms(100);
    assert_eq!(h.ime.state(), state(true, CONV_HIRAGANA), "{}", h.trace());
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
}

/// `is_japanese_ime=false`（入力言語が日本語でない、ADR-223）のとき、推測・補正に基づく書き込み(`issue_open_warrant`)は下りない。
/// `explicit_intent_drift_correction_fires_with_warrant` と同じ列で、drift 補正の書き込みが出ず IME は開いたまま。
#[test]
fn non_japanese_ime_blocks_drift_correction_write() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_HIRAGANA)));
    h.set_japanese_ime(false)
        .advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .block_writes(true)
        .user_set_open(false)
        .advance_ms(50)
        .observe(Source::ImmCross)
        .advance_ms(500)
        .observe(Source::ImmCross);
    assert!(
        !h.writes
            .iter()
            .any(|w| w.origin == WriteOrigin::DriftCorrection),
        "日本語 IME でなければ drift 補正の書き込みは出ない\n{}",
        h.trace()
    );
}

const VK_HANKAKU_ZENKAKU: u16 = 0xF3;

/// 実 Chrome × GJI の外部クローズ（Q4、BUG-172）の入力列: 読めない窓で、明示意図（ユーザーの IME ON）を持った状態から、
/// 他プロセスが目印なしの 0xF3 を注入して IME を閉じる。その後 refresh が prefetch 済みの読みを取り込む。
fn q4_external_close_sequence(h: &mut Harness) {
    h.advance_ms(100)
        .user_set_open(true)
        .advance_ms(100)
        .prefetch_read() // 閉じる前の読み（基準値 = 開）
        .advance_ms(400)
        .external_injected_key(VK_HANKAKU_ZENKAKU)
        .advance_ms(20)
        .prefetch_read() // 注入の 20ms 後の refresh（監視窓の中）
        .advance_ms(500)
        .prefetch_read()
        .advance_ms(1500)
        .prefetch_read();
}

/// Q4 の症状（ADR-205 無し＝v2.0.0 より前）: IME は閉じているのに awase の belief は開のまま（観測が 0 件）。
/// 明示意図が実状態を上書きし続けるので、Engine は ON のまま `kiu` が出る（BUG-172）。
#[test]
fn q4_external_close_is_not_observed_without_adr205() {
    let mut h = Harness::start(Setup::imm32_unavailable(state(true, CONV_HIRAGANA)));
    q4_external_close_sequence(&mut h);
    assert!(
        !h.ime.state().open,
        "IME は外部の注入で閉じている\n{}",
        h.trace()
    );
    let last = h.steps.last().expect("ステップあり");
    assert!(
        last.effective_open && last.explicit_intent,
        "監視窓が無いと belief は開のまま（Q4）\n{}",
        h.trace()
    );
}

/// ADR-205 の回帰: 同じ入力列で、監視窓の中の 1→0 の読みを実状態として取り込み、意図を捨て desired を揃える。
/// awase は IME を開け直さない（所有者方針: 書き込みなし）。
#[test]
fn q4_external_close_is_followed_with_adr205_and_never_reopened() {
    let writes_before_close = 1; // user_set_open(true) の書き込み 1 件だけ
    let mut h = Harness::start(
        Setup::imm32_unavailable(state(true, CONV_HIRAGANA)).with_external_close_watch(true),
    );
    q4_external_close_sequence(&mut h);
    assert!(
        !h.ime.state().open,
        "IME は閉じたまま（開け直さない）\n{}",
        h.trace()
    );
    let last = h.steps.last().expect("ステップあり");
    assert!(
        !last.effective_open && !last.explicit_intent && !h.desired_open(),
        "belief は閉へ追随し、意図は捨てられる\n{}",
        h.trace()
    );
    assert_eq!(
        h.writes.len(),
        writes_before_close,
        "追随のあとに awase の書き込みは無い（開け直さない）\n{}",
        h.trace()
    );
}

// ── クセ Q-key-latency-gji: 物理キー押下から変化が観測されるまでの遅れ（Q3 より桁が大きい）──────
//
// 実測（ADR-176:84-86、n=9: 247/277/321/341/362/391/529/1687/2295ms）。擬似 IME では Q3 と同じ
// `set_readback_lag_ms` の値を変えて表す。中央値 362ms と最大 2295ms を使う。

/// 実測 n=9 の中央値（ms）。
const KEY_LATENCY_MEDIAN_MS: u64 = 362;
/// 実測の最大（ms）。
const KEY_LATENCY_MAX_MS: u64 = 2295;

/// 中央値の遅れの間に古い「開」を読み続けても（Poll を 100ms ごと）、明示意図が無ければ drift 補正で
/// IME を書き換えず、遅れが過ぎた後の観測で belief が真の状態へ収束する。
#[test]
fn key_latency_median_stale_polls_do_not_write_and_converge() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.ime.set_readback_lag_ms(Some(KEY_LATENCY_MEDIAN_MS));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_MUHENKAN);
    for _ in 0..3 {
        h.advance_ms(100).observe(Source::Poll); // 古い「開」
    }
    h.advance_ms(KEY_LATENCY_MEDIAN_MS)
        .observe(Source::ImmCross) // 遅れ後は閉
        .advance_ms(100);
    assert!(!h.ime.state().open, "無変換で閉じたまま\n{}", h.trace());
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert!(
        !h.writes
            .iter()
            .any(|w| w.origin == WriteOrigin::DriftCorrection),
        "古い読み取りで drift 補正の書き込みが出ない\n{}",
        h.trace()
    );
    assert_ok(&h, belief_matches_truth_at_end(&h));
}

/// 実測最大（2.3 秒）の遅れでも同じ: 500ms ごとの古い読みで書き込まず、遅れ後に収束する。
#[test]
fn key_latency_max_stale_polls_do_not_write_and_converge() {
    let mut h = Harness::start(Setup::imm_cross(state(true, CONV_ALNUM)));
    h.ime.set_readback_lag_ms(Some(KEY_LATENCY_MAX_MS));
    h.advance_ms(100)
        .observe(Source::ImmCross)
        .advance_ms(200)
        .key(VK_MUHENKAN);
    for _ in 0..4 {
        h.advance_ms(500).observe(Source::Poll);
    }
    h.advance_ms(KEY_LATENCY_MAX_MS)
        .observe(Source::ImmCross)
        .advance_ms(100);
    assert!(!h.ime.state().open, "無変換で閉じたまま\n{}", h.trace());
    assert_ok(&h, p1_no_warranted_write_without_intent(&h));
    assert_ok(&h, belief_matches_truth_at_end(&h));
}
