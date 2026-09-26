---
id: ADR-201
title: |-
  設定のキー名解決は `from_name` 1関数に集約して寛容にし、握りつぶしを既存の診断(ADR-116)へ流す。GUI保存は `toml_edit` でコメントを保ち、GUIの候補×読み手と文書例をCIで検証する
summary: |-
  BUG-167（設定GUIが書く `Ctrl+Shift+VK_F12` を `parse_hotkey` が `VK_VK_F12` と解釈して無言で無効）の全体調査（2026-09-26）で同型の不整合が見つかった。
  草案（round0）は「`RawConfig`→`resolve`→`KeySpec`+`ConfigReport` の二層化」を提案したが、opus round1 で次を指摘され取り下げた:
  既存の `validate()`/`ValidatedConfig`/`StartupDiagnostics`（ADR-116）の作り直しになる、`KeySpec{mods,vk}` では `"Left Alt"`（Alt なりすまし）が表せず保存で `VK_NONCONVERT` に化ける、
  コアの `validate()` がキーの意味（かな・F15〜F24・JIS専用）を文字列で判定しており名前表を持たないコアでは書けない、
  費用は 800〜1500 行の追加に対し減るのは 60〜80 行。
  決定: (1) `from_name` を唯一の寛容な解決関数にする（`VK_` 任意・前後空白・ASCII大小文字を問わない、ADR-019 の中立名を別名に）。
  コアの文字列検証は移さず、VK値を持たない文字列の正規化関数 `canonical_key_text` を通した比較に直す。修飾キーの解釈も1関数に集約する。
  (2) 無言の握りつぶし5か所と未知キーを、既存の `StartupDiagnostics`/`validate()` の警告に流す（新しい型は作らない。未知キーは `load_warnings` で渡す）。
  (3) GUI保存・`save_auto_start` は `toml_edit` で、「読み込んだときの値」から GUI が変えた項目だけを、保存の直前にディスクから読み直した文書へ書く（三者比較。既定値は書かない、何も変えなければバイト一致、
  外部エディタでの編集を GUI の古い値で上書きしない）。
  (4) 「GUIの候補×読み手」の受理テスト、文書・サンプルの例、合成した実 config 風の集まりをCIに入れる（実装の最初にやる）。
  (5) 古い表記は読み込みで受け付け続ける。`[[keymap]]` は alias でなく `keymaps` に合流させ、`keymaps` を書くときは `keymap` を消す。書き出す正規形は `VK_*` 名。
  型付け（`KeyName` 列挙型）は将来の別ADRに回す。
status: |-
  **提案（2026-09-26 起草。opus round4 で収束、所有者の最終確認待ち）。** 実装未着手。round4: 収束判定。確定前の3点（保存成功後の `base`、保存直前にディスクが無い/読めない場合、
  既定値が `Some` の `Option` 項目）を反映済み。round1〜4 の指摘はすべて反映されたか、この3点に集約された。
  round1: 決定1（二層化）は延期・取り下げ、決定2〜5は範囲を絞って採用。round2: 重大3件・中3件（決定3の比較基準、`keymap` 合流と配列置換の相互作用、段階1が線引きで止まる件、`load_warnings`、修飾キーの解釈、
  挙動変化の告知）を反映。R2-1 は、レビュアーの推奨（編集用と検証済みの値を分ける）と別の解決（保存直前のディスクとの二者比較）を試したが、round3 で「`main.rs:845-851` が直した stale read-modify-write を復活させる」と指摘され取り下げ、
  三者比較（基準＝読み込んだときの値）に改めた。round3 の中程度（alias の旧名、`Dangerous` の扱い、`canonical_key_text` の順序・組み合わせの区切り・テストの向き）も反映。局所修正は BUG-167（PR #330）で済み。
related_adr:
  - "ADR-019"
  - "ADR-116"
  - "ADR-199"
  - "ADR-114"
---

# ADR-201: 設定のキー名解決の集約と、失敗を黙らせない仕組み

## 背景

### 発端（事実）

不具合報告 `01M31QETZG5K23E8E4SDE5M54Q`（v1.21.0）→ BUG-167: 設定 GUI（`awase-settings` の `format_combo`）が
`engine_toggle_hotkey = "Ctrl+Shift+VK_F12"` を書き出す。読み手 `vk::parse_hotkey` は末尾トークンに `VK_` を無条件で付け、
`VK_VK_F12` となって `from_name` が失敗。`register_toggle` の失敗は `warn!`+`.ok()` で握りつぶされ、ホットキーが無言で無効だった。
局所修正（`vk::with_vk_prefix`、PR #330）は済んでいる。

### 全体調査で見つかった不整合（2026-09-26、3領域の読み取りレビュー + opus round1 の実行確認）

| # | 内容 | 影響 | 確認 |
|---|---|---|---|
| 1 | トグルホットキーに `変換`/`無変換`/`かな`/`漢字` を選ぶと `parse_hotkey` が `VK_変換` にして失敗（GUI候補 `KEYMAP_MAIN_KEYS` の内部名は `VK_` なし）。**今のGUIの出力で読めないのはこの4件だけ**（候補表からの推定、GUIは動かしていない） | ホットキーが無言で無効 | コード読解 |
| 2 | `[[post_bypass]] key = "Ctrl+J"`（同梱 `config.toml`・doc の書き方）は `parse_key_combo` が `VK_` を補わず `None`。`filter_map` 内の `?` で無警告に捨てられる | ルールが無言で消える | 一時テスト実行 |
| 3 | 文書・ADR-037・CHANGELOG は `[[keymap]]`、実際のフィールドは `keymaps`（alias なし）。手書き `[[keymap]]` は0件として読まれ `validate()` の警告も出ない | キー割り当てが無言で無視 | 実行で確認（round1） |
| 4 | 保存は `toml::to_string_pretty(self)` の全体再シリアライズ。コメントと未知キーが消え、**入力に無い既定値（`keys.ime_toggle` など）が書き出されて固定される**。GUI だけでなく、トレイの自動起動切り替え `save_auto_start` も同じ経路 | 同梱 `config.toml` の解説コメントが消える。既定値の固定は ADR-199 T14 の移行と衝突 | 実行で確認（round1） |

