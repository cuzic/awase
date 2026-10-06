# ADR-230 段階4(PR #540, 3e8c4427)Opus 敵対的コードレビュー round1

対象: `git diff origin/develop...HEAD`(feat/adr230-scancode-ui)。読み取りと分析のみ。
検証: `scancode_map.rs`/`scancode_pairs.rs`/`scancode_apply.rs`/`scancode_editor.rs` を scratchpad で単体コンパイルし、
`rustc --test`(71 件 PASS)、`clippy-driver`(`clippy::all`/`pedantic`/`nursery` を deny)、小さな探索プログラムを実行した。
探索では、キー4つ(Caps/左Ctrl/無変換/スペース)と0の全組み合わせで既存エントリを最大3件作り、編集後のペア集合と Caps 追加 Ctrl の全組み合わせを試して
「`is_dirty` が真なのに書く内容が既存と同じ」「`is_dirty` が偽なのに書く内容が既存と違う」が起きるかを調べた。どちらも 0 件だった。
`is_dirty` と `compute_swap_write` の no-op 判定は食い違っていない。

## Blocker

なし。レジストリを壊す経路・比較交換(`expect`)の取り違え・`allow_displace` の素通りは見つからなかった。
- `expected` は常に `loaded.entries`(`read_raw_entries` が `parse_entries_strict` で生の値を順に読んだ列)をそのまま渡している。
  プレビューも同じ列で計算しているので、昇格側の `decide` と UI の `displaced` は同じものになる。
- `allow_displace = !plan.displaced.is_empty()` が真になるのは `needs_confirmation()` が真のときだけで、その場合は必ず確認ダイアログを通る。
  ダイアログを通らない経路では displaced は空なので、`allow_displace` も偽になる。

## Should-fix

### S1. US 配列でもワンクリック「変換 ⇄ スペース」を押せて、スペースが消える
`scancode_editor.rs` の `quick_available`/`add_quick` は `jis` を見ていない(`candidates` だけが JIS 専用キーを除いている)。
`main.rs::scancode_editor_ui` もボタンを `keyboard_model` と無関係に出している。
探索で確かめた結果: `candidate_keys(false)` は変換を含まないのに、`quick_available(&QUICK_PAIRS[1])` は真、`add_quick` も真、`preview(...).plan` も `Ok` だった。
失敗シナリオ: US キーボードの利用者がこのボタンを押して適用し、再起動する。物理スペースは変換(0x79)を出すようになり、
変換は物理キーが無いので、スペースを入力する手段が無くなる。ADR-230 決定2の「JIS 専用キーは JIS 配列のときだけ候補に出す」にも反する。
→ `quick_available(&self, quick, jis)` で `is_jis_only` を見る。UI では US 配列のとき出さないか、無効にする。

### S2. 確認ダイアログは非モーダルなので、ダイアログを開いた後の編集が黙って捨てられる
`show_scancode_apply_confirm_modal` は `egui::Window`(Foreground)で、背後の行の編集・「削除」・「元に戻す」・「読み直す」・適用ボタンは操作できる。
この点はリポジトリの既存コメント(`main.rs` 1394 行付近・7944 行付近のテスト)が同じ型の指摘として残している。
`ScancodeApplyConfirm.request` はダイアログを開いた時点の写し。
失敗シナリオ: ダイアログを開いたまま行を1つ足す(または消す)。その後「適用する」を押すと、写しの古いペア集合が書かれる。
成功するとレジストリを読み直すので、足した行は何の知らせも無く消える。メッセージは「適用しました」なので気づけない。
「読み直す」を押した後でも、レジストリが同じなら写しがそのまま通る。
→ ダイアログ(と再起動確認)を出している間はセクションを `add_enabled_ui(false, ..)` で止める。
または「適用する」を押した時点で `request.pairs`/`caps_extra_ctrl` が今の `editor` と一致するかを確かめ、違えばダイアログを閉じる。
既存の `cancel()` がモーダルを閉じるのと同じ型。

### S3. UAC キャンセル・起動失敗でも読み直して、編集内容が消える
`run_scancode_apply` は結果に関係なく `self.scancode_map_view = Some(load_scancode_view())` を実行し、`EditorState` を作り直す。
`Cancelled`(UAC で「いいえ」)と `LaunchError` では昇格側が動いておらず、レジストリは変わっていない。それでも利用者の編集(複数行)が全部消える。
UAC を誤って閉じた直後にもう一度押す、というよくある操作で編集のやり直しになる。
ADR-230 の「失敗後は必ず読み直す」が目的にしているのは、昇格側が書いた可能性がある場合の表示の正しさなので、`Cancelled`/`LaunchError` には当たらない。
→ この2つのときは view を保持する(`expected` が古くなっていても、次の適用で `Changed` になるだけで安全)。
`Rejected(Invalid | DisplaceNotApproved | BadArguments)` も書き込み前に止まっているので、保持してよい。

