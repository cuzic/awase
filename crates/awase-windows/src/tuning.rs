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

/// ProbeWithSettle フェーズでの再 warmup 最大待機時間 (ms)。
///
/// eager_re_warmup で F2 を送信してから GJI settled を待つ最大時間。
pub const RE_WARMUP_MS: u64 = 500;

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

/// Chrome プローブ最小待機時間 (ms)。
///
/// F2 を SendMessageTimeout で送信後、TSF 応答を待つ最低時間。
pub const CHROME_PROBE_MIN_MS: u64 = 20;

/// Chrome プローブ最大待機時間 (ms)。
pub const CHROME_PROBE_MAX_MS: u64 = 120;

/// 長期 idle (`idle_ms_at_last_cold > LONG_IDLE_MS`) cold start 時の Chrome プローブ
/// 最小待機時間 (ms)。GJI が長期 idle 後に reinit に要する時間を見越して延長する。
///
/// Chrome TSF は GJI I/O を出さないため probe は min_ms 経過後に即発火する。
/// 実測で Chrome が F2 受信後 composition context を再初期化するのに ~114ms 必要なケースを
/// 確認（probe は f2_sent_ms 起点なので async overhead ~7ms を差し引くと実効 ~107ms）。
/// 200ms に設定することで十分な余裕を確保する。
pub const CHROME_PROBE_LONG_IDLE_MIN_MS: u64 = 200;

/// 長期 idle 時の Chrome プローブ最大待機時間 (ms)。
///
/// 120ms の上限では GJI が再活性化する前に timeout して literal "ra" が出力される
/// 症状を抑えるため、500ms まで延長する（GJI が settle すれば短く済む）。
pub const CHROME_PROBE_LONG_IDLE_MAX_MS: u64 = 500;

/// Chrome GJI 再初期化（VK_IME_OFF→VK_IME_ON）後、`IMC_GETCONVERSIONMODE` で Hiragana を確認するまでの
/// 最大待機時間 (ms)。
///
/// GJI は VK_IME_ON 受信後 ~50-100ms 以内に IME ON 状態に移行する実測値が多い。
/// 300ms あれば十分な余裕を確保できる。タイムアウト時は強制再送する。
pub const CHROME_GJI_REINIT_CONFIRM_MS: u64 = 300;

/// Chrome GJI 再初期化確認ポーリング間隔 (ms)。
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
///
/// `CHROME_GJI_REINIT_POLL_INTERVAL_MS` と同じ 10ms（同じシグナル・同じ発行機構）。
pub const MS_IME_READY_POLL_INTERVAL_MS: u64 = 10;

/// SacrificialWarmup（VK_A+BS）で composition-confirmed 後、GJI candidate window が
/// 非表示になるのを待つ最大時間 (ms)。
///
/// ## 背景（Chrome IPC race condition）
///
/// Chrome/GJI の IME 処理はクロスプロセス IPC（レンダラー↔ブラウザー）を経由するため、
/// VK_A+BS の EndComposition が TSF スタックを伝播するまでに ~200ms 程度かかる。
///
/// GJI write (+400B) による composition-confirmed は VK_A+BS 送信から ~26ms で検出できるが、
/// そこで即座に実ローマ字を送ると、delayed EndComposition IPC が後続の composition を
/// キャンセルする競合が発生する（例：「korede」が composition 後 225ms で消える）。
///
/// candidate window が非表示になった（HIDE 観測）= EndComposition IPC が Chrome に到達した
/// ことの代理指標として使い、その後に実ローマ字を送ることで race を回避する。
///
/// 実測: VK_A+BS → HIDE まで ~225ms（long-idle cold Chrome）。300ms で十分な余裕を確保。
/// タイムアウト時（window が最初から非表示だった場合等）は即時再送する。
pub const SACR_WARMUP_CHROME_HIDE_WAIT_MS: u64 = 300;

