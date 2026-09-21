---
id: ADR-193-companion-193-implementation-tasks
title: |-
  ADR-193 実装タスク一覧（Chrome idle-sweep E2Eの詳細設計と着手順序）
type: companion-doc
related_adr:
  - "ADR-193"
  - "ADR-186"
---

# ADR-193 実装タスク一覧

[ADR-193](193-extend-existing-e2e-harness-for-chromium-coldstart.md)決定4を、実装可能な単位に分割したタスクリスト。
形式は[163-implementation-tasks.md](163-implementation-tasks.md)を踏襲する（内容・変更ファイル・受け入れ基準・依存）。
各タスクは個別のコミットにすること。**未検証の前提には「(未確認)」を付けた**。

**改訂履歴**: 初版をopus-adversarial-consult（計画round1）でレビューした結果、Blocker1件・Major4件・Minor11件が
見つかり、全指摘を実コードで裏取りして反映した改訂版（v2）。主な訂正は次の通り。
- **PB1**: ビルドキャッシュの`hashFiles`にworkflow自身が入っておらず、T3aは`chrome_probe.exe`が無いまま走る。
- **PM1**: `ColdKind::classify`は`CHROME_LONG_IDLE_MS`(5s)ではなく7s/10sのみを見る（`tuning.rs`のdocがstale）。
- **PM2**: 測るのはkeyboard idleではなく`gji_idle_ms`。GJI休眠の有無は`idle_at_cold`で直接測れる。
- **PM3**: 45試行がrcのANDに潰れ、flakeと撤去効果を分離できない → 試行単位のTALLYと率で判定。
- **PM4**: `chrome_probe`に既存ワークフローが必要とするbelief合わせ（`--activate-gji`相当）が無い。

計画round2で、さらに Blocker1件・Major2件・Minor7件が見つかり反映した（v3）。
- **PB2**: `--activate-gji`の本質はVK_IME_OFFではなく`activate_gji_profile()`（TSFの`ActivateProfile`）。ymlのコメントだけを読んで実装を照合していなかった（同型の誤り4回目）。
- **PM5**: 「ASCII英字を含まない→PASS」はawaseが落ちた run（`か`）をPASSにする。`ensure()`時の観測値を期待値として比較する。
- **PM6**: 設計4の試行数（45 vs 135）が矛盾し、fail率2%は実質0件要求。ベースライン≦10%・撤去≧50%・比4倍以上へ。

計画round3で、さらに Major2件・Minor8件が見つかり反映した（v4）。Blockerは無し。
- **PM7**: `gji_idle_ms()`はGJI監視が未アタッチだとマシン稼働時間を返し（`observer.rs:434-442`）、掃引の全点が`Long`に潰れる。`idle_at_cold`を「並べる」のでなく「検証する」。
- **PM8**: `ensure()`は`kあ`を`Nicola`として合格させるため`expect`自身が部分リテラルでありうる。第0条`BAD_EXPECT`を追加。

## 目的と非目的

- 目的: 実Chrome（GJI・NICOLA ON）で「GJIが長くidleした後の最初の打鍵がリテラル化しない」ことを、
  CI（`e2e-ime.yml`）で自動判定できるようにする。BUG-002型（`という→toいう`）の症状を対象にする。
- 非目的: `bあ`（`9a7e699`）・awaseの分類ロジックの変更・tuning定数の変更・Tauri/Electron・
  クラス名偽装ハーネス（ADR-193「検討した代替案」、別ADR）。

## 設計の要点

### 1. 「idle → 打鍵 → 判定」の1試行

`chrome_probe`の既存`ensure(Setup::Kana)`で「IME ON・かな・Engine ON」を確認済みの状態から、
**キー入力なしで`idle`ミリ秒待ち**、`k`,`a`を目印付き`SendInput`で打ち、ページの`textarea`値を読む。
`ensure()`の最後は必ず`probe()`を通り、`probe()`が末尾でページを`clear`するため、idle前に別途`clear`は要らない。

- **判定は checker（Python）が唯一の判定主体**。`chrome_probe`は生の値をログに書くだけ
  （`classify`相当をRustとPythonに二重化しない。PythonはLinuxで単体テストできる）。
- 既存の`classify()`（`chrome_probe.rs`）は「かなが1文字でもあれば`Nicola`」で`kあ`を`Nicola`扱いするため、
  部分リテラルを見逃す。**既存`classify()`は変更しない**（既存8ケースの判定が変わる）。
- `IDLE`行の書式（案）: `IDLE idle={ms}ms n={i} utc={UTC} awase={bool} expect={ensure時のtext:?} text={text:?} process={bool} focus_lost={bool}`。
  `expect`は`ensure(Kana)`の最後の`probe()`が観測した`k`,`a`の出力（awase起動中はNICOLA文字）。`k`,`a`の出力は同じ配列・設定なら
  同じ文字列になるので、idle後の`text`と突き合わせる（特定のかなをハードコードしない=配列依存のR6も解消）。
  `process`は`probe()`が既に計算している`Process(229)`（`chrome_probe.rs:343-346`）。`utc`は`gji_idle_ms`の突き合わせ（設計2）に使う。
