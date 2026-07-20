# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### 修正

- **`layout/nicola.yab`/`nicola_us.yab`: 数字段の左右親指シフトでキー「4」「5」が「6」「7」と重複していた不具合を修正**
  - 初回コミット時から、左右親指シフトどちらの数字段も `4:［ 5:］ 6:［ 7:］` と括弧記号が重複しており、日本語の鉤括弧「」が数字段のどこにも存在しなかった
  - `4:「 5:」`（無変換+4 → 「、変換+5 → 」）に修正し、`6:［ 7:］` はそのまま残した

## [1.10.0] - 2026-07-19

### 追加

- **「配列編集」タブにセルのコピー/貼り付け（履歴方式）を追加**
  - セルを選択して「コピー」を押すと、その値が履歴の先頭に自動的に積まれる（最大4件、面をまたいでも保持）
  - 履歴はボタン列として表示され、クリックするだけで選択中のセルへ直接貼り付けられる（テキスト欄を経由しないため、ローマ字のかな変換結果を含めて元セルと完全に同じ値になる）
  - 同じ値を続けてコピーしても重複せず、履歴の先頭に移動するだけ
- **US配列: 左右 Alt キーを親指キーへなりすませる機能を追加**
  - US配列にはスペース両隣に無変換/変換キーが無いため、左右 Alt キーを `left_thumb_key`/`right_thumb_key` としてなりすまさせる設定（`left_alt_impersonates_thumb_key`/`right_alt_impersonates_thumb_key`、既定 false）を追加。PowerToys 等の外部リマップと同等のことを awase 単体で完結できる
  - エンジン ON 時のみ発動し、OFF 時は通常の Alt として Alt+Tab 等を損なわない。左右は独立に設定可能で、押しっぱなし中はキー押下時点の状態を保持しstuck modifier化を防ぐ
  - US配列選択時のみ設定画面（awase-settings）に表示
- **US配列: Space親指キーのcomposing中フォールバックと Shift+Space 即時送出を追加**
  - Space を親指キーに割り当てた場合、変換候補ウィンドウ表示中でも単独タップで `VK_SPACE`（変換候補送り）を送出できるようにした（無変換/変換と異なり、composing 中の raw VK_SPACE 送出は IME の正規機能のため）
  - Shift+Space は同時打鍵判定を待たずリテラルなスペースとして即時送出する
  - 設定は `space_thumb_ignore_composing_guard`/`space_thumb_shift_literal`（既定 true）。Space が親指キーの時のみ設定画面に表示
- **設定画面: US⇔JIS配列切替時にホットキー既定値を自動リセット/復元**
  - US配列へ切り替えるとJIS前提のホットキー既定値（無変換/変換キー等）を空にリセットし、JISへ戻すとJIS既定値へ復元するようにした

### 修正

- **BUG-31: Microsoft Teams で連続タイピング中に文字が無音で消失する不具合を修正**
  - アプリ: Microsoft Teams（`TeamsWebView`、Chrome系、Vk mode）。IME: Google 日本語入力（GJI）
  - 再現: Enter確定直後、warm状態(`GjiFsm::OnWarm`)であるにも関わらず物理 `NativeF2Down`（非TSF）イベントが無条件に `MarkCold`+`GjiCompositionReset` を発行して warm 維持用タイマーを kill しており、以降の romaji が per-VK confirm レースに巻き込まれて一部消失していた（実機報告: 「５せっしょん」の一部が無音で消失）
  - `NativeF2Down` を warm 中は無視するようガードを追加
- **BUG-30: GJI候補ウィンドウ可視時に正しく入力できた文字を誤って backspace する不具合を修正**
  - 候補ウィンドウの SHOW イベント（edge-triggered）とライブ可視状態（level-triggered）が別センサーで、`SuspectedLiteral` 判定時に可視でも保護がなかった。可視中は backspace を保留する veto を追加し、TSF/Chrome の literal-detect ロジックも統一した
- **Alt親指キーなりすまし: `OsModifierHeld` bypass により NICOLA 同時打鍵判定が一切効かなくなる不具合を修正**
  - vk 書き換え自体は正しく行われていたが、`GetAsyncKeyState` 由来の別経路（`modifiers.alt`）が物理 Alt 押下をそのまま返しており、3箇所で `BypassReason::OsModifierHeld` が誤発動していた。なりすまし中は `modifiers.alt` を補正するよう修正
- **Alt親指キーなりすまし: 環境によって vk が汎用 `VK_MENU` で届き、なりすましが一切発動しない不具合を修正**
  - `WH_KEYBOARD_LL` の vkCode が左右区別済み `VK_LMENU`/`VK_RMENU` ではなく汎用 `VK_MENU` で届く環境があり、直接比較のみの判定が常に false になっていた。`LLKHF_EXTENDED` フラグで左右を判別するよう修正
- **設定画面: 「適用」ボタンが常に無効だったバグを修正**
  - `awase-settings.exe` の設定リロード通知が `FindWindowW` で存在しないウィンドウクラス名（`awase_msg_window`）を探しており、`awase.exe`（`awase_tray_window`）に一度も通知が届いていなかった。正しいクラス名に修正し、失敗時は `log::warn!` で明示的にログを残すようにした
- **設定画面: `config.toml` パス解決が `awase.exe` と非対称だったバグを修正**
  - `awase.exe` はコマンドライン引数で明示パスを指定できるが `awase-settings.exe` はこれを無視しており、異なる `config.toml` を編集してしまう恐れがあった。優先順位を揃え、編集中のパスを画面上部に常時表示する機能も追加した
- **設定画面: 長い検証警告文がボタン行に切られて表示されない問題を修正**

### 変更

- **GJI/Chrome の cold-start warmup アーキテクチャを大幅に簡素化**
  - 数日間の実機ソーク（cold=60件超、suspected literal ゼロ件）で無破損を確認できたため、待機行列（`WarmupKind::FreshF2`/`ReWarmup`/`ProbeWithSettle` 等の `ColdReason`×`long_idle` 行列）と捨て駒キー機構（`SacrificialWarmupCoro`/`ImeOffOnWarmupFsm`）を物理削除し、romaji を1文字ずつ送って確認する per-VK confirm 方式に一本化した
  - Chrome（`probe_fsm.rs`）と TSF（`gji_warmup_coro.rs`）で重複していた per-VK confirm ループを共通実装に統合（正味 -82行）
  - Chrome の VK 送信への scan code 付与を実験仕様から恒久仕様へ確定
  - 上記を支える基盤として、内部依存クレート `timed-fsm` に `StepCoro::prime()`（ダミー入力なしの self-priming）を追加

## [1.9.1] - 2026-07-12

### 追加

- **設定画面に「配列編集」タブを追加（.yab をその場で編集可能に）**
  - 独立バイナリだった `awase-yab-editor` を `awase-settings` に統合し、削除した（コードを再利用する価値はあるが、別バイナリに分ける価値は無いという判断。CI/配布物/インストーラで2バイナリを同期し続けるコストの方が実利を上回っていた）
  - キーボード風グリッドでセルをクリックして選択 → 種別（打鍵/リテラル/特殊キー/なし）を選んで編集 → 適用、のフローで4面（通常/左親指シフト/右親指シフト/小指シフト）すべてを編集可能
  - 「ローマ字」「キーシーケンス」だった種別を「打鍵」に統合し、入力（アルファベットかどうか）で自動判定するように。JIS キーボード上に存在しないキーを入力した場合はリアルタイムでエラー表示し、適用ボタンも無効化する
  - 全角で記号・英数字を入力しても自動で半角に変換されるため、入力側が半角/全角を意識する必要が無い
  - 開く/保存/名前を付けて保存/再読み込み、Ctrl+S/Ctrl+O/Ctrl+Shift+S/F5 のショートカットに対応

### 修正

- **設定画面「配列編集」タブ（旧プレビュータブ）が表示されない/クラッシュする問題を修正**
  - `layouts_dir`/`config.toml` のパス解決を `awase::paths::resolve_relative_to_exe` に一本化し、`cargo run`/`cargo build` で `target/debug/` 配下から起動した場合にワークスペースルート直下の `layout/nicola.yab` を見つけられなかったバグを修正
  - 2026-07-06（`c3fa08e`）に「配列プレビューの実装が固まっていない」として非表示にしていたタブを再表示
  - タブを開くと無言のまま強制終了する不具合を修正（`egui::Grid` が行内 `add_space()` によるカーソル移動を許可しないことが原因。上記2点に阻まれて実データで描画されたことが一度も無く、今まで誰も踏んでいなかった潜在バグだった）
  - `awase-settings` にログファイル出力（`awase-settings.log`）+ panic フックを追加。GUI サブシステムでコンソールが無いため、今まで panic してもログに一切残らなかった
  - タブを開いたときに `.yab` を毎回同期的に読み込んでいたためウィンドウ生成〜最初の描画までの間が延びていた問題を修正し、遅延読み込みに変更

## [1.9.0] - 2026-07-11

### 追加

- **左Shift単独タップによる「IME-ON 半角英数」持続トグル（BUG-25、MS-IME対応）**
  - 左Shiftキーの単独タップ（他キーを介さない押下→解放）で、IME を開いたまま半角英数入力へ切り替える持続トグルを追加。もう一度単独タップすると通常のかな入力へ復帰する
  - BUG-15 の「Shift 押しっぱなし中は半角英数」（hold 方式）を置き換えた。Shift+文字キーのチョード（`.yab` Shift 面）で MS-IME の Shift 単独タップ誤検知が発火する問題を打ち消す既存の安全網は維持しつつ、hold 方式固有の ASCII パススルー層（`shift_plane_halfwidth`）のみを撤去した
  - 右Shift単独タップは常に安全網の復元のみを実行し、持続トグル中に押すと「緊急解除」としても働く
  - GJI（Google 日本語入力）向けの entry 機構は scan 付き VK 注入・IMC write・scan=0 VK 注入の3通りを実機検証したがいずれも機能せず（CapsLock 汚染、または `SendInput` がフックにすら到達しない）、GJI では機能を無効化した（MS-IME のみ対応）。詳細は docs/known-bugs.md BUG-25、docs/experiments.md エントリ06〜09
