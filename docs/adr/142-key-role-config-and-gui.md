# ADR-142: 物理キー役割代入のconfig形式・設定GUI（Phase B）

## ステータス

**r5（Opus 2体の敵対的レビューを4ラウンド実施しBlockerゼロで収束。
両エージェントとも「r3→r4で新規混入ゼロ、マージしてよい」と判定。
r5は軽微なNit反映のみで、レビュアーへの再確認は両エージェントの
判断により不要）。**

**追補（2026-09-06、後続ADR-143のレビュー中に発覚・決定B7を訂正）**:
決定B7が「なぜ`!is_injected`の早期returnを無くしても実害が出ないか」で
述べた安全性論拠が、ADR-141決定4の不動点制約という別ルールに依存していた
ことが判明した。hold-stateへのstoreのゲートを`!is_injected`単独ではなく
`event_eligible`そのものにすることで、決定4に依存せず構造的に安全にした。
詳細は決定B7本文の「訂正」箇所とADR-141決定1「r-later訂正」を参照。

ADR-141（物理キー役割代入の基盤、Phase A、r3で収束済み）が確立した挿入点・
hold-state設計・全単射制約・Altセンチネル不動点制約を前提に、実際のTOML
config形式・`awase-settings`設定GUI・バリデーションの実装配置を決める。
ADR-141の未解決の疑問1〜3・決定4/6が明示的にPhase Bへ委ねた事項に、本ADR
で回答する。

対象VKはADR-141決定0と同じ「安全な3キー」（`VK_CONVERT`/`VK_NONCONVERT`/
`VK_SPACE`）。

**注記**: ADR-141決定6-3（`transport.rs::plan`への影響）が誤帰属で
あったことが本ADRのr1レビューで判明したため、**ADR-141本体にも訂正
注記を追加済み**（`docs/adr/141-physical-key-role-substitution.md`決定6
の3項目内）。ただし**r2レビューでその訂正先自体にも誤りが見つかった**
（`keymap.rs:29`は役割代入の影響を受けない、正しい訂正先は`win32.rs:169`
のみ、決定B7参照）。

**r0→r1で決定B0/B1/B3/B4/B5/B6/B7を修正した**: 最大の問題は決定B1
（バリデーションをcore/platformに分割する）で、`from_name`（キー名の別名
解決）がplatform専属である以上、core側は独自の不完全な別名表を持たざるを
得ず、これは`awase-settings`が過去に踏んで修正した二重管理バグ（issue #99）
と同型の欠陥だった。両エージェント独立にこの結論へ到達したため、
バリデーションは全てplatform側に一本化した（決定B1参照）。

**r1→r2で決定B1/B2/B3/B7を修正した**: r1の改訂作業自体が新たに4件の
問題を混入させていた——(1)決定B3の「既存の`warn_on_engine_hotkey_collision`
をGUIから直接呼び出す」が、同関数が`pub(crate)`かつ戻り値`()`である
ため実際には呼び出せない（可視性変更・戻り値変更を伴うAPI変更が必要、
両エージェント独立に到達）、(2)決定B1がAlt適用前vk退避の訂正過程で
起動時（`app/bootstrap.rs`）のバリデーション実行を誤って書き落として
いた、(3)決定B7が「Linuxで実行可能」とした`win32.rs:169`のテストは
実際には`#[cfg(windows)]`ゲート内でLinux実行不可能、かつもう一方の
訂正先`keymap.rs:29`は役割代入の影響を受けない静的な判定であり
テストしても何も守らない（実際の露出点は`win32.rs:169`のみ）、
(4)決定B2の「GUIは構造的に無効な組み合わせを生成できない」という
主張が、同じ決定B2内で新設したクロスタブ機構（無効化されても選択を
保存まで保持する）と矛盾していた。

## 背景

### 参考にする既存の前例と、あえて踏襲しない前例

- **`[[keymaps]]`（ADR-114、`src/config.rs:589-604`の`KeymapRule`、実際の
  TOMLフィールド名は複数形`keymaps`——r1レビューで単数形`[[keymap]]`という
  誤記が判明したため訂正）**: 個別ルールを`from`/`to`のペアで表現し、
  不正なルールは`KeymapTable::new`（platformクレート）が個別に
  `log::warn!`してskipする方式。config.tomlには一切触れない（GUI側にも
  「既存config.tomlにAlt付きルールが手書きされていた場合、その値は変更
  されず素通しされる」と明記されている、`main.rs:2172-2176`）。本ADRが
  設計する`[[key_role]]`は、TOMLの表記スタイル（`from`/`to`ペアのリスト）
  は踏襲するが、**バリデーション方式は部分的にしか踏襲しない**——
  ADR-141決定1-2が「全単射制約の違反は個別ルールのskipでは修復できない、
  ルール集合全体を無効化する」と明確に決めている一方、「config.tomlには
  触れない」という`[[keymaps]]`の性質は踏襲する（決定B6参照、r1レビュー
  B-4対応）。
- **ADR-110`key_remap`（撤回済み）**: config名`[[key_remap]]`は
  再利用しない（決定B0参照）。
- **Scancode Mapセクション（ADR-111/126、`awase-settings/src/main.rs:2371-2467`）**:
  「少数プリセットから選ぶ」発想は踏襲するが、即時反映・自己昇格は
  踏襲しない（決定B2参照）。

### `fix-requires-evidence.md`の再発ファミリーへの該当性（r2で訂正）

`.claude/rules/fix-requires-evidence.md`が「物理IMEキーのSuppress/Allow
配送判断」ファミリーとして`runtime/transport.rs::PhysicalKeyDisposition::plan`
を名指ししている。**r0はADR-141決定6-3を根拠にこの合流点への影響を主張
していたが、r1レビューで誤帰属と判明した**——実際に代入後vkの影響を
受けるのは`crates/awase-windows/src/win32.rs:169`（`send_input_safe`の
`conv_mutation`ゲート）のみであり、`transport.rs`は無関係だった（詳細は
決定B7参照）。

**r2レビューで、この訂正後の論拠づけ自体にも問題があると判明した**
（r2レビューarchitect役r2-N1）: `win32.rs`/`hook.rs`/`state/key_role.rs`
のいずれも`fix-requires-evidence.md`の再発ファミリー表に載っていない
ため、「表に名指しされているから該当する」という当初の論法（r0が
`transport.rs::plan`について使っていたもの）はもう使えない。

**決定**: `fix-requires-evidence.md`の表への該当性を主張するのではなく、
決定7（ADR-141）のKeyUp注入がBUG-100の再発ファミリーそのものである
という事実を直接の根拠として、同ルールが要求する水準（回帰テストまたは
`known-bugs.md`記録）を自主的に満たす。表に載っていないことは、この
水準を満たさなくてよい理由にはならない——`win32.rs`の`conv_mutation`
ゲート（`ADR-084/086`のconv actuation系、`fix-requires-evidence.md`の
「conv mode」「force-write/actuationターゲット」再発ファミリーには実質
該当する）についても同様に扱う。具体的な検証方針は決定B7参照。

## 決定

### 決定B0: config形式は`[[key_role]]`（新規名、`[[keymaps]]`のfrom/to様式を踏襲、ただしキー名別名表は共有しない）

```toml
[[key_role]]
from = "VK_CONVERT"
to = "VK_SPACE"

[[key_role]]
from = "VK_SPACE"
to = "VK_CONVERT"
```

- 新規のTOMLテーブル名`[[key_role]]`を使う。`[[key_remap]]`（ADR-110、
  撤回済み）・`[[keymaps]]`（ホットキー再割当て、意味論が異なる）とは
  意図的に別名にする（r0の理由をそのまま維持）。
- フィールド: `from: String`・`to: String`（単一VK名）。`[[keymaps]].to`が
  `Vec<String>`（複数ステップの打鍵列）なのに対し`String`のままとする
  理由は、単に「複数ステップという概念が無いから」ではなく、**多段の
  `to`は置換ではなくなり、ADR-141決定1-2の全単射制約・決定2の
  `decide_role_substitution`の戻り値型・決定8が依拠する`KeymapLatch`の
  Down/Up vk一貫性を同時に破壊するため意味論的に誤りになる**（r1レビュー
  N-2、architect役指摘）。
