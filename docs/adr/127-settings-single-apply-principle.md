# ADR-127: 設定画面（awase-settings）の「単一の適用」原則統一——配列編集タブの反映漏れ解消

## ステータス

**実装完了（v6設計をCodexに実装委譲後、Opusによるコード差分の敵対的レビューを
2周実施し収束。`/code-review`指摘4件も修正。PR #157でdevelopへマージ済み）。**
（本ADRはマージ時に既存の別ADR-126〈`126-caps-as-extra-ctrl-preset.md`〉との
番号衝突が判明したため、ADR-127へ改番した。）
Opus 2体architect/premortemによるround1〜round6の設計レビューで「問題なし、
収束」に至った後、実装差分に対する別のOpusエージェントによる敵対的コード
レビューで2件のblocker（下記D1・D2の追記を参照）と複数の非blocker指摘が
見つかり、すべて修正済み。
round1でD1（`changed()`トリガー）・D2（保存先ダイアログ案）の根本的な欠陥が見つかり
再設計した。round2ではその再設計（`lost_focus()`トリガー）自体が、egui のパネル
描画順・タブ切替・ショートカット処理のタイミングに依存する新しい「適用しても反映
されない」経路を生むことが両者独立に判明し、未収束のまま終わった。v2はコミット
処理を「どのウィジェットがいつフォーカスを失うか」から完全に切り離す設計（`update()`
冒頭での無条件コミット判定）に変更しround2の指摘は解消したが、**round3で
「バッファの中身＝ユーザーの意図した値」という新たな暗黙の前提が崩れる2つの経路
（IME変換の未確定文字列、`ValueKind::Special`の未操作インデックス）と、ステータス
表示の毎フレーム上書き、`self.layout`総入れ替え時のバッファ再同期漏れが両者独立に
指摘され、これも未収束のまま終わった。** 本版はコミット判定を「モデルとの単純比較」
から「セル選択時点のスナップショット（`layout_edit_origin`）との差分＋IME合成中で
ないこと」に変更し、`ValueKind::Special`をコミット判定の対象から外して独立の即時
確定経路に分離した、round3の指摘を反映した第3改訂版。**round3レビューの完全な
再現手順を受けて、キーボードモデル不一致ガード（D2）の発火条件から
`layout_modified`要件も撤回した**（`YabLayout::parse`の列数超過エラーにより、
配列を一切編集せずキーボード配列だけ切り替えて適用する操作でも確実にエンジンが
壊れることが判明したため）。v3をround4レビューにかけたところ、**この実装の
細部（`layout_edit_origin`の型、ガードの判定順序）に3件のblockerが両者独立に
（うち2件は完全に同一の欠陥として）見つかった**——(1)`layout_edit_last_seen`の
更新タイミングによりIME確定した編集が消えうる、(2)`layout_edit_origin`が文字列
のみだったため種別だけの変更（round2 B3）が回帰していた、(3)その修正
（originにkindを含める）だけではADR-115打鍵列セルの保護が種別ラジオ経由で
崩れる、という3点で、(2)と(3)は互いにトレードオフの関係にあり両立には
追加の構造的ガードが必要だった。**さらに、`layout_modified`要件撤回自体にも
両者独立に新しいblockerが見つかった**——`config.toml.sample`が案内する
「`keyboard_model`と`default_layout`を全般設定タブで同時に正しく変更する」
という公式の操作手順が、撤回後のガードでは（配列編集タブを一度でも開いた
という、変更内容と無関係な操作履歴次第で）理由不明のまま止められてしまう
経路が見つかった（W8/R4-5）。v5はガードの判定順序を組み替え、
`layout_edit_origin`をタプル化した上でADR-115セル専用の独立ガード
（`layout_edit_origin_is_sequence`）を追加して両立させ、あわせてキーボード
モデル不一致ガードを「データ保護（`layout_modified`必須）」と「エンジン
健全性検証（`default_layout`の実ファイルを直接検証、操作履歴に非依存）」の
2つに分離した。**architectはv5をもって「問題なし、収束」と判定した
（round1のF1〜F22からround4のW1〜W8まで全て解消）。一方premortemは
round5で、このエンジン健全性ガード(B)を無条件（発火＝常に中止）にした
こと自体が新しい退行を生むことを発見した（R5-1）**——キーボード配列を
一切変更していない適用でも、`default_layout`が（無関係な理由で）既に
壊れているだけで設定画面全体が保存不能になり、ADR-116の起動時診断の
意図と衝突する。v6はガード(B)の中止/警告の分岐を「この適用で
キーボード配列が実際に変わるかどうか」で条件分けし、あわせて
architectの非blocker指摘（X1〜X5・Y1・Y2）も反映した第6改訂版。
**この修正をもってround6でpremortemも「問題なし、収束」と判定し、
両者の収束が揃った。** 実装時に拾うべき非blocker事項（ADR-115セルの
「解除」ボタン到達不能・ステータス上書きタイミング・`config_loaded_model`の
「成功時のみ」更新など）は各決定の本文中に記録済み。

## 背景

### ユーザー報告

配列編集画面（`crates/awase-settings/src/main.rs::tab_layout`）に「適用」ボタンが2つ、
「保存」ボタンがあり、どれを押すと編集した設定が実際に保存・反映されるのか分かりにくい、
という報告があった。

### 調査で判明した現状の構造

配列編集タブには3段階の操作があり、それぞれ全く異なる範囲に効く:

| ボタン | 場所（`main.rs`） | トリガー関数 | 実際の効果 |
|---|---|---|---|
| 適用（編集パネル最下部） | `draw_layout_edit_panel`、2338〜2349行目 | `apply_layout_edit()`（896行目） | 入力欄の生テキストをパース・バリデーションし、成功すれば選択中セルへ`self.layout`（メモリ上）だけ反映。`layout_modified = true`。**ディスク書き込みなし** |
| 保存（ツールバー） | 1950〜1956行目 | `layout_do_save()`（947行目）→`layout_write_to_path()`（955行目） | `self.layout`を`layout_file_path`へ`std::fs::write`で同期的に上書き保存。`layout_modified = false`。**config.toml保存・エンジンへの通知なし** |
| 適用（画面最下部、全タブ共通） | `action_panel`、2615〜2624行目 | `apply()`（585行目）→`apply_confirmed()`（460行目） | `self.config`を検証しバックグラウンドスレッドで`config.toml`へ保存、成功後`send_reload_config_message()`で稼働中`awase.exe`へリロード通知。**`self.layout`/`layout_modified`には一切触れない** |

`cancel()`（596〜616行目）も同様に`self.config`を`config.toml`から読み直すだけで、
`self.layout`/`layout_modified`には触れない。

結果として:
- 画面下部の「適用」を押しても、配列編集タブで**保存し忘れた**編集内容はエンジンに
  一切反映されない（ツールバーの「保存」を先に押す必要があるが、そのことはUI上どこにも
  明示されていない）
- 画面下部の「キャンセル」を押しても、配列編集タブの**未保存の編集は破棄されない**
  （「キャンセル＝すべて元に戻る」というユーザーの期待と矛盾する）

### 全タブ監査（Explore agent による調査、コード変更なし）

`crates/awase-settings/src/main.rs`の全タブを対象に、以下4点の原則に照らして監査した:

1. 画面全体で「変更を反映する」操作はただ1つであるべき
2. フィールド/セル単位の中間コミットボタン（値を変えても別ボタンを押すまでモデルに
   反映されない設計）は排除し、直接操作で即座にモデルへ反映すべき
3. 同一ラベルのボタンが異なる範囲・効果を持つ状態は禁止（Nielsenの一貫性原則）
4. ユーザーに実装上のファイル分離（`config.toml`と`.yab`が別ファイルであること等）を
   意識させるべきではない

結果:

| タブ | 状態 |
|---|---|
| 全般設定（`tab_basic`） | `self.config.general.*`に直接バインド。原則に一致 |
| キー設定（`tab_keys`） | `self.config.general.*`/`self.config.keys.*`に直接バインド。「+追加」「x（削除）」はリストへの即時反映で、中間コミットではない。原則に一致 |
| **配列編集（`tab_layout`）** | **原則1・3・4に違反（上記「現状の構造」参照）** |
| 上級者向け設定（`tab_advanced`） | キー設定と同型。原則に一致 |
| アプリ無効化（`tab_disable_apps`） | `self.config.app_overrides.disable_apps`に直接バインド。原則に一致 |
| ショートカット（`tab_keymap`）主要部分 | `self.config.keymaps`に直接バインド。原則に一致 |
| ショートカット内 **Scancode Map セクション**（`scancode_map_section`、1694〜1753行目） | 「有効にする」/「無効にする」ボタンが`apply_scancode_map_change()`（1759行目）→ UAC昇格フローを即座に起動し、Windowsレジストリを直接書き換える。`self.config`を経由せず、下部の適用/キャンセルとは独立。**原則1には抵触するが、ADR-111決定4/7で「操作直後に反映・状態再読み込み」が意図的に確定済み**（UAC昇格・再起動必須という性質上、遅延コミットに構造的に馴染まない） |
| アプリ別オーバーライド（`tab_app_rules`） | UIのタブ一覧から到達不可（意図的、config.toml手動編集に委ねる設計）。評価対象外 |

**結論: 原則違反は配列編集タブに限定される。** Scancode Map セクションは原則1には
形式上抵触するが、既存ADR（ADR-111）で正当な理由により意図的に例外化されており、
本ADRで変更しない。ラベルも「有効にする」/「無効にする」であり「適用」と衝突しない
ため原則3には抵触しない。

## round1レビューで判明した設計上の欠陥（当初案は不採用）

Opus 2体（architect/premortem）に、当初案（旧D1: `changed()`トリガーのライブコミット、
旧D2: `layout_file_path==None`時に保存先ダイアログを同期的に開く）を敵対的にレビュー
させた。両者が独立に、かつ相互補強する形で以下を検出した。ここに記録するのは
「決定を変えた理由」であり、実装の詳細は下記「決定」節を参照。

### 旧D1（`changed()`トリガー）を却下した理由

- **egui 0.31.1のIME実装を実際に確認したところ、`changed()`はIME変換の未確定文字列
  でも発火する**（`ImeEvent::Preedit`受信時にバッファへ実挿入し`any_change=true`を
  立てる、`egui-0.31.1/src/widgets/text_edit/builder.rs:1063-1069,1100`）。日本語の
  変換を`Esc`で中止する、あるいは`Ctrl+A`→打ち直しの一瞬に空文字列が確定入力として
  届く（同ファイル、"Empty prediction can be produced when user press backspace or
  escape during IME"というコメントあり）。旧D1のまま実装すると、**IME変換を中止する
  という日常的操作だけで、選択中セルの既存の値が`YabValue::None`で無言に消える**
  （premortem P2-1）。
- **`ValueKind::Special`（特殊キー、ComboBoxのみ）と`ValueKind::None`（ラベルのみ）
  には自由入力欄が存在せず、`changed()`が一生発火しない。** 旧D1のまま「適用」ボタンを
  撤去すると、**この2種別はGUIから設定不能になる**（architect F2、premortem P2-4）。
- **キーボードグリッドのクリック処理（セル選択切替）は、同一フレーム内で編集パネルの
  描画より先に`layout_edit_value`を上書きする**（`select_layout_cell`呼び出しが
  グリッド描画の末尾にあり、編集パネルはその後に描画される）。旧D1の「セル切替時に
  コミット判定する」という設計は、**判定が走る前提のバッファが既に次のセルの値に
  上書き済み**のため成立しない（architect F3、premortem P2-5）。

### 旧D2（`layout_file_path==None`時に保存先ダイアログを開く）を却下した理由

- `layout_do_save_as_dialog()`は`std::thread::spawn(...).join()`で**UIスレッドを
  ブロックする同期処理**であり（`rfd::AsyncFileDialog`という名前に反し非同期ではない）、
  これを最も高頻度に押される下部「適用」の経路に持ち込むとダイアログが閉じるまで設定
  画面全体が応答なしになる（architect F8、premortem P5-3）。
- ダイアログの結果は`layout_pending_save_as`に積まれるだけで、実際の書き込みは
  `tab_layout()`が描画されたときにしか消費されない。**配列編集タブ以外にいる状態で
  「適用」→ダイアログ→保存先を選ぶ、という手順を踏んでも、配列編集タブを開くまで
  `.yab`は書かれない**（architect F8、premortem P5-1）。さらに2回目の「適用」で
  1回目に選んだパスが無条件に上書きされ消える（premortem P5-2）。
- 根本的に、`layout_file_path`が`None`になるのは配列読み込み自体が失敗した場合のみで、
  既定パス（`resolve_layouts_dir(layouts_dir).join(default_layout)`）は常に計算可能
  （architect F8）。したがって「ダイアログを開く」のではなく**「未設定という状態自体を
  作らない」**のが正しい解決策。

