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
#[derive(Clone, Copy)]
pub(crate) struct ObservedState {
    /// TSF/GJI: `GoogleJapaneseInputCandidateWindow` が現在表示中かどうか。
    /// EVENT_OBJECT_SHOW/HIDE で更新されるアトミック値のスナップショット。
    pub candidate_visible: bool,
}

/// `apply_ime_open` が最後に OS に送ったコマンド値（制御ログ）。
///
/// 真の観測値ではない。`ImeBelief.ime_on`（SSOT）とは別物。
/// VK_KANJI がトグルキーであるため、重複送信を避けるために参照する。
#[derive(Clone, Copy)]
pub(crate) struct ControlLog {
    /// `apply_ime_open` が最後に OS に送ったコマンド値
    /// （`Output::last_applied_ime_on()` = `LastAppliedImeState::get_or(false)`）。
    pub shadow_on: bool,
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
/// `crate::tsf::observer::aggregator::*` / `TSF_OBS` への直接アクセス禁止。
#[derive(Clone, Copy)]
pub(crate) struct ImeControlView<'a> {
    /// フォーカス分類（長期観測）
    pub focus: FocusFacts<'a>,
    /// OS 揮発性観測値（tick 境界スナップショット）
    pub observed: ObservedState,
    /// 制御ログ（最後に送ったコマンド値）
    pub control: ControlLog,
}
