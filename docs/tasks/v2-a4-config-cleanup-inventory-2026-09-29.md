---
title: v2 A4 設定項目整理の棚卸し（keys.ime_detect・*_solo_tap_ime_action・keyboard_model）
status: 棚卸しのみ（実装なし）。推奨は下の結論表。所有者の判断待ち論点あり（末尾）
created: 2026-09-29
related_adr: ["ADR-153", "ADR-192", "ADR-195", "ADR-196", "ADR-199", "ADR-201", "ADR-202"]
source: docs/tasks/v2-release-checklist-2026-09-29.md A4
base_commit: 88f9c1f8（origin/develop、PR #367 マージ直後）
---

# v2 A4: 設定項目整理の棚卸し（2026-09-29）

所有者決定（2026-09-29）で v2 のスコープに A4 を含めた。A4 の前提は「学習・較正が完成すれば不要になる設定項目」。
この文書は **実装せず**、撤去または既定変更の判断材料を作る。根拠はすべて `88f9c1f8` のコードを読んで確認した
（ビルド・テストは実行していない。行番号は同コミット）。ADR-199 の決定1・9・12・15 と矛盾する提案はしない。

## 結論（推奨）

| 項目 | 推奨 | 一言の理由 |
|---|---|---|
| `keys.ime_detect.{toggle,on,off}` | **残す**（既定も現状維持） | ADR-199 決定9・12 が「存続」と明記。ATOK 本体・未検出・MS-IME 互換モードで唯一のユーザー側の手段。既定 `on`/`off` を空にする案は挙動差があり、v2.0 では見送り（論点 Q1） |
| `muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action` | **残す（v2.0）**。撤去は実機 A/B 後の別ラウンド | 学習・逆算では置き換わっていない。ADR-153 の「@」対策（ケース3改）と GUI の案内（ADR-192 T3）が依存。`keys.ime_on/off` の bare 指定（ADR-192 決定3b）で機能的にはほぼ代替できるが、「@」抑止の同等性が未検証（論点 Q2） |
| `general.keyboard_model` | **残す** | IME の学習・逆算とは別の軸（物理配列 JIS/US）。A4 の前提が当てはまらない |

「撤去可」と言える項目は今回のところ無い。理由は各項の「撤去した場合」に書いた。
なお、類する設定として `keys.engine_on_ime_key` / `engine_off_ime_key` が「能動書き込みの残骸」候補（論点 Q4）。

---

## 1. `keys.ime_detect.{toggle,on,off}`

### (1) 定義・既定・使われ方

- 定義: `ImeDetectConfig`（`src/config.rs:510-517`）。既定は `toggle=[]`、`on=["IMEオン"]`、`off=["IMEオフ"]`（`:519-543`）。`KeysConfig.ime_detect`（`:563`, `:621`）。
  `"IMEオン"`/`"IMEオフ"` は VK 0x16/0x1A（`vk.rs:589-590`）。
- 読込: `init_ime_sync_keys`（`app/mod.rs:383-423`）が VK 化する。親指キーと同じ VK は BUG-140 対策で除外して警告。
  起動時 `bootstrap.rs:1186`、reload 時 `app/mod.rs:827`。
- 使われ方: `FocusTracker::enrich_ime_relevance`（`runtime/focus_tracker.rs:55-75`）が打鍵に `sync_direction`（Toggle/TurnOn/TurnOff）を付ける。
  `kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs:1172-1173`）が **最優先**で採用し、`write_sync_key`（`:1232-1239`）で belief を書く。
  物理キーは消費せず素通し（belief の追随だけ）。
- 保存: GUI に編集ウィジェットは無い（`settings/main.rs:2806` のコメントで撤去済み）。ADR-201 の三者比較保存（`config_save.rs:262-279` のテスト）で、外部エディタでの編集は GUI 保存で消えない。
- 診断: `config_diagnostics.rs:100-102`（新たに効くキー名）、`config_key_resolution_tests.rs:173-175`（解決確認）。bug_report には出ない。
- 文書: `config.toml:27-34`、`docs/usage.html:778-791`・`usage.en.html:738-`（例ブロック `usage-ja-10`/`usage-en-9`。CI が例を検証する）、`README.md:233`・`README.en.md:232`（トラブルシュート）、`docs/design/settings-gui.md:118-154`（撤去済みの「タブ3」を今も記載）。
  テスト: `vk.rs:1318`（既定同士の衝突禁止）、`engine/tests.rs:7213,7242`、`app/mod.rs:904,944`、`settings/main.rs:7429`、fixture `tests/fixtures/configs/notation_*.toml`。

