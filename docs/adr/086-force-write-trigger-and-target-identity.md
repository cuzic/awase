# ADR-086: force-write の単一規律 — 「観測を信じない書き込み」のトリガー条件と書き込みターゲット同一性

## ステータス

**Phase 2（conv 軸）・Phase 3（open/close 軸）は
[ADR-094](094-charset-axis-and-force-policy-removal.md)（2026-08-17）で全撤去。**
`conv_mode_policy` 自体を撤去したのに伴い、`force_pending`/`force_open_pending`
とその武装・消費機構（`arm_force_open_pending`/`consume_force_open_pending`/
`consume_force_pending_and_actuate`）が全て消えた。**Phase 0〜1（INV-14
ターゲット同一性、`ActuationTarget`）は撤去していない**——`actuate_conv_mode`
が `ConvModeTarget::HalfWidthAlnum`（shift-conv-guard 用）の書き込みで今も
使っているため、本 ADR は完全な廃止ではない。以下は Phase 2/3 実装当時の
記録として残す。

北極星仕様。ADR-084 の姉妹編（invariant 番号空間を共有する）。
Phase 0〜1（記録・INV-14 ターゲット同一性の全経路移行）、Phase 2
（conv 軸の INV-15 是正、`force_pending` による arm-on-focus /
fire-on-intent）、Phase 3（open/close 軸への同適用、`force_open_pending`。
item 3 の SendInput ターゲット検証のみ INV-13 の例外として撤退）は
実装完了（2026-08-08）。~~**item 0（`ime_controller.rs` の同期ライブクエリ
IMC write）は未移行のまま記録のみで item 1 を先行投入した**~~——当初
「item 3 着手前に必須」としていた前提を満たせていなかった。この同期 IMC write
（実測根拠は `tuning.rs` の導出コメントより最大 ~100ms）は force-ON 発火の
たびに打鍵ホットパスに乗る（§5 Phase 3 item 0 参照）。

**追記（2026-08-12）: item 0 は
[ADR-089](089-ime-typestate-and-capability-const-table.md) の Phase C item 12 で
是正済み。** `ImmCrossProcessStrategy::apply` / `MsImeDirectStrategy::apply` の
`set_ime_romaji_mode()` を撤去し、`ime_controller::apply_mechanism` の
ROMAN 補完ステップ 1 箇所へ統合したうえで
`ActuationTarget::capture_blocking` → `set_ime_romaji_mode_for_target_blocking`
経由にした（ライブクエリ版の低レベル API `set_ime_romaji_mode` / `_async` は
削除）。**「`ImeOpenStrategy::apply` 自体の非同期化が要る」という本 ADR
Phase 3 の判断は、`verify_still_current` の hwnd 再クエリまで必須と読んだ
場合にのみ正しかった**——捕獲そのものは `get_focused_hwnd()` 1 回であり、
旧 `set_ime_romaji_mode()` が内部でやっていたライブクエリと同一なので、
同期のまま捕獲を write の外へ出せる。再検証は focus 世代の照合のみで行う
（同期経路には await 点が無く hwnd が動く余地が構造的に無いため。
Win32 往復の回数は 1 回のまま＝ホットパスのレイテンシも変えていない）。
**ただしその世代照合は現状では恒真であり、stale target への write を実際に
検出する力は無い**——捕獲と write に同一の `focus_gen` を渡しているため
（ADR-089 §9-22 の訂正、2026-08-12）。**INV-14 のうち同期 ROMAN 補完で
達成できたのは「宛先を write 関数が自己決定しない（捕獲点が 1 箇所に固定され
ログに残る）」までであり、「起案時と実行時のターゲット同一性を検証する」まで
ではない**。

**残る穴**: 同期 ImmCross の open write（`set_ime_open_cross_process`）は
依然として自分でライブクエリする（ADR-089 §9-18）。**この経路は
`runtime/mod.rs::try_force_on_bootstrap` から現に到達する**——当初は
「全呼び出し元の追跡で到達しないことを確認済み」と書いていたが、それは
同関数を数え落とした誤りだった（ADR-089 §9-21 の訂正、2026-08-12）。
したがって Standard（LINE / Qt 等）での bootstrap force-ON では ROMAN 補完と
open write が別ウィンドウへ着弾しうる（§1.2 欠陥1 と同型）。**ADR-089
Phase C 以前から同じ挙動であり新規の回帰ではない。** **実機ソーク未実施**
（確認項目は ADR-089 §9-17、この経路は 17-h）。

Phase 2/3 とも
実機ソーク未実施（測定項目は §5 Phase 2/Phase 3、タスク #17 と同一
セッションで実施予定。Phase 3 のソークは #17 の**後に**別セッションで行い、
conv 軸/open 軸の副作用を切り分けること）。Phase 4（コンパイラ強制）は
未着手。

**2回目 opus アドバーサリアルレビュー（2026-08-08）で Phase 3 実装に
High 2件・Medium 5件・Low 4件、さらにレビュー中に新規 2件（force-ON が
idle-conv-check の汚染防止ガードを素通りする／`ObservedKana` 保護が
force-ON 経路では常に無効）を検出、順次修正中。詳細は §7-11。**
いずれも Windows 実機での動作確認は未実施。**

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
| open/close force-write の実行 | **既存の apply 経路（`apply_ime_open_with_belief` → `on_ime_apply_complete`）**。force であることを `OpenApplyReason::ForcePolicyResend` で申告する（§5 Phase 3 item 2） | force 判断・レート制限・再スケジュールを `apply_force_on_for_imm_broken` の関数本体に埋め込むこと |
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
  **同一の規律に従う**。具体的には (a) 同じトリガー判定（INV-15、`force_pending`/
  `force_open_pending` の武装＝FocusChange・消費＝入力意図という同型の2段構え）、
  (b) 同じターゲット規律（INV-14）、(c) 同じ記録先（軸ごとに専用の型
  ——conv 軸は `ConvMutationReason`、open/close 軸は `OpenApplyReason`
  ——を使うが、いずれも「force か観測是正か」がログ・ジャーナルから
  一意に読める、という記録先の**規律**は同一。既存の `InputModeApplyStrategy`
  や ADR-080/082 ジャーナルへ両軸を無理に相乗りさせるという意味ではない
  ——Phase 3 item 2 の訂正経緯、§7-10 参照）、(d) 同じ `ConvModePolicy` 設定を
  単一の判定関数（`Output::is_force_policy()`）から読む。
  片方の軸にだけガードや安全弁を追加してはならない —— 追加する場合は他方にも
  必要かを検討し、不要と判断した理由をコード内に残す（ADR-084 INV-7 と同型の対称性要求）。
  **例外（Phase 3 item 3 で確定、2026-08-08）**: (b) のターゲット規律は
  `SendInput` を使う書き込み（`ime.rs::send_ime_mode_key` 等）には構造的に
  適用できない——`SendInput` は宛先 hwnd を指定するパラメータを持たず、
  `ActuationTarget`（IMC write 向けに「特定の hwnd への書き込み」を検証する
  ために設計されたパターン）を持ち込む対象が無い。この経路は時間軸フェンス
  （`ime_mode_focus_gen` の照合）のみで代替する。IMC write（`set_ime_romaji_mode()`
  等）は例外の対象外——SendInput と同じ呼び出しチェーンに紛れていても
  (b) の対象のまま（詳細は §5 Phase 3 item 3）。

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

