# P1（フック詰まり検知時点の OS 実 IME 状態の受動記録）設計レビュー

> **これは正式な ADR ではない。** `docs/design/` 配下に置いた一時的な相談記録であり、
> ここでの結論を採用する場合は ADR 起票または `docs/known-bugs/BUG-106.md` への
> 追補を別途行うこと。
>
> 対象: `/tmp/.../scratchpad/p1-design-draft.md`（P1 設計ドラフト）
> 前提: [opus-review-hook-mutex-safety.md](./opus-review-hook-mutex-safety.md) §6
> 日付: 2026-09-13 / 読んだ版: `fix/hook-diagnostic-repost-and-lock-alloc` (`79cc1d9e`)

---

## §0 結論（先に述べる）

**「この設計を修正すべき」。** 修正は 2 点で、片方は P1 の中身の差し替え、
もう片方は P1 より先にやるべき別の修正の発見である。

1. **P1 の probe 機構を差し替える。** ドラフトが選んだ
   `read_ime_state_full_async`（IMM クロスプロセス読み取り）は、**捉えたい対象
   （Chrome / Edge / Teams 上の MS-IME）では値が返らないか、返っても信用できない**
   （§2）。BUG-106 が実機で追従を確認済みの唯一のチャネルは
   `GetKeyState(VK_KANA)&1`＝既存の `observer/kana_lock.rs::read_kana_lock()` であり、
   これは同期・非ブロッキング・ワーカースレッド不要。差し替えると
   非同期タスク・in-flight フラグ・`LEAKED_THREADS` の懸念が**丸ごと消える**（§3）。
2. **P1 より先に、ノイズ 25 件の経路そのものを塞ぐ（P0）。** 調査中に判明したが、
   このノイズは**ログノイズではなく belief 汚染**である。`awase_tray_window` から
   読んだ `is_romaji=false` が Medium confidence で `input_mode = ObservedKana` を
   belief に書き込み、その結果 `decide_needs_romaji_pre_write` が false になって
   **awase 自身の ROMAN 補完が止まる**（§4）。BUG-106 の「1〜2 日直らない」という
   報告と噛み合う道筋が引ける。これは純粋関数のユニットテストで安く潰せる。

推奨順序: **P0（§4）> P1'（§3）> ドラフトの非同期 IMM probe（採用しない）**

なお「まず `fg_class` をログに出すだけ」という軽い代替案については、
**`foreground_class_name()` は既に実装済み**（`observer/kana_lock.rs:26-38`）で、
§3 の推奨設計はそれに 1 行足しただけの規模に収まる。つまり「軽い代替案」と
「P1」は実装コスト上ほぼ同じところに着地する。

---

## §1 裏取り結果（依頼の 6 項目）

### 1. `classify_focus` タイムアウト時に `focus_kind` は `Undetermined` になるか → **なる。ただし穴が 3 つ**

`resolve_focus_kind`（`focus/kind_classifier.rs:74-81`）はタイムアウト時に
`FocusKind::Undetermined` を返し、`classify_focus_probe`
（`runtime/focus_tracking.rs:314-319`）が `FocusStore::focus_kind` に**そのまま代入**する。
キャッシュ汚染も無い（`focus/cache.rs:84` が `Undetermined` の insert を弾く）。
ここまではドラフトの前提どおり。

穴は以下の 3 つで、いずれも「`focus_kind` は**フォーカス probe が最後に成功した
時点**の値であって、watchdog 発火時点のフォアグラウンドの属性ではない」ことに由来する。

- **穴A（偽陰性）**: `resolve_focus_kind` は**エンジンタイマー活性中**（＝ユーザーが
  実際に打鍵中）にも `Undetermined` を返す（`kind_classifier.rs:52-59`）。
  捉えたい「実アプリで入力中」の瞬間が、この分岐で落ちうる。
- **穴B（偽陽性、これが本命）**: **フォーカス probe 自体がタイムアウトすると
  `focus_kind` は前回値のまま維持される**（`focus_tracking.rs:258-262`、
  `Focus probe timed out — skipping update this cycle` で `return None`）。
  BUG-106 追補3 が記録した 25 件はいずれも「`classify_focus` が 300ms
  タイムアウトする状況」＝**probe 系が軒並み詰まっている状況**なので、
  `focus_kind` が Chrome 由来の `TextInput` のまま固まっている確率が高い。
  この状態でトレイにフォーカスがあると、ゲートは**素通りする**。
- **穴C**: `cache_get` ヒット時（`kind_classifier.rs:43-49`）は過去の分類結果が
  そのまま返る。プロセス＋クラス名キーなので通常は妥当だが、「今この瞬間の
  フォアグラウンド」を保証するものではない。

→ **`focus_kind == TextInput` ゲートは 25 件を排除する保証にならない。**
watchdog 時点の事実がほしいなら、`GetForegroundWindow()` のクラス名を
その場で読む方が正確かつ安い（§3）。

### 2. `awase_tray_window` / `awase-settings.exe` の分類 → **構造的にタイムアウト必至**

`awase_tray_window`（`tray.rs:138`）も awase-settings の winit クラス
（`focus/classifier.rs:586` のテストが `"Window Class"` を使っている）も、
`classify_focus` の既知テキスト／非テキストクラス表（`focus/classify.rs:98-161`）の
**どちらにも載っていない**。したがって MSAA へ落ちる（`classify.rs:165`）。

MSAA は `AccessibleObjectFromWindow`（`focus/msaa.rs:111`）で、内部的に対象
ウィンドウへ `WM_GETOBJECT` を送る。**トレイウィンドウの所有スレッドは awase の
メインスレッド自身**であり、そのメインスレッドはこのとき
`resolve_focus_kind` → `run_with_timeout` → `rx.recv_timeout(300ms)` で
**ブロックしている**（`win32-async/src/thread_timeout.rs:87`）。
つまり**自己デッドロックにより 300ms 必ずタイムアウトする**。
BUG-106 追補3 の「25 回中 25 回が `classify_focus timed out` と一致」という
観測は、偶然ではなくこの構造の帰結として説明がつく。

→ ドラフトの疑問点「トレイが `NonText` と正しく分類される可能性」は、
**トレイ自身については事実上ゼロ**（常に `Undetermined`）。
ただし §1-1 の穴B があるため、ゲートの成否はこれとは別問題。

### 3. `read_ime_state_full_async_with_timeout` は **存在しない**（ドラフトの事実誤認）

`ime.rs:753` 付近に実在するのは `read_ime_state_full_async()`（タイムアウト無し）で、
実体は `offload_unsafe` →`win32_async::offload`（`win32-async/src/offload.rs:82`）。
メインスレッドをブロックしない点はドラフトの主張どおり正しい（初回 poll で
`std::thread::spawn` して `Poll::Pending`、完了時に `PostMessageA` ベースの
Waker で起床）。が、**`run_with_timeout` は経由しない**。したがって
opus-review-hook-mutex-safety.md §6 の制約3（「`run_with_timeout` 越しに」）を
**文字どおりには満たさない**。

