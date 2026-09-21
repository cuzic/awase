---
id: ADR-183
title: |-
  VK_KANA（かなキー）をADR-179の`ModeKeyActuationOwner::PhysicalDelivery`に
  合流させ、KeyUp無条件Suppressによる非対称パススルーを解消する設計
summary: |-
  「IME ON・半角英数(Eisu)・Engine ON中にかなキーを押してもひらがなモードへ
  戻らない」というユーザー報告の調査から起票。原因は`transport.rs::
  PhysicalKeyDisposition::plan()`の「KANJI系キー」共通分岐（374-419行目）で、
  VK_KANA(0x15)のKeyDownは`shadow_toggled`（この打鍵でIME open軸が実際に
  変化したか）を条件にSuppress/Allowを決めるのに対し、KeyUpは
  `matches!(event.event_type, KeyEventType::KeyUp)`が無条件にtrueを返すため
  `shadow_toggled`の値に関わらず常時Suppressされること。IMEが既に開いている
  （open軸が変化しない＝shadow_toggled=false）場合、KeyDownはAllowされるのに
  対応するKeyUpだけがフックに握りつぶされ、OS/GJI/MS-IME側は「離鍵の来ない
  かなキー」を受け取ることになり、モード切替（英数→ひらがな）が完了しない。
  ADR-179は`ModeKeyActuationOwner::PhysicalDelivery`（awase自身は一切
  actuateせず、物理キー配送のみでbeliefを追随させる）という型を無変換/変換の
  非親指キー配置に導入したが、`is_target_vk`（`key_pipeline.rs:1449-1452`）は
  `VK_CONVERT`/`VK_NONCONVERT`のみを対象にしており、VK_KANAはスコープ外の
  まま`AwaseExplicit`（awase明示actuate）分類に留まっている。本ADRは
  「VK_KANAはIME ONへの単一方向・冪等な物理キーであり、無変換/変換と同様に
  PhysicalDelivery化できるはず」というユーザーの設計意図を検証し、実装可否を
  opus-adversarial-consultで確認するためのドラフト。
status: |-
  **撤回（2026-09-19、実機検証により前提誤りと確定）**。opus-adversarial-consult
  round1（`docs/adr/183-opus-review-round1.md`）が「対象VKが本当にVK_KANAか
  未検証」「症状はconv軸(charset軸)の問題でADR-137 M-6/BUG-116と同一の
  可能性が高い」と指摘（M2/M1）。実機（Windows Terminal×GJI×TsfNative、
  RUST_LOG=debug）でIME ON・半角英数からかなキーを押す操作を4回検証した
  結果、実際に生成されるVKは`VK_KANA`(0x15)ではなく`VK_DBE_HIRAGANA`(0xF2)
  （scan=0x70）で、`kp_restore_hiragana_for_suppressed_mode_key`
  （ADR-137/BUG-116決定2）が4回とも`sent=true`で正常に発火し症状は再現
  しなかった。本ADRが提案していた「VK_KANAを無条件Allowにする」変更は、
  誤った前提（対象VK）に基づく設計であり、かつVK_KANAはOS側でロック型
  トグルキーのため実装していればBUG-08/BUG-61型の復旧不能な破損を新規に
  作っていた（round1 B1/B3参照）。教訓は
  (教訓: 通称のキー名からVKを仮定せず、実機ログで実際のVKを確認してから設計する)
  （auto-memory）に記録済み。当初の症状報告自体は未解決（ユーザーは同一
  環境で過去に再現していたとのこと）だが、原因はADR-183が調べた経路では
  なく、`kp_restore_hiragana_for_suppressed_mode_key`のガード条件
  （`is_composition_warm()`等）がタイミング依存で稀に不発になる経路の
  可能性が高く、別途タイミング条件を絞って追試する必要がある。
related_adr:
  - "ADR-179"
  - "ADR-181"
  - "ADR-137"
  - "ADR-100"
