# ADR-269 / PR #686 最終確認(round 5)

対象:
- 依頼の HEAD は `4db6d890`。レビュー中に PR の head は `f55ac459`(`origin/develop` のマージ)に進んだ。
- マージで入ったのは ADR-268 D5 の 10 ファイル(`mozc_shm.rs`・`mozc_shm_shadow.rs`・`message_handlers.rs`・`tuning.rs`・`lib.rs`・`architecture_guard.rs`・docs)。`gji_monitor.rs`・`tip_detector.rs`・`converter_target.rs` には触れていない(`git diff 4db6d890 f55ac459 --stat`)。
- コードの判断は `4db6d890` のまま有効。

## 自分で確かめたこと

run 39183922616(`d57b9d38`、修正 `a575de33` を含む)の `ts-tsf-mozc-switch-*` 4 本の awase.log を数え直した。

- `fresh-thread query` は 138 回、すべて `ok`。内訳は 0ms 37・1ms 94・2ms 3・3ms 2・4ms 2。中央値 1ms・最大 4ms は ADR と一致した。p95 は 131 番目が 1ms なので「1〜2ms」が正確(実害なし)。
- **detach の後の `[h1-send] gji_idle=` は 185921〜574734ms**(起動からの tick と同じ桁で、`gji_last_io_ms=0` を意味する)。detach の前は 47・62ms。0 に戻す処理が実際に効いたことの直接の証拠になる。ADR の「直接の証拠は未確認」は「確認済み」に直せる。
- PR の状態(`gh pr view 686`、2026-10-19T01:28 時点): `mergeable=MERGEABLE`、`mergeStateStatus=UNSTABLE`。`f55ac459` の CI(39185520961)・e2e-ime(39185519205)・smoke(39185521005)は実行中。

---

## (1) round 4 の 1-1〜1-4 の修正

### 1-a. 接続したときの書き戻しの撤去(1-1): 正しい
- 種別・識別が変わるのは定期のポーリングのデバウンスだけになった。
- 撤去で失った機能は「接続した直後にすぐ種別を確かめる」こと。種別は接続とは独立にポーリングが決めるので、遅れても最大 2 秒。問題は無い。

### 1-b. 取れない tick を飛ばす(1-2): 正しい。常に失敗する環境の副作用は「develop と同じ」
- `tsf_ctx` が None(COM が使えない): 以前と同じく、ポーリングの塊そのものを通らない。起動時の値も無い。
- `tsf_ctx` は取れたのに新しいスレッドの問い合わせだけが常に失敗する: 起動時の問い合わせ(`gji_monitor.rs:351`、古い mgr)の値のまま固定される。D1a の発見では、develop の古い mgr も起動後の切り替えに追随しなかった。なので**実効的には develop と同じ**で、悪化はしない。D1 の「起動時から Mozc」の経路はこの環境でも効く。効かなくなるのは「起動後の切り替え」だけ。
- 足りないのは気づく手段。失敗は 2 秒ごとの debug ログ(`no observation`)しか残らない。連続 N 回(例: 5 回 = 10 秒)失敗したら 1 回だけ warn を出すと、利用者の報告で「固定されている」と分かる。推奨。

