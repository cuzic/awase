# ADR-122: GJI コールドスタート直後の per-VK confirm が「確認遅延」を「未着弾」と誤認し、回収送信が GJI 自身の非同期処理と競合してモーラが重複する（BUG-75 追加実機データに基づく再検討）

## ステータス

**設計一部収束（Opus 2体 round 1 完了、次段階へ進めるのは案F+Gのみ）。**
architect・premortem_reviewer の双方が独立に、本稿 v1 の根本原因記述に
1件の飛躍（後述）と、決定案の優先順位付けの誤りを指摘した。round 1 で
収束した結論:

- **案F（`grace_hold_verdict` の早期確定バグ修正）＋ 案G（`veto_eligible` を
  idx==0/fresh `cold_seq` に拡張）が第一候補。** 新しい定数・新しい観測
  チャネル・事後推測のいずれも追加せず、既存の安全機構（BUG-30 の候補可視
  veto）の適用範囲の是正だけで本 incident の破壊的回収を防げる。
  [tuning-constants.md](../../.claude/rules/tuning-constants.md) の実測義務も
  発生しない。
- **案C（回収を「確認してから送る」）は根本対策として価値があるが、v1 の
  具体化（HIDE イベントで確認）は BUG-75 が既に破棄した「観測トリガと
  観測対象が同一チャネル」という自己汚染パターンに該当する**ため、v1の
  ままでは採用できない。`DetectRoute` ごとに独立した確認手段が要るが、
  それが何になりうるかは本稿では未解決（後述「未決定事項」）。
- **案A（cold-start 専用の猶予延長）は単独では推奨しない。** `fencing_active
  = last_write_ms != 0` である以上、GJI が新規 I/O をまったく発生させずに
  合成を完了できるケース（本 incident の `write_ops_delta=0` はその疑いを
  示す）には、猶予をいくら延ばしても原理的に効かない。加えて
  `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE=500`（既に long idle 向けに一度
  分岐済みの同一定数ファミリー）の存在は、案Aがこの定数をもう一段
  釣り上げる構図に他ならないことを示す。案Fで足りない場合の次善策として
  保留する。
- **案D（recovery 確定の先送り）は v1 の記述に論理的誤りがあり、かつ
  premortem により「待つほど重複しやすくなる」逆効果が判明したため、
  現状の形では採用しない。**（詳細は根本原因・決定案の各節）
- **案B（`literal_session_confirmed` のセッション跨ぎ拡張）は削除した。**
  案Eの劣化版であり、cold-start では原理的に情報量ゼロ、かつ案Cの HIDE
  意味論と衝突する。
- **案E（cold-start 時は recovery を無効化）は「BUG-33 追補4 の論理的帰結」
  として位置づけを保持するが、v1 の実装記述（recovery を1箇所抑制するだけ）
  は誤り**であり、正しく書き直すと「未送信 VK をどう扱うか」の再設計が
  必要になる。案F+G が idx==0 のケースを実質的にカバーするため、単独案
  としては当面不要。

以下、本文は上記の収束結果を反映した v2。v1 からの主な訂正点は文末
「round 1 レビューでの主な訂正」参照。

## 背景

### BUG-75（`docs/known-bugs.md`）で確定していたこと

2026-08-24、msedge.exe + Google 日本語入力で「つかっても」が「っつかっても」に
文字化けする不具合が報告された（`report_id: 01M0S4S6R4C1YJ581YJ9ZGAXXD`）。
原因は `tsf/warmup/literal_detect_fsm.rs::per_vk_recovery_params` が
`StaleConfirm`（confirm 根拠が古いという判定）を「VK が未着弾」という**証拠のない
仮定**として扱い、romaji 全体を再送していたこと。実機ログでは実際には GJI が
先頭 VK（'T'）を正しく受理し合成を開始していた（`candidate SHOW` が送信18ms後、
`StartComposition` が27ms後に発火）が、`write_delta` の判定は57ms時点で
「evidence_fresh=false」と結論しており、根本は **I/O カウンタのポーリング
サンプリング遅延を『literal の証拠』と誤認していたこと** にあった。