**無言のものと、報告されるものを区別する**（round1 F-2。草案は `keys.*` も無言と書いたが誤り）:

- **失敗するが報告される**: `keys.engine_on/off`・`ime_on/off/toggle`（`app/mod.rs:307 parse_key_combos` → `diag.warn` → トレイ通知）、`keys.ime_detect.*`、`engine_off_solo_repeat`。
- **本当に無言**（`tracing::warn!` のみ、または何も出ない）: (a) `[[post_bypass]]`（`bootstrap.rs:601` の `?`）、(b) `[[keymaps]]` の from/to 失敗（`keymap.rs` は `tracing::warn!` のみ）、
  (c) `keys.engine_on_ime_key`/`engine_off_ime_key`（`bootstrap.rs:580-589` の `.and_then(from_name)`）、(d) `muhenkan_solo_tap_dedicated_fn_key`（`runtime/mod.rs:124-135`）、
  (e) `engine_toggle_hotkey` の登録失敗（`bootstrap.rs:478-480` の `.map_err(warn).ok()`）。

別件（本ADRの範囲外）: `.yab` のリテラルの書き出しがエスケープの逆写像になっていない（`serialize` は `'{s}'` と囲むだけ、行分割は `split(',')`）。別のPRで直す。

### 構造的な原因（round1 で数え直した）

1. **名前→VKの表は1つ**（`vk.rs:522 from_name`）。他の「入口」は、その周りの前置き処理が違うラッパー: `parse_hotkey`（`with_vk_prefix`+`from_name`）、`parse_key_combo`（`from_name` そのまま）、
   `warn_on_engine_hotkey_collision`（`with_vk_prefix`+`parse_key_combo`）、`[[keymap]] to`（`from_name(to).or_else(from_name("VK_"+to))`）。
   **表を通らず文字列で比べている箇所**が、より危ない再発源: `state/alt_impersonation.rs:38 resolve_thumb_key`（`"Left Alt"`/`"Right Alt"` の目印）、`runtime/mod.rs:1752`（`left_thumb_key == "VK_SPACE"`）、
   `src/config.rs` の `validate_thumb_keys`（`"Kana"`/`"VK_KANA"` の完全一致。`from_name` は `"かな"` も受ける）・`validate_dedicated_fn_key`（`"VK_F15"` 等との完全一致）・`THUMB_KEY_ALIASES`・`validate_keyboard_model`、
   Linux/macOS の別表（`awase-linux/src/vk.rs`、`awase-macos/src/vk.rs`）。
2. **無言の失敗**: 上の (a)〜(e)。
3. **保存の非可逆**: 上の 4。
4. **例が検証されない**: 文書・サンプルの書き方を読み込みに通すテストがない（2・3）。GUI の候補表から出る値が読み手で受理されるかを確かめるテストもない
   （あれば BUG-167 と #1 は CI で見つかっていた）。

### 既存の仕組み（草案が見落としていた。round1 F-1）

- `AppConfig::validate() -> (ValidatedConfig, Vec<String>)`（`src/config.rs:1260`）: 不正な値を既定値に戻して警告を返す。`ValidatedConfig` と `From<ValidatedConfig> for AppConfig` で「生の値→検証済みの値」の二層が既にある。
- `StartupDiagnostics`（`crates/awase-windows/src/app/mod.rs:78`、ADR-116）: `warn()` でログ、`report()` でトレイ通知（1回）。起動時（`bootstrap.rs`）と `reload_config` で使われ、GUI の警告欄もある。
- ADR-116 は r1 で「診断用の型を新設」する設計を Opus に指摘され、既存の走査点への追加に縮めた前例。

## 制約

- **ADR-019**: コア `awase` クレートは OS 非依存。VK の**値**をコアに置かない（`awase-windows/src/vk.rs` には生の VK 値が多数あり、これは規則の対象外。規則は「コアに置かない」）。
  ADR-019「結果」は config.toml の既定キー名の中立化（`Nonconvert`/`Convert`/`Kanji`）と、`vk_name_to_code` が `VK_` 付き名も受け付けることを挙げている。現状の既定値（`"無変換"`、`"VK_KANJI"` 等）はこの方針からずれている。
- `awase-vkmap` はコアに依存する側で、別リポジトリ（`awaza`）が再利用する純粋な表。名前解決の責務は足さない（本ADRは `awase-vkmap` を変更しないので `awaza` への影響なし）。
- `awase-settings` は `awase-windows` に依存する。`AppConfig` を使うのは `awase-windows`、`awase-settings`、`awase-linux`/`awase-macos`、`awase-keymap-learn-win`、コア（`config.rs` 等）、`tests/scenarios.rs`。
- **互換性**: 既存ユーザーの `config.toml` を壊さない。v2 の方針（互換性を保ちつつ新規ユーザー向けに簡素化）と同じ向き。
- `toml_edit` は、`toml 0.8` の依存として `Cargo.lock` に既にある（0.22.27）。新しい依存としての負担はほぼない。

## 決定

