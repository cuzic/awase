# ADR-125: egui/winit アプリのウィジェット単位 IME 許可切替と、awase のフォーカスモデルの構造的ギャップ

## ステータス

**根本原因を実機で確定・修正方針採用（未実装）。** タイトルにある
「ウィジェット単位の IME 許可切替とフォーカスモデルのギャップ」という
当初仮説、およびその後の「`Standard` 分類・IMM32 クロスプロセス制御が
機能していない」という第2仮説は、いずれも実機スパイクで**否定された**
（「実機検証ログ」「実機検証ログ2」節）。3回目の実機検証（実際に
`awase.exe` 本体エンジンを起動した状態での再現、「実機検証ログ3」節）で、
**真因は `ImmCapabilityStore`（IMM32 制御能力の学習キャッシュ）が
`class_name` のみをキーにしていること**と判明した——winit の既定クラス名
`"Window Class"` は複数の無関係なプロセスが共有するため、どこか別の
winit アプリ由来の「IMM32 が使えない」という学習結果が
`awase-settings.exe` にまで誤って適用され、実行時に `Standard` から
`Imm32Unavailable` へ降格していた。これは `docs/known-bugs.md` BUG-56
（Qt の汎用クラス名 `Qt663QWindowIcon` が LINE 内の無関係なウィンドウを
巻き込んだ事故）と同一機構の**プロセス間版**である。修正方針は「決定」
節の E 案（学習キャッシュを `(process_name, class_name)` でキーする）
を採用したが、コード変更はまだ行っていない。

最初に検討した対策（`disable_apps` に `awase-settings.exe` を追加し、awase 自身の
GUI では丸ごと無効化する）はユーザーに却下された。理由は「その考え方だと
eframe/egui を使うアプリすべてが awase 非対応という話になってしまう。eframe/egui
でもちゃんと動くように考えるべき」という指摘であり、正当である——`disable_apps`
はプロセス名の後追いリストであり、ユーザーが今後使う可能性のある他の
eframe/egui 製アプリ（自作ツール等）で同じ症状が起きた場合に対処できない。
本 ADR はこの指摘を受け、対症療法ではなく機構そのものを特定する調査に切り替えた。

## 背景

### 症状（ユーザー報告、2026-09-03）

タスクトレイの「不具合を報告」画面（`crates/awase-settings/src/bug_report.rs`、
`--bug-report` 起動）の説明欄で文字入力がとても重く（1打鍵ごとに体感できる遅延が
ある）、さらに入力していないはずのかな文字が突然挿入されることがある。同じ
`awase-settings.exe` の通常の設定画面でも起こりうる（両者は同一バイナリ・同一
GUI スタック）。

### 却下した対策: `disable_apps` へのプロセス名追加

`config.app_overrides.disable_apps`（BUG-78 で `mstsc.exe` 用に作った、フック
レベルで生キーをそのまま OS に通す丸ごとバイパス機構）の既定値に
`awase-settings.exe` を追加する案を最初に検討した。動作上は症状を確実に消せるが、
「awase 自身の GUI だけ特別扱いする」パッチであり、eframe/egui という**フレーム
ワーク全体**との相性問題には対処にならない。ユーザーの指摘どおり却下する。

## 調査で判明した技術的事実（ソースコード読解による仮説——実機検証で否定された部分は「実機検証ログ（2026-09-03）」節参照）

### `egui-winit` はウィジェット単位で IME の on/off を毎フレーム動的に切り替える

`egui-winit-0.31.1/src/lib.rs:866-871`（`handle_platform_output`）:

```rust
let allow_ime = ime.is_some();
if self.allow_ime != allow_ime {
    self.allow_ime = allow_ime;
    profiling::scope!("set_ime_allowed");
    window.set_ime_allowed(allow_ime);
}
```

`ime`（`Option<egui::output::IMEOutput>`）は、**その1フレームでフォーカスを
持つテキストウィジェット（`TextEdit` 等）が実際に IME 入力を欲しがっているか**
を表す。フォーカスがテキストウィジェットから外れる（他のウィジェットをクリック
する、ウィンドウ全体がフォーカスを失う等）と `None` になり、`allow_ime` が
`false` へ遷移した瞬間に `window.set_ime_allowed(false)` が呼ばれる。

