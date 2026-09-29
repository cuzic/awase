---
id: ADR-205
title: |-
  Imm32Unavailable(Chrome 等)の窓で、外部から閉じられた IME を「検証済みの読み取り専用照合」で観測して belief に反映する(BUG-172)
summary: |-
  実 Chrome × GJI では、他プロセスが注入した半角/全角(0xF3)や VK_IME_OFF で IME が閉じても、awase は開閉を観測できず(observed=0)、
  HwndCache が復元した古い ime_on=true のまま NICOLA ローマ字を送る(10/10、run 36540419485)。本 ADR は、Blacklist 戦略(Imm32Unavailable)の観測段に
  「限定した2点(フォーカス確定後の検証読み、外部注入 IME キー後の照合読み)だけ、IMC_GETOPENSTATUS を読み取り専用で読む」経路を足す。
  常に 0 を返す環境(read_ime_state_fast のコメントの主張)で偽の OFF を採用しないよう、そのクラスで 1 を一度読むまで 0 を採用しない検証 latch を持つ。
  belief 反映は既存の `write_observer_poll`(GJI I/O 観測と同じ入口)経由のみ。書き込み・新 actuation 合流点・新タイミング定数は足さない。
status: |-
  起草(2026-09-29)。opus-adversarial-consult 待ち。実装未着手。
related_adr:
  - "ADR-029"
  - "ADR-089"
  - "ADR-178"
  - "ADR-191"
---

# ADR-205: 外部から閉じられた IME を Imm32Unavailable の窓で観測する

## 背景(BUG-172、実 Chrome の測定)

[docs/known-bugs/BUG-172.md](../known-bugs/BUG-172.md) の実 Chrome 測定(2026-09-29、GitHub windows CI、各10試行、`cal-driftrec-chrome-real-*`):

- **GJI × 実 Chrome は、他プロセスが注入した 半角/全角(0xF3)・VK_IME_OFF で 10/10 再現**。IME は `IMC_GETOPENSTATUS` で 1→0 に閉じ、3秒後も閉じたまま(`open_at_probe=Some(0)`)、
  打鍵は `kiu`(ローマ字のまま。Engine は ON のまま=直接入力に落ちる)。
- awase の Chrome 開閉の**観測件数 = 0**(`ObserverPoll`=0、`Imm32Unavailable` 39件)。注入キーは hook で見えている(`injected=true`)が、
  `key_pipeline.rs` の BUG-14 分岐(`event.injected` → 「ユーザー意図に昇格させない — belief 追従は may_change_ime refresh 観測に委譲」)で昇格せず、
  委譲先の refresh が Chrome では `ir_stage_observe` の `Blacklist` 分岐(`Skipping IMM query for known-broken class`)で読まないため、belief は古い ON のまま。
- 物理キー(awase 経由)では起きない(awase が shadow-toggle で Engine も OFF にする)。メモ帳経由(アプリ間)は Chrome に影響しない(IME 開閉は窓/スレッド単位)。
- MS-IME × 実 Chrome は注入の 0xF3/0x1A を効かせず再現できていない。対象はまず GJI。
- 「ゲートへ開閉を要求する」案は実 Chrome の症状を直さないため却下済み(BUG-172)。
- 撤去済みの ADR-178 領域A(reassert・force-on)を戻しても差は出ない(対照 `ci/bug172-pre-teardown`)。回復手段の欠如ではなく**観測の欠如**が原因。

**観測経路に乗ったかの確認**(教訓): 上の 0/10 は「起きなかった」ではなく `observed=0`(判断に届いていない)。本 ADR の効果判定は verdict でなく observed 件数で行う。

## 前提となる既存コードの事実(要独立再検証)

- `focus/class_names.rs::AppImeProfile::can_read_imm32_open_status`: `Imm32Unavailable | TsfNative | InputRelay` は `false`。`ime.rs::read_ime_state_fast` は
  この場合 `ime_on=None`(shadow に委ねる)を返す。コメントの根拠は「Chrome_WidgetWin_1 等は常に 0 を返す」(出典の測定記録は見つけられていない)。
- 一方 `chrome_probe`(`ci/bug172-realistic-close`)は、Chrome の**トップレベル窓**(タイトル `IMEPROBE`)の `ImmGetDefaultIMEWnd` へ `WM_IME_CONTROL/IMC_GETOPENSTATUS` を送って **1→0 を読めた**
  (GJI・MS-IME とも)。両者は矛盾する。「常に 0」は環境(Chrome のバージョン・IME・レンダラの状態)依存の可能性がある。**本 ADR は矛盾を解消せず、両方が起こりうるものとして設計する**(D3)。