さらに `offload` は:

- **タイムアウトを持たない**。`offload_timeout`（`offload.rs:101`）を使っても
  「ワーカースレッドはバックグラウンドで実行継続する」と doc に明記がある。
- **`LEAKED_THREADS` に載らない**。上限 8 のガード（`thread_timeout.rs:55,76-83`）の
  **外側**で無制限に `std::thread::spawn` する。
  3 秒周期の watchdog から永久ブロックする IMM 呼び出しを叩き続けると、
  上限なしにスレッドが積み上がる——`run_with_timeout` より**悪い**資源特性になる。

→ 非同期案を採るなら、制約3 は「`offload` ではなく `run_with_timeout` を
ワーカー側で使う」か「`offload_timeout` + 呼び出し回数の自前上限」まで
書き下さないと成立しない。§3 の推奨設計ならこの論点自体が消える。

### 4. `TIMER_TSF_PROBE` の `BorrowError` 回避策 → **同じ穴は踏まない。ただし理由を取り違えないこと**

`message_handlers.rs:578-589` が `diagnostic_snapshot` をスレッドローカルへ
退避しているのは、`advance_tsf_probe` が**同期的にその場で** `with_app_ref`
（共有借用）を呼ぶためで、`WM_TIMER` ハンドラは `with_app`（`app/mod.rs:376-385`）の
**排他借用の中**にいる。同一スタック上での再借用なので `BorrowError` になる。

`spawn_local(async { ... .await; with_app(..) })` はこれに該当しない。
`spawn_local` した future の初回 poll は最初の `await`（= `offload` の spawn）で
`Pending` を返し、`with_app` は**await から戻った後**＝別のメッセージ配送サイクルで
呼ばれる。`spawn_ime_refresh`（`runtime/mod.rs:757-765`）が既にこの形で、
`&mut self` メソッドの中から `spawn_local` して完了後に `let _ = with_app(..)` している。
**この既存パターンをそのまま踏襲する限り安全**。

注意すべきは別の 2 点:

- 完了ハンドラでは `with_app_or_repost` を使わないこと。診断は取りこぼしてよいので
  `let _ = with_app(..)` が正しい（前回レビュー §3 と同じ理由）。
- 3 秒周期で spawn するため、**完了順と発火順が入れ替わりうる**。
  edge 判定を「前回記録値との比較」で行うなら、順序逆転で偽の edge が出る。

### 5. `idle_conv_check_in_flight_ms` の多重発火防止 → **`Option<u64>`（開始 tick）＋自己回復しきい値**

実体は `GateStore::idle_conv_check_in_flight_since_ms: Option<u64>`
（`state/platform_state.rs:1635`）。`key_pipeline.rs:704-716` で「Some なら
経過を見て、しきい値（同 `:1677` の定数）を超えていれば強制的に自己回復、
そうでなければスキップ」→ 発火時に `Some(now)`、完了ハンドラ 2 箇所
（`key_pipeline.rs:768,775`）で `None` に戻す。

**単なる `bool` ではなく「開始時刻」を持つのが肝**で、完了ハンドラが
何らかの理由で走らなかったときに恒久的に詰まらない。非同期案を採るなら
このイディオムをそのまま流用すればよい（新概念は不要）。

### 6. `LEAKED_THREADS` の資源評価 → **ドラフトの懸念は的を外している。ただし別の穴がある**

- `offload` は `LEAKED_THREADS` を**一切参照しない**（§1-3）。したがって
  「上限 8 の枯渇」は非同期 probe の直接のリスクではない。
- 一方 `read_ime_state_full` は**内部で** `get_gui_thread_info_with_timeout(200ms)`
  を呼び（`ime.rs:658`）、これが `run_with_timeout`（`win32.rs:353-359`）である。
  つまり **1 回の probe = スレッド 2 本**（offload 1 + 内側の run_with_timeout 1）で、
  **内側だけが上限 8 にカウントされる**。
- `TIMER_TSF_PROBE`・`spawn_ime_refresh`・focus probe・`kp_stage_idle_conv_check` が
  既に同じ上限 8 を共有している。3 秒周期の新規 probe を足すと、
  「フォアグラウンドが本当にハングしている」状況（＝まさに watchdog が
  発火する状況）で**上限 8 を食い合い、既存 probe が `refusing to spawn new worker`
  で機能停止する**（`thread_timeout.rs:76-83` は `tracing::error!` を出して `None` を返す）。
  診断のために本番経路を殺すことになりうる。

→ これは§3 の推奨設計（Win32 呼び出しが `GetKeyState` と `GetClassNameW` のみ）なら
発生しない。

---

## §2 設計上の致命的な問題: probe の対象アプリで値が返らない

ドラフトが記録しようとしている `is_romaji` / `conversion_mode` は、
**BUG-106 の対象アプリでは取得できないか、取得できても信用できない**。

- `read_ime_state_full`（`ime.rs:655-740`）が早期 return するのは
  `is_tsf_native_window`（`focus/class_names.rs:51-61`）の場合だけで、
  そのときは `is_romaji: None, conversion_mode: None` を返す。
  **WezTerm / Windows Terminal / UWP では P1 は常に空振りする。**
- Chrome (`Chrome_WidgetWin_1` / `Chrome_RenderWidgetHostHWND`)・Edge・
  Teams (`TeamsWebView`) は `is_tsf_native_window` には**該当しない**ので早期 return
  せず、IMM クロスプロセス読み取りへ進む。しかしこれらは
  `IMM32_UNAVAILABLE_CLASSES`（`class_names.rs:19-35`）であり、その doc が
  「`ImmGet*` / `SendMessage(WM_IME_CONTROL)` は反応しなかったり**無期限に
  ブロックする恐れがある**」と明記している。**`read_ime_state_full` は
  `IMM32_UNAVAILABLE_CLASSES` を参照していない**（`is_tsf_native_window` しか見ない）
  ——ここが構造的な抜けで、§4 のノイズ源そのものでもある。
- BUG-106 本文も「Teams で conv 読み取り経路が存在しない根拠」として
  同じことを書いている。

対して、**BUG-106 は `GetKeyState(VK_KANA)&1` が実機で追従することを確認済み**
（`examples/spike_kana_lock_probe.rs`、`fg_class=TeamsWebView` で
`KANA bit=on/off` が言語バー操作に追従）。これが**唯一、対象アプリで動くことが
実証されているチャネル**である。

→ P1 が IMM を読む限り、「反転の瞬間」を捉える確率は構造的に低い。
実装コストを払っても空振りする設計になっている。

---

## §3 推奨する P1'（差し替え設計）

### 置き場所

`crates/awase-windows/src/runtime/message_handlers.rs` の `TIMER_HOOK_WATCHDOG` 分岐
（`:648-682`）、`stale_ms > 5000` かつ `os_idle_ms < 5000`（＝「フックにイベントが
届いていない疑い」）の枝の中だけ。

