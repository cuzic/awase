---
title: IME 開閉と Engine 活性の belief のゼロベース設計の前提資料(棚卸し)
status: 棚卸しのみ(2026-10-26、origin/develop ff75d5d58)。設計はしていない。コード・テストは変えていない。読み取りのみ・cargo 未実行・実機ログ未読
created: 2026-10-26
related_adr: ["ADR-087", "ADR-179", "ADR-191", "ADR-205", "ADR-245", "ADR-263", "ADR-272", "ADR-273", "ADR-275", "ADR-277", "ADR-279", "ADR-280"]
---

# IME 開閉と Engine 活性の belief — ゼロベース設計の前提資料

所有者の方針(2026-10-26): 「IME ON かつ ひらがな のときは、必ず awase(NICOLA の Engine)が ON である。擬似窓・擬似 IME を充実させたのだから、設計をゼロベースで考え直す」。
きっかけは BUG-223(`origin/fix/focus-desired-fallback-off`、PR #854): IME は ON なのに Engine が OFF のまま数秒〜数十秒続き、NICOLA の打鍵が GJI のローマ字入力として処理された。

| ファイル | 内容 |
|---|---|
| [01-map.md](01-map.md) | 現在の仕組みの地図。`compute_state` の 4 条件、`effective_open` の優先順位(`IntentStore` → `ForceGuard` → 明示意図 → 予測 → 観測 → 字形 → 最新の信頼 → `desired_open`)、証拠源の表、**イベント × 状態の入出力表**、窓切替の後処理、**ON/OFF の非対称 A1〜A10**、明示意図の 5 つの保管場所、擬似窓が写している範囲 |
| [02-ledger.md](02-ledger.md) | 過去の事故と防御の台帳(BUG 21 件・ADR 21 件)、取り下げた方式の索引、定数の根拠 |
| [03-constraints.md](03-constraints.md) | 設計が守るべき条件(方針・逆向き・観測の信頼性・繰り返してはいけない取り下げ・制約)と出典 |
| [04-complexity.md](04-complexity.md) | 偶発的な複雑さ X1〜X11、必然的な複雑さ N1〜N10、判断が分かれるもの |
| [05-design-questions.md](05-design-questions.md) | 設計の論点 Q1〜Q16(結論なし)、擬似窓で検証する列、所有者に確認が要る前提、この調査でできていないこと |

## 要点(1 ページ)

- **Engine ON は保存された状態ではなく、毎回の関数**: `user_enabled ∧ is_japanese_ime ∧ effective_open ∧ input_mode.is_romaji_capable`。`input_mode` が `Unknown`・`ObservedKana` でも OFF(「ひらがな」の定義が要る)。
- **`effective_open` は 2 層**: hub の `IntentStore`(窓ごと、ON 10 秒/OFF 30 秒)が `ImeModel::resolve_open_at`(6 段 + `PanicReset` ガード)を丸ごと上書きする。
- **ON と OFF は経路で非対称**: 静的な開閉キー・トグルは `UserImeSetIntent`(`desired_open`・`last_intent`・IntentStore・`persistent_explicit_off` を書く)、ひらがなキー等は `KeyEffectPredicted`(`key_effect` だけ。`desired_open` 不変)。窓切替は意図・予測・観測を捨てるが `desired_open` は持ち越す。観測が無い窓では `DesiredFallback` の値が `effective_open` になる。
- **観測が来ない窓がある**: TsfNative では poll が走らず、他の窓でも明示意図があると poll が止まる(ADR-279 段階 0: ずれの間の観測 0 件)。
- **取り下げた方式が多い**(force-on・reassert・ActivationSync・focus 時の強制 OFF・調停機構・時間だけの抑制 など)。ゼロベースで再提案する前に `02-ledger.md` §C を確認する。
