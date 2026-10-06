---
id: ADR-231
title: |-
  GJI カスタムキーマップの TSV 生成(現在の実効の表を土台に、振る舞いをキーへコピー。import はユーザー手動)
summary: |-
  要望: IME キーの割り当てを、状態(Composition/Conversion/…)ごとに考えずにできるようにしたい。所有者判断(2026-10-06): GJI の `config1.db` には触らず、TSV を生成するところまでを awase が担い、import は GJI の画面でユーザーが手動で行う。MS-IME 本体・ATOK 本体の設定は対象外(GJI のみ)。提案: 「現在の実効のキーマップ」(CUSTOM ならその表、プリセットならそのプリセットの表)を土台に、「表内の既存キーの行群を新しいキーへ複製する」操作で完全な表を生成する。CUSTOM はプリセットを置き換えるため生成物は完全な表でなければならない。Opus round1 の指摘で、(1)土台を素のプリセットに固定すると既存 CUSTOM ユーザーの割り当てを消す→土台を現在の実効の表に変更、(2)CUSTOM への切替で awase の打鍵時予測が同梱表を失う副作用を明記、(3)Mozc のライセンスは BSD-3-Clause(Apache-2.0 は誤り)、(4)GJI は不正な行を黙って捨てて保存する(拒否しない)・保存時に正規化するので import 後の確認は正規化を模して比較、(5)生成したトグルが awase の能動制御になる副作用を表にする、(6)手順を実物に合わせる(プロパティ OK・起動済みアプリには効かない)、を反映した。
status: |-
  起草(2026-10-06)。Opus round1(Blocker 3・Should-fix 7・Note 8)・round2(新 Blocker なし、ずれた直し方 4・Should-fix 4・Note 2)反映済み(`docs/adr/review/231-opus-review-round{1,2}.md`)。round3(収束、Note 2 件も反映)。実装なし。Scancode Map 側は別 ADR(ADR-230)。
related_adr:
  - "ADR-186"
  - "ADR-195"
  - "ADR-196"
  - "ADR-199"
  - "ADR-206"
  - "ADR-230"
---

# ADR-231: GJI カスタムキーマップの TSV 生成

## 背景

IME キー(IME ON/OFF、トグル、モード切替)を自分の好きなキーに割り当てたいという要望がある。
GJI のキーマップエディタは「入力状態 × キー × コマンド」の表を編集する作りで、状態ごとに入力するのは手間であり、
できないユーザーが多い。

リポジトリには読み取り側の資産がある(`awase-gji-config`: `config1.db` の読み取り、`tsv.rs::parse_custom_keymap_table`、
`keymap.rs::mozc_key_vk_names`、`role.rs::key_role`〈ADR-199〉、`known_keymap.rs`〈ADR-196〉)。書き込みは無い。

所有者の方針(2026-10-06): 生成は TSV まで、import は手動。自由度は低くてよい。MS-IME 本体は対象外(GJI のみ)。

### なぜ TSV を生成するのか(「図解で案内」だけにしない理由)

GJI のエディタの「ファイルからインポート」は**全置換**しかなく、行の追加マージはできない(Mozc `keymap_editor.cc`)。
「この4〜6行を足してください」と案内すると、状態ごとに入力するという当初の困りごとがそのまま残る。

## 裏取りで分かった前提(Mozc master 2026-10-05 とリポジトリで確認)

1. **CUSTOM はプリセットを置き換える**(Mozc `ApplyPrimarySessionKeymap`、ADR-199 L75)。生成物は完全な表。
2. **ATOK プリセット選択中は古い `custom_keymap_table` は無視される**(ADR-186 決定(c))。
3. **中身がプリセットと同一でも、import すると `session_keymap` は CUSTOM になる**(エディタの OK 後にコンボが
   「カスタム」へ切り替わる、`config_dialog.cc`)。プリセットのどこから「編集」を開いても同じ。
4. **GJI の import は不正な行を拒否せず、黙って捨てて保存する**(`keymap_editor.cc` `LoadFromStream`/`Update`):
   - 1行目は中身を見ずに読み飛ばす(ヘッダ必須)。
   - `ReportBug` 行はリリースビルドで捨てる。その版が知らないコマンド名・状態名(`ZeroQuerySuggestion` 等)の行も捨てる。
   - `Kanji`/`ON`/`OFF`/`ASCII InsertCharacter` 等の非表示行は表示せず保持し、**保存時に表の末尾へまとめて付け直す**。
   - 拒否されるのは、表示可能な行でキーが解釈できないときだけ。
