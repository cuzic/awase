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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum UserIntentSource {
    /// 設定された同期キー (Shift+Space 等)
    SyncKey,
    /// 物理 KANJI 押下 (VK_F3/F4)
    PhysicalImeKey,
    /// awase エンジン内部の判断 (Engine から SetOpen 要求等)
    Command,
}

/// Observation のソース (外部観測の種類)。
///
/// `serde::Serialize`/`Deserialize` は ADR-082「第一歩」2. の
/// `state::ime_actuation::DriftCorrectionFixture`（`FeedbackPolicy::Read.source`
/// を JSON フィクスチャとして保存・復元するため）が必要とする。全 variant が
/// データを持たないため機械的に導出可能で、既存の意味論には影響しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// GJI プロセス I/O 活動からの input_mode 推定。
    ///
    /// Blacklist (Imm32Unavailable) アプリではフォーカス後の GJI I/O だけが
    /// 「IME が実際に変換動作をしている = 英数モードではない」ことの真正の
    /// 外部証拠になる（IMM query はスキップ、idle-conv-check は TsfNative 限定）。
    /// `ConvBitsInference` と同じく「何を観測したか」を正直に命名する。
    /// I/O 活動は間接観測のため confidence は `Medium`、方向は
    /// `ObservedEisu → AssumedRomaji` の一方通行のみ（逆方向の推定はしない）。
    GjiIoInference,
    /// conversion mode ビット（`ImmGetConversionStatus` 由来）からの IME open 状態の推定。
    ///
    /// `ConvBitsInference` は input_mode 専用（`PerSourceObservations` には記録されず、
    /// `derive_any()`/`most_recent_trusted()` からは常に不可視）であり、open/close の
    /// 観測としては扱わない設計になっている。しかし `classify_conv_transition` の
    /// `KatakanaShadowOff`/`NativeToggleShadowOff`（shadow=OFF 中に NATIVE/KATAKANA conv
    /// を観測）は、conv ビットから「OS 側 IME はまだ open らしい」という open 状態の
    /// 推測でもある。かつてはこれを `UserImeSetIntent{Command}` として偽装し
    /// `desired_open` を直接書き換えていたため、ユーザーが明示的に OFF にした直後でも
    /// エンジンが勝手に ON に戻る再発バグを起こした（2026-07-08, BUG-19 再発）。
    ///
    /// この source は `PlatformState::report_conv_open_inference()` 専用。conv 由来の
    /// 間接推測のため confidence は `Medium` を上限とし、`desired_open` は変更しない —
    /// 実際に補正が必要かどうかの判断は `check_drift_correction()` に委ねる
    /// （明示意図が無い間、この source 単独では drift correction を発火させない）。
    ConvOpenInference,
    /// TSF observer 由来
    Tsf,
    /// per-HWND IME キャッシュからの復元
    HwndCache,
    /// フォーカス変更後の ImmCross 非同期プローブ
    ///
    /// Qt/LINE 等の ImmCross アプリで、フォーカス直後に `GetGUIThreadInfo.hwndFocus`
    /// （子 hwnd）の IMM32 状態を `read_ime_state_full_async` で読む高信頼ソース。
    /// `FocusProbe` が（同じく `hwndFocus`、真の top-level ウィンドウとは限らない
    /// ——BUG-91 参照）IMC を読む（Low）のと対になる。
    ImmCrossProbe,
    /// 観測が一切ない状態（cache miss 等）での安全デフォルトの推測。
    ///
    /// 実際の外部観測ではなく awase 側のポリシー的な best-guess のため、
    /// 必ず `ObservationConfidence::Low` で record すること。`derive_any()` の
    /// Medium+ 多数決には参加しないが、他に観測が一切ない場合の
    /// `effective_open()` フォールバックとしてのみ使われる。真の観測（Lowでも）が
    /// 後から届けば、鮮度・信頼度が同等以上のため上書きされる。
    HeuristicDefault,
}

/// `ObservationSource` が「外部 IME 状態への書き込み（actuation）の根拠になれるか」
/// を表す属性。ADR-087 §3 案C / §5 Phase 2' の `authority()`。
///
/// `BeliefOnly` の観測は `effective_open()`（engine の内部挙動決定）には使えるが、
/// `issue_open_warrant()`（actuation の根拠、`open_warrant.rs`）の Step 3 には
/// 使えない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationAuthority {
    /// actuation の根拠になれる（直接 API 読み取り相当）。
    Actuating,
    /// belief（engine の内部挙動決定）にのみ使える。actuation の根拠にはならない。
    BeliefOnly,
}

