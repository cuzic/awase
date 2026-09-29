---
id: ADR-203
title: |-
  GjiFsm の OffCold 固着を「send_keys 冒頭の level 突合」と「確かな ON 系イベントでの Reopen」で防ぐ(BUG-170: 固着→毎打鍵 per-VK confirm→StaleConfirm→ESC)
summary: |-
  report 01M3NBQA8KH2JN6S1PYHP8DJRF(GJI+Edge/Google Meet、「これでいい」→「でいい」)。c8bc1adc(ADR-090 A-2 warrant 強制)以降、物理キー(予測 open=true)で belief が ON になっても
  engine 起案の SetOpen が Unwarranted になり、`on_ime_applied` が同期せず(`platform.rs` の early return)、GjiFsm が OffCold に固着する。OffCold では `is_warm()==false` なので
  全打鍵が cold 経路(per-VK confirm)を通り、StaleConfirm(約15%/語)で `escape=true` の VK_ESCAPE が送られ、未確定の直前文字ごと消える。
  75eb3f60(予測経路1点だけ ImeOn を足す)は点パッチであり、半角/全角・sync_direction・OFF 方向・外部開閉は塞がらない。本 ADR は同期の入口の選択(決定)を定める。
status: |-
  起票(2026-09-29)。Opus round3 まで反映(検出点を `WindowsPlatform::send_keys` へ、OFF 同期を廃し ON 系イベントで Reopen、案C は別PR)、round4 の収束確認待ち。実装未着手。
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

## 決定(round3 反映版)

1. **(i) level 突合(ON 方向の主機構)。検出点は `WindowsPlatform::send_keys`(`platform.rs:1169`〜、`&mut self`)の冒頭、`self.output.send_keys(actions)` の前**。
   条件はすべて満たすとき: `actions` に `Char`/`Romaji`(Sequence 内含む)がある、`injection_mode != Unicode`、`output.needs_f2_probe()`(GjiFsm 戦略の実体であり種別推測ではないので INV-42 の K 軸ゲートには当たらない)、
   `gji_state == OffCold`、`!output.has_pending_tsf_work()`(probe 実行中は突合しない。次の send で拾う)。揃ったら `self.sync_gji(GjiFsmSync::OnImeOn{origin: BeliefSync})`。
   - 旧案(`send_romaji_*_gated` の冒頭)は不採用: `Output` は `&self` で `sync_gji`/`dispatch_gji_response` を呼べない(StartProbe→probe_id 保存・TsfProbeStarted 記録・LongIdle タイマー kill が要る)。
     また raw recovery の再送(`*_bypass_gate`、log 737 の `re-sending raw TSF literal`)でも発火し、走行中の古い段の `finish_probe_stage` が新 probe_id を奪って新 Authorized probe を倒す(`output/mod.rs:1719-1729`)。
   - 判定は `state/gji_direct_mechanism.rs` の純粋関数 `needs_belief_sync_on(send_has_romaji, mode, strategy_is_gji, state_label)` に置いてホストで全数テスト。
   - Unicode 注入モードは対象外(`send_romaji_as_unicode` は GjiFsm に KeyInput を送らず composition も迂回する。OffCold のままでも per-VK/ESC の害は無く、失うのは long-cold の defer だけで別件)。
   - 「エンジンがローマ字を送る ⇒ belief ON」: 不一致は engine 活性の更新遅れ(belief OFF→refresh 前の打鍵)か「IME OFF なのに Engine ON」(BUG-162 系)に限られ、後者はどのみちリテラル化するので
     GjiFsm を ON にしても悪化しない。**belief の誤りは (i) では直らないし、悪化もしない。**
