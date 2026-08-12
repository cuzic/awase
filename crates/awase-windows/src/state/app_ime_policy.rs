//! IME 制御のアプリ別ポリシー (Step 1.5)
//!
//! reducer / actuator にアプリ固有分岐がベタ書きされる前に、policy オブジェクトへ
//! 隔離する。Step 2B 以降の reducer 本格化のときに polymorphic な参照点として使う。
//!
//! ## 設計原則
//!
//! - **アプリ差分は AppImePolicy に閉じ込める** — reducer 本体に if-else を増やさない
//! - reducer は policy の "what to do" を参照するだけ、policy 自体に分岐ロジックを持たない

use super::actuation_chain::WriteMechanism;
use super::ime_actuation::FeedbackPolicy;
use super::ime_event::{ImePolicyProfile, ObservationSource};
use super::ime_kind::ImeKindId;
use std::time::Duration;

/// Blind feedback（読み戻し不能プロファイル）で actuation を打ち切るまでの試行回数。
///
/// **未検証の初期値**: 実 Windows 実機での soak テストによる裏付けはまだ無い
/// （このサンドボックスでは実測できない）。`5 × backoff`（`backoff` は現状
/// `DRIFT_CORRECTION_THRESHOLD_MS` を再利用、2026-07-25 時点で 400ms なので
/// 最悪 ~2s）を「遅いが最終的に成功する訂正を早すぎる段階で諦めない程度に長く、
/// かつ本当に stuck な状態を延々叩き続けない程度に短い」妥当な出発点として
/// 置いているだけ。この値を変更する場合は `.claude/rules/tuning-constants.md`
/// に従い実機実測の根拠を本文に添えること。
const IME_ACTUATION_BLIND_MAX_ATTEMPTS: u32 = 5;

// ── caps 表（ADR-089 §2.8、INV-44、Phase C item 11）────────────────────────────

/// `(profile, IME 種別)` → actuation の静的 capability（ADR-089 §2.8、INV-44）。
///
/// **これは ADR-088 の `AxisCapability`（軸 × 読み書き可否）とは別物である**
/// （ADR-089 §9-3）。本型は「(profile, IME種別) × 戦略チェーン / feedback /
/// settle」を宣言する。ADR-088 側を実装するときは、名前の似た 2 つの表が
/// 同居することになる点に注意し、必要なら別ファイルへ分けること。
///
/// **キー値（VK）は持たない** —— `state/key_sequence_policy.rs::ime_key_for` が
/// SSOT のままである（INV-44。`docs/experiments.md` エントリ01 の回帰検知点を
/// 分裂させない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// 試す機構の順序。`Failed` のときだけ次へ進む（`actuation_chain::falls_through`）。
    ///
    /// **到達不能な末尾要素を並べてはならない**（INV-44）。特に `GjiDirect` /
    /// `MsImeDirect` の後ろに `KanjiToggle` を置かないこと——両者は `Failed` を
    /// 返さないため到達不能であり、到達させるには `UnsafeToToggle` を
    /// フォールスルー対象に含めるしかなく、それは Win キー押下中に非冪等な
    /// `VK_KANJI` を送る新経路の新設である（ADR-089 §2.3・§4.9）。
    /// この規則は `caps_chains_have_no_unreachable_trailing_element` が固定する。
    pub chain: &'static [WriteMechanism],
    /// actuation の収束確認方針（ADR-080）。
    pub feedback: FeedbackPolicy,
    /// フォーカス変更後、observer を信頼できるようになるまでの待ち時間 (ms)。
    pub focus_settle_ms: u64,
}

/// `ImmCross` プロファイル × GJI。ImmCross が `Failed` を返したら GJI 冪等キーへ。
const CHAIN_IMM_CROSS_THEN_GJI: &[WriteMechanism] =
    &[WriteMechanism::ImmCross, WriteMechanism::GjiDirect];
