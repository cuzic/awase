---
id: ADR-255
title: |-
  IME OFF のときだけ無変換/変換を Space にする——エンジンの非活性時の親指キー処理に足す(`[[keymap]]` の条件化は採らない)
summary: |-
  顧客報告「IME OFF のとき GJI の設定が反映されず、変換/無変換を空白入力に割り当てても動かない」を GitHub Windows CI で実機検証した(ブランチ ci/e2e-direct-space、run 38058313464・38059338837、各セル n=1)。GJI の CUSTOM 表は読まれ、直接入力の無変換に IMEOn を割り当てると効く(open 0→1。変換は既に開いた状態で試したので未分離)。直接入力の DirectInput 行に InsertSpace/InsertHalfSpace/InsertFullSpace を割り当てても入力欄に空白は入らなかったが、Precomposition の対照でも空白が入らず、「直接入力では不可」と「観測・キー名・コマンドの対象外」を分けられていない(GJI の仕様で不可とは断定しない)。そこで GJI に頼らず awase 側で、エンジンが非活性(理由 ImeOff)のときの無変換/変換の単独押下を Space にする設定を足す。Win32/IMM 系のウィンドウが対象で、IME OFF の判定は、エンジンが NICOLA の活性に使っている awase の ime belief そのもの(非活性・理由 ImeOff)にする。観測の鮮度や品質では絞らないので、Chrome・VS Code・Windows Terminal など IME の状態を読めないアプリでも動く(所有者の提案、r6 で改訂。r2〜r5 では `derive_actuating` による確認を AND にして、そのようなアプリを非対応にしていた)。belief が外れているときは NICOLA も効いていないので、実害は限定的と見る。当初案の `[[keymap]]` への `ime` 条件は、Opus レビュー r1 で、エンジンの活性判定と別の値を見て親指シフトが Space に化ける(B1)・未確定文字列の破棄(B2)・親指ラッチと latch の stale(M3/M4)が指摘されたため採らない。
status: |-
  起草 → Opus レビュー 改訂(2026-10-10): 所有者の提案で、IME OFF の判定をエンジン自身の belief だけにし、IME の状態を読めないアプリでも動かす形に変更(決定3)。Opus r6 で新規 Blocker/Must なし(r6 は所有者の提案による決定3 の置き換え)。**実装は決定3b のゲートで保留(報告者の回答待ち)**。
related_adr:
  - "ADR-114"
  - "ADR-206"
  - "ADR-141"
  - "ADR-230"
  - "ADR-245"
---

# ADR-255: IME OFF のときだけ無変換/変換を Space にする

## ステータス

起草 → Opus レビュー r1(Blocker 2・Must 7・Should 9・代案 3、[review/255-opus-review-round1.md](review/255-opus-review-round1.md))・r2(Blocker 1・Must 5・Should 7、[review/255-opus-review-round2.md](review/255-opus-review-round2.md))・r3(Blocker 1・Must 2・Should 4、[review/255-opus-review-round3.md](review/255-opus-review-round3.md))・r4(新規 Blocker なし・Must 3・Should 6、[review/255-opus-review-round4.md](review/255-opus-review-round4.md))・r5(新規 Blocker/Must なし。収束、[review/255-opus-review-round5.md](review/255-opus-review-round5.md))を反映(2026-10-10)。**実装は決定3b のゲートで保留(報告者の回答待ち)**。

## コンテキスト

### 報告

「IME OFF のとき、Google 日本語入力の設定が反映されず、変換/無変換を空白入力に割り当てても期待どおり動作しない」。

### 実機検証(GitHub Actions windows-latest、ブランチ `ci/e2e-direct-space`)

構成名 `sc-direct-space-*`(`.github/workflows/e2e-ime.yml`、判定なしの回収)。スパイクが IME OFF(`--seq` の 1A)にしてから無変換(0x1D)/変換(0x1C)を注入し、各押下の +1500ms の入力欄末尾(`tail`)と未確定(`comp`)を記録する。**各セル n=1**。run 38058313464(10 構成)、38059338837(対照を足した 13 構成)。成果物は 7 日で失効する。

| 構成(CUSTOM 表の行) | awase | 結果 |
| --- | --- | --- |
| 行なし(基準。`DirectInput,ON,IMEOn` のみ) | なし/あり | 無変換・変換とも open は 0 のまま、空白なし |
| DirectInput の無変換/変換 = InsertSpace | なし/あり/あり(単独タップ Suppress 解除) | 空白なし |
| 同 InsertHalfSpace / InsertFullSpace | なし/あり | 空白なし |
| 対照: DirectInput の無変換/変換 = IMEOn(`--seq=1A,1D,1C,1D`) | なし/あり | **最初の無変換で open 0→1**(両方)。変換は既に開いた状態で押したので、変換の行が効くかは未分離 |
| 対照: Precomposition の無変換/変換 = InsertSpace | なし/あり/あり(Suppress 解除) | IME は開くが空白なし |

### 分かったことと分かっていないこと

- 分かった: (1) CUSTOM 表は読まれ、`Muhenkan` というキー名は GJI に通じ、直接入力の行の**状態を変えるコマンド**(IMEOn)は効く。(2) 直接入力の InsertSpace 系は空白を出さなかった。
- **分かっていない**: Precomposition の対照でも空白が出ないので、次の仮説を分けられていない: (a) GJI の直接入力状態では InsertSpace 系は割り当ての対象外(設定ダイアログの直接入力行で選べるかを確かめれば切れる。未確認)、(b) 無変換/変換というキーでは InsertSpace が文字を出さない、(c) 入力欄の観測(`tail`)が空白を捉えていない(スペースキー自体を注入する正の対照が無い)。したがって「GJI の仕様で不可」とは書かない。本 ADR の決定は GJI の振る舞いに依存しない。
- フックの配送(訂正): フックは Accepted のキーを常に握りつぶし(`hook.rs` の `LRESULT(1)`)、エンジンの判断後にメインスレッドが**再注入**する。「awase は IME OFF の無変換/変換を素通しする」は正確には「再注入で GJI に届ける」。CI で awase あり/なしの結果が同じなので結論は変わらない。`physical_disposition.rs` の無変換/変換の Allow は「再注入する」の意味。

