# ADR-105: エンジンスレッドへの通知はHWND宛のPostMessageWに統一する

## ステータス

**実装済み（2026-08-26、Windows実機ソーク未実施）。** [ADR-102](102-startup-key-delivery-one-way-closure.md)（起動シーケンスとキー配送）の根本的な再設計を検討する過程で、Opus敵対的レビュー・根本原因分析・実機実験を経て見つかった、ADR-102単体のスコープを超える設計原則。Codex CLIが実装し、Opus敵対的レビュー→指摘10件の修正を経て、`cargo fmt`/`cargo xwin check`・`clippy --all-targets -D warnings`/`cargo test -p awase-windows`（445件+architecture_guard 44件）が全てgreenであることをClaude自身も独立に確認済み。Windowsクロスコンパイルとユニットテストのみで、実機での起動・動作確認はまだ行っていない。

## コンテキスト

### 現状: `PostThreadMessageW` によるスレッドID宛の通知

このコードベースでワーカースレッド（フックスレッド・GJI I/Oモニタ・UIAワーカー・非同期タスク等）からエンジンスレッドへ何かを知らせる手段は、`win32.rs:46-90` の `post_to_main_thread`/`post_to_main_thread_with` に集約されている。実装は `PostThreadMessageW(engine_thread_id(), ..)` — つまり**宛先はスレッドID**である。

この2関数は現在**唯一の集約点**であり、呼び出し箇所は10ファイル・13箇所。したがって実装を差し替えれば全箇所に波及する（後述）。

### なぜ「スレッドID宛」になったか（BUG-09の経緯、再導入しないための記録）

`win32.rs:40-45` のコメントが記録するとおり、旧実装は `PostMessageW(None, ..)`（hwnd=NULL）を使っていた。Win32のドキュメント上、hwnd=NULLの `PostMessageW` は**「呼び出しスレッド自身への `PostThreadMessage` と等価」**であり、ワーカースレッド（gji-io-monitor・UIAワーカー等）から呼ぶと、投函先が**呼び出し元スレッド自身の未使用キュー**になり、メッセージは誰にも処理されず消失していた。これにより `WM_IME_KIND_CHANGED`/`WM_FOCUS_KIND_UPDATE` がエンジンスレッドに一度も届かず、MS-IME環境でwarmup戦略が誤ったまま走り続けるバグ（BUG-09）が実機で確認された。修正として「宛先を呼び出し元スレッドではなくエンジンスレッドのIDに固定する」`PostThreadMessageW(engine_thread_id(), ..)` が採用され、以後の集約点になった。

**この歴史がある以上、本ADRの提案（後述、hwnd宛への変更）はBUG-09の再発に見えかねない。しかし両者は別物である**（後述「BUG-09との違い」で明確化する）。

### `PostThreadMessageW` が持つ、BUG-09とは別の2つの脆弱性

[ADR-102](102-startup-key-delivery-one-way-closure.md) の設計・敵対的レビュー・根本原因分析の過程で、`PostThreadMessageW`（スレッドID宛のスレッドメッセージ、hwnd=NULL）自体に構造的な脆弱性があることが判明した。

1. **起動時レース**: `engine_thread_id()` が確定する（`run_message_loop` 冒頭）までの間、`post_to_main_thread` は宛先を持てない（`win32.rs:54-74` の `tid == 0` 分岐は、呼び出し元がメインスレッド自身なら偶然動くが、ワーカースレッドから呼ぶと再び「呼び出し元スレッドへの投函」に戻ってしまう——**この分岐自体がBUG-09と同型の罠を内包している**）。
2. **ネストしたモーダルポンプ中の恒久消失**: `TrackPopupMenu`/`MessageBoxW` 等がスレッド上で独自のネストしたメッセージポンプを回している間、そのポンプの `GetMessage` は hwnd=NULLのスレッドメッセージも取り出すが、`DispatchMessageW` は**ウィンドウを持たないメッセージに対して何もせずに返る**。つまり「キューから除去された上で誰にも処理されない」——BUG-09とは異なる、Win32の別の一方通行である。

この2点が、[ADR-102](102-startup-key-delivery-one-way-closure.md) が積み重ねようとしていた対症療法（起動時レース対策・`ModalPumpGuard`・`NEEDS_ENGINE_RESYNC`等）の根本原因である。

### 集約点を迂回している3箇所（うち2箇所は新規発見のバグ）

`post_to_main_thread{,_with}` を経由せず、生の `PostThreadMessageW` を直接呼んでいる箇所が3つある。

