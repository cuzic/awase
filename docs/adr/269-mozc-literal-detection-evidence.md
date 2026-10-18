---
id: ADR-269
title: |-
  Mozc 系(Mozc Readable)では GJI の I/O を literal 判定の証拠にしない(安全網を外す)。Mozc 用の証拠は観測してから決める
summary: |-
  BUG-207。Mozc Readable で awase が入力直後の文字を消す。literal 判定の証拠(GJI の変換サーバーの書き込み量)が GJI 専用なのに、
  GJI の変換サーバーが同時に動いていると静止した GJI を見て誤判定する。一度目の修正(監視を mozc_server に向ける)は、
  Mozc 単独環境に判定を新しく有効にして tsf で壊した(CI 実測、Opus レビュー round 1)。Mozc 系では GJI の監視を外す(案 E)に改め、
  Mozc 用の証拠(書き込み量・候補ウィンドウ・共有メモリ)は、判定窓ごとの生の証拠を観測してから決める。
status: |-
  実装中(2026-10-18)。段階 1(Mozc 系で GJI の監視を外す)は PR #686。段階 2 以降(観測、Mozc 用の証拠)は未着手。実機は未確認。
related_adr:
  - "ADR-265"
  - "ADR-267"
  - "ADR-268"
  - "ADR-019"
---

# ADR-269: Mozc 系では GJI の I/O を literal 判定の証拠にしない

## 背景

[BUG-207](../known-bugs/BUG-207.md): Mozc Readable を入力方式にすると、awase が入力直後の文字を消す。実機ログ(2026-10-18 23:04、dragonflyg4)に `[raw-tsf-literal] ... suspected → backspace + re-send`、`gji_io=18860ms`、`GoogleIMEJaConverter` と `mozc_server` の同時稼働がある。

`raw-tsf-literal` の判定(`output/probe_io.rs`、`tsf/probe.rs::LiteralDetector`)は、キー送信の前後で次のどちらも起きなければ「リテラル」とする。
1. 変換サーバーの累積書き込み量 `TSF_OBS.gji_write_bytes` が 350B(`COMPOSITION_BYTES_THRESHOLD`、`tsf/probe.rs:625`、Chrome + GJI で実測)以上増える。
2. 候補ウィンドウ `GoogleJapaneseInputCandidateWindow` の SHOW イベント(`win_event_obs.rs`)。

両方とも GJI 専用。さらに判定は `gji_is_active_ime()`(= `gji_monitor_ok` かつ `tsf_active_kind == GJI 系`、`observer.rs`)のときだけ走る(`probe_fsm.rs:160,681` の `env.gji_active`)。**証拠が取れない IME では安全網を外す**、が既存の規律。

## 実測(GitHub Actions、windows-latest、e2e-ime の `ime=mozc`)

| 構成(各 2 回) | 一度目の修正(監視を mozc_server へ、run 39175765485) | 修正前 develop(Mozc 単独、run 39177357191) |
| --- | --- | --- |
| ts-tsf-mozc-{10,40}ms | **INVALID ×4**(`ready` 3 回失敗、疑い 1・諦め 2) | **PASS ×4**(監視は付かず、`raw-tsf-literal` 0 件) |
| ts-edit/rich-mozc-* | PASS(判定は走っていない。下記) | PASS |

- tsf の失敗は IME ON の失敗ではない。`typing_stress.log` の `ready`: 試行 1・2 は `ime_open=true` で text が空(組成が Escape/BS で消された)、試行 3 は `turn_ime_on(2)` が押す VK_CONVERT(0x1C)を Mozc が ON と扱わず、`[mozc-shm] follow external change open=false` で belief が正しく降格し `w` が素通りした。`mozc_server` の書き込みは 1 VK あたり約 0.2〜0.3KB で 350B に届かない。
- **一度目の修正は Mozc 単独環境に判定を新しく有効にした**: develop では GJI の変換サーバーが居ないと監視が付かず、`gji_active` が偽で判定が走らない。修正で `mozc_server` に付くと `gji_monitor_ok` が真になり、判定が走り始めた。
- edit・rich は awase が Unicode(`send_keys: mode=Unicode`)で送るので literal 判定が一度も走らない。「誤判定なし」と「WRITE の 350B 以上が 10ms で約 85%・40ms で約 75%」は、閾値が足りている証拠にならない(per-VK の romaji 送信とは入力経路が違い、10ms のサンプル差分で判定窓とも揃わない)。
- 「tsf の WRITE が 16 件」は入力を受けていない証拠ではなく、ready の数文字・IME キー・Enter だけが打たれたから(INVALID で本番の打鍵は 0 試行)。
- CI の tsf は入力欄が `Chrome_RenderWidgetHostHWND` を名乗り、awase は Chrome の cold-start 経路(`chrome-per-vk-literal`)に入る。実機は `setopen-per-vk-literal`(`TransmitTarget::Tsf`)で段取りは違うが、`LiteralDetector::check_now` と `run_per_vk_confirm` は共有しており、閾値の問題は両方に効く。
- 付随: ts-edit-mozc-10ms-1 の `ready` 試行 1 は `mozc_server` の遅延起動で 1 文字目が落ちた疑い(literal 判定と無関係、awase との関係は未確認、U4)。

## 決定

### D1. Mozc 系では GJI の監視を外す(案 E、段階 1、PR #686)
`converter_target`(コア、純関数): GJI 系かつ `TipIdentity::Gji` でない TIP は `ConverterTarget::None`、それ以外は `Gji`。`gji_monitor` は `None` のとき GJI の監視を外し(`gji_monitor_ok=false`)、`Gji` のときだけ接続する。判定は走らない(Mozc 単独環境の develop と同じ挙動)。GJI 系でない TIP(MS-IME・ATOK)と Google 版 GJI は変わらない。

