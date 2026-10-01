//! 物理 IME キーを OS に届けるか（Allow）握りつぶすか（Suppress）の配送判断の核。
//!
//! 元は `runtime/transport.rs`（`#[cfg(windows)]`）にあった `PhysicalKeyDisposition::plan` の本体を、
//! 挙動を変えずに ungated な本モジュールへ移したもの（ADR-208 L0）。`plan` は `RawKeyEvent`（コアクレートの型）・
//! `AppImeProfile`・`ImeKindId`・`shadow_toggled` だけを入力とする純粋関数で、`transport.rs` の `plan` は
//! `ActiveImeKind` → `ImeKindId` の変換だけを行う殻になった。これにより、`state/explicit_press.rs` の
//! `explicit_press_delivery`（全列挙テスト）が **本番と同じ判断コード** を Linux で呼べる。
//!
//! `transport.rs` にあった `plan_tests` は `#[cfg(windows)]` 配下のため Linux では存在しない（CLAUDE.md）。
//! 一致テストは `explicit_press.rs` 側（Linux で走る）が担う。

use awase::types::{KeyEventType, RawKeyEvent, ShadowImeAction};

use crate::focus::class_names::AppImeProfile;
use crate::state::ime_kind::ImeKindId;
use crate::state::key_sequence_policy;
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
    pub(crate) fn plan_core(
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
