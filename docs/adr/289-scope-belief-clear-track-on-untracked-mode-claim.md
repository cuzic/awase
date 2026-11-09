---
id: ADR-289
title: |-
  belief-v2の明示モード主張がtrackを伴わないとき、窓ごとの追跡中変換モード(key_track.conv)を古い値のまま残さずクリアする(BUG-229)
summary: |-
  `scope_belief.rs::on_explicit`は`track`引数がSomeのときだけ`row.track`を書き換える。ADR-245の半角英数復元・トグルの出入り
  (`InputModeApplied{UserHalfWidthAlnumToggle|PanicReset}`)が作る`ScopeEvent::Explicit`は`track: None`なので、
  これらの明示操作でmodeを書き換えても、窓ごとに持続する`row.track.conv`(ADR-282の設計)は古い値のまま残る。
  直後のテーブルキー予測がこの古いconvを入力にして効果を予測し、`track: Some(prediction.track)`付きで改めて古いモードを
  explicit主張してしまい、170msの確定期限が直後の本物の観測を棄却する(BUG-229、2026-11-09実機)。
  `on_observed`が受理した観測で`row.track.conv = None`へ戻す(:541-543)のと同じ扱いを、該当する`scope_event_of`の
  マッピング側(`UserHalfWidthAlnumToggle`/`PanicReset`の変換、:731-741)にも入れ、`track: None`の代わりに
  `track: Some(KeyTrack{conv: None, ..view.track})`を渡す。
status: |-
  起草(2026-11-09)。Opusレビュー前。実装・実機確認とも未着手。
related_adr:
  - "ADR-282"
  - "ADR-245"
  - "ADR-244"
---

# ADR-289: belief-v2の明示モード主張は、trackが無いとき追跡中convをクリアする(BUG-229)

## 背景

[BUG-229](../known-bugs/BUG-229.md)の実機報告(dragonflyg4、2026-11-09T13:13:19 UTC):
半角英数のまま離れたChrome窓に戻ったところ、ADR-245の「戻った窓の最初の打鍵→半角英数トグルを復元」が
正しくbeliefをkanaへ収束させEngineをONにしたにもかかわらず、同じ打鍵(Backspace)の処理中に70μs後、
`[belief-v2]`が`mode=eisu basis=z-explicit`へ再び食い違いを起こし、43ms後に届いた本物の
`[mozc-shm]`の`"tsf_mode":"hiragana"`を棄却したうえで、さらに43ms後にEngineが再度OFFになった。
期待動作は「IME ON なら Engine ON」(利用者の報告どおり)で、実IMEはhiraganaのままだったため、
これは明確な不具合である。

Opusサブエージェントによるコード調査(commit `6237a0601`、読み取り専用)で、原因を
`crates/awase-ime-core/src/state/scope_belief.rs`の以下の非対称に特定した:

- `on_observed`(観測の受理)は、受理したモード主張で`row.track.conv = None`に戻す(:541-543、
  コメント「観測が来たので、変換モードの追跡は観測へ戻す」)。
- `on_explicit`(明示操作)は、`track`引数がSomeのときだけ`row.track`全体を上書きする(:369-371)。
  `track`がNoneの呼び出し元は、`row.track`(stage・convとも)を一切変更しない。
- `scope_event_of`の`ImeEvent::InputModeApplied{strategy: UserHalfWidthAlnumToggle | PanicReset, ..}`
  マッピング(:731-741)は`mode: Some(m)`を渡す一方、`track: None`で呼ぶ。ADR-245の復元・半角英数
  トグルの出入りはこの経路を通るため、**modeをexplicitに書き換えても、窓ごとに持続する
  `row.track.conv`(ADR-282の設計、フォーカス切替をまたいで保持)は古い値のまま残る**。
- 直後にテーブルキー(表に載っているキー。文字キーに限らずBackspace等も含む)が来ると、
  `scope_event_of_prediction`(:640-657)は`view.track`(=この古いconv)を入力に予測し、
  `track: Some(prediction.track)`付きで改めてexplicit主張する。これが170msの確定期限
  (`settle_ms`)を立て、直後に届く新しい観測(hiragana)を棄却する。

[BUG-192](../known-bugs/BUG-192.md)/ADR-244 M-2は近縁の問題(`KeyTrack::without_conv_while_half_width_alnum`、
トグルが**アクティブな間**だけ追跡中convを捨てる)を修正済みだが、今回のBUG-229は「トグルを抜けた・
復元した直後」の別の抜けで、M-2の対象外。

## 決定

### D1. 修正位置: `on_explicit`の汎用契約ではなく、`scope_event_of`の該当マッピングだけを直す

