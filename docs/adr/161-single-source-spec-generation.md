# ADR-161: 散文の権威を剥奪し、機械可読な単一仕様から生成する＋純粋層にモデル検査をかける

## ステータス

**起票。[ADR-158](158-complexity-reduction-north-star.md)（北極星）の採用Dを分割・詳細化した
子ADR。Aへの依存は解消済み（opus-adversarial-consult round1で論理的に不成立と判明）であり、
他の子ADRの完了を待たず独立して着手できる。2026-09-09に実証実験2件（`spike/syn-xtask-prototype`
ブランチ）を実施し、D1の機構を「synによる事後スキャン」から「dylintによる宣言の強制＋そこから
の生成」へ組み替えた——詳細は「実証実験結果」節。この結果を独立したOpusエージェントに渡し、
敵対的レビューではなく発展的構想（セカンドオピニオン）を依頼したところ、D3（否定の宣言による
`docs/experiments.md`反転史の防止）という新しい決定項目が得られ採用した。さらに機構選定を
ブレストし、5件の追加実証実験（可視性＋permitパターン`spike/permit-pattern`、deriveマクロ・
関数形式手続きマクロ・属性マクロ2種`crates/macro-spike`）すべてを実装・コンパイル・実行まで
検証した上で、どの対象にどの機構が適するかの詳細な当てはめを「機構の選定指針」節に記録した。
2026-09-09、ADR-158〜162横断のopus-adversarial-consult round1（1ラウンド限定、ユーザー指示）を
実施し、Must-fix 5件（`pending_deferred`の所在誤り・可視性パターンの判断基準の粒度誤り・
属性マクロ「未検証」の矛盾記述・D3の機構不一致と対象ファイル誤りと適用範囲の限界・
`docs/experiments.md`エントリ数の誤り）とShould-fix 10件を反映済み。この反映結果をもとに
[158-implementation-tasks.md](158-implementation-tasks.md)（実装タスクリスト）を作成し、
これも別途opus-adversarial-consult round2にかけたところ、タスクリスト側の指摘の一部が
本ADR自身の記述にも波及するMust-fix（D3の機構が実はホストで検証できない`#[cfg(windows)]`
問題、D3候補エントリの選定誤り、判断基準への「存在/不在」の追加）だったため、本ADRにも反映
した。本ADR単体としてのopus-adversarial-consultは未実施。**

## 背景

[ADR-158](158-complexity-reduction-north-star.md)のRC3（保存形式の問題）は、以下の実例で
裏付けられている。

- **actuation入口の数え方が資料間で不一致**: `.claude/rules/fix-requires-evidence.md`は
  「5」（合流点＝新しいgateを足す場所）、`crates/awase-windows/tests/architecture_guard.rs:
  1161`は「6種」（関数名の種類の呼び出し箇所数）、同ファイル1182行のコメントは「11経路」
  （force-write/observation-based correction/Engine intentを含む意味論的経路、ADR-087参照）。
  3つは別の質問への別の答えだが、「入口」という同じ語で呼ばれているため資料間の不一致に
  見える。
- **CLAUDE.md自体が実装と食い違っている**: 「`INPUT_RELAY_APPS`が唯一の例外的な共有可変状態」
  と記載するが、実際は`HOOK_IME_MODE_DIAGNOSTICS`・`PROFILE_DESCRIPTIONS`・
  `LOG_WRITER_STATE`の3系統が他に存在する。「2つのdylint lints」も実際は3つ（`no_vk_as_scan`/
  `ime_event_guard`/`observation_source_guard`）。
- **幻のADR**: [ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)/
  [ADR-152](152-keystroke-step-source-sink-pipeline.md)は本文が一度もcommitされていなかった
  にもかかわらず、他ADRから「決定3にこう書かれていた」と権威として引用され続けた。
- **pre-pushフックの2ファイル乖離**: `.githooks/pre-push`（git管理下）と`.git/hooks/pre-push`
  （実際に実行される実体）の内容が食い違ったまま放置され、[ADR-156](156-unify-deferred-execution-queues.md)
  が「表に追加した対象ファイルの一部がpre-pushの正規表現に含まれておらず自動化が部分的にしか
  効いていない」と自ら記録している。

これらはすべて、**同じ事実がADR・known-bugs.md・`.claude/rules/*.md`・
`architecture_guard.rs`・CLAUDE.md・project memoryという6系統に手書きコピーされ、参照整合性
がない**ことに起因する。概念の同一性/差異が型ではなく名前で管理されている。

### Aへの依存が不成立と判明した経緯

[ADR-158](158-complexity-reduction-north-star.md)の当初案は、本施策（採用D）を採用A（記録・
再生基盤）の後に着手するものとしていた。opus-adversarial-consult round1のMust-fix M8が、
この依存関係が論理的に成立しないと指摘した——本施策は**ビルド時のコード生成**であり、
[ADR-159](159-existing-io-boundary-inventory.md)が提供する実行時の記録・再生には依存しない。
本ADRは他の子ADRと独立に、単独で着手できる。

## 決定

### D1: 宣言をdylintで強制し、そこから表を生成する ※実証実験を経て機構を組み替え

**当初案からの転換**: 「表をコード側の属性から生成する」という当初案は、synによる**事後
スキャン**（既存の呼び出し箇所を後から数え上げる）を想定していた。これは「同じ事実の複数箇所
への手書きコピー」（RC3）は解消できるが、「新しい呼び出し元が無宣言で増える」（RC4）ことは
防げない——数え間違いは直せても、数える対象そのものが野放図に増えることへの歯止めにはならない。
2026-09-09の実証実験（後述「実証実験結果」節）で、この事後スキャン方式が抱える限界（スコープの
見落とし）が実際に確認されたため、以下のように組み替える。

1. **強制（宣言）**: actuation合流点のような「本来1箇所に集約されるべき」呼び出しを、
   dylint（このリポジトリに既に3本の前例——`no_vk_as_scan`/`ime_event_guard`/
   `observation_source_guard`——がある）の4本目のlintとして、指定された呼び出し元関数以外
   からの呼び出しをコンパイルエラーにする。新しい呼び出し元を追加する場合は、lintの許可
   リストを明示的に更新しない限りビルドが通らない——これによりRC4（無宣言の増加）を型
   システム/lintレベルで防ぐ。
