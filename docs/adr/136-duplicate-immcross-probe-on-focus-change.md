# ADR-136: フォーカス変更時の「二重IME probe」仮説はOpus敵対的レビューで反証・却下（副産物としてBUG-78非対称を発見）

## ステータス

**却下（2026-09-05）。** ユーザー依頼による「二重のactuation/probeがないか」の
全体調査で見つけた「`AppImeProfile::Standard`へのフォーカス変更時に
`read_ime_state_full_async()`が同一イベントに対し無駄に2回発行されている」
という当初仮説は、Opus敵対的レビュー（読み取り専用、`opus-review-adr136`）に
より実コード照合で**反証され、決定は不採用（変更なし）で確定した**。

反証の過程で、当初仮説とは無関係の実害候補（経路Bが`disable_apps`
（BUG-78）対象アプリでも抑止されずに走る非対称）を1件発見した。これは
本ADRの対象外として切り出し、`docs/known-bugs.md`等での別記録を検討する
（下記「副産物」節）。

## 当初仮説（反証済み、経緯として記録）

`crates/awase-windows/src/runtime/focus_tracking.rs::on_focus_process_changed`
は、`AppImeProfile::Standard`かつ`is_japanese_ime()==true`のとき、フォーカスが
移った子hwndの実IME状態を`read_ime_state_full_async()`
（`GetGUIThreadInfo`+IMM32連鎖、実測200-300ms）で非同期に読み直し
（以下「経路B」、`focus_tracking.rs:694-725`）、`write_imm_cross_probe`で
beliefへ書き込む。

一方、`on_focus_process_changed`を呼ぶ`spawn_ime_refresh()`
（`runtime/mod.rs:645-652`、`TIMER_IME_REFRESH`経由の通常フォーカス切替の
唯一の定常経路）自身が、`with_app`に入る前に同じ`read_ime_state_full_async()`を
1回呼んで結果（snap_A）を`run_ime_refresh_with_prefetched`経由で
`ir_stage_observe`（Phase 3、以下「経路A」）に渡している。

ここから「同一のフォーカス変更イベントに対し同じ重いAPIが2回発行されており、
経路Bは経路Aより後にしか完了しえないため無意味な重複である」という仮説を
立て、経路Bを条件付きスキップ（候補1）または完全削除（候補2）する決定案の
ドラフトを書いた。

## Opus敵対的レビューによる反証（実コード照合済み、致命的指摘）

`opus-review-adr136`（Opus、読み取り専用）に、ADR本文と引用元コードの
実行順序・confidence・epoch照合・層境界規約適合性を検証させた。以下が
当初仮説を覆す指摘（いずれも実コードのfile:lineで裏付け済み）。

### 1. 実行順序の理解自体は正しい（唯一、反証されなかった部分）

`message_handlers.rs:486` → `mod.rs:645-652`spawn → `run_focus_probe_async`→
`read_ime_state_full_async`(snap_A) → `with_app` → `ir_execute`
（`ime_refresh.rs:63-83`）→ `ir_stage_focus`(:68) → `apply_focus_probe_result`(:98)
→ `on_focus_process_changed`(`focus_tracking.rs:99`) → `FocusChanged`dispatch
（`:541-549`、observationsクリア）→ 経路Bspawn（`:687-725`）→ 同期的に戻って
`ir_stage_strategy`(:80) → `ir_stage_observe`(:81) → `ir_poll_and_learn(snap_A)`
(:211, :342-384)。経路Bが経路Aのsnap_A消費より先に完了できないという点も正しい。

### 2. confidenceレベルが異なり、対等な重複ではない（致命的）

- 経路B: `write_imm_cross_probe`（`platform_state.rs:1374-1386`）→
  `Observed<ImmCrossProbe>`（`evidence.rs:130`）= **High**
- 経路A: `apply_ime_update`（`platform_state.rs:1048,1057-1064`）→
  `Observed<ObserverPoll>`（`evidence.rs:132`）= **Medium**

`derive_filtered`（`observation_store.rs:783-840`）はHighを単独即採用する
（:783-799）が、Mediumは`MediumConsensus`経由でしか採用されず、矛盾する
fresh なMedium+観測が1つでもあると導出自体が`None`に落ちる（:832-838）。
経路Bは「経路Aと同じことを遅れてやる冗長」ではなく、`.claude/rules/
ime-belief-architecture.md`が規定する「Highは`write_imm_cross_probe`、
Mediumは`apply_ime_update`」という設計そのものであり、両者は最初から
異なる役割を持つ。

### 3. `SkipTyping`戦略により、Alt-Tab等の典型ケースでは経路Aが丸ごとno-opになる（最も致命的）

`ir_decide_read_strategy`（`ime_refresh.rs:294-338`）は`idle_ms <
TYPING_IDLE_MS`（=500ms、`tuning.rs:12`）かつexplicit intent無しなら
`ImeReadStrategy::SkipTyping`を返し、`ir_stage_observe`（:158-159）は
`SkipTyping => {}`で**snap_Aを一切消費しない**。フォーカス変更→リフレッシュの
遅延は`focus_debounce_ms=50`（`platform_state.rs:1485`）のため、**Alt-Tab
（本ADRが影響範囲の筆頭に挙げていた典型例そのもの）では、リフレッシュtick
時点のidleが約50ms → `SkipTyping` → 経路Aは何も書かず、経路Bだけが唯一の
観測源になる**。経路Aが次に効くのは次回ポーリング（既定500ms級）だが、
そこでは`process_changed=false`のため経路Bは走らない。