- **Ctrl+変換を IME ON 中に押すと、ひらがな＋ローマ字＋CapsLock OFF へリセットするように変更**
  - 従来は IME OFF→ON にのみ反応していたが、IME が既に ON の状態で Ctrl+変換を押した場合も、入力モードをひらがな・ローマ字・CapsLock OFF へ揃えて再確定させるようにした
  - Ctrl+Shift+変換（EngineOn combo）等、無関係な IME-ON 経路を誤検出しないよう、Ctrl 単独修飾かつ VK_CONVERT 単押しの場合のみ対象とするガードを追加

## [1.8.9] - 2026-07-11

### 追加

- **US (ANSI 104) キーボード対応**
  - `crates/awase-windows/src/scanmap.rs` に US 配列専用のスキャンコード⇔物理位置テーブル（`scan_to_pos_us`/`pos_to_scan_us`）を追加し、`KeyboardModel::{Jis,Us}` に応じて使い分けるよう配線
  - `general.keyboard_model`（"jis"/"us"）設定を再導入。2026-07-06 に「一度も配線されなかった」として撤去された同名フィールドを、今度は `.yab` パース・`HookConfig`（フックの scan_to_pos テーブル選択）まで実際に配線した上で復活させた
  - `KeyboardModel::Us.row_sizes()` の row0 を 13→12 に修正（グレイブキーは物理キー非対応としてグリッド外扱い。旧値は一度も実行されなかった未検証コードの誤り）
  - US 配列用レイアウト `layout/nicola_us.yab` を追加（`layout/nicola.yab` から JIS 専用キーの列を除去したもの。共有するスキャンコードの値はそのまま流用）
  - `keyboard_model = "us"` かつ 無変換/変換キー前提の既定値（`left_thumb_key`/`right_thumb_key`/`[keys]` の各ホットキー）が残っている場合、起動時検証で警告するよう `AppConfig::validate` を拡張
  - 設定画面（awase-settings）に JIS/US 選択・配列プレビューの追随・親指キー候補を追加
  - Linux/macOS 側は今回未対応（`KeyboardModel::Jis` 固定のまま）

- **親指キーに Ctrl/Alt/Win を割り当てても機能しないことが判明（`engine/tests.rs`）**
  - US 配列向けに `VK_LMENU`/`VK_RMENU`（Alt）を親指キーの代替として一度推奨したが、
    実際にはそのキーの KeyDown 自体が `ModifierState::is_os_modifier_held()` →
    `bypass_reason` の `OsModifierHeld` に即座に該当し、`PendingThumb` に一切入らず
    素通しされる（同時打鍵検出そのものが機能しない）ことが判明。Alt に限らず
    Ctrl/Win も同様に不可
  - `test_ctrl_alt_win_thumb_key_never_enters_pending_due_to_os_modifier_bypass` で
    この制約を固定。Shift は `is_os_modifier_held()` の対象外のため `PendingThumb`
    には到達できる（`test_thumb_alone_timeout_suppressed_when_thumb_is_os_modifier`
    で単独タップの suppress を確認済み）が、Shift 面機能との相互作用は未検証
  - 併せて `timeout_pending_thumb` に、親指キーが OS 修飾キーの場合は composing に
    関わらず単独タップの生 VK 送出を suppress する防御を追加（`ClassifiedEvent`/
    `PendingThumbData` に `modifier_key` を伝播）
  - config.rs / config.toml.sample / awase-settings の推奨から Alt を撤回し、
    F13-F24（プログラマブルキーボードでの物理リマップ前提）/ VK_SPACE を代替として案内

- **無変換ソロ連打による緊急 Engine OFF が誤発動しやすかったのを修正**
  - スリープ復帰直後の conv mode 誤観測（カタカナ固定）から復旧しようと無変換キーを連打しただけで、Ctrl スタック時の緊急脱出用に用意していた「無変換単独3連打でエンジン OFF」機構（ADR-055）が誤発動し、`user_enabled` が false になる実機事例が発生（2026-07-08）。一度発動すると `Ctrl+変換`（`ime_on`）等の通常のキー操作では `user_enabled` が戻らず、「何を押しても直らない」状態になっていた
  - 必要連打回数を 3 → 5 に引き上げ（`SOLO_OFF_TRIGGER_COUNT`, `src/engine/nicola_fsm.rs`）、誤発動しにくくした
  - 発動時にトレイ通知を出すようにし、ユーザーが「engine が緊急停止したこと」と「`Ctrl+Shift+変換` で復帰できること」をその場で把握できるようにした。詳細は `docs/adr/055-engine-off-solo-triple.md` の追補

- **Chrome/Edge で conv mode の一発誤読が GJI を実際にカタカナへ固定する不具合を修正（BUG-19）**
  - `GetForegroundWindow()` 基準の conv 読み取り（`get_ime_conversion_mode_raw_timeout`）が、`Chrome_WidgetWin_1` と GJI 候補ポップアップ（`Windows.UI.Input.InputSite.WindowClass`）の間でフォーカスが往復する際に一瞬だけ誤ったカタカナ conv を拾うことがあり、これを `ConvModeMgr` が無条件に確定していた
  - 確定した誤読を eager warmup（`send_eager_tsf_warmup`）が鵜呑みにし、`VK_DBE_KATAKANA` を実送信 → 一過性の誤読が GJI の本当の状態としてロックインされ、以後の入力が全部カタカナ化し、さらに `KatakanaShadowOff` 救済ロジックが繰り返し IME OFF/ON を往復させて先頭文字の literal 漏れを誘発していた
  - `ConvModeMgr::update_from_conv` に、非カタカナ→カタカナ遷移限定のデバウンス（`ImeKindDebounce` と同一の「2 tick 連続確認」パターン）を追加
  - 追補: 上記は warmup 側（`ConvModeMgr`）しか保護しておらず、`classify_conv_transition`（belief 更新・`KatakanaShadowOff` 等の engine 同期）は raw conv を直接再解釈しており同じ誤読に無防備だった。`classify_conv_transition` の引数を `conv: u32` から `ConvModeMgr::get()` 由来の `ConvMode` に変更し、warmup と belief/engine-sync が同一の確定値を参照するよう統一した。詳細は docs/known-bugs.md BUG-19
  - 追補2: デバウンス済み状態でも、ユーザーが IME を明示的に OFF にした直後に conv の誤読で `KatakanaShadowOff`/`NativeToggleShadowOff` が発火すると、`UserImeSetIntent{Command}` を偽装して `desired_open` を直接書き換えてしまい、engine が勝手に ON へ戻る別経路の再発があった。これを `EngineSync::ReportOpenInference` に分離し、`desired_open` を書き換えず `ObserverReported`（`ObservationSource::ConvOpenInference`, Medium confidence）として記録するだけに変更。実際の補正判断は既存の drift correction（BUG-20 で OFF 方向も修正済み）に委譲する。明示的なユーザー意図が一度も無い間はこの観測単独で補正を発火させない source-aware gate も追加した

- **Chrome 入力中、CLSID ベース IME 種別の単発誤検出で `GjiFsm` が単語ごとに再構築され `cold` が発火し続ける不具合を修正（BUG-17）**
  - `gji-io-monitor` ワーカースレッドが 2 秒ごとにポーリングする `ITfInputProcessorProfileMgr::GetActiveProfile` の単発フリップを `WM_IME_KIND_CHANGED` としてそのまま main スレッドへ伝播しており、`set_active_ime_kind` が種別変化のたびに warmup 戦略（`GjiFsm`/`MsImeStrategy`）を無条件で新規生成 → 確立済みの `OnWarm`/`OnComposing` を破棄していた
  - Chrome cold-start reinit が実 `VK_IME_OFF→VK_IME_ON` トグルを送信すること自体が誤検出の引き金となり、「reinit → 誤検出 → GjiFsm 再構築 → 次の単語も cold → 再度 reinit → …」という自己増幅ループを形成していた（実機ログで `cold_seq` が単語ごとに 392→401 と climb、2 回の "StartComposition while engine off" 警告が CLSID ポーリング周期とほぼ一致する 2146ms 間隔で観測）
  - `tsf/gji_monitor.rs` に `ImeKindDebounce` を追加し、同じ新種別が 2 tick 連続で観測されるまで確定させないようにした。詳細は docs/known-bugs.md BUG-17
  - 追補3: デバウンス済みでも、`cold_warmup.rs::preamble()`（cold warmup のたびに実行）が `conv_mode.get()` を無条件に real IME へ書き戻していたため、一度誤って確定した belief がフォーカス往復のたびに再アサートされ続けて自己増幅する経路が残っていた。`ConvModeMgr::needs_conv_restore_write`/`mark_conv_restore_written` を追加し、同じ確定 mode への復元書き込みを1回だけに制限した
  - 追補4: 上記スロットルは `cold_warmup.rs`/`probe_io.rs` の IMM32 書き戻し経路にしか配線されておらず、根本原因分析自体が名指ししていた `output/mod.rs::send_eager_tsf_warmup`（確定キーのたびに高頻度で発火する eager warmup の charset 選択）だけが無防備なまま残っていた。ここにも同じスロットルを適用し、確定済み mode への charset-changing warmup キー（F1/F0 系）送信を1回に制限した
  - 追補5: ユーザーは IME トレイからカタカナ/半角英数を手動選択したことが一度もなく今後もその予定がないと確認したため、`DIAG_FORCE_HIRAGANA_CHARSET` 診断フラグを新設し、charset 追従ロジック（F1/F0 warmup・IMM32 conv_target 書き戻し・F1 leading warmup 前置）を丸ごと無効化して常に Hiragana 扱いにする実験を開始した（観測・ログ自体は継続、行動への反映だけを止める）

