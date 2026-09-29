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
  起草(2026-09-29)。opus round1 反映済み(belief OFF 側をエンジンの特殊キー照合へ合流、旧 "off" の抑止のみを保存、移行の既知差を列挙)。round2 待ち。実装未着手。
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

- belief ON(エンジン活性)側は ADR-199 決定16 が出荷済みで、`ModeKeyConfig` より役割が優先される。**新しいのは belief OFF(エンジン非活性)側だけ**で、
  これは ADR-199 の却下案 N(「エンジン非活性〈IME OFF〉でも能動にする」)と決定16 最終段(「エンジン非活性のときは能動にしない」)を、所有者決定(2026-09-29)で**覆す**ものである
  (ユーザーがエンジンを無効化している間は受動を維持する、決定3)。
- 所有者文面の「素通しになる場合でも」は、主に belief OFF の打鍵(エンジン非活性なので `ModeKeyConfig` が参照されず生キーが常に通る)を指すと読む。
  「Suppress 設定 × トグル役割 × belief ON で awase が閉じる」は現状どおり(出荷済み)。**所有者への確認事項**: この読みでよいか。

### 決定2: 入力は S1 と S2 だけ。S3(`*_solo_tap_ime_action`)と、それ専用の belief OFF 機構を撤去する

- 削除: `nicola_fsm.rs` の S3 フィールド・setter・getter・`ThumbSoloSpecialHandling.explicit_ime_action`・`resolve_explicit_ime_action`(ケース1)、
  `fsm_adapter.rs`/`engine.rs`/`runtime/mod.rs`/`bootstrap.rs` の関連配線、`key_pipeline.rs::explicit_ime_action_target`・`ExplicitImeActionOutcome`・
  `kp_stage_shadow_ime_toggle` のケース2/3改の分岐と KeyUp 早期分岐、`transport.rs` の M19 例外、マーカー `explicit_ime_action_consumed`
  (`ImeRelevance`・`PendingThumbData` のフィールドとそれを渡す `resolve_pending_thumb_as_single` の引数、`input_tracker.rs`・`fsm_types.rs` の伝搬)、
  `architecture_guard.rs` の関連ガード(内容を新構造の不変条件に書き換え)。
  マーカーが要らなくなる理由は決定3: belief OFF の打鍵はエンジンの Phase 1(特殊キー照合)で完結し、FSM に PendingThumb を作らないので二重解決が構造的に起きない。
  `kp_latch_keyup_to_keydown_disposition` の無変換/変換の除外(`key_pipeline.rs:303-324`)は、Down/Up を揃える所有者が「Consume の義務(`UpDuty`)」に変わるので、理由コメントを書き換える。
- M13(`ModeKeyConfig` が Passthrough なら明示 config が発火しない)と、ケース1の composing 中の非発火は消える(決定1 が上書き。`forced_open_action` の規則=composing 中も発火、ADR-192 決定3b に揃う)。
- `config.rs::GeneralConfig` の2フィールドは**読み込み専用の非推奨項目として残す**(既存 config.toml をエラーにしない・ADR-201 の診断で警告を出すため)。**`KeysConfig` 周辺は触らない**。

### 決定3: belief OFF 側の役割由来は、旧ケース2の Windows パイプライン経路ではなく**エンジンの特殊キー照合(S1 と同じ入口)**に合流させる(round1 の代案7を採用)

旧案(ケース2の入力差し替え)は棄却した。理由(round1 指摘 1-A/1-B、コードで裏取り済み):
`kp_stage_shadow_ime_toggle` の OFF→ON は belief を書くだけで、実 IME へ ON を書くのはエンジンの活性化遷移(`transition_activation`)任せである。
`compute_state` が `NotRomajiInput`(GJI の半角英数 conv を保持したまま閉じていた場合等)や `UserDisabled` を返す打鍵では遷移が起きず SetOpen が出ない。
その一方で生キーは M19 で抑止されるため「抑止したのに何も書かない」空振り(ADR-119 型)になり、現状の受動より退行する。KeyUp の Suppress も `Decision::Consume` に乗った場合だけで、非対称(BUG-131/132 型)が残る。

新案: `Engine::match_special_keys`(`engine.rs:878`)に、`match_event` が `None` のときの分岐を1つ足す。

- 条件: `!engine_active` かつ `ctx.is_japanese_ime` かつ `adapter.is_enabled()`(ユーザー無効中は受動)かつ `is_bare_thumb(event, ctx.modifiers)` かつ
  そのキーの `forced_open_action` が `Some`(config の bare は `match_event` が先に一致するので、ここに来るのは役割由来だけ)かつ専用 Fn キー(`muhenkan_solo_tap_dedicated_fn_key`)が無い。
