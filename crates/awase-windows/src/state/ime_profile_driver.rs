//! `AppImeProfile` ごとに独立した capability 宣言型ドライバ（Phase 1a/1b/1c、未配線）。
//!
//! # 位置づけ
//!
//! [ADR-081](../../../../docs/adr/081-per-profile-capability-driver-decomposition.md)
//! の Phase 1 計画（確定）に沿った実装。ADR-080 Phase 1 の `state/ime_actuation.rs`
//! と同じ位置づけで、**既存のランタイム（`runtime/`・`ime_controller.rs`）への配線は
//! まだ行わない**（配線は実機ソーク必須の Phase 1d のスコープ）。`state/app_ime_policy.rs`
//! の `AppImePolicy`（1 struct + `from_profile` の match 式でプロファイル別 *データ*
//! だけを分岐する現行実装）を、プロファイルごとに独立した trait 実装へ分離する:
//!
//! - Phase 0: [`ImmCrossDriver`]（`ImmCross`/`Plain`/`Unknown`）。
//! - Phase 1a: [`Imm32UnavailableDriver`]（Chrome/Edge/UWP）。
//! - Phase 1b: [`TsfNativeDriver`]（WezTerm/Windows Terminal）。
//! - Phase 1c: [`driver_for`] レジストリ + contract test スイート（不変条件5件）。
//!
//! GJI 横断性（`active_ime_kind` = GJI/MS-IME は実行時観測で profile 軸と直交）は
//! design B に従い、各ドライバが [`ImeProfileDriver::uses_gji_direct`] を静的に宣言
//! するだけとし、GJI 直接制御そのものは共有機構
//! [`state/gji_direct_mechanism.rs`](super::gji_direct_mechanism) に1箇所集約する
//! （ドライバ内に GJI 分岐を複製しない）。
//!
//! # 依存の制約（Linux でテスト可能にするため）
//!
//! `tsf/output.rs::ColdReason` や `state/ime_decision_view.rs::ImeControlView`、
//! さらに `crate::vk`・`state/key_sequence_policy.rs`（`ime_key_for`）は本クレートでは
//! `#[cfg(windows)]` 限定型であり、これらに依存すると本モジュールも windows 限定になり
//! Linux 上の `cargo test -p awase-windows --lib` でテストできなくなる（ADR-065 が
//! `state/` に課した非依存の原則）。そのため [`ImeProfileDriver`] のメソッドは windows
//! 限定型を一切取らない・返さない形に分解している:
//!
//! - `ColdReason`（`tsf/output.rs` の 10 variant。`FocusChange`/
//!   `ReinjectConfirmKey`/`CtrlKeyBypass` 等）ではなく `is_confirm_key: bool` と
//!   `long_idle: bool` の 2 引数に分解する（`docs/known-bugs.md`
//!   BUG-01/BUG-21/BUG-40 が実際に参照する軸はこの 2 つ）。
//! - IME 開閉は「どの VK を送るか」ではなく [`ImeOpenMechanism`]（cross-process API か
//!   共有の冪等キー委譲か）だけをドライバに宣言させる。具体 VK 選択（GJI か MS-IME か）
//!   は実行時観測 `active_ime_kind` に依存する動的判断であり、windows-gated な
//!   `ime_key_for` SSOT がランタイム境界（Phase 1d）で解決する — ドライバ／共有機構は
//!   VK を複製しない。

use std::time::Duration;

use super::ime_actuation::FeedbackPolicy;
use super::ime_event::{ImePolicyProfile, ObservationSource};

