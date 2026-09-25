# ADR196-T2「1e前半」配線案（提案A・提案B）への設計レビュー

対象: `docs/tasks/adr196-t2-mismatch-adjudication.md` 1e前半＝`judge_self_verification`等を
`crates/awase-keymap-learn-win/src/main.rs::run_main`へ接続する配線。
読んだ一次情報: ADR-196全文、T2タスク、`judgement.rs`、`known_keymap.rs`、`persist.rs`、
`key_effect_runtime.rs`、`key_effect_predictor.rs`（`from_config`/`is_unmodified_bundled_config`）、
`awase-keymap-learn-win/src/{main,driver,lib}.rs`、`external_write.rs`、`verify.rs`、`exec.rs::press`、
`tsf/tip_detector.rs`、`state/ime_kind.rs`、`gji_charset_autodetect.rs`、`awase-settings`の
`keymap_learn_launcher.rs`/`main.rs::start_keymap_learning`・Cargo.toml群。

---

## 結論（先に）

- **提案A（awase-settings側でTSFに問い合わせ、結果をCLI引数で渡す）は採用しない方がよい。**
  学習プロセスは既に「COM/TSFの土台」を持っている。前提が事実と違う
  （`driver.rs::RealImeDriver::new`が`CoInitializeEx(COINIT_APARTMENTTHREADED)`、`CoCreateInstance(CLSID_TF_ThreadMgr)`、
  `ITfThreadMgr::Activate`を実行済みで、学習窓を持つSTAスレッドそのものがTSFに参加している）。
  判定は**学習プロセス自身が、学習窓のスレッドで**行う（代替案の側）。これはコスト・正確性・安全側既定値の
  どの面でも提案Aより良い（詳細はA-1〜A-5）。
- **提案Bの「学習プロセスが自分でconfig1.dbを読む」方針は妥当。** ただし以下の4点を変える
  （B-1〜B-4）。(1) `learn-win → awase-gji-config`の直接依存は追加せず、`awase-windows`側に
  「突き合わせに使う同梱プリセットを返す」pub関数を1つ作る。(2) TIP同定（GJIかどうか）でゲートする。
  (3) 開始時と終了時の両方で読んで比較する。(4) **`known_keymap::classify_known_gji_keymap`が
  `session_keymap == None`（GJIの既定構成）を「既知でない」と判定しているバグ**を先に直す。
- **論点3で、実装前に解決すべき前提漏れが3件ある**（C-1〜C-3）。1e前半を「judgementを埋めるだけ」で
  実装すると、ADR-196が「失敗」と定めるセッション（外部書き込みの上限超過）で`Accepted`を書いてしまい、
  最小サンプル数300の条件も満たさないまま判定が走る。また読み手（段階4）は`judgement`を一切読まないため、
  `Rejected`と書かれた表がそのまま採用される。

具体的な変更手順は末尾の「推奨する実装順序」にまとめた。

---

## 論点1: `is_ms_ime_native`の入手方法

### A-1【Blocker級の前提誤り】学習プロセスは既にCOM STA＋TSFスレッドマネージャを持っている

`crates/awase-keymap-learn-win/src/driver.rs:108-113`:

```rust
unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
let thread_mgr: ITfThreadMgr = unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)? };
unsafe { thread_mgr.Activate()? };
```

さらに`observe_tsf()`はこのスレッドの`ITfCompartmentMgr`からcompartmentを読んでいる。
「`awase-keymap-learn-win`は現状COM/TSFの土台を一切持たない」というのは誤り。
代替案（学習プロセス自身が問い合わせる）の追加コストは、同じスレッドで
`CoCreateInstance(CLSID_TF_InputProcessorProfiles)`→`GetActiveProfile(GUID_TFCAT_TIP_KEYBOARD)`を
1回呼ぶだけになる。COMの初期化を新たに足す必要は無い。

### A-2【Must-fix】提案Aは「学習した窓のIME」ではなく「awase-settingsのUIスレッドのIME」を測ってしまう