### 読むもの（すべて既存関数、新規 unsafe ゼロ）

```rust
// SAFETY: WM_TIMER ハンドラはメッセージループスレッド上で実行される。
let reading = unsafe { crate::observer::kana_lock::read_kana_lock() };
let fg_class = unsafe { crate::observer::kana_lock::foreground_class_name() };
```

- `read_kana_lock()`（`observer/kana_lock.rs:10-21`）= `GetKeyState(VK_KANA)&1`。
  クロスプロセス通信なし、ブロックしない、ワーカースレッド不要。
- `foreground_class_name()`（同 `:26-38`）= `GetForegroundWindow` + `GetClassNameW`。
  こちらもブロックしない。**§1-1 の穴A/B/C をすべて回避できる**
  （`FocusStore` のキャッシュ値ではなく、その瞬間の実値を読むため）。

### 記録先

`Runtime` の平フィールドを 1 本足す。直接の前例が同じ構造体にある
（`runtime/mod.rs:330` の `kana_lock_hysteresis: KanaLockHysteresis`）。

```rust
/// watchdog が「フック詰まり」を検知した時点の OS かな入力ロック（診断専用、edge 記録用）。
/// belief にも kana_lock_hysteresis にも投入しない。
watchdog_kana_sample: Option<(KanaLockReading, String)>,
```

初期化は `runtime/mod.rs:1542` 付近（`kana_lock_hysteresis` の隣）、
**設定リロード時のリセット（同 `:706`）にも足すこと**——`kana_lock_hysteresis` が
そこでリセットされているのと同じ理由で、足し忘れると reload を跨いで
古い値と比較して edge を取りこぼす。

### edge 判定と出力

前回記録と `(reading, fg_class)` が異なるときだけ `tracing::warn!`。
同じなら何もしない（3 秒ごとのスパムを防ぐ、制約1）。
ログには `stale_ms` / `os_idle_ms` を必ず併記する（§3 末尾の caveat のため）。

### 3 制約との対応

| 制約 | 充足 |
| --- | --- |
| 1. edge トリガ | ○ 前回値との比較のみ |
| 2. belief を書かない | ○ `ImeModel` にも `kana_lock_hysteresis` にも触れない。**`hysteresis` に投入しないことを明示的にコメントへ書く**——BUG-106 追補1 が「このノイズで `kana_input_warn` が誤発火しないか」を次回調査対象に挙げており、watchdog から投入すると打鍵ベース（On 3 連続）の前提を壊してトレイ警告を誤表示する |
| 3. ブロッキング API を通さない | ○ そもそも `run_with_timeout` が必要な API を呼ばない（制約の趣旨を上位互換で満たす） |

新しい `Mutex`・`static`・`AtomicX` はゼロ。ADR-164 の
「hook スレッド⇔メインスレッドの共有 state はロックフリー atomic のみ」との
整合についての理解（**この機能はメインスレッドのみが触るので hook スレッドとの
共有は無い**）は**正しい**。`WM_TIMER` は `with_app`（`app/mod.rs:376-385`）経由で
メインスレッド上、`read_kana_lock`/`foreground_class_name` も同スレッド前提の
`unsafe fn` であり、既存の呼び出し元（`key_pipeline.rs:2717,2728`）と同じ文脈にある。

### 実測で確認すべき既知の caveat（実装前に潰さないこと、ログで測ること）

`GetKeyState` は**呼び出しスレッドの入力キュー同期に依存**して更新される。
「hook にイベントが届いていない」状況では、その同期自体が怪しい可能性がある
——つまり watchdog 時点の読み値が**凍結している**かもしれない。
これは設計を止める理由にはならない（凍結しているなら「凍結していた」こと自体が
issue #165 の新しい証拠になる）が、**`stale_ms` を併記しないと
「反転を検知した／しなかった」の解釈を誤る**。
`.claude/rules/tuning-constants.md` の趣旨どおり、新しい定数は足さず
watchdog 既存の 3 秒周期と 5000ms しきい値をそのまま流用すること。

---

## §4 P1 より先にやるべき修正（P0）: ノイズは belief を汚染している

ここが今回の調査で一番重要な発見である。**BUG-106 追補3 が「ノイズ」と
結論づけた 25 件は、ログに出るだけではなく belief を書き換えている。**

### 経路（すべて確認済み）

1. `input_mode_from_romaji_flag`（`observer/ime_observer.rs:115-133`）が
   `IME input method changed: romaji → kana` を出しつつ
   `Some(InputModeState::ObservedKana)` を返す。
2. `classify_ime_snapshot` がそれを `ImeUpdate::new_input_mode` に載せる。
3. `PlatformState::apply_ime_update`（`state/platform_state.rs:1181-1191`）が
   `ImeEvent::InputModeObserved { confidence: Medium }` を dispatch。
4. `.claude/rules/ime-belief-architecture.md` の規約どおり、`reduce()` は
   **Medium 以上で `input_mode` を上書きする**。→ belief が `ObservedKana` になる。

### 実害（belief が `ObservedKana` だと何が変わるか）

- `decide_needs_romaji_pre_write`（`state/ime_actuation_decision.rs:189-201`）が
  **false** を返す → IME ON 時の **ROMAN 補完（`IMC_SETCONVERSIONMODE`）を送らなくなる**。
- `decide_dispatch_conv_after_open`（同 `:210-217`）も **`ConvAfterOpenId::Skip`**。

どちらも「ユーザーが意図的にかな入力を選んでいる状態を上書きしない」という
**正当な保護**であり、条件式そのものは正しい。壊れているのは入力側——
**awase 自身のトレイウィンドウから読んだ値が「ユーザーの意図」として扱われている**こと。

> ユーザーが「かなになった」と気づいてトレイアイコンをクリックする
> → トレイの IMM 値が `ObservedKana` として belief に入る
> → awase の ROMAN 補完が止まる
> → 直らない

これは**証明ではなく仮説**だが、BUG-106 追補2 の「1〜2 日間ブラウザを閉じても
直らない」という報告と噛み合う道筋であり、しかも安く潰せる。

### 構造的な非対称性（同じ関数の中にある）

`apply_ime_update` の中で、

- **open 軸**（`:1150-1157`）は `AcceptedObservation` を要求する。
  `state/probe_admission.rs:170-190` のとおり、これは private フィールドで
  「admission を通らない write をコンパイラで防ぐ」ための証明トークン。
- **input_mode 軸**（`:1181-1191`）は **`accepted` を一切参照しない**。

これは記憶にある「`shadow_on` と同型の未統合ペア」と同じ形の穴で、
BUG-106 追補3 が「次の一手」として書いた内容と完全に一致する。

### 修正方針（ファイル・関数レベル）