`ScopeEvent::Explicit.track`は列挙型の定義時点で「`None`は変えない」と明記されたフィールドである
(scope_belief.rs:148)。この汎用契約自体は他の呼び出し元(`scope_event_of_prediction`の
`track: Some(prediction.track)`、`UserImeSetIntent`/`PanicReset`の`track: Some(KeyTrack{stage: Stage::None, ..view.track})`)
が前提にしており、`on_explicit`側で「modeがSomeならtrackがNoneでも強制的にconvを消す」という
特殊扱いを足すと、契約の意味が書き換わり他の呼び出し元の意図まで読み直す必要が生じる(§却下した案 参照)。

代わりに、問題のあるマッピング(:731-741)**だけ**を直す。`scope_event_of`は`view: &ScopeView`を
既に引数に持つため、その場で`track: Some(KeyTrack { conv: None, ..view.track })`を作れる
(`stage`は`view.track.stage`をそのまま引き継ぎ、`conv`だけを`None`に落とす)。

```rust
ImeEvent::InputModeApplied {
    mode,
    strategy:
        InputModeApplyStrategy::UserHalfWidthAlnumToggle | InputModeApplyStrategy::PanicReset,
    result: InputModeApplyResult::Applied,
} => mode_class_of(*mode).map(|m| ScopeEvent::Explicit {
    open: None,
    mode: Some(m),
    // 半角英数トグルの出入り・復元は実IMEの変換モードを確定させる行為なので、窓ごとに持続する
    // 追跡中conv(ADR-282)は古い値のまま残さずクリアする(BUG-229)。stageは引き継ぐ。
    track: Some(KeyTrack { conv: None, ..view.track }),
    at_ms,
}),
```

### D2. `on_explicit`自体は変更しない

D1の結果、`track`に常に具体値(`Some`)が渡るようになるため、`on_explicit`の
「`track`がSomeのときだけ上書きする」という既存ロジック自体は変更不要。`ScopeEvent::Explicit`の
列挙型定義コメント("`track`は`None`は変えない")もそのまま正しい。

### D3. 対象は`UserHalfWidthAlnumToggle`/`PanicReset`の2 strategyのみ

`scope_event_of`の他のマッピング(`UserImeSetIntent`/`PanicReset`の開閉用の分岐、:687-697)は
`mode: None`で、今回のconv混入経路に関係しないため変更しない。

## 限界・未確認

- 「古いeisu追跡の出どころが本当に10秒前のその窓での英字入力だったか」は未確認(belief-v2の
  追跡中convはログに出ない、BUG-229本文参照)。本ADRの修正は出どころを特定せずとも効く
  (どの経路で古いconvが残っても、復元・トグル出入りの時点でクリアするため)。
- 実機確認は未実施。回帰テストは`scope_belief.rs`の`#[cfg(test)] mod tests`(Linuxホストで実行可、
  `cargo test -p awase-ime-core --lib`)に、「`UserHalfWidthAlnumToggle`のExplicit事象の前に
  `row.track.conv`がeisu相当の値を持つ状態を作り、事象後に`None`になっていること」を確認する
  ユニットテストを追加する(ADR実装時)。
- 本修正は`row.track.conv`だけをクリアし、`row.mode`/`row.open`の確定期限(`fence_ms`)の仕組み自体は
  変えない。古いconvに基づく誤ったexplicit主張が今後一切発生しなくなることで、170ms期限が無関係な
  観測を棄却する副作用も併せて無くなる想定だが、これは本修正の直接の効果ではなく結果としての解消。

## 却下した案

- **`on_explicit`の汎用契約を変える(「modeがSomeなら無条件にtrack.convもクリアする」)**: D1で
  述べたとおり、`ScopeEvent::Explicit.track`の列挙型定義コメントの意味を書き換えることになり、
  影響範囲の洗い出し(他の呼び出し元が`track: None`で呼ぶ意図を一つずつ確認する)のコストが、
  問題のある呼び出し元1か所を直すコストより高い。狙った箇所だけを直すD1を採る。
- **`KeyTrack::without_conv_while_half_width_alnum`(ADR-244 M-2)の対象を「トグルを抜けた直後」にも
  広げる**: この関数は`toggle_active`という現在のトグル状態(グローバルな一時フラグ)を入力にしており、
  「10秒前に別のタイミングでこの窓がトグルに入っていた」という窓ごとの持続状態(ADR-282の`row.track`)
  とは別の軸。この関数を流用すると、軸の異なる2つの状態を1つの関数に混在させることになり、
  読み手の負担が増える。belief-v2側の`scope_belief.rs`だけで閉じるD1を優先する。
