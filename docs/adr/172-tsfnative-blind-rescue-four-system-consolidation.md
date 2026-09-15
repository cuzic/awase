---
id: ADR-172
title: |-
  TsfNative ON方向救済4系統(force-on/drift correction/warmup/reassert)の現状整理と評価基盤の指針
status: |-
  **opus-adversarial-consult round1〜round3で収束（Blockerゼロ）。決定1・決定2
  ともコード変更なしで確定。** round1・round2で決定2の2つの実装案（`issue_open_
  warrant()`配線、観測ソース信頼フィルタの共有述語抽出）がいずれも実装不能と
  判明し、動機だった「force-onとdrift correctionの双方向衝突」もBUG-110修正で
  既に解消済みと判明したため、force-on側のゲートは意図的に無改造のまま据え置く
  方針に確定した（`state/platform_state.rs`・`state/ime_model.rs`のdocコメント
  に追記済み）。round3 Blocker（「実害が無い」の根拠が未測定だった件）はADR-132
  「次のアクション」1の引き継ぎで解消。実装タスクは無く、次の不具合報告時の
  内訳確認とADR-151案Dの2つの未検討観測経路の扱いのみが残る。
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

# ADR-172: TsfNative ON方向救済4系統(force-on/drift correction/warmup/reassert)の現状整理と評価基盤の指針

## ステータス

**opus-adversarial-consult round1〜round3で収束（Blockerゼロ）。**
round1（Blocker6件）はADRの事実誤認を正した。round2（Blocker3件・Should-fix6件）
は、round1で再設計した決定2がなお実装不能であること、そして決定2が前提とする
「force-onとdrift correctionの衝突」自体がBUG-110修正で既に解消済みであることを
明らかにした。round2完了後、ゼロベースで見直した結果、**決定2を「ゲート実装」
から撤退させ、「実装しない理由の記録」＋「次に触る人向けの評価手順の明記」に
方針転換した**（詳細は「round1→round2で分かったこと」節）。round3（Blocker1件・
Should-fix4件）は、その「実害が無い」という主張の根拠が未測定のまま残っていた
点を指摘し（ADR-132「次のアクション」1が測定手順を未完了のまま持っていた）、
本版でこの引き継ぎと`is_eligible_for_ime_force_on()`/`effective_open()`のdoc
コメント追記を行い収束した。

## 背景

`FeedbackPolicy::Blind`（TsfNativeアプリではIME実状態を信頼して読み戻す手段が構造的
に無い）という制約に対し、少なくとも以下4つの独立した機構が別々のADRで積み増されて
きた。

| 系統 | エントリポイント | 発火の性質 | warrant強制状態 | 解決している問題 |
|---|---|---|---|---|
| force-on | `runtime/mod.rs::apply_force_on_for_imm_broken`（`:949`の`is_eligible_for_ime_force_on`経由）/ `try_force_on_bootstrap`（`:1234`、同ゲート共有） | 周期リフレッシュ（`ir_stage_notify`）から毎tick呼ばれる | A-1（shadow、`issue_actuation_order`をログ用に呼ぶのみ。`runtime/mod.rs:1035`と`:1282`の2箇所） | TsfNative唯一のON方向救済。beliefが実際は閉じているのに開いていると誤認したケースの回復（ADR-098） |
| drift correction | `runtime/ime_refresh.rs::ir_apply_drift_correction`（ゲート: `state/platform_state.rs::check_drift_correction`） | 同じく`ir_stage_notify`から、force-on実行の直後に毎tick呼ばれる | A-1（shadow） | `desired != observed`が閾値超で継続した際の実VK再送 |
| warmup | `output/mod.rs::eager_tsf_warmup_inner`（`send_eager_tsf_warmup`/`latch_eager_warmup_without_send`の2関数経由で到達、直接呼び出し6箇所: `platform.rs`5箇所〔`:305,642,1427,1451,1610`〕、`output/vk_send.rs:692`＋`latch_eager_warmup_without_send`1箇所で計7到達経路） | フォーカス復帰・打鍵起因など複数トリガー、単一合流点を持たない | 対象外（`apply_ime_open_with_view`を経由しない） | TSF composition contextのcold-start対策（先回りVK送信でリテラル化を防ぐ） |
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