/// VK_A+BS atomic batch で SHOW+HIDE が最初の tick より前に完了したときの IPC settle 待機時間（ms）。
///
/// VK_A+BS は1回の SendInput で送るため Chrome は同一メッセージポンプ反復内で処理し、
/// SHOW と HIDE が ~5ms 以内に連続して発火する。この場合 sacr-warmup の最初の tick では
/// `gji_candidate_visible=false` だが EndComposition IPC はまだ Chrome に伝播中（~200ms）。
/// HIDE wait（Phase 2）が機能しないため、固定時間の IPC settle 待機（Phase 3）を使う。
/// ~200ms の IPC 伝播より長い 250ms を設定する。
pub const SACR_WARMUP_CHROME_IPC_SETTLE_MS: u64 = 250;

/// F2NonTsf cold start で物理 F2 送信からこの時間以上経過した場合、
/// Chrome の TSF composition context が失効した可能性があるため
/// programmatic F2 を再送する（ms）。
///
/// 背景: Chrome は F2 受信後 ~326ms で composition context を初期化するが、
/// 一定時間キー入力がないと context が失効する（実測: 1688ms で失効確認）。
/// 失効後のバッチ送信では最初のキーが literal になるバグが発生する
/// （例: まぁ → mあぁ）。1200ms 超過時に programmatic F2 を再送することで
/// context を確実に再初期化する。
pub const F2_STALE_MS: u64 = 1200;

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

/// Chrome/Edge 経路の `is_long_cold` 事前予防を一時的に無効化する診断フラグ。
///
/// 対象は `StartSacrificialWarmup` を送信前に先制発行する分岐
/// （`tsf/warmup/probe_fsm.rs` Phase 2a、BUG-21）。適用箇所は `output/vk_send.rs`
/// （`send_romaji_batched` が Chrome 用 `is_long_cold` を計算する箇所）の1点のみ。
/// `probe_fsm.rs`/`gji_warmup_coro.rs` 自体の分岐ロジックは変更しない（`is_long_cold` を
/// 直接パラメータで受け取るため、既存の BUG-21 回帰テストは本フラグの影響を受けずそのまま
/// 有効）。
///
/// TSF/WezTerm 側（`gji_warmup_coro.rs` Phase 5a）は対象外。あちらの `ctx.is_long_cold`
/// は `ColdKind::is_long()`（cold 突入時点の `gji_idle_ms()` 実IO観測から分類済み）に
/// 由来し、Chrome 側の自己参照タイマー（`idle_ms_at_last_cold()` = 「awase 自身が最後に
/// 送信してからの経過時間」）と異なり既に実IOに基づいている。既に信頼できる信号を
/// 診断目的で止める理由がなく、WezTerm cold-start literal 化の実害リスクの方が大きい。
///
/// `true` の間は Chrome/Edge の予防的 `StartSacrificialWarmup` をスキップし、常に直接送信 +
/// inline `LiteralDetect` の事後検出に委ねる。partial literal 検出後の捨て駒リトライ
/// （`literal_detect_fsm.rs::emit_recovery_actions` の `is_tsf_mode && consecutive==0` 分岐）
/// は対象外で今まで通り動作する。
///
/// 目的: 「本当は long cold ではなかったのに誤って先制で捨て駒を送っていたケース」を
/// 実機ログ（`[h1-probe-diag]` の自己タイマー vs `real_gji_idle_ms`、および
/// `[literal-detect]` の `CompositionConfirmed`/`SuspectedLiteral`/`partial-literal` 実際の
/// 結果）から炙り出すための一時計測。実測データが取れて narrowing 方針が決まったら、
/// このフラグと分岐ごと撤去するか恒久的なゲート条件に置き換えること
/// （cold 突入判定の絞り込み調査の一部、2026-07-09〜）。
///
/// 2026-07-09 追記: 実機7サンプル（目視確認込み）で予防的分岐スキップは無破損だった。
/// 次の実験（`DIAG_CHROME_SACRIFICIAL_KEY_IME_OFFON`）は「予防分岐自体を止める」のではなく
/// 「予防分岐は残しつつ Chrome の犠牲キーを差し替える」検証のため、本フラグは `false` に戻し
/// 予防分岐を通常通り発火させる（そうしないと差し替え後のキーを試す機会がない）。
pub const DIAG_SKIP_PROACTIVE_SACRIFICIAL_WARMUP: bool = false;

