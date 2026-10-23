---
id: ADR-279
title: |-
  belief と実 IME のずれが続く窓では、打鍵列から日本語と判定したら残った明示 OFF の意図を取り下げて Engine を戻し、そのあとで drift 診断バルーンを撤去する
summary: |-
  所有者の方針(2026-10-22): 「belief と実 IME のずれが直らない窓では、belief に合わせて IME を書き直さず、利用者がやりたいこと(打鍵列から日本語かを判定する導線)を優先する。
  導線が効くなら drift 診断バルーン(「このアプリではIME状態を確認できません」)は撤去する」。
  BUG-110 追補1 の窓(Windows Terminal InputSite・Uwp + GJI、`FeedbackPolicy::Read`)では、明示 OFF の意図が残ったまま実 IME は ON で、
  drift correction が 341 秒・attempts 162→243 送り続けても収束しなかった。今の導線(ADR-263 `lang_detect`)は belief OFF・読める窓・同じ窓での継続・既定 observe の
  4 つの理由でこの窓に働かない。
  決定案: `lang_detect` のエピソードに 3 つ目の開始条件「明示 OFF の意図と信頼できる観測 ON のずれが続いている」(`Trigger::Drift`)を足す。
  そのエピソードで日本語(親指と文字の重なり)と判定したら、新しい専用イベント `ImeEvent::ExplicitIntentRefutedByTyping` で `last_intent` を外す。
  IME へは何も書かない。`effective_open` は信頼できる観測(ON)に従い、Engine が戻り、drift correction も `NotExplicitIntent` で止まる。
  英数・判定不能では何もしない。導線を実装して closed-loop・Windows CI・実機で効くのを確かめてから、バルーンだけを撤去する(journal の診断は残す)。
status: |-
  決定済み(2026-10-23、所有者回答反映)。実装はまだ。未確認事項は実装段階で潰す(末尾「未確認」)。
  所有者回答: Q1 この導線だけ observe でも動かす(例外)・Q2 日本語の証拠は親指と文字の重なりだけ・Q3 再試行は制限なし(エピソードごと、窓切替で破棄)・
  Q4 バルーンだけ撤去し drift の journal の診断は残す・Q5 逆方向(明示 ON・実 IME OFF)は扱わない(事例が出てから)。
related_adr:
  - "ADR-132"
  - "ADR-187"
  - "ADR-191"
  - "ADR-212"
  - "ADR-263"
---

# ADR-279: ずれが続く窓では打鍵列の判定で残った明示 OFF の意図を取り下げ、そのあとで drift 診断バルーンを撤去する

## ステータス

**決定済み(2026-10-23、所有者回答反映)**。起草は 2026-10-22。実装はしていない。行番号はすべて origin/develop `9375c91e` 時点。
§7 の未確認事項は、実装の段階(D1〜D4 の PR)で潰す。
調査の元資料は、撤去可否の調査(同日、読み取りのみ)。本 ADR はその「導線を拡張してから撤去」という結論を、所有者の方針に合わせて決定案にしたもの。

## 1. 背景

### 1.1 所有者の方針

> belief と実 IME のずれが直らない窓では、belief に書き直さず、ユーザーがやりたいこと(打鍵列から日本語かを判定する導線)を優先する。
> 導線が効くなら drift 診断バルーンは撤去する。

### 1.2 BUG-110 追補1 の窓

記録は `docs/known-bugs/BUG-110.md:52-89` と `docs/bug-reports-triage.md:64`(不具合報告 `01M1N5RMQ0HGX60VNG37DXWR26`)。
コミット `8f9fe040` の本文は「BUG-109 追補1」と書いているが、分割後の記録は BUG-110 にある(今の BUG-109 は別件)。

- 窓: Windows Terminal + PowerShell(`Windows.UI.Input.InputSite.WindowClass`、app_kind=Uwp、実質 TSF ネイティブ)。
- IME: GJI。JIS 配列、`default_layout=nicola_keytop.yab`。
- 発端: `vk=0xF3 scan=0x29`(半角)の 1 回の押下を「物理 IME キーによる明示 OFF」と解釈し、`last_intent` に OFF が残った。実 GJI は ON のまま。
- 症状: Engine が非活性になり、打鍵が JIS 配列の文字(＠ など)のまま GJI に入った。
- 期間: `[drift] correction ... for 341743ms`。`FeedbackPolicy::Read`(`ImmGetOpenStatus`)で、同じ `Actuation` が `attempts=162` から `243` まで 330 秒間 `Send` を返し続けた(`BUG-110.md:74-81`)。
  不具合報告ダイアログへのフォーカス移動でようやく `last_intent` が消え、Engine が戻った。