### 決定1: `from_name` を唯一の寛容な解決関数にし、コアの文字列検証は正規化で揃える

- `from_name` を次のとおり寛容にする: 前後の空白を除く、先頭の `VK_` は任意、ASCII 部分は大文字小文字を区別しない、ADR-019 の中立名（`Enter`/`Esc`/`Escape`/`Space`/`Backspace`/`Tab`/`Delete` 等）を別名に足す。
  曖昧さは無いことを確認済み（`"F"`→`VK_F`（英字）、`"F1"`→`VK_F1`。修飾キーは最後以外のトークンだけで解釈されるので、末尾の `Shift`→`VK_SHIFT` とぶつからない）。
- これで `parse_key_combo("Ctrl+J")`・`"Ctrl+F12"`・`dedicated_fn_key = "F18"`・`ime_detect = ["F13"]`・ホットキーの `"変換"` が**すべての入口で一度に**直る。`with_vk_prefix`（`vk.rs`）、`keymap.rs` の前置き処理と予備の解決は消せる。
- **修飾キーの解釈も1つの関数（`vk.rs`）にまとめ**、`parse_hotkey`・`parse_key_combo`・GUI の `parse_combo_str`（`main.rs`、今は `Ctrl|Control|Shift|Alt` の完全一致でそれ以外を黙って捨てる）がそれを使う。
  大文字小文字は `from_name` と同じ規則にする（読み手の片方だけ寛容にすると、手書きの `"ctrl+J"` が実行時には効くのに、GUI で開いて保存すると Ctrl が落ちて `"J"` になる）。
- 表を通らない文字列比較は、解決した VK の比較に置き換える: `runtime/mod.rs:1752`（`left_thumb_key == "VK_SPACE"`）、GUI の `main.rs:2700-2701`（`== "VK_SPACE"`）・`:2723-2724`（`== "VK_RETURN"`）。
  決定1で `"Space"`・`"vk_space"` が実行時に効くようになると、比較のままでは GUI の Space・Enter 用の表示条件だけが外れる。
- **コアの文字列検証は Windows 側へ移さない**（round2 R2-3）。移す費用が大きい: `validate()` の呼び出しが7か所（`awase-windows` 起動時・再読み込み・不具合報告、GUI の警告欄・適用、Linux/macOS）、
  `StartupDiagnostics` は非公開の型で GUI から呼べない、`validate_keyboard_model` は物理配列の話で Linux でも意味がある、コアの該当テストが約18本。
  代わりに、**VK の値を持たない文字列の正規化関数 `canonical_key_text(s)` をコアに置き**（ADR-019 に反しない）、既存の文字列の検証
  （`validate_dedicated_fn_key` の `SAFE_RANGE`、`validate_thumb_keys`、`THUMB_KEY_ALIASES`、`validate_keyboard_model`、`validate_thumb_key_in_ime_combos`）をこれを通した比較に直す。
  - **処理の順序は「前後の空白を除く → ASCII を大文字にする → 先頭の `VK_` を除く」**（round3 R3-2）。先に `VK_` を除くと `"vk_f15"` の `VK_` が大文字でなく残り、`"F15"` と揃わない。
  - **`from_name` は最初に `canonical_key_text` を呼び、正規化した名前で表を引く**。これで両側の規則は定義上同じになり、規則は1か所にしか書かれない。
  - **組み合わせの文字列（`"Ctrl+Shift+変換"`）は、`+` で区切って主キー（最後のトークン）だけに `canonical_key_text` をかけ、完全一致で比べる。`contains` は使わない**（round3 R3-3）。
    `validate_keyboard_model` と `validate_thumb_key_in_ime_combos` は今、組み合わせ全体への `contains`/比較で判定しており、`canonical_key_text` を全体にかけると先頭の `Ctrl` にしか効かず、`contains` は今後の追加で別の名前の一部に誤って一致しうる。
    区切りの規則は、修飾キー解釈の関数（上記、`awase-windows/src/vk.rs`。コアからは使えない）と揃える必要がある。そこで、コアに**文字列だけ**の区切り関数 `split_combo(s) -> (mods_text, main_text)` を置き、`vk.rs` の解釈関数もこれを使う。
  - **残る別名**は、`canonical_key_text` では揃わない名前だけ（round3 R3-4 で訂正）。`Nonconvert` は大文字にして `NONCONVERT` になり `VK_NONCONVERT` と揃う。揃わないのは、`変換`/`無変換`/`漢字`/`IMEオン`/`IMEオフ`/`ImeOn`/`ImeOff`/`VK_OEM_AUTO`/`VK_OEM_ENLW`、
    決定1で足す中立名（`Enter`↔`RETURN`、`Esc`↔`ESCAPE`、`Backspace`↔`BACK`）など。このうち**コアの検証が意味を問うキー**（かな、F15〜F24、変換、無変換）に関わるものだけを、`THUMB_KEY_ALIASES` の小さな表に持つ。
  - **一致を保つテスト（向きに注意）**: 検証で実際に困るのは「`from_name` で同じ VK になるのに、コアの `canonical_key_text` と別名の表では同じと判断されない」（**見逃し**。例: `left_thumb_key = "カナ"` でかなキーの警告が出ない）である。
    したがって、コアの検証が意味を問うキー（かな、F15〜F24、変換、無変換）について、**`from_name` がその VK に解決する全ての名前が、コアの `canonical_key_text`＋別名の表で同じ組に入る**ことを、`awase-windows` 側のテストで確かめる。
    これで、`from_name` に別名を足したときにコア側の追加漏れを CI が検出する。（逆向きの「`canonical_key_text` で同じなら `from_name` でも同じ VK」は誤検知の防止で、補助として足してよい。）