### winit は「IME 許可」を IME コンテキストの脱着（`ImmAssociateContextEx`）で実装している

`winit-0.30.13/src/platform_impl/windows/ime.rs:141-151`
（`ImeContext::set_ime_allowed`）:

```rust
pub unsafe fn set_ime_allowed(hwnd: HWND, allowed: bool) {
    if !unsafe { ImeContext::system_has_ime() } {
        return;
    }
    if allowed {
        unsafe { ImmAssociateContextEx(hwnd, 0, IACE_DEFAULT) };
    } else {
        unsafe { ImmAssociateContextEx(hwnd, 0, IACE_CHILDREN) };
    }
}
```

`allowed=true` は既定の IME コンテキストを **同一 HWND に対して**再アタッチし
（`IACE_DEFAULT`）、`allowed=false` はコンテキストをデタッチする（`IACE_CHILDREN`、
HIMC を空にする）。加えて、`winit-0.30.13/src/platform_impl/windows/window.rs:1161`
のとおり **ウィンドウ生成直後は明示的に `set_ime_allowed(false)` で開始**する
（IME は「既定で無効」、テキストウィジェットにフォーカスが入って初めて egui が
`true` にする設計）。

つまり `eframe`/`egui-winit` の GUI は、**Win32 の伝統的な GUI（各コントロールが
個別の子 HWND を持ち、IME コンテキストはコントロール生成時から一貫して保持される）
には存在しないパターン**——**1つの HWND の中で、内部ウィジェットのフォーカスが
動くたびに、その HWND の HIMC（IME コンテキストハンドル）を都度デタッチ/
再アタッチする**——を実装している。

### awase のフォーカス追跡モデルは「トップレベル HWND の切り替え」しか見ていない

awase 本体（`crates/awase-windows/src/runtime/focus_tracking.rs`）は、
フォアグラウンドウィンドウの変化（トップレベル HWND 単位）を契機に IME belief
の再同期・composition warmup を行う設計である。上記の `ImmAssociateContextEx`
呼び出しは**同一 HWND 内**で起き、Win32 のフォーカス変更通知
（`WM_SETFOCUS`/`WM_KILLFOCUS`、あるいは awase が使っているフォアグラウンド
ウィンドウ変更イベント）を一切伴わない。したがって、この種の HIMC 脱着が
起きても、**awase 側にはそれを検知する経路が現状存在しない**。

`ImmAssociateContextEx` による脱着は HIMC を実質的に作り直す操作であり、IME の
ON/OFF・変換モードが OS 既定へ暗黙にリセットされうる。ユーザーの打鍵中に
awase の belief（`desired_open`/`effective_open`）と実際の HIMC の状態が、
awase が想定していない経路で乖離する可能性がある——これは BUG-33/BUG-37
（IMM32/TSF が信頼できないアプリでの belief 乖離）と結果は似るが、原因の軸が
異なる。BUG-33/37 は「そのアプリの IME 制御方式自体が観測不能」という話であり、
本件は「Standard プロファイル（`AppImeProfile::from_class_name` の既定値、
IMM32 クロスプロセス制御が使えるはずのアプリ）であっても、同一ウィンドウ内の
フォーカス移動そのものが HIMC を作り直しうる」という、awase がこれまで一切
想定していなかった軸である。

### 影響範囲: awase-settings.exe に限らない

この機構は `egui-winit`（ひいては同じパターンを採る他の immediate-mode GUI
フレームワーク）に組み込まれた**汎用的な**実装であり、`awase-settings.exe` に
固有ではない。ユーザーが他に使う eframe/egui 製アプリでテキスト入力欄を持つ
ものがあれば、同じ機構により同じ症状（潜在的には）が起こりうる。これが、
プロセス名ベースの `disable_apps` 追加が却下されるべき理由でもある。

## 未確定な点（実機ログでの裏取りが必要）

