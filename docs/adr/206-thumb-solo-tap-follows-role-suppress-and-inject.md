---
id: ADR-206
title: |-
  無変換/変換の単独タップの再設計: IME 側でトグルに割り当てられたキーは「生キー抑止＋belief に従う明示 ON/OFF の注入」、それ以外は Suppress/Passthrough の設定に従う。`*_solo_tap_ime_action` は撤去する
summary: |-
  所有者決定(2026-09-29): 無変換/変換の単独打鍵は Suppress か Passthrough かの設定に従う。ただし素通しになる場合でも、そのキーが IME 側の設定で
  トグルに割り当てられているなら、生キーは抑止し、awase が現在の belief に従って ON/OFF を明示で inject する(belief が ON なら OFF を、OFF なら ON を)。
  従来の `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`(ADR-153 決定1)を、この動作に置き換える。
  事実: belief ON 側(エンジン活性)は ADR-192 決定3b＋ADR-199 決定16 の `forced_open_action`(役割由来)で既にこの動作をする(実装済み)。
  未実装なのは belief OFF 側(エンジン非活性)の役割由来だけで、そこは現状「生キー通過(受動)」。旧 `*_solo_tap_ime_action` は
  belief OFF 側を独自に持ち(ケース2/3改、`explicit_ime_action_target`＋`transport.rs` の M19 例外)、belief ON 側も独自の経路(ケース1)を持つ二重系統になっている。
  本 ADR は「役割(config.toml の bare `keys.ime_*` または IME 設定由来)」を唯一の入力にして二重系統を1本にし、旧設定を読込時に bare `keys.ime_*` 相当へ移して警告する。
status: |-
  起草(2026-09-29)。opus-adversarial-consult 未実施(実施後にここを更新する)。実装・実機検証は未着手。
related_adr:
  - "ADR-092"
  - "ADR-119"
  - "ADR-153"
  - "ADR-182"
  - "ADR-191"
  - "ADR-192"
  - "ADR-199"
  - "ADR-201"
---

# ADR-206: 無変換/変換の単独タップを「役割があれば抑止＋注入、なければ設定に従う」に一本化する

## 背景(事実。コードは 2026-09-29 の origin/develop 先端で確認)

### 1. いま親指(無変換/変換)の単独タップを開閉に使う入力が3つある

| # | 入力 | 由来 | belief ON(エンジン活性)側の経路 | belief OFF(エンジン非活性)側の経路 |
|---|---|---|---|---|
| S1 | bare `keys.ime_on/off/toggle` に無変換/変換 | ユーザーが config.toml に書く(ADR-192 決定3b) | `resolve_pending_thumb_as_single` 優先順位1.5 の `forced_open_action`(KeyUp で解決、チョード優先) | `engine.rs::match_event` の特殊キー照合が Down で即発火(`suppress_ime_combos` はエンジン活性中のみ真) |
| S2 | IME の実キー設定から逆算した役割(GJI の `config1.db` の CUSTOM 表で無変換/変換が全開状態で閉じるトグル) | ADR-199 決定16・T10。`runtime/mod.rs::enrich_thumb_key_role`(親指の非リピート KeyDown ごと) | 同じ `forced_open_action`(S1 が無ければ役割由来。優先 config ＞ 役割) | **無し**(生キーが IME に届く=受動。ADR-199 決定16「エンジン非活性のときは能動にしない」) |
| S3 | `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`(隠し設定) | ADR-153 決定1 | ケース1: `resolve_explicit_ime_action`(100ms のタイマー解決、`ModeKeyConfig` が Passthrough なら発火しない=M13、composing 中は発火しない) | ケース2/3改: `key_pipeline.rs::explicit_ime_action_target`(`PromoteToOn`/`SuppressOnly`)＋`transport.rs::plan` の M19 例外＋KeyUp のステートレス再評価 |

S3 は S1 より優先され(`forced_open_action` は `explicit_ime_action.is_none()` を要求、`nicola_fsm.rs::resolve_pending_thumb_as_single`)、
同じキーに両方あれば S1 は無視されて警告される(`config.rs::validate_thumb_key_in_ime_combos`)。GUI の ADR-192 T3「置き換えを適用」は親指キーのとき S3 を書く
(`awase-settings/src/main.rs::apply_adr192_recommended_replacement`)。

### 2. S3 の belief OFF 側が複雑なのは「@」対策の歴史による(削らずに残すべきものと、S3 と一緒に消えるものを分ける)