**INV-20 以降は [ADR-087](087-open-belief-actuation-warrant-separation.md) で
採番する**（2026-08-10 追記）。ADR-087 は本 ADR が定めなかった第4の軸
（根拠軸 — どれだけの証拠に裏付けられて発火してよいか）を扱い、
`consume_force_open_pending` の eligibility 判定が本 ADR §2.1 の force-write
定義（観測を判断材料にしない）に実装として一致していない点を是正する。

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
   **追補（2026-08-08、2回目 opus レビュー F2）:** 上記6経路の洗い出し自体に
   漏れがあり、`key_pipeline.rs::apply_focus_probe` 内の `ImmCrossProbe`
   かなモード補正書き込みが未移行のまま残っていた（実質 7 経路目）。
   `ActuationTarget::capture` を先頭 await に置く専用の `spawn_local` へ
   切り出し、同様に移行済み（`docs/known-bugs.md` BUG-49 追補4 参照）。
6. ✅ 完了（2026-08-08）: 全経路の移行が済んだため
   `set_ime_romaji_mode_with_target(_async)`（ライブクエリ版）を**削除**した
   （§6 段1 のコンパイラ強制の前段）。`tests/architecture_guard.rs` に
   `actuation_target_capture_call_sites_are_accounted_for` を新設し、
   `ActuationTarget::capture` の呼び出し箇所数（7、上記追補後）を固定することで
   INV-19 の「未追跡の新規経路を検知する」役割を引き継いだ。

### Phase 2（INV-15: トリガーを arm-on-focus / fire-on-intent へ、中リスク）

**状態（2026-08-08）: 実装完了（opus アドバーサリアルレビューによる設計是正を反映）。
実機ソーク（下記）は未実施。**
下記は当初案からの訂正を反映した確定版。§7-9 に訂正の経緯を残す。

0. **`actuate_conv_mode` を `Runtime` から `Output` へ移設する**（`output/conv_actuation.rs`
   新設）。当初「`Output` 層（武装点・消費点候補）と `Runtime` 層（`actuate_conv_mode`）の
   非対称性をどう橋渡しするか」を論点としたが、調査の結果 `actuate_conv_mode` の同期部は
   `self.platform.output.*` しか読んでおらず（`with_app` を呼ぶのは `spawn_local` 後の
   非同期部のみ）、**実体は既に `Output` のメソッドだった**。非対称性は実在しない。
   `Runtime::actuate_conv_mode` は `self.platform.output.actuate_conv_mode(..)` への
   1 行 delegate として残す（ADR-084 INV-1 が指す関数名を変えないため、および
   `key_pipeline.rs::kp_shift_conv_guard_key_down` 等の既存呼び出し元を触らないため）。
1. `ConvMutationReason::ForcePolicy` と `ConvModeTarget::Desired(ConvMode)`
   （**`Explicit(u32)` ではない** — INV-17「生の `u32` を force の目標に渡せなくする」と
   直接矛盾するため。`imm_conv_value()` 内で `ConvMode::to_conv_bits()` を通す。
   `to_conv_bits` は `const fn` のため `imm_conv_value` の `const` 指定は維持できる）
   を追加し、`actuate_conv_mode` が force の目標値を書けるようにする（INV-12）。
   ADR-084 INV-2 に従い `unconfirm()` を同期的に呼ぶ。
2. `force_pending: Cell<Option<u32>>`（1 回分の武装フラグ。`Some` の中身は武装時点の
   `ime_mode_focus_gen`）を `Output` に新設する。武装点は `Output::on_ime_mode_focus_changed`
   （`output/mod.rs:391`、フォーカス系の単一集約点。呼び出し元は `platform.rs:455` の
   1 箇所のみ）とする。**生の `FocusChange` イベントハンドラ（`platform.rs::gji_on_focus_change`）
   に直接書いてはならない** — `tests/architecture_guard.rs` の
   `force_write_is_not_triggered_by_raw_focus_change` がその関数本体を走査する設計であり、
   武装コード自体をそこに置くと将来の書き込みロジック追加時に誤って引っかかる／
   検知をすり抜ける双方のリスクがある。`Output` にはフォーカス関連のもう 1 つの
   契機（`CompositionOutput::on_focus_changed`、`output/mod.rs:597`）が存在するため、
   採用しなかった方をコメントで明示的に否定しておくこと（bool 二重武装は無害だが、
   片方しか発火しない契機を見落とすと force が黙って発火しなくなる）。
