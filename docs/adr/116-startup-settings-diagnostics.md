# ADR-116: 起動時設定診断（awase / awase-settings 共通）

## ステータス

**採用・実装済み（2026-08-31、r5）。Opus 2体（architect/premortem_reviewer）
による設計の敵対的レビュー3ラウンド、実装完了後にOpus 2体
（codereview-a/codereview-b）による独立コードレビュー1ラウンドを実施。
r1→r2でarchitect/premortemの指摘（過剰設計・既存コードとの重複・事実誤認）を
反映、r2→r3でpremortemの再レビュー（`mem::take`順序バグ・US誤警告の再混入・
レイアウトタブ保存の再計算漏れ）を反映、r3→r4でarchitectの実装レビュー
（存在しない関数名の誤記、`mem::take`書き戻しが決定3の方針と両立しない
構造的欠陥、「再スキャン」ボタンの再計算漏れ、行番号ズレ）を反映、
r4→r5で実装後の独立コードレビュー2体（lint警告がセル単位でトレイバルーン/
診断リストを埋め尽くす問題、`layouts_dir`不在時の無言フォールバック、
新規ロジックのテスト不足、パネルの高さ無制限）を反映し実装・テストまで
完了した。**

**前提条件**: 本ADRは BUG-104（`docs/known-bugs.md`、`bootstrap::warn_layout_fallback`）
がdevelopにマージ済みであることを前提とする（PR #131、`52b6d9c7`でマージ済み・
本ADR作成時点で充足済み）。

## r1からの主な変更（レビュー指摘の反映）

r1は独自の `Diagnostic` 型・`diagnose()` 関数・`awase-settings` 側の新設バナーを
提案していたが、Opus 2体のレビューで以下が判明し方針を変更した:

1. **既存コードとの重複**: `LayoutEntry::scan_all`
   （`crates/awase-windows/src/app/bootstrap.rs:699-761`）が既に
   `layouts_dir` 内の全 `.yab` の読込/パース失敗を検出・通知している。
   r1の「新規診断関数」はこの半分を再発明していた。
2. **`yab::lint()` はパース成否と独立に動かす**: r1は「パース成功時に
   `yab::lint()` を実行」としていたが、`yab::lint()` はパース成否と独立に
   動く設計（`src/yab/mod.rs:203-205`）であり、パースを前提にする理由が
   ない。「read + lint のみ」に揃えた（決定1）。
   **なお、同梱 JIS 用レイアウト（13列）を `keyboard_model = "us"`
   （12列上限、`src/scanmap.rs:49-50`）環境で開くと `YabLayout::parse`
   自体が失敗し、`scan_all` が「レイアウト読込失敗」として警告するのは
   r1由来の新規欠陥ではなく、本ADRと無関係に**現時点で既に存在する
   既存挙動**である（architectレビューで訂正、r3→r4）。決定1（lintを
   parseと独立に呼ぶ）はこの既存の誤警告を1件も減らさない。決定2で
   reload時の通知が届くようになる分、この既存誤警告の露出はむしろ
   増える。決定3では、この誤警告を新しい常設UIへ複製しないよう、項目2を
   「読込失敗のみ（`YabLayout::parse` は呼ばない）」に絞ることで対処する
   （後述）。keyboard_model 不一致そのものへの対処（警告文言を分ける等）
   は本ADRのスコープ外とする（決定4）。
3. **`awase-settings` 側の新設バナーは不要**: `config_path_panel`
   （`crates/awase-settings/src/main.rs:605-654`）が既に常設の
   `TopBottomPanel::top` として存在し、`ConfigLoadState::Dangerous` の
   警告もここに表示している。ここに診断リストを追加するだけでよい。
4. **`reload_config` の事実誤認**: 「既存の `StartupDiagnostics` に合流」と
   書いていたが、`crates/awase-windows/src/app/mod.rs:615-618` は
   `config.validate()` の警告を `log::warn!` するだけで、
   `StartupDiagnostics`（＝ユーザーに見える経路）には一切入っていなかった。
   さらに `reload_config` は独立した3つの `StartupDiagnostics` インスタンス
   （ngram用・keys用・layout用）を作り、リロード1回でトレイバルーンが
   最大3回出る既存の問題があった。

r2はこれらを踏まえ、**新しい抽象を導入せず、既存の走査点・既存のUIに
最小限追加する**方針に変更した。

## 背景

BUG-104の調査で、「`default_layout` に指定した `.yab` が UTF-8 でない等の
理由で読込に失敗しても、ユーザーに気づかれずソート順先頭のバンドル版へ
無言でフォールバックしていた」問題が見つかった。この調査を報告した際、
ユーザーから次の依頼があった:

> この件に限らず、ユーザーの設定が正しいか診断して、問題があれば警告する機能を
> 追加してほしい。

