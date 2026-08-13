# ADR-091: 冪等キー中心のIME制御 — open/romaji/charset 3軸の結論、GJI推奨・Muhenkan用F21-F23設計、MS-IME自己責任ポリシー

## ステータス

**決定・実装未着手。charset軸(GJI)のみ実機でconv bits直接読み取りによる検証済み。**

本ADRは、awaseのIME状態制御の設計思想を「awase自身がIME状態を能動的にトグル管理する」から
「基本的にキーはパススルーし、IME状態を動かすキーは冪等(絶対指定)キーだけを使う」へ転換する、
という2026-08-13のセッションでの方針転換を記録する。open/romaji軸は既存の対応を追認するのみ、
charset軸が本ADRの新規決定の中心である。

**本ADRは、同セッション内で先行して5ラウンドのFable×Opus pre-mortemを経た`CharsetSlot`設計
(物理DBEキー押下をbeliefに基づき冪等に再発行する機構、当初MS-IME向けとして設計)の対象を
GJI向けへ付け替える。** MS-IMEはcomposition中判定が不能なため安全な変換ができず、
GJIはF21-F23という実証済みの変換先とstatus別解釈の委譲により判定不要で成立する
(当初F15-F19を計画したが、実機でのターミナル漏洩不具合を受けF21-F23へ変更。
Eisu対応は範囲縮小のため見送った、決定3参照)。
経緯は§4に記す。

---

## 1. 背景

### 1.1 IME状態は3つの独立した軸である

awaseが相手にしている「IMEの状態」は、実際には次の3軸である(ADR-088 §2.4の軸分解を継承)。

- **軸1: open/close** — IMEコンポーネント自体が有効かどうか。
- **軸2: romaji**(入力方式: ローマ字入力/JISかな入力) — IMM32の`IME_CMODE_ROMAN`ビット。
- **軸3: charset**(ひらがな/カタカナ/英数 × 全角/半角) — `IME_CMODE_NATIVE`/`KATAKANA`/`FULLSHAPE`
  ビットの組み合わせ。5値: ひらがな/全角カタカナ/半角カタカナ/全角英数/半角英数。

### 1.2 open軸は既に決着している(変更しない)

`VK_IME_ON`(0x16)/`VK_IME_OFF`(0x1A)は単一効果・冪等キーとして全環境で動作確認済み
(ADR-067、config1.dbバインド不要)。Imm32Unavailableプロファイル(Chrome/Edge)向けの
フォールバックとして残る`VK_KANJI`(真のトグル)の発火は、物理キー押下ではなく
awase内部の`OpenWarrant`(belief)駆動であり、「物理トグル→belief算出→冪等キー送信」への
変換はopen軸については実質完了している(BUG-50修正で`VK_DBE_HIRAGANA`→`VK_IME_ON`へ移行済み)。

### 1.3 romaji軸は制御不能と確定している(抑止するしかない)

BUG-61(`docs/known-bugs.md`)で、Win32にはローマ字入力⇔JISかな入力を外部から切り替える
公式APIが存在しないと実機検証で確定済み(IMC write単体・VK注入・scan固定いずれも無反応。
`SendInput`が`LLKHF_INJECTED`付きイベントを合成入力とみなし、MS-IMEのAlt+かなハンドラが
意図的に無視すると推定)。

**決定: awaseはJISかな入力方式を非サポートとする。** 物理Alt+かな押下(OSのキーボード
レイアウトドライバが`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`を合成送信する経路)は、BUG-62で
導入済みの予防的swallow(`hook.rs:793`、`swallow_alt_kana_input_method_switch`)を
維持する。これは対症療法ではなく、「romaji軸は復旧不能なので遷移トリガー自体を
未然に遮断する」という明示的な設計原則として本ADRで追認する。

### 1.4 charset軸: GJIとMS-IMEで到達可能性が全く異なる

本ADRの本題。2026-08-13のセッションで、以下の調査・実験を行った:

