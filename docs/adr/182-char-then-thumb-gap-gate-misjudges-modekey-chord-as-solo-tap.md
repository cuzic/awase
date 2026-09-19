---
id: ADR-182
title: |-
  文字キー→親指(無変換/変換)の押下間隔が閾値をわずかに超えると、重なって押された
  チョードが「文字単独+無変換単独タップ」に割れ、生の無変換がGJIへ届いて半角英数化・
  エンジン非活性へ連鎖する不具合の設計
summary: |-
  ADR-179のPassthrough実験(実機dragonflyg4、GJI、Windows Terminal/TsfNative、
  無変換=左親指)のawase.log(2026-09-19 07:03〜07:42 UTC、約85k行)解析で発見。
  「文字キー押下→親指キー押下」64件のうち、`NicolaFsm::step_pending_char_thumb`
  (nicola_fsm.rs:1718)の入口ゲート`TimingJudge::is_simultaneous`(押下間隔<n-gram調整
  済み閾値、既定で実効80〜120ms)を通れず`PendingThumb`へ再投入されたものが4件
  (間隔90.1/92.1/93.4/108.8ms、成功60件の最大は90.7ms)。4件とも文字キーは親指押下後
  も押され続けており、文字は単独確定・無変換は`handle_key_up_pending`
  (nicola_fsm.rs:2945)から単独タップ解決されて生の無変換がOSへ渡り、GJIの無変換割り当て
  で半角英数(conv=0x10)化→`ObservedEisu`→`Engine deactivated`→以降の無変換+文字が素通り、
  と連鎖する。入口ゲートは「これから何ms重なるか」を原理的に知り得ないため、決定1は
  **重なり量の閾値ではなく二値**（「文字保留中に来た親指の単独タップは、優先順位3・4の
  IME操作生成経路に入れない」）とし、フラグは`PendingThumbData`に持たせて
  `resolve_pending_thumb_as_single`内で判定する。閾値超過時の判定保留・閾値引き上げは
  実測後に別途判断する。鏡像（親指先押し→文字が閾値超過、`step_pending_thumb_char`）は、同一押下が
  solo tap（`Key(29)`）とshift（親指面のかな）の両方に使われる自己矛盾なので、決定1bとして
  同じフラグで抑止する。
status: |-
  **ドラフトv9(round1〜8反映。決定1c実装済み、実機A/B前)**。実装未着手。
  ユーザー意図は実機検証（2026-09-19 17:43〜17:46 JST）で確認済み（失敗は全てチョードの意図）。
related_adr:
  - "ADR-179"
  - "ADR-112"
  - "ADR-120"
  - "ADR-092"
  - "ADR-147"
  - "ADR-153"
  - "ADR-181"
---

# ADR-182: 文字→親指の押下間隔ゲートによるチョード誤判定と生無変換の漏出

## ステータス

**ドラフトv9（2026-09-19）。** 実機ログ解析（「観測」）に基づき、
`opus-adversarial-consult` round1〜8の指摘とユーザー判断（決定1c）を反映した版。決定1・1b・1cは実装済み
（実機A/B前）。round9で収束確認
してから確定する。

## 観測（2026-09-19 実機ログ、dragonflyg4）

環境: GJI、Windows Terminal(TsfNative)、`nicola_keytop.yab`、`simultaneous_threshold_ms`
=100、n-gramモデルあり、無変換=左親指(vk 0x1D)、変換=右親指(0x1C)。直近の実験コミット:
`f0e36b0e`（Henkan/Muhenkan×Suppress設定の完全無視）、`c0814776`（親指キー設定×IME OFF時も
PhysicalDeliveryに一般化）。

**証拠の所在**: 解析対象の`awase.log`（約85,000行、07:03〜07:42 UTC）は、解析後に
awase が再起動されたため Windows 側では上書きされ現存しない（現存ログは07:51 UTC以降）。
本ADRの数値は解析時に抽出した`gaps.log`等（作業用の抜粋、リポジトリ外）に基づく。
実装前に`tests/journals/`へ最低限の再現入力を固定すること（検証計画参照）。

`engine-input`の`vk=0x1D KeyDown state=PendingChar(...)`（文字キー保留中に無変換が押された）
全64件を、文字キーとのts差と直後のjournal `state_after`で突き合わせた。

| 結果 | 件数 | 文字→無変換の押下間隔 | 備考 |
|---|---|---|---|
| `PendingCharThumb`（チョード候補に入る） | 60 | 0.5〜90.7ms | 重なり最小10.2ms |
| `PendingThumb`（文字を単独確定し親指を新規保留） | 4 | 90.1 / 92.1 / 93.4 / 108.8ms | 重なり 28.8 / 12.1 / 5.1 / 0.8ms |

失敗4件（UTC）: 07:28:09.155(J)、07:35:43.744(D)、07:36:46.713(D)、07:37:45.153(S)。
閾値は基準100msをn-gramで`(base±ngram_adjustment_range_ms(20)).clamp(30,120)`に
調整するため既定の実効範囲は80〜120ms（`ngram.rs:224-239`、`config.rs:484-493`）、
キーごとに違うので90.1msで失敗・90.7msで成功のような逆転が起きる。重なりは成功側にも
10.2〜29.0msの値があり、**成功/失敗を分離するのは重なりではなく押下間隔**である。

### 観測上の注意（round3 B6・N4への対応）

- **`physical="Allow"`は「OSへ届いた」ではない。** journalの`physical`は
  `PhysicalKeyDisposition::plan`（transport.rs）の結果で、`plan`が`Suppress`にするのは
  `VK_DBE_*`/F2などKANJI系のIMEモードキーだけ（`non_kanji_event_always_allowed`テスト）。
  無変換/変換はKANJI系ではないので常に`Allow`と記録されるが、それは「配送機構が追加で
  Suppressしない」の意味で、`Decision::Consume`のイベントは`kp_stage_execute`→
  `execute_from_hook`でフックが消費する。反証: 同じログで`Char`キーの多数の
  `decision="Consume" physical="Allow"`（例: tail.log 07:38:01.353 の I キー、出力は`ぐ`で
  生の`i`は出ていない）。round3のレビュアーはこれを「物理の無変換は毎回OSへ届いている
  →二重配送」と読んだが、この読みは採らない（round4でレビュアーが`execute_relay`のConsume腕が`physical`を見ないことを
  確認し、撤回した）。
- **`b5.log`は161行のフィルタ済み抜粋**で、`send_keys`行も`KeyDown`行の`physical=`も
  含まない。この抜粋では「どのタップが合成キーを出したか」は確認できない。08:09〜08:11は
  awase再起動後で設定が失敗4件の時間帯と同じとは限らない（無変換の合成`Key(29)`が
  1件も無い）ため、conv変化と単独タップの対応付けには使えない。

### 実機検証（2026-09-19 17:43〜17:46 JST = 08:43〜08:46 UTC、debugログ、pid 42332）

ユーザーが文字先押し・親指先押し・単独タップを意図して打ち、**「失敗したのは全部チョードのつもり
だった」**と申告した。ログ（`awase-verify-adr182-20260919-174657.log`に保存、Windows側）から:

| 種別 | 件数 | 結果 |
|---|---|---|
| 文字→無変換（文字先押し） | 24 | 成功21（押下間隔2.8〜70.3ms）、**失敗3**（D、間隔80.7/87.3/92.6ms、重なり87.4/88.3/46.1ms）。失敗3件とも`PendingChar→PendingThumb`、`[Char('て')]`の後に`[Key(29), KeyUp(29)]`（1.5ms以内の連続対）。 |
| 無変換→文字（親指先押し、`Char`のKeyDownで`state_before`が`PendingThumb`） | 72（無変換48・変換24） | 成功71、**失敗1**（08:46:34.232、間隔58.9ms、`[Key(29), Char('な')]`）。同じ`な`で58.8msは成功しており、n-gram調整後の閾値が約58.8msだったことになる。 |
| 単独タップ（Idle起点） | 数件 | 106ms保持でKeyUp解決（08:46:34.715）→`[Key(29), KeyUp(29)]`、108ms保持でタイムアウト（08:46:35.789）→`[Key(29)]`+`[KeyUp(29)]`。設定どおりの生無変換。 |

- 失敗3件（文字先押し）は重なりが87/88/46msと長く、ユーザー申告どおりチョードの意図が明確。
  **決定1の前提（意図が本当にチョードか）は、このセットで確認された**（元の失敗4件との対応は
  ログの粒度の違いで取れないが、同じ機序・同じ間隔帯）。
- 親指先押しの失敗（58.9ms）は文字先押しより短い間隔で起きる。文字先押しの閾値が80〜93msで
  割れたのに対し、`な`のn-gram調整後閾値が約58.8msと低かったため。決定1bの対象。