この診断を受けて一度「先頭 VK は着弾済みなので suffix だけ再送する」方式を
実装・develop へマージした（PR #103, `45f833d3`）が、Sonnet（コーディネータ）+
Opus 2体（アーキテクト役/premortem レビュアー役）による6ラウンドの対話設計で
**複数の致命的欠陥**が判明し revert した（詳細は BUG-75 追補、2026-08-25）。
特に重要な事実:

- **「先頭 VK は必ず着弾している」という新しい無条件仮定にも実機の反例がある**
  （2026-07-22 報告「これでできる」→「kれでできる」、先頭 'k' が実際に
  未着弾のままリテラル化していたケース。**これは本 ADR の主戦場である
  `route=VisibleFencing` と同一経路**）。つまり **「着弾したかどうかを
  事後的に推測する」路線は、前提を反転させても必ずどちらかの実機ケースで破綻する**。
- 検証したすべての事後推測バリアント（`show_changed` ゲート、`consecutive`
  ゲート、`gji_reinit_retry_tombstone` ゲート、`GCS_COMPREADSTR` 直接読取）が
  「証拠なしの仮定」「自己汚染（観測トリガと観測対象が同一チャネル）」
  「セマンティクス誤認」のいずれかで破綻した。
- 対話設計の結論: 筋が良いのは事後推測ではなく **(a) なぜ StaleConfirm が
  誤って発生するのか（検出タイムアウトが短すぎる可能性、
  `EPOCH_FENCE_GRACE_MS=20ms` vs 実測着弾116ms）を直す方向**、**(b) awase が
  既に持っている状態（`literal_session_confirmed_gen`）で安全に判断できないか**、
  **(c) `GetProcessIoCounters` の `WriteOperationCount`（回数、量ではない）を
  活用できないか**、の3方向。
- 実装はせず、まず journal に診断専用フィールド（`write_ops_delta` 等、挙動には
  一切使わない）を追加し、実機データが貯まってから判断する方針とした（PR #105,
  追補2）。**「次のセッションで実機データが集まってから」という宿題として
  残されていた。**

### 2026-09-03、実機で再現（本 ADR の直接のきっかけ）

タスクトレイの不具合報告機能経由で新しい報告が届いた
（`report_id: 01M1JGJNDJT9ZAEMRAEB58ES5A`、LINE (`line.exe`)、Google 日本語入力、
`app_kind=Uwp`）。約42秒のアイドル後、`GjiFsm` が `OnCold(Long)` の状態で入力を
再開したところ、セッション最初の1文字目（romaji `"to"`、idx=1/last_idx=1）で
`StaleConfirm`（`route=VisibleFencing`）が発生し、`escape_composition=true`
（ESC送信）→ romaji `"to"` 全体の再送、という回収が走った（journal seq
38772-38776、`docs/bug-reports-triage.md` 該当行）。

journal と app.log を突き合わせて確定した事実:

1. **候補ウィンドウは実際に可視だった**（`candidate_visible: true`）。GjiFsm も
   `StartComposition(candidate SHOW)` へ遷移しているが、**この遷移が本送信に
   対応するものか、それ以前の世代のものかは journal だけでは断定できない**
   （`route=VisibleFencing` は「送信時点で既に候補ウィンドウが可視」を意味し、
   `probe_fsm.rs:375` の分岐に入る以上、この送信自体では SHOW エッジは
   再発火しないはず。app.log タイムスタンプでの裏取りは未実施 —— round 1
   architect 指摘、v1 の断定を訂正）。
2. 追補2で追加された診断フィールドは **`write_ops_delta=0, read_ops_delta=0,
   other_ops_delta=0`** —— BUG-75 の対話設計が「筋が良い」と結論した「回数
   ベースの I/O カウンタ」を見ても、この incident では **何の兆候も無かった**。
3. **`grace_hold_ms: 31` で stale と断定しているが、これは `deadline_ms` への
   到達によるものではなく、`visible_fencing_verdict` が委譲する
   `grace_hold_verdict`（`crates/awase-windows/src/tsf/probe.rs:762-776`）の
   「猶予切れ時は deadline 未到達でも即 `StaleConfirm` を返す」という
   実装によるもの**（round 1 architect 指摘、v1 の記述はここが曖昧だった）。
   `deadline_ms` 自体（＝ `RAW_TSF_LITERAL_DETECT_MS`/`_LONG_IDLE`、
   `tuning.rs` で300/500ms）にはまだ余裕があった。
