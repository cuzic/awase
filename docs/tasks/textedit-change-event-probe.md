# UIA `AddTextEditTextChangedEventHandler` が composition に発火するかの実測

状態: **完了（2026-11-21）結論: 不採用**

着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch で実施
（`ci/textedit-change-probe` ブランチ）。

## 背景

literal-detect（`crates/awase-ime-core/src/tsf/literal_facts.rs`）は「composition として
確定したか（literal か composition か）」を、`GetProcessIoCounters` の `WriteOperationCount`
差分や `candidate_visible` 等の**間接証拠**から推測している。UIA の `ITextEditProvider`
（`IUIAutomation3::AddTextEditTextChangedEventHandler`、`TextEditChangeType::
Composition`/`CompositionFinalized`/`AutoCorrect`）が実際の composition に対して
**直接**発火するなら、間接推測を置き換える一次証拠になりうる、という仮説を検証した。

## 方法

`crates/awase-windows/examples/spike_textedit_change_probe.rs`（撤去済み、下記参照）が、
自前の top-level window に `EDIT` と `RICHEDIT50W`（`Msftedit.dll`）の2種類の子コントロールを
順に生成し、どちらにも `AddTextEditTextChangedEventHandler` を `Composition`/`AutoCorrect`
の両方で登録。登録後、(1) ASCII を `SendInput` で打つ健全性チェック、(2) MS-IME を
ひらがなへ切り替えて実際に composition を起こすキー列（`k`,`a` → Enter で確定）を送る、
の2段を行い、コールバックの発火回数・種別・運ばれてきた文字列を記録した。

MS-IME の有効化は `.github/workflows/e2e-ime.yml` の `matrix.cfg.ime == 'msime'` ステップと
同じ手順（`Set-WinUserLanguageList` に `ja-JP` を追加 → TIP 登録を最大5分待つ → 必要なら
`Add-WindowsCapability` → `Set-WinDefaultInputMethodOverride` → `ctfmon` 再起動）を
`.github/workflows/textedit-change-probe.yml`（撤去済み）に複製して使った。
windows-latest ランナー3回（run 44880805810 → 44881127870 → 44881401555）。

## 結果

1回目（run 44880805810）: `CoCreateInstance(&CUIAutomation, ...)` で作った `IUIAutomation` を
`IUIAutomation3` へ `.cast()` した時点で `E_NOINTERFACE`（`No such interface supported`）。
**legacy の `CLSID_CUIAutomation` は `IUIAutomation2` 以降を実装しない**
（MSDN: `IUIAutomation2`〜`6` を使うには `CLSID_CUIAutomation8` でインスタンス化する必要が
ある）。`CUIAutomation8` に差し替えて再実行。

2回目・3回目（run 44881127870・44881401555、再現一致）:

```
# form=edit register composition=Ok(()) autocorrect=Ok(())
# form=rich register composition=Err(Error { code: HRESULT(0x80004002), message: "No such interface supported" }) autocorrect=Err(Error { code: HRESULT(0x80004002), message: "No such interface supported" })
# done
```

- **RICHEDIT50W（`Msftedit.dll`）**: 登録そのものが `E_NOINTERFACE` で失敗する。この
  コントロールの既定 UIA プロバイダーは `ITextEditProvider` を実装していない
  （MSDN のサンプルで RichEdit が使われるのは `ITextProvider`/`ITextRangeProvider`
  — テキスト**選択**の読み取りパターンであり、`ITextEditProvider` とは別物）。
- **EDIT**: 登録自体は `Ok(())`（Composition・AutoCorrect 両方）で成功するが、
  ASCII・IME 切替後のローマ字「ka」・Enter による確定のいずれでも、**`TXTEDIT` 行は
  1回も出力されなかった**（=一度も発火しなかった）。登録 API がエラーを返さないことは
  プロバイダーが実際にそのパターンを実装していることを保証しない
  （Win32 の汎用 UIA ブリッジは、サポートしないイベント種別の登録要求もそのまま受理し、
  単に一度も発火しないという形で失敗する）。

## 結論

**不採用。** `ITextEditProvider` はオプトイン実装のパターンであり、自分で完全に制御できる
最も有利なケース（素の Win32 `EDIT`・`RICHEDIT50W`）でさえ実装されていない
（`RICHEDIT50W` は登録自体が失敗、`EDIT` は登録は通るが一度も発火しない）。
literal-detect が実際に観測したいのは Chrome・VS Code 等の**awase が制御できない
サードパーティ製アプリ**の composition であり、自分で作った・制御できるコントロールでさえ
対応していない以上、サードパーティ製アプリで対応している可能性はさらに低いと判断する
（ADR-261 の「UIA 不可・MSAA 可」の知見と同種の、UIA パターン対応がアプリ依存に偏る
事例の一つ）。この方向で `TextEditTextChanged` を literal-detect の一次証拠に使う投資は
見送る。

自前で `ITextEditProvider` を実装したホスト（`text_store_probe.rs` の自前 `ITextStoreACP`
と同じ発想）を作れば発火はできるが、それは**自分の窓の中でしか使えない**ため、
literal-detect が本来必要としている「他プロセスの入力欄を観測する」という目的には
そもそも効かない。追加検証の価値は無いと判断し、ここで打ち切る。

## 撤去

結果がこの文書に出たため、`.claude/rules/one-off-verification-workflows.md` に従い
1回限りの検証物を撤去した。撤去コミット: `3edf5e1e2`
（`git show 3edf5e1e2^:.github/workflows/textedit-change-probe.yml` /
`git show 3edf5e1e2^:crates/awase-windows/examples/spike_textedit_change_probe.rs` で戻せる）。

`ci/textedit-change-probe` ブランチの ref 自体は撤去対象外（このルールの方針どおり、
ブランチの掃除は持ち主が決める）。
