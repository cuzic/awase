# ADR-269 再レビュー(round 2)

対象: `fix/bug-207-mozc-server-io-monitor` の HEAD `50fa63c2`(worktree `/home/cuzic/awase-bug207`)。round 1 からの差分は `git diff 8aad4f74..50fa63c2`。
自分で確かめたこと:
- 修正前 develop の比較 run 39177357191 の `result-ts-tsf-mozc-{10,40}ms-{1,2}` を取得した。4 本とも `verdict=PASS trials=12 literal=0` で、`raw TSF literal` は 0 件、`attached to` も 0 件(`no GJI process found` の繰り返し)。round 1 の予想どおり。
- 修正後の run(e2e-ime 39178169030・CI 39178171800)は、この時点で実行中。結果は未確認。

結論: 案 E(a) の実装は、GJI だけの環境と Mozc だけの環境では develop と同じ挙動になる(下記 a-1・a-2)。直すべき箇所は少ない。ADR の段階 2 の観測の手順(D2・順序の 3)は D1 と両立しないので、書き直しが要る(d-1)。

---

## (a) 新しい設計のバグ・副作用

### a-1. GJI だけの環境: 変わらない(確認した)
- `converter_target(true, Gji)`・`(false, _)` は `Gji`。初期値も `Gji`(`gji_monitor.rs:340`)。
- 接続の条件に `current_target == Gji` が足されただけ。外す条件は `None` のときだけ(`:434`・`:443`)。
- 例外は round 1 の 14 と同じ: `GJI_CLSID`(OnceLock)を取りこぼした場合、`tsf_ctx` が None(COM の初期化に失敗)の場合。後者は target が `Gji` のまま固定されるので、GJI と Mozc が両方ある環境では BUG-207 が残る。「既知の限界」に 1 行足す。

### a-2. Mozc だけの環境: develop と同じ(確認した)
- 初回の問い合わせで `None` になり、接続を一度も試さない。`gji_monitor_ok` は初期値の false のまま。`gji_last_io_ms`・`gji_write_bytes`・`gji_last_write_ms`・`gji_attach_ms` は 0 のまま。
- develop の Mozc 単独(GJI の接頭辞が見つからず接続しない)と、TSF_OBS の値まで同じになる。
- 違いは、develop が出していた 3 秒ごとの `no GJI process found` の debug ログが出なくなることだけ。

### a-3. GJI と Mozc の両方がある環境で detach した後: develop の Mozc 単独とは「同じではない」(中)
- detach は `gji_monitor_ok=false` にするだけで、TSF_OBS の `gji_last_io_ms`・`gji_last_write_ms`・`gji_write_bytes`・`gji_attach_ms` は GJI の最後の値のまま残る(`gji_monitor.rs:434-441`)。
- 読み手のうち、`gji_monitor_ok` を見ないもの:
  - `observer::gji_idle_ms()` → `platform.rs:630,712,744,792,805`(GjiFsm の Long/Short の判断、`Reopen ... gji_idle=`)。develop の Mozc 単独では `now - 0` で常に Long。detach 直後は「GJI が数秒前に動いた」値になり、Short に倒れうる。detach は TIP 切替の 2〜4 秒後なので、GJI の最後の I/O から数秒〜時間が経つまで差が残る。
  - `observe_gji_after_focus`(`observer/gji_observer.rs:34`、`runtime/ime_refresh.rs:189` が `active_ime_kind()==GJI 系`のとき呼ぶ。Mozc も GJI 系)。`GJI_CONFIRM_WINDOW_MS=500` 以内でないと効かないので、detach 後の害はほぼ無い。
  - 診断の `FocusProbe ... gji_io=`、`[h1-send] gji_idle=`(`vk_send.rs:608`)。BUG-207 の手がかりになった `gji_io=18860ms` と同じ欄が、detach 後は「GJI が静止している」と読める値を出し続ける。次の調査を誤らせる。
