# awase ワークアラウンド一覧

> 最終調査: 2026-06-04

コードベースに存在する workaround・timing hack の一覧と、各項目が依然として必要か否かの判定を記録する。

---

## 調査方針

各ワークアラウンドについて以下を確認した。

1. `git log -p` で導入コミットと経緯を確認
2. その後の設計変更（spawn_local 化・executor リファクタ等）で前提が崩れていないか確認
3. 現在のコール経路で依然として必要かを判定

**判定凡例:**
- **削除不可** — 現在も必要なロジック
- **ワークアラウンドではない** — 意図的な設計方針、または正確な SAFETY 記述
- **対処済み** — 調査の結果、改善・更新を実施

---

## カテゴリ 1: タイミング・Sleep 系

### 1-1. BUG-06 派生タイムスタンプ再計算

**場所:** `crates/awase-windows/src/output/mod.rs:153-157`

**内容:** `mark_composition_cold(NativeF2Consumed)` で `eager_warmup_sent_ms` もリセットする理由の説明コメント。  
物理 F2 が押されると WezTerm が TSF を再初期化するため、フォーカス変更時刻を `elapsed` の起点に使うと「古い warmup からの経過時間」を誤って計算し、TSF 未初期化のまま romaji を送信してしまう（「hoんらい」化け）。

**判定: 削除不可**  
WezTerm 系 cold start バグは直近まで継続修正中（`8b90725`）。タイムスタンプ起点の正しい管理はいまも必要。

---

### 1-2. GJI 再起動後の待機

**場所:** `crates/awase-windows/src/gji.rs:218-219`

**内容:** GJI プロセスを `TerminateProcess` で終了させた後、再起動前に待機する処理。

**経緯:** 元々は固定 500ms `thread::sleep` で待っていた。`TerminateProcess` は非同期のため、sleep が短すぎると旧プロセスが死にきっていない状態で新プロセスを起動してしまう可能性があった。

**対処済み (`2f4c766`):**  
`PROCESS_SYNCHRONIZE` 権限を追加し、`WaitForSingleObject(h, 5_000)` でプロセス消滅を確認してから再起動するよう変更。最大 5 秒の確実な待機に改善。

```rust
// before
std::thread::sleep(std::time::Duration::from_millis(500));

// after
let _ = WaitForSingleObject(h, 5_000);
```

---

### 1-3. TSF ポーリング `wait_until_ready`

**場所:** `crates/awase-windows/src/tsf/probe.rs:139,152`

**内容:** GJI 安定待ちを `block_on` ではなく `std::thread::sleep` 10ms ポーリングで実装。

**判定: 削除不可（テスト専用・設計済み）**  
コメントに「主にテストコードおよびフォールバックパスで使用する。本番の TSF プローブは `TIMER_TSF_PROBE + check_now` を使うこと」と明記されており、実際の呼び出しもすべてテスト内（`:583, :607, :631, :654`）に限定されている。  
`block_on` を避ける理由は「ネストされたメッセージループを起動しない」ため。テスト用途では引き続き妥当。

---

### 1-4. `TIMER_OUTPUT_GUARD` (ID=104)

**場所:** `crates/awase-windows/src/lib.rs:222-223`

**内容:** `block_on(sleep)` を排除するため、SendInput 後 50ms 経過を `SetTimer` で待機してから `drain_deferred` を再実行する仕組み。

**判定: 削除不可**  
`executor.rs` の `drain_deferred` / `OutputActiveGuard` / `enqueue_reinject` と連携した重要な機構。`message_handlers.rs` のタイマーハンドラから現役で使用されている。

---

## カテゴリ 2: IME-OFF 救済機構 (Ctrl+無変換)

### 2-1. `TIMER_IME_OFF_RESCUE` (ID=107) と Phase A/B ロジック

**場所:**
- `crates/awase-windows/src/lib.rs:215`（定数定義）
- `crates/awase-windows/src/hook.rs:396`（親指キー除外）
- `crates/awase-windows/src/runtime/key_pipeline.rs:51-77`（Phase A/B ハンドリング）

**内容:** LINE 等での Ctrl+無変換 IME-OFF を実現する機構。「Ctrl 押しながら他キーを誤打した後に無変換を押した」場合と「Ctrl+無変換を意図的に押した」場合を 50ms のウィンドウで区別する。

**判定: 削除不可（ワークアラウンドではなく正規機能）**  
`ba5e4d3` で thumb shift 化けバグを修正した後の正しい実装。`runtime/mod.rs` から現役で使用。  
親指キー除外（`hook.rs:396`）は「Ctrl+無変換を直接押したとき(他キーなし) rescue が誤発動しない」ための意図的な設計。

