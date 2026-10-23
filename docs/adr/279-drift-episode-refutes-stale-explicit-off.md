---
id: ADR-279
title: |-
  belief と実 IME のずれが続く窓では、打鍵列から日本語と判定したら残った明示 OFF の意図を取り下げて Engine を戻し、そのあとで drift 診断バルーンを撤去する
summary: |-
  所有者の方針(2026-10-22): 「belief と実 IME のずれが直らない窓では、belief に合わせて IME を書き直さず、利用者がやりたいこと(打鍵列から日本語かを判定する導線)を優先する。
  導線が効くなら drift 診断バルーン(「このアプリではIME状態を確認できません」)は撤去する」。
  BUG-110 追補1 の窓(Windows Terminal InputSite・Uwp + GJI、`FeedbackPolicy::Read`)では、明示 OFF の意図が残ったまま実 IME は ON で、
  drift correction が 341 秒・attempts 162→243 送り続けても収束しなかった。今の導線(ADR-263 `lang_detect`)はこの窓に働かない。
  Opus レビュー round 1(blocker 2・major 6・minor 7)で、決定案がそのままでは成立しないと分かった:
  (B1) 親指=無変換/変換は `observed_key` で `excluded` になり、エピソードが `ExcludedKey` で閉じるので「親指と文字の重なり」の判定が構造上出ない。
  (B2) `last_intent` だけ外しても `IntentStore`(明示 OFF の有効期限 30 秒)が `effective_open` を上書きし、Engine は戻らない。
  さらに、Engine が非活性の間の親指の 1 打は IME の状態を自分で変える(役割があれば awase の SetOpen、無ければ GJI の 無変換=直接入力)ので、証拠を作るキーが前提を壊す。
  改訂後の決定: **実装の前に段階 0(測定)を置く**。今の develop で BUG-110 型のずれが信頼できる観測付きで 3 秒以上続くか、Engine 非活性中の親指 1 打が何を起こすかを CI で測り、
  その結果で 3 つの道(導線なしでバルーン撤去・既存の経路で足りる・D0〜D4 を実装)から選ぶ。D0〜D4 を実装する場合は、親指の除外を直し(D0)、
  取り下げは `last_intent` と同じ窓の `IntentStore` のエントリの両方を外し(D3)、成立条件に「意図を外した仮定で `resolve_open_at` が信頼できる根拠で ON」を足す(D2)。
status: |-
  決定済み(2026-10-22、所有者回答 Q1〜Q5 反映)→ Opus レビュー round 1(blocker 2・major 6・minor 7)を反映して改訂(2026-10-22)。実装はまだ。
  **段階 0(測定)の結果待ち。Q1(observe でも動かす)は所有者の再確認待ち**(レビューで危険を指摘された。§9)。未確認事項は段階 0 と実装段階で潰す。
related_adr:
  - "ADR-132"
  - "ADR-179"
  - "ADR-187"
  - "ADR-191"
  - "ADR-212"
  - "ADR-263"
---

# ADR-279: ずれが続く窓では打鍵列の判定で残った明示 OFF の意図を取り下げ、そのあとで drift 診断バルーンを撤去する

## ステータス

- 起草 2026-10-22。同日に所有者の回答(Q1〜Q5)を反映して決定済みにした。
- 同日、Opus の敵対的レビュー round 1(blocker 2・major 6・minor 7)を反映して改訂した。
  - レビューで、決定案(旧 D1〜D4)がそのままでは成立しないことが分かった。
  - **実装の前に段階 0(§2)を置き、その結果で道を選ぶ**形に改めた。
- 実装はしていない。行番号はすべて origin/develop `9375c91e` 時点。
- **所有者の再確認が要る点**: Q1(この導線を既定の `observe` でも動かす)。危険と代案を §9 に並べた。

## 1. 背景

### 1.1 所有者の方針

> belief と実 IME のずれが直らない窓では、belief に書き直さず、ユーザーがやりたいこと(打鍵列から日本語かを判定する導線)を優先する。
> 導線が効くなら drift 診断バルーンは撤去する。

### 1.2 BUG-110 追補1 の窓

記録は `docs/known-bugs/BUG-110.md:52-89` と `docs/bug-reports-triage.md:64`(不具合報告 `01M1N5RMQ0HGX60VNG37DXWR26`)。
コミット `8f9fe040` の本文は「BUG-109 追補1」と書いているが、分割後の記録は BUG-110 にある。今の BUG-109 は別件。

- **窓**: Windows Terminal + PowerShell。`Windows.UI.Input.InputSite.WindowClass`、app_kind=Uwp、実質 TSF ネイティブ。
- **IME と配列**: GJI。JIS 配列、`default_layout=nicola_keytop.yab`。
- **発端**: `vk=0xF3 scan=0x29`(半角)を 1 回押したのを「物理 IME キーによる明示 OFF」と解釈し、`last_intent` と `IntentStore` に OFF が残った。実 GJI は ON のままだった。
- **症状**: Engine が非活性になり、打鍵が JIS 配列の文字(＠ など)のまま GJI に入った。
- **期間**:
  - ログには `[drift] correction ... for 341743ms` と残っていた。
  - `FeedbackPolicy::Read`(`ImmGetOpenStatus`)で処理され、同じ `Actuation` が `attempts=162` から `243` まで 330 秒間 `Send` を返し続けた(`BUG-110.md:74-81`)。
  - 不具合報告ダイアログへフォーカスが移ったところで、ようやく解けた。
