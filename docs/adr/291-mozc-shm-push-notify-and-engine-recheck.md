---
id: ADR-291
title: |-
  belief_v2(候補Z)にmozc-shm追随の手放しを反映させ、Engineの活性再評価を明示的に呼ぶ(BUG-231)。
  mozc-shmのsettle機構が食い違う読みを誤って高速確認してしまう穴を塞ぐ(BUG-232)
summary: |-
  ADR-290/BUG-230適用後のdragonflyg4実機(2026-11-11)で、ひらがなキー押下からEngine活性(Active)まで
  最大約5.3秒ローマ字がリテラルで漏れる新しい遅延([BUG-231](../known-bugs/BUG-231.md))を観測した。
  当初「Engineの活性再評価が物理キーで走らない」と誤診したが、Opus敵対的レビューround1で訂正:
  実際には毎物理キーが`Engine::on_input`→`check_active_transition`を再実行していた。本当の原因は
  `general.belief_v2`(既定true、ADR-282)下でEngineが読む候補Z(`scope_shadow`)の変換表
  (`scope_belief.rs::scope_event_of`)が、`ir_do_abandon_half_width_alnum_from_mozc_shm`が書く
  `InputModeApplied{UserTurnOnEisuReset}`を`_ => None`として無視し、Zが英数(Eisu)の主張のまま
  取り残されること(反映の欠落、観測で約3.7秒。ログの`[belief-v2]`食い違い行が直接の証拠)。
  round2レビューで、最初の修正案(a)(mozc-shmの観測を素朴に`InputModeObserved{mode, MozcShm}`
  としてZへ書く)が**ADR-290自身が最初の実装で試して致命的と判定し却下した設計(D1′ F1: `Hiragana`
  を`ObservedKana`として書くとEngineが永久に`Inactive(NotRomajiInput)`のまま戻らない)をそのまま
  再現する**ことが発覚した。round3レビューでさらに、(a)をF1/F2回避済みの形に直してもなお、
  Zの`Explicit`claim由来の170ms柵により書き込み自体が黙って拒否されうる(Abandonは一発勝負で
  再試行が無いため、拒否されると無期限に取り残される)という、(a)固有の別の欠陥が見つかった。
  最終的に(b)(`InputModeApplyStrategy::UserHalfWidthAlnumToggle`、トグルエントリと対になる
  既存のreason)を単独で採用: `ir_do_abandon_half_width_alnum_from_mozc_shm`の既存の
  `eisu_reset_on_turn_on_while_open`ゲート付き呼び出しを、ゲート無しで`AssumedRomaji
  {UserHalfWidthAlnumToggleOff}`を無条件dispatchする形に差し替えるだけで、1回の書き込みが
  旧モデル(無条件reduce)とZ(柵のない`Explicit`)の両方に届く。新しい型・関数・guard変更は
  一切不要(既存の`eisu_recovery.rs`のdocが、まさにこの用途(トグルOFF自体の能動的遷移)を
  対応表の対象外として既に想定していたことも確認済み)。直後に`notify_engine_refresh()`を呼ぶ
  (D1′)。もう1つの原因(mozc-shmの2段ラッチが`TIMER_MOZC_SHM`500msポーリングのみに依存、
  検知遅延約1.6秒)については、round2で効果を「大半がMozc自身の切替時間でD2では縮まらない」と
  下方修正し、実測してから着手を判断する保留(D2)とした。D3(かなキー押下時のbelief即時楽観補正)は
  却下、D4(確定まで打鍵を保持)はADR-290自身の既存の見送り(D2依存)と同じ理由で見送り。

  2026-11-12、別の実機報告([BUG-232](../known-bugs/BUG-232.md)、ChromeでMozc shmの古い読みが
  明示IME OFF直後にbeliefを再びONへ上書きし、「mermaid」の最初の"m"がローマ字変換された)を調査し、
  本ADRのD1'とは別軸(open軸、`state::mozc_shm::decide_use`/`read_is_after_baseline`、ADR-268 D3の
  既存settle機構自体)に、独立した欠陥があると確定した。mozc-readableフォーク実機ソース
  (`tip_state_publisher.cc::ThreadSlot::Flush()`)を直接確認し、`update_tick_ms`がブロック全体の
  dirtyタイムスタンプ(`key_seq`等の高頻度フィールドが変わるだけで更新される)であり、`tsf_open`/
  `tsf_mode`自体の変化の証拠にはならないことを実証した(`read_is_after_baseline`のdocが前提にしていた
  「変化が無ければ書かない」が実機で成立しない)。**D5(採用)**: `decide_use`で、読みがawaseの
  belief(`effective_open`)と**食い違う**ときだけ`update_tick_ms`の高速路を使わず`settle_ms`の
  無条件経過を要求する(belief と一致する読みは従来どおり高速路を残す、副作用ゼロ)。
  Linux単体テスト(`crates/awase-ime-core/src/state/mozc_shm.rs`、実機ログの数値そのままの再現テスト
  込み)で修正前後の挙動差を固定済み、774件全テスト・windows targetのcargo check/clippy/fmtとも
  グリーン。比較検討した他案(OR条件を単純に外す案・`KEY_EFFECT_SETTLE_MS`を上げるだけの案・
  mozc-readable側に専用タイムスタンプを追加する案)はD5節で後述。GitHub Windows CI
  (`mozc-shm-e2e`ワークフロー、実機mozc-readable)での再現・確認は実施中/結果待ち。