/// Chrome/Edge の `StartSacrificialWarmup` 犠牲キーを `VkAThenBackspace` から
/// `ImeOffThenOn`（VK_IME_OFF→VK_IME_ON）に一時的に差し替える診断フラグ。
///
/// 適用箇所は `output/probe_io.rs` の `StartSacrificialWarmup` ディスパッチ1点のみ。
/// `state/key_sequence_policy.rs::sacrificial_warmup_key`（実機確定根拠 `22c3905` の
/// 宣言的テーブル）自体は変更しない — あちらは「TSF は ImeOffThenOn、Chrome は
/// VkAThenBackspace」という実機確定済みの事実の記録であり、診断中の仮説で汚さない。
///
/// ## 背景
///
/// `ImeOffThenOn`（`tsf/warmup/ime_offon_warmup_fsm.rs`）は元々 WezTerm 等 TSF 専用
/// （vim の VK_A 誤爆対策）。Chrome には ADR-048/`22c3905` の「VK_IME_OFF が Chrome
/// TSF context を壊す」という知見から使われてこなかった。ただしこの知見の実機再検証は
/// `6c1732d`→`22c3905`（2026-06-30、6分差）の revert 由来で、今回は改めて実機で
/// 確認する（ユーザーの記憶では当時も実機検証済みとのことだが、念のため再確認）。
///
/// VK_IME_ON 単体（OFF を挟まない）は試さない — `VK_IME_ON`/`VK_IME_OFF` は冪等キー
/// （ADR-067）であり、engine が既に ON の状態で ON を再送しても状態遷移が起きず
/// GJI が無反応になると考えられるため。OFF→ON で実際に状態遷移を起こす。
///
/// `true` の間は Chrome の犠牲キーが VK_A+BS ではなく VK_IME_OFF→ON になる。
/// 壊れた場合の症状: Chrome の TSF composition context が破壊され、ADR-048 が記録した
/// Teams "na" literal 化と同種の症状（部分/全体リテラル化）が予想される。
/// 実機で目視確認しながら試すこと。
///
/// 2026-07-09 追記: 実験中止・`false` に戻した。VK_IME_OFF→ON 自体は Chrome cold-start で
/// 2/2 サンプル成功（composition confirmed・目視でも正しい文字）していたが、同じ実機セッション
/// 後半で Shift/Ctrl が stuck するバグが発生した（`mods(s=true)` が KeyUp 後もクリアされず、
/// `[engine-input] CTRL MISMATCH: mods.ctrl=false だが phys_ctrl=true → synthetic Ctrl↑ が
/// GetAsyncKeyState を汚染した可能性がある` という既存の自己診断WARNまで出た）。
///
/// **本フラグとの因果関係は否定された**（当初「合成IMEキー送信頻度の増加が誘因」と
/// 推定したが、症状が実際に出ていたのは Windows Terminal（TSF/
/// `CASCADIA_HOSTING_WINDOW_CLASS`）で、このフラグは `TransmitTarget::Chrome` 専用の
/// ためそちらのコードパスには一切触れない）。
///
/// 2026-07-09 追記2: stuck modifier バグの真因を確定した（BUG-23,
/// `docs/known-bugs.md`）。Windows ロック画面（Secure Desktop）遷移中は
/// `WH_KEYBOARD_LL` フックがイベントを一切観測できず、ロックの瞬間に押されていた
/// 修飾キーの KeyUp が失われて `hook::PHYSICAL_KEY_STATE` が stuck していた
/// （`hook::reset_physical_key_state()` を新設し `WTS_SESSION_UNLOCK`/`panic_reset()`
/// から呼ぶ修正 済み、コミット `77536d6`）。本フラグとは完全に無関係と確定したため
/// 実験を再開する。
pub const DIAG_CHROME_SACRIFICIAL_KEY_IME_OFFON: bool = true;

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

