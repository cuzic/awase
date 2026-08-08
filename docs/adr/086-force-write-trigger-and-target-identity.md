# ADR-086: force-write の単一規律 — 「観測を信じない書き込み」のトリガー条件と書き込みターゲット同一性

## ステータス

**北極星仕様。ADR-084 の姉妹編（invariant 番号空間を共有する）。
Phase 0〜1（記録・INV-14 ターゲット同一性の全経路移行）は実装完了
（2026-08-08）。Phase 2〜4（トリガー条件の是正、open/close 軸への適用、
コンパイラ強制）は未着手。いずれも Windows 実機での動作確認は未実施。**

本 ADR は個別バグの修正手順書ではなく、**以後の実装判断を評価するための基準**である。
`develop`（`59e96fc8`）時点のコードは §4 の INV-12〜INV-19 のいずれにも適合していなかった。
既存コードの一括作り替えは求めない。求めるのは、**「観測を信じずに外部状態へ書き込む」
機構（= force-write）に触れる変更が、この規律からの距離を縮めるか、少なくとも
広げないこと**である。

対象領域: `crates/awase-windows` の conv-mode force-write（ADR-085 `conv_mode_policy = force`）、
IME open/close force-write（`apply_force_on_for_imm_broken`）、および両者が使う
書き込みターゲット（hwnd）決定経路。

**実機検証の状態**: 本 ADR は新規実測を行っていない（サンドボックスに Windows 実機が無い）。
§1 の「ユーザー実機報告」は 2026-08-07〜08 の口頭報告であり、**ログによる因果の確定は
できていない**。コード読解で確定した事実（§1.2）と、未確定の仮説（§1.3）を本文中で
明示的に区別する。実装着手前に必要な実測は §5 の各 Phase と §7 に明示する。

**invariant 番号について（判断理由）**: 本 ADR の invariant は ADR-084 の INV-1〜INV-11 の
続き番号 **INV-12 から**採番する。理由は 2 つ。

1. **同一の名前空間に属するため**。ADR-084 の INV-1（conv 単一窓口）/ INV-2（書き込みと
   belief 無効化の不可分性）は、本 ADR の force-write にもそのまま適用されるべき規律で
   ある。番号を各 ADR 内でリセットすると、コミット本文や `architecture_guard.rs` の
   テスト名に現れる「INV-2」がどちらの ADR の INV-2 か曖昧になる。この領域は
   `.claude/rules/experiment-logging.md` が記録するとおり反転が繰り返されており、
   **後日の grep で一意に辿れることが規約の実効性そのもの**である。
2. **ADR-084 を直接書き換えないため**。ADR-084 は「物理シフト面・幅の SSOT」という
   別の主題を持ち、その §1 の因果分析（BUG-47 由来）は本 ADR の発端（BUG-59 追補由来）
   とは独立に成立している。084 に追記すると「どの実バグからどの invariant が導かれたか」
   の対応が失われる。Phase 0 で ADR-084 §4 の末尾に「INV-12 以降は ADR-086」という
   1 行のポインタだけを足す。

---

## 1. コンテキスト

### 1.1 発端となった実バグ

2026-08-07、コミット `9c102b02`（`feat(awase-windows): conv_mode_policy=Force の
FocusChange 強制書き込みを MS-IME にも配線（BUG-59 追補）`）が `develop` にマージされた。
このコミットは `crates/awase-windows/src/platform.rs::gji_on_focus_change` に、
「`conv_mode_policy = force` のとき、FocusChange のたびに `desired_mode().to_conv_bits()`
を実 IME へ強制書き込みする」ロジックを追加した。

翌日、ユーザーから実機報告があった。

- **LINE で何を押しても「い」になる。**
- **突然 IME がローマ字ではなく JIS かなになった。**

`conv_mode_policy = force` はデフォルト `observe` の opt-in 設定であり、影響を受けるのは
force を試験運用中のユーザー（＝報告者本人）に限られる。しかし「試験運用中の設定を
有効にしていると入力が壊滅する」状態は、ADR-085 が目指した「軽量な緩和策」の
前提を崩している。

### 1.2 コード読解で確定した 2 つの欠陥

#### 欠陥1: 書き込みターゲット（hwnd）が「書き込む瞬間のライブクエリ」で決まる

`crates/awase-windows/src/ime.rs:782` の `set_ime_romaji_mode_with_target` は、
**実行されたその瞬間に** `get_focused_hwnd()`（`GetGUIThreadInfo().hwndFocus` 優先、
失敗時 `GetForegroundWindow()` フォールバック、`ime.rs:1004`）を呼んで書き込み先を
決める。呼び出し元は書き込み先を指定できない。

一方 `gji_on_focus_change`（`platform.rs:485-513`）の force 書き込みはこう並んでいる。

```
[main]   on_ime_mode_focus_changed()            → ime_mode_focus_gen++
[main]   forced_target / conv_mutation_allowed を capture
[main]   spawn_local(async {
[main]      await get_ime_conversion_mode_raw_timeout_async(50)   ← 最大 50ms
[main]      with_app(... hint 反映 ...)
[main]      current_gen == ime_mode_gen ?                          ← ★ 陳腐化チェックはここだけ
[main]      await set_ime_romaji_mode_with_target_async(Some(target))
[worker]        └─ offload_unsafe → get_focused_hwnd()             ← ★ 実際の宛先決定はここ
[worker]        └─ get_ime_wnd(hwnd) → IMC_SETCONVERSIONMODE
[main]   })
```

★ の 2 点の間には、(a) `offload_unsafe` のワーカースレッド起動、(b) `get_focused_hwnd`
自身の `get_gui_thread_info_with_timeout(30ms)`、という 2 つの非同期／待機の間隙がある。
**世代カウンタ `ime_mode_focus_gen` はディスパッチ直前の陳腐化しかチェックしておらず、
その後にフォーカスが別ウィンドウへ移っても検知できない。** 結果、あるウィンドウ
（例: Windows Terminal）向けに計算した conv bits が、無関係な別ウィンドウ（例: LINE）の
IME コンテキストへ書き込まれ得る。

これは理論上の話にとどまらない。UWP アプリのフォーカスは親ウィンドウ →
`Windows.UI.Input.InputSite.WindowClass` 子ウィンドウという 2 段で確定するため、
1 回のユーザー操作で `gji_on_focus_change` が数 ms 間隔で複数回走る（BUG-59 本体の
実機ログで確認済みの挙動）。前段の起動した spawn_local が、後段のフォーカス先へ
書き込む窓が構造的に開いている。

なお `set_ime_romaji_mode_with_target` が `GetForegroundWindow` ではなく
`get_focused_hwnd` を使うこと自体は BUG-55 の対策として正しい。問題は
「どのウィンドウか」ではなく「**いつ決めるか**」である。

**同種の API は既にターゲット引数版が存在する**: `set_ime_open_for_target(hwnd, open)`
（`ime.rs:71`）、`set_ime_mode_for_target(hwnd, ime_on, set, clear)`（`ime.rs:1337`）、
`set_ime_romaji_mode_state_for_target(hwnd, romaji)`（`ime.rs:1381`）。トレイの
`ImeHiragana` 等はこちらを使い、`tray::menu_target_hwnd()` で捕捉済みの hwnd を渡している
（`message_handlers.rs`）。**「ターゲットを値として運ぶ」パターンはこのリポジトリに
既にあり、force-write の経路だけがそれを使っていない。**

#### 欠陥2: ADR-084 の INV-1/INV-2 違反 —— 「未移行リスト」にすら載っていない新規経路

`crates/awase-windows/src/runtime/conv_actuation.rs` の module doc は、ADR-084 P1 の
第一弾として `Runtime::actuate_conv_mode` を新設したこと、および **未移行の直接書き込み
経路**を列挙している。

> **未移行（次段のスコープ）**: `kp_restore_kana_from_half_width` の復元リトライループ、
> `tsf/warmup/cold_warmup.rs::preamble`、`runtime/executor.rs`、`kp_stage_idle_conv_check`
> のローマ字復元経路。

`9c102b02` が追加した `platform.rs::gji_on_focus_change` の force 書き込みは、
**このリストにすら載っていない**。ADR-084 が書かれた後に、ADR-084 の存在を意識せずに
追加された、完全に新しい直接書き込み経路である。具体的に:

- `actuate_conv_mode` を経由しない（INV-1 違反）。
- `ImeModeFsm::unconfirm()` を同期的に呼ばない（INV-2 違反）。BUG-49 で確立した
  「async タスク内で unconfirm すると、その完了前に `ms_ime_gate_defer` が stale な
  `is_native_ready()==true` を信じて素通しする」という教訓がそのまま素通りしている。
  皮肉なことに、同じコミット群の本体（`bcc36a5c`）はまさにこの
  「confirm フラグの書き込み元が 1 系統でない」問題を直したものだった。
- `ms_ime_gate_give_up` ラッチを解除しない。
- ジャーナル（ADR-080/082）に記録されない。

**現在 `set_ime_romaji_mode_with_target_async` を直接呼んでいる箇所は 7 つある**
（`platform.rs:505`、`conv_actuation.rs:76`、`executor.rs:760`、`key_pipeline.rs:579`、
`key_pipeline.rs:1072`、`key_pipeline.rs:1528`、`cold_warmup.rs:79`）。単一窓口を作った
ADR-084 Phase 1 の後で、窓口を通らない経路が **1 つ減るどころか 1 つ増えた**。

