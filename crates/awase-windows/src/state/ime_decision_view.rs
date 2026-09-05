//! IME 制御の判断サイトで使う統一ビュー型群。
//!
//! 各サブ構造体はデータの「時間的出所」を明示する：
//!
//! | 型 | 更新タイミング | 含む情報 |
//! |---|---|---|
//! | `FocusFacts` | フォーカス変更時 | アプリ分類（長期観測） |
//! | `ObservedState` | OS イベント / ポーリング | 揮発性 OS 観測値 |
//! | `ControlLog` | `apply_ime_open` 呼び出し時 | 最後に送ったコマンド値 |
//! | `ImeControlView` | `apply_ime_open` の tick 境界 | 上記3つのスナップショット |
//!
//! `ImeControlView` は `ImeObservationSnapshot` を完全に置き換える。

use awase::engine::InputModeState;

use crate::focus::class_names::AppImeProfile;
use crate::tsf::observer::TsfObservations;

/// フォーカス中アプリの分類情報（フォーカス変更時に更新される長期観測）。
#[derive(Clone, Copy)]
pub(crate) struct FocusFacts<'a> {
    /// フォーカスウィンドウのクラス名（ログ用）
    pub class_name: &'a str,
    /// フォーカス中アプリの IME 制御プロファイル
    pub profile: AppImeProfile,
    /// view 構築時点のフォーカス世代（`Output::ime_mode_focus_gen`）。
    ///
    /// ADR-086 INV-14 の同期 write（`ime_controller::apply_mechanism` の
    /// ROMAN 補完）が `ActuationTarget` を捕獲・照合するために使う
    /// （ADR-089 §6 Phase C item 12）。`ime_controller.rs` は Runtime/Output の
    /// 内部状態を直接読めないため、view のフィールドとして運ぶ。
    pub focus_gen: u32,
}

/// OS から直接観測した揮発性状態（tick 境界でアトミックをロードしてスナップショット化）。
///
/// 判断層はこの型を通じて観測値を受け取ること。
/// `crate::tsf::observer::tsf_obs()` を判断コードから直接呼んではいけない。
/// スナップショット化が必要でない live 読み取り（`output/` のシーケンスカウンタ等）は
/// `tsf_obs()` を直接使う別カテゴリであり、この型の対象外。
#[derive(Clone, Copy)]
pub(crate) struct ObservedState {
    /// TSF/GJI: `GoogleJapaneseInputCandidateWindow` が現在表示中かどうか。
    /// EVENT_OBJECT_SHOW/HIDE で更新されるアトミック値のスナップショット。
    pub candidate_visible: bool,
    /// GJI プロセスの最終 I/O 変化時刻 (ms)。0 = 未観測。
    /// TSF gate の warmup 判定・GJI アイドル時間計算に使用する。
    pub gji_last_io_ms: u64,
    /// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
    /// `GjiDirectStrategy` の `is_applicable` ゲートに使用する。
    pub gji_monitor_ok: bool,
    /// GJI candidate が SHOW になってから次の `apply_ime_open` 完了まで `true`。
    /// `shadow=false` なのに candidate が表示された desync を `KanjiToggleStrategy` が検出するために使う。
    pub candidate_was_seen: bool,
    /// 現在使用中の IME 種別（`gji_monitor_ok` から派生）。
    /// warmup strategy 切り替え（`WM_IME_KIND_CHANGED`）に使用する。
    pub active_ime_kind: crate::tsf::observer::ActiveImeKind,
    /// （GJI/MS-IME 問わず）IME composition window が可視かどうか（ADR-117、issue #138 診断用）。
    /// MS-IME での信頼性は未検証——`crate::tsf::observer::TsfObservations::ime_composition_active`
    /// の doc コメント参照。
    pub composition_active: bool,
    /// `EVENT_OBJECT_IME_SHOW` の発火回数（ADR-117、診断用）。
    pub ime_show_seq: u32,
    /// `EVENT_OBJECT_IME_CHANGE` の発火回数（ADR-117、診断用）。
    pub ime_change_seq: u32,
}

