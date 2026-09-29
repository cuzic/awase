---
id: ADR-207
title: |-
  `keys.ime_detect.{on,off}` の既定を空にし、`keys.engine_on_ime_key`/`engine_off_ime_key`(エンジン ON/OFF 時の IME モードキー能動送信)を撤去する
summary: |-
  v2 A4(設定項目整理、棚卸し docs/tasks/v2-a4-config-cleanup-inventory-2026-09-29.md)への所有者決定(2026-09-29)の実装方針。
  (1) `ime_detect.on/off` の既定 `IMEオン`/`IMEオフ`(VK 0x16/0x1A)は、hook の静的 `shadow_effect`(同じ 0x16→TurnOn、0x1A→TurnOff)と同方向で、
  IME 種別(GJI/ATOK/MS-IME/未同定)に依らず `shadow_action` 経路が同じ belief 追随を担う。差は `is_japanese_ime()` が偽のときだけ
  (キーボードレイアウトが日本語でないスレッド)で、その場面で日本語 IME の開閉キーを追随する意味は無い。よって代替の無い IME/構成は見つからず、既定を空にしてよい。
  明示値は尊重する(既定だけ変える)。
  (2) `engine_on_ime_key`/`engine_off_ime_key` は既定 None(ADR-092 決定D Step1)・GUI ウィジェット無し・docs は「上級者向け」と注記済みの残骸。
  Engine の ON/OFF 遷移で `send_ime_mode_key` を SendInput する唯一の経路で、撤去すると `send_engine_state_ime_key`・`suppress_engine_state_key` ガード・
  `UiEffect::EngineStateChanged.send_ime_key`・`Output::on_ime_mode_vk_sent` が連鎖して不要になる(すべて削除)。
  既存 config.toml に値が残る場合は「撤去された・無視する」旨を読込時に警告する(無警告の `REMOVED_KEYS` とは別の表 `REMOVED_WITH_NOTICE`)。
status: |-
  起草(2026-09-29)。opus-adversarial-consult にて収束判定中。
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
| 採用条件 | `is_japanese_ime()` に依らない | `is_japanese_ime()` が真のときだけ(`key_pipeline.rs:1174`) | 下記「唯一の差」 |
| `may_change_ime` / `is_ime_mode_key` | 立つ | hook の分類で元から立つ(`vk.rs::may_change_ime`・`is_ime_mode_key_for_ime` は 0x15-0x1A) | 差なし |
| 物理配送(`transport.rs::plan`) | `shadow_action` は静的にも Some | 同左 | 差なし(`sync_direction` を条件にする箇所は `transport.rs:1449` のテスト用 fixture だけ) |
| GjiFsm の Reopen(ADR-203 ii) | ON 系で発火 | 同じ `kp_stage_shadow_ime_toggle` の同じ枝 | 差なし |
| モードキー追随(`kp_stage_mode_key_follow`)・キー効果追跡(`kp_stage_key_effect_track`) | `shadow_action.is_some() \|\| sync_direction.is_some()` で除外 | 同左(`shadow_action` 側で除外) | 差なし |
| F13〜F24 の役割判定(`passive_without_lookup`) | 0x16/0x1A は対象外(`enrich_key_role` は `is_fkey`/半角全角/0x19 だけ) | 同左 | 差なし |
| Engine の特殊キー照合(`engine.rs:951,1132`) | `sync_direction.is_some()` なら `ime_on/off/toggle` コンボと `match_ime_toggle_auto` を**素通し**(二重処理防止) | 素通ししない | 下記「二重処理の余地」 |
| 診断・bug_report・GUI | `config_diagnostics`/`config_key_resolution_tests` が値を解決確認するだけ。bug_report に出ない。GUI に編集ウィジェット無し(`settings/main.rs:2806` で撤去済み)。JIS 切替時に GUI が書く値も無い | 同左(値が空なら何も解決しない) | 差なし |

**唯一の差**: `is_japanese_ime()` が偽のとき。値は `read_ime_state_fast`/`read_ime_state_full` の `lang_id == LANGID_JAPANESE`(フォーカススレッドの HKL の言語、
`ime.rs:553-558,837`)で、**IME の種別(GJI/ATOK/MS-IME/互換モード/未同定)とは無関係**。したがって棚卸しが懸念した
「ATOK・IME未同定・MS-IME互換モードで追随が失われる」は、0x16/0x1A については起きない(`table_ime_kind()` が `None` でも静的な `shadow_action` は付く。
`enrich_key_role` が `table_ime_kind()?` で止まるのは F13〜F24 と 0xF3/0xF4 の役割判定であって、0x16/0x1A の静的分類ではない)。
偽になるのは英語配列などのスレッドにフォーカスがあるときで、その場面の日本語 IME 開閉キーを belief に反映しても意味が無い。
grace 期間の誤答(`key_pipeline.rs:2773-2778` は grace 中は true→false の降格をしない)は、降格を抑える側なので、追随が失われる方向には働かない。

