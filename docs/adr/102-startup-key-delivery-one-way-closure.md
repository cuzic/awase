# ADR-102: 起動シーケンスとキー配送の一方通行を閉じる

## ステータス

**実装済み（2026-08-26、全面改訂、Windows実機ソーク未実施）。** [ADR-105](105-engine-thread-notification-via-hwnd.md)（エンジンスレッドへの通知はHWND宛のPostMessageWに統一する）を前提として全面的に書き直した。旧版（Opus2体による4ラウンドの敵対的レビュー＋追加の批判検証ラウンドを経て収束した版）は、対症療法の寄せ集めになっていた根本原因（`PostThreadMessageW` によるスレッドID宛通知の構造的脆弱性）そのものをOpusによる根本原因分析＋実機実験で解消できると判明したため、大部分を撤去・縮小した。Codex CLIが実装し、Opus敵対的レビュー→指摘10件の修正を経て、Claude自身が`cargo fmt`/`cargo xwin check`・`clippy`/`cargo test -p awase-windows`の全green化を独立に確認済み。実装は `develop` 上で行う（[main-develop-branch-flow](../../.claude/rules/main-develop-branch-flow.md)）。関連: [ADR-103](103-warmup-probe-pending-integrity.md)、[ADR-104](104-observation-freshness-and-hardening.md)、[ADR-105](105-engine-thread-notification-via-hwnd.md)。

### 旧版からの変更点（一覧）

| 旧決定 | 扱い |
| --- | --- |
| 1-a（TID前倒し） | **撤去**。[ADR-105](105-engine-thread-notification-via-hwnd.md) 決定1（エンジン専用ウィンドウの前倒し作成）に置き換え |
| 1-b（配送失敗時 `CallNextHookEx`） | **撤去**。[ADR-105](105-engine-thread-notification-via-hwnd.md) により配送そのものが失敗しなくなる |
| 1-b-2（OS所有マスク・フェーズ表・Altなりすましラッチのフェーズ繰り上げ） | **撤去**。BUG-41/BUG-62/BUG-48領域への最もリスクの高い変更だったが、配送保証を別の手段（ADR-105）で得られる以上、この変更は不要になった。ただし調査中に見つかった既存の穴（`reset_physical_key_state()` が `ALT_*` をクリアしない）は独立した小修正として残す（決定1の末尾） |
| 1-c（再入backstop） | **温存**。位置づけは変わらず |
| 1-d（drain再入安全化） | **温存**。実在するバグであり配送方式とは無関係 |
| 1-e（`deliver_key_event` 単一入口） | **温存・拡張**。[ADR-105](105-engine-thread-notification-via-hwnd.md) の `PumpContext` を受け取るよう拡張 |
| 1-f（`ModalPumpGuard`） | **縮小・温存**。「配送を守る」役割は不要になったが、「メニュー表示中はNICOLA変換しない」という処理ポリシーとしては必要 |
| 1-g（`NEEDS_ENGINE_RESYNC`） | **縮小・温存**。トリガーは「配送失敗」ではなく「意図的なpassthroughポリシーによる孤児KeyDown」のみに縮小 |
| （新設） | 決定2: フックコールバックの `Box::new` ヒープアロケーション撤去（SPSCリング） |
| 2-a（layoutsパニック） | **温存**。変更なし |
| 2-b（フォーカス遷移5層） | **大幅簡略化**。bootstrap専用の入口関数を新設し、「初回」という特殊ケース自体を定常経路から消す |

## コンテキスト

対象の指摘（元のコードレビュー19件のうち `[1][2][3][5]`）:

- **hook.rsの起動時レース**（`[1]`）と**app/mod.rsの再入時ドロップ**（`[3]`）
- **bootstrap.rsの起動時panic**（`[2]`、layoutsが空でindex out of bounds）
- **focus_tracking.rsの初回フォーカス取りこぼし**（`[5]`）

