# 「IME の真の状態」と「Engine の活性」の食い違い: 基準の測定（2026-10-26）

所有者の方針「IME ON かつ ひらがなのときは、必ず awase（NICOLA の Engine）が ON」を、擬似 IME・擬似窓（ADR-277）の上で
検査するオラクルを作り、**いまの実装がどんな列でこの不変条件を破るか**を数えた。後続のゼロベース設計との比較の基準であり、
実装は変えていない。テスト: `crates/awase-windows/tests/belief_engine_invariant.rs`、違反の一覧（golden）:
`crates/awase-windows/tests/golden/belief_engine_invariant_violations.txt`（CI の `test` ジョブ、約 29 秒）。
数値はすべて golden の値（固定 seed・列挙順固定で決定的。CI run 40275882156 で一致を確認）。

## 不変条件の定義

- **真の状態**: 擬似 IME の `TrueState`（いまフォーカスのあるスレッドの IME。ADR-277 の実測の格子 3 種が真値）。
- **Engine の活性**: ハーネスの `StepRecord::engine_active`。本物の `Engine::compute_active` に、本物の `ImeStateHub::effective_open_at`・
  `input_mode`・`is_japanese_ime` から組んだ文脈を渡した値（`user_enabled` はハーネスでは常に真）。
- **I1（必須）**: 真が「開 かつ ひらがな（conv が NATIVE と ROMAN のビットを持つ。ひらがな 0x19・カタカナ 0x1B 等）」なら Engine は Active。
- **I2（参考。逆向き）**: 真が「閉」または「開×半角英数（NATIVE なし）」なら Engine は Inactive。
- 検査は各事象の直後の状態に対して行う。**列の前に最初の観測（ImmCross）で belief を真に揃える**（起動直後の placeholder
  `desired_open=true` による食い違いは BUG-163 の領分なので測らない）。
- **時間の許容 T**: T=0 は事象の直後に食い違っていれば違反。T=2000ms は、最初に違反した後に事象が何も来ないまま 2000ms 経っても残るもの。
  実機の「次の観測が来るまで」に近づけるため、**読める窓（Standard）では 2000ms の間に周期の ObserverPoll が真の開閉を 1 回届ける**場合も数えた
  （読めない窓には届かない）。時間の経過そのもの（観測の鮮度切れ 3000ms）は字母の `advance:3500ms` で入る。

## 窓モデルと IME の種類

- **shared**: 全部の窓が同じスレッドで IME の状態は窓をまたいで共有される。実機の事故（BUG-223: 窓切替の後も IME は ON のまま）に合うので主。
- **per-thread**: 窓ごとに別スレッドで、初めて行く窓の IME は閉×ひらがな（ADR-191 §3.2、ハーネスの既定）。窓を移ると真の状態が入れ替わる。
- 窓: explorer（Standard・読める）、xaml（同じプロセスのクラス違い、imm32-unavailable）、wezterm（tsf-native）、chrome（imm32-unavailable）、
  relay（InputRelay）。explorer は学習による降格（`demote`）で imm32-unavailable にもなる。
- IME の格子: atok（GJI の ATOK プリセット）、msime（GJI の MS-IME プリセット）、msime-native（MS-IME 本体）。**BUG-223 の事故は F2 が閉から ON にする
  格子（msime・msime-native）でだけ再現する**。ATOK プリセットの F2 は閉のまま変わらない（`atok.json` の `off-c19-none|hiragana` = OFF）。

## 字母と列挙の範囲

| 事象 | 内容 |
| --- | --- |
| 物理キー | F0（英数）・F2（ひらがな）・IME-OFF（0x1A）・IME-ON（0x16）・半角/全角（0xF3）。awase は消費せず IME へ通す |
| 利用者の明示 | `user-off`（`ExplicitIntent` を記録して OFF を書く） |
| 窓切替 | 5 つの窓の間（同じプロセス内のクラス変化を含む） |
| 観測 | `obs:poll`（Medium、真を読む）・`obs:focus-probe`（Low・BeliefOnly、真を読む）・`obs:poll-LIE`（Poll が真と逆を返す）・`obs:conv-infer-open`（読めない窓の `ConvInference` が「開」と推測する。閉のときは嘘） |
| 外から | `external-on`/`external-off`（言語バー・他アプリ。awase は見ていない） |
| 時間・学習 | `advance:3500ms`（鮮度切れの先まで）・`demote-by-learning` |