#### 欠陥3: ADR-084 と ADR-085 が互いを知らないまま並行に存在している

`conv_mode_policy = force` の元々の書き込み位置は `tsf/warmup/cold_warmup.rs::run_start`
（`cold_warmup.rs:79`）であり、これも `conv_actuation.rs` の未移行リスト（`cold_warmup.rs`
全体）に含まれる。つまり **ADR-085 は最初から ADR-084 の actuator 規律の外にあった**。
両 ADR は相互参照を一切持たない（ADR-084 §8 に ADR-085 は無く、ADR-085 §関連 ADR に
ADR-084 は無い）。

さらに、`9c102b02` のコミット本文・コード内コメント（`platform.rs:469`）・
`docs/known-bugs.md` の BUG-59 追補は、いずれも `conv_mode_policy = force` を
**「ADR-083」と誤記している**（確認済み: `platform.rs:469`、`known-bugs.md:6165,6261,6685,6701`。
実際は ADR-085。ADR-083 は `InjectionMode` per-VK 統一の NO-GO 調査）。設計記録を
参照せずに実装が進んだことの直接的な証拠であり、番号の訂正だけでなく「なぜ参照
されなかったか」への構造的対策が要る。

#### 欠陥4: force-write は既に 3 つの独立した機構に増殖している

ADR-085 の 2 つの追記（本文の日付が「追記（2026-08-07）」→「追記2（2026-08-06）」と
逆順になっていた。追記2 が「追記1の修正を適用・実機で有効化した後も」と明記して
いる以上、追記1が時系列で先のはずであり、これ自体が記録の整合性の弱さを示す。
本 ADR の Phase 0 で日付表記を「追記（2026-08-06）」→「追記2（2026-08-07）」に
訂正した）は、conv 軸ではなく
**IME open/close 軸**（`ImeModel::desired_open`）に対して、同種の「観測を信じず強制再送する」
パターンを `apply_force_on_for_imm_broken()`（`runtime/mod.rs:553`）へ実装した。
`conv_mode_policy` という同じ設定を読んでいるが、コードとしては完全に独立している。

現時点で「観測を信じない書き込み」は少なくとも 3 系統ある。

| # | 機構 | 軸 | トリガー | ターゲット決定 | actuator 経由 |
|---|---|---|---|---|---|
| A | `cold_warmup.rs::run_start` の `forced_target` | conv | cold 転換（`needs_f2_probe()==true` 必須 → **MS-IME では発火しない**） | ライブクエリ | ✗ |
| B | `platform.rs::gji_on_focus_change` の force 書き込み（`9c102b02`） | conv | 生 FocusChange | ライブクエリ | ✗ |
| C | `apply_force_on_for_imm_broken` の force 分岐 | open/close | `ime_refresh` 周期（既定 500ms） | strategy chain（`SendInput`、フォアグラウンド依存） | ✗（`apply_ime_open_with_belief` 経由だが force 判断は関数内に埋め込み） |

さらに **4 つ目**として、`kp_stage_idle_conv_check` の JIS かな復元
（`key_pipeline.rs:559-584`、BUG-08 対策）が `set_ime_romaji_mode_with_target_async(None)`
を独自のレート制限（`ROMAN_RESTORE_MIN_INTERVAL_MS`）付きで送る。これは
「観測（ROMAN 喪失）に基づく是正」であって force ではないが、**同じ低レベル API を
同じライブクエリ・ターゲットで叩く 4 番目の経路**である。

C については **実機での実害が既に記録されている**（`runtime/mod.rs:591-612`、確認済み）:

> 上のスロットルを単純に外すと、この 20ms 確認チェーンが「再送 → 20ms 後 refresh →
> 再送 → …」と自己駆動の無限ループに縮退する（2026-08-07 実機:「ガタつき・遅延がひどい」、
> 20〜50ms 間隔で VK_IME_ON 連打を確認）。

**force-write のトリガーを「周期」や「イベントの生発火」に置くと、その周期が別の機構と
結合して自己駆動ループになる**という失敗が、既に 1 回、実機で起きている。
`9c102b02` は同じ失敗を conv 軸で（トリガーを FocusChange に置くという形で）繰り返した。

### 1.3 未確定の仮説（ログによる確定が必要）

ユーザー報告の 2 症状について、コード読解の範囲で言えることと言えないことを分ける。

- **「JIS かなになった」**: ROMAN ビット（`IME_CMODE_ROMAN`）の喪失であり、BUG-08 の
  ファミリ（外部注入 `VK_KANA` によるかなロックトグル）と同型である。
  **重要な確定事実として、force-write 自身はこの症状を作れない**:
  `ConvMode::to_conv_bits()`（`src/engine/conv.rs:158`、確認済み）は `romaji == true` のとき
  必ず `IME_CMODE_ROMAN` を含み、`desired_mode` の唯一の書き込み点である
  `message_handlers.rs::set_desired_conv_mode`（`:528-536`、確認済み）は **常に
  `romaji: true` を渡す**（デフォルト値も `romaji: true`、`conv_mode.rs:161`）。
  したがって「force が ROMAN を消した」という説明は成立しない。
  むしろ疑うべきは逆方向で、**欠陥1 により復元系（BUG-08 の
  `[idle-conv-check] JISかな化を検出 → ローマ字入力を復元`）も別ウィンドウへ
  書き込んでいて効いていない**という筋である。切り分けは容易で、このログ行の
  有無と、その直後の conv 再読み値を見ればよい。
- **「LINE で何を押しても『い』になる」**: 現時点で機構が説明できていない。
  LINE は `ImmCross` プロファイル（`.claude/rules` / `feedback_immcross_owns_kanji`
  の対象アプリ）であり、conv の読み書き自体は成立する。JIS かな化と同時期の報告で
  あることから同一原因の可能性が高いが、**推測で因果を書かない**。
  §5 Phase 0 で `docs/known-bugs.md` に BUG-60 として起票し、再現時に取るべきログ
  （書き込み直前の hwnd / クラス名、書き込み後の conv 再読み、`[relay-passthrough]` の
  実 VK 列）を明記する。

### 1.4 ユーザーとの設計上の論点 —— force-write は「いつ」発火してよいか

ユーザーから「`conv_mode_policy=force` は本来、awase 内部の `desired_mode` だけを保持し、
実 IME への書き込みはトレイ操作やキー操作のときだけに限られる設計だったのでは?」という
問いがあった。ADR-085 の記述（「cold 転換のたびに冪等に強制書き込みする」）に照らすと
この理解自体は正確ではない。**しかし本質的な指摘が含まれている。**

| | ADR-085 元設計（機構 A） | BUG-59 追補（機構 B） |
|---|---|---|
| トリガー | `needs_f2_probe()==true` の cold 転換 | 生の `FocusChange` イベント |
| 意味 | **GJI が実際にキー入力を処理しようとして warmup が必要になった瞬間** | フォーカスが変わっただけ。ユーザーはまだ 1 文字も打っていない |
| ターゲットの確からしさ | 「今まさにこの窓に打とうとしている」窓 | 「たった今フォーカスされた（かもしれない、また変わるかもしれない）」窓 |
| 発火頻度 | 打鍵に律速される | UWP 子ウィンドウ遷移で 1 操作あたり複数回 |

機構 A のトリガーは **入力意図に紐づいている**。だから「どの窓に書くか」が自明で、
書き込みが無駄打ちにならず、頻度も打鍵に律速される。機構 B はこの紐づけを失った。
**欠陥1（ターゲット競合）は欠陥2（actuator 迂回）と独立ではなく、トリガー条件を
入力意図から切り離したことが、ターゲットの不確かさを実害に変えた**というのが
本 ADR の中心的な因果認識である。

### 1.5 既存資産との関係

本 ADR は既存 ADR を否定しない。**ADR-085 が導入した「force」という方針を、
ADR-084 が定めた actuator 規律の内側へ移す**ものである。

| 既存 | 何を定めたか | 本 ADR との関係 |
|---|---|---|
| ADR-064 `ConvModePolicy` | conv を書いて**よいか**（`Output::conv_mutation_allowed`） | 許可ゲート。機構 A/B とも正しく見ている。本 ADR は「どこで・いつ・どこへ書くか」を足す |
| ADR-072 conv authority 再同期 | 「遷移エッジではなく apply 完了点で同期する」 | **3 例目の同型の誤り**。機構 B は `FocusChange` という遷移エッジに書き込みを紐づけた |
| ADR-078 belief 3 分割 | `DesiredMode`/`EffectiveMode`/`ModeConstraint`（Phase 1a のみ実装） | `desired_mode`（ADR-085）は `DesiredMode` の先行実装に相当する。統合順序は §7-4 |
| ADR-080/082 actuation ライフサイクル・ジャーナル | actuation の記録先と epoch fencing | INV-14 の「ターゲット同一性」は ADR-080 の epoch fencing と同型の規律を **空間軸（hwnd）** に拡張したもの |
| **ADR-084** | conv 単一窓口（INV-1）、書き込みと belief 無効化の不可分性（INV-2） | **本 ADR の直接の親**。INV-12 以降は 084 の番号空間を継承する |
| **ADR-085** | `conv_mode_policy = force`、`desired_mode`、open/close 軸への拡張 | **本 ADR が規律を与える対象**。ADR-085 は「何を目標にするか」を定め、本 ADR は「いつ・どこへ・どの窓口で書くか」を定める |
| `.claude/rules/ime-belief-architecture.md` | Observe → Pure → Apply、`InputModeApplied` / confidence、3 段構えの強制 | INV 群と強制メカニズムをこの語彙・機構に乗せる。force-write は定義上 **Apply 層の操作**であり、Observe を根拠に持たない点を型で明示する |

