---
id: ADR-159
title: |-
  既存の送受信境界を棚卸しし、記録・再生・シャドー実行の土台にする
summary: |-
  ADR-158採用Aの子ADR。当初案をround1で反証し「既存境界の棚卸しと未収束呼び出し元の特定」に組み替え。2026-09-09の実機スパイクでM1(送信機構はsend_input_safe/send_ime_control の2系統、SendInput:WM_IME_CONTROL比が2セッションとも約7〜8:1で再現性あり)・M6(journal非欠落)を実測で確定、M2(InputRelay gate)はテスト条件不足でMWB検証を当面見送り静的解析ベースで判断。さらに段階0の成果物を「棚卸し文書」から「ADR-161実証実験で検証済みのdylint宣言強制」に定義し直した
status: |-
  起票。TJ2(単体レビュー)実施済み・round4反映済み。段階1(TF1)/段階2(TF2、`shadow_send_trace.rs`、送信内容のシャドー記録)は2026-09-10にPR#193で実装・実機検証済み。再生側（決定点への再投入）は子ADR[163](163-actuation-decision-io-separation-and-replay-harness.md)が引き継ぐ
related_adr:
  - "ADR-119"
  - "ADR-121"
  - "ADR-151"
  - "ADR-152"
  - "ADR-156"
  - "ADR-158"
  - "ADR-160"
  - "ADR-161"
  - "ADR-162"
---

# ADR-159: 既存の送受信境界を棚卸しし、記録・再生・シャドー実行の土台にする

## ステータス

**起票。[ADR-158](158-complexity-reduction-north-star.md)（北極星）の採用Aを分割・詳細化した
子ADR。ADR-158自体はopus-adversarial-consult round1を経ており、本ADRはその反映結果を引き継いだ
状態からスタートする。opus-adversarial-consult round1（本ADR単体、Must-fix 6件・Should-fix 7件）
を受け、2026-09-09に実機スパイク（`spike/io-boundary-instrumentation`ブランチ）でM1・M2・M6を
検証済み——詳細は「実機スパイク結果」節。M1（送信機構は複数ある）・M6（journal非欠落）は実測で
決着、M2（InputRelay gate）はテスト環境の制約で未完了のまま、静的解析に基づき当面のMWB実機検証
は見送る判断とした。round2（M3〜M5・Should-fix反映）は未実施。さらに2026-09-09、
[ADR-161](161-single-source-spec-generation.md)の実証実験で検証された「宣言の強制とSSOT化」
原則（[ADR-158](158-complexity-reduction-north-star.md)参照）を反映し、段階0の成果物を
dylint宣言として明確化した。**

## 背景

[ADR-158](158-complexity-reduction-north-star.md)は、awase（Windows用親指シフトIMEリマッパー）
の設計複雑性の根本原因をRC1〜RC4の4点に整理した。うちRC2（正しさを判定できる装置が実機にしか
なく、CIのガードが多くの仮説を反証できない）を根絶する施策として「採用A」を置いたが、その内容は
opus-adversarial-consult round1で大きく組み替えられた。本ADRはその組み替え後の内容を独立した
ADRとして詳細化する。

### 当初案とround1での反証

当初案は「Win32からの受信・Win32への送信という2つのシーケンス列を、新しいコンポーネント境界
として設計する」（A0）というものだった。ユーザーの「ここの抽象化をうまくすることで全体的な
設計を分かりやすくできる」という直感から出発したが、round1レビューが実コードと突き合わせた
結果、この前提は以下の点で成立しなかった。

**送信側の境界は既に単一で存在する**。`crates/awase-windows/src/win32.rs:268`
`send_input_safe(&[INPUT]) -> u32` がcrate内で**唯一の`SendInput`呼び出し**であり、既に以下を
一手に引き受けている:

```
win32.rs:270  conv_mutation::bump()          // conv-mode 変化の唯一のゲート
win32.rs:277  probe_actuation_fence::bump()  // probe/actuation フェンス
win32.rs:280  LAST_ACTUATION_ISSUE_US.store  // 発行時刻の記録
win32.rs:283  tracing::debug!("[ime-io] actuation SendInput ...")
```

