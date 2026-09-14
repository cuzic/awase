# hook.rs 診断リング Mutex の安全性再検証（相談記録）

> **これは正式な ADR ではない。** `docs/design/` 配下に置いた一時的な相談記録であり、
> ADR-164 フェーズ4「訂正3」の決定を変更するものではない。結論を採用する場合は
> ADR-164 への追記、または新規 ADR 起票を別途行うこと。
>
> 対象: `crates/awase-windows/src/hook.rs::HookState::ime_mode_diagnostics`
> （`Mutex<VecDeque<HookImeModeDiagnosticRecord>>`、hook.rs:57）
> 日付: 2026-09-13 / 読んだ版: develop `3e802984`

---

## 結論（先に述べる）

**「優先度を下げて別の対策を先にやる」。** 内訳:

1. 論点1の批判（O(1) は壁時計時間の保証ではない）は**理屈としては正しい**が、
   この Mutex は issue #165 / #137 で観測されている「Hook watchdog: no activity」の
   原因**ではありえない**。下記「§1 反証」で、コード上の2点から排除できる。
   ADR-164 の決定を蒸し返す理由にはならない。
2. ただし調査中に、この Mutex 本体ではなく**その周辺**に、実害の道筋が具体的に
   描ける欠陥を2件見つけた。どちらも Mutex を消すより安く、効果も大きい:
   - **P2**: `WM_HOOK_IME_MODE_DIAGNOSTIC` が `with_app_or_repost` を使っており、
     トレイメニュー等のネストモーダルポンプ中に repost ループでスピンしうる（§3）。
   - **P3**: ロック保持区間の中にアロケーションが1個だけ残っている（§4）。
     ADR-164 の不変条件を「文字どおり真」にするのに4行で済む。
3. 論点4（詰まり検知時点の OS 実 IME 状態の受動記録）が**最優先（P1）**。
   P2/P3 とは独立に実施可能で、順序依存はない（§6）。
4. 論点3の「専用 lossy SPSC リング新設」「HookKeyRing 汎用化」「診断ごと削除」は
   **いずれも不採用**（§5）。特に「削除」は、既定ログレベルでは代替経路が無いため
   今読んでいる不具合報告そのものを盲目にする。

優先度: **P1（§6）> P2（§3）> P3（§4）>>> 専用リング新設（やらない）**

---

## §1 反証: この Mutex は観測中の症状の原因ではない

### 反証A: 警告を書いているのがメインスレッド自身である（決定的）

「Hook watchdog: no activity for {stale_ms}ms」は
`crates/awase-windows/src/runtime/message_handlers.rs:648-679` の `WM_TIMER` ハンドラが出す。
このタイマーは `crates/awase-windows/src/runtime/mod.rs:1888-1892`
（`start_hook_watchdog`）で **3秒周期**に設定されており、**メインスレッドの
メッセージポンプ上で**動く。

一方、この Mutex を取るメインスレッド側の唯一の箇所は
`hook.rs:975-981 drain_hook_ime_mode_diagnostics`（呼び出し元は
`runtime/message_handlers.rs:100`）。

したがって、この Mutex が hook スレッドを詰まらせるためには、メインスレッドが
`drain_hook_ime_mode_diagnostics` の内側でロックを保持したまま 5000ms 以上
止まっている必要がある。**しかしその間、メインスレッドは 3秒周期の `WM_TIMER` を
ディスパッチして当の警告を書き出すことができない。** 報告では「全チェックの約半数」で
この警告が出ている＝メインスレッドは 3秒おきに生きてポンプを回していたことが
ログ自身によって証明されている。両立しない。

### 反証B: tick の打刻位置がロック取得より手前にある

`tick_hook_alive()` は `hook_callback` の**最初の文**（`hook.rs:995`）で、
ロック取得（`hook.rs:1038` の `push_hook_ime_mode_diagnostic`）より手前にある。
したがって hook スレッドがコールバック N の中でロック待ちに入っても、
`hook_alive_tick_ms` はコールバック N の入口で既に更新済み。
`stale_ms > 5000` に到達するには**ロック保持が 5000ms 継続**する必要があり、
これは反証Aと同じ要求に帰着する。保持区間は 64要素×24バイト
（`HookImeModeDiagnosticRecord` は `u16`+`bool`×3+`u32`+`Option<u64>`、
`journal.rs:103-110`）＝約1.5KB のムーブとアロケーション1回でしかない。

