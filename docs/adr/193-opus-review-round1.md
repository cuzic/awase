# ADR-193 敵対的レビュー round1

対象: `docs/adr/193-tsf-native-and-chromium-e2e-targets.md`（commit `b4137af7`）
レビュー方針: ADR の主張を worktree 内の実コードで裏取りし、成立しない箇所を挙げる。

**総括（先に結論）**: 決定1・決定2に対応する資産は**すでに develop 上に実装され、CI にも載っている**
（`crates/awase-windows/examples/ime_key_matrix_spike.rs` の `RICHEDIT50W` ラウンド、
`crates/awase-windows/examples/chrome_probe.rs` の実Chrome+静的ページ+ローカルHTTP回収、
`.github/workflows/e2e-ime.yml` の windows-latest 実機 IME ジョブ）。ADR はこれらに一度も言及して
いない。さらに決定1の go/no-go は実測を待たずコードから `no-go` が確定しており、決定3は
awase が注入キーを物理キーとして扱うための目印（`hook::TEST_INJECTION_MARKER`）に触れていないため、
そのまま実装すると「awase を素通りして再現しない」を「直った」と誤読する構成になる。
現状は起票し直し（既存ハーネスの拡張 ADR へ書き換え）が要るレベル。

---

## Blocker

### B1. 決定2は既存の `examples/chrome_probe.rs` の再発明。ADR に言及がない

**根拠**

- `crates/awase-windows/examples/chrome_probe.rs:1-17`（doc コメント）が、ADR 決定2の目的を
  そのまま書いている: 「Win32 EDIT ではなく **実際の Chrome** で…IME の内部状態を読むのをやめ、
  **ユーザー要件の結果**（打った文字）で状態を判定する」。
- 同 `:38-70` に検証ページ（`<textarea>` + `keydown`/`keyup`/`compositionstart`/
  `compositionupdate`/`compositionend`/`beforeinput`/`input` を記録し `t.value` を同送）。
- 同 `:483-486` で chrome.exe を探索、`:563` `--user-data-dir=<専用プロファイル>`、
  `:568` `--app=http://127.0.0.1:{port}/`。回収は標準ライブラリの `TcpListener` への POST。
- `--repeat=N` / `--no-awase`（対照実験）/ `--chrome=<path>` / `--log=<path>` まである（`:15`）。
- `tools/e2e/ime_key_matrix/README.md`「構成」表が `chrome_probe.rs` を正式資産として登録済み。
- 極めつけに `crates/awase-windows/src/hook.rs:1075` の doc コメントが
  「テストドライバ（`examples/ime_key_matrix_spike.rs`、`examples/chrome_probe.rs`）」と
  名指ししている。本体ソースに名前が書いてあるので grep 一発で見つかる。

**失敗シナリオ**: ADR の通りに着手すると、既存 `chrome_probe.rs` と役割が重なる2つ目の Chrome
ハーネスが `tests/` に生え、検証ページ・Chrome 起動・前面化・プロファイル管理が二重化する。
どちらが SSOT か分からなくなり、片方だけ直す事故（`.claude/rules/fix-requires-evidence.md` が
言う「同じキューの2窓口の片方だけ配線」と同型）が起きる。

**修正案**: ADR を「新規ハーネスの追加」から「`examples/chrome_probe.rs` の拡張」に書き換える。
そのうえで「chrome_probe で今できないことは何か」を具体的に列挙する（例: BUG-002 の long-idle
条件を作れない、確定文字列の golden 比較がない、CI ジョブに載っていない、など）。
それが本 ADR の実質的な決定になる。

### B2. 決定1（自プロセス RichEdit）も既存。すでに CI でビルド・実行されている

**根拠**

- `crates/awase-windows/examples/ime_key_matrix_spike.rs:1555-1557` が `w!("RICHEDIT50W")` で
  入力欄を作る。`:1781` に「RichEdit 5.0（TSF ネイティブ）のウィンドウクラスは Msftedit.dll が
  登録する」とあり、ADR が書く `msftedit.dll` を `LoadLibrary` する手順まで同じ。
