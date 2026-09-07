# ADR-147: 無変換/変換の `delegate_to_open_axis` は、ユーザーが明示的に選んだ単独タップ「パススルー」設定に道を譲る

## ステータス

**設計確定・実装済み（Opus敵対的レビュー2ラウンドで収束、Blocker1件・
Major2件を検出・解消、Should-fix1件・Minor複数件を反映）。回帰テスト
（必須条件1）追加済み、`cargo test --lib`（995件）・Windowsターゲット
`cargo check -p awase -p awase-windows`・`architecture_guard`/
`layer_boundary_guard`/`gji_charset_autodetect`各テストとも緑。実機ソーク
未実施。** 対象は develop ブランチ。BUG-119として起票。

**r1での変更点（r0からの差分）**: r0は`delegate_to_open_axis`の辞退を
方向（TurnOn/TurnOff/Toggle）を問わず一律に行う案だったが、Opusレビューで
「`kp_stage_shadow_ime_toggle`の所有権判定（`delegate_owns_mode_key_
shadow_toggle`）が`mode_key_config`を一切見ないため、TurnOff/Toggle方向の
delegateが辞退すると誰もbeliefを追随しない『二重の空振り』を新規に作る」
というBlockerが判明した（下記「消費点と所有権のマトリクス」参照）。r1では
**辞退をTurnOn方向に限定**し、TurnOff/Toggle方向は既存どおりdelegateが
優先されるよう修正した。

## 背景

### 既存の仕組み（要約）

`resolve_pending_thumb_as_single`（`src/engine/nicola_fsm.rs:1977`起点、delegate分岐は`:2020-2038`）は、
無変換/変換キーを`left_thumb_key`/`right_thumb_key`（NICOLA親指キー）にも
設定しているユーザーに対して、単独タップ（同時打鍵が不成立）確定時の挙動を
次の優先順位で決める。

1. `special.dedicated_fn_key`（専用Fnキー、隠し設定、ADR-091 §D3.2）
2. `special.delegate_to_open_axis`（GJI/MS-IMEのキー設定自動検出に基づく
   IME open軸への肩代わり、ADR-092決定D Step4b）——`Some`かつ
   `!composing`（`InputContext::composing`、後述）なら、**物理キーを
   完全にSuppressし**（`SmallVec::new()`）、代わりに`ime_open_requested`
   経由で`Effect::Ime(SetOpen)`を発行する（`engine.rs::apply_ime_open_
   request`）
3. `special.mode_key_config`（`ModeKeyConfig`、設定画面の「無変換キー
   単独タップ」で選ぶ Suppress/Passthrough。`muhenkan_solo_tap_always_
   suppress`/`muhenkan_solo_tap_ignore_composing_guard`から`ModeKeyConfig::
   from_legacy_bools`で導出）

2の判定は`composing`引数を見るが、これは`InputContext::composing`——供給元
`crate::tsf::observer::ime_composition_active_now()`——であり、doc曰く
「IME composition **window が可視**かどうか」（`EVENT_OBJECT_IME_SHOW`/
`HIDE`契機で更新）。GJI/Mozcのセッション状態（DirectInput/Precomposition/
Composition/Conversion...）そのものではなく、**候補ウィンドウが実際に
画面上に表示されているか**だけを表す、より狭い概念である。

### BUG-119: delegateがユーザーの明示的なパススルー設定を無条件に上書きする

2026-09-05に追加された`classify_thumb_key_ime_actions`
（`crates/awase-windows/src/gji_charset_autodetect.rs`、BUG-115対策、
v1.19.0で出荷）は、GJIのカスタムキーマップ（`custom_keymap_table`）から
無変換/変換キーに割り当てられたIME ON/OFF/トグル意味論を自動検出し、
`muhenkan_delegate_to_open_axis`/`henkan_delegate_to_open_axis`
（上記2）へ自動的に配線する。この配線自体はBUG-115（GJIが自力でIMEを
ONにしてもawaseのbeliefが追随せず最初の1文字がローマ字化する問題）を
解消する正しい機能だが、**ユーザーが設定画面で明示的に「常に送出する
（パススルー）」（`ModeKeyConfig::is_passthrough() == true`）を選んで
いるかどうかを一切参照せずに2を1・3より優先してしまう**。