2. **生成**: dylintの許可リスト（構造化されたRustのデータ、`RESTRICTED_CALLS`のような定数）
   を単一の宣言的なSSOTとし、そこから生成する。

   **round4 TJ1 M1で訂正: 宣言レコードの最小フィールド集合を確定**。当初「関数名→許可
   呼び出し元関数名」という単純な対応だけを宣言だと想定していたが、以下4つの生成対象の
   うち3つは関数名だけでは生成できないと判明した（`.githooks/pre-push`はファイルパス起点、
   `fix-requires-evidence.md`表は散文の注記を含む、CLAUDE.md該当節はそもそも別のSSOTで
   このリストとは無関係）。宣言レコードは最低限次のフィールドを持つ:
   `callee`（被呼び出し関数名）・`callers`（許可呼び出し元関数名のリスト）・
   `file_path`（calleeの定義ファイルパス）・`adr_ref`（関連ADR番号）・`note`
   （任意、散文の注記——生成時にそのまま転記する自由記述欄）。

   これを踏まえ生成対象を以下に確定する:
   - `.githooks/pre-push`・`.git/hooks/pre-push`の対象ファイル正規表現 —
     `file_path`フィールドから生成する。
   - `architecture_guard.rs`のガード期待値（件数チェックの`expected`値） —
     生成方式はM2（下記）で確定した通り、既存のテキスト一致方式のガードを段階的に
     dylintへ置き換える方針とし、置き換え未了の間はガード側を維持し生成対象としない。
   - ADR該当節・`.claude/rules/fix-requires-evidence.md`の再発ファミリー表 —
     `callee`/`file_path`/`adr_ref`列は宣言から生成し、`note`欄（散文の注記）は宣言側の
     `note`フィールドをそのまま転記するハイブリッド方式とする（表全体を無から自動生成
     するのではなく、宣言に存在しない自由記述は宣言に含めた`note`が唯一の情報源になる）。
   - CLAUDE.md該当節（共有可変状態の一覧、dylint本数等） — **round4 TJ1 M1で対象から
     除外**。actuation許可リストとは無関係な別のSSOTであり、
     `158-implementation-tasks.md`のTB2も既に「本タスクのスコープ外」と明記していた
     （ADR本文側がタスクリストと矛盾していたのを本訂正で解消）。

   （synによる事後スキャンは、dylintの許可リストへ未登録の呼び出しが残っていないかを
   確認する**移行期の監査ツール**として位置づけを縮小する。恒久的な生成元はdylintの宣言側。）

**round4 TJ1 M2で追記: `architecture_guard.rs`の件数ガードとdylint許可リストの数え方の
不一致**。既存ガード（`architecture_guard.rs`）は`.set_ime_open(`のようなテキストの
ドット呼び出し一致で数え、定義行・ログ文字列・docコメントを意図的に除外して期待値を**0**に
固定している。一方dylintの許可リスト（意味論的な呼び出し検出）は同じ`set_ime_open`について
**1〜2件**を検出する（実証実験1・2）。数え方の違いにより、dylint由来の期待値でこのガードを
置き換えるとテキスト方式のガードは即座に壊れる。**方針: 対象ごとに個別のガードをdylintの
許可リストで置き換え、置き換えたガードはテキスト一致方式の期待値チェックを削除する**
（生成側がテキスト方式の除外規則を再現する方式は採らない——2つの数え方を両立させる意味が
ないため）。この置き換えはTA1〜TA3の範囲では行わず、TB2着手時に対象ごとに個別実施する。

**round4 TJ1 M4で追記: dylintの強制はビルドグラフに到達する範囲に限られる**。CIの実行形
（`ci.yml`）は`cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-msvc`で
あり、これがlintするのは`awase-windows`とその上流（path依存のルート`awase`）のみ。
`crates/awase-settings`（`awase-windows`を依存に持つ下流）・`awase-linux`・`awase-macos`は
ビルドグラフに入らず、そこに新しい呼び出し元を足しても静かに素通りする。「無宣言の増加を
lintで防ぐ」という約束はこの範囲に限定される。TA2着手時に`-p`の対象を拡張するか
（`--workspace`等）を判断すること。

これにより「5/6/11」のような不一致、pre-pushフックの2ファイル乖離、CLAUDE.mdの実装との
食い違いが構造的に発生不能になるだけでなく、**新しい合流点が無宣言で増えること自体も防げる**
——RC3とRC4の両方に同時に効く。

### D2: 純粋層へのproptest/モデル検査

`pub fn classify_*`は現状6個しかなく、`state/`20k行の大半は純粋分類ではなく証拠の保管・照合・
失効管理（`observation_store.rs` 2,234行、`platform_state.rs` 2,564行、`open_warrant.rs`
1,428行等）である。この実測は、「belief遷移は純粋関数に閉じている」という既存の主張自体が
現状ややアスピレーショナルであることを示唆する。**この決定は単に「モデル検査を足す」のでは
なく、モデル検査可能な粒度まで純粋層を実際に切り出す再設計を含む**——`awase-windows`に
proptestが1本もない（ルート`awase`と`timed-fsm`にはある）という現状の空白を埋める。

### D3: 否定の宣言（`REJECTED_*`）で`docs/experiments.md`の反転史を防ぐ（2026-09-09、
Opusによる発展的構想からの採用。**タスクTD0〜TD3として2026-09-09に実装完了**——ただし
実装時にTJ1 M3の再判断（既存テストが既に4アームすべてを固定済みと判明）により、新規
`const`配列・非gatedモジュール分割・新規テストはすべて不要と判明し、`key_sequence_policy.rs`
のdoc comment追記＋既存テストへのメッセージ追加という最小実装に簡略化した。詳細は
`158-implementation-tasks.md`のTDグループ参照）

`docs/experiments.md`は現在27エントリ（round1 M-5で訂正: 当初「17エントリ」と3箇所に書いて
いたが実測は当時25件。**TD4（2026-09-09）で完了**: 当時「エントリ17」の重複（28行目と707行目）
に加え、develop側の別セッションの作業で新たに「エントリ18」の重複も生じていたため
（issue #189修正の新規エントリが既存のエントリ18と衝突）、両方の新しい方のエントリをその場で
26・27へ採番し直して解消した。エントリ01は
「TsfNative + GJIのIME OFFに何のキーを送るか」で5日間に6回、採用と撤回が反転した記録である。
`VK_DBE_ALPHANUMERIC`は複数回IME OFFキーとして採用・撤回され、そのたびに「これは半角英数
（IME ON）であって直接入力ではない」という同じ事実が再発見されていた。
`.claude/rules/experiment-logging.md`はこの反転が「なぜ前回それを捨てたのか」がコミット本文
からしか辿れないことに起因すると記録している。

D1・D2が「あるべき状態」を宣言・強制するのに対し、D3は「試して失敗した状態」を同じ機構で
宣言する。例えば（round1 S-9で訂正: `app`フィールドがIME種別とAppKindを1文字列に混在させて
いたため`ime`/`app_kind`に分離）:

```rust
pub const REJECTED_IME_OFF_KEYS: &[Rejection] = &[
    Rejection {
        vk: "VK_DBE_ALPHANUMERIC",
        scope: Scope::ImeOffKey,
        reason: "半角英数(IME ON)であり直接入力ではない",
        evidence: &["098c663", "docs/experiments.md#01"],
        ime: "GJI",
        app_kind: "TsfNative",
    },
];
```

**機構の訂正（round1 M-4）**: 当初「`ime_controller.rs`側でこのVKをIME OFFキーとして書こうと
するとdylintでコンパイルエラーになり」としていたが、これは誤りだった。3点訂正する。

1. **対象ファイルの誤り**: IME ON/OFFキーのSSOTは`ime_controller.rs`ではなく、
   `crates/awase-windows/src/state/key_sequence_policy.rs`の
   `pub(crate) const fn ime_key_for(mechanism: KeyMechanism, op: ImeOperation) -> VkCode`
   である（`ime_controller.rs`のコメント自身が「送信キーはKeySequencePolicyがSSOT」と
   明記している）。
2. **機構の誤り**: D3が実際に防ぎたいのは「`ime_key_for`のmatch表が、宣言された
   `REJECTED_IME_OFF_KEYS`のいずれかを選んでいないこと」という**値の妥当性検証**であり、
   これは「XはYからしか呼ばれない」という*呼び出し関係*を検出するdylintの守備範囲ではない。
   `ime_key_for`は既に小さな`const fn`のmatch表として存在し、`key_sequence_policy.rs`には
   既に`#[cfg(test)] mod tests`がある——後述「判断の手順」問い4の通り、**`REJECTED_IME_OFF_KEYS`
   の全エントリと`ime_key_for`の全組み合わせを突き合わせる普通のユニットテスト1本**で足りる
   見込みが高い（`choke_points!`型のDSLで宣言自体を書きやすくすることと、その宣言を強制する
   手段は別の問題であり、宣言側は実証実験4の関数形式マクロが使えても、強制側はテストでよい）。

   **round4 TJ1 M3で追記・効能の訂正**: 上記の新規ユニットテストが実際に検出できる範囲は、
   `key_sequence_policy.rs`の既存テスト`gji_direct_keys`/`ms_ime_direct_keys`（`ime_key_for`
   の4アームすべてを`VK_IME_ON`/`VK_IME_OFF`という具体定数に固定済み）と`ime_key_sequence_
   golden.rs`のゴールデンが**既にカバーしている**。つまり誰かが`VK_DBE_ALPHANUMERIC`を
   `ime_key_for`に復活させれば、D3の新規テストを書かなくても既存のテスト・ゴールデンが
   落ちる。**D3が実際に追加する価値は「失敗の検出」ではなく「失敗した理由（なぜ前回捨てた
   か、`evidence`フィールドの`git`ハッシュ・`docs/experiments.md`参照）をテスト失敗時に
   直接読める形で残すこと」に限られる**。次節「なぜ前回捨てたのか」の記述もこれに合わせて
   訂正する。
3. **ホスト実行性の訂正（round2 M-1で発覚、round3 MF-2でさらに訂正）**: 当初「ホストターゲット
   で動作し、nightlyもproc-macroも不要」としていたが、これは不正確だった。
   `state/key_sequence_policy.rs`は`state/mod.rs`で`#[cfg(windows)]`が掛かっている
   モジュールツリーの内側にあり、`cargo test -p awase-windows --lib`（ホストターゲット）
   ではこのモジュール自体が存在しないものとして扱われる——CLAUDE.mdが`runtime/`配下の
   `#[cfg(test)]`について警告しているのと同じ罠に、本ADR自身が一度落ちていた。
   **round3 MF-2で判明**: この制約は回避できる。`ime_key_for`・`ImeOperation`・
   `KeyMechanism`自体の依存は`focus::class_names::AppImeProfile`・`crate::vk`・
   `awase::types::VkCode`（いずれも非gated）のみで、`#[cfg(windows)]`を要求している唯一の
   依存（`tsf::observer::ActiveImeKind`）は`gji_direct_applicable`/`ms_ime_direct_applicable`
   という**別の述語関数側**が使っているだけであり、`ime_key_for`自体は使っていない。
   **したがって`ime_key_for`・`ImeOperation`・`KeyMechanism`を非gatedなモジュールへ切り出せば
   （またはファイルを分割すれば）、D3の突き合わせテストはホストで完全に実行できる**——
   TD0でこの切り出しを行うことを推奨する。切り出さない場合のみ、`cargo check --target
   x86_64-pc-windows-msvc -p awase-windows --tests --lib`でのコンパイル確認とwindows-build
   CIへの実行委譲がフォールバックとなる。「新機構（dylint/マクロ）を増やさずに済む」という
   本判断の主眼（判断の手順・問い4）自体は、どちらの場合も変わらない。
4. **適用範囲の限界**: `VK_DBE_ALPHANUMERIC`は`ime_key_for`のmatch表選択以外の経路でも系に
   入る——GJIの実キーマップを実行時に読み取った結果（`gji_charset_autodetect.rs`）、
   config文字列からのパース（`vk.rs`の文字列→`VkCode`match、round2 S-7で訂正: `FromStr`
   実装ではない）、`HalfWidthAlnumAction`経由の注入（`output/mod.rs`）。
   `docs/experiments.md`エントリ07・08・09（scan付き/scan=0の直接注入による反転）はこれらの
   **注入経路側**の失敗であり、D3が対象とする「ソース上の定数選択」の話ではない。
   **D3が防げるのは反転史の一部（ソースコード中の定数選択）のみであり、実行時に決まる経路の
   失敗は防げない**——この限界は「トレードオフ・限界」節に追記する。

「なぜ前回捨てたのか」を`git log --grep`で掘り当てる必要がなくなる、という狙いはD1・D2と
共通する。**round4 TJ1 M3で訂正**: 「次のセッションが同じ提案を思いついた瞬間（テスト実行時）
に気づける」という表現は誤解を招く——気づけるのは既存テスト・ゴールデンによってであり、D3が
追加するのはその失敗が起きたときに読める「なぜ前回捨てたか」の理由である。