### 反証Aの限界（誠実に書いておく）

反証Aが排除するのは「**観測中の慢性的な no-activity の原因**」であって、
「この Mutex で hook スレッドが数十ms止まる瞬間が一度も無い」ことではない。
論点1が指摘するプリエンプション（ページフォルト・EDR スキャン・Efficiency Mode）は
確かに起こりうる。ただしその場合の実害は「キー入力の数十ms遅延」であって、
`LowLevelHooksTimeout`（既定5000ms、`hook.rs:741-770 low_level_hooks_timeout_ms()` が
実値を読む）到達によるフック剥離ではない。桁が3つ違う。

---

## §2 ADR-164 訂正3 の記述のうち、実コードと食い違っている点

ADR-164 の訂正3（`docs/adr/164-global-static-argument-threading-plan.md:298-311`）の
安全性根拠そのものは正しい。ただし本文の「アロケーションは発生するが上限64件の
有界サイズ」という書き方は、**そのアロケーションがロック保持区間の内側にある**という
事実を「有界だから良い」で流している。ここだけが論点1の批判が刺さる箇所であり、
§4 で潰せる。

---

## §3 P2: 診断メッセージが `with_app_or_repost` を使っている（実害の道筋あり）

`crates/awase-windows/src/app/mod.rs:408-412`:

```rust
WM_HOOK_IME_MODE_DIAGNOSTIC => {
    with_app_or_repost(WM_HOOK_IME_MODE_DIAGNOSTIC, |app| {
        message_handlers::handle_wm_hook_ime_mode_diagnostic(app);
    });
}
```

`with_app_or_repost`（`crates/awase-windows/src/lib.rs:252-256`）は、`RUNTIME` が
既に借用中（再入）なら**同じメッセージを自スレッドのキューに post し直す**。

問題の道筋:

1. `hook.rs:1038-1046` の push + `post_to_main_thread_quiet` は、
   `hook.rs:1050` の `if self_injected { return ... }` より**手前**にある。
   つまり awase 自身が actuation で送った `VK_IME_ON`/`VK_IME_OFF`/`F2` 等の
   self-injected な IME モードキーでも、down/up の**2回**リングに積まれ、
   **2回** `WM_HOOK_IME_MODE_DIAGNOSTIC` が post される。warmup バーストでは
   これが1ターンに何発も積み上がる。
2. トレイメニュー（`tray.rs` の `TrackPopupMenu` ネストモーダルループ）や
   `runtime/engine_window.rs::MODAL_DEPTH` 経路のように、**`RUNTIME` を借用したまま
   ネストしたメッセージポンプに入る**区間がある。この間に届いた
   `WM_HOOK_IME_MODE_DIAGNOSTIC` は毎回再入 → repost され、同じモーダルポンプが
   即座に再配送する。**モーダルループが続く限り回り続ける busy repost ループ**になる。

「トレイフォーカス時のノイズしか出ていない」という今回の観測は、この経路の
症状として説明がつく（少なくとも整合する）。

**修正**: この診断メッセージに `with_app_or_repost` は不適切。診断リング自体が
上限64件の耐久ストアであり、取りこぼしても**次の成功した drain が全部拾う**ので
ロスレス配送の必要がない。

```rust
WM_HOOK_IME_MODE_DIAGNOSTIC => {
    let _ = with_app(|app| message_handlers::handle_wm_hook_ime_mode_diagnostic(app));
}
```

`lib.rs:230-231` の `#[must_use]` 注記が言う「意図的に捨てる場合は
`let _ = with_app(...)`」がまさにこのケース。1行、機能追加ゼロ、repost 経路が1本減る。