### (2) 代替されている部分／いない部分

- ADR-199 決定9: `keys.ime_detect` は「物理キーを消費しない belief の追随」で、決定12（belief の観測追随は存続）により **対象外・存続**と明記されている。
- 既定の `IMEオン`/`IMEオフ` は、静的な `ImeKeyKind::shadow_effect`（`vk.rs:146-160`、0x16→TurnOn・0x1A→TurnOff、決定9で存置）と **同じ方向**。冗長に見える。
  ただし経路が違う:
  - `sync_direction` は `is_japanese_ime()` に依らず効く。`shadow_action` は `is_japanese_ime()` が真のときだけ（`key_pipeline.rs:1174-1186`）。
  - 意図の種別が `SyncKey`／`PhysicalImeKey` で異なる（`evidence.rs:381`, `:584`）。
  - F13〜F24 では `sync_direction` があると役割判定をしない（`passive_without_lookup`、`key_effect_runtime.rs:650-660`）。
  つまり既定を空にすると、grace 期間中に `is_japanese_ime()` が偽を誤答する場面で 0x16/0x1A の追随が一瞬効かなくなりうる（コードからの推論。実機未確認）。
- 学習表・`config1.db`・MS-IME レジストリは **役割（能動）と予測（`KeyEffectPredicted`）**を担い、「この VK は ON/OFF/トグルだ」とユーザーが宣言する手段は置き換えない。

### (3) 撤去した場合の影響

- config: フィールドを消すと serde が無視し、未知キー警告（ADR-201 決定2）が出る。警告を止めるなら `config_load_diag.rs:10-18` の `REMOVED_KEYS` にパス `"keys.ime_detect"` を足す（`serde_ignored` は無視された表の最上位パスを報告する想定。要テスト）。
  ただし `output_mode` 等の前例は「効果の無い死んだ設定」を黙って無視するもの。`ime_detect` は **効いていた**ので黙って無視すると belief 追随が無言で消える。撤去するなら1回限りの移行警告が要る。
- 保存: `config_save.rs` の三者比較は未知キーをファイルに残す（同ファイルのテストが `removed_or_unknown = 1` の保持を固定）ので、撤去してもユーザーの config.toml は書き換わらない。
- GUI: ウィジェット無し。影響は `main.rs:7429-7520` のテストのみ。bug_report: 影響無し。`config_diagnostics.rs`・`config_key_resolution_tests.rs` の該当行と `vk.rs:1318` のテストは削除。文書は上記5箇所。

### (4) 代替が不十分な IME

| IME | 現状 | ime_detect の位置づけ |
|---|---|---|
| GJI（`config1.db` あり） | 役割逆算＋学習表（ADR-199） | 補助（通常は不要） |
| MS-IME 本体（新） | レジストリ逆算（ADR-199 決定7） | 補助 |
| MS-IME 互換モード | 半角/全角は受動（決定17） | **唯一の手動手段**（`toggle` に書く） |
| ATOK 本体・Japanist・その他 | 役割なし・受動（決定7 の表） | **唯一の手動手段** |
| IME 未同定（`table_ime_kind()` が None、`runtime/mod.rs:674`） | 役割を付けない | **唯一の手動手段** |

### (5) 再発ファミリー・必要な回帰テスト

キー選択・IME belief の両ファミリー（`runtime/key_pipeline.rs`、`runtime/focus_tracker.rs`）。現状維持なら変更なし。
**既定を空にする場合**（Q1 で所有者が選んだときだけ）は、(a) `kp_stage_shadow_ime_toggle` の `SyncKey`→`PhysicalImeKey` 切替と `is_japanese_ime()` 偽のときの 0x16/0x1A、(b) `transport.rs` の配送（0x16/0x1A は元々 `shadow_action` を持つ）を `crates/awase-windows/tests/golden_scenarios.rs` または `journal_replay.rs` で固定する。`vk.rs:1318` の衝突テストは空でも通る。

---

## 2. `muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action`

### (1) 定義・既定・使われ方

