---
id: ADR-289
title: |-
  半角英数トグルの出入り・復元が明示的にmodeを書き換えるとき、belief-v2の窓ごとの追跡中変換モード(key_track.conv)と、旧belief(ImeBelief)のprev_conversion_modeを、どちらも古い値のまま残さずクリアする(BUG-229)
summary: |-
  `scope_belief.rs::on_explicit`は`track`引数がSomeのときだけ`row.track`を書き換える。ADR-245の半角英数復元・トグルの出入り
  (`InputModeApplied{UserHalfWidthAlnumToggle|PanicReset}`)が作る`ScopeEvent::Explicit`は`track: None`なので、
  これらの明示操作でmodeを書き換えても、窓ごとに持続する`row.track.conv`(ADR-282の設計)は古い値のまま残る。
  直後のテーブルキー予測がこの古いconvを入力にして効果を予測し、`track: Some(prediction.track)`付きで改めて古いモードを
  explicit主張してしまい、170msの確定期限が直後の本物の観測を棄却する(BUG-229、2026-11-09実機)。
  D1は`on_observed`が受理した観測で`row.track.conv = None`へ戻す(:541-543)のと同じ扱いを、該当する`scope_event_of`の
  マッピング側(`UserHalfWidthAlnumToggle`/`PanicReset`の変換、:731-741)にも入れる。
  **round1のOpusレビューで、D1だけでは不十分と判明**: 予測の入力は`track.conv`だけでなく、旧belief
  (`ImeBelief::prev_conversion_mode`)にもフォールバックする(`key_pipeline.rs:2030`)。D4として、旧belief側にも
  `InputModeObserved`の受理時(`ime_model.rs:1005-1006`)と同じ掃除を`InputModeApplied`の該当strategyにも追加する。
  **round2のOpusレビューで、D4の配置が実在のコード構造と合わないこと(`ImeModel`に`belief`フィールドは無く、
  `prev_conversion_mode`は殻`ImeStateHub`側にある)、さらに今回の実機事故(先頭がBackspaceというテーブルキー)
  自体はD1だけで説明がつくこと、しかし先頭が文字キー(非テーブルキー)の場合は復元送信後に走る
  `[focus-conv-check]`(`key_pipeline.rs:3496-3512`)が復元前の値を読んで`prev_conversion_mode`に書き込む
  未測定のレースが別途残ることが判明した。D4をD4a(`ime_model.rs`内、変更なし)とD4b(`prev_conversion_mode`は
  `apply_input_mode_correction`〈`runtime/mod.rs`〉側でクリア、architecture_guardの文字列カウントに触れない形)に
  分割し、D5として`[focus-conv-check]`の書き込みに「探査開始後に明示操作が起きていないか」のガードを追加する。
status: |-
  起草(2026-11-09)。Opus round1〜4レビュー(4ラウンドで収束、round4は「ブロッキングな指摘なし」)を反映。
  D1/D4a/D4b/D5を実装済み(2026-11-10、commit `b384f3698`)。compile・architecture_guard(175件)・
  golden_scenarios(24件)・layer_boundary_guard・awase-windows全テストで確認済み。D4b/D5は
  対になる既存のapply_idle_conv_check同様、直接ユニットテストは追加していない。実機確認は未実施。
related_adr:
  - "ADR-282"
  - "ADR-245"
  - "ADR-244"
---

# ADR-289: 半角英数トグルの出入り・復元時に、belief-v2と旧beliefの両方の追跡中convをクリアする(BUG-229)

## 背景