「パススルー」は、ユーザーが「無変換キー本来の機能を使いたい」——今回の
実例では「GJI自身のカスタムキーマップに無変換キーの意味論（DirectInput
→IMEOn、Composition→確定/Commit等）を完全に委ね、awaseは一切介入しない」
——という意図で明示的に選ぶ設定である。ところがdelegateが有効になった
状態でこの設定を選んでいても、`!composing`（候補ウィンドウ非表示）の間は
物理キーが常にSuppressされ、GJI自身がその物理キーを見る機会が構造的に
失われる。結果:

- v1.18.0まで（`classify_thumb_key_ime_actions`が存在せず`delegate_to_
  open_axis`が常に`None`だったGJIユーザー）: 単独タップは`mode_key_config`
  のみで決まり、パススルー設定どおりに生の`VK_NONCONVERT`/`VK_CONVERT`が
  GJIへ届いていた。**ただし、この時点でも`muhenkan_shadow_override`
  （ADR-141で新設）や`ImeKeyKind::from_vk`のVK_CONVERT/VK_NONCONVERT対応は
  存在せず、`kp_stage_shadow_ime_toggle`によるbelief追随は一切無かった**
  ——GJIが自力でIMEをONにしてもawaseのbeliefが追随しない欠落そのものは
  当時から存在しており、それがBUG-115として別途報告されていた。本ADRは
  「v1.18.0への巻き戻し」ではなく、BUG-115の修正（belief追随）を保った
  ままパススルーユーザーの物理キー配送も復元することを狙う。
- v1.19.0以降: GJIのカスタムキーマップにDirectInput行の検出があると、
  候補ウィンドウが非表示である間（GJI自身は「Composition」状態で確定前の
  かな入力中であっても、変換候補ウィンドウを明示的に呼び出すまでは非表示
  のままであることが多い）、常にdelegateに奪われ、パススルー設定が機能
  しなくなる。ユーザーがGJI側で`Composition→Commit`（確定）を設定していても、
  その物理キーがGJIに届かないため確定が発火しない。

**`ModeKeyConfig::is_passthrough()`（`src/engine/fsm_types.rs:582`、doc:
「非composing（idle）時に単独タップが素通し（`GuardAction::Passthrough`）
か」）という、まさにこの判定に使えるヘルパーが既に定義されているが、本番
コードのどこからも呼び出されていない**（`grep -rn "is_passthrough(" src/`
で確認、ユニットテスト以外に呼び出し箇所ゼロ）。

### 影響範囲

GJI/MS-IMEのキー設定自動検出が無変換/変換キーにIME ON/OFF/トグルのいずれか
を検出しており、**かつ**同じキーをNICOLA親指キーにも設定しており、**かつ**
設定画面の単独タップ設定で「常に送出する（パススルー）」を明示的に選んで
いるユーザーに限定される。検出元は`classify_mode_key_ime_action`
（`crates/awase-windows/src/gji_charset_autodetect.rs:296-`）が扱う4つの
独立したソースがある（優先順位順）:

1. `overlay_keymaps`に`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`（`:145-152`）
   ——**無変換→`Off`・変換→`On`**（状態非依存で固定）。GJIの比較的一般的な
   overlay設定であり、本ADRのBlocker（後述）の直接の当事者になりうる
   （無変換側はTurnOff方向のため、本ADRの修正では救われない）。
2. `session_keymap == CUSTOM`の`custom_keymap_table`——BUG-119の元報告
   （`DirectInput 無変換 IMEOn`）はこちら。
3. `session_keymap == ATOK`——Henkan/Muhenkan双方`Toggle`（opt-in設定
   `gji_thumb_key_ime_toggle`が必要）。
4. MS-IMEレジストリ（`KeyAssignmentMuhenkan`/`Henkan`、
   `crates/awase-windows/src/runtime/message_handlers.rs:841-847`、
   `sync_ime_toggle_auto_detect`）。

`always_suppress = true`（既定、「常に無視する」）を選んでいるユーザーには
影響しない——そちらは元々composing/idle問わずSuppressのため、delegateが
代わりに動くことは退行ではなくBUG-115の修正目的そのものである。Hiragana/
Katakanaキー（`ModeKeyConfig`という設定軸自体が存在しない、常に`None`）に
も影響しない。

**本ADRの修正が実際に適用されるのはTurnOn方向のdelegateのみ**（後述
「決定」参照）。したがって上記1（overlay）の無変換側や3（ATOK、Toggle）は
本ADRの対象外のまま残る——詳細は「残存する既知の限界」参照。

## 消費点と所有権のマトリクス（r1で追加、Blocker対応）

