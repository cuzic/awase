---
id: ADR-141
title: |-
  無変換/変換 delegate-to-open-axis の TurnOn 方向構造的到達不能問題（C2）の解消
summary: |-
  変換/無変換キーのdelegate-to-open-axisが、対象キーが親指キーでない場合や活性化条件を満たさない場合に不活性のまま復旧しない問題への対応
status: |-
  実装済み（PR #177でdevelopマージ済み）
related_adr:
  - "ADR-092"
  - "ADR-119"
  - "ADR-135"
---

# ADR-141: 無変換/変換 delegate-to-open-axis の TurnOn 方向構造的到達不能問題（C2）の解消

## ステータス

**設計確定（Opus敵対的レビュー2ラウンドで収束）・実装済み（PR #177、
BUG-118として起票）。実装後の`/code-review`でMS-IME経路の非親指キー
ケースの二重actuation・`muhenkan_solo_tap_dedicated_fn_key`との優先順位
未考慮の2件を発見・修正済み。実機ソーク（必須条件6）は未実施。**
対象は develop ブランチ。

**経緯**: 初稿は「`route_thumb_key_action` を排他振り分けから
actuation-auto への二重登録に変える」案（本文末尾「棄却した原案」参照）
だったが、Opus敵対的レビュー1ラウンド目で **Blocker 2件・Major 5件**
が指摘され、実コード確認の上で原案・その場しのぎの修正版（代替案B）
とも棄却。**Hiragana/Katakana で同型の問題が ADR-135 で既に解決済み**
という決定的な先例が見つかり、その解法（shadow-toggle 経路への合流）
を無変換/変換にも水平展開する方針（代替案A）へ全面転換した。

2ラウンド目のレビューでは方針自体（代替案A）は正しいと確認された上で、
新たに **Blocker 1件（安全性論拠の事実誤認）・Must-fix 3件・
Should-fix 2件** が見つかり、いずれも設計転換ではなく記述精度・配線
漏れの修正として反映済み（「なぜこの方式を選んだか」項目2の訂正、
「残存する既知の限界」節の新設、「具体的な変更点」への GJI 離脱時
クリア/`turn_on_direction`アーム追加/フィールド命名決定の反映、必須
条件への「eisu_recovery.rs SSOT更新」追加）。Opus からは「反映すれば
収束」と判定されている。

## 背景

### 既存の仕組み（要約）

GJI（Google 日本語入力）が `config1.db` の設定（overlay_keymaps / CUSTOM
キーマップの literal トークン / ATOK プリセット）で無変換/変換キー単体に
IME ON・OFF・トグルの意味論を割り当てている場合、awase は
`gji_charset_autodetect.rs::classify_mode_key_ime_action` でそれを
`ImeToggleKind::{On,Off,Toggle}` として検出し（[ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)
決定D Step4b、[ADR-135](135-generic-thumb-key-ime-toggle-delegate.md)、
BUG-115）、無変換/変換が NICOLA の `left_thumb_key`/`right_thumb_key`
（チョードキー）として設定されている場合は `henkan_delegate_to_open_axis`/
`muhenkan_delegate_to_open_axis`（`src/engine/nicola_fsm.rs`）へ反映する。
`resolve_pending_thumb_as_single`（単独タップ確定判定）の内部でのみ評価
される。

### C2: 発見された構造的欠陥（実機確認済み、2026-09-05）