### その他、round1で新たに発見された重大な問題（当初案には無かった論点）

- **【最重要】キーボード配列（JIS/US）切替とセル単位の`.yab`書き込みを組み合わせると、
  データが無言で欠落する新しい経路が生まれる。** `YabLayout::serialize(model)`は
  `model.row_sizes()`（JIS: `[13,12,12,11]`、US: `[12,12,11,10]`、`src/scanmap.rs:
  47-52`）の範囲外の列を出力しない（`src/yab/mod.rs:345-362`）。**「配列編集タブで
  1セル編集」→「全般設定タブでJIS→US切替」→「下部の適用」という自然な1回の操作**で、
  JIS配列の右端列（`¥`位置等）が4面分まとめて消える。今日はこの経路が「配列編集タブの
  ツールバー保存」という明示操作を要求するため踏みにくいが、両タブの状態を1つの
  「適用」に統合すると**踏みやすくなる**（architect F7、premortem P4-2/P4-3、両者が
  独立に発見）。**本ADRが解決しようとしている「わかりにくさ」とは別に、これ自体が
  新しい重大なデータ消失バグになりうる。**
- **`awase.exe`の`reload_config()`は`layouts_dir`配下かつ`default_layout`と同名の
  `.yab`しか読まない**（`crates/awase-windows/src/app/mod.rs`の`reload_layouts`）。
  「開く」で`layouts_dir`外や別名のファイルを編集して「適用」しても、ファイルには
  書かれるがエンジンには一切反映されない——**本ADRが解決しようとしているのと同種の
  「ラベルと効果の不一致」を、別の入口で再生産する**（architect F9）。
- **画面下部共通の「キャンセル」は、今見ているタブに関係なく即座に押せる。** 配列編集
  タブで数十セル編集した後、別タブでの入力ミスを取り消すつもりで「キャンセル」を押すと、
  **その数十セル分が確認もundoも無いまま消える**。このエディタにundo機構は無い
  （履歴はコピー操作したセルのみ4件分の`layout_clipboard_history`）。現状はキャンセルが
  配列編集に触れないためこの事故は起きない——**旧D2はこれを新たに作り出す**
  （premortem P1-1、最も深刻な指摘）。
- **「名前を付けて保存」は現状`layout_file_path`を新パスに差し替え、`layout_modified`を
  falseにする。** これを残したまま下部「適用」に統合すると、「編集→名前を付けて保存で
  バリエーションを退避→適用」という操作で**元ファイルには何も書かれないまま「適用した
  のに反映されない」という、まさに今回のユーザー報告と同じ症状が別経路で再発する**
  （architect F16）。
- **`.yab`書き込みの失敗は`layout_status`にしか表示されず、これは配列編集タブ内でしか
  描画されない。** 別タブで「適用」した場合、失敗しても画面のどこにも出ない
  （architect F10、premortem P3-2）。
- **`apply_confirmed()`の連打・ボタン非活性化されていない状態での多重クリック**は、
  ガードの置き方次第で「2回目の編集が保存されないまま握りつぶされる」か「`.yab`だけ
  二重に書かれてconfig保存/エンジン通知と噛み合わなくなる」かのどちらかになる
  （premortem P3-1）。
- **`recompute_diagnostics()`が1回の「適用」で最大3回呼ばれる**（`layout_write_to_path`
  内・`apply_confirmed`冒頭・`poll_pending_save`の完了時）。実行時間は小さいが、
  `Dangerous`状態での早期returnとの組み合わせで診断表示が一瞬消えるなど順序上の実害が
  ある（architect F14、premortem P3-4）。

## round2レビューで判明した設計上の欠陥（v1のD1/D2は不採用）

round1改訂（`lost_focus()`トリガー、`layout_file_path`を常に確定、キーボードモデル
不一致ガード等）をOpus 2体に再度敵対的レビューさせたところ、**「適用しても反映
されない」という本ADRの出発点そのものを、`lost_focus()`の観測タイミング起因で
再生産するバグが3件**、**「読み込み失敗時にも既定パスを割り当てる」という修正が
新しいデータ消失経路を作っていること**、その他複数の重大な指摘が両者独立に見つかった。

### `lost_focus()`トリガーを却下した理由（v1のD1）

`egui::Response::lost_focus()`は「そのウィジェットが**今フレームに描画され**、かつ
前フレームでフォーカスを持ち今フレームで失った」ことを示す値であり、**そのウィジェット
が今フレームに描画されること自体が前提**になっている。ところが`update()`の実際の
描画順は「下部`action_panel`（適用/キャンセル）→ 左`SidePanel`（タブ切替）→
`CentralPanel`（選択中タブの内容）」であり、配列編集タブの編集パネルは**最後**に
描画される。これにより:

- **下部「適用」を押した瞬間、その同じフレームでは編集パネルがまだ描画されておらず
  `lost_focus()`を観測できない。** テキスト欄に入力したその場で「適用」を押すと、
  入力内容を含まない`.yab`が書き込まれる——**本ADRが解消しようとしている症状を
  そのまま再生産する**（premortem R2-1）。
- **タブを切り替えると、切替先のタブが描画され配列編集タブは描画されなくなるため、
  `lost_focus()`を観測する主体が存在しなくなる。** さらにeguiは「描画されなかった
  フォーカス保持ウィジェット」を検知してフォーカスを強制的に失わせる仕組みを持つため
  （"dead-man's switch"、`egui-0.31.1/src/memory/mod.rs:628-636`）、次に配列編集
  タブへ戻っても`lost_focus()`は二度と真にならない。**入力中の内容がタブ切替だけで
  無言に消える**（premortem R2-2）。
- **`handle_layout_shortcuts()`は`update()`の先頭近く、あらゆるパネル描画より前に
  呼ばれる。** D3で`Ctrl+S`を「画面全体の確定」に付け替えても、`Ctrl+S`はフォーカスを
  移動させないため`lost_focus()`は真にならず、直前の入力が含まれない適用になる
  （premortem R2-3、architect B7）。

3件とも根は同じ——**`lost_focus()`という「ウィジェットの描画結果」に依存する限り、
描画順や「そもそも描画されるか」に左右される**。トリガーの選び直しでは解けず、
「コミット処理を描画順から切り離す」という設計変更が必要（両者が独立に同じ結論）。

### 「読み込み失敗時にも既定パスを割り当てる」を却下した理由（v1のD2）

`layout_load_from_path()`は成功時にのみ`layout_file_path`をセットし、失敗時は
`self.layout`も変更しない（`empty_yab_layout()`のまま）。v1のD2はここに関係なく
既定パスを無条件で割り当てていたため、**「読み込みに失敗した（壊れた）`.yab`を
開く→気づかず1セル編集→適用」で、ほぼ空の配列が壊れる前の`.yab`を上書きする**、
という新しいデータ消失経路が生まれていた（architect B1、premortem R2-4、両者独立に
発見）。「既定パスは常に計算できるので保存先ダイアログは不要」（round1 F8）という
判断自体は正しいが、「常に書き込んでよい」わけではない——**「パスが決まっている
こと」と「そのパスから正しく読み込めていること」は別の条件**として管理する必要が
ある。

### その他、round2で新たに発見された重大な問題

- **「なし」ラジオを撤去する設計が自己矛盾していた。** v1のD1は「ラジオを切り替えた
  だけでは値を変更しない」と「`ValueKind::None`はラジオの`changed()`で即座にコミット
  する」を同時に書いていたが、「なし」はラジオの選択肢そのものなので**両者は矛盾する
  ——後者を実装すると前者の保護は「なし」に対して成立しない**（architect B2、
  premortem R2-9、両者独立に同一の矛盾を指摘）。5種別中「なし」が最も破壊的
  （値が消え、undoが無い）にもかかわらず、ツールチップを読もうとしてラジオへ
  マウスを合わせクリックする、という自然な誤操作で発火しうる。
- **種別だけを変えて値を確定させる操作の受け皿が無い。** 「なし」を誤操作から守るために
  「ラジオはコミットしない」を貫くと、今度は「打鍵からリテラルへ種別を変え、
  テキスト欄に触れずに別セルへ移る」という操作が永久にコミットされなくなる
  （architect B3）。**B2とB3は同じルールの両面であり、一体で設計しないと
  片方を直すと片方が壊れる。**
- **`.yab`書き込みと`self.config`の`mem::take`区間が競合しうる。** `apply_confirmed()`
  は`std::mem::take(&mut self.config)`（`main.rs:475`）から`self.config =
  AppConfig::from(validated)`（`main.rs:485`）までの間、`self.config`が`Default`値に
  なる。キーボードモデル不一致チェック（v1のD2で追加したもの）はこの区間に置くと
  `Default`値と比較してしまい誤判定する。**最重要指摘として扱ったガードそのものが、
  置き場所を明記しなかったせいで無効化されうる**（architect A1）。
- **`awase.exe`の`reload_config()`が`layouts_dir`配下・`default_layout`同名の
  `.yab`しか読まない**という round1 の指摘（F9）に、v1では対応する決定が存在しな
  かった。「開く」で対象外のファイルを編集して適用すると、書き込みは成功し
  `self.status`は成功を報告するのに、**エンジンには一切反映されない**——記録だけ
  残して決定を書かなかった箇所が、実装時に確実に見落とされる形になっていた
  （architect A2、premortem R2-12）。
- **キーボードモデル不一致の警告文が、実行すると必ず失敗する手順を案内していた。**
  `YabLayout::parse`は列数超過を**エラーにする**（`src/yab/tests.rs::
  parse_too_many_columns`）。「配列編集タブで再読み込みしてください」という v1 の
  案内どおりに、モデル変更後の状態で再読み込みすると、その再読み込み自体が
  パースエラーで失敗し、案内に従っても抜けられない（premortem R2-5）。さらに
  「`.yab`書き込みは止めるがconfig.toml側は保存を継続してよい」としていたため、
  `.yab`は守られてもエンジンはconfig.toml経由で新モデル・旧配列という不整合状態に
  なる（premortem R2-6）。
- **キャンセル確認モーダルが二択では、ユーザーが本当にしたいことができない。**
  「設定だけ元に戻したい、配列の30セルは残したい」という最も典型的な意図に対し、
  v1のモーダルは「破棄する/しない」の二択しか提供せず、**破棄すれば両方消え、
  破棄しなければ設定も戻らない**——どちらを選んでも要求を満たせない
  （premortem R2-7）。加えてこの新しい確認モーダルと既存の`Dangerous`設定確認
  モーダル（いずれも背景をブロックしない`egui::Window`実装）が同時に開ける状態を
  作りうる（premortem R2-8、architect B8）。
- **未保存確認を入れる箇所（D3）が、`layout_load_from_path`を呼ぶ全経路を洗い出せて
  いなかった。** 「開く」「再読み込み」ボタン・`F5`・`Ctrl+O`は塞いだが、パス欄への
  直接入力＋Enter（`layout_do_open_from_text_box`）が漏れていた（premortem R2-10、
  これも「1箇所だけゲートして満足しない」という`fix-requires-evidence.md`と同種の
  失敗パターン）。
- **適用ボタンだけ無効化してキャンセルボタンは無効化しない設計は非対称。**
  `pending_save.is_some()`の間、v1は適用ボタンのみ`add_enabled(false)`にすると
  決めていたが、キャンセル側は早期returnのみで**ボタンは押せるのに何も起きない**、
  というADR自身が問題視するパターンをキャンセル側に残していた（architect C1、
  premortem R2-11）。

## round3レビューで判明した設計上の欠陥（v2のコミット判定条件は不十分）

v2改訂（`update()`冒頭での無条件コミット判定）をOpus 2体に再度敵対的レビューさせた
ところ、round2の3件（R2-1〜R2-3）はいずれも構造的に解消していることが両者独立に
確認された一方、**「バッファに入っている内容＝ユーザーが確定させたかった値」という
v2が新たに置いた前提そのものが崩れる経路が2件**、その他複数の指摘が見つかった。

### 「バッファと現在のモデル値を直接比較する」判定を却下した理由

v2のD1は`commit_pending_layout_edit()`の差分判定を「`layout_edit_value`から構築した
値」対「`self.layout_face(face).get(pos)`の現在値」で行っていたが、これは以下の
2ケースで「ユーザーが一度も触っていない値」を「変更された」と誤判定する。

- **IME変換の未確定文字列がそのままコミットされる。** `commit_pending_layout_edit()`
  は`changed()`のようなイベントではなく毎フレームの状態比較なので、round1で却下した
  `changed()`トリガーが持っていた「IME変換の途中経過でも発火する」という欠陥を、
  トリガーを変えても**形を変えて温存していた**。「…」というリテラルセルを選択し、
  IMEで「てん」に変換中に`Esc`でキャンセルすると、変換中の中間文字列が確定入力の
  瞬間ごとに毎フレーム`self.layout`へ書き込まれ続け、最終的に空文字列に戻っても
  それまでの中間状態（「て」「てん」等）が既にモデルへ反映済みで、`Esc`は
  何も救済しない（architect V3、premortem R3-1、両者独立に発見。round1 P2-1で
  指摘した症状——空文字列コミットで値が消える——とは別の、同じIME起因の新しい
  経路）。