## 観測手段の探索: 3方向は不成立と確認、2方向は未検討のまま残る

ADR-151（案D、belief追随のみでactuateしない方向）の再検討条件の1つは「TsfNativeでも
IME open状態を読める観測手段が手に入ったとき」だった。2026-09-14、これを満たせないか
以下5方向で調査した。

**不成立と確認した3方向（実際にmozc本体ソース・Microsoft公式ドキュメントを確認済み）:**

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

**未検討のまま残っている2方向（round1 S5指摘、実行していない）:**

4. `ImmGetDefaultIMEWnd` + `WM_IME_CONTROL(IMC_GETOPENSTATUS)`という古典的な
   クロスプロセス読み取り経路。このリポジトリには既にスパイク実装
   （`crates/awase-windows/examples/spike_egui_ime_control_probe.rs`、
   `spike_bug112_ime_wnd_race_probe.rs`）がある。TsfNativeアプリでこの経路が成立
   するかどうかを実際に実行して確認した記録は無い。
5. `ObservationSource::Gji`/`Tsf`（authority=Actuating、型のみ存在し本番writer
   ゼロ）へ、awase自身が既に行っているGJIとのI/O観測の一部を記録する余地。
   新しい外部APIを一切追加しない改善であり、実現すればTsfNativeで`Actuating`
   権威の観測プールが構造的に空という現状（round2 B2/B3で明らかになった、決定2が
   literal実装不能だった一因）が変わりうる。

**結論: 「観測手段の探索は尽くした」とは言えない。** 1〜3は再検討できないと確認
したが、4・5は未着手のまま残る。ADR-151の案Dはこの2方向次第では状況が変わりうる
——ただし本ADRのスコープでは4・5に着手しない（決定4）。

## round1→round2で分かったこと: 決定2は「ゲートを直す」形では実装できない

round1は、当初案（`is_eligible_for_ime_force_on()`を`issue_open_warrant()`へ配線
する）が「drift correctionと同じ観測ソース信頼基準を共有する」というADRの主張を
実際には達成しない（`issue_open_warrant()`はむしろ`HeuristicDefault`を鮮度窓なしで
採用し、`FocusProbe`を除外する——drift correctionとは逆方向）ことを明らかにした。

これを受けround1改訂版は、決定2を「`check_drift_correction`が持つ
`ConvOpenInference`/`HeuristicDefault`除外判定を純粋関数として抽出し、
`is_eligible_for_ime_force_on()`にも適用する」という、より狭い再設計に変更した。
**しかしround2は、この再設計もそのままでは実装できないことを示した:**

- **対象入口を1つに限定できない**: `is_eligible_for_ime_force_on()`は
  `apply_force_on_for_imm_broken`と`try_force_on_bootstrap`の**両方が共有する
  1関数**である。関数本体を変えれば両方が対象になる。`try_force_on_bootstrap`は
  「まともな観測が無い（＝`HeuristicDefault`しか残らない）ときの最後の手段」なので、
  除外を入れるとこの機構の発火条件とほぼ完全に潰れ合う。
- **抽出した述語の型が噛み合わない**: drift correction側は`most_recent_trusted()`
  という単一の観測源で判定するが、force-on側の`effective_open()`は
  `resolve_open_at()`の5パターンの解決経路（`BaseDecision`）を通る。BUG-63の当該
  ケースである`ConvOpenInference`（confidence Medium固定）は`derive_any()`による
  `DeriveMedium`枝で決着し、`most_recent_trusted()`を経由しない。単純に移植すると
  **肝心のBUG-63ケースには一切効かず、`HeuristicDefault`ケースだけ中途半端に効く**
  という結果になる。正しく実装するには`resolve_open_at()`が返す診断API
  `DecidedBy`/`BaseDecision`を使う必要があり、これは元の再設計案には書かれて
  いなかった。