- 変換（0x1C）の文字先押しでも`PendingChar→PendingThumb`が3件（08:43:57.071、08:45:17.553、
  08:45:52.354）あったが、いずれもその後の文字と親指がチョード（`っ`/`で`/`お`）になり、
  生の`Key(28)`は出ていない（親指が次の文字と組んで消費された）。**本ADRでは未検証**（意図不明）。
- **半角英数化は、このウィンドウの失敗ケースでは直接には観測できなかった**: 08:46:20〜08:46:41の
  プローブはすべて`conv=none`（対象アプリのIMEウィンドウが取れない。直前の有効な読みは08:46:19の
  `conv=0x19`、直後は08:46:42の`conv=0x19`）で、`Kana/roma → Eisu/roma`の遷移も記録されていない。
  失敗4件（26.767/28.400/31.299/34.232）はすべてこの区間内。08:44〜08:45に見える
  `ObservedEisu 検出`は、直前の`IME OFF (key combo)`（Ctrl+無変換）と対応しており、失敗ケース由来
  ではない。
- **ただし状況証拠がある（round7 M20）**: 失敗クラスタ（08:46:26〜34）の後、08:46:38.200〜41.535に
  BACKSPACE（vk=8）の連打（約100イベント、3.3秒）、08:46:41.689にかなキー（vk=242=0xF2、
  `VK_DBE_HIRAGANA`、PassThrough）、08:46:42.062に`conv=0x19`（ひらがな）への復帰が記録されている。
  「失敗で半角英数になり、半角で出た文字を消して、かなキーで手動復帰した」という筋に整合する。
  **限界**: このクラスタには「clean pair型」（文字先押し失敗3件・単独タップ）と「サンドイッチ型」
  （親指先押し失敗1件、`[Key(29), Char('な')]`）の両方が含まれ、この証拠ではサンドイッチの害の有無を
  切り分けられない（サンドイッチ単独での再現実験の必要性を裏付ける）。失敗ケースの害は、元の
  07:35:43のログ（conv 0x19→0x10、`ObservedEisu`、エンジン非活性）で直接確認済み。

### 修正後の実機A/B（2026-09-19 18:26〜18:28 JST = 09:26〜09:28 UTC、`60832d5d`、debugログ）

決定1・1b・1cを載せたビルドで、同じ設定・同じアプリ（Windows Terminal、GJI+ATOKキーマップ）で打鍵。
ログは`awase-verify-adr182-20260919-182839.log`（Windows側）。

- **生の親指VKの送出はすべて「親指を離した時点の`[Key(29), KeyUp(29)]`の連続対」（19件）で、
  タイマー由来の`[Key(29)]`単独（0件）、`[Key(29), Char(..)]`（0件）は無かった。**
- **親指先押し52件**（間隔2〜750ms、うちタイマー超過136〜750msが14件）は全て`[Char('な')]`のみ
  （親指面のかなだけが出て、生の無変換は出ない）。修正前の検証で1件出ていた`[Key(29), Char('な')]`
  （決定1b）と、100ms超保持のタイマー経路（決定1c）が実機で解消した。
- **オートリピート**: 保持中の同一親指KeyDown 29件が`PendingThumb`のまま無出力で吸収され、生キーの
  連射は無く、離した時点で1回だけ送出された（例: 09:27:35.151〜.213のリピート→35.239に`[Key, KeyUp]`）。
- **単独タップ（Idle起点）・単独長押し**は従来どおり生の無変換が出て半角英数化した
  （`Kana/roma → Eisu/roma`は単独タップ・長押しの解決後にのみ観測、チョードの後には観測されず）。
- **決定1（文字先押しの閾値超過）は実機では未再現**: 文字先押し9件は間隔1.4〜55.3msで全てチョード成立し、
  修正前に失敗した80〜93ms帯の打鍵が得られなかった。`PendingChar→PendingThumb`は変換で1件
  （09:26:59.781、63ms、生Keyなしで解決）のみ。決定1は単体テスト（修正なしで失敗を確認済み）と
  修正前の実機再現（80.7/87.3/92.6ms）に依拠する。

### 修正後の実機A/Bの追加結果（同日18:30〜19:13 JST、しきい値100ms→30ms）

親指先押し（通常しきい値、4ラウンド、累計約15分・チョード500件超）: 生の親指VKの混入は0件
（最大155.6ms、79.7/83.9msの境界付近を含む）。無変換の100ms超長押し+文字（タイマー経路）も生VKは出ず、
OSオートリピートの連射も無かった。変換（delegateあり）の単独タップは従来どおりタイムアウトで
`[Key(28)]`が出る（決定1cの対象外、既知）。決定1（文字先押しの閾値超過）は通常しきい値では打鍵が
再現帯（80〜93ms）に届かなかったため、**`simultaneous_threshold_ms`を一時的に30msへ下げて**再検証した
（実効しきい値は約30〜50ms、`config.toml.bak-adr182-threshold`にバックアップ、検証後に100へ復元）。

- **決定1が実機で効いた**: 10:12:47.876 `D↓`、10:12:47.908 `無変換↓`（間隔31.2ms、閾値約30ms超）で
  `PendingChar→PendingThumb`、`send_keys [Char('て')]`（Dが単独確定、重なり118ms）。親指を離した
  10:12:47.984（`PendingThumb→Idle`）に**生の`[Key(29), KeyUp(29)]`は出なかった**（修正前の同型は
  `[Key(29), KeyUp(29)]`を送出して半角英数化していた）。文字先押し24件中23件がチョード成立、
  失敗の1件がこれ。
- 親指先押し52件（間隔0.3〜81.7ms、しきい値30ms超が7件）は全て`[Char(..)]`のみ（決定1b）。
- 生の`[Key, KeyUp]`14件は全て`Idle→PendingThumb`起点（単独タップ、または文字が先にタイマーで
  単独確定した後に来た親指）。半角英数への遷移は0件。
- 限界: 文字がタイマー（しきい値と同じ長さ）で先に確定した後に親指が来るケースは、親指が`Idle`起点の
  通常のPendingThumbとなるので決定1の対象外（従来どおり単独タップとして扱われる）。

### 07:35:43 の連鎖（1件の詳細）

1. 43.744 D↓(ts差92.1ms)→無変換↓。`step_pending_char_thumb`が`is_simultaneous`=false
   と判定し、D を単独確定（`send_keys Char('て')` 43.746、実VK送出は43.747014の`[h1-run]`）、無変換を
   `PendingThumb`へ。
2. D↑（無変換↓の12.1ms後）、無変換↑。`handle_key_up_pending`（nicola_fsm.rs:2945）が
   `resolve_pending_thumb_as_single`を呼び単独タップ解決（journal seq=9232
   `PendingThumb→Idle`）。
3. 43.772 `send_keys [Key(VkCode(29)), KeyUp(VkCode(29))]`（awaseが合成した無変換の
   単独タップ。`Decision::Consume`のイベントは`physical`に関わらずOSへ届かない
   （`executor.rs::execute_relay`のConsume腕は`physical`を見ずに`Consumed`を返す）ので、
   **これがGJIの受け取る唯一の無変換**）。`て`は43.747014の`[h1-run] vks=[54,45]`で
   すでに送信済み（生VKより25ms前、まだひらがな状態）で、`て`は無傷。
   `[h1-send]`は`output/probe_io.rs:726`の**事後プローブ**（送信後にconvを読み直す）で、
   実際のVK送出は`[h1-run]`（`output/key_injector.rs:293`）である。43.629のプローブは
   conv=0x19、43.798のプローブはconv=0x10（NATIVE=false）で、この間に生の無変換がある。
4. 44.993 idle-conv-check が`conv=0x10`を検出（belief更新は生VK注入の1.2秒後、
   43.798のプローブが既に同じ値を読んでいた）: `belief AssumedRomaji → ObservedEisu`、
   `Engine deactivated (NotRomajiInput)`。**同一ミリ秒に`desired_open := false`
   （`handle_engine_set_open`）と`UserImeSetIntent`(false)がjournalに記録され、
   `apply(open=false)`は`outcome=Unwarranted`（`c8bc1adc`のwarrant）で止まっている**
   （`key_pipeline.rs:1126-1168`）。これはIME ONのままの半角英数をopen軸のOFFとして書く
   別の問題で、決定1では変わらない（別ADR/BUG、決定4の見出し下の「別ADR/BUGとして分離」段落）。
5. 45.299〜 以降の無変換+D/Q は`Idle→Idle PassThrough`（エンジン非活性、
   `[diag-engine-active] ime_on=true なのに非活性`）。

### 失敗4件と通常の単独タップの違い（round1 B1の検証結果）

round1は「失敗4件では`IME open axis delegated (…PhysicalDeliveryFollow)`が1行も出ておらず、
同種の単独タップ22件では出ている。差はbelief追随の有無ではないか」と指摘した。検証結果:

- 同ログ内で`delegated`が出る単独タップは**変換(0x1C、TurnOn)**のもの。無変換(0x1D)の
  単独タップには`delegate_to_open_axis`が設定されておらず（`resolve_delegate_to_open_axis`
  が`Fallthrough(None)`、`ModeKeyConfig`のPassthroughで生`Key(vk)`のみ出る）、
  **無変換の単独タップはこの設定では失敗4件に限らず常に生の無変換をOSへ渡す**。
  これはADR-179のPassthrough実験の意図（無変換をGJIのキーマップに任せる）どおりの動作。
  07:37:56.825の`delegated`は`vk=0x1C KeyUp`と対応（tail.logで確認）。
- 解析後の現存ログ（07:51〜、32,557行）でも `delegated`=7、vk0x1C単独タップ解決=7、
  生`Key(28)`送出=7と一致し、無変換側は`delegated`が付かない。
- したがって「生の無変換がGJIへ届く」こと自体は失敗4件に固有ではない。**失敗4件の問題は
  「チョードのつもりだったものが単独タップ扱いになり、その帰結（生無変換→半角英数）が起きた」
  こと**（意図確認が要る、下記）。無変換にFollowOnly(belief追随)が無く、beliefの更新が
  idle-conv-check頼みで遅れる（手順4の1.2秒）点は別軸の論点としてスコープ外に記録する。

### 生の無変換の効果（round2 B5、ユーザー訂正を反映）

**GJI(ATOKキーマッププリセット)での無変換単独タップの動作（ユーザー申告、ログ外の事実）**:
IME ON・ひらがな から無変換を単独で打つと、**IME ONのまま半角英数(conv=0x10)へ遷移する**。
IMEのopen状態は変化しない。これは仕様であり、ADR-179のPassthrough実験（無変換の単独タップを
GJIのキーマップに任せる）の意図した動作である。したがって半角英数化そのものは症状ではない。

**訂正**: 前版(v3初版)は、現存ログ(08:11)から「生の無変換はKana⇄Eisuの無条件トグルと決着」と
書いたが、**この結論はログから裏付けられていなかったので撤回する**:

- 08:11:07.282の`Kana/roma → Eisu/roma`は、08:11:07.242の無変換↓の**40ms後**にIdleCheckが
  観測したもの。だがこの時点で当該の無変換は`PendingThumb`にあり、生の`Key(29)`は
  まだ送出されていない（Idle起点の単独タップは押下中は保留され、解決時に送出される）。
  idle-conv-checkは次のキーイベントで遅延して状態を読むため、観測は**それ以前の操作の結果**
  である。08:11:13.311の`Eisu/roma → Kana/roma`も同じ理由で、13.278の無変換↓の生VKの
  効果ではありえない。
- 08:09〜08:11のtimeline（`b5.log`）には、`send_keys ... Key(VkCode(29))`の行が1件も
  含まれない（08:10:11.650と08:11:34.377は`IME OFF (key combo)`＝Ctrl+無変換）。
- 定量的な反証もある（round3）: 08:11:16のタップは+808ms後も、08:11:23のタップは+721ms後も
  conv=0x19のまま、08:10:58〜08:11:07は2タップでconv遷移1回（偶数なのに1回）。
- よって「無変換はEisu⇄Kanaの往復トグル」も「08:11:07のタップがEisu化の原因」も未確認。
  07:37:24.993の単独タップの後もengineが活性だった理由（round2 B5）は、**この設定で
  無変換がEisuからひらがなへ戻すのか、他のキー（変換のTurnOn等）で戻したのか**が
  ログからは決められない。ユーザー申告はひらがな→半角英数の方向のみ。

**B5への現時点の答え**: 生の無変換がIME ON・ひらがな→半角英数へ遷移させることは、
ユーザーが確認済みの仕様であり、Idle起点の単独タップも失敗4件も同じ効果を持つ。決定1が
狙うのは「チョードのつもりの打鍵が、意図しない半角英数化を起こす」ことだけで、Idle起点の
単独タップによる半角英数化は意図した動作なので抑止しない（決定1のフラグが立たない）。
07:37:24の単独タップの後にengineが活性だったことは、その時点でGJIが既にひらがな
だった（Eisuから戻していた）という他の説明と矛盾せず、本ADRの決定を左右しない。

- **決定1の有効性は構成上保証される**（round4 A）: `Consume`のイベントはOSへ届かず、合成
  `[Key(29), KeyUp(29)]`がGJIの受け取る唯一の無変換なので、合成を止めればGJIは無変換を
  一切受け取らず半角英数化は起きない。成功チョード60件が無害なのも、`Consume`かつ合成送出が
  無く、GJIが無変換を一度も見ないため。選択肢H（合成送出を常に止める）は選択肢Dと同一。
- 前版は「生VKがかな出力を追い越して`て`が半角`te`になる（順序逆転）」と書いたが、
  `[h1-send]`（事後プローブ）を送信点と取り違えた誤読で、`て`は`[h1-run]`で生VKの前に
  送信済みだった。順序逆転の実例はこのログに無い（選択肢E・決定4を撤回）。

### タイマー経路とKeyUp経路の非決定性（round2 Q5）

07:37:24は148ms保持なのにタイムアウトせずKeyUpで解決（`handle_key_up_pending`）され、
07:37:29は145msでタイムアウト解決（`state_before="Idle"`）された。差の原因は
`message_handlers.rs:636-645`: `OUTPUT_GATE`（awase自身の出力がin-flight）または
`FOCUS_RESYNC`のゲートがアクティブな間、`TIMER_PENDING`は`deferred_engine_timers`へ
drainまで延期される。同じ物理操作がawase自身の出力タイミング次第で
`handle_key_up_pending`(2945)と`timeout_pending_thumb`(3072)のどちらにも流れうる。

### 通常の単独タップの例（決定1の対象外であるべきもの）

07:37:24.844 無変換↓（`Idle→PendingThumb`）→ 24.993 無変換↑（保持148ms、閾値100ms超だが
タイマーより先にKeyUpが届き`handle_key_up_pending`で解決）→ 24.993 生`Key(29)`送出。
文字キーが関与しない通常の単独タップであり、決定1で抑止してはならない。round1は
これを「失敗4件と同型」と見なしたが、`PendingChar`起点ではなく`Idle`起点なので別物
（journal `state_before="Idle" state_after="PendingThumb"`が24.844）。

### 誤検知ではないもの

Ctrl+無変換の`IME OFF (key combo)`（07:28:24、07:33:16）による`Idle→Idle Consume`と、
IME OFF後の無変換+文字の素通り（07:34:38）は設定どおりの動作であり本不具合とは別。

### 未検証の事項

- **ユーザー意図**: 元の失敗4件（07:28〜07:37 UTC）が本当にチョードのつもりだったかは、そのログ
  だけからは分からなかったが、その後の実機検証（上の節）で、同じ機序・同じ間隔帯の失敗3件
  （文字先押し、重なり87/88/46ms）と1件（親指先押し58.9ms）が再現し、ユーザーが全てチョードの
  つもりだったと申告した。決定1・1bの意図に関する前提は確認済みとして扱う。
- 07:36:47.402 `[Key(29), Char('ど')]`、07:37:07 `[Key(29), Char('ー')]`、07:37:35
  `[Key(29), Char('れ')]`、07:38:01 `[Key(29), Char('ぐ')]`、07:40:37/07:40:47
  `[Key(29)]`単独（`execute_from_loop`由来）。前4件は`step_pending_thumb_char`
  （nicola_fsm.rs:1795-1809）の時間超過分岐で親指が単独確定された形で、機序は決定1bで説明する
  （後2件`[Key(29)]`単独は`execute_from_loop`＝メッセージループ＝タイマー由来の送出で、
  `timeout_pending_thumb`が親指を100ms超保持で単独確定した痕跡と整合する。その直後に文字が
  来ていたかは抜粋から追えず、機序は決定1bの「タイマー経路の穴」で扱う）。
- 失敗4件のうち07:28:09(J)は連鎖（Eisu化）まで追っていない。
- `min_overlap_margin_percent`の既定は0（ADR-112決定3で恒久化）。`PendingCharThumb`に
  入りさえすれば文字キーが親指押下後に1μsでも押されていればチョード確定になる
  （`overlap_only_verdict`）。本不具合は重なり判定ではなくその手前の入口ゲートで起きている。
- hook 処理遅延: 07:35:43のDは`delay=92ms`（直前の`[h1-send]`43.629から約114ms hookが
  止まっていた）が、`gap`はハード`ts`差で計算されるため判定は汚れていない。awase自身の
  送信がhookを100ms級で止めている事実は別問題として記録する（本ADRの対象外）。

## 原因

`NicolaFsm::step_pending_char_thumb`（`src/engine/nicola_fsm.rs:1718`）は、保留中の
文字キーに親指が来たとき、`TimingJudge::is_simultaneous`（`elapsed < adjusted_threshold`）
だけで`PendingCharThumb`へ入れるか、文字を単独確定してから親指を再処理する
（`into_reduce_and_continue`）かを決める。

