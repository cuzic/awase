# ADR-158〜162 実装タスクリスト（2026-09-09、round4レビュー反映後）

[ADR-158](158-complexity-reduction-north-star.md)「育て方」ロードマップと、
[ADR-161](161-single-source-spec-generation.md)「機構の選定指針」を、実行可能な単位に
分解したもの。各タスクは独立したPR/コミット単位を想定する。本ドキュメントはADRではなく
実装計画であり、[ADR-158](158-complexity-reduction-north-star.md)「育て方」節から
参照される。

**round2改訂**: opus-adversarial-consultによるタスクリスト単体レビュー（round2、Must-fix
8件・Should-fix 13件）を反映し、以下を変更した。

- タスクIDにT接頭辞を付与し、ADR-160のC1〜C3・ADR-161のD1〜D3・ADR-162のE1〜E4と
  文字面で衝突しないようにした（round2 S-1）。
- ADR-159段階0の主目的（`send_input_safe`/`send_ime_control`、20箇所の宣言）が
  欠落していたため`TB0`として追加した（round2 S13-2）。
- ADR-159段階1・段階2（ADR-162の全施策の着手条件）が欠落していたため`TF1`〜`TF2`として
  追加した（round2 S13-3、最優先級で配置）。
- ADR-160のC1〜C3判断材料収集、ADR-162のE1〜E4、ADR-161のD2（proptest）を、待機中である
  旨を明記した上でタスク化した（round2 S13-1・5・6）。
- 各ADR単体のopus-adversarial-consult実施を`TJ1`として追加した（round2 S13-7）。
- `TC1`（pre-push統一）を2段手順に修正、`TD`群のcfg(windows)問題・宣言置き場所・採番方針を
  修正、`TE1`（tuning）の実装先クレートと段階導入方針を修正、`TE2`（pending_deferred）の
  機構を再選定した。詳細は各タスクの本文参照。

**round3改訂**: round3レビュー（Must-fix 4件・Should-fix 8件）を反映。D3の対象候補を
エントリ01のみに絞り込み、`ime_key_for`等の非gated化でTD2がホスト実行可能と判明、
TE2に検証基準を追加、依存関係グラフとタスクID範囲の表記ゆれを修正した。

**round4改訂**: round4レビュー（Must-fix 3件・Should-fix 6件）を反映。round3 SF-6で
一度採用した「エントリ再採番」方針が既存の相互参照（ADR-100・138・153・index.md・
ADR-161）を破壊すると判明したため撤回し、新規番号（26）付与に戻した（TD4）。
`docs/adr/index.md`のADR-158行がADR-162への反映を「未反映」と誤記していた点を修正した。
依存グラフの`TB2 → TC3`という誤った辺を削除し、着手順推奨のTD2に関する記述を
TD0の非gated化前提と整合させた。

## 前提

- すべてのタスクは[worktree-per-session](../../.claude/rules/worktree-per-session.md)に従い、
  専用worktree＋専用ブランチで実施する。**逐次依存するタスク列（例: TA1→TA2→TA3）は、
  先行タスクが`develop`にマージされてから後続のworktreeを`develop`の新しい先端から切る**
  （[main-develop-branch-flow](../../.claude/rules/main-develop-branch-flow.md)に従い、
  作業ブランチは常に`develop`の先端から切る。先行タスクの未マージブランチから後続を
  分岐させない）（round2 S-11で追記）。
- タスクTA〜TJすべてが対象——「これは新機構を1つ増やす価値があるか、既存資産（テスト・
  可視性）で足りないか」（[ADR-161](161-single-source-spec-generation.md)判断の手順・
  問い4）を、TA〜TDだけでなく特にTE（proc-macroクレート新設・dylint追加本数・実行時記録
  マクロ）にも適用すること（round2 S-10で訂正、round3 SF-2で範囲をTA〜TJ全体に訂正——
  TI・TJは当初この一覧から漏れていた）。
- 各タスクの「検証方法」に実機ソークが必要なものは、`.claude/rules/fix-requires-evidence.md`の
  対象領域に触れる場合、テストまたはknown-bugs.md記録のいずれかを伴わせること。
- **[ADR-161](161-single-source-spec-generation.md)単体のopus-adversarial-consult
  （`TJ1`）を、`TA2`着手前に実施することを推奨する**（round2 S13-7）——D1の機構がround1で
  synからdylintへ組み替わった直後であり、round2でさらに判断基準に「存在/不在」が追加される
  など、まだ変動している。

---

## タスクグループTA: dylint 4本目のlintを本実装に格上げする（[ADR-158](158-complexity-reduction-north-star.md)第1段階）

### TA1: `PlatformRuntime::apply_ime_open`のADR-087 §5 Phase 3判断

**内容**: `src/platform.rs`（ルートクレート）の`PlatformRuntime::apply_ime_open`デフォルト
実装（`self.set_ime_open(open)`を呼ぶ、実質的に呼び出し元ゼロの死んだコード）について、
実配線するか削除するかを判断する。