- **`"Left Alt"`/`"Right Alt"`（Alt なりすまし）は VK 名ではない**ので `from_name` に入れない（`resolve_thumb_key` が目印として特別扱いする現状を維持する）。
  ただし `from_name` が大文字小文字を区別しなくなるので、目印だけが区別すると `"left alt"` が `from_name` に落ちて失敗する（round3 R3-6。今は GUI の候補からしか入らない値なので実害は小さい）。
  `resolve_thumb_key` の比較も大文字小文字を無視する。

### 決定2: 握りつぶしを、既存の診断（`StartupDiagnostics`/`validate()`）に流す

- 新しい型（`ConfigReport` 等）は作らない。`Vec<String>` の警告に、握りつぶされている (a)〜(e) を流す。`KeymapTable::new` は警告の一覧を返す形にする。
  警告を `Vec<ConfigWarning{path, msg, severity}>` のような構造にするかは、実装時に既存の使われ方を見て決める（未決事項3）。
- ホットキーは「名前の解決の失敗」と「`RegisterHotKey` の失敗（他のアプリが先に使っている等）」の両方を診断に流す。
- **未知のキーの検出**: `AppConfig::load`（`src/config.rs:806-812`）は `Result<Self>` で警告を返す口がなく、`validate()` は読み込み後の `AppConfig` を受け取るので、未知キーはその時点で失われている。
  そこで `AppConfig` に `#[serde(skip)] load_warnings: Vec<String>` を持たせ、`load()` が `serde_ignored`（新しい依存。小さい）の結果と `keymap` の合流の警告をここへ入れ、`validate()` がそれを警告に加える。
  `load()` と `validate()` の呼び出し元のシグネチャは変わらない。**読み込みの入口は `AppConfig::from_toml_str(&str)`（`keymap` の合流と `load_warnings` を含む）に一本化する**（round3 R3-5）。
  `load()`、不具合報告（`app/mod.rs:278` は今 `toml::from_str` を直接呼び、合流も未知キーの検出も通らない）、決定4のテストはこれを使う（テストが直接 `toml::from_str` で読むと「実際の読み込み」を検証したことにならない）。`AppConfig::default()` を `toml::Table` にして既知のキーを作る案は、`Option` の `None`（`engine_toggle_hotkey` など）が書き出されず、
  正しい項目を未知と誤報するので採らない（`serde_ignored` は alias の `engine_off_solo_triple` も自然に扱える）。`keymap` に対して「`keymaps` の間違いでは」と近い名前を示す。
- **トレイ通知は「設定した機能が働かない」ものに限る**（値が解決できない、ホットキーの登録失敗）。未知のキー・撤去済みのキーは、ログと GUI の診断欄だけにする。
  撤去済みの設定（`apply_calibrated_mode_keys`、`[[calibration]]`）は「撤去したキーの一覧」を持って警告しない（`toml_edit` にするとこれらが保存後も残るため）。`reload_config` のたびに同じバルーンを繰り返さない（前回と同じ内容なら出さない）。
- 既知の非対称（記録のみ、本ADRでは直さない）: `post_bypass`、`engine_on/off_ime_key`、`engine_toggle_hotkey` の登録は `reload_config`/`apply_config_update` で**解決し直されない**（再起動まで効かず、警告も出ない）。

### 決定3: 保存は `toml_edit` で、「読み込んだときの値」から GUI が変えた項目だけを、保存の直前にディスクから読み直した文書へ書く（三者比較）

- GUI の保存・トレイの `save_auto_start`（同じ全体書き直しの経路）とも、`toml_edit` で文書を編集する。未知キーとコメントが残る。
- **三者比較**（round3 R3-1。保存直前のディスクと `to_save` の二者比較は採らない）:
  - **基準 `base`**: GUI が設定を読み込んだときの値（`validate()` で正規化する前の生の `AppConfig`）。GUI の構造体のフィールド1つ（例: `loaded_snapshot`）で持つ。更新の規則（round4 R4-1）:
    読み込み・キャンセルでの読み直しでは、読んだ生の値（`self.config` と `base` の両方）。**保存の成功では、保存した `to_save` の複製**（保存はバックグラウンドのスレッドで行われるので、`poll_pending_save` が `Saved` を受け取ったときに更新する。
    保存に失敗したときは更新しない）。保存後にディスクから読み直して `base` にはしない（保存中に GUI で行った編集を失う）。保存を始めた時点の値を `base` にもしない（失敗しても `base` が進み、次の保存でその差分が書かれなくなる）。
    正規化した値は保存時に差分として書かれているので、`base` が正規化後の値になっても、次の保存で `speculative` 型の差分が出なくなるのは正しく、ずれは残らない。
    GUI が自分で `save_auto_start` を呼んだ後に `self.config.general.auto_start` を更新するときは、`base` の同じ項目も更新する（書き直しを避ける。害はないが記述をそろえる）。
  - **自分 `to_save`**: GUI が保存しようとしている値（`validate()` の結果を `AppConfig::from(validated)` で戻したもの）。
  - **相手 `disk`**: 保存の直前にディスクから読み直した文書。**書き込み先の文書として使うだけ**で、比較の基準にはしない。
  - **書く項目は、`to_save` が `base` と違う項目だけ**。GUI が変えていない項目は、ディスクの値（外部エディタでの編集を含む）がそのまま残る。
  - 二者比較（`disk` と `to_save` だけ）は成り立たない: GUI を開いた後に外部エディタで `keys.ime_detect` を書き換え、GUI で別の項目だけ変えて「適用」すると、`to_save.ime_detect` は古い値、`disk.ime_detect` は新しい値で「違う項目」になり、
    古い値が書かれて外部の編集が消える。`main.rs:823-851` が直した stale read-modify-write（2026-09-05 のユーザー報告）そのもの。`GUI が変えた` と `ディスク側が変わった` を区別する情報は、`base` にしかない。
  - この方式で、(i) 画面に無い項目の再読み込み（`main.rs:845-851`）は本当に要らなくなり、段階3で消せる、(ii) トレイの `save_auto_start` が間に書いた `auto_start` も残る（別プロセスの変更を含む）、(iii) 両方が同じ項目を変えたときは GUI の値が勝つ（今と同じ）。