### 既存の機構

エンジンは非活性のとき bare の親指キーを `src/engine/engine.rs::thumb_open_role_action`(役割由来の開閉)で扱う。入口の条件に `compute_active`・`is_japanese_ime`・`is_bare_thumb`(物理・無修飾・非注入)・`sync_direction` の除外が揃う。エンジンの活性は `compute_state`(`ctx.ime_on`)が決める。

## 検討した案

- **案 A1(採用)**: エンジンの非活性時の親指キー処理に Space 出力を足す。
- **案 A2(不採用)**: `[[keymap]]`(ADR-114)に `ime = "off"` の条件を足す(当初案)。Opus r1 の指摘: (B1) `[[keymap]]` の照合はエンジンの活性判定と別の値を見るので、エンジンは活性(親指として使う)なのに `[[keymap]]` が先に消費して Space にしうる(`effective_open()` には IntentStore の上書きが重なり `resolve_open_at` と一致しない)。(B2) 誤って「閉」と判定すると `consume_keymap_match` が未確定文字列を破棄する。(M3) フック側の親指ラッチは `[[keymap]]` と無関係に立つ。(M4) latch が stale に残ると IME ON の親指打鍵が消える。(M5) 他ツールが注入する無変換が Space になる。いずれも、エンジンの内側に置けば構造的に消える。一般化(`on` を含む)には消費者もいない(S4)。
- **案 A3(不採用)**: GJI の CUSTOM 表を awase が生成する(ADR-231)。CI で InsertSpace が効いていないので、現時点の証拠では採れない。MS-IME 利用者にも効かない。

## 決定

### 決定1: 設定項目

`[general]` に `thumb_key_when_ime_off = "unchanged" | "space"` を足す。既定は `"unchanged"`(今までと同じ)。名前は仮(疑問5)。`set_space_thumb_config`/`space_thumb_vk`(Space を親指キーにする構成)や単独タップの `Suppress/Passthrough`(`muhenkan_solo_tap_*`)と語が紛れないようにした(R2-S6)。**IME OFF のときだけ指定できる**(IME ON のとき親指キーを別の用途にする設定は作らない。ON ではエンジンが親指として使う。所有者の提案、r1 S4 で `on` を落とした判断と同じ)。無変換/変換の両方に効く。`left_thumb_key`/`right_thumb_key` が無変換/変換のときだけで、それ以外のキーを親指にしている構成では何もしない。

### 決定2: 発動条件(エンジンの内側、すべて AND)

`engine.rs` の `match_special_keys` の `or_else` 連鎖の**末尾**(`thumb_open_role_action` の後。`engine_on` コンボ・`keys.ime_*`・自動検出トグルより後で、緊急復帰経路やユーザー設定を奪わない。R2-M4)に新しい種別(`SpecialKeyMatch` の新 variant、仮に `ThumbSpaceWhenImeOff`)を足し、次をすべて満たすときだけ Space を出す。