`[1]`と`[3]`の根本原因は、[ADR-105](105-engine-thread-notification-via-hwnd.md) が特定したとおり「フック→エンジンスレッドの通知が `PostThreadMessageW`（スレッドID宛のスレッドメッセージ）を使っており、(a) 起動時レース、(b) ネストしたモーダルポンプ中の恒久消失、という2つの構造的脆弱性を持つ」ことである。旧版はこの2点を「フックが配送失敗を検知して振る舞いを変える」という形（OS所有マスク・フェーズ表・`ModalPumpGuard`）で対症的に埋めようとしたが、[ADR-105](105-engine-thread-notification-via-hwnd.md) はこれを配送方式そのものの変更（hwnd宛の `PostMessageW`、実機実験で検証済み）で構造的に解消する。したがって本ADRは[ADR-105](105-engine-thread-notification-via-hwnd.md)適用後の姿を前提に書く。

`[2]`と`[5]`は配送方式とは独立した問題であり、旧版の分析がそのまま通用する。

### 制約

- フックコールバック（`WH_KEYBOARD_LL`）上の panic は OS 全体のキーボード入力をハングさせる。新たな panic 経路を持ち込まない。**フックコールバック上でロック取得・ブロッキング Win32 呼び出し・ログ出力を追加しない。** アロケーションについては決定2でむしろ既存の違反（`Box::new`）を除去する。
- IME belief の Observe → pure decision → Apply 3層分離（[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md)）を破らない。
- タイミング定数（`tuning.rs`）は本 ADR では変更しない。

## 不変条件

- **INV-A（配送）**: [ADR-105](105-engine-thread-notification-via-hwnd.md) により「消費したが誰も処理しない」という一方通行がなくなる。したがって本ADRでのINV-Aの役割は「配送失敗時の代替経路を作る」ことから、「配送は常に成功する前提のもとで、処理ポリシー（すぐ処理する／passthroughする）を正しく選ぶ」ことに変わる。
- **INV-G（押下サイクルの対称性）**: エンジンが KeyDown を見たなら、対応する KeyUp を必ず見るか、見られなかったことを知らされて自力で解放する。旧版では「配送失敗」がこれを破る主因だったが、本ADRでは「意図的なpassthroughポリシー」だけが破りうる（決定1参照）。

---

## 決定1: キー配送の単一入口とネストポンプ時のポリシー

[ADR-105](105-engine-thread-notification-via-hwnd.md) 決定4により、`WM_KEY_FROM_HOOK` はメインループ経由でもネストしたモーダルポンプ経由でも、同じ `engine_wnd_proc` → `dispatch_engine_message` を通って確実に配送される。本ADRが扱うのは、その配送された後の**処理ポリシー**である。

**1-a. キー配送の入口を1つにする（旧1-eを継承・拡張）。**
`INPUT_DEFER` から回収されたイベントは `handle_wm_drain_output_queue`（`message_handlers.rs:945` 付近）で `process_key_event` を**直接**呼び、`handle_wm_key_from_hook` の前段（NonText早期return・post-bypass latch消費、[ADR-103](103-warmup-probe-pending-integrity.md) 決定3の対象）を飛ばす。これは配送方式とは無関係な既存の実在バグであり、[ADR-105](105-engine-thread-notification-via-hwnd.md)適用後も残る。

```rust
// message_handlers: 物理キー1件をエンジンへ届ける唯一の入口。
// hook 経由（Main） / hook 経由（Nested、ADR-105のPumpContext） / INPUT_DEFER drain 経由の
// いずれもここを通る。
pub(crate) fn deliver_key_event(app: &mut Runtime, event: RawKeyEvent, origin: KeyOrigin) -> KeyDelivery;

pub enum KeyOrigin {
    Hook(PumpContext),   // ADR-105 決定4の PumpContext（Main | Nested）をそのまま運ぶ
    DeferredReplay,
}
pub enum KeyDelivery { Consumed, Reinjected }
```

契約は旧版と同じ: **通常のキー配送で `enqueue_reinject` を行うのは `deliver_key_event` だけ**とし、呼び出し側は「effects をいつ flush するか」だけを決める。

