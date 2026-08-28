# ADR-110: 物理キー単純リマップ機能（`key_remap`）

## ステータス

**提案（未実装、2026-08-28）。改訂版（r3）。** 秀Caps（`hide.maruo.co.jp`）の
機能調査から着想。r1 を Opus に敵対的レビューさせ blocking 3件・should-fix
7件（F1〜F10）を反映して r2 に改訂。r2 を同じレビュアーに再レビューさせた
ところ、設計方針の転換は不要だが blocking 4件・should-fix 3件・
worth-a-note 3件（R1〜R10）が見つかり、r3 で全て反映した
（決定2 の latch 設計を VK インデックス方式に作り直し、決定3 に
`toggle_caps_lock` の自己注入化を追加、決定4/7 に Win 系 VK 禁止を追加、
決定5 に `modifier_snapshot.ctrl` 補正を追加、決定7 に `[[keymap]]` 未実装
ラベルを追加）。**r3 の3回目レビューは応答が大幅に遅延（過去2回は各15〜20分
程度のところ、状態確認への返信も含め1時間以上応答なし）したため、ユーザー
判断により3回目の確認は断念し、r3 を実装対象の最終版として確定した
（2026-08-28）。** r1→r2→r3 で opus-adversarial-consult スキルによる
2周の敵対的レビューを経ており、r2→r3 では設計方針の転換を要する指摘は
出ていない。実機検証は未実施。

## コンテキスト

### 要望の経緯

秀Caps の機能一覧を調査した際、「英数キーを CapsLock キーにする」「無変換/変換/
右Alt/右Ctrl キーを他のキーに割り当てる」といった、物理キー1つを恒久的に別の
キーとして扱う一般的なキーリマップ機能があった。この議論の中で要望は次のように
拡張された:

1. 英数キー/CapsLock キーを任意のキー（Ctrl 等）にリマップしたい
2. 逆に Left Ctrl を英数キー/CapsLock を含む任意のキーにリマップしたい
   （CapsLock⇔Ctrl の入れ替えのような一般的な要望をカバーするため）
3. 上記を一般化した「任意の物理キー→任意の物理キー」の汎用テーブルにしたい
   （アプリケーションごと、もしくはグローバルに）

一方で「特定のショートカットに対して一定の連続打鍵（打鍵列）を割り当てる」
というマクロ的な機能も同時に提案されたが、これは 2026-07-22 の設計討議
（[[project_macro_ai_companion_design]]、ADR 未作成・実装未着手）で既に
「トリガー検知はタイミングクリティカルな `awase-windows` のフックスレッドに
残すが、実行本体（rhai スクリプト評価等）は別プロセスのコンパニオンアプリに
分離する」という方針が決まっている。本 ADR はその方針を変更しない。**打鍵列
マクロ機能は本 ADR のスコープ外**とし、上記メモリの続きとして別途 ADR 化する。

### 既存の類似機構1: Alt なりすまし（採用する下敷き）

`crates/awase-windows/src/state/alt_impersonation.rs` は、`left_thumb_key`/
`right_thumb_key` に `"Left Alt"`/`"Right Alt"` を指定すると、`WH_KEYBOARD_LL`
フックの最初期で Alt の vk を無変換/変換相当に書き換え、以後の全パイプラインに
新しい vk として流す（Relay モードが全キーを consume・reinject するため、OS
には物理 Alt イベントが一切渡らず、代わりに書き換え後の vk が SendInput で
再注入される）。この「フック最初期での vk 書き換え＋常時 reinject」という
メカニズムは既に本番で動作実績があり、修飾キー（Alt）を丸ごと別のキーとして
扱う今回の要望と技術的に同じパターンである。

`decide_alt_impersonation`（純粋関数）は「新規押下時点でのみ判定し直し、
押しっぱなし（auto-repeat）・KeyUp は新規押下時点の判定を保持する」という
hold-state 安定化パターンを持つ。これは BUG-41（押しっぱなし中にエンジン
ON/OFF が切り替わるとなりすましフラグが stuck true になる）の再発防止策で、
「なりすまし条件（エンジン ON/OFF）が同一の物理押下セッション中に変化しうる」
という前提に基づく。

### 既存の類似機構2: `[[keymap]]`（`KeymapRule`）— 配線が途切れている

`src/config.rs` の `KeymapRule { app, from, to }` と
`crates/awase-windows/src/keymap.rs` の `KeymapTable` は、「特定アプリで
特定のキーコンボ（例: `Ctrl+I`）をインターセプトし、別のキーに置き換える」
という、まさに「ショートカット再割当て」機能として実装されている。設定 GUI
（`awase-settings/src/main.rs` の `keymap_new_grid` 周辺、capture button 付きの
編集フォーム）まで完成している。

**しかし調査の結果、`KeymapTable::find_match` はコードベースのどこからも
呼ばれていない**（`grep -rn find_match` でヒットするのは定義箇所のみ）。
`active_keymaps` はフォーカス変更時に `filter_active()` で更新される
（`runtime/focus_tracking.rs`）が、その結果を実際のキー処理で参照する箇所が
存在しない。つまり `[[keymap]]` は設定・GUI・コンパイルまでは機能するが、
**実際のキー傍受は行われていない**（動作しない）。これは本 ADR の対象では
ないため深追いしないが、後日 `docs/known-bugs.md` に記録し別途調査すべき
発見として残す。

この事実は設計判断に直結する: `[[keymap]]` は「アプリ文脈を持つコンボ
インターセプト」を志向しており、そのアプリ文脈判定（`active_keymaps`）は
**フォーカス追跡（メインスレッド、`runtime/focus_tracking.rs`）に依存する**。
一方、物理キーの恒久的な役割入れ替え（今回の要望）は、CapsLock の OS ロック
状態トグルを未然に防ぐ必要があり（後述）、`WH_KEYBOARD_LL` フックの最初期
（フォーカス文脈を持たない、専用フックスレッド上）で vk を書き換える必要が
ある。**この2つは技術的に異なるレイヤーで解決すべき別機能である。**

### CapsLock のロック状態トグルについて

CapsLock の点灯状態（キーボード LED・OS のロック状態）は、OS がキーボード
入力を受理した時点で更新される。`WH_KEYBOARD_LL` はこの受理より前段でイベント
を横取りできるため、Relay モードで元の VK_CAPITAL イベントを一切 OS に渡さず
別の VK を reinject すれば、CapsLock は物理的にもロック状態的にも一切トグル
しない。英数キー（`VK_DBE_ALPHANUMERIC`）についても同様に、IME がこのキーを
一切観測しなくなる。**（r2 追記: これは決定1の挿入点に実際に到達した場合の
話であり、到達しない早期分岐が存在することが Opus レビュー F1/F2 で判明した。
詳細と対策は決定1の r2 追記・決定3を参照。）**