1. **`allow_ime` の実際の遷移頻度。** 単純に説明欄へ連続入力しているだけなら
   `TextEdit` はフォーカスを保持し続けるはずで、理論上は `allow_ime` は
   `true` のまま変動しないように見える。それにもかかわらず「1打鍵ごとに
   体感できる重さ」と報告された点をどう説明するかが未確認——`ComboBox`
   （症状カテゴリ選択）操作やプレビュー欄（`egui::ScrollArea` + 別の
   `TextEdit`）との間でフォーカスが実際にどう動いているか、`bug_report.rs`
   側の `PREVIEW_DEBOUNCE` 再描画がフォーカス状態に影響していないか、
   実機ログ（またはデバッグビルドでの `allow_ime` 変化のロギング）で
   確認する必要がある。
2. **「重い」の実体。** IME 側の待ち時間（HIMC 再アタッチ後の awase 側の
   再同期コスト）なのか、それとも `bug_report.rs` 自体の別経路（プレビュー
   JSON 再生成、既に `PREVIEW_DEBOUNCE` で対策済みのはずの重さの再発）
   なのかを切り分ける必要がある。
3. **「かな混入」の実体。** 本当に HIMC 再アタッチに起因する awase 側の belief
   乖離・同時打鍵誤判定なのか、それとも `egui`/`winit` 側の IME
   コンポジション確定処理自体に既知の不具合（バージョン依存）があるのかは
   未確認。

## 実機検証ログ（2026-09-03）

上記1を実機スパイクで直接検証した。専用の使い捨てプローブ
（`crates/awase-windows/examples/spike_egui_himc_reassociation_probe.rs`、
`spike/adr125-egui-himc-probe` ブランチとして起票、実行方法・観測方法は
ファイル冒頭のコメント参照）を作成し、`GetForegroundWindow()` +
`ImmGetContext(そのhwnd)` を100ms間隔でポーリングして、値が変化した
瞬間だけログする形で clipwire 経由の実機（Windows, dragonflyg4）で実行した。

### 手順

1. プローブを起動したまま不具合報告画面（`awase-settings.exe --bug-report`）
   にフォーカスを移した。
2. 説明欄のテキストボックスをクリックして連続入力 → 症状カテゴリの
   `ComboBox` を開いて選択 → 再び説明欄に戻って入力を続ける、という手順を
   約1分間（09:36:02 〜 09:36:36 の約34秒間、フォーカスが
   `awase-settings.exe` に留まっていた区間で確認）繰り返した。

### 観測結果

```
09:36:02.977 [FOCUS] hwnd 0x10176 -> 0x8B61BAA class=Window Class process=awase-settings.exe himc=0x0
09:36:36.773 [FOCUS] hwnd 0x8B61BAA -> 0x100DC8 class=CASCADIA_HOSTING_WINDOW_CLASS process=WindowsTerminal.exe himc=0x0
```

この2行の間（`awase-settings.exe` にフォーカスが留まっていた約34秒間、
手順2のテキスト⇔ComboBox間のフォーカス往復を含む）、**`[HIMC]` 行は
1行も出力されなかった**——`hwnd` 不変のまま `himc` が変化したことを示す
ログが一切無い。

**当初仮説（ウィジェット単位のフォーカス移動のたびに HIMC が脱着される）は、
この実機観測では確認できなかった。** さらに、フォーカス着地の瞬間から
一貫して `himc=0x0`（NULL）だった点も注目に値する——比較対象として同じログに
記録された `msedge.exe`（`Chrome_WidgetWin_1`、`IMM32_UNAVAILABLE_CLASSES`
所属、IMM32 が使えないことが既知）や `WindowsTerminal.exe`
（`CASCADIA_HOSTING_WINDOW_CLASS`、TSF ネイティブとして既知）も同様に
`himc=0x0` であり、これらは awase の既存分類上「IMM32 が使えない」ことが
確定しているアプリである。**`awase-settings.exe` の HIMC もこれらと同じ
「終始 0」というパターンを示した**——`focus/class_names.rs::AppImeProfile::
from_class_name` は `awase-settings.exe` のウィンドウクラス名
（実測: 単に `"Window Class"` という winit の既定クラス名。固有の識別子には
なりえない汎用文字列であることも同時に確認できた）をどの特殊リストにも
マッチさせられず、既定の `Standard`（IMM32 クロスプロセス制御が使えるはず、
という分類）に落ちる。しかし実測の HIMC の挙動は `Standard` の想定
（打鍵に応じて有効な HIMC が読めるはず）ではなく、`Imm32Unavailable`/
`TsfNative` のパターン（終始読めない）に近い。