5. editor の OK だけでは `config1.db` に書かれず、プロパティ画面の OK/適用で書かれる。DirectInput 行の変更は
   **変更後に起動したアプリにしか効かない**(エディタ自身がそう表示する)。
6. 実機サンプル `186-measurements/config1-custom-keymap-table-ignored.tsv` は、素のプリセットではなく**MS-IME プリセットを
   ユーザーが大きく変えた表**(F15〜F19 の `CompositionMode*`、`Ctrl Shift Insert`/`Ctrl Shift Delete` の ON/OFF など。
   プリセットにだけある行は `ReportBug` 等)。
7. Mozc のライセンスは **BSD-3-Clause**(Apache-2.0 ではない)。バイナリ配布でも著作権表示・条件・免責の再掲が要る。
   `crates/awase-gji-config/src/lib.rs` L22 の doc も「Apache-2.0」と誤記している(本 ADR とは別に直す)。

## 決定(提案)

### 決定1: 土台は「現在の実効のキーマップ」

素のプリセットに固定すると、既に CUSTOM を使っているユーザー(まさにこのツールを使いそうな人)の割り当てを黙って消す
(import の上書き確認は差分を見せない)。そこで次の優先順で土台を決める。

| 現在の構成(`config1.db`) | 土台 |
|---|---|
| `session_keymap == CUSTOM` かつ表が空でない | その `custom_keymap_table` |
| `config1.db` が**不在**(Mozc は既定=Windows では MSIME) | MS-IME の表(同梱) |
| プリセット MSIME(不在・NONE・表が空の CUSTOM を含む) / MOBILE(`ms-ime.tsv` と同一、`role.rs` のテストで確認済み) | MS-IME の表(同梱) |
| プリセット ATOK | ATOK の表(同梱) |
| プリセット KOTOERI | **GJI のエクスポートを必須にする**(ことえりは同梱しない) |
| `config1.db` が**読めない・パース不能**(権限・ロック中・GJI の版で protobuf が変わった等。実際は CUSTOM 表を持っている可能性がある) | 同梱表を既定にしない。**GJI のエクスポート(決定2)を必須にする**(または生成を止める) |
| ATOK プリセットに古い custom 表が残っている構成(サンプルがこれ) | 残っている表ではなく ATOK の表(ADR-186 決定(c)) |

「土台を選ぶ」UI は、現在の構成と違う土台を明示的に選んだときだけ「今の割り当ては失われる」と警告する。
不在と「読めない」を分けるのは、awase 既存の規約(`from_config1_db_read` は不在=既定、読めない=不明)と、「ユーザーの現在の
割り当てを知らないまま上書きしない」という本決定の意図に合わせるため(読めないのに MS-IME から生成すると B1 を例外経路で再発させる)。

### 決定2: プリセット表の入手元は「同梱」を既定、GJI のエクスポートを代替入力とする

- 同梱するのは Mozc `src/data/keymap/` の **`ms-ime.tsv` と `atok.tsv` の2つ**(ことえりは Mac の修飾キー行を含み、
  Windows で土台に出す意味が薄いので外す)。Mozc のコミットを固定して記録する。
- **ライセンス**: BSD-3-Clause。バイナリ配布でも表記が要るので、`THIRD_PARTY_NOTICES`(新設)に Mozc の著作権表示・条件・
  免責を書き、Scoop の zip・インストーラへの同梱経路も決める(**実装の前提条件**)。
- **版ずれ**: Windows 版 GJI が実際に持つ表とは違いうる。そのため、ユーザーが GJI の「編集 → ファイルへエクスポート」で
  出した TSV を awase に読ませる経路も用意する(`parse_custom_keymap_table` で足りる。その版の表そのものが土台になり、
  版ずれが消える)。UI では「同梱の表(既定)/GJI からエクスポートしたファイル」を選べるようにする。
- 保守対象が増える(同梱表2つ + 版の追随)。Mozc 側の表の最終変更頻度は低い(2026-09 時点で `b4bbc42f`)ので許容範囲と判断する。
- ADR-199 決定4「TSV 本体は同梱しない」は、役割判定の文脈(トグル行だけの定数表で足りる)の判断であり、
  生成の土台という別用途の同梱は覆すのでなく別の判断として扱う。`role.rs` のテスト抜粋を同梱表へ寄せる話は本 ADR の範囲外にする。

