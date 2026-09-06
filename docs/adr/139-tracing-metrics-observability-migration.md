# ADR-139: ログ/メトリクス基盤を `log` から `tracing`/`metrics` エコシステムへ移行する

## ステータス

採用・実装済み（Opus 2体〈architect役/premortem役〉による敵対的レビューを設計段階3ラウンド、
実装コードレビュー段階2ラウンドの計5ラウンド実施、いずれもBlockerゼロで収束）。
決定1〜4をPR #172でdevelopへマージ済み（マージコミット `48406822`、2026-09-06）。
決定2の非同期化（`non_blocking` writer）・決定5-1（awase-settings観測パネル、
別プロセスIPCが片方向のみのため実現不能と判明し削除）・決定6（`tracing-etw`）は
本ADRのスコープ外のまま未実装（詳細は各決定の本文参照）。

## コンテキスト

### 現状の実測（レビューで訂正済み）

- 全ワークスペースで `log = "0.4"` のみを使用。呼び出し箇所は **1161箇所**
  （`awase-windows/src` 736 + 同 `tests` 301 + core(`src/`) 他）。当初「880箇所超」と見積もったが
  `tests/` 配下を数え落としていた。`tracing::*` マクロ・`#[instrument]` の使用は0箇所。
- バックエンドは `env_logger = "0.11"`。出力ファイル名は **`awase.log`**（`app.log` ではない、
  `app/bootstrap.rs:116`）。`OpenOptions::append` で開いた素の `File` に
  `env_logger::Target::Pipe` で書いており、**`BufWriter` を挟んでいない = 1レコード1
  `WriteFile` syscall**。ローテーションも無く、`RUST_LOG=debug` 起動で
  `awase.log` が **747MB まで肥大化した実測記録**が残っている（`docs/adr/125-*.md:287`）。
- `metrics` crate は workspace のどの `Cargo.toml` にも存在しない。ADR-120 Phase 0a の
  「3鍵仲裁観測カウンタ」等は都度 struct に手書きされ、共通の記録・エクスポート経路を持たない。
- `Cargo.lock` に `tracing 0.1.44` は存在するが、**facade（`tracing`/`tracing-core`/
  `tracing-attributes`）のみ**であり、`winit`（`awase-settings`/`awase-linux` の
  `eframe`/`egui` 経由）からの推移的依存にすぎない。`awase-windows/Cargo.toml` には
  `tracing` は無い。`tracing-subscriber`/`tracing-appender`/`matchers`/`sharded-slab`/
  `thread_local`/`nu-ansi-term`/`crossbeam-channel` は**全て新規依存**であり、
  「既に取り込まれているので格上げに近い」という当初の評価は誤りだった。
- `ApplyGeneration`（`generation`）等の相関情報は、呼び出し元が毎回
  `log::info!("... generation={generation:?}")` のように**手で埋め込んで**おり、
  呼び出し階層をまたいだ自動継承がない。
- 不具合報告機能（`bug_report.rs`, ADR-095）は既に **2種類の構造化/半構造化データ**を
  持っている: `log_excerpt`（journal 由来の構造化 JSON）、`app_log_excerpt`（`awase.log`
  末尾のテキスト、`truncate_text_tail` 経由）、さらに `BugReportStateSnapshot`
  （`bug_report.rs:111-163`）という **`schema_version` 付きの型付き契約**があり、
  `docs/bug-reports-triage.md` と `bug-report-fetch`/`bug-report-latest` Skill という
  既存の消費者がいる。「構造化フィールド抽出ができていない」という当初の課題認識は
  一部誤りで、実際の課題は「新しい観測値を追加する手段が2系統（ログ本文 / この型付き
  snapshot）に分かれていて一貫しない」ことに近い。
- 独自の event-sourced ジャーナル基盤（`journal.rs`, `journal_policy.rs`,
  `tests/journal_replay.rs`）が既にあり、`state::ime_event::ImeEvent` 等の**ドメイン型を
  そのまま**記録する（ADR-082 決定1）。`journal_replay.rs` は `tests/journals/*.json` に
  **手で転記・レビューした** fixture を読む決定論的リプレイ基盤であり、"tracing化すれば
  自動的に良くなる" 類のものではない。

### フックスレッドの実態（当初の記述は誤りだった）

`WH_KEYBOARD_LL` フックコールバック（`hook.rs::hook_callback`, 889-1207行）は、
**通常のキー打鍵経路ではログ呼び出しがゼロ**である。`hook_channel.rs:183-199` に
「フックコールバック上ではロック取得・アロケーション・ブロッキング呼び出し・
**ログ出力を一切行わない**」という不変条件が既に明文化・実装されている。ログが出るのは
IME モードキー・`VK_KANA`・`VK_DBE_ROMAN`/`NOROMAN`・Alt 系 vk 等の**稀な分岐のみ**
（`hook.rs:998` に「VK_KANA は稀なキーなのでログコストは無視できる」という評価コメントが
既にある）。当初「hook.rs はフックスレッド上で同期ログ I/O をしており移行前から
レイテンシリスクがある」と記述したが、これは誤りである。

実際に問題があるのは別の箇所である: `awase.log` への書き込みは**エンジンスレッド**上で
行われ、`BufWriter` なしのため AV スキャン等のディスク stall が数十ms 起きると
エンジンスレッドが止まる。エンジンが止まれば `HOOK_KEYS`（CAP=1024）が埋まり、
`hook_ring_max_occupancy`（`bug_report.rs:162`）という**既存センサーが既に観測できる
形で置いてある**。

### 見落とせない制約

- `awase`（root, core crate）は ADR-019 によりプラットフォーム非依存を維持する義務があるが、
  これはロギング facade の選択とは無関係（`log = "0.4"` は既に root crate の直接依存）。
- 並行モデルはメッセージループ駆動のシングルスレッド（`winmsg-executor`、`spawn_local`、
  tokio 不使用）。`RUNTIME: SingleThreadCell<Runtime>`（`lib.rs:198`）はメインスレッド専用の
  `&mut` アクセスを前提としており、`tracing::Subscriber`/`Layer` トレイトが要求する
  `Send + Sync` とは相性が悪い（後述、決定4）。