---

## 2. 決定

### 2.1 用語

**force-write（強制書き込み）**: 外部状態（実 IME の conv-mode / open 状態）の観測結果を
**判断材料に使わずに**、awase 自身の意図（`desired_mode` / `desired_open`）を根拠として
外部状態を書き換える操作。`.claude/rules/ime-belief-architecture.md` の三層
（Observe → Pure → Apply）で言えば、**Observe 層への依存を意図的に持たない Apply 操作**
である。ゆえに `InputModeObserved` で表現してはならず、常に
`InputModeApplied { strategy, result }` で表現される（同ルールの禁止パターン2）。

**observation-based correction（観測に基づく是正）**: 観測（drift 検出、ROMAN 喪失検出）を
トリガーに持つ書き込み。ADR-084 案3 の評価で「投機ではなく観測に基づく是正」と
区別されたもの。BUG-08 の JIS かな復元がこれに当たる。**本 ADR の INV-14（ターゲット
同一性）は force-write と observation-based correction の両方に適用されるが、
INV-15（トリガー条件）は force-write にのみ適用される。**

### 2.2 責務の再配置（目標状態）

| 関心事 | 所有コンポーネント | 禁止事項 |
|---|---|---|
| 「今どこへ書くべきか」の決定 | **書き込みを起案した時点のコンポーネント**。決定した hwnd を値として運ぶ（`ActuationTarget`） | 低レベル write 関数が `get_focused_hwnd()` をライブクエリして宛先を自分で決めること |
| 決定時点と実行時点のターゲット同一性の検証 | **低レベル write 関数の内部**。実行直前に再確認し、不一致なら書き込まず `Aborted` を返す | 世代カウンタのチェックだけで「陳腐化していない」とみなすこと（gen は時間軸のみを守り、空間軸を守らない） |
| conv-mode force-write の実行 | **`Runtime::actuate_conv_mode`（ADR-084 INV-1）**。`ConvMutationReason::ForcePolicy` として申告する | `platform.rs` / `cold_warmup.rs` が独自に `set_ime_romaji_mode_with_target_async` を呼ぶこと |
| open/close force-write の実行 | **既存の apply 経路（`apply_ime_open_with_belief` → `on_ime_apply_complete`）**。force であることを `InputModeApplyStrategy::ForcePolicyResend` で申告する | force 判断・レート制限・再スケジュールを `apply_force_on_for_imm_broken` の関数本体に埋め込むこと |
| force-write のトリガー条件 | **入力意図（入力の直前・cold 転換）に紐づく単一の判定関数** | 生の `FocusChange` / 生のタイマー周期をトリガーにすること |
| force-write の再スケジュール | **外部の周期（`ime_poll_interval_ms`）にのみ従う独立した予約** | actuation の完了ハンドラが次の actuation を無条件に予約すること（自己駆動ループ） |

### 2.3 原則

#### P6: force-write は「観測を信じない Apply」であり、observation-based correction と型で区別する

（P1〜P5 は ADR-084 が使用済みのため P6 から採番する。）

force-write は Observe 層を入力に取らない。したがって

- `InputModeObserved` を dispatch してはならない。
- `InputModeApplied { strategy: ForcePolicy* , result }` を必ず dispatch する
  （ADR-084 INV-6 の force 版）。
- `ConvMutationReason` に `ForcePolicy` variant を追加する（ADR-084 §2 の
  `DriftCorrection` は observation-based であり、force とは別 variant にする）。

#### P7: 書き込みターゲットは値であり、決定から実行まで一貫して運ばれ、実行直前に検証される

ADR-080 の epoch fencing は「いつの actuation か」を守る（時間軸）。本 ADR はそれを
「どこへの actuation か」（空間軸）へ拡張する。

```rust
/// 「この actuation はこの hwnd へ向けたものである」という宣言。
/// 起案時点で捕捉し、実際の Win32 呼び出しの直前に再検証する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActuationTarget {
    hwnd: HWND,
    /// 起案時点の `ime_mode_focus_gen`（時間軸のフェンス。ADR-080 と同型）。
    focus_gen: u64,
}

impl ActuationTarget {
    /// フォーカス変化イベント等の非同期経路から捕獲する。
    /// 内部で `get_focused_hwnd_async()`（`get_focused_hwnd()` の async 版）を
    /// 呼ぶため async（§7-3 の設計調査で確定、フックスレッドを塞がないため
    /// 同期版は提供しない）。
    pub(crate) async fn capture() -> Option<Self>;

    /// 実行直前に呼ぶ。フォーカスが動いていれば `None` を返し、呼び出し元は
    /// 書き込まずに `Aborted` を返す。`get_focused_hwnd_async()` をもう一度
    /// 呼ぶため async（§7-3 参照。同期版にすると BUG-55 を避けるために必要な
    /// `GetGUIThreadInfo` ベースの解決を諦めて `GetForegroundWindow` へ
    /// 妥協することになりかねないため、非同期のままにする）。
    pub(crate) async fn verify_still_current(self) -> Option<HWND>;
}
```

`verify_still_current` の実装は「今の `get_focused_hwnd()` が `self.hwnd` と一致するか」
の再確認であり、原理的に完全な排他は取れない（確認と `SendMessageTimeoutW` の間にも
理論上の窓は残る）。それでよい。**現状は数十 ms 規模の窓が確実に開いているのに対し、
検証を実行直前に置けば窓は Win32 呼び出し 1 回分に縮む。** 完全性ではなく、
窓の桁を下げることが目的である。窓が残ること自体は `Aborted` のログと
ジャーナル記録で観測可能にする。

#### P8: force-write のトリガーは入力意図に紐づく

生の `FocusChange` や生の周期タイマーを直接のトリガーにしない。理由は 3 つ。

1. **ターゲットが確からしくない**。フォーカスは確定するまでに複数段を経る
   （UWP 親 → InputSite 子）。「打とうとしている窓」なら確定済みである。
2. **無駄打ちが実害になる**。force-write は conv を丸ごと置換するため、ユーザーが
   IME 自身の UI で選んだ状態を消す。打鍵前に消す必要はない。
3. **自己駆動ループを誘発する**（§1.2 欠陥4 の実機記録）。周期に紐づけると、
   actuation の完了が次の周期を呼ぶ経路と結合しやすい。

トリガーの目標形は **arm-on-focus / fire-on-intent**（§3.1 案 C）である。
FocusChange は「次の入力までに 1 回 force してよい」というフラグを**武装**するだけで、
実際の書き込みは次の入力意図（cold 転換 / 送信直前ゲート通過）まで遅延する。

#### P9: 軸が違っても force-write の規律は同一である

conv 軸（機構 A/B）と open/close 軸（機構 C）は、書き込む対象も Win32 API も違うが、
**「観測を信じずに自分の意図を押し付ける」という意味論は同一**である。したがって

- 同じ設定（`conv_mode_policy`。名前が conv 専用に見えるのは §7-5 の未解決論点）で
  制御される以上、同じトリガー規律・同じターゲット規律・同じ記録先に従う。
- 片方だけに付いているガード（C のレート制限）は、もう片方にも必要かを必ず検討する。
- 片方だけに付いていない安全弁（B には `Aborted` 概念が無い）は欠落として扱う。

#### P10: force-write の目標値は ROMAN ビットを落とさない

`ConvMode::to_conv_bits()` は `romaji: true` のとき必ず `IME_CMODE_ROMAN` を含む。
awase の engine は romaji VK 列を出力するため、**ROMAN が落ちた IME へ書き込むことは
出力の壊滅を意味する**（BUG-08）。将来 `desired_mode` に `romaji: false` を設定できる
経路（JIS かな配列サポート、ADR-068）を作る場合は、その設定が engine の出力方式
（romaji VK か kana VK か）と**同時に**切り替わることを型で保証してからにする。

---

## 3. 代替案の比較

この領域は `.claude/rules/experiment-logging.md` が記録するとおり反転が繰り返されている。
**「良いアイデアに見えるか」ではなく「過去にどの条件で壊れたか」で評価する。**

### 3.1 force-write のトリガー条件

#### 案 A: 生の `FocusChange` 上で即時発火（= 現状、`9c102b02`）

- **利点**: MS-IME（`needs_f2_probe()==false`）でも確実に発火する。BUG-59 追補が
  埋めようとした穴（Force 運用中の MS-IME で強制が一度も走らない）に直接効く。
  実装が小さい。