入口時点で分かるのは「文字から何ms後に親指が来たか」だけで、「その文字がこの後何ms
親指と重なるか」は分からない。文字キーは`PendingChar`にある限り**定義上必ず押下中**
（KeyUpが来れば単独確定されて`PendingChar`を出る）なので、「文字が押下中か」は判別材料
にならず、実重なり量の観測点（`PendingThumb`中に届くchar1のKeyUp）は`on_key_up`の
`release_only`（nicola_fsm.rs:2709、`self.state`に触れない契約、同2976-2980）にあり、
そこで状態を書き換えるのは設計違反になる。ADR-112決定3の`overlap_only_verdict`は
`PendingCharThumb`に入った後段の判定で、入口で落ちたケースには一切効かない。

落ちた後の`PendingThumb`は、Idleから来た通常の`PendingThumb`と区別が付かないため、
親指の解決時（`handle_key_up_pending`:2945、`timeout_pending_thumb`:3072ほか）は
`resolve_pending_thumb_as_single`が通常の単独タップとして優先順位1〜4に流し
（`nicola_fsm.rs:2257`のコメント参照）、優先順位4（`ModeKeyConfig`のPassthrough）で生の
親指VKが出る。`resolve_pending_thumb_as_single`の本番呼び出し元は7箇所
（614/1596/1797/1816/2913/2945/3072行）。

## 選択肢

### A. 閾値を上げる（`config.toml`のみ、コード変更なし）

対象はユーザー設定`simultaneous_threshold_ms`（既定100、`config.rs:484`、範囲10〜500）、
`ngram_adjustment_range_ms`(20)、`ngram_min_threshold_ms`(30)、`ngram_max_threshold_ms`(120)。
`.claude/rules/tuning-constants.md`の対象（`crates/awase-windows/src/tuning.rs`）ではない。
実効閾値は`(base±20).clamp(30,120)`なので、108.8msの失敗を救うには`base`と
`ngram_max_threshold_ms`の**両方**を上げる必要がある（例: 120/150）。単独打鍵×2を意図した
高速打鍵の誤チョード化との引き換えになる。コード変更が要らないため、決定1と独立に
ユーザーがいつでも試せる。**規約対象外だが、実測（押下間隔の分布）を添える精神は守る。**

### B. 文字保留中に来た親指の単独タップから、IME操作生成経路を外す（二値版・被害遮断）

`PendingThumbData`に「この親指は文字キー保留中に到着し、文字の単独確定後に`PendingThumb`へ
再投入された」旨のフラグを持たせ、`resolve_pending_thumb_as_single`内でそのフラグが立って
いれば優先順位3（delegate）と4（`ModeKeyConfig`）に入らずno-opとする。チョード自体は
救わず、文字の単独確定（`て`）も変えない。**重なりmsによる閾値は持たない**（round1 B3:
入口時点では重なりは常に真で区別不能、実重なりの観測点は状態不可触）。

### C. 閾値超過時の判定保留（猶予窓）

`is_simultaneous`が偽でも押下間隔が`基準閾値×k`以内なら、文字を確定せず`PendingCharThumb`
（暫定）へ入れ、char1 KeyUpの重なり（`overlap_only_verdict`）とタイムアウトで最終判定する。
チョードを救うが、単独文字の確定が最大で基準100msに対しk=1.5なら150ms待つ（現状も
`PendingChar`は最大100ms待つので増分は最大約50ms）、新しい定数`k`が要る。

### D. Passthrough実験を止める（`muhenkan_solo_tap_always_suppress = true`に戻す）

生の無変換が届かなくなるので連鎖は起きないが、ADR-179の実験の目的を諦める。

### E. 生の機能VKと保留中のかな出力の順序制約（撤回）

「同一打鍵列由来のqueue済みかな出力が実送信されるまで、生の機能VKを出さない」案。動機とした
観測（生VKが`て`を追い越して半角`te`になる）は、`[h1-send]`（事後プローブ）を送信点と
取り違えた誤読で、実例が無い（round4 B7）。理論上ありうるが未観測のため撤回する。

### F. 無変換単独タップにもFollowOnly（belief追随）を付ける（却下）

無変換にFollowOnlyが付かないのは欠落ではなく未設定で正しい（round2 S7）。
`muhenkan_delegate_to_open_axis`は`gji_charset_autodetect.rs`の自動検出が設定する値で、
GJI(ATOKプリセット)の無変換単独タップは、IME ONのままひらがな→半角英数へ遷移する
（ユーザー申告）。open状態が変わらない動作なので、open軸の`ShadowImeAction`では表現できない。**ただし
「GJIのATOKプリセットでは」という限定付き**（round3 M13）: `muhenkan_delegate_to_open_axis`は
`gji_charset_autodetect.rs`（638/722付近）が動的に設定する値で、別のキーマッププリセット
（MS-IME既定に近い割り当てなど）では無変換がopen軸のキーになりうる。本却下は
`delegate_to_open_axis`機構一般の否定ではない。
裏付け（round3 Q3）: 抜粋ログの全`conv observation`行は`open=true`で、conv変化は`NATIVE`
ビットの増減のみ、`Engine deactivated`の理由も一貫して`Inactive(NotRomajiInput)`
（`ime=true`のままromaji不可）であり、`Inactive(ImeOff)`はCtrl+無変換のコンボ1件だけ。
無変換がopen軸を動かしている証拠は1件も無く、open軸で追随させる対象が存在しない
（`conv_classify.rs:44-47`と`key_pipeline.rs:1142-1146`も「conv英数モード観測はIME ONの
確証」と明記）。追随先が無いのではなく**open軸には無い**という意味で、awaseはconv軸の
モデル（`state/conv_mode.rs`、`ObservedEisu`）を持ち、現状はidle-conv-check（生VK注入の
約1.2秒後）の遅延観測が追随を担う（その遅延は本ADRのスコープ外）。
open軸で表現できない動作にFollowOnlyを付ける方法が無いので却下する。

### G. 親指キーだけ別の閾値（round1 M7）

`adjusted_threshold`は`candidate_kana`のn-gramでしか動かず、親指かどうかを見ない。
親指方向のbaseだけを別に持てば文字同士の誤チョード化を避けられるが、新しいconfig項目が
増える。決定2の実測後に検討する。

## 決定（ドラフトv9）

**決定1（採用予定）: 選択肢B（二値版）を実装する。**

- **フラグの意味**: `PendingThumbData.after_char_flush: bool`（仮称）。「この親指は、文字キー
  との同時押し局面で、文字/親指のどちらかが時間超過により単独確定された結果として
  `PendingThumb`になった（または単独確定された）」ことを示す。Idle起点の親指では常にfalse。
- **フラグの立て方（round2 M9で具体化）**: `step_pending_char_thumb`の時間超過分岐
  （nicola_fsm.rs:1754-1757）は`PendingThumbData`を生成せず、`go_idle()`→
  `resolve_pending_char_as_single`→`into_reduce_and_continue(*ev)`で親指を**生イベントとして
  再ディスパッチ**する。したがって`PendingThumbData`は再ディスパッチ先の`Idle`側
  （`enter_pending_thumb`、1226付近）で作られる。立て方は次のどちらかで、
  **(i)を推奨**する。
  - (i) `NicolaFsm`に1イベント寿命のマーカー`next_thumb_after_char_flush: bool`を持ち、
    時間超過分岐で`true`にする。`TimedFsm::decide`（nicola_fsm.rs:1298）の先頭で
    `self.thumb_after_char_flush = mem::take(&mut self.next_thumb_after_char_flush)`
    と2段で受け渡し、`enter_pending_thumb`がその値を`PendingThumbData`へ転写する。
    寿命は「次に`decide`が受け取るトークン1つ」で、`ReduceAndContinue`の再ディスパッチも
    同じ`decide`を通るのでここで確実に消費・クリアされる（立てっぱなしになって以後の
    単独タップを殺すバグの防止、検証計画5で固定する）。
  - (ii) `ClassifiedEvent`にフィールドを足して`remaining`に印を付けて運ぶ
    （`explicit_ime_action_consumed`/`auto_delegate_open_axis_consumed`と同じ流儀）。
    `ClassifiedEvent`のコンストラクタが`awase-windows`側に約20箇所あるので、
    windows層に無関係なコア内部の印が漏れる、コストが大きい。
  なお`resolve_char_and_thumb_as_separate_solos`（2913）が使う`PendingThumbData`は
  `is_simultaneous`=true側（1736-1748）で作られた`PendingCharThumb`由来で、時間超過分岐
  を通らないので**フラグは立たない**（round2 M9: 前版の「別の生成点だから」は誤りで、
  正しくは「立てる経路を通らないから」）。
