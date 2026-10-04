---
id: ADR-209
title: |-
  GJI の MS-IME プリセットでは、TSF の窓で、直接入力の変換が IME を開く(入力モードは閉じる前のまま)。読めない窓(TsfNative/Imm32Unavailable)の打鍵時予測に「変換で開く」を足し、素通しされた変換に Engine を追随させる
summary: |-
  実機(dragonflyg4、JIS、GJI 3.34.6260.0、`session_keymap=2`)で、IME OFF から変換を単独タップすると、awase 停止でも WT・メモ帳・Edge の全てで IME が ON になる。awase は「予測しない」ため Engine が OFF のまま(`ka`→`か`)。
  GitHub Actions(windows-latest、GJI の MS-IME プリセット、awase なし、run 36690572075)で、**実 Chrome(TSF)は開き、素の EDIT(IMM32)は開かない**ことを再現した。古い custom 表の有無は無関係(表なしでも同じ)。
  同梱表(`key_effect_table.rs`)は EDIT(IMM32)で学習したので、TSF の窓では変換について誤っている。実機で、確定後の変換は候補窓を出さず開くだけ、半角英数で閉じた後の変換は半角英数のまま開くことも確認した。
status: |-
  実装済み(2026-09-30、設定 general.predict_henkan_open_in_unreadable_windows 既定 true、v2.0.0 に含まれる)。CI で sc-adr209-chrome-msime*(ccb17771)により実 Chrome×MS-IME を検証、i2_unwarranted 超過の調査は BUG-179 に記録。実機未確認。
  旧(2026-10-04 更新前):
  採用(2026-09-30)。v1(古い表を実効とみなす)は棄却。v2 は Opus round2 で Major 6件。X6・X7 の実機結果と指摘を反映した v3 が、Opus round3 で収束(新しい Major なし)。実装済み(2026-09-30、`feat/adr209-henkan-open-prediction`。設定 `general.predict_henkan_open_in_unreadable_windows`、既定 true)。実機A/B・CI検証はマージ後。
related_adr:
  - "ADR-186"
  - "ADR-191"
  - "ADR-192"
  - "ADR-195"
  - "ADR-196"
  - "ADR-199"
  - "ADR-206"
---

# ADR-209: 読めない窓で、MS-IME プリセットの変換が IME を開くことを予測する

## 経緯(v1 の棄却)
v1 は「MS-IME プリセット(2)で、古い `custom_keymap_table` を実効とみなす」という決定3を置いた。Opus 敵対レビュー(round1)が、次を指摘した: 根拠(実機1台の観測)は未同定で、既存の証拠(ADR-186 決定2(c)、Mozc `keymap.cc`、CI の格子)は逆を指す。
実機で仮説を弁別した結果(2026-09-30):
- **X1**: IME ON で無変換を押すと、`カ`→`ｶ`→`か` と巡回した。古い表(無変換の行なし)が実効なら毎回 `か` のはずで、**プリセットが実効**。
- **overlay**: `config1.db` の field 68 が無い(protobuf を全解析)。overlay 100 ではない。
- **X3**: 変換を IME OFF から押すと、メモ帳・Edge でも IME が ON になった(WT と同じ)。**アプリ依存ではなく、TSF の窓で共通**。
- **X6**(メモ帳、確定した「漢字」の直後で IME OFF から変換): 候補窓は出ず、IME が ON になるだけ(再変換は働いていない)。
- **X7**(半角英数で IME を閉じてから変換): IME は ON になるが、**入力モードは半角英数のまま**(閉じる前のモードを引き継ぐ)。
- **CI**(run 36690572075): GJI の MS-IME プリセット、awase なしで、**実 Chrome は「直接入力→変換」で開く(古い表の有無によらず)**。**素の EDIT(`ime_key_matrix_spike` の `--seq=1C`)は開かない(`open=0`)**。無変換は実 Chrome でも `か`→`カ` と巡回(実機と同じ)。

