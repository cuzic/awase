//! `GjiFsm` 同期義務の宣言と履行（ADR-089 §2.4、INV-42/43）。
//!
//! # 経緯 — 宣言軸を profile → outcome へ移した
//!
//! 本モジュールは ADR-081 Phase 1c で「共有 GJI 直接制御機構」として起こされ、
//! `ImeProfileDriver::uses_gji_direct()`（**profile 軸・静的**）を宣言した
//! ドライバにだけ `GjiDirectAccess` token を発行する形で同期義務をゲートして
//! いた。ADR-081 Phase 1d 検討（2026-08-02）が、その前提が誤りであること——
//! **実際の同期条件は outcome 軸（`outcome != UnsafeToToggle`）だけで決まる**
//! ——を発見し、[`legacy_gji_sync_obligation`] が非対称の証拠として残された。
//!
//! ADR-089 §2.4（INV-42）はこの発見を採用し、同期義務を outcome 軸に一本化した。
//! **その結果 `uses_gji_direct()` と `GjiDirectAccess` token は根拠を失ったため、
//! Phase B（ADR-089 §6 item 8、§4.7）で撤去した**（2026-08-12）。
//! ADR-081 Phase 1c の contract test 不変条件 4・5 は、それぞれ
//! [`ActuationReceipt`]（INV-43）と [`legacy_gji_sync_obligation`]（INV-42）が
//! 引き取っている。
//!
//! # 型で表現している不変条件
//!
//! - **INV-42（同期義務は outcome 軸のみで決まる）**: 導出式は
//!   [`legacy_gji_sync_obligation`] ただ 1 つ。[`ActuationReceipt::settle`] は
//!   式を二重に書かず、この関数を呼ぶ。profile 軸でも K 軸（`ImeKindId`）でも
//!   ゲートしない（ADR-089 §4.3。**推測値で閉じると LINE × GJI で同期が落ちる**）。
//! - **INV-43（receipt は settle されずに drop されない）**:
//!   [`ActuationReceipt`] は `#[must_use]` + `Drop` の `debug_assert`。
//!   **保証水準は「debug ビルドでの実行時検出」までである**（release では
//!   `debug_assert` が消え、`let r = ..` では `#[must_use]` が発火しない。
//!   ADR-089 §8.1）。**これを根拠に `platform.rs` の legacy 同期を撤去しては
//!   ならない**——ADR-081 Phase 1e が踏みかけた BUG-18/22 型の再発条件である。
//!
//! # Linux でテスト可能にするための制約（ADR-065）
//!
//! `GjiFsm`（`tsf/` 配下、`#[cfg(windows)]`）には依存しない。同期義務は
//! [`GjiFsmSync`]（ungated な列挙値）で象徴的に表し、実 `GjiFsm` への写像は
//! [`GjiSyncSink`] の Windows 実装（`platform.rs`）が担う。**送信 VK の解決も
//! ここでは行わない**: 具体 VK は `state/key_sequence_policy.rs::ime_key_for`
//! が握る（SSOT を二重化すると IME OFF キー反転実験
//! （`.claude/rules/experiment-logging.md`）の drift 源になる）。

use awase::platform::ImeOpenOutcome;
use awase::types::KeyAction;

/// GJI 機構経由の IME 状態遷移が課す `GjiFsm` 同期義務のマーカー。
///
/// 現行 `platform.rs` の `gji_on_ime_on` / `gji_on_ime_off`（`GjiFsm` を belief と
/// 同期させるハンドラ）に対応する。[`GjiSyncSink`] の実装がこの値を実 `GjiFsm`
/// 呼び出しへ写像する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GjiFsmSync {
    /// IME を開いた（`gji_on_ime_on` 相当の同期が必要）。
    OnImeOn,
    /// IME を閉じた（`gji_on_ime_off` 相当の同期が必要）。
    OnImeOff,
    /// ADR-203 (i) level 突合: エンジンがローマ字を IME へ送ろうとしているのに `GjiFsm` が
    /// `OffCold` のとき、`OnImeOn` と同じ遷移を **belief 起点**（awase は IME へ書いていない）で行う。
    OnImeOnBelief,
    /// ADR-203 (ii): 確かな ON 系イベント（物理キー予測 ON・shadow toggle ON〈`sync_direction` の on キーを含む〉）
    /// で `GjiFsm` を開き直す（`GjiEvent::Reopen`。遷移表は `tsf/gji_fsm.rs` の `GjiEvent::Reopen` の doc）。
    /// 発生元は journal の trigger に残す（[`ReopenSource`]）。
    Reopen(ReopenSource),
}

