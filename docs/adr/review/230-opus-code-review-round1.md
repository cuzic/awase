# ADR-230 実装(PR #501)Opus コードレビュー round1

対象: `feat/adr230-scancode-pairs`(`origin/develop...HEAD`、247226b2 / 5a06365a / e46596c1)。
方法: 差分の精読。`scancode_pairs.rs` の本体(テスト除く)を scratchpad に切り出し、`rustc --test` で
(1) 300 万件の乱数探索(キー: Caps/LCtrl/無変換/変換/左Alt/右Alt(E0)/0x30/0、from=0 を含む、隣接以外の重複も残す)と
(2) 最小反例の確認を行った。cargo build/test・clippy は実行していない(clippy 指摘は CI 任せ)。

## Blocker

### B1. 完全に同じエントリの複製が Caps 追加 Ctrl に吸われ、値での削除がペアの片方まで消す(`scancode_pairs.rs` `detect_swap_pairs` の caps_pos 判定と `compute_swap_write` 手順6)

`caps_pos` は「未使用の `(Caps, LCtrl)`」を探すだけなので、ペア `Caps⇄LCtrl` に含まれるエントリと**同じ値の複製**が
Caps 追加 Ctrl として分類される。その状態で手順6が**値で**削除する(`!removed.contains(e)`)ため、片方を消すと同じ値のもう片方
(ペアの1エントリ)も消える。rustc で確認した結果:

```
existing = [(3A,1D), (1D,3A), (3A,1D)]
detect   → pairs=[Caps⇄LCtrl, warning=false], caps_extra_ctrl=true, unclaimed=[]
compute_swap_write(existing, [Caps⇄LCtrl], caps=false) → Ok([(1D,3A)])   // ペアを残しチェックを外しただけ
compute_swap_write(existing, [],            caps=true)  → Ok([])          // ペアを外しチェックは残しただけ
```

- 1つ目: 利用者は「入れ替えは残し、追加 Ctrl だけやめる」操作をしたのに、結果は `LCtrl→Caps` の片方向だけになる。
  Caps キーはそのまま Caps、左 Ctrl も Caps を出すので**左側の Ctrl が無くなり、2キーが Caps を出す多対一**。
  削除だけの操作で「悪くならない」が破れ、かつ実装自身の性質テストのアサート
  「指定したペアはすべて読める」(`got.contains(p)`)を満たさない入力(性質テストの生成器が出さなかっただけ)。
- 2つ目: 追加 Ctrl を残したのに値ごと削除されて何も残らない(`after.caps_extra_ctrl != caps`)。
- `decide` は再検査をしないので、昇格側はこのまま書き、読み戻しも一致し `WorkerExit::Ok` で終わる。
- 警告も出ない: `warning` は `!own.contains(&e)` で同値の複製を除外するので `warning=false`。UI はペア(警告なし)と
  チェックボックス ON を同時に表示する(決定1 の排他にも反する)。

発生には他ツール/手動でのエントリ追記が要るが、決定3 は他ツールのエントリと共存することが前提で、結果は左 Ctrl の喪失。
修正案: 削除を値ではなく**位置(index)**で行う(`detect_swap_pairs` が各ペア・プリセットに使った index を返す)、
または `detect_swap_pairs` で「採用済みペアのエントリと同値の複製」を caps 判定の前に Unclaimed(DuplicateFrom)へ回す。
「完全に同じ複製は1つを消すなら全部消す」という段階1の細部方針は、ペア・プリセットの**採用されたエントリ**を巻き込まないよう
限定する必要がある。回帰テストに上の2ケースを足すこと。

## Should-fix

### S1. `Caps→LCtrl` がペア `Caps⇄X` の from 重複でも Caps 追加 Ctrl に分類される(決定3 の表・決定1 と不一致)

```
detect([(3A,7B), (7B,3A), (3A,1D)]) → pairs=[Caps⇄無変換(warning)], caps_extra_ctrl=true, unclaimed=[]
```