対象は`docs/experiments.md`の25エントリのうち、キー選択に関わるもの（**round4 TJ1 S4で訂正**:
「実装時に選定」ではなく、下記の絞り込みにより既にエントリ01の1件に確定済み）から着手
する（[ADR-158](158-complexity-reduction-north-star.md)「育て方」ロードマップ第4段階）。
**候補エントリの訂正（round2 M-2、round3 MF-1でさらに訂正）**: 当初「エントリ01・05・07・08・
09・16程度」を候補として挙げていたが、07・08・09は上記4「適用範囲の限界」が示す通り注入経路
側の失敗でD3の対象外、エントリ16は撤回ではなく2026-08-22にユーザー判断で本採用された変更で
あり`REJECTED_*`に入れると事実誤認になる。round2ではここから「実質的な候補はエントリ01・05の
2件程度」としていたが、**round3 MF-1でエントリ05も除外**——`docs/experiments.md`のエントリ05は
「入口でのモードキー注入」（`345086b`で追加、scan付き`VK_DBE_ALPHANUMERIC`がIME OFF文脈に
着弾してCapsLockをトグルし撤回）であり、これも注入経路側の失敗（07・08・09と同一クラス）で、
`ime_key_for`のmatch表選択とは無関係。**実質的な候補はエントリ01の1件のみ**であり、D3は
「対象がほぼ1件しかないが、代わりに（MF-2の切り出しを行えば）ホストで完全に検証できる、
極めて小さいタスク」という規模になる。既存の`.claude/rules/experiment-logging.md`が求める
「アプリ・IME・再現手順」の3点は、`Rejection`型のフィールドとして引き継ぐ。

### 機構の選定指針（2026-09-09、ブレストと5件の追加実証実験から確定、round1で訂正済み）

「強制」「生成」の実現手段はdylintだけではない。D1はdylintを最初の実証対象として選んだが
（実証実験1・2で検証済みのため）、[ADR-158](158-complexity-reduction-north-star.md)
「育て方」ロードマップの後続段階（tuning定数・キュー・`AppImeProfile`等）では、対象の性質に
応じて別の手段の方が安く済む場合がある。ブレストで洗い出した候補のうち5つ（可視性＋permit
パターン、deriveマクロ、関数形式の手続きマクロ、属性マクロ2種）は`spike/permit-pattern`・
`spike/syn-xtask-prototype`（`crates/macro-spike`・`crates/macro-spike-demo`）ブランチで
追加検証済み。

| 目的 | 手段 | 検証結果 | コスト |
|---|---|---|---|
| 単一の許可呼び出し元に限定（1箇所、同一クレート内） | 可視性（`pub(in ...)`）＋newtypeの「permit」パターン | **`set_ime_open`で検証、否定的**（後述） | 最安（成立する場合）。専用nightlyツールチェイン不要 |
| 複数の離れた許可呼び出し元（N箇所。クレート境界をまたぐ、または同一クレート内でも共通祖先モジュールがクレートルート相当まで登る場合——round4 TJ1 S1で訂正: 当初「クレート境界をまたぐ」を条件としていたが、実際の当てはめ表の`apply_ime_open_with_view`（4箇所）・`send_input_safe`（20箇所）はいずれも単一クレート内であり、判断の手順・問い1と同じ表現に揃えた） | dylint | **実証実験2で検証、成立**（後述「実証実験結果」節、round4 SF-4で「前述」から訂正） | 中〜高。nightly固定・`rustc-dev`コンポーネント・専用ビルドが要る |
| 「まず記録・可視化したい」段階（強制はまだしない） | 属性マクロ＋`#[track_caller]`で実行時に呼び出し元を記録する | **検証、成立**（実証実験5、後述） | 低。関数定義に属性を1つ付けるだけ、既存の`tracing::instrument`と同じ仕組み |
| 宣言からMarkdown/ADR断片の1行分を生成する | deriveマクロ | **検証、成立**（後述） | 低〜中。ただし全件を集める処理は呼び出し側に残る |
| 宣言を読みやすいDSLで書きたい | 関数形式の手続きマクロ | **検証、成立**（後述） | 中。`syn::parse::Parse`のカスタム実装が要るが実装量は小さかった |

**追加実証実験3（可視性＋permitパターン、`spike/permit-pattern`ブランチ）— 否定的な結果**:
`set_ime_open`（[ADR-159](159-existing-io-boundary-inventory.md)の実証実験で許可呼び出し元が
2つ——`awase-windows`側の`set_ime_open_ordered`と、ルート`awase`クレート側のトレイトデフォルト
実装——と判明していた）に、`ActuationPermit`という許可トークン型を導入し、その構築を
`pub(crate)`に絞って実際に`cargo check --target x86_64-pc-windows-msvc`を通したところ、
`error[E0624]: associated function issue_for_ordered_actuation is private`で失敗した。
**原因**: `ActuationPermit`はトレイト`PlatformRuntime`と同じルート`awase`クレートに置く必要が
あるが（トレイトのシグネチャに登場するため）、許可したい呼び出し元`set_ime_open_ordered`は
別クレート`awase-windows`にある。構築を`pub(crate)`にするとルートクレート外（＝
`awase-windows`）から届かず、`pub`にすると`awase`に依存する任意のコードから構築できてしまい
「`set_ime_open_ordered`だけ」という制約を表現できない。**結論**: 可視性＋permitパターンは
「許可呼び出し元がすべて同一クレート内に収まる」場合にのみ安く効き、トレイトを介してクレート
境界をまたぐ呼び出し元制限には使えない——この条件を満たさない対象（`set_ime_open`を含む多くの
actuation合流点）にはdylintの出番が確定する。

**追加実証実験4（deriveマクロ・関数形式手続きマクロ、`crates/macro-spike`）— いずれも成立**:
`#[derive(MarkdownRow)]`は、構造体の名前付きフィールドを走査して`to_markdown_row(&self) ->
String`を生成するマクロとして実装・コンパイル・実行に成功した。ただし**deriveマクロは型定義
（フィールド一覧）までしか見えず、`ACTUATION_CHOKE_POINTS`のようなconst配列の中身（実際に
何件あるか）は見えない**——生成できるのは「1件をどう描画するか」というテンプレートまでで、
全件を集めて表全体を組み立てる処理は呼び出し側のコードが別途要る、という制約が実装を通じて
確認できた。関数形式の手続きマクロ`choke_points! { "callee" => [callers...], adr: "..."; ... }`
は、`syn::parse::Parse`のカスタム実装（1つの構造体に対して30行程度）でDSLをパースし、
実際のactuation合流点データ（`apply_ime_open_with_view`等、これまでの実証実験の実測値）を
使って`CHOKE_POINTS`という配列に展開することに成功した。

