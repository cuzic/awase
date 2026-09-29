---
id: ADR-203
title: |-
  GjiFsm を「belief の開閉変化」に追随させ、Unwarranted/予測経路で OffCold に固着して毎打鍵 per-VK confirm → StaleConfirm → ESC で未確定文字が消える不具合(BUG-170)を構造的に防ぐ
summary: |-
  report 01M3NBQA8KH2JN6S1PYHP8DJRF(GJI+Edge/Google Meet、「これでいい」→「でいい」)。c8bc1adc(ADR-090 A-2 warrant 強制)以降、物理キー(予測 open=true)で belief が ON になっても
  engine 起案の SetOpen が Unwarranted になり、`on_ime_applied` が同期せず(`platform.rs` の early return)、GjiFsm が OffCold に固着する。OffCold では `is_warm()==false` なので
  全打鍵が cold 経路(per-VK confirm)を通り、StaleConfirm(約15%/語)で `escape=true` の VK_ESCAPE が送られ、未確定の直前文字ごと消える。
  75eb3f60(予測経路1点だけ ImeOn を足す)は点パッチであり、半角/全角・sync_direction・OFF 方向・外部開閉は塞がらない。本 ADR は同期の入口の選択(決定)を定める。
status: |-
  起票(2026-09-29)。opus-adversarial-consult 待ち。実装未着手。
related_adr:
  - "ADR-089"
  - "ADR-090"
  - "ADR-191"
---

# ADR-203: GjiFsm を belief の開閉変化に追随させる

## 背景・事実(report 01M3NBQA8KH2JN6S1PYHP8DJRF の journal / awase.log で確認済み)

- 回帰の起点は `c8bc1adc`(2026-09-18、v1.21.0 に含まれない)。`ImeOpenOutcome::Unwarranted` は `record_ime_apply_result` で `NotSent`、`on_ime_applied_inner` で
  「同期義務なし」として early return するため、GjiFsm への `ImeOn`/`ImeOff` が届かない。ただし `legacy_gji_sync_obligation` は Unwarranted に `Some` を返す
  (除外は UnsafeToToggle/NotOwned のみ)ので、`platform.rs` のコメント(「同期義務は無い」)は Unwarranted については事実と異なる。
- journal: 191966 Ctrl+無変換 OFF は Applied → GjiFsm OffCold(同期あり)。193002 物理 F2 ON は `KeyEffectPredicted{open:true}` + Unwarranted で、GjiFsm は OffCold のまま(同期なし)。
  固着の起点は Edge ではなく Windows Terminal(193002)。以後 `StartComposition(candidate SHOW)` が OffCold のまま十数回観測される(=実 GJI は ON)。
- 報告された消失は4回目(215324〜、F2 の11.6秒後)。OffCold は `is_warm()==false`(`tsf/warmup/warmup_strategy.rs`)なので `prepend_f2_warmup` が常に true
  (log で true 13回・false 0回)。「で」で StaleConfirm(`per_vk_recovery_params(true, idx=1) → escape=true`)→ `VK_ESCAPE` が
  未確定の「これ」まで取り消す(awase.log.txt 723/735/737)。**cold-start 保護が効かなかったのではなく、毎打鍵かかっていた。**
- 75eb3f60 が塞いだのは `kp_predict_key_effect` の open=Some(true) だけ。未対応: 半角/全角(`shadow_action`、`kp_stage_shadow_ime_toggle` の OFF→ON は belief のみ)、
  `sync_direction` キー、消費された in_table キー、修飾付き、キーマップ未取得、OFF 方向全般、外部開閉(言語バー・PowerToys 等)。
  GJI 種別ゲートは INV-42(K 軸ゲート禁止)違反、`gji_on_ime_on` 直呼びは `GjiSyncSink::sync_gji` の迂回。OFF 未同期のまま ON だけ同期すると、
  OnWarm が残ったまま ON 直後の打鍵が warm 経路で即送信され Chrome/Edge の TSF 再初期化(~114〜326ms、BUG-002)前にリテラル化する新しい穴が開く。

## 検討する選択肢