- **引数追加ではなく`PendingThumbData`経由**: `resolve_pending_thumb_as_single`は`&self`+7個で
  clippyの`too-many-arguments-threshold = 8`（`clippy.toml`）上限ちょうど。同関数のdoc
  （nicola_fsm.rs:2237-2239）が「呼び出し元7箇所+テスト書き換えコストに見合わない」として
  却下した引数化とは別に、フラグを`PendingThumbData`のフィールドとする。**ただし呼び出し元の
  614/1797/1816/2913/2945/3072は`thumb.scan_code`等を個別に渡しており（round3 S12）、
  フラグだけを伝える経路が無い。** そのため実装時に`resolve_pending_thumb_as_single`の
  シグネチャを`(&self, thumb: &PendingThumbData, composing: bool)`へ束ね直す
  （引数8→3でclippyも緩和される）。この束ね直しは呼び出し元7箇所+テストの書き換えを伴うので、
  決定1のコストに含める。`EngineState::debug_label`（fsm_types.rs:438-440）は`vk`/`is_left`
  のみ出力するのでjournalの`state_after`文字列は変わらず、`nicola_fsm.rs:4378`周辺の
  状態全数列挙テーブルの行も増えない。
- **抑止対象と挿入位置**: `resolve_pending_thumb_as_single`の**優先順位3（delegate）と4
  （`ModeKeyConfig`Passthrough）のみ**。ガードは`auto_delegate_open_axis_consumed`の
  早期return（nicola_fsm.rs:2333）の**直後**、`resolve_delegate_to_open_axis`呼び出し
  （2338）の直前に置く（round2 Q3）。表:

  | 行 | 内容 | 決定1での扱い |
  |---|---|---|
  | 2273 | 優先順位1 `dedicated_fn_key`（ADR-091） | 触れない |
  | 2286 | 優先順位2 `resolve_explicit_ime_action`（ADR-153決定1） | 触れない |
  | 2314 | `explicit_action_consumed` → no-op（BUG-123） | 触れない |
  | 2333 | `auto_delegate_open_axis_consumed` → no-op（ADR-154） | 触れない |
  | **新規** | `after_char_flush` → no-op（`no_op_resolution()`） | **ここに挿入** |
  | 2338 | 優先順位3 `resolve_delegate_to_open_axis` | 抑止される |
  | 2343 | 優先順位4 `mode_key_config` | 抑止される |

  新ガードは既存の2つのconsumedガードと同じno-op方向（生キーを出さない）なので、合成しても
  BUG-123/ADR-153/ADR-154の二重actuation防止の不変条件を壊さない（round2 Q3の確認済み）。
  優先順位3のFollowOnly(belief追随)は、生VKを出さなくなるのでbeliefだけ動かすと不整合に
  なるため、優先順位4と対で抑止する。優先順位1・2はユーザーが明示的に設定した動作なので
  触れない。
- **決定1b（鏡像、round2 M8、round5 B9で採用に変更）: `step_pending_thumb_char`
  （nicola_fsm.rs:1795-1809）の時間超過分岐でも同じフラグを立て、生の親指VK（優先順位3・4）の
  送出を抑止する。ただし立てるのは`candidate.is_some()`（到着文字に親指面のかなが存在する、かつ
  `classify_idle_intent`のSpace/Enter親指のshift-literalエスケープハッチ（1499-1505）が発火しない
  限り、再ディスパッチで親指面のかなが出ることと同値）場合だけ。** 機序（round5 B9、コードとログで確認）: この分岐は`consume_thumb`を呼ばない
  （simultaneous側の1783-1785だけが呼ぶ）。`go_idle()`→`resolve_pending_thumb_as_single`
  （ここで`Key(29)`が出る）→文字を`ReduceAndContinue`で再ディスパッチ、再ディスパッチ先の
  `Idle`では親指がまだ物理的に押下中かつ未消費なので`decide_idle`→`IdleIntent::ActiveThumb`→
  `reduce_active_thumb`（1542-1556）が**親指面のかな**を出し、そこで初めて親指を消費する。
  実例（tail.log 07:38:01）: `seq=10413` 無変換↓ `Idle→PendingThumb`、`seq=10414` `I`↓
  （121ms後）`state_before="PendingThumb"`→`state_after="Idle"`（`PendingChar`ではない）、
  `send_keys [Key(VkCode(29)), Char('ぐ')]`。`ぐ`は濁音＝左親指面のかなで、同一の親指押下が
  **solo tap（生`Key(29)`）とshift（親指面のかな）の両方**に使われている。エンジン自身が
  shiftとして消費した押下をsolo tapとしてGJIへ送るのは1打鍵内の自己矛盾（BUG-46型の
  二重使用）で、意図は曖昧ではない。**抑止されるのは`Key(29)`だけで`Char('ぐ')`は変わらない**ので、
  決定1bで失うものは無い（「親指を長押ししてから文字を打つ運用」は親指面のかなが従来どおり出る）。
  決定1（char→thumb、失敗4件）が「親指がshiftとして使われず、solo tap解釈にも筋が通る」ため
  意図が曖昧なのに対し、**決定1bのほうが根拠が強い**。round4で私が置いた非対称理由(1)(2)は
  撤回する: (1)Ctrl+Iの交絡は、直前に`ImmGetContext returned NULL … cancel skipped`があり
  取り消しは実際には行われておらず、composition cancelは入力モード（`NATIVE`ビット）を変えず、
  併記の`marked cold … will send VK_DBE_HIRAGANA`はひらがな方向（round5 M18）。(2)は上記の
  機序に反証される。**決定1bの根拠は、この自己矛盾（(i)）一本で成立する**（round7 M19）: 抑止されるのは
  `Key(29)`だけで`Char('ぐ')`は変わらず、失うものが無く内部矛盾が消える変更なので、サンドイッチの
  害の有無に依存しない。害の証拠（(ii)）は**傍証（未確定）**に留める: プローブは01.254でconv=0x19、
  01.704で0x10を読んでおり、同型4件（07:36:47/07:37:07/07:37:35/07:38:01）の後に半角英数化の
  証拠があるが、これは同じサンドイッチに関する未検証の事項でもある。
  `candidate.is_none()`（親指面にかなが無い文字）の場合は、`reduce_active_thumb`にも
  入らず親指がshiftとして使われないため、従来の挙動（生の親指VKが出る）を維持する。
  決定1と**別コミット**で実装する（決定1cが決定1bのフラグに依存するため、1bのrevertは1cの効果も
  消す。1cだけのrevertは安全）。
