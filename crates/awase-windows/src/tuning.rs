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
/// `GjiFsm::long_idle_ms_for(InjectionMode::Vk)` が参照し、`ColdKind::classify` の
/// Short/Medium/Long 重症度分岐（cold-start warmup の経路選択に使う）の cutoff になる。
///
/// 予防的な Chrome プローブ最小待機の延長（20ms→200ms）機構自体は 2026-07-18 に
/// 撤去した（`docs/known-bugs.md` BUG-24 参照、per-VK confirm に一本化）。この定数の
/// 元々の実測根拠（idle=6312ms 後に Chrome TSF の composition context 再初期化に
/// ~145ms かかった事例, cold=1040）は撤去された機構向けだったが、値自体は
/// `ColdKind` 分岐の cutoff として引き続き使われている。
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

/// 候補ウィンドウ可視 veto の上限保留時間 (ms)。
///
/// `LiteralDetectCore::poll` が `SuspectedLiteral`（`RAW_TSF_LITERAL_DETECT_MS` 系の
/// deadline 到達）を検出した時点で GJI 候補ウィンドウがまだ可視の場合、backspace を
/// 出さず hold する（可視である以上ほぼ確実に compose 成功しているため、消すと
/// BUG-27 追補5 と同型の regression になる）。この定数はその hold の上限であり、
/// 超過しても backspace はせず無回収の `Done` で打ち切る（候補ウィンドウが固着した
/// 異常系でタイマーが永久に止まらないための安全弁）。
///
/// **実測未了 — 暫定値**: 「候補ウィンドウ可視 → I/O/SHOW 確定」までの実測遅延データが
/// まだ無い。300ms は `CHROME_GJI_REINIT_CONFIRM_MS`（IME ON→NATIVE確認 300ms）等、
/// 同程度の「確認待ち」定数から類推した仮値であり、`tuning-constants.md` が要求する
/// 実測根拠を満たしていない。実機（Windows, Chrome/Teams/WezTerm 等）で計測してから
/// 本番投入すること。
pub const GJI_CANDIDATE_VETO_CAP_MS: u64 = 300;

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
///
/// BUG-33（2026-07-22）: `probe_io.rs` の `RawTsfLiteralRecovery` give-up 分岐
/// （per-VK confirm が2連続で literal 化を検出した場合）からも `send_chrome_gji_reinit_and_poll`
/// を呼ぶようになった。この窓は同時に「連続 give-up による reinit 多重発火」のレート制限
/// （`Output::last_gji_reinit_ms`）にも使われる。
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