- `:414` `const ROUND_NAMES: [&str; ROUNDS] = ["EDIT(標準コントロール)", "RichEdit 5.0(TSFネイティブ)"]`、
  `:29-30` 「全 2 ラウンド（標準 EDIT / RichEdit 5.0）× 20 ステップ」、`:1777` `--round2`。
- `.github/workflows/e2e-ime.yml:172-176` が `cargo build -p awase-windows --example ime_key_matrix_spike`
  し、`e2e` ジョブ（`:186 runs-on: windows-latest`）で実行している。

**失敗シナリオ**: 「作るかどうか」を go/no-go にしている前提が崩れており、スパイク1（RichEdit を
作ってフォーカスする）は**すでに何十回も実行済みの操作**。ログは
`tools/e2e/ime_key_matrix/results/` と `out/` に残っている。時間をかけて同じものを作り直す。

**修正案**: 決定1を「既存 ROUND2（RichEdit）の実行ログから `AppKind`/`AppImeProfile` が何に
なっているかを読み取り、記録する」に差し替える。作る作らないの判断は不要。

### B3. 決定1の go/no-go は実測不要。コード上 `AppKind::Win32` 確定、かつ ADR の因果説明が誤り

**根拠**

- `crates/awase-windows/src/focus/class_names.rs:347-365 detect_app_kind` は**クラス名だけの純関数**:
  `chrome_` 前方一致 / `teamswebview` / `mozillawindowclass` のみ `AppKind::TsfNative`、
  `windows.ui.core.corewindow` / `applicationframewindow` / `windows.ui.input.` が `Uwp`、
  それ以外は `Win32`。`RICHEDIT50W` は必ず `Win32`。
- `AppKind` を書く実運用の経路は `runtime/focus_tracking.rs:307,338` と `runtime/mod.rs:1538` の
  2箇所で、どちらも `detect_app_kind(&class_name)` の戻り値をそのまま代入する。
  （`focus/current.rs:192,224` の `TsfNative` 代入は `#[cfg(test)]` のテスト内。）
- ADR の「学習キャッシュ…次第」は誤り。`focus/imm_learning.rs:45-48` は
  `if new_app_kind != AppKind::Win32 { return; }` で始まり、学習は `ImmCapabilityStore`
  （`(process_name, class_name)` キー）にしか書かない。`AppKind` を書き換える経路は無い。
- ADR の「`Imm32Unavailable` 判定次第」も誤り。`Imm32Unavailable` は `AppImeProfile` の variant
  （`class_names.rs:95-110`）であって `AppKind` の値ではない（`AppKind` は `Win32`/`TsfNative`/`Uwp`）。
  両者は別軸で、ADR は混同している。
- 参考までに `AppImeProfile::from_class_name`（`:120-129`）でも `RICHEDIT50W` は
  `IMM32_UNAVAILABLE_CLASSES`（`:19-35`）にも `is_tsf_native_window`（`:51-59`）にも入らないので
  `Standard` 確定。既存の `EDIT` と同じ ImmCross 経路である。

**失敗シナリオ**: スパイク1に工数を割いた末、「Win32 でした → 作りません」に着地する。
ADR が不確実性として掲げている論点が、実は読めば分かる確定事項である。

**修正案**: 決定1の「未確認」表現を削り、上記の行番号を根拠に「RichEdit は `AppKind::Win32` /
`AppImeProfile::Standard` であり、TsfNative 経路の代表にはならない」と結論を書く。
そのうえで B2 の既存 ROUND2 が何を検証しているのか（TSF text store を持つコントロール相手の
IMM 読み取りの挙動であって、awase の TsfNative *政策* 経路ではない）を明記して整理する。

### B4. 決定3が `AWASE_TEST_INJECTION` / `TEST_INJECTION_MARKER` に触れていない（偽陰性を作る）

**根拠**