- **`ValueKind::Special`のラジオボタンを押しただけで、意図しない値が即座に
  コミットされる。** `layout_edit_special_idx`はテキストバッファ（`layout_edit_value`）
  とは独立したComboBoxの選択インデックスであり、セル切替時に初期化されず前のセルで
  操作した値（または未操作なら既定の0=Backspace相当）を引き継いだまま残る。
  「特殊キー」ラジオへ切り替えた瞬間、`commit_pending_layout_edit()`は
  `SPECIAL_KEYS[layout_edit_special_idx]`を「現在のモデル値と異なる」と判定して
  即座にコミットしてしまう——**ComboBoxを一度も開かず、ラジオを押しただけで
  セルの値がBackspace等へ無言で書き換わる**（premortem R3-2、round2 R2-9で
  「なし」について指摘した自己矛盾と同型の欠陥が、対処範囲を「なし」だけに
  絞ったことで「特殊キー」に取り残されていた）。
- **上記2つの根は同じ設計欠陥である。** 「バッファの現在値」対「モデルの現在値」を
  直接比較するかぎり、**バッファに値は入っているが、ユーザーが選択後に一度も
  実際に操作していない**という状態を区別できない。ADR-115打鍵列セルを読み取り専用
  にする対処（v2 D1）も同じ理由で不完全——`ui.add_enabled(false, ...)`はウィジェットの
  描画属性であって`commit_pending_layout_edit()`の判定条件には一切影響しないため、
  読み取り専用にしても「セルを選択しただけで、シリアライズ表示文字列がリテラルとして
  誤って再コミットされる」という経路は塞がれていない（architect V1）。さらに、
  この「表示文字列がストアされた値へ過不足なく往復する」という前提自体が、将来
  `SpecialKey`のバリアント追加等で`SPECIAL_KEYS`配列の更新を忘れると静かに壊れる
  暗黙の不変条件になっていた（architect V2、一般化した指摘）。

### その他、round3で新たに発見された問題

- **`self.layout`を丸ごと差し替える処理（キャンセルの「両方元に戻す」、
  `layout_do_reload`、`layout_load_from_path`成功時）が、選択中セルのバッファ
  （`layout_selected_pos`/`layout_edit_value`等）を再同期していない。** 差し替え後も
  古いセルが選択されたままだと、次フレームの`commit_pending_layout_edit()`が
  古いバッファ内容を新しい`self.layout`と比較し、**「元に戻したはずの値」を
  再び上書きコミットしてしまう**——round1 P1-2（読み込み後の選択位置ズレ）と同種の
  「モデル総入れ替え後の選択・バッファ再同期漏れ」が、コミット判定を新設した
  ことで別の実害（サイレントな再コミット）として再発する（premortem R3-3）。
- **`layout_status`が、無効な入力が残っているセルを選択している間、毎フレーム
  同じパースエラーで上書きされ続ける。** `commit_pending_layout_edit()`は
  `update()`冒頭で毎フレーム実行されるため、パースに失敗するバッファを持つセルを
  選択したまま他の操作（下部「適用」での保存成功等）をしても、**その成功メッセージが
  画面に表示される前の次のフレームで、`layout_status`へのパースエラー書き込みに
  即座に上書きされ、ユーザーは成功メッセージを一切見られない**（premortem R3-4）。
- 「範囲外」節（旧行477-479）が「IME変換中の打鍵欄における赤エラー表示のちらつきは
  `commit_pending_layout_edit()`化によりモデルへの実害は無くなった」としているのは、
  上記のIME欠陥（R3-1）が未修正のv2時点の記述であり誤り。修正後の記述に更新する
  必要がある（premortem R3-5）。
- **キーボードモデル不一致ガード（D2）の発火条件が`layout_modified`を要求する
  ため、「配列は編集していないが、全般設定タブでキーボードモデルだけ変更して
  適用した」場合はガードが発火しない。** これは既存の別種の不整合（本ADRが新規に
  作るリグレッションではない）だが、`YabLayout::parse`は列数超過を実際にエラーに
  する（`src/yab/tests.rs:453-464`）ため、JIS配列を開いたまま全般設定タブで
  US配列に切り替えて適用する、という**セル編集より遥かに起こりやすい操作**で
  エンジンが確実に壊れる（premortem、スコープ注記で実測経路を確認）。D2がこの
  ガードを「モデル不一致問題への対処」と位置付けている以上、この経路を素通り
  させたままにはできない——`layout_modified`要件そのものを外す方向で決定する
  （後述、決定D2を参照）。

### 収束に向けた設計方針（両者が同意した方向性）

- `commit_pending_layout_edit()`の判定基準を「モデルとの直接比較」から
  **「セル選択時点でのスナップショット（`layout_edit_origin`）との差分」**に変更する。
  `select_layout_cell`実行時に、そのセルの表示用文字列（現在`layout_edit_value`を
  初期化するのに使っている値）を`layout_edit_origin: Option<String>`へ保存し、
  以後は`layout_edit_value != layout_edit_origin`のときのみ「ユーザーが実際に
  編集した」とみなす。ADR-115打鍵列セルは選択しても`layout_edit_value`が
  `layout_edit_origin`（＝初期表示値）から変化しようがない（読み取り専用のため）ので、
  この時点で自然にコミット対象から外れる（V1解消）。将来型が増えても
  「表示文字列がストアされた値に過不足なく往復するか」という不変条件に依存しなく
  なる（V2解消）。
- IME合成中かどうかを追跡する`ime_composing: bool`を追加し（`ctx.input(|i| &i.events)`
  で`egui::Event::Ime`の`Enabled`/`Preedit`受信時に`true`、`Commit`/`Disabled`受信時に
  `false`にする。`ImeEvent`はpublicだが`TextEditState::ime_enabled`は`pub(crate)`で
  参照できないため自前追跡が必要——architect V3の確認事項）、`ime_composing`が真の
  間は`commit_pending_layout_edit()`を完全にスキップする（IME確定後、次フレームで
  `layout_edit_origin`との差分判定が通常どおり走るため、中間状態を経由せず最終的な
  確定結果だけが評価される。R3-1解消）。
- `ValueKind::Special`は`commit_pending_layout_edit()`の対象から外し、ComboBox
  ウィジェット自身の`response.changed()`で即座にコミットする独立経路にする
  （R3-2解消）。ComboBoxの選択は離散的なイベントでIME合成を経由しないため、
  round1でテキスト入力について却下した`changed()`トリガーの欠陥（IME未確定文字列
  での誤発火）はここでは発生しない——「なし」（D1で専用ボタン化済み）と合わせて、
  自由入力欄を持たない2種別はどちらも「値ウィジェットへの明示操作」でのみコミット
  する、という統一ルールになる。
- `self.layout`を丸ごと差し替えるすべての経路で、直後に`layout_selected_pos = None`
  （および`layout_edit_origin`/`layout_edit_value`のクリア）を行うことをD1の不変
  条件として明記する（R3-3解消）。
- `commit_pending_layout_edit()`の冒頭で「前フレームから入力状態
  （`layout_edit_value`/`layout_edit_kind`）が変化したか」を確認し、変化していない
  フレームでは`layout_status`を含め一切の処理を行わずに早期returnする
  （R3-4解消——同じエラーを毎フレーム再書き込みしない）。

## round4レビューで判明した設計上の欠陥（v3のコミット判定・状態管理に3件のblocker）

v3改訂（`layout_edit_origin`との差分判定＋`ime_composing`ガード）をOpus 2体に
再度敵対的レビューさせたところ、round3の指摘（R3-1〜R3-5）はいずれも解消して
いる一方、**その実装の細部に3件のblocker（うち2件は両者が独立に同一の欠陥へ
到達）**が見つかった。

### `layout_edit_last_seen`の更新タイミングにより、IME確定した編集が消える（architect W1）

v3のステップ2は「前フレームと値が違えば`layout_edit_last_seen`を**その場で**
更新してから続行する」という設計だった。IME変換中の各プレビューフレームでも
このステップは「値が変わった」と判定するため、**変換途中の毎フレームで
`layout_edit_last_seen`が更新され続ける**。変換確定（`Esc`を押さない、通常の
確定）の瞬間、`egui`はバッファを`delete_selected`+`insert_text_at`で置き換える
ため、直前のプレビュー文字列と確定後の文字列が同じであれば**バッファは変化
しない**。この場合、確定フレームでは`ime_composing`が偽に戻っているにも
関わらず、`layout_edit_last_seen`は既に（変換中に）その値を記録済みのため、
ステップ2の「変化なし」判定で即returnし、**この編集は一度もパース・コミットの
機会を得ないまま失われる**。日本語入力で変換を確定するという最も日常的な
操作が、無変換入力時よりも高い確率で編集を失う経路になっていた。

### `layout_edit_origin`が文字列のみのため、round2 B3（種別だけの変更）が回帰していた（architect W2、premortem R4-1）

v3のステップ5は`layout_edit_value == layout_edit_origin`（`Option<String>`）の
比較のみで、`layout_edit_kind`を比較対象に含めていなかった。D1自身が「打鍵から
リテラルへ種別を変え、テキスト欄に触れずに別セルへ移る操作も拾われる
（round2 B3解消）」と明記していたにも関わらず、**この記述と矛盾する形で
実際にはステップ5がその操作を握りつぶしていた**——種別ラジオを変えても
`layout_edit_value`自体は変化しないため、文字列だけの比較では「変更なし」と
誤判定される。表示上は新しい種別（例: リテラル）が選ばれたままになるため、
ユーザーは変更が反映されたと誤解する。両者が独立に同一の欠陥へ到達した。

### 上記の修正（originにkindを含める）だけでは、ADR-115打鍵列セルの保護が別経路で崩れる（architect W3、premortem R4-2）

`layout_edit_origin`を`(String, ValueKind)`のタプルに直せばB3は解けるが、
それだけでは**種別ラジオを操作しただけでADR-115打鍵列セル（`CtrlChord`/
`InlineSequence`/`MacroRef`）を破壊できてしまう**という新しい経路が生まれる。
`MacroRef("name")`のセルを選択すると`layout_edit_kind=Literal`・
`layout_edit_value="@name"`で初期化されるが（`main.rs:847-854`）、テキスト欄は
読み取り専用でも**種別ラジオ自体は無効化されていない**。ラジオで「打鍵」に
切り替えるだけで`(value, kind)`のタプルが originから乖離し、`"@name"`が
`YabValue::KeySequence("@name")`としてコミットされ、ADR-115の打鍵列が
無言でリテラル打鍵へ降格する——**まさにround3 architect V1が警告した
「読み取り専用化だけでは防御にならない」が、テキスト欄ではなくラジオという
別のウィジェット経由で再現する。**「B3を成立させる（originにkindを含める）」
ことと「ADR-115セルをステップ5だけで守る」ことは両立しない——**v3は後者を
選んだ結果、前者（round2 B3）が壊れていたことに気づかず両方解消したと
記述していた。**

### その他、round4で新たに発見された問題

- **`ime_composing`が真のまま固着すると、配列編集がエラー表示すら無く全停止
  しうる。** eguiは、IME合成中にフォーカスが移動すると`Event::Ime`を
  イベントキューから除去する（`egui-0.31.1/src/widgets/text_edit/builder.rs:
  796-803`、`response.gained_focus() || response.lost_focus()`時に`retain`で
  除去）。この経路で`Commit`/`Disabled`イベントが一度もアプリに届かないまま
  `ime_composing`が真に固着すると、以後`commit_pending_layout_edit()`が
  永久にスキップされ続け、症状は「何を入力しても反映されない」だけでエラー
  表示も出ない（architect W4）。
- **`ime_composing`の更新タイミングが、D3の`update()`処理順序リストに
  含まれていなかった。** `commit_pending_layout_edit()`より前に更新しないと
  1フレーム古い値で判定してしまい、最初のプレビューフレームを取りこぼす
  （architect W5）。
- **`select_layout_cell`が`layout_edit_last_seen`をリセットする記述が
  漏れていた**（architect W6）。
- **D3の処理順序リストに、実際は`SidePanel`より前に呼ばれる
  `config_path_panel`（`main.rs:2573`）と、`apply_confirmed()`を直接呼ぶ
  `show_dangerous_save_confirm_modal`（`main.rs:2574`）が列挙されておらず、
  グローバル`Ctrl+S`ハンドラの挿入位置を誤る余地があった**（architect W7）。