impl ObservationSource {
    /// この観測ソースが actuation（外部 IME 状態への書き込み）の根拠になれるか。
    ///
    /// ADR-087 §3 案C。`ConvOpenInference`（conv ビットからの間接推測、§1.3 参照:
    /// BUG-26 と ADR-087 発端バグは同じ conv 値で実際の IME 状態が正反対であり、
    /// conv ビットには actuation の根拠となる情報が無い）、`HeuristicDefault`
    /// （観測ゼロの安全デフォルト）、`HwndCache`（キャッシュ復元、実観測ではない）、
    /// `FocusProbe`（BUG-33: belief の自己確認が書き戻される経路があり、
    /// `.claude/rules/ime-belief-architecture.md` の「観測を実際の判断材料にしない」
    /// 原則に照らして actuation には使わない）は `BeliefOnly`。
    ///
    /// `ConvBitsInference`/`GjiIoInference` は input_mode 専用ソースで open/close の
    /// 観測としては記録されない（`PerSourceObservations::get/set` が None/no-op を
    /// 返す）ため、この関数の呼び出し元からは到達しない。`ObservationSource` 全体で
    /// 定義する都合上、網羅性のため `BeliefOnly`（安全側デフォルト）を割り当てる。
    #[must_use]
    pub const fn authority(self) -> ObservationAuthority {
        match self {
            Self::ImmGetOpenStatus
            | Self::ImmCrossProbe
            | Self::ObserverPoll
            | Self::Gji
            | Self::Tsf => ObservationAuthority::Actuating,
            Self::ConvOpenInference
            | Self::HeuristicDefault
            | Self::HwndCache
            | Self::FocusProbe
            | Self::ConvBitsInference
            | Self::GjiIoInference => ObservationAuthority::BeliefOnly,
        }
    }
}

/// 観測の信頼度。reducer が profile 別に judge する際に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
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

/// 入力 chord の種別。
///
/// 旧 `CtrlHenkanImeOn`（IME ON 側 chord）は 2026-07-06 到達不能パス監査 B2 で撤去 —
/// production で構築されたことがなかった。ON 側は barrier を張るのではなく
/// 「chord 中の IME ON 要求で barrier を即時解除する」のが設計
/// （`ImeModel::reduce` の `ImeApplyRequested` arm 参照）。ON の apply は冪等のため
/// 連打フィルタも不要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ChordKind {
    /// Ctrl + 無変換 → IME OFF
    CtrlMuhenkanImeOff,
}

/// Apply 失敗の種別 (Step 7 で使う、Step 0 では定義のみ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
    /// 物理 IME キー / SyncKey による shadow toggle OFF→ON 直後、stale な
    /// `ObservedEisu` を先回りで訂正する。
    ///
    /// `PostSetOpenEisuReset`（Decision 経由の SetOpen(true) パス）と対になる救済。
    /// engine が `NotRomajiInput` で inactive の間は Decision 経由の SetOpen 自体が
    /// 発生しない（activation が mode に塞がれる循環）ため、ユーザー起点の
    /// IME-ON 経路にはこの専用 strategy で同じ訂正を配線する。
    /// 判定は `state::eisu_recovery::eisu_reset_on_ime_on` に集約。
    UserImeOnEisuReset,
    /// TurnOn 系キー（ひらがな/かな 等）受信時、IME は既に open で OFF→ON 遷移が
    /// 起きなかったため `UserImeOnEisuReset` が発火しないケース向けの stale
    /// `ObservedEisu` 訂正。
    ///
    /// 判定は `state::eisu_recovery::eisu_reset_on_turn_on_while_open` に集約。
    UserTurnOnEisuReset,
    /// 左Shift単独タップによる「IME-ON 半角英数」持続トグルの ON/OFF。
    ///
    /// トグルONで `ObservedEisu`（Engineを`Inactive(NotRomajiInput)`へ誘導し、
    /// `SetOpen`を発行させずに素通りさせる）、トグルOFFで `AssumedRomaji` 系へ
    /// 遷移させる。IME の open/close を一切変更しないため、
    /// `state::eisu_recovery` の「IME を ON にする経路」対応表・
    /// `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`
    /// の対象外（SetOpenを経由しないため、対になる eisu 救済の配線は不要）。
    /// 判定・発火箇所は `runtime/key_pipeline.rs::kp_stage_shift_conv_guard`。
    UserHalfWidthAlnumToggle,
}

/// `InputModeApplied` event における適用結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum InputModeApplyResult {
    /// 入力モードを変更した。
    Applied,
    /// ObservedEisu guard 等の条件でスキップした（モード変更なし）。
    Skipped,
}