- 勧める対応: detach のときに `gji_last_io_ms`・`gji_last_write_ms`・`gji_attach_ms` を 0 に戻す。`gji_write_bytes` も戻してよい。detach 中は判定が走らないので読み手がいない。こうすると「Mozc 単独の develop と同じ」が TSF_OBS の値の水準で言える。ADR の D1 の「Mozc 単独環境の develop と同じ挙動」は、現状では GJI と Mozc の両方がある環境に対しては言いすぎ。
- 再接続(Mozc → GJI)で 0 から跳んでも、判定(`LiteralDetector`)は `gji_active` が真になってから作られる。なのでベースラインの問題は起きない(c 参照)。`gji_attach_ms` を新しく書くので、BUG-176 の「接続直後の初回読み」の扱いも従来どおり。

### a-4. 書かれていない挙動の変化: belief の観測経路が 1 本減る(低〜中)
- develop の GJI と Mozc の両方がある環境では、Mozc がアクティブでも GJI の変換サーバーのバックグラウンド I/O が `observe_gji_after_focus` を通って `write_observer_poll(true)` していた(`ime_refresh.rs:189-212`)。Mozc の belief を GJI の無関係な I/O で ON に書く汚染。コメント(`:186-188`)が MS-IME について警告しているのと同じ型。
- D1 の detach 後はこれが止まる。良い方向の変化だが IME belief の挙動の変化なので、ADR の D1 と BUG-207 に 1 行書く。「GJI と Mozc の両方がある環境では、GJI の I/O を Mozc の ON の推測にも使わなくなる」。fix-requires-evidence の「IME belief」ファミリー(`runtime/ime_coordinator.rs`、`state/observation_store.rs`)に触れていないので規約上の義務は無いが、記録として要る。

### a-5. デバウンスの窓(round 1 の 8)は、非対称にすれば安く縮められる(中、提案)
- 証拠を外す向き(`Gji → None`)は安全側(安全網を外すだけで、誤判定は増えない)。1 回の観測で即時に外し、付ける向き(`None → Gji`)だけ 2 連続を要求すればよい。最悪約 4 秒が約 2 秒に縮む。
- `Debounce::observe` を通さず、`observed_target == None` なら即時 `current_target = None` にする 3 行程度で済む。
- round 1 の 8 の「アプリごとの入力方式」の不一致は、別スレッドの `GetActiveProfile` を使う限り直らない。
  - 代案: 前面のプロセスについて ADR-265 の共有メモリ(version 2 の active)が Mozc の稼働を示していたら、`None` 扱いにする。前面アプリに即した信号になる。
  - 実装は今回でなくてよい。ADR の「既知の限界」に候補として書いておく。

### a-6. 細かい点(低)
- `ConverterTarget::None` は `Option::None` と名前がぶつかる。読みにくい(`ConverterTarget::None` と完全修飾で書かれているので実害は無い)。`NoEvidence` などのほうが誤読しにくい。
- `observer.rs:707` の `ActiveImeKind` の doc「`gji_monitor_ok` の状態から派生する」は、既に古い(`active_ime_kind()` は `tsf_active_kind` だけを見る、`:356-362`)。今回の「`gji_monitor_ok=false` でも kind は GJI 系のまま」という前提と食い違って読めるので、ついでに直すとよい(本 PR の範囲外でも可)。

---

## (b) ADR-269・BUG-207.md の記述