### アプリ別スコープを今回見送る理由（r2: 根拠を訂正）

r1 は「`WH_KEYBOARD_LL` の専用フックスレッドはレイテンシ予算の都合上フォーカス/
アプリ判定を行わない設計になっている（`docs/layer-boundaries.md`）」を根拠にして
いたが、**この根拠は誤り**（Opus レビュー F4）。`docs/layer-boundaries.md` に
フックスレッド・レイテンシ予算・フォーカス判定禁止の記述は存在しない。実際、
`FOCUS_APP_DISABLED`（`hook.rs`、`set_focus_app_disabled`）はメインスレッドの
フォーカス追跡結果を lock-free atomic 経由でフックスレッドに渡すパターンが
既に本番稼働しており、`KeymapTable::filter_active(&process_name)` も
`runtime/focus_tracking.rs` からフォーカス変更ごとに呼ばれている。したがって
「アプリ別に絞り込んだテーブルを `CACHED_KEY_REMAPS` へ store するだけ」という
形でアプリ別スコープを持たせることは、フックスレッド側のコードを一切変えずに
技術的に可能である。

**本当の理由は決定2（F1/F2 対応）と同根の非対称性リスクである**: アプリ別
テーブルにすると、物理キーを押している最中にフォーカスが変わっただけで
適用対象テーブルが入れ替わり、down 時点の reinject 先と up 時点の reinject
先が食い違いうる。これは config reload よりずっと高頻度に起こる（Alt+Tab で
対象外アプリへフォーカスが移るだけで発生する）ため、決定2の hold-state 保護を
アプリ別スコープと整合させるには「押下開始時点のアプリ」を押下セッション全体で
固定する追加の状態管理が必要になり、単純リマップの範囲を超える。
**今回はグローバル（全アプリ共通）のみ実装し、アプリ別スコープはこの
非対称性リスクの解決策と合わせて再検討する future work とする。**

## 決定1: フック最初期での vk 書き換え + 常時 reinject（Alt なりすましと同じ機構）

`key_remap` は Alt なりすましと同じ位置（`hook_callback` 内、
`apply_alt_impersonation` の直後）で適用する。対象 vk が設定テーブルに
あれば reinject 用の vk を書き換え、以後の全パイプライン（Ctrl 消費追跡・
`classify_key`・NICOLA エンジン・Relay の reinject）が新しい vk をそのまま
使う。エンジンの有効/無効に関わらず**常時**適用する（秀Caps・PowerToys 等の
一般的なリマップツールと同じ「常時そのキーとして振る舞う」設計。Alt なりすまし
がエンジン ON 時のみ発動するのとは異なる軸の機能であるため独立実装とする）。

**r2 追記（Opus レビュー F1 で判明した既知の残存リスク）**: `hook_callback`
には、この挿入点（`apply_alt_impersonation` の直後）より**手前**で早期
`CallNextHookEx`（生イベントを無変換で OS へ渡す）する分岐が複数ある
（`FOCUS_APP_DISABLED` チェック・`HOOK_KEYS` overflow ラッチチェック）。
同一の物理キー押下の KeyDown と KeyUp が、この早期分岐の前後で**異なる**
経路を通ると（例: KeyDown 時は通常経路で書き換え・reinject され、その後
`disable_apps` 対象アプリへフォーカスが移り、KeyUp 時は早期分岐で生のまま
OS に渡る）、OS 側で reinject 先の vk が「押されたまま」残る（stuck
modifier）。**この構造的な穴は `key_remap` 固有ではなく、既存の Alt なりすまし
（`apply_alt_impersonation` も同じ挿入点にある）にも同一の形で存在する**
（Alt+Tab で `disable_apps` 対象アプリへ Alt を押しっぱなしのまま切り替える、
という現実的な操作で発生しうる）。本 ADR はこの共有の構造的リスクを新規に
作るわけではないが、`key_remap` の主要ユースケース（CapsLock→Ctrl 等、通常の
タイピング中に長時間ホールドされうる修飾キー）ではリスクの発生機会が Alt
なりすまし（IME ON 時の親指キー用途、短時間の押下が主）より多いと考えられる
ため対処する。**r3 追記**: r2 では「config reload・overflow ラッチ由来の
食い違いのみ閉じ、`FOCUS_APP_DISABLED` 由来は本 ADR のスコープ外」として
いたが、Opus レビュー R3 の指摘（「早期分岐の手前で latch 済みキーの KeyUp
を注入する」という数行の対策で閉じられるはずなのに検討せず諦めている）を
受けて撤回した。決定2 r3 追記の `cleanup_latched_remap_before_bypass` で
`FOCUS_APP_DISABLED`・overflow ラッチの**両方**を、早期分岐自体を書き換える
規模の改造なしに閉じる（詳細は決定2参照）。閉じられずに残るのは awase
プロセス自体の異常終了時のみで、これは Alt なりすましとも共有する別軸の
限界として未解決の疑問4に記録する。

## 決定2: hold-state は VK インデックスの latched-target 配列で持つ（r3: 設計を作り直し）

**r2 の決定2（`decide_simple_remap(..., was_down, was_remapping, ...)` の
bool ペア + `target_vk` 引数渡し）は撤回する。** Opus レビュー r2 ラウンドの
R1/R2 で、この設計自体が意図した保護を提供しないことが判明した:

- **R1**: `target_vk` を毎回の呼び出しパラメータで渡す設計だと、config
  reload で `to` が変わった（または `from` エントリごと削除された）瞬間、
  KeyDown で latch した「なりすまし中」フラグはそのままなのに、KeyUp 側の
  `target_vk` は**新しい値**を渡してしまう。結果、古い `to`（例: LCtrl）の
  down は release されず、新しい `to`（例: LShift）の up が誤って注入される
  ——決定2 が防ぐはずだった BUG-41 型破損がそのまま再現する。
- **R2**: hold-state をテーブルの**スロット位置**（`[(AtomicBool, AtomicBool);
  MAX_KEY_REMAPS]`、決定8のダブルバッファのスロットと1:1対応）で持つ設計も、
  config reload でエントリが増減・並び替わるとスロット番号の意味が変わり、
  別の物理キーの hold-state を誤って引き継ぐ。

正しい設計は、**hold-state を「ルールのスロット」ではなく「物理 VK」で
インデックスし、bool ではなく解決済みの target VK 自体を latch する**こと
（Opus レビューの提案をそのまま採用）:

```rust
/// `state/key_remap.rs`
/// `LATCHED_TARGET[vk.0 as usize]`: 0 = このvkは現在リマップ中でない。
/// 非0ならその値が現在有効な reinject 先 vk（新規押下時点で確定し、
/// KeyUp まで固定される）。`PHYSICAL_KEY_STATE` と同じ 256 要素配列。
pub const LATCH_TABLE_SIZE: usize = 256;

/// 新規押下時点でのみ `configured_target` を読み直し、以後の auto-repeat/
/// KeyUp は latch された値をそのまま使う。KeyUp は必ず latch を 0（非リマップ）
/// にクリアする（`decide_alt_impersonation` と同じ「KeyUp 後は必ず false」
/// 不変条件、BUG-41 対策そのもの）。
///
/// 戻り値: `(実際に使う vk, 次に latch すべき値。0 なら非リマップ)`
#[must_use]
pub const fn decide_simple_remap(
    original_vk: VkCode,
    is_keydown: bool,
    latched_target: u16,     // 呼び出し元が LATCHED_TARGET[original_vk] から読む
    configured_target: u16,  // 現在のテーブルの from=original_vk の to（無ければ0）
) -> (VkCode, u16) {
    let is_fresh_press = is_keydown && latched_target == 0;
    let effective_target = if is_fresh_press {
        configured_target
    } else {
        latched_target
    };
    let vk = if effective_target != 0 {
        VkCode(effective_target)
    } else {
        original_vk
    };
    let next_latch = if is_keydown { effective_target } else { 0 };
    (vk, next_latch)
}
```

`was_down` は不要になる（`latched_target == 0` 自体が「新規押下」の判定を
兼ねる——ある物理キーが押されっぱなしの間は latch が非0のまま維持され、
離された瞬間に必ず0へ戻る）。これにより:

- **config reload 中の押下**: KeyDown 時点で latch された `to` がそのまま
  KeyUp まで使われ続けるため、reload で `to` が変わっても・エントリが
  削除されても影響を受けない。R1 が指摘した破損は解消する。
- **決定8（ダブルバッファ）とスロット番号を対応させる必要がなくなる**:
  latch は VK 値そのものをキーにするため、テーブルの並び替え・増減とは
  無関係。R2 が指摘した破損は解消する。

### overflow ラッチ・`FOCUS_APP_DISABLED` 由来の食い違いも閉じる（r3、Opus レビュー R3 対応）

r2 は「overflow ラッチ中の押下は hold-state 参照で閉じる」としていたが、
Opus レビュー R3 の指摘どおり誤りだった: overflow ラッチ・
`FOCUS_APP_DISABLED` の早期分岐は「swallow（`LRESULT(1)`）」か
「`CallNextHookEx`」の2択しかなく、hold-state をただ参照するだけでは
**latch 済みの `to` の KeyUp を OS へ注入する動作そのものがどこにも無い**
ため、いずれを選んでも stuck modifier は解消しない。

r3 では、この2つの早期分岐**両方の入口**に、以下の後始末を追加する
（Opus レビューが提案した「早期分岐の手前で latch 済みキーの KeyUp を注入する」
案を採用し、決定1 r2 追記で「スコープ外」としていた判断を撤回する。
数行の追加で閉じられる範囲を「スコープ外」のままにする理由がないため）:

```rust
/// FOCUS_APP_DISABLED / overflow ラッチの早期リターン直前で呼ぶ。
/// このvkが現在リマップ中（latch != 0）かつ KeyUp なら、latch 済みの
/// target の KeyUp を先に注入してから latch をクリアする。
/// KeyDown の場合は何もしない（disabled 中は新規リマップを一切開始しない
/// ——決定1 の「常時適用」は awase が制御を持っている間に限る、という
/// 既存の disable_apps/overflow の設計思想と整合させる）。
fn cleanup_latched_remap_before_bypass(vk: VkCode, is_keydown: bool) {
    if is_keydown {
        return;
    }
    let latched = LATCHED_TARGET[vk.0 as usize].swap(0, Ordering::AcqRel);
    if latched != 0 {
        // INJECTED_MARKER 付きで SendInput（自己注入として素通しされ、
        // 無限ループにならない。既存の is_self_injected と同じ仕組み）。
        inject_key_up(VkCode(latched));
    }
}
```

これにより、CapsLock を押しっぱなしのまま `disable_apps` 対象アプリへ
フォーカスが移り、そこで指を離しても、離した瞬間に「注入済み Ctrl の up」が
先に送られてから素の CapsLock up が `CallNextHookEx` で通る。**決定1 r2
追記・決定2（r2版）が「本 ADR のスコープ外の既知残存リスク」としていた
`FOCUS_APP_DISABLED` 由来の stuck modifier は、r3 でこの範囲について閉じる。**

ただし、この後始末は「latch 済みキーの KeyUp が early-return 経路に
到達すること」に依存する。`HOOK_KEYS` のオーバーフロー自体（=
`passthrough_or_swallow_for_impersonation` に到達する前の別の早期リターン）
や、awase プロセス自体が異常終了した場合はこの限りではない（プロセスが
無くなれば SendInput 自体ができない）。この残余は決定1 r2 追記の対象として
`docs/known-bugs.md` に記録する範囲を「プロセス終了時のみ」に縮小する
（証拠義務・未解決の疑問参照）。

物理キー1個につき `AtomicU16`（`LATCHED_TARGET`）が1個必要。256要素の
配列（`PHYSICAL_KEY_STATE` と同型・同サイズ）で全 VK 値をカバーする。

## 決定3: CapsLock ロック状態の片道 latch 対策（Opus レビュー F2/R4/R8 対応）

決定1の主張（「Relay が全部消費するので CapsLock は一切トグルしない」）には
決定1 r2 追記と同じ穴がある: `FOCUS_APP_DISABLED`/overflow ラッチ中は
`VK_CAPITAL` が生で OS に届くため、その間に CapsLock は実際にトグルしうる。
その後 awase 側の通常経路に戻ると `from=VK_CAPITAL` ルールにより以後
`VK_CAPITAL` は常に swallow されるため、**ユーザーは CapsLock を OFF に戻す
手段を失う**（awase 終了までロックされたまま）。

対策として、以下のタイミングで OS の CapsLock ロック状態を明示的に OFF へ
正規化する:

1. **`from=VK_CAPITAL` を含むルールが有効化された時点**（起動時 config
   ロード・設定リロードで新たに有効になった場合）。
2. **`from=VK_CAPITAL` を含むルールが有効な状態で、`disable_apps`/overflow
   ラッチの解除後、awase が制御を取り戻した直後**（`runtime/mod.rs::
   apply_app_disable_transition` 等、既存の「無効化解除」処理フックに乗せる。
   **r3 追記（Opus レビュー R8 対応）: この項目にも項目1と同じ「`from=
   VK_CAPITAL` ルールが有効な場合のみ」のゲートを必ず付ける**——無条件に
   すると、`key_remap` を1件も設定していないユーザーでも RDP 等から
   フォーカスが外れて戻るたびに CapsLock が勝手に OFF になる、意図しない
   既存挙動の変更になってしまう）。

