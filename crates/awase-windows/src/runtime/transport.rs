use std::collections::HashSet;

use awase::types::{KeyEventType, RawKeyEvent, VkCode};

use crate::focus::class_names::AppImeProfile;
use crate::tsf::observer::ActiveImeKind;
use crate::vk::VkCodeExt as _;

pub(crate) use crate::state::physical_disposition::PhysicalKeyDisposition;

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
mod plan_shell_tests {
    use super::*;
    use crate::state::ime_kind::ImeKindId;
    use awase::types::{ImeRelevance, KeyClassification, ModifierState, ScanCode, ShadowImeAction};

    /// 判断の決定表のテストは `state/physical_disposition.rs` の `mod tests`（Linux でも走る、ADR-229 T1）にある。
    /// ここは殻 `plan`（= `plan_core(.., active_ime_kind.into())`）の配線だけを見る Windows 専用のテスト。

    fn event(
        vk_code: VkCode,
        event_type: KeyEventType,
        shadow_action: Option<ShadowImeAction>,
    ) -> RawKeyEvent {
        RawKeyEvent {
            was_down: false,
            press_id: None,
            vk_code,
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

    const PROFILES: [AppImeProfile; 4] = [
        AppImeProfile::Standard,
        AppImeProfile::Imm32Unavailable,
        AppImeProfile::TsfNative,
        AppImeProfile::InputRelay,
    ];
    const KINDS: [(ActiveImeKind, ImeKindId); 2] = [
        (ActiveImeKind::GoogleJapaneseInput, ImeKindId::Gji),
        (ActiveImeKind::MicrosoftIme, ImeKindId::MsIme),
    ];

    #[test]
    fn plan_shell_maps_active_ime_kind_to_ime_kind_id() {
        for (active, id) in KINDS {
            assert_eq!(ImeKindId::from(active), id);
        }
    }

    /// 殻 `plan` が、引数の順序・`shadow_toggled`・IME 種別を取り違えたり、`plan_core` の前に分岐を足したり
    /// していないこと: 結果が `shadow_toggled` や kind で変わる代表イベント(無変換/変換、半角/全角の
    /// `Some(Toggle)`、F2、F13 の役割キー、KeyDown/KeyUp)に対し、全プロファイル × 全 IME 種別 ×
    /// `shadow_toggled` の全組合せで `plan == plan_core(.., kind.into())` を確かめる。
    #[test]
    fn plan_equals_plan_core_with_mapped_kind_for_representative_events() {
        let toggle = Some(ShadowImeAction::Toggle);
        let mut events: Vec<(&str, RawKeyEvent)> = Vec::new();
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            events.push(("kanji", event(crate::vk::VK_KANJI, event_type, None)));
            events.push((
                "kanji+toggle",
                event(crate::vk::VK_KANJI, event_type, toggle),
            ));
            events.push(("convert", event(crate::vk::VK_CONVERT, event_type, None)));
            events.push((
                "nonconvert",
                event(crate::vk::VK_NONCONVERT, event_type, None),
            ));
            events.push((
                "sbcschar+toggle",
                event(crate::vk::VK_DBE_SBCSCHAR, event_type, toggle),
            ));
            events.push((
                "dbcschar+toggle",
                event(crate::vk::VK_DBE_DBCSCHAR, event_type, toggle),
            ));
            events.push((
                "hiragana(F2)",
                event(crate::vk::VK_DBE_HIRAGANA, event_type, None),
            ));
            events.push((
                "role-fkey(F13)+toggle",
                event(crate::vk::VK_F13, event_type, toggle),
            ));
        }
        for (label, ev) in &events {
            for profile in PROFILES {
                for (active, id) in KINDS {
                    for shadow_toggled in [false, true] {
                        assert_eq!(
                            PhysicalKeyDisposition::plan(ev, profile, shadow_toggled, active),
                            PhysicalKeyDisposition::plan_core(ev, profile, shadow_toggled, id),
                            "{label} {:?} profile={profile:?} kind={active:?} shadow_toggled={shadow_toggled}",
                            ev.event_type
                        );
                    }
                }
            }
        }
    }
}
