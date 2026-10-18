# ADR-269 下書き 敵対的レビュー(round 1)

対象: `docs/adr/269-mozc-literal-detection-evidence.md`(ブランチ `fix/bug-207-mozc-server-io-monitor`、HEAD `8aad4f74`)
根拠にしたもの: GitHub Actions run 39175765485 のログ(`.../scratchpad/e2e1/<構成>/dist/awase.log`・`dist/typing_stress.log`・`typing.json`・`result.txt`)と、同じブランチのコード。
数え直しのコマンドは各指摘に添える。

---

## 重大度: 致命(方針を変える)

### 1. 段階 1 は Mozc だけの環境に、raw-tsf-literal の誤判定を「新しく持ち込んだ」可能性が高い(コードから)

- `crates/awase-windows/src/tsf/observer.rs:564` `gji_is_active_ime()` = `gji_monitor_ok && tsf_active_kind == GJI`。
- `crates/awase-windows/src/tsf/warmup/probe_fsm.rs:681` `let needs_literal = env.gji_active;`(Chrome 経路)、`:160` も `env.gji_active` を条件にしている。テスト `chrome_without_gji_active_skips_literal_detect`(`probe_fsm.rs:1121` 付近)が「monitor が不健全なら LiteralDetect なしで直接送る」を固定している。
- つまり **develop では、GJI の変換サーバーが居ない環境(Mozc だけ)だと monitor が付かない → `gji_active=false` → per-VK confirm も literal 判定もそもそも走らない**。
- 段階 1 は Mozc 系で `mozc_server` に接続するので `gji_monitor_ok=true` になり、**Mozc だけの環境で初めて literal 判定が有効になる**。CI の tsf の失敗(次の指摘 2)はこの新しく有効になった判定が出したもの。
- BUG-207 の実機(dragonflyg4)は `GoogleIMEJaConverter` も動いていた(`[gji-monitor] attached to GJI process`)から develop でも判定が有効で、静止した GJI を見て誤判定した。段階 1 後は「Mozc を見て、書き込みが閾値に届かず誤判定」に変わるだけで、症状は残ると予想する(指摘 2 の 1 打鍵あたりの量から)。
- **予想(未確認)**: U1 の `ci/bug-207-baseline-mozc`(develop・GJI なし)は tsf で **PASS する**。ADR の U1 の読み方「変わらなければ段階 1 は tsf に効いておらず、原因は別」は前提が逆。PASS なら「段階 1 が Mozc 単独環境を壊した」と読むべき。
- 必要な対応: ADR の「段階 1(実装済み)」節に、この副作用を書く。U1 の判定基準を書き直す。BUG-207 の実際の条件(Mozc がアクティブ+GJI の変換サーバーも動いている)を CI で作る第 3 の構成を足す(指摘 9)。

### 2. 「閾値ではなく IME ON の失敗の可能性が高い」はログで否定される。tsf の失敗は BUG-207 そのものの再現

ts-tsf-mozc-10ms-1 で確かめた(4 構成とも行数・件数が同じ形)。

- `dist/typing_stress.log` の ready 記録:
  - attempt 1: `"ime_open":true, "text":""`
  - attempt 2: `"ime_open":true, "text":""`
  - attempt 3: `"ime_open":false, "text":"w"`
  - (`grep -h '"type":"ready"' ts-tsf-mozc-10ms-1/dist/typing_stress.log`)
  - **3 回中 2 回は IME が開いていた**(ハーネスが自プロセスの HIMC で読んだ値)。