**r3 追記（Opus レビュー R4 対応）: 正規化の書き込み手段そのものに欠陥が
あった。** r2 は「既存の `hook::is_caps_lock_on()`/トレイメニュー『Caps
Lock』項目が同種の状態読み取り・操作を既に行っており、正規化用の書き込み
手段はそこから流用できる」としていたが、その書き込み手段
（`ime.rs::toggle_caps_lock()`）は `dwExtraInfo: 0` で `SendInput` しており、
`hook.rs::is_self_injected`（`INJECTED_MARKER`/`TSF_MARKER`/`IME_KANJI_MARKER`
のみを自己注入と認識）に引っかからない。つまり `toggle_caps_lock()` の
注入イベントは `hook_callback` の通常経路（`key_remap` の適用を含む）に
そのまま入り、**`from=VK_CAPITAL` ルールがある状態で `toggle_caps_lock()`
を呼ぶと、CapsLock を OFF にするはずの注入自体が `VK_LCONTROL` に
書き換えられて CapsLock は一切 OFF にならず、代わりに spurious な Ctrl
down/up だけが飛ぶ**——正規化の自己矛盾。

さらに `toggle_caps_lock()` には既存の呼び出し元が3箇所あり
（`runtime/message_handlers.rs`（トレイメニュー「Caps Lock」項目、
`tray.rs` 参照）・`runtime/key_pipeline.rs::kp_reset_to_hiragana_romaji_
capsoff`（IME-ON コンボの復旧処理））、`from=VK_CAPITAL` を設定した時点で
これら**既存の3機能が同時に壊れる**（クリックしても効かない・ログ上は
「OFF にした」と出るのに実際は OFF にならず spurious Ctrl タップが混ざる）。

対策として、`toggle_caps_lock()`（および決定3の正規化処理）の `SendInput` を
`INJECTED_MARKER` 付きに変更する（`is_self_injected` に自己注入として
認識させ、`key_remap`/Alt なりすまし双方の変換を経ずに素通しさせる）。
これは決定3 新設の正規化処理だけでなく、既存の3呼び出し元も同時に救う。

これにより「CapsLock が意図せず ON のまま固着する」実害は防げるが、
「その間 CapsLock が一瞬 ON になったこと自体」（キーボード LED のちらつき）は
防げない。ADR としてはこの残存挙動を許容する（起動直後・稀な focus 遷移の
瞬間のみで、実害はロック状態の残留の方であり、これは正規化で塞がれる）。

## 決定4: config 形状は `[[key_remap]] from = "..." to = "..."` の配列（r2: Alt系を禁止、r3: Win系も禁止）

```toml
[[key_remap]]
from = "VK_CAPITAL"
to = "VK_LCONTROL"

[[key_remap]]
from = "VK_DBE_ALPHANUMERIC"
to = "VK_CAPITAL"
```

- `from`/`to` は `VkCode::from_name`（`crates/awase-windows/src/vk.rs`）が
  解決できる VK 名の文字列。`VK_CAPITAL`（CapsLock、これまで未定義だったため
  本 ADR で追加）を含む。
- `app` フィールドは持たない（決定なしグローバル固定、上記「アプリ別スコープを
  今回見送る理由」参照）。`[[keymap]]` の `KeymapRule` とは型を分離する
  （コンボ判定の `ctrl`/`shift`/`alt` も持たない、単純な `from`/`to` のみ）。
- 名前解決に失敗したエントリ、`from == to`（無意味）のエントリ、`from` が
  重複するエントリ（2件目以降）は、起動時に `log::warn!` を出して無視する
  （`KeymapTable::new` と同じ「警告して skip、エラーにしない」方針を踏襲）。
- 上限 `MAX_KEY_REMAPS = 8` 件。フックスレッドがロックなしのグローバル
  atomics 経由で読む（具体的な原子性については決定8参照、r1 の単純な
  `AtomicU32` 8スロット案は撤回した）。8 を超えるエントリは警告して無視する
  （8 件あれば今回の要望——英数/CapsLock/Ctrl 数個の入れ替え——には十分
  余裕がある）。
- **`from`/`to` に Alt 系 VK（`VK_MENU`/`VK_LMENU`/`VK_RMENU`）を指定する
  ルールは、名前解決の成否に関わらず `compile_key_remaps` で拒否し
  warn+skip する（r2 新規、Opus レビュー F8 対応）**。理由: Alt なりすまし
  （`state/alt_impersonation.rs`）は `is_alt_impersonation_active()` を介して
  `modifier_snapshot.alt` を強制補正する専用ロジックを持っており、これは
  `hook_callback` 内での実行順序（`apply_alt_impersonation` の**後**に
  `key_remap` が適用される、決定1参照）に強く依存している。`key_remap` で
  Alt を source/target にすると、この補正ロジックの対象外になり
  `is_os_modifier_held` バイパス判定が壊れる（`alt_key_held()` が
  `PHYSICAL_KEY_STATE` ベースで remap を認識しないため、注入した Alt が
  OS レベルで押されたままになる一方 awase 内部では「Alt は押されていない」
  と誤認し、BUG-61/BUG-62 で「復旧不能」と確定済みの入力方式切替を誘発しうる、
  Opus レビュー詳細参照）。Ctrl 系（`VK_CONTROL`/`VK_LCONTROL`/`VK_RCONTROL`）
  は禁止しない — こちらは決定5のトラッキング修正で対応する。
- **`from`/`to` に Win 系 VK（`VK_LWIN`/`VK_RWIN`）を指定するルールも同様に
  禁止する（r3 新規、Opus レビュー R5 対応）**。禁止理由は Alt と全く同型:
  `hook.rs::win_key_held()` も `alt_key_held()` と同じ `PHYSICAL_KEY_STATE`
  ベースの held 判定であり、`tsf/send.rs::send_eager_warmup_vk_pair` と
  `ime.rs::send_ime_mode_key` の両方がこの判定点を頼りに「Win 押下中は
  IME モードキー送信を抑制する」（BUG-48 対策）。`key_remap` で Win を
  source/target にすると、この抑制が空振り（`to=VK_LWIN` で OS レベルの
  Win 押下中に awase が抑制せず `VK_IME_ON`/`OFF` を送ってしまう）または
  過剰発動（`from=VK_LWIN` で OS には Win が届いていないのに awase 内部では
  held と誤認し `WIN_KEY_HELD_STALE_MS` の間抑制し続ける）のいずれかになる。
  **禁止対象は個別に Alt/Win を列挙するのではなく、「`PHYSICAL_KEY_STATE`
  ベースの held 判定を持つキー全般」として一般化する**（現時点では Alt/Win
  の2系統。Shift はこの種の held 判定を持たないため対象外——決定7の
  `KEY_REMAP_KEYS` に残す）。