[BUG-229](../known-bugs/BUG-229.md)の実機報告(dragonflyg4、2026-11-09T13:13:19 UTC):
半角英数のまま離れたChrome窓に戻ったところ、ADR-245の「戻った窓の最初の打鍵→半角英数トグルを復元」が
正しくbeliefの開閉主張(`z=(open=true basis=z-observed-high)`)をkana側へ収束させEngineをONにしたにも
かかわらず、同じ打鍵(Backspace)の処理中に70μs後、`[belief-v2]`が`mode=eisu basis=z-explicit`へ再び
食い違いを起こし、43ms後に届いた本物の`[mozc-shm]`の`"tsf_mode":"hiragana"`を棄却したうえで、さらに
43ms後にEngineが再度OFFになった。期待動作は「IME ON なら Engine ON」(利用者の報告どおり)で、実IMEは
hiraganaのままだったため、これは明確な不具合である。
(`basis=z-observed-high`は**開閉の主張**の根拠であり、`mode`の主張の根拠ではない——mode側がこの時点で
観測から確定していたなら`on_observed`が既に`track.conv`を落としていたはずで、以下の原因と矛盾しない)。

Opusサブエージェントによるコード調査(commit `6237a0601`、読み取り専用、2ラウンド)で、原因を
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

**古いconvの出どころ**(round1で確定、「不明」ではない): トグルが**アクティブな間**、予測の入力からは
convが抜かれる(`key_pipeline.rs:2045-2047`、`KeyTrack::without_conv_while_half_width_alnum`)が、これは
入力側だけの処置で、トグル中に押したテーブルキー自身の予測結果(`after_conv`)は`Some(C10)`のまま
`track`に書き戻る(`key_effect_predictor.rs:648`、`scope_event_of_prediction`:654)。トグル中に1回でも
テーブルキー(Backspace/Enter/Space/Esc等)を押した窓の行には、トグルを抜けた後もこの`C10`が残る。

[BUG-192](../known-bugs/BUG-192.md)/ADR-244 M-2は近縁の問題(`KeyTrack::without_conv_while_half_width_alnum`、
トグルが**アクティブな間**だけ追跡中convを捨てる)を修正済みだが、今回のBUG-229は「トグルを抜けた・
復元した直後」の別の抜けで、M-2の対象外。M-2自身のコメント(`key_effect_predictor.rs:333-335`、
「トグルを抜けた後の予測は、conv_raw、次いで入力モードから変換モードを決める(解除後の実状態はかな
なので方向は正しい)」)は、`conv_raw`がトグル脱出**後**に読み直されていることを前提にしているが、
下記D4の経緯のとおりこの前提も崩れている。

## 決定

### D1. belief-v2: `on_explicit`の汎用契約ではなく、`scope_event_of`の該当マッピングだけを直す

`ScopeEvent::Explicit.track`は列挙型の定義時点で「`None`は変えない」と明記されたフィールドである
(scope_belief.rs:148)。この汎用契約自体は他の呼び出し元(`scope_event_of_prediction`の
`track: Some(prediction.track)`、`UserImeSetIntent`/`PanicReset`の`track: Some(KeyTrack{stage: Stage::None, ..view.track})`)
が前提にしており、`on_explicit`側で「modeがSomeならtrackがNoneでも強制的にconvを消す」という
特殊扱いを足すのではなく、問題のあるマッピング(:731-741)**だけ**を直す。`scope_event_of`は
`view: &ScopeView`を既に引数に持つため、その場で`track: Some(KeyTrack { conv: None, ..view.track })`を
作れる(`stage`は`view.track.stage`をそのまま引き継ぎ、`conv`だけを`None`に落とす)。

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

`stage`を引き継ぐ(`Stage::None`へ強制リセットしない)理由: ひらがな/英数の軸(`conv`)と、変換中かどうかの
軸(`stage`)は別で、半角英数トグルの出入り自体は変換セッションの有無を変えない。トグル解除後に
stageが古い`Typing`/`None`のまま残っても、`conv`が`C19`(ひらがな)に解決されていればテーブルの
`C19→C19`行は`mode_effect`が`None`(変化なし)を返すため(`key_effect_table.rs`の該当行)、本バグの
再発には寄与しない。古い`ConvSpace`/`ConvHenkan`/`ConvMuhenkan`段階が別の変換キーの誤予測を起こす
可能性は、ADR-282の「trackは窓ごとにフォーカスをまたいで持続する」という設計そのものに内在する別の
既知の限界であり(本ADRで悪化も解消もしない)、ここでは対象としない。

