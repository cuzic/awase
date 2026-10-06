---
id: ADR-229-companion-229-opus-fcis-round3
title: |-
  ADR-229 FCIS 設計 opus-adversarial-consult round3(差分確認、収束)
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-224"
---

# FCIS の ADR 改訂文とタスク表の確認(round3、差分確認)

- 対象: local develop の `585ade35`(`git show HEAD`)。
  - `docs/adr/229-…md` の「FCIS による改訂」節、D2 の注記、summary
  - `docs/tasks/fcis-layering-tasks-2026-10-06.md`
- 照合先は `origin/develop` = `d0d42be6` のコード。読み取りのみ。

## 0. 結論

**収束した。** ただし、タスク表の P4 に 1 行だけ誤った指示があるので、それを直すことを条件とする(**M8**。直した後の再確認は不要)。

M5・M6・M7、S6〜S9、N3・N4 は、意図どおりに反映されている。それ以外は Should 4・Nit 4 で、修正は提案側でよい。

---

## 1. round2 の指摘の反映状況

| 指摘 | 状況 | 確認した点 |
|---|---|---|
| M5(殻 13 個、`RECORDERS`) | 済(数え方の書き方に Should) | 内訳 10 + 3、子モジュール、runtime の差分 0、`:1620` を同じ PR で、が書けている。ただし数え方の具体が S10 |
| M6(`for_test`、件数、P3/P4 の分担) | 済(件数に Nit) | P3 で付け替え(gated のまま windows-build で確認)、P4 で gate 解除とテストの書き換え、時間軸の切り分けが取りやめ条件に入っている。`cfg` で壁時計を切り替える案を退けたことも明記されている |
| M7(F-D5) | 済 | F-D5-1 を「同期の処理」に限定し、F-D5-4(非同期の handler は各 `.await` の後に観測し直す)を、根拠(INV-45、`fallback_write`、ADR-180 決定1)つきで足している。正確 |
| S6(違反 0 のファイルだけ、45) | 済(規則の細部に Should 2) | 付録 A の 45 件は round2 の実測と一致。付録 B の 8 件も一致 |
| S7(閉じた列挙、条件①〜③) | 済 | ③「追加には本 ADR の改訂が要る」と、その理由(テキスト走査では trait 呼び出しを検出できない)まで書けている |
| S8 | 済 | `romaji_pre_write` は chain の前処理、focus は MSAA までの同期の段階、UIA は A 種。`ImeProbe` は「実装する PR でリストに追加するときに限る」として、架空の例のまま例外を開かない書き方になっている。良い |
| S9(子モジュール) | 済 | `#[cfg(windows)] mod shell;`、`state/platform_state/shell.rs` |
| N3 | 済 | 付録 B で 3 つになっている |
| N4 | 済 | 「ungate 自体では減らない、P5 で 1 系統ずつ」 |

---

## 2. 新しい指摘

### M8(Must): P4 で `ime_decision_view` の gate まで外す指示になっている

- タスク表 P4 の内容欄は「`state/mod.rs:186-188` の gate(`platform_state`・`ime_decision_view`・`ime_event_log` の組)を外す」「`ime_decision_view.rs` の `ActiveImeKind`・借用の扱いは、gate を外す前に必要な型の移動を確認する」となっている。
- しかし `state/ime_decision_view.rs` には、P4 では外せない依存がある。
  - `use crate::tsf::observer::TsfObservations`(`:17`)
  - gated な `crate::tsf::observer::ActiveImeKind`(`:52`, `:68`)
  - 構築時にグローバル `crate::tsf::observer::candidate_was_seen()` を読む(`:87`)
- しかも `platform_state.rs` は `ime_decision_view` を一切使っていない(`ImeControlView`・`FocusFacts`・`ObservedState` の参照は 0 件)。外す必要が無いうえに、外そうとすると P4 が `tsf::observer` の分解(S-D、F-D2 で「借用ビューの所有化の対象」とした作業)まで巻き込む。
- `ime_event_log` は #492 で既に gate を外している。
- **直し方**: P4 は「`state/mod.rs` の `#[cfg(windows)] pub mod platform_state;` と `#[cfg(windows)] pub use platform_state::PlatformState;`(develop では `:187-190`)の gate を外す。`ime_decision_view` は gated のまま(S-D の対象)」に直す。「`ime_decision_view.rs` の…確認する」の一文は消す。

### S10(Should): `RECORDERS` の「`_in_scope` 版も数える」をそのまま実装すると、意味が変わる

- P2 の後の本番コードには、次の 3 種類の呼び出しがある。
  - runtime/ の 4 か所の `.record_confirmed(`(殻を呼ぶ)
  - core の中の `record_ime_apply_result` → `.record_confirmed_in_scope(`(1)
  - **殻の本体の `self.record_confirmed_in_scope(`(1)**
