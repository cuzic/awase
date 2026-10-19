---
id: ADR-269
title: |-
  Mozc 系(Mozc Readable)では GJI の I/O を literal 判定の証拠にしない(安全網を外す)。Mozc 用の証拠は観測してから決める
summary: |-
  BUG-207。Mozc Readable で awase が入力直後の文字を消す。literal 判定の証拠(GJI の変換サーバーの書き込み量)が GJI 専用なのに、
  GJI の変換サーバーが同時に動いていると静止した GJI を見て誤判定する。一度目の修正(監視を mozc_server に向ける)は、
  Mozc 単独環境に判定を新しく有効にして tsf で壊した(CI 実測、Opus レビュー round 1)。Mozc 系では GJI の監視を外す(案 E)に改め、
  Mozc 用の証拠(書き込み量・候補ウィンドウ・共有メモリ)は、判定窓ごとの生の証拠を観測してから決める。
status: |-
  実装中(2026-10-18)。段階 1(Mozc 系で GJI の監視を外す)は PR #686。段階 2 以降(観測、Mozc 用の証拠)は未着手。実機は未確認。
related_adr:
  - "ADR-265"
  - "ADR-267"
  - "ADR-268"
  - "ADR-019"
---

# ADR-269: Mozc 系では GJI の I/O を literal 判定の証拠にしない

## 背景

[BUG-207](../known-bugs/BUG-207.md): Mozc Readable を入力方式にすると、awase が入力直後の文字を消す。実機ログ(2026-10-18 23:04、dragonflyg4)に `[raw-tsf-literal] ... suspected → backspace + re-send`、`gji_io=18860ms`、`GoogleIMEJaConverter` と `mozc_server` の同時稼働がある。

`raw-tsf-literal` の判定(`output/probe_io.rs`、`tsf/probe.rs::LiteralDetector`)は、キー送信の前後で次のどちらも起きなければ「リテラル」とする。
1. 変換サーバーの累積書き込み量 `TSF_OBS.gji_write_bytes` が 350B(`COMPOSITION_BYTES_THRESHOLD`、`tsf/probe.rs:625`、Chrome + GJI で実測)以上増える。
2. 候補ウィンドウ `GoogleJapaneseInputCandidateWindow` の SHOW イベント(`win_event_obs.rs`)。

両方とも GJI 専用。さらに判定は `gji_is_active_ime()`(= `gji_monitor_ok` かつ `tsf_active_kind == GJI 系`、`observer.rs`)のときだけ走る(`probe_fsm.rs:160,681` の `env.gji_active`)。**証拠が取れない IME では安全網を外す**、が既存の規律。

## 実測(GitHub Actions、windows-latest、e2e-ime の `ime=mozc`)

| 構成(各 2 回) | 一度目の修正(監視を mozc_server へ、run 39175765485) | 修正前 develop(Mozc 単独、run 39177357191) |
| --- | --- | --- |
| ts-tsf-mozc-{10,40}ms | **INVALID ×4**(`ready` 3 回失敗、疑い 1・諦め 2) | **PASS ×4**(監視は付かず、`raw-tsf-literal` 0 件) |
| ts-edit/rich-mozc-* | PASS(判定は走っていない。下記) | PASS |

- tsf の失敗は IME ON の失敗ではない。`typing_stress.log` の `ready`: 試行 1・2 は `ime_open=true` で text が空(組成が Escape/BS で消された)、試行 3 は `turn_ime_on(2)` が押す VK_CONVERT(0x1C)を Mozc が ON と扱わず、`[mozc-shm] follow external change open=false` で belief が正しく降格し `w` が素通りした。`mozc_server` の書き込みは 1 VK あたり約 0.2〜0.3KB で 350B に届かない。
- **一度目の修正は Mozc 単独環境に判定を新しく有効にした**: develop では GJI の変換サーバーが居ないと監視が付かず、`gji_active` が偽で判定が走らない。修正で `mozc_server` に付くと `gji_monitor_ok` が真になり、判定が走り始めた。
- edit・rich は awase が Unicode(`send_keys: mode=Unicode`)で送るので literal 判定が一度も走らない。「誤判定なし」と「WRITE の 350B 以上が 10ms で約 85%・40ms で約 75%」は、閾値が足りている証拠にならない(per-VK の romaji 送信とは入力経路が違い、10ms のサンプル差分で判定窓とも揃わない)。
- 「tsf の WRITE が 16 件」は入力を受けていない証拠ではなく、ready の数文字・IME キー・Enter だけが打たれたから(INVALID で本番の打鍵は 0 試行)。
- CI の tsf は入力欄が `Chrome_RenderWidgetHostHWND` を名乗り、awase は Chrome の cold-start 経路(`chrome-per-vk-literal`)に入る。実機は `setopen-per-vk-literal`(`TransmitTarget::Tsf`)で段取りは違うが、`LiteralDetector::check_now` と `run_per_vk_confirm` は共有しており、閾値の問題は両方に効く。
- 付随: ts-edit-mozc-10ms-1 の `ready` 試行 1 は `mozc_server` の遅延起動で 1 文字目が落ちた疑い(literal 判定と無関係、awase との関係は未確認、U4)。