---

# ADR-183: VK_KANA を `ModeKeyActuationOwner::PhysicalDelivery` に合流させる設計

## 背景・症状

ユーザー報告（2026-09-19）: **IME ON・conv mode が半角英数（Eisu）・awase
Engine ON の状態で物理「かな」キー（このユーザーの環境では VK_KANA (0x15)
が生成される）を押しても、ひらがな入力モードに戻らない。**

ユーザーの設計意図: 「かな」キーは Microsoft の仕様上 IME を常に ON にする
単一方向・冪等（idempotent）なキーであり、OFF にすることは無い。したがって
awase は本来これを素のままパススルーし、belief（`desired_open`/`applied`）を
追随させるだけでよく、awase 自身が明示的に SendInput で actuate する必要は
ない、という設計方針で ADR-179 相当の整理を無変換/変換だけでなく VK_KANA
にも及ぼしたつもりだった。

## 現状の機序（コード調査で確認した事実）

1. `crate::vk::ImeKeyKind::from_vk(VK_KANA)` は `Self::Kana` を返し、
   `shadow_effect()` は常に `ShadowImeEffect::TurnOn`（`vk.rs:83-89,140-151`）。
   `hook.rs::classify_ime_relevance`（コード上は `enrich_ime_relevance` 系、
   VK 単独から機械的に計算）が `event.ime_relevance.shadow_action =
   Some(TurnOn)` を無条件に埋める。
2. `hook.rs:1254-1288` の VK_KANA 専用ブロックは、foreign-injected（BUG-08、
   OS のかなロック汚染防止）と Alt+VK_KANA（BUG-62、入力方式切替の復旧不能
   ハザード）の2パターンのみを swallow し、**物理（非注入）・Alt 非押下の
   単独 VK_KANA はログを出すだけでこのブロックを素通りし**、通常の
   `process_key_event` パイプラインに渡る。ここまではユーザーの意図
   （生キー自体はブロックしない）と一致している。
3. `key_pipeline.rs::kp_stage_shadow_ime_toggle`（1275行目〜）で `intent_kind`
   が `(TurnOn, PhysicalImeKey)` と解決され、`write_physical_key` で belief
   （open 軸）を更新する。IME が既に open の場合、`effective_open() ==
   current` が成立し 1612 行目の no-op 早期 return に入る
   （`shadow_toggled = false` を返す）。ここは意図どおり「belief 追随のみ」
   になっている。
4. **問題箇所**: `runtime/transport.rs::PhysicalKeyDisposition::plan()`。
   VK_KANA は `VK_DBE_HIRAGANA` 専用分岐（292-298行目）にも
   `VK_CONVERT`/`VK_NONCONVERT` 専用分岐（363-372行目、ADR-179 で新設）にも
   該当せず、汎用の「KANJI系キー」分岐（374-419行目）に落ちる:

   ```rust
   let is_kanji_event = event.ime_relevance.shadow_action.is_some(); // VK_KANA は true
   ...
   let ime_actuation_owned = key_sequence_policy::gji_direct_applicable(kind)
       || key_sequence_policy::ms_ime_direct_applicable(kind, profile);
   ...
   ime_actuation_owned
       && (shadow_toggled
           || is_dbe_mode_key_down   // VK_KANA には該当しない
           || matches!(event.event_type, KeyEventType::KeyUp))  // ← 無条件
   ```

   `ime_actuation_owned`（GJI/MS-IME direct strategy が成立する、単一 IME
   環境ではほぼ常時 true）が成立する下で:
   - **KeyDown**: `shadow_toggled` に応じて Allow/Suppress が決まる。IME が
     既に ON（上記3のケース）なら `shadow_toggled = false` → **Allow**。
   - **KeyUp**: `matches!(.., KeyUp)` が無条件に真のため、`shadow_toggled`
     の値に関わらず **常に Suppress**（`execute_relay`、`executor.rs:463-473`
     が `CallbackResult::Consumed` にして `CallNextHookEx` へ進ませない）。

   結果、**KeyDown は OS/GJI/MS-IME に届くが対応する KeyUp だけが消える**、
   という非対称なパススルーになる。IME モードキーの押下は「held down」状態
   のまま完結せず、モード切替（英数→ひらがな）が発火しないと考えられる
   （実機ログでの直接確認はまだ、下記「未解決の疑問」参照）。

