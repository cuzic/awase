# force_tsf(Tsf 注入モード)を使わず Vk で送った場合の CI 測定(measure-no-force-tsf)

2026-10-22〜23。所有者の問い「`force_tsf` ももはや不要では」を、GitHub の Windows CI で測った。
**撤去の PR は作っていない(判定と根拠のみ)。「悪くならなかった」は証明ではなく証拠である。**

## 結論

- **CI で観測できた範囲では、Tsf → Vk にしても悪くならなかった。** 実 Windows Terminal(runner の `wt.exe`)と、
  窓クラスを Windows Terminal にした自前の RichEdit(`--form=tsfnative`)で、GJI・MS-IME、連続打鍵・cold・12 秒停止・
  打鍵途中 800ms 停止・フォーカス往復を、現状(`-tsf`)と `force_tsf = []`(`-vk`)で並べた。**欠け・リテラル化・入れ替わり・置き換わりは
  両方とも 0 件**(下表)。未確定文字(composition)も両方で同じように作られた(`sc-ntsf-preedit-*`、UIA・IMM とも `composing:か`→`かか`)。
- さらに TsfGate のフォーカス直後の保留も無くしたビルド(`-vknogate`、a14)も 0 件……ただし 1 件だけ FAIL があった(下記)。
  同じ構成を 12 回回し直して再現しなかった。
- **撤去の候補にしてよい**(推定で約 1,400 行)。ただし WezTerm は CI に無く、TsfGate の保留が効く状況(フォーカス直後 ≤500ms の打鍵)は
  1 件も作れていない(下記「確かめていないこと」)。実機 dragonflyg4 の Windows Terminal・WezTerm での確認が要る。

## Tsf が必要だった理由(記録から)

- `1268878d`(2026-06-17)・`4e49ee1`(2026-06-25)・ADR-0004「WezTerm の分類変更を巡る試行錯誤」: 当時の既定が **Unicode 注入**
  (`KEYEVENTF_UNICODE`)で、WezTerm・Windows Terminal では IME の composition を通らず **漢字変換(候補窓)が出なかった**。
  `force_tsf` はそれを VK 経由にするための指定だった。送信順も一度 Sequential にしたが `41d8fee` で Chrome と同じ重畳順へ戻している。
- ADR-276(2026-10-21)で既定が Vk になったため、**元の理由(Unicode が composition を迂回する)は既に消えている。**
  残る違いは、cold 時の確認機構(Tsf: `ColdWarmupSequence`+`GjiWarmupCoro`、Vk: `TsfProbeCoro::new_chrome`)、warm の送信関数
  (`send_romaji_as_tsf_warm` と `send_romaji_batch_immediate`)、TsfGate の Probing/Ready 遷移、`probe_fsm.rs` の `is_tsf_mode` 分岐(`needs_literal` 等)。
- BUG-001(WezTerm の cold で 1 文字目がリテラル)・BUG-024 は Tsf 側の cold 機構の記録。今回の cold・12 秒停止の構成はこの症状を見に行った。

## 方法

- ブランチ `ci/measure-no-force-tsf`(`493f61ae`、挙動不変。e2e の構成と `typing_stress --form=tsfnative`〈`ci/dd202-reach` から〉だけ)。
- 構成: `e2e-ime.yml` の `cfg(..., ftsf=...)` で `dist/config.toml` を書き換える。`add`=`force_tsf` に `typing_stress.exe`+`CASCADIA_HOSTING_WINDOW_CLASS` を足す
  (自前窓を Tsf にする、名前の末尾 `-tsf`)/`keep`=そのまま(実 WT は既定の設定で Tsf)/`clear`=`force_tsf = []`(`-vk`)。ビルドは同じ。
  `-vknogate`(名前は `tsx-ntsfng-*`)は `clear` に加えて a14(`Output::on_focus_change_tsf` を即 return、TsfGate を PendingWarmup にしない)のビルド。
- 効いたことの確認: `awase.log` の `KeyInput(tsf)` と `KeyInput(batched)` の件数が、`-tsf` ではすべて tsf、`-vk`・`-vknogate` ではすべて batched。
  実 WT の `[focus-sync] class="CASCADIA_HOSTING_WINDOW_CLASS" ... hint=ForceTsf → mode=Tsf`(`-tsf`)・`hint=Default → mode=Vk`(`-vk`)。