- `from`/`to` が `VK_KANA`・`VK_DBE_ROMAN`・`VK_DBE_NOROMAN` の場合、
  `hook_callback` 内の既存 swallow 分岐（決定1の挿入点より**前**）に
  よって、injected 時・Alt 押下時・（ROMAN/NOROMAN は既定で常時）
  無言で `key_remap` に到達しないことがある。`compile_key_remaps` は
  これらの VK を禁止はしないが、設定 GUI（決定7）でホバーテキストに
  「一部の状況で効かない場合があります」と明記する。

## 決定5: Ctrl 消費追跡（`CTRL_CONSUMED_SINCE_DOWN`）を `key_remap` 対応にする（新規、Opus レビュー F3 対応）

`hook.rs` の Ctrl 消費追跡（`CTRL_CONSUMED_SINCE_DOWN`、Ctrl+無変換 IME OFF
ミスタイプ救済窓の起点判定）と `key_pipeline.rs` の診断ログ（`phys_ctrl`、
"CTRL MISMATCH" 警告）は、いずれも `is_physical_key_down(VK_LCONTROL /
VK_RCONTROL)`（**書き換え前**の物理 VK を見る）と、reset 側の
`is_ctrl_variant(vk)`（**書き換え後**の vk を見る）という、判定基準が
食い違う2つの経路を組み合わせている。`key_remap` で Ctrl⇔他キーの入れ替えを
行うと、この食い違いが実際に踏まれる:

- **`from`=Ctrl系, `to`=非Ctrl**（例: Left Ctrl→CapsLock）: 物理 Ctrl の
  down 自体が「（書き換え前 vk 基準の）ctrl_held=true」を作り、続く
  Ctrl 消費追跡ブロックは（書き換え後の）vk が非 Ctrl なので reset 分岐を
  通らず set 分岐に落ち、**自分自身の押下**を「Ctrl 押下中に別キーが押された」
  と誤認して `CTRL_CONSUMED_SINCE_DOWN=true` を立てる。このキーは
  以後 `is_ctrl_variant(vk)` に一致しなくなるため、reset する手段がなくなる
  （右 Ctrl 等、他の real Ctrl 系キーを押すまで stuck）。
- **`from`=非Ctrl, `to`=Ctrl系**（例: CapsLock→Left Ctrl、要望2そのもの）:
  「Ctrl として振る舞っているキー」が物理的には `VK_CAPITAL` なので、
  `is_physical_key_down(VK_LCONTROL/RCONTROL)` は常に false。結果、
  救済窓（`ctrl_consumed_since_down()`）は**一切機能しなくなる**。
  診断ログの `phys_ctrl` も常に false のまま食い違い続ける。

対策: 両経路が参照する「Ctrl が物理的に held されているか」の判定を、
`key_remap` テーブルを考慮した共有ヘルパーに置き換える。

```rust
/// hook.rs と key_pipeline.rs の両方から呼ぶ、key_remap を考慮した
/// 「実効的に Ctrl が held されているか」の判定。
pub fn effective_ctrl_physically_held(key_remaps: &[(VkCode, VkCode)]) -> bool {
    is_physical_key_down(VK_LCONTROL)
        || is_physical_key_down(VK_RCONTROL)
        || key_remaps
            .iter()
            .any(|&(from, to)| is_ctrl_variant(to) && is_physical_key_down(from))
}
```

さらに reset 側の判定も「書き換え前の vk 自体が Ctrl 系だったか」を
合わせて見るよう修正する（`original_vk`/書き換え後 `vk` の両方を
`hook_callback` 内で保持しておき、`is_ctrl_variant(original_vk) ||
is_ctrl_variant(vk)` で reset する）。これにより:

- `from`=Ctrl系→非Ctrl の自己誤検出（原因は「このイベント自身が Ctrl の
  押下」なのに reset ではなく set に落ちること）は、`original_vk` 基準の
  reset 判定で解消する。
- `from`=非Ctrl→Ctrl系での救済窓の機能不全は、`effective_ctrl_physically_held`
  が `key_remaps` の `to`=Ctrl エントリを見ることで解消する。

`key_pipeline.rs` の `phys_ctrl`（診断ログ・CTRL MISMATCH 警告）も同じ
ヘルパーに置き換える。

### r3 追記（Opus レビュー R6 対応）: `modifier_snapshot.ctrl` の補正が抜けていた

上記の修正だけでは、Alt なりすましが必要とした**もう半分**——
`is_alt_impersonation_active()` → `modifier_snapshot.alt = false` 強制補正
（`hook.rs`、`decide_alt_impersonation` の doc 参照）——の鏡像が
`key_remap` 側に無かった。`from=VK_CAPITAL to=VK_LCONTROL` の場合:

1. hook は CapsLock の KeyDown を consume する。LCtrl の注入は engine スレッド
   経由の非同期 reinject（`executor.rs` の `enqueue_reinject`、
   `OUTPUT_GUARD_MS` で defer されうる）であり、hook 内で即座に SendInput
   されるわけではない。
2. その注入が実際に OS に届く前に次のキー（例: `C`）の `hook_callback` が
   走ると、`read_os_modifiers()`（`GetAsyncKeyState` ベース）はまだ Ctrl を
   観測しておらず `modifier_snapshot.ctrl = false` になる。
3. NICOLA エンジンは `C` を Ctrl 修飾なしの文字キーとして処理し、
   「Ctrl+C」ではなく通常のかな入力になる——**CapsLock を Ctrl にリマップ
   した直後の最初のショートカットが化ける**、最も目立つ症状。

Alt なりすましはこの向きの取りこぼしが起きない（`to` が無変換/変換という
非修飾キーで、`modifier_snapshot` 側で「Alt として扱う」補正が要らない）。
`key_remap` で初めて出る問題。

対策: 決定5 の `effective_ctrl_physically_held` を、`modifier_snapshot`
構築箇所（`hook.rs` 自身に加え、`is_alt_impersonation_active()` の doc が
列挙している全構築箇所——`runtime/mod.rs::build_ctx`・
`runtime/message_handlers.rs` のタイマーハンドラ——と同じ範囲）でも参照し、
`true` なら `modifier_snapshot.ctrl = true` を強制する。`is_alt_impersonation_
active()` の Alt 版補正と対になる、Ctrl 版の補正として同じ箇所に並べて
実装する。

## 決定6: reinject に拡張キーフラグを立てる（新規、Opus レビュー F5 対応）