**二重処理の余地**: ユーザーが `keys.ime_on`/`ime_off` に無修飾の `IMEオン`/`IMEオフ` を書いていると、現状は sync 既定の素通しで Engine の照合が止まり、belief 追随だけになる。
空にすると Engine の照合が有効になり `SetOpen` が発行される。方向固定(on/off)の二重処理は冪等(同じ値を書く)。`keys.ime_toggle` に 0x16/0x1A を書く構成は意味を持たない。
既定の `ime_on = ["Ctrl+変換"]`、`ime_off = ["Ctrl+無変換"]`、`ime_toggle = []` は 0x16/0x1A と重ならない。

**結論(1)**: 0x16/0x1A の追随に代替の無い IME/構成は見つからない。既定を空にしてよい。
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
  それ以前に GUI で保存したユーザーの config.toml には旧既定 `VK_DBE_DBCSCHAR`/`VK_DBE_SBCSCHAR` が残っている可能性がある(要注意: この人たちには警告が出る)。
- 再発ファミリー: キー選択(`ime_controller.rs`/`output/vk_send.rs`)ではなく、`platform.rs` の force-write/actuation ターゲットに近い。
  `lints/actuation_call_guard`(`send_input_safe` 等の許可呼び出し元)・`architecture_guard.rs`・`ime_key_sequence_golden.rs`(`ime_controller.rs::characterize_strategy` の戦略/送信列)は
  `send_engine_state_ime_key`・`engine_on_ime_vk` を参照していない(grep 済み)ので、撤去で更新すべき許可リスト・必須トークンは無い。
  `send_ime_mode_key` は `ime_controller.rs`(GjiDirect/MsImeDirect)が引き続き使う。
- 連鎖して不要になるもの(すべて削除): `WindowsPlatform::{engine_on_ime_vk, engine_off_ime_vk, suppress_engine_state_key}`・`SuppressEngineStateKeyGuard`・
  `Runtime::execute_decision_suppressed`(ガードを立てるだけのラッパー、呼び出し元4箇所は `execute_decision` へ)・`PlatformRuntime::send_engine_state_ime_key`・
  `executor.rs` の `applied_for_engine_key`・`Output::on_ime_mode_vk_sent`(唯一の呼び出し元)・`UiEffect::EngineStateChanged.send_ime_key`(コア `awase` の型。
  `engine.rs::transition_activation` の `suppress_ime_key` 変数も消える。`SetOpen` の抑止条件 `suppress_set_open` は残る)。
  `win32.rs`/`vk.rs`/`config.rs` のコメントにある「ユーザー設定 VK を送る」旨の記述も直す。

## 決定

1. `ImeDetectConfig::default()` の `on`/`off` を空にする。`toggle` は元から空。
2. `KeysConfig` から `engine_on_ime_key`/`engine_off_ime_key` を削除し、上記の連鎖分を削除する。`config_diagnostics.rs`・`config_key_resolution_tests.rs` の対応行も削除。
3. 既存 config.toml に `keys.engine_on_ime_key`/`engine_off_ime_key` が残っていた場合は、読込時に警告して無視する(効果があった設定を黙って消さない。ADR-201 決定2 の方針)。
   既存の `REMOVED_KEYS`(無警告で許容する表)は「効果の無い死んだ設定」用なので使わず、`config_load_diag.rs` に `REMOVED_WITH_NOTICE`(パス→警告文)を足す。
   `from_toml_str` は、この表に載っているパスは未知キー警告ではなく専用文を `load_warnings` に積む。文言は「`keys.engine_on_ime_key` は v2 で撤去されました。値は無視されます
   (エンジン ON/OFF 時に IME モードキーを送る機能は無くなりました。IME の開閉は `keys.ime_on`/`ime_off` を使ってください)」。
4. 文書: `docs/usage.html`/`usage.en.html` の注記を「撤去済み」に更新、`docs/usage*.html` の `ime_detect` 例ブロックは中身を変えず説明を追随、`config.toml` のコメント(`ime_detect` の説明)を新既定に合わせる。
   `docs/design/settings-gui.md` の撤去済み「IME 検出」タブ記述は別件(A4-0)で触らない。

## 検証

- ホストで走る単体テスト: `config.rs` の既定テスト(`ime_detect.on/off` が空、明示値は尊重、`engine_*_ime_key` が無い)、撤去キーが警告つきで無視され他の設定が読める(`test_removed_*` 系)、
  `config_load_diag` の表テスト、コアの `engine/tests.rs`(`EngineStateChanged` の `send_ime_key` 参照2件の更新)。
- Windows ビルドの確認: `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings --tests`。`vk.rs::keys_defaults_do_not_collide_with_ime_detect_defaults` は
  空でも通る(意味を持たなくなるが、明示値の衝突検査として残す)。
- 実機/CI での確認は 1 回だけ: `ime_detect` を空にした状態で VK_IME_ON/OFF(0x16/0x1A)の物理打鍵で belief が追随すること(e2e-ime の該当シナリオ)。

## 未検証・残る論点

- `ime_detect` を空にした後の 0x16/0x1A 追随は、コードの静的な読み(上表)による。実機/CI での確認は上記1回のみ。
- `is_japanese_ime()==false` のスレッドでの挙動差は仕様上の差として受け入れる。
- 撤去した `engine_*_ime_key` の使用ユーザーがいた場合、Engine ON/OFF 時のモード強制が無くなる。読込警告で通知する。追加の救済策は用意しない。