- **欠点（実機で実害が報告済み）**:
  - ターゲット競合（§1.2 欠陥1）。UWP の 2 段フォーカスで 1 操作あたり複数回発火する。
  - ユーザーが 1 文字も打っていない窓の conv を書き換える。IME 自身の UI で選んだ
    状態を無条件に消す。
  - `actuate_conv_mode` を迂回し `unconfirm` を呼ばないため、BUG-49 の
    「stale `is_native_ready()` で素通し」に再び道を開く。同じコミット群の本体
    `bcc36a5c` が塞いだ穴の隣に、別の穴を開けた形になっている。
- **評価**: **却下（再提案禁止）**。ただし「MS-IME に force の発火点が無い」という
  問題提起自体は正しく、案 C が別の形で解決する。

#### 案 B: 実入力に紐づく cold 転換のみ（= ADR-085 元設計、機構 A）

- **利点**: トリガーが「GJI が実際に warmup を必要とした瞬間」＝入力意図に紐づく。
  ターゲットが確からしい。頻度が打鍵に律速される。**GJI で実際に運用実績がある**。
- **欠点（確定した構造的穴）**: `run_start` は `output/vk_send.rs` の
  `if prepend_f2_warmup { ... }` からしか呼ばれず、`prepend_f2_warmup` は
  `warmup_coord.needs_f2_probe()` を要求する。`MsImeStrategy::needs_f2_probe()` は
  **常に `false`**（「MS-IME は常に warm」という BUG-13 以来の前提、
  `tsf/warmup/warmup_strategy.rs:124-126`）。したがって **MS-IME では force が
  一度も発火しない**。BUG-59 追補が正しく指摘した穴である。
- **評価**: **単独では不十分**。トリガーの「質」は正しいが、GJI 固有の warmup 機構に
  相乗りしているため IME 種別で発火可否が割れる（ADR-084 INV-7 の
  「IME 種別の非対称を隠さない」に抵触する）。案 C の土台として採用する。

#### 案 C: arm-on-focus / fire-on-intent（**推奨する北極星**）

- **内容**: `FocusChange` は `force_pending`（1 回分の武装）を立てるだけで書き込まない。
  実際の force-write は次のいずれかで発火する。
  1. cold 転換（案 B の既存経路、IME 種別に依存しない位置へ移す）
  2. 最初の送信要求が送信ゲート（`ms_ime_gate_defer` / `defer_vk_if_probe_in_flight`）を
     通過する直前
  発火したら `force_pending` を落とす。フォーカスが再び変われば再武装する。
- **利点**:
  - **ターゲットが自明**。発火時点で「今この窓へ送ろうとしている」ことが確定して
    いるため、hwnd は送信要求が既に持っている（P7 の `ActuationTarget` を
    自然に埋められる）。
  - **IME 種別に依存しない**。`needs_f2_probe()` に相乗りしないため案 B の穴が消える。
  - **無駄打ちが消える**。フォーカスだけ動いて打鍵しない窓（Alt+Tab の中間ウィンドウ、
    タスクバー、トレイ）には一切書かない。ADR-084 の `ime_apply_should_defer`
    （settle 中の直接呼び出し抑止）と同じ発想。
  - **自己駆動ループが構造的に起きない**。発火が入力に律速されるため、
    actuation → refresh → actuation の閉路を作れない。
- **欠点・リスク**:
  - **最初の 1 文字が force の完了を待つか否か**の設計判断が要る。待つと
    レイテンシが増え（IMC write は最大 2 回の `SendMessageTimeoutW`、`tuning.rs:157`）、
    待たないと最初の 1 文字だけ古い conv で送られる。**ここは実測が要る**
    （`.claude/rules/tuning-constants.md`）。本 ADR は数値を確定しない。
  - 「入力意図」の定義点が複数ある（cold 転換 / 送信ゲート）。両方に配線すると
    二重発火しうるので、`force_pending` の消費は 1 箇所に集約する必要がある。
  - 長時間 1 つの窓に留まったまま drift した場合（フォーカスが変わらないので
    再武装されない）を救えない。BUG-59 追補のコミット本文が「見送った」と
    書いているケースそのものである。→ §7-2 の未解決論点として残す。
- **評価**: **北極星として採用。** §5 Phase 2。

#### 案 D: 周期ポーリングで常時再送（= 機構 C、`apply_force_on_for_imm_broken`）

- **利点**: フォーカスが変わらないまま drift するケースも救える（案 C の弱点を補う）。
  open/close 軸では既に実装されている。
- **欠点（実機で実害が記録済み）**: `runtime/mod.rs:591-612` のとおり、
  `on_ime_apply_complete` → `post_ime_refresh()` の 20ms 確認チェーンと結合して
  **自己駆動の無限ループに縮退し、20〜50ms 間隔で `VK_IME_ON` を連打した**
  （2026-08-07 実機:「ガタつき・遅延がひどい」）。修正として `last_force_on_resend_ms`
  による自前レート制限が入ったが、**これは「周期をトリガーにすると別の周期と結合する」
  という構造の対症療法**である。加えて、周期発火は「今どの窓か」を毎回ライブクエリで
  決めるため欠陥1 をそのまま持つ。
- **評価**: **新規採用しない。** 既存の機構 C は Phase 3 で案 C 側へ寄せる。
  ただし「フォーカス不変のまま drift」を救う手段としての価値は残るため、
  §7-2 で「低頻度の健全性チェック（例: 分オーダー）としてなら別枠で検討可」と
  記録する。**その場合も実測なしに間隔を決めない。**

#### 案 E: ユーザーの明示操作のみ（トレイ / リセットコンボ）

- **内容**: awase は `desired_mode` / `desired_open` を保持するだけで、実 IME への
  書き込みはトレイ操作と `Ctrl+Shift+無変換` → `Ctrl+Shift+変換` のリセットコンボに限る。
  §1.4 でユーザーが想定していた形。
- **利点**: 競合もループも起きない。ターゲットはトレイの `menu_target_hwnd()` で
  既に確定している。最も安全。
- **欠点**: ADR-085 の目的（drift の自動回復）を放棄する。BUG-59 追補の起点である
  ユーザー報告「変な状態になるが手動リセットでしか治らない」に対して
  **何も改善しない**。
- **評価**: **却下（ただし退避先としては有効）**。案 C が実機で否定された場合の
  フォールバックとして、`conv_mode_policy` に第 3 の値（`manual`）を足す形なら
  意味がある。§7-6。

#### 案 F: 案 C + 低頻度の健全性チェック（折衷）

- **内容**: 通常は案 C（arm-on-focus / fire-on-intent）。加えて、**打鍵が一定時間
  途絶えた後の最初の打鍵**（= `session_expired` / long idle、既存の
  `COMPOSITION_TIMEOUT_MS` 判定）を追加の武装トリガーにする。周期タイマーではなく
  **「idle 明けの最初の入力」**という入力起点のイベントである点が案 D と決定的に違う。
- **利点**: 「フォーカス不変のまま長時間 idle → drift」を、周期ポーリングを導入せずに
  救える。ユーザーが BUG-59 追補のきっかけとして述べた「long idle から入力開始の
  ときも同じようなリセットをしたら」という直感に一致する。
- **欠点**: `session_expired` の閾値（`COMPOSITION_TIMEOUT_MS`）は cold-start 判定用に
  調律された値であり、force-write の適切な間隔として妥当かは**未検証**。
  流用してはならない（ADR-084 Phase 1 の実測義務と同型の注意）。
- **評価**: **案 C の次段として採用候補。** ただし閾値の妥当性を実測してから。§5 Phase 2b。

#### 比較表

| 案 | ターゲットが確か | IME 種別非依存 | 無駄打ちなし | 自己駆動ループなし | フォーカス不変 drift を救う | 実装量 | 反転リスク |
|---|---|---|---|---|---|---|---|
| A 生 FocusChange | ✗ | ○ | ✗ | ○ | ✗ | 小 | **高（実害報告済み）** |
| B cold 転換のみ | ○ | **✗** | ○ | ○ | ✗ | — | 中（MS-IME で無効） |
| C arm/fire-on-intent | ○ | ○ | ○ | ○ | ✗ | 中 | 中（レイテンシ未実測） |
| D 周期再送 | ✗ | ○ | ✗ | **✗（実害報告済み）** | ○ | 小 | **高** |
| E 明示操作のみ | ○ | ○ | ○ | ○ | ✗（自動回復を放棄） | 小 | 低 |
| F C + idle 明け | ○ | ○ | ○ | ○ | ○（部分的） | 中 | 中（閾値未実測） |

### 3.2 書き込みターゲット識別をどう最後まで保持するか

#### 案 T1: 低レベル関数がライブクエリで決める（現状）

- **欠点**: §1.2 欠陥1。呼び出し元が「どこへ書いたか」を知る手段が無く、ログにも
  出ない（`[imm-romaji] conv ... → ...` は hwnd を出さない）。**事後の切り分けすら
  できない**のが最大の問題。
- **評価**: **却下。** ただし移行期間中、既存の 7 経路を一度に変えることはできないため、
  暫定措置として **書き込み先 hwnd とクラス名をログに出す**だけでも
  切り分け能力は大きく上がる（Phase 1 の最初のステップ）。

#### 案 T2: hwnd をキャプチャして渡す（検証なし）

- **利点**: 実装が最小。`set_ime_romaji_mode_with_target_for_target(hwnd, target)` を
  足すだけ。既存の `set_ime_mode_for_target` と同じ形。
