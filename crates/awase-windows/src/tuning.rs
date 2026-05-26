//! タイミング定数の集約モジュール。
//!
//! awase-windows 全体で使われるタイミング関連の定数をここに集める。
//! 値を変更する場合はこのファイルだけを編集すればよい。

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

// === TSF warmup タイミング ===

/// cold 発生前のアイドル時間がこれ以上なら「長期 idle」と判定する (ms)。
///
/// 2-9s 程度の「考える・少し読む」では GJI セッションが生存しているため、
/// 低すぎる閾値は NG（GJI I/O が発火せず probe が 1500ms でタイムアウトしてしまう）。
/// 10s 以上の長期 idle（矢印キーナビゲーション等）では GJI セッションリセットが確実。
pub const LONG_IDLE_MS: u64 = 10_000;

/// Composition タイムアウト (ms): 変換確定待機の最大時間。
///
/// warm 状態で elapsed がこれを超えた場合、composition が終了したと判断する。
pub const COMPOSITION_TIMEOUT_MS: u64 = 2000;

/// RAW TSF リテラル検出ウィンドウ (ms)。
///
/// warmup_sent_ms からこの時間内に TSF リテラル文字が来た場合、
/// RAW TSF リテラルとして回収する。
pub const RAW_TSF_LITERAL_DETECT_MS: u64 = 300;

/// ProbeWithSettle フェーズでの再 warmup 最大待機時間 (ms)。
///
/// eager_re_warmup で F2 を送信してから GJI settled を待つ最大時間。
pub const RE_WARMUP_MS: u64 = 500;

/// GJI settled 判定後の第二プローブ最大待機時間 (ms)。
///
/// OBJ_NAMECHANGE 後に GJI が再び発火した場合の二次プローブ上限。
pub const SETTLE_TIMEOUT_MS: u64 = 300;

/// OBJ_NAMECHANGE 後の GJI 二次プローブ最大待機時間 (ms)。
pub const GJI_POST_NAMECHANGE_MS: u64 = 300;

/// Chrome プローブ最小待機時間 (ms)。
///
/// F2 を SendMessageTimeout で送信後、TSF 応答を待つ最低時間。
pub const CHROME_PROBE_MIN_MS: u64 = 20;

/// Chrome プローブ最大待機時間 (ms)。
pub const CHROME_PROBE_MAX_MS: u64 = 120;

/// 長期 idle (`idle_ms_at_last_cold > LONG_IDLE_MS`) cold start 時の Chrome プローブ
/// 最小待機時間 (ms)。GJI が長期 idle 後に reinit に要する時間を見越して延長する。
pub const CHROME_PROBE_LONG_IDLE_MIN_MS: u64 = 100;

/// 長期 idle 時の Chrome プローブ最大待機時間 (ms)。
///
/// 120ms の上限では GJI が再活性化する前に timeout して literal "ra" が出力される
/// 症状を抑えるため、500ms まで延長する（GJI が settle すれば短く済む）。
pub const CHROME_PROBE_LONG_IDLE_MAX_MS: u64 = 500;

// === キャッシュ有効期限 ===

/// フォーカス切り替え時の per-HWND IME 状態スナップショットの最大有効期間 (ms)。
pub const HWND_CACHE_MAX_AGE_MS: u64 = 5_000;

// === 観測失敗カウント ===

/// IME 状態検出の連続失敗がこの回数以上になると Engine を非活性にする。
///
/// ポーリング間隔 500ms × 3 = 1.5秒。一時的な検出失敗は許容しつつ、
/// 長時間の乖離（実際は IME OFF なのにキャッシュが ON のまま）を防ぐ。
pub const IME_DETECT_MISS_THRESHOLD: u32 = 3;

// === グレース・マージン ===

/// TSF warmup 完了直後のグレース期間 (ms)。
///
/// warmup から WARMUP_GRACE_MS 以内に probe 結果が届いた場合、
/// IME 状態変化によるフリップを抑制する。
pub const WARMUP_GRACE_MS: u64 = 300;

/// GJI 静止直後のグレース期間 (ms)。
///
/// フォーカス変更後に GJI I/O が発生し、最後の I/O からこの時間内なら
/// probe 結果による IME 状態フリップを抑制する。
pub const GJI_SETTLE_GRACE_MS: u64 = 300;

/// シャドウ IME グレース期間 (ms)。
///
/// シャドウ IME が有効な状態で probe_age がこの時間内なら抑制する。
pub const SHADOW_GRACE_MS: u64 = 200;

/// 出力送信後の後続キー保護期間 (ms)。
///
/// SendInput 直後この時間は OS キューに出力イベントが残っているため、
/// passthrough キーや ReinjectKey の処理を遅延させて race を防ぐ。
pub const OUTPUT_GUARD_MS: u64 = 50;

// === TSF GJI モニタ ===

/// GJI I/O モニタスレッドのサンプリング間隔 (ms)。
pub const GJI_SAMPLE_INTERVAL_MS: u32 = 10;

/// GJI モニタが切断後に再アタッチを試みる間隔 (ms)。
pub const GJI_REATTACH_INTERVAL_MS: u64 = 3_000;