`RawKeyEvent::reinject`（`crates/awase-windows/src/lib.rs`）は現在
`wScan=0`・`KEYEVENTF_EXTENDEDKEY` 常時なしで `SendInput` する。これは
Alt なりすましの `to`（`VK_NONCONVERT`/`VK_CONVERT`、いずれも非拡張キー）
では問題にならなかったが、秀Caps 由来の要望に明記されている「右Alt/右Ctrl
キーを他のキーに割り当てる」を `key_remap` の `to` に使うと、拡張キー
（`VK_RCONTROL`/`VK_RMENU`・矢印・Home/End/Insert/Delete・テンキー系）を
正しく再現できない可能性がある。`to` の VK が拡張キー相当（Windows の
拡張キー一覧: 右 Ctrl/右 Alt・矢印キー・Home/End/PageUp/PageDown/
Insert/Delete・NumLock・テンキー Enter 等）の場合、`KEYEVENTF_EXTENDEDKEY`
を立てるよう `reinject` を修正する。この修正は Alt なりすましとも共有される
経路であり、既存の非拡張キー用途（無変換/変換）には影響しない
（`KEYEVENTF_EXTENDEDKEY` を条件分岐で追加するだけで、対象外の VK には
何も変えない）。

## 決定7: 設定 GUI は新規キー選択リストを作る（r2: 既存部品の単純再利用を撤回、r3: Win除外・未実装ラベル追加）

**r1 の「既存 `[[keymap]]` エディタの `main_key_combo`/`capture_button` を
そのまま再利用する」は撤回する。** Opus レビュー F6 で確認された事実:
`awase-settings/src/main.rs` の `KEYMAP_MAIN_KEYS`（`main_key_combo` が使う
候補リスト）には修飾キー（Ctrl/Alt/Shift/Win の左右バリアント）も
CapsLock も**1つも含まれていない**（`[[keymap]]` はコンボ UI 側で
Ctrl/Shift/Alt をチェックボックスとして別扱いするため、意図的に除外されて
いる）。したがって ADR 自身の設定例（`from="VK_CAPITAL"`、
`to="VK_LCONTROL"`、要望2の `from="VK_LCONTROL"`）は、r1 の計画どおり
既存ドロップダウンを再利用すると**GUI から1つも設定できない**。

対策として、`key_remap` 専用の新規キー候補リスト `KEY_REMAP_KEYS` を作る。
最低限含めるもの: `VK_CAPITAL`（CapsLock）・`VK_LCONTROL`/`VK_RCONTROL`・
`VK_LSHIFT`/`VK_RSHIFT`・既存の `THUMB_KEY_OPTIONS` の VK 実名エントリ
（無変換/変換/かな/漢字等）。**r3 追記（Opus レビュー R5 対応）:
`VK_LWIN`/`VK_RWIN` は決定4で禁止対象に追加したため候補から外す**。
**r3 追記（Opus レビュー R10 対応）: `THUMB_KEY_OPTIONS` をそのまま
`.chain()` で取り込んではならない**——このリストには `"Left Alt"`/
`"Right Alt"` という、`VkCode::from_name` では解決できない専用センチネル
文字列（`state/alt_impersonation.rs::resolve_thumb_key` だけが特別扱いする）
が混在している。これを `KEY_REMAP_KEYS` にそのまま含めると、ユーザーが
GUI からこれを選んで `from`/`to` に書き込んだ場合、`compile_key_remaps`
は「名前解決失敗」として skip する（動作結果は正しく禁止されるが、GUI の
⚠ ホバー理由が「名前解決失敗」になり、本当の理由（Alt 系は禁止、決定4）が
伝わらない）。`KEY_REMAP_KEYS` を構築する際は `THUMB_KEY_OPTIONS` から
`"Left Alt"`/`"Right Alt"` の2エントリを明示的に除外する。

ドロップダウン（`ui.selectable_label` ベース）で選択する形にし、
**capture button は当面付けない**——`capture_button`/`egui_key_to_internal`
（`main.rs`）は `egui::Key` に修飾キー・CapsLock 用の variant が無いことを
コード自身がコメントで認めており、この機能の主対象キーに対して無力である
ため、キャプチャ機構の拡張（別途 raw scan-code ベースの捕捉に変える等）は
本 ADR のスコープ外の future work とし、初版はドロップダウン選択のみとする。

その他 r1 から維持する方針:
- Ctrl/Shift/Alt チェックボックスは出さない（`from`/`to` とも
  `KEY_REMAP_KEYS` ドロップダウン1個ずつ）。
- アプリ名入力欄は出さない（決定なしグローバル固定）。
- `to` は必須（「（消費のみ）」の選択肢を出さない）。
- 一覧表示中の各行に警告アイコン（`⚠`）を出し、`compile_key_remaps` が
  skip したエントリ（名前解決失敗・重複・上限超過・Alt/Win系禁止・決定9の
  衝突警告）をホバーテキストで理由表示する。
- `from`/`to` が `VK_KANA`/`VK_DBE_ROMAN`/`VK_DBE_NOROMAN` の場合、決定4の
  「一部の状況で効かない」ホバーテキストを表示する。

新しいタブは作らない。ただし決定10（`[[keymap]]` との共存）で述べる理由に
より、既存の「キーマップ」タブとは**視覚的に明確に分離**する（別
`egui::CollapsingHeader` にする等）。

**r3 追記（Opus レビュー R7 対応）**: `docs/known-bugs.md` への記録は
開発者向けであり、設定画面を見ているユーザーには届かない。既存の
「キーマップ」（`[[keymap]]`）セクションのヘッダ直下に「⚠ 現在この機能は
動作しません（キー処理から呼ばれていません）」という `ui.label` を1行
追加し、ユーザーが GUI 上で気づけるようにする。これを決定10の実装前提
条件に含める。

## 決定8: `CACHED_KEY_REMAPS` はダブルバッファで原子的に切り替える（新規、Opus レビュー F7 対応）

r1 は 8 スロット分の `AtomicU32`（1スロット = `(from<<16)|to` の1ワード）を
個別に `store` する設計だったが、テーブル全体の入れ替え（config reload）が
1スロットずつ非原子的に反映されるため、reader が新旧混在のテーブルを見る
瞬間が実在する。単発ルックアップとしては大きな実害は薄いが、CapsLock⇔Ctrl
の**相互入れ替え**（2エントリ）では「片方だけ反映された瞬間」が生じ、
決定2の hold-state 保護と組み合わせても、その瞬間に押されたキーの down/up
非対称を引き起こしうる。