5. **ADR-179 のスコープ外だった**: `ModeKeyActuationOwner` の主判定条件
   `is_target_vk`（`key_pipeline.rs:1449-1452`）は `VK_CONVERT` /
   `VK_NONCONVERT` のみを対象にしている。ADR-179 のタイトル・要約自体が
   「無変換/変換の非親指キー時actuation-auto撤去」であり、VK_KANA や
   `VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA` は最初から検討対象に含まれていない。
   ADR-181（ドラフト、別件）も「検討中の設計方向3」として同種の一般化
   （ひらがな/カタカナへの Passthrough 概念拡張）を「見送った案」として
   触れているが、こちらも未検証・未実装。

## 提案する設計変更（決定案、opus-adversarial-consult で検証してほしい）

`transport.rs::plan()` に、`VK_CONVERT`/`VK_NONCONVERT` 専用分岐
（363-372行目）と対称な **VK_KANA 専用分岐** を追加し、VK_KANA を汎用
「KANJI系キー」分岐（`is_kanji_event`）の対象から外す。

```rust
// 案（擬似コード、実装時に調整）
if event.vk_code == crate::vk::VK_KANA {
    return Self::Allow; // 常時パススルー。shadow_toggled/ime_actuation_owned を見ない
}
```

`ModeKeyActuationOwner` 側も `is_target_vk` に `VK_KANA` を加えるか、
VK_KANA 専用の分岐を新設して `PhysicalDelivery` に分類する
（`key_pipeline.rs:1449-1478` の書き込み点は1箇所に固定する契約
——`tests/architecture_guard.rs`——があるため、既存の1箇所を拡張する形にする）。

### 根拠: なぜ VK_KANA は無変換/変換と同種に扱えると考えられるか

- VK_KANA は `ShadowImeAction::TurnOn` **専用**（`TurnOff`/`Toggle` を
  生成しない、`vk.rs:140-151`）。無変換/変換のように「設定次第で
  ON/OFF/Toggle のどの方向にもなりうる」キーとは異なり、方向が固定
  かつ idempotent（既に ON でも副作用なく ON のまま）。
  「awase が明示 actuate を代行する必要がある」というモチベーション
  （方向を誤ると belief と実 IME が逆方向に乖離しうる、ADR-179 の
  `owner` 定義参照）が、そもそも VK_KANA には当てはまりにくい。
- 二重 actuation 防止（BUG-46、`ime_actuation_owned` Suppress の本来の
  目的）は「awase が SendInput で ON/OFF を明示送信するのと、物理キーが
  素通りして IME 自身が反応するのが両方起きると事故る」ケースを防ぐもの
  だが、VK_KANA が引き起こす効果は「ON にする」の一択であり、awase 側が
  仮に別途 ON を送っていたとしても、物理 VK_KANA も届いて二重に「ON に
  する」だけなら idempotent で実害が無いはず、という仮説（**要検証**、
  下記「未解決の疑問」参照）。

## 変更しない範囲（意図的にスコープ外）

- `hook.rs` の foreign-injected VK_KANA swallow（BUG-08）と Alt+VK_KANA
  swallow（BUG-62）は変更しない。本 ADR は「物理・非 Alt 併用の単独
  VK_KANA」のみを対象とする。
- `VK_DBE_HIRAGANA`（0xF2）/`VK_DBE_KATAKANA`（0xF1）は対象外。これらは
  ADR-100/ADR-137 が扱う TSF warmup（`f2_warmup_owned`）・Shift+かな→
  カタカナ（BUG-116）という別の制約と絡んでおり、本 ADR のスコープに
  含めると論点が混ざる。ADR-181 が扱おうとしている領域であり、そちらで
  別途検討する。