4. app.log の該当行（`[gji-fsm] WarmupAborted ... while composing →
   AbortedCold(Long)` 直後に `[gji-fsm] CompositionReset: ... genuinely warm の
   ため OnCold に倒さず OnWarm を維持`）は、awase 自身のログが「実際には
   warm だった（＝誤判定だった）可能性」を示唆している。
5. `literal_session_confirmed: false` —— per-VK confirm ループは「IME セッション
   最初の1文字専用」（`literal_detect_fsm.rs` 冒頭のドキュメントコメント）であり、
   セッション最初の1文字である以上 `literal_session_confirmed` は定義上つねに
   `false` になる。
6. ESC 送信（`flush escape=true backspace ×0`）から romaji `"to"` 再送
   （`re-sending raw TSF literal romaji="to"`）まではわずか8ms。GJI が元の
   composition をまだ処理中である可能性が高いタイミングで、ESC と再送が
   割り込む。ESC が実際に pending composition を破棄できたか、それとも
   GJI が先に確定させてから ESC が届いたか（＝重複の直接原因）は、本
   journal スキーマでは判別できない（「テキストが確定した」ことを示す
   journal イベント種別が存在しない）。
7. 副次的に、この回収が走っている間ユーザーが打鍵を続けていた形跡がある
   （`[output-drain] replay ... delta=198ms`, `delta=162ms`）。回収処理中は
   output queue がゲートされ、後続の実キー入力が160〜200ms 遅延して
   一括 replay される。**この遅延は独立した副次課題ではなく、案A/C/D の
   いずれもが回収の滞留時間を延ばすことで悪化させる**（round 1 premortem
   指摘、v1 は「別スレッドに切り出す」と誤って独立扱いしていた）。

## 根本原因（round 1 レビューで訂正済み）

### 訂正1: 「20msの猶予が短すぎる」への一般化は誤り（round 1 architect 指摘）

v1 は BUG-75 元報告（+116msで真に着弾）と本 incident（`write_ops_delta` が
すべて 0）を「同じ構図（猶予不足）」と括ったが、これは誤り。**「まだ来て
いない」と「そもそも来ない」は journal からは区別できない。** 猶予を伸ばせば
救えるのは前者だけであり、後者（GJI が I/O ゼロで合成を完了できるケース）には
効かない。本 incident がどちらなのかは、現状の診断ログからは確定できない。

### 訂正2: `fencing_active` の構造的な非対称性（round 1 architect 指摘）

`visible_fencing_verdict` の `fencing_active = (last_write_ms != 0)`
（`probe.rs:718-720, 798-800`）は、awase 起動後に GJI が一度でも I/O を
発生させていれば、その**古い**タイムスタンプのままでも true を保つ。42秒
アイドル後でも `last_write_ms` は非ゼロ（42秒前の値）なので fencing は
有効なまま動作し、`last_write_ms >= epoch_send_ms` は **cold-start 直後は
必ず偽から始まる**。GJI が新規 I/O を発生させて初めてこの不等式から脱出
できる構造であり、**GJI がゼロ I/O で合成を完了できる場合、猶予をいくら
延ばしても脱出できない**。これは案Aの成立可否を左右する一次的な事実であり、
対症療法（猶予延長）の限界を示す。

### 訂正3: 回収機構（ESC + romaji 全体再送）自体が GJI の非同期処理と競合する

判定の精度をどれだけ上げても、`StaleConfirm` が発生した時点で awase は
「ESC を送って composition を破棄したはず」という**確認なしの前進**を行う。
ESC のタイミング次第で、正常に進行していた composition を破棄し損ね、
再送分だけが重複する。**検出精度の改善（案A/F）と、recovery 自体の安全化
（案C/G）は独立した2つの対策軸であり、どちらか一方では実害をゼロにできない。**

### 訂正4: `literal_session_confirmed` はセッション最初の1文字に構造的に無力