- 動作: `Toggle` なら `SpecialKeyMatch::ImeToggle`(`!ctx.ime_on` へ絶対指定の `SetOpen`)。`ime_set_open_effects` は状態遷移が無い場合も `SetOpen` を明示的に積む(`engine.rs:848-855`)ので、
  `NotRomajiInput` でも書かれる。生キーは `Decision::consumed_with` で抑止され、KeyUp は `UpDuty::Consume`(`on_input`)で Down と対になる。
  以降の書き込みは S1 と同じ経路(`handle_engine_set_open` → `dispatch_ime_set_open`)なので、**新しい書き込みの入口は増えない**。
- 親指の bare 照合(S1・S2 共通)に対する2つの追加:
  (a) **自動リピートの Down(`was_down`)** では `SetOpen` を積まず、Consume だけ返す。現状の S1 は `matches_key_combo` が `was_down` を見ず、エンジン非活性で押し続けるとリピートのたびに反転する(既存の穴。round1 1-C)。
  (b) **`ImeOff` × `!ctx.ime_on` × bare 親指は Consume するだけで `SetOpen` を積まない**(BUG-124 の「抑止のみ・強制 actuate しない」の等価物。`applied` が Unknown のとき
  `already_matched` を通らず `VK_IME_OFF` を単発で送る形が旧ケース3=「@」の構成だった〈round1 3-A〉)。これは旧 S3 の `"off"`×belief OFF の挙動と一致し、GUI T3 が書く主流設定
  (無変換=`"off"`)を守る。belief が古く実際は ON のときは、OFF 方向は「何も起きない」になる(受動より劣る。リスク3)。
- 削れるもの: 上のとおり決定2 の Windows 側機構一式。エンジン側の追加は分岐1つ+(a)(b)で数十行、Windows 側は数十行の純減。

### 決定4: 旧設定の移行 — 読込時にメモリ上で S1 相当へ移し、警告する

- `muhenkan/henkan_solo_tap_ime_action = "on"/"off"/"toggle"` が残っている config は、`SpecialKeyCombos` を組み立てる箇所(`runtime/mod.rs` の reload 経路・`bootstrap.rs`)で、
  該当キーの bare コンボを `ime_on`/`ime_off`/`ime_toggle` に**メモリ上でだけ**追加する(config.toml は書き換えない)。同じキーに既存の bare があれば**旧 S3 が勝つ**(旧実装の優先順位を保つため、
  既存の同キー bare を除いてから追加する)。GUI が実際に書く形(無変換=`"off"`)は決定3(b)で保護される。
- 警告(ADR-201 の診断経路、`validate_thumb_key_in_ime_combos` の該当分岐を置換): 「`*_solo_tap_ime_action` は非推奨です。`keys.ime_on/off/toggle` に bare で書くか、削除してください。
  GJI の CUSTOM 表で無変換/変換がトグルなら設定なしで動きます」。
- 移行で変わる差(既知・許容。所有者決定が M13 を上書きする):
  1. M13: 旧 `"toggle"`/`"on"` × `ModeKeyConfig`=Passthrough は「belief ON 中は GJI 自身のかな切替、OFF→ON の復帰だけ config」だった(`nicola_fsm.rs:2070-2076`)。S1 では belief ON でも `forced_open_action` が優先されるので、この使い分けは実現できなくなる。
  2. composing 中: 旧ケース1は composing 中は発火しなかった。S1 は発火する(IME 側もそのキーで閉じる設定であることが前提。旧 S3 ユーザーの IME 設定がトグルとは限らない点に注意)。
  3. エンジン無効中: 旧ケース2/3改は動いたが、旧ケース1(belief ON)は FSM に届かず生キーが通っていた。S1 の `match_event` は `engine_enabled` を見ないので、無効中も belief OFF/ON とも能動になる(S2 は決定3のゲートで受動)。
  4. 旧 `"off"` × belief ON の挙動は同じ(閉じる)。旧 `"on"`/`"toggle"` × belief OFF は「PromoteToOn」から S1 の `ImeOn`/`ImeToggle` に代わり、意図の記録が `PhysicalImeKey` から `Command` になる(`IntentStore` の扱いは S1 の既存挙動)。

### 決定5: GUI(ADR-192 T3)と検証(`validate_thumb_key_in_ime_combos`)を書き換える

