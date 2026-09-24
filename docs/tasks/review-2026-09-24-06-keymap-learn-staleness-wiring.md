---
title: 学習表の陳腐化検出（キーマップ指紋の書き込みと staleness::check の実行時配線）
status: 実装済み（PR、2026-09-24。指紋書き込み・staleness配線・NotSupported失効・採用/再検証ガード。既存表は(b-2)で保護しない）
created: 2026-09-24
related_adr: ["ADR-195", "ADR-196", "ADR-191"]
source_review: 俯瞰レビュー（受動化・actuation撤去・学習/較正・config棚卸し・v2方針、2026-09-24）の B-1（要旨は本文「背景」に引用）
---

# 陳腐化検出の実行時配線（俯瞰レビュー B-1）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。裏取り基準は `5877f982`（origin/develop、PR #296 まで）。既存タスク [adr195-t8-staleness-detection.md](adr195-t8-staleness-detection.md)（実装対象1・2は ADR-196 3a 追記で「即時失効のまま有効」）と同件。

## 実装メモ（2026-09-24、`feat/keymap-learn-staleness-wiring`）

- 主ツリー未コミット差分の特性化テスト（`b10_...`・B-3）は origin/develop に**存在しない**（`git grep`で0件）。B-10 のテストは本実装で `fresh_when_fingerprint_not_supported` を `stale_when_stored_fingerprint_meets_not_supported` へ書き換えた。主ツリー側のテストは取り込まれていないので、その担当が本ブランチのマージ後に `b10_...` の期待値を更新するか破棄すること。
- (b) は **(b-2)**（スキーマ版は上げず、指紋`None`の既存表は保護しない）を採用。(c) プリセット名の照合は指紋（`session_keymap`値を含む）に寄せた。01 の案Aとの重複整理は 01 側で。
- `--adopt-pending-judgement`: 指紋`None`の`NeedsConfirmation`は昇格させない（`AdoptRejected::NoFingerprint`）。既に`Accepted`の再実行は冪等成功のまま。
- 学習本体: `Unavailable`は`Rejected(FingerprintUnavailable)`。指紋方式の無いTIP（`Other`）は`NotSupported`で従来どおり書く（実行時もその構成では予測しない）。
- 未実装: ソース走査ガード、awase-settings画面の「キーマップ変更で失効」表示（02）、実機確認。

## 背景（元レビュー B-1 の要旨）

元レビューはセッションの作業領域にしか無いため、要旨をここに残す:
「`staleness::check` を本番で呼ぶ箇所が0件。学習プロセスも `with_fingerprint` を呼ばず、書き手も読み手もキーマップの指紋を扱っていない。ADR196-T5 の『指紋書き込み配線』は `env_version`（IMEの版）のことで、キーマップ設定の指紋ではない。表ファイルは学習時の IME 種別もプリセットも持たない。ADR-195 の status は段階8をマージ済みと書くが、T8 タスクは実行時配線が未反映と書いており食い違う。」

> **注記（並行作業）**: メイン作業ツリー `/home/cuzic/rust-nicola` に未コミット差分がある（2026-09-24 に `git diff` で確認）。中身はテストの追加だけ:
> - `staleness.rs`: `b10_stored_fingerprint_vs_not_supported_is_currently_fresh`（`(Some(fp), NotSupported)` → `Fresh` を現状として固定する特性化テスト。本文に「方針を決めて直す際は、このテストの期待値を更新すること」とある）。
> - `verify.rs`: `classify_robust_default_k2_misses_a_true_50_50_cell_with_fewer_than_4_obs_per_ctx`（B-3）。
>
> 本タスクで `check` の `NotSupported` 分岐を変えると、前者と**意図的に衝突する**。着手前にこの差分がコミット済みかを確認すること。

## 現状（`5877f982` で裏取り済み）

