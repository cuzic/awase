//! `Output` 層から `Runtime` 層への遅延リクエストを束ねるアウトボックス。
//!
//! `Output` はキー注入中に「フォーカス再分類」「IME リフレッシュ」等の
//! Runtime 操作を直接呼べない（`with_app` 再入を招く）。代わりに
//! `RuntimeRequest` を `RuntimeOutbox` に積み、キー処理の境界で Runtime が
//! `drain` して実行する。
//!
//! # 使い方
//! - push 側: `vk_send.rs` の Chrome cold パスが `StartTsfProbe` を積む（H-4-b 完了）。
//! - drain 側: `Runtime::drain_runtime_requests()` が `WM_EXECUTE_EFFECTS` /
//!   `WM_DRAIN_OUTPUT_QUEUE` の末尾で呼ばれ、各リクエストを実行する。

use awase::types::VkCode;

/// `Output` が `Runtime` に依頼する遅延操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeRequest {
    /// IME 状態を再取得して shadow model を同期する。
    RefreshIme,
    /// アイドル時の変換モードチェックタイマーを再スケジュールする。
    ScheduleIdleConvCheck,
    /// TSF プローブ（cold start ウォームアップ）を開始する。
    ///
    /// `output.install_pending_tsf()` で probe を先にインストールしてから push すること。
    /// `drain_runtime_requests` が `output.pending_tsf_timer()` でタイマー命令を取得し適用する。
    StartTsfProbe,
    /// フォーカスウィンドウの注入クラスを再分類する。
    ReclassifyFocus { vk: VkCode },
}

/// `RuntimeRequest` を蓄積し、Runtime が一括で取り出す FIFO バッファ。
#[derive(Debug, Default)]
pub(crate) struct RuntimeOutbox {
    requests: Vec<RuntimeRequest>,
}

impl RuntimeOutbox {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self { requests: Vec::new() }
    }

    /// リクエストを末尾に積む。
    pub(crate) fn push(&mut self, request: RuntimeRequest) {
        self.requests.push(request);
    }

    /// 蓄積したリクエストを全件取り出してバッファを空にする。
    pub(crate) fn drain(&mut self) -> Vec<RuntimeRequest> {
        std::mem::take(&mut self.requests)
    }

    /// 保留リクエストがなければ true。
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}