1. **`ImeSnapshot` に読み取り対象のクラス名を載せる。**
   `read_ime_state_full`（`ime.rs:681-695`）は**既に** `focused_hwnd` の
   `get_class_name_string` を計算していて、`is_tsf_native_window` 判定に使った後
   捨てている。`ImeSnapshot { focused_class: Option<String>, .. }` を足すだけで、
   新しい Win32 呼び出しはゼロ。
2. **`classify_ime_snapshot`（純粋関数）で採否を決める。**
   `focused_class` が
   - `AppImeProfile::from_class_name` で `Imm32Unavailable`（Chrome/Edge/Teams/UWP）、または
   - `tray::WINDOW_CLASS_NAME`（`"awase_tray_window"`）等 awase 自身の窓、または
   - 追跡中のフォーカスクラスと不一致

   のいずれかなら `new_input_mode = None`（**前回値を維持**。`ImeSnapshot` の
   doc が言う「`None` は偽ではなく不明」の意味論そのまま）。
   ついでに `input_mode_from_romaji_flag` の `tracing::info!` に
   `focused_class` を含めれば、依頼にあった「まず `fg_class` をログに出すだけ」の
   軽い案も同時に満たされる。
3. **`is_romaji` 経路だけを絞ること。** `ime_on`（open 軸）は既に
   `AcceptedObservation` と `derive_open()` の confidence ガードで守られており、
   そちらの挙動は変えない。

### なぜこれが安いか（規約適合）

`classify_ime_snapshot` は `#[must_use]` の**純粋関数**で、
`observer/ime_observer.rs` 末尾に既存のユニットテスト群（ケース 1〜）がある。
**Linux ホストターゲットで走る**（`cargo test --lib`）。
`.claude/rules/fix-requires-evidence.md` の「conv mode」「IME belief」
両ファミリーに該当する fix だが、(a) 回帰テストで満たせる——
`docs/known-bugs/` への散文追記を増やさずに済む。

---

## §5 ドラフトの未決事項 4 点への回答

1. **`focus_kind == TextInput` ゲートで 25 件を排除できるか** → **できない保証がある**
   （§1-1 の穴B）。`focus_kind` は「最後に probe が成功した時点」の値で、
   probe が軒並みタイムアウトしている状況＝まさに 25 件の状況では、
   直前の実アプリ（Chrome）の `TextInput` が残る。
   ドラフトが挙げた「設定画面のテキストボックスで `TextInput` 誤判定」も
   理論上ありうるが（`awase-settings` は egui＝独自描画なので MSAA が
   テキストロールを返す可能性は低い）、それより穴B の方が桁違いに起きやすい。
   → **ゲートを `focus_kind` に置かず、`GetForegroundWindow()` のクラス名を
   その場で読む**（§3）。ついでに「読んだ窓のクラス名」をログに載せられるので、
   ゲートが正しかったかを後から検証できる（`focus_kind` ゲートは
   「なぜスキップしたか」がログに残らない）。
2. **in-flight 管理と既存 probe との資源競合** → **推奨設計では論点が消える**
   （`GetKeyState` は µs オーダー、スレッドを起こさない）。
   どうしても非同期 probe を残すなら、`Option<u64>`（開始 tick）＋自己回復
   しきい値という `idle_conv_check_in_flight_since_ms` のイディオム
   （`state/platform_state.rs:1635,1677`）をそのまま流用すること。
   加えて §1-6 のとおり、`LEAKED_THREADS` 上限 8 を本番 probe と食い合う点を
   必ず設計に書くこと。
3. **記録先** → **`Runtime` の平フィールドでよい。新しい struct は作らない。**
   直接の前例が同じ構造体にある（`runtime/mod.rs:330` の `kana_lock_hysteresis`）。
   ADR-164 との整合の理解（メインスレッド専有なので atomic 不要）は正しい。
   **設定リロード時のリセット（`runtime/mod.rs:706`）への追加を忘れないこと。**
4. **journal への構造化記録** → **今は不要、`tracing::warn!` 1 行でよい。**
   理由は 3 つ。(a) journal はリングで、`DumpTruncated` に埋もれて誤った根拠を
   与えた前例がある（不具合報告では `awase.log.txt` の方が確実に残る）。
   (b) 3 秒周期の診断を journal に流すと、本来記録したい `KeyInput` /
   `ImeEvent` を押し出す。(c) **機械的に解析したいのは §4 の方**であり、
   そちらは `JournalEntry::ImeEvent`（`journal.rs:227-229`）が
   `InputModeObserved` を**既に型として**記録しているので新 variant 不要。
   §4 の修正で `focused_class` を足すなら、`ImeEvent` ではなく
   `ImeUpdate` 側のログ文言に載せるのが最小。

---

## §6 設計全体への批判

- **3 制約の充足**: (1) edge は OK。(2) belief 非書き込みはドラフトの意図としては
  正しいが、**`kana_lock_hysteresis` への投入も「書き込み」に数えるべき**
  （トレイ警告 UI を誤って出す）ことが明示されていない。
  (3) `run_with_timeout` 越し、は**満たしていない**（§1-3、実在するのは
  タイムアウト無しの `offload`）。
- **新しい一貫性の穴を作らないか**: 新 `Mutex`/`static` は増えない。ADR-164 の
  原則との整合の理解は正しい。ただしドラフトの非同期案は、`static` の代わりに
  **上限のないスレッド spawn** という別種の資源穴を作る（§1-3、§1-6）。
  「ロックを増やさない」だけでは ADR-164 の趣旨を満たしたことにならない。
- **費用対効果**: ドラフト案は「非同期タスク＋in-flight ゲート＋完了ハンドラの
  借用作法＋資源競合の検討」を必要とし、その見返りが **§2 のとおり対象アプリで
  空振りする値**である。`.claude/rules/complexity-budget.md` は未発効だが、
  ADR-158 の北極星（削減側に報酬を置く）の趣旨からして、この交換は割に合わない。
  §3 の推奨設計は既存関数 2 本の呼び出し＋フィールド 1 本で、
  「まず `fg_class` を出すだけ」の軽い案とほぼ同コストで、値まで取れる。
- **観測の順序**: P0（§4）を先にやると、25 件のノイズが消えた後のログで
  「本当に実アプリ上で反転したのか」が初めて分離できる。P1' を先にやると、
  ノイズ混じりのログの上にもう一系統の観測を重ねることになり、
  次の不具合報告の読み解きが逆に難しくなる。**順序は P0 → P1'。**

---

## §7 実施時の規約チェック

- **ブランチ**: `.claude/rules/main-develop-branch-flow.md` により `develop` の
  先端から。現在の作業ツリーは `fix/hook-diagnostic-repost-and-lock-alloc`
  （前回レビューの P2/P3 相当）なので、§4 は**別ブランチ**にすること
  （§4 は挙動変更を含む fix であり、診断ログの修正と混ぜない）。
