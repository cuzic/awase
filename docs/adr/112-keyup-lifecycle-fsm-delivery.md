# ADR-112: `Engine::on_input` Phase 0 が KeyUp を FSM に一切届けていない欠陥の修正

## ステータス

**設計確定（2026-08-31、Opus 2体によるpremortem 2ラウンドで収束）。未実装。**
`docs/known-bugs.md` BUG-101 に対応する修正設計。実装は本ADRの決定0〜2を4コミットに分けて行う。決定3（`min_overlap_margin_percent` の実運用値への引き戻し）は実機ソーク後に別ADR/別コミットで扱う、本ADRのスコープ外。

## コンテキスト

`feat/confirm-mode-simplify` ブランチで `timing_margin_percent`/`min_overlap_margin_percent` を設定可能にする作業中、`Engine::on_input` 経由で「char1押下→thumb押下→char1 KeyUp→タイムアウト」というシナリオの回帰テストを書いたところ、bareな `NicolaFsm` 直呼びテストとは異なる（誤った）結果になることを発見した。調査の結果、以下が判明した。

### 根本原因

`Engine::on_input`（`src/engine/engine.rs:357-362`、実運用で唯一のキーイベント入口——`crates/awase-windows/src/runtime/key_pipeline.rs:192` の1箇所のみから呼ばれることを裏取り済み）の「Phase 0」:

```rust
let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
if !is_key_down && self.lifecycle.on_key_up(event.vk_code) {
    return Decision::consumed();
}
```

`KeyLifecycle::on_key_up`（`src/engine/key_lifecycle.rs:56-63`）は、対応するKeyDownが `Decision::is_consumed()==true` だった場合、そのKeyUpを**無条件に** `Decision::consumed()` として即returnする。`NicolaFsm::on_key_up`（`nicola_fsm.rs:1582`、`FsmAdapter::on_event` 経由）には一切到達しない。

この仕組み自体の目的（ADR-020: 「OSへKeyDownを渡さなかった(Consume)なら、対応するKeyUpも必ずOSへ渡さない(Consume)」という対称性の保証）は正しい要求だが、現在の実装は**「OSに渡さない」と「FSMに渡さない」を同じ早期returnで一緒くたにしている**。

### git履歴 — これはリグレッションであり意図的設計ではない

| 対象 | commit | 日付 |
|---|---|---|
| `handle_key_up_pending_char_thumb` の初出（PendingCharThumbのKeyUp解決ロジック） | `b9d0ed56` | 2026-03-28 |
| `KeyLifecycle` 構造体の追加 | `94c73a15` | 2026-03-31 |
| Phase 0早期returnの `Engine::on_input` への配線 | `1b4c08bb` | 2026-03-31 |
| `char1_released_at`/`overlap_only_verdict` の導入（PR #85） | `b1e7474e` | 2026-08-23 |

KeyUpベースの解決ロジックが先（3/28）、Phase 0が後（3/31）。Phase 0混入の瞬間に、既存のKeyUp処理（`handle_key_up_pending_char_thumb`/`handle_key_up_pending`/`handle_key_up_active`/`SpeculativeChar`分岐）がまとめて到達不能になった。ADR-020は「OSに渡さない」しか決めておらず、「FSMにも渡さない」は記載されていない＝意図しない副作用。PR #85（8/23）は、既に死んでいた経路の上に新機能を積んだ被害者である。

### 確定した実害（3件、`docs/known-bugs.md` BUG-101に集約して起票）