- **IME を ON にした直後の最初の1文字で、正しく変換されているのに不要な BS 訂正が発生する不具合を修正（BUG-24）**
  - `is_partial_literal()`（`tsf/warmup/literal_detect_fsm.rs`）は、今回送った romaji 自身の確認信号（候補ウィンドウ SHOW / GJI I/O 変化）ではなく、送信前に確定していた無関係な代理指標 `nc_fired`/`gji_resumed`（別の F2 warmup キーへの応答有無）で部分リテラルを判定していた。`ColdReason::requires_settle()`（`FocusChange`/`NativeF2Consumed`/`SetOpenTrue` — IME が既に ON の状態でも発生しうる）直後は、`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`（cold-start の予防的 warmup を丸ごと無効化し reactive 検出のみに委ねる診断フラグ）下でこの代理指標の元になる確認送信自体が無条件でスキップされるため、`nc_fired` が構造的に常に `false` になり、実機で確実に再現していた
  - `composition_fsm.rs::ConfirmKeyDown` と `platform.rs::on_reinject_key` が、warm な確定キー（Enter/Space/Escape）でも無条件に `MarkCold`/`GjiCompositionReset` を発行していた副作用も特定・修正（連続 typing 中の余分な cold 化が literal-detect の露出機会を増やしていた）
  - IME セッション（打鍵開始〜候補ウィンドウ HIDE）内で literal-detect を1回確認できたら、以降はセッション終了まで検出処理自体をスキップする仕組み（`tsf/observer.rs` の `literal_session_confirmed` 系、`DIAG_LITERAL_SESSION_SKIP`）を追加し、反応速度を落とさないようにした
  - 上記だけではセッション最初の1文字自体（`is_partial_literal()` を初めて通る文字）は直っていなかったため、最終的にセッション最初の1文字に限り romaji の VK を1つずつ送信し、送信した VK 自身への `CompositionConfirmed`/`SuspectedLiteral` を確認してから次の VK を送る設計（`ProbeAction::TransmitSingleVk`、`gji_warmup_coro.rs`）に変更した。2つの VK をまとめて送るために生じていた「どちらの VK の効果か区別できない」問題は、VK 送信の間に意図的な確認ポイントを挟むことで構造的に解消した。実機で症状の解消を確認済み
  - 詳細は docs/known-bugs.md BUG-24

## [1.8.6] - 2026-07-07

### バグ修正

- **Teams（TeamsWebView）が IMM32 クロスプロセス制御対象のまま誤分類されていたのを修正**
  - Microsoft Teams のメインウィンドウクラス `TeamsWebView` が `IMM32_UNAVAILABLE_CLASSES` に未掲載で `Standard` 分類のまま扱われ、Chrome 系と同じ Chromium ベースにもかかわらず信頼できない IMM32 open status 読み取りが試みられていた。`Chrome_*` と同様に `Imm32Unavailable` / `TsfNative` に分類するよう追加
  - あわせて `kp_stage_focus_probe` の完了処理に `sanitize_focus_probe_open_status()` を追加し、apply 時点の `current_app_profile` が IMM32 open status 非対応（Imm32Unavailable/TsfNative）なら `probe.ime_on` を強制的に破棄するよう防御を追加。分類が後から変わるアプリでも stale な `probe.ime_on` を信用しなくなる

### 変更

- **実測学習した IMM32 能力をフォーカスプロファイル判定に配線**（到達不能パス監査 B5）
  - `ImmCapabilityStore`（`ImmGetDefaultIMEWnd`=NULL 検出・IME 検出ミス閾値超えから学習し cache.toml に永続化）は従来、学習・保存するだけで挙動を変える消費者が存在しなかった
  - フォーカス更新時、静的分類が `Standard` かつ学習値が `Unavailable` のクラスを `Imm32Unavailable` に降格するようにした（静的リスト未掲載の IMM-broken アプリで無駄な `SendMessageTimeoutW` を踏まなくなる）
  - 昇格方向は行わない（静的な Imm32 不可 / TSF ネイティブ知識を優先）。`Works` 回復学習で降格は自己解除。降格は `[imm-learning] profile 降格` の INFO ログで監査可能、誤学習は cache.toml の `[imm_capability]` を手編集で解除
  - 注意: 既存の cache.toml に蓄積済みの学習エントリが即座に有効になる。想定外の降格が起きたらログを確認すること
- **到達不能パス監査（2026-07-06、5 観点並列）に基づくクリーンアップ**
  - 孤児 WM ハンドラ 2 件（`WM_PROCESS_DEFERRED` / `WM_IME_KEY_DETECTED`）、構築サイトゼロの enum variant 群（`ImeEffect::RequestRefresh`、`ActivationState::Pending`+`PendingReason`、`InactiveReason::NonTextFocus`、`DecisionOrigin`、`DetectionSource::UiaAsync`、`ConvModeAuthority::TemporarilyUnowned`、`ColdReason::SessionExpired`、`ChordKind::CtrlHenkanImeOn`+`ImeEvent::ChordStarted`）を撤去
  - 空振りしていた event-driven wakeup 機構（`win32-async::AtomicWatcher` + `notify_all`、write-only の `composition_probe` カウンタ）を撤去（待機はポーリング方式に置換済みだった）
  - 恒偽条件の簡約（`used_eager_path`）、恒真化していた `EffectOrigin`/`SetOpen.origin` の畳み込み、dead write（`GjiState::OnCold.saw_native_f2`）の除去
  - いずれもリポジトリ全体 grep と証拠鎖の個別検証で「実行時に存在しない値/通らない経路」を確認済み。挙動変更なし

### バグ修正

- **MS-IME cold start — IME ON 直後の先頭文字リテラル化を修正（BUG-13、「を」→「wお」）**
  - `MsImeStrategy` は「MS-IME は常にウォーム」前提で cold-start 保護がなく、IME OFF→ON 遷移直後 ~130-300ms（実測）の送信で先頭 VK がリテラル化していた
  - 固定待ちではなく GJI probe と同型の confirm-then-transmit を導入: `ImeModeFsm` が NATIVE 未確認のとき romaji を `MsImeReadyCoro` に defer し、`IMC_GETCONVERSIONMODE` ポーリング（10ms 間隔）で NATIVE ビットを確認した瞬間に送信
  - `MS_IME_READY_CONFIRM_MS` (400ms) は安全弁のみ（IMC が読めない環境では強制送信 + give-up latch で毎キー probe 化を防止）
  - 詳細は docs/known-bugs.md BUG-13

### 追加

- **MS-IME キー割り当ての競合を検出して解除を案内**
  - MS-IME の「無変換キー=IME-オフ」「変換キー=IME-オン」割り当てが有効だと、awase が素通しする無変換/変換の単独タップで OS 側だけ IME 状態が反転し belief と乖離する（実機 2026-07-06: IME ON の 92ms 後の無変換タップで「IME OFF・Engine ON」になり親指シフト入力が生ローマ字化）
  - アクティブ IME が MS-IME と確定した最初の `WM_IME_KIND_CHANGED` で `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME` の `IsKeyAssignmentEnabled` / `KeyAssignmentMuhenkan` / `KeyAssignmentHenkan` を読み取り、競合時は警告ポップアップを表示。「はい」で `ms-settings:regionlanguage-jpnime`（Microsoft IME 設定ページ）を直接開く
  - GJI 利用中はチェック自体をスキップ。レジストリは読み取り専用（自動書き換えによる解除は行わない）

### 変更

- **意味を失っていた設定項目を撤去**
  - `keyboard_model`: レイアウトパースが JIS 固定で一度も配線されていなかった（設定画面の jis/us 選択は無効だった）
  - `output_mode`: アプリごとの自動注入方式選択（InjectionMode）に置換済みで、書き込みのみの死に設定だった
  - `hook_mode`: Relay に一本化（Filter は relay 系機能の登場以降テストされていないレガシー経路のため実装ごと削除）
  - 旧 config.toml にこれらのキーが残っていても無害（無視される）。分離前の旧内蔵設定 GUI（gui/main.rs、孤児ファイル）も削除
  - `confirm_mode` は生きた設定として存続（デフォルト wait のまま）。n-gram 系の詳細設定は「n-gram 予測」モード選択時のみ有効化されるよう UI を連動
- **設定画面にアプリ別オーバーライドと緊急脱出キーの UI を追加**
  - 新タブ「アプリ別」: `app_overrides`（force_text / force_bypass / force_vk / force_tsf）と `post_bypass`（tmux prefix 等の素通し）を GUI から編集可能に
  - キー設定タブ: `engine_off_solo_triple`（単独3連打でエンジン OFF、Ctrl スタック時の緊急脱出用）を追加
- **「アプリ別」「プレビュー」タブをサイドパネルから一旦非表示に**
  - 「アプリ別」は高度な機能のため GUI 化を見送り、`config.toml` の直接編集に委ねる
  - 「プレビュー」（配列プレビュー）はまだ実装が固まっていないため一旦非表示（今後の課題）
  - いずれも実装自体は残しており、`config.toml` の `app_overrides` / `post_bypass` / レイアウト設定は従来どおり有効

## [1.8.5] - 2026-07-06

### バグ修正