- **`fix-requires-evidence`**: §4 は `observer/ime_observer.rs`（IME belief /
  conv mode ファミリー）に該当 → (a) `classify_ime_snapshot` の
  ユニットテスト追加で満たす。§3 は挙動を変えない診断追加だが、
  BUG-106 追補として 3〜5 行で記録するのが望ましい（追補3 の「次の一手」に
  対する回答になるため）。
- **`tuning-constants`**: 新しい `_MS` 定数を足さないこと。watchdog の
  既存 3 秒周期・5000ms しきい値を流用すれば実測義務は発生しない。
- **`experiment-logging`**: どちらも revert ではないので対象外。ただし §4 は
  `state/ime_actuation_decision.rs` の挙動（ROMAN 補完の発火頻度）を
  実質的に変えるので、コミット本文に「どのクラスで採用しないことにしたか」を
  必ず書くこと。

---

## §8 追加相談: 「この input_mode 観測は信用できるか」の判定に何を使うか

（2026-09-13、§4 の P0 を実装する直前の設計確認。読み取り専用調査の結果）

### §8.0 結論

**折衷案を採る。**

1. `ImeSnapshot` に **`focused_class: Option<String>` だけ**足す
   （新しい Win32 呼び出しはゼロ——`read_ime_state_full` が `ime.rs:681` で
   既に計算して捨てている文字列を保持するだけ）。
2. **`focused_process_name` は足さない。** `awase-settings.exe` の 1 件は、
   既に追跡済みの `FocusTracker::process_name()`（`focus/tracker.rs:72-74`）を
   判定の第2引数として使えば**追加 syscall ゼロで**排除できる（§8.4）。
3. **`current_app_profile()` を `classify_ime_snapshot` に渡す案は採らない。**
   理由は「stale かどうか」ではなく、**その判定が既に上流で行われており、
   ノイズ 25 件に対して情報量がゼロだから**（§8.2、これが今回一番重要な発見）。

### §8.1 Q1 の答え: `current_app_profile()` は「同期計算・非同期更新」の第3類型

段階を分けて答える。ドラフト相談の二者択一（「MSAA/UIA 依存」か
「WinEvent のたびに同期再計算」か）は、**どちらも正確ではない**。

- **計算そのものは純粋・同期。** `current_app_profile()`
  （`platform.rs:1745-1747`）→ `FocusTracker::current_profile()`
  （`focus/tracker.rs:76-78`）→ `CurrentFocus::app_profile` で、その値は
  `CurrentFocus::update_with_process_name`（`focus/current.rs:79-80`）が
  `AppImeProfile::from_class_and_process(class_name, process_name, relay_apps)`
  で計算する。**MSAA/UIA には一切依存しない**（`resolve_focus_kind` とは無関係）。
  → `focus_kind` の穴A（エンジン活性中に `Undetermined`）・穴C（cache hit）は
  **構造的に存在しない**。
- **しかし更新契機は非同期チェーンのみ。** `update_focus_info_with_process_name` の
  呼び出し元は `runtime/focus_tracking.rs:409` **1 箇所だけ**で、
  `advance_focus_tracking` ← `apply_focus_probe_result` ← `ir_stage_focus`
  ← `spawn_ime_refresh`（`runtime/mod.rs:757-765`、50ms デバウンス後）という
  非同期チェーンの中にある。
- **`EVENT_OBJECT_FOCUS` の同期経路は `app_profile` を更新しない。**
  `win_event_proc`（`app/bootstrap.rs:728-769`）→ `on_window_focus_event`
  （`runtime/mod.rs:1818-1843`）は、その場で `GetClassNameW` + `pid` を読むが、
  更新するのは **`injection_mode` だけ**（`update_injection_mode`、`:1838`）で、
  `CurrentFocus`/`app_profile` には触れない。最後に
  `schedule_ime_refresh(debounce_ms)`（`:1884`）でデバウンスを張るだけ。
- **したがって穴Bの弱い版は残る。** `classify_focus_probe`
  （`focus_tracking.rs:258-263`）が `probe == None` または `process_id == 0` で
  early return すると `app_profile` は前回値のまま。`run_focus_probe_async`
  （`focus/probe.rs:23-44`）は内部の `get_gui_thread_info_with_timeout(150ms)` が
  タイムアウトすると `process_id == 0` を返すので、**フォアグラウンドがハングして
  いる状況では `app_profile` も更新されない**。

**ただし `classify_ime_snapshot` の呼び出し地点に限れば、この穴Bは効かない。**
`ir_execute`（`runtime/ime_refresh.rs:63-83`）は
`ir_stage_focus`（:68、ここで `app_profile` を更新）→ `ir_stage_strategy`（:80）
→ `ir_stage_observe`（:81、ここで snapshot を分類）という**固定順序**で、
しかも `focus` と `ime` は `spawn_ime_refresh` の同一タスク内で連続して
await された同じサイクルの値である。**profile が更新されなかったサイクルでは
snapshot も同じ理由で怪しい**ので、両者の鮮度は構造的に連動している。

→ Q1 の答え: **「MSAA/UIA には依存しない。更新は非同期チェーン経由なので
一般には lag しうるが、`classify_ime_snapshot` の呼び出し地点では
同一サイクルの値であることが `ir_execute` の順序で保証されている。」**

### §8.2 Q2 の答え: 渡してはいけない。**その判定は既に上流で行われている**

これが今回の調査の核心で、代替案を採らない理由は「stale の危険」ではない。
**`current_app_profile()` による IMM 信用判定は、`classify_ime_snapshot` に
到達する手前で既に完了している。**

```
ir_execute (ime_refresh.rs:63)
  ├ ir_resolve_skip_imm_query (:239-241) = !can_use_imm32_cross_process()
  ├ ir_decide_read_strategy   (:296-)     → SkipTyping / Blacklist / OsPoll
  └ ir_stage_observe          (:146-219)
       ├ SkipTyping → ime_snap を使わない
       ├ Blacklist  → ime_snap を使わない（GJI I/O 観測のみ）
       └ OsPoll     → ir_poll_and_learn(.., ime_snap)  ← ここだけが classify する
```

`can_use_imm32_cross_process()`（`focus/class_names.rs:165-170`）は
`Standard` のみ `true`、`Imm32Unavailable | TsfNative | InputRelay` は `false`。
つまり **Chrome / Edge / Teams / UWP / WezTerm の snapshot は、
`classify_ime_snapshot` に**到達しない**（`Blacklist` 分岐で捨てられる）。

そして `classify_fetched_snapshot`（もう一方の入口）の呼び出し元 2 箇所も、
どちらも `matches!(self.platform.current_app_profile(), AppImeProfile::Standard)`
で明示的にガードされている（`runtime/key_pipeline.rs:3048-3051`、
`runtime/focus_tracking.rs:762-765`。なお後者は `write_imm_cross_probe`（open 軸）
のみで input_mode は書かない）。