- **欠点**: **フォーカスが移った後でも、元の（今は非フォーカスの）窓へ確実に書く**。
  「間違った窓に書く」問題は消えるが、「もう関係ない窓に書く」問題が残る。
  非フォーカス窓の IMC を書き換えると、ユーザーがその窓へ戻ったときに
  意図しない状態になる。
- **評価**: **単独では不十分**。ただし T3 の必須の土台。

#### 案 T3: hwnd をキャプチャ + 実行直前に再検証、不一致なら中止（**推奨**）

- **内容**: P7 の `ActuationTarget`。`verify_still_current()` が `None` を返したら
  書き込まず `ActuationOutcome::Aborted { reason: TargetMoved }` を返す。
  **`Aborted` は成功として記録しない**（`applied` キャッシュを更新しない、
  ジャーナルには `Aborted` として残す）。
- **利点**:
  - 「間違った窓に書く」も「もう関係ない窓に書く」も消える。
  - `Aborted` の頻度がログに出るため、**そもそもこの競合が実機でどれくらい
    起きているかを測れる**（現状は測る手段が無い）。
  - トレイ経路（`menu_target_hwnd()`）と同じパターンに収束する。
- **欠点**: 完全な排他ではない（P7 に明記のとおり、検証と Win32 呼び出しの間に
  1 回分の窓は残る）。`get_focused_hwnd()` を 2 回呼ぶコスト
  （`get_gui_thread_info_with_timeout(30ms)` × 2）が乗る。
  → 検証側はタイムアウトを短くする、あるいは `GetForegroundWindow` の
  軽量比較で足りるかを実測で判断する。**数値は本 ADR では確定しない。**
- **評価**: **採用。** §5 Phase 1。

#### 案 T4: 世代カウンタのみで守る（現状の `ime_mode_focus_gen`）

- **欠点**: 世代は**時間軸**（「フォーカス変更が起きたか」）しか表さない。
  gen チェックの後に発生した変更は原理的に見えない。加えて gen チェックは
  `with_app`（メインスレッド）でしか行えないのに対し、実際の書き込みは
  ワーカースレッドで行われるため、**チェックと実行を同じ場所に置けない**。
- **評価**: **単独では却下。** ただし T3 の `ActuationTarget` に `focus_gen` を
  同梱し、hwnd 一致 **かつ** gen 一致を要求する形なら有効
  （hwnd は再利用されうるため、時間軸のフェンスは依然必要）。

#### 案 T5: 書き込みをすべてメインスレッド同期実行にする

- **欠点**: **不可能**。`SendMessageTimeoutW` のクロスプロセス呼び出しをフックスレッド／
  メインループでブロックすると BUG-34（フック応答遅延）を再現する。
  `spawn_local` + `offload` はその制約から来ている。
- **評価**: **却下（再提案禁止）**。

---

## 4. 不変条件（invariant）

ADR-084 の INV-1〜INV-11 を継承し、INV-12 から採番する（採番理由は §ステータス）。
**INV-1（conv 単一窓口）と INV-2（書き込みと belief 無効化の不可分性）は force-write にも
そのまま適用される** —— 以下はその上に積む追加規律である。

- **INV-12（force-write も actuator を通る）**: `conv_mode_policy = force` による
  conv-mode 書き込みは `Runtime::actuate_conv_mode`（ADR-084 INV-1）を通る。
  `ConvMutationReason::ForcePolicy` として申告し、ADR-084 INV-2 に従って
  `ImeModeFsm::unconfirm()` を**同期的に**呼ぶ。`ms_ime_gate_give_up` ラッチも解除する。
  `platform.rs` / `cold_warmup.rs` が `set_ime_romaji_mode_with_target_async` を
  直接呼ぶ形は許容しない。

- **INV-13（軸の対称性）**: conv 軸の force-write と IME open/close 軸の force-write は、
  **同一の規律に従う**。具体的には (a) 同じトリガー判定（INV-15）、(b) 同じターゲット
  規律（INV-14）、(c) 同じ記録先（`InputModeApplied` + ADR-080/082 ジャーナル）、
  (d) 同じ `ConvModePolicy` 設定を単一の判定関数から読む。
  片方の軸にだけガードや安全弁を追加してはならない —— 追加する場合は他方にも
  必要かを検討し、不要と判断した理由をコード内に残す（ADR-084 INV-7 と同型の対称性要求）。

- **INV-14（ターゲット同一性 / target identity）**: 外部 IME 状態への書き込み
  （IMC write・conv 系 VK 注入・`SendInput` による IME 制御キー）は、
  **書き込み先 hwnd を起案時点で値として確定し、実際の Win32 呼び出しの直前に
  同一性を再検証しなければならない**。不一致なら書き込みを中止し `Aborted` を返す。
  - 低レベル write 関数が `get_focused_hwnd()` / `GetForegroundWindow()` を
    自分で呼んで宛先を決めてはならない。
  - `Aborted` を成功として記録してはならない（`applied` 系キャッシュを更新しない）。
  - `Aborted` は必ずログとジャーナルに残す（競合頻度を実測可能にするため）。
  - 世代カウンタ（時間軸）だけでこの要求を満たしたとみなしてはならない。
    hwnd（空間軸）と世代（時間軸）の**両方**を検証する。
  - *一般形*: 非同期に実行される外部書き込みは、「いつの意図か」（epoch、ADR-080）と
    「どこへの意図か」（target）の両方をフェンスしなければならない。片方だけでは
    「正しい世代の、間違った宛先」への書き込みを許してしまう。

- **INV-15（force-write のトリガー条件）**: force-write は **入力意図に紐づくイベント**
  でのみ発火してよい。具体的に許容されるトリガーは次の 3 種のみ。
  1. cold 転換（実際に送信の準備が必要になった瞬間）
  2. 送信要求が送信ゲートを通過する直前（最初の 1 文字の直前）
  3. idle 明けの最初の入力（案 F を採用する場合。閾値は実測に基づくこと）

  **禁止されるトリガー**: 生の `FocusChange` イベント、生の周期タイマー
  （`ime_refresh` 等）、他の actuation の完了ハンドラ。
  `FocusChange` は「次の入力までに 1 回 force してよい」というフラグの**武装**にのみ
  使ってよく、それ自体が書き込みを起こしてはならない。

- **INV-16（自己駆動の禁止）**: actuation の完了ハンドラが、次の actuation を
  無条件に予約してはならない。予約が必要な場合は、**外部の周期（設定値）にのみ従う
  独立した予約**とし、actuation の結果を予約間隔の入力にしない。
  *この invariant が無いことの実害*: 2026-08-07 実機、`apply_force_on_for_imm_broken` の
  force 分岐が `on_ime_apply_complete` → `post_ime_refresh()` の 20ms 確認チェーンと
  結合し、20〜50ms 間隔で `VK_IME_ON` を連打（「ガタつき・遅延がひどい」）。

- **INV-17（ROMAN ビットの保存）**: force-write の目標 conv 値は、
  `desired_mode.romaji == true` である限り必ず `IME_CMODE_ROMAN` を含む。
  ROMAN を落とす値を書けるようにするのは、engine 側の出力方式（romaji VK / kana VK）が
  同時に切り替わることを型で保証できるようになってからとする（BUG-08、ADR-068）。

- **INV-18（force と observation-based correction の型による区別）**: force-write は
  `InputModeApplied { strategy: ForcePolicy.. }`、観測に基づく是正は
  `InputModeApplied { strategy: DriftCorrection / ImmBrokenCorrection.. }` を使い、
  strategy variant を共有しない。ログ・ジャーナルから「これは観測を根拠にしたか」が
  一意に読めなければならない。`InputModeObserved` を force-write に使うことは、
  `.claude/rules/ime-belief-architecture.md` の禁止パターン2（観測の偽装）に該当する。

- **INV-19（未移行リストの単調減少）**: `runtime/conv_actuation.rs` の module doc が
  列挙する「未移行の直接書き込み経路」は、**増やしてはならない**。
  新しい conv 書き込み経路を追加する変更は、`actuate_conv_mode` を経由するか、
  さもなくば未移行リストへの追記とその理由の明記を伴わなければならない。
  `9c102b02` はリストに載せずに 7 番目の経路（`platform.rs`）を増やした
  —— これを機械的に検知する（§6 段3）。

---

## 5. 移行計画

各 Phase は独立してリリース可能で、後の Phase が実機で否定されても前の Phase は残る。

### Phase 0（記録と退避、実機不要）

1. 本 ADR を `docs/adr/086-force-write-trigger-and-target-identity.md` として追加、
   `docs/adr/index.md` に登録。
2. **ADR-084 §4 の末尾に 1 行追記**: 「INV-12 以降は ADR-086 で採番する」。
   ADR-084 §8 に ADR-085/086 への相互参照を追加。
3. **ADR-085 に従属関係を追記**: 「本 ADR が定める force の**目標値**（`desired_mode`）は
   維持するが、**いつ・どこへ・どの窓口で書くか**は ADR-086 が定める」。
   あわせて ADR-085 の「追記」「追記2」の日付逆転を訂正する。
