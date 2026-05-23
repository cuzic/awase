/// タイミング定数の集約モジュール。
///
/// awase-windows 全体で使われるタイミング関連の定数をここに集める。
/// 値を変更する場合はこのファイルだけを編集すればよい。

// === IME 観測タイミング ===

/// 最後のキー活動（物理キー押下 または VK/TSF 出力）から IME ポーリングを
/// 開始するまでの静止時間 (ms)。
///
/// タイピング中は IMM との SendMessage を一切行わない。
pub const TYPING_IDLE_MS: u64 = 500;

/// GJI I/O が静止したと判断するまでの時間 (ms)。
///
/// warmup 後に GJI I/O が発生した場合、この時間以上静止したら settled と判断する。
pub const GJI_IDLE_MS: u64 = 80;

/// GJI 静止確認後の余裕マージン (ms)。
///
/// settled 検出後にさらにこの時間だけ待機してから送信する。
pub const POST_IDLE_MARGIN_MS: u64 = 30;

/// GJI I/O を IME ON の証拠として認める判定ウィンドウ (ms)。
///
/// 直近この時間以内に GJI I/O が観測された場合、Chrome 等の broken IMM
/// アプリでも IME が ON であると判断する。
pub const GJI_CONFIRM_WINDOW_MS: u64 = 500;

// === キャッシュ有効期限 ===

/// フォーカス切り替え時の per-HWND IME 状態スナップショットの最大有効期間 (ms)。
pub const HWND_CACHE_MAX_AGE_MS: u64 = 5_000;

// === TSF warmup ===

/// cold 発生前のアイドル時間がこれ以上なら「長期 idle」と判定する (ms)。
///
/// 2-9s 程度の「考える・少し読む」では GJI セッションが生存しているため、
/// 低すぎる閾値は NG（GJI I/O が発火せず probe が 1500ms でタイムアウトしてしまう）。
/// 10s 以上の長期 idle（矢印キーナビゲーション等）では GJI セッションリセットが確実。
pub const LONG_IDLE_MS: u64 = 10_000;

// === 観測失敗カウント ===

/// IME 状態検出の連続失敗がこの回数以上になると Engine を非活性にする。
///
/// ポーリング間隔 500ms × 3 = 1.5秒。一時的な検出失敗は許容しつつ、
/// 長時間の乖離（実際は IME OFF なのにキャッシュが ON のまま）を防ぐ。
pub const IME_DETECT_MISS_THRESHOLD: u32 = 3;
