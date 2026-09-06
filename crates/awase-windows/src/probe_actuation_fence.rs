//! GJI IME actuation（`SendInput` の kanji_marker/tsf_marker_warmup、
//! `SendMessageTimeoutW` の actuation cmd）が発行されたことを記録する単調カウンタ
//! （ADR-140 Step1 確定設計・案D）。
//!
//! # なぜ `conv_mutation`（[`crate::conv_mutation`]）を流用しないか
//!
//! `conv_mutation::bump()` のゲート（`win32::input_may_mutate_conv` /
//! `imm::send_ime_control` の `IMC_SETCONVERSIONMODE` 判定）は open 専用 VK
//! （`VK_IME_ON`/`VK_IME_OFF`/`VK_KANJI`）や `IMC_SETOPENSTATUS` では増分しない
//! 仕様（`conv_mutation.rs` module doc）。BUG-113 の actuation はまさに GJI の
//! open 軸（`IME_KANJI_MARKER`）であり、既存フェンスはこの actuation を
//! 1 回も数えていない——「issue 時点を見ていない」以前に、このカウンタは
//! そもそもこの actuation を対象にしていなかった。
//!
//! # bump 地点（物理 syscall 境界2箇所、必ず syscall の前）
//!
//! 論理呼び出し箇所（`ime_controller.rs::apply` 等）を個別に bump する方式は
//! 採らない——未発見の呼び出し経路（ADR-140 確定事実5が「少なくとも3つ確認、
//! 全てとは限らない」と明記）が残っていると、issue #136 型の「1箇所塞いで
//! 別箇所に穴」を再演するため。代わりに OS に到達する物理境界そのもの
//! （唯一のチョークポイント）で bump する:
//!
//! - [`crate::win32::send_input_safe`]: ADR-140 Step0 診断ログの
//!   `ime_actuation_marker_kind` 判定と**同一の条件式**で bump する。
//! - [`crate::imm::send_ime_control`]: 同診断ログの `kind=actuation` 判定
//!   （`!matches!(cmd, IMC_GETOPENSTATUS | IMC_GETCONVERSIONMODE)`）と
//!   **同一の条件式**で bump する。
//!
//! 判定を診断ログと共有することで将来の乖離を防ぐ。`Ordering::Relaxed` で
//! 足りる: 単一ロケーションのカウンタで、これ経由で他のデータを publish
//! しないため（`conv_mutation` と同じ理屈）。**ただし精密には次の点で
//! `conv_mutation` と異なる**（`/code-review max`指摘）: `conv_mutation` の
//! 比較は常にメインスレッドから行われるのに対し、本フェンスは
//! `get_ime_conversion_mode_fenced_async` のチェックポイント2（issue(worker)）
//! でワーカースレッドから読む。単一アトミック変数には x86_64/ARM64 上で
//! 単一の全順序があるため観測される staleness は理論上も起きにくく、かつ
//! チェックポイント3（apply、メインスレッド）が第二の検出機会として残るため
//! 実害はないが、「`conv_mutation` と全く同じ理由で安全」という単純な類推は
//! 不正確——ワーカースレッドから読む点は `conv_mutation` に前例がない。
//!
//! **注意（実装レビュー指摘m3）**: 「物理境界が単一チョークポイントである」
//! という上記の主張が保証するのは *syscall の発行口が1箇所である* ことのみで、
//! *その syscall が actuation として bump 対象になるかどうか* は marker/VK の
//! 判定（`ime_actuation_marker_kind`）に依存する。例えば
//! `key_pipeline.rs` の shift-conv-guard が注入する `VK_DBE_HIRAGANA`
//! （`TSF_MARKER`、`VK_IME_ON`/`OFF`ではない）はこの判定に一致せず bump
//! しない——これは probe が読む conv ワード自体を変えうる書き込みだが、
//! `half_width_alnum.is_guard_pending()` による別の discard（apply_idle_conv_check
//! の (a)）で捕捉される前提のため実害はない。ただし本フェンス単体が
//! 「conv/open を変えうる全ての書き込みを bump する」ことまでは保証しない。
//!
//! # 比較点（probe 側の呼び出し元のみ、`ime::offload_unsafe` には絶対に置かない）
//!
//! 比較ロジック自体は [`crate::ime::get_ime_conversion_mode_fenced_async`]
//! に集約されている（ADR-140 Step1b、`/code-review max`指摘: Step1完了時点
//! では`kp_stage_idle_conv_check_inner`だけがフェンスされ、全く同型
//! （`spawn_local`/ループ→クロスプロセス conv 読み取り→`with_app`、
//! focus世代の照合のみ）の他の probe 経路——`output/probe_io.rs::
//! start_ms_ime_ready_poll`〈MS-IME BUG-13 confirm-then-transmitゲート〉・
//! `send_chrome_gji_reinit_and_poll`〈Chrome cold-reinit時のGJI確認〉・
//! `platform.rs`のFocusChange直後IMCヒント probe——がフェンス対象外のまま
//! 残っていた。この関数を共通の入口にすることで、新しい probe 経路を
//! 追加する開発者が「フェンスすべきかどうか」を毎回再検討しなくても、
//! この関数を使う限り自動的にフェンスされる)。
//!
//! `crate::ime::offload_unsafe` は probe/actuation 双方が通る共通ヘルパーの
//! ため、ここに比較を置くと「actuation が actuation を待つ」という、
//! 却下済みの対称ロック方式と同型の失敗モードを1階層上で再現する
//! ——`get_ime_conversion_mode_fenced_async`は意図的にこれを経由しない。
//! 将来さらに別の probe 経路を追加する場合も、比較は必ず
//! `get_ime_conversion_mode_fenced_async`（または同型の専用ヘルパー）
//! 経由で行い、共通ヘルパーには絶対に比較を置かないこと。
//!
//! # abandon カウンタ（決定I: resync 経路と通常経路を分けて数える）
//!
//! resync 経路（[`crate::focus_resync`] 由来）の abandon は「defer 中のキーが
//! `FOCUS_RESYNC_DEADLINE_MS` まで出てこない」という体感遅延に直結する一方、
//! 通常経路の abandon は「今回の idle-conv-check を1回諦めた」だけ
//! ——ユーザー体感コストが桁違いのため、合算した1つのカウンタだと前者に
//! 埋もれて後者の頻発を見逃す。starvation（probe が永久に不成立になり
//! idle-conv-check の本来の目的が静かに死ぬ事態）が起きていないかを実機
//! ソークで確認する（Step1 の完了条件）用に、不具合報告（`bug_report.rs`）へ
//! 両方とも累積値のまま渡す。
//!
//! **[`record_abandoned`] を呼ぶのは checkpoint1/2（issue 前、`idle_conv_check_probe`
//! 内）で検知した abandon のみ**（実装レビュー指摘M3）。checkpoint3（apply 時点、
//! `key_pipeline.rs::apply_idle_conv_check` の `conv_mutation_seq` と同型の比較）は
//! 到達時点で resync gate が既にクローズ済み＝体感遅延ゼロであり、既存の
//! (a)(b)(c) discard（shift ガード/explicit action/conv_mutation_seq 不一致）と
//! 同様にカウントせず discard するだけに留める——resync カウンタへ混ぜると、
//! 決定Iが分離した「体感コストが桁違いの信号」が薄まってしまう。
//!
//! **abandon 率（決定Iが要求する starvation 判定）には分母が必要**（実装
//! レビュー指摘M1）。[`record_spawned`] を probe を実際に spawn するたびに
//! 呼び、[`record_abandoned`] と同じ resync/通常の軸で累積する。
//! `abandoned_*_lifetime_count() / spawned_*_lifetime_count()` が abandon 率。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// 0 は「まだ一度も観測していない」ことを表すセンチネルとして予約する
/// （`conv_mutation`/`send_health` 等、他の fence 実装と同じ規約）ため 1 から始める。
static PROBE_ACTUATION_FENCE: AtomicU64 = AtomicU64::new(1);