1. **`min_overlap_margin_percent` が実運用で常に無効。** `char1_released_at` は実運用では恒久的に `None` のままなので、`timing::overlap_only_verdict`（`timing.rs:254-267`）は常に「char1がまだ押下中」扱いの `Some(true)` を返す。char1を離してから親指キーを押した（＝物理的に重なっていない）2打が、常に同時打鍵として誤確定される。
2. **`KeyAction::Key(vk)` を出力する全キーで、対応する `KeyUp(vk)` が実運用で一度も送出されていない（stuck key）。** `handle_key_up_active`（`nicola_fsm.rs:1812-1830`）だけが `KeyUp(vk)` を再送する経路だが到達不能。`KeyAction::Key(vk)` は `.yab` の明示的なVK指定行だけでなく、`resolve_pending_char_as_single`（`nicola_fsm.rs:1272`、配列定義外キーのフォールバック）や無変換/変換/Space/Enterのsolo-tap passthrough経路（`:1404`/`:1443`付近）からも常に出るため、**エンジンON中に非かな文字（レイアウト定義外のキー等）を打鍵した場合、OS側はそのVKが押されっぱなしだと認識し続ける。** 未検証の疑いではなく、コード上確定した欠陥として扱う。
3. **`OutputHistory.entries`（`output_history.rs:24-26`）が上限のない `Vec` で、`remove_by_scan` が実運用で呼ばれないため単調増加し続ける。** メモリリークであると同時に、修正時の設計上の罠でもある——`remove_by_scan`（KeyUp整合性用）と `recent_kana()`（n-gram文脈用）が同じ `Vec` を参照しているため、Phase 0を素朴に直すと「KeyUpのたびにn-gram文脈から確定済みのかなが消える」という**全打鍵でn-gramタイブレークの入力が変わる**副作用を引き起こす（決定0で先に分離する理由）。

### 未確定だが対処するリスク（設計段階で発見）

- `handle_key_up_pending`（PendingChar/PendingThumb中に該当キー自身が離された場合の即時単独確定）と `SpeculativeChar` 状態のKeyUp確定分岐も同様に到達不能。出力内容自体は現在のタイムアウト経由と同じだが、修正後は「タイムアウト待ちなしで即時確定」に体感レイテンシが変わる。
- `engine_off_extra_key_suppressed.take()`（`nicola_fsm.rs:1590`付近）は現状到達不能でラッチ固着し、2026-08-26に `toggle_enabled` リセットで回避済み（既存メモリ: `project_engine_off_extra_key_latch_fix_2026_08_26`）。本修正で正規経路が復活する。
- `KeyLifecycle::flush_pending_key_ups()`（`key_lifecycle.rs:69-78`、コンテキスト変更時にconsume済み未解放キーのKeyUpを再合成しOSへ再注入する別の安全弁）はFSMを経由しない別経路であり、本修正でも壊してはならない。

## 決定

### 決定0: `OutputHistory` を「解放索引」と「確定ログ」に分割する（挙動不変の純粋リファクタ、Step 0で単独land）

`OutputHistory.entries: Vec<OutputEntry>` を以下2つに分割する:

- `pending_releases: Vec<(ScanCode, KeyAction)>` — KeyUp整合性専用。`push`で追加、`remove_by_scan`で除去。
- `committed: VecDeque<OutputEntry>` — 確定出力ログ。`recent_kana()`（n-gram文脈）と `retract_last()`/`retract_bs_count()`（Speculative）専用。**`remove_by_scan` はこちらには一切触らない。** `NGRAM_CONTEXT_SIZE` + retract余裕分で上限を切り、無制限growthを解消する。

`remove_by_scan` は現状到達不能なので、この分割は**今日の挙動を1bitも変えない**純粋リファクタとして単独でlandできる。分割後は決定2でKeyUpがFSMに届き始めても、`recent_kana()` の入力（n-gramタイブレークの判断材料）は一切痩せない。

**レビュー観点**（CON指摘）: `append_key_up_for`（`nicola_fsm.rs:774`）が `pending_releases` だけを触り `committed` を消さないこと、`update_history` が両方にpushすること、`swap_layout` が両方をクリアすることを確認する。`recent_kana()` がKeyUp後も痩せないことを単体テストで固定する（Step 0の必須ゲート）。

### 決定1: `min_overlap_margin_percent` の既定値を一時的に0へ（挙動不変、Step 1で単独land）

`overlap_only_verdict`（`timing.rs:254-267`）は `overlap_us >= threshold_us * min_overlap_margin_percent / 100` で同時打鍵確定を判定する。**「実質常に同時打鍵成立」に相当する保守的な値は15ではなく0**（`overlap_us` は常に非負なので `min_overlap_us=0` なら常に成立）。現在の既定値は15（`timing.rs:18`、`feat/confirm-mode-simplify` ブランチでは `GeneralConfig::min_overlap_margin_percent` としてユーザー設定可能・既定値15）。