- attempt 1 の awase.log の流れ:
  - ハーネスが VK_IME_ON(0x16)を押す。awase が自分で 0x16 を送る(`GJI direct: send 0x0016`)。
  - `mozc_server` が起動し、23:19:17.05 に `attached to MozcServer`。
  - 'か' を romaji "ka" として per-VK で送る。
  - `per-VK[0/1] confirmed (vk=0x4B)`。根拠は直後の `w_KB=+2.2`(23:19:18.5799)。
  - 'a' の窓の書き込みは `w_ops=+2 w_KB=+0.3`(18.6959)の 1 サンプルだけ。結果は `per-VK[1/1] suspected literal (vk=0x41 backs=0 escape=true)`。
  - 回収は Escape(組成ごと取り消し)と "ka" の再送。
  - 再送の 'k' の窓は `w_KB=+0.2` で `suspected literal ... backs=1`、`giving up`、BS×1。
  - Enter を押して text は `''`。
  - IME が閉じていたなら "ka" が字のまま残るはず。`''` になったのは、**組成が実際に起きていて、awase の Escape と BS がそれを消した**ことを示す。
- attempt 2(0xF2 で ON、`[mozc-shm] ... open=true` 20.90):
  - 'k' の窓は `+0.0KB`(w_ops+1)と `+0.2KB` だけ。
  - `consecutive raw-tsf-literal (count=3) → giving up, backs=1`。
  - 'a' は送られず、text は `''`。
- attempt 3 は `turn_ime_on(2)` が押す VK_CONVERT(0x1C)。
  - `key-effect-predict vk=0x1C open=Some(true)` のあと、`[mozc-shm] follow external change open=false`(24.79)で `DeriveHigh(MozcShm)` が belief を OFF に落とした。
  - 以後 'w' は素通りした。これは **ハーネスの ON 候補キーを Mozc が ON と扱わない**ためで、ADR-268 の降格はむしろ正しく働いている。
  - (`crates/awase-windows/examples/typing_stress/main.rs:172` `turn_ime_on`、`:1265`)
- 結論: Mozc の 1 VK あたりの書き込みは **約 0.2〜0.3KB**(この run の 'k'・'a' の窓)。`COMPOSITION_BYTES_THRESHOLD=350`(`tsf/probe.rs:625`)を per-VK で越えない。**原因は閾値(と、per-VK の証拠の作り方)**であって ON の失敗ではない。
- ADR の 47〜48 行目の見立てと、summary の「IME ON の失敗なのかは未確定」は書き換えが要る。U2 は既存ログでほぼ答えが出ている: 0x16 と 0xF2 では開く。0x1C では開かない。

### 3. 「16 件しかなく mozc_server がほとんど入力を受けていない」は読み違い

- tsf は ready で INVALID になり、本番の打鍵は 0 試行(`typing.json` の `"trials": 0`、`"試行が0件"`)。打たれたのは ready の数文字・IME キー・Enter だけ。
- 16 件は「打たれたキーがほぼ全部 mozc_server に届いた」件数。`VK_IME_OFF`・`F2`・Enter・各 VK のたびに `w_ops=+1〜2` が出ている(`grep -n 'gji-io\] WRITE' ts-tsf-mozc-10ms-1/dist/awase.log`)。
- edit・rich(548〜635 件)とは打鍵数が桁違いなので比べられない。「ほとんど入力を受けていない」根拠にはならない。

### 4. edit・rich の「誤判定なし」と「350B 以上が約 85%」は、tsf の判定の根拠にならない

- edit・rich では literal 判定が一度も走っていない。
  - `grep -cE 'tsf-probe|per-VK|literal detect'` は edit・rich の全 8 本で 0。
  - awase の送信は `send_keys: mode=Unicode`(ts-edit-mozc-10ms-1 で 753 行、`mode=Vk` は 0 行。rich も同じ)。
  - 表の「raw-tsf-literal 0/0」は判定が無いことの裏返しで、閾値が足りていることの証拠ではない。
