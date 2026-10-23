//! Platform abstraction traits and types used by the platform crate
//! (platform runtime, TSF composition hooks, IME actuation outcomes).
//!
//! ADR-030 の `CompositionOutput` トレイト（呼び出し元ゼロ）と、Linux/macOS スタブ
//! crate は 2026-10-22 に撤去した（SF-06・MB-07）。実装は Windows（`awase-windows`）だけ。

// ─── PlatformRuntime Trait ──────────────────────────────────

use std::time::Duration;

use crate::types::{KeyAction, RawKeyEvent};

/// `apply_ime_open` の実行結果。
#[derive(
    strum::IntoStaticStr, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum ImeOpenOutcome {
    /// 実 `SendInput`（VK送信）を伴って確実に設定できた（`GjiDirectStrategy`/
    /// `MsImeDirectStrategy`）。
    Applied,
    /// `ImmSetOpenStatus`（クロスプロセスIMM32 API）のみで設定できた。VK は
    /// 一切送っていない（ADR-167）。`ImmCrossProcessStrategy`（`Standard`
    /// プロファイル限定）専用。旧実装ではこのケースも`Applied`に潰していたが、
    /// 「`Applied` == 実SendInputを伴う」という
    /// [`should_send_accompanying_warmup`] の前提が`ImmCrossProcessStrategy`
    /// には成立しないため、この専用variantに分離した（ADR-149の随伴warmup
    /// ゲートがStandardプロファイル全体を無条件例外にしていたことで、
    /// `ImmCrossProcessStrategy`が`Failed`を返し`GjiDirectStrategy`へ
    /// フォールスルーした場合に随伴warmupが実送信の直後へ重複する欠陥が
    /// あった）。
    AppliedWithoutSendInput,
    /// shadow が既に目標状態のためスキップ
    AlreadyMatched,
    /// 設定に失敗（非日本語環境など）
    Failed,
    /// トグル操作が unsafe のため送信しなかった（shadow 信頼度不足・focus 直後等）
    ///
    /// F21/F22 や IMM32 SetOpen は冪等だが VK_KANJI は冪等でない。
    /// shadow が stale な状態でトグルすると意図と逆方向に反転する恐れがある。
    /// このケースでは apply は行われていないため applied_snapshot / state は更新しない。
    UnsafeToToggle,
    /// このフォーカス先は awase が IME actuation を所有しないため、
    /// 機構を一切試行しなかった（issue #136 / BUG-90、`AppImeProfile::InputRelay`）。
    /// `UnsafeToToggle` と同じく「送っていない」ので applied / belief を書かない。
    NotOwned,
    /// `issue_open_warrant()` が授権を発行しなかったため、機構を一切試行
    /// しなかった（ADR-090 §2.A A-2、根拠軸）。`NotOwned`（InputRelay、
    /// プロファイル所有権軸）とは別の理由なので別 variant にする——同じ
    /// 「送っていない」結果でも診断ログ・journal を読む側が「なぜ」を
    /// 区別できなくなる（ADR-106決定5が戒める「別目的の値を1つの機構に
    /// 混ぜる」の逆、ここでは逆に「別目的の結果を1つのvariantに混ぜない」）。
    /// `UnsafeToToggle`/`NotOwned`と同じく送っていないため applied / belief
    /// を書かない。
    Unwarranted,
}

impl ImeOpenOutcome {
    /// この outcome が「実際に何らかの機構で open 軸へ書き込んだ」ことを
    /// 意味するか（`Applied`/`AppliedWithoutSendInput`の2つ、
    /// ADR-167）。網羅 `match` で書くことで、将来 variant を追加した際に
    /// このヘルパーの呼び出し元全てがコンパイルエラーで追随を強制される
    /// （非網羅な `matches!` の書き直しを1箇所に集約する狙い）。
    #[must_use]
    pub const fn wrote_open_state(self) -> bool {
        match self {
            Self::Applied | Self::AppliedWithoutSendInput => true,
            Self::AlreadyMatched
            | Self::Failed
            | Self::UnsafeToToggle
            | Self::NotOwned
            | Self::Unwarranted => false,
        }
    }
}