- **round3 R3-3の不変条件（`self.layout`総入れ替え時のバッファ再同期）が、
  v3で新設した「単一セルを直接コミットする2経路」——「なし」の解除ボタンと
  「特殊キー」ComboBoxの`changed()`経路——を対象に含めていなかった。**
  「打鍵」セルを選択（origin=("ka", Keystroke)）→ラジオを「特殊キー」に変え
  ComboBoxでEnterを選択（独立経路でモデルは`Special(Enter)`になるが、
  `layout_edit_value`/`layout_edit_origin`は再同期されず"ka"/("ka",Keystroke)
  のまま）→ラジオを「打鍵」に戻す、という手順で、（originにkindを含める
  修正後は）origin==currentとなり何もコミットされないため、**編集パネルの
  表示（打鍵/"ka"）とモデル（`Special(Enter)`）が別セルへ移るまで恒久的に
  食い違う**（premortem R4-3）。
- **（任意、低優先度）`ime_composing`がCommit確定と同じフレームでは
  厳密に1フレーム遅れて偽になる。** `egui`のTextEditが`Ime::Commit`を
  実際に処理するのは`CentralPanel`描画時（`update()`冒頭のイベント走査より
  後）のため、Commitを検出したその同じフレームの`commit_pending_layout_edit()`
  はまだ未確定のプレビュー文字列を評価してしまう可能性がある。通常は次フレーム
  で確定後の値が上書きコミットされるため実害はほぼ無いが、同一フレーム内で
  他の操作（タブ切替・「適用」）と重なる希少ケースでは中間文字列が残りうる
  （premortem R4-4、収束条件には含めないが1行で直せるため採用する）。

### 【追加blocker】キーボードモデル不一致ガードから`layout_modified`要件を外したことで、公式に案内されている正しいモデル切替手順が実行不能になる（architect W8、premortem R4-5）

上記W1〜W7への対応と並行して送った「D2のスコープ注記に対する回答」
（`layout_modified`要件の撤回）に対し、両者が独立に新しいblockerを発見した。

`config.toml.sample`（10-12/38-41/50行目）は「`keyboard_model`を変更するときは
`default_layout`も必ず一緒に変更すること」と明記し、US用の`layout/nicola_us.yab`
も同梱されている。**正しい手順は、全般設定タブで`keyboard_model=us`と
`default_layout=nicola_us.yab`を同時に変更して1回「適用」することであり、
配列編集タブでのセル編集は一切不要。** ところが撤回後のガードは
`layout_loaded_model != config.general.keyboard_model`だけを見るため、この
**完全に正しい操作**を次の手順で止めてしまう:

1. 起動時（config=JIS）に配列編集タブを一度開く→`ensure_layout_loaded`が
   `nicola_keytop.yab`をJISで読み込み、`layout_loaded_model = Jis`になる
2. 全般設定タブで`keyboard_model=us`・`default_layout=nicola_us.yab`に変更
   （セル編集なし、`layout_modified`は偽のまま）
3. 「適用」→ `layout_loaded_model(Jis) != keyboard_model(Us)`でガードが発火
   → `apply_confirmed()`全体が中止される
4. 案内文どおり「配列編集タブで開き直す」を試みても、`再読み込み`は列数超過で
   確実にパースエラーになる（round3で実測済み、`src/yab/tests.rs:453-464`）ため
   使えず、「開く」で`nicola_us.yab`を選ぶしかない——ユーザーはエディタを
   使いたいわけではなく設定を変えたいだけなのに、エディタ操作を強制される

さらに`layout_loaded_model`は「配列編集タブを一度でも開いたか」という、
**変更しようとしている設定の内容とは無関係な操作履歴**で決まる
（`ensure_layout_loaded`は`tab_layout`からしか呼ばれない、`main.rs:1930`）。
一度も開かなければガードは発火せず（`layout_loaded_model`の初期値が未定義
だったことも合わせて指摘された、premortem R4-5）、開いていれば発火する——
同じ設定変更が、ユーザーから見えない理由で通ったり弾かれたりする。

**根本原因はガードの判定対象が間違っていること。** `reload_config()`が実際に
読むのは`resolve_layouts_dir(config.layouts_dir)`配下の`config.default_layout`
であって、エディタで開いている（かもしれない）ファイルではない。エディタの
状態と無関係に「適用後に効力を持つ設定の組がエンジンで壊れないか」だけを
見るべきだった。

### 収束に向けた設計方針（v5/v6で採用）

- `commit_pending_layout_edit()`の判定順序を「ガード（None/Special/ADR-115
  シーケンス/IME合成中）を先にすべて通過してから、初めて`layout_edit_last_seen`
  を参照・更新する」順序に組み替える。`layout_edit_last_seen`の更新は
  **実際にパース・コミット判定まで到達したフレームでのみ**行う（W1解消）。
- `layout_edit_origin`を`Option<(String, ValueKind)>`に変更し、ステップ5の
  比較をタプルで行う（W2/R4-1解消）。
- ADR-115打鍵列セルは、`layout_edit_origin`との差分判定に一切頼らない
  **構造的な無条件ガード**（`layout_edit_origin_is_sequence: bool`）で守る。
  該当セルでは値ウィジェットに加え**種別ラジオの行そのものも無効化**する
  （W3/R4-2解消——V1が求めていた「UIの無効化に依存しない構造的ガード」と
  「UI自体も無効化する」の両方を満たす二重の防御にする）。
- 「解除」ボタン・「特殊キー」ComboBoxの直接コミット経路は、コミット直後に
  `select_layout_cell(pos)`を呼び直して`layout_edit_value`/`layout_edit_origin`/
  `layout_edit_kind`/`layout_edit_last_seen`をまとめて再同期する（`main.rs:893`
  の`paste_layout_cell`が既に採っているパターンを流用。R4-3解消）。
- `ime_composing`は`select_layout_cell`実行時と`layout_selected_pos`が`None`の
  ときに強制的に`false`へリセットする防御的ガードを追加する（W4対応）。
  更新タイミングをD3の処理順序リストの先頭（ステップ0）に明記し、
  `config_path_panel`/`show_dangerous_save_confirm_modal`も含めて列挙する
  （W5/W7対応）。
- `ime_composing`の条件を「フラグが真」から「フラグが真、または当該フレームの
  イベント列に`Event::Ime(_)`が含まれる」へ広げ、Commit確定フレームの1フレーム
  ずれを構造的に消す（R4-4採用）。
- キーボードモデル不一致に関する単一のガードを、**目的の異なる2つの独立した
  ガードに分離する**（W8/R4-5解消）。(A) 配列編集タブの未保存の編集を、モデル
  不一致に気づかず書き込んで壊さないための**データ保護ガード**（`layout_modified`
  かつ`layout_loaded_model`が既知の場合のみ発火——本ADRが当初から持っていた
  役割そのもの）。(B) `config.general.default_layout`が適用後の
  `config.general.keyboard_model`で実際にパースできるかを検証する**エンジン
  健全性ガード**（`layout_loaded_model`・`layout_modified`・配列編集タブを開いた
  かどうかに一切依存せず、ディスク上の実ファイルを直接検証する）。この2つを
  1つの条件式に混ぜていたことが、W8/R4-5の「正しい操作が理由不明で弾かれる」
  「エディタの操作履歴に依存する」という2つの症状を同時に生んでいた。
- ガード(B)を「パース失敗＝常に中止」という無条件の実装にしたこと自体が
  新しい退行を生んでいたため（premortem R5-1）、**中止するか警告に留めるかを
  「この適用でキーボード配列が実際に変わるかどうか」で分岐させる**。
  `config_loaded_model: KeyboardModel`（直近の読み込み/保存成功時点の
  モデル）を追跡し、これと`self.config.general.keyboard_model`が異なる
  （＝モデルが変わる）場合のみ、パース失敗で`apply_confirmed()`全体を
  中止する。モデルが変わらない場合のパース失敗は、既存の不整合を悪化させて
  いないため中止せず、`self.status`への警告表示に留めて保存を継続する
  （ADR-116の起動時診断が想定する「壊れた`.yab`が存在する」状態を、設定
  画面自体が保存不能になる形で悪化させないため）。

### round5で判明した軽微な改善点（X1〜X5、blockerではない）

v4/v5の実装細部について、architectから5件の非blocker指摘があった。いずれも
決定文への一文追記で足りる。

- **X1**: ADR-115打鍵列セルは種別ラジオの行ごと無効化される（W3/R4-2）ため、
  `layout_edit_kind`を`ValueKind::None`に切り替えられなくなり、**「このキーの
  割り当てを解除」ボタン自体に到達できない**（ボタンの表示条件が
  `kind == ValueKind::None`のため）。従来はセルを選んで「なし」に切り替え
  「適用」を押せば打鍵列も解除できたが、この経路がGUIから失われる。
- **X2**: ステップ5（`raw == origin`でreturn）は`layout_edit_last_seen`を
  更新しないため、**パースエラーとなる値を入力した後、元の値に戻しても
  `layout_status`のエラー表示が残ったままになる**（例:「あ」で失敗
  →`layout_status`にエラー→打ち直して`"ka"`〈origin〉に戻す→ステップ5で
  即return、`layout_status`はクリアされない）。
- **X3**: ステップ4（`ime_composing`）でreturnした場合、egui は他に入力が
  無ければ次フレームを自発的に描画しないため、**IME確定によるコミットが
  次に何らかの入力イベントが発生するまで遅延しうる**。「適用」ボタン・
  `Ctrl+S`・他セルのクリックはいずれも先にステップ2〜7を通過するため実害は
  無いが、D5の「未保存の変更があります」バナー等の即時反映が遅れる可能性が
  ある。
- **X4**: バッファ再同期の不変条件（後述）の対象リストに、既存の
  `paste_layout_cell`（`main.rs:884-894`、クリップボード履歴からの貼り付け）が
  含まれていない。この経路も`commit_pending_layout_edit()`の外側で
  `self.layout`を書き換えるが、既存実装は末尾で`select_layout_cell(pos)`を
  呼んでおり、既に不変条件を満たしている。ただし一覧に明記しておかないと、
  将来この呼び出しが誤って削除されたときに検知できない。
- **X5**: `ime_composing`のリセット契機が`select_layout_cell`実行時と
  `layout_selected_pos == None`時の2つだが、**別タブへの切替や配列パス欄
  （`main.rs:1980`）へのフォーカス移動中にIME合成が進行していた場合**も、
  eguiがフォーカス変更時に`Event::Ime`をイベントキューから除去するため
  （`egui-0.31.1/src/widgets/text_edit/builder.rs:796-803`の`retain`。
  なおこの除去はTextEdit描画時＝`CentralPanel`内で起きるため、D3ステップ0
  〈`update()`冒頭のイベント走査〉より後であり、「eguiが除去するせいで
  `ime_composing`が固着する」という直接の原因ではなく、根本はegui/winit側が
  フォーカス喪失時に`Commit`/`Disabled`を送らない場合があること——
  premortemが指摘した記述の正確化）、同様に固着しうる。セルを再選択すれば
  復帰するため実害は限定的。

### 【追加blocker、v5固有】エンジン健全性ガード(B)を無条件にしたことで、キーボード配列を変更しない適用まで巻き込む新しい退行（premortem R5-1）

W8/R4-5への対応でガード(B)を「配列編集タブの状態に一切依存しない」設計に
したこと自体は正しかったが、**「パースに失敗したら常に中止する」という
実装まで無条件にしたことで、キーボード配列を一切変更していない適用まで
巻き込む新しい退行**が生まれていた。

**操作手順**: ユーザーが`layout/nicola_j.yab`を手で編集していて構文を壊す
（列数超過等）。キーボード配列（JIS）は変更していない。設定画面を起動し、
配列編集タブは開かず、別のタブ（例: アプリ無効化）で何かを変更して
「適用」を押す。

**起きる問題**: ガード(B)は`layout_loaded_model`等に一切依存せず、常に
`default_layout`を**現在の**`keyboard_model`（＝変更していないJISのまま）で
パース検証する。手元の壊れたファイルはこの時点で既にパースエラーになる
ため、**キーボード配列を一切触っていないのに`apply_confirmed()`全体が
中止され、どのタブのどの設定も保存できなくなる**。表示される案内文
（「キーボード配列を元に戻してください」）も、そもそも変更していないため
実行不可能で、真因（ファイルの構文エラー）を指していない。

**v4までには無かった退行である点**: `layout_loaded_ok`（既存のD2冒頭の
決定）は`.yab`の**書き込み**だけを止める設計で、`config.toml`の保存は
妨げなかった。ガード(B)を無条件にしたことで、初めて壊れた`.yab`が
`config.toml`保存まで巻き込むようになった。しかも壊れた`.yab`はADR-116の
起動時診断（`scan_yab_files_for_diagnostics`）が想定内の状態として扱って
いるものであり、**その診断を見て修正しに来るための設定画面自体が、
壊れているという理由で保存不能になる**——ADR-116の意図と衝突する。