/// カタカナ/英数 charset への「追従」warmup・VK 前置ロジックを丸ごと無効化し、
/// 常に Hiragana(F2) として振る舞わせる診断フラグ（BUG-19 検証）。
///
/// ## 背景
///
/// BUG-19（`docs/known-bugs.md`）は「GJI の conv mode が謎にカタカナへ変化し、
/// awase がそれに"追従"して実キー VK_DBE_KATAKANA を送ることで、一過性かも
/// しれない状態を real IME の恒久的な状態としてロックインしてしまう」問題。
/// 追補4までにデバウンス・復元書き込みの重複排除などを重ねてきたが、いずれも
/// 「観測されたカタカナに追従して warmup キーを送る」という設計自体は残していた。
///
/// ユーザー（本プロジェクトの唯一の実利用者）は IME トレイからカタカナ/半角
/// 英数を手動選択したことが一度もなく、今後もその予定がないと明言している。
/// つまりこのユーザーの実運用においては、conv mode がカタカナ/英数へ変化する
/// ことがあるとすれば、それは常に GJI 側の何らかの内部ドリフト（本物のユーザー
/// 意図ではない）であり、awase 側がそれに追従する必要（927f2a2/109b4c9 が
/// 保護していた「トレイでカタカナ選択したユーザー」のケース）が実質存在しない。
///
/// `true` の間は以下 3 箇所で charset を常に `Charset::Hiragana` 扱いにする
/// （`ConvModeMgr::effective_charset()` 経由）:
///
/// 1. `output/mod.rs::send_eager_tsf_warmup` — 投機的な eager warmup の
///    charset 選択（F1/F0 系を送らず常に F2）。
/// 2. `tsf/warmup/cold_warmup.rs::preamble()` — cold-start warmup の
///    charset 選択（`WarmupContext::charset`）。
/// 3. `output/probe_io.rs::transmit_tsf` — 実際の romaji バッチ送信時に
///    F1 leading warmup を前置するかどうかの判断。
///
/// `ConvModeMgr::get()`/`update_from_conv()` 自体は無変更 — 観測・ログ
/// （`[conv-mode]` 等）は通常通り継続する。この関数を経由する「実際に
/// charset に追従して動く」消費側だけを止める。
///
/// ## 観測すべきログ
///
/// - `[conv-mode] ... → ZenKata/roma` 等の観測ログ自体は出続けるはず
///   （観測は止めていないため）。
/// - `[tsf-eager-warmup] ZenKata warmup 送信` / `[h1-warmup] ... {ZenKata,
///   HankakuKatakana, ...} warmup+probe 送信` / `f1-leading` ログが
///   **一切出なくなる**はず（出た場合はこのフラグの配線漏れ）。
/// - 画面上で実際にカタカナが打鍵結果として現れる／GJI が持続的にカタカナへ
///   固定される場合 → 927f2a2/109b4c9 相当の退行が実際に起きている証拠
///   （ただしトレイでカタカナを手動選択しない限り想定していない）。
///
/// 実験終了後は `false` に戻すか、結果に応じて `docs/known-bugs.md` BUG-19 に
/// 実測を追記した上で撤去すること（`tuning-constants.md` の実測義務）。
pub const DIAG_FORCE_HIRAGANA_CHARSET: bool = true;