- `.claude/rules/tuning-constants.md` に列挙されている通り、タイミング定数は
  20ms〜500ms 台で実測に基づき詰められているが、これは**メインスレッド上の probe
  予算**であり、フックスレッドが従う予算は別物（`LowLevelHooksTimeout`、既定5000ms、
  `hook.rs:650-675`）。両者を混同しないこと。
- `.claude/rules/fix-requires-evidence.md` の「再発ファミリー」表は、`#[instrument]` の
  効果が最大化される領域を既に列挙している（決定3で全数照合済み）。

## 決定

### 決定1: `log` から `tracing` へ一括移行する（`tracing-log` 恒久併用はしない）

マクロレベルでは `log::info!("...")` → `tracing::info!("...")` は機械的な置換で足りる。
`tracing-log` によるブリッジ併用を恒久方針にはしない — 2つの計装スタックが並存すると
「新しいログをどちらのマクロで書くべきか」を毎回判断するコストが発生する。移行期間中の
一時的な併用のみ許容する。

`tracing-subscriber::fmt` + `EnvFilter` を使い `RUST_LOG` 互換性を維持する。出力行の
フォーマット（`env_logger` の `[ts LEVEL target] msg` から `tracing-subscriber` 既定の
`ts LEVEL target: msg` に変わる）は**そのまま受け入れる**。メッセージ本文自体は変わらない
ため、`docs/adr/048-*.md:148` のようなメッセージ内接頭辞（`[tsf-probe]` 等）に依存した
既存の調査手順は生き残る。

Phase 1 の全数調査（880件超という表現は誤りで、実測1161件のうち）の結果:
- `target:` 指定、`log_enabled!`、`log::max_level`、`set_boxed_logger`、独自 `Log`
  実装、動的フォーマット文字列は**いずれも0件**。単一行640件・複数行475件、
  合計 **1115件は純粋な機械置換（sed 相当）で通る**。
- **機械置換できない残り6箇所**（全数特定済み。タスク分解レビューで6箇所目を追加）:
  1. `crates/awase-windows/src/keymap.rs:225,232` — `level: log::Level` を引数に取り
     `log::log!(level, ...)` する実行時レベル分岐。`tracing::event!` はレベルが
     コンパイル時定数でなければならず直接の等価物が無いため、`match level { Warn => ..,
     Debug => .. }` の5分岐へ展開する（呼び出し元 `runtime/mod.rs:1301,1712` と
     `keymap.rs:616-624` のテスト3件も追随）。この設計は ADR-114 レビュー指摘 m-4 で
     意図的に導入されたもので、`keymap.rs:214-220` に「`recompute_active_keymaps()` は
     フォーカス変更のたびに呼ばれるため Warn 固定だと不具合報告の `awase.log` が
     スパムで埋もれる」という理由が明記されている。**レベルの出し分け設計自体は
     置換後も維持すること。**
  2. `crates/awase-settings/src/main.rs:192` の `log::logger().flush()` — `tracing` に
     等価物は無く、`WorkerGuard` の drop、または明示 flush 呼び出しに置き換える
     （決定2参照。同ファイル186-193行の `log_checkpoint` は「実機で『配列編集タブ関連の
     ログが一切出ない』と報告された際の切り分け用に導入」とドキュメントされており、
     クラッシュ直前の末尾ログを残すという目的を置換後も維持しなければならない）。
  3. `crates/awase-windows/tests/e2e_windows.rs:44` の `env_logger::Builder ...
     .filter_level(log::LevelFilter::Debug)`。
  4. subscriber 初期化4箇所（`awase-windows/src/app/bootstrap.rs:132-153`,
     `awase-linux/src/main.rs`, `awase-macos/src/main.rs`,
     `awase-settings/src/main.rs`）。
  5. **`crates/awase-windows/tests/architecture_guard.rs:1406`** — 最重要。
     `extract_drift_correction_match_block` という関数が、`runtime/ime_refresh.rs` の
     ソーステキスト中の **`"log::warn!("` という文字列をコード領域の構造マーカーとして
     使っている**（`.rfind("log::warn!(")`、見つからなければ `panic!`）。このテストは
     ADR-080 不変条件6／BUG-33「収束偽装」（belief を観測として書き戻して drift 検知が
     二度と発火しなくなる）の再発ガードである。**Phase 1 で `ime_refresh.rs` を置換すると
     このテストが確実に panic する**ため、「Phase 1 は CI が無傷であることを確認してから
     次フェーズへ」という進め方は成立しない。マーカー文字列の更新を `ime_refresh.rs` の
     置換と**同一コミット**に含めることを Phase 1 の必須項目とする。
  6. **`crates/awase-linux/src/output.rs`** — ワークスペース内で唯一
     `use log::warn;`（:8行目）で `warn!` をインポートしており、プレフィックス無しの
     bare `warn!(...)` が6箇所（:131,150,159,165,184,200）ある。他の1155箇所を
     `log::(trace|debug|info|warn|error)!` という正規表現で機械置換しても、この
     6箇所は一致せず取り残される。同ファイルの `log::debug!`（:245,250,253,256）は
     通常の機械置換対象。この6箇所を見落として `log` 依存を Cargo.toml から削除すると
     `awase-linux` が `unresolved import` でビルド不能になるため、個別に
     `tracing::warn!` のフルパス呼び出しへ置換し `use log::warn;` を削除する。

