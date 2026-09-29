---
id: ADR-203
title: |-
  GjiFsm の OffCold 固着を「送信直前の level 突合」と「確かなイベント由来の開閉向き」で防ぐ(BUG-170: 固着→毎打鍵 per-VK confirm→StaleConfirm→ESC)
summary: |-
  report 01M3NBQA8KH2JN6S1PYHP8DJRF(GJI+Edge/Google Meet、「これでいい」→「でいい」)。c8bc1adc(ADR-090 A-2 warrant 強制)以降、物理キー(予測 open=true)で belief が ON になっても
  engine 起案の SetOpen が Unwarranted になり、`on_ime_applied` が同期せず(`platform.rs` の early return)、GjiFsm が OffCold に固着する。OffCold では `is_warm()==false` なので
  全打鍵が cold 経路(per-VK confirm)を通り、StaleConfirm(約15%/語)で `escape=true` の VK_ESCAPE が送られ、未確定の直前文字ごと消える。
  75eb3f60(予測経路1点だけ ImeOn を足す)は点パッチであり、半角/全角・sync_direction・OFF 方向・外部開閉は塞がらない。本 ADR は同期の入口の選択(決定)を定める。
status: |-
  起票(2026-09-29)。Opus round2 の指摘(edge 駆動の穴 B1〜B3)を反映して決定案を組み替え済み、round3 待ち。実装未着手。
related_adr:
  - "ADR-089"
  - "ADR-090"
  - "ADR-191"
---

# ADR-203: GjiFsm を belief の開閉変化に追随させる

## 背景・事実(report 01M3NBQA8KH2JN6S1PYHP8DJRF の journal / awase.log で確認済み)

- 回帰の起点は `c8bc1adc`(2026-09-18、v1.21.0 に含まれない)。`ImeOpenOutcome::Unwarranted` は `record_ime_apply_result`(generation=None)が `NotSent` を返し
  (`state/platform_state.rs:1131-1139`)、`runtime/mod.rs:925` の `drives_composition_side_effects()` が false になって `on_ime_applied` 自体が呼ばれない。
  `on_ime_applied_inner` の early return(`platform.rs:1471-1478`)はこの経路では到達しない。到達すれば `legacy_gji_sync_obligation`(Unwarranted に `Some`)で同期する作りなので、
  同ファイルの「同期義務は無い」コメントは Unwarranted について事実と異なる。
- GjiFsm が OffCold になる入口は物理キー予測だけではない。`sync_ime_kind_from_observation`(`runtime/message_handlers.rs:944-962`)は `set_active_ime_kind` で GjiFsm を**作り直し**(OffCold)、
  `applied_open()==Some(true)` のときだけ ImeOn で戻すが、ADR-191 以降 applied はほぼ更新されない(報告の state_snapshot も `applied: Unknown`)。GJI 種別の検出し直し
  (起動直後の gji-monitor 確定、TIP 切替)のたびに、IME が ON でも OffCold に固着しうる(`tsf/gji_monitor.rs:265-280` が 2026-07-07 に同現象を記録)。この入口では belief の値は変わらない。
- journal: 191966 Ctrl+無変換 OFF は Applied → GjiFsm OffCold(同期あり)。193002 物理 F2 ON は `KeyEffectPredicted{open:true}` + Unwarranted で、GjiFsm は OffCold のまま(同期なし)。
  固着の起点は Edge ではなく Windows Terminal(193002)。以後 `StartComposition(candidate SHOW)` が OffCold のまま十数回観測される(=実 GJI は ON)。
- 報告された消失は4回目(215324〜、F2 の11.6秒後)。OffCold は `is_warm()==false`(`tsf/warmup/warmup_strategy.rs`)なので `prepend_f2_warmup` が常に true
  (log で true 13回・false 0回)。「で」で StaleConfirm(`per_vk_recovery_params(true, idx=1) → escape=true`)→ `VK_ESCAPE` が
  未確定の「これ」まで取り消す(awase.log.txt 723/735/737)。**cold-start 保護が効かなかったのではなく、毎打鍵かかっていた。**
