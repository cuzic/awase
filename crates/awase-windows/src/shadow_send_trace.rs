//! [ADR-159](../../../docs/adr/159-existing-io-boundary-inventory.md) 段階2
//! （シャドー実行）向けの最小実装（`158-implementation-tasks.md` TF2）。
//!
//! # 何を解いているか
//!
//! [`crate::win32::send_input_safe`]（`SendInput`）と
//! [`crate::imm::send_ime_control`]（`WM_IME_CONTROL`）は、既に
//! `tracing::debug!("[ime-io] ...")` で actuation の発行を記録しているが、
//! 送信内容（VK列・cmd値・lparam値）を構造化された単一のタグ（`[shadow-send]`）
//! で記録してはいなかった。本モジュールは同じ2チョークポイントの**既存の
//! 条件分岐をそのまま再利用し**（新しい条件を増やさない、
//! `158-implementation-tasks.md` TF2「最小限(1条件分岐)」の要件）、実際に
//! 送信する内容を`tracing::debug!`で記録する。
//!
//! # スコープを意図的に絞った点（`/code-review`複数系統の指摘を反映、2026-09-10）
//!
//! 当初案は`Mutex<VecDeque<ShadowSendRecord>>`によるプロセス内リングバッファを
//! 持ち、`probe_actuation_fence`/`conv_mutation`と「同型のグローバルstate
//! パターン」と説明していた。しかしレビューで次の2点が判明し撤回した:
//!
//! 1. `probe_actuation_fence`/`conv_mutation`は実際にはロックフリーな
//!    `AtomicU64`単調カウンタであり、`Mutex`を使う本モジュールとは異なる
//!    パターンだった——「同型」という説明自体が誤りで、実際には新規の
//!    重い機構（ロック・ヒープ確保・キュー操作）をactuationのホットパスに
//!    追加していた。
//! 2. そのバッファを読む`snapshot()`はテスト以外に呼び出し元が無く、
//!    TF2の検証方法（実機セッションでこのログが1件以上出力されること）は
//!    バッファを一切使わない——蓄積した内容を消費する設計（ADR-159段階2の
//!    本体、`journal.rs`のタクソノミーへ合流させるか等）が定まっていない
//!    段階で、使われないストレージ層だけを先に作るのは時期尚早だった。
//!
//! そのため現在の実装は`tracing::debug!`1行のみで、蓄積・保持を行わない。
//! 将来、実際の消費者（TH1/TH4の記録トレース再生基盤）を設計する段階で、
//! 必要なら`journal.rs`の`JournalLane`/`LaneKind::Actuation`（既存の
//! bounded-buffer機構）へ合流させることを検討する——新規の第三のバッファ
//! 実装は追加しない。
//!
//! # ログレベル
//!
//! 同じチョークポイントの既存診断ログ（`[ime-io] ...`）はいずれも`debug!`
//! であり、本モジュールもそれに揃える（`info!`は既定フィルタで常時出力され、
//! actuationのたびに全スレッド共有のログライタを介した書き込みが発生する
//! ため採用しない）。
//!
//! # 検証（TF2「残タスク」の検証方法）
//!
//! 実機セッションで`RUST_LOG=debug`のログにこの`[shadow-send]`行が1件以上
//! 出力されることを確認する（`dragonflyg4`実機、2026-09-10確認済み）。

/// [`crate::win32::send_input_safe`]の既存の actuation marker 分岐から呼ぶ。
pub(crate) fn record_send_input(kind: &'static str, vk: &[u16], issue_us: u64) {
    tracing::debug!("[shadow-send] channel=SendInput kind={kind} vk={vk:02X?} issue_us={issue_us}");
}

/// [`crate::imm::send_ime_control`]の既存の actuation 判定分岐から呼ぶ。
/// `lparam`は`IMC_SETOPENSTATUS`ならopen/closeの真偽値、`IMC_SETCONVERSIONMODE`
/// なら新しいconvモードのビット値——`cmd`だけでは方向・値が分からないため必須。
pub(crate) fn record_ime_control(cmd: usize, lparam: isize, issue_us: u64) {
    tracing::debug!(
        "[shadow-send] channel=ImeControl cmd=0x{cmd:04X} lparam={lparam} issue_us={issue_us}"
    );
}
