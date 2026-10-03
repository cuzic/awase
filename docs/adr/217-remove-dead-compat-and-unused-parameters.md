---
id: ADR-217
title: |-
  後方互換の名目だけが残るコード・未使用引数・古くなった dead_code allow を撤去する
summary: |-
  「後方互換のため」「将来のため」と書かれて残っているが、実際には使い手のいないコードを、コードを読んで棚卸しした(2026-10-02、develop `a612a832`)。
  使い手が無いことを確認できたもの(re-export・未使用引数・呼び出し元ゼロの関数)は撤去または私的 `use` へ格下げする。
  本番で使われているのに `#[allow(dead_code)]` とコメントだけが古くなっているものは、allow とコメントを直す(コードは消さない)。
  不具合報告 JSON の常に None のフィールドは、SCHEMA_VERSION を上げずに削除する。
  `GjiAction::SendInput` 系・`compute_active`・config.rs の旧キー受理は残す。`Tab::AppRules` は、所有者の判断(案内していない機能で使用者がいない)で設定画面だけを削除する(config.toml の項目は残す)。
  IME actuation 入口の診断専用コード・常に None の引数は ADR-216 の範囲なので、本 ADR では扱わない。
status: |-
  提案(2026-10-02)。Opus 敵対的レビュー round1 の指摘(`compute_active` を A から外す、`pub use` は私的 `use` に格下げ、C の各項目に判定)と round2 の事実誤り修正を反映済み。round2 で収束(設計上の論点なし、round3 不要)。A・B・C1・C3 を実装済み(2026-10-02、`refactor/adr217-remove-dead-compat` の6コミット + status 更新、PR 経由でマージ待ち)。C2 はコメント訂正のみで実施せず。C3 は所有者の判断で画面を削除した。
related_adr:
  - "ADR-148"
  - "ADR-158"
  - "ADR-163"
  - "ADR-191"
  - "ADR-201"
  - "ADR-206"
  - "ADR-208"
  - "ADR-215"
  - "ADR-216"
---

# ADR-217: 後方互換の名目だけが残るコード・未使用引数・古くなった dead_code allow を撤去する

## 背景

「後方互換のため」「将来のために残す」というコメントつきで残っているコードは、書いた時点では意味があっても、使い手が消えた後も残りやすい。
残っていると、(1) 読む人が「どこかで使われているはず」と考えて調べる、(2) ドキュメントが実在しない re-export を主張する、(3) `#[allow(dead_code)]` が本当に死んだコードを隠す、という害がある。
ADR-158 の方針(複雑性は減らす方向に報酬がない)に沿い、使い手のないものだけを撤去する。

### 棚卸しの方法と、初回の誤りの記録

初回の棚卸し(2026-10-02)には2種類の誤りがあり、Opus レビュー round1 で見つかった。以降の確認はこの教訓に従う。

1. **関数の呼び出し元を「外から `x.f(` と呼ぶ形」だけで数え、`self.f(` を落とした**。`compute_active` を「本番の呼び出し元ゼロ」と誤認した(実際は `engine.rs` 内に13箇所)。
2. **`pub use` を「パスで参照する外部の利用者」だけで数え、定義元モジュール自身が使っている分を落とした**。`yab::SpecialKey`・`vk::vk_to_pos` は、消すとモジュール内でビルドが壊れる。

使い手の確認は「`git grep -nw <名前>` の全件から定義行を引いて残りを分類する」で行い、最終的な証明は grep でなく**両ターゲットでのビルド**(下記「検証」)とする。
また初回は「`literal_session_confirmed`・`drives_composition_side_effects` を削除候補」としていたが、本番の呼び出し元があった(B 参照)。

## 確認した事実(2026-10-02、develop `a612a832`)

### A. 使い手が無い、または名前を解決するだけのもの

