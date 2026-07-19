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

/// 明示的 IME 操作（Ctrl+変換/無変換 等）後に idle-conv-check を抑制する時間 (ms)。
///
/// Ctrl+変換 後に VK_DBE_HIRAGANA が送られ、GJI probe が ImmSetConversionStatus(ROMAN) を
/// 確立するまでの猶予。この間は conv mode が JISかな (0x00000009) のままなので
/// idle-conv-check が誤って belief を ObservedKana に上書きしないようスキップする。
/// GJI probe budget (350ms) + warmup完了マージン を考慮して 1500ms に設定。
pub const EXPLICIT_IME_SUPPRESS_MS: u64 = 1500;

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
///
/// Chrome VK パス固有のアイドル判定は `CHROME_LONG_IDLE_MS` を参照のこと。
pub const LONG_IDLE_MS: u64 = 10_000;

/// Chrome VK パスでの「長期 idle」判定閾値 (ms)。
///
/// この閾値を超えると Chrome プローブ最小待機時間が 20ms → 200ms に延長される。
/// 実測で idle=6312ms 後に Chrome TSF の composition context 再初期化に ~145ms かかる
/// 事例があり (cold=1040)、20ms probe では "ko" が raw text として出力された。
/// 5000ms に設定することで 5s 以上の idle 後の cold start に 200ms 余裕を確保する。
///
/// TSF/GJI パス（WezTerm 等）は GJI セッション生存期間に依存するため `LONG_IDLE_MS` を使用する。
pub const CHROME_LONG_IDLE_MS: u64 = 5_000;

/// Composition タイムアウト (ms): 変換確定待機の最大時間。
///
/// warm 状態で elapsed がこれを超えた場合、composition が終了したと判断する。
pub const COMPOSITION_TIMEOUT_MS: u64 = 2000;

/// RAW TSF リテラル検出ウィンドウ (ms)。
///
/// warmup_sent_ms からこの時間内に TSF リテラル文字が来た場合、
/// RAW TSF リテラルとして回収する。
pub const RAW_TSF_LITERAL_DETECT_MS: u64 = 300;

/// GJI long idle + TSF mode (WezTerm 等) での RAW TSF リテラル検出ウィンドウ (ms)。
///
/// gji_idle > LONG_IDLE_MS(10000ms) 時、GJI は F2 warmup に対して候補ウィンドウを
/// 表示するまで最大 ~370ms かかる実測がある（通常 300ms 以内に収まる）。
/// FreshF2 パス (eager_elapsed > eager_settle_ms) では NameChangeWait を経由しないため
/// LiteralDetect のタイムアウトで補う必要がある。500ms = 実測最大 ~370ms + 130ms マージン。
pub const RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE: u64 = 500;

/// GJI long idle 後の F2×2 に対する GJI I/O 応答確認 NameChangeWait の最大タイムアウト (ms)。
///
/// GJI I/O 早期終了（gji_long_idle_probe）が機能するケースでは GJI_IDLE_MS (80ms) 静止後に
/// 即送信できる。機能しない（WezTerm + keybinds_ok=false 等）場合のフォールバックタイムアウト。
///
/// 実測（28s アイドル後 cold=16）: F2×2 送信から 181ms 後に GJI が初めて VK を受け入れた。
/// 150ms ではタイムアウトが早すぎ「bあ」のような部分リテラル化が発生したため 350ms に延長。
/// 350ms = 実測最大 ~180ms + 170ms マージン。
/// タイムアウト時は VK_IME_OFF→VK_IME_ON セカンドステージへ。
pub const GJI_LONG_IDLE_PROBE_TOTAL_MS: u64 = 350;

/// GJI セッションが「中程度の idle」と判断する GJI アイドル閾値 (ms)。
///
/// LONG_IDLE_MS (10s) 未満でも ~7s 以上の idle 後は WezTerm TSF が応答するまでに
/// ~325ms かかる実測がある（cold=7: gji_idle=8719ms 後 GJI が 325ms 後に起動）。
/// 300ms 程度の短い待機では間に合わないため、gji_long_idle_probe（GJI I/O 応答監視）
/// をこの閾値以上でも有効にする。
pub const MEDIUM_IDLE_PROBE_MS: u64 = 7_000;

/// Chrome/Unicode-mode GJI 再初期化（VK_IME_OFF→VK_IME_ON）後、`IMC_GETCONVERSIONMODE`
/// で Hiragana を確認するまでの最大待機時間 (ms)。
///
/// `Output::send_f22_f21_reinit`（Unicode injection mode の long-cold GJI 再起動）が
/// `send_chrome_gji_reinit_and_poll` 経由で使う。GJI は VK_IME_ON 受信後 ~50-100ms 以内に
/// IME ON 状態に移行する実測値が多い。300ms あれば十分な余裕を確保できる。タイムアウト時は
/// 強制再送する。
pub const CHROME_GJI_REINIT_CONFIRM_MS: u64 = 300;