1. **BUG-25の再検証**: 「GJIへのSendInputは届かない」という従来の理解は、
   `VK_DBE_ALPHANUMERIC`(0xF0)という**1つの非標準VK**の3回連続失敗のみを根拠にした
   過度な一般化だった。標準ファンクションキー(F21/F22、config1.dbバインド)は
   長年SendInputで確実にGJIへ届いていた実績があり、SendInputという手段自体が
   GJIに効かないわけではない。
2. **`crates/awase-gji-config`の再確認**: `GjiModeCommand::SetMode(GjiCompositionMode)`
   として、GJIの`custom_keymap_table`には「現在の状態に関係なく特定モードへ直接遷移する」
   絶対設定コマンド(`CompositionModeHiragana`/`FullKatakana`/`HalfKatakana`/
   `FullAlphanumeric`/`HalfAlphanumeric`)が既に存在すると判明。
3. **実際のconfig1.db抽出**: ユーザー実機の`config1.db`をclipwire経由で取得し、
   `wire.rs`のprotobuf最小パーサで`custom_keymap_table`(TSV、161行)を復元。
   **F13(DirectInput→IMEOn)・F14(Precomposition/Composition/Conversion→IMEOff)・
   F21/F22(IME ON/OFF)に旧awase由来の残骸バインドが実在する**ことを確認、除去した。
   ADR-057が「物理キーが存在しない安全域」と確認したF15-F19に、5つの
   `CompositionMode*`コマンドを新規バインドし、実際にGJIへインポート済み。
4. **自前IMM32プローブによる実機検証**(`crates/awase-windows/examples/gji_composition_probe.rs`):
   メモ帳を対象にしたPowerShell版(clipwire経由)は、Windows 11のストアアプリ化に
   よるウィンドウハンドル不整合と`SetForegroundWindow`のフォアグラウンド強奪制限で
   信頼できない結果しか返さなかったため、自前のWin32ウィンドウ(EDITコントロール)を
   作成し、`ImmGetConversionStatus`でconv bitsを直接読み取るプローブに切り替えた。
   **実機結果(2026-08-13、再現性確認済み)**:

   | キー | コマンド | 観測conversion_mode | 期待ビット | 一致 |
   |---|---|---|---|---|
   | F15 | CompositionModeHiragana | 9 | NATIVE\|FULLSHAPE | ✅ |
   | F16 | CompositionModeFullKatakana | 27 | NATIVE\|KATAKANA\|FULLSHAPE\|ROMAN | ✅ |
   | F17 | CompositionModeHalfKatakana | 19 | NATIVE\|KATAKANA\|ROMAN | ✅ |
   | F18 | CompositionModeFullAlphanumeric | 24 | FULLSHAPE\|ROMAN | ✅ |
   | F19 | CompositionModeHalfAlphanumeric | 16 | ROMAN | ✅ |

   5モード全てで、テキストの見た目ではなく`ImmGetConversionStatus`という権威あるAPIの
   読み取り値が期待通りに一致した。**F15-F19 + config1.db `CompositionMode*`バインドは、
   GJIのcharset軸を冪等に絶対制御する、実証済みの機構である。**

   なお候補ウィンドウ表示中(henkan)シナリオでは、F15押下後にEnterで確定した結果に
   予期しない漢字変換("居合"等)が混入する現象が観測された。プローブ側のシナリオ間で
   IME未確定compositionのキャンセルを行っていなかったことによる測定汚染の疑いが強く、
   核心の結論(conversion_modeの直接一致)には影響しないが、変換中の詳細な挙動を厳密に
   詰めるには追試が要る(§5 未解決参照)。

   **注記**: この検証は「F15-F19にCompositionMode*を全状態で統一バインドする」という
   単純な構成で行い、SendInputでF-keyを送ればGJIのconv bitsを確実に動かせるという
   **機構自体**の実証が目的だった。その後、この構成のままターミナルアプリへ
   エスケープシーケンスが残留する不具合が実機で見つかり、設計を「Precomposition=
   絶対指定/Composition=ネイティブトグル」の二段構成・キー番号もF21-F23へ
   変更した(§3.2)。本節の検証はF15-F19という危険な番号を使った点を除けば、
   基礎機構(SendInputの到達性・`ImmGetConversionStatus`での確認可能性)の
   証明としては有効であり、§3.2の二段構成・新番号での実機検証はPhase 2(§5)で
   改めて行う。