現状の設定検証は以下の**4箇所**に分散しており（r1は前半2つしか把握していな
かった）、いずれも網羅的でもタイムリーでもない:

1. **`AppConfig::validate()`**（`src/config.rs`）: 閾値レンジ・親指キー重複・
   `keys.ime_on` と親指キーの衝突（BUG-93）等、`config.toml` 自体の内容検証。
   `awase.exe` 起動時は `StartupDiagnostics` 経由でトレイバルーン
   「N件の警告があります」として通知する（`bootstrap.rs:890`）。
   **`reload_config`（設定リロード時）はこの警告を `log::warn!` するだけで、
   ユーザーには一切届いていない**（`app/mod.rs:615-618`、起動時との非対称）。
   `awase-settings.exe` は「適用」ボタン押下時にしか呼ばない
   （`apply_confirmed`、`main.rs:462`）——起動直後や、保存せず眺めて
   いるだけの間は一切診断結果が出ない。
2. **`LayoutEntry::scan_all`**（`bootstrap.rs:699-761`）: `layouts_dir` 内の
   **全 `.yab`** について読込（UTF-8デコード含む）失敗・パース失敗を検出し、
   `StartupDiagnostics` に警告として積む。**`awase.exe` の起動時・
   リロード時は両方ともこれを呼んでいる**（`bootstrap.rs:213`、
   `app/mod.rs:676-679`）。**`awase-settings.exe` は呼んでいない**
   （ファイル名一覧だけを返す `scan_layout_names` はあるが、内容の
   読込/パースは行わない）。
3. **`awase::yab::lint()`**（BUG-95、`src/yab/mod.rs`）: `.yab` のクォート
   崩れセルを検出。`awase-settings` のレイアウト読込/保存時にのみ動く
   （`layout_status` に表示、`load_yab_layout`/保存パス）。**配列編集タブを
   開くまで実行されない**（`ensure_layout_loaded`、`main.rs:941-943`、
   `layout_loaded` フィールドで遅延させている）。`awase.exe` 側はどこからも
   呼んでいない。また `default_layout` 以外のレイアウトファイルは
   `awase-settings` でも配列編集タブを開かない限りチェックされない。
4. **BUG-104 のモーダル**（`bootstrap::warn_layout_fallback`）は
   `default_layout` が実際に読み込まれたかどうかだけを見る、個別の
   ピンポイント対応。汎用の「設定診断」ではない。

まとめると、実際に欠けているのは次の2点だけである:

- **(a) `awase.exe` 側**: `scan_all` が拾ったファイル内容に対して
  `yab::lint()` を呼んでいない。`reload_config` の `config.validate()`
  警告がユーザーに届いていない。`reload_config` がトレイバルーンを
  最大3回出す。
- **(b) `awase-settings.exe` 側**: 起動直後に `config.validate()` も
  `scan_all` 相当の走査も `yab::lint()` も一切行われず、`config_path_panel`
  に何も出ない。

## 決定

### 決定1: `awase.exe` — `LayoutEntry::scan_all` に `yab::lint()` を追加する

新しい型・新しい関数は作らない。`LayoutEntry::scan_all`
（`bootstrap.rs:742-759`）の既存ループで、`std::fs::read_to_string` が
成功した直後（`YabLayout::parse` の成否とは独立に）、`awase::yab::lint(&content)`
を呼び、結果を同じ `diag: &mut StartupDiagnostics` へファイル名付きで
`diag.warn()` する。

```rust
match std::fs::read_to_string(&path) {
    Ok(content) => {
        for msg in awase::yab::lint(&content) {
            diag.warn(format!("{}: {msg}", path.display()));
        }
        match YabLayout::parse(&content, model) {
            // 既存のまま
        }
    }
    Err(e) => { /* 既存のまま */ }
}
```

**「パース成功時」を条件にしない**（r1の欠陥の再導入防止）。`lint()` は
入力テキストへの1パス（`.lines()` × `.split(',')` × `YabValue::parse`）で
コストはほぼゼロ（実測: 同梱4ファイル計8,188バイトで99.6µs、100ファイル
相当で2.10ms）。`read_to_string` は既存コードが元々行っているため、
`awase.exe` 側の追加コストは実質「lint関数の呼び出し」だけで、追加の
ファイルI/Oは発生しない。

`scan_all` は起動時（`init_engine_validated`）・リロード時
（`reload_config`）の両方から呼ばれているため、この変更だけで両方に
波及する。

### 決定2: `awase.exe` — `reload_config` の通知欠落と多重バルーンを修正する

1. `config.validate()` の警告（`config_warnings`）を `log::warn!` だけで
   終わらせず、`StartupDiagnostics::warn()` にも積んでユーザーに見える
   ようにする（起動時の `init_engine_validated` と同じ扱いに揃える）。