/// `Runtime::on_ime_apply_complete`（IME open/close 適用の完了）が
/// 「なぜこの apply が起きたか」を申告する理由（ADR-086 §4 INV-18、Phase 3 item 2）。
///
/// `InputModeApplyStrategy` とは別の型——あちらは input_mode（ローマ字/かな等）の
/// **補正手段**専用であり、open/close 自体の適用理由を運ぶ経路ではない
/// （conv-mode 軸の `ConvMutationReason` と同型の役割分担、ADR-086 §5 Phase 3
/// item 2 の訂正経緯参照）。ログ・ジャーナルから「これは force による書き込みか、
/// 観測に基づく是正か、エンジンの通常の決定か」が一意に読めるようにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum OpenApplyReason {
    /// `Engine::on_input`/`on_timeout` の `Decision::SetOpen` エフェクトによる、
    /// 通常のキー入力駆動の適用（`executor.rs::execute_one`/`dispatch_ime_set_open`）。
    EngineDecision,
    /// IMM32 クロスプロセス制御が使えないアプリ（TsfNative 等）向けの、
    /// `force_policy` によらない applied スロットル付き強制 ON
    /// （`apply_force_on_for_imm_broken` の非 force 分岐、既存挙動）。
    ImmBrokenForceOn,
    /// 未知 Imm32Unavailable アプリで IME 検出が連続失敗したときの一時 force-ON
    /// （`try_force_on_bootstrap`）。
    Bootstrap,
    /// 観測値（conv/IMC 読み取り）と belief の乖離を検出しての是正
    /// （`ir_apply_drift_correction`、`kp_apply_conv_engine_sync` の
    /// `EngineSync::DirectInput` 分岐）。
    DriftCorrection,
    /// Shadow IME belief のトグル（`kp_stage_shadow_ime_toggle`）に伴う適用。
    ShadowToggle,
}

/// IME 状態モデルへの全 event。
///
/// 時刻情報は `ImeEventEnvelope::time` に集約する (event 内に重複させない)。
///
/// `serde::Serialize` は ADR-082「決定 1」（`journal.rs::JournalEntry::ImeEvent` の
/// 構造化）が必要とする。全フィールドが `Serialize` 対応のプレーンな値のみで
/// 構成されるため機械的に導出可能。書き出し専用のため `Deserialize` は導出しない
/// （`state::ime_actuation::ActuationRecord` と同じ方針）。
#[derive(Debug, Clone, serde::Serialize)]
pub enum ImeEvent {
    /// ユーザー/awase が IME を toggle したい意図
    UserImeToggleIntent { source: UserIntentSource },

    /// ユーザー/awase が IME を ON/OFF に設定したい意図
    UserImeSetIntent {
        target: bool,
        source: UserIntentSource,
    },

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

    /// Engine の active/inactive 遷移が対称性のために自動発行した `SetOpen` の echo
    /// （`awase::engine::decision::SetOpenOrigin::ActivationSync`）を反映する。
    ///
    /// `UserImeSetIntent` と違い `last_intent` を設定しない。この SetOpen は
    /// `ctx.ime_on`（観測駆動で変化しうる）を Engine がそのまま追認しただけで、
    /// ユーザーが今このキーで ON/OFF を選んだわけではない。`last_intent` を
    /// 設定すると、以後の drift correction がこの echo を「ユーザーの本物の意図」
    /// として扱ってしまい、ユーザーが明示的に IME を OFF にした直後でも Engine が
    /// 勝手に ON へ戻る再発を引き起こす（2026-08-04、`docs/known-bugs.md` 参照）。
    EngineActivationSync { target: bool },

    /// OS への適用を開始した。
    ///
    /// `ctrl_held` は dispatch 時点で Ctrl が押下されていたか。reducer が
    /// 「IME OFF 要求 + Ctrl 押下中 → CtrlImeChord barrier を立てる」判断に使う。
    ImeApplyRequested {
        target: bool,
        generation: super::ApplyGeneration,
        ctrl_held: bool,
    },

    /// OS への適用が成功した (async 完了時、generation 照合必須)
    ImeApplySucceeded {
        target: bool,
        generation: super::ApplyGeneration,
    },

    /// OS への適用が失敗した
    ImeApplyFailed {
        target: bool,
        generation: super::ApplyGeneration,
        error: ApplyError,
    },

    /// 外部観測が値を報告した (desired を直接書き換えない)。
    ///
    /// payload の [`AnyObservation`](crate::state::evidence::AnyObservation) は
    /// **evidence ごとの witness を通してしか構築できない**（ADR-089 §2.2、
    /// INV-40）。「実際には probe していないのに `FocusProbe` を名乗る」形の
    /// 観測偽装は、witness（`&AcceptedObservation` 等）を用意できないため
    /// 構築段階で止まる。confidence も evidence 型が固定するので、呼び出し元は
    /// 選べない。
    ObserverReported(crate::state::evidence::AnyObservation),

