# ADR-114 実装タスク分解（確定、Opus 敵対的レビュー r1〜r3 で収束、2026-08-31。実装中）

ADR-114（`docs/adr/114-keymap-app-scoped-shortcut-wiring.md`、設計確定 r1〜r4 収束済み）
を実装するための具体タスク一覧。実装順は依存関係に従う（T1a→T9 は概ね直列、T10 以降は
仕上げ）。各タスクは単一責務のコミットを想定。

## T1a: `keymap.rs` — `find_match`/`filter_active` の既存バグ修正

- `CompiledKeymap`/`KeymapTable` の構造はそのまま。`find_match` に `mods.win`
  比較を追加（`!mods.win` を条件に加える。現状 `Ctrl+I` が Win+Ctrl+I にも
  誤ってマッチする穴を塞ぐ）。
- `filter_active` の前方一致（`starts_with`）を `state/app_suppression::
  normalize_process_name` 相当の完全一致（小文字化 + 末尾 `.exe` 除去）に
  置き換える。`app_suppression.rs` から関数を再利用するか、同等ロジックを
  `keymap.rs` にコピーするかは実装時に決める（`app_suppression` 側を
  `pub(crate)` にして再利用するのが望ましい）。
- `KeymapRule.app` の doc comment（`src/config.rs:504-506`、現状「プロセス名
  （省略=全アプリ、大文字小文字無視）」）を、上記の完全一致化に合わせて
  「`.exe` の有無を問わず完全一致、前方一致はしない」に更新する（T1b の禁止
  対象の併記は T1b 側で行う）。
- ユニットテスト: `find_match` の `mods.win` 修正・`filter_active` の完全一致化
  （前方一致では拾えていたケースが拾われなくなること・完全一致が通ること）を
  Linux で実行可能なテストとして追加。
- このタスクは既存関数のバグ修正のみで、シグネチャ変更を含まないため単独で
  ビルドが通る。

## T1b: `keymap.rs` — `KeymapTable::new` の禁止 VK チェック（decision5）

T1a の後に実施する（同じファイルの変更が重ならないよう分離するだけで、
機能的な依存はない）。

- `KeymapTable::new` のシグネチャに親指 vk を追加する
  （`KeymapTable::new(rules, left_thumb_vk, right_thumb_vk)` 等）。
  **T1b 時点での呼び出し元は `bootstrap.rs:958` の1箇所のみ**
  （`crate::keymap::KeymapTable::new(&config.keymaps)` — `apply_config_update`
  （`runtime/mod.rs:1426`）は現状 `KeymapTable::new` を呼んでいないため、reload
  経路の呼び出しは T7 が新設する）。`bootstrap.rs:958` の時点では
  `left_thumb_vk`/`right_thumb_vk` は既にローカル変数として束縛済み
  （同ファイル内、`engine.set_thumb_key_solo_tap_config` 等 900〜941行付近で
  既に使われている値をそのまま流用できる）ため、T1b の時点では新しい
  アクセサ関数は不要——そのまま第2・第3引数として渡せば単独でビルドが通る。
  **`hook::thumb_vk_codes()` アクセサ（`cached_hook_config()`（`hook.rs:470`）が
  `CACHED_THUMB_VKS`（`hook.rs:141`）から復元しているのと同じロジックの公開版）
  は T7（reload 経路）で新設する**——`runtime/mod.rs:1472-1476` の
  `resolve_thumb_key(...)` の戻り値は if-let ブロックのスコープ内でしか使えず、
  `else`（`runtime/mod.rs:1553` 付近、"Invalid thumb key names" 警告）に落ちた
  場合は reload 時点で親指 vk が確定しないため、T7 側で安全に取得する手段が
  別途必要になる（T1b では発生しない問題）。
  - IME 制御系 VK（`vk::ImeKeyKind::from_vk` が `Some` を返すもの）を `from`/`to` の
    対象キーとして禁止。
  - Alt 系 VK（`VK_MENU`/`VK_LMENU`/`VK_RMENU`）を `from`/`to` の対象キーとして禁止。
  - **Alt を `from` の修飾子（`combo.alt == true`）として指定することも禁止**（対象
    キーの禁止とは別条件）。
  - Win 系 VK（`VK_LWIN`/`VK_RWIN`）を `from`/`to` の対象キーとして禁止。
  - Shift を `from` の主キー（`combo.vk` が `VK_LSHIFT`/`VK_RSHIFT`/`VK_SHIFT`）に
    指定することを禁止。
  - `VK_CAPITAL` を `from`/`to` の対象キーとして禁止。
  - 親指キー（引数で渡された `left_thumb_vk`/`right_thumb_vk`）を `from`/`to` の
    対象キーとして禁止。
  - 違反ルールは `log::warn!` して該当ルールのみ skip（既存のパース失敗と同じ扱い）。
    **warn メッセージには具体的な禁止理由を含める**（例:
    `"[keymap] 'from' の Alt 修飾は使用できません（ADR-114 決定5）: {from}"`）。
    設定 GUI の `new_keymap_from_alt`（`awase-settings/src/main.rs`、T8 で削除
    予定）で既に `from = "Alt+X"` を保存済みのユーザーがいる可能性があり、その
    ルールが黙って無効化される際に理由が追えないと混乱するため。
  - `KeymapRule.app` の doc comment（T1a で更新済み）に、上記の禁止対象
    （Alt 修飾・IME 制御系・親指キー等）を追記する。
