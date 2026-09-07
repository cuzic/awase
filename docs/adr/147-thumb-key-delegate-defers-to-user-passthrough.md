# ADR-147: 無変換/変換の `delegate_to_open_axis` は、ユーザーが明示的に選んだ単独タップ「パススルー」設定に道を譲る

## ステータス

**r0（起票、敵対的レビュー未実施）。** 対象は develop ブランチ。BUG-119として起票。

## 背景

### 既存の仕組み（要約）

`resolve_pending_thumb_as_single`（`src/engine/nicola_fsm.rs:1977-2038`）は、
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
  GJIへ届いていた。
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

GJI/MS-IMEのキー設定自動検出（GJI: `classify_thumb_key_ime_actions`、
MS-IME: `sync_ime_toggle_auto_detect`）が無変換/変換キーにIME ON/OFF/
トグルのいずれかを検出しており、**かつ**同じキーをNICOLA親指キーにも
設定しており、**かつ**設定画面の単独タップ設定で「常に送出する
（パススルー）」を明示的に選んでいるユーザーに限定される。

`always_suppress = true`（既定、「常に無視する」）を選んでいるユーザーには
影響しない——そちらは元々composing/idle問わずSuppressのため、delegateが
代わりに動くことは退行ではなくBUG-115の修正目的そのものである。Hiragana/
Katakanaキー（`ModeKeyConfig`という設定軸自体が存在しない、常に`None`）に
も影響しない。

## 決定

`resolve_pending_thumb_as_single`の判定2（`special.delegate_to_open_axis`）
に、判定3（`special.mode_key_config`）が既にPassthroughを選んでいる場合は
delegateを無効化するフィルタを追加する。

```rust
if let Some(open_axis_action) = special.delegate_to_open_axis.filter(|_| {
    !(special.injected_guarded_delegate && injected)
        && !special
            .mode_key_config
            .is_some_and(ModeKeyConfig::is_passthrough)
}) {
    // 従来どおり
}
```

`ModeKeyConfig::is_passthrough()`は「idle（非composing）時にPassthrough
かどうか」を返す（`fsm_types.rs:576-584`のdoc参照）。delegateが実際に
介入するのは`!composing`の分岐のみなので、判定すべきはまさに「idle側の
設定がPassthroughかどうか」であり、既存のヘルパーがそのまま使える——新規
ヘルパーを追加する必要はない。

`special.mode_key_config`が`None`（Hiragana/Katakana、あるいは
`muhenkan_vk`/`henkan_vk`が設定されていない場合）なら`is_some_and`は
`false`を返すため、この2キー以外の挙動は一切変わらない。

### なぜこの方式を選ぶか

1. **最小の変更で正確に意図を表現する。** 「ユーザーが明示的にパススルーを
   選んでいる」という条件を、既存の`ModeKeyConfig::is_passthrough()`を
   呼ぶだけで表現できる。新しい設定項目・新しいフィールドは不要。
2. **BUG-115の修正目的を保つ。** `always_suppress = true`（既定）の
   ユーザーには一切影響しない——delegateは引き続き「GJIが自力でIMEを
   ONにしてもawaseのbeliefが追随しない」問題を解消し続ける。退行するのは
   「ユーザーが明示的に別の対処法（パススルー）を既に選んでいる」という
   狭いケースのみで、そのケースでは元々delegateの助けを必要としていない
   （GJI自身が物理キーを見て意味論どおりに動く設計を、ユーザー自身が
   選んでいる）。
3. **`is_passthrough()`という既存の未使用ヘルパーの存在が、この設計判断が
   実装時に見落とされていたことを示す。** 新規ロジックの発明ではなく、
   既存の意図されていたであろう配線を復元するだけで直る。

### 検討した代替案

**代替案A（棄却）: delegateを`always_suppress`ユーザーにのみ適用する
設定項目を新設する。** 挙動としては採用案と同じだが、既存の
`ModeKeyConfig::is_passthrough()`をそのまま使えるにもかかわらず新しい
設定軸を増やすのは不要な複雑化。

**代替案B（棄却）: `classify_thumb_key_ime_actions`側（GJI検出）で
ユーザーのModeKeyConfigを見て検出自体を止める。** 検出（GJI設定の分類）
と適用（delegateとModeKeyConfigの優先順位）は別の関心事であり、検出結果
自体は「GJIが実際にそう設定している」という事実を表すため変える理由が
ない。MS-IME側（`sync_ime_toggle_auto_detect`）にも同じ問題があるため、
両方の呼び出し元を個別に直すより、消費側の`resolve_pending_thumb_as_
single`1箇所で直す方が合流点を増やさない（`fix-requires-evidence.md`の
「IME actuation合流点」の教訓）。

## 必須条件

1. **回帰テスト**: `resolve_pending_thumb_as_single`のユニットテスト
   （`src/engine/tests.rs`）に、`delegate_to_open_axis = Some(TurnOn)`
   かつ`mode_key_config`がPassthrough（`always_suppress=false`,
   `ignore_composing_guard=true`）の組み合わせで、`!composing`時に
   物理キーがPassthroughされ、delegateの`SetOpen`が発行**されない**
   ことを固定するケースを追加する。既存の「delegateが正しく発火する」
   テスト（`always_suppress=true`相当、既定設定）が引き続き緑であること
   も確認する。
2. **`docs/known-bugs.md`のBUG-119を「修正済み」に更新する**（本ADRの
   実装完了後）。
3. **`fix-requires-evidence.md`の「キー選択（IME ON/OFF に送る VK）」
   ファミリーに該当するため、上記1・2の両方を満たす**（テストのみ・
   記録のみの片方では不十分、他のfixで両方要求している前例に揃える）。
4. Hiragana/Katakanaキー（`mode_key_config: None`固定）の既存挙動が
   変わらないことをテストで確認する（`special.mode_key_config`が`None`
   の場合に`is_some_and`が`false`を返す既存のRust意味論に依存するのみ
   だが、既存のHiragana/Katakana向けdelegateテストが変更後も緑のままで
   あることを確認する）。

## 残存する既知の限界（対応せず記録のみ）

- 本ADRは「ユーザーが明示的にパススルーを選んでいる場合」に限定した修正
  であり、`InputContext::composing`が「候補ウィンドウの可視性」であって
  GJI自身のセッション状態と一致しないという、より広い設計上のギャップ
  自体は解消しない。パススルーを選んでいないユーザー（既定の
  `always_suppress=true`）にとっては、GJIが「Composition状態だが候補
  ウィンドウ非表示」の間に無変換単独タップをした場合、依然として
  delegateが優先される（本ADRの対象範囲外——このケースでは元々awase側が
  意味論を肩代わりする設計であり、ユーザーはGJI自身に処理させる選択を
  していないため）。
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
