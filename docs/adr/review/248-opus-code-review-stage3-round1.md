# ADR-248 段階3(置換への一般化)コードレビュー round1(Opus、敵対的)

- 対象: PR #606、ブランチ `feat/adr248-stage3-permutation`、HEAD `0c666a66`(`git diff origin/develop...HEAD`)
- CI: `gh pr view 606`(head `0c666a6650fa`)で全ジョブ SUCCESS・`mergeStateStatus=CLEAN`(2026-10-11 確認)
- 再現: scratchpad `rv3/t.rs`(`#[path]` で `scancode_{map,pairs,apply,editor,diagram}.rs` を並べ `rustc --edition 2021 --test`)。
  下の r1〜r8 はすべて実行して結果を確かめた(既存テスト 120 本も同じバイナリで全件 pass)。
- 実機(UAC・再起動を通した書き込み)は未確認。

## 結論

核(`compute_swap_write_with_cycles`)の安全性の骨格は保たれている。新規の単位は from を他ツールから奪い(displaced)、to の衝突は拒否し、
単位どうしはキーが素なので、書いた結果が「一方向だけ」「多対一」になる経路は見つからなかった(r6: 4キーの置換 24×24=576 遷移で
書いた結果が要求の置換と完全一致、r7: 許可リスト外・0・恒等・重複・Caps を含む乱数 20 万件で「恒等は恒等」「全削除は通る」
「要求した単位は必ず読み戻せる」を確認)。ADR-230 コードレビュー round1 B1 型(同値の複製の値削除が単位の片方を消す)は、
閉じた循環が「from も to も全エントリで1回だけ」でしか読まれないため、循環のエントリに同値の複製が存在し得ず、再発しない。

一方で、**「書いたものが、書いた意図どおりに読み戻される」**が Caps 追加 Ctrl で崩れ(S1)、テストはそれを許す形に緩められている。
古い形式の要求が既存の循環を確認なしで消す(S2)、循環の注意書きが向きを見ない(S3)、図で通るドロップが適用で拒否される(S4)、
ADR の必須事項(ADR-230 表の追記・バイト列の固定テスト・常時の差分表示・`NotPermutation`/`perm=` からの変更の記録)の未了(S6)がある。
Blocker は無し。

---

## Blocker

なし。

(検討して Blocker にしなかったもの: 書き込み結果が置換でなくなる経路、単位の一部だけが消える経路、昇格側の再検証のすり抜け、
`decompose` の無限ループ・パニック。根拠は各節。)

---

## Should-fix

### S1. Caps 追加 Ctrl を足すと、他ツールの開いた鎖が閉じて awase の循環に読み替わる(書いた意図と読み戻しが食い違う)

- 根拠: `scancode_pairs.rs:477-479`(`adds_caps` で `(Caps, 左Ctrl)` を足す)、`:494-502`(`replaced` の特別扱いは長さ2の
  `左Ctrl→Caps` だけ)、`:510-518`(to の衝突検査は `to == 左Ctrl` しか見ない)。`detect_swap_pairs` は循環を Caps 追加 Ctrl より先に見る(`:253-255`)。
- 失敗シナリオ(r1、実行して確認):
  `existing = [(左Ctrl→無変換), (無変換→Caps)]`(他ツールの2件、読み込み時は Unclaimed 2件)で「Caps を Ctrl としても使う」をオン。
  `compute_swap_write_with_cycles(existing, [], [], true)` は `Ok`・`displaced=[]`(確認ダイアログ無し)。書いた結果を読むと
  `cycles=[Caps→左Ctrl→無変換→Caps]`・`caps_extra_ctrl=false`・`unclaimed=[]`。`decide` も `Write` を返す(昇格側でも止まらない)。
  r7 の乱数でも 159 件の読み替わりがあり、主な型はこれ(許可リスト外キー `0x0010` を経由する鎖 `左Ctrl→0x0010→Caps` も閉じる)。
