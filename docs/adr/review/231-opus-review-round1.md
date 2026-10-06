# ADR-231 敵対的レビュー round1(Opus、2026-10-05)

対象: `docs/adr/231-gji-custom-keymap-tsv-generator.md`(起草、未コミット)。
読み取りのみ。裏取りに使ったもの:

- リポジトリ: `crates/awase-gji-config/src/{tsv,keymap,role,known_keymap,lib}.rs`、
  `crates/awase-windows/src/state/key_effect_predictor.rs`、`crates/awase-windows/src/gji_charset_autodetect.rs`、
  ADR-199(L60-95, L180, L236-250)、ADR-186 決定(c)、`docs/adr/186-measurements/config1-custom-keymap-table-ignored.tsv`。
- Mozc master(2026-10-05 取得、raw.githubusercontent.com): `LICENSE`、`src/gui/config_dialog/keymap_editor.cc`、
  `src/gui/config_dialog/config_dialog.cc`、`src/session/keymap.cc`、`src/composer/key_parser.cc`、
  `src/win32/base/keyevent_handler.cc`、`src/data/keymap/{ms-ime,atok,kotoeri}.tsv`。
  行番号はこの取得時点のもの。

---

## Blocker

### B1. 土台を「Mozc の素のプリセット表」に固定すると、既に CUSTOM を使っているユーザーの設定を黙って消す

ADR 決定2 は「土台のプリセット(MS-IME / ATOK / ことえり)」の全行から生成する。既に CUSTOM 表を使っているユーザーが
このツールで1キーを割り当てると、それまでの独自の割り当てがすべて消える。

- 根拠になっている実機サンプル `186-measurements/config1-custom-keymap-table-ignored.tsv` は、ADR の言う
  「プリセット相当の全行」ではなく、**MS-IME プリセットを土台にユーザーが大きく変えた表**である。Mozc の `ms-ime.tsv` と
  比べると、サンプルにしか無い行が 36、プリセットにしか無い行が 20(`comm` で確認)。サンプルにしか無い行は
  F15〜F19 の5状態分の `CompositionMode*`、`Henkan` の `IMEOn`/`CompositionModeHiragana`、`Precomposition Ctrl Shift Insert IMEOn` /
  `Ctrl Shift Delete IMEOff` など。
- まさにこのツールを使いそうなユーザー(IME キーを自分で割り当てたい人)が既に CUSTOM を持っている可能性は高い。
  決定2 の操作をサンプルのユーザーに適用すると、F15〜F19 のモードキーと Ctrl+Shift+Insert/Delete の ON/OFF がすべて消える。
  import は GJI 側で「Do you want to overwrite the current keymaps?」と聞くだけで(`keymap_editor.cc` L483-491)、
  差分は見せない。

修正案: 土台は「現在の実効の表」にする。
- `session_keymap == CUSTOM` かつ表が空でない → `config1.db` の `custom_keymap_table`(既存の読み取り資産で取れる)。
- プリセット(または不在/NONE、CUSTOM で表が空) → そのプリセットの表。
- ATOK プリセットに古い custom 表が残っている構成(サンプルがまさにこれ、ADR-186(c)) → 残っている custom 表ではなく
  ATOK の表。
「土台を選ぶ」UI は、現在の構成と違う土台を明示的に選んだときだけ「今の割り当ては失われる」と警告する。

### B2. CUSTOM に切り替わると awase の打鍵時予測がプリセットの同梱表を失う(ADR が副作用として触れていない)

中身がプリセットと同一の表を import しただけでも `session_keymap` は CUSTOM になる(`config_dialog.cc` L739-742
`EditKeymap()` が editor の OK 後に combobox を "Custom keymap" にする)。awase 側ではこれにより次が起きる。

- `KeyEffectKeymap::from_config` は CUSTOM を `KeymapPreset::Custom` にする(`key_effect_predictor.rs` L607-611)。
  `bundled_table` は空(L248)なので、**同梱表による IME キーの打鍵時予測が無くなる**。学習済みの表があればそれを使う(L42-46 doc)が、
  `fingerprint`(`gji_keymap_fingerprint`)が `custom_table` を含むので、切り替え前の学習表は指紋が合わず使えない。
- `classify_known_gji_keymap` は CUSTOM を常に「既知構成でない」とする(`known_keymap.rs` L66、テスト
  `custom_session_keymap_is_never_known`)。ADR-196 決定1c の同梱表との突き合わせも行われなくなる。

