# ADR-132: 物理IMEキー1回による明示意図が、失敗しても所有権を返さない問題

## ステータス

**Phase 1・Phase 2ともに実装済み（developとの同期後、v5）。** Phase 1
（候補3のUX可視化＋診断ログ7項目）はOpus 2体（architect/premortem）
敵対的レビュー・討論4ラウンドで収束、実機3件の再現を経て根本原因
（`desired_open()`/`effective_open()`/`warmup_ime_on()`の三重SSOT競合）
を確定した。Phase 2はB1（`warmup_ime_on()`が競合する経路）を対象に
Opus 2体で追加2ラウンドの討論を行い、`off_drift_active`ゲート
（INV-B1'）で収束・実装した。詳細は「検討の経緯（v1〜v4のサマリ）」
節（Phase 1）と「Phase 2（B1の修正、v5で追記）」節を参照。

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

## 検討の経緯（v1〜v4のサマリ）とラウンド3・4で判明した重要な訂正

v3で「候補1（engine活性化のbelief分離）」を最有力としたが、Opus 2体
（architect/premortem）による2ラウンドの敵対的討論（ラウンド3: architect
が候補1を「矛盾検出中ラッチ」として具体化→premortemが検証、ラウンド4:
architectが応答・設計を修正→premortemが最終判定）で、**修正後の設計
（候補1-v2）にも2つの独立したblockerが残ることが確定し、最終的に不採用**
となった。この過程で以下の重要な訂正が生まれた（v2/v3の記述を訂正する）:

- **ロックアウトの長さは「最長30秒」ではなく「無期限（`FocusChanged`が
  起きるまで）」。** `IntentStore`の30秒TTLが先に切れても、`ime_model.rs`
  の`last_intent`（TTLなし、`reduce()`の`FocusChanged`アーム1箇所でしか
  クリアされない）が`has_user_explicit_intent()`経由で`resolve_open_at()`
  を止め続けるため、`IntentStore`のTTLは実質無関係だった。今回たまたま
  約29秒で復帰したのは、たまたま不具合報告ダイアログへのフォーカス移動が
  その頃に起きたためであり、上限として保証された値ではない。
- **候補2の根拠にした「潜在バグ」（30秒TTL失効後に`derive_any()`が
  `ConvOpenInference`単独でbeliefを反転させる）は成立しない。** 上記の
  とおり`has_user_explicit_intent()`がtrueの間は`derive_any()`に到達
  しないため。候補2はこの誤った前提の上に立っていたため、それとは
  独立に不採用（実害を1秒も縮めない）。
- **`FocusChanged`による復帰は「観測が残っていて採用された」のではなく
  2段階**: `observation_store.rs::clear_on_focus_change`が観測プールも
  同時に全消しする（`per_source.clear_all()`）ため、正しくは
  「フォーカス変更で`last_intent`と観測プールが同時にクリアされ、その後
  新しいフォーカスプローブが供給した新鮮な観測が採用された」という
  2段階の因果である。

### 候補1-v2「矛盾検出中ラッチ」（却下・設計は記録として残す）

architectが提示した最終形は、`ImeModel`に`ProvisionalGate::{Closed,
Open{opened_at}}`を持たせ、以下のヒステリシス（開閉の非対称条件）で
engineの活性化ゲートだけを`effective_open()`から切り離す設計だった:

- **開く条件（1つ）**: `check_drift_correction()`が
  `Some((desired=false, observed=true, duration_ms))`を返し、
  `duration_ms >= 2000ms`（実測根拠: 本報告のBlindバースト1巡≈3.2秒、
  初回5発完了まで約2秒）。
- **閉じる条件（3つのみ）**: (R1) `FocusChanged`、(R2)
  `ObservationAuthority::Actuating`の観測が`desired_open`と一致、(R3)
  新しい明示意図の記録。
- **意図的に閉じない条件**: 観測のstaleness、`check_drift_correction()`
  が`None`に戻ること、drift correctionのGiveUp/再武装、`BeliefOnly`
  観測の一致、ゲート自体のタイムアウト——いずれも閉じない。

premortemの最終検証で、この設計にも**2つの独立したblocker**が残ると
判明した:

1. **区間がactuation-quietでない（blocker）**: `entering_provisional`/
   `leaving_provisional`という抑制フラグは区間の出入りの2エッジでしか
   `SetOpen`を止めない。drift correction自体（`ir_apply_drift_correction`）
   はactivation stateを見ておらず（`is_user_enabled()`はユーザートグルで
   別物）、区間の全期間にわたって独立に`VK_IME_OFF`を約3.2秒ごとに
   5発ずつ送り続ける。ゲートのタイムアウトを意図的に設けていないため、
   区間長は無期限になりうる。**「NICOLA変換をしながら同じ窓へIME OFF
   キーを撃ち続ける」状態が数分続きうる**——実IMEが開いていれば
   compositionが破壊され（BUG-24/BUG-70型）、閉じていればromajiが
   そのままリテラル出力される。
2. **非対称の向きが安全側と逆（blocker）**: 「弱い証拠（`BeliefOnly`、
   型が『actuationの根拠になれない』と規定するもの）で開き、強い証拠
   （`Actuating`、TsfNativeでは構造的に入手不能）でしか閉じない」という
   設計のため、**誤って開いたことを検出する経路が構造的に存在しない**。
   `tuning.rs`が既に実測付きで記録している「IMM32のNATIVEビットは
   `VK_IME_OFF`で閉じても消えない」（BUG-68）という、まさに今回と同型の
   状況——ユーザーが正しくIMEをOFFにしたのに`ConvOpenInference`が
   `open=true`を報告し続けるケース——で、ゲートは必ず誤って開き、
   TsfNativeでは二度と閉じない。NICOLA変換が閉じたIMEへ勝手に復活し
   romajiリテラルを出す（案Bが持っていたのと同一の実害）。

さらに本質的な論点として、**候補1は「`observed=true`（実IMEは開いている）
の方が正しい」ことを前提にしているが、`tuning.rs:251-253`の既存記述に
照らすとこの前提自体が偽である可能性の方が高い**。前提が偽なら、候補1は
「29秒の無変換パススルー」を「29秒のローマ字リテラル出力」に置き換える
だけで、症状としてはむしろ悪化する。

## 決定

**Phase 1のみを採用する。** 候補3（不確実性のUX可視化）と診断ログ
記録に限定し、候補1/2/4/5は実装しない（候補1-v2は上記のとおり設計を
記録として残し、却下する）。

### Phase 1（採用・実装対象）

1. **候補3（UX可視化）**: `ir_apply_drift_correction`のGiveUp到達
   （`ime_refresh.rs`、`act_gave_up_at == None`側の1回、1フォーカス
   につき通知は1回に制限してノイズを抑える）を起点に、トレイで
   軽い通知を出す。文言は「IME状態が不確実」ではなく、観測不能
   プロファイルの構造的制約として中立に伝える（例:「このアプリでは
   IME状態を確認できません。気になる場合は該当キーをもう一度押して
   ください」）——「awaseが壊れている」と読まれ不具合報告のノイズを
   増やさないため。`desired_open`・`observed`・配送判断・actuationの
   いずれにも触れないため、本ADRの議論で見つかったblockerを一つも
   踏まない。
2. **診断ログの拡充**: 以下7項目を構造化記録する（次にPhase 2の
   可否・BUG-68への転進を判断するために必須）:
   - 矛盾を構成した各観測の`ObservationSource`と信頼度
   - 「矛盾検出中」相当の区間にawaseが送った全VK（出所付き）
   - `[hook] IME-mode`の`self_injected`/`injected`/`scan`と、直前の
     同族DBEイベントとの時間間隔
   - intent昇格時の`IntentKind`（`SyncKey`か`PhysicalImeKey`か）
   - 区間の終了理由（`FocusChanged`か、他の経路か）と区間長
   - 使用中の`.yab`ファイル名（「＠」出力の説明が配列依存で反転するため）
   - `half_width_alnum_toggle_active`の値

### Phase 2（設計凍結・保留）

候補1-v2は上記2つのblockerにより不採用。Phase 1のログでもし「矛盾は
常に`ConvOpenInference`単独由来だった（＝実IMEは正しく閉じていた）」と
判明した場合、真因はBUG-68の未解決部分（conv残骸でdrift correctionが
空回りする）に移る。その場合の対応候補は「`ConvOpenInference`単独では
OFF方向の再送を打ち切る」だが、これは`tuning.rs:259-264`が既に記録する
トレードオフ（打ち切るとBUG-51型の「明示OFF後も実IMEが閉じないまま
最大8分放置される」という別の回復性を失う）を伴う。Phase 1のデータが
揃うまでこの判断はできない。

候補2・4・5は前節（v2/v3）の理由により不採用のまま。