TSFのアクティブプロファイルはスレッド単位で持つ。「アプリウィンドウごとに異なる入力方式を設定する」
（Windowsの言語設定）が有効な環境では、awase-settingsの窓（ユーザーがGJIを使っている）と、学習プロセスが
新規作成した`AwaseKeymapLearnWindow`（既定の入力方式＝Microsoft IMEで開始しうる）とで、アクティブなTIPが
食い違う。
失敗シナリオ: awase-settingsではGJIがアクティブなので`--ms-ime-native`を付けずに起動する。学習窓は
Microsoft IME本体で学習し、`judge_self_verification(.., false, ..)`で`Accepted`になる。こうして
「Microsoft IME本体の表は既定では採用しない」（決定1a）が素通りされる。
`is_ms_ime_native`が表すべきなのは「**この表を測ったIME**」だけなので、`driver.rs`の`edit`窓を持つ
STAスレッドで問い合わせるのが唯一正しい場所になる。

### A-3【Must-fix】CLI引数方式は、フラグが無いときの既定値が危険側に倒れる

`--ms-ime-native`フラグが無い状態が`false`（＝`Accepted`になりうる）になる。フラグを付け忘れた起動が
すべて危険側に倒れる。具体的には、開発者やCIが`awase-keymap-learn-win.exe`を直接叩くとき、今後
「判定書き換えモード」（1b-8）などの別経路から起動するとき、awase-settingsの古いビルドと組み合わせたとき。
3値の引数（`--ime=gji|msime-native|other`で必須）にすれば回避できるが、A-2の問題は残る。

### A-4【Should-fix】awase-settings側でCOMを使う場合の副作用（提案Aを採る場合のみ該当）

- awase-settingsはeframe（winit）のGUIスレッドを持つ。winitのWindows実装は、ドラッグ&ドロップが有効だと
  窓の作成時に`OleInitialize`（STA）を呼ぶ。rfdの同期ダイアログも呼び出したスレッドでCOMを初期化する
  （いずれも実装依存。未検証なので、確認してから依存すること）。UIスレッドで`CoInitializeEx(STA)`を
  呼ぶと`S_FALSE`が返り、呼んだ回数と同じだけ`CoUninitialize`が必要になる。釣り合いを崩すとrfdや
  D&Dのアパートメントを早期に破棄しうる。
- 別スレッドで行うと、COMの問題は消えるがA-2の問題が悪化する（新規スレッドのプロファイルは
  UIスレッドのものと一致する保証すら無い）。
- `awase-settings`は`windows` 0.58、`awase-windows`は0.62を使っている。pub関数のシグネチャに
  `ITfInputProcessorProfileMgr`等の`windows`型を出すと、awase-settingsからは型が合わず呼べない。
  自己完結の`fn() -> Option<TipIdentity>`にする必要がある。

以上から、提案Aは「COM副作用に気をつける」以前に採る理由が無い。

### A-5【Should-fix】新設pub関数の形：GJIのCLSID発見は必要、`TSF_OBS`への書き込みは不要

- `is_ms_ime_native`だけなら、`state/ime_kind.rs::identify_tip`は`MS_IME_JA_TIP_CLSID`との比較だけで
  `MsImeNative`を返すので、`discover_and_cache_gji_clsid`は要らない。ただしB-2（config1.dbを使うのは
  学習対象がGJIのときだけ）のために3値の`TipIdentity`（`Gji`/`MsImeNative`/`Other`）が必要で、`Gji`の
  同定には`find_gji_clsid`（`EnumProfiles`＋説明文字列に"Google"を含むか）が要る。したがって
  **`TipIdentity`をまるごと返す関数**にする。
