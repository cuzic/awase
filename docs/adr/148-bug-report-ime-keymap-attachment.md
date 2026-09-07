# ADR-148: 不具合報告へのIME別キーマップ/キー割り当て設定の添付

## ステータス

**実装済み（`feat/adr148-bug-report-ime-keymap`、develop未マージ）。**
設計は4ラウンドのOpus敵対的レビューで収束済み。実装後、同じ観点で
Opus敵対的コードレビューを実施し、Must-fix 1件（M-1:
`adopted_ime_toggle_combos`が空のとき`None`に潰していた——「MS-IME
非アクティブ」と「MS-IMEアクティブだが採用ゼロ件」を区別できなく
なる設計逸脱）・Should-fix 5件（S-1: テストフィクスチャの
分類系/採用系が実コードで到達不能な組み合わせだった、S-2:
`mozc_key_to_vk_name`allowlist依存の型docコメントが欠落、S-3:
`ime_*_keys`が安全範囲フィルタ適用前である旨の型docコメントが欠落、
S-4: 本ステータス節の更新漏れ〈今回対応〉、S-5: ADR-095送信項目
インベントリへの追記漏れ）を検出、全件反映した。呼び出し等価性
（`classify_thumb_key_ime_actions`/`gate_thumb_key_ime_actions`の
引数、`route_thumb_key_action`の分岐との一致）・`read_config1_db`の
副作用（既存ラッチを汚染しないか）・TypeScript側の整合性はいずれも
「該当なし」（問題なし）。

設計レビュー・実装レビューとも詳細な経緯は以下（設計フェーズ時点の
Must-fix 4件+Should-fix 12件〈F1〜F19〉、G1〜G6、H1・H2）を参照。

**Phase 2は2026-09-07に実装完了・developマージ済み**（PR #181、
コミット`b77bc1da`）。実機確認（`StyleList\Custom`のパス実在・6プリセット
構成・バイナリ形式・「IMEオン/オフ」トグル割り当てのコード`CE`/`CD`・
重複行による非対称な実効挙動）を踏まえ、当初方針のバイト長+ハッシュ
フィンガープリントではなく、実測コードに基づく直接検出
（`msime_legacy_keymap.rs`）を実装した。詳細は後述「Phase 2 実機確認」
「Phase 2 実装」節を参照。マージ前にOpus敵対的コードレビューを実施し、
Must-fix 1件（未知プリセットが実在しないレジストリパスを読みに行き
「割当てなし」と誤判定する設計逸脱）を含む5件を検出・全件反映した。

### 設計フェーズの経緯（参考、折りたたみ）

<details>
<summary>設計フェーズのOpus敵対的レビュー4ラウンドの経緯（折りたたみ）</summary>

Opus敵対的レビュー1ラウンド目でMust-fix 4件（F1・F2・F5・F8）・
Should-fix 12件（F3・F4・F6・F7・F9〜F16）を検出、反映した
（142→148改番含む）。2ラウンド目でF1〜F19は全件解消を確認された上で、
改訂自体が生んだ新規の欠陥がMust-fix 1件（G1: GJI/MS-IME「採用値」
フィールドに`ime_kind`一致ゲートが抜けていた）・Should-fix 2件（G2:
`custom_keymap_table_present`の欠落、G3: GJI側採用値の粒度〈分類系/
採用系〉未分離）検出され、反映した（参考意見G4〜G6も反映済み）。
3ラウンド目でG1〜G6は全件解消を確認された上で、`*_adopted_route`
関連の残課題がShould-fix 1件（H1: 専用Fnキー設定時に
`muhenkan_adopted_route: "Delegate"`が実際には発火しない優先順位の
マスキングが未反映）・実装メモ1件（H2: `ime_kind`評価を1回に固定する
実装上の注意）検出され、反映した。4ラウンド目でH1・H2の解消を確認、
追加指摘なし＝収束を確認済み。

</details>

### 番号について

起票時点では142として書いたが、`git worktree list`で確認したところ
`~/rust-nicola-worktrees/feat-adr140-key-role-substitution`
（ブランチ`feat/adr140-physical-key-role-substitution`、未マージ）が
既に`docs/adr/141-physical-key-role-substitution.md`〜
`146-gji-keymap-swap-tool.md`を、`fix/bug119-muhenkan-passthrough-delegate-priority`
（未マージ）が`147-thumb-key-delegate-defers-to-user-passthrough.md`を
占有していた。develop側の`141-henkan-muhenkan-delegate-inactive-recovery.md`
（PR #177マージ済み）と上記ブランチの141番も既に衝突済みであり、この
ADRを142のままにすると衝突がもう1つ増える。develop上で確実に未使用の
**148**に採番し直した。上記2ブランチをdevelopへマージする際は、
[feedback_bug_number_collision_on_branch_merge]（メモリ）の手順どおり
141〜147を含めた採番の洗い出しが別途必要になる（本ADRの対象外）。

なお`docs/adr/index.md`にはADR-141の行自体が無いことも確認した
（142追加時に気づいたが、本ADRの対象外として修正のみ行い、内容は
変更していない）。

## コンテキスト

### 発端

ADR-095で実装した不具合報告機能（タスクトレイ「不具合を報告」）は、
IME種別（`ime_kind: Gji/MsIme/Unknown`）と内部状態スナップショット
（`BugReportStateSnapshot`）は送るが、**そのIMEが無変換/変換/F13等の
物理キーにどんな意味論を割り当てているか**（キーマップ設定そのもの）は
一切含んでいない。ユーザー（awase開発者本人）から次の3点の要望が出た:

1. GJIを使っている場合、`config1.db`の内容に基づくキーマップ設定を送信したい。
2. 新しいMicrosoft IMEを使っている場合も、同様にレジストリの情報を送信したい。
3. 旧Microsoft IMEの場合も、キーマップの設定を送信したい。

不具合報告は「無変換キーがなぜか効かない」「IMEが勝手にON/OFFする」
といった症状で送られてくることが多く（`SymptomCategory::ThumbKeyMisbehavior`/
`ImeToggledUnexpectedly`）、報告者がGJI/MS-IMEのキーマップをどう
設定しているかが分からないと、awase側の自動検出ロジック
（`gji_charset_autodetect.rs`/`msime_key_assignment.rs`）が実際に
何を読んで何を検出したかを事後に再現できず、切り分けに往復のやり取りが
必要になっている。

### 既存資産（すでに実装済みで、今回は「bug reportに転記するだけ」の部分）

