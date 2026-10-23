---
id: ADR-279
title: |-
  belief と実 IME のずれが続く窓では、打鍵列から日本語と判定したら残った明示 OFF の意図を取り下げて Engine を戻し、そのあとで drift 診断バルーンを撤去する
summary: |-
  所有者の方針(2026-10-22): 「belief と実 IME のずれが直らない窓では、belief に合わせて IME を書き直さず、利用者がやりたいこと(打鍵列から日本語かを判定する導線)を優先する。
  導線が効くなら drift 診断バルーン(「このアプリではIME状態を確認できません」)は撤去する」。
  BUG-110 追補1 の窓(Windows Terminal InputSite・Uwp + GJI、`FeedbackPolicy::Read`)では、明示 OFF の意図が残ったまま実 IME は ON で、drift correction が 341 秒送り続けても
  収束しなかった。今の導線(ADR-263 `lang_detect`)はこの窓に働かない。
  Opus レビュー 2 回で、導線の実装案には前提の未確認が多いと分かった(親指=無変換/変換が `excluded` になる・`IntentStore` が上書きする・Engine 非活性中の親指が IME を変える・
  TSF ネイティブ窓では観測が来ない)。
  **本 ADR の主体は段階 0(Windows CI での測定)の計画**。測定の結果を構成ごとに (a0)(a1)(a2)(b)(c) に分け、それぞれの道(結論を出さない・導線なしでバルーン撤去・別の手段を検討・
  既存の経路で足りる・導線を実装)を決める。導線の実装(D0〜D4)は輪郭だけを書き、詳細は段階 0 の結果の後に確定する。
status: |-
  決定済み(2026-10-22、所有者回答 Q1〜Q5 反映)→ Opus レビュー round 1・2 を反映して改訂(2026-10-22)。実装はまだ。
  **段階 0(測定)の結果待ち。Q1(observe でも動かす)は所有者の再確認待ち**(§7)。導線の実装の詳細は段階 0 の結果の後に確定する。
related_adr:
  - "ADR-132"
  - "ADR-179"
  - "ADR-187"
  - "ADR-191"
  - "ADR-212"
  - "ADR-263"
---

# ADR-279: ずれが続く窓では打鍵列の判定で残った明示 OFF の意図を取り下げ、そのあとで drift 診断バルーンを撤去する

## ステータスと読み方

**この ADR の主体は段階 0(§2、Windows CI での測定)の計画である。** 導線の実装(§3 の D0〜D4)は輪郭と守る制約だけを書く。詳細(型・関数・テスト)は、段階 0 の結果で道が (c) に決まった構成についてだけ、そのあとで確定する。

- 起草: 2026-10-22。同日に所有者の回答(Q1〜Q5)を反映し、Opus の敵対的レビュー round 1(blocker 2・major 6・minor 7)と round 2(major 5・minor 3)を反映した。
- 実装はしていない。
- 行番号はすべて origin/develop `9375c91e` 時点。
- 所有者の再確認が要るのは 2 点: Q1(observe でも動かす)と、(b) の構成でバルーンを撤去するときに「変換を押せば直る」を利用者が知る手段(§7)。

## 1. 背景

### 1.1 所有者の方針

> belief と実 IME のずれが直らない窓では、belief に書き直さず、ユーザーがやりたいこと(打鍵列から日本語かを判定する導線)を優先する。
> 導線が効くなら drift 診断バルーンは撤去する。

### 1.2 BUG-110 追補1 の窓

記録は `docs/known-bugs/BUG-110.md:52-89` と `docs/bug-reports-triage.md:64` にある(不具合報告 `01M1N5RMQ0HGX60VNG37DXWR26`。`8f9fe040` の本文は「BUG-109 追補1」と書いているが、記録は BUG-110 にある)。

- 窓: Windows Terminal + PowerShell。`Windows.UI.Input.InputSite.WindowClass`、Uwp、実質 TSF ネイティブ。
- IME: GJI。JIS 配列、`nicola_keytop.yab`。
- 発端: `vk=0xF3`(半角)の 1 回の押下を明示 OFF と解釈し、`last_intent` と `IntentStore` に OFF が残った。実 GJI は ON のままだった。
- 症状: Engine が非活性になり、打鍵が JIS 配列の文字(＠ など)のまま GJI に入った。
- 長さ: `FeedbackPolicy::Read` で、`attempts=162` から `243` まで、330 秒以上 `Send` を返し続けた(`BUG-110.md:74-81`)。
- バルーン: この事例のあと、ずれの継続時間で出すように変えた(BUG-110 追補2、ADR-132 Phase 1)。今のバルーンは「直らないずれを知らせる」もので、直す手段ではない。