- 定義: `Option<ShadowImeActionConfig>`（`on`/`off`/`toggle`、`src/config.rs:394-397`, `:455-`）。既定 `None`（`:448-449`）。隠し設定（ADR-153 決定1）。
- 配線: `bootstrap.rs:817-826`、reload `runtime/mod.rs:2161-2171`、Engine へ `set_*_solo_tap_ime_action`（`nicola_fsm.rs:825-877`）。
- 効き方（2箇所）:
  - KeyDown 時、belief が OFF のとき: `explicit_ime_action_target`（`key_pipeline.rs:974-1019`）。`on`/`toggle` は `PromoteToOn`（通常の意図昇格に合流）、`off` は `SuppressOnly`（**生キーを Suppress するだけで書かない**、BUG-124 対策）。`is_japanese_ime()` が偽なら不活性（`:987`）。
  - 単独タップ確定時、belief が ON のとき: `resolve_pending_thumb_as_single`（`nicola_fsm.rs` の優先順位、専用 Fn キー ＞ bare `keys.ime_*`（`forced_open_action`、`:2193`） ＞ **`*_solo_tap_ime_action`** ＞ `ModeKeyConfig`）。
- 副作用の依存: 状態依存キー警告（ADR-192）の「素通しでない」判定に使う（`runtime/mod.rs:1477,1483`、`state_dependent_key_warning.rs:188`）。物理配送の例外 M19（`transport.rs:270-300`）。
- GUI: 専用ウィジェットは無いが、ADR-192 T3 の「置き換えを適用」が **親指キーのとき書き込む**（`settings/main.rs:630-668`、プレビュー文 `:2823`）。
- 検証: `config.rs:1195-1203`（`validate_thumb_key_in_ime_combos`、bare と同時指定の警告）。docs/README/usage には記載無し（`config.rs` の doc と ADR/BUG のみ）。

### (2) 代替されている部分／いない部分

- **代替されている**: 「無変換/変換の単独タップで IME を ON/OFF したい」という目的は、`keys.ime_on`/`ime_off` の bare 指定（ADR-192 決定3b、優先順位1.5）と ADR-199 T10（役割由来の `forced_open_action`、`runtime/mod.rs` の `enrich_thumb_key_role`）が担う。決定15により `keys.ime_on/off` 自体は awase 自身の能動設定として存続する。
- **代替されていない（推論を含む）**:
  1. 「off × 既に OFF」で生キーだけ Suppress する処理。GJI の TSF キー横取りが「@」を出す BUG-113/124 の対策で、`explicit_ime_action_consumed` → `transport.rs` の M19 例外が唯一の Suppress 手段だとコードが明記している（`transport.rs:283-300`）。`forced_open_action` 経路は親指を `PendingThumb` として `Decision::Consume` するため同等に見えるが、**エンジン OFF のとき**や KeyDown 時点の挙動が同じかは未検証。
  2. `PendingCharThumb` 等のタイミング（ADR-192 決定3b の議論で `*_solo_tap_ime_action` の制約が列挙されている）。
  3. GUI の案内（T3）が書き込み先にしている。
- ADR-192 決定3b は「`*_solo_tap_ime_action` へ **正規化**しない」と決めただけで、「`*_solo_tap_ime_action` を **撤去**して bare `keys.ime_*` に一本化する」案は評価されていない。

### (3) 撤去した場合の影響

- config: 既存の `muhenkan_solo_tap_ime_action = "off"` 等が **無言で無効化される**と、GJI で無変換にカスタム割り当てのあるユーザーで「@」が再発しうる（上の1）。読込時に「`keys.ime_on/off` の bare 指定へ移行してください」の警告（`AppConfig::validate` の warnings）を出し、その間は値を読み続けて動かす、という2段階が安全。
- GUI: `apply_adr192_recommended_replacement`（`main.rs:630-660`）とその undo・プレビュー・テスト（`:7757-7798`）を bare 書き込みだけに変える。
- 削除範囲は大きい: `config.rs`（`:394-397,448-449,455-`,`:1195-1203` と関連テスト `:2351-`）、`nicola_fsm.rs`（フィールド `:297-300`、setter `:825-877`、`:933,940`、`resolve_explicit_ime_action` とテスト多数）、`key_pipeline.rs`（`explicit_ime_action_target`、ケース2/3改の分岐 `:1122-1162`、KeyUp 早期分岐 `:1043-1050`）、`transport.rs` の M19 例外、`evidence.rs:355,594`、`runtime/mod.rs:1477-1483,2161-2171`、`bootstrap.rs:817-826`。ADR-153 の superseded 化と BUG-113/122/123/124 の記録更新も要る。
- REMOVED_KEYS 登録は上記の移行期間が終わってから。

### (4) 代替が不十分な IME

