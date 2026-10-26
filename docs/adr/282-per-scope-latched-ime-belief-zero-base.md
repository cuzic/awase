---
id: ADR-282
title: |-
  IME の開閉と Engine の活性の belief: 小さな修正(局所修正)と、IME の保持単位ごとに主張をラッチするゼロベース案を同じオラクルで比べて決める(書き込みの根拠は belief から分ける)
summary: |-
  BUG-223(GJI の tsf-native / imm32-unavailable 窓で、IME は ON なのに Engine が OFF のまま数秒〜数十秒続き、「こうしき」が「るしき」になった)と、
  所有者の方針(2026-10-26)「IME が ON でかなモード(ひらがな・カタカナ)なら必ず Engine ON。逆は OFF」を受けた設計の比較。**決定するのは原則だけで、方式は比較の結果と所有者の判断の後に決める**。
  確定: 所有者の判断 4 件、書き込みの根拠(drift correction・授権)を belief から分ける原則(D8)、IME への書き込みの方針は変えない(D9)。
  比較する候補: (S) 今の構造への小さな修正(L1: 予測の開で `desired_open` を書く、L2: 読めない窓の窓切替で ON、L3: `IntentStore` をスレッド鍵にし予測も入れる、L4: 持ち越しを窓の種類で分ける、L5: 窓をまたぐ時刻順で `IntentStore` の古い行を無効にする)と、
  (Z) IME の保持単位(スコープ)ごとに「最後に受け入れた主張」をラッチするゼロベース案。
  測定(擬似 IME のオラクル): IME の状態を全窓で共有するモデル(shared)・全字母で msime の I1 違反 405 本が L1 で 153 本(核の最小列 36 → 0)。窓ごとに別スレッドのモデル(per-thread)・全字母では 185 本で、L1〜L3 は I1 と I2 を同時には減らさない。
  残る 153 本は嘘の観測 32・外部変更 121 で、どの候補でも観測の手段と既定の設計が要る。
  保持単位の CI 測定(各セル 2 試行、Windows Server)と実機(GJI・Windows 11、Edge)では、読める Win32 窓と Chromium の窓は独立。MS-IME × Windows Terminal は「A を開にしておくと WT が開」(一方向)。GJI × tsf-native(BUG-223 の窓)は未確定。
  実機の再現(「r;にちは」)は、窓 A に残った `IntentStore` の明示 OFF(TTL 30 秒の内側)が、A へ戻った時に `resolve_open_at` の結果を上書きした型(A の実 IME が ON だったこと自体は報告による。その理由〈B での操作が効いたか、A の OFF が届いていなかったか〉は未確認)で、L1 では消えない。
  所有者の決定(2026-10-26): (1) O3・O4 は候補 S にも要件として課す(I3・I4・I6 を S にも測る)。(2) O3 と方針が衝突したら「IME ON かつかな」を優先し、O3 の「最後の明示操作」を窓をまたぐ時刻順で定義し直す(窓 A の古い明示 OFF は、A と状態を共有しうる窓で後から ON にされた時点で失効)。
status: |-
  起草(2026-10-26)→ Opus レビュー round 1(B2・M9・m9)→ round 2(B1・M6・m7)→ round 3(B1・M4・m6)を反映(2026-10-26)。所有者の決定 2 件(O3・O4 は S にも課す、U14 は「IME ON かつかな」を優先)を反映。未確定。
  構造を「ゼロベース設計の提案」から「小さな修正とゼロベース案の比較(決定は比較の結果と所有者の判断の後)」へ作り直した(round 2 RM3)。
  依存: PR #857(基準の測定)はマージ済み。PR #868(局所修正 L1〜L3 の測定)は OPEN。保持単位の CI 測定は run 40327981441(ブランチ `ci/adr282-ime-scope`)、文書・PR は別担当が準備中。
  PR #854(BUG-223)は OPEN。CI 検証バッチ(§6.5)の B1a〜B1c(tsf-native の保持単位)が最優先で未実施。L1 は B1a〜B1c の結果と、O3・O4 を S に課す決定(round 3 R3B1 への所有者の回答)の後に決める(round 2 は済んだ)。
  未決 U1〜U13(U14 は所有者の決定で確定)。BUG-223 の区間 2・3・5 は元のログが失われ型が未検証(§1.1)。実装はまだ(コード・テストは不変)。
related_adr:
  - "ADR-082"
  - "ADR-087"
  - "ADR-089"
  - "ADR-098"
  - "ADR-106"
  - "ADR-165"
  - "ADR-179"
  - "ADR-187"
  - "ADR-188"
  - "ADR-191"
  - "ADR-199"
  - "ADR-205"
  - "ADR-208"
  - "ADR-212"
  - "ADR-213"
  - "ADR-214"
  - "ADR-233"
  - "ADR-245"
  - "ADR-255"
  - "ADR-260"
  - "ADR-262"
  - "ADR-263"
  - "ADR-268"
  - "ADR-273"
  - "ADR-277"
  - "ADR-279"
  - "ADR-280"
  - "ADR-281"
---

# ADR-282: IME の開閉と Engine の活性の belief — 小さな修正とゼロベース案を比べて決める

調べた版: `origin/develop` `ebd744b6d`(§4.14 の確認は `82e00b655`。両者の間の差分は `crates/awase-windows/tests/` の 3 ファイルだけで、本番のコードは同じ。`git diff --stat ebd744b6d 82e00b655 -- crates`)。行番号は `ebd744b6d` のもの。
棚卸し: PR #858(`origin/docs/belief-zero-base-inventory` の `docs/tasks/belief-zero-base-inventory-2026-10-26/`、`ff75d5d58`)。基準の測定: PR #857(マージ済み。`37fc37e75` の `docs/tasks/belief-engine-invariant-baseline-2026-10-26.md` と golden)。
局所修正の測定: PR #868(`origin/docs/belief-local-fixes` `5efea3527` の `docs/tasks/belief-engine-invariant-local-fixes-2026-10-26.md`)。cargo はローカルで実行していない。実機のログは本 ADR の起草者は読んでいない(実機の値は team-lead の報告の転記)。
レビューの指摘への対応は §15(round 1)・§16(round 2)。

## 0. 位置づけ: 確定すること・比べること・決め方

### 0.1 確定(本 ADR で決める)

| # | 内容 |
|---|---|
| O1〜O4 | 所有者の判断 4 件(§1.4)。中身は変えない。**O3・O4 は候補 S にも要件として課す**(所有者の決定 2026-10-26、round 3 R3B1 への回答)。適用範囲の問いは U2・U3・U10・U11 |
| O3′ | **O3 の「最後の明示操作」の定義(所有者の決定 2026-10-26、U14 の確定)**: 「IME ON かつかな」と O3 が衝突したら前者を優先する。明示操作の勝ち負けは**窓をまたぐ時刻順**で決める: 窓 A の明示 OFF は、A と IME の状態を共有する(または共有の有無が未確定の)窓で、それより後に ON にされた(明示 ON、または信頼できる観測)時点で失効する。独立と測れた窓の組では、A の記録は A の状態を指すので失効しない(O4 と衝突しない) |
| D8 | **書き込みの根拠を belief から分ける原則**(候補 S にも課す。今の構造でも授権 Step 4c が `desired_open`〈初期値 true・`HwndCacheRestored`・通過の揃えが書く〉を読むので、すでに満たしていない。§4.9)(ADR-087 P11 の適用)。drift correction・授権に入るのは利用者の操作(今の `UserIntentSource` の `PhysicalImeKey`・`SyncKey`・`Command`)だけで、寿命は今のまま(drift は `last_intent` と同じく窓切替・予測・通過で消える、授権 Step 1 は `IntentStore` の 10/30 秒)。belief の側をどう変えても(候補 S・Z のどちらでも)、予測・既定・lang-detect・半角英数トグル・パニックリセットは書き込みの根拠に入れない |
| D9 | IME への書き込みの方針(何をいつ書くか)は変えない。ADR-191・199・212・213 のまま |

### 0.2 比べる候補

| 候補 | 中身 | 節 |
|---|---|---|
| **S: 小さな修正** | L1(予測の開で `desired_open` を書く)、L1′(表のセルの押した後の状態で書く)、L2(読めない窓の窓切替で ON)、L3(`IntentStore` をスレッド鍵にし予測も入れる)、L4(持ち越しを窓の種類で分ける)、L5(窓をまたぐ時刻順で `IntentStore` の古い行を無効にする)とその組み合わせ | §3 |
| **Z: スコープごとのラッチ(ゼロベース案)** | belief を IME の保持単位ごとに持ち、「最後に受け入れた主張」をラッチする。明示操作は ON・OFF・修飾付きを同じ型、受け入れは信頼表と柵、不確実なときは O2 の既定 | §4 |

どちらの候補でも要るもの(候補と独立): 観測の手段と既定の設計(残る 153 本は嘘の観測・外部変更で、どの候補でも減らない。§14.2)、修飾付きのモードキーの写像表(§4.13、BUG-221・ADR-281)、CI 検証バッチ(§6.5)。

### 0.3 決め方(V0 の出口)

1. **前提の測定**: B1a〜B1c(tsf-native の窓の保持単位。BUG-223 の窓)・B2(新しいスレッドの初期状態)。結果で、窓の種類ごとに「持ち越すか」の事実が決まる。
2. **同じオラクルで比べる**: 基準の測定のオラクルを、窓の種類を混ぜた窓モデル(測った事実どおり)に拡張し(§6.2)、候補 S の各案と候補 Z を数える。I1・I2 を並べ、核の最小列・BUG-223 の列・実機の再現の列(§1.1.1、`IntentStore` の上書き型)を合否に使う。
3. **所有者の判断**: S の組み合わせが合格条件(§6.1。I1・I2 に加えて I3・I4・I6、D8)を満たせば、S を採り Z は採らない(U12)。O3・O4 を課すので、TTL で古い値へ戻る・別の窓へ持ち越す S の案は不合格になり、比べる対象は O3・O4・D8 を満たすように作り直した S′(例: L1′+L5a+D8 の調整、TTL を使わない形)と Z に絞られる。そのとき Z の部分のうち、§4.13(写像表)・§4.4 の信頼表の考え方(読めない窓 × GJI 系の IMM を反証に使わない)だけを別に採るかを決める。
   S で満たせない型が残り、Z がそれを消すと示せたときだけ Z へ進む。**Z が今より仕組みを減らせる見込みは、まだ示せていない**(§4.0)。

## 1. 背景

### 1.1 実機の事故(BUG-223)

