//! 物理 IME キーを OS に届けるか（Allow）握りつぶすか（Suppress）の配送判断の核。
//!
//! 元は `runtime/transport.rs`（`#[cfg(windows)]`）にあった `PhysicalKeyDisposition::plan` の本体を、
//! 挙動を変えずに ungated な本モジュールへ移したもの（ADR-208 L0）。`plan` は `RawKeyEvent`（コアクレートの型）・
//! `AppImeProfile`・`ImeKindId`・`shadow_toggled` だけを入力とする純粋関数で、`transport.rs` の `plan` は
//! `ActiveImeKind` → `ImeKindId` の変換だけを行う殻になった。これにより、`state/explicit_press.rs` の
//! `explicit_press_delivery`（全列挙テスト）が **本番と同じ判断コード** を Linux で呼べる。
//!
//! `transport.rs` にあった `plan_tests`（30 本。配送判断の決定表）は、`plan_core` を直接呼ぶ形に書き換えて
//! 本モジュールの `mod tests` へ移した（ADR-229 T1）。`transport.rs` の `plan` は `ActiveImeKind` →
//! `ImeKindId` の殻なので、殻の配線は `transport.rs` 側の Windows 専用テストが 1 本で見る。
//! `explicit_press.rs` の全列挙テストとあわせて、どちらも Linux で走る。

use awase::types::{KeyEventType, RawKeyEvent, ShadowImeAction};

use crate::focus::class_names::AppImeProfile;
use crate::state::ime_kind::ImeKindId;
use crate::state::key_sequence_policy;
use crate::vk::VkCodeExt as _;

/// 元の物理キーイベントを OS に届けるかどうかの配送判断。
/// `Decision`（意味論）とは独立した配送機構上の判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalKeyDisposition {
    /// 元の物理キーイベントをそのまま OS に通す
    Allow,
    /// 元の物理キーイベントを消費（OS に届けない）
    Suppress,
}

impl PhysicalKeyDisposition {
    /// 無変換/変換（ADR-141・ADR-153 決定1 M19）と、役割由来の F13〜F24（ADR-199 決定18(iii)）の配送判断。
    /// どちらも `is_kanji_event` 判定（ImmCross の無条件 Suppress を含む）より前で決まる。該当しなければ `None`。
    /// `plan` の認知的複雑度（clippy 上限）のため関数に切り出した（分岐の中身は下の各コメントのとおり、
    /// 従来の無変換/変換の分岐をそのまま移したもの）。
    ///
    /// - **無変換/変換**: `shadow_action` は belief 追随専用で、物理配送は常に Allow を返す（GJI 自身がこの物理キーを見て
    ///   IME を切り替える設計、BUG-115。Suppress すると「OS 側にも awase 側にも誰も切り替えない」二重の空振りになる）。
    ///   awase が開閉を書く打鍵（開閉の役割があるとき、ADR-206）は、生キーを届けない責務をエンジンの `Decision::Consume`
    ///   （Phase 1 の特殊キー照合・FSM の PendingThumb と、その KeyUp の `UpDuty::Consume`）が負う。`execute_relay` の
    ///   Consume アームは `physical` を参照しないので、この分岐の値は Consume された打鍵には影響しない。
    ///   **VK 分岐そのものは削除しないこと**: 将来この2キーに `shadow_action` が付いたとき（C2 対策の経緯）、下の
    ///   `is_kanji_event` 判定に落ちて ImmCross で無条件に Suppress される（二重の空振り）のを、この分岐が Allow で防ぐ。
    /// - **F13〜F24**: 最初の Down は `shadow_toggled`（awase が実際に開閉を書いたか）で、リピートの Down と Up は
    ///   ラッチ由来の `shadow_action.is_some()` で Suppress する。書かなかった打鍵は Down/Up とも Allow（IME が
    ///   ユーザー設定どおり処理する）。ImmCross でも同じ（`shadow_action` があるだけで Suppress する従来規則だと、
    ///   書かない打鍵が二重の空振りになる）。
    fn thumb_or_role_fkey_disposition(event: &RawKeyEvent, shadow_toggled: bool) -> Option<Self> {
        let suppress = if matches!(
            event.vk_code,
            crate::vk::VK_CONVERT | crate::vk::VK_NONCONVERT
        ) {
            false
        } else if crate::vk::is_role_fkey(event.vk_code) {
            let first_down = event.event_type == KeyEventType::KeyDown && !event.was_down;
            // 役割由来の昇格（`shadow_action` あり）で書いたときだけ。同期キー（`keys.ime_detect`）由来の
            // `shadow_toggled` では書いたことにしない（`shadow_action` は付かない、Opus レビュー PR #328）。
            let role_action = event.ime_relevance.shadow_action.is_some();
            if first_down {
                shadow_toggled && role_action
            } else {
                role_action
            }
        } else {
            return None;
        };
        Some(if suppress {
            Self::Suppress
        } else {
            Self::Allow
        })
    }