3. 消費点を **消費判断を行う関数 1 箇所**に集約する: `Output::send_romaji`
   （`output/mod.rs:1040`、`InjectionMode::Vk`/`Tsf`/`Unicode` すべての合流点かつ
   `vk_send.rs` 内のどの早期 `return`（`prepend_f2_warmup` 分岐等）よりも前）。
   **当初案の「`ms_ime_gate_defer` / `defer_vk_if_probe_in_flight` の手前」は不採用**
   —— 理由は3つ。(a) `ms_ime_gate_defer` の呼び出し箇所自体が2箇所
   （`vk_send.rs:214`/`:324`）あり「1箇所」の要求に反する。(b) 両方とも
   `prepend_f2_warmup` 分岐の `return` の後ろにあるため **cold パスでは到達しない**
   —— cold パスこそ次項4が `cold_warmup.rs` から force を移してくる先であり、
   この位置では item 3 と item 4 が同じフラグを共有できず二重発火防止が働かない。
   (c) `defer_vk_if_probe_in_flight` の呼び出し元は BUG-47 追補で既に死にコード化して
   いる（同関数の doc コメント参照）。`Output::send_romaji` なら `InjectionMode::Unicode`
   （WezTerm 等、`ms_ime_gate_defer` を元々呼ばない）も含め確実に1回通る。
   `send_kana_char`（記号入力、`output/mod.rs:1048`）も消費点として扱う（推奨: 消費する
   — 記号打鍵も入力意図であることに変わりはない）。この場合「消費判断を行う関数
   （`consume_force_pending_and_actuate`）は1つ、呼び出し箇所は2つ」という形になる。
   `Output::flush_raw_tsf_literal_romaji`（`output/mod.rs:1107`）は意図的に対象外
   —— 既に打った文字の再送であり、force を再発火させてはならないため。
   コード側に「意図的に経由しない」旨を明記し、後日「経路漏れ」として誤って
   配線されないようにする。
4. `cold_warmup.rs::run_start` の `forced_target` の **`ConvModePolicy::Force` アームのみ**
   削除する。**`Observe`（デフォルト設定）アームの `None` 書き込みは削除しない** ——
   これは force ではなく BUG-19 由来の「ROMAN ビット確保のみ」の observation-based な
   保護であり、当初案の「`forced_target` を移す」という文言をそのまま丸ごと削除と
   読むと全ユーザー（force 未使用者を含む大多数）からこの保護が失われる。
   `Force` アーム削除により `needs_f2_probe()` への依存が消え、案 B の MS-IME の穴が
   塞がる。**実装時の訂正（2026-08-08）**: 当初「`ConvModePolicy::Force` を直接読む
   箇所が3→2箇所に減る」と見込んでいたが、`cold_warmup.rs` の読み取りは
   `on_ime_mode_focus_changed`（item2 の武装点）の読み取りに置き換わっただけで、
   実際の読み取り箇所数（`output/mod.rs` の武装点 + `runtime/mod.rs`×2 の
   open/close 軸）は3箇所のまま変わらない。§6段3-4 の
   `force_policy_is_read_from_a_single_decision_point` への接近は Phase 3
   （open/close 軸を同じ `force_pending` 機構へ統合したとき）まで持ち越しとする。
5. **再武装（当初案に無かった追加項目、opus レビューで必須と判明）**:
   `force_pending` は同期的に消費（`None` へ）するが、実際の書き込みは
   `actuate_conv_mode` 内の `spawn_local` 内で非同期に行われ、`ActuationTarget::capture`
   が `None`（フォーカス無し）を返す、または `set_ime_conv_for_target` が
   `Aborted{TargetMoved|GenStale}` を返すことがある（Phase 1 が導入した経路であり
   実際に到達可能）。これらの場合、**消費済み・未書き込み**のまま次の `FocusChange`
   まで force が永久に発火しない穴ができる。async 完了時に capture 失敗/Aborted なら
   再武装すること。再武装は INV-16（自己駆動の禁止）には抵触しない —— 次の
   actuation を無条件に予約するのではなく、次の**入力意図**があったときにのみ発火する
   状態に戻すだけであるため（この理由はコードコメントに残し、後日 INV-16 違反として
   誤って削除されないようにする）。ただし同一 `focus_gen` 内での
   arm↔abort ピンポンを防ぐガードが要る —— `force_pending` を `Cell<Option<u32>>`
   （武装時の gen を保持）にしてあるのはこのため。abort 時の現在 gen が武装時の gen と
   同じなら再武装しない、変わっていれば（＝別の正規の FocusChange 武装が既に起きている）
   何もしない。

**`Rejected` の扱い**: `ConvActuationOutcome` を `let _ = ` で捨てないこと。
`Rejected`（`conv_mutation_allowed=false`、エンジン OFF 中等）は「そもそも書くべきで
ない」状態なので消費済みのまま再武装しない、と `Aborted`（書きたかったが書けなかった）
から明示的に区別してログすること。

**実測義務（訂正）**: 当初「最初の 1 文字が force の完了を待つべきか」を決めるための
新規実測が必要としていたが、**この設計では新しいタイミング定数を1つも導入しない**
（Phase 1a→1b で採った論法と同一、`.claude/rules/tuning-constants.md` に抵触しない）。
理由: `actuate_conv_mode` は同期的に `ime_mode_fsm.unconfirm()` を呼び、
`Output::ms_ime_gate_defer`（`output/vk_send.rs:356`）は `fsm.is_native_ready()` が
真のときだけ送信を素通しする既存の confirm-then-transmit ゲートを持つ。
force を発火させると、その直後の打鍵は自動的にこのゲートで待たされ、IMC read で
確認が取れた時点で解放される —— 「N ms 待つ」ではなく「読めるまで待つ」設計であり、
新しい待機セマンティクスを持ち込まない。この経路は `kp_shift_conv_guard_key_down` →
`actuate_conv_mode` で**今日既に動いている**。`FORCE_CONV_CONFIRM_MS` のような定数が
必要だと感じた時点で実装を止め、実機セッションを待つこと —— それこそが「実測が
本当に要る」ことのシグナルである。

ただし副作用として、`actuate_conv_mode` は `ms_ime_gate_give_up.set(false)` で
give-up ラッチを解除する。IMC が構造的に読めないアプリではこのラッチが「もう待たない」
という短絡を担っており、Phase 2 後は **フォーカス変更のたびに force が発火 → give-up
解除 → 最初の打鍵が最大 `MS_IME_READY_CONFIRM_MS`（400ms）待たされる**という経路が
フォーカスを変えるたびに繰り返されうる（現状この経路が起きるのは shift 単独タップ時
だけなので、頻度が桁で変わる可能性がある）。これは Phase 2 の実機ソーク（下記）で
必ず測定する。悪化が確認された場合の対策候補は「`ForcePolicy` reason のときは
give_up を解除しない」だが、これは INV-13（軸対称性）の観点で「なぜ他の reason には
不要か」をコードに残す必要がある片方向ガードになる。