- `crates/awase-windows/src/hook.rs:1077` `pub const TEST_INJECTION_MARKER: usize = 0x5350_494B;`、
  `:1079-1091 is_test_injection`: `AWASE_TEST_INJECTION=1` **かつ** `dwExtraInfo` が目印に一致する
  ときだけ、注入キーを物理キーとして扱う。`:1092-1095` の通り **リリースビルドでは常に false**。
- `tools/e2e/ime_key_matrix/README.md`「仕組み」節が同じことを明記
  （「キーは `SendInput` で注入し、`dwExtraInfo = 0x5350494B` の目印を付ける」）。
- 一方 ADR が「フォーカスは既存の強制前面化ヘルパー(`e2e_windows.rs`)で取る」と参照する
  `crates/awase-windows/tests/e2e_windows.rs:684-712 send_key_to_edit` は
  `dwExtraInfo: 0`（`:698`, `:710`）で、関数コメント自身が
  "Send a keystroke via SendInput (**bypasses hooks**, goes to foreground window)" と書いている。

**失敗シナリオ**: ADR の決定3をそのまま実装（`e2e_windows.rs` の SendInput ヘルパーを流用）すると、
awase のフックが `LLKHF_INJECTED` として無視する（または NICOLA 変換を行わない）ため、
Chrome に届くのは素の romaji になる。これを「リテラル漏れが再現しない＝修正が効いている」と
読むと完全な偽陰性。BUG-14（外部注入 IME モードキーを物理扱いしてしまった件、
`project_bug14_injected_dbe_hiragana`）の裏返しの罠であり、リリースビルドで走らせた場合は
**目印を付けても**無効になる点まで含めて落とし穴が二重にある。

**修正案**: 決定3に以下を明記する。(a) 注入は `dwExtraInfo = hook::TEST_INJECTION_MARKER` を付ける、
(b) awase 側は `AWASE_TEST_INJECTION=1` かつ **debug ビルド** で起動する、
(c) 目印なしの SendInput（`e2e_windows.rs::send_key_to_edit`）は awase 経路の検証には使えない、
(d) 目印を付け忘れた回を検出する手段（`check_multi.py` が awase ログの `extra=0x0` で INVALID に
する既存の仕組み）を使う。

---

## Major

### M1. `document.title` 回収では「イベント列」を取れない（決定2の経路1の前提が崩れる）

ADR は「JS は…イベントのたびに、textarea の value と**イベント列**を `document.title` へ反映する。
ハーネスは `GetWindowTextW` で title を読み、確定文字列・`compositionend.data` を assert する」と
書くが、ハーネスは**ポーリングで最新の title を1回読むだけ**であり、中間状態は原理的に落ちる。
加えて Chrome の window title はレンダラ→ブラウザプロセスの IPC を経てキャプションに反映され、
連続更新は合体されうる。`compositionupdate` ごとに title を書き換えても、読める保証があるのは
「読んだ瞬間の最後の1つ」だけ。

ADR 自身が「titleの長さ制限内の短い文字列向け」と書いている通り、累積ログ化して順序を保つ方向にも
逃げられない。結果として経路1で assert できるのは「最終的な value」程度で、
`compositionend.data` やイベント順は取れない。

**修正案**: 経路1を「最終 value の軽量確認だけに使う補助」に格下げし、主経路を既存 `chrome_probe.rs`
の HTTP POST（ADR の経路2）にする。優先順位を ADR とは逆にする。

### M2. 成功基準が BUG-002 の実際の再現条件と噛み合っていない

`docs/known-bugs/BUG-002.md` が挙げる再現条件は
(a) IME = **Google 日本語入力**（GJI I/O を probe する話）、
(b) `keyboard long idle (>10s)` または `物理 F2 + GJI long idle`、
(c) probe 起点が「F2 送信時刻」か「物理 F2 の `cold_marked_ms`」かの差、
(d) GJI が 12 秒休眠後に Chrome の composition context 再初期化に ~326ms 必要、である。

