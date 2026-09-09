# ADR-158: アーキテクチャ複雑性根絶の北極星 — 記録・再生基盤／非スコープ宣言／単一仕様生成／ガバナンス反転（Bは棄却）

## ステータス

**北極星として起票。opus-adversarial-consult round1完了・反映済み（本版）。round2待ち。**
round1で検出したMust-fix 8件・Should-fix 11件を反映し、**採用A（特にA0）を大幅に組み替えた**
（「新規境界の設計」→「既存境界の棚卸しと未収束呼び出し元の特定」）。採用C・DはAへの依存が
論理的に不成立と判明したため独立させた。採用Eの実装コスト見積りを訂正した。round1の指摘は
[「今後の議論」節](#今後の議論)の履歴として要約を残す。Cの具体案(C1/C2/C3)は判断材料と期限を
追記したが、実施可否そのものはユーザー確認待ちのまま未確定。

## 背景

### 経緯

2026-09-08〜09-09、ユーザーからの「設計が複雑化しすぎていないか」という問いを起点に、
以下の調査を実施した。

1. 実コード・ADR・`.claude/rules/*.md`・テストを読む8方向の並行棚卸し（別セッション7エージェント）。
2. 棚卸し結果を独立したOpusエージェントに渡し、根本原因の特定と非連続的な解決策の立案を依頼。
3. ユーザー指摘を受け、採用Aの内容を「Win32呼び出しの計装」から「受信/送信シーケンスという
   コンポーネント境界を先に設計する」方針(A0)に転換。
4. 別ワークツリー（`docs/adr158-complexity-reduction`ブランチ）上で本ADRをopus-adversarial-consult
   round1にかけ、Must-fix 8件・Should-fix 11件を検出。**特にA0の技術的前提が実コードと矛盾する
   ことが判明**し、本版でA0を組み替えた。

### 確認された事実（一次資料、測定日・スコープ注記つき）

**本ADR自身が「ドキュメントの数値が資料間で食い違う」ことを問題視しているため、以下の数値には
測定日・測定範囲を明記する。特記なき限り2026-09-08〜09、`crates/awase-windows/src`スコープ。**

- **規模**: `awase-windows/src` 78,400行（`state/` 19,937、`runtime/` 13,201、`tsf/` 11,953、
  `output/` 7,142）。ルート`awase`は実質22,725行（32,468−`src/engine/tests.rs` 9,743）。
- **Win32呼び出し**: `windows::Win32`を含む行は161（**うち105行が`use`文**、round1 S1で判明）。
  実際のAPI呼び出し形（`Name(`）の出現は約307件。「Win32 API表面積は小さい」という結論自体は
  307件でも成立するが、`unsafe`箇所数443（旧記載444から訂正）と合わせて工事量を見積もる際は
  この307という数字を使うこと。
- **純粋層**: `pub fn classify_*`は6個。`state/`20k行の大半は証拠の保管・照合・失効管理
  （`observation_store.rs` 2,234行、`platform_state.rs` 2,564行、`open_warrant.rs` 1,428行等）。
- **proptestゼロ**: `awase-windows`にproptestは1本もない（ルート`awase`と`timed-fsm`にはある）。
- **ADR-151/152は本文が一度もcommitされていなかった**（index.mdの当該行に明記）。2026-09-08に
  断片から復元済みだが、「散文グラフは実体のない節点を持ちうる」という構造的欠陥は未解消。
- **直近30日のコミット**: `docs` 71 > `fix` 47 > `diag` 10 > `test` 7（round1で69/46から訂正）。
  文書生産がバグ修正を上回り、テストはその1/7。
- **ADR本数**: index.md記載は157だが、これはindex.md自身を含む数え方の混入で、ADR本体は156本
  （round1 S4で判明）。
- **known-bugs.md**: 16,825行（round1計測時点、旧記載16,892から訂正）、123バグエントリ
  （BUG-113単体1,210行）。
- **`Effect`型**: トップレベル4バリアントだが、葉まで展開すると6
  （`InputEffect`2+`TimerEffect`2+`ImeEffect`1+`UiEffect`1、round1 S3）。「それ自体は小さい」
  という結論は維持されるが、数字は訂正する。
- **actuation入口の数え方**: `fix-requires-evidence.md`の「5」（合流点＝新しいgateを足す場所）、
  `architecture_guard.rs:1161`の「6種」（関数名の種類の呼び出し箇所数、と自己申告）、同ファイル
  1182行の「11経路」（force-write/observation-based correction/Engine intentを含む意味論的経路、
  ADR-087参照）は、**round1レビューにより「不一致」ではなく「同じ語『入口』が3つの異なる概念に
  使われている」**と判明した（S5、`architecture_guard.rs`自身が各数字の粒度を宣言していたため）。
  この訂正版の方がRC3（型ではなく名前で概念を管理している）の証拠としてもより正確である。
  実測: `.apply_ime_open_with_view(`=4箇所、`.apply_ime_open_with_belief(`=2箇所。
- **`shadow_on=None`によるbypass**: round1 S6で、当初「2箇所で別々に実装」としていた記述が
  不正確と判明。実際は `runtime/open_chain.rs:329` の `view.control.shadow_on = None;`
  （viewフィールドの書き換え、**1箇所のみ**）と、`apply_ime_open_with_belief(order, None,
  belief)`として引数に`None`を渡す`key_pipeline.rs:1087`/`ime_refresh.rs:873`（**別レイヤの
  別機構**）を混同していた。
- **グローバル可変状態**: CLAUDE.mdの「`INPUT_RELAY_APPS`が唯一の例外」という記述は不正確で、
  他に`HOOK_IME_MODE_DIAGNOSTICS`・`PROFILE_DESCRIPTIONS`・`LOG_WRITER_STATE`の3系統が確認できた
  （round1 S7で`GJI_CLSID`は「書き込み1回きりで以後不変」と判明したため列挙から除外した）。
  dylintは3本（CLAUDE.mdの「2つ」は誤記、`Cargo.toml:26-31`で確認）。
- **キュー/リングバッファ**: 全体で12個。うち`journal.rs`の`JournalEntry`（`journal.rs:207`）は
  **19バリアント**で、受信側（KeyInput/ImeEvent/FocusTransition/TsfProbeCompleted等）と
  送信側（ImeActuation/ImeOpenApplied等）を既にカバーしている（round1 S2で判明。当初「部分実装」
  としていた記述を訂正）。`tests/journal_replay.rs`・`tests/journals/`コーパスも既存。
- **ADR-156**（2026-09-04）は「defer/replay」意味論を持つ5キューに限って統合を検討し、
  `DeferredExecutionQueue<T>`への大規模統合を不採用とした。**round1 M5で判明した重要な訂正**:
  当初本ADRは「断念の真の理由はオラクル不在」と主張していたが、ADR-156の一次資料
  （`156-...md:87-96`）が挙げる理由は3つあり、うち2つ（構造的な性質の違い、実際の回帰は
  複数キュー間ではなく1キュー内の2窓口間だった、という因果）は**オラクルの有無と無関係**である。
  RC2の主張は後段で訂正する。

### 根本原因（RC1〜RC4）

**RC1（生成因・ドメイン由来、消去不可）**: TsfNativeでIME open状態の真値が読めないため、
awaseは*belief*を持たざるを得ず、beliefは必然的に証拠源・更新規則・乖離検出・乖離修復の
4点セットを要求する。決定的に悪いのは、**修復（VK送信）が証拠自体を汚染する閉ループ**である点
（BUG-113の「@」、CapsLock汚染、GJIによるキー横取り等）。

**RC2（増幅因、round1で部分的に訂正）**: 正しさを判定できる装置が実機にしかなく、CIのガード
（`architecture_guard.rs`のテキスト走査、golden）は多くの仮説を反証できない。反証コストが
高いほど、知見の保存先として「テスト」より「文章」が合理的になる（docs 71 > fix 47 > test 7が
このインセンティブの直接的な計測値）。

**round1での訂正**: 当初「ADR-156が統合を断念した真の理由はオラクル不在」と主張したが、これは
過大な一般化だった。ADR-156の断念理由は(1)構造的な性質の違い（入力側/出力側/TSF固有の状態機械/
別種コマンドキューが混在）、(2)因果的な反証（唯一の実例は複数キュー間ではなく1キュー内の2窓口間
の配線忘れだった）、(3)コスト（型設計・全呼び出し元の移行・実機ソーク）の3つで、**オラクルが
解くのは(3)の一部だけ**。(1)(2)はA（記録・再生基盤）を導入しても解消しない。RC2は「反証コストの
高さが知見を散文へ追いやる」という部分では引き続き成立するが、「統合が進まない主因はオラクル
不在である」という主張は撤回し、「オラクルがあれば(3)のコスト判断は変わりうるが、(1)(2)は
別途評価が要る」と修正する。

**RC3（保存形式）**: ADR156本・known-bugs.md 16,825行・rules 577行・architecture_guard.rs
4,630行・CLAUDE.md・project memoryという6系統に同じ事実が手書きコピーされ、参照整合性がない。
CLAUDE.mdの誤記（dylint本数、共有可変状態の例外主張）、幻のADR-151/152、`shadow_on`の命名衝突
（round1でこの命名衝突自体の記述にも誤りがあったと判明——RC3の主張を裏付ける皮肉な実例）は
すべて同根——**概念の同一性/差異を型ではなく名前で管理している**。

**RC4（ラチェット）**: `fix-requires-evidence.md`は「テストか記録を添えろ」であり削除条項が
ない。**round1 M7で訂正**: `architecture_guard.rs`の85テストはすべて`assert_eq!`による完全ピン
方式で、増加・減少の両方を検知する（「増加のみ検知」という当初の記述は誤り）。RC4の実態は
ガードの検知能力の問題ではなく、**「`expected`値が増える方向に更新されることを止める運用規約が
ない」**という点にある——ガードは変化に気づくが、その変化を許可するかどうかの判断基準を持たない。

RC1は消去不可能（ドメインの性質）。RC3・RC4は除去可能。RC2は反証コストの高さという部分は有効だが、
「統合の障害＝オラクル不在」という単純化は成立しない。

### 却下: 提案B（awase自身がTSFテキストサービスになる）

検討された非連続案の一つに、awase自身をTSFテキストサービスとして登録する案があった。
**ユーザーが「事実上の別プロダクトへのpivotであり本リポジトリのスコープではない」と明示的に
判断し棄却した**——この製品判断は妥当である（round1レビュー判定）。

**round1 S8で追記**: 棄却理由を当初「工数とスコープの判断」としてのみ記録していたが、これでは
将来「工数が確保できれば再検討できる」と読まれ、`.claude/rules/experiment-logging.md`が求める
歯止めにならない。決定的な技術理由を追記する: **TSFはスレッド/プロファイルごとに有効なTIP
（Text Input Processor）が1つ**であり、awaseがTIPとして登録されるとGJI/MS-IMEと**同時に有効
にできない**。awaseの価値提案（かな漢字変換はGJI/MS-IMEに任せ、親指シフト配列だけを担う）と
原理的に両立しない——これは工数の問題ではなく**設計上の不成立**である。加えてTIPはin-proc COM
DLLとして全対象プロセスにロードされるためx86/x64両ビルドが必須、UWP/AppContainerでは
ALL APPLICATION PACKAGES ACLが要る。

なお、この案の副産物として挙がった「`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`をTsfNativeで別プロセス
から読めるか」というスパイク調査（ADR-151/152のBlocker解除条件に直結）については、**round1
S9で事前の見込みが追記された**: TSFのcompartmentは`ITfThreadMgr`経由の`ITfCompartmentMgr`から
取得し、`ITfThreadMgr`は**スレッドローカルかつin-proc**——別プロセスから読む標準的な手段は
存在しない見込みが濃厚（確度は中程度）。スパイク自体は妨げないが、「Blockerはスパイク次第で
解除されうる」という楽観は持たないこと。

## 決定

以下、A・C・D・Eを採択する。**実装詳細は個別の子ADRに分割した**——本ADRで全文を重複させると、
RC3が問題視する「同じ事実を複数箇所に手書きコピーする」構造をこのADR自身が再生産することに
なるため、要約と参照のみ残す。

### 採用A → [ADR-159](159-existing-io-boundary-inventory.md)

当初「受信/送信シーケンスという新しいコンポーネント境界を設計する」(A0)としていたが、
opus-adversarial-consult round1がこの前提を実コードで反証した（送信境界は`send_input_safe`
として既に単一で存在、InputRelay判定の「重複」は`.await`をまたぐ正当な再サンプリング、受信側
5バッファの内訳誤り、A0はADR-152の再発明でそのBlockerが未検討）。「新しい境界を設計する」から
「既存の境界を棚卸しし、届いていない呼び出し元を特定する」に組み替え、詳細を
[ADR-159](159-existing-io-boundary-inventory.md)に分離した。

### 採用C → [ADR-160](160-explicit-non-scope-declaration.md)

非スコープを決定する会議体を持つ。具体案C1(GJI一本化)/C2(アプリホワイトリスト化)/C3(conv-mode
追跡全廃)には判断材料と期限を付し、実施可否そのものは判断材料が揃うまで未確定のまま、詳細を
[ADR-160](160-explicit-non-scope-declaration.md)に分離した。

### 採用D → [ADR-161](161-single-source-spec-generation.md)

散文の権威を剥奪し、機械可読な単一仕様から生成する＋純粋層にモデル検査をかける。round1で
Aへの依存が論理的に不成立と判明したため独立させ、詳細を
[ADR-161](161-single-source-spec-generation.md)に分離した。

### 採用E → [ADR-162](162-governance-reversal.md)

ガバナンスを反転する（複雑性予算制・ADRのTTL・敵対的レビューの向き先変更）。round1で
`architecture_guard.rs`は既に増減両方を検知する完全ピン方式と判明し、実装コスト見積りを
訂正した。ADR-159の記録・再生基盤が機能し始めるまで着手しない、という依存は妥当と確認された。
詳細を[ADR-162](162-governance-reversal.md)に分離した。

## 設計原則: 宣言の強制とSSOT化（2026-09-09、ADR-161実証実験からの一般化）

[ADR-161](161-single-source-spec-generation.md)の実証実験（synによる事後スキャンとdylintに
よる宣言の強制、2つのプロトタイプの比較）から、RC3・RC4の両方に効く一段深い原則が見えてきた。

**事後スキャン（as-is）の限界**: 既存コードを後から機械的に数え上げる方式（synによるテキスト/
AST走査等）は、「同じ事実の手書きコピーが食い違う」（RC3）は解消できるが、スキャン対象の
スコープ自体に見落としが起きうる——実際、synスキャンも既存の`architecture_guard.rs`の正規表現も、
両方ともクレート境界をまたいだ呼び出しを見落としていた（詳細は[ADR-161](161-single-source-spec-generation.md)
「実証実験結果」節）。加えて事後スキャンは「新しい呼び出しが無宣言で増える」（RC4）ことへの
歯止めにはならない——増えた後に気づくだけで、増えること自体を防げない。

**宣言の強制（to-be）**: 「本来1箇所に集約されるべき」種類の関係（合流点、キュー、gate等）は、
型システムまたはlint（dylint）でその宣言を**強制**し、新しい関係が無宣言で追加されることを
コンパイルエラーにする。そのうえで、ダウンストリームの表・文書・ガード期待値は、この強制された
宣言から**生成**する。事後スキャンは、宣言への移行が完了していない箇所を洗い出す監査専用の
補助的役割に格下げする。

**3層構造として整理する**: この原則は「(1)宣言→(2)強制→(3)生成」の3層に分解できる。
(1)**宣言**は「本来1箇所に集約されるべき関係」を型付きデータとして書く。(2)**強制**は
dylint（rustcのHIRを見るため、正規表現やAST走査と違いクレート境界を自然に越える——実証実験2
で確認済み）が宣言外の関係をコンパイルエラーにする。(3)**生成**はドキュメント・ガード期待値・
pre-pushフック・CLAUDE.md該当節を宣言から出力する。既存3本のdylint lintは(1)(2)は既に体現して
いるが(3)はまだ無い——[ADR-161](161-single-source-spec-generation.md)が最初に(3)を実装する
子ADRになる。

このリポジトリには既にこの発想の前例が3つある（`no_vk_as_scan`/`ime_event_guard`/
`observation_source_guard`という既存dylint lint群——特定のenum variant構築を指定箇所以外で
禁止する）。[ADR-161](161-single-source-spec-generation.md)はこの前例をactuation呼び出しの
合流点に拡張した。

**他の子ADRへの適用状況（2026-09-09時点、A・Eは決定に反映済み、Cは将来検討として記録）**:

- **採用A（[ADR-159](159-existing-io-boundary-inventory.md)）**: 反映済み。段階0の成果物を
  「散文の棚卸し」から「dylint宣言」に定義し直した。
- **採用C（[ADR-160](160-explicit-non-scope-declaration.md)）**: 判断材料・期限テーブルの
  構造化を将来検討として記録（未実装、C1〜C3の判断確定後に改めて検討）。
- **採用E（[ADR-162](162-governance-reversal.md)）**: 反映済み。複雑性予算制（E1）の監視対象を
  「`architecture_guard.rs`のexpected値」（D1のもとでは生成物にすぎない）から「宣言そのもの
  （dylint許可リスト）」に組み替え、ADR-162 round1 M4が指摘したD1との矛盾を解消した。ただし
  キュー数・
  tuning定数数・ADR数には対応する宣言機構がまだ無く、予算制の対象は当面gate数・actuation
  合流点数に限定される。

各適用は、実証実験で価値が確認できた
範囲から段階的に広げる（この原則自体、[ADR-161](161-single-source-spec-generation.md)を机上の
議論だけで決めず先に実装して確かめた、という手順から得られたものであることに留意する）。

### 育て方（ロードマップ）

2026-09-09、実証実験2件の結果を独立したOpusエージェントに渡し「敵対的レビューではなく、
このアイデアを発展させ有効活用しきった場合の明るい未来」を構想させた（批判ではなく可能性を
広げる方向のセカンドオピニオン）。そこから、悲観的なリスク列挙ではなく「次に何をすると
一番手応えがあるか」という順序のロードマップが得られたため、以下に反映する。各段階は独立して
価値を持ち、どこかで止まっても他の段階の価値は損なわれない。**各段階を実行可能なタスクへ
分解したものが[158-implementation-tasks.md](158-implementation-tasks.md)にある**
（round1レビュー反映後に作成、依存関係と検証方法つき）。

1. **[ADR-161](161-single-source-spec-generation.md)実証実験2（`actuation_call_guard_spike`）を
   本実装に格上げする**。`set_ime_open`の許可リストを、実証実験で見つかった2件
   （`set_ime_open_ordered`とルートクレートのトレイトデフォルト実装）で確定する——後者は
   ADR-087 §5 Phase 3の「実配線するか削除するか」という既存の未決事項を伴うため、この一歩
   自体が1つの死んだコードの片付けを兼ねる。
2. **生成対象の表を1つだけ選び、実装する**（候補: actuation入口一覧。3つの異なる粒度で既に
   資料間の混同を起こした実績があり、効果が測定しやすい）。生成先は
   `fix-requires-evidence.md`の該当行と`architecture_guard.rs`のガード期待値1件から始め、
   最初から広げない。
3. **pre-pushの対象ファイル正規表現を生成に置き換える**。`.githooks/pre-push`と
   `.git/hooks/pre-push`の2ファイル乖離（[ADR-156](156-unify-deferred-execution-queues.md)
   および[ADR-161](161-single-source-spec-generation.md)背景節で確認済み、現在も未解決。
   round1 S-3で参照先を訂正——ADR-161にround1のM6という指摘は存在しない）が、生成の副産物
   として自然に解消する見込み。
4. **`docs/experiments.md`の反転史を「否定の宣言」に変換する**
   （[ADR-161](161-single-source-spec-generation.md)のD3、詳細は同ADR参照——round4 SF-5で
   「後述」を訂正、D3の実体はADR-158側にはなくADR-161側にのみある）。25エントリ（ADR-161 round1
   M-5訂正、当初17は誤り）のうちキー選択に
   関わるものから着手する。
5. **tuning定数・キュー・`AppImeProfile`の宣言化**を、それぞれ独立に着手する。

各段階は[ADR-161](161-single-source-spec-generation.md)が確立した手順（机上の敵対的レビューを
長く続けるより先に小さく実装して確かめる）を踏襲すること。より長期的・思索的な方向性として、
この3層構造（宣言・強制・生成）自体を`declare-and-enforce`のような汎用クレートとして
切り出せる可能性も指摘されたが、これは本ADR時点では投機的なアイデアであり決定事項ではない
（このリポジトリが`timed-fsm`を既にcrates.io独立公開している実績はあるため、筋道として
非現実的ではない、という程度の位置づけ）。

## フェーズ順序・依存関係

| ADR | 依存 |
|---|---|
| [ADR-159](159-existing-io-boundary-inventory.md)（採用A） | 独立。段階0(棚卸し)・段階1(記録再生)・段階2(シャドー実行)は互いに並行着手可能 |
| [ADR-160](160-explicit-non-scope-declaration.md)（採用C） | ADR-159と独立。判断期限はADR-162のE3(四半期定例棚卸し)を参照 |
| [ADR-161](161-single-source-spec-generation.md)（採用D） | 完全独立。今日から着手可能、RC3を直接潰す |
| [ADR-162](162-governance-reversal.md)（採用E） | ADR-159の記録・再生基盤が実績を出すまで着手しない |

round1(Must-fix M8、レビュー依頼4)の検証により、当初想定していた6つの依存関係のうち妥当なのは
「A→E」「D↔E」の2つのみと判明した。検証根拠の詳細は
[ADR-159](159-existing-io-boundary-inventory.md)の背景節に記載。

## 検討した代替案

### 代替案B（棄却・確定）: awase自身がTSFテキストサービスになる

上記「背景」節「却下: 提案B」参照。ユーザーが明示的に棄却し、round1レビューがTIPの排他性という
決定的な技術理由を追記した。

### 代替案（現状維持）: 個別修正を今後も都度積み重ねる

RC1はドメイン固有で消せないとしても、RC2〜RC4を放置したまま個別fixを続ける案。`docs`が`fix`を
上回るペース（71 > 47、直近30日）と、既存の増殖規模から見て、このペースを許容し続けることは
本ADRの起点となった問いへの回答にならないと判断し、不採用とする。

## 今後の議論

1. **本ADR（北極星）自体のopus-adversarial-consult round2**: round1のMust-fix 8件・
   Should-fix 11件は本版で反映済み。同じレビュアーへ再確認を依頼し、収束するまでround2以降を
   継続すること。
2. **子ADR4本（[159](159-existing-io-boundary-inventory.md)/
   [160](160-explicit-non-scope-declaration.md)/[161](161-single-source-spec-generation.md)/
   [162](162-governance-reversal.md)）を、それぞれ独立にopus-adversarial-consultへかけ、
   収束するまで反復する。** 各ADRの具体的な次アクションはそれぞれの「今後の議論」節を参照。
3. **B副産物のスパイク調査**（`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`の別プロセス読み取り可否）は、
   事前の見込みが否定的（round1 S9、`ITfThreadMgr`はスレッドローカルかつin-proc）であることを
   踏まえて着手判断すること。
4. **「設計原則: 宣言の強制とSSOT化」節の他子ADR（A・C・E）への適用可否**を、各ADRが実装段階に
   進む際に個別検討する。適用する場合も、[ADR-161](161-single-source-spec-generation.md)と
   同様にまず小さな実証実験で検証してから本実装に進む（机上の敵対的レビューだけで長時間検討を
   続けない、という2026-09-09のユーザー方針を踏襲する）。

## 関連

[ADR-159](159-existing-io-boundary-inventory.md)（採用A詳細）、
[ADR-160](160-explicit-non-scope-declaration.md)（採用C詳細）、
[ADR-161](161-single-source-spec-generation.md)（採用D詳細）、
[ADR-162](162-governance-reversal.md)（採用E詳細）、
[ADR-119](119-injected-and-relay-key-consumption-invariant.md)（actuation合流点が4〜5箇所に
分散した経緯そのものの実例）、
[ADR-121](121-explicit-physical-ime-key-idempotent-reassert.md)、
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（本文が一度もcommitされていなかった
実例、ADR-159 round1 M4で判明）、
[ADR-156](156-unify-deferred-execution-queues.md)（「オラクルなき統合」の実例だが、round1 M5
により断念理由の一部はオラクルと無関係と判明——RC2の訂正根拠）、
[ADR-157](157-symmetric-target-resolution-for-drift-correction-and-force-on.md)、
`.claude/rules/fix-requires-evidence.md`（RC4のラチェット構造そのもの）、
`.claude/rules/tuning-constants.md`、`.claude/rules/experiment-logging.md`、
`docs/layer-boundaries.md`、`docs/experiments.md`、`docs/known-bugs.md`。