- checkerの判定（案、`awase=true`のとき。上から順に評価）:
  0. **`expect`が空、またはASCII英字`[A-Za-z]`を含む → `INVALID`（`BAD_EXPECT`）**。`ensure()`は既存`classify()`が`kあ`を`Nicola`と見なすため合格させる
     （`kana_ok`、`chrome_probe.rs:370-377`）ので、`expect`自身が部分リテラルでありうる。その回の比較は意味を持たない（`--pre-settle`はまさに
     「長idle直後の最初の打鍵」を`ensure()`内に作るので最も踏みやすい）。`kana_ok`を厳しくすると既存8ケースの判定が変わるため、checker側で閉じる。
  1. `text == expect` → `PASS`。
  2. `text`が空 → **`process`の値によらず`INVALID`（`EMPTY_PENDING`）**。`probe()`は`k`,`a`の後350msしか待たない（`:332-334`）ため、`process=true`の空は
     「compositionがまだ確定していない」タイミングの問題である可能性が高く、BUG-002型（リテラルが**出てしまう**）とは逆向き。FAILに入れると率にノイズが乗る。
     発生率が高ければ待ち時間を延ばす。
  3. **`text == "か"`かつ`expect != "か"` → `ENGINE_OFF`（INVALID）**。idle中にawaseが落ちた・フックが外れた・Engineが非活性になった状態
     （既存`classify()`の`RomajiKana`=Engine素通し）で、症状ではなくハーネス故障。FAILにすると撤去実験の率を汚す。
  4. 英字`[A-Za-z]`を含む → `process=true`なら`PARTIAL_LITERAL`（英字とかな等の混在）または`LITERAL`（英字のみ）=FAIL（BUG-002型）、
     `process=false`なら`PRECONDITION_DRIFT`（INVALID。IMEがキーを見ていない）。
  5. 上記以外（`expect`と異なり英字も含まない。例: 1文字欠落）→ `MISMATCH`（FAIL）。
  さらに**測定の成立を試行ごとにassertする**（PM7、設計2）: `idle_at_cold`が期待範囲外なら`INVALID`（`IDLE_MISMEASURED`）。
  `awase=false`（`--no-awase`の陽性対照腕）のとき: `text == "か"`（ローマ字かな変換そのまま）→ `PASS`、他は`INVALID`（ハーネス自体が
  キー注入・読み取りできていない証拠）。ただしIME自身がリテラルを出した場合は`control_literal`として別バケットに数える
  （`ch-idle-noawase`が全INVALIDのとき「対照が取れていない」のか「環境が壊れている」のかを区別するため。`text`も`TALLY`に添える）。
  加えて`focus_lost`・`前面化に失敗`・物理キー混入（`check_multi.py::phys_in_awase`をimport再利用）・`=== 全ケース完了 ===`が無い・
  起動ログに期待したフラグが出ていない（`chrome_probe`は未知の引数を黙って無視するため、起動ログの`activate_gji`・`idle_sweep`・
  `idle_repeat`・`pre_settle`を出力させて確認する）は`INVALID`。
- checkerは試行単位の集計行を**stdoutに**出す（runステップが`| Tee-Object -FilePath result.txt`でstdoutを`result.txt`に落とし、`summary`が
  artifactのそれを読む）: `TALLY pass=N fail=N invalid=N mismeasured=N total=N`と、掃引点別の`TALLY idle=8000 pass=N fail=N invalid=N mismeasured=N`（T4の判定に使う、設計4）。
- `utc_stamp()`（`HH:MM:SS.mmmZ`、日付なし）は`check.to_ms`と互換（`check.py:29-31`）。ただし日付を持たないため、**UTC 0時をまたぐ実行では
  `idle_at_cold`の突き合わせが壊れる**（既存ハーネスと同じ性質。対処する場合は負の差分に+86400000msする）。1ジョブ3.5〜4分なので通常は問題にならない。

### 2. 測る量は`gji_idle_ms`、掃引点は4帯を踏む

awaseがcold判定に使う量は**keyboard idleではなく`gji_idle_ms`**（`tsf::observer::gji_idle_ms()`、GJIのI/O観測からの
経過時間。`platform.rs:742,824,869,882`）。Chrome(VK)の帯は次の通り（`gji_fsm.rs`）:

| `gji_idle_ms` | 状態 | `forces_prepend_f2` | 根拠 |
|---|---|---|---|
| < 5s | OnWarm（LongIdleタイマー未発火） | — | `CHROME_LONG_IDLE_MS`=5sは`transition_to_warm`のタイマー長（`:470-479`、`long_idle_ms_for`経由） |
| 5〜7s | OnCold / `Short` | false | `ColdKind::classify`（`:127-136`） |
| 7〜10s | OnCold / `Medium` | **true** | `MEDIUM_IDLE_PROBE_MS`=7s |
| ≥ 10s | OnCold / `Long` | **true**（`is_long`も true） | `LONG_IDLE_MS`=10s |

**`ColdKind::classify`は`CHROME_LONG_IDLE_MS`(5s)を参照しない**。`tuning.rs:88-90`のdocコメント（「`ColdKind::classify`の
Short/Medium/Long重症度分岐のcutoffになる」）は実装と食い違っており**stale**（T6で直す）。
掃引点 **3000 / 6000 / 8000 / 11000 / 14000 ms** は、この4帯（<5 / 5–7 / 7–10 / ≥10）を過不足なく踏む。

