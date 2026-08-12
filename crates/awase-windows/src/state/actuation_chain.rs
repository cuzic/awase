//! Actuation の型状態チェーンと再試行 episode（ADR-089 §2.3・§2.6、INV-41）。
//!
//! # 何をコンパイラへ移したか
//!
//! - **`run_chain` は [`Actuation<Verified>`] にしか生えない**。`Requested` /
//!   `Warranted` の値から直接 write する経路は**型として存在しない**
//!   （ADR-089 §2.3、§7 の compile-fail ケース1）。
//! - **1 つの `Actuation` 値 = 高々 1 回の成功 write（アフィン性）**。
//!   [`Actuation::classify`] は `self` を consume し、フォールスルーする場合だけ
//!   [`WriteErr::Retryable`] で値を返す。値を使い回すことはできない。
//!
//! # 型が保証しないもの（INV-41、誤読防止）
//!
//! **回数制限は型ではなく [`decide_actuation_action`] の責務である。**
//! `FeedbackPolicy::Blind { max_attempts }` の下では、同一 warrant で最大
//! `max_attempts` 回の成功 write が正常に起こりうる（[`DriftEpisode`] が
//! attempt ごとに新しい `Actuation` を作る）。「型が回数を守っている」と
//! 読み替えて `decide_actuation_action` の呼び出しを省くと ADR-080 / BUG-43 の
//! give-up が無効化される。
//!
//! # ADR-089 の記述との差分（実装時の判断、2026-08-12）
//!
//! 1. **`run_chain` は writer を引数に取る。** ADR §2.3 は
//!    `async fn run_chain(self, chain: &[WriteMechanism]) -> ImeOpenOutcome` と
//!    書いているが、実際の write は Win32 FFI（`crate::ime::*`）であり、
//!    `state/` は ADR-065 に従って `#[cfg(windows)]` に依存できない。
//!    機構ごとの実 write は [`MechanismWriter`] / [`AsyncMechanismWriter`] で
//!    受け、走査・フォールスルー判定・アフィン性だけを本モジュールが持つ。
//!    これにより chain の走査規則は Linux で全数テストできる。
//! 2. **同期版と非同期版の 2 本を提供する。** ADR は `run_chain` を async 1 本に
//!    しているが、GJI / MS-IME / KanjiToggle は `SendInput` のみで非ブロッキング
//!    であり、これらを await 越しにすると打鍵ホットパスのレイテンシが変わる
//!    （ADR-089 §8.2 が「Phase B の実機ソークでレイテンシを測ること」と書いて
//!    いる軸）。実機ソークができない状態でホットパスの同期/非同期を変えないため、
//!    **走査規則（[`Actuation::classify`]）を単一の SSOT に置いたうえで**
//!    駆動側だけ 2 本にした。二重経路になっているのは「future を駆動する殻」で
//!    あって、フォールスルー述語でも戦略選択でもない。
//! 3. **`Actuation<Warranted>` の構築に [`Authorization::LegacyUnwarranted`] を
//!    用意した。** ADR §2.3 は `warrant(self, w: OpenWarrant)` だけを想定して
//!    いるが、`issue_open_warrant()`（ADR-087）の**本番呼び出し元は現時点で
//!    ゼロ**（`src/` を grep して確認、2026-08-12。ADR-087 Phase 3 の配線が
//!    未了）。既存の apply 経路に warrant を要求すると ADR-087 Phase 3 を
//!    Phase B に巻き込むことになるため、warrant を持たない既存経路のための
//!    名前付きの入口を分けた。件数は
//!    `tests/architecture_guard.rs::legacy_unwarranted_actuation_sites_are_accounted_for`
//!    が固定しており、増やすには期待値の更新が要る。
//!
//! # compile-fail ケース（ADR-089 §7 ケース1）
//!
//! `trybuild` ではなく **`compile_fail` doctest** で固定する（§7 の
//! 「保守負担についての注記」が挙げているとおり `trybuild` は rustc 更新で
//! `stderr` が変わり CI が赤くなる。doctest は stderr を照合しないため
//! rustc バージョンに依らず、dev-dependency も増えない）。
//! `compile_fail` は「何らかの理由でコンパイルが落ちれば通る」ため、
//! **1 行だけ違う「通る双子」を必ず併記して**、落ちている理由が
//! 目的の型エラーであることを示す。
//!
//! 通る双子（`Verified` から `run_chain` を呼ぶ）:
//!
//! ```
//! use awase::platform::ImeOpenOutcome;
//! use awase_windows::state::actuation_chain::{
//!     Actuation, MechanismWriter, VerifiedTarget, WriteMechanism,
//! };
//!
//! struct W;
//! impl MechanismWriter for W {
//!     fn is_applicable(&self, _m: WriteMechanism) -> bool { true }
//!     fn write(&mut self, _m: WriteMechanism, _open: bool) -> ImeOpenOutcome {
//!         ImeOpenOutcome::Applied
//!     }
//! }
//!
//! let act = Actuation::request(true)
//!     .warrant_pending_adr087()
//!     .verify(VerifiedTarget::FocusImplicit);
//! assert_eq!(act.run_chain(&WriteMechanism::ALL, &mut W), ImeOpenOutcome::Applied);
//! ```
//!
//! `Actuation<Warranted>` から直接 write することはできない
//! （`verify(..)` の 1 行を消しただけ）:
//!
//! ```compile_fail
//! use awase::platform::ImeOpenOutcome;
//! use awase_windows::state::actuation_chain::{
//!     Actuation, MechanismWriter, WriteMechanism,
//! };
//!
//! struct W;
//! impl MechanismWriter for W {
//!     fn is_applicable(&self, _m: WriteMechanism) -> bool { true }
//!     fn write(&mut self, _m: WriteMechanism, _open: bool) -> ImeOpenOutcome {
//!         ImeOpenOutcome::Applied
//!     }
//! }
//!
//! let act = Actuation::request(true).warrant_pending_adr087();
//! // error[E0599]: no method named `run_chain` found for `Actuation<Warranted>`
//! let _ = act.run_chain(&WriteMechanism::ALL, &mut W);
//! ```