/// [`CHROME_GJI_REINIT_CONFIRM_MS`] のポーリング間隔 (ms)。
///
/// `IMC_GETCONVERSIONMODE` を async でこの間隔ごとに発行する。
/// 10ms 間隔で最大 30 回 = 300ms（`CHROME_GJI_REINIT_CONFIRM_MS` に対応）。
pub const CHROME_GJI_REINIT_POLL_INTERVAL_MS: u64 = 10;

/// MS-IME confirm-then-transmit ゲート（BUG-13）の確認期限 (ms)。
///
/// **待ち時間ではなく安全弁**。準備完了の確認は `IMC_GETCONVERSIONMODE` ポーリングが
/// 担い、NATIVE 確認の瞬間に送信するため通常のレイテンシは実際の準備時間 + ポーリング
/// 1 tick で済む。この定数が効くのは IMC が読めない（None が返り続ける）環境のみで、
/// 期限到達で強制送信 + give-up latch（以後 gate 停止）に落ちる。
///
/// 実測 (2026-07-06, Windows Terminal × MS-IME, IME OFF→ON 遷移):
/// - +122ms: conv=0x00000000（未準備。この時点の送信で「を」→「wお」リテラル化 = BUG-13）
/// - +281ms: conv=0x00000009（準備完了。「で」が正常に compose）
///
/// 準備完了の実測上限 ~281ms + マージン ~120ms = 400ms。
pub const MS_IME_READY_CONFIRM_MS: u64 = 400;

/// MS-IME confirm-then-transmit ゲートの IMC ポーリング間隔 (ms)。
pub const MS_IME_READY_POLL_INTERVAL_MS: u64 = 10;

// === キャッシュ有効期限 ===

/// フォーカス切り替え時の per-HWND IME 状態スナップショットの最大有効期間 (ms)。
///
/// awase がすべての IME 状態変化をフックしているため、キャッシュは原則的に正確に保たれる。
/// ただし 1 時間を超えると "昨日の設定" の復元になりユーザーが混乱するため上限を設ける。
pub const HWND_CACHE_MAX_AGE_MS: u64 = 3_600_000;

/// フォーカスがこの時間（ms）未満しか滞在しなかったウィンドウの IME 状態はキャッシュに保存しない。
///
/// 通知ポップアップ等の瞬間フォーカスが正常な状態を上書きするのを防ぐ。
pub const MIN_FOCUS_DURATION_MS: u64 = 100;

// === 観測失敗カウント ===

/// IME 状態検出の連続失敗がこの回数以上になると Engine を非活性にする。
///
/// ポーリング間隔 500ms × 3 = 1.5秒。一時的な検出失敗は許容しつつ、
/// 長時間の乖離（実際は IME OFF なのにキャッシュが ON のまま）を防ぐ。
pub const IME_DETECT_MISS_THRESHOLD: u32 = 3;

// === ドリフト補正 ===

/// `desired` と `observed` の乖離がこの時間以上続いた場合にドリフト補正を発動する (ms)。
///
/// ポーリング間隔 500ms より小さい値にすると、ドリフト検出後の次のポーリング
/// （drift_duration ≈ 500ms）で確実に補正が発動する。
/// 短すぎるとフォーカス変化直後の一時的なズレで誤発動するため 400ms とする。
pub const DRIFT_CORRECTION_THRESHOLD_MS: u64 = 400;

/// ドリフト補正の「信頼できる観測」として許可する最大観測年齢 (ms)。
///
/// この時間より古い観測値は stale とみなしてドリフト補正の根拠として使わない。
pub const DRIFT_CORRECTION_OBS_MAX_AGE_MS: u64 = 1_500;

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

// === 診断用（一時的） ===