| 箇所 | 宛先スレッド | 用途 | 脆弱性2（ネストポンプ中消失）の影響 |
| --- | --- | --- | --- |
| `hook.rs:517` | フックスレッド自身（`hook_thread_id`） | フック解除時の `WM_QUIT` | **対象外**。フックスレッドは自前の最小 `GetMessageW`/`DispatchMessageW` ループしか持たず、ネストしたモーダルポンプを一切回さない |
| `app/bootstrap.rs:608-612`（`install_ctrl_handler`） | エンジンスレッド（`main_thread_id`） | Ctrl+C ハンドラの `WM_QUIT` | **該当する（新規発見）**。トレイメニュー表示中にCtrl+Cを押すと、`WM_QUIT` がネストポンプに捨てられアプリが終了しない |
| `app/bootstrap.rs:736-742`（`--exit-after` デバッグ機能） | エンジンスレッド（`main_thread_id`） | デバッグ用タイムアウト自動終了の `WM_QUIT` | **該当する（新規発見）**。同上のタイミングでタイムアウトが発火すると自動終了に失敗しうる |

### 実機実験による検証（2026-08-26、dragonflyg4）

「hwnd宛の `PostMessageW` はネストしたモーダルポンプ中でも配送されるか」を実機で検証した。ワーカースレッドから150ms間隔で hwnd 宛 `PostMessage` を送り続け、その最中に `MessageBox.Show`（ネストしたモーダルループ）を表示するテストを実施した結果:

```
15:13:39.305 showing MessageBox now (nested modal loop, ~4s until auto-ESC)
15:13:39.385 received tick=0
...
15:13:48.491 received tick=59
```

**MessageBoxが表示されていた約9秒間、送信した60件全てが欠落・遅延なくWndProcに届いた。** これは以下の既存の状況証拠（コード上の裏付け）とも整合する。

- `win32-async` の依存先 `winmsg-executor`（`winmsg-executor-0.3.2/src/util/window.rs:135`）は、既に `WindowType::MessageOnly`（`HWND_MESSAGE` を親とするメッセージ専用ウィンドウ）を使って wake メッセージを `PostMessageA` で送っており、そのdocコメント（`:157-170`）は「wndprocはモーダルダイアログ（例: ポップアップメニュー）から再入されうる」と明記している——ライブラリ作者が実際に観測した挙動である。
- `tray.rs` のdocコメント（2026-07-27実機回帰の記録）は、トレイメニューの `WM_COMMAND` が `TrackPopupMenu` 自身の内部モーダルループから実際に同期配送されることを確認済みと記録している。

## 決定

### 決定1: エンジンスレッド専用のメッセージ専用ウィンドウ（`HWND_MESSAGE`）を bootstrap 最初期に作る

```rust
// crates/awase-windows/src/runtime/engine_window.rs（新設、cfg(windows)）

/// エンジンスレッドが所有する内部専用ウィンドウ（HWND_MESSAGE 配下）。
/// ワーカースレッド・フックスレッドからの全ての内部メッセージの宛先。
/// トレイウィンドウ（top-level、`TaskbarCreated` 受信のため top-level が必須）とは役割を分ける。
static ENGINE_HWND: AtomicIsize = AtomicIsize::new(0);

#[must_use] pub fn engine_hwnd() -> Option<HWND>;

/// bootstrap の `install_hooks_and_hotkeys_validated` より前に呼ぶ。Drop で `DestroyWindow`。
pub fn create_engine_window() -> windows::core::Result<EngineWindowGuard>;
```

`CreateWindowExW` の親に `HWND_MESSAGE` を渡す最小実装（`winmsg-executor` の `WindowType::MessageOnly` と同じパターン、`winmsg-executor-0.3.2/src/util/window.rs:96-141` 参照）。`winmsg-executor` を直接依存に加えて再利用してもよいし（`win32-async` が既に間接依存している）、15行程度なので自前で書いてもよい——どちらでも実装コストの差はほぼ無い。

呼び出し位置は現行の TID 公開点（`app/bootstrap.rs` の `install_hooks_and_hotkeys_validated` 呼び出しの直前）と同じにする。ウィンドウ作成にメッセージポンプが動いている必要はない（`CreateWindowExW` はどのスレッドから呼んでも、そのメッセージは呼び出したスレッドのキューに積まれ、後でポンプされたときに処理される——TIDの前倒しと同じ理屈）。

### 決定2: `post_to_main_thread{,_with}` の実装を `PostMessageW(engine_hwnd(), ..)` に差し替える

```rust
// win32.rs
pub fn post_to_main_thread_with(msg: u32, wparam: usize, lparam: isize) -> bool {
    let Some(hwnd) = crate::runtime::engine_window::engine_hwnd() else {
        // engine window 作成前（bootstrap 最初期のみ）。呼び出し元がメインスレッド自身なら
        // 現行の tid==0 分岐と同じ理屈で自スレッドキューへの投函が正しく届く。
        // ただしワーカースレッドから呼ぶとBUG-09と同型の罠になるため、成功扱いにはしない。
        let _ = unsafe { PostMessageW(None, msg, WPARAM(wparam), LPARAM(lparam as isize)) };
        return false;
    };
    if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam)) }.is_err() {
        log::warn!("[post-main] PostMessageW failed msg=0x{msg:X}");
        return false;
    }
    true
}
```