**追加実証実験5（属性マクロ・実行時記録版、`crates/macro-spike`）— 成立**: `set_ime_open`の
実際の許可呼び出し元（`set_ime_open_ordered`）を模した関数に
`#[actuation_choke_point(callers = "set_ime_open_ordered")]`を付けたところ、`#[track_caller]`
の自動付与を含めて実装・コンパイルに成功した。実際に「許可された」想定の呼び出しと
「許可されていない」想定の呼び出し（`rogue_caller`関数経由）の両方から関数を呼び出したところ、
`cargo run`の出力に両方の呼び出し元の`file:line`が記録された:

```
[actuation-record] set_ime_open_demo called from src/main.rs:60 (許可呼び出し元: set_ime_open_ordered)
[actuation-record] set_ime_open_demo called from src/main.rs:74 (許可呼び出し元: set_ime_open_ordered)
```

`#[track_caller]`が取得できるのは`file:line`までで、呼び出し元の**関数名**までは直接取得
できない（Rustの標準機能の制約）。「許可された呼び出し元かどうか」の判定・警告出力までは
今回実装しなかったが、記録された`file:line`を人間または別のsynスキャンが読んで許可リストの
候補を洗い出す、という運用は成立する。dylintへ許可リストを移行する前の「まず実際の呼び出し
パターンを観測する」段階に使える。

**追加実証実験6（属性マクロ・メタデータ強制版、`crates/macro-spike`）— 成立**: tuning定数を
模した`const`宣言に`#[measured(value_ms = 181, margin_ms = 169, commit = "9a7e699")]`を
付け、必須フィールド（`value_ms`・`commit`）が両方揃った正常系がコンパイルに成功することを
確認した。続けて、`value_ms`を欠いた異常系（`#[measured(commit = "abc123")]`のみ）を実際に
コンパイルし、狙い通りのメッセージでコンパイルエラーになることを確認した:

```
error: #[measured(...)] には value_ms が必須です
       (.claude/rules/tuning-constants.md: 実測msを書かずに定数を変更してはならない)
```

これは`tuning-constants.md`が現在レビュー時の人力チェックに依存させている「実測msを書け」
という規約を、マクロ展開時（`cargo check`時点）の強制に置き換えられることを示す——dylintも
外部ツールも不要で、stable Rustの`proc-macro`機構だけで完結する。異常系のコードは確認後に
取り除き、ビルド可能な状態に戻した。

**「可視性＋permitパターン」には、このリポジトリ自身に既に前例がある**——
[ADR-156](156-unify-deferred-execution-queues.md)「保留」節が、`pending_deferred`の解放条件を
型で強制する将来案として`ReleaseToken<G>`（対応する解放条件を評価した関数からしか構築できない
トークン型）を挙げている。今回の実証実験3の否定的な結果は、この`ReleaseToken<G>`案についても
「対象が単一クレート内に収まるか」を先に確認すべきという教訓を残す——`pending_deferred`の
defer側/drain側窓口は両方とも`awase-windows`クレート内（`input_defer.rs`/`output/vk_send.rs`）
にあるため、この案自体は成立する見込みが高いが、確認は別途要る。

### どの場面でどのマクロ機構を使うか（詳細版、2026-09-09）

抽象的な使い分けだけでは次のセッションが再び同じ試行錯誤をすることになるため、
「何を問えばよいか」と「awaseの実際の対象がどれに当たるか」を具体的に書く。

#### 判断の手順（5つの問い、[ADR-158](158-complexity-reduction-north-star.md) round1 M-2・M-4を反映して訂正）

1. **対象（許可したい呼び出し元）は、単一クレートに収まるか、クレートをまたぐか。加えて、
   クレート内でも許可呼び出し元の共通祖先モジュールが、クレートルート相当まで登らずに済むか。**
   （round1 M-2で訂正: 当初「単一クレート内なら可視性＋permitパターンが最安」としていたが、
   可視性が刻めるのは**モジュール単位**であってクレート単位ではない。許可呼び出し元が同一
   クレート内でも複数の離れたモジュールに散っている場合、permitのコンストラクタは共通祖先
   である`pub(crate)`にせざるを得ず、その瞬間に「クレート内のどこからでも構築できる」＝
   制約ゼロになる。）クレートをまたぐ、または共通祖先がクレートルート相当になる場合は
   dylint一択。許可呼び出し元が狭いモジュール部分木に収まる場合のみ、可視性＋permitパターンが
   最安の選択肢——ただし**この「成立する」ケース自体は本ADR時点でまだ実証していない**
   （実証実験3で確定したのは「クレートをまたぐと不成立」という否定的な結果のみ）。
2. **対象は「個別の呼び出し関係（点）」か、「構造化されたデータの集合（表）」か。** 前者
   （XはYからしか呼ばれない、という1対Nの関係）はdylintまたは可視性パターンの領分。後者
   （tuning定数群、rejectionの一覧のような、フィールドを持つレコードの集合）はderiveマクロ・
   関数形式マクロの領分——dylintは「表」の生成には向かない（dylintは呼び出し関係を検出する
   ものであり、データを保持・整形するものではない）。
3. **今すぐ強制（コンパイルエラー化）したいか、まず記録・可視化したいか。** 強制したいなら
   dylint（実証済み）または可視性パターン（**同一クレート内・狭いモジュール部分木という
   条件下でのみ**、ただし成立ケース自体は未実証）。まだ許可リストの全体像に自信が持てない
   段階（新しく`AppImeProfile`の能力表を書き起こす場合など）では、属性マクロによる実行時記録
   （実証実験5で検証済み）で観測してから、確信が持てた時点でdylintに昇格する方が安全——これは
   実証実験2で見つかった「想定より呼び出し元が多かった」という驚きを、コンパイルエラーで
   いきなり食らうのではなく事前に把握できる、という効能を狙ったもの。
4. **対象が既に小さな`const fn`/match表として存在し、宣言データとの突き合わせが静的な
   ユニットテストで足りるか。**（round1 M-4で追加、これが最も安い選択肢であり見落としていた）
   dylint・可視性パターン・各種マクロはいずれも「新しい機構を1つ増やす」コストを伴う。対象が
   既にコンパクトな純粋関数として存在するなら、**普通の`#[cfg(test)]`ユニットテスト**が
   `REJECTED_*`のような宣言データと突き合わせるだけで十分な場合がある。nightlyもproc-macroも
   不要（**round2 M-1で訂正**: ただし対象が`#[cfg(windows)]`配下にある場合、ホストでは実行
   できずwindows-build CIでの検証に限られる——CLAUDE.mdが警告する既知の制約であり、D3固有の
   弱点ではない）。新機構を検討する前に、まずこの問いを立てること。

   **round4 TJ1 S6で追記・この問いの下位チェックとして最初に立てるべきもの**: 「その事実
   （match表の各アームの値）を既に固定している既存のテスト・ゴールデン・ガードが無いか」を
   先に確認すること。D3自身がこの下位チェックを欠いたまま「新しいユニットテストが要る」と
   判断し（上記M3参照）、実際には`key_sequence_policy.rs`の既存テストと
   `ime_key_sequence_golden.rs`が同じ事実を既に固定していた、という見落としを犯した。
   これは5つの問いの中で最も安い分岐（新機構どころか新規テストの追加すら不要）であり、
   tuning定数の`#[measured(...)]`や`AppImeProfile`能力表など今後の対象でも同じ見落としが
   起こりうるため、機構を選ぶ前に必ず最初に確認する。