**根拠**: Mozc で本物の raw-tsf-literal が起きた記録は 1 件もなく、観測したのは誤判定だけ(BUG-207・CI)。誤判定の害(Escape が組成中の文字を全部取り消す。`literal_detect_fsm.rs:89-92`。2 回目以降は BS×1 で諦める)は、未検出の害(リテラルが見える形で残り、打ち直せる)より大きい。

### D2. Mozc 用の証拠は、観測してから決める(段階 2、未着手)
次の選択肢は、判定窓ごとの生の証拠(`DetectEvidence`: `write_delta`・`write_ops_delta`・`show_changed`・`evidence_fresh`、`probe.rs:632-667`)を Mozc と GJI の両方で観測してから評価する。リテラル側のサンプルが 0 件の今は、閾値を導出できない(`tuning-constants.md`: 閾値を変えるなら実測値を本文に。`COMPOSITION_BYTES_THRESHOLD` は `tuning.rs` ではなく `probe.rs` の `const` なので、規約の精神として適用する。Mozc 用に別の値を足すなら `tuning.rs` に `#[measured]` 付きで置く)。

- **A. 書き込み回数・量**: Mozc はモードキーでも `w_ops=+1〜2`(`+0.0KB`)なので、回数では「受けた」と「組成した」を分けられない。分けられそうなのはバイト数(モードキー 0.0KB、文字キー 0.2〜0.4KB)。F2 の先行バッチの IPC が per-VK の 1 つ目の窓に入るので 1 つ目は汚れる。
- **B. `MozcCandidateWindow` の SHOW**: fork の `src/base/const.h:118`(`cuzic/mozc-readable` の `v3.34.6239-readable.1`、tag 386580f4 の `src/base/const.h` で確認)。子音 1 VK では候補が出ないので、per-VK の 1 つ目は救えない。`gji_candidate_visible_now`(`probe_fsm.rs:387`)も同じクラス名に依存する。
- **C. 共有メモリ(ADR-267 version 3 の `phase`)**: IME 自身の値。反映の遅れは F2 から約 230ms(CI、tsf)、edit で一瞬 false に落ちて 1ms 後に戻る例があり、per-VK の deadline(約 300ms)と同じ桁。遅れの分布を先に測る。awase は version 2 までしか受け入れない(ADR-265 D3)。
- **D. 判定しない**: D1 がこの扱い。

## 既知の限界(D1 の後も残る)

- TIP 切替後、デバウンス(2 秒ポーリング × 2 連続、最大約 4 秒)の間は旧ターゲットを見続ける。Google 版 GJI → Mozc では、その間だけ BUG-207 の症状が出うる。「アプリごとに入力方式を切り替える」設定では前面アプリの TIP と一致しないことがある。
- 監視を外す/付けるとき、`gji_write_bytes` はプロセスをまたいで不連続(`gji_monitor.rs`)。送信前にベースラインを取った `LiteralDetector` が切替をまたぐと、消す(新しい累積が小さい)/二重入力(`gji_last_write_ms` が古いまま `StaleConfirm`)のどちらも起きうる。GJI の再起動(`GJI process exited, will re-attach`)でも前からある跳び。D1 は Mozc 切替では監視を外して `gji_monitor_ok` を偽にするので、跳びを使う経路は絞られるが、ゼロではない。
- `converter_target(true, MsImeNative)` は起こり得ない組み合わせで `Gji` を返す(黙って)。`GJI_CLSID`(`OnceLock`)を起動時に取りこぼすと、Google 版が `(GoogleJapaneseInput, Other)` → `None` になり、GJI だけの環境で判定が無効になる(family と clsid は同じ列挙と同じ照合なので可能性は低い)。

## 実測・検証の順序

1. 修正後の Mozc 単独 CI(`ts-tsf-mozc-*`)で、develop と同じ PASS に戻ること(PR #686 の CI)。
2. **GJI と Mozc を両方入れた構成**(BUG-207 の実際の条件)を `e2e-ime` に足し、develop(誤判定が出るはず)と修正後(出ない)を比べる。現行の CI は GJI なしで、この条件を作れていない。
3. 観測のみの PR: `DetectEvidence` を判定ごとに `awase.log` へ出し、Mozc と GJI の tsf で分布を取る(GJI の子音 1 VK も 350B を越えない〈`probe_fsm.rs:366`〉ので、GJI の per-VK は SHOW と候補可視の近道に支えられている、という仮説もここで確かめる)。
4. Mozc で本物のリテラルが起きるかを、awase なしで測る(`ts-raw-tsf-mozc-*` に cold・idle の揺さぶりを足す)。起きないなら、Mozc に安全網は要らない(D1 で確定)。
5. 実機(dragonflyg4)で、Mozc Readable だけにして文字が消えないこと、`[raw-tsf-literal]` が出ないこと(日本語入力が止まるため、利用者に予告して許可を得てから)。

## 記録の修正(ハーネス側)

- `typing.json` の `cfg.ime` と `result.txt` の見出しが `gji` になる(`e2e-ime.yml` の `ts-*-mozc-*` の `stress=` に ime の指定が無い)。ラベルの誤りで、checker の振る舞いへの影響は未確認(U5)。
- 観測した構成が限られる(ts-raw-tsf-mozc は `10ms-1` を精査、他は件数のみ)。

## 結果

(実測後に追記する。)
