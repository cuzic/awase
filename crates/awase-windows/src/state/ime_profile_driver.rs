//! `AppImeProfile` ごとの**コード構造についての契約宣言**（ADR-081 Phase 1a/1b/1c、
//! Phase 1d/1e は [ADR-090](../../../../docs/adr/090-typestate-effectuation-and-adjacent-adr-closure.md)
//! §2.F で**凍結**）。
//!
//! # 位置づけ — capability の表ではない
//!
//! **このモジュールは capability（どう書くか / 読み戻せるか / どれだけ待つか）を
//! 宣言しない。** capability の宣言点は
//! [`state/app_ime_policy.rs`](super::app_ime_policy) の `caps(profile, kind)`
//! **1 箇所だけ**である（ADR-089 INV-44 / ADR-090 INV-53）。ここに
//! `default_feedback` / `focus_settle_ms` / `ime_open_mechanism` /
//! `probe_budget_ms` を**戻してはならない**。
//!
//! 本 trait が残っているのは、`caps` に自然な置き場所が無い
//! **「コード構造についての契約」**を宣言するためである（ADR-090 §4.7）:
//!
//! - [`ImeProfileDriver::has_ime_on_path`] / [`ImeProfileDriver::stale_eisu_recovery_paired`]
//!   — 「IME-ON 経路を持つプロファイルは stale `ObservedEisu` 救済を**対で配線する**」
//!   という契約（BUG-07 / BUG-22 / BUG-37 ファミリー）。これは capability の
//!   **値**ではなく「対応するコードが存在すること」の宣言であり、const 表に
//!   載る種類の情報ではない。
//! - [`ImeProfileDriver::owns_physical_kanji`] — profile 軸の静的宣言
//!   （ADR-089 §2.5 が `caps` に入れないと決定済み、BUG-46）。
//!
//! # 経緯（ADR-081 Phase 1d/1e の凍結、ADR-090 §2.F）
//!
//! ADR-081 は `AppImePolicy` の match 式をプロファイル別 trait 実装へ分離する
//! Phase 1 を計画していたが、**その表現手段（trait 静的分岐）は ADR-089 §4.1 が
//! 「再提案禁止」で却下し**、capability は `caps(p, k)` という const 表へ
//! 一本化された（`caps` は (profile, IME 種別) の 2 軸を持ち、profile 軸のみの
//! trait より細かい）。Phase 1d は「`AppImePolicy` 参照をドライバ呼び出しへ
//! 置換する」作業であり、凍結しないことは却下済みの案の実装を続けることを
//! 意味する。Phase 1e の成果物（legacy 同期の撤去）は ADR-089 INV-43 が
//! 明示的に禁止しており、**「ブロッカーが解決した」のではなく
//! 「成果物が禁止されて moot になった」**。
//!
//! 凍結にあたり、`caps` と重複していた 4 メソッド
//! （`default_feedback` / `focus_settle_ms` / `ime_open_mechanism` /
//! `probe_budget_ms`）と `ImeOpenMechanism` enum、`BLIND_MAX_ATTEMPTS` /
//! `blind_feedback()` を削除した（ADR-090 決定 F-2）。`probe_budget_ms` の
//! 設計スケッチ（BUG-01 / BUG-21 の重症度別予算）は ADR-081 と git 履歴に残る。
//!
//! **`uses_gji_direct()` は ADR-089 Phase B（§6 item 8、§4.7）で撤去済み**
//! （2026-08-12）。同期義務の宣言軸は profile 軸ではなく outcome 軸である
//! （ADR-089 INV-42）ことが確定し、profile 軸での宣言が根拠を失ったため。
//! 同期義務は
//! [`state/gji_direct_mechanism.rs`](super::gji_direct_mechanism) の
//! `legacy_gji_sync_obligation` / `ActuationReceipt` が引き取っている。
//!
//! # 依存の制約（Linux でテスト可能にするため）
//!
//! `tsf/output.rs::ColdReason` や `state/ime_decision_view.rs::ImeControlView`、
//! さらに `crate::vk`・`state/key_sequence_policy.rs`（`ime_key_for`）は本クレートでは
//! `#[cfg(windows)]` 限定型であり、これらに依存すると本モジュールも windows 限定になり
//! Linux 上の `cargo test -p awase-windows --lib` でテストできなくなる（ADR-065 が
//! `state/` に課した非依存の原則）。残る 3 メソッドはいずれも `bool` を返すだけで
//! この制約に触れない。