`#[expect(clippy::cognitive_complexity)]` が本番コード13箇所にあり
（`clippy.toml` の閾値15、CI で `-D warnings` 明示有効化）、マクロ展開形の違いで
スコアが動く懸念があったが、**実測で否認された**: 構造が同一でマクロだけ異なる関数を
実際にビルドして比較した結果、`log::info!` / `log::warn!(引数あり)` / `tracing::info!`
（メッセージのみ）/ `tracing::info!`（構造化フィールド付き）は**すべて同一スコア**
だった（clippy の `cognitive_complexity` は `ExprKind::If`/`Match` の個数のみを数え、
マクロ展開後の内部構造の差は見ない）。唯一動くのは `keymap.rs::warn_if_vk_conflicts`
（上記非機械的ケース1）だが、元の複雑度が低いため閾値には届かない。Phase 1 の設計項目
ではなく、「置換後に `cargo clippy` を通す」という通常の検証手順で足りる。

dylint 3本（`lints/no_vk_as_scan`, `lints/ime_event_guard`,
`lints/observation_source_guard`）と `layer_boundary_guard.rs` は `log` に一切依存
しておらず、Phase 1 の影響を受けない（`architecture_guard.rs` の `log::` 参照10件も
`:1406` の1件を除き全てコメント/エラーメッセージ文字列で無害と確認済み）。

`fmt` の影響: ログ呼び出し1161件中 **437件（複数行）が `log::` → `tracing::`
のマクロ名長変化で `cargo fmt` の折り返しを変え**、無関係な再インデント差分が出る。
マクロ名の機械置換コミットと `cargo fmt` 適用コミットは分離すること。

**Phase 1 と同時、または直前の独立修正（このADRの完了を待たない）**: 当初
「`.githooks/pre-push` の再発ファミリー正規表現に `transport.rs`/`ime_refresh.rs` が
含まれていない」としたが、タスク分解レビューで再確認したところ **`transport` は
`.githooks/pre-push:28` に既に含まれており、欠けているのは `ime_refresh` のみ**
だった。さらに重大な点として、**このマシンで実際に実行される pre-push フックは
`.githooks/pre-push`（Git 管理下）ではなく `.git/hooks/pre-push`（`core.hooksPath`
が指す、Git 管理外の別ファイル）であり、両者は既に乖離している**
（`.git/hooks/pre-push:28` は `transport` も `ime_refresh` も含まない古いコピー）。
`.githooks/pre-push` だけを直しても、このリポジトリの実際の pre-push 実行結果は
1ビットも変わらない。対応は2段階: (1) `.githooks/pre-push` に `ime_refresh` を追加する、
(2) `.git/hooks/pre-push` を `.githooks/pre-push` の内容で同期する（またはユーザーの
同意を得た上で `core.hooksPath` を `.githooks` に向け直す）。(2) はリポジトリの
フック実行設定を変える操作なのでユーザー確認を要する。

### 決定2: ファイル writer への `BufWriter` + ローテーション導入（同期のまま）。非同期化は本ADRでは決定しない

当初「フックスレッドの同期ログ I/O を除去する」という非連続な提案をしたが、前提が
誤りだった（上記「フックスレッドの実態」参照）。`hook_channel.rs:183-199` の不変条件は
既に構造的にログ出力を排除しており、`tracing-appender::non_blocking` を導入しても
「フックコールバックで自由にログしてよい」ことにはならない（`non_blocking` はフォーマット
後の `Vec<u8>` コピー＝ヒープ確保を呼び出しスレッド側で行うため、まさに
`hook_channel.rs:199` が禁じている操作に抵触する）。

実際に対処すべきは以下の2点であり、こちらを決定2の目的とする:

1. **エンジンスレッドの stall**: `awase.log` の書き込みに `BufWriter` を挟み、
   1レコード1 syscall を解消する。
2. **ログ肥大**: 同期ローテーションを導入し、747MB 肥大化（実測、
   `docs/adr/125-*.md:287`）を防ぐ。

**ローテーション実装の追補（タスク分解レビューで判明）**: 当初 `tracing-appender::rolling`
を想定したが、これは時間ベース（MINUTELY/HOURLY/DAILY/WEEKLY/NEVER）のローテーションしか
持たず、実測根拠（747MB は単一セッション中の肥大化）に合わない。さらにローテーション後の
ファイル名が日付サフィックス付きになるため、`bug_report_log_path()`/`app_log_path.exists()`
（`app/mod.rs:3`, `runtime/message_handlers.rs:1144-1145`）が前提とする「`awase.log` という
固定パスが常に存在する」という契約を壊す。**`tracing-appender::rolling` は採用しない。**
代わりに、固定パス `awase.log` を維持したまま、自前の軽量な1世代ローテーション
（閾値超過で `awase.log` → `awase.log.old` にリネームしてから新規 `awase.log` を開く）を
実装する。awase は常駐トレイアプリで数日〜数週間再起動されないため、**起動時チェックだけ
では不十分**であり、セッション中も（既存のウォッチドッグ tick 等に相乗りして）周期的に
書き込みバイト数を見てローテーションを発火させる。Windows では開いたままの `File` は
`rename` できない（`FILE_SHARE_DELETE` 無しでは `ERROR_SHARING_VIOLATION`）ため、
「`Mutex` 取得 → `BufWriter::flush` → 内側 `File` を drop → `rename` → 新規 `File` を開いて
差し替え」という順序を守る。

この2点は**非同期化（`non_blocking`）を伴わずに同期のまま実現できる**。
`tracing-appender::non_blocking` の既定 `lossy=true` はチャネル溢れ時に**サイレントに
ログを破棄する**挙動であり、これは journal 側が `DumpTruncated`
（`journal.rs:398-407`、レーン別ドロップ件数を明示記録する設計）で徹底している
「ドロップは可視化する」という設計思想と逆行する。しかも最もログ密度が高くなるのは
まさに BUG-113 のような実機タイミング調査（`--debug` 起動）の場面であり、そこで
サイレントに欠落しては本 ADR の目的（デバッグ能力の向上）と逆効果になる。

**したがって非同期化（`non_blocking`）の採否は本 ADR では確定しない。** 採用するとすれば
以下を先に設計すること: (a) `lossy` 方針（ブロックして正確性を優先するか、破棄件数を
`BugReportStateSnapshot` に載せて可視化するか）、(b) `awase-settings/src/main.rs:192`
の `log_checkpoint`（クラッシュ直前ログ保全という既存の目的）を壊さない代替、
(c) `WorkerGuard` の drop タイミングとプロセス終了シーケンスの整合。