/// IME を開閉する実際の機構。
///
/// `ImmCross`（通常 Win32 アプリ）は `ImmSetOpenStatus` の cross-process 呼び出しで
/// 開閉する（VK を一切送らない）。`Imm32Unavailable`/`TsfNative` は冪等 VK
/// （`VK_IME_ON`/`VK_IME_OFF`/`VK_DBE_HIRAGANA` 等）を送るが、**どの VK を送るかは
/// `active_ime_kind`（GJI / MS-IME、実行時観測）で決まる**。ADR-081 の design B
/// では profile 軸（静的）と IME 種別軸（動的）を分離し、ドライバは GJI / MS-IME の
/// 動的分岐を持たない。したがってドライバは「クロスプロセス API か、共有の冪等キー
/// 機構へ委譲するか」だけを静的に宣言し、具体 VK の選択（GJI か MS-IME か）は
/// ランタイム側の合成（Phase 1d）と共有機構（`state/gji_direct_mechanism.rs`・
/// 既存 `MsImeDirectStrategy`）に委ねる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeOpenMechanism {
    /// `ImmSetOpenStatus` 相当のクロスプロセス API 呼び出し（VK 送信なし）。
    CrossProcessApi,
    /// 共有の冪等キー機構（`active_ime_kind` に応じ GJI 直接制御 or MS-IME 直接制御）へ
    /// 委譲する。具体 VK はドライバではなくランタイム側の合成が決める。
    ///
    /// GJI が active な場合の経路は `state/gji_direct_mechanism.rs` の共有機構に
    /// 排他集約されており、その入口は `uses_gji_direct()` の宣言でゲートされる
    /// （ADR-081 Phase 1c 不変条件5）。送信キーは既存 SSOT
    /// `state/key_sequence_policy.rs::ime_key_for` が握るため、ドライバが VK を
    /// 直書きすることはない。
    SharedImeKeyDispatch,
}

/// プロファイルごとに独立した capability を宣言するドライバ。
///
/// [ADR-081](../../../../docs/adr/081-per-profile-capability-driver-decomposition.md)
/// Phase 1 計画の実装。各メソッドは「このプロファイルではこの値を返す」という
/// 静的な宣言のみを持ち、観測値に依存する動的判断（`shadow_on` のスキップ判定、
/// conv ビットに応じた ROMAN 事前設定、`active_ime_kind` に応じた GJI/MS-IME の
/// 具体 VK 選択等）は含まない — それらは Phase 1d（ランタイム配線）で
/// `ImeControlView` を受け取る別メソッドや共有機構との合成として扱う。
///
/// `default_feedback`（`FeedbackPolicy::Read`/`Blind` の選択）は Phase 1 で本 trait に
/// 含める（全面移行に向けたドライバへの責務移設）。値そのものは ADR-080 の
/// `state/app_ime_policy.rs::AppImePolicy::default_feedback` を SSOT とし、drift 回帰
/// テスト（`tests` の `assert_policy_parity`）で両者の一致を固定する。全面移行
/// （Phase 1e）で `AppImePolicy` を撤去した時点でドライバ側が SSOT になる。
///
/// `Sync` を要求するのは、Phase 1 で `ime_controller.rs::ImeController::strategies`
/// （`[&'static dyn ImeOpenStrategy; N]`、`ImeOpenStrategy: Sync`）と同型に
/// `[&'static dyn ImeProfileDriver; N]` として保持する想定のため。
pub trait ImeProfileDriver: Sync {
    /// 物理 KANJI / VK_F3 / VK_F4 を awase が完全所有するか。
    ///
    /// `true` のとき、物理 KANJI イベントはアプリに渡さない
    /// （`state/app_ime_policy.rs::AppImePolicy::owns_physical_kanji` 相当）。
    fn owns_physical_kanji(&self) -> bool;

    /// GJI が active IME のとき、共有 GJI 直接制御機構
    /// （`state/gji_direct_mechanism.rs`）を経由するか。
    ///
    /// ADR-081 design B の中核。profile 軸（静的）と `active_ime_kind` 軸（GJI /
    /// MS-IME、動的）を分離し、ドライバは GJI / MS-IME の**動的分岐を持たず**、
    /// 「共有 GJI 機構を使うか」だけを静的に宣言する。`true` を宣言したドライバ
    /// だけが `GjiDirectMechanism::access_for` から token を得られる（不変条件5）。
    ///
    /// `ImmCross` は cross-process API（`ImmSetOpenStatus`）が一次経路のため
    /// `false`。GJI フォールバックの合成はランタイム（Phase 1d）の責務。
    fn uses_gji_direct(&self) -> bool;

    /// このドライバが IME を OFF→ON にする経路を持つか（不変条件1 の一般化）。
    ///
    /// `true` のドライバは stale `ObservedEisu` 救済を対で配線する義務を負う
    /// （`state/eisu_recovery.rs` の対称性テストの一般化。`.claude/rules/
    /// ime-belief-architecture.md`）。
    fn has_ime_on_path(&self) -> bool;