/// `shift-conv-guard`（BUG-15）の hold 終了（復元開始）ごとに confirm-then-transmit
/// ゲート（BUG-13、`Output::confirm_gate_deadline_override_ms`）へ与える猶予 (ms)。
///
/// `MS_IME_READY_CONFIRM_MS`（400ms）を流用しないこと — あれは IME OFF→ON 遷移の
/// 実測値であり、この復元リトライループとは別の現象を測ったものである
/// （`.claude/rules/tuning-constants.md`「同じ定数ファミリーの盲目的エスカレーション」
/// 参照）。
///
/// この値は「復元リトライが続いている限り `kp_restore_kana_from_half_width` の
/// 各試行の冒頭で毎回押し出される」設計（同関数参照）の **一区間ぶんの猶予**
/// であり、リトライ全体の合計所要時間（0/160/320/480ms、最大 ~960ms）をカバー
/// する単発の待ち時間ではない。したがって導出根拠は「復元が始まってから完了
/// するまでの合計時間」ではなく「1 回の試行が最大でどれだけかかりうるか」:
///
/// - ADR-086 INV-14（2026-08-08 追記）: `set_ime_conv_for_target` の
///   `verify_still_current` が書き込み直前に `get_focused_hwnd_async`
///   （`get_gui_thread_info_with_timeout`、30ms タイムアウト）を1回はさむ
///   = 最大 ~30ms。
/// - `set_ime_conv_for_target`（内部で `set_ime_romaji_mode_for_hwnd` を呼ぶ）は
///   IMC write が最大2回（`ime.rs` の `send_ime_control` 呼び出し、各 50ms
///   タイムアウト）= 最大 ~100ms。
/// - 続く `RETRY_INTERVAL_MS`（160ms）の sleep。
/// - 続く conv 読み取り（`get_ime_conversion_mode_raw_timeout`、10ms タイムアウト）。
///
/// 1 試行の最大所要 ≈ 30+100+160+10 = 300ms（実務上の見積り上限 ~310ms）に対し、
/// 800ms は次の試行が確実に override を再度押し出す前に期限切れしないための
/// マージン（約 2.7 倍）である。ADR-086 移行前の見積りは verify の 30ms を含まず
/// 270ms だったが、マージン比率が十分に大きい（2.7 倍）ため 800ms 自体は
/// 実測なしに動かしていない（`.claude/rules/tuning-constants.md` 準拠）。
/// MS-IME の Shift 単独タップ誤切替そのものの
/// 実測タイミング（shift up 後 ~478ms 後の idle-conv-check で観測、
/// `docs/known-bugs.md` BUG-15 参照）は `MAX_TRIES`（4 回）× `RETRY_INTERVAL_MS`
/// を決める根拠であり、この定数の根拠ではない（Opus pass-5 レビュー指摘: 旧版の
/// コメントは 478ms/960ms を根拠として引用していたが、ループが自己延長する
/// 設計に変わった後はそれらは無関係な数値になっていた）。
pub const SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS: u64 = 800;

/// `shift-conv-guard` の entry（Shift 押下、`kp_shift_conv_guard_key_down`）で
/// confirm-then-transmit ゲートを実質的に無期限へ延長する代わりに使う、有限の
/// 安全キャップ (ms)。
///
/// 実測値ではなく安全側マージン: 通常の hold（チョード確定・単独タップ確定を
/// 問わず Shift 押下から解放まで）は実機ログで ~620ms 程度（BUG-49 known-bugs.md
/// 参照）。Shift の KeyUp が何らかの理由でフックに届かない場合（ロック画面・
/// セキュアデスクトップ遷移等、`project_ctrl_mismatch_stuck_modifier` に記録の
/// ある stuck modifier の既知シナリオ）でも、`u64::MAX` のような真の無期限
/// ではなくこの上限を過ぎれば通常の安全弁（IMC 未確認なら give-up latch）へ
/// 自動的に復帰する。通常の hold 所要時間（~620ms）に対して十分大きく、かつ
/// 「固着したまま気づかれない」時間を有限に抑えることを優先した。
pub const SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS: u64 = 5_000;

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

/// `PHYSICAL_KEY_STATE[VK_LWIN/VK_RWIN]` が「押されたまま」と信頼できる最大保持時間 (ms)。
///
/// これより長く「押されたまま」の値が続いている場合は、KeyUp が
/// `WH_KEYBOARD_LL` フックチェーンの前段（シェル/検索UI側の低レベルフック等、
/// 推測）で消費され awase に届かなかった stale な状態とみなし、
/// `win_key_held()` は「押されていない」として扱う（2026-08-06 実機、
/// Win キー押下で検索UIが開いた際に KeyUp が失われ `VK_IME_ON/OFF` の実送信が
/// 恒久的にスキップされ続けた不具合の対策）。
///
/// **未実測**: 実機での Win キー保持時間の分布は未計測。人間が Win+何かの
/// チョードを行う際の保持時間は通常数百ms 以内で完了するという定性的な
/// 推論に基づく暫定値。実機ソークでの調整余地がある。
pub const WIN_KEY_HELD_STALE_MS: u64 = 2_000;

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

// === IntentStore（ADR-087 §2.3 P15 / §4 INV-24） ===

