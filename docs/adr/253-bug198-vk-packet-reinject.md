---
id: ADR-253
title: |-
  他アプリが注入した VK_PACKET(Unicode 注入)の文字を、保留→再注入で失わない(BUG-198)
summary: |-
  他アプリが `KEYEVENTF_UNICODE` で注入した文字(`VK_PACKET`=0xE7)は、フックでは `scanCode` に文字が載って届き、`hook.rs` が飲み込んで(`Accepted=>LRESULT(1)`)
  PassThrough を必ず再注入に回す。再注入(`RawKeyEvent::reinject`)は `wScan=0`・UNICODE 無しで組むため文字が失われる。保留(relay-defer)の有無とは無関係。
  段階0の CI 実測(run 38045062764)で確認済み。再注入の入力組み立てを純粋関数 `reinject_key_spec` に切り出し、VK_PACKET は UNICODE で送り直す。
status: |-
  段階1実装済み・CI 確認待ち(2026-10-10)。段階0は実測済み(原因確定、サロゲートは無事・ASCII は classify の別原因)。
related_adr:
  - "ADR-156"
  - "ADR-249"
---

# ADR-253: 注入 VK_PACKET の文字を保留→再注入で失わない

## 背景

[BUG-198](../known-bugs/BUG-198.md)。CI(run 38029357773)で、他アプリ相当(`dwExtraInfo=0`)から `KEYEVENTF_UNICODE` で「音声入力テスト」を1文字ずつ注入すると、
awase なしは全 PASS、awase ありは全 FAIL(注入文だけが消え、その後の NICOLA 打鍵は正常)。ログは `vk=0xE7` が `PassThrough` →
`relay-defer`(`runtime/transport.rs` の `check_output_guard_defer`)→ `[reinject] vk=0xe7 down/up`。

コードで確認した事実(`develop` 7cecbcf6):

1. `RawKeyEvent::reinject`(`crates/awase-windows/src/lib.rs`)は `wVk=self.vk_code`・`wScan=vk::reinject_scan_code(..)`・`dwFlags` は KEYUP の有無だけ。
   `reinject_scan_code` は IME モードキー以外では **0**(`vk.rs`)。`KEYEVENTF_UNICODE` は付かない。
2. `crates/awase-windows*/src` に `0xE7`/`VK_PACKET` の特別扱いは無い(grep、7cecbcf6)。
3. したがって再注入されるのは `wVk=0xE7, wScan=0, flags=0` で、`wScan` に載っていた文字が失われる。

**段階0の実測(run 38045062764、`ci/adr253-diag`、`tsx-dict-*-unicode-*`、awase.log の診断行)**:

- フックには `vk=0xE7`・**`scanCode`=注入した文字**(例「音」=0x97F3)・`flags=0x10`(LLKHF_INJECTED)・`extra=0x0` の KeyDown/KeyUp が届く。
- 注入した全イベントが飲み込まれ(`hook.rs` の `ProduceResult::Accepted => LRESULT(1)`)、PassThrough は保留の有無と無関係に `enqueue_reinject` される。
  「最初の KeyDown が直接通過」というログの読みは誤りだった(通過ではなく、再注入の経路に入っていた)。
- 再注入は `wScan=0x0`(元は 0x97F3)・UNICODE 無し。再注入の KeyDown/KeyUp は `extra=0x4B45594D`(自己注入)として届き、素通しされる。文字は復元されない。

したがって原因は「保留・再注入の解放条件」ではなく**再注入の入力の組み立て**(`wScan`/`KEYEVENTF_UNICODE`)。

## 選択肢

| 案 | 内容 | 判定 |
|---|---|---|
| a | 再注入を VK_PACKET 対応にする: `vk==0xE7` のとき `wVk=0`・`wScan=元の文字`・`KEYEVENTF_UNICODE`(+KEYUP)で送る | **採用案(段階1)**。保留の順序保証(awase の出力と他アプリの入力の並び)を保つ |
| b | VK_PACKET を hook で飲み込まず素通し(エンジンに渡さない) | 段階0で、飲み込みは保留と無関係に常に起きると分かったので「保留を避ける」案ではなく「飲み込みを避ける」案になる。順序保証(awase の出力中の文字との並び)を失う。a で不足なら再検討 |
| c | 何もしない(音声入力ソフトの実在が未確認) | 実在する注入元(PowerToys・リモートデスクトップ・IME 系ツール)が未確認なだけで、文字が消えるのは確認済み。a は変更が小さいので却下 |

## 決定(案)

**段階0(原因の切り分け、実装前)**: `ci/e2e-dictation` の `tsx-dict-*-unicode` で、`vk=0xE7` の KeyDown/KeyUp それぞれについて、フックの戻り値(飲み込み/通過)・
保留に入ったか・再注入の `wVk/wScan/flags` をログに出して実測する。確認すること: (i) フックで `scanCode` に文字が入っているか(未確認)、
(ii) 最初の KeyDown が実際に OS に届いているか、(iii) サロゲートペアは2つの VK_PACKET になるか(各 `wScan` が半分)。

**段階0追加実測(run 38046810869、修正入り、注入文「音声2025s}!😀」)**: かな・絵文字(U+1F600、サロゲートペア)は注入どおり届いた(実際の欄に `😀` あり)。
一方 ASCII の `2`・`0`・`5`・`!` 等(scanCode 0x32・0x30 等)は `classify_key` が位置表を引いて NICOLA の Char にし、欄には `そへそ／￥け` が出た(S1の疑いが実測で成立)。
そのため `is_passthrough` に `0xE7` を足し、`VK_PACKET` は常に Passthrough にする。

**段階1(実装、段階0の結果が仮説どおりなら)**:
1. 再注入の入力組み立てを純粋関数 `reinject_key_spec(vk, scan, is_keyup) -> (wVk, wScan, flags_bits)` に切り出す(`awase-windows-core`、`windows` 依存なし)。
   `vk==0xE7` は `(0, scan as u16, UNICODE | KEYUP?)`、他は現行どおり。`windows` の型への変換は `lib.rs` の殻だけが行う。
2. `architecture_guard` で `RawKeyEvent::reinject` が `vk::reinject_key_spec` から `KEYBDINPUT` を取ることを走査で固定し、`REINJECT_FLAG_*` と `KEYEVENTF_*` の一致を `cfg(windows)` の const assert で固定する。
3. Linux の単体テストで、VK_PACKET と通常キー・IME モードキーの組み立てを固定する(`vk.rs::reinject_keeps_scan_code_only_for_ime_mode_keys` の隣)。
4. **自己注入はそのまま**: awase 自身の Unicode 出力(`INJECTED_MARKER`)は `self_injected` で早期 return するので対象外。再注入にも `INJECTED_MARKER` を付けるので二重処理は起きない(現行どおり)。

## 検証

- 再発ファミリー「再注入の組み立て」(物理キー押下ラッチ・defer/replay 周辺)に触れるため回帰テストを添える: 単体テスト(上記2)と、
  CI の `tsx-dict-*-unicode` が awase ありで PASS になること(awase なしと同じ文字列、edit/rich/tsf × GJI/MS-IME)。
- 順序: 注入文の直後に NICOLA 打鍵を続けても、注入文→打鍵の順で出る(既存の A/B が見ている列と同じ)。
- 段階0で仮説が外れた場合、本 ADR を改訂してから実装する(原因が別なら案 a は効かない)。

## 未決

- 実在する注入元(どのアプリが VK_PACKET で注入するか)。BUG-198 は「未確認」のまま。ユーザー報告が出るまで優先度は ADR-249 より低い。