- 既存の`tip_detector::query_active_kind`は`TSF_OBS.set_ime_product_name(..)`（awase-windowsの
  プロセスグローバルな観測ストア）へ書き込む副作用を持つ。学習プロセスからこれを呼ぶと、awase.exeの
  文脈でしか意味の無いグローバル（ADR-164が集約対象にしている種類のstatic）を別プロセスで初期化・
  更新してしまう。新関数は`query_active_kind`を流用せず、`GetActiveProfile`→`identify_tip`の純粋な
  部分だけを共有する形に切り出す。
  - 案: `tsf/tip_detector.rs`に`pub fn query_tip_identity_on_current_sta() -> Option<TipIdentity>`を
    新設する。内部で`create_profile_ctx`、`find_gji_clsid`（`OnceLock`キャッシュを経由しない直呼び。
    学習プロセスは短命で、1セッション2回しか呼ばない）、`GetActiveProfile`、`identify_tip`を順に行う。
    **COMの初期化は呼び出し側の責任とし、関数内では行わない**（`RealImeDriver`が初期化済み。関数内で
    `CoInitializeEx`/`CoUninitialize`を対にすると、呼び出し元のアパートメントの寿命を乱すため）。
    `tip_detector.rs`冒頭のスレッドモデル注記（「`gji-io-monitor`から呼ぶこと」）も更新する。
  - `tsf`モジュールは`pub(super)`で閉じているので、公開するのはこの1関数だけにする
    （`pub use`を`lib.rs`か`tsf/mod.rs`に1行）。

### A-6【Must-fix】学習中のIME切り替えへの対処：開始時と終了時の同定を比べ、食い違えばセッション失敗

学習は数分から20分（ADR-195のGJI予算）かかる。その間にユーザーがIMEを切り替えると、表の前半と後半で
測った対象が別物になる。GJIのMS-IMEプリセットとMicrosoft IME本体は変換モードの値域が同じなので、
`normalized_mode`の復号エラーにも出ず、黙って混ざる。
対処（コストはほぼゼロ）:
1. `RealImeDriver::new`の最後（quiet windowの後、`initial`の観測と同じ位置）で`TipIdentity`を取得し、
   driverに保持する。**ここで`None`（取得失敗）なら学習を始めずにエラー終了する。** 20分学習した後で
   「IMEが分からない」と判明するより、開始直後に失敗させる方が安い。これがエラー処理方針の核になる
   （C-5参照）。
2. 学習と検証ウォークの後、永続化の前にもう一度取得する。開始時と異なる、または`None`なら
   セッション失敗として表を書かない（決定1b項目5の「セッション全体を失敗として終了し、表を書き出さない」
   と同じ扱い）。
3. 任意の追加策: `driver.rs::press`の`check_session_interference`の隣で、N打鍵ごとに`GetActiveProfile`を
   ポーリングする（プロセス内呼び出しで軽い）。切り替えて戻す往復も検出できる。開始・終了の2点比較だけ
   でも大半は捕まるので、最初のPRでは1・2だけでよい。

---

## 論点2: 既知3構成の判定に使うGJI設定の読み込み

### B-1【Should-fix】`learn-win → awase-gji-config`の直接依存は規約違反ではないが、`awase-windows`経由の1関数にまとめる方がよい

依存方向の確認:
- `awase-gji-config`の依存は`tracing`のみの葉クレート。`awase-windows`は既にこれに依存し、
  `learn-win`は`awase-windows`に依存しているので、`learn-win → awase-gji-config`を足しても循環は生じない。
- ADR-195（`persist.rs`冒頭のdocに転記済み）の「型はOS非依存の本クレート（awase-keymap-learn）側で定義し、
  書き手`awase-keymap-learn-win`→本クレートの依存だけで成立させる。`awase-windows`→本クレートの依存は
  段階4側で発生させる」は、**`awase-keymap-learn`（純粋ロジック）が`awase-windows`に依存しないこと**を
  求める規約。`learn-win`の依存先はこの規約の対象外で、既に`awase-windows`へ依存している
  （`Conv`、`diff_against_bundled`）。

それでも直接依存を推奨しない理由:
- 「既知構成か」の定義が**すでに2箇所で食い違っている**（B-4）。学習プロセス側が
  `parse_top_level`＋`classify_known_gji_keymap`を独自に組み立てると、読み手（`key_effect_predictor.rs::
  is_unmodified_bundled_config`）と書き手の定義が別々のコードに固定されてしまう。
- `diff_against_bundled(persisted, preset: KeymapPreset)`が要求するのは`KeymapPreset`で、
  `KnownGjiKeymap`から`KeymapPreset`への対応付けも`awase-windows`側の型に依存する。

推奨: `gji_charset_autodetect.rs`の`windows_impl`（`read_key_effect_keymap`の隣）に次を新設する。
`read_config1_db`は`pub(crate)`のまま広げない。

