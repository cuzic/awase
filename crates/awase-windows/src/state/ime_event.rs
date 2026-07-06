//! IME 状態モデルの event 型定義 (Step 0)
//!
//! Reducer リファクタリングの足場として、IME 状態変更に関する全 event を表現する。
//! 現状 (Step 0) では event を log するのみで、本番判定には使わない。
//!
//! ## 設計原則
//!
//! - **event は immutable record**: 一度記録したら書き換えない。
//! - **時刻ではなく `seq` で順序を決める**: `GetTickCount` は wall clock 由来で
//!   逆転する可能性があるため、reducer の順序判断は必ず `EventTime::seq` を使う。
//! - **event の payload は早めに増やす**: 後で reducer が判断材料に使う情報
//!   (`hwnd`, `confidence`, `generation` 等) は最初から持たせる。

use std::time::Instant;

use awase::engine::InputModeState;

use super::TickMs;

/// HWND の Send-safe な表現 (raw pointer 値を usize で保持)。
///
/// 実際の `HWND` は raw pointer を含むため Send/Sync ではない。
/// event log でクロススレッド伝搬される可能性があるため、ここでは値だけ保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HwndId(pub usize);

impl HwndId {
    pub const NULL: Self = Self(0);

    #[must_use]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

#[cfg(windows)]
impl HwndId {
    /// `HWND` に変換する。`windows` クレートの型変化 (`isize` → `*mut c_void`) に
    /// 対してここだけ修正すれば済むよう、raw cast を一箇所に集約する。
    #[must_use]
    pub fn to_hwnd(self) -> windows::Win32::Foundation::HWND {
        windows::Win32::Foundation::HWND(self.0 as *mut _)
    }
}

#[cfg(windows)]
impl From<windows::Win32::Foundation::HWND> for HwndId {
    fn from(hwnd: windows::Win32::Foundation::HWND) -> Self {
        Self(hwnd.0 as usize)
    }
}

/// Event の時刻情報。reducer の順序判断は `seq` を使い、経過時間計算は
/// `monotonic` を使い、既存ログとの互換には `tick_ms` を使う。
#[derive(Debug, Clone, Copy)]
pub struct EventTime {
    /// 全 event を通じて単調増加する番号。順序判断はこれを使う。
    pub seq: u64,
    /// `Instant::now()` で取得した単調時刻。経過時間計算に使う。
    pub monotonic: Instant,
    /// `GetTickCount64()` 由来の ms。既存ログとの互換用。
    pub tick_ms: u64,
}

/// ユーザー意図のソース。
///
/// `UserImeSetIntent` / `UserImeToggleIntent` の `source` フィールドに使う。
/// 復旧操作 (`PanicReset`) や HWND キャッシュ復元 (`HwndCacheRestored`) は
/// 専用イベントを持つため、このリストには含まない。
/// `Recovery` や `HwndCache` をここに追加すると `desired_open` を
/// "ユーザー意図として" 書き換えられてしまうため、列挙値として存在してはならない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserIntentSource {
    /// 設定された同期キー (Shift+Space 等)
    SyncKey,
    /// 物理 KANJI 押下 (VK_F3/F4)
    PhysicalImeKey,
    /// awase エンジン内部の判断 (Engine から SetOpen 要求等)
    Command,
}

/// Observation のソース (外部観測の種類)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationSource {
    /// フォーカス変更直後の同期プローブ
    FocusProbe,
    /// 500ms 周期のバックグラウンドポーリング
    ObserverPoll,
    /// GJI (GetGuiThreadInfo) 由来
    Gji,
    /// `ImmGetOpenStatus` 直接呼び出し
    ImmGetOpenStatus,
    /// conversion mode ビット（`ImmGetConversionStatus` 由来）からの input_mode 推定。
    ///
    /// idle-conv-check（TsfNative）が読み取った conv ビットを `classify_conv_transition`
    /// で解釈して input_mode を導く経路。`ImmGetOpenStatus` API を直接呼んだわけではない
    /// （open 状態ではなく conversion mode を読んでいる）ため、そのソースを名乗るのは
    /// 偽装になる。conv の読み取り自体は直接 API 成功なので confidence は `High` で扱うが、
    /// 「何を観測したか」を正直に表すためソースを分離する。
    ConvBitsInference,
    /// TSF observer 由来
    Tsf,
    /// per-HWND IME キャッシュからの復元
    HwndCache,
    /// フォーカス変更後の ImmCross 非同期プローブ
    ///
    /// Qt/LINE 等の ImmCross アプリで、フォーカス直後に `GetGUIThreadInfo.hwndFocus`
    /// （子 hwnd）の IMM32 状態を `read_ime_state_full_async` で読む高信頼ソース。
    /// `FocusProbe` が top-level hwnd の IMC を読む（Low）のと対になる。
    ImmCrossProbe,
    /// 観測が一切ない状態（cache miss 等）での安全デフォルトの推測。
    ///
    /// 実際の外部観測ではなく awase 側のポリシー的な best-guess のため、
    /// 必ず `ObservationConfidence::Low` で record すること。`derive_open()` の
    /// Medium+ 多数決には参加しないが、他に観測が一切ない場合の
    /// `effective_open()` フォールバックとしてのみ使われる。真の観測（Lowでも）が
    /// 後から届けば、鮮度・信頼度が同等以上のため上書きされる。
    HeuristicDefault,
}