1. **BUG-207.md:22「修正前(develop)は同じ構成が 6/6 PASS」**: ADR の表は tsf が「PASS ×4」。私が数えた tsf も 4 本。何を 6 と数えたのかを書くか、ADR と揃える。
2. **BUG-207.md の `fix_commits: []`**: `50fa63c2` を入れる(取り下げた `95db75cb` も経緯として本文に)。fix-requires-evidence (b) は修正履歴のハッシュを求めている。
3. **BUG-207.md:26「回帰テスト」**: 元の不具合(GJI と Mozc の両方がある環境)で develop が落ちるテストは、まだ 1 本も無い。単体テストと architecture_guard は文字列と純関数だけを見ている。`ts-tsf-mozc-*` は develop でも PASS する。「元の条件を再現するテストは未作成(ADR-269 の順序 2)」と明記する。今の書き方は、CI の `ts-tsf-mozc-*` が再発防止になっているように読める。
4. **ADR:50「Mozc で本物の raw-tsf-literal が起きた記録は 1 件もなく」**: 事実だが、探す仕組みも無かった。Mozc 単独では判定が走らない。CI の raw 対照は warm で、cold・idle の揺さぶりが無い。「記録が無い(検出も観測もしていなかった)」と書かないと、「起きないことを確かめた」と読まれる。順序の 4 がそれを確かめる手順であることと対にする。
5. **ADR:56(B)「子音 1 VK では候補が出ないので」**: round 1 でも「はず」として書いた推測で、確かめていない。「未確認」を付ける。
6. **ADR:57(C)「edit で一瞬 false に落ちて 1ms 後に戻る」**: round 1 の私の書き方が不正確だった。ts-edit-mozc-10ms-1 では、22.618 に `DeriveHigh(MozcShm)` が false にした。1ms 後(22.619)に共有メモリが `open=true` を報告し、belief が true に戻ったのは 22.669(**約 50ms 後**)。「約 50ms 後に戻る」に直す。
7. **ADR:55(A)「文字キー 0.2〜0.4KB」**: 0.4KB は edit・rich の Unicode 送信の 10ms サンプルの値。per-VK の romaji 送信の判定窓で見たのは 0.2〜0.3KB。ADR:40 で「経路が違う」と書いた値を ADR:55 で混ぜている。出どころを分けて書く。
8. **ADR:63(既知の限界の 2 つ目)**: D1 の下では不正確(c で詳述)。D1 では切替をまたいで「生きている判定」が跳びを読む経路は無い。残るのは「判定の途中で detach または GJI が終了 → カウンタが凍る → deadline で SuspectedLiteral(消す)」だけ。「二重入力(StaleConfirm)」は D1 では起きない。round 1 の 7 は取り下げた一度目の設計(同じループの中で false→true へ付け替える)に対する指摘で、D1 にはそのまま当てはまらない。私の round 1 の書き方が設計を区別していなかった。
9. **ADR:68(順序 1)**: Mozc だけの CI は、D1 では develop とコードの経路が同じ(a-2)。なので「PASS に戻ること」は退行が無いことの確認にしかならず、D1 の効果の確認にはならない。そう明記する。D1 の効果を示せるのは順序 2 だけ。
10. **BUG-207.md の症状「日本語の入力方式を Mozc だけにした状態」と「GoogleIMEJaConverter も動いていた」**: 一見食い違う。GJI の TIP を一覧から外しても変換サーバーが残っていたのか、GJI がアンインストールされずに常駐していたのか。原因の説明の前提なので「未確認」と書く。順序 2 の CI 構成で「TIP の一覧に GJI が無いのに変換サーバーが動いている」状態をどう作るかにも直結する(d-2)。

---

## (c) round 1 の 7 の純関数テストを今回入れるべきか

**今回は不要。観測の PR に回してよい。**

理由:
- D1 では、判定(`LiteralDetector::new*`)が作られるのは `gji_is_active_ime()` が真のときだけ(`probe_fsm.rs:160,681`、`vk_send.rs:639`、`output/mod.rs:1041`)。
- 再接続(`None → Gji`、または GJI の再起動後)は、`gji_monitor_ok=false` の間に起きる。その間は新しい判定が作られない。なので「付け替えをまたいで跳びを読む判定」は存在しない。
- 一度目の設計では、同じループの 1 回の中で `gji_monitor_ok` が false→true に戻るので、主スレッドが一度も false を見ないまま跳びを読む余地があった。D1 で、その経路は構造的に消えた。
- 残る「途中で凍る → 消す」の経路は、跳びではなく、判定が途中の `gji_active` の変化を見ないことによる。
  - テストを入れるなら、純関数より「途中で `gji_active` が偽になったら判定を打ち切る(Unknown、消さない)」という小さな挙動の変更のほうが筋が良い。
  - ただし、GJI の終了と TIP 切替が判定の窓(約 300ms)に重なる確率は低く、BUG-207 の主経路ではない。別件にしてよい。
