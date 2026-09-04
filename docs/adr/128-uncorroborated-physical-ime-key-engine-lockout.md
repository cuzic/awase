# ADR-128: 物理IMEキー1回による明示意図が、失敗しても所有権を返さない問題

## ステータス

**設計継続中（v2、未収束）。** Opus 2体（architect/premortem）の敵対的
レビューで、v1が提示した2案（A: 証拠強度の分離、B: 有界reconciliation）と
その統合案（B': 明示意図の有界失効）のいずれにも、実コードで裏付けられる
blockerが見つかった。因果分析はv1から大きく訂正された（下記）。まだ
「決定」に至っていない——次に検証すべき方向性（B''、後述）は見えているが
実装可能性の検証は未了。

## 背景

### 直接のきっかけ: 不具合報告 `01M1MMK8987NT5B2W73PCPZNZ1`

2026-09-03、Windows Terminal + PowerShell（GJI、JIS配列）で入力中に余分な
「@」が出力されるとの報告が届いた（`docs/bug-reports-triage.md` 該当行）。

### 因果分析（v2で全面訂正）

**v1の因果分析（「drift correctionが無期限リトライを続けたため29秒間
エンジンがOFFのままだった」）は、Opusレビューで実コードに基づき誤りと
判明した。** 実際の機構は以下のとおり:

1. `21:51:55.702`、物理CapsLock位置（scan=0x3A、JISキーボードでは
   「英数」キーでもある）から `vk=0xF0`(`VK_DBE_ALPHANUMERIC`) の
   `KeyDown` が `injected=false` で届いた。
   `kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs`）が
   `write_physical_key()` を呼び、以下の3箇所を同時に書き込んだ:
   - `desired_open = false`
   - `shadow_model.last_intent`（`state/ime_model.rs`）
   - `intent_store` の対象hwndエントリ（`state/intent_store.rs`、
     `EXPLICIT_OFF_INTENT_TTL_MS = 30_000`（`tuning.rs:437`）を持つ）
2. `PlatformState::effective_open_at()`（`state/platform_state.rs:557-591`）
   は `intent_store.resolve_effective_open(focus, shadow, now)` を呼ぶ。
   `IntentStore::resolve_effective_open`（`state/intent_store.rs:158-174`）
   は、対象hwndに**期限内のエントリがあれば、shadow（＝observed）を
   一切見ずに `intent.open` をそのまま返す**。TTLは30秒、**クリアされる
   条件は `FocusChanged` の1箇所のみ**（`ime_model.rs:621`、
   `last_intent`側も同様）。
   - `has_user_explicit_intent()` が true の間、`resolve_open_at()`
     （`ime_model.rs:400-412`）も同様に `desired_open` を無条件採用し、
     `observations` を一切見ない。
3. `21:51:57.796`〜`21:52:07.312` の間 `[drift] correction:
   observed=true ≠ desired=false` → `apply_ime_open(false)` が14回超
   反復したが、**これは29秒間のうち約9.5秒（`FeedbackPolicy::Blind`
   max_attempts=5 × 再武装cooldown 3000ms、`tuning.rs:289` から算出した
   周期と実測が一致）でしかない。残り約17秒はdrift correctionと無関係
   に、`IntentStore`/`last_intent` がフォーカス変更を待つだけの
   純粋な待機だった**。
4. `Engine activated` が再度出る `21:52:24.474` の直前、
   `21:52:24.471` に `FocusChange [17416→14684] awase_tray_window`
   （不具合報告ダイアログへのフォーカス移動）が記録されている。**この
   フォーカス変更こそが `last_intent` をクリアし、ロックアウトを
   解除したトリガーである。** 「報告ボタンが押される瞬間まで29秒
   ロックされていた」のは偶然の一致ではなく、フォーカス変更が起きた
   瞬間まで解除されなかった、という直接の因果である。