use std::marker::PhantomData;

use awase::platform::ImeOpenOutcome;

use super::ime_actuation::{decide_actuation_action, ActuationAction, FeedbackPolicy};
use super::open_warrant::OpenWarrant;

// ── WriteMechanism ────────────────────────────────────────────────────────────

/// IME open を実際に書き込む機構。`ime_controller.rs` の 4 戦略と 1:1。
///
/// **キー値（VK）は持たない**——`state/key_sequence_policy.rs::ime_key_for` が
/// SSOT のままである（ADR-089 §2.8、INV-44。`docs/experiments.md` エントリ01 の
/// 回帰検知点を分裂させない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteMechanism {
    /// `ImmSetOpenStatus` のクロスプロセス呼び出し。VK を送らない。
    ImmCross,
    /// GJI 向けの冪等キー（`VK_IME_ON` / `VK_IME_OFF`）。
    GjiDirect,
    /// MS-IME 向けの冪等キー（`VK_IME_ON` / `VK_IME_OFF`）。
    MsImeDirect,
    /// 非冪等な `VK_KANJI` トグル。最終フォールバック。
    KanjiToggle,
}

impl WriteMechanism {
    /// `tests/ime_key_sequence_golden.rs` の `STRATEGY_NAMES` と同じ綴り。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ImmCross => "ImmCrossProcess",
            Self::GjiDirect => "GjiDirect",
            Self::MsImeDirect => "MsImeDirect",
            Self::KanjiToggle => "KanjiToggle",
        }
    }

    /// 全機構（`ime_controller.rs::ImeController::new` の構築順）。
    ///
    /// **これは「全 `caps` チェーンの和集合」であって、どれか 1 つの
    /// (profile, IME種別) のチェーンではない**（ADR-089 §2.8、Phase C）。
    /// 実際に走査する順序は `caps(p, k).chain`（`state/app_ime_policy.rs`）で
    /// あり、`ALL` を直接 chain として使ってよいのは
    /// 「起案時点の (p, k) を固定できない経路」だけである
    /// （`runtime/open_chain.rs` の非同期チェーン。同モジュール doc 参照）。
    pub const ALL: [Self; 4] = [
        Self::ImmCross,
        Self::GjiDirect,
        Self::MsImeDirect,
        Self::KanjiToggle,
    ];

    /// この機構が [`ImeOpenOutcome::Failed`] を返しうるか
    /// （= 後続要素へフォールスルーしうるか）。
    ///
    /// `ae64431d`〜現在の `ime_controller.rs` を実コードで確認した事実:
    ///
    /// | 機構 | 返しうる outcome |
    /// |---|---|
    /// | `ImmCross` | `Applied` / **`Failed`**（`set_ime_open_cross_process` の失敗） |
    /// | `GjiDirect` | `AlreadyMatched` / `Applied` / `UnsafeToToggle` |
    /// | `MsImeDirect` | `Applied` / `UnsafeToToggle` |
    /// | `KanjiToggle` | `FallbackSent` のみ |
    ///
    /// **`caps(p, k).chain` の末尾以外の要素は、必ずこれが真でなければならない**
    /// ——偽の機構の後ろに要素を置くと、その要素は現行のフォールスルー述語
    /// （[`falls_through`]、`Failed` のときだけ次へ）では**到達不能**になる
    /// （ADR-089 §2.8・§4.9、INV-44）。
    /// `state/app_ime_policy.rs::caps_chains_have_no_unreachable_trailing_element`
    /// が全 (p, k) で固定する。
    ///
    /// **この表が実装から drift していないこと**は、Windows 側の
    /// `ime_controller.rs::caps_chain_matches_legacy_all_scan` が
    /// （`is_applicable` × フォールスルー述語の実挙動と突き合わせる形で）固定する。
    /// 将来 `GjiDirectStrategy` / `MsImeDirectStrategy` が `Failed` を返すように
    /// 変わったら、ここを更新したうえで `caps` の末尾に `KanjiToggle` を足すか
    /// どうかを**実機ソーク付きで**判断すること
    /// （`.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリー）。
    #[must_use]
    pub const fn may_return_failed(self) -> bool {
        matches!(self, Self::ImmCross)
    }
}