5. **MS-IMEの物理DBEキー再発行機構(`CharsetSlot`)は撤回**: 詳細は§4。

---

## 2. 決定

### 決定1(open軸): 現状を設計原則として固定する。変更しない。

§1.2の通り。VK_IME_ON/OFFが冪等キーとして機能し、VK_KANJIはImm32Unavailable向け
フォールバックとしてbelief駆動で残る。

### 決定2(romaji軸): 制御しない。抑止する。

§1.3の通り。JISかな入力方式は非サポート。物理Alt+かなは予防的swallowを維持する。

### 決定3(charset軸): GJIを推奨IMEとし、config1.dbバインドで制御する。Muhenkan単独打鍵のみ、3個の新設Fnキーで「belief強制」と「composition中のネイティブ挙動尊重」を両立する。

#### 3.1 スコープ: Muhenkan単独打鍵のみに絞る(Eisuは対象外)

**当初Muhenkan相当3キー+Eisu相当2キーの計5キー(F15-F19)を計画していたが、
実機でF15-F19使用時にターミナルアプリ(WezTerm等)へ不審なエスケープ
シーケンスが残留する不具合が判明したため、範囲を縮小した。**

原因は当初の想定違いだった。ADR-057は「F15〜F20も物理キーが存在しないが」
としつつ、実際には**F15-F20を意図的に避けてF21/F22を選んだ**——
「より高い番号ほど実使用例(≒ターミナルのエスケープシーケンス割当て)が
少ない」という経験則に基づき、実測で問題が無いことを確認できたのはF21/F22
だけだった(ADR-057:81-85)。F15-F19を安全域として扱ったのは、この経緯を
見落とした誤りだった。

mozc本家`src/composer/key_parser.cc`のキートークン語彙(`kSpecialKeyMap`)を
確認した結果、関数キーはF24が上限で、F23/F24はADR-057同様未実測、テンキー系
キー(`numpad0`-`9`/`multiply`/`add`/`separator`/`subtract`/`decimal`/
`divide`/`equals`/`comma`/`clear`)はF-keyのVT/xterm規約の対象外の可能性が
あるが未実測。マルチメディア/ブラウザ/音量キーはmozcのキートークン語彙に
そもそも存在せず、config1.dbで紐付け不可能なため使えない。

**結論: 実測確認済みのF21/F22の安全性だけに依拠し、範囲をMuhenkan単独打鍵
(3状態サイクル)だけに絞る。** Eisu(英数キー)のbelief強制は諦め、ネイティブの
`ToggleAlphanumericMode`にそのまま委ねる(決定4のMS-IME同様、パススルー+
自己責任的な扱い)。

#### 3.2 新設Fnキー: 3個(F21-F23)、Precomposition限定の絶対指定+Composition/Conversionはネイティブトグル

各Fnキーは、「Precomposition用の絶対指定ターゲット」と「Composition/
Conversion用のネイティブトグルコマンド(`SwitchKanaType`)」の2つを状態別に
持つ。GJI自身がPrecomposition/Composition/Conversionを区別してコマンドを
解釈するため、**awase側は「今composition中かどうか」を一切判定しない**
——物理`Muhenkan`押下時に現在のbeliefだけを見て、対応するFnキーを1つ選んで
送るだけでよい。