### スコープ外の発見（別途検討の価値あり）

2ラウンドの討論を通じ、「観測不能プロファイル（TsfNative）で
`ConvOpenInference`しか観測が無い環境において、その観測を根拠に
何か（belief でも engine 活性化でも）を自動的に動かそうとすると、
必ず『conv ビットは開閉と無関係』という物理的制約に突き当たる」ことが
繰り返し確認された。次に投資すべきは訂正機構の精緻化ではなく、
**TsfNativeでIME開閉状態を読む観測能力そのものの獲得（あるいはその
不可能性の確定）**かもしれない。本ADRのスコープ外だが、Phase 1のデータが
揃った時点で検討する価値がある。

## 実装状況

**Phase 1実装済み（未マージ、実機ソーク中）。** トレイバルーン通知
（1フォーカス1回）と診断ログ7項目（`DriftGiveUpDiagnostic`/
`HookImeModeDiagnostic`/`DriftGiveUpIntervalEnded`）を追加。
`cargo check`/`clippy`/`fmt --check`/`cargo dylint`/
`architecture_guard.rs`/`layer_boundary_guard.rs`/`cargo test --lib`
（core・awase-windowsとも）は全てpass。`desired_open`・`observed`・
`shadow_effect()`・`transport.rs::plan`・`IntentStore`のいずれも変更
していない。`docs/known-bugs.md` BUG-110として記録済み。

ユーザーが実機で2件目の再現（`01M1N5RMQ0HGX60VNG37DXWR26`）を確認した
過程で、トレイ通知が`FeedbackPolicy::Blind`の`GiveUp`到達だけを
トリガーにしていたため`FeedbackPolicy::Read`（今回の再現ケースが該当）
では絶対に発火しない設計上の穴が判明し、修正済み（トリガーを
`duration_ms`ベースの`Blind`/`Read`共通発火点に変更、新規タイミング
定数は追加せず既存の`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`を再利用）。
詳細は`docs/known-bugs.md` BUG-110追補1・追補2を参照。

**3件目の再現（`01M1NA7WYH1HCYAFWGA3F95AVY`）で、本ADRの4ラウンド討論
（A/B/B'/候補1-v2）が前提にしていなかった根本原因が判明した。**
`ir_check_drift_correction`が読む`desired`は`ImeModel::desired_open()`
（生のbeliefフィールド）だが、`runtime/mod.rs::
apply_force_on_for_imm_broken`（`is_eligible_for_ime_force_on()`=
`is_japanese_ime() && effective_open()`を条件に周期リフレッシュへ
相乗りして`VK_IME_ON`を再送し続ける、ADR-086 INV-15が例外的に許容する
「nonaiyo問題対策」の周期タイマー経路）が読む条件は`effective_open()`
（`IntentStore`+`derive_any()`を経た**導出値**）——**本来同じ意味のはずの
2値が別関数・別解決経路で独立に計算されており、乖離しうる。**
`FocusChanged`が`last_intent`をクリアした直後、`effective_open()`は
観測ベースの`derive_any()`にフォールバックして`true`を返しうる一方、
`desired_open()`（生フィールド）は次の明示書き込みまで`false`のまま
残る窓があり、この窓で**drift correction（`VK_IME_OFF`）と
`ImmBrokenForceOn`（`VK_IME_ON`）が互いの存在に気づかないまま同じ実IMEへ
競合送信し続ける**。3件目の報告では`VK_IME_ON`92件・`VK_IME_OFF`32件
（いずれも`injected=true/self_injected=true`で確定、`ImeActuation`
journalには`VK_IME_OFF`側しか記録されず`VK_IME_ON`側はdrift correction
のActuation管理を経由しないことも確認済み）が競合しており、乖離継続
6分5秒という記録もこれで説明できる。詳細は`docs/known-bugs.md`
BUG-110追補3を参照。

