# ADR-091: 冪等キー中心のIME制御 — open/romaji/charset 3軸の結論、GJI推奨・かな形状は設定+ベストエフォート助言(新規beliefなし)、MS-IME自己責任ポリシー

## ステータス

**決定・実装未着手。charset軸のGJI SendInput到達性は実機でconv bits直接読み取りによる検証済み
(ただし、その検証を経て採用した「完全パススルー」版の`CharsetSlot`は不採用となり、
「ユーザー設定+ベストエフォート助言」方式に再収束した。§3参照)。**

本ADRは、awaseのIME状態制御の設計思想を「awase自身がIME状態を能動的にトグル管理する」から
「基本的にキーはパススルーし、IME状態を動かすキーは冪等(絶対指定)キーだけを使う」へ転換する、
という2026-08-13のセッションでの方針転換を記録する。open/romaji軸は既存の対応を追認するのみ、
charset軸が本ADRの新規決定の中心である。

**charset軸の結論は、検討の過程で複数回反転している。** 当初は物理DBEキー押下を
beliefに基づき冪等に再発行する`CharsetSlot`という機構を5ラウンドのFable×Opus pre-mortem
で設計し(MS-IME向け→GJI向けへ付け替え、F15-F19→F21-F23への変更を経た)、実機検証まで
行った(§1.4)。その後、GJIのMS-IME互換プリセットが物理`Muhenkan`1キーで
belief不要の自己完結サイクルを既にネイティブ提供していると判明し、「かな形状は
awaseが一切感知・強制・観測しない、完全パススルー」へ一度反転した。しかしこの
「完全パススルー」自体が、既存の2つの実機インシデント対応保護機構
(`muhenkan_solo_tap_always_suppress`、BUG-52のDBEレンジSuppress)と正面衝突する
ことが第2回Opusレビューで判明し、**最終的に「抑止/パススルーをユーザー設定で選べ、
awaseはconfig1.db/レジストリの読み取りに基づきベストエフォートで安全な構成を助言・
支援する」という3層構成に再収束した(決定3 §D3.1)。** かな形状(3値)について
新しいbeliefを持たないという結論自体は一貫して維持されている。理由は決定3(§2)と
§3に記す。この一連の反転を、当初の設計・実機検証の記録ごと本ADRに残す
(このリポジトリの`.claude/rules/experiment-logging.md`が定める「反転の記録」規約に倣う)。

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
   読み取り値が期待通りに一致した。**注意: F15(Hiragana)の期待ビットのみ`ROMAN`が
   含まれていない(9=NATIVE|FULLSHAPE、他4行は16=ROMANを含む)。これが
   `CompositionModeHiragana`コマンド自体の意図した挙動なのか、それとも当時の
   期待値設定・観測いずれかの誤りなのか、この記録の時点では未解明のまま残っている。
   本ADRの最終決定(決定3)はこの検証結果に依存しないため実害は無いが、この
   `gji_composition_probe.rs`のデータを将来再利用する場合は先に再測定して
   確認すること。**

   F15-F19 + config1.db `CompositionMode*`バインドは、
   GJIのcharset軸を冪等に絶対制御する、実証済みの機構である。

   なお候補ウィンドウ表示中(henkan)シナリオでは、F15押下後にEnterで確定した結果に
   予期しない漢字変換("居合"等)が混入する現象が観測された。プローブ側のシナリオ間で
   IME未確定compositionのキャンセルを行っていなかったことによる測定汚染の疑いが強く、
   核心の結論(conversion_modeの直接一致)には影響しないが、変換中の詳細な挙動を厳密に
   詰めるには追試が要る(§4 Phase 2参照)。

   **注記**: この検証は「F15-F19にCompositionMode*を全状態で統一バインドする」という
   単純な構成で行い、SendInputでF-keyを送ればGJIのconv bitsを確実に動かせるという
   **機構自体**の実証が目的だった。その後、この構成のままターミナルアプリへ
   エスケープシーケンスが残留する不具合が実機で見つかり、設計を「Precomposition=
   絶対指定/Composition=ネイティブトグル」の二段構成・キー番号もF21-F23へ
   変更した(§3.2)。本節の検証はF15-F19という危険な番号を使った点を除けば、
   基礎機構(SendInputの到達性・`ImmGetConversionStatus`での確認可能性)の
   証明としては有効であり、当時計画していたPhase 2の新番号での実機検証は
   §3.2の反転により不要になった(§3参照)。

5. **`CharsetSlot`は、MS-IME向け→GJI向けへの付け替えを経たのち、最終的に不採用**:
   詳細は§3。上記1-4で実証したのは「SendInputでGJIのcharset軸を絶対指定で確実に
   動かせる」という**機構としての実現可能性**であり、この事実そのものは有効なまま
   残る。しかし、この機構を実際に採用するかどうかの検討で、(a) mozc本家の
   MS-IME互換プリセットが物理`Muhenkan`1キーに対し、awase側のbeliefを一切使わない
   自己完結的な3状態サイクルを既にネイティブで提供していること、(b) このかな形状の
   値を消費するawase側の機能が`state/conv_mode.rs::Charset`(トレイアイコン表示のみ)
   に限られ、機能的な判断には使われていないこと、(c) NICOLAエンジンON中は既存の
   ADR-084/086 force-writeがconv modeをRomajiHiraganaへ強制するためこの軸自体が
   実質作用しないこと、の3点が判明し、`CharsetSlot`を作る積極的な理由が無いと
   結論した。決定3参照。

---

## 2. 決定

### 決定1(open軸): 現状を設計原則として固定する。変更しない。

§1.2の通り。VK_IME_ON/OFFが冪等キーとして機能し、VK_KANJIはImm32Unavailable向け
フォールバックとしてbelief駆動で残る。

### 決定2(romaji軸): 制御しない。抑止する。

§1.3の通り。JISかな入力方式は非サポート。物理Alt+かなは予防的swallowを維持する。

### 決定3(charset軸・GJI): config1.dbの自動判定を主UXとし、必要な判断のみポップアップで問う。持続的なユーザー設定は前面に出さない。beliefは持たない。