決定2で確定するのはあくまで**同期のままの2点（BufWriter・ローテーション）**であり、
これには以下を条件として付す:

- `--debug`（stderr）経路は同期のまま維持する。
- **フラッシュ方針を明記する**: `BufWriter` はクラッシュ時にバッファ内容（＝直近の、
  不具合報告に添付される最も価値の高い行）を失いうる。`warn!`/`error!` レベルの
  イベント到達時、および不具合報告起動（`launch_bug_report` 呼び出し）直前に
  明示的に flush する方針をここで決定する。`LineWriter` への変更は syscall 削減効果を
  消すため採らない。**flush 経路の実装内では tracing マクロを呼ばない**こと
  （`std::sync::Mutex` は再入不可であり、flush 用の writer ハンドルと同じ Mutex を
  ログ出力側も取得するため、flush の内側でログを出すと自己デッドロックする）。
  不具合報告起動直前の flush は **awase.exe プロセス内**（`runtime/message_handlers.rs`
  の `launch_bug_report` 呼び出し直前）で行う — 実際に `awase.log` を読むのは
  `--applog` 引数で起動される別プロセスの awase-settings.exe 側であり、awase.exe が
  保持する writer ハンドルにはそちらから触れないため、flush は必ず awase.exe 側で
  完了させておく必要がある。
- WARN 以上での自動 flush は、`tracing_subscriber::Layer` ではなく
  **`MakeWriter::make_writer_for(&Metadata)`** を実装したカスタム `MakeWriter`
  として実装する（`fmt::Layer` が保持する `MakeWriter` は外部の `Layer` から
  アクセスできないため、`Layer` 経由の実装は成立しない）。
- サブスクライバの設置は必ず **`tracing_subscriber::util::SubscriberInitExt::init()`
  （または `try_init()`）経由**で行い、`tracing::subscriber::set_global_default()` を
  直接呼ばないこと。`tracing-log`（`tracing-subscriber` の default feature）による
  外部 crate（egui/winit/zbus 等）の `log` 出力の自動収集（`LogTracer`）は
  `SubscriberInitExt::{init,try_init}` の内部でのみ設置され、
  `set_global_default()` を直接使うと無言で失われる（`awase-settings/src/main.rs:150`
  の doc が記録している「`env_logger` 未初期化で `log::warn!` が no-op になっていた」
  事故の再演になる）。`tracing-subscriber` の依存指定に `default-features = false` を
  付けないこと。
- `hook_channel.rs:183-199` の不変条件は維持することを明記し、`architecture_guard.rs`
  に「`hook_callback` 本体にログ/tracing マクロ呼び出しが増えていない」ことを保証する
  テキスト走査テストを追加する（この移行で invariant が緩む方向のリスクを構造的に防ぐ）。

### 決定3: 相関情報をログ引数から span フィールドへ昇格する（対象ファイルを表と全数照合）

`ApplyGeneration`（`generation`）、`AppKind`、対象 `HWND`、VK 等、現在は呼び出し元が
毎回 `format!` 文字列に手で埋め込んでいる相関情報を、hot-path 関数の
`#[instrument(skip(...), fields(...))]` に昇格する。

対象は `.claude/rules/fix-requires-evidence.md` の再発ファミリー表と**全数照合済み**
（Opus 2体が独立に照合し、1件の食い違い〈`executor.rs::applied_snapshot`〉のみ発見・
統合済み）。確定リスト:

- `ime_controller.rs::apply`（trait メソッド。実体は**4つの実装**
  `ImmCrossProcessStrategy`/`GjiDirectStrategy`/`MsImeDirectStrategy`/
  `KanjiToggleStrategy`、`ime_controller.rs:73/150/219/301` — `#[instrument]` は
  trait 定義側に付けられないため**4箇所全てに付与する**。「1箇所付ければ全経路を
  カバーできる」という誤読は、`fix-requires-evidence.md` の「IME actuation 合流点」行が
  警告する issue #136 の失敗〈gate を1箇所にしか置かず二重 actuation を作った〉と
  同型なので明記する）
- `runtime/open_chain.rs::run_open_chain_async` / `fallback_write` / `imm_cross_write`
- `runtime/executor.rs::dispatch_ime_set_open` / `execute_one` /
  **`applied_snapshot` を書くメソッド**（`execute_from_hook`：宣言`:171`/書込`:179`、
  `execute_from_loop`：宣言`:202`/書込`:208`。`applied_snapshot` 自体は関数ではなく
  フィールド〈宣言 `:101`、消費 `:590`〉なので instrument 単位はこれを書く関数側）
- `runtime/conv_actuation.rs`, `output/conv_actuation.rs`
- `runtime/transport.rs::plan`（`self` を取らない関連関数）
- `output/tsf_warmup_coord.rs`, `output/probe_io.rs`, `output/ime_apply_planner.rs`
- `state/ime_model.rs`, `state/observation_store.rs`, `runtime/ime_coordinator.rs`
- `focus/classifier.rs` **および** `focus/classify.rs`（両方実在する別ファイル。
  当初は前者のみを挙げていた）
- `focus/uia.rs`, `focus/msaa.rs`, `runtime/focus_tracking.rs`
- `state/conv_mode.rs`, `ime.rs`
- `output/vk_send.rs`
- `platform.rs`（特に `build_ime_control_view`）
- `runtime/ime_refresh.rs::ir_apply_drift_correction`
- `runtime/key_pipeline.rs`（idle-conv-check の DirectInput 回復経路）
- `tsf/` の probe/observer/output 各層（`tsf/probe.rs`, `tsf/observer.rs`,
  `tsf/output.rs` を含む）

