---
title: conv 軸（変換モード）と、それに付随する開閉軸の書き込み経路の棚卸し（09 T5）
status: 完了（2026-09-25）。撤去・維持の判断は別（下記「結果」の分類が入力）
created: 2026-09-25
related_adr: ["ADR-191", "ADR-189", "ADR-084", "ADR-086", "ADR-094"]
source_review: docs/tasks/review-2026-09-24-09-remaining-active-writes-inventory.md の T5
---

# conv 軸の書き込み経路の棚卸し（09 T5）

裏取り基準は `13fede08`（origin/develop、PR #310 まで）。`docs/tasks/actuation-confluence-inventory.md` と同じ粒度（1経路1行、根拠の行番号付き）で、
分類軸は本タスク専用（ADR-191 決定1の線引きに照らした3分類）。着手時は `.claude/rules/worktree-per-session.md` に従う。

## 起点と方法

09 T5 の指示どおり、「3関数の grep」ではなく次の2つを起点にした。

1. 低レベルの唯一の書き手 `ime.rs::modify_conv_mode`（`:364`、`IMC_SETCONVERSIONMODE` を発行する）に到達する全経路を、呼び出し元を逆にたどって出した。
   `ActuateCmd::SetConversionMode`（`imm.rs:133`）の使用箇所は `modify_conv_mode` の1か所だけ（`ime.rs:380`）で、`IMC_SETCONVERSIONMODE` の直接発行も `imm.rs` 内だけ。
2. conv を変える**モードキーの注入**（`VK_DBE_HIRAGANA`/`VK_DBE_ALPHANUMERIC`）を、`vk.rs` 定数の使用箇所から洗い出した。

`apply_input_mode_correction`（`runtime/mod.rs:922`）は `ImeEvent::InputModeApplied` を dispatch するだけの **belief 更新**で、OS への書き込みは無い（対象外）。

## 書き込みの入口（`modify_conv_mode` の直接の呼び出し元）

| 入口 | 場所 | 何をするか |
| --- | --- | --- |
| `set_ime_romaji_mode_for_hwnd` | `ime.rs:676` | `target_conv` を書く。`None` なら現在値に ROMAN ビットを足す。下の3関数が共有する実体 |
| `set_ime_hiragana_mode_cross_process` | `ime.rs:708` | NATIVE\|FULLSHAPE\|ROMAN を立て KATAKANA を落とす（宛先はフォーカス中の窓をライブ取得） |
| `set_ime_mode_for_target` | `ime.rs:1691` | 開閉を書いた後、開のとき conv にマスクを適用する |

`set_ime_romaji_mode_for_hwnd` の呼び出し元は `set_ime_romaji_mode_for_target_blocking`（`ime.rs:1184`）、`set_ime_conv_for_target`（`ime.rs:1231`）、
`set_ime_open_then_conv_for_target`（`ime.rs:1342`）の3つ。

## 経路一覧（11経路と、他に無いことの確認）

分類: **A** 決定1の線引きに反する（撤去候補） / **B** 正当な例外（ユーザーの明示操作・緊急リセット・awase が作った状態の後始末） / **C** 撤去対象外（warmup、ADR-191 決定1・§6 のガイド）。