status: |-
  設計確定(2026-11-11)。Opus敵対的レビューround1(原因診断の誤り、F1)・round2(修正案(a)がADR-290の
  既に却下済みの設計を再現する指摘、G1/G2)・round3((a)固有の別の欠陥〈170ms柵による書き込み
  拒否〉を指摘され(b)へ設計変更、H1〜H3)を経て、round4で「ゲート除去は他の2つのトグル抜け口
  (`key_pipeline.rs`・`ime_refresh.rs`のfocus-suspend)と揃えるだけで新しいリスクではない」と
  確認され収束(これ以上のレビューラウンドは不要と判定)。実装PRで直す3件のコメント同期
  (`half_width_alnum.rs`・`architecture_guard.rs`docstring・`mode_state.rs`)を残し、
  実装・実機確認とも未着手。

  D5(BUG-232)はコード実装済み(`crates/awase-ime-core/src/state/mozc_shm.rs::decide_use`、
  Linux単体テスト2件追加、既存52件含め全774件グリーン、windows target check/clippy/fmtグリーン)。
  GitHub Windows CI(実機mozc-readable、新規`tickchurn`シナリオ)での外部事実(update_tick_msが
  stateの変化無しに進む)の再現を2026-11-12に確認済み(run 43145173691、詳細は「未解決」6)。
  `tickchurn`シナリオは[PR #934](https://github.com/cuzic/awase/pull/934)として
  **developへマージ済み**(`6b29383d8`)。D5の修正コード自体も
  [PR #935](https://github.com/cuzic/awase/pull/935)として**developへマージ済み**
  (`97d849bfe`、2026-11-12)。本ADRドキュメントブランチ自体のdevelopへのマージは
  ADR-292のドキュメントPRと合わせて行う(2026-11-12追記)。

  **追記(2026-11-12)**: `decide_use`の食い違う読み用settle起点はADR-292 G3で
  さらに変更された(ローリング値 → IME開閉/モード送信だけに限定)。D5が変えた
  「一致する読みの高速路」自体は変わっていない。詳細は
  [ADR-292](292-key-effect-prediction-vs-observation-priority.md)。
related_adr:
  - "ADR-290"
  - "ADR-282"
  - "ADR-244"
  - "ADR-268"
  - "ADR-270"
  - "ADR-265"
  - "ADR-292"
  - "BUG-232"
---

# ADR-291: belief_v2(候補Z)にmozc-shm追随の手放しを反映させ、Engineの活性再評価を明示的に呼ぶ

## 背景

[ADR-290](290-half-width-alnum-exit-via-mode-key-and-gji-confirm.md)/[BUG-230](../known-bugs/BUG-230.md)で、
半角英数トグルをひらがなキー(F2)で抜けたときに`toggle_held`が解除されない問題を、
mozc-shm(mozc-readableの共有メモリ)の観測ベースの2段ラッチで修正し、developへマージ・
dragonflyg4へ投入した(PR #933、commit `e2f438871`)。

2026-11-11、dragonflyg4の実機ログで、この修正そのものが持つ新しい遅延を観測した
([BUG-231](../known-bugs/BUG-231.md)に詳細)。ユーザーからは「もっとmozc-readableからawaseに
即時のイベントを通知することはできないか、もしくはひらがなキー押下後のbeliefを適切にするとか」
という2方向の改善案が提案された。本ADRはログの実測とソース読みでこの2案(および気付いた3つ目の
独立原因)を検証し、採否を決める。

## 観測した事実(dragonflyg4、2026-11-11T05:47:16〜05:47:24Z、BUG-231より)

| 時刻(UTC) | 事象 |
|---|---|
| 05:47:16.733 | 左Shift単独タップ→半角英数トグルON |
| 05:47:16.869 | `Engine deactivated (reason=Inactive(NotRomajiInput))` |
| 05:47:18.911 | ひらがなキー(vk=0xF2)押下1回目、PassThrough |
| 05:47:20.544 | (+1.63秒) `[shift-conv-guard]` mozc-shmの観測でかなへの追随を確認→トグルを手放す。`apply_input_mode_correction`実行。**旧`ImeModel`はromaji-capableへ補正されるが、Zは英数のまま**(下記原因2) |
| 05:47:20.544〜24.195 | バックスペース×4・T×2・Space・Enter等、複数の物理キー押下。**いずれも`Engine::on_input`→`check_active_transition`を実際に再実行していた**(当初「再実行しない」と誤診、Opusレビューで訂正)。再チェックした先のZが英数のままだったので非活性が続いた |
| 05:47:24.195 | ひらがなキー押下2回目(ユーザーの再押下)。打鍵時予測が`scope_shadow_key`でZを直接kanaへ動かす |
| 05:47:24.231 | (手放しから+3.65秒) `Engine activated (reason=Active)`(直後に発行された`notify_engine_refresh()`で検知) |

1回目のかなキー押下(18.911)からEngine活性(24.231)まで**合計約5.32秒**。このうち約1.63秒が
mozc-shm観測の検知遅延(大半はMozc自身の切替時間)、約3.65秒がbelief_v2(Z)への反映の欠落という、
独立した2つの原因で構成される。

## 検討した案

### 案A′(採用・D1′): 手放し処理でZが受理する形でモード軸を書き、かつ`notify_engine_refresh()`を呼ぶ

当初「`ir_do_abandon_half_width_alnum_from_mozc_shm`が`notify_engine_refresh()`を呼んでいない
から、次の物理キー/フォーカス変更までEngineに伝わらない」と考えた(案A)。これはOpus敵対的
レビューで**factとして誤り**と指摘された: `key_pipeline.rs`の特殊キー以外は全て
`Engine::on_input`を通り、`on_input_body`のPhase2が非活性の早期returnより前に
`check_active_transition(ctx)`を呼ぶ。実際、BUG-231のBS×4・T×2・Space・Enterはすべて
`check_active_transition`を再実行していた。

**本当の原因**: `general.belief_v2`(既定`true`、[ADR-282](282-per-scope-latched-ime-belief-zero-base.md)
V2-3、所有者決定O8)が真の間、Engineが読むbeliefは`platform_state.rs::engine_belief()`経由で
候補Z(`scope_shadow`)。`crates/awase-ime-core/src/state/scope_belief.rs::scope_event_of`は
`ImeEvent::InputModeApplied`のうち`strategy`が`UserHalfWidthAlnumToggle`・`PanicReset`のものだけ
を`ScopeEvent::Explicit`としてZへ写し、**それ以外(`UserTurnOnEisuReset`を含む)は`_ => None`で
無視する**(ADR-268がmozc-shmの open軸だけをZへ接続した設計の裏返しで、mode軸はこれまで
Zへ届く経路が無かった)。`ir_do_abandon_half_width_alnum_from_mozc_shm`が書く
`InputModeApplied{UserTurnOnEisuReset}`はこの穴に落ち、旧`ImeModel`はromaji-capableへ補正
されるのに、Engineが実際に読むZは英数のまま取り残される。2回目のかなキー押下の打鍵時予測
(`scope_shadow_key`)がZを直接動かすまで、Engineの再評価は毎回「正しく再実行されるが、古い
入力を読んでいるので結果が変わらない」状態が続いた。

**対比(なぜADR-244 D3は同じ穴に落ちないか)**: `ir_follow_direct_mode_key_read`
(ADR-244 D3、直接観測による追随)は`InputModeObserved`を書く。これは`scope_event_of`で
`ScopeEvent::Observed`に写り、Zが受理する。mozc-shm経由の手放しだけが`InputModeApplied`
(Zの変換表の対象外の`strategy`)を使っていたため、この穴に入った。

**同じ穴を持つ既知の箇所(今回のADRでは修正対象にしないが記録する)**: [BUG-222](../known-bugs/BUG-222.md)の
`kp_exit_lang_detect_toggle_on_hiragana`(`key_pipeline.rs`)も`UserTurnOnEisuReset`を書くため、
belief_v2が真の環境では同じ理由でZに伝わらない可能性が高い(未検証、別途確認が必要)。
一般化すると、**`{UserHalfWidthAlnumToggle, PanicReset}`以外の`InputModeApplyStrategy`はすべて
Z配下でEngineに無視される**。これはADR-282自身の設計上の既知の穴であり、本ADRの範囲を超える
ため、ADR-282の追補課題として別途記録することを推奨する(本ADRでは直接の修正対象(手放し経路)
のみ扱う)。

**修正の方向、round2レビューでさらに訂正(重要: 単純に`InputModeObserved{mode, MozcShm}`を
書くだけでは、ADR-290が既に試して致命的と判定した設計をそのまま再現する)**:

`architecture_guard.rs::input_mode_observed_construction_sites_are_accounted_for`のコメントに
「ADR-290(BUG-230)はこの軸にInputModeObservedを構築しない設計(D1″、Opusレビュー後)に
落ち着いた」と明記されているとおり、**ADR-290自身が最初の実装(D1′、「第1版」)でまさに
`InputModeObserved{source: MozcShm}`をbeliefへdispatchする設計を試し、Opus敵対的レビューで
2件の欠陥により却下している**(`docs/adr/290-*.md`「却下した案」節):

- **ADR-290 D1′ F1(致命的)**: Mozcの`CompositionMode::Hiragana`を、素朴に
  `InputModeState::ObservedKana`(「JISかな直接入力と確認できた」、`is_romaji_capable()`が
  `false`、ADR-091決定2でサポート対象外)として書いていた。`Hiragana`はローマ字テーブル入力
  (NICOLAの前提)とJISかな入力を区別できないため、手放しに成功するたびにbeliefが
  `ObservedKana`になり`engine.rs::compute_state`が`Inactive(NotRomajiInput)`を返し続け、
  **BUG-230を直すはずの変更がむしろ恒久的に悪化させていた**。
- **ADR-290 D1′ F2(高)**: この観測が`report_mozc_shm`内の`decide_use`のHeld/Split判定を
  素通りしていたため、トグルに入った直後、共有メモリにまだ残っているエントリ**前**の古い
  `hiragana`値を「かなへの追随」と誤認し、入った直後にトグルを即座に手放す経路があった。

**本ADRが提案する書き込みがこの2件に当たらない理由(必ず明記すること)**:
- F1対策: モードの値を`ObservedKana`では**なく**、[ADR-244](244-bug186-msime-chrome-persistent-toggle-follow.md)
  D3の前例(`follow_direct_read_in_scope`、`platform_state.rs:904-918`、非英数側で
  `InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken }`を書く)と同じ形で、
  **`InputModeState::AssumedRomaji { reason: AssumedReason::UserHalfWidthAlnumToggleOff }`
  (既存のreason、`src/engine/mode_state.rs:37`)に固定する**。これはromaji-capable(`is_romaji_capable()`
  が`true`)なので、Zの`mode_class_of`は(命名が直感に反するので要注意、下記「命名の罠」参照)
  `ModeClass::Kana`になり、Engineが`Inactive(NotRomajiInput)`に落ち込まない。「Mozcが実際に
  かな入力方式(ローマ字でなくJISかな)だったらどうするか」は、v5共有メモリブロックの`prefer_kana_input`フィールド
  (`crates/awase-ime-core/src/state/mozc_shm.rs:197,209`)を読めば区別できるが、現在
  `mode_read`は`(CompositionMode, update_tick_ms)`しか運んでいない(`mozc_shm_shadow.rs:89-90,160`)
  ため、安い拡張でなければ今回は見送り、常に保守的な`AssumedRomaji`を書く。
- F2対策: 本ADRの書き込みは`report_mozc_shm`の毎tickの`decide_use`には一切相乗りせず、
  既に実装済みの2段ラッチ(`plan_mozc_shm_follow`が`Abandon`を返した、そのtickのみ)でしか
  発火しない。したがってエントリ前の古い`hiragana`値を誤認するF2の経路は生じない。

上記のF1/F2対策により、`InputModeObserved{mode: AssumedRomaji{...}, source: MozcShm}`
(以下(a))自体はADR-290 D1′と同じ欠陥を再現しない形にできる。しかし、round3レビューで
(a)には別の、より深刻な欠陥があることが判明した(下記H1)。最終的な決定は(a)ではなく、
より単純な(b)(`InputModeApplyStrategy::UserHalfWidthAlnumToggle`)である。

**G2(round2): (a)が`InputModeApplied`書き込みを置き換えると`belief_v2=false`環境を壊す**:
現在の`InputModeApplied{UserTurnOnEisuReset}`は旧`ImeModel::reduce`が**無条件**に適用する
(`crates/awase-ime-core/src/state/ime_model.rs:1014-1019`)。これを`InputModeObserved`で
**置き換える**と、旧モデルは`reconcile_key_effect_mode`(ADR-191決定3の柵、
`KEY_EFFECT_SETTLE_MS = 170ms`、`tuning.rs:479`)を通るため、直近170ms以内に打鍵の予測が
あると観測が**拒否される**。

**H1(round3、重要な訂正): (a)自身にも、より悪い性質の「柵」が隠れている**。
round2までは「(a)はZに対して柵を持たず、(b)はZに対して170msの柵を持つから(a)を推奨」と
書いていたが、これは**事実と逆**だった。Zの`on_observed`は`accepts(row.mode, confidence,
read_started_ms, …)`(`scope_belief.rs:533-545`)を通ってからでないと書き込まず、`accepts`は
`read_started_ms < c.fence_ms`なら**拒否(false)を返す**(`:788-790`)。直前に**`Explicit`な
モードの主張**(トグルエントリの`InputModeApplied{UserHalfWidthAlnumToggle}`、または
**モードを持つ打鍵時予測**(`scope_event_of_prediction`が`prediction.effect.mode`を
`ScopeEvent::Explicit`に写す、`scope_belief.rs:~651-653`、`scope_shadow_key`経由、
`platform_state.rs:410-418`))があると、その`fence_ms = at_ms + settle_ms`(170ms)の間、
**(a)の`InputModeObserved`書き込みは黙って拒否される**。一方(b)が使う`on_explicit`
(`:353-388`)には柵が一切無く、常に無条件で上書きする(`:369-387`、確認済み)。

**この取り逃しが(b)の170ms柵より深刻な理由**: 手放し(Abandon)はedge-triggeredの一発勝負で、
`abandon_on_observed_follow()`がbeliefの書き込み**より前**にトグル状態自体を解除し
(`ime_refresh.rs:150-157`)、以後`ir_abandon_half_width_alnum_from_mozc_shm_if`は早期returnし
続ける(`:112-114`)——**書き込みの再試行が無い**。つまり、手放しの直前170ms以内にモードを
持つ打鍵時予測が1つでも入ると、(a)の書き込みは黙って失敗し、Zは英数のまま、次にモードを
持つ打鍵時予測かZ向けの観測が来るまで**上限なく**取り残される——BUG-231の症状そのものが
形を変えて再発する。BUG-231自身のタイムラインはこれに当たらなかった(2回目のF2予測が
`mode=None`だったため)が、これは潜在的な別の再発経路であり、ADR-290 R2-1と同じ「エッジケースの
見落とし」族に属する。

**結論: (b)(`InputModeApplyStrategy::UserHalfWidthAlnumToggle`)を単独で採用する**。
理由:
1. (b)の170ms柵は「Abandon直後の約170msだけ、mozc-shmからの食い違う観測を無視する」という
   負け筋だが、この経路でZのモード軸にmozc-shm以外の観測を供給する手段は無い(現状、
   mozc-shmはopen軸しかZへ届いていない、ADR-268)ため、実害がほぼゼロの柵。(a)の柵は
   「書き込み自体が時々黙って失敗し、再試行もない」という上限の無い失敗であり、比較にならない。
2. `on_explicit`は`confidence`を取らない(`scope_event_of`の`InputModeApplied`写経路に
   confidenceフィールドは無い)ため、(a)で論点になっていた「High/Medium信頼度の選択」自体が
   消える。
3. (b)を選ぶと**新しい`InputModeObserved`構築箇所が1つも増えない**——`architecture_guard.rs`の
   構築数固定テスト・そのコメント・`mozc_shm_reader_touches_belief_only_through_report_mozc_shm`
   のdocコメント・ADR-268本文のいずれも**変更不要**になる(G3で挙げた更新作業がすべて消える)。
4. `crates/awase-ime-core/src/state/eisu_recovery.rs`のモジュールdocが、まさにこの用途を
   想定した記述を既に持っている:「`InputModeApplyStrategy::UserHalfWidthAlnumToggle`は
   [IME-ON経路×ObservedEisu救済の]対応表の対象外」節(`:63-71`)に、「トグルOFF自体
   (`ObservedEisu → AssumedRomaji`)は救済ではなくawase自身の能動的な意図的遷移であり、
   `user_ime_on_paths_are_paired_with_eisu_reset`の監視対象にも含めない」と明記されている。
   mozc-shm経由の手放しも、SetOpenを一切発行しない・IME-ON遷移を伴わない・ObservedEisuから
   AssumedRomajiへのトグルOFF自体の能動的遷移という**全く同じ形**であり、このdocの想定に
   そのまま収まる(現状は`UserTurnOnEisuReset`+`eisu_reset_on_turn_on_while_open`という、
   本来別の目的(「IMEが開いたままturn-onする」救済専用、対応表の別の行)の関数を間借りして
   いたほうが、むしろこのdocの整理と噛み合っていなかった)。round4で確認: この節の文面は
   `kp_stage_shift_conv_guard`(key_pipeline.rs側のトグルエントリ)だけを念頭に書かれており、
   focus-suspend抜け口(`ime_refresh.rs:715-723`)も明示されていない(既存の記述漏れ)。
   ガード自体には影響しないため必須ではないが、実装PRで「同じ理屈が`ime_refresh.rs`の
   focus-suspend抜け口と、本ADRのmozc-shm手放し抜け口にも当たる」の1行を追加するとよい。
5. `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`の
   `UserTurnOnEisuReset`関連のチェック(`:918-919`)は`read_crate_file("src/runtime/
   key_pipeline.rs")`の内容のみを走査し、`ime_refresh.rs`は対象外であることを確認済み——
   本ADRの変更対象(`ime_refresh.rs`の呼び出し)はこのガードに引っかからない。

**設計(確定)**: `ir_do_abandon_half_width_alnum_from_mozc_shm`の既存の
```
if let Some(new_mode) = eisu_reset_on_turn_on_while_open(true, self.platform_state.ime.input_mode()) {
    self.apply_input_mode_correction(new_mode, InputModeApplyStrategy::UserTurnOnEisuReset, tick_ms);
}
```
を、
```
self.apply_input_mode_correction(
    InputModeState::AssumedRomaji { reason: AssumedReason::UserHalfWidthAlnumToggleOff },
    InputModeApplyStrategy::UserHalfWidthAlnumToggle,
    tick_ms,
);
```
に置き換える(`eisu_reset_on_turn_on_while_open`のゲートを外し、Abandon自体を条件に
**無条件**で書く)。新しい型・新しい呼び出し口・新しい`ImeEvent`/`EngineCommand` variantは
一切不要——既存の`apply_input_mode_correction`(`InputModeApplied`をdispatchするだけの
既存ヘルパー)への引数を変えるだけで、1回のdispatchが旧モデル(無条件reduce)とZ
(`scope_event_of`が`UserHalfWidthAlnumToggle`を`ScopeEvent::Explicit`として受理)の
**両方**に届く。

**ゲートを外すことは安全(round4で確認): 他の2つのトグル抜け口と揃えるだけ**: 半角英数トグルの
抜け口は実は3箇所あり、残り2箇所は**すでにゲート無しで無条件に**`AssumedRomaji
{UserHalfWidthAlnumToggleOff}`+`UserHalfWidthAlnumToggle`を書いている
——`kp_restore_kana_from_half_width`(`key_pipeline.rs:3117-3123`)と、フォーカス変更時の
強制抜け(`ime_refresh.rs:715-723`)。ゲート付きだったmozc-shm経由の手放しが唯一の例外
(BUG-222の`UserTurnOnEisuReset`の形を間借りしていた名残)であり、ゲートを外すことは
**新しい挙動の導入ではなく、3箇所を同じ形に揃える**ことに相当する。どちらのreasonの値
(`AppKindExcluded`→`UserHalfWidthAlnumToggleOff`)にも分岐するロジックは存在しない
(`grep`で`AppKindExcluded`への`matches!`/`==`が本番コードに0件、確認済み)。strategyの値も
旧`ImeModel::reduce`は見ない(無条件適用、`ime_model.rs:1014-1019`)——strategyに反応するのは
Zの`scope_event_of`だけで、それこそが本ADRの変更点。

**ゲートが守っていた唯一の縁(既に他の2箇所も受け入れている、新しいクラスのリスクではない)**:
トグル中に本当に`ObservedKana`(JISかな直接入力、ADR-091非対応)へ観測/予測が動いていた場合、
無条件化によってそれをromaji-capableへ上書きし、JISかな入力のIMEに対してEngineを活性化
してしまう。この状態に至るには、トグルエントリが`ObservedEisu`を書いた**後**に、JISかな入力
モードの観測/予測が別途入る必要があり、shift-conv-guardが有効な間はObserverの書き込み自体が
抑止されている(`ReadReason::ShiftConvGuard`、`ir_decide_read_strategy`)ため、相当まれ。
かつ、`kp_restore_kana_from_half_width`と focus-suspend の既存2箇所は既にこのリスクを
無条件で受け入れている——mozc-shm経由の手放しだけがこれを気にするのは非対称。将来の精密化
(v5の`prefer_kana_input`フィールドを読んで判別する、上記のF1対策の節で触れた拡張)は
3箇所共通の改善として別途検討する。

**`notify_engine_refresh()`も必要(変わらず)**: `ir_execute`の実行順は`ir_stage_notify()`
(`ir_notify_engine_refresh`を呼ぶ)→`mozc_shm_on_tick`→`ir_abandon_half_width_alnum_from_mozc_shm_if`
であり、**同一tick内で手放しがEngine通知より後に起きる**ため、通常のリフレッシュtick経由でも
手放しの直後にEngineは再評価されない(次のtickか次のキーまで待つ)。さらに`TIMER_MOZC_SHM`
専用タイマー経由(ADR-268 D5、`explicit_intent`でリフレッシュtickが止まっている間の経路)では
`ir_stage_notify`自体が一切呼ばれないため、`notify_engine_refresh()`を明示的に呼ばない限り
Engine通知が永久に発生しない。したがって、上記の書き込みと`notify_engine_refresh()`の
**両方が必要**(片方だけでは直らない)。呼び出し順序: 既存の`abandon_on_observed_follow()`→
`confirm_gate_deadline_override_ms.set(0)`→`bump_shift_conv_guard_gen()`→ログ→上記の書き込み
→`notify_engine_refresh()`(belief書き込みの後に通知、の順を変えない)。

**命名の罠(必ず正確に引用すること)**: `scope_belief.rs::mode_class_of`の実際のマッピングは
`AssumedRomaji`/`ObservedRomaji` → `ModeClass::Kana`、`ObservedKana` → `ModeClass::KanaInputMethod`
(`:587-596`)。**`ModeClass::Kana`がromaji-capable(NICOLA前提)を意味し、
`ModeClass::KanaInputMethod`がJISかな直接入力(非romaji-capable)を意味する、という
直感に反する命名**であり、"RomajiInputMethod"のような変種名は実在しない。実装時に
「正しい名前に直そう」として`KanaInputMethod`側に書き換えると、ADR-290 D1′ F1の
致命的回帰をそのまま再現する。

### 案B(保留・D2計測が先): mozc-readableのPulseイベントを待ち受けて即時に読み直す

`~/mozc-readable-release/repo/src/win32/tip/tip_state_publisher.cc`は、共有メモリへの書き込み
(`Flush()`)ごとに`Pulse()`(named event `Local\mozc-readable-ime-changed`、`CreateEventW`で
session-local・sharable security descriptor付きで作成、`SetEvent`→`ResetEvent`)を呼んでいる。
awase側の`crates/awase-windows/src/observer/mozc_shm.rs`はこのイベントを一切使わず、
`TIMER_MOZC_SHM`(`MOZC_SHM_TIMER_MS = 500ms`)の定期ポーリングのみで共有メモリを読んでいる
(`grep -n "ChangeEvent|WaitForSingleObject|OpenEventW|CreateEventW"`が0件)。

**実装方針**: `crates/win32-worker`の`WorkerThread`/`ShutdownToken`(常駐ワーカースレッド+
シャットダウン用named eventのRAII、既存)を土台に、`OpenEventW(SYNCHRONIZE, FALSE,
"Local\\mozc-readable-ime-changed")`で取得したハンドルと既存のshutdownイベントの両方を
`WaitForMultipleObjects`で待つ常駐スレッドを1本追加する。変化イベントが発火したら、
メインスレッドへカスタムウィンドウメッセージ(`PostMessageW`)を送り、ハンドラで既存の
`mozc_shm_on_timer`相当の読み直し(`mozc_shm_read_once`→`ir_abandon_half_width_alnum_from_mozc_shm_if`)
を1回走らせる。`TIMER_MOZC_SHM`の定期ポーリングは**そのまま残す**(下記のPulseの既知の
レース、およびmozc-readable未導入環境でのフォールバックのため)。

**Pulseパターンの既知のレース、および想定より効果が小さい可能性**:
1. `Pulse()`は手動リセットイベントに対する`SetEvent`直後の`ResetEvent`であり、これはWin32の
   `PulseEvent()`API自体が持つのと同型のレースを内包する。BUG-231のF2押下では、`key_seq`更新
   (打鍵自体)の書き込みとモード変化の書き込みが短時間に連続しうるため、ワーカースレッドが
   1回目のPulseで起きて`PostMessageW`し、`WaitForMultipleObjects`に戻る前に2回目のPulseが
   素通りする取り逃しが、まさに狙いたい場面(モード変化の検知)で起きうる。
2. **ポーリングが足せる遅延は最大500ms(平均約250ms)のみ**。BUG-231の1.63秒のうち、
   少なくとも約1.13秒はMozc自身の実際の切替時間であり([BUG-230](../known-bugs/BUG-230.md)の
   実測、F2押下から`hiragana`表示まで約2秒、と整合)、D2はこれを一切縮められない。つまりD2が
   実際に削れる幅は「ポーリング由来の部分」だけで、見積もりは「約1.6秒の大部分短縮」ではなく
   **「最大500ms程度」**に下方修正する必要がある。
3. 起動順序(awaseはログオン時に起動し、mozc-readableのTIPがまだ`CreateEventW`していない
   可能性がある)、AppContainer化されたTIPホスト(`Local\`が`BaseNamedObjects`ではなく
   AppContainer専用名前空間に解決され、Pulseが届かない)、取り逃し対策の再読み直し猶予
   (新しいtuning定数が必要)など、設計の詳細が未決定のまま残る。

**mozc-readable未導入環境への影響なし**(この点は変わらず): `OpenEventW`はmozc-readableのTIP
プロセスが実際に走っていて初めて名前付きイベントが存在するため、未導入環境では単に開けず、
既存のポーリングのみで従来どおり動作する。

**保留の理由**: 上記2により、D2の実装コスト(`win32-worker`への`WaitForMultipleObjects`対応
追加、常駐スレッド、カスタムウィンドウメッセージ、DACL/AppContainerの実機確認、取り逃し対策)
に対して得られる実益が「最大500ms」に留まる可能性が高く、`.claude/rules/complexity-budget.md`
の精神(まだ強制発効はしていないが)に照らしても投資に値するか怪しい。**着手前に、
mozc-shmのブロックが持つ`update_tick_ms`(Mozc自身の`GetTickCount64`)と手放し検知時の
`now_ms`の差を1行ログに出し(既に両方の値は`ir_do_abandon_half_width_alnum_from_mozc_shm`
到達時点で手元にある)、実際のポーリング由来の遅延がどの程度かを複数回の実機観測で測ってから
着手を判断する**。小さければ、`TIMER_MOZC_SHM`間隔を半角英数トグルが有効な間だけ短縮する
(実測を伴う`tuning.rs`変更、`.claude/rules/tuning-constants.md`準拠)方が遥かに安い代案になる。

### 案C(却下、ただし理由を訂正): かなキー押下時にbeliefを即座に楽観補正する(ユーザー提案の2つ目)

ひらがなキー押下を検知した時点で、Mozcの実際の切替完了を待たずにbelief(`input_mode`・Engine活性)
を即座に書き換える案。却下は維持するが、**理由をBUG-230との比較だけで済ませるのは不正確**:
現状でも、Mozcの実際の切替が完了する前のあの数秒間は、Engineが非活性なので打鍵がそのまま
qwertyローマ字でOSへ素通りしている(BUG-230タイムライン項目4「F押下→変換されず」がまさにこれ)。
つまりD3はこの期間の出力を「ローマ字のまま漏れる」から「NICOLA変換されて半角英数Mozcへ送られる」
に変えるだけで、どちらも誤った出力である点は変わらない。**却下すべき本当の理由は、確認なしに
確定させたbeliefが、確認が来なかった場合(取りこぼされたキー、Mozcが実際には切り替わらない、
共有メモリが読めない窓)に、Engine-activeのまま半角英数Mozcへの誤送信が有限の数秒ではなく
無期限に続く、訂正不能な状態を作ることにある**(BUG-192/ADR-290 round1 F1/F2が指摘したのと
同じ種類のリスク)。

### 案D(新規・今回は見送り): 確定するまで打鍵を保持する

もう1つの選択肢: ひらがなキー押下を検知してから、mozc-shmの手放し確認が出るまで、以降の打鍵を
(既存の`INPUT_DEFER.replay_later`・`focus_switch_hold`・`resume_hold`・ADR-263のepisode hold
と同系統の機構で)保持し、確認が出たら保持列をEngine活性状態で再生する。タイムアウトで
素通りへフォールバックする安全弁付き。これはD1′/D2と違い、**確認が出るまでの区間自体の
誤出力(現状のローマ字漏れ)も無くせる唯一の案**。

**ADR-290自身が同じ案を既に検討・却下している**(`docs/adr/290-*.md`却下した案「半角英数トグルを
抜けるまでNICOLAの送信自体を保留する(holdキュー)」): 実装コストが高く、ADR-290のD2
(GJI脱出直後のMozc確認・回収への相乗り)で同じ結果(最終的に正しい文字が残る)が得られる見込み、
という理由で見送られている。**この見送りはADR-290 D2の実装を前提にしており、D2は現在も
未着手のまま残る**([未解決]参照)。したがって本ADRのD4見送りと、ADR-290の過去の見送りは
**同じ未実装の前提(ADR-290 D2)に依存している**——D4を今回も見送るなら、D2が未着手である限り
「確認までの区間の誤出力」自体への対策は依然どこにも存在しないことを、両ADRのどちらかが
明記しておく必要がある。本ADRでは、D1′(belief_v2の穴)を先に塞ぐ方が優先度が高く、保持機構の
追加はタイムアウト定数の実測や既存の保持機構との競合確認などスコープが大きいため見送るが、
`.claude/rules/fix-requires-evidence.md`のdefer/replay系ファミリーの延長として今後の選択肢に残す。

### 案E(採用・D5、2026-11-12追記): BUG-232 — open軸のmozc-shm settle機構自体の穴を塞ぐ

[BUG-232](../known-bugs/BUG-232.md)(dragonflyg4、2026-11-12T01:40:37〜39Z、Chrome×GJI)で、
D1′とは別軸(mode軸ではなくopen軸)の独立した欠陥を確認した。明示的にIME OFFキーを押した112ms後、
`[mozc-shm] follow external change open=true`→`effective-open-flip → true decided_by=
DeriveHigh(MozcShm)`が発火し、たった今自分で設定した明示OFFを古いmozc-shmの読みが上書きした
(打鍵1個が誤ってローマ字変換された)。

**原因の確定(mozc-readableフォーク実機ソースで直接確認)**: `state::mozc_shm::decide_use`/
`read_is_after_baseline`(ADR-268 D3 R3'、本ファイル`crates/awase-ime-core/src/state/mozc_shm.rs`)は
「`update_tick_ms > baseline_ms`なら、`tsf_open`/`tsf_mode`自体がawaseの操作より後に変化したことが
確実」という前提で、settle_ms(170ms)未満でも高速に読みを信用する設計だった。しかし
`~/mozc-readable-release/repo/src/win32/tip/tip_state_publisher.cc`の`ThreadSlot::Flush()`
(`Snapshot::operator==`/`SameState()`)を直接確認したところ、`update_tick_ms`は**ブロック全体の
dirtyタイムスタンプ**であり、`key_seq`・`preedit_cursor`・`composition_key_seq`等の高頻度フィールド
(ほぼ毎打鍵で変わる)のどれかが変わるだけで更新される。`pulse`(実際に state が変わったかの内部信号、
`!last_.SameState(pending_)`)はこれとは別に正しく判定されているが、別のタイムスタンプとして
外部(共有メモリ)に公開されていない。つまり`tsf_open`が全く変わっていなくても、同じ窓で別の打鍵が
あれば`update_tick_ms`だけ新しくなる——`read_is_after_baseline`の高速路はこの「見た目だけ新しい」
読みと「本当にopen/modeが変わった」読みを区別できない。BUG-232の実機再現はこれ(「M」キー自体が
Mozc側の`key_seq`を進め、`update_tick_ms`を再スタンプした)と整合する。

検討した案:

- **案E1(採用): `decide_use`で、belief(`effective_open`)と食い違う読みだけ高速路を外す**。
  読みが`effective_open`と**一致**する(awase自身の操作の確認)ときは高速路をそのまま残す
  (危険が無く、ADR-268 D3が意図した「速い確認」の利点を失わない)。**食い違う**ときは
  `update_tick_ms`を一切信用せず、`settle_ms`の無条件経過のみを要求する(既存のN2「書き込みロスト」
  救済と同じ扱いに統一する——書き込みが本当に失われた場面と、見た目だけ新しい食い違う読みの場面を、
  awase側からは区別する手段が無い以上、両方を「settle経過まで待つ」に揃えるのが安全側)。
  新しい型・パラメータは不要、既存の`effective_open`引数だけで判定できる。
- **案E2(却下、効果が無いことを論証で確認): `KEY_EFFECT_SETTLE_MS`を上げるだけ**。
  `update_tick_ms > baseline_ms`の高速路は`settle_ms`の値を一切参照しない(OR条件の左辺が真なら
  右辺の値に関係なく成立する)ため、settle_msをどれだけ上げても、Mozcが打鍵のたびに
  `update_tick_ms`を再スタンプする限り高速路は即座に(baseline+1ms程度で)成立し続ける。
  `.claude/rules/tuning-constants.md`が求める実測をするまでもなく、ブール論理だけで効果ゼロと
  確定できる。
- **案E3(却下、別の実害を再現する): `update_tick_ms > baseline_ms`の高速路を単純に削除**
  (常に`settle_ms`経過のみを要求する)。BUG-232は直るが、「awase自身の操作をMozcが実際に速く
  反映した」場面でも一律`settle_ms`(170ms)を待つことになり、ADR-268 D3が獲得した高速確認の
  利点を全て失う(E1はこの損失が無い)。
- **案E4(見送り、本ADRの範囲外・別リポジトリ側の改善として記録): mozc-readable側にstate専用の
  タイムスタンプを追加する**。`ThreadSlot::Flush()`の`if (!last_.SameState(pending_))`の分岐内
  でのみ更新する`state_update_tick_ms`相当のフィールドを共有メモリブロックに追加すれば、awase側は
  本来の意図どおり「状態が変わった時刻」を直接読める、最も筋の良い恒久対策。しかし
  (a) mozc-readable(`cuzic/mozc-readable`)側のブロック形式変更・バージョンアップ・MSI再ビルドが
  必要で本ADR(このリポジトリ)の範囲外、(b) E1だけで実害(BUG-232)は解消済みのため緊急性が無い。
  将来mozc-readable側で version bump する機会があれば検討する、として記録のみ残す。

**比較のまとめ**:

| 案 | BUG-232を直すか | 高速確認の利点を保つか | 実装コスト |
|---|---|---|---|
| E1(採用) | 直る(Linux単体テストで確認) | 保つ(belief一致時は従来どおり) | 既存関数内の分岐追加のみ |
| E2 | 直らない(論理的に無効) | — | tuning値変更のみだが無意味 |
| E3 | 直る | 失う(常にsettle待ち) | 既存の高速路を削除 |
| E4 | 直る(本来最も正確) | 保つ | 他リポジトリの変更が前提、未着手 |

## 決定

- **D1′(採用、round3で(a)→(b)に設計を訂正)**: `ir_do_abandon_half_width_alnum_from_mozc_shm`の
  既存の`eisu_reset_on_turn_on_while_open`ゲート付き呼び出しを、ゲート無しの無条件呼び出しに
  差し替える: `self.apply_input_mode_correction(InputModeState::AssumedRomaji{reason:
  AssumedReason::UserHalfWidthAlnumToggleOff}, InputModeApplyStrategy::UserHalfWidthAlnumToggle,
  tick_ms)`。既存の`apply_input_mode_correction`ヘルパー1つへの引数変更のみで、1回のdispatchが
  旧`ImeModel`(無条件reduce)とZ(`UserHalfWidthAlnumToggle`を`ScopeEvent::Explicit`として受理)の
  **両方**に届く(新しい型・呼び出し口・`ImeEvent`/`EngineCommand` variantは一切不要)。モードの値は
  **常に`AssumedRomaji`に固定し、`ObservedKana`は書かない**(ADR-290 D1′ F1が実証した致命的回帰の
  再現を防ぐ)。この書き込みの直後に`self.notify_engine_refresh()`を呼ぶ(両方必須、片方だけでは
  直らない)。当初検討した(a)(`InputModeObserved{source: MozcShm}`)は、Zの`Explicit`claim由来の
  170ms柵(`accepts`)により書き込みそのものが黙って拒否されうる上、Abandon自体が一発勝負で
  再試行が無いため、採用しない(round3 H1)。
- **D2(保留)**: mozc-readableのPulseイベントによる即時通知は、まず`update_tick_ms`との差で
  ポーリング由来の遅延を実機で複数回測定し、実益が投資に値するか確認してから着手を判断する。
- **D3(却下)**: かなキー押下時のbelief/Engineの即時楽観補正。理由は「訂正不能な無期限の
  誤送信を作る」こと(BUG-192/ADR-290 round1と同種のリスク)。
- **D4(見送り)**: 確定まで打鍵を保持する案。ADR-290自身が同じ案を既に見送っており、その見送りは
  ADR-290 D2(未着手)の実装を前提にしている。本ADRでもD1′を優先し見送るが、D2(本ADRのD2・
  ADR-290のD2のどちらも)が未着手である限り、確認が出るまでの区間自体の誤出力への対策は
  依然存在しないことを記録する。
- **D5(採用、2026-11-12追記、BUG-232)**: `decide_use`(`crates/awase-ime-core/src/state/mozc_shm.rs`)
  で、読みがbelief(`effective_open`)と食い違うときは`update_tick_ms`による高速路
  (`read_is_after_baseline`)を使わず、`settle_ms`の無条件経過のみで判断する(belief と一致する
  読みは高速路を維持)。実装済み・Linux単体テスト(`bug232_stale_update_tick_does_not_bypass_
  settle_when_it_disagrees_with_belief`・`decide_use_still_fast_paths_a_read_that_agrees_
  with_belief`を含む全774件)・windows target check/clippy/fmt、いずれもグリーン。却下した
  E2〜E4は上記「案E」節参照。

## 影響

- **変わる挙動**: belief_v2(既定true)環境で、半角英数トグルの手放し検知からEngine活性までの
  約3.65秒の反映欠落がD1′で解消される見込み(実機確認後に確定)。`belief_v2=false`環境では、
  旧モデルへの書き込みが無条件化される(ゲートが外れる)が、同じ`is_romaji_capable()==true`な
  値を書くだけなので実質的な挙動は変わらない(journal上の証拠のreasonラベルが変わるのみ、
  上記トレードオフ参照)。D2は保留のため、mozc-shm自体の検知遅延(最大500ms程度+Mozc自身の
  切替時間)は本ADRの範囲では変わらない。
- **新しいコード**: `ir_do_abandon_half_width_alnum_from_mozc_shm`内の既存呼び出し1箇所の引数
  変更(ゲート除去+strategy変更)+`notify_engine_refresh()`呼び出し1行のみ。新しい型・関数・
  `EngineCommand` variant・`ImeEvent` variantは一切不要。
- **既存のガード/文書は変更不要(round3で確定)**: (b)は新しい`InputModeObserved`構築箇所を
  増やさないため、`architecture_guard.rs::input_mode_observed_construction_sites_are_accounted_for`・
  `mozc_shm_reader_touches_belief_only_through_report_mozc_shm`・ADR-268本文のいずれも
  変更不要。`tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`の
  `UserTurnOnEisuReset`関連チェック(`:918-919`)は`key_pipeline.rs`のみを走査し
  `ime_refresh.rs`は対象外であることを確認済みのため、本ADRの変更はこのガードにも触れない。
  `crates/awase-ime-core/src/state/eisu_recovery.rs`のモジュールdoc「`UserHalfWidthAlnumToggle`は
  対応表の対象外」節(`:63-71`)には、既にこの用途(トグルOFF自体の能動的遷移、SetOpen無し)が
  想定されているが、mozc-shm経由の2つ目の呼び出し元が増えたことを一文追記するとよい(必須ではない)。
- **変わらないもの**: IMEへの能動的な送信(D1′はbeliefの正確さと再評価のタイミングだけを変え、
  OSへの新しい書き込みを増やさない)。
- **実装PRで直すべき、今は本ADRの変更と整合しなくなるコメント3件(round4で洗い出し済み、
  いずれもコメントのみでロジックは変わらない)**:
  1. `crates/awase-hook-core/src/state/half_width_alnum.rs:648-650`(`plan_mozc_shm_follow`の
     doc)が「呼び出し元がBUG-222と同じ経路(`eisu_reset_on_turn_on_while_open`→
     `InputModeApplied`)でromaji-capableへ戻すこと」と**旧い形を指示している**。新しい形
     (`UserHalfWidthAlnumToggle`、無条件)を指すように書き換える。この3件の中で最も重要
     (この判定関数のすぐ隣にあり、実装者が最初に読む場所)。
  2. `architecture_guard.rs:775-791`(`input_mode_applied_construction_sites_are_accounted_for`の
     docstring)の呼び出し元一覧に`ime_refresh.rs`側の`UserHalfWidthAlnumToggle`呼び出しが
     (focus-suspend分も含めて)列挙されていない(既存の記述漏れ)。両方を追記する
     (件数自体は変わらない、`apply_input_mode_correction`に集約されているため)。
  3. `src/engine/mode_state.rs:34-36`の`UserHalfWidthAlnumToggleOff`のdocが「awase自身が
     `VK_DBE_HIRAGANA`注入+conv復元writeでかな入力へ戻した直後」と書いており、OSへの書き込みを
     伴わないfocus-suspend抜け口にも本ADRのmozc-shm抜け口にも既に当てはまらない。「半角英数
     トグルが手放された直後の仮定値(awase自身の書き込み・フォーカス変更・観測への追随のいずれでも)」
     へ広げる。

## 未解決

1. **dragonflyg4の`belief_v2`設定値の確認が望ましい**: 既定は`true`だが、config.tomlでの直接
   確認はしていない。ただし、BUG-231のタイムラインで`[belief-v2] 食い違い開始`/`食い違い終了`の
   ログ行が実際に出力されており(`platform_state.rs:481`の`scope_shadow_compare`が有効である
   証拠)、設定が`true`だったことの直接証拠として既に確認済みと言える。
2. **回帰テストはbelief_v2有効のハーネスで書く**: 既存の`UserTurnOnEisuReset`のgoldenは旧
   `ImeModel`のreducerのみを通す(`run_reducer`)ため、このバグがあっても通ってしまう
   (`fix-requires-evidence.md`の(a)要件を満たすには、`tests/support/harness.rs`の
   `set_engine_belief_z(true)`を使う構成が必須)。最低限、(i) Z有効ハーネスで「手放し後、
   追加のキー無しでEngineが活性化する」こと(ゲートを外したことで直る、G2/H1回帰の固定)、
   (ii) 旧モデルで「手放し後、常にromaji-capableへ終わる」こと、(iii) 書き込むモードが常に
   romaji-capableで`ObservedKana`/`ModeClass::KanaInputMethod`にならないこと(ADR-290 F1回帰の
   固定、H3の命名の罠を踏まえて明示的に`ModeClass::Kana`であることを assert する)、の3つを
   追加する。(i)は`ImeModel::input_mode()`ではなく`engine_belief()`経由(またはEngineの活性
   そのもの)をassertすること——Z自体を確かめないと、このADRが直す反映欠落を素通りしてしまう
   (round4確認)。
3. **[BUG-222](../known-bugs/BUG-222.md)の`kp_exit_lang_detect_toggle_on_hiragana`も同じ
   `UserTurnOnEisuReset`を書くため、belief_v2有効時は同じ穴を持つ可能性が高い**(未検証、
   BUG-231の該当F2の`[key-effect-predict]`が`mode=None`だったのと同じ着眼点でBUG-222自身の
   ログから確認可能)。本ADRでは手放し経路(BUG-231の直接原因)のみ修正し、BUG-222側は別途確認・
   必要なら追随修正する。**BUG-222側で同じ引数スワップ(本ADRと同型)を行う場合**、
   `architecture_guard.rs`の`lang_detect_toggle_is_exited_by_hiragana_key_only_when_marked`
   (`:~9612-9645`)が`key_pipeline.rs`内の`InputModeApplyStrategy::UserTurnOnEisuReset`を
   固定しているため、そのガードの更新も必要になる(round4確認。本ADR自身の変更は
   `ime_refresh.rs`側のみなので、このガードには触れない)。
4. `{UserHalfWidthAlnumToggle, PanicReset}`以外の`InputModeApplyStrategy`がすべてZ配下で
   Engineに無視されるという一般化した穴(ADR-282自身の設計)は、本ADRの範囲外として
   ADR-282の追補課題に記録することを推奨する。
5. 本ADRはBUG-231(手放し後の遅延)のみを対象とし、[BUG-230](../known-bugs/BUG-230.md)のD2
   (GJI側の抜け出し送信にMozcの実際の切替完了の確認を足す、ADR-290で未着手のまま残る項目)には
   踏み込まない。両者は別の未解決項目として残す。D4(本ADR)もADR-290のD2に依存する(上記決定参照)。
6. **D5(BUG-232)のGitHub Windows CI検証(2026-11-12、完了)**: `mozc-shm-e2e`ワークフローに新規
   シナリオ`tickchurn`を追加し(実機mozc-readable、`.github/workflows/mozc-shm-e2e.yml`・
   `tools/mozc_shm_ci/`)、「アイドル中は`update_tick_ms`が進まないが、入力中は`tsf_open`/
   `tsf_mode`が変わらなくても`update_tick_ms`だけ進む」という、本ADRの原因診断が前提にしている
   外部事実そのものを実機で直接確認した(ソース読みだけでなく実機再現):
   run [43145173691](https://github.com/cuzic/awase/actions/runs/43145173691)、
   `measure (tickchurn)` = success、ログ: `idle (3.0s): update_tick_ms: 146265 → 146265
   ticked=False` / `typing: update_tick_ms: 146265 → 150078` / `state_unchanged=True
   tick_advanced=True`。同runのフルmatrix(13ジョブ)は11件成功(`tickchurn`・`stale`・
   `threads`×2・`flip`・`appcontainer`×2・`tidreuse`)、失敗2件(`focus`系、`mismatch (state)`は
   0件で原因は別のフォーカス帰属タイミングの既知フレーク)はD5修正と無関係(退行ではない)。
   `tickchurn`は`stale`/`tidreuse`と同種の常設シナリオとして`.github/workflows/mozc-shm-e2e.yml`
   のmatrixに残す方針([one-off-verification-workflows.md](../../.claude/rules/
   one-off-verification-workflows.md)の対象外、常設workflowが使う部品)。
   このシナリオ自体はawaseの`decide_use`を一切呼ばない(実機Mozcの外部事実の観測のみ)ため、
   D5の修正コード(本ファイル`mozc_shm.rs`)とは独立にPR化した: 検証中に一度`decide_use`の
   修正コミットがCI検証ブランチ側にも重複して乗ってしまったため、リベースで取り除き
   (`tickchurn`関連の2コミットのみに整理、`develop`比で`mozc_shm.rs`の差分ゼロを確認済み)、
   PR [#934](https://github.com/cuzic/awase/pull/934)(`ci/mozc-shm-e2e2` → `develop`、
   **develop へマージ済み**`6b29383d8`)として`tickchurn`シナリオ追加のみを提出した。
   D5の修正コード自体は[PR #935](https://github.com/cuzic/awase/pull/935)として
   **developへマージ済み**(`97d849bfe`、2026-11-12)。
7. **案E4(mozc-readable側のstate専用タイムスタンプ)は未着手のまま**。本ADR(このリポジトリ)の
   範囲外として記録し、`cuzic/mozc-readable`側で将来のバージョンアップ機会があれば別途検討する。