**`#[instrument]` は既定で `self` を含む全引数を `Debug` で記録するため、
`skip(...)` で個別に大きい引数だけを列挙する方式は使わない。全対象に
`skip_all` を付け、相関性の高いプリミティブ値だけを `fields(...)` で明示する。**
理由: 上記対象には `Debug` を derive していない型が複数含まれる
（4戦略の裸のユニット構造体 `ImmCrossProcessStrategy` 等、`ImeControlView`
〈`Clone, Copy` のみ〉、`impl Into<..>` 引数）。`skip(view)` のように個別列挙すると
`self` 等の記録漏れが残りコンパイルエラーになる。`skip_all` ならこの問題が
構造的に発生しない。`runtime/open_chain.rs` の `async fn` は `#[instrument]` を
付けると `Instrumented<Future>` を返すが、`winmsg-executor::spawn_local` はただの
`Future` を取るため問題ない。

**高頻度経路（打鍵ごとに通る `runtime/transport.rs::plan`, `output/vk_send.rs`,
`runtime/executor.rs::execute_from_hook`/`execute_from_loop` 等）は
`#[instrument(level = "debug", skip_all, ...)]` とし、`info!` 相当の既定レベルを
使わない。** `#[instrument]` の既定レベルは INFO で、`app/bootstrap.rs` の既定
フィルタも `"info"` のため、既定のまま付与すると通常起動でも打鍵ごとにエンジン
スレッドで span 生成が走り、`.claude/rules/tuning-constants.md` が守る ms 単位の
予算を無用に消費する。

`tuning.rs` は定数のみでインストルメントすべき関数が無いため対象外とする。

**Phase 2 完了条件として、以下の3者の関係を機械的に保証するガードテストを追加する**:
`fix-requires-evidence.md` の再発ファミリー表、本決定3のファイルリスト、
`.githooks/pre-push` の正規表現。ただし3者は**完全な集合一致にはならない**
（`tuning.rs` は表/正規表現には現れるが決定3では明示的に対象外、正規表現は
ディレクトリ/パスのプレフィックス単位でありファイル単位の決定3リストとは粒度が
異なる）。テストは「**決定3のファイルリスト ⊆ (表 ∪ 正規表現が示すパス集合)**」
という包含関係を assert し、`tuning.rs` 等の意図的な例外はテストコード内に
コメント付き定数として明記する。この包含関係は**一方向のみ**（決定3が対象範囲を
逸脱していないかは検知できるが、表/正規表現が守るべきファイルを決定3が
instrument し忘れていないかは検知できない）ことをテストの doc コメントに明記する。

### 決定4: journal 基盤との統合方式を確定する（Option C: journal → tracing の一方向 fan-out、設置場所は `UnifiedJournal::absorb`）

**Option B（journal を `tracing_subscriber::Layer` として再実装し、`tracing` →
journal の方向で統合する）は不採用と確定する。** 理由は以下の3点（いずれも実コードで
検証済み）:

1. **`journal_policy.rs` の判定は `Layer::enabled` では表現できない。** 例えば
   `literal_detect_is_notable`（`journal_policy.rs:50-62`）は `LiteralVerdict` の
   値に応じた `match` で採否を決めるが、`tracing::Layer::enabled()` は `&Metadata`
   （target/level/name/callsite）しか受け取れず、フィールド値を見られない。値が届く
   `on_event` では `dyn Value` に型消去済みで、`match` の網羅性検査を失う。
2. **抑制カウンタが `Layer` に持てない。** `platform.rs:216-231` は述語が false のとき
   単に捨てず、抑制回数を溜めて次の採用レコードに `suppressed_confirms` として添付する
   状態を持つサンプリングであり、`Layer` へ移すには可変状態と `PlatformState` への
   参照が要る。
3. **決定打: `with_app` 再入で、記録したい最重要イベントほど消える。** journal は
   `Runtime`（`RUNTIME: SingleThreadCell`, `lib.rs:198`）の中にあり、`push_journal_entry`
   や `journal.record()` は `&mut self` を要求する。`Layer::on_event` からこれに触るには
   `with_app`（`lib.rs:207-217`、thread-local `RefCell::try_borrow_mut`）を通すしかない。
   `runtime/` 配下のログ呼び出しはほぼ全て `with_app(|app| ...)` の中から出るため、その中で
   `tracing::info!` を叩けば `on_event → with_app → try_borrow_mut` 失敗 → `None`
   （さらに `lib.rs:210` が二次的に `warn!` を出す）。無限再帰はしないが、**入れ子で
   発生したイベントが黙って捨てられる**。これは `state/platform_state.rs:168` の
   `JournalEntry::ImeEvent`（ADR-082 決定1 の主役、IME belief 系の中心）を含む経路で
   実際に発生し、決定論的リプレイの正確性要件を根本から壊す。

なお `journal_replay.rs` が読むのは `tests/journals/*.json` に**手で転記・レビューした**
fixture（`journal_replay.rs:22-24,35-40`）であり、発生源を `tracing` に一本化しても
この人手の転記工程は消えない。Option B の便益はほぼゼロである。

**Option A（現状維持、統合しない）も不十分である。** journal 記録は約49箇所、`log::` は
`awase-windows/src` だけで736箇所と規模もカーディナリティも違うため、両者は本来
統合対象ではない。本当の重複はもっと狭く、`platform.rs:169-186` のように**同じ事象を
`log::warn!` と `push_journal_entry` の両方で書いている個別箇所**である。

**採用: Option C — journal を SSOT のまま変えず、journal → tracing の一方向 fan-out を
`UnifiedJournal::absorb`（`journal.rs:643`）1箇所に追加する。**

journal への入口は2系統ある: (1) `platform.rs::push_journal_entry`（10箇所、
`pending_journal_entries` に積んでから `drain_journal_entries` 経由で `absorb` へ）、
(2) `journal.record()` の直接呼び出し（`state/platform_state.rs:168` の
`JournalEntry::ImeEvent` を含む13箇所以上）。`record()`（`journal.rs:635-639`）は
内部で `self.absorb(envelope)` を呼ぶため、**両経路が最終的に `absorb` で合流する**。
（当初 `push_journal_entry` への設置を提案したが、これでは経路2、特に
`JournalEntry::ImeEvent` が漏れることが判明したため訂正した。この種の「合流点を
数えずに1箇所へ置く」失敗は `fix-requires-evidence.md` が挙げる ADR-119 の事例と
同型であり、本 ADR 自身がその再発例になりかけた。）