- **バルーンの位置づけ**:
  - この事例を受けて、バルーンは `GiveUp` ではなく、ずれが続いた時間で出すように変えた(BUG-110 追補2、ADR-132 Phase 1)。
  - 今のバルーンは「直らないずれを利用者に知らせる」もので、直す手段ではない。

**この事例が今も起きるかは分からない(M2)。**
- BUG-110 の状態は「要確認」で、v2 で作り直した後に今の条件で確かめた記録が無い(`BUG-110.md:10`)。
- 追補1 は v2 より前の 2026-09-04 のもの。
- ADR-132 の 3 件目の再現(BUG-110 追補3)では、長く続いた原因の一部が分かっている(`132-...md:556-575`)。
  - `apply_force_on_for_imm_broken` が周期的に `VK_IME_ON` を送り、drift correction の `VK_IME_OFF` とぶつかっていた。
  - この機構は ADR-179 領域 A(`621bf93c`)で撤去済み。
- 今の develop について、さらに分かっていること:
  - 実質 TSF ネイティブの窓では、ポーリングを常に止める(`runtime/mod.rs:1115-1133`)。
  - CI の測定ツールも、「TsfNative では閉じた後、awase は開閉を観測しない」と書いている(`tools/e2e/ime_key_matrix/check_drift_recovery.py` の冒頭)。
- 追補1 の窓が `Read` になった理由は解明されていない。
- このため、今の develop で同じずれが信頼できる観測付きで続くかどうかは、段階 0 で測るまで分からない。

物理 IME キーは今も `PhysicalImeKey` の意図を作る(`crates/awase-ime-core/src/state/explicit_press.rs:118,134`)。BUG-110 型の入口そのものは残っている。

### 1.3 今の導線(ADR-263)がこの窓で働かない理由

| # | 理由 | 根拠(9375c91e) |
|---|---|---|
| 1 | belief が OFF。保持の前提 `lang_detect_engine_active()`(`engine.compute_active`)が偽で、`lang_detect_may_hold()` が偽 | `crates/awase-windows/src/runtime/lang_detect_shadow.rs:253-256,262-273`(`:266`) |
| 1' | 書き込みの確認では先頭で `skipped-belief-off` になり、明示 OFF の意図があれば `skipped-explicit-off` になる | `src/lang_detect.rs:828-840`(`:830`・`:836`) |
| 2 | Windows Terminal は実質 TSF ネイティブ。既定の `scope=unobserved` は `!is_effectively_tsf_native && is_demoted_by_learning()` だけを対象にする | `lang_detect_shadow.rs:235-249` |
| 3 | 同じ窓で打ち続けている。エピソードが始まるのは窓切替(`Trigger::WindowSwitch`)か 3 秒の無操作(`Trigger::Idle`)だけ | `src/lang_detect.rs:572-584`(`arm` は `:574`) |
| 4 | 既定の `mode` は `Observe`(何も書かない) | `src/config.rs:465-466` |
| 5 | 導線の動作は「ひらがなキーを送る」「半角英数トグルに入る」だけ。この窓で要るのは「Engine を戻す」こと(実 IME は既に ON)で、その動作が無い | `lang_detect_shadow.rs:405-415` |
| 6 | **(B1)** 親指=無変換/変換の Down は `excluded` になる。エピソードの開始に使えず、途中に来るとエピソードが `ExcludedKey` で閉じる。そのため「親指と文字の重なり」は既定の NICOLA 構成では判定に届かない(§3 D0) | `crates/awase-windows-base/src/vk.rs:351-356`(`is_ime_mode_key_for_ime` は 0x1C/0x1D で真)。`lang_detect_shadow.rs:154-157`(`is_ime_key` が `is_ime_mode_key` を含む)・`:165`(`observed_key` の `excluded`)。`src/lang_detect.rs:595`(開始は `!key.excluded` だけ)・`:608-610`(`ExcludedKey`) |

(B1)の補足:
- ADR-263 の追跡器のテストは、親指を `excluded: false` として作った打鍵で試している(`src/lang_detect.rs:1101-1106`)。`observed_key`・`is_ime_key` には単体テストが無い。
- 既定構成の observe の journal には、`evidence="thumb-overlap"` が出ていないはず。
- ADR-263 の「日本語の確定はほぼ全部が重なり」は人工データでの話で、実機の経路では確かめていない。

## 2. 段階 0: 実装の前に測る(M2・M3・B1)

D0〜D4 を書く前に、次の 3 つを測る。どれも IME へ新しく書き込むものではない。CI の構成とスクリプトの追加だけで済む。

### S0-1. 今の develop で BUG-110 型のずれが続くか

- **流用する構成**: `e2e-ime.yml` の `sc-driftrecovery-ctrlmuhenkan-*`(`:564-570`)。
  - 物理 Ctrl+無変換で OFF にしても実 IME が ON のまま残る、というずれを作る。
  - これは BUG-110 と同じ向き(明示 OFF・実 ON)。`check_drift_recovery.py` の `gap_made`/`not_corrected` がまさにこの形。
  - 旧版の §5.2 は「向きが逆」と書いていたが、誤りだった。