- 実害: (1)適用→読み直しでチェックがオフに戻り、図に身に覚えのない3巡回が出る。(2)他ツールの2件が「awase の入れ替え」に
  分類が変わり、以後「戻す」「すべて解除」で **displace の確認なしに消える**(`removed_own` は `displaced` に入らない、`:484-489`)。
  ADR-248 非目標「他ツールのエントリを awase が消すこと」への抜け道。長さ2の `左Ctrl→Caps` を `replaced` に入れている(`:500`)のは
  まさにこの読み替わりを防ぐためで、長さ3以上で同じ穴が再発している。
- テストの緩め: `scancode_pairs.rs:1005-1010`・`:1138-1143` で「閉じた循環に読み替わった」を許容条件に足しており、この穴を性質テストが検出できなくなっている。
- 推奨修正(どちらか):
  - (a) 一般形: `compute_*` の最後に **読み戻し分類の一致**を事後条件として置く。`detect_swap_pairs(&out)` の単位集合が
    「要求した単位」と、caps が `caps_extra_ctrl` と、unclaimed が「残した他ツールのエントリ」と一致しなければ `SwapError::Reclassified`
    (仮)で拒否する。昇格側の `decide` も同じ関数を通るので両方で効く。将来の同型の穴もまとめて塞がる。
  - (b) 局所形: `adds_caps` のとき、残る他ツールのエントリだけで `左Ctrl` から to をたどって `Caps` へ戻る鎖があれば拒否
    (または鎖の先頭 `左Ctrl→X` を `replaced` に入れて displaced の確認に回す)。
  - いずれでも、テストの緩め(上記2箇所)を戻し、r1 を固定ケースとして足す。

### S2. 古い形式の要求(`cycles=` 無し)が、レジストリの既存の循環を確認なしで消す

- 根拠: `scancode_apply.rs:196`(`cycles: cycles.unwrap_or_default()`)、`decide`(`:276-305`)は空の循環で `compute_*` を呼ぶ。
  `compute_*` は「要求に無い既存の単位は削除」(`scancode_pairs.rs:484-489`)。
- 失敗シナリオ(r2、実行して確認): レジストリ `[(0010→0011),(0011→0012),(0012→0010),(無変換→左Alt),(左Alt→無変換)]`、
  要求 `pairs=0038-007B;caps=0;expect=…;displace=0`(段階2以前の形式)。`decide` は
  `Write { [(無変換→左Alt),(左Alt→無変換)] }`、つまり 3巡回の3件を `displace=0` のまま消す。
  段階2以前の UI はこの3件を「他のツールが設定している項目(awase は変更しません)」として保持するつもりで要求を作っている。
- 起こりうるか(推測): 昇格側は `current_exe()` の自己起動(`scancode_map_admin.rs:165`)なので通常は同じ版。ただし Scoop の
  `current` ジャンクション経由で起動した設定画面を開いたまま更新すると、`GetModuleFileNameW` がジャンクションのパスを返し、古い UI が新しい
  ワーカーを起動しうる(未確認)。逆向き(新 UI → 古いワーカー)は `UnknownKey("cycles")` で `BadArguments`(書かない)なので安全。
- 推奨修正: `cycles=` が**省略されたとき**は「既存の循環に触れない」(= `detect_swap_pairs(existing).cycles` を要求に含めたものとして扱う)
  にする。新 UI は常に `cycles=`(空でも)を出す(`to_spec` の `if self.cycles.is_empty()` 分岐をやめる)。互換テスト
  `spec_with_cycles_round_trips_and_old_specs_without_cycles_still_parse` に「古い形式 + 既存の循環 → 循環が残る」を足す。

### S3. 循環の注意書きが向き(位置 → 機能)を見ていない(ADR 決定6「注意書きは位置 → 機能で書き直す」「cautions の向きのケース」が未了)

