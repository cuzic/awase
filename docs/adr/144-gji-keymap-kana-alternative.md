# ADR-144: Google日本語入力キーマップによるかな系キー入れ替え代替案（検討・比較用）

## ステータス

**検討・比較用（2026-09-06起票、実装しない前提の記録）。**

ADR-141（物理キー役割代入基盤、r3収束・未実装）・ADR-142（config/GUI、
収束・未実装）・ADR-143（かな系キー役割代入、r25収束・未実装）に対する
代替設計案として、ユーザーの疑問「ここまでやらなくても Google 日本語入力を
前提とした上で、GJI 上で適切に設定を入れ替えるような補助ツールを提供する
方が 80% ニーズは満たせるし簡単だったのでは」に答えるために、GJI(Mozc)
キーマップ機構の実際の能力を調査した上で起票する。**「MS-IME非対応」
「ADR-141の3キー汎用入れ替えは代替不可」というスコープ制約により、
awaseの現行方針（IME非依存・全単射モデル全体の代替）としては不採用と
判断するが、GJI専用に限定した「かな系キーのIME挙動入れ替え」1機能に
限れば、Mozc本体のソース確認により技術的な実現性は高いことが判明した**
（決定3・[ADR-145](145-scancode-map-kana-alternative.md)末尾の比較参照）。

**追記（2026-09-06）**: 決定3が最後に残していた不確実性（DirectInput
状態での変換キー配送）をユーザーが実機で確認し解消したため、この
GJIキーマップ方式を実際に実装する方向へ進むことになった。具体的な
ツール設計は[ADR-146](146-gji-keymap-swap-tool.md)を参照。

## 背景

ADR-141/142/143は、物理キー（変換・無変換・スペース・かなスロットの
4要素の全単射）の役割を、awaseのWin32低レベルフックでvkを書き換えることで
入れ替える。かなスロットについては物理→他キーへの代入（scan値ベースの
from判定）と、他キー→かなスロットへの代入（sentinel VK経由でIME
actuationを起動するto方向）の両方を扱い、さらに「Shift+代入先キーで
カタカナへ切り替える」機能まで実装した（decision7）。この設計は
Opus 2体（`adr143-architect`/`adr143-premortem`）による敵対的レビューを
25ラウンド超（ADR-141/142と合わせるとさらに多い）実施して収束したが、
2026-09-06時点でコードは1行も実装されていない。

ユーザーはこの経緯を振り返り、「GJI利用を前提にできるなら、GJI自身の
設定を入れ替える補助ツール（例: keymap.txtを生成して配布する、または
GUIでキーマップエディタ相当の操作を代行する）の方が、はるかに小さい
実装コストで大半のニーズ（かな⇔変換等の入れ替え）を満たせたのでは
ないか」と問うた。これに答えるため、Mozc(GJI)のキーマップ機構が
実際に「物理キーの役割入れ替え」を代替しうるかを調査した。

## 調査結果（`gji-keymap-capability-research`エージェント、2026-09-06）

1. **キーマップの形式**: Mozcのキーマップは、入力モード（DirectInput /
   Precomposition / Composition / Conversion / Suggestion / Prediction）
   ごとに、修飾キー+VKの組み合わせにコマンド（`IMEOn`/`IMEOff`/
   `InputModeHiragana`/`InputModeFullKatakana`等）を紐付ける形式。GUIの
   キーマップエディタ、またはタブ区切りテキスト（`keymap.txt`、
   `Mode\tKey\tCommand`、複合コマンドは`|`区切り）でインポート/
   エクスポート可能。
2. **DirectInput状態でのIME ON割当は確認済み**: `keymap_editor.cc`の
   ソース確認により、DirectInputモードでも`IMEOn`コマンドは常時登録
   されている（macOSのみ非表示だが機能自体は有効）。Google公式ヘルプにも
   「直接入力時に特定キーでIMEを有効化する」設定例が案内されている。
   つまり「物理キー押下でIME ONにする」という、ADR-143 decision2
   （to=かな方向の一部）が実現した挙動の**一部はGJI自身の設定だけで
   既に可能**。
3. **Shift+かな→カタカナ相当は有望だが未検証**: ADR-143 decision7の
   「Shift+代入先キー→カタカナ」はIME ON状態（Precomposition/
   Composition）でのコマンド発火であり、DirectInputの制約対象外。
   かな系キーのShift+カタカナ切替はGJIのデフォルトキーマップ自体が
   持つ既存挙動である可能性が高く、これを「変換キー」等の別VKにも
   同じコマンドを追加バインドするだけで実現できる可能性が高い。ただし
   実際にキーマップファイル内で該当バインドが存在するか、Windows版で
   確証は取れていない（推測の域を出ない）。
