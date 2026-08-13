# ADR-091: 冪等キー中心のIME制御 — open/romaji/charset 3軸の結論、GJI推奨・F15-F19設計、MS-IME自己責任ポリシー

## ステータス

**決定・実装未着手。charset軸(GJI)のみ実機でconv bits直接読み取りによる検証済み。**

本ADRは、awaseのIME状態制御の設計思想を「awase自身がIME状態を能動的にトグル管理する」から
「基本的にキーはパススルーし、IME状態を動かすキーは冪等(絶対指定)キーだけを使う」へ転換する、
という2026-08-13のセッションでの方針転換を記録する。open/romaji軸は既存の対応を追認するのみ、
charset軸が本ADRの新規決定の中心である。

**本ADRは、同セッション内で先行して5ラウンドのFable×Opus pre-mortemを経た`CharsetSlot`設計
(物理DBEキー押下をbeliefに基づき冪等に再発行する機構、当初MS-IME向けとして設計)の対象を
GJI向けへ付け替える。** MS-IMEはcomposition中判定が不能なため安全な変換ができず、
GJIはF15-F19という実証済みの変換先とstatus別解釈の委譲により判定不要で成立する。
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

5. **MS-IMEの物理DBEキー再発行機構(`CharsetSlot`)は撤回**: 詳細は§4。

---

## 2. 決定

### 決定1(open軸): 現状を設計原則として固定する。変更しない。

§1.2の通り。VK_IME_ON/OFFが冪等キーとして機能し、VK_KANJIはImm32Unavailable向け
フォールバックとしてbelief駆動で残る。

### 決定2(romaji軸): 制御しない。抑止する。

§1.3の通り。JISかな入力方式は非サポート。物理Alt+かなは予防的swallowを維持する。

### 決定3(charset軸): GJIを推奨IMEとし、config1.dbバインドで制御する。冪等原則は「入力モード」サブ軸のみに適用する。

**冪等キー限定の原則は、charset軸のうち「次に入力する文字のデフォルトモード」
(以下「入力モードサブ軸」)にのみ適用する。「今まさに画面に見えている
composition文字列の文字種変換」(以下「変換中サブ軸」)には適用しない。**

理由: 冪等原則がそもそも防ごうとしていたのは、open/romaji軸のように「間違えると
気づかないまま状態が持続する」危険である(open軸はawaseのbeliefと実状態が
silent desyncする、romaji軸は復旧不能になる、等)。composition中の文字列は
画面に表示されており即座にフィードバックがあるため、仮に循環トグルで一段
ズレても目視で気づいて押し直せばよく、実害の質が全く異なる。したがって
変換中サブ軸ではGJIのネイティブなトグル系コマンドを採用してよい。

**入力モードサブ軸(Precomposition/DirectInput/Prediction/Suggestion): 冪等・絶対指定。**
config1.dbの`custom_keymap_table`に、F15-F19を以下へバインドする:

| VK | コマンド | Charset |
|---|---|---|
| F15 | `CompositionModeHiragana` | ひらがな |
| F16 | `CompositionModeFullKatakana` | 全角カタカナ |
| F17 | `CompositionModeHalfKatakana` | 半角カタカナ |
| F18 | `CompositionModeFullAlphanumeric` | 全角英数 |
| F19 | `CompositionModeHalfAlphanumeric` | 半角英数 |

F13/F14/F21-F24は使わない(F13/F14はADR-057当時ターミナルへのエスケープシーケンス
漏れ・DirectInputゲーム競合で捨てられたキー、F21-F24は旧awase config1.db管理機構
[ADR-067で削除済み]の残骸バインドが実機に残存していることを確認済み)。

**変換中サブ軸(Composition/Conversion): GJIネイティブのトグル系コマンドを許容する。**
実際のconfig1.dbには、この用途向けに`ConvertToHiragana`/`ConvertToFullKatakana`/
`ConvertToHalfWidth`/`ConvertToFullAlphanumeric`/`ConvertToHalfAlphanumeric`という
別系統のコマンドが既に存在し、デフォルトでF6-F10に割り当て済みだった(実機確認済み)。
さらに、MS-IMEの無変換単独打鍵のネイティブな挙動(precomposition時は
「全角カタカナ→半角カタカナ→ひらがな」の循環)を再現したい場合、GJI側の
`SwitchKanaType`/`CompositionModeSwitchKanaType`(相対トグル系、
`GjiModeCommand::ToggleKanaType`として`awase-gji-config`が既に分類済み)を
Composition/Conversion状態に割り当てることで実現できる。無変換単独の
状態別バインド例:

| status | key | command |
|---|---|---|
| Precomposition/DirectInput/Prediction/Suggestion | Muhenkan | `SwitchKanaType`(または`CompositionModeHiragana`等の絶対指定) |
| Composition/Conversion | Muhenkan | `SwitchKanaType`または`ConvertTo*` |

現在のconfig1.dbには無変換単独の割り当ては存在しない(空白、実機確認済み)ため、
GJI組み込みの「MS-IME互換」キー設定プリセットが標準でこれを含んでいるかは
別途確認が必要(§5 未解決)。含んでいなければ、上表のように`custom_keymap_table`へ
明示的に追加する。