- **「観測ソースの信頼判定」がこれで4箇所目の独立判定になる**: 既に
  `check_drift_correction`、`issue_open_warrant()`のStep3、`decide_conv_
  inference_drift`（`ime_actuation.rs`、BUG-113対策）の3箇所が、それぞれ違う
  理由で「どの観測源を信頼するか」を独立に判定している。新述語は4箇所目であり、
  しかも`decide_conv_inference_drift`と**同じファイルに同居**する。ADRが
  「BUG-110型の分散判定を直す」と言いながら、新しい分散判定を1つ増やして終わる
  ことになる。

**さらに重要なのは、round2 S6が示した前提の陳腐化である**: 決定2の動機だった
「force-onとdrift correctionが互いに逆方向へ書き込みを取り合う」という双方向の
衝突は、**BUG-110追補9の修正時点で既にdrift correction側が沈黙しており、解消
済み**である。残っているのは「force-onだけが`HeuristicDefault`という弱い観測を
信じてONを書く」という**片側だけの動作**であり、これは「衝突の解消」ではなく
「TsfNativeのON方向救済を弱めるかどうか」という、ADR-151のBlocker（force-onの
構造的永久停止）に直結するリスク受容の判断である。**現在進行形の実害（衝突）は
既に無い。残っているのは理論上の懸念（BUG-63パターンの再発可能性）だけ。**

## 決定（ゼロベースで見直した結論）

当初案の却下には**ADR-157が既に示した教訓**（「発火する仕組みの上に抑止する仕組み
を重ねるより、発火源そのものの条件を直す方が優れている」— force-onとdrift
correctionの衝突を`DriftBurst`調停機構で解決しようとして実機ソークまで完了させたが、
既存ガードへの2行追加に置き換えて全体を撤回した実例）がそのまま効いた（round2 B3
「新しい分散判定を1つ増やすだけ」の却下）。

**ただし round1・round2 を経て最終的に採った「実装しない」という判断は、ADR-157とは
別の原則に基づく（round3 S2）。** ADR-157は**実機ソークを完了させた後**に撤回した
——測ってから捨てた事例である。本件はround1・round2の時点で**まだ何も実機測定して
いない**まま、設計が2回とも実装不能と判明したために撤退する——**測らずに撤退する**
判断であり、証拠の無い変更はしない（YAGNI寄り）という別の原則が働いている。この
区別は重要で、「実害が無い」という表現は「測って実害が無いと確認した」ではなく
「実害の証拠が無い（＝まだ測っていない）」の意味で使う（下記決定1・決定2、
`.claude/rules/fix-requires-evidence.md`が要求する記録・測定の精神と整合させる）。

### 決定1: warmupのGated/Actuated非対称は、今は統合しない。既知の限界として記録する

`platform.rs:1610`（Actuated系）が`resolve_warmup_ime_on()`の`off_drift_active`
ゲート（INV-B1'）を通らない非対称は実在する（ADR-132「Phase 2」節に既に記録
済み）。**ただし「実害が無い」の中身は限定的（round3 S4）: `off_drift_active`
ゲートを通らないこと自体が原因と特定された実機インシデントは無い。Actuated経路
そのものは無害ではなく、BUG-113（「@」混入、半角/全角キー単独タップでの`VK_IME_
ON`重複送信）の当事者として既にADR-149/167が一度触っている**（送信回数を3回→
2回に削減、残り1回がこのActuated経路由来として既知）。7箇所の呼び出し
（`eager_tsf_warmup_inner`への到達経路として）を単一合流点へ集約すること自体が
Gated/Actuatedのどちらかへ挙動を倒す決定を伴う以上、**ゲート非対称そのものが
原因の新規インシデントが出るまで統合しない**。

**決定: コード変更は行わない。** ADR-132の既存記述で十分にカバーされているため、
本ADRでは決定1の理由づけ以外の新規の記録は追加しない。`latch_eager_warmup_
without_send`の存在と`architecture_guard.rs:4192`の`origin`区別は、ADR-132に
無い補足情報としてこの節に残す。将来この非対称が原因のインシデントが出て統合を
検討する際の出発点にする。

