---
id: ADR-207
title: |-
  `keys.ime_detect.{on,off}` の既定を空にし、`keys.engine_on_ime_key`/`engine_off_ime_key`(エンジン ON/OFF 時の IME モードキー能動送信)を撤去する
summary: |-
  v2 A4(設定項目整理、棚卸し docs/tasks/v2-a4-config-cleanup-inventory-2026-09-29.md)への所有者決定(2026-09-29)の実装方針。
  (1) `ime_detect.on/off` の既定 `IMEオン`/`IMEオフ`(VK 0x16/0x1A)は hook の静的 `shadow_effect` と同方向で、IME 種別に依らず `shadow_action` 経路が同じ belief 追随を担う。
  ただし `shadow_action` の採用は `is_japanese_ime()` に依り、それは awase のワーカースレッドの HKL 由来で偽になりうる(MS-IME + en-US 既定の環境など)。
  そのため 0x16/0x1A の静的 `shadow_action` だけは `is_japanese_ime()` を問わず採用する小修正を同時に入れることを条件に、既定を空にする。明示値は尊重する。
  (2) `engine_on_ime_key`/`engine_off_ime_key` は Engine の ON/OFF 遷移で `send_ime_mode_key` を SendInput する残骸(既定 None、GUI 無し)。撤去し、
  `send_engine_state_ime_key`・`suppress_engine_state_key` ガード・`EngineStateChanged.send_ime_key`・`on_ime_mode_vk_sent` を連鎖削除する
  (`applied_snapshot` の楽観更新と `uses_kanji_toggle` は他の消費者/テストがあるので残す)。旧 config に値が残る場合はトレイ警告で通知し(無警告の `REMOVED_KEYS` とは別の `REMOVED_WITH_NOTICE`)、
  設定の保存(`save_edit` を通る全ての保存)で撤去キーを削除する。
status: |-
  採択・実装済み(2026-09-29)。opus-adversarial-consult round1〜2 で収束(round2 の必須指摘は本文の整合のみで、反映済み)。実機/CI での確認(MS-IME ジョブ、フォーカス変更直後の物理 0x16/0x1A)は未実施。
related_adr:
  - "ADR-092"
  - "ADR-199"
  - "ADR-201"
  - "ADR-133"
  - "ADR-153"
---

# ADR-207: `keys.ime_detect` 既定を空に、`engine_on/off_ime_key` を撤去する

## 背景