/// フォールスルー述語の SSOT（ADR-089 §2.3、INV-44）。
///
/// 現行 `ImeController::apply_iter` と**同値**にする: 次の機構へ進むのは
/// `Failed` のときだけ。
///
/// **`UnsafeToToggle` を含めてはならない。** `UnsafeToToggle` は「Win キー押下中で
/// `send_ime_mode_key` が未送信」の意であり、ここでフォールスルーさせると
/// **Win キー押下中に非冪等な `VK_KANJI` を送る新経路**が生まれる
/// （ADR-089 §2.3・§4.9）。
#[must_use]
pub const fn falls_through(outcome: ImeOpenOutcome) -> bool {
    matches!(outcome, ImeOpenOutcome::Failed)
}

/// IME ON の直前に ROMAN ビットを補完する同期 IMC write が要るか
/// （ADR-089 §6 Phase C item 12 = ADR-086 INV-14 の未移行分の是正）。
///
/// # なぜこの述語がここ（ungated）にあるのか
///
/// Phase C 以前、この条件は `ime_controller.rs` の 2 つの戦略の中に**別々に**
/// 書かれていた（`ImmCrossProcessStrategy::apply` と
/// `MsImeDirectStrategy::apply`。どちらも `crate::ime::set_ime_romaji_mode()` を
/// 直接呼んでいた）。どちらも Win32 FFI と同居していたため Linux から
/// 条件を検査できず、`output/conv_actuation.rs` の doc が
/// 「ADR-086 Phase 1〜2 の『7 経路』の数え漏れ」と書いていた 2 経路そのもので
/// あった。Phase C で **書き込み口を 1 箇所（`ime_controller::apply_mechanism`）に
/// 統合**し、その発火条件だけをここへ純粋関数として切り出した。
///
/// # 条件（Phase C 以前と同値であること）
///
/// - `open == true` のときだけ（OFF 方向は ROMAN を触らない）。
/// - 機構が `ImmCross` または `MsImeDirect` のときだけ
///   （`GjiDirect` / `KanjiToggle` は元から ROMAN を書かない）。
/// - `kind == MsIme` のときだけ。旧 `ImmCrossProcessStrategy` は
///   `active_ime_kind == MicrosoftIme` を明示的に見ており、旧
///   `MsImeDirectStrategy` は見ていなかったが、`MsImeDirect` の
///   `is_applicable` 自体が `MicrosoftIme` を要求するため**同値**である
///   （`apply_mechanism` は `is_applicable` が真の機構に対してしか呼ばれない）。
/// - `belief_input_mode != ObservedKana` のときだけ——ユーザーが意図的に
///   かな入力を選んでいる状態を ROMAN で上書きしない（既存の保護、
///   `runtime/mod.rs::force_on_and_correct_romaji` の N2 も参照）。
#[must_use]
pub const fn needs_romaji_pre_write(
    mechanism: WriteMechanism,
    open: bool,
    kind: crate::state::ime_kind::ImeKindId,
    belief_input_mode: awase::engine::InputModeState,
) -> bool {
    open && matches!(
        mechanism,
        WriteMechanism::ImmCross | WriteMechanism::MsImeDirect
    ) && matches!(kind, crate::state::ime_kind::ImeKindId::MsIme)
        && !matches!(
            belief_input_mode,
            awase::engine::InputModeState::ObservedKana
        )
}

// ── 型状態 ────────────────────────────────────────────────────────────────────

/// 要求はあるが warrant がない段階。
#[derive(Debug)]
pub struct Requested;
/// 授権済み（ADR-087 `OpenWarrant`、または未配線経路の暫定授権）。
#[derive(Debug)]
pub struct Warranted;
/// 書き込み先が確定した段階（ADR-086 INV-14）。`run_chain` はここにしか生えない。
#[derive(Debug)]
pub struct Verified;