この事例が今も起きるかは分からない。
- BUG-110 は「要確認」のまま(`BUG-110.md:10`)。
- 長く続いた原因の一部(`apply_force_on_for_imm_broken` が `VK_IME_ON` を周期的に送り、drift correction とぶつかった。BUG-110 追補3)は、ADR-179(`621bf93c`)で撤去済み。
- 今の develop は、実質 TSF ネイティブ窓ではポーリングを常に止める(`runtime/mod.rs:1115-1133`)。

物理 IME キーは今も `PhysicalImeKey` の意図を作るので、入口そのものは残っている(`awase-ime-core/src/state/explicit_press.rs:118,134`)。

### 1.3 今の導線(ADR-263)がこの窓で働かない理由

| # | 理由 | 根拠 |
|---|---|---|
| 1 | belief が OFF。`lang_detect_may_hold()` の前提 `lang_detect_engine_active()` が偽。書き込みも `skipped-belief-off`・`skipped-explicit-off` で止まる | `lang_detect_shadow.rs:253-273`、`src/lang_detect.rs:828-840` |
| 2 | 実質 TSF ネイティブ窓で、既定の `scope=unobserved` の対象外 | `lang_detect_shadow.rs:235-249` |
| 3 | エピソードの開始が窓切替か 3 秒の無操作だけ。同じ窓で打ち続けると始まらない | `src/lang_detect.rs:572-584` |
| 4 | 既定の `mode` が `Observe` | `src/config.rs:465-466` |
| 5 | 動作は「ひらがなキー」「半角英数トグル」だけ。「Engine を戻す」動作が無い | `lang_detect_shadow.rs:405-415` |
| 6 | 親指=無変換/変換の Down が `excluded` になり、エピソードが `ExcludedKey` で閉じる。既定の NICOLA 構成では「親指と文字の重なり」に届かない(round 1 B1) | `awase-windows-base/src/vk.rs:351-356`、`lang_detect_shadow.rs:154-157,165`、`src/lang_detect.rs:595,608-610` |

## 2. 段階 0: 実装の前に測る

どれも IME への新しい書き込みは足さない。CI の構成とスクリプトを足すだけ。

### S0-1. ずれが続くか、そのとき何が観測されているか(round 2 N2・N3)

**ずれの作り方**
- `e2e-ime.yml` の `sc-driftrecovery-ctrlmuhenkan-*`(`:564-570`)を流用する。BUG-110 と同じ向き(明示 OFF・実 ON)のずれを作る構成。
- これに Windows Terminal(InputSite)の窓を足す(`wt-probe.yml` の窓の起動を流用)。

**この作り方の限界**
- Ctrl+無変換は、コンボの意図の経路(`kp_stage_post_decision`)を通る。
- BUG-110 の発端は物理キーの意図(`write_physical_key`)で、経路が違う。
- 物理キーの経路は、実利用の journal でしか見られない。

**試行ごとに記録するもの**
- `check_drift_recovery.py` の分類(`gap_made`・`gap_not_made`・`corrected`・`not_corrected`)。
- ずれの間の次の 4 つ:
  - ハーネスが別に読んだ実 IME の開閉。
  - belief(`effective_open`)と Engine の活性。
  - `derive_any` が `HighSingle` を返していた時間の割合と、そのときのソース。
  - そのソースの値と、ハーネスが読んだ実 IME の開閉との一致率。
- `DriftGiveUpDiagnostic`(= バルーン)の件数と、窓のフィードバック方針(Read/Blind)。

### S0-2. Engine が非活性のとき、親指 1 打が何を起こすか(round 1 M3)

コードから読める予想:

| IME | 親指の役割 | Engine 非活性中の親指 1 打 | 根拠 |
|---|---|---|---|
| GJI / MS-IME | あり | awase が `SetOpen` を書く。左親指は明示 OFF を作り直し、右親指は明示 ON になって Engine が戻る(今の経路だけで直る見込み) | `src/engine/engine.rs:986-1009` `thumb_open_role_action` |
| GJI | なし | 生の無変換/変換が GJI に届く。GJI の既定は 無変換=直接入力、変換=ひらがな。左親指が IME を閉じ、右親指は何も変えない見込み | `awase-windows-base/src/vk.rs:339-341` のコメント |
| MS-IME | なし | 未確認 | — |

