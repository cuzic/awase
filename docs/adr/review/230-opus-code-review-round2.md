# ADR-230 実装(PR #501)Opus コードレビュー round2

対象: `e46596c1..2e944f97`(round1 の反映)。方法: 新しい `scancode_pairs.rs` 本体を scratchpad に切り出し、round1 と同じ
300 万件の乱数探索(宇宙: Caps/LCtrl/無変換/変換/左Alt/右Alt(E0)/0x30/0、from=0・非隣接の重複を含む)を `rustc --test` で再実行。
新たな Blocker は無し。

## 再確認の結果(round1 の指摘)

- B1/S1: 解消。乱数探索で「Caps 追加 Ctrl と Caps を含むペアの同時成立」「恒等が崩れる」「削除だけが失敗する」
  「削除で残したペアが消える」「指定したペアが読めない」はいずれも 0 件。caps 候補の新しい条件と値での削除の組み合わせで新しい穴は出なかった。
- 「Caps 追加 Ctrl を指定したのに読めない」は残るが、すべて「他ツールの from 重複が残り、ペアが `Caps⇄X` や `Caps⇄LCtrl` に読み替わる」
  形で、該当するエントリは `revealed` に入る(UI が警告できる)。テストの例外条件・ADR の記述と整合。
- S2: 残る「実効の写像が悪くなる」例は、すべて**未登録のキーは自分自身を出す**(Caps 追加 Ctrl やペアを外すと Caps が Caps を出し、
  他ツールの `X→Caps` と重なる)型。ADR がこれを「保証しない」と明記したので、指摘の意図どおり。
- S5: `classify_read_back` は意図どおり。読み戻しが `None`・壊れた値のときは他の書き手でも巻き戻すが、頻度と害は小さく Note 未満。

## Should-fix

### S2-1. `revealed` が「displaced を消す」経路を拾わず、ADR の「どの操作でも」と食い違う(`scancode_pairs.rs` `compute_swap_write` の `revealed`)

`revealed` は「from が `removed_own`(外したペア・外した Caps 追加 Ctrl)と同じ」だけを条件にしており、`replaced` を消して
効き出すエントリを見ない。通常の `replaced` は from が `added_from` と同じなので同じ from のエントリはすべて消えて問題にならないが、
**Caps 追加 Ctrl を足すときに特別に消す他ツールの `左 Ctrl→Caps`** だけは from(LCtrl)が追加エントリに無いため、
同じ from の他のエントリが効き出しても報告されない。rustc で確認:

```
existing = [(1D,3A), (1D,E038)]          // どちらも Unclaimed(DuplicateFrom)
compute_swap_write(existing, [], caps=true)
→ entries=[(1D,E038), (3A,1D)], displaced=[(1D,3A)], revealed=[]
// 前: 左 Ctrl は Caps か 右Alt(OS 依存)。後: 左 Ctrl は確実に 右Alt
```

ADR 決定3 は `revealed` を「Caps 追加 Ctrl を外す・ペアを外す・**displaced を消す**、どの操作でも」と定義しているので、実装を
「from が `removed_own` または `replaced` のいずれかと同じ」に広げる(このケースでは `(1D,E038)` が `revealed` に入る)か、
ADR の記述をこの例外に合わせる。displaced の確認ダイアログは出るので実害は小さいが、UI が `revealed` を唯一の根拠に警告する設計なので
揃えておくこと。回帰ケースとして上の入力を固定する。