- `ir_stage_observe` の `Blacklist` 分岐は、既に GJI I/O 観測(`observe_gji_after_focus`)を `write_observer_poll` へ流している。読み取り専用の観測を足す前例と入口がある。
- 読み取りは `imm::probe_ime_control(ime_wnd, ProbeCmd::GetOpenStatus, 20ms)`(`SendMessageTimeout`)で、`read_ime_state_fast_async` が `offload_unsafe` でワーカーへ逃がしている(フックスレッドをブロックしない)。
- `TSF ITfCompartmentEventSink` は ADR-029 で「thread-manager スコープで他プロセスの変更を検知できない」として削除済み。ADR-191 の2〜5ms 実測(`compartment_notify_probe`)は**自プロセス・自スレッドの compartment**の測定。

## 案の比較

| 案 | 内容 | 実 Chrome の症状に効くか | 副作用・リスク | 判定 |
|---|---|---|---|---|
| A | `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` の通知購読 | **効かない可能性が高い**。thread compartment は awase のスレッドのもので Chrome のスレッドの変化は届かない(ADR-029 の実装・検証での結論、BUG-172 のメモ帳/Chrome 独立の測定と整合)。グローバル compartment 経由で届くかは**未検証** | 新しい COM sink・寿命管理・STA の要件。効くと確認できていない | 不採用(未検証を理由に採らない。B が効かなければ A の跨スレッド到達性だけをスパイクで測る) |
| **B** | 限定した点で `IMC_GETOPENSTATUS` を読み取り専用で照合し、`write_observer_poll` へ | 効く見込み(実 Chrome で 1→0 が読めることは測定済み) | 「常に 0」環境での**偽の OFF**(→ Engine が誤って OFF になる)。D3 の検証 latch で抑える | **採用** |
| C | フォーカス復帰時に HwndCache の ime_on を鮮度切れ扱い | **効かない**。再現条件は「Chrome にフォーカスがあるまま外部注入で閉じる」でフォーカス復帰を伴わない | 復帰のたびに belief が不明になり Engine の挙動が揺れる | 不採用(B の検証読みが結果的に同じ役割を果たす) |
| D | 何もしない・既知の制限として記録 | 効かない | なし | B が却下された場合の代替。頻度が低いという判断もありうる |
| E | `SetWinEventHook`(`EVENT_OBJECT_IME_*`)で購読 | 効かない見込み。既存コメントが「GJI TSF モードでは発火しない」と記す。IME_CHANGE は開閉ではなく変換対象の変化 | — | 不採用 |
| F | 打鍵ごとに読む | 効くが、打鍵ホットパスにクロスプロセス SendMessage を載せる(BUG-34 型のブロック、`TYPING_IDLE_MS` の設計に反する) | 高 | 不採用 |

## 決定

### D1. 読む点は2つに限定する(打鍵ごとに読まない)

`ir_stage_observe` の `ImeReadStrategy::Blacklist` 分岐に、読み取り専用の開閉照合を1つ足す。読む点は既存の refresh が既に走る次の2点だけ:

1. **フォーカス確定後の refresh**(`focus.focus_changed` の回): 検証読み(D3)と、閉じたまま戻ってきた場合の検出を兼ねる。
2. **外部注入 IME キー(`may_change_ime` かつ `event.injected`)が引いた refresh**: BUG-14 分岐が委譲した「belief 追従の観測」そのもの。

`SkipTyping`(打鍵中)では読まない(既存の判定のまま)。追加の周期タイマー・新しい tuning 定数は作らない。

### D2. 読み取りは `imm::probe_ime_control`(既存の20msタイムアウト・ワーカー offload)を使い、判断は純粋関数に切り出す

- 副作用部: `ime.rs` に、`can_read_imm32_open_status` のプロファイルゲートを**通さない**読み取り専用関数 `read_open_status_ungated_async()` を足す(`read_ime_state_fast` と同じ hwnd 解決・`probe_ime_control`)。書き込み系(`IMC_SETOPENSTATUS`)は呼ばない。
- 純粋部: `observer/ime_observer.rs` に `classify_blacklist_open_read(validated: bool, read: Option<bool>) -> BlacklistOpenVerdict`。`BlacklistOpenVerdict` は `NoEvidence`(読めず/時間切れ) / `Validate`(1 を読んだ=経路が生きている) / `ObserveOpen(bool)`(採用する観測) / `DiscardUnvalidatedClosed`(未検証の 0 は捨てる)。
- 反映: `ObserveOpen(v)` を `write_observer_poll(v, tick_ms, AcceptedObservation::for_sync(focus_fence))`(GJI I/O 観測と同じ入口、`ImeModel::reduce()` が唯一の書き込み点)へ渡す。
  `ImeEvent` の新 variant・`UserImeSetIntent`・`HeuristicDefault` の偽装は使わない。

### D3. 検証 latch: そのクラスで 1 を一度も読めていない間は 0 を採用しない

「常に 0」説と「実測で 1→0 が読める」説が両立しないため、偽の OFF を構造的に防ぐ。