/// 書き込み先の同一性（ADR-086 INV-14）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedTarget {
    /// `ActuationTarget::capture()` を通った（ImmCross 非同期経路）。
    ///
    /// **hwnd 値そのものはここに持ち出さない。** `crate::ime::ActuationTarget` は
    /// フィールドを private にして「`verify_still_current` を経由せずに hwnd を
    /// 取り出せない」ことを型で保証しており（ADR-086 §6 段1）、その保証を
    /// 迂回するアクセサを生やさないため。
    Captured,
    /// VK 送信機構（GjiDirect / MsImeDirect / KanjiToggle）はフォアグラウンドの
    /// フォーカスへ送るため hwnd を捕獲しない。
    ///
    /// **これは ADR-086 INV-14 の未移行分である**（ADR-089 §6 Phase C item 12）。
    /// `Verified` に入れているのは「現行の到達性を変えない」ためであり、
    /// 「検証済み」という意味ではない。Phase C でここを潰す。
    FocusImplicit,
}

/// 授権の根拠。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorization {
    /// ADR-087 の `issue_open_warrant()` が発行した warrant。
    Warrant(OpenWarrant),
    /// ADR-087 Phase 3（`issue_open_warrant()` の本番配線）が未了の既存経路。
    ///
    /// **新しい呼び出し元を足さないこと。** 件数は architecture_guard が固定する。
    LegacyUnwarranted,
}

/// actuation 1 回分の値。**1 値 = 高々 1 回の成功 write**（INV-41）。
#[derive(Debug)]
pub struct Actuation<S> {
    open: bool,
    authorization: Authorization,
    target: Option<VerifiedTarget>,
    _state: PhantomData<fn() -> S>,
}

impl<S> Actuation<S> {
    /// この actuation が目指す open 値。
    #[must_use]
    pub const fn open(&self) -> bool {
        self.open
    }

    /// 授権の根拠。
    #[must_use]
    pub const fn authorization(&self) -> &Authorization {
        &self.authorization
    }
}

impl Actuation<Requested> {
    /// 要求を起こす。ここから warrant → verify を経ないと write できない。
    #[must_use]
    pub const fn request(open: bool) -> Self {
        Self {
            open,
            authorization: Authorization::LegacyUnwarranted,
            target: None,
            _state: PhantomData,
        }
    }

    /// ADR-087 の `OpenWarrant` で授権する（正規経路）。
    ///
    /// `warrant.target` と `self.open` が食い違う場合は `None`——warrant は
    /// 「その値を書いてよい」という授権であり、別の値の根拠にはならない。
    #[must_use]
    pub fn warrant(self, warrant: OpenWarrant) -> Option<Actuation<Warranted>> {
        if warrant.target != self.open {
            return None;
        }
        Some(Actuation {
            open: self.open,
            authorization: Authorization::Warrant(warrant),
            target: None,
            _state: PhantomData,
        })
    }

    /// ADR-087 Phase 3 が未配線の既存経路のための暫定授権（モジュール doc の
    /// 「ADR-089 の記述との差分」3 を参照）。
    ///
    /// **新規の呼び出し元を足さないこと。**
    #[must_use]
    pub fn warrant_pending_adr087(self) -> Actuation<Warranted> {
        Actuation {
            open: self.open,
            authorization: Authorization::LegacyUnwarranted,
            target: None,
            _state: PhantomData,
        }
    }
}

impl Actuation<Warranted> {
    /// 書き込み先を確定する（ADR-086 INV-14）。
    #[must_use]
    pub fn verify(self, target: VerifiedTarget) -> Actuation<Verified> {
        Actuation {
            open: self.open,
            authorization: self.authorization,
            target: Some(target),
            _state: PhantomData,
        }
    }
}

/// 1 機構への write の帰結。
#[derive(Debug)]
pub enum WriteErr {
    /// 次の機構へフォールバックしてよい。`Actuation<Verified>` を返して
    /// 連鎖を保存する（値を落とさない）。
    Retryable(Actuation<Verified>, ImeOpenOutcome),
    /// 連鎖を打ち切る。
    Fatal(ImeOpenOutcome),
}

impl Actuation<Verified> {
    /// 確定した書き込み先。
    #[must_use]
    pub const fn target(&self) -> VerifiedTarget {
        match self.target {
            Some(t) => t,
            // `Verified` は `verify()` でしか作れないため常に `Some`。
            None => VerifiedTarget::FocusImplicit,
        }
    }

    /// 1 回の write の結果を評価し、連鎖を続けるか打ち切るかを決める。
    ///
    /// **`self` を consume する**（アフィン性）。続ける場合だけ
    /// [`WriteErr::Retryable`] が値を返す。
    ///
    /// # Errors
    /// `Failed` のとき [`WriteErr::Retryable`] を返す（＝次の機構へ進む）。
    pub fn classify(self, outcome: ImeOpenOutcome) -> Result<ImeOpenOutcome, WriteErr> {
        if falls_through(outcome) {
            Err(WriteErr::Retryable(self, outcome))
        } else {
            Ok(outcome)
        }
    }

