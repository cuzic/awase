---
id: ADR-249
title: |-
  他アプリが注入した Ctrl/Shift を、期限付きで修飾キーとして数える(BUG-197、Spokenly の Ctrl+V が「ふ」になる)
summary: |-
  音声入力ソフト Spokenly は Ctrl↓→V↓↑→(約100ms 後)Ctrl↑ を SendInput で注入して貼り付ける。awase は物理押下状態(PHYSICAL_KEY_STATE)を
  非注入イベントだけで更新する(ADR-054、X サーバーの Ctrl KeyUp 欠落による stuck 対策)ため、注入 Ctrl を修飾として数えず、
  V を ctrl=false の Char として NICOLA 変換して「ふ」を出す。注入された Ctrl/Shift の押下を別枠で記録し、期限(TTL)内だけ
  修飾キーとして数える。KeyUp 欠落でも期限で自動復帰するので ADR-054 の stuck 対策と両立する。
status: |-
  起草(2026-10-10)。Opus レビュー前。実装なし。TTL の値は未測定(Spokenly の保持は報告 journal の2例で 101ms・102ms)。
related_adr:
  - "ADR-054"
  - "ADR-052"
---

# ADR-249: 他アプリが注入した Ctrl/Shift を、期限付きで修飾キーとして数える

## 背景

[BUG-197](../known-bugs/BUG-197.md)。報告 `01M4J72G985T0FFT6XPN0SWCQQ`(GJI、awase 2.0.0)で、Spokenly の音声入力を使うと
「ふ」1文字だけが入る(awase を止める・やまぶきR なら正常)。

### 事実(報告の journal と CI のログ。観測したものだけ)

1. Spokenly の貼り付けは、注入キー(`injected=True`)の列
   `左Ctrl↓(scan 29) → V↓(scan 47) → V↑ → 左Ctrl↑` で、Ctrl↓ から Ctrl↑ まで 101ms と 102ms(2例、時刻 29858→29959、54579→54681)。
2. awase は V を `ctrl=false` の `Char` と判定し、`PendingChar(0x56)` → Consume(出力3件)。NICOLA 配列で V は「ふ」。
3. `[engine-input]` の診断(CI の注入 Ctrl+V、run 38029357773)は V の KeyDown で `mods(c=false …) gas_ctrl=true phys_ctrl=false`。
   OS(`GetAsyncKeyState`)は Ctrl 押下と認識しているが、awase の物理状態は押下なし。
4. 修飾の取得は `observer/focus_observer.rs::read_os_modifiers`。**Ctrl と Shift は `PHYSICAL_KEY_STATE`、Alt と Win は `GetAsyncKeyState`**
   (非対称)。Ctrl を物理状態にしたのは、awase 自身の synthetic Ctrl↑ で `GetAsyncKeyState` が汚染される(物理押下中なのに false)ためと、
   X サーバーが注入した Ctrl が KeyUp なしで終わる stuck を避けるため([ADR-054](054-physical-key-state-injected-filter.md)、コミット `8351bcf9`)。
5. コアエンジンは `modifiers` の Ctrl/Alt/Win のいずれかが押下中なら `BypassReason::OsModifierHeld` で素通しにする。
   つまり `mods.ctrl=true` で V が届けば、貼り付けは成立する。

### 制約

- ADR-054 の2つの問題(awase 自身の synthetic Ctrl↑ による汚染、他アプリの注入 Ctrl の KeyUp 欠落)を再発させない。
- 注入キーの通り道は他にもある。**PowerToys Keyboard Manager などのリマップは、変換結果の文字キーを注入する**
  (報告者の環境にも `PowerToys` がある)。注入された文字キーの NICOLA 変換を止めると、リマップ後のキーが変換されなくなる。

## 選択肢

| 案 | 内容 | 判定 |
|---|---|---|
| A | 注入された Ctrl/Shift の KeyDown を別枠(`foreign_mod_down_at_ms`)に記録し、**期限 TTL 内だけ**修飾として数える。対応する注入 KeyUp、同じ VK の物理 KeyUp、`reset_physical_key_state` で解除する | **採用案** |
| B | 注入された文字キーは常に素通しにする | 却下。リマップ(PowerToys 等)の注入キーを変換できなくなる |
| C | Ctrl も `GetAsyncKeyState` で読む(Alt/Win と同じ) | 却下。ADR-054 の汚染(物理押下中に awase の synthetic Ctrl↑ で false)と stuck を再発させる |
| D | 注入元アプリ(プロセス名)で例外扱い | 却下。LL フックでは注入元を特定できない。個別アプリ名の列挙は脆い |
| E | 注入 Ctrl↓ の「直後の1文字キー」だけ素通し(状態を持たない) | 保留。Ctrl の保持が長い注入(連続貼り付け等)や Ctrl+Shift+V に弱い。A の簡易版 |