- `list_src_files()` が子モジュールの `shell.rs` も走査すると、単純な合計は 6 になる。そこで期待値を 6 に書き換えると、「新しい記録の呼び出し元が 1 つ増えた」ことと区別できなくなる。
- **直し方**: P2 の内容欄に、「`state/platform_state/shell.rs` を `RECORDERS` の走査から除外し(殻は委譲するだけ)、`.record_confirmed(` と `.record_confirmed_in_scope(` の合計を 5、`.record_optimistic(` と `.record_optimistic_in_scope(` の合計を 1 に保つ」と書く。同じ注意は、`:3767-3775` の呼び出し元の固定にも当てはまる。殻の本体は `_in_scope` という別名を呼ぶので一致はしないが、念のため「殻のファイルはガードの走査対象外にする」方針を 1 行書いておくと、後続の殻で迷わない。

### S11(Should): 不変の static の扱いが、ADR とタスク表で食い違っている

- ADR の F-D6 と Tier-2 の②は「不変のディスパッチ表は可」としている。一方、付録 B は `ime_profile_driver.rs` を「違反のある 8 ファイル」に入れたまま、「許可に分類する(規則の対象外とするか、規則の書き方で除外)」と保留している。どちらなのかが決まっていない。
- テキスト走査で「可変か不変か」を判定するのは難しい(型に `Atomic`/`Mutex`/`Cell`/`OnceLock`/`LazyLock` があるかで見ても、newtype で包むと漏れる)。
- **直し方(推奨)**: P0 の規則②は「`CORE_MODULES` のファイルに `static` を置かない(`thread_local!` も)」と単純にする。不変の表は `const` にするか、そのファイルを `CORE_MODULES` に載せない。`ime_profile_driver.rs` の 3 つはゼロサイズの構造体なので、`const` による `&'static dyn` の昇格で置き換えられる見込み(**未確認**)。その上で、ADR の「不変のディスパッチ表は可」を「Tier-2 の外として扱う(`CORE_MODULES` には載せない)」に揃える。

### S12(Should): `state/mod.rs` は「`mod` 宣言の例外」だけでは違反 0 にならない

- 付録 B は `mod.rs` の 8 件を「規則の対象外(`mod` 宣言)」としている。だが実際には、8 件のうち 4 件は `use` の宣言。
  - `:32-35` `#[cfg(windows)] pub(crate) use conv_mode::{ConvModeMgr, …}`
  - `:189-190` `pub use platform_state::PlatformState`
  - `:194-195` `pub(crate) use ime_decision_view::{…}`
  - ほか
- 例外を「`mod` 宣言」だけにすると、`mod.rs` は違反のまま。逆に `use` まで例外にすると、core のファイルに gated な再公開を置く抜け道ができる。
- **直し方**: `mod.rs`(モジュール宣言の集約ファイル)は**恒久的に `CORE_MODULES` に載せない**と付録 B に書き、例外は「`mod` 宣言と `#[cfg(any(windows, test))]`」のままにする。S9 の殻の `#[cfg(windows)] mod shell;` は、`platform_state.rs` 側の `mod` 宣言なので例外で通る。

### S13(Should): P0 の 4 規則を実装するときの落とし穴(タスク表の P0 に注記を)