/// 観測の信頼度。reducer が profile 別に judge する際に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObservationConfidence {
    /// 推測ベース (FocusProbe で blacklist 回避等)
    Low,
    /// 間接観測 (GJI / TSF observer)
    Medium,
    /// 直接 API 成功 (ImmGetOpenStatus 成功)
    High,
}

/// state 層が保持するアプリ IME 制御プロファイル。
///
/// `focus::class_names::AppImeProfile`（クラス名判定に特化した focus 層の型）への
/// 逆依存を断つため、state 層では独自の列挙型を定義する。
/// `FocusChanged` event のペイロードとして運ばれ、reducer が `AppImePolicy` を導出するために使う。
///
/// `From<AppImeProfile> for ImePolicyProfile` は focus 層（`focus::class_names`）に実装し、
/// runtime 境界でフォーカス変更時に変換する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImePolicyProfile {
    /// 通常の Win32 アプリ。IMM32 クロスプロセス制御（ImmCross）が使用可能。
    ImmCross,
    /// Chrome/Edge/UWP 等。IMM32 クロスプロセス制御が使えず、VK_KANJI で制御する。
    Imm32Unavailable,
    /// TSF ネイティブ（例: WezTerm/Windows Terminal）。`VK_DBE_HIRAGANA` + TSF probe が必要。
    TsfNative,
    /// IME 制御が不要なシンプルなアプリ（将来拡張用）。
    Plain,
    /// 未分類。起動直後または分類情報が得られない場合のデフォルト。
    #[default]
    Unknown,
}

/// 入力 chord の種別 (Step 4 で使う、Step 0 では定義のみ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordKind {
    /// Ctrl + 無変換 → IME OFF
    CtrlMuhenkanImeOff,
    /// Ctrl + 変換 → IME ON
    CtrlHenkanImeOn,
}

/// Apply 失敗の種別 (Step 7 で使う、Step 0 では定義のみ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyError {
    /// タイムアウト
    Timeout,
    /// クロスプロセス IMM 呼び出し失敗
    CrossProcessFailed,
    /// トグル操作が unsafe（shadow 信頼度不足・focus 直後等）で送信しなかった
    UnsafeToToggle,
    /// その他
    Other,
}

/// `InputModeApplied` event における適用手段。
///
/// awase が能動的に入力モードを変更するとき、どの経路で行ったかを記録する。
/// reducer が適用後の belief 更新や競合解決に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputModeApplyStrategy {
    /// IMM-broken アプリ（Chrome/Edge 等）向けの強制補正 (AssumedRomaji)。
    /// IMM クロスプロセス呼び出しが不可のため、観測値を捨てて仮定に切り替える。
    ImmBrokenCorrection,
    /// パニックリセット時の強制 ObservedRomaji 設定。
    PanicReset,
    /// hwnd キャッシュからの入力モード復元（前回フォーカス時の belief を再現）。
    CacheRestore,
    /// `SetOpen(true)` 適用直後、stale な `ObservedEisu` を先回りで訂正する。
    ///
    /// 外部を観測したのではなく、awase 自身が直前に発行した SetOpen の帰結
    /// （GJI がひらがなへ遷移するはず）を先読みする内部補正。1500ms 後の
    /// idle-conv-check が実際の GJI 状態で再確認・再訂正する。
    PostSetOpenEisuReset,
}

/// `InputModeApplied` event における適用結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputModeApplyResult {
    /// 入力モードを変更した。
    Applied,
    /// ObservedEisu guard 等の条件でスキップした（モード変更なし）。
    Skipped,
}

/// IME 状態モデルへの全 event。
///
/// 時刻情報は `ImeEventEnvelope::time` に集約する (event 内に重複させない)。
#[derive(Debug, Clone)]
pub enum ImeEvent {
    /// ユーザー/awase が IME を toggle したい意図
    UserImeToggleIntent { source: UserIntentSource },

    /// ユーザー/awase が IME を ON/OFF に設定したい意図
    UserImeSetIntent { target: bool, source: UserIntentSource },

    /// パニックリセット: 復旧として desired_open を `target` に戻す。
    ///
    /// `UserImeSetIntent` と違い `last_intent` を設定しない。
    /// `ForceGuard::PanicReset` が IME ON を保証するため、このイベントは
    /// `desired_open` のみ安全デフォルト値に戻す（`has_user_explicit_intent()` を
    /// 汚染しない）。Recovery コードは `UserImeSetIntent` ではなくこれを使うこと。
    PanicReset { target: bool },

    /// HWND キャッシュ復元: 前回フォーカス時の desired_open を回復する。
    ///
    /// `UserImeSetIntent` と違い `last_intent` を設定しない。
    /// キャッシュ復元はユーザーの能動的操作ではないため、`has_user_explicit_intent()`
    /// を true にしてはならない。HwndCache 復元コードはこれを使うこと。
    HwndCacheRestored { target: bool },

