---
id: ADR-175
title: |-
  物理半角/全角キー（VK_DBE_SBCSCHAR/DBCSCHAR）がOS側で同一方向のVKを繰り返し報告するとき、shadow-toggleのno-op誤判定でIME ON固着から抜け出せなくなる問題を修正する（BUG-142）
status: |-
  起草（opus-adversarial-consult未実施）。実機検証（2026-09-15）で根本原因を
  「GJIへの送信経路の故障」から「shadow-toggleのno-op誤判定」に絞り込み済み
  （詳細は[BUG-142](../known-bugs/BUG-142.md)「送信経路は生きている」節参照）。
  本ADRはその修正方針を起票する。
related_adr:
  - "ADR-121"
  - "ADR-153"
  - "ADR-172"
  - "ADR-173"
  - "ADR-174"
---

# ADR-175: 物理半角/全角キー（`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`）がOS側で同一方向のVKを繰り返し報告するとき、shadow-toggleのno-op誤判定でIME ON固着から抜け出せなくなる問題を修正する（BUG-142）

## 背景・確定した事実（2026-09-15、実機検証で裏取り済み）

[BUG-142](../known-bugs/BUG-142.md)（Windows Terminal + PowerShell + GJI）で、
以下の状態遷移が実機で確認されている（ユーザー言語化）:

```
IME OFF, Engine OFF
  → 変換キー1回タップ →
IME ON, Engine OFF
  → 半角/全角キー1回タップ →
IME ON, Engine ON
  → 半角/全角キー1回タップ →
IME ON, Engine ON  ← 以降、何回半角/全角を押してもここに固着
```

固着は実際に文字を打って確認済み（かなのまま変わらない）。

### VKと方向の固定マッピング（`vk.rs`）

`crate::vk`は`VK_DBE_SBCSCHAR`(0xF3)を「半角モード（IME OFF扱い）」、
`VK_DBE_DBCSCHAR`(0xF4)を「全角モード（IME ON）」という**固定の絶対方向**
として分類している。`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle`は
この固定方向（`ShadowImeAction::TurnOn`/`TurnOff`）を`event.ime_relevance.
shadow_action`としてそのまま採用し、`action.resolve(current)`で`new_val`を
計算する。`new_val == current`のとき（`:1544`）は「actuate不要」と判断し
`GjiDirectStrategy`の送信自体を呼ばない（`[shadow-toggle] no-op`ログ）。

固着中、物理半角/全角キーを押すたびにOSが**同じVK（0xF4、TurnOn方向）を
繰り返し報告**することを実機で確認済み（`docs/known-bugs/BUG-142.md`
「固着の完全な再現とログでの機序特定」節）。ユーザーの意図は明らかに
「OFFにしたい」だが、awase側は「TurnOn方向で、既にON→変更不要」と
機械的に判断し、送信を一度も試みない。

### 決定的な検証: 送信経路自体は生きている（Ctrl+無変換で実機確認）

固着状態のままCtrl+無変換（`ime_controller.rs::GjiDirectStrategy`が送る
`VK_IME_OFF`(0x1A)の通常のSendInput、`UserIntentSource::Command`経由）を
押すと、**実タイピングで確認可能な形で実際にIMEがOFFになる**ことを
実機確認した。すなわち:

- `WM_IME_CONTROL`（`IMC_SETOPENSTATUS`/`IMC_SETCONVERSIONMODE`）直接書込み
  → 実タイピングでは効かない（API`success`は返るが実TSFエンジンに届かない）
- 物理DBEキーのVK直接SendInput（`VK_DBE_SBCSCHAR`注入） → 実タイピングでは
  効かない
- `VK_IME_ON`/`VK_IME_OFF`のSendInput（`GjiDirectStrategy`の通常経路、
  Ctrl+無変換/変換が使うのと同じ） → **実タイピングで確認可能な形で効く**

この3点セットの実験結果から、**固着の原因はGJIへの配送失敗ではなく、
awase自身が`GjiDirectStrategy`の送信を試みていないこと**に絞り込まれる。

## なぜOS側の方向報告が同じ値に固定されるのか（未解明、本ADRの非スコープ）

物理DBEキーが同一方向のVKを繰り返し報告するようになる正確な機序（OS/
キーボードドライバ内部の状態か、awase自身のフックが物理キーを常時
Suppressすることの副作用か）は、2026-09-15の複数回の実機検証（drift
correction・ADR-121・charset軸デッドロックの3仮説を実機で棄却済み、
`docs/known-bugs/BUG-142.md`参照）でも特定できていない。**本ADRはこの
「なぜOSが誤った方向を報告するのか」の解明を目的としない**——awase側の
shadow-toggleが「OSの報告する方向を無条件に信頼して送信を省略する」設計に
なっていること自体が、たとえOS側の異常が別途修正されても再発しうる脆さ
であるため、awase側の判断ロジックを堅牢化することを目的とする。