/// WezTerm/TSF 側の予防的 warmup を丸ごと止め、reactive な検出のみに委ねる診断フラグ。
///
/// 対象は `gji_warmup_coro.rs` の早期 SendFreshF2 + long-cold 時の
/// StartSacrificialWarmup escalation。これらを止め、`LiteralDetect`
/// （事後の compose 確認 + partial/suspected literal 回収）のみに委ねる。
///
/// `DIAG_SKIP_PROACTIVE_SACRIFICIAL_WARMUP`（Chrome 専用）の対称版。あちらのコメントに
/// 明記の通り「TSF/WezTerm 側は対象外」だったため、BUG-24（`docs/known-bugs.md`）の
/// 偽陰性疑いを実機で検証するために新設した。
///
/// ## 目的（BUG-24 検証）
///
/// BUG-24 は「`is_partial_literal()` が romaji 自体の compose 結果ではなく、別の
/// warmup F2 キーへの応答（`nc_fired`/`gji_resumed`）を代理指標にしている」ことに
/// 起因する偽陽性・偽陰性リスクを記録したもの。ユーザー仮説: 現状は予防的 warmup が
/// 広く・保守的にかかっているため `needs_literal=false`（LiteralDetect 自体を
/// スキップする分岐）に実際には滅多に入らず、偽陰性（部分リテラルの検知漏れ）が
/// 実害として顕在化していないだけではないか。予防を絞れば顕在化するはず、というもの。
///
/// `true` の間は以下 3 箇所を無効化する:
///
/// 1. **Phase 2 (`SendFreshF2`) をスキップ**: `gji_coro_body` の pre-idle スキップ判定の
///    直後に、フラグが立っていれば無条件で `break 'initial (false, false)` する
///    （F2 warmup キー自体を送らない → `nc_fired`/`gji_resumed` は常に false のまま）。
/// 2. **Phase 5a (`StartSacrificialWarmup`) をスキップ**: `long_cold && is_tsf_mode` の
///    条件を満たしても発火させず、Phase 5b（直接 Transmit）にフォールスルーさせる。
/// 3. **`effective_prepend_f2` を強制 false**: 上記 1・2 を無効化しても、実際の
///    romaji バッチに F2 を直接同梱する第3の防御層（`effective_prepend_f2`、
///    `should_prepend_f2` の入力）が生きていると予防が実質効いたままになる。
///    2026-07-11 の実機ログで、1・2 だけ無効化した状態でも idle=1188ms の "ko" 送信が
///    `vks=[F2,4B,4F]`（F2 先頭同梱）でカバーされ、無防備な状態を検証できていなかった
///    ことが判明したため追加した。
///
/// いずれも `plan.needs_literal` 自体の計算式（`decide_transmit_plan`）は変更しない。
/// 早期 F2 を送らないことで `nc_fired` が常に false になり、
/// `needs_literal` の第2項（`!nc_fired && is_tsf_mode && gji_active && !gji_resumed`）が
/// 真になりやすくなるため、実際には LiteralDetect 自体は有効なままのケースが多いと
/// 予想される。**もし本当に「予防を絞ると偽陰性が顕在化する」なら、それでも
/// `needs_literal=false` になってしまうケース、あるいは LiteralDetect が有効でも
/// 検知漏れするケースが実機ログに現れるはず**——このフラグはあくまで「予防を薄くして
/// reactive パスの露出を増やす」ための道具であり、偽陰性の直接証明ではない。
///
/// ## 観測すべきログ
///
/// - `[gji-coro] cold=N DIAG_DISABLE_PROACTIVE_TSF_WARMUP → skip FreshF2 (BUG-24 検証)`:
///   フラグが実際に発火した回数・頻度。
/// - `[literal-detect] cold=N partial literal ...` / `suspected literal ...`:
///   reactive 検出が実際に発火したか（発火すれば偽陰性ではない、正常に機能）。
/// - 画面上で実際に文字化け（ローマ字がそのまま残る等）が見えるのに、上記
///   `[literal-detect]` ログが一切出ていない場合 → **偽陰性が実際に起きている証拠**。
/// - `[gji-coro-diag] ... skip-verify`（`needs_literal=false` だった場合の事後診断）が
///   出た場合、その後実際に問題があったかどうか（画面の目視確認と突き合わせる）。
///
/// 実験終了後は `false` に戻すか、結果に応じて `docs/known-bugs.md` BUG-24 に実測を
/// 追記した上で撤去すること（`tuning-constants.md` の実測義務）。
///
/// 2026-07-11 追記: 実機1セッションで `SetOpenTrue`（cold=1,10,11,12,14、
/// real_gji_idle_ms 282〜1188ms）の"ko"送信は全件 ESC-based 回収（`4e31b64`）が
/// 正しく機能し文字化けなし。`nc_fired=true`（cold=5,6,9,13）で `needs_literal=false`
/// となり LiteralDetect 自体が起動しなかったケースも目視で文字化けなしを確認
/// （`nc_fired=true` の判定がこのケースでは実態と一致していたと解釈、偽陰性の
/// 証拠にはならない）。詳細は `docs/known-bugs.md` BUG-24 参照。1セッション分の
/// データのみのため、ユーザー判断で `true` のまま実運用を継続し、より広い条件
/// （長い idle・他の cold_reason・他アプリ・複数セッション）で追加検証中。
pub const DIAG_DISABLE_PROACTIVE_TSF_WARMUP: bool = true;