- ユニットテスト: 上記の禁止ケースそれぞれについて `KeymapTable::new` が該当ルールを
  skip することを確認するテスト（Linux で実行可能、`cargo test -p awase-windows --lib`）。

## T2: `ime.rs` — `HeldModifiers` の `pub(crate)` 切り出し + マーカー引数化

- `HeldModifiers`（現状 `ime.rs` の private struct）を `output/held_modifiers.rs`
  （新設）または `output/key_injector.rs` へ `pub(crate)` として移動。
- `push_release`/`push_restore` の `IME_KANJI_MARKER` ハードコードを引数化する
  （`marker: usize` 引数を追加し、呼び出し元が明示的に渡す）。
- `ime.rs` 側の既存3呼び出し（`post_kanji_toggle_to_focused`・`send_ime_mode_key`・
  `send_ime_mode_key_with_shift_release_prefix`）は `IME_KANJI_MARKER` を渡すよう更新
  （Alt の扱いはそれぞれ現状のまま変更しない — `post_kanji_toggle_to_focused` は全解放、
  他2つは `alt: false`）。
- 「共通化は構造体とフィールドの切り出しに留め、どの修飾を解放するかは呼び出し側が
  明示する」という ADR-114 決定3の方針を守る（デフォルト全解放のヘルパーは作らない）。
- `[[keymap]]` からの呼び出し用に、同じ `output/held_modifiers.rs` へ
  `pub(crate) fn send_keymap_target(release_ctrl: bool, release_shift: bool,
  target_vk: VkCode)` を追加する（`HeldModifiers::read()` → 指定された分だけ
  `push_release` → `target_vk` の Down/Up ペアを同一 `SendInput` バッチで送信
  （`INJECTED_MARKER` 付き）→ `push_restore` を内包する）。`release_ctrl`/
  `release_shift` の由来は T4 参照——`find_match` がマッチした時点で
  `combo.ctrl == mods.ctrl`・`combo.shift == mods.shift` が成立しているため、
  マッチした `from` の combo を別途取得しなくても `event.modifier_snapshot`
  （`ModifierState`）の `ctrl`/`shift` をそのまま渡せる。T4 の `message_handlers.rs`
  はこの関数を呼ぶだけにし、送信ロジックの直書きはしない。