通常の呼び出し規約は変えない（戻り値は無視可能）。ただし `WAKE_PENDING` のような再試行可能ラッチを持つ呼び出し元は、戻り値 `false` を見てラッチを解除し、次のイベントまたは既存ウォッチドッグで再postできるようにする。実装1関数の差し替えだけで、呼び出し元の大半が新方式に移行する。これが「集約点を先に作っておいた」過去の設計判断（BUG-09修正時）の利得である。

### 決定3: 集約点を迂回している生の `PostThreadMessageW` / `PostMessageW(None, ..)` を塞ぐ

`app/bootstrap.rs:608-612`（Ctrl+Cハンドラ）と `:736-742`（`--exit-after`）を、`post_to_main_thread(WM_QUIT)` 経由に変更する。`hook.rs:517`（フックスレッド自身へのWM_QUIT）は対象外（上記の理由により脆弱性2に該当しない）。

同じく `tsf/probe_bridge.rs::post_drain_output_queue` に残っていた生の `PostMessageW(None, WM_DRAIN_OUTPUT_QUEUE, ..)` は、集約点を迂回する NULL-hwnd thread message であるため `win32::post_to_main_thread(WM_DRAIN_OUTPUT_QUEUE)` へ移す。

なお `timer.rs::SetTimer(None, 0, ms, None)` と `app/bootstrap.rs::RegisterHotKey(None, ..)` は、Win32 API の性質上まだ呼び出しスレッドのメッセージキューへ `WM_TIMER` / `WM_HOTKEY` を投函する NULL-hwnd の thread message 源である。今回のADR-105実装では即時修正しないが、トレイメニュー等のネストしたモーダルポンプ中に取り出されると外側の手書きdispatchへ戻らず消失しうる残存脆弱性として扱う。これは `post_to_main_thread` 集約点の迂回ではなく、API自体の登録先が thread queue である別カテゴリの問題である。

**証拠義務**: `tests/architecture_guard.rs` に「`crates/awase-windows/src/` 内で `PostThreadMessageW`/`PostMessageW(None, ..)` を直接呼んでよいのは `win32.rs`（集約点の実装自身）と `hook.rs`（フックスレッド自身へのWM_QUIT）だけ」の grep guard を追加し、件数を固定する。`docs/known-bugs.md` に暫定 **BUG-82**「トレイメニュー表示中のCtrl+C/--exit-afterでアプリが終了しない」を起票する（実機未確認、[ADR-102](102-startup-key-delivery-one-way-closure.md) の実験と同じ機構による理論上のバグとして記録）。

### 決定4: `run_message_loop` の特殊caseマッチを、エンジン窓の本物の `WndProc` に置き換える

現行 `app/mod.rs:343-350` の `loop { GetMessageW(..); match msg.message { WM_TIMER => ..., WM_EXECUTE_EFFECTS => ..., ... } }` という手書きディスパッチは、**`DispatchMessageW` を素通りしている**。この形のままだと、ネストしたモーダルポンプ側の `GetMessage`/`DispatchMessageW` はこの `match` を経由しないため、hwnd宛にしただけでは足りない。

```rust
// runtime/engine_window.rs
unsafe extern "system" fn engine_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if dispatch_engine_message(msg, wparam, lparam) {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 唯一のディスパッチテーブル。旧 run_message_loop の match アームの移設先。
fn dispatch_engine_message(msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    match msg {
        WM_TIMER => { let _ = with_app(|app| unsafe { message_handlers::handle_wm_timer(app, wparam.0, ..) }); true }
        WM_EXECUTE_EFFECTS => { let _ = with_app(|app| unsafe { message_handlers::handle_wm_execute_effects(app) }); true }
        // ... 他の WM_* アームも同様に移設
        _ => false,
    }
}
```

`run_message_loop` 本体は標準形（`GetMessageW` → `TranslateMessage` → `DispatchMessageW`）に単純化する。`DispatchMessageW` はメッセージのhwndからウィンドウクラスを引いて `engine_wnd_proc` を呼ぶ——**これは呼び出し元が `run_message_loop` 自身であってもネストしたモーダルポンプであっても同じ経路**である。これにより:

- [ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-a（TID前倒し）・1-b（配送失敗時のOS返却）は不要になる（配送自体が失敗しなくなる）。
- [ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-f（`ModalPumpGuard`によるフック側の消費抑止）・1-g（`NEEDS_ENGINE_RESYNC`）は「配送を守る」役割からは解放されるが、「メニュー表示中はNICOLA変換せずpassthroughする」という**処理ポリシー**の判断材料としては引き続き必要——これは[ADR-102](102-startup-key-delivery-one-way-closure.md)の書き直しで扱う。

### 決定5: `WM_QUIT` はネストしたポンプ中でも標準どおり動く（確認事項、変更なし）

`WM_QUIT` はhwnd無しの特別なメッセージで、`GetMessage`/`PeekMessage` は原則どの窓に対する呼び出しでも `WM_QUIT` を最優先で取り出す（Win32の標準仕様）。これは `PostThreadMessageW` でも `post_to_main_thread` 経由でも変わらない。決定3が塞ぐのは「ネストしたモーダルポンプが `WM_QUIT` を**そもそも受け取らない**」ケースではなく、「スレッドメッセージとして送られた `WM_QUIT` が、ネストポンプの `DispatchMessageW` では何も起こさず捨てられる」ケースである（`WM_QUIT` は `DispatchMessageW` されず `GetMessage` 自体が偽を返してポンプを終了させる設計だが、**ネストしたモーダルループ自身がそれで終了してしまい、外側の `run_message_loop` には伝わらない**——これも一種の「意図しない場所での消費」であり、hwnd宛メッセージにしても解決しない。したがって決定3のCtrl+C/`--exit-after`修正は、`WM_QUIT` を送るタイミングを「次にネストポンプを抜けた後」まで遅延させる仕組み（`request_quit()`が既にグローバルフラグを立てている前提を活かし、`WM_QUIT`実送信を`ModalPumpGuard`のDrop時点まで遅延させる）を含める必要がある——単純なhwnd宛への差し替えだけでは不十分である点を実装時に見落とさないこと。

## 「BUG-09の再導入ではない」ことの説明

| | `PostMessageW(None, ..)`（BUG-09、旧実装） | `PostMessageW(Some(engine_hwnd), ..)`（本ADR） |
| --- | --- | --- |
| 宛先の決まり方 | **呼び出したスレッド自身**（Win32仕様で hwnd=NULL は自スレッドの意） | **`engine_hwnd` を所有するスレッド**（＝常にエンジンスレッド、呼び出し元スレッドに依存しない） |
| ワーカースレッドから呼んだ場合 | 消失（ワーカースレッドはこのメッセージをポンプしない） | 正しくエンジンスレッドへ届く |
| 起動時レース | 無関係（そもそも別の失敗形） | `engine_hwnd()` が `None` の間だけ従来と同じ理屈のフォールバックが必要（決定2） |

**BUG-09が「宛先をNoneにするな」だったのに対し、本ADRは「宛先を実在するhwndにする」であり、退行ではなく同じ教訓（宛先を明示的かつスレッド非依存にする）の延長線上にある。**

## 未解決の疑問（実装時に確認・実機ソークで確認すること）

- 決定4のディスパッチ移設は `run_message_loop` の広範な書き換えになる。移設漏れがないことを、既存の `match` アーム一覧と `dispatch_engine_message` の対応表で機械的に確認すること。
- 決定5の `WM_QUIT` 遅延送出は、`ModalPumpGuard`（[ADR-102](102-startup-key-delivery-one-way-closure.md) 書き直し後の版）のDropフックに載せる形が自然だが、両ADRの実装順序に依存関係が生まれる。**本ADRの決定3は[ADR-102](102-startup-key-delivery-one-way-closure.md)のモーダルポンプ検出機構と同一コミットか、直後のコミットで入れること。**
- `create_engine_window()` が失敗した場合（`CreateWindowExW` の失敗はまれだが起こりうる）のフォールバック——決定2のフォールバック分岐がその代替経路になるが、恒久的にフォールバックのままだと決定4の恩恵（ネストポンプ耐性）を失う。起動失敗として扱うべきか、フォールバック運用を許容するかは実装時に判断する。
- `architecture_guard` のgrep guardは、[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) が言及するdylintの限界（ファイル名・件数のテキスト検査であり、新しい呼び出し箇所の追加は検知するが型では強制しない）と同じ弱さを持つ。件数固定で十分と判断する（この領域はdylintを新設するほどの意味論的偽装ではなく、単純な「呼び出し箇所が2つに限定される」という事実の固定で足りる）。

## 関連

[ADR-102](102-startup-key-delivery-one-way-closure.md)（起動シーケンスとキー配送）は本ADRの決定を前提として書き直す。書き直し後は決定1-a/1-b/1-b-2/1-f/1-gの多くが不要または縮小され、1-c/1-d/1-e/2-a/2-bは独立した価値を保つため温存される見込み。