/// プラットフォーム固有の副作用実行インターフェース。
///
/// `DecisionExecutor` がこのトレイトを通じて OS 操作を行う。
/// 実装は Windows（`awase-windows` の `WindowsPlatform`）だけ。
pub trait PlatformRuntime {
    // ── キー出力 ──

    /// `KeyAction` のスライスを順に実行する
    fn send_keys(&mut self, actions: &[KeyAction]);

    /// 元のキーイベントを再注入する（IME OFF 時の遅延キー再生用）
    fn reinject_key(&mut self, event: &RawKeyEvent);

    // ── タイマー ──

    /// 指定 ID のタイマーを開始する
    fn set_timer(&mut self, id: usize, duration: Duration);

    /// 指定 ID のタイマーを停止する
    fn kill_timer(&mut self, id: usize);

    // ── IME 制御 ──

    /// IME の ON/OFF を設定する。成功時 true を返す。
    ///
    /// このメソッド自体は `awase-windows` の `runtime/ime_refresh.rs`
    /// （focus change 強制 OFF・drift correction の ImmCross 経路）で実際に
    /// 呼ばれている（2026-08-10、ADR-087 §5 Phase 3 item14 実 actuation 入口棚卸しで判明。
    /// ADR-158 TA1でこのメソッドをラップするだけだった`apply_ime_open`デフォルト実装を
    /// 削除したため、この段落の「以下のapply_ime_open」という言及も併せて削除した）。
    fn set_ime_open(&mut self, open: bool) -> bool;

    /// IME 状態キャッシュの非同期リフレッシュを要求する
    fn post_ime_refresh(&mut self);

    // ── トレイ ──

    /// エンジン有効/無効に応じてトレイアイコンを更新する
    fn update_tray(&mut self, enabled: bool);

    /// バルーン通知を表示する
    fn show_balloon(&mut self, title: &str, message: &str);

    /// 配列名をトレイに表示する
    fn set_tray_layout_name(&mut self, name: &str);
}

/// TSF / IMM composition 特有の platform フック（Windows 固有の意味論）。
///
/// `PlatformRuntime` のコア（キー出力・タイマー・トレイ・IME open/close 制御）とは分離し、
/// TSF composition warmup 特有の判定・フック（warm/cold 判定、passthrough / reinject 時の
/// composition 状態更新等）をまとめる。本トレイトは全メソッドがデフォルト実装（no-op /
/// `false` / `u64::MAX`）を持つ。
///
/// Windows では `WindowsPlatform` が本トレイトを override し、`tsf` サブシステムに委譲する。
pub trait TsfComposition {
    /// 最後のキー出力からの経過時間 (ms) を返す。一度も送信していなければ `u64::MAX`。
    fn output_in_flight_ms(&self) -> u64 {
        u64::MAX
    }

    /// TSF composition context が warm 状態かどうかを返す。
    fn is_composition_warm(&self) -> bool {
        false
    }

    /// IME apply 完了後の platform 状態更新フック。
    ///
    /// `applied_snapshot` 更新・latch・mark_cold・eager warmup を platform 内で処理する。
    /// executor は outcome を受け取ったら必ずこのメソッドを呼ぶこと。
    fn on_ime_applied(&mut self, _open: bool, _outcome: ImeOpenOutcome) {}

    /// キー再注入時の composition 状態更新フック。
    ///
    /// confirm キー KeyDown の reinject 時に cold 化する（`VK_IME_ON` の eager warmup は送らない）。
    fn on_reinject_key(&mut self, _vk: crate::types::VkCode, _is_keydown: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrote_open_state_distinguishes_real_send_from_immcross_only() {
        // ADR-167: 「何か書き込んだか」（wrote_open_state）は3つとも true、
        // 「実SendInputを伴ったか」（should_send_accompanying_warmupの否定）は
        // AppliedWithoutSendInputだけ異なる、という非対称性を固定する。
        assert!(ImeOpenOutcome::Applied.wrote_open_state());
        assert!(ImeOpenOutcome::AppliedWithoutSendInput.wrote_open_state());
        assert!(!ImeOpenOutcome::AlreadyMatched.wrote_open_state());
        assert!(!ImeOpenOutcome::Failed.wrote_open_state());
        assert!(!ImeOpenOutcome::UnsafeToToggle.wrote_open_state());
        assert!(!ImeOpenOutcome::NotOwned.wrote_open_state());
    }
}