- 75eb3f60 が塞いだのは `kp_predict_key_effect` の open=Some(true) だけ。未対応: 半角/全角(`shadow_action`、`kp_stage_shadow_ime_toggle` の OFF→ON は belief のみ)、
  `sync_direction` キー、消費された in_table キー、修飾付き、キーマップ未取得、OFF 方向全般、外部開閉(言語バー・PowerToys 等)。
  GJI 種別ゲートは INV-42(K 軸ゲート禁止)違反、`gji_on_ime_on` 直呼びは `GjiSyncSink::sync_gji` の迂回。OFF 未同期のまま ON だけ同期すると、
  OnWarm が残ったまま ON 直後の打鍵が warm 経路で即送信され Chrome/Edge の TSF 再初期化(~114〜326ms、BUG-002)前にリテラル化する新しい穴が開く。

## 検討した選択肢と結論

- **案A(Unwarranted でも `on_ime_applied` を通す = `drives_composition_side_effects` のゲートを外す)**: 不採用。理由: (1) エンジンが SetOpen を出したときしか動かず、すでに
  active・`NotRomajiInput`・GjiFsm の作り直しでは出ない。(2) Unwarranted には belief を変えない SetOpen(コマンド/ホットキー由来で IME は実際には動いていない)が混ざりうる(要確認)。
  ※「target が実 IME と一致する保証がない」は B(同じ effective_open 由来)にも当てはまるため、A を退ける理由にはしない。
- **案B'(旧案B「effective_open の変化で駆動」)**: 不採用。`effective_open` は時刻の関数(明示意図 TTL 30s → 予測 → 鮮度 3s の観測 → `desired_open`=既定 true、`state/ime_model.rs:458-480`)で、
  (B1) 打鍵もイベントも無いまま反転し、起動直後に偽の ImeOn が出、低信頼観測の揺れ(journal 221052/221053)が ImeOn↔ImeOff の往復になる。(B2) GjiFsm の作り直しは belief が変わらず拾えない。
  (B3) belief 由来の ImeOff は `CancelProbe` → `cancel_probe()` の `take_pending_deferred()` でユーザーの本物の打鍵(deferred VK)を捨てる。
- **案C(OffCold で StartComposition を受けたら ON へ自己修復)**: 保険として採る(下記)。

## 決定(round2 反映版)

1. **(i) level 突合(ON 方向の主機構)**: GJI へローマ字を送る直前(`send_romaji_batched_gated`/`send_romaji_as_tsf_gated` の冒頭、`KeyInput` shadow routing の手前)で
   「GjiFsm が OffCold なのにエンジンが IME 経由でローマ字を送ろうとしている」を検出したら `GjiSyncSink::sync_gji(OnImeOn)` を出す。エンジンがローマ字を IME へ送るのは belief が ON のときだけなので、
   この不一致はそれ自体が同期漏れの証拠。時刻反転・起動時既定値・観測の揺れに影響されず、入口(予測/半角全角/sync_direction/外部開閉/GjiFsm 作り直し)を問わず拾う。output 層で完結し ADR-065 に触れない。
   種別(K 軸)ゲートは付けない(INV-42)。判定ロジックは `state/gji_direct_mechanism.rs` の純粋関数に置き、output はそれを呼ぶだけにする。
2. **(ii) OFF 方向と「開き直し」は確かなイベントの向きだけ**: 物理キーの予測(`KeyEffectPredicted{open}`)、shadow toggle の向き、`ImeApplySucceeded`、中・高信頼の観測(DeriveHigh/Medium)。
   低信頼(HeuristicDefault/Low)と時刻による反転は使わない。state 側の純粋関数が `GjiFsmSync` 値型で同期義務を返し、runtime が `sync_gji` で実行する(ADR-089 §2.4 と同形)。
3. **belief 由来の ImeOff は deferred VK を捨てない**: `GjiEvent::ImeOff` に reason を持たせる(または同期前に `flush_pending_deferred_vks()`)。物理 OFF の場合、deferred の打鍵は OFF キーより前のものなので
   OFF より先に送り出すのが正しい順序。awase 自身が actuate した OFF(Applied)は従来どおり。
4. **同期由来の ImeOn では long-cold reinit(`send_f22_f21_reinit`、フックコールバック内の VK_IME_OFF→ON)を行わない**(ADR-191「awase は書かない」・ADR-090 A-2 の warrant を迂回しない)。
   `GjiFsmSync` に origin(`Actuation` | `BeliefSync`)を持たせ、`BeliefSync` の StartProbe では reinit を抑止する。
5. **案C は保険**: OffCold かつ「直近 N ms 以内に awase 起点・観測起点の OFF 同期が無い」ときだけ StartComposition で ON へ遷移(OFF 直後の候補窓残像対策)。N は tuning-constants.md に従い
   SHOW と OFF の遅延を実測して決める。(i) が効けば1語早く直るので、猶予は保守的でよい。