/// [`GjiFsmSync::Reopen`] の発生元（journal の `GjiFsmTransition.trigger` に残し、e2e・bug report から
/// どの入口で開き直したかを区別できるようにする。ADR-203 決定9）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenSource {
    /// 物理キー予測（`KeyEffectPredicted{open: Some(true)}`）。
    Predict,
    /// shadow toggle の no-op 分岐（belief が既に ON の `TurnOn` キー。OFF を見逃した後の ON）。
    ShadowNoop,
    /// shadow toggle で OFF→ON に倒した瞬間。
    ShadowToggle,
}

impl ReopenSource {
    /// journal の trigger 文字列。
    #[must_use]
    pub const fn trigger(self) -> &'static str {
        match self {
            Self::Predict => "Reopen(BeliefSync:predict)",
            Self::ShadowNoop => "Reopen(BeliefSync:shadow-noop)",
            Self::ShadowToggle => "Reopen(BeliefSync:shadow-toggle)",
        }
    }
}

/// 同期の起点（ADR-203 決定3）。`BeliefSync` は awase が IME へ書いていない同期であり、
/// long-cold の reinit（VK_IME_OFF→VK_IME_ON の awase 起点書き込み）を行ってはならない
/// （ADR-191「awase は書かない」、ADR-090 A-2 の warrant を迂回しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GjiSyncOrigin {
    /// awase 自身の actuation の結果としての同期（従来の `OnImeOn`/`OnImeOff`）。
    Actuation,
    /// belief・観測・予測からの同期。
    BeliefSync,
}

impl GjiFsmSync {
    /// この同期の起点。
    #[must_use]
    pub const fn origin(self) -> GjiSyncOrigin {
        match self {
            Self::OnImeOn | Self::OnImeOff => GjiSyncOrigin::Actuation,
            Self::OnImeOnBelief | Self::Reopen(_) => GjiSyncOrigin::BeliefSync,
        }
    }

    /// モジュール private。外から `GjiFsmSync::OnImeOn/OnImeOff` を作る唯一の経路は
    /// [`legacy_gji_sync_obligation`] であり、導出式が 1 箇所であることを
    /// 可視性で担保する（INV-42）。
    #[must_use]
    const fn for_open(open: bool) -> Self {
        if open {
            Self::OnImeOn
        } else {
            Self::OnImeOff
        }
    }
}

/// `GjiFsm` 同期の実行口（ADR-089 §2.4）。
///
/// **`&mut GjiFsm` では受けられない。** `GjiFsm::on_sync` は存在せず、`GjiFsm` 本体は
/// `output.warmup_coord.tsf_warmup`（`RefCell`）の中にあり、1 回の同期は
/// `output.gji_on_event(..)` が返す `Response<GjiAction, GjiTimer>` を
/// `dispatch_gji_response` へ流すところまでを含む。つまり実装側は
/// `&mut WindowsPlatform` 相当を必要とするため、ungated 側は trait で受ける
/// （ADR-089 §1.3(f)、INV-42）。
pub trait GjiSyncSink {
    /// 同期義務 1 件を履行する。
    fn sync_gji(&mut self, sync: GjiFsmSync);
}