## 決定

### D1. Mozc 系では GJI の監視を外す(案 E、段階 1、PR #686)
`converter_target`(コア、純関数): GJI 系かつ `TipIdentity::Gji` でない TIP は `ConverterTarget::NoEvidence`、それ以外は `Gji`。`gji_monitor` は `NoEvidence` のとき GJI の監視を外し(`gji_monitor_ok=false`、`gji_last_io_ms`・`gji_last_write_ms`・`gji_attach_ms`・`gji_write_bytes` も 0 に戻す)、`Gji` のときだけ接続する。判定は走らない。GJI 系でない TIP(MS-IME・ATOK)と Google 版 GJI は変わらない。

- **Mozc 単独環境**: 一度も接続せず、TSF_OBS の値まで develop と同じ(0 のまま)。違いは、develop が 3 秒ごとに出していた `no GJI process found` の debug ログが出なくなることだけ。
- **GJI と Mozc の両方がある環境**: detach で上の値を 0 に戻すので、`gji_monitor_ok` を見ない読み手(`observer::gji_idle_ms` → GjiFsm の Long/Short の判断、FocusProbe の `gji_io=`、`[h1-send] gji_idle=`)も Mozc 単独の develop と同じになる。**副次の変化**: develop では Mozc がアクティブでも GJI の変換サーバーのバックグラウンド I/O が `observe_gji_after_focus` を通って `write_observer_poll(true)` し、Mozc の belief を GJI の無関係な I/O で ON に書く汚染があった(`ime_refresh.rs:189-212`)。D1 の後はこれが止まる(IME belief の挙動の変化。良い方向)。
- **切替の窓を縮める**: 証拠を外す向き(`Gji → NoEvidence`)は安全側(安全網を外すだけ)なので 1 回の観測で即時に切り替え、付ける向きだけ 2 連続を要求する(窓は最大約 4 秒 → 約 2 秒)。

**根拠**: Mozc で本物の raw-tsf-literal が起きた記録は 1 件もない。ただし**探す仕組みも無かった**(Mozc 単独では判定が走らず、CI の awase なしの対照は warm で cold・idle の揺さぶりが無い)ので、「起きないことを確かめた」わけではない。順序 4 がそれを確かめる。観測したのは誤判定だけ(BUG-207・CI)。誤判定の害(Escape が組成中の文字を全部取り消す。`literal_detect_fsm.rs:89-92`。2 回目以降は BS×1 で諦める)は、未検出の害(リテラルが見える形で残り、打ち直せる)より大きい。

### D2. Mozc 用の証拠は、観測してから決める(段階 2、未着手)
次の選択肢は、判定窓ごとの生の証拠(`DetectEvidence`: `write_delta`・`write_ops_delta`・`show_changed`・`evidence_fresh`、`probe.rs:632-667`)を Mozc と GJI の両方で観測してから評価する。リテラル側のサンプルが 0 件の今は、閾値を導出できない(`tuning-constants.md`: 閾値を変えるなら実測値を本文に。`COMPOSITION_BYTES_THRESHOLD` は `tuning.rs` ではなく `probe.rs` の `const` なので、規約の精神として適用する。Mozc 用に別の値を足すなら `tuning.rs` に `#[measured]` 付きで置く)。