### この結果の解釈（未確定、要追加検証）

2つの可能性が残り、今回のスパイクだけでは切り分けられない:

- **(a) 本当に `awase-settings.exe` の HIMC は awase から見て終始無効**
  （`Standard` という分類自体が実態と合っていない）。この場合、原因は
  「ウィジェット単位の頻繁な脱着」ではなく「そもそも awase の IMM32
  クロスプロセス制御（`ImmGetOpenStatus`/`ImmSetOpenStatus`）がこの
  ウィンドウに対して機能していない」可能性が高く、BUG-33/BUG-37 と
  同じ「belief が実状態を観測できないまま実行され続ける」失敗パターンに
  近い、より根の深い問題になる。
- **(b) このスパイクの観測方法自体に見落としがある。** 例えば
  `ImmGetContext` のクロスプロセス読み取りが、他プロセスのスレッドに
  `AttachThreadInput` していない状態では正しい値を返さない、といった
  Win32 の既知の落とし穴が影響している可能性がある。awase 本体は既に
  `Standard` プロファイルのアプリに対して `ImmGetOpenStatus` 等の
  クロスプロセス制御を実運用で使っている（`class_names.rs` の
  `can_use_imm32_cross_process`）ため、**この実運用コードパス自体が
  `awase-settings.exe` に対して機能しているかどうかを確認するのが、
  次に切り分けるべき最も直接的な方法**である（本スパイクの独自実装より
  信頼できる——同じ疑いが本スパイク側にもあてはまらない）。

## 実機検証ログ2: 実運用コードパス（IMM32クロスプロセス制御）の直接確認（2026-09-03）

上記「この結果の解釈」の(a)/(b)を切り分けるため、awase 本体が実際に使っている
コードパスと同一の Win32 呼び出し列（`imm.rs::get_ime_wnd`/`send_ime_control`
と同じロジック、`ImmGetDefaultIMEWnd` + `WM_IME_CONTROL`/`IMC_GETOPENSTATUS`
を `SendMessageTimeoutW` で送る）を再実装した専用スパイク
（`crates/awase-windows/examples/spike_egui_ime_control_probe.rs`）を作成し、
同じく clipwire 経由の実機で実行した。**この検証は `awase.exe`（本体エンジン）
を起動せずに行った**——本スパイクはそれ自身が独立して `ImmGetDefaultIMEWnd`/
`WM_IME_CONTROL` を呼ぶため、エンジンの起動有無に依存しない（IMM32
クロスプロセス制御そのものがこのウィンドウに対して機能するかを見る検証であり、
NICOLA エンジンの打鍵処理そのものは対象にしていない）。

### 手順

不具合報告画面の説明欄で、OS標準のIME（`awase.exe`は未起動）を使って日本語
入力を行い、途中でIMEを手動でOFF→ONと切り替えた。

### 観測結果

```
10:22:00.594〜10:22:05.177  ime_wnd=0xAA17CC（有効なIMEウィンドウ、終始一定）
                            open=Some(true)、elapsed_msは概ね0、最大13ms
10:22:05.295                open=Some(false)  ← IMEをOFFにした操作と一致
10:22:06.826                open=Some(true)   ← IMEをONに戻した操作と一致
（以降 open=Some(true) が継続）
```

`ime_wnd` は常に非NULLの有効なハンドルであり、`WM_IME_CONTROL`
（`IMC_GETOPENSTATUS`）は一度も失敗・タイムアウトせず（`elapsed_ms` は
すべて50msのタイムアウト内、大半は0〜13ms）、返る値も実際にユーザーが
行ったIME ON/OFF操作と正確に一致した。