/// `IntentStore` に記録された **ON 意図**の保持窓 (ms)。
///
/// この時間を超えると、対象への明示 ON 意図は `issue_open_warrant()` の
/// Step 1 から外れる（Step 4 の既定推測にフォールバックする）。
///
/// **未実測・暫定値**: 既存の `EXPLICIT_OFF_CACHE_SUPPRESS_MS`
/// （`runtime/focus_tracking.rs`、Windows専用コードのため本ファイルには
/// 移設していない。ADR-087 §4 INV-24(a) が将来の統合を求めている）と
/// 同じ 10 秒を仮に採用した。ON/OFF で TTL を非対称にする理由は
/// `EXPLICIT_OFF_INTENT_TTL_MS`（下記）を参照。値を変更する場合は
/// `.claude/rules/tuning-constants.md` に従い実測根拠を示すこと。
pub const EXPLICIT_ON_INTENT_TTL_MS: u64 = 10_000;

/// `IntentStore` に記録された **OFF 意図**の保持窓 (ms)。ON より意図的に
/// 長く取る（ADR-087 §4 INV-24(a)、§7 round4 M-A）。
///
/// Step 4（`HeuristicGuess`/`OwnSsot`）の既定推測は観測ゼロのとき ON 方向に
/// のみバイアスを持つ。そのため ON 意図の失効は Step 4 と同じ結論になり
/// 実害が薄いが、OFF 意図の失効は Step 4 が正反対の結論を出す（round3
/// シナリオ7/9）。round3 時点では「OFF は無期限（TTL なし）」としていたが、
/// round4 の Opus レビューで「対象ごとに永続する `IntentStore` では、
/// フォーカス単位で有界だった旧 `last_intent` と違い、無期限は
/// drift correction が永久に再同期できない固着を作る」と指摘された
/// （実 precedent: `HwndImeCache`（`focus/hwnd_cache.rs`）は
/// `HWND_CACHE_MAX_AGE_MS` で必ず期限を切っている）。
///
/// この定数が答えるべき問いは「明示意図はどれだけ長く有効か」ではなく
/// 「`last_intent` を消すフォーカス断絶（奪取→復帰）のギャップを何秒まで
/// カバーするか」である（2026-08-11 BUG-51 追補 v3、pre-mortem #2 で再定義）。
/// 実際に観測された断絶は sub-second〜数秒のオーダー: BUG-57 の Pushbullet
/// 通知による奪取（sub-second）、スリープ復帰直後のフォーカス再構築（数秒）。
/// 既存の同種判断 `EXPLICIT_OFF_CACHE_SUPPRESS_MS`（`runtime/focus_tracking.rs`、
/// 10秒 = 「明示 OFF をフォーカス遷移からどれだけ保護するか」の precedent）と
/// `EXPLICIT_ON_INTENT_TTL_MS`（10秒）に対し、OFF 側は非対称に3倍の 30秒とし、
/// 観測オーダー（数秒）に対して十分なマージンを持たせる。
///
/// 当初 `HWND_CACHE_MAX_AGE_MS`（1時間）を転用していたが、`IntentStore` が
/// `effective_open()` から実際に読まれるようになると、誤記録・stale 化などの
/// あらゆる失敗モードの最悪持続時間そのものになるため、30秒へ短縮した。
/// なお `HwndImeCache`（`(pid, class)` キー、`HWND_CACHE_MAX_AGE_MS`=1時間）は
/// `IntentStore` とは別経路として残る（`docs/known-bugs.md` BUG-51 追補の
/// 残存リスク参照——`effective_open()` の結果を洗浄済みの値として保存し、
/// `HwndCacheRestored` で `desired_open` へ再注入するため、この30秒 TTL の
/// 外側で最大1時間 IntentStore 由来の値が生き残る経路が別途存在する）。
pub const EXPLICIT_OFF_INTENT_TTL_MS: u64 = 30_000;