- **`gji_idle_ms`は`ensure()`直後のGJI I/Oからの経過**であり、掃引点と一致するとは限らない（何かの拍子にGJI I/Oが走るとリセットされる）。
  さらに**GJI監視（`gji_monitor`）が未アタッチだと`gji_last_io_ms`が0のままで、`gji_idle_ms()`は`current_tick_ms()`=マシン稼働時間を返し**
  （`tsf/observer.rs:434-442`）、掃引の全点が`Long`（≥10s）に潰れる。`e2e-ime.yml:231`はセットアップでGJIプロセスを意図的に落とす
  （`Stop-Process`）ので、awase起動時点でGJIプロセスが不在で`try_attach`が失敗し、`gji_monitor_ok=false`のまま始まる現実味がある
  （`tsf/gji_monitor.rs:401-421`）。放置すると、**測定が一度も成立していないのに、G0が「出ない」で確定しうる**。
  したがって`idle_at_cold`は「記録する」のでなく**「検証する」**（不一致はINVALID）:
  1. **掃引開始前の前提**: awaseログに`[gji-monitor] attached to GJI process (I/O monitoring enabled)`（`gji_monitor.rs:401`）が無ければ、
     そのrun全体を`INVALID`。`chrome_probe`側は`--activate-gji`の後に1打鍵してGJIプロセスを立ち上げてから掃引に入る（`ensure()`が実質兼ねるが明示）。
  2. **試行ごとの突き合わせ**: `IDLE`行のUTCで、awaseログの`[h1-probe] cold=… idle_at_cold=…ms`（`vk_send.rs:279`）を突き合わせ、
     `|idle_at_cold − 掃引点| ≤ max(掃引点の30%, 1500ms)`（初期案、T3bで較正）を要求する。範囲外は`INVALID`（`IDLE_MISMEASURED`、TALLYの`mismeasured`）。
     `[h1-probe]`はcold経路（`prepend_f2_warmup`）でしか出ないため、行が無い場合は掃引点が7s以上のときだけ`IDLE_MISMEASURED`とし、
     それ未満は許容する（5s未満はOnWarmでcold経路に入らない。**未確認**、T0/T3bの実測で較正）。
  3. **G0の確定条件**: `mismeasured`が過半（初期案: 20%以上）の間は「出ない」を確定させない（判定不能）。
  この仕組みは、`--settle`/`--pre-settle`/ポーリング（R8）が`gji_idle_ms`に与える影響の検出にもなる。
  GJI休眠（~12s）が実際に起きているかも、これで直接測れる。
- T4候補1（`prepend_f2_warmup`撤去）が効くのは`gji_idle_ms`≥7s（`forces_prepend_f2`）＝掃引点8000/11000/14000。
  3000/6000で差が出ないのは正常。

### 3. GJIのアクティブ化とbelief合わせ（`--activate-gji`を`chrome_probe`へ写す）

既存スパイクの`--activate-gji`（`ime_key_matrix_spike.rs:1785-1794`）は**4つのこと**をする。`e2e-ime.yml:294-299`のコメントは③しか説明しておらず、
実装を読まずにコメントだけで仕様を決めると本質を取り違える（本計画v2の誤り）:

| # | 内容 | `chrome_probe`で必要か |
|---|---|---|
| ① | **`activate_gji_profile()`（`:1590-1613`）: `ITfInputProcessorProfileMgr::ActivateProfile`をGJIのCLSID/プロファイルGUIDに対し`TF_IPPMF_ENABLEPROFILE\|TF_IPPMF_FORSESSION`で呼ぶ**。docが理由を明記: 「CI(GitHub Actions)のように`Set-WinUserLanguageList`が次回サインインまで有効にならない環境用」。**これが無いとGJIが非アクティブTIPのままで、`ensure()`が全試行falseになる** | **必要（本質）** |
| ② | awaseがアクティブTIPを検出するまで14秒待つ（`:1788`） | 必要（awaseは起動済み。activate後に待ってからChromeを起動・前面化）。**TIP種別の検出とI/O監視のアタッチは別物**: awaseのTIP再検出は2秒周期+2tickデバウンスで2〜4秒（`gji_monitor.rs:371-398`、後からactivateしても働き14sは十分）だが、`gji_monitor_ok`（GJIプロセスへのI/O監視アタッチ）は別で、そちらは設計2の前提条件で見る |
| ③ | 手順の前に`VK_IME_OFF`を1回注入し、awaseのbelief(起動時推定=ON)と実状態(OFF)をそろえる（`:1790-1792`、CI run 35482240969） | 必要 |
| ④ | スパイク自身のLLフックの遅延インストール | 不要（`chrome_probe`はフックを張らない） |

- フラグ名は既存と揃えて**`--activate-gji`**にする（対応を見失わない）。①②③を行う。`--msime`分岐（`:1594-1604`）は`ch-*`がGJI固定なので**移さない**。
- **順序はスパイクと異なる（意図的）**: スパイクは`create_window()`→activate→14s後に手順開始、`VK_IME_OFF`は手順開始の2秒前に**自窓が前面の状態**で打つ
  （`:1784-1792`）。`chrome_probe`は自窓を持たず、IMEのオープン状態はフォーカス窓のコンテキストに効くので、③は**Chrome前面化の後**に置く
  （activate → awase検出待ち → Chrome起動 → 前面化 → `VK_IME_OFF` → ケース）。将来「スパイクと違う」と揃え直すと退行する。