これは`.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」
（issue #136/ADR-119）よりも一段深い、**合流点そのものが存在しない**
構造的欠陥——2つの独立した書き込み経路が共通の調停なしに同じ対象へ
書き込める。

**その後の3ラウンド討論で「不採用」判定・SSOTは3つ以上と訂正
（詳細は`docs/known-bugs.md` BUG-110追補4）。** 既存の`OpenWarrant`
授権機構（ADR-087/090の「A-2」フェーズ）を`apply_force_on_for_imm_broken`
にだけ強制適用する修正案をOpus 2体で検討したが、(1) `issue_open_warrant()`
は`desired_open`を直接読ませる仕組みではなく5段の優先順位付き梯子で、
最下段（Step 4c）に落ちた場合しか一致しない——特にStep 4bは
「force-onを発火させた同じguardが自分自身を無条件で授権する」自己整合
ループになっており適用がno-opになりうる、(2) `send_eager_tsf_warmup`
（`warmup_ime_on()` = `applied ?? belief`）という**第三のSSOT**が
`issue_actuation_order`を一切通らず存在し、実データ（report4のapp_log）
で実際に`VK_IME_ON`を送信していることを確認——**92件の内訳は#6単体
ではなくwarmup経路が相当量を占める可能性が高く**、A-2 on #6は問題の
一部にしか届かない、という2つのblockerにより不採用となった。
「2つのSSOTが競合」という上記の記述は「**少なくとも3つのSSOT
（`desired_open()`/`effective_open()`/`warmup_ime_on()`）が独立に
存在し、後者はwarrant機構の対象外**」に訂正する。

Phase 2再検討時は、本ADRが検討したdrift correction単体の問題
（A/B/B'/候補1-v2）に加えて、この三重SSOTと`apply_force_on_for_imm_broken`/
`send_eager_tsf_warmup`との調停不在を主要な論点に加える必要がある。
真に必要な修正は「起案点ごとのA-2適用」ではなく、warrantを通らない
送信経路をwarrant配下へ含める、ADR-090 §2.Aの11箇所棚卸しのやり直しで
ある可能性が高い——ただしこれは本ADRの対応範囲を超える、より大きな
仕事として次回以降に持ち越す。Phase 2（保留中の修正案の再検討）は
Phase 1のログ収集後に判断する。

**棚卸しのやり直しは完了した（詳細は`docs/known-bugs.md` BUG-110追補5）。**
「実際にWin32へIME関連の書き込みを行う全経路」をボトムアップで洗い出した
結果、`issue_actuation_order`を経由する11箇所（ADR-090の数え上げどおり、
誤りなし）に加え、**warrant非経由の経路が新たに4系統（B1〜B4）**見つかった。
最重要なのは**B1（`send_eager_tsf_warmup`、`warmup_ime_on() = applied ??
belief`という第3のSSOTを読む）**——本ADRで確認した競合の直接の当事者。
B2（`send_engine_state_ime_key`、engineの活性化遷移という第4の独立した
トリガー軸）は未検証だが同型のリスクを理論上残す。B3（panic_reset）は
意図的な例外として妥当、B4（BUG-25半角英数トグル）は相対的に低リスク。
実質的な書き込み経路は「(A) warrant経由11箇所 + (B) warrant非経由4系統
= 15系統」であり、ADR-090はそのうち11しかwarrant管理下に置いていな
かった、というのが最終結論。Phase 2の設計対象はB1を最優先に含める
必要がある。

## Phase 2（B1の修正、v5で追記）

### 位置づけ: 「修正」ではなくB1/#6の切り分けの一手

**本Phase 2はBUG-110を完全解決するものではない。** B1
（`send_eager_tsf_warmup`、`warmup_ime_on()`）× drift correction
の競合だけを構造的に消す。#6（`apply_force_on_for_imm_broken`、
`effective_open()`を読む）× drift correctionの競合はこのPhaseの
対象外で無傷のまま残る。3件目実機報告（`01M1NA7WYH1HCYAFWGA3F95AVY`）の
`VK_IME_ON` 92件のうちB1由来がどれだけを占めるかは journal の
`DumpTruncated`（total 2123 / emitted 1004）により未確定であり、
本Phaseの効果は「B1が主犯なら大きく改善、#6が主犯ならほぼ無風」という
幅を持つ。次の実機再現でこの内訳を確定できるよう、D2（下記）で
ログを整備した。

### 設計の経緯: v1（`desired_open`ゲート）→ blocker → v2（`off_drift_active`ゲート）

Opus 2体（architect/premortem）による追加討論2ラウンドを実施。

**v1（不採用）**: `resolve_warmup_ime_on`が`self.model().desired_open()`
（`ImeModel`の生beliefフィールド）とのAND条件でwarmupをゲートする案。
premortemのレビューで**blocker**と判定された: `ImeEvent::FocusChanged`
のreduceアーム（`ime_model.rs:611-640`）は`last_intent`/観測プール/
`applied`をクリアするが**`desired_open`はクリアしない**——本ADRが
`:536-538`で「`desired_open()`は次の明示書き込みまでfalseのまま残る
窓がある」とバグの原因として記述したその性質を、v1は修正の根拠として
使ってしまっていた。具体的な回帰シナリオ: Chrome（ImmCross）で
IMEを明示OFF（`desired_open=false`）→Windows Terminal（TsfNative、
GJIでIMEは実際にON）へフォーカス移動→`last_intent`/`applied`/観測は
クリアされるが`desired_open`はstaleなfalseのまま持ち越し→新フォーカスの
観測で`effective_open()=true`→v1のゲートは「BUG-110の競合窓」と
「正当なcross-window（別ウィンドウで実際にIMEが開いている）」を
区別できず後者まで抑止し、初回打鍵がリテラル化するBUG-02型の退行を
再導入する。

**v2（採用）**: ゲートを`desired_open`ではなく、**OFF方向の乖離
（desired≠observed）が今まさに検出中か**（`ImeStateHub::
check_drift_correction()`が`Some((false, true, _))`を返しているか）に
変更した（「進行中」ではなく「検出中」——`check_drift_correction`は
drift correction本体が実際に`VK_IME_OFF`を送信する条件の全てを
共有しているわけではない。詳細は下記「実装後レビューでの指摘と対応」
参照）。

```rust
// src/platform.rs（core、OS非依存）
impl WarmupImeOn {
    pub const fn from_applied_or_belief_unless_off_drift(
        applied: Option<bool>,
        belief_open: bool,
        off_drift_active: bool,
    ) -> Self {
        if off_drift_active {
            return Self(false);
        }
        Self::from_applied_or_belief(applied, belief_open)
    }
}