`explicit_ime_action_target` が要るのは `is_japanese_ime()` だけで、IME 種別の同定は不要（`key_pipeline.rs:987`）。したがって **ATOK 本体・未同定の IME でも効く**。一方、役割由来の `forced_open_action` は GJI と MS-IME 本体（同定できたとき）だけ（`runtime/mod.rs:674`、MS-IME 本体の無変換/変換は T17 Phase 4 まで受動）。config 由来の bare `keys.ime_*` は IME に依らないので、撤去後の移行先として使える。GJI 以外で「@」相当の問題があるかは不明。

### (5) 再発ファミリー・必要な回帰テスト

キー選択ファミリー（`nicola_fsm.rs::resolve_pending_thumb_as_single`、BUG-119 の前例）と物理キー配送（`transport.rs::plan`）、IME belief（`kp_stage_shadow_ime_toggle`）の3つに触れる。撤去するなら次が必要:
- `src/engine/tests.rs`: 移行先（bare `keys.ime_on/off` の `forced_open_action`）で、旧テスト（`nicola_fsm.rs:3468-3740` 周辺）と同じ入力列が同じ `ImeOpenRequest` になること。
- `transport.rs::plan_tests`: 「off × 既に OFF」の生キー Suppress が、移行先の経路でも維持されること（Windows CI のみ実行）。
- 実機 A/B（Windows Terminal + GJI で無変換を IME Off 系に割り当て、BUG-113/124 の手順）。これが無いと「@」再発を否定できない。
- 新規の `docs/known-bugs/` は撤去で不具合が出たときに起票する（今は不要）。

---

## 3. `general.keyboard_model`

### (1) 定義・既定・使われ方

- 定義: `GeneralConfig.keyboard_model: KeyboardModel`（`config.rs:213`）、既定 `Jis`（`:435`）。値は `jis`/`us`（`scanmap.rs:28-79`）。
- 読み手: フックの物理位置判定 `scan_to_pos(config.keyboard_model, scan)`（`hook.rs:198`, キャッシュ `:637-650`, 設定 `hook.rs:702`）、`.yab` パースの列数上限（`bootstrap.rs:249`, `:946-992`）、エンジンへ（`runtime/mod.rs:2088`）、bug_report の診断項目（`bug_report.rs:559,703`、`message_handlers.rs:1379,1762`）。
- GUI: コンボボックスとレイアウト連動（`settings/main.rs:2549-2640`）。JIS↔US 切替で親指キー・ホットキー既定を入れ替える。
- 検証: `validate_keyboard_model`（`config.rs:1241-1300`）。文書: `README.md:28,122`、`README.en.md`、`docs/usage.html:720`、`config.toml` のコメント、`docs/design/settings-gui.md:32`。
- 自動検出は無い。`check_keyboard_layout_on_change`（`app/mod.rs:472`）は入力言語（HKL）が日本語でないと警告するだけで、`keyboard_model` は決めない。

### (2)〜(4) 代替・撤去・IME

- ADR-195/196/199/201・学習表・`config1.db`・レジストリは **IME のキー効果**を扱う。`keyboard_model` は「物理キーボードが JIS か US か」で、キー位置（同時打鍵の判定）と `.yab` の列数を決める。学習・逆算では代替できない。A4 の前提（学習が完成すれば不要）が当てはまらない。
- 撤去するなら自動検出（`GetKeyboardType` 等）が前提になるが、入力言語（HKL）とハードウェア配列は別で、VM・RDP・外付けキーボードで誤る恐れがある。撤去の利益（設定1つ）に比べ、US ユーザー（README で明示対応）を壊すリスクが大きい。
- IME 別の差は無い（IME 非依存）。

### (5) 再発ファミリー

該当しない（キー選択・belief の表に無い）。現状維持なので回帰テストの追加は不要。

---

## 4. 類する設定（参考。A4 の指定外）

| 設定 | 状況 | 所見 |
|---|---|---|
| `keys.engine_on_ime_key` / `engine_off_ime_key` | 既定 `None`（ADR-092 決定D Step1）。GUI 無し。消費者あり（`bootstrap.rs:721-727` → `platform.rs:1353` で SendInput） | エンジン ON/OFF 時の **能動的な IME モードキー送信**。ADR-199 の「受動が原則」に反する残骸。使用実績が分からない。撤去候補だが actuation 合流点に触れるため別調査（Q4） |
| `muhenkan_solo_tap_dedicated_fn_key` | ADR-091 D3.2。`config_diagnostics` の解決対象、bug_report に有無を出力（`bug_report.rs:261`） | 学習で置き換わらない（GJI の SwitchKanaType を Fn キーへ割り当てる用途）。残す |
| `*_solo_tap_always_suppress` / `ignore_composing_guard` | `ModeKeyConfig` の legacy bool（`runtime/mod.rs:1467-1483`）。ADR-192 の警告判定の入力 | 残す（A4 の範囲外。ADR-092 決定B の総関数へ寄せるのは別件） |
| `use_learned_keymap_table` | ADR-195 M-b の opt-out | 残す（学習表を切る唯一の手段） |