一方 ADR の検証計画は「フォーカス直後の入力タイミングを制御する必要がある（idle 時間を変えて
『フォーカス→N ms→入力』を掃引）」。これは (b) の keyboard idle / GJI idle のどちらでもない。
N を数百 ms で掃引しても `long_idle` 分岐に入らないので、`という→toいう` は永久に再現しない。

さらに ADR が Chrome の症状として並べる `bあ` は BUG-002 ではない。
`.claude/rules/tuning-constants.md` の記載通り `9a7e699`（`GJI_LONG_IDLE_PROBE_TOTAL_MS` 150→350ms、
「F2×2 後 GJI が VK 受付可能になるまで実測 181ms」）の症状で、BUG-002 の修正履歴
（`b101153` / `79134f5`）には含まれない。ADR は別々のバグを同じ括りにしている。

**修正案**: 成功基準を「keyboard idle >10s と GJI 休眠 ~12s を実際に作れること」「物理 F2 相当の
入力（= 目印付き SendInput の F2）とプログラム的 F2 を区別して打てること」まで具体化する。
1ケースあたり十数秒の実 idle が必要になるので、ジョブ時間見積もり（現行 `e2e-ime.yml` は
`timeout-minutes: 25`）への影響も書く。`bあ` は対象から外すか、別バグとして分けて書く。

### M3. 「修正前のコミットで再現、修正後で消える」は実行計画として成立しない

`b101153` / `79134f5` 時点のツリーには新ハーネスも `chrome_probe.rs` も存在しないため、
「修正前のコミットで再現する」にはハーネスを数ヶ月前のツリーへバックポートする必要がある
（`Cargo.lock`・windows-rs のバージョン差も踏む）。

一方、このリポジトリには既に確立した代替手段がある: `tools/e2e/ime_key_matrix/ablations/aN-*.sh`
が現在のツリーに対して「修正の撤去」をスクリプトで当て、`e2e-ime.yml:161-168` が
「撤去が差分を作らなかったら fail」まで含めて CI 化している。tuning 定数を旧値へ戻す撤去
スクリプトを1本足すほうが、はるかに安く再現性がある。

**修正案**: 成功基準を「`ablations/` に BUG-002 相当の撤去（`CHROME_PROBE_LONG_IDLE_MIN_MS` を
20ms へ戻す等）を1本追加し、撤去あり=FAIL / 撤去なし=PASS が N 回安定して出ること」に書き換える。

### M4. CI に関する記述が不正確（「continue-on-error で不安定」ではなく「CIで一度も走っていない」）

ADR は「TsfNative の実機検証は Windows Terminal…のみ。…前面化・フォーカスが CI で不安定
（`continue-on-error`）」と書くが、実際は次の通り。

- 当該テストは `crates/awase-windows/tests/e2e_windows.rs:2687-2689`
  `fn e2e_msime_windows_terminal_vk_mode_coldstart_interactive()`。冒頭 `:2691-2694` で
  `is_interactive_session()` が false なら即 return。
- `is_interactive_session()`（`:447-464`）は `CI` または `GITHUB_ACTIONS` が設定されていれば
  `false` を返す（`AWASE_E2E_INTERACTIVE=1` の明示 opt-in がある場合を除く）。
- `.github/workflows/ci.yml:343` のブロッキングステップのフィルタは
  `--skip e2e_sendinput --skip e2e_ime_status_detection` で、この WT テスト名は一致しない。
  つまり WT テストは**ブロッキング側に入り、早期 return で空パスしている**。
  `continue-on-error` のステップ（`:347-355`）は `e2e_sendinput` と `e2e_ime_status_detection`
  だけを対象にしており、WT テストとは無関係。

**失敗シナリオ**: 「不安定だから continue-on-error になっている」という誤った現状認識のまま、
「安定化すれば CI で使える」と読める優先順位を立てる。実際の課題は
「そもそも CI で実行させる判断をしていない」であり、打ち手（環境変数の扱い・専用ワークフロー）が違う。

