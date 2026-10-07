# BUG-189 / BUG-190 の実機確認手順(2026-10-06)

[BUG-189](../known-bugs/BUG-189.md)(古い ImmCrossProbe が優先され Engine が一時的に止まる、ADR-233、#538)と
[BUG-190](../known-bugs/BUG-190.md)(一過性の `conv=0` を 1 回の読みで英数モードと採用して Engine が止まる、ADR-238、#542)は、
CI の実機 E2E(GitHub-hosted Windows)では修正を確認したが、**実機(L4、[teardown-verification-guide](../teardown-verification-guide.md) §2)は未検証**。
本書は、実機で確かめる手順と、見るログ・合格基準をまとめる。確認そのものは Windows の実機で人が行う(打鍵と目視が要る)。

## 0. 準備

1. 最新の `develop`(`6aea6e76` 以降)を実機でビルドして起動する(`awase-build` スキル、または `cargo build --release --target x86_64-pc-windows-msvc`)。
2. ログを残して起動する: `set RUST_LOG=debug` のうえ `awase.exe > awase.log 2>&1`(`README.md` のログの取り方と同じ)。
3. IME は **Microsoft IME**(主対象)と **Google 日本語入力**(対照)の両方で、できれば次の入力先ごとに行う。
   - Standard プロファイル(ImmCross)になる入力先: メモ帳(従来型の EDIT)・WinForms/WPF のテキストボックス・Notepad++・LibreOffice Writer・OpenOffice Writer・Java Swing(`tools/e2e/java_forms`)・Flutter(`tools/e2e/input_apps/flutter`)・Qt。
   - 対照(影響が無いはずの経路): Chrome・Windows Terminal(TsfNative)。

## 1. BUG-189(Flutter × MS-IME 等、打鍵の途中で Engine が一時的に OFF になる)

**再現条件(修正前):** フォーカス取得時に ImmCrossProbe(`IME=false`、High)が 1 件記録され、その後 IME を ON にしても `expires_at: None` で残る。
打鍵中は poll の書き込みが止まる(`typing active`)ので、**最後の観測から 3 秒以上、止まらずに打ち続ける**と反転する。

手順:
1. 入力先を開いて、IME を OFF のままフォーカスする(最初の打鍵が IME OFF キー〈無変換等〉でもよい)。その後 IME を ON にする。
2. 休まずに **5 秒以上**、NICOLA で連続して打つ(単打・親指シフト・混在)。これを数回繰り返す。
3. 出力が途中から全角英字(`ｍぎｇ…`)・カタカナになったり、生のローマ字に落ちたりしないかを見る。

ログ: `awase.log` を grep する。
- `[effective-open-flip] → false decided_by=... MostRecentTrusted(ImmCrossProbe)` が **0 件**であること(修正前はここが 26/26 だった)。
- `Engine deactivated (ime=false ... Inactive(ImeOff))` が、IME を実際に OFF にしていない時間帯に出ないこと。
- `[mrt-shadow]` が出た場合は、`new=true(ObserverPoll,...)` が `old=false(ImmCrossProbe,...)` に勝っている行で、直後の poll と一致していること。

## 2. BUG-190(MS-IME × 一過性の conv=0、本物の英数切替が壊れていないこと)

### 2a. 本物の英数切替が追随する(修正で壊してはいけない側)
1. MS-IME で IME ON(ひらがな)にして NICOLA で数文字打つ。
2. **半角英数**に切り替える(英数キー、または Ctrl+無変換、Shift 単独タップ〈設定次第〉、言語バーのマウス)。それぞれ別に試す。
3. 英数に切り替えた直後の数打鍵が、かな(NICOLA)に変換されずに半角英字のまま出ること(Engine が止まること)。
4. ひらがなへ戻して、NICOLA が再開すること。

ログ: 切替ごとに `[eisu-adopt] decision=candidate` → 約 60〜120ms 後に `decision=confirmed`(または予測が英数なら即確定で `[eisu-adopt]` が出ない)。
`[eisu-candidate] outcome=confirmed age_ms=...` の `age_ms` が **数百 ms 以内**であること(長いと、切替の直後の打鍵が数文字かなのまま出る)。
**予測が効かない切替(言語バー・マウス・アプリの `ImmSetConversionStatus`)で、確定が遅れすぎないか**を特に見る(ADR-238 の「代償」)。

### 2b. 一過性の conv=0 で Engine が止まらない
1. 打鍵を始める直前の「間」(打鍵後 0.5 秒以上空けた後)に、すぐ打ち始める、を長時間繰り返す(数分〜十数分)。
2. 出力の先頭が生のローマ字・全角英字になる回が無いこと。

ログ: `[eisu-adopt] decision=candidate` が出ても、次の `[eisu-candidate] outcome=cleared age_ms=...`(0x19 に戻った)で終わり、
`Engine deactivated (... reason=Inactive(NotRomajiInput))` が **出ない**こと。`outcome=expired` が多いなら、確認の読みが届いていない(`reschedule_ime_refresh` の枝を疑う)。

## 3. 報告すること
- 入力先・IME・実施した手順ごとの合否と、該当ログ行(上の grep の結果)。
- `[eisu-candidate]` の `age_ms` の分布(特に `cleared` の最大値。`EISU_CANDIDATE_LIFETIME_MS`=1500ms、`MODE_KEY_PASS_REREAD_MS`=60ms の根拠になる)。
- 失敗したら、その時刻前後の `awase.log` の 20 秒ぶんと、`awase-settings --bug-report` の報告(`/bug-report-latest` で取得できる)。

## 4. 補助: 実機で `typing_stress` を回す(任意)
CI と同じ高速打鍵を実機で回して、読み戻しまで自動で比べる。**回している間は実機のキーボードを使えない(フォーカスを奪う)**。
```
cargo run -p awase-windows --example typing_stress --target x86_64-pc-windows-msvc -- \
  --form=edit --interval=20 --layout=layout\nicola_keytop.yab --msime --activate-gji --settle-read
```
`--form=ext --ext=<名前>`(`tools/e2e/input_forms/forms.toml`)で WinForms・WPF・Flutter・Java などを入力先にできる。