[ADR-135](135-generic-thumb-key-ime-toggle-delegate.md#実装レビューで発覚したc2既存の欠陥phase-1発見時点では未検出-henkanmuhenkan-delegateのturnon方向は構造的に発火できない)
（[docs/known-bugs.md](../known-bugs.md) BUG-115 節にも記録）で発見された
既存の欠陥。`resolve_pending_thumb_as_single` は `NicolaFsm` のチョード
処理の一部であり、`src/engine/engine.rs::on_input_body` の Phase 2 で
`!self.compute_active(ctx)` の場合に Phase 3（`NicolaFsm` 呼び出し）へ
到達する前に `Decision::pass_through()` で早期 return する（本 ADR
執筆時に `on_input_body` を直接確認して確定済み）。

```
Phase 1: check_special_keys(ctx, &event)   // ime_on/off/toggle・auto系はここ
Phase 2: if !compute_active(ctx) { return PassThrough; }   // ← ここで打ち切り
Phase 3: NicolaFsm::on_event(...)          // resolve_pending_thumb_as_single はこの内部
```

`compute_active` は `user_enabled && is_japanese_ime && ctx.ime_on &&
is_romaji_capable` の AND。したがって **IME が OFF（`ctx.ime_on ==
false`、`InactiveReason::ImeOff`）の間は Phase 3 に到達できず、delegate
は評価すらされない**。これはまさに「OFF→ON復帰」をさせたい場面そのもの
であり、delegate の `TurnOn`（あるいは `Toggle` のON側）は**原理的に**
発火しない。

Phase 2 で早期 return した場合、物理キーはそのまま OS へ `PassThrough`
される。つまり「awase は何もしないが、GJI 自身がその物理変換キーを
受け取って自分の設定（overlay/ATOK/CUSTOM）どおりに IME を ON にする」
ことがあり得る——IME は（GJI自身の力で）ON になるが、**awase の belief
（`ImeModel`/`InputContext::ime_on`）がそれに追随しない**ため
`compute_active` が false のままになり、NICOLA のローマ字→親指シフト
変換が起動しない、というのが BUG-115 の元症状の直接の機序である。
追随はやがて受動的な観測（TsfNative の conv 観測等）で訂正されるが、
数秒かかる上、観測不能なアプリ（`Imm32Unavailable`、BUG-115 の元報告
環境である UWP）では訂正手段自体が存在しない。

**必要なのは「actuation（awase自身がIMEを操作すること）」ではなく
「追随（GJIが自力でONにした事実にbeliefを合わせること）」である**——
この一点が、後述の設計転換の核心になる。

## 決定

### 決定的な先例: Hiragana/Katakana では同型の問題が ADR-135 で既に解決済み

Opusレビューで、`crates/awase-windows/src/runtime/key_pipeline.rs:1137-1150`
（`kp_stage_shadow_ime_toggle` 内）に、**C2 とまったく同じ問題に対する
解決策が既に実装・出荷済み**であることが指摘された（ADR-135 の C1 修正）:

```rust
// Phase 3 delegate が「所有」するのは、親指キー×delegate armed に加えて
// エンジンが活性(=belief ON)である場合だけ（C1、Opus実装レビュー指摘）。
// (中略) つまりbelief OFFの間、delegateは構造的に発火できない。
// ここでbelief状態を見ずに「親指キー×armed」だけでshadow-toggleを
// 止めると、delegateも発火せずshadow-toggleも止まる「誰も何も
// しない」状態を作り、IME OFFからひらがな/カタカナ親指キーで
// 復帰できなくなる（BUG-115の元症状そのものの再現、観測できない
// アプリ——UWP等——では恒久的に固着する）。
let delegate_owned = self.mode_key_delegate_owns_shadow_toggle(event.vk_code)
    && self.platform_state.ime.effective_open();
```

このコメントは C2 の症状（「delegate は belief OFF の間は構造的に発火
できない」「UWP 等では恒久固着」）を一字一句記述している。つまり
ADR-135 のレビュー時点で C2 の機序は既に理解され、Hiragana/Katakana に
ついては **「belief OFF の間は delegate に譲らず、shadow-toggle（＝
物理キーを素通ししたまま belief を追随させる、`hook.rs`→
`ImeKeyKind::from_vk`→`key_pipeline.rs::kp_stage_shadow_ime_toggle`の
既存機構）に処理させる」** という形で解決済みである。解決の形は
**actuation の追加ではなく、追随機構への合流**だった。

無変換/変換だけがこの機構に乗れないのは1点のみが原因: `crates/
awase-windows/src/vk.rs::ImeKeyKind::from_vk`（122-136行）が
`0x15/0x16/0x17/0x19/0x1A/0xF0-0xF4` のみを返し、`0x1C`(VK_CONVERT)/
`0x1D`(VK_NONCONVERT) を含まない。したがって `focus_tracker.
enrich_ime_relevance` が静的な `shadow_action` を付けず、
`kp_stage_shadow_ime_toggle` の `intent_kind` が常に `None` になり、
この経路に一切乗らない——`gji_charset_autodetect.rs`（205-211行）の
doc コメントも同じ事実を明記している。

### 採用する修正方針: 無変換/変換を shadow-toggle 経路に合流させる

Hiragana/Katakana で確立済みの「`gji_*_shadow_override` →
`enrich_ime_relevance` → `kp_stage_shadow_ime_toggle`（`&&
effective_open()` で delegate と排他）」という**単一の既存機構**に、
無変換/変換も対称的に接続する。新しい機構は作らない。

具体的な変更点（4点、Opus再レビューでMust-fix/Should-fix反映済み）:

1. `Runtime` に無変換/変換用の shadow override フィールドを追加する
   （既存の `gji_hiragana_shadow_override`/`gji_katakana_shadow_override`、
   `runtime/mod.rs` と対称）。**命名は `gji_` 接頭辞を付けない
   `henkan_shadow_override`/`muhenkan_shadow_override` とする**
   （Opus再レビュー Should-fix (b)）——必須条件3で MS-IME 側も同じ
   フィールドに書き込むため、GJI 専用を示す `gji_` 接頭辞は誤称になる。
   これは `henkan_delegate_to_open_axis`/`muhenkan_delegate_to_open_axis`
   が既に採用している「GJI/MS-IME 両方が書き込む共有フィールド＋
   呼び出し順序（GJI→MS-IMEの順、`gji_charset_autodetect.rs:614-619`）
   への依存」という確立済みパターンをそのまま踏襲する（Hiragana/
   Katakana 版のような新規パターンを増やさない）。
   `sync_gji_charset_autodetect` が `wiring.henkan`/`wiring.muhenkan`
   から `ime_toggle_kind_to_shadow_action` 経由で設定する。
   **GJI離脱時のクリアを忘れないこと**（Opus再レビュー Must-fix、
   下記「GJI離脱時のクリア」参照）。
2. `Runtime::enrich_ime_relevance`（`runtime/mod.rs:435-447`）で、
   VK_CONVERT/VK_NONCONVERT にもこの override を差す。**ただし
   Hiragana/Katakana 版 `resolve_mode_key_shadow_override_for_event`
   （`gji_charset_autodetect.rs:449-452`）が持つ「親指キーなら `None`
   を返す」早期 return はそのまま流用しない**——あちらは静的
   `shadow_action`（`ImeKeyKind::from_vk` 由来）を壊さないための配慮
   であり、無変換/変換にはそもそも守るべき静的値が存在しないため、
   親指キーとして設定されている場合でも override を差す必要がある。
3. `delegate_owns_mode_key_shadow_toggle`（`gji_charset_autodetect.rs:
   464-473`）と `mode_key_delegate_owns_shadow_toggle`（`runtime/mod.rs:
   1380-1390`）を無変換/変換にも拡張し、`henkan_delegate_to_open_axis()`/
   `muhenkan_delegate_to_open_axis()` も見るようにする。あとは既存の
   `&& effective_open()` ゲートが自動的に C2 を解消する——belief OFF
   の間は delegate ではなく shadow-toggle が処理し、belief ON の間は
   （既存どおり）delegate がチョード安全に処理する。
   **`key_pipeline.rs` の `turn_on_direction` match（1265-1280行付近）
   に無変換/変換のアームを追加する**（Opus再レビュー Should-fix (a)）
   ——現状 `VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA` のみで `_ => None` →
   `.unwrap_or(action)` にフォールバックしており、今日は `action`
   （GJI override）と delegate が同じ `wiring.henkan` 由来なので偶然
   一致するが、必須条件3（MS-IME配線）を入れると乖離しうる。MS-IME側は
   レジストリから delegate を設定するため、GJI override が未設定/stale
   な状態と組み合わさると方向が食い違い、TurnOff方向のdelegateなのに
   TurnOn向けのeisu救済を誤って走らせる（この`match`自体が同じ理由の
   /code-review指摘で追加された前例がある）。
4. **GJI離脱時のクリアに新フィールドを追加する**（Opus再レビュー
   Must-fix）。`sync_gji_charset_autodetect` の `!is_gji` 分岐
   （`gji_charset_autodetect.rs:620-632`）は現在
   `set_gji_mode_key_shadow_overrides(None, None)`/
   `set_gji_mode_key_delegate_to_open_axis(None, None)`/
   `set_gji_thumb_key_delegate_to_open_axis(None, None)` をクリアして
   いる。新設する2フィールドをここに追加しないと、GJI由来のstale
   overrideが非GJI文脈へ無期限に残留する——同箇所のコメントは、まさに
   `set_gji_thumb_key_delegate_to_open_axis`の行が過去に抜けていて
   「無関係なアプリでの単独タップがIME状態を静かに反転させる」バグを
   踏んだ経緯を記録している。同じ轍を踏まない。

### なぜこの方式を選んだか

1. **新設計ではなく、既に出荷済みの解法の水平展開**である。合流点が
   増えない——4つの mode key（Hiragana/Katakana/Henkan/Muhenkan）が
   単一機構に収束する。`fix-requires-evidence.md` が警告する「IME
   actuation 合流点」をこれ以上増やさない。
2. **`TurnOn` 方向（C2 が問題にしている方向）は belief 追随のみで、
   `kp_stage_shadow_ime_toggle` 自身は能動的な actuation を行わない**
   （Opus再レビューで訂正: 初稿の「actuate も consume もしない」は
   不正確だった。`key_pipeline.rs` の OFF→ON 分岐は stale Eisu 救済
   （`eisu_reset_on_ime_on`）のみを行い、open軸への能動的 actuation
   （`ImeController::apply`/`run_open_chain_async`）は行わない、
   1306-1333行——ただし半角英数持続トグルON中は
   `kp_restore_kana_from_half_width`経由でSendInputが発生しうるが、
   これはopen軸のactuationではなくHiragana/Katakanaの既存挙動と同一
   であり新規リスクではない）。したがって **InputRelay でも GJI
   自身の ON が生きる**（`transport.rs::plan` が `Allow` を返す限り、
   awase 側は actuate しようとすらしないため `NotOwned` の空振りすら
   発生しない）。

   一方 **`TurnOff` 方向は belief 書き込みに加えて能動的 actuation を
   行う**（`key_pipeline.rs:1349-1395` の ON→OFF 分岐、
   `ImeController::apply`/`run_open_chain_async`）——これは
   Hiragana/Katakana の既存挙動と同一で、deactivation が activation
   のような自動 `SetOpen` 相当を持たないための対称的な補完である
   （同ファイルの「activation (inactive→active) が
   `ImeEffect::SetOpen(true)` を生成して OS IME を強制 ON するのと
   対称な処理」というコメント参照）。InputRelay 下ではこの actuation
   試行も `NotOwned` で空振りするだけだが、これは Hiragana/Katakana
   等の既存キーでも既に起きている挙動であり、本 ADR が新規に導入する
   リスクではない。

   オートリピートが安全なのは「actuate しないから」ではなく、
   `effective_open() == current` の冪等 no-op 分岐（`key_pipeline.rs:
   1234`付近）が2回目以降のリピートを早期 return させるためである。
   **`Toggle`（opt-in時）だけはこの冪等性が効かない**——詳細と対応方針
   は下記「残存する既知の限界」節を参照。
3. **原案（後述）が持っていた欠陥が構造的に発生しない**（次節参照）。
4. 症状に対して正確——BUG-115 の症状は「GJI の力で IME は ON になるが
   awase の belief が追随しない」であり、必要なのは追随機構であって
   actuation ではない。

### 必須条件（この6点を満たさない実装は別の退行を作る）

**1. `transport.rs::plan` に follow-only 例外を追加する（最重要）**

`shadow_action` を付けると、`transport.rs:322` の
`let is_kanji_event = event.ime_relevance.shadow_action.is_some(); if
!is_kanji_event { return Self::Allow; }` という early return を抜けて
しまう。現在 VK_CONVERT/VK_NONCONVERT は `shadow_action` が無いため
必ず `Allow` で通っているが、付けた途端に下流の Suppress 判定に入る:

- ImmCross アプリ（`profile.can_use_imm32_cross_process()`、
  `transport.rs:326-328`）→ KANJI 関連 VK は Down/Up ともに無条件
  Suppress。
- `ime_actuation_owned && (shadow_toggled || KeyUp)`（`transport.rs:
  363-367`）→ shadow-toggle 発火時の KeyDown と全 KeyUp を Suppress。

対策なしだと、LINE/Qt 系の ImmCross アプリで物理変換/無変換キーが
消え、「誰も何もしない」を別ルートで新規に作ってしまう。`InputRelay`
分岐（`transport.rs:271`）や `VK_DBE_HIRAGANA` 分岐と同じ位置、すなわち
`is_kanji_event` 判定より**前**に「無変換/変換の `shadow_action` は
belief 追随専用（follow-only）であり、物理配送は常に `Allow`」という
明示的な例外を置く。`transport.rs` 末尾の `plan_tests` に回帰テストを
追加する（`fix-requires-evidence.md` の再発ファミリー表に
`runtime/transport.rs::PhysicalKeyDisposition::plan` が BUG-46/52/116
として明記されており、`.git/hooks/pre-push` のチェック対象でもある）。

**2. decision table テストの `PipelineOutcome` 型を変更する**

`gji_charset_autodetect.rs:1427-1440` の `PipelineOutcome` は `Nothing |
Delegate | ActuationAuto | ShadowOverride` の排他 enum で、
`actual_outcome_henkan_muhenkan`（1499-1530行）は該当する帰結が `Some`
なら他を見ずに早期 return する。本 ADR の変更では、親指キー設定時の
帰結が「delegate armed **かつ** shadow-override armed」という組に
なるため、型を変えないと既存テストが無改造で緑のまま通り、新機能の
検証がゼロになる。`expected_outcome`（1462-1494行）も、既存コメント
（1455-1461行）が明示する「本番コードのコピーにしない」方針に従って
仕様側を書き直す。

**3. MS-IME 経路も同時に配線する**

`message_handlers.rs:844-847` の `sync_ime_toggle_auto_detect` は、
レジストリ `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan` から
`set_muhenkan/henkan_delegate_to_open_axis` を呼ぶだけで、shadow
override 相当を一切設定していない。C2 は delegate 機構そのものの
構造的欠陥なので、GJI 側だけ直すと MS-IME で同じ設定をしているユーザー
には C2 がそのまま残る。本 ADR のスコープに含め、同時に配線する
（GJI 版と対称な override 設定を MS-IME 側の同期経路にも追加する）。

**4. `docs/known-bugs.md` に C2 を正式な BUG 番号で起票する（実施済み: BUG-118）**

調査時点では ADR-135 に「別issue化」と書かれたまま、実際には起票
されていなかった。`main-develop-branch-flow.md` に従い develop 経由で
記録する。`fix-requires-evidence.md` の (a) 回帰テスト（決定2、
`gji_charset_autodetect.rs`の`gji_detection_to_application_pipeline_
decision_table`拡張・`transport.rs::plan_tests`新設）と (b) known-bugs.md
記録（[BUG-118](../known-bugs.md)）の両方を満たす。

**5. `eisu_recovery.rs` の経路×救済 SSOT を更新する**（Opus再レビュー
Must-fix）

`.claude/rules/ime-belief-architecture.md` は「IME を ON にする経路を
追加したら、stale `ObservedEisu` の救済を必ず対で配線し、経路×救済の
対応表（`state/eisu_recovery.rs` のmodule doc がSSOT）を更新すること」
と定めている。本ADRは無変換/変換を `write_physical_key(` 経由の
IME-ON経路に新たに乗せるため、この module doc に追記が必要。あわせて
`crates/awase-windows/tests/architecture_guard.rs` の
`user_ime_on_paths_are_paired_with_eisu_reset`/
`apply_ime_open_with_belief_call_sites_are_accounted_for`
（`write_sync_key(`/`write_physical_key(`/`write_set_open_request(` の
出現箇所をファイル名と件数のペアで直書きする件数固定テスト）が、
既存の呼び出し箇所を再利用するだけなら件数不変のはずだが、必須条件3
（MS-IME側の新規配線点追加）で件数が動きうるため、緑であることを
実装完了時に確認する。

**6. 実機ソーク項目**

GJI アクティブ・IME OFF 状態で、GJI 側が overlay/ATOK プリセットで
変換キーに On/Toggle を割り当てている環境において、変換キー単独タップ
直後に NICOLA 変換が即座に有効化されることを確認する。加えて:

- active 時の通常のチョード打鍵（無変換/変換+文字キー）に回帰が無い
  こと。
- **ImmCross アプリ（LINE/Qt 系）で物理変換/無変換キーが Suppress
  されていないこと**（必須条件1の検証）。
- InputRelay（MWB/RDP）環境で変換キーが従来どおり GJI に届くこと。
- MS-IME で `KeyAssignmentMuhenkan`/`Henkan` を設定した環境でも同様に
  復帰すること（必須条件3の検証）。

## 残存する既知の限界（Opus再レビューで発見、対応せず記録のみ）

### Toggle opt-in 時のオートリピート

`Toggle`（`gji_thumb_key_ime_toggle=true` の opt-in 時のみ関係）は
`effective_open() == current` の冪等 no-op 分岐が効かない
（`new_val = !current` が毎回 `current` と異なるため）。したがって
変換/無変換キーを押しっぱなしにすると belief の反転とそれに伴う
OFF方向 actuation がオートリピート周期（~30回/秒）で連射される——
原案（棄却済み）が持っていた「オートリピート」Blocker が、Toggle
opt-in 時に限り本方式でも残存する。既定（opt-in無し）では無関係。
恒久対策（Phase 1相当のリピートラッチ新設）は本ADRのスコープ外とし、
実機ソークで実害が確認された場合に別途 issue 化する。

### `is_japanese_ime()` grace期間中の自己修復手段の欠如

`kp_stage_shadow_ime_toggle` の `intent_kind` 判定（`key_pipeline.rs:
1152-1161`）は `PhysicalImeKey` 分岐で `self.platform_state.ime.
belief.is_japanese_ime()` を要求する。Hiragana/Katakana 等の合成VK
（`0xF0`-`0xF4`等）は `should_upgrade_is_japanese_ime`（`vk.rs:
237-239`）により「このKeyDownが届くこと自体が日本語IMEの証拠」として
`is_japanese_ime()` を自己修復できるが、**無変換/変換は日本語キー
ボードに物理的に実在する実キーであり、届いたこと自体が日本語IMEの
証拠にはならない**ため、`should_upgrade_is_japanese_ime` に追加しては
ならない。

結果として、スリープ復帰/フォーカス変更直後の grace 期間中
（`key_pipeline.rs:1090-1095` が明記する `is_japanese_ime()` の既知の
弱点）に `is_japanese_ime()` が一時的に false を誤答している窓では、
Hiragana/Katakana は自己修復して shadow-toggle に乗れるが無変換/変換
は乗れず、**C2 がこの窓の間だけ残存する**。BUG-115 の元報告環境
（UWP、観測不能）とこの窓が重なる可能性があり、修復手段を追加する
場合は「GJI の config1.db が無変換/変換に IME 意味論を宣言している
と分かっている」ことを根拠にする別ルートが必要になる
（`should_upgrade_is_japanese_ime` へのVK追加とは別設計）。本ADRの
スコープでは対策せず、既知の限界として記録するに留める。

## 棄却した原案・代替案

### 原案（初稿）: `route_thumb_key_action` を actuation-auto へ二重登録

`route_thumb_key_action` を「delegate と actuation-auto への排他振り
分け」から「両方への無条件登録」に変える案。安全性の論拠は、既存の
ガード `suppress_ime_combos = engine_active && is_bare_thumb` が
active/非active で自然に排他分割するため二重発火しない、というもの
だった。Opusレビューで以下が判明し棄却:

- **Blocker（InputRelay で完全無反応化）**: 現状は非活性時に物理キーが
  `PassThrough` されるため GJI 自身が処理できるが、actuation-auto に
  登録すると Phase 1 で `Decision::consumed_with` され、InputRelay 下
  では awase 側も `NotOwned` で actuate しないため「二重の空振り」
  （ADR-119 が禁じるパターン）を新規に作る。「悪化させも改善もしない」
  という原案の主張は誤りだった。
- **Blocker（オートリピート）**: delegate はチョード確定時に1回しか
  発火しないが、actuation-auto は `is_key_down` ごとに毎回評価され、
  `RawKeyEvent` に repeat フラグが無いため押しっぱなしで `SetOpen` が
  連射される（`Toggle` opt-in 時は IME が点滅する）。
- **Major（Win修飾の穴）**: `is_bare_thumb` は `is_os_modifier_held()`
  （ctrl||alt||win）で判定するが、combo マッチ側は win を見ないため、
  engine **active**時でも Win+親指キーで新経路に到達し「active時の
  挙動は変更されない」という原案の前提が崩れる。
- **Major（`UserDisabled` 露出）**: 無変換3連打の緊急エンジンOFF直後、
  その無変換キー自体が IME を操作し始めるという分かりにくい挙動を
  新規に作る。
- **Major（テストが素通りする）**: 既存の decision table テストは
  排他 enum のままだと無改造で緑のまま通り、新機能の検証にならない。
- **Major（対称性の欠落）**: MS-IME 経路への配線が抜けていた。
- **Major（既存の設計方針との矛盾）**: `gji_charset_autodetect.rs` 自身
  が「Hiragana/Katakanaをactuation-autoへ載せる処理は、既存
  shadow-toggleとの二重actuationを作るため採用しない」と明記して
  おり、無変換/変換だけ逆方向に進む理由が無かった。

### 代替案B: 原案を維持しつつ非活性時は consume せず pass_through にする

Blocker（InputRelay）だけを潰す最小改修案。`ime_set_open_effects`/
`apply_special_key_match` に非consumeの入口を新設する必要がある上、
残る4つの問題（オートリピート・Win修飾・`UserDisabled`露出・テスト
形骸化）は個別に対策が必要で、加えて `Toggle`（非冪等）だけは
pass_through にできないため「方向によってconsumeするかどうかが変わる」
という新たな非対称を持ち込む——これは原案が自称していた「非対称な
特別扱いをしない」という方針と正面から矛盾する。5つの特別扱いを
新設するコストが、採用した方式（Hiragana/Katakanaの既存機構への合流、
新設0）を上回るため棄却。

## 関連

[ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)、
[ADR-135](135-generic-thumb-key-ime-toggle-delegate.md)（C1修正が本ADRの
解法そのもの）、[ADR-119](119-injected-and-relay-key-consumption-invariant.md)
（InputRelayの「解釈しない入力は消費しない」不変条件）、
[docs/known-bugs.md](../known-bugs.md) BUG-115、
[[project_henkan_muhenkan_ime_control_review_2026_09_06]]（本 ADR の
発端になった調査メモリ）。