決定3 の表では「ペアがそろい、さらに同じ from の別エントリ」は Unclaimed(from 重複)。実装は from 重複判定より先に caps_pos が
拾うので、表の規則が適用されない。UI は決定1 で排他のはずの「Caps を含むペア」と「Caps 追加 Ctrl ON」を同時に出し、
`Caps→LCtrl` は他ツールの読み取り専用一覧に出ない。`caps_pos` の候補から「from が採用済みペアのキー(Caps)」のものを除き、
Unclaimed(DuplicateFrom)にすべき。B1 と同じ箇所なので一緒に直せる。

### S2. 「悪くならない」の検査がエントリの個数だけを見ていて、実際の写像(未登録キーの恒等・重複 from の隠れたエントリ)を見ていない

性質テストの `dup_from`/`dup_to` は「エントリ列の中の重複数」で、(a) エントリの無いキーは自分自身を出すこと、
(b) from 重複のとき実際にはどちらか一方しか効かないこと、を数えない。そのため決定3 の「削除だけの操作は多対一を新たに作らない」は
**実効の写像では成り立たない**。rustc で確認した例(from 重複は先勝ちと仮定):

```
existing = [(3A,1D), (38,79), (38,1D), (79,0), (3A,79)]   // caps_extra_ctrl=true
compute_swap_write(existing, [], caps=false) → Ok([(38,79), (38,1D), (79,0), (3A,79)])
// 前: Caps→LCtrl、左Alt→変換、変換→無効 … 変換を出すのは左Alt だけ
// 後: 隠れていた Caps→変換 が効き、左Alt と Caps が両方 変換 を出す
```

B1 の `[(1D,3A)]` も個数指標では「改善」(dup_to 1→0)と数えられてしまう。後勝ちと仮定しても同種の例が出る
(`[(1D,7B),(79,7B),(7B,30),(1D,38),(38,1D)]` から `左Alt⇄LCtrl` を外すと LCtrl→無変換 が効き、変換と LCtrl が両方 無変換)。
ADR は S13(`A→Z` が効くようになる)だけを UI 警告で扱っているが、**Caps 追加 Ctrl を外すとき・displaced を消すとき**にも
同じ「隠れていたエントリが効き出す」ことが起きる。対処案: `WritePlan` に「この書き込みで新たに効き出す(隠れていた)エントリ」を
返して UI が全経路で警告する、または ADR の主張を「エントリ列として重複を増やさない」に弱めて明記する。少なくとも ADR 決定3 の
「多対一も from 重複も新たに作らない」という記述は、実効写像の意味では誤り。

### S3. 性質テストが ADR の要求より弱く、生成器に盲点がある(`scancode_pairs.rs` `property_removal_identity_round_trip_and_no_new_collisions`)

- ADR 決定3 は「`detect(compute(...).entries)` の pairs・caps が入力と**一致**、unclaimed が『existing の Unclaimed − displaced』と一致
  (保持の検査を括弧書きで済ませない)」を要求。実装は「指定ペアを**含む**」「余分なペアは元 Unclaimed 由来なら可」
  「caps は吸われた場合を除く」「消したペアと同値の複製は保持しなくてよい(`same_as_removed`)」に弱めている。
  実装の挙動(削除で隠れたペアが現れる、同値複製は全部消す)の方が ADR より現実的なので、**ADR 側を実装に合わせて書き換える**か、
  テストを ADR どおりにして実装を合わせるか、どちらかに揃えること。現状は ADR の「必須のテストケース」が満たされていない。
- 生成器の盲点: `existing.dedup()` は隣接のみ・最大6件・2万回・宇宙7キーで、E0 キー(`0xE038`)と from=0 を含まない。
  B1 の形(非隣接の同値複製+ペア)を一度も踏まなかった。宇宙を小さく(例: Caps・LCtrl・2キー+0)して
  4件以下を**全列挙**すれば数十万件で済み、乱数より確実。B1・S1 の反例を固定ケースとしても足すこと。