1. `!compute_active(ctx)` かつ非活性の理由が `ImeOff`(エンジン自身の判定。r1 B1 を構造的に解消。`ctx.ime_on` は IntentStore の上書きを含む `effective_open()` なので、上書きで「開」ならエンジンは活性になり発動しない)。`UserDisabled`/`NotRomajiInput` は今回含めない(決定7)。
2. `ctx.is_japanese_ime` かつ `adapter.is_enabled()`。
3. `is_bare_thumb(event, ctx.modifiers)`(`key_classification` が親指で、Shift・OS 修飾なし、**非注入**。alt-ime-ahk など他ツールが注入する無変換を除く。r1 M5)。**加えて、Alt なりすまし(`left/right_alt_impersonates_thumb_key`)由来の打鍵を除く**: フックは `cached_engine_enabled` が真のとき Alt を無変換に書き換えるが、このキャッシュは `EngineStateChanged` でしか更新されず、GJI 側で IME が閉じた直後の最初の打鍵が Alt だとまだ真のままで、Alt が Space になる(R2-M2)。書き換え後の vk では区別できないので、**フックが書き換えたときに `RawKeyEvent` へ `impersonated: bool` の印を付ける**(`apply_alt_impersonation`、`hook.rs`)。scan code(Alt 0x38 / 無変換 0x7B)での判定は、右 Alt→変換(0x79)の取りこぼしや Scancode Map・リマッパで scan が変わる問題があるので採らない(R3-S1)。印は境界 journal(ADR-250)にも乗る。`is_bare_thumb` の「物理」はなりすましを含むので、本条件で別に除く(R2-S7)。
4. **このキーが IME の機能を持たない**こと(R2-M1、R3-B1)。次の2つを**両方**満たす。
   - (i) 従来の4源がすべて None: `bare_ime_action(vk)`(`keys.ime_*`)、`thumb_role_open_actions()` の該当側(単独タップ設定が Passthrough かどうかを問わない)、`muhenkan_solo_tap_dedicated_fn_key`(専用 Fn キー)、`event.ime_relevance.sync_direction`。`thumb_open_role_action` は流用しない(専用 Fn キー設定済み、または役割があっても単独タップ設定が Passthrough でないとき `None` を返す。この場合エンジン非活性では生キーを通して IME 自身に開かせているので、`None` を「役割なし」と読むと IME を開く手段を奪う)。
   - (ii) **このキーの直接入力状態での効果の三値が `NoFunction`**: `KeyDirectInputEffect = HasFunction | NoFunction | Unknown`。**発動は `NoFunction` のときだけ**(名前は「直接入力でこのキーに IME の機能があるか」。`HasFunction` は IMEOn だけでなく `Reconvert`・`InsertSpace` 等の「開かないが機能がある」行も含む。R5-S3)。awase の `KeyRole`(`awase-gji-config/src/role.rs`)は `ImeToggle` の1つだけで、**開くだけの割り当て**(CI の `ctl-loaded` 表の `DirectInput,Henkan,IMEOn` 単独行、GJI の「変換/無変換で IME ON/OFF」オーバーレイ、MS-IME の「変換 = IME-オン」)では None を返すため、(i) だけでは IME を開く手段を奪う。三値の写像は `state/` の純粋関数にして Linux で表を固定する:
     - GJI(表が読める): 照合は**無修飾のキー名の完全一致**で行う(`Shift Henkan` のような修飾付きの行は、無修飾の変換の行と数えない。R4-S3。`awase-gji-config/src/role.rs` の `KeyStates::of` は「無修飾の行だけを表の順に読む(後勝ち)」で既にこの区別をしており、`Effect::of` が `DirectInput` の行を `Open`(IMEOn・絶対モード指定)か `Other`(Reconvert・InsertSpace 等)に分ける。「行があれば機能あり」は `states[DirectInput].is_some()` で書ける。テストで固定する。R5-S2)。プリセットまたは CUSTOM 表の `DirectInput` 行にそのキーの行があれば `HasFunction`(`IMEOn` 以外のコマンド〈`Reconvert`・`InputModeHiragana`・**`InsertSpace` 系**等〉でも**機能ありに数える**。Space で奪わない)。既知のオーバーレイ(`SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`)があれば `HasFunction`。**未知のオーバーレイ・未知の `session_keymap`(`source()` が `Source::Unknown` を返す場合)は `Unknown`**(R4-S2。`key_role` が未知のオーバーレイで全キーを受動にするのと同じ理由)。行が無ければ `NoFunction`。表が読めない・`table_ime_kind` が None も `Unknown`。
     - **プリセットの `DirectInput` 行を本番の表として持つ**(R4-M1): いまの `awase-gji-config/src/role.rs` はプリセットについて「トグルを持つキーの名前の一覧」(`Preset::toggle_vk_names`)しか持たず、Mozc の TSV 本体は同梱しない(ADR-199 決定4)。各プリセットの `DirectInput` 行はテストの定数(`MS_IME_TSV`/`ATOK_TSV`/`KOTOERI_TSV` と `*_DIRECT_INPUT`)にしか無い。本 ADR の実装は、4 プリセット(MS-IME・ATOK・KOTOERI・MOBILE)の変換/無変換の `DirectInput` 行を本番の定数として足す。出典は Mozc `b4bbc42f`(テストの定数と同じ)。Mozc の更新で古くなる性質があるので、`passive_open_vk_names_outside_table` と同じく手で取り直し、既存のテスト(`passive_open_vk_names_outside_table_match_mozc_direct_input_rows` 相当)で突き合わせる。前例は ADR-211 の `passive_open_vk_names_outside_table`(プリセットごとの定数を `source()` で選ぶ形)。この表で判定した結果(実装前にテストの定数から読んだもの):

       | GJI のキー設定 | 無変換 | 変換 |
       | --- | --- | --- |
       | MS-IME(**既定**。未設定・NONE・空の CUSTOM もこれ) | `NoFunction` → 発動 | `DirectInput\tHenkan\tReconvert` → 機能あり、**発動しない** |
       | ATOK | `IMEOn` → 発動しない | `IMEOn` → 発動しない |
       | KOTOERI | `NoFunction` → 発動 | `NoFunction` → 発動 |
       | MOBILE | MS-IME と同じ | MS-IME と同じ |
       | CUSTOM | 表しだい | 表しだい |

     - MS-IME 本体: 無変換/変換の値(0/1/2/3)の意味と「値なし」の既定の効果を確かめていないので、**今回は `Unknown`(発動しない)**。値の意味を実機で確かめた後に別途広げる。
     - TIP 未同定(`!ime_identified`。BUG-179 のように MS-IME を Other と同定する事例がある): `Unknown`。
   - つまり**当面発動するのは、GJI で、表が読めて、そのキーの `DirectInput` 行もオーバーレイも無いとき**だけ。上の表のとおり、**既定の MS-IME プリセットでは無変換しか Space にならず(変換は `Reconvert`)、ATOK プリセットでは両方ならない**。報告(変換/無変換の両方)に対して、既定のままの構成では半分しか効かない。さらに、**報告者自身が GJI の CUSTOM 表に足した `DirectInput` の無変換/変換の行(InsertSpace 等)があると、機能ありと数えるので両キーとも発動しない**(R4-M2)。案内は決定3b で扱う。
   - **キー効果の運び方**(R4-S4): 判定に要る GJI の表は、シェル側の `key_effect_keymap.get_gji(now_ms)`(I/O と間引きあり、`runtime/mod.rs::derive_key_shadow_action`)にあり、エンジン(OS 非依存)は自分で読めない。既存の `enrich_thumb_key_role` が押下ごとに `set_thumb_role_open_actions` で押した側だけ書く形に揃え、同じ場所で `KeyDirectInputEffect` も押した側だけ書く(古い値を残さない)。

5. **belief の品質では絞らない**(r6、決定3): 条件1 の判定(エンジン自身の belief)だけを「IME OFF」の根拠にする。`derive_actuating`・観測の鮮度・信頼度・`cannot_verify_real_ime_state` では絞らない。**除外するのは belief の品質と無関係な2つだけ**: (a) InputRelay(RDP・VM・PowerToys MWB。ローカルの belief はリモート側の IME 状態を表さない。ADR-206 も InputRelay では役割を付けない。R2-M3)、(b) ADR-245 の「戻り待ち」が立っている間(復元と順序が競合する。R2-S2)。この2つはシェルが `InputContext` の bool(`thumb_space_blocked` 仮称)で運ぶ。
6. 未確定文字列(composition)が無い(`!ctx.composing`、r1 B2)。composition があるのに「IME OFF」とみなすのは矛盾した証拠なので発動しない。**TSF のアプリで効くかは未検証**(`ime_composition_active_now()` は WinEvent の `EVENT_OBJECT_IME_SHOW`/`HIDE` が書くグローバルなフラグで、別アプリの composition 窓でも立つ。MS-IME での信頼性は `ime_decision_view.rs` が未検証とする。立ちっぱなしは安全側〈発動しない〉、立たないと変換の害になる)。検証計画2(c) で TsfNative 相当の窓と Chrome を含めて確かめる。