#### D3.1 結論

**GJIのcharset軸(かな形状・かな⇔英数境界)について、awaseが特定の状態を予測して
belief化することはしない。** また、**設定画面の煩雑化を避けるため、ユーザーが
手動で選ぶ持続的な設定を主UXにはしない。** 代わりに、config1.db/レジストリの
読み取りに基づく自動判定を主とし、ユーザーへの問いかけは「今すぐ行うべき判断が
あるとき」のポップアップに限定する:

1. **自動判定(主UX)**: GJI検出時、awaseはconfig1.db(`session_keymap`/
   `custom_keymap_table`、`awase-gji-config`の既存読み取り機構)を確認する。
   §D3.2の専用Fnキーバインドが既に存在すれば、**ユーザーの手動設定なしに**
   「専用Fnキー変換」モードを自動的に有効化する(Composition中のかな形状トグルが
   使えるようになる)。存在しなければ既定の「抑止」のまま据え置く。
2. **ポップアップ(判断が必要なときのみ)**:
   - **GJI、Fnキー未設定**: 無変換単独打鍵がGJI既定の「文字種変更」動作のまま
     素のパススルーに(手動で)設定されており、かつ§D3.2のFnキー設定が
     まだ完了していない場合、設定を促すポップアップを出す(新規、後述)。
   - **MS-IME、既定のかな切替のままパススルー**: 決定4のとおり、GJI利用を
     推奨するポップアップを出す。
3. **設定支援**: ポップアップからユーザーが同意すれば、§D3.2の専用Fnキーの
   config1.dbへの追加をawaseが実行する(バックアップ・原子的置換・既存バインド
   との衝突検出込み。無断・自動では書き込まない、必ずポップアップでの同意が前提)。
4. **手動設定(上級者向けの逃げ道)**: `Hiragana`/`Katakana`物理キー・`Eisu`/
   `Shift+Muhenkan`の「抑止/素のパススルー」、無変換単独打鍵の
   「抑止/素のパススルー/専用Fnキー変換」は、設定ファイル上は残すが、通常の
   設定画面の前面には出さない(既存の`muhenkan_solo_tap_always_suppress`相当の
   位置づけ、上級者向け)。専用Fnキー変換が有効化される主経路は上記1の自動判定で
   あり、この手動設定は非標準構成向けの明示的オーバーライドに限る。

**いずれの経路でも、動作を保証するものではない(ベストエフォート、自己責任)。**
`Henkan`(変換キー)は`Reconvert`という別機能でcharsetのモード切替とは無関係、対象外。
`Hankaku/Zenkaku`/`Kanji`/`ON`/`OFF`はopen軸(決定1の範囲、既に冪等キーで解決済みの
ためこの自動判定の対象外、§1.2)。

**「belief を持たない」のはかな形状(3値サイクル)に限った話である。** かな⇔英数の
境界(2値)は、エンジンON/OFF判断に使われる既存の別機構(`state/eisu_recovery.rs`の
ObservedEisu救済、BUG-57)が引き続き担う(§D3.4、本ADRの対象外、変更しない)。

#### D3.2 推奨構成: Composition/Conversion限定の専用Fnキー(Precompositionは意図的に未バインド)

無変換の素のパススルーは、後述(§D3.3)のとおり既存の複数の保護機構と衝突するため、
**推奨構成では素のMuhenkan/Hiragana/Katakanaは送らず、代わりに専用のFnキー1個
(例: 実測済みで安全な`F21`、ADR-057)を新設する**。config1.dbには次の1エントリ
のみを追加する:

| 状態 | コマンド |
|---|---|
| Precomposition | (バインドしない) |
| Composition/Conversion | `SwitchKanaType` |
| Prediction/Suggestion | `SwitchKanaType`(候補表示中も対称に扱う。第3回Opusレビュー
  指摘: `awase-gji-config::keymap.rs`はこの2状態をIME-ON statusに含めており、
  対応を欠くと候補表示中にFnキーが未バインドのままアプリへ漏れる) |

Precomposition・Prediction・Suggestion(候補表示中を含む、`awase-gji-config`の
`keymap.rs`がIME-ON statusとして扱う範囲)時にこのFnキーを送っても、GJI側に対応
バインドが無ければフォーカス中のアプリケーションへそのまま流れる可能性がある。
**この設計が許容できる根拠は、F21が「物理キーが存在しない安全域」(ADR-057で
WezTerm実測済み)という、キー番号そのものの性質にある**——たとえアプリへ漏れても
実害が低いという前提でこの構成を採用する(第3回Opusレビューで、ADR-057の実測が
「全状態にバインドした構成」でのものであり「未バインド時に安全に流れる」ことまでは
実測していないという指摘を受けたが、漏れた先でF21自体が無害という理由でこの
リスクは許容する、というのが本ADRの判断)。**未バインド時の実際の挙動(本当に
アプリへ流れるのか、GJIが別の形で処理するのか)はPhase 2で実機検証する。**
Composition/Conversion時は`SwitchKanaType`がGJI自身の今の実際の内部状態を見て
トグルする(mozc本家`session.cc`で確認済みの通り、外部beliefを一切参照しないため
driftが原理的に発生しない)。

物理無変換の単独打鍵を検知したら、awaseは(素のVK_NONCONVERTを送るのではなく)
**常に**このFnキーをSendInputする。GJIの状態によって送信するキーを選ぶ必要が
無いため、CharsetSlotのようなbelief駆動の選択ロジックは不要になる。

**この構成による新機能追加**: 「専用Fnキー変換」モードを選んだ場合、Composition/
Conversion時にかな形状トグルという新機能が使えるようになる。これは
`muhenkan_solo_tap_always_suppress`(既定の「抑止」)とは独立したモードであり、
「抑止」を選んでいる限りは現状の全面抑制のまま変わらない(§D3.1)。