- BUG-113/BUG-124: GJI + Windows Terminal(TsfNative)で、**半角(belief OFF)状態で生の `VK_NONCONVERT`/`VK_CONVERT` が GJI に届くと「@」が出る**(GJI 自身の TSF キー横取り)。
  実機 A/B(`docs/experiments.md` エントリ25 Phase3)で「生キーを Suppress し、何も送らなければ『@』は消える」ことが確認済み。
  旧ケース3(belief が変わらなくても毎回強制 actuate)は「単発の IME 制御 SendInput だけで『@』を誘発しうる」ため撤回し、「抑止のみ」(ケース3改)に再設計した。
- BUG-122/123: ケース2(`"on"`/`"toggle"` × belief OFF → awase が ON を書き、生キーは Suppress)は実機確認済みで「@」が出ない。
  ただし actuation 後に FSM が同じ単独タップを Passthrough で再送すると、GJI が「開いた状態の無変換」=かな切替と解釈してカタカナへ飛ぶ(BUG-123)。
  `explicit_ime_action_consumed` マーカーが `resolve_pending_thumb_as_single` の後続(優先順位1.5/2/`ModeKeyConfig`)を打ち切って防ぐ。
- つまり **「生キー抑止＋awase が1回だけ書く」(ケース2)は実機で「@」なしと確認された構成**で、「生キーを通す」「毎回強制で書く」は「@」を誘発した。
  本 ADR の設計は前者の構成を役割由来に広げるものである。

### 3. 役割の取得は既にある(新しい判定点は要らない)

`enrich_thumb_key_role` は親指キーの非リピート・非 injected・無修飾の KeyDown ごとに `engine.thumb_forced_open_actions()` を「config の bare(S1) ＞ 役割由来(S2)」で設定し直す
(`state/key_effect_runtime.rs::thumb_forced_action`)。役割由来は GJI の CUSTOM 表で全開状態がトグルのときだけ `Some(Toggle)`(ADR-199 決定4・11)。
MS-IME 本体は T17 Phase 4 まで受動(`None`)。`table_ime_kind()` が `None`(ATOK・未同定)なら役割は付かない。**この判定は belief OFF でも同じ打鍵で既に走っている**。

### 4. A4 棚卸し(docs/tasks/v2-a4-config-cleanup-inventory-2026-09-29.md)の指摘

S3 は「@」抑止の同等性が S1/S2 経路で未検証のため v2.0 では残す推奨だった。本 ADR は所有者の方針決定(下記)を受け、同等性の検証を CI e2e の追加と実機 A/B に置き、S3 を撤去する。

## 決定

### 決定1(所有者決定・2026-09-29): 単独タップの扱いは「開閉の役割があるか」で分ける

親指キー K(無変換/変換)の単独タップ(同時打鍵と解決されなかった打鍵)について:

1. **K に開閉の役割がある**(S1: config の bare `keys.ime_*`、または S2: IME 設定由来のトグル)→ **生キーを抑止し、awase が belief に従う明示の ON/OFF を1回だけ書く**
   (belief が ON なら OFF、OFF なら ON。`Toggle` を絶対指定 `SetOpen(bool)` に解決して送る)。`ModeKeyConfig`(Suppress/Passthrough)は関与しない。
2. **役割がない**(役割が無い・IME 未同定・MS-IME 本体〈Phase 4 まで〉・ATOK・修飾付き・injected)→ `ModeKeyConfig` の Suppress/Passthrough に従う(現状のまま)。
   Passthrough なら生キーがそのまま IME に届く(受動)。

決定1-1 で「Suppress のときも役割があれば注入する」のは、出荷済みの ADR-199 決定16(役割由来は `ModeKeyConfig` より優先。`forced_open_action` は `ModeKeyConfig` の前)と同じで、
新しい挙動ではない。所有者文面の「単独打鍵は Suppress か Passthrough かの設定に従う」は、決定1-2(役割なし)への言及と解釈する。

### 決定2: 入力は S1 と S2 だけ。S3(`*_solo_tap_ime_action`)を撤去する

- `nicola_fsm.rs` の `muhenkan/henkan_solo_tap_ime_action` フィールド・setter・getter、`ThumbSoloSpecialHandling.explicit_ime_action`、`resolve_explicit_ime_action`(ケース1)を削除する。
  M13(`ModeKeyConfig` が Passthrough なら明示 config が発火しない)は消える(決定1-1 が所有者決定でそれを上書きする)。composing 中の非発火(ケース1)も消え、
  `forced_open_action` の規則(composing 中も発火、ADR-192 決定3b)に揃う。