5. **検出したいのは「存在」（許可されていない呼び出しが起きた）か、「不在」（本来あるべき
   呼び出しが欠けている）か。**（round2 M-7で追加）dylintの許可リスト方式（実証実験2）が
   検出できるのは**存在**のみ——「関数Xが、許可リストにない場所から呼ばれた」という事象は
   検出できるが、「関数Yが、本来呼ぶべき関数Zを呼び忘れた」という**不在**は、`check_fn`が
   Yの本体を走査してもZへの呼び出しが1件もない、というだけで何のイベントも発生しないため
   原理的に検出できない。[ADR-156](156-unify-deferred-execution-queues.md)が記録する
   ADR-123→ADR-128回帰（defer側/drain側の片方だけへの配線忘れ）は不在の典型例であり、
   これを検出したい場合は許可リスト方式のdylintを使えない。代案は2つ: (a) defer側・drain側
   両方が呼ぶべき条件判定を1つの共有関数に集約し（通常のリファクタ）、「その共有関数が
   defer側・drain側の両方から実際に呼ばれているか」という**存在**の確認に問いを反転する
   （この形なら`check_fn`をdefer側・drain側の2つの既知の関数に限定し、それぞれの本体を
   走査して対象呼び出しの有無を見る、というdylintの別実装で対応できる見込み——未実証）、
   (b) 型システムでの強制（[ADR-156](156-unify-deferred-execution-queues.md)の
   `ReleaseToken<G>`案）——ただしこれも対象が複数モジュールに散っている場合は
   round1 M-2の限界がそのまま当てはまる。

#### awaseの実際の対象ごとの当てはめ

| 対象 | 性質 | 適した機構 | 根拠 |
|---|---|---|---|
| `apply_ime_open_with_view`（[ADR-159](159-existing-io-boundary-inventory.md)段階0） | 個別の呼び出し関係、4箇所がクレート内の複数モジュールに分散 | dylint | 単一クレート内だが複数モジュールにまたがり、同一モジュール可視性では表現できない |
| `apply_ime_open_with_belief`（同上） | 個別の呼び出し関係、2箇所 | dylint | 同上 |
| `set_ime_open`（同上） | 個別の呼び出し関係、2箇所がルート`awase`と`awase-windows`にまたがる | dylint | 実証実験3で可視性＋permitパターンが不成立と確定済み |
| `send_input_safe`/`send_ime_control`（同上） | 個別の呼び出し関係、20＋箇所が多数のファイルに分散（すべて`awase-windows`内） | dylint | クレート境界はまたがないが箇所数が多く、可視性境界を1つに絞れない |
| `pending_deferred`の窓口（[ADR-156](156-unify-deferred-execution-queues.md)） | **不在**の検出（defer側/drain側の片方だけへの配線忘れ）。所有は`output/tsf_warmup_coord.rs`（round1 M-1で訂正、`input_defer.rs`ではない）。round2 S-12で再実測: 出現は8ファイル（`output/tsf_warmup_coord.rs`44件・`output/mod.rs`33件・`output/vk_send.rs`22件・`journal.rs`13件・`platform.rs`9件・`journal_policy.rs`/`output/probe_io.rs`/`tsf/warmup/probe_fsm.rs`各1件）と、round1の訂正時点よりさらに分散が広い | 判断基準5の代案(a)（共有ゲート関数への集約＋dylintで「呼ばれているか」を確認）または(b)（`ReleaseToken<G>`型強制）、いずれも未実証 | 許可リスト方式のdylint（実証実験2）は不在を検出できない（round2 M-7）。可視性＋permitパターンも、実測の分散度からすると成立見込みが当初より下がっている |
| D3の`Rejection`（`docs/experiments.md`の反転史） | 構造化データの集合（vk名・理由・証拠・IME種別・AppKind） | **宣言の記述**は関数形式の手続きマクロ（実証実験4で検証済み）で構わないが、**強制**は普通のユニットテスト（round1 M-4で訂正、判断基準4） | D3が防ぎたいのは`state/key_sequence_policy.rs::ime_key_for`という既存の小さな`const fn`のmatch表が`REJECTED_*`と矛盾しないことであり、これは`#[cfg(test)]`で両者を突き合わせれば足りる。dylintでは「値」の妥当性は検証できない。詳細はD3節本文の訂正を参照 |
| tuning定数の実測根拠（`.claude/rules/tuning-constants.md`） | 個々の定数に付随するメタデータ（実測ms・コミット・条件） | 属性マクロ（メタデータ強制、**検証、成立**、実証実験6） | 呼び出し関係ではなく「この定数にはこのメタデータが必須」という制約——`#[measured(value_ms=181, ...)]`が無いとコンパイルエラーにする、という使い方 |
| `AppImeProfile`の能力表（issue #136型の再発防止） | 構造化データの集合＋個別の呼び出し関係の両方 | 記録用の属性マクロ（実証実験5で検証済み）で観測してからdylintへ昇格 | 新規に書き起こす表であり、当初から確信を持てないため段階的に強制を強める |
| ADR該当節・CLAUDE.md該当節のMarkdown化 | 生成（「表」を人間可読な文書に変換） | deriveマクロ（実証実験4で検証済み）＋通常のRustコードでの集約 | deriveは1件の描画までしか生成できないため、全件の収集・結合は呼び出し側が担う |

#### stable/nightly・ツール要件の一覧

dylintのみnightlyツールチェイン固定を要求する（round1 S-10で訂正: メインワークスペース全体が
nightlyに固定されるのではなく、**各lintクレート個別**に`rust-toolchain`ファイルで`channel`を
指定する方式——本体`awase`/`awase-windows`のビルドはstableのまま）。加えて
`rustc-dev`/`llvm-tools-preview`コンポーネント、`cargo-dylint`/`dylint-link`のインストールを
要求する。可視性＋permitパターン・属性マクロ・deriveマクロ・関数形式手続きマクロは、
**いずれもstable Rustの`proc-macro`機構の範囲内で動作し、追加のツールチェインを要求しない**
（今回の実証実験4も、通常の`cargo run`だけで動作を確認できた）。「新しい仕組みを1つ増やす」
という導入コストは、dylintの方が実証実験1・2の時点で既に払い済みである一方、他の4手段は
まだ導入コストがゼロの状態にある——[ADR-158](158-complexity-reduction-north-star.md)「育て方」
第5段階（tuning定数・キュー・`AppImeProfile`の宣言化）に着手する際は、この導入コストの差も
判断材料に含めること。