- **`staleness::check`（`crates/awase-keymap-learn/src/staleness.rs:72`）の呼び出し元は0件（確認済み）**。`staleness` の参照は `awase-keymap-learn/src/lib.rs:36`（`pub mod`）と `revalidation.rs` の doc コメントだけ。`awase-windows` の `probe_actuation_fence.rs` / `lifetime_counter.rs` は一般名詞として "staleness" を書いているだけ。`develop-weekly-code-review-2026-09-23.md` B-10 の再確認とも一致。
- **ただしスキーマ版の失効は既に効いている**。`key_effect_runtime.rs::read_persisted_table`（`:456`）→ `persist::from_json` → `RejectReason::SchemaVersionMismatch` の経路。未配線なのは**キーマップの指紋（と IME 種別の照合）だけ**。
- 学習プロセスは `with_fingerprint` を呼ばない（`crates/awase-keymap-learn-win/src` に `fingerprint` の出現0件。本番外の呼び出しは `persist.rs:199` と `awase-windows/src/bug_report.rs:1162` のテストだけ）。`persist.rs:55-57` の doc は「`fingerprint` は当面 `None` のまま運用」と書く。
- **`persist.rs:99` の doc は誤り**。`with_fingerprint` に「ADR196-T5 が実配線」とあるが、T5 が配線したのは `env_version`（`main.rs:292-293` の `with_env_version`）。`adr196-t5-revalidation-not-invalidation.md:3` / `:107` の「指紋書き込み配線」も同じ意味で紛らわしい。
- 表ファイルに IME 種別もプリセットも無い。`PersistedTable` のフィールドは `schema_version / fingerprint / env_version / cells / verification / judgement`（`persist.rs:59-80`）。`fingerprint` は既に `Option` + `#[serde(default)]` で存在するので、**書き込むだけならスキーマ版を上げる必要は無い**。
- 表の書き手は3つ（`crates/awase-keymap-learn-win/src/main.rs`）: 学習本体（`persist_judged_table`、`:285-295`）、`--adopt-pending-judgement`（`adopt_pending_judgement_at`、`:370-384`。`judgement` だけ書き換える）、`--revalidate`（`revalidate_table`、`:443-483`、`revalidation.rs:164` の `apply_revalidation`。`verification`/`env_version`/`judgement` だけ書き換える）。後の2つは `fingerprint` をそのまま残す（コードで確認）が、**IME 種別・指紋を照合しない**。
- 読込側の `RuntimeTableCache::get`（`key_effect_runtime.rs:537-570`）は、表ファイルのスタンプ（mtime+長さ）か `(preset, check_against_bundled)` が変わったときだけ `load()` を呼び直す。
- ADR-195 の status は「段階0/1/2/3/4/5/6/8は全てdevelopへマージ済み」のまま（status 4行目）。T8 タスクと食い違う（[10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) で同期）。

## 失敗シナリオ

1. GJI のカスタムキーマップ A で学習した後、GJI 設定で無変換を IME OFF に割り当て直す（キーマップ B、プリセットは Custom のまま）。表ファイルのスタンプも `(preset, check_against_bundled)` も変わらないので、`RuntimeTableCache` は**読み直しすらしない**。学習表 A で予測を続け、belief が実 IME とずれ続ける（ADR-196 3a 追記が即時失効を求めるケース）。
2. GJI で学習した後、再割り当てありの Microsoft IME 本体へ切り替える。`preset` が `MsImeNative` に変わるので表は**読み直される**。ただ `is_unmodified_bundled_config()` が偽で `check_against_bundled=false` になり、照合する項目が無いまま素通りして、GJI の表を MS-IME の予測に使う。原因は「キャッシュが古い」ではなく「読み直しても検証する項目が無い」こと。

## 方針（設計の代案を取り込み済み、ADR 改訂で確定）