- **決定1c（タイマー経路、round6 B10、ユーザー判断（2026-09-19）で(b)を採用）**:
  `consume_thumb`の呼び出し元は1196/1545/1700/1784/2654の5箇所だけで、親指を単独タップとして
  解決する側（`timeout_pending_thumb`3072、`flush_pending`614、1797、1816、`handle_key_up_pending`
  2945）は呼ばない。`active_thumb_side`（3208-3217）は「物理的に押下中かつ未消費」で
  `Some`を返すので、`親指↓ → 100ms経過（TIMER_PENDING発火）→ timeout_pending_thumbが生
  Key(29)を送出（state=Idle、親指は押下中のまま）→ 文字↓ → decide_idle → ActiveThumb →
  reduce_active_thumbが親指面のかなを出して初めて消費`となり、決定1bと同じ二重使用が
  **2回のディスパッチに分かれて**起きる。`Key(29)`の送出が文字の到着より前なので、決定1bの
  フラグ（到着した文字を見てから立てる）では原理的に覆えない。しかも本ADRが記述した
  OUTPUT_GATEによる非決定性（観測「タイマー経路とKeyUp経路の非決定性」）で、同じ運指が
  `step_pending_thumb_char`経路（決定1bが効く）とタイマー経路（効かない）に振り分けられる。
  07:40:37/07:40:47の`[Key(29)]`単独（`execute_from_loop`由来）はこの経路の痕跡と整合する。
  選択肢の性質（round6 A-6）と本ADRの判断:
  - (a)単独タップ解決時に`consume_thumb`する: 後続文字が通常面になり、決定1bの「かなは変わらない」
    と正反対。採らない。
  - (b)タイムアウト経路の生VK送出を親指KeyUpまで遅らせる: 決定1bと同じ意味論だが、全ての単独
    長押しの生VK送出タイミング（レイテンシ、長押しの意味）を変えるので、単独の実測（下記の
    実験）と別ADRが要る。
  - **(c)穴として受容し記録する（当初案、不採用）**: 影響範囲の広い(b)を後回しにする案だったが、
    ユーザーが「今すぐ直す」を選んだ。
  **採用: (b)を、タイムアウトで生VKを送出する無変換/変換に限定して実装する（実装済み）。** 設計:
  - `NicolaFsm::defers_solo_until_release(thumb, composing)`: OS修飾キーでなく、
    `engine_off_solo_repeat_vk`でなく、専用Fnキー・ユーザー明示config（優先順位1・2）・
    `delegate_to_open_axis`（優先順位3）のいずれも持たず、`mode_key_config`がSomeで
    `SoloTapAction::Passthrough`（`for_composing(composing)`）のキー。`mode_key_config`がSomeなのは
    無変換/変換だけなのでSpace/Enter親指は自然に除外される（round8 S29）。**変換に
    `delegate_to_open_axis`（TurnOn追随）が設定されている場合、その変換は対象外**: タイムアウト時に
    belief追随/明示actuationが発火する既存契約（ADR-092決定D、ADR-147、ADR-153）と、それを固定する
    テスト群（17件が失敗した）が「タイムアウトで解決」を前提にしており、送出タイミングを変えない。
    このため、`delegate`を持つ変換ではタイマー経路の二重使用が**残る**（既知の制約）。
  - `on_timeout`の`PendingThumb`腕: 上記に該当すれば`timeout_pending_thumb`を呼ばず、`on_timeout`が
    冒頭で`state`をIdleへ置換しているので`PendingThumb`を**明示的に書き戻し**（round8 S28）、
    `solo_counter.reset()`（`timeout_pending_thumb`のelse節相当）を行い、アクション無しで
    `TimerIntent::CancelAll`を返す。解決は親指KeyUp（`handle_key_up_pending`）か次のキー
    （文字は決定1b、その他は`decide_pending_thumb`）に委ねる。新しい状態は持たない。
  - **OSオートリピート（round8 B11）**: `observe_thumb_watch_window`は統計専用（戻り値`()`）で
    抑止しない。オートリピートKeyDownはFSMまで届き、`step_pending_thumb_thumb`が単独確定して生キーを
    連射する。同じ親指キーのKeyDownで、`defers_solo_until_release`に該当する`PendingThumb`なら
    `ParseAction::Shift { timer: Keep }`で無視するガードを`step_pending_thumb_thumb`に入れた
    （対象を限定するのはSpace等のリピート挙動を変えないため）。
  - 効果: 該当する親指を100ms超押したまま文字を打つと、親指面のかなだけが出て生の無変換は出ない。
    単独の長押しは、生の無変換が**親指を離すまで**遅れる（オートリピートは無視、離した時点で
    `[Key, KeyUp]`を1回）。
  - **KeyUp喪失時の滞留（/code-review指摘）**: タイムアウトを保留した`PendingThumb`は、フォーカス移動
    （`flush_pending`）以外でKeyUpが届かない（hook再起動・取りこぼし）と、タイマー無しで次のキーまで
    残る。次のキーで解決される（文字なら親指面のかなが1回だけ出て以後は回復）ため影響は限定的で、
    従来も親指の物理状態（`phys.left_thumb_down`）はKeyUp喪失で残っていた。最大保持時間のフォール
    バックタイマーは新しい定数（tuning規約の実測義務）を要するので本ADRでは入れず、既知の制約とする。
  - **新設される取りこぼし窓（round8 M22）**: フォーカス移動（コンテキスト境界）では
    `flush_pending`が一律`ThumbRawVkEmission::Denied`（engine.rs:393/573/681、623-629の`Denied`腕は
    無条件suppress）なので、保留された`PendingThumb`は**生キーを出さず黙って消える**。安全側
    （別ウィンドウへの誤注入は無い）だが、「無変換を押したのに半角英数にならなかった」という
    今日は存在しない窓（今日は100msで送出済み）。テストで固定した。
  - `engine_off_solo_repeat`（round8 M23）: 既定は`VK_INSERT`（config.rs:657）、トリガは5回
    （`SOLO_OFF_TRIGGER_COUNT`）なので、既定では除外条件に当たらず無変換/変換の1cは有効。
    無変換/変換に設定するとその親指では1cが無効になりタイマー経路の二重使用が残る（既知の制約）。
  - **決定1bとの結合（round8 S27）**: 1cが行うのは「タイマーで解決しない」だけで、実際に生キーを
    止めているのは決定1bのフラグ（`step_pending_thumb_char`の時間超過分岐）。**1bをrevertすると
    1c下でも文字到着時に生`Key`が出て今日と同じ`[Key, Char(親指面のかな)]`に戻る**（悪化はしないが
    1cの効果も消える。1cだけのrevertは安全）。
  - bypassキー（round8 S30）: `PendingThumb`の滞在が延びるので、Ctrlを後から足す
    Ctrl+無変換の`IME OFF (key combo)`では`handle_bypass`の`flush_pending(.., Allowed)`が生Keyを先に出す
    並びが増える。実機A/Bの観察項目とする。
  **実機A/Bの合格条件**: 無変換を**150〜400ms程度**（Windowsのオートリピート開始前）押したまま文字を
  打ち、生の無変換が出ず親指面のかなだけが出ること。保持時間の上限を超える（オートリピート開始後）
  ケースは、生Keyは離すまで出ない（ガード）が、確認は上限内で行う。
- **誤診断のリスク（round1 S5、round2 A-3）**: 決定1は「文字を押したまま無変換を叩いてIMEを切り替える」
  運用を握りつぶす。文字の確定（`て`等）は変わらず、ユーザーは文字キーを離してから叩けば回避でき、
  被害は回復可能・回避可能である。**ただし決定1はモードキーの漏出だけを止め、割れたかな自体
  （失敗4件では文字が通常面の`て`で確定し、親指面のかなを失っている）は修復しない**（それは
  選択肢Cの担当で、決定2の優先度判断に効く）。実機検証（上の節）でユーザーは失敗した打鍵を全てチョードの
  つもりと申告しており、重なりも87/88/46msと長かったので、誤診断のリスクは小さい。
  round1 M4の「`PendingChar`起点の正当な単独タップは0件」は循環論法で採らない。
  Idle起点の通常の単独タップ（07:37:24型、08:46:34.715型）はフラグが立たず影響を受けない。

**決定2（実測後に判断）: 選択肢A/C/Gの採否は、押下間隔と重なりの分布を実測してから決める。**
必要なデータ: 文字→親指の押下間隔・重なりの分布（チョード意図/単独打鍵×2意図の別）。
ユーザーが「今のはチョードのつもりだった/違う」を時刻付きでメモする運用と、journalダンプ
（ADR-159/163の記録・再生基盤、`tests/journals/`）が要る。

**決定3（併用可）: 選択肢D（`muhenkan_solo_tap_always_suppress = true`）とA（`config.toml`）は、
決定1が入るまでユーザーが自己判断で使える暫定措置**として位置付け、ADRでは規約化しない。

**決定4（撤回）**: 前版は選択肢E（生VKと保留かな出力の順序制約）を決定4としていたが、動機の
観測（半角`te`化）が誤読だったため撤回した（選択肢E参照）。

**別ADR/BUGとして分離（round3 D-4）**: `ObservedEisu`検出時に、awaseがopen軸へ`false`を
書く（`key_pipeline.rs:1126-1168`の`EngineSync::DirectInput`分岐。同じ数行で
`effective_open: true, confident: true`と断定しながら`handle_engine_set_open(…, false, …)`と
`apply(open=false)`を実行する）。07:35:44.993のログで`UserImeSetIntent`（journal行は`event_kind`のみで値を持たず、`false`は同一
ミリ秒の`open=false`群からの推定）が記録され、`apply`は`Unwarranted`で止まるがbelief書き込みは
残る。約2分後の07:37:56.711にも`was_open_before=true last_intent_before=Some(false)`が出て
いるが、間の07:37:49.508にも同種の`Engine deactivated (NotRomajiInput)`＝再検出があり、
07:35:44.993の書き込みの持続とは断定できない（同種の書き込みが繰り返された結果の可能性が高い）。決定1が抑止しない
Idle起点の単独タップでも同様に起きる、IME beliefの再発ファミリー（`state/ime_model.rs`・
`runtime/ime_coordinator.rs`）の問題で、ADR-178がforce-ONを撤去した直後で消費側の
前提も変わっているため、単独で評価する。**本ADRでは修正案を出さない。**
（07:34:38の`belief_on=false explicit_intent=Some(false)`は、conv=0x19の07:33:16
Ctrl+無変換による正規のIME OFFで、この件の根拠にならない。）

## ADR-179との関係（round1 M3）

ADR-179は`ModeKeyActuationOwner`を`kp_stage_shadow_ime_toggle`（windows層）の1箇所で
計算する設計で、モードキー関連の対症療法の撤去が主目的（収束済み・実装未着手）。決定1は
コアの`NicolaFsm`側に抑止点を1つ足す提案で、方向は逆に見える。それでもコア側に置く理由:
重なり（文字キー保留後の再投入）はコアFSMの内部状態（`PendingChar`→`PendingThumb`の遷移）
にしか存在せず、windows層は`ClassifiedEvent`だけを見て「文字保留後の再投入か」を知り得ない
（ADR-019、コアは事前分類済みイベントのみ受け取る）。`ModeKeyActuationOwner`に条件を足す
案は、この情報をwindows層へ渡す新しい経路（`ClassifiedEvent`の拡張）が要り、抑止点を
1つ減らすのではなく経路を1つ増やす。**この判断はround2で妥当と確認された**（windows層は`ClassifiedEvent`しか受け取らず、
再投入の情報はコアFSMの内部状態にしか無い。ADR-019）。