5. journalのK,Y,O,Uキー入力（`decision: PassThrough`, `state: Idle`）は
   この期間中（wall-clock `21:52:06.8`〜`21:52:07.5` 相当）に発生して
   おり、NICOLA変換が一切かからず生のJIS配列文字が素通りしていたことと
   符合する。ただし「＠」がどの物理キーに由来するかはjournalから直接
   特定できておらず、使用中の `.yab`（本報告の添付ファイルは`＠`を
   左親指シフト面のPキー右隣に明示的に割り当てている）に依存する
   因果である点は未確定のまま残る。

**再定義した問題**: 「証拠強度」でも「リトライ回数」でもなく、
**「awaseが物理IMEキー1回でIME状態のactuation所有権（30秒の絶対的な
`explicit intent`）を取り、actuationが実際に失敗し続けている（drift
correctionが14回超空振りした）という機構内部で既に判明している情報が
あっても、その所有権を返す経路が存在しない」** ことが実害の本質。

### `vk=0xF0 scan=0x3A` の信頼性について（v1からの訂正）

v1は「`vk=0xF0 scan=0x3A` は既知の低信頼シグナル」と前提したが、
これは不正確だった。`docs/known-bugs.md` BUG-15追補7が指摘する
「spuriousな0xF0を生む」経路は **scan=0x70（かなキー）側**であり、
`scan=0x3A` はJIS(kbd106)における英数キーの**正規のコード**である。
`vk.rs:196-198`（ADR-093）のコメント「この5 VKは通常の物理キーボードに
存在しない合成コード」の方が、このJIS専用キーの実在を見落としている。

一方で、`scan` は物理キー押下と合成/エコーイベントを区別する識別子に
**なりえない**——`VK_DBE_ALPHANUMERIC` を `MapVirtualKeyW` で逆引きすると
機械的に `scan=0x3A` が得られるため、awase自身や他プロセスが合成した
0xF0も同じscanを持つ（`docs/known-bugs.md` BUG-25追補1）。また
`src/config.rs:289-291` には、MS-IMEの「キーとタッチのカスタマイズ」に
よるOS/ドライバレベルの翻訳が `injected=false` で届く実機事例が
既に記録されている。**したがって、今回のイベントが「ユーザーが実際に
英数キーを押した」のか「何らかのIME側のエコー」なのかは、
`injected=false` という事実だけからは断定できない。** ただし
`send_gji_half_width_alnum_toggle`（`output/mod.rs:1102-1111`、
BUG-25半角英数トグル）はSendInputを使うため該当すれば`injected=true`に
なるはずで、この特定の自機能によるエコーは今回は除外できる。

### 検討した3案とその棄却理由

Opusレビュー（architect/premortem、2ラウンド）で以下の3案を検討し、
いずれも実コードで裏付けられる blocker が見つかったため、**このまま
採用しない**:

#### (A) 証拠強度の分離（`vk=0xF0 scan=0x3A` の意図昇格を弱める）

- **`shadow_effect()` を `None` にする（A-1）と、`shadow_action` を
  唯一の入力源とする `runtime/transport.rs:224-227`（`is_kanji_event`
  判定）が連動して即座に `Allow`（素通し）に倒れる。** 同ファイル
  `:243-262` の `is_dbe_mode_key_down`（`dbe_mode_key_policy=Suppress`
  既定、BUG-52対策）は `is_kanji_event` の内側にあるため丸ごと
  バイパスされ、実IMEがネイティブに英数へ切り替わってしまう
  （BUG-52・BUG-15追補7の「belief ON × 実OFF」窓の再導入。JIS配列
  全般 × 任意IME × 英数キーで発生しうる）。
- **A-2（裏付け待ちの保留）は、裏付けとなる2回目の証拠がTsfNative/
  Imm32Unavailable（本報告のプロファイルそのもの）では構造的に
  取得できない**（`state/observation_store.rs`
  `ObservationOutcome::NotObservable`）。今回の再発防止を狙った変更が、
  同じプロファイルで英数キーを恒久的に無効化する形にしかならない。