/// actuation 1 回分の「`GjiFsm` を同期する義務」を運ぶ値（ADR-089 §2.4、INV-43）。
///
/// # 使い方
///
/// actuation を起動した呼び出しフレームのローカル値として持ち、**同じフレームで**
/// [`settle`](Self::settle) する。**`WindowsPlatform` のフィールドに持たせない**
/// ——`receipt.settle(&mut platform)` は receipt と platform の 2 つの可変借用を
/// 同時に取るため、platform 内に格納すると借用検査に落ちる（ADR-089 §2.4 細目3）。
///
/// # `settle(self)` の consume 形を採らない理由（ADR-089 §4.4）
///
/// `Drop` を実装した型はフィールドを move できず、`self` を consume する
/// メソッドでは `ManuallyDrop` / `mem::forget` が要る。`settled: bool` +
/// `Drop` での `debug_assert` のほうが単純で、目的（settle 忘れの検出）を
/// 同等に達成する。**「`settle(self)` のほうが綺麗だ」と書き換えると `Drop` と
/// 衝突する。**
///
/// # compile-fail ケース（ADR-089 §7 ケース4）
///
/// 束縛せずに捨てた receipt は `#![deny(unused_must_use)]` 下でエラーになる。
/// **固定できるのはこの「未束縛」の形だけである**——`let r = ..;` は
/// `#[must_use]` を発火させないため compile-fail にできない（ADR-089 §8.1）。
///
/// 通る双子（束縛して settle する）:
///
/// ```
/// #![deny(unused_must_use)]
/// use awase::platform::ImeOpenOutcome;
/// use awase_windows_core::state::gji_direct_mechanism::{
///     ActuationReceipt, GjiFsmSync, GjiSyncSink,
/// };
///
/// struct Sink;
/// impl GjiSyncSink for Sink {
///     fn sync_gji(&mut self, _sync: GjiFsmSync) {}
/// }
///
/// let mut receipt = ActuationReceipt::new(true, ImeOpenOutcome::Applied);
/// receipt.settle(&mut Sink);
/// assert!(receipt.is_settled());
/// ```
///
/// 未束縛で捨てるとコンパイルが通らない（最後の 2 行を 1 行にしただけ）:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use awase::platform::ImeOpenOutcome;
/// use awase_windows_core::state::gji_direct_mechanism::ActuationReceipt;
///
/// // error: unused return value of `ActuationReceipt::new` that must be used
/// ActuationReceipt::new(true, ImeOpenOutcome::Applied);
/// ```
#[must_use = "ActuationReceipt は settle() して GjiFsm を同期する義務を運ぶ（ADR-089 INV-43）"]
#[derive(Debug)]
pub struct ActuationReceipt {
    outcome: ImeOpenOutcome,
    want: bool,
    settled: bool,
}

impl ActuationReceipt {
    /// actuation の帰結から receipt を作る。
    ///
    /// `want` は「その actuation が目指した open 値」、`outcome` は実際の帰結。
    pub const fn new(want: bool, outcome: ImeOpenOutcome) -> Self {
        Self {
            outcome,
            want,
            settled: false,
        }
    }

    /// 同期義務を履行する。
    ///
    /// 導出は [`legacy_gji_sync_obligation`] に委ねる（式を二重に書かない、INV-42）。
    /// `outcome == UnsafeToToggle` のときは sink を呼ばずに settle 済みにする
    /// （送信していないため同期する事実が無い）。
    pub fn settle<S: GjiSyncSink + ?Sized>(&mut self, sink: &mut S) {
        if let Some(sync) = legacy_gji_sync_obligation(self.want, self.outcome) {
            sink.sync_gji(sync);
        }
        self.settled = true;
    }

    /// 既に settle 済みか（テスト・診断用）。
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        self.settled
    }

    /// この receipt が運ぶ帰結。
    #[must_use]
    pub const fn outcome(&self) -> ImeOpenOutcome {
        self.outcome
    }
}

impl Drop for ActuationReceipt {
    fn drop(&mut self) {
        // ADR-089 §9-1 の決定（Phase B 実装時、2026-08-12）:
        // actuation 中に panic すると receipt は settle されないまま drop される。
        // unwind 中に `debug_assert!` が panic すると double panic → abort になり、
        // **本来の panic の原因が失われる**（panic_detect.rs のクラッシュ報告も
        // 元の payload を拾えなくなる）。`std::thread::panicking()` で unwind 中を
        // 除外し、通常フローの settle 忘れだけを検出する。
        if std::thread::panicking() {
            return;
        }
        debug_assert!(
            self.settled,
            "ActuationReceipt が settle されずに drop された（ADR-089 INV-43）: \
             want={} outcome={:?}",
            self.want, self.outcome
        );
    }
}

