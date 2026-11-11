---
id: ADR-291
title: |-
  mozc-shm観測をPulseイベントで即時化し、belief補正後にEngineの活性再評価を明示的に呼ぶ(BUG-231)
summary: |-
  ADR-290/BUG-230適用後のdragonflyg4実機(2026-11-11)で、ひらがなキー押下からEngine活性(Active)まで
  最大約5.3秒ローマ字がリテラルで漏れる新しい遅延([BUG-231](../known-bugs/BUG-231.md))を観測した。
  ログ(awase.log)とソース読みで、独立した2つの原因を特定した: (1) mozc-shmの2段ラッチが
  `TIMER_MOZC_SHM`(500ms)の定期ポーリングでしか共有メモリを読まない一方、mozc-readable側は
  実際には`TipStatePublisher::Pulse()`(named event `Local\mozc-readable-ime-changed`)で変化を
  即時シグナルしているのに awase 側がこれを一切使っていない(検知遅延、観測で約1.6秒)。
  (2) `ir_do_abandon_half_width_alnum_from_mozc_shm`がbeliefの`input_mode`を補正しても、
  `engine::Engine::check_active_transition`を再実行しないため、次の物理キー/フォーカス変更の
  `EngineCommand`が来るまでEngineの活性判定が古いまま固定される(反映遅延、観測で約3.7秒・
  今回の主要因)。(2)は既存の`Runtime::notify_engine_refresh()`(`EngineCommand::RefreshState`、
  `process_deferred_keys`等で既に使われている既存のヘルパー)を同じ場所で呼ぶだけで直る。
  (1)はmozc-readableの既存のPulseイベントをawase側で待ち受ける新しい常駐ワーカースレッドが要る。
  第3の案(ユーザー提案: かなキー押下時にbeliefを即座に楽観補正する)はBUG-192/ADR-290 round1の
  D1′と同型のリテラル漏れ回帰リスクがあるため却下する。
status: |-
  起草(2026-11-11)。Opusレビュー予定、実装・実機確認とも未着手。
related_adr:
  - "ADR-290"
  - "ADR-268"
  - "ADR-270"
  - "ADR-265"
---

# ADR-291: mozc-shm観測をPulseイベントで即時化し、belief補正後にEngineの活性再評価を明示的に呼ぶ

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
| 05:47:20.544 | (+1.63秒) `[shift-conv-guard]` mozc-shmの観測でかなへの追随を確認→トグルを手放す。`apply_input_mode_correction`実行。**Engine再チェックなし** |
| 05:47:20.544〜24.195 | バックスペース×4・T×2・Space・Enter等、複数の物理キー押下。いずれも`Engine::check_active_transition`を起動しない |
| 05:47:24.195 | ひらがなキー押下2回目(ユーザーの再押下と見られる) |
| 05:47:24.231 | (手放しから+3.65秒) `Engine activated (reason=Active)` |

1回目のかなキー押下(18.911)からEngine活性(24.231)まで**合計約5.32秒**。このうち約1.63秒が
mozc-shm観測の検知遅延、約3.65秒がEngine再評価の反映遅延という、独立した2段階の遅れで構成される。

## 検討した案

### 案A(採用・D1): `notify_engine_refresh()`を手放し処理の直後に呼ぶ

`crates/awase-windows/src/runtime/ime_refresh.rs::ir_do_abandon_half_width_alnum_from_mozc_shm`は
`apply_input_mode_correction`でbeliefを書き換えるだけで終わっており、`engine::Engine::
check_active_transition`を一切再実行しない。`check_active_transition`は`Engine::on_input`
(物理キー経由)か`EngineCommand::FocusChanged`/`RefreshState`(`on_command`経由)からしか
呼ばれないため、Engineが非活性の間に通常キーが`on_input`を経由しない設計(BUG-231の観測5と
整合)だと、beliefの補正自体は次に`EngineCommand`が発行されるまでEngineに伝わらない。

`Runtime::notify_engine_refresh()`(`crate::runtime::mod.rs`、`EngineCommand::RefreshState`を
発行する既存のヘルパー。`process_deferred_keys`・`ir_notify_engine_refresh`で既に使用例あり)が
まさに「物理キーでもフォーカス変更でもなく、belief補正だけが起きたときにEngineの活性判定を
再実行する」ための既存の手段。`ir_do_abandon_half_width_alnum_from_mozc_shm`の
`apply_input_mode_correction`呼び出しの直後にこれを1行足すだけで、BUG-231の約3.7秒の
反映遅延を解消できる見込み(**新しい型・新しい`EngineCommand` variant・新しい`ImeEvent` variantは
一切不要**、既存のbelief規約(`.claude/rules/ime-belief-architecture.md`)にも触れない——
belief自体は`apply_input_mode_correction`が既に正しく書いており、追加するのはEngine側の
「読み直すタイミング」だけ)。

**なぜ`EngineCommand::FocusChanged`ではなく`RefreshState`か**: `FocusChanged`の
`handle_focus_changed`は「前のウィンドウで入力途中だったキーを別のウィンドウに持ち越さない」
という`ThumbRawVkEmission::Denied`での強制フラッシュを伴う(ウィンドウが実際に変わった前提の
副作用)。今回はウィンドウは変わっていないため、`FocusChanged`を流用すると無関係な副作用を
踏む恐れがある。`RefreshState`は`check_active_transition`を呼ぶだけ(フラッシュなし)で、
用途が正確に一致する。

### 案B(採用・D2): mozc-readableのPulseイベントを待ち受けて即時に読み直す

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