| 枝 | `deliver_key_event` の中でやること | 戻り値 |
| --- | --- | --- |
| `KeyOrigin::Hook(Nested)`（メニュー等の表示中） | `enqueue_reinject(event)`（NICOLA変換せずOSへ返す） | `Reinjected` |
| NonText早期return | `enqueue_reinject(event)` | `Reinjected` |
| post-bypass `PassthroughKeepArmed`/`ConsumeAndPassthrough`（[ADR-103](103-warmup-probe-pending-integrity.md) 決定3） | `enqueue_reinject(event)` | `Reinjected` |
| `process_key_event` → `PassThrough` | ctrl-check（composition cancel）の後 `enqueue_reinject(event)` | `Reinjected` |
| `process_key_event` → `Consumed` | 何もしない | `Consumed` |

例外は pending replay の2箇所に限定する。`runtime/key_pipeline.rs` の IME OFF rescue pending replay と、`runtime/message_handlers.rs` の `TIMER_IME_OFF_RESCUE` replay は、既に保留済みの同一イベントを救済窓の満了/中止でOSへ返す処理であり、hook/INPUT_DEFERから新規に配送されるキーを横取りする入口ではない。この2箇所は単一入口契約の対象外として明示的に許容し、`tests/architecture_guard.rs::enqueue_reinject_call_sites_are_accounted_for` で総数を固定する。

`origin` の用途は (i) `last_hook_activity_ms` の更新（`Hook(_)` のときのみ）、(ii) `Hook(Nested)` のときNICOLA処理をスキップする、の2つだけ。`tests/architecture_guard.rs` に「`process_key_event` を直接呼ぶのは `deliver_key_event` の中だけ」の grep guard を追加する。

**なぜ `Nested` のときNICOLA変換をスキップするか**: 「メニュー表示中に親指シフトで日本語を打つ」というユースケースは存在しない。メニュー・ダイアログのアクセラレータキーが正しく効くことを優先する（旧1-fの効果をそのまま継承。[ADR-105](105-engine-thread-notification-via-hwnd.md)適用後は、この判定に「配送が失敗するかもしれない」という心配が伴わない点が変わる）。

**1-b. `Nested` へ入退場する押下サイクルの対称性を保つ（旧1-g、トリガーを縮小）。**
旧版の `NEEDS_ENGINE_RESYNC` は「配送失敗」と「モーダルポンプ入場」の両方をトリガーにしていたが、[ADR-105](105-engine-thread-notification-via-hwnd.md)適用後は配送失敗が構造的に起きなくなるため、**トリガーは「モーダルポンプへの入退場」だけになる**。DOWN が `Main` で配送されエンジンが処理した後、対応する UP が `Nested`（passthrough）に回ると、エンジンは KeyDown だけを受け取り対応する KeyUp を永久に受け取らない——押下サイクルの対称性が破れる。

```rust
static NEEDS_ENGINE_RESYNC: AtomicBool;   // ModalPumpGuard の Enter/Drop で立てる
```

- **立てる側**: [ADR-105](105-engine-thread-notification-via-hwnd.md) の `ModalPumpGuard`（決定4で温存されるモーダルポンプ検出機構、詳細はそちらを参照）の Enter と Drop の両方で `true` にする（メニュー表示中に押されたキーの押下サイクルが、メニューを開く前・閉じた後のどちらでも跨ぎうるため）。
- **落とす側**: `deliver_key_event` の先頭で次を実行する。

  ```rust
  if NEEDS_ENGINE_RESYNC.swap(false, Ordering::AcqRel) {
      let ctx = app.build_ctx();
      let decision = app.engine.on_command(EngineCommand::FocusChanged, &ctx);
      app.execute_decision_suppressed(decision);
  }
  ```

  `on_command`（`src/engine/engine.rs:471`）は `&InputContext` を要求し、`handle_focus_changed` は `flush_to_effects`（入力途中のsuppress）と `flush_pending_key_ups`（Consume済みでKeyUpが来ていないキーの再注入）の結果を `Decision` に詰めて返す。**`Decision` を捨てると、pendingリストは空にされた上で `ReinjectKey` Effectだけが実行されない**——1-bの存在理由が逆転する。`execute_decision_suppressed`（`runtime/ime_refresh.rs:263` と同じ形、`suppress_engine_state_key_guard` 付き）を使うことで、resync のたびにengine状態遷移由来のIMEキー送信が起きないようにする。