キーマップ読み取りロジック自体は、awase本体の自動追随機能として
**既に実装・実機確認済み**である。今回のスコープは新規の解析ロジックの
実装ではなく、既存の解析結果を`BugReportPayload`（`crates/awase-windows/
src/bug_report.rs`）に載せる配線が中心になる。ただし後述のとおり、
「配線するだけ」で済まない箇所がいくつかある（決定1・決定2・
「実装スコープの訂正」参照、レビューF10〜F12・F15・F16対応）。

- **GJI**: `crates/awase-gji-config`（独立crate）が`config1.db`の
  protobuf wire formatを解析し、`read_gji_ime_keys`/`read_gji_mode_keys`
  でIME ON/OFF/トグルキーと入力モード変更キーのVK名一覧を返す。
  `wire::parse_top_level`は`session_keymap`（CUSTOM/ATOK/MSIME/MOBILE等の
  プリセット種別）と`overlay_keymaps`（`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`
  等）も返す。呼び出し元は`crates/awase-windows/src/
  gji_charset_autodetect.rs::sync_gji_charset_autodetect`で、
  `config1_db_path()`（`%USERPROFILE%\AppData\LocalLow\Google\Google
  Japanese Input\config1.db`）を読んで解析している。同ファイルには
  無変換/変換/ひらがな/カタカナキーのIME意味論を`On`/`Off`/`Toggle`の
  3値へ集約する`classify_thumb_key_ime_actions`/
  `classify_mode_key_ime_action`と、opt-inゲートを適用する
  `gate_thumb_key_ime_actions`（結果は`ThumbKeyImeWiring { henkan,
  muhenkan, warning: ThumbKeyImeWarning }`、BUG-115）もある。
- **MS-IME（シンプルキー割当て）**: `crates/awase-windows/src/
  msime_key_assignment.rs`が`HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME`
  のDWORD値を読む: `IsKeyAssignmentEnabled`（マスタースイッチ）、
  `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`（無変換/変換への割当て、
  実機確認済み: `1`=IME-オフ/オン、`2`=トグル、未設定/`0`=既定）、
  `KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace`（同ADR-092決定D
  Step4a）。

  **（レビューF1で訂正）** 起票時点の本ADRは「これらは警告用途
  （`check_and_warn`）にしか使われておらず、bug reportには渡っていない」
  と書いたが、これは事実誤認だった。`read_toggle_assignment_from_registry`/
  `read_delegate_to_open_axis_assignment_from_registry`の2関数は
  `runtime/message_handlers.rs::sync_ime_toggle_auto_detect`
  （同ファイル`sync_ime_kind_from_observation`から呼ばれる、MS-IME確定の
  たびに実行される「単一の合流点」）経由で**Engineの実挙動を駆動して
  いる**:
  - `read_toggle_assignment_from_registry()` →
    `Engine::set_ime_toggle_auto_keys(toggle_assignment.to_combos(
    skip_shift_space))`
  - `read_delegate_to_open_axis_assignment_from_registry()` →
    `Engine::set_muhenkan_delegate_to_open_axis`/
    `set_henkan_delegate_to_open_axis` + `Runtime::
    set_thumb_key_shadow_overrides`（ADR-141 C2対策、無変換/変換が
    親指キーとして設定されている場合のみ）

  「警告専用」なのは`check_and_warn`が内部で使う**別の**読み取り関数
  （`windows_impl::read_from_registry`、`MsImeKeyAssignment`型、
  現状private）だけである。この事実誤認は、GJI側の決定1で行った
  「フィルタ前の生値と、実際にawaseが採用した値の両方を送るべきか」
  という論点が、**MS-IME側にも同じ形で存在する**ことを見落とす原因に
  なっていた（後述「決定2」で対応）。

### Web調査で確認した事実（新IME/旧IMEの正確な関係）

ユーザーの要望は「新IME」「旧IME」という2つの独立した設定系統がある
という前提だったが、調査の結果これは不正確で、正しくは**同じ変換
エンジン（v15.0）の上に2種類の設定UIが載っている**という関係だった。