- `apply_adr192_recommended_replacement` は、親指キーのとき `keys.ime_on`/`ime_off` に該当 bare 要素(`"変換"`/`"無変換"`)を**追記**する(既に含まれていれば何もしない。リストを丸ごと置き換えると
  既定の `Ctrl+無変換` 等が消える〈round1 3-D〉)。`*_always_suppress = true` の同時設定は決定1 により不要なので書かない。非親指キーの既存の置き換え動作は変えない。
  snapshot/undo から S3 の項目を外し、プレビュー文と単体テスト(`main.rs:7757-7798`)を更新する。
- `validate_thumb_key_in_ime_combos` の「`*_solo_tap_ime_action` が優先され…無視されます」の分岐を削除し、旧設定の警告は決定4に移す。
- 決定5 は決定3(b)(抑止のみ分岐)が入るまで単独で入れない(GUI が書く設定が BUG-124 の構成になるため)。同じ PR で扱う。

### 決定6: 検証計画

- 単体(Linux で走る): `src/engine/tests.rs` に、エンジン非活性(IME OFF/`NotRomajiInput`)× 親指の役割由来 Toggle で「Consume＋絶対指定 `SetOpen(true)` が1つ、KeyUp も Consume」、
  `ImeOff`×belief OFF で「Consume だけ・`SetOpen` なし」、リピート Down で「Consume だけ」、ユーザー無効・専用 Fn キー設定・`is_japanese_ime=false` で「素通し」、
  belief ON(エンジン活性)は FSM の `forced_open_action`(KeyUp 解決)で従来どおり、を固定する。S3 依存の既存テストは S1(`set_thumb_forced_open_actions`)ベースへ置き換える。
  `config.rs` に、旧設定→S1 相当の移行(同キー既存 bare より旧 S3 が勝つ)と警告のテストを足す。
- 統合: `architecture_guard.rs` を更新(旧ケース2/3改とマーカーの再導入禁止、`match_special_keys` の新分岐が `is_user_enabled` ゲートと `was_down` 分岐を持つこと、`SetOpen` 直積みをしないこと)。
  `ime_key_sequence_golden.rs` は `ImeController` の戦略選択と与えた view への送信列の検証であり、「1回の押下で何回書くか」は表現できない(`runtime/` は `#[cfg(windows)]` で Linux では走らない)ので対象にしない。
  `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings --tests` でコンパイルを確認する。
- CI e2e(`e2e-ime.yml` に `sc-solotap-*` を追加。`gh workflow run e2e-ime.yml --ref <ブランチ> -f only='sc-solotap-*'`): GJI + CUSTOM 表で無変換=トグル/非トグル × 直接入力/かな入力 × `--seq=1D,1D`、
  変換側の対称、および **round1 5 のシナリオ「素通しの英数キーで閉じた直後に無変換」**(`applied` が古いと `already_matched` で書き込みが省略され、抑止済みの生キーとあわせて誰も開かない BUG-156 型)。
  判定は consistency(実 IME の開閉と Engine の追随)と、1回の押下での開閉回数=1、`[shadow-toggle]`/物理配送のログ、`observed` 件数。「@」は CI では出ない可能性が高いので、
  CI では「生キーが GJI に届かない」で代替し、「@」そのものは実機 A/B(Windows Terminal + GJI、半角状態で無変換/変換の単独タップ、旧 `"off"` 設定も)で確認する(未検証事項)。
- 記録: `docs/known-bugs/` には新規 BUG を起こさず、BUG-113/123/124 に本 ADR への追記を1行足す。ADR-153・192・199 のステータスに本 ADR による置換を追記する。

## リスクと反論(親エージェントの懸念への回答)

1. **二重トグル/二重信号**: belief OFF はエンジンの Phase 1 で「Consume＋SetOpen 1つ」で完結し、FSM に PendingThumb が作られない(BUG-123 型の二重解決が起きない)。KeyUp は `UpDuty::Consume` で対になる。
   リピートは決定3(a)。belief ON は FSM の KeyUp 解決(出荷済み、チョード優先)。S1 と S2 は同時に発火しない(`match_event` が先に一致すれば決定3の分岐は評価されない)。
2. **「@」**: 「生キー抑止＋awase が1回だけ書く」は BUG-122/123 の実機確認で「@」なしだった構成(ただし旧ケース2は Windows パイプライン経由の書き込みで、S1 と同じエンジン経由の書き込みが同じ結果になるかは**未検証**)。
   OFF 方向 × belief OFF は抑止のみ(BUG-124)。方向固定の役割・ATOK・MS-IME 本体は受動のまま(所有者決定の範囲外)。