- `HeldModifiers` の既存テストは無い（`ime.rs` の private struct のまま新規
  テストが書かれたことがない）ので探しに行く必要はない。`send_keymap_target` 自体のテストは
  Windows 実機依存（`SendInput`）のため Linux では書けない——呼び出し条件
  （どの vk でどの修飾を渡すか）を T10 のシナリオテストで間接的に確認する。

## T3: latch テーブルの型と `Runtime`/`PlatformState` への追加

- `Vec<VkCode>`（または `HashSet<VkCode>`、実装時に選択）を保持する新しい型
  （例: `state/keymap_latch.rs::KeymapLatch`）を新設。
  - `is_latched(vk) -> bool`
  - `latch(vk)` — 既に latch 済みなら no-op として防御的に冪等にしてよい。
    **ただし repeat 判定の根拠はステップ1（T4）の `is_latched(vk)` チェックで
    あって `latch()` の戻り値ではない**——T4 のステップ1 が `is_latched` で
    先に分岐して return する設計上、ステップ2（`find_match` 照合）に到達する
    時点でその vk は必ず未 latch であり、`latch()` の冪等性は正しさの要件
    そのものではない（実装者がこれを見て「ステップ1 の `is_latched` チェックは
    不要では」と誤って単純化しないための注記）。
  - `release(vk) -> bool`（latch されていれば解放して true、なければ false）
  - `release_all()`（**テーブルを空にするだけ**。`target_vk` の KeyUp は注入しない
    — ADR-114 decision4「latch 漏れ対策」参照。ADR-110 の
    `release_all_latched_remap_targets()` とは前提が異なるので真似しない）
- `active_keymaps`（`state/platform_state.rs` の `KeymapStore` sub-struct、
  `PlatformState::keymap: KeymapStore` 経由）と同じ `KeymapStore` に
  `keymap_latch: KeymapLatch` フィールドを追加する（`PlatformState` 直下では
  ない）。`KeymapStore` は `#[derive(Debug, Default)]` のため、`KeymapLatch` にも
  `Default` を導出すれば `PlatformState::new()`側の明示初期化は不要。
  T4/T5/T6 からのアクセスパスは `app.platform_state.keymap.keymap_latch` になる。
- ユニットテスト: `latch`/`release`/`release_all`/`is_latched` の基本動作、
  二重 latch が no-op になること、を Linux で実行可能な純粋関数テストとして書く
  （BUG-100 の教訓——latch ライフサイクルはテスト必須、ADR-114 影響範囲節参照）。

## T4: `message_handlers.rs` — `deliver_key_event` への配線本体

ADR-114 決定2 の最終形（r4 収束版）に従う。

- **ステップ1（`deliver_key_event` 冒頭、ただし現状の先頭2行
  `if matches!(origin, KeyOrigin::Hook(_)) { app.platform_state.gate.
  last_hook_activity_ms = hook::current_tick_ms(); }`（`message_handlers.rs:132-134`）
  の**直後**。この値は `runtime/ime_refresh.rs:284` の keyboard idle 判定
  （GJI/Chrome long-idle 分岐の起点）に使われるため、latch チェックをこれより前に
  置くと「latch 中のキー＝実際にユーザーが打鍵中」が hook activity として
  記録されず idle 判定が誤って倒れる。ADR-114 決定2 の「一番最初」は
  「`Nested`/`NonText` 早期returnより前」の意味であり、この2行より後ろにする
  ことと矛盾しない。`Nested`/`NonText` 早期returnより前、`origin`/`focus_kind`
  を問わず必ず実行）**:
  - `let is_key_down = matches!(event.event_type, awase::types::KeyEventType::
    KeyDown);` の定義（現状 `message_handlers.rs:168`、ステップ1 の挿入位置より
    後ろにある）をステップ1 の前に移動する。既存の `consume_post_bypass(app,
    event, is_key_down)` / `cancel_composition_and_arm_post_bypass_on_ctrl(...,
    is_key_down)` 呼び出しはそのまま同じ変数を参照できるため実害はない。
  - イベントの vk が `keymap_latch.is_latched(vk)` なら:
    - **KeyUp** の場合: `keymap_latch.release(vk)` して `KeyDelivery::Consumed` を返す。
    - **KeyDown** の場合（repeat 抑制）: `find_match` を呼ばず黙って
      `KeyDelivery::Consumed` を返す（`target_vk` は再送しない）。**この分岐に
      例外を設けない**——latch が T5 の経路4（`HOOK_KEYS` overflow）で stale に
      残っていた場合でも同じロジックを適用する（ADR-114 本文の該当箇所参照）。
      「stale latch を検出して上書きし新規扱いにする」という分岐を追加しては
      いけない——追加すると repeat 抑制と stale-latch 上書きが同じ入力
      （KeyDown + latch にエントリあり）に対して衝突する状態が復活する
      （BUG-100 と同型の設計ミス）。stale latch は次にその vk の KeyUp が来た
      時点で `release()` されて自然に解消する。
  - latch されていない vk はこのステップでは何もせず素通り。