- **`AppConfig` の項目から、文書のどのキーを書くかを特定する方法**: `base` と `to_save` を serde で `toml::Value` に変換し、再帰的にたどって違うパスを集め、`DocumentMut` の同じパスに書く（serde の `rename` は変換の時点で反映される）。
  値が `None` になったパスはキーを消す。配列の表は、差があれば丸ごと置き換える。`skip_serializing` の `legacy_keymap` はここに現れない。
- **alias を持つ項目を書くときは、文書から旧名のキーを消す**（round3 R3-1a。`keymap`→`keymaps`（決定5）を一般の規則にする）。`engine_off_solo_repeat` には `#[serde(alias = "engine_off_solo_triple")]` があり、
  旧名が書かれたファイルで GUI がこの値を変えると、正規名が新しく書かれて旧名も残り、次の読み込みで serde が重複として**読み込み全体を失敗**させる（`Dangerous`）。
  テスト: 「旧名だけのファイルでその項目を編集して保存 → 再読み込みできる」。
- **`validate()` による正規化は、今と同じく保存する**。`base` は正規化する前の値（例: `confirm_mode = "speculative"`）、`to_save` は `AppConfig::from(validated)` で正規化した値（`two_phase`）なので、違いとして書かれる。
  これは GUI の `apply`（`main.rs:858-868`）が直した「警告が『適用』のたびに永遠に再表示される」不具合を戻さない。
  したがって範囲外の数値（`simultaneous_threshold_ms`、`speculative_delay_ms`、`*_percent`、`layouts_dir` の `..` など）が既定値に戻ってディスクに書かれる挙動は**今と同じ**（この ADR で悪化も改善もしない）。
  キー名については、`validate()` は文字列を書き換えない（解決の失敗は Windows 側で起きる）ので、「解釈できない値→既定値がディスクに書かれる」は今も起きていない。
  「範囲外の値を実行時だけ既定値に戻し、保存しない」ようにするには、編集用の値と検証済みの値を分けて持つ GUI の大きな変更が要る。将来の課題として残す（未決事項7）。
- **既定値と同じ値は書かない**: GUI が値を**変えて**、その結果が既定値に等しいときは、キーを消す（そのキーに付いていたコメントも一緒に消える。許容する）。GUI が変えていない項目は、既定値と同じ値が明示されていても触らない。
  **「既定値」は、キーが無いときに serde が読む値、つまり `AppConfig::from_toml_str("")` の結果**とする（round3 R3-1b）。`GeneralConfig::default()` や同梱の `config.toml` の値ではない
  （フィールド単位の `#[serde(default = "…")]` が3つあり、同梱の `config.toml` とコアの既定値は食い違う。`layouts_dir` はコアでは `"config"`、同梱では `"layout"`）。「同梱の値に戻したらキーが消えて、コアの既定値に変わる」事故を防ぐ。
  **すでに固定されたファイル**（例: 過去に保存された `ime_toggle = ["VK_KANJI"]`）は、GUI が変えない限り触らないので固定されたまま残る。この扱いは ADR-199 T14 と一緒に決める（未決事項4）。
- 配列の表（`[[keymaps]]`・`[[post_bypass]]`）は、変わったら配列ごと置き換える（その配列のコメントは失う）と割り切る。同じ項目がドット付きキー・インライン表・`[keys]` の表のどれで書かれていても見つけて書き換え、表が無ければ作る。
- **保存の直前にディスクが無い・読めないとき**（round4 R4-2。読み込み時は `Loaded` でも、GUI を開いている間にファイルが消えた・外部エディタの書きかけで TOML として壊れた・共有違反などで一時的に読めない、が起きうる）:
  ファイルが**存在しない** → 空の文書から始め、`to_save` のうち既定値と違う項目をすべて書く（`base` との差分だけだと、GUI が知っている既定値以外の設定を失う）。
  TOML として**読めない**、または読み取りに**失敗した** → 保存を中止し、「ファイルが外部で編集されていて読めません」とエラーを出す（今の全体書き出しと違い、外部の書きかけを上書きしない）。全体書き出しにするなら `Dangerous` と同じくバックアップを取ってから。
- **`None` を保存できない項目**（round4 R4-3。今もある制約を、この規則が引き継ぐ）: `GeneralConfig`・`KeysConfig` は構造体の単位で `#[serde(default)]` を持つので、キーが無いと `Default` の値が入る。
  キーが無いときの既定値が `Some` の `Option` 項目（`ngram_file`、`engine_off_solo_repeat`）は、TOML に null が無いため `None` をキーの削除でしか表せず、削除すると既定値の `Some` に戻る（実行で確認: `None` を保存して読み直すと既定値の `Some` に戻る）。
  つまり「値が `None` になったパスはキーを消す」は、これらの項目では「既定値に戻る」の意味になる。`engine_off_solo_repeat` は空文字 `Some("")` を「無効」の意味に使って回避している（`bootstrap.rs` の `.filter(|s| !s.is_empty())`）。
  **GUI の n-gram ファイル欄（`main.rs:4079-4085`）は、空にして保存しても次に読むと既定のファイルに戻る**（今もある不具合。既知の問題として記録し、直すのは別件）。