## 決定案(案 A)

1. `hook.rs` の注入イベント分岐(`is_injected && !self_injected`)で、VK が Ctrl(0xA2/0xA3)・Shift(0xA0/0xA1)のとき、
   KeyDown なら `foreign_mod_down_at_ms[vk] = now`、KeyUp なら 0 にする。`physical_key_state` は従来どおり更新しない(ADR-054 を壊さない)。
2. `read_os_modifiers`(および同じ判定の呼び出し元)は `ctrl = physical || foreign_ctrl_fresh`、`shift = physical || foreign_shift_fresh`。
   `foreign_*_fresh = down_at != 0 && now - down_at < FOREIGN_MOD_TTL_MS`。判定は純粋関数
   `foreign_modifier_active(now_ms, down_at_ms, ttl_ms) -> bool` に切り出して Linux で単体テストする。
3. 解除: 対応する注入 KeyUp、同じ VK の物理 KeyUp、`reset_physical_key_state`(画面ロック復帰・パニックリセット、BUG-023 の経路)。
4. 追加の保険(Opus への論点): `foreign_*_fresh` のとき `GetAsyncKeyState` の同意も必須にするか。OS が離したと認識しているのに
   記録が残る状況(KeyUp を別経路で失った等)を避けられる反面、awase の synthetic Ctrl↑ による汚染で一時的に false になりうる。
5. `FOREIGN_MOD_TTL_MS` は `tuning.rs` に `#[measured(...)]` 付きで置く([tuning-constants](../../.claude/rules/tuning-constants.md))。
   測るもの: 注入 Ctrl の保持時間(Ctrl↓→Ctrl↑)。現状の実測は Spokenly の 101ms・102ms の2例のみ。
   他の貼り付け系(AutoHotkey の `Send ^v`、Windows Terminal の Ctrl+Shift+V、PowerToys など)を実機・CI で測り、
   最大値に余裕を足して決める(導出式を本文に書く)。**値は未決**。

## 検証

- 単体(Linux): `foreign_modifier_active` の境界(期限内・期限切れ・down_at=0)。
- hook の状態遷移テスト: 注入 Ctrl↓→V→Ctrl↑ で V が `OsModifierHeld`(素通し)。注入 Ctrl↓ のみで KeyUp 欠落 → 期限後は通常の NICOLA 変換。
  物理 Ctrl と注入 Ctrl が重なる場合(物理が先に離れる/注入が先に離れる)。ADR-054 の既存テスト(synthetic Ctrl↑ の汚染)が通ること。
- CI の A/B(`ci/e2e-dictation`): `tsx-dict-*-paste`。事前に CI の paste 模擬を直す
  (クリップボードを Win32 API で設定し、フォーカスを奪わない。現状は PowerShell が前面を奪い全試行 `focus_ok=False` で無効)。
  修正前は「ふ」が出る(FAIL)、修正後は挿入文が入る(PASS)ことを確認し、awase なしを対照にする。
- 再発ファミリー(物理キー押下ラッチ)なので `fix-requires-evidence` の (a) 回帰テストを添える。

## 影響・リスク

- Shift を含めると、注入 Shift+V などを素通しにする。リマップが注入する Shift 付きの文字キーに影響しうる(論点)。
  Ctrl だけに絞る案もある(Windows Terminal の貼り付けは Ctrl+Shift+V で、Shift の欠落は問題にならない可能性が高い、未確認)。
- TTL を超えて Ctrl を保持する注入は、従来どおり(修飾なし)の動きに戻る。悪化はしない。
- 別件 [BUG-198](../known-bugs/BUG-198.md)(注入 `VK_PACKET` の文字が保留→再注入で消える)は、同じ CI・同じリリースで直したいが、原因が別なので別の変更にする。

## Opus レビューで特に見てほしい点

1. 注入 Ctrl↓ の記録を `physical_key_state` と分けるだけで、ADR-054 の stuck を再発させないと言えるか(TTL・KeyUp・reset の網羅)。
2. `GetAsyncKeyState` の同意を必須にすべきか(決定案 4)。
3. Shift まで広げる是非と、リマップ(PowerToys KBM)への影響。
4. 案 E の方が単純で十分ではないか。
5. 欠けている事実(上の「事実」で、ログやコードで裏取りできていないもの)。