- ImmCross プロファイル（`profile.can_use_imm32_cross_process()`）向けの
  「物理 IME キーを一切見せない」という既存の設計原則
  （`feedback_immcross_owns_kanji` 記憶、KANJI 系キー Down/Up 共に無条件
  Suppress）を VK_KANA にも適用し続けるか、除外するかは **未確定**
  （下記「未解決の疑問」）。

## 未解決の疑問（opus-adversarial-consult で検証してほしい点）

1. **KeyUp 無条件 Suppress が実際に症状の直接原因か**: 実機ログ
   （`[hook] VK_KANA down/up`、`[relay-*]` 系トレース）で、IME ON かつ
   Eisu 状態でかなキーを押した際に KeyDown は OS へ届くが KeyUp が
   `Consumed` になっている、という一次証拠をまだ取っていない。design を
   確定する前に実機で確認すべきか、それとも今回のコード調査（transport.rs
   の条件式を読んだだけ）で十分と判断してよいか。
2. **VK_KANA 起点の OFF→ON 遷移（`shadow_toggled=true` のケース）で、
   今日 awase は実際に何が IME を ON にしているのか**: 本 ADR は
   `shadow_toggled=false`（IME 既に ON）のケースを中心に調べたが、
   IME が OFF から VK_KANA で ON にする場合、現状の `plan()` は
   `ime_actuation_owned && shadow_toggled=true` で **KeyDown も Suppress**
   する。この場合に実際に IME を ON にしている経路（awase の明示 SendInput
   か、`Decision::find_ime_set_open_with_origin` 経由の別 effect か）を
   特定しないと、VK_KANA を無条件 Allow にした際にこの経路と物理キーが
   衝突しないか判断できない。
3. **ImmCross プロファイルでも VK_KANA を Allow してよいか**:
   `feedback_immcross_owns_kanji`（「ImmCross アプリには物理 IME キーを
   見せない」設計原則）は KANJI 系キー全般を対象にしている。VK_KANA だけ
   例外化してよい積極的な理由があるか、それとも ImmCross は現状維持
   （Suppress のまま）で GJI/MS-IME 系（TsfNative/Imm32Unavailable）のみ
   Allow 化するのが安全か。
4. **BUG-14 型の再発リスク**: foreign-injected VK_KANA は `hook.rs` で
   既に swallow されているため `transport.rs::plan()` に到達する時点で
   `event.injected` は基本的に false のはずだが、`plan()` 自身にも
   `event.injected` チェック（318-325行目）がある。VK_KANA 専用分岐を
   `event.injected` チェックより前に置くか後に置くかで、この既存の
   BUG-14 保護をすり抜けるルートを新設しないか確認が必要。
5. **`ModeKeyActuationOwner::PhysicalDelivery` への合流は本当に必要か、
   `transport.rs` 側だけの修正で十分か**: `ModeKeyActuationOwner` は
   `ActivationSync` effect の除去（`key_pipeline.rs:457-463`）等、
   `transport.rs::plan()` とは独立した消費点を複数持つ
   （`.claude/rules/fix-requires-evidence.md` の「IME actuation合流点」表
   参照）。`transport.rs` の Suppress/Allow だけ直しても、他の消費点が
   古い `AwaseExplicit` 前提のままだと不整合が残らないか。
6. この修正は `.claude/rules/fix-requires-evidence.md` の「キー選択」
   ファミリーに該当するため、golden テスト（`ime_key_sequence_golden.rs`
   または `transport.rs::plan_tests`）か `docs/known-bugs/BUG-NNN.md` の
   いずれかが必要。`plan_tests` に VK_KANA の Down/Up 対称性を固定する
   テストを追加するのが適切か。