// state/platform_state.rs::resolve_warmup_ime_on
let off_drift_active = matches!(
    self.check_drift_correction(now, self.explicit_intent()),
    Some((false, true, _))
);
WarmupImeOn::from_applied_or_belief_unless_off_drift(
    applied.applied_open(), self.effective_open(), off_drift_active,
)
```

`check_drift_correction`はdrift correction自身が使う判定式そのもの
（同じ`ImeStateHub`のメソッド）であり、B1とdrift correctionが
**同じ判定式を共有する**——「別々の関数が別々の解決経路で独立に計算する」
というBUG-110の構造的欠陥をこの2者間では新たに作らない。

**不変条件 INV-B1'**: `send_eager_tsf_warmup`が`VK_IME_ON`を送信する
瞬間、OFF方向のdrift correctionは検出されていない。`off_drift_active
== false`のとき戻り値は`from_applied_or_belief`とbit-identical
（`architecture_guard`の呼び出し件数固定テストと組み合わせ、今日より
warmupが多く送られる経路が生まれないことを保証）。

`applied`の`Some`/`None`両枝にゲートが掛かる点が
`from_applied_or_belief`本来の単調性（appliedがSomeの間は同じ値を
返す）と異なるが、これは意図的: OFF方向drift検出中は`applied`自体が
drift correction（`Confirmed{open:false}`）とforce-ON
（`Confirmed{open:true}`）の両方に交互に書かれてping-pongしており、
「単調な事実」という前提そのものが崩れているため。

### 検討したが不採用の代替案

- **既存の`OpenWarrant`授権機構（`issue_open_warrant`）配下に含める**:
  B1のrequested=trueで梯子を辿るとStep 3（`derive_actuating`）で
  `ObservationSource::Gji`/`Tsf`のActuating権威によりStep 4c
  （`desired_open`を見る最下段）に届く前に`true`が発行されてしまい、
  BUG-110の競合窓でも授権されてしまう。warrant機構は「B1とdrift
  correctionの非矛盾」を原理的に保証できない。
- **IntentStoreベースのfocus-scopedゲート**（`IntentStore::
  lookup(current_focus, now)`、エントリ無しなら抑止しない）: 一見
  focus-scopedで有望だったが、3件目実機報告の実測値
  （`DriftGiveUpDiagnostic.drift_duration_ms: 24834`、
  `DriftGiveUpIntervalEnded.elapsed_ms: 365422`、`IntentStore`の
  OFF方向TTLは`tuning::EXPLICIT_OFF_INTENT_TTL_MS = 30_000`）と
  突き合わせると、24.8秒の区間はTTL内でヒットしうるが、**365秒の
  区間はその92%（30秒経過後）がTTL切れで`lookup`が必ずミスし、
  抑止できない**。最も実害の大きい区間を素通しするため不採用。

### 受け入れたトレードオフ（自己矛盾→一貫した誤りへの変換）

`check_drift_correction`の判定材料（`observations`のdrift追跡・
`most_recent_trusted`）は`ImeEvent::FocusChanged`のreduceアームが
呼ぶ`clear_on_focus_change`でフォーカス変更ごとに必ずクリアされる
ため、v2のゲートはfocus-scoped——**新しいフォーカス直後は必ず解除
され、cross-windowのcold-start warmup（BUG-02対策）を壊さない**。

ただし新フォーカス後、stale な`desired_open`（前の窓での明示意図が
持ち越されたもの）と新フォーカスの観測が食い違うと、
`DRIFT_CORRECTION_THRESHOLD_MS`（既存の`tuning.rs`定数、新規追加なし）
経過後にdriftが再確立し、以降その窓でのwarmupは（ユーザーが明示的に
IMEキーを押すまで）抑止され続ける。この状態では同時にdrift correction
が`VK_IME_OFF`を送っているため、**v2は「開けながら閉じる」という
自己矛盾を「（stale な desired が解消されるまで）閉じたまま・
warmupしない」という一貫した誤りへ変える**という設計判断であり、
「修正」ではなくトレードオフとして受け入れる。ユーザー体感は
「ON/OFFのちらつき」から「IMEが開かないまま」に変わる。
`crates/awase-windows/tests/warmup_gate_focus_scope.rs`の
`drift_re_establishes_in_new_window_when_stale_desired_conflicts_with_fresh_observation`
がこの挙動を機械可読な形で固定している——将来「cross-windowは
無条件に守られている」と誤読しないための回帰ガード。

なお同一プロセス内のhwnd切替（`ObservationStore::
update_focus_window`）はdrift/観測プールをクリアしない。BUG-110の
3件目報告では乖離が6分5秒継続した事実（=その間`FocusChanged`は
一度も発火していない、発火すればdriftが即座に消え32件の
`VK_IME_OFF`が説明できなくなる）から、当該報告の窓内warmupは
FocusChange経由ではなくexecutorのcomposition経路のみと推論でき、
実害にはならないと判断した。将来Chromeのタブ切替等で顕在化した
場合は再検討すること。

### v2が機能する前提（実データに基づく訂正）

`check_drift_correction`は矛盾観測が`ObservationSource::
ConvOpenInference`単独かつ`explicit_intent()==None`のとき`None`を
返す早期returnを持つ（BUG-19対策）。もし3件目報告の矛盾観測が
`ConvOpenInference`単独だったなら、この早期returnによりv2は
no-opになる——しかし同時にdrift correction自身も同じ早期return
で止まるため、それでは実測された32件の`VK_IME_OFF`が説明できない。
**したがって3件目報告の矛盾観測はActuating権威のソース
（`Gji`/`Tsf`/`ObserverPoll`）だったはずであり、v2はこの前提の
上で機能する。** 同じ観測がStep 3（`derive_actuating`）にヒットして
warrant案を破ったことと整合する。

### D2（観測の最小強化）

`output/mod.rs`の`[tsf-eager-warmup] VK_IME_ON 送信`ログを
`debug!`から`info!`へ格上げした。`apply_force_on_for_imm_broken`側
（`force-ON (ImmBrokenForceOn)`、既にinfo!）と合わせてgrep2本で
「92件の内訳」（B1由来 vs #6由来）が次回の実機報告で確定できる…
はずだったが、実装直後の敵対的コードレビューで、この前提が崩れて
いることが発覚した（下記「実装後レビューでの指摘と対応」参照）。
最終的には`send_eager_tsf_warmup`に`origin: &'static str`引数
（`"gated"`/`"actuated"`/`"off"`）を追加し、ログに`origin=`を
載せることで内訳を正確に分離できるようにした。