(この専用Fnキー案に至るまで、Fnキー3個(F21-F23)にawaseのbeliefを基にした絶対指定
コマンドを充てる、より複雑な`CharsetSlot`機構を設計・実機検証していた。その撤回の
経緯は§3(設計変遷)に保存する。)

**2026-08-15追記(F22-F24予備バインドの追加提案→即日撤回)**: 同日、config1.dbへの
書き込みはユーザーにサインアウト/インを要求する高コストな操作のため、F21に加えて
F22/F23/F24にもPrecomposition時の絶対設定コマンド(`CompositionModeHiragana`/
`FullKatakana`/`HalfKatakana`)を「将来使うかもしれない予備」として一度に書き込む
拡張を実装した(`write_dedicated_fn_key_set`)。

しかしユーザーとの対話で、これが§3.2で撤回した`CharsetSlot`(F21-F23への
belief駆動絶対指定コマンド)と同じ前提——「どのキーを実際にいつ送信するかを
awase側のbeliefで判断する」という未実装の判断ロジックが無ければF22-24は
一切機能しない——を抱えていることが明確になった。判断ロジックを設計・実装する
具体的な計画が無い状態でGJI側の受け皿だけ先に用意しても、**「すぐには使えない
キーのために、ユーザーに複雑な確認ダイアログとリスク（既存バインドとの衝突）を
負わせるだけ」**であり、config1.db書き込み回数を減らすという当初の動機に見合わない
と判断し、**即日撤回してF21のみの構成に戻した**(`5ada7bcd`/`82f6a890`/`bbb350e2`の
revert)。

**教訓**: 「将来使うかもしれないので今のうちに」という動機での予備的な拡張は、
その「将来」を実現する具体的な後続タスクが無い限り、`CharsetSlot`が抱えていた
のと同じ未解決の設計課題(awase側の判断ロジック)を素通りしたまま実装してしまう
リスクがある。config1.db書き込みのコスト削減それ自体は妥当な動機だが、
「今すぐ使える構成を1回で正しく書く」ことを優先し、未使用のまま残るキーの
予約は、判断ロジックの設計と同じタイミングで行うこと。

**設定未完了時のポップアップ(新規)**: GJIが検出され、かつ無変換単独打鍵がGJI既定の
「文字種変更」動作のまま(§D3.5のMS-IME互換プリセット下での挙動)手動で「素の
パススルー」に設定されているが、config1.dbに§D3.2のFnキーバインドがまだ存在しない
場合、awaseは設定変更を促すポップアップを出す(「専用Fnキー変換を有効にしますか?
GJIの設定ファイルに安全なFnキーバインドを追加します」)。**同意されれば、
上記の設定支援(§D3.1項目3)でconfig1.dbへ書き込む。** これはD3.1の自動判定が
「Fnキー未設定→抑止のまま据え置く」を選ぶ通常経路の外側にある状態(ユーザーが
手動でパススルーへ上書きしている場合)をカバーするための、決定4のMS-IME向け
ポップアップに対応するGJI向けの片割れである。

#### D3.3 なぜ素のパススルーではダメなのか: 既存の保護機構とその理由

無変換単独打鍵・`Hiragana`/`Katakana`物理キーの**素のパススルー**には、以下の
既存の防御機構が既に存在し、いずれも実機インシデントの再発防止のために追加された:

- **`muhenkan_solo_tap_always_suppress`**(既定`true`、`src/config.rs:222`): MS-IMEの
  「キーとタッチのカスタマイズ」既定で、無変換キー単独打鍵に「かな切替」(≈IMEオン
  相当)が割り当てられている。composing=falseの場面で素の無変換を送出すると、この
  MS-IME既定割当てに横取りされawaseの管理外でIMEモードが切り替わる
  (2026-08-07実機、composing=falseの単独タップ直後に非注入の`VK_DBE_ALPHANUMERIC`→
  `VK_DBE_HIRAGANA`が観測されshadow toggleがIMEをONにした)。
- **BUG-52のDBEレンジSuppress**(`runtime/transport.rs`、`ime_actuation_owned`時):
  NICOLAの物理「IME ON」キー(scan 0x70、JIS配列の「カタカナ・ひらがな・ローマ字」
  キー位置)を、IMEが既にONの状態で押すと、**Windowsのキーボードレイアウト変換層が
  `VK_DBE_HIRAGANA`(0xF2)の代わりに`VK_DBE_KATAKANA`(0xF1)を生成することがある**
  (同一物理キーに対するOS側の状態依存トグル変換、awaseの関与しない層)。これを
  素通しするとMS-IMEが仕様通りカタカナへ能動的に切り替わってしまう(2026-08-05実機、
  ユーザー報告「また謎にカタカナになる事象が再発しました」)。**アプリが謎に
  VK_DBE_*を送出しているのではなく、OS自身のキー変換層が特定の物理キーに対して
  状態依存で異なるVKを生成する既知の挙動であり、awaseの観測やbeliefの話ではない。**

**いずれもawaseのbelief管理とは無関係で、「パススルーそのものがOS/MS-IME側の
副作用を誘発する」という別種のリスクへの防御である。** 「観測も強制もしない」という
本ADRの思想を採用しても、この種のパススルー起因のリスクは消えない。そのため、
§D3.2の専用Fnキー(素のVK_NONCONVERT/DBEキーを一切送らない)を推奨構成とし、
それでも素のパススルーを選びたいユーザーには§D3.1のとおり設定と助言を提供する、
という切り分けにした。

なお`has_katakana`(`state/conv_classify.rs`)がエンジンopen belief推論・BUG-50の
復旧理由選択に使われている件は、**新しいbeliefを必要とせず既存の観測
(`ConvModeMgr`/idle-conv-check)を読むだけ**であり、本決定の「新しいbeliefは
持たない」という原則と衝突しない。

**2026-08-17追記(ADR-094で撤去)**: 上記の判断は当時(BUG-50原因2が未解明の
段階)は妥当だったが、BUG-50の元インシデントがBUG-52の機構(修正済み)だったと
後日判明したことを受け、`has_katakana`/`ConvModeMgr`の既存観測利用も含めて
charset軸の残存追跡を全撤去した。詳細は[ADR-094](094-charset-axis-and-force-policy-removal.md)参照。