    /// OS への適用を開始した。
    ///
    /// `ctrl_held` は dispatch 時点で Ctrl が押下されていたか。reducer が
    /// 「IME OFF 要求 + Ctrl 押下中 → CtrlImeChord barrier を立てる」判断に使う。
    ImeApplyRequested {
        target: bool,
        generation: u64,
        ctrl_held: bool,
    },

    /// OS への適用が成功した (async 完了時、generation 照合必須)
    ImeApplySucceeded { target: bool, generation: u64 },

    /// OS への適用が失敗した
    ImeApplyFailed {
        target: bool,
        generation: u64,
        error: ApplyError,
    },

    /// 外部観測が値を報告した (desired を直接書き換えない)
    ObserverReported {
        open: bool,
        source: ObservationSource,
        hwnd: HwndId,
        confidence: ObservationConfidence,
        /// 観測が受理されたフォーカスエポック (`probe_admission::FocusEpoch`)。
        /// 同期 probe は呼び出し時点の現在エポック。
        /// 非同期 probe は `ImmLikeTicket::admit()` が照合済みのエポック。
        focus_epoch: crate::state::probe_admission::FocusEpoch,
    },

    /// フォーカスが変わった
    FocusChanged {
        from: Option<HwndId>,
        to: HwndId,
        profile: ImePolicyProfile,
        /// インクリメント後のフォーカスエポック。
        /// reducer が `ObservationStore::current_focus_epoch` を更新するために使う。
        focus_epoch: crate::state::probe_admission::FocusEpoch,
    },

    /// Chord transaction の開始 (Ctrl+無変換 押下時等)
    ChordStarted { kind: ChordKind },

    /// Chord transaction の終了 (Ctrl KeyUp 等)
    ChordEnded { kind: ChordKind },

    /// desired と observed の乖離が一定時間続いた
    DriftDetected {
        desired: bool,
        observed: bool,
        duration_ms: u64,
    },

    /// 入力モード（ローマ字/かな/英数 等）を外部から観測した。
    ///
    /// GJI probe・IMM クエリ・conv_mode ビット変化など passively 取得した値を通知する。
    /// reducer は `confidence >= Medium` の場合のみ `ImeModel::input_mode` をこの値で
    /// 上書きする（ON/OFF の `derive_open()` と同じ考え方: Low confidence だけでは
    /// belief を動かさない）。`source` に見合わない confidence を付けないこと —
    /// 実際に外部 API/probe を呼んでいない場合はこのイベントを使わず、
    /// awase 自身の能動的な訂正は `InputModeApplied` を使うこと。
    ///
    /// `at` は観測を取得したときの tick_ms（envelop time と一致することが多いが、
    /// 非同期 probe が完了した時刻を明示したい場合は別値になることがある）。
    InputModeObserved {
        mode: InputModeState,
        source: ObservationSource,
        confidence: ObservationConfidence,
        at: TickMs,
    },

    /// awase が能動的に入力モードを変更した（または変更しようとした）。
    ///
    /// IMM-broken 補正・パニックリセット・フォーカスリセット・キャッシュ復元など、
    /// awase 側が belief を書き換える経路はすべてこのイベントで通知する。
    /// `result` が `Skipped` の場合 reducer は `input_mode` を更新しない。
    InputModeApplied {
        mode: InputModeState,
        strategy: InputModeApplyStrategy,
        result: InputModeApplyResult,
        at: TickMs,
    },

    /// ユーザーが入力モードを明示的に変更した。
    ///
    /// Ctrl+Caps・VK_DBE_ROMAN・VK_DBE_HIRAGANA などのユーザー操作で
    /// input_mode が決定したときに通知する。
    UserChangedInputMode { mode: InputModeState, at: TickMs },
}

impl ImeEvent {
    /// `apply_ime_open` の outcome を Succeeded/Failed event に変換する。
    /// sync / async 両経路で使う single source of truth。
    #[must_use]
    pub const fn from_apply_outcome(
        target: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: u64,
    ) -> Self {
        use awase::platform::ImeOpenOutcome;
        match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => Self::ImeApplySucceeded { target, generation },
            ImeOpenOutcome::Failed => Self::ImeApplyFailed {
                target,
                generation,
                error: ApplyError::CrossProcessFailed,
            },
            ImeOpenOutcome::UnsafeToToggle => Self::ImeApplyFailed {
                target,
                generation,
                error: ApplyError::UnsafeToToggle,
            },
        }
    }
}

/// Event log に積まれる envelope。時刻情報と event 本体をまとめる。
#[derive(Debug, Clone)]
pub struct ImeEventEnvelope {
    pub time: EventTime,
    pub event: ImeEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hwnd_id_null_check() {
        assert!(HwndId::NULL.is_null());
        assert!(!HwndId(0x1234).is_null());
    }

    #[test]
    fn confidence_ordering() {
        assert!(ObservationConfidence::Low < ObservationConfidence::Medium);
        assert!(ObservationConfidence::Medium < ObservationConfidence::High);
    }
}