    /// IME-ON 経路に対し stale `ObservedEisu` 救済を対で配線すると宣言するか。
    ///
    /// [`has_ime_on_path`](Self::has_ime_on_path) が `true` のとき、これも `true` で
    /// なければならない（contract test が強制、不変条件1）。Phase 1 では静的宣言
    /// レベルの検証で足り、実際の救済ロジック配線は Phase 1d のスコープ。
    fn stale_eisu_recovery_paired(&self) -> bool;

    /// このプロファイルの actuation デフォルト feedback（収束確認）方針。
    ///
    /// `state/app_ime_policy.rs::AppImePolicy::default_feedback` と parity を取る
    /// （読み戻し可能な `ImmCross` 系は `Read`、構造的に読み戻せない
    /// `Imm32Unavailable` / `TsfNative` は有界終端の `Blind`）。`Blind` give-up 後に
    /// observation を書かない不変条件（不変条件3、BUG-33 型）は
    /// `state/ime_actuation.rs::decide_actuation_action` が SSOT。
    fn default_feedback(&self) -> FeedbackPolicy;

    /// フォーカス変更後、observer を信頼できるようになるまでの待ち時間 (ms)。
    ///
    /// `state/app_ime_policy.rs::AppImePolicy::focus_settle_ms` 相当。
    fn focus_settle_ms(&self) -> u64;

    /// cold-start probe の探索予算 (ms)。
    ///
    /// `docs/known-bugs.md` BUG-01（WezTerm/TsfNative の `eager_settle_ms`）・
    /// BUG-21（Chrome/Imm32Unavailable の重症度別予算）が実際に依存していた
    /// 2 軸（確定キー起因か・long idle か）だけを引数に取る。`ColdReason`
    /// （`tsf/output.rs`、2026-07-25 時点で `FocusChange`/`SetOpenTrue`/
    /// `SetOpenFalse`/`NativeF2Consumed`/`PassthroughConfirmKey`/
    /// `ReinjectConfirmKey`/`SymbolVkSent`/`F2NonTsf`/`RawTsfLiteralRecovery`/
    /// `CtrlKeyBypass` の 10 種。旧 `SessionExpired` は 2026-07-06 の到達不能
    /// パス監査で撤去済み）を完全に再現する解像度は Phase 1 で `ColdReason`
    /// 自体を windows-gated メソッドとして追加する際に扱う。
    fn probe_budget_ms(&self, is_confirm_key: bool, long_idle: bool) -> u64;

    /// IME を開く/閉じる実際の機構と送信内容。
    ///
    /// 返すのは開閉機構そのものだけで、`ImmCross` 経路が IME ON 時に行う
    /// conv ビット依存の ROMAN 補完（`set_ime_romaji_mode`。MS-IME + ImmCross の
    /// かなモード残留でエンジンが停止する既知バグの対策）は含まない。これは
    /// 現在の conv mode という観測値に依存する動的判断であり、trait doc に記した
    /// 通り Phase 1（`ImeControlView` を受け取るメソッド）のスコープ。
    fn ime_open_mechanism(&self, open: bool) -> ImeOpenMechanism;
}

/// `ImePolicyProfile::ImmCross` 相当のドライバ（通常 Win32 アプリ、LINE/Qt 等）。
///
/// [[feedback_immcross_owns_kanji]]（ImmCross アプリには物理 IME キーを見せない
/// 設計原則）に基づき `owns_physical_kanji = true`。IMM32 クロスプロセス制御が
/// 使えるため cold-start probe は不要（`ImmSetOpenStatus` は同期的に完了する）。
#[derive(Debug, Clone, Copy, Default)]
pub struct ImmCrossDriver;

impl ImeProfileDriver for ImmCrossDriver {
    fn owns_physical_kanji(&self) -> bool {
        // Step 1/1b の決定（`state/app_ime_policy.rs` と同値）。
        true
    }

    fn uses_gji_direct(&self) -> bool {
        // ImmCross は `ImmSetOpenStatus` の cross-process 制御が一次経路。
        // GJI 検出時のフォールバック合成はランタイム（Phase 1d）の責務であり、
        // 本ドライバは共有 GJI 機構を静的な一次経路としては宣言しない。
        false
    }

    fn has_ime_on_path(&self) -> bool {
        true
    }

    fn stale_eisu_recovery_paired(&self) -> bool {
        // IME-ON 経路を持つため救済を対で配線すると宣言する（不変条件1）。
        true
    }