mozc本家`src/session/session.cc::Session::CompositionModeSwitchKanaType`を
直接確認し、ひらがな→全角カタカナ→半角カタカナ→(循環)という3状態サイクル
であることを検証済み(英数系モード中はこのコマンド自体が無効/no-opである
ことも確認済み)。「beliefがXのとき、次の状態Yへ絶対指定で進める」キーを、
サイクルの3つの出発点それぞれについて用意する:

| Fn | 使用条件(現在のbelief) | Precomposition | Composition/Conversion |
|---|---|---|---|
| F21 | belief=ひらがな | `CompositionModeFullKatakana`(絶対指定) | `SwitchKanaType`(ネイティブ) |
| F22 | belief=全角カタカナ | `CompositionModeHalfKatakana`(絶対指定) | `SwitchKanaType`(ネイティブ) |
| F23 | belief=半角カタカナ | `CompositionModeHiragana`(絶対指定) | `SwitchKanaType`(ネイティブ) |

F21/F22はADR-057でWezTermにおける実測でエスケープシーケンスを生成しないと
確認済み。**F23は未実測**(§5 Phase 2で実機確認すること)。F13/F14/F24は
使わない(F13/F14はADR-057当時ターミナルへのエスケープシーケンス漏れ・
DirectInputゲーム競合で捨てられたキー、F24は実測情報が無い)。旧awase
config1.db管理機構[ADR-067で削除済み]によるF21/F22の残骸バインドは、
本ADRの作業中に実機で確認・削除済み(§1.4項目3)。

DirectInput状態(IMEが閉じている)については、mozc本家`ms-ime.tsv`で
Muhenkan/Shift+Muhenkanがそもそも一切バインドされていない(押しても
何も起きず、IME ONにすらならない)。新設Fnキーもこれに倣い、DirectInput
状態でのバインドは設けない。

#### 3.3 物理トリガーとCharsetSlotのロジック

**ケアするのは物理`Muhenkan`(単独)キーのみ。** `Eisu`・`Shift+Muhenkan`は
CharsetSlotの対象外とし、GJIのネイティブな`ToggleAlphanumericMode`/
`ConvertToFullAlphanumeric`にそのまま委ねる(パススルー、動作保証はしない)。

**`Hiragana`/`Katakana`物理キーも対象外**(既に絶対指定でbelief乖離リスクが
無いため、awase側での介入・swallow自体が不要。BUG-52対応の無条件Suppressから
この2キーを除外することを検討する、§5 Phase 1)。`Henkan`(変換キー)は
`Reconvert`という別機能でcharsetのモード切替とは無関係、対象外。
`Hankaku/Zenkaku`/`Kanji`/`ON`/`OFF`はopen軸(決定1の範囲)で対象外。

送信後、awaseは自身のbeliefを「送ったFnキーが示す絶対指定ターゲット」に
楽観的に更新する(precomposition分岐が実際に発火した前提)。実際には
composition/conversion中でネイティブトグルが発火していた場合、beliefは
実状態とズレるが、composition文字列は画面に見えており低リスク(決定3の
根底にある原則、§1.4参照)。beliefが英数系(全角/半角英数)にある場合、
Muhenkanの循環コマンド自体がno-op(§3.2)なので、CharsetSlotはこの場合
何もしない(Eisu状態からの復帰はネイティブのEisuキー/Shift+Muhenkanに
委ねる、パススルー)。

#### 3.4 なぜMS-IME互換プリセットを基準にしたか(ATOK/ことえりとの比較)

mozc本家の3プリセットを比較した結果、この種のキーの意味論はIME文化ごとに
大きく異なると判明した:

- **MS-IME互換**(採用): Muhenkan=かな循環、Eisu(+Shift+Muhenkan)=ひらがな⇄
  半角英数トグル。
- **ATOK互換**: Muhenkanは`CancelAndIMEOff`(IME自体を閉じる、MS-IMEと正反対)。
  英数トグル相当は`Eisu`/`F10`/`Kana`/`Shift+Muhenkan`の**4キー**に分散。
