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
  (`ImeBelief::prev_conversion_mode`)にもフォールバックする(`key_pipeline.rs:2030`)。旧beliefの同じ値は
  `InputModeApplied`では一度もクリアされない(`ime_model.rs:1014-1019`)ため、D1だけだとBackspaceの予測は
  このフォールバック経由で同じeisuを再現しうる。D4として、旧belief側にも`InputModeObserved`の受理時(同ファイル
  :1005-1006)と同じ掃除を`InputModeApplied`の該当strategyにも追加する。
status: |-
  起草(2026-11-09)。Opus round1レビュー反映(D4追加・D3の記述修正・既存テストの反転を明記・限界の「出どころ不明」を解消)。
  round2レビュー待ち。実装・実機確認とも未着手。
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

### D4. 旧belief(`ImeBelief`): `InputModeApplied`の受理時にも`prev_conversion_mode`と`key_track.conv`をクリアする

**round1レビューで判明した追加の必要箇所。** belief-v2の`track.conv`をクリアしても、Z(belief-v2)の
予測入力はそれだけでは決まらない: `key_pipeline.rs:2042-2049`が組み立てる予測入力`conv_raw`は、
**旧belief**の`ImeBelief::prev_conversion_mode()`(`key_pipeline.rs:2030`)から来る。この値は、BUG-229の
実機シナリオでは、ADR-245の復元(GJI exit送信)より**前**に、戻った窓への`[focus-conv-check]`(`key_pipeline.rs:3512`
付近)が実IME(まだ半角英数のまま)を直接読んで書き込んだものであり、復元の**後**には誰も読み直さない
(MS-IME/IMC経路のR1再読みに相当する仕組みがGJI経路には無い)。予測器はconvの解決を
`track.conv` → `conv_raw` → 入力モードの既定、の順で試す(`key_effect_predictor.rs:557-563`)ため、
D1で`track.conv`を`None`にしても、`conv_raw`に古いeisu読みが残っていればそこへフォールバックし、
Backspackのセル`C10→C10`から同じ`mode_effect=Eisu`が再び出て、同じ`z-explicit eisu`主張が再現しうる。

旧belief自身にも、観測の受理時(`InputModeObserved`の`confidence`判定が通った枝、`ime_model.rs:1005-1006`)
に`self.key_track.conv = None`へ戻す処理が既にある。これと対になる処理が`InputModeApplied`
(`ime_model.rs:1014-1019`、現状は`result == Applied`なら`self.input_mode = mode`を書くだけ)には無い。
D1と対にして、`UserHalfWidthAlnumToggle`/`PanicReset`の2 strategyで`Applied`のときは、同じ箇所で
`prev_conversion_mode`と`key_track.conv`も`None`へ戻す(`PanicReset`は既存の`ImeBelief::reset_for_panic`
〈`belief.rs:57-61`〉がこの2値を含め既にクリアしているため、実質的に新しい掃除が要るのは
`UserHalfWidthAlnumToggle`側)。

```rust
ImeEvent::InputModeApplied { mode, strategy, result } => {
    if result == InputModeApplyResult::Applied {
        self.input_mode = mode;
        if matches!(
            strategy,
            InputModeApplyStrategy::UserHalfWidthAlnumToggle | InputModeApplyStrategy::PanicReset
        ) {
            // D1(belief-v2のtrack.conv)と対になる掃除。トグルの出入り・復元は実変換モードを
            // 確定させる行為なので、古い conv_raw / key_track.conv へのフォールバックを断つ(BUG-229)。
            self.belief.set_prev_conversion_mode(None);
            self.key_track.conv = None;
        }
    }
}
```

(`PanicReset`はここで重ねて`None`にしても`reset_for_panic`と結果が変わらず無害。既存の挙動を壊さない)。

D1・D4のどちらも「belief-v2側だけで閉じる」という当初案(旧版の却下した案の前提)は成り立たない。
予測が2つの古い値(`track.conv`と`conv_raw`)のどちらからも古いeisuを再構成できるため、両方を同時に
断つ必要がある。

## 限界・未確認

- 実機確認は未実施。
- 回帰テストは手でconv相当の値を仕込むのではなく、**実際に再現する手順**を使う:
  半角英数トグルON → テーブルキー(例: Enter)を1回 → トグルOFF(復元) → テーブルキー(例: Backspace)、
  という列をZ(belief-v2)に通し、最後のBackspackの予測結果が`mode: None`(変化なし)になることを確認する
  (`crates/awase-ime-core/src/state/scope_belief.rs`の`#[cfg(test)] mod tests`、Linuxホストで実行可、
  `cargo test -p awase-ime-core --lib`)。
  - **D1のみを先に実装・テストすると、このテストは依然として失敗する想定**(D4が無いと`conv_raw`
    フォールバックで同じeisuが出るため)。D4実装後にこのテストが通ることを確認してから、BUG-229の
    `fix_commits`を更新する。
  - 旧belief側(`ime_model.rs`)にも対になるユニットテストを追加する: `InputModeApplied{UserHalfWidthAlnumToggle, Applied}`
    の前に`prev_conversion_mode`/`key_track.conv`が非`None`の状態を作り、事象後に両方`None`になっている
    ことを確認する。
- **既存テストの明示的な反転が必要**: `scope_belief.rs`の既存テスト
  `panic_reset_and_half_width_toggle_enter_the_claims`(:1220-1235付近)は、現在
  `assert_eq!(z.view().track, before.track)`で「追跡も変えない」ことを**意図的に固定**している
  (コメント「追跡も変えない」)。D1の実装でこの前提が反転するため、実装時にこのアサーションを
  「`track.conv`は`None`になる(`stage`は引き継ぐ)」側へ書き換える。新規テストの追加だけでは
  この既存の固定を見落とす。
- **関連だが対象外の2点**(round1レビューで発見、別ADR/BUG候補):
  1. ADR-245の離脱時トグルOFF(`ir_notify_focus_changed`経由の`InputModeApplied{AssumedRomaji, UserHalfWidthAlnumToggle}`)
     は、離脱先の新しい窓Bの行に対して発行される可能性が高い(`focus_tracking.rs`と`ime_refresh.rs`の
     呼び出し順を完全には追えていない、未確証)。その場合、Bは本来関係のない`mode=kana`主張と170ms
     フェンスを持つことになるが、実害は小さい(D1適用後はBのconvも`None`になるだけ)。
  2. `UserImeOnEisuReset`/`UserTurnOnEisuReset`strategy(旧beliefをeisu→kanaへ動かす)は`scope_event_of`
     の対象外(:742でNoneにマップ)で、Z側のmode・convのどちらも更新されない。旧beliefとZが
     ここでも分岐しうるが、本ADRのバグとは別の発生経路であり対象としない。

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