- **キー名解決は`VkCode::from_name`（`crates/awase-windows/src/vk.rs:388-`）
  を使うが、r0が主張した「日本語表記とVK_接頭辞表記の両方を受け付ける」は
  スペースキーについては事実ではないと判明した**（r1レビューB-1、両
  エージェント独立に到達）: `from_name`の照合表は`\"VK_SPACE\"`の1形態
  のみで、`\"スペース\"`という別名は存在しない（`vk.rs:438`）。一方
  `変換`/`無変換`には複数の別名がある（`vk.rs:444-445`）。
  **決定**: v1では`from_name`に新たな別名を追加しない。config例は
  `\"VK_CONVERT\"`/`\"VK_NONCONVERT\"`/`\"VK_SPACE\"`のVK名表記に統一する
  （日本語表記の`\"変換\"`/`\"無変換\"`は`from_name`が既に受け付けるため
  引き続き使えるが、`\"スペース\"`という日本語表記の追加は本ADRのスコープ
  外とする）。`from_name`は`app/bootstrap.rs`・`keymap.rs`・
  `gji_charset_autodetect.rs`・`awase-settings`の複数の表示/解決関数が
  共有するSSOTであり、ここへの別名追加は`key_role`専用の都合で行わない
  （他機能への波及を伴う変更は別ADRで扱う）。
- **`src/config.rs`の`THUMB_KEY_ALIASES`（`:997-998`）は流用しない**（r1
  レビューB-2、両エージェント独立に到達）: この定数は
  `validate_thumb_key_in_ime_combos`と`validate_keyboard_model`の**両方**
  が参照する単一情報源であり、後者はUS配列ユーザー向けの「JIS専用キーの
  残存検出」に使われる（`:1090-1094`の`mentions_jis_only`）。ここに
  `(\"スペース\", \"VK_SPACE\")`を追加すると、その警告文自体が推奨する
  「スペースキーを親指キーにする設定」（`:1132`）を警告してしまう自己
  矛盾を生む。`[[key_role]]`のキー名正規化は`THUMB_KEY_ALIASES`を経由せず、
  `VkCode::from_name`の解決結果（`VkCode`の値）だけで比較する。
- 明示されないキーは恒等（自分自身へ写る）として補完する（ADR-141決定1-2）
  ため、`[[key_role]]`のエントリ数は0〜3個の範囲で、必ずしも全3キー分を
  書く必要はない。同じ`from`が複数回現れる設定（例: `変換→スペース`と
  `変換→無変換`が両方存在する）は、写像として不整合（1つの入力に2つの
  出力）であるため明示的にエラーとし、first-wins/last-winsのような
  暗黙の優先順位を設けない（r1レビューpremortem役W-5）。

### 決定B1: バリデーションは全てplatformクレート側に一本化する（core/platform分割は不採用）

ADR-141未解決の疑問1（core/platformどちらに置くか）への回答。

**r0は「core=全単射の構造チェック、platform=Altセンチネル不動点チェック」
という分割を提案していたが、r1レビューで両エージェントが独立に、この
分割自体が成立しないと判定した（r1レビューB-3、architect役・premortem役
一致）。理由**: coreクレート（`src/config.rs`）は`VkCode::from_name`
（platform専属）を呼べない。したがってcore側で「全単射か」を判定しようと
すると、core独自の文字列比較・独自の別名表を持たざるを得ない。しかし
`from_name`が受け付ける別名（`無変換`に対する`VK_NONCONVERT`/
`VK_MUHENKAN`/`Nonconvert`/`無変換`の4表記等）をcore側の別名表が完全に
網羅する保証はなく、`from=\"無変換\" to=\"スペース\"`と
`from=\"VK_MUHENKAN\" to=\"変換\"`のような、`from_name`上は同一vkを指す
がcoreの文字列比較では別物に見えるルールが混在すると、core側は「4要素の
全単射」として通過させてしまうが、実際にはplatform側で解決すると
「1つのvkに2つの`to`が付いた、写像ですらないテーブル」になる。

これは`awase-settings::is_muhenkan_thumb_key`（`main.rs:3937-3950`）の
docコメントが明示的に警告する、issue #99と同型の二重管理バグである
（同docは「独自の別名リストを持たず、実際のキー入力解決に使われる
`VkCodeExt::from_name`に解決させて比較する。文字列比較の一覧を二重管理
すると、vk.rs側に別名が追加されたときに表示条件だけ追従し忘れて同じ
不具合が再発する」と明記している）。

**決定**: 全単射の構造チェック・Altセンチネル不動点チェックの両方を
`crates/awase-windows`（platformクレート）側に置く。新設モジュール
（例: `crates/awase-windows/src/state/key_role.rs`）に、`VkCode::from_name`
で解決済みの`VkCode`値上でのみ比較を行う純粋関数として実装する。
`keymap.rs::KeymapTable`が「Windows APIに依存しない純粋な値比較なので
ungated、Linuxで`cargo test -p awase-windows --lib`から全数テストできる」
という前例（`crates/awase-windows/src/lib.rs:58-64`）と同じ立場であり、
ADR-019（coreのOS非依存維持）は制約にならない——coreに置かなくても
テスト可能性は達成できる。

**両関数は起動時（`app/bootstrap.rs`）とreload時（`app/mod.rs:615`の
`reload_config()`、既存のconfig reload経路の実体、r0が挙げていた
`runtime/focus_tracking.rs`は誤り——r1レビューarchitect役N-7）の
**両方**から呼ぶ**（r1は「reload時のみ」と誤って書いていた——
r2レビューarchitect役r1-M1: 起動時バリデーションを落とすと、不正な
`[[key_role]]`を書いたconfigで起動した場合に無検証で有効化されてしまう。
決定B3が同じ2箇所を正しく挙げているので、決定B1もそれに揃える）。

新設モジュール内での実装上の注意点（r2レビューarchitect役r1-N1）:
Altセンチネル解決には`resolve_thumb_key`が必要だが、これは**必ず
`crate::state::alt_impersonation::resolve_thumb_key`を直接importする**
こと。既存の呼び出し元（`app/bootstrap.rs:202,207`）は`hook.rs:70`の
再エクスポート（`hook::resolve_thumb_key`）経由だが、`hook`モジュール
自体が`#[cfg(windows)]`ゲートされているため、この再エクスポート経路を
真似ると`state/key_role.rs`をungated登録してもコンパイルが通らず、
`#[cfg(windows)]`で回避した瞬間にLinuxテストが黙って消える
（決定B7が警告するのと同種の罠の一段深い版）。

**重複`from`エントリの検出もこのモジュールに含める**（r2レビュー
premortem役N-5）: `VK_MUHENKAN`と`無変換`を同一vkとして扱う判定
（`from_name`解決後の比較）が必要なため、config構文レベル（core）では
検出できず、決定B1のplatform側モジュールでのみ検出できる。

どちらのチェックも失敗した場合、`[[key_role]]`のルール集合**全体**を
**実行時に**無効化する（個別ルールのskipではない、ADR-141決定1-2/決定4）。
ただし「無効化」の実装は`[[keymaps]]`の前例（config.tomlには一切触れず、
`KeymapTable::new`が実行時にskipするだけ）を踏襲し、`AppConfig::validate()`
が返す`ValidatedConfig`から`[[key_role]]`のエントリを削除する実装には
**しない**（決定B6でこの理由を詳述、r1レビューB-4対応）。

### 決定B2: GUIは「6通りの置換」を列挙するプリセット選択式にする（自由編集リストにしない）

3要素の集合上の全単射（置換）は、恒等写像を含めてちょうど6通りしか
存在しない。この事実を利用し、自由な追加/削除リストUIにはせず、6通りの
置換をあらかじめ全て列挙したプリセットから選ぶUIにする:

1. 無効（恒等写像、`[[key_role]]`エントリなし）
2. 変換⇔無変換 入れ替え
3. 変換⇔スペース 入れ替え
4. 無変換⇔スペース 入れ替え
5. 変換→無変換→スペース→変換（3巡）
6. 変換→スペース→無変換→変換（3巡、5の逆）

この設計により、GUIは全単射違反（写像として不整合な組み合わせ）を
構造的に生成できない。**ただし、GUIがAltセンチネル不動点制約違反まで
生成不能というわけではない**（r2レビューpremortem役N-4、r0/r1の記述
「TOML手書きユーザーに対してのみ実際にエラーを返しうる」を訂正）:
後述のクロスタブ無効化フローにより、有効なプリセットを選択した後で
親指キー設定をAltセンチネルへ変更すると、GUI操作だけで不動点制約
違反の状態を作れる。この場合の保存時の扱いは末尾の「クロスタブでの
無効化」で決定する。

**Altセンチネル不動点制約によるグレーアウト判定の実装方法を、r1で
具体的に修正した**（r1レビューB-5、premortem役指摘）: r0の記述
「片側のみAltセンチネルの場合、不動点になるのは1キーだけ」を素朴に
「左スロットがセンチネルならVK_NONCONVERTが不動点」と実装すると誤る。
`ALT_IMPERSONATION_OPTIONS`（`main.rs:3968-3969`）は左右どちらの
ドロップダウンにも`"Left Alt"`/`"Right Alt"`の両方を出せるため、
`left_thumb_key="Right Alt"`のように「左スロットに右用センチネル名」が
入る構成が可能であり、この場合`resolve_thumb_key`
（`alt_impersonation.rs:38-45`）が実際に固定するのは`VK_CONVERT`である
（`VK_NONCONVERT`ではない）。**決定**: グレーアウト判定は必ず
`resolve_thumb_key()`の戻り値vkから計算する（どちらのスロットにどの
センチネル名が入っているかではなく）。両スロットが同じセンチネル名を
指す構成（例: 両方`"Left Alt"`）では不動点は1つだけであり、「両方
センチネルなら常に恒等写像のみ」という判定も誤りである——ADR-141決定4-1
の「左右ともAltセンチネル」という記述は「異なる2つのAltセンチネル
（Left AltとRight Alt）が使われる場合」に限定して読むこと。

グレーアウトの判定ロジック（禁止理由を含む）は決定B1のplatform側
モジュールにSSOTを置き、GUIはそれを呼び出すだけにする
（`keymap_forbidden_reason`→`forbidden_target_vk_reason`という既存の
「理由はplatform、GUIは表示のみ」という構造、`main.rs:4427`、を踏襲する
——r1レビューarchitect役N-3）。

**Altセンチネル使用時、5/6のプリセットが同時に無効になりうる（両側で
異なるAltセンチネルを使う「US配列の標準構成」の場合）ため、理由の提示は
ツールチップだけでは不十分である**（r1レビューpremortem役W-10）。
**決定**: 各無効化されたプリセットについて、選択肢の直下（ツールチップ
ではなく常時可視のテキスト）に「`right_thumb_key = \"Right Alt\"`が
変換キーの役割を固定しているため選択できません」の形式で理由を表示する。

**3巡プリセット（5・6）はラベルだけでは方向が読み取りにくい**（r1レビュー
premortem役W-7）ため、選択中のプリセットについて「現在の左親指キー:
物理◯◯キー / 右親指キー: 物理◯◯キー」というプレビュー表示を追加する。
**このプレビューは選択した置換σそのものではなく、その逆写像σ⁻¹で
計算すること**（r2レビューarchitect役r1-N2）: ADR-141決定4が「親指キーの
役割はその全単射の**逆写像**で決まる物理キーへ再配置される」と定義して
おり（「今、配置後にVK_NONCONVERTを生成するのはどの物理キーか」を
知るには逆写像が要る）、σを順方向に適用すると3巡プリセットでは逆の
答えが出る。

**タブ配置**: 「キー設定」タブ（`left_thumb_key`/`right_thumb_key`と
同居）に置く（r1レビューpremortem役W-11）。理由: グレーアウト条件が
同タブ内の親指キー設定に依存するため、クロスタブでの説明の分かりにくさ
（下記参照）を最小化できる。「ショートカット」（`[[keymaps]]`）タブには
決定B4の情報提示のみを表示する。

**クロスタブでの無効化（依頼シナリオ1への回答、r1レビューpremortem役
B-6）**: ユーザーが「無変換↔スペース」を選択した状態で、後から
`left_thumb_key`を`"Left Alt"`に変更し、Altセンチネル不動点制約に
違反する状態になった場合の遷移を明示する。既存の`main_key_combo`
（`main.rs:4593-4620`）は「現在の値が候補に無くても、警告もクリアも
せずそのまま表示する」という無言の前例を持つが、本機能ではこれを
踏襲しない。**決定**: 親指キー設定の変更が既存の`[[key_role]]`選択を
Altセンチネル不動点制約違反にする場合、その変更を行った**その場**で
（保存を待たずに）、「キー設定」タブ上に「この変更により役割代入設定
（現在: 無変換↔スペース）が無効になります」という通知を表示する。
選択状態自体は保存されるまで変更しない（ユーザーが親指キー設定を
元に戻せば復旧できるようにするため）。

**r1→r2で保存時の扱いを追加した**（r2レビューpremortem役N-4対応）:
上記の通知が出ている状態のまま「適用」を押すと、GUI操作だけで
Altセンチネル不動点制約違反の`[[key_role]]`がconfig.tomlに保存される
経路になる。**決定**: この状態で「適用」を押した場合、確認ダイアログ
（「役割代入設定は現在の親指キー設定と両立しないため保存後は適用されません。
(a) このまま保存する (b) 役割代入設定を『無効』にしてから保存する
(c) 親指キー設定の変更を取り消す」の3択）を挟む。これにより「GUIは
Altセンチネル不動点制約違反を生成できない」という主張は撤回し、正しくは
「GUIは全単射違反を生成できないが、Altセンチネル不動点制約違反は
クロスタブ操作を通じて生成しうる（ただし保存時に確認ダイアログを経由
する）」とする。

この確認ダイアログの実装は、`awase-settings`の既存の確認モーダル前例
（`egui::Window::new(\"確認\")`、`main.rs:1018,1055,1094`）に倣うが、
`apply()`（`main.rs:788-796`）冒頭の相互排他ガードに本ダイアログの
表示条件を追加すること（r2レビューarchitect役r2-N2）: 同ガードの
既存コメントが「確認モーダルを実質無視できる」という過去の/code-review
指摘を記録しており、`egui::Modal`のようなブロッキング機構が無い
（`main.rs:808,6629`）ため、手動での排他制御が必須である。

**7つ目の状態（全単射違反・解決不能なキー名・スコープ外VK等、6プリセット
のいずれにも一致しない「認識できない設定」、決定B6参照）と、この
「有効なプリセットだが現在の親指キー設定により一時的に無効化されている」
状態は、GUI上で明確に区別する**（r2レビューpremortem役N-6）: 前者は
「認識できない設定のため、明示的に選び直すまで触りません」、後者は
「選択中ですが、現在の親指キー設定では適用されません」という別々の
表示にする。両方を同一の「カスタム」状態に潰すと、ユーザーが選んだ
プリセットが「認識できない設定」として復旧不能に見えてしまう。

UIコンポーネントは`awase-settings`の標準パターン（ADR-127の単一Apply
原則）に従い、`self.config`に直接バインドする単一のラジオボタン/
ドロップダウンとする。画面全体の「適用」ボタンで確定し、Scancode Map
のような即時反映・自己昇格は行わない。

### 決定B3: 既定ホットキー・sync keyとの衝突警告は既存の`warn_on_engine_hotkey_collision`を拡張する

