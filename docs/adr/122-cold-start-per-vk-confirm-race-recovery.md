# ADR-122: GJI コールドスタート直後の per-VK confirm が「確認遅延」を「未着弾」と誤認し、回収送信が GJI 自身の非同期処理と競合してモーラが重複する（BUG-75 追加実機データに基づく再検討）

## ステータス

**Opus 2体 round 1〜3 完了。案Fを本ADRの decision として確定（実装は
下記4前提条件を満たしてから）。案G/G'は blocker 未解決のため本ADRの
スコープから外し、別ADR/将来課題へ切り出す（round 3 で architect・
premortem 双方が同一の分割案を提示、採用）。**

**[GitHub issue #149](https://github.com/cuzic/awase/issues/149) として
追跡。ユーザー判断により実装は一旦ペンディング（2026-09-03）。次に着手
する際は実装前提条件4（観測フェーズ）から開始すること。**

**収束の経緯**: round 1 で「案F+案Gの二段構え」を提案したが、round 2 で
「案Gは per-VK 経路（本incidentの経路）では no-op」と判明。round 2 で
安全な適用条件（ループローカルの confirm 実績）に絞った修正案を round 3
でレビューしたところ、architect・premortem 双方が独立に**2つの blocker**
を発見した——(1) 案Gの終端状態「候補可視のまま cap timeout で無回収
`Done`」は、`probe_fsm.rs:561-575` のコメントが明記する **2026-07-22
「これでできる」→「kれでできる」の revert 済み regression をそのまま
再生産する**（案Gが「保険をかける対象」として引用した実機報告を、案G
自身が引き起こす）。(2) 適用条件に使うつもりだった `pending_confirm`
（`probe_fsm.rs:534`）はループ先頭（`:464`）で毎回 `take()` されるため、
`StaleConfirm` 判定地点では**常に `None`**——round 2 で危険と結論した
「idx非依存」版と実質的に同じ挙動になってしまう。いずれも「記述の精度」
ではなく**設計レベルの穴**であり、round 3 内では解決しないため、両
エージェントの提案どおり案Gを本ADRの decision から外す。

- **案F（`grace_hold_verdict` の早期確定バグ修正、deadline まで判定を
  待つ）を decision として確定。実装前に以下4点が前提条件:**
  1. `grace_hold_verdict` を純粋関数へ切り出し、Linux fixture テストを
     書く（本 incident の journal に既に記録済みの `grace_hold_ms`/
     `last_write_ms`/`epoch_send_ms`/`deadline_ms` をそのまま入力にできる）。
  2. ADR-079 側で「猶予は deadline 到達後まで引き延ばさない」という
     早期確定を入れた理由を回収する（`grace_hold_verdict` の docstring
     `probe.rs:744-748` が明記する意図的設計、ADR-079 Opus レビュー
     欠陥2対処。experiment-logging.md の「なぜ前回それを捨てたのか」
     パターン、未実施）。
  3. `EPOCH_FENCE_GRACE_MS` の処遇を決める（案F後、`grace_hold_verdict`
     内で事実上死ぬ定数になる）。
  4. **【round 3 で新たに判明】本 incident の実際の deadline は
     `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE`（500ms）ではなく
     `RAW_TSF_LITERAL_DETECT_MS`（300ms）である**（`target=Chrome` の
     per-VK 送信は `probe_fsm.rs:676` で 300ms をハードコードしており、
     long-idle 分岐 `:144` の `is_tsf_mode` 条件を満たさない）。案Fが
     本 incident に与える猶予は 31ms→300ms にすぎず、これで足りるかは
     未知数。**実装前に「verdict 確定後も `last_write_ms` を一定時間
     追跡し、遅れて到着した write の実際の遅延を journal に残す観測
     フェーズ」を先行させる**（追補2と同じ挙動変更ゼロのパターン）
     ことを両エージェントが推奨。これなしに実装すると、効かなかった
     場合に「もっと待たせよう」（案A、定数の釣り上げ）へ流れる圧力が
     生まれる——tuning-constants.md が警告するエスカレーションの再現。
  5. 経路の絞り込みは `DetectRoute::VisibleFencing` ではなく
     `DetectPath::PerVk` を使う（round 3 premortem 指摘、後述）。
- **案G（per-VK の `StaleConfirm` 分岐への新設ゲート）・案G'（既存
  `veto_decision` への配線）はいずれも本ADRのスコープ外へ切り出す。**
  2つの blocker（終端状態の regression 再生産、`pending_confirm` の
  ライフタイム誤認）を解消しないかぎり実装できない。加えて、blocker
  (1) を「hold中はループ継続」に修正しても、案Gが実際に救える対象は
  「idx==0 の `StaleConfirm` で、その VK が実際には着弾していたケース」
  （BUG-75 元報告、msedge「つかって」）に限られ、**同じ route・同じ idx
  で着弾していなかった「kれでできる」と区別する信号がない**——これは
  BUG-75 の6ラウンドが到達した結論と同型であり、実質的に「案E（疑わしき
  は罰せず）を idx==0・候補可視に絞ったもの」に過ぎない。**本ADRでは
  この位置づけを記録するに留め、実装は案Fの実機データが出てから
  再検討する。**
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
- **案E（cold-start 時は recovery を無効化）は、案Gが実質的に「案Eを
  idx==0・候補可視に絞ったもの」であると round 3 で判明したことで、
  評価軸が案Gと同一であることが確定した。** 単独案としては不要（案Gの
  検討時にまとめて再検討する）。

以下、本文は上記の round 3 収束結果を反映した v4。round 1〜3 の訂正点は
文末「Round 1/2/3 レビューでの主な訂正」参照。

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
再開したところ、セッション最初のモーラ「と」（romaji `"to"`）の**2番目のVK
「o」を確認中**（`idx=1, last_idx=1`。1番目のVK「t」は`idx=0`で別途
confirm 済み — round 2 architect 指摘、v2 の「1文字目」という記述は自己矛盾
していたため訂正）に `StaleConfirm`（`route=VisibleFencing`）が発生し、
`escape_composition=true`
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
   `deadline_ms` 自体にはまだ余裕があった。**この deadline は
   `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE`（500ms）ではなく
   `RAW_TSF_LITERAL_DETECT_MS`（300ms）である**——`target=Chrome` の
   per-VK 送信は `probe_fsm.rs:676` で 300ms をハードコードしており、
   long-idle 分岐（`:144`、`is_tsf_mode` 条件）を通らない（round 3
   premortem 指摘、`docs/bug-reports-triage.md:54` の `target: Chrome`
   が一次証拠。v3 まで500msと誤認していた）。
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

## 根本原因（round 1〜3 レビューで訂正済み）

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

### 訂正4: `literal_session_confirmed` の無力さは「セッション最初の1文字」ではなく「直近の候補HIDE以降」に起因する（round 2 architect 指摘、訂正）

v2 では「per-VK confirm ループはセッション最初の1文字専用であるため、この
経路に来るケースは定義上つねに `literal_session_confirmed=false` である」
と書いたが、これは不正確。実際のゲートは `probe_fsm.rs:673` の
`env.literal_session_confirmed_gen != Some(cold_seq)` であり、
`reset_literal_session_confirmed()` は候補ウィンドウ **HIDE** で呼ばれる
（`platform.rs:816`、`gji_on_end_composition`）。したがって
`literal_session_confirmed=false` が意味するのは「セッション（`cold_seq`）
最初のモーラ」ではなく **「直近の候補 HIDE 以降で最初のモーラ」** —— 同一
`cold_seq` 内で2モーラ目以降であっても、1モーラ目が確定してHIDEした直後
なら再び per-VK confirm に入り `literal_session_confirmed=false` になる。
BUG-75 対話設計が提案した「(b) 既存の session 内 confirm 状態を使う」方向は、
「対象ケースそのものに原理的に適用できない」のではなく、**「同一 `cold_seq`
内で既に他のモーラが確定済みのケースを排除できない」という別の弱点を持つ**
（この弱点は後述の案G検証で決定的に効いてくる）。

## 検討したが採らない案（BUG-75 の教訓の再確認）

以下は BUG-75 の6ラウンド対話設計で既に破綻が確認された方向であり、
**本 ADR では再提案しない**（再提案する場合は新しい反証不能な根拠が要る）:

- suffix のみ再送（先頭 VK は着弾済みと仮定） — 反例あり（"kれでできる"）
- 各種「事後に着弾有無を推測するゲート」（`show_changed`/`consecutive`/
  `reinit_retry_tombstone`） — 証拠なし・自己汚染のいずれかで破綻
- `GCS_COMPREADSTR` による composition 文字列の直接読取 — 未確定子音の
  セマンティクスが不明、hwnd 配線の増設が必要、sync path に await を
  持ち込むトレードオフが未解決のまま棚上げ

## 決定案（round 3 収束後）

### 【decision・本incidentに効く可能性がある唯一の案】案F: `grace_hold_verdict` の早期確定を deadline まで先送りする

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

**「効く」は round 3 で「効く可能性がある」に弱めた（round 3 architect
指摘）**: 見出し・以下の記述は当初「本incidentに効く唯一の案」と断定して
いたが、これは本 ADR 自身の訂正1（「まだ来ていない」のか「そもそも来ない
（ゼロI/O）」のかは journal からは区別できない）と矛盾する。案Fが効くのは
GJI の I/O が 31ms〜deadline の間に**genuinely**到着していた場合に限られ、
現時点でその可能性は未確定。詳細は下記「実装前提条件4」参照。

**実装前提条件（round 2・round 3 で判明、実装前に要対応）**:

1. **`EPOCH_FENCE_GRACE_MS` を実質的に廃止する変更である**（round 2
   architect 指摘）。変更後は猶予内分岐と猶予切れ分岐が同一の式
   `(now >= deadline_ms).then_some(...)` に収束するため、`grace_hold_
   verdict` 内でこの定数は事実上死ぬ。定数を削除するか、`check_now` 側の
   別の意味づけとして残すかを実装タスクで明記すること。
2. **この早期確定は `grace_hold_verdict` の docstring
   （`probe.rs:744-748`）が明記する意図的な設計であり、ADR-079 の Opus
   レビュー欠陥2対処として書かれた行である**（round 2 premortem 指摘）。
   「猶予は『少し待てば追いつくかも』の窓であり、deadline 到達後まで
   引き延ばす理由はない」という当時の判断を反転させる前に、**ADR-079側で
   なぜこの打ち切りを入れたのかを回収する**（experiment-logging.md が言う
   「なぜ前回それを捨てたのか」パターン、未実施）。これは実装の前提条件。
3. **経路の絞り込みは `DetectRoute::VisibleFencing` ではなく
   `DetectPath::PerVk` を使う**（round 3 premortem 指摘、round 1〜2 の
   設計を訂正）。ルート分岐（`VisibleFencing` か `check_now` か）は
   `await_vk_detection` 冒頭（`probe_fsm.rs:377`）の
   `env.gji_candidate_visible_now` を**1 tick だけ読んだスナップショット**
   で決まる。候補ウィンドウが可視化するタイミングがこの読みの ±1 tick
   （10ms）ずれるだけで、同一の cold-start incident が `check_now` 側
   （SHOW エッジ発火 → `show_confirmed=true` → 20ms で早期確定、案Fの
   対象外）へ落ちる。つまり `VisibleFencing` 限定は「安全な絞り込み」に
   見えて、実際には**修正が確率的にしか効かない**設計になる。一方
   `check_now` の SHOW-only 分岐は「常時到達する warm/高速タイピングの
   通常ケース」（下記4）でもあり、そこへの適用はレイテンシ回帰になる
   ため単純な `check_now` 全体への拡大もできない。per-VK 経路は設計上
   「cold セッション最初のモーラ専用」（`probe_fsm.rs:673`、
   `gji_warmup_coro.rs:173-177` のゲート）であり warm 高頻度パスと構造的に
   重ならないため、**`DetectPath::PerVk` で絞れば `VisibleFencing`/
   `check_now` の両方を均一にカバーしつつ warm path 回帰を避けられる**。
4. **【round 3 で判明、最重要】本 incident の実際の deadline は
   `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE`（500ms）ではなく
   `RAW_TSF_LITERAL_DETECT_MS`（300ms）である。** `target=Chrome`
   （`docs/bug-reports-triage.md:54` が一次証拠）の per-VK 送信は
   `probe_fsm.rs:676` で `literal_detect_ms: RAW_TSF_LITERAL_DETECT_MS`
   を**ハードコード**しており、long-idle 分岐（`probe_fsm.rs:144`、
   `is_long_cold && env.is_tsf_mode` 条件）を通らない。案Fが本 incident に
   与える猶予は 31ms→300ms にすぎず、これで足りるかは現時点で不明。
   **実装前に、verdict 確定後も `last_write_ms` を一定時間追跡し「遅れて
   到着した write の実際の遅延 ms」を journal に残す観測フェーズを
   先行させる**（追補2と同じ挙動変更ゼロのパターン。両エージェントが
   同一の推奨）。これがあれば、訂正1が言う「まだ来ていない」／「そもそも
   来ない」を実データで分離でき、後者が支配的だと判明すれば案Fは本
   incident に効かないと分かり、案C（回収自体の安全化）へ優先度を移す
   判断ができる。**観測なしに案Fだけ入れると、効かなかった場合に「もっと
   待たせよう」（案A、定数の釣り上げ）へ流れる圧力が生まれる**——
   tuning-constants.md が警告するエスカレーションの再現であり、避ける
   べき順序。

**長所**: (1) 新しい定数を作らないため tuning-constants.md の実測義務が
発生しない。(2) `visible_fencing_verdict`/`check_now` 側のループは既に
「`Some` が返るまで tick ごとに呼び直す」構造のため呼び出し側の変更が
不要。(3) `DetectPath::PerVk` に限定する限り、deadline はこの経路が元々
待つ上限なので検出待ちのレイテンシ最大値は変わらない（ただし
output-gate 保持時間は下記短所(4)のとおり単独でも伸びる）。(4) 判定
ロジック内部に閉じ、新しい観測チャネルも事後推測も増やさない。(5)
`grace_hold_verdict` は既にほぼ純粋関数（`now`/`last_write_ms`/
`epoch_send_ms`/`deadline_ms`/`hold_since` を引数化すればよい）なので
Linux 上の fixture テストに落とせる（round 2 premortem 確認、本 incident
の journal に既に記録済みの値をそのままフィクスチャ化できる）。

**短所**: (1) 真に stale だったケースの回収が deadline まで遅れる（十数〜
数十ms）。ただし `StaleConfirm` の回収は `backs=0` のため、遅れても画面上の
破損は増えない。(2) 上記「実装前提条件」1〜4が未対応（うち4は round 3 で
新たに判明、最優先）。(3) 既存テスト（`probe.rs:1010-1219`）の期待値更新が
必要（Windows専用、CI待ち）。(4) **【round 3 architect 指摘】案F単独でも
output-gate 保持時間は 31ms→最大300ms（本 incident の経路。他経路では
最大500ms）に伸びる**——これは F+G を組み合わせた場合特有のリスクでは
なく案F単独の短所であり、以前は「組み合わせ設計」節にのみ書いていたが
ここへ移設した。回収が後ろ倒しになる間もユーザーは打鍵を続けるため、
最終的に `StaleConfirm` へ落ちた場合、ESC+再送は**より多くの後続キーが
溜まった状態で**割り込むことになり、重複の出方が悪化しうる（背景節
項目7が指摘する「回収中の遅延キーが連鎖的に誤判定を誘発しうる」条件を、
案Fは発生確率ではなく持続時間の側から押し上げる）。

**round 3 の全体評価**: architect は「案F単独ならマージ可能な水準に収束」
（上記前提条件1〜4を満たせば）と評価。premortem は「あと一歩」（前提条件
4の観測フェーズを経ないと『本incidentに効く』という結論の根拠が不足）と
評価。**両者の一致点は、案Fを単独の decision として切り出せば
fix-requires-evidence.md の (a) 回帰テスト要件を満たしつつ前進できる、
という点。**

### 【本ADRのスコープ外へ切り出し、別ADR/将来課題】案G・案G': per-VK の `StaleConfirm` 分岐に候補可視 hold を新設する

**【round 2・最重要】案G は round 1 の記述（`veto_eligible` の適用範囲を
拡張する）では no-op である。architect・premortem 双方が独立に同じ結論に
達した:**

- `veto_eligible()` の唯一の読み手は `literal_detect_fsm.rs:437-449` の
  `veto_decision`。
- `veto_decision` を呼ぶのは `literal_detect_fsm.rs:351`、
  `LiteralDetectCore::poll` の **`SuspectedLiteral` アームのみ**。
  `StaleConfirm`（本 incident の verdict）は `word_level_recovery_params`
  へ直行し、veto を一切参照しない（`:394-411`）。
- `LiteralDetectCore` を構築するのは word パス（warm の `LiteralDetectFsm`
  と `gji_warmup_coro.rs:224` の cold word パス）だけであり、本 incident の
  経路である per-VK パス（`probe_fsm.rs::run_per_vk_confirm`）は
  `LiteralDetectCore` を一切構築しない（`probe_fsm.rs` に `veto` の
  出現は0件、round 2 premortem が直接確認）。

したがって `probe_io.rs:660` の `veto_eligible=false` を `true` に変えても、
**その値を読むコードが per-VK 経路上に存在しない**。round 1〜round 2冒頭の
「BUG-30 という既に確立済みの安全機構の適用範囲を是正するだけ」という
売り文句は成立しない。実際に候補可視 hold を per-VK の `StaleConfirm`
分岐に効かせるには、`probe_fsm.rs:561-601` の `StaleConfirm` アームに
**新設のゲート**を追加する必要がある。この事実は本コードの誤解を招く
コメント（`veto_decision` の doc が「veto 対象外（per-VK Chrome パス...）」
と書いていた）に由来していたため、round 2 の過程でコメントを修正した
（`crates/awase-windows/src/output/probe_io.rs:655-670`、
`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs:432-444`、
挙動変更なし・commit で反映済み）。

**さらに、安全な適用条件に絞ると本 incident は案Gでは救えない**
（round 2 architect 指摘）。round 2 冒頭で「`cold_seq` が新規・前モーラ
確定実績なし（idx非依存）」へ広げた条件も誤りだった: `literal_session_
confirmed=false` は「セッション最初のモーラ」ではなく「直近の候補HIDE
以降で最初のモーラ」を意味するにすぎない（訂正4参照）。**round 2 では
これを `run_per_vk_confirm` のループローカル状態（`pending_confirm`、
`probe_fsm.rs:534` 相当）で置き換えられると考えたが、round 3 でこの
条件が実装として機能しないと判明した（blocker 2、後述）。**

- 意図した条件: このモーラ内でまだ1つも confirm していない（＝ idx=0 で
  初めて `StaleConfirm` になった） → hold してよい。このモーラ内で既に
  1つ VK が confirm 済み（idx>0） → その VK が開いた候補ウィンドウを
  今確認中の別の VK の証拠として使うことになり、BUG-30 が名指しで避けた
  「前の VK が開いた候補ウィンドウの誤用」になる → hold してはいけない。
- **round 3 architect の訂正**: idx==0 で hold する場合も、その候補
  ウィンドウは「idx==0 自身が開けた窓」である可能性が高く（idx==0 の VK
  は既に送信済みのため）、これを idx==0 自身の hold 根拠にするのは
  厳密には自己汚染である。安全条件として言えるのは「前の**VK**由来では
  ない」までであり、「自己汚染ではない」という round 2 の表現は言い過ぎ
  だった（結論=idx>0では hold しない、は変わらない）。

**本 incident は idx=1 であり idx=0 は confirm 済みなので、この安全な
条件では hold されない。** つまり「案Gで本 incident を救う」という
round 1〜round 2 冒頭の主張は、条件を正しく絞ると成立しなくなる。
**案Gは本 incident とは別の失敗パターン——`idx==0` で `StaleConfirm` が
発生するケース（BUG-75 が既に記録した2026-07-22「これでできる」→
「kれでできる」がまさにこの型）——への保険として位置づけを変更する。**

**【round 3・blocker 1】終端状態「無回収 `Done`」は、案Gが保険をかける
対象そのものを再生産する。** `probe_fsm.rs:561-575` の `StaleConfirm`
アーム冒頭コメントは、まさにこの挙動（検出のみ・recovery なしで単に
`Done` にする）を試して revert した記録である: 「まだ送信していない
後続 VK が一切送られないまま処理が終了し、既に送信済みの VK だけが
取り残されて消失・文字化けする実害が生じた（実機報告 2026-07-22:
『これでできる』→『kれでできる』）」。round 2 で書いた案Gの終端仕様
「`candidate_visible` のまま cap timeout に達したら無回収で `Done`」
（round 2 版）は、この revert 済み regression を**文言どおり復元する**。
すなわち同一 ADR 内で「kれでできる」を「案Gが守る対象」と「案Gが壊す
例」の両方に使ってしまっていた自己矛盾があった。逃げ道（cap timeout でも
残りの VK を送る）は「既送信プレフィックスを再送しない suffix-only 再送」
になり、これは BUG-75 で revert した方式そのものである。**案Gの終端は
文書化済みの2つの regression に挟まれており、ここを設計しない限り
前進できない。**

**【round 3・blocker 2】適用条件に使うつもりだった `pending_confirm` は、
判定地点で常に `None` である。** `pending_confirm` は `probe_fsm.rs:460`
で宣言され、**`:464`（ループ先頭）で毎回 `take()` される**。
`StaleConfirm` アーム（`:562`付近）に到達した時点では、直前の VK が
confirm 済みかどうかに関わらず**必ず `None`**。これをそのまま条件に
使うと、idx=1 でも hold が成立してしまい、round 2 で「危険」と結論した
「idx非依存」版と実質的に同じ挙動になる（BUG-30 が避けた自己汚染ケースで
hold してしまう）。round 2 の「`probe_fsm.rs:534` 相当」という記述も、
534行は `pending_confirm` への**代入地点**であって参照可能な状態では
ない点を取り違えていた。実装するには `let mut confirmed_in_mora = false;`
のような**専用の新規フラグ**が必要で、これは「ループローカル状態を使う
だけ」という round 2 の説明より明示的な追加実装を要する。

**代替案G'（architect 提案、round 3 で非推奨と判明）**: 新しい語彙・
新しいゲートを作るのではなく、per-VK の `StaleConfirm` を
`SuspectedLiteral` が既に使っている `veto_decision` と同じチェックに
通す案。**round 3 architect の再検討で非推奨と判明**: `veto_decision`
の `Expired → 無回収Done` セマンティクスが安全なのは word パスでは
「その時点で romaji 全体が既に送信済み」だからであり（`LiteralDetectCore`
は語をまとめて送った後に判定する）、per-VK では `idx` までしか送って
いないため同じ「打ち切り」が未送信 suffix の喪失になる（blocker 1と
同じ罠）。前提条件が異なるため「既存機構の再利用」という利点も実際には
出ず、加えて word パスと per-VK パスが同じ関数を共有すると、以後 word
パス側の veto を変更するたびに per-VK に副作用が出る結合が生まれる。
**案G' は不採用とする。**

**cap の扱い（architect・premortem 双方が同じ結論）**: `GJI_CANDIDATE_
VETO_CAP_MS`（`tuning.rs:96`）自身の doc コメントが「実測未了 —
暫定値」（「候補ウィンドウ可視 → I/O/SHOW 確定」までの実測遅延データが
まだ無く、300msは他の定数からの類推値に過ぎない）と明記している。
案Gがこの定数を新しい経路（per-VK）に持ち込むこと自体が、
tuning-constants.md が要求する実測根拠を満たさない定数の適用範囲を
広げる行為になる。**両エージェントの推奨は「cap/hold という2本目の
タイマーを持たず、deadline 到達時に `candidate_visible` を一度だけ評価
する述語にする」**（詳細は下記「タイマー設計」）。これなら
`GJI_CANDIDATE_VETO_CAP_MS` に一切触れずに済む。

**テスト可能性の非対称性（round 2 premortem 指摘）**: 案Fが変更する
`grace_hold_verdict` はほぼ純粋関数で Linux fixture テスト化できるのに
対し、案G/G' は `run_per_vk_confirm`（`#[cfg(windows)]` 配下の async
コルーチン）への新規実装になるため、現状は Linux で回帰テストが書けない。

**評価軸は案Eと同一である（round 3 architect 指摘）**: blocker 1 を
「hold中はループ継続」に修正して案Gが成立したとしても、案Gが実際に
救える対象は「idx==0 の `StaleConfirm` で、その VK が実際には着弾して
いたケース」（BUG-75 元報告、msedge「つかって」、候補SHOW+18msで着弾
確認済み）に限られる。**同じ route・同じ idx で着弾していなかったのが
「kれでできる」であり、両者を区別する信号がない。** これは BUG-75 の
6ラウンドが到達した結論「前提を反転させても必ずどちらかの実機ケースで
破綻する」と同型であり、**案Gの実質は「案E（疑わしきは罰せず）を
idx==0・候補可視に絞ったもの」に過ぎない**。「案Eは単独では不要」と
「案Gは保険として実装」という一見矛盾する2つの位置づけは、「案Eの
縮小版として案Gを扱う」という一貫した記述に整理すべき。

**round 3 の結論: 案G/G'は本ADRのスコープから外し、案Fの実機データが
出てから（別ADRまたは本ADRの追補として）再検討する。** blocker 1・2は
記述の精度ではなく設計レベルの穴であり、案Fの審査・実装を待たせるべき
ではない、というのが architect・premortem 共通の判断。

### タイマー設計（案G再検討時のための記録）

将来 案G を再検討する際、`GJI_CANDIDATE_VETO_CAP_MS`（実測未了の暫定値、
上記参照）と `RAW_TSF_LITERAL_DETECT_MS`/`_LONG_IDLE`（案Fが judge を
先送りする対象）という独立した2本のタイマーをどう組み合わせるかで
round 3 に議論があった。

- **「hold 開始を verdict 確定時にする」** → 案Fが verdict を deadline
  まで押し出した**後**にさらに cap 分 hold するため、ゲート保持が
  加算される（本 incident の経路では deadline=300ms・cap=300ms のため
  最大600ms、long-idle 経路では 500+300=800ms）。
- **「既存の `veto_started_at_ms.get_or_insert(now)` パターン（最初の
  Hold 時点で開始）を踏襲する」** → cap（300ms）が deadline（300〜
  500ms）到達**前**に必ず失効しうる、または本 incident の経路（300ms
  vs 300ms）では tick の丸め次第でどちらに転ぶか非決定になる
  （round 3 premortem が新たに指摘）。
- **推奨（architect・premortem 一致）: 2本目のタイマーを持たない。**
  案Fが既に「少し待てば追いつくかもしれない」役割を deadline 内で
  果たしているため、deadline 到達後にさらに待つ理由がない。
  「`deadline_ms` 到達時点で `StaleConfirm` かつ `candidate_visible==true`
  かつこのモーラ内で未confirmなら、回収せず次の VK へ進む（hold も cap
  も持たない、1回評価する述語）」という設計に単純化できる。これなら
  ゲート保持時間は deadline 止まり（案Fの分のみ）で済み、
  `GJI_CANDIDATE_VETO_CAP_MS` にも一切触れない。ただし blocker 1
  （終端状態の設計）自体は解消しておらず、「回収せず次のVKへ進む」が
  具体的に何を送る／送らないかの設計は依然として必要。

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
示すとおり無条件には成立しない。**round 3 で、案G（idx==0・候補可視限定の
新設ゲート）が実質的に「案Eをこの条件へ絞ったもの」であり評価軸が同一と
判明した（上記「決定案」節の案G参照）。したがって案Eを単独の対策として
別途検討する必要はなく、将来案Gを再検討する際にまとめて扱う。**

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

## 未決定事項（round 1〜3 で明確化）

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
    **案G/G'は round 2 で判明したとおり `run_per_vk_confirm`（async
    コルーチン）への新設実装になるため、この経路では現状テストできない**
    （round 1 時点の見込みを round 2 で訂正）。ループローカルの hold
    判定自体は純粋関数化できる可能性があるが、ループ継続を伴う制御フロー
    全体のテストには追加の設計が要る。
  - [fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
    を満たす経路は、この純粋関数抽出を先行させるか、それが間に合わない
    場合は (b) `docs/known-bugs.md` への追記で代替する。**実装に進む場合は
    純粋関数抽出を最初のタスクにすること。**
- **案A（および round 3 で案Fにも適用範囲が広がった）を評価する前提として、
  「verdict 確定後も一定時間 `last_write_ms` を追跡し続け、遅れて到着した
  write の実際の遅延を journal に残す」観測フェーズが必要**（現状の追補2
  フィールドは verdict 確定時にしか記録されないため、「本当は何 ms 待てば
  間に合ったか」が原理的に取れない）。これは選択肢の一つではなく、案F
  実装の前提条件4（decision案F節参照）でもある。
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

## Round 2 レビューでの主な訂正（記録）

ユーザー指摘（「案Fと案Gは組み合わせられるか」）を受け、round 1で
「idx==0」としていた案Gの適用条件を「cold_seqが新規・前モーラ確定実績
なし（idx非依存）」へ広げた版を作成し、architect・premortemへround 2の
レビューを依頼。両者が独立に、この訂正の土台ごと崩れる事実を発見した。

- architect・premortem 双方【最重要】: `veto_eligible`の唯一の読み手
  `veto_decision`（`literal_detect_fsm.rs:437`）は`LiteralDetectCore::
  poll`の`SuspectedLiteral`アームからしか呼ばれず、本incidentの経路
  （per-VK、`StaleConfirm`）は`LiteralDetectCore`自体を構築しない
  （`probe_fsm.rs`に`veto`の出現0件、両者が独立に確認）→ 案Gは現状
  no-op。「既存機構の適用範囲是正」ではなく新設ゲートが必要と全面的に
  書き直し。誤解の原因になっていた`veto_decision`のdocコメントと
  `probe_io.rs`側のコメントも実装タスクとして修正（別コミットで先行
  反映済み）。
- architect: round 2冒頭の訂正根拠2つがいずれも誤り。(1)
  `literal_session_confirmed=false`は「セッション最初のモーラ」では
  なく「直近の候補HIDE以降で最初のモーラ」を意味するにすぎず、同一
  `cold_seq`内で他のモーラが確定済みのケースを排除できない
  （`platform.rs:816`のHIDEトリガ + `probe_fsm.rs:673`のゲート）。(2)
  BUG-30の懸念は引用元コメント（`probe_io.rs:657`）が「前の**VK**」と
  明記しており「前モーラ限定」ではない → 訂正4を書き直し。
- architect: 安全な条件（ループローカルのconfirm実績、idx==0相当）に
  絞ると、本incident（idx=1、idx=0は確認済み）は案Gでは救えないと
  判明 → 案Gの位置づけを「本incidentへの対策」から「別incident（"kれで
  できる"型、idx==0のStaleConfirm）への保険」へ変更。
- architect: 案Gを新設ゲートとして実装する場合も、案Eで一度破綻した
  「hold中に`return`すると未送信VKが失われる」罠が同様に生じる → ループ
  継続型の実装が必要と明記。
- architect: 代替設計として案G'（per-VKのStaleConfirmを、SuspectedLiteral
  が既に使う`veto_decision`と同じチェックへ配線する）を提案 → 案Gと
  並記し round 3 で比較検討する論点として追加。
- premortem: 案Fは`grace_hold_verdict`のdocstring（`probe.rs:744-748`）が
  明記する意図的な設計（ADR-079レビュー欠陥2対処）を反転させる変更で
  あり、当時の理由をADR-079側から回収すべき → 実装前提条件として追記。
- premortem: 案Fの`check_now`（warm path）への影響は「実利が薄い」では
  なく、常時到達する分岐でのレイテンシ回帰 → `VisibleFencing`限定を
  既定にすべきと訂正。
- premortem: 案Fと案Gの同時有効化で、`GJI_CANDIDATE_VETO_CAP_MS`(300ms)
  と`RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE`(500ms)という独立した2本の
  タイマーが衝突し、hold開始のタイミング次第でeither「最大800ms保持」
  either「案Gが最も効いてほしいlong-idleケースで必ず失効」のどちらかに
  倒れる相互作用バグを発見 → 組み合わせ設計節に新設、round 3で明示的な
  設計が必要な論点として追加。
- premortem: output-gate保持時間の合算（F+Gで最大800ms）が未評価だった
  → 背景節項目7の批判が自分の第一候補にも当てはまることを明記。
- premortem: 「veto cap timeout超過後のliteral見逃しリスクは実例未確認」
  という round 1 の記述は誤り、実例（2026-07-22「kれでできる」）は
  ADR自身が既に引用していた → 訂正。
- premortem: 案Fはほぼ純粋関数化できテスト可能、案G/G'は`#[cfg(windows)]`
  下のasyncコルーチンへの新規実装でLinux回帰テストが書けない
  → テスト可能性の非対称性を明記。

## Round 3 レビューでの主な訂正（記録）

round2の改訂（案Gをループローカル`pending_confirm`条件・新設ゲートとして
再設計した版）をarchitect・premortemへround 3のレビューとして依頼。
両者が独立に、案Gの再設計自体に2つのblockerを発見し、案Fについても
実際のdeadline値の誤認という重大な訂正を発見した。

- architect・premortem 双方【blocker 1】: 案Gの終端状態「候補可視のまま
  cap timeout で無回収`Done`」は、`probe_fsm.rs:561-575`のコメントが
  明記する2026-07-22「これでできる」→「kれでできる」のrevert済み
  regressionをそのまま再生産すると判明（未送信の後続VKが失われる）。
  ADRが同一報告を「案Gが守る対象」と「案Gが壊す例」の両方に使っていた
  自己矛盾も発覚 → 案Gの終端仕様を要再設計と明記、blocker未解決のため
  本ADRのdecisionから除外。
- architect・premortem 双方【blocker 2】: 適用条件に使うつもりだった
  `pending_confirm`（`probe_fsm.rs:534`）はループ先頭（`:464`）で毎回
  `take()`されるため、`StaleConfirm`判定地点では常に`None` → round 2の
  「ループローカル状態を使う」設計が実装として機能しないと判明、専用の
  新規フラグが必要と訂正。
- premortem【最重要・案Fへの訂正】: 本incidentは`target=Chrome`
  （`docs/bug-reports-triage.md:54`が一次証拠）であり、Chrome per-VK送信は
  `probe_fsm.rs:676`で`RAW_TSF_LITERAL_DETECT_MS`(300ms)をハードコードして
  おり、long-idle分岐(`:144`)の`is_tsf_mode`条件を満たさない。round1〜2で
  「deadline=500ms」と誤認していたが実際は300ms → 案Fが本incidentに
  与える猶予は31ms→300msにすぎず、効くかどうかは未確定と訂正。実装前に
  「verdict確定後もlast_write_msを追跡する観測フェーズ」を先行させるべき
  と明記。
- premortem: F+Gのタイマー分析（300ms cap vs 500ms deadline）も、
  本incidentの経路では実際には300ms vs 300msの同値であり、tick丸め次第の
  非決定という第三の状態がある → 組み合わせ設計節の前提を訂正。
- architect: `GJI_CANDIDATE_VETO_CAP_MS`自身のdocコメントが「実測未了 —
  暫定値」と自認していることを発見 → 案Gがこの定数を新経路に持ち込むこと
  自体が実測義務を回避できていないと指摘、cap廃止案を後押し。
- architect・premortem 双方: F+Gのタイマー相互作用の解決策として「cap/hold
  という2本目のタイマーを持たず、deadline到達時に候補可視を一度だけ評価
  する述語にする」という第三の設計を提案 → 「タイマー設計」節として記録。
- premortem: 案Fの`VisibleFencing`限定は、ルート分岐が1 tick読みの
  タイミングレースで決まるため確率的にしか効かないと指摘、
  `DetectPath::PerVk`での絞り込みを提案 → 実装前提条件3を訂正。
- architect: 案F単独でもoutput-gate保持時間は31ms→最大300ms(本incident
  の経路)に伸びる、これはF+G固有のリスクではなく案F単独の短所 →
  「組み合わせ設計」節から案Fの短所へ移設。
- architect: 「自分が開けた窓を自分の証拠にする自己汚染ではない」という
  round2の表現は言い過ぎ、idx==0の候補窓もidx==0自身が開けた可能性が
  高い → 「前のVK由来ではない」までに表現を弱める訂正。
- architect: 案Gの評価軸は案Eと同一（「リテラル化見逃し」と「重複」の
  どちらのコストを取るかという同じ賭け）と指摘 → 案Eの位置づけ記述を
  案Gと整合させて修正。
- architect・premortem 双方: 「案F+Gの組み合わせ」という当初の枠組み自体を
  維持すべきでない、案Gはblocker未解決のまま本ADRのdecisionに含めるべき
  ではない → ADRを分割する方針を両者が独立に提案、採用。ステータス節・
  決定案の見出しを全面改訂。

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