- **ステップ2（既存の `NonText` パススルー早期returnの後、`consume_post_bypass` の
  前）**: KeyDown のみを対象に、現在の `active_keymaps.find_match(vk,
  event.modifier_snapshot)` を呼ぶ（`mods` は `event.modifier_snapshot`
  （`ModifierState`）——フック時点でキャプチャした修飾キー状態スナップショット）。
  - `None`（マッチなし）: 何もせず後続へ。
  - `Some(None)`（消費のみ）: 元の KeyDown を consume。`keymap_latch.latch(vk)`。
    `KeyDelivery::Consumed` を返す。
  - `Some(Some(target_vk))`: 元の KeyDown を consume。T2 の
    `send_keymap_target(event.modifier_snapshot.ctrl, event.modifier_snapshot.shift,
    target_vk)` を呼ぶ（`find_match` がマッチした時点で `combo.ctrl == mods.ctrl`
    ・`combo.shift == mods.shift` が成立しているため、マッチした `from` の
    combo を別途取得する必要はない）。`keymap_latch.latch(vk)`。
    `KeyDelivery::Consumed` を返す。
- 挿入位置の正確な行番号は実装時に `deliver_key_event` の現状コードを再確認して
  決める（ADR-114 内の行番号引用は執筆時点のものであり実装時にズレている可能性がある）。

## T5: latch 漏れ対策（`release_all()` の呼び出し配線）

ADR-114 決定4「latch 漏れ対策」経路3・5に対応する3箇所すべてに
`keymap_latch.release_all()` の呼び出しを追加する。

- `crates/awase-windows/src/runtime/focus_tracking.rs:472` 付近
  （`hook::clear_hook_latches_for_app_disable(transition)` の呼び出し地点、
  `FOCUS_APP_DISABLED` 遷移時）。
- `crates/awase-windows/src/runtime/message_handlers.rs:831`
  （`WTS_SESSION_UNLOCK` ハンドラ、`hook::reset_physical_key_state()` の直後）。
- `crates/awase-windows/src/runtime/mod.rs:1633` 付近（panic reset 経路、
  `hook::reset_physical_key_state()` の直後）。
- 経路4（`HOOK_KEYS` overflow）は明示的に対処しない（ADR-114 decision4 の
  「残存リスクとして受容」を実装コメントとして残す）。**T4 ステップ1 の repeat
  抑制ロジックに、この経路専用の分岐を追加しない**（T4 参照——latch 存在＝
  repeat という判定を経路4 にも例外なく適用することで、BUG-100 型の
  恒久固着ではなく「次の1打鍵だけが消える」に残存リスクを閉じ込める。実装
  コメントには「latch が vk 単位で毎回の KeyUp で必ず解放される設計により、
  overflow 由来の stale latch は BUG-100 のような恒久固着にはならない」と
  書く）。

## T7: `reload_config()` の `all_keymaps` 再構築 + `active_keymaps` 再計算の一本化（decision8）