    /// chain を使い切ったので値を捨てる。`Fatal(Failed)` に落とす。
    #[must_use]
    pub fn abandon(self) -> WriteErr {
        WriteErr::Fatal(ImeOpenOutcome::Failed)
    }

    /// chain を順に試す（同期 writer 版）。
    ///
    /// `apply_iter`（`ime_controller.rs`）と同値の走査:
    /// `is_applicable` な機構だけを順に試し、`Failed` のときだけ次へ進む。
    /// 適用可能な機構が 1 つも無い / 全て `Failed` の場合は `Failed`。
    #[must_use]
    pub fn run_chain<W: MechanismWriter + ?Sized>(
        self,
        chain: &[WriteMechanism],
        writer: &mut W,
    ) -> ImeOpenOutcome {
        let mut act = self;
        for &mechanism in chain {
            if !writer.is_applicable(mechanism) {
                continue;
            }
            let open = act.open();
            let outcome = writer.write(mechanism, open);
            match act.classify(outcome) {
                Ok(terminal) => return terminal,
                Err(WriteErr::Retryable(next, _)) => {
                    log::debug!(
                        "[apply-ime] {} failed, trying next fallback",
                        mechanism.name()
                    );
                    act = next;
                }
                Err(WriteErr::Fatal(outcome)) => return outcome,
            }
        }
        match act.abandon() {
            WriteErr::Fatal(outcome) | WriteErr::Retryable(_, outcome) => outcome,
        }
    }

    /// chain を順に試す（非同期 writer 版）。走査規則は [`Self::run_chain`] と
    /// 同一で、判定はどちらも [`Self::classify`] 1 箇所に集約している。
    pub async fn run_chain_async<W: AsyncMechanismWriter + ?Sized>(
        self,
        chain: &[WriteMechanism],
        writer: &mut W,
    ) -> ImeOpenOutcome {
        let mut act = self;
        for &mechanism in chain {
            if !writer.is_applicable(mechanism) {
                continue;
            }
            let open = act.open();
            let outcome = writer.write(mechanism, open).await;
            match act.classify(outcome) {
                Ok(terminal) => return terminal,
                Err(WriteErr::Retryable(next, _)) => {
                    log::debug!(
                        "[apply-ime] {} failed, trying next fallback (async)",
                        mechanism.name()
                    );
                    act = next;
                }
                Err(WriteErr::Fatal(outcome)) => return outcome,
            }
        }
        match act.abandon() {
            WriteErr::Fatal(outcome) | WriteErr::Retryable(_, outcome) => outcome,
        }
    }
}

// ── writer ────────────────────────────────────────────────────────────────────

/// 機構ごとの実 write。Windows 側だけが実装する（`state/` は FFI を持たない）。
pub trait MechanismWriter {
    /// この機構が現在のコンテキストで適用可能か。
    fn is_applicable(&self, mechanism: WriteMechanism) -> bool;
    /// 実際に書き込む。
    fn write(&mut self, mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome;
}

/// [`MechanismWriter`] の非同期版（ImmCross のクロスプロセス書き込み用）。
pub trait AsyncMechanismWriter {
    /// この機構が現在のコンテキストで適用可能か。
    fn is_applicable(&self, mechanism: WriteMechanism) -> bool;
    /// 実際に書き込む。
    fn write(
        &mut self,
        mechanism: WriteMechanism,
        open: bool,
    ) -> impl std::future::Future<Output = ImeOpenOutcome>;
}

// ── DriftEpisode（ADR-089 §2.6）────────────────────────────────────────────────

/// 再試行 episode。attempt ごとに新しい [`Actuation<Warranted>`] を作る。
///
/// **warrant の有効性は episode 単位**であり、`Actuation` 値のアフィン性
/// （1 値 = 高々 1 回の成功 write）と、[`decide_actuation_action`] による
/// 回数制限がここで組み合わさる（INV-41）。
#[derive(Debug, Clone)]
pub struct DriftEpisode {
    warrant: OpenWarrant,
    policy: FeedbackPolicy,
    attempts: u32,
}

impl DriftEpisode {
    /// episode を開始する。`warrant` は episode 全体で有効。
    #[must_use]
    pub const fn new(warrant: OpenWarrant, policy: FeedbackPolicy) -> Self {
        Self {
            warrant,
            policy,
            attempts: 0,
        }
    }

    /// これまでに払い出した attempt 数。
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// この episode の feedback 方針。
    #[must_use]
    pub const fn policy(&self) -> FeedbackPolicy {
        self.policy
    }