## 背景(実機・CI の事実)
- GJI の MS-IME プリセットでは、直接入力の変換の定義は `Reconvert`(Mozc の `ms-ime.tsv`。IME の開閉とは無関係)。**それでも実 Chrome・WT・メモ帳・Edge では IME が開く(候補窓は出ない。入力モードは閉じる前のまま)。IMM32 の素の EDIT では開かない。**(仕組みは未確認。事実は CI と実機の観測)
- **awase の窓の分類(`AppImeProfile`、`focus/class_names.rs`)は TSF/IMM32 の区別ではない**(クラス名の固定リスト)。メモ帳(RichEdit、TSF)・Firefox・WPF・Office は `Standard` になる。よって本 ADR は「**読めない窓(予測で追う: `TsfNative`/`Imm32Unavailable`)**」と「**読める窓(観測で追う: `Standard`/`ImmCross`)**」で分ける。`InputRelay` は awase が観測も actuation も持たない窓なので対象外。
- 同梱表(`key_effect_table.rs` の MSIME 表、`grid-tables/msime.json`)は、学習プロセスの EDIT(IMM32)で測ったもの。**TSF ネイティブの窓の予測には、そのまま使えない**(変換だけでなく、他のキーも窓の種類で違う可能性。未測定)。
- awase の予測(`key_effect_predictor.rs::predict_with_override`)は、`custom_keymap_table` がそのキーの行を持つと予測を打ち切る(`custom_table_overrides`)。**GJI はプリセット(CUSTOM 以外)のとき `custom_keymap_table` を読まない**(ADR-186 決定2(c)の ATOK、今日の X1)ので、この打ち切りは ATOK/MS-IME プリセットでは不要で、この実機では予測を止めている(古い表が変換の行を持つため)。
- TsfNative の窓では開閉を観測できない(`read_ime_state_*` は `None`、`ConvOpenInference` は開閉を区別できない)。**追随の手段は打鍵時予測(`KeyEffectPredicted`)だけ**。
- 学習(ADR-195/196)は、この実機では完走しない(BUG-178)。学習プロセスの入力先は素の EDIT なので、学習しても TSF の窓の効果は得られない。

## 決定
1. **読めない窓の打鍵時予測に「変換で開く」を足す**: GJI の MS-IME プリセット(`session_keymap` が不在/`NONE`/MSIME。Windows の既定)で、`cannot_verify_real_ime_state` な profile(`TsfNative`/`Imm32Unavailable`。`InputRelay` は除く)では、**閉状態(DirectInput)の無修飾の変換(0x1C)は「IME を開く」**と予測する。**読める窓(`Standard`/`ImmCross`)は観測に任せる**(メモ帳は X3 で開くと確認済みで、観測で追随する)。
2. **予測は開閉だけ**: 入力モード(`mode`)・段階(`stage`)は予測しない(`None`)。X7: 開いたときの入力モードは閉じる前のまま(半角英数なら半角英数)。belief が `Unknown` のときだけ、既存の種(`kana_mode()`)を使う(`predict_in_table` の既存の挙動と同じ)。X6: 候補窓は出ない(再変換は働かない)ので、段階は `None` のままでよい。
3. **学習表より窓別の規則を優先する(読めない窓の、閉×変換のセルだけ)**: 学習プロセスは素の EDIT で測るので、学習が完走しても `off|henkan=OFF` が入る(CI の格子と同じ)。それを理由に、読めない窓の変換が「開かない」に戻らないよう、このセルは学習表より窓別の規則を先に引く。**これは ADR-196(学習結果を内蔵表より優先)の例外**で、理由は「学習の入力先は素の EDIT で、窓の種類を表せない」(ADR-196 側にも1行追記する)。
   **置き場所は純関数の中に固定する**: `PredictInput` に窓の種類(`unreadable: bool` 等)を足し、`predict_with_override` の**先頭**で「GJI の MSIME 系プリセット(`session_keymap` 不在/`NONE`/`2`)かつ `unreadable` かつ閉状態かつ `vk==0x1C`」なら窓別の規則の予測を返す。順序は「窓別の規則 → 学習表 → 打ち切り → 同梱表」。パイプライン(`kp_predict_key_effect`)の仕事は、`current_app_profile().cannot_verify_real_ime_state(class)` と `!= InputRelay` を `PredictInput` に詰めるだけ(規則をパイプライン側に書くと、Linux の単体テストで固定できず、`runtime/` の `#[cfg(windows)]` の陰で消える)。未知の overlay(100 以外)があるときは窓別の規則を当てない(既存の `has_overlay` の打ち切りと揃える)。単体テストで固定する。