| # | 経路 | 場所（入口→起点） | 書くもの | 分類 | 根拠・備考 |
| --- | --- | --- | --- | --- | --- |
| 1 | ROMAN 補完（開閉書き込みの前段） | `ime_controller.rs:208` `romaji_pre_write`（`:410`、`:439` で `set_ime_romaji_mode_for_target_blocking`）。条件は `state/ime_actuation_decision.rs:230` `decide_needs_romaji_pre_write`（開く方向・ImmCross/MsImeDirect・MS-IME・belief がかな入力でない） | conv に ROMAN を足す | **A** | 開閉の書き込みに**付随する conv 書き込み**。開閉自体は固定セット等で許されても、conv 軸は「IMEに任せて追随」（決定1）。MS-IME が開いた直後にかな入力へ落ちる対処が動機。撤去には MS-IME 本体の実機確認が要る |
| 2 | ROMAN 補完（非同期 ImmCross の後段） | `runtime/open_chain.rs:312` `imm_cross_write` → `ime.rs:1342` `set_ime_open_then_conv_for_target`。条件は `state/ime_actuation_decision.rs:249` `decide_dispatch_conv_after_open`（開く方向・belief がかな入力でない）。`runtime/executor.rs:925` が `dispatch_ime_set_open` から `ConvAfterOpen` を組み立てる | conv に ROMAN を足す（`Write(None)`） | **A** | 1と同型の別の窓口（sync/async の2系統）。**1と2の条件は意図的に別**（`decide_needs_romaji_pre_write` は MS-IME に限る、こちらは種別を見ない）。両方を撤去するか、片方を残すかを決める |
| 3 | cold-start の ROMAN 保護 | `output/vk_send.rs:440` → `tsf/warmup/cold_warmup.rs:45` `run_start`（`:94` で `set_ime_conv_for_target(target, None)`）。`conv_mutation_allowed` のときだけ | conv に ROMAN を足す | **C** | warmup は撤去対象外（ADR-191 決定1、BUG-19。カタカナ/英数への明示復元は撤去済みで、ROMAN 確保だけが残る） |
| 4 | 半角英数トグルの entry（左 Shift 単独タップ） | `runtime/key_pipeline.rs:2205` `actuate_conv_mode` → `runtime/conv_actuation.rs:27` → `output/conv_actuation.rs:141` `actuate_conv_mode`（`:176` で `set_ime_conv_for_target(target, Some(0x0000))`）。GJI では加えて `key_pipeline.rs:2255` が `VK_DBE_ALPHANUMERIC` を注入（`output/mod.rs:1238`） | conv=0x0000（MS-IME 等）／半角英数キー（GJI） | **B** | ユーザーの明示操作（左 Shift の本物の単独タップ、opt-in の機能）が起点。awase が「半角英数トグル」という持続状態を作る |
| 5 | 半角英数トグルの復元（IMC 書き込みの再試行） | `runtime/key_pipeline.rs:2366` `kp_restore_kana_from_half_width`（`:2562` で `set_ime_conv_for_target(target, Some(NATIVE\|FULLSHAPE\|ROMAN))`、最大4回）。起点は `runtime/ime_refresh.rs:302`（フォーカス変更時にトグルを強制解除）、`key_pipeline.rs:1463`/`:1506`/`:1761`（IME ON 経路でトグル中なら解除）、`:2289`（Shift の KeyUp） | conv=かな入力（NATIVE\|FULLSHAPE\|ROMAN） | **B** | 4の**対**（awase が作った状態を巻き戻す）。4を残す限り必須で、4を撤去するなら同時に不要になる。フォーカス変更起点のものは「他アプリへ持ち越さない」ための後始末 |
| 6 | 半角英数トグルの復元（MS-IME のモードキー注入） | `runtime/key_pipeline.rs:2366`（`:2457`〜`:2470` で `VK_DBE_HIRAGANA` を注入）。`MicrosoftIme` かつ開かつ Win/Alt 非押下のときだけ | `VK_DBE_HIRAGANA` の押下/離し | **B** | 5と同じ関数の前段。5と同じく4の対 |
| 7 | 物理かなキーの埋め合わせ | `runtime/key_pipeline.rs:205`〜`:262`（`:250` で `send_gji_half_width_alnum_toggle(Exit)` → `output/mod.rs:1239` の `VK_DBE_HIRAGANA`）。`conv_mutation_allowed`・IME ON・warm・かなロック Off のときだけ | `VK_DBE_HIRAGANA` の注入 | **A** | Suppress した物理かなキーの代わりに、awase がモードキーを注入する（ユーザーのキー押下の代行）。決定1の「入力モードを変えるキーはIMEに任せる」に反する。Suppress を止めれば不要になる（Suppress の判断に従属） |
| 8 | Ctrl+変換で IME が既に ON のときのリセット | `runtime/key_pipeline.rs:1738` → `:1805` `kp_reset_to_hiragana_romaji_capsoff`（`:1847` で `set_ime_conv_for_target(target, Some(mask))`） | conv=ひらがな＋ローマ字（Caps Lock も Off） | **B** | ユーザーの明示コンボ操作（`is_default_ime_on_combo`）が起点。`conv_mutation_allowed` のゲートを通らない（関数内に既存の注意書きあり） |
| 9 | 焦点プローブでのかなモード修正 | `runtime/key_pipeline.rs:558` → `:2894` `apply_focus_probe`（`:3158` で `set_ime_conv_for_target(target, None)`）。かなモード（MS-IME）かつ IME ON のとき | conv に ROMAN を足す | **A** | **観測に反応して自動で訂正する**書き込み。ユーザー操作が起点ではない。受動化（決定1）の対象で、撤去候補のうち最も原則に反する。撤去したときの影響は MS-IME のかなモード誤入力（実機確認が要る） |
| 10 | パニックリセット | `runtime/mod.rs:1844`（`panic_reset` の中）→ `ime.rs:740` `set_ime_hiragana_mode_cross_process_async` → `ime.rs:708` | conv=ひらがな＋ローマ字（開閉も OFF→ON） | **B** | 緊急リセット。IME 関連キーの連打が起点 |
| 11 | トレイ「状態をリセット」 | `runtime/message_handlers.rs:1210`（`handle_wm_command`、`ResetState`）→ `ime.rs:1691` `set_ime_mode_for_target(hwnd, true, NATIVE\|FULLSHAPE, KATAKANA)` | 開を書き、conv にマスクを適用 | **B** | ユーザーのトレイ操作が起点（ADR-094 で書き込みマスクから ROMAN を外した） |
| 12 | 起動時・その他の直接呼び出し | なし | — | — | `modify_conv_mode` の呼び出し元を `grep` で網羅した結果、上の入口以外は存在しない |

