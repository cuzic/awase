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
}

impl GjiFsmSync {
    /// モジュール private。外から `GjiFsmSync` を作る唯一の経路は
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
/// use awase_windows::state::gji_direct_mechanism::{
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
/// use awase_windows::state::gji_direct_mechanism::ActuationReceipt;
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
/// MsImeDirect / KanjiToggle）で actuate したか、ひいてはどの `ImeProfileDriver` を
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

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_OUTCOMES: [ImeOpenOutcome; 6] = [
        ImeOpenOutcome::Applied,
        ImeOpenOutcome::FallbackSent,
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
            ImeOpenOutcome::FallbackSent,
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
}