Alt なりすましの親指は、Engine が非活性だと Alt のまま届き、`excluded` になる(`hook.rs:240-276`)。

**測るもの**
- S0-1 の構成で、`gap_made` のあとに次の 3 種類の打鍵をする:
  - 右親指 1 打
  - 左親指 1 打
  - 親指と文字を重ねた打鍵列
- それぞれのあとに記録するもの:
  - 明示意図
  - `IntentStore`
  - 観測
  - Engine の活性
  - 実 IME
  - ずれが解けたか
- 構成は上の 4 通り(GJI / MS-IME × 役割あり / なし)。
- **drift correction は判定の間も `Read` の OFF を送り続ける**(round 2 n2)。awase 自身の OFF で実 IME が閉じた試行は、分けて数える。

### S0-3. observe の journal の `evidence` の分布(round 1 B1)

既存の observe の journal(CI と実利用の不具合報告)で、`LangDetectRecord.evidence` を数える。親指=無変換/変換の構成では `thumb-overlap` が 0 件のはず。

### S0-4. 実利用のバルーンの件数(round 2 N2)

不具合報告の journal で、次の 2 つを数える。
- `DriftGiveUpDiagnostic` の件数と向き(明示 OFF・実 ON か、その逆か)。
- 物理キーの意図(0xF3 など)が発端になった区間の数。

### 段階 0 の結果での分岐(**構成ごとに決める**、round 2 N2・N4)

| 道 | 条件(その構成で) | どうするか |
|---|---|---|
| (a0) | `GAP_NOT_MADE`(CI でずれを作れなかった) | 結論を出さない。S0-4 の実利用の件数だけで判断するか、ずれの作り方を変えて測り直す |
| (a1) | `gap_made` が十分にあり、ずれが 3 秒以上続かず、Engine の停止も残らない。**かつ** S0-4 で実利用の `DriftGiveUpDiagnostic` がほぼ 0 | 導線は作らない。バルーンだけを撤去する |
| (a2) | `gap_made` はあるが、awase がずれを観測しない(ハーネスは実 ON を読んでいるのに Engine が止まったまま) | D2 は観測を要するので使えない。バルーンも元々出ないので撤去しても害は無いが、BUG-110 の実害は残る。別の手段を検討する(本 ADR の範囲外。所有者に報告) |
| (b) | ずれは続くが、右親指(変換)1 打で今の経路だけで直る(役割ありの構成の見込み) | 導線は要らない。バルーンを撤去するなら、「変換を押せば直る」を利用者が知る手段を所有者に確認する(§7) |
| (c) | ずれが続き、今の経路でも直らない(役割なしの GJI の見込み) | D0〜D4 を実装する(§3)。S0-1 で一致率を確かめたソースがある場合に限る(N3) |

構成によって (b) と (c) が混ざるのが普通の見込み。その場合は、(c) の構成のために導線を入れる。バルーンの撤去は、すべての構成が (a1)・(b)・(c) のどれかで片付いてから行う(§5)。

## 3. 導線の輪郭(道 (c) の構成だけ。詳細は段階 0 の後に確定)

- **D0. 親指の除外を直す**
  - `observed_key`(`lang_detect_shadow.rs:160-190`)で、親指に分類された打鍵は `is_ime_mode_key` を除外の理由にしない。
  - **`observed_key` の時点では親指の役割は分からない**(round 2 N1)。
    - 無変換/変換には `shadow_action` が付かない(`vk.rs:255-268`、`runtime/mod.rs:640-672`)。
    - 役割は `enrich_*` が `kp_run_inner`(`key_pipeline.rs:63-66`)で付ける。これは取り込み口の `lang_detect_on_key`(`app/mod.rs:708`)より後。
    - したがって**役割付きの親指も証拠に数える**。役割の `SetOpen` が意図を変えたエピソードは、D2 の「エピソード中に向きが変わった」で弾く。
    - 役割の有無を入力に足す(殻が `Engine` の役割の表を引く)かどうかは、段階 0 の後に決める。
  - D0 は ADR-263 の `ime_on`/`ime_on_eisu` の利用者の挙動も変える(round 2 n1)。親指からエピソードが始まり、保持し、重なりで確定する。
    - S0-3 を見てから入れる。
    - 保持した親指の再生が NICOLA の同時打鍵の判定に影響しないかを確かめる。
    - ADR-263 の status にも追記する。