    fn default_feedback(&self) -> FeedbackPolicy {
        // `AppImePolicy::from_profile(ImmCross).default_feedback` と parity。
        // 読み戻し可能（`ImmGetOpenStatus`）なので `Read`。
        FeedbackPolicy::Read {
            source: ObservationSource::ImmGetOpenStatus,
            deadline: Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS),
        }
    }

    fn focus_settle_ms(&self) -> u64 {
        // `AppImePolicy::from_profile(ImmCross)` と同値。
        100
    }

    fn probe_budget_ms(&self, _is_confirm_key: bool, _long_idle: bool) -> u64 {
        // ImmCross（通常 Win32、IMM32 クロスプロセス）は `ImmSetOpenStatus` が
        // 同期的に完了するため、TSF/GJI 向けの cold-start probe（BUG-01/BUG-21 が
        // 対象とする非同期初期化待ち）自体が不要。0 は「probe しない」を意味する。
        0
    }

    fn ime_open_mechanism(&self, _open: bool) -> ImeOpenMechanism {
        // `ime_controller.rs::ImmCrossProcessStrategy::apply` は
        // `set_ime_open_cross_process`（`ImmSetOpenStatus` の cross-process 版）
        // を呼ぶだけで VK を送らない。
        ImeOpenMechanism::CrossProcessApi
    }
}

/// `Blind` feedback で actuation を打ち切るまでの試行回数。
///
/// `state/app_ime_policy.rs::IME_ACTUATION_BLIND_MAX_ATTEMPTS`（現状 private）と
/// 同値。全面移行（Phase 1e）で `AppImePolicy` が撤去されると本値が SSOT になる。
/// それまでは各ドライバの `default_feedback` parity テストが両者の drift を検出する。
const BLIND_MAX_ATTEMPTS: u32 = 5;

/// `Imm32Unavailable` / `TsfNative` 共通の `Blind` feedback を構築する。
///
/// 読み戻し手段が構造的に無いプロファイルは有限回で必ず打ち切る（BUG-33 型の
/// 収束偽装を型で防ぐ ADR-080 の方針）。`AppImePolicy::from_profile` の対応アームと
/// parity を取る。
fn blind_feedback() -> FeedbackPolicy {
    FeedbackPolicy::Blind {
        max_attempts: BLIND_MAX_ATTEMPTS,
        backoff: Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS),
    }
}

/// `ImePolicyProfile::Imm32Unavailable` 相当のドライバ（Chrome/Edge/UWP 等）。
///
/// IMM32 クロスプロセス制御が使えず、`active_ime_kind` に応じた冪等 VK
/// （GJI: `VK_IME_ON`/`VK_IME_OFF`、MS-IME: `VK_DBE_HIRAGANA`/`VK_IME_OFF`）で
/// 制御する。GJI 分岐はドライバ内に複製せず共有機構（`state/gji_direct_mechanism.rs`）
/// へ委譲する（design B）。読み戻し不能のため feedback は `Blind`。
#[derive(Debug, Clone, Copy, Default)]
pub struct Imm32UnavailableDriver;

impl ImeProfileDriver for Imm32UnavailableDriver {
    fn owns_physical_kanji(&self) -> bool {
        // Step 1/1b の決定: Chrome/Edge も awase が物理 KANJI を所有
        // （`AppImePolicy::from_profile(Imm32Unavailable)` と同値）。
        true
    }

    fn uses_gji_direct(&self) -> bool {
        // IMM32 が使えないため、GJI が active な場合は共有 GJI 機構が一次経路。
        true
    }

    fn has_ime_on_path(&self) -> bool {
        true
    }

    fn stale_eisu_recovery_paired(&self) -> bool {
        // Imm32Unavailable は stale ObservedEisu 循環デッドロックの主戦場
        // （`state/eisu_recovery.rs` の背景参照）。救済の対配線は必須（不変条件1）。
        true
    }

    fn default_feedback(&self) -> FeedbackPolicy {
        blind_feedback()
    }

    fn focus_settle_ms(&self) -> u64 {
        // Chrome/Edge は GJI/IMM が信頼できないため settle 長め
        // （`AppImePolicy::from_profile(Imm32Unavailable)` と同値）。
        500
    }

