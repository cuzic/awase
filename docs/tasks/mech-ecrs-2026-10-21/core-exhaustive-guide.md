# 核の全列挙検証の道具(`awase-ime-core`、Linux で動く)

「撤去できるか」を実機 CI を待たずに判定するための道具。ImeModel の reducer へイベント列を全列挙で流し、旧判定と新判定が全状態で同値か、または不変条件が全状態で成り立つかを調べる。

## 部品

- `crates/awase-ime-core/src/state/exhaustive_seq.rs`(`#[cfg(test)]`): `explore(init, vocab, max_len, step, visit)`。語彙から長さ `max_len` までの全列を深さ優先で辿り、到達した全節で `visit(state, trail)` を呼ぶ。`step` が `None` を返した枝は「現実に起きえない列」として切る。状態機械・語彙・検査は呼び出し側。
- 使用例: `state/ime_model_apply_acceptance_exhaustive.rs`(`ime_model.rs` の末尾から `#[path]` で子モジュールとして取り込む。`ImeModel` の private フィールドを読むため)。

## 使い方(型紙)

1. 語彙 `enum Ev`(`Copy`)を小さく決める(世代は 2〜3 個、向きは両方)。
2. 状態 `St`(`ImeModel` + 時計・seq・epoch)と `step(&St, Ev) -> Option<St>`。現実の前提(採番は単調、完了は払い出し済みだけ等)は `None` で表明する。
3. 旧判定は本番の写しをテスト内に置き、**毎回本番の関数と突き合わせる**(写しの腐敗検出)。新判定の候補は別関数で書く。
4. `visit` で各状態の全プローブ(generation × outcome × open)について旧/新を比べ、不一致を最短の反例つきで集計する。
5. 前提を 1 つずつ外した run を並べると、「どの前提に依存して同値か」が反例の形で出る。

節の数: 語彙 11・長さ 6 で約 100 万(前提の枝刈り後はもっと少ない)。語彙が 15 を超えるなら長さを 5 に下げる。

## DD2-05 の結果(apply 完了の受理: `focus_epoch` と `focus_generation_watermark`)

- 本番の前提(`InitialFocusScopeEstablished` は最初の apply 要求より前、generation は単調、完了は払い出し済み)の下で、`pending.focus_epoch == current_epoch` を `pending.generation >= watermark` に置き換えても(Superseded の腕だけ完了側の `generation >= watermark` にしても)、`ImeApplyAcceptance` は全列で一致する。
- 反例は 1 種類だけ: `InitialFocusScopeEstablished` が `ImeApplyRequested` の後に来る列(最短 `[Req(1,_), Init]` + 完了 gen 1)。fence の epoch だけが進み watermark が進まないため、epoch を撤去すると古い pending の完了が受理される(調査の例外 (i))。
- bootstrap でも watermark を進める案は、Init の前に払い出した generation の Superseded を旧判定より多く落とす(`[Req(1,_), Init, Req(2,_)]`)ので、同値の証拠にはならない。「Init が最初の要求より前」を表明する方が簡単。
- Superseded の腕の epoch 照合も消す案(B)は、完了の generation が未払い出しでもよいとすると崩れる(`Req(1)`, `Focus`, 完了 gen 2)。本番は払い出し済みの generation にしか完了を返さないので成り立つが、前提に依存する。pending の `generation >= watermark` を両方の腕に残す案(A)は前提なしでも一致する。

結論: epoch の撤去は、「Init は最初の要求より前」を型か `debug_assert` で表明するなら安全(案 A を推奨)。本番コードの変更は別 PR。

## DD2-02(`ConvModeMgr::observe` の fence 比較)は全列挙できない理由

- 主張は「呼び出し元 2 か所が常に同じ fence を渡す」で、これは**呼び出し側(殻、`cfg(windows)`)の性質**。`observe` 自体は `obs.fence != current` なら棄却する単純な比較で、核の全列挙で示せるのは「`fence == current` なら fence では棄却しない」という自明な側だけ。
- 呼び出し元: `key_pipeline.rs:868`(idle-conv-check。`fence: spawn_fence` と `self.focus_fence()`、`spawn_fence` は `:716` の `accepted.fence`)、`:3395`(focus-conv-check。両引数とも `accepted.fence`)。`git grep -n 'conv_mode.observe(' -- crates/awase-windows/src`。
- 注意: idle-conv-check は非同期(apply が spawn の後)で、`spawn_fence` と apply 時の `focus_fence()` は別の時点の値。DD2-02 の調査の「同じ同期フレーム」は focus-conv-check には当てはまるが、idle-conv-check の fence 比較は spawn〜apply の間の窓切替を実際に止めうる(`:694` 付近の世代照合が先に弾く場合を除く)。idle-conv-check 側を撤去する前に、`:694-716` の先行ガードが fence の変化を必ず先に弾くことの確認が要る(未確認)。