- **追加する窓**: Windows Terminal(InputSite)。`wt-probe.yml` の窓の起動を流用する。
- **測るもの**:
  - ずれが 3000ms 以上続いた区間の数。
  - その間に信頼できる観測(`most_recent_trusted_excluding(Conv/MozcShm/Glyph)`)が `DRIFT_CORRECTION_OBS_MAX_AGE_MS` 以内の新しさで来ていたか。
  - `DriftGiveUpDiagnostic`(= バルーン)の件数。
  - 窓のフィードバック方針(Read/Blind)。

### S0-2. Engine が非活性のとき、親指 1 打が何を起こすか(M3)

旧版は「親指は IME へ素通りする」と書いていたが、正しくない。今のコードから読める範囲を次の表にまとめた。

| IME | 親指に IME 設定由来の役割 | Engine 非活性中の親指 1 打 | 根拠 |
|---|---|---|---|
| GJI | あり(無変換=OFF・変換=ON の役割) | awase が `SetOpen` を書く(明示意図・`IntentStore`・実 IME)。左親指は明示 OFF を作り直す。右親指は明示 ON になり、Engine が戻る(**今の経路だけで直る**) | `src/engine/engine.rs:986-1009` `thumb_open_role_action` |
| GJI | なし | 生の無変換/変換が GJI へ届く。GJI 既定は 無変換=直接入力・変換=ひらがな。**利用者の左親指が IME を閉じる**。学習表・GJI 設定の予測(`KeyEffectPredicted`)が付けば、`last_intent` と `IntentStore` も外れる | `crates/awase-windows-base/src/vk.rs:339-341` のコメント。予測の経路は `platform_state.rs:307-345`(親指で発火するかは未確認) |
| MS-IME | あり | GJI の「あり」と同じ(役割による `SetOpen`) | `engine.rs:986-1009` |
| MS-IME | なし | 生の無変換/変換が MS-IME へ届く。MS-IME の既定の割り当てでの開閉の変化は未確認 | 未確認 |

Alt なりすましの親指は、Engine が非活性のとき Alt のまま届き、`alt` 修飾として `excluded` になる(`hook.rs:240-276`)。この構成では重なりの証拠は取れない。

**測るもの**: S0-1 の構成に、ずれができた後(`gap_made`)に「右親指 1 打」「左親指 1 打」「親指と文字を重ねた打鍵列」を足す。上の 4 通りの構成で、それぞれ次を記録する。
- 明示意図・`IntentStore`・観測・Engine の活性・実 IME の開閉
- ずれが解けたか

### S0-3. 今の observe の journal の `evidence` の分布(B1)

既存の observe の journal(CI と実機の不具合報告)で、`LangDetectRecord.evidence` の分布を数える。
- B1 が正しければ、既定の親指構成では `thumb-overlap` は 0 件のはず。
- これは Q2 を決めたときの判断材料(「重なりだけで十分に取れる」)の確認でもある。

### 段階 0 の結果での分岐

| 結果 | 選ぶ道 |
|---|---|
| (a) S0-1 で、信頼できる観測付きのずれが 3 秒以上続く区間が 0(ずれが起きない、または観測が来ない) | バルーンは今のコードでは実質出ない。**導線を作らずにバルーンだけを撤去する**(§7)。根拠は S0-1 の件数。BUG-110 の状態の「要確認」も、この測定で更新する |
| (b) ずれは続くが、S0-2 で右親指(変換)1 打が今の経路(役割の `SetOpen`、または `KeyEffectPredicted`)で直す | 導線は要らない。直す手段は「変換を押す」になる。バルーンは撤去する(§7)。代わりの通知は所有者判断(§9 Q4) |
| (c) ずれが続き、今の経路でも直らない(役割なし・予測なし等) | D0〜D4 を実装する(§3)。S0-2 で、親指が IME を閉じてずれが自然に解けるかも見て、D2 の条件を合わせる |

## 3. 決定案(段階 0 が (c) のとき)

### D0. 親指の除外を直す(B1 への最小の変更)

- `lang_detect_shadow.rs::observed_key`(`:160-190`)を変える。
  - `key_classification` が `LeftThumb`/`RightThumb` のときは、`is_ime_mode_key` を `excluded` の理由にしない。
  - `is_ime_control`・`is_sync_key`・`shadow_action.is_some()` は、今までどおり除外の理由に残す。
  - 役割を持つ親指は、S0-2 のとおり awase が書き込むキーなので、除外のままにする。そのため D0 で重なりの証拠を取れるのは「役割の無い親指」だけになる。
- `observed_key` と `is_ime_key` を、`#[cfg(windows)]` の無い場所(`src/lang_detect.rs`、または `awase-ime-core`)へ純粋関数として出す。入力は `RawKeyEvent` から抜いた小さな事実の構造体にする。
  - Linux のテストで、`VK_NONCONVERT` の親指と文字の重なりが `Verdict::Japanese(JapaneseEvidence::ThumbOverlap)` になることを固定する。
  - 役割付きの親指は `excluded` のままであることも固定する。
