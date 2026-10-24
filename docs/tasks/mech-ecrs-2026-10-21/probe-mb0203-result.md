# MB-02・MB-03 の到達プローブ結果(warm パスの LiteralDetectFsm・TsfGate の Probing)

2026-10-22。`INDEX.md` の MB-02(warm パスの `LiteralDetectFsm`)と MB-03(`TsfGate` の `Probing`)に本番で届くかを、
GitHub の Windows CI(`e2e-ime.yml`)に目印ログを入れた版で回して数えた。
**「0 件」は証明ではなく証拠である**(e2e の構成が触れない経路は、0 件でも到達しないとは言えない)。

## 結論

- **MB-02: 撤去する**(PR #803)。設置の条件(`TsfGate` が Probing かつ GJI かつ long idle でない かつ非 Tsf モード)は
  1 度も成立しなかった。warm パス(`send_romaji_as_tsf_warm`)に入ったのは Tsf モードの窓(Windows Terminal)だけで、
  4,800 回すべて `gate=Ready`・`is_tsf_mode=true` だった。撤去版で同じ構成を回し、e2e の判定の差は 1 回だけの反転
  (両方向、撤去したコードが動かない構成)に限られた。
- **MB-03: 今回は撤去しない**。`Probing` は `plan_literal_detect` の読み以外に、`BypassConfirmed` を受けて `Bypass` へ移る
  (`Ready` は無視する)という違いを持つ。この違いが効く場面(`Probing`/`Ready` 中の `BypassConfirmed`)は 0 件だったが、
  統合したときにどちらの動きに寄せるかは決めておらず、統合版の e2e は回していない。次の候補として残す。

## 方法

- プローブ: `ci/probe-mb0203`(`fe73fa89` が目印、`a57c053a` が Windows Terminal の構成追加。挙動不変、PR にしない)。
  目印は `tracing::info!("[reach-probe] ...")` で、文字・キーの内容は出さない(判断・状態・条件だけ)。
  - `MB-02 warm plan=<plan> gate=<TsfGate> is_tsf_mode gji_active long_idle`(warm 送信ごと)と `MB-02 INSTALL`。
  - `MB-03 gate->Probing from=<state>`・`Probing->Ready`・`Probing->Bypass`・`Probing->PendingWarmup(focus)`。
- 撤去版: `ci/probe-remove-mb0203`(`a7ba8032`。撤去 PR のコード `c4665aad` + 同じ構成追加 + 目印。PR にしない)。
  目印は `MB-02 warm(removed) would_install=<旧 plan なら Install だったか>` と `MB-03 BypassConfirmed in=<Probing|Ready>`。
- Windows Terminal: `tools/e2e/input_forms/forms.toml` の `windows-terminal` は `imes = []`(CI に出さない)なので、
  プローブ版だけ `imes = ["gji", "msime"]`・`intervals = [20, 60]`・`runs = 4` にした(`tsx-ext-windows-terminal-*`、16 ジョブ)。
  e2e の `config.toml`(リポジトリの既定)は `WindowsTerminal.exe` を `force_tsf` に持つので Tsf モードになる。
  **これを入れないと、e2e-ime の既存構成では Tsf モードの送信が 1 度も起きない**(`ts-tsf-*`・`tsx-tsf-*` の「tsf」窓も
  `send_char_as_vk` の Vk モードで打たれる)。
- 集計: artifact `result-*/dist/awase.log` を `grep reach-probe`(ジョブログには出ない)。PASS/FAIL は各 `result-*/rc.txt`・`inv_rc.txt`。
  確かめたコマンド: `gh run download <run> -p 'result-*'` → `grep -rah reach-probe --include=awase.log . | sort | uniq -c`。

## 回した run

| run ID | ref | 構成(only) | ジョブ(artifact) |
|---|---|---|---|
| 39856072996 | probe-mb0203 | A: `ts-tsf-gji-{40,10,2}ms`・`ts-tsf-msime-{40,10}ms`・`tsx-tsf-*`・`tsx-chrome{page,bar}-gji-20ms-cold*`・`tsx-edit-*`・`tsx-rich-*`・`tsx-bugreport-*`・`tsx-bug171-tsf-*`・`tsj-tsf-*`・`sc-reopen-tsf-*`・`sc-driftrecovery-*`・`sc-eisu-km-tsf-*`・`ts-tsf-mozc-{cold,idle}*`・`ts-chromepage-mozc-cold*`・`tsx-chromepage-gji-30ms-idle*`・`atok-passthrough-cold*` | 252 |
| 39859843954 | probe-mb0203 | B: `tsx-ext-windows-terminal-*`・`sc-*` | 218 |
| 39864821372 | probe-remove-mb0203 | A と同じ | 252 |
| 39867728435 | probe-remove-mb0203 | B と同じ | 218 |

注: プローブ版は `origin/develop` `6a1e2d1e`、撤去版は `2ec217c4` の上にある(間に他の撤去 PR が入った)。判定の差には
ベースの差も混ざりうる。

## 目印の集計

| 目印 | A | B | A(撤去版) | B(撤去版) |
|---|---|---|---|---|
| `MB-02 warm ...`(warm 送信) | **0** | 4,800 | **0** | 4,800 |
| うち `gate=Ready is_tsf_mode=true gji_active=true long_idle=false`(GJI) | — | 2,400 | — | 2,400 |
| うち `gate=Ready is_tsf_mode=true gji_active=false long_idle=true`(MS-IME) | — | 2,400 | — | 2,400 |
| `MB-02 INSTALL` / `would_install=true` | **0** | **0** | **0** | **0** |
| `MB-03 gate->Probing`(from PendingWarmup / from Bypass) | 183 / 5 | 123 / 75 | 208 / 7 | 133 / 75 |
| `MB-03 Probing->Ready` | 0 | 16(Windows Terminal の各ジョブ 1 回) | 0 | 16 |
| `MB-03 Probing->PendingWarmup(focus)` | 156 | 155 | 138 | 151 |
| `MB-03 Probing->Bypass`(プローブ版)/ `BypassConfirmed in=Probing|Ready`(撤去版) | **0** | **0** | **0** | **0** |
| raw recovery の再送(`[raw-tsf-literal] re-sending`) | 72(すべて gate=Bypass の一括送信) | — | — | — |

読み:

- warm パスに入るのは Tsf モードの送信だけで、そのときは `is_tsf_mode=true` なので `plan_literal_detect` は必ず `Skip`
  (`gate=Ready` なら `GateNotProbing`、`Probing` でも最後の項 `TsfMode` で落ちる)。Tsf モードの最初の送信は cold(`prepend_f2_warmup`)で、
  段末に `Probing->Ready` になってから warm の送信が来る。
- 非 Tsf モードで `send_romaji_as_tsf` に入る入口は raw recovery の再送(`send_romaji_dispatching_on_gate` の非 Bypass 側)だが、
  観測した 72 回はすべて gate=Bypass で一括送信(`send_romaji_batched_bypass_gate`)へ行った。
- MS-IME(`gji_active=false`)と GJI の long idle も設置の条件から外れる。

## 撤去前後の e2e 判定

構成ごとの PASS/FAIL・不変条件(`rc`/`inv_rc`)の回数を並べた(`cmp.py` 相当)。

- A(252): 差は 5 構成で各 1 回。撤去版で悪化 3(`ts-chromepage-mozc-cold-20ms` の全角英数化 1、`tsx-tsf-gji-20ms-cold` の
  先頭 1 文字の欠落 1、`tsx-bug171-tsf-gji-30ms-pause8000` の不変条件 1)、改善 2(`sc-reopen-tsf-gji-henkan-gap600`・
  `tsx-bugreport-msime-30ms-pause320`)。どれも A で warm パスに入らない構成(撤去したコードが動かない)。
- B(218): 差は 12 構成で、awase を動かさない対照(`*-noawase`)の反転・不安定と既知の `sc-startup-msime-chrome-on-*`・
  `rc=3`(INVALID)の増減・`sc-dbe-gji-msime-drv` の 1 回(probe-reach-result.md でも単発で出た構成)。
  **Windows Terminal の 16 ジョブ(撤去したコードが動いていた唯一の構成)は前後とも 16/16 PASS**(各 12 試行、fail=0)。

## 未確認

- 実利用の journal・不具合報告での到達(e2e の構成外)。WezTerm は CI に無い(Windows Terminal で代用)。
- raw recovery の再送が非 Bypass(Tsf モード以外で gate が PendingWarmup/Probing/Ready)のときの経路。72 回は全部 Bypass だった。
- MB-03 の統合(`Probing` と `Ready` を 1 つにする)の e2e。`BypassConfirmed` を受けるか無視するかを決めてから回す。
