---
id: ADR-218
title: |-
  architecture_guard.rs の「ファイル別・出現数の固定」型ガードを、宣言テーブル + 汎用チェッカーにする
summary: |-
  `crates/awase-windows/tests/architecture_guard.rs`(5,914 行、`#[test]` 110 本)のうち、`&[(&str, usize)]` / `&[(&str, &[(&str, usize)])]` の
  「ファイル → needle の出現数」表を作って `assert_eq!` するだけの同型テストが少なくとも 17 か所ある。これを `GuardRule { id, needle, scope, sites, adr, why }` の
  宣言テーブル 1 つと、チェッカー 1 本に寄せる。構造検査(`extract_fn_body` / `extract_all_balanced_blocks` を使う 67 か所の呼び出し)は対象外で Rust のまま残す。
  新しい DSL 構文は作らず、Rust の const 配列(パーサ不要)で始める。パイロットは 5 ルールで、行数が減らなければ中止する。
status: |-
  提案(2026-10-02)。未レビュー。実装なし。
related_adr:
  - "ADR-129"
  - "ADR-158"
  - "ADR-161"
  - "ADR-162"
---

# ADR-218: architecture_guard.rs の「出現数固定」型ガードを宣言テーブルにする

## 背景

`architecture_guard.rs` は 5,914 行・`#[test]` 110 本で、リポジトリで最大級のテストファイルである。
その中に次の形のテストが繰り返し現れる(2026-10-02、`origin/develop` を grep)。

```rust
let known_sites: &[(&str, usize)] = &[("src/hook.rs", 1), ("src/runtime/mod.rs", 1), ...];
for (path, expected) in known_sites {
    let content = read_crate_file(path);
    let count = count_real_calls(&content, "thumb_down_timestamps(");   // または production.matches("InputRelay").count()
    assert_eq!(count, *expected, "{path} ... ADR-129 参照");
}
```

`known_sites` / `checks` / `expectations` / `table` の名前で、この型の表を持つテストは行 211, 532, 637, 1024, 1209, 1265, 1896, 1945, 2321, 3570, 3633, 3790, 4158, 4355, 4375, 4411, 5700 の 17 か所。
違うのは (a) needle、(b) 数え方(`count_real_calls` か `production_code_only` + `matches` か `non_comment_lines` + `matches`)、
(c) 表の中身、(d) エラー文に書く ADR 番号と理由、の 4 つだけである。

この形には、実際に出ている問題が 3 つある。

1. **数え方がテストごとに違う**。`input_relay_profile_wiring_occurrence_counts_are_pinned`(4158 行)は `production.matches("InputRelay").count()` で、
   コメント中の `InputRelay` も数える。そのため表の注記に「周辺コメントに残っているので件数は変化しない」と書く必要が出ている
   (`ime_controller.rs` は 1、`open_chain.rs` は 4)。コメントを 1 行直しただけで落ちる、または落ちない(別のコメントと相殺する)。
   一方 3570 行のテストは `non_comment_lines` でコメントを落としてから数える。同じ意図で、数え方が揃っていない。
2. **「なぜこの数か」が表の外にある**。各エントリの理由は行末コメントや前後の `///` に散らばっていて、エラー時に表示されない。
   エラー文は ADR 番号を手書きで繰り返している。
3. **`lints/actuation_call_guard::RESTRICTED_CALLS`(dylint)との二重管理**。同じ「`set_ime_open` の呼び出し元」を、architecture_guard の正規表現と dylint の許可リストの両方が持つ。
   dylint のドキュメント自身が「正規表現ベースの architecture_guard が構文の違いで見落とすケースを塞ぐ」と書いており、両者が併存する前提になっている。

ADR-161 D1 は「宣言を強制し、そこから生成する」と決めた。本 ADR はその考えを、**生成までは行かず、まず宣言テーブル化まで**に絞って
architecture_guard の count 固定型に適用する。ADR-162/`complexity-budget.md` の「同種 1 件の削除を伴わない追加は許さない」精神に合わせ、
新しい仕組みの追加は 5,914 行の純減で正当化できる場合に限る。

## 決定

### D1: 宣言テーブルを 1 つ、チェッカーを 1 本にする

`crates/awase-windows/tests/guard_rules.rs`(新規、テスト専用モジュール)に、パーサの要らない Rust の const 配列で置く。

```rust
struct GuardRule {
    id: &'static str,            // テスト失敗時に表示する安定 ID
    needle: &'static str,
    counting: Counting,          // RealCalls | ProductionText | NonCommentProduction
    sites: &'static [(&'static str, usize)],   // 書かれていないファイルは 0 件を期待しない(下の D2)
    scope: Scope,                // この crate の src/ 全体か、列挙ファイルのみか
    adr: &'static str,           // "ADR-129" 等。エラー文に必ず出す
    why: &'static str,           // 1 行。新しい呼び出し元を足す前に何を確認するか
}
```