/// IME セッション内で literal-detect を初回のみ実行し、以降は候補ウィンドウ HIDE まで
/// スキップする診断フラグ（BUG-24 追補）。
///
/// ## 背景
///
/// `is_partial_literal()`（`tsf/warmup/literal_detect_fsm.rs`）は、今回送った romaji
/// 自身の確認信号（`DetectionResult::CompositionConfirmed` = 候補ウィンドウ SHOW /
/// GJI I/O 変化）ではなく、送信前に確定していた無関係な代理指標 `nc_fired`/`gji_resumed`
/// （別の F2 warmup キーへの応答有無）で判定している。`ColdReason::requires_settle()`
/// （`FocusChange`/`NativeF2Consumed`/`SetOpenTrue`）の直後は、`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`
/// 下でこの代理指標の元になる確認送信自体が無条件でスキップされるため、`nc_fired` は
/// 構造的に常に `false` になり、cold 直後の最初の1文字がほぼ確実に「正しく変換されて
/// いるのに不要な ESC+BS 訂正」を受ける（実機ログで確認、`docs/known-bugs.md` BUG-24）。
///
/// ## 方針（ユーザー提案）
///
/// 「IME セッション（打鍵開始から候補ウィンドウ HIDE まで）の最初の1文字だけ実際に
/// `CompositionConfirmed` を確認し、確認できたらそのセッションの残りは literal-detect
/// 自体をスキップして即送信する」という設計に変更する。反応速度を落とさず、かつ
/// 無関係な代理指標ではなく「今回のセッションで実際に compose が機能した」という
/// 直接の事実だけを判断材料にする。
///
/// `true` の間、`LiteralDetectCore::poll`（`tsf/warmup/literal_detect_fsm.rs`）は
/// `tsf::observer::literal_session_confirmed()` が `true` ならチェック自体を行わず
/// 即 `Done` を返す。`CompositionConfirmed`（かつ非 partial-literal）を確認できたら
/// `mark_literal_session_confirmed()` を呼ぶ。候補ウィンドウ HIDE
/// （`platform.rs::gji_on_end_composition`）で `reset_literal_session_confirmed()` を
/// 呼び、次のセッションの最初の1文字は改めて確認を受ける。
///
/// `cold_reason` の種類には依存しない — cold/warm 問わず、セッション内で一度
/// 確認済みかどうかだけで判定する。
///
/// ## 観測すべきログ
///
/// - `[literal-detect] cold=N partial literal ...`/`suspected literal ...` が、
///   同一セッション内の2文字目以降で発生しなくなるはず（1文字目のみ発生しうる）。
/// - 実機でカタカナ化やGJI応答遅延など、実際には literal 化しているのに検出漏れ
///   （偽陰性）が起きていないか目視確認が必要 — セッション判定の起点・終点
///   （HIDE のタイミング）がずれると、本来チェックすべき文字をスキップしてしまう
///   リスクがある。
///
/// 実験終了後は `false` に戻すか、結果に応じて `docs/known-bugs.md` BUG-24 に
/// 実測を追記した上で撤去すること（`tuning-constants.md` の実測義務）。
pub const DIAG_LITERAL_SESSION_SKIP: bool = true;