### S4. ADR-230 本文・索引の状態表記が食い違っている

- `status` が「段階1〜3は実装済み…。実装なし。」と矛盾した文になっている(旧文の「実装なし」が残存)。
- 「実装の段階」節が `1, 2(実装済み), 3(実装済み), 2, 3, 4, 5` と番号が重複している(旧 2・3 の行が残っている)。
- `docs/adr/index.md` の 230 行が「起草(2026-10-06、未レビュー、実装なし)」のまま(docs-frontmatter-convention: index は
  frontmatter の短縮表示。round3 収束・段階1〜3実装済みを短く反映する)。

### S5. 読み戻し不一致の巻き戻しが、間に入った他の書き手の値を黙って消す(`scancode_map_admin.rs` `run_elevated_pairs_worker_windows` → `rollback_windows`)

読み戻しが「書いた値でも元の値でもない、正しく読める値」だったとき(書き込みと読み戻しの間に他ツール・別ユーザーの
awase-settings が書いた)、現在の実装はそれを不一致として元の値で上書きし `RolledBack` を返す。比較交換で守ろうとした
「他の書き手の変更を壊さない」が、書き込み後の窓で破れる。窓は短いが、決定5 が TOCTOU を明示的に扱っている以上、
読み戻し値が `parse_entries_strict` で読めて `written` とも `original` とも違う場合は巻き戻さず `Changed`(書いた後)相当で返し、
画面に再読込させる方が一貫する。

## Note

### N1. `detect_swap_pairs` は入力順で結果が変わり、OS がどちらを効かせるかと一致する保証が無い

```
[(7B,38),(7B,79),(79,7B),(38,7B)] → 無変換⇄左Alt
[(7B,79),(79,7B),(7B,38),(38,7B)] → 無変換⇄変換
```

同じ値に対しては決定的なので UI と昇格側で食い違いはしない(`decide` は同じ entries に同じ関数を通す)。ただし from 重複時に
キーボードクラスドライバが先勝ちか後勝ちか未確認なので、「編集できるペア」として出したものが実際に効いている組とは限らない。
警告文言でそれを言うか、実機確認の項目に足す。

### N2. 消すペアと同値の Unclaimed 複製が、新規ペアを誤って拒否する(偽陽性)

```
compute_swap_write([(7B,38),(38,7B),(7B,38)], [変換⇄左Alt], false) → Err(CollidesWithForeignTarget{(79,38)})
```

手順5は保持しない予定の「消したペアと同値の複製」(手順6で消える)も保持扱いで検査する。安全側なので害は小さいが、
利用者は「無変換⇄左Alt を外して変換⇄左Alt にする」が1回でできない。手順5で `removed` に入る値も除外すれば直る。

### N3. 入力順で別のペアに読まれた組へ切り替えると、実質同じ値なのに displace 承認を求める

N1 の1つ目の並びで `無変換⇄変換` に切り替えると、Unclaimed の `(7B,79)`,`(79,7B)` が displaced になり `displace=1` が要る。
書くのは同じ値なので、displaced から「追加するエントリと同値のもの」を除くと確認ダイアログが減る。

### N4. 0 を含む `(0,X)`,`(X,0)` がペアとして読まれる

`detect_swap_pairs` は `b == 0` は除外するが `a == 0` を除外しないので、`[(7B,0),(0,7B)]` が `0⇄無変換`(警告なし)になる。
決定3 の表では `A→0x0000` は Disabled。実害は小さい(削除で2件消えるだけ)が、表との不一致と UI の「?⇄無変換」表示になる。
`a == 0` も除外するのが表どおり。

### N5. 段階4(UI)への申し送り

- `expected` は**生の値を順に読んだ列**(`parse_entries`/`parse_entries_strict` の結果そのまま)を渡すこと。`detect_swap_pairs` の
  結果から組み立て直すと順序が変わり常に `Changed` になる。現状 UI 側の読み取りは寛容な `parse_entries` で、
  厳密パースで壊れている値でも一覧を出してしまい、UAC 承認後に初めて `ExistingCorrupt` が返る。読み取り時に
  `parse_entries_strict` を使い、壊れた値は適用ボタンを出す前に知らせるとよい。