- 代償: モーダルポンプ入退場のたびに入力途中が1回flushされる。頻度は「配送失敗のたび」だった旧版より大幅に低い（メニュー操作自体が低頻度であるため）。

**1-c. `WM_KEY_FROM_HOOK` の再入は捨てずに `INPUT_DEFER` へ戻す（backstop、旧1-c）。**
`app/mod.rs:515` の `debug_assert!` を削除し、`with_app` が `None` を返したら `INPUT_DEFER.replay_later(std::iter::once(event))` にする。**これは新しい機構ではない**——`app/mod.rs:512` 付近の `has_pending_drain` 分岐が既に同じ呼び出しを使っており、1-cはその `else` 側に3行足すだけである。

**到達可能性について**: `with_app` が `None` を返すのは (i) 再入、(ii) `RUNTIME` 未初期化/シャットダウン中、の2通り。従来の分析では `run_message_loop` のトップレベルからしか呼ばれないため到達可能性が低いとされていたが、[ADR-105](105-engine-thread-notification-via-hwnd.md) 決定4で `WM_KEY_FROM_HOOK` がネストしたモーダルポンプ経由でも `engine_wnd_proc` へ届くようになる。**ネストポンプの中で `with_app` を握ったまま何かを待つ経路（例: `with_app` 保持中の `SendMessageTimeoutW` 待機中に、シェルが `WM_TRAY_CALLBACK` を send し `TrackPopupMenu` が始まる）が存在すると、再入が今までより現実的になりうる**。実装時にこの経路の有無を再確認すること（未解決の疑問に記載）。

**1-d. drainを再入安全にする。ただし通常経路のレイテンシは増やさない（旧1-d、変更なし）。**
`handle_wm_drain_output_queue` の `take_all()` を `with_app` の**内側**へ移す。

```rust
pub(crate) fn drain_deferred_inputs(app: &mut Runtime);
static DRAIN_PENDING: AtomicBool = AtomicBool::new(false);
```

通常経路は現状維持。再入時は `DRAIN_PENDING` を立てるだけでその場では何も post しない。回収点は「次にRuntimeを掴んだハンドラの末尾」と、既存のウォッチドッグタイマー（backstop、新しいタイマーも新しい時間定数も導入しない）。

**証拠義務**: 打鍵消失の再発ファミリー。`docs/known-bugs.md` に暫定 **BUG-80** を起票し、「[ADR-105](105-engine-thread-notification-via-hwnd.md)適用前は専用フックスレッドが常時ポンプする実機構＋モーダルポンプがhwnd=NULLのスレッドメッセージを捨てる実機構により打鍵が消えていたが、適用後はhwnd宛配送により消失経路が閉じ、残るのは押下サイクルの対称性（1-b）と再入時の順序保存（1-c/1-d）だけになった」という経緯を記録する。テストは `deliver_key_event` の入口テスト（`KeyOrigin::Hook(Nested)` でNICOLAを通らず1回だけreinjectされる）、`NEEDS_ENGINE_RESYNC` のswap規約テスト、`DRAIN_PENDING` の回収テスト。

### 独立した小修正: `reset_physical_key_state()` の `ALT_*` クリア漏れ

旧1-b-2（OS所有マスク）の検討過程で、`reset_physical_key_state()`（`hook.rs:309-319`）が `PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS`/`LEFT/RIGHT_THUMB_DOWN_AT_US` はクリアするが、**`ALT_L/R_IMPERSONATING`/`ALT_L/R_WAS_DOWN` を一切クリアしない**という既存の穴が見つかった。OS所有マスクの設計自体は撤去したが、この穴は独立した実在の潜在バグである——セッションロック中（`hook.rs:295-308`、2026-07-09実機確認済みでKeyUpがフックに届かないケース）にAltを押していた場合、`panic_reset()`/セッションロック解除でも `ALT_*` は固着したままになる。`reset_physical_key_state()` に `ALT_L/R_IMPERSONATING`/`ALT_L/R_WAS_DOWN` のクリアを追加する（1行の独立した修正、[ADR-105](105-engine-thread-notification-via-hwnd.md)の適用と無関係に先に入れてよい）。