- 根拠: `scancode_editor.rs:662-689`(`cautions_with_cycles` は「循環が Caps を含むか」「親指キーを含むか」だけを見る)。
- 失敗シナリオ(r5、実行して確認):
  - 親指キー=無変換。`Caps→無変換→スペース→Caps`(Caps の位置は無変換、無変換の位置はスペース、スペースの位置は Caps を出す)と
    逆向きの `Caps→スペース→無変換→Caps` が、どちらも `[CapsWithThumbKey{無変換}, SpaceMoved]`。文面(`:695-699`)は
    「英数の Shift+CapsLock の動きが『無変換』の位置へ移る」だが、前者では英数の機能はスペースの位置へ移り、無変換の位置へは移らない(誤情報)。
  - 親指キー=無変換・変換で `無変換→変換→スペース→無変換`: 左親指の位置が右親指の機能を出すのに `ThumbKeysSwapped` が出ない
    (`ThumbKeyMoved`×2 だけ)。ペアでは出る(`:641-643`)。
- 推奨修正: 循環を「位置 → 機能」の対応 `(p, σ(p))` に展開してから判定する。`CapsWithThumbKey` は「Caps の機能が親指キーの位置へ行く
  (σ(thumb) == Caps)」、`ThumbKeysSwapped` は「親指キーの位置が他方の親指キーの機能を出す」で判定する。ペアの `cautions` も同じ関数に寄せる。
  向きの2ケース(上の2つ)を固定テストにする。

### S4. 図で通るドロップが、適用の段で `ime_role_conflict`/`caps_filter_conflict` に拒否される(判定と表示が別の関数)

- 根拠: `drop_function`(`scancode_diagram.rs:334-378`)は位置の `Locked` と `compute_*` しか見ない。`ime_role_conflict`(`scancode_editor.rs:290-301`)・
  `adds_caps_ctrl_mapping`(`:316-331`)は「読み込み時に無かった単位に、そのキーが含まれるか」で判定する。循環では、キー K の位置も機能も
  変えないドロップでも、K の属する単位の中身が変わると「新しい単位」になる。ペアでは単位を変えるには K の位置に触る必要があったので起きなかった。
- 失敗シナリオ(r4、実行して確認): 読み込み時 `無変換→変換→スペース→左Alt→無変換`、変換が IME の ON/OFF を兼ねる親指キー
  (位置「変換」は `Locked{ThumbImeRole, releasable:true}`)。スペースの位置の機能(左Alt)を左Alt の位置へ戻すドロップは
  `drop_function` で `Ok`(結果 `スペース→無変換→変換→スペース`、変換の位置の機能も、変換の機能を出す位置も変わらない)。
  `set_units` 後の `ime_role_conflict()` は `true` で、「入れ替えを適用」で `Unavailable::ThumbImeRole` の文面が出て止まる。
  ドロップ直後には何も出ない。Caps フィルタドライバ + Caps を含む循環でも同型(`caps_filter_conflict`)。
- 安全側に倒れる(書かない)ので Blocker ではないが、ADR 決定5「判定と表示を同じ関数にする」に反し、理由の文面(「キー設定の IME ON/OFF から外すと
  設定できます」)も的外れになる。
- 推奨修正: 判定を「単位が新しいか」ではなく「そのキーの σ(位置の機能)または σ⁻¹(機能の位置)が読み込み時から変わったか」にし、
  `drop_function` にも同じ判定を入れて `DropRefusal` で返す。最小なら、IME 兼用キー・Caps/左Ctrl(フィルタあり)を含む循環は**循環全体**を
  `Locked{releasable:true}` にする(部分的なドロップを図で止める)。

### S5. ワンクリックで足したペアの位置が、他ツールの from があっても `Editable` になり、図のドロップが displace を伴う(段階2からの穴が循環で広がる)

- 根拠: `quick_unavailable`(`scancode_editor.rs:411-428`)は他ツールのエントリを見ない(`EditorState` が unclaimed を持たない)。
  `position_state`(`scancode_diagram.rs:274-288`)は「自分の単位に属する位置」では他ツールの from/to を見ない(警告つきペアは `detected.pairs` に
  あるものだけ)。
