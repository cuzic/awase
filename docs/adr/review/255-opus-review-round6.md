---
id: ADR-255-companion-255-opus-review-round6
title: |-
  ADR-255（IME OFF のときだけ無変換/変換を Space にする）Opus敵対的レビュー round6
type: companion-doc
related_adr:
  - "ADR-255"
---

# ADR-255 敵対的レビュー round6(Opus)

対象: `docs/adr/255-ime-off-thumb-key-space.md`(ワークツリー keymap-ime、HEAD `df6ec3db`)。行番号はこの HEAD のもの。

## 総評

**新規の Blocker/Must なし。**

- 所有者の提案(IME OFF の判定をエンジン自身の belief だけにする)は、実コードの上で成り立つ。r2 の Blocker R2-B1「押すタイミングで Space になったりならなかったり」も、この方式では**構造的に消える**(下の Q1-4)。
- belief が外れたときの実害は、「NICOLA がすでに効いていない状態に Space が 1 つ加わる」程度に収まる。ただし ADR の見積もりには 2 点の書き漏れがある。(1) Space が IME に入って変換や全角スペースとして解釈されうる。(2) composition のフラグの有効範囲は未検証。いずれも Should。
- **判断**: 下の S1(古い参照の掃除)を直せば、docs として develop にマージしてよい。実装は従来どおり決定3b のゲートで保留。

---

## 自己申告の確認: 決定2-6 の復元

- r3 の版 `01a98ccb` の項目6(`git show 01a98ccb:docs/adr/255-ime-off-thumb-key-space.md` の 78 行目)は「6. 未確定文字列(composition)が無い(`!ctx.composing`、r1 B2)。composition があるのに「IME OFF」とみなすのは矛盾した証拠なので発動しない。」。
- r6 の項目6 はこの文と**一致**し、末尾に「TSF のアプリでも `ime_composition_active_now()` は効く。」が**新たに足されている**。締めの段落「不確かなときは今までどおり素通し……debug ログに出す(r1 M2)」も一致する。
- 足された一文は裏付けが無い(S2)。「復元」と言うなら、元の文だけに戻すか、足した文を検証待ちの主張として書き分けること。r5 で削除に気づかなかったのは私(レビュアー)の見落とし。r5 では差分のうち改訂箇所だけを読み、決定2 の項目数を数えなかった。

---

## 見てほしい点への回答

### Q1. 決定3 の根拠は実コードで成り立つか

1. **「エンジンの判断と一貫する」**: 成り立つ。条件1 は Phase 1 の `match_special_keys` で `compute_state(ctx)` を見る。ctx は同じ打鍵の `on_input` に渡る値(`key_pipeline.rs` の `ctx` 構築、`effective_open()` を含む)なので、Space 化の判定と NICOLA の活性は**同じ値**で決まる。食い違いは原理的に起きない。
2. **「belief が外れているときは NICOLA もすでに効いていない」**: 成り立つ。belief が OFF ならエンジンは `Inactive(ImeOff)` で、Phase 2 が `pass_through` を返す(`engine.rs:512-522`)。利用者はローマ字の生打鍵を IME に送っている状態になる。
3. **実害の見積もりの書き漏れ**(Should、S3): ADR は「Space が入る」とだけ書いている。実際に IME が ON のとき、注入した `VK_SPACE` は **IME がまず解釈する**。
   - 未確定文字列があり、条件6 のフラグが立たなかった場合: GJI のどのプリセットでも Composition の Space は変換(Convert)。未確定文字列が変換される。
   - 未確定文字列が無い場合: Precomposition の Space は InsertSpace で、既定では**全角スペース**になりうる。
   - 加えて、本来 IME に届いていた無変換が届かなくなる。MS-IME プリセットの Precomposition の無変換は `CompositionModeSwitchKanaType` で、belief の回復手段ではないので実害は小さい。ただし、物理 IME キーが IME に届いたことを契機に走る観測(`may_change_ime` → refresh、ADR-188 の窓内の直接読み)の機会も 1 回失う。belief の外れが少し長く続きうる。
   - この 3 点を決定3 の「belief が外れたときの実害」に足すこと。