- **差分と上書きの単位**（round4 (e)）: 葉の値か、配列全体（普通の配列も含む。配列の一部の要素だけが変わっても配列ごと置き換える）。外部エディタが同じ配列の別の要素を変えていた場合は、GUI の古い要素で上書きされる（配列の単位では GUI が勝つ）。 `toml_edit` で読めるかどうかに関係なく、今と同じ全体の書き出し（バックアップを取った後）にする。`Dangerous` には「TOML として正しいが serde で型が合わない」場合（数値の項目に文字列など）があり、
  この場合は `toml_edit` では読めるので差分だけ書く経路に入ってしまうが、壊れた項目が残って次も `Dangerous` になり、`base` は（読み込みに失敗したので）既定値のため、ユーザーが見ていない項目の差分が大量に出る。
- **`toml_edit` の版**: `Cargo.lock` に 0.22.27（`toml 0.8` 経由）と 0.25.13 がある。直接の依存にするときは 0.22 系に揃え、`toml 0.8` と共有する。
- **必須のテスト**: (1) 何も編集せずに保存すると、ファイルがバイト単位で変わらない。(2) GUI を開いた後に外部エディタで画面に無い項目（`keys.ime_detect` 等）を書き換え、別の項目だけ変えて保存しても、外部の編集が残る
  （2026-09-05 の stale read-modify-write の回帰テスト）。(3) トレイの `save_auto_start` が間に書いた `auto_start` が、GUI の保存で消えない。(4) alias の旧名テスト（上記）。

### 決定4: GUI の候補×読み手、文書・サンプル、合成した実 config 風の集まりをCIに入れる（実装の最初にやる）

1. **GUI の候補 × 読み手の受理テスト**: `awase-settings` の候補表（`THUMB_KEY_OPTIONS`、`ALT_IMPERSONATION_OPTIONS`、`IME_MODE_KEY_OPTIONS`、`SOLO_REPEAT_EXTRA_OPTIONS`、`KEYMAP_MAIN_KEYS`）の全内部名について、
   `format_combo` を通した文字列が、**その項目の実際の読み手**（`parse_hotkey`/`parse_key_combo`/`resolve_thumb_key`/`from_name`/keymap の `to` の解決）で `Some` になることを確かめる。
   `parse_hotkey` は `#[cfg(windows)]` なので、Linux では前置き処理まで含めた同等の関数で確かめ、実物は windows-build CI に任せる。
   あわせて GUI の**読み手**（`parse_combo_str`）→ `format_combo` の往復で修飾キーが落ちないことも確かめる（決定1）。
   **段階0だけでは緑にならない**: ホットキーの `変換` など4件（背景 #1）が残っているので、段階0では「既知の失敗の一覧」（この4件）を期待値として書き、段階1でその一覧を空にする。
2. **文書・サンプルの例**: 対象は `config.toml`、`README.md`、`docs/usage*.html`（ユーザーが実際に読むもの）。例のブロックは `example-begin`/`example-end` のような目印で囲む（コメントアウトの説明文が混ざるので、`#` を外すだけでは TOML にならない）。
   **ADR は対象外**（当時の記録であり、現在の読み込みで診断ゼロを要求すると過去の記録の書き直しを強いる。`[[keymap]]` の古い表記が ADR-114 に52回ある）。「デフォルト値」と書いた例は `KeysConfig::default()` と一致することも確かめる。
3. **合成した config の集まり**（round2 R2-8）: 実際の不具合報告（ADR-095）の `config.toml` は、ユーザーが awase に送ったもので、公開リポジトリに置くことへの同意がない。匿名化しても、`app_overrides` のプロセス名・クラス名の組み合わせ、
   `[[keymaps]]` の `app`、`layouts_dir` のパスなど、使っているソフトや環境が分かる情報が残る。したがって**実物はリポジトリに置かない**。
   (a) 実物を元に**作った**合成の config（表記のゆれを網羅する）を `tests/fixtures/configs/` に置き、「診断の件数が増えないこと」を確かめる。(b) 実物は R2 から取ってローカルでだけ流す（リポジトリにも CI にも置かない）。
   互換性（決定5）を裏付けるのは往復テストではなく、この集まり。

### 決定5: 古い表記は読み込みで受け付け続ける。`[[keymap]]` は合流させ、`keymaps` を書くときは `keymap` を消す。正規形は `VK_*` 名

- 別名・`VK_` の有無・日本語名は、決定1の寛容な `from_name` で受理する。移行スクリプトは作らない。
- **`[[keymap]]` は `alias` にしない**。`alias = "keymap"` にすると、ファイルに `[[keymap]]` と `[[keymaps]]` の両方があるとき serde が重複として**読み込み全体を失敗**させる（`ConfigLoadState::Dangerous`）。
  読み込みのときに `keymap` を `keymaps` に合流させる（両方あれば連結して警告）。実装は、`#[serde(default, rename = "keymap", skip_serializing)] legacy_keymap: Vec<KeymapRule>` のような隠しフィールドを持ち、`load()` の後で `keymaps` に移す（`AppConfig` の形の変更）。
- **保存のとき、`keymaps` を書くなら文書の `keymap` 配列を消す**（合流した結果を `keymaps` に一本化する）。これがないと、読み込みで合流 → GUI で1つ編集 → 配列ごと置換で `[[keymaps]]` に全体が書かれる →
  `[[keymap]]` も残る → 次の読み込みで2回合流 → 保存のたびに規則が倍になる。`keymaps` を書かない保存（GUI がキー割り当てを触らなかった）では `keymap` はそのまま残す。
  テスト: 「`[[keymap]]` だけがあるファイルで `keymaps` を1つ編集して保存 → 再読み込み → 規則の数が変わらない」。