- `fsm_adapter.rs`・`engine.rs`・`runtime/mod.rs`(`set_*_solo_tap_ime_action`、`ModeKeyConfig` 再構築の `is_some()` 参照)・`bootstrap.rs`・`state/evidence.rs` の関連コメント/引数を削る。
- `config.rs::GeneralConfig` の2フィールドは**読み込み専用の非推奨項目として残す**(`ShadowImeActionConfig` ごと。既存の config.toml をエラーにしない・ADR-201 の診断で
  警告を出すため)。値はエンジンに渡さない(決定4で S1 相当へ移す)。**`KeysConfig` 周辺は触らない**(並行編集中の a4b と衝突させない)。

### 決定3: belief OFF 側(エンジンは有効だが IME が閉じていて非活性)の役割由来は、旧ケース2の経路をそのまま使い、入力だけを差し替える

- `key_pipeline.rs::explicit_ime_action_target` の入力を、旧 `engine.muhenkan/henkan_solo_tap_ime_action()` から
  「`engine.thumb_forced_open_actions()` の該当キーの値のうち、**config の bare(S1)が無いもの**(=役割由来 S2)」に差し替える
  (S1 は belief OFF では `match_event` の特殊キー照合が既に処理する。両方が反応すると1回の押下で2回書く二重 actuation になる)。
- 追加のゲート: **エンジンが有効(`engine.is_user_enabled()`)のときだけ**能動にする。エンジンをユーザーが無効化している間(Ctrl+Shift+無変換等)は
  ADR-199 決定16「エンジン非活性のときは能動にしない」どおり受動(生キーを通す)。旧ケース2にはこのゲートが無く、エンジン無効中に Down だけ Suppress され
  KeyUp が GJI へ漏れうる(`Decision::Consume` に乗らないため)ので、S2 の新設に合わせて閉じる。
- 役割由来は `Toggle` だけなので、belief OFF の結果は常に `PromoteToOn`(ON を書く)。旧 `SuppressOnly`(`"off"`×belief OFF、BUG-124)のアームと
  KeyUp のステートレス再評価は S3 専用の帰結なので撤去する(BUG-124 の回帰ガード `architecture_guard.rs` は「抑止のみ・actuate しない」の
  形が S2 に無いことを固定する内容へ更新。S1 の `"off"` は決定4を参照)。
- 二重注入の防止(BUG-123/BUG-46 型)は既存のマーカー `explicit_ime_action_consumed`(名前は変えない=ガードトークンの churn を避ける)で行う:
  Down で `PromoteToOn` した打鍵は、100ms 後/KeyUp の `resolve_pending_thumb_as_single` で `forced_open_action` も `ModeKeyConfig` の Passthrough も打ち切られる
  (`!explicit_action_consumed` は現行のまま)。物理配送は `transport.rs::plan` の M19 例外(マーカーで Suppress)と、活性化後の `Decision::Consume` の2段。

### 決定4: 旧設定の移行 — 読込時に S1 相当へ移し、1回だけ警告する

- `muhenkan/henkan_solo_tap_ime_action = "on"/"off"/"toggle"` が残っている config は、`SpecialKeyCombos` を組み立てる箇所で該当キーの bare コンボを
  `ime_on`/`ime_off`/`ime_toggle` に**メモリ上でだけ**追加する(config.toml は書き換えない)。警告(ADR-201 の診断経路)は「`keys.ime_*` に bare で書くか、削除してください。
  GJI の CUSTOM 表で無変換/変換がトグルなら設定なしで動きます」。
- 既知の差: 旧 `"off"` × belief OFF は「抑止のみ」(BUG-124)だったが、S1 になると `match_event` の `ImeOff` が belief OFF でも `SetOpen(false)` を発行する。
  この経路が GJI で「@」を誘発するかは**未検証**(旧ケース3が誘発した「強制 actuate」に近い)。検証は決定6の e2e と実機 A/B に置く。
  誘発が確認されたら、移行時に `"off"` だけは「無視して警告」に倒す(受動に戻る)案を代替として持つ。

### 決定5: GUI(ADR-192 T3)と検証(`validate_thumb_key_in_ime_combos`)を書き換える

- `apply_adr192_recommended_replacement` は親指キーでも bare コンボ(`ime_on=["変換"]`・`ime_off=["無変換"]`)を書く(S3 分岐を削除)。`*_always_suppress = true` の同時設定は
  決定1-1 により不要(役割/bare があれば `ModeKeyConfig` に関わらず注入される)なので書かない。snapshot/undo から S3 の項目を外す。プレビュー文も更新する。