**修正案**: 上記の行番号で現状を書き直す。「CI で実行されていない」「実行させたいなら
`e2e-ime.yml` 側（既に windows-latest で実 IME を入れて SendInput を回している）へ寄せるのが筋」
という整理にする。

### M5. 「Windows Terminal は確定文字列をプロセス外から読めない」は現行コードと矛盾する

既存 WT テストは、PowerShell の `Read-Host` にプロンプトを出させ、入力結果を
`Set-Content` でファイルへ書かせてそれを読み戻している（`e2e_windows.rs:2849-2875`、
判定は `:2900-2917` で `を` / `wお` を検査）。`WM_GETTEXT` が使えないだけで、確定文字列は取れている。

**失敗シナリオ**: 「読めないから新しい回収手段が要る」という前提で決定2を正当化しているが、
前提が成立していない。既存のファイル回収方式を一般化する（対象アプリに書かせて読む）という、
より安い選択肢の検討が飛んでいる。

**修正案**: 背景節を「`WM_GETTEXT` では読めず、アプリごとの回収手段（WT は Read-Host + ファイル、
Chrome は HTTP POST）が要る」と正確に書き直し、Chrome 側の回収手段が既に
`chrome_probe.rs` にある事実（B1）へ接続する。

### M6. go/no-go の判定変数が目的とずれている（`AppKind` ではなく `AppImeProfile`）

ADR が通したいのは「TsfNative 経路（Vk 注入・force-on・warmup）」だが、その分岐の大半は
`AppImeProfile` が支配する:

- `class_names.rs:186-190 can_use_imm32_cross_process`（`Imm32Unavailable`/`TsfNative`/`InputRelay` → false）
- `:231-253 effectively_tsf_native`（doc が「`*profile == AppImeProfile::TsfNative` ではなく必ず
  このメソッドを使うこと」と明記）
- `:305-330 From<AppImeProfile> for ImePolicyProfile`（actuation chain の選択）

`AppKind` が効くのは `output/types.rs:9-29`（`InjectionMode` を `Vk` にするか `Unicode` にするか）
だけである。ADR の go/no-go は「`AppKind` をログで確認する」だが、仮に `TsfNative` になっても
`AppImeProfile` が `Standard` のままなら、force-on も warmup も ImmCross 側へ流れる。

**修正案**: go/no-go を立てるなら判定対象を `AppImeProfile`（および `effectively_tsf_native`）にする。
ただし B3 の通り `RICHEDIT50W` は `Standard` 確定なので、判定するまでもない。

### M7. 見落とした代替案: 自前ウィンドウのクラス名を分類テーブル上の名前で登録する

awase の分類は**クラス名の文字列一致のみ**である（`class_names.rs:19-35`, `:51-59`, `:347-365`）。
したがってハーネス側で `RegisterClassExW` するクラス名を

- `"Chrome_WidgetWin_1"` → `detect_app_kind` = `TsfNative` **かつ** `AppImeProfile` = `Imm32Unavailable`
- `"org.wezfurlong.wezterm"` / `"CASCADIA_HOSTING_WINDOW_CLASS"` → `AppImeProfile` = `TsfNative`

にすれば、**外部アプリを一切起動せずに**、awase の TsfNative / Imm32Unavailable の政策分岐を
決定的に走らせられる。入力欄は自前の `EDIT` / `RICHEDIT50W` のままなので確定文字列は
`WM_GETTEXT` で厳密に assert でき、フォーカスも `SetFocus` で決まる。

限界も明確に書ける: 模倣できるのは「awase が何を送るか」までで、Chromium が実際に TSF で
どう受けるか（cold-start の context 再初期化、composition のタイミング）は再現しない。
逆に言えば、キー選択・gate・force-on・suppress/allow といった
`.claude/rules/fix-requires-evidence.md` の「再発ファミリー」の大半はこれで固定できる。