- バルーンはこの事例を受けて、`GiveUp` ではなくずれの継続時間で出すように変えた(BUG-110 追補2、ADR-132 Phase 1)。
  つまり今のバルーンは「直らないずれを利用者に知らせる」ためにある。直す手段ではない。

物理 IME キーは今も `PhysicalImeKey` の意図を作る(`crates/awase-ime-core/src/state/explicit_press.rs:118,134`)。BUG-110 型の入口は残っている。

### 1.3 今の導線(ADR-263)がこの窓で働かない理由

| # | 理由 | 根拠(9375c91e) |
|---|---|---|
| 1 | belief が OFF。保持の前提 `lang_detect_engine_active()`(`engine.compute_active`)が偽で、`lang_detect_may_hold()` が偽 | `crates/awase-windows/src/runtime/lang_detect_shadow.rs:253-256,262-273`(`:266`) |
| 1' | 書き込みの確認でも先頭で `skipped-belief-off`、明示 OFF の意図で `skipped-explicit-off` | `src/lang_detect.rs:828-840`(`:830`・`:836`) |
| 2 | Windows Terminal は実質 TSF ネイティブで、`Read` が成立する読める窓。既定の `scope=unobserved` は `!is_effectively_tsf_native && is_demoted_by_learning()` だけを対象にする | `lang_detect_shadow.rs:235-249` |
| 3 | 341 秒、同じ窓で打ち続けている。エピソードは窓切替(`Trigger::WindowSwitch`)か 3 秒の無操作(`Trigger::Idle`)でしか始まらない | `src/lang_detect.rs:572-584` |
| 4 | 既定の `mode` は `Observe`(何も書かない) | `src/config.rs:465-466` |
| 5 | 導線の動作は「ひらがなキーを送る」「半角英数トグルに入る」だけ。この窓で要るのは「Engine を戻す」(実 IME は既に ON)ことで、その動作が無い | `lang_detect_shadow.rs:405-415` |

逆に、窓を切り替えると `last_intent` は `FocusChanged` で消え(`crates/awase-ime-core/src/state/ime_model.rs:1145`)、drift の評価は
`NotExplicitIntent` で止まる(`state/drift_correction.rs:139-141`)。バルーンの窓と導線の窓は構造的にほとんど交わらない。

## 2. 決定案

### D1. エピソードの 3 つ目の開始条件「ずれの継続」(`Trigger::Drift`)

- `src/lang_detect.rs::Trigger`(`:385`)に `Drift` を足す。`label()` は `"drift"`。
- `EpisodeTracker::arm`(`:573`)が、殻から渡される `drift_hint: bool` を受ける。`on_key_step`(`:518`)・`HoldController::on_key`(`:727`)の引数に足す。
  - エピソードが無く、`drift_hint` が真のときに武装する。
  - 窓切替・無操作より優先度は低い(どちらも成り立てば既存の契機を使う。判定の材料は同じ)。
  - **再試行の回数は制限しない**(所有者回答 Q3)。判定はエピソードごとに行い、判定が出ずに閉じたら(`Timeout`・`MaxKeys`)、ずれが続いている限り次の Down で
    また武装する。窓切替(scope の変化)でエピソードと武装は破棄する。回数の記録は持たない。
- 純粋なので、Linux の単体テストで固定する。

`drift_hint` は殻(`lang_detect_shadow.rs::lang_detect_on_key`、`:374`)が集める。事実は次の 3 つで、判断は D2 の純粋関数に置く。
- `ImeStateHub::explicit_intent()`(`crates/awase-windows/src/state/platform_state.rs:303`)が `Some(false)`。
- 信頼できる観測(drift と同じ `most_recent_trusted_excluding`、`ConvOpenInference`・`MozcShm`・`TaskbarGlyph` を除く)が ON で、`DRIFT_CORRECTION_OBS_MAX_AGE_MS` 以内。
- `observations.drift_duration(now)`(`observation_store.rs:630`)が `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`(3000ms)以上。バルーンと同じ閾値を使い、新しい tuning 定数は足さない。