4. **予測の打ち切りを見直す**: `session_keymap` が ATOK/MS-IME/不在/`NONE` のとき、`custom_keymap_table` の行を理由に予測を打ち切らない(GJI はプリセットのときこの表を読まない。ADR-186 決定2(c)、X1)。KOTOERI/MOBILE は `preset=Custom` で同梱表が空なので、外しても変わらない。ATOK+古い表では、同梱の ATOK 表で予測するようになる(ADR-186(c)の実機結果と一致)。CUSTOM のときは従来どおり。**この変更で新たに予測されるようになるキー**(古い表に行があった英数・ひらがな・Space/Enter/Esc/BS など)は、EDIT で測った同梱表の予測が当たる。既知の差の候補は、開状態の変換(`Reconvert`。TSF で直前に文字があっても再変換は働かないことを X6 で確認)。
5. **止める設定**: 新しい bool 設定(名前は実装時)を1つ置く。既定は有効。**止める対象は決定1・3(窓別の規則)**で、パイプラインが `unreadable=false` を詰めることで無効にする。決定4(打ち切りを外す)は、古い表という別の前提(プリセットでは表を読まない、X1・ADR-186)の訂正なので、この設定の対象に含めない。偽 ON が実機で出たら、ビルドし直さずに止められる。
6. 新しいイベント・I/O・actuation の合流点・tuning 定数は作らない。fence は既存の `KEY_EFFECT_SETTLE_MS`。
7. **記録**: BUG-143(根本原因の記述が X1 で否定された)・ADR-174 に訂正を追記し、本修正のコミットを `fix_commits` に入れる(新しい BUG を起こすなら相互に参照)。診断関数 `gji_charset_autodetect.rs::classify_mode_key_ime_action`(MSIME で表を優先)の doc に「X1 で前提は否定された」と書くか、決定4 と揃える。テスト `realdev_msime_preset_with_stale_custom_table` の期待値とコメントを、「EDIT では開かない/TSF の窓では開く」に更新する。

## 非目的
素通し後に awase が IME へ書くこと(ADR-191 決定1)。ADR-206 決定1(α)(Suppress × エンジン非活性では生キーが IME に届く)の変更。窓別の表の**全キーの網羅**(別 ADR: TSF 形式の入力先での学習、または受動学習)。学習プロセスの修正(BUG-178)。PR #360(別件、保留)。

## 代替案
- **v1(古い表を実効とみなす)**: 棄却(上)。
- **利用者への案内(ADR-192 の `UserOverride`)**: 設定の食い違いではなく、プリセットの通常の挙動なので該当しない。
- **受動学習(窓の種類別に、実際の効果を観測して覚える)**: 原理的に最も一般的だが、新しい保存・証拠・採否の設計が要る(v2 の後)。この ADR の窓別表は、その学習の初期値になる。
- **候補窓の検出による自己修復(ADR-203 案C)**: 追随が遅れる(最初の数文字が `か`)。安全網として別 ADR。

## リスク
1. **偽 ON(窓の誤分類)**: 「開かない窓」を `TsfNative`/`Imm32Unavailable` と判定すると、開かないのに「開く」と予測する(読めない窓では自動で直らず、hwnd キャッシュ経由で `desired_open` に1時間残り、ActivationSync が `VK_IME_ON` を送りうる。round1 M1)。**未測定のクラス**: UWP の `Windows.UI.Core.CoreWindow`/`ApplicationFrameWindow`、`XamlExplorerHostIslandWindow`(エクスプローラーの検索欄等)、`Intermediate D3D Window`、`PseudoConsoleWindow`、wezterm(独自の TSF 実装)。緩和: profile 全体に当て、決定5の設定で逃がす(クラス許可リストは複雑さに見合わない)。未確認のクラスは未検証事項に名前で残す。
2. **他の GJI バージョン**: 実機は 3.34.6260.0。CI の版は未記録。CI のワークフローで `GoogleIMEJaConverter.exe` のファイルバージョンを1行ログに出す。
3. **予測は `desired_open` を書かない**: 直前の Ctrl+無変換(明示 OFF、TTL 30秒)の後、belief だけが ON になる。**Blind の窓(Edge/Chrome)では、drift の「検知」(`DriftDetected` で `applied` を `Optimistic` に偽装、試行回数の加算、「IME状態を確認できません」のバルーン)まで進みうる**(授権で送信は止まるが、BUG-163 の早期 return は ImmCross 限定)。テストと CI で固定する。**CI の合格条件(`[drift]` の行・送信・通知が無い)が落ちたときの手当ては、「授権が下りないなら検知しない」という BUG-163 の早期 return を Blind の窓にも広げる(条件1つ)ことに決める。予測に `desired_open` を書かせる案は取らない**(ADR-191 の規律に反する)。明示意図なしで起動した直後(`desired_open` が初期値 `true`)は、ActivationSync が `VK_IME_ON` を1回送りうるが、予測が正しければ IME は既に ON で冪等なので、合格条件にはしない。
5. **入力モードが `Unknown` の間に閉状態で変換を押す**と、既存の種(かな)で Engine が活性化する。閉じる前が半角英数だった場合は外れる(X7)。他の予測と同じ既存の限界で、この ADR が作る穴ではない。
4. **GjiFsm の開き直し(`kp_reopen_gji_fsm(Predict)`)**: 予測が正しければ IME は開いているので通常の ImeOn と同じ。偽 ON のときは round1 M1(d) のとおり(F2/ESC が漏れうる)。closed_loop の負の場合で送信キー列を見る。

