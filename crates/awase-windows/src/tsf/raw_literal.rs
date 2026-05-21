/// raw TSF literal 検出後の回収ペイロード。
///
/// バックスペース数とローマ字再送文字列を一括管理する。
/// WM_DRAIN_OUTPUT_QUEUE ハンドラが `flush_raw_tsf_literal_recovery()` で消費する。
#[derive(Debug)]
pub struct RawTsfLiteralPending {
    /// 送信すべきバックスペースの数
    pub backs: std::sync::atomic::AtomicUsize,
    /// 再送すべきローマ字文字列（空文字列 = 再送なし）
    pub romaji: std::sync::Mutex<String>,
}

impl RawTsfLiteralPending {
    const fn new() -> Self {
        Self {
            backs: std::sync::atomic::AtomicUsize::new(0),
            romaji: std::sync::Mutex::new(String::new()),
        }
    }
}

pub static RAW_TSF_LITERAL: RawTsfLiteralPending = RawTsfLiteralPending::new();