- **ことえり互換**(macOS): `Eisu`のみ存在、Muhenkan/Kana/Katakana/Hiragana
  相当のキー自体が無い(macOSキーボードにこれらの物理キーが無いため)。

普遍的な正解は無いため、Windowsで最も一般的なMS-IME互換の意味論を基準に選んだ。
ユーザーはGJIのキー設定で「MS-IME」プリセットを選択し(Hiragana/Katakanaの
`CompositionMode*`絶対指定に加え、Muhenkan/Eisuのネイティブトグルも標準で
揃う)、その上に3.2のFnキー(F21-F23)を`custom_keymap_table`で追加する。
プリセット選択とカスタム追加は共存できる(実際のconfig1.dbで`session_keymap`
フィールドとカスタムオーバーレイの共存を確認済み)。Eisuについては決定3.1の
通りawaseは関与せず、ネイティブの`ToggleAlphanumericMode`のままとなる。

#### 3.5 実装の要点

- `CharsetSlot`(GJI向け、物理キー→Fnキー変換機構): 物理`Muhenkan`/`Eisu`/
  `Shift+Muhenkan`の押下を検知し、現在のbeliefに応じて3.2の表からFnキーを
  選んでSendInputする。IMC write(`ImmSetConversionStatus`)ではなくVK注入を
  使う理由: GJI(mozc)のTIPはIMC writeをUIミラーとしてのみ扱い、実コンポーザ
  には反映しない(BUG-25追補2、mozc本家ソース確認済み)。
- 書き込み機構: `awase-gji-config`crateは現状読み取り専用(`extract_ime_keys`/
  `extract_mode_keys`)。config1.dbへの書き込み機能の実装は本ADRのスコープ外の
  別タスクとする(§5 Phase構成)。当面はユーザーがGJIの設定UIで手動インポートする
  運用とする。

#### 3.6 belief同期: 即時更新+定期再同期の二本立て(過去のMS-IME設計を踏襲)

CharsetSlotがFnキーを送る判断はawaseのbeliefに依存するため、beliefが実際の
GJI状態からズレたままだと誤った遷移先を送りうる。ドリフトの発生源は
CharsetSlot自身の送信だけでなく、`Hiragana`/`Katakana`物理キー(既に絶対指定
なので変換自体は不要だが、押されたという事実はawaseに伝わらない)、トレイ操作、
GJI自身のUI操作、F6-F10系の`ConvertTo*`(§3.1で言及)等、多岐にわたる。
対策は過去のMS-IME向け設計(ADR-084 INV-11、ADR-078等)と同様に2本立てとする:

1. **即時・楽観的更新**: CharsetSlotがFnキーを送信したら、そのFnキーが示す
   絶対指定ターゲットへbeliefを即座に更新する(precomposition分岐が実際に
   発火した前提。§3.3で既述)。
2. **定期的な実観測による再同期**: 既存の`ConvModeMgr`/idle-conv-check機構
   (`apply_idle_conv_check`等)をGJI向けにも適用し、`ImmGetConversionStatus`で
   読んだ実際のconversion_modeでbeliefを補正する。発生源を問わず網羅的に
   ドリフトを検出できる。conversion_modeの読み取りはcomposition中でも安全に
   行えることは`gji_composition_probe.rs`の実機検証(§1.4項目4)で確認済み
   ——「今composition中かどうか」を判定する必要はなく、単に現在のビット値を
   読むだけでよい。