ADR-179の実装者向けの注意: 無変換は`delegate_to_open_axis=None`のため`FsmDelegate`に
入らず`PhysicalDelivery`になるはずだが、決定1により「`PhysicalDelivery`だがFSM側の文脈
（文字保留後の再投入）で生VKの送出を拒否する」という**コア側の拒否権**を持つことになる。
`ModeKeyActuationOwner`をwindows層が算出する際、この拒否権を所有者判定に混ぜない
（所有者は`PhysicalDelivery`のまま、実際に配送されるかはコアの`resolve_pending_thumb_as_single`
の結果で決まる）こと。

また現ブランチ（ADR-178領域Aの撤去作業中）に新しい対症療法を足すことになる点は
認識している。決定1は`resolve_pending_thumb_as_single`に条件を1つ足すもので、
撤去済みの`reassert`/force-onとは別の領域（FSM内の単独タップ解決）である。

## 検証計画

1. **再現テスト**（実装前に書く、`src/engine/tests.rs`）。round1 B4: `tests.rs`の既定
   ヘルパーはn-gramモデルを設定しないため`adjusted_threshold`は`threshold_us`
   (100ms)そのままで、gap 92msは`is_simultaneous`=trueになり不具合を再現しない。
   シナリオは **gap 108ms側**（`Char↓(t=0)→親指↓(t=108ms)→Char↑(t=109ms)→親指↑(t=116ms)`）
   とする。**期待値の書き方（round2 S9）**: 親指↓後に届くChar↑は`on_key_up`の`release_only`
   （nicola_fsm.rs:2709）を通り、`output_history`から`Char`を引いて`Suppress`を返す
   （同2985-2989、実機ログ07:35:43.760 `send_keys actions=[Suppress]`と一致）ので、
   現状の固定値は`[Char(て相当), Suppress, Key(親指VK), KeyUp]`のような列になる。
   決定1の実装後は末尾の`Key(親指VK), KeyUp`が消えることを確認する。n-gram設定版
   （gap 92ms）も別途用意する。
2. **対照（決定1が抑止してはならないもの）**: (a)Idleから親指↓→148ms保持→親指↑
   （07:37:24型、`handle_key_up_pending`経路）で生の親指VKが出続けること。(b)同じくタイム
   アウト（>100ms保持）経路で出続けること（OUTPUT_GATEによるタイマー延期で同一操作が
   どちらにも流れうるので両方固定、round2 Q5）。(c)専用Fnキー（優先順位1）と
   `explicit_action`（優先順位2）がフラグの有無に関わらず動くこと。(d)Suppress設定
   （既定）では従来どおり何も出ない。
3. **フラグ伝播の表**: `resolve_pending_thumb_as_single`の7入口（614/1596/1797/1816/2913/
   2945/3072）それぞれで、フラグが立った`PendingThumb`を解決したときにIME操作が抑止される
   こと、フラグが立っていない`PendingThumb`では抑止されないことを固定する。入口の列挙で
   なく「フラグが立った`PendingThumb`はどの入口から解決されても覆われる」で説明する
   （round2 S10、前版の入口列挙は1797が鏡像ケースであって誤解を招く記述だった）。
   `resolve_char_and_thumb_as_separate_solos`は`min_overlap_margin_percent>0`でのみ到達し、
   そのときフラグは立たない。
4. **フラグ寿命**（round2 M9、round3 S11）: 時間超過分岐で立てたマーカーが次の`decide`で
   必ず消費・クリアされること。連続2回の単独タップで、1回目の直後のIdle起点の親指タップに
   フラグが残っていないことを固定する（立てっぱなしで以後の単独タップを殺すバグの防止）。
   `timed-fsm`の`parse()`（parser.rs:344-400）は`ReduceAndContinue`を必ず次の`decide`へ
   再入するが、`MAX_REDUCE_CONTINUE_STEPS`超過時は`decide`を呼ばず早期returnするため、
   `on_key_down`（nicola_fsm.rs:1431）で`self.parse(ev)`の戻り値を返す直前に
   `next_thumb_after_char_flush = false`を無条件に置き、上限で打ち切られた直後のイベントで
   フラグが立っていないことも固定する。
5. **鏡像（決定1b）**: `Thumb↓(t=0)→Char↓(t=121ms、親指面のかなが存在する文字)`で、現状は
   `[Key(親指VK), Char(親指面のかな)]`が出ることを固定してから、決定1b実装後に`Key(親指VK)`が
   消え`Char(親指面のかな)`は変わらないことを確認する。`candidate.is_none()`の文字では従来どおり
   `Key(親指VK)`が出ること、閾値内（`is_simultaneous`かつcandidateあり）は従来どおり
   親指を消費して`Key(親指VK)`が出ないことも固定する。
   **タイマー経路（決定1c）**: `Thumb↓(無変換)→100ms超のタイマー発火→ Char↓`で`Key(親指VK)`が
   出ず親指面のかなだけが出る（決定1b・1cの実装後）。タイマー発火後に文字が来ないまま`Thumb↑`
   なら`[Key(親指VK), KeyUp(親指VK)]`が親指を離した時点で出る。Space/Enter親指と
   `engine_off_solo_repeat_vk`の親指は従来どおりタイムアウトで単独確定される（対照）。
   親指のオートリピートKeyDown、フォーカス移動による`flush_pending`も固定する。
6. `cargo test --lib`（ホスト）、`cargo check --target x86_64-pc-windows-msvc
   -p awase -p awase-windows`、`cargo clippy`（引数8個上限の確認）。
7. **fix-requires-evidence規約**: `resolve_pending_thumb_as_single`は再発ファミリー表の
   「キー選択」行の対象（BUG-119以来）。回帰テスト(a)を必須とし、実機依存部分は
   `docs/known-bugs/BUG-145.md`（現在の最大はBUG-144。採番は実装時に並行ブランチとの
   衝突を確認）を新規作成する。
8. **実機A/B**: 同一の運指を繰り返し、`PendingChar`起点の`PendingThumb`→KeyUp解決由来の
   生`Key(29)`注入（と、それに続く`Kana/roma → Eisu/roma`）が消えること、Idle起点の単独タップ
   （07:37:24型）の生`Key(29)`と半角英数化は残ることを`awase.log`で確認する
   （判定基準を分ける。生注入の有無だけでは通常の単独タップが混ざる）。

## 明示的にスコープ外

- 親指→文字方向の入口ゲート`is_simultaneous`そのものの閾値調整（決定2）。生の親指VKの抑止は
  決定1bで扱う。
- `min_overlap_margin_percent`の変更（ADR-112で恒久化済み）。
- Eisu検出後のエンジン非活性そのもの（実際に半角英数になったので正しい動作）。ただし同一tickの
  open軸への`desired_open=false`書き込みは別問題で、別ADR/BUGで扱う（「別ADR/BUGとして分離」段落）。
- Eisu化から`belief`更新までの1.2秒の検出遅延（43.798のプローブが`NATIVE=false`を
  読んでいるが`ObservedEisu`は44.993、別軸の改善余地）。
- awase自身の送信がhookを100ms級で止める件（07:35:43のDが`delay=92ms`）。
- ADR-181（GJI ATOKキーマップのVK_DBE_HIRAGANA外部エコー）。別現象。

## 未解決の疑問（round8で検証してほしい点）

round7までに収束した点（決定1、決定1b（機序、`candidate.is_some()`ゲート、(i)一本化）、選択肢F却下、
ADR-179との関係、`physical="Allow"`の扱い、`DirectInput`分岐の分離、実機検証節の数値）には、
未解決の欠陥は残っていない。round7のM19〜M21・S26は反映済み。

1. **決定1c（ユーザー判断で(b)を採用）の設計**: `on_timeout`の`PendingThumb`腕で、無変換/変換
   のときだけ単独確定せず`PendingThumb`を維持する方式に穴が無いか。特に
   (a) `PendingThumb`が長時間残ることで他のキー（passthrough、Ctrl等の修飾、親指どうし、
   オートリピートKeyDown）の扱いが壊れないか（`step_pending_thumb_thumb`が単独確定して生キーを出す
   等）、(b) `flush_pending`（フォーカス移動）、(c) `solo_counter`/`engine_off_solo_repeat`、
   (d) 決定1bのフラグとの相互作用、(e) 単独長押しの生キー送出がKeyUpまで遅れることの影響
   （`thumb_watch_window`、ADR-092/135/147/153の不変条件、`ThumbRawVkEmission`）。
2. 書き換え残りが他に無いか。

## レビュー経緯（記録）