    fn probe_budget_ms(&self, _is_confirm_key: bool, _long_idle: bool) -> u64 {
        // Chrome は F2 受信後に composition context を非同期初期化する（BUG-02）。
        // その待機予算は既存 tuning SSOT `CHROME_GJI_REINIT_CONFIRM_MS`（IME ON→
        // NATIVE 確認 300ms）を参照する。ColdReason×idle の解像度別エスカレーション
        // （`CHROME_PROBE_*` 系の歴史、`.claude/rules/tuning-constants.md`）は
        // windows-gated な `ColdReason` メソッドとして Phase 1d で精緻化する。
        crate::tuning::CHROME_GJI_REINIT_CONFIRM_MS
    }

    fn ime_open_mechanism(&self, _open: bool) -> ImeOpenMechanism {
        // 具体 VK（GJI か MS-IME か）はランタイム合成が決める。ドライバは委譲のみ宣言。
        ImeOpenMechanism::SharedImeKeyDispatch
    }
}

/// `ImePolicyProfile::TsfNative` 相当のドライバ（WezTerm/Windows Terminal 等）。
///
/// TSF ネイティブアプリ。`VK_DBE_HIRAGANA` + TSF probe が必要で、cold-start に
/// composition context の非同期初期化待ちがある（BUG-01）。TSF が KANJI を正しく
/// 処理するため物理 KANJI は**通す**（`owns_physical_kanji=false`）。読み戻し不能の
/// ため feedback は `Blind`。
#[derive(Debug, Clone, Copy, Default)]
pub struct TsfNativeDriver;

impl ImeProfileDriver for TsfNativeDriver {
    fn owns_physical_kanji(&self) -> bool {
        // WezTerm 等は TSF が KANJI を正しく処理するため通す
        // （`AppImePolicy::from_profile(TsfNative)` と同値）。
        false
    }

    fn uses_gji_direct(&self) -> bool {
        // TSF アプリでも GJI が active なら共有 GJI 機構経由（VK_IME_ON/OFF は
        // TSF 層で処理される、`ime_controller.rs::GjiDirectStrategy` doc 参照）。
        true
    }

    fn has_ime_on_path(&self) -> bool {
        true
    }

    fn stale_eisu_recovery_paired(&self) -> bool {
        true
    }

    fn default_feedback(&self) -> FeedbackPolicy {
        blind_feedback()
    }

    fn focus_settle_ms(&self) -> u64 {
        // `AppImePolicy::from_profile(TsfNative)` と同値。
        200
    }

    fn probe_budget_ms(&self, _is_confirm_key: bool, long_idle: bool) -> u64 {
        // WezTerm は F2 受信後 TSF composition context 初期化に実測 ~300–936ms
        // かかる（BUG-01）。~7s 以上の idle 後は特に長い（`MEDIUM_IDLE_PROBE_MS` の
        // 実測根拠）。いずれも既存 tuning SSOT を参照し新規定数を導入しない。
        if long_idle {
            crate::tuning::MEDIUM_IDLE_PROBE_MS
        } else {
            crate::tuning::WARMUP_GRACE_MS
        }
    }

    fn ime_open_mechanism(&self, _open: bool) -> ImeOpenMechanism {
        ImeOpenMechanism::SharedImeKeyDispatch
    }
}

// ── ドライバレジストリ ────────────────────────────────────────────

static IMM_CROSS_DRIVER: ImmCrossDriver = ImmCrossDriver;
static IMM32_UNAVAILABLE_DRIVER: Imm32UnavailableDriver = Imm32UnavailableDriver;
static TSF_NATIVE_DRIVER: TsfNativeDriver = TsfNativeDriver;

/// `ImePolicyProfile` に対応する静的ドライバを返す。
///
/// `ImmCross`/`Plain`/`Unknown` は [`ImmCrossDriver`] に集約する（ADR-081 Phase 1
/// 計画で確定。ドライバ数を増やすこと自体が「同期すべき箇所」を増やし反証データの
/// 失敗モードを増やすため。**分類 enum `ImePolicyProfile` 自体は統合しない** —
/// driver へのマッピングだけを collapse し、分類情報は将来の分割判断のため残す）。
#[must_use]
pub fn driver_for(profile: ImePolicyProfile) -> &'static dyn ImeProfileDriver {
    match profile {
        ImePolicyProfile::ImmCross | ImePolicyProfile::Plain | ImePolicyProfile::Unknown => {
            &IMM_CROSS_DRIVER
        }
        ImePolicyProfile::Imm32Unavailable => &IMM32_UNAVAILABLE_DRIVER,
        ImePolicyProfile::TsfNative => &TSF_NATIVE_DRIVER,
    }
}