### D2. `on_explicit`自体は変更しない

D1の結果、`track`に常に具体値(`Some`)が渡るようになるため、`on_explicit`の
「`track`がSomeのときだけ上書きする」という既存ロジック自体は変更不要。`ScopeEvent::Explicit`の
列挙型定義コメント("`track`は`None`は変えない")もそのまま正しい。

### D3. 対象は`UserHalfWidthAlnumToggle`/`PanicReset`の2 strategyのみ(crates/全体をgrepで確認済み)

`crates/`・`src/`全体を実際にgrepした結果、`ScopeEvent::Explicit`の本番コードでの構築箇所は
`scope_belief.rs`内の3箇所だけで、`mode: Some(..)`と`track: None`を同時に持つのは:731-741のこの
1箇所だけだった(他2箇所は`scope_event_of_prediction`の`track: Some(..)`常時、`UserImeSetIntent`/
`PanicReset`開閉分岐の`mode: None`)。`scope_event_of`を呼ぶ殻側(`platform_state.rs`)は結果を
パターンマッチするだけで`Explicit`を直接構築しない。したがって本ADRの対象は完全(=今後`None`+`None`
以外の組み合わせを作る新しい呼び出し元を追加しない限り、この1箇所の修正で全ケースを尽くす)。

### D4a. 旧belief(`ImeModel`): `InputModeApplied`の受理時にも`key_track.conv`をクリアする

**round1レビューで判明した追加の必要箇所。** belief-v2の`track.conv`をクリアしても、Z(belief-v2)の
予測入力はそれだけでは決まらない: `key_pipeline.rs:2042-2049`が組み立てる予測入力`conv_raw`は、
**旧belief**の`ImeBelief::prev_conversion_mode()`(`key_pipeline.rs:2030`)から来る。予測器はconvの解決を
`track.conv` → `conv_raw` → 入力モードの既定、の順で試す(`key_effect_predictor.rs:557-563`)ため、
`conv_raw`に古いeisu読みが残っていればD1だけでは防げない(D4bの背景も参照)。

旧belief側では、`key_track`(`ImeModel`自身の**private**フィールド、`ime_model.rs:332`。round3訂正:
旧版は誤って`pub(crate)`と書いていた)が同じ役割を持つ。観測の受理時(`InputModeObserved`の
`confidence`判定が通った枝、`ime_model.rs:1005-1006`)に`self.key_track.conv = None`へ戻す処理が
既にある。これと対になる処理が`InputModeApplied`(`ime_model.rs:1014-1019`、現状は`result == Applied`
なら`self.input_mode = mode`を書くだけ)には無い。`key_track`は`ImeModel`自身のフィールドなので、
この修正は同じ`reduce()`関数内で閉じる(**round2で確認: コンパイルも通る配置**)。

```rust
// `ime_model.rs`冒頭の `use super::ime_event::{..}` に `InputModeApplyStrategy` を追加する必要がある
// (round3訂正: 現状は`InputModeApplyResult`のみimportされ、`InputModeApplyStrategy`はテスト内の
// 完全修飾参照しかない)。
ImeEvent::InputModeApplied { mode, strategy, result } => {
    if result == InputModeApplyResult::Applied {
        self.input_mode = mode;
        if matches!(
            strategy,
            InputModeApplyStrategy::UserHalfWidthAlnumToggle | InputModeApplyStrategy::PanicReset
        ) {
            // D1(belief-v2のtrack.conv)と対になる掃除。トグルの出入り・復元は実変換モードを
            // 確定させる行為なので、古いkey_track.convへのフォールバックを断つ(BUG-229)。
            self.key_track.conv = None;
        }
    }
}
```