per-VK confirm ループは「セッション最初の1文字専用」であるため、この経路に
来るケースは定義上つねに `literal_session_confirmed=false` である。BUG-75
対話設計が提案した「(b) 既存の session 内 confirm 状態を使う」方向は、
**対象ケースそのものには原理的に適用できない**。

## 検討したが採らない案（BUG-75 の教訓の再確認）

以下は BUG-75 の6ラウンド対話設計で既に破綻が確認された方向であり、
**本 ADR では再提案しない**（再提案する場合は新しい反証不能な根拠が要る）:

- suffix のみ再送（先頭 VK は着弾済みと仮定） — 反例あり（"kれでできる"）
- 各種「事後に着弾有無を推測するゲート」（`show_changed`/`consecutive`/
  `reinit_retry_tombstone`） — 証拠なし・自己汚染のいずれかで破綻
- `GCS_COMPREADSTR` による composition 文字列の直接読取 — 未確定子音の
  セマンティクスが不明、hwnd 配線の増設が必要、sync path に await を
  持ち込むトレードオフが未解決のまま棚上げ

## 決定案（round 1 レビュー後の優先順位順）

### 【第一候補】案F: `grace_hold_verdict` の早期確定を deadline まで先送りする

```rust
// crates/awase-windows/src/tsf/probe.rs:762-776 付近、最終行を変更
if now.saturating_sub(hold_since) < Self::EPOCH_FENCE_GRACE_MS {
    return (now >= deadline_ms).then_some(DetectionResult::StaleConfirm);
}
// 変更前: Some(DetectionResult::StaleConfirm)  ← 猶予切れなら deadline 未到達でも確定
// 変更後:
(now >= deadline_ms).then_some(DetectionResult::StaleConfirm)
```

`StaleConfirm` は BUG-33 追補4 が確立したとおり「literal である証拠ではない」
のだから、deadline より早く回収を発動する理由がない。本 incident の
`grace_hold_ms=31` はまさにこの早期確定経路によるもので、`deadline_ms`
自体にはまだ余裕があった。

**長所**: (1) 新しい定数を作らないため tuning-constants.md の実測義務が
発生しない。案Aが狙う効果（cold-start に長い猶予を与える）を、既存の
`RAW_TSF_LITERAL_DETECT_MS`/`_LONG_IDLE`（300/500ms）の枠内で実測なしに
得られる。(2) `visible_fencing_verdict` 側のループは既に「`Some` が返るまで
tick ごとに呼び直す」構造のため呼び出し側の変更が不要。(3) deadline は
per-VK ループが元々待つ上限なので、レイテンシの最大値そのものは変わらない
（案Dのように新しい待ち時間の上限を持ち込まない）。(4) 判定ロジック内部に
閉じ、新しい観測チャネルも事後推測も増やさない。

**短所**: (1) 真に stale だったケースの回収が deadline まで遅れる（十数〜
数十ms）。ただし `StaleConfirm` の回収は `backs=0` のため、遅れても画面上の
破損は増えない。(2) `check_now` の SHOW-only 分岐（word パス/warm パス）も
同じ関数を共有するため影響範囲の精査が必要（word パスは既に
`word_level_recovery_params(is_stale=true) → backs=0` のため実利は薄いと
見込まれるが未検証）。影響を `VisibleFencing` 経路だけに限定したい場合は
関数に「deadline まで粘るか」の引数を足す。(3) 既存テスト
（`probe.rs:1010-1219`）の期待値更新が必要（Windows専用、CI待ち）。

### 【第一候補】案G: `veto_eligible` を idx==0・`cold_seq` 新規のケースへ拡張する

`output/probe_io.rs:656-663` は per-VK detector を無条件に `veto_eligible=false`
で構築している。理由は「前の VK が開いた候補ウィンドウが可視のまま残っている
状態で今回の VK が真にリテラル化するケース（前モーラ由来の誤 veto）を
避けるため」（BUG-30）。**この理由は idx==0（セッション最初の1文字、
`cold_seq` 新規）には構造的に当てはまらない —— 「前の VK」が存在しないため。**