チェッカー `check_rule(&GuardRule)` は 1 本で、`#[test] fn guard_rules_hold()` が全ルールを回し、**失敗は全ルール分をまとめて**報告する(現状は最初の失敗で止まる)。

### D2: `Scope::WholeSrc` を既定にする(=列挙漏れを検出できるようにする)

現状の多くのテストは「列挙したファイルだけ」を見る。列挙に無いファイルへ新しい呼び出し元が増えても気づけない。
`Scope::WholeSrc` では `list_src_files()` で全ファイルを走査し、`sites` に無いファイルは 0 件を期待する。
これは挙動の強化であり、移行時に既存の暗黙の穴が見つかる可能性がある。見つかった分は(a)表に足す(実在の呼び出し元)か(b)ADR/BUG に記録する。
**この強化は移行 PR と分け、移行 PR は同じ判定結果を保つ(`Scope::Listed` を使う)**。

### D3: 数え方は 3 種に限り、コメントの扱いを表に明示する

`Counting` は `RealCalls`(関数定義行を除いた呼び出し)、`ProductionText`(テストモジュールを除く、コメント込み)、`NonCommentProduction`(コメント行を除く)の 3 種。
新規ルールの既定は `NonCommentProduction`。既存の `ProductionText` を使うルールは、移行時に **コメント由来の件数が混ざっているエントリ**を洗い出し、
コメント行を除いた件数に直せるものは直す(`ime_controller.rs` / `open_chain.rs` の `InputRelay` はこの対象)。直す場合は件数が変わるので別コミットにする。

### D4: 対象外

次は表にしない。

- `extract_fn_body` / `extract_all_balanced_blocks` / `find_balanced_close` を使う、関数本体の構造検査(67 か所の呼び出し)。
- 「この文字列が存在しない」だけの否定ガード(`!production.contains("explicit_ime_action_case3_off")` 型)。ただし `needle` と `sites: &[]` の表で書き直せるものは、パイロット後に別途検討する。
- dylint の `RESTRICTED_CALLS`。dylint クレートはナイトリー固有の `rustc_private` に依存し、テスト用の const を共有できない。**二重管理の解消は本 ADR では約束しない**(検討した代替案を参照)。

### D5: パイロットは 5 ルール、中止基準を先に決める

最初に表へ移すのは、行 211(`thumb_down_timestamps`)、532、637、1209、4158(`InputRelay`)の 5 テスト。
次の**どれか**に当たれば、残りは移さず、本 ADR を「取り下げ」にして、パイロットで入れた `guard_rules.rs` を削除する。

- 5 ルールの移行後、`architecture_guard.rs` + `guard_rules.rs` の合計行数が移行前より減らない(コメントと ADR 参照の移設を含めて数える)。
- 移行前後でテストの合否が、現在の `origin/develop` に対して 1 件でも変わる(D2/D3 の強化は除く。強化は別 PR)。
- 5 本のうち 2 本以上で、`GuardRule` の項目では表せない例外(ファイル別に数え方が違う等)が出る。

## 期待する効果と、まだ測っていない点

- 見込み: 17 テストが 1 テスト + 17〜20 エントリ(各 6〜10 行)になる。現在の 17 テストは 1 本あたり約 25〜60 行(コメント含む)なので、**数百行の純減**を見込む。ただしこれは見積もりで、実測していない。D5 の中止基準で検証する。
- 失敗が 1 つずつではなく一括で出る。
- `id` と `adr` で失敗箇所の検索がしやすくなる。
- 数え方の違い(コメントを数えるか)がテストごとの暗黙ではなく表の項目になる。

## 検討した代替案

- **何もしない**: 5,914 行は読みにくいが、動いていて、失敗時のメッセージも十分に具体的。追加コストは小さい。
  → 却下はしない。D5 の中止基準を満たせなければ、これが結論になる。
- **TOML など外部ファイルに置いてパースする**: 表を非 Rust の人も触れるが、パーサ・スキーマ検証・エラー位置の表示が必要になり、5,914 行を減らす目的に対して仕組みが増える。Rust の const 配列で足りる。
- **dylint の `RESTRICTED_CALLS` に一本化する**: HIR で見られて正規表現より正確だが、`set_ime_open` のように単純な呼び出し元制限以外(識別子の出現数、`#[cfg(test)]` 除外、コメント扱い)は表せない。CI の dylint は `--tests` を含めない既存規約もある。ADR-161 が検討済みで、本 ADR では蒸し返さない。
- **`macro_rules!` で `guard!(...)` を作る**: const 配列で足りるので不要。マクロはエラー位置が悪くなる。

## 影響

- テスト専用の変更で、本番コードに影響しない。
- Windows 実機は不要(source-scanning テストでホストの Linux で動く)。
- `.claude/rules/fix-requires-evidence.md` の「再発ファミリー」表で `architecture_guard.rs` を指している箇所は、移行が済んだら `guard_rules.rs` も併記する。