- 全字母（19 種）× 長さ 3 まで: shared の 3 格子（検査した列 3,412 / 3,360 / 3,360）。per-thread は核の字母（12 種）で長さ 3（1,110）。
- 小さい字母（7 種: F2・OFF キー・明示 OFF・窓 3 つ・時間）× **長さ 4 まで**: msime shared（1,969）。実機の事故の長さに合わせた。
- I2 は核の字母（12 種）× 長さ 3 まで。
- ランダム探索: 固定 seed・全字母・長さ 8〜12・設定ごとに 300 本、最初の違反で打ち切り（違反の最小化は先頭 20 本）。
- **「最小列」= 事象を 1 つ削ると違反しない列**。全列挙は違反した列を延ばさないので、集めた列は「最後の事象で初めて違反する列」で、
  削った列の違反は集めた集合の接頭辞照合で判定している。
- 文字キー・明示 ON・ImmCross の観測は時間の予算（CI 約 30 秒）のため字母から外した（`UserSet(true)` は `key:ime-on` と似た結果、ImmCross は Poll と同じ観測の性質）。
  最初に流した版は全字母（22 種）で CI の `test` が約 50 秒だった。

## 違反の総数（I1）

| 設定 | 列の範囲 | 初めて違反する列（T=0） | 2000ms 事象なしで残る | poll が届いても残る | 最小列 | うち「awase の操作・窓切替・正しい観測だけ」 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| atok shared | 全字母・長さ 3 | 315 | 315 | 290 | 31 | 18 |
| msime shared | 全字母・長さ 3 | 405 | 405 | 375 | 65 | 36 |
| msime-native shared | 全字母・長さ 3 | 405 | 405 | 375 | 65 | 36 |
| msime shared | 小さい字母・長さ 4 | 136 | 136 | 136 | 4 | 4 |
| msime per-thread | 核の字母・長さ 3 | 9 | 9 | 5 | 9 | 6 |

- ランダム探索（300 本中の違反）: atok shared 182、msime shared 220、msime-native shared 218、msime per-thread 171。最小化した 20 本のうち全列挙の最小列に無いもの
  （長さ 4 以上や、字母を絞った per-thread の全字母側の列）が 1〜9 本あった。たとえば `advance:3500ms > obs:poll-LIE > key:hiragana > key:ime-on`（belief は開だが入力モードが
  `ObservedEisu` のまま Engine が OFF）。
- **T の許容の効果**: 事象が来ない 2000ms では**一つも治らない**（T=0 と T=2000 の違反数が同じ）。観測を届ける仕組みが無い限り、時間では治らない。
  読める窓（explorer）の周期 poll が 1 回届くと、msime shared では 405 本中 30 本（explorer(std) で終わる 84 本のうち 30 本）が治る。**読めない窓（tsf-native・
  imm32-unavailable・relay）で終わる違反は 100% 残る**（msime shared: chrome 93・wezterm 93・relay 93・xaml 30 本のすべて）。

## 違反の分類（msime shared・全字母・長さ 3。T=0 の件数）

| 分類（`decided_by`） | 件数 | poll が届いても残る | 意味・最小列の例 |
| --- | ---: | ---: | --- |
| belief-off/desired-fallback | 297 | 282 | **BUG-223 の本体**。IME ON の予測（F2・F0・ON キー・半角/全角）が窓切替で捨てられ、`desired_open`（前窓の false）に戻る。`key:hiragana > focus:wezterm` |
| belief-off/derive-high | 30 | 30 | 起動時に揃えた High（ImmCross の閉）が鮮度 3000ms の間、後の Medium より勝つ。`external-on` |
| belief-off/explicit-intent | 23 | 23 | 明示意図の `desired_open=false` が真の ON と食い違ったまま残る |
| belief-off/most-recent-trusted | 21 | 13 | 観測が鮮度切れでフォールバックした値が閉 |
| belief-off/key-effect | 18 | 17 | 予測の閉が真の ON を覆っている（外部変更との組み合わせ） |
| belief-on-but-engine-off（mode=ObservedEisu） | 10 | 9 | belief は開だが**入力モードが半角英数の観測のまま**で Engine が OFF（belief の開閉でなく入力モード軸の食い違い） |
| belief-off/derive-medium | 6 | 1 | Poll の単独の false（嘘の観測 `poll-LIE` のとき）。poll が届けば治る |