#### D3.4 かな⇔英数境界(Eisu)は別軸: 既存機構を変更しない

エンジンON/OFFのようにawaseの能動的な挙動を左右する判断は、かな⇔英数のどちらに
いるかを知っている必要があり、これは既存の別機構(`state/eisu_recovery.rs`の
ObservedEisu救済、BUG-57で追加済み)が既に担っている。この機構は物理Eisuキーの
挙動を横取りするのではなく観測(conv bits読み取り)で動くため、`Eisu`/
`Shift+Muhenkan`をパススルー(または抑止、ユーザー設定次第)にしても影響を受けない。
本ADRはこの既存機構を変更しない。

#### D3.5 なぜMS-IME互換プリセットを推奨するか(ATOK/ことえりとの比較)

mozc本家の3プリセットを比較した結果、この種のキーの意味論はIME文化ごとに
大きく異なると判明した:

- **MS-IME互換**(推奨): Muhenkan=かな循環相当、Eisu(+Shift+Muhenkan)=ひらがな⇄
  半角英数トグル。
- **ATOK互換**: Muhenkanは`CancelAndIMEOff`(IME自体を閉じる、MS-IMEと正反対)。
  英数トグル相当は`Eisu`/`F10`/`Kana`/`Shift+Muhenkan`の**4キー**に分散。
- **ことえり互換**(macOS): `Eisu`のみ存在、Muhenkan/Kana/Katakana/Hiragana
  相当のキー自体が無い(macOSキーボードにこれらの物理キーが無いため)。

普遍的な正解は無いため、Windowsで最も一般的なMS-IME互換の意味論を基準に推奨する。
「MS-IME」プリセット選択の有無はconfig1.dbの`session_keymap`フィールド(既存の
`awase-gji-config::wire::parse_top_level`が読み取り済み)からベストエフォートで
確認できる(§D3.1項目2)。

物理無変換キーが存在しないキーボード(US配列等、`nicola_us.yab`系)では、
この決定は単に無関係になる(Muhenkanの物理トリガー自体が無いため)。

#### D3.6 実装の要点

- **自動判定ロジック(主UX)**: GJI検出時にconfig1.dbを読み、§D3.2のFnキー
  バインドの有無を確認する。存在すれば「専用Fnキー変換」モードを自動的に
  有効化する(ユーザー操作不要)。存在しなければ既定の「抑止」のまま据え置き、
  D3.1項目2の条件を満たす場合のみポップアップで案内する。設定画面には、
  この自動判定の結果(現在有効なモード)を表示するだけで足り、ユーザーが
  持続的に選び続ける項目にはしない。
- **手動設定(隠し/上級者向け)**: `Hiragana`/`Katakana`物理キー・`Eisu`/
  `Shift+Muhenkan`は「抑止/素のパススルー」の2択、無変換単独打鍵は
  「抑止/素のパススルー/専用Fnキー変換」の3択を設定ファイル上に残すが、
  通常の設定画面の前面には出さない。既存`muhenkan_solo_tap_always_suppress`
  相当のbool群を拡張するのではなく、キーごとの意味論が異なる(Muhenkanは3択、
  他は2択、`Shift+Muhenkan`は独立VKではなくModifier+VKの組)ため、専用の
  struct/enumで表現する(第3回Opusレビュー指摘、既存`GeneralConfig`の単純な
  bool群への相乗りは避ける)。既定はすべて「抑止」で現状維持。
- **「専用Fnキー変換」モードの実装は`muhenkan_solo_tap_always_suppress`の
  早期returnより手前で分岐させる**(C1、第3回Opusレビュー指摘)。
  `src/engine/nicola_fsm.rs`の単独タップ確定処理で、このモード(自動判定または
  手動設定のいずれかで有効)なら既存のsuppress判定を経由せず直接Fnキー送出へ
  回す独立した経路とする。物理無変換単独打鍵を検知したら、素のVK_NONCONVERTでは
  なく§D3.2の専用Fnキーを常に送る。beliefは不要。
- `awase-gji-config`にconfig1.db書き込み機能を追加し、§D3.2の専用Fnキーエントリの
  追加をポップアップでのユーザー同意を経てのみ実行する(バックアップ・原子的
  置換・既存バインドとの衝突検出込み)。現状`awase-gji-config`はどこからも
  依存されていない単体crateであり、config1.dbのパス探索・読み込み呼び出し元・
  依存追加・GJI検出フックへの配線も本Phase 1の作業に含む(第3回Opusレビュー指摘)。
- ポップアップロジック: GJI向け(D3.2「設定未完了時のポップアップ」)・MS-IME向け
  (決定4)の2種類。いずれも状況判定は起動時・IME検出時に一度行い、継続的な
  ポーリングは行わない(D3.1で述べた「新しいbeliefは持たない」原則と整合)。

### 決定4(charset軸・MS-IME): レジストリの無変換キー割当てを見て、IME ON/OFFへのカスタマイズなら決定1のopen軸機構を横取り実装で肩代わりし、既定のかな切替のままならGJI利用を推奨するポップアップを出す。

**決定3と同じ枠組み(自動判定を主UXとし、必要な判断のみポップアップで問う)を、
MS-IMEにも適用する。** ただしMS-IMEにはGJIのconfig1.dbに相当するawase側から
編集可能なキーマップ機構が無いため、§D3.2のような「専用Fnキーで安全に置き換える」
設計はMS-IME向けには存在しない。代わりに、**MS-IMEにはawase自身が既に確立した
open軸の冪等機構(決定1、`VK_IME_ON`/`VK_IME_OFF`をbeliefに基づいて送信する。
config1.db相当の追加セットアップ一切不要、全環境で動作確認済み)がある**ため、
これを転用できる場面ではそちらを使う。

**ベストエフォート助言は、同じレジストリ読み取り(`enabled`/`muhenkan_ime_off`、
`msime_key_assignment.rs::read_from_registry()`)から2つの分岐を導く:**