`PanicReset`について: 既存の`ImeEvent::PanicReset`アーム(`ime_model.rs:909-921`)は`key_track`に触れない。
`key_track`全体をリセットするのは`FocusChanged`(`:1176`)だけで、`UserImeSetIntent`は`stage`のみリセットする
(`:904`)。したがってD4aの`PanicReset`分は**既存動作の変更**(今まで`key_track.conv`は`PanicReset`後も
残っていた)である。実害は無いと見るが、「無変更(no-op)」ではなく新しい挙動として明記する
(round2訂正。旧版は誤って「既にクリアしている」と書いていた)。

### D4b. 旧belief(`ImeStateHub`): `prev_conversion_mode`は殻側(`apply_input_mode_correction`)でクリアする

**round2レビューで判明した配置の誤り。** `prev_conversion_mode`は`ImeModel`のフィールドではなく、
殻(`awase-windows`crate)の`ImeStateHub::belief: ImeBelief`(`platform_state.rs:38`)が持つ。
`ImeModel::reduce()`(core crate)から`self.belief...`は参照できず、D4aと同じ場所には書けない
(round2で発覚、旧版はコンパイルしないコードを提案していた)。

代わりに、`UserHalfWidthAlnumToggle`の`InputModeApplied`を実際に発行する経路
`Runtime::apply_input_mode_correction`(`runtime/mod.rs:1109-1123`)の`strategy`引数を見て、
`dispatch_event`の前後どちらでもよいが同じ呼び出しの中で、**ハブ自身の既存setter**
`self.platform_state.ime.set_prev_conversion_mode(None)`(`platform_state.rs:2181-2183`。round3訂正:
旧版は`.ime.belief.set_prev_conversion_mode(..)`という、本番コードに前例の無い経路を書いていた。
`focus_tracking.rs:412`・`key_pipeline.rs:870`・`:3512`と同じ、ハブのsetterを素通しで呼ぶ既存の
作法に揃える)を呼ぶ。

```rust
pub fn apply_input_mode_correction(
    &mut self,
    mode: InputModeState,
    strategy: crate::state::ime_event::InputModeApplyStrategy,
    tick_ms: crate::state::TickMs,
) {
    if matches!(strategy, InputModeApplyStrategy::UserHalfWidthAlnumToggle) {
        // D4a(ImeModel::key_track.conv)と対になる掃除。殻側が持つprev_conversion_modeも
        // 同じタイミングでクリアする(BUG-229)。
        self.platform_state.ime.set_prev_conversion_mode(None);
    }
    self.platform_state.ime.dispatch_event(
        crate::state::ime_event::ImeEvent::InputModeApplied { mode, strategy, result: /* Applied */ },
        tick_ms,
    );
}
```

**`PanicReset`はD4bの対象から外す(round3訂正)**: 旧版は`UserHalfWidthAlnumToggle | PanicReset`の
両方をD4bの対象と書いていたが、`PanicReset`はこの経路を通らない。`apply_panic_reset`
(`platform_state.rs:1891-1897`)は`InputModeApplied{PanicReset}`を**直接**構築し、直後に
`self.belief.reset_for_panic()`(`:1898`)を呼んで`prev_conversion_mode`を含め既にクリアしている
(architecture_guardのコメント`:774-775`も「直接構築、`apply_input_mode_correction`未経由」と
明記)。D4bの`matches!`は`UserHalfWidthAlnumToggle`単独にする——`PanicReset`はD4a(`ime_model.rs`側)
だけが新しい挙動で、D4b側はそもそも関与しない。

**architecture_guardの罠(round2で発見)**: `tests/architecture_guard.rs::input_mode_applied_construction_sites_are_accounted_for`
(`:793-816`)は、`platform_state.rs`/`key_pipeline.rs`/`ime_refresh.rs`/`runtime/mod.rs`の各ファイル内で
文字列`"ImeEvent::InputModeApplied {"`が出現する回数をテキストで数え、想定値(それぞれ2・0・0・1)と
一致しないとCIを落とす。この修正は`apply_input_mode_correction`の**既存の**構築箇所(`runtime/mod.rs`の
1件)の前後に`if`節を足すだけで、新たに`"ImeEvent::InputModeApplied {"`という文字列を増やさないため
このガードには触れない(実装時に`cargo test -p awase-windows --test architecture_guard`で確認する)。