**対策として、ガード(B)の中止/警告の分岐を「この適用でキーボード配列が
実際に変わるかどうか」で条件分けする**（上記D2の決定に反映済み）。
キーボード配列が変わる適用でのパース失敗は、R2-6が懸念した「モデル切替
だけでエンジンが壊れる」を防ぐため引き続き中止する。キーボード配列が
変わらない適用でのパース失敗は、既存の不整合を新たに悪化させるものでは
ないため中止せず、`self.status`への警告表示に留めて保存を続行する。

## 決定

### D1: 配列編集タブのセル単位「適用」ボタンを撤去し、`update()`冒頭で無条件にコミット判定する

**コミット処理をegui のフォーカス/描画順から完全に切り離す。** 個々のウィジェットの
`changed()`/`lost_focus()`には一切依存せず、`update()`の**先頭**（`poll_pending_save`
の隣、既存の`main.rs:2545`付近、`action_panel`・`SidePanel`・`CentralPanel`・
`handle_layout_shortcuts`の**どれよりも前**）で、次の処理を毎フレーム無条件に行う
`commit_pending_layout_edit()`を新設する。

**判定基準は「モデルとの直接比較」ではなく「セル選択時点のスナップショットとの差分」
にする（round3で判明した、バッファの中身を無条件に信用することの危険性への対処）。**
新設する状態:

- `layout_edit_origin: Option<(String, ValueKind)>` — `select_layout_cell`実行時に、
  そのセルの表示用文字列（現在`layout_edit_value`の初期値として使っている値と
  同じもの）と`layout_edit_kind`の組を保存する。**文字列だけでなく`ValueKind`も
  含める**（round4 architect W2/premortem R4-1——文字列のみだと「種別だけ変えて
  テキストには触れない」操作を「変更なし」と誤判定し、round2 B3が回帰する）。
  セル選択が変わるたび、または`self.layout`が丸ごと差し替わるたびに再設定・
  クリアする（後述の不変条件を参照）。
- `layout_edit_origin_is_sequence: bool` — `select_layout_cell`実行時に、選択した
  セルの**元の`YabValue`**が`CtrlChord`/`InlineSequence`/`MacroRef`のいずれかで
  あれば`true`にする。この値は`layout_edit_origin`のタプル比較とは独立した
  構造的なガードとして使う（後述。round4 architect W3/premortem R4-2）。
- `ime_composing: bool` — `update()`内で毎フレーム`ctx.input(|i| &i.events)`を走査し、
  `egui::Event::Ime(ImeEvent::Enabled | ImeEvent::Preedit(_))`を観測したら`true`、
  `egui::Event::Ime(ImeEvent::Commit(_) | ImeEvent::Disabled)`を観測したら`false`に
  更新する（`ImeEvent`はpublic、`egui-0.31.1/src/data/input.rs:475,545-557`で確認済み。
  `TextEditState::ime_enabled`は`pub(crate)`のため参照できず、自前追跡が必要）。
  **この更新は`commit_pending_layout_edit()`本体より前、`update()`の実質的な
  ステップ0として行う**（round4 architect W5——後でないと1フレーム古い値で
  判定し最初のプレビューフレームを取りこぼす）。加えて、`select_layout_cell`
  実行時・`layout_selected_pos`が`None`になった時点・**`active_tab`が変わった
  時点**（round5 architect X5——配列パス欄`main.rs:1980`等、配列編集タブ以外の
  テキスト欄でIME合成中にタブが切り替わった場合も同様に固着しうる）で強制的に
  `false`へリセットする（round4 architect W4）。この防御的リセットが必要な
  根本原因は、eguiがIME合成中にフォーカスが移動すると`Event::Ime`をイベント
  キューから除去すること自体（`egui-0.31.1/src/widgets/text_edit/builder.rs:
  796-803`）**ではなく**——この除去はTextEdit描画時＝`CentralPanel`内で起き、
  D3ステップ0（`update()`冒頭のイベント走査）より後なので、ステップ0の走査
  からこの除去で`Commit`/`Disabled`が隠される訳ではない——**egui/winitの側が
  フォーカス喪失時に`Commit`/`Disabled`イベント自体を送らない場合がある**こと
  （round5 premortemによる根拠の正確化）。防御的リセットの判断自体は変わらず
  必要。判定条件は「`ime_composing`が
  真」だけでなく「**当該フレームのイベント列に`Event::Ime(_)`が1つでも含まれる**」
  も含める（round4 premortem R4-4——`Ime::Commit`をeguiのTextEditが実際に処理
  するのは`CentralPanel`描画時で`update()`冒頭のイベント走査より後のため、
  Commit検出と同じフレームでは1フレームだけ未確定文字列を評価してしまう
  可能性があった。この条件追加で構造的に消える）。
- `layout_edit_last_seen: Option<(String, ValueKind)>` — 直近に**実際にパース・
  コミット判定まで到達した**入力状態を保持する、ステータス上書き抑制専用の
  キャッシュ。**この更新タイミングが重要**（後述）。

`commit_pending_layout_edit()`の処理内容:

1. `layout_selected_pos`が`None`なら何もしない。
2. `layout_edit_origin_is_sequence`が真なら、ここで**無条件に**returnする
   （ADR-115打鍵列セルは`layout_edit_value`/`layout_edit_kind`がどう変化しても
   一切コミットしない。`layout_edit_last_seen`も更新しない）。
3. `layout_edit_kind == ValueKind::None`または`ValueKind::Special`なら、ここで
   returnする（前者は専用の「解除」ボタン、後者はComboBox自身の`changed()`で
   コミットする独立経路のため、このステップでは扱わない。詳細は後述。
   `layout_edit_last_seen`は更新しない）。
4. `ime_composing`が真（上記の拡張条件を含む）ならreturnする（IME変換の中間状態を
   一切モデルへ書き込まない。`layout_edit_last_seen`は更新しない——**ここが
   round4 architect W1の修正点**: v3は「値が変わったかどうか」を`ime_composing`
   より先に判定し、変換中の毎フレームで`layout_edit_last_seen`を更新していた
   ため、変換確定後にバッファが変化しない場合〈確定文字列が直前のプレビューと
   同じ〉、確定フレームで「既に見た値」と誤判定されコミット機会を失っていた。
   ガードをすべて`layout_edit_last_seen`の参照・更新より前に置くことで、
   `layout_edit_last_seen`は「実際に処理を試みた入力」だけを記録するようになる）。
   **このreturnの直前で`ctx.request_repaint()`を呼ぶ**（round5 architect X3——
   eguiは他に入力が無ければ次フレームを自発的に描画しないため、呼ばないと
   IME確定によるコミットが次の何らかの操作まで遅延しうる。D5のバナー等の
   即時反映のため1行追加する）。
5. `(layout_edit_value.clone(), layout_edit_kind)`（以下`raw`）が`layout_edit_origin`
   と一致するなら、**`raw`が`layout_edit_last_seen`と異なる場合にのみ
   `layout_status`をクリアし`layout_edit_last_seen = Some(raw.clone())`に
   更新した上で**returnする（＝選択した時点から実質的な変更が無い。round5
   architect X2——このクリア処理が無いと、パースエラーとなる値を入力した後に
   元の値へ打ち直しても、ステップ6以降に到達しないため`layout_status`の
   エラー表示が残り続ける）。
6. `raw`が`layout_edit_last_seen`と一致するならreturnする（＝前回このステップまで
   到達したときと同じ入力のまま——round3 R3-4解消。無効な入力を残したまま
   他の操作をしても、同じエラーが毎フレーム`layout_status`へ再書き込みされ
   続けることがなくなる）。不一致なら`layout_edit_last_seen = Some(raw.clone())`
   に更新してから続行する。
7. `layout_edit_value`から`YabValue`を構築する（`apply_layout_edit()`が現在行って
   いるパース・バリデーションロジックをそのまま流用する）。パースが成功し、
   かつ構築結果が`self.layout_face(face).get(pos)`の現在値と異なる場合のみ
   `self.layout_face_mut(face).insert(pos, value)`を実行し`layout_modified = true`、
   `layout_edit_origin = Some(raw)`に更新する（同じ値を毎フレーム再コミットしない
   ため）。`layout_status`はクリアする。パースが失敗した場合はモデルを変更せず
   `layout_status`にエラーメッセージを表示する（ステップ6の早期returnにより、
   これは入力内容が変化した最初のフレームでしか起きない）。

この設計により、「テキスト欄がその瞬間に描画されているか」「フォーカスが今フレーム
移動したか」に一切依存しなくなる。**下部「適用」を押したその同じフレームでも、
`action_panel`が処理される前に直前の入力が確定済みになる**（round2 R2-1解消）。
**タブ切替やウィンドウ操作でその後編集パネルが二度と描画されなくても、次フレーム
以降のどこかで`update()`が呼ばれる限りコミットが確実に実行される**（round2 R2-2
解消——`SidePanel`のタブ切替処理より`commit_pending_layout_edit()`が先に走る）。
**`Ctrl+S`等のショートカットも、`handle_layout_shortcuts()`より前に確定済みの状態で
処理される**（round2 R2-3解消）。`layout_pending_select`のような1フレーム遅延バッファ
は不要になる——セル選択の切替自体は即座に行ってよく（`select_layout_cell`を現状どおり
グリッド描画内で呼ぶ）、次フレーム冒頭のコミット処理はそれより前に完了しているため、
選択切替でバッファが上書きされる前に必ず確定している。

**空文字列を`YabValue::None`としてコミットする現状の分岐は、この自動コミットの対象
から除外する。** テキスト欄を空にしただけでは既存の値を保持し、`layout_status`に
「空にすると値が消えます。「なし」を選んでください」等のガイドを出す（IME変換中止
（`Esc`）や`Ctrl+A`→打ち直しの一瞬に空文字列が確定入力として届く経路——round1
premortem P2-1——から、既存の値を保護する。なお`ime_composing`ガード（ステップ4）
により、変換中の中間的な空文字列自体がコミットされることはなくなったが、
変換を伴わない`Ctrl+A`→`Delete`等での意図的な全消去は引き続きこのガードの対象と
する）。

**打鍵欄（`ValueKind::Keystroke`）の`resp.changed()`で即座に呼ばれている
`normalize_keystroke_input()`（`main.rs:2237-2239`）の呼び出しにも`ime_composing`
ガードを追加する。** これは`commit_pending_layout_edit()`とは別に、テキスト欄の
`Response::changed()`をトリガーとしてその場で`layout_edit_value`自体を正規化
（大文字小文字統一等）して書き戻す既存コードで、IME合成中でも`changed()`は発火
する（round1で確認済みの性質）ため、変換中のプレビュー文字列を横から書き換えて
しまいうる（round1 P2-3、round3 premortemが`ime_composing`導入の副次効果として
指摘）。`ime_composing`が真の間はこの正規化処理もスキップする。

**「なし」と「特殊キー」の2種別は、いずれも自由入力欄（`layout_edit_value`との
差分追跡）を持たないため、`commit_pending_layout_edit()`の対象から外し、それぞれ
専用の明示的操作でのみコミットする（round2 B2+B3、round3 R3-2を統合した設計）。**
種別ラジオ（`ValueKind`の5種別）を切り替える操作そのものは、`layout_edit_kind`という
**表示上の状態**を変えるだけで、モデルへの書き込みは一切行わない。

- **「なし」**: 編集パネルに`layout_edit_kind == ValueKind::None`かつ現在のセルの
  値が`YabValue::None`でないときにのみ表示される「このキーの割り当てを解除」
  ボタンを新設し、これを明示的に押した場合のみ`YabValue::None`をコミットする
  （「解除」というラベルなので原則3の「適用」多義禁止には抵触しない——「削除」系の
  操作に確認的なワンクッションを残すのは一般的なUXパターンでもある）。コミット
  直後に`select_layout_cell(pos)`を呼び直し、`layout_edit_value`/`layout_edit_origin`/
  `layout_edit_kind`/`layout_edit_last_seen`を新しいモデル値（`YabValue::None`）と
  再同期する（後述の拡張した不変条件、round4 premortem R4-3解消）。