**補足（同じ形の既存の正解が repo 内にある）**: `hook_channel.rs:203-212`
`request_engine_wake` は `WAKE_PENDING.swap(true)` で post をコアレスしている。
診断側の post（`hook.rs:1046`）には同等のガードが無く、IME モードキー1イベントにつき
1 `PostMessageW` を素で撃っている。上の1行修正で足りない場合（実測で post 数が
問題になった場合）は、この既存パターンをそのまま流用するのが筋で、新概念は要らない。

---

## §4 P3: ロック保持区間からアロケーションを外す（4行）

現状（`hook.rs:975-981`）:

```rust
pub(crate) fn drain_hook_ime_mode_diagnostics() -> Vec<crate::journal::HookImeModeDiagnosticRecord>
{
    let Ok(mut queue) = HOOK_STATE.ime_mode_diagnostics.lock() else {
        return Vec::new();
    };
    queue.drain(..).collect()          // ← ロック保持中にアロケーション
}
```

`collect()` は `HeapAlloc` を呼ぶ。Windows のヒープは（LFH のバケットに乗らなければ）
プロセス共有のロックを取る。つまり現状は

```
hook スレッド → ime_mode_diagnostics の Mutex → （メインスレッドが保持中）→ ヒープロック
```

という2段のロックネストがあり、ヒープロックの相手には `run_with_timeout` が起こす
ワーカースレッドや `LEAKED_THREADS` に park されたスレッドも含まれる。
論点1が言う「保持時間は理論上無限大になりうる」が唯一まともに刺さるのがここ。

**修正案**（置換用の空 deque をロック取得**前**に確保しておく）:

```rust
pub(crate) fn drain_hook_ime_mode_diagnostics() -> Vec<crate::journal::HookImeModeDiagnosticRecord>
{
    // 置換用バッファはロック取得前に確保する（保持区間からアロケーションを外す）
    let fresh = VecDeque::with_capacity(HOOK_IME_MODE_DIAGNOSTIC_CAP);
    let taken = {
        let Ok(mut queue) = HOOK_STATE.ime_mode_diagnostics.lock() else {
            return Vec::new();
        };
        std::mem::replace(&mut *queue, fresh)   // 3ワードの swap のみ
    };
    taken.into_iter().collect()                 // ロック外でアロケーション
}
```

これで保持区間は**構造体3ワードの入れ替えだけ**になり、`push_hook_ime_mode_diagnostic`
側（`hook.rs:965-973`）も capacity 64 の deque を受け取るため
`pop_front`/`push_back` で二度と grow しない＝**hook スレッド側もアロケーションフリー**が
保証される（現状は「`drain` が capacity を保持するから結果的にフリー」という
偶然に依存している）。

新しい型・static・テストは増えない。ADR-164 の不変条件
（`hook.rs:38-45` の doc コメント、および ADR-164:308-311）が主張している内容が、
はじめて文字どおり真になる。doc コメントの「ロック下でのアロケーションは
上限64件の `Vec` 収集のみに留める」という緩和条項は**削除できる**。

---

## §5 論点3への回答: 新しいリングは作らない、削除もしない

- **HookKeyRing の汎用化は不可**。`hook_channel.rs:32-42, 78-107` を読むと、
  この型は `CAP=1024` の `RawKeyEvent` スロット＋ overflow ラッチ
  （`OVERFLOW_LATCH_BIT`）を持ち、**ラッチが立つと hook コールバックが以後
  パススルー固定になる**という、キー再生順序の整合性と密結合した設計。
  診断用途で共有すると、診断のオーバーフローがキー入力経路のパススルー切替を
  誘発しうる。ADR-164 が避けた「ホットパスと診断の結合」を自分から作ることになる。
- **専用 lossy SPSC リング新設も不採用**。§4 の4行で保持区間の問題は消える。
  新しい unsafe な `UnsafeCell<MaybeUninit<..>>` リング（60行＋ Sync 実装＋
  `architecture_guard.rs` の Mutex カウントテスト更新）を、既に反証済みの
  懸念のために足すのは ADR-158 の北極星に逆行する。