本 incident は `candidate_visible: true` であり、idx==0 で veto を有効化して
いれば「候補可視 → backspace を出さず hold → `GJI_CANDIDATE_VETO_CAP_MS`
超過後も無回収の `Done` で打ち切る」という**既存の安全機構**により、回収
そのものが発動しなかった。

**長所**: BUG-30 という既に確立済みの安全機構の適用範囲を是正するだけであり、
新しい事後推測ではない。案Fと組み合わせると「deadline まで待つ→それでも
証拠ゼロ→ただし候補可視なら無回収で打ち切る」という、判定を間違えても実害が
出ない経路が既存機構だけで完成する。

**短所/caveat**: 42秒アイドルを跨いで前セッションの候補ウィンドウが残存して
いる可能性はゼロではない。`cold_seq` が新規である（＝前世代の残骸を現世代の
証拠として使わない、ADR-079 の趣旨）ことを条件に含める必要がある。ただし
これは「veto を有効にしてよいか」という**安全側に倒す判断**であり、BUG-75で
破綻した「回収を発動してよいか」という**破壊側に倒す推測**とはコストの
非対称性が異なる（誤ってveto発動＝最悪でも「literal化を見逃す」、誤って
veto不発動＝現状維持）。

### 【根本対策として並走】案C: 回収を「確認してから送る」設計に変える

現状の recovery は「ESC を送って composition を破棄したはず」という確認なしの
前進である。**ESCが確実に効いたなら、判定が誤りでも最終的な出力は正しくなる**
（composition 破棄 → romaji 再送 → 正しい文字）。重複が出るのは「ESCが
間に合わなかった／GJIが先に確定させた」場合だけであり、案Cは**検出精度を
改善せずに実害だけをゼロにできる唯一の案**という点で、他の案とは軸が異なる。

**v1 からの重要な訂正（round 1 premortem 指摘）**: 当初「候補ウィンドウの
HIDE イベントで確認する」という具体化を提案したが、**これは今回の誤判定を
引き起こした `gji_candidate_visible_now`（SHOW/HIDE）と同一チャネルであり、
BUG-75 の6ラウンドが棄却した「観測トリガと観測対象が同一チャネル」という
自己汚染パターンに該当する。** 具体的な破綻シナリオ: ESC が VK 'T' より
先に GJI に届く（composition 未成立）→ HIDE は発火しない → 待ち合わせが
タイムアウト → その間に GJI が 't' を処理して composition 開始 →
タイムアウト後の再送で「tと」/「っと」型の新しい文字化けが起きる。加えて
HIDE は `platform.rs::gji_on_end_composition` で `reset_literal_session_
confirmed()` のトリガでもあり（＝旧案Bが跨ぎたいセッション境界フラグの
リセット条件そのもの）、案Cと（削除した）案Bは HIDE の意味論を巡って
そもそも両立しない。

**HIDE 以外の確認チャネルが何になりうるかは本稿では未解決。** `route=
VisibleFencing`（候補可視）と `check_now` の write-stale/show-stale 分岐
（候補非可視のまま composition だけ存在、未確定子音のケース）とでは使える
シグナルが異なり、`DetectRoute` ごとに個別設計が必要（未決定事項参照）。
さらに ESC〜再送の間に await を挟む設計は、BUG-36 の順序保証
（reinit の `VK_IME_OFF` が preedit を commit するため BS/ESC より後に
送る必要がある）を壊しうる —— 待機中に give-up/reinit 経路が割り込むと、
確定済みリテラルが回収不能になる。**根本対策として有望だが、具体的な
確認チャネルの設計が本 ADR の範囲では終わっていない。**

### 【保留、案Fの効果測定後】案A: cold-start 専用の猶予延長

