# ADR-125: egui/winit アプリのウィジェット単位 IME 許可切替と、awase のフォーカスモデルの構造的ギャップ

## ステータス

**調査中・設計未確定。実機スパイクで当初仮説は再現せず、原因はまだ特定できていない。**
ユーザー報告（2026-09-03）を契機に、`winit`/`egui-winit` の実ソース
（`~/.cargo/registry` にキャッシュされた `winit-0.30.13`/`egui-winit-0.31.1`）を
読解し、「ウィジェット単位のフォーカス移動で同一 HWND の HIMC が脱着される」という
仮説を立てた。同日、この仮説を実機スパイク（`spike_egui_himc_reassociation_probe`）
で直接検証したところ、**想定した頻繁な脱着は一度も観測されなかった**（「実機検証ログ
（2026-09-03）」節）。**このため本 ADR は実装を決定しない。** 当初仮説は棄却または
保留とし、観測された別の事実（HIMC が終始 0 のまま）を手がかりに、次のアクション
（「次のアクション」節、実測結果を踏まえて更新）を実行してから改めて原因を特定する。

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

### D: 何もせず実機データの収集を優先する

**現時点ではこれを採用する（「決定」節）。** 根本原因を確定しないまま実装
すると、ADR-121 が辿った経緯（「物理キーが OS に届いていない」という当初の
診断が、実は「原因の半分でしかなかった」と後から判明した）と同じ失敗を
繰り返すリスクが高い。今回は ADR-121 のとき以上に検証が薄く、**Windows
実機での再現・ログ取得すら行っていない状態**での机上の推測に基づいている。
`.claude/rules/fix-requires-evidence.md` の精神（fix には回帰テストか記録の
いずれかを伴わせる——その前提として、まず実際に何が起きているかを観測する）
に照らしても、実装より先に実機データを取るべき局面である。

## 決定

**本 ADR の時点では実装方針を確定しない。当初仮説（選択肢 B/C が前提としていた
「ウィジェット単位の頻繁な HIMC 脱着」）は実機スパイクで再現しなかったため、
これを前提にした実装（B/C）には進まない。** 「実機検証ログ（2026-09-03）」節の
解釈(a)/(b)のどちらが正しいかを次のアクションで切り分けてから、改めて原因を
特定し実装 ADR を起票し直す。

### 次のアクション

1. **最優先: 実運用コードパスでの確認。** 実機（Windows）で `RUST_LOG=debug`
   を有効にした**本物の `awase.exe`** を起動した状態で `awase-settings.exe`
   （設定画面・不具合報告画面）にフォーカスを移し、
   `focus/class_names.rs::AppImeProfile::from_class_name` がこのウィンドウを
   実際にどう分類しているか（`Standard` になっているはずだが実測で確認する）、
   および `Standard` 経路の IMM32 クロスプロセス制御
   （`ImmGetOpenStatus`/`ImmSetOpenStatus` 呼び出し）がこのウィンドウに
   対して成功しているか・無言で失敗/no-op しているかをログで確認する。
   これは「実機検証ログ」節で残った解釈(a)/(b)の分岐点そのものであり、
   本スパイクの独自実装より信頼できる一次情報になる。
2. 上記と同時に、不具合報告画面の説明欄に症状（重さ・かな混入）を再現させ、
   `journal`/`awase.log` を取得する（保存先は `crates/awase-windows/src/
   journal.rs` 起点の既定パスを確認する）。
3. 取得したログを `docs/journal-replay-guide.md` のリプレイ基盤で解析し、
   「未確定な点」2〜3（重さの実体、かな混入の実体）を裏取りする——1（`allow_ime`
   遷移の有無・頻度）は「実機検証ログ」節により**否定的な結果が既に出ている**
   （少なくとも今回の手順では観測されなかった）ため優先度を下げてよい。
4. 手順1で `Standard` 分類・IMM32 クロスプロセス制御が実際に機能していない
   ことが確認できた場合、原因は「解釈(a)」（`AppImeProfile::from_class_name`
   の既定フォールバックが egui/winit ウィンドウの実態に合っていない）に
   確定し、BUG-33/BUG-37 と同系統の対策（belief が実状態を観測できない
   アプリとして扱う、`Imm32Unavailable` 相当の分類を検討する等）を設計する
   別 ADR を起票する。
5. 手順1で IMM32 クロスプロセス制御が実際には機能している（=「解釈(b)」、
   本スパイクの観測方法の欠陥）と判明した場合、当初仮説（ウィジェット単位の
   HIMC 脱着）の再検証、または全く別の原因（`bug_report.rs` 自体の別経路等）
   の調査に切り替える。

## 範囲外

- `disable_apps` へのプロセス名追加（却下済み、「検討した方向性」節 A）。
- 実装そのもの（本 ADR は調査・方針確定前の設計 ADR であり、コード変更を
  一切含まない）。
- BUG-33/BUG-37/BUG-69 系の「IMM32/TSF が信頼できないアプリでの belief
  乖離」全般の再設計（関連はするが、本 ADR は「トップレベル HWND 単位の
  フォーカスモデルに収まらない IME コンテキスト変化」という新しい軸に限定
  する）。

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