- **D1. 開始条件 `Trigger::Drift`**
  - `src/lang_detect.rs` の `Trigger` と `arm` に、ずれの継続の契機を足す。
  - ずれの判定は `evaluate_drift` の結果(明示 OFF・観測 ON、`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` 以上)を使い、再実装しない。新しい tuning 定数は足さない。
  - 再試行は制限しない。窓切替で破棄する(Q3)。
- **D2. 取り下げてよいかの判断**(純粋関数、`awase-ime-core/src/state/` に新設)。次をすべて満たすとき:
  - ずれが続いている。
  - 意図を外したと仮定した `resolve_open_at` が ON で、その根拠が **S0-1 で一致率を確かめたソースの `DeriveHigh`** である(round 2 N3。`ImmGetOpenStatus` を含む High のすべてを無条件には信じない)。
  - 同じ窓で、フォーカス追跡が追いついている。
  - エピソードの間に、意図・`IntentStore`・観測の向きが変わっていない。
  - 日本語の判定が親指と文字の重なりによる(Q2)。
  - 親指が `VK_SPACE` でない。
- **D3. 専用イベント `ImeEvent::ExplicitIntentRefutedByTyping { refuted_target }`**
  - 入口は `ImeStateHub` の 1 つだけ。
  - 入口で `last_intent` を外し、**同じ窓・同じ向きの `IntentStore` のエントリも消す**(前例は `apply_key_effect_prediction`、`platform_state.rs:307-345`。round 1 B2)。
  - IME へは何も書かない。
  - `UserImeSetIntent`(意図の偽装)は使わない。`ObserverReported` + Low も使わない(明示意図の分岐と `IntentStore` を越えられない)。
  - 構築箇所は `architecture_guard` と dylint `lints/ime_event_guard/src/lib.rs:70-81`(`ALLOWED_FNS`・`RESTRICTED_VARIANTS`)で固定する。
- **D4. 配線と設定**
  - `Trigger::Drift` のエピソードは保持しない。判定だけを使う。
  - 設定は Q1 の再確認(§7)で決める。どちらの案でも次を守る:
    - `exclude_apps` を尊重する。
    - `include_apps` を指定した利用者では、対象のアプリでだけ取り下げる(round 2 n3)。
    - 入力中継ツールは除く。
    - 設定の文言・設定画面・CHANGELOG を直す。
- **検証**
  - Linux: 純粋関数の単体テストと全列挙、`ImeStateHub::effective_open_at` で Engine の活性まで見るテスト、閉ループの BUG-110 型シナリオ。
  - Linux の閉ループは、`#[cfg(windows)]` の配線(`lang_detect_on_key`・`observed_key`・D4)を通らない。配線は Windows CI(S0-2 の構成に親指と文字を重ねる打鍵を足したもの)で確かめる。
  - 実機(dragonflyg4)では Ctrl+無変換でずれを作る。

## 4. 英数・判定不能・誤判定

- **英数・判定不能**: 何もしない。判定不能なら、ずれが続く限り次のエピソードでまた判定する。
- **誤判定の害**
  - 英字を打ちたい利用者を日本語と判定すると、Engine が戻る。歯止めは、親指の重なりだけを証拠にすること。
  - High の観測が誤り(実 IME は OFF)だと、ローマ字が素の英字として出る。歯止めは、S0-1 の一致率で確かめたソースだけを根拠にすること。
- **誤って開いたとき、この窓では検出する経路が無い**(round 2 N5)。
  - 実質 TSF ネイティブ窓はポーリングを止める。取り下げで drift correction(ずれの間の読み戻し)も止まる。そのため、そのあと新しい観測は来ない。
  - 閉じる手段は、物理 IME キーと窓切替だけ。これは ADR-132 B' の (iii) と同じ固着で、残るリスクとして認める。
  - 対策(取り下げ後しばらく観測を読み直す等)は、段階 0 の結果で (c) になった場合に、実装の詳細と一緒に検討する。

## 5. バルーンの撤去(Q4)

- 撤去するのはバルーンだけ。対象は `ir_notify_drift_giveup_diagnostic` の `show_tray_balloon` の呼び出しと文言。
- journal の `DriftGiveUpDiagnostic`・`DriftGiveUpIntervalEnded` は残す。
- バルーンは向きを問わずに出る(`drift_plan.rs:359-361`)。撤去すると、次の場合の通知も消える。
  - 逆向き(明示 ON・実 OFF、Q5 で扱わない向き)。
  - 英字だけを打つ利用者。
  - 親指を重ねない利用者。