分散しているのは境界ではなく、**この関数を呼ぶ側の意思決定**である。呼び出し元は12ファイル
20箇所（`hook.rs:324`、`output/mod.rs:753,766`、`lib.rs:377`、`output/vk_send.rs:158`、
`ime.rs:185,311,401,1785`、`output/held_modifiers.rs:134`、`output/key_injector.rs:112,138,
154,222,241`、`output/probe_io.rs:208`、`runtime/mod.rs:2155`、`tsf/send.rs:43`、
`tsf/output.rs:185`、`runtime/key_pipeline.rs:2391`）。新しい型を1枚被せても、この20箇所の
呼び出し判断ロジックは消えない。

**「重複」とされたInputRelay判定は、実は重複ではなかった**。`imm_cross_write`/`fallback_write`/
`run_open_chain_async`冒頭の3箇所（`runtime/open_chain.rs:154,335,379`）で見つかった同種の
判定を「別々にコピーされた重複」と見なしていたが、`open_chain.rs:369-370`のコメントに
「`fallback_write`も同様の再検出を行う（そちらは各機構ごとにviewを作り直すため独立に必要）」
と明記されている。`open_chain.rs:41-51`が理由を実害つきで記録している——起案時点の
`caps(p,k).chain`を固定すると、`.await`中にprofileがTsfNativeへ移った場合、完了時点では
適用可能な機構がchainに載っていない、という取りこぼしが生まれる。これは同一述語のコピーでは
なく、**`.await`境界をまたいだ3つの異なる時刻における可変グローバル（focus）のサンプリング点**
である。1つのインタフェースに畳むと、(a) 1回だけサンプリングして残り2箇所を消す→既知の
取りこぼしが再発する、(b) インタフェース内部に3つのサンプリング呼び出しを残す→「境界の内部に
if分岐が集中するだけ」のいずれかにしかならない。

**受信側「5種類のバッファ」の内訳が誤っていた**。当初、打鍵→core到達までに`HOOK_KEYS`・
`INPUT_DEFER`・executorのreinjectリスト・`TsfGate`の保留・`ime_off_rescue_pending`という
5種類の独立バッファがあるとしていたが、実態は以下の通りだった。

| 当初の分類 | 実態 |
|---|---|
| `HOOK_KEYS` のring buffer | `hook_channel.rs:32` — `UnsafeCell<MaybeUninit>` + `unsafe impl Sync`の**lock-free SPSC**。フックスレッド→エンジンスレッドの**スレッド境界**を越える唯一の構造。overflowラッチプロトコル（`hook_channel.rs:89-100`）付き |
| `INPUT_DEFER` のVecDeque | `input_defer.rs:1-3`のdoc「OUTPUT_GATE active中・TsfGate drainなど**複数の退避経路を集約する**」——**既に統合点として設計されている** |
| executorのreinjectリスト | `runtime/executor.rs:89-95` `guard_held: Option<RawKeyEvent>` — `Effect::Input(InputEffect::ReinjectKey)`のparkスロット。**coreが出したEffectをOSへ流す送信側の構造であり、「打鍵→core到達」の経路上には無い**（受信側への誤分類だった） |
| `TsfGate` の保留 | `tsf/tsf_gate.rs:199` `inner: HoldingGate<TsfGateMachine, RawKeyEvent>` — `timed_fsm::HoldingGate<M,T>`という汎用ジェネリックを既に使用 |
| `ime_off_rescue_pending` | `runtime/mod.rs:530,538` — 単なる`Option<RawKeyEvent>`の単発スロット |