/// `ImmCross` プロファイル × MS-IME。`MsImeDirect` は
/// `!can_use_imm32_cross_process()` を要求するため適用されず、最終フォールバックの
/// `KanjiToggle` が受ける（ADR-089 §2.8「`ImmCross × MsIme` に `MsImeDirect` を
/// 入れない理由」）。
const CHAIN_IMM_CROSS_THEN_KANJI: &[WriteMechanism] =
    &[WriteMechanism::ImmCross, WriteMechanism::KanjiToggle];
/// IMM32 クロスプロセス不可 × GJI。
const CHAIN_GJI_ONLY: &[WriteMechanism] = &[WriteMechanism::GjiDirect];
/// IMM32 クロスプロセス不可 × MS-IME。
const CHAIN_MS_IME_ONLY: &[WriteMechanism] = &[WriteMechanism::MsImeDirect];

/// 読み戻し可能プロファイルの feedback（`AppImePolicy::from_profile` の旧リテラル）。
const FEEDBACK_READ: FeedbackPolicy = FeedbackPolicy::Read {
    source: ObservationSource::ImmGetOpenStatus,
    deadline: Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS),
};
/// 読み戻し手段が構造的に無いプロファイルの feedback。
const FEEDBACK_BLIND: FeedbackPolicy = FeedbackPolicy::Blind {
    max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
    backoff: Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS),
};

/// capability の唯一の宣言点（ADR-089 §2.8、INV-44）。
///
/// **trait 静的分岐へ展開してはならない**（ADR-089 §4.1、再提案禁止）。
///
/// # `Plain` / `Unknown` 行について
///
/// **現時点で構造的に到達不能である**（ADR-089 §1.3(e)）。実行時に profile を
/// 供給するのは `impl From<AppImeProfile> for ImePolicyProfile`
/// （`focus/class_names.rs`）だけで、`AppImeProfile` は 3 値
/// （`Standard` → `ImmCross`）。`ImeModel` の初期値も
/// `AppImePolicy::standard()` = `from_profile(ImmCross)` である。
/// この 2 profile は「将来配線されたときのための安全デフォルト」であり、
/// **`ImmCross` 行と同一内容でなければならない**（INV-44、§4.5）。別扱いにすると
/// 起動直後 × MS-IME で非冪等な `VK_KANJI` へ直行する shadow desync 経路が
/// 生まれる。`plain_and_unknown_caps_are_identical_to_imm_cross` が固定する。
///
/// # K（IME 種別）依存性
///
/// **`feedback` / `focus_settle_ms` は K に依存しない**（ADR-089 §2.5）。
/// `ImeKindId` は `gji_monitor_ok` 由来の推測値であり（INV-45）、同一フォーカス中に
/// 反転しうる。K で分岐させると `focus_settle_ms` / `default_feedback` が
/// 「フォーカス中に変わりうる動的値」になり、既に計算済みの `settle_until` や
/// `Blind { max_attempts }` の試行カウントと組み合わさったときの挙動が現行と
/// 変わる。K 分岐を足すのは別コミット・別ソークにすること
/// （`caps_feedback_and_settle_are_k_independent` が固定する）。
#[must_use]
pub const fn caps(profile: ImePolicyProfile, kind: ImeKindId) -> Caps {
    match (profile, kind) {
        // ImmCross: 通常 Win32。IMM32 クロスプロセスが使えるので読み戻せる。
        // Plain / Unknown は安全デフォルト（ImmCross 同等）。
        (
            ImePolicyProfile::ImmCross | ImePolicyProfile::Plain | ImePolicyProfile::Unknown,
            ImeKindId::Gji,
        ) => Caps {
            chain: CHAIN_IMM_CROSS_THEN_GJI,
            feedback: FEEDBACK_READ,
            focus_settle_ms: 100,
        },
        (
            ImePolicyProfile::ImmCross | ImePolicyProfile::Plain | ImePolicyProfile::Unknown,
            ImeKindId::MsIme,
        ) => Caps {
            chain: CHAIN_IMM_CROSS_THEN_KANJI,
            feedback: FEEDBACK_READ,
            focus_settle_ms: 100,
        },
        // Imm32Unavailable: Chrome/Edge 等。GJI/IMM が信頼できないので settle 長め。
        (ImePolicyProfile::Imm32Unavailable, ImeKindId::Gji) => Caps {
            chain: CHAIN_GJI_ONLY,
            feedback: FEEDBACK_BLIND,
            focus_settle_ms: 500,
        },
        (ImePolicyProfile::Imm32Unavailable, ImeKindId::MsIme) => Caps {
            chain: CHAIN_MS_IME_ONLY,
            feedback: FEEDBACK_BLIND,
            focus_settle_ms: 500,
        },
        // TsfNative: WezTerm 等。
        (ImePolicyProfile::TsfNative, ImeKindId::Gji) => Caps {
            chain: CHAIN_GJI_ONLY,
            feedback: FEEDBACK_BLIND,
            focus_settle_ms: 200,
        },
        (ImePolicyProfile::TsfNative, ImeKindId::MsIme) => Caps {
            chain: CHAIN_MS_IME_ONLY,
            feedback: FEEDBACK_BLIND,
            focus_settle_ms: 200,
        },
    }
}