4. **入力側キーの指定範囲は確認済み（当初「未確認」としていたが解消）**:
   Mozc本体のキーパーサー実装
   ([`src/composer/key_parser.cc`](https://github.com/google/mozc/blob/master/src/composer/key_parser.cc)、
   `kSpecialKeyMap`)を直接確認した。かな系キーはすべて有効なトリガー名
   として受理される: `henkan`(HENKAN)、`muhenkan`(MUHENKAN)、
   `kana`/`hiragana`(いずれも`KeyEvent::KANA`の別名——`GetSpecialKeyString`
   が`KANA`を`"hiragana"`という文字列表現に変換して出力することも確認)、
   `katakana`(`KATAKANA`、`KANA`とは別の特殊キー)、`eisu`(EISU)、
   `hankaku`/`zenkaku`/`hankaku/zenkaku`(いずれも`HANKAKU`)、`kanji`(KANJI)。
   `keymap_editor.cc`のGUI側で`KANJI`/`ON`/`OFF`が非表示になっているのは
   **GUIの表示上の制限**であり、`keymap.txt`をテキストで直接編集する
   経路ではパーサー自体がこれらのキー名をすべて受理する。**つまり
   「変換キー(henkan)が押されたときにIMEOn/InputModeHiragana等の
   コマンドを発火させる」というキーマップ行は、パーサーの制約としては
   何の問題もなく記述できる**——これがADR-143 decision2/decision7の
   from=変換→to=かな方向の入れ替えを、GJIキーマップだけで表現できない
   理由にはならないことが判明した。
   
   ただし、これはあくまで**Mozcの設定パーサーがそのキーマップ行を
   受理できるか**の確認であり、**Windowsが実際にDirectInput（IME OFF）
   状態で変換キーのKeyDownをMozcエンジンへ配送するか**は別の問題として
   残る（決定2参照）。
5. **残る懸念点（本ADRを不採用と判断する根拠）**:
   - **MS-IME非対応**: キーマップはMozc内部機構のためGJI専用。awaseは
     IME非依存（MS-IME/GJI両対応）を掲げており、これを切り捨てることは
     ADR-143固有の問題ではなくawase全体の設計方針との衝突になる。
   - **アプリ種別依存性が未検証**: TSF-native（Chrome/VS Code等）/
     ImmCross/legacy IMM32のいずれで動作していても、OSが生キーをGJI
     プロセスへ配送する経路自体は同じはずだが、awaseが観測している
     「ImmCross/GjiDirect/MsImeDirect」のような戦略差がGJIキーマップ層
     にも影響するかは未確認。
   - 設定は `keymap.txt` の手動編集かキーマップエディタのGUI操作が必要で、
     一般ユーザーには敷居がある。awaseの設定GUI・エンジンON/OFF切替・
     全単射モデルとは統合されない（後述、決定4）。

## 想定する設計（仮に採用する場合の骨子）

- awase側のhook層・engine層には一切手を入れない。
- awase-settings（または単独の補助ツール）に、ユーザーが選んだ入れ替え
  パターン（例: 変換⇔かな）から `keymap.txt` の該当エントリ（DirectInput
  モードの `IMEOn`、Precomposition/Composition モードの `InputModeHiragana`/
  `InputModeFullKatakana` 等）を生成し、Mozcの設定ディレクトリへ配置する
  か、ユーザーにインポート手順を案内する機能を追加する。
- GJIの再起動またはプロセスの設定リロードで反映（Mozc側の挙動、awase側の
  制御は及ばない）。

## 決定1: MS-IME非対応をスコープ外として受け入れるかどうかは本ADR単独では決められない

awaseはMS-IME/GJI両対応を前提としたIME非依存アーキテクチャ
（`AppKind`/`ImeRelevance`等）を核としており、「GJI専用の便利機能」を
追加すること自体は既存の他機能（例: BUG-25のGJI半角英数entry）と
同型で問題ない。しかし「かな系キー入れ替え機能そのものをGJI専用にする」
という判断は、ADR-141/142/143が最初から目指していた「IME非依存の
物理キー入れ替え基盤」というスコープを狭める製品判断であり、本ADRの
技術調査だけでは決定できない。

## 決定2: 汎用3キー入れ替え（ADR-141）はGJIキーマップで代替できない

ADR-141が提供する「変換・無変換・スペースの3キー同士の任意入れ替え」は
IMEコマンドとは無関係な**物理キーの役割そのものの入れ替え**（かな入力に
一切関与しないキーも対象）であり、Mozcキーマップは「IMEコマンドの
発火条件」を変えるだけで物理キーの意味そのものを差し替える機構ではない。
したがってGJIキーマップ案は**ADR-143（かな系拡張）の代替にはなり得ても、
ADR-141（3キー基盤）の代替にはならない**。ここはユーザー自身が指摘した
「打鍵後の一連の処理の流れをリファクタする作業、基板側の整理そのものには
価値がある」という観察と一致する——ADR-141の価値はADR-143のかな系拡張とは
独立に評価すべきである。

## 決定3: 残るのは「パーサーが受理するか」ではなく「Windowsが配送するか」という別レイヤーの問題

`key_parser.cc`の確認により、**キーマップの表現力そのものは制約になら
ない**ことが判明した。残る技術的不確実性は、Windows環境でMozc(GJI)の
IMEプロセスが、DirectInput（IME OFF）状態で変換キーのKeyDownイベント
自体を受け取れるか、という**Windows側のキー配送経路**の問題に絞られる。
これはMozc自身のコードだけでは判定できず（IMM32/TSFとOSのキー配送規約
に依存する）、実機検証が必要（未解決の疑問1）。ただし決定的観測2
（GJI公式ヘルプが「直接入力時に特定キーでIMEを有効化する」設定例を
案内している）は、**少なくとも何らかのキーについてはDirectInput状態
でもMozcへの配送が現に機能している**ことを示しており、変換キー
（henkanトリガー）が例外的に配送されない理由は特定できていない。

## 却下理由（本ADRを不採用とする理由）

1. MS-IME非対応がawaseの中核方針（IME非依存）と衝突する。
2. ADR-141の3キー汎用入れ替え自体は代替できず、部分的な代替にしかならない。
3. エンジンON/OFF連動（`engine_enabled`/`kana_cycle_active`、ADR-143
   decision3）に相当する動的切替はGJIキーマップの静的設定では実現できない。
4. Windows側のキー配送経路の実機検証（決定3・未解決の疑問1）が未実施
   であり、決定的観測2の傍証はあるものの確定はしていない。

**理由1〜3はADR-143の「かな系拡張」機能そのものをGJI専用に縮小しない限り
解消しない構造的な制約であり、理由4のように実機検証で解消しうる不確実性
とは性質が異なる。**

## 採用を再検討する条件

- 実機でMozcの`keymap.txt`に`henkan`等のかな系トリガーキーを登録し、
  DirectInput状態で実際にコマンドが発火するか検証できた場合（決定3・
  未解決の疑問1）。パーサーの受理自体は確認済みのため、この検証は
  「動くかどうか」ではなく「Windows側の配送経路がこのキーを対象に
  含むか」に絞られる。
- MS-IMEユーザーへの別解決策（例: 単純な機能縮小の告知、ADR-145の
  ScanCode Map案との併用）が製品判断として許容される場合。

## 未解決の疑問

1. Windows版GJIが、DirectInput（IME OFF）状態で変換キー（`henkan`
   トリガー）のKeyDownイベントをMozcエンジンへ実際に配送するか。
   決定的観測2（IMEOn設定例の公式案内）は一部のキーでの配送を示唆する
   が、変換キー個別の実機検証は未実施。
2. Windows版Mozcの実装は、DirectInput状態での`InputModeXXX`系コマンドを
   OSS版同様に無効化しているか（[mozc issue #246](https://github.com/google/mozc/issues/246)
   はOSS本体の挙動であり、Windows版の別実装テーブルの有無は未確認）。
   ただしShift+代入先キー→カタカナ機能はPrecomposition/Composition
   モード（IME ON状態）での発火であり、この制約の対象外（決定的観測3参照）。

## 比較

ADR-141/142/143との比較は [ADR-145](145-scancode-map-kana-alternative.md)
末尾の比較表を参照（3案まとめて記載）。

## 関連ファイル

[Mozc keymap_editor.cc](https://github.com/google/mozc/blob/master/src/gui/config_dialog/keymap_editor.cc)、
[Google 日本語入力ヘルプ（一般設定）](https://support.google.com/ime/japanese/answer/166764?hl=ja)、
[Mozc Keymap Editor（サードパーティ）](https://azishio.github.io/mozc_keymap_editor/)。
関連ADR: 141, 142, 143, 145。