4. **R2-B1 の「時々」はどう変わるか**: 消える。`derive_actuating` の 3 秒の鮮度窓を使わないので、観測の古さで判定が反転することはない。判定が変わるのは belief そのものが変わったときだけで、そのとき NICOLA の効き方も同時に変わる。利用者からは「NICOLA が効いていない間は無変換が Space」という一つの規則に見える。残る「不安定さ」は belief 自体の外れで、計画0(iii) が測る対象と一致している。
5. **新しいウィンドウ直後の既定値**: `assume_closed_for_new_thread`(`platform_state.rs:1621-1645`)が Low の `HeuristicDefault=false` を記録し、他に観測が無ければ `MostRecentTrusted(HeuristicDefault)` で belief は閉になる。この規則(awase 起動後に作られたスレッドの IME は閉で始まる、`focus/thread_scope.rs`)は根拠のある推測なので、発動してよい。
   - 一方、**起動直後の placeholder は発動しない**。`desired_open` の初期値は `true`(`ime_model.rs` の `new()`、`desired_is_placeholder: true`)なので、観測が一切ない起動直後は `DesiredFallback=true` → エンジンが活性 → 発動しない。「情報ゼロで Space になる」経路は既に無い。
6. **ADR-188/191 の予測が belief を誤らせる場合**: 予測は**押した打鍵自身**には安全側に働く。無変換/変換に開く効果があると予測されるのは、IME の表にそのキーの行があるとき。そのときは決定2-4(ii) で `HasFunction` になり、発動しない。予測が外れて belief が OFF のまま残る(例: ADR-188 追記6 の `[key-effect-miss]`)と、それ以降の無変換が Space になる。これは Q1-3 の「belief が外れた場合」と同じ扱いで、計画0(iii) の計測対象に入る。新しい種類の害ではない。

### Q2. Low confidence の既定値だけのときも発動してよいか

- **許容してよい**。Q1-5 のとおり、情報ゼロの placeholder は既に発動しない。Low の閉を作るのは `assume_closed_for_new_thread`(根拠のある規則)と `FocusProbe`(Low)だけ。どちらの間も NICOLA は効いておらず、決定3 の「エンジンと一貫する」原則に従えば発動させるのが筋。
- 計画0(iii) で絞りが要ると分かったときの**最小の絞り**は、「`resolve_open_at` の base が `MostRecentTrusted(src)` で、src の confidence が Low(`HeuristicDefault`・`FocusProbe`)のときは発動しない」。`DecidedBy` を `InputContext` に 1 bit で運ぶだけで、`derive_actuating` のような別の判定は要らない。疑問1 にこの候補を書いておくこと。

### Q3. 閉じた指摘のうち再び開くもの

| ID | 判定 |
| --- | --- |
| R2-M3(InputRelay) | 閉じたまま。決定2-5(a) で除外を維持している |
| R2-S2(戻り待ち) | 閉じたまま。決定2-5(b) |
| R3-S2(遷移の前置) | 閉じたまま。決定4。`check_active_transition` は `emit_set_open=false`(`engine.rs:393-395`) |
| R4-S4(運び方) | 閉じたまま。`KeyDirectInputEffect` の運び方は変わらない |
| R3-M2(CI の入力先) | 形を変えて閉じている。計画2 の入力先を両方の窓にした。ただし (c) の composition の負の対照が「Standard の窓で」のままで、TSF の窓が入っていない(S2) |
| r1 B2(composition) | **重みが増した**。r3〜r5 では TSF の窓で発動しなかったので、フラグの有効範囲は Standard の窓だけ気にすればよかった。r6 で Chrome 等が対象になり、条件6 が TSF の窓で効くかが実害の大きさを左右する(S2) |

---

## Should

### S1. 廃止した `ime_off_confirmed` が残っている(マージ前に直す)

- 影響範囲の節(:162)「`state/` に `ime_off_confirmed` と `KeyDirectInputEffect` の純粋関数」。
- 計画1(:167)「非活性理由 × `ime_off_confirmed` × composing ×……」。
- どちらも r6 で廃止した変数。`thumb_space_blocked`(仮称、InputRelay・戻り待ち)に置き換えること。
- 対応表(r1 M1 :177、R2-M3 :202、R3-M1 :218)は当時の記録なのでそのままでよい。ただし表の見出しか末尾に「r6 で決定3 を置き換えたため、`derive_actuating`/`ime_off_confirmed`/`cannot_verify_real_ime_state` に関する行は失効」と一行あると、読み手が現行の決定と取り違えない。

