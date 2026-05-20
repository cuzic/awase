//! action 層 — judgement 結果を元に SendInput を組み立て実行する。
//!
//! - `ColdReason`: cold になった理由（タイミングパラメータを決定する）
//! - `TsfOutput`: warmup F2 前置き、ローマ字送信、raw-TSF-literal 回収を実行