- キー = `(process_name, class_name)`。セッション内のメモリのみ(永続化しない)。`Validate`(read=Some(true))で `validated=true`。
- `validated=false` の間の `Some(false)` は `DiscardUnvalidatedClosed`(採用せず、ログに `[chrome-open-read] discarded unvalidated 0` を出す)。
- `validated=true` の後は `Some(true)`/`Some(false)` とも `ObserveOpen`。
- 帰結: 「常に 0」環境では一度も 1 が読めないので従来と同じ挙動のまま(観測ゼロ、退行なし)。1 が読める環境では、以後の 0 は本物の閉じた証拠になる。
- 限界: 検証前に(IME が閉じたまま)初めてフォーカスした場合は、そのクラスの最初の 1 を読むまで検出できない。許容する。

### D4. 書き込み・合流点・定数は足さない

- 新しい actuation 合流点なし(`RESTRICTED_CALLS` 不変)。既存の drift correction が、観測 OFF と食い違う明示意図があれば従来どおり動くだけ(BUG-14 昇格をしないので、外部注入だけの場合は明示意図が無く動かない)。
- 新しい `_MS` 定数なし。20ms は既存の `probe_ime_control` の値。読む点は既存の refresh の再利用。complexity-budget の対象(actuation 合流点・tuning 定数)は増えない。
- ADR-178 領域A撤去・ADR-191 受動化の方針(能動書き込みを足さず観測だけを足す)に沿う。

### D5. 対象範囲

`AppImeProfile::Imm32Unavailable` のうち `read` が成立するクラス。`TsfNative`(RichEdit スーパークラス等)・`InputRelay` は対象外(BUG-172 のもう一つの経路〈msime-ready ゲート〉は別問題として見送りのまま)。
GJI に限定はしない(観測は IME の種類に依らず開閉を読むだけ)が、効果の検証対象は GJI × 実 Chrome。MS-IME は再現できていないので観測のみで効果は主張しない。

## リスクと検証計画

| リスク | 対策・確認 |
|---|---|
| 偽の OFF で Engine が誤って OFF になる(最大のリスク) | D3 の検証 latch。`[chrome-open-read] discarded unvalidated 0` の件数と、通常打鍵(IME ON のまま)の e2e で `ObserveOpen(false)` が **0 件**であることを確認する(`ts-chrome` 高速打鍵、`cal-driftrec-chrome-*` の対照)。 |
| Chrome 内でのフォーカス移動(アドレスバー↔ページ)で窓ごとに値が違う | 読む点はフォーカス確定後と注入キー後のみ。同一 FocusChange 内では latch はクラス単位のため、アドレスバー(別クラス)は別キー。実 Chrome での omnibox 往復を e2e で確認(未実施)。 |
| 読み取りがブロックする | 20ms タイムアウト+`offload_unsafe`。時間切れは `NoEvidence`(miss に数えない〈BUG-158追補〉)。 |
| 観測 OFF が drift correction の再 ON を誘発する | 外部注入だけなら明示意図が無く動かない。物理 ON 後の外部 close なら drift が動く(ImmCross と同じ既存設計。意図を尊重して外部 OFF に逆らうかは D4 の範囲外、レビューで論点にする)。 |
| MS-IME・edit・他アプリへの副作用 | Standard(ImmCross)は既存の OsPoll のまま変更しない。Blacklist の GJI I/O 観測とは同じ入口だが互いに独立(値の食い違いは `most_recent_trusted` の既存規則)。 |
| spurious apply が起きる | 本 ADR は書き込みを足さない。`apply_ime_open_*` の呼び出し件数を architecture_guard で固定したまま変えない。 |

**回帰テスト**(host で走るもの): `classify_blacklist_open_read` の表(未検証0捨て・検証済み0採用・検証1・時間切れ)。`ImeModel::reduce()` に `ObserverPoll(false)` を渡すと `effective_open()` が false になる既存性質のリプレイ。architecture_guard: `write_observer_poll` の呼び出し元件数と、`read_open_status_ungated_async` の呼び出し元を Blacklist 分岐1か所に固定。

**実機・CI**: 測定用ブランチから `gh workflow run e2e-ime.yml --ref <branch> -f only='cal-driftrec-chrome-real-*'`(乱発しない)。合格条件:
`cal-driftrec-chrome-real-hz-ext-gji` と `imeoff-ext-gji` が FAIL 10/10 → PASS、かつ **observed 件数 ≥ 試行数**(現状 0)。対照として物理キー相当(目印付き 0xF3)と MS-IME が退行しないこと、通常の Chrome 打鍵シナリオで偽 OFF 0 件。

## 未決事項(所有者判断の候補)

1. 検証 latch で「最初にフォーカスした時点で既に閉じている」ケースを取りこぼすことの許容。
2. 物理 ON 後の外部 close で drift correction が再 ON を試みる挙動(既存の意図優先設計)を Chrome にも適用してよいか。
3. 「常に 0」説と「1→0 が読める」説の矛盾の解消(別途 `read_ime_state_fast` のコメントの出典を調べる)。

## 敵対レビューの記録

(各ラウンドの指摘と対応をここに追記する)
