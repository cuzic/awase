/// IME 関連キー押下のタイムスタンプ（循環バッファ）。
/// フックコールバックはメインスレッドで実行されるため `SingleThreadCell` で十分。
pub(super) static RAPID_IME_TIMESTAMPS: awase_windows::SingleThreadCell<RapidPressTracker> =
    awase_windows::SingleThreadCell::new();

/// 連打検出用の軽量トラッカー
pub(super) struct RapidPressTracker {
    /// 直近のタイムスタンプ（最大 `THRESHOLD` 個保持）
    buf: [u64; 3],
    /// 次の書き込み位置
    cursor: usize,
    /// 有効なエントリ数
    count: usize,
}

impl RapidPressTracker {
    /// 検出閾値: この回数以上の IME キー押下で発動
    const THRESHOLD: usize = 3;
    /// 検出ウィンドウ（ミリ秒）
    const WINDOW_MS: u64 = 1000;

    pub(super) const fn new() -> Self {
        Self {
            buf: [0; Self::THRESHOLD],
            cursor: 0,
            count: 0,
        }
    }

    /// タイムスタンプを記録し、連打が検出されたら `true` を返す。
    pub(super) fn push(&mut self, now_ms: u64) -> bool {
        self.buf[self.cursor] = now_ms;
        self.cursor = (self.cursor + 1) % Self::THRESHOLD;
        if self.count < Self::THRESHOLD {
            self.count += 1;
        }

        if self.count < Self::THRESHOLD {
            return false;
        }

        // 全エントリが WINDOW_MS 以内に収まっているか
        let oldest = *self.buf.iter().min().unwrap_or(&0);
        now_ms.saturating_sub(oldest) < Self::WINDOW_MS
    }

    /// バッファをクリアする（発動後のリセット用）
    pub(super) fn clear(&mut self) {
        self.buf = [0; Self::THRESHOLD];
        self.cursor = 0;
        self.count = 0;
    }
}