- WRITE の量も、Unicode(VK_PACKET)送信に対する mozc_server の書き込み。tsf が使う romaji VK の per-VK 送信とは入力経路が違う。
- `[gji-io] WRITE` は 10ms のサンプル差分。10ms 間隔の打鍵では複数キーが 1 サンプルに入る。LiteralDetector の判定窓(送信前のベースライン〜deadline、`since_vk_sent_ms=312`)とも揃っていない。
- 数字そのもの(`.../e2e1/` で `w_KB` を 1024 倍して 350 以上を数えた):
  - edit: 10ms 86.9%・85.8%、**40ms 76.4%・76.3%**
  - rich: 10ms 84.8%・86.6%、**40ms 74.6%・75.6%**
  - 「約 85%」は 10ms だけの値。全体では 75〜87%。
- ログは 0.1KB 単位に丸めているので、`0.3`(256〜358B)が 350B の上か下かは決められない。「350B 以上」は近似の値。
- 件数そのもの(edit 548〜617・467〜490、rich 488〜635・414〜478、tsf 16・1、疑い 1・諦め 2)は ADR の表と一致した。

---

## 重大度: 高

### 5. 誤判定の害の見積もりが小さすぎる(選択肢 D の評価が逆向き)

- per-VK の 2 つ目以降の VK で疑いになると、`per_vk_recovery_params(false, idx>0)` は `(backs=0, escape=true)` を返す(`tsf/warmup/literal_detect_fsm.rs:89-92`)。Escape は**組成中の文字列を全部取り消す**。NICOLA では確定までの数文字が 1 つの組成なので、消えるのは 1 文字ではなく、組成中の全部。attempt 1 で「か」が消えたのがこれ。
- `consecutive` は confirm でしか 0 に戻らない。CI では ready の試行をまたいで count=3 まで積み上がった。
- 2 回目以降は「BS×1・再送なし」の諦め分岐に入る。打った文字が毎回消え、入力が成り立たない。実機ログの `cold=2..8 giving up` の連続もこれ。Mozc では 1 VK で 350B を越えるのが偶然(Enter の履歴書き込みなど 2.2KB の塊と重なったとき)だけなので、reset も偶然に頼ることになる。
- 未検出の害(リテラル "ka" が見える形で残り、利用者が打ち直せる)とは非対称。しかも **Mozc で本物の raw-tsf-literal が起きた記録は 1 件も無い**。BUG-207・CI とも、Mozc で観測したのは誤判定だけ。
- 指摘 1 のとおり、D は「develop の Mozc 単独環境の挙動そのもの」で、実利用の苦情も出ていない。D を「最後の手段」に置く理由が ADR に無い。むしろ既定値にして、A〜C は「Mozc で本物のリテラルが観測されたら」の後段に回すのが筋。

### 6. 見落としている選択肢 E(最小で元に戻せる): Mozc 系がアクティブなときは literal 判定用の monitor を健全にしない

- `converter_target` が `MozcServer` を返すときの扱い:
  - (a) 何にも接続しないで `gji_monitor_ok=false` にする。または (b) 診断用に接続はするが、`gji_is_active_ime()` が偽になるようにする(例: `gji_active` に `target == Gji` を足す)。
- 効果:
  - 実機の条件(GJI も動いている): 静止した GJI を見ない。
  - Mozc 単独: develop の挙動のまま。
  - 両方で誤判定が止まる。
  - GJI だけの環境は変わらない。
- D との違い: 判定ロジックに手を入れない。「証拠が取れない IME では安全網を外す」という既存の規律(`!gji_active → needs_literal=false`)に乗るだけ。
- warmup の `TsfReadinessProbe::check_now`(`tsf/probe.rs:126`)は monitor 不健全なら max_deadline 待ちになる。この影響は develop の Mozc 単独と同じで、新しい挙動ではない。
- 段階 1 の「mozc_server に接続する」部分は、この E に置き換えるか、`gji_active` から切り離した観測のみ(journal に per-VK の delta を出すだけ)に格下げすることを勧める。

### 7. 付け替え時のベースラインの扱い: 誤判定(消す)と二重入力(足す)の両方が起きうる

`gji_monitor.rs:438-460`・`:503-508` の読みから。

