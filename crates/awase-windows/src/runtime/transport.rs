use std::collections::HashSet;

use awase::types::{KeyEventType, RawKeyEvent, VkCode};

use crate::focus::class_names::AppImeProfile;
use crate::tsf::observer::ActiveImeKind;
use crate::vk::VkCodeExt as _;

pub(crate) use crate::state::physical_disposition::PhysicalKeyDisposition;

impl PhysicalKeyDisposition {
    /// `Suppress` の場合のみ理由ラベルを返す（`kp_stage_execute` の debug log と
    /// journal 記録（`JournalEntry::KeyInput::physical`）で共用し、2箇所が
    /// 別々に判定ロジックを持って乖離することを防ぐ）。
    ///
    /// BUG-90 調査用: journal の `KeyInput.decision` は engine の意味論的判断
    /// （PassThrough/Consume）であり、この配送判断（実際に OS へ届いたか）とは
    /// 独立している。この関数を journal に記録することで両者を突き合わせられる
    /// ようにする（`docs/known-bugs.md` BUG-90 参照）。
    pub(crate) fn suppress_reason(
        self,
        event: &RawKeyEvent,
        profile: AppImeProfile,
    ) -> Option<&'static str> {
        if self != Self::Suppress {
            return None;
        }
        Some(if crate::vk::is_role_fkey(event.vk_code) {
            // F13〜F24（ADR-199 決定18）。profile に依らず「awase が実際に書いた打鍵」だけ Suppress される。
            "role-fkey"
        } else if profile.can_use_imm32_cross_process() {
            "imm-cross"
        } else {
            "imm32-off"
        })
    }
}

/// passthrough キーの Down/Up 対称性と output guard defer を管理するキュー。
///
/// `check_output_guard_defer` で defer した KeyDown の VK を `deferred_vks` に記録し、
/// 対応する KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ（WezTerm 対策）。
/// 各メソッドが `Some(event)` を返したとき、呼び出し元が `ReinjectKey(event)` をキューに
/// 積んで `Consumed` を返す責務を持つ。
///
/// `deferred_vks` は `VkCode`（u16）の `HashSet` なので有界（メモリリークではない）。
/// 0xF3/0xF4 のような「ペア表現」の KANJI 系キー（対応する KeyUp が原理的に来ない
/// 場合がある）ではエントリが残留し得るが、BUG-46 の修正で KANJI 系 KeyUp は原則
/// Suppress されるようになり `check_output_guard_defer` に到達しなくなったため inert
/// （BUG-173 追補のラッチ `keyup_follows_keydown` が KeyUp を Allow に揃える場合は到達し得るが、
/// KeyDown も Allow で defer 済みなら `check_keyup_symmetry` が対で処理するため残留しない）。
/// 「leak しているように見える」からと TTL/クリア機構を追加する前に、まずこの残留が
/// 実際に `check_keyup_symmetry` の誤発火につながる経路があるか確認すること。
pub(crate) struct PassthroughQueue {
    deferred_vks: HashSet<VkCode>,
}

impl PassthroughQueue {
    pub(crate) fn new() -> Self {
        Self {
            deferred_vks: HashSet::new(),
        }
    }

    /// KeyUp 対称性チェック。
    /// deferred KeyDown の VK に対応する KeyUp を reinject に揃える。
    /// `Some(event)` を返したら呼び出し元が `ReinjectKey(event)` を積んで `Consumed` を返す。
    pub(crate) fn check_keyup_symmetry(&mut self, event: &RawKeyEvent) -> Option<RawKeyEvent> {
        let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
        if !is_key_down && self.deferred_vks.remove(&event.vk_code) {
            tracing::debug!(
                "[relay-sym] PassThrough KeyUp vk={:#04x}: KeyDown was deferred → force reinject for symmetry",
                event.vk_code,
            );
            return Some(*event);
        }
        None
    }

    /// output guard / pending queue による defer チェック。
    /// `Some(event)` を返したら呼び出し元が `ReinjectKey(event)` を積んで `Consumed` を返す。
    ///
    /// 例外: 修飾キー (Ctrl/Alt/Win) KeyUp は defer しない（Ctrl 残留窓を作らないため）。
    /// KeyDown が defer 済みのケースは `check_keyup_symmetry` が先に捕捉する。
    pub(crate) fn check_output_guard_defer(
        &mut self,
        event: &RawKeyEvent,
        output_in_flight: bool,
        in_flight_ms: u64,
        has_pending: bool,
    ) -> Option<RawKeyEvent> {
        let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
        if !is_key_down && event.vk_code.is_non_shift_modifier() {
            return None;
        }
        if has_pending || output_in_flight {
            let reason = if output_in_flight && !has_pending {
                format!("output in-flight ({in_flight_ms}ms ago)")
            } else if has_pending && output_in_flight {
                format!("pending effects + output in-flight ({in_flight_ms}ms)")
            } else {
                "pending effects".to_string()
            };
            tracing::debug!(
                "[relay-defer] PassThrough deferred: {reason}, reinject(vk={:#04x} {})",
                event.vk_code,
                if is_key_down { "down" } else { "up" },
            );
            if is_key_down {
                self.deferred_vks.insert(event.vk_code);
            }
            return Some(*event);
        }
        None
    }
}