    /// 物理キーを OS に届けるかどうかの純粋関数（核）。`runtime/transport.rs::PhysicalKeyDisposition::plan` はこれを呼ぶ薄い殻で、
    /// `ActiveImeKind` → `ImeKindId` の変換だけを行う。`explicit_press_delivery`（ADR-208 L0）も同じ核を共有する。
    ///
    /// **F2 (VK_DBE_HIRAGANA)**: 常に Allow（BUG-173）。以前は TSF mode かつ
    /// `f2_warmup_owned=true`（GJI 戦略）で Suppress していたが、ADR-100 決定2 で
    /// warmup が `VK_IME_ON` 単発になり「代わりに F2 を再送する」契約が崩れていた。
    /// 詳細は下の F2 分岐のコメント参照。
    ///
    /// **KANJI 関連キー**:
    /// - ImmCross プロファイル: Down/Up 共に Suppress（spurious 連鎖を構造的に遮断）
    /// - それ以外（Imm32Unavailable / TsfNative）: `apply-ime` が `GjiDirectStrategy` /
    ///   `MsImeDirectStrategy` で実際に actuate する場合（`ime_actuation_owned`）のみ、
    ///   shadow_toggle 発火時 KeyDown と全 KeyUp を Suppress。
    ///   **例外: 半角/全角（0xF3 SBCSCHAR / 0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で
    ///   別処理）のうち、awase が beliefに基づく開閉トグルとして書くキー
    ///   （`Runtime::enrich_key_role` が役割から `Some(Toggle)` を付けたもの、ADR-199 決定8。GJI は `config1.db` から逆算、MS-IME本体は仕様固定。ただし採用中の学習表が
    ///   半角/全角を開閉トグルでないと示すと`shadow_action`が付かず、この分岐の前に Down/Up とも Allow、ADR-195追記）の
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
    pub fn plan_core(
        event: &RawKeyEvent,
        profile: AppImeProfile,
        shadow_toggled: bool,
        kind: ImeKindId,
    ) -> Self {
        // InputRelay: この窓は入力面ではなく、awase は actuation を所有しない
        // （issue #136 / BUG-90 決定4）。物理 IME キーは常に Allow。
        if profile == AppImeProfile::InputRelay {
            return Self::Allow;
        }

        // F2 (VK_DBE_HIRAGANA): 常に Allow（BUG-173）。
        //
        // 旧実装は「TSF mode かつ GJI 戦略（`f2_warmup_owned`）なら Suppress」だった。この
        // Suppress は「awase 自身が warmup として物理 F2 の代わりに SendInput(F2) を再送する」
        // 契約（double-F2 防止）とセットの設計だったが、ADR-100 決定2（2026-08-22）で eager
        // warmup の送信キーが `VK_DBE_HIRAGANA` から `VK_IME_ON` 単発（open 軸のみ）へ変わった
        // 時点で契約が崩れていた。物理 F2 は消されるのに、代わりに届くのは open 軸だけで
        // charset 軸（カタカナ→ひらがな）は戻らない「食い逃げ」になり、IME belief が OFF の
        // ときは埋め合わせ（`kp_restore_hiragana_for_suppressed_mode_key`、`effective_open`
        // 必須）も見送られて物理ひらがなキーが完全に無反応になった（ADR-137 M-6、
        // BUG-173: GJI + Windows Terminal でカタカナから物理ひらがなキーで戻れない）。
        //
        // awase は物理 F2 の代わりに何も送らない（cold 化と GjiFsm 通知だけ、
        // `WindowsPlatform::composition_native_f2_down`）ので、物理 F2 を素通ししても二重 actuation に
        // ならない（conv は GJI 自身が物理キーとして処理する）。判定は VK だけで決まる。
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA {
            return Self::Allow;
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
        // 例外（旧 ADR-153 決定1 M19）は ADR-206 で撤去した: 生キーを届けない責務は、開閉を書く打鍵では
        // エンジンの `Decision::Consume` が負う（`thumb_or_role_fkey_disposition` の doc 参照）。
        if let Some(disposition) = Self::thumb_or_role_fkey_disposition(event, shadow_toggled) {
            return disposition;
        }

        let is_kanji_event = event.ime_relevance.shadow_action.is_some();
        if !is_kanji_event {
            return Self::Allow;
        }
        let suppress = if profile.can_use_imm32_cross_process() {
            // ImmCross: KANJI 関連 VK は原則 Down/Up 共に Suppress。
            // 0xF2 HIRAGANA は上の専用分岐で常に先に Allow になる（BUG-173。MS-IME 本体が物理 F2 で開く
            // 経路も残る、ADR-190）。
            true
        } else {
            // apply-ime が GjiDirect/MsImeDirect で実際に actuate する場合のみ、
            // shadow_toggle 発火時 KeyDown + 全 KeyUp を Suppress（BUG-46）。
            let ime_actuation_owned = key_sequence_policy::gji_direct_applicable(kind)
                || key_sequence_policy::ms_ime_direct_applicable(kind);
            // 半角/全角 (0xF3 SBCSCHAR / 0xF4 DBCSCHAR。0xF2 HIRAGANA は上の専用分岐で
            // 既に処理済みのためここには来ない) の KeyDown は、**awase が beliefに基づく
            // 開閉トグルとして書くキー**（`enrich_key_role` が役割から `Some(Toggle)` を付けた 0xF3/0xF4、
            // ADR-199 決定8）に限り、`shadow_toggled` に関わらず常に Suppress。
            // （採用中のGJI学習表が半角/全角を開閉トグルでないと示す場合は`shadow_action`が付かず、
            // 上の`is_kanji_event`判定でDown/UpともAllow済みでここに来ない。ADR-195追記）
            // 素通しすると、awase が書く開閉に加えて実 IME が同じキーを能動的に処理する
            // 二重 actuation になる（BUG-46/BUG-52）。
            //
            // **ADR-191（撤去後）**: 英数(0xF0)・カタカナ(0xF1)は awase が書かない
            // （`shadow_action` を持たない）。実 IME に処理させて Engine は観測に追随する
            // ので、Suppress してはならない——握りつぶすと OS にも awase にも誰も何もしない
            // 「二重の空振り」になる。この2キーは上の `is_kanji_event` 判定で既に Allow だが、
            // 判定の根拠を「awase が書くキー」に揃えるため、ここでも VK を列挙せず
            // 役割由来の `shadow_action`（`Some(Toggle)`）で決める（BUG-116/ADR-137 の Shift+0xF1 の特例は、
            // 0xF1 が常に Allow になったため不要になり撤去した）。
            //
            // 設定 `dbe_mode_key_policy`（Passthrough で本条件を外す隠し設定）は撤去した
            // （ADR-191、レビュー指摘B-M3）: 0xF3/0xF4 は `enrich_key_role` で（役割が無い・採用中の学習表が
            // 開閉トグルでないと示す場合を除き）`Toggle` の `shadow_action` を持ち
            // `shadow_toggled` で Suppress されるため、
            // Passthrough を選んでも 0xF3/0xF4 は Suppress のままで、それ以外のキーには
            // そもそも効かない、実質死んだ設定だった。旧 config.toml にキーが残っていても
            // 未知キーとして無視され警告は出ない（`src/config.rs` のテストで固定）。
            let is_dbe_mode_key_down = is_role_toggle_hz_key_down(event);
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

/// 役割由来の `Some(Toggle)` が付いた半角/全角(0xF3/0xF4)の KeyDown か（ADR-199 T4）。`plan` の
/// Suppress 判定の根拠（awase が開閉として書くキー）。認知的複雑度の上限（clippy）のため関数に切り出した。
fn is_role_toggle_hz_key_down(event: &RawKeyEvent) -> bool {
    event.event_type == KeyEventType::KeyDown
        && matches!(
            event.ime_relevance.shadow_action,
            Some(ShadowImeAction::Toggle)
        )
        && matches!(
            event.vk_code.ime_kind(),
            Some(crate::vk::ImeKeyKind::DbeSbcsChar | crate::vk::ImeKeyKind::DbeDbcsChar)
        )
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
    ///
    /// 本番の呼び出し元（`runtime/key_pipeline.rs`）は `#[cfg(windows)]` のため、非 Windows の本番ビルドでは
    /// 使い手が無い。`#[cfg(any(windows, test))]` で dead_code 警告を避ける（前例: `focus/thread_scope.rs`）。
    #[cfg(any(windows, test))]
    pub fn suppress_reason(
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

#[cfg(test)]
mod tests {
    use super::*;
    use awase::types::{
        ImeRelevance, KeyClassification, ModifierState, ScanCode, ShadowImeAction, VkCode,
    };

    fn kanji_event(
        event_type: KeyEventType,
        shadow_action: Option<ShadowImeAction>,
    ) -> RawKeyEvent {
        RawKeyEvent {
            was_down: false,
            press_id: None,
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
    const ANY_IME_KIND: ImeKindId = ImeKindId::Gji;

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
                    PhysicalKeyDisposition::plan_core(&ev, profile, false, ANY_IME_KIND),
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
                    for active_ime_kind in [ImeKindId::Gji, ImeKindId::MsIme] {
                        let ev = non_kanji_event(event_type);
                        assert_eq!(
                            PhysicalKeyDisposition::plan_core(
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
                    PhysicalKeyDisposition::plan_core(
                        &ev,
                        AppImeProfile::Standard,
                        shadow_toggled,
                        ImeKindId::MsIme
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
                    PhysicalKeyDisposition::plan_core(
                        &ev,
                        AppImeProfile::Standard,
                        false,
                        ImeKindId::MsIme
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
                    PhysicalKeyDisposition::plan_core(
                        &ev2,
                        AppImeProfile::TsfNative,
                        true,
                        ImeKindId::Gji
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
    // profile ではなく ImeKindId から導出することで、Imm32Unavailable と TsfNative を
    // 同じ suppress ロジックに統一する。

    /// `plan()` の `(profile, active_ime_kind)` の全組み合わせで suppress 挙動が
    /// Imm32Unavailable と TsfNative で一致することを固定する。
    fn owned_actuation_cases() -> Vec<(AppImeProfile, ImeKindId, &'static str)> {
        vec![
            (
                AppImeProfile::Imm32Unavailable,
                ImeKindId::MsIme,
                "Imm32Unavailable+MsIme (Chrome/Edge, 従来通り)",
            ),
            (
                AppImeProfile::Imm32Unavailable,
                ImeKindId::Gji,
                "Imm32Unavailable+GJI",
            ),
            (
                AppImeProfile::TsfNative,
                ImeKindId::Gji,
                "TsfNative+GJI (Windows Terminal, BUG-46 再現条件)",
            ),
            (
                AppImeProfile::TsfNative,
                ImeKindId::MsIme,
                "TsfNative+MsIme (WezTerm)",
            ),
        ]
    }

    #[test]
    fn owned_actuation_keydown_allowed_when_not_shadow_toggled() {
        for (profile, active_ime_kind, label) in owned_actuation_cases() {
            let ev = kanji_event(KeyEventType::KeyDown, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan_core(&ev, profile, false, active_ime_kind),
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
                PhysicalKeyDisposition::plan_core(
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
                    PhysicalKeyDisposition::plan_core(&ev, profile, false, active_ime_kind),
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
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ImeKindId::Gji),
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
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ImeKindId::Gji),
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
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ImeKindId::Gji),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn injected_f2_is_allowed() {
        let ev = injected(f2_event(KeyEventType::KeyDown));
        assert_eq!(
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ANY_IME_KIND),
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
            PhysicalKeyDisposition::plan_core(
                &ev,
                AppImeProfile::Standard,
                false,
                ImeKindId::MsIme
            ),
            PhysicalKeyDisposition::Allow
        );
    }

    #[test]
    fn input_relay_kanji_down_and_up_are_allowed() {
        for event_type in [KeyEventType::KeyDown, KeyEventType::KeyUp] {
            let ev = kanji_event(event_type, Some(ShadowImeAction::TurnOn));
            assert_eq!(
                PhysicalKeyDisposition::plan_core(
                    &ev,
                    AppImeProfile::InputRelay,
                    true,
                    ImeKindId::Gji
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
                PhysicalKeyDisposition::plan_core(&ev, profile, false, active_ime_kind),
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
                    PhysicalKeyDisposition::plan_core(
                        &ev,
                        profile,
                        shadow_toggled,
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
    // VK_DBE_HIRAGANA (かな) は専用分岐で Allow される（当時は TSF mode 以外のみ、BUG-173 で常に）ことが
    // 判明した（「かなは効くが英数は効かない」という報告症状と一致）。
    // この非対称性を journal から確認できるようにする `suppress_reason` を
    // ここで固定する。

    #[test]
    fn suppress_reason_is_none_when_allowed() {
        let ev = f2_event(KeyEventType::KeyDown);
        let disposition =
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ANY_IME_KIND);
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
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ANY_IME_KIND);
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
                let disposition = PhysicalKeyDisposition::plan_core(
                    &ev,
                    AppImeProfile::Standard,
                    shadow_toggled,
                    ImeKindId::Gji,
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
        let disposition =
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::TsfNative, false, ImeKindId::Gji);
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
                                PhysicalKeyDisposition::plan_core(
                                    &ev,
                                    profile,
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
                        PhysicalKeyDisposition::plan_core(&ev, profile, false, active_ime_kind),
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
                        PhysicalKeyDisposition::plan_core(&ev, profile, false, active_ime_kind),
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
                        PhysicalKeyDisposition::plan_core(
                            &ev,
                            profile,
                            false,
                            ImeKindId::Gji
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
        active_ime_kind: ImeKindId,
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
    const ALL_IME_KINDS: [ImeKindId; 2] = [ImeKindId::Gji, ImeKindId::MsIme];
    const ALL_BOOLS: [bool; 2] = [false, true];

    /// `kanji_family_keyup_suppress_verdict_is_independent_of_specific_vk`が
    /// 行のグルーピングに使う、vk種別を除いた入力キー。
    type PlanKey = (AppImeProfile, bool, ImeKindId, bool);

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
                    let result =
                        PhysicalKeyDisposition::plan_core(&ev, profile, false, ImeKindId::Gji);
                    rows.push(PlanRow {
                        vk_label: "VK_DBE_HIRAGANA",
                        event_type,
                        profile,
                        shadow_toggled: false,
                        active_ime_kind: ImeKindId::Gji,
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
                                let result = PhysicalKeyDisposition::plan_core(
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
                        let result =
                            PhysicalKeyDisposition::plan_core(&ev, profile, false, ImeKindId::Gji);
                        rows.push(PlanRow {
                            vk_label: if vk == crate::vk::VK_CONVERT {
                                "VK_CONVERT"
                            } else {
                                "VK_NONCONVERT"
                            },
                            event_type,
                            profile,
                            shadow_toggled: false,
                            active_ime_kind: ImeKindId::Gji,
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
                            let result = PhysicalKeyDisposition::plan_core(
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
                    let result =
                        PhysicalKeyDisposition::plan_core(&ev, profile, false, ImeKindId::Gji);
                    rows.push(PlanRow {
                        vk_label: "non-kanji",
                        event_type,
                        profile,
                        shadow_toggled: false,
                        active_ime_kind: ImeKindId::Gji,
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
            for kind in [ImeKindId::Gji, ImeKindId::MsIme] {
                let d = PhysicalKeyDisposition::plan_core(ev, profile, shadow_toggled, kind);
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
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::Standard, false, ImeKindId::Gji),
            PhysicalKeyDisposition::Allow
        );
        let ev = fkey_event(KeyEventType::KeyDown, false, Some(ShadowImeAction::Toggle));
        let d =
            PhysicalKeyDisposition::plan_core(&ev, AppImeProfile::Standard, true, ImeKindId::Gji);
        assert_eq!(
            d.suppress_reason(&ev, AppImeProfile::Standard),
            Some("role-fkey")
        );
    }
}