→ **`current_app_profile()` を classify の引数として渡しても、`Standard` 以外は
そもそも来ないので分岐が死ぬ。** ノイズ 25 件の発生源である
`awase_tray_window` / `awase-settings.exe` は**どちらも `Standard`**
（`IMM32_UNAVAILABLE_CLASSES`（`class_names.rs:19-35`）にも
`is_tsf_native_window`（同 `:51-61`）にも載っていない）なので、
**profile 引数は 25 件のうち 1 件も排除できない**。

これは「穴Bと同型の罠を踏むか」という心配より悪い——**罠に落ちるのではなく、
何もしないコードが1本増える**。ADR-158 の北極星（加算より減算）から見ても
採ってはいけない形である。

**必要な識別子は「この `Standard` ウィンドウは awase 自身の UI か」であり、
それは profile という語彙では表現できない。**

### §8.3 Q3 の答え: `ImeSnapshot` に足すのは `focused_class` **だけ**

`focused_class` の追加コストは実質ゼロである。`read_ime_state_full`
（`ime.rs:681-695`）は既に

```rust
let class = crate::focus::classify::get_class_name_string(focused_hwnd);
tracing::debug!("read_ime_state_full: focused_hwnd={focused_hwnd:?} class={class:?}");
if is_tsf_native_window(&class) { return ImeSnapshot { .. }; }
```

を実行しており、`class` を `ImeSnapshot` に載せるだけ。
**新しい Win32 呼び出しはゼロ**、`String` 1 本のムーブが増えるだけ。
`ImeSnapshot` は `Option<T>` 一貫の 3 値意味論（`Some`=成功 / `None`=不明）を
doc で宣言しているので、`focused_class: Option<String>`（null hwnd 時 `None`）は
既存の型規約にそのまま乗る。早期 return する TSF-native 分岐でも
`focused_class: Some(class)` を返せる（デバッグ価値がある）。

一方 `focused_process_name` の追加は**避けるべき**。理由は「syscall が重い」
だけではない:

1. `focus/classify.rs:211-245 get_process_name` は
   `OpenProcess` + `QueryFullProcessImageNameW` + `CloseHandle` の 3 API で、
   ハンドルを開く。**500ms ポーリングのたび**に実行されることになる
   （現状これはフォーカス変更のたび 1 回、`classify_focus_probe` の
   `imm_learning` クロージャ経由、`focus_tracking.rs:274-288`）。
2. より本質的に、**同じ情報を 2 箇所で別々に取得する SSOT 重複になる**。
   `CurrentFocus::process_name`（小文字化済み）が既に存在し、
   `ir_stage_focus` が `ir_stage_observe` の直前に更新している（§8.1）。
   そこから読めば済むものを、`ime.rs` 側でもう一度 OS に聞き直す形は、
   このリポジトリが繰り返し踏んできた「同じ値の供給元が 2 つに分かれ、
   片方だけが古くなる」パターンそのもの
   （`.claude/rules/fix-requires-evidence.md` の `shadow_on` 行が記録している型）。

### §8.4 推奨する実装形（折衷案）

**判定関数は `focus/class_names.rs` に置く。** このモジュールの doc が
「`classify.rs`・`ime.rs`・`focus_observer.rs` で重複していたクラス名リストと
判定ロジックを一元管理する」と宣言しており、置き場所として既に正しい。

```rust
/// awase 自身の UI ウィンドウ（トレイ／設定画面）か。
///
/// ここから読んだ IMM の conv/romaji 値は「ユーザーが編集しているアプリの
/// 入力方式」ではないため、input_mode belief に採用してはならない（BUG-106 追補3）。
///
/// `process_name` は `FocusTracker::process_name()`（小文字）を渡すこと——
/// awase-settings は winit 既定の "Window Class" という一般的なクラス名を使うため、
/// クラス名だけでは無関係な winit アプリと区別できない。
pub fn is_own_ui_window(class_name: &str, process_name: &str) -> bool {
    class_name == crate::tray::WINDOW_CLASS_NAME      // "awase_tray_window"
        || process_name == "awase-settings.exe"
}
```

**呼び出し地点は `classify_ime_snapshot` の外**（純粋関数の引数として `bool` を
渡す）。`.claude/rules/ime-belief-architecture.md` の「Observe → pure
`classify_*` → `reduce()`」を守るため、`classify_*` の中で Win32 も
グローバル状態も見ないこと。既存シグネチャに 1 引数足す形になる:

```rust
pub fn classify_ime_snapshot(
    snap: &ImeSnapshot,
    now_ms: u64,
    current_ime_on: bool,
    current_force_on_guard_active: bool,
    current_input_mode: InputModeState,
    current_prev_conversion_mode: Option<u32>,
    trust_input_mode: bool,   // ← 追加（= !is_own_ui_window(..)）
) -> ImeUpdate
```

呼び出し元は 2 箇所:
`observer/ime_observer.rs::poll_and_classify_ime`（同期版）と
`classify_fetched_snapshot`。両方で
`!is_own_ui_window(snap.focused_class.as_deref().unwrap_or(""), platform.focus.process_name())`
を渡す。

#### 実装上の必須の細部（ここを外すとノイズが1 poll ずれるだけで消えない）

`trust_input_mode == false` のとき、**`new_input_mode` に加えて
`new_prev_conversion_mode` も `None` にすること。**
`apply_ime_update`（`state/platform_state.rs:1192-1194`）は
`new_prev_conversion_mode` を**無条件で** `belief.prev_conversion_mode` に書く。
ここにトレイの conv 値が入ると、次の「本物のアプリ」での poll が
`input_mode_from_conversion`（`observer/ime_observer.rs:135-153`）で
`classify_transition(prev=トレイのconv, curr=実アプリのconv)` を評価し、
**偽の遷移**を作る。採用しないと決めたサイクルは
`ImeSnapshot` の doc が言う「`None` は偽ではなく不明、observer はキャッシュ値を
維持する」に従って何も書かないのが正しい。

#### スコープを広げないこと

`ime_on`（open 軸）の扱いは**今回変えない**。open 軸は
`AcceptedObservation`（`state/probe_admission.rs:170-190`）と
`derive_open()` の confidence ガードで既に守られており、ここを同時に触ると
BUG-07 型（偽の Low false が `most_recent_trusted()` を支配して Engine OFF）の
再燃リスクを負う。**今回の実害として道筋が引けているのは input_mode 軸だけ**
（§4 の `ObservedKana` → `decide_needs_romaji_pre_write` が false）。
ただし `focused_class` をログ（`input_mode_from_romaji_flag` の
`tracing::info!`、`ime_observer.rs:121-126`）に載せておけば、
次の報告で open 軸も同じ汚染をしているかを追加実装なしで判定できる。

### §8.5 Q4 の答え: `awase-settings.exe` の 1 件は**排除する**。ただし syscall は足さない

