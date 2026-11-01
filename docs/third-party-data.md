# 第三者のデータの出典と帰属

awase に同梱する**統計データ**(判定用の表)の元になった、第三者のデータの出典と帰属を書く。依存クレート(Rust)のライセンスは `Cargo.lock` とそれぞれのクレートに従い、ここでは扱わない。
**法的な助言ではない**。解釈が確定していない部分は、そう書く(「継承条件の扱い」)。

## 同梱するデータと、元になったもの

| 同梱するもの | 元のデータ | 元のライセンス | 表に含まれるもの |
|---|---|---|---|
| `data/ngram_hiragana.csv.gz` | 日本語 Wikipedia の CirrusSearch ダンプ(Sudachi + UniDic で形態素解析) | CC BY-SA 4.0(Wikipedia のコンテンツ) | ひらがなの 2 連・3 連の頻度。文は含まない |
| `data/latin_bigram.txt` | このリポジトリの `*.md`・`*.rs`、`/usr/share/dict/american-english`(Debian の `wamerican`、SCOWL)、`/usr/share/locale/<言語>/LC_MESSAGES/*.mo`(GNU・GNOME 等の翻訳カタログ。各パッケージの GPL/LGPL) | 左のとおり | 格子の 45 文字上の 1 連・2 連の遷移確率(整数)。単語・翻訳文は含まない |
| `data/lang_lr_v1.txt` | 日本語 Wikipedia・英語 Wikipedia(Hugging Face `wikimedia/wikipedia` の 20231101 のスナップショット、各 1 シャード)、Tatoeba(jpn・eng・fra・spa・deu・ita・por)、このリポジトリのファイル | Wikipedia: CC BY-SA 4.0。Tatoeba: 多くが CC BY 2.0 FR。リポジトリ: 著作権者が同じ | 約 11.5 万個の int8 の重み。文・単語・キー列は含まない |
| `tests/fixtures/lang_lr_vectors.tsv` | 上の学習データの test 分割から取ったキー列(最大 12 打)と、その累積値 | 同上 | キー位置の列(86 件)。文は含まない |

`tests/fixtures/lang_detect_eval.tsv` は、このリポジトリの n-gram 表と手書きの語から作った合成データで、外部のコーパスを含まない。

### `data/lang_lr_v1.txt` の作り方
1. 日本語は Sudachi(mode C、辞書は `SudachiDict-core`)で読みを付け、NICOLA の未シフト面で打てる仮名が続く区間のキー列にする。ラテン文字は 1 語 = 1 エピソード(最大 12 打)。
2. キーの 1〜3 連(開始記号つき)のロジスティック回帰を学習し、int8 に量子化する。
3. 手順とコードは `tools/lang-detect-model/`(`README.md`)。学習データ自体はリポジトリに置かず、実行時に元のサイトから取得する。
- Sudachi(`sudachipy` 0.7.0)と `sudachidict-core`(20261015)は学習の道具で、配布物には含まれない。パッケージのメタデータは Apache-2.0。辞書の元データ(UniDic など)の条件は SudachiDict の記載に従う(**この文書では未確認**)。

## 帰属(利用者に示す文)

- **Wikipedia**: 日本語 Wikipedia・英語 Wikipedia のコンテンツ(Wikipedia contributors)から作った統計量を含む。コンテンツは
  [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/deed.ja) で提供されている。<https://ja.wikipedia.org/> <https://en.wikipedia.org/>
- **Tatoeba**: Tatoeba プロジェクトの文(多くは [CC BY 2.0 FR](https://creativecommons.org/licenses/by/2.0/fr/))から作った統計量を含む。
  各文の著者は Tatoeba に記録されている。<https://tatoeba.org/>
- **語彙・翻訳**: `data/latin_bigram.txt` は、SCOWL の単語表(`wamerican`)と、GNU・GNOME 等の翻訳カタログの英字の遷移から作った統計量を含む。

## 継承条件の扱い(解釈が確定していない部分)

- Wikipedia の CC BY-SA 4.0 には継承条件(ShareAlike)がある。**統計量(頻度・重み)が元のコンテンツの「翻案」にあたり、継承条件が及ぶか**は、この文書では確定させない。
  既存の `data/ngram_hiragana.csv.gz`(日本語 Wikipedia 由来)も同じ状況で、同じ扱いにしている。
- 現状の方針: (1) 出典と帰属を明記する(この文書)、(2) 表に元の文・単語を残さない(頻度・遷移確率・重みだけ)、
  (3) 元のコンテンツ自体は再配布しない、(4) 疑義が出たら、該当の表を取り除く・作り直す。
- このリポジトリのコードは Apache-2.0 または MIT(`LICENSE-APACHE`・`LICENSE-MIT`)。上の表のライセンスは、コードのライセンスとは別に付く。

## 配布物への反映(未了)

- [ ] MSI・Scoop の配布物に、この文書(またはその要約)を同梱する。
- [ ] アプリ内の「バージョン情報」から、帰属を見られるようにする。
- [ ] リリースノートに、第三者のデータの追加(LR の表)を書く。

これらは配布物の変更なので、リリースの前に別の PR で行う(`release-develop-to-main`)。