**依存**: なし（TA2の前提）。

**検証方法**: 削除する場合は`cargo check --target x86_64-pc-windows-msvc -p awase -p
awase-windows`が通ること。実配線する場合は呼び出し元を追加した上で同様に確認し、
`docs/known-bugs.md`または新規ADRに判断理由を記録する。

### TA2: `actuation_call_guard_spike`を`lints/`配下の正式なlintに昇格

**内容**: `spike/syn-xtask-prototype`ブランチの`lints/actuation_call_guard_spike`を、
既存3本と同じ構成で`lints/`直下に正式追加する。許可リストは`set_ime_open`→
`set_ime_open_ordered`（TA1の判断が「削除」ならこれで確定、「実配線」なら呼び出し元
`apply_ime_open`自体を許可リストに追加する——**round2 S-6で訂正**: 追加すべきは
「`set_ime_open`を呼んでいる関数の名前」であって「`apply_ime_open`の呼び出し元」ではない。
取り違えるとlintが黙って素通りする）。

**依存**: TA1。

**round2訂正（S-2・S-3）**: CI組み込み自体は`.github/workflows/ci.yml:171-176`が
`cargo dylint --all -p awase-windows`を`DYLINT_RUSTFLAGS="-D warnings"`付きで既に実行して
おり、`--all`のため`Cargo.toml`の`[workspace.metadata.dylint].libraries`に1行足せば自動的に
対象になる——**「CI組み込み」は新規作業ではない**。実際に必要な作業は次の2つに絞られる:
1. `Cargo.toml`への1行追加。
2. `lints/actuation_call_guard_spike/ui/main.rs`には既に発火想定ケース3つ（トレイト
   default本体・`::`修飾呼び出し・ドット呼び出し）があるが、対応する`ui/main.stderr`
   （期待出力ファイル）が無い——これを`dylint_testing`のbless機構で生成し固定する。
3. `ci.yml:129`付近のlint名コメント列挙を更新する。

**検証方法**: 上記1〜3の後、`DYLINT_RUSTFLAGS="-D warnings" cargo dylint --all -p
awase-windows -- --target x86_64-pc-windows-msvc`が全4本のlintで警告ゼロで通ること、
`cargo test -p actuation_call_guard_spike`（UIテスト）が通ることを確認する。

### TA3: 型解決の要否を判断する

**内容**: **round2 S-5で追加**——現在のスパイク実装（`segment.ident.name`による名前一致のみ、
型解決なし）のままにするか、`DefId`ベースの型解決を追加するかを判断する。追加しない場合は
「無関係な型の同名メソッドにも誤って発火しうる」という制約をlintのdocコメントに明記する。

**依存**: TA2。

---

## タスクグループTB: 既存境界の宣言（[ADR-159](159-existing-io-boundary-inventory.md)段階0の主目的、[ADR-158](158-complexity-reduction-north-star.md)第1〜2段階）

### TB0（round2 S13-2で追加、優先度高）: `send_input_safe`/`send_ime_control`の宣言

**内容**: [ADR-159](159-existing-io-boundary-inventory.md)が段階0の「実際の成果物」と太字で
明記している、`send_input_safe`（20箇所・12ファイル）と`send_ime_control`（10箇所・2ファイル
——round4 TJ2 MF1で訂正、当初「20箇所」に含めていたが実際は別勘定）の呼び出し元を、dylintの
許可リストとして宣言する。**round4 TJ2 MF2で追記**: `send_ime_control`の宣言キーは関数名単体
ではなく`(関数名, cmd)`のペアとする（`imm.rs`の`SendMessageTimeoutW`は`cmd`引数で
actuation/probeを判別しており、関数名だけをキーにするとprobe専用の8箇所が混入する）。
dylintでこの粒度が表現できない場合は「actuationを起こす`cmd`のみ対象、probe専用の`cmd`は
対象外」という運用規約で代替する。TA2で確立した本実装パターンに従う追加のlintとなる
（round3 SF-3で訂正: lint本数は着手時点までに確定した本数に応じて変わるため、固定の
「n本目」という数字は本文に埋め込まない）。

**依存**: TA2（実装パターンの確立）。

**検証方法**: `xtask-spike`（synスキャン）の実測値と、dylintの許可リストの件数が一致することを
確認する。**round4 TJ2 MF1で訂正**: 一致確認は`send_input_safe`（20箇所）と`send_ime_control`
（10箇所、うちactuation対象は`cmd`ベースで2箇所）を分けて行う——合算した「20箇所」との照合は
成立しない。

### TB1: `apply_ime_open_with_view`/`_with_belief`の宣言

**内容**: 4箇所（`apply_ime_open_with_view`）・2箇所（`apply_ime_open_with_belief`）の
許可呼び出し元を宣言する。

**依存**: TA2。

### TB2: `fix-requires-evidence.md`とガード期待値1件の生成

**内容**: TB0・TB1で確定した宣言から、`.claude/rules/fix-requires-evidence.md`の「IME
actuation合流点」該当行、および`crates/awase-windows/tests/architecture_guard.rs`の
`.apply_ime_open_with_view(`に対応するガード期待値（1件）を生成するxtaskを実装する。

