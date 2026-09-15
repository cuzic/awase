---
id: ADR-172
title: |-
  TsfNative ON方向救済4系統(force-on/drift correction/warmup/reassert)の整理方針
status: |-
  起草（opus-adversarial-consult round1反映済み・round2待ち）。round1でBlocker6件・
  Should-fix6件を受け、決定1「挙動不変」の主張を撤回、決定2を「issue_open_warrant()へ
  の配線」から「観測ソース信頼フィルタの共有述語抽出」へ再設計、決定3をreassertが
  既にA-2済みという事実に基づき書き直した。決定はまだ確定していない。
related_adr:
  - "ADR-087"
  - "ADR-090"
  - "ADR-098"
  - "ADR-121"
  - "ADR-132"
  - "ADR-149"
  - "ADR-151"
  - "ADR-153"
  - "ADR-157"
  - "ADR-163"
---

# ADR-172: TsfNative ON方向救済4系統(force-on/drift correction/warmup/reassert)の整理方針

## ステータス

**起草。opus-adversarial-consult round1を反映済み、round2は未実施。** round1
（`opus-review-adr172-round1.md`）はBlocker 6件・Should-fix 6件を検出した。最大の
指摘は、決定2の当初案（`issue_open_warrant()`への配線）が「drift correctionと同じ
観測ソース信頼基準を共有する」というADRの主張を実際には達成しない（`issue_open_
warrant()`はむしろ`HeuristicDefault`を鮮度窓なしで採用し、`FocusProbe`を除外する
——drift correctionとは逆方向）という事実誤認だった。本版はこれを踏まえ、決定2を
「`issue_open_warrant()`への配線」から「drift correctionの観測ソース信頼フィルタを
共有述語として抽出しforce-on側にも適用する」という、より狭い変更へ再設計した。

## 背景

`FeedbackPolicy::Blind`（TsfNativeアプリではIME実状態を信頼して読み戻す手段が構造的
に無い）という制約に対し、少なくとも以下4つの独立した機構が別々のADRで積み増されて
きた。

| 系統 | エントリポイント | 発火の性質 | warrant強制状態 | 解決している問題 |
|---|---|---|---|---|
| force-on | `runtime/mod.rs::apply_force_on_for_imm_broken`（`:949`の`is_eligible_for_ime_force_on`経由）/ `try_force_on_bootstrap`（`:1234`、同ゲート共有） | 周期リフレッシュ（`ir_stage_notify`）から毎tick呼ばれる | A-1（shadow、`issue_actuation_order`をログ用に呼ぶのみ。`runtime/mod.rs:1035`と`:1282`の2箇所） | TsfNative唯一のON方向救済。beliefが実際は閉じているのに開いていると誤認したケースの回復（ADR-098） |
| drift correction | `runtime/ime_refresh.rs::ir_apply_drift_correction`（ゲート: `state/platform_state.rs::check_drift_correction`） | 同じく`ir_stage_notify`から、force-on実行の直後に毎tick呼ばれる | A-1（shadow） | `desired != observed`が閾値超で継続した際の実VK再送 |
| warmup | `output/mod.rs::eager_tsf_warmup_inner`（`send_eager_tsf_warmup`/`latch_eager_warmup_without_send`の2関数、呼び出し元7箇所: `platform.rs`5箇所〔`:305,642,1427,1451,1610`〕、`output/vk_send.rs:692`、および`platform.rs:299`の`send_eager_warmup`ラッパー経由で`runtime/ime_refresh.rs`のFocusChange処理から間接的に1箇所） | フォーカス復帰・打鍵起因など複数トリガー、単一合流点を持たない | 対象外（`apply_ime_open_with_view`を経由しない） | TSF composition contextのcold-start対策（先回りVK送信でリテラル化を防ぐ） |
| reassert | `runtime/mod.rs::reassert_explicit_physical_key`（ADR-121 D1） | 物理IMEキー検出という**イベント駆動**（`key_pipeline.rs`, `message_handlers.rs`、settle中に見送られた分は`TIMER_IME_REFRESH`の同一tickで消費される） | **A-2（強制、`:1189-1196`で`order.would_have_blocked()`なら実送信しない。4系統中唯一）** | 物理IMEキーでの訂正がno-opに握り潰される問題（BUG-37）への冪等再送。**根本原因はADR-121が明示的に未解明のまま** |