- **D0 は ADR-263 の observe の journal も変える**(既定構成で `thumb-overlap` が出るようになる)。ADR-263 の status にも追記する。
- **成立しない構成**: 役割を持つ親指、Alt なりすましの親指、Space の親指。これらでは重なりの証拠は取れない。
  - 役割付きの構成は、S0-2 のとおり右親指 1 打で今の経路だけで直る。
  - Alt なりすまし・Space 親指の構成には、本 ADR は手段を持たない。証拠を n-gram に広げない限り無理(Q2 で却下済み)。

### D1. エピソードの 3 つ目の開始条件「ずれの継続」(`Trigger::Drift`)

- `src/lang_detect.rs` を次のように変える。
  - `Trigger`(`:385`)に `Drift` を足す。`label()` は `"drift"`。
  - `EpisodeTracker::arm`(`:574`)が、殻から渡される `drift_hint: bool` を受ける。
  - `on_key_step`(`:518`)と `HoldController::on_key`(`:727`)の引数に足す。
- エピソードが無く、`drift_hint` が真なら武装する。
  - 窓切替・無操作の契機より優先度は低い。
- **再試行の回数は制限しない**(所有者回答 Q3)。
  - 判定はエピソードごとに行う。判定が出ずに閉じたら(`Timeout`・`MaxKeys`)、ずれが続いている限り、次の Down でまた武装する。
  - 窓切替(scope の変化)で、エピソードと武装は破棄する。
- `drift_hint` は、`evaluate_drift`(`state/drift_correction.rs`)の結果から作る(m4)。D2 の事実を再実装しない。
  - `Ok(DriftCorrection { desired: false, observed: true, duration_ms })` で、`duration_ms >= DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`(3000ms、バルーンと同じ閾値。新しい tuning 定数は足さない)のときだけ真。
  - 殻は `ImeStateHub` の読み取り専用の口から、この結果を受け取る。

### D2. 取り下げてよいかの判断(純粋関数)

`crates/awase-ime-core/src/state/intent_refute_plan.rs`(仮)を新設する。

```rust
pub struct RefuteFacts {
    pub drift: Option<DriftCorrection>,   // evaluate_drift の Ok(D1 と同じ。m4)
    pub what_if_open: Option<(bool, BaseDecision)>, // 明示意図と IntentStore を外したと仮定した resolve_open_at(M1)
    pub same_scope: bool,                 // エピソードの開始時と判定時の前面窓が同じ
    pub focus_in_sync: bool,              // lang_detect_focus_in_sync()
    pub intent_changed_in_episode: bool,  // エピソードの間に明示意図・IntentStore・信頼できる観測の向きが変わった(M3)
    pub verdict_by_thumb_overlap: bool,   // Verdict::Japanese(JapaneseEvidence::ThumbOverlap)(Q2)
    pub thumb_is_space: bool,             // 親指が VK_SPACE(m3。hold_config_ok は流用しない)
}
pub fn plan_refute(f: RefuteFacts) -> Result<(), RefuteSkip>;
```

取り下げるのは、次のすべてを満たすときだけ。満たさない理由は `RefuteSkip` の列挙で返し、journal に残す。
1. `drift` が明示 OFF・観測 ON で、3000ms 以上続いている。
2. **what-if**: 明示意図と `IntentStore` を外したと仮定して評価した `resolve_open_at` が ON。しかもその根拠が `DeriveHigh(..)`(Conv を含まない High の単独観測)である(M1)。
   - `DeriveMedium`(Conv を含みうる合意)・`IndicatorGlyph`・`MostRecentTrusted`・`DesiredFallback` では取り下げない。
   - 判断に使った観測と、取り下げた後に Engine を決める観測が食い違わないようにするため。
3. 同じ窓で、フォーカス追跡が前面窓に追いついている。
4. エピソードの間に、明示意図・`IntentStore`・信頼できる観測の向きが変わっていない(M3)。
   - 役割なしの左親指が GJI を閉じた、役割付きの親指が `SetOpen` を書いた、のどちらかが起きたエピソードでは取り下げない。
5. 日本語の判定が**親指と文字の重なり**による(所有者回答 Q2)。n-gram だけの判定では取り下げない。
6. 親指が Space でない。英文の Space と文字の重なりを、親指の重なりと見分けられないため。

判断の表は、`drift_plan.rs` と同じく全列挙のテストで固定する。新しいモジュールは `CORE_MODULES` と `.cargo/mutants-awase-*.toml` の `examine_globs` に足す。

### D3. belief への反映: 専用イベント 1 つ(B2 を含む)

`.claude/rules/ime-belief-architecture.md` の「真のユーザー操作ではないが belief を動かす場合は専用イベント」に従う。

- `UserImeSetIntent` は使わない。利用者の意図の偽装になる。
- `ObserverReported` + Low も使わない(m2)。
  - 打鍵の判定はヒューリスティックな推測だが、Low の観測では `resolve_open_at` の明示意図の分岐(`ime_model.rs:536-539`)を越えられない。
  - `IntentStore` の上書き(`intent_store.rs:158-174`)も越えられない。
  - したがって、Low の観測では目的(Engine を戻す)に届かない。

**イベントの定義**: `crates/awase-ime-core/src/state/ime_event.rs`(`enum ImeEvent`、`:399`)に足す。