### S2. 条件6 の「TSF のアプリでも効く」は未検証。CI で確かめる

- `ime_composition_active_now()` は `TSF_OBS.ime_composition_active` で、書き込むのは WinEvent の `EVENT_OBJECT_IME_SHOW`/`EVENT_OBJECT_IME_HIDE` だけ(`tsf/win_event_obs.rs:199-212`)。
  - **窓ごとではなくグローバル**なフラグで、別アプリの composition 窓の表示でも立つ。
  - `state/ime_decision_view.rs:53-55` は「MS-IME での信頼性は未検証」としている。
  - GJI が Chrome・Windows Terminal で composition 中にこのイベントを出すかは、コードからは分からない。
- 決定2-6 の追記を「TSF のアプリで効くかは未検証(計画2 で確かめる)」に直す。計画2(c) の「composition 中は入らない」の入力先に、TsfNative 相当の窓と Chrome を足すこと。立ちっぱなし(古い true)は安全側(発動しない)、立たない(false)は Q1-3 の変換の害になる。

### S3. 実害の見積もりに「IME が Space を解釈する」を足す

- Q1-3 のとおり、IME が実際に ON のときの Space は、変換(Composition)や全角スペース(Precomposition)として IME に解釈される。決定3 の「belief が外れたときの実害」と設定画面の注記(3)を「無変換が Space(IME によっては変換や全角スペース)になることがある」に直す。
- 計画2(d) の記録項目に「入力欄に何が入ったか(半角/全角スペース、変換)」を足す。

### S4. 疑問1 に最小の絞りの候補を書く

- Q2 の候補(base が `MostRecentTrusted` の Low ソースのときは発動しない)。placeholder が既に発動しない事実(`desired_open` の初期値は true)も併記し、「絞りが要るかは計画0(iii) で決める」と結ぶ。

### S5. 観測の機会の喪失

- 無変換を Space に変えると、その打鍵が IME に届かず、物理 IME キーを契機に走る観測(refresh・ADR-188 の窓内の直接読み)の機会が失われる。belief が外れている間に無変換を繰り返し押すと、外れが続きやすい。実害は小さい見込みだが、計画0(iii) の「belief が OFF・実際は ON」の持続時間を測るときに、この影響を区別できるようにする(Space 化を有効にした構成と無効の構成で比べる)。

---

## マージの判断

- 新規の Blocker/Must なし。**S1 を直せば docs として develop にマージしてよい**。S2〜S5 は ADR の記述の精度の問題で、実装着手(決定3b のゲート通過)までに直せばよい。
- ステータスは「提案(Opus r1〜r6、r6 は所有者の提案による決定3 の置き換え)。実装は決定3b のゲートで保留(報告者の回答待ち)」とし、index.md の行と review の補助資料の行(round6 まで)を合わせて更新すること。

---

## 確認に使ったコマンド(HEAD `df6ec3db`)

- `git show 01a98ccb:docs/adr/255-ime-off-thumb-key-space.md | grep -n "^6\. "`(項目6 の原文)
- `grep -n "cannot_verify\|ime_off_confirmed\|derive_actuating" docs/adr/255-ime-off-thumb-key-space.md`(古い参照)
- `grep -rn "ime_composition_active" crates/awase-windows/src`、`sed -n 180,220p crates/awase-windows/src/tsf/win_event_obs.rs`(composition フラグの書き込み元)
- `sed -n 300,330p crates/awase-windows-core/src/state/ime_model.rs`(`desired_open: true`・`desired_is_placeholder: true`)
- `sed -n 512,522p src/engine/engine.rs`(非活性時は pass_through)
- 未確認: GJI が Chrome・Windows Terminal で `EVENT_OBJECT_IME_SHOW` を出すか(S2)。Precomposition の Space が全角になるかは GJI の「スペースの入力」設定しだい(S3)。