**依存**: TB0、TB1。

**検証方法**: 生成結果を既存の手書き記述と比較し、差分がない（または差分が正しい訂正である）
ことを確認する。生成後、宣言に呼び出し元を1件追加してxtaskを再実行し、
`fix-requires-evidence.md`該当行が自動更新されることを確認する。

**round2追記（S13-4）**: [ADR-161](161-single-source-spec-generation.md)のD1が挙げる
生成対象のうち、CLAUDE.md該当節（共有可変状態の一覧・dylint本数）の生成は**本タスクの
スコープ外**とする（[ADR-158](158-complexity-reduction-north-star.md)第2段階「最初から
広げない」方針に従う）。CLAUDE.mdのdylint本数の誤記（「2つ」、実際は3本）は生成を待たず
手で先に直してよい。

---

## タスクグループTC: pre-push regex統一（[ADR-158](158-complexity-reduction-north-star.md)第3段階）

### TC1: `.githooks/pre-push`と`.git/hooks/pre-push`の和集合マージ（round2 M-3で2段手順に修正）

**内容**: **round2で判明**——2ファイルの乖離は片方向ではなく**双方向**である。追跡下の
`.githooks/pre-push`のみが含む対象（`runtime/transport.rs`・`runtime/ime_refresh.rs`・
`src/engine/nicola_fsm.rs`・evidence側の`src/engine/tests.rs`）と、実行される
`.git/hooks/pre-push`のみが含む対象（`runtime/message_handlers.rs`・`runtime/outbox.rs`・
`input_defer.rs`）がある。**単純に`core.hooksPath`を切り替えるだけでは、後者3ファイルが
チェック対象から脱落する**（ADR-156が問題視した穴の再生産）。

まず`.githooks/pre-push`側に両ファイルの正規表現の**和集合**を反映し、`runtime/mod.rs`
（`reassert_explicit_physical_key`、`fix-requires-evidence.md`の合流点表に明記されている
5つ目の入口——round2で発見、どちらのファイルにも現状含まれていない）も含めて確認する。

**依存**: なし。

**検証方法**: マージ後の`.githooks/pre-push`の対象ファイル正規表現が、両ファイルの正規表現
それぞれに一致していたファイルすべてを（和集合として）カバーすることを、実際に
`git diff --name-only`相当のテストケースで確認する。

### TC2: `core.hooksPath`の切り替え

**内容**: TC1完了後、`git config core.hooksPath .githooks`へ切り替える（ユーザー承認の上で
実施）。

**依存**: TC1（**必須**。TC1を経ずに実施すると3ファイルが対象から脱落する、round2 M-3）。

**検証方法**: `git config core.hooksPath`が`.githooks`を指すことを確認し、`.githooks/pre-push`
を実際に編集してpushし、フックが発火することを確認する。

### TC3: 対象ファイル正規表現の生成への置き換え

**内容**: TB0・TB1の宣言（および今後追加される再発ファミリー対象ファイル一覧）から、
`.githooks/pre-push`の対象ファイル正規表現を生成する。

**依存**: TC2、TB0、TB1。

**検証方法**: 生成された正規表現が、TC1で確定した和集合をすべてカバーすることを確認する。

---

## タスクグループTD: D3（否定の宣言）の実装（[ADR-158](158-complexity-reduction-north-star.md)第4段階）

### TD0: `ime_key_for`等を非gatedモジュールへ切り出す（round3 MF-2で内容を確定）

**内容**: `state/key_sequence_policy.rs::ime_key_for`・`ImeOperation`・`KeyMechanism`の
依存を確認したところ、`#[cfg(windows)]`を要求する依存（`tsf::observer::ActiveImeKind`）は
`gji_direct_applicable`/`ms_ime_direct_applicable`という別の述語関数側だけが使っており、
`ime_key_for`自体は`focus::class_names::AppImeProfile`・`crate::vk`・`awase::types::VkCode`
（いずれも非gated）にしか依存していない。**この3つ（`ime_key_for`・`ImeOperation`・
`KeyMechanism`）を非gatedなモジュールへ切り出す**（または`key_sequence_policy.rs`を
gatedな述語部分とungatedな純粋関数部分に分割する）。これによりTD2のテストがホストで
完全に実行できるようになる。

`REJECTED_IME_OFF_KEYS`の宣言自体は、切り出し先と同じ非gatedモジュールに素の`const`配列
として置く（実証実験4の関数形式マクロによるDSL化は後回し——round2 S-8: DSL化すると
proc-macroクレートが1つ増え、TD2が謳う「proc-macro不要」という利点が崩れる）。

**依存**: なし（TD1の前提）。

**検証方法**: 切り出し後、`cargo check --target x86_64-pc-windows-msvc -p awase -p
awase-windows`が通ること、既存の呼び出し元（`gji_direct_applicable`等）が引き続き
コンパイルできることを確認する。