```rust
/// ADR-279: ずれが続く窓で、打鍵列から日本語と判定した。残った明示意図(`refuted_target`)を外す。
/// `ImeStateHub::refute_explicit_intent_by_typing` 専用。desired_open は観測から導ける開閉があるときだけ揃える。
ExplicitIntentRefutedByTyping { refuted_target: bool },
```

struct 形式にする。unit variant だと dylint の構築検出に掛からない(`ModeKeyPassedThrough` と同じ理由)。

**`state/ime_model.rs::reduce` の腕**:
- `last_intent.target == refuted_target` のときだけ、`last_intent = None` にする。
- `desired_open` は、観測から導ける開閉(`derive_any`)があればそれへ揃える。
  - `ModeKeyPassedThrough { align_desired: true }` の腕(`:1062-`)と共通の部分は、ヘルパーに出す。
  - 揃える根拠が無いときに `desired_open=false` が残る件(m6)は、実装の PR で `desired_open` を読む消費者を洗い出す。対象は GjiDirect の already-matched、`open_chain` の早期 exit など。
- `applied` は、取り下げた向きと食い違うなら `Unknown` へ落とす(`KeyEffectPredicted` の腕 `:1048-1058` と同じ)。

**入口 `ImeStateHub::refute_explicit_intent_by_typing(tick_ms)`**(`platform_state.rs`)は、`apply_key_effect_prediction`(`:307-345`)を前例にする(B2)。
- 上のイベントを dispatch する。
- **同じ窓(`shadow_model.current_focus()`)の `IntentStore` のエントリも消す**。
  - 消すのは `lookup` したエントリの `open` が `refuted_target` と一致するときだけ。
  - `IntentStore` は `effective_open_at`(`:1161-`)で `shadow_model` より優先され、明示 OFF の有効期限は `EXPLICIT_OFF_INTENT_TTL_MS = 30_000`。消さないと、取り下げても 30 秒間 Engine が戻らない。
  - しかも drift correction だけは止まるので、直す前より悪くなる。
- 入口はこの 1 つだけにする。

**ガード**:
- `tests/architecture_guard.rs` に、この variant の構築箇所を固定するガードを足す。
- dylint `lints/ime_event_guard/src/lib.rs` の指定関数の一覧(`:67`・`:80` の並び)に足す(m1)。
- 規約ファイルの「4 つのイベント」の節に、5 つ目として追記する。

**取り下げた後に起きること**:
- Engine を戻すための別の書き込みはしない。Engine の活性は `build_ctx`(`runtime/mod.rs:497-505`)が `effective_open_at` から作る。
- `last_intent` と `IntentStore` が無くなれば、D2 の 2 で確かめた `DeriveHigh` の値(ON)に従う。
- drift correction は `NotExplicitIntent` で止まる(`drift_correction.rs:139-141`)。**IME へは何も送らない。**
- 実装の段階では、Engine の活性に他のラッチ(engine deactivate のフラグ等)が絡まないことも確かめる(§9 未確認)。

### D4. 殻の配線と設定(M5・M6)

**配線**:
- `lang_detect_shadow.rs::lang_detect_apply`(`:405`)で、次の条件のときに D2 の事実を集めて `plan_refute` を呼び、`Ok` なら `refute_explicit_intent_by_typing` を呼ぶ。
  - `outcome.trigger == Trigger::Drift`
  - 日本語の判定
- journal の `LangDetectRecord.action` には、`"explicit-off-refuted"` か `RefuteSkip` のラベルを残す。
- `Trigger::Drift` のエピソードは**保持しない**。今の `"observe-only"`(`:437-442`)と同じ流れで、判定だけを使う。
- 殻から純粋側へ渡すところまで(`Trigger::Drift` の結果を `RefuteFacts` に組むまで)は、純粋関数に出して Linux でテストする(M6)。

**設定(所有者回答 Q1 と、レビューの指摘 M5)**:
- 所有者の回答は「この導線だけ `observe` でも動かす」。
- レビューは、これが既定の利用者全員の belief と Engine の活性を黙って変える点を問題にした。
  - 既定の `observe` の説明は「挙動を変えない」(`config.rs:462-464`、設定画面 ADR-271 も同じ)。
  - 止める手段が `mode=off` だけで、それを選ぶと journal の観測も止まる。
  - n-gram の表を読めないと `lang_detect` 全体が `Off` になる(`lang_detect_shadow.rs:92-96`)。重なりだけを証拠にする導線が、関係の無い n-gram の読み込みの失敗で黙って無効になる。
- 2 案を §9 Q1 に並べ、所有者の再確認を待つ。
- どちらの案でも、次の 3 つは守る。
  - `exclude_apps` を尊重する。
  - 設定の文言・設定画面・CHANGELOG に、挙動が変わることを書く。
  - 入力中継ツール(`InputRelay`)は除く。

## 4. 英数と判定したとき・判定不能のとき

- **英数と判定したとき**: 何もしないのを既定にする。明示 OFF の意図を残し、drift correction は今と同じに動く。
  - 英数の判定は、「利用者は OFF のつもりで英字を打っている」と整合する。
  - `ime_on_eisu` の「半角英数トグルに入る」動作は、`Trigger::Drift` のエピソードでは呼ばない。belief が OFF の窓ではトグルの前提が成り立たない(`lang_detect_shadow.rs:337-339` の `entry_ime_ok`)。