| レジストリ状態(`enabled && muhenkan_ime_off`) | 意味 | awaseの対応 |
|---|---|---|
| 真(無変換=IME ON/OFFへカスタマイズ済み) | ユーザーは無変換キーにIME ON/OFFの意味を明示的に与えている。素通しするとMS-IMEのネイティブ処理がawaseのbeliefと無関係にIME状態を変えてしまう(既知の二重オーナー問題) | **新設(今回の着想)**: 素通しせず、awase自身が物理無変換単独打鍵を検知して抑制し、代わりに決定1のopen軸機構(belief駆動の`VK_IME_ON`/`VK_IME_OFF`送信)で同じ意図を安全に実現する。ユーザーの意図(無変換=IME ON/OFF)はそのまま尊重しつつ、実現手段をMS-IMEのネイティブ処理からawase自身の冪等機構へ差し替える。**既存の`conflict_warning()`によるカスタマイズ解除の呼びかけは不要になる**(警告して回避を頼むのではなく、awase側で安全に肩代わりする)。`henkan_ime_on`(変換=IME ON)も対称に扱う。 |
| 偽(無変換=既定のかな切替のまま) | 素のパススルーを選ぶと2026-08-07インシデントと同型のリスクがある(この場合はawase側に肩代わりできる冪等な代替機構が無い、かな形状軸はGJIのようなstatus別解釈委譲が無いため) | 前回設計のまま: ユーザーが無変換単独打鍵を「パススルー」に設定している場合のみ、起動時にGJI利用を推奨するポップアップを表示する(「無変換キーのかな切替がMS-IME既定のままパススルー設定になっています。この組み合わせは意図しないIME切替を招くことがあります。GJI〈Google日本語入力〉の利用を推奨します」といった内容) |

いずれもレジストリへの書き込みは行わない(既存方針を維持)。上段の肩代わり機構は
`muhenkan_solo_tap_always_suppress`とは独立した経路とし、決定3 §D3.6で述べた
「専用Fnキー変換」と同様、`src/engine/nicola_fsm.rs`の単独タップ確定処理で
既存のsuppress判定より手前に分岐させる。

- 物理DBEキー(`VK_DBE_HIRAGANA`/`KATAKANA`/`ALPHANUMERIC`等)へのBUG-52対応
  Suppressは、決定3の新設ユーザー設定(§D3.6)の対象に含める(既定は「抑止」で
  現状維持)。
- 動作の質(意図通りに切り替わるか、スプリアスな挙動が無いか)は、パススルーを
  選んだ場合(下段)はMS-IME自身の実装に依存し、awaseは保証しない(自己責任)。
  上段(IME ON/OFFカスタマイズ済み)の肩代わりは決定1の既存冪等機構を使うため、
  他のopen軸制御と同水準の動作を期待できる。

---

## 3. `CharsetSlot`の設計変遷 — MS-IMEからGJIへの付け替え、完全パススルーへの撤回、そして設定+助言方式への再収束

### 3.1 第1の付け替え: MS-IME向け`ImmSetConversionStatus`書き込み → GJI向けFnキー変換

2026-08-13の同セッション内で、Fable(レビュアー)×Opus(設計者)のpre-mortemを
**5ラウンド**回し、MS-IMEの物理DBEキー押下(無変換等、BUG-52修正以降
`ime_actuation_owned`なら無条件Suppressされ意図が握りつぶされている)を、
awase自身がbeliefに基づいて`ImmSetConversionStatus`へ冪等に再発行する
`CharsetSlot`という機構を設計した(INV-54〜77、S27〜S47、P23〜38、
訂正1〜11を含む詳細設計。実コード裏取り済み、round5でCONVERGED判定)。

**この設計(MS-IME向け、`ImmSetConversionStatus`書き込み)は不採用とし、
`CharsetSlot`という概念自体はGJI向け(物理Muhenkan単独打鍵→Fnキーへの変換)へ
付け替えた。** 撤回ではなく対象の変更であり、「物理キーの握りつぶされた意図を
冪等な書き込みへ変換する」という`CharsetSlot`の中核アイデア自体は維持された。

**MS-IME向け実装を採らなかった理由**:

1. MS-IME向けの`CharsetSlot`は、物理キー押下を正しい`ImmSetConversionStatus`
   書き込み値へ変換するために、**awase自身が「今composition中かどうか」を
   判定する必要があった**。この情報はawaseから安定して観測できない
   (TSFネイティブアプリではcomposition context自体がIMM32互換レイヤーに
   現れない、候補ウィンドウ可視性はcomposition開始前のかな入力段階を
   捉えられない、等)。判定できない条件で安全に分岐する変換機構は作れない。
2. GJIには、awase側の判定を不要にする仕組み(config1.dbの`custom_keymap_table`
   がstatus別にコマンドを解釈する)が既に存在し、かつ実機でF15-F19の効果が
   検証できた。同じ「物理キー→冪等書き込み」という変換アイデアを、
   awase側の状態判定が不要なGJI向けに転用すれば、判定不能問題を回避できる、
   という見立てだった(この見立て自体は§3.2で述べる通りさらに後で覆る)。

**破棄しない情報**: MS-IME向け`CharsetSlot`設計時に確定した実機事実(BUG-52の
無条件Suppress挙動、MS-IMEでのIMC write到達性、TsfNative/ImmCrossの
idle-conv-check非対称性等)は`docs/known-bugs.md`・既存ADR(084/086/087/088)に
既に記録されており、失われない。INV-54〜77・S27〜S47・P23〜38の番号空間は
**MS-IME向けの意味では本ADRで使用しない**。MS-IME向けの`ImmSetConversionStatus`
ベース書き込みを将来再提案する場合は、このセクションを読み、composition中
判定不能問題が未解決のままであることを踏まえること。

### 3.2 第2の付け替え: GJI向けFnキー変換(F15-F19→F21-F23) → 完全パススルーへ撤回