- **書き出す正規形は `VK_*` 名**（今の GUI の出力のまま）。理由: (i) 書き換えの差分が最小、(ii) `from_name` がどの版でも受けてきた唯一の形でダウングレードに一番強い
  （ホットキーだけは PR #330 より前の版で不可。既知として記録する）、(iii) 日本語の通称は曖昧（下記）。GUI に見せる表示名は別に持つ。ADR-019 の「中立名を既定値に」が Windows では守られていない現状は、追認として注記する。
- 名前の意味の曖昧さ: `"かな"`/`"Kana"`→`VK_KANA`（0x15）だが、JIS の物理的な「カタカナ/ひらがな」キーは `VK_DBE_HIRAGANA`（0xF2）を送る。`"漢字"`→`VK_KANJI`（0x19）だが、物理的な「半角/全角」キーは 0xF3/0xF4 を送る。
  Linux の表では `"Kanji"` が**かなキー**（`KEY_KATAKANAHIRAGANA`）を指す（プラットフォーム間の意味のずれ。別の不具合として記録する）。

### 挙動の変化（寛容化で「今まで無効だった設定が有効になる」）

- 今受理されている文字列の意味が変わる入力は**見つからなかった**（round2 で確認: 無視していた大文字小文字・前置きは、今はすべて失敗している。名前を小文字にして別の VK と衝突するものは表に無い。修飾キーは最後のトークンにならない）。
- 本当に変わるのは、**今まで黙って無視されていた設定が有効になる**こと: `[[post_bypass]] key = "Ctrl+J"`、`[[keymap]]`（合流）、`ime_detect = ["F13"]`、`keys.engine_on = ["Ctrl+F12"]`（今は警告を出して捨てる）、`dedicated_fn_key = "F18"`。
  ユーザーが昔書いて、効かないので忘れていた再割り当てが、更新しただけで効き始める。ADR-199 の Q2（明示 config と役割が重なったら config を優先）により、今まで捨てられていた `keys.ime_*` の値が解決されると、役割の逆算の結果も変わる。
- 対応: CHANGELOG に明記する。最初の起動で「以前は無視されていた設定 N 件が有効になりました」を診断（ログと GUI の欄。トレイには出さない）に出すかは未決事項1。

## 検討して採らなかった案

- **`RawConfig`→`resolve`→`KeySpec`+`ConfigReport` の二層化（草案の決定1・2）**: round1 で取り下げ。
  - 既存の `validate()`/`ValidatedConfig`/`StartupDiagnostics`（ADR-116）の作り直しになる。
  - `KeySpec{mods, vk}` では `"Left Alt"`（Alt なりすまし）が表せず、保存で `VK_NONCONVERT` に変わり、Alt で親指シフトしていたユーザーが保存しただけで無変換キーに変わる。
  - コアの `validate()` がキーの意味を文字列で判定しているので、名前表を持たないコアでは書けなくなる（トレイトに意味の問い合わせを足すか、検証を全部 Windows 側へ移す必要がある）。`KeysConfig::default()` もコアにあり、名前表なしで作れなくなる。
  - 費用: 型付けの対象が18項目（`GeneralConfig` 4、`KeysConfig` 8、`ImeDetectConfig` 3、`KeymapRule` 2、`PostBypassRule` 1）、同じ構造体のキー以外の項目は `GeneralConfig` だけで37項目。
    GUI が `&mut String` を直接編集する箇所が 57、コア約29、`awase-windows` 約30。追加 800〜1500 行（推定、未実装）に対し、減る重複は 60〜80 行。
- **`KeyName(String)` の新型で包むだけ**: 検証はできるが、二重加工は止まらない。
- **`#[serde(flatten)] extra: toml::Table` で未知キーだけ残す**: コメントは消えたまま。`flatten` は既知の制約（`deny_unknown_fields` と併用できない、入れ子の各階層に付ける必要）を持ち込む。`toml_edit` が既に依存にあるので選ぶ理由が薄い。
- **表を `awase-vkmap` に置く**: `awaza` が再利用する純粋な表 crate に名前解決を混ぜたくない。vkmap はコアに依存する側でもある。

## 将来の別ADR（今回は扱わない）

- **型付け**: 必要になったら、コアに VK 値を持たない列挙型 `KeyName`（`Convert`・`F12`・`OemPlus`…）、各プラットフォームに `KeyName → VkCode` の `match` を1つ持つ形（ADR-019 を守り、コアの検証がキーの意味を列挙型で判定でき、Linux/macOS の日本語名の問題も自然に直る。トレイト注入は要らない）。
  「不正な値を元の文字列のまま持って読み込みは成功させる」包み型（`Lenient<T>`）で、設定全体の読み込み失敗（`Dangerous`）を避ける。
- Linux/macOS スタブの名前表（現状は日本語名・`VK_` 付きを受けず、既定値でも起動時エラー）。

## 段階（各段階は単独で価値がある。順序が重要）