4. **`ADR-083` 誤記の訂正**: `platform.rs:469` のコメント、`docs/known-bugs.md` の
   BUG-59 追補、`9c102b02` の記述が参照する「ADR-083」を ADR-085 へ訂正
   （コミット本文は書き換えられないため、known-bugs.md 側に訂正注記を置く）。
5. **`docs/known-bugs.md` に BUG-60 を起票**: 「`conv_mode_policy = force` 運用中に
   LINE で全打鍵が『い』になる / IME が JIS かなになる（原因未確定）」。
   §1.3 の切り分け手順（`[idle-conv-check] JISかな化を検出` ログの有無、
   書き込み先 hwnd/クラス、書き込み後の conv 再読み）を明記する。
   これは `.claude/rules/fix-requires-evidence.md` の (b) を満たす。
6. **`9c102b02` の扱い（下記）**。

#### `9c102b02`（BUG-59 追補）をどう扱うか

**推奨: revert する**（`hotfix` ではなく `develop` 上の通常の revert）。理由:

- 実機で実害が報告されている一方、**このコミット自体は「Windows 実機での動作確認は
  未実施」のまま `develop` に入った**（コミット本文に明記）。
- 埋めようとした穴（MS-IME で force が発火しない）は Phase 2 の案 C で
  **より正しい形で**埋まる。急いで残す理由が無い。
- 残したまま Phase 1（ターゲット同一性）を先に入れると、「競合は減ったが
  トリガーは間違ったまま」という中途半端な状態で実機ソークすることになり、
  どちらの改善が効いたのか切り分けられなくなる。

**revert コミットの本文には `.claude/rules/experiment-logging.md` に従い次を必ず書く**:
アプリ（LINE / Windows Terminal）、IME（MS-IME、`conv_mode_policy = force` 有効、
TsfNative / ImmCross 両方でフォーカス往復）、再現手順と症状（force 有効時に
LINE で全打鍵が「い」になる・IME が JIS かなになる。生 FocusChange をトリガーにした
force 書き込みが、ワーカースレッド上の `get_focused_hwnd()` ライブクエリで
別ウィンドウへ着弾しうる構造）。加えて **「MS-IME で force が一度も発火しない」という
指摘自体は正しく、ADR-086 Phase 2 で案 C として再導入予定**である旨を書く
（これを書かないと、後日「MS-IME の穴が塞がっていない」と気づいた別セッションが
同じ生 FocusChange トリガーを再導入する）。

**代替案（判断が割れる点）**: revert せず、`9c102b02` を本 ADR の Phase 0 として
追認し、Phase 1 でターゲット検証を足して救う。`conv_mode_policy` はデフォルト
`observe` であり影響範囲は試験運用中のユーザーのみ、という事実がこの選択を支える。
ただしこの場合でも **`develop` の HEAD に「INV-12/14/15 すべてに違反するコードが
意図的に残っている」状態**になるため、known-bugs.md に「Phase 2 完了まで
`conv_mode_policy = force` は使用しないこと」と明記する必要がある。

### Phase 1（INV-14: ターゲット同一性、中リスク）

**状態（2026-08-08）: Phase 1a・1b とも実装完了。** §7-3 の設計調査により
「新しいタイミング定数を導入しない」設計に確定したため、Phase 1a の実測義務
（起案-実行間 ms・hwnd 変化率・検証コスト）は新規タイムアウト値の決定には
使わず、Phase 1b へ直接進んだ（既存の `get_focused_hwnd`/`get_gui_thread_info_with_timeout`
の値をそのまま流用する設計のため、新規実測が導出に必要な値そのものが無い）。
実機での競合頻度計測は今後の実機ソークで別途行う。全経路の移行完了に伴い、
`set_ime_romaji_mode_with_target(_async)`（ライブクエリ版）は削除済み
（Phase 1b step6、下記 6 参照）。

**Phase 1a（観測のみ、実機データ収集）**

1. `set_ime_romaji_mode_with_target` のログに**書き込み先 hwnd とウィンドウクラス名**を
   追加する。書き込みを起案した時点の hwnd（`ActuationTarget` の前身として、
   呼び出し元で捕捉してログに渡す）と、実際に書いた hwnd の**両方**を出す。
2. これで「起案時と実行時で宛先がずれる頻度」が実機で測れるようになる。
   コード変更は診断のみで挙動は変えない。

**実測義務**（`.claude/rules/tuning-constants.md`）: Phase 1b に進む前に、
(a) 起案から実際の Win32 write までの経過 ms、(b) その間に hwnd が変わった割合、
(c) `get_gui_thread_info_with_timeout` を検証用に 2 回呼ぶコストの実測、を取る。
**これらの実測なしに検証タイムアウト値を決めない。**

**Phase 1b（`ActuationTarget` 導入）**

3. `ActuationTarget { hwnd, focus_gen }` と `verify_still_current()` を新設。
4. `set_ime_conv_for_target(target: ActuationTarget, conv: Option<u32>) -> ActuationOutcome`
   を追加（`ActuationOutcome` は `#[must_use]`、`Written` / `Aborted{TargetMoved|GenStale}` /
   `Failed`）。既存の `set_ime_romaji_mode_state_for_target` と同じ hwnd 引数形。
5. **conv 書き込みの全 6 経路**（`9c102b02` の `platform.rs` 経路は §1.2 欠陥1の
   誤爆バグごと revert 済みのため対象外。実質 6 経路）を段階的にこちらへ移す。
   ✅ 完了（2026-08-08）: `conv_actuation.rs::actuate_conv_mode`、
   `cold_warmup.rs::run_start`、`executor.rs::dispatch_ime_set_open`
   （`set_ime_open_then_conv_for_target`、open/conv を同一 hwnd に対して行う
   特殊版）、`key_pipeline.rs` の `kp_stage_idle_conv_check`（BUG-08）・
   `kp_reset_to_hiragana_romaji_capsoff`（read-modify-write、read 側は
   `get_ime_conv_for_target`）・`kp_restore_kana_from_half_width`
   （復元リトライループ、hwnd はループ外で1回 capture して全試行で使い回す —
   毎試行 capture は検証を no-op 化するため不採用、opus アドバーサリアル
   レビュー 2026-08-08）。
6. ✅ 完了（2026-08-08）: 全経路の移行が済んだため
   `set_ime_romaji_mode_with_target(_async)`（ライブクエリ版）を**削除**した
   （§6 段1 のコンパイラ強制の前段）。`tests/architecture_guard.rs` に
   `actuation_target_capture_call_sites_are_accounted_for` を新設し、
   `ActuationTarget::capture` の呼び出し箇所数（6）を固定することで
   INV-19 の「未追跡の新規経路を検知する」役割を引き継いだ。

### Phase 2（INV-15: トリガーを arm-on-focus / fire-on-intent へ、中リスク）

1. `ConvMutationReason::ForcePolicy` と `ConvModeTarget::Explicit(u32)` を追加し、
   `actuate_conv_mode` が force の目標値を書けるようにする（INV-12）。
   ADR-084 INV-2 に従い `unconfirm()` を同期的に呼ぶ。
2. `force_pending`（1 回分の武装フラグ）を `Output` に新設。`FocusChange` は
   これを立てるだけにする。
3. 消費点を **1 箇所**に集約する: 送信要求が送信ゲートを通過する直前
   （`ms_ime_gate_defer` / `defer_vk_if_probe_in_flight` の手前）。
   ここは既に「今この窓へ送ろうとしている」ことが確定しており、
   `ActuationTarget` を自然に埋められる。
4. `cold_warmup.rs::run_start` の `forced_target` を `actuate_conv_mode` 経由に移し、
   同じ `force_pending` を消費する形にする（二重発火の防止）。
   これにより `needs_f2_probe()` への依存が消え、案 B の MS-IME の穴が塞がる。

**実測義務**: 「最初の 1 文字が force の完了を待つべきか」を決めるため、
`actuate_conv_mode` の IMC write 完了から IMC read で目標 conv が確認できるまでの
実測 ms が要る。**ADR-084 Phase 1 が要求している実測と同一のもの**であり、
まとめて 1 回の実機セッションで取ればよい。既存の `MS_IME_READY_CONFIRM_MS`（400ms）は
IME OFF→ON 遷移の実測であり、conv 書き換えの実測ではない。流用してはならない。

**Phase 2b（案 F、Phase 2 の実機ソーク後）**: idle 明けの最初の入力を追加の
武装トリガーにする。閾値の妥当性（`COMPOSITION_TIMEOUT_MS` を流用してよいか）を
実測してから。

### Phase 3（INV-13/INV-16: open/close 軸を同じ規律へ、中リスク）

1. `apply_force_on_for_imm_broken` の force 分岐を、関数本体に埋め込まれた
   レート制限（`last_force_on_resend_ms`）ごと、Phase 2 の `force_pending` 機構へ移す。
   周期トリガー（案 D）から入力意図トリガー（案 C）へ変える。
2. `InputModeApplyStrategy::ForcePolicyResend` を追加し、force による ON 再送が
   ログ・ジャーナルから一意に識別できるようにする（INV-18）。
3. `apply_ime_open_with_belief` の送信先も `ActuationTarget` で守る（INV-14）。
   `SendInput` はフォアグラウンドに届くため、IMC write とは別の検証形になる
   （送信直前のフォアグラウンド一致確認）。**この形が実際に守れるかは未検証**であり、
   守れないなら「open/close 軸は SendInput のためターゲット検証が構造的に不可能」と
   いう事実を INV-13 の例外として明記する（隠さない、ADR-084 INV-7 と同じ姿勢）。