- **A. 書き込み回数・量**: Mozc はモードキーでも `w_ops=+1〜2`(`+0.0KB`)なので、回数では「受けた」と「組成した」を分けられない。分けられそうなのはバイト数(モードキー 0.0KB、文字キー: per-VK の romaji 送信の判定窓で 0.2〜0.3KB。edit・rich の Unicode 送信の 10ms サンプルでは 0.4KB 程度も出るが、入力経路が違うので混ぜない)。F2 の先行バッチの IPC が per-VK の 1 つ目の窓に入るので 1 つ目は汚れる。
- **B. `MozcCandidateWindow` の SHOW**: fork の `src/base/const.h:118`(`cuzic/mozc-readable` の `v3.34.6239-readable.1`、tag 386580f4 の `src/base/const.h` で確認)。子音 1 VK では候補が出ないはず(未確認)で、per-VK の 1 つ目は救えないと考えている。`gji_candidate_visible_now`(`probe_fsm.rs:387`)も同じクラス名に依存する。
- **C. 共有メモリ(ADR-267 version 3 の `phase`)**: IME 自身の値。反映の遅れは F2 から約 230ms(CI、tsf)、edit で `DeriveHigh(MozcShm)` が一瞬 false にし、共有メモリが 1ms 後に `open=true` を報告したが belief が戻ったのは約 50ms 後だった例があり、per-VK の deadline(約 300ms)と同じ桁。遅れの分布を先に測る。awase は version 2 までしか受け入れない(ADR-265 D3)。
- **D. 判定しない**: D1 がこの扱い。

### D1a. アクティブな TIP の問い合わせは新しい STA スレッドで行う(CI で判明、段階 1 の前提)
`gji-io-monitor` スレッドで起動時に作った `ITfInputProcessorProfileMgr` の `GetActiveProfile` は、起動後にセッション内で入力方式を切り替えても古い値を返し続けた(CI: 起動時は Google 版 GJI、`typing_stress --switch-mozc-ms` が `ActivateProfile`〈`TF_IPPMF_FORSESSION`〉で 7 秒後に Mozc へ切り替えても 15 秒以上 `initial IME kind: ... (Gji)` のまま、`detaching` も出ない。run 39181228739)。2 秒ごとのポーリングを短命の新しい STA スレッドで行う(`tip_detector::query_active_kind_on_fresh_thread`)。**この変更は Mozc に限らず、全 IME のアクティブな TIP の判定経路に及ぶ**(デバウンスは従来どおり)。

- **原因は未確定**: 「メッセージループが無いから」は未検証(新しいスレッドにもループは無いのに追随した)。インスタンスの寿命かスレッドの状態かは分からない。BUG-009 の「per-thread `GetActiveProfile` 固着」仮説の棄却は『起動時の検出は正しかった』ことが根拠で、『起動後に追随しない』ことは否定していない(矛盾しない)。BUG-195 の CI 観測(窓を持たない監視スレッドは、前面窓の切り替えでなく既定の入力方式の上書きを見る可能性が高い)の方が原因に近い。**Win+Space と同じ効果か、「アプリごとに入力方式を切り替える」が ON のとき前面アプリの TIP と一致するかは未確認**(BUG-195 では CI で Win+Space が効かなかった)。D1a は BUG-195 の挙動も変えうる(起動後の上書きの変更に追随する)。
- **取れなかった tick は飛ばす**: 新しいスレッドの問い合わせが失敗/時間切れ(`run_with_timeout` 300ms)なら「観測なし」で何も更新しない。古い mgr の値には戻さない(新旧の値が混ざるとデバウンスが壊れる。外す向きは 1 回で確定するので、古い値が Mozc だと GJI の安全網が約 4 秒外れる)。**GJI への接続時にも種別を書き戻さない**(古い mgr の値でデバウンスを通った新しい値を上書きし、MS-IME の利用者が GJI の literal 判定を受ける経路があった)。種別・識別・証拠の対象は、ポーリングのデバウンスを通ってだけ変わる。
- **専用の孤児スレッド枠と連続失敗の warn**(Opus round 5): `run_with_timeout` の既定の共有プール(フォーカス分類の IMM32/MSAA/UIA と共有、上限 8)を使うと、TSF が詰まったとき約 16 秒で全 IME のフォーカス判定まで止まる。専用の `LeakedThreadPool`(上限 2)にした。新しいスレッドの問い合わせが 5 回連続で取れなければ warn を 1 回出す(取れない環境では種別が起動時の値のまま固定される。develop の古い mgr も起動後の切り替えに追随しないので、実効的には develop と同じで悪化はしない)。
- **所要時間(CI、run 39183922616、`ts-tsf-mozc-switch-*` 4 本の `[tip-detect] fresh-thread query: Nms`)**: 138 回、中央値 1ms、p95 2ms、最大 4ms、時間切れ・観測なし 0。per-VK の判定窓(約 300ms)に比べて十分小さい。CI の専用デスクトップでの値で、実機は未測定。
- **前面のTIPとずれたときの最悪**: 新しいスレッドの値が前面アプリの実際の TIP とずれるとき、新しく悪化するのは『前面は Google 版 GJI なのに Mozc と読む』だけ(GJI の安全網が外れ、cold リテラルが回収されず残る向き。消す向きではない)。GJI と Mozc を両方入れ、「アプリごと」ON か上書きと実際の利用がずれる利用者に限る。『前面は Mozc なのに GJI と読む』『MS-IME・ATOK なのに GJI と読む』『GJI なのに MS-IME と読む』は develop と同じ(BUG-195 と同型)。