### S4. 親指キーの判定から「Left Alt/Right Alt」と、かなキー(かな/ひらがな/カタカナ)が漏れている
`main.rs::thumb_key_scancode` は無変換・変換・スペースしか見ていない。
- `ALT_IMPERSONATION_OPTIONS`(`"Left Alt"`/`"Right Alt"`)は左右の親指キーとして GUI で選べる。対応する 0x0038/0xE038 は `ALLOWED_SCANCODES` に入っている。
  失敗シナリオ: 親指キーを Left Alt にしている利用者が「無変換 ⇄ 左 Alt」を作る。注意書きも確認も出ないまま、親指シフトの物理位置が無変換へ移る。
- `THUMB_KEY_OPTIONS` の `"かな"(VK_KANA)`・`"ひらがな"(VK_DBE_HIRAGANA)`・`"カタカナ"(VK_DBE_KATAKANA)` は、JIS では物理的にかなキー(0x0070)。
  0x0070 も許可リストに入っているので、同じく警告が出ない。
ADR-230 決定2の「X が NICOLA の親指キー(config の実際の値から決める)」を満たしていない。
→ `"Left Alt"`/`"Right Alt"` を先に文字列で判定する(`hook.rs::resolve_thumb_key` と同じ目印)。VK_KANA/VK_DBE_HIRAGANA/VK_DBE_KATAKANA は 0x0070 にする。
`thumb_key_scancode` のユニットテストも足す(今は無い。表記ゆれの `"Space"`/`"VK_SPACE"`/`"無変換"`/`"VK_NONCONVERT"`/`"Left Alt"` を含める)。

### S5. 注意書きが「既に適用済みのペア」にも毎回出る
`EditorState::preview` は `cautions(&self.pairs(), ..)` で、編集後の全ペアを対象にしている。
探索で確かめた結果: レジストリに既に「変換 ⇄ スペース」があり、親指キーが変換のとき、無関係な「左 Alt ⇄ かな」を足すだけで
`[ThumbKeyMoved{変換}, SpaceMoved]` が出て `needs_confirmation()` が真になった。
適用のたびに同じ警告と確認が出ると、利用者は警告を読まずに「適用する」を押すようになり、本当に新しい危険(CapsWithThumbKey 等)が埋もれる。
逆に、親指キーを含む既存ペアを**削除**するときは、物理位置がまた変わるのに何も出ない(探索で `[]` を確認)。
→ 対象を「初期集合との差分(足したペア+外したペア)」にする。外したペアの文言は「元の位置に戻ります」とする。

### S6. 利用者向け文書と、旧 ADR の status が撤去を反映していない
- `docs/usage.html` 636〜645 行が「Caps(英数) / Ctrl プリセット」の3択(無効/入れ替え/追加)の説明のまま残っている。画面にはもう存在しない。
- `docs/adr/111-*.md`・`docs/adr/126-*.md` の frontmatter `status` と `docs/adr/index.md` の 111/126 行が
  「実装済み(…`scancode_map.rs`…が現存)」のまま。プリセット機構(`ScancodeMapPreset`/`--scancode-map`)は撤去したので、
  「ADR-230 段階4で UI と CLI を撤去・一般化(旧 Swap はペア、旧 CapsAsExtraCtrl はチェックボックスとして読める)」と追記すべき。
  docs-frontmatter-convention の「status は全文を frontmatter に、index は短縮」の流儀で更新する。
- `.github/workflows`・`scripts/` には `--scancode-map` の呼び出しは無かった(grep で確認)。tray.rs ほかのコードにも旧 API の参照は残っていない。

### S7. `scancode_map_section` の doc コメントに旧プリセットの説明が残っていて、新しい説明と矛盾している
`main.rs` 3139〜3146 行で、旧 doc「Caps(英数)⇔Ctrl 入れ替え / … プリセット(ADR-111 / ADR-126)…撤回済み」の直後に、
新 doc「「キーの入れ替え」セクション(ADR-230)…」が続いている(古い段落を消し忘れている)。
同じく `main.rs` の `--scancode-pairs` 分岐のコメント「(`--scancode-map` と同型のヘッドレス分岐)」は、撤去したフラグを参照している。

## Note