## トレードオフ・限界

dylint許可リストの保守コストが発生する（新しい正当な呼び出し元を追加するたびに許可リストの
更新が要る——ただしこれは「気づかず増える」ことへの防波堤そのものであり、コストというより
意図した摩擦）。宣言に落ちない例外的分岐は実際には多く、「表に入らない例外」の置き場所を巡って
新しい複雑さが生まれる可能性がある（実証実験でも、ルートクレートのトレイトデフォルト実装という
想定外の呼び出し元が見つかった——後述）。モデル検査は「モデルが実機と一致していること」までは
保証しない（RC1が残る限りモデルの妥当性は結局実機頼み）。ただし**同じ乖離が2度起きないこと**は
保証でき、`docs/experiments.md`の反転史はまさにその種の再発である。

D3固有の限界: `Rejection`宣言は「過去に確認された失敗」を記録するものであり、環境（Windows
バージョン、GJI/MS-IMEのバージョン等）が変われば同じ手法が実は成立するようになる可能性を
排除しない——D3は「安易な再提案を防ぐ」ためのものであり、「未来永劫正しい」ことの証明では
ない。再検討したい場合は、宣言自体を明示的に削除・更新する運用（誰がそれを判断するかは
[ADR-162](162-governance-reversal.md)のE3四半期棚卸しの対象候補）が要る。**さらに
round1 M-4で判明した適用範囲の限界**: D3（ユニットテストによる強制）が防げるのは
`ime_key_for`のようなソースコード上の定数選択の失敗のみであり、GJIキーマップの実行時読み取り・
config文字列パース・注入経路といった**実行時に決まる系**での失敗（`docs/experiments.md`
エントリ07・08・09）は防げない。反転史のうち定数選択に起因する部分（エントリ01等）と、
実行時経路に起因する部分は別問題として扱うこと。

D1固有の限界（round1 S-5・S-8で訂正）: 「dylintはクレート境界をまたいだ呼び出しを自然に
越える」という当初の説明は不正確だった。正確には、dylintの走査範囲は手書きのパス指定ではなく
**ビルドグラフ**（`cargo dylint`が対象とするワークスペースメンバとその依存関係）が決める
——実証実験2でルート`awase`クレートが見えたのは、`crates/awase-windows/Cargo.toml`が
`awase = { path = "../.." }`という**ワークスペースメンバ依存**を持つためであり、crates.ioの
外部依存であれば同様には見えない。また「rustcの型チェック済みHIRを見る」という説明も、
今回のスパイク実装（`lints/actuation_call_guard_spike`）自体では未実証——実装は
`segment.ident.name`という**名前一致**のみで型解決をしておらず、無関係な型の同名メソッドにも
発火しうる。型解決（`DefId`ベース化）は本実装時の追加作業として残る。同様に「事後スキャン
（syn）はスキャン対象のスコープでしか見えない」という表現も、synそのものの限界ではなく
「走査範囲を人が宣言するか、ビルドグラフが自動的に決めるか」という運用上の違いにすぎない
（`crates/xtask-spike`自体は走査rootを引数で受け取れる設計であり、複数クレートを渡せば
見える）。また、実証実験2のlint（`RESTRICTED_ACTUATION_CALL`）は`Warn`レベルで宣言されて
おり、D1が想定する「コンパイルエラーにする」という強制は`DYLINT_RUSTFLAGS="-D warnings"`
という運用（CLAUDE.mdが既に前提としている）があって初めて成立する——CI環境でこの環境変数の
指定が抜けると、強制が静かに無効化される点に注意すること。

## 実証実験結果（2026-09-09）

`spike/syn-xtask-prototype`ブランチで、機構選定のための2つの実証実験を行った（この方針自体、
「敵対的ADRで無駄な思考を長時間続けた過去の失敗」を繰り返さないため、机上検討より先に実装を
試すという判断による）。

### 実験1: synベースの事後スキャンxtask試作

`crates/xtask-spike`として、`syn`で`crates/awase-windows/src`（142ファイル）を構文解析し、
指定した関数への呼び出し箇所を数え上げるツールを作成した。

- 142ファイル全てのパースに成功（失敗0件）——実コードベース（属性マクロ・unsafeブロック・
  ジェネリクス込み）に対して`syn`が問題なく通用することを確認。
- 既知の件数と完全一致: `apply_ime_open_with_view`=4、`apply_ime_open_with_belief`=2、
  `apply_ime_open_with_applied`=0、`send_input_safe`=20。
- **既存ガードの見落としを発見**: `set_ime_open`が1件ヒット（`platform.rs:1722`、
  `PlatformRuntime::set_ime_open(self, open)`という`::`修飾呼び出し）。既存の
  `architecture_guard.rs`は`".set_ime_open("`という**ドット呼び出し限定**の文字列一致で
  「呼び出し元0件」としているが、実際には`set_ime_open_ordered`（正規のwarrantラッパー）が
  `::`構文でこの関数を呼んでいる。正規表現ベースの走査が構文の書き方の違いを構造的に見落とす
  ことを実証した。

### 実験2: dylint 4本目のlint試作（許可リスト方式の呼び出し強制）

既存の3本（`no_vk_as_scan`/`ime_event_guard`/`observation_source_guard`）と同じ枠組みで、
「指定した関数への呼び出しを、許可リストにある呼び出し元関数以外から行うとコンパイルエラーに
する」lint（`actuation_call_guard_spike`、`RESTRICTED_ACTUATION_CALL`）を試作し、実際に
`cargo dylint --lib actuation_call_guard_spike -p awase-windows`で`awase-windows`＋依存する
ルート`awase`クレートに対して実行した。