### D5. `[focus-conv-check]`の書き込みに、復元後に走った場合の棄却ガードを足す(D4bで塞がらない残りの経路)

**round2レビューで判明**: D4bで`prev_conversion_mode`をクリアしても、復元の**後**に別の経路がもう一度
古い値を書き込みうる。フォーカス探査(`[focus-conv-check]`、`key_pipeline.rs:3496-3512`、
`self.platform_state.ime.set_prev_conversion_mode(Some(conv))`)は、フォーカス変更の瞬間ではなく
**最初の打鍵**で`spawn_local`される(`kp_stage_focus_probe`、`:106`、`consume_focus_barrier`:425)。
最初の打鍵がBackspace等のテーブルキーならBUG-229の実機事故自体はD1だけで説明がつく(下記限界参照)が、
最初の打鍵が**文字キー(非テーブルキー)**の場合、そのキー自体の予測は`track.conv`も`conv_raw`も
素通りする一方、同じ打鍵から`spawn_local`された探査タスクは、ADR-245の復元送信(GJI exit)と**非同期に
競合**する。探査が復元送信より先に完了すれば、まだ半角英数のままの実IMEを読んでしまい、D4bが
クリアした**後**に`prev_conversion_mode`へ古いeisuを書き戻す。次のテーブルキー(確定のSpace・BS等)が
これを読んでeisuを再構成し、同じ`z-explicit eisu`主張と170msフェンスが再現しうる。この競合が実際に
起きる頻度は未測定。

**比較は「時刻の前後」ではなく「値の一致」にする(round3訂正)**: 探査の起案(`kp_stage_focus_probe`、
`:106`)と復元の`note_explicit_ime_action`呼び出し(`kp_restore_kana_from_half_width`内、`:2822`付近)は
同じ`kp_run_inner`呼び出しの中で、どちらも`GetTickCount64`(分解能~15.6ms)から取るため、通常は
**同じ値**になる。「探査開始時刻 `probe_started_ms` より後に明示操作の時刻が来ていれば棄却」という
比較(`>`)は、まさにD5が狙うこの同一打鍵のケースでは**成立しない**(両者が等しいため「後」にならない)。

代わりに、idle-conv-checkが既に使っている**値スナップショット**の作法(`last_explicit_ime_action_ms_raw()`、
`platform_state.rs:1302-1323`。このgetter自身のdocが、age/時刻比較よりも値の一致判定を使う理由
〈BUG-34の長時間ブロックへの耐性、時計の分解度に左右されない〉を説明している)を転用する: 探査を
`spawn_local`する時点(`:430`付近)で`last_explicit_ime_action_ms_raw()`を**値として**キャプチャし、
探査タスクが完了して書き込む時点(`:3512`)でもう一度読み、**値が変わっていたら書かない**。
`probe_started_ms`自体は既に`apply_focus_probe`まで配線済み(`:3378`、既存の利用例`:3389`)なので、
新しい配線はこのキャプチャした値を1つ、`spawn_local`のクロージャと`apply_focus_probe`の引数に
追加するだけで足りる(「配線は実装時に確認」としていたが、既存の仕組みの組み合わせで足りることが
round3で分かったため、この限定は外す)。

**ガードの範囲**: `:3512`の`set_prev_conversion_mode(Some(conv))`だけを止める。同じ`apply_focus_probe`が
更新する`is_japanese_ime`(`:3397-3400`)や開閉観測は止めない(round3確認)。

**過剰な棄却(round3指摘)**: このガードは、最初の打鍵がモードキー等、半角英数トグル以外の明示操作で
あっても読みを棄却する(保守的)。その場合、予測器は入力モードから既定のconvへフォールバックし、
`EXPLICIT_IME_SUPPRESS_MS`経過後にidle-conv-checkが`conv_raw`を再度埋める。実害は小さいと見るが、
実装時に意図して受け入れる。