2. `reload_config` 内の3つの独立した `StartupDiagnostics` インスタンス
   （ngram用・keys用・layout用）を1つに統合し、`report()` の呼び出しを
   最後に1回にする。現状は設定リロード1回につきトレイバルーンが
   最大3回（「N件の警告があります」）出る既存のバグで、決定1で
   `lint()` の警告を追加すると4回目のバルーンを生みかねないため、
   本決定と同時に直す。

これにより `reload_config` は「1回のリロードにつき、集約された警告が
最大1回のトレイバルーンで通知される」という、起動時と対称な挙動になる。

### 決定3: `awase-settings.exe` — 既存の `config_path_panel` に診断リストを追加する

新しい常設パネルは作らない。`config_path_panel`
（`main.rs:605-654`）に、`Dangerous` 警告ブロックの下へ折りたたみ可能な
診断リスト（0件なら非表示）を追加する。

**診断内容**（`SettingsApp` に新フィールド `startup_diagnostics: Vec<String>`
を持たせ、専用メソッド `recompute_diagnostics()` で計算する）:

1. `config.validate()` の警告。
2. `layouts_dir` 内の全 `.yab` の**読込失敗のみ**（`std::fs::read_to_string`
   がエラーを返すケース。UTF-8 デコード失敗を含む）。
3. 読込に成功した全 `.yab` の `yab::lint()`。

**項目2・3は決定1と全く同じ「read + lint のみ、`YabLayout::parse` は呼ば
ない」原則に揃える**（r2→r3変更）。r2では「`YabLayout::parse` のエラー
メッセージを直接集める」としていたが、これは「背景」で説明した
keyboard_model 不一致による既存の誤警告（同梱 JIS 用ファイルはいずれも
1行目13列、`KeyboardModel::Us` の12列上限で `bail!` する）を、
`awase-settings` 側の新しい常設パネルへ**新規に複製**してしまう欠陥だった
（premortem r2レビュー指摘）。`awase.exe` の `scan_all` は実際に
`YabLayout` を読み込む必要があるため引き続きパース失敗を検出するが、それは
「ロードできるか」という別の関心事であり、`awase-settings` の診断リストが
それを再現する必要はない。

**`config.validate()` と `layouts_dir` 走査の順序**（r4で書き直し、
architectレビュー指摘）: `recompute_diagnostics` は次の順序で実装する。

```rust
fn recompute_diagnostics(&mut self) {
    if matches!(self.config_load_state, awase::config::ConfigLoadState::Dangerous(_)) {
        self.startup_diagnostics.clear();
        return;
    }
    let layouts_dir = resolve_layouts_dir(&self.config.general.layouts_dir);
    let mut diagnostics = scan_yab_files_for_diagnostics(&layouts_dir);

    // apply_confirmed の mem::take + 書き戻しパターンを**あえて使わない**
    // （r3では使っていたが、architectレビューで構造的な欠陥と判明:
    // 書き戻すと validate_layouts の正規化(".." 含みパスを "layout" へ
    // 書き換え)が self.config に確定してしまい、ユーザーがまだ「適用」を
    // 押していないのに画面表示上の layouts_dir が黙って変わる。しかも
    // 2回目以降の recompute では「検証前の生の値」がメモリ上に存在しなく
    // なるため、r3の「layouts_dirを先に確定させれば揃う」という理由付け
    // 自体が初回しか成立しなかった）。ここは警告文を覗くだけなので
    // clone で十分——AppConfig は小さく、この関数はユーザー操作のたび
    // （毎フレームではない）にしか呼ばれない。
    let (_, warnings) = self.config.clone().validate();
    diagnostics.extend(warnings);

    self.startup_diagnostics = diagnostics;
}
```

`resolve_layouts_dir`（`crates/awase-settings/src/main.rs`、既存の
`scan_layout_names`/`ensure_layout_loaded` が使っているのと同じ関数）を
使うことで、`layouts_dir` の解決を GUI の他の部分と揃える。`self.config`
を clone → validate → 破棄する（書き戻さない）ことで、`recompute_diagnostics`
が副作用として `self.config` を変えないことを保証する——`layouts_dir`
が「検証前の生の値」であり続ける、という決定3の前提もこれで初めて
成立する（mem::take + 書き戻し版では初回しか成立しなかった）。

**再計算タイミング**（r4で1件追加）:

- `SettingsApp::new()`（初回）
- `poll_pending_save` の `PendingSaveResult::Saved` 分岐
  （`main.rs:530-537`、保存成功で設定が変わった直後）
- `cancel()`（`main.rs:578-589`、既存の `scan_layout_names` 呼び出しの隣に
  追加。ファイルから再読込した内容を診断する）
- `layout_write_to_path` の成功分岐（`main.rs:855-865`）: 配列編集タブで
  `.yab` を保存すると `layout_status` に lint結果が出るが、上部パネルの
  診断リストは古いままだと二重表示が矛盾する（タブ下は「警告なし」、
  パネル上は「クォート崩れあり」等）。