**却下した代替案（outbox）**: `force_pending` の消費・実書き込みの起動に
`runtime/outbox.rs::RuntimeRequest` を使う案を検討したが却下した。
`Runtime::drain_runtime_requests` は `WM_EXECUTE_EFFECTS`/`WM_DRAIN_OUTPUT_QUEUE` の
**末尾**（`runtime/message_handlers.rs:265`/`:762`）で呼ばれ、つまりキーが**既に送出
された後**である。outbox 経由では「最初の1文字を送ってから conv を書き換える」という、
Phase 2 が正すべき順序の正反対になる。

**実機ソーク（Phase 2 完了後、タスク #17 と同一セッションで実施）**: 測定項目は
(a) force の発火頻度、(b) `ActuationTarget` の `Aborted` 率（§7-8 の「100ms 超なら
offload 設計を再検討」の判断材料）、(c) フォーカス後最初の打鍵の追加レイテンシ
（上記 give-up 解除の副作用）、(d) LINE × MS-IME × force で BUG-60 が再現するか
（§7-1、Phase 2 は MS-IME で force-write が発火する最初の実装になるため、BUG-60 の
唯一の現実的な再現機会でもある）、(e) `desired_mode` 永続化のユーザー体感（§7-7）、
(f) ADR-084 Phase 1 の実測（IMC write 完了→IMC read で目標 conv 確認までの ms、
Phase 2 の実装自体には不要になったが ADR-084 側の義務として残る）。

**Phase 2b（案 F、Phase 2 の実機ソーク後）**: idle 明けの最初の入力を追加の
武装トリガーにする。閾値の妥当性（`COMPOSITION_TIMEOUT_MS` を流用してよいか）を
実測してから。

### Phase 3（INV-13/INV-16: open/close 軸を同じ規律へ、中リスク）

**状態（2026-08-08）: 実装着手前の設計調査完了（opus アドバーサリアルレビュー）。**
下記は当初案からの訂正を反映した確定版。§7-10 に訂正の経緯を残す。

**前提として確定した事実（コード読解）:**

- `apply_force_on_for_imm_broken` の呼び出し元は `runtime/ime_refresh.rs::ir_stage_notify`
  （周期リフレッシュ連鎖の末尾）**1 箇所のみ**。これは INV-15 が禁止する「生の周期
  タイマー」トリガーそのものである。
- open 軸には conv 軸（Phase 2）と同じ confirm-then-transmit ゲートが**既に配線
  済み**: `platform.rs::on_ime_applied` が `Applied`/`FallbackSent` のとき
  `ime_mode_fsm.on_set_open_applied(open)` で belief を unconfirm し、以降の送信は
  `Output::ms_ime_gate_defer`（BUG-13）が `is_native_ready()` で自動的に待つ。
  よって Phase 3 も Phase 2 と同じ論法（新規タイミング定数ゼロ）が使える。
- TsfNative（Blacklist）環境は IMC が構造的に読めず、`ir_apply_drift_correction`
  （観測ベースの是正）は観測が存在しないため発火できない。**open 軸の TsfNative
  では、周期 force-ON が唯一の自己回復手段**であり、これを単純に「入力意図待ち」
  へ倒すと、周期 poll の谷間で実 IME が OFF に落ちたケースを打鍵まで検知できない
  空白期間が生まれる（§7-2 参照、conv 軸より実害が具体的）。
- `apply_ime_open_with_belief` → `ime_controller::CONTROLLER.apply` の呼び出し
  チェーンは**完全に同期**（`spawn_local` を使わない）。Phase 1/2 が非同期
  `ActuationTarget::capture` を要した理由（BUG-34、フックスレッドのブロック禁止）は
  フックスレッド固有の制約であり、ここはメインスレッド（メッセージループ）上の
  呼び出しのため直接は当てはまらない。
- `ime_controller.rs` の `ImmCrossProcessStrategy::apply`（L72）と
  `MsImeDirectStrategy::apply`（L175）が **`crate::ime::set_ime_romaji_mode()`
  （宛先をライブクエリで自己決定する同期 IMC write）を呼んでいる**ことが本調査で
  判明した。これは INV-14 未移行かつ `conv_actuation.rs` の未移行リストにも
  `architecture_guard` の出現数固定にも載っていない——**INV-19 違反**（未追跡の
  新規経路）でもある。Phase 1〜2 の「7 経路」の数え漏れであり、item 0 として
  独立に処理する（下記）。

0. **`ime_controller.rs:72`/`:175` の `set_ime_romaji_mode()` を INV-14/19 準拠へ**
   （item 3 の着手前に必須）。`ActuationTarget` 経由へ移行するか、移行が難しい
   場合は `runtime/conv_actuation.rs` の未移行リストへ明記して INV-19 を満たす。
   これを放置したまま force を打鍵時トリガーへ移す（item 1）と、この同期 IMC
   write が打鍵ホットパスに載り BUG-34 の再発条件になる。
