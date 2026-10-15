---
id: ADR-261
title: |-
  観測の無い窓の IME 状態を、タスクバーの入力インジケーターの字形から読む(IME キー学習で字形の表を作り、表に当たったときだけ証拠にする)
summary: |-
  報告 01M4QSFH1B(一太郎・GJI、issue #623)の「belief が実機とずれる」「英数を押してもずれが解消しない」は、窓内の IMM が読めない窓(観測の無い窓)では、
  ADR-260(読みが効く窓の英数トグル)では直らない。タスクバーの入力インジケーターの 32×48 の画面矩形の字形ハッシュが、GitHub Actions 上で実 IME の状態に追随した
  (GJI は 6 モード全部が別ハッシュ、本物の MS-IME は 5 つ〔半角英数=閉〕。実 Chrome・tsf でも追随、遅れは中央 0.18 秒・最大 0.68 秒)。
  ハッシュは環境(DPI・テーマ・IME)依存なので、内蔵表ではなく、IME キー学習(ADR-195/196)で「既知の状態のときの字形」を環境指紋つきで記録して表を作る。
  実行時は観測の無い窓でだけ、表に当たったときだけ、低い確からしさの証拠として読む(当たらなければ何もしない、IME へは書かない)。
  段階1 は belief に入れず journal・不具合報告にだけ出して一致率を見る(shadow)。フォーカス切替後の遅れ・コスト・環境差などは未測定(D0)。
status: |-
  起草(2026-10-15)。実装なし。Opus レビュー前。測定は使い捨てのスパイクブランチ `exp/mode-key-recheck`(develop にはマージしない)の GitHub Actions による。
related_adr:
  - "ADR-148"
  - "ADR-165"
  - "ADR-188"
  - "ADR-191"
  - "ADR-195"
  - "ADR-196"
  - "ADR-227"
  - "ADR-229"
  - "ADR-259"
  - "ADR-260"
---

# ADR-261: 観測の無い窓の IME 状態を、タスクバーの入力インジケーターの字形から読む

## ステータス

起草(2026-10-15)。実装なし。ADR-259(PR #624)・ADR-260(PR #626/#627)とは独立に書いているが、`docs/adr/index.md` の同じ位置に行を足すので、
後からマージするものに軽い衝突が出る。測定は `exp/mode-key-recheck`(HEAD `9cf0ae0f`)の run 38717579288・38761351107・38810058235・38810649639・38811194999・
38811705842・38812641962 による。

## 背景

不具合報告 `01M4QSFH1B02DQ9B8AXBYVRQ39`(GJI・一太郎、issue #623)に 2 つの問題がある。

1. 実 IME の状態と awase の belief がずれる。app log: 別アプリで IME OFF にして一太郎へ移ると、`HwndCache: restore ime_on=true (141688ms ago)` で古い ON を復元し、
   `FocusProbe: ime_on=true(shadow)` と belief が ON のまま。一太郎(`taro36.exe`/`JsTaroCtrl`)は `[imm-learning] 降格: Standard → Imm32Unavailable`
   (`ImmCapability::Unavailable`)で、窓内の IMM が読めない。journal の観測は `ObserverPoll` 1 件のみ(`FocusChanged.profile` は降格前の `ImmCross` を記録するので、journal だけでは読み違える)。
2. 英数を押しても入力モードのずれが解消しない。所有者は 2 を重く見ている(「英数を押してモードずれが解消しないことの方を問題視」)。

[ADR-260](260-eisu-toggle-no-mode-prediction-when-observable.md) は、窓内の IMM の読みが**効く**窓(`tsf` フォーム・実 Chrome。プロファイルは `Imm32Unavailable` でも
`[direct-follow]` が約 25ms で追随する)で、英数トグルの入力モードを予測しないようにして 2 を直す。**観測の無い窓**(報告の一太郎のように
`ImmGetDefaultIMEWnd` が NULL で読み自体ができない窓)には効かない(ADR-260 の測定で、観測の無い窓を模した `noread` は従来と変わらなかった)。#623 はこの窓の問題であり、
ADR-260 だけでは閉じない。

観測の無い窓で awase が頼れる情報は、打鍵時の予測(ADR-191)・`HwndCache` の古い値・間接観測(GJI プロセス I/O、`ObservationSource::GjiIoInference`、`ObservedEisu→AssumedRomaji` の
一方通行のみ)だけで、**実 IME の現在の状態を直接読む手段がない**。ADR-191 の実測では、GJI の開閉・変換モードは TSF スレッド単位で保持され、窓を離れている間に変化しない。
つまり「窓を移ったときの推測が悪い」より、離れる時点の belief が窓内の操作と逆位相になっていることが多い。直接読めれば、推測ではなく証拠で直せる。

### 着想と、所有者の判断

Windows のタスクバーは入力インジケーター(「あ」/「A」など)を描いており、これは別プロセスの TSF スレッドの状態を反映する。画面上の矩形を読む案は所有者が出した
(「画面上の矩形を使うアイデアはありませんでした。脆いというのはそうだけど、今のどうしようもない状況が少しでも改善できるんだったら使っていい」)。さらに、
字形と状態の対応は「**IME キー学習で作る**」案(所有者)、読む場面は「**観測の無い窓だけ。マウス操作・Alt+Tab・タイピングのときに、抑止機構つきで**」(所有者)。
本 ADR はこの 2 つを設計に落とす。

### 実測(GitHub Actions windows-latest)

- **UIA の Name・HelpText・LegacyIAccessible は状態では変わらない**(`IME is disabled`/`Right click to show IME options` の 2 値が、フォーカスとプロファイルの有効化で替わるだけ)。
  読めるのは画面上の字形だけ。
- **字形ハッシュは実 IME の状態に追随する**。インジケーター要素(`SystemTray.NormalButton`、名前 `Tray Input Indicator…`)の矩形 32×48 を取り込み、
  背景との差が閾値を超える点で 2 値化して FNV でハッシュした。
  - **GJI(run 38761351107、自前の窓 26 押下 + キー列 61 押下 × 2 プリセット)**: 6 モード全部が別のハッシュ。ひらがな `d49577`・カタカナ `385e42`・全角英数 `069551`・
    半角カナ `9812cd`・半角英数 `0bbfa6`・閉 `def643`。MS-IME プリセットでも ATOK プリセットでも同じハッシュ(字形は IME の入力モードの表示で、キー配置に依らない)。
    全角英数と半角カナは通常のキーでは届かないので、GJI のカスタムキー表で `InputModeFullAlphanumeric`/`InputModeHalfKatakana` を F5〜F9 に割り当てて作った。
  - **本物の MS-IME(run 38810058235、IMM の書き込み 30 + キー注入 36)**: 別のハッシュ体系。ひらがな `eb8708`・カタカナ `c6b2e9`・全角英数 `37fb64`・半角カナ `490507`・閉 `691729`。
    **本物の MS-IME では半角英数と閉が同じ状態・同じ字形**(半角英数を書くと閉になる)。区別できるのは 5 つ。本物の MS-IME は IMM の書き込みが効く(GJI では効かない)。
  - **観測の無い窓でも追随する**: 実 Chrome×本物の MS-IME 34 押下で字形は API の開閉と全件一致、tsf フォーム×本物の MS-IME 18 押下も全件一致。
    実 Chrome×GJI カスタム表(run 38811194999・38811705842・38812641962)は ひらがな 9/9・カタカナ 3/3・半角カナ 3/3・全角英数 3/3・半角英数 2/3
    (1 件は閉の字形、タイミングの疑い)。
  - **遅れ**: 押下から字形が最初に変わるまで、中央 0.18 秒・最大 0.68 秒(60 件、run 38717579288 の時系列)。
- **環境依存**: 矩形は GJI の run で (822,720)–(854,768)、本物の MS-IME の run で (778,720)–(810,768) と、トレイの並びで位置が変わる(UIA で毎回探す必要がある)。

### 測定で踏んだ罠(同じ失敗を繰り返さないために記録する)

- **2 値化を「平均より明るい点」だけにしたため、明るい背景に暗い字形(本物の MS-IME の黒い「A」)が全部空白になった**。その結果「GJI は区別できるのは 3 クラス、
  カタカナ・全角英数・半角カナ・閉は空白で同一」「本物の MS-IME では動かない」と**誤結論を出した**(報告済み)。画面を実際に保存して見て(`--shots`)発覚し、
  `abs_diff(mean) > 20` に直して 6 モードが分かれた。**結論を出す前に画像を見る**。
- IMM(`WM_IME_CONTROL`)で変換モードを書いても、GJI では読み戻しは変わるが**字形は動かない**(GJI の内部状態と食い違う)。状態はキーか GJI のカスタムキー表で作る。
- 1 回目の集計で「ずれ 16/93」と見えたのは、押下から 1.8 秒後の字形を取ったため、keymatrix のセットアップが約 1.3 秒後に状態を戻す変化を拾ったノイズ。
  時系列で見ると字形は押下の 0.1〜0.2 秒後に追随していた。
- `ime_key_matrix_spike` の `--seq` は、フックが認識するキー(F5 等は無し)しか記録しない。本物の MS-IME で F5/F6(ローマ字)を押すと入力欄のフォーカスを奪い、以後の IMM 読み取りが失敗した。

## 決定

### 決定 0: 原則 — 書かない・推論しない・表に当たったときだけ

字形は**実 IME の状態を読む証拠**であり、IME へは何も書かない(actuation を足さない。ADR-212 の方向、所有者の「窓に移ったときの強制書き込みはやめる」と整合)。
**表に無いハッシュ・環境指紋が合わないとき・矩形が見つからないとき・フォーカスがずれたときは何もしない**(否定的な証拠から閉を推論しない、ADR-227 の所有者方針)。
「空白=閉」のような決め打ちは置かない。

### 決定 1: 観測 — `observer/` の新 probe と、`state/` の純粋な分類

- `observer/`: UIA でインジケーター要素の矩形を探す(ブロックしうる COM なので `run_with_timeout`、300ms)→ 画面の矩形を GDI で取り込む → 明暗の両側で 2 値化(背景の平均との差の絶対値が閾値超)→ FNV ハッシュ。
  戻り値は `GlyphSnapshot { hash, rect_size, taken_at, focus_gen }` だけ(画素は保持しない)。
- `state/`(OS 非依存の awase-windows-core、Linux でテストする): `classify_glyph(&GlyphTable, &Env, &GlyphSnapshot) -> GlyphReading`。
  `GlyphReading` は `Hit { open, mode_class }`・`Miss(理由)` の 2 種(理由: 指紋不一致・表に無い・衝突で無効・矩形サイズ違い)。判断はここだけに置く
  (観測側に `if` を書かない。`.claude/rules/ime-belief-architecture.md` の Observe → classify → reduce)。

### 決定 2: 表は IME キー学習で作る(内蔵表は置かない)

- 学習(`awase-keymap-learn-win`、ADR-195/196)は、各状態に到達して settle した時点で (開閉, 変換モード) を既に知っている。そこへ**字形ハッシュの記録を相乗り**させる
  (学習の巡回は IME の状態をキーで作るので、GJI で IMM が効かない問題は避けられる)。
- 表は `GlyphTable { env: EnvFingerprint, ime: TipId, entries: [(hash, open, mode_class)] }`。同じハッシュが異なる状態に割り当てられたら、そのハッシュを**無効**にする(衝突は使わない)。
- 学習でキーが届かない状態(GJI の全角英数・半角カナ。カスタムキー表を持たない利用者)は表に入らず、読んでも `Miss` になるだけで害はない。
- 永続化は学習表の付帯として保存し、版を上げる(`awase-keymap-learn` の persist。詳細は段階 1 の設計で詰める)。ハッシュは GJI と本物の MS-IME で体系が違うので、`TipId`(CLSID+profile)ごとに持つ。

### 決定 3: 環境指紋 — 学習時と違う環境では使わない

`EnvFingerprint` = DPI スケール、インジケーター矩形のサイズ、タスクバーの背景の明暗(テーマ)、OS ビルド。**学習時と一致しないときは表を使わない**(再学習を促す)。
理由: ハッシュは DPI・テーマ・矩形で変わる(GJI と MS-IME 本体で矩形が違う実測)。環境が変わっても「同じハッシュだから同じ状態」と思い込むと誤った証拠になる。

### 決定 4: 使う窓 — 観測の無い窓だけ

学習済みの `ImmCapability::Unavailable` で、`direct-follow`/`idle-conv-check` が効かない窓に限る。読みが効く窓では使わない(直接の読みが優先。二重に解釈しない。
ADR-260 の範囲と重ならない分担)。

### 決定 5: 読む場面と抑止(数値は D0 の実測で決める)

- 読む場面(所有者案): ①フォーカス変更から settle 後、②その窓の最初の打鍵まわり、③マウス操作・Alt+Tab のあと。字形の遅れ(中央 0.18 秒・最大 0.68 秒)があるので、
  「読んだ時点の字形が、実 IME の最新状態とは限らない」前提で、**打鍵の判断を待たせない**(読んだ結果は次の判断から効く)。
- 抑止: 最短読み取り間隔、同一の `FocusGen` で結果が変わらなければ再読しない、連続 `Miss` でバックオフ、UIA・GDI の連続失敗でそのセッションは停止、
  インジケーターが見つからない(自動非表示・トレイのオーバーフロー等)ときは停止。**具体値は tuning 定数にし、D0 の実測 ms を添える**
  ([tuning-constants](../../.claude/rules/tuning-constants.md)。20→100→200→350ms 型の盲目的エスカレーション禁止)。
- await をまたぐ失効判定は `FocusGen`(`state/focus_gen.rs`)で行い、読んだ時点と適用時点でフォーカスが変わっていれば捨てる。前面窓が対象窓でないときも捨てる
  (字形は前面スレッドの状態を映すため)。

### 決定 6: belief への入れ方 — 2 段階

- **段階 1(shadow)**: belief には入れない。journal と不具合報告(ADR-148 型の allowlist: `glyph_probe: { env_match, hits, misses, reason 別件数, 直近の未知ハッシュ最大 3 }`。
  ハッシュは環境固有で個人情報ではない)にだけ出し、同じ窓の belief/実機の一致率を実機で見る。
- **段階 2**: 一致率が十分なら、新しい `ObservationSource::TaskbarGlyph`(`ObserverReported`、`confidence: Medium`、`state/` の `classify_glyph` を経由)として入れる。
  `resolve_open_at` の優先順位(明示意図 > 打鍵時予測 > `derive_any`〔High 単独か Medium の合意〕> `most_recent_trusted` > desired)のどこに効くかは、段階 2 の別レビューで
  コードに当たって決める(`Medium` 単独は `derive_any` を満たさず、`most_recent_trusted` 経由になる。打鍵時予測が付いている間は予測が勝つので、予測を覆さない ADR-260/188 の fence と整合させる必要がある)。
  **本 ADR では段階 2 の配線は決めない**(段階 1 の一致率が出てから)。入力モードは `InputModeObserved`(Medium 以上のみ `input_mode` を上書き)の規約に従い、
  `classify_*` を経由する。観測を偽装した内部補正にしない。

### 決定 7: プライバシー・安全

取り込むのはタスクバーの 32×48 の矩形だけ。メモリ上で 2 値化してハッシュだけを保持し、**画素・画像は保存しない**(診断の ASCII アートは学習時と明示の診断モードだけ)。
画面全体は取り込まない(スパイクの `--shots` は測定専用で、製品には入れない)。

## 却下案

- **UIA の Name・HelpText・LegacyIAccessible**: 状態で変わらない(実測)。
- **IMM(`WM_IME_CONTROL`)で読む/書く**: 観測の無い窓では読めない。GJI では書いても字形に出ない。
- **窓に移った最初の 1 回だけ、ひらがな/カタカナ/英数のキーで actuation する案**: 所有者が議論の末に取り下げた(推論・書き込みでなく、証拠を読む方向へ)。
- **ずれたらユーザーに別のキー(MS-IME プリセットのひらがな、半角全角)を押してもらう案内**: 保留。字形が読めれば、案内より先に自動で直せる。残すなら別の小さな UI 変更。
- **内蔵のハッシュ表**: ハッシュは DPI・テーマ・矩形・IME で変わり、GJI と本物の MS-IME で体系も違う。決定 2・3 のとおり学習で作る。
- **OCR・画像認識**: 重く、脆さが増える。2 値化ハッシュで足りることを実測した。
- **`ITfLangBarItemSink`(言語バーのモードボタン)の push 購読**: 字形より正確で脆さが少ない可能性がある。ただし cross-process 到達性は BUG-025 追補4 で「未検証」のまま
  (`spike_langbar_input_mode.rs --watch-langbar`、ADR-138 の放棄の経緯も参照)。**段階 0 で先にこの spike を測り、読めるなら字形の案は置き換える/併用を再検討する**(D0-0)。

## 未測定・リスク(段階 0 = D0。ここを測ってから段階 1 の実装に入る)

| ID | 内容 | 測り方 |
|---|---|---|
| D0-0 | `ITfLangBarItemSink`/`ITfLangBarMgr` で別プロセスの入力モードが読めるか(字形案との比較) | `spike_langbar_input_mode.rs --watch-langbar` を CI の実窓で |
| D0-1 | **フォーカス切替後**に字形が前面窓の状態へ追随するまでの遅れ(状態の違う 2 窓を Alt+Tab) | 2 窓構成の CI、字形の時系列と切替時刻 |
| D0-2 | 一太郎相当(`ImmGetDefaultIMEWnd`=NULL の窓)で前面にしたとき、字形がその窓の TSF スレッドの状態を映すか | 近似の窓を作る(既存の `cache=unavailable` は awase 側の近似にすぎない)。最終は実機(dragonflyg4) |
| D0-3 | プローブのコスト(UIA の探索 + GDI の取り込みが何 ms か、メインスレッドを塞がないか) | ms を計測して tuning 定数の根拠にする |
| D0-4 | 環境差: DPI 125/150%、ダークテーマ、自動非表示タスクバー、複数モニタ、トレイのオーバーフロー(インジケーターが隠れる)、Windows 10 | 構成ごとの字形と矩形 |
| D0-5 | 同じ状態でのハッシュの安定性(マウスのホバー・通知バッジ・時刻の再描画で変わらないか) | 長時間の静止状態でハッシュの揺れを数える |
| D0-6 | 本物の ATOK(GJI の ATOK プリセットではない)の字形 | ATOK 本体が CI で動くか自体が未確認。対象外にして表が無ければ使われない |
| D0-7 | 実 Chrome×GJI の閉(`def643`)と、tsf×GJI のカスタム表の追随(今回は readiness 失敗で測れていない) | カスタム表の構成の前提づくりを直す |
| D0-8 | 実 Chrome×本物の MS-IME で F1 を押すとひらがなの字形のまま(自前の窓ではカタカナ字形)。awase が間に入った影響の疑い | awase 有無の対照 |

リスク: ①字形の遅れの間に別の状態へ変わる(証拠が古い)。②インジケーター位置・見た目が Windows のアップデートで変わる(表が当たらなくなるだけで、決定 0 により害は無い)。
③表に無い状態(全角英数等)は読めない。④UIA の探索が重い窓システムで固まる(`run_with_timeout`・連続失敗で停止)。

## 段階

- **段階 0**: D0-0〜D0-8。D0-0 で `ITfLangBarItemSink` が読めるなら、本 ADR を改訂して置き換える/併用を決める。
- **段階 1**: `observer/` の probe、`state/` の `classify_glyph`、学習への相乗りと永続化、shadow(journal・不具合報告)。実機(dragonflyg4 と報告者環境)で一致率を見る。
- **段階 2**: belief への配線(別 ADR/追記でレビュー)。#623 の本体の解消はここ。
- **段階 3**: 設定画面の「IME キー学習」に字形の学習状況を出す(環境指紋が変わったら再学習を案内)。

## 関連

[ADR-260](260-eisu-toggle-no-mode-prediction-when-observable.md)(読みが効く窓の英数トグル。観測の無い窓は本 ADR)・
[ADR-188](188-tsfnative-conv-only-mode-key-engine-follow.md)(直接観測)・
[ADR-191](191-calibration-experiments.md)/[ADR-195](195-keymap-learn-productization.md)/[ADR-196](196-keymap-learn-truth-priority.md)(IME キー学習)・
[ADR-165](165-tsf-cache-restore-recency-guard.md)(`HwndCache` の復元)・[ADR-227](227-bug074-giveup-as-closed-ime-evidence.md)(推論しない)・
[ADR-229](229-os-independent-core-and-dumb-windows-executor.md)(層の引き直し)・[ADR-259](259-bug-report-custom-keymap-rows-and-learned-cells.md)(不具合報告への学習表の添付)。
BUG-025 追補4(`ITfLangBarItemSink` の未検証)。issue #623。