### TD1: `REJECTED_IME_OFF_KEYS`宣言の作成

**内容**: `docs/experiments.md`の25エントリのうち、キー選択に関わるものを対象にする。
**round2 M-2・round3 MF-1で候補を段階的に訂正**: エントリ07・08・09は注入経路側の失敗で
D3の対象外、エントリ16は撤回ではなく採用済みのため除外、**エントリ05も同じ注入経路側の
失敗（`345086b`でのモードキー注入、CapsLock汚染で撤回）と判明し対象外**——**実質的な候補は
エントリ01の1件のみ**。フィールドは`vk`/`scope`/`reason`/`evidence`/`ime`/`app_kind`。

**round3 SF-5で追記**: `Rejection.vk`は文字列（例: `"VK_DBE_ALPHANUMERIC"`）だが
`ime_key_for`が返すのは`VkCode`型。突き合わせには名前→`VkCode`の解決が要り、
`crates/awase-windows/src/vk.rs`の文字列matchが使える（非gatedのため利用可）。
また候補に挙がりうる`F22`は`b271aee`で定数自体が削除されている
（`docs/experiments.md:59`参照）——解決できないVK名を宣言に含める場合の扱い
（文字列のまま保持して解決失敗を許容するか、宣言対象から外すか）を実装前に決めること。

**依存**: TD0。

**検証方法**: 宣言が意図通りの件数・内容でコンパイルできることを確認する。

### TD2: `key_sequence_policy.rs`とのユニットテストによる突き合わせ

**内容**: `ime_key_for`に対する`#[cfg(test)]`ユニットテストとして実装する。
`REJECTED_IME_OFF_KEYS`の全エントリと、`ime_key_for`が実際に返すVKの組み合わせを突き合わせ、
禁止されたVKが選ばれていないことを検証する。

**依存**: TD1（TD0の切り出しが前提）。

**round3 MF-2で訂正**: TD0で`ime_key_for`等を非gatedモジュールへ切り出していれば、本テストは
**ホストの`cargo test -p awase-windows --lib`で実行できる**——「意図的に実装を書き換えて
テストが落ちることを確認してから元に戻す」という検証が可能。切り出しを行わない場合のみ、
`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`でのコンパイル
確認とwindows-build CIへの実行委譲がフォールバックとなる（round2 M-1が指摘した制約は
TD0で解消される前提）。

### TD3: 適用範囲の限界をドキュメント化

**内容**: TD2のテストが防げるのは`ime_key_for`のmatch表選択の失敗のみであり、GJIキーマップの
実行時読み取り・config文字列パース・注入経路といった実行時経路の失敗（`docs/experiments.md`
エントリ07・08・09）は防げないことを、テストのdocコメントと
`.claude/rules/experiment-logging.md`に追記する。

**依存**: TD2。

**検証方法**: テストのdocコメントに上記の限界が明記されていることをレビューで確認する。

### TD4（別件、round1の副産物）: `docs/experiments.md`のエントリ17重複採番を修正

**内容**: `docs/experiments.md`の28行目（ファイル冒頭、エントリ01より前にある「BUG-25 GJI
半角英数entry」）と707行目（`key_remap`撤回、こちらは17〜25の連番として整合している）が
両方「エントリ17」を名乗っている。**round2 M-8で採番方針を訂正**: 707行目側を繰り下げると
505行目の既存エントリ16と衝突する。

**round3 SF-6で「時系列位置へ移動し16〜25を繰り下げる」方針へ変更したが、round4 MF-1・
MF-2で2つの欠陥が判明し撤回した**:

- MF-1: 却下の論拠にした「エントリ01〜25は日付順」という前提が実データで成立しない
  （例: エントリ11＝2026-05-15はエントリ03＝07-06より前、エントリ13＝08-06は
  エントリ12＝08-07より前、エントリ16＝08-06はエントリ12〜15より前、エントリ23＝08-05は
  エントリ19〜22＝09-02〜09-05より前、エントリ18は日付記載なし）。加えて範囲指定も
  誤っていた（28行目のエントリは2026-08-27で、エントリ16の最終日付08-24より後・
  エントリ17＝08-30より前なので、時系列挿入するなら対象は17〜25であって16〜25ではない）。
- MF-2: エントリ16〜25は`docs/adr/100-gji-warmup-vk-ime-on-reinit.md`（エントリ16を
  8箇所以上、専用節見出しあり）・`docs/adr/153-*.md`（エントリ25）・
  `docs/adr/138-*.md`（エントリ21・22・23）・`docs/adr/index.md`・
  `docs/adr/161-single-source-spec-generation.md`から番号で参照されており、
  再採番はこれらを静かに破壊する。ADR-158群が解消しようとしているRC3
  （同じ事実が複数系統に手書きコピーされ参照整合性がない）を、このタスクリスト自身が
  新規に作り出す変更になっていた。