| 場所 | 内容 | 扱い |
|---|---|---|
| `src/yab/mod.rs:13` | `pub use crate::types::SpecialKey;`(「backward compatibility」)。外部パス `yab::SpecialKey` 経由は `src/engine/tests.rs:3191,3823` の2箇所のみ。ただし `yab/mod.rs` 自身が `:29` と `:190-` の match でこの `pub use` を通して名前を解決している | **私的な `use crate::types::SpecialKey;` に格下げ**し、`engine/tests.rs` の2箇所を `crate::types::SpecialKey` に直す |
| `crates/awase-windows/src/vk.rs:757` | `pub use awase_vkmap::vk_to_pos;`(「既存呼び出し元との互換性」)。`vk::vk_to_pos` 経由は0件だが、`vk.rs:535` の `to_pos` が解決に使う(`vk` は ungated) | **私的な `use awase_vkmap::vk_to_pos;` に格下げ**。doc の「互換性のためre-export」を直す。`awaza`(別リポジトリ)は `awase-vkmap` を直接使う設計で `awase-windows` に依存しないため、互換の読み手はいない |
| `crates/awase-windows/src/focus/classifier.rs:6` | doc が「`runtime` は `pub use crate::focus::classifier::*` で後方互換を維持」と主張。その re-export は存在せず、全利用者が `crate::focus::classifier::…` を直接使う | doc の1行を削除 |
| `crates/awase-windows/src/tsf/mod.rs:54-55` | `#[cfg(windows)] pub use awase::gate::GateAction;`。`tsf::GateAction` 経由の利用者は0件(`tsf_gate.rs:55` は `timed_fsm::GateAction` を直接 import) | 削除(Windows ターゲットの check で確認) |
| `focus/uia.rs:174` | `uia_classify_focus(automation, _hwnd)` の `_hwnd`。本体で未使用。呼び出し元(267行)の `hwnd` はその後もログと LPARAM で使われるので、呼び出し元の変数は残る | 引数を削除 |
| `runtime/key_pipeline.rs:301` | `kp_stage_focus_probe(&mut self, _event: &mut RawKeyEvent)` の `_event`。呼び出し元は86行の1箇所 | 引数を削除(ADR-216 が同関数の `gji_last_io_ms`〈310-311行〉を「消さない」と決めているため、そこには触れない) |
| `runtime/mod.rs:506` | `focus_epoch()`(`#[allow(dead_code)]`、「API として意図的に残す」)。`self.focus_fence().epoch` の薄いラッパーで呼び出し元ゼロ | 削除(`focus_epoch()` 自身の doc〈503-504行〉にある `focus_hwnd()` との対称の記述も一緒に消える。`focus_hwnd()` の doc は `focus_epoch` に触れていないので直さない。歴史記録の `docs/adr/106-*.md:529` の言及は書き換えない) |
| `state/platform_state.rs:1593` | `last_intent_source()`(`#[allow(dead_code)]`、「診断用アクセサとして残す」)。呼び出し元ゼロ。消しても `RecordedIntent::source` は他で読まれ続け、連鎖する dead_code は出ない | 削除 |

### B. 本番で使われているのに、古くなった allow・コメントが残っているもの

コードは消さない。allow とコメントを実態に直す。

| 場所 | 実態 | 扱い |
|---|---|---|
| `tsf/observer.rs:598` `literal_session_confirmed` | コメントは「非テストの呼び出し元がない」だが、`tsf/probe.rs:665`(`evidence_now`)が本番から呼ぶ。`observer` は `#[cfg(windows)]` なので Linux には存在しない。なお、できあがる値を読むのはテストだけで診断専用(`evidence_now` の doc〈630行〉も「journal 記録用」)。値の撤去は ADR-216 と同種の別件 | **allow を単純に削除**し、古いコメントを直す(`cfg_attr` は不要) |
| `state/ime_model.rs:48` `drives_composition_side_effects` | 呼び出し元は `runtime/mod.rs:947` の1箇所。`ime_model` は ungated、呼び出し元は windows のみなので、Linux ホストビルドでは dead_code 警告が出る | `#[cfg_attr(not(windows), allow(dead_code))]` に置き換える(`state/mod.rs` に13箇所、crate 全体で36箇所ある既存の書き方) |
| `state/mod.rs:71` `explicit_press`(モジュール単位の allow) | コメントは「L0 では `select_shadow_intent` 等だけを呼ぶ、それ以外は L1 以降」だが、ADR-208 L3a/L3b がマージ済みで、本番は `select_shadow_intent`(`key_pipeline.rs:967`)・`shadow_noop_write_target`(`:1202`)・`ShadowIntentKind`(`:21`)・`engine_set_open_filtered_by_chord`(`platform_state.rs:633`)を使う | `#[cfg_attr(not(windows), allow(dead_code))]` に変え、コメントを「ungated にして全列挙テストを Linux で回すため。本番の呼び出し元の一部は Windows 専用」に直す。Windows の clippy `-D warnings` で警告が出る項目があれば、その項目だけに局所 allow を付ける(モジュール全体には戻さない) |
| `state/mod.rs:100-104` `ime_actuation_decision`(モジュール単位の allow) | コメントは「まだどこからも呼ばれない、配線は別タスク」だが、既に本番配線済み(`ime_controller.rs:43,206,462,522`、`journal.rs:728`、ADR-163 TH1b-2b) | 同上(`cfg_attr` に変え、コメントを配線済みの状態に直す) |
| `state/mod.rs:79-84` `ime_profile_driver` | コメントが「配線は Phase 1 のスコープ」だが、ADR-090 §2.F で Phase 1d/1e は凍結済み。モジュール自体は ADR-090 §4.7 が契約宣言として意図的に残したもの | **コメントだけ**「契約宣言とテストのみのモジュール」に直す。撤去するかは別 ADR |