impl PhysicalKeyDisposition {
    /// 物理キーを OS に届けるかどうかの判断。本体は `state/physical_disposition.rs::plan_core`
    /// （ungated。ADR-208 L0 で挙動を変えずに移した）。ここは `ActiveImeKind` → `ImeKindId` の
    /// 変換だけを行う殻で、判断の詳細（F2 の常時 Allow、KANJI 関連キーの ImmCross/Imm32Unavailable の
    /// 分岐、BUG-46/52/116 の経緯）は `plan_core` の doc とコメントを参照。
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(?profile, shadow_toggled = shadow_toggled, ?active_ime_kind)
    )]
    pub(crate) fn plan(
        event: &RawKeyEvent,
        profile: AppImeProfile,
        shadow_toggled: bool,
        active_ime_kind: ActiveImeKind,
    ) -> Self {
        Self::plan_core(event, profile, shadow_toggled, active_ime_kind.into())
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use awase::types::{ImeRelevance, KeyClassification, ModifierState, ScanCode, ShadowImeAction};

    fn kanji_event(
        event_type: KeyEventType,
        shadow_action: Option<ShadowImeAction>,
    ) -> RawKeyEvent {
        RawKeyEvent {
            was_down: false,
            vk_code: crate::vk::VK_KANJI,
            scan_code: ScanCode(0x1E),
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance {
                shadow_action,
                ..ImeRelevance::default()
            },
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            left_thumb_down_snapshot: None,
            right_thumb_down_snapshot: None,
            injected: false,
        }
    }

    fn non_kanji_event(event_type: KeyEventType) -> RawKeyEvent {
        kanji_event(event_type, None)
    }

    fn dbe_mode_event(
        vk_code: VkCode,
        action: ShadowImeAction,
        event_type: KeyEventType,
    ) -> RawKeyEvent {
        RawKeyEvent {
            was_down: false,
            vk_code,
            ..kanji_event(event_type, Some(action))
        }
    }

    /// awase が beliefに基づく開閉トグルとして書く VK_DBE_*（半角/全角、ADR-189/191）。
    /// これらの KeyDown は `shadow_toggled` に関わらず
    /// Suppress される（二重 actuation の防止、BUG-46/BUG-52）。
    fn dbe_written_vks() -> Vec<(VkCode, ShadowImeAction, &'static str)> {
        vec![
            (
                crate::vk::VK_DBE_SBCSCHAR,
                ShadowImeAction::Toggle,
                "VK_DBE_SBCSCHAR (0xF3)",
            ),
            (
                crate::vk::VK_DBE_DBCSCHAR,
                ShadowImeAction::Toggle,
                "VK_DBE_DBCSCHAR (0xF4)",
            ),
        ]
    }

    /// awase が書かない VK_DBE_*（英数・カタカナ。ADR-191 撤去後は awase が代行しない）。
    /// 実イベントでは `shadow_action` が無い（`ImeKeyKind::shadow_effect` が `None`）が、
    /// 撤去前の合成イベント（`shadow_action` あり）でも Suppress されないことを固定するため
    /// アクションを付けた形も用意する。
    fn dbe_unwritten_vks() -> Vec<(VkCode, ShadowImeAction, &'static str)> {
        vec![
            (
                crate::vk::VK_DBE_ALPHANUMERIC,
                ShadowImeAction::TurnOff,
                "VK_DBE_ALPHANUMERIC (0xF0)",
            ),
            (
                crate::vk::VK_DBE_KATAKANA,
                ShadowImeAction::TurnOn,
                "VK_DBE_KATAKANA (0xF1)",
            ),
        ]
    }

    /// 決定表・KeyUp 系のテストが全 VK_DBE_*（0xF0/0xF1/0xF3/0xF4）を回すための和集合。
    fn dbe_mode_vks() -> Vec<(VkCode, ShadowImeAction, &'static str)> {
        let mut v = dbe_written_vks();
        v.extend(dbe_unwritten_vks());
        v
    }

    fn f2_event(event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            was_down: false,
            vk_code: crate::vk::VK_DBE_HIRAGANA,
            ..kanji_event(event_type, None)
        }
    }

    fn injected(mut event: RawKeyEvent) -> RawKeyEvent {
        event.injected = true;
        event
    }

    // F2/非KANJI テストでは ime_actuation_owned 判定に到達しないため、
    // active_ime_kind はどちらでもよい filler として GoogleJapaneseInput を使う。
    const ANY_IME_KIND: ActiveImeKind = ActiveImeKind::GoogleJapaneseInput;

    // ── F2 (VK_DBE_HIRAGANA): 常に Allow（BUG-173） ──

    /// 旧 `f2_tsf_mode_suppresses_down_and_up` / BUG-10 回帰 / 非TSF の3テストを統合。TSF mode + GJI 戦略でも
    /// 物理 F2 は Down/Up とも全プロファイルで素通しする（ADR-100 決定2 で warmup が `VK_IME_ON` 単発になり、
    /// 「代わりに F2 を再送する」契約が無い。MS-IME も従来から素通し＝BUG-10）。
    #[test]
    fn f2_is_always_allowed_down_and_up() {
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            for profile in [
                AppImeProfile::Standard,
                AppImeProfile::Imm32Unavailable,
                AppImeProfile::TsfNative,
                AppImeProfile::InputRelay,
            ] {
                let ev = f2_event(event_type);
                assert_eq!(
                    PhysicalKeyDisposition::plan(&ev, profile, false, ANY_IME_KIND),
                    PhysicalKeyDisposition::Allow,
                    "{profile:?} {event_type:?}: 物理 F2 は常に素通し（BUG-173）"
                );
            }
        }
    }

    // ── 非 KANJI イベントは常に Allow (プロファイル/shadow_toggle 不問) ──

    #[test]
    fn non_kanji_event_always_allowed() {
        for profile in [
            AppImeProfile::Standard,
            AppImeProfile::Imm32Unavailable,
            AppImeProfile::TsfNative,
        ] {
            for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                for shadow_toggled in [false, true] {
                    for active_ime_kind in [
                        ActiveImeKind::GoogleJapaneseInput,
                        ActiveImeKind::MicrosoftIme,
                    ] {
                        let ev = non_kanji_event(event_type);
                        assert_eq!(
                            PhysicalKeyDisposition::plan(
                                &ev,
                                profile,
                                shadow_toggled,
                                active_ime_kind
                            ),
                            PhysicalKeyDisposition::Allow,
                            "非KANJIイベントは profile={profile:?} shadow_toggled={shadow_toggled} \
                             event_type={event_type:?} active_ime_kind={active_ime_kind:?} でも常に Allow"
                        );
                    }
                }
            }
        }
    }

    // ── ImmCross (Standard): KANJI 関連 VK は Down/Up 共に Suppress (spurious連鎖の構造的遮断) ──

    #[test]
    fn immcross_suppresses_kanji_down_and_up_regardless_of_shadow_toggled() {
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            for shadow_toggled in [false, true] {
                let ev = kanji_event(event_type, Some(ShadowImeAction::TurnOn));
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        AppImeProfile::Standard,
                        shadow_toggled,
                        ActiveImeKind::MicrosoftIme
                    ),
                    PhysicalKeyDisposition::Suppress,
                    "ImmCross (Standard) は shadow_toggled={shadow_toggled} event_type={event_type:?} \
                     でも常に Suppress (spurious VK_F3/F4 連鎖の根本修正、08b8661)"
                );
            }
        }
    }

    // ── 無変換/変換（ADR-141、C2対策）: shadow_action が付いても物理配送は
    //    常にAllow（follow-only、GJI自身が物理キーでIMEを切り替える設計）──

    fn henkan_muhenkan_event(
        vk_code: VkCode,
        action: Option<ShadowImeAction>,
        event_type: KeyEventType,
    ) -> RawKeyEvent {
        RawKeyEvent {
            was_down: false,
            vk_code,
            ..kanji_event(event_type, action)
        }
    }

    #[test]
    fn henkan_muhenkan_always_allowed_even_with_shadow_action_under_suppress_conditions() {
        for vk in [crate::vk::VK_CONVERT, crate::vk::VK_NONCONVERT] {
            for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                // ImmCross (Standard): KANJI 系 VK なら shadow_action 有りで
                // 無条件 Suppress される条件（`immcross_suppresses_kanji_
                // down_and_up_regardless_of_shadow_toggled` と同じ形）。
                let ev = henkan_muhenkan_event(vk, Some(ShadowImeAction::TurnOn), event_type);
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        AppImeProfile::Standard,
                        false,
                        ActiveImeKind::MicrosoftIme
                    ),
                    PhysicalKeyDisposition::Allow,
                    "無変換/変換(vk={vk:?}, event_type={event_type:?}) は shadow_action が \
                     付いても ImmCross 下で常に Allow（follow-only、ADR-141）"
                );

                // TsfNative + GJI（ime_actuation_owned=true）+ shadow_toggled=true:
                // KANJI 系 VK なら Suppress される条件
                // （`owned_actuation_cases`系のテストと同じ形）。
                let ev2 = henkan_muhenkan_event(vk, Some(ShadowImeAction::TurnOn), event_type);
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev2,
                        AppImeProfile::TsfNative,
                        true,
                        ActiveImeKind::GoogleJapaneseInput
                    ),
                    PhysicalKeyDisposition::Allow,
                    "無変換/変換(vk={vk:?}, event_type={event_type:?}) は shadow_toggled=true \
                     かつ ime_actuation_owned な状況でも常に Allow（follow-only、ADR-141）"
                );
            }
        }
    }

    // ── Imm32Unavailable / TsfNative 共通: apply-ime が GjiDirect/MsImeDirect で
    //    actuate する場合、shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress ──
    //
    // BUG-46: 旧実装は profile.should_pass_physical_key()（TsfNative で常に true）のみで
    // 判定しており、TsfNative + GJI/MsIme（Windows Terminal 等）では awase 自身の
    // apply-ime SendInput と、素通しされた物理 KANJI 系キーの reinject が二重に actuate
    // していた。ImeActuationOwned（gji_direct_applicable / ms_ime_direct_applicable）を
    // profile ではなく ActiveImeKind から導出することで、Imm32Unavailable と TsfNative を
    // 同じ suppress ロジックに統一する。

    /// `plan()` の `(profile, active_ime_kind)` の全組み合わせで suppress 挙動が
    /// Imm32Unavailable と TsfNative で一致することを固定する。
    fn owned_actuation_cases() -> Vec<(AppImeProfile, ActiveImeKind, &'static str)> {
        vec![
            (
                AppImeProfile::Imm32Unavailable,
                ActiveImeKind::MicrosoftIme,
                "Imm32Unavailable+MsIme (Chrome/Edge, 従来通り)",
            ),
            (
                AppImeProfile::Imm32Unavailable,
                ActiveImeKind::GoogleJapaneseInput,
                "Imm32Unavailable+GJI",
            ),
            (
                AppImeProfile::TsfNative,
                ActiveImeKind::GoogleJapaneseInput,
                "TsfNative+GJI (Windows Terminal, BUG-46 再現条件)",
            ),
            (
                AppImeProfile::TsfNative,
                ActiveImeKind::MicrosoftIme,
                "TsfNative+MsIme (WezTerm)",
            ),
        ]
    }

    #[test]
    fn owned_actuation_keydown_allowed_when_not_shadow_toggled() {
        for (profile, active_ime_kind, label) in owned_actuation_cases() {
            let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan(&ev, profile, false, active_ime_kind),
                PhysicalKeyDisposition::Allow,
                "{label}: shadow_toggle が発火していない KeyDown は物理キーを通す"
            );
        }
    }

    #[test]
    fn owned_actuation_keydown_suppressed_when_shadow_toggled() {
        for (profile, active_ime_kind, label) in owned_actuation_cases() {
            let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan(
                    &ev,
                    profile,
                    true,
                    active_ime_kind
                ),
                PhysicalKeyDisposition::Suppress,
                "{label}: shadow_toggle 発火時の KeyDown は awase が既に apply-ime 済みのため Suppress"
            );
        }
    }

    /// 2026-08-05 実機: NICOLA の物理「IME ON」キー（scan 0x70）は IME が既に
    /// 目的の状態にある時に押されると `VK_DBE_HIRAGANA` (0xF2) ではなく `VK_DBE_*` が
    /// 生成されることがある。awase が beliefに基づく開閉トグルとして書く 0xF3/0xF4
    /// （ADR-189/191）は、shadow_toggle が不発（既に目的の状態）でも、素通しすると
    /// awase が書く開閉に加えて実 IME が同じキーを処理する二重 actuation になるため、
    /// shadow_toggled=false でも Suppress する（BUG-46/BUG-52 回帰ガード）。
    /// GJI・MS-IME本体の両方（`owned_actuation_cases`）で固定する。
    #[test]
    fn dbe_mode_keydown_suppressed_even_when_not_shadow_toggled() {
        for (vk, action, vk_label) in dbe_written_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                let ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                assert_eq!(
                    PhysicalKeyDisposition::plan(&ev, profile, false, active_ime_kind),
                    PhysicalKeyDisposition::Suppress,
                    "{vk_label} / {label}: shadow_toggle 不発でも実IMEへの意図しない \
                     モード切替を防ぐため Suppress"
                );
            }
        }
    }

    #[test]
    fn injected_dbe_mode_keydown_is_allowed_when_shadow_not_toggled() {
        let ev = injected(dbe_mode_event(
            crate::vk::VK_DBE_ALPHANUMERIC,
            ShadowImeAction::TurnOff,
            KeyEventType::KeyDown,
        ));
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::TsfNative,
                false,
                ActiveImeKind::GoogleJapaneseInput
            ),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn physical_dbe_mode_keydown_stays_suppressed() {
        let ev = dbe_mode_event(
            crate::vk::VK_DBE_SBCSCHAR,
            ShadowImeAction::Toggle,
            KeyEventType::KeyDown,
        );
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::TsfNative,
                false,
                ActiveImeKind::GoogleJapaneseInput
            ),
            PhysicalKeyDisposition::Suppress
        );
    }

    #[test]
    fn injected_kanji_keyup_is_allowed() {
        let ev = injected(kanji_event(
            KeyEventType::KeyUp,
            Some(ShadowImeAction::TurnOn),
        ));
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::TsfNative,
                false,
                ActiveImeKind::GoogleJapaneseInput
            ),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn injected_f2_is_allowed() {
        let ev = injected(f2_event(KeyEventType::KeyDown));
        assert_eq!(
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::TsfNative, false, ANY_IME_KIND),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn injected_kanji_under_immcross_profile_is_allowed() {
        let ev = injected(kanji_event(
            KeyEventType::KeyDown,
            Some(ShadowImeAction::TurnOn),
        ));
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::Standard,
                false,
                ActiveImeKind::MicrosoftIme
            ),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn input_relay_kanji_down_and_up_are_allowed() {
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            let ev = kanji_event(event_type, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan(
                    &ev,
                    AppImeProfile::InputRelay,
                    true,
                    ActiveImeKind::GoogleJapaneseInput
                ),
                PhysicalKeyDisposition::Allow,
                "{event_type:?}"
            );
        }
    }

    /// 対照実験: 同じ「shadow_toggle 不発」条件でも `VK_KANJI` 等の DBE 範囲外の
    /// 一般 KANJI キーは引き続き Allow のまま（`VK_DBE_*` 専用の例外であり、KANJI
    /// 系キー全体の挙動を変えていないことを固定する）。
    #[test]
    fn owned_actuation_keydown_allowed_when_not_shadow_toggled_is_unaffected_by_dbe_mode_fix() {
        for (profile, active_ime_kind, label) in owned_actuation_cases() {
            let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan(&ev, profile, false, active_ime_kind),
                PhysicalKeyDisposition::Allow,
                "{label}: VK_KANJI は VK_DBE_* 向け修正の影響を受けない"
            );
        }
    }

    #[test]
    fn owned_actuation_keyup_always_suppressed() {
        for (profile, active_ime_kind, label) in owned_actuation_cases() {
            for shadow_toggled in [false, true] {
                let ev = kanji_event(KeyEventType::KeyUp, Some(ShadowImeAction::TurnOn));
                assert_eq!(
                    PhysicalKeyDisposition::plan(&ev, profile, shadow_toggled, active_ime_kind),
                    PhysicalKeyDisposition::Suppress,
                    "{label}: KANJI KeyUp は shadow_toggled={shadow_toggled} でも常に Suppress \
                     (二重制御による物理キー再送を防ぐ、BUG-46)"
                );
            }
        }
    }

    // ── suppress_reason: journal 記録用ラベル（BUG-90 調査） ──
    //
    // PowerToys Mouse Without Borders 使用中に「英数」キーが効かない不具合報告
    // (docs/known-bugs.md BUG-90) の調査で、ImmCross プロファイル下では
    // VK_DBE_ALPHANUMERIC (英数) が Down/Up とも無条件 Suppress される一方、
    // VK_DBE_HIRAGANA (かな) は専用分岐で Allow される（当時は TSF mode 以外のみ、BUG-173 で常に）ことが
    // 判明した（「かなは効くが英数は効かない」という報告症状と一致）。
    // この非対称性を journal から確認できるようにする `suppress_reason` を
    // ここで固定する。

    #[test]
    fn suppress_reason_is_none_when_allowed() {
        let ev = f2_event(KeyEventType::KeyDown);
        let disposition =
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::TsfNative, false, ANY_IME_KIND);
        assert_eq!(disposition, PhysicalKeyDisposition::Allow);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            None
        );
    }

    #[test]
    fn hiragana_in_tsf_mode_has_no_suppress_reason() {
        // BUG-173: 物理 F2 は TSF mode でも Suppress されないので reason も無い。
        let ev = f2_event(KeyEventType::KeyDown);
        let disposition =
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::TsfNative, false, ANY_IME_KIND);
        assert_eq!(disposition, PhysicalKeyDisposition::Allow);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            None
        );
    }

    #[test]
    fn suppress_reason_is_imm_cross_for_dbe_mode_key_under_immcross_profile() {
        // BUG-90 調査で確認した事実の一つ: ImmCross プロファイル（`Standard`）
        // では VK_DBE_ALPHANUMERIC (英数) は shadow_toggled にも event_type
        // (Down/Up) にも関わらず常に Suppress され、journal 上は "imm-cross"
        // として記録される。ただし report2 の実データ（explorer.exe/sakura.exe、
        // いずれも非ImmCrossプロファイル）は「imm32-off」経路（下の
        // `suppress_reason_is_imm32_off_for_owned_actuation_dbe_mode_key`）で
        // 説明される。GJI 稼働時は profile を問わず英数キーが Suppress される
        // ことが症状の実体であり、ImmCross はその一経路に過ぎない
        // （docs/known-bugs.md BUG-90 参照）。
        for shadow_toggled in [false, true] {
            for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                let ev = dbe_mode_event(
                    crate::vk::VK_DBE_SBCSCHAR,
                    ShadowImeAction::Toggle,
                    event_type,
                );
                let disposition = PhysicalKeyDisposition::plan(
                    &ev,
                    AppImeProfile::Standard,
                    shadow_toggled,
                    ActiveImeKind::GoogleJapaneseInput,
                );
                assert_eq!(
                    disposition,
                    PhysicalKeyDisposition::Suppress,
                    "shadow_toggled={shadow_toggled} event_type={event_type:?} でも \
                     ImmCross は英数キーを Suppress する"
                );
                assert_eq!(
                    disposition.suppress_reason(&ev, AppImeProfile::Standard),
                    Some("imm-cross")
                );
            }
        }
    }

    #[test]
    fn suppress_reason_is_imm32_off_for_owned_actuation_dbe_mode_key() {
        let ev = dbe_mode_event(
            crate::vk::VK_DBE_SBCSCHAR,
            ShadowImeAction::Toggle,
            KeyEventType::KeyDown,
        );
        let disposition = PhysicalKeyDisposition::plan(
            &ev,
            AppImeProfile::TsfNative,
            false,
            ActiveImeKind::GoogleJapaneseInput,
        );
        assert_eq!(disposition, PhysicalKeyDisposition::Suppress);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            Some("imm32-off")
        );
    }

    // ── ADR-191: awase が書かない英数(0xF0)・カタカナ(0xF1)は Suppress せず IME へ素通し ──
    //
    // 撤去後は awase が代行しないので、握りつぶすと OS にも awase にも誰も何もしない
    // 「二重の空振り」になる。BUG-116/ADR-137 の「Shift+0xF1 だけ Allow」は、0xF1 が
    // 常に Allow になったため不要になった（Shift の有無に依らない）。

    fn with_shift(mut event: RawKeyEvent) -> RawKeyEvent {
        event.modifier_snapshot.shift = true;
        event
    }

    /// 実イベントの形: 英数・カタカナは `shadow_action` を持たない（`ImeKeyKind::shadow_effect` が
    /// `None`）。既定の Suppress でも、全プロファイル・全 IME・Down/Up・Shift の有無で Allow。
    #[test]
    fn alphanumeric_and_katakana_without_shadow_action_are_always_allowed() {
        for (vk, _action, vk_label) in dbe_unwritten_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                    for shift in [false, true] {
                        let mut ev = RawKeyEvent {
                            vk_code: vk,
                            ..kanji_event(event_type, None)
                        };
                        if shift {
                            ev = with_shift(ev);
                        }
                        {
                            assert_eq!(
                                PhysicalKeyDisposition::plan(&ev, profile, false, active_ime_kind),
                                PhysicalKeyDisposition::Allow,
                                "{vk_label} / {label} / {event_type:?} / shift={shift}: \
                                 awase が書かないキーは IME へ素通し（ADR-191）"
                            );
                        }
                    }
                }
            }
        }
    }

    /// 撤去前の合成イベント（`shadow_action` あり）でも、`shadow_toggled=false` の KeyDown は
    /// 既定の Suppress で握りつぶされない（Suppress の根拠は「awase が書くキー」だけ）。
    #[test]
    fn alphanumeric_and_katakana_keydown_not_suppressed_even_with_synthetic_action() {
        for (vk, action, vk_label) in dbe_unwritten_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                for shift in [false, true] {
                    let mut ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                    if shift {
                        ev = with_shift(ev);
                    }
                    assert_eq!(
                        PhysicalKeyDisposition::plan(&ev, profile, false, active_ime_kind),
                        PhysicalKeyDisposition::Allow,
                        "{vk_label} / {label} / shift={shift}: \
                         awase が書かない英数/カタカナを握りつぶさない（ADR-191）"
                    );
                }
            }
        }
    }

    /// `plan()` の判定そのもの: `shadow_action=Some(Toggle)` を持つ半角/全角（0xF3/0xF4、GJI・MS-IME本体の両方）は、
    /// `modifier_snapshot.shift` の値に依らず Suppress される（`plan()` は Shift を見ない）。
    ///
    /// **本番ではこの組み合わせ（shift=true かつ shadow_action あり）は生成されない**: `enrich_key_role` は
    /// 修飾キー付きの物理キーに `shadow_action` を付けないので、Shift+半角/全角は `is_kanji_event=false` で **Allow**
    /// （IME 側で別の意味を持ちうるため）。この名前を「Shift+半角/全角も Suppress される」と読まないこと（round2 B-NB4）。
    /// なお修飾キーを途中で押す/離すと、同じ物理キーの KeyDown（無修飾で Suppress）と KeyUp（修飾ありで Allow）の
    /// 配送が非対称になりうる（実害未確認、稀な操作）。
    #[test]
    fn plan_suppresses_toggle_hankaku_zenkaku_keydown_regardless_of_modifier_snapshot() {
        for (vk, action, vk_label) in dbe_written_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                for shift in [false, true] {
                    let mut ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                    if shift {
                        ev = with_shift(ev);
                    }
                    assert_eq!(
                        PhysicalKeyDisposition::plan(&ev, profile, false, active_ime_kind),
                        PhysicalKeyDisposition::Suppress,
                        "{vk_label} / {label} / shift={shift}: awase が beliefトグルとして書く \
                         キーは二重 actuation 防止のため Suppress（ADR-189/191）"
                    );
                }
            }
        }
    }

    /// ADR-195追記: 採用中の学習表が半角/全角を開閉トグルでないと示すと（または役割が無いと）`enrich_key_role`は
    /// `shadow_action`を付けない（`None`）。このとき GJI の ImmCross（Standard）・GjiDirect
    /// （Imm32Unavailable/TsfNative）のいずれでも 0xF3/0xF4 は Down も Up も Allow（KeyDownだけが残らない）。
    #[test]
    fn gji_hankaku_zenkaku_without_shadow_action_is_allowed_down_and_up() {
        let mut checked = 0;
        for vk in [crate::vk::VK_DBE_SBCSCHAR, crate::vk::VK_DBE_DBCSCHAR] {
            for profile in [
                AppImeProfile::Standard,
                AppImeProfile::Imm32Unavailable,
                AppImeProfile::TsfNative,
            ] {
                for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                    let ev = RawKeyEvent {
                        vk_code: vk,
                        ..kanji_event(event_type, None)
                    };
                    assert_eq!(
                        PhysicalKeyDisposition::plan(
                            &ev,
                            profile,
                            false,
                            ActiveImeKind::GoogleJapaneseInput
                        ),
                        PhysicalKeyDisposition::Allow,
                        "{vk:?} / {profile:?} / {event_type:?}: shadow_action=None のGJI半角/全角は素通し"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 12);
    }

    // 決定表の絞り込みを書くときは「対象行が空でないこと」を assert すること。ラベルを完全一致で絞り、実際のラベルが
    // `"VK_DBE_SBCSCHAR (0xF3)"` のように後置きを持つために対象行が0件になったテストが、Linux では走らず
    // windows-build で初めて失敗した実例がある（`kanji_family_keyup_suppress_verdict_is_independent_of_specific_vk`）。

    // ── ADR-166: plan() の全数決定表 ──
    //
    // `src/engine/nicola_fsm.rs::run_flush_matrix`（BUG-129）と同じパターン:
    // VK種別ごとに意味のある軸だけを総当たりし、各行を`PlanRow`として記録する。
    // 上記の個別サンプルテスト（36件）は削除せず維持し、本節は「見落としの
    // 空白セルがないか」を横断的に確認する独立した第二の防衛線として追加する。
    // 決定表そのものの解説は`docs/adr/166-physical-key-disposition-decision-table.md`
    // を参照（本テストは決定表の内容を機械的に固定するのが目的で、決定表
    // 自体の可読な説明はADR側が担う）。
    //
    // BUG-131（今回のカタカナ固着バグ）は`plan()`自体の誤りではなく、
    // `plan()`が返すSuppress判定の**根拠(vk種別)**を、別のコード
    // (`key_pipeline.rs::kp_restore_hiragana_for_suppressed_mode_key`)が
    // 「KeyDownと同じvk_codeのKeyUpが来る」という`plan()`が保証していない
    // 前提で誤読していたことが原因だった。`kanji_family_keyup_suppress_
    // verdict_is_independent_of_specific_vk`は、その「`plan()`側は
    // vk種別を問わず一貫している」という性質自体を固定する。

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct PlanRow {
        vk_label: &'static str,
        event_type: KeyEventType,
        profile: AppImeProfile,
        shadow_toggled: bool,
        active_ime_kind: ActiveImeKind,
        injected: bool,
        result: PhysicalKeyDisposition,
    }

    const ALL_PROFILES: [AppImeProfile; 4] = [
        AppImeProfile::Standard,
        AppImeProfile::Imm32Unavailable,
        AppImeProfile::TsfNative,
        AppImeProfile::InputRelay,
    ];
    const ALL_EVENT_TYPES: [KeyEventType; 2] = [KeyEventType::KeyDown, KeyEventType::KeyUp];
    const ALL_IME_KINDS: [ActiveImeKind; 2] = [
        ActiveImeKind::GoogleJapaneseInput,
        ActiveImeKind::MicrosoftIme,
    ];
    const ALL_BOOLS: [bool; 2] = [false, true];

    /// `kanji_family_keyup_suppress_verdict_is_independent_of_specific_vk`が
    /// 行のグルーピングに使う、vk種別を除いた入力キー。
    type PlanKey = (AppImeProfile, bool, ActiveImeKind, bool);

    #[expect(clippy::too_many_lines)]
    fn run_plan_matrix() -> Vec<PlanRow> {
        let mut rows = Vec::new();

        // 1. VK_DBE_HIRAGANA (0xF2): 専用分岐。shadow_toggled/active_ime_kind/
        //    shift/半角トグル/親指キー設定はどれも参照されないため
        //    固定値(既定値)1通りに絞る。
        for &event_type in &ALL_EVENT_TYPES {
            for &profile in &ALL_PROFILES {
                for &injected in &ALL_BOOLS {
                    let mut ev = f2_event(event_type);
                    ev.injected = injected;
                    let result = PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        false,
                        ActiveImeKind::GoogleJapaneseInput,
                    );
                    rows.push(PlanRow {
                        vk_label: "VK_DBE_HIRAGANA",
                        event_type,
                        profile,
                        shadow_toggled: false,
                        active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
                        injected,
                        result,
                    });
                }
            }
        }

        // 2. DBEモードキー (0xF0/0xF1/0xF3/0xF4): awase が書くキー(0xF3/0xF4)と書かない
        //    キー(0xF0/0xF1)の違いは`plan()`の`shadow_action`（役割由来の`Some(Toggle)`）だけで決まる
        //    （ADR-191。Shift/半角トグル/親指キー設定は参照されない）。
        for &(vk, action, label) in &dbe_mode_vks() {
            for &event_type in &ALL_EVENT_TYPES {
                for &profile in &ALL_PROFILES {
                    for &shadow_toggled in &ALL_BOOLS {
                        for &active_ime_kind in &ALL_IME_KINDS {
                            for &injected in &ALL_BOOLS {
                                if injected && shadow_toggled {
                                    continue;
                                }
                                let mut ev = dbe_mode_event(vk, action, event_type);
                                ev.injected = injected;
                                let result = PhysicalKeyDisposition::plan(
                                    &ev,
                                    profile,
                                    shadow_toggled,
                                    active_ime_kind,
                                );
                                rows.push(PlanRow {
                                    vk_label: label,
                                    event_type,
                                    profile,
                                    shadow_toggled,
                                    active_ime_kind,
                                    injected,
                                    result,
                                });
                            }
                        }
                    }
                }
            }
        }

        // 4. VK_CONVERT/VK_NONCONVERT (ADR-141): event_type/profile自体は
        //    この分岐で参照されない（injected/InputRelayの2つの早期returnにのみ
        //    関与）ため、その短絡を確認する目的で回す。
        for &vk in &[crate::vk::VK_CONVERT, crate::vk::VK_NONCONVERT] {
            for &event_type in &ALL_EVENT_TYPES {
                for &profile in &ALL_PROFILES {
                    for &injected in &ALL_BOOLS {
                        let mut ev = henkan_muhenkan_event(vk, None, event_type);
                        ev.injected = injected;
                        let result = PhysicalKeyDisposition::plan(
                            &ev,
                            profile,
                            false,
                            ActiveImeKind::GoogleJapaneseInput,
                        );
                        rows.push(PlanRow {
                            vk_label: if vk == crate::vk::VK_CONVERT {
                                "VK_CONVERT"
                            } else {
                                "VK_NONCONVERT"
                            },
                            event_type,
                            profile,
                            shadow_toggled: false,
                            active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
                            injected,
                            result,
                        });
                    }
                }
            }
        }

        // 5. 一般KANJI系VK（shadow_action=Some、DBEモードキー集合には属さない
        //    例: VK_KANJI）。`is_dbe_mode_key_down`はこのVK群には無関係だが、
        //    「無関係であること」自体を確認するため回す。
        for &event_type in &ALL_EVENT_TYPES {
            for &profile in &ALL_PROFILES {
                for &shadow_toggled in &ALL_BOOLS {
                    for &active_ime_kind in &ALL_IME_KINDS {
                        for &injected in &ALL_BOOLS {
                            if injected && shadow_toggled {
                                continue;
                            }
                            let mut ev = kanji_event(event_type, Some(ShadowImeAction::Toggle));
                            ev.injected = injected;
                            let result = PhysicalKeyDisposition::plan(
                                &ev,
                                profile,
                                shadow_toggled,
                                active_ime_kind,
                            );
                            rows.push(PlanRow {
                                vk_label: "VK_KANJI(generic)",
                                event_type,
                                profile,
                                shadow_toggled,
                                active_ime_kind,
                                injected,
                                result,
                            });
                        }
                    }
                }
            }
        }

        // 6. 非KANJI系VK（shadow_action=None）。常にAllowのはず。
        for &event_type in &ALL_EVENT_TYPES {
            for &profile in &ALL_PROFILES {
                for &injected in &ALL_BOOLS {
                    let mut ev = non_kanji_event(event_type);
                    ev.injected = injected;
                    let result = PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        false,
                        ActiveImeKind::GoogleJapaneseInput,
                    );
                    rows.push(PlanRow {
                        vk_label: "non-kanji",
                        event_type,
                        profile,
                        shadow_toggled: false,
                        active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
                        injected,
                        result,
                    });
                }
            }
        }

        rows
    }

    /// `run_plan_matrix`は約380行の決定表を生成するため、複数のテストが
    /// 同じ表を参照する場合はここで1回だけ計算してキャッシュする
    /// （/code-review指摘、PR #206棚卸し。以前は3テストが独立に呼び毎回
    /// 全行を再生成していた）。
    fn cached_plan_matrix() -> &'static Vec<PlanRow> {
        static CACHE: std::sync::OnceLock<Vec<PlanRow>> = std::sync::OnceLock::new();
        CACHE.get_or_init(run_plan_matrix)
    }

    /// `run_plan_matrix`が全行を構築できること自体が「任意の入力でpanicしない」
    /// を実質的に検証する（`conv_classify.rs`の同種コメント参照）。
    #[test]
    fn plan_matrix_covers_all_branches_without_panicking() {
        let rows = cached_plan_matrix();
        assert!(
            rows.len() > 300,
            "決定表が想定より小さい: {} 行",
            rows.len()
        );
    }

    /// BUG-131の背景となった性質そのものを固定する: `is_kanji_event`な
    /// DBEモードキー群のKeyUpに対するSuppress判定は、`profile`/
    /// `shadow_toggled`/`active_ime_kind`/`injected`/が同じなら
    /// **vkの種類（0xF0/0xF1/0xF3/0xF4のどれか）に依存しない**。
    /// `plan()`自身はこの性質を最初から満たしており、BUG-131は`plan()`の
    /// 外側（`kp_restore_hiragana_for_suppressed_mode_key`）がKeyDown/KeyUpの
    /// vk一致を誤って前提にしたことが原因だった、という対比を残す。
    #[test]
    fn kanji_family_keyup_suppress_verdict_is_independent_of_specific_vk() {
        let rows = cached_plan_matrix();
        let dbe_family = [
            "VK_DBE_ALPHANUMERIC",
            "VK_DBE_KATAKANA",
            "VK_DBE_SBCSCHAR",
            "VK_DBE_DBCSCHAR",
        ];
        let relevant: Vec<&PlanRow> = rows
            .iter()
            .filter(|r| {
                // ラベルは "VK_DBE_SBCSCHAR (0xF3)" のように16進を後置している
                dbe_family.iter().any(|f| r.vk_label.starts_with(f))
                    && r.event_type == KeyEventType::KeyUp
            })
            .collect();
        assert!(!relevant.is_empty());

        let mut seen: Vec<(PlanKey, PhysicalKeyDisposition, &'static str)> = Vec::new();
        for row in relevant {
            let key: PlanKey = (
                row.profile,
                row.shadow_toggled,
                row.active_ime_kind,
                row.injected,
            );
            if let Some((_, result, first_vk)) = seen.iter().find(|(k, _, _)| *k == key) {
                assert_eq!(
                    *result, row.result,
                    "vk={first_vk}(先着) と vk={}(今回) でKeyUpのSuppress判定が \
                     食い違う: key={key:?}",
                    row.vk_label
                );
            } else {
                seen.push((key, row.result, row.vk_label));
            }
        }
    }

    /// issue #136/BUG-90決定4: InputRelayプロファイルは他のどの軸の値でも
    /// 常にAllow（awaseはこの窓のactuationを所有しない）。
    #[test]
    fn input_relay_always_allows_regardless_of_other_axes() {
        let rows = cached_plan_matrix();
        let input_relay_rows: Vec<&PlanRow> = rows
            .iter()
            .filter(|r| r.profile == AppImeProfile::InputRelay)
            .collect();
        assert!(!input_relay_rows.is_empty());
        for row in input_relay_rows {
            assert_eq!(
                row.result,
                PhysicalKeyDisposition::Allow,
                "InputRelayプロファイルは常にAllowのはず(issue #136/BUG-90決定4): {row:?}"
            );
        }
    }

    // ── F13〜F24（ADR-199 決定18(iii)）: 「その打鍵の最初の Down で awase が実際に書いたときだけ」Suppress ──

    fn fkey_event(
        event_type: KeyEventType,
        was_down: bool,
        shadow_action: Option<ShadowImeAction>,
    ) -> RawKeyEvent {
        RawKeyEvent {
            was_down,
            vk_code: VkCode(0x7C),
            ..kanji_event(event_type, shadow_action)
        }
    }

    /// 全プロファイル・全 IME 種別で同じ規則（ImmCross でも「書かなかった打鍵」は Allow）。
    fn fkey_disposition(ev: &RawKeyEvent, shadow_toggled: bool) -> PhysicalKeyDisposition {
        let mut seen = None;
        for profile in [AppImeProfile::Standard, AppImeProfile::TsfNative] {
            for kind in [
                ActiveImeKind::GoogleJapaneseInput,
                ActiveImeKind::MicrosoftIme,
            ] {
                let d = PhysicalKeyDisposition::plan(ev, profile, shadow_toggled, kind);
                assert!(
                    seen.is_none_or(|p| p == d),
                    "{profile:?}/{kind:?} で規則が変わってはいけない"
                );
                seen = Some(d);
            }
        }
        seen.unwrap()
    }

    #[test]
    fn fkey_first_down_is_suppressed_only_when_awase_wrote() {
        let down = fkey_event(KeyEventType::KeyDown, false, Some(ShadowImeAction::Toggle));
        assert_eq!(
            fkey_disposition(&down, true),
            PhysicalKeyDisposition::Suppress
        );
        // `shadow_action` が暫定で付いていても、書かなかった（`shadow_toggled=false`）なら Allow。
        assert_eq!(
            fkey_disposition(&down, false),
            PhysicalKeyDisposition::Allow
        );
    }

    /// 同期キー（`keys.ime_detect`）に F キーを書いて `shadow_toggled` が立っても、役割由来でなければ（`shadow_action=None`）
    /// Allow のまま（決定9。Suppress すると belief だけ反転して IME に届かない）。
    #[test]
    fn fkey_sync_key_toggle_without_role_stays_allowed() {
        let mut down = fkey_event(KeyEventType::KeyDown, false, None);
        down.ime_relevance.sync_direction = Some(ShadowImeAction::Toggle);
        assert_eq!(fkey_disposition(&down, true), PhysicalKeyDisposition::Allow);
    }

    #[test]
    fn fkey_repeat_and_up_follow_the_latched_shadow_action() {
        for (event_type, was_down) in [
            (KeyEventType::KeyDown, true), // 自動リピート
            (KeyEventType::KeyUp, true),
        ] {
            // ラッチが「書いた」を持ち越した（`shadow_action=Some`）→ Suppress。`shadow_toggled` は見ない。
            let wrote = fkey_event(event_type, was_down, Some(ShadowImeAction::Toggle));
            assert_eq!(
                fkey_disposition(&wrote, false),
                PhysicalKeyDisposition::Suppress
            );
            // 書かなかった（ラッチ `None`、または scan 不一致で `None`）→ Allow。
            let passive = fkey_event(event_type, was_down, None);
            assert_eq!(
                fkey_disposition(&passive, true),
                PhysicalKeyDisposition::Allow
            );
        }
    }

    #[test]
    fn fkey_injected_is_always_allowed_and_labelled_role_fkey_when_suppressed() {
        let mut ev = fkey_event(KeyEventType::KeyDown, false, Some(ShadowImeAction::Toggle));
        ev.injected = true;
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::Standard,
                false,
                ActiveImeKind::GoogleJapaneseInput
            ),
            PhysicalKeyDisposition::Allow
        );
        let ev = fkey_event(KeyEventType::KeyDown, false, Some(ShadowImeAction::Toggle));
        let d = PhysicalKeyDisposition::plan(
            &ev,
            AppImeProfile::Standard,
            true,
            ActiveImeKind::GoogleJapaneseInput,
        );
        assert_eq!(
            d.suppress_reason(&ev, AppImeProfile::Standard),
            Some("role-fkey")
        );
    }
}