2. **(ii) 確かな ON 系イベントでは、GjiFsm がどの状態でも開き直す(新イベント `GjiEvent::Reopen{gji_idle_ms}`)**。OffCold は通常の ImeOn、OnWarm/OnComposing/OnCold は OnCold(Short 以上、proactive)へ。
   対象: `KeyEffectPredicted{open: Some(true)}`(物理キー予測)、shadow toggle で ON に倒した瞬間、`sync_direction` の on キー(ユーザーが意図を宣言したキー)。
   `ImeOn` の「already on, ignored」は変えない。`handle_composition_reset` は Short で warm に留めるが Reopen は留めない、という違いを FSM 特性テストで固定する。
   **OFF は同期しない**(ImeOff は従来どおり awase の actuation〈Applied の receipt〉のみ)。OFF 同期が要る唯一の理由「次の ON で already on にならず cold にならない」は、ON キーで常に開き直すことで消える。
   これにより B3(belief 由来 ImeOff が deferred VK を捨てる/flush 順序)と OFF の信頼度の議論は不要。代償は、既に ON のときの ON キーで1語だけ per-VK になること(GJI の F2 は ON 中でも
   composition context を触り、F2NonTsf の CompositionReset が既に出ているので追加の損失は小さい)。
   `ImeApplySucceeded` は対象外(generation 付き awase actuation 専用で receipt が INV-42 で同期済み、重複するうえ Unwarranted を含まない)。shadow toggle は ON 方向のみ(向きは belief 次第で
   Imm32Unavailable では逆になりうるため)。
3. **origin を FSM を通して運ぶ**: `GjiFsmSync`/`Reopen`/`ImeOn` に origin(`Actuation` | `BeliefSync`)を持たせ、`GjiAction::StartProbe` は origin を持たないので `ProbeParams` に `suppress_reinit` を足す等で
   `dispatch_gji_response` の StartProbe の Unicode 分岐(`platform.rs:554-563`)へ届ける。`BeliefSync` 由来では long-cold reinit(`send_f22_f21_reinit`、フックコールバック内の VK_IME_OFF→ON)を行わない
   (ADR-191「awase は書かない」・ADR-090 A-2 の warrant を迂回しない)。(ii) は Unicode モードの窓でも発火するため必須。
4. **既存の点パッチの扱い**: `presync_applied_open_on`(HwndCacheRestored 相当、BUG-18)と `sync_ime_kind_from_observation` の `applied_open()==Some(true)` の ImeOn は、(i) が GjiFsm 作り直し(B2)も拾うため
   最終的には撤去可能。ただし本 PR では残し、(i) が e2e(b) で B2 を拾えると実証できてから別コミットで撤去するか決める。
5. **75eb3f60(予測経路の直呼び)は撤去**し、(ii) の Reopen に置き換える。撤去は本実装と**同じ PR**(単独だと OFF/開き直しの穴が残る/先に外すと何も同期しない)。
   `architecture_guard::key_effect_prediction_open_true_notifies_gji_fsm` も削除し、BUG-170.md の「修正」「回帰テスト」欄を書き換える。撤去コミット本文に
   「失敗による revert ではなく ADR-203 による置き換え(観測した失敗なし)」と明記(experiment-logging.md)。
6. **案C(OffCold で StartComposition を受けたら ON へ自己修復)は本 ADR の実装から分離し別 PR**とする。(i) が主機構で C 無しでも効果は成立し、猶予 N ms は tuning-constants.md により
   SHOW と OFF の遅延の実測が要る(実測前の暫定値は規約違反)。SHOW はクラス名だけで絞っている(`win_event_obs.rs:154-172`)ので、別 PR で窓・プロセスを確認する。
   C 導入後は自己修復の遷移自体を別 WARN(`self-heal`)で記録し、e2e (d) は `StartComposition while engine off` と `self-heal` の両方 0件を見る。
7. **予測が外れた場合**: 専用の戻す経路は作らない。Imm32Unavailable では ON 予測が外れてもエンジンは belief どおり送りどのみちリテラル化(GjiFsm では直らず害も増えない)、OFF 予測が外れても
   belief が OFF でエンジンは送らず、次の ON 系イベント(Reopen)か (i) で回復する。観測できる窓の外れ(`[key-effect-miss]`)は従来どおり belief 側で観測が勝つ。
8. **StaleConfirm → ESC が途中の語で既存の未確定文字を巻き込む問題は対象外**(BUG-171 として別起票)。GjiFsm を直しても残るため、本 ADR の検証で「文字消失が止まった」ことを効果の証明としない。
   **ADR-203 を入れても消失経路が残る具体的な順序**: 1語目の probe が `StartComposition` より前に Stale/recovered で終わると `WarmupAborted` → `OnCold(kind, NotStarted)` に戻り、
   候補窓は1語目の未確定文字で既に可視なので、2語目の per-VK が StaleConfirm → ESC で1語目を消す(BUG-171 そのもの)。