### C. 判断を付けたもの

| 項目 | 判定 | 理由 |
|---|---|---|
| **C1** 不具合報告 JSON の常に `None` のフィールド(`bug_report.rs` の `*_adopted_*`・`thumb_key_ime_warning`、`message_handlers.rs` の `GjiAdoptedFields`・`MsImeAdoptedFields` の `adopted_*_delegate`) | **今回やる(最後の独立コミット)** | 読み手は調べ尽くした。サーバ(`services/report-worker/src/index.ts:86`)は中身を検証せず不透明に扱う。Rust 側に `deny_unknown_fields` は無く、`Option` の欠落は `None` になるので、exe の版が食い違っても両方向で壊れない。分析スキル・テスト fixture にも言及は無い。ADR-191 以前の報告では値があり、以後は `null` なので、キーごと消えた方が「その版には機構が無い」と明確に伝わる。**`SCHEMA_VERSION`(`bug_report.rs:36`、現在3)は上げない**——`index.ts:519` は `schema_version !== SCHEMA_VERSION` を拒否するため、上げると Worker を再デプロイするまで新クライアントの報告が全て弾かれる。範囲: `bug_report.rs` の doc とフィールド・fixture(`:1157-1175`)、`message_handlers.rs:1442-1462`・`:1549-1630`(`MsImeAdoptedFields` は `adopted_ime_toggle_combos` だけになるので `Option<Vec<String>>` 1つにする)。ADR-148 に「ADR-217 でフィールド削除」と1行追記する |
| **C2** `GjiAction::SendInput`/`SendInputDirect`/`PendingInput`(`tsf/gji_fsm.rs`) | **やらない(コメントの訂正のみ)** | `PendingInput` は `SendInput` の中身であるだけでなく、`GjiEvent::KeyInput` の payload(`gji_fsm.rs:238`、生成は `vk_send.rs:210,388`)であり `OnCold`/`Warming` の pending バッファ(`:177,192,409,433,489`)でもある。`DiscardPending { count }` の件数もここから出る。`SendInput` 系だけを消すとテストの観測点を失うだけでバッファは残り、全部消すなら FSM の状態の形(バッファ→カウンタ)の変更になる。それは warmup/cold-start 系の再発ファミリーに入り、得るものは String の確保1つ分。`gji_fsm.rs:78-79` の doc を「`KeyInput` の payload と `DiscardPending.count` の元。romaji の中身はテストだけが見る」に直す程度 |
| **C3** `awase-settings` の `Tab::AppRules`(アプリ別上書き force_text/force_bypass/force_vk/force_tsf と `post_bypass` の設定画面。2026-08-26 の `1a3dcf5c` から非表示) | **画面だけ削除する(所有者判断、2026-10-02)** | 互換コードではなく製品判断だった。所有者の判断: 使い方を案内していない機能なので使用者はいない。**config.toml の項目(`app_overrides.force_*`・`post_bypass`)は残す**(既存ユーザーの設定ファイルとの互換、および config 側で動作は生きているため。画面の削除は挙動を変えない)。削除範囲: `Tab::AppRules` と `tab_app_rules`・`override_list_ui`・専用バッファ(`new_override_bufs`・`new_pb_*`)・テストの呼び出し。将来 UI を作り直すなら、自動判定できない「素通し」(`force_bypass`/`disable_apps`)だけを、フォーカス中アプリの登録ボタンつきで出す案を出発点にする(`force_vk`/`force_tsf`/`force_text` は自動判定の不具合として直すべきもので、UI で隠さない) |
| **C4** `explicit_press` の allow | **今回やる** | B に統合した |