```rust
// journal.rs — enum ごとの判別子文字列は journal.rs 内に閉じた private fn として
// 実装する（型自体に as_str() を生やさない。ConvClassifyCall 等は core crate
// (awase::engine::InputModeState 等) の型を含むため、型側への実装だと ADR-019 の
// 依存追加議論に踏み込む。journal.rs 内で完結させることでそれを回避する）。
// 文字列値は journal の JSON シリアライズ（serde、variant 名そのまま）と揃え、
// tracing 側と journal 側で語彙が食い違わないようにする。
fn literal_verdict_str(v: LiteralVerdict) -> &'static str {
    match v {
        LiteralVerdict::SuspectedLiteral => "SuspectedLiteral",
        LiteralVerdict::StaleConfirm => "StaleConfirm",
        // ... 残りの variant も同様（_ => を書かない。網羅性維持が目的）
    }
}

// JournalEntry に構造化 tracing イベントとして吐くメソッドを追加
impl JournalEntry {
    /// 呼ぶのは JournalEnvelope::emit_tracing の内側だけ(1箇所)。
    /// 既定レベルは debug!。info! に格上げするのは行わない（後述、頻度の理由）。
    fn emit_tracing(&self, seq: u64, elapsed_ms: u64) {
        match self {
            Self::TsfProbeStarted { source, cold_seq, probe_id, gji_state,
                                    consecutive_at_start, pending_deferred_len } => {
                tracing::debug!(target: "awase::journal",
                    seq, elapsed_ms, source, cold_seq, ?probe_id, gji_state,
                    consecutive_at_start, pending_deferred_len,
                    "tsf probe started");
            }
            Self::LiteralDetect { record, suppressed_confirms, since_vk_sent_ms } => {
                tracing::debug!(target: "awase::journal",
                    seq, elapsed_ms,
                    verdict = literal_verdict_str(record.facts.verdict), // Debugでなく判別子文字列
                    suppressed_confirms, since_vk_sent_ms, "literal detect");
            }
            // ... 残りの variant も同様。_ => は書かない（網羅性がこの機構の安全装置）。
        }
    }
}

impl JournalEnvelope {
    fn emit_tracing(&self) {
        self.entry.emit_tracing(self.seq, self.elapsed_ms);
    }
}

// journal.rs:643 — absorb の先頭で呼ぶ。lane push より前。
pub fn absorb(&mut self, envelope: JournalEnvelope) {
    envelope.emit_tracing();
    let lane = envelope.entry.lane_kind();
    match lane { /* 既存の push 処理、無変更 */ }
}
```

この設置場所と設計が Option B の3つの成立不能理由を全て回避する:

- 述語は**呼ぶ前に**既に評価済み（`platform.rs:216` 等）なので `Layer::enabled` に
  フィールド値が渡らない問題が発生しない。
- 抑制カウンタは `&mut self` を持つ既存コードの中に留まる。
- `emit_tracing` が呼ばれるのは**既に `&mut Runtime`（`UnifiedJournal`）を借りている
  中**であり、`on_event` 側は journal どころか `RUNTIME` に一切触れないため
  `with_app` 再入が起きない。この設置場所を選んだことで、「journal 由来の tracing
  subscriber は awase の状態（`RUNTIME`）に一切アクセスしない」という不変条件が
  構造的に満たされる。

**この設計を実装する上での必須条件（6点、タスク分解レビューで3点追補）:**

1. **`seq`/`elapsed_ms` を明示フィールドとして必ず出す。** `push_journal_entry` は
   発生時点で `stamper.stamp()`（`journal.rs:569-577`）して `seq`/`elapsed_ms` を
   確定させるが、`absorb` は drain 時まで遅れて呼ばれることがある。tracing の
   イベントタイムスタンプは（drain 時刻ではなく）`JournalEnvelope` が保持する
   `seq`/`elapsed_ms` から読み取れるようにする。
2. **`emit_tracing` の実装内で `?`/`%` シギル（Debug/Display フォーマット）を禁止し、
   enum は `&'static str` 判別子を明示的に持たせる。** `?record` のような書き方は
   `tracing::field::Value` が任意の型付き値を運べないために発生する Debug 文字列化
   であり、ADR-082 決定1（`description: String` の自由文字列を廃止し型として取り出せる
   形にした）を実質的に巻き戻す。判別子文字列は enum の型自体に `as_str()` を生やす
   のではなく、**`journal.rs` 内に閉じた private fn**として実装する（`ConvClassifyCall`
   等が保持する `awase::engine::InputModeState` のような core crate の型に手を入れず、
   ADR-019 の依存追加議論を避けるため）。この文字列値は journal の JSON シリアライズ
   （serde、variant 名そのまま）と**同じ表記に揃える** — tracing 側で独自の
   snake_case 等を作ると、`log_excerpt`（JSON）を見る人と `app_log_excerpt`
   （awase.log）を見る人とで語彙が食い違い、不具合報告の triage を混乱させる。
   `architecture_guard.rs` に `journal.rs` の `emit_tracing` 実装を対象とした
   テキスト走査テストを追加し、`?`/`%` の出現を機械的に禁止する。
3. **`emit_tracing` の `match` に `_ =>`/`..` のワイルドカードアームを書かない。**
   ワイルドカードを使うと、将来 `JournalEntry` に variant が追加されたときに
   コンパイルエラーで検知できなくなり、この機構の唯一の安全装置（網羅性検査）が
   失われる。上記の `?`/`%` 禁止と同じテキスト走査テストに、ワイルドカードアームの
   禁止も同居させる。
