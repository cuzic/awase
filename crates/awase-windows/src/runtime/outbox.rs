//! `Output` 層から `Runtime` 層への遅延リクエストを束ねるアウトボックス。
//!
//! `Output` はキー注入中に「フォーカス再分類」「IME リフレッシュ」等の
//! Runtime 操作を直接呼べない（`with_app` 再入を招く）。代わりに
//! `RuntimeRequest` を `RuntimeOutbox` に積み、キー処理の境界で Runtime が
//! `drain` して実行する。

use awase::types::VkCode;

/// `Output` が `Runtime` に依頼する遅延操作。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // #17 (H-4-b) で vk_send.rs の with_app 置換時に構築される
pub(crate) enum RuntimeRequest {
    /// IME 状態を再取得して shadow model を同期する。
    RefreshIme,
    /// アイドル時の変換モードチェックタイマーを再スケジュールする。
    ScheduleIdleConvCheck,
    /// TSF プローブ（cold start ウォームアップ）を開始する。
    StartTsfProbe,
    /// フォーカスウィンドウの注入クラスを再分類する。
    ReclassifyFocus { vk: VkCode },
}

/// `RuntimeRequest` を蓄積し、Runtime が一括で取り出す FIFO バッファ。
#[derive(Debug, Default)]
#[allow(dead_code)] // #17 (H-4-b) で Output に組み込まれる
pub(crate) struct RuntimeOutbox {
    requests: Vec<RuntimeRequest>,
}

#[allow(dead_code)] // #17 (H-4-b) で呼び出される
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