いずれも最終的に`apply_ime_open_with_view`/`apply_ime_open_with_belief`（実OS
actuationの発火点）を呼ぶ独立入口であり、`.claude/rules/fix-requires-evidence.md`の
「IME actuation合流点」表が挙げる同期/非同期経路の合流点とは別レイヤーに位置する。
`crates/awase-windows/tests/architecture_guard.rs`が`.apply_ime_open_with_view(`の
呼び出し件数を固定値（現在4）でガードしており、force-on・reassertはこのカウントに
含まれる（warmupは同関数を経由しないため対象外）。

**reassertが4系統の中で唯一、既にADR-090 A-2（warrant強制）まで進んでいる**という
事実は、当初「発火の性質が違うから別カテゴリ」という理由づけで見落としていた
（round1 S2）。決定3で扱い直す。

## 今回の出発点: 「観測手段を見つければ4系統を統合できるのでは」を検証し、否定した

ADR-151（案D、belief追随のみでactuateしない方向）の再検討条件の1つは「TsfNativeでも
IME open状態を読める観測手段が手に入ったとき」だった。2026-09-14、これを満たせないか
以下の観点で調査した。

1. mozc（GJIの元になっているOSS、Apache 2.0）自身の内部プロセス間通信を能動的に問い
   合わせる: `Status.activated`は欲しい値そのものだが、`session_id`がGJIのTIP内部
   ハンドルであり、awase側から今フォーカス中のセッションを解決する経路がプロトコル上
   存在しない。構造的に不可能と確認した。
2. 同IPCを受動的に傍受する: 候補ウィンドウ描画用の`RendererCommand`には実HWND付きで
   欲しい状態が流れているが、named pipeは1対1のrequest/response接続であり、傍受には
   TIP（各ホストアプリのプロセス内にin-processでロードされる）または
   `mozc_renderer.exe`へのコード注入、もしくはnpfs.sysに対するカーネルドライバが必要。
   「プロトコルを読んで理解する」を超えて他プロセスへ実際に手を入れる話になり、
   セキュリティソフトの誤検知・対象プロセスの不安定化という新しいリスクを背負う。
3. TSF公開API `ITfUIElementMgr`/`ITfUIElementSink`: それ自体は完全にin-process専用
   （`ITfThreadMgr`は呼び出し元スレッドのプロセス内に作られ、他プロセスの既存
   インスタンスへ外部接続する手段はドキュメント上存在しない）。クロスプロセスで唯一
   正規の経路はUI Automationの`ITextEditProvider`（`UIA_TextEdit_*`イベント、Windows
   8.1〜）だが、(a) 実装義務はGJIではなくホストアプリの編集コントロール自身にあり、
   Chrome/VS Code/WezTermのような自前描画コントロール（まさにTsfNativeに分類される
   理由そのもの）が実装しているかは未確認、(b) 実装されていても対象は変換中文字列の
   下線表示変化のみで、**IME ON/OFF（開閉）そのものは対象外**。

**round1指摘（S5）: 上記3方向は「外部からTSF-nativeアプリのIME状態を覗く」新しい
経路の探索であり、以下の2つを見落としていた。**

4. **`ImmGetDefaultIMEWnd` + `WM_IME_CONTROL(IMC_GETOPENSTATUS)`という古典的な
   クロスプロセス読み取り経路**。このリポジトリには既にスパイク実装
   （`crates/awase-windows/examples/spike_egui_ime_control_probe.rs`、
   `spike_bug112_ime_wnd_race_probe.rs`）があり、手順のdocコメントも書かれている。
   TsfNativeアプリでこの経路が成立するかどうかを実際に実行して確認した記録は
   本ADRには無い。**「不成立」と断定する前に、このスパイクを実機で走らせて結果を
   引用する必要がある**（未実施）。
5. **awase自身が既に持っている観測を、型としては存在するが本番で一度も使われて
   いない`ObservationSource::Gji`/`Tsf`（authority=Actuating）へ記録する**という、
   新しい外部APIを一切追加しない改善余地。`state/evidence.rs`にこの2つは型として
   宣言されているが、本番コードでの`Observed::<evidence::Gji>`/`<evidence::Tsf>`の
   構築点はゼロ（テストとdocのみ）。GJI戦略が実際に行っているGJIとのI/O
   （`GjiIoInference`という別ソースは既にある）の一部をここへ記録できれば、
   TsfNativeで`Actuating`権威の観測プールが構造的に空という現状（決定2の再設計の
   根拠そのもの、後述）が変わる可能性がある。

**結論（訂正）: 今回調査した3方向（mozc IPC能動/受動傍受、TSF公開UI要素API）に
限っては統合の決め手にならないと確認したが、4・5は未検討のまま残っている。
「観測手段の探索は尽くした」とは言えない。** ADR-151の案Dは、少なくとも1〜3の
経路では再検討できないことが分かったが、4・5次第では状況が変わりうる。決定4で
扱いを明確にする。