**実測(run 39182258683、`ts-tsf-mozc-switch-{8000,20000}ms`、起動時は実 GJI、`ime=mozc-gji-real`):** 切り替え(`[switch]`)から `converter target → NoEvidence` と `detaching` が 1 回ずつ出るまで 1.49〜1.82 秒(4/4、run 39182258683)。round 4 の修正後(run 39183922616)も 1.59〜1.77 秒で 4/4(PASS ×4)。8000ms は修正前・一度目の修正 INVALID ×4 → PASS ×2 だが、**8000ms は切り替えから detach の窓の間に打鍵が無い**(`ready` は 22 秒以降)ので、「切り替えの後の打鍵」の確認であって窓の確認ではない。窓の中で打っているのは 20000ms の 2 本だけ(n=2、warm の連続打鍵、誤判定なし)。20000ms の疑い 1 はどちらも切り替えの前(本物の GJI のリテラルか誤判定かは未確認)。`detaching` の行に加え、detach 後の `gji_idle` が 185921〜574734ms(0 起点で Mozc 単独の develop と同じ大きな値)になっていることを、run 39183922616 のログで確認した(Opus round 5)。

## 既知の限界(D1 の後も残る)

- TIP 切替後、旧ターゲットを見続ける窓がある。Google 版 GJI → Mozc(外す向き)は 1 回の観測で即時に外すので、最大約 2 秒(ポーリング周期)。Mozc → GJI(付ける向き)は 2 連続の観測が要るので最大約 4 秒(その間は証拠が無いだけで、誤判定は増えない)。GJI → Mozc の約 2 秒の間は BUG-207 の症状が出うる。「アプリごとに入力方式を切り替える」設定では前面アプリの TIP と一致しないことがある。
- 判定の途中で detach または GJI が終了すると、カウンタが凍って deadline で SuspectedLiteral(消す)になりうる。判定が `gji_active` の変化を見ないことによる(跳びではない)。GJI の終了/TIP 切替が判定の窓(約 300ms)に重なる確率は低く、BUG-207 の主経路ではない。別件にしてよい。(「二重入力〈StaleConfirm〉」は D1 では起きない: 判定が作られるのは `gji_active` が真のときだけで、再接続は `gji_monitor_ok=false` の間に起きるため、切替をまたいで跳びを読む判定は構造的に存在しない。一度目の設計〈同じループで付け替え〉に対する指摘だった。)
- 前面アプリの TIP と、別スレッドの `GetActiveProfile` が示す TIP が一致しないことがある(「アプリごとに入力方式を切り替える」設定)。直らない。候補: 前面プロセスの共有メモリ(ADR-265 の version 2 の `active`)が Mozc の稼働を示していたら `NoEvidence` にする(今回は実装しない)。
- `tsf_ctx`(COM 初期化)が取れないと target が `Gji` のまま固定され、GJI と Mozc が両方ある環境では BUG-207 が残る。
- `converter_target(true, MsImeNative)` は起こり得ない組み合わせで `Gji` を返す(黙って)。`GJI_CLSID`(`OnceLock`)を起動時に取りこぼすと、Google 版が `(GoogleJapaneseInput, Other)` → `None` になり、GJI だけの環境で判定が無効になる(family と clsid は同じ列挙と同じ照合なので可能性は低い)。

## 段階 2 の前提: Mozc で本物のリテラルは起きるか(CI、観測のみ)