**F6-F10はCharsetSlotの送信先には使わない。** F6-F10は実在する物理キーで
あり、GJIが`ConvertTo*`として消費するのはComposition/Conversion状態のときに
限られる(§3.1)。もしawaseがSendInputでF6-F10を送った時点でGJIが実際には
Precomposition状態だった場合、GJI側に未バインドのため**フォーカス中の
アプリケーションへそのまま素通りし**、ブラウザ更新・スペルチェック等の
無関係なアプリケーションショートカットとして誤発火しうる。F21-F23が安全なのは(F23は未実測)
「物理キーが存在しない安全域」(ADR-057)だからであり、F6-F10にはこの安全性が
無い——F13/F14を避けたのと同じ理由がF6-F10にも当てはまる。F6-F10は
(ユーザーが実際にComposition中に押した場合の)**観測対象**としてのみ、
上記2の定期再同期に含めてよい。

### 決定4(charset軸・MS-IME): `CharsetSlot`は作らない。パススルー+自己責任とする。

**MS-IMEでは、charset軸のモードトグルキー(無変換単独打鍵等)の物理押下を
awaseは一切インターセプト・変換しない。素通しする。** GJIと違い、MS-IMEには
「入力中かどうかをawase側で判定せずに済む」ようなstatus別コマンド解釈の仕組みが
無い。物理キー押下を安全に冪等な書き込みへ変換するには、awase自身が「今
composition中かどうか」を知る必要があるが、この情報をawaseは観測できない
(既知の制約、§1.4末尾)。この状態で変換を試みると、変換中の文字列を意図せず
壊す・確定させる等の実害を生みうる。**判定できない条件で分岐する変換機構は
安全に作れないため、GJI向けの`CharsetSlot`とは異なりMS-IME向けには作らない。**

- 物理DBEキーは、BUG-52対応の無条件Suppressをこの用途には適用せず、MS-IME
  ネイティブのモードトグル処理へそのまま渡す(パススルー)。
- 動作の質(意図通りに切り替わるか、スプリアスな挙動が無いか)はMS-IME自身の
  実装に依存し、awaseは保証しない(自己責任)。気になるユーザーはMS-IME設定で
  該当キーの割り当てを無効化するか、GJIへの乗り換えを検討する。

---

## 3. GJIのcharset軸が実証されたことによる既存決定への影響

- **config1.db書き込み復活の正当化**: 2026-08-11の「config1.db書き込みは復活させない」
  という判断の根拠は、ADR-067の前提(`VK_IME_ON`/`OFF`は冪等でconfig1.db不要)が
  open軸限定であり、charset軸には元々適用されないことが本ADRで確認された。ただし
  ADR-067が挙げたもう一つの反論(GJIプロセス再起動要否・登録状態監視・セットアップUI
  という保守コスト)は今回も同型で残る。今回の設計はF13/F14ではなくF21-F23という
  安全域(F23は未実測)を使うことで**到達性の問題(a)は回避できるが、保守コストの問題(b)には
  正面から答えていない**。書き込み機構の実装着手時に改めて評価すること。
- **候補ウィンドウ問題の位置づけ**: §1.4項目4の実機結果により、「候補ウィンドウ
  表示の有無をawaseは観測できない」という既知の制約(§1.4末尾)は、GJI側の
  `CompositionMode*`コマンドが変換中でも何らかの形で処理を続ける(単純に無視は
  されない)ことを示唆しているが、詳細な安全性はまだ厳密には検証できていない。
  「区別しない、GJI側の挙動に委ねる」という方針を維持しつつ、追試を推奨する。

---

## 4. `CharsetSlot`の対象をMS-IMEからGJIへ付け替えた経緯

2026-08-13の同セッション内で、Fable(レビュアー)×Opus(設計者)のpre-mortemを
**5ラウンド**回し、MS-IMEの物理DBEキー押下(無変換等、BUG-52修正以降
`ime_actuation_owned`なら無条件Suppressされ意図が握りつぶされている)を、
awase自身がbeliefに基づいて`ImmSetConversionStatus`へ冪等に再発行する
`CharsetSlot`という機構を設計した(INV-54〜77、S27〜S47、P23〜38、
訂正1〜11を含む詳細設計。実コード裏取り済み、round5でCONVERGED判定)。

