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

use crate::focus::class_names::AppImeProfile;

/// フォーカス中アプリの分類情報（フォーカス変更時に更新される長期観測）。
#[derive(Clone, Copy)]
pub(crate) struct FocusFacts<'a> {
    /// フォーカスウィンドウのクラス名（ログ用）
    pub class_name: &'a str,
    /// フォーカス中アプリの IME 制御プロファイル
    pub profile: AppImeProfile,
}

/// OS から直接観測した揮発性状態（tick 境界でアトミックをロードしてスナップショット化）。
///
/// 判断層はこの型を通じて観測値を受け取ること。
/// `crate::tsf::observer::tsf_obs()` を判断コードから直接呼んではいけない。
/// スナップショット化が必要でない live 読み取り（`output/` のシーケンスカウンタ等）は
/// `tsf_obs()` を直接使う別カテゴリであり、この型の対象外。
#[derive(Clone, Copy, Default)]
pub(crate) struct ObservedState {
    /// TSF/GJI: `GoogleJapaneseInputCandidateWindow` が現在表示中かどうか。
    /// EVENT_OBJECT_SHOW/HIDE で更新されるアトミック値のスナップショット。
    pub candidate_visible: bool,
    /// GJI プロセスの最終 I/O 変化時刻 (ms)。0 = 未観測。
    /// フォーカスプローブの grace 期間判定に使用する（将来の判断ロジック用）。
    #[allow(dead_code)]
    pub gji_last_io_ms: u64,
    /// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
    /// `GjiDirectStrategy` の `is_applicable` ゲートに使用する。
    pub gji_monitor_ok: bool,
    /// GJI candidate が SHOW になってから次の `apply_ime_open` 完了まで `true`。
    /// `shadow=false` なのに candidate が表示された desync を `KanjiToggleStrategy` が検出するために使う。
    pub candidate_was_seen: bool,
}

impl ObservedState {
    /// 現時点の TSF/GJI 観測値を全フィールドに一括ロードして返す（tick 境界スナップショット）。
    ///
    /// 判断サイトはこのメソッドで 1 回スナップショットを取り、以降は `&ObservedState` を参照する。
    pub(crate) fn capture_now() -> Self {
        let obs = crate::tsf::observer::tsf_obs();
        Self {
            candidate_visible:  obs.gji_candidate_visible(),
            gji_last_io_ms:     obs.gji_last_io_ms(),
            gji_monitor_ok:     obs.gji_monitor_ok(),
            candidate_was_seen: crate::tsf::observer::candidate_was_seen(),
        }
    }
}

/// `apply_ime_open` が最後に OS に送ったコマンド値（制御ログ）。
///
/// 真の観測値ではない。`ImeModel.applied_open / applied_at_ms`（SSOT）から
/// 各 apply サイクルの先頭で pre-fetch されるスナップショット。
/// VK_KANJI がトグルキーであるため、重複送信を避けるために参照する。
#[derive(Clone, Copy)]
pub(crate) struct ControlLog {
    /// `apply_ime_open` が最後に OS に送ったコマンド値。
    pub shadow_on: bool,
    /// 最後の apply 完了時刻 (ms)。0 = 未確認（フォーカス変更後 / soft presync）。
    /// `applied_at_ms > 0` は「フォーカス変更後に実 apply が 1 回以上完了した」を示す。
    pub applied_at_ms: u64,
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
}