---

## カテゴリ 3: VK 送信系

### 3-1. 同一 VK 連続バッチ分割

**場所:** `crates/awase-windows/src/output/vk_send.rs:299-304`

**内容:** "nn" のように同一 VK が連続する場合、バッチを分割して別の `SendInput` 呼び出しで送る。

**判定: 削除不可**  
Windows IME が `N↓N↓N↑N↑` を含む単一バッチの 2 番目の `N↓` をオートリピートと判定して破棄する実際の挙動に対応。削除すると "nn" → "n" になる。

---

### 3-2. `VK_OEM_MINUS` 後の composition warm 維持

**場所:** `crates/awase-windows/src/output/vk_send.rs:458-464`

**内容:** `VK_OEM_MINUS` (0xBD, no-shift) = 「ー」は GJI ローマ字モードで composition context に取り込まれる（context がリセットされない）。これを利用して warm 状態を維持し、直後の romaji を warmup sleep なしで即送信する。

**判定: 削除不可**  
GJI の composition 動作の実際の挙動に依存した最適化。削除すると「ー」後の入力がすべて cold start 扱いになり余分な warmup 待機が発生する。

---

## カテゴリ 4: IME 戦略フォールバック

### 4-1. `post_kanji_toggle_to_focused`（VK_KANJI フォールバック）

**場所:** `crates/awase-windows/src/ime.rs:156-175`

**内容:** GJI 非稼働時（MS-IME 等）の最終フォールバック。旧実装は候補ウィンドウ表示中に `Ctrl+Enter` で候補を確定してから `VK_KANJI` を送っていたが、Chrome フォームを submit させる副作用があり廃止（`1d7315e`）。現在は bare `VK_KANJI` のみを送り、候補ウィンドウへの吸われは許容する。

**判定: 削除不可（すでにクリーン）**  
コメントは「Ctrl+Enter 廃止済みの経緯」として適切。コードも現在の仕様を正確に反映している。

---

### 4-2. GJI 全プロファイル共通戦略

**場所:** `crates/awase-windows/src/ime_controller.rs:14-18`

**内容:** GJI 稼働中はアプリ種別（Standard / Imm32Unavailable / TsfNative）によらず F13/F14 を使い、`VK_KANJI` トグルアーティファクトを回避する設計方針。

**判定: ワークアラウンドではない**  
`4312706` で意図的に設計した正規方針。F13/F14 は IME 層で処理されフォアグラウンドアプリのプロファイルに依存しないため、GJI 稼働時は常に安全に使える。

---

## カテゴリ 5: 再入・スレッド安全系

### 5-1. `capture_imc` の `run_with_timeout` ワーカー分離

**場所:** `crates/awase-windows/src/ime_diagnostic.rs:215-219`

**内容:** `capture_imc`（クロスプロセス IMC クエリ）を `run_with_timeout` でワーカースレッドへ offload し、メインスレッドを `recv()` でブロックする。

**旧コメントの問題:** `38e74c2`（`in_with_app` 再入ガード削除）以降、「`with_app` 再入の回避」という説明が古くなっていた。実際の理由は「`RUNTIME` 借用中にメインスレッドがメッセージポンプを回すと、フックの `try_borrow_mut()` が失敗してキーがパススルーされる」こと。

**対処済み (`3306ff7`):**  
コメントを正確な理由に更新。`run_with_timeout` 自体は引き続き必要。

---

### 5-2. `CreateToolhelp32Snapshot` ハンドルの明示クローズ

**場所:** `crates/awase-windows/src/tsf/observer.rs:330-332`

**内容:** スナップショットハンドルをループ終了後に `CloseHandle` する際の SAFETY コメント。

**判定: ワークアラウンドではない**  
`observer.rs:372-419` の RAII Drop とは別のハンドルであり、明示クローズが必要な箇所に対する正確な SAFETY 記述。

---

## カテゴリ 6: アプリ固有バグ対応系

### 6-A. Ctrl↑ で `eager_warmup_sent_ms` をリセット

**場所:** `crates/awase-windows/src/runtime/executor.rs:467-479`

**内容:** Ctrl が WezTerm に届いている間、GJI TSF 初期化が中断される可能性がある。Ctrl↑ 後に composition が cold 状態であれば `eager_warmup_sent_ms` をリセットし、GJI recovery 時間（500ms）を Ctrl↑ 起点で再計測する。

**症状:** Ctrl を離した直後にひらがなを入力すると「この → kおの」になる。

**判定: 削除不可**（WezTerm × GJI の実際の挙動への対応）