**方針を再確定（round4反映、round5 SF-2で既定手順をさらに訂正）**: 707行目側の連番
（17〜25）はそのまま動かさない。**28行目のエントリは移動せず、その場（28行目）で
見出しの番号だけを「エントリ17」から「エントリ26」に書き換える**のを既定の手順とする
（番号がファイル出現順にならない＝26, 01, 02, …, 25という見た目になる代償のみで、
`docs/experiments.md`内の他エントリの行番号は1行も動かない）。

**round4時点では「ファイル末尾へ移動」も許容していたが、round5 SF-2で却下した**:
移動すると28行目より後ろの約13行分、`docs/experiments.md`の行番号が全て繰り上がり、
`158-implementation-tasks.md:226`（TD1、round3 SF-5で追加した`docs/experiments.md:59`
参照）・`161-single-source-spec-generation.md:105`・`161-single-source-spec-generation.md:481-482`・
本タスク自身の「28行目」「707行目」という記述が、MF-2で守ろうとした番号参照とは別に
**行番号参照として**壊れる。その場書き換えならこの追随作業自体が不要になる。

**依存**: なし（TD1〜TD3とは独立、いつ着手してもよい）。

**検証方法**: (1) 採番部分を抽出し、連番かつ重複がないことを確認するスクリプト
（単純な`grep -c`ではなく、番号を抽出してソート・重複チェックする）で確認する。
(2) **round4 MF-2を受けて追加**: `docs/experiments.md`のエントリ番号・行番号を
参照している既知の全箇所（`docs/adr/100-gji-warmup-vk-ime-on-reinit.md`、
`docs/adr/153-*.md`、`docs/adr/138-*.md`、`docs/adr/index.md`、
`docs/adr/161-single-source-spec-generation.md`、および`grep -rn "experiments.md.*エントリ\|エントリ.*experiments.md"`で
新規に見つかる箇所）を変更前後で洗い出し、リンク切れ・番号不整合が生じていないことを
確認する。

---

## タスクグループTE: tuning定数・キュー・AppImeProfileの宣言化（[ADR-158](158-complexity-reduction-north-star.md)第5段階）

### TE1: tuning定数への`#[measured(...)]`本実装

**内容**: `#[measured(value_ms, margin_ms, commit)]`属性マクロを実装し、
`crates/awase-windows/src/tuning.rs`の定数に適用する。

**round2訂正（M-5・M-6）**:
- **実装先**: `crates/awase-vkmap`は通常ライブラリでproc-macroを同居できない（`[lib]`に
  `proc-macro = true`が必要かつマクロ以外をexportできない）。**新規crateを1つ作る**。
- **定数の現況**: `tuning.rs`の`_MS`定数は実測**34個**（round2 S-12で33から訂正、非`_MS`の
  `IME_DETECT_MISS_THRESHOLD`を含めると35個）。`.claude/rules/tuning-constants.md`が引用する
  4定数（`CHROME_PROBE_MIN_MS`等）は**現在のtuning.rsに1件も存在しない**（リネーム/削除
  済み、ルールファイル自体が古い）。tuning.rs内にコミットハッシュ表記は0件。
- **段階導入**: `#[measured]`は`value_ms`と`commit`を必須にする実装のため、34定数すべてに
  一度に適用しようとすると34件分のgit考古学が前提になり「順次適用する」という当初方針と
  矛盾する。**未計測の定数向けに`#[measured(pending = true)]`のような猶予用バリアントを
  用意するか、計測が取れた定数から1つずつ適用するか**を先に決める。

**依存**: なし（TA〜TDと並行できる）。

**検証方法**: 属性を付けた定数について`cargo check`が通ること。`.claude/rules/
tuning-constants.md`記載の実測値（`79134f5`の326ms等）が、対応する**現行の**定数名に
転記できるかをまず確認してから着手する。

### TE2: `pending_deferred`の宣言強制（round2 M-7で機構を再選定）

**内容**: **round2で判明**——`actuation_call_guard_spike`型の許可リスト方式dylintは
「許可外から呼ばれた」という**存在**は検出できるが、「本来呼ぶべき窓口で呼ばれていない」
という**不在**（ADR-123→ADR-128回帰そのもの）は原理的に検出できない
（[ADR-161](161-single-source-spec-generation.md)判断の手順・問い5参照）。機構を以下の
いずれかに変更する:

- (a) defer側・drain側両方が呼ぶべき条件判定を1つの共有関数に集約し（通常のリファクタ）、
  「その共有関数がdefer側・drain側の両方の関数本体から実際に呼ばれているか」という**存在**の
  確認に問いを反転する新しいdylintを書く。
- (b) [ADR-156](156-unify-deferred-execution-queues.md)の`ReleaseToken<G>`案（型システムに
  よる強制）。

いずれも本タスク着手前に小さな実証実験で実現可能性を確認すること（[ADR-161](161-single-source-spec-generation.md)
の手順を踏襲）。