`ts-{,raw-}{tsf,chromepage}-mozc-{cold,idle}-20ms`(run 39182258683。cold=準備確認を省いた最初の入力、idle=各試行の前に 5 秒待つ。awase なしの対照と、安全網なしの awase ありの両方。入力先は tsf と CI の Chrome のページ): 32 本とも PASS(リテラル・消失・入れ替わり・誤判定 0)。機会の数は、cold の開始 16 回(awase なし 8・あり 8)と 5 秒 idle の後の試行 96(48・48)。**この条件では、リテラルは観測されなかった。** idle 30 秒の `ts-{,raw-}{tsf,chromepage}-mozc-longidle-20ms` も 8 本(試行 48 回、awase なし・あり 各 24)すべて PASS、リテラル 0(run 39183148517。run 全体は実 GJI の構成 1 本のインストール失敗で failure だが、この 8 本は成功)。

**陽性対照の結果(run 39183922616): `ts-raw-{tsf,chromepage}-gji-{cold,idle,longidle}-20ms`(GJI・awase なし、`--mozc` なし)は 20/20 PASS、literal 0(試行は cold 2・idle 6・longidle 6 を各本ごと)。**この構成(awase なしの raw 打鍵、CI の専用デスクトップ)では、GJI でも本物のリテラルが出なかった。GJI のリテラルの既知の機序は awase が文字を送る経路の段取りなので、awase なしの raw 打鍵は既知の機序を通っていない。したがって **Mozc の 0 件は、Mozc にリテラルが起きないことの根拠にならない**(検出力が確かめられていない)。理由の仮説(未検証): GJI のリテラルは、実機の Chrome の TSF 文脈の再初期化(idle 後)や、awase が文字を送る経路の段取り(F2 の先行バッチ・per-VK の確認)に依存して出ており、raw 打鍵の対照や CI の専用デスクトップの Chrome では出ない。

**結論**: リテラルの有無を CI で再現・修正することは、現状ではできない(GJI でも再現できない)。D1(Mozc では判定しない)は『誤判定の害 > 未検出の害』と『Mozc でリテラルが起きた観測が無い』(探す仕組みが無かった点を含む)に立つ暫定の既定であり、Mozc でリテラルが起きないことの証明ではない。Mozc 用の証拠(段階 2)を決めるには、**実機(Chrome、長い idle 後)の観測**が要る。そのための安全な手段は、判定を走らせず証拠だけを `awase.log` に出す観測専用の経路(D2 の (i) `mozc_server` への観測専用の接続、または (ii) 影の判定)で、利用者の実利用のログを集めること。

## 実測・検証の順序