D1・D4a・D4b・D5のいずれも「belief-v2側だけで閉じる」という当初案(旧版の却下した案の前提)は成り立たない。
古いeisuの再構成経路は(a) belief-v2の`track.conv`、(b) 旧beliefの`key_track.conv`、(c) 旧beliefの
`prev_conversion_mode`、(d) `prev_conversion_mode`への復元後の遅延書き込み、の4つあり、D1/D4a/D4b/D5は
それぞれ(a)〜(d)に対応する。

## 限界・未確認

- 実機確認は未実施。
- **回帰テストの層を分ける(round2で訂正)**: `scope_belief.rs`の`#[cfg(test)] mod tests`(Z単体、
  Linuxホストで実行可、`cargo test -p awase-ime-core --lib`)は`conv_raw`を一切モデル化していない
  (`scope_event_of_prediction`は呼び出し側が組み立てた`Prediction`を受け取るだけで、`conv_raw`の
  解決はZの外、殻側にある)。したがって「半角英数トグルON→テーブルキー→トグルOFF→テーブルキー」
  という列をZだけに通すユニットテストは、**D1だけで通る**(旧版はここを「D4が無いと失敗する」と
  誤って書いていた。round2の指摘で訂正)。このテストはD1の確認にはなるが、D4a/D4b/D5が実際に塞ぐ
  `conv_raw`経由の再発は検出できない。
  - D1用: 上記のZ単体テストを`scope_belief.rs`に追加する。
  - D4a用: `ime_model.rs`に、`InputModeApplied{UserHalfWidthAlnumToggle, Applied}`の前に
    `key_track.conv`が非`None`の状態を作り、事象後に`None`になっていることを確認するユニットテストを
    追加する。
  - D4b用: Z単体やImeModel単体では`conv_raw`の経路そのものが現れないため、殻を含む層
    (`crates/awase-windows/tests/support/harness.rs`のハーネス、または`key_effect_predictor.rs`の
    予測関数に`conv_raw = Some(0x00)`〈eisu相当〉を直接渡すテスト)で確認する必要がある。
  - D5用: `apply_focus_probe`自体は`#[cfg(windows)]`でLinuxでは走らない(CLAUDE.md参照)。
    「スナップショットした値と現在値が違えば書かない」という判定を、`apply_focus_probe`本体から
    呼び出し可能な純粋関数(他の値スナップショット判定と同様、引数を取って真偽値を返すだけの形)として
    切り出せれば、その関数だけはLinuxホストでユニットテストできる。切り出さない場合、D5は
    [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md)の(b)(`docs/known-bugs/`への
    記録)で代替する——実装時にどちらの経路を取ったかをBUG-229に書く。
- **既存テストの明示的な反転が必要**: `scope_belief.rs`の既存テスト
  `panic_reset_and_half_width_toggle_enter_the_claims`(:1220-1235付近)は、現在
  `assert_eq!(z.view().track, before.track)`で「追跡も変えない」ことを**意図的に固定**している
  (コメント「追跡も変えない」)。D1の実装でこの前提が反転するため、実装時にこのアサーションを
  「`track.conv`は`None`になる(`stage`は引き継ぐ)」側へ書き換える。新規テストの追加だけでは
  この既存の固定を見落とす。
- **トグルenter時の副作用(round2で確認、無害)**: D4a/D4bはトグルの出入り**両方**(enter/exit)に効く。
  トグルenter時にクリアすると、enter前の(ひらがな側の)古い`conv_raw`がトグル中に誤って使われる
  経路を塞ぐ(副次的な改善。既存の`without_conv_while_half_width_alnum`〈ADR-244 M-2〉が予測の入力
  からconvを抜く処理と役割は重ならない——M-2はトグル中のテーブルキー自身が`C10`を書き戻すのは防げない、
  round1 finding 6参照)。既存の挙動を壊す退行ではない。