**ADR-132自身の「次のアクション」1・2（`origin=`タグのgrep突合せによる内訳確定、
`apply_force_on_for_imm_broken`側との同型修正の要否判断）は未実施のまま残って
いる。** これは低コストな計装済み測定であり、次の不具合報告で確認できる（詳細は
「次のアクション」節）。

### 決定2: force-on側のBUG-63パターンは実装せず、既知の技術的負債として明記する。次に触る人への評価手順を残す

**決定: `is_eligible_for_ime_force_on()`へのコード変更は行わない。** 理由:

- 動機だった「双方向の衝突」は既に解消済み（round2 S6）。**ただし「実害が無い」は
  「測って無いと確認した」ではなく「実害の証拠が無い（＝まだ測っていない）」の
  意味である**（round3 B1）。ADR-132の「次のアクション」1・2はこの内訳（Gated/
  Actuatedのゲート非対称由来 vs force-on由来）を測る手順を未完了のまま持っており、
  ADR-172はこのタスクを引き継ぐ形で「次の不具合報告時にgrep突合せする」を
  「次のアクション」に追加する（下記）。有意な内訳が出た場合、本決定（実装しない）
  を再検討する。
- round1・round2の2ラウンドを費やしてなお、「対象入口を1つに絞れない」
  「型が噛み合わずBUG-63ケースを取り逃す」「新しい分散判定を1つ増やすだけ」という
  実装不能な設計しか出せなかった。これは個々の設計者の力量の問題ではなく、
  **「どの観測源をどれだけ信頼してよいか」がTsfNativeの構造的な盲目性
  （`FeedbackPolicy::Blind`）のもとでは、原理的に机上の議論だけで正しく決め
  切れない**ことを示している。

**この判断はコード側のdocコメントにも反映済み（round3 S1対応、本ADRと同時に
実施）**: `state/platform_state.rs:764-772`（`is_eligible_for_ime_force_on`）と
`state/ime_model.rs:355-362`（`effective_open`）はいずれも「ADR-087 item15/17で
`issue_open_warrant()`に置換予定」と書いていたが、ADR-172の検証結果（置換の効能
が無い、代替案も実装不能）を追記し、次の読み手が同じ2ラウンドを繰り返さないよう
にした。doc追記のみで挙動変更は無い。

**次にこのゲートを変更する人への評価手順（実施はしない、指針のみ）**: このADRの
round1・round2はいずれも、手で列挙したケースと机上の議論だけで設計を検証しようと
して、実装直前まで進んでから欠陥が見つかるということを2回繰り返した。次に同じ
議論を繰り返さないために、**変更を提案する際は実際の`ActuationDecision`journal
コーパスに対する差分検証を伴うこと**を推奨する。

- `DecisionSite::ForceOnRomajiCorrection`/`ForceOnBootstrap`は既にjournalへ記録
  される設計になっている（`runtime/mod.rs:1040`、`state/ime_actuation_decision.rs`）。
- ADR-163 Part D（TH1d'）が構築した不具合報告経由の実機コーパス収集基盤により、
  実際に1件（`crates/awase-windows/tests/journals/actuation_decision/
  bug-131-report-01m29kdnz.json`、37レコード）が存在する。**ただし内訳は
  `ForceOnRomajiCorrection`2件・`ForceOnBootstrap`0件と薄い**（round3 S3実測）。
  round2 B1が「除外を入れるとほぼ完全に潰れ合う」と指摘した`try_force_on_
  bootstrap`側のデータがこのコーパスには無いため、**次に変更を検討する人は、
  まずbootstrap経路を含む不具合報告を集める必要がある**——現状のコーパスだけで
  「検証できる」と思って着手しないこと。
- `state/open_warrant.rs::differential_old_gate_vs_issue_open_warrant`が「旧ゲート
  vs 新ゲートの判定をケースごとに突き合わせる」手法の前例として既にある（ただし
  対象は手で列挙したケース表であり、実機コーパスではない）。

