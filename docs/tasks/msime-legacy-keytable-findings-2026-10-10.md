# MS-IME 互換モード旧UIキー表の実測(CI スパイク、2026-10-10)

ブランチ `ci/msime-legacy-keytable-spike`。windows-latest(26100)・MS-IME 本体・互換モード ON(`NoTsf3Override2=1`、`DisableNewIME=1`)。
ハーネスは `ime_key_matrix_spike --auto --msime --seq=...`。各セルは n=1(再現回数の裏取りは未実施)。

## 観測(CI 1 環境・各セル n=1。閉じた状態の実験だけ信頼できる)

1. **`keystyle=Custom` のとき、閉じた状態ではレジストリの表(S4key)が効く。名前付きスタイルの `key` が開いた状態で効くかは未確認(M3)。** 名前付きスタイル(NATURAL/MS-IME2000/ATOK/VJE/WX)の `key` を書き換えても無視される(内蔵の表が使われる)。
   名前付きスタイルの切り替えは `IMJPUEXC.EXE setkeytemplate <Microsoft_IME|IME_Standard|ATOK|VJE|WX>`。
2. **IME が閉じている間のキーは `Custom\S4key` が決める**。`key` だけを書いても閉じた状態では無反応。
   ADR-197 は閉じた状態で `key` だけを書いて検証した(本質の誤りは `key` の 1 列目を「直接入力」と読んだ列の誤読)。ND の ON/OFF は閉じた状態の `S4key` の結果を変えなかったが、`key` の 1 列目の `D5` など、コードによっては ND で効果が変わる(ADR-254 実測6)。`StyleList\Custom` の他の値(`S1key`〜`SEkey`)は NATURAL のコピーを置いた。
3. **`key` の列は旧UIの列見出し(No Input・Only Input・Converted・Showing・Changing・Char Input)と1対1。1列目は「開いている・入力なし」**(ADR-197 の「1列目=直接入力」は列の誤読)。
4. **UI の Apply が書く値**(`setkeytemplate` と同じ): `MSIME\keystyle`・`IMEUserName`・`option2`・`SerialNo` など。`SerialNo` の更新や ctfmon 再起動は効果に無関係(V2/V3)。
5. 表の形式: 行は `<キー名>=<コード1> … <コード6>`(Shift-JIS、NUL 区切り、終端は NUL が 1 つ多い)。

## 機能コードの効果

**開いた状態の列は無効**(ハーネスの ESC が未確定の `ｋ` を消せず、「入力中」で測っていた。ADR-254 の Opus r1 B1)。閉じた状態(`Custom\S4key` の「無変換」行)の列だけを参照する。
また、閉じた状態で「IME が開く」コードが大半なのは、コード固有の意味ではなく「S4key に行がある=IME が閉じている間にキーを横取りし、まず開いてから機能を実行する」ためと読める(r1 M1)。実際の S4key に現れるのは `87`・`CE` だけ。

| コード | 閉じた状態で押す | 開いた状態で押す |
| --- | --- | --- |
| 00・80・FF | 変化なし | 変化なし |
| B3 | 変化なし | conv 0x19→0x10(半角英数) |
| A2・A4・D0 | conv→0x10(閉じたまま) | A2・A4 は conv→0x10、D0 は変化なし |
| CD | open 0→1 | conv→0x10 |
| CE(トグル) | open 0→1 | 変化なし |
| D5 | conv→0x10(閉じたまま) | **open 1→0(閉じる)** |
| 81〜89・8A〜8E・92・99・AC・AD・B0・B1・B7・C1〜C7・CA・CC・D1〜D3・F0・F1・F5〜FA・FE | open 0→1 | 多くは変化なし(B0・B1・F0・F1 は未確定文字を消す、81〜83・87・88 は未確定文字へ文字を足す) |
| 95・97・98・9A〜9C・A1・C9 | open 0→1 かつ conv 変化(0x13/0x01/0x18 など) | conv 変化 |
| 94 | 不定(open が `?`) | 不定 |

(旧「読み方」の段落は、無効な開いた状態の掃引を根拠にしていたので削除した。ND の効果と、IME が閉じるコードの話は ADR-254 の実測5〜10 を参照。)

## 未検証

- 開いた状態の表は `key` の 2〜6 列目(入力中・変換中など)が別の状態に対応するはずだが、列と状態の対応は未検証(今回は 6 列を同じコードで埋めた)。
- 「直接入力モードを使用しない」OFF で、`key` の 1 列目が使われる場面があるか(コード掃引は ON のみ)。
- 実機の UI 保存(`Custom` が `S4key` を含めて書かれるか)。UI の Advanced ダイアログは構造だけ採取(`Settings` ダイアログ、ReportView・Assign・Modify・Remove・Template コンボ)。
- 修飾付き(Ctrl/Shift)のキー、他のキー(英数・カタカナ・半角/全角・F キーなど)。

## 実装への含意

本ファイルの観測の最新の読み方と実装方針は ADR-254(`docs/adr/248-msime-legacy-custom-keytable-read.md`)を参照。ここには、閉じた状態の観測と、スパイクの手順だけを残している。