---

## 決定2: フックコールバックのヒープアロケーションを撤去する（新設）

`hook.rs:927` の `Box::new(event)` は、フックコールバックが**毎打鍵でグローバルアロケータのヒープロックを取る**ことを意味する。これは本ADR自身が交渉不可の制約として掲げる「フックコールバック上でロック取得・ブロッキング呼び出しを追加しない」に、既存コードが**既に抵触している**（禁止すべき新規追加ではなく、既存の是正対象）。

```rust
// crates/awase-windows/src/hook_channel.rs（新設、ungated な純粋部 + cfg(windows) の post）

/// フックスレッド(単一producer) → エンジンスレッド(単一consumer) の SPSC リング。
/// アロケーション・ロック・ログ・ブロッキング呼び出しを一切しない。
pub struct HookKeyRing {
    slots: [UnsafeCell<MaybeUninit<RawKeyEvent>>; CAP],  // CAP = 256（2の冪）
    head: AtomicUsize,     // producer が Release store
    tail: AtomicUsize,     // consumer が Release store
    dropped: AtomicU32,    // 満杯で捨てた件数
}

impl HookKeyRing {
    /// フックスレッド専用。満杯なら最新を捨てて dropped を +1（既存の順序を壊さない）。
    pub fn produce(&self, ev: RawKeyEvent) -> ProduceResult;   // Accepted | Overflow
    /// エンジンスレッド専用。タイムスタンプ順に全件取り出す。
    pub fn consume_all(&self, sink: &mut impl FnMut(RawKeyEvent));
    pub fn take_dropped(&self) -> u32;
}
pub static HOOK_KEYS: HookKeyRing = HookKeyRing::new();

/// 合図の重複を潰す。consumer は drain の前に false へ戻す。
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);
```

フックコールバック末尾（`hook.rs:925-942` の置換）:

```rust
let overflow = HOOK_KEYS.produce(event);          // アトミック store のみ
if !WAKE_PENDING.swap(true, Ordering::AcqRel) {
    crate::win32::post_to_main_thread(WM_KEY_FROM_HOOK);   // ADR-105の集約点、失敗しない
}
LRESULT(1)
```

`RawKeyEvent` は既に `Copy` の POD（スタック値のみ、ヒープ参照なし、`src/types.rs:190-220`）なので、リングへの格納に追加のアロケーションは不要。

- **`Box::new`/`Box::from_raw` の二重解放・リークの可能性が消える。**
- **合図（`WM_KEY_FROM_HOOK`）が1回失われても実害がない。** イベントはリングに残っており、次の合図・次のハンドラ末尾（1-d の `DRAIN_PENDING` 相当の考え方をリング側にも適用）・既存のウォッチドッグタイマーのいずれかで必ず回収される。
- オーバーフロー（256件未処理＝エンジンが数秒止まっている）だけが唯一の異常系で、そこでのみ 1-b 相当の resync を1回発行すればよい。

**証拠義務**: 打鍵消失の再発ファミリー。`HookKeyRing` の純粋部（`produce`/`consume_all`/オーバーフロー時の挙動）は Linux 実行可能な全数テストを置く。`docs/known-bugs.md` の暫定 **BUG-80** に、フックコールバックの `Box::new` 撤去も含める。

---

## 決定3: 起動シーケンスの「初回だけ通らない」を消す

**3-a. レイアウト集合を「空でないこと」が型で保証された値にし、失敗を回復可能な形で伝える（旧2-a、変更なし）。**
`app/bootstrap.rs:193` の `&layouts[index]` は `layouts` が空なら panic する。`runtime/mod.rs::reload_layouts` は同じ空チェックを既に持っており、起動パスだけが抜けている。

`runtime::NonEmptyLayouts::new(Vec<LayoutEntry>) -> Option<Self>` を導入し、空なら `None`。この型は純粋な「空でないこと」の保証だけを持ち、MessageBox表示や設定画面起動のようなUI副作用をコンストラクタに持たない。空だった場合の `win32::show_error_dialog` と設定画面起動は bootstrap の呼び出し側で行い、`runtime/mod.rs::reload_layouts` も同じ `NonEmptyLayouts` を通して同一判定に揃える。