併せて`resolve_warmup_ime_on`にゲート発火時の`[warmup-gate]` info
ログを追加し、抑止の発火回数も追える——ただしこちらも初版には
バグがあり（後述）修正済み。journal化（`WarmupEmitted`エントリ、
`WarmupImeOn`への診断フィールド追加）は`WarmupImeOn`の型設計意図
（生beliefを渡す経路をコンパイラで塞ぐ）を薄める・`platform.rs`
5箇所への配線が本体（D1）より広い事故面積を持つ、との判断で見送った。

### D3（再発防止ガード）

`architecture_guard.rs::
warmup_ime_on_from_applied_or_belief_is_called_only_from_the_gated_constructor`
が、`WarmupImeOn::from_applied_or_belief`の本番コードからの実呼び出しを
ゲート版（`from_applied_or_belief_unless_off_drift`）内部の1箇所に
固定する。`from_applied_or_belief`はcoreクレートの`pub const fn`
であるため、走査対象に`crates/awase-windows/src/`だけでなく
core（`src/`）・`crates/awase-linux/src/`・`crates/awase-macos/src/`
も含める（PR #127のコードレビューで同種の見落としが実際に
起きた前例に倣う）。加えて`warmup_gate_third_arg_is_never_a_bare_literal_in_production_code`
（実装後レビュー指摘、下記参照）が、ゲート版自体への第3引数
（`off_drift_active`）にリテラル`true`/`false`を直書きする迂回
（呼び出し件数は1のまま保ったうえで実質的にゲートを無効化する
最も安直な手口）を検出する。