- 撤去するのは、すべての構成が (a1)・(b)・(c) のどれかで片付いたとき。(b) では利用者への伝え方を所有者が決めたあと、(c) では導線を入れたあとの journal で、残る `DriftGiveUpDiagnostic` が導線で解ける区間だけになったことを確かめてから。
- (a0)・(a2) が残る構成があるうちは撤去しない。所有者が逆向き・未解決の通知も消えることを承知した場合は別。
- 撤去したら、ADR-132 Phase 1 の status に追記する。

## 6. 却下した案

- **belief に合わせて IME を書き直す**: BUG-110 追補1 で、`Read` の drift correction は 341 秒・243 回で収束しなかった。書き直す向きの元は、誤検出した明示 OFF だった。ADR-212 P6(awase は推測を IME に書かない)にも、所有者の方針にも合わない。
- **字形だけで判定する**: `TaskbarGlyph` は drift の根拠から除いている(`drift_correction.rs:166-170`)。それに、足りないのは IME の状態ではなく「利用者が日本語を打ちたいか」で、字形では分からない。
- **段階 0 なしでバルーンを今すぐ撤去する**: 所有者の条件(導線が効くなら)を満たすかどうかが分からない。
- **`ModeKeyPassedThrough` を流用する**: dispatch 元が 1 か所に固定されていて(`ime_model.rs:863`)、意味も違う。
- **ADR-132 の B'(明示意図の有界失効)・候補 1(矛盾検出中ラッチ)との関係**
  - 本案は B' と同じく意図を外す。違いは次の 3 点。
    - 外す条件に、利用者の打鍵(親指の重なり)と、一致率を確かめた High の観測の 2 つの独立した証拠を要する。
    - Conv だけ・Medium の合意だけでは外さない。
    - `GiveUp` ではなく `evaluate_drift` の継続時間を使う。
  - それでも、**誤って開いたことをこの窓で検出できない点は B'(iii)・候補 1 と同じ**(§4)。これは残るリスクで、本案はそれを High の観測の正しさ(S0-1 で測る)に賭けている。

## 7. 所有者の回答・再確認・未確認

### 回答(2026-10-22)

- **Q1**: この導線だけ `observe` でも動かす。**→ 再確認が要る(下記)。**
- **Q2**: 日本語の証拠は、親指キーと文字キーの重なりだけ。
- **Q3**: 再試行は制限しない。エピソードごとに判定し、窓切替で破棄する。
- **Q4**: 撤去するのはバルーンだけ。drift の journal の診断は残す。
- **Q5**: 逆向き(明示 ON・実 IME OFF)は今回扱わない。事例が出てから判断する。

### 再確認が要ること

**Q1(observe でも動かすか)**: 2 案を並べる。推奨は案 2。

| | 案 1: `lang_detect.mode = "observe"` でも動かす | 案 2: drift の側の専用スイッチ(既定 ON) |
|---|---|---|
| 既定の利用者 | belief と Engine の活性が変わるのに、設定の説明は「挙動を変えない」のまま(`config.rs:462-464`) | 変わる。ただし設定の名前と説明が、この動作そのものになる |
| 止める手段 | `mode = "off"` だけ。journal の観測も一緒に止まる | 専用スイッチを切る。observe の journal は残る |
| n-gram の表が読めないとき | `lang_detect` 全体が `Off` になり、この導線も黙って止まる(`lang_detect_shadow.rs:92-96`) | 重なりだけで動くよう、依存を分ける |

**(b) の構成の伝え方**: (b) の構成でバルーンを撤去するなら、「変換を押せば直る」を利用者がどうやって知るか(round 2 N4)。

### 未確認(段階 0 と実装で潰す)

- 今の develop で、ずれが続くか。そのときの `HighSingle` のソースと、実 IME との一致率(S0-1)。
- Engine が非活性のときの親指 1 打の効果。特に MS-IME の役割なしの構成と、役割なしの親指で `KeyEffectPredicted` が発火するか(S0-2)。
- 物理キーの意図(0xF3)を発端にしたずれの頻度(S0-4)。
- Engine の活性が `effective_open` だけから導かれるか。明示 OFF で別のラッチが立たないか。
- 取り下げ後に `desired_open=false` が残ったとき、それを読む消費者(GjiDirect の already-matched・`open_chain` の早期 exit 等)が何をするか。
- BUG-110 で TsfNative の窓に `Read` が使われた理由。