`*_delegate_to_open_axis`（`muhenkan_delegate_to_open_axis`/
`henkan_delegate_to_open_axis`）には**3つの独立した消費点**がある。
r0はこのうち1のみを見て「1箇所で直せば十分」と誤って結論していた
（後述の「棄却した代替案」参照）。

| # | 場所 | 用途 | 評価タイミング |
|---|---|---|---|
| 1 | `resolve_pending_thumb_as_single`（`src/engine/nicola_fsm.rs:2020`） | 単独タップ確定時の実際の出力決定（本ADRの修正対象） | エンジンPhase 3、単独タップと確定した**後** |
| 2 | `mode_key_delegate_owns_shadow_toggle`（`crates/awase-windows/src/runtime/mod.rs:1440-1453`）→`delegate_owns_mode_key_shadow_toggle`（`crates/awase-windows/src/gji_charset_autodetect.rs:464-487`） | `kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs:1150`）の所有権判定。`true`ならshadow-toggle側はbelief書き込み・actuationを丸ごとスキップする | `build_input_context`/エンジン`on_input`より**前**、単独タップか同時打鍵かがまだ分からない時点 |
| 3 | `turn_on_direction`（`runtime/key_pipeline.rs:1277-1279`） | shadow-toggleのno-op分岐で、stale `ObservedEisu`救済の方向判定に使う補助情報 | 2と同じタイミング |

### なぜ2への対称な修正（`!is_passthrough`を追加）が使えないか