- ADR-121（no-op時の冪等再送）の救済経路と同じ`kp_stage_shadow_ime_toggle`
  の同じ分岐を奪い合う（意図に昇格しなければno-op分岐にも到達しない）。
- ADR-093の「これらのVKは合成コードであり受信自体が証拠」という前提と
  真っ向対立する（同じイベントを「信頼できる外部事実」と
  「awaseの推測で割り引く対象」の両方として扱うことになる）。
- ADR-119決定1（「解釈しない入力は消費しない」）とBUG-52の「toggleが
  発火したか否かに関わらずSuppressせよ」という要求が両立しない中間状態を
  作る。

#### (B) 有界reconciliation（drift correctionが乖離し続けたらdesiredをobservedに訂正）

- **今回の`observed=true`はほぼ確実に`ConvOpenInference`
  （`kp_stage_idle_conv_check`起源）であり、`state/evidence.rs:138`で
  `ObservationAuthority::BeliefOnly`（＝actuationの根拠にしてはならない）
  と型で宣言済みのソースである。** `state/ime_event.rs:115-130`は、
  過去にこの推論を`desired_open`書き換えの根拠にして「ユーザーが
  明示的にOFFにした直後にエンジンが勝手にONへ戻る」BUG-19を起こした
  経緯を名指しで記録している。Bをそのまま実装するとBUG-19の型どおりの
  再発になる。
- `ConvOpenInference`はIMM32のNATIVEビット（開閉と無関係な持続設定、
  BUG-68）から作られるため「observedが乖離し続ける」ことは「実IMEが
  開いている」ことを含意しない——**そもそも収束しえない性質の観測**。
- `ConvOpenInference`を除外すると今回のケースでは一度も発火しない
  （TsfNativeでは`Actuating`権威を持つ観測源が構造的に来ない）。
  「安全にすると効かない、効かせるとBUG-19を作る」というジレンマ。
- `desired_open`への書き込みは`ImeModel::reduce()`の4 variantに限定
  されており（`.claude/rules/ime-belief-architecture.md`）、Bは5つ目の
  書き込み経路を必要とする。既存の3 variant（`UserImeSetIntent`等）の
  流用は意図の偽装になり不可、新設するなら`lints/ime_event_guard`と
  `architecture_guard.rs`の同時更新が要る。

#### (B') 明示意図の有界失効（`desired_open`は書き換えず、GiveUp N巡後に`last_intent`/`IntentStore`のエントリだけを失効させる）

architectがA・Bの両方の欠陥を踏まえて提案した第三案。「`desired_open`を
捏造しない」という点でBUG-19の型を形式上は回避しているように見えたが、
premortemの検証で**実効挙動はBとほぼ同一であり、むしろ新たな固着状態を
生む**ことが判明した:

- **`last_intent`を消した瞬間、`resolve_open_at()`は`derive_any()`に
  フォールバックし、`ConvOpenInference`1件（Medium confidence）だけで
  `effective_open()`をtrueに反転させる。** これは既存テスト
  `resolve_open_at_decided_by_derive_medium_mise_bug_scenario`
  （`ime_model.rs:1317-1345`）がBUG-26のために意図的に固定している
  挙動そのもの——「ユーザーの明示OFFがconv 1件で覆される」という
  Bと同一の実害が、書き込み経路を変えただけで再現する。
- `IntentStore`のエントリ削除は、まさに「壊れたconv 1件だけでは
  `effective_open()`を反転させない」ことを守るために存在する装置
  （`intent_store.rs:149-156`、Linux CIで`tests/intent_store_effective_open.rs`
  が固定）を名指しで無効化する行為になる。
