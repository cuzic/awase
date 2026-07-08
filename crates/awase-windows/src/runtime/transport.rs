use std::collections::HashSet;

use awase::types::{KeyEventType, RawKeyEvent, VkCode};

use crate::focus::class_names::AppImeProfile;
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

/// passthrough キーの Down/Up 対称性と output guard defer を管理するキュー。
///
/// `check_output_guard_defer` で defer した KeyDown の VK を `deferred_vks` に記録し、
/// 対応する KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ（WezTerm 対策）。
/// 各メソッドが `Some(event)` を返したとき、呼び出し元が `ReinjectKey(event)` をキューに
/// 積んで `Consumed` を返す責務を持つ。
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
    /// - Imm32Unavailable: shadow_toggle 発火時 KeyDown と全 KeyUp を Suppress
    /// - TsfNative: Allow（物理キーを通す）
    pub(crate) fn plan(
        event: &RawKeyEvent,
        profile: AppImeProfile,
        shadow_toggled: bool,
        is_tsf_mode: bool,
        f2_warmup_owned: bool,
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
            // Imm32Unavailable: shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress
            !profile.should_pass_physical_key()
                && (shadow_toggled || matches!(event.event_type, KeyEventType::KeyUp))
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

    fn f2_event(event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: crate::vk::VK_DBE_HIRAGANA,
            ..kanji_event(event_type, None)
        }
    }

    // ── F2 (VK_DBE_HIRAGANA): TSF mode 判定は KANJI/shadow_toggle と独立 ──

    #[test]
    fn f2_tsf_mode_suppresses_down_and_up() {
        let ev = f2_event(KeyEventType::KeyDown);
        assert_eq!(
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::TsfNative, false, true, true),
            PhysicalKeyDisposition::Suppress
        );
        let ev = f2_event(KeyEventType::KeyUp);
        assert_eq!(
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::TsfNative, false, true, true),
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
                PhysicalKeyDisposition::plan(&ev, AppImeProfile::TsfNative, false, true, false),
                PhysicalKeyDisposition::Allow,
                "MsImeStrategy は F2 warmup を送らないため物理 F2 ({event_type:?}) を素通しする"
            );
        }
    }

    #[test]
    fn f2_non_tsf_mode_allows() {
        let ev = f2_event(KeyEventType::KeyDown);
        assert_eq!(
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::Standard, false, false, false),
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
                    let ev = non_kanji_event(event_type);
                    assert_eq!(
                        PhysicalKeyDisposition::plan(&ev, profile, shadow_toggled, false, false),
                        PhysicalKeyDisposition::Allow,
                        "非KANJIイベントは profile={profile:?} shadow_toggled={shadow_toggled} \
                         event_type={event_type:?} でも常に Allow"
                    );
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
                    PhysicalKeyDisposition::plan(&ev, AppImeProfile::Standard, shadow_toggled, false, false),
                    PhysicalKeyDisposition::Suppress,
                    "ImmCross (Standard) は shadow_toggled={shadow_toggled} event_type={event_type:?} \
                     でも常に Suppress (spurious VK_F3/F4 連鎖の根本修正、08b8661)"
                );
            }
        }
    }

    // ── Imm32Unavailable: shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress ──

    #[test]
    fn imm32_unavailable_keydown_allowed_when_not_shadow_toggled() {
        let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
        assert_eq!(
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::Imm32Unavailable, false, false, false),
            PhysicalKeyDisposition::Allow,
            "shadow_toggle が発火していない KeyDown は物理キーを通す"
        );
    }

    #[test]
    fn imm32_unavailable_keydown_suppressed_when_shadow_toggled() {
        let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
        assert_eq!(
            PhysicalKeyDisposition::plan(&ev, AppImeProfile::Imm32Unavailable, true, false, false),
            PhysicalKeyDisposition::Suppress,
            "shadow_toggle 発火時の KeyDown は awase が既に VK_KANJI を SendInput 済みのため Suppress"
        );
    }

    #[test]
    fn imm32_unavailable_keyup_always_suppressed() {
        for shadow_toggled in [false, true] {
            let ev = kanji_event(KeyEventType::KeyUp, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan(&ev, AppImeProfile::Imm32Unavailable, shadow_toggled, false, false),
                PhysicalKeyDisposition::Suppress,
                "Imm32Unavailable の KANJI KeyUp は shadow_toggled={shadow_toggled} でも常に Suppress \
                 (二重制御による OS 側 spurious VK_F3/F4 の生成を防ぐ)"
            );
        }
    }

    // ── TsfNative: KANJI 関連キーは常に Allow (TSF が物理キーを処理する) ──

    #[test]
    fn tsf_native_always_allows_kanji_event() {
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            for shadow_toggled in [false, true] {
                let ev = kanji_event(event_type, Some(ShadowImeAction::TurnOn));
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        AppImeProfile::TsfNative,
                        shadow_toggled,
                        false,
                        false
                    ),
                    PhysicalKeyDisposition::Allow,
                    "TsfNative は shadow_toggled={shadow_toggled} event_type={event_type:?} でも \
                     常に Allow (TSF が物理キーを処理するため awase は介入しない)"
                );
            }
        }
    }
}