/// GJI IME actuation の物理 syscall 境界（[`crate::win32::send_input_safe`] /
/// [`crate::imm::send_ime_control`]）で呼ぶ。
pub(crate) fn bump() {
    PROBE_ACTUATION_FENCE.fetch_add(1, Ordering::Relaxed);
}

/// 現在の値を読む。probe 側の spawn 時スナップショットと issue/apply 時の
/// 再読み取りを比較するために使う（ビット一致で判定すること、経過時間で
/// 判定しない——`conv_mutation::current()` と同じ規約）。
pub(crate) fn current() -> u64 {
    PROBE_ACTUATION_FENCE.load(Ordering::Relaxed)
}

/// [`crate::ime::get_ime_conversion_mode_fenced_async`] の結果（ADR-140
/// Step1 決定E/F、Step1bで全 probe 呼び出し元共通の型として抽出）。
///
/// `Read(None)`（純粋な読み取り失敗、`SendMessageTimeoutW` のタイムアウト等）と
/// `Abandoned`（probe issue 前後で GJI actuation フェンスの不一致を検知し、
/// probe 自体を安全側に諦めた）を区別可能な別の値として表現する。呼び出し元
/// ごとに `Abandoned` の扱いは異なってよい（例:
/// `kp_stage_idle_conv_check_inner` は resync gate の扱いを変える、
/// `start_ms_ime_ready_poll`/`send_chrome_gji_reinit_and_poll` は
/// 単に「今回のtickは進展なし」として次のpollへ進む）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FencedProbeOutcome {
    /// probe を実際に issue し、結果が返った（成功/タイムアウトを問わない）。
    Read(Option<u32>),
    /// probe issue 前後でフェンス不一致を検知し、OS 呼び出し自体を発行しなかった。
    Abandoned,
}