**この評価手順自体を今すぐ整備することはしない**（消費者＝実際に変更したい人が
いない状態でテスト基盤だけ先に作るのは、消費ロジックの無い予備投資を避けるという
このリポジトリの既存の教訓と同型の無駄になる、という判断）。誰かが実際にこの
ゲートを変えたいと言い出したときに、上記の材料（journal記録・既存コーパス・
差分オラクルの前例）を使って実機データで検証してから変更する、という順序だけを
ここに残す。

### 決定3: reassertは「別カテゴリ」だが、force-on/drift correctionより先行している

**round1（S2/S3）により理由づけを訂正する。** reassertは周期tickではなく物理IMEキー
検出というイベント駆動で、根本原因も未解明（ADR-121）という点は変わらないが、
**reassertは4系統の中で唯一、既にADR-090 A-2（warrant強制）まで進んでいる**
（`runtime/mod.rs:1189-1196`、`order.would_have_blocked()`なら送信しない）。
つまり「発火の性質が違うから統合対象に含めない」という結論は妥当だが、warrantの
観点では「reassertが先行しており、force-on/drift correctionはまだA-1（shadow）
段階」というのが正確な位置づけ。決定2を見送ったため、この差は当面埋まらない
——それ自体は問題ではなく、reassertがイベント駆動で挙動変化のリスクが局所的
だったため先に進められた、という経緯の記録として残す。

**同一tickでの相互作用（round1 S3）**: settleで見送られたreassertは`TIMER_IME_
REFRESH`の同一tick上で消費され、その同じtickで`ir_stage_notify`がforce-on→drift
correctionを連続実行する。3者は独立に`Instant::now()`を取るため、境界を跨ぐ評価の
食い違いが理論上ありうる（`resolve_warmup_ime_on`のdocが「`now`はバッチ内で一貫
しない」と既に記録している問題と同型）。**決定2を見送ったため、この節で新たな
コード変更は発生しない。** 既存の限界として記録するのみ。

### 決定4: 未検討の観測経路2つは、本ADRでは着手せず別issue化する

**決定: 上記「観測手段の探索」節の4・5（`WM_IME_CONTROL(IMC_GETOPENSTATUS)`古典
経路の実機検証、`ObservationSource::Gji`/`Tsf`への記録追加）は、実行する価値が
あるかどうかも含めて別issueで検討する。本ADRでは着手しない。**
`classify_and_push`のカバレッジ穴修正（ADR-151案Dのもう1つの再検討条件）も同様に
別issue化する。

## 将来この領域を触る人への申し送り（本ADR自体のリグレッションチェックリストではない）

本ADR自体はコード変更を行わないため、直接のリグレッションチェックリストでは
ない。将来decision2/decision1を実際に実装する人向けに、これまでの調査で判明した
落とし穴を記録として残す。

1. BUG-69: TsfNative+GJI+TSF注入モードのフォーカス復帰時、実際にOSへ届くactuationが
   eager warmupだけになる窓を作らない（決定1関連）。
2. BUG-113: 半角/全角キー単独タップで`VK_IME_ON`が重複SendInputされない
   （ADR-149/167適用後の実測で3回→2回、残り1回はwarmup由来として既知。決定1に
   触る場合、この値が変わらないことを確認する）。
3. BUG-37: 物理IMEキーでの訂正がno-opに握り潰されるケースを再発させない
   （reassertの存在理由そのもの）。
4. BUG-110追補7〜9: 「force-onとdrift correctionが二重SSOTとして衝突しない」は
   既に解消済み（round2 S6）。将来決定2を実装する場合に確認すべきは衝突の有無
   ではなく、**除外を追加した結果「force-onが止まった後、誰がON方向へ戻すのか」**
   （ADR-151のBlockerと同型）。
5. ADR-098 F5: 「TsfNative」判定に`AppImeProfile`の単純matchを使うと、Windows
   Terminal（`is_effectively_tsf_native`では真だが`AppImeProfile`ではImm32Unavailable）
   を誤って対象から漏らす罠が過去2回実際に踏まれている。
