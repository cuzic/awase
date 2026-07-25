//! P2-1: IME キー戦略のキャラクタライゼーション（ゴールデン）テスト。
//!
//! # 目的
//! `ime_controller.rs` の 4 戦略と `output/probe_io.rs` の warmup キー分岐は、実機での
//! 試行錯誤と revert を繰り返して現在の姿に落ち着いた（`git log --oneline | grep -i revert`
//! で 24 件）。今後これらを宣言的テーブル（KeySequencePolicy, P2-2/P2-3）へリファクタする
//! 前提として、**現在の挙動をキャラクタライゼーションテストとして固定**する。
//!
//! # 何を実行し、何をドキュメントとして固定するか
//! `ImeOpenStrategy::apply()` は `SendInput` 等の Win32 副作用を持つため、テストから実行できない
//! （`ProbeIo` と違い注入可能なシーム seam がない）。一方 `is_applicable()` は純粋関数である。
//! そこで本テストは:
//!
//! - **実行して固定**: 戦略「選択」（どの戦略が最初に `is_applicable` を返すか）。
//!   `awase_windows::ime_controller::characterize_strategy` 経由で現状のコードを呼び出す。
//! - **ソース由来ドキュメントとして固定**: 各戦略が実際に送信するキー列・outcome、および
//!   probe_io の warmup キー選択。これらは `apply()` を実行できないため、ソース該当箇所と
//!   実機で確定した根拠コミットハッシュを注記した固定テキストとしてゴールデンに埋め込む。
//!
//! # 実行方法
//! Windows 専用コードのため実行は Windows 上でのみ可能:
//! ```text
//! cargo test --target x86_64-pc-windows-gnu --test ime_key_sequence_golden -p awase-windows
//! ```
//! Linux ホストではコンパイル確認のみ（`--no-run`）。ゴールデンを再生成/更新するには:
//! ```text
//! UPDATE_GOLDEN=1 cargo test --test ime_key_sequence_golden -p awase-windows
//! ```
#![cfg(windows)]

use std::path::PathBuf;

use awase_windows::ime_controller::characterize_strategy;
use awase_windows::state::ime_event::ImePolicyProfile;
use awase_windows::state::ime_profile_driver::{driver_for, ImeOpenMechanism};

/// 戦略選択テーブルの走査対象。`(active_ime_kind ラベル, active_gji, profile)`。
///
/// `active_ime_kind` は 2 値（GoogleJapaneseInput / MicrosoftIme）のみ。
/// `AppImeProfile` は 3 値（Standard / Imm32Unavailable / TsfNative）。
const COMBOS: &[(&str, bool, &str)] = &[
    ("GJI", true, "Standard"),
    ("GJI", true, "Imm32Unavailable"),
    ("GJI", true, "TsfNative"),
    ("MS-IME", false, "Standard"),
    ("MS-IME", false, "Imm32Unavailable"),
    ("MS-IME", false, "TsfNative"),
];

const HEADER: &str = "\
# IME キー戦略 キャラクタライゼーションゴールデン (P2-1)
#
# 生成元: crates/awase-windows/tests/ime_key_sequence_golden.rs
# このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成すること。
#
# ── 戦略選択テーブル ──────────────────────────────────────────────
# ImeController の is_applicable のみを評価した実行結果（apply は未実行）。
# dispatch: apply = 通常経路 / apply_skipping_imm = async IMM が Failed を返した後の経路。
# 列: active_ime_kind <TAB> profile <TAB> dispatch <TAB> selected_strategy
";