3. **belief が古いとき**: 絶対指定なので同方向の再送は冪等だが、(i) belief が実際と逆のとき、受動なら IME が 1 回で正しく処理していた打鍵が「何も起きない」(1打鍵消える)になる。これは所有者が受け入れるコスト(BUG-172 が窓を作る)。
   (ii) round1 5: `dispatch_ime_set_open` は `applied_snapshot` の陽性証拠(`Some(x)`)で送信を省くが、`applied` は `ObserverReported`/`ModeKeyPassedThrough` ではリセットされない。素通しの別キーで閉じた直後の S2 で、
   抑止済みの生キーと合わせて誰も開かない(BUG-156 型)可能性がある。**S1 と `ActivationSync` が既に同じ経路を通る既存の性質**で、本 ADR は S2 をその経路に載せる。緩和は `already_matched` を無効にする(=強制 actuate、「@」の危険)
   のではなく、CI シナリオで実測して結果次第で別 ADR で扱う。`ControlLog.shadow_on` は `Option<bool>` のまま扱う(`bool` に潰さない)。
4. **受動化の方針**: ADR-191・ADR-178 領域A撤去・ADR-199 決定1 の「役割を持つキーだけ能動」の範囲内。能動を増やすのは GJI の CUSTOM 表で無変換/変換がトグルのときの belief OFF 側だけ(決定1 の但し書きのとおり却下案 N を覆す)。
   旧 S3 の専用機構(ケース1・ケース2/3改・M19・マーカー)を削るので、能動経路の数は純減。
5. **NICOLA 同時打鍵(PendingCharThumb・BUG-119)**: belief ON の発火点は `resolve_pending_thumb_as_single`(単独タップ確定)だけで、チョードでは発火しない(変更なし)。優先順位は 専用Fnキー ＞ `forced_open_action`(bare ＞ 役割)＞ `ModeKeyConfig`
   (S3 が消えるので旧2.が無くなる)。`suppress_solo_output`(ADR-182 決定1b)・押下後 Shift のガードはそのまま。belief OFF はエンジン非活性でチョード判定自体が無い。
6. **IME actuation 合流点**: 新しい入口は作らない(エンジンの `SetOpen` → 既存の `dispatch_ime_set_open`)。旧ケース2 が `IntentKind::PhysicalImeKey` で合流していた分は減る。`RESTRICTED_CALLS`/tuning 定数は増減なし。
   確認事項(実装時): (a) `kp_reopen_gji_fsm(ReopenSource::ShadowToggle)`(ADR-203)がエンジン経由の `SetOpen(true)` でも走るか(走らなければ S1 も同様で、別件)、(b) eisu 救済(BUG-159)は Decision 経由の `PostSetOpenEisuReset` が担う。

## 実装タスクの分割案

- T1: エンジン: `match_special_keys` の新分岐・(a) リピート・(b) OFF×belief OFF の抑止のみ・専用 Fn ゲート、単体テスト。
- T2: S3 とマーカーと Windows 側ケース2/3改・M19 の撤去(決定2)、`architecture_guard` の更新、`kp_latch_keyup_to_keydown_disposition` のコメント更新。
- T3: `config.rs` の非推奨化・旧設定→S1 の移行(決定4)・警告・`validate_thumb_key_in_ime_combos` の整理。
- T4: `awase-settings` の T3 書き換え(決定5)。
- T5: `sc-solotap-*` の追加(決定6)。
- T6: docs(ADR-153/192/199 のステータス追記、BUG-113/123/124 に1行、README/usage の隠し設定の記述があれば削除)。

## 未検証事項(実装後も残るもの)

- 半角状態(belief OFF)で GJI の CUSTOM 表の無変換=トグルを押したときの「@」の有無(実機 A/B、Windows Terminal + GJI)。旧ケース2 と、エンジン経由の書き込み(S1 と同じ)で結果が同じかも含む。
- 旧 `"off"`(GUI T3 の設定)の実機挙動が旧実装と同じか(半角状態で無変換/変換の単独タップ→「@」が出ない・生キーが GJI に届かない)。
- `applied_snapshot` が古いときの S2(リスク3(ii))の CI 実測。
- MS-IME 本体・ATOK は対象外(受動のまま)。MS-IME 本体は ADR-199 T17 Phase 4 と B4 計画が決まってから同じ入力(S2)に合流させる。