- **COM初期化は`main`スレッドで**: `chrome_probe`はHTTPサーバを別スレッドで動かすが、`CoInitializeEx`（戻り値は無視でよい）と`CoCreateInstance`は同一の`main`スレッドで行う。
- **コード共有**: `activate_gji_profile()`は`examples/`内の関数（`ime_key_matrix_spike.rs:1590-1642`、**約51行**、`log_active`・`ActivateProfile`呼び出し・
  `sleep(1500)`+`log_active("後")`を含む）で、examples同士は`use`できない。(a)`chrome_probe.rs`に約50行を複製し（`chrome_probe.rs`には現在COM/TSFの参照が1つも無いので、
  `CoInitializeEx`/`COINIT_APARTMENTTHREADED`/`CoCreateInstance`/`CLSCTX_INPROC_SERVER`/`CLSID_TF_InputProcessorProfiles`/`ITfInputProcessorProfileMgr`/
  `GUID_TFCAT_TIP_KEYBOARD`/`TF_INPUTPROCESSORPROFILE`/`HKL`のimportも要る。featureは`awase-windows`パッケージ単位で有効なので`Cargo.toml`の追加は不要）、両側のコメントに
  出典と「複製である」旨を明記する（**既定**）、(b)`#[path]`で共有モジュール、(c)`awase-windows`のlibへ出す（決定4-4「本体ソースは新たには変更しない」に抵触するので不可）。
- `ActivateProfile`の戻り（HRESULT）と`GetActiveProfile`の結果をログに出す（スパイクの`log_active`相当）。G1の原因分解で「GJIが非アクティブTIP」を
  `PRECOND_FAIL`（belief不整合の疑い）から区別するために使う。
- 併せて`ensure()`がfalseを返した回数をログ末尾に出す（`PRECOND_FAIL=n`）。
- **未確認**: `FORSESSION`のアクティブ化を、ウィンドウを持たない`chrome_probe`（別プロセス）から呼んでも、後から起動するChromeのウィンドウに効くか。
  スパイクは自分の窓のプロセス内で呼んでいる。T3aで、activate後の`GetActiveProfile`ログと`ensure()`の成否で確かめる。

### 4. 撤去実験の判定は試行単位の率で行う

`summary`ジョブは各ジョブの`rc`（0=PASS/1=FAIL/3=INVALID）の数だけを見る。**1構成 = 15試行/ジョブ（5掃引点×`--idle-repeat=3`）× 3ジョブ（`matrix.run:[1,2,3]`）= 45試行**。
rcのANDに潰れると、ベースラインの1回のflakeで`NG`、撤去側は環境flakeだけで`OK`になり、「撤去に検出力がある」ことを示せない。

- checkerの`TALLY`行（構成全体と掃引点別、設計1）を`summary`が合算する（`TALLY`が無い既存構成は従来のrc集計にフォールバック）。
- 撤去が効くはずの帯（候補1なら8/11/14s）に絞ると、実効n = 3掃引点×3反復×3run = **27試行**。
- 採否は**率の対比**（初期案）: **ベースラインfail率≦10%**（27試行で2件まで）、**撤去ありfail率≧50%**、**両者の比が4倍以上**（比が小さければ環境要因を疑う）。
  n=27・p=0.5のとき撤去側のFAIL件数が2件以下になる確率は事実上0なので、ベースラインを10%に緩めても検出力は落ちない。
  fail率2%は27〜45試行で上限0.5〜0.9件=**0件要求**と同義で、1件のflakeでベースラインが恒常的に`NG`になり対照が成立しない。
  **T3bの`observe`実行で得た実測のflake率から較正するが、較正後も「ベースライン0件要求」にはしない**（PM3の再発防止）。
  「135」は「CI実行3回分を合算する」場合の数で、単一のCI実行の基準には使わない。
- 満たせない撤去候補は「E2Eで検出力を示せない」として`docs/experiments.md`に記録し、別候補へ移る（`--idle-repeat`を増やすのは最後の手段）。

### 5. CIへの載せ方

`e2e-ime.yml`の`cfg()`に`driver`フィールド（既定`spike`、新規`chrome`）を足し、`driver=chrome`の構成だけ`chrome_probe.exe`を走らせる。
別ワークフロー(`e2e-chrome.yml`)案は、既存390行を触らず爆発半径が小さい反面、GJI導入・言語設定・`config1.db`書き込みの
約50行を複製する。本計画は`driver`方式を採り、既存構成の回帰は「T3aの前後で既存`baseline`構成の結果が変わらないこと」で確認する。
`config.toml`書き換え部分を別ステップに切り出しても、`${{ }}`は各ステップで展開され、書き換え結果は`dist\config.toml`ファイルとして
次ステップへ渡るので成立する（pwshの変数はステップをまたいで生存しない点に注意）。確認用の`Select-String`行も一緒に移す。**切り出したステップには`if:`条件を付けない**（`driver`に依存しない。付けると`driver=chrome`側で`config.toml`が未加工のまま渡る）。

`cache.toml`へのImm capability事前投入が`chrome_probe`に**不要な見込み**な理由: 学習を行う`focus/imm_learning.rs:45-48`は
`new_app_kind != AppKind::Win32`なら早期returnし、ChromeはクラスがChrome_*（`detect_app_kind`が`AppKind::TsfNative`）なので
学習経路に入らない（`IMM32_UNAVAILABLE_CLASSES`は`AppImeProfile`の軸で別）。**要確認**（T3aで実際に誤学習が起きないことを見る）。

## 判定ゲート