- `explicit_intent()`が`None`になると`check_drift_correction`
  （`platform_state.rs:821-823`）の`ConvOpenInference`短絡ガードが
  発火し、**drift correction自体が停止する**。定常状態は「belief=ON
  （B'-1により反転）× 実IME=OFF（VK_IME_OFFを14回送った後ならこちらの
  可能性が高い）× 訂正の契機なし」——NICOLA変換が実行されつつ実IMEには
  届かず、シェルにromajiが直接入力される（BUG-02/BUG-63型のリテラル
  出力）。**元の症状（エンジンOFFで生キー素通り）より実害が大きい
  可能性がある。**
- 失効後にユーザーが再度英数キーを押すと、`effective_open()`が既に
  trueに反転しているため`current≠new_val`となりno-op判定を通らず、
  同じ書き込み→GiveUp N巡→失効のサイクルが**周期的に繰り返される**
  （29秒の1回のロックではなく、フォーカスが変わるまで続く限界サイクル）。
- `GiveUp`という概念自体が`FeedbackPolicy::Blind`（TsfNative/
  Imm32Unavailable）専用で、`Read`ポリシー（ImmCross系、LINE/Qt/WezTerm等）
  では発火しない——一般的な安全弁としてはプロファイルの約半分で
  dead code になる。
- カウンタの置き場所（`Actuation`は`AnyFreshEvidence`再武装のたびに
  `discard_actuation()`で破棄される）が未解決のままで、そもそもN巡に
  到達する前にリセットされ続ける。

### 次に検証すべき方向性（B''、未検証）

premortemが対案として提示した、まだ実装可能性を検証していない方向性:

**belief（IME open/closedの信念）とengine activation（NICOLA変換の
稼働）を分離する。** 具体的には:

1. 明示意図を失効させる際、`last_intent`を消すのではなく**強度を
   下げる**（`Explicit` → `Weak`のような区別を導入する）。`Weak`の間は
   `ObservationAuthority::Actuating`を持つ観測源（`ImmGetOpenStatus`/
   `ImmCrossProbe`/`ObserverPoll`/`Gji`/`Tsf`）のHigh単独/Medium合意
   でのみoverrideを許し、`ConvOpenInference`等の`BeliefOnly`ソースでは
   overrideさせない。TsfNativeでは`Actuating`ソースが構造的に来ないため
   belief=OFFのまま据え置かれ、B'-1/B'-3の反転・固着を回避できる。
2. **そのうえで、NICOLA変換エンジンの活性化条件を`effective_open()`
   から切り離す。** 今回の実害は「IME状態の信念が間違っていたこと」
   ではなく「エンジンが29秒間停止し続けたこと」なので、beliefの意味論に
   一切触れずにこの実害だけを消せる可能性がある。

この方向性は`NotRomajiInput`/`ObservedEisu`ガードや`build_ctx().ime_on`
の消費者群（複数箇所）との整合が未検証であり、実装可能性そのものが
次のレビュー対象になる。

### 関連ADR/既知バグ

- **BUG-15追補6・7**、**BUG-14**、**BUG-25追補1**、**BUG-68**、
  **BUG-19**、**BUG-26**、**ADR-087**（belief vs actuationの権威分離の
  出典）: 上記の因果・棄却理由で個別に参照。
- **ADR-093**（`docs/adr/093-*.md`）: 合成VKコード基盤。今回0xF0が
  JIS実キーであることが判明し、この ADR の前提（「物理キーボードには
  存在しない」）自体に見直しの余地があると判明した（副産物、本ADRの
  スコープ外）。
- **ADR-121**（実装未着手）: 物理IMEキーがeffective_openと同じ方向
  （no-op）だったときの欠落を扱う。D4で「OFF方向は別課題」と明記して
  おり、その「別課題」が本ADRの対象領域にあたる——決定時にADR-121 D4を
  本ADRが吸収するか明記する必要がある。B'案はADR-121のno-op救済経路も
  実質的に無効化する（`last_intent`消去後はno-op判定自体が成立しない）
  ため、両ADRの決定は独立に確定させられない。

## 問題（再定義）