```rust
/// ADR-196決定1c: 学習対象のIME（学習窓で同定したTipIdentity）と現在のconfig1.dbから、
/// 同梱表との突き合わせに使うプリセットを返す。既知構成でなければNone。
pub fn bundled_preset_for_adjudication(tip: TipIdentity) -> BundledPresetLookup
```

戻り値は`Known(KeymapPreset)` / `NotKnown` / `ConfigUnreadable`の3値にし、読めなかった場合を
「既知でない」と区別する（B-3・C-5）。`learn-win`は`awase-windows`だけを呼ぶので、Cargo.tomlは変わらない。

### B-2【Must-fix】config1.dbは「どのIMEで学習したか」とは無関係に存在するので、TIP同定でゲートする

`config1.db`はGJIがインストールされていれば、学習中のIMEがGJIでなくても読める。
失敗シナリオ: GJIを「ATOKプリセット」に設定したまま、JustSystemsのATOK本体（`TipIdentity::Other`）か
Microsoft IME本体で学習した。提案Bのままだと`classify_known_gji_keymap`が`Some(Atok)`を返し、GJIの
ATOK同梱表と大量に不一致になって全セルが再測定され、1b-8の30%超過で`NeedsConfirmation(SystematicMismatch)`
になる。誤った突き合わせ相手による偽の「要確認」である。
規則: `TipIdentity::Gji`のときだけconfig1.dbを見る。`MsImeNative`は既知構成判定が未実装（T2タスク1c
「未着手のうちは既知構成と判定しない」）なので`NotKnown`、`Other`は常に`NotKnown`。
この点で論点1（TIP同定）と論点2（config読み取り）は独立でなく、**論点2は論点1の結果に依存する**。
両方を同じプロセス（学習プロセス）で行うべきもう1つの理由になる。

### B-3【Should-fix】config1.dbも開始時と終了時の2回読み、内容が変わっていればセッション失敗にする

「セッション終了時点で1回読む」だけでは、学習中にユーザーがGJIのキー設定を変えた場合に、前半の測定と
食い違う設定で既知構成と判定してしまう。ADR-196決定3aの追記は「キーマップ設定自体の変更は即時失効」と
しているので、セッション中の変更はそのセッション自体を無効にするのが一貫する。
実装: 開始時に`config1.db`のバイト列（または`config1_db_stamp`の(mtime, len)）を保持し、終了時に比較する。
T5のフィンガープリントの計算材料がちょうどこれなので、将来`with_fingerprint`に流用できる。

### B-4【Must-fix・既存コードのバグ】`classify_known_gji_keymap`はGJIの既定構成（`session_keymap`不在）を既知構成と判定しない

`crates/awase-gji-config/src/known_keymap.rs:291-312`は`session_keymap`が`None`なら`_ => None`になる。
一方、読み手の`key_effect_predictor.rs::from_config`（529-531行）は
`None | Some(SESSION_KEYMAP_NONE | SESSION_KEYMAP_MSIME) => KeymapPreset::MsIme`とし、コメントで
「不在/NONEはWindows版GJIの既定でMSIME相当」と明記している。protobufは既定値のフィールドを省略して
直列化するので、**GJIのキー設定を一度も変えていないユーザー（最も多い構成）で`session_keymap`は`None`
になる**。
失敗シナリオ: 既定構成のGJIユーザーが学習すると、`classify_known_gji_keymap`が`None`を返して
内蔵表との突き合わせが丸ごと飛ばされる。ADR-195 B-1の回答部品だった「読めない窓での誤予測の防止」と、
1b-8の系統的バグへの安全弁の両方が、最多構成で無効になる。
もう1点、`custom_keymap_table`の扱いも食い違っている。読み手の`is_unmodified_bundled_config`は
`custom_table.is_none()`を要求するので、空文字列や無関係な行だけでも「既知でない」になる。
`classify_known_gji_keymap`は空・無関係な行なら既知と判定する（ADR-196 1cの定義はこちら）。
対処: `classify_known_gji_keymap`に`None`と`Some(SESSION_KEYMAP_NONE)`をMSIMEプリセットとして扱う枝を
加え、テスト`custom_session_keymap_is_never_known`の`classify_known_gji_keymap(None, &[], None) == None`
という期待値を反転する。読み手側の5%チェック（`is_unmodified_bundled_config`）は、C-3で退役させるまで
現行定義のまま残してよい。ただし、1c（ADR）が正とする定義は`known_keymap.rs`側だとコメントで明示する。