**この設計(MS-IME向け、`ImmSetConversionStatus`書き込み)は不採用とし、
`CharsetSlot`という概念自体はGJI向け(決定3、Muhenkan単独打鍵→F21-F23への変換)へ付け替える。**
撤回ではなく対象の変更であり、「物理キーの握りつぶされた意図を冪等な書き込みへ
変換する」という`CharsetSlot`の中核アイデア自体は維持される。

**MS-IME向け実装を採らない理由**:

1. MS-IME向けの`CharsetSlot`は、物理キー押下を正しい`ImmSetConversionStatus`
   書き込み値へ変換するために、**awase自身が「今composition中かどうか」を
   判定する必要があった**。この情報はawaseから観測できない(既知の制約、
   §1.4末尾)。判定できない条件で安全に分岐する変換機構は作れない。
2. GJIには、awase側の判定を不要にする仕組み(config1.dbの`custom_keymap_table`
   がstatus別にコマンドを解釈する)が既に存在し、かつ実機でF15-F19の効果が
   検証できた。**同じ「物理キー→冪等書き込み」という変換アイデアを、
   awase側の状態判定が不要なGJI向けに転用すれば、判定不能問題を回避できる**。
3. MS-IMEの候補ウィンドウ表示中の挙動の違いを区別できない制約
   (§2.4.7、既知の制約として設計に組み込み済み)や、TsfNative限定の
   鮮度スタンプ供給(INV-74)、トレイ経由のfresh intent不足(INV-75)等、
   MS-IME向け設計固有の複雑さは、GJI向けの単純な変換表には持ち越さない
   (GJI向けはstatus別解釈をGJI自身に委ねるため、これらの複雑さが
   構造的に発生しない)。

**破棄しない情報**: MS-IME向け`CharsetSlot`設計時に確定した実機事実(BUG-52の
無条件Suppress挙動、MS-IMEでのIMC write到達性、TsfNative/ImmCrossの
idle-conv-check非対称性等)は`docs/known-bugs.md`・既存ADR(084/086/087/088)に
既に記録されており、失われない。INV-54〜77・S27〜S47・P23〜38の番号空間は
**MS-IME向けの意味では本ADRで使用しない**(GJI向け`CharsetSlot`の実装〈決定3〉は
別途新しい番号を起こす。MS-IME向けの`ImmSetConversionStatus`ベース書き込みを
将来再提案する場合は、このセクションを読み、composition中判定不能問題が
未解決のままであることを踏まえること)。

---

## 5. 移行計画

### Phase 0(記録のみ)

1. 本ADRを`docs/adr/091-*.md`として追加し、`docs/adr/index.md`に登録する。
2. `docs/known-bugs.md`に、config1.dbの残骸バインド(F13/F14/F21/F22)が
   実機に残存しうる既知の事実として記録する(次にconfig1.db関連の作業をする際、
   同じ残骸に驚かないため)。

### Phase 1(GJI charset軸の本実装、実機ソーク必須)

1. `awase-gji-config`にconfig1.db書き込み機能を追加する(バックアップ・
   原子的置換・既存バインドとの衝突検出込み)。
2. awase起動時またはGJI検出時に、F21-F23の3個の二段バインド(§3.2)の存在を
   確認し、無ければユーザーに「MS-IME」プリセット選択+`custom_keymap_table`
   追加を促す(自動書き込みは要検討、§3の保守コスト論点)。
3. **`CharsetSlot`(物理Muhenkan単独打鍵→F21-F23変換ロジック)を
   実装する。** §3.3の対応表に基づき、物理キー押下時のawase側belief(現在の
   charset目標値)を見て、5つのFnキーから1つを選びSendInputする。
4. `Hiragana`/`Katakana`物理キー(`VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA`)を
   BUG-52対応の無条件Suppress対象から除外できるか検証する(§3.3、既に絶対指定
   でbelief乖離リスクが無いため、素通しでも安全なはず)。