1. `apply_force_on_for_imm_broken` の force 分岐を、関数本体に埋め込まれた
   レート制限（`last_force_on_resend_ms`）ごと、**専用の武装フラグ**
   `Runtime::force_open_pending: Option<u32>`（武装時の `ime_mode_focus_gen`）へ
   移す。周期トリガー（案 D）から入力意図トリガー（案 C）へ変える——これは
   「呼び出し元（`ir_stage_notify` の周期）を残したまま武装/消費だけ分離する」
   のではなく、**実際の書き込みが起きる場所自体を周期リフレッシュから
   打鍵イベントへ移す**ことを意味する（そうでなければ消費点も周期のままで
   INV-15 を満たさない）。
   - **Phase 2 の `Output::force_pending`（conv-mode 専用）とは統合しない**。
     層が異なる（open 軸は `Runtime::on_ime_apply_complete`（`&mut Runtime`、
     record_ime_apply_result + post_ime_refresh + on_ime_applied の SSOT）を
     必ず経由する必要があり、`Output::send_romaji`（`&self`、`with_app` の
     内側）から `with_app` を再度呼ぶと再入する）。消費すべきタイミングも違う
     （conv は「文字を出す瞬間」でよいが、open は VK_IME_ON が romaji VK より
     先に届く猶予を作るため「キーが届いた瞬間」まで前倒しする必要がある）。
     再武装のセマンティクスも違う（open の apply は完全同期で `Aborted` 概念が
     なく、代わりに既存の `ImeOpenOutcome::UnsafeToToggle` が「送らなかった」を
     表す）。INV-13 が要求する対称性は「同じ変数」ではなく「同じ規律」——
     (a) 同じ policy 判定関数（`is_force_policy()`、item 0.5 相当。
     `output/mod.rs::on_ime_mode_focus_changed`・
     `runtime/mod.rs::apply_force_on_for_imm_broken`・`::reschedule_ime_refresh`
     の3箇所の `ConvModePolicy::Force` 直接読みをこれへ集約し、§6段3-4
     `force_policy_is_read_from_a_single_decision_point` を満たす）、
     (b) 同じ arm-on-focus 契機（`ir_notify_focus_changed`。
     `platform.rs::gji_on_focus_change` には書かない——`architecture_guard::
     force_write_is_not_triggered_by_raw_focus_change` の走査対象に
     `ir_notify_focus_changed` を追加し、「武装のみ許可」を機械的に固定する）、
     (c) 同じ provenance 記録（item 2 参照）、で満たす。
   - 消費点は **`runtime/key_pipeline.rs::kp_run_inner`**（`&mut Runtime` を持つ
     全キーイベントの唯一の入口）。`try_hold_key`（TsfGate hold）の早期 return と
     ime-off-rescue のネスト再入（`skip_rescue_defer=true`）**より後**、
     `kp_stage_focus_probe` → `kp_stage_idle_conv_check` →
     `kp_stage_shadow_ime_toggle` の**後**、`build_input_context` **より前**に
     置くこと（**訂正、2026-08-08 2回目 opus アドバーサリアルレビュー**:
     当初は「`try_hold_key`/ime-off-rescue より後」とだけ書いていたが、それだけ
     だと `kp_stage_focus_probe` より前に置いてしまい、`kp_stage_focus_probe` が
     one-shot で消費する `input_barrier` が未消費のまま `ime_apply_should_defer`
     が真になり続け、**フォーカス変更後の 1 打鍵目を必ず取りこぼして 2 打鍵目で
     発火する**という、Phase 3 が解決しようとした症状をそのまま 1 打鍵分だけ
     再現するバグを生んだ）。
   - **`ime_apply_should_defer()`（settle ガード）は消費点では呼ばない**
     （**訂正**: 当初はこのガードを流用する想定だったが、`kp_stage_focus_probe`
     の後ろでは `input_barrier` が既に消費済みのため settle 判定が構造的に
     常に false になり「ガードを呼んでいるつもり」で実質何も守っていない
     死んだ条件になる）。代わりに、消費条件そのものを「本物の入力意図」に
     直接紐づける: `KeyDown` かつ `!event.injected`（BUG-14: MS-IME が
     毎打鍵で注入する `VK_KANA` 等を除外）かつ Ctrl/Alt/Win 修飾キー非押下
     （Alt+Tab 等のショートカットは text-input 意図ではない）かつ、その
     キー自体が IME モードキー（sync/shadow アクション対象）ではないこと。
     settle ガードが本来防いでいた「フォーカスがまだ確定していない中間
     ウィンドウ（`XamlExplorerHostIslandWindow` 等）への誤射」（2026-07-05
     修正）は、この「打鍵が来た＝ユーザーが打とうとしている窓が確定して
     いる」という直接判定で代替する。
   - **force-ON は `Output::note_explicit_ime_action` を呼ぶこと**
     （**追加、2回目レビュー新規指摘**）。`kp_stage_idle_conv_check` の
     3 つの汚染再検証ガード（shift ガード・`last_explicit_ime_action_ms`
     一致・`last_send` 一致）は、awase 自身が能動的に IME へ書き込んだ
     ことをこのフラグで判定する。force-ON の同期 IMC write を呼ばずに
     いると、Phase 3 で `kp_stage_idle_conv_check` の隣（同一イベント内）に
     移動した結果、force-ON 自身の書き込みが「外部観測」として idle-conv-check
     に誤読される衝突が構造的に起きる。
   - **`ObservedKana` 保護を効かせること**（**追加、2回目レビュー新規指摘**）。
     `apply_ime_open_with_belief(true, None, belief)` は内部で
     `belief_input_mode: InputModeState::Unknown` 固定の view を作るため、
     `MsImeDirectStrategy`/`ImmCrossProcessStrategy` の「ユーザーが意図的に
     かな入力を選んでいれば romaji 復元で上書きしない」ガードが force-ON
     経路では一度も効かない。`Runtime::shadow_ime_control_view()` 相当
     （`belief_input_mode = input_mode()`）を使う経路に変更すること。
   - 消費直前の1点で `ime_mode_focus_gen` を照合する（**訂正**: 当初「消費直前と
     apply 直前の2点で照合」としていたが、`ime_mode_focus_gen` を進める唯一の
     経路の唯一の呼び出し元が武装の直前1行であるため、`armed_gen != current`
     は構造的に到達不能——武装される瞬間＝gen が進む瞬間なので、2点目の照合は
     常に1点目と同じ結果にしかならない。実装は1点のみの照合で十分であり、
     これは「将来この経路が非同期化された場合の回帰検知用の確認」という
     位置づけにとどまる、実効的なターゲット検証ではない）。
   - `ImeOpenOutcome::UnsafeToToggle`（Win キー押下中）は無条件に再武装する
     （外部条件＝Win キー解放で必ず終わるため試行回数上限は不要）。
     `Failed`（Win32 呼び出し自体の失敗）も再武装するが、**試行回数上限
     （armed_gen ごとに2回）を付ける**（**訂正**: 当初「Failed は再武装しない」
     としていたが、これは「次の周期 refresh が拾う」という前提に基づいており、
     その周期経路自体を本 item で撤去したため前提が崩れている。かといって
     無制限に再武装すると、`Failed` が恒久的に返る環境で打鍵のたびに
     同期 IMC write ~100ms を伴う再試行が延々と続く。ADR-080 の
     `Actuation.attempts`/`FeedbackPolicy::Blind{max_attempts}` と同型の
     有限リトライで折り合いをつける）。
   - **既存の `AppliedImeState` スロットル**（`Optimistic(true)|Confirmed{open:
     true}` なら送らない、force 分岐には適用されない）は新しい打鍵時消費でも
     引き続き読まない——force の趣旨は「applied が誤って ON にラッチされた
     状態を破ること」であり、このスロットルを読むと趣旨と矛盾する。後日
     「重複ガードだ」として誤って足されないよう、コードに否定コメントを残す。
   - **実送信のレート制限**（**追加、2回目レビュー指摘 M3**）: フォーカス
     チャーン環境（Chrome 連続フォーカスイベント、UWP 2段フォーカス、
     通知フォーカスチャーン）下で高速タイピングすると「毎打鍵で再武装→
     毎打鍵で発火」＝20〜50ms 間隔になり、§1.2 欠陥4 が実機記録した
     `9c102b02` の連打問題と同じレート（周期版より悪化）で再現しうる。
     `ime_poll_interval_ms`（既定500ms、撤去した `last_force_on_resend_ms`
     が与えていた下限と同一値、新規タイミング定数は導入しない）を実送信の
     下限間隔として使う。レート制限に掛かった場合は**消費せず武装を維持**
     する（他の武装維持分岐と同じ扱い。破棄すると BUG-16 型のリテラル化
     取りこぼしに直結するため）。
   - **段階導入**: まず本項目（案 C、周期を落として打鍵に紐づける）だけを
     入れて実機ソーク（§5 Phase 3 実機ソーク参照）で「フォーカス不変のまま
     IME が OFF に落ちる」ケースが実際に再現するかを測る。再現したら案 F
     （idle 明け武装）か、INV-16 の自己駆動禁止を守れる形（外部周期にのみ
     従う独立予約）を追加検討する。両方を一度に入れると、どちらが効いたか
     切り分けられなくなる（Phase 0 が `9c102b02` を revert した理由と同じ
     失敗パターン）。