6. BUG-63（「mise」→「くした」）: 将来決定2を実装する場合、`ConvOpenInference`は
   `derive_any()`の`DeriveMedium`枝で決着し`most_recent_trusted()`を経由しない
   ため、除外述語は`resolve_open_at()`の`DecidedBy`/`BaseDecision`を材料にする
   こと（round2 B2）。`most_recent_trusted()`ベースの述語では効かない。

## 非スコープ（明示的に諦めるもの）

- mozc内部IPCの解析・傍受を実装の結合先にすること（観測手段の探索1・2で不成立と
  確認済み）。
- 4系統を1つの統一state machine/調停エンジンへ完全統合すること（ADR-157の教訓により、
  それ自体を目的にしない）。
- reassertを他3系統と同じ周期tick駆動の形に作り替えること。
- `issue_open_warrant()`への全面配線（ADR-090 A-2のforce-on入口への適用、
  `try_force_on_bootstrap`を含む）。
- **決定1（warmup Gated/Actuated統合）・決定2（force-onの観測ソース信頼フィルタ）
  の実装そのもの。** 実害の記録が無い限り、本ADRの範囲では着手しない。
- `WM_IME_CONTROL(IMC_GETOPENSTATUS)`スパイクの実機検証、`ObservationSource::
  Gji`/`Tsf`への記録追加（決定4、別issue化）。
- force-onゲート変更提案の実機コーパス差分検証基盤の先行整備（決定2、消費者が
  いない状態でのインフラ投資は避ける）。

## 次のアクション

1. 本ADRに実装タスクは無い。round3（opus-adversarial-consult）で本版の記録内容
   （特に「決定2は実装しない」という結論とその根拠）の確認を得た（round3
   Blocker 1件・doc追記2箇所を本コミットで解消、収束）。
2. 決定4の2経路（`WM_IME_CONTROL`スパイク実機検証、`Gji`/`Tsf`観測記録）を
   着手するかどうかは別途ユーザー判断を仰ぐ。
3. `state/platform_state.rs:764-772`（`is_eligible_for_ime_force_on`）・
   `state/ime_model.rs:355-362`（`effective_open`）のdocコメントへ、本ADRの
   検証結果（`issue_open_warrant()`置換は効能が無い、代替案も実装不能）を
   追記済み（round3 S1対応）。
4. **ADR-132「次のアクション」1・2を引き継ぐ（round3 B1対応）**: 次に
   TsfNativeのIME関連不具合報告を受け取った際、`[tsf-eager-warmup] origin=`/
   `[warmup-gate]`/`force-ON (ImmBrokenForceOn)`をgrepし、warmup非対称由来
   （ADR-132 B1）とforce-on由来（ADR-172決定2が対象とする#6）の内訳を確定する。
   force-on由来が有意に出た場合、決定1・決定2の「コード変更なし」判断を再検討
   する。ADR-132自身の「次のアクション」2（`apply_force_on_for_imm_broken`側の
   同型修正の要否判断）は、この内訳確定と合わせて本ADRが引き取る——ADR-132側は
   このタスクをクローズ済みとして扱ってよい。
5. 決定2の評価手順で参照した実機コーパス（`bug-131-report-01m29kdnz.json`）は
   `ForceOnBootstrap`のレコードが0件（round3 S3）。決定2に将来着手する人は、
   まずbootstrap経路を含む不具合報告の収集を優先すること。

## 関連

ADR-087（`issue_open_warrant()`の設計、`effective_open()`をactuation根拠に
使わない方針）、ADR-090（A-1/A-2ロールアウト計画、決定2が当初触れようとして
非スコープ化した対象）、ADR-098（force-on/drift correctionの原型）、ADR-121
（reassert、D1）、ADR-132（INV-B1'、warmupのoff_drift_activeゲート、決定1が
扱うGated/Actuated非対称の出典）、ADR-149/151/153（TsfNative ON方向救済の設計
変遷、案D含む）、ADR-157（調停機構より発火源修正が優れていた前例、本ADRが
最終的に採った「実害が無いなら触らない」という結論の直接の先例）、ADR-163
（actuation決定のI/O分離・再生基盤、`ActuationDecision`journalと不具合報告
コーパスが決定2の「次に触る人への評価手順」の土台になっている）。