`EPOCH_FENCE_GRACE_MS` を一律に上げるのではなく、`gji_state` が
`OnCold(Long)` の場合にのみ適用される別の猶予定数を新設する案。
[tuning-constants.md](../../.claude/rules/tuning-constants.md) により実測が
必須だが、**現時点でこの実測データは存在しない**。加えて上記「訂正2」の
とおり、`fencing_active` は cold-start 直後は構造的に stale から始まるため、
GJI がゼロ I/O で合成を完了できるケースには猶予をいくら延ばしても効かない
可能性がある。さらに `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE=500`（`tuning.rs`）
は既に long idle 向けにこの定数ファミリーを一度分岐させた前例であり、
案Aは同じ役割の定数をもう一段（今度は grace 側で）分岐させることになる —
tuning-constants.md が Chrome probe 定数（20→100→200→350ms）で警告する
釣り上げパターンと構造が一致する。**着手するとしても、まず案Fの実装後に
「それでも足りないケースが実機で観測されるか」を見てから判断する。**

### 【要再設計、現状不採用】案D: per-VK confirm の「同期的・即断」を「遅延確定」に置き換える

v1 では「連続入力中なら次の文字の到着を待ってから判定する」という設計を
提案したが、round 1 で以下が判明し、**現状の形では採用しない**:

1. **論理誤り（round 1 architect）**: 「次の文字の到着」はユーザーの指に
   ついての事実であり、GJI についての事実ではない。証拠になるとすれば
   次 VK の confirm **結果**であって、到着そのものではない。
2. **逆効果（round 1 premortem）**: 待つ間に GJI が composition を確定
   （候補ウィンドウ HIDE）してしまう確率が上がる。確定後に発火する ESC は
   もう何も破棄できないため、**「即断せず待つ」は false positive を減らす
   代わりに、発火してしまった場合の重複確率を上げる**。ユーザーが1文字
   打って離席したケースが最悪シナリオ。
3. **未評価コスト（round 1 architect）**: (a) per-VK confirm は「ベースライン
   取得→1VK送信→判定→次VK」という厳密な逐次ループで、`LiteralDetector`
   は1インスタンスにつき1VK分のベースラインしか持たない。判定を先送り
   すると、次VKを送らずブロックする（output-gate 遅延をさらに悪化させる、
   ADR自身が問題視した現象を正常系に持ち込む）か、複数 `LiteralDetector`
   を並走させる（`show_stale_hold_since_ms` が前提する「1インスタンス
   につき1経路」を破り、後発VKの証拠が先発VKの判定に混入する ——
   ADR-079 が防ごうとした世代混同の再発）かのどちらかになる。(b) 判定を
   先送りしている間に N モーラ進んでいた場合、stale 確定時の回収対象は
   1モーラではなく N モーラになるが、`PARTIAL_LITERAL_BS=1`（「literal
   プレフィックスは経験的に1文字」という前提）や `failed_idx` 単位の
   設計はこれを想定していない。BUG-33 追補3・4（BS が別スコープの確定
   済み文字を消した実機バグ）を踏まえると、事故が起きたら不可逆な領域。

なお、[docs/experiments.md エントリ10](../experiments.md)（待機行列・捨て駒
キー撤去）との関係については round 1 で整理がついた: エントリ10が撤去した
のは*送信前*の予防的待機であり、案Dが変えようとしたのは*送信後*の verdict
確定タイミングであるため、発想として矛盾はしない。**案Dの問題はエントリ10
の蒸し返しではなく、上記1〜3の未評価コストである。**

### 【位置づけ変更、単独では不要】案E: cold-start 時は per-VK confirm の recovery を発動しない

v1 では「recovery のディスパッチを1箇所抑制するだけ」と書いたが、**これは
実装として誤り**（round 1 architect/premortem 双方が指摘）。
`probe_fsm.rs:561-601` の `StaleConfirm` 分岐は recovery を emit して
`return` する構造であり、recovery だけを抑制すると未送信の後続 VK
（"to" の 'o' 等）が一切送られないまま終わる —— これは
`probe_fsm.rs` 自身のコメントが「2026-07-22 の実機報告でこの構成が壊れた」
と明記する、**既知の regression の再現**である。正しく実装するなら
「confirmed とみなしてループを継続する」でなければならない。加えて、
recovery を止めると `consecutive_count` が増えず、GJI が本当に死んでいる
場合の最終手段である reinit（give-up 経路）に永久に到達しなくなる副作用が
新たに生じる。