**解釈(b)が確定した。** awase 本体が実際に使っているクロスプロセスIME制御
機構（`ImmGetDefaultIMEWnd` + `WM_IME_CONTROL`）は `awase-settings.exe` の
ウィンドウに対して正常に機能している。「実機検証ログ（2026-09-03）」節の
`ImmGetContext` 直読みによる「HIMC終始0」という結果は、awase が実際には
使っていない無関係な観測方法によるものであり、`AppImeProfile::Standard`
という既定分類は誤っていなかった。

### この時点での結論

**当初の2つの仮説（HIMC脱着によるbelief乖離／`Standard`分類の誤り）は、
いずれも実機検証で否定された。** IMM32クロスプロセス制御の入出力機構自体は
正常に機能しているため、「重い」「かな混入」の原因はこの機構の外側——
おそらく `awase.exe` 本体エンジンが実際に打鍵を処理している最中の
タイミング・同時打鍵判定・focus/warmup管理のいずれか——にある可能性が
高くなった。ここまでの2つの検証はいずれも `awase.exe` を起動せずに
行っており、症状そのもの（NICOLA/thumb-shift打鍵によるかな変換）を
一度も再現できていない点に注意——次は実際にエンジンを動かした状態での
再現とログ取得が必要（「次のアクション」節を参照・更新）。

## 実機検証ログ3: 実エンジン稼働下での再現、根本原因の確定（2026-09-03）

「実機検証ログ2」までの2回の検証はいずれも `awase.exe` 本体エンジンを起動
せずに行っていた。今回は実際に `awase.exe`（`RUST_LOG=debug`）を起動した
状態で不具合報告画面にフォーカスし、親指シフトで日本語入力を行った。

### 再現結果

1回目は目立った重さは無かったが、**説明欄の先頭に意図しない「あ」が
混入した**（ユーザー報告）。2回目の再試行でも同じく**先頭に「あ」が
混入し、再現性が高い**ことを確認した（「重さ」は今回はいずれも顕著では
なかった——「かな混入」と「重さ」は必ずしも同時に起きる同一原因ではない
可能性がある）。

### ログ解析で判明した根本原因

`target/debug/awase.log`（`RUST_LOG=debug` 起動、747MBまで肥大化していた
既存ログは調査の妨げになったため一度クリアしてから再現し直した）に、
以下の行が記録されていた:

```
[imm-learning] profile 降格: class="Window Class" Standard → Imm32Unavailable
（実測学習 ImmCapability::Unavailable。誤学習なら cache.toml の
[imm_capability] から該当クラスを削除）
```

`crates/awase-windows/src/focus/tracker.rs::apply_learned_imm_capability`
（`focus/imm_learning.rs` が書き込む `ImmCapabilityStore` の学習結果）が、
実機の `cache.toml` に既に保存されていた `"Window Class" = "unavailable"`
というエントリに基づき、`awase-settings.exe` の静的分類 `Standard`
（`AppImeProfile::from_class_name` の既定フォールバック）を実行時に
`Imm32Unavailable` へ降格させていた。`target/debug/cache.toml` を直接
確認したところ、実際に `"Window Class" = "unavailable"` が記録されていた。

**これは `docs/known-bugs.md` BUG-56 と全く同じ機構の再発である。**
BUG-56（2026-08-07）は Qt が使う汎用クラス名 `Qt663QWindowIcon` が、LINE
アプリ内の無関係な一時ウィンドウ（通知アイコン等）と本物のチャット入力欄
とで使い回されていたため、前者がたまたま `ImmGetDefaultIMEWnd`=NULL を
返しただけで、`class_name` をキーとする学習キャッシュ経由で後者まで
`Imm32Unavailable` に巻き込まれ降格した、という事故だった。修正
（`ImmCapabilityStore::record_null_probe`、2回連続観測するまで確定しない
デバウンス）は**同一プロセス内で同じクラス名を使い回すケース**を主眼に
設計されており、実際に現在の `cache.toml` にもその修正後もなお
`Qt663QWindowIcon = "unavailable"` が残っている（LINE 側は本当に
Unavailable と確定したのか、再学習されていないだけかは今回未確認）。