`min_overlap_margin_percent` を先に0へ落として単独landする。決定2でKeyUpがFSMに届くようになっても、この値が0である限り重なり不足判定は実質的に無効のままであり、「経路修正」と「判定の有効化」が分離される。**この「挙動不変」は、現状 `char1_released_at` が常に `None` であることに構造的に依存している——決定2適用後の世界でこの前提は崩れるため、決定3（15へ戻す）は必ず決定2の後、独立した実機ソーク・実測を経てから行う。**

**`feat/confirm-mode-simplify` ブランチとの調整が必要**: 同ブランチは本ADRと独立に `min_overlap_margin_percent`/`timing_margin_percent` を `GeneralConfig` の設定値として導入済み（既定値15/30）。マージ順によっては、本ADRの決定1（既定値0）と衝突する。先にマージされた側が既定値を確定させ、後からマージする側はrebase時に既定値を揃えること。

### 決定2: Phase 0を「Consume義務の予約」と「単一出口での`force_consume`格上げ」に再設計する

Phase 0が同時に担っている2つの責務（「対応KeyDownがConsume済みか」の判定・除去と、「イベントをどう処理するか」）を分離する。判定と `active_keys` からの除去は関数先頭で行うが、そこでreturnせず義務フラグとして保持し、イベント自体は通常経路（Phase 1〜3）へ流す。関数の**唯一の出口**で、義務があれば必ずConsumeへ格上げする。

```rust
pub fn on_input(&mut self, event: RawKeyEvent, ctx: &InputContext) -> Decision {
    let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
    let up_duty = if is_key_down { UpDuty::None } else { self.lifecycle.take_key_up_duty(event.vk_code) };

    let mut decision = self.on_input_body(event, ctx, is_key_down, up_duty);
    if up_duty == UpDuty::Consume {
        decision.force_consume(); // PassThrough→Consumed / PassThroughWith(fx)→ConsumedWith(fx)。Effectsは絶対に落とさない
    }
    decision
}
```

`UpDuty` は `None`/`Consume` の**二値**とする（設計初期案では特殊キー用に三値化を検討したが、根拠——「Phase 1の特殊キーは`output_history`に登録されない」——が誤りだったため撤回。無変換/変換はsolo-tap経路で`Key(vk)`として登録されうる。scan一致で拾えるなら正しい対発行であり、避けるべきものではない）。

**Phase 2（`compute_active(ctx)==false`）の早期pass_through対応**: `force_consume` はDecisionを直すだけで、FSM内部状態（`PendingCharThumb`・`pending_releases`）は取り残る。非活性時のKeyUp専用に、chord判定を一切走らせない狭い入口 `release_only` を設ける——`pending_releases.remove_by_scan` → `Key(vk)` なら `KeyUp(vk)` を対発行する、それだけ。`flush(ContextChange::ImeOff)` の思想（コンテキストを失ったら同時打鍵判定を再開しない）と一貫させる。

```rust
if !self.compute_active(ctx) {
    if up_duty == UpDuty::Consume {
        let mut d = self.adapter.release_only(&event);
        d.force_consume();
        return d.prepend_effects(transition_effects);
    }
    // 既存のpass_through経路
}
```

この設計により、「FSMがConsumeしたKeyDownに対応するKeyUpは、OSへは絶対に漏らさない」という元々の不変条件を、関数の唯一の出口で機械的に保証しつつ、KeyUpが実際にFSMへ届くようになる。

### 決定3（本ADRのスコープ外・将来の別ADR）: 実測付きで`min_overlap_margin_percent`を実用値へ戻す

決定2適用後、develop上で数日の実機ソーク（「文字が消える」「余計なKeyUpが出る」の観測）を行い、実測付きで既定値を引き締める。`tuning-constants.md` の実測義務に従う。

## テスト方針（`fix-requires-evidence.md` 対応）

