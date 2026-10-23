# ADR-279 段階 0 の測定結果(2026-10-23)

[ADR-279](../adr/279-drift-episode-refutes-stale-explicit-off.md) §2 の S0-1〜S0-4 を GitHub の Windows CI(`e2e-ime.yml`)で測った。実装(D0〜D4)はしていない。
測定用の変更(ログ・ハーネス・構成)は `ci/adr279-stage0` ブランチだけにあり、develop には入れていない(下の「再現」)。
dragonflyg4 の実機ログ・利用者の不具合報告の journal は読んでいない(S0-4 は CI の journal だけ)。

## 結論(構成ごとの道)

| 構成 | 判定 | 根拠(要点) |
|---|---|---|
| 既存 `sc-driftrecovery-ctrlmuhenkan-{gji-tsf,gji-chrome,msime-tsf}` | **(a0)** | Ctrl+無変換の OFF が効き、ずれを作れない(`GAP_NOT_MADE` 10/10 ずつ) |
| 既存 `sc-driftrecovery-ctrlmuhenkan-msime-chrome` | **(a2)** | ずれが 10/10 で残る(`NOT_CORRECTED`)。awase の観測は古いものだけ(`drift_eval=StaleObservation`)、Engine は OFF のまま。BUG-185(MS-IME × 実 Chrome の OFF)の既知の制限 |
| 新設 `adr279-s0-msime-{role,norole}-*`(MS-IME) | **(a2) かつ (b)** | ずれは 3 秒以上続き、awase は観測しない(a2)。ただし右親指(変換)1 打で、今の経路だけで 10/10 直る(b) |
| 新設 `adr279-s0-gji-{role,norole}-*`(GJI) | **(a2)** | ずれは 3 秒以上続き、awase は観測しない。親指 1 打では日本語に戻らない(役割あり: 実 IME も閉じて OFF でそろう / ATOK プリセット: 逆向きのずれができる)。親指と文字の重なりなら、今の経路で 10/10 直る |

- **(a1) に当たる構成は無い**(ずれを作れた構成ではすべて 3 秒以上続いた)。
- **(c) に当たる構成も無い**。D2 は「S0-1 で一致率を確かめた High の観測」を要するが、ずれの間の `derive_any` は全構成で `HighSingle` が 0%(観測が 1 件も来ない)で、根拠にできるソースが無い(ADR §2 の (c) の条件「一致率を確かめたソースがある場合に限る」を満たさない)。
- §5 により、(a0)・(a2) が残るうちはバルーンを撤去しない。**次に選ぶ経路**: 所有者へ報告し判断を仰ぐ(ADR の (a2) の扱い)。
  - MS-IME は (b): 「変換を押せば直る」を利用者がどう知るか(ADR §7 の未確認事項)を所有者が決める。
  - GJI と MS-IME 共通の (a2): ずれの間に観測が来ない理由は、**明示意図があると IME の読み直し(ポーリング)が止まる**こと(`runtime/mod.rs::reschedule_ime_refresh`、下の S0-1)。導線(D0〜D4)の前提(High の観測)が無いので、別の手段(本 ADR の範囲外)を検討する。

## 測定の条件