- `validate_thumb_key_in_ime_combos` の「`*_solo_tap_ime_action` が優先され、この強制ON/OFFの設定は無視されます」の分岐を削除する。
  旧設定が残っているときの警告は決定4に移す。

### 決定6: 検証計画

- 単体: `src/engine/tests.rs`/`nicola_fsm.rs::tests` の S3 依存テストを、S1(`set_thumb_forced_open_actions`)で同じ意図を検証するテストへ置き換える
  (belief ON で単独タップ→絶対指定の SetOpen が1回、`ModeKeyConfig` が Passthrough でも生キーが出ない、チョードでは発火しない、`explicit_action_consumed` で後続を打ち切る)。
- 統合: `crates/awase-windows/tests/architecture_guard.rs` の必須トークン(`explicit_ime_action_target(` 呼び出し数・M19 マーカー・`fn resolve_explicit_ime_action(` の存在)を更新し、
  「belief OFF 側の入力が bare(S1)を除いた役割由来だけ」「`is_user_enabled()` ゲートがある」を固定する。`transport.rs` の M19 例外は残す(名前とコメントは S2 用に直す)。
  `ime_key_sequence_golden.rs` に、役割由来の Toggle × belief OFF/ON の期待送信列(書く回数=1)を足す。
- CI e2e(`.github/workflows/e2e-ime.yml` の sc-* 構成に追加。`gh workflow run e2e-ime.yml --ref <ブランチ> -f only='sc-solotap-*'`):
  GJI + CUSTOM 表で無変換=トグル/非トグル × 直接入力/かな入力(belief OFF/ON)× `--seq=1D,1D`(無変換の単独タップ2回)の consistency(実 IME の開閉と Engine の追随)、
  二重トグルが起きないこと(1回の押下で開閉が1回)、変換側の対称、`observed` 件数(判定に使った観測の件数)を summary に出す。
  「@」は CI の Windows Terminal では観測できない可能性が高いので、CI では「生キーが GJI に届かない=Down/Up とも Suppress/Consume」(awase.log の `[shadow-toggle]`/物理配送の disposition)で代替し、
  「@」そのものは実機 A/B(Windows Terminal + GJI、半角状態で無変換/変換の単独タップ)で確認する(未検証事項として残す)。
- 回帰の記録: `docs/known-bugs/` には新規 BUG を起こさず、BUG-113/123/124 に本 ADR への追記を1行足す(再発ファミリー「キー選択」「IME actuation 合流点」に触れるため、
  fix-requires-evidence.md の (a) 回帰テスト を主とし、(b) は追記で足りる)。

## リスクと反論(親エージェントの懸念への回答)

1. **二重トグル(1回の押下で開閉が2回)**: 起きうる経路は3つ。(a) 書いた上に生キーも通る → 決定3の M19 例外＋`Decision::Consume` の2段で塞ぐ(実機確認済みのケース2と同じ構成)。
   (b) belief OFF で書いた後、同じ打鍵が KeyUp で `forced_open_action` としてもう一度 `Toggle` を解決する → `explicit_action_consumed` で打ち切る(BUG-123 の修正と同じ機構、
   S2 でも同じマーカーが立つことを単体テストで固定)。(c) 自動リピート Down → `enrich_thumb_key_role` は非リピートの Down だけで値を設定し直し、FSM は PendingThumb 中のリピートを
   新しい打鍵として扱わない(要確認: 下の未検証事項)。
2. **「@」**: 「生キー抑止＋awase が1回だけ書く」は BUG-122/123 の実機確認で「@」が出なかった構成。一方「生キー通過(Passthrough)」は BUG-124 で「@」を出した構成。
   本設計は belief OFF でも役割があれば前者にする(現状 S2 は後者=受動)ので、GJI で無変換をトグルにしたユーザーの半角状態での「@」は**減る**方向。
   ただし役割が方向固定(IME オン/オフだけ)・ATOK・MS-IME 本体の場合は受動のまま(所有者決定の範囲外)で、「@」が残りうる。