位置づけとしては、`StaleConfirm` が「literal の証拠ではない」という
BUG-33 追補4 の原則を ESC+全体再送という（backspace より侵襲の大きい）
回収にも徹底するという意味で**既存原則の論理的帰結**ではあるが、
「これでできる」→「kれでできる」（同一 `VisibleFencing` 経路での反例）が
示すとおり無条件には成立しない。**案F+G が idx==0 のケースを実質的に
カバーするため、単独の対策としては現時点で不要と判断する。**

### 削除: 案B（`literal_session_confirmed` のセッション跨ぎ拡張）

v1 で提案したが round 1 で削除。理由:

1. **決定的な反例**: 2026-07-22「これでできる」→「kれでできる」は、同一
   アプリでの連続入力中の incident であり、直前セッションはほぼ確実に
   confirm 済みだったはず。案Bはこの唯一の documented true-positive を
   確実に握り潰す。
2. **cold-start では原理的に情報量ゼロ**: 「直前セッションで confirm
   成功」の実績は定義上アイドル前（warm期）のものであり、42秒アイドル後
   の GJI の状態については何も語らない。
3. **案Cとの意味論衝突**: セッション跨ぎで持ち越したい確認フラグと、案Cが
   使おうとする HIDE イベント（`reset_literal_session_confirmed()` の
   トリガそのもの）が同じチャネルを取り合う。
4. 実質的に「案Eの、不falsifiableな事前確率とBUG-39の既知の不正確さを
   上乗せした劣化版」（round 1 architect）。

## 未決定事項（round 1 で明確化）