**r0は「`src/config.rs`（core）に新規関数を追加する」としていたが、
r1レビューで(a)この検査は既に`crates/awase-windows/src/keymap.rs:270-326`
の`warn_on_engine_hotkey_collision`として実在すること、(b)この関数は
`\"Ctrl+Shift+変換\"`のような修飾キー付きコンボを`vk::parse_key_combo`
（platform専属）で解析する必要があり、core側には移せないこと、の両方が
判明した（r1レビューB-4/B-9、architect役・premortem役独立に到達）。**

**決定**: 新規関数を作らず、既存の`warn_on_engine_hotkey_collision`を
拡張し、`[[key_role]]`ルール（decision B1で解決済みのvk値を渡す）も
衝突検査の対象に含める。この関数は既に`app/bootstrap.rs`と
`app/mod.rs:652`（reload経路）の両方から呼ばれているため、決定B1が要求
する「親指キー設定・`[[key_role]]`設定のどちらが変更されたreloadでも
再実行する」という要件は追加配線なしで満たされる。

**表示経路の欠落を修正する（r1→r2でAPI変更点を具体化した——r2レビュー
architect役r1-B1/premortem役N-2、両エージェント独立に到達）**: r1は
「この関数を直接呼び出して結果を取得する」としていたが、現状の
`warn_on_engine_hotkey_collision`のシグネチャではこれは不可能だと判明
した——(1) `pub(crate) fn`（`keymap.rs:270`）であり別クレートの
`awase-settings`から呼べない、(2) 戻り値が`()`で警告は`log::warn!`に
直接吐かれる（`keymap.rs:307,317`）ため「結果を取得」できない、(3)
引数の`&[ParsedKeyCombo]`を組み立てるヘルパ`parse_key_combos`
（`app/mod.rs:223`）はプライベート関数かつ`&mut StartupDiagnostics`を
取る設計でGUIから使いにくい。**決定**: 以下のAPI変更を行う——
(a) `warn_on_engine_hotkey_collision`を`pub`にする、(b) 戻り値を
`Vec<String>`にする、(c) 既存の2呼び出し元（`app/bootstrap.rs`、
`app/mod.rs:652`）は受け取った`Vec`を`diag.warn`へ流す（これにより
既存の`[[keymaps]]`衝突警告も、本ADR以前から存在した「ログにしか出ず
ユーザーに届かない」欠落が副次的に解消される——意図的な副次改善として
扱う）、(d) GUI側は`vk::parse_key_combo`（`pub fn`、`vk.rs:538`）を
直接使ってコンボ列を組み立てる（`parse_key_combos`を経由しない）。
上記(a)〜(d)を経て、GUI側の保存直後ステータス表示・起動時診断パネルから
この関数を直接呼び出し、既存の`Vec<String>`警告表示経路（`main.rs:564`,
`:914`）へ合流させる。

**sync keyの所在を訂正する**: r0は「per-appの`sync_toggle_keys`等」と
書いていたが、r1レビューで**これらはper-appではなくグローバル設定**
であることが判明した（r1レビューB-7/W-1、architect役・premortem役
独立に到達）。実体は`config.rs`の`keys.ime_detect.{toggle,on,off}`
（`ImeDetectConfig`、`src/config.rs:466-473`）で、`app/mod.rs:648`/
`bootstrap.rs:986`の`init_ime_sync_keys`から供給される。既定値
（`on:[\"IMEオン\"]`/`off:[\"IMEオフ\"]`/`toggle:[]`）は安全な3キーを
含まないため**既定**configでの衝突面はゼロである——ただし
`ImeDetectConfig`は`#[serde(default)]`付きでユーザーが`on = [\"変換\"]`
のように自由に書き換えられるため、「既定でゼロ」であって「衝突が
起こらない」わけではない。警告実装自体は必要である（r2レビュー
premortem役N-7）。**さらに`keys.ime_detect`
は`awase-settings`の設定GUIから編集できないフィールドである**
（`main.rs:552-558`の「保存直前にディスクから読み直す非GUIフィールド」
リストに含まれる、r1レビューpremortem役W-2）。したがってこのフィールド
に関する警告文は「設定画面で直してください」ではなく「config.tomlを
直接編集してください」と案内する。

`vk_may_mutate_conv`の非対称性による実害は、決定B7のテストで確認する
（対象は`win32.rs:169`のみであり、`transport.rs::plan`はもちろん
`keymap.rs:29`も無関係——`keymap.rs:29`はconfig.tomlに書かれた静的な
vkしか見ないため役割代入の影響を受けない、決定B7参照）。

### 決定B4: `[[keymaps]]`との重複は警告色で、発火する物理キーを名指しする

ADR-141未解決の疑問2への回答。**r0は「エラーにも警告にもしない設計も
あり得る」「通常色の補足テキスト」としていたが、r1レビューで実害が
r0の想定より大きいと判明した**（r1レビューB-8、premortem役指摘）。

既定config（`left_thumb_key=無変換`、`right_thumb_key=変換`）では、
`forbidden_target_vk_reason`（`keymap.rs:26-28`）が親指キーを`[[keymaps]]`
の`from`/`to`双方で既に禁止しているため、実際に重複しうるのは
`VK_SPACE`のみである。具体的な実害シナリオ: (1) ユーザーが
`[[keymaps]] from=\"Shift+VK_SPACE\" to=[\"VK_F7\"]`を設定（現状有効）、
(2) 後日「無変換⇔スペース」プリセットを有効化、(3) `[[keymaps]]`は
代入後vkで照合されるため（ADR-141決定1）、このルールは**物理スペース
バーではなく物理無変換キー**で発火するようになる。ユーザー体験は
「Shift+SpaceのF7が効かなくなり、代わりに無変換キーで暴発する」という
サイレントな機能移動になる。

**決定**: r0の「情報提示（通常色）」を「警告（警告色）」に格上げし、
文面には移動先の**物理キー名を明示する**（例:「この`[[keymaps]]`ルール
は現在、無変換↔スペースの役割代入により物理無変換キーで発火します」）。
検出述語自体（`[[key_role]]`の`from`と`[[keymaps]]`の`from`が同じvkを
指すか）はr0のままでよい。

なお、役割代入によって`[[keymaps]]`のルールが親指キーへ「飲み込まれる」
方向の悪化は起きないことを付記する: `forbidden_target_vk_reason`の
親指キー判定は代入後vk空間で一貫しており、代入後に物理スペースバーが
生成する`VK_NONCONVERT`（親指キー）は引き続き`[[keymaps]]`の対象から
禁止される（r1レビューpremortem役B-8補足）。

### 決定B5: 単独タップ設定の説明は、実際に表示されているセクション側に注記する

ADR-141決定6#6のPhase B注記への回答。**r0は「スペースキーの単独タップ
設定セクションへ注記を追加する」としていたが、r1レビューでこのセクション
自体が既定configでは画面に表示されないことが判明した**（r1レビュー
premortem役W-8）: `space_thumb_*`系セクションの表示条件は文字列の直接
比較（`main.rs:1998-1999`、`left_thumb_key == \"VK_SPACE\" ||
right_thumb_key == \"VK_SPACE\"`）であり、既定config
（`left_thumb_key=\"無変換\"`）のまま「無変換⇔スペース」プリセットを
有効にしても、この文字列は`\"無変換\"`のままなので条件を満たさず、
セクション自体が表示されない。

**決定**: 注記を追加する先は`space_thumb_*`系ではなく、実際に表示され
続ける`muhenkan_*`系セクション（`is_muhenkan_thumb_key`/
`is_henkan_thumb_key`、`main.rs:3951`/`:3957`、表示分岐`main.rs:2044-2055`）
側にする。文言例:「現在『無変換↔スペース』の役割代入が有効なため、この
設定は物理スペースバーに適用されます」。動的なUI見出し切り替え（役割名
ベースの表示）は本ADRのスコープ外とする（r0の判断を維持）。

### 決定B6: バリデーションエラーは実行時無効化に留め、config.tomlの内容は変更しない