## 決定

1. **A を実施する**。`pub use` は削除でなく私的な `use` への格下げ(定義元モジュール自身が使うため)。
2. **B を実施する**。allow とコメントを実態に直し、コードは消さない。Linux の扱いは各項目に書いたとおり(`cfg_attr` が要るのは ungated 定義で呼び出し元が windows のもの)。
3. **C1 を実施する**(`SCHEMA_VERSION` は据え置く)。**C2 はコメント訂正のみ**。C3 は画面だけ削除する(config.toml の項目は残す)。
4. コミットは1コミット1種類、すべて `refactor` 型(挙動不変、fix ではないので fix-requires-evidence の対象外と本文に1行書く。`.githooks/pre-push` の警告は出うるがブロックはしない):
   (i) re-export の格下げ・削除、(ii) 未使用引数、(iii) 呼び出し元ゼロの関数、(iv-a) 関数単位の allow とコメントの訂正(`literal_session_confirmed`・`drives_composition_side_effects`・`ime_profile_driver` のコメント)、(iv-b) モジュール単位の allow の `cfg_attr` 化(`explicit_press`・`ime_actuation_decision`。Windows clippy で局所 allow が増えたら理由を本文に書く)、(v) C1(不具合報告 JSON)、(vi) C3(設定画面 `Tab::AppRules` の削除)。
   (v) を最後にするのは、外部の読み手がいる唯一の項目なので、単独で revert できるようにするため。
5. **ADR-216 のマージ後の `develop` から切り直して実装する**。**衝突はない**: ADR-216 の実装ブランチ(`5363c2b1`、R1〜R4 実装済み)の diff は、ADR-217 の対象行と1つも重ならない(`key_pipeline.rs` は1268行付近のみ、`ime_model.rs` は56行以降、`runtime/mod.rs` は963行以降、`state/mod.rs` には触れない)。どの hunk も ADR-217 の対象行より後ろにあり、マージ後も行番号はずれない。したがって順序の制約は無く、ADR-216 のマージが遅れるなら先に進めてよい。ADR-216 が先にマージされる見込みなので、マージ後の `develop` から切るのを既定とする。
6. 新しい型・gate・抽象は足さない(`#[expect(dead_code)]` への置き換えも採らない: Linux/Windows と `cfg(test)` の組み合わせで「満たされなかった」警告が片方のターゲットだけに出て、`cfg_attr` の組み合わせが増える)。

## 決定しないこと(再提案を防ぐ)

- **`compute_active`(`src/engine/engine.rs:285`)**: 残す。`engine.rs` 内に本番の呼び出し元が13箇所、`engine/tests.rs` に約45箇所、`crates/awase-windows/tests/support/harness.rs:216,609` からも呼ばれ、`architecture_guard.rs:823` が `thumb_open_role_action` の本体に文字列 `self.compute_active(ctx)` があることを ADR-206 決定3のゲートとして検査している。`compute_state(ctx).is_active()` の糖衣として正当に使われているので、doc の「(後方互換 API)」を「短縮形」に直すだけにする。置き換えると約60箇所の機械置換とガード書き換えが要り、消える行は3行で、減らす方向に反する。
- **config.rs の旧キー・旧値の受理は全て残す**。リポジトリの外にある読み手(既存ユーザーの `config.toml`)を守っており、本 ADR の判定基準(リポジトリ内の grep で使い手ゼロ)が当てはまらない。
  - `src/config.rs:589` `#[serde(alias = "engine_off_solo_triple")]`: 消すと旧名で書いたユーザーの**緊急脱出キーの設定値が失われ、既定の `VK_INSERT` に戻る**(ADR-201 の未知キー警告〈`src/config.rs:889`、`load_warnings`〉は出るが、警告を読まないユーザーは緊急時に初めて気づく。エンジンが壊れたときの逃げ道なので、壊れたときの被害が最も大きい種類の設定)。
  - `:74-89` `ConfirmMode` の `speculative`/`two_phase`/`adaptive_timing` → `Wait`: v1 から v2 に移るユーザーが踏む経路で、消すと `unknown_variant` で `AppConfig::load` が Err を返し、**awase が起動しない**(`app/mod.rs:204`)。`origin/v1-develop:src/config.rs:114-118` に旧値が実在する。
  - `:820-824,947-950` `legacy_keymap`(`[[keymap]]` 旧表記、ADR-201 決定5)。
  - `:1797-1805` 撤去済みの `output_mode`/`hook_mode` の許容。
