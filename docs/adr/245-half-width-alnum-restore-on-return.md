---
id: ADR-245
title: |-
  プロセスをまたぐフォーカス移動で半角英数トグルを、離れるときでなく「戻ってきたとき」に復元する(BUG-193)
summary: |-
  BUG-193: 窓 A で持続半角英数(左 Shift 単独タップ)にしたまま別プロセスの窓 B へ移ると、離脱時の強制復元は IMC 書き込みが世代の bump で必ず中断し、
  F2/VK_DBE_HIRAGANA は移動先 B に届く。A は半角英数のまま残り、awase のトグルフラグだけが下りる(MS-IME 本体・GJI とも CI で 5/5)。
  仕様(ADR-107 の検証項目 4)は「往復しても英数状態が持ち越されない」。本 ADR は、離脱時には何も送らず、A を覚えておいて A に戻ったときに復元する案(B)を推奨する。
status: |-
  起草(2026-10-08)。Opus レビュー前。未決 Q1〜Q3 あり。
related_adr:
  - "ADR-107"
  - "ADR-084"
  - "ADR-212"
---

# ADR-245: 半角英数トグルの復元を、戻ってきたときに行う(BUG-193)

## 事実(確かめたもの)

- 離脱時の強制復元は `ir_notify_focus_changed`(`ime_refresh.rs:441`)が `kp_restore_kana_from_half_width(false)` を呼ぶ。起案時に `owner_gen`/`focus_gen` を捕獲して `spawn_local` するが、同じ同期呼び出しの後半で `gji_on_focus_change` → `on_ime_mode_focus_changed` が両世代を進める(`ime_refresh.rs:694`、`platform.rs:632`)。タスク先頭の世代確認が必ず失敗し、**IMC 書き込みは一度も行われない**(`key_pipeline.rs:2299,2308`)。
- 同期で残るのは SendInput(MS-IME は scan 付き VK_DBE_HIRAGANA、GJI は F2)だけで、配送時点の前面窓(= 移動先 B)に届く。B の belief が閉ならスキップ、開なら B に届く。
- IMC 書き込みだけでは新 MS-IME(TSF)の実モードは英数から戻らない(`key_pipeline.rs` のコメント、2026-07-07 実機)。つまり**旧窓 A が前面でない間、A を確実に戻す手段は無い**。
- CI(BUG-193、`sc-focusrestore-*`、run 37781262341 / 37782133675): A は戻っても 0x10 のまま、戻して打つと素通し(`w`)。awase のトグルフラグは `begin_restore_kana` で下り、戻ってからの 2 回目の左 Shift タップは A を変えない。
- 仕様: ADR-107 の検証項目 4「トグル ON → フォーカス変更 → 戻る、を往復しても英数状態が持ち越されないこと」(BUG-25)。実機検証(Task 9)は未実施のままで、実装が仕様を満たしていない。
- 復元が呼ばれるのはプロセスをまたぐ移動だけ(`advance_focus_tracking` が pid 差で `process_changed` を決める)。同一プロセス内の窓移動は対象外。

## 案

| 案 | 内容 | 評価 |
|---|---|---|
| A. 離脱時に旧窓へ書く | A の hwnd を捕獲して IMC 書き込み | MS-IME(TSF)には効かない。GJI は IMC が読めない(Imm32Unavailable)。不可 |
| **B. 戻ったときに復元(推奨)** | 離脱時は何も送らない。A の前面スコープ(`ForegroundScope{pid,hwnd}`)を覚え、A が前面に戻ったときに既存の復元を走らせる | 仕様を満たす。B/C への余計な F2 が消える。A の前面窓に SendInput が届く。状態が 1 つ増える |
| C. 窓ごとに半角英数を維持 | 戻ったときの半角英数を正とし、トグルフラグも窓ごとに持つ | 仕様と逆。2 回目タップで抜けられる利点はあるが、半角英数が他アプリへ続くと感じる利用者の不満が元の動機(BUG-25) |
| D. 何もしない | 強制復元を消すだけ | A は半角英数のまま、フラグは下り、2 回目タップは「開始」になる(現状の悪化版)。不可 |