- **G0（T1bの後に確定）**: 現行コードで、GJI・Chrome・長idle後に部分リテラルが**出る/出ない**。
  T0の`--settle`は「モードキー押下後の待ち」で、`gji_idle_ms`も伸びるが、モードキー直後にGJIがwarmへ戻っている可能性を
  区別できない。**T0の結論でG0を確定させず、T1b（idle-sweep）の結果で確定する**。
  - 出る → T4（撤去実験）へ。成功基準は「撤去あり=FAIL / なし=PASS」（設計4の率で判定）。
  - **`mismeasured`が20%以上（初期案）の間は、どちらも確定せず判定不能とする**（設計2。測定が成立していない）。
  - 出ない → ADR決定4-0の3条件（ablation必須・陽性対照・INVALID維持）をすべて満たす場合のみCIに載せる。
    ablationで現行機構のいずれかの撤去が症状を戻さないなら、そのassertは何も守っていないのでCIに載せず、
    `docs/experiments.md`に記録して終える。陽性対照（`--no-awase`腕）が取れない回は`INVALID`に落とす。
- **G1（T3aの後）**: CI上でChrome+GJI+awaseの既存8ケースが実行でき、結果が`observe`として集計される。
  失敗した場合は原因を分解する（`chrome_probe.exe`不在＝PB1、Chrome不在、前面化失敗、**GJIが非アクティブTIP**＝`ActivateProfile`のHRESULT/`GetActiveProfile`ログ、
  `PRECOND_FAIL`＝belief不整合の疑い）。`PRECOND_FAIL`だけではTIP非アクティブとbelief不整合を区別できない。
  「CIでは無理」と結論するのは、これらを切り分けたうえで、なお解けないときだけとする。

## タスク

### T0（コード変更なし）: 実機で粗い確認をする（G0は確定しない）

- 内容: Windows実機（GJI、awase debugビルド+`AWASE_TEST_INJECTION=1`）で既存`chrome_probe`のケース4
  （`半角英数→ひらがな=かな`、F2→待ち→打鍵でBUG-002の形）を`--settle=500`（対照）と`--settle=3000/6000/8000/11000/14000`で実行する。
  併せて`--no-awase`の対照も1回取る。**ログでは`PROBE action後:`行の`text=`を見る**（`Class`は部分リテラルを`Nicola`等に落とすため。
  `setup:…`・`action後2回目`行も同じ`PROBE `接頭辞なので`action後:`だけを絞る）。
- **実行時間**: `chrome_probe`にケース選択のフラグは無く、常に8ケース全部を回し（`for (i, c) in CASES.iter().enumerate()`）、`--settle`は全ケースに効く。
  `--repeat=1`を明記する（8ケース×(`ensure`1.2〜2.6s + settle + probe0.6〜1.1s)。settle14sで約2.4分/回×掃引6点≒15分。`--repeat`既定3だと3倍）。
- ログ回収: `chrome_probe.log`と、`idle_at_cold`突き合わせ用の`awase.log`の**両方**を回収する
  （`clipwire-targets.example.toml`の`Get-Content chrome_probe.log`の系統）。
- 実施: clipwireで実機へ（push→Windows側チェックアウトブランチ確認→ビルド→実行）。
- 成果物: 結果表を`tools/e2e/ime_key_matrix/results/`に置く。ADR-193へ「症状の兆候の有無」を追記（**G0は確定しない**）。
- 受け入れ基準: 掃引点ごとに`text`と`idle_at_cold`が記録される。
- 依存: なし。

### T2: checker（Python）を書く

- 変更: `tools/e2e/ime_key_matrix/check_chrome_probe.py`（新規）、`fixtures/chrome_probe/*.log`（新規）、
  `test_check_chrome_probe.py`（新規、`python -m unittest`でLinux実行可）。
- 内容: 2モード。
  - `cases`: 既存ログの`SUMMARY PASS=n RECOVER=n FAIL=n INVALID=n`と`RESULT`行を読む。`=== 全ケース完了 ===`が無ければ`INVALID`（rc=3）。
    終了コードは既存`check_multi.py`と同じ（有効回すべてPASS=0 / FAILあり=1 / 有効回なし=3）。`TALLY`行も出す。
  - `idle`: 設計1の判定。`idle_at_cold`をawaseログから`utc`で突き合わせて併記する。
- 受け入れ基準: fixture（PASS / PARTIAL_LITERAL / LITERAL / PRECONDITION_DRIFT / INVALID（完了マーカー欠落・期待フラグが起動ログに無い）各1件、実機ログ由来が理想）で
  単体テストが通る。`chrome_probe`の`utc_stamp()`は`check.to_ms`と互換（`check.py:29-31`、確認済み）。fixtureでも確認する。
  fixtureに`ENGINE_OFF`（`text="か"`）・`MISMATCH`（1文字欠落）・`BAD_EXPECT`（`expect="kあ"`かつ`text="kあ"`がPASSにならないこと）・
  `EMPTY_PENDING`・`IDLE_MISMEASURED`（`idle_at_cold`が掃引点と大きくずれる）・`gji_monitor`未アタッチのrun（全体INVALID）と、
  `awase=false`腕のPASS/INVALID/`control_literal`を含める。
- 依存: なし（T0と並行可）。

### T1a: `chrome_probe`にGJIアクティブ化とbelief合わせ・前提失敗カウントを足す

- 変更: `crates/awase-windows/examples/chrome_probe.rs`（+ `README.md`のフラグ表）。**規模は「フラグ1つ」ではない**: 約50行の`activate_gji_profile()`複製、COM初期化（`main`スレッド）とimport一式
  （設計3）、14秒の待ちが入る。