1. 修正後の Mozc 単独 CI(`ts-tsf-mozc-*`)で、develop と同じ PASS に戻ること(PR #686 の CI)。**D1 では Mozc 単独のコードの経路が develop と同じ**なので、これは退行が無いことの確認にしかならず、D1 の効果の確認にはならない。効果を示せるのは順序 2 だけ。
2. **GJI と Mozc を両方入れた構成** `ts-{edit,tsf}-mozc-gji-{40,10}ms`(`ime=mozc-gji`、BUG-207 の実際の条件)を `e2e-ime` に足した。**同名のダミープロセス**(`powershell.exe` のコピーを `GoogleIMEJaConverter.exe` と名付けて待機させる。awase の監視はプロセス名で探し、累積 I/O が増えないことを根拠にするので、静止した GJI の変換サーバーと同じ見え方になる)を動かしたまま、入力方式を Mozc Readable だけにする。実際の GJI を入れる方法は、GJI の TIP が既定でアクティブのままになり(`initial IME kind: GoogleJapaneseInput (Gji)`)、`regsvr32 /u` でも外れず、Mozc がアクティブという条件を作れなかった(run 39178603274・39179133289)。ダミーは実 GJI の I/O の全てを再現はしない(U6: 実 GJI の変換サーバーは起動直後や実打鍵で I/O が動く)。条件が成立しなければ(ダミーが動いていない、`awase.log` の `initial IME kind` が `(Other)` でない)ジョブを失敗にする(条件不成立を PASS と読まないため)。develop(`attached to GJI process`、誤判定が出るはず)と修正後(接続しない。起動時から `NoEvidence` なので `detaching` は出ない。`detaching` は途中で切り替える順序 5 の構成で出る)を、awase.log で比べる。develop 側は `ci/bug-207-baseline-mozc`。
3. 観測のみの PR: `DetectEvidence` を判定ごとに `awase.log` へ出し、Mozc と GJI の tsf で分布を取る。**D1 では Mozc で判定が走らないので、Mozc の `DetectEvidence` は 1 件も出ない。**分布を取るには、(i) `mozc_server` に観測専用で接続する(`gji_monitor_ok` とは別の旗にし、`gji_is_active_ime()` には効かせない)か、(ii) 判定を影で走らせる(verdict は計算して journal に出すが回収〈Escape・BS・再送〉はしない。送り方は Mozc 単独の develop のまま〈一括送信〉にして、窓だけ測る。per-VK の送り方自体が挙動を変えるため)かのどちらかが要る。(ii) のほうが判定窓と揃う(GJI の子音 1 VK も 350B を越えない〈`probe_fsm.rs:366`〉ので、GJI の per-VK は SHOW と候補可視の近道に支えられている、という仮説もここで確かめる)。
4. Mozc で本物のリテラルが起きるかを、awase なしで測る(`ts-raw-tsf-mozc-*` に cold・idle の揺さぶりを足す)。起きないなら、Mozc に安全網は要らない(D1 で確定)。
5. 起動時は実 GJI で始めて途中で Mozc に切り替える構成 `ts-tsf-mozc-switch-{8000,20000}ms`(`typing_stress --switch-mozc-ms=N`、`ime=mozc-gji-real`)。監視を外す経路・`gji_*` を 0 に戻す処理・即時の切り替えを CI で動かす(ダミー方式は起動時から `NoEvidence` で、これらを一度も実行しない。Opus round 3)。`awase.log` の `detaching` の時刻と `raw-tsf-literal` の有無を並べて、切替の窓を測る。
6. 実機(dragonflyg4)。読むだけの前提確認(日本語入力は止まらない): GJI がインストールされたままか、変換サーバーがなぜ常駐しているか、「アプリごとに入力方式を切り替える」の設定。そのうえで Mozc Readable だけにして、文字が消えないこと、`[raw-tsf-literal]` が出ないこと、`detaching` が 1 回出て `gji_io=` が 0 起点になること(日本語入力が止まるため、利用者に予告して許可を得てから)。

## 記録の修正(ハーネス側)

- `typing.json` の `cfg.ime` と `result.txt` の見出しが `gji` になる(`e2e-ime.yml` の `ts-*-mozc-*` の `stress=` に ime の指定が無い)。ラベルの誤りで、checker の振る舞いへの影響は未確認(U5)。
- 観測した構成が限られる(ts-raw-tsf-mozc は `10ms-1` を精査、他は件数のみ)。

## 結果

**BUG-207 の条件の CI 再現と修正の効果(2026-10-19、`ime=mozc-gji`: 同名のダミー変換サーバーが静止して動き、アクティブな TIP は Mozc、`initial IME kind: GoogleJapaneseInput (Other)` を `awase.log` で確認):**

| 構成 | 修正前 develop(`ci/bug-207-baseline-mozc`、run 39179709667) | 修正後(`107cbacd`、run 39179706704) |
| --- | --- | --- |
| ts-tsf-mozc-gji-{10,40}ms(各 2 回) | **INVALID ×4**(`attached to GJI process` 1、`raw TSF literal suspected` 1、`giving up` 2) | **PASS ×4**(`attached` 0、`raw-tsf-literal` 0、`converter target` 1) |
| ts-edit-mozc-gji-{10,40}ms | PASS(判定は走らない) | PASS |

**実 GJI をインストールし、変換サーバーを起動したまま Mozc Readable もインストールし、`typing_stress --mozc` で Mozc をアクティブにした構成 `ime=mozc-gji-real`(2026-10-19、GJI の TIP も有効なまま。`initial IME kind: GoogleJapaneseInput (Other)` を確認):**

| 構成 | 修正前 develop(run 39180505943) | 修正後(`561ad8b6`、run 39180466125) |
| --- | --- | --- |
| ts-tsf-mozc-gjireal-{10,40}ms(各 2 回) | **INVALID ×4**(`attached to GJI process` 1、疑い 1、諦め 2) | **PASS ×3**(`attached` 0、literal 0、`converter target: NoEvidence`)。残り 1 本は GJI の chocolatey インストールが 3 回失敗〈環境の揺れ、修正と無関係〉で未実行 |
| ts-edit-mozc-gjireal-{10,40}ms | PASS ×2 | PASS ×2 |

利用者の条件(GJI と Mozc の両方をインストールして Mozc を有効にしている)が、実 GJI でも CI で再現し、修正で消えた。
修正前は BUG-207 の症状(静止した GJI を見て誤判定し、組成を消して諦める)が CI で再現し、修正後は消えた。**未確認**: 実機(dragonflyg4)、実 GJI の変換サーバー(ダミーは同名・静止のみ再現。U6)、TIP 切替の窓、段階 2(Mozc 用の証拠)。