**修正案**: 代替案節にこの案を追加し、「決定的に固定できる範囲（awase の送信側）」と
「実 Chrome でしか見えない範囲（受け側のタイミング）」の切り分けを ADR の軸に据える。
決定1（RichEdit）より安く、決定2の負担も減る。

### M8. 「未確認」と書かれた前提のうち3件は、リポジトリ内で既に答えが出ている

ADR は「`windows-latest` に同梱の想定（未確認）」「CI(windows-latest)で IME がそもそも使えるのか」
「SendInput 到達性」を未確認として残しているが、`.github/workflows/e2e-ime.yml`（**develop に存在**、
`git cat-file -e origin/develop:.github/workflows/e2e-ime.yml` で確認済み）が既に答えている:

- `:186 runs-on: windows-latest` の e2e ジョブで
- `:201-205` chocolatey で Google 日本語入力を導入（約30秒）
- `:207-241` `Set-WinUserLanguageList` / `Set-WinDefaultInputMethodOverride` / `config1.db` 生成 /
  `ctfmon` 再起動まで行い
- `:244-262` は MS-IME 本体（ja-JP 言語機能の導入、TIP 登録待ち最大5分）
- その上で目印付き SendInput の実機 E2E を回している。

逆に ADR のコスト見積もり（決定1「数十〜百行程度」）には、この IME 導入・言語設定・ctfmon 再起動
（1ジョブ当たり数分）と、Chrome プロファイル管理のコストが入っていない。

**修正案**: 「未確認」を消し、`e2e-ime.yml` の該当ステップを根拠として引用する。そのうえで
本 ADR の追加分がこの既存ジョブにどう載るのか（新しい構成名を `plan` ジョブの表に足すのか、
別ワークフローにするのか）を書く。

---

## Minor

### m1. 行番号・参照の精度

- 「Edit 生成（503〜555行付近）」: `e2e_windows.rs:503` が親ウィンドウ、`:532` が `EDIT` 子、
  エラー処理が `:559` まで。おおむね妥当。
- 「2571行以降の BUG-13 Vk-mode gap 節」: 節コメントは `:2568` から。おおむね妥当。
- 「`focus/classify.rs` は `RICHEDIT50W` を…`TextInput` にする（101〜105行）」: 実際は `:104`
  （`matches!` ブロックは `:98-110`、`TextInput` を返すのは `:120-124`）。範囲としては許容。
  ただし **`classify.rs` の `FocusKind::TextInput` と `class_names.rs` の `AppKind` は別物**であり、
  ADR の書き方は「classify.rs を見れば AppKind の話が続いている」と読める。B3 と併せて分離して書くこと。

### m2. frontmatter は規約違反なし。ただし関連 ADR に抜けがある

- `related_adr: ["ADR-0002", "ADR-0003"]` は旧4桁系列の実在 ADR
  （`docs/adr/0002-tsf-coldstart-warmup.md` id=`ADR-0002`、`0003-chrome-vk-injection.md` id=`ADR-0003`）で、
  話題（TSF cold-start warmup / Chrome VK injection）も適合。誤記ではない。
- `docs/adr/index.md:197` に1行追加済み、1ファイル1トピック、`summary`/`status` あり →
  `.claude/rules/docs-frontmatter-convention.md` は満たす。
- 記憶用 wikilink `[[...]]` の混入なし、日本語のみ。
- 抜け: **ADR-186**（既存ハーネス `ime_key_matrix` / `chrome_probe` / `e2e-ime.yml` の根拠 ADR。
  `chrome_probe.rs:1` が「ADR-186 実機E2Eプローブ」と自称している）が `related_adr` に無い。
  逆に `ADR-159` / `ADR-163` は列挙されているが本文で一度も参照されていない。

### m3. complexity-budget の扱い

決定4の「actuation 合流点の許可リスト・tuning 定数は触らない（complexity-budget の対象外）」は
判断として正しい（`.claude/rules/complexity-budget.md` の対象は `RESTRICTED_CALLS` と
`tuning.rs` の `pub const`）。ただし M3 の修正案（旧 tuning 値へ戻す撤去スクリプト）を採る場合、
`tuning.rs` 自体は変更せず `ablations/` のスクリプトで差分を当てる形になるので、
この整合も ADR に一行書いておくとよい。

