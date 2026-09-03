# ADR-123: `pending_deferred` の flush ガードが GJI reinit-retry 完了しか見ていないため、reinit 完了を待つ間に到着した別モーラが独立 probe で追い越し、deferred VK が確定済みセッションの後ろに取り残されて出力順が入れ替わる

## ステータス

**実装着手（2026-09-03）。** round 1・round 2（Opus 2体、architect/premortem）で
根本原因を確定させた後、「実装着手の条件」としていた確証データ
（`TsfProbeStarted.pending_deferred_len > 0` の実発生）が
report_id `01M1KEGZ081YHJ1T2NC765SYYH`（2026-09-03）で得られた。この報告を
受けて decision を Opus 敵対的レビュー3ラウンドで詳細化し、実装計画を確定した
（下記「2026-09-03 実装着手（round 3）」節）。

[GitHub issue #148](https://github.com/cuzic/awase/issues/148) として追跡。

### 2026-09-03 実装着手（round 3）: 確証データと decision の詳細化

不具合報告機能（ADR-095）経由、report_id `01M1KEGZ081YHJ1T2NC765SYYH`。
Windows Terminal + GJI(Google日本語入力) + TsfNativeで「github pages には
しないで」と入力したが「github pages sにはないで」となった（`docs/known-bugs.md`
BUG-74「2026-09-03 追補3」に詳細、当初`SuppressedExistingPoll`（ADR-101決定5）
由来と誤診断していたが、`KeyInput.timestamp_us`ベースで再検証した結果、本ADR
と同一の「`pending_deferred`追い越し」機序と確定した——訂正の経緯もBUG-74
追補3参照）。

journalで`TsfProbeStarted`の`pending_deferred_len`を確認したところ、
cold_seq=27時点で2、cold_seq=28時点で4（いずれも非ゼロ）——「実装状況」節が
待っていた確証データが得られた。

Opus敵対的レビュー3ラウンドで、当初案（`defer_if_probe_in_flight`の条件を
単純に3-way ORへ拡張するだけ）から以下の欠陥を発見・修正し、下記「決定
（確定・詳細化版）」へ収束した:

1. **retry/回収再送の吸い込み**: gateを`send_romaji_as_tsf`/`send_romaji_batched`
   の入口に置くと、`send_romaji_dispatching_on_gate`経由で呼ばれる
   `flush_raw_tsf_literal_romaji`（raw recovery回収再送）と
   `resend_gji_reinit_retry_romaji`（決定3のADR-101 retry）まで
   `!pending_deferred.is_empty()`項に吸い込まれ、ADR-101決定3の「retryは
   通常送信経路に戻してprobeに守らせる」という設計目的を破壊する
   （今回の事案では`len=4`のため確実に発火する）。→ gate免除の専用入口が必要。
2. **defer直後の無防備な生VK送出**: `send_keys`は末尾で必ず`OutputSession`を
   dropしdrainをpostする（`output/mod.rs:1291`）。probe/reinitどちらも
   in-flightでない（`!pending_deferred.is_empty()`項**のみ**が理由）状態で
   新規モーラを単純にdeferすると、同一メッセージループのdrainが
   `flush_stale_deferred_vks_after_recovery`経由でそのまま生VKとして即座に
   送出し、本来probeに守られるはずだったモーラが無防備な生VK送信に格下げ
   される。→ この場合はdeferではなく「先にキューをflushしてから新規モーラを
   通常送信する」（drain-before-send）にする。
3. **discard経路がユーザー生入力を新たに巻き込む**: gateを強めるほど
   `pending_deferred`に長く滞留するVKが増える。既存の
   `discard_raw_recovery_if_focus_stale`等はfocus不一致時に`pending_deferred`を
   丸ごと破棄する設計で、これは元々「awase自身が再送しようとしていたromaji」
   を対象にしていたが、gate拡張後は「ユーザーが実際に打鍵したがまだ送信
   されていない入力」も同じキューに乗る。→ `DeferredVk`に由来
   （`UserInput`/`RecoveryResend`）を持たせ、enqueue側の型シグネチャで
   `origin`を必須にする。
4. **上限超過時の退避行動**: 「最も古いエントリから強制flush」はprobeの
   per-VK confirmの最中に生VKを割り込ませることになり、BUG-38/本ADRが
   潰したinterleavingを再現する危険がある。→ 強制flushではなく「deferを
   諦めて新規モーラを通常送信する（＝今日の挙動へdegrade）＋`log::error!`」
   にする。

**関連する事実（記録漏れの補足）**: drainハンドラは`flush_raw_tsf_literal_recovery`
を先に呼び、その後でINPUT_DEFERをreplayする（`runtime/message_handlers.rs`）。
replay経路（`KeyOrigin::DeferredReplay`）はOUTPUT_GATEを再チェックしないため、
`Polling`中のguardは後続入力を守らない。また
`flush_deferred_vks_after_gji_reinit_completion`はretry再送自身が先に新しい
probeを立てる構造のため、実際には常に0件になる（決定4「retry→post-send effects
→deferred flush→guard drop」の記述と実態が食い違う——[ADR-101](101-bug74-giveup-retry-with-focus-guard.md)
決定4に訂正注記が必要）。

## 確定した根本原因（app.log 生ログのタイムスタンプ順再構成、`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`）

以下は `wrangler r2 object get` で取得した report JSON の `payload.app_log_excerpt`
（`log::` 出力そのもの、journal とは別フィールド）を時刻順に読んだ結果で、
推測を含まない一次証拠である（該当行はタイムスタンプ `02:42:53.606`〜
`02:42:54.129`、約523ms の窓）。

1. **`02:42:53.668`** 「た」の1文字目('T', cold=235) が per-VK confirm で
   `suspected literal` → backspace ×1 + 再送 "ta" が予約される
   （`[raw-tsf-literal] cold=235 raw TSF literal suspected → backspace ×1
   + re-send "ta" scheduled`）。
2. **`02:42:53.670`** probe(442→443) の切替中、drain キュー（`queue_len=6`）
   がまとめてリプレイされる。backspace 送信 → "ta" 再送（cold=236 の新しい
   probe443 を開始）。
3. **`02:42:53.680`**「と」が engine を通過し `send_keys` で romaji "to"
   に変換されるが、**probe443 が in-flight のため** `[tsf] probe in flight
   → deferred 2 VK(s) for "to"` — **`pending_deferred` に T,O の2VKが積まれる**。
4. **`02:42:53.681`**「え」も同様に engine を通過し romaji "e" に変換される
   が、probe443 がまだ in-flight のため `[tsf] probe in flight → deferred
   1 VK(s) for "e"` — **`pending_deferred` に E が追加され、計3VK
   （T,O,E）が滞留する。**
5. **`02:42:54.006`**「た」の再送（cold=236）も per-VK confirm で
   **2回目の** `suspected literal`（`consecutive=2`）→ **give-up**
   （`giving up, backs=1 cleanup only (no re-send)`）。
6. **`02:42:54.007`** probe443 が終了（`ProbeDone`）→ backspace ×1 送信。
7. **`02:42:54.013`** **`[chrome-reinit] cold=236 VK_IME_OFF→VK_IME_ON
   強制リセット送信 + IMC ポーリング開始`** — give-up は実際に GJI reinit
   を送信していた（round 1 版の「reinit 不発火」判定は誤り、round 2 の
   訂正が正しかったことが確定）。
8. **`02:42:54.019`** **`[raw-tsf-literal] skip stale deferred flush while
   GJI reinit retry is polling: result=StartedRetryPolling { poll_token: 7 }`**
   — reinit のポーリングが終わるまで、`pending_deferred`（T,O,E の3VK）の
   flush は意図的にスキップされる。**この時点で `discard_raw_recovery_if_focus_stale`
   は発火していない**（focus は変化していない）。round 2 の「focus churn で
   全破棄」仮説（第一仮説としていた `discard_raw_recovery_if_focus_stale`）は
   **本インシデントでは不成立と確定した**。
9. **`02:42:54.019`〜`.022`**「ば」の物理キーが drain される
   （314/278/181/158ms の追加遅延を伴う、事実2で測定した遅延バーストの
   正体）。
10. **`02:42:54.022`**「ば」が engine を通過し romaji "ba" に変換される。
    **ここで `[tsf] probe in flight → deferred` ログが出ない** ——
    probe443 は既に `ProbeDone` で終了しており `has_pending_tsf()` が
    false のため、「ば」は deferred されず**新しい独立 probe444（cold=237）
    を開始する。**
11. **`02:42:54.083`** probe444 で「ば」の1文字目('B') が
    `candidate SHOW` を伴って正常に confirm される。
12. **`02:42:54.086`** reinit のポーリング（手順7で開始）が完了
    （`IMC poll #1: conv=0x00000019 NATIVE=true write_delta=+315B` →
    `status=Confirmed`、`origin_focus_gen=431 == current_focus_gen=431`）。
    完了トリガーで **「た」の再送が `retry romaji via normal path: "ta"`
    として発行される**（3回目の "ta" 送信）。
13. **`02:42:54.086`〜`.089`** この「た」再送は、**ちょうど今 in-flight な
    probe444（元は「ば」用）に相乗りする形**で `vks=[54,41]`（'T','A'）
    として送信・confirm される（`[tsf-transmit] cold=237 romaji="ta" →
    vk-run`）。
14. **`02:42:54.103`**「ば」の2文字目('A') も confirm され、
    `per-VK: 全 2 VK 確認済み → セッション確認`（session_marked）。
    **この直後**、`[tsf-probe] deferred 3 VK(s) を romaji 直後に送出 (Tsf)`
    ——手順3-4で滞留していた `pending_deferred`（T,O,E）が、**「ば」と
    「た」(3回目)がどちらも確定した後に**、ようやく flush される。

**確定した機序**: 「た」の give-up が予約した GJI reinit のポーリング完了を
待つ間、`pending_deferred`（と・え、3VK）は正しく flush を保留された
（手順8、これ自体は意図通りの安全策）。しかし**この保留期間中に到着した
「ば」は、`pending_deferred` の存在を一切考慮しない別経路（`has_pending_tsf()`
のみを見る `defer_if_probe_in_flight`）を通り、独立した新しい probe を
起動して先に確定してしまう**。さらに reinit 完了トリガーで「た」の再送が
その「ば」の probe に相乗りして追加確定される。結果、`pending_deferred` に
先に入っていた「と」「え」が、後から来た「ば」「た」に**追い越され**、
最終的な flush（手順14）は両者が確定した後になる。**この「追い越し」こそが
文字脱落・順序入替の直接原因であり、GJI 内部の解釈やモーラ融合の推測は
不要だった。**

出力の最終形（「ばたと」、「え」消失）は、この追い越し順序（ば→た→と+え）と
整合する。「え」が完全に消えた理由は本ログ窓の外（`02:42:54.129` 以降）に
あると見られ、`pending_deferred` の flush（手順14、raw な生 VK 送信で probe
を経由しない）が、既にセッション確認済みの composition に対してどう作用
したかは GJI 内部の挙動に依存し確定できないが、**出現順序そのものはこれで
完全に説明できる。**

### round 1/2 の各仮説との決着

| 仮説 | 判定 |
|---|---|
| round 0: 「え+ば が `pending_deferred` で融合し単一の巨大合成イベントになった」 | **不成立（確定）**。融合ではなく「追い越し」。write_delta=315 は「ば」自身の confirm 値（正しい解釈だった、`[chrome-reinit] cold=236 ... write_delta=+315B` のログは実は reinit の IMC polling 側の値であり、`LiteralDetect` の write_delta=315 とは別物と判明——両者はたまたま近い値だが無関係） |
| round 1 F-1: cold-start は word-level `LiteralDetectFsm` ではなく per-VK confirm | **確定**（`path=PerVk, target=Tsf`） |
| round 2 R2-2: 「journal に reinit イベントが無い」は false negative | **確定的に正しかった。reinit は実際に発火した**（手順7） |
| round 2 R2-4: `discard_raw_recovery_if_focus_stale` が第一仮説 | **不成立と確定**。focus_gen は変化しておらず discard 経路は通っていない |
| round 2 R2-3: 「deferred flush は『ば』の probe 完了より後に来る」 | **確定（architect の予測が正確に的中）**。ただし機序は「backspace/reinit 待ちで単純に遅延した」のではなく「`pending_deferred` の存在を無視して『ば』が追い越した」 |

## 確定した真因

**`defer_if_probe_in_flight`（`output/mod.rs:1277-1290`）およびそれに準ずる
gating が `has_pending_tsf()`（TSF probe オブジェクトの生存）だけを見ており、
`pending_deferred` が非空である（＝まだ flush されていない先行モーラの VK が
存在する）ことを考慮しない。** そのため、probe が一旦終了（give-up 等で
`ProbeDone`）した後、その `pending_deferred` がまだ flush されていない
（reinit retry 等の後続処理待ちで保留中の）状態でも、次に届いたモーラは
「probe は in-flight ではない」と判定されて即座に独立した新しい probe を
開始でき、結果として `pending_deferred` の中身を追い越して先に確定する。

### 関係ファイル・関数一覧（確定版）

| 役割 | file:line |
|---|---|
| gating の欠落箇所（`has_pending_tsf()` のみで `pending_deferred`/`raw_recovery_owns_deferred()` を見ない） | `crates/awase-windows/src/output/mod.rs:1277-1290`（`defer_if_probe_in_flight`） |
| `pending_deferred` 実体 | `crates/awase-windows/src/output/tsf_warmup_coord.rs:51-57,296-337` |
| `raw_recovery_owns_deferred`（reinit 予約中かの判定、既存だが `defer_if_probe_in_flight` からは参照されていない） | `crates/awase-windows/src/output/mod.rs:1400-1417` |
| give-up → GJI reinit 予約・ポーリング | `crates/awase-windows/src/output/probe_io.rs:766-782`, `crates/awase-windows/src/output/probe_io.rs:167-` |
| reinit ポーリング完了 → romaji 再送のトリガー（既存 probe への相乗り） | `chrome-reinit-retry completion` 経路（`platform.rs` 内、ログ行 `[chrome-reinit-retry] retry romaji via normal path`） |
| deferred flush 実行（`[tsf-probe] deferred N VK(s) を romaji 直後に送出`） | `crates/awase-windows/src/output/key_injector.rs:302-313` |
| TSF cold-start の per-VK confirm 経路 | `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs:172-181` |

## BUG-38 との異同

**BUG-38 とは異なる、しかし近縁の欠陥。** BUG-38 は「同一 probe の give-up
サイクル内で `pending_deferred` を flush し忘れる」ことを塞いだ。本件は
「`pending_deferred` が flush 待ちの間、**別の独立した send がそれを一切
考慮せず追い越せる**」という、flush 忘れとは異なる gating の抜け穴。
BUG-38 の修正（`flush_stale_deferred_vks_after_recovery`）自体は正しく
機能している（手順14で実際に flush されている）。抜けているのは
「flush されるまでの間、新規 send を止める」側のガードである。

## 実装状況（2026-09-03）

**ユーザー判断により、本 decision（`defer_if_probe_in_flight` の gating 拡張）
自体の実装は今回見送り、次に挙げる診断ログの計装のみを先行実装した。**
理由: 根本原因は「た/ば/と/え」1件のインシデントの forensic 再構成から
導いたものであり、再発時にこの仮説を機械的に確定/反証できる一次データが
まだ整っていない。再発を待ち、下記フィールドで仮説を確定させてから
decision を実装する。

実装したフィールド（すべて既存の journal 構造化イベントへの追加、または
これまで `log::debug!`/`log::warn!` の自由文字列でしか残らず journal
（構造化・容量優先度あり）には一切現れなかった事実の新規構造化）:

- `JournalEntry::TsfProbeStarted` に `probe_id: Option<u64>`（`cold_seq` との
  混同を解消、round 2 architect 指摘の修正）と `pending_deferred_len: usize`
  （**本件の核心**: 新しい probe が `pending_deferred` を追い越して開始した
  ことを直接示す。非ゼロなら次のインシデントでこの仮説が即座に確定する）
  を追加。
- `JournalEntry::DeferredRecoveryFlush`（新設）: `pending_deferred` の
  flush/discard/skip の3分岐（`RawRecoveryOutcome`）を構造化。
- `JournalEntry::GjiReinitRetryCompleted`（新設）: reinit retry 完了時の
  `focus_matches`・flush/discard 件数を構造化。

次回同種のインシデントが再発した際は、journal（`app_log_excerpt` を別途
読まなくても）だけで「`TsfProbeStarted.pending_deferred_len > 0` のケースが
実際に発生したか」を機械的に確認できる。これが確認できれば decision の
実装に進む。関連ファイル: `crates/awase-windows/src/journal.rs`,
`crates/awase-windows/src/output/mod.rs`,
`crates/awase-windows/src/output/tsf_warmup_coord.rs`,
`crates/awase-windows/src/platform.rs`。

## 決定（確定・詳細化版、2026-09-03 round3）

`defer_if_probe_in_flight`（および同種の gating 判定）の条件を
`has_pending_tsf() || raw_recovery_owns_deferred() || !pending_deferred.is_empty()`
に拡張する、という基本方針自体は当初案から変えていない。round3 の Opus
敵対的レビュー3ラウンドで発見された4つの欠陥（上記「2026-09-03 実装着手」節）
を踏まえ、以下の5変更として実装する。**依存関係上、実装順序は
D→E→B→(A+C) を厳守すること**（B は A より前に型で縛る必要があり、C は A と
不可分——理由は各項目参照）。

### 変更D（診断・挙動不変、最初に出す）

`JournalEntry::DeferredRecoveryFlush` が `vk_count:0` の無情報エントリを
大量に記録している（実測: 294件中293件）。`vk_count>0` または非`Flushed`の
ときだけ記録するようにする。挙動は変えない。以降の変更A/Cの効果測定基盤にする。

### 変更E（診断・挙動不変、Dの直後）

`pending_deferred` に単調増加の順序トークンを持たせ、flush 時に「投入順と
flush 順が食い違った」ケースを検出する（選択肢Eの最小版）。**ログレベルは
`log::warn!`**にすること（`log::error!`ではない）——変更A+C（gate拡張・
drain-before-send）が未実装の現状では、この順序違反は実際に頻発する
既知の状態（「確定した根本原因」節参照）であり、`error!`にすると
不具合報告機能が収集する`app_log_excerpt`（診断に使う当のログ）が
ノイズで汚染される。変更A+C（同一PR）がマージされた後、`error!`へ
昇格する（round3・round4のOpus敵対的レビューで確定した順序）。
本decision実装後も「万一gatingに見落としがあった場合の安全網」として
軽量に併設する。

### 変更B（型強制、AとCより前）

`DeferredVk` に由来（`origin: DeferredOrigin::UserInput | RecoveryResend`）を
追加する。**型強制は enqueue 側**（`defer_vks_if_in_flight`／新 gate の引数）
に置き、`origin` を必須引数にする——これにより「B抜きのA」（origin なしで
gate を通す実装）がコンパイル不能になる（discard 側に必須化しても enqueue
経路を縛れないため誤り、round3で訂正済み）。この時点では `origin` は
記録のみで挙動を変えない。

### 変更A+C（同一PR、分割不可）

同一PRにまとめる理由: Cの計数上限がないと、A単独でgateを強めた際に
`pending_deferred`の滞留量が青天井になり、下記「上限超過時」の安全弁が
機能しないため（round3指摘）。

- **gate条件の3-way OR拡張**: `defer_if_probe_in_flight`
  (`output/mod.rs:1311`) と `tsf_warmup_coord.rs:296-299` の
  `defer_vks_if_in_flight` 内部の早期return、**両方**を同時に緩める
  （片方だけだと無言でno-opになる、round3指摘）。
- **配線箇所**: `send_romaji_as_tsf`/`send_romaji_batched` の入口
  （`assess_warmth()` による warm/cold 分岐**より前**）にgate判定を置く。
  現状 `defer_if_probe_in_flight` は `prepend_f2_warmup` 分岐と
  `ms_ime_gate_defer` からしか呼ばれず、warm判定されたモーラは
  `send_romaji_as_tsf_warm`(`vk_send.rs:403`)へ直行してgateを一切通らない
  ——この穴を塞ぐ。`vk_send.rs:369`の`ms_ime_gate_defer`は
  `needs_f2_probe()==false`で早期returnするためGJI専用の`pending_gji_reinit`
  条件を足しても実質デッドコード、触らない。
- **gate免除入口**: `send_romaji_dispatching_on_gate`(`output/mod.rs:1648`)
  経由で呼ばれる `flush_raw_tsf_literal_romaji`（raw recovery回収再送）と
  `resend_gji_reinit_retry_romaji`（ADR-101決定3のretry）は、3-way OR gate
  の対象から恒久的に除外する専用の送信入口を新設し、そちらを呼ぶ（下記
  欠陥1の対策。`origin`分離とは別軸、gateに入るか自体を制御する）。
- **drain-before-send**: `has_pending_tsf()`/`raw_recovery_owns_deferred()`
  がどちらもfalseで`pending_deferred`が非空**なだけ**の場合、新しいモーラを
  追加でキューに積むのではなく、先に`pending_deferred`を（既存のflush経路で）
  flushしてから、新しいモーラを通常どおり（probe保護付きで）transmitする。
  flushは`assess_warmth()`より**前**に固定すること（後に置くとwarm/cold判定が
  flush前の状態で下され、直後のprobeが汚染された`last_send_ms`/write_delta
  を自分の証拠として読みうる、round3指摘）。（下記欠陥2の対策）
- **件数上限**: `pending_deferred`の所有者である`TsfWarmupCoordinator`が
  件数上限を保持する。**上限超過時は「最も古いエントリから強制flush」では
  なく「deferを中止し新規モーラを通常送信する（＝今日の挙動へdegrade）＋
  `log::error!`」**にする（強制flushはprobeのper-VK confirm中に生VKを
  割り込ませることになりBUG-38/本ADRが潰したinterleavingを再現する危険が
  あるため、round3で強制flush案を却下）。時間(ms)上限は別途計装してから
  実測に基づき別PRで導入する（`tuning-constants.md`の実測義務対象、
  件数上限は`_MS`定数ではないため対象外）。（下記欠陥4の対策）

上記4欠陥（吸い込み/無防備送出/discard巻き込み/上限超過時の危険な退避）の
詳細は「2026-09-03 実装着手（round 3）」節を参照。

- **選択肢D/A（round 2 で却下）との関係は不変**: probe連鎖の抑制や
  `SendInput`分割は不要——`pending_deferred`に積む条件と送信タイミングを
  直すだけなので、round 2 で懸念された「単一 SendInput のメッセージポンプ
  保護喪失」（BUG-02 系 race 再燃リスク）を新設しない。
- **短所/リスク（残存）**: gate拡張により、後続モーラの入力が今まで以上に
  長く待たされる可能性がある。件数上限は導入するが、時間上限は実測後の
  別PRになるため、上限に達するまでの待ち時間そのものは本PRの範囲では
  未計測のまま残る。

## 未決定事項

1. **実装後のレイテンシ影響の実測**: gating 強化で後続モーラの待ち時間が
   伸びるケースがどの程度あるか。
2. **`FOCUS_RESYNC` gate が2打鍵目で事実上解除される件**（round 1 F-2）は
   本ADRのスコープ外の別欠陥として引き続き切り出しを推奨。
3. **`discard_raw_recovery_if_focus_stale` 自体の経路**（本件では不発火
   だったが実在する）は、focus churn が実際に reinit retry 中に起きる
   別インシデントで別途検証が必要——変更Bの`origin`分離は最小実装(b)
   （記録のみ、破棄そのものは止めない）に留めるため、本 decision の gating
   拡張後も `pending_deferred` ごと破棄されるケース自体は残る（根絶ではなく
   緩和）。`UserInput`由来を破棄せず再送を試みる案(a)は別PRの検討課題。
4. Unicode 側の同型フラットバッファ2箇所（`UnicodeColdWarmupFsm::deferred_chars`、
   `Output::unicode_cold_deferred`）は「同型だが未観測」として
   `docs/known-bugs.md` に別記録し、本ADRのスコープからは外す。
5. **回帰テストの置き場所**: `defer_if_probe_in_flight` は純粋関数に近い
   判定ロジックを含むため、`output/tsf_warmup_coord.rs` の既存ユニット
   テスト、または `.claude/rules/fix-requires-evidence.md` に従い
   `crates/awase-windows/tests/golden_scenarios.rs` への追加を検討する。

## 関連

BUG-38、BUG-89（`FOCUS_RESYNC`/`OUTPUT_GATE` の deferred replay 経路）、
BUG-45/BUG-75/ADR-122（per-VK confirm の代理指標ベース判定の構造的欠陥）、
[docs/bug-reports-triage.md](../bug-reports-triage.md)
（`01M1JJD54XQXSEJTHHFKV1WKA1`・`01M1KEGZ081YHJ1T2NC765SYYH` 該当行）、
[ADR-101](101-bug74-giveup-retry-with-focus-guard.md)
（決定3/決定4/決定5、gate免除入口が保護する対象）、`docs/known-bugs.md`
BUG-74「2026-09-03 追補3」（本件の当初誤診断とその訂正）。
