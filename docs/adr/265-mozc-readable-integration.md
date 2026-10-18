---
id: ADR-265
title: |-
  mozc-readable の配布・追随と、awase が共有メモリから IME 状態を読む(自動判定)
status: |-
  提案(2026-10-17)。段階 0(配布の自動化と awase.cc の欄)は着手済み、awase 側の読み取りは未実装
related_adr:
  - "ADR-191"
  - "ADR-261"
---

# ADR-265: mozc-readable の配布・追随と、awase が共有メモリから IME 状態を読む

## 背景

awase は、Win32 の IMM32 や TSF の compartment から IME の開閉・入力モードを読めない窓(`Imm32Unavailable`)で、belief を推測に頼っている。ADR-191 §2 の CI 実測で、外部プロセスが TSF compartment からフォーカス窓の IME 状態を読めないことは確認済み。

[cuzic/mozc-readable](https://github.com/cuzic/mozc-readable) は、Mozc の TIP から IME の開閉・モードを名前付き共有メモリへ書き出す fork である。
- 書き出し先: `Local\mozc-readable-ime-<スレッド ID>`(48 バイト、seqlock、`tip_state_publisher.h` の `TipSharedState`)。
- 通知: `Local\mozc-readable-ime-changed`(manual-reset イベントを SetEvent→ResetEvent で pulse)。
- 公式 Mozc とは別 CLSID・別 UpgradeCode・別製品名で、同時にインストールできる。
- 効くのは mozc 系だけ(MS-IME・配布版 GJI には効かない)。全 IME 向けの近似は ADR-261(字形)が担う。

## 決定

### D1. 配布は mozc-readable 側の GitHub Releases、awase.cc は最新版へのリンクだけ持つ

- MSI は `cuzic/mozc-readable` の Release に `v<upstream の版>-readable.<N>` として置く(SHA256SUMS.txt 付き)。
- `docs/index.html` / `index.en.html` の「インストール」に任意項目を置き、Releases API の最新の MSI へ張る(既存の awase の MSI と同じ方式)。awase の Release には同梱しない(版が独立し、署名・ライセンス表記が別になるため)。
- 署名は無し。SmartScreen の警告を明記する。SignPath Foundation は後付けで検討する。

### D2. upstream への追随は「リリースタグ」を基準に自動化する

- `readable-upstream-sync.yaml`: 毎日 `google/mozc` の `X.Y.Z` タグの最新を確認し、新しければ readable の差分をそのタグへ rebase した `sync/<タグ>` を作り、ビルドと検証(`readable-build.yaml`、インストール・共有メモリ・アンインストールの CI)を起動する。衝突したら Issue を立てて止まる(自動解決しない)。
- `readable-release.yaml`: その検証が成功したら Release を公開し、載せ替え中に readable が動いていなければ readable を進める(動いていたら Issue で止め、readable は上書きしない)。
- master 追随(nightly)は採らない: upstream が壊れている日の影響を受け、公開物の品質が日替わりになるため。

### D3. awase は「設定なし」で自動判定する。設定画面は状態表示とリンクだけ

- インストール済みかの判定: 日本語で有効な TIP の列挙(`EnabledJaTip`)に mozc-readable 専用の CLSID があるか。純関数 `identify_*` に足し、Linux で全数テストする(`ime_kind.rs` の `TipIdentity` に variant を足す形)。
- 窓ごとの状態の判定: 前面窓のスレッド ID の共有メモリが開け、`magic`/`version` が一致し、seqlock の読みが 2 回一致し、`update_tick_ms` が新しいときだけ「読めた」。それ以外は従来経路へ戻る(誤って採ると belief を壊すので、読めないことは無かったことにする)。
- belief への入れ方: 観測(Observe)として取り、`classify_*` の純関数を経て `ObserverReported`/`InputModeObserved` へ流す(`ime-belief-architecture.md`)。**信頼度は実機の測定で決める**(測定前は Medium 以下)。`dispatch_event` を直接呼ぶ近道は作らない。
- 通知イベントは「読み直す合図」にだけ使う(pulse なので取りこぼす前提で、起きるたびに前面窓の共有メモリを読み直す)。
- 設定画面: 検出状態(未インストール / インストール済み・共有メモリ読み取り中 / 読めない窓あり)の表示と、ダウンロードへのリンクのみ。ON/OFF スイッチは置かない(自動判定が外れた報告が出たら強制オプションを足す)。

## 未確認(段階 1 の前に実測する)

- Chrome 等の TSF ネイティブ、UWP/AppContainer で、`Local\` 名前空間の共有メモリが読めるか(AppContainer は分離される恐れ)。読めない窓は従来経路へ戻る設計だが、割合を測る。
- 公式 Mozc との first-key(新プロセスの最初のキーが落ちる件)の比較。
- 共有メモリの読み取りの信頼度(上の Medium 以下をどこまで上げてよいか)。
- 変換モード以外の状態(必要になれば `TipSharedState` に追記、version を上げる)。

## 段階

0. 配布と追随の自動化、awase.cc の欄(mozc-readable PR #1、本 ADR と同じブランチの docs)。
1. awase に TIP 同定と共有メモリ読み取りの Observe を足す(読むだけ、belief へは入れず journal に出して実機で一致を測る)。
2. 測定に基づき belief へ流す(信頼度を決める)。設定画面に状態表示。
3. awase.cc の記述「自動で判定します」は段階 2 のマージまで公開しない。
