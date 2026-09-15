---
id: ADR-172
title: |-
  TsfNative ON方向救済4系統(force-on/drift correction/warmup/reassert)の整理方針
status: |-
  起草（opus-adversarial-consult未実施）。決定はまだ確定していない。
related_adr:
  - "ADR-098"
  - "ADR-121"
  - "ADR-149"
  - "ADR-151"
  - "ADR-153"
  - "ADR-157"
  - "ADR-163"
---

# ADR-172: TsfNative ON方向救済4系統(force-on/drift correction/warmup/reassert)の整理方針

## ステータス

**起草。opus-adversarial-consultはまだ実施していない。** 本ADRは実装計画ではなく、
2026-09-14に行った調査（後述）を踏まえた「次にどこへ手を入れるべきか」の一次案。
確定させる前にレビューを通す。

## 背景

`FeedbackPolicy::Blind`（TsfNativeアプリではIME実状態を信頼して読み戻す手段が構造的
に無い）という制約に対し、少なくとも以下4つの独立した機構が別々のADRで積み増されて
きた。

| 系統 | エントリポイント | 発火の性質 | 解決している問題 |
|---|---|---|---|
| force-on | `runtime/mod.rs::apply_force_on_for_imm_broken` → `force_on_and_correct_romaji`（ゲート: `state/ime_actuation.rs::force_on_attempt_allowed`） | 周期リフレッシュ（`ir_stage_notify`）から毎tick呼ばれる | TsfNative唯一のON方向救済。beliefが実際は閉じているのに開いていると誤認したケースの回復（ADR-098） |
| drift correction | `runtime/ime_refresh.rs::ir_apply_drift_correction` | 同じく`ir_stage_notify`から、force-on実行の直後に毎tick呼ばれる | `desired != observed`が閾値超で継続した際の実VK再送 |
| warmup | `output/mod.rs::send_eager_tsf_warmup`（呼び出し元6箇所: `platform.rs`4箇所、`output/vk_send.rs`、`runtime/ime_refresh.rs`） | フォーカス復帰・打鍵起因など複数トリガー、単一合流点を持たない | TSF composition contextのcold-start対策（先回りVK送信でリテラル化を防ぐ） |
| reassert | `runtime/mod.rs::reassert_explicit_physical_key`（ADR-121 D1） | 物理IMEキー検出という**イベント駆動**（`key_pipeline.rs`, `message_handlers.rs`） | 物理IMEキーでの訂正がno-opに握り潰される問題（BUG-37）への冪等再送。**根本原因はADR-121が明示的に未解明のまま** |

いずれも最終的に`apply_ime_open_with_view`/`apply_ime_open_with_belief`（実OS
actuationの発火点）を呼ぶ独立入口であり、`.claude/rules/fix-requires-evidence.md`の
「IME actuation合流点」表が挙げる同期/非同期経路の合流点とは別レイヤーに位置する。
`crates/awase-windows/tests/architecture_guard.rs`が`.apply_ime_open_with_view(`の
呼び出し件数を固定値でガードしており、force-on・reassertはこのカウントに含まれる
（warmupは同関数を経由しないため対象外）。

## 今回の出発点: 「観測手段を見つければ4系統を統合できるのでは」を検証し、否定した

ADR-151（案D、belief追随のみでactuateしない方向）の再検討条件の1つは「TsfNativeでも
IME open状態を読める観測手段が手に入ったとき」だった。2026-09-14、これを満たせないか
3方向で調査した。

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

**結論: 観測手段の発見によって4系統を統合する道は、現時点では閉じている。** ADR-151
の案Dはこの条件が満たされない限り再検討できない。もう1つの再検討条件（
`classify_and_push`のカバレッジ穴修正）は独立に着手可能なタスクとして残っている
（本ADRのスコープ外、別途トラッキングする）。

## 決定

観測手段探しを諦め、代わりに**ADR-157が既に示した教訓**（「発火する仕組みの上に
抑止する仕組みを重ねるより、発火源そのものの条件を直す方が優れている」— force-onと
drift correctionの衝突を`DriftBurst`調停機構で解決しようとして実機ソークまで完了
させたが、既存ガードへの2行追加に置き換えて全体を撤回した実例）を、今回の4系統
整理の設計原則として明示的に採用する。

具体的には以下を提案する（実装順は独立でよい）:

### 決定1: warmupの6箇所の呼び出しを単一の合流点に集約する（構造のみ、判断ロジック不変）