- **列の性質**: awase の操作・窓切替・正しい観測だけ 228、嘘の観測を含む 56、外部変更を含む 121（msime shared、初めて違反する列）。外部変更（awase に見えない）は
  観測が無ければ原理的に直せない。**根治すべき核は「awase の操作・窓切替・正しい観測だけ」の違反**（最小列 36 本）。
- 最後の事象は、`focus`（窓切替）が 252/405 で最多、`external-on` 129、`key:eisu`・`key:hankaku-zenkaku` 各 10、`obs:poll-LIE` 4。
- 違反が出た時点の窓（msime shared）: chrome 93・relay 93・wezterm 93・xaml 30・explorer(std) 84・explorer(imm32-unavailable) 12。**窓切替で
  読めない窓へ移った直後が大半**。

### 実機の事故（BUG-223）の再現

`bug223_sequences` の 4 本（OFF キー → F2 → 窓切替、明示 OFF → F2 → 窓切替、tsf-native で OFF → F2 → imm32-unavailable、imm32-unavailable で明示 OFF → F2 → tsf）は、
**msime・msime-native の shared で 4 本とも I1 違反として出る**（分類 `belief-off/desired-fallback`、事象なしでも poll が届いても残る）。atok と per-thread では違反しない
（前者は F2 が ON にしない、後者は窓を移ると別スレッドの IME が閉になって真と一致する）。専用のテスト
`bug223_sequences_are_i1_violations_when_ime_state_is_shared` が、msime・msime-native の shared でこの 4 本が違反であることを固定している。

**最小列は OFF キーも明示 OFF も要らない**: 小さい字母・長さ 4 の全列挙の最小列は 4 本だけで、`key:hiragana > focus:wezterm`・`key:hiragana > focus:chrome`、
`focus:wezterm > key:hiragana > focus:xaml`、`focus:chrome > key:hiragana > focus:xaml`。つまり「F2 が `KeyEffectPredicted` としてしか記録されず `desired_open` を書かない」
非対称そのものが原因で、前の OFF は事故を起こしやすくするだけ（BUG-223 の「より強い原因」の項と一致）。ただし msime 小さい字母の 136 本はすべて `desired-fallback`・
窓切替の直後で、(a) の「`DeriveMedium{ObserverPoll}` の単独」と「`DeriveHigh(ImmCrossProbe)` の単独」の 2 区間は**この字母では再現しない**
（ハーネスは読めない窓の IMM 系の観測を流さない。下記の限界）。

## 窓・IME 別の傾向

- **IME 別**: msime と msime-native は同じ総数（405 本）だが内訳が違う（msime-native は `key-effect` が 28 で、`ObservedEisu` が 0。F0 が MS-IME 本体では閉にするため）。
  atok は F2 が ON にしないので `desired-fallback` が 189 と少なめで、原因は ON キー（0x16）・半角/全角・外部変更に寄る。
- **窓別**（msime shared）: 読める explorer(std) 84 本のうち poll で治るのは 30 本、**読めない窓は poll が届かないので 100% 残る**。窓切替の直後の違反が多いのは
  `FocusChanged` が予測を捨てる（`reduce_focus_changed`）ため。
- **per-thread（窓ごとに別スレッド）**: 違反は 9 本（msime、核の字母・長さ 3）に減る。原因は窓切替での持ち越しではなく、F0 で入力モードが半角英数の観測のままになる軸
  （`key:eisu > focus:xaml > key:eisu`）と、戻った窓で起きる `desired-fallback`（`key:eisu > focus:wezterm > focus:explorer`）。

## I2（参考。逆向き）

I2 の違反は核の字母・長さ 3 で atok 43・msime 35・msime-native 35・msime per-thread 57 本。最小列の大半は**嘘の観測を含む**
（`focus:wezterm > obs:conv-infer-open`: IME が閉なのに conv の推測が「開」を報告し、Engine が ON。GJI × TsfNative の既知の誤判定を字母に入れたので出る）。
「awase の操作・窓切替・正しい観測だけ」の最小列は shared で 4 本（`key:eisu > focus:wezterm > key:eisu` など、F0 の半角英数を ON と読み違えて Engine ON）、
per-thread では `key:eisu > focus:xaml`（新しい窓の IME は閉なのに、前の窓の belief を持ち越して Engine ON）が加わる。I1 が主、I2 は誤って ON のとき生の英字が
NICOLA の文字になる逆の事故の基準。

## 限界と偽陽性の疑い