**Phase 3 が実機で否定されても Phase 1+2 で conv 軸の実害は解決済みのため撤退可能である。**
この撤退可能性が段階分割の主目的である。

### Phase 4（INV-1/INV-19 のコンパイラ強制、低リスク・Phase 1〜3 完了後）

`runtime/conv_actuation.rs` の未移行リストが空になった時点で、低レベル conv write API を
`actuate_conv_mode` を持つモジュールの private にする（ADR-084 §6 段1 が予定していた強制）。
このリストが空になるまでこの強制は導入できない、と ADR-084 が既に明記している。

### revert する場合の義務

`.claude/rules/experiment-logging.md` に従い、本 ADR 由来の変更を revert するコミットは
本文に **アプリ / IME（種別と状態、`conv_mode_policy` の値）/ 再現手順と症状** を必ず
記載する。この領域は反転を繰り返しており、「なぜ前回それを捨てたのか」が辿れないことが
反転の最大の原因だった。

---

## 6. 強制メカニズム

`.claude/rules/ime-belief-architecture.md` 末尾の 3 段構え、および ADR-084 §6 の
4 段構えに倣う。同ルールの判断基準に従い、**dylint の新設は「型では防げない意味論的偽装」に
のみ投資する**。

### 段1: コンパイラ（最強、可能な限りここへ寄せる）

- **INV-14**: `set_ime_romaji_mode_with_target(_async)`（ライブクエリ版）を
  **削除**し、`set_ime_conv_for_target(target: ActuationTarget, ..)` のみを残す。
  「宛先を指定せずに書く」ことが型として書けなくなる。
  `ActuationTarget` のフィールドは private とし、`verify_still_current()` を
  通さずに `hwnd` を取り出せないようにする（`ForceGuardSet.guards` を private 化して
  `clear()` を唯一の口にしたのと同じ手法）。
- **INV-14（`Aborted` の握り潰し防止）**: `ActuationOutcome` を `#[must_use]` にする。
- **INV-12**: Phase 4 で低レベル API を `conv_actuation` モジュールの private にする。
- **INV-17**: force の目標値を `u32` の生値ではなく `ConvMode` で受け取り、
  `to_conv_bits()` を通す形に限定する（生の `u32` を force の目標に渡せなくする）。
- **INV-18**: `InputModeApplyStrategy` に `ForcePolicyConv` / `ForcePolicyResend` の
  新 variant を追加させる（既存の運用どおり）。

### 段2: dylint（HIR レベル、意味論的偽装の検出のみ）

**新規 dylint crate は原則作らない**。既存 crate の拡張のみ:

- `lints/observation_source_guard` を拡張し、
  **`InputModeApplied { strategy: ForcePolicy.., .. }` が `actuate_conv_mode` /
  `apply_force_*` の designated 関数以外で構築されたら warning**。
  `lints/ime_event_guard` が `PanicReset` / `HwndCacheRestored` を designated 関数に
  限定しているのと同型で、追加コストが小さい。**INV-12/INV-18**

### 段3: CI テスト（Linux で実行可能、`tests/architecture_guard.rs`）

既存の「テキスト走査による出現数固定」手法に倣う。

1. `conv_write_call_sites_are_target_explicit` — ライブクエリ版 conv write API の
   出現数を **0** に固定（Phase 1b 完了後）。それ以前は現在値 7 に固定して増加を検知。
   **INV-14/INV-19**
2. `force_write_is_not_triggered_by_raw_focus_change` — `platform.rs::gji_on_focus_change`
   および `FocusChange` ハンドラ群の本体に、conv write / IME 制御キー送信のシンボルが
   出現しないことを固定。**INV-15**
3. `unmigrated_conv_write_list_is_monotonically_decreasing` —
   `runtime/conv_actuation.rs` の module doc に列挙された未移行経路の数と、
   実際の直接呼び出し箇所数が一致することを固定。**INV-19**
   （`9c102b02` はリストに載せずに経路を増やした。このテストがあれば CI で落ちた。）
4. `force_policy_is_read_from_a_single_decision_point` — `ConvModePolicy::Force` の
   マッチ箇所が単一の判定関数に限られることを固定。**INV-13**
5. `actuation_completion_does_not_schedule_next_actuation` —
   `on_ime_apply_complete` の到達先に force 再送のシンボルが出現しないことを固定。
   **INV-16**

`tests/layer_boundary_guard.rs` の module doc の警告（「ルールを弱めないこと」）は
本 ADR のガードにもそのまま適用する。

### 段4: golden テスト + known-bugs 記録

`.claude/rules/fix-requires-evidence.md` の要求（回帰テストか known-bugs 記録の
少なくとも一方）を満たす。**本 ADR の対象領域は両方を要求する。**

- `tests/golden_scenarios.rs` に **force policy シナリオ**を追加:
  「`policy=Force` + FocusChange のみ（打鍵なし）→ conv write が **0 回**」
  「`policy=Force` + FocusChange → 打鍵 → conv write が **1 回**、以後の同一フォーカス内の
  連続打鍵では **0 回**」。**INV-15**
  既存のシナリオ15（`half_width_alnum_toggle` の belief 遷移）の隣に置ける。
- `tests/golden_scenarios.rs` に **ターゲット中止シナリオ**: 起案時と実行時で hwnd が
  変わったとき `Aborted` になり `applied` キャッシュが更新されないことを固定。**INV-14**
- `docs/known-bugs.md`: BUG-60（§5 Phase 0）に加え、BUG-59 追補のエントリに
  「この修正は ADR-086 Phase 0 で revert / 追認された」旨を追記する。

> **注意**: `gji_on_focus_change` は `spawn_local` を含む async orchestration であり、
> 既存にも単体テストが無い（`9c102b02` のコミット本文が明記）。**Phase 1〜3 の
> 非同期タイミング部分は golden で守れない。** この部分に限り、機械的強制の代わりに
> known-bugs.md への記録と実機ソークが防衛線になることを受け入れる。
> ただし **トリガー条件（どのイベントで発火するか）と `force_pending` の消費回数は
> 純粋なロジックとして切り出せる**ため、そこは必ずテストで固定する。

---

## 7. 未解決の論点

1. **「LINE で何を押しても『い』になる」の機構が未確定。** §1.3 のとおり、
   force-write 自体は ROMAN を落とせないため、JIS かな化との因果は現時点で
   説明できていない。BUG-60 として起票し、再現時のログ項目を先に決めておく
   （書き込み直前の hwnd / クラス名、書き込み後の conv 再読み、`[idle-conv-check]`
   の ROMAN 復元ログの有無、`[relay-passthrough]` の実 VK 列）。
   **原因が確定するまで、本 ADR の Phase 1〜3 が「この症状を直す」とは主張しない。**

2. **フォーカスが変わらないまま drift するケースをどう救うか。** 案 C は
   FocusChange を武装トリガーにするため、同じ窓に留まったままの drift を救えない。
   案 F（idle 明けの最初の入力）が第一候補だが、閾値の妥当性が未検証。
   案 D（低頻度の周期チェック）を分オーダーで別枠に置く選択肢も残るが、
   **INV-16 の自己駆動禁止を守れる形（外部周期にのみ従う独立予約）でしか採用しない**。
   間隔は実測に基づくこと。