2. **provenance 記録**。当初案の `InputModeApplyStrategy::ForcePolicyResend`
   は不採用——同 enum は input_mode（ローマ字/かな等）の**補正手段**専用であり、
   open/close 自体の適用理由を運ぶ経路ではない。加えて調査の結果、
   **force-ON は現状イベントを1つも出していない**ことが判明した:
   `record_ime_apply_result`（`state/platform_state.rs:562`）は
   `generation.is_some()` のときだけ `ImeEvent::from_apply_outcome` を
   dispatch するが、force 分岐・bootstrap・drift correction はすべて
   `generation: None` を渡している。variant を足すだけでは INV-18 を
   満たせない。
   - `state/ime_event.rs` に `OpenApplyReason`（`EngineDecision`/
     `ForcePolicyResend`/`ImmBrokenForceOn`/`Bootstrap`/`DriftCorrection`/
     `ShadowToggle`）を新設し、`Runtime::on_ime_apply_complete` へ**必須引数**
     として追加する（`Option` にしない——デフォルトが入ると provenance が
     欠落する）。呼び出し元 6 箇所を機械的に更新する。
   - `reason` はジャーナルへ残す（`on_ime_apply_complete` 内で
     `journal.record(...)` を1行追加。`ir_apply_drift_correction` が
     `ActuationRecord::new(act_origin, ..)` で同種のことを既に行っている
     前例に倣う）。
   - `generation` 化（force 経路にも `allocate_event_generation()` を払い出し
     既存の `ImeApplySucceeded`/`Failed` イベント自体を発火させる）は
     **本 Phase のスコープ外**とする。`pending_generation()` の照合セマンティクス
     と `discard_actuation` の相互作用の再設計が要るため、§7 に未解決論点として
     起票し Phase 4 以降へ回す。
3. **`apply_ime_open_with_belief` の送信先を `ActuationTarget` で守る（INV-14）
   —— 不採用。SendInput 経路は撤退し、INV-13 の例外として明記する。**
   検討した3案:
   - (a) 同期の軽量ターゲット確認（`GetForegroundWindow()` を送信直前に呼び、
     事前に記録した hwnd と比較）。**効果が小さい**: `send_ime_mode_key`
     （`ime.rs`）は SendInput 直前に `win_key_held()`（キャッシュ読み）と
     `HeldModifiers::read()`（`GetAsyncKeyState`、ローカル数 µs）しか挟まず、
     Win32 往復ゼロ——P7 が言う「検証窓を Win32 呼び出し1回分に縮める」目標は
     この経路ではほぼ達成済み。加えて**比較対象となる「起案時 hwnd」がそもそも
     存在しない**（`PlatformState` に現在フォーカス HWND のキャッシュは無い、
     §7-3 で確定済み）——`get_focused_hwnd_async()`（30ms）を打鍵ホットパスに
     持ち込むことになり、得るものより失うものが大きい。
   - (b) 既存の非同期 `ActuationTarget` を流用し `apply_force_on_for_imm_broken`
     自体を非同期化する。**波及が大きすぎる**: `apply_ime_open_with_belief` は
     完全同期で戻り値を呼び出し元が同期的に `on_ime_apply_complete` へ渡す設計
     契約（`executor.rs::dispatch_ime_set_open` の `sync_outcomes` 契約、
     `kp_stage_shadow_ime_toggle` の同期分岐、`ir_apply_drift_correction` の
     ADR-080 epoch/attempts 管理）に依存する複数の呼び出し元を再設計することに
     なる。
   - (c) **採用**: SendInput にはターゲット検証が構造的に適用できないと結論し、
     INV-13 の例外として明記する。`SendInput` は宛先 hwnd を指定するパラメータを
     持たず「OS が今フォーカスを持つとみなすウィンドウ」へ配送する仕様のため、
     `ActuationTarget`（IMC write 向けに「特定の hwnd への書き込み」を検証する
     ために設計されたパターン）をそのまま持ち込む対象が無い。
   - **ただし撤退するのは SendInput 部分だけ**。同じ呼び出しチェーンに紛れている
     `ime_controller.rs` の `set_ime_romaji_mode()`（IMC write）は item 0 で
     別途処理し、撤退対象に含めない。
   - 空間軸の検証を諦める代わりに、item 1 で述べた**時間軸フェンス**
     （消費直前に `ime_mode_focus_gen` を照合）を入れる。空間軸が構造的に
     取れない経路では時間軸だけでも入れる方が良い、という理由を明記する。
     **訂正（2026-08-08 2回目 opus アドバーサリアルレビュー）**: 当初
     「UWP の2段フォーカスで1操作あたり複数回 FocusChange が来るケースを
     実際に弾ける」としていたが、これは誤り。`ime_mode_focus_gen` を進める
     唯一の経路（`Output::on_ime_mode_focus_changed`）の唯一の呼び出し元は
     武装処理の直前1行であるため、gen が進む瞬間＝武装が最新化される瞬間が
     常に一致し、`armed_gen != current` という不一致状態は構造的に到達
     不能——2段フォーカスは武装を**弾く**のではなく**上書きして最新化する**
     だけである。この時間軸フェンスは「弾く」効果を持たず、将来この経路が
     非同期化された場合の回帰検知用の確認にとどまる（実効的なターゲット
     検証ではない）。
   - この「SendInput にはターゲット検証を適用できない」旨は
     `runtime/key_pipeline.rs`（`VK_DBE_HIRAGANA` 注入について）に既に同趣旨の
     注記が存在する。本 ADR を SSOT とし、コード側のコメントは本節への
     ポインタへ差し替える。