### 1-c. `run_with_timeout` 300ms(1-3): 共有の孤児スレッド枠を使っている(中、推奨)
- `run_with_timeout` は既定の共有プール `LEAKED_THREADS`(上限 8)を使う(`crates/win32-async/src/thread_timeout.rs:89-95`)。このプールはフォーカス分類の IMM32・MSAA・UIA の呼び出しと同じ枠。
- TSF が詰まって問い合わせが毎回時間切れになると、2 秒ごとに 1 本ずつ孤児が増え、約 16 秒で枠が埋まる。以後は**フォーカス分類の `run_with_timeout` も `refusing to spawn` で全部 None** になり、IME の種類を問わず全利用者のフォーカス・IME の判定が劣化する。
- 同じファイルの `run_with_timeout_in` の doc(`:97-102`)が、まさに「用途の異なるブロッキング呼び出しが同じ枠を奪い合わないよう、専用の `LeakedThreadPool` を渡せ」と書いている。
- 直し方: `static TIP_QUERY_POOL: LeakedThreadPool`(上限 1〜2)で `run_with_timeout_in` を使う。枠が埋まっている間は None(tick を飛ばす)になり、1-b と同じ扱いに落ちる。CI の実測(最大 4ms)では起きていないが、起きたときの波及が全 IME に及ぶので推奨にした。
- 細かい点(低):
  - 時間切れで孤児になったスレッドは、後で `query_active_kind` の中の `TSF_OBS.set_ime_product_name` を遅れて書く。BugReport の製品名が古い値で上書きされうる(診断のみ)。
  - `run_with_timeout` の doc の「クロージャ内では COM を呼び出せない」は「他スレッドで作った COM」の意味。クロージャの中で作って解放する今回は当てはまらない。そう 1 行コメントに書いておくと、次の人が規約違反と誤読しない。
  - 300ms の待ちの間は I/O の標本化が止まる。実測は最大 4ms なので、上限を 50ms 程度に下げても問題なく、per-VK の判定窓(約 300ms)との重なりも避けられる。

### 1-d. `CoInitializeEx` の成功時だけ解放(1-4): 正しい
- `init.is_ok()` は `S_OK`・`S_FALSE` の両方で真。`S_FALSE` のときも対の `CoUninitialize` が必要なので、これで正しい。

### 1-e. 撤去の副次: `current_tip_identity()` と identity_debounce(低、記録の要)
- PR 本文の最後の「`current_tip_identity()` が Mozc 系を `Gji` と偽る件は別 PR」は、ADR・BUG のどこにも記録が無い(`grep -rn current_tip_identity docs/adr/269* docs/known-bugs/BUG-207.md` は 0 件)。
- 中身: `observer.rs:405-414` は kind が GJI 系なら一律に `TipIdentity::Gji` を返す。そのため `monitor_loop` の `identity_debounce.observe(identity, TSF_OBS.current_tip_identity())` は、Mozc では「観測 Other、現在 Gji」が毎回食い違い、2 tick ごとに確定を繰り返す。今は `set_ms_ime_native_identified(false)` が変化なしで終わるので無害。
- ほかの呼び出し元(`config1.db` など Google 版だけに当てるはずの処理)は、Mozc を Google 版として扱う。PR が別 PR に回すなら、ADR-269 の既知の限界か新しい BUG に 1 行残す(PR 本文は残らない)。

---

## (2) ADR-269・BUG-207.md の記述

1. **D1a の「detach 後に `gji_io=`/`gji_idle=` が develop と同じ値になったことの直接の証拠は未確認」** → 確認できる(上記: detach 後の `gji_idle` は 185921〜574734ms、前は 47・62ms、run 39183922616 の 4 本)。確認済みに直す。
2. **「段階 2 の前提」の結論「リテラルの有無を CI で再現・修正することは、現状ではできない」**: 陽性対照の解釈(Mozc の 0 件は根拠にならない)は正しい。ただし「CI で…できない」は、今回試した構成についての言い方に限定する。試したのは raw 打鍵、tsf と CI の Chrome のページ、cold・5 秒・30 秒の idle。
   - 陽性対照は **awase なしの raw 打鍵**。GJI のリテラルが実機で観測された経路は、awase が romaji を送る経路(SetOpen・F2 の後の cold、per-VK)だった(BUG-002 の型)。なので今回の陽性対照は「既知の機序の対照」ではない。ADR の「理由の仮説」はこれを書いているので、結論の文でも「この構成では」と揃える。
   - 例:「今回の構成(awase なしの raw 打鍵、CI の専用デスクトップ)では GJI でもリテラルが出ず、CI でリテラルの有無を確かめる手段は、まだ無い」。