**r0は「`validate()`が返す`Vec<String>`警告リストに追加する一項目として
扱う」としていたが、r1レビューでこれが致命的なデータ損失を招くと判明
した**（r1レビューB-4、両エージェント独立に到達）。

`awase-settings`の保存経路（`main.rs:568-577`、`config.rs:813-816`）は
`validate()`が返す`ValidatedConfig`を`self.config`へ書き戻してから
`save()`する。つまり`validate()`が「ルール集合全体を無効化する」ことを
`ValidatedConfig`から`[[key_role]]`エントリを取り除く実装で行うと、
ユーザーが手書きした（全単射違反の）`[[key_role]]`が、**無関係な設定を
1つ変更して保存しただけで**config.tomlから消える。ステータスバーには
「設定を保存しました（警告: …）」という成功を主語にした1行しか出ない
ため、この消失にユーザーは気づけない。

**決定**: 決定B1で述べた通り、「ルール集合全体の無効化」は**config.toml
の内容には触れず、実行時（起動時・reload時にメモリ上でルールテーブルを
構築する段階）でのみ**行う（`[[keymaps]]`が個別ルールに対して既に採用
している「configには触れない」性質を、集合全体への適用に拡張したもの）。
`AppConfig::validate()`（core）は`[[key_role]]`のTOML構文（`from`/`to`が
文字列であること等）のみを検証し、全単射チェック・Altセンチネル不動点
チェック（決定B1、platform側）はこれとは別の実行時ゲート
（`app/mod.rs:reload_config()`が呼ぶ）として扱う。

**エラーメッセージの表示先を修正する**: 決定B1の実行時無効化が発生した
場合、既存の「設定を保存しました（警告: …）」という成功優先の文面
（`main.rs:722-732`）ではなく、警告部分を先頭に出す文面にする（例:
「役割代入設定が無効です（一切適用されません）: 変換とスペースの両方が
無変換へ写っています」）——他の警告は「保存はできた」で正しいが、
このケースは「保存はしたが機能しない」ため優先度が異なる（r1レビュー
premortem役W-6）。

**GUIとTOML手書きの併存で発生しうる「6プリセットのいずれにも一致しない
状態」への対応**（旧未解決の疑問3、r1レビューpremortem役W-9が「理論上
発生しない」という楽観を否定）: GUIが読み込むconfig.tomlには、(a)全単射
でない手書き集合、(b)`from_name`で解決できないキー名、(c)スコープ外VK
（ADR-141決定0が対象外としたVK_KANA等）が現実に含まれうる。この場合、
GUIのプリセット選択欄は「カスタム/認識できない設定」という7つ目の状態を
明示的に表示し、**ユーザーが明示的にプリセットのいずれかを選び直すまで
このセクションの保存内容を変更しない**（決定B1・本決定の「config.tomlに
触れない」原則の帰結）。

`ValidatedConfig`/`AppConfig`間の詰め替え（`From<ValidatedConfig> for
AppConfig`）に`key_role`フィールドを追加する際は、過去に`keystroke_macro`
フィールドがこの詰め替えで失われたことがあり、2本の回帰テスト
（`config.rs:2222-`, `:2252-`）が対策として存在する。同型のround-trip
テストを決定B7に追加する（r1レビューpremortem役W-3）。

### 決定B7: テスト計画

**r0の3項目のうち2つがr1レビューで訂正・大幅拡充が必要と判明した**
（r1レビューB-6/B-7、architect役・premortem役）。

- **`golden_scenarios.rs`への追加は撤回する**（r1レビューB-6、architect役
  指摘）: このファイルは`ImeEvent`→`ImeModel`のreducerシーケンスのみを
  扱い、`RawKeyEvent`を構築しない・`transport::plan`を呼ばないため、
  役割代入（hook.rs内のvk書き換え）を検証できる抽象レベルではない。

  **r1は代替として`keymap.rs:29`と`win32.rs:169`の2箇所へのユニットテスト
  追加を「Linuxで実行可能」としたが、r2レビューで両方とも誤りと判明した**
  （r2レビューpremortem役N-1/N-3、収束点）:
  - `keymap.rs:29`（`forbidden_target_vk_reason`の`vk_may_mutate_conv`
    チェック）は、config.tomlに書かれた`[[keymaps]]`の`from`/`to`を
    解決した**静的な**vkを見る判定であり、hook.rsで書き換えられる代入後
    vkとは無関係——役割代入はこの判定の入力を一切変えない。ここに
    テストを追加しても何も保護しない。
  - `win32.rs:169`（`send_input_safe`の`conv_mutation`ゲート）は実際に
    代入後vk（`RawKeyEventExt::reinject()`が`wVk`に渡す値）を見る、
    正しい露出点である。しかし`win32.rs`自体が`#[cfg(windows)]`
    ゲート内にあり（`lib.rs:85-86`）、対象関数が`windows`クレートの
    `INPUT`構造体を引数に取るため、**Linuxのテストバイナリには存在
    せず、型自体も構築できない**。「Linuxで実行可能」は誤り。

  **決定**: `win32.rs:169`（`conv_mutation`ゲート）への影響は、
  Windows専用の`#[cfg(test)]`として実装し、`windows-build` CIジョブ
  でのみ検証する（`fix-requires-evidence.md`が許容する「実機依存で
  自動化できない場合は known-bugs.md 記録で代替する」の(b)を選ばず、
  (a)のテストを選ぶ——CI上でのWindows実行自体は可能なため）。

  **r2→r3で「2層で責務を分ける」記述の型を訂正した**（r2レビュー
  premortem役R-2、実装指示としての実質的な誤り）: r2は「
  `decide_role_substitution`が返す`confirmed_target`の値が`win32.rs`側の
  conv_mutation発火判定にそのまま渡る」としていたが、これは誤り。
  ADR-141決定2のシグネチャ`-> (VkCode, Option<VkCode>)`のうち、hookが
  `vk`へ代入し最終的に`RawKeyEventExt::reinject()`の`wVk`（ひいては
  `win32.rs`の`send_input_safe`の引数）へ流れるのは**第1要素（書き換え
  後vk）**である。`confirmed_target`は第2要素（次に保持する状態）で
  あり、hold-stateスロットに格納されるだけでSendInputには渡らない。
  **決定**: Linux側の単体テストは「`decide_role_substitution`が返す
  第1要素（書き換え後vk）が、`vk_may_mutate_conv`の真偽をどう反転
  させるか」を検証する（`vk_may_mutate_conv`自体のungated全数テストは
  `vk.rs:933-966`に既存）。Windows側CIは「その書き換え後vkを実際に
  `INPUT.wVk`として構築したときに`conv_mutation::bump()`が正しく発火/
  抑制されること」を検証する。この2層により、r2レビューpremortem役
  R-3が指摘した「テスト対象は`send_input_safe`本体（実際にSendInputで
  キーを注入し、`continue-on-error`扱いの非決定的領域に踏み込む）では
  なく、`win32.rs:160-169`の純粋判定部（`INPUT`を受けて
  `vk_may_mutate_conv(ki.wVk)`を返すだけの関数）に絞る」という要件も
  同時に満たす。`keymap.rs:29`へのテスト追加は行わない（保護対象が
  無いため——上記「fix-requires-evidence.mdの再発ファミリーへの該当性」
  節参照）。
- **`decide_role_substitution`の網羅テーブルテストを、値を区別する形に
  訂正する**（r1レビューB-7、premortem役指摘）: r0は
  「`is_keydown`×`was_down`×`confirmed_target.is_some()`×
  `rule_target.is_some()`の16通り」としていたが、これは
  `decide_alt_impersonation`の状態が`bool`（`was_impersonating`）である
  ために16通りで全数になっているのを、`Option<VkCode>`を持つ
  `decide_role_substitution`にそのまま当てはめた誤りである。
  `.is_some()`に潰すと、「`confirmed_target = Some(A)`かつ
  `rule_target = Some(B)`（A≠B）のKeyUpが正しく`A`を返す」という、
  決定3・決定8が依拠する唯一の不変条件（KeyDown時点の確定値をKeyUpまで
  保持する）が検査対象から漏れる——config reload中に写像が変わると
  Down/Upで異なるvkを返す実装バグが、この16通りテーブルを全て通過して
  しまう。**決定**: 値を区別する具体的なテストケース（`confirmed_target`
  と`rule_target`が異なる場合のKeyUp）を明示的に追加する。加えて
  `alt_impersonation.rs:200-212`が実践する「オラクル値は関数docの仕様
  から手で導出し、実装をコピーした別実装は行わない（トートロジー防止）」
  という規律を明記する。
