# 窓の種類ごとの GJI 書き込み量の実測(2026-10-24)

擬似窓(擬似 IME の「証拠の出方」の層)のプロファイルの元データ。BUG-215(Zoom のチャットで正しく入った文字を
awase が誤判定して Backspace で消す)の調査から出た。判定は `tsf/probe.rs` の `LiteralDetector`:
打鍵の 300ms(実測では 313ms 時点)後に、GJI 変換サーバー(`GoogleIMEJaConverter`)の書き込みバイトが
基準値から `COMPOSITION_BYTES_THRESHOLD`(350B)以上増えたか、候補窓が出たかで「IME に届いた」と判断する。

## 測り方

`tools/e2e/write_profile/profile.ps1`(`GetProcessIoCounters` で `WriteTransferCount` を 1ms おきに採取)。
IME ON の状態で、子音 `k`(VK 0x4B)と母音 `i`(VK 0x49。`ki` で「き」が完成する)を 1 つずつ打ち、
打鍵から 313ms までの累積バイトと、最初の書き込みが見えるまでの時間を記録する。入力中(未確定)の文字を
`len` 個ためてから測る(「あ」を `len` 回打つ)。`gap` は打鍵前の待ち(3000ms=冷え、200ms=温まり)。
awase は動かさない(止めて生のキーを打つ)。

- 実機: dragonflyg4(GJI)、Zoom のメイン会議ウィンドウ内のチャットタブ(`ConfMultiTabContentWndClass`)。
- CI: `.github/workflows/ime-write-profile.yml`(windows-latest、GJI を chocolatey で導入)、run 40065722169。

## 結果(1 打鍵あたりの書き込みバイト、313ms 時点。同じ条件の繰り返しはどれも同じ値だった)

| 窓 | len | `k` | `i` | 最初の書き込みまで |
|---|---|---|---|---|
| Zoom(実機) | 0 | 407 | 535 | 39〜56ms |
| Zoom(実機) | 12 | 280 | 324 | 43〜61ms |
| メモ帳(CI) | 0 | 170 | 268 | 25〜47ms |
| メモ帳(CI) | 12 | 556 | 606 | 25〜47ms |
| メモ帳(CI) | 30 | 992 | 1098 | 約 31〜37ms |
| Windows Terminal(CI、cmd) | 0 | 157 | 111 | 25〜40ms |
| Windows Terminal(CI、cmd) | 12 | 281 | 325 | 26〜46ms |
| Windows Terminal(CI、cmd) | 30 | 499 | 599 | 31〜37ms |
| Edge(自前の textarea) | - | 測れず | 測れず | IME が ON にならず書き込み 0(実機・CI とも) |

## 読み取れること

- **書き込みの遅れは原因ではない。** どの窓でも最初の書き込みは 25〜61ms で、判定の 313ms に十分間に合う。
- **書き込み量は入力中の文字数 `len` が増えるほど大きくなる**(メモ帳・Windows Terminal)。`len=0` では
  メモ帳 170/268B、Windows Terminal 157/111B と、閾値 350B を**下回る**。つまり「閾値より小さい=IME に届いていない」
  とは言えず、短い入力の最初の打鍵は閾値の手前になる。Zoom の `len=12`(280/324B)は Windows Terminal の
  `len=12`(281/325B)とほぼ同じ値で、Zoom 固有というより GJI 側の性質に見える。
- 例外: Zoom の `len=0`(407/535B)は他より大きい。Zoom が入力欄の周辺の文字列(周囲テキスト)を TSF で
  渡している可能性があるが、未検証。
- 同じ打鍵なのに `k`(子音)と `i`(母音)の大小は窓で逆転する(Windows Terminal の `len=0` では `k`>`i`)。
  単純な「子音は小さい」ではない。
- 結論: 書き込み量の閾値は窓・入力中の長さ・打鍵で大きく変わり、どこに置いても誤判定は残る。BUG-215 の修正は
  窓単位の扱い(Zoom のチャットで 2 番目以降の疑いを信用しない)に留め、閾値 350B は動かしていない。

## 擬似窓のプロファイルに使うとき

- 窓の型ごとに「`len` と打鍵(k/i)から書き込みバイトを返す表」を持たせる。上の表の値をそのまま使う(補間はしない。
  表にない `len` は、測った 2 点の間を直線で結ぶ、と仮定として明記する)。
- Edge は IME ON の前提づくりに別の手段が要る(`chrome_probe` が使う手順を流用する)。