/// アプリ別の IME 制御ポリシー。
///
/// `AppImeProfile` (クラス名から決定) を基に派生する。
/// reducer / actuator はこのポリシーを参照して挙動を変える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppImePolicy {
    /// 物理 KANJI / VK_F3 / VK_F4 を awase が完全所有するか。
    ///
    /// `true` のとき、物理 KANJI イベントはアプリに渡さない (Step 1/1b 実装済の概念)。
    /// LINE/Qt / Chrome/Edge ともに `true`。WezTerm は `false`。
    ///
    /// **注意（BUG-46）**: これは `AppImeProfile`（静的）軸のみの値であり、実際に
    /// 物理キーを配送するかどうかの SSOT ではない。実効的な disposition は
    /// `runtime/transport.rs::PhysicalKeyDisposition::plan`（`transport.rs:144` 付近）
    /// が `ActiveImeKind`（動的、GJI/MS-IME 検出）も加味して決める。TsfNative でも
    /// GJI/MS-IME が actuate する場合は物理キーを Suppress する（`owns_physical_kanji=false`
    /// のまま実際には所有している）。本フィールドを直接参照して suppress 判定を書かない
    /// こと（`ime_profile_driver.rs` の Phase 1d SSOT 化時も同様）。
    pub owns_physical_kanji: bool,

    /// フォーカス変更後、observer を信頼できるようになるまでの待ち時間 (ms)。
    ///
    /// **値の決定点は [`caps`] である**（ADR-089 §2.5・Phase C item 11）。
    /// 本フィールドは既存の読み手（`state/ime_model.rs` の `settle_until` 算出、
    /// `state/platform_state.rs` のアクセサ、`runtime/mod.rs` の
    /// `schedule_settle_retry`）を触らずに済ませるためのファサードであり、
    /// SSOT ではない。
    pub focus_settle_ms: u64,

    /// このプロファイルの actuation デフォルト feedback（収束確認）方針。
    ///
    /// 読み戻し可能なプロファイル（ImmCross 系）は `Read`、読み戻し手段が
    /// 構造的に無いプロファイル（Imm32Unavailable / TsfNative）は `Blind`。
    ///
    /// **値の決定点は [`caps`] である**（`focus_settle_ms` と同じ理由）。
    pub default_feedback: FeedbackPolicy,
}