- **判定不能のとき**: そのエピソードでは何もしない。ずれが続いていれば、次のエピソードでまた判定する(D1)。
- どちらでも、journal に `verdict`・`trigger="drift"`・`action="none"` を残す。

## 5. 誤判定の害と歯止め

| 誤判定 | 起きること | 歯止め |
|---|---|---|
| 利用者は IME OFF で英字を打ちたいのに、日本語と判定した | 明示 OFF の意図が消え、Engine が戻る | 親指の重なりの証拠だけを採る(英文では無変換/変換を文字と重ねない)。Space 親指の構成では採らない |
| 信頼できる観測が誤り(実 IME は OFF なのに ON と読めた。BUG-68 型) | Engine が戻り、ローマ字が素の英字として出る | 取り下げた後に Engine を決める値が、`DeriveHigh`(Conv を含まない High の単独観測)であることを要する(D2 の 2)。ずれが 3000ms 以上続いていることも要する |
| エピソードの途中で、親指が IME の状態を変えた(役割の `SetOpen`、GJI の 無変換=直接入力) | 取り下げと作り直しが打鍵ごとに繰り返される。awase 自身の OFF で実 IME が閉じ、利用者の望みと逆の向きに収まる | エピソードの間に、意図・`IntentStore`・観測の向きが変わったら取り下げない(D2 の 4)。役割付きの親指は証拠に数えない(D0) |
| 判定中に、利用者が別の明示操作をした | 新しい意図を消してしまう | reduce の腕と入口で、`refuted_target` を今の `last_intent`・`IntentStore` と照合する。違えば何もしない |
| 窓を切り替えた直後に、前の窓の判定を使ってしまう | 前の窓の判定で、次の窓の意図を消す | 窓切替で `last_intent` は既に消える。加えて `same_scope`・`focus_in_sync` を要する。エピソードは scope が変わると閉じる(`close_if_stale`、`:561`) |
| 同じずれで何度も動く | journal が増える | 判定不能の再試行は制限しない(Q3)。ただしエピソードは最大 7 打鍵・2.5 秒で、journal は 1 エピソード 1 行。取り下げのあとに物理 IME キーや役割付きの親指が意図を作り直せば、また動く(m5。「1 回で済む」とは限らない) |

## 6. 検証計画

1. **段階 0(§2)**: Windows CI。`sc-driftrecovery-ctrlmuhenkan-gji-tsf` の変種と、WT の窓で測る。
2. **Linux**: PR の CI で走る。
   - `src/lang_detect.rs` の単体テスト。
     - `Trigger::Drift` の武装を固定する(判定不能のあとも再武装する・scope が変わると破棄する・窓切替と無操作が優先する)。
     - D0 の純粋関数を固定する(役割の無い `VK_NONCONVERT` の親指と文字の重なりが `ThumbOverlap` になる。役割付きの親指は `excluded` のまま)。
   - `intent_refute_plan.rs` の全列挙テスト。
   - `ImeStateHub::effective_open_at` で Engine の活性まで見るテスト(B2)。
     - 取り下げで `IntentStore` のエントリも消え、ON になる。
     - エントリの向きが違えば消えない。
     - `ImeModel` 単体では、`IntentStore` の上書きを検証できない。
     - 判定本体は `IntentStore::resolve_effective_open` が ungated なので Linux で走る(`intent_store.rs:140-157`)。
   - `crates/awase-windows/tests/closed_loop_scenarios.rs` に BUG-110 型のシナリオを足す。擬似 IME(`tests/support/pseudo_ime.rs`)を使う。
     - 物理 IME キーの OFF を誤検出する → 擬似 IME は ON のまま → 信頼できる観測 ON が続く → 3 秒後に親指と文字を重ねて打つ → 意図が消え、Engine が活性に戻り、drift の送信が止まる。
     - 対照として、英字だけを打った場合と、エピソード中に左親指が IME を閉じた場合は、何も変わらないことを確かめる。
   - `pseudo_ime_exhaustive.rs`(ADR-277)で、新しい不変条件違反が出ないことを確かめる。
   - ガード: architecture_guard と dylint。
   - **Linux の閉ループは、`runtime/` の `#[cfg(windows)]` の配線(`lang_detect_on_key`・`observed_key` の殻・D4)を通らない**(M6)。B1 のような配線の誤りは Linux では見つからない。
3. **Windows CI**: 配線の検証。
   - S0-1 の構成に、`gap_made` の後に親指と文字を重ねる打鍵列を足す。
   - awase.log・journal に `trigger="drift"`・`action="explicit-off-refuted"` が出て、その後 `[drift] correction` が止まり、Engine が活性になることを判定する。
   - CI で InputSite を再現できない場合は、TSF のフォーム(`--form=tsf`)で代える。InputSite 固有の挙動は未確認として残す。
4. **実機(dragonflyg4)**:
   - Windows Terminal + PowerShell + GJI、JIS 配列 `nicola_keytop.yab`。
   - 発端(0xF3 を明示 OFF と誤検出させる)は BUG-52 の不安定さに頼るので、確実には再現できない。
   - そのため、実機では Ctrl+無変換の変種でずれを作る。0xF3 の自然な発生は、不具合報告の journal を待つ。

