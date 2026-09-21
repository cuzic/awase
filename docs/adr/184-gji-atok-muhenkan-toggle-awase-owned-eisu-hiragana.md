---
id: ADR-184
title: |-
  GJI(ATOKキーマッププリセット)の無変換/変換Toggleを、ADR-179決定2が
  既に持つ「Toggleはawase能動actuate・冪等キーはfollow」の仕組みへ
  実際に配線し直すだけの最小修正
summary: |-
  GJI(Google日本語入力)のキーマップをATOKプリセットにすると、無変換単独
  タップはIME ONのままひらがな⇔半角英数を切り替える（ユーザー実機確認、
  2026-09-19）。`classify_mode_key_ime_action`はこれを`ImeToggleKind::
  Toggle`と判定するが、`gji_thumb_key_ime_toggle`が既定OFFのため
  `resolve_delegate_to_open_axis`の`Toggle`分岐（ADR-179決定2で既に実装
  済み、awaseが能動的にactuateする）へ到達せず、生キーがGJIへそのまま
  中継されるだけになっている。本ADRは**この既存の分岐を実際に到達可能に
  する配線変更のみ**を行う。新しい型・軸の区別・較正ウィザード連携は
  作らない。理論的に想定しうる不整合（GJIの実挙動がopen軸かconv軸か等）
  は事前に設計で潰そうとせず、配線後に実機で確認し、実際に問題が
  出たものだけ個別に対応する方針とする（ユーザー判断、2026-09-19）。
status: |-
  **方針転換（2026-09-19）。設計を大幅に簡素化した。**
  ここまでopus-adversarial-consultをround1〜6まで回し、そのたびに
  発見された理論的な不整合（ATOK判定の情報源混在、`ImeToggleKind`への
  origin追加の不成立、MS-IMEレジストリ経路・ADR-176較正ウィザードとの
  相互作用等）に対して、そのつど新しい型・新しいフィールド・新しい
  区別を追加して対応しようとした結果、**設計がラウンドを追うごとに
  複雑化し続けていた**。ユーザーから「その場しのぎを積み増すのをやめ、
  シンプルな設計にしてほしい。多少の不具合が出てもよいので実機検証で
  判断する」と明確な指示があり、方針を転換した。

  **新方針**: 「Toggleと判定されたキーはawaseが能動actuateする、冪等
  （On/Off）なら生キーをそのまま通す」というADR-179決定2の原則は
  **既にコードにある**（`resolve_delegate_to_open_axis`、
  `nicola_fsm.rs:2211-2262`）。本ADRはこれを新設せず、**現状この分岐に
  到達できていない配線の穴を塞ぐだけ**にスコープを絞る。ATOK判定が
  本当にopen軸かconv軸か、MS-IMEレジストリ経路への影響、較正ウィザード
  との統合、InputRelayの動的判定精度——これらはすべて**実装前に理論で
  解決しようとしない**。配線を有効にした上で実機A/Bを行い、実際に
  観測された不具合だけをdocs/known-bugs/へ記録して個別に対処する。

  opus-adversarial-consult round1〜6のログ（`docs/adr/184-opus-review-
  round{1..6}.md`）は、検討の過程で発見した実コードの事実（行番号、
  ADR-182が既にマージ済みであること等）の記録として残すが、
  round5〜6で組み立てた型設計・別フィールド案は**採用しない**。
related_adr:
  - "ADR-183"
  - "ADR-179"
  - "ADR-181"
  - "ADR-182"
---

# ADR-184: GJI(ATOK)無変換/変換Toggleの配線を有効化する

## 症状

GJI（Google日本語入力）のキーマップをATOKプリセットにしていると、無変換
単独タップは「IME ON・ひらがな ⇔ IME ON・半角英数」を切り替える（IME
の開閉状態には触れない）。ユーザー実機で確認済み（2026-09-19）。

現状、awase はこの単独タップを**ただの生キーとして GJI へ中継するだけ**
で、awase 自身は何もしない（`resolve_pending_thumb_as_single` の
「`.yab` 配列定義外キーは `Key(vk_code)` へフォールバック」経路）。GJI
自身の内部処理に任せた結果、`idle-conv-check` の受動観測と絡んで
Engine の活性/非活性が不安定になる、という不具合が実際に起きている
（Passthrough 設定 `muhenkan_solo_tap_always_suppress=false` を使って
いるユーザーで発生。Passthrough は公式には非推奨設定だが、実際には
多くのユーザーが選択しており、直接の不具合報告もある）。

## 既存の仕組み（ADR-179決定2、実装済み）

`gji_charset_autodetect.rs::classify_mode_key_ime_action` は GJI の
キーマップ設定から、無変換/変換それぞれに `ImeToggleKind::On`/`Off`/
`Toggle` を判定する。この判定を消費する `nicola_fsm.rs::resolve_
delegate_to_open_axis`（2211-2262行、Passthrough設定時の腕）には、
既に次の分岐がある:

```rust
// nicola_fsm.rs:2244-2261（Some(_) 腕、Henkan/Muhenkan・Passthrough設定）
if matches!(open_axis_action, ShadowImeAction::Toggle) {
    if composing { return DelegateResolution::Fallthrough(None); }
    explicit_resolve()          // ← awase が能動的に actuate する
} else if composing {
    DelegateResolution::Fallthrough(None)
} else {
    // TurnOn/TurnOff: 辞退してModeKeyConfig側の生キー送出に委ねるが、
    // belief追随だけは今ここで確定させる。
    DelegateResolution::Fallthrough(Some(open_axis_action))   // ← 生キーをそのまま通す
}
```

**「Toggleならawaseが能動的にactuateする、冪等（On/Off）ならそのキーを
そのまま通す」という、ユーザーが求める設計はこの中に既にある。**

## 問題: この分岐に到達できていない

`gji_thumb_key_ime_toggle`（config.toml の隠しフラグ、既定 `false`）が
`Toggle` 判定を `muhenkan_delegate_to_open_axis`/`henkan_delegate_to_
open_axis` へ書き込む前段のゲートになっており、既定では書き込まれない
（`special.delegate_to_open_axis` が `None` のまま）。したがって上記の
`Toggle` 分岐は**現状誰にも到達されていない**（実機ログで
`Fallthrough(None)` を確認済み）。

## 決定

`gji_thumb_key_ime_toggle` の配線を、`muhenkan_solo_tap_always_
suppress=false`（Passthrough）を選んでいるユーザーに対して有効にする
（新しいフラグは作らない。Passthrough を選んだ時点で、ユーザーは既に
無変換/変換の生キーが外部へ渡ることを受け入れているため）。

これ以外の変更はしない。具体的には:

- `ImeToggleKind` の型・`resolve_delegate_to_open_axis` の `Toggle`
  分岐そのもの・`ShadowImeAction` は一切変更しない。
- ATOK 判定が指す軸（open か conv か）の区別、新しい型、別フィールドは
  作らない。
- MS-IME レジストリ経路・ADR-176 較正ウィザードとの統合は検討しない
  （既存の消費者は今回の配線変更の影響を受けない——`Toggle` 判定自体は
  既存の意味のまま、書き込まれる先だけが変わる）。

## 実機で確認すること（設計ではなく検証で判断する）

1. Passthrough 設定・ATOK プリセットで、無変換/変換単独タップが
   実際に `explicit_resolve()` 経路（awase の明示 actuate）へ入るか。
2. その actuate（`ShadowImeAction::Toggle` の open 軸反転）が、実際の
   症状（Engine の不安定化）を解消するか、それとも別の見た目の不具合
   （IME が意図せず閉じる等）を生むか。
3. 2 で新しい不具合が出た場合は、`docs/known-bugs/BUG-NNN.md` に症状・
   再現手順・実機ログを記録し、その具体的な不具合単位で個別に対処する
   （設計を先回りして複雑化しない）。
4. 変換（Henkan）側も同じ分岐を通るため、無変換と合わせて実機確認する。

**症状が出たときの当たりどころ（opus round6より、設計変更ではなく
観察の着眼点として記録）**:
- 無変換/変換は `injected_guarded_delegate: false`
  （`gji_charset_autodetect.rs:1005/1013`）——BUG-14 ガードが
  現状この2キーには効いていない。配線を有効化して初めて実効化する
  経路なので、外部プロセス由来の注入イベントで誤発火するような
  挙動が出たら、まずここを疑う。
- GJI から他アプリへフォーカスが移ったとき、GJI 由来の
  `delegate_to_open_axis` 設定値が残留し、無関係なアプリで無変換
  単独タップが意図せず actuate する、という挙動が出たら、GJI 離脱時の
  クリーンアップ漏れ（BUG-115で過去に一度実際に踏んだ穴と同型）を疑う。

## 参考: 検討したが採用しなかった経緯

opus-adversarial-consult round1〜6で、ATOK判定の情報源混在（CUSTOM
キーマップで無変換に本物のIME ON/OFFを割り当てているユーザーとの
区別）、`ImeToggleKind`への origin 追加が型的に成立しないこと、
MS-IMEレジストリ経由の別の書き手がいること、ADR-176較正ウィザードの
永続化スキーマへの影響、`resolve_pending_thumb_as_single`の戻り値型
拡張（`ModeKeyRequest`）等、多数の理論的な論点が発見された
（`docs/adr/184-opus-review-round1.md`〜`round6.md`）。これらは実在の
懸念だが、**実機で問題が確認されるまでは対応しない**——先回りして
設計を複雑にすることが、このリポジトリで繰り返されてきた「対症療法の
積み重ね」パターンそのものだとユーザーが指摘したため（ADR-178が
まさにこの種の積み重ねを撤去している最中のブランチである）。