- 内容: 設計3の`--activate-gji`（①②③、`activate_gji_profile()`の約50行を出典コメント付きで複製）、`ActivateProfile`のHRESULT/`GetActiveProfile`のログ、
  ログ末尾の`PRECOND_FAIL=n`、起動ログに`activate_gji`・`idle_sweep`・`idle_repeat`・`pre_settle`を出力。
  流れは「`--activate-gji`ならactivate → awaseの検出待ち → Chrome起動 → `bring_to_front()` → `VK_IME_OFF`を1回注入 → 既存のケース」。
  既存の8ケースの挙動は`--activate-gji`無しで不変。
- 受け入れ基準: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --examples`が通る（`link.exe`不在のためビルド・実行は不可、CLAUDE.md）。
  実機（clipwire、GJIは元からアクティブ）で`--activate-gji`付き既存8ケースが**`PRECOND_FAIL=0`で完走**する。CI固有の「非アクティブTIP」の再現は
  実機では不可なので、T3aで`ActivateProfile`の効果を確認する。
- 依存: なし。

### T3a: `driver=chrome`をCIに載せ、既存8ケースのsmokeを回す

- 変更: `.github/workflows/e2e-ime.yml`のみ。
  1. **ビルドキャッシュ（PB1）**: `hashFiles(...)`に`'.github/workflows/e2e-ime.yml'`を追加する（無いと、`ch-smoke`は`bin='baseline'`で
     キャッシュが必ずヒットしてビルドがスキップされ、`dist\chrome_probe.exe`が無いまま走る）。
     受け入れ基準に`Test-Path dist\chrome_probe.exe`の確認を入れる（無ければ即`INVALID`）。
  2. `cfg()`に`driver='spike'`を追加。新規構成`ch-smoke`（`driver='chrome'`, `expect='observe'`, `check='chrome-cases'`, `args='--activate-gji'`）。
  3. `build`ジョブで`--example chrome_probe`も`dist/`へコピー（全構成、コンパイル時間の増分は**未確認**）。
  4. 既存runステップの`config.toml`書き換え部分（約12行と確認用`Select-String`）を共通ステップに切り出す（機械的な抽出。既存構成の挙動を変えない）。
  5. `driver=chrome`用のステップ: Chromeの有無確認（`Test-Path 'C:\Program Files\Google\Chrome\Application\chrome.exe'`、無ければ
     `choco install googlechrome -y`）→ awase起動（debug、`AWASE_TEST_INJECTION=1`、`RUST_LOG=debug`）→ `awase.log`に起動完了が出るまで確認 →
     `chrome_probe.exe --activate-gji … --log=chrome_probe.log`を時間制限付きで実行（ログに`=== 全ケース完了 ===`が無ければ checker が`INVALID`）→
     `check_chrome_probe.py`（T2）→ `rc.txt`。`chrome_probe`は`--activate-gji`付きで起動する（設計3）。
  6. upload artifactの`path:`に`dist/chrome_probe.log`・`dist/awase.log`を追加し、存在しない側（`driver`違い）があっても落ちないことを確認する。
  7. `plan`の`ONLY`に、`ci/e2e-chrome`ブランチのとき`ch-*`を選ぶ分岐を追加し、`on.push.branches`にも`ci/e2e-chrome`を足す。
- 受け入れ基準: 起動ログに`--activate-gji`が効いた旨が出る（T1aより前にビルドされたバイナリでは、未知の引数が黙って無視されるため）。
  `ci/e2e-chrome`へのpushで`ch-smoke`が3回実行され、artifactに`chrome_probe.log`が残る（結果はFAILでもよい=G1、ただし
  G1の原因分解ができるだけの情報＝`PRECOND_FAIL`・`Test-Path`結果・awase起動ログが残る）。既存`baseline`構成の結果がT3a前後で変わらない。
- 依存: T2, T1a。

### T1b: `chrome_probe`にidle-sweepを足す

- 変更: `crates/awase-windows/examples/chrome_probe.rs`のみ（+ `README.md`）。
- 内容: `--idle-sweep=<ms,ms,…>`と`--idle-repeat=N`（既定3）。指定時は既存の8ケースを回さず、各`(idle, 反復)`で
  `bring_to_front()` → `ensure(Setup::Kana)` → `sleep(idle)` → `k`,`a`（既存`probe()`と同じ押下: 30ms保持・30ms間隔・350ms待ち）→ `snap`で`text`取得 →
  `IDLE`行（設計1の書式）。`ensure`失敗時は`RESULT INVALID: 前提状態にできなかった`。`probe()`を内部関数と既存の分類付きラッパーに分ける
  （既存呼び出しの挙動は不変）。`expect`を得るため`ensure()`が最後の`probe()`の`text`も返すよう変更する（現状は`-> bool`、`chrome_probe.rs:370`。
  例: `-> Option<String>`。既存8ケースの呼び出し側（`:648`）も直す。`Setup::Kana`の最後のprobeは`setup:IME_ON後`か`setup:ひらがな後`のいずれかで一意）。**判定（PASS/FAIL）はログに書かない**。
  「keyboard short idle かつ GJI long idle」（旧BUG-002表の物理F2+GJI休眠）を作る`--pre-settle=<ms>`（モードキー押下の**前**に待つ）も
  ここで足す（ADR決定4-1）。
- ページの30ms周期`fetch('/cmd')`ポーリング（`chrome_probe.rs:57-64`）が、idle中もレンダラを起こし続ける。`gji_idle_ms`には影響しないが、
  Chrome側のidle挙動（TSF composition contextの破棄・再初期化）を歪めうる。**まず現状（30ms）で測り、結果が不安定なら間引き版
  （idle中のみ1000ms、停止はしない=`snap`/`clear`が届かなくなる）と比較する**。
- 受け入れ基準: `cargo check … --examples`が通る。実機で`--idle-sweep=3000,11000 --idle-repeat=2`が完走し`IDLE`行が出力される。
  T2の`idle`モードがその実機ログをパースでき、`idle_at_cold`が掃引点と突き合わせられる。
- 依存: T2（入力形式の固定）、T3a（CI smokeで土台が動くことを確認してから）。

### T3b: `ch-idle`構成を追加

- 変更: `.github/workflows/e2e-ime.yml`のみ。`cfg('ch-idle', 'observe', driver='chrome', check='chrome-idle',
  args='--activate-gji --idle-sweep=3000,6000,8000,11000,14000 --idle-repeat=3')`。
  **陽性対照腕**として`cfg('ch-idle-noawase', 'observe', driver='chrome', check='chrome-idle', awase='false', args='--activate-gji --no-awase --idle-sweep=… ')`も
  同時に追加する（ADR決定4-0の陽性対照。`awase=false`ではawaseを起動せず、`text=="か"`を期待、設計1）。
  あわせて`summary`が`TALLY`（構成全体・掃引点別）を合算するよう拡張（設計4、`TALLY`が無ければ従来のrc集計）。
- 見積もり式: Σ(idle)×repeat + 試行数 ×（`ensure`1.2〜2.6s + 打鍵0.6〜1.1s + 前面化）。Σidle=42s、repeat=3、15試行 → 126s + 15×(3〜5s) ≒ **3.5〜4分/ジョブ**
  （Chrome起動は最大40s待ち+1.5s+0.8sでプロセス当たり1回）。matrixの`run:[1,2,3]`で3ジョブ。`timeout-minutes: 25`内の見込み
  （GJI導入約30秒+`ctfmon`再起動等は別、実測で確認）。`--idle-repeat`を増やす場合は式に戻して見積もり直す。
  **25分を超える場合の対処順**: 掃引点を減らす → 構成を分ける → `run:[1,2,3]`を減らす（別ワークフロー化は最後）。
- 受け入れ基準: `ci/e2e-chrome`で`ch-idle`が3回完走し、`observe`として集計される。`TALLY`から実際のベースラインflake率（掃引点別）が得られる。`ch-idle-noawase`が全試行PASS（ハーネス自体が動いている証拠）。
  `mismeasured`の割合が得られ、G0確定の許容（20%未満）に収まる、または収まらない原因（GJI監視の未アタッチ等）が特定できる。
- 依存: T1b, T3a。

### T4（G0が「出る」、または「出ない」でもablationが効くと示したいとき）: 撤去スクリプトを追加

- 変更: `tools/e2e/ime_key_matrix/ablations/a8-*.sh`（新規、a5と同形式=Pythonで`assert a in s`して文字列置換）、
  `e2e-ime.yml`の`plan`に`cfg('ch-a8-…', 'fail', mutator='a8-….sh', driver='chrome', check='chrome-idle', …)`。
- 撤去対象の候補（**どれが症状を防いでいるかはT1b/T3bの結果で決める。現時点では未特定**）:
  1. `output/vk_send.rs`のcold-start分岐（`prepend_f2_warmup`、`:256`。定義`output/mod.rs:405`、条件`:1435`）を無効化する。
     **有効域は`gji_idle_ms`≥7s**（`forces_prepend_f2`）＝掃引点8000/11000/14000のみ。3000/6000で差が出ないのは正常。
  2. `tsf/warmup/probe_fsm.rs::run_per_vk_confirm`（**`:454`**）が各VKを即confirm扱いにする。
  3. `RawTsfLiteralRecovery`の回収分岐: **所在を確定してから**候補にする（`literal_detect_fsm.rs`には無い。出現は
     `output/tsf_warmup_coord.rs:411,607,633`、`output/mod.rs:309,314,1827,1982,1985`）。どのファイルのどの分岐を撤去するかを先に特定する。
- 採否基準: 設計4（試行単位のTALLY、ベースラインfail率≦10%・撤去ありfail率≧50%・比4倍以上、撤去が効くはずの帯（n=27）で差が出ること。数値はT3bで較正し、0件要求にはしない）。
  満たせない候補は`docs/experiments.md`に記録して別候補へ。
- 受け入れ基準: `summary`で撤去構成が`OK`（期待FAILで実際にFAIL）、ベースライン構成が`OK`（期待PASS）を、**別々の2回のCI実行で再現**する。
- 依存: T3b、G0。

### T5: 記録を更新する

- 変更: `docs/known-bugs/BUG-002.md`（T4の結果に基づき「現在の対策」を per-VK confirmに書き直す）、ADR-193のstatus・決定4の結果欄、
  （新規バグ発見なら）`docs/known-bugs/BUG-NNN.md`を次の連番で新規作成（frontmatter規約に従う）、（撤去しても壊れない場合）`docs/experiments.md`に1行。
- 受け入れ基準: known-bugsの「現在の対策」が現行コードに存在する定数・関数だけを指す（`grep`で存在を裏取り）。
- 依存: T4（または「出ない」判定）。

### T6（独立・いつでも）: `tuning.rs`のstale docを直す

- 変更: `crates/awase-windows/src/tuning.rs:88-90`のdocコメントのみ。`CHROME_LONG_IDLE_MS`(5s)は`transition_to_warm`のOnWarm→OnColdタイマー長
  （`gji_fsm.rs:470-479`）であり、`ColdKind::classify`のcutoffは`MEDIUM_IDLE_PROBE_MS`(7s)/`LONG_IDLE_MS`(10s)であることを書く。
- **定数値は変えない**（`tuning-constants.md`の実測義務は発生しない）。pre-pushフックが再発ファミリー対象ファイルの変更として
  警告を出しうるが、コメントのみの変更でありテスト・known-bugsの追加は不要（警告は無視してよい旨をコミット本文に書く）。
- 受け入れ基準: `cargo check --target x86_64-pc-windows-msvc -p awase-windows`が通る。
- 依存: なし。

## 着手順序

```
T0 ─────────────────────────────────────────────────┐(粗い確認のみ)
T2 ─┐
T1a ┴► T3a ─(G1)► T1b ─► T3b ─► (G0確定) ─► [T4] ─► T5
T6（独立）
```

T0・T2・T1a・T6は並行可。T3aはT1bより先（ハーネスがCI上で動くかの確認を、新機能の実装より先に行う）。
**G0はT1b（idle-sweep）の結果で確定する**（T0では確定しない）。実装ブランチは`develop`から専用worktreeで切る（`worktree-per-session`）。
ADR・本計画のdevelopへの反映を先に行う。CI実行は`ci/e2e-chrome`ブランチへのpush（`ci/e2e-scenarios`と同じ運用）。

## 規約との関係

- 変更対象は`examples/`・`tools/`・`.github/workflows/`・docsと、T6の`tuning.rs`のコメントのみ。`fix-requires-evidence.md`の再発ファミリー
  対象ファイル（`src/`配下のwarmup/focus/belief等）のロジックは**変更しない**。撤去スクリプトはCI実行時に作業ツリーへ差分を当てるだけ。
- `tuning.rs`の定数を新設・変更しない、`RESTRICTED_CALLS`許可リストも触らない（`complexity-budget`対象外）。
- checkerはfixtureを持つPython単体テストで担保する（E2E自体は実機依存でLinuxでは走らない）。
- 本ADRが自ら掲げる規律: **known-bugsの「現在の対策」だけでなく、`tuning.rs`等のdocコメントも、workflow（`e2e-ime.yml`）のコメントも、
  現行実装で裏取りしてから前提にする**（PM1でdocコメントの鵜呑みが計画・ADR・レビュアー自身の指摘にまで伝播し、PB2で`e2e-ime.yml`のコメントだけを読んで
  `--activate-gji`の本質を取り違えた。同型の誤りは、既存資産・known-bugs・`tuning.rs`のdoc・ymlのコメントと4回続いた）。

## リスク

| # | リスク | 対策 |
|---|---|---|
| R1 | `chrome_probe`はCIで一度も実行されておらず、Chromeの前面化・初回起動ダイアログ・ループバックHTTPがランナーで動くか不明 | T3aのsmokeを先行し、G1で原因を分解して判断する |
| R2 | GJI休眠（~12s）が実際に起きているか、`gji_idle_ms`が掃引と連動しているか | `idle_at_cold`を試行ごとに**検証**（範囲外はINVALID、`mismeasured`をTALLY・summaryに出す、設計2） |
| R3 | 実IMEのflake（1回の失敗で判定が揺れる） | 試行単位のTALLYと率で判定（設計4）。`observe`から始め、ベースラインflake率を測って数値を較正 |
| R4 | e2e-ime.ymlの共通ステップ切り出しで既存構成が壊れる | 機械的抽出に限定し、T3a前後で`baseline`の結果を比較 |
| R5 | T0の`--settle`は「モードキー後の間隔」で、キー無入力idleとは別物 | T0はG0を確定させず、確定はT1b |
| R6 | NICOLA出力の文字が配列・設定に依存する | 判定を「ASCII英字を含まない」にし、特定のかな範囲を要求しない |
| R7 | GJIが非アクティブTIP、またはbeliefのずれで全試行が`INVALID`になり、G1で誤診する | `--activate-gji`（`ActivateProfile`+VK_IME_OFF、T1a）、`ActivateProfile`のHRESULTログと`PRECOND_FAIL`カウントで原因を分解 |
| R8 | ページの30msポーリングがChromeのidle挙動を歪める | まず現状で測り、不安定なら間引き版と比較（T1b） |
| R9 | ビルドキャッシュが古いままT3aを走らせる | `hashFiles`にworkflowを追加、`Test-Path dist\chrome_probe.exe`を受け入れ基準に（PB1） |
| R10 | `ActivateProfile(FORSESSION)`を別プロセスの`chrome_probe`から呼んで、後から起動するChromeに効くか不明 | T3aで`GetActiveProfile`ログと`ensure()`成否を確認。効かなければChrome起動後にactivateする順序を試す |
| R11 | idle中にawaseが落ちる・Engineが非活性になり、`か`がPASS扱いになる | `expect`との突き合わせで`ENGINE_OFF`（INVALID）に落とす（設計1） |
| R12 | GJI監視（`gji_monitor`）が未アタッチで`gji_idle_ms()`がマシン稼働時間を返し、全点が`Long`に潰れる（`e2e-ime.yml:231`がGJIを落とす） | 掃引前に`[gji-monitor] attached`を確認（無ければrun INVALID）、`idle_at_cold`の突き合わせ、`mismeasured`過半ならG0を確定しない |
