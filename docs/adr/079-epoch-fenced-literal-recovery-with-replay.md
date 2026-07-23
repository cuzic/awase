# ADR-079: per-VK confirm の stale confirm 誤帰属と、ESC スコープを利用した epoch-fenced literal recovery + 限定 replay

## ステータス

Stage 1（epoch fencing の検出 + 既存 `SuspectedLiteral` と同型の回収）実装済み
（2026-07-22、`docs/known-bugs.md` BUG-35。統合ブランチでの改番注記は同エントリ
参照 — 実装当時は「BUG-33」として書かれていたが、main 側に既に別の BUG-33 が
存在したため統合時に BUG-35 へ改番）。当初は「検出のみ・recovery 無し」
だったが、per-VK confirm の1文字目でこの経路が発火すると未送信 VK が欠落する
regression を実機で引き起こしたため、BUG-35 追補で「stale confirm を検出したら
既存の backspace + romaji 再送に倒す」方式に修正済み。Stage 2（quarantine →
ESC + 消えた文字の retype + 後続入力の限定 replay）は未実装。詳細は BUG-35 の
「未解決の follow-up」を参照。Stage 2 着手前に、実機で Stage 1 の
`[epoch-fence-stale]` ログを観測し信号の質を確認することを推奨する
（設計レビューで見つかった懸念、後述「未解決事項」参照）。

## コンテキスト

### 症状（実機ログ、2026-07-22、Windows Terminal）

Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`、GJI、TSF-native）で、ユーザーが Ctrl+無変換で IME OFF → `4`→`1`（半角数字、直接パススルー）→ 物理サムキーで IME 再 ON → 「ふん」（romaji "fu"+"nn"）と高速に連続入力したところ、**「41分」と入力したはずが「4分」になった**（`1` が消失）。`f`/`u` 側が消えたのではなく、`1` という既に確定済みの実文字が消えた点がユーザーにとって直感に反していた。

### 診断（本 ADR に先立つ会話で確定した事実）

ログを時系列で追った結果、以下が判明した:

```
29.058  vk-run 送信 (cold=263, romaji "fu" の F,U を1文字ずつ送信)
29.065  [gji-io] WRITE: w_ops=+1 (Fキー処理のGJI I/O)
29.079  per-VK[0/1] confirmed (vk=0x46 "F")     ← 29.065のWRITEを根拠に確定
29.171  deadline(300ms)超過 → per-VK[1/1] suspected literal (vk=0x55 "U")
29.388  → [raw-tsf-literal] backspace ×1 + re-send "fu" scheduled + mark cold(cold=264)
29.392  backspace実行、"fu" 再送開始(cold=264)
29.429  ★ candidate SHOW #892: last_gji_write=360ms ago
        → 29.429 - 360ms ≈ 29.069 ≈ 29.065(cold=263 の WRITE と一致)