- **golden_scenarios.rsへの影響は未確認(round2で指摘)**: `crates/awase-windows/tests/golden_scenarios.rs`
  の`UserHalfWidthAlnumToggle`系シナリオ(:663-740付近)が`KeyTrack`(`serde::Serialize`)を含む値を
  snapshotしていれば、D4aでgoldenの期待値がずれる可能性がある。実装時に
  `cargo test -p awase-windows --test golden_scenarios`で確認する。
- **関連だが対象外の点**(round1・round3レビューで発見、別ADR/BUG候補):
  1. ADR-245の離脱時トグルOFF(`ir_notify_focus_changed`経由の`InputModeApplied{AssumedRomaji, UserHalfWidthAlnumToggle}`)
     は、離脱先の新しい窓Bの行に対して発行される可能性が高い(`focus_tracking.rs`と`ime_refresh.rs`の
     呼び出し順を完全には追えていない、未確証)。その場合、Bは本来関係のない`mode=kana`主張と170ms
     フェンスを持つことになるが、実害は小さい(D1適用後はBのconvも`None`になるだけ)。
  2. `UserImeOnEisuReset`/`UserTurnOnEisuReset`strategy(旧beliefをeisu→kanaへ動かす)は`scope_event_of`
     の対象外(:742でNoneにマップ)で、Z側のmode・convのどちらも更新されない。旧beliefとZが
     ここでも分岐しうるが、本ADRのバグとは別の発生経路であり対象としない。
  3. `[focus-conv-check]`の同じ処理ブロックは、`set_prev_conversion_mode`の直前(`:3501-3510`)で
     同じ古い読みを`conv_mode.observe(ConvObservation{source: FocusCheck, ..})`にも渡している。
     この値はwarmupの最初のVK選択や`ImmSetConversionStatus`のターゲット値に使われる
     (`:855-857`のコメント)。D5は`:3512`の`set_prev_conversion_mode`だけをガードし、この2つ目の
     書き込み先には触れない(同じ古い読みが別の消費者〈conv mode再発ファミリー〉に残る余地がある)。
     D5を実装する人が「なぜブロック内の片方だけガードしたのか」で迷わないよう、この区別を明記する
     目的でここに残す。両方をガードするかは別の判断(conv mode actuationの変更になるため)。
  4. `apply_ime_update`(`platform_state.rs:1977`、`ime_refresh`のポーリング)もconv_rawを書けるが、
     TsfNative/Chromeでは何も返さない(`runtime/mod.rs:1127`)。素のWin32窓でこの経路が復元送信と
     競合する可能性は残るが、そうした窓では確定済みのモード観測も同時に`on_observed`へ届くため
     影響は薄いと見て、残存リスクとして受け入れる(round3指摘)。

## 却下した案

- **`on_explicit`の汎用契約を変える(「modeがSomeなら無条件にtrack.convもクリアする」)**: D1で
  述べたとおり、`ScopeEvent::Explicit.track`の列挙型定義コメントの意味を書き換えることになる。
  D3のgrep結果により「対象となる呼び出し元は現状1箇所だけ」と確定しているため、汎用契約を変えても
  今日時点の挙動はD1と同一になるが、将来新しい呼び出し元が`track: None`で来たときに意図を
  再確認する機会を失う。狙った箇所だけを直すD1を優先する(汎用契約を変えるコストが高いからではなく、
  局所修正の方が将来の呼び出し元に対して安全側に働くため)。
- **`KeyTrack::without_conv_while_half_width_alnum`(ADR-244 M-2)の対象を「トグルを抜けた直後」にも
  広げる**: この関数は`toggle_active`という現在のトグル状態(グローバルな一時フラグ)を入力にしており、
  「10秒前に別のタイミングでこの窓がトグルに入っていた」という窓ごとの持続状態(ADR-282の`row.track`)
  とは別の軸。さらにこの関数は予測の**入力**だけを変える処置で、D4が直す`conv_raw`(旧beliefの
  `prev_conversion_mode`)には触れられない。軸の異なる状態を1つの関数に混在させるより、D1・D4で
  それぞれの保存場所を直接クリアする方が読み手の負担が小さい。