## 決定

観測手段探しに全面的には賭けず、代わりに**ADR-157が既に示した教訓**（「発火する
仕組みの上に抑止する仕組みを重ねるより、発火源そのものの条件を直す方が優れている」
— force-onとdrift correctionの衝突を`DriftBurst`調停機構で解決しようとして実機
ソークまで完了させたが、既存ガードへの2行追加に置き換えて全体を撤回した実例）を、
今回の4系統整理の設計原則として明示的に採用する。

具体的には以下を提案する。

### 決定1: warmupの呼び出しを整理する（挙動不変ではない、Gated/Actuatedの扱いが決定の本体）

**round1指摘（B6）により「挙動不変の構造リファクタ」という当初の前提は撤回する。**
7箇所の呼び出しは実際には2系統に分かれる:

- **Gated系**（`platform.rs:305,1427,1451`等）: `resolve_warmup_ime_on()`経由で
  `check_drift_correction`と同一の`off_drift_active`ゲート（INV-B1'）を通す。
- **Actuated系**（`platform.rs:1610`、`WarmupImeOn::from_actuated`）: このゲートを
  **通らない**。force-onが`SetOpen(true)`を適用した直後にもここを通るため、drift
  correctionがOFF方向へ送り続けている最中でも随伴warmup（`VK_IME_ON`）が飛びうる
  （既知の限界、ADR-132「Phase 2」節）。

単一の合流点へ集約し「呼ぶかどうかの条件だけを渡す」形にすると、このGated/Actuated
の作り分けをどちらかへ倒すことになり、挙動が変わる。**決定: 集約するかどうか自体を
決定の本体とし、以下を明示する:**

- 倒すなら「Actuated系にもINV-B1'ゲートを適用する」方向（=warmup全系統をGatedに
  揃える）にする。逆方向（Gatedを無条件化する）はBUG-110型の衝突を広げるため採らない。
- `latch_eager_warmup_without_send`（`output/mod.rs:1150`、`platform.rs:1610`の
  else分岐、ADR-149 M3）は`compute_focus_probe_grace`の唯一の入力
  `eager_warmup_sent_ms`の供給元であり、集約後も必ず保存する。
- `architecture_guard.rs:4192`が固定する`origin`（`"gated"`/`"actuated"`）区別を
  維持するか、統合後の新しい診断手段に置き換えるかを実装前に決める。

### 決定2: force-on側に、drift correctionと同じ観測ソース信頼フィルタを共有述語として適用する（再設計）

**round1（B1〜B5）で当初案（`issue_open_warrant()`への配線）を検証した結果、以下が
判明し、当初案は不採用とする。**

- `issue_open_warrant()`（ADR-087/090）は`HeuristicDefault`を**除外せず、鮮度窓
  なしで明示的に採用する**（Step 4a）。drift correctionの除外ガードとは**逆方向**
  であり、「観測ソース信頼基準を揃える」という当初の効能は達成されない。
- TsfNativeでは`Actuating`権威の観測源（`Gji`/`Tsf`）が本番で一度も記録されない
  ため、`issue_open_warrant()`はTsfNativeにおいて実質「`desired_open` +
  `IntentStore` + 鮮度窓なし`HeuristicDefault`」に縮退する。これは当初案が書いて
  いない、別の変更である。
- `issue_open_warrant()`は`FocusProbe`（`BeliefOnly`権威）を必ず除外する。drift
  correction側は「`FocusProbe`除外はBUG-16/BUG-20型の固着を再導入するリスクが
  ある」として**意図的に除外していない**。当初案はこの拒否済みの基準をforce-on側
  だけに持ち込むことになる。
- `is_eligible_for_ime_force_on()`の呼び出し元は2箇所あり（`apply_force_on_for_
  imm_broken`と`try_force_on_bootstrap`）、後者はコード中に「差分オラクルが判明
  した中で最大の挙動変化」「A-2で倒すのは最後に回す」と明記されている
  （`runtime/mod.rs:1277-1281`、ADR-090 A-R1/§4.9）。当初案は前者しか対象にして
  いなかった。
- 当初案は実質ADR-090 §2.A A-2そのものであり、ADR-090自身が「規模大・実機ソーク
  必須・挙動変化最大9通り」と評価済みの作業を、軽い「配線」と言い換えていた。