- **指紋はキーマップの生の入力から作る**（ADR-196:185 の「セッション/カスタム/オーバーレイキーマップ3値のハッシュ」と一致）。
  - GJI: `awase_gji_config::wire::parse_top_level` の `session_keymap` / `custom_keymap_table` / `overlay_keymaps`（値そのもの）。
  - MS-IME 本体: `read_raw_key_assignment_dwords` の3つの DWORD（`IsKeyAssignmentEnabled`/`KeyAssignmentHenkan`/`KeyAssignmentMuhenkan`）。既存の `native_assignment_stamp`（`msime_key_assignment.rs:261`）がこの3値を詰めた内容値なので、そのまま使える。
  - これに「GJI か MS-IME 本体か」の区別を加えて、安定したハッシュにする（`std` の `DefaultHasher` は使わない）。
  - 計算はキーマップを読む箇所（`read_key_effect_keymap` / `read_key_effect_keymap_native`）で同時に行い、`KeymapCache` がキーマップと指紋を組で持つ。こうすれば awase.exe で追加のファイル読込は要らず、キーマップが `Some` なのに指紋だけ取れない状態も生じない。
  - 不採用にした案（その1）: `KeyEffectKeymap` の5値（`preset` / `custom_table` / `has_overlay` / `henkan_reassigned` / `muhenkan_reassigned`）から作る。`from_config`（`key_effect_predictor.rs:529-546`）は overlay の中身を `has_overlay` の真偽値に潰し、`for_msime_native`（`:553-566`）は再割り当て値を「0以外か」の真偽値に潰す。そのため overlay を別の overlay へ替える変更や、MS-IME の無変換を「IME-オフ」から「IME-オン/オフ」へ替える変更（0以外→別の0以外）を見逃す。真偽値は予測を止めるかの判定には足りるが、学習表が記録する「その構成での実挙動」を識別するには足りない（前回レビュー代案12の欠陥、再確認レビュー N1）。
  - 不採用にした案（その2）: 既存の `ConfigFingerprint`（`gji_charset_autodetect.rs:123-137` の `current_fingerprint`、`msime_key_assignment.rs:278` の `current_registry_fingerprint_hash`）を流用する。理由は3つ: VK ごとの指紋で表全体の指紋ではない／MS-IME 側が `DefaultHasher`（`:281`）で版を越えて値が保証されない／GJI 側が overlay を含まない。したがって [07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の (2)（`ConfigFingerprint` の撤去）は 06 を待たずに進めてよい。
  - 不採用にした案（その3）: `config1_db_stamp`（mtime+len）を指紋にする。GJI がキーマップ以外の設定を保存しても mtime が変わると推測され、数十分かかる再学習を頻繁に強いることになる（推測の根拠は**未確認**、実機で見ていない）。
- **`preset` を指紋に含めれば、IME 種別・プリセットの別フィールドは要らない**。`KeymapPreset::MsImeNative` は MS-IME 本体専用（`key_effect_predictor.rs:35-46`）なので、シナリオ2も GJI 内のプリセット切替も指紋の不一致として検出できる。`table_ime_kind()` との照合という2本目の判定源も要らなくなる。なお `KeymapPreset`/`ImeKindId` は `awase-windows` の型で、OS 非依存の `awase-keymap-learn::persist` には置けない（`persist.rs:7-8` の依存方向）。指紋は不透明な `Fingerprint` のまま保存する。
- **B-10（`(Some, NotSupported)` → `Fresh`）の見送り理由はもう成り立たない**。理由(2)は「IME 切替は T5 の `needs_revalidation` が担う」だったが、`needs_revalidation` を呼ぶのは `awase-settings/src/keymap_learn_status.rs:99` だけで、awase.exe は呼ばない。`EnvVersion`（`revalidation.rs:34-40`）は IME 種別を持たず、MS-IME 本体の版取得は ADR-197 待ち。IME 切替を検出する仕組みは現状どこにも無い。上の方式なら GJI・MS-IME 本体の両方で指紋が取れるので `NotSupported` が出ない。それ以外の IME では予測自体をしない（`key_pipeline.rs:1967` の `ActiveImeKind::MicrosoftIme => return`、キーマップが `None` なら `:1969-1971` で return）。ただし保険として `(Some, NotSupported)` は B-10 推奨どおり `FingerprintNotSupported`（stale）を新設する。

## タスク

- [ ] **ADR 改訂で決める（実装の前）**: (a) 指紋の計算方法（上の方針）。(b) 既存の表の扱いを次の二択から選ぶ。`fingerprint: None` のままの既存表は `check` の `(None, _) → Fresh` で**永久に保護されない**。
  - (b-1) スキーマ版を上げて、全ユーザーに再学習させる（数十分）。
  - (b-2) 版は上げず、`None` の表は保護しない。代わりに設定画面で再学習を促す。
  [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) の案A（学習側で内蔵表と突き合わせ済みかの記録＋突き合わせたプリセット名を `#[serde(default)]` で追加、スキーマ版は上げない）と同じ ADR 改訂で決める。01 は版を上げないので、(b-1) を選ぶなら版上げは 06 の都合だけで行うことになり、01 の新フィールド追加と同じリリースにまとめる（版上げは1回）。(c) プリセット名の照合を 01 の記録と 06 の指紋のどちらに寄せるか（06 の指紋は preset を含むので、指紋の照合が先に棄却するなら 01 側の照合は冗長になる）。`opus-adversarial-consult` で収束させる（プロジェクトの慣行）。
- [ ] 指紋の計算を2つに分ける: (1) 生の値（GJI の3値／MS-IME の3 DWORD）→ `Fingerprint` の純粋関数（cfg 無し、ホストでテストする）、(2) 生の値の読み取り（`read_raw_key_assignment_dwords` は `#[cfg(windows)] mod windows_impl`〈`msime_key_assignment.rs:135-136`〉の中にある）。これらを `awase-windows` 側に `pub` で置き、学習プロセスと awase.exe の両方で使う。学習プロセスは既に `awase_windows` にリンクしている（`main.rs:456` の `read_config1_db` 呼び出し）。一方で `config1_db_stamp`（`gji_charset_autodetect.rs:319`）、`read_key_effect_keymap`（`:334`）、`read_key_effect_keymap_native`・`native_assignment_stamp`（`msime_key_assignment.rs:249` / `:261`）は `pub(crate)` なので、公開範囲を変えるか、公開の窓口関数を足す必要がある（`read_raw_key_assignment_dwords`（`msime_key_assignment.rs:235`）も `pub(crate)`）。
- [ ] 学習本体（`persist_judged_table`）で `with_fingerprint` を書く。指紋は学習開始時に読んだ設定から作る（GJI は既存の `config1_db_at_start`〈`main.rs:455-456`〉と同じ内容。学習中の設定変更は終了時の `config1_db_at_end` 比較〈`:528-529`〉が `gji_config_changed` でセッション失敗にする）。**指紋 `None` の表が `Accepted` で本体に入る経路を、書き手と昇格の両方で塞ぐ**（`check` の `(None, _) → Fresh` で永久に保護されないため）:
  - 書き手: 指紋を計算できない（`config1.db` が読めない・解析できない等）ときは、判定を必ず `Rejected(FingerprintUnavailable)`（`RejectedReason` に新設）にする。「`NeedsConfirmation` のまま退避ファイルへ書く」は採らない。`adopt_pending_judgement_at`（`main.rs:370-392`）は退避ファイルを読み、`adopt_needs_confirmation`（`judgement.rs:260-270`）で `NeedsConfirmation`/`Accepted` を `Accepted` に書き換えて `table_path` へ書き、`fingerprint` を見ないため、設定画面の「採用」だけで指紋 `None` の表が本体に昇格してしまう。`Rejected` なら `adopt_needs_confirmation` が拒む（`:267`）。
  - 昇格: `--adopt-pending-judgement` も「`fingerprint: None` の表は `Accepted` へ昇格しない」を判定する（06 以前の学習プロセスが書いた既存の退避ファイルは指紋 `None` の `NeedsConfirmation` なので、書き手の修正だけでは塞がらない）。判定は `awase-keymap-learn::judgement` の純粋関数に置く（`adopt_needs_confirmation` に表の指紋の有無を渡す等）。`table_path` を読む冪等成功の経路（既存の指紋 `None` の `Accepted` 表）をどう扱うかは (b) の結論に合わせる。
  - `RejectedReason` は永続化される enum なので、旧版が新しい値を読んだときの挙動を確認しておく。`Rejected` は通常は退避ファイルに書かれ（`main.rs:297-301`）、`--revalidate` だけは本体の表に書くが、いずれも Rejected は予測に使われない。旧版の不具合報告（`bug_report.rs:439` の `read_persisted_table`）では `Parse` と表示され、旧版の `--adopt-pending-judgement` では `ParseFailed` になる（いずれも昇格はしない）。
  - `--revalidate` は、表の指紋が現在の指紋と違えば `Err` にする（GJI の表を MS-IME の下で「再検証合格」させない）。`--adopt-pending-judgement` と `--revalidate` が `Some` の `fingerprint` を保つことをテストで固定する。
- [ ] awase.exe の読込で `staleness::check` を呼び、`Fresh` 以外は内蔵表へ戻す。判定は `validate_and_convert` 側の純粋関数に入れる（引数に `current: FingerprintProbe` を足す）。こうすればホストでテストできる。
- [ ] `staleness::check` の `(Some, NotSupported)` を `FingerprintNotSupported`（`is_stale()` = true）にする。`fresh_when_fingerprint_not_supported` と主ツリーの `b10_...` テストの期待値を、意図した変更として書き換える。
- [ ] **`RuntimeTableCache` のキャッシュキーに現在の指紋を含める（必須）**。これが無いとシナリオ1で `check` が二度と呼ばれない。なお読み手側の `FingerprintUnavailable` は、指紋をキーマップと同時に計算する方式では生じない（キーマップが取れなければ予測自体をしない、`key_pipeline.rs:1969-1971`）ので、専用の扱いは要らない。`staleness::check` の `Unavailable` の分岐は保険として残す。
- [ ] doc の訂正: `persist.rs:55-57` / `:99` と `adr196-t5-revalidation-not-invalidation.md` の「指紋書き込み配線」を「版（`env_version`）の書き込み」に直す。`staleness.rs` 冒頭の「`config1.db`、レジストリのキー割り当て」も、採った指紋方式に合わせて直す（[10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) の docs 同期に回してもよい）。

## 受け入れ条件

`key_effect_runtime.rs` は belief 予測の入口なので、`.claude/rules/fix-requires-evidence.md` の IME belief ファミリーとして**回帰テストが必須**（`state/key_effect_*` が表・pre-push に未掲載な件は [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) B-8）。

- **ホスト Linux（`cargo test -p awase-windows --lib`、`cargo test -p awase-keymap-learn`）**:
  - `validate_and_convert`（cfg 無し）に次の単体テストを足す: (a) 指紋が違う表 → 棄却、(b) GJI の指紋の表を `MsImeNative` の指紋で読む → 棄却（シナリオ2）、(c) `(Some, NotSupported)` → 棄却、(d) overlay の中身だけが違う GJI の指紋 → 棄却、(e) MS-IME の再割り当て値だけが違う（0以外→別の0以外）指紋 → 棄却。(d)(e) は指紋の計算関数自体の単体テスト（値が変われば指紋が変わる）でもよい。
  - `RuntimeTableCache::get`（cfg 無し）に次の単体テストを足す: (i) preset もスタンプも同じまま指紋だけ変わる → 読み直して棄却（シナリオ1）。
  - `apply_revalidation` / 採用で `Some` の `fingerprint` が保たれることを `awase-keymap-learn` のテストで固定する。
  - 学習本体が指紋を計算できないとき判定が `Rejected(FingerprintUnavailable)` になること（書き手の判定を純粋関数に切り出してテストする。`awase-keymap-learn-win` は `#[cfg(windows)]` なので、判定は `awase-keymap-learn` 側に置く）。
  - 指紋 `None` の `NeedsConfirmation` の表を採用の純粋関数に通しても `Accepted` にならないこと（06 以前の退避ファイルを想定）。
  - `RejectedReason::FingerprintUnavailable` が JSON の往復で保たれること。
  - `journal_replay.rs` は使わない。このテストは `classify_*` の遷移を再生するもので、学習表の読込を通らない（`key_effect` / `PersistedTable` の出現0件）。
- **Windows ターゲットのコンパイル / windows-build CI**: 呼び出し側の `load_and_log`（`key_effect_runtime.rs:400`、`#[cfg(windows)]`）、`key_pipeline.rs:1984`、`runtime/mod.rs:1265`（`runtime/` 全体が `#[cfg(windows)]`）が現在の指紋を渡していることは、Linux では走らない。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` で確認し、実行は windows-build CI に任せる。書き手の `awase-keymap-learn-win`（`main.rs` 先頭が `#[cfg(windows)]`）のテストも同じ扱い。必要なら `architecture_guard.rs` 型のソース走査ガードを足す（`load_and_log` が `staleness` 判定を経由すること）。
- **実機**: GJI でキーマップを書き換えた後、awase.exe のログに失効と内蔵表への切り戻しが出ること。GJI → MS-IME 本体への切替でも同様であること。

## 他ファイルとの依存

- [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md): **06（少なくとも preset を含む指紋の照合）は 01 と同時か、01 より先に入れる**（旧版の「01 が先」から向きを反転）。理由: 今の既定構成（再割り当ての無い MS-IME 本体、GJI のプリセットそのまま）では、`check_against_bundled=true` の5%判定（`MAX_MISMATCH_RATIO = 0.05`）が別 IME・別プリセットで学習した表を棄却しており、事実上の安全網になっている。01 の案Aは「突き合わせ済みで、かつ記録のプリセットが今の preset と一致する表」の5%判定だけを飛ばすので、プリセット名の照合が 01 と 06 で重なる（どちらに寄せるかはタスク (c) で決める）。06 が先に入っていれば preset 不一致の表は指紋の照合で棄却される。なお「5%判定がほぼ確実に棄却する」は表どうしの不一致率を測っていないので**未確認**。スキーマ版は 01 が上げないので、06 が (b-1) を選ぶ場合だけ上げる（タスク (b)）。
- [02](review-2026-09-24-02-settings-status-display.md): 設定画面の状態表示（`keymap_learn_status.rs:97-99` は `env_version` の要再検証だけを出す）に「キーマップ変更で失効」を足す。無いと、awase.exe は内蔵表に戻っているのに画面は「学習結果を使用中」に見える。06 の判定関数ができてから 02。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md): ADR-195/196 の status 同期、上記 doc 訂正。10 は 06 の判断が固まった後。
- [07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md): 06 は `ConfigFingerprint` を流用しない（方針の「不採用にした案（その2）」）。07 のタスク(3)への回答はこれで、07(2) の撤去は 06 を待たない。07 側・[11](review-2026-09-24-11-low-priority-backlog.md) の「06 → 07(2)」の書き換えは各担当で行う。
- ADR-191: `KeymapPreset::MsImeNative` と打鍵時予測の前提。