---

## 実装の分割案（PR 単位）

前提: **#366（ConfirmMode）が `src/config.rs`・`config_save.rs`・`settings/main.rs`・`config.toml`・README・usage を編集済み**。A4 の config.rs 編集は #366 のマージ後に develop から切る。

| PR | 内容 | 触るファイル | 条件 |
|---|---|---|---|
| A4-0（推奨、今すぐ可） | 文書の整理のみ: `docs/design/settings-gui.md` の撤去済み「タブ3 IME 検出」節の更新、`ime_detect` の説明を「ATOK 本体・未検出・MS-IME 互換モード向けの手動指定」と明確化（`config.toml:27-34`・`usage*.html`・README の該当行） | docs、`config.toml`（コメントのみ）。例ブロックの中身は変えない | #366 と `config.toml`・usage が重なるので、#366 マージ後に。挙動変更なし |
| A4-1（Q1 で「空にする」を選んだ場合のみ） | `ImeDetectConfig::default()` の `on`/`off` を空に | `src/config.rs:519-543`、`vk.rs:1318`、golden/journal テスト、docs | キー選択・belief ファミリー。回帰テスト必須（上記(5)） |
| A4-2（Q2 で「撤去」を選んだ場合） | 1) 実機 A/B と ADR（ADR-153 を supersede する新 ADR）→ 2) GUI の案内を bare 書き込みのみに変更 → 3) 読込警告つきで値を読み続ける期間 → 4) 削除と `REMOVED_KEYS` 登録 | 「2(3)」に列挙 | 大きい。v2.0 に間に合わせるより v2.x が現実的。1)〜4) は別 PR |
| A4-3 | `keyboard_model` は変更なし | — | — |

`config_load_diag.rs`・`config_diagnostics.rs` の変更は A4-2 の最後の PR に集約する（撤去済みキー表を複数 PR で触らない）。

## 所有者に決めてほしい論点

- **Q1**: `keys.ime_detect` の既定 `on=["IMEオン"]`/`off=["IMEオフ"]` を空にするか。静的な `VK_IME_ON/OFF` の追随（`shadow_effect`）と重複するが、`is_japanese_ime()` 偽のときの追随と意図の種別が変わる。推奨は **空にしない**（削る利益が小さく、キー選択ファミリーの回帰リスクがある）。
- **Q2**: `*_solo_tap_ime_action` の扱い。(a) v2.0 は残し文書で「非推奨・`keys.ime_on/off` の bare 指定へ」と案内、(b) 実機 A/B のうえ v2.0 で読込警告つきの移行期間に入る、(c) v2.x 以降に撤去。推奨は (a) から (c)。A/B は Windows Terminal + GJI（無変換を IME Off 系に割り当て）とエンジン OFF の両方で、これは実機が要る。
- **Q3**: `keyboard_model` の自動検出（`GetKeyboardType`）を既定にするか。推奨は **しない**（設定は残す）。
- **Q4**: `engine_on/off_ime_key` を A4 に含めるか。含めるなら使用実績（不具合報告に載せる、または警告ログ）を先に取る。
- **Q5**: 撤去した設定が既存 config.toml に残っていたときの扱い。効果のあった設定は **1回限りの移行警告**、死んだ設定は `REMOVED_KEYS` で無警告、を推奨する（ADR-201 決定2 の方針と整合）。

## 調べて分かったこと／未確認

- `ime_detect` の既定が静的追随と重複していること、`sync_direction` が `shadow_action` より優先され `is_japanese_ime()` に依らないことは、`key_pipeline.rs:1172-1186` から確認した。その結果として起きる実機での差は未確認。
- `*_solo_tap_ime_action` と `forced_open_action` の「@」抑止の同等性は **未検証**。撤去判断の最大の不確定要素。
- ビルド・テストは実行していない（ディスク逼迫のため）。ここに書いたテスト名・行番号は読んで確認したもの。