**再設計: `issue_open_warrant()`への配線（ADR-090 A-2のforce-on入口への適用）は
別途トラッキングし、本ADRでは扱わない。代わりに、drift correctionが`check_drift_
correction`内に持つ「`ConvOpenInference`/`HeuristicDefault`を明示意図なしでは
信頼しない」という除外判定だけを純粋関数として抽出し（`state/ime_actuation.rs`、
`.claude/rules/ime-belief-architecture.md`の`classify_*`規約に沿う）、
`check_drift_correction`と`is_eligible_for_ime_force_on()`の両方から呼ぶ。**

この案を選ぶ理由:

- warmup↔drift correction間で既に同型の「共有述語」パターン（`resolve_warmup_
  ime_on`が`check_drift_correction`と同じ判定式を再利用）が実際に機能しており、
  前例がある。
- `FocusProbe`の扱いを変えない（`is_eligible_for_ime_force_on()`は引き続き
  `effective_open()`を主たる根拠として使い、その上に`ConvOpenInference`/
  `HeuristicDefault`単独ケースの除外だけを追加する）ため、B2が指摘した「force-on
  が止まるシナリオ」（`FocusProbe`実測がある状況）を新たに作らない。
- ADR-090 A-2の「大規模・実機ソーク必須・挙動変化9通り」というスコープを本ADRの
  対象から切り離せる——`is_eligible_for_ime_force_on()`のBUG-63パターン
  （`effective_open()`を直接actuationの根拠にする構造）自体は解消しないが、
  それはADR-087/090の別イニシアチブとして残し、本ADRは「BUG-110型の観測ソース
  非対称」という当初の動機だけに絞る。

**この再設計でも対応が必要な既知の論点（round1 B5、チェックリストへ追加）:**

- BUG-63（「mise」→「くした」誤入力）の再現条件（`ConvOpenInference`単独での
  force-on eligibility）を、除外導入後も別の経路で再発させないこと。
- ADR-151のBlocker（force-onの構造的永久停止）を再導入しないこと——除外を追加
  した結果、force-onの発火頻度が実機ソークで実質ゼロに落ち込んでいないか確認する。
- 決定3で扱う同一tick内でのreassert/force-on/drift correctionの相互作用
  （round1 S3、下記チェックリスト6参照）。

**対象を`apply_force_on_for_imm_broken`（1入口）に限定する。`try_force_on_
bootstrap`はADR-090 A-R1/§4.9の指示どおり対象外とし、別途最後に検討する。**

### 決定3: reassertは「別カテゴリ」だが、決定2が追いつく相手として位置づける

**round1（S2/S3）により理由づけを訂正する。** reassertは周期tickではなく物理IMEキー
検出というイベント駆動で、根本原因も未解明（ADR-121）という点は変わらないが、
**reassertは4系統の中で唯一、既にADR-090 A-2（warrant強制）まで進んでいる**
（`runtime/mod.rs:1189-1196`、`order.would_have_blocked()`なら送信しない）。
つまり「発火の性質が違うから統合対象に含めない」という結論は妥当だが、warrantの
観点では逆に「reassertが先行しており、決定2はその水準にforce-onを近づける動き」
と位置づけるのが正確。

**同一tickでの相互作用（round1 S3）**: settleで見送られたreassertは`TIMER_IME_
REFRESH`の同一tick上で消費され、その同じtickで`ir_stage_notify`がforce-on→drift
correctionを連続実行する。3者は独立に`Instant::now()`を取るため、`DRIFT_
CORRECTION_THRESHOLD_MS`等の境界を跨ぐ評価の食い違いが理論上ありうる。**決定2を
共有述語の抽出（issue_open_warrantを経由しない設計）に留めたため、warrant評価の
二重化・時刻ズレというS3の主要な懸念自体は今回のスコープでは発生しない**——ただし
reassertが独自に呼ぶ`issue_actuation_order`とforce-on/drift correctionのtick評価
との間の一般的な時刻ズレは既存の限界として残る（`resolve_warmup_ime_on`のdocが
「`now`はバッチ内で一貫しない」と既に記録している問題と同型）。新たに悪化させない
ことを決定2のチェックリストに含める。

### 決定4: 「観測手段が見つかれば」に全面的には賭けないが、未検討の2経路は残す

ADR-151案Dの再検討条件のうち観測手段側は、**今回検討した3方向（mozc IPC能動/受動
傍受、TSF公開UI要素API）に限って**閉じたことを記録する。ただし以下は未検討のまま
残し、「探索は尽くした」と誤読されないようにする（round1 S5）:

- `WM_IME_CONTROL(IMC_GETOPENSTATUS)`経路の実機検証（既存スパイク
  `spike_egui_ime_control_probe.rs`を実行して結果を記録する、未実施）。