    /// この episode が目指す open 値。
    #[must_use]
    pub const fn target(&self) -> bool {
        self.warrant.target
    }

    /// 次の attempt を払い出す。`decide_actuation_action` が `GiveUp` を返したら
    /// `None`（**回数制限は型ではなくこの関数の責務**、INV-41）。
    ///
    /// `Actuation` 値を使い回さないこと——毎回ここで新規に作るのが
    /// アフィン性の実効条件である。
    pub fn next_attempt(&mut self) -> Option<Actuation<Warranted>> {
        if decide_actuation_action(self.policy, self.attempts) == ActuationAction::GiveUp {
            return None;
        }
        self.attempts += 1;
        Actuation::request(self.warrant.target).warrant(self.warrant.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ime_event::ObservationSource;
    use crate::state::open_warrant::WarrantBasis;

    const ALL_OUTCOMES: [ImeOpenOutcome; 5] = [
        ImeOpenOutcome::Applied,
        ImeOpenOutcome::FallbackSent,
        ImeOpenOutcome::AlreadyMatched,
        ImeOpenOutcome::Failed,
        ImeOpenOutcome::UnsafeToToggle,
    ];

    fn warrant(target: bool) -> OpenWarrant {
        OpenWarrant {
            target,
            basis: WarrantBasis::DirectRead(ObservationSource::ImmGetOpenStatus),
        }
    }

    fn verified(open: bool) -> Actuation<Verified> {
        Actuation::request(open)
            .warrant_pending_adr087()
            .verify(VerifiedTarget::FocusImplicit)
    }

    /// 記録付きのフェイク writer。適用可能な機構と、各機構が返す outcome を指定する。
    struct FakeWriter {
        applicable: Vec<WriteMechanism>,
        outcomes: Vec<ImeOpenOutcome>,
        calls: Vec<(WriteMechanism, bool)>,
    }

    impl FakeWriter {
        fn new(applicable: &[WriteMechanism], outcomes: &[ImeOpenOutcome]) -> Self {
            Self {
                applicable: applicable.to_vec(),
                outcomes: outcomes.to_vec(),
                calls: Vec::new(),
            }
        }
    }

    impl MechanismWriter for FakeWriter {
        fn is_applicable(&self, mechanism: WriteMechanism) -> bool {
            self.applicable.contains(&mechanism)
        }
        fn write(&mut self, mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome {
            self.calls.push((mechanism, open));
            let idx = self.calls.len() - 1;
            *self.outcomes.get(idx).unwrap_or(&ImeOpenOutcome::Applied)
        }
    }

    impl AsyncMechanismWriter for FakeWriter {
        fn is_applicable(&self, mechanism: WriteMechanism) -> bool {
            MechanismWriter::is_applicable(self, mechanism)
        }
        async fn write(&mut self, mechanism: WriteMechanism, open: bool) -> ImeOpenOutcome {
            MechanismWriter::write(self, mechanism, open)
        }
    }

    /// future を 1 回だけ poll して結果を取り出す（テスト用の最小ドライバ）。
    /// `FakeWriter` の `write` は await 点を持たないため必ず 1 回で完了する。
    fn poll_once<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = Box::pin(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("test future must complete without yielding"),
        }
    }

    /// フォールスルー述語は `Failed` のときだけ真（ADR-089 §2.3 の表と全数一致）。
    #[test]
    fn falls_through_only_on_failed() {
        for outcome in ALL_OUTCOMES {
            assert_eq!(
                falls_through(outcome),
                outcome == ImeOpenOutcome::Failed,
                "{outcome:?}"
            );
        }
    }

    /// **`UnsafeToToggle` はフォールスルーしない**（§2.3・§4.9: Win キー押下中に
    /// 非冪等な `VK_KANJI` を送る新経路を作らない）。
    #[test]
    fn unsafe_to_toggle_stops_the_chain_before_kanji_toggle() {
        let mut writer = FakeWriter::new(
            &[WriteMechanism::GjiDirect, WriteMechanism::KanjiToggle],
            &[ImeOpenOutcome::UnsafeToToggle],
        );
        let outcome = verified(true).run_chain(&WriteMechanism::ALL, &mut writer);
        assert_eq!(outcome, ImeOpenOutcome::UnsafeToToggle);
        assert_eq!(writer.calls, vec![(WriteMechanism::GjiDirect, true)]);
    }

    /// `Failed` のときだけ次の機構へ進む。
    #[test]
    fn failed_falls_through_to_next_applicable_mechanism() {
        let mut writer = FakeWriter::new(
            &[WriteMechanism::ImmCross, WriteMechanism::GjiDirect],
            &[ImeOpenOutcome::Failed, ImeOpenOutcome::Applied],
        );
        let outcome = verified(false).run_chain(&WriteMechanism::ALL, &mut writer);
        assert_eq!(outcome, ImeOpenOutcome::Applied);
        assert_eq!(
            writer.calls,
            vec![
                (WriteMechanism::ImmCross, false),
                (WriteMechanism::GjiDirect, false),
            ]
        );
    }

    /// 適用不能な機構は呼ばれない（`apply_iter` の `is_applicable` と同値）。
    #[test]
    fn inapplicable_mechanisms_are_skipped() {
        let mut writer = FakeWriter::new(
            &[WriteMechanism::KanjiToggle],
            &[ImeOpenOutcome::FallbackSent],
        );
        let outcome = verified(true).run_chain(&WriteMechanism::ALL, &mut writer);
        assert_eq!(outcome, ImeOpenOutcome::FallbackSent);
        assert_eq!(writer.calls, vec![(WriteMechanism::KanjiToggle, true)]);
    }

    /// 適用可能な機構が無ければ `Failed`（現行 `apply_iter` の末尾と同じ）。
    #[test]
    fn empty_chain_yields_failed() {
        let mut writer = FakeWriter::new(&[], &[]);
        assert_eq!(
            verified(true).run_chain(&WriteMechanism::ALL, &mut writer),
            ImeOpenOutcome::Failed
        );
        assert!(writer.calls.is_empty());
    }

    /// 全機構が `Failed` を返したら `Failed`。
    #[test]
    fn all_failed_yields_failed() {
        let mut writer = FakeWriter::new(&WriteMechanism::ALL, &[ImeOpenOutcome::Failed; 4]);
        assert_eq!(
            verified(true).run_chain(&WriteMechanism::ALL, &mut writer),
            ImeOpenOutcome::Failed
        );
        assert_eq!(writer.calls.len(), 4);
    }

    /// 非同期版と同期版の走査結果が全 outcome 組み合わせで一致すること
    /// （走査規則の SSOT が `classify` 1 箇所であることの実行時確認）。
    #[test]
    fn async_and_sync_chains_agree() {
        for first in ALL_OUTCOMES {
            for second in ALL_OUTCOMES {
                let chain = [WriteMechanism::ImmCross, WriteMechanism::GjiDirect];
                let mut sync_writer = FakeWriter::new(&chain, &[first, second]);
                let mut async_writer = FakeWriter::new(&chain, &[first, second]);
                let sync = verified(true).run_chain(&chain, &mut sync_writer);
                let asy = poll_once(verified(true).run_chain_async(&chain, &mut async_writer));
                assert_eq!(sync, asy, "first={first:?} second={second:?}");
                assert_eq!(sync_writer.calls, async_writer.calls);
            }
        }
    }

    /// `warrant()` は warrant の target と食い違う要求を弾く。
    #[test]
    fn warrant_must_agree_with_requested_open() {
        assert!(Actuation::request(true).warrant(warrant(true)).is_some());
        assert!(Actuation::request(true).warrant(warrant(false)).is_none());
    }

    /// `DriftEpisode` は `Blind` の `max_attempts` で払い出しを止める
    /// （**回数制限は型ではなく `decide_actuation_action`**、INV-41）。
    #[test]
    fn drift_episode_stops_at_blind_max_attempts() {
        let policy = FeedbackPolicy::Blind {
            max_attempts: 3,
            backoff: std::time::Duration::from_millis(1),
        };
        let mut episode = DriftEpisode::new(warrant(true), policy);
        for expected in 1..=3 {
            let attempt = episode.next_attempt().expect("まだ諦めない");
            assert!(attempt.open());
            assert_eq!(episode.attempts(), expected);
        }
        assert!(episode.next_attempt().is_none(), "max_attempts で打ち切る");
        assert_eq!(episode.attempts(), 3, "GiveUp では attempts を進めない");
    }

    /// `Read` は試行回数では打ち切らない（`decide_actuation_action` と同じ挙動）。
    #[test]
    fn drift_episode_never_gives_up_under_read_policy() {
        let policy = FeedbackPolicy::Read {
            source: ObservationSource::ImmGetOpenStatus,
            deadline: std::time::Duration::from_millis(1),
        };
        let mut episode = DriftEpisode::new(warrant(false), policy);
        for _ in 0..32 {
            assert!(episode.next_attempt().is_some());
        }
        assert_eq!(episode.attempts(), 32);
    }

    /// episode の warrant は attempt へそのまま引き継がれる。
    #[test]
    fn episode_attempts_carry_the_episode_warrant() {
        let policy = FeedbackPolicy::Read {
            source: ObservationSource::ImmGetOpenStatus,
            deadline: std::time::Duration::from_millis(1),
        };
        let mut episode = DriftEpisode::new(warrant(true), policy);
        let attempt = episode.next_attempt().unwrap();
        assert_eq!(
            attempt.authorization(),
            &Authorization::Warrant(warrant(true))
        );
        assert!(episode.target());
    }

    /// `verify` した target は最後まで保持される（ADR-086 INV-14 の型状態化）。
    #[test]
    fn verified_target_is_preserved_across_fallthrough() {
        let target = VerifiedTarget::Captured;
        let act = Actuation::request(true)
            .warrant_pending_adr087()
            .verify(target);
        assert_eq!(act.target(), target);
        let Err(WriteErr::Retryable(next, outcome)) = act.classify(ImeOpenOutcome::Failed) else {
            panic!("Failed は Retryable でなければならない");
        };
        assert_eq!(outcome, ImeOpenOutcome::Failed);
        assert_eq!(next.target(), target, "フォールバック後も同じ宛先");
    }

    /// `may_return_failed` は `ImmCross` だけが真（ADR-089 §2.3 の実コード調査）。
    #[test]
    fn only_imm_cross_may_return_failed() {
        for mechanism in WriteMechanism::ALL {
            assert_eq!(
                mechanism.may_return_failed(),
                mechanism == WriteMechanism::ImmCross,
                "{mechanism:?}"
            );
        }
    }

    /// `may_return_failed` が真の機構だけが `falls_through` を起こしうる
    /// ——`run_chain` の走査と `caps` の末尾規則が同じ事実に依っていることの確認。
    #[test]
    fn only_failed_capable_mechanisms_can_fall_through() {
        for mechanism in WriteMechanism::ALL {
            let reachable_fall_through = ALL_OUTCOMES
                .iter()
                .filter(|o| falls_through(**o))
                .any(|_| mechanism.may_return_failed());
            assert_eq!(reachable_fall_through, mechanism.may_return_failed());
        }
    }

    // ── needs_romaji_pre_write（ADR-089 Phase C item 12 / ADR-086 INV-14）──

    use crate::state::ime_kind::ImeKindId;
    use awase::engine::{AssumedReason, InputModeState};

    const ALL_INPUT_MODES: [InputModeState; 5] = [
        InputModeState::ObservedRomaji,
        InputModeState::ObservedKana,
        InputModeState::ObservedEisu,
        InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        },
        InputModeState::Unknown,
    ];

    /// Phase C 以前の 2 戦略の条件と同値であることを全数で固定する。
    ///
    /// 旧条件:
    /// - `ImmCrossProcessStrategy::apply`:
    ///   `open && active_ime_kind == MicrosoftIme && belief != ObservedKana`
    /// - `MsImeDirectStrategy::apply`: `open && belief != ObservedKana`
    ///   （`is_applicable` が `MicrosoftIme` を要求するため kind 条件は暗黙）
    #[test]
    fn romaji_pre_write_condition_matches_the_pre_phase_c_strategies() {
        for mechanism in WriteMechanism::ALL {
            for open in [true, false] {
                for kind in ImeKindId::ALL {
                    for mode in ALL_INPUT_MODES {
                        let expected =
                            open && matches!(
                                mechanism,
                                WriteMechanism::ImmCross | WriteMechanism::MsImeDirect
                            ) && kind == ImeKindId::MsIme
                                && mode != InputModeState::ObservedKana;
                        assert_eq!(
                            needs_romaji_pre_write(mechanism, open, kind, mode),
                            expected,
                            "{mechanism:?} open={open} {kind:?} {mode:?}"
                        );
                    }
                }
            }
        }
    }

    /// GJI 経路では ROMAN 補完を一切行わない（Phase C 以前も同じ）。
    #[test]
    fn romaji_pre_write_never_fires_for_gji_mechanisms() {
        for mechanism in [WriteMechanism::GjiDirect, WriteMechanism::KanjiToggle] {
            for kind in ImeKindId::ALL {
                assert!(!needs_romaji_pre_write(
                    mechanism,
                    true,
                    kind,
                    InputModeState::Unknown
                ));
            }
        }
    }

    /// `ObservedKana`（ユーザーが意図的にかな入力を選んだ状態）は上書きしない。
    #[test]
    fn romaji_pre_write_respects_observed_kana() {
        assert!(!needs_romaji_pre_write(
            WriteMechanism::MsImeDirect,
            true,
            ImeKindId::MsIme,
            InputModeState::ObservedKana
        ));
    }

    /// 機構名は golden（`tests/ime_key_sequence_golden.rs`）の綴りと一致する。
    #[test]
    fn mechanism_names_match_strategy_names() {
        assert_eq!(
            WriteMechanism::ALL.map(WriteMechanism::name),
            ["ImmCrossProcess", "GjiDirect", "MsImeDirect", "KanjiToggle"]
        );
    }
}