他3系統は既に単一関数がエントリポイントだが、warmupだけが6箇所に散っている。まずこれ
を`send_eager_tsf_warmup`一本の呼び出しに集約し、各呼び出し元は「呼ぶかどうかの条件」
だけを渡す形にする。挙動を変えない純粋なリファクタであり、他系統との関係を論じる前提
条件として先に片付ける。

### 決定2: force-on × drift correctionの発火条件の重なりを検証する（未実施、次の調査）

両者は同じ`ir_stage_notify`から毎tick連続実行される。ADR-157はこの2つが実際に衝突した
前例（BUG-110追補7）であり、そのときの根治は「調停」ではなく「drift correction側の
除外ガードに1バリアント足す」だった。同じパターンが他にも隠れていないか、
`force_on_attempt_allowed`と`ir_apply_drift_correction`のゲート条件を並べて、両方が
同時に真になりうる状態空間が存在するかを調べる。存在する場合、ADR-157の前例に倣い
「どちらのゲートが本来除外すべきだったか」を特定して直す。調停機構（優先順位表・専用
リトライタイマー等）を新設する案は、それ単独では採用しない——ADR-157の反例が示す通り、
複雑さに見合う効果が無いことが多い。

### 決定3: reassertを「同カテゴリ」として扱わない

reassertは周期tickではなく物理IMEキー検出というイベント駆動で、根本原因も未解明
（ADR-121）。他3系統と発火の性質が異なる以上、無理に同じ統合の対象に含めない。
ドキュメント上も「TsfNativeのON方向救済」という括りではなく、「periodic-tick駆動の
2系統（force-on/drift correction）」と「event駆動の1系統（reassert）」を明示的に
別カテゴリとして記述し直す。

### 決定4: 「観測手段が見つかれば」に賭けない

ADR-151案Dの再検討条件のうち観測手段側は当面閉じたことを記録し、`classify_and_push`
のカバレッジ穴修正（案Dのもう1つの条件）を独立issueとして切り出す。これは今回の4系統
整理そのものではないが、次にこの領域へ戻ってくる際の入り口を1つに絞る。

## 落としてはいけない既知シナリオ（変更時のチェックリスト）

1. BUG-69: TsfNative+GJI+TSF注入モードのフォーカス復帰時、実際にOSへ届くactuationが
   eager warmupだけになる窓を作らない。
2. BUG-113: 半角/全角キー単独タップで`VK_IME_ON`が重複SendInputされない
   （現状3回→2回、残り1回はwarmup由来として既知）。
3. BUG-37: 物理IMEキーでの訂正がno-opに握り潰されるケースを再発させない
   （reassertの存在理由そのもの）。
4. BUG-110追補7: force-onとdrift correctionが同一シナリオで二重SSOTとして衝突しない
   （ADR-157の反面教師）。
5. ADR-098 F5: 「TsfNative」判定に`AppImeProfile`の単純matchを使うと、Windows
   Terminal（`is_effectively_tsf_native`では真だが`AppImeProfile`ではImm32Unavailable）
   を誤って対象から漏らす罠が過去2回実際に踏まれている。

## 非スコープ（明示的に諦めるもの）

- mozc内部IPCの解析・傍受を実装の結合先にすること（前節の調査で閉じたと判断）。
- 4系統を1つの統一state machine/調停エンジンへ完全統合すること（ADR-157の教訓により、
  それ自体を目的にしない）。
- reassertを他3系統と同じ周期tick駆動の形に作り替えること。

## 次のアクション

1. 決定1（warmup呼び出し集約）は独立に着手可能、低リスク。
2. 決定2（force-on×drift correctionのゲート重なり調査）はコードレベルの検証が必要、
   本ADR時点では未実施。
3. 本ADRをopus-adversarial-consultにかけ、収束後に実装へ進む（不具合診断後は
   すぐ実装せずADR起票→opus-adversarial-consultで収束させてから実装する、という
   このリポジトリの既存フローに倣う）。

## 関連

ADR-098（force-on/drift correctionの原型）、ADR-121（reassert、D1）、ADR-149/151/153
（TsfNative ON方向救済の設計変遷、案D含む）、ADR-157（調停機構より発火源修正が優れて
いた前例）、ADR-163（actuation決定のI/O分離・再生基盤、TH1eが本ADR的な「実削除+差分
ゼロ証明」の発効条件になっている）。