- **差分golden**: 既存 `golden_scenarios` の全シナリオを `Engine::on_input` 経由でも回すハーネスを追加し、bare `NicolaFsm` 直呼び版との出力列一致をassertする。今回の欠陥の本質は「bare FSMでは通るがEngine経由では通らない」だったので、この二重回しがそのまま再発防止になる。
- 以後、bare `NicolaFsm` 直呼び（`.on_event()`/`.on_timeout()` をctx無しで呼ぶ形）の新規テスト追加を `architecture_guard` のソーススキャンで検出・警告する。既存テストは残してよいが、新規のKeyUp配線検証は必ず `Engine::on_input` 経由で書く。
- **不変条件テスト**: 「Consume済みKeyDownのvkに対するKeyUpのDecisionは必ず `is_consumed()`」を、`compute_active` が途中でfalseになるケース（IME OFF・フォーカス変更を挟む）を含めて検証する。
- 決定0: `recent_kana()` がKeyUp後も痩せないことの単体テストを必須化。
- 決定2: 「正規のKeyUpで `engine_off_extra_key_suppressed` ラッチがdrainされること」をEngine経由テストで固定する（2026-08-26の `toggle_enabled` リセットは冪等で安価なため二重の安全弁として残すが、それが唯一のdrain手段でないことを明記する）。

## 段階投入（4コミット）

0. `OutputHistory` を `pending_releases`/`committed` に分割（挙動不変・上限導入）
1. `min_overlap_margin_percent` の既定値 → 0（挙動不変）
2. Phase 0分解 + `UpDuty`二値 + `force_consume` + `release_only` + テスト一式
3. （別ADR/別コミット）実機ソーク後、実測付きでmarginを実用値へ

kill switchは置かない——各コミットは独立してrevert可能であり、env varより`git revert`の方が確実。

## Premortemの経緯

設計は2ラウンドのadversarial premortem（Opus 2体、PRO/CON）を経て収束した。

**ラウンド1（PRO提案）**: `UpDuty`三値化（`None`/`ConsumeWithoutFsm`/`ConsumeAfterFsm`）によるブラスト半径限定を提案。`min_overlap_margin_percent` は「現行の実効挙動と等価な保守的値」でlandingするとしたが具体値は示さず、stuck keyは実機検証タスクとして先送りした。

**CON批判（ラウンド1）**: 6点の反証。(1) stuck keyは`.yab` grepではなく`resolve_pending_char_as_single`等のコード経路から確定できる欠陥であり未検証扱いは誤り。(2) `OutputHistory` が上限なし`Vec`であり、`remove_by_scan`とn-gram文脈`recent_kana()`が同じVecを共有しているため、Phase 0修正だけで段階(1)が「挙動不変」にならない。(3) `MIN_OVERLAP_MARGIN_PERCENT`の現行値は15であり、「実質常に成立」の正しい値は0（PROの理解が逆だった）。(4) Phase 2の`compute_active`=false早期returnで`force_consume`だけではFSM内部状態が取り残る。(5) `ConsumeWithoutFsm`の存在根拠（Phase 1キーはoutput_historyに無い）が誤り。(6) `engine_off_extra_key_suppressed`ラッチの正規経路復活時の扱いが未検討。

**PRO再反論（ラウンド2・最終）**: 1〜6全てを認め、決定0（OutputHistory分割の先行land）・決定1（margin既定値を0に訂正）・決定2の`release_only`追加・`UpDuty`二値化への撤回、を含む現在の設計へ改訂。「Consume義務の予約とFSM到達の分離、単一出口での`force_consume`格上げ」という核だけは、CON指摘4が「不要」ではなく「必要だが不十分」という指摘だったとの理解のもと維持した。

**CON最終確認**: ラウンド2案に合意。決定0をPhase 0修正の前に置いた順序を特に評価し、「(1)が無害でない」という最大の懸念が構造的に解消されたことを確認。追加条件として、決定0/1の「挙動不変」が`char1_released_at`常時`None`という現状の欠陥に依存していることをADR本文に明記するよう要求（本文中に反映済み）。