## 検証方針
- **CI(windows-latest)の閉ループ**: `chrome_probe`(実 Chrome、`Imm32Unavailable`)を **awase あり**で、GJI の MS-IME プリセットで実行し、「直接入力→変換」の後に Engine が追随して NICOLA の文字になることを確認する(今の awase では `か`)。**合格条件に、「明示 OFF → 変換 → 2秒待つ」の後、`[drift]` の行・`VK_IME_ON`/`VK_IME_OFF` の送信・通知が無いことを含める**(`ka` が NICOLA になるだけでは、後から閉じられる事故を捕まえられない)。CI で試せるのは `Imm32Unavailable`(実 Chrome)だけで、`TsfNative`(WT の InputSite)は実機の A/B だけになる(明記)。
- **単体テスト**: 予測器(MS-IME × `TsfNative`/`Imm32Unavailable` × 閉 × 変換 → 開く、mode/stage は `None`。`Standard`/`ImmCross` → 予測しない。`custom_keymap_table` に行があっても、プリセットでは予測する。学習表に `off|henkan=OFF` があっても、読めない窓では開くと予測する)。既存テスト `realdev_msime_preset_with_stale_custom_table` は、変換(読めない窓では開く/読める窓では予測しない)に加えて、**英数(0xF0。打ち切りが外れて予測するようになる)の期待値も更新**し、「プリセットでは表の行があっても同梱表で予測する」ことを英数とひらがなで固定する。
- **`closed_loop_scenarios`**: 疑似 IME(`pseudo_ime.rs`)は ATOK の格子しか持たない。MSIME の格子を読めるようにし、「窓の種類」で閉状態の変換の結果だけを差し替える形にして、正(開く窓)・負(窓判定を誤り、開かない)の両方を同じ仕組みで書く。負の場合で、drift・`VK_IME_ON`・F2/ESC の送信が無いこと。
- **`architecture_guard`**: `KeyEffectPredicted` の dispatch 元が1箇所のまま。
- **実機 A/B(dragonflyg4)**: 基準値として、**v2 を入れる前の awase が、メモ帳(`Standard`、観測経由)でどう振る舞うか**を記録する。そのうえで、WT(`Imm32Unavailable`)・Edge・未測定のクラス(設定アプリの検索欄〔UWP〕、エクスプローラーの検索欄〔XAML〕、WezTerm)で「IME OFF → 変換 → `ka`」が `きう`(NICOLA)になること。
- `ime_key_sequence_golden`: キー列は変えないので影響なし(F2/`VK_IME_ON` が送られる負の場合は closed_loop で押さえる)。

## 未検証事項
- Reconvert が TSF の窓で IME を開く仕組み。他の TSF の窓(VS Code、Electron、UWP、Firefox、WPF、Office)でも開くか。上の未測定クラス。
- 変換以外のキー(この ADR で新たに予測されるようになるキーを含む)の、窓の種類別の差。
- 実機以外の GJI のバージョンでの挙動。
- 直前に文字を選択した状態での変換(X6 は確定後の直後のみ確認。候補窓は出なかった)。
