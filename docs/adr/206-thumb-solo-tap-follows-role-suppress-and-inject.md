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
  実装済み(PR #376 ほか、v2.0.0 に含まれる)。実機確認は X1 を 2026-09-30 に実施: 既定(Suppress)で IME OFF の無変換は awase が生キーを再注入して『@』が出る一方向の結果が出た(決定3〔iii〕に進むかは同日時点で所有者判断待ち、その後の決定は本ファイルから確認できず要確認)。Passthrough の役割由来は実機の GJI 設定が対象外で実機未確認。
  旧(2026-10-04 更新前):
  起草(2026-09-29)。opus round1〜3 反映済み(エンジン側合流、リピート印、eisu 保持、削除一覧、3(b)撤回、非固着の条件、ADR-205 相互参照)。opus round1〜5 で収束(round5=2026-09-29 夕の仕様訂正後。記述3点を反映済み)(round4: 設計変更不要、前提 P3/P4 等の記述3点を反映済み)。固着の定義(所有者)反映済み。コア・Windows・GUI・テスト・CI シナリオを実装済み(PR)。実機 A/B(「@」)と CI e2e は未検証。
related_adr:
  - "ADR-092"
  - "ADR-119"
  - "ADR-153"
  - "ADR-182"
  - "ADR-191"
  - "ADR-192"
  - "ADR-199"
  - "ADR-201"
  - "ADR-205"
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

## 所有者方針(2026-09-29 追加)と受け入れ基準 — ADR-205 と共通

**方針**: awase は能動書き込み(自発的な開け直し等)を一切しない。ユーザーがモードキー(Ctrl+変換/半角全角/漢字/かな/無変換・変換の単独タップ等)を押したときだけ、awase が belief に従って正しいキーを送り、モードずれを解消する。
**受け入れ基準: 固着する不具合を絶対に起こさない。「固着」の定義(所有者、2026-09-29)= 何度モードキーを押しても状態が変わらないこと。** belief が古くて1回目が逆方向に効き、2回押せば期待した状態になるのは**許容**(問題なし。そのために設計を複雑化しない)。
したがって基準は「押すたびに状態が変化する、または絶対指定で確実に収束する」。検証シナリオでは「2回目で期待状態になる」を PASS、「N回押しても変化しない」を FAIL とする。
前提の相互参照: [ADR-205](205-observe-external-ime-close-in-imm32-unavailable-windows.md)(外部 close の追随と `applied` の実状態への訂正〈D6〉・Blind 窓での `applied` の押下ごとの Unknown 化〈D7〉)。本 ADR の単独タップ解決の書き込みは、ADR-205 が「絶対指定キー」の保証を置く経路
(`handle_engine_set_open`)そのものなので、belief/`applied` の鮮度の前提(下記「非固着の条件」)を ADR-205 と共有する。

## 決定

### 決定1(所有者決定・2026-09-29、同日訂正): 単独タップの Suppress は IME を動かさない。Passthrough のときだけ、IME 側でトグルの親指を awase が代わりに書く

「単独タップの Suppress / Passthrough」は親指キー(無変換/変換)の単独タップの設定(`ModeKeyConfig`)である。親指キー K の単独タップ(同時打鍵と解決されなかった打鍵)について:

1. **Suppress**: **IME を動かさない**。生キーを飲み込むだけで、awase は開閉しない(IME 設定でトグルでも同じ)。
2. **Passthrough**: そのキーが IME 側の設定でトグルに割り当てられている(S2: GJI の CUSTOM 表で全開状態で閉じるトグル)なら、**生キーを抑止し、awase が belief に従う絶対指定の ON/OFF を1回だけ書く**
   (belief が ON なら OFF、OFF なら ON)。トグルでなければ(役割が無い・IME 未同定・MS-IME 本体〈Phase 4 まで〉・ATOK・修飾付き・injected)生キーを素通しする。
3. **bare `keys.ime_on/off/toggle`(S1、ユーザーが awase 側に明示した設定)**: 従来どおり、単独タップの設定に関係なく発火する(明示された awase 自身の設定)。意図に反しうる組み合わせは下の「訂正に伴う整理」に列挙する。

実装上は、S1 を `forced_open_action`(設定に関係なく)、S2 を別入力 `role_open_action`(`ModeKeyConfig` が単独タップの `composing` の値で Passthrough のときだけ)として FSM に渡す。
`enrich_thumb_key_role` は S2 だけを `set_thumb_role_open_actions` に設定する(bare がある側は役割を引かない)。この訂正前は S1 と S2 を `forced_open_action` 1つに合成していた(役割があれば設定に関係なく発火)ため、Suppress でも書いていた。

- エンジン活性側(FSM の KeyUp 解決)の S2 は、この訂正で ADR-199 決定16(`ModeKeyConfig` より役割が優先)を**上書き**する(既定が Suppress なので、GJI で無変換をトグルにしただけのユーザーは何もしなくなる。Passthrough にしたときだけ awase が閉じる)。
- **エンジン非活性側**(IME OFF 中、および開いていても英数等で `NotRomajiInput` のとき)の S2 も Passthrough のときだけ書く。これは ADR-199 の却下案 N を、Passthrough に限って所有者決定で覆す。ユーザーがエンジンを無効化している間は受動を維持する。
- **Suppress × エンジン非活性で生キーが IME にそのまま届く従来動作**は、仕様(Suppress は IME を動かさず飲み込むだけ)と食い違う。エンジンが非活性だと FSM に届かず、`ModeKeyConfig` が参照されないため、
  生キーは常に通る(GJI 自身が設定どおり開閉する=受動)。選択肢は次の3つだった。**所有者決定(2026-09-29): (α) 現状維持。「Suppress というのはそういう仕様。IME OFF だとそのまま届け、IME ON だと飲み込まれる一方向になる」。警告は出さない。(β)(γ) は採らない。**
  (α) 現状維持: 非活性側の Suppress は受動(IME が自分で処理)。既定が Suppress の全ユーザーで挙動が変わらない。「飲み込む」は活性側の単独タップにだけ効く、と読む。
  (β) 非活性側でも Suppress の親指単独押下を飲み込む(Down/Up とも Consume、リピート含む)。仕様には忠実だが、IME OFF 中に無変換で IME を開くという既存の使い方(GJI の既定プリセットで無変換=直接入力等)を全ユーザーで塞ぐ。
  (γ) 役割(S2)があるキーだけ、非活性側でも Suppress なら飲み込む。トグルを Suppress に設定した意図(IME を動かさない)に忠実で、影響が S2 を持つユーザーに限られるが、非活性側に「飲み込むだけ」の入口が増える。
  採用は (α)。**所有者が仕様として受け入れた影響**:
  - (α) 既定(Suppress)のユーザーで GJI の CUSTOM 表の無変換がトグルの場合、無変換は**一方向のキー**になる: エンジン非活性(IME OFF)では生キーが GJI に届いて**開く**が、エンジン活性(IME ON)では単独タップの Suppress で飲み込まれて**閉じない**。
    所有者の固着の定義(何度押しても変わらない)に文面上は当てはまり、ADR-199 決定16(出荷済み。Suppress でも閉じていた)からの**退行**でもある。Suppress をユーザー自身が選んだ場合の「IME を動かさない」としては筋が通るが、Suppress は既定値。
    一方向になっていることをユーザーに知らせる手段(状態依存キー警告、または設定 GUI の「無変換は IME 側でトグルですが、単独タップが Suppress のため awase は閉じません」)は**採らない**(所有者決定 2026-09-29: 警告は出さない)。
  - (β) 両方向とも動かない(一貫はするが、IME OFF 中に無変換で開くという既存の使い方を全員から奪う)。
  - (γ) 役割を持つ既定ユーザーでは両方向とも動かない(トグルを Suppress にした意図には忠実。影響は S2 のユーザーだけ)。
  - 旧来の `always_suppress = false`(idle は Passthrough、composing は Suppress)のユーザーでは、S2 は入力中(composing)には発火しなくなる。ADR-199 決定16 の「composing 中も発火」も上書きする。

### 決定2: 入力は S1 と S2 だけ。S3(`*_solo_tap_ime_action`)と、それ専用の非活性側機構を撤去する

削除する(round2 で grep 全列挙して突き合わせた一覧):
- `nicola_fsm.rs`: S3 フィールド・setter・getter・`ThumbSoloSpecialHandling.explicit_ime_action`・`resolve_explicit_ime_action`(ケース1)。`resolve_pending_thumb_as_single`/`defers_solo_until_release` の S3 分岐と `explicit_action_consumed` 引数、
  `PendingThumbData.explicit_ime_action_consumed` とその全 flush 経路の受け渡し、`input_tracker.rs`・`fsm_types.rs` の伝搬、テストの構築子(`nicola_fsm.rs` のテスト、`confirm_policy.rs`、`fsm_types.rs`)。
- `fsm_adapter.rs`/`engine.rs`/`runtime/mod.rs`/`bootstrap.rs` の S3 配線。`runtime/mod.rs::set_passthrough_thumb_mode_keys` と `state_dependent_key_warning.rs::passthrough_thumb_vks` は
  S3 を「awase が単独タップを消費する=素通しでない」判定に使っているので、S3 を消すだけでは S1/S2 の Passthrough キーで**不要な状態依存キー警告**が出る。判定を `bare_ime_action` に置き換える。IME 設定由来の役割は打鍵ごとにしか求まらず静的に判定できないので含めない(実装 `set_passthrough_thumb_mode_keys` のとおり。Passthrough かつトグル役割の無変換には、awase が消費するのに状態依存キー警告が出うる=案内であり動作には影響しない。2026-09-29 レビュー指摘2で ADR を実装に揃えた)。
- `key_pipeline.rs`: `explicit_ime_action_target`・`ExplicitImeActionOutcome`・`kp_stage_shadow_ime_toggle` のケース2/3改の分岐と KeyUp 早期分岐。`kp_latch_keyup_to_keydown_disposition` の無変換/変換の除外は、Down/Up を揃える所有者が「Consume の義務(`UpDuty`)」に変わるので理由コメントだけ書き換える(除外は残す)。
- マーカー `explicit_ime_action_consumed`: `ImeRelevance`(`hook.rs:291-296`、`platform_state.rs:2536` の構築子を含む)、`state/evidence.rs::IntentWitness::from_physical` の受理条件とテスト
  `explicit_config_consumed_marker_alone_is_a_valid_physical_witness`。
- `transport.rs::thumb_or_role_fkey_disposition`: **「マーカーなら Suppress」の条件だけ**を消し、無変換/変換の VK 分岐そのものは Allow を返す形で残す(BUG-115/ADR-141、C2 対策。分岐ごと消すと将来 `shadow_action` が付いたとき ImmCross で Suppress され二重の空振りになる)。コメントを更新する。
- `eisu_recovery.rs` の module doc の SSOT 表と `architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset` の needle(shadow toggle 経由の明示 config の行)。
  `architecture_guard.rs::forced_thumb_path_preserves_inactive_orphan_keyup_suppression`(`SuppressOnly` とマーカー代入を**必須**にしている)は削除し、置き換え先を「エンジン側の分岐(決定3)が Consume＋絶対指定 `SetOpen` を返し、`SetOpen` を直接積まず `ime_set_open_effects` を経由する」ことを固定するガードにする。
  `explicit_ime_action_case1_keeps_m13_but_case2_3_does_not` など関連ガード(`:3878-3985`)も削除/書き換え。
- docs: `.claude/rules/fix-requires-evidence.md` のキー選択の行と `.githooks/pre-push` の正規表現から `*_solo_tap_ime_action` を外す。
- M13(`ModeKeyConfig` が Passthrough なら明示 config が発火しない)と、ケース1の composing 中の非発火は消える(決定1 が上書き)。
- `config.rs::GeneralConfig` の2フィールドは**読み込み専用の非推奨項目として残す**。**`KeysConfig` 周辺は触らない**。

### 決定3: エンジン非活性側の役割由来は、旧ケース2 の Windows パイプライン経路ではなく**エンジンの特殊キー照合(S1 と同じ入口)**に合流させる

旧案(ケース2の入力差し替え)を棄却した理由(round1 1-A/1-B): `kp_stage_shadow_ime_toggle` の OFF→ON は belief を書くだけで、実 IME へ ON を書くのはエンジンの活性化遷移任せ。`NotRomajiInput`/`UserDisabled` の打鍵では遷移が起きず SetOpen が出ない一方、
生キーは抑止されるので「抑止したのに何も書かない」空振りになる(現状の受動より退行)。

新案: `Engine::match_special_keys`(`engine.rs:878`)の `match_event` が `None` のときの分岐を1つ足す。

- 条件: `!engine_active` かつ `ctx.is_japanese_ime` かつ `adapter.is_enabled()` かつ `is_bare_thumb(event, ctx.modifiers)` かつ `event.ime_relevance.sync_direction.is_none()`(`keys.ime_detect` との二重処理の防止。
  `match_event` と同じガード)かつ、そのキーの `forced_open_action` が `Some` かつ専用 Fn キー(`muhenkan_solo_tap_dedicated_fn_key`)が無い。config の bare(S1)は `match_event` が先に一致するので、ここに来るのは役割由来だけ(`sync_direction` が無い場合)。
- 動作: `Toggle` なら `SpecialKeyMatch::ImeToggle`(`!ctx.ime_on` への絶対指定 `SetOpen`)。`ime_set_open_effects` は状態遷移が無い場合も `SetOpen` を明示的に積む(`engine.rs:848-855`)ので `NotRomajiInput` でも書かれる。
  生キーは `Decision::consumed_with` で抑止され、KeyUp は `UpDuty::Consume`(`on_input`)で Down と対になる(round2 で Phase1〜KeyUp の噛み合わせを確認済み)。書き込みは S1 と同じ経路(`handle_engine_set_open` → `dispatch_ime_set_open`)で、
  **新しい書き込みの入口は増えない**。意図の記録は `PhysicalImeKey` から `Command`(`record_explicit_intent`)になる(S1 の既存挙動と同じ)。
- **リピートは指令を作らない(不変条件、round2 1-1・round3 2-2)**: 最初の Down でエンジンが活性化するので、リピートの Down は `!engine_active` 分岐に来ず FSM に新しい PendingThumb として入り、離した時に `forced_open_action` の Toggle がもう一度発火する
  (押して開き、離して閉じる二重トグル。旧マーカーが守っていた経路。既存の S1 も同じ穴)。二重に守る: (i) 新分岐の条件に **`!event.was_down`** を入れ、`was_down` の Down は `Decision::consumed()` だけを返して `SetOpen` を積まない。
  (ii) Phase 1(`check_special_keys`)で親指の Down を Consume したとき、その vk を Engine の小さな印(`phase1_held: Option<VkCode>`)に記録し、同じ vk の `was_down` Down は Phase 1 の前で `Decision::consumed()` を返して FSM に渡さない
  (「活性化の後に FSM へ新しい PendingThumb として入る」側を塞ぐ)。印は同じ vk の KeyUp、同じ vk の非リピート Down(置き直し)で更新し、`flush`・フォーカス変更で消す。flush で消えた後のリピートは (i) が指令を作らせない。
  KeyUp を取りこぼして印が残った場合(hook の張り直し、issue #165)は、フック側の `was_down` も同じ理由で古くなるので、次の本物の押下が1回食われて KeyUp で印が消える(失われるのは押下1回で、永続しない)。
- **旧案の (b)「`ImeOff × !ctx.ime_on` × bare 親指は Consume するだけ」は採用しない**(受け入れ基準による撤回)。belief OFF/実 ON のとき OFF キーを食うだけにすると、何度押しても閉じられない固着になる(所有者定義の固着そのもの)。
  代わりに OFF 方向は常に絶対指定の `SetOpen(false)` を書く(現行の S1 と同じ)。ADR-205 D7 が Blind 窓で押下ごとに `applied` を Unknown に落とすなら、`applied` が Unknown の間は毎押下 `VK_IME_OFF` が送られる。
  **残る代償**: belief OFF で OFF キー(旧 `"off"` からの移行、GUI T3 が書く `ime_off=[無変換]`)を押すたびに `VK_IME_OFF` を単発で送る形になり、GJI + Windows Terminal では「@」を誘発する可能性がある
  (BUG-124 の旧ケース3=毎回強制 actuate と同じ構成。**未検証。実機 A/B を develop マージの条件にする**。CI では観測できない)。
  「@」が確認された場合の**所有者判断の選択肢**: (iii) 間に他のキーを挟まず同じ OFF キーを2回続けて押したときだけ送る(1回目は抑止のみ。固着せず最悪 2 回で閉じる。「@」は半角での2連打時だけ。時間定数は使わない)。
  抑止のみ(旧 (b))は受け入れ基準に違反するので選択肢にならない。ADR-205 D7 の「Imm32Unavailable かつ非 TsfNative では `VK_IME_OFF` を1回送る」との整合は、本 ADR が(b)を採らないので常に送る側に含まれる。TsfNative だけ (iii) にする、が最小の調整になる。
- **InputRelay の窓(round3 5-1、受け入れ基準への反例の遮断)**: `enrich_thumb_key_role` はプロファイルを見ずに役割を付ける。InputRelay(RDP/VM/PowerToys MWB、ADR-119)ではエンジン非活性で新分岐が生キーを Consume する一方、
  `dispatch_ime_set_open` のゲートは `NotOwned` を返して何も送らず、リモート側の IME に何も届かない(ADR-119/issue #136 の「二重の空振り」)。**`enrich_thumb_key_role` は `current_app_profile() == InputRelay` のとき役割を付けない**(config の bare だけにする)。
  architecture_guard に「`enrich_thumb_key_role` が InputRelay を見ている」を入れる。この修正は S2 のエンジン活性側(出荷済み、ADR-199 決定16)も同時に直す(`thumb_solo_special_handling` は同じ `forced_open_action` を読む)。残るのは S1 の bare だけ(既知差として記録。根治は「この打鍵は actuation を所有しない窓」をエンジンに伝える経路で、別件)。
- **ActivationSync という例外(方針との関係、round3 5-2)**: フォーカス settle 中の押下では `strip_ime_set_open_if_settling` が `SetOpen` だけ剥がし `prev_activation` が進んだままになる。次の**文字キー**で `check_active_transition` が
  `SetOpen(false, ActivationSync)` を出し、押しっぱなしの親指の KeyUp が再注入される。ユーザーがモードキーを押していないのに awase が書く点で、所有者方針(自発的に書かない)の例外である。S1 も同じ既存の性質。
  根治(剥がすときに `prev_activation` を戻す、またはエンジンに通知する)は別 ADR の候補として残し、本 ADR の範囲外とする。窓は狭く、固着ではない(次の押下で状態が変わる)。

### 決定4: 旧設定の移行 — 親指キーのときだけ、読込時にメモリ上で S1 相当へ移し、警告する

- `muhenkan/henkan_solo_tap_ime_action = "on"/"off"/"toggle"` が残っていて、そのキーが**親指キーに割り当てられている**config は、`SpecialKeyCombos` を組み立てる箇所(`app/mod.rs:845` の reload 経路と `app/bootstrap.rs:1214` の起動経路。`runtime/mod.rs::apply_config_update` は組み立て済みの値を受け取るだけで、冒頭で `thumb_forced_open_actions(&special_keys)` を求めるので、**その前=組み立ての時点**で足す)で
  該当キーの bare コンボを `ime_on`/`ime_off`/`ime_toggle` に**メモリ上でだけ**追加する(config.toml は書き換えない)。同じキーに既存の bare が**あれば旧設定は移行しない**(ユーザーが明示した `keys.ime_*` を優先し、黙って上書きしない。2026-09-29 コードレビュー指摘1で「旧 S3 が勝つ」から変更。警告は「無視されます」)。GUI の置き換えは旧設定を `None` に戻す(undo で復元)。
- 親指キーでない無変換/変換の旧設定は移行せず警告して読み捨てる(旧 S3 のエンジン非活性側は親指かどうかを見ていなかったが、S1 に移すと非親指キーは `suppress_ime_combos` の対象外で**エンジン活性中も毎回 awase が書く**ようになるため、意味が広がりすぎる。受動〈IME が自分で処理〉に戻る)。読み捨てた非親指の旧 `"off"` は旧実装では抑止のみで「@」から守られていたので、警告文に「この設定は今後効きません。半角で『@』が出る場合は無変換/変換を親指キーにしてください」と回避策を書く(GUI は親指のときにしか旧設定を書かないので、対象は手書きのユーザーだけ)。
- 警告(ADR-201 の診断経路、`validate_thumb_key_in_ime_combos` の該当分岐を置換): 「`*_solo_tap_ime_action` は非推奨です。`keys.ime_on/off/toggle` に bare で書くか、削除してください。GJI の CUSTOM 表で無変換/変換がトグルなら設定なしで動きます」。
- 移行で変わる差(既知・許容。所有者決定が M13 を上書きする): (1) M13(旧 `"toggle"`/`"on"` × `ModeKeyConfig`=Passthrough の「エンジン活性中は GJI 自身のかな切替」は実現できなくなる)。
  (2) composing 中も発火する(旧ケース1は発火しなかった。IME 側もそのキーで閉じる設定であることが前提)。(3) エンジン無効中: S1 の `match_event` は `engine_enabled` を見ないので無効中も能動(S2 は決定3のゲートで受動)。
  (4) 旧 `"off"` × エンジン非活性は「抑止のみ」から「絶対指定の OFF を書く」に変わる(決定3の代償)。(5) 意図の記録が `PhysicalImeKey` から `Command` になる。

### 決定5: エンジン非活性側の eisu 救済に GJI の英数保持を渡す(round2 5-1、BUG-159 の再発防止)

Decision 経由の `SetOpen(true)` の救済 `kp_stage_post_decision` の `eisu_reset_on_ime_on(applied && new_ime_on, input_mode, false)`(`key_pipeline.rs:1580-1584`)は `mode_retained` が固定の `false` なので、GJI が閉→開で英数を保持するのに awase だけ
`AssumedRomaji` に戻し、エンジンが NICOLA を送ってリテラルの `ka` が出る(BUG-159 の症状)。旧ケース2は `gji_retains_tracked_eisu` を渡していた。この経路にも同じ `mode_retained`(`shadow toggle` 経路の `:1322-1327` と同じ引数)を渡す。S1 の既存の穴も同時に直る。

### 決定6: GUI(ADR-192 T3)と検証(`validate_thumb_key_in_ime_combos`)を書き換える

- `apply_adr192_recommended_replacement` は、親指キーのとき `keys.ime_on`/`ime_off` に該当 bare 要素(`"変換"`/`"無変換"`)を**追記**する(既に含まれていれば何もしない。丸ごと置き換えると既定の `Ctrl+無変換` 等が消える)。
  `*_always_suppress = true` は不要なので書かない。非親指キーの既存の置き換え動作は変えない。snapshot/undo から S3 の項目を外し、プレビュー文と単体テスト(`main.rs:7757-7798`)を更新する。
- `validate_thumb_key_in_ime_combos` の「`*_solo_tap_ime_action` が優先され…無視されます」の分岐を削除し、旧設定の警告は決定4に移す。

### 決定7: 非固着の条件(受け入れ基準への回答)

**固着の定義(所有者、2026-09-29)= 何度モードキーを押しても状態が変わらないこと。** 1回目が期待と逆になり2回押せば期待どおり、は許容する。以下は「どの構成でも、押し続ければ状態が変わる」ことを示す。
前提: (P1) `SetOpen(v)` の送信が実際に行われれば `applied := v` になり、`applied == v` のときだけ `already_matches` が送信を省く(`gji_direct_already_matches`、GjiDirect のみ。ImmCross・MsImeDirect は常に送る)。
(P2) 送った `VK_IME_ON/OFF` が実 IME に効く。**Chrome は `VK_IME_ON/OFF` を受け付けなかった記録がある(`docs/experiments.md:103`、2026-05-22)。その後 GJI 全般を `VK_IME_*` に移した(`b271aee`/`489cdf1`)ので今は効くと推定されるが、
S2 は「生キーで GJI 自身が確実に処理していた打鍵」を awase の送信に置き換える。受け付けない入力先が残っていれば、生キーは抑止され送信は無視され、何度押しても変わらない=固着になる**。よって CI (a) に実 Chrome の入力先を必ず入れて P2 を確かめる。
(P3) force guard(`apply_panic_reset` の `PanicReset`、`expires_at: None`)が有効でないこと。有効な間は OFF 方向の要求がすべて Unwarranted(`ImeController::apply` は授権の無い order を実行しない)になり、guard は belief も上書きするので
トグルの指令も毎回 OFF のままになる。フォーカス変更で解ける既存の性質で、「状態をリセット」の後にフォーカスを変えず OFF 方向を押した場合の既知の制限として記録する(CI のゲートにはしない)。根治(明示のユーザー押下の OFF は guard より優先する等)は ADR-087/090 の領域で、別件の候補。
(P4) リレー型の窓が InputRelay に分類されていること。`INPUT_RELAY_APPS` に載っていない独自のリモートビューア・VM コンソールでは NotOwned にならず、ローカルに `VK_IME_*` が送られて生キーが消費され、リモート側の IME に何も届かない
(回避策は `input_relay_apps` への追加。分類漏れの窓では固着しうる既知の制限)。

1. **S2(トグル役割)と S1 の `keys.ime_toggle`**: 各押下で必ず belief が反転し(送信を省いても belief は書かれる、`handle_engine_set_open`)、次の押下の指令は反対向きになる(ON, OFF, ON, …)。連続する2回の指令のうち少なくとも1回は `applied` と異なるので必ず送信される(P1)。
   したがって belief/`applied` がどう古くても、押し続ければ ON と OFF の両方が実 IME に届き、状態は押すたびに変わる。永続的な固着は無い。最悪ケースは、`applied` も古く最初の指令が一致して省略される場合(素通しの別キーで閉じた後に
   `ModeKeyPassedThrough` が `applied` を触らない BUG-156 型)の「1 回目は変化なし、2 回目は逆向き、3 回目で期待どおり」で、所有者の定義では許容範囲。ADR-205 D6/D7 が入れば無駄押しは減る(D6 は観測できた外部 close だけ、D7 は Blind 窓の押下ごと)。
   生キーは常に抑止するので、IME 自身のトグルと awase の指令が打ち消し合うこともない。
2. **S1 の方向固定キー(`ime_on`/`ime_off`)**: 決定3で(b)を採らないので常に絶対指定で書き、belief が古くても押下ごとに送信される。ただし `applied` が実状態と食い違ったまま指令と一致する(検出できなかった外部変化)と送信が省かれ、何度押しても変わらない
   (BUG-156 型)。これは既存の S1 の性質で、ADR-205 D7(Blind 窓で押下ごとに `applied` を Unknown にする)が塞ぐ。**D7 の対象には、`bare_ime_action` または `forced_open_action` を持つ親指の非リピート Down を明示する**
   (親指の S1/S2 は `shadow_action`/`sync_direction` を持たないので、「shadow toggle で扱うキー」だけを対象にすると親指が漏れる。エンジン活性側は Down → FSM → KeyUp で送るが間に `applied` を書くものは無いので、Down で Unknown にしておけば KeyUp の送信は省かれない。round3 1-2)。
   出荷順は「ADR-205 D7 と同時、または D7 の後」(理由: 方向固定キー〈S1、移行した旧 S3 を含む〉のため。トグル系は D7 なしでも基準を満たす)。相互参照: ADR-205、ADR-208(BUG-172 の草稿)。
3. **失われる押下は固着ではない**: settle 中の `SetOpen` 剥がし・`already_matches` の省略・belief が古いときの「見た目変化なし」は、いずれも次の押下で状態が変わる(1 のとおり)。
4. **自発的な書き込みは増やさない**: 本 ADR が増やす書き込みは、ユーザーが親指キーを押した打鍵に対する1回の `SetOpen` だけ。タイマー・観測起点の開け直しは無い。例外は既存の `ActivationSync`(決定3の最後の項)。

### 訂正に伴う整理(2026-09-29 夕、所有者回答)

- **S1 × 単独タップ Suppress(`always_suppress`)**: S1 は設定に関係なく発火するので、`*_solo_tap_always_suppress = true` を残した設定でも bare の親指は書く。`always_suppress`（Suppress）は **S2（役割由来）の発火と、役割が無いときの素通しを止める**が、S1 bare の発火は止めない。
  意図に反しうる点: (i) 「Suppress にしたから IME を動かさない」と思ったユーザーが bare `keys.ime_*` に親指を書いていた場合、書かれる(bare は awase 側の明示設定なので従来どおり。**所有者決定 2026-09-29: S1 は単独タップが Suppress でも発火させる**〈Passthrough 限定にしない。GUI T3 が書く主流設定が動かなくなるため〉)。(ii) 旧 GUI T3 が書いた `*_solo_tap_ime_action` + `always_suppress = true` の組(親指)は、移行後は S1 になり Suppress でも発火する(旧実装のケース1も Suppress では発火したので同じ。ただし旧ケース1は Passthrough では発火しなかったので、移行後は Passthrough では新たに発火する〈M13〉)。
- **GUI T3 が書く主流設定(旧 `muhenkan_solo_tap_ime_action = "off"` + `always_suppress = true`)の移行後の挙動**: bare の `keys.ime_off` に「無変換」が入った S1 になり、Suppress のままでも発火する。エンジン活性側は単独タップ確定で絶対指定の OFF、
  エンジン非活性側は Down のエンジン特殊キー照合で絶対指定の `SetOpen(false)`(旧ケース3改の「抑止のみ」ではない。固着回避の決定3・7のとおり。旧 `"off"` × belief OFF の「抑止のみ」は戻らない)。GUI の新しい書き込み(bare の追記)も同じ挙動。
- **決定7(非固着)への影響**: S2 は Passthrough のときだけ能動なので、Suppress の S2 は「何もしない(飲み込むか素通し)」で状態を変えない設定であり、固着の議論の対象外。
  Passthrough の S2 は従来どおり(押すたびに belief が反転し指令が交互になる。2〜3回で期待状態)。S1 は変更なし。
- **Ctrl↑ を契機とする専用の actuation が無い(マージ条件、所有者。BUG-174 の再導入禁止)**: BUG-113/124 の「@」は Ctrl↑ 側の actuation が関与した(BUG-174: 旧 `CompositionEvent::CtrlUp` の eager warmup が Ctrl 押下中に `VK_IME_ON` を注入、`aa53eb4b` で撤去済み)。
  新設計は Ctrl↑ に actuation を持たない: 親指の開閉は修飾なしの親指だけが対象(`is_bare_thumb`)で Ctrl+無変換/変換は対象外、`on_ctrl_key_up` は chord barrier の解除だけ。これを次で固定する:
  `architecture_guard::ctrl_key_up_never_actuates_ime`(旧 CtrlUp 識別子の不在、`on_ctrl_key_up` の本体に SendInput/apply_ime_open_* 等が無いこと、パイプラインの Ctrl 系 KeyUp ブロックが `on_ctrl_key_up` の呼び出しだけ)と、
  `src/engine/tests.rs::ctrl_release_after_role_thumb_open_never_emits_ime_effects`(Ctrl+無変換の Down/Up と Ctrl↑ の決定に IME 効果が無い)。CI e2e は注入キーしか作れず物理の Ctrl↑ を再現できないため対象外(制約)。
  `origin/fix/eager-warmup-modifier-guard`(BUG-175: eager warmup を Ctrl/Shift/Alt/Win 押下中に抑止)は本 PR に含めない(別 PR。未マージ)。
  **この条件が保証する範囲と、範囲外の残る被疑**(round5 §3): 上のガードは「Ctrl↑ 専用の actuation ハンドラが無い」ことの固定で、文字どおりの「Ctrl↑ のイベントの決定に IME 効果が一切載らない」ではない。
  (1) Ctrl↑ も `engine.on_input` を通り、Phase 2 の `check_active_transition`(`engine.rs` の Phase 2)が、直前のキーから Ctrl↑ までの間に観測(poll・ADR-205 の watch・drift)で `ctx.ime_on` が変わっていれば
  `SetOpen(.., ActivationSync)` をその Decision に載せる(押されている修飾キーは `send_ime_mode_key` の `HeldModifiers` が一時的に離すので、BUG-174 の「Ctrl 押下中の `VK_IME_ON`」の構成にはならない見込みだが、条件の文面どおりではない)。
  (2) BUG-174 の残る被疑2(確定キー Enter/Esc の `ConfirmKeyDown` と reinject の eager warmup が 1 打鍵あたり `VK_IME_ON` を 2 回送る。ADR-167 の「連続 2 回以上の SendInput」に関わる、より強い被疑)と、
  BUG-175(修飾キー押下中の eager warmup、未マージ)は、Ctrl「押下中」の注入であり本条件の外。「@」の残りの被疑として別に追う。
  文面どおりの保証が必要なら、エンジンに「OS 修飾キーの KeyUp では `check_active_transition` の `SetOpen` を出さず次の非修飾キーへ持ち越す」1 分岐とテストを足す案(ii)があるが、ActivationSync のタイミングを変えるので**所有者判断**(本 PR は範囲を広げず (i) の文面)。
- **「@」の実機 A/B はマージ条件から外す**(所有者)。「@」の検証は未実施(旧 `"off"` × belief OFF の絶対指定 `VK_IME_OFF` 単発は BUG-124 の旧ケース3と同じ構成で、既知のリスクとして BUG-124 に追記)。
- 副次: `apply_config_update` の `set_thumb_role_open_actions(None, None)` は reload のたびに押下中の役割を消す(親指を押したまま reload が入るとその 1 打鍵は開閉しない。二重 actuation の方向ではない)。

## 検証計画

- 単体(Linux で走る、`src/engine/tests.rs`): エンジン非活性(IME OFF/`NotRomajiInput`)× 親指の役割由来 Toggle で「Consume＋絶対指定 `SetOpen(true)` が1つ、KeyUp も Consume」。
  **「belief OFF → Down → リピート Down ×3 → Up で `SetOpen` がちょうど1つ」**(round2 1-1)。`ImeOff`×belief OFF でも `SetOpen(false)` が積まれること(決定3の撤回の固定)。ユーザー無効・専用 Fn・`is_japanese_ime=false`・`sync_direction` あり・修飾付きで「素通し」。
  エンジン活性(FSM の `forced_open_action` の KeyUp 解決)は従来どおり。S3 依存の既存テストは S1 ベースに置き換える。`config.rs` に旧設定→S1 相当(親指のみ、同キー既存 bare があれば旧設定は移行しない、非親指は読み捨て)と警告のテスト。
- 統合(`architecture_guard.rs`): 決定2 の置き換えガード、`match_special_keys` の新分岐が `is_user_enabled`・`sync_direction`・専用 Fn のゲートを持ち `SetOpen` を直接積まないこと、`phase1_held` と `!was_down` ガードの存在、post_decision の eisu 救済が `mode_retained` を渡すこと、`enrich_thumb_key_role` が InputRelay で役割を付けないこと。
  `ime_key_sequence_golden.rs` は `ImeController` の戦略選択と送信列の検証であり「1回の押下で何回書くか」は表現できない(`runtime/` は `#[cfg(windows)]`)ので対象にしない。`cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings --tests` でコンパイル確認。
- CI e2e(`e2e-ime.yml` に `sc-solotap-*` を追加、`gh workflow run e2e-ime.yml --ref <ブランチ> -f only='sc-solotap-*'`)。**次の3系統は develop マージのゲート**(round2 §4):
  (a) GJI + CUSTOM 表(入力先に実 Chrome 相当を含め、awase の `VK_IME_*` が効くこと=P2 を確かめる)で無変換=トグル/非トグル × 直接入力/かな入力 × `--seq=1D,1D` の consistency と、1回の押下での開閉回数=1、変換側の対称。
  (b) **belief が古い状態でトグル系を押す(所有者要求。判定: 押下を重ねて状態が変わり、2〜3 回目で期待状態に到達すれば PASS、N 回押しても変化しなければ FAIL)**: 外部注入(ADR-205 の `cal-driftrec` 系と同じ手段)で IME を閉じた直後に無変換を 1〜3 回押し、期待状態に到達し、その後も押すたびに状態が変わることを見る。閉じた後 / 開いた後の両方向。
  (c) **素通しの英数キーで閉じた直後に無変換**(BUG-156 型。CUSTOM 表に「英数=IME を無効化」を明示的に入れる。GJI の状態〈GjiFsm が OffCold のまま残り BUG-170 型になっていないか〉も見る)。
  判定は consistency(実 IME の開閉と Engine の追随)、`observed` 件数、`[shadow-toggle]`/物理配送のログ。「@」は CI の入力先では出ない可能性が高いので CI では「生キーが GJI に届かない」で代替し、実機 A/B(Windows Terminal + GJI、半角状態で無変換/変換の単独タップ、旧 `"off"` 設定)で確認する。
- 記録: `docs/known-bugs/` には新規 BUG を起こさず、BUG-113/123/124 に本 ADR への追記を1行足す。ADR-153・192・199 のステータスに本 ADR による置換を追記する。

## リスクと反論(親エージェントの懸念への回答)

1. **二重トグル/二重信号**: エンジン非活性はエンジンの Phase 1 で「Consume＋SetOpen 1つ」で完結し、リピートは `phase1_held` で Phase 1 に閉じ、KeyUp は `UpDuty::Consume` で対になる(決定3)。エンジン活性側は FSM の KeyUp 解決(出荷済み、チョード優先)。
   S1 と S2 は同時に発火しない(`match_event` が先に一致すれば新分岐は評価されない)。
2. **「@」**: 「生キー抑止＋awase が1回だけ書く」は BUG-122/123 の実機確認で「@」なしだった構成(ただし旧ケース2は Windows パイプライン経由の書き込みで、S1 と同じエンジン経由の書き込みが同じ結果になるかは**未検証**)。
   OFF 方向 × エンジン非活性は決定3の代償(「@」の可能性、所有者判断候補)。方向固定の役割・ATOK・MS-IME 本体は受動のまま(範囲外)。
3. **belief が古いとき**: 決定7。トグル系は 2〜3 回押しで収束、絶対指定は ADR-205 D6/D7 に依存。`ControlLog.shadow_on` は `Option<bool>` のまま扱い、「送信を省略してよい」は陽性の確認済み証拠にだけ基づかせる。`already_matched` を切る案(強制 actuate=「@」の危険)は採らない。
4. **受動化の方針**: ADR-191・ADR-178 領域A撤去・ADR-199 決定1 の「役割を持つキーだけ能動」の範囲内。能動を増やすのは GJI の CUSTOM 表で無変換/変換がトグルのときのエンジン非活性側だけ(決定1 の但し書き)。旧 S3 の専用機構を削るので能動経路の数は純減。
5. **NICOLA 同時打鍵(PendingCharThumb・BUG-119)**: エンジン活性側の発火点は `resolve_pending_thumb_as_single`(単独タップ確定)だけでチョードでは発火しない。優先順位は 専用Fnキー ＞ `forced_open_action`(bare ＞ 役割)＞ `ModeKeyConfig`。
   `suppress_solo_output`(ADR-182 決定1b)・押下後 Shift のガードはそのまま。エンジン非活性側にはチョード判定が無い。開いた直後にエンジンが活性化した状態で親指が押されたままだと、次に押した文字キーは親指面になりうる(旧ケース2と同じ、記録のみ)。
6. **IME actuation 合流点**: 新しい入口は作らない(エンジンの `SetOpen` → 既存の `dispatch_ime_set_open`)。`RESTRICTED_CALLS`/tuning 定数は増減なし。`kp_reopen_gji_fsm(ShadowToggle)`(ADR-203 (ii))は shadow toggle 経路にしか無く、
   Decision 経由で awase が実際に書けば receipt が届くので不要だが、`already_matched` で送信を省いた場合は届かず GjiFsm が OffCold のまま残る(BUG-170 型)。検証 (c) で GjiFsm の状態も見る。

## 実装タスクの分割案

- T1: エンジン: `match_special_keys` の新分岐・`phase1_held`・専用 Fn/`sync_direction` ゲート、単体テスト。
- T2: post_decision の eisu 救済に `mode_retained` を渡す(決定5)。
- T3: S3 とマーカーと Windows 側ケース2/3改の撤去(決定2)、`architecture_guard` の更新。
- T4: `config.rs` の非推奨化・移行・警告(決定4)。
- T5: `awase-settings` の T3 書き換え(決定6)。
- T6: `sc-solotap-*` の追加(検証計画)。
- T7: docs(ADR-153/192/199 のステータス追記、BUG-113/123/124 に1行、`fix-requires-evidence.md`・`.githooks/pre-push`・README/usage)。

## 未検証事項(実装後も残るもの)

- 半角状態でエンジン非活性(IME OFF)のとき GJI の CUSTOM 表の無変換=トグルを押した場合の「@」の有無(実機 A/B、Windows Terminal + GJI)。旧ケース2 と、エンジン経由の書き込み(S1 と同じ)で結果が同じかも含む。
- 旧 `"off"`(GUI T3 の設定)が決定3の代償(フォーカス直後の `VK_IME_OFF` 単発)で「@」を出すか。
- `applied` が古いときの S2(決定7-1)の実測(CI シナリオ (b)(c))。ADR-205 D6/D7 の実装状況に依存する。
- MS-IME 本体・ATOK は対象外(受動のまま)。MS-IME 本体は ADR-199 T17 Phase 4 と B4 計画が決まってから同じ入力(S2)に合流させる。
- **CI e2e `sc-solotap-*` の限界（2026-09-29 実行結果を受けて）**: 5 構成×2 回すべて完走（toggle/henkan-toggle/nontoggle は consistency PASS 2/2、stale/after-passthrough は observe で FAIL 0）。
  ただしハーネスは `SendInput` で注入したキーを送り、awase は注入イベントを対象にしない（`is_bare_thumb` は `!event.injected`、BUG-14）ので、**この構成は本 ADR の新分岐（物理の親指単独押下）を通らない**。
  PASS は「この設定で awase が追随を壊さない」ことの確認であり、Consume＋絶対指定 `SetOpen`・リピート・stale belief の挙動そのものは検証していない（`src/engine/tests.rs` の単体テストが代わりに固定）。
  実際の物理押下は実機 A/B でしか検証できない（`SendInput` 注入では物理キー状態が作れない、`feedback_sendinput_cannot_test_physical_key_state`）。

## 追記(2026-10-04): 「IME に任せる(Passthrough)」の位置づけと設定画面の文言(PR #470)

**報告:** `keys.ime_on` に無変換を入れたうえで無変換の単独タップをパススルーにすると期待どおり動かない。

**結論: 設計どおりの挙動で、変更しない。** bare の `keys.ime_*`(`forced_open_action`)は `ModeKeyConfig` より優先され(決定の優先順位)、
単独タップは絶対指定の `SetOpen` になって生キーは IME に届かない。IME ON のときだけ素通しにする案は**不採用**:
エンジン(コア)は IME の状態を知らないので、プラットフォーム層が親指の KeyDown ごとに belief を見て役割を外す(`role_open_action` と同じ方式)ことになり、
belief が読めない窓(TsfNative、BUG-149/150)でずれると「ON なのに素通しだけして何も起きない」「OFF なのにキーを握りつぶす」になる。
IME を状態の正とする ADR-191 の方針とも逆向き。

**位置づけの統一(所有者決定):**
- `keys.ime_on/off/toggle` は、通常の IME 切替用ではなく、**モードずれが起きたときに awase と IME の状態を強制的にそろえる**キー。
- IME のオン/オフ・確定・再変換などを無変換/変換で行いたい場合は、**IME 側のキー設定で割り当て**(Microsoft IME は「キーとタッチのカスタマイズ」、
  Google 日本語入力はプロパティの「キー設定」の「コマンド」)、awase 側は単独タップを「IME に任せる(Passthrough)」にする。`keys.ime_*` には入れない。
- 既定の「無効にする(Suppress)」は、NICOLA 入力中に単独タップを飲み込む(IME にも送らない)。IME 側の割り当ては動かない。

**変更(PR #470。文言・警告・案内のみ、エンジンと hook は不変):**
- 設定画面: 「常に無視する/常に送出する」→「無効にする/IME に任せる」。ホバーに用途を記載。
- 設定画面: Passthrough 選択中に、そのキーが `keys.ime_*` にも入っていれば効かない旨をインライン警告。`keys.ime_on/off` のラベルを「強制的にそろえるキー(モードずれ補正用)」に変更。
- 読み込み時の警告(`config.rs::validate_thumb_key_in_ime_combos`)に同じ旨を追加。`KeysConfig::has_bare_role_key` を設定画面と共有。
- 設定画面に IME 側の設定を開くボタン: Microsoft IME は `ms-settings:regionlanguage-jpnime`(ページを開くだけ)、
  Google 日本語入力は `GoogleIMEJaTool.exe --mode=config_dialog`(プロパティを開くだけ)。開けない場合の手順(タスクトレイの「あ」/「A」→「プロパティ」等)は常に案内文として出す。

**実機確認(2026-10-04、dragonflyg4、GJI 3.34.6260.0):** `C:\Program Files (x86)\Google\Google Japanese Input\GoogleIMEJaTool.exe --mode=config_dialog` で
「Google 日本語入力 プロパティ」ウィンドウが開く。exe 内の文字列調査でもキー設定タブを直接開く引数は見つからず、タブの選択は利用者が行う。
他に `dictionary_tool`・`word_register_dialog`・`about_dialog` 等のモード名がある(未使用)。

**採らなかったもの:** 設定アプリ内の「キーとタッチのカスタマイズ」ボタンを UIA で押して直接遷移する案。
実証済み(`msime_key_assignment_settings_probe`)だが、AutomationId・表示言語・OS バージョン・互換モードに依存し、
プローブは遷移のために `SystemSettings.exe` を `taskkill` するため製品機能には使えない(所有者判断: OS バージョン依存は避ける)。

**未整理・未検証:**
- 設定画面の 2 択は Passthrough 選択時に `ignore_composing_guard=true` を固定で書く(idle も composing も素通し)。`config.toml` を直接編集して
  `*_solo_tap_always_suppress=false` だけ書くと「idle だけ素通し、composing 中は Suppress」になる(`ModeKeyConfig::from_legacy_bools`)。
  この 3 状態目を 2 択に統合するかは、MS-IME で変換中に無変換を素通しして誤爆しないかの実測が先(未実施)。
- Windows 実機での設定画面の見た目とボタンの動作は未確認(CI は windows-settings のビルド・テストまで)。