これを読むための読み取り専用の口を `ImeStateHub` に 1 つ足す(仮名 `stale_explicit_off_facts(now) -> StaleOffFacts`)。belief のフィールドには触らない。

### D2. 取り下げてよいかの判断(純粋関数)

`crates/awase-ime-core/src/state/` に `intent_refute_plan.rs`(仮)を新設する。

```rust
pub struct RefuteFacts {
    pub explicit_intent: Option<bool>, // ImeStateHub::explicit_intent()
    pub trusted_open: Option<bool>,    // drift と同じ除外での最新の信頼できる観測
    pub trusted_fresh: bool,           // DRIFT_CORRECTION_OBS_MAX_AGE_MS 以内
    pub drift_ms: Option<u64>,
    pub same_scope: bool,              // エピソード開始時と判定時の前面窓が同じ
    pub focus_in_sync: bool,           // lang_detect_focus_in_sync()
    pub verdict_by_thumb_overlap: bool,// Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
    pub thumb_overlap_reliable: bool,  // Space を親指にした構成でない(hold_config_ok と同じ条件)
}
pub fn plan_refute(f: RefuteFacts) -> Result<(), RefuteSkip>;
```

- 取り下げるのは次のすべてを満たすときだけ。満たさない理由は `RefuteSkip` の列挙で返し、journal に残す。
  - 明示意図が OFF、信頼できる観測が ON で新しい、ずれが 3000ms 以上続いている。
  - 同じ窓で、フォーカス追跡が前面窓に追いついている。
  - 日本語の判定が**親指と文字の重なり**による(n-gram の対数尤度比だけの日本語判定では取り下げない。所有者回答 Q2、理由は §4)。
  - Space を親指にした構成でない(英文の Space と文字の重なりを親指の重なりと見分けられないため)。
- 判断の表は `drift_plan.rs` と同じく、全列挙のテストで固定する(`tests/` の exhaustive の形)。

`CORE_MODULES`・`.cargo/mutants-awase-*.toml` の `examine_globs` に新しいモジュールを足す。

### D3. belief への反映: 専用イベント 1 つ

`.claude/rules/ime-belief-architecture.md` の「真のユーザー操作ではないが belief を動かす場合は専用イベント」に従う。`UserImeSetIntent` は使わない(利用者の意図の偽装になる)。
`ObserverReported` も使わない(打鍵の判定は IME の状態の観測ではない)。

- `crates/awase-ime-core/src/state/ime_event.rs`(`enum ImeEvent`、`:399`)に足す:
  ```rust
  /// ADR-279: ずれが続く窓で、打鍵列から日本語と判定した。残った明示意図(`refuted_target`)を外す。
  /// `ImeStateHub::refute_explicit_intent_by_typing` 専用。desired_open は観測から導ける開閉があるときだけ揃える。
  ExplicitIntentRefutedByTyping { refuted_target: bool },
  ```
  struct 形式にする(unit variant は dylint の構築検出に掛からない、`ModeKeyPassedThrough` と同じ理由)。
- `state/ime_model.rs::reduce` の腕:
  - `last_intent.target == refuted_target` のときだけ `last_intent = None`(判定の間に利用者が別の明示操作をしていれば何もしない)。
  - `desired_open` は、観測から導ける開閉(`derive_any`)があればそれへ揃える。`ModeKeyPassedThrough { align_desired: true }` の腕(`:1062-`)と同じ扱いにし、
    共通の部分はヘルパーに出す。古い `desired_open=false` を残すと、別の経路が「観測 ≠ desired」と見て書き戻す(BUG-157 と同じ型)。
  - `applied` は、予測と食い違うなら `Unknown` へ落とす(`KeyEffectPredicted` の腕 `:1048-1058` と同じ。送信の省略を古い記録に基づかせない)。
- 入口は `ImeStateHub::refute_explicit_intent_by_typing(tick_ms)` の 1 つだけ(`platform_state.rs`、`pass_through_observed` `:776` と同じ形)。
  `tests/architecture_guard.rs` に「この variant の構築はこのメソッドの中だけ」のガードを足し、規約ファイルの「4 つのイベント」の節に 5 つ目として追記する。