- **「特殊キー」**: ComboBoxウィジェット自身の選択が実際に変わった時点で、
  選択された`SpecialKey`を直接`self.layout_face_mut(face).insert(pos, ...)`
  へコミットする（`commit_pending_layout_edit()`を経由しない独立経路）。ComboBoxの
  選択は「特定の選択肢がクリックされた」という離散的なイベントであり、IME合成を
  経由しないため、round1でテキスト入力について却下した`changed()`トリガーの欠陥
  （IME未確定文字列での誤発火）はここでは発生しない。これにより
  `layout_edit_special_idx`は「ラジオへ切り替えただけで未操作のまま残っている値」
  としてコミットされることがなくなる（round3 R3-2解消）。**「なし」と同様、
  コミット直後に`select_layout_cell(pos)`を呼び直して各バッファを再同期する**
  （怠ると、後で種別ラジオを他の種別に戻したときに編集パネルの表示とモデルが
  恒久的に食い違う。round4 premortem R4-3解消）。
  **【実装時の訂正】** 当初「ComboBoxウィジェット自身の`response.changed()`が
  真になった時点で」と書いていたが、egui 0.31.1では`ComboBox::show_ui`
  （`show_index`とは異なる素の版）の`combo_box_dyn`実装が、内側の
  `selectable_value`のクリックを`Response::changed()`へ一切伝播しない
  （`mark_changed()`を呼ぶのは`show_index`のみ）。そのため上記のまま実装すると
  `response.changed()`が恒久的に偽となり、特殊キーがGUIから設定不能になる
  （round1 F2/premortem P2-4が別の実装で再発）——実装後のOpusコードレビューで
  発覚したblocker。実装では選択前後の`layout_edit_special_idx`比較に置き換えて
  変更を検知している（`commit_special_key_if_changed()`）。「ComboBoxの選択は
  離散的なイベントでIME合成を経由しない」という設計判断自体は変わらないが、
  「`response.changed()`で検知する」という実装手段が特定バージョンのeguiでは
  成立しなかった、という記録として残す。

`ValueKind::Special`を除く**`None`以外の3種別**（打鍵/リテラル/VK）については、
テキスト欄の内容が現在示している状態を`commit_pending_layout_edit()`が毎フレーム
評価し、`layout_edit_origin`（文字列と種別のタプル）から差分があればコミットする。
これにより「打鍵からリテラルへ種別を変え、テキスト欄に触れずに別セルへ移る」
といった、値ウィジェット自体は変更していないが種別だけ変えた操作も、次フレームの
コミット判定で正しく拾われる（round2 B3、round4 W2/R4-1で再発したことが判明し
再解消——「ラジオ操作そのものはコミットしない」という制約は「なし」「特殊キー」の
2種別に限定し、それ以外の種別には適用しないことで、B2とB3が両立する）。

**ADR-115の打鍵列（`CtrlChord`/`InlineSequence`/`MacroRef`）を含むセルは、
値ウィジェットに加え種別ラジオの行そのものも無効化し（`ui.add_enabled(false, ...)`）、
`commit_pending_layout_edit()`側でも`layout_edit_origin_is_sequence`による
無条件スキップ（ステップ2）で構造的に守る、二重の防御にする。** これらは現状も
「編集して確定すると`YabValue::Literal`へ降格し打鍵列としての意味を失う」という
既知の限界（ADR-115決定9(a)）を持つが、これまでは「明示的に適用ボタンを押す」
というワンクッションがあった。自動コミットにするとこのクッションが失われるため、
この限界に該当するセルは編集不能にし、変更したい場合は`.yab`を直接編集するよう
案内する。**当初v3は「`layout_edit_origin`との差分判定により、ウィジェットの
無効化に頼らず自然にコミット対象から外れる」という設計だったが、`layout_edit_origin`
にround4 W2/R4-1の修正で`ValueKind`を含めたことにより、種別ラジオ（無効化して
いなければ）を切り替えるだけでタプルが乖離しコミットされてしまう経路が新たに
生まれた（round4 architect W3/premortem R4-2）。** これがラジオも含めて無効化し、
かつ`layout_edit_origin`のタプル比較に頼らない独立の`layout_edit_origin_is_sequence`
ガードを置く理由——**「UIを無効化するだけでは構造的な防御にならない」
（round3 architect V1）という原則と、「実際にUIも操作不能にして誤操作の経路を
物理的に塞ぐ」という2つの防御を、片方だけでなく両方とも満たす。**

**この種別ラジオの無効化により、ADR-115打鍵列セルは「なし」への切り替えも
できなくなるため、D1の「このキーの割り当てを解除」ボタン（表示条件が
`layout_edit_kind == ValueKind::None`）にも到達できず、GUIからこれらのセルの
割り当てを解除する手段が失われる（round5 architect X1）。** 従来は「セルを
選んで『なし』に切り替え→適用」で打鍵列も解除できていたが、この経路は
本ADRの範囲では復元しない——値の変更と同じく解除も`.yab`の直接編集に委ねる、
という一貫した扱いにする（値だけ編集不能で解除だけGUIに残す非対称な方が
かえって分かりにくいと判断する）。

**`select_layout_cell`の冒頭で`layout_status`をクリアし、`layout_edit_origin`
（文字列と種別のタプル）・`layout_edit_origin_is_sequence`を新しいセルの内容で
再設定し、`layout_edit_last_seen`を`None`に戻し、`ime_composing`を`false`に
リセットする。** 直前のセルのエラーメッセージが別のセルを編集中も残り続けることを
防ぐ（round2 A4）とともに、新しいセルのバッファ初期値が「まだ編集されていない」
状態として正しく扱われるようにする（round4 architect W6——`layout_edit_last_seen`の
リセット漏れを解消）。**【実装時の注意】** `select_layout_cell`がこのように
`layout_status`をクリアするため、`select_layout_cell(pos)`を呼んだ**後**に
`layout_status`へメッセージを設定しなければならない（先に設定すると即座に
消える）。`paste_layout_cell`（クリップボード貼り付け）が「解除」ボタン・
「特殊キー」ComboBoxの再同期対応（上記R4-3）の際にこの順序を誤り、
「貼り付けました」というメッセージが一切表示されない退行が実装後の
Opusコードレビューで発覚した。
**`commit_pending_layout_edit()`を経由せずに`self.layout`を変更するすべてのコード
パスは、変更直後にこの節の状態（`layout_selected_pos`・`layout_edit_value`・
`layout_edit_origin`・`layout_edit_origin_is_sequence`・`layout_edit_last_seen`・
`ime_composing`）を再同期しなければならない、というD1の不変条件とする（round4
premortem R4-3を踏まえ、round3 R3-3の不変条件の対象を「`self.layout`を丸ごと
差し替えるパス」から拡張したもの）。** 対象は次の6箇所:

1. `layout_do_reload`（既存実装で`layout_selected_pos = None`を実行済み、
   `main.rs:1014`）
2. `layout_load_from_path`（既存実装で同様、`main.rs:1038`）
3. D2で新設する「両方元に戻す」の独自ブロック（`layout_selected_pos = None`へ
   ——新規に満たす必要がある）
4. D1で新設する「このキーの割り当てを解除」ボタン（`select_layout_cell(pos)`の
   再実行で再同期——新規に満たす必要がある）
5. D1で新設する「特殊キー」ComboBoxの直接コミット経路（同じく`select_layout_cell(pos)`
   の再実行で再同期——新規に満たす必要がある）
6. 既存の`paste_layout_cell`（クリップボード履歴からの貼り付け、`main.rs:884-894`）
   ——`commit_pending_layout_edit()`を経由せず`self.layout`へ`insert`し
   `layout_modified = true`にする既存経路だが、末尾で既に`select_layout_cell(pos)`
   を呼んでおり、この不変条件を**既に満たしている**（round5 architect X4——
   将来この呼び出しが誤って削除されたときに検知できるよう、対象として明記する）

1・2は`self.layout`を丸ごと差し替えて選択を解除するのに対し、4・5・6は単一セルの
モデル値だけを`commit_pending_layout_edit()`の外側で書き換えるため、
「選択解除」ではなく「選択したままバッファだけを新しいモデル値に合わせて
再初期化する」（＝`select_layout_cell`の再実行）という異なる再同期が必要になる
点に注意する。これを怠ると、差し替え後も選択中セルの古いバッファが残ったままとなり、
次フレームの判定が新しい`self.layout`との差分（＝正当な変更）を「ユーザーの
新しい編集」と誤認して再コミットする（1〜3、round3 R3-3）か、逆に編集パネルの
表示とモデルが恒久的に食い違ったままになる（4・5、round4 R4-3）。

### D2: 配列編集タブの状態（`self.layout`/`layout_modified`）を、画面下部共通の「適用」「キャンセル」の対象に含める

- **配列読み込みの成否を、パスとは独立に追跡する。** `layout_loaded_ok: bool`
  フィールドを新設し、`layout_load_from_path()`の成功時にのみ`true`にする（失敗時は
  `false`のまま、または`false`に戻す）。**`layout_file_path`は読み込み成否に関わらず
  常に確定させてよい**（既定パス`resolve_layouts_dir(layouts_dir).join(default_layout)`
  を新規作成時の既定値として割り当てる、round1 F8の判断は維持）が、`.yab`への
  書き込みは`layout_modified && layout_loaded_ok`の両方が真のときのみ許可する。
  **【実装時の訂正】** `layout_file_path`を「読み込み成否に関わらず常に確定
  させてよい」は、`layout_file_path`が**まだ`None`**（起動直後の初回読み込み等、
  F8が想定する「新規作成時」）の場合にのみ適用する。既に有効なパスが設定済みの
  状態で「開く」「パス欄入力」が失敗した場合にまで無条件に新パスへ差し替えると、
  直前まで有効だった配列ファイルへの参照を失い、以後`F5`再読み込みも壊れた
  パスを見続ける退行になる（実装後のOpusコードレビューで発覚、
  round1 F8の意図はあくまで「保存先が未設定の状態を作らない」ことであり
  「既存の有効なパスを壊れたパスで上書きする」ことではない）。
  読み込みに失敗している間（壊れた`.yab`を開いた場合等）は、たとえ`layout_modified`が
  真でも`.yab`書き込みだけをスキップし、**`apply_confirmed()`全体は中止しない**。
  **【実装時の訂正】** 当初「`self.status`に…と表示する」という記述は
  `apply_confirmed()`全体を中止する実装と解釈されたが、これはround5 R5-1が
  確立した原則（配列編集タブに閉じた問題が無関係な設定の保存を巻き込んでは
  ならない）と矛盾する——壊れた`.yab`を開いた状態で他タブの設定だけを変更
  しても保存できてしまうべきである。ガード(B)と同じ扱いに揃え、
  `pending_status_notes`へ警告を積んでconfig.toml保存は続行する
  （round2 B1/R2-4、round5 R5-1解消）。
- **`.yab`書き込みとキーボードモデル比較は、`apply_confirmed()`内の
  `self.config = awase::config::AppConfig::from(validated)`（`main.rs:485`）より
  **後**、かつ`pending_save.is_some()`の早期return（`main.rs:461-468`）より**後**に
  置く。** `std::mem::take(&mut self.config)`から485行目までの間は`self.config`が
  `Default`値であり、ここでモデル比較を行うと誤判定する（round2 A1解消——最重要
  指摘として扱ったガードの実装位置を明記する）。