物理IMEキー1回の検出（`write_physical_key`、`IntentKind::PhysicalImeKey`）
が、明示的なホットキー操作（`SyncKey`、例: `Ctrl+無変換`）と全く同じ
30秒・フォーカス限定解除の絶対的権威（`IntentStore`/`last_intent`）を
獲得する。actuation（drift correction）が実際に失敗し続けている
という情報がシステム内に既にあっても、この権威を返却する経路が
存在しないため、フォーカスが変わるまで最長30秒（今回は約29秒）、
NICOLA変換エンジンが停止し続ける。

なお、0xF0を個別に塞いでも`SyncKey`や`VK_KANJI`等、他の経路からの
1回の誤検出で同じ29秒ロックアウトが再現しうる（`last_intent`が
`FocusChanged`でしかクリアされないという構造そのものが原因のため）。
**入口（どのVKを信頼するか）ではなく、出口（獲得した権威をいつ・どう
返すか）を直す設計でなければ、この問題ファミリー全体は解決しない。**

## 俯瞰的な根本原因（v3、複数の非連続案）

A/B/B'がことごとくblockerに突き当たった理由を一段抽象化すると、
共通の構造的原因が見える: **このシステムは論理的に別の3つの関心事を
1本のスカラー（`effective_open()` / `desired_open` / 単一の
`IntentStore`）に押し込めている**——

1. **actuation対象への権威**（実IMEをどちらへ動かそうとするか）
2. **NICOLAエンジンの活性化ゲート**（ローカルなキー変換をするか）
3. **その状態変化を引き起こした証拠の確度**（ホットキーか、単発の
   曖昧なDBEキーか、観測からの推論か）

BUG-19/BUG-26/BUG-68はいずれも(1)を守るために作られた防御だが、(2)は
(1)と同じパイプを共有しているため、(1)向けの防御をそのまま相続して
しまい、(2)にとっては過剰に重い代償（29秒の変換停止）を払う。加えて
権威の解除条件（`FocusChanged`のみ）が「不確実性が実際に解消したか」
と無関係な偶発的イベントに紐付いているため、ロックアウトの長さは
「たまたまフォーカスが変わるまで」という無原則な値になる。

以下は、この3つの関心事の**分離**を軸にした、非連続な変更を含む
複数の方向性。単独でも、組み合わせても成立しうる。

### 候補1: engine活性化をIME belief から分離する（B''の具体化、最有力）

NICOLA変換の可否を`effective_open()`から完全に切り離し、独自の
活性化ゲートを持たせる。既定は「明確な反証（`SyncKey`＝ホットキー、
または`ObservationAuthority::Actuating`の観測）が無い限りengineは
動き続ける」——今回のように単発の曖昧な`PhysicalImeKey`イベントは
actuation対象の`desired_open`は更新してよいが、**engineのON/OFFには
一切関与させない**。

- **利点**: BUG-19/26/68がガードするactuation経路には一切触れない
  ため、それらの防御を壊さない。premortemが提示した中で唯一
  「belief意味論に触れない」方向。
- **リスク**: 実IMEが本当にOFFへ切り替わっていた場合、engineが動き
  続けるとromajiが直接アプリへ送られうる（BUG-02/63型のリテラル
  出力）。ただし、これは「見えない29秒のロックアウト」より「即座に
  目に見える誤入力→ユーザーがBackspaceで気づいて訂正できる」形で
  あり、このリポジトリが他所（LiteralDetect、ADR-120の事後訂正）で
  既に採用している「検出して修復」パターンと同じ性質のトレードオフ。
  `NotRomajiInput`/`ObservedEisu`ガード等、既存の消費者との整合は
  未検証。

### 候補2: 権威の粒度を`IntentKind`単位に分割し、`derive_any()`の
既存の潜在バグも同時に塞ぐ