---

## 論点3: その他の設計上の問題

### C-1【Blocker】ADR-196のセッション失敗条件がどこにも配線されていない。このまま判定を足すと、失敗すべきセッションに`Accepted`を書く

`driver.rs:353-358`は`let _session_should_fail = self.check_session_interference();`で結果を捨てている。
`session_invalidated_trials()`、`observation_alive()`、`measurement_suspicious()`はリポジトリ内のどこからも
呼ばれていない（grepで確認済み）。
ADR-196決定1b項目4・5は「フックの自己注入が1件でも観測されなければセッション失敗」「無効化がN回を
超えたらセッション全体を失敗として終了し、表を書き出さない」と定めている。現状の`run_main`は常に
`persist_learned_table`まで進む。1e前半で`judge_self_verification`だけを足すと、**awase.exeが学習窓へ
warmupを打ち込み続けたセッション（A'の崩れ）でも、正答率が95%を超えれば`Accepted`と書く**。
ADR-196が最も防ぎたかった経路である（A'の崩れは自己検証スコアに現れない、round1の指摘）。
対処: 判定より前に、次のゲートを`run_main`に入れる。
1. `SessionMonitor`に`is_failed()`を足す（`invalidated_trials > invalidation_limit`）。`driver`に
   `session_failed() -> bool`を公開し、学習ループ（`run`/`retry_nondeterministic_cells_once`）の後と
   検証ウォークの後に確認する。
2. `hook_monitor.liveness().is_alive()`（自己注入が全件観測されたか）を同じ位置で確認する。
3. どちらかが偽なら、表を書かずに`result status=failure reason=external_write|hook_lost|ime_switched`を
   出して終了する。**「不採用（Rejected）として書く」のではなく「書かない」**（ADR 1b項目5の文言どおり。
   `RejectedReason`にこれらを足さない）。

C-1は「1e前半」の前提そのものなので、同じPR（最低でも先行するPR）で入れること。

### C-2【Must-fix】最小サンプル数300（決定1a、round2 NM3-2）が満たされないまま判定が走る

`main.rs`の`VERIFICATION_WALK_STEPS = 300`は**全ステップ数**である。ADR-196 1aは
「**予測した**ステップ数（`correct + incorrect`）が最低300」を求めている。縮退率20%の上限いっぱいなら
予測は約240歩になり、`exec.press`が`None`を返した歩（`delivered=false`）はウォークに入らないので、
さらに減る。`judge_self_verification`も標本数を見ていない。
失敗シナリオ: 予測した歩が約240で正答率95.8%のとき`Accepted`になる。ADR-196が定めた統計的な下限を
下回ったままの採用である。
対処（どちらか）:
- ウォークを「`correct + incorrect >= 300`になるまで続ける。ただし全体の上限は例えば1500歩」に変え、
  上限に達しても300未満なら`Rejected(HighDegeneration)`とみなす（縮退しているから予測が集まらないので、
  理由として整合する）。
- `judge_self_verification`に`min_predicted_steps`引数を足し、下回ったら`Rejected`にする
  （新しい`RejectedReason::InsufficientSamples`を足す場合は、schema v2が未リリースのうちに足すこと）。
  判定規則が純関数側にまとまるので、こちらを推奨する。

### C-3【Blocker（読み手とセットで出すこと）】段階4の読み手は`judgement`を一切読まない

`key_effect_runtime.rs::validate_and_convert`（355-374行）は、縮退率（カバレッジ80%）と、既知構成なら
5%不一致チェックを見るだけで、`table.judgement`/`table.verification`を読まない。
失敗シナリオ: 書き手が`Rejected(LowAccuracy)`（正答率90%）と書いても、カバレッジが80%以上なら
awase.exeは表を採用する。同時にT4の状態表示が「学習結果を採用しませんでした（自己検証90%）」と出すと、
UIと実挙動が矛盾する。`NeedsConfirmation(UnverifiedMsImeNative)`も同じく素通りし、決定1a
「Microsoft IME本体の表は既定では採用しない」が実行時に効かない。
現状（`judgement`が常に`None`）と比べて退行ではないが、1eの判定を書いた瞬間に「書いたのに効かない」
状態になる。対処:
- 読み手に`RejectReason::NotAccepted { judgement }`を足し、`judgement != Some(Accepted)`なら不採用に
  する（ユーザーの明示採用はC-6）。