## 7. バルーンの撤去(所有者回答 Q4 と、レビューの指摘 M4)

- **撤去するのはバルーンだけ**。
  - 対象は `ir_notify_drift_giveup_diagnostic` の中の `show_tray_balloon` の呼び出しと、その文言(約 5〜15 行)。
- **journal の `DriftGiveUpDiagnostic`・`DriftGiveUpIntervalEnded` は残す**。
  - BUG-110 追補3 で、不具合報告の分析に実際に使った。
  - 関数名を残せば、`architecture_guard.rs:3440-3507` の件数ガードと `drift_plan_table.txt` の golden は変わらない。
- **バルーンは向きを問わずに出る**(`should_notify_diagnostic`、`drift_plan.rs:359-361`)。撤去すると、次の場合の通知も消える。
  - 本 ADR が直さない逆向き(明示 ON・実 OFF。Q5 で扱わないと決めた向き。BUG-200 型に近い)。
  - 英字だけを打つ利用者。
  - 親指を重ねない利用者。
  - 役割付き・Alt・Space 親指の構成。
- このため、撤去の条件を次のどれか 1 つにする。
  - **段階 0 が (a) のとき**: CI で `DriftGiveUpDiagnostic` が 0 件。今のコードでは実質出ていない、という根拠になる。
  - **(b)・(c) のとき**: 導線(または既存の経路)を入れた後の journal で数える。`DriftGiveUpDiagnostic` の区間が「明示 OFF・実 ON の向きで、取り下げ(または変換の 1 打)で解けた区間」だけになっていること。
  - **それを待たない場合**: 逆向きのずれの通知も消えることを、所有者が承知した上で撤去する(§9 Q4 の再確認)。
- 撤去したら、ADR-132 Phase 1 の status に「バルーンは ADR-279 で撤去」と追記する。

## 8. 却下した案

### 8.1 belief に合わせて IME を書き直す(drift correction を強める・延長する)

- BUG-110 追補1 で、`FeedbackPolicy::Read` の drift correction は、341 秒・attempts 162→243 の `Send` を返し続けて収束しなかった(`BUG-110.md:74-81`)。
- しかも、書き直そうとした向きの元は誤検出した明示 OFF で、利用者の望み(日本語入力)と逆だった。
- ADR-212 P6 は「awase が自分の推測を IME に書かない」方向に絞っている。所有者の方針(belief に書き直さない)とも合わない。

### 8.2 字形(タスクバーの入力インジケーター、ADR-261・ADR-273)だけで判定する

- 字形は `TaskbarGlyph` として、drift の根拠から明示的に除いている(`drift_correction.rs:166-170`)。
- 足りないのは「IME の状態」ではなく、「利用者が日本語を打ちたいか」。それは字形では分からない。

### 8.3 バルーンを今すぐ撤去する(段階 0 も導線も無し)

- 段階 0 の測定なしでは、所有者の条件(導線が効くなら)を満たすかどうか分からない。
- 段階 0 が (a) なら、測定の結果を根拠に撤去してよい(§2)。

### 8.4 `ModeKeyPassedThrough` を流用する

- reduce の効果はほぼ同じ。しかしこの variant は `pass_through_observed` 専用で、dispatch 元をガードで 1 か所に固定している(`ime_model.rs:863`)。
- 意味(モードキーを通過させた)も違い、journal で区別できなくなる。
- 腕の中身はヘルパーで共有し、variant は分ける。

### 8.5 `ObserverReported` + Low で推測を入れる(m2)

明示意図の分岐と `IntentStore` の上書きを越えられず、Engine は戻らない(D3)。

### 8.6 ADR-132 の B'(明示意図の有界失効)・B''・候補 1(矛盾検出中ラッチ)との関係(M1)

本案は B' と同じ問題空間にある(`last_intent` と `IntentStore` を外す)。B' の却下理由(`132-...md:175-211`)の項目ごとに、本案に当たるかどうかを書く。

| ADR-132 B' の却下理由 | 本案での扱い |
|---|---|
| `last_intent` を消すと `derive_any` に落ち、`ConvOpenInference` 1 件(Medium)で開に倒れる(BUG-26 依拠) | D2 の 2 で、意図を外した仮定の `resolve_open_at` の根拠が `DeriveHigh`(Conv を含まない High)であることを要する。Conv だけ・Medium の合意だけでは取り下げない。**残るリスク**: `DeriveHigh` の観測そのものが誤る場合(ImmCross の読み違い等)は防げない |
| `IntentStore` は「壊れた conv 1 件では反転させない」装置で、それを名指しで無効化する | 消すのは D2 をすべて満たしたときだけ。根拠は conv ではなく、High の観測と利用者の打鍵(親指の重なり)の 2 つの独立した証拠。前例は `apply_key_effect_prediction` が打鍵の予測で同じエントリを消していること(ADR-191 決定3) |
| 明示意図が `None` になると drift correction が止まり、リテラル出力のまま固着する | 本案は「観測が High で ON」のときだけ外す。外した後は Engine が ON の観測に従って活性になり、リテラル(Engine 非活性での素通り)とは逆の状態になる。**残るリスク**: High の観測が誤りで実 IME が OFF なら、ローマ字のリテラルになる(§5 の 2 行目) |
| 失効後の英数キーで、書き込み→失効のサイクルが周期的に繰り返される | 本案の取り下げは時間・回数ではなく打鍵の判定を契機にする。物理 IME キーで意図が作り直されれば、利用者の新しい操作として尊重する(D2 の 4・§5) |
| `GiveUp` は `Blind` 専用で、`Read` では発火しない | 本案は `GiveUp` を使わず、`evaluate_drift` の継続時間を使う(`Read`/`Blind` 共通) |
| (前提)BUG-68 に照らすと「観測 ON の方が正しい」という前提が偽の可能性 | `DeriveHigh` を要することで、BUG-68 の源(IMM32 の NATIVE ビット・conv の推論)を根拠から外す。それでも前提が偽になりうるので、段階 0 の S0-1 で、ずれの区間の観測と実 IME(CI のハーネスが別に読む開閉)を突き合わせる |