/// 現行（legacy）経路が実際に課す `GjiFsm` 同期義務を [`GjiFsmSync`] へ写像した
/// 純粋関数。**同期義務の導出式はここ 1 箇所である**（INV-42）。
///
/// `WindowsPlatform::on_ime_applied`（`platform.rs`）の実装をそのまま反映する:
/// `outcome == UnsafeToToggle` / `NotOwned` の場合のみ同期しない（送信していないため）。**それ以外は
/// `open` の値だけを見て無条件に同期する** — どの戦略（ImmCross / GjiDirect /
/// MsImeDirect）で actuate したか、ひいてはどの `ImeProfileDriver` を
/// 経由したかは一切問わない。
///
/// # profile 軸 / K 軸でゲートしてはならない（ADR-089 §4.3、INV-42）
///
/// ADR-081 Phase 1d が profile 軸（`uses_gji_direct()`）で、ADR-089 の設計 r2 が
/// K 軸（`ImeKindId`）で、**同じ失敗を 2 回している**。
///
/// - profile 軸: `ImmCrossDriver`（LINE/Qt 等）は `uses_gji_direct() == false` を
///   宣言するため機構経由では `GjiFsmSync` を得られないが、**LINE × Google
///   日本語入力は実在する組み合わせ**であり legacy は今もそこで同期している。
/// - K 軸: `ImeKindId::MsIme` は「MS-IME を観測した」ではなく「**GJI を検出できな
///   かった**」である（`tsf/observer.rs:498-502`）。GJI 起動直後・フォーカス直後の
///   未検出ウィンドウでは GJI 環境でも `MsIme` になり、同期が落ちる。
///
/// どちらも「belief を actuate 抜きで ON にする高速パスが `GjiFsm` 同期を踏み抜く」
/// BUG-18/22 型の再発条件そのものである。**無条件同期は無害**（`GjiEvent::ImeOn` は
/// `GjiFsm` 側で自己ゲートし、MS-IME 環境でもコスト・副作用ゼロ）であり、
/// 推測値でゲートして落とすリスクのほうが一方的に大きい（原則 P20）。
#[must_use]
pub fn legacy_gji_sync_obligation(open: bool, outcome: ImeOpenOutcome) -> Option<GjiFsmSync> {
    if matches!(
        outcome,
        ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned
    ) {
        return None;
    }
    Some(GjiFsmSync::for_open(open))
}

/// ADR-203 (i) level 突合: `send_keys` の直前に `GjiFsm` を `OnImeOnBelief` で同期すべきか。
///
/// エンジンがローマ字を IME 経由で送るのは belief が ON のときだけなので、`GjiFsm` が `OffCold` の
/// ままなのに送ろうとしている不一致はそれ自体が同期漏れの証拠になる（時刻反転・起動時既定値・観測の
/// 揺れの影響を受けない、入口も問わない）。**種別（K 軸）ではなく戦略の実体（`needs_f2_probe`）で
/// ゲートする**（INV-42）。
///
/// 対象外: Unicode 注入モード（`GjiFsm` に `KeyInput` を送らず composition も迂回するため per-VK/ESC
/// の害が無い）、probe・raw recovery/reinit の実行中（probe_id の相関が崩れる。次の送信で拾う）。
#[must_use]
pub const fn needs_belief_sync_on(
    send_has_romaji: bool,
    injection_is_unicode: bool,
    strategy_is_gji_fsm: bool,
    gji_is_off_cold: bool,
    probe_or_recovery_blocking: bool,
) -> bool {
    send_has_romaji
        && !injection_is_unicode
        && strategy_is_gji_fsm
        && gji_is_off_cold
        && !probe_or_recovery_blocking
}

/// `send_keys` の `actions` が、IME 経由のローマ字/文字の送信を含むか。
///
/// 対象は cold-start 保護（per-VK confirm）経路を通るもの。`Char`/`Romaji` に加え、`KeySequence`（`.yab` の全角記号 `，` `－` 等。VK モードでは
/// `send_char` を1文字ずつ呼び `Char` と同じ経路に進む）と、`Sequence` の中身（再帰）を見る。
/// ADR-203 決定1「Sequence 内含む」（PR #354 のコードレビュー M1）。`Key`/`KeyUp`/`CtrlChord` は含めない。
#[must_use]
pub fn send_carries_romaji(actions: &[KeyAction]) -> bool {
    actions.iter().any(|a| match a {
        KeyAction::Char(_) | KeyAction::Romaji(_) | KeyAction::KeySequence(_) => true,
        KeyAction::Sequence(items) => send_carries_romaji(items),
        _ => false,
    })
}