- **ADR-141が要求した以下5種類のテストを、実際にLinuxで検証可能な
  範囲を精査した上で列挙する**（r1レビューB-7(c)。**r2→r3でこの範囲を
  訂正した**——r2レビューarchitect役r2-M2、項目1〜4を「`state/key_role.rs`
  で全てLinux実行可能」としていたr1/r2の前提は誤りだった）:

  Alt impersonationの既存実装形と照合すると、純粋関数として切り出されて
  いるのは`decide_alt_impersonation`（`state/alt_impersonation.rs`、
  ungated）だけであり、held-state用のstatic変数（`ALT_L_WAS_DOWN`等、
  `hook.rs:88,98-99,103,114`）とそれらへの配線（`apply_alt_impersonation`、
  `hook.rs:79-115`）は`#[cfg(windows)]`側にある。役割代入も同型であれば、
  以下1〜4のうち実際にLinuxで検証できるのは純粋関数`decide_role_substitution`
  自体の網羅テーブル（上記項目）のみで、1〜4個別の主張はhook.rs側の配線の
  性質になってしまう（この制約は、直後の決定によって項目1を除き解消する）。

  **決定**: ADR-141決定1-1・1-3が定めるイベント単位の判定（Alt適用前後の
  比較・`!is_injected`ガード）を、`decide_role_substitution`（決定2）の
  パラメータへ吸収する形でADR-141決定2のシグネチャを拡張する（Phase A
  実装時にこの拡張版シグネチャを採用する、という本ADRからPhase Aへの
  申し送り）: `event_eligible: bool`（`!alt_impersonated && !is_injected`、
  呼び出し側であるhook.rsが計算して渡す）を追加パラメータとする。

  **r3→r4で`event_eligible`の参照タイミングを訂正した**（r3レビューで
  architect役・premortem役が独立に到達、r3-M1/S-1）: r3は「`false`の
  場合は無条件に`(original_vk, confirmed_target)`を返す」としていたが、
  これは`decide_alt_impersonation`が`engine_enabled`を扱う方法（新規押下
  時点でのみ参照し、以後の押しっぱなし中は直前の判定を維持する——これが
  BUG-41の修正内容そのもの）と正反対であり、同型のstuck keyを再導入する。
  具体的な破れ: 物理スペースキーのKeyDownが`event_eligible=true`で
  `confirmed_target=Some(VK_NONCONVERT)`を確定しOSへ`VK_NONCONVERT`の
  downを送出した後、対応するKeyUpがリレーツール（Mouse Without Borders・
  リモートデスクトップ・AutoHotkey等）経由で`LLKHF_INJECTED`付きで届く
  （ADR-141が「Down/Upが対で来ない注入は珍しくない」と明記した状況）と、
  そのKeyUpは`event_eligible=false`のため無条件に`(VK_SPACE,
  Some(VK_NONCONVERT))`を返してしまい、OSは`VK_NONCONVERT`のdownを
  受け取ったまま`VK_SPACE`のupを受け取る——決定7がKeyUp注入で防ごうと
  したstuck keyが、この新パラメータ自体から別経路で再現する。加えて
  決定2の「`was_down`と`confirmed_target`は必ずペアで更新する」不変
  条件も、この早期returnでは保証されない。

  **決定（訂正版）**: `event_eligible`は`decide_alt_impersonation`の
  `engine_enabled`と全く同じ扱いにする——**新規押下
  （`is_keydown && !was_down`）時点でのみ**参照し、`confirmed_target`を
  確定する（`event_eligible=false`ならこの時点で`confirmed_target=None`
  を確定する）。auto-repeatのKeyDownとKeyUpは、その時点の`event_eligible`
  の値に**関わらず**、既に確定済みの`confirmed_target`をそのまま対称に
  使う（KeyUpは通常どおり`confirmed_target`と`was_down`を両方`None`/
  `false`にクリアする）。これにより、押しっぱなし中に`event_eligible`が
  途中で変化しても（Down時は物理、Up時はリレー経由の注入、等）、Down/Up
  ペアの対称性が保たれる。

  1. **6変数のスロット混線防止テスト**は、`decide_role_substitution`を
     異なる`original_vk`（変換/無変換/スペース）で独立に複数回呼び出し、
     一方の呼び出し列がもう一方の`confirmed_target`に影響しないことを
     確認するテストとして、Linuxで実行できる。**ただしこれが保証するのは
     「純粋関数自体はスロット間で独立に振る舞う」ことだけであり、
     「hook.rsが正しいスロット（代入前の物理vk）で引数を渡している」
     という配線の正しさまでは保証しない**（r3レビューpremortem役S-4）
     ——ADR-141決定2のN2（代入先でキーイングすると双方向スワップで
     2つの物理キーが同じスロットを奪い合う）が防ごうとした配線ミスは、
     テスト側のローカル変数が正しいスロットを模倣してしまう以上、
     この種のテストでは検出できず、hook.rs側のコードレビュー事項として
     残る。
  2. **`alt_impersonated`フラグによるイベント単位判定**と
     3. **`!is_injected`ガード**は、`event_eligible`パラメータの
     `true`/`false`を新規押下時点で切り替えるテストケースとして、
     `decide_role_substitution`の網羅テーブルに統合できる（独立した
     テスト関数を新設する必要がなくなる）。
  4. **「1イベントにつき高々1回だけ適用」**は、「冪等性テスト」（同じ
     イベントで2回呼んでも結果が変わらないこと）では検証できない
     ——純粋関数である以上、内部でσをn回適用する誤実装であっても
     入力が同じなら常に同じ出力を返すため、この種のテストは定義上
     必ず通過してしまう（r3レビューpremortem役S-2）。**決定**: 3-cycle
     プリセット（決定B2のプリセット5・6）に対する網羅テーブルの期待値
     として、「`from=変換`・`rule_target=σ(変換)=無変換`の新規押下が
     `無変換`（σの1ホップ）を返し、`スペース`（σ²）にも`変換`（σ³、
     元に戻る）にもならないこと」を明示的な期待値として固定する。これで
     「引けなくなるまで繰り返す」実装と「偶数回で元に戻る」実装の両方を
     検出できる。
  5. **決定7のKeyUp注入テスト**（`reset_physical_key_state`・
     `clear_hook_latches_for_app_disable`・overflowアームの3経路それぞれ
     で、stuck keyが発生しないこと、BUG-100再発ファミリー対策）は、
     上記の拡張後もなお`hook.rs`側の統合的な振る舞い（実際のSendInput・
     latch状態を伴う）であり、純粋関数への切り出しが本質的に難しい。
     `win32.rs:169`と同様、`windows-build`CIジョブで実行する
     `#[cfg(test)]`として実装する（r2レビューpremortem役N-1）。
  6. **store側のゲートテスト**（2026-09-06追加、ADR-143レビューで判明）:
     `decide_role_substitution`自体の網羅テーブル（項目3、`event_eligible`
     の32通り）は「呼び出し結果が何を返すか」のみを検証し、「呼び出し元
     （hook.rs）が結果をhold-stateへstoreするかどうか」は対象外である。
     `event_eligible=false`のイベント（injected、またはAlt impersonation
     由来）を処理した後に`{KEY}_WAS_DOWN`/`{KEY}_CONFIRMED_TARGET`が
     変化しないことを、項目5と同様`windows-build`CIジョブの
     `#[cfg(test)]`として別途追加する（vk変換自体は無条件に行われる点と
     混同しないこと——検証対象はstore側のみ）。

  **申し送り内容の追記**（r3レビューpremortem役S-3、r4レビューarchitect役
  Nit）: 上記のシグネチャ拡張はADR-141決定2だけでなく**決定1-1も
  書き換える**——決定1-1は現在「`alt_impersonated`がfalseの場合に
  **のみ**役割代入を適用する」「役割代入の適用とhold-state更新は
  `hook.rs:1137`の`!is_injected`ガード**内**で行う」という、**hook.rs側
  で呼ぶかどうかを決める2つのゲート**として書かれている。
  `event_eligible`への吸収後は、この2つのゲート（Alt判定・
  `!is_injected`判定）はどちらも`decide_role_substitution`の呼び出し
  **前**の分岐ではなくなり、hook.rsは安全な3キーのイベントを**無条件に
  `decide_role_substitution`へ渡し**、`event_eligible`パラメータの計算
  だけを担う（ガードを呼び出し側の早期returnから、渡す値の計算へ移す）。
  この書き換えを怠り`!is_injected`の早期returnをhook.rs側に残したまま
  `event_eligible`だけ追加すると、注入イベントが`decide_role_substitution`
  に到達せず`is_injected`成分が実質デッドコードになり、項目3の網羅
  テーブルテストは緑になるのに本番では通らない経路を検証していることに
  なる（カバレッジの偽陽性）。

  ただし決定1-1が警告する「`is_alt_impersonation_active()`という
  グローバルラッチを条件に使ってはならない、`hook.rs:1128`の
  `rewritten_vk != vk`をイベント単位のフラグとして取ること」という要件は、
  `event_eligible`の計算方法として引き続き必須であり、申し送りから
  落とさないこと（落とすとグローバルラッチ由来の非対称がS-1と別の形で
  復活する）。また、`decide_alt_impersonation_exhaustive_16_combinations`
  に対応する網羅テーブルは、決定2が元々持つ4つの真偽軸に`event_eligible`
  が加わるため16通りではなく32通りになる点も申し送りに含める。

  **なぜ`!is_injected`の早期returnを無くしても実害が出ないか**（r4
  レビューarchitect役、後任が同じ懸念を再検証せずに済むよう明記）:
  注入イベントで`was_down`が立ち後続の物理押下がauto-repeat扱いに
  なっても、KeyUpで`confirmed_target`と`was_down`が両方クリアされる
  ため自己修復的であり、「一時的に代入がスキップされる」（安全側）に
  留まる——ADR-141決定7の「stuck-trueは危険だがstuck-falseはそうならない」
  という非対称と同じ性質である。

  **訂正（2026-09-06、ADR-143レビューで判明、Major相当）**: 上記の
  「Alt impersonation出力が安全な3キーと同じスロットを引く経路も、
  ADR-141決定4の不動点制約により結果的に無害」という論拠は、**store側の
  ゲートを`!is_injected`単独にした場合にのみ必要になる**もので、
  決定4という別ルールへ安全性を依存させてしまっていた。正しくは、
  hold-state（`{KEY}_WAS_DOWN`/`confirmed_target`）へのstoreのゲートを
  `!is_injected`ではなく**`event_eligible`そのもの**にする（vk変換自体は
  引き続き無条件——ここは変えない）。これによりAlt impersonation由来の
  イベント（injectedではないがAlt適用済み）もstoreされなくなり、
  「物理無変換キーを押したままLeft Altを離す」という操作でAlt側のKeyUpが
  誤ってstoreされ物理無変換キー自身のhold-stateが途中でクリアされる
  （NONCONVERT_WAS_DOWNが誤ってfalseに変わってしまう）経路が構造的に閉じる。決定4の
  不動点制約は結果として引き続き成立するが、**それに依存しなくても安全**
  という形に強化された。詳細はADR-141決定1「r-later訂正」を参照。