use super::ime_event::ImePolicyProfile;

/// プロファイルごとの**コード構造契約**を宣言する trait。
///
/// **capability（チェーン / feedback / settle）は宣言しない**——その SSOT は
/// [`caps(profile, kind)`](super::app_ime_policy::caps) 1 箇所であり、
/// ここへ戻してはならない（ADR-090 INV-53）。
///
/// 各メソッドは「このプロファイルではこの値を返す」という静的な宣言のみを持ち、
/// 観測値に依存する動的判断（`shadow_on` のスキップ判定、conv ビットに応じた
/// ROMAN 事前設定、`active_ime_kind` に応じた GJI/MS-IME の具体 VK 選択等）は
/// 含まない。
///
/// `Sync` を要求するのは [`ALL_DRIVERS`] / [`driver_for`] が
/// `&'static dyn ImeProfileDriver` として保持するため。
pub trait ImeProfileDriver: Sync {
    /// 物理 KANJI / VK_F3 / VK_F4 を awase が完全所有するか。
    ///
    /// `true` のとき、物理 KANJI イベントはアプリに渡さない
    /// （`state/app_ime_policy.rs::AppImePolicy::owns_physical_kanji` 相当）。
    ///
    /// **注意（BUG-46）**: これは profile 軸（静的）のみの宣言であり、実効的な
    /// disposition の SSOT ではない。`false` を宣言していても GJI / MS-IME が
    /// actuate する場合は、実際には
    /// `runtime/transport.rs::PhysicalKeyDisposition::plan` が物理キーを Suppress
    /// する（TsfNative + GJI の実例）。本 trait を suppress 判定の SSOT に
    /// する場合は、profile 軸（本メソッド）と `active_ime_kind`（実行時観測）を
    /// `plan()` と同じ形で合成すること。
    fn owns_physical_kanji(&self) -> bool;

    /// このドライバが IME を OFF→ON にする経路を持つか（不変条件1 の一般化）。
    ///
    /// `true` のドライバは stale `ObservedEisu` 救済を対で配線する義務を負う
    /// （`state/eisu_recovery.rs` の対称性テストの一般化。`.claude/rules/
    /// ime-belief-architecture.md`）。
    fn has_ime_on_path(&self) -> bool;

    /// IME-ON 経路に対し stale `ObservedEisu` 救済を対で配線すると宣言するか。
    ///
    /// [`has_ime_on_path`](Self::has_ime_on_path) が `true` のとき、これも `true` で
    /// なければならない（contract test が強制、不変条件1）。
    fn stale_eisu_recovery_paired(&self) -> bool;
}

/// `ImePolicyProfile::ImmCross` 相当のドライバ（通常 Win32 アプリ、LINE/Qt 等）。
///
/// [[feedback_immcross_owns_kanji]]（ImmCross アプリには物理 IME キーを見せない
/// 設計原則）に基づき `owns_physical_kanji = true`。
/// このプロファイルの capability（`ImmSetOpenStatus` の cross-process 呼び出しを
/// 一次機構とし、読み戻し可能なので `Read`）は
/// [`caps(ImePolicyProfile::ImmCross, _)`](super::app_ime_policy::caps) が宣言する。
#[derive(Debug, Clone, Copy, Default)]
pub struct ImmCrossDriver;

impl ImeProfileDriver for ImmCrossDriver {
    fn owns_physical_kanji(&self) -> bool {
        // Step 1/1b の決定（`state/app_ime_policy.rs` と同値）。
        true
    }

    fn has_ime_on_path(&self) -> bool {
        true
    }

    fn stale_eisu_recovery_paired(&self) -> bool {
        // IME-ON 経路を持つため救済を対で配線すると宣言する（不変条件1）。
        true
    }
}

/// `ImePolicyProfile::Imm32Unavailable` 相当のドライバ（Chrome/Edge/UWP 等）。
///
/// IMM32 クロスプロセス制御が使えず、`active_ime_kind` に応じた冪等 VK
/// （GJI: `VK_IME_ON`/`VK_IME_OFF`、MS-IME: `VK_DBE_HIRAGANA`/`VK_IME_OFF`）で
/// 制御する。その機構チェーンと feedback（読み戻し不能なので `Blind`）は
/// [`caps(ImePolicyProfile::Imm32Unavailable, kind)`](super::app_ime_policy::caps)
/// が宣言する——**ドライバは宣言しない**（ADR-090 INV-53）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Imm32UnavailableDriver;

