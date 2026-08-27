use std::collections::HashSet;

use awase::config::DbeModeKeyPolicy;
use awase::types::{KeyEventType, RawKeyEvent, VkCode};

use crate::focus::class_names::AppImeProfile;
use crate::state::key_sequence_policy;
use crate::tsf::observer::ActiveImeKind;
use crate::vk::VkCodeExt as _;

/// 元の物理キーイベントを OS に届けるかどうかの配送判断。
/// `Decision`（意味論）とは独立した配送機構上の判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhysicalKeyDisposition {
    /// 元の物理キーイベントをそのまま OS に通す
    Allow,
    /// 元の物理キーイベントを消費（OS に届けない）
    Suppress,
}

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
        Some(if event.vk_code == crate::vk::VK_DBE_HIRAGANA {
            "tsf-f2"
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
/// 場合がある）ではエントリが残留し得るが、BUG-46 の修正で KANJI 系 KeyUp は常に
/// Suppress されるようになり `check_output_guard_defer` に到達しなくなったため inert。
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
            log::debug!(
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
            log::debug!(
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
    /// 物理キーを OS に届けるかどうかの純粋関数。
    ///
    /// **F2 (VK_DBE_HIRAGANA)**:
    /// - TSF mode かつ `f2_warmup_owned=true`（GJI 戦略）: Down/Up 共に Suppress。
    ///   awase 自身が warmup として SendInput(F2) を再送する契約とセットの
    ///   double-F2 防止（`send_eager_tsf_warmup` の NativeF2Consumed 代替送信）。
    /// - TSF mode かつ `f2_warmup_owned=false`（MsImeStrategy）: **Allow**。
    ///   MS-IME 戦略は F2 warmup を送らない（`needs_f2_probe()=false`）ため、
    ///   ここで消すと物理ひらがなキーが「食い逃げ」され、intent/Engine だけ ON で
    ///   実 IME が OFF のまま乖離する（BUG-10、2026-07-06 実機）。MS-IME は
    ///   VK_DBE_HIRAGANA をネイティブ処理して IME ON にするため素通しが正しい。
    /// - 非 TSF mode: Allow
    ///
    /// **KANJI 関連キー**:
    /// - ImmCross プロファイル: Down/Up 共に Suppress（spurious 連鎖を構造的に遮断）
    /// - それ以外（Imm32Unavailable / TsfNative）: `apply-ime` が `GjiDirectStrategy` /
    ///   `MsImeDirectStrategy` で実際に actuate する場合（`ime_actuation_owned`）のみ、
    ///   shadow_toggle 発火時 KeyDown と全 KeyUp を Suppress。
    ///   **例外: `VK_DBE_*`（0xF0 ALPHANUMERIC / 0xF1 KATAKANA / 0xF3 SBCSCHAR /
    ///   0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で別処理）の KeyDown は
    ///   `shadow_toggled` に関わらず常に Suppress**（`ime_actuation_owned` の場合）。
    ///   NICOLA の物理「IME ON」キー（scan 0x70）は、IME が既に目的の状態にある時に
    ///   押されると `VK_DBE_HIRAGANA` (0xF2) の代わりにこれらの `VK_DBE_*` を生成する
    ///   ことがある（実機で 0xF0/0xF1 を確認）。`VK_KANJI` 等と違い `VK_DBE_*` は
    ///   素通しすると実 IME（MS-IME）が Windows 標準仕様どおりネイティブ効果
    ///   （英数/カタカナ/半角/全角への切替）を能動的に実行してしまうため、toggle が
    ///   発火したかどうかに関係なく漏らしてはならない（2026-08-05 実機、
    ///   `docs/known-bugs.md` BUG-52 参照）。
    ///
    /// `ime_actuation_owned` を profile 単独ではなく `ActiveImeKind` からも導出するのは、
    /// TsfNative（Windows Terminal 等）で GJI が起動している場合に awase 自身の
    /// `SendInput(VK_IME_ON/OFF)`（`GjiDirectStrategy`）と、素通しされた元の物理 KANJI 系
    /// キーの reinject が **二重に actuate** してしまうため（BUG-46）。旧実装は
    /// `profile.should_pass_physical_key()`（TsfNative で常に true）のみで判定しており、
    /// 「TSF が KANJI を正しく処理する」という前提が `GjiDirectStrategy` の全プロファイル
    /// 適用化（`ime_controller.rs`）より前のまま残っていたことが原因だった。
    pub(crate) fn plan(
        event: &RawKeyEvent,
        profile: AppImeProfile,
        shadow_toggled: bool,
        is_tsf_mode: bool,
        f2_warmup_owned: bool,
        active_ime_kind: ActiveImeKind,
        dbe_mode_key_policy: DbeModeKeyPolicy,
    ) -> Self {
        // F2 (VK_DBE_HIRAGANA): TSF mode かつ warmup 戦略が F2 を自前送信する場合のみ Suppress
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA {
            return if is_tsf_mode && f2_warmup_owned {
                Self::Suppress
            } else {
                Self::Allow
            };
        }

        let is_kanji_event = event.ime_relevance.shadow_action.is_some();
        if !is_kanji_event {
            return Self::Allow;
        }
        let suppress = if profile.can_use_imm32_cross_process() {
            // ImmCross: KANJI 関連 VK は Down/Up 共に Suppress
            true
        } else {
            // apply-ime が GjiDirect/MsImeDirect で実際に actuate する場合のみ、
            // shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress（BUG-46）。
            let ime_actuation_owned = key_sequence_policy::gji_direct_applicable(active_ime_kind)
                || key_sequence_policy::ms_ime_direct_applicable(active_ime_kind, profile);
            // VK_DBE_* (0xF0 ALPHANUMERIC / 0xF1 KATAKANA / 0xF3 SBCSCHAR / 0xF4
            // DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で既に処理済みのためここには
            // 来ない) の KeyDown は shadow_toggled に関わらず常に Suppress。
            // 素通しすると実IME（MS-IME）がWindows標準仕様どおり能動的にネイティブ
            // 効果（英数/カタカナ/半角/全角への切替）を適用してしまうため、
            // VK_KANJI 等と違い「toggleが不発だったから安全に通してよい」という
            // 前提が成り立たない（2026-08-05実機、0xF0/0xF1 で確認、known-bugs.md）。
            //
            // `dbe_mode_key_policy = Passthrough`（隠し設定、既定 Suppress で
            // 現状維持、ADR-091 §D3.6）ならこの追加 Suppress 条件自体を無効化する
            // ——上級者が BUG-52 のリスクを引き受けて素のパススルーを選んだ場合の
            // 抜け道。`shadow_toggled`/KeyUp 側の既存 Suppress 条件は変更しない。
            let is_dbe_mode_key_down = matches!(dbe_mode_key_policy, DbeModeKeyPolicy::Suppress)
                && matches!(
                    event.vk_code,
                    crate::vk::VK_DBE_ALPHANUMERIC
                        | crate::vk::VK_DBE_KATAKANA
                        | crate::vk::VK_DBE_SBCSCHAR
                        | crate::vk::VK_DBE_DBCSCHAR
                )
                && event.event_type == KeyEventType::KeyDown;
            ime_actuation_owned
                && (shadow_toggled
                    || is_dbe_mode_key_down
                    || matches!(event.event_type, KeyEventType::KeyUp))
        };
        if suppress {
            Self::Suppress
        } else {
            Self::Allow
        }
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
            vk_code,
            ..kanji_event(event_type, Some(action))
        }
    }

    /// BUG-52 の対象 VK_DBE_* 一覧（0xF2 HIRAGANA は専用分岐で別処理のため対象外）。
    fn dbe_mode_vks() -> Vec<(VkCode, ShadowImeAction, &'static str)> {
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
            (
                crate::vk::VK_DBE_SBCSCHAR,
                ShadowImeAction::TurnOff,
                "VK_DBE_SBCSCHAR (0xF3)",
            ),
            (
                crate::vk::VK_DBE_DBCSCHAR,
                ShadowImeAction::TurnOn,
                "VK_DBE_DBCSCHAR (0xF4)",
            ),
        ]
    }

    fn f2_event(event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: crate::vk::VK_DBE_HIRAGANA,
            ..kanji_event(event_type, None)
        }
    }

    // F2/非KANJI テストでは ime_actuation_owned 判定に到達しないため、
    // active_ime_kind はどちらでもよい filler として GoogleJapaneseInput を使う。
    const ANY_IME_KIND: ActiveImeKind = ActiveImeKind::GoogleJapaneseInput;

    // ── F2 (VK_DBE_HIRAGANA): TSF mode 判定は KANJI/shadow_toggle と独立 ──

    #[test]
    fn f2_tsf_mode_suppresses_down_and_up() {
        let ev = f2_event(KeyEventType::KeyDown);
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::TsfNative,
                false,
                true,
                true,
                ANY_IME_KIND,
                DbeModeKeyPolicy::Suppress
            ),
            PhysicalKeyDisposition::Suppress
        );
        let ev = f2_event(KeyEventType::KeyUp);
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::TsfNative,
                false,
                true,
                true,
                ANY_IME_KIND,
                DbeModeKeyPolicy::Suppress
            ),
            PhysicalKeyDisposition::Suppress,
            "TSF mode では F2 Up も double-F2 防止のため Suppress"
        );
    }

    /// BUG-10 回帰: MsImeStrategy（f2_warmup_owned=false）では TSF mode でも物理 F2 を通す。
    /// Suppress すると代替の F2 warmup が送られず、ユーザーの物理ひらがなキーが
    /// 食い逃げされて「Engine ON なのに実 IME OFF」の乖離を作る（2026-07-06 実機）。
    #[test]
    fn f2_tsf_mode_msime_strategy_allows_physical_key() {
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            let ev = f2_event(event_type);
            assert_eq!(
                PhysicalKeyDisposition::plan(
                    &ev,
                    AppImeProfile::TsfNative,
                    false,
                    true,
                    false,
                    ANY_IME_KIND,
                    DbeModeKeyPolicy::Suppress
                ),
                PhysicalKeyDisposition::Allow,
                "MsImeStrategy は F2 warmup を送らないため物理 F2 ({event_type:?}) を素通しする"
            );
        }
    }

    #[test]
    fn f2_non_tsf_mode_allows() {
        let ev = f2_event(KeyEventType::KeyDown);
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::Standard,
                false,
                false,
                false,
                ANY_IME_KIND,
                DbeModeKeyPolicy::Suppress
            ),
            PhysicalKeyDisposition::Allow
        );
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
                                false,
                                false,
                                active_ime_kind,
                                DbeModeKeyPolicy::Suppress
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
                        false,
                        false,
                        ActiveImeKind::MicrosoftIme,
                        DbeModeKeyPolicy::Suppress
                    ),
                    PhysicalKeyDisposition::Suppress,
                    "ImmCross (Standard) は shadow_toggled={shadow_toggled} event_type={event_type:?} \
                     でも常に Suppress (spurious VK_F3/F4 連鎖の根本修正、08b8661)"
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
                PhysicalKeyDisposition::plan(
                    &ev,
                    profile,
                    false,
                    false,
                    false,
                    active_ime_kind,
                    DbeModeKeyPolicy::Suppress
                ),
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
                    false,
                    false,
                    active_ime_kind,
                    DbeModeKeyPolicy::Suppress
                ),
                PhysicalKeyDisposition::Suppress,
                "{label}: shadow_toggle 発火時の KeyDown は awase が既に apply-ime 済みのため Suppress"
            );
        }
    }

    /// 2026-08-05 実機: NICOLA の物理「IME ON」キー（scan 0x70）は IME が既に
    /// 目的の状態にある時に押されると、`VK_DBE_HIRAGANA` (0xF2) ではなく
    /// `VK_DBE_ALPHANUMERIC` (0xF0) や `VK_DBE_KATAKANA` (0xF1) が生成されることが
    /// ある（実機ログで両方確認）。この場合 shadow_toggle は不発（既に目的の状態）
    /// となるが、これら `VK_DBE_*` を素通しすると実 IME が能動的にネイティブ効果
    /// （英数/カタカナ/半角/全角への切替）を適用してしまうため、`VK_KANJI` 等と
    /// 異なり shadow_toggled=false でも Suppress する必要がある（0xF3/0xF4 は
    /// 実機での漏洩は未確認だが、同じコードパスを通るため同様に対象とする）。
    #[test]
    fn dbe_mode_keydown_suppressed_even_when_not_shadow_toggled() {
        for (vk, action, vk_label) in dbe_mode_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                let ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        false,
                        false,
                        false,
                        active_ime_kind,
                        DbeModeKeyPolicy::Suppress
                    ),
                    PhysicalKeyDisposition::Suppress,
                    "{vk_label} / {label}: shadow_toggle 不発でも実IMEへの意図しない \
                     モード切替を防ぐため Suppress"
                );
            }
        }
    }

    /// ADR-091 §D3.6: `dbe_mode_key_policy = Passthrough`（隠し設定、上級者向け）
    /// を選んだ場合、上のテストと対照的に BUG-52 の追加 Suppress 条件が外れ、
    /// DBE レンジキーの KeyDown が Allow になる（shadow_toggled=false の場合）。
    /// `shadow_toggled=true`（明示的にトグル発火）の場合は
    /// `owned_actuation_keydown_suppressed_when_shadow_toggled` の Suppress が
    /// 別条件として引き続き効くため対象外。
    #[test]
    fn dbe_mode_keydown_allowed_when_policy_is_passthrough() {
        for (vk, action, vk_label) in dbe_mode_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                let ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        false,
                        false,
                        false,
                        active_ime_kind,
                        DbeModeKeyPolicy::Passthrough
                    ),
                    PhysicalKeyDisposition::Allow,
                    "{vk_label} / {label}: dbe_mode_key_policy=Passthrough なら \
                     shadow_toggle 不発の DBE レンジキーは Allow"
                );
            }
        }
    }

    /// `dbe_mode_key_policy=Passthrough` でも、`shadow_toggled=true`（awase 自身が
    /// 意図した切替）の KeyDown は引き続き Suppress される。Passthrough が緩めるのは
    /// 「shadow_toggle 不発の DBE レンジキー」という BUG-52 の再現条件のみであり、
    /// awase が能動的に actuate した場面まで緩めてはならない。
    #[test]
    fn dbe_mode_keydown_still_suppressed_when_shadow_toggled_even_with_passthrough() {
        for (vk, action, vk_label) in dbe_mode_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                let ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        true,
                        false,
                        false,
                        active_ime_kind,
                        DbeModeKeyPolicy::Passthrough
                    ),
                    PhysicalKeyDisposition::Suppress,
                    "{vk_label} / {label}: dbe_mode_key_policy=Passthrough でも \
                     shadow_toggled=true の KeyDown は引き続き Suppress"
                );
            }
        }
    }

    /// `dbe_mode_key_policy=Passthrough` でも、DBE レンジキーの KeyUp は
    /// （`shadow_toggled` の値に関わらず）引き続き Suppress される
    /// （`owned_actuation_keyup_always_suppressed` と同じ既存条件、Passthrough は
    /// `is_dbe_mode_key_down` のゲートにのみ作用し KeyUp 側の条件は変更しない）。
    #[test]
    fn dbe_mode_keyup_still_suppressed_with_passthrough() {
        for (vk, action, _vk_label) in dbe_mode_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                for shadow_toggled in [false, true] {
                    let ev = dbe_mode_event(vk, action, KeyEventType::KeyUp);
                    assert_eq!(
                        PhysicalKeyDisposition::plan(
                            &ev,
                            profile,
                            shadow_toggled,
                            false,
                            false,
                            active_ime_kind,
                            DbeModeKeyPolicy::Passthrough
                        ),
                        PhysicalKeyDisposition::Suppress,
                        "{label}: dbe_mode_key_policy=Passthrough でも DBE レンジ \
                         KeyUp は shadow_toggled={shadow_toggled} に関わらず Suppress"
                    );
                }
            }
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
                PhysicalKeyDisposition::plan(
                    &ev,
                    profile,
                    false,
                    false,
                    false,
                    active_ime_kind,
                    DbeModeKeyPolicy::Suppress
                ),
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
                    PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        shadow_toggled,
                        false,
                        false,
                        active_ime_kind,
                        DbeModeKeyPolicy::Suppress
                    ),
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
    // VK_DBE_HIRAGANA (かな) は専用分岐で TSF mode 以外 Allow されることが
    // 判明した（「かなは効くが英数は効かない」という報告症状と一致）。
    // この非対称性を journal から確認できるようにする `suppress_reason` を
    // ここで固定する。

    #[test]
    fn suppress_reason_is_none_when_allowed() {
        let ev = f2_event(KeyEventType::KeyDown);
        let disposition = PhysicalKeyDisposition::plan(
            &ev,
            AppImeProfile::TsfNative,
            false,
            false, // 非 TSF mode → Allow
            true,
            ANY_IME_KIND,
            DbeModeKeyPolicy::Suppress,
        );
        assert_eq!(disposition, PhysicalKeyDisposition::Allow);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            None
        );
    }

    #[test]
    fn suppress_reason_is_tsf_f2_for_hiragana_in_tsf_mode() {
        let ev = f2_event(KeyEventType::KeyDown);
        let disposition = PhysicalKeyDisposition::plan(
            &ev,
            AppImeProfile::TsfNative,
            false,
            true,
            true,
            ANY_IME_KIND,
            DbeModeKeyPolicy::Suppress,
        );
        assert_eq!(disposition, PhysicalKeyDisposition::Suppress);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            Some("tsf-f2")
        );
    }

    #[test]
    fn suppress_reason_is_imm_cross_for_alphanumeric_under_immcross_profile() {
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
                    crate::vk::VK_DBE_ALPHANUMERIC,
                    ShadowImeAction::TurnOff,
                    event_type,
                );
                let disposition = PhysicalKeyDisposition::plan(
                    &ev,
                    AppImeProfile::Standard,
                    shadow_toggled,
                    false,
                    false,
                    ActiveImeKind::GoogleJapaneseInput,
                    DbeModeKeyPolicy::Suppress,
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
            crate::vk::VK_DBE_ALPHANUMERIC,
            ShadowImeAction::TurnOff,
            KeyEventType::KeyDown,
        );
        let disposition = PhysicalKeyDisposition::plan(
            &ev,
            AppImeProfile::TsfNative,
            false,
            false,
            false,
            ActiveImeKind::GoogleJapaneseInput,
            DbeModeKeyPolicy::Suppress,
        );
        assert_eq!(disposition, PhysicalKeyDisposition::Suppress);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            Some("imm32-off")
        );
    }
}