- **擬似 IME が表せないクセ**: ADR-277 の QUIRKS のうちモデル化済みは 6 件中 3 件（Q3 読み戻し遅延・Q4 外部クローズ監視・Q6）。cold/warm・TSF の composition・入力中の
  文字の確定は無い。本テストは Q3・Q6 のスイッチも、ADR-205 の外部変更の監視窓（`with_external_close_watch`）も使っていない（監視窓が無い＝Q4 の症状のまま）。
- **窓切替の本物の処理の一部が写っていない**: `on_focus_process_changed` の hwnd キャッシュの復元、TSF ネイティブ窓の ON キャッシュの復元（`plan_tsf_cache_restore`、
  BUG-128/218）、新しいスレッドの閉の仮定は、ハーネスの `focus_to` は写していない（`harness.rs` の doc）。**TSF 窓へ「戻る」型の列は、本物では ON キャッシュの復元で
  救われる可能性があり、偽陽性の疑い**がある。初めて行く窓（最小列の `key:X > focus:wezterm`）には効かない。
- **読めない窓の観測**: ハーネスは読めない窓（tsf-native・imm32-unavailable・InputRelay）へ `ObserverPoll`・`ImmCrossProbe` を流さない（本番のプロファイルの意味どおり）。
  BUG-223 (a) の「GJI の tsf-native/imm32-unavailable 窓の IMM の読みが信頼できず false を報告する」区間は、この字母では出ない（`obs:poll-LIE` は読める窓だけ）。
  `FocusProbe` の観測は真を読むものだけ。TSF 系の観測（`TsfObserver` 等）・`GjiIoInference`・`MozcShm`・`TaskbarGlyph`（ADR-273）は無い。
- **周期の poll は事象として入れていない**（T=2000 の列でだけ 1 回入れる）。本物の ObserverPoll は 500ms 周期で、読める窓の違反は数 100ms〜数秒で治るはず。この表の
  「poll が届いても残る」が現実に近い。
- **嘘の観測の仮定**: `obs:conv-infer-open` は閉のときだけ嘘で、保守的に全て「嘘」側に数えた。`obs:poll-LIE`（読める窓の Poll が真と逆）が実機でどのくらい起きるかは未測定。
  列の性質の「嘘の観測を含む」はこの意味で偽陽性の疑いがあり、「外部変更を含む」は観測が無ければ原理的に直せない（ADR-205 の監視窓は Imm32Unavailable×GJI の一部だけ）。
- **打鍵の内容**: ハーネスは文字キーを NICOLA の変換に回さず IME へ通す（`Engine::on_input` は呼ばない）。Engine が ON のとき F2 などの物理キーを消費するかも写していない。
- **窓ごとの IME**: 窓ごとにスレッドが別かグローバルかは実機でも一様でない。本テストは両方（shared が主）を測っており、結果は大きく違う（405 本と 9 本）。
  実機の事故が shared に合うことは BUG-223 の記述（窓切替の後も IME は ON）から判断した。
- **`user_enabled`**: ハーネスは常に有効。利用者が awase を OFF にしている場合は検査の対象外。
- **字母の絞り込み**: 時間の予算で文字キー・明示 ON・ImmCross を外し、長さを 3（小さい字母のみ 4）にした。長さ 5〜6 の全列挙は行っていない（ランダム探索は長さ 8〜12 を 300 本ずつ）。

## ゼロベース設計のための読み方

- 目標は golden の I1 の違反ゼロ。**まず「awase の操作・窓切替・正しい観測だけ」の最小列**（shared で msime 36 本・小さい字母では F2 と窓切替の 4 本）を消すこと。
  これらは観測の信頼度や外部要因ではなく、「F2 が `desired_open` を書かない」「窓切替が予測を捨てて前窓の `desired_open` に戻す」という belief の更新規則そのものの結果。
- 外部変更・嘘の観測を含む違反は、観測の仕組みが無いと直らない（読めない窓の poll は届かない）。設計が「観測が無いときに何を既定にするか」を決めない限り残る。
- 入力モード軸（`ObservedEisu` が残る）は開閉の belief とは別の食い違いで、10 本（msime shared）。F0 の後 ひらがなに戻ったときの入力モードの追随。
- 擬似窓のハーネスを充実させるときの優先: (1) 窓切替の本物の処理（キャッシュ復元）の写し、(2) 読めない窓の観測（FocusProbe の値・TSF 系）、(3) 周期 poll の事象化。
