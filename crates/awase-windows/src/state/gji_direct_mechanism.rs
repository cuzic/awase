//! 共有 GJI 直接制御機構（ADR-081 Phase 1c、design B）。未配線。
//!
//! # なぜドライバから分離した1箇所に置くのか（design B）
//!
//! [ADR-081](../../../../docs/adr/081-per-profile-capability-driver-decomposition.md)
//! の「GJI 横断性の設計」節で確定した通り、`active_ime_kind`（GJI / MS-IME）は
//! `AppImeProfile` に対して静的でなく**実行時観測**であり、profile 軸（アプリの IME
//! 受容能力、静的）とは直交する2次元である。GJI 直接制御（現行
//! `ime_controller.rs::GjiDirectStrategy` 相当）を各ドライバ内の動的分岐として
//! 持たせると、3ドライバがそれぞれ GJI 分岐を再実装することになり、Phase 0 で
//! 見つかった反証データ6件（BUG-21/29/30/31/32/35、「既に部分分離されていた実装が
//! 同期を怠ってバグを生んだ」）と同じ失敗モードを再生産する。
//!
//! そこで GJI 機構は**全プロファイル横断で適用される共有機構**として本モジュール
//! 1箇所に実装し、各ドライバは
//! [`ImeProfileDriver::uses_gji_direct`](super::ime_profile_driver::ImeProfileDriver::uses_gji_direct)
//! を**静的に宣言するだけ**にする。driver と GJI 機構の実行時合成はランタイム側
//! （Phase 1d の配線）の責務であり、ドライバ自体は GJI / MS-IME の分岐を持たない。
//!
//! # 型で表現する不変条件（ADR-081 Phase 1c contract test 4・5）
//!
//! - **アクセスの排他性（不変条件5）**: 本機構を呼ぶには [`GjiDirectAccess`] token が
//!   要る。token の唯一の公開コンストラクタは [`GjiDirectMechanism::access_for`] で、
//!   `uses_gji_direct() == true` のドライバにのみ `Some` を返す。`uses_gji_direct()` を
//!   宣言しないドライバからは構造的に本機構へ到達できない。
//! - **同期義務との分離不能性（不変条件4）**: 作動要求の唯一の帰結が
//!   [`GjiActuation`]（[`GjiFsmSync`] 同期義務を内包）であり、同期義務を伴わずに
//!   GJI で IME を ON にする経路を提供しない。これにより「実 apply をスキップして
//!   belief だけ ON にする高速パス」（BUG-18/22 型）が `GjiFsm` 同期を踏み抜くことを
//!   型レベルで防ぐ。
//!
//! # Linux でテスト可能にするための制約（ADR-065）
//!
//! `GjiFsm`（`tsf/` 配下、`#[cfg(windows)]`）そのものには依存しない。同期義務は
//! [`GjiFsmSync`]（ungated な列挙値）で象徴的に表し、実 `GjiFsm` への写像は Phase 1d
//! のランタイム配線が担う。**送信 VK の解決は本モジュールでは行わない**: 具体 VK は
//! 既存 SSOT の `state/key_sequence_policy.rs::ime_key_for`（`crate::vk` に依存する
//! ため `#[cfg(windows)]`）が握り、Phase 1d の windows 境界で
//! `GjiActuation` の open 方向をキーに解決する。ここで VK を複製すると
//! `ime_key_for` SSOT が二重化し、IME OFF キー反転実験
//! （`.claude/rules/experiment-logging.md`）の drift 源になるため、あえて持たない。

use super::ime_profile_driver::ImeProfileDriver;

/// GJI 機構経由の IME 状態遷移が課す `GjiFsm` 同期義務のマーカー。
///
/// 現行 `runtime` 層の `gji_on_ime_on` / `gji_on_ime_off`（`GjiFsm` を belief と
/// 同期させるハンドラ）に対応する。Phase 1d のランタイム配線がこの値を実 `GjiFsm`
/// 呼び出しへ写像する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GjiFsmSync {
    /// IME を開いた（`gji_on_ime_on` 相当の同期が必要）。
    OnImeOn,
    /// IME を閉じた（`gji_on_ime_off` 相当の同期が必要）。
    OnImeOff,
}

impl GjiFsmSync {
    #[must_use]
    const fn for_open(open: bool) -> Self {
        if open {
            Self::OnImeOn
        } else {
            Self::OnImeOff
        }
    }
}