- Engine を戻す別の書き込みはしない。Engine の活性は `effective_open` から導かれる(`compute_active`)。`last_intent` が無くなれば `effective_open` は信頼できる観測(ON)に従う。
  drift correction は `explicit_intent != Some(desired)` で `NotExplicitIntent` を返して止まる(`drift_correction.rs:139-141`)。**IME へは何も送らない。**

### D4. 殻の配線

- `lang_detect_shadow.rs::lang_detect_apply`(`:405`)で、`outcome.trigger == Trigger::Drift` かつ日本語の判定なら、D2 の事実を集めて `plan_refute` を呼び、
  `Ok` なら `refute_explicit_intent_by_typing` を呼ぶ。journal の `LangDetectRecord.action` は `"explicit-off-refuted"` か `RefuteSkip` のラベル。
- `Trigger::Drift` のエピソードは**保持しない**(Engine が非活性で、打鍵は既に OS へ流れている)。今の `"observe-only"`(`:437-442`)と同じ流れで判定だけ使う。
  したがって `may_hold` の条件(`mode`・`scope`)は変えない。
- **例外: この導線だけは既定の `mode = "observe"` でも動かす**(所有者回答 Q1)。`mode = "off"` では動かさない。IME に書かないが belief は変わるので、
  ADR-263 の「`observe` は挙動を変えない」の例外として扱う。**他の `lang_detect` の経路(保持・ひらがなキー・半角英数トグル)は `observe` では今までどおり何もしない。**
  `scope` と `include/exclude_apps` は見ない(ずれが観測で確かめられている窓だけが対象なので)。入力中継ツール(`InputRelay`)は除く。

## 3. 英数と判定したとき・判定不能のとき

- **何もしない**を既定にする。明示 OFF の意図を残し、drift correction とバルーン(撤去前)は今と同じ。
  - 英数の判定は「利用者は OFF のつもりで英字を打っている」と整合する。実 IME は ON なので打鍵は IME に入っているが、それを直すのは drift correction の仕事で、本 ADR は足さない(却下案 §6.1)。
  - `ime_on_eisu` の「半角英数トグルに入る」動作も、`Trigger::Drift` のエピソードでは呼ばない(belief OFF の窓でトグルの前提が成り立たない。`lang_detect_shadow.rs:337-339` の `entry_ime_ok`)。
- 判定不能(期限・打鍵数の上限)は、そのエピソードでは何もしない。ずれが続いていれば次のエピソードでまた判定する(回数の制限なし、D1)。
- journal には `verdict`・`trigger="drift"`・`action="none"` を残す(撤去後の分析の材料)。

## 4. 誤判定の害と歯止め

| 誤判定 | 起きること | 歯止め |
|---|---|---|
| 利用者は IME OFF で英字を打ちたいのに、日本語と判定 | 明示 OFF の意図が消え、Engine が戻る。実 IME が観測どおり ON なら、打鍵は IME に入りつつ Engine が仮名にする(今は JIS 配列の文字が IME に入っている。どちらも利用者の望みではないが、IME が ON であることは変わらない) | 親指の重なりの証拠だけを採る(英文では無変換/変換を文字と重ねない)。Space 親指の構成では採らない |
| 信頼できる観測が誤り(実 IME は OFF なのに ON と読めた) | Engine が戻り、ローマ字が素の英字として出る(BUG-002 型のリテラル) | 観測は drift と同じ「信頼できる観測」に限り、新しさ(`DRIFT_CORRECTION_OBS_MAX_AGE_MS`)を要する。ずれが 3000ms 以上続いていることも要する(一瞬の食い違いでは動かない) |
| 判定中に利用者が別の明示操作をした | 新しい意図を消してしまう | reduce の腕で `refuted_target` と今の `last_intent` を照合する。違えば何もしない |
| 窓を切り替えた直後の誤適用 | 前の窓の判定で次の窓の意図を消す | 窓切替で `last_intent` は既に消える。加えて `same_scope`・`focus_in_sync` を要する。エピソードは scope の変化で閉じる(`close_if_stale` `:561`) |
| 同じずれで何度も動く | journal が増える | 取り下げは 1 回で済む(取り下げたあとは明示意図が無いので開始条件が成り立たない)。判定不能の再試行は制限しないが(Q3)、エピソードは最大 7 打鍵・2.5 秒で、journal は 1 エピソード 1 行 |