- `TSF_OBS.gji_write_bytes` には接続中のプロセスの**累積値をそのまま**入れている(`m.last_write_bytes()`)。
- 付け替えると、同じ値が「GJI の累積」から「mozc_server の累積」へ不連続に跳ぶ。
- 送信前にベースラインを取った LiteralDetector(`probe.rs:583`/`:611`)が付け替えをまたぐと、次のどちらかになる。
  - 新しい累積 < ベースライン: `saturating_sub` で 0 → `SuspectedLiteral` → 消す。長く動いた GJI の累積は大きく、起動したての mozc_server は小さいので、こちらになりやすい。
  - 新しい累積 > ベースライン+350: `write_confirmed=true`。一方 `gji_last_write_ms` は付け替えで消さず、GJI の古い時刻のまま。mozc_server の最初の書き込みサンプルまで更新されない(`:522-525`、try_attach の初回 sample は TSF_OBS に書かない)。すると `evidence_is_fresh=false` → `StaleConfirm` → **BS なしで romaji を再送** → 二重入力(例: 「かか」)。
- 付け替えはデバウンスで TIP 切替の 2〜4 秒後に起きる。利用者が Win+Space の直後に打ち始めると、打鍵の最中に当たりうる。
- 対策の候補:
  - 付け替え時に、プロセスをまたいで単調な「オフセット付き累積」を TSF_OBS に出す。
  - 付け替えで `gji_last_write_ms` を 0 に戻す。
  - 付け替えの世代番号を LiteralDetector に持たせ、世代が変わったら判定しない(Unknown)。
- ADR はこの問題に触れていない。GJI の再起動(`GJI process exited, will re-attach`)でも同じ跳びが前からある。段階 1 で頻度が上がる。

### 8. 2 秒ポーリング × 2 連続のデバウンスの間は、旧ターゲットを見続ける

- TIP を GJI から Mozc に切り替えてから最大約 4 秒+接続まで、monitor は GJI を見たまま `gji_monitor_ok=true` で残る。この間は BUG-207 の症状そのもの(静止した GJI → 誤判定)が出る。
- 逆向き(Mozc → GJI)も同じ。
- `query_active_kind` は gji-io-monitor の別スレッドから読んでいる。「アプリごとに入力方式を切り替える」設定(Windows の言語設定)では、前面アプリの TIP と一致しないことがある(`gji_monitor.rs:262-267` の doc が既に単発フリップを観測している)。
- BUG-207 の利用者の環境は GJI と Mozc が同時に動いていた。まさに「アプリごとに違う TIP」の可能性がある。ADR の「GJI と Mozc の同時稼働の選び分けは単体テストのみ」の「単体テスト」は、プロセス名の照合だけを見ている。選び分けの時間的な正しさは見ていない。

---

## 重大度: 中

### 9. 実測の順序に足りないもの(暫定の方針 1〜4 への対案)

1. **既存ログで U2 を閉じる**(指摘 2)。新しい実行は要らない。
2. **U1 に構成を足す**。BUG-207 の本当の条件は「Mozc がアクティブで、GoogleIMEJaConverter も動いている」なので、`ime=mozc+gji`(両方入れ、Mozc をアクティブにし、GJI の変換サーバーを一度起動させる)を develop と本ブランチの両方で回す。現行の U1(GJI なし)だけでは、段階 1 がどちらの環境を良く・悪くしたかが分からない。
3. **per-VK の判定窓の生の証拠を出す(観測のみの PR)**。`DetectEvidence`(`write_delta`・`write_ops_delta`・`show_changed`・`evidence_fresh`)は組み立て済み(`probe.rs:632-667`)なのに、awase.log の `literal detect` 行には verdict しか出ていない。ADR の数字は 10ms サンプルからの推測で、判定窓の値ではない。per-VK の 1 判定ごとに出し、Mozc と GJI の両方の tsf(既存の `ts-tsf-gji-*`)で分布を取る。GJI でも子音 1 VK は 350B を越えない(`probe_fsm.rs:366` のコメント「子音単体 VK は WriteTransferCount 閾値も原理的に越えない」)。なので GJI の per-VK は SHOW と候補ウィンドウ可視の近道に支えられている、という仮説もこの分布で確かめられる。
4. **Mozc で本物のリテラルが起きるかを、awase なしで測る**。`ts-raw-tsf-mozc-*` に cold・idle の揺さぶり(`perturb.cold`/`idle_ms`)を足す。起きないなら、検出の安全網そのものが Mozc には要らない(D/E で確定)。
5. 3 と 4 で「確認側」と「リテラル側」の両方の分布が取れてから、A・B の閾値を議論する。`tuning-constants.md` は「導出」を求める。リテラル側のサンプルが 0 件の今は、閾値を導出できない。