- Windows 10 バージョン2004（2020年）以降、Text Service Framework (TSF)
  の実装が大きく変更され、Microsoft IMEの設定UIも「時刻と言語 → 地域と
  設定 → Microsoft IME → キーとタッチのカスタマイズ」という新UIに
  簡略化された（[atmarkit記事](https://atmarkit.itmedia.co.jp/ait/articles/2202/17/news021.html)、
  [jpwinsup公式ブログ](https://jpwinsup.github.io/blog/2021/10/03/UserInterfaceAndApps/LanguageSupport_IME/previous_ime/)）。
  この新UIで設定できるのは**無変換キー・変換キー・Ctrl+Space・
  Shift+Spaceの4箇所のみ**で、割当て可能な機能もIME-On/Off/トグル等の
  限られた選択肢だけ（[relief.jp記事](https://www.relief.jp/docs/windows11-ime-keybinding.html)）。
  この設定は上記の`MSIME`直下のDWORD値として保存される
  （2026-07-06・2026-08-15実機確認済み、ADR-092）。
- 一方、レガシーなIMM32アプリとの互換性のため「互換性 → 以前の
  バージョンのMicrosoft IMEを使う」というトグルが用意されており、
  これをONにすると`IMJPUEX.EXE`（詳細設定）/`imjpuexc.exe`
  （コマンドライン設定ツール、`C:\Windows\System32\IME\IMEJP\`配下）
  経由の**旧来の全キー・全モードのカスタムキーマップ編集UI**
  （「ユーザー定義」）にアクセスできるようになる。これは
  `HKEY_CURRENT_USER\Software\Microsoft\IME\15.0\IMEJP\StyleList\Custom`
  にバイナリ値として保存される
  （[note.com記事1](https://note.com/optim/n/n2597ade5dd6e)、
  [マイナビ「窓辺の小石(215)」](https://news.mynavi.jp/article/pebble_in_the_window-215/)）。
  マイナビ記事は「レジストリなどを見てみると、バージョンとしては15.0の
  ままで、変換エンジンなどは同じで、GUIなど見た目だけが変更されている」
  と明記しており、**「新IME」「旧IME」は別製品ではなく同じMS-IMEのUI
  互換モードの違い**であることを裏付けている。

  **（レビューF13で指摘）このレジストリパス（`StyleList\Custom`）は
  今回のWeb調査（複数のブログ記事）由来であり、awase開発者自身の実機で
  確認したものではない。** 上記`MSIME`直下のDWORD値がADR-092で実機diff
  により確定済みであるのとは信頼性のレベルが異なる。この非対称性を
  踏まえ、Phase 2着手前に実機確認を前提条件とする（後述）。

  したがって本ADRでは「新IME/旧IME」という製品区分の代わりに、
  **機能区分**で呼ぶ:
  - **シンプルキー割当て**（`MSIME`直下のDWORD値、新UIから設定される
    4キーのみの割当て。実機確認済み）
  - **詳細キーカスタマイズ**（`StyleList\Custom`のバイナリblob、旧UI
    互換モードでのみ編集可能な全キー・全モードの「ユーザー定義」。
    パス自体が未確認）

  両者は互いに独立したレジストリ値であり、どちらのUIを使っている
  ユーザーでも両方の値が(値が設定されていれば)同時にレジストリ上に
  存在しうる。awase側で「ユーザーが新UI/旧UIのどちらを使っているか」を
  確定判定する必要はなく、両方の値を読めるだけ読めばよい。

- **`StyleList\Custom`のバイナリフォーマットは未解読**。Web調査で
  確認できたのは「バイナリ形式だが、シフトJIS文字列をバイト配列に
  したもの」という粒度の情報のみで、レコード長・キーコード表現・
  状態(モード)数・機能ID表など、意味のある要約を組み立てるために
  必要なフィールドレイアウトを示す一次資料は見つからなかった。GJIの
  `config1.db`はGoogle非公開だがOSS実装（Mozc）がありfield番号を
  そこから確認できた（`crates/awase-gji-config/src/lib.rs`のdoc参照）
  のに対し、MS-IMEはクローズドソースで参照実装が存在しないため、
  同水準の解読には実機での意図的な設定変更→レジストリexport diff
  という総当たり調査が別途必要になる。

## 決定

### 実装スコープの訂正（レビューF12対応）

起票時点の本ADRは「既存の解析結果をbug reportへ配線するだけ」と
書いたが、これは半分だけ正しい。GJI側の`sync_gji_charset_autodetect`は
**「GJI継続区間で1回だけ」**`config1.db`を読む設計になっており
（`LAST_GJI_STREAK_CHECKED`ラッチ、継続的ポーリングをしないため）、
かつその読み取り結果（`GjiRawConfig`）はRuntimeにキャッシュされて
いない。つまり不具合報告時点でawase本体が「最後に採用した」値を
そのまま参照する経路は存在しない。

本ADRが採るのは、**bug report生成時に`config1.db`/レジストリを独立に
再読み込みする**方式である（既存の読み取り関数を新しい呼び出し元から
再度呼ぶだけで、Runtime側に新規の状態を追加する必要はない——この点は
「配線だけ」という当初の見積もりのうち保てる部分）。この場合、
報告に載る値は「報告した瞬間のファイル/レジストリの中身」であり、
「その報告が問題になった瞬間にEngineが実際に採用していた値」とは
理論上ズレうる（`config1.db`をセッション中に書き換えた場合など）。
このズレは実運用上まれで、既存の`config_toml`/`layout_yab`添付
（`crate::app::read_bug_report_attachments`、これも報告時点のファイル
内容を都度読み直す設計）と同じ性質の限界であるため許容する。ADR本文
にこの限界を明記する（決定1・決定2の型のdocコメントに反映）。

MS-IME側（`msime_key_assignment.rs`の3関数）はレジストリの都度読み
出しであり、GJIのようなセッションラッチが無いため、この限界は
存在しない（都度読みがそのまま最新の実効値と一致する）。

### Phase 1（本ADRのスコープ、実装対象）

不具合報告ペイロードに、IME種別ごとの構造化されたキーマップ要約を
追加する。**生のレジストリバイナリ値・`config1.db`の生バイト列・
`custom_keymap_table`の生TSV全文は送らない**。

**（レビューF6で訂正）** ADR-095決定4は「journal生データはマスキング
せず既定ONで送り、安全弁は送信前プレビューでの手動編集に一本化する」
という設計であり、本ADRが生バイナリを送らない判断はこれと対立する
新方針ではなく、**同じ安全弁のメカニズムに乗れるかどうか**の帰結
である: プレビューUIはJSON全文をテキストとして表示・編集できることが
前提であり、GJIの構造化データ（VK名の配列や`session_keymap`の整数値）
はテキストとして意味が読めて不要なら削れるのに対し、`StyleList\Custom`
の生バイナリ（base64/hex化しても）は中身が読めず「削るべきかどうか」
の判断自体をユーザーに委ねられない。つまりADR-095の安全弁が構造的に
機能しない対象だから送らない、という理由づけに統一する。

**（レビューF7で追記）** GJI側の構造化データが安全な理由は「構造化
だから」ではなく、`crate::keymap::mozc_key_to_vk_name`
（`crates/awase-gji-config/src/keymap.rs:75-89`）が固定エイリアス表と
`F1`-`F24`以外を`None`に落とす**allowlistが唯一の防壁**であるという
事実に依存している。同関数は未対応トークンを`tracing::warn!(
"gji-config: 未対応のキートークンをスキップしました: key={key}")`で
ログに出しており、**将来「診断のためスキップしたトークンもpayloadへ
含めよう」という一見自然な改善を加えると、この防壁を素通りして
`config1.db`由来の任意文字列を送信する経路に静かに変質する**。実装時
・将来の変更時にこの前提を壊さないよう、`BugReportGjiKeymapSummary`の
型doc自体にこの依存関係を明記する。MS-IME側（DWORD由来のbool/enum）は
そもそも自由文字列が混入する経路が無いため、この懸念は適用されない。

1. **`BugReportGjiKeymapSummary`型を新設**（`crates/awase-windows/src/
   bug_report.rs`）。`awase_gji_config::wire::GjiRawConfig`と
   `read_gji_ime_keys`/`read_gji_mode_keys`、および
   `gji_charset_autodetect.rs`の`classify_thumb_key_ime_actions`/
   `gate_thumb_key_ime_actions`の出力から構築する:

   - `config1_db_status: "NotFound" | "ParseFailed" | "Ok"`
     （`gji_charset_autodetect.rs`の`bytes_read_ok`と同じ2値を診断に
     残す。`Ok`のときのみ以下のフィールドが意味を持つ）
   - `session_keymap: Option<i64>`（`SESSION_KEYMAP_CUSTOM`等の生値）
   - `has_henkan_muhenkan_overlay: bool`
     （`overlay_keymaps`に`SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`
     を含むか）
   - **`custom_keymap_table_present: bool`**（レビューG2対応、新規
     フィールド）: `custom_keymap_table`（field 42）そのものが
     存在するか（`session_keymap`の値は問わない）。
   - **`custom_keymap_table_is_effective: bool`**（レビューF11対応、
     新規フィールド）: `session_keymap == Some(SESSION_KEYMAP_CUSTOM)`
     **かつ**`custom_keymap_table_present`が`true`のときのみ`true`
     （**実装コードレビューで訂正**: 当初`session_keymap`の条件のみで
     計算していたが、`gji_charset_autodetect.rs`の実際のガードは
     `session_keymap == CUSTOM`のearly returnの**さらに後**に
     `let Some(table) = raw.custom_keymap_table else { return }`という
     2段目のガードを持つ。前者だけを再現すると、CUSTOM選択中だが
     field 42が不在の環境で本フィールドが誤って`true`になる）。
     `awase_gji_config::read_gji_ime_keys`/
     `read_gji_mode_keys`自体は`session_keymap`を一切見ず
     `custom_keymap_table`（field 42）があれば無条件に解析するが、
     awase本体（`gji_charset_autodetect.rs:789-805`）は「session_keymap
     がCUSTOMでなければ、custom_keymap_tableに何が残っていてもGJIは
     それを参照しない」という必須ガードを持っている（過去のOpus
     レビューで一度確定した判断）。このガードをbug report側で
     再現しないと、「ATOKプリセットに切り替える前の古いカスタム
     テーブルの残骸」を「現在有効な設定」であるかのように報告して
     しまい、過去に一度潰した誤読をpayloadの形で復活させることに
     なる。**（レビューG2で追記）** ただし`custom_keymap_table_present`
     が`true`かつ本フィールドが`false`という組み合わせ自体が
     「以前CUSTOMだった残骸が残っている」という診断上有用な手がかり
     になるため、`present`は`effective`の値に関わらず常に載せる
     （中身〈`ime_*_keys`等〉は`effective`が`false`の間は載せない）。
   - `ime_on_keys: Option<Vec<String>>` / `ime_off_keys:
     Option<Vec<String>>` / `ime_toggle_keys: Option<Vec<String>>`
     （**レビューG6で`Vec`から`Option<Vec>`に変更**:
     `custom_keymap_table_is_effective`が`false`のときは`None`
     （「非該当」）、`true`のときは`Some(vec![...])`（空配列もありうる
     ＝「該当キーなし」）として、両者を区別できるようにする。値は
     `GjiImeKeys`のVK名。**レビューF4で訂正**: 起票時点の本ADRは
     これらを「安全範囲フィルタ適用前の生の抽出結果」と呼んでいたが、
     `extract_ime_keys`（`crates/awase-gji-config/src/keymap.rs`）は
     既に(1)修飾キー付き行〈`key`に空白を含む行、l.243-249〉
     (2)`mozc_key_to_vk_name`のallowlist外トークン〈l.138-141〉
     (3)状態間で矛盾する割当て〈`classify_and_push`、l.285-289〉の3種を
     除外済みであり、「生」ではない。本ADRでは正しく「stage 1の
     スコープ内に絞られた抽出結果（`awase-gji-config`crateのdoc参照）」
     と呼ぶ）
   - `mode_set_keys: Option<Vec<(String, String)>>`（VK名と
     `GjiCompositionMode`の文字列表現。**レビューF17対応**: `Debug`
     表記に依存せず、`GjiCompositionMode`の全バリアントを網羅した
     ローカルなmatch式（`message_handlers.rs::gji_composition_mode_str`）
     で文字列化する——`Debug`は外部crateのバリアント名変更で静かに
     壊れるが、網羅的match式なら新バリアント追加時にコンパイルエラーで
     気づける。**（実装レビューR-5で確認）** F17は当初
     `serde::Serialize`の追加を想定していたが、実装ではこの網羅的match
     方式を採用した——コンパイラが変更を強制する点で狙いは同じであり、
     `GjiCompositionMode`自体への変更（クレート横断の依存追加）が
     不要になる分、こちらの方が筋が良い。したがってG4が言及した
     `GjiCompositionMode`の可視性調整も不要だった（既に`pub`）。）/
     `mode_toggle_alphanumeric_keys: Option<Vec<String>>` /
     `mode_toggle_kana_type_keys: Option<Vec<String>>`（`GjiModeKeys`
     から、同じく`custom_keymap_table_is_effective`との対応で
     `Option<Vec>`にする）
   - **`henkan_classified_kind: Option<String>` / `muhenkan_classified_kind:
     Option<String>`**（レビューF10対応、新規フィールド。以下「分類系」
     と呼ぶ）: `classify_thumb_key_ime_actions`が返す`ImeToggleKind`
     （`On`/`Off`/`Toggle`）の文字列表現。`config1.db`の内容だけから
     決まる純粋計算のため、`custom_keymap_table_is_effective`にも
     `ime_kind`にも関わらず常に計算する——`classify_thumb_key_ime_actions`
     はoverlay/CUSTOM/ATOKプリセットの3系統を優先順位付きで1つの結論に
     まとめる関数であり、ATOKプリセット（`custom_keymap_table`無し）の
     ユーザーでも`Some(Toggle)`等を返しうる。上記`ime_*_keys`だけでは
     ATOKプリセットユーザーの無変換/変換キーの実際の意味論
     （BUG-115の中心そのもの）が復元できないという指摘に対応する。
   - **`henkan_adopted_kind: Option<String>` / `muhenkan_adopted_kind:
     Option<String>` / `henkan_adopted_route: Option<"Delegate" |
     "ActuationAuto">` / `muhenkan_adopted_route: Option<...>` /
     `thumb_key_ime_warning: Option<"ToggleDeclined" | "ToggleHonored">`**
     （レビューF10・G1・G3対応、新規フィールド。以下「採用系」と呼ぶ）:
     `gate_thumb_key_ime_actions`が返す`ThumbKeyImeWiring{henkan,
     muhenkan, warning}`と、`route_thumb_key_action`
     （`gji_charset_autodetect.rs:522-550`、親指キーとして設定されて
     いれば`delegate-to-open-axis`、そうでなければ
     `ime_on_auto`/`off_auto`/`toggle_auto`〈actuation-auto〉へ振り分ける）
     の行き先の両方を反映する。

     **（レビューG1で追記、Must-fix）** 「分類系」は`config1.db`の
     内容を解釈するだけの純粋計算なので`ime_kind`に関わらず常に載せて
     よいが、「採用系」は**`ime_kind == Gji`のときのみ**`Some`にする
     （それ以外は全部`None`）。理由: `gji_charset_autodetect.rs:
     641-676`のとおり、GJIから離脱すると（`is_gji == false`）awaseは
     GJI由来の値を`clear_gji_ime_on_off_auto_keys`/
     `set_gji_thumb_key_delegate_to_open_axis(None, None)`/
     `set_thumb_key_shadow_overrides(None, None)`で**全部解除する**。
     ここを`ime_kind`でゲートしないと、MS-IMEユーザーの報告に
     「GJI（ATOKプリセット等）由来の`Toggle`/`ToggleDeclined`」が
     載ってしまい、実際には解除済みの設定をトリアージ側が「これが
     原因」と誤診する——決定2でMS-IME側だけ気をつけていた「生値と
     採用値の混同」が、`ime_kind`ゲート撤回（決定3、F8/F9）によって
     GJI側に逆流していた欠陥。`thumb_key_ime_warning`も同じ理由で
     `ime_kind == Gji`ゲートの対象に含める（`None`/`ToggleDeclined`/
     `ToggleHonored`の3値のうち、ゲート対象外なら`None`＝フィールド
     自体が`None`、ゲート対象内で警告不要なら`Some("None"に相当する
     値なし")`ではなく素直にフィールド自体を出さない設計とする——
     実装時に`Option<String>`のバリアント名を`ToggleDeclined`/
     `ToggleHonored`の2値のみに絞ることで表現する）。

   `custom_keymap_table_present`/`custom_keymap_table_is_effective`/
   「分類系」（`henkan_classified_kind`等）は`ime_kind`に関わらず常に
   計算してよい（config1.dbが読めれば意味を持つ、Engineの現在の
   IME種別とは独立な情報のため）。

   **`muhenkan_dedicated_fn_key_configured: bool`**（レビューH1対応、
   新規フィールド、`Runtime::muhenkan_dedicated_fn_key_configured()`
   から取得。`ime_kind`非依存で常時計算する——config.tomlの設定値で
   あり、GJI/MS-IMEどちらの`muhenkan_adopted_route`にも同じ優先順位で
   効く共通情報のため）: `muhenkan_adopted_route == Some("Delegate")`
   であっても、これが`true`の場合は
   `resolve_pending_thumb_as_single`の優先順位（専用Fnキー >
   delegate）により**その無変換delegateは実際には発火しない**
   （`gji_charset_autodetect.rs:477-486`の
   `delegate_owns_mode_key_shadow_toggle`ガード、および同873-895行の
   `warn_thumb_key_toggle_if_needed`が出す警告「無変換キーのIME
   open軸への追従は無効化されます（変換キー側のみ有効）」を参照）。
   この情報が無いと、`muhenkan_adopted_route: "Delegate"`だけを見て
   「無変換delegateが誤発火している」という実際には起きていない仮説を
   トリアージ側が追いかねない。本フィールドはGJI/MS-IME両方の
   summary型に同じ意味で持たせる（変換キー側にはこの専用Fnキー概念が
   存在しないため`henkan_adopted_route`には影響しない）。

   **（レビューG4で訂正）** `GjiCompositionMode`は現状既に`pub`
   （`crates/awase-gji-config/src/lib.rs:33`で`pub use`されている）。
   可視性調整が必要なのは`ImeToggleKind`/`ThumbKeyImeWarning`の
   2つのみ（どちらも`awase-windows`内`pub(crate)`）であり、実装時に
   そのシリアライズ表現を`bug_report.rs`から参照できるよう必要最小限の
   範囲で可視性を調整する。

2. **`BugReportMsImeKeyAssignmentSummary`型を新設**。
   `msime_key_assignment.rs`の読み取り関数の結果を、**生値と採用値の
   両方**で束ねる（レビューF15・F16対応）:

   - **生のDWORD**（レビューF16対応、新規）: `is_key_assignment_enabled:
     Option<u32>` / `key_assignment_muhenkan: Option<u32>` /
     `key_assignment_henkan: Option<u32>` / `key_assignment_ctrl_space:
     Option<u32>` / `key_assignment_shift_space: Option<u32>`。
     起票時点の本ADRは`MsImeKeyAssignment`/`MsImeToggleAssignment`/
     `MsImeDelegateToOpenAxisAssignment`という**解釈済み**の型
     （`== Some(1)`/`== Some(2)`で分岐、それ以外は「宣言なし」に
     潰す）だけを載せる設計だったが、これだと(a)将来値`3`以降が
     追加された場合に「未知の値が設定されている」ことが報告から
     消える、(b)マスタースイッチ（`IsKeyAssignmentEnabled`）がOFFの
     場合、`read_toggle_assignment_from_registry`/`read_delegate_
     to_open_axis_assignment_from_registry`はどちらも即座に既定値
     （空/`None`）を返すため、Ctrl+Space/Shift+Spaceの**実際の
     レジストリ値**（ユーザーが一度設定してからマスタースイッチだけ
     切った、等のケース）が報告から消える。生のDWORDを併載すれば
     どちらも復元可能になる。
   - **採用値**（レビューF15・G1対応、新規）: `adopted_ime_toggle_combos:
     Option<Vec<String>>`（`MsImeToggleAssignment::to_combos(
     skip_shift_space)`の結果を`"Ctrl+Space"`/`"Shift+Space"`のような
     表現に変換したもの。`skip_shift_space = app.space_is_thumb_key()`
     は報告生成時の`Runtime`から取得できる現在値を使う） /
     `adopted_muhenkan_delegate: Option<String>` /
     `adopted_henkan_delegate: Option<String>`（`ShadowImeAction`の
     文字列表現）。

     **（レビューG1で追記、Must-fix。GJI側決定1と同じ理由）**
     これら「採用値」フィールドはすべて**`ime_kind == MsIme`のときのみ**
     計算する（それ以外は常に`None`）。加えて`adopted_muhenkan_delegate`/
     `adopted_henkan_delegate`は`is_configured_thumb_key(VK_NONCONVERT/
     VK_CONVERT)`が`false`の場合も`None`とする——
     `sync_ime_toggle_auto_detect`の実装が親指キーとして設定されて
     いないキーには`delegate`をoverrideへ渡さないのと同じ条件で、
     生のレジストリ値だけを見ると「無変換が親指キーでないのに何か
     割り当てられているように見える」誤読を防ぐ。この2条件（`ime_kind`
     ゲート＋thumb-keyゲート）により、GJI側の「分類系」/「採用系」の
     区別と対称に、「レジストリに何が書いてあるか」（生のDWORD、常時）
     と「awaseが（今アクティブなIMEとして）実際に何を採用したか」
     （採用値、`ime_kind`一致時のみ）の両方が報告から読み取れる。
   - `StyleList\Custom`（詳細キーカスタマイズ）の扱いはPhase 1の
     スコープに**含めない**（後述「Phase 2」参照、レビューF13・F14
     対応でスコープから外した）。

3. 両型とも`ime_kind`に関わらず**常に取得を試みる**（レビューF8・F9
   対応、当初の「`ime_kind`に応じて片方のみ取得」から変更）。

   **（レビューF8で指摘）** `current_bug_report_ime_kind()`が`Unknown`
   を返すのは、IME種別確定イベント（`WM_IME_KIND_CHANGED`）がまだ
   届いていない、またはATOK等の第三者IMEでCLSIDベース判定にヒット
   しない場合である。`sync_gji_charset_autodetect`/`sync_ime_toggle_
   auto_detect`はどちらも`detected`（IME種別確定済み）をゲート条件に
   しているため、**`Unknown`のときはEngine側の配線が「最後に走った
   同期の残骸」のまま**になっている（`gji_charset_autodetect.rs:
   665-674`のコメントが、この残留自体が実バグ〈stale delegateの無期限
   残留〉として記録済みであることを示す）。`ImeToggledUnexpectedly`/
   `ThumbKeyMisbehavior`症状はこの状態でこそ報告される可能性が高く、
   `ime_kind`でゲートすると本ADRの動機（切り分けの往復を減らす）が
   最も必要な場面で機能しない。

   **（レビューF9で指摘）** 却下していた「`ime_kind`と独立に両方読む」
   という代替案は、却下理由（「今使っていないIMEの設定まで送るのは
   ペイロードサイズ的に無駄、診断上の意味もない」）がどちらも成立
   しない: 両summary型のサイズは数百バイト程度で`MAX_BODY_BYTES`
   （512KiB）に対して無視できる。診断上の意味も上記F8のとおりある。
   したがって**この代替案を採用する**よう決定を変更する: `config1.db`
   と MS-IMEレジストリはそれぞれ独立にファイル/レジストリの存在有無
   だけで読み取りを試み（GJI未インストール環境では`config1_db_status:
   "NotFound"`に、MS-IME値が一切無い環境では全フィールド`None`に、
   自然にフォールバックする）、`ime_kind`による**型単位**のフィルタは
   行わない。

   **（レビューG1で追記、Must-fix）** ただし「型単位でのゲート撤回」を
   「型内の全フィールドを無条件にする」と読み違えてはならない。
   決定1・決定2の「採用系」フィールド（GJI側`henkan_adopted_kind`等・
   `thumb_key_ime_warning`、MS-IME側`adopted_*`）は、そのIMEが**現在
   アクティブ（`ime_kind`が一致）**でなければ、実際にはawaseが
   解除・不採用にしている値である。ここまで`ime_kind`非依存にすると、
   「非アクティブなIME側の、既に解除済みの設定」を「現在の設定」で
   あるかのように報告してしまう（決定1・決定2内の該当箇所を参照）。
   `ime_kind`非依存で常時載せてよいのは、config1.db/レジストリの内容を
   解釈するだけの「生値」「分類系」フィールドに限る。

4. 両型とも`BugReportPayload`/`BugReportInput`に
   `attach_ime_keymap: bool`（既定ON、他の`attach_*`と同じ粒度）+
   `gji_keymap: Option<BugReportGjiKeymapSummary>` +
   `msime_key_assignment: Option<BugReportMsImeKeyAssignmentSummary>`
   の形で追加する。

   **（レビューF2で訂正）** 起票時点の本ADRは「`BugReportDiagnostics`/
   `BugReportPayload`に`attach_ime_keymap`を追加する」と書いていたが、
   `BugReportDiagnostics`（`crates/awase-windows/src/bug_report.rs:
   308-324`）は`attach_*`フラグを1つも持たない型であり、これは
   事実誤認だった。既存の`config_toml`/`layout_yab`と同じ配線
   パターンに合わせる: **`attach_*`フラグの置き場所は`BugReportInput`
   （341-369）・`BugReportPayload`（181-210）、および
   `awase-settings`側のUI状態`App`構造体
   （`crates/awase-settings/src/bug_report.rs:25-29, 111-115`）**であり、
   `BugReportDiagnostics`にはデータ（`Option<BugReportGjiKeymapSummary>`/
   `Option<BugReportMsImeKeyAssignmentSummary>`、attachフラグ抜き）
   のみを追加する。

   **（レビューF3対応）** `BugReportDiagnostics`への新フィールド追加は
   `#[serde(default)]`を必須にする——同型のdocコメント自身が警告して
   いるとおり（317-323行目付近）、`awase-settings::bug_report::
   load_diagnostics`は`serde_json::from_str(...).ok().unwrap_or_default()`
   （`crates/awase-settings/src/bug_report.rs:374-382`）で読むため、
   1フィールドでも欠落すると`state_snapshot`を含む診断情報全部が
   静かに消える。加えて`BugReportDiagnostics`の`Default`実装は
   deriveではなく手書き（326-339行目）なので、新フィールド用の
   デフォルト値（`None`）をそちらにも追記する。

   **（レビューH2対応、実装メモ）** 決定1・決定2の「採用系」ゲートは
   `ime_kind`を必要とするが、`current_bug_report_diagnostics(app:
   &Runtime)`（`message_handlers.rs:1241`）自体は`ime_kind`を受け取って
   いない。呼び出し元（同1140-1141行目、`current_bug_report_
   ime_kind()`を呼んでいる箇所）から見て、`current_bug_report_
   ime_kind()`は内部でグローバルなatomicを読むため、diagnostics
   構築のためにもう一度呼ぶと、その間のフォーカス変化で
   `BugReportPayload.ime_kind`（既存フィールド）と、本ADRが追加する
   採用系フィールドのゲート判定に使った値とが食い違う報告が生成され
   うる。実装時は1140-1141行目で`ime_kind`を1回だけ評価し、
   `current_bug_report_diagnostics(app, ime_kind)`のように引数として
   渡すこと。

5. **サーバ側（`services/report-worker/src/index.ts`）**は、ADR-120
   決定0a-report（`retro_eval_stats`追加時）と同じパターンを踏襲する:
   `SCHEMA_VERSION`は**上げない**。新フィールドは`optionalBoolean`/
   `optionalNullableRecord`で読み、整合性チェックを追加する
   （`validatePayload`内、`index.ts:344-427`の既存パターンと同型で
   あることをレビューで確認済み、観点4「該当なし」）。**（レビューG5で
   訂正）** `attach_ime_keymap`は1つのフラグだが紐づくデータは
   `gji_keymap`/`msime_key_assignment`の**2オブジェクト**なので、
   整合性チェックも`gji_keymap_requires_attach_ime_keymap`/
   `msime_key_assignment_requires_attach_ime_keymap`の**2本**必要
   （既存の1フラグ1オブジェクトの`attach_config_requires_...`等とは
   カーディナリティが異なる点に注意）。実装時には`BugReportPayload`
   TypeScript interfaceと`test/index.test.ts`のフィクスチャ更新も伴う
   （レビュー参考意見）。理由はADR-120と同じで、`schema_version`を
   上げると
   `validatePayload`が`unsupported_schema_version`で旧クライアントの
   報告を拒否するようになり（`index.ts:344-345`）、クライアント
   （awase.exe）とサーバ（Cloudflare Worker）の同時デプロイが必須に
   なってしまう。ただし**Workerを再デプロイするまでは新フィールドが
   実際にR2へ保存されない**（`validatePayload`が返すオブジェクトは
   フィールドを明示列挙するallowlist方式であり、`...value`のような
   スプレッドはしていないため）。

6. **UI（`crates/awase-settings/src/bug_report.rs`）**: 既存の
   `draw_attachment_checkboxes`に「IMEキーマップ設定を添付する」
   チェックボックスを1つ追加する（GJI/MS-IME共通の1つのトグルでよい
   ——ユーザーがどちらのIMEかは自動判定済みで、選ぶのは「送るか
   送らないか」だけ）。送信前プレビュー（ADR-095決定4）にJSON全文
   として表示されるため、構造化データである本フィールドも他の
   フィールドと同様プレビューで確認・編集可能になる。決定3の変更
   （`ime_kind`非依存の常時取得）により、GJI/MS-IMEどちらも未検出
   （`Unknown`）の環境でチェックボックスをONにしても大抵は両方
   `None`のまま送られる自然な結果になる（レビューF19、参考意見）。
   ホバーテキスト等の具体的な文言は実装時に決める。

### Phase 2（本ADRでは方針のみ、実装は別途）

`StyleList\Custom`の詳細キーカスタマイズを実際に要約して送るには、
バイナリフォーマットの解読が前提になる。これは以下の理由で本ADRの
実装スコープに含めない:

- **レジストリパス自体が実機未確認**（レビューF13）。前述のとおり、
  今回の一次情報はWeb調査のみであり、awase開発者自身の実機で
  `StyleList\Custom`の存在・値の変化を確認していない。パスや値名が
  異なっていた場合、Phase 1に含めた「存在有無フラグ」は**常に
  `false`を返し続け、それに誰も気づけない**（全報告に「詳細
  カスタマイズ未使用」という一律の誤った結論が付くことになるが、
  検証手段が無いため発覚しない）。したがって「存在有無フラグ」も
  含めてPhase 1のスコープから完全に外し、実機確認後のPhase 2に回す。
- フォーマットが未解読のまま生バイナリを送っても、報告を読む側
  （awase開発者自身）が手動で解析する以外に使い道がなく、実質
  「未解読のバイナリを診断情報と称して集める」だけになる。
- 生バイナリの中身に、キー割当て以外の情報（ユーザー辞書由来の
  文字列等）が混入する可能性を今回のWeb調査だけでは排除できない
  （「シフトJIS文字列のバイト配列」という記述はあるが、その文字列が
  常にキー名/機能名のみであるという保証がない）。

方針: まず実機（awase開発者本人の環境）で「詳細キーカスタマイズ」
UIを開き、`StyleList\Custom`の値の有無・変化を確認する。存在が
実機確認できた段階で、次に**単純な`bool`ではなくバイト長+ハッシュ
（例: SHA-256の先頭数バイトのみ）のフィンガープリント**を送る設計を
検討する（レビューF14対応: 単なる存在有無だけだと「旧UIを開いて
既定のままOKしただけ」と「実際にキーを変更した」を区別できず偽陽性
になる。長さ+ハッシュなら生バイト非送信を維持したまま、既定blobとの
一致判定や複数報告間の設定同一性比較ができる）。フィールドレイアウトの
解読自体は、意図的な単純設定（特定キー1つだけを既定から変更）→
レジストリexport diffという総当たり調査（GJIの`config1.db`解析で
BUG-115時に行った手法と同種）が前提になる。見通しが立った段階で、
本ADRの追補または新規ADRとして実装を設計する。

### Phase 2 実機確認（2026-09-07、追記）

上記「未決事項」のうち、レジストリパス自体の実機未確認だった点を
dragonflyg4（awase開発者本人の実機）の`reg export`で確認した。
バイナリフォーマットについても、実際に1項目だけ変更→export diffを
2回行い、以下が判明した。

- **パスは実在した**: `HKCU\Software\Microsoft\IME\15.0\IMEJP\
  StyleList\Custom`はWeb調査どおりの場所に存在した。さらに`StyleList`
  配下には`Custom`以外に`ATOK`/`MS-IME2000`/`NATURAL`/`VJE`/`WX`の
  合計6プリセットが存在し、どれが有効かは`IMEJP\MSIME\keystyle`
  （文字列値、実測`"NATURAL"`）で決まる。`imjpuexc.exe SETKEYTEMPLATE`
  はこのプリセット切替のみを行うコマンドラインツールで、個々のキー
  割り当ての編集はできない（`Microsoft_IME`/`IME_Standard`/`ATOK`/
  `VJE`/`WX`の5テンプレート名のみを受け付ける）。
- **バイナリの実体はテキストだった**: `hex:`（REG_BINARY）としてexport
  されるが、生バイト列はShift-JISテキストであり、`<キー名>=<コード1>
  <コード2> <コード3> <コード4> <コード5> <コード6>\0`という記録が
  NUL区切りで連なり、リスト末尾はさらに`\0`が1つ多い、という単純な
  形式だった。`<キー名>`は物理キー名（「無変換」「変換」「半角/全角」
  等、`Ctrl+`/`Shift+`/`Alt+`修飾子付き表記もある）で、`ImeOn`/`ImeOff`
  （VK_IME_ON/VK_IME_OFF相当の専用行）も存在する。6個のコードは
  入力モード（直接入力＋IME ON側5モード）ごとに割り当てられた機能を
  表す列だと判明した（下記参照）。各プリセットは`key`（本体テーブル）
  の他に`S1key`〜`SEkey`という補助テーブル、`defRoma`、
  `DisableFunctions`（DWORD）を持つ。
- **「IMEオン/オフ」トグル機能のコードを実測**: `Custom`プリセットの
  「無変換」「変換」それぶれに、旧UI（`IMJPUEX.EXE`）の「ユーザー定義」
  タブから「IMEオン/オフ」（トグル。**単体のIME ON/単体のIME OFFという
  選択肢はこの機能一覧には存在しない**）を割り当てたところ、2回とも
  同一パターンが再現した:

  ```
  無変換=CE CD CD CD CD CD
  変換 =CE CD CD CD CD CD
  ```

  1列目（直接入力＝IME OFF状態）だけ`CE`、残り5列（IME ON側の各モード）
  は`CD`という非対称なパターンから、`CE`=「IMEをONにする」方向、
  `CD`=「IMEをOFFにする」方向のコードだと推測した。なお「ユーザー定義」
  タブを開いて保存すると、`keystyle`が自動的に`"Custom"`に切り替わる
  （明示的に選択しなくても発生する副作用）。
- **重複行と非対称な実効挙動（Must-know）**: 「変換」に割り当てた回で、
  同じキー名`変換=`の行がテーブル内に**2箇所**（値が食い違う状態で）
  存在することを発見した。1箇所目は上記の`CE CD CD CD CD CD`、2箇所目
  （`Ctrl+変換`の直前に新規挿入）は`CE 00 00 00 00 00`（2〜6列目が`00`）。
  実機でMS-IMEをアクティブにして検証したところ、**直接入力中に「変換」
  を単独で押すとIME ONになった（CE側は効いている）が、IME ON中
  （composition）に押してもIME OFFにはならず、ネイティブの変換機能の
  ままだった（CD側は効いていない）**。つまり2箇所目（2〜6列目が`00`）
  の方が実際には優先され、`00`は「上書きなし・ネイティブ機能を使う」
  という意味のセンチネル値である可能性が高い。この非対称性はawaseの
  危険度評価にとって重要: **旧UIでの「IMEオン/オフ」割り当てが実際に
  危険なのは「IME OFF中に押すと予期せずIMEがONになる」方向だけ**で、
  新UI（`MSIME`直下のDWORD、`msime_key_assignment.rs`が検出する）の
  `KeyAssignmentHenkan`/`KeyAssignmentMuhenkan`とは危険性の性質が
  異なる（新UI側は割り当てた方向がそのまま両方向に効く）。
- **未解決のまま残った点**（次にこの調査を継続する場合の前提）:
  - 重複行の先勝ち/後勝ちの一般規則（今回は「後から挿入された行が勝つ」
    ように見えたが、1事例のみで確認した推測）。
  - 「無変換」側で同じ非対称性（ON方向のみ有効）が起きるかは未検証
    （実機テストは「変換」のみで実施）。
  - `00`が本当に「センチネル（無効/未割り当て）」を意味するかの直接
    確認（他の未割り当てキー行の値と比較する等）。
  - `S1key`〜`SEkey`補助テーブルの役割・本体`key`との重ね合わせ規則。
  - 「IMEオン/オフ」以外の機能（カタカナ/ひらがな切替等）のコード値。
  - 実験で書き込んだ`StyleList\Custom`内の`無変換`/`変換`の割り当て
    自体はexport diff後も**レジストリ上に残っている**（`keystyle`を
    `"NATURAL"`に戻したことで参照されなくなっただけ）。将来
    `StyleList\Custom`を読む実装を書く際、この実験由来の残骸を
    「ユーザーが実際に設定したもの」と誤読しないよう注意が必要
    （このADR自身がその実例を残してしまっている）。

### Phase 2 実装（2026-09-07、追記2）

上記の実機確認結果を踏まえ、`crates/awase-windows/src/
msime_legacy_keymap.rs`として実装した。**当初方針（バイト長+ハッシュの
フィンガープリント）ではなく、実測で判明したコード意味に基づく直接検出**
に変更した——実機確認で「1列目`CE`のみが実際に効く」ことが分かったため、
フィンガープリント（変更の有無しか分からない）よりも「無変換/変換キー
（修飾子なし）の1列目が`CE`か」という具体的な判定の方が、フォーマット
未解読の他の懸念（キー割当て以外の情報混入等）を回避しつつ実用的な
情報を送れると判断した。検出範囲は実機で確認できた部分のみに厳密に
限定する（modifier付きキー・`S1key`〜`SEkey`・他の機能コードは対象外、
モジュールdoc参照）。

`BugReportLegacyMsImeKeymapSummary`（`active_style`/
`muhenkan_ime_on_toggle`/`henkan_ime_on_toggle`）として既存の
`attach_ime_keymap`フラグに相乗りする形でbug report payloadに追加、
`services/report-worker`のバリデーションスキーマ・テストも合わせて
更新した。ユニットテストは2026-09-07実機で実際に生成されたバイト列
（`無変換=CE CD CD CD CD CD`等）をそのまま使用し、重複行・修飾子付き
キーの除外・壊れたレコードの読み飛ばしを回帰させている。

## 却下した代替案

- **生のレジストリバイナリ/`config1.db`生バイト列をそのまま添付する**:
  GJI側はすでに構造化された解析結果（`GjiImeKeys`/`GjiModeKeys`/
  `ThumbKeyImeWiring`）が存在するため生データを送る理由がない。
  MS-IME側の`StyleList\Custom`は前述の理由（フォーマット未解読・
  パス自体未確認・機微情報混入の可能性）でPhase 2に送った。
- **サーバ側（Cloudflare Worker）でconfig1.db/レジストリバイナリを
  解析する**: クライアント側（Rust）に`awase-gji-config`という既存の
  解析クレートがあり、TypeScript側で二重実装する理由がない。加えて
  ADR-095の設計（Workerはstorage-onlyで解析ロジックを持たない）とも
  整合しない。

## 未決事項・今後の課題

- `StyleList\Custom`のレジストリパス・バイナリレイアウトの実機確認
  （Phase 2の前提条件、レビューF13）。
- 「以前のバージョンのMicrosoft IMEを使う」トグルに対応するレジストリ
  値は未特定のまま。ただし本ADRの設計はこのトグルの値そのものには
  依存しない（シンプルキー割当て・詳細キーカスタマイズのどちらの
  レジストリ値も、UIモードに関わらず独立に読めるため）。
- `ImeToggleKind`/`ThumbKeyImeWarning`/`GjiCompositionMode`の
  可視性調整（現状`pub(crate)`、`bug_report.rs`から参照可能にする
  実装時の具体的な変更範囲は実装着手時に確定する）。
- `docs/adr/index.md`のADR-141行の欠落は、本ADRの追加作業の一環として
  修正するが、内容自体は本ADRの対象外。

## 関連

- [ADR-095](095-tray-bug-report-cloudflare-intake.md) — 不具合報告機能
  そのものの設計（プライバシー方針・送信前プレビュー必須・allowlist方式）。
- [ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md) —
  MS-IMEキー割当てレジストリ値の実機確認（`KeyAssignmentMuhenkan`等）、
  `sync_ime_toggle_auto_detect`の設計根拠。
- [ADR-135](135-generic-thumb-key-ime-toggle-delegate.md)（BUG-115） —
  GJIの`config1.db`から無変換/変換/ひらがな/カタカナキーのIME意味論を
  読み取るロジック（`classify_thumb_key_ime_actions`/
  `gate_thumb_key_ime_actions`）の設計・実機確認。
- [ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md) —
  `set_thumb_key_shadow_overrides`とdelegate-to-open-axisの関係
  （C2対策）。
- [experiment-logging](../../.claude/rules/experiment-logging.md) /
  [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) —
  Phase 2着手時、実機調査の記録方法はこの2ルールに従う。