- `judgement: None`（1e以前の書き手が出したv2ファイル）は**不採用**に倒す（`NotJudged`）。
  keymap-learnがまだリリースに含まれていなければ、既存ユーザーへの影響は無い。確認する。
- 読み手の`MAX_MISMATCH_RATIO = 0.05`による棄却は、ADR-196が撤廃を決めたものだが、**再測定
  オーケストレーション（1b項目7〜8）が入るまでは残す**。再測定が無いうちに外すと、既知構成での
  内蔵表との照合がどこにも無くなる（B-1回答の欠落。round1が指摘したのと同じ穴）。再測定を実装する
  PRで、書き手の`SystematicMismatch`判定に置き換えてから外す。

### C-4【Must-fix】`with_verification`/`with_judgement`の呼び出し順と、`persist_learned_table`のシグネチャ

`PersistedTable`のビルダーは`const fn`で互いに独立しており、呼び出し順そのものに意味は無い。
重要なのは`run_main`の中での**判定と永続化の順序**である。推奨する流れ:

```
RealImeDriver::new           … quiet window。TipIdentity(開始)取得、失敗なら学習前にErr
config1.db(開始)の読み取り    … B-3の比較基準
run + retry_nondeterministic_cells_once
統計の確定（既存どおり、ウォーク前）
session_failed / hook liveness の確認 → 偽なら書かずに終了（C-1）
run_verification_walk        … 専用Rng・予測300歩以上（C-2, C-7）
session_failed / hook liveness の再確認（ウォーク中の外部書き込み）
TipIdentity(終了)・config1.db(終了)の再取得、開始時と比較 → 違えば書かずに終了（A-6, B-3）
judgement = judge_self_verification(&score, tip == MsImeNative, ACC, DEGEN)
（将来）Known(preset)なら diff_against_bundled → 再測定 → SystematicMismatch で Accepted を上書き
         ※ Rejected は上書きしない。MsImeNative は現状 NotKnown なので UnverifiedMsImeNative と競合しない
PersistedTable::new(cells).with_verification(ScoredVerification{score, seed}).with_judgement(judgement)
write_atomic
result 行に judgement=... を追加
```

- `persist_learned_table(table: &Table)`は`(table, verification: ScoredVerification, judgement: TableJudgement)`を
  受け取る形に変え、判定の計算そのものは`run_main`（または純関数`adjudicate(..)`）に置く。
  `persist_learned_table`は書くだけにする。判定をpure関数に閉じておけば、main.rs側
  （`#[cfg(windows)]`でLinuxのテストが存在しない）にロジックが漏れない。
- 閾値（`0.95`/`0.20`/将来の`0.30`）は`judgement.rs`に`pub const`で置く。T4のUI文言と、読み手の
  `MIN_COVERAGE_RATIO`との関係をコメントで結ぶ。現状、読み手の`MIN_COVERAGE_RATIO = 0.80`（セル単位の
  カバレッジ）と`judge_self_verification`の縮退率20%（ウォーク歩単位）は**分母が違う別の量**で、同じ
  「20%」に見えて意味が異なる。どちらがADR-195 (ii)-1の「縮退率」なのかをコメントで明示する。
- stdoutの`result`行に`judgement=accepted|needs_confirmation:msime_native|rejected:low_accuracy|...`を
  足す。`parse_learn_line`は未知のフィールドを無視するので、awase-settingsを壊さない。ただし決定1eの
  とおり、表示の正は表ファイルなので、T4は表ファイルを読むこと。`status=success`は「表を書けた」の
  意味のまま残し、`Rejected`でも`success`になる点をT4側に申し送る（「学習完了」と表示して利用者を
  誤解させない）。

### C-5【Must-fix】エラー処理の方針（「安全側」の向きを決める）