- 失敗シナリオ(r8、実行して確認): レジストリ `[(スペース→0x003B)]`(他ツール)。ワンクリック「変換 ⇄ スペース」は足せる。以後スペースの位置は
  `Editable`。無変換の機能をスペースの位置へ落とすと `Ok`(3巡回)、計画の `displaced=[(スペース→0x003B)]`。ADR 決定5「図からは他ツールの
  エントリを displace しない」に反する(確認ダイアログは出るので無言ではない)。段階2(ペアの核)でもワンクリック自体が displace しうる既存の穴だが、
  段階3でその位置を起点に循環を作れるようになった。
- 推奨修正: `position_state` で、読み込み時の単位(`detected.pairs`/`detected.cycles` に値で一致するもの)以外に属する位置にも他ツールの
  from/to の検査を当てる。ワンクリックも同じ判定(`position_state` が `Editable` でない位置を含むなら `Unavailable::OtherTool` 等)にそろえる。

### S6. ADR の必須事項・記録の未了

- `docs/` の差分が無い(`git diff origin/develop...HEAD --stat` に docs が無い)。ADR-248 段階3は「ADR-230 に改訂を追記する」(決定6・段階3)。
  未了の項目:
  - **ADR-230 決定3 の分類表に行を足す**(決定6「分類の優先順位と単位」): 実装の決定は「ペアが先(`A→B, B→A, B→C, C→A` は `A⇄B` +
    Unclaimed 2件)」「循環が Caps 追加 Ctrl より先(`Caps→左Ctrl, 左Ctrl→X, X→Caps` は3巡回)」。後者は S1 と絡むので、表に書く前に S1 の扱いを決める。
  - **昇格側の検査の形の変更の記録**: ADR は `perm=…` と `SwapError::NotPermutation`(from/to の重複・from 集合 = to 集合・固定点なし)を
    定めたが、実装は `cycles=a>b>c`(型 `Cycle` が重複・0・長さ<3 を拒否、単位間は `DuplicateKey`)。単位の和は構造上つねに置換になるので
    `NotPermutation` は不要という判断は妥当だが、ADR に書かれていない。
  - **バイト列までの非対称3巡回の固定テスト**(決定6「必須テスト」): 無い。ペアは対称なので from/to の取り違えがこれまで観測できなかった。
    循環は取り違えると**逆向きの置換**になる。`scancode_map::build_bytes(&Cycle::new(&[無変換,変換,スペース]).entries())` の期待バイト列
    (`to, from` の順の LE。Microsoft の文書どおりなら `[79 00 7B 00][39 00 79 00][7B 00 39 00]` の順を含む)と、`parse_entries_strict` で
    戻したときの `detect_swap_pairs` が同じ向きの `Cycle` になることを固定する。`settings-screenshot.yml` の `cycle` シナリオは
    PowerShell 側の想定とパーサの整合しか見ていない。
  - **変わる位置の常時の差分表示**(決定6「確認ダイアログ」): 「適用ボタンの直上に位置 → 機能の一覧」が無い。3巡回で注意の対象を含まないもの
    (例 `左Alt→左Ctrl→右Alt→左Alt`)は確認ダイアログも出ないまま適用される。ADR は「長さ3以上の循環を含むときに確認を足すかは実機で決める」
    としているので、少なくとも差分表示を入れるか、未了として ADR の実装状況に書く。
  - ADR-248「実装状況」に段階3の行が無い。
- 推奨: 上記を ADR-248 実装状況と ADR-230 への追記で記録する(コードの変更を伴わないものは同じ PR で足せる)。

### S7. 「戻す」が循環を丸ごと解くことが、ボタンから分からない

- 根拠: `release_position`(`scancode_diagram.rs:420-436`)は位置を含む単位を丸ごと消す。`main.rs:5473` のボタンは「戻す」とだけ表示。
  一方、同じ位置の機能を自分の位置へ落とすと、その位置だけが恒等に戻り循環は1つ縮む(`drop_function` の `function == pos`)。