**なぜ既定レイアウトへのフォールバックにしないか**: 埋め込みの既定レイアウトはリポジトリに存在せず、「空でもとりあえず動く」は「親指シフトが効かない」としか見えない無言の劣化になる。

**3-b. フォーカス遷移の「初回」を、bootstrap専用の入口関数で消す（旧2-bを大幅簡略化）。**

旧版は `FocusTransition` enum（Bootstrap/SameProcess/ProcessChanged）を5層（`advance_focus_tracking`内部・戻り値・呼び出し分岐＋`on_focus_process_changed`の分割・`ir_notify_focus_changed`・reduceアーム）に配り直す設計だった。これは「起動直後の最初のフォーカスで `process_changed` が `false` になる」という**定常経路のbool判定を、特殊ケース(Bootstrap)にも対応させようとした**結果であり、5箇所すべてで「このケースはBootstrapか否か」を正しく判定し続ける必要があった。

**より根本的な解決**: bootstrapの初回フォーカスは、message loop上の稀なケースではなく、`initialize_ime_cache()` という単一の決定論的な呼び出しに帰着する（`bootstrap.rs:966,982,992` の順序——`install_focus_hook`から`run_message_loop`までの区間、メインスレッドは一度もポンプしないため、WinEvent由来のフォーカスイベントはここに割り込めない）。**「初回」を型で表現して定常経路に流すのではなく、bootstrapに専用の入口を与えれば `Bootstrap` というケース自体が定常経路に到達しなくなる。**

```rust
impl Runtime {
    /// 起動時に1度だけ呼ぶ。belief には一切触れず、「今どの窓を見ているか」という
    /// スコープだけを確立する。これ以降 platform.focus.current.pid != 0 が恒真になるため、
    /// advance_focus_tracking の process_changed は特殊ケースを考慮しない正直なboolに戻る。
    pub(crate) fn establish_initial_focus_scope(&mut self) {
        // reset_candidate_was_seen / last_focus_change_ms 初期化 / focus_epoch += 1 /
        // notify_focus_changed / active_keymaps 更新 / update_focus_info。
        // on_focus_process_changed のうち belief に触れない部分だけを切り出したもの。
    }
}
```

`bootstrap.rs` は次の2行になる。

```rust
let _ = with_app(Runtime::establish_initial_focus_scope);
initialize_ime_cache();     // ここで初めて実観測 → belief が立つ
```

効果:

- `advance_focus_tracking`・`apply_focus_probe_result`・`ir_notify_focus_changed`・reduceアームは**変更不要**になる。`process_changed`/`FocusTransition`という型を新設する必要自体がなくなる（message loop上では、起動後最初のフォーカスも含めて`last_pid`が既にセット済みなので、常に正直な比較になる）。
- 「Bootstrapでbeliefを触ってはならない」は、そもそもbeliefを触るコードがbootstrap経路に存在しなくなるので、規約ではなく構造で保証される。
- `on_focus_process_changed`（`focus_tracking.rs:255`）自体は分割せず現状維持でよい——定常経路（`SameProcess`でない実際のプロセス変更）でのみ呼ばれる関数として、そのままの凝集度で十分に正しい。

**確認事項**: `establish_initial_focus_scope` が `focus_epoch` をインクリメントした直後の `initialize_ime_cache()`（`IoMode::Sync` の同期経路）は非同期probeをspawnしないため、世代の不整合は起きない。

**証拠義務**: focus遷移ファミリー。`docs/known-bugs.md` に暫定 **BUG-81** を起票し、「bootstrapの初回フォーカスは専用入口を持ち、定常経路の`process_changed`判定には特殊ケースが存在しない」という設計方針を記録する。テストは `establish_initial_focus_scope` の本体に belief 書き込みパターンが一切出現しないことを固定する `architecture_guard`（`uia_async_focus_kind_handler_does_not_write_belief` と同型）1本と、起動シーケンスのcharacterizationテスト（`establish_initial_focus_scope`→`initialize_ime_cache`の順で呼ばれ、`focus_epoch`が1つだけ進むこと）。