**今回の事故はこれとは異なる軸——`ImmCapabilityStore` のキャッシュが
`class_name` のみをキーにしており、プロセスをまたいだ衝突を一切防げない**
点にある。`winit`（`awase-settings.exe` が使う GUI フレームワーク）は
「調査で判明した技術的事実」節で確認したとおり、`"Window Class"` という
**ハードコードされた既定クラス名**を、`with_class_name` で明示的に上書き
しない限りあらゆる winit アプリで共有する。したがって、**この Windows
実機上で過去に動かした別の何らかの winit ベースのアプリ（awase-settings.exe
自身の別セッションも含みうる）が `Imm32Unavailable` と学習されていれば、
その学習結果が `awase-settings.exe` にもそのまま適用されてしまう**。
実際、`cache.toml` を全体確認したところ、`Button`/`Edit`/`ComboBox`/
`Static`/`SysListView32` のような**極めて一般的な Win32 標準コントロールの
クラス名**まで多数 `unavailable` として学習されており、同種の汚染リスクが
`class_name` 単独キーの学習キャッシュ全体に構造的に存在することが分かった。

「実機検証ログ2」で確認したとおり、`awase-settings.exe` に対する
`ImmGetDefaultIMEWnd`/`WM_IME_CONTROL`（実運用コードパスと同一の呼び出し）
は実際には正常に機能する（有効な IME ウィンドウ・50ms 以内の応答・正確な
ON/OFF 追従）。つまり `Imm32Unavailable` への降格は**誤学習**であり、
`Imm32Unavailable` プロファイル用の VK_KANJI トグル・物理 IME キー抑止と
いった、本来 IMM32 が使えないアプリ向けの制御方式が、実際には IMM32 が
正常に機能するウィンドウに対して発動してしまうことで、belief と実状態の
ズレ・物理キーの意味論の不一致が生じ、「先頭の あ」のような症状に
つながっていると考えられる（VK_KANJI トグル自体はまだ実測で追跡できて
いないため、正確な発火メカニズムは今後の裏取り対象）。

## 検討した方向性（Alternatives）

### A: `disable_apps` へのプロセス名追加（却下）

「背景」節参照。プロセス名の後追いであり、eframe/egui というフレームワーク
全体との相性問題を解決しない。

### B: ウィンドウクラス名で egui/winit アプリを個別に検知する

`focus/class_names.rs` の `IMM32_UNAVAILABLE_CLASSES`/`is_tsf_native_window`
と同じパターンで、egui/winit が使うウィンドウクラス名を新しいリストに追加する
案。

- 課題1: winit の Win32 ウィンドウクラス名はバージョン依存かつ、アプリ側が
  `with_class_name` 等で上書きしない限り安定した固定文字列とは限らない
  （要実機確認）。
- 課題2: egui/winit 以外にも同種の「1 HWND 内でウィジェット単位に IME
  on/off を動的に切り替える」実装を持つ GUI フレームワーク（他の
  immediate-mode GUI 等）が今後登場しうる。クラス名の個別列挙では、その
  たびに追随が必要になり、A案（プロセス名の個別列挙）と本質的に同じ
  「後追いリスト」問題を抱える。

### C: HIMC の変化そのものを汎用的な「フォーカス相当イベント」として検知する

`ImmGetContext` で取得できる HIMC ハンドルを、既存のポーリング
（`ime_poll_interval_ms`）や打鍵直前の probe のたびに前回値と比較し、
**トップレベル HWND が変わっていなくても HIMC が変わっていれば**、フォーカス
変更相当の再同期（belief resync・composition re-warmup）をトリガーする案。

- 利点: ウィンドウクラス名やプロセス名のハードコードが要らない汎用的な検知
  であり、egui/winit に限らず「同一 HWND 内で IME コンテキストを動的に
  切り替える」あらゆる将来のアプリに対応できる可能性がある。