- 失敗シナリオ: 4巡回の1つの位置の「戻す」で、利用者が触っていない3つの位置も元に戻る。ADR 決定5 の文言(「この位置を戻す」= その入れ替えを解除)
  どおりだが、ADR 同節が避けたかった「元の位置へ戻せば消えると思う/思わない」の取り違えが逆向きに起きる。
- 安全性: 単位ごとの削除は常に通るので、丸ごと解く設計自体は妥当(位置単位で縮めると S4 の判定や `compute_*` で拒否されうるため、
  解除の経路としては丸ごとの方が確実)。許可リスト外・US 配列で隠れる位置を含む循環も、図(`diagram_rows` が単位内の許可リスト内キーを表示、
  `Locked{releasable:true}`)と一覧(`unlisted_units`)の両方から解ける(`a_cycle_outside_the_allowlist…`・`jis_only_units_on_us…` で確認)。
  ADR-230 round2 B4 型の後退は見つからなかった。
- 推奨: 単位の長さが3以上のとき、ボタンを「この入れ替え(N個)を解除」にするか、`on_hover_text` で戻る位置の一覧を出す。

---

## Nit

### N1. `drop_hint` が、恒等の位置を自分自身へ落としたとき「元に戻ります」と出す

- 根拠: `scancode_diagram.rs:398-402`(`unit_len == 0` の腕が `source == pos` の腕より先)。
- 再現(r3): 何も入れ替えていない状態でスペースをスペースへホバー → 「ここへ落とすと、「スペース」の位置が元に戻ります。」(実際は何も変わらない)。
- 推奨: `source == pos` の腕を先に置く。

### N2. `drop_hint` の「N個のキーの循環になります」は、ドロップが循環を分割して残りが別の単位になる場合を言わない

- 例: 4巡回の中の2位置を入れ替えて 2つのペアに分かれると、pos 側は「入れ替わります」、source 側の単位がどうなったかは出ない。
  互換 (s pos) なので変わるのは2位置だけで、誤りではない。文面として「ほかの位置は変わりません」を足すと、段階2の「解いてから作る」で
  覚えた利用者への説明になる(ADR 決定5「段階2で覚えた操作の結果が段階3で変わるので所有者へ示す」)。

### N3. `set_pairs` が循環を変えずに残る公開 API

- 根拠: `scancode_editor.rs:364`。本番の呼び出し元は `set_units` だけ(`main.rs` は `set_units` のみ、`grep` で確認)。
  循環のキーと重なるペアを入れると不正な状態になる(`compute_*` が `DuplicateKey` で止めるので書き込みは起きない)。
- 推奨: `set_pairs` を非公開にするか、`set_units` に統合する。

### N4. テストのアサーションが弱い・足すべきテスト

- `creating_keeping_editing_and_removing_a_cycle`(`scancode_pairs.rs:1301`): 3巡回 → ペアを `entries.len() == 2` だけで見ている。
  `sorted(entries) == Pair::new(無変換,変換).entries()` で中身まで見る。
- 置換どうしの遷移の全列挙(r6 の 24×24=576、`compute(σ1 の書き込み結果, σ2)` が σ2 と完全一致・unclaimed 空)を足す。今の
  `every_permutation_of_four_keys_round_trips_through_write_and_read` は空のレジストリからだけで、「3巡回 → 別の3巡回」「2ペア → 4巡回」
  「3巡回 → ペア」の古いエントリが消えることを網羅していない(r6 で全件 OK を確認済みなので、固定する価値がある)。
- 乱数・大きい宇宙のテスト: 足すべき。4キー24通りは「他ツールのエントリ・許可リスト外・0・恒等・Caps」と循環の組み合わせを含まない。
  r7 の形(既存 = 10 キーの宇宙から 0〜5 件の乱数エントリ、要求 = 許可リスト内4キーの置換 + caps)で、「恒等」「全削除」「要求した単位は読み戻せる」
  に加え、S1 の修正後は「読み戻しの単位集合が要求と一致」までを性質にする(20 万件で約 2 秒)。決定的にするため固定シードの xorshift で十分。
