// pub mod が必要: lib.rs の pub use crate::state::{...} 再エクスポートチェーンを支える。
// unreachable_pub lint はこの再エクスポートパターンを認識できないため抑制する。
#![allow(unreachable_pub)]

// 下位の crate が順に再公開する（`awase-key-effect` が `awase-windows-base` の `state` を再公開する）ので、最上位の 1 つだけを並べる。
// 両方を並べると名前がすべて重なって「未使用の import」になる（nightly の lint が指摘）。
pub use awase_key_effect::state::*;

// ── 純粋サブモジュール（全プラットフォーム）──────────────────────────────────────
pub mod belief;
pub use belief::*;

pub mod conv_mode;
pub use conv_mode::ConvModeAuthority;
pub use conv_mode::ConvModeMgr;
pub use conv_mode::{ConvActuationOutcome, ConvModeTarget, ConvMutationReason};

pub mod app_ime_policy;
pub mod indicator_glyph;
pub mod indicator_glyph_seeds;
pub mod indicator_glyph_store;
// ADR-265 段階 1: mozc-readable の共有メモリ(48 バイト、version 2)の解釈。受け入れ条件(magic・version・seq・pid・active)と
// journal の記録への変換。Win32 を呼ばない(Linux でテストする)。belief には入れない。
pub mod ime_actuation;
pub mod ime_update;
pub mod mozc_shm;
// ADR-208 L0: `runtime/transport.rs::PhysicalKeyDisposition::plan` の本体（配送判断の核）を挙動を変えずに
// 移した。ungated にして、`explicit_press` の全列挙テストが本番と同じ判断コードを Linux で呼べるようにする。
pub mod physical_disposition;
// ADR-208 L0: 明示キー押下 1 回の配送（物理の届け方と awase の書き込み）を既存の判断の合成として決める純粋関数と、
// その全列挙テストの入力型。ungated にして全列挙テストを Linux で回すため、本番の呼び出し元の一部
// （`runtime/key_pipeline.rs` の `select_shadow_intent`/`shadow_noop_write_target`/`ShadowIntentKind`）が
// Windows 専用である非 Windows ビルドでは、それらが未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod explicit_press;
// ADR-208 L1: 「この押下で既に書いた」の予約（`last_written_press`）と同一押下の二重送信の防御・衝突の優先順位（純粋）。
pub mod press_ledger;
// ADR-089 §2.3/§2.6: Actuation の型状態チェーンと再試行 episode。ungated（走査
// 規則を Linux で全数テストするため）。実 write は Windows 側の
// `MechanismWriter` 実装（`ime_controller.rs`）が担う。
pub mod actuation_chain;
// ADR-081 Phase 1a/1b/1c の「コード構造についての契約宣言」とそのテストのみのモジュール
// （本番の呼び出し元は無い）。Phase 1d/1e は ADR-090 §2.F で凍結した。app_ime_policy と
// 同じ ungated パターンで Linux 上の `cargo test -p awase-windows --lib` から実行できるようにし、
// 両ターゲットで dead_code を許可する。
#[allow(dead_code)]
pub mod ime_profile_driver;
// ADR-089 §2.4: `GjiFsm` 同期義務（INV-42/43）。ADR-081 Phase 1c の共有 GJI 機構
// として起こされ、Phase B（2026-08-12）で `ActuationReceipt` + `GjiSyncSink` へ
// 置き換えて本番（`platform.rs::on_ime_applied`）へ配線した。
pub mod gji_direct_mechanism;
// 純粋関数モジュール。テストを Linux CI で実行できるよう ungated にするが、唯一の
// 呼び出し元 runtime/key_pipeline.rs は #[cfg(windows)] のため非 Windows では未使用に
// なる（ADR-065 と同じ局所抑制パターン）。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod conv_classify;
// 純粋関数モジュール（conv_classify と同じ ungated パターン）。呼び出し元は
// #[cfg(windows)] の runtime/ のみ。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod eisu_candidate;
pub mod eisu_recovery;
pub mod snapshot_input_mode;
// ADR-163 TH1a: `crate::ime::ConvAfterOpen` の ungated ミラー。将来の
// actuation 決定出力が Windows-gated 型を state 層へ持ち込まないための境界型。
pub mod conv_after_open;
// ADR-163 TH1b-1: IME actuation の「何を送るか」を Win32 I/O から切り離した
// 純粋決定関数。本番は `ime_controller.rs`/`journal.rs`（TH1b-2b で配線済み）から呼ばれる。
// ungated なので、呼び出し元が Windows 専用の非 Windows ビルドでは未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod ime_actuation_decision;
// FCIS（ADR-229）: `executor.rs::dispatch_ime_set_open` の判断（gate・claim 結果・ImmCross 先頭か）の純粋部分。
pub mod ime_set_open_plan;
// ADR-163 Part B（TH1c）: attempt単位の決定点ジャーナルスキーマとcrate内
// 再生ハーネス。ime_actuation_decisionと同じ「追加のみ、本番経路への配線は
// 別タスク（TH1d/TH1e）」のモジュール。
pub mod actuation_decision_record;
// ADR-089 §2.1/§2.2: open 観測の evidence 型（プール分離 + データ witness）。
pub mod evidence;
// drift correction の判定本体（旧 `ImeStateHub::check_drift_correction` の本体）。
// ungated（Linux の `tests/closed_loop_scenarios.rs` から呼ぶため）。本番の呼び出し元は
// `#[cfg(windows)]` の `platform_state.rs` だけなので、非 Windows では未使用になる。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod drift_correction;
pub mod drift_plan;
pub mod force_guard;
pub mod ime_event;
// 呼び出し元（`imm.rs`・`observer/ime_observer.rs`）は `#[cfg(windows)]` のため、非 Windows では未使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod imm_evidence;
// `ModeKeyPassLatch`の一部メソッド（`note_awase_write`/`window_remaining_ms`/`expiry_wait_ms`/
// `align_after_expired`）は`platform_state.rs`（`#[cfg(windows)]`）からしか呼ばれない。
// alt_impersonation等と同じ「純粋判定をLinuxでテストできるようにする」ungatedパターン。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod mode_key_pass;
// ADR-205: 外部注入 IME キー直後の監視窓（`ImeStateHub::follow_external_change` からしか呼ばれない純粋な状態機械）。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod external_change_watch;
// FCIS F1: `runtime/ime_refresh.rs::ir_decide_read_strategy` の純粋な核（読み取り方針の決定と理由）。
#[cfg(test)]
pub(crate) mod exhaustive_seq;
pub mod ime_model;
pub mod ime_read_strategy;
// ADR-087 Phase 1' 試験実装。app_ime_policy/ime_profile_driver と同じ ungated
// パターンで Linux 上の `cargo test -p awase-windows --lib` から実行できるように
// する。runtime への配線（既存 `ImeModel.last_intent` との統合）はまだ無い
// （配線は ADR-087 Phase 3 のスコープ、§7 round3 S4 参照）。
pub mod intent_store;
// ADR-087 Phase 2'/3 試験実装。intent_store と同じ ungated・未配線パターン。
pub mod open_warrant;
pub use ime_model::AppliedImeState;
pub mod input_barrier;
pub mod observation_store;
pub mod probe_admission;
// フォーカス変更時に belief を applied へ写してよいかの可否（純粋。ADR-262 段階 A）。本番の呼び出し元 runtime/ は #[cfg(windows)]、Linux ではテストだけが使う。
pub mod applied_mirror_plan;
pub mod transition;
// ADR-245 追補 2 R1: 復元の書き込み直後の読み直しと、それより前に始まった ImmCrossProbe の読みの棄却の判定。
pub mod resume_observation;

// 純粋関数モジュール（conv_classify と同じ ungated パターン）。呼び出し元は
// #[cfg(windows)] の ime_controller/runtime のみ。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod key_sequence_policy;

pub mod ime_event_log;