判定の単位はエピソード(最大 7 打鍵・2.5 秒、ADR-263 の既定)のまま。「連続何打鍵」の新しい閾値は足さない(既存のエピソードの上限で足りるかは CI と実機の journal で確かめる、§5)。

## 5. 検証計画と撤去の順序

### 5.1 Linux(PR の CI で走る)

1. `src/lang_detect.rs` の単体テスト: `Trigger::Drift` の武装(判定不能のあとも再武装する・scope 変化で破棄・窓切替/無操作が優先)。
2. `intent_refute_plan.rs` の全列挙テスト(`RefuteFacts` の全組み合わせと `RefuteSkip` の優先順位)。
3. `ime_model.rs` の reduce のテスト: `refuted_target` 一致/不一致、`desired_open` の揃え(観測あり/なし)、`applied` の降格。
4. `crates/awase-windows/tests/closed_loop_scenarios.rs` に BUG-110 型のシナリオ(擬似 IME `tests/support/pseudo_ime.rs` を使う):
   物理 IME キーの OFF を誤検出 → 擬似 IME は ON のまま、信頼できる観測 ON が続く → 3 秒後に親指と文字を重ねる打鍵 →
   `last_intent` が消え、Engine が活性に戻り、drift の送信が止まる。対照として、英字だけを打つ列では何も変わらないこと。
5. `pseudo_ime_exhaustive.rs`(ADR-277)に、取り下げで新しい不変条件違反が出ないことを足す。
6. `architecture_guard.rs`: 新しい variant の構築箇所の固定。

### 5.2 GitHub Windows CI(既存構成の流用)

- `e2e-ime.yml` の `sc-driftrecovery-*`(`:559-570`)は「IME を外から閉じる」構成で、向きが逆(明示 ON・実 OFF)。
  BUG-110 型(明示 OFF・実 ON)を作るには、`drift-on` の stress に「物理 IME キーの OFF を awase に誤検出させる」手順が要る。新しい構成 `sc-driftrefute-gji-tsf`(仮)を足す。
  判定は awase.log/journal の `trigger="drift"` と `action="explicit-off-refuted"`、そのあとの `[drift] correction` の停止。
- Windows Terminal の窓は `wt-probe.yml` の窓の起動を流用する。CI で InputSite が再現できない場合は TSF のフォーム(`--form=tsf`)で代える(InputSite 固有の挙動は未確認として残す)。

### 5.3 実機(dragonflyg4)

- Windows Terminal + PowerShell + GJI、JIS 配列 `nicola_keytop.yab`。BUG-110 の発端の 半角キー(`vk=0xF3 scan=0x29`)を押してずれを作り、親指シフトで日本語を打つ。
- 確かめること: Engine が戻るまでの時間(エピソードの長さ)、取り下げたあと `[drift] correction` が止まること、英字だけを打ったときに何も変わらないこと。
- journal の `DriftGiveUpDiagnostic` の件数を、撤去前の数日分の実利用で数える(バルーンがどれだけ出ていたかの基準)。

### 5.4 撤去の順序

1. 導線(D1〜D4)の PR。バルーンは残す。
2. 5.1・5.2 が緑、5.3 で BUG-110 型が直ることを確かめる。
3. バルーンだけを撤去する(所有者回答 Q4。調査の案 A。`ir_notify_drift_giveup_diagnostic` の `show_tray_balloon` 呼び出しと文言、約 5〜15 行)。
   **journal の `DriftGiveUpDiagnostic`・`DriftGiveUpIntervalEnded` は残す**(BUG-110 追補3 で不具合報告の分析に実際に使った。導線が効かなかった事例を後から見つける唯一の材料)。
   関数名を残せば `architecture_guard.rs:3440-3507` の件数ガードと `drift_plan_table.txt` の golden は変わらない。
4. 診断ごとの撤去(調査の案 B、本番約 150 行・テスト約 130 行・golden 差し替え)は、本 ADR の範囲外。導線を入れて数か月の journal で
   `DriftGiveUpDiagnostic` が「取り下げで解消した区間」しか無いと分かってから、別に判断する。
5. ADR-132 Phase 1 の status に「バルーンは ADR-279 で撤去」と追記する。