### 決定3: 操作は「振る舞いのコピー」(照合は Mozc と同じ正規化で行う)

ユーザーが選ぶのは「土台」「新しいキー」「振る舞い元 = 土台の表内の既存キー」(例: `Henkan`、または IME ON/OFF トグル =
`Hankaku/Zenkaku` の行群)。

生成: 土台の全行をコピーし、(a) 振る舞い元の行群の `key` 列を新しいキーへ差し替えた行を追加、
(b) 新しいキーの既存の行を**全状態で**除去、(c) 振る舞い元の行は残す(コピー。除去する「移動」は別オプション)。

- **順序**: (b) の除去を先に行い、振る舞い元の行群は**除去前に取り出しておく**。新しいキーと振る舞い元が正規化後に同じ
  (別名を含む。例: `Hankaku/Zenkaku` に `zenkaku` の行群をコピー)なら、UI で選べなくする。「移動」でも、振る舞い元の除去が
  新しいキーの行を消さないことをテストで固定する。
- **照合は文字列一致でなく Mozc の `key_parser` 相当に正規化**する(大文字小文字を区別しない、別名 `zenkaku`/`hankaku`/
  `hankaku/zenkaku`、`kana`/`hiragana` を同一視。既存 `mozc_key_vk_names` の別名処理と揃える)。文字列一致だと別名で書かれた
  行が残り、`KeyMap::AddRule` の後勝ちでコピーした行を打ち消すことがある。
- **無修飾の行だけ**をコピーする。新しいキーの修飾付き行(`Shift F13` 等)は衝突とみなさず残す。
- 状態の継承(Suggestion→Composition、Prediction→Conversion)があるので、(b) は全状態で行う。
- 振る舞い元に行が無い状態は、新しいキーでも行が無い(素通し)になる。例: MS-IME の `Muhenkan` は DirectInput 行が無いので、
  F13 にコピーすると F13 の `DirectInput IMEOn` は消える。UI はコピー前に「このキーの現在の行/コピー後の行」を状態別に見せる。
- 新しいキー・振る舞い元に選べないもの: `Kanji`/`ON`/`OFF`(非表示行で、保存時に末尾へ移るため後勝ちの前提が崩れる)、
  `Caps`(Mozc では修飾キーで、キーとして割り当てられない)。`Eisu` は選べる(Mozc の表にある)が、英数キーは kbd106 で
  状態により `VK_DBE_ALPHANUMERIC`/`VK_CAPITAL` を出し分けるので注記を出す。
- **overlay**: `overlay_keymaps` は CUSTOM でも後勝ちで重なる(`ApplyOverlaySessionKeymap`)。overlay が有効で変換/無変換を
  書き換える構成では、その2キーを候補から外すか警告する(`role.rs` も overlay があれば変換/無変換を受動にしている)。
- 「新しいキー」は物理キー名でなく **OS が出す VK** で選ばせる(ADR-230 と用語を揃える。F13〜F24 は JIS 物理キーに無いので、
  実用上は Scancode Map などで作ったキーに振る舞いを付ける)。

### 決定4: 生成したトグルは awase の能動制御になる(UI に表示し、期待値を表で固定する)

`role.rs::key_role` は CUSTOM 表を状態表で評価し、候補キーがトグル形で `ON`/`OFF` 行が全開状態に揃っていれば `ImeToggle` を返す
(ADR-199 L180)。ただし**awase が実際にそのキーを能動制御するかは `key_role` の後にも条件が掛かる**
(`state/key_effect_runtime.rs::key_shadow_action`)。次の表の右列は `key_role` の結果で、実際の扱いは下の条件で変わる。

- **明示 config と重なると受動**: ユーザーが `keys.ime_on`/`ime_off`/`ime_toggle` に同じキーを書いていれば、GJI 表由来の役割は使われない
  (ADR-199 決定8、`explicit_overlap`)。
- 学習表との矛盾で狭められる(`hz_omit_verdict`、半角/全角)。
- `overlay_keymaps` があれば変換/無変換は受動。
- 土台に `ON`/`OFF` 行が全開状態で揃っていなければ `key_role` 自体が受動(`custom_table_has_toggle`)。**土台がユーザーの CUSTOM 表**に
  なったので、`ON`/`OFF` 行の無い古い表(手書き・他所から入手した TSV を import した表)ではトグルをコピーしても受動になる。