3. **belief が古いとき逆方向に書く**: 役割由来の `Toggle` は belief から方向を決めるので、belief が古い(観測できていない、BUG-172)と、受動なら IME が正しく処理していたキーを
   逆方向に書く/何も起きない。緩和: (i) 注入は `SetOpen(bool)` の絶対指定なので、同じ方向の再送は冪等(既に合っていれば no-op)。(ii) `ControlLog.shadow_on` は `Option<bool>`
   のまま扱い、「送信を省略してよい」は陽性の確認済み証拠(`Some(x)`)にのみ基づく(fix-requires-evidence.md の罠。`already_matched` のバイパス条件は既存の
   `apply_ime_open_with_belief` の規則に従い、この変更では増やさない)。(iii) belief の観測強化は別エージェント(BUG-172)の領域で、本 ADR は入力を差し替えるだけで
   belief 更新経路は触らない。**受動より劣る点は残る**(古い belief では誤動作する)ことを許容するのが所有者の選択。
4. **受動化の方針との整合**: ADR-191(IME が真実)・ADR-178 領域A撤去・ADR-199 決定1 の「役割を持つキーだけ能動」の範囲内。能動を増やすのは
   「GJI の CUSTOM 表で無変換/変換がトグルのときの belief OFF 側」だけ(ADR-199 決定16 が belief ON 側で既にやっていることの対称化)。S1(ユーザーが config に書いた)は増えない。
   旧 S3 という別系統を減らす分、能動経路の数は純減(ケース1・ケース2/3改・M19 のうち S3 専用部分を削除)。
5. **NICOLA 同時打鍵(PendingCharThumb・BUG-119)**: 発火点は `resolve_pending_thumb_as_single`(単独タップ確定)だけで、同時打鍵と解決した打鍵は発火しない(チョード優先、変更なし)。
   優先順位は 専用Fnキー ＞ bare/役割(`forced_open_action`) ＞ `ModeKeyConfig`(S3 が消えるので2.が無くなる)。`suppress_solo_output`(ADR-182 決定1b)と押下後 Shift の既存ガードは
   `forced_open_action` 側にあり、そのまま効く。belief OFF 側(エンジン非活性)には同時打鍵がそもそも無い。
6. **IME actuation 合流点**: 新しい書き込みの入口は作らない。belief OFF の書き込みは旧ケース2と同じ `kp_stage_shadow_ime_toggle` → `IntentKind::PhysicalImeKey` →
   既存の共通処理(`eisu_reset_on_ime_on` を含む)に合流し、`ime_controller.rs::apply` 以降は変更しない。`RESTRICTED_CALLS`/tuning 定数は増減なし(複雑性予算に触れない)。

## 実装タスクの分割案

- T1: `nicola_fsm.rs`/`fsm_adapter.rs`/`engine.rs` から S3(フィールド・setter・getter・`resolve_explicit_ime_action`・関連テスト)を削除し、
  `thumb_solo_special_handling` から `explicit_ime_action` を外す。`defers_solo_until_release`・`resolve_pending_thumb_as_single` の分岐を整理する。
- T2: `key_pipeline.rs::explicit_ime_action_target` の入力を役割由来(S1 除外)＋`is_user_enabled()` ゲートに差し替え、`SuppressOnly` アームと KeyUp 早期分岐を削除。`transport.rs` のコメント更新。
- T3: `config.rs`(`GeneralConfig` の2項目を非推奨・読み込み専用に、doc 更新、`validate_thumb_key_in_ime_combos` の分岐削除、移行警告)、`SpecialKeyCombos` 組み立て(`runtime/mod.rs`・`bootstrap.rs`)での S1 相当への移行。
- T4: `awase-settings` の T3 書き換え(決定5)。
- T5: 回帰テスト(決定6)・`architecture_guard` 更新・sc-solotap-* 構成の追加。
- T6: docs(ADR-153/192/199 のステータス追記、BUG-113/123/124 に1行、`docs/adr/index.md` に1行、README/usage の隠し設定の記述があれば削除)。

## 未検証事項(実装後も残るもの)

- 半角状態(belief OFF)で GJI の CUSTOM 表の無変換=トグルを押したときの「@」の有無(実機 A/B、Windows Terminal + GJI)。
- 旧 `"off"` の S1 相当への移行後、belief OFF での `SetOpen(false)` 発行が「@」を誘発するか(決定4)。
- 親指キーの自動リピート Down が belief OFF の `explicit_ime_action_target` を再度通ったときの挙動(現行は `current` が ON になっているので `Inactive`。エンジンの活性化が同じ打鍵で間に合わない環境は未確認)。
- MS-IME 本体・ATOK は対象外(受動のまま)。MS-IME 本体は ADR-199 T17 Phase 4 と B4 計画が決まってから同じ入力(S2)に合流させる。