## 決定案(B)

1. **離脱**: トグル中に `process_changed` で別プロセスへ移ったら、IME へ何も送らない。トグルフラグを下ろし、`ForegroundScope` を「戻り待ち」に積む(現在の `begin_restore_kana` + 強制復元の呼び出しを置き換える)。
2. **記録**: Enter 時に前面の `ForegroundScope` を保持する(`commit_enter_imc`/`commit_enter_gji` に渡す)。離脱時に戻り待ちへ移す。
3. **戻り**: `process_changed` で新しい前面スコープが戻り待ちの 1 件と `pid` と `hwnd` の両方で一致したら、そのエントリを取り出して既存の復元(`kp_restore_kana_from_half_width`)を走らせる。一致しなければ保持する。
4. **容量**: 戻り待ちは小さな固定長(例: 4 件、古いものから捨てる)。TTL は持たない(定数を増やさない)。pid と hwnd の両方の一致で、pid 再利用の取り違えを避ける。
5. **純粋な核**: `state/half_width_alnum.rs` に、事実(トグル状態・戻り待ち・新しい前面スコープ)から計画 `Nothing | Suspend | Resume` を決める純関数を置く(FCIS、ADR-229 の「事実 → 理由つきの Plan → 殻が実行」)。殻は `ir_notify_focus_changed` で実行するだけ。
6. **GJI の非冪等 F2**(INV-B): Resume は戻り待ちから取り出した 1 回だけ。取り出しと同時にエントリが消えるので二重送信は無い。

## 検証

- Linux 単体(`state/half_width_alnum.rs`): 離脱で Suspend、戻りで Resume、別窓では保持、容量超過、pid だけ一致の取り違えは Resume しない、を表で固定。
- CI: `sc-focusrestore-*` を `--strict` に格上げする。合格条件は、A に戻ったあとの conv が NATIVE、`typed == か`、窓 B の conv が不変、離脱時の SendInput が B に出ていないこと。MS-IME 本体・GJI の両方(各 5 試行)。
- fix-requires-evidence: focus 遷移・conv mode ファミリーなので、上の単体テストと CI 構成が (a)(b) を満たす。

## 未決(Opus レビューと実測で決める)

- **Q1: Resume の実行位置**。`ir_notify_focus_changed` は観測ステージより前に走る(`ime_refresh.rs` の stage 順)。この時点の `effective_open()` は A の belief ではなく直前(B)のままかもしれず、復元の SendInput の条件(`effective_open()` が真)を誤判定しうる。戻った直後ではなく、A の観測(settle 後)の後に実行すべきかを、CI で時点ごとの `effective_open`/conv を測って決める。
- **Q2: 復元の宛先の確認**。Resume 時に前面が本当に A かを `foreground_scope()` で再確認し、移動の途中(A→C に素早く移った)なら実行しない、を入れるか。
- **Q3: A の 0x0 → 0x10 の遷移**。離脱後に A の conv が 0x0 から 0x10(ROMAN だけ)に変わる主体は未確認(awase のログに出ない)。Resume の復元書き込みは `NATIVE|FULLSHAPE|ROMAN` を書くので結果は同じ見込みだが、原因が awase(ROMAN 補完 #9〜#11)なら別の BUG。

## 範囲外

- 同一プロセス内の窓移動(`process_changed` が偽)。移動先で IME の実状態が違うときの扱いは未測定。
- 実 Chrome・UWP を A/B にした場合(CI の観測は自作 Win32 窓)。
- 観測起点の書き込みの整理(`docs/tasks/observation-driven-writes-inventory-2026-10-08.md` の A〜C)。本 ADR は個別の欠陥の修正で、整理の方向とは独立。