- ADR の「既知の限界」は (b)-8 のとおり書き直す。

---

## (d) 実測の順序で足りないもの

### d-1. 順序 3(観測のみの PR で Mozc の `DetectEvidence` の分布を取る)は、D1 と両立しない(高)
- D1 では Mozc で判定が一切走らないので、`DetectEvidence` は Mozc では 1 件も出ない。分布を取るには、どちらかが要る。
  - (i) `mozc_server` に**観測専用**で接続する。`gji_monitor_ok` とは別の旗にして、`gji_is_active_ime()` には効かせない。
  - (ii) 判定を「影」で走らせる。verdict は計算して journal に出すが、回収(Escape・BS・再送)はしない。
- (ii) のほうが判定窓と揃った値が取れる。ただし per-VK の送信の段取り(1 VK ずつ送る)自体が挙動を変える。影の判定は「送り方は Mozc 単独の develop のまま(一括送信)、窓だけ測る」形にしないと、観測が本番の挙動を変える。
- これを ADR の D2 と順序 3 に書かないと、段階 2 は着手した時点で行き詰まる。

### d-2. 順序 2(GJI と Mozc の両方の構成)の作り方が未定義(中)
- BUG-207 の条件は「アクティブな TIP は Mozc、GJI の変換サーバーが動いている」。`install-gji.ps1` の後に `install-mozc-readable.ps1` で TIP の一覧を Mozc だけにしても、GoogleIMEJaConverter が起動している保証が無い。
- GJI の変換サーバーは GJI の TIP が一度アクティブになるか、broker が起動したときに立ち上がるはず(未確認)。構成の手順に次の 3 つを入れる。
  - 変換サーバーを確実に起動する: GJI を一度アクティブにしてから一覧を Mozc にする、または `GoogleIMEJaConverter.exe` を直接起動する。
  - 起動を `Get-Process` で確かめる。
  - awase.log の `attached to GJI process`(develop)と `detaching`(修正後)で、条件が成立したかを機械的に判定する。
- 条件が成立しなかった run は INVALID にする。そうしないと、Mozc 単独と同じ PASS を「直った」と読む誤りが起きる。

### d-3. TIP 切替の窓を測る構成が無い(中)
- 「既知の限界」の約 4 秒(a-5 で約 2 秒に縮められる)は推定値。GJI → Mozc を Win+Space 相当(`ActivateProfile` か `Set-WinUserLanguageList`)で切り替えた直後の 0〜5 秒に打鍵し、`detaching` の時刻と `raw-tsf-literal` の有無を並べる構成があると、限界の実害と、非対称デバウンスの効果を測れる。
- 順序 2 の派生として足すのが安い。

### d-4. 実機(順序 5)の前提確認(低)
- 実機で確かめる前に、dragonflyg4 で次を確かめる。読むだけで、日本語入力は止まらない。
  - GJI がインストールされたままか。
  - 変換サーバーがなぜ常駐しているか。
  - 「アプリごとに入力方式を切り替える」の設定。
- 修正後の awase.log では `detaching` が 1 回出ること、その後 `gji_io=` が(a-3 を直したなら)0 起点になることを確認項目に入れる。

### d-5. 規約(低)
- `tuning-constants`: 今回の差分は定数を変えていないので対象外。D2 の記述(`probe.rs` の const は規約の精神として、新設は `tuning.rs` に `#[measured]`)は妥当。
- `fix-requires-evidence`: (b) の BUG-207.md はある。(b)-2・(b)-3 のハッシュと「再現テストは未作成」の明記を足せば満たす。
- `experiment-logging`: 一度目の修正(`95db75cb`)は revert ではなく上書きの修正(`50fa63c2`)なので、revert の規約の文面の対象外。ただし「mozc_server を見る」案は IME 制御の採用・撤回そのもの。`docs/experiments.md` に 1 行(アプリ: CI の tsf 入力欄、IME: Mozc Readable、症状: 1 VK 0.2〜0.3KB で 350B に届かず ready 4/4 INVALID、run 39175765485)を足す。同じ案が後日また出たときの歯止めになる。