### m4. 決定4「本体ソースは変更しない」と既存のテスト用フックの整合

本体には既にテスト専用の入口がある（`hook.rs:1077-1095` の `TEST_INJECTION_MARKER` /
`is_test_injection`、debug ビルド限定）。決定4を「本体を**新たに**変更しない」の意味で書くなら
問題ないが、B4 の通りこのフックを**使う**ことは必須なので、「既存のテスト用フック
（`AWASE_TEST_INJECTION`）に依存する」と明記しないと、決定4が B4 の対策を禁じているように読める。

### m5. 前面化ヘルパーの共有は不可能（3つ目のコピーが生まれる）

決定3の「フォーカスは既存の強制前面化ヘルパー(`e2e_windows.rs`)で取る」は、
`force_foreground` が `tests/e2e_windows.rs:2584` 付近のテストローカル関数であるため、
`examples/` からは参照できない。`chrome_probe.rs:24-33` も独自に `AttachThreadInput` 版を持っている。
実装すると同じヘルパーの3つ目のコピーになる。共有したいなら `awase-windows` の
`#[cfg(debug_assertions)]` なテスト支援モジュールに出す必要があり、これは決定4（本体不変更）と衝突する。
ADR でどちらを取るか決めること。

---

## 全体としての「作る価値」の判断について

1. **楽観バイアス**: ADR が挙げる4つの「未確認」のうち、3つ（Chrome/Edge の同梱、CI での IME 利用、
   SendInput 到達性）は `e2e-ime.yml` が既に実証済み、1つ（RichEdit の分類）はコードから確定できる。
   つまり ADR が不確実性として提示しているものの実体はほぼ無く、逆に**本当に難しい論点**
   （title 更新の合体とイベント順の喪失 M1、cold-start の idle 条件の作り方 M2、
   旧コミットでの再現手段 M3）は「検証で確認する」で流されている。不確実性の配分が逆。
2. **論理の飛躍**: 「自プロセスの素直なコントロールでは再現しない」→「だから RichEdit を作る」
   の間に、「RichEdit は自プロセスの素直なコントロールではないのか」という検証が無い。
   B3 の通り awase から見れば `EDIT` と同じ `Standard` であり、この飛躍が決定1の根拠を壊している。
3. **棚卸しの欠落**: 決定1・決定2に対応する資産が develop 上にあり、かつ本体の doc コメント
   （`hook.rs:1075`）から名指しで辿れる状態だったにもかかわらず、代替案節にも背景節にも出てこない。
   ADR の起票前に `tools/e2e/` と `crates/awase-windows/examples/` を見ていないことが明らか。

**推奨**: 本 ADR は「提案」のまま破棄せず、次の形へ全面的に書き直す。

- 背景: 既存資産（`ime_key_matrix_spike`（EDIT/RichEdit 2ラウンド）、`chrome_probe`（実Chrome+HTTP回収）、
  `e2e-ime.yml`（windows-latest + GJI/MS-IME））の棚卸しと、それらで**現在できていないこと**の列挙。
- 決定1（差し替え）: RichEdit は `AppKind::Win32`/`AppImeProfile::Standard` であるという結論を記録し、
  TsfNative 政策分岐を決定的に検証したいなら M7 のクラス名登録案を採るか否かを決める。
- 決定2（差し替え）: `chrome_probe.rs` に BUG-002 の long-idle 条件と確定文字列 golden を足す。
  `document.title` は補助に格下げ（M1）。
- 決定3（補強）: 目印付き SendInput + `AWASE_TEST_INJECTION=1` + debug ビルドを必須要件として明記（B4）。
- 成功基準（差し替え）: 旧コミットのチェックアウトではなく `ablations/` の撤去スクリプトで
  「撤去あり FAIL / なし PASS」を N 回安定させる（M3）。