`IntentStore`は現在「`SyncKey`（ホットキー、複数キー同時押しで
偶発的な誤発火が構造的に起きにくい）」と「`PhysicalImeKey`（単発の
DBEキー、IME/ドライバのエコーと区別不能）」を同じ30秒TTLで扱っている。
これを`IntentKind`ごとに独立したTTL・権威にする（例:
`PhysicalImeKey`は数秒、`SyncKey`は現状の30秒を維持）。

**このレビュー中に副産物として見つかった独立の潜在バグ**: `IntentStore`
のエントリは現在の実装でも30秒で自然に期限切れになる。期限切れ後は
`resolve_open_at()`が`derive_any()`にフォールバックし、`Medium`単独の
`ConvOpenInference`だけで`effective_open()`が反転しうる
（`ime_model.rs:1317-1345`のテストが固定する挙動）。**つまり今回の
インシデントがもし報告ダイアログへのフォーカス移動で中断されず
30秒を超えていたら、B'と同じ「belief=ON×実IME=OFFの固着」が、
どの設計変更も無しに今の`develop`で発生していた。** これは本ADRの
決定と無関係に、それ自体で別途（`docs/known-bugs.md`記録または
別ADR）修正または少なくとも記録する価値がある。候補2を採るなら
「単発証拠のTTLを短くする」だけでなく「TTL失効時に`derive_any()`へ
無条件フォールバックしない（複数の合意、または`Actuating`権威を
要求する）」も同時に直す必要がある——さもないと単にロックアウトの
時間を29秒から数秒に縮めるだけで、同じ形の固着がより早く起きる。

### 候補3: 状態不確実性を「隠れて自動修復を試みる」から
「即座にユーザーへ可視化する」へ転換（UXレベルの非連続案）

TsfNative等の観測不能プロファイルでは、実IME状態はそもそも
プログラム的に解決不能な既知の限界（CLAUDE.mdが述べる「IME状態追跡を
resilientかつauditableにする」という本プロジェクトの存在意義その
もの）。drift correctionが規定回数（例: GiveUp 3巡）失敗した時点で、
`desired_open`もengineも一切書き換えず、**トレイアイコンの一時的な
点滅や控えめな通知で「IME状態を見失った、気になったら該当キーを
もう一度押してほしい」と明示する**。

- **利点**: `desired_open`/`observed`/`derive_any()`のどれにも触れない
  ため、A/B/B'が踏んだblockerを一つも踏まない。実装は既存の
  `ActuationRecord`のGiveUpイベントを起点にするだけで完結する。
- **弱点**: 単独では29秒の変換停止という実害そのものは消えない
  （候補1と組み合わせるのが自然）。ユーザーへの通知UXの設計が
  別途必要（過剰な通知は逆にノイズになる）。

### 候補4: 証拠の質フィルタをobserverレイヤーへ引き上げる
（構造的にもっとも筋が良いが最も侵襲的）

A案が失敗した本質的理由は、「証拠を割り引く」判断を`shadow_effect()`
（belief層）に置いたため、`transport.rs`のSuppress/Allow判定
（配送層）と分離できなかったこと。`.claude/rules/ime-belief-architecture.md`
のObserve→純粋`classify_*`→`reduce()`という規律に従うなら、証拠の質
判定は**Observeの時点**（`hook.rs`、実際のKeyDown/KeyUpタイミングを
持っている層）に置くべきである。

具体的には、`VK_DBE_ALPHANUMERIC`等のDBEモードキーについて、
（a）対応する物理的なKeyUpまでの保持時間（BUG-14/ADR-093が既に
「0.5〜4ms=合成、数十ms以上=物理」という基準を持っている）、または
（b）ペアとなる相補的なDBEイベント（0xF0/0xF2等）の時間間隔、を
observerレイヤーで判定し、**確度が閾値未満のイベントはそもそも
`ImeRelevance`の`shadow_action`を持たない別の型として`hook.rs`が
分類する**。こうすればbelief層・配送層のどちらも「証拠が弱い」ことを
自然に扱え、A案が起こした「配送はAllow、beliefはNone」という
中間状態の矛盾が起きない。