「25 件中 1 件のために複雑さを足す価値があるか」という問いの前提が、
§8.4 の形では成り立たない——**追加コストは `is_own_ui_window` の第2引数と
`||` の 1 項だけ**で、新しいフィールドも syscall も増えない。
複雑さの単位で言えば「1 件のために足す」のではなく「判定関数を最初から
2 引数で書く」だけである。

逆に、クラス名だけで済ませて `awase-settings.exe` を既知ギャップとして
文書化する案は割に合わない。理由:

- **awase-settings は「かなになった」と気づいたユーザーが最も開きやすい窓**
  である（BUG-106 追補3 が推測している「トラブルシューティングのつもりで
  トレイアイコンをクリックした」の続きの動作そのもの）。**残す 1 件が、
  実害シナリオの中心に位置している。**
- 既知ギャップとして `docs/known-bugs/BUG-106.md` に追補を書くコストの方が、
  `||` 1 項より高い（30 行ルールの枠も消費する）。

一方で、**`focused_process_name` を `ImeSnapshot` に足してまで**排除するのは
やりすぎ（§8.3）。「排除はする、ただし追加の OS 問い合わせはしない」が答え。

### §8.6 残存リスクと、それを検証可能にしておく方法

1. **`snapshot` の窓と `tracker` の窓がずれるケース。** `focused_class` は
   `read_ime_state_full` が独自に `GetGUIThreadInfo` で解決した hwnd 由来、
   `process_name` は `run_focus_probe_async` が解決した hwnd 由来で、
   2 つの await の間にフォーカスが動くとずれる。`is_own_ui_window` は
   OR 判定なのでこのずれは**過剰排除の方向**にしか効かない
   （どちらかが awase の窓なら採用しない）。observation を 1 サイクル
   捨てるだけなので安全側。
2. **将来 awase が別のクラス名の窓を増やしたとき**に漏れる。
   `tray::WINDOW_CLASS_NAME` は定数参照なので追従するが、
   `"awase-settings.exe"` はリテラルになる。`tray.rs:132-138` の doc が
   既に「awase-settings 側がこの文字列を直書きで参照している」という
   同型の結合を記録しているので、`is_own_ui_window` の doc に
   同じ注意書きを添えること。
3. **修正が効いたことの確認方法**: `input_mode_from_romaji_flag` の
   `tracing::info!` に `focused_class` を含めておけば、次の不具合報告で
   「`IME input method changed` が残っているか、残っているならどの窓か」を
   grep 一発で判定できる。これが §4 の (a) 回帰テストと対になる実機側の証拠。

### §8.7 テストと規約

- `classify_ime_snapshot` は純粋関数で、`observer/ime_observer.rs` 末尾に
  既存のユニットテスト群がある（Linux ホストで `cargo test --lib`）。
  `trust_input_mode = false` のとき `new_input_mode` と
  `new_prev_conversion_mode` が**両方** `None` になることを固定するテストを
  1 本足せば `.claude/rules/fix-requires-evidence.md` の (a) を満たす。
- `is_own_ui_window` 自体も `class_names.rs` の既存テストに 3 ケース
  （tray / settings / 無関係な winit アプリの "Window Class"）で足せる。
  **3 件目（無関係な winit アプリを排除しないこと）が本質**——
  過剰排除の回帰を防ぐのはこのテストだけ。
- `classify_ime_snapshot` のシグネチャ変更は `crates/awase-windows/tests/` の
  golden には影響しないが、`architecture_guard.rs` が
  `InputModeObserved` の構築箇所数を固定している点に注意
  （構築箇所は増えないので抵触しないはずだが、実装後に
  `cargo test -p awase-windows --test architecture_guard` を回すこと）。

---

## §9 P1' の最終設計確認（P0 実装後、実装委譲の直前）

（2026-09-13、P0 = `ac4d4ab1` `fix/bug106-tray-input-mode-belief-poisoning` /
PR #211 を読んだうえでの再確認。読み取り専用調査）

### §9.0 結論

**§3 の設計は有効。ただし 1 点を訂正し、1 点を追加する。**

- **訂正**: §3 が「設定リロード時のリセット（`runtime/mod.rs:706`）にも足すこと」と
  書いたのは**誤り**。`:706` は設定リロードではなく**エンジン無効化**の分岐であり、
  そもそも**この新フィールドはどこでもリセットしてはいけない**（§9.2-A）。
- **追加**: `is_own_ui_window` は **gate ではなく label として使う**（§9.1）。
  絞らない。ただしログ行に `own_ui=true/false` を必ず出す。

### §9.1 Q1 の答え: 絞らない。`is_own_ui_window` は**タグ**として使う

**結論: 記録対象を絞らない。ただし `is_own_ui_window` の結果をログに出す。**

理由は「edge のキーに何を使うか」を分けて考えると明確になる。

- **edge の比較キーは `KanaLockReading` だけにする。** `fg_class` は比較に
  含めない（＝ペイロード扱い）。`GetKeyState(VK_KANA)&1` が返すのは
  **awase のスレッド入力キューが持つトグルビット**であり、どのウィンドウが
  フォアグラウンドかで値が決まるものではない。クラス名を比較キーに混ぜると、
  値が変わっていないのにフォーカス移動のたびに行が出る（＝レベルトリガに近づく）。
- **`fg_class` はペイロードとして毎回出す。** 「反転が起きた瞬間にどの窓に
  いたか」が P1' の目的そのものなので、reading が変化した行に同梱すれば
  「トレイクリック直後に反転していた」という情報は**失われない**
  （相談で懸念されていた点は、絞らないことではなく edge キーの設計で解決する）。
- **`is_own_ui_window(fg_class, tracked_process_name)` を gate にしない。**
  P0 でこれを gate にしたのは、あちらが **belief を書く**経路だったから
  （汚染したら実害が出る）。P1' は**何も書かない診断**であり、
  gate にすると「トレイにフォーカスがある間に反転した」という観測を
  収集時点で捨ててしまう——これは BUG-106 追補3 が
  「実アプリ上での反転を一件も観測できていない」と結論した状況を、
  別の軸で再生産する。
- **代わりにタグを出す。** `own_ui=true` をログに含めれば、
  解析時に `grep -v own_ui=true` で機械的に分離できる。
  **収集は広く、解釈は後で絞る**——診断チャネルの正しい設計であり、
  belief 経路（P0）とは逆の判断になるのが正しい。

タグ用の `process_name` は `self.platform.focus.process_name()`
（`focus/tracker.rs:72-74`、追跡済み・小文字化済み）を使う。**新しい syscall を
足さないこと**（P0 の §8.3 と同じ理由）。`fg_class` は
`foreground_class_name()` の**新鮮な値**、`process_name` は**追跡値**という
出所の違いが残るが、`is_own_ui_window` は OR 判定なので過剰タグ方向にしか
効かず、タグ（gate ではない）なので実害はない。**この出所の混在は
doc コメントに 1 行書いておくこと**。

### §9.2 Q2 の答え: 落とし穴 6 点