- **`state/key_role.rs`は必ずungatedで登録する**（r1レビュー
  premortem役W-4）: `state/mod.rs`は`alt_impersonation`等を
  `#[cfg_attr(not(windows), allow(dead_code))] pub mod`として登録して
  おり（BUG-41がWindows実機で初めてテストされるまで発見されなかった
  ことの再発防止）、`#[cfg(windows)]`を付けるとLinuxでのテスト実行が
  黙って偽になる（`cargo nextest list`にも出ない）。
- **`ime_key_sequence_golden.rs`（Windows実機限定）**: 役割代入がIME戦略
  選択・送信キー列に影響を与える経路が実装中に見つかった場合のみ、
  期待値を追加する。
- **`ValidatedConfig`/`AppConfig`のround-tripテスト**（決定B6参照、
  r1レビューpremortem役W-3）: `keystroke_macro`の前例（`config.rs:2222-`,
  `:2252-`）と同型のテストを`key_role`フィールドに対して追加する。
- **決定B2のプレビュー表示（逆写像σ⁻¹）のテストは3-cycleプリセットで
  行う**（r2レビューpremortem役R-4）: 2-cycleプリセット（2〜4）は
  自己逆（σ=σ⁻¹）であるため、順方向写像と逆写像を取り違える実装バグは
  2-cycleのテストだけでは検出できない。3-cycleプリセット（5・6）で
  σとσ⁻¹が異なる結果になることを確認するテストケースを用意する。

## 却下した代替案

- **`[[key_remap]]`という名前の再利用**: 決定B0参照。
- **自由な追加/削除リストUI**: 決定B2参照。
- **Scancode Mapと同じ即時反映・自己昇格パターン**: 決定B2参照。
- **バリデーションのcore/platform分割（r0案）**: `from_name`の別名解決が
  platform専属であるため、core側は不完全な独自別名表を持たざるを得ず、
  issue #99と同型の二重管理バグを再導入する。決定B1で全てplatform側に
  一本化した。
- **バリデーションを個別ルールskip方式にする（`[[keymaps]]`のskip部分
  だけ踏襲、config.tomlに触れない性質は踏襲しない、r0案）**:
  `validate()`から`[[key_role]]`を削る実装は、無関係な設定変更の保存を
  きっかけに手書きルールを消失させる（決定B6参照）。「config.tomlには
  触れない」という`[[keymaps]]`の性質は踏襲し、無効化は実行時ゲートに
  留めた。
- **`[[keymaps]]`との重複を通常色の情報提示のみにする（r0案）**:
  既存の`[[keymaps]]`ルールが発火する物理キーがサイレントに変わる実害を
  過小評価していた。警告色＋移動先の物理キー名明示に格上げした
  （決定B4）。
- **`src/config.rs`にホットキー衝突検査を新規実装する（r0案）**:
  `Ctrl+Shift+変換`のような修飾キー付きコンボの解析（`parse_key_combo`）
  はplatform専属であり、既に存在する`warn_on_engine_hotkey_collision`
  を拡張する方が正しい（決定B3）。

## 未解決の疑問

1. 決定B5の「役割名ベースの動的UI見出し」再設計は、実際のユーザー
   フィードバックが無い時点では過剰設計になりうるため、本ADRでは静的な
   注記に留めた——次に着手すべきかは実装後のユーザー反応を見て判断する。
2. 決定B2のクロスタブ通知（親指キー変更が既存の役割代入選択を無効化する
   旨の即時表示）の具体的なUIコンポーネント（バナー、ダイアログ、
   インライン警告等）は実装時のGUIレビューで確定する。
3. **解決済み**（r3、premortem役N-8指摘を受け先送りせず対応）: 本ADRが
   前提とするADR-141はr3までで収束しているが、r4（3回目の再確認）は
   未実施である。ADR-141決定6-3の誤帰属（`transport.rs::plan`ではなく
   `win32.rs:169`のみ——`keymap.rs`は無関係）は、本ADRのr1/r2レビュー中に
   ADR-141本体（`docs/adr/141-physical-key-role-substitution.md`決定6の
   3項目内）へ訂正注記を追加済み。次にADR-141のr4を実施する際、この
   訂正注記を本文へ正式に統合すること。