加えて数え漏れが1件あった: `state/hook_state.rs:50` `inner: HoldingGate<SyncKeyGateMachine,
SyncKeyItem>` — `TsfGate`と**同じ`HoldingGate`ジェネリックを共有している**受信側の保留構造が
当初の5点に入っていなかった。つまり実態は「5種類が別名・別実装で存在する」ではなく、「2つは
既に同一ジェネリックを共有、1つは既に集約点、1つは送信側の誤分類、1つはスレッド境界を越える
lock-free SPSC、1つは`Option`」であり、統合の余地は当初想定より桁違いに小さい。`HOOK_KEYS`を
他と同じ型（Mutex等）に押し込むと、LL keyboard hookのホットパス（OSがタイムアウトを課す）が
ロックを取ることになり、hook timeout→キー欠落という実害の重い回帰になる——これは意味論の違い
ではなく**並行性制約の違い**という、より強い障壁である。

**A0はADR-152の再発明だった**。[ADR-152](152-keystroke-step-source-sink-pipeline.md)は
「打鍵をsource（意図の発生源）→sink（実際の配送/actuation先）のパイプラインとして再構成する
構想」であり、当初のA0が列挙するのと同じ構造を名指ししている。ADR-152は
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)と同じBlocker
（「`Applied`を詐称するとTsfNative唯一のON方向救済機構`apply_force_on_for_imm_broken`が
構造的に永久停止する」）を共有しており、着手できないまま**本文自体が一度失われた**——他ADRから
「決定3にこう書かれていた」と権威として引用され続けたが、参照先は空だった（2026-09-08に断片
から復元）。このBlockerは記録・再生を導入しても解けない意味論的制約であり、当初のA0案は一切
触れていなかった。

**受信スコープがRC1と噛み合っていなかった**。当初のA0の受信シーケンスは`RawKeyEvent`と上記
5バッファだけを対象とし、RC1（「複雑さの所在はOS APIバインディングではなくその値を信じてよい
かの推論にある」）が指す観測・belief側を含んでいなかった。実際のWin32受信入口は最低5つ
（`hook.rs:904` `hook_callback`、`tsf/win_event_obs.rs:132` `observation_event_proc`、
`app/bootstrap.rs:722` `win_event_proc`、`runtime/engine_window.rs:117` `engine_wnd_proc`、
`tray.rs:1071` `tray_wnd_proc`）、ポーリング側の観測源は`state/ime_event.rs:89`
`ObservationSource`が11バリアント（FocusProbe/ObserverPoll/Gji/ImmGetOpenStatus/
ConvBitsInference/GjiIoInference/ConvOpenInference/Tsf/HwndCache/ImmCrossProbe/
HeuristicDefault）。これらを含めない限り、記録・再生の対象範囲がbelief遷移を再現できない。

## 決定

「新しい境界を設計する」のではなく、**既に存在する境界を正確に把握し、そこにまだ届いていない
呼び出し元・そこから誤って除外されている構造を特定する**方針に転換する。3段階で構成する。

### 段階0（棚卸し）

**成果物の定義（[ADR-158](158-complexity-reduction-north-star.md)「設計原則: 宣言の強制と
SSOT化」節の適用、round1 M4への回答）**: 段階0の成果物は棚卸し文書（散文）ではなく、
[ADR-161](161-single-source-spec-generation.md)の実証実験で検証済みの機構——dylintの許可
リスト方式（既存3本＋実証実験済みの4本目`RESTRICTED_ACTUATION_CALL`と同型）による**宣言の
強制**とする。棚卸しで「本来1箇所に集約されるべき」と確認された対象ごとに、許可された呼び出し元
関数のリストをdylintのlintとしてコード化し、リストにない呼び出しをコンパイルエラーにする。
これにより、段階0は「完了したかどうか判定できない終わりのない調査」ではなく、「対象ごとに
lintが1本ずつ増える、進捗が機械的に確認できる作業」になる。