- lintは正常にビルド・実行でき、既存の3本と同じ運用（`cargo dylint`経由）に乗ることを確認。
- **実験1よりさらに深い見落としを発見**: `set_ime_open`への呼び出しがもう1件検出された——
  ルート`awase`クレート側の`src/platform.rs`にある`PlatformRuntime`トレイトの**デフォルト
  実装**（`apply_ime_open`）が`self.set_ime_open(open)`を呼んでいる。この呼び出しは
  実験1のsynスキャン（`crates/awase-windows/src`のみを走査）にも、既存の
  `architecture_guard.rs`（同様に`awase-windows`クレート限定）にも**両方とも見えていなかった**
  ——理由はどちらもクレート境界をまたいで見ていなかったため。dylintはrustcの型チェック済み
  HIRを見るため、依存クレートを含む実際のコンパイル単位全体を自然にカバーできる。

  なお、このデフォルト実装自体はコード中のコメントで「どこからも呼ばれない死んだコード
  （ADR-087 §5 Phase 3で実配線するか削除するか判断すること）」と既に記録されている——
  「`apply_ime_open`という関数自体が誰にも呼ばれていない」という主張と、「その関数の本体
  という**ソース上の記述**として`set_ime_open`の呼び出しが実在する」というのは別の主張であり、
  今回の発見は後者。

### 実証実験からの結論

「事後スキャン（syn）」と「宣言の強制（dylint）」のどちらも技術的に実現可能だが、
到達範囲に明確な差がある。事後スキャンは自分がスコープに指定したディレクトリしか見えない
のに対し、dylintは型チェック済みのコンパイル単位全体（クレート境界をまたぐ依存関係込み）を
見る。**D1の恒久的な機構をdylint（宣言の強制）に定め、synによる事後スキャンは移行期の監査
専用に格下げする**、という「決定」節の組み替えはこの実証結果に基づく。

## 検討した代替案

### 代替案（不採用）: synによる事後スキャンのみで完結させる

実験1のみを採用し、宣言の強制（dylint）を導入しない案。RC3（同じ事実の手書きコピー）は
解消できるが、実験2が示した通り事後スキャンには到達範囲の限界があり、かつ「新しい呼び出しが
無宣言で増える」（RC4）ことへの歯止めにならない。宣言を強制する仕組みを追加するコストに
見合う価値（RC3・RC4の両方に効く）があるため不採用とする。

### 代替案（現状維持・棄却）: 散文での手動同期を継続する

ADR・known-bugs.md・`.claude/rules/*.md`・CLAUDE.md間の整合性を、レビュー時の人力チェックだけで
維持する案。既にCLAUDE.mdの2箇所の誤記、pre-pushフックの2ファイル乖離、幻のADR-151/152という
実例が示す通り、この方式は構造的に破綻するため不採用とする。

## 今後の議論

1. dylint 4本目のlintを本実装に格上げする際の許可リストを、実証実験で見つかった2件
   （`set_ime_open_ordered`と、ルートクレートのトレイトデフォルト実装）を含めて確定する。
   後者については、コメントが示唆する通りADR-087 §5 Phase 3の判断（実配線するか削除するか）
   を先に行うべきかも合わせて検討する。
2. 生成対象とする最初の表を1つ選ぶ（候補: actuation入口一覧、これは3つの異なる粒度で既に
   資料間の混同を起こしている実例であり、最も効果が測定しやすい）。
3. `pub fn classify_*` 6個に対して既存のproptestパターン（ルート`awase`クレートに実例あり）を
   適用するスパイクを行う。
4. D2（純粋層の再切り出し）の対象範囲を、`observation_store.rs`等の証拠管理コードから見極める。
5. 「宣言の強制＋そこからの生成」という考え方を、D1(単一仕様生成)だけでなく[ADR-158](158-complexity-reduction-north-star.md)
   の他の子ADR（A・C・E）にも横断的な設計原則として適用できないか検討する——詳細は
   ADR-158「設計原則: 宣言の強制とSSOT化」節参照。
6. D3の対象エントリ（round3 MF-1で訂正: `docs/experiments.md`エントリ01の**1件のみ**、
   05・07・08・09・16はいずれも対象外と判明済み）について、`Rejection`型のフィールド構成を
   確定する。
7. [ADR-158](158-complexity-reduction-north-star.md)「育て方」ロードマップの第1〜3段階
   （`actuation_call_guard_spike`本実装化→表1つの生成→pre-push regex生成）に着手する。
8. ~~`docs/experiments.md`のエントリ17重複採番バグ（round1 M-5で発見）を修正する。~~
   **完了（TD4、2026-09-09）**: エントリ17・エントリ18それぞれの重複（後者はdevelop側の
   別セッションの作業で新たに発生）を、新しい方のエントリの番号をその場で26・27へ
   採番し直して解消した。
9. `pending_deferred`窓口（round1 M-1で所在を訂正、round2 S-12で利用側をさらに再実測）の
   宣言・強制を確定する。**round2 M-7で訂正**: 当てはめ表が示す通り、許可リスト方式のdylint
   では不在（配線忘れ）を検出できないため、判断の手順・問い5の代案（(a)共有ゲート関数への
   集約＋別方向のdylint、または(b)`ReleaseToken<G>`型強制）のどちらを採るか、実証実験で
   確認してから確定する。所有は`output/tsf_warmup_coord.rs`、利用側は`output/vk_send.rs`・
   `output/mod.rs`・`platform.rs`・`journal.rs`・`journal_policy.rs`・`output/probe_io.rs`・
   `tsf/warmup/probe_fsm.rs`の計8ファイル（round2 S-12実測）。
10. D3は`state/key_sequence_policy.rs::ime_key_for`を対象に、`REJECTED_IME_OFF_KEYS`との
    突き合わせを行う普通のユニットテストとして実装する（round1 M-4で機構を訂正、dylintでは
    ない）。**round3 MF-2で追記**: `ime_key_for`・`ImeOperation`・`KeyMechanism`を非gated
    モジュールへ切り出せば、このテストはホストで完全に実行できる見込み——実装時にまず
    この切り出しを検討すること。適用範囲がソース上の定数選択に限られ、GJIキーマップの
    実行時読み取り等の経路は
    別問題として残ることを実装時に明記する。

## 関連

[ADR-158](158-complexity-reduction-north-star.md)（北極星、本ADRの親。「設計原則: 宣言の
強制とSSOT化」節に本ADRの実証実験から一般化した原則を記載）、
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（幻のADRの実例）、
[ADR-156](156-unify-deferred-execution-queues.md)（pre-pushフックのregex乖離の先行事例）、
`.claude/rules/fix-requires-evidence.md`、`.claude/rules/ime-belief-architecture.md`
（既存のdylint運用の先例）、`CLAUDE.md`、`spike/syn-xtask-prototype`ブランチ
（`crates/xtask-spike`、`lints/actuation_call_guard_spike`、`crates/macro-spike`、
`crates/macro-spike-demo`。いずれも実証実験用で本ADRのマージ対象には含めない）、
`spike/permit-pattern`ブランチ（可視性＋permitパターンの否定的な実証実験。意図的に
コンパイルが通らない状態のまま記録、マージ対象に含めない）。
