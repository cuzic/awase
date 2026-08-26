# ADR-103: Warmup/Probe 過渡期の pending 取りこぼしと FSM 整合性

## ステータス

**提案（未実装、2026-08-26）。** 直近のコードレビューで確証された指摘のうち、「probe/warmup の過渡期に必ず通るべき出口が無い」「一発限りフラグの所有スコープが未定義」という2系統をまとめる。Opus 2体によるドラフト→敵対的レビューを4ラウンド実施し収束させた。関連: [ADR-102](102-startup-key-delivery-one-way-closure.md)（本 ADR の決定3は ADR-102 決定1-a の `deliver_key_event` に依存する。ADR-102は[ADR-105](105-engine-thread-notification-via-hwnd.md)を前提に全面改訂されており、`deliver_key_event`の決定番号が旧1-eから1-aへ変わっている点に注意）、[ADR-104](104-observation-freshness-and-hardening.md)。

## コンテキスト

対象の指摘:

- **`post_bypass_passthrough` フラグの残留**: Ctrl+vk バイパス直後に NonText ウィンドウへフォーカスが移ると唯一の消費点に到達できず残留し、無関係な将来のキー入力に誤適用される。
- **probe_io.rs の早期 return が deferred VK フラッシュを飛ばす**: TSF 向け `dispatch_probe_actions` の3箇所の早期 return が `flush_deferred_and_mark_warmup` を呼ばず、貯まっていた VK が滞留し `GjiFsm` が `WarmupComplete` を永久に受け取らない（BUG-27 の未解決 follow-up）。
- **gji_fsm.rs の pending 消失と params 捏造**: `ImeOff`/`FocusChange` ハンドラの pending 件数計算が `OnComposing(AwaitingProbe)` を見落とし警告なく消える。`EndComposition` が `ColdKind`/`ProbeParams` を固定値で再構築する（[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) が禁じる「弱い代理指標だけでの無条件書き換え」）。

これらは同じ失敗形に収束する: **過渡期に「必ず通る出口」が無い**。probe/warmup の途中で早期 return すると、deferred VK のフラッシュや FSM への完了/中断通知といった「段の終わりに必ずやること」が飛ぶ。BUG-27/BUG-38 は既に同じ形で2回起きており、`flush_deferred_and_mark_warmup`（`output/probe_io.rs:488`）の doc コメントがその経緯を記録している——**共通関数に括るだけでは呼び忘れを防げず、呼び忘れられる出口自体を消す必要がある**。

一発限りフラグ（post-bypass latch）も同じ形の亜種で、**判定材料（フォーカス情報）を stale なまま使っている**ことが根本原因である。

### 制約

