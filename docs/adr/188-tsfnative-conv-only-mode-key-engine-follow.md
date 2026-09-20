---
id: ADR-188
title: |-
  Chrome等(Imm32Unavailable/TsfNative)で、convだけを変えるモードキー(ひらがな、Shift+無変換)の後にEngineを追随させる
  (モードキー後の遅延conv読み取り、実機A/Bで4案を比較)
summary: |-
  BUG-149: ChromeでGJI(ATOK)のかな→半角英数(ひらがなキー/Shift+無変換)のあと、EngineがOFFにならず英数なのにNICOLAが動く
  (実機6/6失敗、awase停止の対照は24/24正常)。原因は、convを読む経路(`idle-conv-check`)がChrome(Imm32Unavailable)に入らず
  (ガード2が「TsfNativeのみ」)、20ms再読み取りも`SkipTyping`/`Blacklist`で読まないこと。実験4案(E1入口を広げる/E2モードキー後300msの
  強制チェック/E3=E2+Shiftガード中は再試行)を実機で比較し、E3が全ケースで最初の打鍵から正しい唯一の案だった。本ADRは実験の設計を
  実装に落とす前のレビュー対象で、フィールドの積み増しを最小にする形を探す。
status: |-
  **ドラフト(実験のみ、未実装)**。レビュー対象。実験パッチ: `188-measurements/e3-experimental.patch`(実験用、そのまま採用しない)。
related_adr:
  - "ADR-186"
  - "ADR-187"
  - "ADR-179"
---

# ADR-188: convだけを変えるモードキーの後にEngineを追随させる(TsfNative/Imm32Unavailable)

## 問題(BUG-149、実機再現)

`chrome_probe`(`spike/ime-key-matrix`の`crates/awase-windows/examples/chrome_probe.rs`、専用プロファイルのChromeで`k`,`a`を打って
出た文字でEngine/IMEの状態を判定)で、awase起動中(ADR-186実装+Shift修正入り)のChromeの8ケース×3周:

- 無変換/変換のON/OFFとShift+無変換のOFF中は18/18 PASS。
- **かな→半角英数(ひらがなキー0xF2、Shift+無変換)は6/6失敗**: IMEは半角英数になるがEngineがONのまま、`k`,`a`が`kiu`になる。
- awase停止の対照は24/24 PASS(GJI自身はChromeで正しく動く)。待ち2秒でも直らない。

## 原因(ソースとログで特定)

1. `idle-conv-check`(`runtime/key_pipeline.rs::kp_stage_idle_conv_check_inner`、次の打鍵で変換モードを読む唯一の機構)の入口
   `should_run_idle_conv_check`(`src/engine/idle_check.rs`)のガード2が`is_tsf_native`。Chromeは`profile=Imm32Unavailable`で、
   `AppImeProfile::is_effectively_tsf_native`は`is_tsf_native_window(class_name)`(WezTerm/Windows Terminal/XAML系の5クラスのみ)に
   委ねるため、Chrome(`Chrome_WidgetWin_1`)は黙ってfalse(ログ無し)。検証ログで`[idle-conv-check]`は0件。
2. モードキー押下の20ms後の再読み取り(`TIMER_IME_REFRESH`→`ir_decide_read_strategy`)は、押したキー自身が「入力中」を成立させ
   `SkipTyping`、入力中でなくても`skip_imm_query`のプロファイルは`Blacklist`(OsPollしない)。
3. Chromeのconvは読める(`[cold-diag] pre-send conv=0x00000019`、`WM_IME_CONTROL`経由)。読みに行く経路が無いだけ。

## 実験(実機、Chrome、awase起動、`188-measurements/chrome-ablation-summary.md`、`ablations/a8〜a10`)

| 構成 | 内容 | 8ケース×3周 | Shift 700ms長押し |
|---|---|---|---|
| base | 変更なし | 18 PASS、6 FAIL(ケース3・7が3/3失敗) | ― |
| E1 | ガード2を`cannot_verify_real_ime_state`に広げる(1行) | 18 PASS、**6 RECOVER**(最初の打鍵は誤り、次の打鍵で追随) | ― |
| E2-150ms | モードキー(KeyDown、物理)の150ms後にガード3/4/5を無視して`idle-conv-check`を1回強制 | 21 PASS、3 RECOVER(ケース7が回復のみ) | ― |
| E2-300ms | 同、300ms後 | 24/24 PASS | 14 PASS、**2 RECOVER**(Shift長押しで凍結) |
| **E3-300ms** | E2 + 強制チェックが`half_width_alnum`ガード中なら100msごとに再試行(最大30回) | **24/24 PASS** | **16/16 PASS** |

