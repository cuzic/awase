# ADR-230 段階4(PR #540)Opus 敵対的コードレビュー round2

対象: `git diff 3e8c4427..df6cfac3`。読み取りと分析のみ。
検証: 4ファイルを scratchpad で単体コンパイルして `rustc --test`(74 件 PASS)。
`clippy-driver`(all/pedantic/nursery を deny)で、`scancode_editor.rs` に新しい警告は無かった。
引っかかったのは round1 N11 と同じ `scancode_pairs.rs::compute_swap_write` の too_many_lines(102/100)1件だけ。

## 問われた3点の確認

1. **S3 の `registry_untouched` の分類は正しい。**
   - 読み直さないもの: `Cancelled`・`LaunchError`(昇格側が動いていない)、
     `Rejected(Invalid | DisplaceNotApproved | BadArguments)`(`decide` の中、または引数の解釈で、書き込み前に止まる)。
   - 読み直すもの:
     - `Changed`(`decide` の期待値の食い違いと、書いた後の ForeignWrite の両方。どちらもレジストリは期待値と違う)
     - `ExistingCorrupt`(読み直すと Corrupt 表示になる)
     - `Failed`(読み取り・書き込みの失敗のほか、未知の終了コード=昇格側の異常終了も含む)
     - `RolledBack`/`RollbackFailed`
   - いずれも ADR-230 の「書かれた可能性があれば読み直す」に合っている。
   - 保持した編集の `expected` が古くなっていても、次の適用で `Changed` になるだけで安全。
2. **S2 の `add_enabled_ui(false)` は egui 0.31 の意味として妥当。**
   - 中の Button/ComboBox/Checkbox は反応しなくなる。`clicked()` は偽になり、ComboBox のポップアップも開かない。
   - モーダルの描画(`update` 内)はパネルの描画より前なので、`modal_open` はそのフレームの状態を反映している。
   - 枠の外にある「今すぐ再起動…」ボタンだけは、確認ダイアログを出している間も押せる。
     押すと、2つ目の確認ダイアログが重なるだけで、状態の食い違いは起きない(許容)。
3. **S5 の「初期集合」が更新されるタイミングに穴は無い。**
   - 適用成功・`Changed`・巻き戻しの後は、読み直しで `initial_pairs` もレジストリの今の値になる。
   - 編集を保持する結果(S3)ではレジストリが変わっていないので、古い `initial_pairs` がそのまま正しい。
   - ただし差分の取り方に下の S-a の穴がある。

## Should-fix

### S-a. 親指キーのペアの相手を変えると「位置が変わります」と「元に戻ります」が同時に出る(S5 の直し方の穴)
`preview` は、足したペアに `cautions()` を、外したペアの親指キーに `ThumbKeyRestored` を、互いを見ずに積んでいる。
探索で確認した。既存「無変換 ⇄ スペース」で親指キーが無変換のとき、行の相手を左 Alt に変えて「無変換 ⇄ 左 Alt」にすると、次の2つが同時に出る。
- `ThumbKeyMoved{無変換}`「…物理的な位置が変わります」
- `ThumbKeyRestored{無変換}`「…物理的な位置が、元に戻ります」

同じキーについて矛盾した2文が並ぶ。
→ 足したペアにもその親指キーが含まれるときは、`ThumbKeyRestored` を出さない(`ThumbKeyMoved`/`CapsWithThumbKey` を優先する)。
このケースをテストに足す。

### S-b. `thumb_key_scancode` の Alt 判定が、実際の解決規則(`resolve_thumb_key`)と違う(S4 の直し方のずれ)
`main.rs::thumb_key_scancode` は `"Left Alt"`/`"Right Alt"` を完全一致の `match` で判定している。
コメントには「`hook.rs::resolve_thumb_key` と同じ目印」とあるが、実際の判定は `awase_windows::state::alt_impersonation::resolve_thumb_key` にあり、
`name.trim()` + `eq_ignore_ascii_case` で比べている(ADR-201 決定1)。
config.toml を手で書いた `"left alt"`・`" Left Alt"` は親指キーとして効くのに、注意書きからは漏れる。
- 設定画面は同じモジュールの `is_thumb_key_vk` を既に使っている(`main.rs` 2719・2744 行)。
  `resolve_thumb_key(name)` を呼んで `(vk, impersonate)` から求めるほうが、規則を二重に持たずに済む。
  - `impersonate` が真なら、物理 Alt(左は 0x0038、右は 0xE038)を返す。
  - 偽なら VK から求める(無変換/変換/スペース/かな系)。
- なりすまし時の VK は無変換/変換なので、親指キーを Left Alt にしていても物理の無変換キーは親指として効いている可能性がある。
  そうなら、Alt と無変換の両方を返すべき。`resolve_thumb_key` の戻り値で `vk` も見れば自然に両方入る。ここは実際の挙動を確かめてから決めること。
- テストに `"left alt"`(小文字・前後の空白)を足す。

## 収束扱いでよいもの

- S1・S6・S7・N1・N3・N4・N5・N7 は、意図どおりに直っている。
- usage.html の説明、ADR-111/126 の status と index も新しい画面・状態に合っている(docs-frontmatter-convention の「旧 status を残して追記」の形も守っている)。