**Phase 3 全体を撤退する必要はない。** item 1（周期を落として打鍵に紐づける）は
既存の confirm-then-transmit ゲートに乗るため新規タイミング定数ゼロで実装でき、
撤退も item 1+item「周期削除」の1コミットの revert で済む。item 3 だけが
構造的に不可能で、本 ADR がその可能性を最初から織り込んでいる（隠さない、
ADR-084 INV-7 と同じ姿勢）。

**実機ソーク（実装完了後、タスク #17 と同一セッションで行わないこと——理由は
下記）**: 測定項目は (a) フォーカス後1打鍵目の追加レイテンシと
`MS_IME_READY_CONFIRM_MS`（400ms）到達率——TsfNative（Windows Terminal /
WezTerm）× MS-IME / GJI の4組、(b) 周期撤去後に「フォーカス不変のまま IME が
OFF」（2026-08-06 実機報告のロック解除後静寂期パターン）が再現するか、
(c) 1打鍵目のリテラル化（`bあ`/`korede` 系）が増えないか、(d)
`UnsafeToToggle`/gen 不一致による再武装頻度、(e) **force-ON 1回あたりの
`kp_run_inner` 滞在時間**（item 0 未移行の同期 IMC write により ~100ms 程度が
乗る想定、§5 Phase 3 item 0 参照）、(f) **Alt+Tab で Tab を連打したとき、
force-ON（`ForcePolicyResend`）ログが中間ウィンドウ（`XamlExplorerHostIslandWindow`
等）宛に出ていないか**（`[apply-ime]` のクラス名と併せて確認。item 1 の
入力意図ガードが機能しているかの確認）、(g) **TsfNative × force で
`[drift] correction:` が周期で実際に発火するか**（item 1 の
`reschedule_ime_refresh` 例外復元が意味を持ったかの確認）。
**Phase 2 のソーク（#17）を先に単独で回してベースラインを取ってから、本
Phase の消費点コミットをマージすること**——両者を同一セッションで一緒に
測ると、上記副作用が conv 軸由来か open 軸由来か切り分けられなくなる。

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
- **INV-18**: conv 軸は `ConvMutationReason::ForcePolicy`、open/close 軸は
  `OpenApplyReason::ForcePolicyResend`（§5 Phase 3 item 2）を使う。
  **訂正（2026-08-08）**: 当初 `InputModeApplyStrategy` に `ForcePolicyConv`/
  `ForcePolicyResend` を追加する案だったが、同 enum は input_mode 補正専用
  であり意味論が合わないため採らなかった（Phase 2 は `ConvMutationReason`
  という別の型で解決済み、Phase 3 も同様に `OpenApplyReason` という別の型
  で解決する）。

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
   **open 軸での追記（2026-08-08、Phase 3 設計調査）**: conv 軸ではこれは理論上の
   懸念だったが、open 軸（TsfNative Blacklist）では**実害が既に報告されている**。
   TsfNative は IMC が構造的に読めず、観測ベースの是正（`ir_apply_drift_correction`）
   は観測が存在しないため発火できない。周期 force-ON がこの環境で唯一の自己回復
   手段であり、Phase 3 item 1 でこれを入力意図トリガーへ倒すと、周期 poll の谷間で
   実 IME が OFF に落ちたケースを次の打鍵まで検知できない空白期間が生まれる
   （2026-08-06 実機報告「ロック解除後の長い静寂期間で belief=ON × 実IME=OFF」が
   この空白期間の実例）。このため Phase 3 item 1 は段階導入とし、実機ソークで
   この懸念が実際に顕在化するかを見てから案 F 等の追加武装トリガーを検討する
   （§5 Phase 3 参照）。

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

9. **Phase 2 の当初案からの訂正経緯（2026-08-08、実装着手前の opus アドバーサリアル
   レビュー）。** §5 Phase 2 は当初案から次の点を訂正した確定版に置き換えた。
   - `ConvModeTarget::Explicit(u32)` → `Desired(ConvMode)`（INV-17 と直接矛盾していた）。
   - 消費点「`ms_ime_gate_defer` 手前」→「`Output::send_romaji`」（当初案の位置は
     cold パスで到達不能であり、item 4 の cold_warmup.rs 移行と両立しなかった）。
   - item 4「`forced_target` を移す」→「`Force` アームのみ削除」（Observe デフォルトの
     ROMAN 保護を巻き込んで消してしまう誤読を防ぐ明確化）。
   - 実測義務を撤回し、根拠（既存の confirm-then-transmit ゲートが新定数なしで
     待機を担う）を明記。
   - 再武装ロジック（item 5）を追加。当初案は「消費して書き込む」までしか設計しておらず、
     `ActuationTarget` の `Aborted`/capture 失敗で「消費済み・未書き込み」のまま
     force が永久に発火しなくなる穴が未検討だった。
   - `Output`/`Runtime` の非対称性という論点自体が実在しないと判明し、
     `actuate_conv_mode` の `Output` への移設を item 0 として追加した。
   - outbox 経由の案を検討し、drain タイミング（送信後）が Phase 2 の目的と
     正反対になるため却下した。