## 関連ファイル

`src/config.rs`（`KeymapRule`との対比、`THUMB_KEY_ALIASES`、
`validate_thumb_key_in_ime_combos`、新設`KeyRoleRule`、
`ValidatedConfig`/`AppConfig`の詰め替え）、
`crates/awase-windows/src/vk.rs`（`VkCode::from_name`、決定B0のSSOT）、
`crates/awase-windows/src/keymap.rs`（`forbidden_target_vk_reason`、
`warn_on_engine_hotkey_collision`、決定B1/B3の拡張対象）、
`crates/awase-windows/src/win32.rs`（`send_input_safe`の`conv_mutation`
ゲート、決定B7）、`crates/awase-windows/src/state/alt_impersonation.rs`
（`resolve_thumb_key`、決定B1/B2が依存）、
`crates/awase-windows/src/app/mod.rs`（`reload_config():615`、決定B1の
再検証エントリポイント）、`crates/awase-settings/src/main.rs`
（`tab_keymap:2126-`、`THUMB_KEY_OPTIONS:3920-3936`、
`ALT_IMPERSONATION_OPTIONS:3968-3969`、`is_muhenkan_thumb_key:3951`、
`main_key_combo:4593-4620`、Scancode Mapセクション`:2371-2467`）、
`crates/awase-windows/src/win32.rs`・`crates/awase-windows/src/state/key_role.rs`
（決定B7、`golden_scenarios.rs`ではなくこれらのモジュール内の
`#[cfg(test)]`、`keymap.rs`は対象外）。関連ADR: ADR-141（本ADRの
前提、Phase A、r3で収束・r4未実施）、ADR-114（`[[keymaps]]`、意味論の
対比元）、ADR-111/126（Scancode Map、GUIパターンの対比元）、ADR-127
（設定GUI単一Apply原則）、ADR-019（coreのOS非依存維持、決定B1で不採用と
判断した根拠）。

## レビュー履歴

- r0（2026-09-06）: 初版。
- r1（2026-09-06）: Opus 2体（architect役・premortem役）による1ラウンド目
  の敵対的レビューで、config例が`from_name`で解決できない事実誤認
  （決定B0）、`THUMB_KEY_ALIASES`拡張がUS配列検証を自己矛盾させる問題
  （決定B0）、バリデーションのcore/platform分割がissue #99と同型の
  二重管理バグを再導入する根本的欠陥（決定B1、両エージェント独立に
  到達）、既に存在する`warn_on_engine_hotkey_collision`の再発明（決定B3、
  両エージェント独立に到達）、`validate()`でのルール集合無効化が
  config.tomlから手書きルールを消失させるデータ損失（決定B6、両
  エージェント独立に到達）、`golden_scenarios.rs`が抽象レベル・cfgゲート
  の両面でテスト不可能な場所だった問題（決定B7）、sync keyがper-appでは
  なくグローバルだった事実誤認（決定B3）、Altセンチネルのグレーアウト
  判定をスロット位置ではなく`resolve_thumb_key()`の戻り値から計算すべき
  という実装上の誤り（決定B2）、`[[keymaps]]`重複の実害過小評価（決定B4）、
  単独タップ設定の注記対象セクションの誤り（決定B5）を検出。決定B0/B1/
  B2/B3/B4/B5/B6/B7を修正して反映。
- r2（2026-09-06）: 同一レビュアーへの再確認（2ラウンド目）で、r1の
  改訂作業自体が新たに混入させた問題を検出——
  `warn_on_engine_hotkey_collision`が`pub(crate)`かつ戻り値`()`のため
  「直接呼び出して結果を取得する」が実際には不可能だった（決定B3、
  両エージェント独立に到達）、決定B1がAlt適用前vk退避の訂正過程で起動時
  バリデーションの実行を誤って書き落としていた、`resolve_thumb_key`の
  import経路がcfgゲートの罠を持つ（決定B1）、`win32.rs:169`は
  `#[cfg(windows)]`かつ`windows`クレート型を要求するためLinux実行不可能
  という「Linuxで実行可能」の誤り、かつ訂正先のもう一方`keymap.rs:29`は
  役割代入の影響を受けない静的判定で保護対象が無い（決定B7、両エージェント
  独立に到達）、決定B2の「GUIは無効な組み合わせを生成できない」という
  主張が同じ決定内のクロスタブ機構と矛盾していた（決定B2）、プレビュー
  表示が逆写像ではなく順方向写像になっていた（決定B2）、を検出。決定B1/
  B2/B3/B7を修正して反映。あわせてADR-141本体の決定6-3（誤帰属していた
  `transport.rs::plan`への影響）に訂正注記を追加した。
- r3（2026-09-06）: 同一レビュアーへの再々確認（3ラウンド目）で、
  両エージェントとも「設計判断としては収束」と判定した上で、r2の
  訂正が本文の一部にしか反映されず矛盾が残っていた4箇所（背景の
  `fix-requires-evidence.md`該当性の節、決定B3末尾、未解決の疑問3、
  関連ファイル節——いずれも`keymap.rs:29`をテスト対象として言及した
  まま）、決定B7の「2層で責務を分ける」記述が`decide_role_substitution`
  の戻り値の要素を取り違えていた実装指示レベルの誤り（`confirmed_target`
  ではなく書き換え後vk＝第1要素が正しい）、決定B7の項目1〜4が実際には
  Linux実行不可能なhook.rs側配線の性質だった問題（ADR-141決定2への
  申し送りとして`event_eligible`パラメータへの吸収を決定）、を検出。
  上記4箇所の文言統一、決定B7の型誤り修正とテスト範囲の再設計、
  `fix-requires-evidence.md`該当性の論拠の張り直し、確認ダイアログの
  排他制御、3-cycleプリセットでのプレビューテスト要件を反映した。
- r4（2026-09-06）: 同一レビュアーへの再々々確認（4ラウンド目）で、
  r3が新設した`event_eligible`パラメータ自体に、両エージェントが
  独立にBUG-41と同型の欠陥（`event_eligible=false`の無条件早期return
  がstuck keyを別経路から再現し、決定2の「`was_down`/`confirmed_target`
  ペア更新」不変条件も破る）を発見した。修正は「`decide_alt_impersonation`
  の`engine_enabled`と同じく、新規押下時点でのみ`event_eligible`を
  参照する」の1点で両エージェントが同一の解に到達。あわせて項目4の
  「冪等性テスト」が純粋関数の性質上検出力を持たないという指摘
  （3-cycleプリセットでの期待値固定に差し替え）、申し送り先がADR-141
  決定2だけでなく決定1-1にも及ぶこと（グローバルラッチ禁止の警告の
  継承・16→32通りへの訂正を含める）、項目1のスロット混線テストが
  保証する範囲の明確化（純粋関数の独立性のみ、配線の正しさは対象外）
  を反映した。
- r5（2026-09-06）: 同一レビュアーへの再々々々確認（5ラウンド目）で、
  両エージェントとも**Blockerゼロ、収束、マージ可**と判定した。
  premortem役が新規押下限定参照の派生ケース（injected KeyDownが
  fresh pressの場合、対応するDownが無い孤児KeyUpの場合）を検証し
  問題なしと確認。architect役は「新規押下限定参照は`was_down`返り値化
  という自分の提案より優れている（既存の`decide_alt_impersonation`と
  規律が揃う副次的利点もある）」と評価を訂正した。残ったNit（決定1-1の
  `!is_injected`ガードが呼び出し側の早期returnのままでは`event_eligible`
  の`is_injected`成分がデッドコードになる、ADR-141本体への申し送り
  ポインタが無いとADR-141単体を読む実装者が旧シグネチャのまま実装
  しかねない、申し送りブロックの位置がリスト構造を分断していた）を
  反映し、ADR-141本体（決定1）にも申し送りポインタを追加した。両
  エージェントとも「これ以上のレビューラウンドは不要」と判定。