- 未検証の懸念: 毎回 `ImmGetContext`/`ImmReleaseContext` を追加で呼ぶ
  コスト、既存の warmup/probe タイミング・BUG-16 系の focus-settle 制御との
  整合、そもそも「HIMC 変化＝フォーカス変更相当」という等式が全ての
  IME/アプリの組み合わせで安全か、が未検証。

### D: 何もせず実機データの収集を優先する（一時的に採用、「実機検証ログ3」で確定に至った）

B/C いずれの仮説も机上の推測にとどまっていた段階では、根本原因を確定しない
まま実装すると ADR-121 が辿った経緯（「物理キーが OS に届いていない」という
当初の診断が、実は「原因の半分でしかなかった」と後から判明した）と同じ失敗を
繰り返すリスクが高いと判断し、実機データの収集を優先した。「実機検証ログ3」
（実際に `awase.exe` を起動した状態での再現とログ解析）により、この方針が
実際に根本原因の確定（下記 E）につながった。

### E: `ImmCapabilityStore` の学習キャッシュを `class_name` 単独ではなく `(process_name, class_name)` でキーする（採用、BUG-56 の教訓の拡張）

「実機検証ログ3」で確定した根本原因（`cache.toml` の `"Window Class" =
"unavailable"` という、`awase-settings.exe` とは無関係などこか別の
winit アプリ／プロセスに由来する学習結果が、`class_name` のみをキーとする
`ImmCapabilityStore`（`focus/classifier.rs`）経由で `awase-settings.exe`
にまで誤って適用されていた）に対する直接の修正案。

- `ImmCapabilityStore::cache: HashMap<String, ImmCapability>` の key を
  `class_name: String` から `(process_name: String, class_name: String)`
  のタプル（または結合文字列）に変更する。`learn`/`get`/
  `record_null_probe`/`clear_pending_unavailable`（すべて `focus/
  classifier.rs`）と、呼び出し元 `focus/imm_learning.rs::
  learn_imm_capability_on_focus`・`focus/tracker.rs::update`/
  `apply_learned_imm_capability` に process_name を通す配線が必要。
- **利点:** 本 ADR が発見した「winit の既定クラス名 `"Window Class"` の
  ような、複数の無関係なプロセスが共有する汎用クラス名」によるプロセス間の
  学習汚染を構造的に防げる。今回の実機 `cache.toml` を確認したところ、
  `Button`/`Edit`/`ComboBox`/`Static`/`SysListView32` のような Win32
  標準コントロールの汎用クラス名まで多数 `unavailable` として学習されており、
  同種のリスクはクラス名単独キー方式**全体**に構造的に存在する——本件は
  氷山の一角である可能性が高い。
- **BUG-56 との関係・役割分担:** BUG-56 が対処したのは**同一プロセス内**で
  同じクラス名を複数の異なるウィンドウ（本物の入力欄／無関係な一時ウィンドウ）
  が使い回すケース（Qt の `Qt663QWindowIcon`）であり、`process_name` を
  キーに加えても**この種の衝突は防げない**（同一プロセス内なので
  process_name も同じ）——BUG-56 の「2回連続観測するまで確定しない」
  デバウンスは引き続き必要で、本 ADR の変更後もそのまま維持する。本 ADR が
  追加で防ぐのは**異なるプロセス間**での衝突であり、両者は直交する独立した
  対策として共存する。
- **前例との整合:** `config.rs::AppOverrideEntry`（`force_tsf`/`force_vk`/
  `force_bypass` 等が使う既存の設定型）は既に `{ process, class }` の組で
  アプリを識別しており、`(process_name, class_name)` キーはこのリポジトリの
  既存パターンに沿う。