- run: **39901109196**(`ci/adr279-stage0` の `376689ca` のバイナリ。workflow は `eb9dc8d9`。全 30 ジョブ success)。S0-3 の再実行は **39902395375**(`3a007ddc`)。
- 窓: `typing_stress --form=tsf`(RichEdit を `Chrome_RenderWidgetHostHWND` へスーパークラス化した自前の窓)。awase の分類は **`Imm32Unavailable`**(TsfNative ではない)。
- ずれの作り方(新設 `adr279-s0-*`): VK_IME_ON で開く → マーカー付き物理 Ctrl+無変換で awase が OFF(明示 OFF、`IntentStore` の source=`Command`)→ 400ms 後、awase のフックが自己注入として素通しする目印(`INJECTED_MARKER`)付きの VK_IME_ON で**実 IME だけ**開け直す(明示 OFF・実 ON = BUG-110 と同じ向き)。+0.5/2/3.5/5 秒でハーネスが `ImmGetOpenStatus` を読み、awase は 200ms ごとに belief の内訳を `[adr279-diag]` に出す。各構成 10 試行。
- 役割: GJI の役割あり = CUSTOM 表で無変換・変換を開閉のトグル(awase の導出 `role=Some(Toggle)` をログで確認)。GJI の役割なし = ATOK プリセット(`role=None`)。MS-IME の役割あり = レジストリ `KeyAssignmentMuhenkan/Henkan=2`(awase は `Some(Toggle)` と導出。MS-IME 自体の挙動は役割なしと同じだった)。MS-IME の役割なし = 既定。

## S0-1: ずれが続くか、そのとき何が観測されているか

| 構成 | 試行 | gap_made | 3 秒以上続いた | `HighSingle` の時間の割合 | 一致率 | drift の発火 | DriftGiveUpDiagnostic |
|---|---|---|---|---|---|---|---|
| `adr279-s0-*`(16 構成) | 160 | 160 | **160**(+5 秒でも実 IME=ON、belief・Engine=OFF) | **0%**(`derive_any=None`、観測 0 件) | 算出不能(対象 0) | 0 | 0 |
| `sc-driftrecovery-ctrlmuhenkan-msime-chrome` | 10 | 10 | 10(`NOT_CORRECTED`、+2 秒まで) | 0%(`StaleObservation`) | 算出不能 | 0 | 0 |
| `sc-driftrecovery-ctrlmuhenkan-{gji-tsf,gji-chrome,msime-tsf}` | 30 | 0 | — | — | — | — | — |

- `none`(親指を打たない)構成の最後の打鍵(+約 5.5 秒)は「ｗ」(生キーが ON の IME に入る。BUG-110 の症状と同じ)で 40/40。
- 観測が来ない理由: ずれの間、awase の IME refresh が 1 回も走らない(`[stage-observe]` が 0 行)。明示意図があると `reschedule_ime_refresh` がポーリングを止める。drift correction は観測が無いので `NotDrifting` のまま発火せず、バルーンも出ない。

## S0-2: Engine 非活性中の親指 1 打(各 10 試行、結果は 10/10 そろった。1 行だけ例外を注記)

| IME / 役割 | 右親指(変換) | 左親指(無変換) | 親指と文字の重なり |
|---|---|---|---|
| GJI / あり | 生キーが GJI に届き実 IME が**閉じる**。明示 OFF は残る(OFF でそろう)。次の打鍵は「w」 | 同左 | 意図が外れ(`ModeKeyPassedThrough` の読み直し→`DeriveMedium(ObserverPoll)`)、実 IME=ON・Engine=ON。次の打鍵「か」 |
| GJI / なし(ATOK) | 実 IME は**閉じる**が、`KeyEffectPredicted` が ON と予測し Engine=ON → **逆向きのずれ**。次の打鍵は空(文字が消える) | 同左 | `KeyEffectPredicted` で Engine=ON、実 IME=ON。次の打鍵「か」 |
| MS-IME / あり | 実 IME=ON のまま、`ModeKeyPassedThrough` で意図が外れ Engine=ON。次の打鍵「か」(**直る**) | Engine=ON だがカタカナに切り替わる(次の打鍵「カ」) | Engine=ON・カタカナ(「エガリダ」) |
| MS-IME / なし | 同上(直る) | 同上(カタカナ) | 同上(1/10 は先頭が「ｗ」) |

- ADR §2 の予想(役割ありは awase が `SetOpen` を書き、右親指で明示 ON)とは違い、どの構成でも親指は生キーとして IME に届き(`mode-key-follow PassThrough`)、awase の `SetOpen` は出なかった。
- drift correction は判定の間も OFF を送り続けるはずだった(ADR round 2 n2)が、この測定では観測が無く発火 0 だったので、awase 自身の OFF で閉じた試行は 0。