ADR-196の「安全側」は、「誤って採用しない」方向（内蔵表または予測なしへ倒す）と、「失敗は失敗として
書かない」方向（1b項目5）で一貫している。各失敗をこれに当てはめると次のとおり。

| 失敗 | 振る舞い | 理由 |
|---|---|---|
| 開始時のTIP同定が`None` | **学習を始めずにエラー終了**（`result status=failure reason=ime_unidentified`） | 開始直後なので失敗させるコストが低い。20分の学習後に判明するより良い |
| 終了時のTIP同定が`None`、または開始時と異なる | 表を書かずにセッション失敗 | 何を学習した表か確定できない |
| `TipIdentity::Other`（ATOK本体、Japanist等） | `is_ms_ime_native=false`で判定し、突き合わせはしない | 決定1aの要確認対象はMicrosoft IME本体だけ。Otherは内蔵表の無い「カスタム」相当 |
| Gjiだがconfig1.dbが読めない・パースできない | 判定は続行し、突き合わせは`ConfigUnreadable`として行わない。表にその旨を記録する | 1c「誤って除外しても学習自体は妨げない」。ただし「既知でない」とは区別して不具合報告に残す |
| config1.dbが開始時と終了時で異なる | 表を書かずにセッション失敗 | B-3 |
| `table_file_path()`が`None`（config.toml不在） | 既存どおり`result status=failure` | 変更なし |

`is_ms_ime_native`を「不明なら`true`」（`NeedsConfirmation`に倒す）とする案は採らない。GJIユーザーに
「Microsoft IME本体は検証待ち」という誤った文言が出る（決定2のround4 S-gが文言を分けた意味が失われる）。
「不明なら書かない」の方が一貫する。

### C-6【Should-fix・スキーマ】1e前半で`schema v2`を埋め始める前に、将来のフィールドを先に決めておく

`CURRENT_SCHEMA_VERSION = 2`はT2で上げたばかりで、判定フィールドはまだ一度も書かれていない。ここで
実ファイルが出回る前に、次の2点を決めておくとv3への版上げを1回減らせる。
1. **学習対象のIME（`TipIdentity`相当）を表ファイルに記録する。** 現状の`PersistedTable`は「どのIMEで
   学習した表か」を持たず、`fingerprint`も`None`のままである。GJIで学習した表を、ユーザーがMicrosoft
   IME本体に切り替えた後も読み手がそのまま使う（`RuntimeTableCache`の`validation_key`はプリセットを
   比較するが、ファイル側にプリセットが無いので突き合わせ相手が無い）。C-3の読み手で「記録されたIMEと
   現在のIMEが異なれば不採用」にでき、T5のフィンガープリントの土台にもなる。
2. **ユーザーの明示採用（1b-8の判定書き換えモード）の表現。** 書き換えモードが`judgement`を`Accepted`に
   上書きすると、元の理由（`UnverifiedMsImeNative`/`SystematicMismatch`）が消え、T4が「ユーザーが採用した
   要確認表」と「最初から合格した表」を区別できない。`user_adopted: bool`か
   `TableJudgement::AdoptedByUser(NeedsConfirmationReason)`を今のうちに足す。

### C-7【Should-fix】検証ウォークの乱数シードの記録が、ADRの意味（再現可能なウォークのシード）と合わない

`run_main`は`Rng::new(195)`（固定値）を学習と検証ウォークで**共有**している。`ScoredVerification.seed`に
195を書いても、ウォークの系列は学習でどれだけ乱数を消費したかに依存するので再現できない。しかも固定
シードなので、毎回ほぼ同じウォークになり、「独立ウォーク」としての性質も弱い。
対処: 検証ウォーク専用に`Rng::new(walk_seed)`を作る。`walk_seed`は時刻などから取り、その値を
`ScoredVerification.seed`に記録する。

### C-8【Nice-to-have】ReconciliationSummaryと判定の合成規則をpure関数に閉じる

