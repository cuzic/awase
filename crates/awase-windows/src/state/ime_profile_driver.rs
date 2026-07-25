//! `AppImeProfile` ごとに独立した capability 宣言型ドライバ（試験実装、未配線）。
//!
//! # 位置づけ
//!
//! [ADR-081](../../../../docs/adr/081-per-profile-capability-driver-decomposition.md)
//! の Phase 0（検証専用）で Go と判定された範囲の最小実装。ADR-080 Phase 1 の
//! `state/ime_actuation.rs` と同じ位置づけで、**既存のランタイム（`runtime/`・
//! `ime_controller.rs`）への配線はまだ行わない**。`state/app_ime_policy.rs`
//! の `AppImePolicy`（1 struct + `from_profile` の match 式でプロファイル別
//! *データ* だけを分岐する現行実装）を、プロファイルごとに独立した trait 実装へ
//! 分離した場合の実測サンプルとして、`ImePolicyProfile::ImmCross` 相当の
//! [`ImmCrossDriver`] のみを実装する。
//!
//! # 依存の制約（Linux でテスト可能にするため）
//!
//! `tsf/output.rs::ColdReason` や `state/ime_decision_view.rs::ImeControlView`
//! は本クレートでは `#[cfg(windows)]` 限定型であり、これらに依存すると本
//! モジュールも windows 限定になり Linux 上の `cargo test -p awase-windows --lib`
//! でテストできなくなる（ADR-065 が `state/` に課した非依存の原則）。そのため
//! [`ImeProfileDriver`] のメソッドは windows 限定型を一切取らない・返さない形に
//! 分解している:
//!
//! - `ColdReason`（`tsf/output.rs` の 10 variant。`FocusChange`/
//!   `ReinjectConfirmKey`/`CtrlKeyBypass` 等）ではなく `is_confirm_key: bool`
//!   + `long_idle: bool` の 2 引数に分解する
//!   （`docs/known-bugs.md` BUG-01/BUG-21/BUG-40 が実際に参照する軸はこの 2 つ）。
//! - 送信 VK は `awase::types::VkCode`（windows 非依存のニュータイプ）で返す。
//!   `ImmCross` プロファイルは VK ではなく `ImmSetOpenStatus`（cross-process API）
//!   で IME を開閉するため、[`ImeOpenMechanism`] で「VK を送るか API を呼ぶか」
//!   自体をドライバに選ばせる（`ime_controller.rs` の
//!   `ImmCrossProcessStrategy`/`GjiDirectStrategy`/`MsImeDirectStrategy` が
//!   機構そのものが異なるという既存の事実を型で表す）。

use awase::types::VkCode;

/// IME を開閉する実際の機構。
///
/// `ImmCross`（通常 Win32 アプリ）は `ImmSetOpenStatus` の cross-process 呼び出しで
/// 開閉する（VK を一切送らない）。`Imm32Unavailable`/`TsfNative` は冪等 VK
/// （`VK_IME_ON`/`VK_IME_OFF`/`VK_DBE_HIRAGANA` 等）を送る。この違いは
/// `ime_controller.rs` の `ImmCrossProcessStrategy` vs `GjiDirectStrategy`/
/// `MsImeDirectStrategy` の実装がそもそも異なることに現れており、単一の
/// `ime_open_key() -> VkCode` を全プロファイル共通の trait メソッドにすると
/// `ImmCross` に対して意味のない値を返させることになる（型で嘘をつく）ため、
/// この enum で機構自体を明示的に分岐させる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeOpenMechanism {
    /// `ImmSetOpenStatus` 相当のクロスプロセス API 呼び出し（VK 送信なし）。
    CrossProcessApi,
    /// 指定した VK を `SendInput` で送る（冪等キー、GJI/MS-IME 等）。
    ///
    /// Phase 1 で `GjiDirect`/`MsImeDirect` 相当のドライバがこの variant を
    /// 返す際は、VK を直書きせず既存 SSOT の
    /// `state/key_sequence_policy.rs::ime_key_for(KeyMechanism, ImeOperation)`
    /// から取得すること（`ime_controller.rs` の各 Strategy が使っているのと
    /// 同じ経路）。IME OFF キー反転のような実験で `ime_key_for` だけを更新して
    /// ドライバ配線側が旧 VK を送り続ける drift を防ぐ。
    Vk(VkCode),
}

/// プロファイルごとに独立した capability を宣言するドライバ。
///
/// [ADR-081](../../../../docs/adr/081-per-profile-capability-driver-decomposition.md)
/// Phase 0 の試験実装。各メソッドは「このプロファイルではこの値を返す」という
/// 静的な宣言のみを持ち、観測値に依存する動的判断（`shadow_on` のスキップ判定、
/// conv ビットに応じた ROMAN 事前設定等）は含まない —
/// それらは Phase 1（ランタイム配線）で `ImeControlView` を受け取る別メソッド
/// として追加する想定であり、本 Phase 0 実装のスコープ外。
///
/// なお `state/app_ime_policy.rs::AppImePolicy` が持つ静的な per-profile 値の
/// うち `default_feedback`（`FeedbackPolicy::Read`/`Blind` の選択）は、本
/// Phase 0 trait には **意図的に含めていない**。drift correction feedback は
/// ADR-080 の `Actuation`/`FeedbackPolicy` 経路が SSOT であり、ここで再宣言
/// すると二重定義になるため。ドライバへ移設するかどうかは全面移行を判断する
/// Phase 1 以降で扱う（[ADR-081](../../../../docs/adr/081-per-profile-capability-driver-decomposition.md)
/// 2 節の見積り表もこの feedback メソッドを全面移行時のコストに含めている）。
pub trait ImeProfileDriver {
    /// 物理 KANJI / VK_F3 / VK_F4 を awase が完全所有するか。
    ///
    /// `true` のとき、物理 KANJI イベントはアプリに渡さない
    /// （`state/app_ime_policy.rs::AppImePolicy::owns_physical_kanji` 相当）。
    fn owns_physical_kanji(&self) -> bool;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imm_cross_driver_owns_physical_kanji() {
        assert!(ImmCrossDriver.owns_physical_kanji());
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
        let driver: &dyn ImeProfileDriver = &ImmCrossDriver;
        assert!(driver.owns_physical_kanji());
    }
}