4. **`emit_tracing` は既定で `tracing::debug!` を使い、`info!` への格上げは行わない。**
   journal への入口となる `dispatch_event`（`state/platform_state.rs:167-170`）は
   `JournalEntry::ImeEvent` を無条件・無制限に記録しており、しかもその多くは
   ポーリング由来の観測（`InputModeObserved`/`ObserverReported` 等）でティックごとに
   発火する。`KeyInput`（打鍵ごと）よりさらに高頻度になりうる。variant やレーン
   ごとに「これは低頻度だから `info!` でよい」と個別判断すると、新しい呼び出し元が
   増えるたびに前提が崩れる。**全 variant を一律 `debug!` にし、既定フィルタ
   （`"info"`）のもとでは journal 由来の tracing 出力が一切出ない**ようにすることで、
   決定2（ログ肥大防止）と構造的に矛盾しなくする。詳細を見たい場合は
   `RUST_LOG=awase::journal=debug` 等で明示的に有効化する。
5. **journal のレーン容量超過で捨てられるエントリも tracing には出ることを明記する。**
   `emit_tracing` を `absorb` の**先頭**（lane push より前）に置くため、
   `JournalLane::push`（`journal.rs:481-495`）が容量超過で黙って捨てるエントリも
   tracing 側には出力される。これは意図的である（tracing は人間向けの、独自フィルタを
   持つ可能性のあるチャネル、journal はリプレイ用の有界リング、という役割分担）が、
   書いておかないと後日「不整合だ」として誤って揃えられる恐れがあるため明記する。
6. **深くネストした variant は当面トップレベルの主要フィールドのみ出力してよい。**
   `JournalEntry` は実測19 variant（当初「約24種」としたのは過大）で、うち
   `ImeEvent { event: state::ime_event::ImeEvent }` は内側の `ImeEvent` 自体が
   約17 variant を持つ深いネストである。全ての内側 variant を今回のPRで再帰的に
   `&'static str` へ展開することは求めない（`event_kind` のような大枠の判別子と、
   既存の primitive フィールドのみで初版とし、完全展開は将来のフォローアップとする）。
   ただし `match` 自体の網羅性（条件3）は維持すること。

これにより、`platform.rs:169` のような重複した `log::warn!` は削除できる（同じ情報が
構造化フィールド付きで journal 側から自動的に出るため）。Phase の位置づけは
Phase 2（`#[instrument]` 導入と同時期）とし、Phase 1（機械置換）とは分ける。

### 決定5: `metrics` crate は導入しない。観測カウンタは `BugReportStateSnapshot` の拡張で対応する

当初「アプリ内完結の `metrics` レコーダー」を提案したが、**これは既に存在する**。
`bug_report.rs:111-163` の `BugReportStateSnapshot` が、lifetime counter
（`wake_post_failed_lifetime_count` 等）・high-water gauge（`hook_ring_max_occupancy`）・
gauge（`working_set_bytes` 等）・最新値（`send_health_last_elapsed_ms` 等）・state
（`desired_open` 等）を既に保持しており、**`schema_version`（`bug_report.rs:167`）で
版管理された型付きの契約**として `docs/bug-reports-triage.md` と
`bug-report-fetch`/`bug-report-latest` Skill という消費者を既に持つ。

これを `metrics::counter!("awase.hook.wake_post_failed")` のような文字列キー方式に
置き換えると、(a) フィールド名のタイポがコンパイルエラーでなく黙った欠損になる、
(b) `schema_version` の契約が消える、(c) 既存 triage Skill とドキュメントが壊れる —
明確な後退である。

**決定: 新しい観測カウンタ（ADR-120 Phase 0a の仲裁カウンタ等）は `metrics` crate
ではなく、`BugReportStateSnapshot` に新規フィールドを追加し `schema_version` を
上げる、という既存の方法で行う。** `metrics` crate facade は導入しない。

**決定5-1（awase-settings への観測カウンタパネル）は本 ADR から削除する。**
`awase-settings.exe` は `tray.rs:837` の `Command::new(&exe).spawn()` で起動される
**別プロセス**であり、`tray.rs:130-133`「awase-settings は awase-windows に依存しない
ため定数を直書きで参照している」という設計と整合する形で、既存の IPC は
`awase-settings/src/main.rs:5211-5221` の `FindWindowW` + `PostMessageW` による
**settings → awase.exe の片方向のみ**である。awase.exe 側が保持する
`BugReportStateSnapshot` を settings 側の UI に流す戻りチャネルは存在しない。
実現するには共有メモリ・`WM_COPYDATA` の戻り便・中間ファイル等の**新規 IPC 設計**が
必要であり、これは observability 移行という本 ADR のスコープを超える。将来
本当に必要になった場合は「awase.exe → awase-settings の診断チャネル新設」という
独立した ADR を起こすこととし、本 ADR では扱わない。

**`feature = "dev-metrics-http"`（開発時ローカル Prometheus エクスポート）は却下する**
（却下した代替案を参照）。

### 決定6: `tracing-etw` は採用しない。発火条件付きで将来検討に留める

Chrome cold-start のリテラル化（BUG-02系）や BUG-113 のような ms 単位のタイミング
競合バグの調査には ETW（Event Tracing for Windows）が有効に見えるが、
**awase は既に µs 分解能の単調クロック**（`hook.rs:1211-1222` の `now_timestamp_us`、
`quanta::Clock` ベース）と **seq 順序付きの journal**（`journal.rs:569-577`）を
持っている。BUG-02/BUG-113 の調査で実際に足りなかったのはタイムスタンプの分解能でも
カーネルイベントとの相関でもなく、**相手プロセス（GJI/MS-IME）の内部状態**であり、
ETW はそれを見せてくれない。

したがって決定6は「Phase 5 で spike する」という予定ではなく、**「ETW が答えを
出せる問い（＝カーネル側の事象が原因だと具体的に疑える症状）を実際に1件観測して
から spike を起こす」という発火条件付きの保留**とする。発火条件が満たされるまで
着手しない。