5. `VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`がmozcのキー名語彙上どの物理トークンに
   対応するか(あるいは対応が無いか)を確認する。
6. **belief同期機構(§3.6)を実装する。** 既存の`ConvModeMgr`/idle-conv-check
   機構をGJI検出時にも適用し、`ImmGetConversionStatus`による定期再同期で
   CharsetSlotのbeliefを補正する。F6-F10等の観測対象キーの扱いも含める。

### Phase 2(候補ウィンドウ挙動・実機ソークの追試)

1. `gji_composition_probe.rs`を拡張し、§3.2の5個のFnキー(状態別二段バインド)を
   実機で送信して、Precomposition時は絶対指定通りの`conversion_mode`になるか、
   Composition/Conversion時はネイティブトグル(`SwitchKanaType`/
   `ToggleAlphanumericMode`)が実際に発火し破壊的な影響が無いかを検証する。
2. `ConvertToHalfWidth`等のF6-F10系コマンド(§3.1で言及、変換候補の再変換用途と
   推定)の正確な用途を確認し、決定3の設計と混同しないよう整理する(現時点では
   F21-F23の設計に必須ではない)。

### Phase 3(MS-IME自己責任ポリシーのドキュメント化)

ユーザー向けドキュメント(`docs/usage.html`等)に、MS-IME使用時はモードトグルキーの
無効化を推奨する旨、無効化しない場合は動作保証外である旨を明記する。

---

## 6. 関連ファイル

- `crates/awase-gji-config/`(`command.rs`/`keymap.rs`/`tsv.rs`/`wire.rs`) —
  読み取り機構は既存、書き込み機能はPhase 1で追加。
- `crates/awase-windows/examples/gji_composition_probe.rs` — 本ADRの検証に
  使った自前IMM32プローブ。使い捨てツールとして残す。
- `crates/awase-windows/src/runtime/transport.rs` — MS-IME物理DBEキーの
  無条件Suppress(BUG-52対応)。決定4により変更しない(自己責任ポリシーの下では
  現状維持で問題ない)。
- `docs/known-bugs.md` BUG-25(GJI SendInput/IMC write到達性)・BUG-52
  (物理DBEキー漏洩)・BUG-61(romaji軸解決不能)・BUG-62(Alt+かなswallow)。
- `docs/adr/057-gji-keybind-f13f14-to-f21f22.md`(F13/F14を避ける根拠)、
  `docs/adr/067-vk-ime-on-off-migration.md`(config1.dbバインド撤廃の経緯、
  今回の復活判断の前提確認元)。
- mozc本家ソース(`github.com/google/mozc`、Apache-2.0、2026-08-13時点の
  masterブランチを直接確認):
  - `src/data/keymap/ms-ime.tsv`/`atok.tsv`/`kotoeri.tsv` — 3プリセットの
    キーバインド定義。決定3.2/3.4の直接的な根拠。
  - `src/session/session.cc::Session::CompositionModeSwitchKanaType` —
    かな循環(ひらがな→全角カタカナ→半角カタカナ)の実装。
  - `src/composer/composer.cc::Composer::ToggleInputMode` — 英数トグル
    (ひらがな⇄半角英数の二値トグル、全角英数は経由しない)の実装。

## 7. 関連ADR

- ADR-067: open軸のVK_IME_ON/OFF移行。本ADRのopen軸の結論はこれを維持。
- ADR-085/086/087: force-write・warrant分離。charset軸の意図管理(トレイ操作等)は
  これらの既存機構と整合させる(Phase 1実装時に詳細設計)。
- ADR-088/089/090: 軸capabilityモデル・型状態パターン。本ADRのcharset軸の
  「GJI推奨・MS-IME自己責任」という結論は、これらが提案していた「プロファイルごとの
  能力表」という考え方の実践例と位置づけられる(GJI=Idempotent、MS-IME=対象外)。