## 未確認点

- GJI がキーマップと無関係な設定を保存したとき、`config1.db` の mtime と長さが変わるか（`config1_db_stamp` 案を退けた根拠。実機では見ていない）。
- 既定構成で、GJI の学習表と MS-IME 本体の内蔵表の不一致率が実際に5%を超えるか（01 との依存の向きの根拠）。
- 主ツリーの未コミット差分が、着手時点でコミット済みかどうか。

## レビュー反映メモ（2026-09-24、Opus レビュー 19 項目）

- 反映: 1（呼び出し元0件を確認済みに）、2（スキーマ版の失効は既に有効）、3（未コミット差分の中身を記載）、4（`persist.rs:99` の doc 誤り）、5（01 との依存の向きを反転。5%判定の棄却率は未確認と明記）、6（シナリオ2の流れを訂正）、7（journal replay をやめ、Linux / Windows CI / 実機に分けた）、8（キャッシュキーを必須化）、9（書き手3つ。`apply_revalidation` と採用が `fingerprint` を保つことはコードで確認し、レビューの未確認を解消）、10（02 を依存に追加）、11（`pub(crate)` の件）、12・13（指紋方式の代案を方針に採用。別フィールド案は不採用）、14（既存表の扱いの二択）、15（B-10 の見送り理由の失効）、16（B-1 の要旨を引用）、17（ADR-191 を追加）、19（回帰テスト必須を明記）。
- 反映しなかった: 18（本文先頭の「状態:」行）。`review-2026-09-24-*` 系は frontmatter の `status` で揃っており、「状態:」行を grep する運用は確認できなかった。
- 裏取り基準をレビュー時の `cbae84ff` / `d00ac8dd` から `5877f982` に更新した。引用した行番号はすべて `5877f982` で再確認済み（`main.rs` の書き手の行はレビュー時から移動していたので直した）。