/// resync 経路（`kp_trigger_focus_resync` 由来）の probe abandon 累計回数
/// （プロセス生存期間中、リセットしない）。`fetch_add` は `u32::MAX`到達時に
/// ラップするが、1プロセスの生存期間中にそこまで到達することは実用上ない
/// ため `saturating_add` にはしていない（他の lifetime カウンタ、
/// 例えば `hook_channel::WAKE_POST_FAILED_LIFETIME_COUNT`、と同じ判断）。
static ABANDONED_RESYNC_LIFETIME_COUNT: AtomicU32 = AtomicU32::new(0);
/// 通常経路（`kp_stage_idle_conv_check`）の probe abandon 累計回数。
static ABANDONED_NORMAL_LIFETIME_COUNT: AtomicU32 = AtomicU32::new(0);
/// resync 経路の probe を実際に spawn した累計回数（abandon 率の分母、
/// 実装レビュー指摘M1）。
static SPAWNED_RESYNC_LIFETIME_COUNT: AtomicU32 = AtomicU32::new(0);
/// 通常経路の probe を実際に spawn した累計回数。
static SPAWNED_NORMAL_LIFETIME_COUNT: AtomicU32 = AtomicU32::new(0);

/// probe が actuation との交錯を検知して checkpoint1/2（issue 前）で abandon
/// したことを記録する。`resync_generation.is_some()` なら resync 経路、`None`
/// なら通常経路として別々に数える（module doc 参照。checkpoint3〈apply 時点〉
/// はここを呼ばない——同 doc の M3 注記参照）。
pub(crate) fn record_abandoned(resync_generation: Option<u64>) {
    if resync_generation.is_some() {
        ABANDONED_RESYNC_LIFETIME_COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        ABANDONED_NORMAL_LIFETIME_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

/// `ABANDONED_RESYNC_LIFETIME_COUNT` を消費せずに読む（不具合報告用診断）。
pub(crate) fn abandoned_resync_lifetime_count() -> u32 {
    ABANDONED_RESYNC_LIFETIME_COUNT.load(Ordering::Relaxed)
}

/// `ABANDONED_NORMAL_LIFETIME_COUNT` を消費せずに読む（不具合報告用診断）。
pub(crate) fn abandoned_normal_lifetime_count() -> u32 {
    ABANDONED_NORMAL_LIFETIME_COUNT.load(Ordering::Relaxed)
}

/// idle-conv-check probe を実際に spawn したことを記録する（abandon 率の分母、
/// 実装レビュー指摘M1）。`record_abandoned` と同じ resync/通常の軸で数える。
pub(crate) fn record_spawned(resync_generation: Option<u64>) {
    if resync_generation.is_some() {
        SPAWNED_RESYNC_LIFETIME_COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        SPAWNED_NORMAL_LIFETIME_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

/// `SPAWNED_RESYNC_LIFETIME_COUNT` を消費せずに読む（不具合報告用診断）。
pub(crate) fn spawned_resync_lifetime_count() -> u32 {
    SPAWNED_RESYNC_LIFETIME_COUNT.load(Ordering::Relaxed)
}

/// `SPAWNED_NORMAL_LIFETIME_COUNT` を消費せずに読む（不具合報告用診断）。
pub(crate) fn spawned_normal_lifetime_count() -> u32 {
    SPAWNED_NORMAL_LIFETIME_COUNT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_advances_current_monotonically() {
        let before = current();
        bump();
        let after = current();
        assert!(after > before, "bump() は current() を単調に進める");
    }

    // 実装レビュー指摘m4: これらのカウンタはプロセス共有の static であり、
    // 同じテストバイナリ内の他のテストが並行して bump/record_* を呼びうる
    // （BUG-65と同型のテスト分離リスク）。したがって「他スレッドがちょうど
    // 割り込まなかった」ことを前提にする厳密等値ではなく、「自分が呼んだ分は
    // 少なくとも反映されている」ことだけを検証する（`>=`）。
    //
    // 実装レビュー指摘m5（再レビュー）: この `>=` 化により、以下2テストは
    // 「resync/normal を分けて数える」という split 自体はもはや検証していない
    // ——両カウンタを無条件に加算する実装に壊れても緑のまま通る。プロセス
    // 共有 static に対して flaky にならずに split を検証するには専用の
    // ローカルインスタンスへの切り出しが要るが、このモジュールの薄さに対して
    // 過剰と判断し、テスト名を実態（「呼んだ側の引数に対応するカウンタが
    // 増える」ことのみ）に合わせるに留める。split の正しさは
    // `crate::probe_actuation_fence` module doc の決定Iの記述と、
    // 呼び出し元 `key_pipeline.rs`（`record_abandoned(resync_generation)`/
    // `record_spawned(resync_generation)`）のコードレビューで担保する。

    #[test]
    fn record_abandoned_increments_the_counter_matching_its_argument() {
        let resync_before = abandoned_resync_lifetime_count();
        let normal_before = abandoned_normal_lifetime_count();

        record_abandoned(Some(42));
        assert!(abandoned_resync_lifetime_count() >= resync_before + 1);

        record_abandoned(None);
        assert!(abandoned_normal_lifetime_count() >= normal_before + 1);
    }

    #[test]
    fn record_spawned_increments_the_counter_matching_its_argument() {
        let resync_before = spawned_resync_lifetime_count();
        let normal_before = spawned_normal_lifetime_count();

        record_spawned(Some(42));
        assert!(spawned_resync_lifetime_count() >= resync_before + 1);

        record_spawned(None);
        assert!(spawned_normal_lifetime_count() >= normal_before + 1);
    }
}