**T6（衝突検出）は T7 の成果物（`active_keymaps` 再計算の一本化されたヘルパー）に
依存するため、T6 より先に実施する。**

- `app/mod.rs::reload_config()` から `apply_config_update` 経由で
  `Runtime::all_keymaps` を `KeymapTable::new(&config.keymaps, ...)`（T1b のシグネチャ
  変更後の形。親指 vk は `hook::thumb_vk_codes()`、T1b が定義するシグネチャ拡張の続き）で差し替える。
  **`KeymapTable::new` の再構築と `recompute_active_keymaps()` の呼び出しは、
  `resolve_thumb_key(...)` の if-let ブロック（`runtime/mod.rs:1472-1476`）の
  **外・後**に置く**——ブロック内に置くと `else`（`runtime/mod.rs:1553` 付近
  "Invalid thumb key names" 警告）に落ちたときに親指 vk が確定せず reload が
  丸ごとスキップされる。`hook::thumb_vk_codes()` は値の入手性を解決するだけで
  配置までは強制しないため、この配置指示を独立に守る必要がある。
- **`active_keymaps` の再計算ロジックを `Runtime::recompute_active_keymaps()` として
  1箇所に集約する**（`filter_active` 呼び出し + T6 で追加する衝突検出/警告を内包）。
  現状 `active_keymaps = all_keymaps.filter_active(&process_name)` は
  `focus_tracking.rs::enter_focus_scope`（`focus_tracking.rs:146-167`、
  「コードレビュー指摘9」で2箇所の手動コピーを統合した経緯を持つ関数）の中に
  インライン化されている。ここを `recompute_active_keymaps()` 呼び出しに置き換え、
  `reload_config` 経路（T7 本体）からも同じ関数を呼ぶ。**書き込み点を2つに
  増やさない**（T6 の追加フィルタ/警告を両方に入れる際の片方忘れを避ける、
  `enter_focus_scope` 自身がまさにこの種の重複を統合した前例）。
- **latch は reload に対して安全**（ADR-114 decision8 で確認済み——latch は vk 単位で
  ルール参照を持たないため）なので、reload 時に latch をクリアする必要はない。

## T6: `[[keymap]]` と実行時依存キーの衝突検出（ADR-114 未解決の疑問4・5）

T7 で新設した `Runtime::recompute_active_keymaps()` を拡張する形で実装する。

- **`muhenkan_solo_tap_dedicated_fn_key`（GJI 専用 Fn キー、実行時に確定）**:
  `recompute_active_keymaps()` 内で、現在の `Runtime` が保持する dedicated fn key
  の vk と `[[keymap]]` の `from`/`to` が一致するルールを検出し `log::warn!` する
  （skip はしない——config 上は妥当なルールで、実行時に偶然衝突しているだけの
  ため。挙動は「両方のロジックが同じ物理キーに反応する」ことを警告するに留める、
  実装時に skip すべきか再検討してもよい）。**ただし dedicated fn key が変わる
  タイミングは「フォーカス変更」でも「reload_config」でもなく、
  `Runtime::set_muhenkan_dedicated_fn_key_auto`（`runtime/mod.rs:1230`、
  `handle_wm_gji_charset_fn_key_activated` から config1.db 書き込み成功後に呼ばれる）
  と `set_muhenkan_dedicated_fn_key_config`（`runtime/mod.rs:1217`）でも変わる。
  この2つの setter からも `recompute_active_keymaps()` を呼ぶこと**
  （呼ばないと次のフォーカス変更まで衝突が検出されない）。
