//! [ADR-159](../../../docs/adr/159-existing-io-boundary-inventory.md) 段階2
//! （シャドー実行）向けの最小実装（`158-implementation-tasks.md` TF2）。
//!
//! # 何を解いているか
//!
//! [`crate::win32::send_input_safe`]（`SendInput`）と
//! [`crate::imm::send_ime_control`]（`WM_IME_CONTROL`）は、既に
//! `tracing::debug!("[ime-io] ...")` で actuation の発行を記録しているが、
//! これは人間が読む診断ログであり、後から構造化データとして取り出して
//! 別実装との送信列差分を取る（ADR-159 段階2が求める「シャドー実行の差分
//! 記録」）用途には使えない。本モジュールは同じ2チョークポイントの
//! **既存の条件分岐をそのまま再利用し**（新しい条件を増やさない、
//! `158-implementation-tasks.md` TF2「最小限(1条件分岐)」の要件）、
//! 実際に送信した内容をプロセス内リングバッファへ構造化して保持する。
//!
//! # なぜグローバルな static か（`probe_actuation_fence`/`conv_mutation`と同型）
//!
//! [`crate::win32::send_input_safe`]/[`crate::imm::send_ime_control`]は
//! `self`を取らない`pub(crate) fn`で、`Journal`（`PlatformState`が所有）へ
//! 直接書き込む経路を持たない。[`crate::probe_actuation_fence`]が同じ制約に
//! 対して採った解法（物理syscall境界にグローバルなstateを置く）をそのまま
//! 踏襲する。`Journal`への統合（段階1のタクソノミーへ合流させるか）は
//! 別途の設計判断とし、本実装はまず「送信内容が実際に記録される」ことだけを
//! 満たす。
//!
//! # 検証（TF2「残タスク」の検証方法）
//!
//! 実機セッションでこのモジュールの `tracing::info!("[shadow-send] ...")`
//! が1件以上出力されることを確認する。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// リングバッファの上限件数。`journal.rs`の実装同様、無限成長を避けるための
/// 固定容量（ここでは差分検証用の直近サンプルが取れれば足りるため小さめ）。
const CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShadowSendChannel {
    /// [`crate::win32::send_input_safe`]（`SendInput`）経由。
    SendInput,
    /// [`crate::imm::send_ime_control`]（`WM_IME_CONTROL`）経由。
    ImeControl,
}

/// 実際に送信した内容1回分の記録。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShadowSendRecord {
    pub channel: ShadowSendChannel,
    /// `SendInput`側は`ime_actuation_marker_kind`が返す種別
    /// （`"kanji_marker"`/`"tsf_marker_warmup"`）。`WM_IME_CONTROL`側は常に
    /// `"actuation"`（probe系cmdはこの記録の対象外、既存の条件と同一）。
    pub kind: &'static str,
    /// `SendInput`のみ非空（`actuation_vks`と同じ重複排除済みVK列）。
    pub vk: Vec<u16>,
    /// `WM_IME_CONTROL`のみ`Some`（`IMC_SETOPENSTATUS`/`IMC_SETCONVERSIONMODE`
    /// のいずれか、probe系cmdはそもそも記録しないため常にactuation cmd）。
    /// `send_ime_control`の引数型（`usize`、`WPARAM`へそのまま渡す）に合わせる。
    pub cmd: Option<usize>,
    pub issue_us: u64,
}

fn buffer() -> &'static Mutex<VecDeque<ShadowSendRecord>> {
    static BUFFER: OnceLock<Mutex<VecDeque<ShadowSendRecord>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)))
}

/// [`crate::win32::send_input_safe`]の既存の actuation marker 分岐から呼ぶ。
pub(crate) fn record_send_input(kind: &'static str, vk: &[u16], issue_us: u64) {
    push(ShadowSendRecord {
        channel: ShadowSendChannel::SendInput,
        kind,
        vk: vk.to_vec(),
        cmd: None,
        issue_us,
    });
}

/// [`crate::imm::send_ime_control`]の既存の actuation 判定分岐から呼ぶ。
pub(crate) fn record_ime_control(cmd: usize, issue_us: u64) {
    push(ShadowSendRecord {
        channel: ShadowSendChannel::ImeControl,
        kind: "actuation",
        vk: Vec::new(),
        cmd: Some(cmd),
        issue_us,
    });
}

fn push(record: ShadowSendRecord) {
    // 実機での検証方法（TF2「残タスク」）: この行が1件以上出力されることを
    // RUST_LOG=debug のセッションログで確認する。
    tracing::info!(
        "[shadow-send] channel={:?} kind={} vk={:02X?} cmd={:?} issue_us={}",
        record.channel,
        record.kind,
        record.vk,
        record.cmd,
        record.issue_us,
    );
    // Mutex poisoning は致命的ではない（診断用の補助データであり、他の
    // actuation経路をブロックしてはならない）ため、`send_health`と同様
    // poison時はロック内容をそのまま引き継いで継続する。
    let mut buf = buffer()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if buf.len() >= CAPACITY {
        buf.pop_front();
    }
    buf.push_back(record);
}

/// 現在保持しているレコードのスナップショットを返す（消費しない）。
/// 不具合報告への添付・テスト用。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn snapshot() -> Vec<ShadowSendRecord> {
    buffer()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 実装レビュー指摘m4（`probe_actuation_fence`と同型の注意）: このバッファは
    // プロセス共有のstaticであり、同じテストバイナリ内の他のテストが並行して
    // record_*を呼びうる。したがって「自分が積んだレコードが`snapshot()`の
    // どこかに現れる」ことだけを検証し、末尾位置や他レコードの不在には
    // 依存しない（他テストの割り込みに対してflakyにならないため）。

    #[test]
    fn record_send_input_appends_a_retrievable_record_with_expected_fields() {
        record_send_input("tsf_marker_warmup", &[0xF0AB, 0xF0AC], 999_001);
        let found = snapshot().into_iter().find(|r| r.issue_us == 999_001);
        let record = found.expect("record_send_input で積んだレコードが見つかること");
        assert_eq!(record.channel, ShadowSendChannel::SendInput);
        assert_eq!(record.kind, "tsf_marker_warmup");
        assert_eq!(record.vk, vec![0xF0AB, 0xF0AC]);
        assert_eq!(record.cmd, None);
    }

    #[test]
    fn record_ime_control_appends_a_retrievable_record_with_expected_fields() {
        record_ime_control(0x0016, 999_002);
        let found = snapshot().into_iter().find(|r| r.issue_us == 999_002);
        let record = found.expect("record_ime_control で積んだレコードが見つかること");
        assert_eq!(record.channel, ShadowSendChannel::ImeControl);
        assert_eq!(record.kind, "actuation");
        assert!(record.vk.is_empty());
        assert_eq!(record.cmd, Some(0x0016));
    }

    #[test]
    fn buffer_never_exceeds_capacity() {
        for i in 0..(CAPACITY as u64 + 10) {
            record_ime_control(0x1234, 2_000_000 + i);
        }
        assert!(snapshot().len() <= CAPACITY);
    }
}