**Pulseパターンの既知のレース(実装前に踏まえる前提)**: `Pulse()`は手動リセットイベントに対する
`SetEvent`直後の`ResetEvent`であり、これはWin32の`PulseEvent()`API自体が持つのと同型のレース
(イベントが`Set`から`Reset`に戻るまでの間に待機側がちょうど`WaitForSingleObject`に入っていないと、
そのシグナルを取り逃す)を内包する。したがって本イベントは「確実な通知」ではなく「検知を早める
ための加速材」として扱い、既存の定期ポーリングを置き換えない設計とする(取り逃しても、次の
`TIMER_MOZC_SHM`ティックで追いつく)。

**mozc-readable未導入環境への影響なし**: `OpenEventW`はmozc-readableのTIPプロセスが実際に
走っていて初めて名前付きイベントが存在するため、未導入環境では単に開けず(既存の`NotFound`の
扱いと同じ思想)、ワーカースレッドは起動しない・既存のポーリングのみで従来どおり動作する。

### 案C(却下): かなキー押下時にbeliefを即座に楽観補正する(ユーザー提案の2つ目)

ひらがなキー押下を検知した時点で、Mozcの実際の切替完了を待たずにbelief(`input_mode`・Engine活性)
を即座に書き換える案。これは[BUG-230](../known-bugs/BUG-230.md)/ADR-290 round1でOpus敵対的
レビューにより却下されたD1′(モードキー受信時に`toggle_held`を即座に解除コミットする案)と
本質的に同型であり、同じ理由で却下する: Mozc自身のモード切替には実測で最大約2秒かかる
(BUG-230のタイムライン、08:09:59.859押下→08:10:01.984切替完了)。awase側のbeliefをその完了より
前に「成功」として進めると、NICOLAのローマ字が送られる先がまだ半角英数のMozcであるため、
BUG-230の元の症状(リテラル漏れ)をそのまま再現する。本ADRが解決しようとしているBUG-231は
「確認を待つこと自体の遅延」であり、「確認をやめる」ことは別のバグへ後退するだけで解決にならない。

## 決定

- **D1(採用)**: `ir_do_abandon_half_width_alnum_from_mozc_shm`の`apply_input_mode_correction`
  呼び出しの直後に`self.notify_engine_refresh()`を1行追加する(案A)。新しい型・variant・
  belief書き込み経路は増えない。
- **D2(採用)**: mozc-readableの既存のPulseイベント(`Local\mozc-readable-ime-changed`)を
  `crates/win32-worker`ベースの常駐ワーカースレッドで待ち受け、変化を検知したら即時に
  `mozc_shm_read_once`を1回走らせる(案B)。既存の`TIMER_MOZC_SHM`ポーリングは安全側の
  バックストップとして残す(Pulseパターンのレースのため、置き換えではなく加速材)。
- **D3(却下)**: かなキー押下時のbelief/Engineの即時楽観補正(案C)。BUG-192/ADR-290 round1の
  既存の却下理由がそのまま当てはまる。

## 影響

- **変わる挙動**: mozc-readable導入環境で、半角英数トグルの手放し検知からEngine活性までの
  遅延が、D1だけで約3.7秒の反映遅延が消え、D1+D2で約1.6秒の検知遅延も大部分短縮される見込み
  (実測は実装後)。mozc-readable未導入環境はD2の影響を受けない(ワーカースレッドが起動しない)。
  D1はmozc-readableの有無に関わらず効く(他の`ir_abandon_half_width_alnum_from_mozc_shm_if`
  呼び出し元にも一般化可能だが、本ADRはBUG-231の直接原因である手放し経路のみを対象とする)。
- **新しいコード**: D1は既存ヘルパーの呼び出し1行。D2は`crates/win32-worker`へ
  `WaitForMultipleObjects`対応の追加(または新しい小さなラッパー)、`observer/mozc_shm.rs`付近へ
  `OpenEventW`呼び出しと常駐スレッドの起動・シャットダウン配線、メインループへカスタム
  ウィンドウメッセージのハンドラ1つ。`tuning.rs`への新定数追加は不要(`MOZC_SHM_TIMER_MS`は
  バックストップとして現状維持)。
- **変わらないもの**: belief更新の経路・信頼度(`ObservationSource::MozcShm`、ADR-268のまま)、
  IMEへの能動的な送信(D1もD2も観測・反映の速度だけを変え、OSへの新しい書き込みを増やさない)。

## 未解決

1. **D2の実機検証が必要**: `Local\`名前空間のイベントがawase.exe(非昇格)とmozc-readableのTIP
   (Mozcサーバー/UIプロセスの権限コンテキスト)の間で`OpenEventW`できるかは、`WinSandbox::
   MakeSecurityDescriptor(kSharableEvent)`の実際のDACLを実機で確認するまで未確定。失敗時は
   単純にポーリングのみへ縮退する想定(フェイルセーフ)だが、実機で確認する。
2. **D1単独の効果測定**: D1は実装が小さいため先行してマージし、dragonflyg4で約3.7秒の反映遅延が
   実際に消えるかをログ(`Engine activated`のタイムスタンプ)で確認してからD2に進む、という
   段階的な投入を想定している(D1とD2は独立に有効)。
3. 本ADRはBUG-231(手放し後の遅延)のみを対象とし、[BUG-230](../known-bugs/BUG-230.md)のD2
   (GJI側の抜け出し送信にMozcの実際の切替完了の確認を足す、ADR-290で未着手のまま残る項目)には
   踏み込まない。両者は別の未解決項目として残す。