- run: `39876929142`(tsf/vk 全種・preedit・WT 連続)、`39878331401`(vknogate)、`39879642449`(MS-IME・12 秒停止 ×6。vknogate の 7 ジョブは成果物の名前の誤りで無効)、
  `39880505036`(WT の cold/12 秒/往復、vknogate の MS-IME・12 秒 ×6)、`39882029072`(往復直後に待たずに打つ `-switch0`)。
  再集計: `gh run download <run> -p 'result-*'` → `result.txt` の `TYPING_STRESS:` 行。

## 集計(ジョブ回数・試行数・崩れ = loss+literal+substitute+reorder+extra)

| 入力先 | IME | 現状(-tsf) | Vk(-vk) | Vk+保留なし(-vknogate) |
|---|---|---|---|---|
| 実 Windows Terminal(20ms、連続/cold/12s/往復) | GJI | 10 回・120 試行・0 | 10・120・0 | 10・120・0 |
| 同 | MS-IME | 10・120・0 | 10・120・0 | 10・120・0 |
| tsfnative(40/10ms、連続/cold/12s/停止/往復/往復直後) | GJI | 24・432・0 | 24・432・0 | 22・396・0 |
| 同 | MS-IME | 34・612・0 | 34・612・0 | 34・612・**1 試行 FAIL(24)** |

- vknogate の FAIL 1 件(`tsx-ntsfng-tsfnative-msime-10ms-idle12s-vknogate-1`、run 39878331401): 準備の「か」+Enter の直後に IdleCheck が
  `conv=0x00000000` を読み、belief を `ObservedEisu` にして Engine が非活性になり、最初の試行が全角英字になった。フォーカスから約 4 秒後で、
  TsfGate の保留期間(≤500ms)の外。a14 との因果は**未確認**(同構成を 12 回、Tsf/Vk で各 12 回回して再現 0)。IdleCheck の一過性の読み取りの問題に見える。
- preedit(`sc-ntsf-preedit-tsfnative-*`、各 1 回・6 試行): GJI・MS-IME とも、Tsf・Vk・vknogate で `verdict=OBSERVED uia=READS imm=READS`、中身も同じ。

## 撤去した場合に消えるもの(推定、origin/develop `f5049ff1` で数えた)

`tsf/tsf_gate.rs`(551 行)・`tsf/warmup/gji_warmup_coro.rs`(248)・`tsf/warmup/cold_warmup.rs`(119)、`output/vk_send.rs` の
`send_romaji_as_tsf*`・`send_romaji_as_tsf_warm`・`send_char_as_tsf`(約 240)、`output/sender.rs` の `TsfSender`、`probe_fsm.rs` の
`is_tsf_mode`(13 箇所)、`runtime/mod.rs::settle_tsf_gate_after_refresh`・`key_pipeline.rs` の保留・`TIMER_TSF_GATE`、`InjectionMode::Tsf`・`InjectionHint::ForceTsf`・
`force_tsf` 設定(`src/config.rs`・classifier・設定画面)。関係する名前が出るファイルは 35(`grep -rl` で TsfGate・is_tsf_mode・send_romaji_as_tsf・
GjiWarmupCoro・ForceTsf 等)。合計は約 1,400 行と**推定**。
注意: `Output::send_romaji_dispatching_on_gate`(raw TSF リテラルの回収再送)は gate が Bypass でないと Tsf 経路を使う。Vk の窓でもフォーカス直後の PendingWarmup 中は
ここに入りうるので、撤去時は常に batched にそろえる。`force_tsf` を書いた既存の設定は、`force_vk` と同じく読み込みエラーにせず無視・通知にする。

## 確かめていないこと(未確認)

- **WezTerm**(CI に無い)。実機 dragonflyg4 の WezTerm・Windows Terminal での確認が要る。
- **TsfGate の保留そのもの**: すべての run で `tsf-gate-hold` は 0 件。`typing_stress` はフォーカスを戻してから約 420ms 後に打ち始め、その前に gate が解けていた。
  a14 の比較は「保留が起きない状況で無くしても変わらない」ことしか示さない。
- **変換候補(Space で候補窓を出して確定)**: 観測する構成が無い。代わりに composition が作られることを見た(旧症状〈Unicode で composition を迂回〉の代わりの観測)。
- 実 WT は 20ms・`single,thumb,mixed` 4 試行 × 2 回のみ。runner は 1 種類(同一イメージ)。