impl AppImePolicy {
    /// `ImePolicyProfile` から派生する。
    ///
    /// **`focus_settle_ms` / `default_feedback` は [`caps`] の薄いファサード**で
    /// ある（ADR-089 §2.5、Phase C item 11）。値をここに二重に書くと
    /// `AppImePolicy` と `caps` の二重 SSOT になる——ADR-081 の
    /// `ImeProfileDriver` が `AppImePolicy` と parity テストで同期を取り続けて
    /// いるのと同じ負債を 3 本目として増やすことになるため、リテラルは
    /// `caps` 側にしか置かない。
    ///
    /// `caps` は K（IME 種別）を取るが、この 2 フィールドは K 非依存である
    /// （`caps_feedback_and_settle_are_k_independent` が固定）。ここで
    /// `ImeKindId::Gji` を渡しているのは「代表値を 1 つ選ぶ」以上の意味を
    /// 持たない。**K で分岐させたくなったら、まず `AppImePolicy` が
    /// `FocusChanged` 時点の profile スナップショットである
    /// （`state/ime_model.rs`）ことと矛盾しないかを確認すること**
    /// （ADR-089 §2.5）。
    ///
    /// `owns_physical_kanji` は `caps` に吸収しない（ADR-089 §2.5）——
    /// BUG-46 の物理キー抑止は `ActiveImeKind`（実行時観測）と組み合わせて
    /// 判断する動的軸であり、静的 profile 軸の `caps` に入れると意味が壊れる。
    #[must_use]
    pub const fn from_profile(profile: ImePolicyProfile) -> Self {
        let c = caps(profile, ImeKindId::Gji);
        Self {
            // Step 1/1b の決定: ImmCross と Imm32Unavailable は KANJI を awase が所有。
            // WezTerm 等（TsfNative）は TSF が KANJI を正しく処理するため通す
            // （静的 profile 軸のみ。実効的な disposition は ActiveImeKind も見る
            // `PhysicalKeyDisposition::plan` が決める、BUG-46）。
            owns_physical_kanji: !matches!(profile, ImePolicyProfile::TsfNative),
            focus_settle_ms: c.focus_settle_ms,
            default_feedback: c.feedback,
        }
    }

    /// `ImmCross` プロファイルのデフォルト値。初期化時 / 不明 profile 時に使う。
    #[must_use]
    pub const fn standard() -> Self {
        Self::from_profile(ImePolicyProfile::ImmCross)
    }
}