**不確かなときは今までどおり素通し**(`unchanged` と同じ)。素通しの理由は debug ログに出す(r1 M2)。

(注: 決定2 の項目5・6 と締めの段落は、r4 の改訂で誤って削除されていた〈r5 のレビューでも気づかれなかった〉。r6 で、項目5 を新しい内容にして復元した。)

### 決定3: IME OFF の判定はエンジン自身の belief だけにする(r6。r2〜r5 の決定3 を置き換える)

**所有者の提案**(2026-10-10): 親指キーの Space 化は IME OFF のときだけ指定でき、IME OFF の判定は awase の ime belief を使う。

経緯: r2〜r5 では、`derive_actuating(now) == Some(false)`(Actuating プールの観測が3秒以内)を AND にし、述語 `cannot_verify_real_ime_state` が真のアプリ(Chrome・Edge・VS Code・Windows Terminal・UWP・コンソール・RDP 等)では発動させなかった。理由は、これらのアプリでは `ObserverPoll` が書かれず、IME OFF のまま打っていると `derive_actuating` が 3 秒で `None` になり、「OFF にした直後の3秒だけ Space になる」切り分けの難しい挙動になるため(Opus r2 R2-B1)。

見直し: 次の理由で、belief だけにする方が良い。
- **エンジンの判断と一貫する**。エンジンは同じ belief で NICOLA の活性を決めている。「エンジンが IME OFF とみなしている間は親指キーが Space、ON とみなしている間は親指キー」は、利用者が見ている NICOLA の効き方と一致する。別の根拠(`derive_actuating`)を足すと、「NICOLA は効いていない(エンジン非活性)のに無変換が Space にならない」という食い違いを新たに作る。
- **belief が外れているときは、NICOLA もすでに効いていない**。IME が実際は ON なのに belief が OFF なら、エンジンは非活性で、利用者はすでにローマ字が出ている。そのとき無変換を押して Space が入っても、状況が新たに悪くなる範囲は限られる(下の「belief が外れたときの実害」)。
- 「確かめられるアプリでだけ動く」は、IME を実際に読めないアプリ(Chrome 等)を、報告者が最初に試す可能性が高いのに外してしまう。
- belief には awase 自身の actuation・打鍵時予測・明示意図・観測が入っており、エンジンが NICOLA の活性に使う値として既に審査されている(ADR-087・ADR-188・ADR-191)。

決定:
- 条件1(エンジンの `compute_state` が `Inactive(ImeOff)`)だけを IME OFF の根拠にする。`ime_off_confirmed`(`derive_actuating`)は導入しない。述語 `cannot_verify_real_ime_state` で除外しない。
- InputRelay と ADR-245 の戻り待ちは除外する(決定2-5)。
- **belief が外れたときの実害**(実装前に検証計画0 で測る):
  - 実際は IME ON、belief は OFF(エンジン非活性)で無変換を押す: 親指として使うつもりなら Space が入る(従来は無変換が IME に素通しされ、何も入らなかった)。IME が実際に ON のとき、この Space は IME によって変換(Composition)や全角スペース(Precomposition)として解釈されうる。変換中(未確定文字列あり)は条件6 で発動しないが、条件6 の信頼性は未検証(上記)。
  - 新しいウィンドウの直後(観測が無く既定値の「閉」が belief になっている間)に無変換を押す: 同上。この間 NICOLA も効いていない。
  - 実際は IME OFF、belief は ON(エンジン活性)で無変換を押す: Space にならず、従来どおり親指キー(NICOLA が効いている状態と一貫)。
- **記録**(切り分けのため): 発動・不発動の理由と、そのときの belief の根拠(`resolve_open_at` の `DecidedBy`)を debug ログに出し、境界 journal(ADR-250)に残せる形にする(疑問6)。「belief が外れたときの実害」の頻度を、実機・報告 journal で後から数えられるようにする。
- **観測品質で絞る案は捨てない**: 検証計画0 の計測で「belief が OFF なのに実 IME は ON だった」割合がアプリ種別ごとに無視できない大きさなら、別 ADR で「既定値だけが根拠のときは発動しない」(`DesiredFallback`・`HeuristicDefault` のみ)などの最小の絞りを足す(疑問1)。

### 決定3b: 実装着手の条件(ゲート)(R3-M1、R4-M2・M3。r6 で範囲を改訂)

この機能が報告に効くかは、報告者の GJI のキー設定に強く依存する(決定2-4・その表)ので、**報告者に次の4点を確認するまで実装に着手しない**。アプリの種類は、決定3 の見直しで「動くかどうか」の条件ではなくなったが、belief が外れたときの実害を見積もる材料として、引き続き集める。
1. **どのアプリで、IME OFF のとき無変換/変換を空白にしたいか**、および**直接入力か半角英数か**: 「タスクバーの A」は直接入力でも半角英数でも「A」と出るので答えられない(R4-M3)。代わりに、**問題の起きるアプリで、IME OFF の状態のまま無変換を押した直後に、トレイの「不具合を報告」(ADR-095)を送ってもらう**。報告にはフォーカス窓のクラス・プロファイル・belief(open・input mode)が入るので、(a) 押した時点でエンジンが非活性(ImeOff)だったか、(b) 直接入力か半角英数か(半角英数は決定7で範囲外)を客観的に読める。手で確かめてもらうなら、GJI の言語バーの入力モード表示で「直接入力」か「半角英数」かを見てもらう。
2. **押し続けて Space が連続して出る必要があるか**(決定4はリピートしない)。
3. **GJI のキー設定**(プリセット名、または CUSTOM)。決定2-4 の表のとおり、既定の MS-IME プリセットでは変換が発動せず(`Reconvert`)、ATOK では両方発動しない。
4. **CUSTOM の場合、直接入力の無変換/変換の行**(InsertSpace 等)があるか。**報告者の構成では、GJI の CUSTOM 表に足した InsertSpace 等の行が残っている可能性が高く、その行があると両キーとも発動しない**(機能ありと数える。R4-M2)。案内: 「GJI のキー設定の CUSTOM から、直接入力の無変換/変換の行を消す」(推奨。InsertSpace 系を `NoFunction` に数える案は、仮説 a〜c が未分離で、効く環境で Space が二重になる危険が残るので、設定ダイアログで直接入力行に InsertSpace を選べるかの確認〈検証計画0-iv〉が済むまで採らない)。この案内は設定画面の注記にも書く。