3. **`ActuationTarget::verify_still_current()` のコスト。** `get_focused_hwnd()` は
   `get_gui_thread_info_with_timeout(30ms)` を含む。書き込みごとに 2 回呼ぶのが
   許容できるか、あるいは検証側は `GetForegroundWindow`（軽量）の比較で足りるかは
   **実測が要る**。`GetForegroundWindow` はトップレベル窓しか返さないため、
   UWP の InputSite 子窓の切り替わりを検知できない可能性がある —— この点は
   BUG-55 で既に問題になった論点であり、安易に軽量版へ倒さないこと。

   **調査結果（2026-08-08、実装着手前の設計調査、コード読解のみ・実測は別途要）:**
   この論点は「コストが許容できるか」だけでなく、そもそも**「起案時点で誰が
   hwnd を安全に捕獲できるか」自体が未解決**だったことが分かった。

   - `PlatformState`（`state/platform_state.rs::FocusStore` 他）を全数確認したが、
     「現在フォーカス中の HWND」を安く同期的に読めるキャッシュは**どこにも
     存在しない**。`FocusStore` は `focus_epoch`（世代カウンタ、INV-14 の時間軸
     フェンスと同型）・`app_kind`・`focus_kind` は持つが `HWND` 自体は保持しない。
     `focus/current.rs::CurrentFocus` も pid/class_name/profile のみで HWND を
     持たない。`ClassifiedFocus`/`FocusSnapshot` はフォーカス変化イベント処理中に
     一時的に hwnd を運ぶだけで、後から参照できる形では保存されない。
   - `set_ime_romaji_mode_with_target` 内の `get_focused_hwnd()`
     （`ime.rs:786`）は、6 経路すべてにおいて**既に** `spawn_local` +
     `offload_unsafe`（ワーカースレッド）の内側からのみ呼ばれている
     （`conv_actuation.rs`/`cold_warmup.rs`/`executor.rs`/`key_pipeline.rs` の
     いずれも、フック駆動の同期処理経路からは一度も直接呼んでいないことを
     `grep` で確認済み）。つまり**現状、この関数がフックスレッドを直接
     ブロックすることは無い**。
   - `get_focused_hwnd()` 自身が内部で `get_gui_thread_info_with_timeout`
     （= `win32_async::run_with_timeout`、さらに別のワーカースレッドへ
     offload してタイムアウト付きで待つラッパー、`win32.rs:139`）を呼んで
     いるため、**「起案時点」を「6経路の呼び出し元コードが実行される時点
     （多くはフック駆動の同期処理内）」まで早めて hwnd を捕獲しようとすると、
     現状ゼロだったフックスレッドの同期ブロックを新規に持ち込む**
     （BUG-34 の再発条件そのもの）。
   - 一方、`GetForegroundWindow()` 単体（`run_with_timeout` を挟まない軽量版）
     へ切り替えて起案時点に同期で呼ぶ案は、BUG-55（`known-bugs.md` 該当項）が
     「トップレベル窓基準の解決では TsfNative/InputSite 子ウィンドウの実際の
     ターゲットと食い違い、JIS かな入力ロックから復旧できなくなる」ことを
     実機で確定させているため**採用できない**。

   **結論（次段の実装方針）:** 「起案時点で同期的に安く hwnd を得る」という
   前提自体を諦める。代わりに:
   1. `ime.rs` に `get_focused_hwnd_async()`（`get_focused_hwnd()` を
      `offload_unsafe` で包んだだけの薄い async 版）を新設する。
   2. `ActuationTarget`（フィールド private）に
      `pub(crate) async fn capture() -> Option<Self>` を生やし、内部で
      `get_focused_hwnd_async()` を呼ぶ。6 経路それぞれの `spawn_local`
      ブロックの**先頭**（他の `.await` より前）でこれを呼び、以後の処理
      （`unconfirm()` 呼び出し・実際の IMC write）に運ぶ。「起案からブロック
      までの間隙をゼロにはできないが、既存の 50ms の conv 事前読み取り等を
      挟まず最短経路にする」ことで §1.2 欠陥1 の間隙を縮める。
   3. `verify_still_current()` は **同期関数ではなく非同期関数**にする
      （ADR §2.3 P7 の擬似コードは同期シグネチャで書いたが、上記の理由により
      実装時は `pub(crate) async fn verify_still_current(self) -> Option<HWND>`
      へ修正する）。内部で `get_focused_hwnd_async()` をもう一度呼び、
      `self.hwnd`・`self.focus_gen` の両方と突き合わせる。
   4. **配置先は `ime.rs`。** `ActuationTarget`/`get_focused_hwnd_async`/
      `verify_still_current` はいずれも Win32 hwnd 解決の詳細に密結合するため、
      既存の `get_focused_hwnd`/`set_ime_romaji_mode_with_target` と同じ
      ファイルに置く（`ConvModeTarget`/`ConvActuationOutcome` が `state::conv_mode`
      に、それを使う `actuate_conv_mode` が `runtime::conv_actuation` にある、
      という既存の「型は state 系、実行は runtime/ime 系」という分離パターンに
      倣う）。`runtime/conv_actuation.rs` 等の呼び出し元は
      `crate::ime::ActuationTarget::capture().await` を呼ぶだけでよく、
      hwnd 解決の詳細を知る必要はない。
   5. §17（実測ゲート）で測るべき数値はこの結論を前提にする: (a)
      `ActuationTarget::capture()` から実際の IMC write までの経過 ms、(b)
      その間の hwnd 不一致率、(c) `verify_still_current()` を追加すること
      自体のオーバーヘッド（現状より Win32 呼び出しが 1 回増える）。

4. **ADR-078 との統合順序。** ADR-085 の `desired_mode` は ADR-078 の `DesiredMode` の
   先行実装に相当する。本 ADR の Phase 2 は `desired_mode` の**消費点**を整理するが、
   型分割そのものには踏み込まない。ADR-084 §7-6 が「P1（`actuate_conv_mode`）の先行を
   推奨する」と結論しているのと同じ理由で、**本 ADR も ADR-078 の全面実装を待たない**。

5. **`conv_mode_policy` という設定名が 2 軸を制御している。** ADR-085 の追記により、
   この設定は conv 軸と open/close 軸の両方の force を同時に切り替える。
   INV-13（軸の対称性）はこれを積極的に肯定する立場だが、
   **ユーザーから見て「conv mode の設定」が IME の開閉挙動を変えるのは説明が難しい**。
   設定名を `force_policy` 等へ改名するか、軸ごとに分けるかは本 ADR では決定しない
   （config.toml の互換性を壊すため、独立した判断が要る）。

6. **`conv_mode_policy` に第 3 の値（`manual`）を足すか。** 案 E（明示操作のみ）は
   自動回復を放棄する代わりに完全に安全である。案 C が実機で否定された場合の
   退避先として意味があるが、値を増やすと組み合わせ検証の負荷も増える。
   Phase 2 の実機ソーク結果を見てから判断する。

7. **`desired_mode` の永続化。** ADR-085 §未対応のとおり、`desired_mode` はプロセス
   再起動でデフォルト（全角ひらがな）に戻る。force policy がトリガー起点で
   確実に発火するようになると、「再起動したら勝手にひらがなへ引き戻された」という
   新しい体感が生まれうる。本 ADR では決定しないが、Phase 2 の実機ソーク時に
   ユーザー体感を確認する項目に含めること。

8. **`Aborted` が高頻度で出た場合の解釈。** Phase 1a の実測で「起案と実行で宛先が
   ずれる」割合が高いと分かった場合、対策は 2 方向ある: (a) 起案から実行までの
   遅延を縮める（`offload` の見直し）、(b) そもそも起案を遅らせる（案 C）。
   **本 ADR は (b) を選んでいる**が、(a) が必要なほど遅延が大きい（例: 100ms 超）なら
   `offload_unsafe` の設計自体を再検討する必要がある。数値は実測待ち。

---

## 8. 関連

- **ADR-084**: conv-mode の単一所有権と幅の SSOT（**本 ADR の直接の親**。INV-1/INV-2 を
  継承し、INV-12 から採番する。ADR-084 §5 Phase 1 の実測義務は本 ADR Phase 2 と同一）
- **ADR-085**: `conv_mode_policy = force`（**本 ADR が規律を与える対象**。目標値
  `desired_mode` は維持し、いつ・どこへ・どの窓口で書くかを本 ADR が定める。
  同 ADR の 2 つの追記が open/close 軸へ拡張した force は INV-13 の対象）
- ADR-064: `ConvModePolicy` による conv mutation ゲート（許可の明示化。force-write も
  `conv_mutation_allowed` を尊重する）
- ADR-072: `conv_mode_authority` を apply 完了ごとに再同期（**遷移エッジ依存の誤りの
  先例。ADR-084 §1.3 が 2 例目、`9c102b02` の生 FocusChange トリガーが 3 例目**）
- ADR-078: IME mode belief の Desired/Effective/Constraint 分割（`desired_mode` は
  `DesiredMode` の先行実装に相当。統合順序は §7-4）
- ADR-080/082: actuation ライフサイクルと epoch fencing / ジャーナル
  （**INV-14 は epoch fencing を空間軸（hwnd）へ拡張したもの**。`Aborted` の記録先）
- ADR-083: `InjectionMode` per-VK 統一の検討（**NO-GO**。`9c102b02` が
  `conv_mode_policy = force` の出典として誤って参照していた番号）
- ADR-033: `AppImeProfile`（`Standard`/`Imm32Unavailable`/`TsfNative`）—— force の
  必要性が profile ごとに異なる根拠
- `docs/known-bugs.md`:
  BUG-08（外部注入 `VK_KANA` による JIS かな化、INV-17 の発端）、
  BUG-13（MS-IME cold-start・confirm-then-transmit ゲート、`needs_f2_probe()==false` の起点）、
  BUG-19（観測追従型の自己増幅ループ、ADR-085 が回避対象とした構造）、
  BUG-25（GJI に conv entry 機構が無い、IME 種別の非対称）、
  BUG-34（フックスレッドをブロックできない制約、案 T5 却下の根拠）、
  BUG-49（async unconfirm が送信ゲートを素通しさせる、INV-12 が同期呼びを要求する根拠）、
  BUG-50（conv 帰属の欠落、ADR-084 INV-11）、
  BUG-52（物理キー漏洩、ADR-085 の発端）、
  BUG-55（`ImmGetDefaultIMEWnd` の hwnd ターゲット問題、§7-3 の関連論点）、
  BUG-59 とその追補（**本 ADR の発端**）、
  BUG-60（本 ADR Phase 0 で起票する未確定バグ）
- `.claude/rules/ime-belief-architecture.md`（Observe → Pure → Apply の語彙。
  force-write は Observe に依存しない Apply）/ `experiment-logging.md`（revert 本文の義務）/
  `tuning-constants.md`（Phase 1a/2 の実測義務）/ `fix-requires-evidence.md`（段4）
- `lints/observation_source_guard`（段2 の拡張先）、`lints/ime_event_guard`（同型の先例）