- **案A(受け取りの境界で塞ぐ)**: Unwarranted でも `on_ime_applied` で GjiFsm 同期義務を実行する(`legacy_gji_sync_obligation` の意図どおり)。
  懸念: Unwarranted は「awase が IME へ書かなかった」ので、SetOpen の target が実 IME の状態と一致する保証がない(engine が ON を望んでも実 IME が OFF のまま、があり得る)。
  一致しない場合は GjiFsm を実 IME と逆に倒す。予測経路(物理キーで実 IME が動く)以外の Unwarranted を区別できない。
- **案B(belief の開閉変化から駆動)**: `effective_open` の変化点1箇所(runtime 層)で、変化の向きに応じ `sync_gji(OnImeOn/OnImeOff)` を出す(K 軸ゲートなし)。
  BUG-18(`presync_applied_open_on`)、`sync_ime_kind_from_observation`、75eb3f60 の点パッチを集約できる。懸念: state 層は GjiFsm に依存できない(ADR-065)ため
  変化検出は runtime で「前回同期した値」との差分で行う必要がある。belief 自体が誤っている場合(予測外れ)は GjiFsm も誤る(現状も belief は誤り)。
- **案C(自己修復)**: `OffCold` で `StartComposition`(実 GJI が composition 中という強い観測)を受けたら ON とみなして遷移する。原因が何であれ固着が1語で解ける。
  A/B と併用可能。懸念: 実 IME が OFF なのに候補窓イベントが来るケース(直後に OFF にした残像等)。

## 暫定の決定案(要 Opus レビュー)

1. 案B を採る(同期の入口を1箇所に集約、ON/OFF 両方向、種別ゲートなし、`sync_gji` 経由)。75eb3f60 の `kp_predict_key_effect` 内の直呼びは撤去する。
2. 案C を併用する(観測が無条件に勝つ自己修復)。ただし実 IME が OFF へ遷移した後の残像との区別方法を決める。
3. 案A は採らない(Unwarranted の target の正しさが保証されない)。ただし `platform.rs` の誤ったコメントは直す。
4. StaleConfirm → ESC が途中の語で既存の未確定文字を巻き込む問題は本 ADR の対象外とし、別 BUG(BUG-171)として起票する(GjiFsm が正しくても OnCold(Medium) 等で再発するため)。

## 検証方針

- Linux: 同期規則を純粋関数(`state/gji_direct_mechanism.rs`)に置き、`{Some(true),Some(false),None}` × 種別を全数テスト。`GjiFsm` の特性テスト
  (今回の journal の順序: ImeOff → FocusChange → ImeOn → CompositionReset → KeyInput → StartComposition → WarmupComplete で OnComposing(AlreadyWarm))。journal リプレイ fixture
  (`tests/journals/`)で「effective_open=true 後、次の StartComposition までに OffCold を出ている」「OffCold で StartComposition を受けない」を不変条件として assert。
- windows-latest e2e: IME を閉じた状態で awase を起動し(`GjiFsm::new()` は OffCold)、`AWASE_TEST_INJECTION` 付きで VK 0xF2 を注入(予測経路を通る)、
  awase log の GjiFsm 遷移(ImeOn による OffCold→OnCold、`StartComposition while engine off` 0件、`prepend_f2_warmup=true` が Enter 区間ごとに1回以下)で決定的に判定する。
  文字消失そのものは主判定にしない(StaleConfirm は確率依存)。**修正前 FAIL・修正後 PASS の両方を実測してからマージ**。
  先に安価な確認: c8bc1adc 以降の既存 ts-*/sc-* artifact に `StartComposition while engine off` / `prepend_f2_warmup=true` の連続が既にあるか(既存 CI が OffCold 固着で走っている疑い、BUG-168 の読み方に影響)。

## 未決事項

- 変化検出の置き場所(`Runtime` のどのイベント境界か)と、`effective_open`(IntentStore/予測込み)と `applied` のどちらの変化を trigger にするか。
- 予測が外れた場合(`[key-effect-miss]`)に GjiFsm を戻す経路。
- Unicode 注入モード + gji_idle ≥ 10s で ImeOn → `send_f22_f21_reinit` がフックコールバック内で awase 由来の IME 書き込みになる件(ADR-191「awase は書かない」との関係)。