つまり「MS-IME プリセットの F13 に半角/全角の振る舞いを足すだけ」の操作で、awase の予測の精度が
「同梱表あり」から「学習するまで予測なし」に下がる。これは ADR-196/199 の前提を変える副作用で、ADR に
(a) 書く、(b) 受け入れるか対策するかを決める必要がある。対策の候補:
- 予測側で「CUSTOM 表が既知プリセットの表 + 少数の行差分」と判定できたらプリセットの同梱表を使う
  (差分のキーのセルだけ未知にする)。これは本 ADR の範囲を広げるので、少なくとも別タスクとして明記する。
- 生成後、学習(ADR-195)をやり直すよう案内する。

### B3. ライセンスの事実誤認: Mozc は Apache-2.0 ではなく BSD-3-Clause

決定1 は「Mozc `src/data/keymap/`、Apache-2.0」と書いているが、google/mozc の `LICENSE` は
「Copyright 2010-2018, Google Inc. All rights reserved. Redistribution and use in source and binary forms ...」の
**BSD 3-Clause**(取得して確認)。BSD-3 は**バイナリ配布でも著作権表示・条件・免責をドキュメント等に再掲する**ことを求める。
`include_str!` で `awase-settings.exe` 等に埋め込むなら、Scoop の zip・インストーラにその表記を同梱する必要がある。
リポジトリには `THIRD_PARTY` 系のファイルがまだ無い(`LICENSE-APACHE`/`LICENSE-MIT` のみ)ので、配布物への同梱経路も
決める必要がある。なお既存の `crates/awase-gji-config/src/lib.rs` L22 の doc コメントも「Apache-2.0」と誤記している(本 ADR の範囲外だが同じ誤り)。

(B1 の修正案を採ると、CUSTOM ユーザーでは同梱表が不要になり、プリセットユーザーでも下の S1 の代案なら同梱自体が不要になる。)

---

## Should-fix

### S1. 同梱しなくても、GJI 自身が「その版の」プリセット表を出せる。決定1 の根拠(版ずれ)を弱める代案を検討していない

`keymap_editor.cc` を読むと、GJI のキーマップエディタには次がある。
- 「Import predefined mapping」→ ATOK / MS-IME / Kotoeri(L262-268、L497-503)。GJI バイナリに埋め込まれた表を読み込む。
- 「Export to file...」(L273-274、L505-506)。
- プリセット選択中に「編集」を押すと、そのプリセットの表が最初から読み込まれた状態で開く(`config_dialog.cc` L720-736)。

したがって「編集 → (そのまま) エクスポート → awase に読ませる → 生成 → インポート」なら、
**ユーザーの GJI の版の表そのもの**が土台になり、未確認事項3(版ずれ)とライセンス(B3)が両方消える。手順は1つ増えるが、
import 自体も手動なので導線の性質は変わらない。決定1 は「同梱する」を確定とせず、
(i) CUSTOM ユーザーは `config1.db` から、(ii) プリセットユーザーは GJI のエクスポート、または同梱表(フォールバック)、
の比較として書き直すべき。少なくとも却下するなら理由を書く。

### S2. 「GJI が不正な表を拒否すれば何も起きない」は誤り。GJI は不正な行を黙って捨てて保存する

決定1 末尾の安全性の議論は成り立たない。`KeyMapEditorDialog::LoadFromStream` / `Update`(L345-461)の実挙動:
- 1行目は**中身を見ずに読み飛ばす**(L330-332「must have 1st line」)。ヘッダを書き忘れると最初のデータ行が消える。
- `ReportBug` コマンドの行はリリースビルドで捨てる(`IsValidEntry`、`#ifdef NDEBUG`、L170-176)。Mozc の3プリセットには
  `Conversion F1/F3 ReportBug` がある。実機サンプルにもこの2行が無い(上の `comm` 差分で確認)。
- その版の GJI が知らないコマンド名の行は `LOG(ERROR)` して**黙って捨てる**(L426-430)。状態名が
  `kKeyMapStatus`(DirectInput/Precomposition/Composition/Conversion/Suggestion/Prediction、**ZeroQuerySuggestion が無い**、L80-83)に
  無い表示可能な行も同様に捨てる(L419-423)。
- `Kanji`/`ON`/`OFF` キー、`InsertCharacter` 等の非表示行は表示せず保持し、**保存時に表の末尾へまとめて付け直す**(L367-372、L455)。
- 拒否されるのは、表示可能な行でキーが解釈できない場合だけ(「Invalid key」、L432-440)。

つまり同梱した Mozc master の表に新しいコマンド名が入っていて、ユーザーの GJI が古いと、その行だけ欠けた表が保存される。
「`config1.db` を壊さない」は「GJI 設定ファイルとしては壊れない」の意味でしかなく、Backspace 等が欠けた表になりうる。
決定1 の文言を直し、次の S3 の確認で拾えるようにする。