将来`SystematicMismatch`を合成するとき、「`Rejected`は上書きしない、`Accepted`だけを`NeedsConfirmation`へ
下げる、`UnverifiedMsImeNative`と`SystematicMismatch`が同時に立ったらどちらを残すか」を`main.rs`に
書くと、Linuxのテストが存在しない（`#[cfg(windows)] mod app`）場所に判定ロジックが入る。今のうちに
`judgement.rs`へ`fn combine(self_verification: TableJudgement, reconciliation: Option<&ReconciliationSummary>, threshold) -> TableJudgement`
の形で置き、1e前半では`None`を渡しておくと、後続PRが`main.rs`を触らずに済む。

### C-9【Nice-to-have】不採用の結果で、既存の採用済み表を上書きしてよいか

決定1eは「不採用でも表ファイルに書き出す」としている。単一ファイル`keymap-learn-table.json`に
アトミックに上書きすると、**以前`Accepted`だった良い表が、再学習の失敗（例えば正答率90%）で失われる**。
awase.exeは内蔵表（カスタムキーマップなら予測なし）へ退行する。決定3aの「軽量再検証が閾値を割って
初めて失効」の精神からすると、利用者が「もう一度学習」を押しただけで良い表を失うのは不自然である。
ADRに明記が無いので、実装前にユーザーへ確認すること。案: 不採用の結果は
`keymap-learn-last-attempt.json`に書き、採用・要確認の結果だけ本体を置き換える。

---

## 推奨する実装順序（これから実装するセッション向け）

1. **B-4**: `known_keymap.rs`で`session_keymap`の`None`/`NONE`をMSIMEプリセットとして扱う。テストの期待値を
   反転する（ホストで`cargo test -p awase-gji-config`）。
2. **A-5**: `tsf/tip_detector.rs`に、`TSF_OBS`に触れず、COM初期化を呼び出し側に任せる
   `pub fn query_tip_identity_on_current_sta() -> Option<TipIdentity>`を新設する。
3. **B-1/B-2**: `gji_charset_autodetect.rs`に`pub fn bundled_preset_for_adjudication(tip) -> {Known(KeymapPreset)|NotKnown|ConfigUnreadable}`
   を新設する（1e前半では戻り値を記録するだけで、突き合わせ・再測定はまだ行わない）。
4. **C-1**: `SessionMonitor::is_failed`、`RealImeDriver::session_failed`、フック生存確認を`run_main`に
   配線し、失敗なら表を書かない。
5. **A-6/B-3**: 開始時にTIPを同定して`None`なら学習前に失敗させる。終了時に再同定し、config1.dbも比較する。
6. **C-2/C-7**: 検証ウォークを専用の`Rng`にし、予測300歩以上を確保する（または純関数側で標本数を判定する）。
7. **C-4/C-8**: `judgement.rs`に閾値定数と合成関数を置き、`persist_learned_table`に`with_verification`/
   `with_judgement`を渡し、`result`行に`judgement=`を足す。
8. **C-3**: 読み手の`validate_and_convert`で`judgement != Some(Accepted)`を不採用にする（`None`も不採用）。
   5%チェックは残す。**7と8は同じPRで出すこと**（書いたのに効かない期間を作らない）。
9. **C-6/C-9**: スキーマ追加（学習対象のIME、ユーザー採用フラグ）と不採用時の上書き方針は、ユーザーに
   確認してから7の前に確定する。

### 最終判定

- 提案A: **変更が必要。** 検出は学習プロセス（学習窓のSTAスレッド）で行い、CLI引数では渡さない。
  既存の`RealImeDriver`のCOM/TSF初期化に相乗りするだけで、追加の土台は要らない。
- 提案B: **方針（学習プロセスが自分で読む）は維持し、細部を変える。** 直接依存を追加する代わりに
  `awase-windows`に1関数を置く。TIP同定でゲートし、開始時と終了時で比較する。**既存の
  `classify_known_gji_keymap`の`None`扱いのバグ（B-4）を先に直す。**
- 論点1と論点2の方式が分かれる不整合は、Aを学習プロセス側に寄せれば解消する。「セッションの環境事実は、
  測った本人（学習プロセス）が開始時と終了時に取る」という1つの規則に揃う。
- 1e前半の最小スコープに**C-1（セッション失敗の配線）とC-3（読み手が判定を読む）を含めない限り、
  実装してはいけない。** judgementを埋めるだけの実装は、ADR-196の安全装置が効いているように見えて
  実際には効かない状態を作る。