## 却下した代替案

- **OpenTelemetry SDK フルスタック導入**: 分散トレーシング前提の重量級構成であり、
  単一プロセスのデスクトップアプリには過剰。
- **`tracing-log` の恒久併用**: 決定1参照。移行期間中の一時利用のみ許容する。
- **metrics の Prometheus 常時エクスポート、および `dev-metrics-http`**:
  エンドユーザー環境・開発機のどちらでも常時 HTTP サーバーを立てるのは受益者不在の
  リスク。同じ情報は決定5で採用した `BugReportStateSnapshot` の拡張で得られる。
- **`metrics` crate facade の導入そのもの**: 決定5参照。文字列キー方式は
  `BugReportStateSnapshot` の型付き・版管理された契約からの後退であり、既存の
  triage Skill/ドキュメントとの整合を壊す。
- **決定4 Option B（journal を `tracing::Layer` として再実装し、tracing→journal
  方向で統合する）**: `Layer::enabled` にフィールド値が渡らない、状態を持つ
  抑制カウンタが `Layer` に持てない、`with_app` 再入で最重要イベント
  （`JournalEntry::ImeEvent`）が黙って drop される、の3点で技術的に成立しない。
  この却下理由は `.claude/rules/experiment-logging.md` の趣旨に従い、再導入を
  防ぐためにここへ残す。
- **決定4 Option C の設置場所として `push_journal_entry` を使う案**: journal への
  入口がもう1系統（`journal.record()` の直接呼び出し）あり、そちらが
  `JournalEntry::ImeEvent` を含むため漏れる。正しい設置場所は両経路の合流点である
  `UnifiedJournal::absorb` である（決定4参照）。
- **決定5-1 を新規 IPC 無しで実現する案**: `awase-settings.exe` は別プロセスであり、
  既存 IPC は片方向（settings→awase.exe）のみ。新規 IPC 設計は本 ADR のスコープ外。

## 未解決の疑問

- ~~Phase 1 の機械置換がバイナリサイズ・起動時間に与える影響は未計測~~ →
  **実測済み（2026-09-06）**。`cargo xwin build --target x86_64-pc-windows-msvc
  --release -p awase-windows`（lld-link 使用）で `awase.exe` を移行前後で
  ビルドし比較: 移行前（`log`/`env_logger`）4,881,920 バイト → 移行後
  （`tracing`/`tracing-subscriber`）4,792,832 バイト（**-89,088 バイト、
  -1.82%**）。懸念された「`tracing` サブスクライバースタックはコンパイル時
  展開が大きい」という増加は観測されず、むしろわずかに縮小した
  （`env_logger`/`log` の削除分が `tracing-subscriber` 等の追加分を上回った
  とみられる）。起動時間は実機（Windows）でしか計測できないため未計測のまま
  だが、バイナリサイズの観点では Phase 1 完了の判断を妨げる要因はない。
- 決定2で保留にした非同期化（`non_blocking` writer）の採否・`lossy` 方針・ドロップ
  可視化の設計は未確定。必要になった時点で別途決定する。
- `#[cfg(windows)]` 配下にある `#[cfg(test)]` ユニットテスト（`runtime/` 等）は
  Linux ネイティブテストバイナリに存在しない（CLAUDE.md 参照）。tracing subscriber の
  初期化コード自体がテストで到達可能かどうかは Windows CI 側でしか検証できない。
- clippy の `cognitive_complexity` が影響しないという結論は clippy 0.1.97 での実測に
  基づく。CI が固定しているツールチェーンのバージョンが変わった場合、再検証が必要。
- 決定4の `emit_tracing`（24種程度の `JournalEntry` variant 分の match アーム）の
  保守コストは、呼び出し元49箇所への分散ではなく `journal.rs` 内の1関数に閉じるため
  低いと評価しているが、実装時に実際の行数・レビュー負荷を確認する。

## 関連ファイル

- `Cargo.toml`（root, awase core）
- `crates/awase-windows/Cargo.toml`, `crates/awase-linux/Cargo.toml`,
  `crates/awase-macos/Cargo.toml`, `crates/awase-settings/Cargo.toml`,
  `crates/awase-gji-config/Cargo.toml`, `crates/win32-async/Cargo.toml`
- `crates/awase-windows/src/hook.rs`, `hook_channel.rs`（フックコールバックの
  不変条件、決定2）
- `crates/awase-windows/src/app/bootstrap.rs`（`env_logger` 初期化、`awase.log`
  writer、決定2）
- `crates/awase-windows/src/journal.rs`, `journal_policy.rs`,
  `tests/journal_replay.rs`（決定4）
- `crates/awase-windows/src/platform.rs`（`push_journal_entry`, `build_ime_control_view`,
  決定3・4）
- `crates/awase-windows/src/lib.rs`（`RUNTIME: SingleThreadCell`, `with_app`、決定4）
- `crates/awase-windows/src/bug_report.rs`（`BugReportStateSnapshot`、決定5）
- `crates/awase-windows/src/tray.rs`（awase-settings のプロセス分離とIPC、決定5-1却下の根拠）
- `crates/awase-windows/src/keymap.rs`（実行時 `log::Level` 分岐、決定1）
- `crates/awase-windows/tests/architecture_guard.rs`（`:1406` のマーカー依存、
  決定1・2・3・4で追加するガードテスト）
- `.githooks/pre-push`（再発ファミリー正規表現、決定1・3）
- `.claude/rules/fix-requires-evidence.md`（決定3の対象ファイル一覧の根拠）
- `.claude/rules/tuning-constants.md`, `.claude/rules/experiment-logging.md`
- `docs/adr/019-platform-independence.md`（core crate 依存追加の可否）
- `docs/adr/082-*.md`（journal のドメイン型記録、決定4の前提）
- `docs/adr/120-retroactive-ngram-correction.md`（決定5で置換対象となる手書きカウンタの例）
- `docs/adr/125-*.md`（`awase.log` 747MB 肥大化の実測記録、決定2の根拠）