### S3. 決定5 の import 後の確認は、GJI の正規化を考慮しないと必ず「不一致」になる

上の S2 のとおり GJI は保存時に (a) `ReportBug` 行を落とし、(b) 非表示行(`Kanji`/`ON`/`OFF`/`ASCII InsertCharacter` 等)を
末尾へ移す。生成した TSV と `custom_keymap_table` を文字列で比べると、正しく import できていても毎回不一致になる。
比較は「(status, 正規化したキー, command) の多重集合」で行い、GJI が落とすと分かっている行(`ReportBug`)は除外する。
この「GJI の editor の import を模した正規化」を純粋関数として持ち、決定4 のテストに入れる(往復テストの相手は
`parse_custom_keymap_table` ではなくこの関数であるべき)。
並べ替えは後勝ちの評価に影響しない(非表示キーは表示キーと別のキーなので同じ (状態, キー) の行が前後しない)ことも確認済みとして書ける。

### S4. 決定5 の手順が不足: 「プロパティの OK/適用」と「DirectInput の変更は起動済みアプリに効かない」

- editor の OK だけでは `config1.db` に書かれない。プロパティ画面の OK/適用で `custom_keymap_table_` が config に入る
  (`config_dialog.cc` L600-602)。手順に明記しないと、ユーザーは import 済みのつもりで awase の確認が失敗する。
- editor 自身が「Changes of keymaps for direct input mode will apply only to applications that are launched after
  making your modifications.」と出す(`keymap_editor.cc` L444-452、`_WIN32`)。新しいキーに IME ON(DirectInput 行)を持たせる
  のが本 ADR の主用途なので、**起動済みのアプリでは新しいキーで IME が開かない**。手順に「アプリの再起動(または再サインイン)」を入れる。
  awase が新しいキーを能動トグルにする場合(S5)は awase が `VK_IME_ON` を送るので症状が隠れるが、受動のキー(F13 に ATOK の `Henkan` を
  コピーした等)では隠れない。
- 「「カスタム」へ切り替え → 「編集」」の順は不要。どのプリセットから「編集」を開いても、OK 後に CUSTOM へ切り替わる
  (`config_dialog.cc` L739-742)。逆に先に「カスタム」に切り替えて表が空だと、プロパティ画面が空の表の警告を出す(L371-376)。
  手順は「編集 → 編集メニュー → ファイルからインポート → 上書き確認で OK → OK → プロパティで OK」が実物に近い。

### S5. 生成したトグルが awase の役割判定で能動になる副作用を書いていない(ADR-199 との整合)

`role.rs::key_role` は CUSTOM 表を状態表で評価し、候補キー(半角/全角・F13〜F24・変換/無変換)がトグル形で、かつ `ON`/`OFF` 行が
全開状態に揃っていれば `ImeToggle` を返す(L126-162)。Mozc プリセットを土台にすると `ON`/`OFF` 行は揃っている
(非表示行として GJI が保持)ので、「IME ON/OFF トグル」(= `Hankaku/Zenkaku` の行群)を候補キーにコピーした瞬間に
そのキーは awase の能動制御になる(ADR-199 L180「ユーザーが別のキー(例: F13)をトグルにしたら、そのキーを awase が能動制御する」)。

- F13〜F24: awase が Suppress して `VK_IME_ON`/`VK_IME_OFF` を送る経路に入る。意図どおりだが ADR-231 に書かれていない。
- **変換/無変換は NICOLA の親指キー**。ここにトグルをコピーすると ADR-199 決定16 / ADR-206 の `role_open_action` が
  単独タップの扱いを変える(エンジン非活性側でも Consume + 絶対 `SetOpen`、ADR-199 L24)。親指シフトのユーザーが
  「無変換に IME OFF を」と選びがちな箇所なので、UI でどう振る舞うかを明示する必要がある。
- 逆向き: ATOK の `Henkan`(Composition が `Convert`)のような非トグルをコピーした候補キーは受動のまま。
  「ATOK の変換キーの振る舞い」を F13 に付けたユーザーは、awase の能動制御が付かないことを知らない。
- `Hankaku/Zenkaku` の行を「移動」(元の行を除去)すると、半角/全角は受動に変わり(ADR-199 決定4)、0x19 も
  `Hankaku/Zenkaku` 行に従う(ADR-202 決定1、`key_effect_predictor.rs` の `lookup_vk`)ので Alt+半角/全角も受動になる。