**ゲートが止めるのは実装(コード変更)だけ**。検証計画0 の計測・分類の確認・仮説 a〜c の切り分けの CI スパイクは、報告者の回答を待たずに進めてよい(R5-S6。(iv) の結果は報告者への返答の材料になる)。**報告者が回答しない間は実装しない**。保留のまま、別の報告で需要が出たら決定3b から再開する(後のセッションが「起草済み・未実装」を実装待ちと誤読しないため。R5-S5)。

結果に応じて:
- (a) 報告者のキー設定が発動範囲に入る(または入れてもらえる)なら実装する。アプリの種類は問わない。
- (b) キー設定が範囲外(ATOK プリセット・`Reconvert` を残したい等)なら、報告者には案内(設定の変更、または GJI の直接入力状態の制約の切り分け結果)を返し、実装は保留する。再変換を上書きする選択肢(疑問9)は需要が出てから扱う。

### 決定4: 出力と Down/Up

- Down で `VK_SPACE` を 1 回タップ(Down+Up)する。`VK_SPACE` はエンジンに生の VK 定数を持たせないため(ADR-019)、`set_space_thumb_config`(`engine.rs`)と同じくプラットフォームから渡す(R2-M4)。
- KeyUp は `KeyLifecycle` の `UpDuty::Consume` で回収する(`[[keymap]]` の latch は使わない)。
- **リピートしない**: 新しい種別にも `!event.was_down` の条件を付ける。既存の `check_special_keys` 先頭のガードは `thumb_open_role_action` 専用で新しい種別には効かず、`phase1_held` は flush 等で消えうるため、付けないとリピートで Space が連射される経路が残る(R2-M4)。無変換を押し続けても Space は 1 個(r1 S5)。本物の Space キーや GJI の InsertSpace(リピートする)とは違う。
- **Phase 1 の早期 return の遅れへの手当**(R2-S1): Phase 1 で Consume すると Phase 2 の `check_active_transition` を通らずに return するので、GJI 側で IME が閉じた直後の最初の打鍵が無変換だと、`EngineStateChanged{false}`(トレイ表示・Alt なりすましのキャッシュ)、保留出力の解放、FSM の flush が次の打鍵まで遅れる。Space 経路は IME を変えないので遅れたままになる。Space の前に `check_active_transition` の effects を前置する(`prepend_effects`)。**前置するのは `check_active_transition` の戻り値そのまま**(flush・保留出力の解放・`EngineStateChanged`。R3-S2、R4-S5、R5-S1): `check_active_transition` は `transition_activation(new_state, false)` で呼ばれ(`src/engine/engine.rs` の `check_active_transition`、ADR-213 決定3 P2b「観測・RefreshState 由来の遷移は SetOpen を出さない」)、`SetOpen` は元から含まれない。この経路は IME を変えない(Space を出すだけ)ので、IME actuation 合流点ファミリー(`fix-requires-evidence.md`)に触れず、`press` の配線も要らない。テストで、前置した effects に `SetOpen` が無いことを固定する。
- Platform 側の述語との整合(R2-M4): `match_special_keys` は Platform からも呼ばれる。`matches_ime_set_open`(shadow の書き込みの抑止、`engine_owns_open_key`)と `matches_ime_off`(Ctrl+無変換の救済窓)では、新しい種別は**開閉ではない**ので `None`/`false` を返す。網羅 `match` のテストで固定する。
- 押している間に IME が ON になったとき: 活性化後、文字キーはフックのスナップショット(`ctx.left_thumb_down`)で親指シフト扱いになりうる。既存の役割経路(変換 = IME ON の単独押下)にも同じ窓がある。**期待値は既存の役割経路と同じにする**(実装前に現挙動を調べ、検証計画1で固定する。違えるなら理由を書く。R2-S4)。KeyUp は活性化後に FSM へ Down なしの親指 Up として届く(`engine.rs` は非活性時だけ `release_only`)ので、この挙動も単体テストで固定する。
- 再生(`OUTPUT_GATE`/`INPUT_DEFER` で退避された Down)は、再生時点の状態で評価する(r1 S1)。

### 決定5: 競合と設定画面

- 決定2-4 により、`keys.ime_*`・専用 Fn キー・IME 設定/学習表由来の役割があるキーでは発動しない。重なりは設定読み込み時に警告する(「既存の衝突警告に載せる」ではなく新規。`warn_if_vk_conflicts` は dedicated fn key の2箇所だけで、`keys.ime_*` との衝突警告は実在しない。r1 M6-2)。
- **awase が読めない IME 側の割り当て**(MS-IME 本体の「キーとタッチのカスタマイズ」、TIP 未同定、表が読めない場合)は、決定2-4(ii) により `Unknown` になり、**この機能は何もしない**(Space で上書きしない。R4-S6)。設定画面の注記に「IME の割り当てが読めないときは何もしない」と書く。
- 設定画面は、親指キーの設定の近くに「IME OFF のとき無変換/変換を Space にする」のチェックを置く。注記に、(1) リピートしない、(2) 「半角英数」(IME は開いたまま英数モード)の状態では動かない、(3) awase が IME 状態を取り違えているとき(IME は ON なのに awase が OFF と思っているとき)は、無変換が Space(IME によっては変換や全角スペース)になることがある、(4) IME の割り当てが読めないときは何もしない、を書く。
- panic 検出(`record_ime_keydown`)は `deliver_key_event` より前に数えるので、Space に変えても計数は増減しない。重なる構成では決定2-4 により発動しないので、速い交互押下は Space 機能と無関係に今と同じ(R2-S7。r1 の検証計画4 は削る)。

### 決定6: リリース

- オプトイン(既定 `"unchanged"`)。Scancode Map(ADR-230)や、かな/無変換/変換の挙動変更とは同時にリリースしない。回帰テスト(検証計画1・2)が通り、develop で実機確認が済むまで次の v2 リリースに入れない。v1 への backport はしない(新機能)。