- N1. `update()` 冒頭のコメント「ADR-126 D6: ウィンドウを閉じる操作には config/layout どちらの未保存確認も入れない」は残っている。
  今回キーの入れ替えだけ閉じる時の確認を足したので、この例外(ADR-127 追記)をコメントに一言足すと、次の読み手が D6 違反と誤解しない。
  なお、破棄確認で「破棄して閉じる」を選ぶと、config 側の未保存の編集も確認なしで失われる(D6 どおりだが、文言からは分からない)。
- N2. 閉じる確認のループは起きない。「破棄」で `scancode_map_view = None` にしてから `Close` を送るので、次のフレームの `close_requested` では
  `scancode_has_unapplied_changes()` が偽になる。X ボタンで確認ダイアログを閉じた場合(`!open`)は「戻る」と同じ扱い。いずれも問題なし。
- N3. 「読み直す」(Loaded 時)は、未適用の編集を確認なしで捨てる。あわせて `scancode_map_last_message` も消すので、成功後に押すと
  「再起動が必要」の文が消え、「今すぐ再起動…」ボタンだけが残る(ADR-127 追記の「再起動後に有効と明示」が弱まる)。
  再起動待ちの文は `scancode_restart_pending` から常に出すほうが確実。
- N4. `scancode_restart_pending` は `Success` のときだけ立つ。`RollbackFailed`(レジストリが書いた値のまま残っている可能性あり)と、
  書いた後に他の書き手が書いた `Changed`(ForeignWrite)では、起動時と違う値がレジストリにあるのに再起動ボタンが出ない。
- N5. `request_restart` の `Command::new("shutdown")` について:
  - GUI サブシステムの exe からコンソールプログラムを起動するので、一瞬コンソール窓が出る(`CREATE_NO_WINDOW` を付けると出ない)。
  - Rust の Windows 版 `Command` は exe と同じディレクトリを先に探すので、`%SystemRoot%\System32\shutdown.exe` の絶対パスにするほうが堅い。
  - 確認文「保存していない作業は失われます」は、この設定画面自身の未保存の設定についても同じことが言える。設定が未保存なら一言出してもよい。
- N6. `run_scancode_apply` は UI スレッドで `WaitForSingleObject(INFINITE)` を呼ぶ(旧プリセットから引き継いだ形)。
  UAC ダイアログを出している間、設定ウィンドウは「応答なし」になる。既存の挙動なので今回の差分の問題ではない。
- N7. 表示名が配列に合わせていない。`key_label(0x0029)` は US 配列でも「半角/全角」と出る(ADR-230 決定2「表示名も配列に合わせる」。US では `` ` ``)。
  他ツールのエントリ一覧の `A→0x0000`(無効化)は「0x0000」と出るので、「(無効)」と出すほうが読める。
- N8. 起動時から「Caps 追加 Ctrl」と「左 Ctrl を含む他ツールのペア」(例: 左Ctrl⇄左Alt)が同居している状態は、
  `from_detected` がそのまま両方を持つ(探索で確認: caps=true・行に 0x1D・`caps_extra_available()=false`)。
  その行の反対側を変えると、候補の絞り込みは通るが `compute_swap_write` が `CapsExtraConflict` を返し、適用時にエラー文になる。
  安全側の挙動だが、行を選ぶ段階では分からない。`set_key` は候補の絞り込みを強制しないが、UI からは candidates 経由でしか呼ばれないので、新たにこの状態を作る経路は無い。
- N9. ComboBox の id に行の index を使っている(`("scancode_pair", i, is_b)`)。行を消すとポップアップの状態が隣の行へずれうるが、
  ポップアップを開いたまま「削除」は押せないので、実害は見当たらない。どちらの側も「未選択」に戻す選択肢は無い(行削除で代替できるので許容範囲)。
- N10. 確認ダイアログの文面は注意点と他ツールのエントリだけで、「何と何を入れ替えるか」の一覧は出ない。適用する内容の要約を1行ずつ出すと確認の意味が強まる。
- N11. clippy: `scancode_editor.rs` 自体は `clippy::all`/`pedantic`/`nursery` の deny で警告ゼロ(本 PR の範囲外で、`scancode_pairs.rs::compute_swap_write` は
  `too_many_lines`(102/100)に当たった。PR #501 由来。CI の windows-build clippy で通っているなら、手元の toolchain(1.98.1)との差。一応確認を)。
  `unreachable_pub` は `pub mod scancode_editor` なので問題なし。layer_boundary/architecture_guard に関係する変更は無い(純粋モジュールで windows-rs 非依存)。
- N12. `.cargo/mutants-awase-windows.toml` の `examine_globs` に `scancode_pairs.rs`/`scancode_apply.rs`/`scancode_editor.rs` が入っていない。
  プラットフォーム非依存の純粋モジュールなので、ミューテーション検査の対象に足せる。