## S0-3: `LangDetectRecord.evidence` の分布

- 既存の CI の journal(`e2e-ime`・`e2e-ime-smoke` の 71 run、2026-10-19〜10-23、awase.log 3,648 本): **全 3,648 本が「影の判定は無効」**(n-gram の表が dist に無く `lang_detect` が `Off`)。LangDetect は 0 件。
- n-gram を置いた再実行(run 39902395375、`ts-{tsf,edit}-{gji,msime}-40ms`×2 と `adr279-s0-*-overlap` の 12 ジョブ): 記録は 3 件(すべて `trigger=idle`・`verdict=undecided`・`stop=non-material-key`)、**`thumb-overlap` は 0 件**。
- コード上も、親指=無変換/変換は `excluded` で、武装後の最初の打鍵が親指だとエピソードが始まらない(`src/lang_detect.rs::maybe_start`)、途中なら `ExcludedKey` で閉じる(`feed`)。ADR §1.3 #6 の予想どおり。
- 実利用の journal は読んでいない(リポジトリの `tests/journals` にも LangDetect は 0 件)。

## S0-4: DriftGiveUpDiagnostic の件数(CI の journal だけ)

同じ 71 run・3,648 本で 49 件(49 本)。

| 向き | 件数 | 構成 |
|---|---|---|
| 明示 ON・実 OFF(Q5 で扱わない向き) | 41 | `cal-driftrec-*`(30)、`sc-driftrecovery-gji-tsf`(10)、`sc-follow-chrome-atok-ime-on`(1)。いずれもハーネスが実 IME を直接閉じる構成 |
| **明示 OFF・実 ON(本 ADR の向き)** | **8** | `sc-eisu-km-chrome-msime`(5)、`sc-eisu-km-tsf-msime`(3)。ずれの継続 64〜102 秒 |

- 物理キーの意図(0xF3 等)が発端の区間の数は、CI の構成がすべてハーネスの操作でずれを作るため、数えていない(実利用の journal が要る)。
- 実利用の件数は**未測定**(指示により実機ログ・不具合報告は読んでいない)。(a1) の条件「実利用でほぼ 0」は判定できない。ただし S0-1 で (a1) の前提(ずれが続かない)が崩れているので、判定には影響しない。

## 限界

- 窓は自前の `Imm32Unavailable` の窓だけ。**Windows Terminal(InputSite、TsfNative)は測っていない**(`typing_stress` の drift 系は自プロセスの窓でだけ動く。TsfNative ではポーリングは明示意図の有無に関わらず止まるので、観測が来ないことは同じ見込み、未確認)。
- 明示 OFF の source は `Command`(Ctrl+無変換)。BUG-110 の発端の `PhysicalKey`(0xF3)の経路は通していない。
- 実 IME を開け直す手段(自己注入の目印付き VK_IME_ON)は人工のもので、実機で何が IME を開けたままにするかは別の問題。

## 再現

- ブランチ `ci/adr279-stage0`(origin/develop `ff68fe0f` から)。コミット: `376689ca`(`[adr279-diag]`・`[adr279-role]` ログ、`typing_stress --drift-reopen-hidden/--drift-checks/--drift-thumb`、`adr279-s0-*` の 16 構成)、`eb9dc8d9`・`3a007ddc`(n-gram を dist に置く、構成の絞り込み)、`06819e75`(集計 `tools/e2e/ime_key_matrix/adr279_s0.py`)。
- 実行: ブランチへの push で `e2e-ime` が動く(ONLY はブランチ名で固定)。集計: 各ジョブの artifact `result-adr279-s0-*` を落とし `python adr279_s0.py result-adr279-s0-*`。
- develop にはこれらの変更を入れていない([one-off-verification-workflows](../../.claude/rules/one-off-verification-workflows.md) の撤去対象は無い)。