## レビュー反映メモ（2026-09-24、Opus 再確認レビュー N1〜N3・m1〜m3）

- 反映: N1（`KeyEffectKeymap` の5値は overlay の中身と MS-IME の再割り当て値を真偽値に潰すことを `key_effect_predictor.rs:529-566` で確認。指紋を生の入力から作る方式に改め、旧案を不採用に移した。受け入れ条件に (d)(e) を追加）、N2（`ConfigFingerprint` を流用しない理由3つを `gji_charset_autodetect.rs:123-137`・`msime_key_assignment.rs:278-281` で確認して方針に追記、依存に 07 を追加）、N3（読み手側の `FingerprintUnavailable` の扱いと (ii) を削除し、書き手側で指紋が取れないとき `Accepted` を書かないタスク・受け入れ条件を追加。前提として指紋をキーマップと同時に計算する旨を方針に明記）、m1（`:1967` / `:1969-1971`）、m2（`revalidate_table` は `:443-483`）。
- 06 側で直さなかった: m3（[01] の 123行・157行の「IME種別の追加」「スキーマ版は1回だけ上げる」）。01 の担当範囲。本文の依存欄にも書いてあるとおり 01 の再確認で拾う。

## レビュー反映メモ（2026-09-24、Opus 2回目の再確認 R1・R2・r1〜r3）