6. **75eb3f60(予測経路の直呼び)は撤去**する。撤去は本 ADR の実装と**同じ PR**で行い(単独だと OFF 未同期の穴が残る/先に外すと何も同期しない)、
   `architecture_guard::key_effect_prediction_open_true_notifies_gji_fsm` も削除、BUG-170.md の「修正」「回帰テスト」欄を書き換える。コミット本文に
   「失敗による revert ではなく ADR-203 による置き換え(観測した失敗なし)」と明記する(experiment-logging.md)。
7. **予測が外れた場合**: 観測できる窓(ImmCross 等)は (ii) の観測イベントで ImeOff/ImeOn により戻る。Imm32Unavailable(今回の Edge)は観測が無いが、ON 予測が外れても
   エンジンは belief どおり送りどのみちリテラル化(GjiFsm では直らず害も増えない)、OFF 予測が外れても belief が OFF でエンジンは送らず、次の打鍵の StartComposition(案C)か (i) で回復する。専用の戻す経路は作らない。
8. **StaleConfirm → ESC が途中の語で既存の未確定文字を巻き込む問題は対象外**(BUG-171 として別起票)。GjiFsm を直しても OnCold(Medium)・AbortedCold 等で残るため、
   本 ADR の検証で「文字消失が止まった」ことを効果の証明としない(消失0件でも ADR-203 の効果とは断定しない)。

## 検証方針

- **Linux**: (i) の突合条件と (ii) の「確かなイベント」判定を `state/gji_direct_mechanism.rs` の純粋関数に置いて全数テスト(`{OffCold,OnCold,OnWarm,OnComposing}` × 送信意図 × イベント種別 × 信頼度)。
  GjiFsm 特性テスト(今回の journal の順序: ImeOff → FocusChange → ImeOn → CompositionReset → KeyInput → StartComposition → WarmupComplete で OnComposing(AlreadyWarm))。
  `ImeOff(reason=BeliefSync)` で deferred が捨てられないこと(B3)を単体テストで担保。journal リプレイは同じ純粋関数に journal の `elapsed_ms` から時刻を合成して流す表駆動にする
  (runtime が同じ関数を通らないと写しを検査するだけになるため)。不変条件 I1 は「エンジンが送信した時点で GjiFsm が OffCold でない」、I2 は「OffCold で StartComposition を受けない」。
- **windows-latest e2e**(4構成、判定は awase log の GjiFsm 遷移で行い、文字消失は主判定にしない): (a) IME を閉じて awase を起動 → **F2 注入の直前に OffCold であることを log で確認**(前提条件、無ければ INCONCLUSIVE)→
  `AWASE_TEST_INJECTION` 付き 0xF2 → 最初の文字の前に ImeOn(O1 は「0xF2 の予測より後に初めて起きた」ことが条件)、`StartComposition while engine off` 0件、Enter 区間ごとの `prepend_f2_warmup=true` が1回以下。
  (b) B2 用: IME ON のまま GJI 種別を検出し直させる(起動後の最初の gji-monitor 確定または TIP 切替)→ 打鍵 → OffCold 固着の有無。
  (c) OFF 方向用: 物理 OFF(TEST マーカー付き 0xF3 等)→ 1秒以内に物理 ON → 即打鍵、ON 後の最初の語が cold 経路になるか。(d) 既存の全 e2e に「`StartComposition while engine off` 0件」の不変条件チェックを足す。
  **修正前 FAIL・修正後 PASS の両方を実測してからマージ**。
- **step 0(マージ前必須)**: c8bc1adc 以降の既存 ts-*/sc-* artifact を `StartComposition while engine off` と `prepend_f2_warmup=true` の連続で grep。既存 CI が OffCold 固着で走っていた場合は
  BUG-168 の残りの失敗(文字重複)の読み方と ts-* のベースラインが変わる。

## 未決事項

- (i) の検出点が `send_romaji_batched_gated` と `send_romaji_as_tsf_gated` の2本だけで十分か(他の送信経路、Unicode 注入モードのバイパス)。
- (ii) の「確かなイベント」の正確な列挙(`ImeApplySucceeded` が Unwarranted を含むか、shadow toggle の向きの信頼度)。
- 案C の猶予 N ms の実測。`win_event_obs.rs` の SHOW が窓・プロセスで絞られているか(別窓の候補窓による誤 ON の可能性)。
- Unwarranted に混ざる「belief を変えない SetOpen」の実在確認(案A不採用理由(2))。