- 変換/無変換で `role_open_action`(ADR-206)が効くのは、単独タップが Passthrough のときだけ。「Suppress して `VK_IME_ON/OFF` を送る」も
  `transport.rs::plan` の条件次第で、言い切れない。

| コピーの内容 | `key_role` の結果 |
|---|---|
| トグル(`Hankaku/Zenkaku` の行群)を F13〜F24 へ | `ImeToggle`(能動の候補。実際に Suppress して `VK_IME_ON/OFF` を送るかは上の条件と `transport.rs::plan` 次第) |
| トグルを変換/無変換へ | `ImeToggle`。**NICOLA の親指キー**なので、条件を満たせば ADR-199 決定16/ADR-206 の `role_open_action` が単独タップの扱いを変える(単独タップが Passthrough のとき)。UI で明示する |
| 非トグル(ATOK の `Henkan` の `Convert` など)を候補キーへ | `None`(受動のまま。awase の能動制御は付かない)。UI でその旨を表示する |
| `Hankaku/Zenkaku` の行を「移動」(元の行を除去) | 半角/全角は `None`(受動)になる。0x19 も `Hankaku/Zenkaku` 行に従うので Alt+半角/全角も受動(ADR-202) |

UI は生成前に「このキーは awase が IME を開閉します/しません」を出すが、判定は `key_role` だけでなく**少なくとも `explicit_overlap`
(現在の config.toml)と土台の `ON`/`OFF` 行の有無**も見る。テストは `key_role` の結果をこの表どおりに固定し、加えて
「土台に ON/OFF 行が無い CUSTOM 表ではトグルをコピーしても受動」を固定する。

### 決定5: CUSTOM への切替が awase の予測に与える副作用を受け入れ、別タスクにする

import すると、中身がプリセットと同一でも `session_keymap` は CUSTOM になる。awase 側では:

- `KeyEffectKeymap::from_config` が `KeymapPreset::Custom`(同梱表は空)にするので、**同梱表による打鍵時予測が無くなり**、
  学習済みの表は指紋(`custom_table` を含む)が合わず使えない。
- `classify_known_gji_keymap` は CUSTOM を常に「既知構成でない」とする(ADR-196 決定1c の突き合わせが行われない)。

対応: (a) この副作用を UI の説明に出す(「割り当て後は学習(ADR-195)をやり直すと、既存の IME キーの予測が戻ります。学習・予測の対象は13キーで F13〜F24 は含まれません」)、
(b) 予測側で「CUSTOM 表が既知プリセットの表 + 少数の行差分」と判定できたら同梱表を使う(差分のキーのセルだけ未知にする)
改善は、本 ADR の範囲を広げるので**別タスク(別 ADR)**とする。本 ADR は(a)のみ実施し、(b)は未着手と明記する。

### 決定6: 検証は GJI の import を模した正規化を通して行う

GJI は保存時に ReportBug 行を落とし、非表示行を末尾へ移す(前提4)。生成した TSV と import 後の `custom_keymap_table` を
文字列で比べると、正しく import できていても毎回不一致になる。

- **GJI の editor の import を模した正規化関数**(純粋、`awase-gji-config`): **1行目は内容を見ずに無条件で捨てる**、
  列分割は GJI と同じ規則(連続タブは1区切り、`absl::SkipEmpty`。`tsv.rs::parse_custom_keymap_table` の `split('\t')` とは違う)、
  `ReportBug` 除去、非表示行(キー由来の `Kanji`/`ON`/`OFF`・キーが解釈できない行、コマンド由来の `InsertCharacter`/`EditInsert`)の
  末尾移動を再現し、`(status, 正規化したキー, command)` の**多重集合**にする。
- import 後の確認も、この正規化を通した多重集合で比較する(`ReportBug` 等 GJI が落とすと分かっている行は除外)。
  キー由来の非表示行は表示キーと別なので並べ替えは後勝ちに影響しないが、コマンド由来の非表示行(表示キーに `InsertCharacter` を
  割り当てた行など)では成り立たない。ユーザーの CUSTOM 表を土台にする以上、**同じ (状態, キー) に非表示行と表示行が両方ある表は、
  生成前に警告する**。
- テスト: 生成関数の出力を正規化関数に通し、`key_role`・`extract_ime_keys`・`classify_known_gji_keymap` の結果が
  決定4 の表どおりになること(「同梱表を無変更で出力して parse して一致」は自明で価値が低いので入れない)。