対策: 静的な2面バッファ `[[AtomicU32; MAX_KEY_REMAPS]; 2]` + 現在面を示す
`AtomicUsize`（0 or 1）を持ち、`set_key_remaps` は**使われていない面**へ
全スロットを書き込んだ後、最後に面インデックスを1回 `store`（`Release`）
して切り替える。reader（`hook_callback`）は面インデックスを `Acquire` で
読んでからその面の8スロットを読む。これによりテーブル全体の入れ替えが
reader から見て単一の原子的操作になる。Ordering は `CACHED_THUMB_VKS` に
揃えて面インデックスは `Acquire`/`Release`、各面のスロット自体は書き込み
完了後にしか参照されないため `Relaxed` で構わない。

また `HookConfig` に `[(VkCode, VkCode); MAX_KEY_REMAPS]` を値として埋め込む
r1 の案（`classify_key` の公開シグネチャに現れる型のため無関係な呼び出し元・
テストに波及する）は撤回し、`apply_alt_impersonation` と同様に
`apply_key_remap()` が自分専用の atomics を直接読む形にする（`HookConfig`
は変更しない）。

## 決定9: `from` の衝突を起動時に警告する（新規、Opus レビュー F9 対応）

r1 は「`key_remap` の `to`/`from` が親指キー・IME 制御キー等と衝突しても
無警告で許容し、実機フィードバック待ちにする」としていたが、これは
初版から入れるべき小さな作業だと判断し前倒しする。`compile_key_remaps`
（または呼び出し元）で、`from` が以下のいずれかと一致する場合に
`log::warn!` を出す（既存 `validate_thumb_key_in_ime_combos` と同じ
「警告するが動作は止めない」枠に揃える）:

- 現在の `left_thumb_key`/`right_thumb_key` の VK（一致すると NICOLA の
  同時打鍵チョードそのものが機能しなくなる——既定の無変換/変換を誤って
  `from` に指定する事故を防ぐ）。
- `keys.engine_on`/`engine_off`/`ime_on`/`ime_off`/`ime_toggle` の
  ホットキーが単一 VK（修飾キーなし）の場合、その VK。
- `keys.engine_off_solo_repeat` の VK。

## 決定10: `[[keymap]]` との共存方針（r2: 理由を訂正、known-bugs.md 記録を前提条件にする）

r1 は「2機構を並立させる」判断自体を却下した代替案の節で説明していたが、
理由の一つ（F4 で誤りと判明した「フォーカス文脈を持たない専用スレッド」論）
を撤回し、以下の2点に絞る:

1. **CapsLock/英数のロック状態・IME 観測を未然に防ぐには、`[[keymap]]` の
   ようなフォーカス文脈を経由する層より前——`WH_KEYBOARD_LL` 最初期——で
   vk を書き換えなければならない。**
2. **押下セッション（KeyDown→auto-repeat→KeyUp）をまたぐ hold-state 対称性
   の要求が `[[keymap]]` の想定（単発のコンボ intercept）と異なる。**

この2点は `[[keymap]]` の延長では解決できない別軸の要求であり、2機構が
並立する設計判断自体は維持する。

**ただし r2 では以下を実装着手の前提条件とした（Opus レビュー F10 対応）**:
現状 `[[keymap]]` は設定 GUI・config parse まで完成しているのに
`KeymapTable::find_match` が一度も呼ばれておらず**動作しない**（コンテキスト
参照）。この状態のまま、隣（決定7で述べたとおり視覚的に分離するとはいえ
同じ設定画面内）に「見た目は似ているが実際に動く」`key_remap` を追加すると、
ユーザーが「`[[keymap]]` も当然効くはず」と誤認したまま気づかない事故が
起きやすくなる。**`key_remap` の実装 PR に先行して（または同じ PR で）、
`docs/known-bugs.md` に `[[keymap]]` 未配線の事実を記録すること**を
本 ADR の実装着手の前提条件とする。

**r3 追記（Opus レビュー R7 対応）**: `docs/known-bugs.md` は開発者向け
ドキュメントであり、設定画面を見ているユーザー自身には届かない。前提条件を
以下の**両方**に拡張する: (a) `docs/known-bugs.md` への記録、(b) 決定7で
述べた「キーマップ」セクション自体への「⚠ 現在この機能は動作しません」
ラベル表示。(a) だけでは r1 の F10 が要求していたユーザー向けの誤認防止を
満たさない。

## 却下した代替案

- **`[[keymap]]` の `KeymapRule`/`find_match` をそのまま拡張して使う**:
  却下。理由は決定10の2点（フック最初期での書き換えが必須／hold-state
  対称性の要求が異なる）に絞る。r1 が理由に含めていた「フォーカス文脈を
  持たない専用スレッド」は F4 で誤りと判明したため削除した。
- **打鍵列マクロ機能として一体で実装する**: 却下。2026-07-22 の設計討議
  （[[project_macro_ai_companion_design]]）で「実行本体は別プロセスに分離」と
  既に決定済みであり、本 ADR のスコープではない。
- **アプリ別スコープを今回から持たせる**: 却下（見送り）。上記「アプリ別
  スコープを今回見送る理由（r2: 根拠を訂正）」参照。理由は「フックスレッドが
  アプリ判定できない」（誤り、F4）ではなく、決定2と同根の押下中フォーカス
  変更による非対称性リスク。
- **`key_remap` を hold-state を持たないステートレスな判定にする**:
  r1 で採用していたが **r2 で撤回**（決定2参照）。前提だった「適用条件が
  変化するのは config reload のみ」が誤りで、`FOCUS_APP_DISABLED`/overflow
  ラッチという高頻度な条件変化が同じ穴を開けることが Opus レビュー F1 で
  判明した。
- **既存 `[[keymap]]` の設定 GUI 部品（`main_key_combo`/`capture_button`）を
  そのまま `key_remap` に再利用する**: r1 で採用していたが **r2 で撤回**
  （決定7参照）。`KEYMAP_MAIN_KEYS` に修飾キー・CapsLock が無く、ADR 自身の
  設定例が GUI から作れないことが Opus レビュー F6 で判明した。
- **`key_remap` の `to`/`from` に Alt 系 VK を許可する**: 却下（決定4）。
  Alt なりすましの補正ロジックとの相互作用（Opus レビュー F8）が
  `is_os_modifier_held` バイパス判定を壊し、BUG-61/62 の「復旧不能」領域に
  踏み込むリスクが高いため、初版では禁止する。
- **衝突検出（決定9）を今回は無警告で見送る**: r1 で採用していたが **r2 で
  撤回**。Opus レビュー F9 の指摘どおり、既存の枠（`validate_thumb_key_in_
  ime_combos`）に揃えるだけの小さい作業であり、初版から入れないことに
  見合う理由がなかった。
- **hold-state を `(AtomicBool, AtomicBool)` のスロット配列＋`target_vk`
  パラメータ渡しで持つ**: r2 で採用していたが **r3 で撤回**（決定2参照）。
  `target_vk` を毎回引数で渡す設計は config reload 中に down/up の `to` が
  食い違う（Opus レビュー R1）、スロット位置ベースの hold-state はテーブルの
  並び替え・削除で意味がズレる（R2）ことが判明した。VK インデックスの
  latched-target 配列（`[AtomicU16; 256]`）に置き換えた。