決定4 のテストに「生成後の `key_role` が期待どおり」を入れるとあるが、期待値そのもの(どのコピーで能動になるか)を
ADR に表で書き、UI でも「このキーは awase が IME を開閉します」と表示するべき。

### S6. 「振る舞いのコピー」の行の選び方と衝突除去の照合が未定義で、後勝ち規約と組み合わさると壊れる

決定2 の (a)(b) は「キー列が一致する行」とだけ書いている。実際の照合は文字列一致では足りない。
- Mozc のキー名は大文字小文字を区別せず、別名がある: `hankaku`/`zenkaku`/`hankaku/zenkaku` → HANKAKU、`kana`/`hiragana` → KANA
  (`key_parser.cc` L98-116)。衝突除去を文字列一致で行うと、別名で書かれた行(例: ユーザー表の `zenkaku`)が残り、
  `KeyMap::AddRule` の後勝ちで、コピーした行より後ろにあればそれが勝つ。照合は key_parser 相当の正規化(既存の
  `mozc_key_vk_names` の別名処理と揃える)で行うこと。
- 振る舞い元の「行群」に修飾付きの行(`Shift Henkan`、ATOK の `DirectInput Shift Henkan Reconvert` 等)を含むかが未定義。
  無修飾の行だけをコピーし、新しいキーの修飾付き行(`Shift F13` 等)は衝突とみなさず残す、と明記する。
- 追加する行の位置: 追加行を末尾に置き、除去を正規化照合で行えば後勝ちの問題は出ないが、S3 のとおり GJI が非表示行を末尾に
  移すので、「末尾に置いたから勝つ」という前提は非表示キー(`Kanji`/`ON`/`OFF`)に対しては成り立たない。新しいキーにこれらを
  選べないようにする(下の S7)。
- 状態の継承: 振る舞い元に Suggestion/Prediction の行があれば一緒にコピーされるが、新しいキーの既存の Suggestion 行を除去しないと
  継承より優先されて残る。(b) の除去は「全状態」で行うと明記する(Mozc の3プリセットでは該当キーの Suggestion/Prediction 行は
  無い、`role.rs` L313-317 のコメントのとおりだが、B1 の修正で CUSTOM 表を土台にするとあり得る)。
- 振る舞い元に行が無い状態は、新しいキーでも「行が無い」(= そのキーは素通しでアプリに届く)になる。例: MS-IME の `Muhenkan` は
  DirectInput 行が無いので、F13 にコピーすると F13 の `DirectInput IMEOn` が消える。「コピー」の意味としては正しいが、
  UI で事前に見せないと驚かれる。

### S7. 決定3 の逆引きと「未確認」の事実誤認

- 「Mozc の TSV に無いキー名(`Eisu` など)」は誤り。`ms-ime.tsv` には `DirectInput Eisu IMEOn`・`Composition Eisu ToggleAlphanumericMode`
  等があり、`key_parser.cc` L103 に `eisu` がある。`role.rs` テストの `MS_IME_DIRECT_INPUT` にも入っている。
- `Caps` はキーとして割り当てられない: `key_parser.cc` の `caps` は修飾キー `KeyEvent::CAPS` で、Windows の `VK_CAPITAL` は
  `CAPS_LOCK` に写る(`keyevent_handler.cc` L82)。未確認事項に挙げる必要はなく、候補から外すと書けばよい。
- 英数キーは kbd106 で状態により `VK_DBE_ALPHANUMERIC` と `VK_CAPITAL` を出し分ける(ADR-199 L236)ので、`Eisu` に振る舞いを
  コピーしても、物理キーを押したときに効くとは限らない。候補に出すなら注記が要る。
- `VK_KANA`(0x15)・`VK_KANJI`(0x19)は Mozc の Windows 側で `NO_SPECIALKEY`(IMM32、`keyevent_handler.cc` L83/L93)。
  `Kanji` はエディタで非表示(L125)なので、新しいキー/振る舞い元のどちらにも出さない。
- 逆引きの多対一: `VK_DBE_HIRAGANA` → `Kana`/`Hiragana`、0xF3/0xF4 → `Hankaku/Zenkaku`。出力の表記は1つに決め(プリセット表で
  使われている `Hiragana`・`Hankaku/Zenkaku`)、照合は S6 の正規化で行う。
- 未確認事項1(「`Kanji` 行・修飾キー付き行を受け入れるか」)はソースで答えが出ている: `Kanji`/`ON`/`OFF` は非表示行として保持され
  末尾へ移る、修飾付きは表示行として通常どおり受け入れる(`KEY_DOWN`/`KEY_UP` 修飾だけ非表示)。実機サンプルにも `Kanji` 4行・
  `Ctrl Shift Insert` 行が残っている。実機確認は「ソースどおりか」の確認に格下げできる。

