---
id: ADR-253
title: |-
  他アプリが注入した VK_PACKET(Unicode 注入)の文字を、保留→再注入で失わない(BUG-198)
summary: |-
  `RawKeyEvent::reinject` は wVk と wScan(IME モードキー以外は 0)で組み立て、`KEYEVENTF_UNICODE` を付けない。VK_PACKET の文字は wScan に載るので、
  保留(relay-defer)を経た再注入で文字が消える疑いがある。ただし「最初の KeyDown が直接通過と記録されているのに全文字が消える」点が説明できていない。
  先に原因の切り分け(段階0)を CI で行い、そのうえで再注入を VK_PACKET 対応にする(案 a)。
status: |-
  起草(2026-10-10)。未実装。原因は仮説(段階0で確認する)。
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

未説明の点: ログでは最初の KeyDown が「直接通過」とも記録されている。直接通過なら文字は届くはずで、全文字が消える説明にならない。
(LL フックの戻り値の扱い、保留が全 KeyDown に及ぶか、KeyUp だけが再注入されるか、のどれかを切り分ける必要がある。)

## 選択肢

| 案 | 内容 | 判定 |
|---|---|---|
| a | 再注入を VK_PACKET 対応にする: `vk==0xE7` のとき `wVk=0`・`wScan=元の文字`・`KEYEVENTF_UNICODE`(+KEYUP)で送る | **採用案(段階1)**。保留の順序保証(awase の出力と他アプリの入力の並び)を保つ |
| b | VK_PACKET を hook で飲み込まず素通し(エンジンに渡さない) | 保留しないので awase の出力中の文字と順序が入れ替わりうる。再注入も順序が入れ替わるのは同じだが、保留の目的(出力と並べる)を失う。a で不足なら再検討 |
| c | 何もしない(音声入力ソフトの実在が未確認) | 実在する注入元(PowerToys・リモートデスクトップ・IME 系ツール)が未確認なだけで、文字が消えるのは確認済み。a は変更が小さいので却下 |

## 決定(案)

**段階0(原因の切り分け、実装前)**: `ci/e2e-dictation` の `tsx-dict-*-unicode` で、`vk=0xE7` の KeyDown/KeyUp それぞれについて、フックの戻り値(飲み込み/通過)・
保留に入ったか・再注入の `wVk/wScan/flags` をログに出して実測する。確認すること: (i) フックで `scanCode` に文字が入っているか(未確認)、
(ii) 最初の KeyDown が実際に OS に届いているか、(iii) サロゲートペアは2つの VK_PACKET になるか(各 `wScan` が半分)。

**段階1(実装、段階0の結果が仮説どおりなら)**:
1. 再注入の入力組み立てを純粋関数 `reinject_key_spec(vk, scan, is_keyup) -> (wVk, wScan, flags_bits)` に切り出す(`awase-windows-core`、`windows` 依存なし)。
   `vk==0xE7` は `(0, scan as u16, UNICODE | KEYUP?)`、他は現行どおり。`windows` の型への変換は `lib.rs` の殻だけが行う。
2. Linux の単体テストで、VK_PACKET と通常キー・IME モードキーの組み立てを固定する(`vk.rs::reinject_keeps_scan_code_only_for_ime_mode_keys` の隣)。
3. **自己注入はそのまま**: awase 自身の Unicode 出力(`INJECTED_MARKER`)は `self_injected` で早期 return するので対象外。再注入にも `INJECTED_MARKER` を付けるので二重処理は起きない(現行どおり)。

## 検証

- fix-requires-evidence の再発ファミリー「defer/replay キューの解放条件」に触れるため回帰テストを添える: 単体テスト(上記2)と、
  CI の `tsx-dict-*-unicode` が awase ありで PASS になること(awase なしと同じ文字列、edit/rich/tsf × GJI/MS-IME)。
- 順序: 注入文の直後に NICOLA 打鍵を続けても、注入文→打鍵の順で出る(既存の A/B が見ている列と同じ)。
- 段階0で仮説が外れた場合、本 ADR を改訂してから実装する(原因が別なら案 a は効かない)。

## 未決

- 実在する注入元(どのアプリが VK_PACKET で注入するか)。BUG-198 は「未確認」のまま。ユーザー報告が出るまで優先度は ADR-249 より低い。