- `ObservationSource::Gji`/`Tsf`（Actuating権威、型のみ存在し本番writerゼロ）への
  記録を追加する余地の検討。

これらは本ADRのスコープ外として別issue化する。`classify_and_push`のカバレッジ穴
修正（案Dのもう1つの再検討条件）も同様に独立issueとして切り出す。

## 落としてはいけない既知シナリオ（変更時のチェックリスト）

1. BUG-69: TsfNative+GJI+TSF注入モードのフォーカス復帰時、実際にOSへ届くactuationが
   eager warmupだけになる窓を作らない。
2. BUG-113: 半角/全角キー単独タップで`VK_IME_ON`が重複SendInputされない
   （ADR-149/167適用後の実測で3回→2回、残り1回はwarmup由来として既知——決定1が
   `should_send_accompanying_warmup`分岐に触るため、この値が変わらないことを
   確認する）。
3. BUG-37: 物理IMEキーでの訂正がno-opに握り潰されるケースを再発させない
   （reassertの存在理由そのもの）。
4. BUG-110追補7〜9: force-onとdrift correctionが同一シナリオで二重SSOTとして
   衝突しない（ADR-157の反面教師、決定2の直接の動機）。
5. ADR-098 F5: 「TsfNative」判定に`AppImeProfile`の単純matchを使うと、Windows
   Terminal（`is_effectively_tsf_native`では真だが`AppImeProfile`ではImm32Unavailable）
   を誤って対象から漏らす罠が過去2回実際に踏まれている。
6. BUG-63（「mise」→「くした」）: 決定2の除外導入で`ConvOpenInference`単独での
   force-onを止めるのは意図した改善だが、その結果ON方向救済が別のシナリオで
   消えないこと。ADR-151のBlocker（force-onの構造的永久停止）を実機ソークの
   発火頻度で確認する。
7. 決定3のS3: reassert・force-on・drift correctionが同一tickで評価される際の
   `Instant::now()`ズレを、決定2の変更が新たに悪化させないこと。

## 非スコープ（明示的に諦めるもの）

- mozc内部IPCの解析・傍受を実装の結合先にすること（前節の調査で閉じたと判断）。
- 4系統を1つの統一state machine/調停エンジンへ完全統合すること（ADR-157の教訓により、
  それ自体を目的にしない）。
- reassertを他3系統と同じ周期tick駆動の形に作り替えること。
- `issue_open_warrant()`への全面配線（ADR-090 A-2のforce-on入口への適用、
  `try_force_on_bootstrap`を含む）。本ADRの決定2はこれを行わず、別途トラッキング
  する。

## 次のアクション

1. **決定1**: Gated/Actuatedの作り分けをどちらへ倒すか（Actuated系にINV-B1'
   ゲートを適用する方向を推奨）を確定した上で着手する。`latch_eager_warmup_
   without_send`の保存と`architecture_guard.rs`の`origin`区別の扱いを実装計画に
   明記する。
2. **決定2**: (i) 対象を`apply_force_on_for_imm_broken`1本に限定すると明記する
   （`try_force_on_bootstrap`は対象外）。(ii) `check_drift_correction`から
   `ConvOpenInference`/`HeuristicDefault`除外判定を純粋関数として抽出し
   `state/ime_actuation.rs`へ置く。(iii) `is_eligible_for_ime_force_on()`へ
   同判定を追加する。(iv) チェックリスト項目6（BUG-63再発・ADR-151 Blocker
   頻度）を実機ソークで確認する。(v) `.claude/rules/fix-requires-evidence.md`の
   「キー選択」ファミリー該当につき、golden更新か`docs/known-bugs/BUG-NNN.md`
   のどちらかを添える。
3. 本ADRをopus-adversarial-consult round2にかけ、収束後に実装へ進む。

## 関連

ADR-087（`issue_open_warrant()`の設計、`effective_open()`をactuation根拠に
使わない方針）、ADR-090（A-1/A-2ロールアウト計画、決定2が当初触れようとして
非スコープ化した対象）、ADR-098（force-on/drift correctionの原型）、ADR-121
（reassert、D1）、ADR-132（INV-B1'、warmupのoff_drift_activeゲート、決定1が
扱うGated/Actuated非対称の出典）、ADR-149/151/153（TsfNative ON方向救済の設計
変遷、案D含む）、ADR-157（調停機構より発火源修正が優れていた前例）、ADR-163
（actuation決定のI/O分離・再生基盤、TH1eが本ADR的な「実削除+差分ゼロ証明」の
発効条件になっている）。