### 開閉軸の書き込み（conv の経路と同じ場所で行われるもの。T5 の指示で併記）

| # | 経路 | 場所 | 分類 | 備考 |
| --- | --- | --- | --- | --- |
| O1 | フォーカス変更時の強制 OFF | `runtime/ime_refresh.rs:599`〜`:606`（`set_ime_open_ordered`、`focus_change_enforce_off`） | **A** | 09 の T2・T3 の対象。ImmCross のみ。撤去可否は T2 の測定待ち（別セッション） |
| O2 | パニックリセット（OFF→ON） | `runtime/mod.rs:1841`〜`:1842`（`set_ime_open_cross_process_async(false)` → `(true)`） | **B** | 10と同じ緊急リセット |
| O3 | トレイ「状態をリセット」の開書き込み | `runtime/message_handlers.rs:1210` → `ime.rs:1691`（`set_ime_open_for_target`）。`ime_on=true` を渡す | **B** | 11の前半 |
| O4 | フォーカス変更時の eager warmup | `platform.rs` の `send_eager_tsf_warmup` 呼び出し（`:305`/`:642`/`:1427`/`:1451`/`:1570`）。フォーカス変更時の呼び出しは `runtime/ime_refresh.rs:596` 付近のログが目印 | **C** | warmup は撤去対象外（ADR-191 決定1） |

固定セット（`0x19`・`0xF3`・`0xF4`、ADR-189）と `keys.ime_on/off/toggle` の開閉書き込みは、**08 の担当**（(B) 案は開閉軸だけを扱う）で、conv 軸の経路ではない。
本表の結論は 08 の (B) 案と独立で、08 の結果によって分類は変わらない（conv を書く経路は固定セットに含まれない）。

## 結果

- conv 軸を書く経路は **11経路**（表の1〜11。12は「他に無い」という確認）。内訳は **A: 4**（1・2・7・9）、**B: 6**（4・5・6・8・10・11。4〜6は「半角英数トグル」機能で一組）、**C: 1**（3）。
- 開閉軸で conv の経路と同じ場所にあるものは O1〜O4（A: 1、B: 2、C: 1）。
- **A の撤去候補の優先順**（原則との距離と、撤去の実機確認の重さから）:
  1. **9（焦点プローブでのかなモード修正）**: 観測に反応する自動訂正で、原則からいちばん遠い。撤去後の症状は MS-IME のかなモード誤入力に限られる。
  2. **1・2（ROMAN 補完）**: 1と2は sync/async の2つの窓口。片方だけ撤去すると、もう片方が残る側で挙動が非対称になる（`.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」の教訓）。**両方を同時に**扱うこと。
  3. **7（物理かなキーの埋め合わせ）**: Suppress の判断（ADR-192 系）に従属する。単独では決められない。
- **B の維持条件**: 4・5・6は一組（4を撤去するなら5・6も不要）。8・10・11 はユーザーの明示操作なので維持。
- **C は撤去対象外**（ADR-191 の方針どおり）。ただし3は「ROMAN ビットの確保だけ」に縮小済み。

## ADR-191 決定5 指標5 への加算

指標5（IME へ書く振る舞いの数）に、上の経路を次のように数える。
conv 軸: 固定の例外 0、opt-in の単独タップ（半角英数トグル）1 組（経路4〜6）、ユーザー明示操作（Ctrl+変換のリセット8、トレイ11、パニック10）3、warmup（3）1、観測への自動訂正（9）1、開閉書き込みへの付随（1・2）2 窓口、物理キーの代行（7）1。

## タスク（この表を受けて）

- [ ] 撤去する経路を決める（A の 9 →1・2 →7 の順を推奨）。撤去ごとに、`.claude/rules/fix-requires-evidence.md` の対象ファイル（`ime_controller.rs`、`runtime/key_pipeline.rs`、`runtime/open_chain.rs` ほか）なので回帰テストか `docs/known-bugs/` を添える。
- [ ] 9・1・2 の撤去は MS-IME 本体（かなモードへ落ちる症状）の実機 A/B が要る。CI の MS-IME 構成（`e2e-ime.yml`）で足りるかを確かめる。
- [ ] `set_ime_conv_for_target` の呼び出し元件数ガードを `architecture_guard.rs` に足す（09 T6。現状は `gji_on_focus_change` からの呼び出し禁止だけが固定されている）。

## 未確認点

- 経路9（焦点プローブ）の `should_restore` 条件が、実際にどの構成で真になるか（MS-IME 本体のみか）。コードは `key_pipeline.rs:3136`〜`:3140`。
- 経路7が Suppress の代行になる条件（物理かなキーを Suppress するのが既定か、設定次第か）。
- `set_ime_conv_for_target` の呼び出し元は5か所（`cold_warmup.rs:94`、`conv_actuation.rs:176`、`key_pipeline.rs:1847`/`:2562`/`:3158`）で、上の表と一致する。今後増えていないかは件数ガードで確かめる（T6）。