- **診断ごと削除も不採用**。`hook.rs:1030-1037` に同内容の `tracing::debug!` が
  あるが、既定のログレベルは `info`（`app/bootstrap.rs:151`、`--debug` 時のみ
  `debug`、同 :137）。したがって**通常運用のユーザーの不具合報告には
  この debug 行は一切入らない**。ジャーナルリング（`journal.rs:276`
  `JournalEntry::HookImeModeDiagnostic`）が唯一の到達経路であり、
  削除すると今まさに読んでいる報告の種類の情報源を失う。

---

## §6 P1: 論点4（詰まり検知時点の OS 実 IME 状態の受動記録）

**これを最優先にすべき、という判断に同意する。P2/P3 とは独立に実施可能で、
順序依存は無い**（触るファイルは `runtime/message_handlers.rs:648-682` の
watchdog ハンドラで、`hook.rs` の診断リングには触れない）。

設計上、先に決めておくべき制約を3点だけ挙げる:

1. **3秒周期ではサンプリング粒度が粗すぎるので、レベルトリガではなくエッジトリガに
   すること。** watchdog は `runtime/mod.rs:1890` の通り3秒周期。毎回 journal に
   書くと3秒ごとのスパムになり、かつ「反転の瞬間」は依然として捉えられない。
   前回サンプルと**値が変わったときだけ**記録すれば、journal には
   「変化が起きた3秒窓」が残り、区間が絞れる。
2. **観測は絶対に belief を書かないこと。** `.claude/rules/ime-belief-architecture.md`
   と `feedback_observer_never_overrides_desired` の通り、Observe → 純粋
   `classify_*` → `reduce()` を通す。ここを近道すると、`098c663` が記録している
   「フォーカス変更時に spurious `apply_ime_open(false)` が発火して直接入力に落ちた」
   と同じ失敗ファミリーを新設することになる。watchdog は3秒ごとに必ず走るので、
   ここに書き込み経路を作るのは特に危険。
3. **IMM/TSF 読み取りは `run_with_timeout`（300ms、`win32-async/src/thread_timeout.rs`）
   越しにすること。** 現在の watchdog ハンドラは `GetTickCount64` と
   `GetLastInputInfo`（`hook.rs:730-739`）しか叩いておらず、ブロッキング
   Win32 呼び出しがゼロの区間。ここに素の IMM 呼び出しを置くと、
   「hook が詰まっているときにメインスレッドも詰まる」という最悪の相関を作る。

また、BUG-106（ローマ字⇔かな反転）に効かせたいなら、記録すべきは
open/close ではなく**ローマ字/かな入力フラグと conv mode** である点に注意
（open 状態だけ記録しても反転は見えない）。

---

## §7 ADR-164 をどう扱うか

**決定は変更不要。** ただし、次のセッションが同じ疑い（「唯一の Mutex が
hook 詰まりの犯人では？」）を再度持ち出して調査を繰り返さないよう、
ADR-164 訂正3 の末尾に**短い追記1段落**を足すことを勧める。
`.claude/rules/experiment-logging.md` が「なぜ前回それを捨てたのか」を残せと
言っているのと同じ動機で、今回は「採用/撤回」ではないが
**「疑ったが §1 の2点で排除した」という否定的証拠**こそ残す価値がある。

追記に含めるべき内容（3行で足りる）:

- watchdog 警告はメインスレッドの3秒 `WM_TIMER`（`runtime/mod.rs:1888-1892`）が
  出すため、その警告が出続けている時点でメインスレッドは当該ロックを保持していない
  （反証A）。
- `tick_hook_alive()` が `hook_callback` の最初（`hook.rs:995`）にあるため、
  `stale_ms > 5000` はロック保持 5000ms を要求する（反証B）。
- issue #165 / #137 の犯人は別にいる。

§3・§4 を実施する場合は、`hook.rs` を触るので
`.claude/rules/fix-requires-evidence.md` の対象判定を確認すること
（`hook.rs` は再発ファミリー表に直接は載っていないが、§3 の変更は
診断メッセージのディスパッチ方針変更であり挙動変更を含む）。