### 決定7: 範囲外

- `NotRomajiInput`(半角英数)での Space 化(r1 S3)。報告者の「IME OFF」がタスクバーの「A」(半角英数)なのか、直接入力なのかを先に確認する。
- TSF ネイティブでの Space 化(決定3)。
- `[[keymap]]` の条件化、`ime = "on"`(r1 S4)。必要になったら別 ADR。
- GJI の CUSTOM 表の生成(A3)。

### 影響範囲と再発ファミリー(R2-M5)

`src/engine/engine.rs`(`match_special_keys`・新 variant・`on_input_body` の前置)、`src/engine/nicola_fsm.rs`(役割の判定の純粋関数)、`src/config.rs`(設定)、`state/` に `KeyDirectInputEffect` の純粋関数と、InputRelay・戻り待ちを運ぶ `thumb_space_blocked`(仮称)、`crates/awase-gji-config/src/role.rs` にプリセットの `DirectInput` 行の本番定数(決定2-4)、`runtime/mod.rs` の `enrich_thumb_key_role` に `KeyDirectInputEffect` を押した側だけ書く経路、`runtime/key_pipeline.rs`(`InputContext` の構築)、`hook.rs`(なりすまし由来の印)、`crates/awase-settings`(チェックと注記)。`fix-requires-evidence.md` の再発ファミリーの**キー選択(IME ON/OFF に送る VK)**(`engine.rs::thumb_open_role_action` はエンジン非活性側の入口として明記されている)と**物理キー押下ラッチ(Down/Up 非対称)**に触れる。同じ PR に (a) 回帰テストを含める。置き場所は `src/engine/tests.rs`(`cargo test --lib`、ホストで実行可)と `state/` の純粋関数のテスト。`runtime/` 配下の `#[cfg(test)]` は Linux に存在しないので使わない。

## 検証計画

0. **実装前の確認と計測**(R2-B1、R3-M1、R3-S3、r6): (i) **報告者への確認4点(決定3b)**。(ii) 入力先の分類の確認: CI の入力先(ADR-193 の RichEdit スーパークラス化は「TsfNative 相当」)と、素の Edit コントロール(Standard の IMM の窓)で、`AppImeProfile` と `cannot_verify_real_ime_state` の値をログで確かめる。Windows 11 のメモ帳の分類もあわせて確かめる(決定3 の見直しで「動くかどうか」の条件ではなくなったが、belief の外れやすさの見積もりに使う)。(iii) **計測の目的は「belief が外れる頻度」**: アプリ種別(Standard・TsfNative 相当・Chrome・Windows Terminal)ごとに、無変換を押した時点の「エンジンの判定(`Inactive(ImeOff)`)」と「実 IME の open(スパイクの `A(open)`)」を突き合わせ、「belief は OFF、実際は ON」の割合を数える(決定3 の「belief が外れたときの実害」の大きさ)。大きければ疑問1 の絞りを別 ADR で検討する。**無変換を Space に変えると、その打鍵が IME に届かず、物理 IME キーを契機に走る観測(refresh・ADR-188 の窓内の直接読み)の機会が失われる**ので、belief が外れている間に無変換を繰り返し押すと外れが続きやすい。実害は小さい見込みだが、「belief が OFF・実際は ON」の持続時間を測るときは、Space 化を有効にした構成と無効の構成で比べて、この影響を区別する(R6-S5)。(iv) 正の対照(スペースキー 0x20 の注入で入力欄の `tail` に空白が出るか)、直接入力行の変換(0x1C)の IMEOn、設定ダイアログで直接入力行に InsertSpace を選べるか(仮説 a〜c の切り分け)。(v) 基準構成の「無変換・変換とも open は 0 のまま」は確認済み: run 38059338837 のジョブ `e2e (sc-direct-space-baseline-noawase-1)` と `e2e (sc-direct-space-baseline-awase-1)` の成果物 `dist/ime_key_matrix_spike.log` の KEY 行(2026-10-10 に確認)。各 n=1。
1. **Linux 単体**(`cargo test --lib`、判断は `src/engine` と `state/` の純粋関数): 発動条件の表(非活性理由 × `thumb_space_blocked`(InputRelay・戻り待ち)× composing × 注入 × なりすまし由来 × 修飾 × 従来の4源 × **`KeyDirectInputEffect` の各行(GJI の `DirectInput` 行あり/なし・オーバーレイ・CUSTOM の `ctl-loaded` 表・表なし(`Unknown`)・MS-IME の値 0〜3(`Unknown`)・TIP 未同定(`Unknown`)・**CUSTOM の `DirectInput` 行が InsertSpace 系(機能ありで発動しない。R4-M2)・修飾付きの行〈`Shift Henkan`〉のみ(無修飾は `NoFunction`。R4-S3)・未知のオーバーレイ/未知の `session_keymap`(`Unknown`。R4-S2)・4プリセットの変換/無変換(決定2-4 の表)**)** × `was_down` × 設定値)。InputRelay・戻り待ちのとき発動しない。**`cannot_verify_real_ime_state` が真でも、エンジンが `Inactive(ImeOff)` なら発動する**(決定3。観測の鮮度に依らないことを固定する)。`KeyLifecycle` の Down/Up/リピート。活性化中の押下の期待値(決定4)。網羅 `match` による `matches_ime_set_open`/`matches_ime_off` の固定。
2. **Windows CI**(`e2e-ime.yml`): 構成に GJI の CUSTOM 行(InsertSpace 等)を**入れない**。入力先は**Standard の IMM の窓と、TsfNative 相当の窓(ADR-193 の RichEdit スーパークラス化)の両方**にする(決定3 の見直しで、どちらでも発動する)。(a) IME OFF の無変換で入力欄に空白が1つ入る、(b) IME ON では入らず、無変換+文字キーが親指シフト文字になる、(c) **負の対照**: 注入された 0x1D では入らない/composition 中は入らない(Standard の窓で)/ `DirectInput,Henkan,IMEOn` の行がある構成と、GJI の「変換/無変換で IME ON/OFF」オーバーレイの構成で、変換が IME を開き Space にならない/ Alt なりすまし + GJI 側からの IME OFF → 最初の Alt が Alt のまま/ Ctrl+無変換(救済窓)と Shift+無変換が従来どおり/ 長押しで Space が1個だけ。(d) **belief が外れた場合の確認**(TsfNative 相当の窓): IME を実際に ON にしたまま awase の belief が OFF になっている状態(外部から IME を ON にして、awase が観測する前)で無変換を押したときの結果を記録する(決定3 の「belief が外れたときの実害」)。記録項目に「入力欄に何が入ったか(半角/全角スペース、変換)」を含める(R6-S3)。
3. **実機**: 報告者の構成(GJI の設定、親指キー、`keys.ime_*`、アプリ)。