---

### 6-B. KeyUp INJECTED_MARKER 対称性

**場所:** `crates/awase-windows/src/runtime/executor.rs:431-443`

**内容:** KeyDown を reinject（`INJECTED_MARKER` 付き）で送った場合、対応する KeyUp も reinject で揃える。WezTerm が `INJECTED↓ + physical↑` という非対称ペアを異常扱いする可能性を排除するため。

**判定: 削除不可**（WezTerm の INJECTED キーペア対称性要件への対応）

---

### 6-C. Shadow desync 時の EngineIntent 強制送信

**場所:** `crates/awase-windows/src/runtime/executor.rs:800-840`

**内容:** フォーカス変更直後や起動時に実 IME 状態が unknown になり、`applied_snapshot=None` のまま IME が ON になっていることがある。この状態で `KanjiToggle/GjiDirect` が「`shadow=desired` → スキップ」してしまい Ctrl+無変換 が効かなくなる。ユーザーの明示的操作（`EngineIntent`）では shadow desync を無視して必ず送信することで対処する。

スキップ判定は方向で異なる:
- `SetOpen(false)` 方向: `applied_at_ms > 0`（実 apply 確認済み）なら永続スキップ → 定常状態での VK_KANJI 二重送信防止
- `SetOpen(true)` 方向: 300ms ウィンドウ → KeyDown+KeyUp 二重送信防止

**判定: 削除不可**（ImmCross 非対応アプリ×フォーカス変更直後の状態不定への対応）

---

### 6-D. `spawn_ime_refresh` での eager warmup スキップ

**場所:** `crates/awase-windows/src/runtime/mod.rs:303-309`

**内容:** `focus_transition_pending=true` の時点では `injection_mode` が前ウィンドウ（WezTerm 等）の stale な `Tsf` のまま。このまま `send_eager_tsf_warmup()` を呼ぶと、フォーカス先が Chrome/Edge の場合に誤って `VK_DBE_HIRAGANA` を送信して Chrome の IME を ON にしてしまう。eager warmup は `run_with_prefetched` 内で `injection_mode` 確定後に送る。

**判定: 削除不可**（フォーカス遷移中の stale `injection_mode` による Chrome IME 誤 ON への対応）

---

### 6-E. 物理 KANJI キー後の `mirror_applied_open`

**場所:** `crates/awase-windows/src/runtime/mod.rs:498-504`

**内容:** 物理 KANJI キーは `apply_ime_open` を経由しないため `last_applied` が更新されない。このまま Engine が activate → `SetOpen(true)` → `KanjiToggleStrategy` が `last_applied(false) != desired(true)` と判定して VK_KANJI を余分に送信し、Chrome で IME が逆転する。`process_deferred_effects` 完了後に OS 観測値で `mirror_applied_open` を呼び同期する。

**判定: 削除不可**（物理 KANJI キーが `apply_ime_open` を迂回することへの対応）

---

### 6-F. `F2NonTsf` での programmatic F2 スキップ

**場所:** `crates/awase-windows/src/output/vk_send.rs:132-144`

**内容:** ユーザーが物理 F2 を押して Chrome の composition context が初期化済みの場合、プログラム的な F2 送信（`SendMessageTimeout + SendInput`）をスキップする。スキップしないと Chrome が F2 を 3 回受け取り composition がリセットされ「かんりのつごう → kaんりのつごう」になる。ただし物理 F2 から `F2_STALE_MS`（1200ms）経過後は context が失効している可能性があるため programmatic F2 を再送する。

**判定: 削除不可**（Chrome が F2 を重複受信すると composition をリセットする実際の挙動への対応）

---

### 6-G. KEYEVENTF_UNICODE + VK 混在によるリテラル化バグ回避

**場所:** `crates/awase-windows/src/output/probe_io.rs:162-165`

**内容:** `KEYEVENTF_UNICODE` 直後に VK ストロークを送ると、WezTerm/IME が N キーをリテラル `'n'` として扱い「のあたり → nおあたり」になる。deferred_vks がある場合は Unicode kana パスを使わず VK ローマ字パスで送ることで回避する。

**判定: 削除不可**（WezTerm + IME の Unicode/VK 混在送信時の実際の挙動への対応）

---

## 対処済みアクション一覧

| コミット | 内容 |
|---|---|
| `2f4c766` | `gji.rs`: `thread::sleep(500ms)` → `WaitForSingleObject(5000ms)` に置換 |
| `3306ff7` | `ime_diagnostic.rs`: `capture_imc` コメントを「RUNTIME 借用中のメッセージポンプ防止」に更新 |