/// GJI cold-start warmup の予防的な F2 送信・`TsfReadinessProbe` 待機
/// （`ColdReason`×`long_idle` で決まる `eager_settle_ms`/`probe_min_ms` の行列、
/// `WarmupKind::FreshF2/ReWarmup/ProbeWithSettle` の分岐）を、それぞれ独立に
/// 実行中トグルできる実験用フラグ（BUG-24 追補、設計思想の転換に伴う簡素化実験）。
///
/// ## 背景
///
/// `cold_warmup.rs::run_start` は本来「GJI が composition を受け付け可能になる
/// まで待ってから送る」という設計（温めてから打鍵）のために、cold になった理由
/// （`ColdReason`）と直近の idle 時間に応じて待機時間を細かく作り分け、さらに
/// 送信前に予防的な F2 warmup を送っていた。しかし BUG-24 で「1文字ずつ送って
/// confirm し、ダメなら backspace で訂正する」per-VK confirm ループ
/// （`DIAG_LITERAL_SESSION_SKIP`）が入ったことで、送信が早すぎた/GJI がまだ
/// 温まっていなかった場合の保険は既にそちらが reactive に持っている。つまり
/// 予防的 F2 送信も `TsfReadinessProbe` の事前待機も、per-VK confirm と同じ
/// 保険を二重にかけているだけの可能性がある（ユーザー仮説、2026-07-16）。
///
/// ## 方針
///
/// 「温めてから打鍵」をやめ、「打鍵してダメなら都度直す」に一本化できるかを
/// 実機で試す実験。他の `DIAG_*` 定数と違いコンパイル時定数ではなく
/// `AtomicBool` にしてあり、トレイメニュー（`tray.rs` の「実験: cold warmup」
/// サブメニュー）から実行中に個別 on/off できる。「何をスキップして何を
/// スキップしないか」を実機で試行錯誤しながら選び、問題が出た方だけを
/// オフに戻せるようにするための構成（ユーザー方針、2026-07-16）。
///
/// - [`DIAG_COLD_SKIP_F2`]: `true` の間、`ColdWarmupSequence::run_start` は
///   F2 warmup を送らない。
/// - [`DIAG_COLD_SKIP_PROBE_WAIT`]: `true` の間、`TsfReadinessProbe` を
///   `min_ms=0`/`total_max_ms=0`（実質即座に release）で構築する。
///
/// 両方 `true` のとき、`preamble()` 完了後間を置かず romaji の per-VK confirm
/// （`DIAG_LITERAL_SESSION_SKIP` の `TransmitSingleVk` ループ）に進む。GJI が
/// hiragana composition を受け付けない場合、1文字目が `SuspectedLiteral` に
/// なり `emit_recovery_actions` の `StartSacrificialWarmup` 経路（TSF mode +
/// consecutive==0）が再確立を担う。`WarmupKind` 分岐・`eager_settle_ms`/
/// `probe_min_ms`/`RE_WARMUP_MS` は両方 `true` のとき未使用になるが、比較・
/// 切り戻し用にコードは残してある。
///
/// ## 既知の懸念（実験前から分かっている、実機で検証する）
///
/// - **チラつき**: F2 も待機もなしで即送信すると、GJI 側の準備が本当に整うまでの
///   間に一瞬 unconverted な表示が見える、あるいは1文字目が literal 化して
///   backspace 訂正が入るなど視覚的なチラつきが増える可能性がある。
/// - **速度の逆転**: 状況によっては「F2 を送ってある程度待ってから一括送信」の
///   方が「1文字ずつ confirm を待ちながら送る」per-VK ループより体感が速い
///   ケースがあるかもしれない（未検証）。
///
/// ## 観測すべきログ
///
/// - `[h1-warmup] cold=N DIAG_COLD_SKIP: skip_f2=.. skip_wait=.. ...`: この
///   トグルの組み合わせが実際に何回・どの組み合わせで通ったか。
/// - `[gji-coro] cold=N per-VK[...] suspected literal`: per-VK confirm の
///   backspace 訂正がどれくらいの頻度で発生するか（F2・待機を削った分、
///   増えるはず。増加が体感できるチラつき・タイプ遅延に繋がっていないか）。
/// - 画面上のチラつき・変換ミスの目視確認、および typing の体感速度。
///
/// 実験終了後はデフォルトを `false` に戻すか、結果に応じて `docs/known-bugs.md`
/// に実測を追記した上で `ColdReason` の待機行列を撤去すること
/// （`tuning-constants.md` の実測義務）。
pub static DIAG_COLD_SKIP_F2: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// [`DIAG_COLD_SKIP_F2`] を参照。`TsfReadinessProbe` の待機のみを独立にスキップする。
pub static DIAG_COLD_SKIP_PROBE_WAIT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);