/// GJI 直接制御の1回の作動要求の帰結。`GjiFsm` 同期義務を内包する。
///
/// GJI で IME を ON/OFF するには本 struct を得る（= 同期義務を受け取る）以外の経路が
/// 無いため、「belief を actuate 抜きで ON にする高速パスが `GjiFsm` 同期を踏み抜く」
/// （BUG-18/22 型）を型レベルで防ぐ（ADR-081 Phase 1c 不変条件4）。
///
/// 送信する具体 VK は保持しない（module doc の SSOT 二重化回避方針を参照）。Phase 1d の
/// windows 配線が [`open`](Self::open) を `ime_key_for(KeyMechanism::GjiDirect, ..)` へ
/// 渡して解決する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GjiActuation {
    /// この作動が IME を開く方向か（`ime_key_for` の `ImeOperation::from_open` に渡す）。
    pub open: bool,
    /// この作動が課す `GjiFsm` 同期義務。
    pub fsm_sync: GjiFsmSync,
}

/// 共有 GJI 機構へのアクセス権を表す capability token。
///
/// フィールドが非公開のため本モジュール外では構築できず、唯一の公開経路は
/// [`GjiDirectMechanism::access_for`]。`uses_gji_direct()` を宣言するドライバからのみ
/// 発行される（ADR-081 Phase 1c 不変条件5）。
#[derive(Debug)]
pub struct GjiDirectAccess(());

/// 全プロファイル横断で共有される GJI 直接制御機構。状態を持たない。
#[derive(Debug, Clone, Copy)]
pub struct GjiDirectMechanism;

impl GjiDirectMechanism {
    /// ドライバが `uses_gji_direct()` を宣言している場合にのみアクセス token を発行する。
    ///
    /// これが共有 GJI 機構への唯一の入口。宣言しないドライバには `None` を返し、
    /// 本機構を呼べないことを構造的に保証する（不変条件5）。
    #[must_use]
    pub fn access_for(driver: &dyn ImeProfileDriver) -> Option<GjiDirectAccess> {
        driver.uses_gji_direct().then_some(GjiDirectAccess(()))
    }

    /// GJI 直接制御の作動要求を処理し、`GjiFsm` 同期義務を内包した帰結を返す。
    ///
    /// `access` を要求することで、`uses_gji_direct()` を宣言したドライバ経由でしか
    /// 呼べないことを型で縛る。どのドライバ経由でも同一の [`GjiFsmSync`] を返す
    /// （不変条件5: 同一の `GjiFsm` 同期を通る）。
    #[must_use]
    pub fn actuate(access: &GjiDirectAccess, open: bool) -> GjiActuation {
        // token は所有証明（呼び出し可能性のゲート）としてのみ使う。
        let GjiDirectAccess(()) = access;
        GjiActuation {
            open,
            fsm_sync: GjiFsmSync::for_open(open),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ime_profile_driver::{
        Imm32UnavailableDriver, ImmCrossDriver, TsfNativeDriver,
    };

    #[test]
    fn access_granted_only_to_gji_direct_drivers() {
        // uses_gji_direct を宣言するドライバのみ token を得られる（不変条件5）。
        assert!(GjiDirectMechanism::access_for(&Imm32UnavailableDriver).is_some());
        assert!(GjiDirectMechanism::access_for(&TsfNativeDriver).is_some());
        // ImmCross は cross-process API 経路。共有 GJI 機構へは到達できない。
        assert!(GjiDirectMechanism::access_for(&ImmCrossDriver).is_none());
    }

    #[test]
    fn actuation_bundles_direction_with_matching_fsm_sync() {
        // IME-ON 作動を要求する唯一の経路が OnImeOn 同期義務と分離不能であること
        // （不変条件4: 高速パスが GjiFsm 同期を踏み抜けない）。
        let access = GjiDirectMechanism::access_for(&Imm32UnavailableDriver).unwrap();

        let on = GjiDirectMechanism::actuate(&access, true);
        assert!(on.open);
        assert_eq!(on.fsm_sync, GjiFsmSync::OnImeOn);

        let off = GjiDirectMechanism::actuate(&access, false);
        assert!(!off.open);
        assert_eq!(off.fsm_sync, GjiFsmSync::OnImeOff);
    }

    #[test]
    fn all_gji_drivers_share_identical_sync_path() {
        // どのドライバ経由でも同一の帰結を返す（不変条件5）。
        let a = GjiDirectMechanism::access_for(&Imm32UnavailableDriver).unwrap();
        let b = GjiDirectMechanism::access_for(&TsfNativeDriver).unwrap();
        for open in [true, false] {
            assert_eq!(
                GjiDirectMechanism::actuate(&a, open),
                GjiDirectMechanism::actuate(&b, open),
            );
        }
    }
}