10. **Phase 3 の当初案からの訂正経緯（2026-08-08、実装着手前の opus アドバーサリアル
    レビュー）。** §5 Phase 3 は当初案から次の点を訂正した確定版に置き換えた。
    - item 0（新規追加）: `ime_controller.rs` の `set_ime_romaji_mode()`
      ライブクエリ IMC write が INV-14/19 未対応のまま残っていたと判明
      （7 経路の数え漏れ）。item 1 着手前に処理する前提を明記。
    - item 1「呼び出し元（周期）はそのまま、武装/消費だけ分離する」と読める
      曖昧さを排除し、「実際の書き込みが起きる場所自体を打鍵イベントへ移す」
      と明記。Phase 2 の `Output::force_pending` とは統合せず、`Runtime` 側に
      専用の `force_open_pending` を新設する方針に確定（層・タイミング・
      再武装セマンティクスが conv 軸と異なるため）。
    - item 2「`InputModeApplyStrategy::ForcePolicyResend`」→「`OpenApplyReason`
      新設 + `on_ime_apply_complete` への必須引数追加」（前者は input_mode
      補正専用の enum で意味論が合わない。加えて force-ON が現状イベントを
      1つも出していないという、当初案が見落としていた前提条件が判明した）。
    - item 3「実際に守れるかは未検証」→「(a)(b)(c) 案を評価した結果 (c)（撤退 +
      時間軸フェンスのみ）に確定」。SendInput が宛先 hwnd を持たない構造的
      制約と、`apply_ime_open_with_belief` の同期呼び出し契約への波及の大きさ
      を根拠に明記した。
    - 段階導入（案 C のみ先行、実機ソークを見てから案 F を検討）を明記。
      TsfNative では周期 force-ON が `ir_apply_drift_correction` の代替に
      なっておらず唯一の自己回復手段であるという §7-2 の追記と対応する。

11. **Phase 3 実装完了後の2回目 opus アドバーサリアルレビュー（2026-08-08）と
    その対応。** §5 Phase 3 の確定版どおりに実装しコミットした後、実装内容
    そのものを2回目のアドバーサリアルレビューにかけた結果、High 2件・
    Medium 5件・Low 4件、レビュー中に新規2件（計13件）を検出した。
    §5 Phase 3 本文は最終的な訂正後の状態のみを記載しており、以下は
    「一度実装してからレビューで見つかった問題とその訂正」という経緯の記録。

    - **H1（実装済み）**: 消費点を `try_hold_key`/ime-off-rescue より後にしか
      置いておらず、`kp_stage_focus_probe` より前だったため、フォーカス変更後
      1打鍵目を必ず取りこぼし2打鍵目で発火する実装ミスがあった。§5 item 1 の
      「消費点」の記述を `kp_stage_focus_probe` 等の後という訂正版に置き換えた。
    - **H2（記録のみ、未解消）**: item 0 が実際には未移行のまま item 1 を
      投入していた。ステータス行に明記。
    - **M1（ドキュメントのみ訂正）**: gen フェンスが「UWP 2段フォーカスを
      弾ける」という記述は誤りだった。§5 item 1/item 3 を訂正。
    - **M2（実装修正）**: `architecture_guard` の走査対象移動案がテストを
      トートロジー化するところだった。武装専用関数の抽出はしつつ、guard の
      走査対象自体は移動せず出現数固定方式へ変更する設計に訂正した。
    - **M3/M4（実装修正）**: 周期レート制限撤去がフォーカスチャーン環境で
      撤去前より高頻度の連打を招く恐れ、および `Failed` の再武装方針転換
      （無制限ではなく試行回数上限付き）を §5 item 1 へ統合した。
    - **M5（実装修正・判断保留付き）**: `reschedule_ime_refresh` の
      force_policy 例外撤去が `ir_apply_drift_correction`（BUG-20 の
      non-ImmCross 分岐）の周期実行機会も巻き添えで奪っていた。例外を
      復元するが、「force policy ユーザーだけが周期 drift correction を
      持つ」という新たな非対称を生むため、本来はポリシー非依存に判断すべき
      という留保付きで §7-12 に未解決論点として起票する。
    - **新規N1（実装修正）**: force-ON が `note_explicit_ime_action` を
      呼んでおらず、`kp_stage_idle_conv_check` の汚染防止ガードを素通り
      していた。§5 item 1 に追加。
    - **新規N2（実装修正）**: force-ON 経路が常に `belief_input_mode:
      Unknown` を使うため `ObservedKana` 保護（ユーザーが意図的に選んだ
      かな入力を上書きしない）が一度も効いていなかった。§5 item 1 に追加。
    - **L1〜L4**: 軽微な訂正・テスト追加（対応状況はコミット履歴参照）。

12. **`reschedule_ime_refresh` の force_policy 例外が生む新たな非対称
    （M5、2026-08-08、未解決）。** §5 Phase 3 item 1 で復元した例外により、
    `conv_mode_policy = force` のユーザーだけが TsfNative で周期
    `ir_apply_drift_correction`（BUG-20 の non-ImmCross 分岐）の実行機会を
    持ち、`observe`（デフォルト）のユーザーは元々この周期を持たない
    （`reschedule_ime_refresh` の `is_tsf_native` 早期 return はポリシー
    非依存のため）。この非対称は Phase 3 が意図して導入したものではなく、
    force-ON の周期経路を撤去する際に巻き添えで生じた副作用の最小復元に
    すぎない。本来は「TsfNative で drift correction の周期実行機会を
    持たせるべきか」を `conv_mode_policy` に関わらず判断すべき論点であり、
    本 ADR のスコープでは未解決のまま残す。実機ソークで TsfNative × observe
    環境の drift 未検出頻度が問題になるようなら、この例外条件を
    `is_force_policy()` ではなく `is_effectively_tsf_native()` （ポリシー
    非依存）へ広げることを検討する。

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