- **キーボードモデル不一致に関するガードを、目的の異なる2つの独立したガードに
  分離する（round4 architect W8/premortem R4-5解消——単一の条件式に混ぜていた
  ことが「正しい操作が理由不明で弾かれる」原因だった）。** いずれも検出したら
  `.yab`書き込み・config.toml保存の両方を中止する（部分適用を許さない。
  `.yab`だけ守ってconfig.toml側の保存を継続する（v1採用時の判断）は却下する
  ——それでもconfig.toml経由でエンジンに新モデル・旧配列という不整合が伝播する
  ため、round2 R2-5/R2-6）。

  **(A) データ保護ガード（配列編集タブの未保存の変更を守る）。** 配列を
  読み込んだ時点の`keyboard_model`を`layout_loaded_model: Option<KeyboardModel>`
  として保持する（`layout_load_from_path`/`layout_do_reload`の両方の**成功時に
  のみ**`Some(model)`に更新し、未読み込み・読み込み失敗中は`None`のままにする
  ——premortem R4-5、初期値の未定義を解消）。適用時、**`layout_modified`が真かつ
  `layout_loaded_model`が`Some(m)`かつ`m != self.config.general.keyboard_model`**
  のときにのみ発火し、「配列編集タブに未保存の変更がありますが、読み込み時と
  異なるキーボード配列（JIS/US）に変更されているため保存できません。キーボード
  配列を元に戻して適用するか、配列編集タブで変更を破棄してキーボード配列に
  合った配列を開き直してください」と表示する。目的は`YabLayout::serialize`が
  列数超過を**エラーにせず黙って列を落とす**ことによる`self.layout`書き込み時の
  データ消失を防ぐことだけであり、`layout_modified`が偽（＝これから何も書き込ま
  れない）なら発火する理由が無い——**round3で一度撤回した`layout_modified`要件は、
  W8/R4-5を受けてこのガード(A)に限り復活させる**（後述のガード(B)がW8/R4-5の
  本質的な原因だった「実際に壊れる操作を止められていない」を別途カバーする）。

  **(B) エンジン健全性ガード（`config.toml`保存後にエンジンが読む実ファイルを
  検証する）。** `layout_loaded_model`・`layout_modified`・配列編集タブを
  一度でも開いたかどうかに**一切依存せず**、適用しようとしている
  `resolve_layouts_dir(&self.config.general.layouts_dir).join(
  &self.config.general.default_layout)`が実際にディスク上に存在する場合、その
  内容を`YabLayout::parse(_, self.config.general.keyboard_model)`（＝適用後に
  有効になるモデル）で試しにパースする。**このガードは、`.yab`のディスク
  書き込みより前に評価する**（round5 architect Y1——`serialize()`は常に
  `self.config.general.keyboard_model`と整合した出力を作るため、書き込み前に
  検証しても偽陽性は出ない。「部分適用を許さない」という本ADRの一貫方針上、
  書き込んでから検証するのは順序が逆）。パースが失敗した場合の扱いは、
  **この適用でキーボード配列が実際に変わるかどうかで分岐させる**（round5
  premortem R5-1——分岐させないと、キーボード配列を一切変更していないのに、
  無関係な理由で既定配列ファイルが壊れているだけで設定画面全体が保存不能に
  なる新しい退行を生む）:
  - `config_loaded_model: KeyboardModel`（`SettingsApp::new()`時・`cancel()`
    実行時・`poll_pending_save`の保存成功〈`main.rs:550-562`〉時、**いずれも
    `AppConfig::load`（または保存）が成功した場合にのみ**更新する、直近に
    読み込み/保存が成功した時点のモデル——`cancel()`の`AppConfig::load`は
    失敗しうり〈`main.rs:610-613`のErr分岐、`Dangerous`〉、その場合
    `self.config`は変わらないため`config_loaded_model`も更新してはならない。
    `layout_loaded_model`と対称に「成功時のみ」を徹底する——premortem、
    実装時の注意点）と
    `self.config.general.keyboard_model`が**異なる場合**（＝この適用で
    キーボード配列が変わる）: `apply_confirmed()`全体を中止し、「現在の既定の
    配列ファイル（`default_layout`）が、これから適用するキーボード配列
    （JIS/US）で読み込めないため保存できません。`default_layout`をその
    キーボード配列に合ったファイルに変更するか、キーボード配列を元に戻して
    ください」と表示する（R2-6が懸念した「モデル切替だけでエンジンが壊れる」を
    確実に止める。architect W8の正しい同時変更——配列編集タブでJISを開いた
    **後に**全般設定タブで`keyboard_model=us`・`default_layout=nicola_us.yab`を
    正しく同時変更する——では、`nicola_us.yab`がUsで正しくパースできるため
    このガードは発火しない）。
  - 一致する場合（＝この適用ではキーボード配列を変更していない）:
    **`apply_confirmed()`は中止せず**、「既定の配列ファイル`{default_layout}`
    を読み込めません（{パースエラー}）。awaseエンジンはこの配列を読み込めない
    状態です」という警告を併記して保存処理を続行する。**この警告は
    `self.status`へ直接書き込まず、後述（本D2内）の`pending_status_notes`へ
    積んで`poll_pending_save`の成功メッセージに合流させる**（保存完了時に
    `self.status`が上書きされ、直接書き込んだ警告が消えてしまうことを防ぐ
    ——architect、実装時の注意点）。既に壊れている`.yab`をたまたま報告する
    だけで、この適用自体は新しい
    不整合を作っていないため（ADR-116の`scan_yab_files_for_diagnostics`が
    起動時診断として想定している状態そのものであり、その診断結果を見て
    修正しに来た画面が、壊れているという理由で保存不能になってはならない
    ——premortem R5-1）。
  - 対象ファイルが存在しない場合はこのガードをスキップする。存在しない
    `default_layout`自体への警告は、次の項目（`reload_config()`の実際の
    反映条件の警告）ではなく**ADR-116の`recompute_diagnostics`
    （`scan_yab_files_for_diagnostics`）およびエンジン側`LayoutEntry::
    scan_all`の`diag.warn`**が別途カバーする（round5 architect Y2——round1 F9の
    警告は`layout_file_path`が`default_layout`と一致しない場合の警告であり、
    `default_layout`自体の不存在は対象外なので参照先を訂正する）。
- **`awase.exe`のエンジンに実際に反映される条件を、適用時に警告する。**
  `reload_config()`は`resolve_layouts_dir(&config.general.layouts_dir)`配下かつ
  `config.general.default_layout`と同名の`.yab`しか読まない（round1 F9）。
  `apply_confirmed()`実行時、`layout_file_path`がこの条件を満たさない場合は
  `.yab`書き込み自体は行うが、`self.status`に「この配列ファイルは現在の配列
  フォルダ／既定の配列と異なるため、awaseエンジンには反映されません」という警告を
  併記する（round2 A2/R2-12解消）。
- **`.yab`書き込みの成否は`self.status`（画面下部、全タブ共通）に統合する。**
  `layout_status`（配列編集タブ内のみ）への書き込みは残してよいが、失敗時は
  `self.status`にも必ず反映する。成功時のメッセージも「設定を保存しました（配列
  nicola_j.yab を含む）」のように、実際に書いたものを列挙する。**この統合は
  `self.status`への直接上書きでは実現できない点に注意する（architect、
  実装時の注意点）。** `config.toml`の保存は非同期（バックグラウンドスレッド）
  であり、完了時に`poll_pending_save`が成功メッセージで`self.status`を
  上書きするため（`main.rs:550-553`）、`apply_confirmed()`内で同期的に
  `self.status`へ書いた`.yab`関連の警告（round1 F9由来の警告、上記ガード(B)の
  「警告して続行」時のメッセージを含む）は、保存完了の直後に消えてしまう。
  **`pending_status_notes: Vec<String>`のような、`apply_confirmed()`実行中に
  蓄積する警告の置き場を新設し、`poll_pending_save`の成功メッセージ生成時に
  これらを連結する**ことで、両者が同じ最終メッセージに合流するようにする。
- **`recompute_diagnostics()`の呼び出しを一本化する。** `layout_write_to_path`に
  「診断を再計算するか」の`bool`引数を追加し、統合後の適用経路からは`false`で呼ぶ
  （`apply_confirmed()`側の1回の再計算に任せる）。ツールバー等、既存の単独呼び出し
  経路は`true`のまま変更しない。
- **`apply_confirmed()`/`cancel()`双方を、`pending_save.is_some()`の間は対称に
  無効化する。** 「適用」ボタンだけでなく「キャンセル」ボタンも
  `ui.add_enabled(false, ...)`で実際に無効化し、両者とも「保存中です。少々お待ち
  ください…」を`self.status`に出す（round2 B6/C1、R2-11解消——押せるのに何も
  起きないボタンを作らない）。
- **`cancel()`を次のように再設計する:**
  1. `pending_save.is_some()`なら早期returnする（上記の対称無効化と合わせて、
     ボタン自体が押せないため二重の防御になる）。
  2. `layout_modified`が真なら、**3択の確認モーダル**を表示する:
     「設定だけ元に戻す（配列編集の変更は残す）」「両方元に戻す」「やめる」
     （round2 R2-7解消——二択では「設定だけ戻したい」という最も典型的な意図が
     満たせなかった）。「やめる」を選んだ場合は何もしない。
  3. 「設定だけ元に戻す」を選んだ場合: 現行の`cancel()`と同じくconfig.tomlを
     読み直すのみ。`self.layout`/`layout_modified`には触れない。
  4. 「両方元に戻す」を選んだ場合: config.tomlを読み直した**後**（`keyboard_model`
     等が復元された後）、その`keyboard_model`を使って`self.layout`を
     `layout_file_path`から読み直し、`layout_modified = false`にする（順序を誤ると
     復元前の`keyboard_model`で`.yab`をパースしてしまう）。**この読み直しが失敗した
     場合（`layout_file_path`のファイルが外部で削除・破損した等）は、通常の
     `layout_load_from_path`失敗時と同じ扱いにする**——`layout_loaded_ok = false`、
     `self.layout`は変更せず（＝直前まで画面に表示していた未保存の編集内容をその
     まま残す）、`self.status`に読み込み失敗を表示する。`layout_modified`は
     **falseにしない**（「両方元に戻す」は失敗しており、実際にはまだ未保存の編集が
     `self.layout`に残っているため。`layout_modified = false`にすると、その後
     D2の書き込みガードが「変更なし」と誤認し、壊れた状態のまま何もせず「適用成功」
     と表示しうる——architect V4）。いずれの場合も上記D1の不変条件どおり
     `layout_selected_pos`はNoneへ戻す。
  5. `layout_modified`が偽なら、確認モーダルを出さず現行どおり即座にconfigを
     読み直す。
  6. `show_dangerous_save_confirm`（既存のDangerous設定確認）は、上記の新しい
     確認モーダルとは独立に、**常に**（`cancel()`が呼ばれた時点で）falseにする
     既存の不変条件を維持する（`main.rs:4875-4887`のテストが固定している挙動）。
     2つの確認モーダルが同時に開くことは無い——`cancel()`が呼ばれた時点でDangerous
     モーダルは必ず閉じ、その**後**に（必要なら）新しい3択モーダルを開くという
     順序を守ることで、round2 R2-8の懸念（2つのモーダルの同時オープン）を回避する。

### D3: ツールバーの「保存」ボタンを撤去し、「名前を付けて保存」は「エクスポート」として再定義する

- ツールバーの「保存」ボタンは撤去する（D2により下部「適用」が実質的な保存を兼ねる
  ため冗長）。**`Ctrl+S`ショートカットは、`handle_layout_shortcuts()`（配列編集タブ
  限定）からではなく、`update()`のタブに依存しない箇所から`apply()`を呼ぶよう
  付け替える。** `handle_layout_shortcuts()`内に残したままだと「配列編集タブでだけ
  `Ctrl+S`が全体適用、他タブでは無反応」という新しい非一貫が生まれるため
  （round2 architect B7）。
- **`update()`冒頭の処理順序を次のとおり明記する（round3 architect V5、
  round4 architect W5/W7で以下のとおり補強）:**
  0. `ime_composing`の更新（D1。`ctx.input(|i| &i.events)`を走査するだけの
     軽量な処理で、`commit_pending_layout_edit`より前に置かないと1フレーム
     古い値で判定してしまう）
  1. `poll_pending_save`
  2. `commit_pending_layout_edit`（D1）
  3. グローバル`Ctrl+S`判定→`apply()`（本項目）
  4. `handle_layout_shortcuts`（配列編集タブ限定、`Ctrl+O`/`Ctrl+Shift+S`/`F5`のみ
     残る。`Ctrl+S`はステップ3に移設済みのため扱わない）
  5. `config_path_panel`（`main.rs:2573`）・`show_dangerous_save_confirm_modal`
     （同2574、「続行」で`apply_confirmed()`を直接呼ぶ）
  6. 各パネルの描画（`SidePanel`・`action_panel`・`CentralPanel`）

  この順序により、ステップ3のグローバル`Ctrl+S`は必ずステップ2でその時点の編集内容が
  確定した**後**に評価される。ステップ5を明示的に列挙するのは、
  `show_dangerous_save_confirm_modal`が独自に`apply_confirmed()`を呼ぶ経路であり、
  グローバル`Ctrl+S`（ステップ3）の挿入位置をこれより前にする必要がある——との
  順序関係を実装時に見落とさないため（round4 architect W7）。
- **グローバル`Ctrl+S`判定は、`self.capturing.is_some()`（ショートカットタブの
  キー捕捉モード、`main.rs:314`）の間は発火させない。** キー捕捉モード中は
  `process_keymap_capture()`が`Ctrl+S`を「Sキー＋Ctrl修飾」という捕捉対象の
  キーコンボそのものとして解釈する（`egui_key_to_internal`経由）。ここでグローバル
  `Ctrl+S`も同時に反応すると、ユーザーが「`Ctrl+S`という組み合わせを`[[keymap]]`の
  `from`に登録したい」だけなのに、意図せず設定全体が保存されてしまう。既存の
  `process_keymap_capture`が`Escape`（無修飾）だけを特別扱いしキャプチャを
  キャンセルする設計（`main.rs:1113-1116`）と対称に、「捕捉モード中は他の全ての
  グローバルショートカットより捕捉が優先される」というルールをステップ3の条件に
  明記する（round3 architect V6）。**同じ抑止条件（キー捕捉モード中に加え、
  Dangerous確認・3択キャンセル確認・配列破棄確認の各モーダル表示中）を、
  ステップ4の`handle_layout_shortcuts`（`Ctrl+O`/`Ctrl+Shift+S`/`F5`）にも
  適用する**（実装後のOpusコードレビュー指摘——グローバル`Ctrl+S`だけに
  条件を足すと、確認モーダルを開いた状態で`F5`を押した際に破棄確認モーダルが
  追加で開き、2つの非ブロッキング`egui::Window`が同一フレームに共存する。
  round2 R2-8と同型の退行）。