2の判定は**エンジンが単独タップか同時打鍵かを判定するより前**に走る
ため、「ユーザーがパススルーを選んでいる」という情報を2に渡して
delegateの所有権を降ろすと、**通常のNICOLA同時打鍵（チョード入力）の
全打鍵**で2が「delegateは所有しない」と誤判定し、`kp_stage_shadow_ime_
toggle`が`PhysicalImeKey`意図に昇格させてbeliefを書き換え、TurnOff方向
なら実IMEまでOFFにしてしまう（[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md#なぜこの方式を選んだか)
が「`TurnOff`方向は belief 書き込みに加えて能動的actuationを行う」と
明記する経路、実体は`key_pipeline.rs:1345-1408`）。この`&& effective_
open()`ゲート自体が、まさに「チョード入力中に誤発火させない」ための
ものであり（`key_pipeline.rs:1138-1149`のC1コメント参照）、2を変えると
このゲートの前提そのものを壊す。**2は変更しない。**

### 3への影響

3は1が辞退した後も`muhenkan_delegate_to_open_axis()`を素直に読むため、
`TurnOn`方向なら「delegateが発火する前提」でEisu救済を走らせる。実際に
IMEをONにするのはGJI自身（パススルーされた生キー経由）であり結果は
ほぼ同じだが、「delegateが発火した」という前提と実際の帰属が食い違う。
実害は無いと判断し、対応せず記録のみに留める（下記「残存する既知の限界」）。

## 決定

**辞退の対象を`TurnOn`方向のdelegateに限定する。** `resolve_pending_
thumb_as_single`の判定2（`special.delegate_to_open_axis`）に、
判定3（`special.mode_key_config`）が既にPassthroughを選んでおり、**かつ**
delegateの方向が`TurnOn`である場合にのみdelegateを無効化するフィルタを
追加する。

```rust
if let Some(open_axis_action) = special.delegate_to_open_axis.filter(|action| {
    !(special.injected_guarded_delegate && injected)
        && !(matches!(action, crate::types::ShadowImeAction::TurnOn)
            && special
                .mode_key_config
                .is_some_and(ModeKeyConfig::is_passthrough))
}) {
    // 従来どおり
}
```

`TurnOn`方向に限定する理由（上記マトリクスの2への影響が実害を生まない
ことの根拠）:

- **belief OFF（`effective_open() == false`）の間**、2の`delegate_owned`
  は`mode_key_delegate_owns_shadow_toggle(vk) && effective_open()`の
  `&& effective_open()`により**方向を問わず常にfalse**になる。つまり
  shadow-toggleは1の辞退有無に関わらず常にbeliefを追随する——BUG-118/C2
  はこの`effective_open()`ゲート自体が原因で別の問題（delegateがOFF中に
  一切発火しない）を起こしていたが、逆に言えば「belief OFF中は2が
  delegateに委ねることは無い」ため、1が辞退してもbeliefが孤立しない。
- **belief ON（`effective_open() == true`）の間**、1がTurnOn方向で辞退し
  生キーをGJIへ渡しても、GJI自身のTurnOn相当の処理は（IMEが既にONなら）
  通常no-opであり、実IMEもbeliefも共に`true`のまま食い違わない
  （`key_pipeline.rs:1235`の冪等分岐は`delegate_owned`が`true`の間は
  方向を問わず常に成立するため、TurnOn/TurnOffを区別する論拠には
  ならない——区別の根拠は上記のGJI側の実際の処理内容のみ）。
- 対照的に`TurnOff`/`Toggle`方向は、belief ON中に発火すると**実際に
  状態を反転させる**——2が「delegateが処理する」と誤信して身を引いた
  まま1も辞退すると、GJI自身が生キーでIMEをOFFにする一方awaseの
  beliefは`true`のまま取り残される（「消費点と所有権のマトリクス」の
  Blocker）。したがってこの2方向は**辞退の対象から除外し、既存どおり
  delegateに委ねる**（=本ADR適用後もBUG-119はこの2方向×パススルーの
  組み合わせでは未修正のまま——「残存する既知の限界」参照）。

`ModeKeyConfig::is_passthrough()`は「idle（非composing）時にPassthrough
かどうか」を返す（`fsm_types.rs:576-584`のdoc参照）。delegateが実際に
介入するのは`!composing`の分岐のみなので、判定すべきはまさに「idle側の
設定がPassthroughかどうか」であり、既存のヘルパーがそのまま使える——新規
ヘルパーを追加する必要はない。`composing == true`側では元々delegateが
`if !composing`で素通りして`mode_key_config.composing`に委ねられて
いたため、`.filter()`に置いても composing 時の挙動は変わらない。

`special.mode_key_config`が`None`（Hiragana/Katakana、あるいは
`muhenkan_vk`/`henkan_vk`が設定されていない場合）なら`is_some_and`は
`false`を返すため、この2キー以外の挙動は一切変わらない。

### なぜこの方式を選ぶか

1. **消費点2・3を変更せずに済む。** `TurnOn`方向に限定することで、
   `kp_stage_shadow_ime_toggle`の所有権判定（消費点2）を一切変更する
   必要がなくなり、上記Blockerが構造的に発生しない。
2. **BUG-115の修正目的を保つ。** `always_suppress = true`（既定）の
   ユーザーには一切影響しない——delegateは引き続き「GJIが自力でIMEを
   ONにしてもawaseのbeliefが追随しない」問題を解消し続ける。
3. **BUG-119の元報告（`DirectInput→IMEOn`）を正確に救う。** 元報告は
   `TurnOn`方向のdelegateであり、本方式でそのまま解消する。

### 検討した代替案

**方式2（r1レビューで提示、棄却）: delegate辞退時にactuationを伴わない
belief追随専用のワンショットシグナルを新設する。** 単独タップ確定時
（同時打鍵と区別が付いた後）に「actuateしないがbeliefだけ合わせる」
意図を`ime_open_requested`とは別チャネルで発行し、`TurnOff`/`Toggle`
方向でも安全に辞退できるようにする案。構造的にはより正しく、`TurnOff`/
`Toggle`方向のBUG-119も解消できるが、新しいIME actuation合流点を
1つ増やす（`fix-requires-evidence.md`が警告する「合流点を増やさない」
方針に反する）ため、TurnOn限定で足りる現状は採用せず、実機でTurnOff/
Toggle方向のパススルー要望が実際に出た時点で改めて検討する。

**方式3（r1レビューで提示、棄却）: パススルー送出後にIME refreshを
スケジュールする。** `VK_CONVERT`/`VK_NONCONVERT`は`may_change_ime`
対象外（`vk.rs:1000-1008`のテストが固定）のため観測経路が無く、
Imm32観測可能アプリでしか救えない上、単独では不十分（方式2の補助にしか
ならない）。

**代替案A（棄却）: delegateを`always_suppress`ユーザーにのみ適用する
設定項目を新設する。** 挙動としては採用案と同じだが、既存の
`ModeKeyConfig::is_passthrough()`をそのまま使えるにもかかわらず新しい
設定軸を増やすのは不要な複雑化。

**代替案B（棄却、r1で理由を訂正）: `classify_thumb_key_ime_actions`側
（GJI検出）でユーザーのModeKeyConfigを見て検出自体を止める。** 検出
（GJI設定の分類）と適用（delegateとModeKeyConfigの優先順位）は別の
関心事であり、検出結果自体は「GJIが実際にそう設定している」という事実を
表すため変える理由がない。**r0時点の棄却理由（「消費側の1箇所で直す方が
合流点を増やさない」）は`fix-requires-evidence.md`の教訓を誤読していた
——同資料が警告するのは「1箇所だけ直して満足せず実際の呼び出し経路を
すべて洗い出せ」であって「消費点が1つである」という主張ではない
（実際には上記の通り消費点は3つある）。** 正しい棄却理由は、GJI/MS-IME
の検出コードは2箇所（`gji_charset_autodetect.rs`と`message_handlers.rs`）
に分かれており、検出側で止めるとこの2箇所を両方直す必要がある一方、
消費点1（`resolve_pending_thumb_as_single`）は両ソースの合流後の唯一の
出力決定点であり、ここで1箇所直せば両ソースに対称に効く、という点に
ある。

## 必須条件

1. **回帰テスト**（`src/engine/tests.rs`、`resolve_pending_thumb_as_
   single`のユニットテスト）。以下の組み合わせをすべて固定する
   （r1でMajor指摘を受けて拡充、方向とキーの両軸を網羅）:
   - `delegate_to_open_axis = Some(TurnOn)` × `mode_key_config`が
     Passthrough（`always_suppress=false`, `ignore_composing_guard=true`）
     × `!composing` → 物理キーがPassthroughされ、delegateの`SetOpen`が
     発行**されない**（本ADRが直す退行そのもの）。
   - `delegate_to_open_axis = Some(TurnOff)` × 同じPassthrough設定 ×
     `!composing` → **delegateが従来どおり発火し**、物理キーはSuppress
     される（TurnOn限定であることの固定、Blockerの再発防止）。
   - `delegate_to_open_axis = Some(Toggle)` × 同じPassthrough設定 →
     上記TurnOffと同様、delegateが従来どおり発火する。
   - 上記3ケースを**変換（henkan）側**でも対称に追加する（既存の
     `src/engine/tests.rs:7621-7628`が同種の指摘で追加された前例に倣う。
     無変換だけでなく変換も検証しないと片側だけ直る事故が起きうる）。
   - `mode_key_config`が非対称値`from_legacy_bools(false, false)`
     （idle=Passthrough、composing=Suppress）の場合でも、`TurnOn`方向
     delegateが同様に辞退することを固定する（設定GUIからは到達しないが
     `config.toml`手編集で到達しうる値）。
   - 既存の「delegateが正しく発火する」テスト（`always_suppress=true`
     相当、既定設定、`src/engine/tests.rs:7402,7437,7476,7544,7579,
     7605,7628`等）が引き続き緑であることを確認する
     （`is_passthrough()==false`のため無改造で通るはずだが、実際に確認する）。
2. **`docs/known-bugs.md`のBUG-119を「修正済み（TurnOn方向のみ、
   TurnOff/Toggle方向は既知の限界として残存）」に更新する**（本ADRの
   実装完了後、「修正済み」と無条件に書かない——範囲限定を明記する）。
3. **`fix-requires-evidence.md`の「キー選択（IME ON/OFF に送る VK）」
   ファミリーに該当するため、上記1・2の両方を満たす**（テストのみ・
   記録のみの片方では不十分、他のfixで両方要求している前例に揃える）。
4. Hiragana/Katakanaキー（`mode_key_config: None`固定）の既存挙動が
   変わらないことをテストで確認する（`special.mode_key_config`が`None`
   の場合に`is_some_and`が`false`を返す既存のRust意味論に依存するのみ
   だが、既存のHiragana/Katakana向けdelegateテストが変更後も緑のままで
   あることを確認する）。
5. **消費点2・3（上記マトリクス）には変更を加えないため、
   `gji_charset_autodetect.rs`のdecision-tableテスト
   （`gji_detection_to_application_pipeline_decision_table`）・
   `delegate_owns_mode_key_shadow_toggle`関連のテストは無改造のまま
   緑であることのみ確認すれば足りる**（本ADRの方式ではこれらの関数の
   シグネチャ・挙動を一切変えないため、新規テスト追加は不要——変更した
   場合は方式選択の前提が崩れている）。
6. **実装時、`resolve_pending_thumb_as_single`のフィルタ箇所に「なぜ
   TurnOn限定なのか」を説明するコメントを置く**（Opusレビュー
   Should-fix）。この設計の正しさは**別クレート**（`crates/awase-windows`）
   の`delegate_owns_mode_key_shadow_toggle`が`mode_key_config`を一切
   見ない、という`awase`コア（プラットフォーム非依存）からは見えない
   外部の不変条件に依存している。コメントが無いと、将来「TurnOn限定は
   保守的すぎる、TurnOff/Toggleにも広げよう」という一見自然な変更が
   Blockerをそのまま復活させる。`gji_charset_autodetect.rs:479-485`の
   `muhenkan_dedicated_fn_key_configured`に関する同種のコメント
   （コア`awase`クレートはOS非依存を保つ必要があるため、コード参照では
   なく散文＋本ADRへのリンクで書く）に揃える。

## 残存する既知の限界（対応せず記録のみ）

- **`TurnOff`/`Toggle`方向のdelegate×パススルー設定の組み合わせは
  未修正のまま残る**（r1でBlocker判明、方式2で解消可能だが本ADRのスコープ
  外——上記「検討した代替案」参照）。具体例: GJIのoverlay設定
  `OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`（無変換→`Off`）を使っており、
  かつ無変換をパススルーに設定しているユーザーは、本ADR適用後も無変換
  単独タップがdelegateに奪われ続ける（本ADR適用前と同じ、退行ではないが
  未解消）。ATOKプリセット（`Toggle`、opt-in）も同様。
- 本ADRは「ユーザーが明示的にパススルーを選んでいる場合」に限定した修正
  であり、`InputContext::composing`が「候補ウィンドウの可視性」であって
  GJI自身のセッション状態と一致しないという、より広い設計上のギャップ
  自体は解消しない。パススルーを選んでいないユーザー（既定の
  `always_suppress=true`）にとっては、GJIが「Composition状態だが候補
  ウィンドウ非表示」の間に無変換単独タップをした場合、依然として
  delegateが優先される（本ADRの対象範囲外——このケースでは元々awase側が
  意味論を肩代わりする設計であり、ユーザーはGJI自身に処理させる選択を
  していないため）。
- **（r1補足、PRレビューで訂正）`TurnOn`分類は構造的に「全状態でOFFに
  ならない」ことを保証する。** 当初「`Precomposition`行が`CancelAndIMEOff`
  であってもDirectInput行だけを見て`TurnOn`に誤分類しうる」という懸念を
  記録していたが、実コード確認の結果これは起きない。`classify_and_push`
  （`crates/awase-gji-config/src/keymap.rs:279-284`）は`off_statuses`が
  空の場合に限り`on`バケットへ積む。`CancelAndIMEOff`は`command.rs:72`で
  `ImeOff`にエイリアスされる（BUG-115対策）ため、DirectInput→IMEOn +
  Precomposition→CancelAndIMEOffの組み合わせは`on_statuses`/
  `off_statuses`が両方非空になり、`Toggle`に分類される（`On`にはならない）。
  つまり**GJI検出由来で`TurnOn`と分類されたキーは、定義上どの状態にも
  OFF相当のバインドを持たない**——これが「belief ON中に辞退してもbelief
  乖離が起きない」ことのより強い（no-opに頼らない）根拠になる。MS-IME側
  （`crates/awase-windows/src/msime_key_assignment.rs:244-253`）も無変換は
  `TurnOff`/`Toggle`のみを取り、`TurnOn`は変換の`KeyAssignmentHenkan=1`
  （IME ON固定）のみのため同様に安全。
- 消費点3（`turn_on_direction`、Eisu救済の方向判定）は、辞退後も
  `muhenkan/henkan_delegate_to_open_axis()`を素直に読み続けるため
  「delegateが発火した」という前提で方向を決めるが、TurnOn方向の辞退時は
  実IME側もno-op相当（上記「決定」参照）のため実害なしと判断し、対応せず
  記録のみ。
- MS-IME側（`sync_ime_toggle_auto_detect`）が同じdelegateフィールドに
  書き込む経路も、本ADRの修正（`resolve_pending_thumb_as_single`という
  単一の消費点）で自動的に対象になるが、MS-IME固有のレジストリキー割り当て
  パターンでの実機ソークは未実施。

## 関連

BUG-119（本ADRの対象）、BUG-115（`classify_thumb_key_ime_actions`の
追加元、ADR-092決定D Step4b）、BUG-118／[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)
（同じdelegate機構のTurnOn方向構造的到達不能、C2——本ADRとは独立した別欠陥）、
[ADR-135](135-generic-thumb-key-ime-toggle-delegate.md)（Hiragana/Katakana
版のdelegate機構）。