impl ImeProfileDriver for Imm32UnavailableDriver {
    fn owns_physical_kanji(&self) -> bool {
        // Step 1/1b の決定: Chrome/Edge も awase が物理 KANJI を所有
        // （`AppImePolicy::from_profile(Imm32Unavailable)` と同値）。
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
}

/// `ImePolicyProfile::TsfNative` 相当のドライバ（WezTerm/Windows Terminal 等）。
///
/// TSF ネイティブアプリ。TSF が KANJI を正しく処理するため物理 KANJI は**通す**
/// （`owns_physical_kanji=false`、profile 軸のみ）。ただし GJI/MS-IME が actuate
/// する場合は実効的に Suppress される（BUG-46、`owns_physical_kanji` の doc 参照）。
/// 機構チェーンと feedback は
/// [`caps(ImePolicyProfile::TsfNative, kind)`](super::app_ime_policy::caps) が宣言する。
#[derive(Debug, Clone, Copy, Default)]
pub struct TsfNativeDriver;

impl ImeProfileDriver for TsfNativeDriver {
    fn owns_physical_kanji(&self) -> bool {
        // WezTerm 等は TSF が KANJI を正しく処理するため通す（profile 軸のみ）。
        // （`AppImePolicy::from_profile(TsfNative)` と同値）。GJI/MS-IME actuate 時の
        // 実効 disposition は `owns_physical_kanji` doc と BUG-46 を参照。
        false
    }

    fn has_ime_on_path(&self) -> bool {
        true
    }