1. **テキストの切り方**: 「`#[cfg(test)] mod tests` より前、コメント行を除く」だけでは不十分。
   - (a) 行末のコメント(`foo(); // Instant::now()`)
   - (b) 文字列リテラル(ログの文言に `std::fs` などを書いた場合)
   - (c) `mod tests` 以外の名前のテストモジュール(#493 の `plan_shell_tests` の例。`architecture_guard.rs:4175-` が `strip_any_test_module` を別に用意した理由と同じ)
   - (d) `mod tests` より前にある `#[cfg(test)] impl`/`#[cfg(test)] fn`(例: `platform_state.rs:1610` の `#[cfg(test)] impl ImeStateHub`)

  → **`layer_boundary_guard.rs` の `code_lines`/`test_block_mask`(`:47-112`。コメントを除き、`#[cfg(test)]` が付いた item の本体を丸ごと覆う)を使う**のが最も確実。P0 のテストを `layer_boundary_guard.rs` 側に置くか、同じヘルパーを移す。
2. **`#[cfg(windows)]` の検出**: 完全一致だけだと、`#[cfg(all(windows, …))]`・`#[cfg(target_os = "windows")]`・`#[cfg_attr(windows, …)]` を見逃す。許可する形(`mod` 宣言の直前、`any(windows, test)`)を明示的に除いた上で、`cfg` の中に `windows` の語があれば違反、とする。
3. **壁時計**: `Instant::now()`/`SystemTime::now()` だけでなく、`quanta::Clock::new()`/`quanta::Instant::now()`(journal が使う)、`timed_fsm` の実時計も対象にする。`journal` を将来 `CORE_MODULES` に入れるなら、ここが効く。
4. **FS/環境変数**: `std::fs`・`fs::` に加えて、`File::open`、`Path::exists`/`metadata`(`std::path` 経由の FS 読み取り)。逆に `env!`/`option_env!`/`include_str!` はコンパイル時の展開なので許可する(誤検出しない)。

### S14(Should): 付録 A の、#492〜#495 マージ後の差分を書いておく

付録 A の見出しに「再実測してから確定する」とはあるが、何が変わるかが書かれていない。P0 の実装者が迷わないよう、予想を列挙しておく。

| ファイル | マージ後の予想 | 扱い |
|---|---|---|
| `state/physical_disposition.rs` | #493 で `#[cfg(any(windows, test))]` が 1 件入る | 例外なので違反 0 のまま、付録 A に残る |
| `state/ime_event_log.rs` | #492 で ungated になるが、`Instant::now()`(`:41`) | 付録 B 側(違反あり)。直すなら `record_at` を使う側へ |
| `focus/hwnd_cache.rs` | #494 で ungated、違反 0 の見込み | `state/` の外。P0 の初期対象を `state/` に限るなら載せない。載せるなら「`state/` 以外も対象にする」と書く |
| `tsf/tsf_gate.rs` | #492 で ungated。`Duration` だけで違反 0 の見込み | 同上 |
| `journal.rs` | #495 で ungated。`#[cfg(windows)]` の dump 関数 2 件と、`quanta::Clock` | 違反あり。記録係なので core に入れる必要は薄い |

---

## 3. 確認した範囲(指摘なし)

- **行番号と件数**(`d0d42be6`)
  - `architecture_guard.rs`: `:1382`(`HubClock::wall(crate::hook::current_tick_ms)`)、`:1620`(`RECORDERS`)、`:3767-3775`
  - `platform_state.rs`: テスト 48 本、`:2561` `test_foreground_scope`、殻の名前のメソッドのテストからの呼び出し 16(9/3/3/1)、`foreground_scope()` の直接呼び出し 9
  - 付録 B の各行番号
  - いずれも実コードと一致した。
- **件数の訂正(N5)**: P3 の「`PlatformState::new()` 37 か所」は、テストの中では **36**。round2 の私の数え方に、本番の `:1800`(`ime: ImeStateHub::new()`)が混ざっていた。「19 か所」は `effective_open()` の**呼び出し**の数で、構築の 36 か所とは別の集合。
- **ADR の F-D1〜F-D6・レシピ・所有者の判断**は、round1・round2 の結論と矛盾しない。D2 の見出しへの「置き換え」の注記と、summary の追記も適切。
- **CI で判定できるか**
  - P0: `architecture_guard`(または `layer_boundary_guard`)の Linux の `test` ジョブ。
  - P1〜P3: windows-build の lib テストと、Linux の `test`。
  - P4: Linux の `test` のテスト名の列挙と、`never used` が 0。
  - P5: `--test closed_loop_scenarios`。
  - RW: Linux の lib テスト。

  いずれも判定のジョブが特定でき、ローカルでのビルドは要らない。P4 の「時間軸の違いか本物のずれかの切り分け」は CI では自動判定できないが、取りやめ条件(止めて調べる)として書かれているので妥当。切り分けの手掛かりとして、「同じテストを `HubClock::manual` の基準 tick を変えて 2 通り流す」と添えると、実装者が迷わない(**N6**)。
- **順序の制約**(P1 は #495 の後、P2 → P3 は直列、RW は並行可、P0 は単独): §2 の表と §5 の両方に書けている。

## 4. Nit

- **N5**: P3 と §5-5 の「37 か所」→「36 か所」。「19 か所」が呼び出しの数であることを明記する。
- **N6**: P4 の取りやめ条件に、切り分けの手順(基準 tick を変えて 2 通り流す)を 1 行添える。
- **N7**: ADR の F-D6 本文にも「初期の 45 は d0d42be6 時点。#492〜#495 のマージ後に再実測」の一言を入れる(今はタスク表の付録 A にだけある)。
- **N8**: ADR の指標の「`allow(dead_code)` の数(46 か所/11 ファイル)」は、厳密には `cfg_attr(not(windows), allow(dead_code))` が 45 で、その他の `cfg_attr(not(windows), …)` が 1(round1 の私の実測)。指標として使うなら、数え方(grep のパターン)を書いておく。