- **「名前を付けて保存」は「別ファイルへ書き出す（エクスポート）」として再定義し、
  `layout_file_path`と`layout_modified`を変更しない専用の処理にする。** 現状の
  実装（保存先を新しい「現在のファイル」として差し替える）のままD2と組み合わせると、
  「編集→名前を付けて保存でバリエーションを退避→適用」で元ファイルに何も書かれない
  まま「適用したのに反映されない」が再発するため（round1 F16）。ラベルも「別名で
  書き出す」等、「保存」という語彙を外した表記に変える。`Ctrl+Shift+S`はこの
  再定義後の関数を指すよう変更する。
- **`layout_load_from_path`を呼ぶ全経路に、`layout_modified`時の未保存確認を入れる。**
  洗い出した呼び出し元は次の4つ:
  1. `ensure_layout_loaded`（起動時の初回読み込み）——**確認不要**（この時点では
     編集していない）。
  2. `layout_do_open_dialog`経由（ツールバー「開く」、`Ctrl+O`）——確認する。
  3. `layout_do_open_from_text_box`（パス欄への直接入力＋Enter）——確認する
     （round2 R2-10、当初の列挙から漏れていた経路）。
  4. `layout_do_reload`（ツールバー「再読み込み」、`F5`）——確認する。
  2〜4はいずれも同じ「未保存の配列編集を破棄してよいか」の確認処理（D2の3択モーダル
  とは別の、単純な「破棄する/しない」の二択でよい——config.tomlは無関係なため）を
  経由するよう、共通のヘルパー関数に集約する。
  **【実装時の訂正】** この破棄確認モーダルの「破棄」ボタンハンドラで、
  実処理（ダイアログを開く／パス欄を再読み込む／再読み込みする）の**前に**
  `layout_modified = false`を無条件で立てる実装がされ、blockerとして
  Opusコードレビューで発覚した——`layout_do_open_dialog`のようにダイアログが
  非同期（`layout_pending_open`に結果が積まれるのを後続フレームで消費する
  設計）の経路では、ユーザーがファイル選択ダイアログを**キャンセル**すると
  何も置き換わらないまま`layout_modified`だけが偽になり、未保存の編集が
  「保存済み」であるかのように扱われる（D5バナーが消え、「適用」が`.yab`を
  書かなくなる）——本ADRの出発点である「適用したのに反映されない」を新しい
  経路で再生産する、最も重大な回帰だった。`layout_modified = false`は
  各読み込み関数（`layout_load_from_path`/`layout_do_reload`相当）の
  **成功時にのみ**行われるべきで、確認モーダル側では一切操作しない。

### D4: Scancode Map セクションは対象外とする

「背景」節のとおり、ADR-111決定4/7で確定済みの設計を変更しない。本ADRのスコープに
含めない。

### D5: 配列編集の未保存状態を、配列編集タブ以外からも視認できるようにする

D2により、他タブで「適用」を押しても配列編集タブの変更が(モデル不一致で保留された
場合を除き)書き込まれる。逆に「キャンセル」は他タブから押しても配列の変更を破棄
しうる（D2の3択確認モーダル経由）。**どちらの操作も、ユーザーが配列編集タブを見て
いない状態で実行されうる**ため、画面下部の`action_panel`に、`layout_modified`が
真のときだけ「配列編集に未保存の変更があります」という一行を追加する。「適用」
ボタンのホバーテキストにも「（配列編集の変更も保存されます）」を追記する。

**このバナーの文言は、D2のキーボードモデル不一致ガードで`apply_confirmed()`全体を
中止した場合の`self.status`メッセージと矛盾しないようにする（round3 architect V7）。**
モデル不一致で適用が中止された直後は「配列編集に未保存の変更があります」という
D5のバナーと、D2の「…保存できません」という中止メッセージの両方が同時に表示
されうる。前者だけを見ると「まだ保存されていないので改めて適用すればよい」と
誤解しかねないため、D2の中止メッセージ側に「（この変更は下部のバナーが示す
未保存の配列編集と同じものです）」等、両者が同一の状態を指していることが分かる
文言を含める。

### D6: ウィンドウを閉じる操作への対応は現状維持とする（意図的な決定、理由を明記）

`config.toml`側にも現状「ディスクとの差分」を検出する仕組みは無く（`self.config`が
最後にロード/保存した内容と一致するかの比較は行っていない）、ウィンドウクローズ時の
確認は`config`側・`layout`側のどちらにも実装しない。**この非対称（キャンセルボタンは
確認する、ウィンドウを閉じる×ボタンは確認しない）は意図的である**（round2 architect
B8の指摘に対する回答）: ウィンドウを閉じる操作はOSレベルで「保存せずに閉じる」という
一般的な期待を伴う一方、「キャンセル」ボタンは同じ画面内で「元に戻す」という復元を
明示的に期待させるラベルであり、両者に同じ確認義務を課す理由が無い。D5の可視化
インジケータと、D1の`update()`冒頭コミット（＝アプリを閉じるまでの間はどのタイミング
で操作しても直前の入力が失われない）で、ウィンドウクローズ時の実害を下げる。将来
`config`側にも同様の確認を入れる場合は、両者を対称に扱う別ADRとする。

## 未解決・実装時に確定させる点

- D2の3択確認モーダル・D3の二択確認（未保存の配列破棄）の具体的なUI実装
  （`show_dangerous_save_confirm_modal`と同様の非ブロッキングな`egui::Window`にするか、
  より確実にブロックする手段があるか）。3つの独立した確認モーダル
  （Dangerous設定確認・D2のキャンセル3択・D3の破棄確認）が状態として共存する場合の
  管理方法（単純な複数の`Option`/`bool`フィールドのままでよいか、1つの
  `enum PendingConfirmation`に統合すべきか）は実装時に決める。
- `commit_pending_layout_edit()`が`layout_edit_kind`ごとのバリデーションロジックを
  `apply_layout_edit()`から抽出・共有する具体的なリファクタリング方法。
- `test_settings_app`（`main.rs:4048-4049`）が`layout_file_path`に実際のリポジトリ内
  ファイル（`layout/nicola_j.yab`等）を指す状態でテストされているため、D2の書き込みが
  `layout_modified`（および`layout_loaded_ok`）でガードされていることを確認する
  回帰テストを追加すること（round2 architect A3）。最低限: (a) 変更ありで適用→一時
  ディレクトリの`.yab`に書かれフラグが落ちる、(b) 変更なしで適用→`.yab`の内容が
  変わらない（上書き事故そのものの回帰）、(c) キャンセルで編集が破棄されディスクの
  内容に戻る、(d) キーボードモデル不一致時に書き込み・config保存の両方が中止される、
  (e) 読み込み失敗中（`layout_loaded_ok == false`）は書き込みが行われない。
- 上記に加え、round3で判明した経路の回帰テスト: (f) ADR-115打鍵列セルを選択した
  だけ（テキスト欄に触れない）では`commit_pending_layout_edit()`が何もコミットしない、
  (g) `ValueKind::Special`のラジオへ切り替えただけ（ComboBoxを操作しない）では
  コミットされず、ComboBoxで選択肢を選んだ時点でのみコミットされる、(h) キャンセルの
  「両方元に戻す」または`layout_do_reload`実行後、選択中セルの古いバッファが復元後の
  モデルへ再コミットされない、(i) 無効な入力を残したまま他の操作をしても
  `layout_status`が毎フレーム上書きされ続けない（他の場所で設定したステータス
  メッセージが1フレーム以上残る）。IME合成中のコミット抑止（`ime_composing`）は
  実機のIME入力に依存するため自動テスト化が難しく、known-bugs.md相当の記録または
  手動確認手順の明記で代替してよい。
- round4で判明した経路の回帰テスト: (j) 種別ラジオだけを変更（テキスト欄には
  触れない）してから別セルへ移ると、新しい種別でコミットされる（round2 B3/
  round4 W2/R4-1の再発防止——`layout_edit_origin`のタプル比較が正しく動作する
  ことの確認）、(k) ADR-115打鍵列セル（`CtrlChord`/`InlineSequence`/`MacroRef`）を
  選択し種別ラジオだけを操作しても、`layout_edit_origin_is_sequence`により
  コミットされない（round4 W3/R4-2——(j)のテストが通るよう`ValueKind`を
  originに含めた結果、この経路が再度壊れていないことを確認する対になるテスト）、
  (l) 「このキーの割り当てを解除」または「特殊キー」ComboBoxでコミットした直後に
  種別ラジオを他の種別へ切り替えても、コミット直前の（解除後/選択後の）モデル値が
  正しくoriginとして扱われ、意図しない値が再コミットされない（round4 R4-3）、
  (m) IMEで変換し無変換のまま確定した場合に、確定後の文字列が正しくコミットされる
  （round4 W1——実機のIME入力に依存するため自動テスト化が難しければ、
  `ctx.input_mut`等でIMEイベント列を直接注入するegui向けのテストヘルパーで
  代替するか、(i)と同様に手動確認手順の明記で代替する）。
- round4のW8/R4-5で判明したガード分離の回帰テスト: (n) 配列編集タブを一度も
  開かずに（`layout_loaded_model == None`）全般設定タブのみで`keyboard_model`と
  `default_layout`を対応する組へ正しく同時変更した場合、ガード(A)は発火せず
  （`layout_modified`が偽）、ガード(B)も発火しない（新しい`default_layout`が
  新しい`keyboard_model`で正しくパースできる）ため適用が成功する（W8の
  repro自体の再発防止）、(o) 配列編集タブでJISの`.yab`を開いた**後**に、
  全般設定タブだけで`keyboard_model`と`default_layout`を対応する組へ正しく
  同時変更した場合も同様に適用が成功する（`layout_loaded_model`が`Some(Jis)`に
  なっていてもガード(A)は`layout_modified`が偽なので発火しないことの確認——
  architectの具体的なrepro手順そのもの）、(p) 配列編集タブで編集した後
  （`layout_modified = true`）、全般設定タブでキーボード配列だけ変更して
  `default_layout`を変えずに適用すると、ガード(A)が発火し中止される
  （データ保護ガードの本来の役割）、(q) `default_layout`が指す`.yab`が
  存在しない場合はガード(B)をスキップし、ADR-116の`recompute_diagnostics`
  （`scan_yab_files_for_diagnostics`）による既存の警告表示に委ねる。
- round5のR5-1で判明したガード(B)の回帰テスト: (r) `default_layout`が指す
  `.yab`が構文的に壊れている状態で、キーボード配列を変更せずに（他のタブの
  設定のみ変更して）適用すると、`apply_confirmed()`は中止されず`config.toml`
  の保存は成功し、`self.status`に既定配列ファイルが読み込めない旨の警告が
  併記される（premortem R5-1、既存動作からの退行防止）。

## 範囲外

- `tab_app_rules`（画面から到達不可、config.toml手動編集に委ねる既存の意図的設計）。
- Scancode Map セクション（D4）。
- 配列編集以外の既存タブの実装変更（監査の結果、既に原則に一致しているため不要）。
- `config.toml`側の「ディスクとの差分」検出・ウィンドウクローズ時の確認（D6、将来の
  別ADR）。
- ADR-115打鍵列セルの編集機能そのものの拡充（D1では「編集不能にする」という後退的
  対応に留め、打鍵列を安全に編集できるUIの新設はスコープ外とする）。**種別ラジオ
  行の無効化に伴い、これらのセルをGUIの「割り当てを解除」ボタンで解除する経路も
  同様にスコープ外とする（round5 architect X1）。値の変更・解除のいずれも
  `.yab`の直接編集に委ねる。**
- IME変換中の打鍵欄における赤エラー表示のちらつき（round1 P2-2/P2-3で指摘）。
  **（round3で修正・記述更新: `ime_composing`ガード導入前のv2時点では未解決だった
  「変換中の中間文字列がモデルへ書き込まれる」という実害は、D1の`ime_composing`
  ガードにより解消済み。ここでいう「ちらつき」は、変換完了までパースエラー表示等が
  一時的に切り替わって見える表示上の見た目のみを指し、モデルへの実害は無い——
  premortem R3-5）。**
- 打鍵欄の`resp.changed()`で即座に呼ばれる`normalize_keystroke_input()`
  （`main.rs:2237-2239`、round1 P2-3）がIME合成中のバッファを横から書き換える
  経路は、D1に追記した`ime_composing`ガードにより解消するが（上記D1「打鍵欄の
  `resp.changed()`で即座に呼ばれている…」の項を参照）、これは`ime_composing`
  導入の副次効果として直すものであり、単独の検証項目としては追わない。

## 関連

- ADR-111（Caps⇔Ctrl入れ替え、Scancode Map方式の決定4/7——本ADRが変更しない部分）。
- ADR-115（.yab打鍵列機能、決定9(a)「編集UIでの表示・編集時の限界」——D1がこの限界を
  悪化させないための追加対応の根拠）。
- ADR-116（起動時設定診断、`recompute_diagnostics()`——D2で呼び出し回数を一本化する
  対象）。