/// contract test / 将来のランタイム走査用の全ドライバ一覧（重複なし、1 実装 1 要素）。
pub const ALL_DRIVERS: &[&dyn ImeProfileDriver] = &[
    &IMM_CROSS_DRIVER,
    &IMM32_UNAVAILABLE_DRIVER,
    &TSF_NATIVE_DRIVER,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::app_ime_policy::AppImePolicy;

    #[test]
    fn imm_cross_driver_owns_physical_kanji() {
        // focus_settle と同様、直値ではなく SSOT（AppImePolicy）と照合して
        // owns_physical_kanji の drift も検出できるようにする。
        use crate::state::app_ime_policy::AppImePolicy;
        use crate::state::ime_event::ImePolicyProfile;
        assert!(ImmCrossDriver.owns_physical_kanji());
        assert_eq!(
            ImmCrossDriver.owns_physical_kanji(),
            AppImePolicy::from_profile(ImePolicyProfile::ImmCross).owns_physical_kanji
        );
    }

    #[test]
    fn imm_cross_driver_focus_settle_matches_app_ime_policy() {
        // ドライバ分離がデータ面で既存ポリシーと食い違わないことの回帰テスト。
        // 直値ではなく SSOT（AppImePolicy）を直接参照することで、AppImePolicy 側の
        // focus_settle_ms が実測に基づき変更されたらこのテストが失敗して drift を
        // 検出できるようにする（直値固定だと SSOT が動いても気付けない）。
        use crate::state::app_ime_policy::AppImePolicy;
        use crate::state::ime_event::ImePolicyProfile;
        assert_eq!(
            ImmCrossDriver.focus_settle_ms(),
            AppImePolicy::from_profile(ImePolicyProfile::ImmCross).focus_settle_ms
        );
    }

    #[test]
    fn imm_cross_driver_never_needs_cold_start_probe() {
        // is_confirm_key/long_idle の全組み合わせで probe 予算 0（probe しない）。
        for is_confirm_key in [false, true] {
            for long_idle in [false, true] {
                assert_eq!(
                    ImmCrossDriver.probe_budget_ms(is_confirm_key, long_idle),
                    0,
                    "is_confirm_key={is_confirm_key} long_idle={long_idle}"
                );
            }
        }
    }

    #[test]
    fn imm_cross_driver_opens_via_cross_process_api_not_vk() {
        assert_eq!(
            ImmCrossDriver.ime_open_mechanism(true),
            ImeOpenMechanism::CrossProcessApi
        );
        assert_eq!(
            ImmCrossDriver.ime_open_mechanism(false),
            ImeOpenMechanism::CrossProcessApi
        );
    }

    /// 型シグネチャレベルの見積り（ADR-081 Phase 0 記録 2節）が実装可能であることの
    /// 確認: trait オブジェクトとして扱えること（Phase 1 で複数ドライバを配列に
    /// 保持する設計、`ime_controller.rs::ImeController::strategies` と同型）。
    #[test]
    fn driver_is_usable_as_trait_object() {
        // strategies (`[&'static dyn ImeOpenStrategy; N]`, `ImeOpenStrategy: Sync`)
        // と同型に保てること = `dyn ImeProfileDriver` が Sync であることを固定する
        // （trait の Sync 境界が外れたらこのテストが壊れて気付ける）。
        fn assert_sync<T: Sync + ?Sized>() {}
        assert_sync::<dyn ImeProfileDriver>();
        let drivers: [&dyn ImeProfileDriver; 1] = [&ImmCrossDriver];
        assert!(drivers[0].owns_physical_kanji());
    }

    // ── Phase 1a/1b: 新ドライバ × AppImePolicy SSOT の parity 回帰 ──
    //
    // `focus_settle_ms` / `owns_physical_kanji` / `default_feedback` は直値ではなく
    // AppImePolicy を直接参照して drift を検出する（Phase 0 の ImmCross と同方針）。

    /// `(driver, profile)` の parity を AppImePolicy SSOT と照合するヘルパ。
    fn assert_policy_parity(driver: &dyn ImeProfileDriver, profile: ImePolicyProfile) {
        let policy = AppImePolicy::from_profile(profile);
        assert_eq!(
            driver.owns_physical_kanji(),
            policy.owns_physical_kanji,
            "owns_physical_kanji drift ({profile:?})"
        );
        assert_eq!(
            driver.focus_settle_ms(),
            policy.focus_settle_ms,
            "focus_settle_ms drift ({profile:?})"
        );
        assert_eq!(
            driver.default_feedback(),
            policy.default_feedback,
            "default_feedback drift ({profile:?})"
        );
    }

    #[test]
    fn imm32_unavailable_driver_matches_app_ime_policy() {
        assert_policy_parity(&Imm32UnavailableDriver, ImePolicyProfile::Imm32Unavailable);
    }

    #[test]
    fn tsf_native_driver_matches_app_ime_policy() {
        assert_policy_parity(&TsfNativeDriver, ImePolicyProfile::TsfNative);
    }

    #[test]
    fn imm_cross_driver_matches_app_ime_policy_default_feedback() {
        // Phase 0 は owns/focus_settle のみ照合していた。default_feedback も SSOT と照合。
        assert_eq!(
            ImmCrossDriver.default_feedback(),
            AppImePolicy::from_profile(ImePolicyProfile::ImmCross).default_feedback
        );
    }

    #[test]
    fn registry_maps_profiles_to_expected_driver_capabilities() {
        // `driver_for` が各 profile を正しいドライバへ写像し、集約
        // （ImmCross/Plain/Unknown → ImmCrossDriver）が期待通りであることを
        // capability（owns_physical_kanji / uses_gji_direct）で確認する。
        for profile in [
            ImePolicyProfile::ImmCross,
            ImePolicyProfile::Plain,
            ImePolicyProfile::Unknown,
        ] {
            let d = driver_for(profile);
            assert!(d.owns_physical_kanji(), "{profile:?} → ImmCross");
            assert!(
                !d.uses_gji_direct(),
                "{profile:?} → ImmCross (cross-process 一次)"
            );
        }
        assert!(driver_for(ImePolicyProfile::Imm32Unavailable).uses_gji_direct());
        assert!(!driver_for(ImePolicyProfile::TsfNative).owns_physical_kanji());
    }

    #[test]
    fn probe_budget_references_existing_tuning_ssot() {
        // 新規タイミング定数を導入せず既存 tuning SSOT を参照していることの回帰
        // （`.claude/rules/tuning-constants.md`: 実測なしのエスカレーション禁止）。
        assert_eq!(
            Imm32UnavailableDriver.probe_budget_ms(false, false),
            crate::tuning::CHROME_GJI_REINIT_CONFIRM_MS
        );
        assert_eq!(
            TsfNativeDriver.probe_budget_ms(false, true),
            crate::tuning::MEDIUM_IDLE_PROBE_MS
        );
        assert_eq!(
            TsfNativeDriver.probe_budget_ms(false, false),
            crate::tuning::WARMUP_GRACE_MS
        );
    }

    // ── Phase 1c: contract test スイート（不変条件5件、ADR-081 Phase 1 計画） ──
    //
    // Phase 1 の位置づけ上、5件とも「未配線の型レベル」の検証。実行時の完全な保証は
    // Phase 1d のランタイム配線が担う。

    /// 不変条件1: IME-ON 経路を持つドライバは stale `ObservedEisu` 救済を対で持つ
    /// （`state/eisu_recovery.rs` の対称性テストの一般化）。
    #[test]
    fn invariant_1_ime_on_path_drivers_pair_eisu_recovery() {
        for d in ALL_DRIVERS {
            if d.has_ime_on_path() {
                assert!(
                    d.stale_eisu_recovery_paired(),
                    "IME-ON 経路を持つドライバは stale ObservedEisu 救済を対で配線する義務がある。\
                     Imm32Unavailable 系で engine 永久 inactive の循環デッドロックを防ぐ。"
                );
            }
        }
    }

    /// 不変条件2: `owns_physical_kanji()==true` のドライバは物理 KANJI を漏らさない。
    ///
    /// 静的宣言レベルの一貫性チェック: 所有ドライバの IME 制御機構が
    /// cross-process API（VK 送信なし）か共有の冪等キー委譲であること。どちらも
    /// 物理 KANJI トグル（`VK_KANJI`）そのものを送る variant ではない。
    /// 具体 VK が `VK_KANJI` に落ちないことは `ime_key_for` の SSOT 特性であり、
    /// windows 側の `tests/ime_key_sequence_golden.rs` が別途固定している（本 Linux
    /// テストでは windows-gated な `crate::vk` を参照できないため機構宣言の一貫性で表す）。
    #[test]
    fn invariant_2_kanji_owning_drivers_use_non_kanji_mechanism() {
        for d in ALL_DRIVERS {
            if !d.owns_physical_kanji() {
                continue;
            }
            for open in [true, false] {
                assert!(
                    matches!(
                        d.ime_open_mechanism(open),
                        ImeOpenMechanism::CrossProcessApi | ImeOpenMechanism::SharedImeKeyDispatch
                    ),
                    "所有ドライバが物理 KANJI を送る機構を宣言してはならない"
                );
            }
        }
    }

    /// 不変条件3: `Blind` give-up 後に observation を書かない（BUG-33 型の観測偽装防止）。
    ///
    /// `decide_actuation_action`（SSOT）を各ドライバの `default_feedback` で駆動し、
    /// `Blind` ドライバが `max_attempts` で厳密に `GiveUp` へ終端することを固定する。
    /// `GiveUp` が observation を書かない不変は `ime_actuation.rs` の型 doc が保証する。
    #[test]
    fn invariant_3_blind_drivers_terminate_without_writing_observation() {
        use crate::state::ime_actuation::{
            decide_actuation_action, ActuationAction, FeedbackPolicy,
        };
        for d in ALL_DRIVERS {
            match d.default_feedback() {
                FeedbackPolicy::Blind { max_attempts, .. } => {
                    assert_eq!(
                        decide_actuation_action(d.default_feedback(), max_attempts),
                        ActuationAction::GiveUp,
                        "Blind は max_attempts で GiveUp（observation 非書き込み）に終端する"
                    );
                }
                FeedbackPolicy::Read { .. } => {
                    // Read は試行回数で打ち切らない。
                    assert_eq!(
                        decide_actuation_action(d.default_feedback(), u32::MAX),
                        ActuationAction::Send
                    );
                }
            }
        }
    }

    /// 不変条件4: belief を actuate 抜きで ON にする高速パスは必ず `GjiFsm` を同期させる。
    ///
    /// 型レベル: GJI の IME-ON キーを取り出す唯一の経路（共有機構の `actuate`）が
    /// `GjiFsmSync::OnImeOn` 同期義務と分離不能であることを、各 GJI ドライバ経由で確認する。
    #[test]
    fn invariant_4_gji_ime_on_is_inseparable_from_fsm_sync() {
        use crate::state::gji_direct_mechanism::{GjiDirectMechanism, GjiFsmSync};
        for d in ALL_DRIVERS {
            let Some(access) = GjiDirectMechanism::access_for(*d) else {
                continue;
            };
            let on = GjiDirectMechanism::actuate(&access, true);
            assert_eq!(
                on.fsm_sync,
                GjiFsmSync::OnImeOn,
                "IME-ON キーは OnImeOn 同期義務と束ねられていなければならない（BUG-18/22）"
            );
        }
    }

    /// 不変条件5: GJI 機構の状態遷移はどのドライバ経由で呼ばれても同一の `GjiFsm`
    /// 同期を通る（= `uses_gji_direct()==true` を宣言しないドライバから共有 GJI 機構を
    /// 呼び出せない）。
    #[test]
    fn invariant_5_gji_access_gated_by_static_declaration() {
        use crate::state::gji_direct_mechanism::GjiDirectMechanism;
        for d in ALL_DRIVERS {
            assert_eq!(
                GjiDirectMechanism::access_for(*d).is_some(),
                d.uses_gji_direct(),
                "共有 GJI 機構へのアクセスは uses_gji_direct() の静的宣言でゲートされる"
            );
        }
    }

    /// design B の一貫性: `uses_gji_direct()` を宣言するドライバは共有キー機構へ
    /// 委譲する（`SharedImeKeyDispatch`）。cross-process API 経路と混同しないこと。
    #[test]
    fn uses_gji_direct_implies_shared_key_dispatch_mechanism() {
        for d in ALL_DRIVERS {
            if d.uses_gji_direct() {
                for open in [true, false] {
                    assert_eq!(
                        d.ime_open_mechanism(open),
                        ImeOpenMechanism::SharedImeKeyDispatch,
                        "uses_gji_direct ドライバの機構は共有キー委譲でなければならない"
                    );
                }
            }
        }
    }
}