`origin/fix/focus-desired-fallback-off:docs/known-bugs/BUG-223.md`(PR #854、OPEN)より。dragonflyg4(GJI、develop 43359222)で「こうしき」が「るしき」。journal の `effective-open-flip → false` のうち所有者の IME-OFF キーでない区間が 5 つ。

- (a) 区間 1・4: 単独の `DeriveMedium{ObserverPoll}`、`DeriveHigh(ImmCrossProbe)` が偽の閉(BUG-219 と同型)。
- (b) 区間 2・3・5: 観測が無く `desired_open`(前窓の false)へ後退(`DesiredFallback`)。区間 3 は直前の操作が F2。
- 未確認(BUG-223 自身): (b) の各区間の直前の明示 OFF の有無。窓切替の後も IME が ON だった理由(全窓で共有か、移動先の状態か)。
- **注意(round 2 RB1 の教訓)**: `[effective-open-flip]` の行の `decided_by` は `resolve_open_at` の根拠で、`IntentStore` の上書きは別の項目 `intent_store_override` に出る(書式は `crates/awase-windows/src/state/platform_state.rs:1254`、`intent_store_override = decision.value != shadow`)。
  BUG-223 の区間の分類が `intent_store_override` を見て行われたかは未確認(BUG-223.md の記述からは分からない)。
- **区間 2・3・5 の型は未検証(team-lead の報告、2026-10-26)**: 元の `awase.log` はサイズで回転して失われた(残っている `awase.log.old`・`awase.log.before-*` は 10/26 08:31Z 以降と 10/25 08:52Z 以降)。当時抽出した行は 200 文字で切れていて、`shadow_model=`・`intent_store_override=` の手前で切れていた。
  よって区間 2・3・5 が `desired_open` の後退(`DesiredFallback` が false)か `IntentStore` の上書きかは分からない。区間 1(単独の `DeriveMedium{ObserverPoll}`)は検証済み。
  **ハーネスの「BUG-223 の 4 本」は、今のコードに `desired-fallback` の型があることの回帰の列としては有効だが、実機の BUG-223 の事故を写しているとは言えない**(合格条件の根拠を弱めて使う。§6.1)。

### 1.1.1 実機の再現(2026-10-26、team-lead の報告の転記。起草者は `awase.log` を読んでいない)

所有者の dragonflyg4(GJI、develop `aa208a1c`。lang-detect の設定は報告では `mode=ImeOnEisu scope=All eisu_threshold=13 ja_threshold=14`。転記の誤りでないかは未確認)。
手順: 窓 A で自動の半角英数 → 窓 B で F2 → 窓 A に戻って親指シフトで「こんにちは」→ 結果「r;にちは」(最初の 2 文字が素の英数)。時刻は UTC。行は team-lead が全文を再取得したもの(検証できたのは 20:29〜20:32 UTC の 8 行)。

| # | 時刻 | 行 | 読み |
|---|---|---|---|
| 1 | 20:31:10.522 | `→ false decided_by=ExplicitIntent shadow_model=false intent_store_override=false` | 明示 OFF(`last_intent`・`desired_open=false`・`IntentStore` に A の hwnd の OFF の行、TTL 30 秒) |
| 2 | 20:31:12.585 | `→ true decided_by=DesiredFallback shadow_model=true intent_store_override=false` | 窓 B への入場。**`DesiredFallback` が true** = この時点で `desired_open` は true(1 と 2 の間で何が true にしたかは未確認。候補: 入場時のキャッシュ復元 `HwndCacheRestored`) |
| 3 | 20:31:13.976 | `[key-effect-predict] vk=0xF2 open=None mode=None track=KeyTrack{conv:Some(C19), stage:None}` | B で F2。belief が既に開なので開閉の予測は付かない(予測器は変化があるときだけ出す、`key_effect_predictor.rs:641`) |
| 4 | 20:31:16.629 | `→ false decided_by=DecidedBy { base: DesiredFallback, .. } shadow_model=true intent_store_override=true` | A へ戻る。`resolve_open_at` は true(`DesiredFallback` = true)だが、**A の hwnd に残っていた `IntentStore` の OFF の行(手順 1 から約 6.1 秒、TTL 30 秒の内側)が上書きして false**。Engine OFF |
| 5 | 20:31:17.934 | `→ true decided_by=KeyEffectPrediction` | 予測が `IntentStore` の行を消す(`platform_state.rs:361-365`)ので true に戻る。その間の 2 打鍵(lang-detect の `presses=2`)が素の `r;` |

ほかに単独の `DeriveMedium{ObserverPoll, second: None}` による OFF への反転が 2 回(20:29:08.757、0.5 秒後に true。20:32:06.059、0.7 秒後に true。BUG-223 の区間 1 と同じ型)。

**読み(round 2 RB1 で訂正)**:
- この再現の型は「**窓 A に残った明示 OFF の `IntentStore` の行が、A へ戻った時の真の ON を上書きした**」で、`desired_open` の持ち越しではない。**L1(`desired_open` を書く)では消えない**(`IntentStore` は `resolve_open_at` の結果を丸ごと上書きする)。
- A の実 IME が戻った時に ON だった理由は未確認で、2 通りある: (i) A と B が開閉を共有していて、B の F2 が A にも効いた、(ii) 手順 1 の OFF が A の実 IME を閉じていなかった。A・B の窓の種類も未確認。
- **(i) なら**、「最後の明示操作が勝つ」は窓をまたいだ時刻順で解釈すべき(B の F2 〈20:31:13.97〉は A の OFF 〈20:31:10.52〉より新しい)。**(ii) なら**、awase の明示 OFF の記録と実 IME が食い違っていて、記録を信じ続けるほど誤りが長く続く。
- **手順 1 の OFF が利用者の操作だったか**(team-lead の報告では「所有者の IME-OFF 操作」)を、ログの手順 1 の直前の行で確かめる必要がある。自動の半角英数の入り(lang-detect)が開閉軸の OFF の意図を残しているなら、それ自体が別の不具合。
- 案 Z をそのまま(スコープを per-thread、主張に寿命なし)入れると、この OFF は A のスコープに行の寿命(1 時間)まで残り、**今の 30 秒より悪化する**。今の 30 秒は、O3 と方針の衝突を時間で丸めている。
- **所有者の決定(O3′)の下での扱い**: (i) の読みの列(A と B が開を共有、または共有が未確定)では、B で後から ON にされた時点で A の OFF は失効し、A に戻ったとき Engine は ON。ただし B の F2 は今 `open=None`(手順 3)で開の操作として記録されないので、押した後の状態での記録(L1′・§4.13-5)が要る。(ii) の読み(A の OFF が A の実 IME に届いていない)は、観測の無い窓ではどの候補でも検出できない(§2.1)。

### 1.2 基準の測定(PR #857)と局所修正の測定(PR #868)の数字(字母をそろえて読む)

| 窓モデル | 字母 | base の I1 違反(msime) | 出典 |
|---|---|---:|---|
| shared(全窓が同じスレッド) | 全字母 19 種・長さ 3 | 405 | 基準の測定・PR #868 |
| per-thread(窓ごとに別スレッド、初めての窓は閉) | 全字母 19 種・長さ 3 | **185** | PR #868(`5efea3527` の文書の表) |
| per-thread | 核の字母 12 種・長さ 3 | 9 | 基準の測定 |

- round 1 までの本文と summary の「shared 405 本、per-thread 9 本」は字母が違う数字を並べていた(round 2 RM1)。同じ全字母では per-thread も 185 本ある。
- shared の主因は `desired-fallback` 297(ON の予測が窓切替で捨てられ前窓の `desired_open` に戻る)。最小列 `key:hiragana > focus:wezterm`。BUG-223 の 4 本は shared でだけ違反。
- ハーネスの物理キーは全部 `has_shadow_action: false`・`unreadable: false`・`observable_toggle_guard: false` で予測の経路に流す(`harness.rs` の `key_mods`)。`key:ime-on`・`key:hankaku-zenkaku` で始まる列は本番と経路が違う。
- **ハーネスは実機の再現(§1.1.1)の型を今の字母で作れるか未確認**: `IntentStore` の上書きが効く列(明示 OFF → 別の窓で ON → 30 秒以内に戻る)と、窓の種類で共有が違う窓モデルが要る(§6.2)。

### 1.3 今の構造の問題(棚卸し 04-complexity の X1〜X11)

| # | 問題 | 候補 S | 候補 Z |
|---|---|---|---|
| X1 | 明示意図の保管場所が 5 つ(`desired_open`・`last_intent`・`IntentStore`・`persistent_explicit_off`・`key_effect.open`)で寿命がばらばら | 減らない(L3 は `IntentStore` の役割を増やす) | belief 側は 1 つ。書き込みの根拠(D8)に `last_intent`・`IntentStore` が残るので 5 → 3 |
| X2 | `desired_open` の意味が 8 つ混在 | 減らない(L1・L2 は書き手を増やす) | 廃止して意味ごとに分ける |
| X3 | ON と OFF の非対称(A1〜A9) | L1 で経路の非対称の一部が減る | belief 側は対称 |
| X4 | 窓切替の後処理が窓の種類ごとに別 | L2・L4・L5a で規則が増える(L4・L5a は「窓の組の共有の表」を要し、これは Z の `ScopeModel` と同じデータ) | スコープの切り替え + 既定 + 窓の種類ごとの表 |
| X6 | 優先順位が 2 層(`resolve_open_at` の上に `IntentStore`) | 残る(実機の再現の型はこの 2 層目で起きた) | belief 側は 1 層 |

### 1.4 所有者の方針と判断(2026-10-26)、保持単位の測定

方針: IME が ON でかなモード(ひらがな・カタカナ)なら、必ず Engine ON。例外は `user_enabled` が偽のとき(ほかは U1・U3)。逆に IME OFF・半角英数・直接入力なら Engine OFF。

| # | 所有者の判断 |
|---|---|
| O1 | 「かな」の範囲 = ひらがな・カタカナ。半角英数・直接入力は含まない |
| O2 | 不確実なとき(観測が読めない窓で IME の状態が分からない)は窓の種類で分ける。読めない窓は ON 寄り、読める窓は観測を信じる |
| O3 | 最後の明示操作が勝つ。ON も OFF も対称。窓切替や観測の寿命切れで古い値に戻らない。観測が反証する場合の扱いは設計する |
| O4 | 窓切替では IME の状態を窓ごとに独立させ、持ち越さない。新しい窓は、その窓の直前の記録、無ければ不確実の方針 |
| (追加) | L1 は tsf-native の CI 測定(B1a〜B1c)と O3・O4 を S に課す決定の後に決める(2026-10-26、team-lead 経由) |
| (決定) | O3・O4 は候補 S にも要件として課す(2026-10-26) |
| (決定) | O3 と方針が衝突したら「IME ON かつかな」を優先。O3 の「最後の明示操作」は窓をまたぐ時刻順(O3′、§0.1)(2026-10-26) |

**保持単位の測定**(小標本。「スレッドごと」と言い切らない):

| 出典 | 環境 | IME | A > B | 開閉 | 変換モード | 試行 |
|---|---|---|---|---|---|---|
| `191-gji-state-scope-spec.md` §2.1(run 35623301916。artifact は失効、プローブのブランチも無い) | Windows Server、SPI=0、自前の Win32 EDIT、MS-IME は IMM32 HKL として同定 | GJI・MS-IME | 同じスレッドの別欄/別窓は共有、別スレッド・別プロセスは独立 | 同左 | 同左 | 2 回 × 反復 2 |
| run 40327981441(`ci/adr282-ime-scope`、ジョブログの `SCOPE:` 行を `gh api .../jobs/{121589556404,121589557165}/logs` で確認) | Windows Server | MS-IME 本体 | edit>edit・edit>chrome・chrome>edit・wt>edit | independent | independent | 各 2/2 |
| 同 | 同 | MS-IME 本体 | **edit>wt**(B = Windows Terminal、tsf-native) | **shared-open**(A を開にしておくと WT で `ka` が `か`) | independent | 2/2 |
| 同 | 同 | GJI | edit>edit・edit>chrome・chrome>edit | independent | independent | 各 2/2 |
| 同 | 同 | GJI | wt>edit | independent | independent | **1/1**(2 試行中 1 回有効) |
| 同 | 同 | GJI | edit>wt | **inconclusive**(B で ON キーが効かない、ctl_on=mixed 1/2) | inconclusive | — |
| 実機(team-lead の報告。ブランチ `exp/ime-scope-realmachine` `37ab1cf86` で Chrome の代わりに Edge。起草者は生ログ未読) | Windows 11 23H2(ビルド 22631)、SPI=0、awase 停止 | GJI | edit>edit・edit>Edge・Edge>edit | independent | independent | 各 2/2 |

- 測っていないもの: GJI × tsf-native(実機・CI とも決まらず。**BUG-223 と §1.1.1 の窓の種類**)、WezTerm、UWP/XAML、同じプロセスの別スレッド、閉の向き(A を閉にして WT へ)・往復・WT の起動順(A を設定した後に WT を起動したなら「引き継ぎ」ではなく新しい窓の初期化かもしれない)、マウス・タスクバーの切替。
- MS-IME × WT の振る舞いの仕組みは未検証(TSF の keyboard open-close コンパートメントは候補の 1 つ)。**測ったのは「開の一方向」だけ**。

## 2. 不変条件(比較の物差し)

検査の語彙は `harness.rs` と基準の測定のもの。「真」は擬似 IME の、いまフォーカスのあるスレッドの状態。**窓モデルは測った事実で窓の種類を混ぜたもの(§6.2)を主にし、shared・per-thread の純粋なモデルは感度の確認として並べる**。

| # | 不変条件 | 検査の形 | 合格条件 |
|---|---|---|---|
| I1 | 真が 開 ∧ かな ∧ `user_enabled` ∧ 日本語 IME なら Engine は Active | オラクル(全列挙・ランダム探索) | 「awase の操作・窓切替・正しい観測だけ」かつ予測の外れを含まない列で 0。BUG-223 の 4 本・§1.1.1 の型の列で 0 |
| I2 | 真が 閉、または 開 ∧ 半角英数なら Inactive | 同 | 既定の区分(O2 で ON に倒した列)以外で 0。既定の区分を含めた総数を窓の種類別に旧と並べ、増える分を所有者が受け入れるか判断(U10) |
| I3 | 最後の明示操作が勝つ: スコープ S で明示操作 v の後、S に新しい明示操作・受け入れた観測が無い限り、S にフォーカスがある間の belief は v | `op(v) @S` の後に他スコープへの移動・時間・受け入れない観測を挟んで S へ戻る。v は ON・OFF | **S・Z とも 0**(所有者の決定)。**「スコープ」は測った保持単位**。O3′ により、共有(または未確定)の窓の組では、他の窓での後の ON が S の OFF を失効させる(その場合の belief は ON で、I3 の違反に数えない)。開を引き継ぐ窓での定義の細部は U11 と B1a の後(round 2 m4) |
| I4 | スコープの独立 | 変成関係(S の事象と全体の事象だけを残した列と一致) | S・Z とも、独立と測れた組(PerThread)で 0 |
| I5 | 古い観測で戻らない(settle 170ms・字形 1300ms・旧 epoch/hwnd) | 読み取り開始時刻を op より前にした観測 | 0 |
| I6 | 寿命で後退しない | `advance_ms(N)`、Z は N < `HWND_CACHE_MAX_AGE_MS`(3,600,000) | S・Z とも、N = 3,500・11,000・31,000・60,000 で違反 0(Z は 3,599,000 も)。**今の構造の `IntentStore` の TTL(10/30 秒)・鮮度 3 秒で古い値へ戻る S の案は不合格**(L3 の長い静止での悪化〈核の最小列 36 → 80〉はこれに当たる) |
| I7 | 既定・予測・awase 起点の操作は書き込みを授権しない(D8) | 授権の入力の検査、`WriteCommand` の件数 | S・Z とも 0(D8 を S にも課す)。Step 4c に届く本番の呼び出し元の列挙(§4.9)を前提にする(round 3 m5) |
| I8 | `user_enabled` が偽なら Inactive | 事象を足す | 0 |
| I9 | Engine が非活性になった瞬間に保留キーを flush | Engine の単体テスト | 維持 |
| I10 | 修飾付きのモードキー(0xF0〜0xF4 × Ctrl・Shift・Alt)で効果が 開 ∧ かな なら KeyDown の直後に Active(BUG-221)。効果の無い修飾付きで Active にしない | 格子の「届いた VK × 修飾」のセル。擬似 IME は修飾を知らない(`key_ctrl` の doc、`grid-tables/*.json` に修飾付きのセルが無い)ので拡張が要る | 0 |

### 2.1 0 にできない違反

- 外部変更 × 読めない窓、嘘の観測 × 読める窓: どの候補でも残る(§14.2 の 153 本)。
- 既定の区分: O2 の帰結。**per-thread の窓(新しいスレッドは閉で始まる)では、候補 Z の D5 も L2 と同じ向きで I2 を増やす**(L2 の実測: msime per-thread の I2 57 → 165)。避けられるのは U2(実測の事前で閉)を採った場合だけ(round 2 RM1)。
- 予測の外れ: 候補 Z・L1 では窓切替をまたいで残る(今は窓切替で消えるので偶然直る)。
- 保持単位の誤り: 真が開を共有するのに独立として持つ(またはその逆)と、I1 と I3 は両立しない(§4.1)。

## 3. 候補 S: 小さな修正

定義はすべて `ImeModel::reduce`(`crates/awase-ime-core/src/state/ime_model.rs`)と `ImeStateHub`(`crates/awase-windows/src/state/platform_state.rs`)の範囲。確かめた前提: ポーリングの停止は `explicit_intent()` = `last_intent` だけを見る(`platform_state.rs:319-321`、`runtime/mod.rs:1196,1217`)。
`KeyEffectPredicted` は `desired_open` を書かず、開閉の予測があれば現フォーカスの `IntentStore` の行を消す(`platform_state.rs:361-365`)。`resolve_effective`(`:2186`)は `IntentStore` の行があれば `resolve_open_at` の結果を丸ごと上書きする。

| 案 | 変えること | 測定(PR #868、msime) | 狙う型 | 副作用・壊すテスト |
|---|---|---|---|---|
| L1 | `KeyEffectPredicted{open: Some(v)}` で `desired_open = v`(`last_intent` は `None` のまま) | shared: I1 405 → 153、核の最小列 36 → 0、I2 35 → 31。per-thread: I1 185 → 185、核の最小列 20 → 32、I2 57 → **109** | `desired-fallback` | per-thread の窓で持ち越した ON が I2 になる。`ime_model` の 2 本・`architecture_guard` の 1 本(予測は `desired_open` を書かない) |
| L1′ | L1 を、変化があるときだけでなく、表のセルが決まっていれば押した後の状態(`after_open`)で書く | 未測定 | belief が既に開のときの F2(§1.1.1 の手順 3) | 予測器の出力の変更(`key_effect_predictor.rs:641`) |
| L2 | `reduce_focus_changed` の最後で profile が TsfNative/Imm32Unavailable(InputRelay 除く)なら `desired_open = true` | shared: I1 405 → 256、I2 35 → **99**。per-thread: I1 185 → 216、I2 57 → **165** | O2 の最小形 | 読めない窓へ移って真が閉のとき I2。`warmup_gate_focus_scope` の 2 本・`check_drift_correction_ignores_heuristic_default_alone_without_explicit_intent` |
| L3 | `IntentStore` の鍵を窓のスレッドに、予測値も記録(TTL 10/30 秒) | shared: I1 405 → 165。長い静止の字母で核の最小列 36 → **80** | 戻った窓での予測 | TTL 切れで悪化。授権 Step 1 に予測が入る(ADR-087 に反する)。`architecture_guard` の `IntentStore` の記録箇所・`random_key_walks_keep_predictions_consistent_with_truth`・`prediction_exhaustive_golden` |
| L4 | L1(または L1′)+ 持ち越しを窓の種類で分ける: 開を引き継ぐと測れた窓への入場だけ `desired_open` を持ち越し、独立と測れた窓(読める Win32・Chromium)への入場では持ち越さず窓の記録/O2 の既定を使う。**GJI × tsf-native の扱いは B1a の後に決める**(round 2 m6) | 未測定(§6.2 の混合の窓モデルが要る) | 窓の種類で違う保持単位 | 窓の種類の表が要る(今の構造に「窓の種類ごとの持ち越し規則」が 1 つ増える) |
| L5a | `IntentStore` を引くとき、**他の窓でそれより新しい明示操作(予測を含む)があれば、古い行で上書きしない**(窓をまたぐ時刻順。開を共有する窓の組に限る) | 未測定 | §1.1.1 の実機の再現(A の古い OFF の行が B の F2 の後も効く) | 開を共有しない組(読める Win32・Chromium)では誤り(B の操作は A に効かない)。組の判定は L4 と同じ表が要る |
| L5b | 予測が `IntentStore` の行を消す範囲を、現フォーカスの hwnd から「同じスレッドの全 hwnd」へ広げる | 未測定 | 同じスレッドの別 hwnd に残る古い行 | §1.1.1 の A・B が別スレッドなら効かない |

- **D8 との関係(round 3 R3M1)**: L1(予測)・L2(既定)は `desired_open` を書くので、授権 Step 4c(Blind の窓で `desired_open` を読む、`open_warrant.rs:224`)を通じて予測・既定が書き込みの根拠になり、D8 に反する。S の案は「**+ D8 の調整**(Step 4c の入力を `desired_open` から利用者の操作の値へ替える)」の形で定義し直して測る。L1 は加えて、ADR-191 決定 3 の実装の固定(予測は `desired_open` を書かない。ドリフト補正が IME へ書き戻さないため)を破る。
- **O3・O4 との関係(所有者の決定)**: L1・L2 は別の窓へ値を持ち越すので O4(独立と測れた組)に、L3 は TTL で古い値へ戻るので O3 に反し、そのままでは不合格。残るのは、持ち越しを「共有(または未確定)の組」に限り(L4)、TTL を使わずに時刻順で古い OFF を失効させる(L5a)形。
- **L5a の定義(round 3 R3M2)**: 「他の窓」= A と IME の状態を共有すると測れた窓、または共有の有無が未確定の窓(O3′)。「明示操作」= 利用者の明示 ON/OFF・表のセルで決まるモードキー(予測を含む。押した後の状態で記録、L1′)・信頼できる観測。A の行より新しい操作があれば A の行は失効する。
  **向き**: 共有と測れた組では対称(B の後の閉〈F0 の予測など〉も A の ON を失効させる)。共有が未確定の組では、所有者の決定(「IME ON かつかな」を優先)に沿って **ON 向きだけ**失効させる(B の閉で A の ON は失効させない)。
- **§1.1.1 を消すには L1′+L5a が要る**: B の F2 は `open=None`(開の記録が無い)ので、L5a 単独では A の OFF を失効させる操作が B に無い。
- L1+L2・L2+L3 は単独より悪い(shared: 184・194)。
- L5 の「`IntentStore` の OFF の TTL を短くする」は採らない(時間だけの規則で、実測の根拠が無い。取り下げ #5・#8)。
- **候補 S の要点**: ハーネスの `desired-fallback` の型は L1 で消える(実機の BUG-223 の区間がこの型かは未検証、§1.1)。実機の再現の型(`IntentStore` の上書き)は L1 では消えず、**L1′+L5a** が要る。O3・O4・D8 を課すと、採れる S′ は「L1′+L4+L5a+D8 の調整(TTL に頼らない)」の形に絞られ、これは「窓の組の共有の表」と「窓をまたぐ時刻順の記録」を今の構造に足すことになる(X4・X6 の 2 層は残る)。

## 4. 候補 Z: スコープごとのラッチ(ゼロベース案)

### 4.0 Z が簡素化になるか(round 2 RM3 の問い)

Z の状態: スコープの表(粒度は窓の種類 × IME × 軸のデータ)、信頼表(窓の種類 × IME × 証拠源、共有メモリの block の有無の軸を含む)、書き込みの根拠(D8: `last_intent`・`IntentStore` をそのまま保持)、観測プール、`Hwnd` → (pid, tid) の正規化、行の寿命、スコープごとの `key_track`、古い `ImeEvent` の再生の写像。
今と比べて減るのは belief 側の保管場所(5 → belief 1 + 書き込み側 2)と `resolve_open_at` の段・`desired_open` の 8 つの意味。増えるのはスコープの表・粒度の表・正規化。移行は `effective_open()` 161 か所・27 ファイルの分類を含む(§9.1)。
**差し引きで仕組みが減ると、今は示せていない**。Z を採るのは、候補 S で消せない型が残り、Z がそれを消すと V0 で示せたときに限る。

### 4.1 スコープと粒度

- **粒度をデータにする**: `ScopeModel` を (IME の種類, 窓の種類, 軸) ごとに持つ。値は `PerThread`(鍵 = (pid, フォーカス先のスレッド))か `Shared`。
- **初期値は O4 のとおり全部 `PerThread`**(round 2 RM6)。測定で開の共有が見つかった組(MS-IME × WT の edit>wt)は、U11 の選択肢として「入場時に開を引き継ぐ(`InheritOnEntry` の候補)」を挙げるが、**所有者の回答と B1a(閉の向き・往復・起動順)の前には初期値に入れない**。候補の規則を書くなら「開の引き継ぎだけ、直前の窓から」に狭める(round 2 RM2)。
- **I1 と I3 の両立**: 粒度が真と一致すれば両立する(I3 の「スコープ」が真の共有の範囲そのものになるので、最後の明示操作 = 真を最後に変えた操作)。一致しないとき両方を満たす規則は無い(新しい窓に入ったとき、独立なら窓自身の直前の状態、共有なら全体の最新の操作が正しい。同じ入力から逆の結論を求める)。別スコープの操作を弱い証拠として伝播する案も、どちらに倒しても片方の真で外れる。よって粒度は測って決める。
- **§1.1.1 の型と Z**: A・B が開を共有するなら(i)、`Shared`(または引き継ぎ)のスコープで B の F2(押した後の状態で記録、§4.13-5)が A の OFF より新しいので勝ち、Engine は ON。独立なら(ii)、A の OFF が A のスコープに残り、Z は今(30 秒)より長く OFF を保つ(悪化)。どちらかは B1a〜B1c で決まる。
- **鍵の関数を 1 つにする**(round 1 M2): `scope_of(foreground_tid) = (pid, GetGUIThreadInfo(foreground_tid).hwndFocus のスレッド)`(前例 `crates/awase-windows/src/focus/thread_scope.rs:365`)を、明示操作の帰属(押下時点)と参照の両方で使う。WebView2・`AttachThreadInput`・別スレッドの子窓・UWP で `hwndFocus` が IME の状態を持つスレッドを返すかは未確認(BUG-91: `hwndFocus` は真の最上位とは限らない)。
  tid が取れないときは `Hwnd(hwndFocus)` に落とし、後で tid が取れたら (pid, tid) の行へ移して 1 行にする(新しい主張を残す)。
- **行の寿命**: 最後のフォーカスから `HWND_CACHE_MAX_AGE_MS`(3,600,000、`tuning.rs:223`)。新しい定数を足さないための流用で、意味は「キャッシュの鮮度」から「主張の最大持続時間」に変わる。`EXPLICIT_OFF_INTENT_TTL_MS` の doc(`tuning.rs:365-397`)は、`IntentStore` に 1 時間を流用したのを ADR-087 round4 M-A で「誤記録の最悪持続時間そのもの」と指摘されて 30 秒へ縮めた経緯を書いている。
  Z は belief 側だけで 1 時間を使い、書き込みの根拠(D8)には使わないので drift の固着は起きないが、**belief 側の誤り(§1.1.1 の (ii) の型)は最大 1 時間続きうる**。

### 4.2 状態

```rust
// 擬似コード。名前は案。
struct BeliefStore {
    scopes: Map<ScopeKey, ScopeBelief>,
    focused: Option<(ScopeKey, FocusFence, WindowClass)>,
    observations: ObservationStore,   // 今の観測プール(授権 Step 3 の入力でもある)
    globals: Globals,                 // user_enabled・is_japanese_ime・ime_kind・panic guard
    write_basis: WriteBasis,          // D8: drift の根拠(今の last_intent 相当)・授権の根拠(今の IntentStore)。寿命は今のまま
}
struct ScopeBelief { open: Option<Claim<bool>>, mode: Option<Claim<ModeClass>>, last_op: Option<ExplicitOp>, key_track: KeyTrack, half_width: HalfWidthLink, last_focused_at: Tick }
struct Claim<T> { value: T, basis: ClaimBasis, at: Tick, fence: ClaimFence }
enum ClaimBasis { Explicit { origin: OpOrigin, certainty: Certainty, press: Option<PressId> }, Observed { source, confidence }, MeasuredPrior(PriorRule) /* U2 で認められた場合だけ */, UncertainDefault(WindowClass) }
enum Certainty { Written, Predicted }
enum OpOrigin { UserKey, UserCommand, LangDetect, HalfWidthToggle, PanicReset }
enum ModeClass { Kana, Eisu, KanaInputMethod, Unknown }   // Kana = ひらがな・カタカナ(ローマ字入力)
```

- `ModeClass` と `InputModeState` の対応: `ObservedRomaji`・`AssumedRomaji{..}` → `Kana`、`ObservedEisu` → `Eisu`、`ObservedKana`(JIS かな入力方式、`src/engine/mode_state.rs:14`)→ `KanaInputMethod`(U5)、`Unknown` → `Unknown`。Engine へは今の `InputModeState` に写す。
- `applied`(ADR-098)・`force_guards`・`observe_miss_monitor`・`focus_generation_watermark` は今どおり(belief の主張ではない。窓切替の扱いも `reduce_focus_changed` のまま)。

### 4.3 更新(`evolve`)

1. スコープの決定(観測は柵が今のフォーカスと一致するとき、明示操作は押下時点の `scope_of`)。2. 受け入れ判定(§4.4。却下は journal へ)。3. 主張の更新(明示操作は `last_op` も。`OpOrigin ∈ {UserKey, UserCommand}` は §4.9 の規則で `write_basis` にも)。4. 観測は受け入れの可否によらず今どおりプールに記録(プールの寿命切れは主張に影響しない)。

### 4.4 受け入れ判定と信頼表(1 ソースの観測で成り立つ形)

前提: 観測プールはソースごとに最新 1 件(`per_source`)。`derive_any` は「新鮮な High 単独 → Medium 以上の無競合多数決(1 ソースでも可)」(テスト `derive_open_medium_single_source`)。授権 Step 3 にも `MediumConsensus{second: None}` → `SingleIndirect`(`open_warrant.rs:205-210`)。

| 新しい事象 | 今の主張の根拠 | 覆す条件 |
|---|---|---|
| 明示操作 | どれでも | 常に覆す(O3) |
| 観測 High・Medium(信頼表で反証可) | 明示操作 | 読み取り開始時刻が `fence`(op + 170ms、字形は + 1300ms)より後で、プール全体で矛盾が無い。Medium 1 つでも覆す |
| 同上 | 観測 | 読み取り開始時刻の新しいほう(同時刻なら信頼度の高いほう。ADR-233 A')。古い High は新しい Medium に勝たない(BUG-189)。BUG-219 型は信頼表で防ぐ |
| 同上 | 既定・実測の事前 | 1 つで覆す |
| 観測 Low(`FocusProbe`・`HeuristicDefault`) | 主張なし・既定 | 入る/覆す |
| 観測 Low | 明示操作・Medium 以上 | 覆さない(C-c6) |
| 信頼表で反証不可 | どれでも | 覆さない(記録だけ) |
| 時間の経過・観測の寿命切れ | どれでも | 覆さない(I6) |

信頼表(データ。初期値は今の挙動から作り、ハーネスと実機 CI で確かめる):

| 窓の種類 × IME | IMM 系(`ObserverPoll`・`ImmCrossProbe`・`FocusProbe`) | `MozcShm` | ADR-205 の監視窓 | ADR-188/244 の直接観測 | `TaskbarGlyph` |
|---|---|---|---|---|---|
| 読める窓 × GJI 系 | 反証可 | 反証可 | — | — | 設定が belief 配線のときだけ(今は `ui_max_level` が Observe で不可) |
| 読める窓 × MS-IME 本体(IMC 経路) | 反証可。**半角英数トグル中の開閉の読み(閉)は反証不可**(`half_width_alnum.rs:738-740` のコメント、CI run 39119247220) | — | — | — | 同上 |
| tsf-native・imm32-unavailable × GJI(Google 版)・Mozc 系で共有メモリの block がある | **反証不可**(BUG-223(a)・BUG-219) | 反証可(ADR-268) | 反証可(開閉の遷移のみ。今は Mozc 系を外している) | モード軸のみ(今は Mozc 系を外している) | 同上 |
| tsf-native・imm32-unavailable × Mozc 系で共有メモリの block が無い | 反証不可 | — | 未確定(U13、§4.14-2) | 未確定(U13) | 同上 |
| tsf-native・imm32-unavailable × MS-IME 本体 | 未確定(U7) | — | 反証可 | モード軸のみ | 同上 |
| InputRelay | 反証不可(`class_names.rs:192-193`「この窓由来の open 観測も belief に取り込まない」) | — | — | — | — |

- round 1 で足した「共有メモリの `Held` の読みで観測の年齢を延ばす」規則は**削除した**(round 2 RM4): `MozcShm` は `BeliefPool`(`evidence.rs` の `declare_evidence!`)で授権 Step 3 の入力にならず、ADR-268 は drift の根拠からも外している。`Held` は「操作の直後で IME の処理前かもしれない」読み(`mozc_shm.rs:380-384`)で、新しい確認として鮮度を作るのは観測の偽装(`.claude/rules/ime-belief-architecture.md` の禁止パターン 2)の精神に反する。§4.14-1 への対応は「主張のラッチで失効の影響を受けない」だけ。

### 4.5 窓切替

前のスコープの行は残し、新しいスコープの行を引く(無ければ §4.6)。観測プールの柵を進める(ADR-106)。`key_track` はスコープの行に持つ(191 spec §3.4)。`applied` 等は今どおり。`write_basis` の drift の根拠は今の `last_intent` と同じく窓切替で捨てる。授権の根拠(`IntentStore`)は今のまま。
消える処理: グローバルな `desired_open` の持ち越し、`plan_tsf_cache_restore`、`apply_hwnd_cache_restore` を belief に書くこと、`reset_stale_ime_on_for_imm_broken` を belief の観測として扱うこと。
BUG-128(粗い鍵)・BUG-218(追跡の遅れ)は、粗い鍵を使わないこと・押下時点のスコープへの記録と時刻の比較で守る(`tsf_cache_restore.rs:80` の規則を一般化)。

### 4.6 不確実なときの既定(O2)

1. 読める窓: 同期の `FocusProbe`(Low)・非同期の `ImmCrossProbe` が届けば主張にする。読めなければ 2。
2. 読めない窓、または 1 で読めなかった窓: 開閉 = ON、モード = かな(`UncertainDefault`)。
- 実測の事前(「awase の起動後に作られたスレッドは閉で始まる」、`thread_scope.rs` の doc・191 spec §3.2)は、使うと新しいスレッドの読めない窓で O2 を部分的に反転させるので、決定には入れない(U2)。**per-thread の窓で I2 を増やさない条件は U2 と U10 にかかっている**(§2.1)。
- モードの主張が無い・古い根拠の `Eisu` だけのときは、開閉 ON なら `Kana`(今の `eisu_reset_on_ime_on` と experiments エントリ 26 の E4〈循環デッドロック対策〉の役割を引き継ぐ。TsfNative での効果は検証不能のまま)。
- 失敗のモード: 端末(Windows Terminal・WezTerm は tsf-native で長時間動くプロセス、英字のコマンド入力が主)で、実 IME が OFF のとき ON に倒すと `git status` などが NICOLA の仮名のローマ字として入る。OFF キー 1 回でそのスコープは OFF を保つが、行の寿命・パニックリセット・再起動で既定に戻る。BUG-200 の機会も増える。既定は授権に入れない(D8)。

### 4.7 Engine の活性の導出(O1)

`Engine::compute_state`(`src/engine/engine.rs:272`)の 4 条件のまま、`ime_on` と `input_mode` をフォーカス中のスコープの主張から作る。読める窓で窓切替の直後にモードが `Unknown` のとき、開閉が ON(主張か既定)なら `AssumedRomaji` として写す(`Unknown` のままだと読みが返るまで `NotRomajiInput`)。
PR #817(マージ済み、「窓切替直後、refresh 前の最初の打鍵を保持して再生する」)の対象の窓は未確認。

### 4.8 今の構造との対応(候補 Z を採った場合)

| 今の仕組み | Z での行き先 |
|---|---|
| `desired_open` | 明示意図の値 → 主張と `last_op`、既定 → §4.6、drift の比較 → `write_basis`、持ち越し → 無し(`Shared` の組なら 1 つの主張が続く)、placeholder → 主張なし、授権 Step 4c → §4.9、`decide_use`(`platform_state.rs:2146-2149`)→ スコープの主張 |
| `last_intent` | belief からは消え、drift の根拠(`write_basis`)として残る(寿命は今のまま) |
| `IntentStore` | belief からは消え(§1.1.1 の上書きの層が無くなる)、授権 Step 1 の根拠として残る |
| `persistent_explicit_off` | 時刻の一般則に吸収 |
| `key_effect`・`input_mode` | 主張(`Predicted`)。窓切替で消えない |
| `ObservationStore` | 残る。`most_recent_trusted`・`DesiredFallback` の段は belief では不要 |
| `HeuristicDefault` の観測 | 授権 Step 4a の入力として書き込みを残す。belief では既定・実測の事前として読む |
| `ModeKeyPassedThrough{align_desired}` | 揃える役割は不要。通過マークは読み直しの予約と drift の根拠を消す役割で残る |
| `ImeEvent` の列挙値 | journal の再生のため残す(ADR-082) |

### 4.9 書き込みの根拠(D8)と授権

**今の経路 → `OpOrigin` → belief/drift/授権の写像**(round 2 RM5。`ebd744b6d` で確認):

| 今の経路 | 今の記録 | `OpOrigin` | belief の主張(Z) | drift の根拠 | 授権 Step 1(`IntentStore`) |
|---|---|---|---|---|---|
| 物理 IME キーで `shadow_action` を持つもの(0x16/0x1A、トグル解決の 0x19・0xF3/F4、F13〜F24 の役割 Toggle) | `UserImeSetIntent{PhysicalImeKey}`(`IntentWitness::from_physical`、`evidence.rs:395-399`) | `UserKey`(`Written`) | 入る | 入る(今と同じ) | 入る(今と同じ) |
| 設定された同期キー(Shift+Space 等) | `UserImeSetIntent{SyncKey}`(`from_sync_key`、`evidence.rs:403-407`) | `UserKey`(`Written`) | 入る | 入る | 入る |
| Engine の `SetOpen` 要求(`keys.ime_*` の組み合わせ・無変換/変換の単独タップの開閉など、`on_engine_set_open_request`、`platform_state.rs:984-995`。`write_set_open_request` `:2012` も同じ `Command`) | `UserImeSetIntent{Command}` + `record_explicit_intent(.., Command, ..)` | `UserCommand`(`Written`) | 入る | 入る | 入る |
| 表引きの物理モードキー(F2・F1・F0・無変換/変換の生キー通過など。修飾付きは §4.13) | `KeyEffectPredicted`(`desired_open` を書かず `IntentStore` の行を消す) | `UserKey`(`Predicted`) | 入る | 入らない(今と同じ。予測は `last_intent` を立てない) | 入らない(今と同じ) |
| lang-detect のひらがなキー(`lang_detect_write_hiragana`、`lang_detect_shadow.rs:382`。授権を通さない一回きり、`applied` を書かない、ADR-262 D3) | 予測の経路で追随 | `LangDetect` | 入る | 入らない | 入らない |
| 半角英数トグルの出入り(ADR-107・245) | `InputModeApplied` | `HalfWidthToggle` | モード軸に入る | 入らない | 入らない |
| パニックリセット | `PanicReset` + `ForceGuard` | `PanicReset` | 入る | 入らない | 入らない(Step 0 の安全弁は今どおり) |

- `drift_correction.rs:230` の `UserIntentSource::Command` はテスト(`fn with_intent`)の構築で、本番の経路ではない。
- 旧版の `OpOrigin::Engine` は `UserCommand` に統合した(今の `Command` が Engine の `SetOpen` を含むため。分けると `keys.ime_*` が書き込みの根拠から漏れる)。

**授権の Step ごとの入力**(`open_warrant.rs:158-227`): Step 0(安全弁)・押下(ADR-208 D3)・Step 1(`IntentStore`)・Step 3(`derive_actuating`)・Step 4a(`HeuristicDefault` の観測)は今のまま。
**Step 4c に届きうる非押下の order の起案元(`ebd744b6d` で列挙、round 3 R3M1・m5)**: drift correction(`runtime/ime_refresh.rs:1030`・`:1287`)、shadow toggle(`runtime/key_pipeline.rs:1420`・`:1494`。押下 ID が載れば `press_warrant` に替わる)、Engine の判断(`state/sync_actuation.rs:365`・`:373`、`with_press(press)` で押下 ID が無いときは非押下)、InputRelay の gate の記録用(`:329`、捨てる order)。いずれも `ActuationOrder::issue`(`actuation_chain.rs:353-362`)で `issue_open_warrant` を通る。実際に Step 4c まで落ちるか(Step 1 の `IntentStore` の行・Step 3 の観測・Step 4a が無い場合)は経路ごとに未確認。
Step 4c(`OwnSsot`、Blind のとき `desired_open`)の新しい入力は未決: 今の `desired_open` は窓切替で持ち越され初期値 true を含む。`write_basis.drift` は窓切替で消えるので、そのまま使うと Step 4c は窓切替の後ほぼ授権しなくなる(round 2 m5)。**Step 4c に届く本番の呼び出し元を列挙してから入力と合格条件を決める**(未確認)。候補 S では Step 4c は今のまま(L1・L2 は `desired_open` を変えるので Step 4c の結果が変わりうる。PR #868 の `WriteCommand` は base 131〜136 に対して全変種 131〜136)。

### 4.10 半角英数トグル・戻り待ち(ADR-107・244・245)

表現はモード軸の主張(`HalfWidthToggle`・`LangDetect`)。出入りの条件は今のまま(取り下げ #6: `024ca336` のキー押下での手放しは実 Chrome × MS-IME 本体で `か` のまま・GJI の MS-IME プリセットで `ro`、revert `6b1e91b8`)。トグル中の予測は今どおり捨てる(ADR-244 M-2)。
トグルの出入りは開閉軸を触らない(MS-IME 本体のトグル中の閉の読みは信頼表で反証不可)。窓切替では離脱したスコープの行が `Eisu` と戻り待ちを持ったまま残る。BUG-220 の取り合いは `plan_rebuild_from_return_for_language_detect` のまま(前提の食い違いは §4.14-3)。

### 4.11 lang-detect・ADR-262・ADR-279

lang-detect のひらがなキーは Z の belief では `LangDetect` の主張、書き込みの根拠には入れない(§4.9)。誤判定の重さは U8。InputRelay は lang-detect の対象外(`lang_detect_shadow.rs:315`)。ADR-262 の ON の送信は主張として記録しない(「belief・IntentStore は動かさない」)。ADR-279 は受け入れ判定に打鍵列の反証の源を足す形で載る(本 ADR では実装しない)。

### 4.12 ポーリング(前提つきの案 P6)

ADR-191 決定 3 が予測を `UserImeSetIntent` にしなかった理由は 2 つ(QB1・QB2): (1) `last_intent` を立てるとポーリングが止まる、(2) `desired_open` への新イベントは `resolve_open_at` で観測に負ける。Z・L1 とも予測で `last_intent` を立てないので (1) は起きない。
P6(明示意図があってもポーリングを止めない)は別の目的の案で、今の早期 return(`runtime/mod.rs:1187-1222`)が担う役割を置き換えてからでないと採らない(experiments エントリ 13): ADR-187 の通過マークの保護、ADR-279 P0 の `drift_follow_would_ensure_timer` の起点(`:1202`・`:1219`)、「GJI の I/O 推論を drift の根拠にしない」(`:1199-1201` のコメント)、drift の周期実行の機会、「@」(BUG-113 系)との関係(未確認、§6.5 B8)。採らない場合も Z・S は成り立つ。

### 4.13 モードキーの効果の表(修飾付きを含む。BUG-221・ADR-281。候補 S・Z と独立に要る)

所有者の判断(2026-10-26): 「BUG-221 の残りは、ゼロベース設計の中でやる」。今の状態: ADR-281 の A′(GJI/Mozc 系は 0xF0〜0xF4 × Ctrl・Shift・Alt を素のキーとして予測、MS-IME 本体〈CLSID で同定〉は 0xF2 × Ctrl だけ、Win は抑止、判定は届いた VK)で修正済み・所有者の実機確認済み(Ctrl+F2)。未実施: MS-IME 本体の 0xF0・0xF1・0xF3・0xF4 × 修飾、D2'。

1. 修飾付きも、表で効果が決まるセルなら明示操作(Z)/予測(S)。決まらないセルは観測に委ねる。
2. 今の述語(`plan_key_effect_track` の `ModeKeyModifiers`)を IME ごとの写像表のデータにし、各行に根拠の種類((a) 実測の格子、(b) IME の仕様の読み、(c) IME の設定の読み取り)を付ける。修飾付きは注入学習しない(ADR-281 の却下案)。
3. D2'(GJI の F2 の後の Converter I/O で確定・取り消し)は、読めない窓 × GJI 系の開閉の観測の源の候補。まず観測のみ(§6.5 B6)。
4. ADR-281 の Shift の注意(`mode_key_follow_admits_modifiers`、`mode_key_pass.rs:265`)を引き継ぐ。
5. **押した後の状態**: 予測器は開閉を変化があるときだけ出す(`key_effect_predictor.rs:641`)。Z(と L1′)で明示操作を「最後の操作の状態」として持つには、表のセルの押した後の状態(`after_open`)が要る。round 1 版はこれを §1.1.1 の手順 3 で根拠づけたが、§1.1.1 の Engine OFF の原因は `IntentStore` の上書きだった(RB1)。手順 3 の F2 が開の記録にならなかったこと自体は事実で、L5a(窓をまたぐ時刻順)で B の F2 を A の OFF より新しい操作として扱うにも、この記録が要る。
6. 予測の入力の作り直し: `kp_predict_key_effect`(`key_pipeline.rs:1908`)の `PredictInput` は、`open` = `effective_open()`(`IntentStore` の上書きを含む)、`mode` = グローバルな `input_mode()`、`conv_raw` = `prev_conversion_mode()`、`track` = トグル中は変換モードを捨てた `key_track()`。
   「`ObservedEisu` 中の F2 が no prediction」(BUG-222 のログにも)の原因は `conv_raw`・`Indeterminate`・`observable_toggle_guard`・開閉の入力のどれかで未確認(GJI の表には `on-c10-none|hiragana → ON/0x19` がある)。Z では入力をスコープの行から作る。BUG-212 の早期 return(§4.14-4)と合わせ、「予測しない」と「段階の追跡を更新する」を分ける。

### 4.14 別セッションの Opus コードレビューからの入力 4 件(`82e00b655` で確認、症状はいずれも未確認)

1. **MozcShm の観測が打鍵中に失効しうる**: `report_mozc_shm`(`platform_state.rs:2138-2182`)は `decide_use`(`mozc_shm.rs:397-418`)が `Use` のときだけ報告し、`Held` では何も出さない。基準は `hook::last_operation_tick_ms()`(`hook.rs:345-350`、全キーと ImmCross の書き込み)。IME の状態が変わらない打鍵中は `Held` が続き、`MOZC_SHM_OBSERVATION_TTL_MS`(1000)で失効しうる。頻度は未実測(B10)。反映: Z の主張のラッチで失効の影響を受けない。S では対象外(U13 と同じく別に扱う)。
2. **`gji_evidence_absent()` が共有メモリの block の実在を見ていない**: `direct_mode_key_watch_kind_for`・`external_change_watch_applies_for`(`runtime/mod.rs:2501-2522`)は、Mozc 系(表示名の一致で GJI 系だが Google 版でない、`converter_target.rs:30-35`)なら直接読みと監視窓を外す。理由は「共有メモリが担う」だが `block_present()`(`mozc_shm_shadow.rs:34`)は見ていない。反映: 信頼表の軸、U13、B9。
3. **BUG-220 の修正の前提の食い違い**: `plan_rebuild_from_return_for_language_detect`(`half_width_alnum.rs:351-366`)は「窓 A の実 IME は半角英数のまま」を前提にし、`plan_key_stage` のコメント(`:738-740`)は「GJI の TSF 窓は持ち越す」を前提にする。保持単位の問題(§4.1)そのもの。B1 に「B でひらがなにして A に戻る」セル。コードは本 ADR では変えない。
4. **BUG-212 の早期 return が段階の追跡を飛ばす**: `key_effect_predictor.rs:594-596` の `return None` が Esc の段階の規則(BUG-162、`:610-633`)より前。反映: §4.13-6。

含めないもの(team-lead の指示どおり): Zoom の key_pacer、lang-detect の保持破棄での Shift、パニックリセットと lang-detect の保持。

## 5. O3 と方針の衝突 → 所有者の決定(O3′)

- 衝突: 方針「IME が ON でかななら必ず Engine ON」と O3「最後の明示操作が勝つ(寿命で戻らない)」は、awase の明示操作の記録と実 IME が食い違い、それを覆す観測が無い窓で衝突する(§1.1.1)。今の構造は `IntentStore` の 30 秒の TTL と、予測で行を消す規則で時間的に丸めている。
- **所有者の決定(2026-10-26)**: 「IME ON かつかな」を優先する。O3 の「最後の明示操作」を窓をまたぐ時刻順で定義し直す(O3′、§0.1)。窓 A の古い明示 OFF は、A と状態を共有する(または共有が未確定の)窓で後から ON にされた時点で失効する。独立と測れた組では失効しない(O4)。
- 帰結: 候補 S では L1′+L5a(+L4・D8 の調整)、候補 Z では「スコープ」を共有の組でまとめ、未確定の組では ON 向きの失効を足す形になる。どちらも「窓の組の共有の表」が要る。
- 残る限界: (ii) の読み(awase の OFF が A の実 IME に届いていない)は、他の窓での ON が無ければ失効しないので、観測の無い窓ではどの候補でも検出できない。合格条件から分けて「誤りが続いた時間」を報告する(§6.1)。

## 6. 検証計画

### 6.1 合格条件(V0)

| 窓モデル | I1 | I2 | I3・I4・I6・I7(O3・O4・D8) |
|---|---|---|---|
| 測った事実で混ぜたモデル(主)。**未確定の組(GJI × tsf-native など)は「独立」「共有」の両方の値で回し、両方で合格を要件にする**(round 3 R3M3。満たせない候補は B1a の結果が出るまで採らない) | 核の列(予測の外れを除く)で 0。§1.1.1 の型は **(i) の読みの列(共有または未確定の組で、B で後から ON)で 0**。(ii) の読み(OFF が A の実 IME に届いていない)は合格条件から外し、件数と「誤りが続いた時間」を報告。ハーネスの BUG-223 の 4 本は今のコードの型の回帰の列として 0(実機の事故の再現とは扱わない、§1.1) | 既定の区分以外で 0。既定の区分を含めた総数を窓の種類別に旧と並べる(U10・U2 の判断材料) | **S・Z とも同じ強さで 0**(所有者の決定) |
| 純粋な shared・純粋な per-thread(感度) | 件数を報告 | 件数を報告 | 件数を報告 |

- 候補 S′(O3・O4・D8 を満たす形に作り直した S)と候補 Z を同じ表に並べる。S′ が主のモデルで合格すれば S′ を採る(U12)。

### 6.2 ハーネスの拡張

| 写していないもの | 扱い |
|---|---|
| **窓の種類で保持単位を混ぜた窓モデル**(読める Win32・Chromium は独立、MS-IME × WT は B1a の結果に従う、GJI × tsf-native は B1a の後) | 擬似 IME の窓モデルに窓の種類ごとの共有の規則を足す。未確定の組は両方の値で回す |
| **`IntentStore` の上書きの列**(明示 OFF → 別の窓で ON → TTL 内に戻る。§1.1.1) | 字母に「30 秒以内の往復」と、必要なら `advance:11000ms`・`31000ms` を足す |
| hwnd キャッシュ復元・TSF の ON キャッシュ復元 | 旧の比較用に写す(B7 を基準線に) |
| 起動前からあるスレッド(真の初期状態が開/閉) | 両方で列挙 |
| 押下時点の前面と追跡のずれ(BUG-218) | `focus_to` の遅延 |
| 通過マーク・パニックリセット・`user_enabled` の切り替え | 事象を足す |
| lang-detect の保持・半角英数トグル・戻り待ち | 取り下げ #6 の失敗の列を必ず入れる |
| MS-IME 本体のトグル中の閉の読み | 擬似 IME に足す |
| 周期の poll、読めない窓の IMM の嘘 | 事象・字母に足す |
| 修飾付きのモードキー | 遷移表に「届いた VK × 修飾」の行(ADR-281 の CI の結果、未測定は B4) |
| 予測の入力(`unreadable`・`observable_toggle_guard`・`conv_raw`) | 本番と同じ入力の作り方を写す。BUG-222 の列 |
| 授権の入力・`WriteCommand` | Step 4c に届く入力を写す |
| `Engine::on_input`・flush、TSF の composition・cold/warm | Engine の単体テスト・実機 CI |

### 6.3 その他の検査

参照モデル(§4.4 の表から書いた小さなもの)との差分、I4 の変成関係、`cargo mutants`(受け入れ判定の条件)、台帳の回帰列(§7)。

### 6.4 段階 V0(本番は触らない)

0. 前提: B1a〜B1c・B2 の結果(または未確定の組を両方で回す)。PR #868 の測定。
1. 候補 S の未測定の案(L1′・L4・L5a・L5b)と候補 Z の純粋な核を、ハーネスの上でだけ実装する(Z の核は `crates/awase-ime-core` の新モジュール、本番からは呼ばない)。
2. §6.1 の表を CI の `test` ジョブで回す。
3. 所有者が §6.1 の結果と U2・U10・U12 で判断する。
4. 戻し方: 本番に配線していないので消すだけ。

### 6.5 CI 検証バッチ(今は実施しない。優先度順)

| # | 何を測るか | 構成 | 合否・読み方 | 結果で何が変わるか | 規模 |
|---|---|---|---|---|---|
| B0 | **観測のみのログの改善(挙動は変えない)**: `[effective-open-flip]` の行に、上書きした `IntentStore` の行の記録時刻・hwnd・経過、`desired_open` の値、直前の明示操作の種類(`UserIntentSource`・予測・lang-detect など)を足す。BUG-223 の区間 2・3・5 のように型を後から確かめられない事態を防ぐ(team-lead の提案) | 本番(develop)に診断として入れ、所有者の実機の次の事象で読む | — | 次の実機の事象で、`IntentStore` の上書き・`desired_open` の後退・予測の型を区別できる | `platform_state.rs` のログの 1 行の拡張(数十行)。V0 より前に入れてよい |
| B1a | **(最優先)GJI × Windows Terminal の保持単位**。CI の `edit>wt` が inconclusive(B で ON キーが効かない)。原因(注入の宛先・WT の起動直後・GJI の TSF の warm/cold)を切り分け、試行数を増やす。**MS-IME × WT も含め、閉の向き(A を閉にして WT へ)・往復(WT で変えて A へ、A で変えて WT へ)・起動順(WT を A の設定の前/後に起動)のセルを足す**(round 2 RM2) | `ime-scope.yml`(`ci/adr282-ime-scope`) | 対照(B 単独で ON キー → `か`)が 3/3 以上のうえで各セル 3 試行以上 | 開を共有する組の表(L4・L5a・Z の `ScopeModel`、U11)。BUG-223・§1.1.1 の窓の種類の説明 | ハーネスの修正、数十行 |
| B1b | WezTerm・UWP/XAML・同じプロセスの別スレッド・WebView2 のホスト | 同 | 各セル 3 試行以上 | 表の未確定の行 | 起動部品、数十〜百行 |
| B1c | 実機の tsf-native 窓での手動確認(所有者。dragonflyg4 に WT か WezTerm を入れる) | 手順書(A で ON にして B で `ka`、逆向き、閉の向き、各 3 回、awase 停止) | 3/3 | CI と実機の差、BUG-223 の窓の種類での決め手 | 手順書のみ |
| B2 | 新しいスレッドの窓は閉で始まるか(U2)。TSF で同定した MS-IME 本体・実アプリの新しい窓 | B1 と同じ | 3/3 閉 | U2 の判断材料 | B1 に含める |
| B3 | 窓切替から IME の状態が確かになるまでの遅れ | `sc-focusrestore-*` に journal の集計 | 分布 | 読める窓の既定・§4.1 の誤りの持続時間の見積もり | 判定 1 本 |
| B4 | BUG-221 の残り (1): MS-IME 本体の 0xF0・0xF1・0xF3・0xF4 × Ctrl・Shift・Alt | `sc-adr281-nat-*`(`tools/e2e/ime_key_matrix/check_modifier_mode_key.py`) | 各 3/3、素のセルとの一致、Ctrl の KeyUp 後の競合 | 写像表・擬似 IME の修飾の行 | 構成の追加十数行 |
| B5 | BUG-221 の残り (2): 修飾付き F2 の直後の最初の NICOLA 打鍵を文字で確かめる | `sc-adr281-awase-*` に文字の判定 | 3/3 で素の `E` が出ない | 反映の時点 | 構成と判定 |
| B6 | D2'(観測のみ)| GJI 構成で journal に予測・I/O・実 IME | 一致率・遅れ | 信頼表に入れるか | 観測専用のログ |
| B7 | TSF 窓の ON キャッシュ復元の効き方(BUG-128・218) | 実 Chrome・WT の往復 | 旧の記録 | 基準線 | 構成の追加 |
| B8 | P6 の副作用(「@」・GJI の I/O 推論が drift の材料に入るか) | P6 を有効にした版で GJI × WT の `sc-*`・`ts-*` | 「@」0 件、drift の送信件数が旧と同じ | P6 を採るか | V1 の段階 |
| B9 | 共有メモリの無い Mozc 系(公式 Mozc)× 実 Chrome で外部切替に追随する証拠があるか | `*-mozc-*` に公式 Mozc | 追随の有無・時間 | U13 | 構成の追加 |
| B10 | 打鍵中の MozcShm の `Held` の割合と失効の回数 | `mozc-shm-e2e.yml` に打鍵ストレスと観測専用のログ | 割合・回数 | §4.14-1 の影響の大きさ | 観測専用のログ |

- いずれも windows-latest(Windows Server)。所有者の実機(Windows 11)とは違いうる。

## 7. 台帳との照合(棚卸し 02-ledger の各規則)

「引き継ぐ」「代替で守る(Z)」「不要(Z)」。候補 S は今の構造に足すだけなので、全部「引き継ぐ」(壊すテストは §3)。

| 出典 | 規則 | Z での扱い |
|---|---|---|
| BUG-46・052・116、ADR-191 | 物理 IME キーの配送 | 引き継ぐ(belief の外) |
| BUG-110・114(experiments 20) | 明示意図が無い間の drift の抑止・無限再送の防止 | 引き継ぐ(D8: drift の根拠は今のまま) |
| BUG-113 | `shadow_on` を `Option<bool>` のまま | 引き継ぐ。P6 との関係は未確認(B8) |
| BUG-128・218 | 粗い鍵の復元・追跡の遅れ | 代替で守る(§4.5) |
| BUG-151 | 通過後の読み直し・意図でポーリングが止まらない | 引き継ぐ |
| BUG-157 | 通過後に `desired_open` を揃える | 不要(Z では `desired_open` が無い) |
| BUG-163 | placeholder の揃え | 代替で守る(主張なしに最初の観測) |
| BUG-186・193・220・222、ADR-244・245 | 半角英数トグルの出入り・戻り待ち | 引き継ぐ(§4.10、BUG-220 の前提は §4.14-3) |
| BUG-189・ADR-233 | 新しい Medium が古い High に勝つ | 引き継ぐ(§4.4) |
| BUG-194・200・207 | raw TSF リテラル回収 | 引き継ぐ(BUG-200 は既定 ON で機会が増える) |
| BUG-205・211・212 | 予測器の段階規則 | 引き継ぐ(BUG-212 の早期 return は §4.13-6) |
| BUG-219 | Mozc 系を `[direct-follow]` から外す | 代替で守る(信頼表。外す条件を「共有メモリの block がある」に寄せる案は U13) |
| BUG-221・ADR-281 | A′ | 引き継ぐ+範囲を本 ADR に含める(§4.13) |
| BUG-223 | (修正なし) | 比較の目標(§6.1) |
| C-b3・C-b4・experiments 26 E4 | キャッシュの `ObservedEisu`・IME-ON の経路と eisu reset | 代替で守る(§4.6・§4.10) |
| ADR-087・089・098・106・165・179・187・188・199・205・208・212・213・244・255・260・262・263・268・273・280 | (棚卸し 02-ledger §B) | 引き継ぐ(書き込み側・予測器側・観測の源は変えない。ADR-187 の「意図を捨てる」と ADR-188/205/268 の追随の尾部は、Z では受け入れ判定に吸収) |
| 定数 | `OBSERVATION_FRESH_WINDOW_MS`=3000 等 | 値は変えない。`EXPLICIT_*_INTENT_TTL_MS` は授権の根拠に残る |

## 8. 取り下げた方式との照合(棚卸し 03-constraints (d) の 9 件 + experiments エントリ 13・20・26)

| # | 取り下げた方式 | S | Z |
|---|---|---|---|
| 1 | 観測・belief に合わせて IME へ書き直す補正 | 繰り返さない(書き込みは変えない) | 繰り返さない(D8・D9) |
| 2 | 観測が無いことを根拠に意図を偽装する | L2 は `desired_open` を窓の種類で書く。`last_intent` を立てないので意図の偽装ではないが、`desired_open` の「既定」の意味をさらに混ぜる(X2)。L1 も予測で `desired_open` を書き、ADR-191 決定 3 の実装の固定(予測は `desired_open` を書かない。ドリフト補正が IME へ書き戻さないため)を破る | 既定は自分の名前の根拠で持つ |
| 3 | 窓種から「日本語 IME が無い」を観測として書く | 繰り返さない | 繰り返さない |
| 4 | 信頼度優先で古い High を残す | 変えない | 繰り返さない |
| 5 | 時間だけで復元を抑制する | L5 の「OFF の TTL を短くする」は採らない | 時刻の新しさで決める |
| 6 | キー押下でトグルを手放す | 変えない | 繰り返さない |
| 7 | IME OFF に送るキーを動かす | 変えない | 変えない |
| 8 | 実測なしの暫定定数 | 足さない | 足さない(行の寿命は流用) |
| 9 | 窓切替で「ON へ戻す」方向の修正は BUG-128・218 を再発させうる | L1・L2・L4 は ON へ戻す方向を含む。L2 は I2 を増やす(実測)。BUG-128・218 の列をハーネスで確かめる | 既定の ON は記録の無い読めない窓で ON に倒す(O2)。授権に入らないので force-ON は起きない |
| E13 | 1 つのガードが複数の経路を守る | — | P6 は前提つき(§4.12) |
| E20 | drift の無限再送 | — | D8 |
| E26 | eisu reset の循環デッドロック対策 | — | §4.6 |

## 9. 移行(候補ごと)

- **候補 S**: 案ごとに 1 PR。壊すテスト(§3)の期待を、測定(§14)と所有者の判断を根拠に書き換える。再発ファミリーなので `.claude/rules/fix-requires-evidence.md` の (a)(回帰テスト: オラクルの golden と BUG-223・§1.1.1 の列)。戻し方は revert(`.claude/rules/experiment-logging.md` に従い失敗の条件を本文に書く)。
- **候補 Z**: V0 → V1(shadow: 本番で旧と新を並べ、食い違いを journal の `[belief-v2]` に)→ V2(隠し設定で切替)→ V3(撤去。`ImeEvent` の列挙値は残す)。各段の終わりの判定は、V1 で「新が誤り」の分類が既定の区分以外で 0、V2 で所有者の実機で一定期間 BUG-223 型の報告が無いこと。

### 9.1 候補 Z の見積もり(round 1 M8)

参照(`git grep -F -c <語> ebd744b6d -- '*.rs'`): `desired_open` 402 か所・39 ファイル、`effective_open()` 161・27、`intent_store` 86・14、`input_mode()` 83・17、`explicit_intent()` 76・14。`platform_state.rs` 5,634 行・`key_pipeline.rs` 3,632 行・`ime_model.rs` 4,417 行・`focus_tracking.rs` 975 行。
主な作業: 新モジュール(800〜1,500 行の見当、根拠なし)、ハーネスの拡張(千行規模)、`effective_open()`・`input_mode()` の読み手 27 ファイルの分類(`half_width_alnum.rs`・`conv_classify.rs`・`explicit_press.rs`・`force_guard.rs`・`focus_probe_plan.rs`・`output/conv_actuation.rs`・`output/mod.rs`・`runtime/ime_refresh.rs`・`indicator_glyph_shadow.rs`・`lang_detect_shadow.rs`・`message_handlers.rs`・`mozc_shm_shadow.rs` ほか)、
V1 の shadow で殻(ADR-275 で core へ移さないと決めた `platform_state.rs`)が一時的に数百行太る、journal の再生(`journal_replay.rs`)、`architecture_guard.rs` の固定テスト群、dylint(`ime_event_guard`・`observation_source_guard`)、`drift_follow_would.rs`、`ime_decision_view.rs`、不具合報告のスナップショット、予測器(§4.13)。
候補 S の各案は数行〜数十行(PR #868 の文書〈`5efea3527`〉の変種の定義の行: L1 +7 行、L2 +9 行、L3 +33/−12 行。`ime_model.rs`/`platform_state.rs` の差分)+ 壊すテストの書き換え。O3・O4・D8 を課した S′(L1′+L4+L5a+D8 の調整)は未実装で規模も未見積もり。

**L4 か S の組み合わせが合格したときに撤去する Z の部分**(round 2 RM3 の求め): スコープの表・`ScopeBelief`・`Claim` のラッチ・`Hwnd` の正規化・行の寿命・`OpOrigin` の belief 側の型・`evolve`/`decide`/`derive` の核・V1〜V3。残すもの: D8 の原則(今の構造がすでに満たしているかを確かめるだけ)、§4.13 の写像表、§4.4 の信頼表の考え方(「読めない窓 × GJI 系の IMM を反証に使わない」を今の `derive_any` の入口に入れるか)、CI 検証バッチ。

## 10. 却下した案

| 案 | 理由 |
|---|---|
| 局所修正だけで済ませる | **まだ捨てていない**(§0.3・U12) |
| hwnd を鍵にする | 読める窓では同じスレッドの別 hwnd は共有(191 spec §3.1)。読めない窓は B1 で見直す |
| グローバル 1 つのまま窓切替で既定へリセット | O4 に合わない、戻った窓の明示 OFF を失う |
| belief の主張に TTL を付ける(Z)・時間で既定へ戻す折衷 | O3 に合わない。U14 は所有者の決定(O3′: 窓をまたぐ時刻順で失効)で確定し、時間による折衷は採らなかった |
| `IntentStore` の OFF の TTL を短くする | 時間だけの規則で実測の根拠が無い(取り下げ #5・#8) |
| 不確実なときは常に OFF・lang-detect の判定を待つ | O2 に反する / 重さと誤判定(ADR-280) |
| 観測を窓ごとに保存して観測として使う | ADR-106 に反する |
| 別スコープの明示操作を弱い証拠として伝播する | どちらに倒しても片方の真で外れる(§4.1) |
| `Written` と `Predicted` で覆す条件を変える | O3(同じ重さ)に合わない |
| 共有メモリの `Held` の読みで観測の年齢を延ばす | round 2 RM4(§4.4) |
| `InheritOnEntry` を初期値に入れる | round 2 RM2・RM6(§4.1)。U11 と B1a の後 |

## 11. リスク

1. 保持単位の誤り(読めない窓は未測定。粒度を誤ると I1 と I3 が両立しない)。
2. 既定 ON の誤り(端末、BUG-200)。per-thread の窓で I2 が増える(U2・U10)。
3. 開を共有する窓の組の一般化(測ったのは開の一方向・2 試行)。
4. 押下時点の鍵の取得(`hwndFocus` が UWP・TSF ネイティブで真のフォーカスを返すか)。
5. P6 の副作用(§4.12)。
6. lang-detect の誤判定が belief で利用者の操作と同じ重さ(U8)。
7. 予測の外れが窓切替をまたいで残る(Z・L1)。
8. Z の移行の規模(§9.1)。
9. Z の寿命の無い主張: §1.1.1 の (ii) の型(OFF が実 IME に届いていない)は、O3′ でも他の窓での ON が無ければ失効しないので、今の 30 秒から最大 1 時間へ延びうる。S′ も TTL に頼らないなら同じ(§5)。
10. MS-IME 本体のトグル中の閉の読み(信頼表の行の漏れ)。
11. ハーネスの仮定への過適合。
12. 候補 S の L4・L5a は「窓の組の共有の表」を今の構造に足す。測定が小標本のまま表を作ると、誤った組で持ち越し/上書きの規則が働く。

## 12. 未決(所有者の確認が要ること)

| # | 問い | 提案(決定ではない) |
|---|---|---|
| U1 | 方針の例外に `disable_apps`・`FocusKind::NonText`・日本語でない IME を含めるか | 含める(今の挙動) |
| U2 | 実測の事前(新しいスレッドは閉)を O2 より先に使うか(per-thread の窓で I2 を増やさない条件) | **判断は V0 の後にする(その了承を求める)**。判断材料: B2、ハーネスでの差 |
| U3 | InputRelay を O2 の ON 寄りに含めるか(事実: actuation を所有しない〈`ime_actuation_decision.rs:152`〉、観測を belief に取り込まない〈`class_names.rs:192-193`〉、lang-detect の対象外〈`lang_detect_shadow.rs:315`〉、予測の `unreadable` から除く。窓切替の後処理での扱いは未読) | 含めない |
| U4 | O4 の「窓ごと」を「IME が状態を持つ単位ごと」と読んでよいか | 測った単位で読む(読める窓・Chromium は独立。tsf-native は B1a〜B1c の後) |
| U5 | JIS かな入力方式のときの Engine | OFF のまま |
| U6 | `compute_state` の外の入口(X7)をそろえるか | そろえない |
| U7 | 信頼表の未確定のセル | V0 と実機 CI で決める |
| U8 | awase 起点の明示操作(lang-detect 等)の belief での重さ | 同じ重さ(書き込みには入れない) |
| U9 | 修飾付きの割り当てを IME の設定から読む範囲 | 今は扱わない |
| U10 | O2 の適用範囲: 端末(WT・WezTerm)を ON 寄りから外すか | **判断は V0 の後にする(その了承を求める)**。判断材料: 窓の種類別の I2 の増加(L2 の実測: msime shared 35 → 99、per-thread 57 → 165。増えたのは読めない窓) |
| U11 | 実 IME が窓をまたいで開を引き継ぐと測れた組(候補: MS-IME × WT、開の一方向・2 試行)に O4 を適用するか | B1a の閉の向き・往復・起動順の結果を見て問う。それまで初期値は `PerThread` |
| U12 | V0 で候補 S が合格したら、Z ではなく S を採るか | 採る |
| U13 | 共有メモリの無い Mozc 系で直接読み・監視窓を戻すか | B9 の後 |
| (U14、確定) | O3 と方針の衝突 | **所有者の決定(2026-10-26)**: 「IME ON かつかな」を優先。O3′(§0.1・§5) |

## 13. 所有者の判断(2026-10-26、決定)

1. 「かな」= ひらがな・カタカナ。半角英数・直接入力は含まない。
2. 不確実なときは窓の種類で分ける。読めない窓は ON 寄り、読める窓は観測。
3. 最後の明示操作が勝つ(ON も OFF も対称)。窓切替や観測の寿命切れで古い値に戻らない。観測が反証する場合の扱いは設計する。
4. 窓切替では IME の状態を窓ごとに独立させ、持ち越さない。
5. BUG-221 の残りはこの設計の中でやる。CI 検証はこの文脈でまとめてやる。L1 は tsf-native の CI 測定(B1a〜B1c)と、O3・O4 を S に課す決定の後に決める。
6. **O3・O4 は候補 S にも要件として課す**(round 3 R3B1 への回答)。
7. **O3 と方針が衝突したら「IME ON かつかな」を優先**。O3 の「最後の明示操作」は窓をまたぐ時刻順(O3′)。

## 14. 測定の結果

### 14.1 局所修正 L1〜L3(PR #868、OPEN。`5efea3527` の文書の表を確認して引いた。CI run 40296295547 で決定的に一致と同文書)

I1(T=0。括弧は読める窓の poll が 1 回届いた後も残る本数):

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread |
|---|---:|---:|---:|---:|---:|
| base | 315 (290) | 405 (375) | 405 (375) | 197 (172) | 185 (155) |
| L1 | 183 (164) | 153 (135) | 153 (135) | 191 (172) | 185 (167) |
| L2 | 229 (208) | 256 (232) | 256 (232) | 201 (180) | 216 (192) |
| L3 | 189 (173) | 165 (148) | 165 (153) | 195 (179) | 181 (164) |
| L1+L2 | 191 (172) | 184 (164) | 184 (164) | 199 (180) | 216 (196) |
| L2+L3 | 197 (183) | 194 (177) | 194 (184) | 203 (189) | 220 (203) |

- 核の最小列(msime): shared base 36 → L1 0・L2 14・L3 16・L1+L2 6・L2+L3 22。per-thread base 20 → L1 32・L2 18・L3 20。長い静止の字母で shared の L3 は 80。
- I2(核の字母、msime): shared base 35 → L1 31・L2 99・L3 29・L2+L3 67。per-thread base 57 → L1 109・L2 165・L3 57・L2+L3 165。
- BUG-223 の 4 本 + `key:hiragana > focus:wezterm`: base の msime/msime-native shared で 5/5 違反、全変種で 0/5。T=2000ms でも本数は変わらない。`WriteCommand` は base・全変種とも 131〜136。
- 限界(同文書): TSF 窓・hwnd キャッシュの復元を写していない。L3 の授権の参照は窓の鍵のまま。**§1.1.1 の `IntentStore` の上書きの型は、この測定の字母で現れているかが分からない**。

### 14.2 結論(現時点)

- **ハーネスの shared の型(BUG-223 の 4 本)は L1 で消える**。ただし実機が全窓で共有という事実は無く(§1.4)、実機の BUG-223 の区間 2・3・5 がこの型かは未検証(§1.1)。L1 は O4(独立の組への持ち越し)と D8(Step 4c)に反するので、そのままでは採れない。
- **実機の再現(§1.1.1)は `IntentStore` の上書きの型で、L1 では消えない**。消すには **L1′+L5a**(押した後の状態で記録し、共有または未確定の組で窓をまたぐ時刻順に古い OFF を失効させる。所有者の決定 O3′ に沿う)が要る。(ii) の読みはどの候補でも消せない。
- **残る 153 本(L1 の後、msime shared)は嘘の観測 32・外部変更 121** で、どの候補でも減らない。観測の手段(信頼表・MozcShm・ADR-205・字形・ADR-279)と既定の設計が要る。
- **per-thread の窓では、L1〜L3 は I1 と I2 を同時に減らさない**。O2 を守る限り、Z の既定も L2 と同じ向きで I2 を増やす(§2.1)。per-thread の窓で I2 を増やさない条件は U2・U10 で、Z を採ること自体ではない(round 2 RM1 で訂正。round 1 版の「per-thread ではゼロベースが要る」は支えられない)。
- **窓の種類で保持単位が違う**(読める窓・Chromium は独立、MS-IME × WT は開の一方向の引き継ぎの候補、GJI × tsf-native は未確定)。単一の局所修正は片方の窓で I2 を増やすので、窓の種類で規則を分ける L4(と L5a)を V0 で測る。L4・L5a は「窓の組の共有の表」を今の構造に足すだけで、Z ほどの移行は要らない。
- **Z が要るのは、L4・L5a の組み合わせでも消えない型が残り、Z がそれを消すと示せたときだけ**。今は示せていない(§4.0)。

## 15. Opus レビュー round 1 への対応(2026-10-26)

| ID | 対応 |
|---|---|
| B1 | 反映(出典の範囲の限定、粒度のデータ化、モデル別の合格条件、B1 を V0 の前提に)。round 2 で数字の取り違え(9 本)をさらに訂正 |
| B2 | 反映(D8、Step 0〜4c、drift (a) の訂正、Step 4a のため `HeuristicDefault` の書き込みを残す) |
| M1 | 反映(1 ソースの観測で成り立つ受け入れ判定、Low の行) |
| M2 | 反映(鍵の関数を 1 つ、正規化) |
| M3 | 反映(MS-IME 本体のトグル中の閉の読みは反証不可) |
| M4 | 反映(I1 から予測の外れを除く、I2 の総数を旧と並べる、I6 の範囲) |
| M5 | 反映(L1〜L3 を定義・測定、§9 A の理由の訂正) |
| M6 | 反映(実測の事前を U2 に)。round 2 RM6 で `InheritOnEntry` も初期値から外した |
| M7 | 反映(端末のコスト、U10、U3 の事実) |
| M8 | 反映(見積もりの作り直し) |
| M9 | 反映(P6 を前提つき、早期 return の役割、experiments 13・20・26) |
| m1〜m9 | 反映(`engine.rs:272`、`applied` の置き場所、読める窓の `Unknown`、値の流用の理由、U3 の事実、BUG-113 の行、status の依存 PR) |

## 16. Opus レビュー round 2 への対応(2026-10-26)

| ID | 対応 |
|---|---|
| RB1 | 反映: §1.1.1 をログの書式(`platform_state.rs:1254`、`intent_store_override`)に合わせて書き直した(手順 4 の OFF は `IntentStore` の上書き、`DesiredFallback` は true)。L1 では消えないこと、Z は 30 秒から 1 時間へ悪化させることを明記。§4.13-5 の根拠を書き直し。手順 1 の中身の確認と、O3 と方針の衝突を U14・§5 に。L5a・L5b を候補 S に足した |
| RM1 | 反映: §1.2 に字母をそろえた表(shared 405・per-thread 全字母 185・核の字母 9)。summary の「9 本」を削除。§2.1・§14.2 に「O2 の下では Z も per-thread で L2 と同じ I2 のコスト。避ける条件は U2・U10」 |
| RM2 | 反映: `InheritOnEntry` を初期値から外し、候補の規則を「開の引き継ぎだけ、直前の窓から」に狭めた。B1a に閉の向き・往復・起動順のセルを足した |
| RM3 | 反映: タイトル・summary・§0 を「比べて決める」に作り直し、確定(O1〜O4・D8・D9)と比べる候補(S・Z)を分けた。§4.0 に Z が簡素化になるかの評価(示せていない)、§9.1 に S が合格したときに撤去する Z の部分の一覧 |
| RM4 | 反映: `Held` で年齢を延ばす規則を削除(§4.4・§10) |
| RM5 | 反映: §4.9 に今の経路 → `OpOrigin` → belief/drift/授権の写像表。`OpOrigin::Engine` を `UserCommand` に統合。`drift_correction.rs:230` はテストの構築と確認 |
| RM6 | 反映: `ScopeModel` の初期値は O4 のとおり全部 `PerThread`。開の共有は U11 の選択肢 |
| m1 | 確認のみ(引用は一致) |
| m2 | 反映: lang-detect の設定値は報告の転記で未確認と明記 |
| m3 | 対応不要(妥当との評価) |
| m4 | 反映: I3 の開を引き継ぐ窓での定義は U11 と B1a の後に書くと明記 |
| m5 | 反映: Step 4c の入力は未決とし、本番の呼び出し元の列挙を先にする(§4.9・I7) |
| m6 | 反映: L4 の定義に「GJI × tsf-native は B1a の後」 |
| m7 | 反映: 保持単位の表に試行数を併記し、「スレッドごと」と言い切らない |

## 17. Opus レビュー round 3 への対応と所有者の決定(2026-10-26)

| ID | 対応 |
|---|---|
| R3B1 | 反映(所有者の決定): O3・O4 を S にも要件として課す。§0.1・§0.3・§2(I3・I4・I6・I7 を S・Z とも)・§6.1 の表に I3・I4・I6・I7 の列を足し、TTL で戻る・持ち越す S の案は不合格と明記。比べる対象を S′ と Z に絞った |
| R3M1 | 反映: L1・L2 が Step 4c を通じて D8 に反することを §3 に明記し、S を「+ D8 の調整」の形で定義し直す。今の構造でも Step 4c は D8 を満たしていないこと(§0.1)、Step 4c に届きうる非押下の order の起案元の列挙(§4.9) |
| R3M2 | 反映: §1.1.1 を消すには L1′+L5a が要る(§3・§14.2)。L5a の「他の窓」(共有または未確定の組)・「明示操作」(予測の閉を含む)・向き(共有の組は対称、未確定の組は ON 向きだけ、所有者の決定に沿う)を定義 |
| R3M3 | 反映: 未確定の組は両方の値で合格を要件に(§6.1)。§1.1.1 の (i) の読みだけを合格条件にし、(ii) は報告と「誤りが続いた時間」に分けた |
| R3M4 | 反映(確認できず): 区間 2・3・5 は元のログが失われ型が未検証(team-lead の確認)と §1.1 に明記し、ハーネスの BUG-223 の 4 本を「今のコードの型の回帰の列」に弱めた。B0(観測のみのログの改善)を §6.5 に足した |
| m1 | 反映: summary の「B で ON にした後」を「A の実 IME が ON だった(理由は未確認)」に |
| m2 | 反映: status・§1.4・§13 の L1 の条件を「B1a〜B1c と O3・O4 を S に課す決定の後」に |
| m3 | 反映: §1.3 の S の列に L4・L5a の「窓の組の共有の表」(Z の `ScopeModel` と同じデータ)を足した |
| m4 | 反映: L3 +33/−12 行の出典(PR #868 の文書の変種の定義)を添えた |
| m5 | 反映: I7 の合格条件を Step 4c の呼び出し元の列挙を前提に(§2・§4.9) |
| m6 | 反映: §8 #2 に L1 が ADR-191 決定 3 の実装の固定を破ることを足した |

所有者の決定の反映箇所: (1) O3・O4 を S に課す → §0.1・§0.3・§1.4・§2(I3・I4・I6・I7)・§3(O3・O4 との関係)・§6.1・§13-6。(2) U14 → §0.1 の O3′・§1.1.1・§2(I3)・§3(L5a の向き)・§5・§12・§13-7・§14.2。
U2・U10 は「判断を V0 の後にすることの了承」に絞った(§12)。