```

`last_gji_write=360ms ago` という既存の観測フィールドから逆算すると、29.429 の候補ウィンドウ SHOW は **cold=264（2回目の送信）ではなく cold=263（1回目、backspace で見捨てた方）の 'f' が実際に処理された結果**だと特定できる。つまり:

1. **1回目(cold=263)の "fu" 合成は実際には成功していた**。候補ウィンドウの表示が単に 300ms deadline に対して ~41ms 遅れただけで、GJI 自体は正常に動いていた（false positive）。
2. false positive の場合、composition はまだ pre-edit 状態で **`f`/`u` のどちらも実テキストとして確定していない**。つまり backspace ×1 が「消すべき対象（literal な `u`）」は最初から存在しない。
3. `VK_BACK` は「疑わしい文字を狙って消す」命令ではなく、**カーソル直前の1文字を無条件に消す**命令である。消すべき literal が存在しない以上、backspace は必然的にその手前の**唯一実在する確定済み文字**、すなわち IME OFF 中に直接パススルーされた `1` を消す。
4. さらに、cold=264（2回目の再送）自身の `per-VK[0/1] confirmed (vk=0x46)`（29.435）も、実は同じ stale な SHOW イベント(#892)に便乗して確定しており、**世代をまたいで古い非同期シグナルを現在の試行の証拠として誤って使い回している**。今回はたまたま元の合成が有効だったため出力（「分」）自体は正しく収束したが、これは設計上の偶然であり、confirm 機構が試行世代（`cold=N`）を区別せずに任意の遅延シグナルを受理してしまう構造的な穴が残っている。

### 理論的背景

この種の「タイムアウトで死んだと判定するか、遅いだけとみなすか」は非同期システムにおける古典的な不可解決性を持つ:

- **Unreliable Failure Detectors**（Chandra & Toueg, 1996）: 非同期システムでは accuracy（誤検出しない）と completeness（有限時間で必ず検出する）を同時に満たす故障検出器は原理的に構築できない。timeout を伸ばすアプローチ（`tuning-constants.md` が「盲目的エスカレーション」として既に警告している 20→100→200→350ms の積み重ね）は、このトレードオフ曲線上を移動しているだけで曖昧さ自体は消えない。
- **TCP retransmission ambiguity / Karn's Algorithm / Eifel Detection Algorithm**: 再送後に来た ACK が元の送信のものか再送のものか区別できない問題への古典的解法は、送信に epoch/timestamp を埋め込み、証拠側にそれを照合させること。本件の `last_gji_write` は偶然この役割を果たせるデータを持っていた。
- **Fencing token**（分散ロック一般、Kleppmann “How to do distributed locking”）: 「死んだと判定したアクターが実は生きていて後から作用する(zombie)」問題への対策は、消費側で世代番号をチェックし、古い世代の副作用が新しい世代の状態を汚染しないようにすること。

## 決定

### 1. Epoch fencing による stale confirm の検出

`probe_fsm.rs` の per-VK confirm ロジック（`run_per_vk_confirm` およびその上流の `LiteralDetector`）が使う確定根拠（候補 SHOW イベント、GJI I/O WRITE）に、**その根拠が発生した時刻が現在の試行世代の送信時刻より前でないか**を照合する fencing チェックを追加する。既存の `last_gji_write` 相当のタイムスタンプと、`apply_vk_sent` が記録する送信時刻（`cold=N` に紐付く）を突き合わせれば判定できる。

- 根拠が現在世代の送信時刻以降 → 通常通り confirm 採用。
- 根拠が現在世代の送信時刻より前 → その根拠は**過去に見捨てた世代（前回の backspace 対象だった世代）由来のstale confirm**であり、「実はその見捨てた世代の判定こそが誤りだった（本当は合成できていた）」という強いシグナルとして扱う。

### 2. Stale confirm 検出時のリカバリ: ESC ベースの安全なリセット + 限定 replay

Stale confirm を検出したら、以下の順で対処する:

1. **ESC を送信する**。ESC の破壊スコープは「現在 IME ON になっている composition」に限定される（BUG-29 検討時に確認済みの Windows API 仕様）。IME OFF 時に確定した文字（本ケースの `1`）はこのスコープの**外側**にあるため、ESC では一切触れない。これにより「何文字消すか」を数える必要がなくなり、backspace のような盲目的な文字数カウントの誤りが構造的に起きなくなる。
2. **誤って backspace で消してしまった文字（例: `1`）を retype する。**
3. **backspace 以降に実際に発生した後続入力も、記録してあれば replay する。** 高速タイピング下では、stale confirm が判明する頃には既に後続キー（スペース、次のかな等）が処理済みのことが多い（本トレースでも 41ms の間に space と「ん」の合成が既に走り出していた）。ESC は「その瞬間の pending composition」を丸ごと消すため、何もせず ESC すると **正しく進行中だった後続の合成まで巻き込んで消してしまう**。したがって、backspace 発生時点から stale 判定確定までの間に発生したアクションを世代付きでバッファし、ESC 後にまとめて replay する。

### 3. Replay 対象からの除外: 変換トリガー系キー

バッファ・replay の対象は、エンジンが確定した Char/passthrough アクションの列とする。ただし **Space / Enter 等、IME の変換候補確定・選択のトリガーとして働きうるキーは replay 対象から除外する。**

理由: これらのキーの意味は「その時点で pending composition が存在するか」に依存する。元の実行では pending composition が存在する状態で Space が「変換」として機能していたとしても、ESC 後の（まっさらな）状態で同じ Space を replay すると「pending composition が無いので単なる半角スペース挿入」に化けるなど、**シーケンスとしては同一でも意味論的に同一の結果になる保証がない**。この非対称性を repair する一般解は用意しない方針とし、変換トリガー系がバッファに含まれていた場合はそこで replay を打ち切る。結果としてユーザーに一部の不整合（例: 変換トリガー以降の入力が意図通りに再現されない）が見える可能性があるが、既存の drain queue / deferred VK 機構と同程度の許容範囲として受け入れる。

### 使用する既存インフラ

新規に大きな仕組みを作るのではなく、既存の以下のパターンを拡張する:

- 世代カウンタ: 既存の `cold=N`（`GjiFsm`/warmup coroutine）をそのまま epoch として流用する。
- 物理イベントの一時保留: 既存の output-gate / drain queue（`[output-gate] deactivated ... pending_drain=N` → `WM_DRAIN_OUTPUT_QUEUE`）と同型のバッファリングパターンを、Char/passthrough アクションのレベルに拡張する。
- 合成後アクションの遅延: 既存の deferred-VK queue（`[tsf] probe in flight → deferred N VK(s)`）に近い設計で、replay バッファの投入・排出を行う。

## 検討したが採用しなかった案

- **VK_ESCAPE を候補 SHOW 強制のために使う案**（BUG-29 で既に却下済み）: 本 ADR の用途はそれとは異なる（進行中の composition 確認のための強制 HIDE ではなく、誤りと判明した composition の完全破棄）。BUG-29 の却下理由（「VK0 で確定した文字ごと消してしまう危険」）は**同一 composition セッション内**の話であり、本 ADR が前提とする「IME OFF 時に別セッションで確定した文字には届かない」という ESC のスコープ理解とは矛盾しない。
- **timeout（300ms deadline）を実測値ベースで単純に延長する案**: `tuning-constants.md` が指摘する「同じ定数 family の盲目的エスカレーション」の再演になるため、根治にならないと判断し採用しない。Chandra-Toueg の観点からも、timeout 延長は accuracy/completeness トレードオフ上の移動に過ぎず、別の（より遅い）ケースで同型の false positive が再発しうる。
- **ring buffer による無条件補償 retype**: 「backspace 実行直後、まだ何もcommitされていなければ retype」という単純ガードだけでは、本トレースのような高速タイピング下（stale 判定までに後続入力が既に進行している）を救えないため、本案（後続入力も含めた限定 replay）に発展させた。

## 未解決事項 / 実装時に詰めるべき点（2026-07-22 実装セッションで決定）

### 1. Fencing チェックの実装箇所・判定条件（決定）

`tsf/observer.rs` に既存の `gji_last_write_ms()`（GJI プロセスの最終 WriteOperationCount 変化時刻、live 読み取り）を、そのまま fencing の判定基準に使う。新規のタイムスタンプ計装（例: SHOW 専用タイムスタンプ）は追加しない。

判定条件は単一: **`gji_candidate_show` の SHOW / `gji_write_bytes` の閾値超過のどちらが confirm 信号を出したかに関わらず、`gji_last_write_ms() >= epoch_send_ms`（自分の世代が VK を送信した時刻）を満たさない限り、その confirm を「自分の世代の証拠」として採用しない**。理由: 実機トレース（本 ADR コンテキスト節）では、SHOW イベント自体は cold=264 の送信後に本当に発火していたが、対応する `gji_last_write_ms` は cold=263 送信時点の値のまま更新されておらず、「候補ウィンドウが遅れて反応しただけで、実際の GJI I/O は現世代のものではない」ことが分かる。この1条件で SHOW/write-bytes 両シグナルの stale 判定を一元的にカバーできる。

配置は `tsf/probe.rs::LiteralDetector` に決定（`tsf/warmup/probe_fsm.rs::run_per_vk_confirm` 側には置かない）。`LiteralDetector` に `epoch_send_ms: u64` フィールドを追加し（`new_with_pre_send_baseline` 等、VK 送信直前にベースラインを取得する既存の呼び出し箇所で同時に取得可能）、`check_now` の戻り値 `DetectionResult` に第三のバリアント `StaleConfirm` を追加する。

理由: BUG-30 で確立した「`check_now` は信号検出（signal detection）に専念し、veto のような行動判断は `LiteralDetectCore::poll` 側に置く」という分離は維持する。ただし fencing は「この信号にどう反応するか」という policy ではなく「この信号は誰の送信に帰属するか」という検出そのものの精度の問題であり、veto とは性質が異なる（veto は「信号は現世代のものだが、行動を保留すべきか」の判断。fencing は「その信号は現世代のものか、そもそも別世代のものか」の判定）。よって `check_now` の責務を拡張する形が一貫する。

呼び出し側（`run_per_vk_confirm`／`LiteralDetectCore::poll`）は `StaleConfirm` を新たに受け取る分岐を追加するだけで済み、`veto_eligible`/`VetoDecision` 機構（`NotApplicable`/`Hold`/`Expired`）とは独立に共存する（veto は「有効な `SuspectedLiteral` をどう扱うか」、fencing は「そもそも `CompositionConfirmed`/`SuspectedLiteral` の判定材料が正しい世代のものか」であり、判定順序は fencing → veto）。

### 2. Replay バッファの保持期間・上限・所有者（決定）

**新たに判明した欠落:** 既存コードには「backspace で消した文字が何だったか」を記録する仕組みが一切ない（`RawTsfLiteralRecovery` アクションは `backs`＝消す文字数のみを持ち、消した文字の内容は保持しない）。ADR 本文の replay 対象は「backspace 以降の後続入力」のみだが、stale 判明時に retype すべき「誤って消してしまった文字自身」を復元するには、**直近送信済み出力アクションの小さな履歴リング**を新設する必要がある。

- **保持期間:** 新規の時間定数は導入しない。既存の `RAW_TSF_LITERAL_DETECT_MS`(300ms) / `RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE`(500ms) の confirm/timeout ウィンドウにそのまま乗せる（このウィンドウ内で `StaleConfirm` が確定するか、確定しないまま `Done` になれば用済み）。
- **上限:** 16 件（`INPUT_DEFER::MAX_CAPACITY`(1024) は無関係に長時間分をカバーするものであり参考にしない。300〜500ms のタイピング速度から見て 16 件で十分）。上限超過時は log と共に「今回の recovery は replay を諦めて従来通り（backspace のみ、後続 UX 不整合は許容）」にフォールバックする（silent には切り捨てない）。
- **所有者:** `TsfWarmupCoordinator`（probe インスタンスの差し替えを跨いで生存する唯一のコンポーネント。既存の `pending_deferred: RefCell<Vec<DeferredVk>>` と並ぶ新フィールドとして追加する）。
- **破棄タイミング:** フォーカス変更（`on_focus_changed()`）、非 stale の通常 `Done`、stale-recovery 完了（ESC+retype+replay 実行後）のいずれかで破棄する。

### 3. 「変換トリガー系」キーの列挙（決定）

新規の列挙は作らず、既存の `vk::is_composition_confirm_key`（`crates/awase-windows/src/vk.rs:212-219`、Space/Enter/Escape の3つ）を再利用する。この関数は既に「composition を確定／キャンセルするキー」として定義・テスト済みであり、本 ADR が要求する「pending composition の有無で意味が変わるキー」の概念と完全に一致する。replay バッファの走査は、この関数が `true` を返す VK に到達した時点で打ち切る（そのキー自体は replay せず、以降も replay しない）。

### 4. テスト・記録義務（変更なし）

`fix-requires-evidence.md` に基づき、実装時は golden test（`ime_key_sequence_golden.rs` 等、`DetectionResult::StaleConfirm` を含む純粋関数の回帰テスト）と `docs/known-bugs.md` への記録の両方を行う（本件は warmup/cold-start・キー選択領域に該当するため両方が望ましいケース）。

### 5. タイミング定数（変更なし）

`tuning-constants.md` に基づき、本設計は新規の時間定数を導入しない（決定 2 の通り既存定数を再利用）。fencing 判定自体も新規の待機時間を追加するものではなく、既存の confirm/timeout ウィンドウ内で判定条件を精緻化するのみ。実機実測が必要になるとすれば、リプレイバッファの上限 16 件が実際のタイピング速度で十分かの検証であり、これは Windows 実機ソークテストで確認する（このサンドボックスでは実行不可、フラグを立てて別途実施）。

## 関連

- `docs/known-bugs.md` BUG-29, BUG-30（per-VK confirm の suspected literal 誤判定、SHOW/WRITE ヒューリスティックの限界）
- `.claude/rules/tuning-constants.md`（timeout 定数の盲目的エスカレーション禁止）
- `.claude/rules/fix-requires-evidence.md`（warmup/cold-start・キー選択領域の fix にはテストか記録が必須）
- `docs/windows-api-constraints.md` §1-2（VK_ESCAPE の composition キャンセル仕様）