(**注**: r6 で決定3 を置き換えたため、以下の対応表のうち `derive_actuating`/`ime_off_confirmed`/`cannot_verify_real_ime_state` に関する行は失効している。現行の決定は「決定」の節が正。)

## Opus レビュー r1 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| B1 | 反映。`[[keymap]]` 案を廃し、エンジン自身の非活性判定に乗る(決定2-1) |
| B2 | 反映。composition があれば発動しない(決定2-6)。キャンセル処理を通らない |
| M1 | 反映。`derive_actuating` を使い、DecidedBy の写像表は作らない(決定2-5) |
| M2 | 反映。実装前の計測(検証計画0)と、素通しの理由のログ(決定2) |
| M3 | 反映。エンジンが Down を見るので親指ラッチと食い違わない。窓は回帰テストで固定(決定3、検証計画1) |
| M4 | 反映(構造的に解消)。`[[keymap]]` の latch を使わない |
| M5 | 反映。`is_bare_thumb` の非注入条件(決定2-3)、負の対照(検証計画2c) |
| M6 | 反映。役割由来の開閉が勝つ(決定2-4)、警告は新規実装(決定4)、panic 検出は検証計画4 |
| M7 | 反映。フックの配送の訂正、無変換だけが open した事実、仮説(a)(b)(c)、n=1 を summary に明記 |
| S1 | 反映。二重配送は起きない(フックは常に握りつぶし、再注入のみ)。再生は再生時点で評価(決定3) |
| S2 | 不要(`find_match` を触らない) |
| S3 | 反映。決定7で範囲外とし、報告者に確認する |
| S4 | 反映。`on` は入れない |
| S5 | 反映。リピートしないことを決定3・5に明記 |
| S6 | 反映。検証計画3 |
| S7 | 反映。検証計画の0〜4 |
| S8 | 該当なし(`[[keymap]]` の項目を増やさない)。新しい設定名は疑問5 |
| S9 | 反映。決定6 |
| A1/A2/A3 | A1 を採用、A2・A3 は不採用(検討した案) |

## Opus レビュー r2 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| R2-B1 | 反映(決定3)。TSF ネイティブでは決定的に非対応。鮮度を問わない根拠は今回採らない。検証計画0 をプロファイル別の計測に直した |
| R2-M1 | 反映(決定2-4、決定5)。`thumb_open_role_action` を流用せず独立した純粋関数で 4 源すべて None を見る。IME 側の割り当てを上書きする旨を注記 |
| R2-M2 | 反映(決定2-3)。なりすまし由来を印か scan code で除く。検証計画2 に対照を足した |
| R2-M3 | 反映(決定2-5a)。InputRelay では `ime_off_confirmed=false` |
| R2-M4 | 反映(決定2 冒頭、決定4)。`match_special_keys` の末尾、`!was_down`、Platform 側述語の `None`/`false`、Space の VK はプラットフォームから渡す |
| R2-M5 | 反映(影響範囲と再発ファミリー) |
| R2-S1 | 反映(決定4)。`check_active_transition` の effects を前置する |
| R2-S2 | 反映(決定2-5b)。戻り待ちの間は偽。ADR-245 側にも同じ記述を足す(PR 2 のときに) |
| R2-S3 | 反映(検証計画2c)。Win32 の窓で確かめる |
| R2-S4 | 反映(決定4)。既存の役割経路と同じ期待値にし、実装前に現挙動を調べる |
| R2-S5 | 反映(検証計画 0〜3)。journal への記録は疑問6 |
| R2-S6 | 反映(決定1)。`thumb_key_when_ime_off = "unchanged" | "space"` |
| R2-S7 | 反映(決定2-3、決定5)。`is_bare_thumb` の「物理」はなりすましを含む。panic 検出の計画は削除 |

## Opus レビュー r3 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| R3-B1 | 反映(決定2-4)。`KeyDirectInputEffect = HasFunction\|NoFunction\|Unknown` を新設し、発動は `NoFunction` のときだけ。MS-IME 本体・TIP 未同定・表が読めないときは `Unknown`。`DirectInput` 行の IMEOn 以外のコマンドも機能ありに数える。当面発動するのは GJI で表が読め、そのキーの行もオーバーレイも無いときだけ |
| R3-M1 | 反映(決定3、決定3b)。述語を `cannot_verify_real_ime_state` に確定し、実効範囲を明記。報告者の確認3点を実装着手のゲートにした |
| R3-M2 | 反映(検証計画0-ii、2)。CI の入力先は Standard の IMM の窓。TsfNative 相当の窓は「入らない」の対照 |
| R3-S1 | 反映(決定2-3)。フックが `impersonated` の印を付ける |
| R3-S2 | 反映(決定4)。実装時に `emit_set_open` と actuation 合流点ファミリーへの該当を確認、`press=None` |
| R3-S3 | 反映(検証計画0・1・2) |
| R3-S4 | 反映(決定3、検証計画0-v)。Chrome は `Imm32Unavailable`、WezTerm/Windows Terminal は `TsfNative`。基準構成の確認に run ID とジョブ名を添えた |

## Opus レビュー r4 への対応(指摘 ID ごと)