### 10. 規約の当てはめ

- `COMPOSITION_BYTES_THRESHOLD` は `tsf/probe.rs:625` の `const` で、`tuning.rs` に無い。`tuning-constants.md` の `paths:` の対象外。ADR の方針 3 は「規約の精神として適用する」と明記しないと、レビューで抜ける。Mozc 用に別の値を足すなら、`tuning.rs` に `#[measured]` 付きで置く。この場合は `complexity-budget.md` の対象定数にもなる(未発効だが)。
- `fix-requires-evidence.md`: BUG-207.md(b)と単体テスト(a)はある。ただし (a) の `converter_target` の単体テストと `architecture_guard::gji_monitor_selects_converter_by_active_tip` は、プロセス名の照合と文字列の有無だけを見ている。実際の失敗(閾値・Escape の回収・付け替えのベースライン)を捕まえる回帰テストではない。指摘 7 の跳びは純関数に切り出せば Linux で固定できる。
- BUG-207.md の「修正(PR)」「状態: 修正を PR にした(CI 待ち)」は、CI の結果(指摘 2)と食い違う。少なくとも「段階 1 の後も tsf で再現(run 39175765485)」を書く。
- BUG-207.md の「原因(推定)」の 3 は「GJI の変換サーバーは静止しているので」のままで、閾値の要因(1 VK あたり約 0.2〜0.3KB)が無い。
- BUG-207.md の実機の証拠には**アプリ名が無い**(`KeyInput(tsf)`・`setopen-per-vk-literal`・`profile=TsfNative himc_null=true` だけ)。`experiment-logging.md` と `fix-requires-evidence.md` の「再現手順」の水準に足りない。

### 11. U3(CI の tsf と実機)の書き方が不正確。違いはもっと具体的に言える

- CI の tsf の入力欄は `Chrome_RenderWidgetHostHWND` を名乗る(`result.txt` の `入力欄クラス=Chrome_RenderWidgetHostHWND`)。awase は Chrome として扱っている: `[h1-window] cold=1 class=Chrome_WidgetWin_1`、`ChromeProbe 完了`、`TransmitTarget::Chrome` の `chrome-per-vk-literal`。
- 実機の BUG-207 は `setopen-per-vk-literal`。これは `TransmitTarget::Tsf`(gji-coro、`probe_fsm.rs:328-331`)。
- 2 つは per-VK の殻が違う経路だが、`LiteralDetector::check_now` と `run_per_vk_confirm` を共有している。閾値の問題は両方に効く。違うのは warmup の段取り(Chrome は F2 の先行バッチ)。
- ADR の「CI の tsf は Chrome ではなく RichEdit のスーパークラス化」は、ウィンドウの実体としては正しい。だが awase の分岐は Chrome 側に入っている、と書かないと誤解を招く。

### 12. edit の ready attempt 1 の失敗が書かれていない(mozc_server の起動直後の 1 文字目の消失)