- **`engine_toggle_hotkey`/`special_keys.ime_toggle` とのコンボ衝突**: 同じキー
  コンボの `[[keymap]]` ルールが存在する場合に警告ログを出す独立関数
  `warn_on_engine_hotkey_collision(&[KeymapRule], &SpecialKeyCombos)` として実装し、
  **`app/mod.rs:647`（`apply_config_update` 呼び出し直後、reload 経路）と
  `bootstrap.rs:958`（`KeymapTable::new` 呼び出しの隣、起動経路）の両方**から呼ぶ。
  `apply_config_update` の呼び出し元は `app/mod.rs:647` の1箇所のみで、起動時は
  `bootstrap.rs` が直接フィールドを設定する別経路のため、両方に置かないと
  「config.toml を手書きして衝突を書いたユーザーは起動時に何も言われず、設定画面
  から reload したときだけ警告が出る」という非対称が生じる。
  `sync_ime_toggle_auto_detect`（`message_handlers.rs:696`）によるレジストリ由来の
  自動追加分は**対象外と明記する**（`sync_ime_toggle_auto_detect` 自体が
  `sync_ime_kind_from_observation` から起動後の任意タイミングで呼ばれるため、
  そこからも衝突検出を呼ぶ設計はスコープが広がりすぎる。将来必要になれば
  別タスクで拡張する）。
- 上記2つは「警告のみで動作は変えない」スコープにする（ユーザー設定を勝手に
  無効化しない）。ADR-114 の「未解決の疑問」に残っている通り、将来 skip 化する
  かどうかは別判断。

## T8: 設定 GUI — `new_keymap_from_alt` チェックボックスの削除

- `crates/awase-settings/src/main.rs` の `new_keymap_from_alt` は **7箇所すべて**を
  削除する（1箇所だけ消すと GUI からは見えないまま Alt 修飾付きルールが生成され
  T1b の warn+skip と組み合わさって「GUI で作れるのに保存すると効かない」状態に
  なる）:
  - L304: フィールド定義（`new_keymap_from_alt: bool,`）
  - L397: 初期化（`new_keymap_from_alt: false,`）
  - L1036: キーキャプチャ時の代入（`self.new_keymap_from_alt = alt;`）
  - L1571: チェックボックス UI（`ui.checkbox(&mut self.new_keymap_from_alt, "Alt")`）
  - L1610: ルール生成時に引数として渡している箇所
  - L1629: 入力欄リセット（`self.new_keymap_from_alt = false;`）
  - L3750: テスト用の初期化
- Alt 修飾での `[[keymap]]` 作成を GUI からできないようにする（T1b のバリデーションと
  対称にする——GUI で作れてしまうと config 手書き時との非対称が生まれる）。
- 既存の `[[keymap]]` エディタの他の部分（Ctrl/Shift チェックボックス等）は変更しない。

## T9: `docs/known-bugs.md` への記録（ADR-114 未解決の疑問1）

- `[[post_bypass]]` の `reload_config` 未対応ギャップを新規 known-bugs エントリとして
  記録する（本 ADR 実装と同時に行う、と決定）。症状・再現手順・
  「本 ADR で `[[keymap]]` 側は直したが `[[post_bypass]]` 側は対象外」という経緯を書く。

## T10: テスト — シナリオ・順序固定

**実質的な主戦力は T1b（禁止 VK 検証、7ケース）と T3（latch ライフサイクル）——
どちらも純粋関数として Linux で全数実行できる。** T10 はそれに加えるシナリオ層。

- クロスプラットフォームな `crates/awase-windows/tests/golden_scenarios.rs` /
  `tests/golden/` に、`[[keymap]]` の KeyDown→target 送信、KeyUp latch 解放、
  repeat 抑制、`[[post_bypass]]` との優先順位（同じコンボが両方に設定された場合
  `[[keymap]]` が勝つ）を確認するシナリオを追加する。**`ime_key_sequence_golden.rs`
  は `#![cfg(windows)]` で Linux では動かないため対象外**（golden の期待値追加先を
  誤ると CI の Linux ジョブでは何も検証されない）。