- **送信側**: 当初「`win32::send_input_safe`を唯一の送信動作点」としていたが、実機スパイク
  （後述「実機スパイク結果」節）で誤りと判明した。実際の送信機構は`send_input_safe`
  （`SendInput`）と`imm::send_ime_control`（`WM_IME_CONTROL`）の**2系統**で、両方とも実際に
  actuationを発生させている。**round4 TJ2 MF1で訂正**: 「呼び出し元20箇所（12ファイル）」は
  `send_input_safe`**のみ**の実測値であり、`send_ime_control`の呼び出し元は別途
  **10箇所（2ファイル: `ime.rs`8箇所・`ime_diagnostic.rs`2箇所）**ある（2系統合計は約30箇所・
  14ファイル）。`158-implementation-tasks.md`のTB0受入基準は両方の宣言を対象にしつつ「20箇所」
  とのみ照合しているため、実装時は「`send_input_safe`は20箇所、`send_ime_control`は10箇所」と
  分けて照合すること。

  **round4 TJ2 MF2で追記・宣言の粒度**: `send_ime_control`（`imm.rs`の`SendMessageTimeoutW`
  呼び出し1箇所に集約）は、内部で`cmd`引数により**actuation**（`IMC_SETOPENSTATUS`・
  `IMC_SETCONVERSIONMODE`、10箇所中2箇所のみ）と**probe**（`IMC_GET*`、残り8箇所）を判別して
  いる（`conv_mutation::bump()`/`probe_actuation_fence::bump()`の呼び分けが既にこの区別を
  反映）。関数名だけをキーにした許可リストを作ると、probe専用の8箇所が「actuation合流点の
  宣言」に混入し、SSOTが希釈される。**方針: `send_ime_control`の宣言キーは関数名単体ではなく
  `(関数名, cmd)`のペアとする**（TB0着手時にdylintでこの粒度が表現できるかを確認し、
  表現できない場合は「actuationを起こす`cmd`のみを許可リストの対象とし、probe専用の`cmd`は
  対象外」という運用規約で代替する）。

  呼び出し元を棚卸しし、それぞれの呼び出し前
  判定ロジック（InputRelay検出、`shadow_on` bypass等）のうち、`.await`境界をまたぐ再サンプリング
  という構造的な理由で独立が必要なもの（前述の3箇所）と、単に整理されていないだけで統合できる
  可能性があるものを仕分ける。仕分けが終わった時点で、各機構への呼び出しを許可リスト化した
  dylint lintを追加する——[ADR-161](161-single-source-spec-generation.md)の実証実験は
  `set_ime_open`という1関数だけでも、想定より多い呼び出し元（`set_ime_open_ordered`に加え
  ルートクレートのトレイトデフォルト実装）が見つかっており、`send_input_safe`/
  `send_ime_control`の合計約30箇所でも同種の見落としが起きうることを踏まえる（**round4
  TJ2 SF3で訂正**: ただし`send_input_safe`/`send_ime_control`はいずれも`pub(crate)`であり、
  `set_ime_open`のような**クレート外の呼び出し元**はこの2関数では構造的に発生しない。実際に
  想定すべき見落としは別種で、`output/key_injector.rs`のように1つの呼び出し元が複数の判断
  経路を束ねているラッパー関数がある場合、ラッパーの内側を見落とすことである）。
- **受信側**: `HOOK_KEYS`（lock-free SPSC、並行性制約によりこのまま維持）、`INPUT_DEFER`
  （既存の集約点、このまま維持）、`timed_fsm::HoldingGate<M,T>`（`TsfGate`と`SyncKeyGate`が
  既に共有するジェネリック——`ime_off_rescue_pending`をこのジェネリックに合流できるか検証する）、
  `journal.rs::JournalEntry`（19バリアント、既存の受信/送信タクソノミー）を対象に、それぞれが
  実際に何を集約しているか、まだ集約されていない呼び出し元がないかを確認する。受信側は送信側と
  異なり「単一関数への呼び出し」ではなく「特定の型（`HoldingGate<M,T>`等）を経由しているか」が
  問題になるため、dylint宣言の強制対象は関数呼び出しではなく型の使用箇所になる可能性がある——
  この違いが実際にdylintで表現しやすいかは別途検証が要る（「今後の議論」参照）。
- **観測側**: 上記5つのWin32コールバックと`ObservationSource`11バリアントを棚卸し対象に含める。
  これを欠くと段階1の記録・再生がbelief遷移を再現できない。