impl Default for AppImePolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl From<ImePolicyProfile> for AppImePolicy {
    fn from(profile: ImePolicyProfile) -> Self {
        Self::from_profile(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ImePolicyProfile` の全 variant。`caps` の全数テスト用。
    ///
    /// `ImePolicyProfile` 自体に `ALL` を生やさないのは、本 crate 全体で
    /// 「到達可能なのは 3 値だけ」（ADR-089 §1.3(e)）という事実を薄めない
    /// ため。ここはあくまで **表の全数検査用**のリストである。
    const ALL_PROFILES: [ImePolicyProfile; 5] = [
        ImePolicyProfile::ImmCross,
        ImePolicyProfile::Imm32Unavailable,
        ImePolicyProfile::TsfNative,
        ImePolicyProfile::Plain,
        ImePolicyProfile::Unknown,
    ];

    #[test]
    fn all_profiles_covers_every_variant() {
        for p in ALL_PROFILES {
            // match の網羅性で「ALL_PROFILES に載せ忘れた variant」を検出する。
            match p {
                ImePolicyProfile::ImmCross
                | ImePolicyProfile::Imm32Unavailable
                | ImePolicyProfile::TsfNative
                | ImePolicyProfile::Plain
                | ImePolicyProfile::Unknown => {}
            }
        }
    }

    #[test]
    fn imm_cross_owns_physical_kanji() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        assert!(p.owns_physical_kanji);
        assert_eq!(
            caps(ImePolicyProfile::ImmCross, ImeKindId::Gji).chain,
            CHAIN_IMM_CROSS_THEN_GJI
        );
    }

    #[test]
    fn imm32_unavailable_owns_physical_kanji() {
        // Step 1/1b の決定: Chrome/Edge も awase が KANJI を所有
        let p = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        assert!(p.owns_physical_kanji);
        assert_eq!(p.focus_settle_ms, 500);
    }

    #[test]
    fn tsf_native_does_not_own_physical_kanji() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        assert!(!p.owns_physical_kanji);
        assert_eq!(p.focus_settle_ms, 200);
    }

    #[test]
    fn unknown_falls_back_to_imm_cross() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::Unknown);
        assert!(p.owns_physical_kanji);
        assert_eq!(p, AppImePolicy::from_profile(ImePolicyProfile::ImmCross));
    }

    // ── caps 表の全数テスト（ADR-089 §7「新設するもの — 全数テスト」、INV-44）──

    /// `caps` の 10 行が ADR-089 §2.8 の表と一致すること（chain はリテラル照合）。
    #[test]
    fn caps_chains_match_the_adr089_table() {
        let expected: &[(ImePolicyProfile, ImeKindId, &[WriteMechanism])] = &[
            (
                ImePolicyProfile::ImmCross,
                ImeKindId::Gji,
                CHAIN_IMM_CROSS_THEN_GJI,
            ),
            (
                ImePolicyProfile::ImmCross,
                ImeKindId::MsIme,
                CHAIN_IMM_CROSS_THEN_KANJI,
            ),
            (
                ImePolicyProfile::Plain,
                ImeKindId::Gji,
                CHAIN_IMM_CROSS_THEN_GJI,
            ),
            (
                ImePolicyProfile::Plain,
                ImeKindId::MsIme,
                CHAIN_IMM_CROSS_THEN_KANJI,
            ),
            (
                ImePolicyProfile::Unknown,
                ImeKindId::Gji,
                CHAIN_IMM_CROSS_THEN_GJI,
            ),
            (
                ImePolicyProfile::Unknown,
                ImeKindId::MsIme,
                CHAIN_IMM_CROSS_THEN_KANJI,
            ),
            (
                ImePolicyProfile::Imm32Unavailable,
                ImeKindId::Gji,
                CHAIN_GJI_ONLY,
            ),
            (
                ImePolicyProfile::Imm32Unavailable,
                ImeKindId::MsIme,
                CHAIN_MS_IME_ONLY,
            ),
            (ImePolicyProfile::TsfNative, ImeKindId::Gji, CHAIN_GJI_ONLY),
            (
                ImePolicyProfile::TsfNative,
                ImeKindId::MsIme,
                CHAIN_MS_IME_ONLY,
            ),
        ];
        assert_eq!(expected.len(), ALL_PROFILES.len() * ImeKindId::ALL.len());
        for &(profile, kind, chain) in expected {
            assert_eq!(caps(profile, kind).chain, chain, "{profile:?} × {kind:?}");
        }
    }

    /// **INV-44**: `chain` に、現行のフォールスルー述語（`Failed` のときだけ次へ）
    /// では到達しない要素を並べてはならない。
    ///
    /// 「次の要素へ進めるのは直前の機構が `Failed` を返しうるときだけ」なので、
    /// 末尾以外の全要素が [`WriteMechanism::may_return_failed`] を満たす必要がある。
    /// この検査があるため、`GjiDirect` / `MsImeDirect` の後ろに `KanjiToggle` を
    /// 足した瞬間にテストが落ちる（ADR-089 §4.9 の r3 の誤りの再発防止）。
    #[test]
    fn caps_chains_have_no_unreachable_trailing_element() {
        for profile in ALL_PROFILES {
            for kind in ImeKindId::ALL {
                let chain = caps(profile, kind).chain;
                assert!(!chain.is_empty(), "{profile:?} × {kind:?}: chain が空");
                for (idx, mechanism) in chain.iter().enumerate() {
                    if idx + 1 == chain.len() {
                        continue;
                    }
                    assert!(
                        mechanism.may_return_failed(),
                        "{profile:?} × {kind:?}: {mechanism:?} は Failed を返さないため \
                         後続要素 {:?} は到達不能",
                        chain[idx + 1]
                    );
                }
            }
        }
    }

    /// **INV-44**: `Plain` / `Unknown` 行は `ImmCross` 行と同一内容でなければ
    /// ならない（ADR-089 §1.3(e)・§4.5）。
    #[test]
    fn plain_and_unknown_caps_are_identical_to_imm_cross() {
        for kind in ImeKindId::ALL {
            let base = caps(ImePolicyProfile::ImmCross, kind);
            for profile in [ImePolicyProfile::Plain, ImePolicyProfile::Unknown] {
                assert_eq!(caps(profile, kind), base, "{profile:?} × {kind:?}");
            }
        }
    }

    /// **ADR-089 §2.5**: `feedback` / `focus_settle_ms` は K に依存しない。
    #[test]
    fn caps_feedback_and_settle_are_k_independent() {
        for profile in ALL_PROFILES {
            let gji = caps(profile, ImeKindId::Gji);
            let ms = caps(profile, ImeKindId::MsIme);
            assert_eq!(gji.feedback, ms.feedback, "{profile:?}");
            assert_eq!(gji.focus_settle_ms, ms.focus_settle_ms, "{profile:?}");
        }
    }

    /// `AppImePolicy` が `caps` のファサードであること（二重 SSOT の防止、
    /// ADR-089 §2.5）。旧 `from_profile` のリテラルは `caps` 側にしか無いため、
    /// この照合は「ファサードが表から外れていないか」を見る。
    #[test]
    fn app_ime_policy_is_a_facade_over_caps() {
        for profile in ALL_PROFILES {
            let policy = AppImePolicy::from_profile(profile);
            for kind in ImeKindId::ALL {
                let c = caps(profile, kind);
                assert_eq!(policy.focus_settle_ms, c.focus_settle_ms, "{profile:?}");
                assert_eq!(policy.default_feedback, c.feedback, "{profile:?}");
            }
        }
    }

    /// `caps` の `focus_settle_ms` が、`ae64431d` 時点の
    /// `AppImePolicy::from_profile` のリテラルと一致すること。
    ///
    /// `app_ime_policy_is_a_facade_over_caps` だけでは「両方まとめて間違えた」
    /// 場合を検出できないため、実測由来の直値をここに 1 箇所だけ残す。
    #[test]
    fn caps_settle_values_match_the_pre_phase_c_literals() {
        for kind in ImeKindId::ALL {
            assert_eq!(caps(ImePolicyProfile::ImmCross, kind).focus_settle_ms, 100);
            assert_eq!(
                caps(ImePolicyProfile::Imm32Unavailable, kind).focus_settle_ms,
                500
            );
            assert_eq!(caps(ImePolicyProfile::TsfNative, kind).focus_settle_ms, 200);
        }
    }

    #[test]
    fn default_is_standard() {
        assert_eq!(AppImePolicy::default(), AppImePolicy::standard());
    }

    #[test]
    fn imm_cross_default_feedback_is_read_via_imm_get_open_status() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        assert!(matches!(
            p.default_feedback,
            FeedbackPolicy::Read {
                source: ObservationSource::ImmGetOpenStatus,
                deadline,
            } if deadline
                == Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        ));
    }

    #[test]
    fn plain_and_unknown_share_imm_cross_read_feedback() {
        for profile in [ImePolicyProfile::Plain, ImePolicyProfile::Unknown] {
            let p = AppImePolicy::from_profile(profile);
            assert!(matches!(
                p.default_feedback,
                FeedbackPolicy::Read {
                    source: ObservationSource::ImmGetOpenStatus,
                    ..
                }
            ));
        }
    }

    #[test]
    fn imm32_unavailable_default_feedback_is_blind() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        assert!(matches!(
            p.default_feedback,
            FeedbackPolicy::Blind {
                max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
                backoff,
            } if backoff == Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        ));
    }

    #[test]
    fn tsf_native_default_feedback_is_blind() {
        let p = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        assert!(matches!(
            p.default_feedback,
            FeedbackPolicy::Blind {
                max_attempts: IME_ACTUATION_BLIND_MAX_ATTEMPTS,
                backoff,
            } if backoff == Duration::from_millis(crate::tuning::DRIFT_CORRECTION_THRESHOLD_MS)
        ));
    }

    #[test]
    fn from_trait_impl() {
        let p: AppImePolicy = ImePolicyProfile::Imm32Unavailable.into();
        assert_eq!(p.focus_settle_ms, 500);
    }
}