- **テスト経路の訂正（round 1 双方が指摘、v1 の誤りを修正）**:
  `crates/awase-windows/tests/journal_replay.rs` は `state::conv_classify::
  classify_conv_transition` **専用**のフィクスチャ基盤であり、`LiteralDetector`
  の verdict はこの基盤の対象外。本報告の journal をそのまま入力にすることは
  **できない**（v1 の記述は誤り）。`tsf/probe.rs` のテストは
  `#[cfg(test)] #[cfg(windows)]` のため Linux 実行不可、かつ
  `check_now`/`visible_fencing_verdict` はグローバル観測 atomic
  （`TSF_OBS`、`gji_last_write_ms()`）と `current_tick_ms()` を直接読むため
  現状はフィクスチャ駆動にできない。
  - **案Fはまさにこの verdict ロジックの1行変更であり、`now`/`last_write_ms`/
    `epoch_send_ms`/`deadline_ms`/`hold_since` を引数に取る純粋関数として
    切り出せば `#[cfg(windows)]` から外れ、Linux でフィクスチャ駆動テスト
    （本 incident の journal に既に記録済みの `grace_hold_ms`/`last_write_ms`/
    `epoch_send_ms`/`deadline_ms` をそのままフィクスチャ化）が書ける。**
    案Gの `veto_eligible` 判定も同様に `(idx, cold_seq が新規か)` からの
    純粋関数にすれば同じ経路でテスト可能になる。
  - [fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
    を満たす経路は、この純粋関数抽出を先行させるか、それが間に合わない
    場合は (b) `docs/known-bugs.md` への追記で代替する。**実装に進む場合は
    純粋関数抽出を最初のタスクにすること。**
- **案A を評価する前提として、「verdict 確定後も一定時間 `last_write_ms` を
  追跡し続け、遅れて到着した write の実際の遅延を journal に残す」観測
  フェーズが必要**（現状の追補2フィールドは verdict 確定時にしか記録され
  ないため、「本当は何 ms 待てば間に合ったか」が原理的に取れない）。これは
  案Aの選択肢の一つではなく前提条件。
- **案C の具体的な確認チャネル**は本稿では未解決。`DetectRoute` 別に何を
  確認シグナルとして使うか（候補非可視・composition のみ存在するケースを
  含め）が次の検討課題。
- 案D と案E は相互に排他的（D は「待って正確に判定する」、E は「判定せず
  何もしない」で哲学が逆）。ともに現状不採用のため、この排他性は当面
  意思決定に影響しない。

## Round 1 レビューでの主な訂正（記録）

- architect・premortem 双方が指摘: v1 の根本原因記述「20ms猶予が短すぎる」は
  BUG-75元報告（+116msで真に着弾）と本incident（`write_ops_delta`全て0）を
  同一視する飛躍だった → 根本原因節を「まだ来ていない」と「そもそも来ない」
  の区別を明記する形に訂正。
- architect: `fencing_active = last_write_ms != 0` の構造上、cold-start
  直後は常に stale から始まり、GJI がゼロI/Oで合成完了できる場合は猶予を
  伸ばしても脱出できないことを指摘 → 根本原因節に追加。
- premortem: `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE=500` が既に同一定数
  ファミリーの一段目のエスカレーションであり、案Aがもう一段になることを
  指摘 → 案Aの短所に追加。
- architect: 案Eは「最もラジカルで根拠が薄い」という v1 の位置づけが不当。
  ESC+全体再送は backspace より侵襲的なのに同じ「証拠なし」判定で発動して
  いる既存の不徹底であり、BUG-33追補4の論理的帰結 → 位置づけを変更、
  ただし実装記述の誤り（後述）も訂正。
- architect・premortem 双方: 案Eの「recovery を1箇所抑制するだけ」という
  実装記述は誤り（未送信VKが失われる、2026-07-22の既知regressionの再現）
  → 正しい実装（confirmedとみなしてループ継続）を明記、reinit到達不能の
  副作用も追加。
- architect・premortem 双方: 案Bは案Eの劣化版、決定的反例あり、案Cとの
  意味論衝突あり → 削除。
- premortem: 案C（HIDE確認）はBUG-75が棄却した自己汚染パターンに該当する
  具体的破綻シナリオを指摘 → 案Cを「HIDE前提」から「確認チャネル未解決の
  根本対策候補」へ格下げ・再構成。BUG-36順序保証への影響も追加。
- architect: 案Fを新規提案（`grace_hold_verdict`の早期確定バグ修正、
  deadline到達まで待つだけ） → 第一候補として追加。
- architect: 案Gを新規提案（`veto_eligible`をidx==0/cold_seq新規に拡張、
  BUG-30安全機構の適用範囲是正） → 第一候補として追加、案Fと組み合わせて
  本incidentを実質的に防げることを明記。
- architect・premortem 双方: 案D「次のキー到着を待つ」の論理誤り（到着は
  ユーザーの事実でGJIの事実ではない）と、待つほど重複確率が上がる逆効果、
  per-VKループ不変条件の破壊、回収ペイロード会計の破綻を指摘 → 要再設計・
  現状不採用に変更。エントリ10との整合性自体はADRの主張どおり成立すると
  確認された。
- architect・premortem 双方: v1「journal_replayが現実的なテスト経路」は
  誤り（ConvClassifyFixture専用、LiteralDetector非対応）→ 未決定事項節を
  訂正、純粋関数抽出を先行タスクとして明記。
- premortem: 案Fは既存deadline予算内での確定タイミング変更に留まる一方、
  output-gate遅延（項目7）はA/C/Dいずれの滞留時間延長でも悪化するため
  独立した副次課題として切り出すべきではない → 該当記述を統合。
- architect: incident記述中の「StartComposition」がこの送信に対応する
  ものかは`route=VisibleFencing`の性質上不確実 → 断定を避ける記述に訂正。
- architect: `grace_hold_ms=31`はdeadline到達ではなく猶予切れの早期確定
  経路によるものと明記すべき → 本文中に追記。

## 関連

BUG-75（`docs/known-bugs.md`）、BUG-30（候補可視 veto の導入元）、BUG-33
追補3・4（`is_stale` で backspace を外した経緯）、BUG-35（epoch fencing
導入元）、BUG-36（reinit の送信順序保証）、BUG-39
（`literal_session_confirmed_gen` の世代付けと既知の不正確さ）、ADR-079
（epoch fencing）、[docs/experiments.md エントリ10](../experiments.md)
（待機行列・捨て駒キー撤去の経緯）、
[docs/bug-reports-triage.md](../bug-reports-triage.md)
（`01M1JGJNDJT9ZAEMRAEB58ES5A`, `01M1JJD54XQXSEJTHHFKV1WKA1` 該当行）、
[tuning-constants.md](../../.claude/rules/tuning-constants.md)、
[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)。
