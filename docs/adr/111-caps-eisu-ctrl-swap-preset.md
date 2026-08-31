# ADR-111: Caps(英数)⇔Ctrl 入れ替え専用プリセット（Scancode Map 一本化）

## ステータス

**採用・実装済み（2026-08-31、r4。key_remap撤回はPR #123、Scancode Map実装は
PR #124でdevelopマージ済み。Windows実機ソーク未実施——未解決の疑問2参照）**

## 背景

ADR-110（`key_remap`、PR #120・#121）で「任意の物理キーを別の物理キーとして
恒常的にリマップする」汎用機構を実装した。実装直後、`opus-adversarial-consult`
の round3 レビューで latch ライフサイクルの blocking な穴が3件見つかり
（`docs/known-bugs.md` BUG-100 として修正済み）、hook.rs 側の複雑度・
stuck modifier のリスクが実感された。

その後の会話で、以下の疑問・要望・調査結果が出た:

1. 「汎用的な機能として実装したが、人気がある一部の組み合わせを超簡単に
   できる方がいいのでは」→ **対象を「Caps(英数) ⇔ Left Ctrl」1種類に絞る**
   方針に転換。
2. JISキーボードの「英数」キーとUS/ANSIの「CapsLock」キーは物理的に同一
   スキャンコード（Set 1: `0x3A`）を共有し、レイアウトドライバがShift状態で
   `VK_DBE_ALPHANUMERIC`(0xF0、英数単独)/`VK_CAPITAL`(0x14、Shift+英数)に
   分岐する。
3. **r1（本ADR初版）は Scancode Map 方式と key_remap 方式の両方を実装する
   案だったが、Opus 2体による並列敵対的レビューで key_remap 方式に
   blocking な欠陥が複数指摘された:**
   - `docs/experiments.md` エントリ07/08/09で、`VK_DBE_ALPHANUMERIC`を
     `SendInput`で注入する手法（Ctrl→Eisu方向に相当）は**このリポジトリで
     既に3回失敗・撤去済み**と判明（scan値を変えても awase 自身のフックにすら
     届かない、または CapsLock を物理的に点灯させる）。`key_remap`の
     3ルール構成のうち1つが、既に反証済みの手法の無自覚な再導入だった。
   - Eisu単独押下（Shiftなし）のKeyUpがWin32k内部の IME 関連処理で
     フックに届かない可能性が指摘された（このリポジトリの実データでは未確定
     だが、Windows内部実装の資料からの推論として無視できない）。
   - 両方式を同時有効化した場合の「相殺される」という想定も誤りで、
     Shift+Ctrl（元Eisu位置）のチョードが壊れる経路が指摘された。
