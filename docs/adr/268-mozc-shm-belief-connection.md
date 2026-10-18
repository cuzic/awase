---
id: ADR-268
title: |-
  mozc-readable の共有メモリを belief に High で接続し、食い違う applied を降格する
summary: |-
  ADR-265 の段階 2。共有メモリの開閉を ObservationSource::MozcShm(High・BeliefOnly)として belief へ入れ、外から変わったときは既存の追随の経路に乗せる。
  drift correction の根拠からは除外する(除外しないと IME を開け直し続ける)。applied は、共有メモリと食い違ったら Unknown に降格するだけで、Confirmed への書き込みは
  効果を journal で示せてから別に足す。実機実験とOpusレビュー2ラウンドで設計を改めた。E1(外からの変更が共有メモリに出るか)の不一致は未解決。
status: |-
  実装済み(2026-10-18)だが、**実機の検証で 2 点の問題が見つかった(同日追記)**: (1) TTL=1000ms は不足(tick の間隔の実測、1000ms 超が 8.8%)、
  (2) **明示意図が残っている間は、外から IME を閉じられても belief が追随しない**(`reschedule_ime_refresh` が tick を止めるため。実機で有効な 5 周中 0 周が追随)。
  (2) の対策として D5(共有メモリの読みを refresh の tick から切り離した専用の軽いタイマー)を提案中(利用者の判断待ち、未実装)。
  D1〜D4 は develop にマージ済み(核 #678、配線 #679、降格と journal #682。CI は通過、実機は #679 まで確認)。未確認: E1 の不一致(未解決 1)、#682 の実機での動作、Edge などの TSF ネイティブの窓・物理キー・実際の NICOLA の打鍵。
related_adr:
  - "ADR-265"
  - "ADR-267"
  - "ADR-214"
  - "ADR-208"
  - "ADR-205"
  - "ADR-256"
  - "ADR-098"
---

# ADR-268: mozc-readable の共有メモリを belief に High で接続し、食い違う applied を降格する

## 背景

[ADR-265](265-mozc-readable-integration.md) の段階 1 で、mozc-readable が書き出す共有メモリ(`Local\mozc-readable-ime-<tid>`、48 バイト、version 2)を awase が読み、journal に残す観測(影の読み)を実装した。
段階 2 は、これを IME の開閉の belief へ入れること。ADR-265 は「Low から始め、基準を満たしたら Medium」としていたが、**利用者の決定(2026-10-18)で、共有メモリは Mozc 自身の状態の事実なので、最初から High で入れる**。v2 の設計はモードずれが一定の確率で起きる前提で、mozc-readable を入れる利用者は最初から事実を優先する動作を期待している、という理由。

この ADR は、High で入れたときに起きる問題を、実機実験と Opus の敵対的レビュー(2 ラウンド)で洗い出し、それを塞ぐ形に設計を固める。詳しい設計は [adr265-belief-connection-design.md](../tasks/adr265-belief-connection-design.md)、実験の結果は [adr265-belief-experiments-result.md](../tasks/adr265-belief-experiments-result.md)、レビューは [review/268/](review/268/) にある。

## 実機実験で分かったこと(dragonflyg4、1 台。詳細は結果の文書)

- 注入キーから共有メモリが変わるまで: 200/200 が期待の向きで最大 95ms(素の Win32 窓、負荷 8 でも 94ms)。
- IMM の書き込み(`IMC_SETOPENSTATUS`): 素の Win32 窓は 42/50 が反映。TSF ネイティブの Notepad は 0/50 で、IMM の読み戻しは書いた値に追随したのに、実際の入力は変わらず、共有メモリが実際の状態に合った。
- `applied` を共有メモリで Confirmed にする規則 R の検証(素の Win32 窓 A=40・B=50、Edge A=40): R を通った読みの食い違い 0、R なしの生の読みの食い違いも 0。**古い読みは 1 件も観測されなかった**(最短 14ms でも新しい状態)。
- **ツールバーのメニュー操作が共有メモリに出るか(E1)**: 3 回の測定が不一致(1 回目 0/20、2・3 回目は変化あり)。原因は未解決。
- パスワード欄の `tsf_open != effective_open` は、測定器の不備で結論なし。

## 決定

### D1. belief への入れ方
- 新しい `ObservationSource::MozcShm`(+ `evidence::MozcShm`)。信頼度は **High**、authority は **`BeliefOnly`**(IME へ送る根拠 = `derive_actuating`・授権には入れない)。自動判定(設定なし)。
- 入口は `ImeStateHub::report_mozc_shm_open(open, hwnd, focus_epoch, tick)` の 1 つ。`tsf_open` を既定の値にする。`tsf_open != effective_open` のときは belief に報告せず、`applied` を降格する(必須)。
- `derive_filtered` の `is_identity_ok` に `MozcShm` を足す(epoch+hwnd)。
- 鮮度: `record_any` で `source == MozcShm` のときだけ `expires_at = Some(at + TTL)`。`derive_any`・`most_recent_trusted`・`_excluding`・`consensus` の全読み手が `is_expired` を見るので、同時に失効する。ADR-087 INV-22 撤回(`heuristic_default` の「鮮度上限を持ち込まない」)の**例外**になることを明記する。TTL は tick の最大間隔(約 500ms + prefetch の 300ms のタイムアウト + ジッター)より長く、実測で導く。
- 読む対象: (tid, pid) ごとに `NotFound` を覚え、フォーカスが変わるまで読まない(mozc-readable を使わない利用者に毎 tick の Win32 呼び出しを課さない)。

### D2. drift correction から除外する(実装前に必須)
`evaluate_drift` の根拠(`most_recent_trusted_excluding`)と `read_back` の `EXCLUDED_FROM_ANY_FRESH_EVIDENCE` から `MozcShm` を**除外**する。除外しないと、High の共有メモリが明示意図と食い違うたびに drift correction が IME を開け直し(Standard の窓)、Blind の窓で BUG-114 型の再武装ループと BUG-124 型の「@」を起こす。drift correction と `read_back` は authority を見ないので、`BeliefOnly` だけでは止まらない。

### D3. 外から変わったときの追随(R3')
`resolve_open_at` の優先順は 明示意図 → 打鍵予測 → `derive_any`、さらに `IntentStore`(ON 10 秒・OFF 30 秒)が上にあり、High だけでは外から閉じられた場面で belief が直らない。
- 共有メモリの値が `effective_open` **または** `desired_open` と食い違い、かつ次の **R3'** を満たすとき、既存の追随と同じ 3 手を実行する: `report_mozc_shm_open`(High)→ `intent_store.remove(hwnd)` → `pass_through_observed(tick, true, true)`。新しい `ImeEvent` variant は要らない。
  ```
  R3' = (update_tick_ms > 基準) || (読んだ時刻 − 基準 >= SETTLE)
  基準 = max(hook_alive_tick_ms, 最後の ImmCross の書き込みの発行時刻)
  SETTLE = KEY_EFFECT_SETTLE_MS (170ms、既存の定数を流用)
  ```
  時計は同じ(`GetTickCount64`)。新しい欄は「ImmCross の発行時刻」1 つだけ。第 2 項は、書き込みが IME に届かず共有メモリが変わらない場面(N2)と、`GetTickCount64` の分解能 約 15.6ms で `>=` が古い読みを通す経路(warmup の `VK_DBE_HIRAGANA` に続く `SetOpen`、chord、自動リピート)の両方を塞ぐ。
- R3' を満たさない読みは、belief に報告せず追随もしない(ただし降格はする、D4)。
- 打鍵予測の照合の前に、「打鍵より後に変化していない読み」は照合しない(ハブで止める)。追随の入口が増えるので、`architecture_guard` の列挙に追記する。Edge のように既存の追随(ObserverPoll)も効く窓では、`MozcShm` が受け入れられた tick は ObserverPoll の追随を飛ばす。

### D4. `applied` は降格だけ(Confirmed は書かない)
- 共有メモリの受け入れた読みが `applied_open()` と食い違うとき、`applied` を `Unknown` にする。R3' は不要(古い読みで降格しても被害は余分な送信 1 回)。`tsf_open != effective_open` のときも降格する。
- これで ADR-214 の P1(トレイ起点の Engine コマンドの `SetOpen` が、古い `applied` で GjiDirect の送信を省かれる=Engine の固着)と、共有メモリが読める窓での P3 が直る。共有メモリが読めない窓は対象外で、従来どおり。
- **Confirmed を書かない理由**(利用者決定): 検証が通った窓の種類で、Confirmed にして減らせる送信がほとんど無い。素の Win32 窓は ImmCross が先頭で `applied` を見ない。Edge(`Imm32Unavailable`)は押下で `applied` を未知にするので、効くのは押下以外の送信だけで、得は「余分な送信 1 回」。ADR-214 の `Sent`/`Confirmed` の型の分離はしない(降格は `AppliedImeState` に新しい値を書かないので、ADR-214 の混ぜを悪化させない)。
- Confirmed を足す条件: journal の `ActuationDecision` で、「`applied=Unknown` のせいで送られた、押下以外の余分な送信」の件数を数え、便益を示せたとき。足すなら、R3'、(プロセス名, クラス) の許可リスト(Edge は `msedge.exe`)、「`update_tick_ms` が最後に `applied` を `Unknown` にした時刻より後」の条件、新しい variant(`scope` を持たせない。`ForegroundScope` は `Serialize` でない)、追随→確認の順の固定、不変性のテストを守る。

## 検討した案

| 案 | 判断 |
| --- | --- |
| ADR-265 のとおり Low から始める | 利用者決定で却下(High で最初から) |
| High を drift correction の根拠にも使う | 却下。IME を開け直し続ける(D2) |
| 共有メモリを `Actuating`(授権)にする | 却下。「すでにその状態なら送らない」は `applied` の判断で、授権(`open_warrant.rs`)は別。入れても省略は変わらない |
| `applied` を共有メモリで Confirmed にする(規則 R、条件 3 は `>=`) | 当面入れない(D4)。`>=` は同じ目盛りの古い読みを通す危険があり、R3' に改めた |
| 追随せず、High だけにする | 却下。明示意図・`IntentStore` が上に効き、BUG-200 の場面で belief が直らない |

## 影響
- **足すもの**: `MozcShm`(source・evidence)、`report_mozc_shm_open`、降格の入口(単独で呼べるかを確認)、追随の入口、tuning 定数 1 本(TTL)、新しい欄 1 つ(ImmCross の発行時刻)。**新しい `ImeEvent` variant は無い**。`complexity-budget.md` は未発効。
- **変わる挙動**: mozc-readable の窓で、belief が共有メモリに従う。外から変えられると追随する。共有メモリと食い違う `applied` が降格する(余分な送信が 1 回増えうる)。
- **変わらないもの**: IME への送信の根拠(授権)、drift correction の発火条件、共有メモリが読めない窓。

## 未解決・実装前の確認
1. **E1(外からの変更が共有メモリに出るか)の不一致**。公開されない外部変更があると、High の共有メモリが、正しい ObserverPoll(Medium)を上書きして、belief が古いまま固定される(mozc-readable を入れていない場合より悪くなりうる)。降格は `applied` の安全側なので影響しないが、belief の High にはこの危険が残る。IMM で読める窓で、High の共有メモリが新しい IMM の直接読みと食い違ったときの扱い(共有メモリを採らない、など)を、実装前に決める。
2. TTL の値: **実測した(2026-10-18、下の「実機での検証と訂正」)**。1000ms は不足(1000ms 超の間隔が 55/625 件)。1500ms で 1000〜1500ms の範囲(11 件)は覆えるが、1500ms 超 44 件(tick が止まる間隔)は TTL では覆えない → D5。~~journal の頻度の方式~~ → 解決(#682: 同じ報告は journal に書かない)。
3. ~~`demote_applied` を、追随なしで単独に呼べるか。~~ → 解決(#682: `ImeEvent::AppliedDemoted` を足した。新しい variant は要らない、という当初の想定は外れた)。
4. 「新しい窓の最初のキーは効かない」(先に a+Enter を 1 回打つと IME キーが効く)の因果と BUG-200 との関係(別の調査)。
5. 共有メモリの version の上限(ADR-265 D3 の「version の扱い」、ADR-267 の v3 との関係)。

## 実装の状況(2026-10-18)
- **済(develop にマージ、CI 通過、実機未確認)**: `ObservationSource::MozcShm`(High・BeliefOnly)、`Observed<MozcShm>::from_shm`、`record_any` の TTL(`MOZC_SHM_OBSERVATION_TTL_MS = 1000`、`pending`)、`is_identity_ok`、drift correction と `read_back` からの除外(D2)、純粋関数 `decide_use`・`read_is_after_baseline`(R3')、`ImeStateHub::report_mozc_shm`(報告+追随+降格)、`hook::last_operation_tick_ms`(基準 = `hook_alive_tick_ms` と ImmCross の発行時刻)、共有メモリが開けなかった (tid, pid) の記憶。ガード `mozc_shm_reader_touches_belief_only_through_report_mozc_shm`。
- **#682 で追加(develop にマージ、CI 通過、実機未確認)**: `tsf_open != effective_open` のときの降格(`ImeEvent::AppliedDemoted { cause: AppliedDemoteCause }`、reducer は `applied` だけを書く。`ModeKeyPassedThrough` は常に `last_intent` を捨てるので流用できなかった。dispatch 元は `demote_applied_for_mozc_shm_split` の 1 箇所。`architecture_guard` の `.applied = ` の件数 9→10)、同じ共有メモリの報告の journal の重複除去(reduce はするが journal には書かない。`event_seq` に欠番ができる)。
- **未実装**: なし(D1〜D4)。Confirmed(将来の拡張、§D4)は、効果を journal で示せてから別に足す。
- **挙動の変化(マージ済み)**: mozc-readable の窓で、belief が共有メモリ(High)に従い、外から変えられると追随する。共有メモリが読めない窓は従来どおり。診断の記録(`[diagnostics] mozc_shm_shadow`)は従来どおりオプトイン。

## 実機での検証と訂正(2026-10-18)
詳細・数字は [adr265-belief-experiments-result.md](../tasks/adr265-belief-experiments-result.md) の追補。実機 dragonflyg4、1 台。

### TTL の実測(tick の間隔)
- 方法: 計測専用のブランチ(`measure/ttl-tick-gap`、マージしない)で、`mozc_shm_on_tick` の呼び出しごとに前回との間隔(ms)を awase.log に 1 行出した。通常の使い方(GJI、**mozc-readable は使っていない**)で約 22 分(UTC 22:25:59〜22:47:52)、625 件。
- 結果: p50 **515ms**、p90 937ms、p95 2688ms、p99 27.6 秒、最大 **525 秒**。500〜599ms が 449 件(72%)。1000ms 超 **55 件**(8.8%)、うち 1000〜1500ms は 11 件(最大 1297ms)、1500ms 超 44 件、3 秒超 30 件。
- **TTL=1000ms(500 + 300 + 200 の導出)は不足**: tick が動いている間でも 1000〜1500ms の間隔が出る(prefetch の待ち 300ms に、タイマーの張り直しとメインスレッドの遅れが重なると見られる。原因の突き合わせはしていない)。1500ms なら、この範囲は覆える。
- 1500ms 超の間隔は、TTL では覆えない。コードの読みでは `reschedule_ime_refresh`(`runtime/mod.rs`)が、効果的に TSF ネイティブな窓と、`explicit_intent` があるときに tick のタイマーを張り直さない(再開はフォーカス変更・IME キーのとき)。ログとの突き合わせは未実施。
- 限界: mozc-readable を使っていない状態の、tick の分布。長い間隔の頻度は利用者の使い方(明示意図・TSF ネイティブの窓)に依存し、mozc-readable を使ったときに同じとは限らない。

### 明示意図が残っているとき、外からの変更に追随するか(D3 の穴)
- 方法: awase を `AWASE_TEST_INJECTION=1` で起動し、マーカー付きの注入キー(物理キー扱い)で漢字キーを 2 回送って明示意図を作り(ログ: `[shadow-toggle] ... step="IntentPromoted" kind="PhysicalImeKey"`、belief は `decided_by ExplicitIntent`)、6 秒待ってから、外から IMM で閉じる。awase.log の `follow external change` と `effective-open-flip` を見る。自前の窓(`ui_host_exp.py`)、Mozc Readable だけ。ドライバー `tools/adr265_stage2/drive_belief_exp.py e5`。
- 結果: **フォーカスが保たれ、外からの閉が実際に効いた有効な 5 周のすべてで、6 秒間、追随しなかった**(belief は `ExplicitIntent` の「開」のまま、IME は閉)。最初の測定(8 周)でも、きれいに測れた 1 周は同じ。フォーカスが他の窓へ移った周は、明示意図が消えるので結論に使わない(14 周中 9 周が該当)。
- **訂正**: 前の「実機で D3 が動いた」(外から 50 回開閉して 51 件追随、drift 0)は、**明示意図が無い状態**の確認で、BUG-200 に近い場面(awase か利用者が開けたあと、ツールバーなどで閉じられる)を含んでいなかった。この場面では、D3 は働いていない。

### D5(提案、未実装、利用者の判断待ち): 共有メモリの読みを refresh の tick から切り離す
- 内容: フォーカスの窓に共有メモリのブロックがあるとき(`NotFound` の記憶がない窓だけ)、専用の軽いタイマー(約 500ms)で `mozc_shm_on_tick` 相当(読み・報告・追随・降格)を動かす。refresh の tick(他の stage を含む)は、これまでどおり止まったまま。
- 理由: refresh の tick は、`explicit_intent` があるとき・効果的に TSF ネイティブなときに、他の stage(ObserverPoll の観測、drift など)を走らせないために止めている。共有メモリの読みだけを切り離せば、その設計に触れずに、追随が動く。
- 実装の見込み: タイマー ID の追加(`TIMER_*`、`handle_wm_timer` の分岐)、`mozc_shm_on_tick` の呼び出し元の付け替え、ガード(`architecture_guard`)の更新。TTL は、この専用タイマーの間隔を実測して決め直す(間隔が安定するなら 1500ms 前後の見込みだが、実測してから)。

## 段階
1. 実装(D1〜D4)。テスト: drift の除外、TTL の全読み手への効果、R3' の 2 つの項、追随の 3 手、降格、ガードの更新。回帰は `fix-requires-evidence` の「IME belief」「force-write / actuation」のファミリー。tuning 定数は実測値を本文に。
2. 実機での確認(CI の実機を含む)。journal で、追随・降格の件数と、belief の誤りの件数を数える。
3. Confirmed の便益の測定(D4)。足すなら別の ADR。