- ts-edit-mozc-10ms-1 は attempt 1 が `ime_open=true, text=''`。
- awase は 18.778 に Unicode で 'か' を送った。mozc_server への接続は 20.19。20.68 に `w_ops=+80 w_KB=+312.5`(サーバーの初期化と見られる)。
- literal 判定と関係なく、**遅延起動した mozc_server の最初の 1 文字が落ちた**ように見える。「edit PASS ×4」の陰に、awase と関係あるかどうか分からない消失がある。BUG-207 の観測と混ぜないために、表の注記か U4 として分けて書く。

---

## 重大度: 低

### 13. 選択肢の記述の不正確さ

- A「Mozc は 1 打鍵 +1〜3 回」: ログでは文字以外のキーでも `w_ops=+1〜2` が出る(VK_IME_OFF・F2・Enter で `w_KB=+0.0`)。書き込み回数では「Mozc がキーを受けた」と「組成した」を分けられない。分けられそうなのはバイト数のほう(モードキー 0.0KB、文字キー 0.2〜0.4KB)。ただしリテラル側のサンプルが無い(指摘 9-5)。F2 の先行バッチは per-VK の 1 つ目の約 10ms 前に送られ、その IPC が 1 つ目の判定窓に入る。回数でも量でも、1 つ目は汚れる。
- B: per-VK の子音 1 VK(「k」)では、候補もサジェストも出ないはず(Mozc は GJI と同じ系統)。B を OR に足しても per-VK の 1 つ目は救えない。「OR 条件の追加として使う」では効果の範囲が書けていない。`gji_candidate_visible_now` の可視判定(per-VK の近道)も同じクラス名に依存しているはずなので、B を足すならそちらも同じ扱いにする必要がある(`probe_fsm.rs:387`)。
- C: 共有メモリの反映遅れがログに見える。tsf attempt 2 では F2 から `open=true` の報告まで約 230ms。edit の attempt 2 では、22.618 に `DeriveHigh(MozcShm)` が一瞬 false に落とし、1ms 後に open=true へ戻っている。per-VK の deadline(約 300ms)と同じ桁なので、phase を証拠にするなら遅れの分布を先に測る必要がある。
- `MozcCandidateWindow`(fork の `src/base/const.h:118`)は、このレビューでは確かめていない(fork のソースがこの worktree に無い)。ADR では「確認済み」と読める書き方なので、確かめたコミットを添える。

### 14. 細かい記述

- 「awase なしの対照(`ts-raw-tsf-mozc-*`)は ready が 1 回目で成功」: 手元にあるのは `ts-raw-tsf-mozc-10ms-1` の 1 本だけ。`*` と複数形で書くなら、他の 3 本(10ms-2・40ms-1・40ms-2)の結果も確かめたと添える。
- `typing.json` の `cfg.ime` は `"gji"`、`result.txt` の見出しも `IME=gji`。ハーネスに `--ime` が渡っておらず、記録のラベルが誤っている。checker が ime で振る舞いを変えるなら、判定にも影響しうる(`.github/workflows/e2e-ime.yml` の `ts-*-mozc-*` の `stress=` に ime の指定が無い)。
- `converter_target(true, TipIdentity::MsImeNative) => Gji` は起こり得ない組み合わせ。黙って Gji にするより、`debug_assert` かログで残すほうが、TIP 判定の異常に気づける(低)。
- `GJI_CLSID` は `OnceLock`。起動時に Google の CLSID を取りこぼすと、Google 版 GJI が `(GoogleJapaneseInput, Other)` → `MozcServer` を選び、GJI だけの環境で monitor が付かなくなる(=literal 判定が無効になる)。family と clsid は同じ列挙と同じ「Google」照合なので可能性は低いが、「GJI だけの環境で挙動が変わらない」と書くなら前提として明記する。
- `find_gji_pid` の「見つからない」ときの debug ログは `google/japanese procs` を並べる。Mozc 系のプロセス(`mozc_*`)は出ないので、Mozc で見つからないときの調査に使えない。