4. **PowerToys（Microsoft製、awaseよりずっとリソースのあるOSSプロジェクト）
   のKeyboardManagerを調査した結果、全く同じ「CapsLock→Ctrl + 日本語IME」
   の組み合わせで2020年から問題を抱え、2024年以降の新しいissueでも
   再発が報告され続けていると判明**（[Issue #3397](https://github.com/microsoft/PowerToys/issues/3397)、
   [PR #4123](https://github.com/microsoft/PowerToys/pull/4123)、
   [Issue #32344](https://github.com/microsoft/PowerToys/issues/32344)）。
   根本原因は「Shift+CapsLock/Alt+CapsLock/Ctrl+CapsLockは日本語IMEの
   入力方式切替のグローバルショートカットであり、IME側がフックより前の層で
   キー状態を読むため、フック側でCapsLockを抑制・リマップしても検出されて
   しまう」構造的な問題。
5. 上記を踏まえ、**「Scancode Map方式を主軸にする、key_remap方式は廃止する」**
   と方針決定（r2）。汎用GUIエディタも撤去し、TOML手書き専用に縮小（r3）。
6. さらに検討の結果、「アプリケーションごとに動的にキー割当てを変更する」
   将来機能（本ADRでは未設計）が構想され、**現行の `key_remap`（グローバル・
   静的なリマップのみ）はその将来機能によって置き換えられる可能性が高い**
   という判断に至った。バックエンド（`state/key_remap.rs`・`hook.rs`の
   latch/ダブルバッファ・`config.toml`の`[[key_remap]]`スキーマ）を
   TOML専用機能として残したまま将来機能を設計するより、**一旦完全に
   撤回し、将来のアプリ別動的リマップ機能を設計する際にゼロから
   （必要なら過去の設計・実装を参照しつつ）作り直す方が良い**と判断した
   （r4）。これは ADR-108/109 のような「一時保留」ではなく、
   `[[keymap]]`（BUG-99、別の既存の非機能コンポーネント）とは独立した、
   実装済み・動作確認済みの機能の明示的な revert である。

## スコープ

- **対象を「Caps(英数) ⇔ Left Ctrl の入れ替え」1種類に絞る。実現方式は
  Scancode Map（レジストリ、ドライバレベルのスキャンコード置換）のみ。**
  key_remap（ADR-110の汎用フック機構）はこのプリセットには一切使わない。
- **ADR-110 の `key_remap` 機構をバックエンドごと完全に撤回する**
  （`state/key_remap.rs` 削除、`hook.rs` の latch/ダブルバッファ関連コード
  削除、`config.toml` の `[[key_remap]]` TOML スキーマ削除、`awase-settings`
  側のGUI一式削除）。背景6参照。将来「アプリケーションごとに動的にキー
  割当てを変更する」機能を設計する際に、必要であれば ADR-110/本ADRの過去の
  設計・実装（git履歴）を参照しつつ作り直す。決定7・決定8参照。
- Right Ctrl は対象外（英数キーは物理的に1つしかなく、対応する Ctrl は
  近傍の Left Ctrl が自然）。
- 「権限昇格を許容できないユーザー」への代替手段は本ADRでは提供しない
  （背景3・4より、hookベースの代替は安全に提供できないと判断したため）。
  昇格を拒否するユーザーは、この特定プリセットの恩恵を受けられない
  （却下した代替案・参照）。

## 決定

### 決定1: Eisu/CapsLock キーの物理的正体とスキャンコード

JIS キーボードの「英数」キーと US/ANSI キーボードの「CapsLock」キーは、
**物理的に同一のスキャンコード（Set 1: `0x3A`、CapsLock の位置）を共有し、
レイアウトドライバ（JIS: `kbdjpn.dll` 系）が Shift 状態で異なる VK に翻訳する**
（Web検索で複数のスキャンコード仕様書から `0x3A` を確認、
[Stanford scancodes-9](https://www.scs.stanford.edu/10wi-cs140/pintos/specs/kbd/scancodes-9.html)。
Opus round1レビューで「英数単独→0xF0、Shift+英数→0x14」の分岐自体は
Windows内部実装（`kbd106.c`の`VkToFuncTable_106`）に基づき妥当と確認された
一方、この分岐が発生する正確な段階の記述には検証中の余地が残る——ただし
本ADRはこの分岐をアプリ層（フック）で扱わないため、Scancode Map方式には
影響しない）。

Left Ctrl のスキャンコードは Set 1 標準の `0x1D`（非拡張）。

### 決定2: Scancode Map 方式のバイナリ構成

`HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout\Scancode Map`
（`REG_BINARY`）に書き込む値は、Windows のドキュメント化された固定フォーマット
（4バイトヘッダ×2 + エントリ配列 + null終端）に従う。エントリは
リトルエンディアン `u16` ペア `[to_scancode, from_scancode]` の順（Opus
round1レビューで Windows ドライバソース `ntinput.c::MapScancode` の実装
（`HIWORD`=元、`LOWORD`=変換先）に基づき検証済み。エントリの並び順は
`MapScancode`が線形走査・最初の一致でbreakするため任意でよい）:

```
00 00 00 00                 ; Header: Version (常に 0)
00 00 00 00                 ; Header: Flags (常に 0)
03 00 00 00                 ; エントリ数 + null終端分 = 3
1D 00 3A 00                 ; from=0x003A(英数/CapsLock位置) → to=0x001D(LCtrl)
3A 00 1D 00                 ; from=0x001D(LCtrl) → to=0x003A(英数/CapsLock位置)
00 00 00 00                 ; null終端エントリ
```

決定1の通りスキャンコードレベルの置換なので、JIS レイアウトドライバの
Shift 依存 VK 分岐（英数/CapsLock の二重人格）はこの段階で「物理的に別の
キー（Left Ctrl）」にすり替わった後に評価される。Shift+（旧英数、今はCtrl）
は素直に Shift+Ctrl の通常のチョードとして解釈される。

このバイト列生成・パース（決定3参照）は**純粋関数として実装し、Linuxで
単体テストする**（Opus round1レビューの指摘: 生成側とマージ判定側を
共有できるようにする）。

### 決定3: 既存 Scancode Map 値との共存 — 検出・マージ・保護

`Scancode Map` は awase 以外のツール（PowerToys等）も使う可能性があり、
かつ **マシン全体・全ユーザー・ログオン画面より前に効く値**であるため、
無条件の上書きは事故時の復旧コストが極めて高い（Opus round1レビュー
blocking指摘、B2）。以下を実装する:

1. **有効化時**: 既存値を読み取り、パースする。
   - 空（未設定）→ 決定2の2エントリのみで新規作成。
   - 既存エントリに `from=0x003A` または `from=0x001D` を持つものがあれば、
     それを decision2 の該当エントリで**置き換え**（重複除去）。
   - それ以外の既存エントリ（awase と無関係な remap）は**そのまま保持**して
     追記する。
2. **書き込み後**: 必ず読み戻し、期待した内容（decision2の2エントリを含む）
   になっているか検証する。一致しなければ「書き込みに失敗しました」を表示し、
   config には一切触れない。
3. **無効化時**: 決定2の2エントリ（`from=0x3A`/`from=0x1D`のペア）**のみ**を
   削除し、他のエントリが残っていれば書き戻す。全エントリが空になったら
   値ごと削除する。
4. 上記のパース・マージ・比較ロジックは決定2のバイト列生成と同じ
   モジュールに置き、Linux で単体テストする。

### 決定4: 昇格フロー — awase-settings 自身を `--scancode-map=<on|off>` で自己昇格起動する

`reg.exe` を `ShellExecuteW`+`runas` で起動する方式は、(a) 成否を確実に
取得できない（`ShellExecuteW` は `SEE_MASK_NOCLOSEPROCESS` を渡せずプロセス
ハンドルを取得できない）、(b) レジストリキーパスの空白（`Keyboard Layout`）を
含むコマンドライン引用を自前で組み立てる必要がある、(c) 既存値のマージ
（決定3）が実質不可能、という3つの問題を抱える（Opus round1レビュー
blocking指摘、B3/B4、両レビュアーが独立に到達）。

代わりに、このリポジトリに既にある2つの実績パターンを組み合わせる:

- `restart_as_admin()`（`crates/awase-windows/src/tray.rs:675`、
  `ShellExecuteW`+`runas`で自分自身を昇格再起動する既存実装）と同型の
  ロジックを `awase-settings.exe` に追加し、`--scancode-map=on` /
  `--scancode-map=off` の起動引数を解釈させる。
- 昇格起動した側は `RegGetValueW`/`RegSetKeyValueW`/`RegDeleteKeyValueW`
  （`crates/awase-windows/src/autostart.rs:16` と同型の直接呼び出し）で
  決定3のロジックを実行する。
- **`ShellExecuteExW` + `SEE_MASK_NOCLOSEPROCESS` を使い、起動した昇格
  プロセスのハンドルを取得**。`WaitForSingleObject` で完了を待ち、
  `GetExitCodeProcess` で終了コードを取得する。終了コード `0`=成功、
  それ以外=失敗としてGUIに表示する。UACキャンセル時は
  `ShellExecuteExW` 自体が失敗しエラーコード `ERROR_CANCELLED`(1223) 相当が
  返る（`GetLastError()`で判定）ので、これも区別して表示する。
- 昇格側プロセスは決定3の処理結果（成功/失敗、書き込んだバイト列）を
  終了コードまたは一時ファイル経由で非昇格側に伝える（実装時に確定、
  終了コードのビット幅で足りない場合は一時ファイル方式を優先する）。

`awase-settings.exe` の通常起動自体は既存方針（BUG-79対策、`asInvoker`）を
維持し、昇格が必要なのは決定4の自己再起動フローに限定する。

`crates/awase-settings/Cargo.toml` に `Win32_System_Registry` と
`Win32_UI_Shell` の feature 追加が必要（現状 `Win32_Foundation` 等のみ）。

### 決定5: 反映には再起動が必要（サインアウトでは不十分）

Microsoft公式ドキュメントは Scancode Map の反映に一貫して **reboot** の
みを要求しており、「サインアウト→サインインで足りる」という記述は不正確
（Opus round1レビュー指摘、旧XP/2003時代の per-user 対応の残滓の可能性）。
GUI上には「変更を反映するには**再起動**が必要です」と明記する。高速スタート
アップ（Fast Startup）環境では `shutdown /s` でもレジストリの再読み込みが
発生しない場合があるため、その旨も注記する。

**自動再起動は行わない**（ユーザーの未保存作業を失わせる破壊的操作になり
うるため）。書き込み成功後はメッセージ表示のみに留める。

### 決定6: 適用範囲は「マシン全体・全ユーザー」— 明示する

Microsoft公式ドキュメントは「Scancode Map は接続されている全キーボードに
適用され、キーボード単位のマップは作成できない」と明記している。同居する
他ユーザー（awase を使わないユーザーを含む）にも影響することを GUI上に
明記する。またリモートデスクトップ（Terminal Services）配下では正しく
動作しないとも明記されており、awase の `disable_apps` 既定値が `mstsc.exe`
であることを踏まえ、この既知の制限も注記する。

### 決定7: GUI 変更 — key_remap 関連 UI を全撤去し、Scancode Map専用セクションに置き換え

`crates/awase-settings/src/main.rs` の `key_remap` 関連コードを全て削除する
（決定8のバックエンド revert と対になる）:

- `tab_key_remap`（ルール一覧+追加フォーム本体）
- `KEY_REMAP_KEYS`、`key_remap_key_combo`、`key_remap_row_warning`
- `new_key_remap_from`/`new_key_remap_to` フィールド（`SettingsApp`構造体
  本体・テストフィクスチャ構築箇所の両方）
- `key_remap_key_combo`/`key_remap_row_warning`をimportする既存テスト
  モジュール
- `tab_keymap` 内の `CollapsingHeader::new("物理キー単純リマップ
  （key_remap）")` 呼び出し自体

その跡地（`tab_keymap` 内）に、新しい独立したセクション
「Caps(英数)⇔Ctrl 入れ替え」を追加する:

- 現在の状態表示（未設定 / 有効 / awase以外の設定と混在（決定3参照）/
  読み取りエラー）。タブを開いた時と書き込み直後にのみ
  `RegGetValueW` で読み直す（毎フレーム読まない）。
- 「有効にする」/「無効にする」ボタン（決定3・決定4のフローを起動）。
- 処理結果メッセージ表示欄（決定4の終了コード/エラーに応じて表示）。
- 「変更の反映には再起動が必要です」の注記（決定5）。
- 「この設定はこのPCの全ユーザーに影響します。リモートデスクトップ内では
  動作しません」の注記（決定6）。

`SettingsApp` 構造体には、状態キャッシュ用フィールド（最終読み取り結果・
最終操作結果メッセージ）を追加する（既存の `apply()`→`confirm_*`→
`apply_confirmed()` パターンは今回のフローには直接当てはまらない — この
機能はconfig保存と独立した即時操作のため、別の状態フィールドで管理する）。

### 決定8: `key_remap` バックエンドの revert 範囲と手順

PR #120（ADR-110実装）・PR #121（BUG-100修正）が develop に導入した内容を、
`git revert` で機械的に取り消す（squash merge のため各PRは1コミット、
かつ develop 側でこれらのファイルへの後続変更が無いことを確認済み——
クリーンに revert 可能）。対象:

- `crates/awase-windows/src/state/key_remap.rs`（ファイル削除）
- `crates/awase-windows/src/hook.rs`: `LATCHED_TARGET`/`CACHED_KEY_REMAPS`/
  `CACHED_KEY_REMAPS_ACTIVE_PAGE`、`set_key_remaps`/`cached_key_remaps`/
  `apply_key_remap`/`cleanup_latched_remap_before_bypass`/
  `release_all_latched_remap_targets`/`inject_synthetic_key_up`/
  `key_remap_ctrl_effectively_held`/`normalize_caps_lock_if_needed`、
  `hook_callback` 内の呼び出し・`original_vk`/`vk_before_key_remap` 捕捉・
  Ctrl消費追跡の決定5対応部分
- `crates/awase-windows/src/runtime/mod.rs`・`message_handlers.rs`・
  `focus_tracking.rs`・`key_pipeline.rs`: 上記関数群の呼び出し箇所
- `crates/awase-windows/src/app/bootstrap.rs`: 初期化時の`set_key_remaps`/
  `normalize_caps_lock_if_needed`呼び出し
- `crates/awase-windows/src/vk.rs`: `VK_CAPITAL`/`is_extended_key`等
  key_remap専用に追加した部分（`VK_CAPITAL`自体は他機能が使っていないか
  revert前に要確認）
- `crates/awase-windows/tests/architecture_guard.rs`の該当2テスト
- `src/config.rs`: `KeyRemapRule`構造体、`AppConfig`/`ValidatedConfig`の
  `key_remap`フィールド
- `crates/awase-settings/src/main.rs`: 決定7参照

`git revert`後にコンパイルエラー・テスト失敗が出た箇所（他機能が偶然
`VK_CAPITAL`等の追加定数を使い始めていた場合等）は手動で解消する。

### 決定9: revert の記録— known-bugs.md・experiments.md・ADR-110ステータス更新

`.claude/rules/experiment-logging.md`の規約に従い、revertコミット本文に
観測された失敗条件（何を試し、なぜ撤回するか）を明記する。加えて:

- `docs/known-bugs.md` BUG-100 のエントリに、「本機能（`key_remap`）自体が
  ADR-111 r4決定によりバックエンドごと撤回されたため、本エントリは
  適用対象のコードが存在しない（記録として残す）」旨を追記する。
- `docs/experiments.md` に新規エントリを追加し、「`key_remap`（ADR-110）を
  Caps(英数)⇔Ctrlプリセット向けに縮小しようとしたが、Opus 2体レビュー・
  PowerToys実例調査・`docs/experiments.md`エントリ07/08/09の先例により、
  hookベースでこの特定キーを扱うこと自体が構造的に危険と判断し、
  機能全体を撤回。アプリ別動的リマップという将来機能で置き換える構想」
  を記録する。次に同じ発想（hookベースのCapsLock/Eisu⇔Ctrl入れ替え）が
  再浮上したときに、この経緯を即座に参照できるようにする。
- `docs/adr/110-simple-physical-key-remap.md`のステータスを「撤回
  （2026-08-30、ADR-111 r4決定によりrevert、詳細は同ADR参照）」に更新し、
  `docs/adr/index.md`の該当行も同期する。

## 却下した代替案

- **key_remap 方式の併用**: 背景3・4参照。`docs/experiments.md` エントリ
  07/08/09で実証済みの失敗パターンの再導入になる、PowerToysが同じ組み合わせで
  4年以上苦戦している実例がある、の2点から、hookベースでこの特定キーを
  安全に扱えるという確信が持てないため不採用。
- **昇格を許容しないユーザー向けの代替機能提供**: 上記の理由により、
  hookベースの安全な代替が無いため、今回は提供しない。将来的に
  `docs/known-bugs.md`/`docs/experiments.md` に「この組み合わせはhookベースでは
  対応不可」と明記し、次に同じ発想が出たときに再検討の手間を省く。
- **HKCU への Scancode Map 書き込み**: Windows 7 以降は無視されるため不採用
  （[Microsoft Q&A](https://learn.microsoft.com/en-us/answers/questions/2438286/remap-keyboard-with-scancode-map-for-current-user)。
  ただしOpus round1レビューで、この回答は匿名投稿でありMS公式言明ではない
  こと、一次資料〈`ntinput.c::InitScancodeMap`〉ではXP/2003時代にHKCU→HKLMの
  順で読んでいたことが指摘された。現行OSでの削除を示す一次証拠は見つかって
  いないが、結論〈HKLMを使う〉自体は変えない）。
- **`reg.exe` を `ShellExecuteW`+`runas` で起動**: 決定4参照、成否判定・
  引用符・マージの3点で自己昇格方式に劣るため不採用。
- **自動再起動/サインアウト実行**: 決定5参照、破壊的なため不採用。
- **既存値の無条件上書き**: 決定3参照、マシン全体に影響する値であるため
  不採用。

## 未解決の疑問

1. **決定4の昇格側→非昇格側の結果伝達方式**: 終了コードのビット幅で
   実際に十分か（成功/失敗の2値なら足りるが、エラー種別まで返したい場合は
   一時ファイル等が必要になる）。実装時に確定させる。
2. **実機検証**: JISキーボード実機での Scancode Map 適用後の動作確認
   （再起動後に実際に入れ替わるか、Shift+Ctrl(旧英数)が正常なチョードとして
   機能するか）は本ADRのレビュー段階では未実施。

## 関連ファイル

`crates/awase-windows/src/tray.rs`（`restart_as_admin`参照実装）,
`crates/awase-windows/src/autostart.rs`（レジストリ直接操作の参照実装）,
`crates/awase-settings/src/main.rs`, `crates/awase-settings/Cargo.toml`。
関連: ADR-110（本ADRにより撤回）, BUG-100（本ADRにより適用対象コード消滅、
決定9参照）, `docs/experiments.md`。