## 決定（案、opus-adversarial-consult未実施）

物理半角/全角キー（`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`）の**新規押下**
（auto-repeatではない、`was_down: false → true`の遷移）が、direct
mapping方向で解決した`new_val`が`current`と一致する（no-op）状態を
**N回連続**で記録した場合、Ctrl+無変換/変換と同型の「現在状態に関わらず
明示的に反対方向を設定する」`GjiDirectStrategy`呼び出しにフォールバック
する。

### 検討すべき設計上の論点（opus-adversarial-consultで詰める）

1. **auto-repeatとの区別**: 物理キーを押しっぱなしにした場合の
   auto-repeat KeyDownは同じVK・同じ方向を繰り返すのが正常であり、
   これをno-opカウントに含めてはならない。`hook.rs::HOOK_STATE.
   physical_key_state`（`was_down`）を使った新規押下判定が必要。
2. **カウントのリセット条件**: 「N回連続no-op」のNとリセット条件
   （フォーカス変更、他のIME操作の成功、タイムアウト等）を決める必要が
   ある。リセットが緩すぎると正常なケース（本当に「既にON」を確認して
   いるだけの連打）まで誤ってフォールバックを発火させ、Ctrl+無変換型の
   無条件送信を過剰に行うリスクがある（余分なactuationがGJIのTSF
   composition経由で別の副作用を誘発しないか、ADR-137/BUG-46等の既存の
   「二重actuation」教訓と照合すること）。
3. **`ime-belief-architecture.md`との整合**: フォールバック送信は
   `GjiDirectStrategy::apply`（`ImeEvent`経由の`reduce()`を必ず通す）を
   使い、ここで新しい観測偽装（`ObserverReported`の`source`偽装等）や
   `desired_open`の直接書き換えを行わないこと。
4. **他のDBE系キー（`VK_DBE_ALPHANUMERIC`/`KATAKANA`等）への適用範囲**:
   本BUGは`VK_DBE_SBCSCHAR`/`DBCSCHAR`（半角/全角キー）で確認されたが、
   同じ固定方向マッピング設計を持つ他のVKにも同型の固着リスクがあるか
   確認し、スコープを決める。
5. **物理DBEキーの直接Suppressをやめる代替案との比較**: フォールバック
   送信を追加する代わりに、そもそも物理DBEキーをSuppressせず素通しする
   （ADR-173/ADR-174が検討した方向）という設計変更も再検討の余地がある
   ——ただし`feedback_immcross_owns_kanji`の設計原則（ImmCrossアプリには
   物理IMEキーを見せない）やBUG-52（`docs/known-bugs.md`参照、素通しする
   と実IMEがネイティブ効果を暴走させる）と衝突するため、今回はSuppress
   構造自体は維持し、no-op検出後のフォールバック送信で対処する案を主
   決定とする。

## 非スコープ

- OS側がなぜ同一方向のVKを繰り返し報告するようになるかの根本原因解明
  （上記「なぜOS側の方向報告が...」節参照）。
- ADR-174（パススルー+belief再観測）・ADR-173（`solo_tap_ime_action_apps`）
  の設計自体の変更——両ADRとも保留のまま、本ADRとは独立に扱う。

## 次のアクション

1. 本ADRをopus-adversarial-consultにかけ、上記「検討すべき設計上の論点」
   （特にauto-repeat除外・Nの決め方・過剰フォールバックのリスク）を詰める。
2. 実装後、BUG-142の再現手順（変換キー1回→半角/全角キー2回以上）で固着が
   解消することを実機で確認する（実タイピングでの確認を必須とする、
   `WM_IME_CONTROL`等のAPI読み取りのみでの確認は不可——本BUGの調査で
   API成功表示が実TSFエンジンの挙動と乖離することが判明しているため）。
3. `crates/awase-windows/tests/ime_key_sequence_golden.rs`等、キー選択の
   reincidence family向けテストへの回帰テスト追加を検討する
   （`.claude/rules/fix-requires-evidence.md`参照）。

## 関連

BUG-142（本ADRが解消を目指す「IME ON固着」）、ADR-121（物理IME訂正キー
no-op時の冪等再送、VK不一致により本BUGとは無関係と判明・棄却）、
ADR-153（`explicit_ime_action_target`、無変換/変換の明示config設計）、
ADR-172（TsfNative ON方向救済4系統の整理）、ADR-173
（`solo_tap_ime_action_apps`、却下された代替案）、ADR-174
（パススルー+belief再観測、Blocker未解消のまま保留）。
