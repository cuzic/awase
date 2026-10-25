# ADR-280 C0: `eisu_threshold` の配線確認(GitHub Windows CI、2026-10-24)

固定の合成列だけを打つ。利用者の本文・打鍵列は含まない。PR #828、ブランチ `ci/adr280-c0-wiring`。

## 構成

e2e-ime.yml の `sc-langdetect-eisu14` / `sc-langdetect-eisu16`(GJI + awase.exe の debug ビルド、`AWASE_TEST_INJECTION=1`、`RUST_LOG=debug`)。
config は `half_width_alnum_toggle = "all"`、`[auto_detect] mode = "ime_on_eisu"`・`scope = "all"`・`eisu_threshold = 14.0 / 16.0`。
自前の EDIT 窓に `typing_stress --mode=langdetect` が、IME ON にそろえて 3.8 秒の無操作のあとに列を打つ(各 2 回)。判定は `check_langdetect.py`
(awase.log の `[lang-detect]` の journal 行を、打鍵の時刻で切り出す)。配線は起動時の観測行 `[lang-detect] config:` の実効の閾値で見る。
前提の n-gram 表は dist に無いと判定器が無効になる(初回 run 40080328944 は `[lang-detect]` が 0 件で INVALID。`dist\data` を置いて解消)。

## 列と期待

配列の定義済みキー数 N で 1 打ごとに `+ln N` が乗る(nicola.yab は 44、CI の既定 nicola_keytop.yab は 47)。

| 列 | 内容 | llr の推移(N=44 / N=47 の最終) | 閾値 14 | 閾値 16 |
|---|---|---|---|---|
| B 境界 `ayre` | 親指なし・Space なし | 1.5741, -4.6360, -8.5266, **-14.9978** / **-14.734** | 4 打目で NotJapanese | 決まらない(Undecided) |
| E 英数 `build` | 同上 | -6.4998, -15.6293, -19.4536(N=47: -6.434, -15.497, -19.256) | 2 打目で NotJapanese | 3 打目で NotJapanese |
| J | `k` を押して左親指(無変換)を重ねる | — | (期待は Japanese。下記のとおり CI では成立せず) | 同左 |

Rust の判定との一致: `tests/lang_detect_eval.rs::eisu_threshold_boundary_sequence_flips_between_14_and_16`(両配列で固定)。

## 実測

### 配線(両構成とも確認)

run 40087340285(head bd5e8aed)・40092243328(5339b903)とも、awase.log の
`[lang-detect] config: mode=ImeOnEisu scope=All en_threshold_effective=14 eisu_threshold=14 ...`(eisu16 は `=16`)。
`eisu_threshold` は実際の awase.exe の判定器の閾値(`max(eisu_threshold, en_threshold)`)まで届く。

### B・E(run 40087340285・40092243328・40094716805 で同じ。各 2 回とも同値)

| 構成 | 列 | verdict | llr | 打鍵数 | stop | action |
|---|---|---|---|---|---|---|
| eisu14 | B | not-japanese | -14.734 | 4 | decided | skipped-explicit-on |
| eisu14 | E | not-japanese | -15.497 | 2 | decided | skipped-explicit-on |
| eisu16 | B | **undecided** | -14.734 | 4 | timeout(2500ms) | none |
| eisu16 | E | not-japanese | -19.256 | 3 | decided | skipped-explicit-on |

期待どおり: 同じ境界列が、14 では英数と決まり、16 では決まらない。llr は Python 再現の N=47 の値と一致(-14.734、-15.497、-19.256)。

### J(CI では確かめられない)

- run 40087340285(親指を先に押す): `[lang-detect]` の記録 0 件。awase.log では 無変換 Down → k Down が PendingThumb → 「ぎ」で解決。
  仮説(未証明): 親指の Down が無操作の長さをリセットし、k がエピソードの開始(idle)にならなかった。
- run 40092243328(k を先に押して親指を重ねる): k Down でエピソードが始まり(llr 2.898、1 打)、無変換 Down で
  `stop="excluded-key" verdict="undecided"` として閉じた。無変換が IME 関連のキー(`is_ime_key`)として除外されるためと見られる
  (読みであり、**`is_ime_key` による除外だと示す該当のログ行は確認していない**。この構成は keymap=1 の ATOK プリセット)。親指の重なりによる Japanese 判定は CI の構成では成立しない。
- 扱い: J は合否に含めず、チェッカーは記録を情報として表示するだけにした。重なりでの日本語確定の確認は、親指に IME 機能が無い構成が要る(未確認)。

### 限界

- action は B・E とも `skipped-explicit-on`(eisu16 の B は `none`)。各試行の前の `turn_ime_on`(IME OFF→ON のキー)で明示意図 ON が立ち、
  書き込みが見送られた。**判定(verdict)は確認できたが、半角英数トグルへの実際の入りは確認していない。** 表の「入る/入らない」は verdict の意味。
- 窓は自前の EDIT 1 種類(IMM)。他の窓の種類・実 Chrome 等は未確認。

## CI

- 第 2 回: e2e-ime run 40087340285(head bd5e8aed)、ジョブ `e2e (sc-langdetect-eisu14 …)` 120853318857・`e2e (sc-langdetect-eisu16 …)` 120853318891。判定 FAIL(B=2/2 E=2/2 J=0/2)。
- 第 3 回: run 40092243328(head 5339b903)、ジョブ 120867673720・120867673735。判定 FAIL(同上、J のみ不合格)。
- 最終: run 40094716805(head c2703dcf、チェッカーの合否を B・E だけにした版)。ジョブ `e2e (sc-langdetect-eisu14 …)` 120874537522・`e2e (sc-langdetect-eisu16 …)` 120874537558。
  各ジョブの result.txt(artifact `result-sc-langdetect-eisu{14,16}-1`)の最終行:
  `LANGDETECT: verdict=PASS eisu=14.0 effective=14.0 B=2/2 E=2/2 J=0/2`、`verdict=PASS eisu=16.0 effective=16.0 B=2/2 E=2/2 J=0/2`。
  B・E の中身は上の実測の表のとおり(ジョブの success ではなく result.txt の記録で確認)。J は情報のみ(0/2)。

## 結論

`[auto_detect] eisu_threshold` は実際の awase.exe の判定器の閾値に届き、同じ境界列(`ayre`)が 14 では NotJapanese、16 では決まらない(Undecided)ことを実機(GJI、自前の EDIT 窓)で確認した。
未確認: 半角英数トグルへの実際の入り(`skipped-explicit-on`)、親指の重なりでの日本語確定(J)、自前の EDIT 以外の窓。