- フックコールバック上で新たな panic 経路・ブロッキング呼び出しを持ち込まない。
- [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の3層分離を破らない。実際に注入していない副作用を「完了」として通知しない（`WarmupComplete` は実際に送信したときだけ）。
- タイミング定数は変更しない。

## 不変条件

- **INV-B（運搬）**: 一度確定した値（probe の `ProbeParams`、cold の種別）は、後段で再導出せず運ぶ。弱い代理指標からの再計算は belief 汚染の常習経路である。
- **INV-D（副作用の通知は事実に一致する）**: FSM へ送る完了通知は、実際に起きた副作用とだけ対応させる。「送っていないが段は終わった」を「warmup 完了」として通知しない。

---

## 決定3: 一発限りフラグに所有者スコープを与え、判定材料を stale にせず、消費と passthrough を区別する

`platform_state.gate.post_bypass_passthrough: bool` は `handle_wm_key_from_hook` の NonText 早期 return より後でしか消費されない。Ctrl+J 直後にタスクバー等へフォーカスが移ると消費点に到達せず無期限に残る。

```rust
#[derive(Clone, Copy)]
pub struct PostBypassLatch { pub armed_pid: u32, pub armed_focus_epoch: u64 }

pub enum PostBypassAction {
    NotArmed,                 // latch 無し
    PassthroughKeepArmed,     // このキーは passthrough するが latch は維持する
    ConsumeAndPassthrough,    // このキーを passthrough し latch を落とす
    DisarmOnly,               // latch を落とすだけ（このキーは通常処理へ）
    Proceed,                  // latch は維持、このキーは通常処理へ
}

pub const fn post_bypass_action(
    latch: Option<PostBypassLatch>,
    now_pid: u32,             // 評価直前に取り直す
    is_key_down: bool,
    ctrl_held: bool,
    is_modifier_like: bool,
) -> PostBypassAction;
```

**KeyUp が孤児化しないための5値化**: 現行はフラグを false にするのが KeyDown のときだけで、KeyUp は区別なく passthrough される。tmux の Ctrl+J で Ctrl を先に離すと、その `J↑` がエンジンへ入って孤児 KeyUp になるか、latch が KeyUp で消費され prefix が毎回無駄になるかのどちらかになりうる。現行の意味論（KeyDown で消費・KeyUp は消費せず passthrough）は `is_key_down == true → ConsumeAndPassthrough`、`false → PassthroughKeepArmed` として**全数テストで固定する**。

**スコープは `focus_epoch` ではなく `armed_pid`（プロセス同一性）にする**。`focus_epoch` は一瞬のフォーカス奪取（通知ポップアップ等、BUG-57 の実例）でも進むため、Windows Terminal で Ctrl+J 直後に通知が出て消えると次の `n` が prefix として扱われず tmux が壊れる——正しく動いているケースの退行になる。プロセス同一性なら通知の往復（別 pid → 元の pid）で latch は生き残る。`armed_focus_epoch` はログ・診断用に持つが判定には使わない。

**`now_pid` の stale 問題**: `handle_wm_key_from_hook` 冒頭では `platform.focus` の pid はまだ前のキーの値であり、そのまま使うと別アプリでの最初の1キーが誤って `armed_pid == now_pid` と判定される。latch が armed のときに限り、評価の直前に `GetForegroundWindow` → `GetWindowThreadProcessId` で pid を取り直す。毎打鍵で Win32 が増えることはない（`None` の早期 return がそれ以外の全打鍵を素通しする）。

評価位置は NonText 早期 return の前へ移し、[ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-a の `deliver_key_event` の中に置く。これで hook 経由と `INPUT_DEFER` drain 経由の両方が同じ判定を通る。許容する誤りの向き: `[6]` の実害（latch 残留による無関係な別アプリへの誤適用）を消すことを優先し、代償（同一プロセス内の NonText 子窓で latch が1回無駄に消費される、Ctrl+J を押し直せば回復）を許容する。

**新しい時間定数（TTL）を導入しない**: TTL は「ユーザーが prefix の次を押すまでの思考時間」という人間側の変数を ms で当てる必要があり、[tuning-constants](../../.claude/rules/tuning-constants.md) が要求する実測の対象にできない。プロセス同一性というアプリ側で観測可能な事実だけでスコープを閉じる。

**証拠義務**: focus 遷移ファミリー。`post_bypass_action` の全数テスト（`armed_pid` 一致/不一致、`is_key_down`、`ctrl_held`/`is_modifier_like` の全組み合わせ、通知往復系列）と、現行の KeyDown 消費・KeyUp 非消費を固定する characterization テスト。`docs/known-bugs.md` に暫定 **BUG-80** を起票し、スコープに `focus_epoch` ではなく `armed_pid` を選んだ理由を残す。

---

## 決定4: probe の送信段に「必ず通る出口」を1つ作る。出口は2つの FSM の両方を畳む

`output/probe_io.rs` の `dispatch_probe_actions` は、`TransmitTarget::Tsf` アームで `gate_is_bypass()` と `chars.is_empty()`、`TransmitSingleVk` の Tsf アームの計3箇所が早期 return で抜け、`flush_deferred_and_mark_warmup` を呼ばない。gate が Bypass に切り替わった瞬間に貯まっていた deferred VK が順序が狂って遅延し、`GjiFsm` は完了通知を永久に受け取らず `OnCold` に固着する。

**「早期 return の直前に1行足す」は採らない**。同じ2行を3箇所に散らす形は、同じ役割の対症療法が複数箇所に散らばる設計そのものである。`flush_deferred_and_mark_warmup` は既に BUG-27/BUG-38 の再発を受けて共通化された関数であり、それでもなお呼び出しを忘れた出口が3つ残った。**共通関数化は呼び忘れを防げないと実証済みなので、出口自体を1つにする**。

**中断が起きうる場所自体を潰す**: `TransmitSingleVk` の per-VK 列の gate 判定を `idx == 0` へ引き上げる（列に入る前に一度だけ評価。ここで Bypass なら1文字も注入していないので batch アームと同じ「注入前スキップ」として扱える）。列の途中で gate が Bypass へ落ちた場合は、列を捨てずに**輸送手段を落として送り切る**（残りの VK を非 TSF 経路 `VkMarker::InjectedWithScan` で送る）。`gate=Bypass` は「この窓では TSF composition context が使えないと確定した」という意味であり、非 TSF 経路が正しい輸送手段になる状況である（降格の実機妥当性は演繹であり未検証、ソーク項目に明記）。

```rust
enum TransmitAttempt {
    Sent { degraded: bool, ze_bs_count: usize, detector: Option<LiteralDetector> },
    Skipped(TransmitSkip),   // GateBypass | NoResolvableVk
}

impl TransmitSkip {
    const fn sink_marker(self) -> VkMarker {
        Self::GateBypass => VkMarker::InjectedWithScan,   // TSF注入を見送った以上、同じ関数内でTSF markerは自己矛盾
        Self::NoResolvableVk => VkMarker::Tsf,             // TSF経路自体は生きている
    }
}

/// probe 段の唯一の出口。2つの FSM（probe FSM と GjiFsm）の両方をここで畳む。
fn finish_probe_stage<M: TickableFsm + ?Sized>(
    machine: &mut M,
    io: &impl ProbeIo,
    attempt: &TransmitAttempt,
) -> DispatchResult;
```

`finish_probe_stage` の中身:

1. **deferred VK の解放は常に行う**（`Sent`/`Skipped` を問わない）。deferred VK は既にフックが消費したユーザーの打鍵であり、保持し続けることが BUG-27 の失敗形そのものである。
2. **probe FSM を終端へ落とす**。`TickableFsm` に `fn apply_transmit_skipped(&mut self, reason: TransmitSkip)` をデフォルト実装なしで追加し、全実装に対応を強制する。既存のタイムアウト畳み込みと同じ終端へ合流させ、以後 tick されても新しい `Transmit` を emit しないことを不変条件にする。
3. **`GjiFsm` への通知は事実に一致させる**（INV-D）。`Sent` → 現行どおり `WarmupComplete`。`Skipped(reason)` → **`WarmupComplete` を出さない**。新設する `GjiEvent::WarmupAborted { probe_id, reason }` を出す。

**`GjiEvent::WarmupAborted` の意味論**: 「この probe は1文字も注入せずに終わった。ただし段の終わりに溜まっていたキューは解放済みで、probe FSM も終端に落ちている」。

- `pending` は `WarmupComplete` 時と同じ形で解放し空にする（`finish_probe_stage` が deferred VK を解放した以上、shadow 側も同じタイミングで空にしないと bookkeeping が実体からずれる）。
- 遷移先: `OnCold` で受けた場合 → `OnCold { kind, probe: NotStarted, pending: [] }`（**`OnWarm` には絶対に遷移しない**）。`OnComposing { warmup: AwaitingProbe }` で受けた場合 → `OnComposing` に留まる。`ComposingWarmup` に `AbortedCold { kind, params }` を追加し、後続の `EndComposition` がこの `kind`/`params` を使って `OnCold` を再構築する（決定5-b と同じ材料）。`AbortedCold` は `AlreadyWarm` と**別の variant**にする（丸めると warm 扱いになり、決定4 が塞いだリテラル漏れが別経路で開く）。

橋渡し（`output/tsf_warmup_coord.rs:37` の bool フラグ）は `Option<ProbeStageOutcome>`（`Warmed { degraded }` / `Aborted(TransmitSkip)`）に置き換える（bool のままだと「中断」も「降格」も運べない）。

**証拠義務**: warmup/cold-start ファミリー、かつ BUG-27 の未解決 follow-up。`ProbeIo` モックで「`gate_is_bypass()==true` でも `take_pending_deferred_vks` が呼ばれる」「そのとき `store_gji_warmup_result` が呼ばれない（`WarmupComplete` が出ない、本決定の核心）」「`Skipped` の2系列で `apply_transmit_skipped` が呼ばれ以後 `Transmit` を再 emit しない」「per-VK 列の途中で gate が反転しても列が捨てられず送り切られる」を固定する。`gji_fsm`（Linux 実行可）に `WarmupAborted` 受信後の遷移テストを追加する。`docs/known-bugs.md` の BUG-27 エントリに follow-up 完了を追記する。

---

## 決定5: FSM の pending は単一アクセサから読み、params は運ぶ

**5-a. `pending_len()` を `running_probe_id()` と同じ SSOT に置き、pending の破棄を明示的な行為にする。**
`tsf/gji_fsm.rs:558`（ImeOff）と `:585`（FocusChange）の pending 件数計算は `GjiState::OnCold { pending, .. }` しか見ないが、`OnComposing { warmup: AwaitingProbe { pending, .. } }` も pending を持つ。同じ関数内の `running_probe_id()` は両方を正しく見ている——片方だけが古い形のまま取り残されている。

```rust
fn probe_and_pending(&self) -> (Option<ProbeId>, usize);   // running_probe_id はこれの .0 を返す薄いラッパへ
```

アクセサ統一だけでは足りない（両アームは直後に `self.state` を上書きするため bookkeeping が失われ、破棄が起きても警告すら出ない）。`GjiAction::DiscardPending { count, reason }` を新設し、pending を捨てるときは必ずこのアクションを emit してから状態を上書きする。破棄そのものは維持する（`ImeOff` はエンジン停止、`FocusChange` は宛先ウィンドウが変わっており、そのまま送ると別ウィンドウへの誤送信になる——[ADR-101](101-bug74-giveup-retry-with-focus-guard.md) が focus 世代照合で塞いだのと同じ事故）。`INPUT_DEFER` へ戻す案は採らない（`PendingInput` は romaji + deferred VK であって `RawKeyEvent` ではなく、順序保証の合流点に入れられる型ではない）。

**5-b. `EndComposition` は `ColdKind`/`ProbeParams` を再導出せず運ぶ。**
`tsf/gji_fsm.rs:774` は `AwaitingProbe` から `OnCold` へ戻す際に `ColdKind::Short` と `ProbeParams { forces_prepend_f2: false, is_long_cold: false }` を**固定値で捏造**している。元の probe が Medium/Long 想定（`forces_prepend_f2: true`）で認可されていた場合、composition 終了だけを理由に params が黙って false へ書き換わる。

`ComposingWarmup::AwaitingProbe { probe_id, pending }` に `kind: ColdKind, params: ProbeParams` を追加し、`StartComposition` で `OnCold` から遷移するときにそのまま持ち込む。`EndComposition` は持ち込んだ値で `OnCold` を再構築する。決定4 の `ComposingWarmup::AbortedCold { kind, params }` も同じ材料を使う。

**なぜ `gji_idle_ms` を足して `ColdKind::classify()` を呼ばないか**: (i) composition 直後は GJI I/O が活発なので idle は必ず小さく `classify` は事実上 `Short` を返す——今の固定値を「観測してから決めた」ように見せかけるだけ。(ii) [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) が BUG-33 の教訓として禁じる「弱い代理指標だけで無条件に書き換える」形そのもの。(iii) 認可済み probe の params は確定値としてすでに存在する（INV-B）。

**証拠義務**: warmup/IME belief ファミリー。`gji_fsm` の状態遷移テスト（Linux 実行可）に「`AwaitingProbe(kind=Medium, params.forces_prepend_f2=true)` → `EndComposition` → `OnCold` の kind/params が保存される」「`OnComposing(pending>0)` での `ImeOff`/`FocusChange` が `DiscardPending{count>0}` を emit する」を追加。`docs/known-bugs.md` に暫定 **BUG-82** を起票する。

---

## 実装順序

決定4＋決定5 は同一 PR（決定4 の `AbortedCold` が決定5-b の `kind`/`params` 保持に依存するため分割しない。`apply_transmit_skipped` は `TickableFsm` の破壊的変更なので同 PR に閉じる）。決定3 は [ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-a の `deliver_key_event` が入った後でないと置き場所が無い。

## 却下した代替案

- **19件を1件ずつ直す**: `[7]` は「共通関数に括ったのに呼び忘れた出口が3つ残った」という既に一度失敗した対症療法の再演になる。
- **per-VK 列の中断をそのまま許容し「部分注入済み」フラグを持たせて別途回収する**: 状態が2種に増え、回収は事実上リテラル回収の再実装になる（BUG-27追補2 で実機破綻を確認済み）。そもそも中断しないほうが状態数が減る。
- **`TickableFsm` に出口を追加せず probe FSM 側は自己終端に任せる**: 早期 return 3箇所のうち `chars.is_empty()` で降りた場合、probe FSM は「Transmit を emit したのに完了通知を受け取っていない」まま tick され続け、次の打鍵で新しい probe と二重に走る。

## 未解決の疑問（実機ソークで確認すること）

- 決定4「per-VK 列の途中で gate が Bypass に落ちたら降格して送り切る」は演繹であり実測ではない。降格して送り切ったモーラが正しく出るか、`degraded=true` のログ件数と併せて確認する。
- 決定5-a の `DiscardPending` は当面「明示化とカウント」までで、破棄自体は維持する。実機で発生件数が有意なら、`FocusChange` 時に限って romaji を再送すべきかを別 ADR で検討する。

## 設計の経緯

Opus 2体でドラフト→敵対的レビューを4ラウンド実施した。主な転換点: (1) 初版の「`TransmitAttempt` の値に関わらず `flush_deferred_and_mark_warmup` を通す」は `WarmupComplete` → `OnWarm` を意味し BUG-02 型のリテラル漏れを新規に開くと判明し破棄、`WarmupAborted` を新設。(2) per-VK 列の途中中断への対処が「出口を1つにする」だけでは probe FSM 側に届いていないと判明し、`apply_transmit_skipped` を追加。(3) post-bypass latch のスコープを `focus_epoch` にする初期案が通知 churn で tmux prefix を壊すと判明し `armed_pid` へ変更。