3. **ADR の longidle の run 39183148517**: run 全体の conclusion は `failure`。`gh run view --json jobs` で見える 30 ジョブはすべて success で、何が失敗したかは確かめていない。根拠に使う run が赤なので、失敗の中身(要約の段の observe 期限、ジョブの一覧の上限など)を 1 行書くか、緑の run を引く。
4. **既知の限界の「GJI → Mozc 最大約 2 秒」**: 実測 1.49〜1.82 秒と合う。問い合わせの所要(最大 4ms)を足しても変わらない。
5. BUG-207 の「未確認」の「Mozc でリテラルが起きるか(CI では GJI でも再現できず…)」は妥当。
6. BUG-195 の関連の 1 行は「可能性がある」「未確認」と書かれていて妥当。

---

## (3) PR #686 の本文と実際の内容の食い違い(`gh pr view 686`、head `f55ac459`)

1. **「Mozc の 0 件(72 本)」**: 72 の出どころが無い。ADR は cold/idle 32 本と longidle 8 本で 40 本(試行は cold 16・5 秒 idle 96・30 秒 idle 48 の計 160)。ADR の数に揃える。
2. **CI の表の「起動時 GJI → 途中で Mozc に切り替え | 修正前 develop: 追随せず」**:
   - 「追随せず」を観測したのは、一度目の修正の後で D1a の前の run 39181228739。develop ではない。develop には detach の仕組み自体が無く、結果は 8000ms で INVALID ×4。
   - 「修正前 develop: INVALID ×4(監視を外す仕組みが無い)/D1a 前: 15 秒以上追随せず/修正後: 1.5〜1.8 秒で detaching、PASS」と分けて書く。
3. **CI の表に run ID が無い**。`agent-handoff.md` の 3(事実の主張には確かめた手段を添える)。ADR にある run ID(39180505943・39180466125・39179709667・39179706704・39182258683・39183922616)を列に足す。
4. **「Opus レビュー 4 ラウンドの全文」**: round 5 で 5 ラウンドになる。
5. **「ローカルではビルド・テストしていない(CI に任せた)」**: 事実の記述として妥当。マージ前に `f55ac459` の CI の結果を本文に反映する(下の必須 1)。
6. 本文の残り(原因・変更・副次・未確認・やらないこと)は、コードと ADR に一致している。

---

## (4) マージ前に足りないもの

### 必須
1. **`f55ac459`(develop のマージ後)の CI・e2e-ime・smoke が緑になるのを待つ**。ADR-268 D5 は Mozc の共有メモリの読みを専用タイマーに移す変更で、Mozc の belief のタイミングを変える。`*-mozc-*` の e2e(とくに `ts-tsf-mozc-switch-*` と `ts-tsf-mozc-gjireal-*`)がマージ後も PASS することを確かめ、結果を PR 本文に書く。
2. **Mozc 以外の IME での退行確認を 1 回**。D1a は全 IME のアクティブな TIP の判定経路を変えた(PR 本文が自分でそう書いている)。この PR で回したのは次の 2 つだけ。
   - Mozc 系の構成
   - smoke(GJI の baseline・atok-passthrough-cold の 2 構成)
   - MS-IME 本体の構成と、GJI の per-VK の経路(`ts-tsf-gji-*`・`tsx-chrome*`)は未実行。
   - 回すもの: `only` に GJI と MS-IME の代表を入れて 1 回(例: `ts-tsf-gji-*,ts-tsf-msime-*,tsx-chrome*,sc-table-msime-*`)。
   - 確かめること: PASS が develop と同じであること。awase.log の `IME kind candidate`・`TIP identity candidate`・`converter target →` の件数が develop と比べて増えていないこと(round 4 の 1-7)。
3. **PR 本文の (3)-1・(3)-2 の誤り**(72 本・develop の「追随せず」)を直す。

### 推奨
- 1-c: 専用の `LeakedThreadPool`(上限 1〜2)。時間の上限を 50ms 程度に。
- 1-b: 連続失敗の warn を 1 回出す。
- 1-e: `current_tip_identity()` の件を ADR か BUG に 1 行残す。
- (2)-1: 0 に戻す処理の直接の証拠を「確認済み」に。(2)-2: 結論の文を「この構成では」に限定する。(2)-3: 赤い run を根拠に使う理由を書く。
- (3)-3・4: run ID の列、ラウンド数。