- **`hook.rs:205-210` の `resolve_thumb_key` の re-export と `observer/focus_observer.rs:9` の `detect_app_kind` の re-export**: 互換の名目だが現に使われている(前者6件、後者2件)。呼び出し元の書き換えは churn でしかない。
- **`tsf/observer.rs:715` の `WinEventHookGuard` の re-export**: `install_observation_hooks` の戻り値型なので、外すと名前を付けられない型になる。`journal.rs:15` の `LaneKind` の re-export は価値が低く本 ADR では触らない。
- **`explicit_press.rs:743` の `SHADOW_NOOP_WRITES_IN_TSF_NATIVE: bool = false`**: ADR-208 L3' の実機 A/B を解禁条件とする意図的な足場で、ADR-208 の管轄。
- ADR-216 の範囲(`dispatch_ime_set_open`・`apply_ime_open_with_view`/`_with_belief` 周辺の診断専用計算・常に `None` の引数)。
- **`literal_session_confirmed` の値そのもの(`LiteralEvidence` の診断専用フィールド)の撤去**: BUG-75 の恒久対策案の判断材料として journal に残しているもの(`probe.rs:628-630`)なので、BUG-75 の決着とセットで判断する。
- トレイト実装・cfg スタブの `_` 引数(非トレイト関数の `_` 引数を全数確認し、本 ADR の2件以外は全てトレイト実装か cfg スタブだった)と、`output/sender.rs:67`(RAII ガード)・`measured-macro/src/lib.rs:48`(記録用メタデータ)の allow。

## 検証

最終的な証明は両ターゲットでのビルド・lint とする(grep では re-export の格下げが「0件」にならない)。**合否は CI と同じコマンドで判定する**。

```
cargo clippy --target x86_64-pc-windows-msvc -p awase-windows -- -A clippy::cargo_common_metadata -D warnings -W clippy::cognitive_complexity   # CI windows-build(ci.yml:277)と同じ
cargo clippy --lib -- -D warnings                  # CI の Linux clippy ジョブ(ci.yml:123)と同じ。(i) はルートクレートの src/yab・src/engine/tests.rs に触れる
cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib
cargo test --lib
cargo nextest run -p awase-windows --test architecture_guard --test layer_boundary_guard --test golden_scenarios --test explicit_press_exhaustive
cargo test -p awase-windows --lib bug_report      # C1(fixture の更新)
```

- `clippy --tests` は develop の基準と比べる参考扱いにする(既存テストコードの無関係な違反で赤になりうる)。
- C1 の `message_handlers.rs` は `runtime` 配下(windows)で、Linux ではテストできない。Windows の check/clippy でしか検証できない。
- C1 の回帰の釘: 既存テスト `bug_report.rs:1486`(「旧形式のJSONを読めること」)の旧形式 fixture に、削除するキー(`henkan_adopted_kind` 等)を残す。誰かが `deny_unknown_fields` を足して R2 の過去の報告が読めなくなる事態を CI が捕まえる(既存 fixture に数行足すだけで、新しい仕組みではない)。
- 撤去・格下げする名前ごとに、`git grep -nw <名前>` の全件から定義行を引いた残りが、意図どおり(0件、または定義元モジュール内のみ)であることを確認する。
- ガードテストが撤去する名前を文字列で参照していないかは、round1 で `compute_active` 以外に無いことを確認済み(`compute_active` は残す)。

## 未確定・リスク

- B の `cfg_attr` 化で Windows の clippy に新しい警告が出る項目があれば、局所 allow に倒す(モジュール全体には戻さない)。実装時に確認する。
- ADR-216 は実装済み・マージ待ち(`5363c2b1`)。diff が重ならないことは確認済みで、順序は任意。

## 参考

- [ADR-216](216-remove-diagnostic-only-open-belief-and-unread-apply-arguments.md)(IME actuation 入口の診断専用コードの撤去)
- [ADR-191](191-calibration-experiments.md)(機構撤去の経緯)
- [ADR-148](148-bug-report-ime-keymap-attachment.md)(不具合報告のスキーマ)
- `.claude/rules/complexity-budget.md`