---

## Note

### N1. ADR-199 決定4「TSV は同梱しない」を覆す理由が弱い

ADR-199 の「同梱しない」は役割判定の文脈(プリセットはトグル行だけの定数表で足りる)での判断で、ADR-231 決定1 は
それとは別の用途(生成の土台)なので「見直す」というより「別の用途で同梱するか」の判断になる。S1 の代案を採れば覆す必要もない。
`role.rs` のテスト抜粋を同梱表に寄せる話(決定1 第2項)は本 ADR の範囲外にしてよい。

### N2. 複雑性予算は現時点で直接は効かない

`.claude/rules/complexity-budget.md` は未発効で、対象も actuation 合流点と `tuning.rs` 定数だけ。本 ADR の追加
(`awase-gji-config` の生成関数、`awase-settings` の UI)は対象外。ただし ADR-158 RC4 の精神からは、
「同梱表 3 ファイル + 版の追随」という保守対象が増えることは書いておくべき(Mozc 側の表は 2026-09 時点の最終変更が
`b4bbc42f` で、変更頻度は低い)。

### N3. 「図解で案内」だけの代案は採れない理由を ADR に書いておく

GJI の editor の「ファイルからインポート」は**全置換**しかなく(L483-503)、行の追加マージはできない。図解で「この4〜6行を
足してください」と案内すると、状態ごとの入力という当初の困りごとそのものが残る。よって TSV 生成は妥当。ただしこの理由を
ADR に書かないと、次のレビューで同じ代案が再浮上する。

### N4. overlay_keymaps は CUSTOM でも残る

Mozc は CUSTOM でも `ApplyOverlaySessionKeymap` を後勝ちで重ねる(`keymap.cc` L150-155)。overlay(変換→IMEOn、無変換→IMEOff)が
有効なユーザーが変換/無変換に振る舞いをコピーしても上書きされる。awase の `key_role` も overlay があれば変換/無変換を受動にする
(`role.rs` L135-141)。生成時に `overlay_keymaps` を読み、該当キーを候補から外すか警告する。overlay は GJI の画面のどこで切り替わるか
(Windows の GUI に出るか)も未確認。

### N5. ATOK/ことえりプリセットから切り替えたときに失われるもの

プリセットの表をそのまま土台にする限り、表で表現される挙動(ATOK の `CancelAndIMEOff`・変換/無変換の DirectInput `IMEOn`、BUG-115 の前提)は
CUSTOM でも同じ行として残る。失われるのは表の外のもの: (a) awase 側の「既知プリセット」扱い(B2)、(b) ADR-186(c) の
「ATOK 選択中は古い custom 表を無視する」前提に依存していた awase のコード経路(`gji_charset_autodetect.rs` L207-214 付近)が
CUSTOM 側の評価に移る。ことえりは Mac 向けの表(`Option F1` 等の Mac 修飾キー行を含む)で、Windows で土台に出す意味は薄い。
土台は MS-IME/ATOK の2つに絞ってよい。

### N6. 決定4 の「同梱表を無変更で出力 → parse して元と一致」は自明で価値が低い

`parse_custom_keymap_table` は空白や順序を正規化しないので、出力関数が文字列をそのまま連結する限り必ず一致する。
意味のある往復テストは S3 の「GJI の import を模した正規化」を通したうえで、`key_role`・`extract_ime_keys`・
`classify_known_gji_keymap` の結果が期待どおりになること。

### N7. 生成する TSV のファイル形式

- 先頭行は必ず `status\tkey\tcommand`(S2: 1行目は中身を見ずに捨てられる)。
- 改行は CRLF でも可(`Util::ChopReturns`)。`#` で始まる行はコメントとして捨てられるので、生成元・Mozc の版・生成日時を
  `#` 行で入れておくと、後で `custom_keymap_table` を見たときに awase 生成物かどうかを判別できる……ただし GJI は保存時に
  コメント行を落とすので `config1.db` には残らない(ファイル側だけ)。
- BOM を付けると1行目(ヘッダ)に付くだけなので実害は無いが、付けない方が無難。

### N8. ADR-230(Scancode Map)との接続

F13〜F24 は日本語キーボードに物理的に無いので、実用上は ADR-230 の Scancode Map か awase のリマップで作ったキーに振る舞いを
付けることになる。UI の「新しいキー」は物理キー名ではなく「OS が出す VK」で選ばせる、と ADR-230 と用語を揃えておく。