所有者決定(2026-09-29)で A4 の一部を実施する。棚卸し(PR #368)は当初「`ime_detect` の既定を空にしない」「`engine_*_ime_key` は別調査」と推奨したが、
所有者が「(1) `ime_detect.on/off` の既定を空にする」「(2) `engine_on_ime_key`/`engine_off_ime_key` を撤去する」と決めた。
本 ADR は、決定の前提になる「失われる挙動が本当に無いか」の調査結果と、実施方法を残す(行番号は `f2eb36f3`)。

## 調査 1: `ime_detect` の既定 `on=["IMEオン"]`/`off=["IMEオフ"]` を空にすると何が変わるか

`IMEオン`/`IMEオフ` は VK 0x16/0x1A(`vk.rs`)。`FocusTracker::enrich_ime_relevance`(`runtime/focus_tracker.rs:55-75`)が打鍵に `sync_direction` を付け、
`kp_stage_shadow_ime_toggle`(`runtime/key_pipeline.rs:1172-1186`)が **`sync_direction` を `shadow_action` より先に**採用する。

空にした後に同じ打鍵を扱うのは hook の静的 `classify_ime_relevance`(`hook.rs`)が付ける `shadow_action`(`ImeKeyKind::shadow_effect`、`vk.rs:146-160`:
0x16→TurnOn、0x1A→TurnOff)。全経路での差:

| 経路 | sync 既定あり(現状) | 空(`shadow_action` のみ) | 差 |
| --- | --- | --- | --- |
| belief の書き込み(`write_sync_key` / `write_physical_key`) | `UserImeSetIntent{source: SyncKey}` | 同 `{source: PhysicalImeKey}` | 呼び出す `dispatch_event`・`record_explicit_intent` は同一。`last_user_explicit_off_ms`(`platform_state.rs:157-160`)・`hwnd_cache` の `from_explicit_off_intent`(`focus_tracking.rs:429-440`)・drift 補正(`drift_correction.rs:56`)は両ソースを同列に扱う。**差なし** |
| 採用条件 | `is_japanese_ime()` に依らない | 0x16/0x1A は `is_japanese_ime()` に依らず採用する(下記「対策」)。それ以外は真のときだけ | 対策で差なし |
| `may_change_ime` / `is_ime_mode_key` | 立つ | hook の分類で元から立つ(`vk.rs::may_change_ime`・`is_ime_mode_key_for_ime` は 0x15-0x1A) | 差なし |
| 物理配送(`transport.rs::plan`・KeyUp ラッチ `kp_latch_keyup_to_keydown_disposition`) | `shadow_toggled` が立つ | 対策なしでは偽の窓で立たない。対策で同じ | 対策で差なし(`sync_direction` を条件にする箇所は `transport.rs:1449` のテスト fixture だけ) |
| GjiFsm の Reopen(ADR-203 ii) | ON 系で発火 | 同じ `kp_stage_shadow_ime_toggle` の同じ枝 | 差なし |
| モードキー追随(`kp_stage_mode_key_follow`)・キー効果追跡(`kp_stage_key_effect_track`) | `shadow_action.is_some() \|\| sync_direction.is_some()` で除外 | 同左(`shadow_action` 側で除外) | 差なし |
| F13〜F24 の役割判定(`passive_without_lookup`) | 0x16/0x1A は対象外(`enrich_key_role` は `is_fkey`/半角全角/0x19 だけ) | 同左 | 差なし |
| Engine の特殊キー照合(`engine.rs:951,1132`) | `sync_direction.is_some()` なら `ime_on/off/toggle` コンボと `match_ime_toggle_auto` を**素通し**(二重処理防止) | 素通ししない | 下記「二重処理の余地」 |
| 診断・bug_report・GUI | `config_diagnostics`/`config_key_resolution_tests` が値を解決確認するだけ。bug_report に出ない。GUI に編集ウィジェット無し(`settings/main.rs:2806` で撤去済み)。JIS 切替時に GUI が書く値も無い | 同左(値が空なら何も解決しない) | 差なし |

**差(Opus round1 で訂正)**: `is_japanese_ime()` が偽のとき。当初「フォーカススレッドの HKL で、偽になるのは英語配列の窓だけ」と書いたが誤りだった。
`read_ime_state_fast`(`ime.rs:836-843`)は `keyboard_layout_info()`(`GetKeyboardLayout(0)` = **呼び出したスレッド自身**の HKL、`ime.rs:813-820`)を使い、
`read_ime_state_fast_async` が `offload_unsafe` の**ワーカースレッド**で実行する(`ime.rs:648-651`)。`read_ime_state_full` も `get_gui_thread_info_with_timeout` 失敗時は自スレッドの HKL になる。
結果はフォーカス変更ごとに `set_is_japanese_ime`(`key_pipeline.rs:2773-2778`)へ入るので、既定入力言語が en-US で ja-JP + MS-IME を追加した環境(CI の MS-IME ジョブ、`e2e-ime.yml:488-510` がまさにこれ)では、
フォーカス変更後に偽になりうる。降格を抑える grace(`compute_focus_probe_grace`)は warmup 直後/GJI の I/O 後だけで、MS-IME には無い。
救済の ADR-093(`should_upgrade_is_japanese_ime`)は 0x16/0x1A を意図的に対象外にしている(`vk.rs:295-303`)。
つまり現状、偽の窓で 0x16/0x1A を追随しているのは sync 既定だけである。偽の窓で対策なしに既定を空にすると:
(a) belief に書かれない(`intent_kind=None`)、(c) 明示意図(`last_intent`・`last_user_explicit_off_ms`・hwnd_cache の `from_explicit_off_intent`)が記録されず、
真に戻った後に drift 補正が逆向きに働きうる。(b) 物理配送は `shadow_toggled` に依存するので、表の「差なし」は対策なしでは誤りだった
(KeyUp は BUG-173 追補のラッチ `kp_latch_keyup_to_keydown_disposition` が最初の KeyDown にそろえるため、Down/Up の非対称は起きない。
ImmCross の偽の窓では Engine が inactive で `SetOpen` が出ず、現状でもキーが Suppress されて実 IME が動かないのは sync 既定の有無に依らない既存の問題で、本 ADR の範囲外)。
失われるのは (a)(c) で、対策でどちらも戻る。

**対策(決定1に含める)**: `kp_stage_shadow_ime_toggle` の `intent_kind` 判定で、hook が付けた静的 `ImeOn`/`ImeOff`(0x16/0x1A)の `shadow_action` だけは `is_japanese_ime()` を問わず採用する。
この2キーはどの IME でも冪等(`vk.rs:143-146`)なので、`sync_direction` と同じ扱いでよい。0x19(Toggle)・役割由来の F13〜F24・0xF3/0xF4 は現行どおり `is_japanese_ime()` で絞る。
判定は純粋関数 `vk::is_static_idempotent_open_key(vk)`(0x16/0x1A だけ真)に切り出してホストで単体テストし、`kp_stage_shadow_ime_toggle` では
`sync_direction` の直後・`is_japanese_ime()` 分岐の**前**に `is_static_idempotent_open_key(vk) && shadow_action.is_some()` で採用する(`.or_else(explicit…)` の優先順位を変えないため)。
これで sync 既定を空にしても上表の全行が等価になる(`SyncKey` と `PhysicalImeKey` で分岐する本番コードは無くラベルのみの差)。
副作用(改善側): `ime_detect` に IMEオン を含めない明示値(`usage.html:781` の `on=["VK_DBE_HIRAGANA"]` など)を書いたユーザーも、偽の窓で 0x16/0x1A を追随するようになる。

**二重処理の余地**: ユーザーが `keys.ime_on`/`ime_off` に無修飾の `IMEオン`/`IMEオフ` を書いていると、現状は sync 既定の素通しで Engine の照合が止まり、belief 追随だけになる。
空にすると Engine の照合が有効になり `SetOpen` が発行される。方向固定(on/off)は、Engine の `SetOpen` が比較するのは OS 確認済みの `applied_snapshot` であり belief ではない(`executor.rs:762`)ため early exit しにくいが、`applied_snapshot` は `Optimistic` や古い `Confirmed` でありうる。applied が古い drift 中に明示設定の `ime_on=["IMEオン"]` を押すと、現状は Down が Allow されて OS が開き直すのに対し、変更後は Engine が消費して `already_matched` で送信しない場合がある(明示設定時のみの狭いケース。受け入れる。コード読みのみ、未実機検証)。
`keys.ime_toggle` に無修飾 0x16/0x1A を書く構成は、静的 TurnOn の後に Engine の Toggle が反転して消費するので押しても動かない(`engine.rs:1118-1131` のガードが防いでいた不具合と同型。
0x19 と `ime_toggle=["漢字"]` の組み合わせは静的 `shadow_action` に明示設定との重なり除外が無いため既に同じ状態)。ADR-199 で `ime_toggle` は任意キー指定になったが、開閉の方向が固定のキーをトグルに割り当てる構成は意味がなく、
本 ADR では対処しない(Engine 側ガードの拡張は 0x19 を含む別件)。また `sync_direction` は修飾キーを見ずに VK だけで立つ(`focus_tracker.rs:57-76`)ため、現状は Ctrl+IMEオン等も Engine 照合から外れていた。
空にすると修飾付きコンボも照合される。
既定の `ime_on = ["Ctrl+変換"]`、`ime_off = ["Ctrl+無変換"]`、`ime_toggle = []` は 0x16/0x1A と重ならない。

**結論(1)**: 0x16/0x1A の追随に代替の無い IME/構成は見つからない。上記「対策」を同時に入れることを条件に、既定を空にしてよい。
`ime_detect` 自体(`toggle`・`on`・`off` の3項目)は ADR-199 決定9・12 のとおり存続し、ユーザーが明示した値は既定と無関係に尊重される
(構造体単位の `#[serde(default)]` なので、`[keys.ime_detect] toggle = [...]` だけ書いた既存 config では `on`/`off` は空になるが、上記のとおり静的経路が同じ追随を担う)。
`AppConfig::save` は全フィールドを明示出力するため、GUI で一度保存した既存ユーザーの config.toml には `on = ["IMEオン"]`/`off = ["IMEオフ"]` が残り、従来どおり動く(害は無い)。

## 調査 2: `engine_on_ime_key`/`engine_off_ime_key`

- 定義 `KeysConfig`(`src/config.rs:590-606`)。既定 `None`(ADR-092 決定D Step1、2026-08-15)。GUI ウィジェット無し(設定 GUI は `apply_confirmed` で保持するだけ)。
  同梱 `config.toml` に記載無し。docs は `usage.html:678`・`usage.en.html:654` の「上級者向け・既定で無効」の注記だけ。README に無し。
- 消費: `app/bootstrap.rs:721-727` が名前を VK に解決(解決失敗は診断に流す、ADR-201 決定2(c))→ `WindowsPlatform::{engine_on_ime_vk, engine_off_ime_vk}`
  → `platform.rs:1320-1363 send_engine_state_ime_key` が `crate::ime::send_ime_mode_key(vk)` で SendInput。
  呼び出しは `executor.rs:734-741`(`UiEffect::EngineStateChanged{ send_ime_key: true }`)だけ。
  送信条件: 抑止ガード(`suppress_engine_state_key`)が偽、`applied != enabled`(`apply_ime_open` が既に揃えていない)、プロファイルが `uses_kanji_toggle()` でない。
  つまり「Engine の状態は変わったが IME の開閉は変わらない」場合だけ、ユーザー指定の VK を送る。
- 失われる挙動: 設定した上級者について、Engine ON/OFF の切り替えで IME のモード(全角/半角)を、ユーザー指定 VK で追加強制する機能。
  ADR-092 は「ADR-091 決定1(open 軸は `VK_IME_ON`/`VK_IME_OFF`)より前の機構の残骸」と位置づけ、ADR-199 の「受動が原則」(awase が IME を能動的に動かさない)とも矛盾する。
  ADR-133 表の呼び出し元 #3・ADR-175 の「自己注入フィルタが唯一の防御」の記述も、この機構が無ければ不要になる。
- 使用実績: 不明(bug_report に出力する項目が無い)。既定値を 2026-08-15 に None へ変えたが `AppConfig::save` が全フィールドを出力するため、
  それ以前に GUI で保存したユーザーの config.toml には旧既定 `VK_DBE_DBCSCHAR`/`VK_DBE_SBCSCHAR` が残っていて、機能は今も有効なはず(`e4cd0497`)。この層は変更で Engine ON/OFF 時の 0xF4/0xF3 送信が止まる。
- 再発ファミリー: キー選択(`ime_controller.rs`/`output/vk_send.rs`)ではなく、`platform.rs` の force-write/actuation ターゲットに近い。
  `lints/actuation_call_guard`(`send_input_safe` 等の許可呼び出し元)・`architecture_guard.rs`・`ime_key_sequence_golden.rs`(`ime_controller.rs::characterize_strategy` の戦略/送信列)は
  `send_engine_state_ime_key`・`engine_on_ime_vk` を参照していない(grep 済み)ので、撤去で更新すべき許可リスト・必須トークンは無い。
  `send_ime_mode_key` は `ime_controller.rs`(GjiDirect/MsImeDirect)が引き続き使う。
- 連鎖して不要になるもの: 決定2に一覧する(`applied_snapshot` の楽観更新と `uses_kanji_toggle` は消費者/テストがあるので残す)。

## 決定

1. `ImeDetectConfig::default()` の `on`/`off` を空にする(`toggle` は元から空)。あわせて上記「対策」の `vk::is_static_idempotent_open_key` を `kp_stage_shadow_ime_toggle` に入れる。
2. `KeysConfig` から `engine_on_ime_key`/`engine_off_ime_key` を削除し、連鎖分を削除する。連鎖の完全な一覧(Opus round1 で漏れを補った):
   `config.rs`(フィールド・既定・既定テスト2件)、`config_diagnostics.rs`・`config_key_resolution_tests.rs` の対応行、`bootstrap.rs` の `resolve_ime_key` クロージャと `engine_on/off_ime_vk`、
   `platform.rs` の `engine_on_ime_vk`/`engine_off_ime_vk`/`suppress_engine_state_key`/`SuppressEngineStateKeyGuard`/`send_engine_state_ime_key`(実装)、`src/platform.rs:447` のトレイト既定実装、
   `Runtime::execute_decision_suppressed`(呼び出し元 `runtime/mod.rs:1364`・`message_handlers.rs:125`・`ime_refresh.rs:334,1097` は `execute_decision` へ)、`executor.rs` の `applied_for_engine_key`、
   `Output::on_ime_mode_vk_sent`(`output/mod.rs:901-911`)、コア `UiEffect::EngineStateChanged.send_ime_key`(`decision.rs:79-81,357`、`engine.rs:440-443`)、
   `vk.rs:272-276`・`win32.rs:185`・`config.rs` などの doc コメント、`awase-settings/src/main.rs:7441-7498` のテスト(GUI ウィジェット無しの保持対象を `input_relay_apps`/`keystroke_macro` に絞る)、
   `tests/fixtures/configs/notation_japanese_names.toml:13-14`(`engine_*_ime_key` の行を削除。`FIXTURE_BASELINE` の 0 件が保たれる)。
   **残すもの**: `executor.rs:805-817` の `applied_snapshot = Optimistic(open)` は、`build_ime_control_view`(BUG-113 の `shadow_on` 供給元)と `resolve_warmup_ime_on` も読むので削除しない
   (コメントだけ「`send_engine_state_ime_key` をスキップさせる」から実際の消費者に書き換える。`ime_model.rs:132-145` の `applied_open` の呼び出し元一覧も直す)。
   `focus/class_names.rs::uses_kanji_toggle` は撤去後に本番の呼び出し元が無くなる(テストの oracle のみ)。テストごと削除せず、`current_app_profile()` の分類 API として残す
   (別件の整理対象。消すと `AppImeProfile` のテスト網羅が減る)。
   `SetOpen` の抑止条件 `suppress_set_open`(NotRomajiInput)は残し、`engine/tests.rs` の該当テスト(`SetOpen` が出ないことの assertion)は `send_ime_key` の参照だけ削って残す。
3. 既存 config.toml に `keys.engine_on_ime_key`/`engine_off_ime_key` が残っていた場合は、読込時に警告して無視する。
   - 無警告の `REMOVED_KEYS` は使わず、`config_load_diag.rs` に `REMOVED_WITH_NOTICE`(パス→警告文)を足す。`from_toml_str` の未知キー処理より**先に**判定する
     (`suggest` の接頭辞ルールが `engine_on_ime_key` に `keys.engine_on` を提案し、従うと IMEオン がエンジン ON ホットキーになって有害なため。テストで固定)。
   - `load_warnings` はログだけ(ADR-201 決定2、`app/mod.rs::warn_config`)なので、そこには積まない。`AppConfig` に `#[serde(skip)] removed_notices` を持たせ、`validate()` が
     `load_warnings` の**後ろ**に足す。`load_notes` には含まれないので `warn` 側(トレイ通知、内容が同じなら再表示しない)に流れる。
   - 文言: 「`keys.engine_on_ime_key` は撤去されました。値は無視されます。エンジンの ON/OFF に合わせて IME のモードキーを送る機能は無くなり、代わりの設定はありません。config.toml から削除してください」
     (失われる機能は IME の開閉ではなく文字種モードの強制。旧既定は `VK_DBE_DBCSCHAR`/`VK_DBE_SBCSCHAR`)。
   - `save_edit` を通る全ての保存(設定 GUI の保存、トレイの自動起動切替 `save_auto_start`)は、撤去キーをファイルから削除する(`migrate_legacy_confirm_mode` と同じ位置)。
     GUI は保存成功後に `removed_notices` を落とし、保存直後の `Saved{warnings}` に「削除してください」が残らないようにする(テストで固定)。
   - 影響層: 2026-08-15 の既定 None 化(`e4cd0497`)より前に GUI で一度でも保存したユーザーの config.toml には旧既定が残っていて、機能は今も有効。この人たちは、この変更で
     Engine ON/OFF 時の 0xF4/0xF3 送信が止まる。CHANGELOG(Unreleased)に明記し、トレイ警告で通知する。
4. 文書: `docs/usage.html`/`usage.en.html` の注記を「撤去済み」に更新、`ime_detect` の説明と既定値の記述(`usage.html:780-784` の「既定: ["IMEオン"]」、`config.toml:29-35` の例)を新既定に合わせる。
   `docs/design/settings-gui.md` の撤去済み「IME 検出」タブ記述は別件(A4-0)で触らない。
5. ログの `[shadow-toggle] kind=` は 0x16/0x1A の既定で `SyncKey` から `PhysicalImeKey` に変わる(意図の出所ラベルのみ。書き込み先は同一)。

## 検証

- ホストで走る単体テスト: `config.rs` の既定テスト(`ime_detect.on/off` が空、明示値は尊重)、撤去キーが読み込めて `removed_notices` に文が積まれ `load_warnings` には入らないこと・
  `suggest` より先に判定されること・`validate()` が警告として返すこと、`config_save` が撤去キーを保存時に削除すること、`vk::is_static_idempotent_open_key`(0x16/0x1A だけ真、0x19/0xF3/0xF4/F13〜F24 は偽)、
  コアの `engine/tests.rs`(`EngineStateChanged` の `send_ime_key` 参照2件の更新)。
- Windows ビルドの確認: `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings --tests`。
- 実機/CI での確認は 1 回だけ、`is_japanese_ime` が偽になりうる条件を踏む: MS-IME ジョブ(既定 en-US + ja-JP 追加)で、**フォーカス変更の直後に**物理(`AWASE_TEST_INJECTION=1` の debug ビルド。
  release の SendInput は injected 扱いで BUG-14 により両設定とも空振りになり何も区別できない)0x16/0x1A を押し、ログの focus probe の `is_japanese_ime` を併記して belief 追随を確かめる。
  既存 e2e の準備手順が VK_IME_ON を使う(`e2e-ime.yml:149,168,252`)ので、`invalid`/`NOT_OBSERVED` の件数が前後で増えていないことも見る。

## 未検証・残る論点

- `ime_detect` を空にした後の 0x16/0x1A 追随は、コードの静的な読み(上表)による。実機/CI での確認は上記1回のみ。
- `keys.ime_toggle`/`ime_on`/`ime_off` に無修飾の 0x16/0x1A を書いた構成の Engine 側の二重処理は、上記のとおり冪等性を executor の読みで確認しただけ。
- 撤去した `engine_*_ime_key` の使用ユーザーがいた場合、Engine ON/OFF 時のモード強制が無くなる。読込警告で通知する。追加の救済策は用意しない。

## レビュー記録

- round1(Opus): `is_japanese_ime` の前提の誤り、警告がログのみで不可視、連鎖範囲の漏れ、`suggest` の有害提案 → 反映。
- round2(同じレビュアー): 静的採用の等価性を裏取りで確認。KeyUp 非対称は既存ラッチで防止済みと訂正。本文の旧記述の矛盾、GUI 保存直後の警告 → 反映。
- round3(同じレビュアー、実装 PR #373 対象): **収束**(追加の必須指摘なし)。`kp_stage_shadow_ime_toggle` の分岐、`latch_step`・KeyUp ラッチ・`transport::plan` に副作用なし、
  連鎖削除に漏れ・過剰なし(`actuation_call_guard` 許可リストは更新不要、`applied_snapshot` 楽観更新と `uses_kanji_toggle` を残す判断は妥当)、
  通知の配線・頻度(起動ごとに 1 回、リロードでは同内容なら出ない)は意図どおり。任意の小修正(通知文に「設定画面で保存しても消えます」、CHANGELOG の ADR-201 項からの除外、
  usage の「代わりの設定はありません」)は反映済み。ADR 番号は 205(BUG-172)・206(solo-tap)・208 と衝突なし。

## 追記（2026-10-02）: 撤去した設定の通知と、旧既定値の自動削除

所有者決定（2026-10-02）。「効果があった設定を黙って消さない」を、キーの存在でなく**値が既定でないとき**に広げた。

- `general.gji_thumb_key_ime_toggle = true` と `general.dbe_mode_key_policy` が `suppress` 以外のときだけ、`removed_notices`（トレイ通知）に積む。
  v1 の設定画面は全項目を書き出すので、既定値（`false`・`"suppress"`）はほぼ全員の config.toml に残っており、キーがあるだけで通知すると全員に出る。
  設定画面の保存は、どちらのキーもファイルから消す（`config_save::remove_retired_keys`）。
- v1 の設定画面が書き出した旧既定値（`keys.ime_toggle = ["VK_KANJI"]`、`keys.ime_detect.on = ["IMEオン"]`・`off = ["IMEオフ"]`）と**ちょうど同じ**値は、
  読み込み時に空として扱い（`KeysConfig::drop_retired_default_values`）、保存でファイルからも消す（`config_save::remove_retired_default_values`）。
  旧既定に別のキーを足した値は尊重する。これで、「ユーザーが明示した値は既定と無関係に尊重する」（本文の調査 1）のうち、旧既定と同一の値だけが覆る（v2 の既定が既存ユーザーにも効く）。
  明示した `ime_toggle = ["VK_KANJI"]` は GJI の 0x19 の役割判定（ADR-202）が担うので、動作は変わらない見込み（実機未確認）。