| ID | 対応 |
| --- | --- |
| R4-M1 | 反映(決定2-4)。プリセットの `DirectInput` 行を本番の定数として足す(出典 Mozc `b4bbc42f`)。判定表を載せ、「報告の構成に合う」を取り下げた(既定の MS-IME では無変換だけ、ATOK では両方ならない) |
| R4-M2 | 反映(決定2-4、決定3b-4)。報告者の InsertSpace 行は機能ありに数えるので発動しない。案内は「CUSTOM から該当行を消す」(推奨 a)。InsertSpace 系を `NoFunction` に数える案(b)は仮説 a の確認後 |
| R4-M3 | 反映(決定3b-1)。トレイの「不具合を報告」で集める。確認事項は4点に |
| R4-S1 | 反映(未解決の疑問)。再変換を上書きする選択肢は需要が出てから |
| R4-S2 | 反映(決定2-4)。未知のオーバーレイ・未知の `session_keymap` は `Unknown` |
| R4-S3 | 反映(決定2-4)。無修飾のキー名の完全一致 |
| R4-S4 | 反映(決定2-4、影響範囲)。`enrich_thumb_key_role` と同じ形で押した側だけ書く |
| R4-S5 | 反映(決定4)。`check_active_transition` は元から `SetOpen` を出さない(r5 で事実の書き方を訂正) |
| R4-S6 | 反映(決定3、決定5)。メモ帳の注記、MS-IME 本体の注記の書き換え |

## Opus レビュー r5 への対応(新規 Blocker/Must なし。Should 6 件)

| ID | 対応 |
| --- | --- |
| R5-S1 | 反映(決定4)。`check_active_transition` は元から `SetOpen` を出さない。テストで固定 |
| R5-S2 | 反映(決定2-4)。`KeyStates::of`・`Effect::of` を引用して閉じた |
| R5-S3 | 反映。`KeyOpenEffect` → `KeyDirectInputEffect = HasFunction \| NoFunction \| Unknown` に改名 |
| R5-S4 | 反映。review ファイルに frontmatter を付け、round1 に改名の注記。index の補助資料の行も更新 |
| R5-S5 | 反映。ステータスを更新し、回答が無い間は実装しない旨を決定3b に書いた |
| R5-S6 | 反映(決定3b)。ゲートが止めるのは実装だけ。CI スパイクは回答を待たず進めてよい |

## 所有者の提案による改訂(r6)

| 変更 | 内容 |
| --- | --- |
| 決定1 | IME OFF のときだけ指定できる(ON 用の設定は作らない)と明記 |
| 決定2-5・2-6 | r4 の改訂で誤って削除されていた項目5・6 を復元。項目5 は `ime_off_confirmed`(`derive_actuating`)を廃止し、除外は InputRelay と戻り待ちだけに。項目6(composition なし)は元の内容 |
| 決定3 | 置き換え。IME OFF の判定はエンジン自身の belief だけ。`cannot_verify_real_ime_state` で除外しない。belief が外れたときの実害と記録を明記 |
| 決定3b | アプリの種類は「動くかどうか」の条件ではなくなった。ゲートは報告者のキー設定が中心 |
| 検証計画 | 計測の目的を「belief が外れる頻度」に。CI の入力先は Standard と TsfNative 相当の両方。belief が外れた場合の確認を追加 |
| 設定画面の注記 | 「TSF ネイティブでは動かない」を削除し、「awase が IME 状態を取り違えているとき Space になることがある」を追加 |
| 影響する旧指摘 | R2-B1(`derive_actuating` の鮮度)・R3-M1(実効範囲)・R3-M2(CI の入力先)。R2-M3(InputRelay)は決定2-5 で維持 |

## Opus レビュー r6 への対応(新規 Blocker/Must なし。Should 5 件)

| ID | 対応 |
| --- | --- |
| R6-S1 | 反映。影響範囲・検証計画1 の `ime_off_confirmed` を `thumb_space_blocked` に。旧対応表に失効の注記 |
| R6-S2 | 反映(決定2-6、検証計画2c)。TSF での composition フラグは未検証と明記し、CI で確かめる |
| R6-S3 | 反映(決定3、設定画面の注記、検証計画2d)。IME が Space を変換・全角スペースと解釈しうる |
| R6-S4 | 反映(疑問1)。最小の絞りの候補を併記 |
| R6-S5 | 反映(検証計画0-iii)。観測の機会の喪失を区別して測る |

## 未解決の疑問

1. 決定3 の見直し後、観測品質で絞る最小の案を要するか。最小の候補は「`resolve_open_at` の base が `MostRecentTrusted` の Low ソース(`HeuristicDefault` 等)のときは発動しない」(R6-S4)。なお `desired_open` の初期値は true なので、情報ゼロの起動直後はもともと発動しない。絞りが要るかは検証計画0-iii で決める。検証計画0-iii の「belief は OFF、実際は ON」の割合しだい。Windows 11 のメモ帳など XAML/RichEdit 系の分類は未確認(検証計画0-ii)。
2. 決定2-1 と IntentStore: IntentStore の上書きが「開」ならエンジンは活性になり発動しない(安全側)。ただし Phase 1 と Phase 2 の順序(R2-S1)で、遷移の前後どちらの `ctx` で判定するかを実装時に確かめる。
3. 親指キーが無変換/変換以外の構成での扱い(今回は何もしない)。
4. リピートしない仕様で報告者の期待に合うか。
5. 設定名(`thumb_key_when_ime_off`)と値。
6. エンジンの判断(発動・不発動の理由)を ADR-250 の境界 journal に残すか、debug ログだけにするか(報告 journal で「なぜ Space にならなかったか」を追えるように)。
7. 報告者への確認4点(決定3b)は未回答。回答次第で実装するか保留するかが決まる。
8. MS-IME 本体の無変換/変換の値 0〜3 と「値なし」の既定の効果。確かめるまで `Unknown`(発動しない)のままだが、MS-IME 利用者にも効かせる要望が出たときの測定方法。
9. 再変換(既定の MS-IME プリセットの変換は `Reconvert`)を Space で上書きしてよいと選べる余地を残すか。今回は GJI のキー設定を CUSTOM にして該当行を消す案内で足りるはずなので入れず、需要が出てから扱う(R4-S1)。