- **未解決の設計課題（実装時に確定させること）:**
  1. 既存 `cache.toml` の `[imm_capability]` は `class_name = "works"/
     "unavailable"` というフラットな形式——キー形式を変える際、旧形式の
     取り扱い（起動時に一度だけ捨てて学習し直させるか、`"process_name\x1f
     class_name"` のような結合文字列キーへ機械的に移行するか）を決める必要が
     ある。BUG-56 の教訓（誤学習は `cache.toml` を手で編集して除去できる）を
     踏まえ、複雑な自動移行より「捨てて学習し直す」方が安全な可能性が高い。
  2. `process_name` の取得コスト（`get_process_name` は Win32 プロセス
     ハンドルを開くため高コスト、`class_names.rs:191` のコメント参照）が、
     学習判定のたびに毎回発生してよいか——`learn_imm_capability_on_focus` は
     フォーカス変更のたびに（学習済みでなければ）呼ばれるため、頻度と
     コストのバランスを確認すること。
  3. 「実機検証ログ3」で確認した「先頭の あ」という具体的な症状の発生
     メカニズム（`Imm32Unavailable` 降格後にどの VK_KANJI トグル／物理キー
     抑止ロジックがどう発火して誤った文字を生むか）はまだ実測で追跡できて
     いない——本修正の効果検証は、`cache.toml` から該当エントリを削除した
     状態（BUG-56 と同じ暫定回避）で症状が消えることの実機確認と合わせて
     行う。

## 決定

**当初の2つの仮説（B/C 案が前提とした「ウィジェット単位の頻繁な HIMC
脱着」、および解釈(a)「`Standard` 分類・IMM32 クロスプロセス制御そのものが
機能していない」）はいずれも実機スパイクで否定されたが、3回目の実機検証
（実際に `awase.exe` を起動した状態での再現）で根本原因を確定できた。**
`ImmCapabilityStore`（`focus/classifier.rs`）の学習キャッシュが
`class_name` のみをキーにしており、winit の既定クラス名 `"Window Class"`
のような複数プロセスが共有する汎用クラス名を介して、無関係な別アプリ由来の
`Imm32Unavailable` 学習結果が `awase-settings.exe` に誤って適用されていた
（BUG-56 と同機構、プロセス間版）。**E 案（`(process_name, class_name)`
キー化）を採用し、実装に進む。**

### 次のアクション

1. E 案を実装する（「未解決の設計課題」1〜2 を実装時に確定させる）。
2. 実装前後の暫定回避として、`cache.toml` の `[imm_capability]` から
   `"Window Class"` エントリを削除し、`awase-settings.exe` で症状が
   実際に消えることを実機確認する（BUG-56 と同じ手順、E 案の効果検証の
   ベースラインにもなる）。
3. `docs/known-bugs.md` に新規 BUG として起票し、BUG-56 との関係
   （同機構・プロセス間版であること）を明記する。
4. 「未解決の設計課題」3（`Imm32Unavailable` 降格後の具体的な誤動作
   メカニズム）は、実装後の実機ソークで余力があれば追加調査する
   （本 ADR のスコープでは必須としない——原因の特定と一般的な修正で
   十分価値があり、詳細メカニズムの解明は効果検証で代替できる）。

## 範囲外

- `disable_apps` へのプロセス名追加（却下済み、「検討した方向性」節 A）。
- 選択肢 B（クラス名個別列挙）・C（HIMC 変化検知）——いずれも実機データで
  前提が崩れたため不採用。
- BUG-56 自身の再修正（同一プロセス内の衝突対策は現状のデバウンスのまま
  維持し、本 ADR では変更しない）。
- BUG-33/BUG-37/BUG-69 系の「IMM32/TSF が信頼できないアプリでの belief
  乖離」全般の再設計（関連はするが、本 ADR は学習キャッシュのキー設計という
  新しい軸に限定する）。
- `Imm32Unavailable` 降格後の具体的な誤動作メカニズムの完全解明（「次の
  アクション」4、必須ではない追加調査）。

## 関連

- `docs/known-bugs.md` BUG-107（本件の症状・調査経緯を記録、ステータスは
  「調査中・未修正」）。
- BUG-33/BUG-37（IMM32/TSF 非信頼アプリでの belief 乖離、原因の軸は異なる
  が症状のパターンが類似）。
- BUG-78（却下した A 案が再利用しようとした `disable_apps` 丸ごとバイパス
  機構の初出）。
- ADR-121（「原因の半分」を早期に確定として扱ってしまい、後の敵対的
  レビューで訂正された教訓——本 ADR が「実機未検証のまま実装を決定しない」
  という慎重な立場を取る直接の理由）。