9. **可視性**: (i)/(ii) の同期は `GjiFsmTransition.trigger` に発生元を残す(`ImeOn(BeliefSync:level)`、`Reopen(BeliefSync:predict)` 等)。`architecture_guard` で GjiFsm の
   ImeOn/ImeOff/Reopen の呼び出し元(presync・kind 同期・(i)・(ii)・receipt)を列挙して件数を固定し、入口が増えたら検出できるようにする。

## 検証方針

- **journal 追跡(Opus round3、期待)**: 今回の journal を新設計で追うと、193002 で固着の起点が消え(予測 ON → Reopen → OnCold(Short))、Edge では1語目だけ per-VK(ChromeProbe cold=53)で StartComposition 後に AlreadyWarm、
  以後は warm 経路(per-VK/StaleConfirm/ESC なし)で4回目の消失は起きない。ただし1語目の probe が StartComposition より前に Stale で終わると決定8の経路が残る。実機で確認する。
- **Linux**: `needs_belief_sync_on`(`{OffCold,OnCold,OnWarm,OnComposing}` × 送信意図 × Unicode × 戦略 × pending tsf work)と (ii) の対象イベント判定を純粋関数で全数テスト。GjiFsm 特性テスト
  (Reopen が OnWarm/OnComposing から OnCold(Short 以上)へ落ちること、ImeOn は already on を無視すること、今回の journal の順序で OnComposing(AlreadyWarm) になること)。journal リプレイは同じ純粋関数に
  `elapsed_ms` から時刻を合成して流す表駆動(runtime が同じ関数を通らないと写しの検査になるため)。不変条件 I1「エンジンが送信した時点で GjiFsm が OffCold でない」、I2「OffCold で StartComposition を受けない」。
- **windows-latest e2e(判定は awase log の GjiFsm 遷移、文字消失は主判定にしない)**: (a) IME を閉じて起動 → **F2 注入の直前に OffCold であることを確認**(無ければ INCONCLUSIVE)→ `AWASE_TEST_INJECTION` 付き 0xF2 →
  最初の文字の前に `Reopen(BeliefSync:predict)` による OffCold→OnCold(0xF2 の予測より後に初めて起きたことが条件)、`StartComposition while engine off` 0件、Enter 区間ごとの `prepend_f2_warmup=true` が1回以下。
  (b) B2: IME ON のまま GJI 種別を検出し直させる → 打鍵 → (i) の `ImeOn(BeliefSync:level)` で OffCold 固着が解けること(点パッチ撤去の判断材料)。
  (c) 開き直し: 物理 OFF(TEST マーカー付き 0xF3 等)→ 1秒以内に物理 ON → 即打鍵。**PASS 条件は「ON 後の最初の語が cold 経路になる(`prepend_f2_warmup=true`)」**(開き直しの確認)。
  (d) 既存の全 e2e に `StartComposition while engine off` 0件の不変条件チェック。**修正前 FAIL・修正後 PASS の両方を実測してからマージ**。
- **step 0(マージ前必須)**: c8bc1adc 以降の既存 ts-*/sc-* artifact を `StartComposition while engine off` と `prepend_f2_warmup=true` の連続で grep。既存 CI が OffCold 固着で走っていた場合は
  BUG-168 の残りの失敗(文字重複)の読み方と ts-* のベースラインが変わる。

## 未決事項

- `HwndCacheRestored`(フォーカス復帰での belief 復元)を (ii) に吸収するか、presync のまま残すか(決定4 のとおり本 PR では残す)。
- `ProbeParams.suppress_reinit` 等、origin を FSM 経由で運ぶ具体形。
- `CHROME_LONG_IDLE_MS`(`gji_fsm.rs:1015`)と打鍵間隔の関係で、4回目の最初の語が OnCold(Long) になる場合の実測(ESC が消すのは最初の語自身の未確定文字だけ、BUG-171 の範囲外)。