- E1は、次の打鍵で読む方式なので最初の1〜2文字が誤り。
- E2でShift長押しが負けるのは、強制チェックが`kp_stage_shift_conv_guard`(`half_width_alnum.is_guard_pending()/is_toggle_active()`)で
  凍結されるため(E3の再試行ログがShift700msで10回、40msで0回)。
- Win32 EDITの通常10手順の回帰(倍速12回、Engine判定含む): E2-300ms 12/12 PASS。E3は実行中(結果が出たら追記)。
- 強制読み取りは、物理のモードキー1押下につき1回(24ケースで66回)。

## 実験の実装(`188-measurements/e3-experimental.patch`、採用形ではない)

- `lib.rs`に`TIMER_MODEKEY_CONV = 110`。`GateStore`に`pending_modekey_event: Option<RawKeyEvent>`・`force_conv_check: bool`・
  `modekey_retries: u8`の3フィールド。
- `kp_stage_idle_conv_check`(毎キー呼ばれる)で、KeyDown・`is_ime_mode_key`・`!injected`のとき、イベントを保存して300msタイマー武装。
- `run_modekey_conv_check`(タイマー発火時): Shift関連ガード中なら100ms後に再試行、そうでなければ保存イベントを
  `is_ime_mode_key=false`にして`force_conv_check=true`で`kp_stage_idle_conv_check_inner`を呼ぶ。
- `..._inner`は`force`のとき、`output_idle_ms`と`explicit_age`を`u64::MAX`にし、`is_tsf_native`を強制`true`にする。

## 関連する他セッションの設計(重複の可能性)

`origin/adr/187-atok-passthrough-belief-follow`のADR-187は、ATOK+パススルー(opt-in無し)で、実IMEの開閉にEngineが追随しない問題を
「生キー通過後に読み直してbeliefを追随」する観測型で検討し、`IntentStore`/`ImeModel::last_intent`の二重固定を外すのに新しい
`ImeEvent` variantが要り重いとして見送った(status欄に「followのスパイクが成功したため撤回候補」とある)。
本ADRの「モードキー後に読み直す」は、機構が重なる可能性がある。

## レビューしてほしい点(批判的に)

1. 新しいタイマー+GateStore 3フィールドの積み増しは最小か。既存の機構(`FocusResyncGate`、`TIMER_IME_REFRESH`、`idle_conv_check_in_flight_since_ms`、
   ADR-187のfollow方式)を再利用して、フィールドを減らせないか。ADR-184の反省(型・フィールドを積まず最小の配線変更で)に照らして。
2. `force`で`is_tsf_native`を`true`にすると、**Standardプロファイルの通常アプリ(Win32 EDIT等)でも**、モードキーの後にTsfNative用の
   `idle-conv-check`適用経路(`apply_idle_conv_check`)が走る。副作用は無いか(実機のWin32 EDITの回帰は12/12だが、網羅か)。
3. BUG-113(読み取りとactuationの時間的近接で「@」)、BUG-34(`SendMessageTimeoutW`のブロック)、BUG-33/37(Chrome等でImmGet*が不安定)の
   再発リスク。モードキー1押下ごとに追加のクロスプロセス読み取りが入る。`is_ime_mode_key`のキー集合(無変換/変換/ひらがな/VK_IME_ON等)全てで良いか。
4. 競合: 強制チェックが保留中に、フォーカス変更、別のモードキー、`Engine`のON/OFF、ユーザーの打鍵が来たとき。`pending_modekey_event`が古い
   イベントを使い回す/フォーカスが変わったのに読む、を防げているか。`apply_idle_conv_check`のepoch/hwnd/`explicit_action_ms`照合は足りるか。
5. 300msと100ms×30回(最大3秒)の根拠(実測が不足)。`tuning.rs`の規約(`#[measured]`、実測ms)にどう載せるか。
6. Shift+無変換のShift長押し以外の凍結要因(`half_width_alnum`の`toggle_active`が長く続く場合)で、再試行が無限/長時間になる懸念。
7. メモ帳・Windows Terminal・Edgeでは未検証。Chromeでだけ効く実装になっていないか(`is_ime_mode_key`、プロファイル依存)。
8. もっと単純な代案があるか(例: モードキーのKeyUp/Shift解放を契機にする、既存の`schedule_ime_refresh`経路でconvを読む、E1+RECOVERを許容して
   最初の1〜2文字の誤りを許す、決定3の予測反転)。