#### A. リセットは**一切しない**（§3 の訂正）

`kana_lock_hysteresis` は 3 箇所でリセットされる——
フォーカス変更（`runtime/ime_refresh.rs:122`）、エンジン無効化
（`runtime/mod.rs:706`）、初期化（`:1543`）。**新フィールドを同じ場所に
足してはいけない。**

- `kana_lock_hysteresis` がリセットされるのは、それが**トレイに表示される
  ユーザー可視の警告状態**（On 3 連続で警告、フォーカスが変われば
  切り替え先で検知し直す必要がある）だから。
- 新フィールドは**ログの重複を抑えるためだけの前回値メモ**で、
  ユーザー可視の状態を持たない。リセットすると次の watchdog で
  同じ値がもう一度 edge 扱いされ、**ログ行が増えるだけで情報は増えない**。
- 初期化（`Runtime` の構造体リテラル、`runtime/mod.rs:1536-1546` 付近）だけは
  必要だが、これはコンパイラが強制するので漏れようがない。

#### B. `kana_lock_hysteresis` に**投入しない**（§3 の再掲、最重要）

`KanaLockHysteresis::observe()` は「On 3 連続で警告、Off 2 連続で解除」
（`src/engine/kana_input_warn.rs`）で、**1 打鍵 1 サンプル**を前提に較正されている
（`kp_stage_kana_lock_warn`、`runtime/key_pipeline.rs:2703-2740`）。
そこに 3 秒周期の watchdog サンプルを混ぜると、**ユーザーが一度も打鍵して
いないのに 9 秒でトレイ警告が出る**。BUG-106 追補1 が「このノイズで
`kana_input_warn` が誤発火しないか」を次回調査対象に挙げていた、まさにその事故を
自分で作ることになる。**新フィールドは `kana_lock_hysteresis` と完全に独立**。

#### C. フィールド名を紛らわしくしない

`kana_lock_hysteresis` の隣（`runtime/mod.rs:329-330`）に置くことになるので、
`kana_lock_*` で始まる名前は避ける。`watchdog_kana_edge:
Option<KanaLockReading>` 等、**watchdog 由来であることが名前から分かる形**にし、
doc コメントに「`kana_lock_hysteresis` には投入しない（B の理由）」を明記する。

#### D. `read_kana_lock()` は `Unknown` を返さない

`KanaLockReading` の enum は `Off`/`On`/`Unknown` の 3 値だが、
`observer/kana_lock.rs:10-21` の実装は `is_toggle_key_on()` の `bool` から
`On`/`Off` のどちらかしか返さない。**`Unknown` に到達する match 腕を書いて
「取得失敗時はこうする」というコメントを付けない**こと（実在しない分岐の
ための説明はコードを誤読させる）。「まだ一度もサンプルしていない」は
`Option` の `None` で表現する。

#### E. 既存の `5000` リテラルを二重に書かない

現在の watchdog 分岐（`runtime/message_handlers.rs:648-682`）は
`os_idle_ms < 5000` を**フォーマット文字列の中の `if` 式**として書いている。
サンプル採取の条件も同じ `os_idle_ms < 5000` なので、
`let hook_starved = os_idle_ms < 5000;` を先に束縛して**両方でそれを使う**こと。
リテラルを 2 箇所に増やすと、片方だけ直す退行の温床になる
（`.claude/rules/tuning-constants.md` が対象にしている「同じ役割の定数の
段階的釣り上げ」の芽）。**新しい `tuning.rs` 定数は作らない**
（実測義務が発生し、かつ watchdog の既存周期を流用すれば足りる）。

#### F. `None`（`GetLastInputInfo` 取得失敗）の腕には入れない

`os_last_input_tick_ms()` が `None` を返す腕では `os_idle_ms` が求まらず、
「フックにイベントが届いていない疑い」かどうかを判定できない。
ここではサンプルしない（edge メモも更新しない）。

#### G. 自動テストは付かない（承知のうえで進める）

`runtime/` は `#[cfg(windows)]` 配下（CLAUDE.md 記載）なので、
この変更に Linux で走る回帰テストは付けられない。
`.claude/rules/fix-requires-evidence.md` の (a) が使えないため、
**(b) `docs/known-bugs/BUG-106.md` への追補**（何を記録するようにしたか、
次の報告でどう読むか、3〜5 行）で満たすこと。検証は
`cargo check --target x86_64-pc-windows-msvc -p awase-windows` +
`cargo clippy --target x86_64-pc-windows-msvc -p awase -- -A clippy::cargo`。

### §9.3 Q3: 実装者（Codex CLI）への必須指示（5 行）

1. **`runtime/message_handlers.rs` の `TIMER_HOOK_WATCHDOG` 分岐のみを触る。**
   `Some(os_last_input)` の腕で `let hook_starved = os_idle_ms < 5000;` を束縛し、
   既存のフォーマット文字列の `if` もそれを使う形に直したうえで、
   `hook_starved` が真のときだけサンプルする（`None` の腕では何もしない）。
2. **読むのは既存関数 2 本だけ**——`observer::kana_lock::read_kana_lock()` と
   `observer::kana_lock::foreground_class_name()`（どちらも `unsafe fn`、
   SAFETY コメントに「WM_TIMER ハンドラはメッセージループスレッド上」と書く）。
   **IMM/TSF/`run_with_timeout`/`spawn_local`/ワーカースレッドは一切使わない。**
3. **`Runtime` にプレーンフィールドを 1 本足す**（`Option<KanaLockReading>`、
   `kana_lock_hysteresis` の隣、名前は `kana_lock_` で始めない）。
   **初期化のみ行い、リセットはどこにも足さない。**
   **`kana_lock_hysteresis` には絶対に投入しない**（トレイ警告が誤発火する）。
4. **edge の比較キーは reading だけ。** 前回値と同じなら何も出さない。
   変化したときだけ `tracing::warn!` で
   `prev → now` / `fg_class` / `own_ui=<is_own_ui_window(fg_class,
   platform.focus.process_name())>` / `stale_ms` / `os_idle_ms` を 1 行に出す。
5. **belief に触れない。** `ImeModel`/`dispatch_event`/`apply_*`/`platform_state.ime`
   への書き込みを一切含めないこと。新しい `static`/`Mutex`/`Atomic`/
   `tuning.rs` 定数も追加しない。差分は 30 行未満に収まるはず。

### §9.4 差分レビュー時に見る点（先に宣言しておく）

受け取った差分で以下を確認する: (a) `kana_lock_hysteresis.observe(..)` の
新しい呼び出しが**無い**こと、(b) 新フィールドのリセットが
`ime_refresh.rs:122` / `runtime/mod.rs:706` に**増えていない**こと、
(c) `5000` リテラルが増えていないこと、(d) `spawn_local` /
`run_with_timeout` / `offload` の呼び出しが**無い**こと、
(e) `platform_state.ime` への書き込みが**無い**こと。