- awase側は、charset軸の**入力モードサブ軸**の意図(ユーザーがトレイ操作等で
  明示的にモードを選んだとき、または`conv_mode_policy=force`のような既存機構が
  desired_modeを持つとき)を、GJI検出時はF15-F19のSendInputに変換して送信する。
  IMC write(`ImmSetConversionStatus`)ではなくVK注入を使う理由: GJI(mozc)のTIPは
  IMC writeをUIミラーとしてのみ扱い、実コンポーザには反映しない(BUG-25追補2、
  mozc本家ソース確認済み)。

- **`CharsetSlot`(GJI向け、物理DBEキー→F15-F19変換機構)**: 現状、物理DBEキー
  (`VK_DBE_HIRAGANA`/`KATAKANA`/`SBCSCHAR`/`DBCSCHAR`/`ALPHANUMERIC`)の押下は
  `ime_actuation_owned`なら無条件Suppressされ、ユーザーの意図(「カタカナキーを
  押した」等)が握りつぶされて消えている(BUG-52対応の副作用)。`CharsetSlot`は
  この握りつぶされた物理キーの意図を、対応するF15-F19のSendInputへ変換する
  単純な変換表として実装する(入力モードサブ軸のみが対象。無変換単独等、
  変換中サブ軸で使うキーはGJI自身へのパススルーで足り、CharsetSlotの変換対象
  ではない)。

  GJI自身が`custom_keymap_table`のstatus別コマンド解釈(Precomposition/
  Composition/Conversion/Prediction/Suggestion)を内部で行うため、**awase側は
  「今入力中かどうか」を判定する必要がない**——1つの物理キー押下に対して
  1つのF-keyを送るだけの単純な対応表で足りる(物理VKとF15-F19ターゲットの
  正確な対応は`vk.rs`の`ImeKeyKind`分類を出発点にPhase 1で確定する)。これは
  MS-IME向けに検討していた「入力中/非入力中で正しい書き込み値を判定する」という
  複雑さ(候補ウィンドウ非可視性問題)を、GJI自身への委譲によって回避できている
  ことの帰結である。

- 書き込み機構: `awase-gji-config`crateは現状読み取り専用(`extract_ime_keys`/
  `extract_mode_keys`)。config1.dbへの書き込み機能の実装は本ADRのスコープ外の
  別タスクとする(§5 Phase構成)。当面はユーザーがGJIの設定UIで手動インポートする
  運用とする。

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
  という保守コスト)は今回も同型で残る。今回の設計はF13/F14ではなくF15-F19という
  安全域を使うことで**到達性の問題(a)は回避できるが、保守コストの問題(b)には
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
`CharsetSlot`という概念自体はGJI向け(決定3、F15-F19への変換)へ付け替える。**
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
2. awase起動時またはGJI検出時に、F15-F19バインドの存在を確認し、
   無ければユーザーに設定を促す(自動書き込みは要検討、§3の保守コスト論点)。
3. charset軸の意図(トレイ操作等)をGJI検出時はF15-F19のSendInputへ変換する
   実装を、既存の`ime_controller.rs`のStrategy群に追加する。
4. **`CharsetSlot`(物理DBEキー→F15-F19変換表)を実装する。** `vk.rs`の
   `ImeKeyKind`分類を出発点に、`VK_DBE_HIRAGANA`/`KATAKANA`/`SBCSCHAR`/
   `DBCSCHAR`/`ALPHANUMERIC`のうちGJI検出時に`ime_actuation_owned`で
   Suppressされているものを、対応するF15-F19のSendInputへ変換して送信する。
   status別解釈はGJI自身に委ねるため、composition中かどうかの判定はawase側で
   行わない(§4参照)。

### Phase 2(候補ウィンドウ挙動・変換中サブ軸の追試)

1. `gji_composition_probe.rs`のシナリオ分離を改善(未確定compositionの明示
   キャンセル)した上で、henkan中の`CompositionMode*`押下の挙動を再検証する。
2. GJIの「MS-IME互換」キー設定プリセットが、無変換単独打鍵の状態別挙動
   (precomposition時は`SwitchKanaType`相当の循環、composition時は文字種変換)を
   標準で含んでいるか実機で確認する。含んでいればそのプリセットを推奨設定として
   案内し、含んでいなければ決定3の状態別バインド例を`custom_keymap_table`へ
   追加する形で補う。
3. `ConvertToHalfWidth`(半角カタカナ専用ではなく汎用の半角変換コマンド)が、
   Composition/Conversion状態でF17(半角カタカナ)ターゲットに対して期待通り
   動作するか実機で確認する。

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

## 7. 関連ADR

- ADR-067: open軸のVK_IME_ON/OFF移行。本ADRのopen軸の結論はこれを維持。
- ADR-085/086/087: force-write・warrant分離。charset軸の意図管理(トレイ操作等)は
  これらの既存機構と整合させる(Phase 1実装時に詳細設計)。
- ADR-088/089/090: 軸capabilityモデル・型状態パターン。本ADRのcharset軸の
  「GJI推奨・MS-IME自己責任」という結論は、これらが提案していた「プロファイルごとの
  能力表」という考え方の実践例と位置づけられる(GJI=Idempotent、MS-IME=対象外)。