- `decide` の「循環を含む恒等の要求は `Stop(Ok)`」(既存の循環を `cycles=` で送り返したとき書かない)のテストが無い。

### N5. `settings-screenshot.yml` の push トリガが作業ブランチ名に置き換わっている

- `.github/workflows/settings-screenshot.yml:8` が `feat/adr248-filter-guard` → `feat/adr248-stage3-permutation`。マージ後は無効なトリガが残るだけで
  害は無いが、毎回書き換える運用なら `workflow_dispatch` だけにするか `ci/settings-screenshot` だけに戻す。

### N6. 他ツールが書いた完全な循環が、本 PR から「awase の入れ替え」に分類が変わる

- 段階2以前は `A→B→C→A`(他ツール)を Unclaimed として「awase は変更しません」の一覧に出していた。本 PR では編集できる循環になり、
  「すべて解除」で消える(確認ダイアログは注意の対象を含むときだけ)。ADR 決定6・7 どおり(「入れ替えの形の設定は他のツールが書いたものでも解除する」)
  なので設計上の変更だが、決定7 の「確認ダイアログに、解除する入れ替えの一覧を出す」は「すべて解除」で常に出ていない(`needs_confirmation` は
  注意・displaced・revealed のときだけ、`scancode_editor.rs:730-736`)。段階2からの未了だが、対象が増えたので記録しておく。

---

## 論点別の確認結果(指摘にしなかったもの)

1. **`find_cycles` の分類**(`scancode_pairs.rs:169-218`): 候補は「from も to も全エントリで1回だけ・採用済みペアのキーを含まない・0 と恒等を除く」。
   閉じた循環の from 集合 = to 集合なので、循環のキーに触れる他のエントリは存在しない(循環は常に孤立)。よって他ツールのエントリや複製と
   組み合わさって編集できる入れ替えに誤読されることは無く、読み落としは「重複で循環でなくなる」場合だけ(それは Unclaimed で正しい)。
   `chain.contains(&next)` は「開始点以外へ戻る鎖」で抜けるが、候補内では from が一意なので起きない(防御として妥当)。長さ2の循環は
   先のペア検出で必ず取られるので候補に残らない。読み取り→恒等の書き込みは常に完全一致(`compute_*` で新規・削除が空になる。r7 で 20 万件確認)。
2. **単位化**(`:362-399`・`:409-543`): 「値で残す/消す」「新規だけに許可リスト・Caps 衝突」は循環でも保たれる。`Unit::same` の順序無視の比較は、
   循環は正規化済みで、ペアと循環は長さが違うので取り違えが起きない(副作用なし)。3巡回→別の3巡回・3巡回→ペア・2ペア→4巡回で古いエントリが
   消え新しいエントリが入ることは r6(576 遷移)で確認。削除だけの要求は `added` が空なので手順2・5のどの検査にも当たらず常に `Ok`。
   B1 型の再発なし(結論欄)。
3. **`drop_function`/`decompose`**(`scancode_diagram.rs:141-170`・`:334-378`): σ' = σ∘(s pos) の計算は正しい(`σ'(s)=σ(pos)`・`σ'(pos)=function`、
   恒等になった位置を落とす)。`decompose` は `chain` が互いに異なる値だけを積み、値は `sig` の to から来るので有限で終わる。添字アクセスが無く
   パニックしない。壊れた入力(置換でない)では開いた鎖を黙って閉じた循環にする/`Cycle::new` が `None` なら黙って捨てるが、入力は常に
   互いに素な単位の σ なので到達しない(到達しても後段の `compute_*` が `DuplicateKey` 等で止める)。`position_of`/`function_at_units` は自分の単位
   だけを見る(他ツールの未解釈エントリ・Caps 追加 Ctrl は無視)が、それらに触れる位置は `Locked` なのでドロップの起点・終点にならない。
   `displayed_function` の Caps 追加 Ctrl は Caps の位置だけ左 Ctrl と表示し、その位置は `Locked{CapsExtraOn}` なのでドラッグの payload にならない。
   `unit_len` は「ドロップ後に pos が属する単位の長さ」で、0 ⇔ function == pos(N1 を除き文面と一致)。
