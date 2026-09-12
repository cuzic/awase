---
id: ADR-164
title: |-
  グローバルstatic縮小 — 引数引き回し優先＋残りは単一singleton集約の段階的リファクタ計画
summary: |-
  ADR-158とは独立に、「グローバルstaticが76件(30%が`hook.rs`)あること自体がスメル」というユーザー指摘から起票。ワークスペース全体を実地調査しA(OSコールバック境界)/A2(クロススレッド共有state、round1で新設)/B(引数引き回し可能)/C(正当な可変シングルトン)/C-immutable(不変ディスパッチテーブル、対象外)/D(要追加調査)に分類。opus-adversarial-consult round1で当初のフェーズ4案(`hook.rs`22件を`&mut HookState`引き回し)の前提が実コードと矛盾すると判明し全面書き直し(20件をロックフリーstruct-of-atomics singletonへ、Mutex明示的に禁止)。round2で新フェーズ3(`tray.rs`)に新たなリスク(トレイメニュー消失)が見つかりユーザー判断で対象外化。round3で反映時の記述矛盾3件を訂正し収束
status: |-
  round1〜round3実施・収束(Must-fixゼロ)。フェーズ1(PR#197)・フェーズ2(PR#198)・フェーズ8(PR#199)・フェーズ5(PR#200)・フェーズ6(PR#204)・フェーズ4(PR#205)がいずれもdevelopマージ済み。フェーズ3・7は対象外化、フェーズ9はユーザー判断で保留。ほぼ完了
related_adr:
  - "ADR-119"
  - "ADR-158"
  - "ADR-159"
  - "ADR-162"
---

# ADR-164: グローバルstatic縮小 — 引数引き回し優先＋残りは単一singleton集約の段階的リファクタ計画

## ステータス

起草。opus-adversarial-consult round1〜round3実施・反映済み・**収束（Must-fixゼロ）**。
round1で当初案（フェーズ4の「`hook_callback`以下を`&mut HookState`引き回し」）の前提が
実コードと矛盾すると判明し、決定節を全面的に書き直した。round2でフェーズ4の訂正版設計
（struct-of-atomics、Mutex禁止、アクセサ署名不変）は成立すると確認され、一方フェーズ3
（`tray.rs::MENU_TARGET_HWND`）は訂正しても新たなリスク（全トレイマウスイベントでの
150msブロッキング、排他借用への格上げによるトレイメニュー消失リスク）が残ると判明し、
ユーザー判断により**対象外へ変更**した。round3はround2反映時にADR自身に混入した3件の
記述矛盾（診断リングのMutex/atomic混同、同居の安全性根拠の誤り、検証方法節とフェーズ
一覧の矛盾）を指摘・訂正し、round4不要（収束）と判定された。
[ADR-158](158-complexity-reduction-north-star.md)の複雑性棚卸しの追調査から派生した独立ADR
（158の採択A〜Eいずれの子ADRでもない、新規の観点）。

**実装状況（2026-09-12）**: フェーズ1（`gji_charset_autodetect.rs`の3ラッチ）は
PR [#197](https://github.com/cuzic/awase/pull/197)でdevelopにマージ済み。
フェーズ2（`msime_key_assignment.rs::LAST_WARNED`）はPR
[#198](https://github.com/cuzic/awase/pull/198)でdevelopにマージ済み。
フェーズ8（`state/probe_admission.rs`の3カウンタ）はPR
[#199](https://github.com/cuzic/awase/pull/199)でdevelopにマージ済み。
フェーズ5（`probe_actuation_fence.rs`の5静的）はPR
[#200](https://github.com/cuzic/awase/pull/200)でdevelopにマージ済み
（実機ソーク実施——途中、Windows共有作業ディレクトリを他セッションが同名
ローカルブランチで上書きする事故が発覚し、`git checkout -B`で正しいコミットへ
復元・developの最新も取り込んだ上で再ソークして確認済み。
worktreeをセッションごとに分離すべき理由として過去にも記録済みの事故の実例）。
フェーズ7は実装着手時の再分類でADR起票時の誤り（別モジュール・別cfgゲートの
2静的を「同一ファイルだから」まとめようとしていた）が判明し、対象外へ廃止した
（コード変更なし、developへ直接マージ済み）。
フェーズ6（`lib.rs`の3静的）はPR [#204](https://github.com/cuzic/awase/pull/204)で
developにマージ済み（トレイ「終了」経由でCtrl+Cハンドラと同一コード経路の実機確認済み）。
フェーズ9は保留（ユーザー判断、2026-09-12）。
フェーズ4（`hook.rs`の20静的）はPR [#205](https://github.com/cuzic/awase/pull/205)で
developにマージ済み（本ADR全体で最高リスクのフェーズ、詳細はフェーズ4本文の実装節参照）。
フェーズ3は対象外（round2で決着済み）。

**現時点（2026-09-11）のまとめ**: フェーズ1・2・4・5・6・8が全てdevelopマージ済み、
フェーズ3・7は対象外、フェーズ9はユーザー判断で保留。本ADRが対象としたフェーズは
実質完了している。

## 背景

### 発端

2026-09-10、ADR-158複雑性インベントリ（`158-complexity-inventory-2026-09-10.md`）の追調査として、
`decide_gate`/`decide_chain`/`decide_attempt`の全数enumeration・`RESTRICTED_CALLS`許可呼び出し元の
grep・グローバルstatic76件の参照回数スイープという3種の機械的死活チェックを実施したが、いずれも
「死んでいる（到達不能な）」staticやコード経路は見つからなかった。

死活とは別軸で、ユーザーが「グローバルstaticが（`crates/awase-windows/src`だけで）76件あること
自体が強いスメルではないか」と指摘。個々のstaticは正当な理由を持っていても、集合として見たときの
設計原則が欠けている、という指摘である。

### 原則の確定

議論の結果、以下の原則で合意した:

- **第一選択は関数引数での引き回し**。あるstaticを必要とする全呼び出し元が、既存の呼び出し木の中で
  共通の祖先関数（Rustの通常呼び出しで辿れる、FFI境界でない、**かつ同一OSスレッド上**）から
  到達できるなら、そこから`&mut State`のような形で引数として引き回すリファクタを行う。
- **static構造体への集約は次善策**。以下のいずれかで引数引き回しが構造的に不可能な場合に限る:
  - Win32の`HOOKPROC`/`WNDPROC`/`WinEventProc`のようにOS側が直接呼び出す固定シグネチャの
    コールバック。
  - **複数のOSスレッドから共有される「メールボックス」状態**（一方のスレッドが書き、
    他方が読む。呼び出し元が2つ以上のスレッドに分散しており、Rustの通常の呼び出し木を
    辿っても共通祖先が存在しない）。
  - 複数の独立したsyscallチョークポイントから叩かれ、単一の呼び出し木を持たない診断カウンタ等。
  - いずれの場合も、N個の裸staticをただ1つのstruct内staticにまとめるだけであり、それ以上の
    意味論的凝集は保証しない。**構造体化にあたってMutexを導入してはならない場合がある**
    （下記フェーズ4の訂正3参照——ホットパスでロックを挟むとOS側のタイムアウトでフックが
    サイレントに外れる等の実害が出うる）。
- 判定基準: 「そのstaticを読み書きする全関数を辿ったとき、共通の非FFI・単一スレッドの祖先関数に
  到達できるか」。到達できれば(B)引数引き回し対象、できなければ（かつ上記いずれかの構造的理由が
  あれば）(A)構造体化許容。
- **(B)と追加原則の適用範囲の違い（round2 M2後半、明文化）**: (B)引数引き回しは、対象が
  同一ファイルに**1件だけ**でも実施対象になりうる——引数引き回しはstatic自体を消せる
  （singletonに残す集約とは異なり、フィールド化ではなくRuntime等の既存構造体へ吸収する
  ため）。一方、下記「追加原則」（同一並行性ドメイン内の裸static複数個を1つのsingletonに
  集約する）は、**2件以上が同じ並行性ドメインに並んでいる場合にのみ**適用する規則であり、
  1件しかない裸staticを「原則を満たしていないから」という理由だけでsingleton化する必要は
  ない。ただし、その1件を(B)化しようとした結果、呼び出し元の借用境界（モーダルループ等）を
  跨ぐ設計変更が必要になり、それによってリスクが上がる場合は、**(B)化そのものを見送り
  対象外に留める**という判断もありうる（フェーズ3参照、round2 M1/M2）。
- **追加原則（2026-09-10、ユーザー指摘により拡張）**: 「引数引き回しできない箇所を単に放置する」
  のは不十分。**同一ファイル/モジュール内に裸のtop-level static（`AtomicXxx`/`Mutex`/`OnceLock`
  等のプリミティブを直接staticで持つもの）が複数並んでいる状態そのものを解消する**——
  引数引き回しが効かない（B）以外の全てについても、**1つの独立した並行性ドメイン（同じスレッドの
  組み合わせから読み書きされる状態のまとまり）につき1つ**のsingleton構造体に集約し、個々の
  フィールドとして持たせる。1ファイルに複数の並行性ドメインが存在する場合（例:
  スレッド専有state＋クロススレッド共有stateが同居）は、ドメインごとに1つ、計2つ以上の
  staticになってよい——「ファイルにつき機械的に1つ」ではなく「並行性ドメインにつき1つ」が
  正しい単位である（round1 M3で訂正）。
  - **例外**: 可変状態を一切持たない不変静的データ（`LazyLock`で初期化される定数テーブル、
    ゼロサイズ構造体への`&'static dyn Trait`ディスパッチテーブル等）は対象外とする。これらは
    「複数のグローバル可変状態が個別に生きている」という本ADRが問題視するスメルとは無関係
    （データ競合もreentrancyも発生しない、コンパイル時定数のルックアップに過ぎない）。
    下記の分類(C)のうち、この例外に該当するものは(C-immutable)として区別する。
  - **例外2**: 1関数内にスコープされたwrite-onceキャッシュ（`OnceLock`を関数ローカル
    staticとして持つパターン）は、既に最小のスコープで正当化されており対象外
    （round1 M4——これらを親singletonへ引き上げるとスコープが逆に拡大し、原則の目的に反する）。

### 全体調査結果（2026-09-10実施、round1指摘を受けて訂正済み）

ワークスペース全体（ルート`awase`コア`src/`、`crates/awase-windows/`、`awase-linux/`、
`awase-macos/`、`win32-async/`、`win32-worker/`、`awase-settings/`、`awase-gji-config/`、
`awase-vkmap/`、`timed-fsm/`）を対象に、`static`宣言（`OnceLock`/`Mutex`/`RwLock`/`AtomicXxx`/
`RefCell`/`thread_local!`を含む）を実地に`rg`+`Read`で洗い出し、各ファイルを以下の分類に振った。

再現用コマンド（round1 N1指摘、以後の再カウントはこれを使う）:
```sh
rg -n '^\s*(pub(\([a-z()]*\))?\s+)?static [A-Z_0-9]+\s*:' crates src
```
このコマンドでは97件ヒットする一方、最初のfork調査では88件と集計しており、**差分9件は
未特定**（関数ローカルstaticの扱いの違いが疑われるが未確認）。`crates/awase-windows/src`
限定では76件で両集計が一致している。**実装着手時は上記コマンドで対象ファイルを再カウント
すること**。

- **(A) OS/FFIコールバック境界・スレッド専有領域で真にグローバルが必要**: シグネチャを
  変更できないコールバック、または特定の1スレッドのみがアクセスすることが前提の状態
  （例: `HOOK_HANDLE`、hookスレッド専有）。次善策（構造体化、ただし単独1件なら現状で
  既に条件を満たす）の対象。
- **(A2) クロススレッド共有state（新設、round1 M1）**: 一方のスレッドが書き、他方が読む
  「メールボックス」。引数引き回しは構造的に不可能（呼び出し元が別スレッドにあり
  共通祖先を持たない）。**ロックフリーなstruct-of-atomics singleton**に集約する
  （Mutexは原則禁止、理由は下記フェーズ4参照）。
- **(B) 引数引き回しへリファクタ可能**: 呼び出し元が共通の非FFI・単一スレッドの祖先を持つ。
  本ADRの最優先実施対象。
- **(C) 正当なプロセスグローバル可変状態（要singleton集約）**: 複数の独立したsyscallチョーク
  ポイントから叩かれ単一の呼び出し木を持たない診断カウンタ等。**同一ファイルに複数ある場合は
  1つのsingleton構造体に集約する**。
- **(C-immutable) 不変静的データ（対象外）**: `LazyLock`定数テーブル、ゼロサイズ構造体への
  `&'static dyn Trait`ディスパッチテーブル等、可変状態を持たないもの。集約不要。
- **(D) 判断保留**: 呼び出し経路が未確認、またはスタブ実装で成熟度が低い。

**ファイル別分類（主要ファイルのみ、round1で実コード確認・訂正済み）**:

| ファイル | 件数 | 分類 | 根拠 | singleton集約後の目標static数 |
|---|---|---|---|---|
| `hook.rs` | 21（module-level）+ 2（関数ローカル、対象外） | A(1: `HOOK_HANDLE`)+A2(20) | `HOOK_HANDLE`(:706)のみhookスレッド専有の(A)。残り20件（物理キー状態9件・キャッシュ設定7件・起動ハンドシェイクスロット1件`HOOK_TID_INIT_SLOT`・診断/生存監視3件、round2 N1で`HOOK_TID_INIT_SLOT`を診断枠から分離）が**全てhookスレッド⇔メインスレッド間の双方向メールボックス**（round1 M1で実コード確認、詳細はフェーズ4参照）。関数ローカルの`CACHE`(:661)・`BASELINE`(:1248)は対象外(例外2) | **2**（`HOOK_HANDLE`は不変、20件を`HOOK_STATE`struct-of-atomicsへ） |
| `probe_actuation_fence.rs` | 5 | C（うち1件は actuation判定ロジック本体） | `send_input_safe`/`send_ime_control`という2つの独立syscallチョークポイントから叩かれ、単一呼び出し木を持たない。`PROBE_ACTUATION_FENCE`自体は`FencedProbeOutcome::Abandoned`判定に使われる**actuation合流点**（round1 M7で訂正、診断カウンタは4件のみ） | **1**（ロックフリー必須、フェーズ5参照） |
| `lib.rs` | 5 | A2(3)+既存集約点(1)+C-immutable相当(1) | `MAIN_THREAD_ID`(:123)/`QUIT_REQUESTED`(:132)/`ELEVATED`(:141)の3件がCtrl+Cハンドラ等クロススレッド共有（round1 M5で`ELEVATED`の記載漏れを訂正）。`RUNTIME`(:204)は既存の集約点。`RAW_TSF_LITERAL`(:200)は既に`RawTsfLiteralPending`という3フィールドsingleton構造体で**原則を既に満たしている** | **3**（`RUNTIME`・`RAW_TSF_LITERAL`は変更不要、3件を1 singletonへ） |
| `ime_controller.rs` / `state/ime_profile_driver.rs` | 4+3 | **C-immutable** | ゼロサイズ構造体の`&'static dyn`ディスパッチテーブル、可変状態なし | 対象外（例外） |
| `hook_channel.rs` | 4 | A2(3)+既存集約点(1) | `HOOK_KEYS`(:184)は既に`HookKeyRing`構造体で原則を満たす。`WAKE_PENDING`(:185)/`WAKE_POST_FAILED`(:191)/`WAKE_POST_FAILED_LIFETIME_COUNT`(:198)の3件が裸のまま並ぶ（round1 N2で実測、フェーズ9） | **2** |
| `runtime/engine_window.rs` | 3 | A2 | `ENGINE_HWND`/`MODAL_DEPTH`/`NEEDS_ENGINE_RESYNC`（:14-16）が裸のまま並ぶ。`engine_wnd_proc`（`WNDPROC`固定署名）およびネストしたモーダルポンプからの再入で使われる（round1 N2で実測、フェーズ9） | **1** |
| `gji_charset_autodetect.rs` | 3 | **B** | doc既に2関数（`sync_gji_charset_autodetect`/`reset_streak_latch_for_reload`）限定、両方とも既に`&mut Runtime`を保持 | **0**（Runtimeフィールド化、staticそのものを消せる） |
| `state/probe_admission.rs` | 3 | C | プロセス生存期間の棄却統計カウンタ | **1**（フェーズ8） |
| `awase-settings/src/main.rs` | 4（うち2件は`#[cfg(test)]`専用） | **対象外（2026-09-10実装着手時に訂正）** | `SETTINGS_LOG_FILE`（crateルート、全OS共通）と`SIMULATED_REGISTERED`（`autostart_bridge`サブモジュール、非Windows限定）は別モジュール・別cfgゲート・別並行性ドメインで、「同一ファイル」以外の共通点が無い | 対象外（フェーズ7廃止、現状の2件のまま） |
| `runtime/message_handlers.rs` | 2 | D | `DRAIN_PENDING`/`DRAIN_RERUN_PENDING`は`tray_wnd_proc`からも呼ばれるとdocにあるが呼び出し経路未確認、A/Bどちらか要追加調査 | 判断保留 |
| `focus/classifier.rs`(`INPUT_RELAY_APPS`) | 2 | A | CLAUDE.mdが「唯一の意図的な例外」と明記済み。`read_ime_state_fast`が`self`無し`pub unsafe fn`のため。**変更対象外** | 1（現状維持） |
| `app/bootstrap.rs`(`LAST_FOCUS_HWND`) | 1 | A | `win_event_proc`（`WinEventProc`固定署名）内static、`hook_callback`と同型 | 1（既に1件のみ、対応不要） |
| `tray.rs`(`MENU_TARGET_HWND`) | 1 | **A（round2で対象外に変更）** | `handle_wm_app_tray`→（`TrackPopupMenu`ネストモーダルループ経由）→`runtime/message_handlers.rs::handle_wm_command`が`tray::menu_target_hwnd()`経由で読む。(B)化を試みたがround2 M1/M2でモーダルループ境界を跨ぐ借用格上げが新たなリスクを生むと判明し、対象外に変更（旧フェーズ3参照） | 1（現状維持、対象外） |
| `msime_key_assignment.rs`(`LAST_WARNED`) | 1 | **B** | `check_and_warn()`は引数なし、唯一の呼び出し元は`runtime/message_handlers.rs:1043`で同一関数内に`app: &mut Runtime`が既にスコープ内（round1 N3、`sync_gji_charset_autodetect(app, ..)`と同じ関数） | **0** |
| その他（診断カウンタ・OnceLockシングルトン等、約20ファイル） | 各1 | 主にC | 1ファイル1staticで既にsingleton条件を満たす。**未検証の残余——round1 N4指摘、実装着手時に上記rgコマンドで全件再確認すること** | 1（対応不要と推定、要確認） |
| Linux/macOSスタブ`hook.rs`各2〜3件 | — | D | プラットフォーム実装が未成熟、対応方針が固まるまで判断保留 | 判断保留 |

## 決定

ADR-158の複雑性予算制（`.claude/rules/tuning-constants.md`や
`.claude/rules/complexity-budget.md`とは対象が異なる——本ADRはtuning定数でもactuation合流点数でもなく
「グローバル可変状態の個数」を扱う、対応する予算制機構は現状無い）とは独立に、以下の順で
リファクタを段階的に実施する。**各フェーズは独立したPRとし、フェーズ間で
opus-adversarial-consultやユーザーレビューを挟んで良い**。

### フェーズ1: `gji_charset_autodetect.rs`の3ラッチ（最優先）

対象: `LAST_GJI_STREAK_CHECKED`/`LAST_TOGGLE_WARNING`/`LAST_MODE_KEY_THUMB_WARNING`。

呼び出し元の`sync_gji_charset_autodetect`/`reset_streak_latch_for_reload`は既に`&mut Runtime`を
引数に持つため、3ラッチをRuntimeのフィールド（またはRuntime配下の専用struct）に移すだけで完結する。
新規のFFI境界を通らない。

- 規模: 小（2関数＋呼び出し元1箇所）。
- リスク: 中。`.claude/rules/fix-requires-evidence.md`の「キー選択（IME ON/OFF に送る VK）」ファミリーに
  触れる（BUG-115系、具体的には`sync_gji_charset_autodetect`が書く
  `set_gji_mode_key_shadow_overrides`/`set_gji_mode_key_delegate_to_open_axis`/
  `set_thumb_key_shadow_overrides`が`src/engine/nicola_fsm.rs::resolve_pending_thumb_as_single`の
  `delegate_to_open_axis`/`ModeKeyConfig`優先順位＝BUG-119ファミリーに直結、round1 M8）。
  **`tests/ime_key_sequence_golden.rs`は`#![cfg(windows)]`でこのLinuxサンドボックスでは
  実行検証できない**（CLAUDE.mdに明記の通り）。fix-requires-evidence.md自身の表が指す
  **`src/engine/tests.rs`（`cargo test --lib`、ホストターゲットで実行可）でのテスト追加を
  完了条件に含めること**。加えて、以下2点の意味論を壊さないことを設計レビューで
  明示的に確認する:
  1. `reset_streak_latch_for_reload`によるラッチリセット（設定リロード時、BUG-115 F4対策）。
  2. `LAST_TOGGLE_WARNING`が「GJI離脱ではリセットしない」（Q3方針、GJI⇔MS-IME往復のたびに
     再警告しないための意図的な非対称）。
  Runtimeフィールド化した際、将来Runtimeの再作成/再初期化経路が増えると黙ってリセットされ、
  警告が再発火する挙動変化が起きうる——golden/engine testsでは検出できないため、設計時に
  「いつRuntimeのこのフィールドがリセットされてよいか」を明文化すること。

### フェーズ2: `msime_key_assignment.rs::LAST_WARNED`

`check_and_warn()`（引数なし）単体内で完結するデデュープラッチ。唯一の呼び出し元
（`runtime/message_handlers.rs:1043`）は`sync_gji_charset_autodetect(app, ..)`と同じ関数内にあり、
`app: &mut Runtime`が既にスコープ内にある（round1 N3で確認済み）。`check_and_warn(app: &mut
Runtime)`へシグネチャ変更しRuntimeフィールド化する。

- 規模: 極小。
- リスク: 低。警告表示ロジックのみでactuationには触れない。
- **注意（round2 N2）**: `check_and_warn`は`MessageBoxW`がフックスレッドのメッセージ処理を
  止めるのを避けるため、確認ダイアログを`spawn_yes_open_ime_settings_dialog`
  （`msime_key_assignment.rs:295`）で**別スレッドにspawn**している。`check_and_warn(app: &mut
  Runtime)`へシグネチャ変更する際、その`&mut Runtime`参照をspawnするクロージャへ渡さないこと
  （`Runtime`は`SingleThreadCell`前提でスレッド跨ぎ不可）。

### フェーズ3（廃止・対象外へ変更）: `tray.rs::MENU_TARGET_HWND`

**round1 M6で当初案（リスク「低」）の誤りを訂正し(B)化を試みたが、round2 M1/M2で
訂正後の設計にも新たなリスクが見つかり、ユーザー判断により(B)化を見送り対象外とした。**

当初、`MENU_TARGET_HWND`は`handle_wm_app_tray`→（`TrackPopupMenu`ネストモーダルループ経由）→
`runtime/message_handlers.rs::handle_wm_command`（`tray::menu_target_hwnd()`経由）という
経路で読まれるため、「`handle_tray_message`を呼ぶ前の`with_app`クロージャ内で捕捉を完了させる」
設計でRuntimeフィールド化を試みた。しかしround2レビューで以下2点が判明した。

- **(M1) `WM_RBUTTONUP`フィルタは`handle_tray_message`の内側**（`tray.rs:578-580`）にあり、
  `handle_wm_app_tray`の借用ブロックより後。Shellはトレイアイコン上の全マウスイベント
  （移動・左クリック等含む）でコールバックを呼ぶため、「呼ぶ前に捕捉」を字義通り実装すると
  `tray.rs:586-590`の150msブロッキング`get_gui_thread_info_with_timeout`が**カーソルを
  乗せている間ずっと**排他借用を保持したまま繰り返し走り、その間の`with_app`/
  `with_app_or_repost`が軒並み再入扱いになる——直そうとしたバグより悪い回帰になりうる。
  `WM_COMMAND`はモーダルループ中に同期配送されるため「戻り値で受けて後から書く」代替も
  成立しない。
- **(M2) 書き込みには`with_app`（排他借用）への格上げが必須**で、現状の読み取り専用
  `with_app_ref`（共有借用）より失敗条件が広がる。共有借用中でも失敗するようになり、
  `.unwrap_or_default()`（`message_handlers.rs:1187`）が空の`layout_names`等を返す
  確率が上がる＝レイアウト項目が消えたトレイメニューが表示されうる。

**結論**: static 1件を消すコストに見合わない。加えて、上記「(B)と追加原則の適用範囲の違い」
の通り、`tray.rs`はそもそも裸staticが1件だけであり、追加原則（複数個が並ぶ場合の集約）の
対象でもない。`MENU_TARGET_HWND`は**(A)「モーダルポンプ境界を跨ぐ受け渡し」として現状維持**
とし、本ADRの対象から外す。

### フェーズ4: `hook.rs`の20件（クロススレッド共有state）を単一struct-of-atomics singletonへ集約（実装済み、develop未マージ・実機ソーク未実施）

**round1 M1〜M4で当初案（「`hook_callback`以下を`&mut HookState`として全22件を引数引き回し」）の
前提が実コードと矛盾すると判明し、全面的に書き直した。round2レビューで訂正版の設計骨格
（struct-of-atomics・Mutex禁止・アクセサ署名不変・`HOOK_HANDLE`は別static維持）は
成立すると確認され、round3は不要と判定された。round2 M3・M4を反映済み（下記）。**

#### 訂正1: 対象は22件ではなく20件（round1 M4）

`hook.rs`のmodule-level static宣言は実測**21件**。うち`HOOK_HANDLE`(:706)は
`SingleThreadCell<HHOOK>`＝**hookスレッド専有**（doc:「このスレッドのみがアクセスする」）で、
`hook_callback`と同じ意味でのFFI境界所有物であり、**現状のまま変更不要**（既に1件のみで
原則を満たす）。残り**20件**が本フェーズの対象。関数ローカルの`CACHE`(:661)・`BASELINE`(:1248)は
write-onceキャッシュとして既に最小スコープであり対象外（例外2、これらを親singletonへ
引き上げるとスコープが拡大し原則の目的に反する）。

#### 訂正2: 「B分類16件」は存在しない、全て引数引き回し不可能なクロススレッド共有state（round1 M1）

`install_hook()`（`hook.rs:757-830`）は専用スレッド`"awase-hook"`をspawnし、その中で
`SetWindowsHookExW`と独自の`GetMessageW`ポンプを回す。したがって`hook_callback`は
**メイン/エンジンスレッドとは別スレッド**で動く。当初ADRが(B)（引数引き回し可能）と分類した
物理キー状態9件・キャッシュ設定7件は、実際には**hookスレッドとメインスレッドの双方向
メールボックス**であり、共通の非FFI祖先が存在しない（メインスレッド側の書き込み元:
`runtime/mod.rs:2089`/`runtime/message_handlers.rs:1094`の`hook::reset_physical_key_state()`、
`runtime/focus_tracking.rs:510`の`clear_hook_latches_for_app_disable`、
`app/bootstrap.rs:559-562`/`runtime/mod.rs:1860,1889-1890`/`runtime/executor.rs:749`の
キャッシュ設定書き込み。hookスレッド側の読み書き: `hook_callback`自身、`cached_hook_config()`
等）。この20件は「原則の確定」で新設した**(A2) クロススレッド共有state**に該当し、
`&mut HookState`のような引数引き回しはそもそも成立しない（メインスレッドとhookスレッドは
同時に走っており、どちらも同じ`&mut`を持てない）。

#### 訂正3: Mutexは禁止、ロックフリーなstruct-of-atomicsのみ（round1 M2）

`WH_KEYBOARD_LL`のコールバックは`LowLevelHooksTimeout`（既定5000ms、`hook.rs:660
low_level_hooks_timeout_ms()`が参照する値）内に返らないと**Windowsがフックをサイレントに
外す**。`Mutex<HookState>`のような設計にすると、メインスレッドがロック保持中に他のブロッキング
処理（`run_with_timeout`の300ms、`tray.rs:588`の150ms `get_gui_thread_info_with_timeout`、
UIA/MSAA呼び出し等）に入るたびに、hookスレッドがロック待ちで止まり**マシン全体のキー入力が
停止する**。現状、毎打鍵で触る19件がlock-free atomicなのは意図的な設計（残る1件の診断リングは
Mutexだが、下記の通り保持区間がO(1)に限定されているため実害が無い、round3 M1で訂正）。

**設計**: `HOOK_STATE`という1つのsingleton staticを新設し、**19フィールドを現状と同じ型の
atomic**（`AtomicBool`/`AtomicU32`/`AtomicU64`/`[AtomicBool; 256]`/`[AtomicU64; 256]`等）として
struct内に直接持たせ（Mutexで包まない、`&'static HOOK_STATE`への共有参照から各フィールドの
atomic操作を直接呼ぶ）、**残り1フィールド（`HOOK_IME_MODE_DIAGNOSTICS`、
`Mutex<VecDeque<..>>`）はMutexで包んだまま同居させる**（round2 M4で「20件全てatomic」の
誤記を訂正——正しくは19 atomic + 1 Mutex）。

**同居の安全性根拠（round3 M2で訂正）**: 当初「hookスレッドのホットパスには無い」と
書いたが誤り——`push_hook_ime_mode_diagnostic`（`hook.rs:876`）の唯一の呼び出し元は
`hook_callback`内部（`hook.rs:947`）で、IMEモードキー（`VK_KANA`/`VK_IME_ON`/`VK_JUNJA`/
`VK_KANJI`/`VK_IME_OFF`/`VK_DBE_*`）のKeyDown/KeyUpに限りhookスレッド自身がこのロックを
取る。正しい根拠は「ロックはhookスレッド側でも取られるが、両側とも保持区間がO(1)の
deque操作のみ（`pop_front`/`push_back`、または`drain_hook_ime_mode_diagnostics`
（`hook.rs:886`）のロック下での上限64件`Vec`への`drain(..).collect()`——アロケーションは
発生するが上限64件の有界サイズで、I/Oやブロッキング処理を含まない）で、最悪保持時間が
`LowLevelHooksTimeout`（5000ms）に対して桁違いに小さいマイクロ秒オーダーであるため実害が
無い」。これは本ADR以前からの既存挙動であり、singleton集約によって変化しない——だからこそ
以下の不変条件が必須である。**同居を安全に保つ不変条件**: このロックを
**ブロッキング処理や非有界な処理を跨いで保持しない**こと（ロック下でのアロケーションは
上限64件の`Vec`収集のみに留める）。これが崩れると訂正3が禁止した「hookスレッドがロック待ちで
止まる」経路が復活する。

既存のアクセサ関数（`cached_hook_config()`/`reset_physical_key_state()`/
`clear_hook_latches_for_app_disable()`等）は**シグネチャを変えず**、内部実装だけを
20個の裸staticから`HOOK_STATE`のフィールド参照に置き換える——「`&mut HookState`を引数として
引き回す」という当初の設計方針はこのファイルには適用しない（クロススレッド共有stateには
意味を成さない）。

結果として`hook.rs`のtop-level static宣言は**`HOOK_HANDLE`（変更なし）＋`HOOK_STATE`（新設）の
2つ**になる。

**memory ordering保存要件（round2 M3、この改修で最も重要な完了条件）**: `hook.rs`は
static ごとにordering を意図的に使い分けている（実測: `Relaxed` 49・`Release` 9・
`Acquire` 7・`SeqCst` 1）。特に`HOOK_TID_INIT_SLOT`（`install_hook()`のスピン待ちが依存する
唯一の`SeqCst`使用箇所、`Release`/`Acquire`とペアで使う）や、`FOCUS_APP_DISABLED`
（書き`Release`・アクセサ読み`Acquire`・ホットパス読み`Relaxed`という意図的な非対称）は、
機械的な置き換えで1箇所でも取り違えると（例: `Acquire`→`Relaxed`、非対称を誤って
「揃える」）、`cargo check`もclippyもgoldenも実機ソークも検出できない稀なクロススレッド
staleness（BUG-46/52/116ファミリーと同種の症状）を生む。**完了条件として、20フィールド
全ての移行前後で`Ordering::`引数が1対1で一致することを、識別子ごとの差分レビューで
確認すること**（`rg -o 'Ordering::\w+'`のヒストグラム一致だけでは不十分、個々の読み書き
箇所を突き合わせる）。

**実装前調査（2026-09-10、フェーズ1・2マージ後に実施）**: 上記の21件・4分類・ordering実測値
（Relaxed 49・Release 9・Acquire 7・SeqCst 1）・`HOOK_TID_INIT_SLOT`/`FOCUS_APP_DISABLED`の
非対称・`HOOK_IME_MODE_DIAGNOSTICS`の同居安全性根拠を、いずれも実コード（develop最新）に対して
再検証し、**全て一致**を確認した（コード変更なし、read-only調査）。ただしメインスレッド側
書き込み元の行番号（`runtime/mod.rs:2089`等）は、フェーズ1・2で`Runtime`に新規フィールド/
メソッドを追加した副作用で一部ズレている——実装時は行番号を当てにせず再取得すること。

**この調査で新たに判明した、ADR本文に無かった完了条件の漏れ**:
`crates/awase-windows/tests/architecture_guard.rs`に、`hook.rs`のソースコードをリテラル
文字列で走査するテストが2件あり、本フェーズの改修（static→`HOOK_STATE`フィールド化）で
**確実に壊れる**:

1. `disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana`
   — `"FOCUS_APP_DISABLED.load(Ordering::Relaxed)"`等のリテラル文字列を`.find()`/`.expect()`
   で探しており、`HOOK_STATE.focus_app_disabled.load(...)`のような形に書き換えると
   `.expect(...)`がpanicする。
2. `cross_thread_shared_lock_declarations_are_accounted_for` — 行頭`static `/`pub `/`pub(`
   かつ`": Mutex<"`を含む行数をカウントし固定リストと突き合わせる。`HOOK_IME_MODE_DIAGNOSTICS`
   がstructのフィールドになると`static `プレフィックスの行でなくなり、カウントが崩れる。

壊れること自体は想定内（意図的な変更なら期待値更新でよい設計のテスト）だが、**フェーズ4の
完了条件に、この2テストの期待値・リテラル文字列パターンの更新を追加する**（下記「検証方法」
節にも反映）。`hook_callback_log_call_count_is_pinned`はstatic名に依存しないため対象外。

- 規模: 大。20フィールドの`HOOK_STATE`構造体設計、既存アクセサ関数の内部実装差し替え、
  呼び出し元（メインスレッド側6箇所以上）の動作が変わらないことの確認、上記ordering保存の
  確認。
- リスク: **高**。`.claude/rules/fix-requires-evidence.md`の「キー選択」および「物理IMEキーの
  Suppress/Allow配送判断」の両再発ファミリーに直結する（BUG-46/52/116系）。
  golden回帰テスト（`tests/ime_key_sequence_golden.rs`）の拡充、および実機ソークが
  マージ条件。
- フェーズ1〜2を先に終えてから着手し、「singleton集約」パターン自体の運用を、より小さく
  安価な変更で先に検証する（実際にはフェーズ8→7→5→6の順で先に検証してから着手した）。

**実装（2026-09-12）**: ブランチ`refactor/adr164-phase4-hook-state`。実装前調査で確認した
21件・4分類・ordering実測値（Relaxed 49・Release 9・Acquire 7・SeqCst 1）は develop最新でも
完全一致していることを再確認してから着手した。

- `HOOK_IME_MODE_DIAGNOSTICS`（`Mutex<VecDeque<..>>`）+19個の`Atomic*`静的
  （`[AtomicBool; 256]`/`[AtomicU64; 256]`含む）を`HookState`構造体1つに集約し、
  `static HOOK_STATE: HookState`1つへ縮小（`HOOK_HANDLE`は設計通り変更なし、
  `hook.rs`のtop-level static宣言は2つに）。
- 実装手順: (1) 全20フィールドの型・doc・呼び出し箇所を事前に読了、(2) 新struct+`const fn
  new()`+`static HOOK_STATE`を1箇所に新設、(3) 元の18個の個別`static`宣言を削除、
  (4) Pythonスクリプトで識別子境界（`\bIDENT\b`）を`HOOK_STATE.<field>`へ機械置換
  ——**この段階では識別子の前後のトークン（`Ordering::`引数含む）には一切触れないため、
  ordering保存は構造的に保証される**（"1対1一致の確認"は正しさの検証であり、置換操作
  自体が改変不可能な設計）。
- **memory ordering保存の確認（完了条件）**: 置換後も`rg -o 'Ordering::\w+' hook.rs | sort |
  uniq -c`のヒストグラムが移行前と完全一致（Relaxed 49・Release 9・Acquire 7・SeqCst 1）。
  加えて全19フィールドについて識別子ごとに`Ordering::`引数の集合を突き合わせ、
  `HOOK_TID_INIT_SLOT`（現`hook_tid_init_slot`）の`SeqCst`/`Release`/`Acquire`の3値使い分けと
  `FOCUS_APP_DISABLED`（現`focus_app_disabled`）の書き`Release`・アクセサ読み`Acquire`・
  ホットパス読み`Relaxed`という非対称が変化していないことを確認済み。
- ログ文言修正: 機械置換により`tracing::info!`のログメッセージ文字列2箇所
  （`reset_physical_key_state`/`clear_hook_latches_for_app_disable`）に内部フィールドパス
  `HOOK_STATE.physical_key_state`が意図せず混入したため、人間可読な元の表記
  （`PHYSICAL_KEY_STATE`）に手動で戻した（コード識別子ではなくログ文言のみの修正）。
- `tests/architecture_guard.rs`の2テスト更新（実装前調査で予告済みの既知の壊れ）:
  `disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana`
  のneedleを`HOOK_STATE.focus_app_disabled.load(Ordering::Relaxed)`に更新。
  `cross_thread_shared_lock_declarations_are_accounted_for`から`src/hook.rs`を削除
  （裸のtop-level `static X: Mutex<`が無くなったため、正しい検出結果）。検出力の欠落を
  埋めるため、新規テスト`hook_state_struct_has_exactly_one_mutex_field`を追加し、
  `HookState`構造体内の`Mutex<`が引き続き1件のみであることを固定した。
- 検証: `cargo check`（host/windows両ターゲット、`--tests --lib`含む）/clippy/fmt/
  `cargo nextest run -p awase-windows --test architecture_guard --test layer_boundary_guard
  --test golden_scenarios`（123/123 passed）/`cargo test --lib -p awase`（1014 passed）/
  `cargo test --lib -p awase-windows`（668 passed、host targetでコンパイル可能な範囲）を
  実行者側で確認済み。
- **windows-build CI・実機ソークは未実施** — フェーズ4は本ADR全体で最高リスクのため、
  develop mergeにはこれらに加え、複数アプリ種別（Win32/TSF-native/UWP）を跨いだ
  拡張ソーク（物理キー状態追跡・IMEモードキー・Alt なりすまし・disable_apps）が必要
  （検証方法節4参照）。

### フェーズ5: `probe_actuation_fence.rs`の5件をsingleton集約（実装済み、develop未マージ・実機ソーク未実施）

**round1 M7で当初のリスク評価（「診断/カウンタ用途でactuationの判定ロジックには使われない」）が
誤りと判明、訂正。**

`PROBE_ACTUATION_FENCE`（`probe_actuation_fence.rs:117`）はカウンタではなく**フェンス**で、
`ime.rs:906,910`/`platform.rs:783,798`/`runtime/key_pipeline.rs:698,903`/
`output/probe_io.rs:239,251,516,530`が読んで`FencedProbeOutcome::Abandoned`
（`probe_actuation_fence.rs:146-152`）を決め、resync gateの扱いを変える——**actuation判定
ロジックそのもの**。診断なのは4件の`LifetimeCounter`（`ABANDONED_RESYNC`/`ABANDONED_NORMAL`/
`SPAWNED_RESYNC`/`SPAWNED_NORMAL`）のみで、5件中1件は判定ロジック本体。`bump()`の呼び出し元
（`win32.rs:278`/`imm.rs:154`）はどちらも`lints/actuation_call_guard/src/lib.rs`の
`RESTRICTED_CALLS`チョークポイント（`send_input_safe`/`send_ime_control`）であり、
`.claude/rules/fix-requires-evidence.md`の「IME actuation 合流点（ADR-119）」再発ファミリーに直撃する。
`current()`は`output/probe_io.rs`の`run_with_timeout`ワーカースレッドからも読まれる。

5件を1つの`PROBE_FENCE`singleton構造体のフィールドに統合する（フィールドの意味論は変えない）。
**hook.rsと同じ理由でMutex禁止、ロックフリーなstruct-of-atomicsのみ**（ワーカースレッドからの
読み取りがあるため）。

- 規模: 小。読み書き箇所を1つの構造体アクセスに置き換えるだけ。
- リスク: **中**（当初「低」から訂正）。actuation判定ロジックに触れるため、
  `.claude/rules/fix-requires-evidence.md`の(a)(b)義務（golden/architecture_guardでの回帰テスト、
  またはknown-bugs.mdへの記録）を満たすこと。
- **実装（2026-09-10）**: ブランチ`refactor/adr164-phase5-probe-actuation-fence`。
  `PROBE_ACTUATION_FENCE`（`AtomicU64`、フェンス値）と`ABANDONED_RESYNC`/
  `ABANDONED_NORMAL`/`SPAWNED_RESYNC`/`SPAWNED_NORMAL`（`LifetimeCounter`）の5裸static
  を`ProbeFence`構造体（`fence_value`+4フィールド、全て変更前と同じ`Ordering::Relaxed`）
  に集約し、`static PROBE_FENCE: ProbeFence`1つに縮小。`bump()`/`current()`/
  `record_abandoned()`/`record_spawned()`/4つの`*_lifetime_count()`アクセサの
  シグネチャ・ロジックは無変更。(a)充足の根拠: 本ファイル内の既存
  `#[cfg(test)]`ユニットテスト3件（`bump_advances_current_monotonically`等）が
  変更後もWindowsターゲットで`cargo check --tests --lib`コンパイル確認済み
  （このファイルは`#[cfg(windows)]`ゲート下のためLinuxでは実行不可、CLAUDE.md
  参照）。**windows-build CIでのテスト実行、および実機ソーク（検証方法節4）は
  未実施** — フェーズ4と同様、develop マージ前に必要。

### フェーズ6: `lib.rs`の3件（`MAIN_THREAD_ID`/`QUIT_REQUESTED`/`ELEVATED`）をsingleton集約（実装済み、develop未マージ・実機ソーク未実施）

**round1 M5で棚卸しの誤り（`ELEVATED`の記載漏れ、`RAW_TSF_LITERAL`は既に対応済みと誤認識、
リスク機序の誤り）を訂正。**

実測5件: `MAIN_THREAD_ID`(:123)/`QUIT_REQUESTED`(:132)/`ELEVATED`(:141)/`RAW_TSF_LITERAL`(:200)/
`RUNTIME`(:204)。このうち`RAW_TSF_LITERAL`は既に`RawTsfLiteralPending`という3フィールドの
singleton構造体（`lib.rs:155-199`）であり、**ADR自身の原則を既に満たしている**（対応不要）。
`RUNTIME`は既存の確立済み集約点（対応不要）。実際のフェーズ6スコープは
`MAIN_THREAD_ID`＋`QUIT_REQUESTED`＋`ELEVATED`の3件のみ。

これら3件はCtrl+Cハンドラ（別スレッド）等からアクセスされる、hook.rsと同型の**(A2)クロス
スレッド共有state**であり、1つのロックフリーsingleton構造体（例: `PROCESS_FLAGS`）に統合する。

リスク機序の訂正: 当初「`RUNTIME`排他ロック中のアクセスパターンを壊すと再入デッドロック」と
記載していたが誤り。`with_app`（`lib.rs:213`）は`try_borrow_mut()`を使い、再入時は
`tracing::warn!`を出して`None`を返す（doc に「UBなし」と明記）。実際の失敗モードは
**デッドロックではなくメッセージのサイレント消失**であり、そのために`with_app_or_repost`が
存在する。本フェーズの3件はこの`with_app`経路とは別のクロススレッド共有stateであり、
`with_app`の再入自体には関与しない——ただし新設するsingletonの読み書きが`with_app`クロージャの
外（Ctrl+Cハンドラ等）から行われることに変わりはなく、その設計を維持すること。

- 規模: 小。
- リスク: 中。Ctrl+Cハンドラでの動作（プロセス終了シーケンス）に触れるため、変更後は
  Ctrl+C動作の手動確認（実機）を行う。
- **実装（2026-09-12）**: ブランチ`refactor/adr164-phase6-lib-process-flags`。
  `MAIN_THREAD_ID`（`AtomicU32`）/`QUIT_REQUESTED`/`ELEVATED`（`AtomicBool`）の
  3裸staticを`ProcessFlags`構造体に集約し、`static PROCESS_FLAGS: ProcessFlags`
  1つに縮小。フィールドごとのOrdering（`main_thread_id`/`quit_requested`は
  `SeqCst`、`elevated`は`Relaxed`）は集約前と完全一致で変更なし。
  `main_thread_id()`/`is_quit_requested()`/`is_elevated()`/`set_main_thread_id()`/
  `request_quit()`/`set_elevated()`のシグネチャ・ロジックは無変更（呼び出し元は
  全てこれらの関数経由で、生staticへの直接参照は`lib.rs`外に無いことを
  `grep`で確認済み）。`cargo check`（host/windows両ターゲット、`--tests --lib`
  含む）/clippy/fmt/`cargo nextest run -p awase-windows --test architecture_guard
  --test layer_boundary_guard --test golden_scenarios`（122件）は通過済み。
  **Ctrl+Cハンドラの実機動作確認・windows-build CIは未実施**——develop merge前に
  必要（検証方法節4参照）。

### フェーズ7（廃止・対象外へ変更）: `awase-settings/src/main.rs`の本体2件

**2026-09-10、実装着手時の再分類でADR起票時の誤りを訂正——「1つの構造体にまとめる」を
見送り対象外とした。**

`#[cfg(test)]`専用の`COUNTER`類2件は元々対象外（テストのみに存在し本体スメルではない）。
残る`SETTINGS_LOG_FILE`（`OnceLock<Arc<Mutex<File>>>`、ファイル先頭・全プラットフォーム
共通、`init_logging`/`log_checkpoint`が使う）と`SIMULATED_REGISTERED`（`AtomicBool`、
`#[cfg(not(target_os = "windows"))] mod autostart_bridge`内のLinux専用autostart登録
シミュレーション）は、**同じファイルに書かれているという以外に共通点が無い**——別モジュール
（crateルート vs `autostart_bridge`サブモジュール）、別cfgゲート（無条件 vs 非Windows限定）、
別の並行性ドメイン（tracingのwriterはログ発生元の任意スレッドから呼ばれうる vs 設定GUIの
チェックボックス操作はGUIスレッド限定）、別の目的（ログ基盤 vs Windows Runキー操作の開発用
スタブ）。「原則の確定」が定める単位は「1つの独立した並行性ドメインにつき1つ」であり
「1ファイルにつき1つ」ではない（round1 M3の訂正と同じ理由）。この2件は元々それぞれが
自分のモジュール/cfgスコープで単独の静的であり、追加原則が問題視する「裸staticが複数並ぶ」
状態には該当しない。

ADR起票時のファイル別分類表がこの区別をせず「同一ファイルだから」で2件をまとめて1行に
記載していたこと自体が誤りだった。両者を無理に1つのstructへ押し込めると、`SIMULATED_REGISTERED`
フィールドだけが`#[cfg(not(target_os = "windows"))]`という条件付きフィールドになり、
無関係な概念を型レベルで結合するだけでスメルの実質的な解消にはならない。

**結論**: `SETTINGS_LOG_FILE`・`SIMULATED_REGISTERED`とも現状維持。本ADRの対象から外す。

### フェーズ8: `state/probe_admission.rs`の3件をsingleton集約（実装済み、develop未マージ）

プロセス生存期間の棄却統計カウンタ、`drain_stats()`で消費するのみで判定ロジックには
使われない。3件を1つの構造体にまとめる。

- 規模: 極小。
- リスク: 低。
- **実装（2026-09-10）**: ブランチ`refactor/adr164-phase8-probe-admission-counters`。
  `REJECTED_EPOCH_MISMATCH`/`REJECTED_HWND_MISMATCH_SAME_ROOT`/
  `REJECTED_HWND_MISMATCH_CROSS_ROOT`の3裸staticを`RejectionCounters`構造体
  （フィールドは変更前と同じ`LifetimeCounter`型、全て`Ordering::Relaxed`）に集約し、
  `static REJECTION_COUNTERS: RejectionCounters`1つに縮小。呼び出し元
  （`admit()`・`record_hwnd_mismatch()`・`drain_stats()`）のロジック・戻り値は無変更。

### フェーズ9: `hook_channel.rs`（3件）・`runtime/engine_window.rs`（3件）をsingleton集約

**round1 N2を受け、「要精査」から実測に基づく確定スコープへ変更。**

- `hook_channel.rs`: `HOOK_KEYS`(:184)は既に`HookKeyRing`構造体で原則を満たす（対応不要）。
  `WAKE_PENDING`(:185)/`WAKE_POST_FAILED`(:191)/`WAKE_POST_FAILED_LIFETIME_COUNT`(:198)の
  3件が裸のまま並んでいる。これらを1つのsingleton構造体に統合する（`HOOK_KEYS`とは
  別、`HookKeyRing`自体の内部実装は変更しない）。
- `runtime/engine_window.rs`: `ENGINE_HWND`/`MODAL_DEPTH`/`NEEDS_ENGINE_RESYNC`(:14-16)が
  裸のまま並んでいる。`engine_wnd_proc`（`WNDPROC`固定署名）およびネストしたモーダル
  ポンプからの再入で使われるため、hook.rsと同種の設計判断（引数引き回し可否の検証、
  Mutex可否の検証）が必要。1つのsingleton構造体に統合する。

優先度は低い（フェーズ1〜8と異なり、これら2ファイルは今回調査で複数staticの存在は
確認済みだが、各フィールドの呼び出し経路の網羅的な洗い出しはまだ行っていない）。
着手前に、フェーズ4と同様「引数引き回し可能か／クロススレッド共有か」の判定を
実コードで行うこと。

### 対象外（変更しない）

- 分類(C-immutable)の全ファイル（`ime_controller.rs`／`state/ime_profile_driver.rs`の
  dynディスパッチテーブル、`tsf/output.rs::TABLE`等の不変静的テーブル）——可変状態を
  持たないため本ADRのスメルの対象外。
- `focus/classifier.rs::INPUT_RELAY_APPS`——CLAUDE.mdが明記する唯一の意図的な例外。
- `hook.rs::HOOK_HANDLE`・`lib.rs::RUNTIME`・`lib.rs::RAW_TSF_LITERAL`・
  `hook_channel.rs::HOOK_KEYS`——既に単独1件、または既にsingleton構造体化済みで
  原則を満たしている。
- `tray.rs::MENU_TARGET_HWND`——round2で(B)化を見送り対象外に変更（旧フェーズ3参照）。
  `TrackPopupMenu`のネストモーダルループ境界を跨ぐ受け渡しであり、(A)「モーダル
  ポンプ境界」として現状維持する。
- `hook.rs`の関数ローカルstatic2件（`CACHE`/`BASELINE`）——write-onceキャッシュとして
  既に最小スコープ、親singletonへ引き上げるとスコープが拡大するため対象外。
- `awase-settings/src/main.rs::SETTINGS_LOG_FILE`・`SIMULATED_REGISTERED`——2026-09-10
  実装着手時にフェーズ7を廃止し対象外に変更（旧フェーズ7参照）。別モジュール・別cfg
  ゲート・別並行性ドメインで「同一ファイル」以外の共通点が無く、それぞれ既に自分の
  モジュール/cfgスコープで単独の静的として原則を満たしている。
- 既に1ファイル1staticになっている約20ファイル——追加原則を既に満たしている
  **と推定されるが、round1 N4指摘の通り全件は未検証。実装着手時に上記rgコマンドで
  再確認すること**。

### 保留（次回調査で確定してから判断）

- `runtime/message_handlers.rs::DRAIN_PENDING`/`DRAIN_RERUN_PENDING` — `tray_wnd_proc`からの
  呼ばれ方を確認してからA/Bを確定する。
- `tsf/tip_detector.rs`の2件 — 専用COM STAスレッド設計の中で完結しており優先度低。
- `awase-linux/src/hook.rs`・`awase-macos/src/hook.rs` — プラットフォーム実装の成熟度が
  上がってから判断する。

## 検証方法

`crates/awase-windows`はWindows専用のホットパスが大半であり、このLinuxサンドボックスでは
実行時の動作確認ができない。各フェーズについて:

1. `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`でコンパイル確認。
2. `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`で
   `#[cfg(windows)]`ゲート下の`#[cfg(test)]`ツリーも含めてコンパイル確認する
   （round1 N6——素の`cargo check`はこのツリーを一切コンパイルせず、エラーもスキップ
   メッセージも出ない。CLAUDE.md参照）。
3. Linux側で実行可能なgolden/architecture_guard/layer_boundary_guard/`src/engine/tests.rs`
   （`cargo test --lib`）テストを確認する。
4. フェーズ4・5・6・9はWindows実機（`clipwire-exec`等）でのソークを経てからマージする
   （round2 N3——フェーズ6もCtrl+Cハンドラの動作確認が必要なため追加）。
5. フェーズ4は追加で、`tests/architecture_guard.rs`の
   `disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana`
   と`cross_thread_shared_lock_declarations_are_accounted_for`の2テスト（`hook.rs`の
   ソースコードをリテラル文字列で走査しており、static宣言の形が変わると確実に壊れる）を
   新しいコード形に合わせて更新することを完了条件に含める（2026-09-10実装前調査で判明、
   フェーズ4本文の「実装前調査」節参照）。

## 非スコープ

- 本ADRは「グローバルstatic個数の予算制」（ADR-158複雑性予算制のような1-in-1-out規約）を
  新設するものではない。今回のフェーズ1・2・4〜9で対象範囲を実施しきることを目標とする
  （旧フェーズ3は対象外へ変更、上記参照）。
  将来これを機械化する（ファイルごと/並行性ドメインごとのtop-level static数を数えるlint等）
  場合は、第二の並列予算制を新設せず`.claude/rules/complexity-budget.md`（ADR-162 E1）の下に編入し、
  「宣言の強制とSSOT化」の既存原則に揃えること（round1 N5）——ADR-162が問題視した
  「ガバナンスが加算のみを義務化し減算に報酬がない」非対称性を再生産しないため。
- ADR-159（記録・再生基盤）の完成を待たない。フェーズ1・2・4〜9はADR-159の記録・再生基盤の
  完成を待たず、既存のgolden/`src/engine/tests.rs`と（フェーズ4・5・6・9は追加で）実機
  ソークで検証して独立に進める（round3 M3——「golden回帰テストのみで十分」という限定句は
  実機ソーク必須のフェーズ5・6・9の記述〈検証方法節・フェーズ6本文〉と矛盾していたため
  削除）。フェーズ4（hook.rs本体）はopus-adversarial-consult round1・round2を経て設計が
  確定済み。
- (C-immutable)分類の不変ディスパッチテーブルへの集約は行わない（可変状態がなくスメルの
  対象外という判断、上記「原則の確定」参照）。ユーザーがこの判断に同意しない場合は
  再考する。

## 関連

- [ADR-158](158-complexity-reduction-north-star.md) — 本ADRの発端となった複雑性棚卸し。
- `.claude/rules/fix-requires-evidence.md` — フェーズ1・4・5が触れる再発ファミリーのテスト/記録義務。
- `.claude/rules/worktree-per-session.md` — 実装は専用worktree/branchで行う。
