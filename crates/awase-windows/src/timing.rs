/// output.rs / executor.rs 内の同名定数と統合する共通タイミング定数。

/// Composition タイムアウト (2000 ms): 変換確定待機の最大時間
pub const COMPOSITION_TIMEOUT_MS: u64 = 2000;

/// 長期アイドル閾値 (10000 ms): ログ出力を抑制するアイドル判定
pub const LONG_IDLE_MS: u64 = 10_000;

/// RAW TSF リテラル検出ウィンドウ (300 ms)
pub const RAW_TSF_LITERAL_DETECT_MS: u64 = 300;

/// タイピングアイドル閾値 (500 ms): IME ポーリング間隔相当
pub const TYPING_IDLE_MS: u64 = 500;

/// PassThrough 出力ガード遅延 (50 ms): SendInput 直後のレース防止
pub const OUTPUT_GUARD_MS: u64 = 50;

/// HWND キャッシュ最大年齢 (5000 ms): フォーカス変更後の IME キャッシュ有効期間
pub const HWND_CACHE_MAX_AGE_MS: u64 = 5_000;