- **利点**: BUG-15追補7が指摘する不安定性の根本（このVK×scanの
  組み合わせを信頼できるかどうかを個別のconsumerがバラバラに
  判断している現状）を、生成源で一度だけ解決する。ADR-093
  自体の前提（「物理キーボードに存在しない」）の誤りも同じ場所で
  訂正できる。
- **リスク**: `classify_ime_relevance`のシグネチャ変更・
  `architecture_guard.rs`/`layer_boundary_guard.rs`への影響・新しい
  保持時間の実測（`tuning-constants.md`の実測義務）を要する、最も
  大掛かりな変更。

### 候補5（最も非連続）: 単発DBEキーをbelief書き込み源から完全に除外し、
config化されたホットキーのみを「明示的IME OFF」の唯一の入口にする

`docs/known-bugs.md`の履歴（BUG-15追補6・7、5日で6回反転したIME OFF
キー選択、BUG-25追補1の scan衝突）を俯瞰すると、`VK_DBE_ALPHANUMERIC`/
`scan=0x3A`という組み合わせは**2026-07-07から一貫して不安定であり
続けている**。今回のインシデントはその同じ不安定性が7週間越しに
新しい形（belief誤確定→ロックアウト）で再発したものと見なせる。

大胆な選択肢として、この特定のVK×scan組み合わせ（および相応の
既知不安定ペア）を**そもそも`IntentKind::PhysicalImeKey`の対象から
恒久的に除外**し、`is_japanese_ime()`のupgradeという既に確立している
安全な用途（ADR-093）だけに限定する。ユーザーが物理IMEキーで
確実にIMEを切り替えたい場合は、awase自身のホットキー（`keys.ime_off`
等、複数キー同時押しで偶発発火しにくい）を設定して使う運用に一本化
する。候補4のobserverレイヤー判定と組み合わせれば「既知に不安定な
組み合わせだけ即除外、それ以外は保持時間で判定」という段階的な
適用も可能。

- **利点**: 実装は最小（`shadow_effect()`の対象から除く1行に近い）。
  BUG-15追補7以来7週間、誰もこの組み合わせを完全に信頼できたことが
  ないという実績に率直に向き合う。
- **リスク**: 英数キー単独でIME切替をする既存ユーザー体験を退行させる
  （UXの後退として明示的に許容するかはユーザー判断）。

## 決定

**未確定。** A/B/B'はいずれも不採用。上記5候補（単独・組み合わせ
いずれも可）のうちどれを次に検証するかはユーザー判断待ち。候補2で
見つかった`derive_any()`の潜在バグは、本ADRの決定と独立に対処する
価値がある。

## 実装状況

未着手。

## 次のアクション

1. **実機検証（最優先、1分で可能）**: 同一環境でエンジンOFFを再現させ、
   フォーカスを他アプリへ移して戻すだけで即座にエンジンが復帰するかを
   確認する。復帰すれば上記の因果分析（`last_intent`のFocusChanged依存）
   が実証される。
2. `[hook] IME-mode`ログ（`hook.rs:837`）の`self_injected`/`injected`/
   `scan`生値と、0xF2との時間間隔を確認し、今回のvk=0xF0が真に物理
   キー押下だったか、IME側エコーだったかを裏取りする。
3. B''（belief/engine分離）が`NotRomajiInput`/`ObservedEisu`ガードや
   `build_ctx().ime_on`消費者と整合するかを設計・検証する。
4. `.claude/rules/fix-requires-evidence.md`の「IME belief」
   「キー選択」両ファミリーに該当するため、決定後は回帰テストまたは
   `docs/known-bugs.md`記録を伴わせること。「どのキーをIME OFF意図と
   解釈するか」は過去5日間で6回反転した領域（`docs/experiments.md`
   エントリ01）であり、拙速な決定は同じ轍を踏みやすい。