**この事実により、候補1（`Prefetched`のとき経路Bをスキップ）は、
「経路Bが唯一の観測源であるまさにその典型ケース」を狙い撃ちで潰す誤った
決定であり、候補2（経路Bの完全削除）も同様にAlt-Tab系フォーカス切替で
FocusChanged直後の観測を丸ごと失わせる。両候補とも不採用。**

### 4. epoch/fence照合も経路Bにしかない安全性（当初「同等」と誤認）

`is_identity_ok`（`observation_store.rs:757-780`）は`ImmCrossProbe |
FocusProbe`にのみepoch+hwnd照合を適用し、`ObserverPoll`は素通しする。
さらに経路Aが使う`AcceptedObservation::for_sync`のdoc
（`probe_admission.rs:190-196`）は「同期プローブ専用、spawn〜complete間に
フォーカスは変わらない」という前提を明記しているが、`Prefetched`分岐の
snap_Aはasync offload由来であり、この前提を既に踏み外している。つまり
「経路Aは経路Bと同等の安全性を持つ」という当初ADRの含意は逆で、**経路Aの
方が構造的に弱い**。

### 5. Sync経路でも二重発行は起こりうる（当初「該当しない」は不正確）

bootstrapは`establish_initial_focus_scope`が先に`update_focus_info`を
済ませるため`process_changed`が構造的に`false`になり、`on_focus_process_changed`
を通ることは絶対にない（当初ADRの結論自体は正しいが、理由が誤っていた）。
一方`WM_INPUTLANGCHANGE`/panic resetは、`EVENT_OBJECT_FOCUS`→50msデバウンスの
隙間にプロセス変更を先取り観測する形で理論上`process_changed=true`になり
うる。この場合`ime_snap=None`のため`ir_poll_and_learn`は同期的に
`poll_and_classify_ime`を実行しつつ経路Bも走るため、**Sync経路でも二重発行は
起こりうる**——当初ADRの「Sync経路は本問題の対象外」という前提は不正確
だった。

## 決定

**変更なし（却下）。** 経路A・経路Bとも現状のまま維持する。両者は同じ信号の
無駄な重複ではなく、confidenceレベルとepoch安全性が異なる、意図的に補完的な
2つの観測経路である。

## 副産物: `disable_apps`（BUG-78）対象アプリでも経路Bが抑止されない非対称

反証の過程で、本ADRの対象とは独立の実害候補が見つかった。`ir_execute`の
`app_disabled`早期return（`ime_refresh.rs:76-78`）は`ir_stage_focus`の
**後**にある。`disable_apps`（既定`mstsc.exe`、BUG-78の設計意図は
「observe/notify/drift/warmup/probeが全停止」）対象アプリへフォーカスが
切り替わっても、`ir_stage_focus`内で経路Bは既にspawnされてしまっており、
**経路Bだけは抑止されずbeliefを書き込む**。

本ADRの決定（変更なし）とは独立の別課題として切り出す。実装するかどうか・
`docs/known-bugs.md`への記録要否はユーザー判断待ち。

## 関連する副次的発見（本ADRのスコープ外、参考記録・当初仮説の反証とは無関係）

同じ調査で見つかった、独立に扱うべき2つの発見（反証の影響を受けない）:

1. **`output/probe_io.rs`の2つのポーリングループが構造的に重複したコード**:
   `send_chrome_gji_reinit_and_poll`（`probe_io.rs:167-305`）と
   `Output::start_ms_ime_ready_poll`（`probe_io.rs:400-`）は、`spawn_local`
   →10ms間隔で`get_ime_conversion_mode_raw_timeout_async`→`with_app`内で
   focus_gen照合→`update_ime_mode_from_imc`→終了、というほぼ同一の構造を
   別のenumで独立実装している。バグではなく単なるコード重複であり、共通
   pollヘルパーへ統合できる余地がある。着手するかは別途判断。
2. **`ime_mode_focus_gen`がIME種別切替（GJI⇔MS-IME）による無効化をカバー
   しない**: `output/mod.rs:817-821`でフォーカス変更時のみインクリメント
   され、`set_active_ime_kind`（`tsf_warmup_coord.rs:100`）は世代を進めない。
   実害は未確認・未起票。実機での再現待ち。

## 教訓（このADR自体から得られたもの）

「同じAPIが2回呼ばれている」という表面的なコード読解だけでは、その2回が
本当に冗長かどうか判定できない。confidenceレベル・戦略分岐（`SkipTyping`
等）による片方の実質的no-op化・epoch/fence照合の非対称性まで実行時の
条件分岐を辿って初めて「重複」か「補完」かが分かる。本件は
`.claude/rules/ime-belief-architecture.md`が定める「High/Medium confidence
の使い分け」という**意図的設計**を、コード上の見た目の類似性だけで
「重複」と誤診断した例であり、次に同種の「二重probe/actuation」を疑う際は、
書き込み先のconfidence/evidence型が同一かどうかを最初に確認すべきである。

## 関連

`.claude/rules/ime-belief-architecture.md`（Observe→純粋classify→reduce規律、
High/Medium confidence使い分けの規定元）、
[ADR-075](075-imm-cross-probe-belief.md)（ImmCrossProbeによるbelief補正の由来）、
[ADR-077](077-observation-admission-epoch.md)（ObservationAdmission Layer、
epoch照合——経路Bの`admit_epoch_in_app`の由来）、`docs/known-bugs.md`
BUG-78（`disable_apps`の設計意図——副産物で見つかった非対称の参照元）。