- `ElevationOutcome::Rejected(exit)` の文言が `{exit:?}`(列挙子名)そのまま。各終了コードに対応する利用者向けの文言が要る。
- `count=1` で終端が無い・`count=0` などの値は永久に `ExistingCorrupt` で、awase からは直せない(手動 regedit のみ)。
  「壊れた値を削除する」明示操作を用意するかは UI で決める。

### N6. 昇格ワーカー(Windows 専用部分)の確認結果

- `ShellExecuteExW` の `lpParameters` はシェルを通らないので `;` `>` `=` は解釈されない。spec は空白・引用符を含まず、
  Rust の Windows 引数分解(`CommandLineToArgvW` 相当)で1引数のまま届く。長さはエントリ1件あたり約10文字で、
  32,767 文字の上限には現実的に届かない。
- 終了コード: `i32::try_from(u32).unwrap_or(i32::MAX)` → `from_code` 未知値 → `Failed`。panic(101)・NTSTATUS も `Failed` に落ちる。
  問題なし。ただし `WaitForSingleObject` の戻り値を見ていない(既存コードから)ので、待機失敗時は `STILL_ACTIVE`(259)を
  `Failed` と読み、その後ワーカーが書き終える可能性がある。UI は失敗時も再読込するので実害は小さい。
- 書き込みエラー時は巻き戻さず `Failed` を返す。レジストリ値の書き込みは値単位で原子的なので部分書き込みは起きないと見てよい。
  読み戻しが `Err` のときは巻き戻しへ進む(保守的で妥当)。`sm::delete()` は値が無くても `Ok`(ERROR_FILE_NOT_FOUND を成功扱い)
  なので、元が無かった場合の巻き戻しで正しく動く。巻き戻し後の確認 `now.as_deref() == original` は、長さ0の元値
  (`Some([])`)も `read()` が `Some(Vec::new())` を返すので一致する。
- 既存の `--scancode-map` は `launch_elevated_windows` 共通化後も `Ok(0)→Success / Ok(非0)→Failed / 起動失敗・キャンセルは従来どおり`
  で挙動は変わっていない。`main.rs` の分岐順(`--scancode-map` が先)も既存動作に影響しない。

### N7. セキュリティ

権限昇格の経路にはならない: 書き先は固定の HKLM 値1つ、新規に作れるのは許可リスト内のペアと固定の `Caps→LCtrl` だけ、
他のエントリは保持か(承認つきで)削除のみ。昇格側は `compute_swap_write` で許可リスト・自己ペア・重複・衝突を再検証している
(決定5 どおり)。UAC が境界で、昇格を承認させられる呼び出し元は reg.exe でも同じことができる。`displace` は真偽値だが、
`expected` の一致で displaced 集合が一意に決まるので十分。巨大入力はコマンドライン長で上限があり、O(n²) の処理でも問題ない。
ただし B1 のとおり、昇格側の再検証は「悪くならない」を保証していない(`compute_swap_write` 自体の穴)。

### N8. 規約・ツール

- `parse_entries_strict` はヘッダの Version/Flags を見ない・終端の後ろの余分なバイトを許す(他ツールの値を誤って壊れ扱いしない)。
  妥当。
- 新しい純粋モジュール `scancode_pairs.rs`/`scancode_apply.rs` は `.cargo/mutants-awase-windows.toml` の `examine_globs` に無い
  (既存の `scancode_map.rs` も無いので一貫はしているが、純粋ロジックで変異テストの価値が高い)。
- 決定2 は許可リストを `scancode_map.rs` に置くとしているが実装は `scancode_pairs.rs`(実害なし、ADR の記述を合わせる程度)。
- clippy(pedantic/nursery)は未実行。`fix-requires-evidence` の再発ファミリーには該当しない(feat、対象ファイル外)。