## 6. 却下した案

### 6.1 belief に合わせて IME を書き直す(drift correction を強める・延長する)

BUG-110 追補1 で、`FeedbackPolicy::Read` の drift correction は 341 秒・attempts 162→243 の `Send` を返し続けて収束しなかった(`BUG-110.md:74-81`)。
しかも書き直す向きの元は誤検出した明示 OFF で、利用者の望み(日本語入力)と逆だった。ADR-212 P6 は「awase が自分の推測を IME に書かない」方向に絞っており、
ここで書き込みを足すのはその流れに反する。所有者の方針(belief に書き直さない)とも合わない。

### 6.2 字形(タスクバーの入力インジケーター、ADR-261・ADR-273)だけで判定する

字形は「複数の証拠の 1 つ、取れなければ諦める」位置づけで(ADR-261)、`TaskbarGlyph` は drift の根拠から明示的に除いている(`drift_correction.rs:166-170`)。
またこの窓では実 IME が ON であることは既に `Read` で読めている。足りないのは「IME の状態」ではなく「利用者が日本語を打ちたいか」で、それは字形では分からない。

### 6.3 バルーンを今すぐ撤去する(導線なし)

追補1 の「長いずれで何も出ない」状態に全ての窓で戻る。所有者の条件(導線が効くなら)を満たさない。

### 6.4 `ModeKeyPassedThrough` を流用する

reduce の効果はほぼ同じだが、その variant は `pass_through_observed` 専用で、dispatch 元をガードで 1 箇所に固定している(`ime_model.rs:863`)。
意味(モードキーの通過)も違い、journal で区別できなくなる。腕の中身はヘルパーで共有し、variant は分ける。

## 7. 未確認と所有者の回答

### 未確認(実装段階で潰す)

以下は D1〜D4 の実装の PR で、コードを読む・テストを書くことで確かめる。確かめた結果で決定案が変わる場合は本 ADR に追記する。


- Engine の活性が `effective_open` だけから導かれ、明示 OFF の押下で別のラッチ(engine deactivate のフラグ等)が立っていないか。BUG-110 のログの「vk=0xF3 で engine deactivate」が belief 経由かどうかをコードで確かめていない。
- Engine が非活性のとき、親指キー(無変換/変換)は IME へ素通りしている。素通りした無変換が GJI の状態を変え、判定の途中で観測が OFF に変わる場合の扱い(D2 の `trusted_open` で弾かれるはず、未検証)。
- `Trigger::Drift` のエピソードで、ADR-263 の判定器(最大 7 打鍵・2.5 秒)が JIS 配列の素通り中の打鍵でも親指の重なりを拾えるか(物理位置で `KeyRole::Thumb` を決めているかは `observed_key` を読んで確かめる)。
- BUG-110 で `Blind` 専用のはずの TsfNative 窓に `Read` が使われた理由(BUG-110 追補1 の未解明点)。本 ADR は方針に依らず働くが、CI で窓の再現をするときに効く。
- 実利用でバルーンがどれだけ出ているか(`DriftGiveUpDiagnostic` の件数)。降格された窓で信頼できる観測が入ってずれが起きる実例。
- ADR-191/212 の規律(明示意図を観測で外す)との整合。ADR-187(観測成功後に意図だけ外す)を前例として扱ったが、打鍵の判定で外す前例は無い。

### 所有者の回答(2026-10-23)

- **Q1**(既定の `observe` で働かせるか): **この導線だけ `observe` でも動かす**。例外として D4 に明記した。他の `lang_detect` の経路は `observe` のまま何もしない。
- **Q2**(日本語の証拠): **親指キーと文字キーの重なりだけ**。n-gram だけの日本語判定では取り下げない(D2)。
- **Q3**(判定不能のときの再試行): **制限なし**。エピソードごとに判定し、窓切替で破棄する(D1)。
- **Q4**(撤去の範囲): **バルーンだけ撤去し、drift の journal の診断(`DriftGiveUpDiagnostic`・`DriftGiveUpIntervalEnded`)は残す**(§5.4)。
- **Q5**(逆向き、明示 ON・実 IME OFF): **今回は扱わない**。事例が出てから別に判断する。本 ADR は明示 OFF・実 IME ON の向きだけを決める。