---

## 実装順序

| Phase | 内容 | 依存 |
| --- | --- | --- |
| 0 | [ADR-105](105-engine-thread-notification-via-hwnd.md) の実装（エンジン専用ウィンドウ・集約点差し替え・Ctrl+C/`--exit-after`修正） | なし |
| 1 | 独立した小修正: `reset_physical_key_state()` の `ALT_*` クリア追加 | なし（Phase 0と並行可） |
| 2 | 決定1: 1-d（drain再入安全化）→ 1-a（`deliver_key_event`単一入口、`PumpContext`統合）→ 1-c（backstop）→ 1-b（`NEEDS_ENGINE_RESYNC`縮小版） | Phase 0 |
| 3 | 決定2（`HookKeyRing`、`Box`撤去） | Phase 0（`post_to_main_thread`が失敗しない前提を使う） |
| 4 | 決定3（3-a独立、3-bは`establish_initial_focus_scope`新設） | 独立 |

## 却下した代替案

- **旧版のOS所有マスク・フェーズ表による配送保証**: [ADR-105](105-engine-thread-notification-via-hwnd.md)が構造的に配送を保証するようになったため、フック側で複雑な分岐（BUG-41/BUG-62/BUG-48領域への並び替えを要求する）を持つ必要がなくなった。実装リスクの高い変更を避けられる。
- **`FocusTransition`を5層に配り直す（旧2-b）**: bootstrap専用の入口関数で「初回」というケース自体を消すほうが、変更箇所も新設する型も少ない。ただし`on_focus_process_changed`が将来さらに複雑化した場合、スコープ確立とbelief確立を分割する価値が出てくる可能性はある——そのときは旧版の層2の分割案を再検討する。
- **`with_app`に汎用の再入キューイング機構を入れる**: 全ハンドラの実行タイミングが変わり `INPUT_DEFER` の順序保証と競合する。

## 未解決の疑問（実機ソークで確認すること）

- 1-cの到達可能性が[ADR-105](105-engine-thread-notification-via-hwnd.md)適用後に実際どう変わるか（ネストポンプ中に`with_app`を握ったまま待機する経路が新たに生まれていないか）を実装時にコードで再確認すること。
- 決定2の`HookKeyRing`のオーバーフロー（256件）が実機でどの程度の頻度で起きるか、`dropped`カウンタで観測すること。
- 決定3-bの`establish_initial_focus_scope`と`initialize_ime_cache`の間で、将来非同期化される変更が入った場合に世代整合性が壊れないか、変更のたびに確認すること。
- 決定1-bのモーダルポンプ入退場によるresync flush頻度は、配送失敗トリガーだった旧版より大幅に低いと予想されるが、実機ソークで確認する。

## 設計の経緯

19件のコードレビュー指摘を起点に、Opus 2体でドラフト→敵対的レビューを4ラウンド実施し初版を収束させた。その後、この初版自体に対する追加の敵対的レビュー（批判→その批判自体の検証、の2段構え）で3件の実装可能性の問題を是正した（フェーズ表の見落とし、5層モデルの目的未達成、疑似コードの誤り）。

その後、ユーザーの「もっと根本的に良い設計はないか」という問いを受けてOpusによる根本原因分析を実施し、旧版が対症療法の寄せ集めになっていた単一の根本原因（`PostThreadMessageW`によるスレッドID宛通知の構造的脆弱性）を特定した。実機実験（dragonflyg4、2026-08-26）でhwnd宛`PostMessageW`がネストしたモーダルポンプ中でも確実に配送されることを検証し、この知見を独立した基盤ADR（[ADR-105](105-engine-thread-notification-via-hwnd.md)）として切り出した上で、本ADRをその上に立つ形へ全面的に書き直した。旧版で最もリスクが高いと評価されていた決定（OS所有マスクとAltなりすましラッチのフェーズ繰り上げ）が丸ごと不要になったことが、この根本再設計の最大の成果である。