### 段階1（記録・再生、`journal.rs`拡張）

`journal.rs`の`JournalEntry`（`journal.rs:207`）は既に19バリアントを持ち、受信側
（KeyInput/ImeEvent/FocusTransition/TsfProbeCompleted等）と送信側（ImeActuation/
ImeOpenApplied等）を既にカバーしている。既存の`tests/journal_replay.rs`
（242行）・`tests/journals/`コーパス・`tests/drift_correction_replay.rs`を土台に、段階0で
特定した未収録の呼び出し元・観測源をこのタクソノミーに追加する。**段階0の完了を待たず着手
できる**——`JournalEntry`は事前の境界設計なしに既に成立しているため。

**round4 TJ2 MF4で訂正・「待たず着手できる」の範囲**: これが成立するのは**段階1（この節、
`TF1`タスク）のみ**である。段階1が消費するのは受信側の既存タクソノミー（`ObservationSource`
11バリアント＋5つのWin32コールバック、いずれも単なる列挙）であって、段階0の成果物（送信側の
dylint宣言）ではない。**段階2（次節、`TF2`タスク）は事情が異なる**——次節が明記する通り、
差分記録の挿入点は段階0が特定する送信機構2箇所（`send_input_safe`/`send_ime_control`）
そのものであり、`158-implementation-tasks.md`のTF2は`依存: TB0`と正しく書かれている。
本節・次節とも「段階0を待たず着手できる」と書いていたのは誤りで、**正しくは「段階1は
段階0を待たずに着手できるが、段階2はTB0（段階0の送信側宣言）の完了後に着手する」**。

これにより (1) 実機で1回起きた不具合が永久に再生可能な資産になり、`docs/experiments.md`の
散文的な実験記録がテストに置き換わる。(2) リファクタ・統合の安全性を「N本の記録トレースを
再生して送信列が一致するか」で判定できるようになる。

### 段階2（シャドー実行）

**実機スパイクで判明した訂正**: 送信動作点は`send_input_safe`だけでなく`send_ime_control`
（`WM_IME_CONTROL`経由）も含むため、差分記録の挿入点は**この2箇所の両方**が必要。
`send_input_safe`にだけ挿すと、`WM_IME_CONTROL`経由のactuation（実測で**約6〜8:1**——
round4 TJ2 MF3で訂正: 従来「約1/6」「7〜8:1で一致」の2通りの表現が併存していたが、表
（後述）の実測は初回セッション64:10≈6.4:1・再測定セッション130:17≈7.6:1で、いずれも
「約6〜8:1」の範囲に収まる。「1/6」は範囲外、「7〜8:1で一致」は初回セッションと厳密には
合わないため、この表現に統一する）を取りこぼす。`send_ime_control`側は`imm.rs`内の
`SendMessageTimeoutW`呼び出し1箇所のチョークポイントに集約されている。**round4 TJ2 MF4で
訂正**: 挿入自体は段階0（特に`TB0`によるこの2箇所の宣言）の完了後に着手する——上記
「段階1（記録・再生）」節の訂正を参照。成立すれば、日常入力が自動A/B装置になり、実験単価が
「翌日revert」から「差分ゼロ件を確認してから投入」に落ちる。

### 前提条件

段階0の棚卸しの結果、actuation合流点の**再配置**（呼び出し元の統合・削減）に踏み込む場合は、
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)/
[ADR-152](152-keystroke-step-source-sink-pipeline.md)のBlocker（Applied詐称によるON方向救済の
構造的永久停止）への対処要否を最初に確認すること。対処せずに進めると同じ場所で止まる。

## 実機スパイク結果（2026-09-09）

round1レビュー（M1・M2・M6）を実機で検証するため、`open_chain.rs`の3箇所のInputRelay判定と
`ime_apply_planner.rs::reduce_open_belief`に一時的な診断ログ（`[spike-io]`タグ、
`spike/io-boundary-instrumentation`ブランチ、本ADRのマージ対象には含めない）を追加し、
実際のGJI利用セッション（Chrome/WezTerm/VS Code、通常入力・意図的なフォーカス高速切替・
idle→バースト再現）で採取した。