**round2訂正（S-12）**: 対象窓口の所在は実測**8ファイル**（`output/tsf_warmup_coord.rs`
44件・`output/mod.rs`33件・`output/vk_send.rs`22件・`journal.rs`13件・`platform.rs`9件・
`journal_policy.rs`/`output/probe_io.rs`/`tsf/warmup/probe_fsm.rs`各1件）——round1の訂正
（5ファイル）よりさらに分散が広く、(b)の`ReleaseToken<G>`案の成立見込みにも疑問符が付く。

**依存**: TA2（dylint運用パターン）、実証実験（未実施、本タスクの最初のステップとする）。

**検証方法（round3 MF-3で追加）**: 着手前の小さな実証実験で、以下の合否条件を確認する。
- (a)案の合否: 「共有ゲート関数がdefer側・drain側の両方から実際に呼ばれているか」という
  存在確認を行う簡易dylintを試作し、片方の呼び出しだけを意図的に削除したコードで検知
  できれば合格。8ファイルへの分散を踏まえ、検知対象の関数を明確に列挙できるかも確認する。
- (b)案の合否: `ReleaseToken<G>`型の構築を、8ファイルにまたがる利用側から実際に行えるか
  （`pub(crate)`で足りるか、共通祖先モジュールがクレートルート相当になり制約が働かないか、
  [ADR-161](161-single-source-spec-generation.md)実証実験3と同型の問題が起きないか）を
  最小コードで確認する。
- **(a)(b)双方が不成立の場合**: 新機構の追加は見送り、defer側・drain側の条件判定を
  共有関数に集約するリファクタと、それを検証する通常のユニットテスト（ADR-123→ADR-128型の
  回帰を模したテストケース）で代替する。この分岐は8ファイルという分散度から見て現実的な
  可能性として想定しておくこと。

### TE3: `AppImeProfile`能力表の段階的宣言化

**内容**: `focus/class_names.rs::AppImeProfile`の各バリアントが持つ能力を、まず
[ADR-161](161-single-source-spec-generation.md)実証実験5の属性マクロ（実行時記録版）で
観測し、数セッション分のログを集めた上で、確信が持てた範囲からdylintの許可リストへ
昇格する。

**依存**: **round2 S-9で訂正**——TA2（dylint運用パターン、round3 SF-1で表記統一）に加えて、
実証実験5（`#[actuation_choke_point]`）の本実装化タスクが必要（現状このタスクリストに
存在しないため、TE3着手前に「実証実験5を`crates/macro-spike`から正式なcrateへ格上げする」
というサブタスクを別途起票すること）。

**検証方法**: 観測フェーズでは実機ログに`[actuation-record]`相当の出力が実際に現れることを
確認。昇格フェーズでは、issue #136型の回帰（gateを1箇所だけに置いて自己回帰する）を模した
テストケースで新lintが検知することを確認する。

---

## タスクグループTF: ADR-159段階1・段階2（round2 S13-3で追加、[ADR-162](162-governance-reversal.md)全施策の着手条件）

### TF1: journal.rsへの観測側記録追加（段階1の一部）

**内容**: `journal.rs::JournalEntry`（19バリアント）に、`ObservationSource`11バリアントと
5つのWin32受信入口の記録を追加する設計を詰め、最小限（1〜2バリアント）を実装する。

**依存**: なし。

**検証方法**: `tests/journal_replay.rs`に新バリアントを使ったテストケースを1本追加し、通ること
を確認する。

### TF2: `send_input_safe`/`send_ime_control`への差分記録（段階2の一部）

**内容**: TB0で宣言した2つのチョークポイントに、シャドー実行の差分記録を最小限（1つの
条件分岐のみ）挿入する。

**依存**: TB0。

**検証方法**: 実機セッションで差分記録が1件以上出力されることを確認する
（[ADR-162](162-governance-reversal.md)が要求する「1件以上の実績」の最小達成基準）。

**注記**: TF1・TF2はそれぞれ最小実装でよい——[ADR-162](162-governance-reversal.md)の着手
条件「段階1または段階2が実際に1件以上の実績を出す」を満たすことが当面の目的であり、
段階1・段階2の完全な実装は別途段階的に進める。

---

## タスクグループTG: ADR-160 C1〜C3の判断材料収集（round2 S13-5で追加）

### TG1: C1〜C3それぞれの判断材料を収集する

**内容**: [ADR-160](160-explicit-non-scope-declaration.md)が定める判断材料
（C1: ADR-095集計、C2: `focus/`学習キャッシュのカバレッジ実測とBUG-107型汚染の発生頻度、
C3: conv-mode約214箇所の用途棚卸し）を収集する。

**依存**: なし（判断材料の収集自体はいつでも着手できる）。ただし判断確定の期限は
[ADR-162](162-governance-reversal.md)のE3（四半期定例棚卸し）に依存する——E3が未確立の
場合、収集した材料は確定判断に使われないまま待機する。

**検証方法**: 各案の判断材料が数値または具体的な事実として記録されること。

---

## タスクグループTH: ADR-162 E1〜E4（round2 S13-6で追加、待機中であることを明記）

