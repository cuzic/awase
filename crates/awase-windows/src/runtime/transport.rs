use std::collections::HashSet;

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
    ///   **例外: 半角/全角（0xF3 SBCSCHAR / 0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で
    ///   別処理）のうち、awase が beliefに基づく開閉トグルとして書くキー
    ///   （`ImeKeyKind::is_open_toggle_for`、ADR-189/191、GJI・MS-IME本体の両方）の
    ///   KeyDown は `shadow_toggled` に関わらず常に Suppress**（`ime_actuation_owned`
    ///   の場合）。NICOLA の物理「IME ON」キー（scan 0x70）は、IME が既に目的の状態に
    ///   ある時に押されると `VK_DBE_HIRAGANA` (0xF2) の代わりに `VK_DBE_*` を生成する
    ///   ことがあり、素通しすると awase が書く開閉に加えて実 IME が同じキーを能動的に
    ///   処理する二重 actuation になる（2026-08-05 実機、BUG-46/BUG-52）。
    ///   **ADR-191（撤去後）**: awase が書かない英数(0xF0)・カタカナ(0xF1)・ひらがな(0xF2)
    ///   などは Suppress せず OS（IME）へ素通しする（`shadow_action` を持たないので
    ///   `is_kanji_event` 判定で Allow）。BUG-116/ADR-137 の「Shift+0xF1 だけ Allow」の
    ///   特例は、0xF1 が常に Allow になったため撤去した。
    ///
    /// `ime_actuation_owned` を profile 単独ではなく `ActiveImeKind` からも導出するのは、
    /// TsfNative（Windows Terminal 等）で GJI が起動している場合に awase 自身の
    /// `SendInput(VK_IME_ON/OFF)`（`GjiDirectStrategy`）と、素通しされた元の物理 KANJI 系
    /// キーの reinject が **二重に actuate** してしまうため（BUG-46）。旧実装は
    /// `profile.should_pass_physical_key()`（TsfNative で常に true）のみで判定しており、
    /// 「TSF が KANJI を正しく処理する」という前提が `GjiDirectStrategy` の全プロファイル
    /// 適用化（`ime_controller.rs`）より前のまま残っていたことが原因だった。
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            ?profile,
            shadow_toggled = shadow_toggled,
            is_tsf_mode = is_tsf_mode,
            f2_warmup_owned = f2_warmup_owned,
            ?active_ime_kind
        )
    )]
    pub(crate) fn plan(
        event: &RawKeyEvent,
        profile: AppImeProfile,
        shadow_toggled: bool,
        is_tsf_mode: bool,
        f2_warmup_owned: bool,
        active_ime_kind: ActiveImeKind,
    ) -> Self {
        // InputRelay: この窓は入力面ではなく、awase は actuation を所有しない
        // （issue #136 / BUG-90 決定4）。**F2分岐より先に判定する**
        // （/code-review指摘で発見・修正）: F2分岐のSuppressは「awase自身の
        // warmup F2送信との衝突防止」が目的だが、`is_tsf_mode`/
        // `f2_warmup_owned`はグローバルな状態でありAppImeProfileとは独立に
        // 決まるため、理論上InputRelay windowにフォーカス中でも両方が
        // 真になりうる（MWB自身の中継ウィンドウがTSFネイティブ判定される
        // ことは通常無いが、将来別の中継ツールで起こりうる一般的な穴）。
        // InputRelay windowではawaseがそもそもこの窓向けのactuationを
        // 行わない（condition (a)）ため、F2分岐が守ろうとしている「awase
        // 自身のwarmup F2との衝突」という保護対象自体が存在しない。ここを
        // F2分岐より前に置くことで、この組み合わせでも物理かなキーが
        // Suppressされない（condition (b)）ことを構造的に保証する。
        if profile == AppImeProfile::InputRelay {
            return Self::Allow;
        }

        // F2 (VK_DBE_HIRAGANA): TSF mode かつ warmup 戦略が F2 を自前送信する場合のみ Suppress。
        //
        // **訂正（2026-09-06、BUG-116/ADR-137）**: このコメントは元々「awase 自身が
        // warmup として物理 F2 の代わりに SendInput(F2) を再送する契約」を前提に
        // 書かれていたが、ADR-100 決定2（2026-08-22）で eager warmup の送信キーは
        // `VK_DBE_HIRAGANA` から `VK_IME_ON` 単発（open 軸のみ）へ変更済み
        // （`output/mod.rs::send_eager_tsf_warmup`）。つまり物理 F2 の代替として
        // 実際に送られるのは open 軸のみで、charset 軸（カタカナ→ひらがな）を
        // 戻す効果は無い。この「埋め合わせの片肺化」が、GJI 環境で物理かなキー
        // 単独ではひらがなに戻せない副問題（ADR-137 M-6）の真因であり、
        // `key_pipeline.rs::kp_restore_hiragana_for_suppressed_mode_key`
        // （BUG-116 決定2）がこの埋め合わせを別経路で補っている。
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA {
            return if is_tsf_mode && f2_warmup_owned {
                Self::Suppress
            } else {
                Self::Allow
            };
        }

        // BUG-136 (issue #136): 他プロセスの SendInput (LLKHF_INJECTED) 由来のイベントは、
        // key_pipeline.rs::kp_stage_shadow_ime_toggle (BUG-14) が shadow_toggled への
        // 昇格を既に禁止しているため、awase 自身が actuate することはない。
        // 「解釈しない入力は消費しない」— awase が actuate しないのに物理キーだけ
        // Suppress すると、OS 側にも awase 側にも誰も IME を切り替えない
        // 「二重の空振り」になる（PowerToys Mouse Without Borders 等の正規リレー
        // ツールでリモート側の英数/かなキーが完全に無反応になる、ADR-119 参照）。
        //
        // この early return は下の ImmCross アーム（`profile.can_use_imm32_
        // cross_process()` → 無条件 Suppress）よりも先に来るため、ImmCross
        // アプリでも injected イベントは貫通する。ImmCross の無条件 Suppress は
        // 「spurious 連鎖の構造的遮断」（`feedback_immcross_owns_kanji`
        // の設計原則 — ImmCross アプリには物理 IME キーを見せない）という別種の
        // 保護だが、injected イベントは shadow_toggled を発火させないため awase
        // 自身が actuate することはなく、spurious 連鎖の前提（awase の自
        // actuation と物理キー通過の競合）がそもそも成立しない。したがって
        // ここを貫通させても `feedback_immcross_owns_kanji` が防ごうとした
        // リスクは再現しない（ADR-119 決定1参照）。
        if event.injected {
            debug_assert!(
                !shadow_toggled,
                "injected イベントで shadow_toggled が立つのは設計違反 \
                 (BUG-14 ガード kp_stage_shadow_ime_toggle が必ず false にする)"
            );
            return Self::Allow;
        }

        // 無変換/変換（ADR-141、C2対策）: shadow_action は belief 追随専用
        // （follow-only）であり、物理配送は既定で Allow する。C2対策で
        // これら2キーにも`shadow_action`（`enrich_ime_relevance`経由の
        // shadow_action override）が付くようになったため、対策なしだと
        // 下の`is_kanji_event`判定を抜けてKANJI関連VK同様にSuppressされ
        // うる——GJI自身がこの物理キーを見てIMEを切り替えることに
        // 依存している設計（BUG-115）なので、Suppressすると「OS側にも
        // awase側にも誰もIMEを切り替えない二重の空振り」（ADR-119と同型）
        // になる。VK_DBE_HIRAGANA等の静的KANJIキーと異なり、無変換/変換は
        // 既定では awase自身がactuationを所有する対象ではない（delegate/
        // shadow-toggleのどちらが処理する場合もbelief追随のみで、OS側の
        // 実際の切替はGJI自身が物理キー配送を通じて行う）ため、
        // `is_kanji_event`判定より前でこの分岐を置く。
        //
        // **例外（ADR-153決定1 M19）**: 明示config
        // （`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`）が
        // `kp_stage_shadow_ime_toggle`でこの打鍵に反応済み
        // （`event.ime_relevance.explicit_ime_action_consumed`）の場合のみ
        // Suppress する。このマーカーは2つの別経路から立つ:
        //
        // - **ケース2**（belief OFF→ON昇格）: 実際にIME open軸のactuationを
        //   発行済み。ただしこの経路の物理配送停止は`Decision::Consume`
        //   （NicolaFsmがこの打鍵をPendingThumbとして消費する）が別途
        //   担っており、`execute_relay`の`Decision::Consume`アームは
        //   `physical`を一切参照しないため、ケース2単独ではこの分岐の値は
        //   無害な冗長値になる。
        // - **ケース3改**（2026-09-08再設計、BUG-124対策、"off"×既にOFF）:
        //   `kp_stage_shadow_ime_toggle`はactuationを一切行わず
        //   マーカーだけを立てる——**この経路にとって、この分岐こそが
        //   唯一の実効的なSuppress手段**である（`Decision::Consume`には
        //   乗らない）。ここでSuppressしないと生の`VK_NONCONVERT`/
        //   `VK_CONVERT`がGJIへ届き、GJI自身のTSFキー横取り
        //   （`ITfKeyEventSink`）が「@」を誘発する（BUG-113の根本原因
        //   そのもの、実機A/B確認済み・BUG-124参照）。この分岐を
        //   「無害な冗長値」と誤認して削除すると、ケース3改が事実上の
        //   無防備になり「@」が再発する。
        if matches!(
            event.vk_code,
            crate::vk::VK_CONVERT | crate::vk::VK_NONCONVERT
        ) {
            return if event.ime_relevance.explicit_ime_action_consumed {
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
            // ImmCross: KANJI 関連 VK は原則 Down/Up 共に Suppress。
            // ただし 0xF2 HIRAGANA は上の専用分岐で先に Allow になる場合がある
            // （MS-IME 本体が物理 F2 で開く経路を残す、ADR-190）。
            true
        } else {
            // apply-ime が GjiDirect/MsImeDirect で実際に actuate する場合のみ、
            // shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress（BUG-46）。
            let kind: crate::state::ime_kind::ImeKindId = active_ime_kind.into();
            let ime_actuation_owned = key_sequence_policy::gji_direct_applicable(kind)
                || key_sequence_policy::ms_ime_direct_applicable(kind);
            // 半角/全角 (0xF3 SBCSCHAR / 0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で
            // 既に処理済みのためここには来ない) の KeyDown は、**awase が beliefに基づく
            // 開閉トグルとして書くキー**（`ImeKeyKind::is_open_toggle_for`、ADR-189/191。
            // GJI・MS-IME本体の両方）に限り、`shadow_toggled` に関わらず常に Suppress。
            // 素通しすると、awase が書く開閉に加えて実 IME が同じキーを能動的に処理する
            // 二重 actuation になる（BUG-46/BUG-52）。
            //
            // **ADR-191（撤去後）**: 英数(0xF0)・カタカナ(0xF1)は awase が書かない
            // （`shadow_action` を持たない）。実 IME に処理させて Engine は観測に追随する
            // ので、Suppress してはならない——握りつぶすと OS にも awase にも誰も何もしない
            // 「二重の空振り」になる。この2キーは上の `is_kanji_event` 判定で既に Allow だが、
            // 判定の根拠を「awase が書くキー」に揃えるため、ここでも VK を列挙せず
            // `is_open_toggle_for` で決める（BUG-116/ADR-137 の Shift+0xF1 の特例は、
            // 0xF1 が常に Allow になったため不要になり撤去した）。
            //
            // 設定 `dbe_mode_key_policy`（Passthrough で本条件を外す隠し設定）は撤去した
            // （ADR-191、レビュー指摘B-M3）: 0xF3/0xF4 は `enrich_ime_relevance` で必ず
            // `Toggle` の `shadow_action` を持ち `shadow_toggled` で Suppress されるため、
            // Passthrough を選んでも 0xF3/0xF4 は Suppress のままで、それ以外のキーには
            // そもそも効かない、実質死んだ設定だった。旧 config.toml にキーが残っていても
            // 未知キーとして無視され警告は出ない（`src/config.rs` のテストで固定）。
            let is_dbe_mode_key_down = event.event_type == KeyEventType::KeyDown
                && event
                    .vk_code
                    .ime_kind()
                    .is_some_and(|k| k.is_open_toggle_for(kind));
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
                ANY_IME_KIND
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
                ANY_IME_KIND
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
                    ANY_IME_KIND
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
                ANY_IME_KIND
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
                        false,
                        false,
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
                        false,
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
                        false,
                        false,
                        ActiveImeKind::GoogleJapaneseInput
                    ),
                    PhysicalKeyDisposition::Allow,
                    "無変換/変換(vk={vk:?}, event_type={event_type:?}) は shadow_toggled=true \
                     かつ ime_actuation_owned な状況でも常に Allow（follow-only、ADR-141）"
                );
            }
        }
    }

    // ── ADR-153 決定1 M19対策: 明示config（ケース3）が既にこの打鍵の
    //    IME open軸actuationを発行済み（explicit_ime_action_consumed）の
    //    場合のみ、上記follow-only原則の例外としてSuppressする ──

    #[test]
    fn henkan_muhenkan_suppressed_when_explicit_ime_action_already_consumed() {
        for vk in [crate::vk::VK_CONVERT, crate::vk::VK_NONCONVERT] {
            for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
                let mut ev = henkan_muhenkan_event(vk, None, event_type);
                ev.ime_relevance.explicit_ime_action_consumed = true;
                assert_eq!(
                    PhysicalKeyDisposition::plan(
                        &ev,
                        AppImeProfile::Standard,
                        false,
                        false,
                        false,
                        ActiveImeKind::MicrosoftIme
                    ),
                    PhysicalKeyDisposition::Suppress,
                    "無変換/変換(vk={vk:?}, event_type={event_type:?}) は明示config \
                     （ADR-153決定1ケース3）が既にactuate済みならSuppressする \
                     （抑止とactuationの1対1対応、B7/B8対策）"
                );
            }
        }
    }

    #[test]
    fn henkan_muhenkan_allowed_when_explicit_ime_action_not_consumed() {
        // マーカーが立っていない（既定値 false）通常時は、明示config未使用
        // ユーザーも含め従来どおり follow-only Allow のまま。
        for vk in [crate::vk::VK_CONVERT, crate::vk::VK_NONCONVERT] {
            let ev = henkan_muhenkan_event(vk, None, KeyEventType::KeyDown);
            assert!(!ev.ime_relevance.explicit_ime_action_consumed);
            assert_eq!(
                PhysicalKeyDisposition::plan(
                    &ev,
                    AppImeProfile::Standard,
                    false,
                    false,
                    false,
                    ActiveImeKind::MicrosoftIme
                ),
                PhysicalKeyDisposition::Allow,
                "vk={vk:?}: マーカー未設定時は既定のfollow-only Allowのまま"
            );
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
                PhysicalKeyDisposition::plan(&ev, profile, false, false, false, active_ime_kind),
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
                    PhysicalKeyDisposition::plan(
                        &ev,
                        profile,
                        false,
                        false,
                        false,
                        active_ime_kind
                    ),
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
                false,
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
                false,
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
                false,
                false,
                ActiveImeKind::GoogleJapaneseInput
            ),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn injected_f2_stays_suppressed_when_tsf_warmup_owns_f2() {
        let ev = injected(f2_event(KeyEventType::KeyDown));
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::TsfNative,
                false,
                true,
                true,
                ANY_IME_KIND
            ),
            PhysicalKeyDisposition::Suppress
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
                false,
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
                    false,
                    false,
                    ActiveImeKind::GoogleJapaneseInput
                ),
                PhysicalKeyDisposition::Allow,
                "{event_type:?}"
            );
        }
    }

    /// /code-review指摘: F2(VK_DBE_HIRAGANA)分岐は`is_tsf_mode`/`f2_warmup_owned`という
    /// AppImeProfileとは独立なグローバル状態で判定するため、理論上InputRelay window
    /// にフォーカス中でも両方が真になりうる。InputRelayの判定をF2分岐より前に置く
    /// ことで、この組み合わせでも物理かなキーがSuppressされない(condition (b))ことを
    /// 固定する（この2条件が偶然両方trueでもAllowになることが本テストの主眼）。
    #[test]
    fn input_relay_f2_is_allowed_even_when_tsf_warmup_flags_are_true() {
        let ev = f2_event(KeyEventType::KeyDown);
        assert_eq!(
            PhysicalKeyDisposition::plan(
                &ev,
                AppImeProfile::InputRelay,
                false,
                true, // is_tsf_mode
                true, // f2_warmup_owned
                ActiveImeKind::GoogleJapaneseInput
            ),
            PhysicalKeyDisposition::Allow,
            "InputRelay では is_tsf_mode/f2_warmup_owned が真でも F2 を Suppress してはならない"
        );
    }

    /// 対照実験: 同じ「shadow_toggle 不発」条件でも `VK_KANJI` 等の DBE 範囲外の
    /// 一般 KANJI キーは引き続き Allow のまま（`VK_DBE_*` 専用の例外であり、KANJI
    /// 系キー全体の挙動を変えていないことを固定する）。
    #[test]
    fn owned_actuation_keydown_allowed_when_not_shadow_toggled_is_unaffected_by_dbe_mode_fix() {
        for (profile, active_ime_kind, label) in owned_actuation_cases() {
            let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan(&ev, profile, false, false, false, active_ime_kind),
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
                        active_ime_kind
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
        );
        assert_eq!(disposition, PhysicalKeyDisposition::Suppress);
        assert_eq!(
            disposition.suppress_reason(&ev, AppImeProfile::TsfNative),
            Some("tsf-f2")
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
                    false,
                    false,
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
            false,
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
                                PhysicalKeyDisposition::plan(
                                    &ev,
                                    profile,
                                    false,
                                    false,
                                    false,
                                    active_ime_kind
                                ),
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
                        PhysicalKeyDisposition::plan(
                            &ev,
                            profile,
                            false,
                            false,
                            false,
                            active_ime_kind
                        ),
                        PhysicalKeyDisposition::Allow,
                        "{vk_label} / {label} / shift={shift}: \
                         awase が書かない英数/カタカナを握りつぶさない（ADR-191）"
                    );
                }
            }
        }
    }

    /// awase が書く半角/全角（0xF3/0xF4、GJI・MS-IME本体の両方）は、Shift の有無に依らず
    /// Suppress される（Shift は 0xF1 の特例の名残で、もう何の意味も持たない）。
    #[test]
    fn hankaku_zenkaku_keydown_suppressed_for_gji_and_msime_regardless_of_shift() {
        for (vk, action, vk_label) in dbe_written_vks() {
            for (profile, active_ime_kind, label) in owned_actuation_cases() {
                for shift in [false, true] {
                    let mut ev = dbe_mode_event(vk, action, KeyEventType::KeyDown);
                    if shift {
                        ev = with_shift(ev);
                    }
                    assert_eq!(
                        PhysicalKeyDisposition::plan(
                            &ev,
                            profile,
                            false,
                            false,
                            false,
                            active_ime_kind
                        ),
                        PhysicalKeyDisposition::Suppress,
                        "{vk_label} / {label} / shift={shift}: awase が beliefトグルとして書く \
                         キーは二重 actuation 防止のため Suppress（ADR-189/191）"
                    );
                }
            }
        }
    }

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
        is_tsf_mode: bool,
        f2_warmup_owned: bool,
        active_ime_kind: ActiveImeKind,
        injected: bool,
        explicit_ime_action_consumed: bool,
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
                for &is_tsf_mode in &ALL_BOOLS {
                    for &f2_warmup_owned in &ALL_BOOLS {
                        for &injected in &ALL_BOOLS {
                            let mut ev = f2_event(event_type);
                            ev.injected = injected;
                            let result = PhysicalKeyDisposition::plan(
                                &ev,
                                profile,
                                false,
                                is_tsf_mode,
                                f2_warmup_owned,
                                ActiveImeKind::GoogleJapaneseInput,
                            );
                            rows.push(PlanRow {
                                vk_label: "VK_DBE_HIRAGANA",
                                event_type,
                                profile,
                                shadow_toggled: false,
                                is_tsf_mode,
                                f2_warmup_owned,
                                active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
                                injected,
                                explicit_ime_action_consumed: false,
                                result,
                            });
                        }
                    }
                }
            }
        }

        // 2. DBEモードキー (0xF0/0xF1/0xF3/0xF4): awase が書くキー(0xF3/0xF4)と書かない
        //    キー(0xF0/0xF1)の違いは`plan()`の`is_open_toggle_for`だけで決まる
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
                                    false,
                                    false,
                                    active_ime_kind,
                                );
                                rows.push(PlanRow {
                                    vk_label: label,
                                    event_type,
                                    profile,
                                    shadow_toggled,
                                    is_tsf_mode: false,
                                    f2_warmup_owned: false,
                                    active_ime_kind,
                                    injected,
                                    explicit_ime_action_consumed: false,
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
                        for &explicit_consumed in &ALL_BOOLS {
                            let mut ev = henkan_muhenkan_event(vk, None, event_type);
                            ev.injected = injected;
                            ev.ime_relevance.explicit_ime_action_consumed = explicit_consumed;
                            let result = PhysicalKeyDisposition::plan(
                                &ev,
                                profile,
                                false,
                                false,
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
                                is_tsf_mode: false,
                                f2_warmup_owned: false,
                                active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
                                injected,
                                explicit_ime_action_consumed: explicit_consumed,
                                result,
                            });
                        }
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
                                false,
                                false,
                                active_ime_kind,
                            );
                            rows.push(PlanRow {
                                vk_label: "VK_KANJI(generic)",
                                event_type,
                                profile,
                                shadow_toggled,
                                is_tsf_mode: false,
                                f2_warmup_owned: false,
                                active_ime_kind,
                                injected,
                                explicit_ime_action_consumed: false,
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
                        false,
                        false,
                        ActiveImeKind::GoogleJapaneseInput,
                    );
                    rows.push(PlanRow {
                        vk_label: "non-kanji",
                        event_type,
                        profile,
                        shadow_toggled: false,
                        is_tsf_mode: false,
                        f2_warmup_owned: false,
                        active_ime_kind: ActiveImeKind::GoogleJapaneseInput,
                        injected,
                        explicit_ime_action_consumed: false,
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
}