- `deliver_key_event` の新しい早期return順序（latch チェック → Nested → NonText →
  keymap 新規照合 → post_bypass → NICOLA）そのものは `deliver_key_event` が
  `&mut Runtime` を要求し `FocusKind`/`SendInput` 等 Windows 依存が強いため、
  golden での直接検証は難しい可能性が高い。**実質的には T11 と同じ
  `architecture_guard.rs` 型のソーステキスト走査（`extract_fn_body` で本体の
  出現順を検査）になる公算が高いことを見込んでおく**（golden で書けると想定して
  着手すると詰まる）。
- **テストで検知できない項目を明記しておく**（レビュー時に「テストが無いから
  軽視してよい」と誤解されないため）:
  - T4 の `last_hook_activity_ms` 更新位置（`message_handlers.rs:132-134` の直後に
    latch チェックを置く）— `runtime/ime_refresh.rs:284` の idle 判定への間接的な
    副作用であり、ユニットテストにも golden にも現れない。
  - T7 の rebuild 配置指示（`resolve_thumb_key` の if-let ブロックの外・後に置く）
    — 実装者がこの指示を無視してブロック内に置いてしまってもコンパイルは通り、
    親指キー名が不正という通常踏まない前提条件でしか症状が出ないため、テストで
    機械的に守れない。

## T11: `architecture_guard.rs` の既存テキスト検査への追随（ADR-114 影響範囲注記）

- `deliver_key_event_nontext_early_return_excludes_ime_off_rescue_replay` と
  `key_events_reach_engine_only_via_deliver_key_event` を読み、T4 の新しい早期return
  （latch チェック）が想定と食い違わないか確認する。食い違う場合はテスト側の期待値を
  更新する（本文検査の対象範囲を「`deliver_key_event` と文書化済みの例外」に latch
  チェックを含める形で更新）。

## チェックリスト（T12。コミット単位のタスクではなく、節目ごとに回す確認事項）

- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`
- `cargo clippy --target x86_64-pc-windows-msvc -p awase -- -A clippy::cargo`
- `cargo test --lib` / `cargo nextest run --workspace --lib`（Linux で実行できる範囲）
- `cargo fmt -- --check`
- `cargo machete`（dead-dependency チェック、T2 の切り出しでファイル移動がある場合）
- Windows 実機での動作確認はこのセッションでは実施できない（CI の `windows-build`
  ジョブに委ねる）。
- **実装完了時のドキュメント更新**（3箇所すべて、更新漏れがあると次のセッションが
  「まだ確定していない設計」と誤読する）:
  - `docs/adr/114-keymap-app-scoped-shortcut-wiring.md` — ステータスを
    「ドラフト（Opus 敵対的レビュー中）」から実装済みへ更新。
  - `docs/adr/index.md` — ADR-114 の行のステータス列を「設計確定（未実装）」から
    実装済みへ更新。
  - `docs/adr/114-implementation-tasks.md`（本ファイル）— 冒頭のステータスを更新。
- **今回のスコープに含めないもの（明記しておき、実装時に規約違反ではと迷って
  手が止まらないようにする）**:
  - `CHANGELOG.md` — `release-develop-to-main` スキルがリリース時に一括更新する
    運用のため、実装 PR での更新は不要。
  - `docs/experiments.md` — `.claude/rules/experiment-logging.md` は revert
    コミット向けの規約であり、本 ADR の実装（revert ではない）は対象外。

## 実装順の依存関係（要約）

```
T1a/T1b (keymap.rs 基盤) ─┬─→ T4 (配線本体) ─→ T5 (漏れ対策) ─→ T7 (reload + recompute集約) ─→ T6 (衝突検出)
T2 (HeldModifiers) ──┘        │
T3 (latch 型) ────────────────┘
T8 (GUI) は T1a/T1b と並行可
T9 (known-bugs) は独立、いつでも可
T10/T11 (テスト) は T4〜T6 の後
T12 (最終確認) は最後
```

（T7 の見出し番号は T6 より小さいが、`recompute_active_keymaps()` を T7 で新設し
T6 がそれを拡張する関係のため、実装順は T7 → T6。番号はタスク文書内の記載順を
維持するために振り直していない。）