impl Default for ObservedState {
    fn default() -> Self {
        Self {
            candidate_visible: false,
            gji_last_io_ms: 0,
            gji_monitor_ok: false,
            candidate_was_seen: false,
            active_ime_kind: crate::tsf::observer::ActiveImeKind::MicrosoftIme,
            composition_active: false,
            ime_show_seq: 0,
            ime_change_seq: 0,
        }
    }
}

impl ObservedState {
    /// `TsfObservations` スナップショットから `ObservedState` を構築する。
    ///
    /// 呼び出し元は `crate::tsf::observer::tsf_obs()` で取得した参照を渡す。
    /// これにより `state/` レイヤーが `tsf::observer` を直接参照することなく
    /// スナップショット化を行える（レイヤー境界の維持）。
    ///
    /// 判断サイトはこのメソッドで 1 回スナップショットを取り、以降は `&ObservedState` を参照する。
    pub(crate) fn from_snapshot(snapshot: &TsfObservations) -> Self {
        Self {
            candidate_visible: snapshot.gji_candidate_visible(),
            gji_last_io_ms: snapshot.gji_last_io_ms(),
            gji_monitor_ok: snapshot.gji_monitor_ok(),
            candidate_was_seen: crate::tsf::observer::candidate_was_seen(),
            active_ime_kind: snapshot.active_ime_kind(),
            composition_active: snapshot.ime_composition_active(),
            ime_show_seq: snapshot.ime_show_seq(),
            ime_change_seq: snapshot.ime_change_seq(),
        }
    }
}

/// `apply_ime_open` が最後に OS に送ったコマンド値（制御ログ）。
///
/// 真の観測値ではない。`ImeModel.applied_open()`（SSOT、`AppliedImeState`）から
/// 各 apply サイクルの先頭で pre-fetch されるスナップショット。
/// VK_KANJI がトグルキーであるため、重複送信を避けるために参照する。
///
/// **`Option<bool>` であって `bool` ではないことが重要**（BUG-113 Blocker、
/// Opus 敵対的レビューで発見）: `None` は「まだ何もコマンドを送っていない、
/// または `applied` が `AppliedImeState::Unknown`（フォーカス変更直後等）」を
/// 表し、「確認済み OFF」（`Some(false)`）とは明確に区別する。`bool` に
/// 潰して `unwrap_or(false)` してしまうと、`None`（証拠なし）を
/// 「確認済み OFF」と誤認してしまい、OFF 方向の already-matched スキップ
/// 判定が正当な再送（drift correction・idle-conv-check の DirectInput
/// 回復等、意図的に shadow を無視して送る設計の経路）を無音で握り潰す
/// （`state/ime_model.rs::applied_open()` の doc コメントが警告する
/// ADR-098 決定1-b の罠と同型、docs/known-bugs.md BUG-113 参照）。
#[derive(Clone, Copy)]
pub(crate) struct ControlLog {
    /// `apply_ime_open` が最後に OS に送ったコマンド値。
    /// `Some(true)`=確認済み ON、`Some(false)`=確認済み OFF、`None`=未知
    /// （まだ根拠が無い——「送信すべき」側として扱うこと）。
    pub shadow_on: Option<bool>,
}

/// `apply_ime_open` / `ImeOpenStrategy` 用の統一スナップショットビュー。
///
/// 以前の `ImeObservationSnapshot` を置き換える型。
/// フォーカス分類・OS 観測値・制御ログをまとめて1つの構造体として扱うことで、
/// 各フィールドの出所が型構造から自明になる。
///
/// ## アーキテクチャ制約
/// このビューを利用するコードは観測値を自ら読んではいけない。
/// すべての観測値はこの型を通じて受け取ること。
/// `crate::tsf::observer::tsf_obs()` の直接呼び出し禁止（スナップショット経由で受け取ること）。
#[derive(Clone, Copy)]
pub(crate) struct ImeControlView<'a> {
    /// フォーカス分類（長期観測）
    pub focus: FocusFacts<'a>,
    /// OS 揮発性観測値（tick 境界スナップショット）
    pub observed: ObservedState,
    /// 制御ログ（最後に送ったコマンド値）
    pub control: ControlLog,
    /// ImeModel の入力方式 belief（`apply` 戦略がかな/ローマ字を区別するために使う）。
    /// `build_ime_control_view` 時点では `Unknown`。呼び出し元が `input_mode()` で上書きすること。
    pub belief_input_mode: InputModeState,
}