const KEY_DOC: &str = "\
# ── 戦略別 送信キー / outcome ──────────────────────────────────────
# ソース: crates/awase-windows/src/ime_controller.rs（apply は Win32 副作用のため未実行）。
# 各戦略が open=true / open=false で送るキーと ImeOpenOutcome。根拠コミットを注記する。
#
# ImmCrossProcess (is_applicable: profile==Standard):
#   ON/OFF ともに ImmSetOpenStatus クロスプロセス呼び出し = ime::set_ime_open_cross_process(open)。
#   成功→Applied / 失敗→Failed（次の適用可能戦略へフォールスルー）。VK キーは送らない。
#   MS-IME + ImmCross + open かつ belief!=ObservedKana のとき、直前に set_ime_romaji_mode()
#   （ROMAN ビット付与）で JIS かな入力化けを防ぐ。
#
# GjiDirect (is_applicable: active_ime_kind==GoogleJapaneseInput):
#   ON  → shadow_on なら送信せず AlreadyMatched（VK_IME_ON no-op 見込みでスキップ）、
#         さもなくば VK_IME_ON (0x16) = ime::post_gji_ime_on() → Applied。
#   OFF → VK_IME_OFF (0x1A) = ime::post_gji_ime_off() → Applied。
#   VK_IME_ON/OFF は Windows 標準の冪等キーで GJI が TSF 層でネイティブ処理する。
#   GJI+TsfNative の OFF は旧 VK_KANJI から VK_IME_OFF へ移行済み（489cdf1）。
#   （履歴: adb856c で一時 VK_KANJI フォールバックへ戻したが 489cdf1 で VK_IME_OFF に再修正）
#
# MsImeDirect (is_applicable: active_ime_kind==MicrosoftIme && !can_use_imm32_cross_process()):
#   ON  → 現 conv が KATAKANA ビット立ちなら送信せず AlreadyMatched（conv 破壊防止）、
#         さもなくば（belief!=ObservedKana のとき set_ime_romaji_mode() 後）
#         VK_DBE_HIRAGANA (0xF2) = ime::post_ms_ime_on() → Applied。
#   OFF → VK_IME_OFF (0x1A) = ime::post_ime_off_direct()（DirectInput へ、冪等）→ Applied。
#   OFF が VK_IME_OFF（冪等）である根拠: 48a667a。VK_DBE_ALPHANUMERIC は半角英数（IME-ON）に
#   留まるため不可、VK_KANJI はトグルのため不可。
#   （履歴: 9c3f11e→668a131 revert、be3b056 で一時 VK_KANJI、48a667a で VK_IME_OFF に確定）
#
# KanjiToggle (is_applicable: 常に true / 最終フォールバック):
#   ON/OFF ともに VK_KANJI トグル = ime::post_kanji_toggle_to_focused() → FallbackSent。
#   冪等でないトグルのため already_matched 判定はせず送信する。GJI/MS-IME 環境では前段が
#   処理するため稀にしか到達しない。
";

const WARMUP_DOC: &str = "\
# ── warmup / sacrificial キー選択 ─────────────────────────────────
# ソース: crates/awase-windows/src/output/probe_io.rs（impl ProbeIo for Output）。
# dispatch_probe_actions が呼ぶ ProbeIo メソッドと、Output 実装が送るキー。キーは Output 側に
# ハードコードされており分岐は主に TransmitTarget（Chrome / Tsf）で切り替わる。
#
# SendFreshF2         → send_fresh_f2(): VK_DBE_HIRAGANA (0xF2) down/up。
#                       Medium/Long cold（forces_prepend_f2）では send_extra_f2() で F2×2 連続。
# RawTsfLiteralRecovery → set_raw_literal(backs, romaji) で BS×backs + 再送を予約し mark_cold。
#                         consecutive>0 のときは再送せず BS のみ（give-up）。
#
# 捨て駒キー機構（StartSacrificialWarmup/SacrificialResend、VK_A+BS・VK_IME_OFF→ON、
# SacrificialWarmupCoro/ImeOffOnWarmupFsm）は 2026-07-18 に物理削除した。実機ソークで
# per-VK confirm（TransmitSingleVk、上記とは別の cold-start パス）のみで安全網として
# 十分と確認済み（docs/known-bugs.md 参照）。send_chrome_gji_reinit_and_poll() は
# Unicode injection mode の long-cold GJI 再初期化として probe_io.rs に残っているが、
# ProbeAction 経由ではなく Output::send_f22_f21_reinit() から直接呼ばれる。
";

/// 戦略選択テーブル（実行結果）+ ドキュメント section を連結したゴールデン本文を生成する。
fn build_report() -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    for &(active, active_gji, profile) in COMBOS {
        for (dispatch, skip_imm) in [("apply", false), ("apply_skipping_imm", true)] {
            let selected = characterize_strategy(active_gji, profile, skip_imm);
            out.push_str(&format!("{active}\t{profile}\t{dispatch}\t{selected}\n"));
        }
    }
    out.push('\n');
    out.push_str(KEY_DOC);
    out.push('\n');
    out.push_str(WARMUP_DOC);
    out
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("ime_key_sequences.txt")
}

#[test]
fn ime_key_strategy_selection_matches_golden() {
    let actual = build_report();
    let path = golden_path();

    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    if update || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        if update {
            eprintln!("[golden] UPDATE_GOLDEN: wrote {}", path.display());
            return;
        }
        // 初回ブートストラップ: ゴールデンが無ければ生成して成功扱いにする。
        eprintln!("[golden] bootstrapped {}", path.display());
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        actual, expected,
        "IME キー戦略の挙動がゴールデンと乖離した。意図的な変更なら \
         UPDATE_GOLDEN=1 cargo test --test ime_key_sequence_golden -p awase-windows \
         で再生成し、差分の根拠を確認すること。"
    );
}