/// ADR-203 (ii): 確かな ON 系イベントで `GjiFsm` を開き直す同期義務。
///
/// 候補窓が可視（=入力中）なら出さない。`GjiFsm` の `OnWarm` は `EndComposition` の取りこぼしで
/// 候補窓が出ていても `OnWarm` に見えうるための二重防御（`GjiFsm` 側も `OnComposing`/`OnCold` では
/// 何もしない）。入力の途中で cold に落とすと per-VK confirm → StaleConfirm → ESC で未確定文字が
/// 消える（BUG-171、BUG-033 追補3・4 と同型）。
///
/// **Unicode 注入モードでも出す**（PR #354 のコードレビュー M2 で一度「Unicode では出さない」にしたが、
/// e2e-ime-smoke の baseline/atok-passthrough-cold（Unicode モードで動く）で GjiFsm が OffCold に固着して
/// I4 が FAIL したため取り下げた）。Reopen 後の long-idle で `needs_unicode_cold_warmup` が VK_IME_ON poke
/// を出すのは Unicode long-cold の既存設計（同期が正常に届く Unicode ユーザーでは元から起きる挙動）で、
/// OffCold に固着すると失われるのは「long-cold の defer」であり、それを取り戻すのが同期の目的である。
#[must_use]
pub const fn reopen_obligation(
    candidate_visible: bool,
    source: ReopenSource,
) -> Option<GjiFsmSync> {
    if candidate_visible {
        None
    } else {
        Some(GjiFsmSync::Reopen(source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_OUTCOMES: [ImeOpenOutcome; 5] = [
        ImeOpenOutcome::Applied,
        ImeOpenOutcome::AlreadyMatched,
        ImeOpenOutcome::Failed,
        ImeOpenOutcome::UnsafeToToggle,
        ImeOpenOutcome::NotOwned,
    ];

    /// 同期呼び出しを記録するフェイク sink。
    #[derive(Default)]
    struct RecordingSink {
        calls: Vec<GjiFsmSync>,
    }

    impl GjiSyncSink for RecordingSink {
        fn sync_gji(&mut self, sync: GjiFsmSync) {
            self.calls.push(sync);
        }
    }

    #[test]
    fn legacy_obligation_is_none_only_for_unsafe_to_toggle() {
        assert_eq!(
            legacy_gji_sync_obligation(true, ImeOpenOutcome::UnsafeToToggle),
            None
        );
        assert_eq!(
            legacy_gji_sync_obligation(false, ImeOpenOutcome::UnsafeToToggle),
            None
        );
        assert_eq!(
            legacy_gji_sync_obligation(true, ImeOpenOutcome::NotOwned),
            None
        );
        assert_eq!(
            legacy_gji_sync_obligation(false, ImeOpenOutcome::NotOwned),
            None
        );
        for outcome in [
            ImeOpenOutcome::Applied,
            ImeOpenOutcome::AlreadyMatched,
            ImeOpenOutcome::Failed,
        ] {
            assert_eq!(
                legacy_gji_sync_obligation(true, outcome),
                Some(GjiFsmSync::OnImeOn)
            );
            assert_eq!(
                legacy_gji_sync_obligation(false, outcome),
                Some(GjiFsmSync::OnImeOff)
            );
        }
    }

    /// **INV-42 の全数固定**: `settle` の同期判定は全 `ImeOpenOutcome` × `open` で
    /// `legacy_gji_sync_obligation` と一致する（ADR-089 §7「新設するもの — 全数テスト」）。
    #[test]
    fn settle_matches_legacy_obligation_for_every_outcome_and_open() {
        for outcome in ALL_OUTCOMES {
            for want in [true, false] {
                let mut sink = RecordingSink::default();
                let mut receipt = ActuationReceipt::new(want, outcome);
                receipt.settle(&mut sink);
                let expected: Vec<GjiFsmSync> = legacy_gji_sync_obligation(want, outcome)
                    .into_iter()
                    .collect();
                assert_eq!(sink.calls, expected, "outcome={outcome:?} want={want}");
                assert!(receipt.is_settled());
            }
        }
    }

    /// `UnsafeToToggle` でも settle 済みになる（＝ drop 時に debug_assert が
    /// 発火しない）。送信していないので sink は呼ばれない。
    #[test]
    fn unsafe_to_toggle_settles_without_calling_sink() {
        let mut sink = RecordingSink::default();
        let mut receipt = ActuationReceipt::new(true, ImeOpenOutcome::UnsafeToToggle);
        receipt.settle(&mut sink);
        assert!(receipt.is_settled());
        assert!(sink.calls.is_empty());
    }

    /// receipt は同期義務以外の情報（どの戦略で actuate したか）を要求しない
    /// ——outcome 軸だけで決まるという INV-42 を、型の形として固定する。
    #[test]
    fn receipt_carries_only_outcome_and_want() {
        let receipt = ActuationReceipt::new(false, ImeOpenOutcome::Applied);
        assert_eq!(receipt.outcome(), ImeOpenOutcome::Applied);
        assert!(!receipt.is_settled());
        // settle しないまま drop すると debug ビルドでは debug_assert が発火する。
        // ここでは検出そのものを確認せず（テストを落とさないため）settle して捨てる。
        let mut receipt = receipt;
        let mut sink = RecordingSink::default();
        receipt.settle(&mut sink);
    }

    #[test]
    fn belief_sync_on_decision_table_is_exhaustive() {
        // 全 2^5 組: 真になるのは「ローマ字あり・非Unicode・GjiFsm戦略・OffCold・非blocking」の1通りだけ。
        for bits in 0u8..32 {
            let b = |i: u8| bits & (1 << i) != 0;
            let (romaji, unicode, f2, off, blocking) = (b(0), b(1), b(2), b(3), b(4));
            let want = romaji && !unicode && f2 && off && !blocking;
            assert_eq!(
                needs_belief_sync_on(romaji, unicode, f2, off, blocking),
                want,
                "bits={bits:05b}"
            );
        }
    }

    #[test]
    fn reopen_is_suppressed_only_while_candidate_visible() {
        use ReopenSource::*;
        for src in [Predict, ShadowNoop, ShadowToggle] {
            assert_eq!(reopen_obligation(false, src), Some(GjiFsmSync::Reopen(src)));
            assert_eq!(reopen_obligation(true, src), None);
        }
    }

    #[test]
    fn reopen_triggers_distinguish_every_entry() {
        use ReopenSource::*;
        let t = [
            Predict.trigger(),
            ShadowNoop.trigger(),
            ShadowToggle.trigger(),
        ];
        assert!(t.iter().all(|s| s.starts_with("Reopen(BeliefSync:")));
        assert_eq!(t.iter().collect::<std::collections::HashSet<_>>().len(), 3);
    }

    #[test]
    fn send_carries_romaji_sees_key_sequence_and_nested_sequence() {
        use awase::types::VkCode;
        assert!(send_carries_romaji(&[KeyAction::Char('あ')]));
        assert!(send_carries_romaji(&[KeyAction::Romaji("ka".into())]));
        // 全角記号（`.yab` のクォート無し記号）は KeySequence（M1）
        assert!(send_carries_romaji(&[KeyAction::KeySequence("，".into())]));
        assert!(send_carries_romaji(&[KeyAction::Sequence(vec![
            KeyAction::Suppress,
            KeyAction::Sequence(vec![KeyAction::Romaji("ka".into())]),
        ])]));
        assert!(!send_carries_romaji(&[]));
        assert!(!send_carries_romaji(&[
            KeyAction::Key(VkCode(0x41)),
            KeyAction::KeyUp(VkCode(0x41)),
            KeyAction::CtrlChord(VkCode(0x41)),
            KeyAction::Suppress,
        ]));
    }

    #[test]
    fn belief_origin_syncs_never_claim_actuation_origin() {
        assert_eq!(GjiFsmSync::OnImeOn.origin(), GjiSyncOrigin::Actuation);
        assert_eq!(GjiFsmSync::OnImeOff.origin(), GjiSyncOrigin::Actuation);
        assert_eq!(
            GjiFsmSync::OnImeOnBelief.origin(),
            GjiSyncOrigin::BeliefSync
        );
        assert_eq!(
            GjiFsmSync::Reopen(ReopenSource::Predict).origin(),
            GjiSyncOrigin::BeliefSync
        );
    }
}