- **B''・候補 1-v2(却下理由: 誤って開いたことを検出できない、開閉の非対称の向きが安全側と逆)との違い**:
  - 候補 1 は、矛盾を検出している間ずっと Engine の活性を belief から切り離すラッチだった。
  - 本案はラッチを持たない。belief の意図を 1 回外すだけで、その後は普通の観測の連鎖に戻る。誤って開いた場合に閉じる経路(観測 OFF での追随、物理 IME キー、窓切替)は、今のまま残る。
  - ただし「誤って開いたことを検出できない」のは本案も同じで、それは `DeriveHigh` の正しさに依存する。これは残るリスクとして認める。

## 9. 未確認と所有者の回答・再確認

### 所有者の回答(2026-10-22)

- **Q1**(既定の `observe` で働かせるか): この導線だけ `observe` でも動かす。**→ 再確認が要る(下記)**。
- **Q2**(日本語の証拠): 親指キーと文字キーの重なりだけ。n-gram だけの日本語判定では取り下げない。
  - D0 を入れないと証拠が出ない(B1)。
  - 役割付き・Alt・Space の親指の構成では、D0 を入れても証拠は取れない。
- **Q3**(判定不能のときの再試行): 制限なし。エピソードごとに判定し、窓切替で破棄する(D1)。
- **Q4**(撤去の範囲): バルーンだけ撤去し、drift の journal の診断は残す(§7)。
  - 撤去の条件(逆向きの通知も消える点)の扱いは §7 の 3 つから選ぶ。段階 0 の結果を見て決める。
- **Q5**(逆向き、明示 ON・実 IME OFF): 今回は扱わない。事例が出てから判断する。

### Q1 の再確認: 危険と代案

| | 案 1: 所有者回答のまま(`lang_detect.mode = "observe"` でも動かす) | 案 2: 専用スイッチ(drift の側の独立した設定、既定 ON) |
|---|---|---|
| 既定の利用者への影響 | belief と Engine の活性が変わる。それなのに設定の文言は「挙動を変えない」のまま(直す必要がある) | 同じく変わる。ただし設定の名前と説明がこの動作そのものになる |
| 止める手段 | `mode = "off"` だけ。journal の観測も一緒に止まる | 専用スイッチを OFF にする。observe の journal は残る |
| n-gram の表を読めないとき | `lang_detect` 全体が `Off` になり、この導線も黙って無効になる(`lang_detect_shadow.rs:92-96`) | 重なりの判定だけで動かせる(n-gram が無くても `ThumbOverlap` は出る)。実装で読み込みの依存を分ける |
| ADR-263 の約束(「observe は挙動を変えない」) | 例外を 1 つ作る | 守られる |
| 実装量 | 少ない | 設定項目 1 つ・設定画面 1 行・文言 |

推奨は案 2。どちらの場合も `exclude_apps` は尊重する(D4)。

### 未確認(段階 0 と実装段階で潰す)

- 今の develop で、BUG-110 型のずれが信頼できる観測付きで 3 秒以上続くか(S0-1)。BUG-110 追補3 の原因の一部(force-on)は撤去済み。
- Engine 非活性中の親指 1 打の効果(S0-2)。特に次の 2 つ。
  - 役割なしの親指で `KeyEffectPredicted` が発火するか。
  - MS-IME の既定の割り当てでどうなるか。
- 既定構成の observe の journal の `evidence` の分布(S0-3)。
- Engine の活性が `effective_open` だけから導かれ、明示 OFF の押下で別のラッチが立っていないか。BUG-110 のログには「vk=0xF3 で engine deactivate」とあるが、それが belief 経由かどうか。
- 取り下げ後に `desired_open=false` が残る場合、それを読む消費者(GjiDirect の already-matched、`open_chain` の早期 exit 等)が何をするか(m6)。
- F13〜F24 の親指が、役割由来の `shadow_action` で `is_ime_key` に掛かるか。
- BUG-110 で `Blind` 専用のはずの TsfNative の窓に `Read` が使われた理由。
- 打鍵から明示意図を外す前例として、`KeyEffectPredicted`(`ime_model.rs:1043-1049`)と `apply_key_effect_prediction`(ADR-191 決定3)がある。本案はこれを前例として扱う。旧版の「前例は無い」は誤りだった。