### 回帰テスト

- **T1**（`src/platform.rs`の`mod tests`、core・Linuxで実行）:
  `from_applied_or_belief_unless_off_drift`の12通り全数
  （applied 3値 × belief 2値 × off_drift_active 2値）を固定。
  `off_drift_active=false`の6通りがゲート無し版とbit-identicalで
  あること、`off_drift_active=true`の6通りがすべて抑止されること
  （INV-B1'）を検証する。
- **T2/T3**（`crates/awase-windows/tests/warmup_gate_focus_scope.rs`、
  新規・ungated統合テスト、Linuxで実行）: `resolve_warmup_ime_on`
  自体は`#[cfg(windows)]`でLinuxではコンパイルされないため、v2が
  依存する唯一の性質——`ObservationStore`のdrift追跡が
  `ImeEvent::FocusChanged`のreduceで確実にクリアされること——を
  `ImeModel`+`ObservationStore`（いずれもungated）だけで直接固定する。
  併せて`desired_open`がFocusChangedでクリアされずstaleに残ること
  （v1がblockerと判定された性質そのもの、将来の再提案へのブレーキ）と、
  受け入れたトレードオフ（新フォーカスでdriftが再確立されうること）も
  固定する。

### 実装後レビューでの指摘と対応（Opus敵対的コードレビュー）

実装完了後、`docs/adr/132-*.md`と実コード（コミット`448b1521`）を
別の読み取り専用Opusエージェント（design討論を行ったarchitect/
premortemとは独立）にレビューさせた。指摘のうち、対応したもの:

1. **INV-B1'は`WarmupImeOn`の全構築経路には及ばない**:
   `platform.rs::on_ime_applied`（実actuation直後、force-ONが
   `SetOpen(true)`を適用した直後にも通る）は`WarmupImeOn::
   from_actuated`を使い、`resolve_warmup_ime_on`のゲートを経由
   しない。ADR/doc/ガードテストのdocがこの経路の存在を無視して
   「送信の瞬間、drift は検出されていない」と無条件に主張していた
   のは不正確だった。**対応**: `from_actuated`経路をゲート下に
   含める設計変更（新たなOpus討論が必要な範囲）は見送り、
   INV-B1'の主張を「`from_applied_or_belief*`経由で構築された
   `WarmupImeOn`について」とスコープ限定する doc 訂正、および
   下記2の`origin`タグで区別可能にする対応に留めた。
2. **D2のログ内訳がB1由来へ過大計上される**: 1の結果、force-ON
   由来の随伴warmupも同じ`[tsf-eager-warmup] VK_IME_ON 送信`ログを
   出すため、grep突合せでの内訳確定が崩れていた。**対応**:
   `send_eager_tsf_warmup`に`origin: &'static str`引数を追加し
   （`platform.rs`の`dispatch_composition_response`/
   `feed_composition_event`/各wrapper関数、計9箇所に機械的に
   スレッディング）、`"gated"`（`resolve_warmup_ime_on`経由）/
   `"actuated"`（`on_ime_applied`の`from_actuated`経由）/`"off"`
   （構造的に到達しない`WarmupImeOn::off()`経路）を区別してログに
   載せた。`WarmupImeOn`型自体には触れず（型設計意図を保つ）、
   呼び出し元から並行して渡す形にした。
