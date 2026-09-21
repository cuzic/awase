# ADR-184 opus-adversarial-consult round1

対象: `docs/adr/184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md`
（ドラフト、起票直後）
レビュー日: 2026-09-19 / ブランチ `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（HEAD `c8bc1adc`）
方式: 読み取りのみ。ADR本文の引用・主張はすべて実コードで裏取りした。

---

## 総評

**現状のドラフトは実装着手に進めない。** 診断の核（3点）のうち少なくとも
2点が実コード・同日起票の兄弟ADRと食い違っており、提案設計は「新しい
actuationを追加しない」という中心的な主張が自己矛盾を含む。加えて、
本ADRの前提条件として採用している「GJIキーマップ＝ATOKプリセット」が、
awase自身が実機`config1.db`から読み取った過去2回の記録（CUSTOM / MSIME）
と矛盾しており、そのままでは提案するゲートが実機で一度も成立しない
可能性がある。

ただし**ユーザーが観測した挙動そのもの（無変換単独タップ＝conv軸単独
トグル）と、「awase主導でconv軸を持つべきではないか」という設計方向自体は
否定しない**。ADR-182が同じ実機ログから既に到達している結論（生の無変換が
GJIへ届くこと自体が起点）と統合した上で、下記Blockerを解消してから
再レビューすべきである。

重大度の内訳: Blocker 5件 / Must-fix 10件 / Nits 4件。

---

## Blocker

### B1. 「約600ms後に物理IME ONキーが押された」という因果の鍵が、同日起票のADR-181が記録した現象と区別されていない

**該当**: ADR-184「現状の機序」手順5〜7
（`物理「IME ON」キー（VK_DBE_HIRAGANA、scan=0x70）の押下により`）

**問題**: `docs/adr/181-gji-atok-keymap-hiragana-key-external-echo-reverts-ime-off.md`
は**まったく同じ環境（dragonflyg4 / GJI ATOKキーマップ / TsfNative /
2026-09-19）**で、次を実機ログから記録している:

> GJI が `VK_DBE_HIRAGANA`(0xF2) を `injected=false`（`LLKHF_INJECTED`なし・
> awase自身の self-injected マーカーも無し）で不規則（**0.4秒〜28秒間隔**）に
> 送ってくる。……IME OFF直後に、無関係な外部由来のF2イベントがIMEを
> 再びONへ戻してしまう。

ADR-184の「約600ms後」は、この0.4〜28秒の窓の**ど真ん中**にある。
ADR-181が記録した再現契機は「drift correction由来のOFF」「Ctrl+無変換の
明示OFF」の両方であり、**「何らかの経路でIMEがOFFになった直後」という条件は
ADR-184の手順4（`Engine deactivated (ImeOff)`）でも成立している**。

つまり手順5〜7は、ADR-181が既に「ユーザー操作なし」と結論づけた現象の
別の観測である可能性が高い。ADR-184はこれを「物理『IME ON』キーの押下」と
断定しているが、ログの`injected`値・直前直後の他の物理キー活動の有無・
同一セッション内での同種イベントの周期性のいずれも提示していない。

**失敗シナリオ**: もし手順5がGJIの自己主張であれば、ADR-184が「1秒の
往復」と呼ぶ現象は「awaseがOFFを書く ← 本ADRの対象」と「GJIが勝手に
ONへ戻す ← ADR-181の対象」という**2つの独立した不具合の重畳**であり、
無変換の意味論をawase側へ引き取っても手順5〜7側は残る。ADR-184の設計を
実装した後に「まだ直っていない」となり、原因の切り分けが一段難しくなる。

**要求**: 手順5のF2イベントについて、(a)`injected`/`self_injected`の実値、
(b)ユーザーがそのタイミングで実際にキーを押したかの申告、(c)同一ログ内で
同種の孤立F2が他にも周期的に出ていないか、の3点をログから示すこと。
ADR-181とADR-184は`related_adr`で相互参照すること。

---

### B2. 前提条件「GJIキーマップ＝ATOKプリセット」が、awaseが実機から読んだ値と矛盾しており、提案ゲートが永久に不成立になりうる

**該当**: ADR-184「提案する設計」1
（`GJI キーマップが ATOK プリセット（classify_mode_key_ime_action の
ImeToggleKind::Toggle 判定）であることを前提条件に含める`）、および
「未解決の疑問」5

**実コードの確認**: `crates/awase-windows/src/gji_charset_autodetect.rs:296-309`

```rust
match raw.session_keymap {
    Some(v) if v == awase_gji_config::SESSION_KEYMAP_ATOK => match key {
        ModeKeyCandidate::Henkan | ModeKeyCandidate::Muhenkan => Some(ImeToggleKind::Toggle),
        ...
    },
    Some(v) if v == SESSION_KEYMAP_MSIME || v == SESSION_KEYMAP_MOBILE => match key {
        ModeKeyCandidate::Hiragana | ModeKeyCandidate::Katakana => Some(ImeToggleKind::On),
        ModeKeyCandidate::Henkan | ModeKeyCandidate::Muhenkan => None,   // ← Muhenkanは None
    },
    ...
}
```

`Muhenkan → Some(Toggle)` が返るのは、(i) `session_keymap == ATOK(1)`
かつ `custom_keymap_table` に該当行が無い場合、または (ii) CUSTOM literal
トークンが `Toggle` 分類された場合、の2つだけである。

**矛盾する既存記録（いずれも同じプロジェクトオーナー実機）**:

| 出典 | 日付 | 読んだ `session_keymap` |
|---|---|---|
| `docs/known-bugs/BUG-115.md`（実機`config1.db`を汎用protobufスキャナで検証） | 2026-09-05 | **field 41 = 0 (CUSTOM)** |
| `gji_charset_autodetect.rs:275-281` のADR-174実機検証コメント | 2026-09-15 | **MSIME=2** |
| ADR-182 / ADR-181 / ADR-184（ユーザー申告） | 2026-09-19 | ATOKプリセット |

さらに **ADR-182「失敗4件と通常の単独タップの違い」節が、同じ実機ログで
`muhenkan_delegate_to_open_axis` が未設定（`resolve_delegate_to_open_axis`
が `Fallthrough(None)`）であることを確認している**。ATOK(1) が読めていれば
`Toggle` が返り、`gji_thumb_key_ime_toggle` が既定`false`でゲートされて
`None` になる——この観測はATOK説と「MSIMEでそもそも`None`」説の**どちらとも
整合してしまう**ので区別材料にはならないが、ADR-174の MSIME=2 と合わせると
「awase が読んでいる値はATOKではない」という側に傾く。

**失敗シナリオ**: 「GJIのUI上はATOKプリセットを選んでいる」というユーザーの
申告が正しくても、`config1.db` の `session_keymap` フィールドがそれを
反映していない（あるいは `custom_keymap_table` に行が残っていて
`gji_charset_autodetect.rs:290-295` のフォールスルーが先に効く）場合、
ADR-184の前提条件は実機で一度も成立しない。実装しても**何も起きず**、
「実装したが効かない」の原因調査に時間を溶かす。BUG-115が
`SESSION_KEYMAP_FIELD = 22→41` の誤りで「もっともらしい誤った値」を
返し続けた前例がある領域である。

**要求**: 実装着手の**前に**、現在の実機`config1.db`の
`session_keymap` / `overlay_keymaps` / `custom_keymap_table` の実値を
（clipwire経由で）取得し、`classify_mode_key_ime_action(Muhenkan, raw)` が
実際に何を返すかをログで確認すること。ATOKでなければ、ゲートの設計
（「未解決の疑問」5）は「任意にするか」ではなく**設計の成否そのもの**の
問題になる。

---

### B3. 「doc コメントと実装が食い違う」という主張は誤読であり、未解決の疑問1はその誤読の上に立っている

**該当**: ADR-184「現状の機序」手順3、「未解決の疑問」1

**ADR-184の主張**:
> `conv_classify.rs::EngineSync::DirectInput` のコード内 doc コメント
> （「`effective_open=true` の belief を直接注入して apply する」）は
> この実装と食い違っている

**実コード** `crates/awase-windows/src/state/conv_classify.rs:44-47`（全文）:

```rust
/// `ObservedEisu` 観測 → engine OFF + DirectInput。conv の英数モードは IME-ON の
/// 確証（conv=0x10 は ROMAN ビット付き半角英数）のため、`effective_open=true` の
/// belief を直接注入して apply する。
DirectInput,
```

doc コメントは**第一文で明示的に「engine OFF」と書いている**。ADR-184は
この第一文を落として第二文だけを引用し、「`effective_open=true` を注入する
＝IMEを開ける意図だったはず」と読んでいるが、第二文の `effective_open=true`
は **apply に渡す「現在状態の仮定」**（`already_matched` による送信省略を
バイパスするための前提）であって、送信ターゲットではない。

`crates/awase-windows/src/runtime/key_pipeline.rs:1143-1153` が明示している:

```rust
// conv の英数モード観測は IME-ON の確証。direct belief で already_matched を
// バイパスして apply する。
let belief = crate::output::OpenBelief { effective_open: true, confident: true };
let order = self.issue_actuation_order(false, "idle_conv_check_direct_input");
let (outcome, mut record) = self.platform.apply_ime_open_with_belief(order, None, belief);
```

`issue_actuation_order(false, ..)` がターゲット、`belief.effective_open=true`
が前提。両者が異なるからこそ `already_matched` で握り潰されずOFFが送られる
——**コメントと実装は一致している**。

**帰結**: 「未解決の疑問」1（不一致を別バグとして切り出すか、本ADRで
間接的に無害化するか）は、そのままでは成立しない問いである。
**ただし「`ObservedEisu`（＝IME ONのままの半角英数）を検出してopen軸に
`false`を書くのは妥当か」という問い自体は依然として有効な論点**であり、
それは既に **ADR-182 の「別ADR/BUGとして分離（round3 D-4）」節が独立項目
として切り出し済み**である（下記M1）。本ADRは「コメントと実装の不一致」
という誤った枠組みを撤回し、ADR-182が切り出した論点への参照に置き換える
こと。

---

### B4. 決定4「新しいactuationを追加しない、beliefの変化から自然に導く」は自己矛盾している

**該当**: ADR-184「提案する設計」4

**問題**: 決定4は「Engine の活性/非活性は既存の activation ゲート
（`ime_on && input_mode.is_romaji_capable()`）が `input_mode` belief
（`ObservedEisu`/`AssumedRomaji`）の変化から自然に導く」としているが、
**誰が `input_mode` belief を変えるのかが書かれていない。**

awase が能動的に conv を書き換えた場合、`input_mode` を動かす経路は
2つしかない:

1. **能動的訂正** `apply_input_mode_correction(..., InputModeApplyStrategy::..)`
   ——左Shift版が実際に使っている経路。`key_pipeline.rs:2303-2307`（IMC経路）
   および `2324-2328`（GJI SendInput 経路）で、`ObservedEisu` +
   `UserHalfWidthAlnumToggle` を必ず書く。exit側も `2702` で対称に書く。
2. **受動観測** idle-conv-check ——これが `classify_conv_transition` →
   `EngineSync::DirectInput` を発火させる、**本ADRがまさに避けたいと
   言っている経路そのもの**。

決定4は1を「新しいactuationを追加しない」として排除しているので、
残るのは2だけになり、**設計が回避対象の経路に依存する**。

さらに左Shift版は `note_explicit_ime_action(now_tick)`
（`platform_state.rs:449`）も必ず対で呼んでいる。これは
「idle-conv-check が復元途中の conv を読んで ObservedEisu → DirectInput に
落とさないよう、明示的 IME 操作として抑止する」ための抑止であり
（`key_pipeline.rs:2431-2433` のコメント）、これを落とすと awase 自身が
書いた conv を idle-conv-check が読み直して DirectInput を叩く——
**本ADRが報告した症状を、awase自身のトグルで再生産する。**

**要求**: 決定4を撤回し、左Shift版と同じ「conv書き込み + `note_explicit_ime_action`
+ `apply_input_mode_correction`」の3点セットを設計に明記すること。
その上で `InputModeApplyStrategy` の新variant（M9）を定義すること。

---

### B5. 「生キーのSuppress」の実現手段として挙げた `transport.rs::plan()` は、この経路に構造的に効かない

**該当**: ADR-184「提案する設計」5、「未解決の疑問」4

**ADR-184の主張**:
> 生キーの物理配送は Suppress する必要がある（`transport.rs::plan()` に
> 無変換/変換専用分岐が既にあるので、そこへ「ATOK Toggle として awase が
> 処理済みなら Suppress」という条件を足す形になる見込み）

**実コード・既存ADRで確認した事実**:

1. 単独タップとして解決される無変換の**物理**KeyDownは、FSMが
   `PendingThumb` として保持しており `Decision::Consume` になる。
   `executor.rs::execute_relay` の `Consume` 腕は `physical`
   （＝`PhysicalKeyDisposition::plan()` の結果）を**一切参照しない**
   ——ADR-182「観測上の注意（round3 B6・N4への対応）」および同
   「決定1の有効性は構成上保証される（round4 A）」で確認済み。
2. したがって **GJI が受け取る無変換は、awase が
   `resolve_pending_thumb_as_single` 優先順位4（`ModeKeyConfig` の
   `SoloTapAction::Passthrough`、`nicola_fsm.rs:2350-2357`）で合成する
   `[Key(VkCode(29)), KeyUp(VkCode(29))]` **のみ**である
   （ADR-182 07:35:43.772 の実ログで確認済み）。
3. `transport.rs:363-372` の無変換/変換分岐が効くのは
   `Decision::PassThrough`/`PassThroughWith` の場合だけ。既存の
   `explicit_ime_action_consumed` マーカー（ADR-153決定1 ケース3改）が
   ここを使うのは、**ケース3改が `Decision::Consume` に乗らない**からで
   あって、単独タップ解決経路一般に効くからではない
   （`transport.rs:341-362` のコメントが明記）。

**失敗シナリオ**: `transport.rs` に条件を足しても、Consume されている
物理キーには何の影響も無い一方、合成 `Key(29)` は従来どおり GJI へ届く。
「Suppressしたつもりが二重発火が残る」＝ADR-184自身が懸念した
BUG-52/BUG-46型の二重actuationを、対策を入れた上で踏む。

**正しいレバー**: `resolve_pending_thumb_as_single` 内で
`no_op_resolution()` を返す（＝合成しない）こと。これは既存の
`explicit_action_consumed`（`nicola_fsm.rs:2314`）/
`auto_delegate_open_axis_consumed`（同`2333`）の2つの早期returnと同じ
方向の追加であり、**ADR-182 決定1 が新ガードを挿入しようとしている
まさにその位置**（2333の直後、2338の直前）と競合する。設計を統合すること。

---

## Must-fix

### M1. ADR-181 / ADR-182 との重複・矛盾が未整理（`related_adr` にも本文にも無い）

ADR-184の`related_adr`は `ADR-183/179/135/084/094`。しかし同日・同環境・
同一実機ログ由来で、**未コミットの作業ツリーに ADR-181 と ADR-182 が
並存**している（`git status` で確認）。

- **ADR-182** は「生の無変換がGJIへ届く → 半角英数化 → `ObservedEisu` →
  `Engine deactivated`」という連鎖を、より詳細なログ（64件の統計、
  失敗4件の時刻付き内訳、実機A/B 2セッション）で既に記述している。
  ADR-184「現状の機序」はこの部分集合であり、新規性は「600ms後の巻き戻し」
  だけである（それもB1で疑義）。
- **ADR-182「別ADR/BUGとして分離（round3 D-4）」** が、`EngineSync::DirectInput`
  の open 軸書き込みを独立項目として切り出し済みで、しかも
  **`apply(open=false)` は `outcome=Unwarranted` で止まっている**と
  記録している（M8参照）。ADR-184 手順3の「実IMEを強制的に閉じる
  SendInput(VK_IME_OFF) を送信している」と食い違う。
- **ADR-181** は B1 のとおり手順5の代替説明そのもの。

**要求**: ADR-184 は ADR-181/182 を `related_adr` に加え、本文で
「どこまでがADR-182の記述で、本ADRが新たに主張するのは何か」を明示する。
同じ症状に対して**互いに参照していない2つの設計（ADR-182決定1 と
ADR-184）が並行して起票されている**状態は、本リポジトリが
`experiment-logging.md`/`fix-requires-evidence.md` で繰り返し警告している
「同じ論点が別セッションで再発見される」パターンそのものである。

### M2. `resolve_pending_thumb_as_single` の優先順位の中でどこに入るかが未定義

`nicola_fsm.rs:2257-2262` が定める既存の優先順位:

| 順位 | 内容 | 行 |
|---|---|---|
| 1 | `muhenkan_solo_tap_dedicated_fn_key`（ADR-091 §D3.2） | 2273 |
| 2 | `*_solo_tap_ime_action`（ADR-153決定1、ユーザー明示config） | 2286 |
| — | `explicit_action_consumed` → no-op（BUG-123） | 2314 |
| — | `auto_delegate_open_axis_consumed` → no-op（ADR-154） | 2333 |
| 3 | `resolve_delegate_to_open_axis` | 2338 |
| 4 | `mode_key_config` Suppress/Passthrough | 2343 |

ADR-184 の新トグルはこの表のどこに入るのか、既存の各項目が設定されて
いる場合に勝つのか負けるのかが一切書かれていない。特に:

- **順位1（専用Fnキー）との関係**: BUG-115 F5 は、`muhenkan_solo_tap_dedicated_fn_key`
  が設定済みだと無変換側の delegate が**黙って無効化される**非対称を記録し、
  `warn_thumb_key_toggle_if_needed` で警告する対処まで入れている。
  同じマスキングが新トグルにも起きる。
- **順位2（`muhenkan_solo_tap_ime_action`）との関係**: これは
  「ユーザーが明示的に設定した無変換の意味論」であり、新トグルは
  同じキーの意味論を**自動検出で**決める。両方が設定されたらどちらが勝つか。
- **ADR-182 決定1 の新ガード**（2333直後に挿入予定）との位置関係。
- **BUG-115 の既知の穴 F1**: `resolve_pending_thumb_as_single` は
  「単独タップ確定時」だけでなく**チョード判定に失敗した経路からも
  呼ばれる**（`nicola_fsm.rs` の 614/1596/1797/1816/2913/2945/3072 の
  計7箇所、ADR-182で確認）。ADR-184「提案する設計」1 は
  「既存の NICOLA チョード判定をそのまま使う」と書いているが、
  **ADR-182 の主題はまさにその判定が誤判定することである**。
  非冪等な `Toggle` を誤発火経路に載せる危険は BUG-115 が
  `gji_thumb_key_ime_toggle` を既定OFFにした理由1そのもの。

### M3. `HalfWidthAlnumState` の転用は現状の型では成立しない（レビュー観点2への回答）

`crates/awase-windows/src/state/half_width_alnum.rs` を全文確認した結果、
この型は**左右Shift専用**に固く作られている:

- `plan_half_width_alnum_action(shift_up: ShiftKeyUpKind, ..)` の入力は
  `LeftTap`/`LeftChord`/`RightTap`/`RightChord` の4値のみ（:14-23, :51）。
  無変換タップを表す値が無い。
- `note_physical_key_down(vk)` は `VK_LSHIFT`/`VK_RSHIFT` を直接比較して
  左右の候補を折る（:146-153）。
- `take_shift_up_kind_disarming_both` は BUG-25追補11 の左右非対称
  再発防止のため「必ず両方disarm」を関数名に刻んでいる（:190-215）。
- `entry_policy: HalfWidthAlnumTogglePolicy`（:124、`config.general.half_width_alnum_toggle`）
  は左Shiftトグルの kill switch。

**設計判断が必要な点（ADR未記載）**:

1. **ラッチ共有か分離か**。共有（`toggle_held` を再利用）すると、
   無変換で入った半角英数が左Shift 1回タップや右Shift緊急解除で
   抜けることになる——利便性としては妥当かもしれないが、
   `plan_half_width_alnum_action` の「チョードではexitしない」
   （BUG-25追補11 の実機報告由来）規則が無変換にも適用されるべきかは別問題。
   分離すると、**2つのラッチが同一の conv 状態を独立に所有する**ことに
   なり、`begin_restore_kana`/`commit_enter_*` の commit 規律が二重化して
   ADR-184 自身が懸念する「awase側 `toggle_held` と GJI 側 conv のズレ」を
   awase 内部で再生産する。
2. **`entry_policy` の適用範囲**。`HalfWidthAlnumTogglePolicy::Off`
   （kill switch）が無変換経路も殺すのか、`MsImeOnly` のとき無変換経路が
   どうなるのか。
3. **無変換特有の差**: 左Shiftは「押している間は修飾キー」なので
   `LeftChord` という中間状態が意味を持つが、無変換は NICOLA の親指面
   （シフト面）として**文字を出す**。`resolve_pending_thumb_as_single`
   に到達した時点で「単独タップ」は確定しているので `ShiftKeyUpKind`
   相当の分岐は不要——つまり `HalfWidthAlnumState` の主要な複雑さ
   （4値判定 + arm/disarm）は無変換側では使わない。**共通化する価値が
   あるのは `toggle_held` 1フィールドと commit-on-success 規律だけ**で
   あり、型ごと転用するのは過剰である可能性が高い。

### M4. 「open軸に一切触れない」はコード上、一部成立しない（レビュー観点5への回答）

`output/mod.rs:1230-1273` の `send_gji_half_width_alnum_toggle` を確認:

| 行 | 内容 | ADR-184の主張への影響 |
|---|---|---|
| 1238-1239 | Enter → `VK_DBE_ALPHANUMERIC`、Exit → `VK_DBE_HIRAGANA` | 送るVK自体は静的`shadow_action`を持つ（`TurnOff`/`TurnOn`）。`IME_KANJI_MARKER`付きで自己注入判定されるため`kp_stage_shadow_ime_toggle`には届かない（ADR-181「内部送信元の除外」で確認済み）——**awase自身のbeliefは動かない**、ここはADRの主張どおり。 |
| 1241-1247 | `ime_mode_key_injection_blocked_by_modifier()` → 送信スキップ | Win/Alt押下中は**無送信**。 |
| 1248-1254 | `if !ime_open { スキップ }` | **open軸を読む**。`effective_open()==false` なら何もしない。ユーザー観測「IME OFF → 不変」とは整合するが、「open軸に触れない」ではなく「open軸に依存する」が正確。 |
| 1255-1268 | Enter時、composition/候補表示中でも発火（ADR-107決定5の緩和） | preedit破壊の兆候が出たらガードを戻せ、とコメントが明記。無変換は打鍵頻度が左Shift単独タップより高くなりうるため、露出が増える。 |

**より危険なのは失敗時の commit 規律**。exit側 `kp_send_gji_restore_exit`
（`key_pipeline.rs:2381-2410`）は、`prepend_synthetic_shift_up == false`
のとき **SendInput が見送られても `true` を返し、呼び出し元は
`apply_input_mode_correction(AssumedRomaji)` を進める**（:2401-2409 の
コメントが「呼び出し元の文脈上belief補正は必須のため続行する」と明記）。
無変換タップ起点は物理Shiftを伴わないので `prepend=false` が自然だが、
**そのまま流用すると「実GJIは半角英数のまま、awaseのbeliefだけひらがなに
戻る」＝BUG-25追補3と同型の実害**（engineがpass-throughを抜けて生ローマ字を
送る）になる。無変換経路は左Shift版の「ユーザーが今まさに再試行できる
文脈」に**該当する**ので、`rearm_after_failed_gji_exit` 側の扱いが要る。

### M5. 「GJI向けに実機検証済みの安全な注入経路」という評価は、ATOKプリセット下では未検証

ADR-184「提案する設計」2 は、`send_gji_half_width_alnum_toggle` を
「GJI 向けに実機検証済みの安全な注入経路」と呼んでいる。しかしその検証
（BUG-25追補5、2026-08-27 ユーザー確認）が行われた実機の`session_keymap`は
**CUSTOM（BUG-115、2026-09-05時点）または MSIME（ADR-174、2026-09-15時点）**
であり、ATOKプリセットではない（B2）。

ADR-135「スコープ確定」節が記録する `atok.tsv` 実データ:

```
atok.tsv: DirectInput {Hankaku/Zenkaku,Henkan,Kanji,Muhenkan,ON} → 各種（Hiragana/Katakana行なし）
```

ATOKプリセットには `Hiragana` 行が無い。`Precomposition` 側の行は本ADR・
ADR-135とも未確認である。**`VK_DBE_ALPHANUMERIC`/`VK_DBE_HIRAGANA` を
ATOKプリセットのGJIへ送って期待どおり conv が切り替わるか**は、
実機で確認していない前提である。

加えて `VK_DBE_ALPHANUMERIC` は、`.claude/rules/experiment-logging.md` が
「5日間に6回、採用と撤回が反転した」と名指しで記録している当事者VKであり
（`534051a`→`098c663`→…→`489cdf1`）、ADR-135「スコープ確定」節も
「これは半角英数(IME ON)であって直接入力ではない、という同じ事実を
その都度再発見していた」「Eisuを含めると過去に複数回振り出しに戻った
論点を検証不十分なまま作り込むことになる」として**明示的にスコープ外に
した**キーである。本ADRはこのVKを無変換という新しい起点に結び付けようと
しているので、ADR-135がスコープ外とした理由に正面から答える必要がある。

### M6. 無変換3連打によるエンジンOFF（`engine_off_solo_repeat_vk`）との相互作用が未検討

`src/engine/nicola_fsm.rs:221` / `engine.rs:100` の
`engine_off_solo_repeat_vk` は、無変換の単独タップ**連打回数**を数えて
エンジンをOFFにする機能である（auto-memory
`project_consecutive_muhenkan_engine_off`）。

ADR-184 の設計では、**同じ単独タップが同時に conv トグルも起こす**。
3連打すると conv が3回トグル（奇数回→半角英数で終端）した上で
エンジンがOFFになる。ユーザーが「エンジンを止めたい」だけのつもりで
打った3連打が、IMEを半角英数に置いて終わる。

さらに `output.conv_mutation_allowed`（`output/mod.rs:306,695`）は
`ConvModeAuthority::UserOwned`（engineがuser-disabled）の間 false になり、
その文脈では conv に一切触れてはならない（`key_pipeline.rs:219-225`
の M-4 コメント）。**3連打の3打目は、その打鍵自身がauthorityを奪う**ので
順序依存の穴になる。

### M7. `AppImeProfile::InputRelay` と `conv_mutation_allowed` の扱いが無い

- **InputRelay**（MWB/RDP等、ADR-119決定4/issue #136）: awase自身が
  actuationを所有しないプロファイル。ここで無変換の生キーを
  no-op（B5の正しいレバー）にしつつ awase も conv を書かないと、
  「OS側にもawase側にも誰も切り替えない二重の空振り」——ADR-119が
  明文で禁じた不変条件に**新規に**該当する。BUG-115「Phase 3設計上の
  既知の限界」節が、`Decision::Consume` 経路には `transport.rs::plan` の
  Allow が構造的に効かないことを既に記録している。
- **`conv_mutation_allowed`**: 上記M6のほか、`AwaseOwned` でない文脈
  （`ConvModeAuthority::UserOwned`）全般で新トグルを止める必要がある。

`.claude/rules/fix-requires-evidence.md` の
「IME actuation 合流点（新しい gate/precondition を足す場所、ADR-119）」
行が列挙する合流点すべてについて、新しい gate の要否を洗い出すこと
（issue #136 の自己回帰の再演を避ける）。

### M8. 証拠の出所・ビルド（コミットハッシュ）が記載されていない

ADR-184 の全ログ引用に、(a)ログファイルのパスと保全状況、(b)取得時刻、
(c)**走っていた awase のビルド（コミットハッシュ / PID）** が無い。

これは形式の問題ではなく、結論を左右する。現ブランチHEAD `c8bc1adc`
（`feat(adr090): warrant強制(A-2)を実装`）は
`ImeOpenOutcome::Unwarranted` を新設し、warrant無しの書き込みを
**実際に止める**ようにした。ADR-182 は同じ実機ログで
「`apply(open=false)` は `outcome=Unwarranted`（`c8bc1adc`のwarrant）で
止まっている」と記録している。

一方 ADR-184 手順3 は `[apply-ime] GJI direct: send 0x001A (open=false)`
＝**実際に送っている**ログを引いている。両者が同じビルドなら矛盾、
違うビルドなら ADR-184 のログは `c8bc1adc` 以前のもの——**つまり現在の
HEAD では既に実IMEは閉じられていない**可能性がある。この場合、
ADR-184が「真因」と呼んだ往復の前半は既に解消済みで、残るのは
belief書き込み（`handle_engine_set_open(false)`）とB1側だけになる。

ADR-182 は参考になる書き方をしている（「解析対象の`awase.log`は……
現存しない。本ADRの数値は`gaps.log`等に基づく」）。同水準の記載を求める。
`.claude/rules/tuning-constants.md` の精神（実測の出所を残す）に加え、
auto-memory の
`feedback_confirm_physical_key_and_config_source_before_remote_test`
（押したキー・configパス・PID/コミットを毎回確認してから結果解釈）に
直接該当する。

### M9. `ime-belief-architecture.md` 準拠の設計記述が無い

新しい能動的訂正を入れるなら、同ルールの
「新しい能動的訂正を追加する場合は `InputModeApplyStrategy` に専用の
variant を追加すること」に従う必要がある（既存: `ImmBrokenCorrection` /
`PanicReset` / `CacheRestore` / `PostSetOpenEisuReset` /
`UserImeOnEisuReset` / `UserHalfWidthAlnumToggle`）。
`UserHalfWidthAlnumToggle` を無変換経路で流用すると、journal/ログ上で
「左Shift由来」と「無変換由来」が区別できなくなる——ADR-159/163の
記録・再生基盤（`ActuationDecision` の `DecisionSite`）が
`IdleConvCheckDirectInput` を後から足して同型の問題を潰した前例がある
（`key_pipeline.rs:1154-1158` の `/code-review` 指摘 B-2）。

加えて `crates/awase-windows/tests/architecture_guard.rs` の
`InputModeObserved`/`user_ime_on_paths_are_paired_with_eisu_reset` 等の
件数固定テストへの影響を設計段階で確認すること。

### M10. 回帰防止の選択が「golden か known-bugs か」の二択になっている（「未解決の疑問」7）

本変更は `fix-requires-evidence.md` の再発ファミリーの**4つ**に同時に
該当する: 「キー選択（`nicola_fsm.rs::resolve_pending_thumb_as_single` を
名指しで含む）」「IME belief」「conv mode」「IME actuation 合流点」。

- `resolve_pending_thumb_as_single` の優先順位変更は
  `src/engine/tests.rs`（ルート`awase`クレート、Linuxで実行可）が
  BUG-119/ADR-147 の前例どおり最適。ADR-182「検証計画」1〜5 が
  既に具体的なテストシナリオ（gap 108ms、フラグ寿命、7入口の伝播表）を
  設計しており、統合すれば大半を共有できる。
- 送信キー列（`VK_DBE_ALPHANUMERIC`/`VK_DBE_HIRAGANA`）の固定は
  `ime_key_sequence_golden.rs`（Windows専用）。
- 実機依存の部分のみ `docs/known-bugs/BUG-NNN.md`（本文30行以内）。

「どちらか」ではなく「どこまでをどれで担保するか」を設計に書くこと。

---

## Nits

### N1. ADR本文に auto-memory の wikilink を書いている（ユーザールール違反・4回目）

`docs/adr/184-...md:7`（summary内）:
```
ADR-183調査（VK誤認識訂正、(教訓: 通称のキー名からVKを仮定せず、実機ログで実際のVKを確認してから設計する)）
```

auto-memory の `feedback_no_memory_wikilinks_in_source_code`
（「記憶システムのwikilink`[[...]]`構文をソースコード/リポジトリ内docに
書かない、2026-09-13で3回目の再発」）に違反。ADR本文での参照は
「ADR-183参照」の平文にすること。`docs/adr/183-vk-kana-physical-delivery-passthrough.md`
の status 節にも同じ違反がある（そちらは本レビューの対象外だが同時修正推奨）。

### N2. タイプミス

- 本文「未解決の疑問」1: `conv-only トグン` → `トグル`
- 「現状の機序」冒頭: `resolve_pending_ thumb_as_single`（行折り返しの
  位置にスペース）——コピペでgrepできなくなる。

### N3. 「新しい不具合ではなく既存の reincidence family の一発現」という自己評価の根拠が弱い

`docs/known-bugs/BUG-015.md` 系統との「同型」性が本文で説明されていない
（BUG-015 は存在するファイル）。同型と言うなら、どの機序が共通なのかを
1行で書くか、記述を落とすこと。

### N4. `status` に実装前提条件が書かれていない

ADR-181/182/183 と異なり、ADR-184 の status は
「ドラフト（起票、opus-adversarial-consult round1前）」のみ。B2の
実機確認を「実装着手の前提条件」として status に明記すると、次セッションが
未確認のまま着手するのを防げる。

---

## 次のラウンドへの要求（優先順）

1. **B2 の実機確認を先に行う**（`config1.db` の `session_keymap` 実値と
   `classify_mode_key_ime_action(Muhenkan, ..)` の戻り値）。ここが ATOK で
   なければ、設計の前提条件そのものを組み直す必要がある。
2. **B1 の切り分け**（手順5のF2が物理か ADR-181 の自己主張か）。
3. **B3 を撤回し、ADR-182「別ADR/BUGとして分離」への参照に置き換える。**
   合わせて M8（ビルド特定）を行い、`c8bc1adc` 後に実送信が起きるのか
   `Unwarranted` で止まるのかを確定する。
4. **ADR-182 決定1 との統合方針を決める**（B5/M2）。同じ
   `resolve_pending_thumb_as_single` の同じ位置に、互いを参照しない
   2つのガードを別々のADRで入れようとしている状態を解消する。
5. 上記が片付いた上で、B4（belief書き込みの3点セット）・M3（状態の
   持ち方）・M4（commit規律）・M6/M7（他機能との相互作用）を設計に反映し、
   round2 へ。