5. **`scancode_apply.rs`**: `parse_cycles` は `parse_hex4`(4桁の16進のみ)と `Cycle::new`(長さ≥3・重複なし・0 なし)で、空要素・不正な16進・
   1〜2キー・重複キーをすべて `BadValue` にする。パニック経路なし(`min_by_key(...)?` は長さ≥3の後)。`decide` は `expected` の比較交換のあと
   循環込みで `compute_*` を呼び直すので、許可リスト外の新規の循環・単位間の重複キー・Caps 衝突・他ツールの to 衝突はすべて昇格側で止まる
   (`decide_writes_a_cycle_and_rejects_a_bad_one`)。UI の不具合で不正な cycles が渡ったときの最悪は「既存の循環を要求から落とした = 削除」(S2 と同じ形)で、
   キーボードが使えなくなる書き込み(片方向・多対一)は検査で作れない。IME 兼用・Caps フィルタのガードは UI 側だけ(段階2から同じ)。
6. **`EditorState`**: `is_dirty`・`reset`・`release_all`・`preview`・`caps_extra_available`・`quick_unavailable`・`ime_role_conflict`・
   `adds_caps_ctrl_mapping`・`registry_state` は循環を反映済み。`candidates`(行の UI 用)は循環を見ないが、行の UI は段階2で撤去済み
   (`main.rs` に `candidates`/`set_key`/`add_row` の呼び出しなし)。
7. **`main.rs`**: `ApplyRequest` の `cycles` は `preview` と同じ `editor.cycles()` から作られ、渡し忘れなし(`main.rs:3446-3452`)。
   `load_scancode_view` は `from_detected` 経由で循環を初期化。`scancode_diagram_ui` の `ctx` の借用は最後の `set_units` より前で終わる(CI の型検査で通過)。
   循環の一覧の `names[0]` は循環が長さ≥3なので空にならない。「解除」で `selected` を消していないが、次のクリックは現在の状態から `function_at_units`
   を引き直すので不整合は起きない。
8. **clippy**: CI(`clippy`・`windows-build`・`windows-settings`)が head `0c666a66` で SUCCESS。`#[cfg(test)]` は各ファイル末尾の `mod tests` のみ。

## 実機で確かめるべきこと(ADR-230 段階5 と同じ機会に)

1. **向き**: 非対称な3巡回(例 無変換→変換→スペース: 無変換の位置で「変換」、変換の位置で「スペース」、スペースの位置で「無変換」)を UAC 経由で
   書き、再起動後に3つの物理キーの出力を確かめる。ペアは対称なので、from/to のバイト順の取り違えはこれまで実機でも検出できなかった。
2. **Scancode Map が推移的に適用されないこと**(推測: ペアの入れ替えが実際に動いているので1回だけの写像と考えられるが、3巡回で未確認)。
3. 昇格プロセスへ `cycles=` を含む引数が渡り、`decide` が `Write` → 読み戻し一致 → `Ok` で終わること。読み直しで同じ循環が図に出ること。
4. 循環の「戻す」「すべて解除」→ 適用 → 再起動で、3つのキーがすべて元に戻ること(復旧手順(1)が循環でも効くこと)。
5. SharpKeys・PowerToys(PowerToys はフック方式なので対象外の可能性)など他ツールが書いた3巡回を読み、図・一覧の表示と S1/N6 の扱いを確かめる。
6. Ctrl2cap が入った機種(dragonflyg4)で、Caps を含む循環が作れない・既存は「戻す」だけできること(S4 の図と適用のずれも合わせて見る)。