3. **`[warmup-gate]`ログの重複排除欠如・過大計上**: 打鍵駆動で
   頻繁に呼ばれる経路のため、乖離が長時間続くケース（3件目報告の
   6分5秒）でログが飽和しうる欠陥、および「ゲート無しでも元々
   送らない値だった」場合まで「抑止した」に数える不正確さが
   あった。**対応**: `intent_override_logged`と同じ`Cell<bool>`
   dedupパターンを導入（抑止の開始/終了の遷移時のみログ）、抑止の
   判定条件に「ゲート無しなら送っていたはずか」を加えた
   （`applied_open.unwrap_or(effective)`、`from_applied_or_belief`
   と等価だがガードテストの件数固定と衝突しないよう素の
   `Option::unwrap_or`で再現）。副次的に`effective_open()`の
   二重評価（TTL境界を跨ぐとログ値とゲート判定値が食い違いうる
   欠陥）も1回化で解消した。
4. **doc内の型名誤り**: `explicit_intent`引数の説明が
   `PlatformState::explicit_intent()`と誤記していた（正しくは
   `ImeStateHub::explicit_intent()`、`PlatformState`は別型）。
   **対応**: `[Self::explicit_intent]`への相対参照に訂正。
5. **`list_src_files()`と新設`list_rs_files_under("src")`の重複
   実装**。**対応**: 前者の本体を後者の呼び出しに置き換え。

指摘のうち、**対応せず既知の限界としてドキュメント化に留めたもの**
（実コードの変更ではなく、記述の正確化で対応——設計の再検討が
必要な範囲であり、これ以上の拡張は別途Opus討論を要するため）:

- **ゲート条件（`check_drift_correction`）とdrift correction本体
  （`ir_apply_drift_correction`）の実際の発火条件がずれている**:
  後者は前者に加えて`is_user_enabled()`/`is_japanese_ime()`/
  settle待ち/`FeedbackPolicy::Blind`のGiveUp状態という4条件を
  追加で持つ。特にBlindは「~400ms間隔で最大5回送信→GiveUp→
  3秒クールダウン」というデューティサイクルのため、**3秒の
  クールダウン中は`VK_IME_OFF`が1本も飛んでいないのにゲートは
  閉じたまま**になる区間がある。「B1とdrift correctionが同じ
  判定式を共有する」という本Phase 2の中心的主張は、`desired`と
  `observed`の矛盾を検出する式についてのみ厳密に成立し、
  「実際に対向の書き手が存在するか」までは保証しない。実害は
  限定的（抑止の方向自体は誤っていない、単に長めに抑止する）
  ため、次のいずれかで別途対応する: (a) ゲート条件に上記4条件を
  足して真に同じ判定式にする、(b) ADRの「進行中」という表現を
  「検出中」に訂正する（今回はこちらのみ実施）。
- **`now: Instant`のバッチ内不一致**: `executor.rs`の5箇所が
  独立に`std::time::Instant::now()`を呼ぶため、凍結された
  `applied_snapshot`とliveな`now`が同一batch内で混在し、理論上
  400ms境界を跨いで判定が反転しうる。設計討論で期待された
  「バッチ内一貫性も同時に得られる」という利点は実装に入って
  いない。実害は小さいと判断し（`resolve_warmup_ime_on`自体の
  ドキュメントに `now` は呼び出しごとの live 値でありバッチ内
  一貫性は保証しないことを追記した）、`DecisionExecutor`への
  `batch_now`フィールド導入は見送った。

## 次のアクション

1. Phase 2（B1修正）を実機ソークし、次の実機報告で`[tsf-eager-warmup]`
   /`[warmup-gate]`/`force-ON (ImmBrokenForceOn)`のgrep突合せから
   92件相当の内訳（B1由来 vs #6由来）を確定する。
2. 内訳確定後、#6（`apply_force_on_for_imm_broken`）側の同型修正
   （`check_drift_correction`との排他、または別の設計）を要否判断する。
   B1由来がほぼゼロだった場合、Phase 2は「効果薄だが無害な予防線」
   として維持しつつ、#6側の修正を優先する。
3. B2（`send_engine_state_ime_key`）は未検証のまま残っている
   （BUG-110追補5参照）。同型のリスクがあるか別途調査すること。
4. `.claude/rules/fix-requires-evidence.md`の「IME belief」
   「キー選択」両ファミリーに該当するため、Phase 2実装のコミットには
   本ADRの追記と`docs/known-bugs.md` BUG-110追補6を伴わせた
   （満たし済み）。