- **GJI に捨てられた行の検出**: awase にはユーザーの GJI の版が知るコマンド名の一覧を得る手段が無い(版ずれは Linux のテストで
  原理的に拾えない)。拾えるのは import 後の比較だけなので、比較で「生成したのに `custom_keymap_table` に無い行」を
  **「GJI に捨てられた行」として一覧表示**する(`ReportBug` は既知として除外)。Backspace 等の基本キーが含まれていれば
  「GJI の版が古い可能性があります。GJI からエクスポートした表を土台にして作り直してください」と案内する(決定2 の代替入力へつなぐ)。
  比較が不一致のときの文言は「失敗」一般にせず、「捨てられた行」と「余分な行」を分けて出す。
- 「GJI が不正な表を拒否するから安全」とは言えない。安全性の根拠は `config1.db` を直接書き換えないこと(import は GJI 自身の機能)と、
  決定1(既存の割り当てを消さない)・決定6(import 後に結果を確かめる)である。

### 決定7: ファイル形式と import の手順

- 先頭行は必ず `status\tkey\tcommand`。BOM は付けない。生成元・Mozc の版・生成日時は `#` コメント行に入れてよいが、**1行目は必ずヘッダ、コメントは2行目以降**
  (GJI は1行目を内容を見ずに捨てるので、コメントを1行目に置くとヘッダがデータ行として読まれ、非表示行として `config1.db` に残る。
  コメント行自体は読み込み時に捨てられ、ファイル側にしか残らない)。
- 手順(実物に合わせる): GJI のプロパティ → キー設定 →「編集」→ 編集メニューの「ファイルからインポート」→ 上書き確認で OK →
  editor を OK → **プロパティ画面でも OK/適用**。どのプリセットから「編集」を開いても OK 後にカスタムへ切り替わる
  (先に「カスタム」を選んで表が空だと空表の警告が出るので、この順は避ける)。
- 注意書き: **DirectInput 行の変更は、設定後に起動したアプリにしか効かない**(新しいキーで IME を開く主用途に直撃する)。
  アプリの再起動(または再サインイン)を手順に入れる。awase が新しいキーを能動トグルにする場合は awase が `VK_IME_ON` を送るので
  症状が隠れるが、受動のキーでは隠れない。
- import 後に awase が `config1.db` を再読込し、決定6 の比較で結果を表示する。`config1.db` が読めないときは確認もできないので、
  成功とも失敗とも表示せず「結果を確認できません」と出す。

## 非目標

- `config1.db` への書き込み、MS-IME 本体・ATOK 本体のレジストリ設定、GJI 以外。
- 自由な状態×キー×コマンドの編集(GJI のキーマップエディタの代替)。
- awase 自身の `keys.ime_on` などとの**自動連動**(TSV 生成が `keys.*` を書き換えること)。これらは「モードずれがあったときの強制復帰キー」で
  あり、二重管理にならない(所有者確認 2026-10-06)。ただし**独立ではない**: 新しいキーが `keys.*` に既に書かれていれば、GJI 表由来の
  役割は awase では使われない(ADR-199 決定8)ので、UI で警告する(決定4)。
- 打鍵時予測が CUSTOM 表を既知プリセット+差分として扱うようにする改善(決定5(b)、別 ADR)。

## 未確認

ソースで答えが出たもの(`Kanji` 行・修飾付き行の受け入れ、`Eisu` の存在)は実機確認を「ソースどおりか」に格下げした。残り:

- 同梱した Mozc master の表が、ユーザーの GJI の版にどこまで通るか(知らないコマンド名の行が捨てられる範囲)。
- `overlay_keymaps` が Windows の GUI のどこで切り替わるか。
- DirectInput 行の変更が起動済みアプリに効かない件の、awase 能動/受動それぞれでの実際の症状。

既存の実機 CI(`config1.db` を直接生成する方法、`sc-t1c-*` 系)と clipwire 経由の実機で確認する。

## 実装の段階(提案)

1. `THIRD_PARTY_NOTICES` の新設と配布物への同梱経路(前提条件)、Mozc 表2つの同梱。
2. 正規化関数・逆引き・直列化・振る舞いコピー・`key_role` の期待値テスト(`awase-gji-config`、Linux でテスト)。
3. 実機確認(未確認の項目)。
4. `awase-settings` の UI(土台の選択、状態別の差分表示、能動/受動の表示、TSV 保存、手順の表示、import 後の確認)。