以下はすべて[ADR-159](159-existing-io-boundary-inventory.md)の段階1（TF1）または段階2
（TF2）が実際に1件以上の実績を出すまで**着手しない**。着手条件が満たされ次第、
このグループから着手する。

### TH1: E1運用規約の配置場所確定

**内容**: `.claude/rules/`のどのファイルに追記するか、または新規ファイルとするかを決める。
監視対象となる宣言の場所・定数名（TB0・TB1で確定する）を確定する。対象は「actuation合流点数」
のみ（round4 TJ4 M1で「gate数」を対象外に整理）。tuning定数数もE1の初期対象に含める
（round4 TJ4 S3）。棚卸しによる宣言追加と新規複雑性の追加を`git log -S`で区別する運用
（round4 TJ4 M2）、超過を許す例外条項（round4 TJ4 M3）も規約に含める。

**依存**: ADR-159の記録・再生基盤が能力ベースの基準（実際の削除・統合を1件、記録トレースの
再生で送信列差分ゼロと検証できたこと。round4 TJ4 M5でTF1/TF2の配線確認だけでは不十分と訂正）
を満たすこと、TB0、TB1。

### TH2: E2 ADRのTTL期間設定

**内容**: 着手判断時点までのADR起票〜確定の実測期間から、TTL期間を設定する。あわせて
`docs/adr/index.md`の行に対応する本文ファイルが存在しない場合をCIで検出するチェックを追加する
（round4 TJ4 M4——TTLの動機付け事例をADR-151/152からADR-156型の「議論完了・実施判断未了」の
放置ケースへ差し替えた上での代替案。ADR-151/152型の「本文ファイル自体が無い」失敗にはこちらが
直接効く）。

**依存**: なし（round4 TJ4 S1でADR-159依存を撤回——コードを削除しないため）。今すぐ着手可。

### TH3: E3四半期定例棚卸しの実施形式確定

**内容**: 本ADR群のような並行調査+opus-adversarial-consult構成を毎回繰り返すか、簡略化するかを
決める。

**依存**: なし（round4 TJ4 S1でADR-159依存を撤回——コードを削除しないため）。今すぐ着手可。
ただしADR-160のC1〜C3判断期限の基準として使われるため、着手が遅れるとADR-160側の期限も
連動して遅れる（ADR-160側には2026-12-31の絶対バックストップを別途設定済み）。

### TH4: E4 known-bugs.mdのコード回帰

**内容**: known-bugs.md（16,825行）の一部を、TF1・TF2の再生トレースへ段階的に変換し、散文は
要約1行に落とす。**round4 TJ4 S2で追加**: `.claude/rules/fix-requires-evidence.md`の選択肢
(b)（known-bugs.mdへの追記）を「再生トレースの追加」に置き換えるか散文の分量上限を定める
改訂を本タスクに含める——これを欠くと新規fixによる散文の流入が止まらず、在庫だけを変換する
片働きになる。

**依存**: ADR-159の記録・再生基盤が能力ベースの基準（TH1と同基準）を満たすこと、TH3
（E3の定例サイクルで対象を選定する運用を想定）。

---

## タスクグループTI: ADR-161 D2（純粋層proptest、round2 S13-1で追加）

### TI1: `classify_*`関数へのproptest適用スパイク

**内容**: `pub fn classify_*` 6個に対して、既存のproptestパターン（ルート`awase`クレートに
実例あり）を適用するスパイクを行う。

**依存**: なし。

### TI2: D2対象範囲の見極め

**内容**: `observation_store.rs`等の証拠管理コードから、モデル検査可能な粒度まで純粋層を
切り出す対象範囲を見極める。

**依存**: TI1。

---

## タスクグループTJ: 各ADR単体のopus-adversarial-consult実施（round2 S13-7で追加）

### TJ1: ADR-161単体のround1レビュー

**内容**: [ADR-161](161-single-source-spec-generation.md)は北極星ADR-158からの分割時に
round1を経ているが、本ADR単体としてのopus-adversarial-consultはまだ実施していない。
D1の機構がsynからdylintへ組み替わった直後であり、round2でさらに判断基準が追加される等
まだ変動しているため、**`TA2`着手前に実施することを推奨する**。

**依存**: なし（最優先で着手可能）。

### TJ2〜TJ4: ADR-159・160・162単体のround1レビュー（round3 SF-8でID範囲を訂正）

**内容**: それぞれのADRのステータス欄が「本ADR単体としてのopus-adversarial-consultは
未実施」と明記している。各ADRの実装（TB・TC・TD・TE・TF・TG・TH群）に着手する前に実施する
ことが望ましい。対象は3ADR（TJ2=ADR-159、TJ3=ADR-160、TJ4=ADR-162）——round2時点の
見出し「TJ2〜TJ5」は4ID分あったが対象ADRは3件しかなく、round3 SF-8で「TJ2〜TJ4」に訂正した。

**依存**: なし。

---

## 依存関係の全体図（round3 SF-8で4点訂正・round4 SF-1でさらに1点訂正・TJ1〜TJ4反映で3点訂正）