- 反映: R1（`adopt_pending_judgement_at`〈`main.rs:370-392`〉が `fingerprint` を見ずに `adopt_needs_confirmation`〈`judgement.rs:260-270`〉で `NeedsConfirmation` を `Accepted` へ書き換えて本体へ書くことをコードで確認。「退避ファイルへ書く」の選択肢を削り、修正案 (A) 書き手は必ず `Rejected(FingerprintUnavailable)` と (B) 昇格側でも指紋 `None` を拒む、の両方を採った。(A) だけでは 06 以前に書かれた指紋 `None` の退避ファイルが残るため (B) も要る。受け入れ条件を3件追加）、R2（58行・81行の「採用フラグ」前提を 01 の案A〈突き合わせ済みの記録＋プリセット名、スキーマ版は上げない〉に合わせ、プリセット名照合の重複をタスク (c) として1か所にまとめ、依存欄はそこを参照する形にした）、r1（`RejectedReason` は `Serialize`/`Deserialize` 付きの永続 enum。`Rejected` は退避ファイルにしか書かれず〈`main.rs:297-301`〉、旧版で読むのは不具合報告〈`bug_report.rs:439`〉と `--adopt-pending-judgement` だけで、どちらも解析失敗になるだけで昇格しないことを確認して本文に記載）、r2（指紋計算を純粋関数と `#[cfg(windows)]` の読み取りに分けると明記。`msime_key_assignment.rs:135-136` を確認）、r3（書き手の指紋は `config1_db_at_start` と同じ内容から作ると明記。`main.rs:455-456` / `:528-529` を確認）。