**手順上の失敗と訂正**: 初回の計測は、計装コードをコミットしないままブランチをoriginへpushして
しまい、Windows側が無計装のコード（develop相当）をcheckoutしてビルドしていたことが後で判明した。
そのため初回の「InputRelay gate 0ヒット」「conv_mode乖離0件」は**無意味な結果**（ログ文自体が
バイナリに存在しなかった）だった。既存ログ（`[ime-io]`/`[ime-fallback]`）に基づく送信機構の
頻度測定と、journal.rsの`DumpTruncated`確認は既存コードに基づくため初回でも有効だったが、
`[spike-io]`タグに依存する2項目は計装をコミット・pushし直し、Windows側で再fetch・再ビルド・
再セッションを行って再測定した。以下は再測定後の確定値。

### 送信機構の実測内訳（M1の検証）

| 機構 | 実際のactuation件数(初回セッション) | 実際のactuation件数(再測定セッション) |
|---|---|---|
| `SendInput`（`win32::send_input_safe`） | 64件 | 130件 |
| `WM_IME_CONTROL`（`imm::send_ime_control`、`kind=actuation`のみ） | 10件（`kind=probe`は1352件） | 17件（`kind=probe`は2192件） |
| `VK_KANJI`トグル（KanjiToggle機構） | 0件 | 0件 |

両セッションともSendInput:WM_IME_CONTROLの比率はおよそ7〜8:1で一致しており、**再現性のある
結果**。**M1の指摘（送信動作点は1つではない）を実測で確認**。段階2の挿入点の訂正（上記）に
反映済み。KanjiToggle機構は2回とも未使用——今回のテスト環境（GJI中心）では経路自体に到達
していない。

### InputRelay判定gateの実測（M2の検証）

再測定セッションでも3箇所とも0ヒット。ログ全体を確認しても`InputRelay`という文字列は1件も
出現していない。**今回は計装が実際にビルドへ反映された状態での結果であり、有効なnull結果**
（初回のような「ログ文自体が存在しなかった」ケースではない）。今回のテストセッションに
InputRelayプロファイル対象アプリ（PowerToys「境界のないマウス」等の中継ウィンドウ）が
含まれていなかったため、gateが実際に3箇所のうちどれで捕まえられるか、という当初の検証目的
（round1 M2）自体は依然として未検証のまま残る。

検証環境（PowerToys Mouse without Borders等、2台のマシンまたは特定のネットワーク構成が必要）を
新規に用意するかどうかを検討した結果、以下の理由で**当面は見送る**と決定した。

- `open_chain.rs`のコード自体が3箇所それぞれの独立の必要性を実害付きで記録しており
  （`369-370`行、`41-51`行のモジュールdoc）、この一次資料は既に高い確信度を与えている。
- この3箇所は[ADR-119](119-injected-and-relay-key-consumption-invariant.md)が扱う
  issue #136/BUG-90の再発防止として存在し、その修正時点で既に実機検証は完了している。
- 実機検証が本当に必要になるのは、3箇所のいずれかを実際に削除・統合する変更を書く段階であり、
  その時点で対象を絞った回帰テストとして検証環境を用意する方が費用対効果が高い。

したがって、InputRelay gate 3箇所の統合可否の判断は、静的解析（上記一次資料）に基づいて行い、
実際にgateを削減する変更を提案する際に、その変更に限定したMWB環境での回帰テストを条件として
課す（本ADRの決定には含めない、着手時の個別判断とする）。

### journalの欠落実測（M6の検証）

idle→バースト再現を含むセッションで2回、既存のダンプトリガー（Alt+変換→Alt+無変換を2回連続）
を実行し、生成された`awase_journal_*.json`を確認したところ、**いずれも`DumpTruncated`
エントリなし**（`dropped_state`/`dropped_timing`/`dropped_actuation`/`dropped_key_input`が
すべて0）。今回のセッション負荷では、journal.rsのring bufferは溢れなかった。**M6が懸念した
「journalが実際にどれだけ欠落するか」について、通常利用+idle-burstの範囲では欠落なしという
実測結果が得られた。** ただし、より極端なバースト（複数アプリ間の高速連続フォーカス切替を
数十回連続、等）では未検証であり、A1の等価判定コーパスにトレースを採用する際は、引き続き
`DumpTruncated`の有無を確認する運用を維持する。