- **round1（2026-09-19、opus）**: ログの数値主張は全て再現でき事実誤認なし。Blocker4
  （B1:belief追随の差分、B2:実入口`handle_key_up_pending`の未記載、B3:重なり閾値は実装不能、
  B4:検証シナリオが既定設定で再現しない）、Must-fix7（M1:優先順位1・2まで殺す、
  M2:引数上限、M3:ADR-179との衝突、M4/M5:抽出母数、M6/F1/F2:選択肢Aの事実誤り、
  M7:見落とした代案E/F/G）を検出。B1はmuhenkanに`delegate`が無い設計との検証結果を
  上記に反映（belief追随の非対称は別軸として残す）。M4のうち「07:37:24.993は失敗4件と同型」は
  Idle起点であり誤りと判断した（上記「通常の単独タップの例」）。他の指摘は本v2に反映。
- **round2（2026-09-19、opus、同一レビュアー）**: round1のB1（同一VKの単独タップ22件では
  delegatedが出る）は、22件を全て0x1Dと誤認した誤りだったとして**撤回**（tail.logで
  delegatedが0x1CのKeyUpと同一µs帯、抜粋6ファイルで`[shadow-toggle]`が出るのは0x1C/0xF2のみ
  で0x1Dはゼロ）。M5（07:37:24が失敗と同型）は起点がIdleだったので半分撤回。ただしM4は
  循環論法で証拠価値ゼロとして失効（v3で本文から削除）。新規Blocker B5（生無変換→Eisu化が
  1件でしか確認できず、Idle起点の07:37:24では起きていないように見える）は、当初は解析後の
  現存ログ(08:11)から「無条件のKana⇄Eisuトグル」と決着させたが、ユーザーの指摘（IME ON・
  ひらがなからの無変換単独タップは、IME ONのまま半角英数へ遷移する仕様）を受けて撤回した
  （idle-conv-checkの遅延観測を、直前のタップの効果と取り違えていた。上記「観測」参照）。Must-fix M8（鏡像を別ADRに逃がすのは
  誤り）、M9（時間超過分岐は`PendingThumbData`を生成しないので、フラグの立て方と寿命の具体化が
  必要、2913の記述も誤り）、S7（Fはopen軸で表現不能として却下）、S8〜S10・N2〜N3をv3に反映。
  Q3（優先順位3・4限定は不変条件を壊さない）、Q4（ADR-179で表現不能）、Q5（OUTPUT_GATEによる
  タイマー延期）は収束と判断された。
- **round3（2026-09-19、opus、同一レビュアー）**: 未収束（Blocker1・Must-fix2・Should-fix3）。
  Q1（フラグ寿命、`timed-fsm`の`parse()`ループを読んで健全と確認）とQ4（ADR-179）は収束。
  round2のM8/M9/S7〜S10/N2/N3は適切に反映済みと確認された。指摘のうち、M11
  （`b5.log`は「無条件Kana⇄Eisuトグル」を支持しない、反例2件・パリティ破綻1件・陽性2件も別要因で
  説明可能）は正しく、ユーザー指摘と合わせて撤回済みの内容を裏付けた。S11（`parse`が
  `MAX_REDUCE_CONTINUE_STEPS`で打ち切られるとマーカーが残る）、S12（`resolve_pending_thumb_
  as_single`のシグネチャ束ね直し、呼び出し元がフィールド個別渡しのため）、N4は採用。
  **B6（物理の無変換が`physical="Allow"`で毎回OSへ届く二重配送）とM12（選択肢H:
  Passthroughをno-opにする）は、`physical="Allow"`の読み違いと判断して採らなかった**
  （「観測上の注意」参照）が、round4で再検証を依頼する。M11の実験（`muhenkan_solo_tap_
  always_suppress=true`で単独タップ5回のconv読み取り）は、ユーザーによる実機実験が
  必要なので未実施。
- **round3（訂正版、2026-09-19、opus）**: Blocker無し、Must-fix3。round2のB5は解消（機序が
  一様と確定、決定1は相関を撃っているだけという疑義は取り下げ）。ユーザー申告の裏付けが
  リポジトリ内にあることを確認（`conv_classify.rs:44-47`、`key_pipeline.rs:1142-1146`、
  b5.logのconv observation 35/35が`open=true`）。選択肢F却下に「ATOKプリセットでは」の限定が要る
  （M13、反映済み）。依頼した「半角英数をIME OFF相当に扱っている疑い」は事実として確認
  （`DirectInput`分岐のopen軸`false`書き込み）、別ADR/BUGとし本ADRには手順4に1行のみ。
- **round4（2026-09-19、opus）**: B6/M12は**レビュアー自身の誤読として全面撤回**
  （`Decision::Consume`は`physical`を見ず`Consumed`を返す。`[reinject]`は`PassThrough`行にのみ対応）。
  一方、round1でレビュアーが報告した「`て`が半角`te`になる順序逆転」は`[h1-send]`（事後プローブ）を
  送信点と取り違えた誤読で、`[h1-run]`では`て`は生VKの25ms前に送信済みだった。ADRに取り込まれて
  いたので削除（B7）、選択肢E・決定4を撤回。決定1bの保留理由を「害が未確認」から
  「害はあるが意図が未確認」に書き換え（M16）、S18/S19を反映。決定1の設計そのものには
  4ラウンドを通じて未解決の欠陥は残っていないとの判定。
- **round5（2026-09-19、opus）**: 未収束（Blocker2・Must-fix2）。B8（手順1に「実送信は43.798」が
  残り手順3と矛盾）を修正。**B9**: 決定1bの非対称理由(2)は`seq=10414`（`state_before=
  "PendingThumb"`→`state_after="Idle"`）と`step_pending_thumb_char`が`consume_thumb`を呼ばない
  構造に反証され、07:38:01では同じ親指押下がsolo tap（`Key(29)`）とshift（`Char('ぐ')`）の
  両方に使われている（BUG-46型の二重使用）。決定1bを保留から**採用**へ変更（`candidate.is_some()`
  のとき）。M18（Ctrl+I交絡は`cancel skipped`で過大評価）、M17（DirectInput段落の「持続」は
  07:37:49.508の再検出を挟むので弱める）、S20〜S23・N6を反映。
- **round6（2026-09-19、opus）**: 未収束（Blocker1・Should-fix3）。B9の機序の本文化と
  `candidate.is_some()`ゲートは妥当と確認（`step_pending_thumb_char`と`classify_idle_intent`/
  `reduce_active_thumb`が同じ`side`・`lookup_face`を引く）。**B10**: `timeout_pending_thumb`経由
  でも同一の二重使用が起き（`consume_thumb`は5箇所だけ、`active_thumb_side`は押下中かつ未消費で
  `Some`）、決定1bのフラグでは原理的に覆えず、OUTPUT_GATEの非決定性で同じ運指が両経路に
  振り分けられる。本ADRは(c)「穴として受容し記録」を採り、実験と(b)を追補に回す。
  S24（割れたかなは修復されない）、S25（「後4件」→「後2件」、タイマー経路の痕跡）、
  N7（エスケープハッチの前提）を反映。
- **round7（2026-09-19、opus）**: Blocker無し、Must-fix3・Should-fix1。実機検証の数値を独立に
  再計算してほぼ完全一致（24件・成功21・間隔2.853〜70.308ms・失敗3件の80.721/87.296/92.576ms・
  重なり87.417/88.264/46.113ms、`[Key(..), Char(..)]`型は全ログで1件、58.79ms成功/58.90ms失敗）。
  M19（決定1bの正当化を(i)自己矛盾に一本化、(c)の理由を(b)のコストに一本化、サンドイッチ実験は
  (c)→(b)昇格判断の入力）、M20（BACKSPACE連打→かなキー→conv=0x19復帰の状況証拠と切り分け限界）、
  M21（親指先押し69→72件）、S26（conv=none区間を08:46:20〜41に）を反映。
- **ユーザー判断（2026-09-19）**: タイマー経路は(b)（生の無変換を親指を離すまで遅らせる）を
  「今すぐ直す」と決定。実装は決定1・1b・1cを別コミットで、現ブランチから新しいworktreeで進める。
  BUG-145（本件）を実装コミットで起票、BUG-146（`DirectInput`分岐のopen軸`false`書き込み）は
  起票のみで修正は後回し。
- **round8（2026-09-19、opus）**: 決定1cの方向性（(b)を無変換/変換に限定、新状態を持たず既存経路と
  決定1bのフラグを再利用）は妥当。ADR自身が挙げた要確認2点がどちらも予想と逆だった: (1)
  `observe_thumb_watch_window`は統計専用でオートリピートを抑止せず、`step_pending_thumb_thumb`が
  同一vkのリピートで生キーを連射する（B11）→同一親指のリピートを無視するガードを追加、
  (2) フォーカス移動の`flush_pending`は`Denied`で生キーを出さず、単独タップが黙って消える（M22）→
  「取りこぼし窓」として明記しテストで固定、`engine_off_solo_repeat`既定`VK_INSERT`（M23）、
  S27〜S30・N9を反映。実装時、`delegate_to_open_axis`を持つ変換は既存の17テストと契約に反するため
  対象外にした（ADR-092/147/153のタイムアウト時発火を維持）。