GJI向けに付け替えた`CharsetSlot`は、実際にF21-F23への二段バインド(Precomposition=
絶対指定/Composition・Conversion=ネイティブトグル)・belief同期(ヒューリスティック
ゲート+定期再同期)まで設計し、機構としての到達性を実機で検証した(§1.4)。
その後のOpusレビューで、mozc本家のMS-IME互換プリセットが物理`Muhenkan`1キーに
対し**awase側のbeliefを一切使わない自己完結的な3状態サイクル**を既にネイティブで
提供している(`ms-ime.tsv:151`/`49`)ことを指摘され、既存コード調査で
(a) かな形状の値を消費するawase側の機能が無い(トレイ表示のみ)、(b) NICOLAエンジンON中は
既存のADR-084/086 force-writeがconv modeを強制固定するためこの軸が実質作用しない、
の2点が追加で判明した(当時の見立て)。

**これらを総合し、GJI向け`CharsetSlot`も撤回し、かな形状軸はawaseがbeliefを
一切持たない完全パススルーへ反転した。** F21-F23への二段バインド設計・
実機検証結果自体は無駄ではなく、「SendInputでGJIのcharset軸を絶対指定で確実に
動かせる」という機構としての実現可能性の証明として、§1.4に保存した
(将来、何らかの理由でawase側の能動的な介入が必要になった場合の再利用材料)。

### 3.3 第2回Opusレビューで判明した「完全パススルー」の誤り、設定+助言方式への再収束

第2回Opusレビューで、§3.2の「完全パススルー」自体が実コードと矛盾することが
判明した:

1. (a)の「消費先はトレイ表示のみ」は誤りだった。`state/conv_classify.rs`の
   `has_katakana`が`ConvSyncReason::KatakanaShadowOff`(エンジンopen belief推論)・
   BUG-50の復旧理由選択に使われている。ただしこれは既存の観測(`ConvModeMgr`)を
   読むだけで新しいbeliefを必要としないため、実害のある衝突ではなく単なる
   記述ミスと判明した(決定3 §D3.3で訂正済み)。
2. (b)の「force-writeがRomajiHiraganaへ強制ロックする」は誤りだった。
   `ConvModePolicy`は既定`Observe`(Force化はオプトイン)であり、Force時の
   ターゲットもトレイが選んだモードであって固定`RomajiHiragana`ではない。
3. **最も重大な誤り**: 「無変換・Hiragana・Katakana物理キーは単純にパススルー
   できる」という前提そのものが、実在する2つの保護機構と正面衝突していた——
   `muhenkan_solo_tap_always_suppress`(既定`true`、2026-08-07実機: MS-IMEの
   「キーとタッチのカスタマイズ」既定かな切替への横取り)と、BUG-52のDBEレンジ
   Suppress(2026-08-05実機: 物理「IME ON」キーがOS側の状態依存トグル変換で
   `VK_DBE_KATAKANA`を誤生成しMS-IMEが仕様通りカタカナへ切り替わる)。
   いずれもawaseのbelief管理とは無関係な、パススルー自体が引き起こす副作用への
   既存の防御であり、「観測も強制もしない」という思想を採用してもこのリスクは
   消えない。

**これを受けて、ユーザーとの対話で最終的に決定3 §D3.1-D3.6の「設定+ベストエフォート
助言+任意の設定支援」という3層構成に収束した。** 中心的な技術的着想は、素の
Muhenkan/DBEキーを送らず、GJIのconfig1.dbに新設する専用Fnキー(Precomposition
未バインド・Composition/Conversionのみ`SwitchKanaType`)を代わりに送るという
決定3 §D3.2の設計であり、これにより既存の2つの保護機構に一切触れずに
Composition中のかな形状トグルという新機能を安全に提供できる。**「beliefを
持たない」という結論そのものは維持されている**(専用Fnキー方式はGJIの実際の
内部状態を読むだけで、awase側の予測beliefを必要としない)。

---

## 4. 移行計画

### Phase 0(記録のみ)

1. 本ADRを`docs/adr/091-*.md`として追加し、`docs/adr/index.md`に登録する
   (既存の`index.md:94`はF15-F19/`CharsetSlot`前提の古い要約になっているため、
   本ADRの最終版〈設定+ベストエフォート助言方式〉に合わせて書き換える)。
2. `docs/known-bugs.md`に、config1.dbの残骸バインド(F13/F14/F21/F22。§1.4項目3で
   検出・削除済みの旧awase実験由来のもの)が実機に残存しうる既知の事実として
   記録する(次にconfig1.db関連の作業をする際、同じ残骸に驚かないため)。

### Phase 1(自動判定・config1.db書き込み・ポップアップロジックの実装)

**実装状況(2026-08-14 追記)**: 項目2(隠し設定、`DbeModeKeyPolicy`/
`muhenkan_solo_tap_dedicated_fn_key`)と項目4(専用Fnキー変換モード本体)は
実装済み(`645473b2`〜`d33127f8`、4コミット、`develop`未マージ)。項目1
(config1.db自動判定)・項目3(`awase-gji-config`配線・書き込み)・項目5
(GJI向けポップアップ)・項目6(MS-IME向け決定4)は未着手。実装済み分は
既定無効(手動でVK名を指定した場合のみ動作、既定挙動は変化なし)。
専用Fnキーの許可範囲は`VK_F15`-`VK_F24`(`VK_F13`/`VK_F14`はターミナル
エスケープシーケンス漏れの実機確認によりADR-057で危険と確定、常に除外。
`VK_F21`/`VK_F22`はVK自体は安全だがBUG-64の残骸バインドと同番号のため、
項目3の衝突検出が入るまではユーザーが手動でGJI側の既存設定との衝突を
確認する運用、`src/config.rs::validate_dedicated_fn_key`)。

**項目1実装時に必ず考慮すること(ユーザー指摘、2026-08-14)**:

- **config1.db書き込みはGJIプロセス再起動まで反映されない。** 項目3の
  書き込み直後に項目1の自動判定を再実行しても、GJI側は新バインドを
  まだ認識していない(反映待ちの過渡状態)。書き込み後にこの過渡状態を
  ログ/UIでどう扱うか(「反映待ち」を示す、GJI再起動を促す、あるいは
  awase側でGJIプロセスの再起動を試みる等)を項目5のポップアップ設計と
  合わせて決めること。
- **GJI未インストール環境(config1.db自体が存在しない)の分岐を必ず正しく
  扱う。** ファイルが見つからない場合は例外を投げず、「専用Fnキー変換」を
  無効のまま(既定の「抑止」)にフォールバックし、ポップアップも出さない
  (`awase-gji-config`の既存パーサーは「パース失敗は常に空の結果に
  静かにフォールバック」という設計方針を既に持つ、`lib.rs`のdoc参照。
  項目1のファイル探索自体もこの方針を踏襲すること)。

1. GJI向け自動判定ロジックを実装する: GJI検出時にconfig1.db
   (`session_keymap`/`custom_keymap_table`)を読み、§D3.2のFnキーバインドの
   有無を確認する。存在すれば「専用Fnキー変換」モードを自動的に有効化する
   (ユーザー操作不要)。設定画面には現在有効なモードの表示のみを追加し、
   持続的な選択項目としては前面に出さない(D3.1、設定画面の煩雑化を避ける)。
2. `Hiragana`/`Katakana`物理キー(BUG-52のDBEレンジSuppress対象)・`Eisu`/
   `Shift+Muhenkan`の「抑止/素のパススルー」、無変換単独打鍵の
   「抑止/素のパススルー/専用Fnキー変換」を、設定ファイル上の隠し/上級者向け
   項目として追加する(既存bool群への相乗りではなく専用struct/enumで表現、
   §D3.6)。既定は全て「抑止」、現状維持。**[実装済み]**
3. `awase-gji-config`を実際の呼び出し元に配線する(現状どこからも依存されて
   いない単体crate。config1.dbのパス探索、依存追加、GJI検出フックへの接続を
   含む)。書き込み機能を追加し(バックアップ・原子的置換・既存バインドとの
   衝突検出込み)、§D3.2の専用Fnキーエントリ(例F21、Precomposition・
   DirectInputは未バインド、Composition/Conversion/Prediction/Suggestionは
   `SwitchKanaType`)の追加を、後述のポップアップでの明示的な同意を経てのみ
   実行する(自動・無断では書き込まない)。
4. 「専用Fnキー変換」モードを実装する。`muhenkan_solo_tap_always_suppress`の
   早期returnより手前(`src/engine/nicola_fsm.rs`の単独タップ確定処理)で
   分岐する独立経路とし、既存のsuppress判定を経由しない(C1)。有効時は
   物理無変換単独打鍵の検知ごとに、素のVK_NONCONVERTの代わりに専用Fnキーを
   送る。beliefは不要(決定3 §D3.2)。**[実装済み]**
5. GJI向けポップアップ(新規、決定3 §D3.2「設定未完了時のポップアップ」):
   無変換単独打鍵がGJI既定の「文字種変更」動作のまま手動で「素のパススルー」に
   設定されており、かつ§D3.2のFnキーバインドがconfig1.dbに存在しない場合、
   設定完了を促すポップアップを表示し、同意されれば項目3の書き込みを実行する。
6. MS-IME向け肩代わり機構+ポップアップ(新規、決定4): `msime_key_assignment.rs`の
   既存レジストリ読み取り(`enabled`/`muhenkan_ime_off`、`henkan_ime_on`)を、
   2つの分岐の判定に再利用する。
   (a) `enabled && muhenkan_ime_off`(無変換=IME ON/OFFへカスタマイズ済み、
   `henkan_ime_on`も対称に扱う)の場合は、物理無変換単独打鍵を検知して素通しを
   抑制し、代わりに決定1のopen軸機構(belief駆動の`VK_IME_ON`/`VK_IME_OFF`
   送信)を発火させる肩代わり経路を実装する。`muhenkan_solo_tap_always_suppress`
   とは独立させ、決定3 §D3.6の「専用Fnキー変換」と同様、
   `src/engine/nicola_fsm.rs`の単独タップ確定処理で既存suppress判定より手前に
   分岐させる。これにより既存の`conflict_warning()`によるカスタマイズ解除の
   呼びかけは不要になる(ユーザーの意図は尊重したまま安全に実現する)。
   (b) `!(enabled && muhenkan_ime_off)`(無変換=既定のかな切替のまま)は
   新規ロジックで判定し、この条件が真・MS-IMEがアクティブIME・ユーザーが
   無変換単独打鍵を「パススルー」に設定している場合、起動時にGJI利用を
   推奨するポップアップを表示する。
7. `VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`がmozcのキー名語彙上どの物理トークンに
   対応するか(あるいは対応が無いか)を確認する(`key_parser.cc`の`kSpecialKeyMap`に
   DBE系トークンは存在しないと確認済み、追加調査不要、注記のみ)。

### Phase 2(実機ソークでの確認)

1. §D3.2の専用Fnキー(Precomposition/DirectInput未バインド、Composition/
   Conversion/Prediction/Suggestionは`SwitchKanaType`)を実機に投入し、
   未バインド状態時の実際の挙動(フォーカス中アプリへ実害なく流れるか、GJIが
   何らかの形で処理するか、第3回Opusレビュー指摘)、Composition/Conversion時に
   破壊的な影響(意図しない確定・文字列破損)なくトグルすることを確認する。
2. パススルー設定を有効化した場合、`muhenkan_solo_tap_always_suppress`/
   BUG-52 Suppressを無効化した状態でGJI環境における実際のリスク(2026-08-05/
   2026-08-07の各インシデントがGJI環境でも再現するか)を確認する。
3. ベストエフォート助言ロジック(GJI/MS-IME双方)が誤った推奨を出さないか
   (config1.dbの`session_keymap`列挙値とプリセット名の対応関係、MS-IME
   レジストリ判定の誤検知・見逃しを含め)実機で確認する。

### Phase 3(ドキュメント化)