```
TJ1〜TJ4（実施済み、2026-09-09）
TA1 → TA2 → TA3
        ├─→ TB0（ADR-159段階0の主目的）
        ├─→ TB1
        ├─→ TE2（実証実験含む）
        └─→ TE3（実証実験5の格上げも要）
TB0 → TB2
TB1 → TB2
TC1 → TC2 → TC3（TB0/TB1にも依存）
TD0 → TD1 → TD2 → TD3
TD4（独立）
TE1（独立）
TF1（独立）
TF2（TB0依存）
TB0・TB1・ADR-159実績（能力ベース） → TH1
TH2（独立、今すぐ着手可）
TH3（独立、今すぐ着手可）
TH3 → TH4（ADR-159実績（能力ベース）も着手条件）
TG1（独立、判断確定はTH3のE3に依存）
TI1 → TI2（独立）
```

**round3 SF-8での訂正内容**:
- TB0→TB1という直列の辺は誤り。TB1の依存はTA2のみ（TB0には依存しない）——両者は
  TA2から並行に分岐し、TB2の手前で合流する。
- TH1の依存にTB0・TB1が抜けていた（TH1本文: 「依存: TF1またはTF2（着手条件）、
  TB0、TB1。」）。
- TH3→TH4の辺が抜けていた（TH4本文: 「依存: TF1、TF2（着手条件）、TH3」）。
- TJ2〜TJ4（round2版では存在しないID範囲「TJ2〜TJ5」だった）が図から漏れていた。

**round4 SF-1でさらに訂正**: `TB2 → TC3`という辺は誤りだった。TC3本文の依存は
「TC2、TB0、TB1」でTB2を含まない（着手順推奨側は元々TC3をTB0/TB1完了後としており
本文と一致していた）。上記の図から該当行を削除した。

**TJ1〜TJ4反映（2026-09-09）でさらに訂正**: ADR-162側のround4 TJ4レビューで、E2（TH2）・
E3（TH3）はコードを削除しないためADR-159への依存が不要と判明し（S1）、TF1/TF2完了を着手条件
から外した。E1（TH1）・E4（TH4）は依存を維持するが、基準を「TF1/TF2の配線確認」から「ADR-159
の記録・再生基盤が能力ベースの基準（実際の削除・統合1件を送信列差分ゼロで検証できたこと）を
満たすこと」に訂正した（M5）——TF1/TF2の完了と同義ではなくなったため、上記の図では
「TF1」「TF2」ノードへの依存辺を「ADR-159実績（能力ベース）」という別ノードに置き換えた。

## 着手順の推奨（TJ1〜TJ4実施済み、2026-09-09）

1. ~~TJ1〜TJ4~~（ADR-159・160・161・162単体レビュー、実施済み。指摘は各ADR本文・本タスク
   リストに反映済み）
2. TA1 → TA2 → TA3（第1段階、dylint運用パターンを確立。TJ1レビューで着手可と確認済み）
3. TH2・TH3（E2のTTL・E3の棚卸し形式確定。TJ4レビューでADR-159依存が不要と判明したため、
   最優先級に繰り上げ——TH3はADR-160のC1〜C3判断期限の基準でもあるため早期着手が有利）
4. TC1 → TC2（コストが低いが2段手順が必須、単純な1行変更ではない点に注意）
5. TD4・TD0 → TD1 → TD2 → TD3（第4段階、機構がユニットテストに訂正されたため実装コストは
   低い。**round4 SF-2で訂正**: TD0で`ime_key_for`等を非gatedモジュールへ切り出せば、
   TD2の検証はホストの`cargo test -p awase-windows --lib`で完結し、windows-build CI待ちには
   ならない。**TJ1 M3で追記**: TD0の必要性自体、既存テスト・ゴールデンが同じ事実を既に
   固定していないか確認してから着手する）
6. TB0・TB1（TA2完了後、並行着手可。**TJ2 MF1/MF2を反映**: `send_input_safe`20箇所と
   `send_ime_control`10箇所を分けて宣言し、後者は`(関数, cmd)`粒度で宣言する）→ TB2
   （第2段階、ADR-159段階0の主目的を含む。**TJ1 M1/M2を反映**: 宣言レコードのフィールド
   最小集合とarchitecture_guard.rsの置き換え方針を先に確定する）
7. TF1・TF2（ADR-162のE1・E4のブロッカー解除、最小実装でよい）
8. TC3（第3段階、TB0/TB1完了後）
9. TE1・TE2（実証実験含む）・TE3（第5段階、並行着手可能）
10. TG1（判断材料収集はいつでも着手可。**TJ3レビューを反映**: C1はしきい値・報告JSON再取得
    手順を、C2は計装新設コストを、C3は読み取り/書き込み別建ての棚卸しと半角英数状態を通過する
    操作を含む追加実測を、着手前に材料の定義へ織り込む）
11. TH1（TB0/TB1・ADR-159実績[能力ベース]完了後）・TH4（TH3・ADR-159実績[能力ベース]完了後）
12. TI1 → TI2（並行着手可能）