    fn stale_eisu_recovery_paired(&self) -> bool {
        true
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
    use crate::state::app_ime_policy::{caps, AppImePolicy};
    use crate::state::ime_kind::ImeKindId;

    /// contract test の走査対象プロファイル。`ImePolicyProfile` の全 variant。
    ///
    /// `app_ime_policy.rs` 側の `ALL_PROFILES`（`caps` 表の全数検査用）と同じ内容
    /// だが、あちらは `#[cfg(test)] mod tests` の private なのでここでも持つ。
    const ALL_PROFILES: [ImePolicyProfile; 5] = [
        ImePolicyProfile::ImmCross,
        ImePolicyProfile::Imm32Unavailable,
        ImePolicyProfile::TsfNative,
        ImePolicyProfile::Plain,
        ImePolicyProfile::Unknown,
    ];

    /// `owns_physical_kanji` を `AppImePolicy` SSOT と照合するヘルパ。
    ///
    /// **ADR-090 決定 F-2 で `focus_settle_ms` / `default_feedback` の 2 項目を
    /// 落とした**——それらは `caps(p, k)` が SSOT であり、driver 側に写しを
    /// 持たなくなったため（INV-53）。
    fn assert_owns_kanji_parity(driver: &dyn ImeProfileDriver, profile: ImePolicyProfile) {
        assert_eq!(
            driver.owns_physical_kanji(),
            AppImePolicy::from_profile(profile).owns_physical_kanji,
            "owns_physical_kanji drift ({profile:?})"
        );
    }

    #[test]
    fn imm_cross_driver_owns_physical_kanji() {
        assert!(ImmCrossDriver.owns_physical_kanji());
        assert_owns_kanji_parity(&ImmCrossDriver, ImePolicyProfile::ImmCross);
    }

    #[test]
    fn imm32_unavailable_driver_owns_physical_kanji_matches_app_ime_policy() {
        assert_owns_kanji_parity(&Imm32UnavailableDriver, ImePolicyProfile::Imm32Unavailable);
    }

    #[test]
    fn tsf_native_driver_owns_physical_kanji_matches_app_ime_policy() {
        assert_owns_kanji_parity(&TsfNativeDriver, ImePolicyProfile::TsfNative);
    }

    /// 型シグネチャレベルの見積り（ADR-081 Phase 0 記録 2節）が実装可能であることの
    /// 確認: trait オブジェクトとして扱えること。
    #[test]
    fn driver_is_usable_as_trait_object() {
        // `dyn ImeProfileDriver` が Sync であること（trait の Sync 境界が外れたら
        // `ALL_DRIVERS` / `driver_for` の `&'static dyn` 保持が壊れる）。
        fn assert_sync<T: Sync + ?Sized>() {}
        assert_sync::<dyn ImeProfileDriver>();
        let drivers: [&dyn ImeProfileDriver; 1] = [&ImmCrossDriver];
        assert!(drivers[0].owns_physical_kanji());
    }

    #[test]
    fn registry_maps_profiles_to_expected_driver() {
        // `driver_for` が各 profile を正しいドライバへ写像し、集約
        // （ImmCross/Plain/Unknown → ImmCrossDriver）が期待通りであることを、
        // **残った契約宣言だけ**で確認する。`ime_open_mechanism` による確認は
        // ADR-090 決定 F-2 で削除した（機構は `caps` が宣言する）。
        for profile in [
            ImePolicyProfile::ImmCross,
            ImePolicyProfile::Plain,
            ImePolicyProfile::Unknown,
        ] {
            assert!(
                driver_for(profile).owns_physical_kanji(),
                "{profile:?} → ImmCrossDriver"
            );
        }
        assert!(driver_for(ImePolicyProfile::Imm32Unavailable).owns_physical_kanji());
        assert!(!driver_for(ImePolicyProfile::TsfNative).owns_physical_kanji());
    }

    // ── contract test スイート（ADR-081 Phase 1c の不変条件、ADR-090 §2.F で
    //    凍結後も残す 3 件。**恒真な assert を置かない**、INV-53）──

    /// 不変条件1: IME-ON 経路を持つドライバは stale `ObservedEisu` 救済を対で持つ
    /// （`state/eisu_recovery.rs` の対称性テストの一般化。BUG-07/22/37 ファミリー）。
    ///
    /// **これが `ImeProfileDriver` を trait ごと削除しない理由である**
    /// （ADR-090 §4.7）——capability の値ではなく「対応するコードが存在すること」
    /// の宣言なので、`caps` const 表に自然な置き場所が無い。
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

    /// 不変条件2（**ADR-090 決定 F-2' で作り直した**）: 物理 KANJI を所有する
    /// プロファイルは、その物理キーの代わりに送る**一次機構**として
    /// `KanjiToggle` を使わない。
    ///
    /// # なぜ作り直したのか — 旧 assert は恒真だった
    ///
    /// 旧 `invariant_2_kanji_owning_drivers_use_non_kanji_mechanism` は
    /// `matches!(driver.ime_open_mechanism(open), CrossProcessApi | SharedImeKeyDispatch)`
    /// を assert していたが、`ImeOpenMechanism` はこの 2 variant しか持たない
    /// ため**どう実装を壊しても落ちない**。「代替なしに消える契約」ではなく
    /// 「元から効いていなかった契約」だったので、意図を実際に検査する形へ
    /// 移した（ADR-090 §2.F 決定 F-2'）。
    ///
    /// # なぜ「先頭」なのか
    ///
    /// `(ImmCross, MsIme)` の chain は `[ImmCross, KanjiToggle]` なので、
    /// **末尾**を条件にすると成立しない。末尾の `KanjiToggle` は `ImmCross` が
    /// `Failed` を返したときのフォールバックであり、INV-44 の到達可能性検査
    /// （`caps_chains_have_no_unreachable_trailing_element`）が正当化している。
    #[test]
    fn invariant_2_kanji_owning_profiles_do_not_lead_with_kanji_toggle() {
        use crate::state::actuation_chain::WriteMechanism;
        let mut checked = 0usize;
        for profile in ALL_PROFILES {
            if !driver_for(profile).owns_physical_kanji() {
                continue;
            }
            for kind in ImeKindId::ALL {
                let chain = caps(profile, kind).chain;
                assert_ne!(
                    chain.first(),
                    Some(&WriteMechanism::KanjiToggle),
                    "{profile:?} × {kind:?}: 物理 KANJI を所有するプロファイルの一次機構が \
                     KanjiToggle になっている（物理キーを抑止したうえで同じ非冪等トグルを \
                     自分で送ることになる）"
                );
                checked += 1;
            }
        }
        // 恒真化の防止: 実際に検査対象が存在したことを固定する
        // （`owns_physical_kanji` が全 false になれば loop が空回りする）。
        assert_eq!(
            checked,
            4 * ImeKindId::ALL.len(),
            "owns_physical_kanji=true のプロファイルは ImmCross/Imm32Unavailable/\
             Plain/Unknown の 4 つ（TsfNative のみ false）"
        );
    }

    /// 不変条件3（**ADR-090 決定 F-2 で駆動元を `caps(p, k).feedback` へ差し替えた**）:
    /// `Blind` は `max_attempts` で厳密に打ち切り、observation を書かずに終端する
    /// （BUG-33 型の収束偽装防止）。
    ///
    /// 旧実装は `driver.default_feedback()` で駆動していたが、そのメソッドは
    /// `caps` と重複していたため削除した。**主題が SSOT へ寄るぶんむしろ強くなる**
    /// （ADR-090 F-R1）——driver 側の写しではなく、実際に `ir_apply_drift_correction`
    /// が読む値そのものを駆動元にしている。
    ///
    /// `GiveUp` が observation を書かない不変は `ime_actuation.rs` の型 doc と
    /// `tests/architecture_guard.rs::drift_correction_giveup_and_confirmed_do_not_write_observations`
    /// が保証する。
    #[test]
    fn invariant_3_blind_profiles_terminate_without_writing_observation() {
        use crate::state::ime_actuation::{
            decide_actuation_action, ActuationAction, FeedbackPolicy,
        };
        let mut saw_blind = false;
        let mut saw_read = false;
        for profile in ALL_PROFILES {
            for kind in ImeKindId::ALL {
                let feedback = caps(profile, kind).feedback;
                match feedback {
                    FeedbackPolicy::Blind { max_attempts, .. } => {
                        saw_blind = true;
                        assert_eq!(
                            decide_actuation_action(feedback, max_attempts),
                            ActuationAction::GiveUp,
                            "{profile:?} × {kind:?}: Blind は max_attempts で GiveUp \
                             （observation 非書き込み）に終端する"
                        );
                        assert_eq!(
                            decide_actuation_action(feedback, max_attempts.saturating_sub(1)),
                            ActuationAction::Send,
                            "{profile:?} × {kind:?}: max_attempts 未満では諦めない"
                        );
                    }
                    FeedbackPolicy::Read { .. } => {
                        saw_read = true;
                        // Read は試行回数で打ち切らない。
                        assert_eq!(
                            decide_actuation_action(feedback, u32::MAX),
                            ActuationAction::Send,
                            "{profile:?} × {kind:?}"
                        );
                    }
                }
            }
        }
        // 恒真化の防止（INV-53「恒真な assert を置かない」）: caps 表から
        // 片方の variant が消えたらこのテストが空回りしないようにする。
        assert!(
            saw_blind && saw_read,
            "caps 表に Blind と Read の両方が現れる"
        );
    }

    /// ADR-081 不変条件4・5 は ADR-089 Phase B で引き取った（§6 item 8、§4.7、
    /// 2026-08-12）。
    ///
    /// - 不変条件4（belief を actuate 抜きで ON にする高速パスは必ず `GjiFsm` を
    ///   同期させる）→ INV-43。`ActuationReceipt` が `#[must_use]` + `Drop` の
    ///   `debug_assert` で運ぶ（`gji_direct_mechanism.rs`）。**保証水準は
    ///   「debug ビルドでの実行時検出」までである**（ADR-089 §8.1）。
    /// - 不変条件5（どのドライバ経由でも同一の `GjiFsm` 同期を通る）→ INV-42。
    ///   同期義務が profile 軸ではなく outcome 軸だけで決まることが確定したため、
    ///   「ドライバ経由でゲートする」という前提自体が消えた。
    ///
    /// ここに残すのは、**profile / IME 種別の静的宣言が実際の同期義務を
    /// ゲートしないこと**の確認である。ADR-090 決定 F-2 で
    /// `driver.ime_open_mechanism()` が消えたので、駆動元を `caps(p, k).chain`
    /// （= どの機構で書くかの SSOT）へ移した。
    #[test]
    fn sync_obligation_does_not_depend_on_profile_or_chain() {
        use crate::state::gji_direct_mechanism::{legacy_gji_sync_obligation, GjiFsmSync};
        for profile in ALL_PROFILES {
            for kind in ImeKindId::ALL {
                // chain（どの機構で書くか）を読んでも同期義務は変わらない（INV-42）。
                let _declared = caps(profile, kind).chain;
                assert_eq!(
                    legacy_gji_sync_obligation(true, awase::platform::ImeOpenOutcome::Applied),
                    Some(GjiFsmSync::OnImeOn),
                    "{profile:?} × {kind:?}: 同期義務は profile 軸・機構軸に依存しない"
                );
            }
        }
    }
}