ユーザー向けドキュメント(`docs/usage.html`等)に、以下を明記する:

- 無変換単独打鍵・`Hiragana`/`Katakana`・`Eisu`/`Shift+Muhenkan`の抑止/パススルーは
  設定可能であり、既定は安全側(抑止)であること。無変換単独打鍵には追加で
  「専用Fnキー変換」(GJI向け)の選択肢があること。
- パススルーを選ぶとMS-IME/GJI側の副作用(BUG-52等)が発生しうること、awaseは
  ベストエフォートで推奨設定を助言するが動作を保証しないこと(自己責任)。
- GJI使用時は「MS-IME」プリセット+§D3.2の専用Fnキー構成を推奨する旨(決定3 §D3.5)。
- MS-IME使用時、無変換キーが既定の「かな切替」のままパススルーを選ぶと、
  起動時にGJI利用を推奨するポップアップが出ることがある旨(決定4)。

---

## 5. 関連ファイル

- `crates/awase-gji-config/`(`command.rs`/`keymap.rs`/`tsv.rs`/`wire.rs`) —
  読み取り機構は既存。config1.db書き込み機能(§D3.2の専用Fnキー追加支援用)を
  Phase 1で新設する。
- `crates/awase-windows/examples/gji_composition_probe.rs` — 本ADRの検証(§1.4)に
  使った自前IMM32プローブ。「完全パススルー」版`CharsetSlot`は不採用になったが、
  SendInputのGJIへの到達性・`ImmGetConversionStatus`による確認可能性の実証記録
  として使い捨てツールのまま残す。§D3.2の専用Fnキー構成の実機検証(Phase 2)にも
  再利用する。
- `crates/awase-windows/src/runtime/transport.rs` — BUG-52対応のDBEレンジ
  無条件Suppress。Phase 1で決定3 §D3.6の新設ユーザー設定によって条件分岐する
  よう変更する(既定は現状維持の「常にSuppress」)。
- `src/config.rs` — `muhenkan_solo_tap_always_suppress`(既存)。Phase 1で
  `Hiragana`/`Katakana`/`Eisu`/`Shift+Muhenkan`向けの対になる設定項目を追加する。
- `crates/awase-windows/src/msime_key_assignment.rs` — MS-IMEキー割当ての
  レジストリ読み取り+警告(`check_and_warn()`)。決定4 §D3.1のベストエフォート
  助言ロジックがこの既存パターンを踏襲する(実装追加は不要、位置づけの追認のみ)。
- `crates/awase-windows/src/state/conv_classify.rs` — `has_katakana`が
  `ConvSyncReason::KatakanaShadowOff`(BUG-50)に使われている。既存の観測
  ベースの機構であり、決定3が新設する設定・Fnキーには影響を受けない(§D3.3)。
- `crates/awase-windows/src/state/eisu_recovery.rs` — かな⇔英数境界の
  belief回復(ObservedEisu、BUG-57)。決定3 §D3.4で、かな形状とは別軸として
  参照するのみで変更しない。
- `docs/known-bugs.md` BUG-14(MS-IME打鍵毎DBEキー送出)・BUG-25(GJI
  SendInput/IMC write到達性)・BUG-50(カタカナ復旧デッドロック)・BUG-52
  (物理DBEキー漏洩、2026-08-05実機)・BUG-57(ObservedEisu救済)・
  BUG-61(romaji軸解決不能)・BUG-62(Alt+かなswallow)。muhenkan_solo_tap_always_suppress
  導入根拠の2026-08-07実機事象(既知バグ番号未確認、`src/config.rs:217`の
  docコメント参照)。
- `docs/adr/057-gji-keybind-f13f14-to-f21f22.md`(F13/F14を避ける根拠。
  F21/F22が安全な理由の直接的な根拠として§1.4/§3で参照)、
  `docs/adr/067-vk-ime-on-off-migration.md`(config1.dbバインド撤廃の経緯)。
- mozc本家ソース(`github.com/google/mozc`、Apache-2.0、2026-08-13時点の
  masterブランチを直接確認):
  - `src/data/keymap/ms-ime.tsv`/`atok.tsv`/`kotoeri.tsv` — 3プリセットの
    キーバインド定義。決定3 §D3.2/§D3.5の直接的な根拠。
  - `src/session/session.cc::Session::CompositionModeSwitchKanaType`/
    `SwitchKanaType`/`SwitchInputMode` — かな循環の実装。`SwitchInputMode`が
    永続`input_mode`を書き換えるのに対し`SwitchKanaType`(Composition/
    Conversion用)はそれを書き換えない、という非対称性の根拠(決定3 §D3.2)。
  - `src/composer/composer.cc::Composer::ToggleInputMode` — 英数トグル
    (ひらがな⇄半角英数の二値トグル、全角英数は経由しない)の実装。
  - `src/composer/key_parser.cc::kSpecialKeyMap` — キートークン語彙。F24上限、
    DBE系・マルチメディア系トークンが存在しないことの根拠(§1.4項目3類)。

## 6. 関連ADR

- ADR-067: open軸のVK_IME_ON/OFF移行。本ADRのopen軸の結論はこれを維持。
- ADR-084/086: conv-mode単一所有権・force-write。**当初は「エンジンON中は
  ConvModeAuthority::AwaseOwnedがconv modeをRomajiHiraganaへ強制ロックするため
  かな形状の切替が実質無意味になる」という論拠に使ったが、第2回Opusレビューで
  誤りと判明した(`ConvModePolicy`既定は`Observe`でForce化はオプトイン、Force時の
  ターゲットもトレイ選択モードであって固定`RomajiHiragana`ではない)。この論拠は
  撤回し、決定3は§D3.2の専用Fnキー構成に一本化した。**
- ADR-088/089/090: 軸capabilityモデル・型状態パターン。本ADRのcharset軸の
  「GJI推奨、抑止/パススルーはユーザー設定+ベストエフォート助言、awaseは新しい
  beliefを持たない」という結論は、これらが提案していた「プロファイルごとの
  能力表」という考え方の実践例と位置づけられる。