### 副次的な発見: conv_modeとfallback式の乖離（ADR-160 S1との関連）

`ime_apply_planner.rs::reduce_open_belief`に、`conv_mode`使用時の値とfallback式
（`shadow_on||candidate_visible||...`）の値を常に比較するログを追加し、再測定セッションで
**乖離0件**を確認した。同セッション中、conv_modeの読み取りを含む`WM_IME_CONTROL`のprobe
呼び出しは2192件と高頻度で発生しており、conv_modeが取得できる場面自体は多かったにもかかわらず
乖離が一度も起きなかった——**初回の「0件」とは異なり、今回は計装が正しく動作した上での有効な
結果**。ADR-160 S1（静的解析）が指摘した「conv_modeは`effective_open`の最優先決定要因」という
事実と矛盾はしないが、動的には実際に差が出るケースが（少なくともGJI中心のセッションでは）稀で
ある可能性を示す。MS-IME等での検証は未実施のため一般化には注意が必要。詳細は
[ADR-160](160-explicit-non-scope-declaration.md)へ反映する。

## 検討した代替案

### 代替案（棄却・確定）: 新規のインタフェース型を設計してWin32呼び出しを移行する

当初案。背景節で述べた通り、送信側の境界は既に存在し、受信側の重複は想定より桁違いに小さく、
かつ既存の境界に対する不用意な再設計はADR-151/152の前例通りBlockerで止まる。棚卸しを経ずに
新規設計へ進むことは棄却する。

## 今後の議論

1. 送信側20箇所の呼び出し元を実際に一覧化し、`.await`をまたぐ再サンプリングという構造的制約が
   ある箇所とそうでない箇所を仕分ける最初の棚卸し作業に着手する。
2. 仕分けが終わった送信側の対象（`send_input_safe`/`send_ime_control`）から、
   [ADR-161](161-single-source-spec-generation.md)の実証実験で検証済みのdylint許可リスト
   方式を適用し、宣言を強制する。**これが段階0の実際の成果物**（上記「成果物の定義」参照）。
3. `ime_off_rescue_pending`を`HoldingGate<M,T>`に合流できるかを検証するスパイクを行う。
4. 受信側（型経由の宣言強制がdylintで表現できるか）を、送信側（関数呼び出しの宣言強制）の
   実装で得た知見をもとに検証する。関数呼び出しより表現が難しい場合は、受信側は当面synによる
   事後スキャンに留める、という判断もありうる。
5. `journal.rs::JournalEntry`に観測側（`ObservationSource`11バリアント、5つのWin32コールバック）
   の記録を追加する設計を詰める。
6. `send_input_safe`/`send_ime_control`への差分記録（段階2）の実装方式を設計する。
7. 段階0でactuation合流点の再配置が必要と判明した場合は、ADR-151/152のBlockerへの対処を
   別途検討する（本ADRのスコープには含めない）。

## 関連

[ADR-158](158-complexity-reduction-north-star.md)（北極星、本ADRの親。「設計原則: 宣言の
強制とSSOT化」節）、
[ADR-119](119-injected-and-relay-key-consumption-invariant.md)（actuation合流点が分散した
経緯の実例）、
[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md)、
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（本ADRの前身にあたる構想、同じ
Blockerを共有）、
[ADR-156](156-unify-deferred-execution-queues.md)（キューの性質の違いによる統合断念の先例）、
[ADR-161](161-single-source-spec-generation.md)（段階0の成果物の定義に用いる宣言の強制機構、
実証実験元）、
[ADR-162](162-governance-reversal.md)（本ADRの記録・再生基盤が機能することがEの着手条件）。