- **設定画面が複数ディスプレイの DPI 遷移で操作不能になる問題を修正** ([a7d3e53](https://github.com/cuzic/awase/commit/a7d3e53), [7cb1625](https://github.com/cuzic/awase/commit/7cb1625), [7207413](https://github.com/cuzic/awase/commit/7207413))
  - DPI スケールの異なるディスプレイへ移動するとウィンドウが移動先モニタに収まらず、適用/キャンセルボタンが画面外に出ていた
  - モニタサイズへの自動クランプ + ボタンを常時表示の下部パネルへ移動 + 最小ウィンドウサイズを設定
  - 狭い幅ではリフロー（キーボード図の縮小スケール・keymap 行の折り返し）で対応し、収まらない場合のみスクロールバーを表示。デフォルト幅を 580px に拡大（従来はプレビューの右端が切れていた）

## [1.8.4] - 2026-07-06

### 新機能

- **親指キーの選択肢に F13-F24 を追加** ([8951418](https://github.com/cuzic/awase/commit/8951418))
  - プログラマブルキーボード（QMK/ZMK 等）で親指位置に F13-F24 を割り当てているユーザー向け
  - `VkCode::from_name` にも VK_F13〜VK_F24 のパースを追加

## [1.8.3] - 2026-07-06

### バグ修正

- **Edge/Chrome フォーカスの約500ms後に Engine が必ず OFF になる問題を修正 (BUG-07)** ([0d67f20](https://github.com/cuzic/awase/commit/0d67f20))
  - TsfGate の bypass 確定処理が probe 未実行のまま `write_focus_probe(false)` の偽観測を毎リフレッシュ注入していた（「非TSFウィンドウには日本語IMEが存在しない」という誤前提、ce45b82 の revert）
  - 実観測経路を持たない Imm32Unavailable では偽 Low false が belief を支配し訂正不能だった。architecture_guard で `write_focus_probe` の呼び出し箇所を実 probe 経路に固定
- **ワーカースレッド発のメッセージが main スレッドに届かない問題を修正 (BUG-09)** ([69f271d](https://github.com/cuzic/awase/commit/69f271d))
  - `post_to_main_thread` の `PostMessageW(NULL)` は「呼び出しスレッド自身への投函」であり、gji-io-monitor 発の `WM_IME_KIND_CHANGED` が消失 → MS-IME 環境でも warmup 戦略がデフォルトの GjiFsm のまま迷走していた（検出層は正しいのに出力層だけ壊れる split-brain）
  - `PostThreadMessageW(engine_thread_id())` 化 + メッセージループ開始時の IME 種別 pull 同期。実機で MicrosoftIme 検出の 3ms 後に MsImeStrategy 切替を確認
- **MS-IME で物理ひらがなキーを押しても IME ON にならない問題を修正 (BUG-10)** ([9d5040b](https://github.com/cuzic/awase/commit/9d5040b))
  - TSF mode の物理 F2 (VK_DBE_HIRAGANA) 無条件 Suppress は GJI 戦略の「F2 代替送信」契約とセットだったが、MsImeStrategy では代替が送られず食い逃げになっていた（Engine ON・実 IME OFF の乖離）
  - Suppress を GJI 戦略（`f2_warmup_owned`）に限定し、MS-IME では物理キーを素通し
- **合成 VK_KANA によるかなロック反転で JIS かな入力化する問題への防御 (BUG-08)** ([b38d67f](https://github.com/cuzic/awase/commit/b38d67f))
  - 物理押下では不可能な間隔（135µs）の合成 VK_KANA ペアがパススルーされ、GJI がローマ字→JIS かな入力に反転していた
  - `LLKHF_INJECTED` 付き VK_KANA を hook で swallow し、全 VK_KANA 到達に注入元特定用の診断ログを追加

### 撤回・無効化

- **UIA 非同期 focus 分類の適用を無効化 (BUG-11/BUG-12)** ([d941721](https://github.com/cuzic/awase/commit/d941721), [f88e89b](https://github.com/cuzic/awase/commit/f88e89b))
  - BUG-09 の配送修正で史上初めて実行された受信ハンドラに 2 段階の実害が露出（キャッシュキー取り違え → Edge 永久 NonText、修正後も (pid,class) キャッシュ粒度とブラウザのウィンドウ内要素粒度の構造的不一致で再発）
  - ハンドラをログのみに変更し、配送修正前の実績ある挙動へ意図的に復帰。sync 分類（既知クラス・WS_EX_NOIME・MSAA）は従来どおり
- **JIS かな自動復元（restore_roman）の TsfNative での発火を撤回** ([92fddc8](https://github.com/cuzic/awase/commit/92fddc8), [f88e89b](https://github.com/cuzic/awase/commit/f88e89b))
  - MS-IME × TsfNative では conv の ROMAN=0 が偽陽性であり、復元書き込みが conv を 0x19⇄0x09 で往復させ、直接入力中の spurious Engine/IME ON を誘発した
  - `is_roman_reliable=true` の文脈のみ発火する仕様に変更（TsfNative idle 経路では実質無効）。経緯は docs/experiments.md エントリ 03 参照

## [1.8.2] - 2026-07-05

### バグ修正

- **LINE 等で drain キュー処理中に後続の入力文字が消える問題を修正** ([233b6e3](https://github.com/cuzic/awase/commit/233b6e3))
  - `UnicodeColdWarmupFsm` が飛行中に新たな Unicode 文字が来ると FSM が上書きされ `deferred_chars` が失われていた
  - `push_deferred_unicode_chars` を `TickableFsm` に追加し、飛行中 FSM に追記できるようにして上書きを回避
- **MS-IME 環境で起動直後に GJI warmup（VK_A+BS）が誤発火する問題を修正** ([b694ffc](https://github.com/cuzic/awase/commit/b694ffc))
  - gji_monitor スレッドが `WM_IME_KIND_CHANGED` を起動時に送信せず、`MsImeStrategy` の切り替えが遅延していた
  - 初期 IME 種別検出後に無条件で `WM_IME_KIND_CHANGED` を post し、起動直後から正しい warmup 戦略を適用

### リファクタリング

- **unicode-cold-warmup ロジックをヘルパーメソッドに抽出** ([60a0b79](https://github.com/cuzic/awase/commit/60a0b79))
  - `start_unicode_cold_warmup` / `flush_unicode_cold_deferred_chars` を `WindowsPlatform` に追加し、`send_keys` と `dispatch_gji_response` の重複コードを統合
- **メソッド抽出で多段ネスト・重複を削減** ([b0cd4e9](https://github.com/cuzic/awase/commit/b0cd4e9), [c4900a3](https://github.com/cuzic/awase/commit/c4900a3), [f5023b4](https://github.com/cuzic/awase/commit/f5023b4))
  - `fmt_conv()` / `store_gji_warmup_if_probing()` で `probe_io.rs` の重複ブロックを集約
  - `observer::gji_idle_ms()` で 2行の idle 計算を関数化（6箇所削減）
  - `dispatch_gji_event()` で `gji_on_event + dispatch_gji_response` の 4行パターンを 1行化（5メソッド）
  - `Output::on_f22_f21_sent()` で IME OFF→ON 通知の 3行パターンを 1行化（2箇所）
  - `KeyInjector::format_vk_run()` で VK 列フォーマットを関数化（`send_vk_runs*` 3メソッド共用）
  - raw `SendInput` 7箇所を既存の `win32::send_input_safe()` に統一し `unsafe` ブロックを撤去

## [1.8.1] - 2026-07-05

### バグ修正

- **Alt+Tab 等の高速フォーカス遷移中に IME の内部認識 (belief) と実際の OS 側 IME 状態がずれる問題を修正** ([dd6f208](https://github.com/cuzic/awase/commit/dd6f208), [08ce474](https://github.com/cuzic/awase/commit/08ce474), [11ee689](https://github.com/cuzic/awase/commit/11ee689), [435e2d3](https://github.com/cuzic/awase/commit/435e2d3))
  - GJI 強制 IME-ON ガードが「GJI プロセスの生存」ではなく「実際にアクティブな IME 種別」を見るよう修正し、MS-IME 環境での VK_DBE_HIRAGANA 二重送信を解消
  - フォーカス遷移直後の settle バリア (`settle_until`) を配線し、`Engine`/`ImeEffect::SetOpen` の全呼び出し経路（キーボード入力・フォーカス変更通知・IME リフレッシュポーリング・ホットキー等）を `execute_from_loop`/`execute_from_hook` の2箇所で一括ガード
  - `AppImeProfile::from_class_name` の分類優先順位により Windows Terminal (`CASCADIA_HOSTING_WINDOW_CLASS`) が `TsfNative` に一切分類されない仕様を見落として「非 TSF ネイティブ」と誤判定していた5箇所を `is_effectively_tsf_native` に統一
- **BrokenAppBootstrap force guard がユーザーの明示的 OFF 意図を上書きする問題を修正** ([a06ad69](https://github.com/cuzic/awase/commit/a06ad69))

## [1.8.0] - 2026-07-05

### 新機能

- **FocusEpoch + ImmLikeTicket による観測受理層 (ObservationAdmission) を新設** ([604cf99](https://github.com/cuzic/awase/commit/604cf99), [569bf0f](https://github.com/cuzic/awase/commit/569bf0f), [67e63ad](https://github.com/cuzic/awase/commit/67e63ad), [1345d95](https://github.com/cuzic/awase/commit/1345d95)) (ADR-077)
  - フォーカス遷移をエポックで管理し、古い観測を構造的に棄却することで ImmCross アプリの誤観測連鎖を根本から防止
  - ImmCrossProbe に shadow grace 抑制を追加し、全パスでエポック照合を徹底
  - 棄却カウンタとダンプ時ログを追加し観測受理状況を可視化
- **設定画面で親指キーをドロップダウンで選択できるように変更** ([9e1621a](https://github.com/cuzic/awase/commit/9e1621a))
  - 親指左・親指右キーの割り当てをドロップダウン UI から直接変更可能になり、設定変更を即時反映
- **ウェブサイトから MSI/ZIP を直接ダウンロード可能に** ([877b1af](https://github.com/cuzic/awase/commit/877b1af), [15c6ff5](https://github.com/cuzic/awase/commit/15c6ff5), [21eadb1](https://github.com/cuzic/awase/commit/21eadb1))
  - awase.cc から GitHub Releases の最新アセットを直接ダウンロードできるリンクを追加

### バグ修正

- **ImmCross アプリで IME ON 後にかなモードのままエンジンが停止する問題を修正** ([81f6576](https://github.com/cuzic/awase/commit/81f6576), [e4378c6](https://github.com/cuzic/awase/commit/e4378c6))
  - IME ON 直後の ObservedKana 観測を ImmCross アプリで抑制し、エンジンが誤って非活性化されるケースを解消
- **MS-IME + ImmCross アプリで IME ON 時に romaji モードが維持されない問題を修正** ([91631a0](https://github.com/cuzic/awase/commit/91631a0), [1969cd3](https://github.com/cuzic/awase/commit/1969cd3), [4c02f2e](https://github.com/cuzic/awase/commit/4c02f2e))
  - MS-IME で IME ON 前に ROMAN モードを強制し、ObservedKana 時は romaji 強制をスキップ
- **IME 種別判定をプロセス存在からCLSID API に統一し IME 動的切り替えに対応** ([e6456b2](https://github.com/cuzic/awase/commit/e6456b2), [ce252fe](https://github.com/cuzic/awase/commit/ce252fe))
  - GJI 固定ポリシーを廃止し、実行時に CLSID API で IME 種別を判定することで GJI/MS-IME の動的切り替えをサポート
- **設定画面起動時に黒いコンソールウィンドウが表示される問題を修正** ([49d4f31](https://github.com/cuzic/awase/commit/49d4f31))
- **設定画面から親指キーを変更した際に即時反映されるよう修正** ([6cd1178](https://github.com/cuzic/awase/commit/6cd1178))
- **HwndCache 由来 intent では即時ドリフト補正しないよう修正** ([d418035](https://github.com/cuzic/awase/commit/d418035))
- **XamlExplorerHostIslandWindow を NonText に分類** ([a8d9a00](https://github.com/cuzic/awase/commit/a8d9a00))

### リファクタリング

- **全 ImmCrossProbe をエポック照合に移行・shadow grace 撤去** ([f478ee7](https://github.com/cuzic/awase/commit/f478ee7))
- **TipDetector のファイルキャッシュを廃止しプロセス内 OnceLock のみに統一** ([9e0ecf0](https://github.com/cuzic/awase/commit/9e0ecf0))

## [1.7.1] - 2026-07-02

### バグ修正

- **LINE 等 ImmCross アプリで GJI 英数→ひらがな切替後にエンジンが停止する問題を修正** ([4bcf8b0](https://github.com/cuzic/awase/commit/4bcf8b0))
  - GJI は英数モードでも ROMAN bit を立てないため `classify_transition` がひらがな復帰を検出できず `ObservedEisu` が残留してエンジンが `Inactive(NotRomajiInput)` になっていた
  - `classify_ime_snapshot` に stale 回復ブランチを追加し、ImmCross アプリで conv_mode が非英数に変化した時点で `AssumedRomaji` へ自動リセット

### リファクタリング

- **ImmCrossProbe の input_mode 更新を `classify_fetched_snapshot` 経由に統一** ([c2c8b55](https://github.com/cuzic/awase/commit/c2c8b55))
  - Observe → pure decision → belief の三層分離を ImmCrossProbe ハンドラに適用し、インライン判定を撤去

## [1.7.0] - 2026-07-02

### 新機能

- **IME ON のまま英数直接入力になっている状態 (ObservedEisu) を自動検出し IME OFF (直接入力) へ切替** ([30e6c5f](https://github.com/cuzic/awase/commit/30e6c5f), [1ef82ca](https://github.com/cuzic/awase/commit/1ef82ca), [754a7a4](https://github.com/cuzic/awase/commit/754a7a4)) (ADR-074)
  - `InputModeState::ObservedEisu` を新設し「shadow=ON だが実際は半角英数直接入力」を明示的に区別
  - `idle_conv_check` が `ObservedEisu` を検出すると自動で IME OFF + Engine 非活性化を発行
  - `SetOpen(true)` 直後に `ObservedEisu` が残っているケースは `AssumedRomaji` にリセットして Engine を即活性化
- **トレイメニューの「内部状態をリセット」をプロセス再起動に置き換え** ([a720c7a](https://github.com/cuzic/awase/commit/a720c7a))
  - FSM / IME 状態が壊れた際、マウス操作だけでプロセス全体を再起動し完全にクリーンな状態へ復帰できるように変更
  - GJI 種別判定 (`active_ime_kind`) のプロセス中固定 (ADR-073) など、実行時に確定した状態をリセットする唯一の確実な手段として位置づけ

### 改善

- **Warmup 送信 VK を Charset (ひらがな/カタカナ/JIS かな) 対応に拡張** ([536d93a](https://github.com/cuzic/awase/commit/536d93a))
  - GJI が非アクティブ（MS-IME 使用時等）な場合の TSF warmup 送信を判定に集約しスキップ漏れを解消
- **`ConvModePolicy` による conv_mode 変更の一括ゲートを導入** ([2826248](https://github.com/cuzic/awase/commit/2826248), [cc9f745](https://github.com/cuzic/awase/commit/cc9f745), [af3b776](https://github.com/cuzic/awase/commit/af3b776)) (ADR-064)
  - `Output` に `conv_mutation_allowed` ゲートを追加し、conv_mode への書き込みを一箇所で検証可能に
  - `is_romaji_mode: bool` を `ConvModePolicy` 型に置き換えて意味を明確化

### バグ修正

- **MS-IME での Ctrl+変換 / Ctrl+無変換 が正しく動作しないバグ群を修正** ([f527a18](https://github.com/cuzic/awase/commit/f527a18), [baff902](https://github.com/cuzic/awase/commit/baff902), [301e911](https://github.com/cuzic/awase/commit/301e911), [48a667a](https://github.com/cuzic/awase/commit/48a667a))
  - Ctrl+変換 で MS-IME がひらがなにならない・半角英数から戻らない不具合を修正
  - `MsImeDirectStrategy` の DirectInput 切替を冪等な `VK_IME_OFF` に統一
  - GJI が一度確定した後は MS-IME への誤降格を禁止（プロセス中固定、ADR-073）
- **ImmCrossProbe / GJI+TsfNative フォーカス時の IME 誤認識・打鍵消失を修正** ([496926c](https://github.com/cuzic/awase/commit/496926c), [489cdf1](https://github.com/cuzic/awase/commit/489cdf1), [15ac8d8](https://github.com/cuzic/awase/commit/15ac8d8), [500dab6](https://github.com/cuzic/awase/commit/500dab6)) (ADR-071, ADR-075)
  - Qt / GJI フォーカス時に `ImmCrossProbe` が IME 状態を誤認識するバグを修正
  - GJI+TsfNative の IME OFF 送信を `VK_KANJI` トグルから冪等な `VK_IME_OFF` に変更
  - TSF cold-start probe 実行中に後続キーが消失し「にゅうりょく」→「にうりょく」になるバグを修正（deferred VK キューの所有権を probe machine から `TsfWarmupCoordinator` へ移管）
- **`conv_mode_authority` の再同期漏れによる IME/Engine desync を修正** ([e2199e7](https://github.com/cuzic/awase/commit/e2199e7), [c887da8](https://github.com/cuzic/awase/commit/c887da8), [1f96598](https://github.com/cuzic/awase/commit/1f96598)) (ADR-072)
  - IME apply 完了ごとに `conv_mode_authority` を再同期し、パニックリセット直後や2回目の Ctrl+変換 で古い権限値が残る問題を解消
  - `already_matched` 判定を全 IME 種別で一貫させ、`candidate_was_seen` を誤って混入させないよう修正
- **カタカナ / JIS かなモードでの IME・NICOLA エンジン状態同期バグ群を修正** ([66bfc33](https://github.com/cuzic/awase/commit/66bfc33), [58fa2ac](https://github.com/cuzic/awase/commit/58fa2ac), [e8b09de](https://github.com/cuzic/awase/commit/e8b09de), [927f2a2](https://github.com/cuzic/awase/commit/927f2a2), [0f652ae](https://github.com/cuzic/awase/commit/0f652ae))
  - カタカナモード切替で NICOLA エンジンが OFF のまま残る／トレイで半角カタカナ選択後にひらがなに戻される不具合を修正
  - JIS かな・カタカナのタスクバー経由モード変更を belief に正しく反映
  - HanKata → ZenKata の誤ダウングレードを抑制
- **idle-conv-check 起因の Engine 状態同期漏れ・誤検出を修正** ([0f75b5b](https://github.com/cuzic/awase/commit/0f75b5b), [ea3da7f](https://github.com/cuzic/awase/commit/ea3da7f), [ed862bb](https://github.com/cuzic/awase/commit/ed862bb), [a325df7](https://github.com/cuzic/awase/commit/a325df7), [29bc9c4](https://github.com/cuzic/awase/commit/29bc9c4))
  - カタカナ+shadow=OFF や HanAlpha→Hiragana 遷移時に Engine が復帰しない不具合を修正
  - ALT+TAB 後の Chrome IME 状態誤認識、tray desync（`user_enabled=true` だが `ime_on=false`）を修正
- **GJI cold-start / partial literal 対策を SacrificialWarmup (VK_A+BS) に統合** ([6c1732d](https://github.com/cuzic/awase/commit/6c1732d), [22c3905](https://github.com/cuzic/awase/commit/22c3905))
  - `VK_IME_OFF→ON` + `write_bytes` 検出ベースの warmup に置き換え、vim 等との互換性を確保
- **PC スリープ復帰後の最初の打鍵でエンジンが固定されるバグを修正** ([c4df99b](https://github.com/cuzic/awase/commit/c4df99b)) (ADR-076)
  - スリープ復帰直後に `read_ime_state_fast` が一時的に `is_japanese_ime=false` を返し、エンジンが `NotJapaneseIme` で非活性化するバグを修正
  - `apply_focus_probe` 内で grace 期間の判定を `set_is_japanese_ime` より前に計算し、shadow grace active 中は `false` へのダウングレードを抑制（`true` への更新は常時許可）
  - `imc_open` と `is_japanese_ime` の grace 保護を対称化
- **プロセス再起動時に named mutex レースで起動に失敗するバグを修正** ([bb84696](https://github.com/cuzic/awase/commit/bb84696))
  - 本バージョンで追加したトレイの「プロセス再起動」機能自体に存在した不具合

### 内部改善

- **凝集性リファクタ H-1〜M-5 全21タスク完了**: 型定義の循環依存解消、状態層の OS グローバル直接呼び出し除去、`PlatformState` を `FocusStore`/`GateStore`/`KeymapStore` に、`Runtime` を `FocusTracker`/`RefreshScheduler`/`ImeCoordinator` に分割（ADR-069） ([a7a1e23](https://github.com/cuzic/awase/commit/a7a1e23), [811036a](https://github.com/cuzic/awase/commit/811036a), [8aad49a](https://github.com/cuzic/awase/commit/8aad49a))
- **`ApplyBelief` → `OpenBelief` にリネームし `reduce_open_belief` 純粋関数に統合**（IME apply 判定ロジックを一箇所に集約・単体テスト容易化）（ADR-070） ([1e09e90](https://github.com/cuzic/awase/commit/1e09e90), [8c34984](https://github.com/cuzic/awase/commit/8c34984))
- **`RuntimeOutbox`/`RuntimeRequest` イベントバスを導入し `Output` → `Runtime` の逆依存 (`with_app()`) を撤去** ([4782e3e](https://github.com/cuzic/awase/commit/4782e3e), [7568180](https://github.com/cuzic/awase/commit/7568180))
- **`TsfWarmupCoordinator` / `KeyInjector` / `ImeApplyPlanner` を巨大化していた `Output` から抽出** ([03ebfc9](https://github.com/cuzic/awase/commit/03ebfc9), [e1993bf](https://github.com/cuzic/awase/commit/e1993bf), [080e96d](https://github.com/cuzic/awase/commit/080e96d))
- **`ConvMode`/`Charset` 判定を `nicola` クレートへ移動し純粋関数化、`#[cfg(windows)]` ゲートを個別モジュール単位に緩和して Linux 単体テストを追加**（ADR-065） ([b1ab86f](https://github.com/cuzic/awase/commit/b1ab86f), [5ee724b](https://github.com/cuzic/awase/commit/5ee724b), [b27a218](https://github.com/cuzic/awase/commit/b27a218))

## [1.6.0] - 2026-06-29

### 新機能

- **Microsoft IME 完全対応** ([45edf19](https://github.com/cuzic/awase/commit/45edf19), [56cd9a5](https://github.com/cuzic/awase/commit/56cd9a5))
  - `MsImeDirectStrategy`（`VK_DBE_HIRAGANA` / `VK_DBE_ALPHANUMERIC`）による冪等 ON/OFF 制御（ADR-063）
  - `WM_IME_KIND_CHANGED` でランタイムに GJI / MS-IME を切り替え
- **TSF EnumProfiles による GJI CLSID 動的発見** ([55233f0](https://github.com/cuzic/awase/commit/55233f0), [005f0a6](https://github.com/cuzic/awase/commit/005f0a6))
  - プロセス名依存から CLSID ベースの IME 種別判定に移行
  - CLSID を `cache.toml` に永続化し再起動後のコールドスタートコストを削減

### 改善

- **GJI IME 制御を VK_IME_ON/OFF (0x16/0x1A) に移行** — config1.db パッチ不要に ([b271aee](https://github.com/cuzic/awase/commit/b271aee), [8c39b8e](https://github.com/cuzic/awase/commit/8c39b8e))
  - `gji.rs`（config1.db パッチ・プロセス管理）を完全削除（-692 行）
  - トレイメニューの「GJI セットアップ / 解除」を撤去、初回インストール作業が不要に
  - Chrome / WezTerm / Windows Terminal すべてで VK_IME_ON/OFF の動作を実機確認（2026-06-28）
  - `gji_keybinds_ok` フラグ・Observer の config1.db 監視ループを撤去
- **IME 種別判定を CLSID ベースに一本化** ([2d5cfe9](https://github.com/cuzic/awase/commit/2d5cfe9))
  - `gji_write_idle_ms` ヒューリスティックを撤去し判定精度を向上

### バグ修正

- **WezTerm で Enter 後の最初の文字（「な」等）が突然消えるバグを修正** ([3ffbe66](https://github.com/cuzic/awase/commit/3ffbe66))
  - ReinjectConfirmKey + TSF mode で `nc_fired` を昇格し LiteralDetect 誤検出を抑制
- **MS-IME アクティブ時の LiteralDetect 誤発火による BS 連射を修正** ([699ab5f](https://github.com/cuzic/awase/commit/699ab5f))
- **パニックリセットでカタカナ・JIS かな状態からもローマ字ひらがなに復帰** ([a88bb36](https://github.com/cuzic/awase/commit/a88bb36), [6ee20bf](https://github.com/cuzic/awase/commit/6ee20bf))
  - `IMC_SETCONVERSIONMODE` で `NATIVE | FULLSHAPE | ROMAN` を強制し `KATAKANA` を落とす
  - 半角カタカナ（JIS かな入力モード含む）・全角カタカナからでもローマ字ひらがなに戻る

## [1.5.0] - 2026-06-27

### 新機能

- **トレイメニューに「内部状態をリセット」を追加** ([1591593](https://github.com/cuzic/awase/commit/1591593))
  - IME 状態や FSM が壊れたときにマウス操作だけで内部状態を初期化可能
  - キーボードが正常に動かない状況でも確実にリセットできる（ADR-052）
- **StepCoro — タイマー駆動コルーチン基盤を導入** ([e548e63](https://github.com/cuzic/awase/commit/e548e63))
  - `GjiWarmupFsm` / `LiteralDetectFsm` / `SacrificialWarmupFsm` / `TsfProbeMachine` を StepCoro に置き換え（ADR-053）
  - FSM ステートの中間変数をコルーチンのローカル変数として記述でき、コード量を大幅削減
  - `timed_fsm::coro` モジュールとして昇格（[118b3cf](https://github.com/cuzic/awase/commit/118b3cf)）
- **InjectionMode を cache.toml で永続化** ([c6fed60](https://github.com/cuzic/awase/commit/c6fed60))
  - `InjectionModeStore` を追加し、セッションをまたいで注入モードを記憶
  - 再起動後の cold-start コストを削減（ADR-058）
  - `imm_cache.toml` を `cache.toml` に統合（[10918c2](https://github.com/cuzic/awase/commit/10918c2)）
- **InjectionMode 事後昇格: GJI write_bytes 観測で自動昇格** ([68c24e1](https://github.com/cuzic/awase/commit/68c24e1))
  - フォーカス直後に Unicode モードで送信したキーが GJI に処理されたか WriteTransferCount で判定
  - GJI が動作していれば injection_mode を Tsf に自動昇格（ADR-062）
- **Unicode long-cold 対応: UnicodeColdWarmupFsm** ([70d3fed](https://github.com/cuzic/awase/commit/70d3fed))
  - Unicode モードで長時間アイドル後に VK_IME_ON poke を送り GJI の起動を確認してから文字を送信
  - GJI が応答するまで文字送信を保留することで partial literal を防止
- **競合ソフトウェア起動時チェックを追加** ([9875c2c](https://github.com/cuzic/awase/commit/9875c2c))
  - やまぶき等の NICOLA 対応 IME が同時に起動している場合、バルーン通知で警告（ADR-060）

### 改善

- **自動起動を schtasks → HKCU\\Run レジストリに移行** ([0d09d80](https://github.com/cuzic/awase/commit/0d09d80))
  - ログオン即起動（30 秒遅延が不要に）、管理者権限なしで登録可能（ADR-059）
  - 旧 schtasks タスクは起動時に自動削除（[584897a](https://github.com/cuzic/awase/commit/584897a)）
- **Windows Terminal を force_tsf に追加** ([4e49ee1](https://github.com/cuzic/awase/commit/4e49ee1))
  - GJI コンポジションを確実に有効化し、Windows Terminal での変換精度を向上

### バグ修正

- **Win キー押下中の IME キー注入をスキップ** ([6469f51](https://github.com/cuzic/awase/commit/6469f51))
  - Win+A でスタートメニューが開いてしまう誤動作を修正（ADR-061）
- **仮想デスクトップ切替時の誤 IME キー送信を防止** ([a97173a](https://github.com/cuzic/awase/commit/a97173a))
  - 仮想デスクトップを切り替えた直後に LINE 等へ誤って IME キーが送信されていたフォーカスガード漏れを修正
- **Partial literal の残骸を BS で除去** ([f3ff84d](https://github.com/cuzic/awase/commit/f3ff84d))
  - TSF literal recovery が give-up したとき terminal に残ったリテラル文字を BS で削除
- **Partial literal 検出後の resend を SacrificialWarmup 化** ([62ad28b](https://github.com/cuzic/awase/commit/62ad28b))
  - 部分リテラル検出後の再送を安全な warmup フローに統一
- **SacrificialWarmup Phase2 早期 HIDE 後に IPC settle 待機を追加** ([88d562f](https://github.com/cuzic/awase/commit/88d562f))
  - 候補ウィンドウが早期 HIDE された後、GJI IPC が完了するまで待機してから次入力を解放
- **Unicode long-cold で VK_IME_ON 単体 → VK_IME_ON+VK_A+BS 犠牲キーに変更** ([6cb175f](https://github.com/cuzic/awase/commit/6cb175f))
  - GJI の WriteTransferCount を確実に増加させ cold 判定の精度を向上
- **TsfNative/Imm32Unavailable で shadow 値を代替観測として記録** ([1e77002](https://github.com/cuzic/awase/commit/1e77002))
  - IMM32 が利用不可なウィンドウでも shadow_on を観測値として保存し状態推定を安定化

### 内部改善

- **`#[allow]` を `#[expect]` に一括置換**（全クレート）([c2b0685](https://github.com/cuzic/awase/commit/c2b0685))
  - Rust 2024 edition の lint 強化に対応、抑制理由が不要になった時点でコンパイラが警告
- **`unsafe extern` を明示**（Rust 2024 lint 対応）([f645d3c](https://github.com/cuzic/awase/commit/f645d3c))
- **awase-settings を edition 2024 に移行** ([ee2487c](https://github.com/cuzic/awase/commit/ee2487c))
- **GjiWarmupFsm / StartLiteralDetect を撤去**（StepCoro 移行後の dead code 除去）([f01d401](https://github.com/cuzic/awase/commit/f01d401))
- **MSRV を 1.85 に引き上げ**（`timed_fsm::coro` が `async`/`await` 構文を使用）

---

## [1.4.0] - 2026-06-25

### 新機能

- **[[post_bypass]] 設定を追加**: tmux (`Ctrl+B`) や screen など、コマンドキーの直後の次打鍵を NICOLA スキップする設定を追加 ([a67ebb5](https://github.com/cuzic/awase/commit/a67ebb5))
  - `[[post_bypass]]` セクションで対象キーと待機時間を指定可能
  - tmux 向けのコメントと設定例を TOML サンプルに追記 ([49b8343](https://github.com/cuzic/awase/commit/49b8343))
- **SacrificialWarmup を導入**: Chrome cold-start と WezTerm long-idle の部分リテラル化を、自己犠牲 VK_A+BS バッチで解消 ([28fc97a](https://github.com/cuzic/awase/commit/28fc97a), [16a288d](https://github.com/cuzic/awase/commit/16a288d))
  - Chrome の「kおのなかで」型 partial literal を warmup フェーズで事前修正
  - WezTerm が長時間アイドル後に Engine-ON/IME-OFF 状態になるケースを解消
  - long-cold + TSF mode 専用に限定して通常時の性能低下を防止 ([d02ec44](https://github.com/cuzic/awase/commit/d02ec44))
- **ImeModeFsm + ChromeGjiReinitFsm を実装**: F22→F21 送信後、GJI が Hiragana モードに落ち着いたことを確認してから次入力を許可する FSM を追加 ([2c8d647](https://github.com/cuzic/awase/commit/2c8d647))
- **Chrome 用 LiteralDetector を WriteTransferCount ベースに改善**: パイプへの書き込みバイト数で composition 完了を判定し、誤 BS を抑制 ([c7ef500](https://github.com/cuzic/awase/commit/c7ef500))
- **F21/F22 全送信パスで belief 更新 + VK 直後の FocusChange 抑制**: IME モード state の信頼度を高め、不要な re-probe を削減 ([1a0e54e](https://github.com/cuzic/awase/commit/1a0e54e))

### バグ修正

**Ctrl+J 問題の完全修正**

- **IME ON 状態で Ctrl+J が tmux に届かない問題を修正** ([32a037d](https://github.com/cuzic/awase/commit/32a037d))
  - TsfGate が IME ON 中の Ctrl+J を GJI warmup パスに誤送信していた
- **Ctrl+J が GJI に横取りされる問題を再修正** ([ee0b1fd](https://github.com/cuzic/awase/commit/ee0b1fd))
- **Ctrl バイパス後に modifier 先行リリースが来ると J↑ を誤 Suppress する問題を修正** ([bd51ea1](https://github.com/cuzic/awase/commit/bd51ea1))
- **Ctrl+key bypass 直後の次キーを NICOLA スキップするよう修正** ([ddb6b58](https://github.com/cuzic/awase/commit/ddb6b58))
  - バイパス直後のキーが親指シフト同時打鍵と誤判定されるケースを排除

**SacrificialWarmup の安定化**

- **gate=Bypass で即終了するバグを修正** ([0426fb6](https://github.com/cuzic/awase/commit/0426fb6))
- **VK_A+BS を同一バッチで送信して文字フラッシュを防止** ([af906b1](https://github.com/cuzic/awase/commit/af906b1))
- **write_bytes ベースラインを VK_A 送信前に取得し閾値を 350B に調整** ([26bc0fe](https://github.com/cuzic/awase/commit/26bc0fe))
- **Chrome cold タイムアウト時に F22→F21 で GJI を強制リセット** ([9630acb](https://github.com/cuzic/awase/commit/9630acb))
- **Chrome warm 確認後、HIDE 待機してから実ローマ字を再送するよう修正** ([2355761](https://github.com/cuzic/awase/commit/2355761))
- **VK_A+BS atomic batch 後の早期 HIDE で「おお」が消える IPC race を修正** ([123efb1](https://github.com/cuzic/awase/commit/123efb1))
- **`composition_was_seen` が最初の tick で false になる drain 順序バグを修正** ([e40a2eb](https://github.com/cuzic/awase/commit/e40a2eb))

**WezTerm**

- **WezTerm long-idle 後の 2 文字目リテラル化を PendingGjiConfirm 状態で修正** ([7041b20](https://github.com/cuzic/awase/commit/7041b20))
- **フォーカス直後 injection_mode を同期し、Unicode long-cold 時に F22→F21 再初期化** ([6ecb8e9](https://github.com/cuzic/awase/commit/6ecb8e9))
- **HWND キャスト型を `*mut c_void` に修正**（`isize` への暗黙変換が Windows API 境界で UB）([ff1f092](https://github.com/cuzic/awase/commit/ff1f092))

**GJI FSM**

- **Unicode モードの `StartProbe` で即 `WarmupComplete` を dispatch**（OnCold 固着バグ修正）([444e9a6](https://github.com/cuzic/awase/commit/444e9a6))
- **ImeOn 直後の FocusChange で proactive probe を維持するよう修正** ([782f10d](https://github.com/cuzic/awase/commit/782f10d))
- **FocusChange spawn_local ポーリングに generation ガードを追加**（二重適用防止）([90c9962](https://github.com/cuzic/awase/commit/90c9962))

**その他**

- `ValidatedConfig` に `post_bypass` フィールドを追加（ビルドエラー修正）([089858d](https://github.com/cuzic/awase/commit/089858d))
- Unknown → 実値 の IME 状態遷移を drift WARN から initial confirm DEBUG に格下げ ([4347928](https://github.com/cuzic/awase/commit/4347928))
- `sleep_ms` に `u64` を渡していた型ミスマッチを修正 ([ddc6be3](https://github.com/cuzic/awase/commit/ddc6be3))

### 内部改善

- **HoldingGate / GateAction を timed-fsm クレートへ移植**: ゲートロジックをライブラリ層に集約し再利用性を向上 ([ebbc9e4](https://github.com/cuzic/awase/commit/ebbc9e4))
- **HwndId に `to_hwnd()` を追加し raw cast を一箇所に集約** ([1932948](https://github.com/cuzic/awase/commit/1932948))
- **`gji_candidate_visible` を env 経由で参照**（直接ポーリング撤去）([19f4372](https://github.com/cuzic/awase/commit/19f4372))
- **SacrificialResend から不要な `plan` / `observations` フィールドを削除** ([61c4c12](https://github.com/cuzic/awase/commit/61c4c12))

---

## [1.3.0] - 2026-06-23

### バグ修正

- **ALT+TAB が連続して押せない問題を修正** ([d6cb1a4](https://github.com/cuzic/awase/commit/d6cb1a4))
  - フォーカス変化のたびに F21/F22 (IME ON/OFF キー) を送信する際、ALT を一時的に解放していたため ALT+TAB スイッチャーが「ALT 離した＝確定」と誤認していた
  - F21/F22 は GJI 専用の仮想 VK のため ALT を保持したまま送信しても正常に動作する
- **GJI long-idle 後の「kお」cold start バグを修正** ([e4a6248](https://github.com/cuzic/awase/commit/e4a6248), [c571acf](https://github.com/cuzic/awase/commit/c571acf))
  - GJI が 8〜10 秒以上アイドル後、最初のキー入力が部分リテラル化（「こ」→「kお」）するケースを修正
  - medium idle (7〜10 秒) でも GJI 無応答タイムアウト時に F2 をバッチ同梱するよう改善 ([2159dca](https://github.com/cuzic/awase/commit/2159dca))
- **部分リテラル（「kお」「seつぞく」）の修正** ([5562b6a](https://github.com/cuzic/awase/commit/5562b6a), [d54f5b1](https://github.com/cuzic/awase/commit/d54f5b1), [125d2c1](https://github.com/cuzic/awase/commit/125d2c1))
  - GJI I/O 応答後に `gji_resumed` を設定して部分リテラルを救済
  - TSF mode でも LiteralDetect を有効化（「seつぞく」の再発防止）
  - 部分リテラル BS 数を `chars.len()` から 2 固定に修正
- **Chrome パスの LiteralDetector を改善** ([c279fc7](https://github.com/cuzic/awase/commit/c279fc7))
  - `new_gji_resumed` に切り替えて GJI resume 後のリテラル誤検出を抑制
- **probe-fsm: F2 二重送信・遅延の修正** ([d195c43](https://github.com/cuzic/awase/commit/d195c43), [4de228b](https://github.com/cuzic/awase/commit/4de228b))
  - ReWarmup/non-eager パスで TSF バッチへの F2 二重送信を抑制
  - GJI pre-idle 時に fresh F2 + NameChangeWait をスキップして遅延を削減
- **composition-fsm: Long cold 状態を正しく維持するよう修正** ([c96598f](https://github.com/cuzic/awase/commit/c96598f))

### 新機能

- **GjiWarmupFsm を新規作成**: GJI cold-start warmup 専用 FSM を導入し warm-up ロジックを独立化 ([fcd1b82](https://github.com/cuzic/awase/commit/fcd1b82), [f768944](https://github.com/cuzic/awase/commit/f768944))
- **LiteralDetectFsm を新規作成**: warm パス・GJI post-transmit で共用するリテラル検出 FSM ([8608062](https://github.com/cuzic/awase/commit/8608062), [660ee19](https://github.com/cuzic/awase/commit/660ee19))
- **ChromeProbe を新規作成**: `pending_tsf` を `Box<dyn TickableFsm>` に換装し Chrome 専用 probe を追加 ([51901c0](https://github.com/cuzic/awase/commit/51901c0))
- **ColdKind::Medium を追加**: GJI idle 時間を Long / Medium / Short に分類して warmup 戦略を最適化 ([bf3eade](https://github.com/cuzic/awase/commit/bf3eade))
- **NameChangeWait で candidate 可視時に即 transmit**: WezTerm での probe 待ち時間を最大 300ms 短縮 ([25182eb](https://github.com/cuzic/awase/commit/25182eb))

### 内部改善

- **TickableFsm トレイト定義**: `TsfProbeMachine` / `GjiWarmupFsm` / `LiteralDetectFsm` / `ChromeProbe` が共通インターフェースを実装 ([d22e987](https://github.com/cuzic/awase/commit/d22e987))
- **ImeWarmupStrategy トレイト定義**: `GjiFsm` / `MsImeStrategy` を統一インターフェースで扱えるよう抽象化 ([eb8b9d4](https://github.com/cuzic/awase/commit/eb8b9d4))
- **GjiFsm 大規模リファクタリング**: `ProbeStatus` を `Authorized+Executing` に分離し Cell 3本を撤去、`OutputActiveGuard` を Output に移動 ([0ca92b2](https://github.com/cuzic/awase/commit/0ca92b2), [65e0a1a](https://github.com/cuzic/awase/commit/65e0a1a))
- **transport リファクタリング**: `PassthroughQueue` を抽出・`PhysicalKeyDisposition::plan()` に F2 ケースを統合 ([3cfc57b](https://github.com/cuzic/awase/commit/3cfc57b), [005ed17](https://github.com/cuzic/awase/commit/005ed17))

---

## [1.2.0] - 2026-06-21

### 新機能

- **GjiFsm を新規追加**: GJI (Google 日本語入力) の内部 composition 状態を推測する FSM を導入 ([b152c7e](https://github.com/cuzic/awase/commit/b152c7e))
  - Phase 2a: GjiFsm を Output に接続し FocusChange / ImeOn / ImeOff / WarmupComplete を配線 ([bb228c2](https://github.com/cuzic/awase/commit/bb228c2))
  - Phase 2b: CompositionReset・KeyInput を配線し `is_composition_warm` を FSM 化 ([d49c516](https://github.com/cuzic/awase/commit/d49c516))
  - Phase 3: `is_composition_warm` を GjiFsm SSOT に切替 ([2b6d25f](https://github.com/cuzic/awase/commit/2b6d25f))
  - Phase 4: legacy epoch warm 追跡を撤去し GjiFsm を SSOT に一本化 ([588ea32](https://github.com/cuzic/awase/commit/588ea32))
- **panic ログ強化**: panic 発生時の場所とメッセージを `awase.log` に記録するフックを追加 ([de0226a](https://github.com/cuzic/awase/commit/de0226a))
- **更新履歴ページ自動生成**: GitHub Actions で `CHANGELOG.md` → `changelog.html` を自動生成するワークフローを追加 ([efc4310](https://github.com/cuzic/awase/commit/efc4310), [6fb7a38](https://github.com/cuzic/awase/commit/6fb7a38))

### バグ修正

- **WezTerm TSF cold start の F2+S race** と `gji_resumed` 後の false-positive BS を修正 ([b754277](https://github.com/cuzic/awase/commit/b754277))
- **CoreWindow キャッシュミス時** の IME ON carry-over によるひらがな注入を修正 ([1c5cc91](https://github.com/cuzic/awase/commit/1c5cc91))
- **NICOLA 同時打鍵** で `StartProbe` が上書きされる `debug_assert` パニックを修正 ([a5a9412](https://github.com/cuzic/awase/commit/a5a9412))
- **Chrome: f2_gji_long_idle** フラグ有効時も programmatic F2 を強制送信するよう修正 ([43dca5a](https://github.com/cuzic/awase/commit/43dca5a))
- **probe: SetOpenTrue** 時も `consecutive_count` をリセットするよう修正 ([cbd1946](https://github.com/cuzic/awase/commit/cbd1946))
- **tray**: `WM_CLOSE` を明示的にハンドルして意図しないシャットダウンを防止 ([c924eed](https://github.com/cuzic/awase/commit/c924eed))

### 内部改善

- **probe-fsm**: `TransmitPlan` / `ProbeObservations` 導入により FSM レイヤー境界を整理 ([3c36f21](https://github.com/cuzic/awase/commit/3c36f21))
- **chord 管理**: ImeStateHub に完全集約（Phase 2 完了） ([fd17da0](https://github.com/cuzic/awase/commit/fd17da0))
  - Chord 開始/終了判断を reducer に集約 ([a7218e0](https://github.com/cuzic/awase/commit/a7218e0))
  - `pending_warmup_on_keyup` を CompositionFsm に昇格 ([f3b0448](https://github.com/cuzic/awase/commit/f3b0448))
- **transport**: `suppress_physical` を `PhysicalKeyDisposition` に分離 ([8a045bb](https://github.com/cuzic/awase/commit/8a045bb))
- Clippy pedantic 対応（CI Rust 1.96） ([881f824](https://github.com/cuzic/awase/commit/881f824))

---

## [1.1.1] - 2026-06-20

### バグ修正

- **Chrome → WezTerm フォーカス切替後の IME-OFF Engine-ON 状態** を修正 ([12dd094](https://github.com/cuzic/awase/commit/12dd094), [56f5e49](https://github.com/cuzic/awase/commit/56f5e49))
  - TsfNative 入場時に GJI F21 を `shadow_on` を無視して強制送信するよう変更
  - TsfNative cache miss 時の belief を carry-over (true) から安全デフォルト OFF に変更
  - フォーカス cache TTL を 5 秒 → 1 時間に延長（IME ON でウィンドウを離れて戻ると cache miss 扱いになっていた問題を解消）
  - 短期フォーカス (< 100ms) の cache 保存をスキップ（通知ポップアップ等が正常な状態を上書きするのを防止）
- **WezTerm gji_resumed 後の LiteralDetect false positive** を修正（「あ」が「a」になるケース） ([da8dad1](https://github.com/cuzic/awase/commit/da8dad1))
- **WezTerm gji_resumed 後の composition 早期確認** を実装（GJI I/O 変化を検知して待ち時間を短縮） ([aa8a79d](https://github.com/cuzic/awase/commit/aa8a79d))
- **comp-probe: RUNTIME 再入借用バグ** を修正（`shadow_on` / `jp` が常時 false になっていた） ([2225578](https://github.com/cuzic/awase/commit/2225578))
- **nicola-fsm: ソロ連打** でシフトカウンターが残存するバグを修正 ([a53344b](https://github.com/cuzic/awase/commit/a53344b))

### 内部改善

- GJI プロセスの I/O 統計 (ReadOperationCount / ReadTransferCount) を監視ログに追加 ([93236a8](https://github.com/cuzic/awase/commit/93236a8))

---

## [1.1.0] - 2026-06-20

### 重要な変更

- **GJI キーバインドを F13/F14 → F21/F22 に変更** ([7f8291f](https://github.com/cuzic/awase/commit/7f8291f))
  - F21/F22 は実キーボードに存在しない仮想キーで VT エスケープシーケンスを生成しない
  - WezTerm・Windows Terminal での Nop バインド設定が不要になった
  - **アップグレード時は必ずトレイメニューから「Google 日本語入力のセットアップ」を再実行してください**

### 新機能

- **GJI keybind 自動監視**: config1.db から F21/F22 エントリが消去された場合、30 秒以内に検知して自動再登録 ([58557e9](https://github.com/cuzic/awase/commit/58557e9))
- **トレイメニュー拡張**: GJI teardown・自動起動トグルを追加 ([e67cb49](https://github.com/cuzic/awase/commit/e67cb49))

### バグ修正

- **WezTerm long-idle 後の最初の文字リテラル化**（「こ」→「ko」）を修正（LiteralDetect + BS 再送方式） ([84e6942](https://github.com/cuzic/awase/commit/84e6942))
- **GJI IME-ON 不能バグ**を修正: `DirectInput\tF21\tIMEOn` エントリ欠落により F21 が無視されていた ([9d11cd7](https://github.com/cuzic/awase/commit/9d11cd7))
- **Teams / Chrome での partial literal**（「kおんな」→「こんな」変換失敗）を修正 ([3744457](https://github.com/cuzic/awase/commit/3744457), [040f8f8](https://github.com/cuzic/awase/commit/040f8f8))
- **GJI long-idle 後の LiteralDetect false positive** を修正 ([a6b4c0d](https://github.com/cuzic/awase/commit/a6b4c0d))

### パフォーマンス

- フォーカス変更直後の probe 待ち時間を 300ms → 100ms に短縮（入力レスポンス改善） ([23052fb](https://github.com/cuzic/awase/commit/23052fb))

### ドキュメント

- awase.cc の全ページで F13/F14 → F21/F22 に更新、WezTerm Nop 設定手順を削除

### 内部改善

- TSF probe の KeySeq 機構を削除（dead code）([550781f](https://github.com/cuzic/awase/commit/550781f))

## [1.0.1] - 2026-06-15

### バグ修正

- **Chrome VK モード**の「んい→に」変換バグを修正 ([71a4d68](https://github.com/cuzic/awase/commit/71a4d68))
- **Imm32Unavailable** 入場時に stale な `ime_on=false` が残るバグを修正 ([bfad1a8](https://github.com/cuzic/awase/commit/bfad1a8))
- Ctrl/Alt/Win 保持中の **KeyUp** を `on_key_down` と対称にバイパスするよう修正 ([5904d67](https://github.com/cuzic/awase/commit/5904d67))
- executor: **Relay モード**で Timer を即時実行し deferred timer の誤発火を修正 ([71ebfb9](https://github.com/cuzic/awase/commit/71ebfb9))
- executor: イベントキュー (`VecDeque`) の `push` を `push_back` に修正 ([3823f12](https://github.com/cuzic/awase/commit/3823f12))

### ドキュメント

- **ランディングページ**を大幅リニューアル（技術的差別化・ネーミング由来を追加）
- **使い方ページ** (usage.html) を新設（設定画面・config.toml 全項目・緊急操作手順を掲載）
- **内部動作解説ページ** (internals.html) を新設
- **FAQ** を大幅拡充
  - 高速タイピング時のシフト漏れ
  - Google IME でのトグルではなく冪等な IME 制御
  - 他ツール（やまぶき R 等）で起きがちな4つの症状と対策
  - Windows Terminal / WezTerm の F21/F22 Nop 設定手順
- コメント内の用語を統一（IMM → IMM32、IME-ON/OFF → IME ON/OFF、Henkan/Muhenkan → 変換/無変換）

### 削除

- `awase-gji-setup.exe` を配布物から削除（機能は awase 本体に統合済み）

### 内部改善

- `config`: `#[serde(default)]` 構造体に昇格して `default_*` 関数群 18 個を撤去（-142 行）
- `vk`: `enum VkMarker` を導入して bool/fn_ptr によるマーカー選択を型統一
- `fsm` / `nicola_fsm` / `ngram`: 重複コード・ラッパー関数・dead code を整理（合計 -270 行）
- rustfmt / clippy 整形

## [1.0.0] - 2026-06-14

最初の安定版リリース。

**Full Changelog**: https://github.com/cuzic/awase/compare/v0.1.0...v1.0.0

[Unreleased]: https://github.com/cuzic/awase/compare/v1.8.5...HEAD
[1.8.5]: https://github.com/cuzic/awase/compare/v1.8.4...v1.8.5
[1.8.4]: https://github.com/cuzic/awase/compare/v1.8.3...v1.8.4
[1.8.3]: https://github.com/cuzic/awase/compare/v1.8.2...v1.8.3
[1.8.2]: https://github.com/cuzic/awase/compare/v1.8.1...v1.8.2
[1.8.1]: https://github.com/cuzic/awase/compare/v1.8.0...v1.8.1
[1.8.0]: https://github.com/cuzic/awase/compare/v1.7.1...v1.8.0
[1.7.1]: https://github.com/cuzic/awase/compare/v1.7.0...v1.7.1
[1.7.0]: https://github.com/cuzic/awase/compare/v1.6.0...v1.7.0
[1.2.0]: https://github.com/cuzic/awase/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/cuzic/awase/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/cuzic/awase/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/cuzic/awase/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/cuzic/awase/compare/v0.1.0...v1.0.0