    /// フォーカスが変わった
    FocusChanged {
        from: Option<HwndId>,
        to: HwndId,
        profile: ImePolicyProfile,
        /// インクリメント後のフォーカスエポック。
        /// reducer が `ObservationStore::current_fence`（`clear_on_focus_change`
        /// 経由）を更新するために使う。
        focus_epoch: crate::state::probe_admission::FocusEpoch,
    },

    /// 同一プロセス内でフォーカス hwnd だけが変わった（ADR-106 決定3）。
    ///
    /// `FocusChanged`（プロセス変更、epoch インクリメント + 観測プールクリア）とは
    /// 別のイベント。`focus_epoch` はプロセス変更でのみ進むため、`AppKind`
    /// (`TsfNative`⇔`Uwp` 等) 往復のような同一プロセス内のウィンドウ切り替えは
    /// `FocusChanged` を経由しない。この場合でも `ImmLikeTicket::admit()` が照合する
    /// 生の hwnd（`platform.focus.current.hwnd`）は毎 tick 追従するため、reducer 側の
    /// `ObservationStore::current_fence`（`update_focus_window` 経由）もこのイベントで
    /// 追従させないと、
    /// `derive_any()` の `is_identity_ok` が古い hwnd と比較し続け、以後の
    /// `ImmCrossProbe`/`FocusProbe` 観測を次のプロセス変更まで恒久的に拒否する
    /// （code review 2026-08-26 で発見された退行）。
    FocusHwndUpdated { hwnd: HwndId },

    // 旧 ChordStarted は 2026-07-06 到達不能パス監査 B2 で撤去 — production の
    // dispatch サイトがなく（chord 開始は ImeApplyRequested { target:false,
    // ctrl_held:true } の内部で行われる）、golden テストだけが生かしていた。
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
    /// 上書きする（ON/OFF の `derive_any()` と同じ考え方: Low confidence だけでは
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
        generation: super::ApplyGeneration,
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

    // ── ObservationSource::authority()（ADR-087 §3 案C） ──

    #[test]
    fn conv_open_inference_is_belief_only() {
        // BUG-26 と ADR-087 発端バグ（mise→くした）は同じ conv 値で
        // 実際の IME 状態が正反対だった。conv ビットには actuation の
        // 根拠となる情報が無いため BeliefOnly。
        assert_eq!(
            ObservationSource::ConvOpenInference.authority(),
            ObservationAuthority::BeliefOnly
        );
    }

    #[test]
    fn heuristic_default_and_hwnd_cache_and_focus_probe_are_belief_only() {
        assert_eq!(
            ObservationSource::HeuristicDefault.authority(),
            ObservationAuthority::BeliefOnly
        );
        assert_eq!(
            ObservationSource::HwndCache.authority(),
            ObservationAuthority::BeliefOnly
        );
        // BUG-33: belief の自己確認が観測として書き戻される経路があるため。
        assert_eq!(
            ObservationSource::FocusProbe.authority(),
            ObservationAuthority::BeliefOnly
        );
    }

    #[test]
    fn direct_read_sources_are_actuating() {
        assert_eq!(
            ObservationSource::ImmGetOpenStatus.authority(),
            ObservationAuthority::Actuating
        );
        assert_eq!(
            ObservationSource::ImmCrossProbe.authority(),
            ObservationAuthority::Actuating
        );
        assert_eq!(
            ObservationSource::ObserverPoll.authority(),
            ObservationAuthority::Actuating
        );
    }

    /// `Gji`/`Tsf` は型としては `Actuating` だが、production の open 観測
    /// （`ObserverReported` の dispatch 元）を grep すると実際には一度も
    /// 書かれない（ADR-087 §7 round3 Opus S6）。この事実は型テストでは
    /// 検知できないため、コメントとして残す——将来 `Gji`/`Tsf` 由来の
    /// open 観測を実際に record する経路を追加する場合、この前提が
    /// 変わることを意識すること。
    #[test]
    fn gji_and_tsf_are_actuating_by_type_though_unused_in_practice() {
        assert_eq!(
            ObservationSource::Gji.authority(),
            ObservationAuthority::Actuating
        );
        assert_eq!(
            ObservationSource::Tsf.authority(),
            ObservationAuthority::Actuating
        );
    }

    #[test]
    fn input_mode_only_sources_default_to_belief_only() {
        // ConvBitsInference/GjiIoInference は open/close 観測としては
        // 記録されないため、この分類自体は実質到達しない防御的デフォルト。
        assert_eq!(
            ObservationSource::ConvBitsInference.authority(),
            ObservationAuthority::BeliefOnly
        );
        assert_eq!(
            ObservationSource::GjiIoInference.authority(),
            ObservationAuthority::BeliefOnly
        );
    }
}
