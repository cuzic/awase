// pub mod が必要: lib.rs の pub use crate::state::{...} 再エクスポートチェーンを支える。
// unreachable_pub lint はこの再エクスポートパターンを認識できないため抑制する。
#![allow(unreachable_pub)]

// ── TickMs ─────────────────────────────────────────────────────────────────────

/// `GetTickCount64` 由来のミリ秒タイムスタンプを表すニュータイプ。
///
/// state/ 層が `hook::current_tick_ms()` を直接呼び出す代わりに、
/// 呼び出し元（runtime 層）からタイムスタンプを注入するために使う。
/// これにより state/ が hook 実装に依存しない純粋な型になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize)]
pub struct TickMs(pub u64);

impl TickMs {
    /// `self - base` を飽和演算で計算して返す。
    #[must_use]
    pub const fn saturating_sub(self, base: u64) -> u64 {
        self.0.saturating_sub(base)
    }
}

// ── 純粋サブモジュール（全プラットフォーム）──────────────────────────────────────
pub mod belief;
pub use belief::*;

pub mod hook_state;
pub use hook_state::*;

pub(crate) mod conv_mode;
pub use conv_mode::ConvModeAuthority;
#[cfg(windows)]
pub(crate) use conv_mode::ConvModeMgr;
#[cfg(windows)]
pub(crate) use conv_mode::{ConvActuationOutcome, ConvModeTarget, ConvMutationReason};

// 純粋関数モジュール（conv_classify と同じ ungated パターン）。唯一の呼び出し元
// hook.rs は #[cfg(windows)] のため非 Windows では未使用になる。BUG-41
// （decide_alt_impersonation の KeyUp 状態クリア漏れ）が Windows 実機で初めて
// テストが実行されるまで発見されなかったことの再発防止として、hook.rs から移設。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod alt_impersonation;
// ADR-106 決定1: `ApplyGeneration` 専用アロケータ。`ImeEventLog.next_seq` から
// 独立させ、fence 用の識別子が別目的の数を借用する問題（原因A）を解消する。
// ungated（Linux で `allocate()` の単調増加・折り返し・wire エンコード往復を
// 全数テストするため）。`ImeEvent`/`ImeTransition`（ungated）が `ApplyGeneration`
// を保持するため非 Windows でも使用される。
pub mod generation;
pub use generation::{ApplyGeneration, GenerationAllocator};
pub mod app_ime_policy;
// hook.rs (#[cfg(windows)]) の唯一の呼び出し元。alt_impersonation と同じ
// 「純粋判定を Linux でテストできるようにする」移設パターン。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod app_suppression;
// hook.rs (#[cfg(windows)]) の唯一の呼び出し元。alt_impersonation と同じ
// 「純粋判定を Linux でテストできるようにする」移設パターン。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod win_key_guard;
// ADR-082「第一歩」: EventOrigin/Generation/EventSource の最小実装。既存コードへの
// 配線はまだ無い（モジュール冒頭のスコープ節参照）。
pub mod event_origin;
pub mod ime_actuation;
// ADR-089 §2.3/§2.6: Actuation の型状態チェーンと再試行 episode。ungated（走査
// 規則を Linux で全数テストするため）。実 write は Windows 側の
// `MechanismWriter` 実装（`ime_controller.rs`）が担う。
pub mod actuation_chain;
// ADR-081 Phase 0 試験実装（未配線）。app_ime_policy と同じ ungated パターンで
// Linux 上の `cargo test -p awase-windows --lib` から実行できるようにする。
// 呼び出し元は Windows/非 Windows どちらにも現時点で存在しない
// （配線は Phase 1 のスコープ）ため、両ターゲットで dead_code を許可する。
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
pub mod eisu_recovery;
// ADR-089 §2.1/§2.2: open 観測の evidence 型（プール分離 + データ witness）。
pub mod evidence;
pub mod force_guard;
pub mod ime_event;
// ADR-089 §2.8「K 軸の型」。`caps(p, k)` の導入（Phase C）に先立ち、Linux で
// 全数テストできる ungated な IME 種別を置く。変換は `tsf/observer.rs` の
// `From<ActiveImeKind>` 1 箇所のみ。
pub mod ime_kind;
pub mod ime_model;
// ADR-087 Phase 1' 試験実装。app_ime_policy/ime_profile_driver と同じ ungated
// パターンで Linux 上の `cargo test -p awase-windows --lib` から実行できるように
// する。runtime への配線（既存 `ImeModel.last_intent` との統合）はまだ無い
// （配線は ADR-087 Phase 3 のスコープ、§7 round3 S4 参照）。
pub mod intent_store;
// ADR-087 Phase 2'/3 試験実装。intent_store と同じ ungated・未配線パターン。
pub mod open_warrant;
#[cfg(windows)]
pub(crate) use ime_model::AppliedImeState;
pub mod focus_resync_policy;
pub mod input_barrier;
// output/types.rs から移設（InjectionHint 依存の From 実装のみ output/ に残す）。
// 唯一の ungated 呼び出し元は tsf::gji_fsm。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod injection_mode;
pub mod observation_store;
pub(crate) mod post_bypass;
pub mod probe_admission;
pub(crate) mod scoped_latch;
pub mod transition;

// ── Windows 専用サブモジュール ───────────────────────────────────────────────────
#[cfg(windows)]
pub mod platform_state;
#[cfg(windows)]
pub use platform_state::PlatformState;

#[cfg(windows)]
pub(crate) mod ime_decision_view;
#[cfg(windows)]
pub(crate) use ime_decision_view::{ControlLog, FocusFacts, ImeControlView, ObservedState};

#[cfg(windows)]
pub(crate) mod key_sequence_policy;

#[cfg(windows)]
pub mod ime_event_log;