/// 選択ロジックの不変条件をスモークテストとして固定する（ゴールデン破損時の一次診断用）。
#[test]
fn strategy_selection_invariants() {
    // Standard プロファイルは常に ImmCrossProcess が先取りする（GJI/MS-IME 問わず）。
    assert_eq!(
        characterize_strategy(true, "Standard", false),
        "ImmCrossProcess"
    );
    assert_eq!(
        characterize_strategy(false, "Standard", false),
        "ImmCrossProcess"
    );

    // GJI 検出時は（ImmCross を除けば）常に GjiDirect。
    assert_eq!(
        characterize_strategy(true, "Imm32Unavailable", false),
        "GjiDirect"
    );
    assert_eq!(characterize_strategy(true, "TsfNative", false), "GjiDirect");
    assert_eq!(characterize_strategy(true, "Standard", true), "GjiDirect");

    // MS-IME × 非 Standard は MsImeDirect。
    assert_eq!(
        characterize_strategy(false, "Imm32Unavailable", false),
        "MsImeDirect"
    );
    assert_eq!(
        characterize_strategy(false, "TsfNative", false),
        "MsImeDirect"
    );

    // MS-IME × Standard で IMM を飛ばすと最終フォールバック KanjiToggle まで落ちる。
    assert_eq!(
        characterize_strategy(false, "Standard", true),
        "KanjiToggle"
    );
}

// ── ADR-081 Phase 1d shadow parity（Linux 側前倒し）─────────────────
//
// ADR-081 の `ImeProfileDriver`（`state/ime_profile_driver.rs`）は Phase 1c 時点で
// 未配線（実 apply 経路には一切繋がっていない）。Phase 1d（実機ソーク必須のランタイム
// 配線）そのものはこのサンドボックスでは実行できないが、「ドライバの静的宣言だけから
// 一次経路の戦略名を導出し、既存 `characterize_strategy` と一致するか」は Win32 の
// 副作用を一切伴わない read-only 比較であり、Linux（cross-compile での --no-run）でも
// 固定できる。実 apply には触れない。
//
// スコープ外（意図的）: `skip_imm=true`（ImmCross 失敗後の GJI/MsImeDirect/KanjiToggle
// フォールバック合成）は Phase 1d のランタイム配線が担う領域であり、`ImeProfileDriver`
// はまだそれを表現するメソッドを持たない（ADR-081 本文の「GJI フォールバックの合成は
// ランタイム（Phase 1d）の責務」を参照）。よってこの比較は `skip_imm=false` の一次経路
// のみを対象とする。

/// `characterize_strategy` の `profile: &str` 引数と `ImePolicyProfile` の対応。
///
/// `AppImeProfile`（windows-gated、3値: Standard/Imm32Unavailable/TsfNative）と
/// `ImePolicyProfile`（`state/`、Linux 可視、5値）は別の型だが、`driver_for` の
/// レジストリが `ImmCross`/`Plain`/`Unknown` を単一の `ImmCrossDriver` に集約している
/// ため、`"Standard"` は `ImmCross` に写せば十分（`Plain`/`Unknown` は
/// `AppImeProfile` に対応する値がなく本テストの対象外）。
fn app_profile_to_policy_profile(profile: &str) -> ImePolicyProfile {
    match profile {
        "Standard" => ImePolicyProfile::ImmCross,
        "Imm32Unavailable" => ImePolicyProfile::Imm32Unavailable,
        "TsfNative" => ImePolicyProfile::TsfNative,
        other => panic!("unknown profile: {other}"),
    }
}

/// `ImeProfileDriver` の静的宣言のみから、`characterize_strategy` の一次経路
/// （`skip_imm=false`）に相当する戦略名を導出する read-only アダプタ。
///
/// `apply()` を含む本番経路（`ime_controller.rs::CONTROLLER`）からは一切参照されない
/// テスト専用のシームであり、Win32 副作用を持たない。
fn driver_shadow_strategy_name(active_gji: bool, profile: &str) -> &'static str {
    let driver = driver_for(app_profile_to_policy_profile(profile));
    match driver.ime_open_mechanism(true) {
        ImeOpenMechanism::CrossProcessApi => "ImmCrossProcess",
        ImeOpenMechanism::SharedImeKeyDispatch => {
            if active_gji && driver.uses_gji_direct() {
                "GjiDirect"
            } else {
                "MsImeDirect"
            }
        }
    }
}

/// ADR-081 Phase 1d の shadow parity を前倒しで固定する。
///
/// `ImeProfileDriver` ベースの導出（`driver_shadow_strategy_name`）と、現行の
/// `ImeController` 戦略選択（`characterize_strategy`、`skip_imm=false`）が
/// `COMBOS`（golden と同じ全組み合わせ）で一致することを検証する。不一致は
/// ADR-081 の想定するドライバ配線（Phase 1d）が現行実装と drift していることを
/// 意味するため、golden 更新ではなく設計の見直しが必要。
#[test]
fn driver_shadow_parity_matches_characterize_strategy_primary_path() {
    for &(active_label, active_gji, profile) in COMBOS {
        let expected = characterize_strategy(active_gji, profile, false);
        let actual = driver_shadow_strategy_name(active_gji, profile);
        assert_eq!(
            actual, expected,
            "ADR-081 driver-based 選択が現行 characterize_strategy から drift: \
             active_ime_kind={active_label} profile={profile} (skip_imm=false)"
        );
    }
}