| 段階 | 内容 | 直る不整合 | 規模の見積もり（未実装の概算） |
|---|---|---|---|
| 0 | 決定4のテスト（GUI候補×読み手、文書・サンプル、合成 config）。**既知の失敗の一覧を期待値**にして緑にする | 再発の検出 | 80〜150 行 |
| 1 | 決定1: `from_name` と修飾キー解釈を寛容・1関数に。文字列比較を VK 比較へ。コアの検証は `canonical_key_text` で揃える（移さない）。段階0の既知の失敗の一覧を空にする | 1、2 | 40〜80 行（削除を含む） |
| 2 | 決定2・5の一部: (a)〜(e)、`load_warnings`（`serde_ignored`）、`keymap` の合流（隠しフィールド）、`KeymapTable::new` の戻り値 | 2、3、握りつぶし全般 | 80〜150 行 |
| 3 | 決定3: `toml_edit` の保存（`save_auto_start` を含む、三者比較〈`base` のフィールドと更新〉、alias の旧名と `keymap` を消す書き出し、`main.rs:845-851` の削除、`Dangerous` のときの全体書き出し） | 4 | 260〜480 行 |

合計 460〜860 行（推定）。二層化（800〜1500 行＋GUI の大改修）より小さい。段階3は ADR-199 T14 の既定値の移行より前にやる価値がある。

## 実装時に決める細部（ADR で固定しない。round4 で確認済みの注意点）

- `DocumentMut` でのパスの引き方（ドット付きのキー・インライン表・`[keys]` の表のどれで書かれていても同じパスを引く）。
- `split_combo` の端の場合: `"Ctrl+"`（主キーが空）、`"+"`、前後の空白、修飾キーが1つも無い `"F12"`。`+` の文字そのものは `VK_OEM_PLUS` という名前で書くので区切りとぶつからない。
- 表を正規化した形で書き直した後の受理テスト: 今の `from_name` の表の全ての名前が、書き直した後も同じ VK に解決されること（未決事項8のテストに含める）。書き直しで正規化の漏れがあると、その名前だけ受理されなくなる。
  条件は、表のキーを正規化した形（`VK_A`→`"A"`、`ImeOn`→`"IMEON"`、`IMEオン`→`"IMEオン"`、`VK_OEM_1`→`"OEM_1"`）で書くこと。`to_ascii_uppercase` は日本語名に無害（全角の文字は変わらず、全角表記は今も受けていない）。
- `str::trim` は全角の空白（U+3000）も除く。寛容にする方向なので許容する。

## 未決事項

1. 「以前は無視されていた設定 N 件が有効になりました」を最初の起動で診断（ログ・GUI の欄。トレイには出さない）に出すか。CHANGELOG への明記は行う。
2. （閉じた）コアの検証と Windows 側の検証の線引き → 決定1の `canonical_key_text`（順序: 空白→大文字→`VK_`）と `split_combo` で段階1は線引きを待たない。VK の意味を使う検証を移す話は、将来の型付け（`KeyName` 列挙型）の ADR で扱う。
3. 診断を構造化（`ConfigWarning{path, msg, severity}`）するか、`Vec<String>` のまま行くか。
4. すでに固定された既定値（`ime_toggle = ["VK_KANJI"]` 等）の扱い。ADR-199 T14 と同時に決める。
5. **握りつぶしの禁止をどう強制するか**: まずは正規表現のアーキテクチャガードは入れず（`.ok()`/`filter_map` は設定と無関係な箇所でも多用され誤検出が多い）、
   決定4-1のテストと、キー項目ごとの「不正な値→診断に1件出る」表形式のテストで押さえる。型での強制は将来の型付けの ADR に回す。
6. `docs/usage.html` の「デフォルト値」の例（`ime_toggle = ["VK_KANJI"]`）が ADR-199 T14 の後に意味の上でずれる件（読み込みには通るのでテストは緑のまま）。
7. 範囲外の値を実行時だけ既定値に戻し、保存しない方式（編集用と検証済みの値を分けて持つ GUI の変更）は、今回は採らず、将来の課題とする。今の挙動（保存する）は悪化しない。
8. `from_name` の寛容化で意味が変わる入力は見つからなかったが、実装時に「`from_name` の全ての名前を小文字・`VK_` なしにして別の VK と衝突しないか」を機械的に確かめるテストを足す。

## 検証

- **冪等性**: 合成 config の各値 `s` について `resolve(write(resolve(s))) == resolve(s)`（正規形で書いたものを同じ版が読める）。ただし、これだけでは過去の表記の受理・意味の正しさ・ダウングレードは保証できない。
- **合成 config の集まり**（決定4-3）: 過去の表記がすべて受理され、診断の件数が増えない。
- **バイト一致**（決定3）: 何も編集せずに保存すると、ファイルが変わらない。
- **外部編集の保持・auto_start の保持・alias の旧名**（決定3 の必須テスト (2)〜(4)）。
- **`Dangerous` のとき**は、`toml_edit` で読めても全体の書き出しになる。保存の直前にファイルが無い・壊れている・読み取りに失敗したときの規則（決定3）。
- **`base` の更新**: 保存の成功では `to_save` の複製、失敗では更新しない。保存中に GUI で行った編集が、成功後の更新で失われない。
- **`keymap` の合流**（決定5）: `[[keymap]]` だけがあるファイルで `keymaps` を1つ編集して保存 → 再読み込み → 規則の数が変わらない。
- **GUI の候補 × 読み手**と、GUI の読み手→`format_combo` の往復で修飾キーが落ちない（決定4-1）。キー項目ごとの「不正な値→診断に1件」の表形式テスト。
- **コアと `from_name` の一致**（決定1）: コアの検証が意味を問うキー（かな、F15〜F24、変換、無変換）について、`from_name` がその VK に解決する全ての名前が、`canonical_key_text`＋別名の表で同じ組に入る（見逃しの防止。逆向きは補助）。
- 実機: 既存の `config.toml`（同梱サンプル）を読み込み、診断が期待どおりであること。