- **`FOCUS_APP_DISABLED`/overflow ラッチ由来の stuck modifier は本 ADR の
  スコープ外のまま `docs/known-bugs.md` に記録するだけにする**: r2 で
  採用していたが **r3 で撤回**（決定1 r2 追記・決定2参照）。Opus レビュー
  R3 の指摘どおり、早期分岐の手前で latch 済みキーの KeyUp を注入する
  という数行の対策で実際に閉じられることが分かり、これを検討せずに
  「スコープ外」と結論するのは弱いと判断した。

## 証拠義務

### (a) Linux で回せる自動テスト

- `state/key_remap.rs::decide_simple_remap`（純粋関数、決定2 r3版）の
  網羅テスト: `(is_keydown, latched_target, configured_target)` の組み合わせ
  （0/非0 で実質2値×2値×2値の8通り）と、「KeyUp 後は必ず `next_latch=0`」
  という BUG-41 型不変条件。config reload で `configured_target` が変わっても
  `latched_target != 0` の間は無視されることを明示的にテストする（R1 が
  指摘した破損の再発防止テスト）。`cargo test -p awase-windows --lib` で
  実行できる形にする。
- `compile_key_remaps`（config → テーブル変換）のユニットテスト: 名前解決
  失敗・`from == to`・重複・上限超過・Alt/Win系禁止（決定4）・
  センチネル文字列除外（決定7、R10）・衝突警告（決定9）それぞれで該当
  エントリが skip/警告され、他の正常エントリは影響を受けないことを検証する。
- `effective_ctrl_physically_held`（決定5）のユニットテスト: `key_remaps`
  に `to`=Ctrl系のエントリがある/ない場合、`is_physical_key_down` の
  モック相当（テスト用の直接呼び出し）で期待通りに held/not-held を返す
  ことを検証する。reset 側の `is_ctrl_variant(original_vk) ||
  is_ctrl_variant(vk)` も同様に両方向のケースを網羅する。
- `crates/awase-windows/tests/architecture_guard.rs` に以下を固定する
  テキスト検査を追加する（Alt なりすましとの相互作用の暗黙前提が壊れた
  ときに検知するため）:
  - `key_remap` の vk 書き換えが `apply_alt_impersonation` の**後**・
    Ctrl 消費追跡ブロックの**前**という順序。
  - **r3 追記（Opus レビュー R9 対応）**: 決定5 が依存する
    「`original_vk`（書き換え前）の捕捉が vk 書き換えより手前で行われて
    いる」という前提。この前提が無いと決定5 の `is_ctrl_variant(original_vk)
    || is_ctrl_variant(vk)` が書けない。
- 決定8のダブルバッファ切り替え（`set_key_remaps` → 面インデックス切替）が
  「新旧混在を reader に見せない」ことを検証する単体テスト（マルチスレッドで
  writer が繰り返し切替をしている間に reader が読んでも、常にどちらか
  一方の面の完全なスナップショットになることを確認する形。決定的な
  再現は難しいため、最低限「切替後に古い面のデータが混ざらない」という
  ロジックレベルの検証に留める）。
- 決定2 r3追記の `cleanup_latched_remap_before_bypass` のユニットテスト:
  `latched_target != 0` かつ KeyUp の場合のみ target の KeyUp 注入が発火し、
  KeyDown・`latched_target == 0` では何もしないことを検証する。

### (b) 自動テストで代替できないもの（記録で担保する）

- CapsLock の実機ロック状態が本当にトグルしないこと、および決定3の
  正規化（`disable_apps`/overflow ラッチ解除後・ルール有効化時、r3の
  `toggle_caps_lock` 自己注入化を含む）が実際に機能することの確認
  （Windows 実機のみ）。既存3呼び出し元（トレイメニュー・IME復旧処理）が
  `from=VK_CAPITAL` 設定下でも正常に動作することも合わせて確認する。
- 決定2 r3追記（`cleanup_latched_remap_before_bypass`）が、`disable_apps`/
  overflow ラッチ中に押下・その後解除した場合に実際に stuck modifier を
  防げることの実機再現確認。プロセス異常終了時（決定2 r3 追記で明示した
  残存リスク）は対象外。
- 設定 GUI（決定7の `KEY_REMAP_KEYS` ドロップダウン、`[[keymap]]` の
  「未実装」ラベル）・警告アイコン表示の見た目確認（Windows 実機のみ）。
- 決定5・決定5r3追記（Ctrl消費追跡 + `modifier_snapshot.ctrl` 補正）の
  修正後、実際に「CapsLock→Ctrl remap 直後の最初の Ctrl+C 等のショート
  カットが化けない」こと、および Ctrl+無変換 ミスタイプ救済窓が機能する
  ことの実機確認。

いずれも実機検証待ちのため、実装後は `docs/known-bugs.md` ではなく
このファイルの「未解決の疑問」に実機確認結果を追記する運用とする。

## 未解決の疑問（実装着手前後で確認すること）

1. **（r2 で決定済み）** `[[keymap]]::find_match` 未配線の事実は、決定10で
   実装着手の前提条件として `docs/known-bugs.md` に記録することを確定した
   （r3 で GUI ラベル表示も前提条件に追加、決定10参照）。
2. **（r2 で決定済み）** `from` の衝突検出は決定9で警告を追加することを
   確定した。
3. `MAX_KEY_REMAPS = 8` で実際に十分か。秀Caps 由来の要望（英数/CapsLock/
   右Ctrl の入れ替え、Alt/Win 系は決定4で禁止済み）なら 4 件程度で足りる
   はずだが、実機フィードバック次第で引き上げる。
4. **（r2 で提起、r3 で大部分決定済み）** `FOCUS_APP_DISABLED`/overflow
   ラッチ由来の stuck modifier リスクは、決定2 r3追記の
   `cleanup_latched_remap_before_bypass` で通常のケースは閉じた。残るのは
   「awase プロセス自体の異常終了」のみで、これは Alt なりすましも含めた
   一般的な「プロセスクラッシュ時の SendInput 済み状態の後始末」という
   別軸の問題であり、本 ADR では対応しない（既存の Alt なりすましも同じ
   限界を持つ）。
5. **（r2 新規）** 決定7で「capture button 拡張は future work」とした
   キャプチャ機構（修飾キー・CapsLock を raw scan-code ベースで捕捉する
   仕組み）を、`key_remap` のためだけに前倒しで作るべきか。初版は
   ドロップダウン選択のみで十分か、実装時にユーザビリティを見て判断する。