- 「再スキャン」ボタン（`main.rs:1225`付近、`tab_layout` 内。r4追加、
  architectレビュー指摘）: `scan_layout_names` を呼び直して配列選択肢
  一覧を更新する既存ボタンで、診断リストだけ再計算から漏れていた。

**`ConfigLoadState::Dangerous` のときはスキップする**。`Dangerous` は
`config.toml` の読込自体に失敗し `default_config()` を表示している状態
（`SettingsApp::new()`、`main.rs:372-379`）であり、その既定設定を診断
しても無意味な結果が、本当に重要な「読込失敗」という赤字警告の隣に
並ぶだけになる。

**同期ファイルI/Oについて（premortem r2レビュー指摘）**: 項目2・3は
`layouts_dir` に対する新規の `read_dir` + 複数 `read_to_string` を
UI スレッド（egui）で同期実行する。`apply_confirmed` の保存処理が
バックグラウンドスレッドへ逃がされている理由（AV/OneDrive 等のロックで
最大200ms再描画不能だった実測、`main.rs:434-446`）と同種の懸念が
再発しうる。ただし `scan_layout_names`（`cancel()`/`new()` から既に
同期呼び出しされている既存関数）も同じ `layouts_dir` に対して
`read_dir` を行っており、本決定はその延長（`read_to_string` を追加する
だけ）にとどまる。同梱4ファイル計8KB程度なら実測上ミリ秒未満（lint単体で
99.6µs、ローカルディスクの`read_to_string`はこれより数桁遅くなることは
通常ない）。`layouts_dir` がネットワーク越し等で実際にUIの応答性を
損なう報告が来たら、`apply_confirmed` と同じくバックグラウンドスレッドへ
逃がすことを検討する（決定4と同じく実害待ち。今回はスコープ外）。

**正規化警告が消せない件**（premortem r2レビュー指摘）: `config.validate()`
の警告には `confirm_mode "speculative" は廃止されました`
（`src/config.rs:749-757`）のような、GUIに対応する表示欄が無い正規化警告が
含まれる。これらは「適用」を押すまで診断リストに残り続けるが、実害は
軽微（1行残るだけ）と判断し、対応不要とする（決定4と同じ扱い）。

### 決定4: 対象外（今回は設計しない）

- config.toml 内のキーコンボ重複検出の強化（`[[keymap]]` と `keys.*` の
  横断チェック等、既存 `validate()` の範囲を超えるもの）
- `awase-gji-config`（GJI 設定ファイル）側の整合性チェック
- 診断結果のユーザー向けアクション（「今すぐこのタブを開く」ディープリンク）
- BUG-104 のモーダル・`launch_settings` 自動起動の挙動そのものの変更
  （本ADRは決定1〜3の追加通知がBUG-104の通知と表示回数として競合しない
  ことまでは確認していない。起動時に問題があるレイアウトが1つでもあると、
  最悪 モーダル→設定画面自動起動→トレイバルーン→（設定画面が開けば）
  診断リストの4経路で似た情報が出うる。実害が報告されるまでは許容し、
  対応不要と判断する——通知が煩雑すぎると分かった場合に統合を検討する）
- 「名前を付けて保存」（`layout_do_save_as_dialog`）で `layouts_dir` の外
  （デスクトップ等）へ保存した場合、`layout_write_to_path` 成功時の
  `recompute_diagnostics()` は常に `self.config.general.layouts_dir` を
  見るため、保存先の診断は更新されない（codereview実装レビュー指摘）。
  `layouts_dir` 外へ保存したファイルは `default_layout` の候補にもならない
  ため実害は小さいと判断し対象外とする。
- Ctrl+S のキーリピートで `layout_do_save` が連射されると、
  `recompute_diagnostics` 経由の `layouts_dir` 全走査も同じ頻度で走る
  （codereview実装レビュー指摘）。`layouts_dir` がローカルディスクなら
  実用上問題ないはずだが、UNC/OneDrive 等では悪化しうる。Ctrl+S 自体の
  デバウンスは本ADRのスコープ外とし、実害が報告されたら別途対応する。

## 未解決の疑問（r1の2件はレビューで解決済みのため削除）

なし。

## 関連

- BUG-104（`docs/known-bugs.md`）: `default_layout` 読込失敗のモーダル通知。
  本ADRの前提。
- BUG-95（`docs/known-bugs.md`）: `yab::lint()` の元になった調査。
- ADR-095: タスクトレイ不具合報告機能（`StartupDiagnostics` の既存利用例）。
- PR #127: `apply_confirmed` の `mem::take` + 書き戻しパターンの前例
  （`recompute_diagnostics` は書き戻しを必要としないため、あえて
  `clone()` を使う——r4参照）。
