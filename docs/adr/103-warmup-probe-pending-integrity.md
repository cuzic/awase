# ADR-103: Warmup/Probe 過渡期の pending 取りこぼしと FSM 整合性

## ステータス

**提案（未実装、2026-08-26、ラウンド6で決定4を根本設計へ差し替え、決定3を汎用プリミティブへ分解）。** 直近のコードレビューで確証された指摘のうち、「probe/warmup の過渡期に必ず通るべき出口が無い」「一発限りフラグの所有スコープが未定義」という2系統をまとめる。Opus 2体によるドラフト→敵対的レビューを4ラウンド実施したのち、5ラウンド目で全指摘を実コードに突き合わせて再検証し、**6ラウンド目（本版）で「共通関数を1つ用意して6箇所から呼ぶ」という決定4の中心的な抽象そのものが実コードの制御フローと噛み合っていないことが判明したため、型で強制する形へ差し替えた**（下記「設計の経緯」）。関連: [ADR-102](102-startup-key-delivery-one-way-closure.md)、[ADR-104](104-observation-freshness-and-hardening.md)、[ADR-105](105-engine-thread-notification-via-hwnd.md)。

## コンテキスト

対象は、19件の Opus コードレビュー指摘（[ADR-102](102-startup-key-delivery-one-way-closure.md) が `[1][2][3][5]` を、本 ADR が `[6][7]` を引き取る）のうち次の3件である。

- **`[6]` `post_bypass_passthrough` フラグの残留**: Ctrl+vk バイパス直後に latch（`state/platform_state.rs:1423`）が立つが、消費点（`runtime/message_handlers.rs:96-112`）は NonText 早期 return より後にしかない。Ctrl+J 直後に別アプリへ移ると、latch を持ったまま**無関係な別プロセスの最初の1キー**に誤適用されうる。
- **`[7]` `probe_io.rs` の早期 return が deferred VK フラッシュと GjiFsm 通知の両方を飛ばす**: `dispatch_probe_actions`（`output/probe_io.rs:543-894`）に、段の後始末（`flush_deferred_and_mark_warmup`、`:481-491`）を通らずに抜ける出口が複数ある。BUG-27 の「未解決の follow-up」が名指ししている構造的な穴と同型である。
- **gji_fsm.rs の pending 消失と kind 捏造**: `ImeOff`（`tsf/gji_fsm.rs:559-562`）と `FocusChange`（`:586-589`）の pending 件数計算が `OnCold` しか見ず `OnComposing { warmup: AwaitingProbe }` を見落とす。`EndComposition`（`:770-788`）が `ColdKind`/`ProbeParams` を固定値で再構築する。

これらは同じ失敗形に収束する: **過渡期に「必ず通る出口」が無い**。probe/warmup の途中で早期 return すると、deferred VK のフラッシュや FSM への完了/中断通知といった「段の終わりに必ずやること」が飛ぶ。BUG-27/BUG-38 は既に同じ形で2回起きており、`flush_deferred_and_mark_warmup` の doc コメントがその経緯を記録している——**共通関数に括るだけでは呼び忘れを防げず、呼び忘れられる出口自体を型で消す必要がある**。

一発限りフラグ（post-bypass latch）も同じ形の亜種で、**判定材料（どのアプリを相手にしているか）を持たないまま「次の1キー」という時間的スコープだけで生きている**ことが根本原因である。

### 用語: 「段（stage）」

本 ADR で **段** とは、`install_pending_tsf`（`output/tsf_warmup_coord.rs:195-212`）で probe machine が `pending_tsf` に入ってから、その machine が二度と `restore_pending_tsf` されずに drop されるまでの区間を指す。1つの段は複数の `TIMER_TSF_PROBE` tick（＝複数回の `dispatch_probe_actions` 呼び出し）にまたがる。「段の終わり」に必ずやることは3つある: **(a) deferred VK キューの解放、(b) `GjiFsm` への完了/中断通知、(c) `OUTPUT_GATE` ガードと `TsfGate` の後始末**。

### 制約

- フックコールバック（`WH_KEYBOARD_LL`）上で新たな panic 経路・ブロッキング呼び出しを持ち込まない。本 ADR の決定3が追加する Win32 呼び出しは**エンジンスレッド**（`deliver_key_event`）上であり、フックコールバック上ではない。
- [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の3層分離を破らない。実際に注入していない副作用を「完了」として通知しない（`WarmupComplete` は実際に送信したときだけ）。
- タイミング定数（`tuning.rs`）は変更しない。新しい時間定数も導入しない。
- `TickableFsm`（`tsf/warmup/tickable_fsm.rs`）にデフォルト実装付きの capability メソッドを増やさない。同 trait の module doc が記録するとおり、**デフォルト no-op の capability を増やすとラップ型（`ChromeProbe` → `TsfProbeCoro`）の委譲漏れがコンパイルを通ってしまい、それが BUG-27 の直接原因だった**。本 ADR は `TickableFsm` を一切変更しない。
- **RAII（`Drop`）による段末フックは採らない**（却下理由は「却下した代替案」）。

## 不変条件

- **INV-B（運搬）**: 一度確定した値（cold の種別）は、後段で再導出せず運ぶ。弱い代理指標からの再計算は belief 汚染の常習経路である。
- **INV-C（派生値は関数に一元化し、欠落は潰さない）**: `ProbeParams` は `ColdKind` の純関数である（現行の3つの構築点 `gji_fsm.rs:363-366` / `:391-394` / `:642-645` はすべて `kind` から同じ式で作っている）。この関係を関数として固定し、`ProbeParams` の独立した構築を禁止する。運ぶべき確定値は `kind` ただ1つになる。**あわせて `unwrap_or_default()` による「捏造値と同じ既定値への暗黙の潰し」も禁止する**（[ADR-104](104-observation-freshness-and-hardening.md) INV-C（欠落の保存）と同じ趣旨）。
- **INV-D（副作用の通知は事実に一致する）**: FSM へ送る完了通知は、実際に起きた副作用とだけ対応させる。「送っていないが段は終わった」を「warmup 完了」として通知しない。**リテラル回収を出した段は、たとえ注入していても warm を主張しない**（リテラル化は「GJI が ready ではなかった」ことの積極的な証拠だから）。
- **INV-E（deferred VK の順序）**: coordinator の `pending_deferred` は「今送っているモーラを全部送り切った後」にだけ解放してよい。列の途中で解放すると後続打鍵が先行モーラの内部へ割り込む（BUG-38 と同型）。
- **INV-F（段の所有権はただ1つ）**: ある時点で `pending_deferred` の解放権を持つのは、飛行中の段か、raw literal 回収（`flush_raw_tsf_literal_recovery` 経路）か、GJI reinit retry のいずれか**1つだけ**である。段末の解放はこの所有権を照会してから行う。

---

## 決定3: 一発限りフラグに「今どの窓を相手にしているか」のスコープを与え、機構を汎用プリミティブへ括り出す

**意味論は前版から変えない**（スコープはプロセス同一性中心、TTL なし、`is_modifier`/`is_passthrough` は別扱い、評価位置は現状維持）。変えるのは機構である: 前版の「8アームの総関数 `post_bypass_action`」は、`latch=None` / スコープ不一致 / 取得失敗という3軸を判定関数の中に抱え込んでいたため、latch の失効責任が呼び出し側と判定関数に分裂していた。この3軸を**スコープ付きワンショット latch という再利用可能な型**へ吸収し、VK 分類だけを純関数に残す。

### 3-a. `ScopedOneShot<S, T>`（新設、`state/scoped_latch.rs`、ungated）

```rust
/// 「あるスコープ S の中でだけ有効な、一度きりの予約」を表す汎用 latch。
///
/// `peek` はスコープ不一致をその場で失効させる。呼び出し側に「disarm し忘れ」を
/// 残さないのが本型の存在意義であり、post-bypass latch が `[6]` で持っていた
/// 「スコープを持たないまま次の1キーを待ち続ける」形を構造的に不可能にする。
pub(crate) struct ScopedOneShot<S: Copy + PartialEq, T: Copy = ()> {
    armed: Option<(S, T)>,
}

pub(crate) enum ScopeCheck<T> {
    /// 予約なし。
    NotArmed,
    /// 予約はあったがスコープが変わっていた。この呼び出しで失効させた。
    Expired,
    /// 予約が有効。payload を返す（消費はしない）。
    Live(T),
}

impl<S: Copy + PartialEq, T: Copy> ScopedOneShot<S, T> {
    pub(crate) const fn new() -> Self { Self { armed: None } }
    pub(crate) fn arm(&mut self, scope: S, payload: T) { self.armed = Some((scope, payload)); }
    pub(crate) fn peek(&mut self, now: S) -> ScopeCheck<T> {
        match self.armed {
            None => ScopeCheck::NotArmed,
            Some((s, _)) if s != now => { self.armed = None; ScopeCheck::Expired }
            Some((_, t)) => ScopeCheck::Live(t),
        }
    }
    pub(crate) fn disarm(&mut self) -> Option<T> { self.armed.take().map(|(_, t)| t) }
    pub(crate) const fn is_armed(&self) -> bool { self.armed.is_some() }
}
```

`state/probe_admission.rs` の `ImmLikeTicket::admit`（spawn 時スコープを捕まえ、完了時に照合して棄却する）と同じ発想を、非同期 probe ではなく「次の1キー」に適用したものである。**本 PR で移行するのは post-bypass latch の1件のみ**。リポジトリ内には同型のスコープ無しワンショットフラグが他にもある（`Output::ms_ime_gate_give_up`、`confirm_gate_deadline_override_ms`、`GateStore::shift_conv_guard_pending`、`half_width_alnum_toggle_active`、`left_shift_tap_candidate`）が、いずれも BUG-36/BUG-49/BUG-58/BUG-74 が積み重なった領域であり、同じ PR で触るのはリスクが実利を上回る。移行候補表として本 ADR に記録するに留める。

### 3-b. スコープは「前景プロセス＋前景トップレベル窓」

```rust
/// win32.rs（新設）: post-bypass latch のスコープ。武装時と評価時で必ず同じ関数で採る。
///
/// `GetForegroundWindow` / `GetWindowThreadProcessId` はいずれも非ブロッキングで
/// どのスレッドからも呼べる（`win32.rs:196-214` の既存 SAFETY コメントと同じ根拠）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundScope {
    pub pid: u32,
    /// `GetForegroundWindow()` の生値（`HWND` は `Send` でないため isize で持つ）。
    pub hwnd: isize,
}

impl ForegroundScope {
    /// 取得失敗（前景窓なし・pid 0）。実在のスコープとは決して等しくならない。
    pub const INVALID: Self = Self { pid: 0, hwnd: 0 };
    #[must_use] pub const fn is_valid(self) -> bool { self.pid != 0 && self.hwnd != 0 }
}

#[must_use]
pub fn foreground_scope() -> ForegroundScope;   // 失敗時は INVALID
```

`GateStore::post_bypass_passthrough: bool`（`state/platform_state.rs:1423`）を次で置き換える。

```rust
pub post_bypass: ScopedOneShot<ForegroundScope, PostBypassArm>,

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostBypassArm {
    /// ログ・診断専用。判定には使わない（理由は 3-d）。
    pub armed_focus_epoch: u64,
}
```

**なぜ pid だけでなく hwnd も持つか（前版からの強化）**: `ApplicationFrameHost.exe` は複数の別々の UWP アプリの前景フレーム窓を**同一 pid で所有しうる**。pid だけをスコープにすると UWP アプリ A → B の切り替えで `armed_pid == now_pid` が成立し、無関係な別アプリの最初の1キーに latch が誤適用される——`[6]` が指摘した実害そのものが UWP に限って残る。前景トップレベル窓の hwnd を組にすれば、同一プロセスの別ウィンドウも区別できる。**追加の Win32 呼び出しは発生しない**（pid を採るのに `GetForegroundWindow()` の戻り値をどのみち使う）。同じ理由づけは [ADR-104](104-observation-freshness-and-hardening.md) 決定6-a が `ObservationTicket` に `focus_hwnd` を足した判断と同型である。

**`app.platform.focus` の pid/hwnd を使ってはならない。** これは `GUITHREADINFO.hwndFocus`（前景トップレベル窓ではなく**フォーカスを持つ子窓**）由来で、かつ非同期 focus probe の完了時にしか更新されない。UWP/`ApplicationFrameHost` 系では前景窓と子窓のプロセスが実際に異なりうる（`focus/classifier.rs` の InputSite フォールバックが「focus 側クラスと前景窓クラスは別物」という前提でそもそも書かれている）。武装側と評価側で採取元が違うと、そのアプリでは**恒久 mismatch**（毎回 `Expired`）になり、tmux prefix 機能が無言で止まる。

`foreground_scope()` が呼ばれるのは (i) 武装時（`post_bypass_rules` に一致した Ctrl+key の PassThrough、実運用では稀）と (ii) **`post_bypass.is_armed()` が真の間の各打鍵**だけである。`is_armed()`（フィールド読み1回）で先に絞ってから `foreground_scope()` を呼ぶ形にすることで、大多数の打鍵で Win32 呼び出しが増えないことを**呼び出し順として固定する**（前版は「純関数に `now_pid` を引数で渡す」形だったため、この遅延評価が本文の主張と実装で食い違いうる状態だった）。

武装は `scope.is_valid()` のときだけ行う（無効スコープで武装すると、以後どの打鍵でも `Expired` になるだけの死んだ latch になる）。

**既知の限界（今回のスコープ外）**: 武装判定そのもの（`PostBypassEntry::matches` に渡す `process_name`/`class_name`、`message_handlers.rs:138-144`）は従来どおり `app.platform.focus` の非同期キャッシュを読む。ここを `foreground_scope()` 系に揃えるのは別軸の変更であり、本 ADR では触らない。pid の再利用（プロセス終了後に同じ pid が別プロセスへ割り当てられる）も理論上は誤判定になりうるが、hwnd も同時に一致する確率は無視でき、被害は「1キーが NICOLA をスキップして素通しされる」に留まるため許容する。

### 3-c. VK 分類は4値の純関数に縮小する

```rust
/// state/post_bypass.rs（新設、ungated）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostBypassKey {
    /// latch を維持し、このキー自体は通常処理へ流す。
    /// （Ctrl 押下中＝別の bypass / 修飾キー単独 / passthrough キーの孤児 KeyUp）
    KeepArmed,
    /// このキー自体が prefix のコマンドキーだが NICOLA の対象外（矢印・F キー・Esc・Tab 等）。
    /// latch を落とすだけで、キー自体は通常処理へ流す。
    ConsumesPrefixSilently,
    /// prefix のコマンドキー本体。NICOLA をスキップして素通しし、latch を消費する。
    ConsumeAndPassthrough,
    /// 対応する KeyDown で既に消費済みのはずの孤児 KeyUp。素通しするが latch は維持する。
    PassthroughKeepArmed,
}

/// `vk` そのものではなく分類済み bool を取るのは、`vk.rs` の分類関数を SSOT として
/// 保ち、テストが VK 表を再実装しないため。
///
/// **判定順序は入れ替えてはならない。** `vk::classify_modifier` が `Some` を返す集合
/// （0x10/0xA0/0xA1、0x11/0xA2/0xA3、0x12/0xA4/0xA5、0x5B/0x5C）は
/// `vk::is_passthrough`（`vk.rs:238-258`、F1-F24・矢印・Tab・Esc・テンキー・CapsLock
/// 等60個超）の**真部分集合**であり、両者は排他ではない。modifier 判定を
/// passthrough 判定より後ろへ動かすと `prefix + Shift + 5`（tmux の `%`）が壊れる。
pub(crate) const fn classify_post_bypass_key(
    is_key_down: bool,
    ctrl_held: bool,
    vk_is_modifier: bool,     // vk::classify_modifier(vk).is_some()
    vk_is_passthrough: bool,  // vk::is_passthrough(vk)
) -> PostBypassKey {
    if ctrl_held { return PostBypassKey::KeepArmed; }              // 次の Ctrl+key は「別の bypass」
    if vk_is_modifier { return PostBypassKey::KeepArmed; }         // 次に来るキーの修飾（必ず下より前）
    if vk_is_passthrough {
        return if is_key_down {
            PostBypassKey::ConsumesPrefixSilently                  // prefix + ← 等、これ自体がコマンド
        } else {
            PostBypassKey::KeepArmed                               // KeyDown を取りこぼした孤児 KeyUp
        };
    }
    if is_key_down { PostBypassKey::ConsumeAndPassthrough } else { PostBypassKey::PassthroughKeepArmed }
}
```

`ConsumesPrefixSilently` は**現行挙動からの意図的な変更である。** 現行の分岐条件は `flag && !ctrl && !vk.is_passthrough()`（`message_handlers.rs:97-100`）であり、`prefix + ←` を押しても latch が落ちず同一プロセス内に残留する。決定3はこの残留も閉じる。**証拠義務に含める characterization テストは、この変更後の期待値で書く**（変更前の挙動を仕様として固定してはならない）。`ConsumeAndPassthrough` / `PassthroughKeepArmed`（KeyDown 消費・KeyUp 非消費）は現行どおり固定する。

### 3-d. 呼び出し側（`deliver_key_event`）の形

評価位置は**現状のまま**（`runtime/message_handlers.rs:64-157` の中、`Hook(Nested)` 早期 return と NonText 早期 return の後、`process_key_event` の前）。[ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-a の枝表の該当行はそのまま有効である。

```rust
// message_handlers.rs:96-112 の置換
if app.platform_state.gate.post_bypass.is_armed() {
    let now = crate::win32::foreground_scope();
    match app.platform_state.gate.post_bypass.peek(now) {
        ScopeCheck::NotArmed => {}                       // is_armed() で絞ったため到達しない
        ScopeCheck::Expired => {
            log::debug!("[post-bypass] expired: 前景が変わった → latch 失効");
        }
        ScopeCheck::Live(_arm) => {
            match classify_post_bypass_key(
                is_key_down,
                event.modifier_snapshot.ctrl,
                event.vk_code.classify_modifier().is_some(),
                event.vk_code.is_passthrough(),
            ) {
                PostBypassKey::KeepArmed => {}
                PostBypassKey::ConsumesPrefixSilently => { app.platform_state.gate.post_bypass.disarm(); }
                PostBypassKey::ConsumeAndPassthrough => {
                    app.platform_state.gate.post_bypass.disarm();
                    app.executor.enqueue_reinject(event);
                    post_to_main_thread(WM_EXECUTE_EFFECTS);
                    return KeyDelivery::Reinjected;
                }
                PostBypassKey::PassthroughKeepArmed => {
                    app.executor.enqueue_reinject(event);
                    post_to_main_thread(WM_EXECUTE_EFFECTS);
                    return KeyDelivery::Reinjected;
                }
            }
        }
    }
}
```

`ScopeCheck::Expired` はそのまま通常処理へ落ちる（前版の `DisarmOnly` と同じ）。`foreground_scope()` が `INVALID` を返した場合も `Expired` になる——**fail-safe**（prefix を1回無駄にするほうが、無関係なアプリへ誤適用するより軽い）。

- `Hook(Nested)`（トレイメニュー等の表示中）では評価に到達しない。awase 自身のトレイメニューを開いて前景が awase になっても latch には触れない——これは望ましい（メニュー操作で prefix を潰さない）。
- `INPUT_DEFER` drain 経由のリプレイも `deliver_key_event(.., KeyOrigin::DeferredReplay)` を通るため、hook 経由と同じ判定を受ける。

### 3-e. スコープに `focus_epoch` を使わない・TTL を持たせない

`focus_epoch` は一瞬のフォーカス奪取（通知ポップアップ等、BUG-57 の実例）でも進む。epoch をスコープにすると、Windows Terminal で Ctrl+J 直後に通知が出て消えただけで次の `n` が prefix として扱われなくなる——**正しく動いているケースの退行**である。`ForegroundScope` なら通知の往復（別 pid/hwnd → 元の pid/hwnd）を跨いで latch が生き残る。`armed_focus_epoch` はログ・診断用に持つが判定には使わない。

TTL は「ユーザーが prefix の次を押すまでの思考時間」という人間側の変数を ms で当てる必要があり、[tuning-constants](../../.claude/rules/tuning-constants.md) が要求する実測の対象にできない。前景の同一性というアプリ側で観測可能な事実だけでスコープを閉じる。

**証拠義務**: focus 遷移ファミリー。`state/` に置く（Linux 実行可）。

- `ScopedOneShot` の全数テスト（未武装／スコープ一致で `Live`／スコープ不一致で `Expired` かつ**その場で失効している**こと／`disarm` の冪等性）。
- `classify_post_bypass_key` の全数テスト（`is_key_down` × `ctrl_held` × `vk_is_modifier` × `vk_is_passthrough` の16通り、4アームすべてが少なくとも1回返ること）。
- 順序依存の回帰テスト: `vk_is_modifier=true && vk_is_passthrough=true`（実在の組み合わせ。Shift/Ctrl/Alt/Win はすべてこれ）で `KeepArmed` が返ること。これが `ConsumesPrefixSilently` になったら `prefix + Shift + 5` が壊れる。
- 系列テスト: 通知往復（armed(scope=P) → 別 scope の打鍵で `Expired` → 元 scope へ戻っても latch は無い、が **通知中に打鍵しなければ** `Live` のまま）、`prefix + Shift + 5`、`prefix + ←`（`ConsumesPrefixSilently` の後、次の文字キーが `NotArmed`）。
- `docs/known-bugs.md` に暫定 **BUG-84** を起票し、スコープに `focus_epoch` ではなく `ForegroundScope{pid, hwnd}` を選んだ理由、`ApplicationFrameHost` 同一 pid 問題、`ConsumesPrefixSilently` で現行挙動を変えた理由を残す。`fix-requires-evidence.md` の family 表には `message_handlers.rs`/`platform_state.rs` が含まれないため pre-push フックは発火しない——**known-bugs.md への記録は手動で行うこと**。

---

## 決定4: 段の終わりを型で強制し、段の後始末を1関数に閉じる

### 4-a. 実コードで確認した事実（前版の記述はここが誤っていた）

`dispatch_probe_actions`（`output/probe_io.rs:543-894`）から抜ける経路は**8つ**あり、前版の表が「出口」として挙げた6行のうち3行は**出口ではなく flow 途中の flush 点**だった。

| 位置 | 現行 | 種別 |
| --- | --- | --- |
| `:560` | `ProbeAction::Done => return DispatchResult::Done` | **出口**（前版は数えていなかった） |
| `:574-577` | `Transmit`/Tsf、`gate_is_bypass()` | 出口 |
| `:578-580` | `Transmit`/Tsf、`chars.is_empty()` | 出口 |
| `:616` | Tsf batch 送信後の `flush_deferred_and_mark_warmup` | **flush 点**（直後に `apply_transmit_done` があり、false ならループ継続） |
| `:626-633` | Tsf batch、`apply_transmit_done` が true | 出口 |
| `:646` | Chrome batch 送信後の flush | **flush 点** |
| `:656-663` | Chrome batch、`apply_transmit_done` が true | 出口 |
| `:682-688` | `TransmitSingleVk`/Tsf、`gate_is_bypass()` | 出口 |
| `:709-712` | `TransmitSingleVk`、`is_last` の flush | **flush 点**（直後に `:713` の trace push と `:720` の `machine.apply_vk_sent` があり、ループは次の action へ進む） |
| `:723-727` | `UpgradeToTsf` | 出口（`LearnedTsf`） |
| `:893` | ループ脱出 | 出口（`Continue`） |

**前版の表どおりに `:709-712` を `return` へ置き換えると per-VK confirm が全モーラで壊れる。** `machine.apply_vk_sent` が呼ばれなくなり、次 tick で `run_per_vk_confirm` の `let Some(sent) = vk_input.vk_sent else { .. }`（`tsf/warmup/probe_fsm.rs:478-492`）が発火する。この分岐は BUG-27 追補2 で「msedge の `Chrome_WidgetWin_1` において打鍵のたびに毎回発火し、正しく入力できていた文字まで backspace で消えて実質何も入力できなくなった」ことが実機確認済みの経路である。

**`:560` は段の終わりの合流点として決定的に重要である。** `ProbeCoroState::tick`（`tsf/warmup/probe_coro_state.rs:82-85`）は `CoroStep::Complete => vec![ProbeAction::Done]` に変換するため、**コルーチン内部の早期 return はすべて次 tick で `:560` に到達する**。前版は「coro 側はスコープ外」と書いていたが、段末に後始末を置けば dispatcher 側と coro 側の穴が同時に閉じる。

そのうえで、早期 return が実際に起こす害は次の2つである（前版の分析は正しい）。

1. **deferred VK が置き去りになり、順序が反転する。** 早期 return 後は `has_pending_tsf()` が false になるため次の打鍵は defer されず、新しい probe を張ってその `Transmit` で送信される。古い deferred VK は新しいモーラの後ろへ回る（BUG-27 の「とうろく」→「と」が消え「うろ」が「ろう」に反転する症状と同型）。
2. **`GjiFsm` が完了通知も中断通知も受け取らない。** `current_gji_probe_id` は set のまま、状態は `OnCold { probe: Authorized }` のまま固着する。以後 `KeyInput` は pending を積むだけ（`gji_fsm.rs:650-653`）で新しい `StartProbe` も出さず、`is_warm()` が false のままなので `assess_warmth().prepend_f2_warmup`（`output/mod.rs:1188`）が**毎打鍵 true** になる。

**「早期 return の直前に1行足す」は採らない**。`flush_deferred_and_mark_warmup` は既に BUG-27/BUG-38 の再発を受けて共通化された関数であり、それでもなお呼び忘れた出口が残った。**共通関数化は呼び忘れを防げないと実証済みなので、値なしに脱出できない形にする**。

### 4-b. 段の終わりを値で表現し、`return` を関数から消す

```rust
/// tsf/gji_fsm.rs（`GjiEvent::WarmupAborted` が運ぶため FSM 側に置く。
/// `output/probe_io.rs` は既に `crate::tsf::gji_fsm::ProbeId` を import しており、
/// output → tsf の依存方向は現行のまま。逆向き（tsf → output）は作らない）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageEndReason {
    /// `ProbeAction::Done`。正常完了と、コルーチン内部中断（`vk_sent` 未設定 /
    /// `SuspectedLiteral` / `StaleConfirm`）の両方がここへ合流する。
    ProbeDone,
    /// 段に入る前に `gate_is_bypass()` だった（batch / per-VK の `idx == 0`）。
    GateBypass,
    /// romaji から解決できる VK が1つも無い（`chars.is_empty()`）。
    NoResolvableVk,
    /// `UnicodeLiteralObserverFsm` が Tsf 昇格を決めた。
    UpgradedToTsf,
}

/// output/probe_io.rs
pub(crate) struct StageEnd { pub(crate) reason: StageEndReason }

pub(crate) enum DispatchResult {
    /// 段は継続。`step_probe` が machine を `restore_pending_tsf` する。
    Continue,
    /// 段は終わった。`step_probe` が machine を drop し `finish_probe_stage` を呼ぶ。
    Ended(StageEnd),
}

impl DispatchResult {
    /// 既存テスト（`probe_io.rs` に24箇所）の意味を変えずに残すための互換アクセサ。
    #[cfg(test)]
    pub(crate) fn is_done(&self) -> bool {
        matches!(self, Self::Ended(e) if e.reason != StageEndReason::UpgradedToTsf)
    }
    #[cfg(test)]
    pub(crate) fn is_learned_tsf(&self) -> bool {
        matches!(self, Self::Ended(e) if e.reason == StageEndReason::UpgradedToTsf)
    }
}
```

`dispatch_probe_actions` の本体をラベル付きブロックにし、**関数本体から `return` を1つ残らず消す**。

```rust
let ended: Option<StageEndReason> = 'stage: {
    while let Some(action) = queue.pop_front() {
        match action {
            ProbeAction::Done => break 'stage Some(StageEndReason::ProbeDone),
            ProbeAction::Transmit { .. } => {
                // Tsf: gate=Bypass  → break 'stage Some(StageEndReason::GateBypass)
                // Tsf: chars 空      → break 'stage Some(StageEndReason::NoResolvableVk)
                // 送信 → apply_transmit_done が true なら
                //                     break 'stage Some(StageEndReason::ProbeDone)
            }
            ProbeAction::TransmitSingleVk { idx, .. } => {
                // Tsf かつ idx == 0 かつ gate=Bypass
                //                   → break 'stage Some(StageEndReason::GateBypass)
                // それ以外は必ず送信し、apply_vk_sent まで通す（4-c）
            }
            ProbeAction::UpgradeToTsf => break 'stage Some(StageEndReason::UpgradedToTsf),
            /* 他の action は副作用のみ、ループ継続 */
        }
    }
    None
};
match ended {
    None => DispatchResult::Continue,
    Some(reason) => DispatchResult::Ended(StageEnd { reason }),
}
```

- 段が終わる出口は **`break 'stage <理由>` という形でしか書けない**。理由なしの脱出はコンパイルエラーになる（grep ガードではなく型検査で強制する）。
- `flush_deferred_and_mark_warmup`（`:481-491`）と `store_gji_warmup_if_probing`（`:471-479`）は**削除する**。deferred の解放は段末へ移り、注入の記録は 4-d のとおり注入メソッド自身が行う。
- `tests/architecture_guard.rs` に、既存の `extract_fn_body` ヘルパを使って **「`dispatch_probe_actions` の本体に `return DispatchResult` が0件」** を固定するテストを追加する（型検査の第二の防衛線）。

### 4-c. per-VK 列は gate で中断しない（前版 4-b を撤回）

前版は「gate 判定を `idx == 0` へ引き上げ、`idx > 0` で Bypass に落ちたら輸送手段を降格して送り切る」としていたが、これは実装不可能だった: `run_per_vk_confirm`（`probe_fsm.rs:436-634`）は**1 tick に1 VK しか流さない**（`idx=0` と `idx=1` は別々の `dispatch_probe_actions` 呼び出し）ため、「列に入る前に一度だけ評価する」という記述に対応する場所が存在せず、`degraded` フラグの保持場所も未定義だった。

**改訂: gate は段への入場条件としてのみ見る。**

```rust
// probe_io.rs:682-688 の置換
if target == TransmitTarget::Tsf && idx == 0 && io.gate_is_bypass() {
    break 'stage Some(StageEndReason::GateBypass);
}
```

`idx > 0` では gate を一切再確認せず、`target` が決めた marker（`Tsf` → `send_single_tsf_vk`、`Chrome` → `send_single_chrome_vk`）でそのまま送り切る。理由:

- **モーラの途中で中断することが最も破壊的である。** 送信済みの VK は生文字として残り、未送信の VK は永久に送られない（BUG-27 追補2 の「これでできる」→「kれでできる」がこの形）。gate が Bypass へ落ちたという事実は、**既に開いている composition が無効になった証拠ではない**。
- gate が `Bypass` へ遷移するのは `BypassConfirmed`（focus probe が非 TSF と確定）か `WarmupTimeout`（`tsf/tsf_gate.rs:138-179`）だけであり、いずれもフォーカス起点の事象である。フォーカスが本当に変わったなら `GjiEvent::FocusChange` → `CancelProbe` → `cancel_probe()` が段ごと畳む（4-f）。
- 結果として `degraded` フラグ・一方向ラッチ・`TransmitSkip::sink_marker`・`deferred_sink_marker` がすべて不要になる。`Output::flush_pending_deferred_vks`（`output/mod.rs:1622-1635`）の既存の marker 選択をそのまま使えばよく、新しい関数を1つも足さずに済む。

### 4-d. 「実際に注入したか」は注入メソッド自身が記録する

`TsfWarmupCoordinator::pending_gji_warmup: Cell<bool>`（`output/tsf_warmup_coord.rs:38`）を段単位の記録へ置き換える。

```rust
/// output/tsf_warmup_coord.rs
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StageRecord {
    /// この段で1文字以上を実際に注入したか。
    pub injected: bool,
    /// この段が raw literal 回収（`mark_cold_raw_tsf`）を出したか。
    /// 出したなら GJI は ready ではなかったので、注入していても warm を主張しない（INV-D）。
    pub recovered: bool,
}

// TsfWarmupCoordinator のフィールド（`pending_gji_warmup` を置き換える）
stage: Cell<StageRecord>,

pub(crate) fn begin_stage(&self)          { self.stage.set(StageRecord::default()); }
pub(crate) fn note_stage_injection(&self) { let mut s = self.stage.get(); s.injected = true;  self.stage.set(s); }
pub(crate) fn note_stage_recovery(&self)  { let mut s = self.stage.get(); s.recovered = true; self.stage.set(s); }
pub(crate) fn take_stage_record(&self) -> StageRecord { self.stage.take() }
```

2つのフィールドはどちらも**単調（false→true のみ）**なので、前版の `Cell<Option<ProbeStageOutcome>>` で問題になった「1回の dispatch で複数の outcome が書かれたときの後勝ち規則が未定義」という合成規則の穴が構造的に存在しない。

`ProbeIo`（`output/probe_io.rs:25-105`）の変更:

- `fn store_gji_warmup_result(&self)` を**削除**し、`fn note_stage_injection(&self)` を追加する。
- `impl ProbeIo for Output` の**注入メソッド4つが自分で呼ぶ**: `transmit_tsf`（`:112-131`）、`transmit_chrome`（`:133-135`）、`send_single_tsf_vk`（`:137-139`）、`send_single_chrome_vk`（`:141-145`）。dispatcher からは1行も呼ばない。
- `fn mark_cold_raw_tsf(&self)` の本番実装（`:173-176`）に `self.warmup_coord.note_stage_recovery();` を足す。**`mark_cold_raw_tsf` は `RawTsfLiteralRecovery` アームの全分岐で無条件に呼ばれる唯一の呼び出し**（`probe_io.rs:844`、`consecutive==0` / give-up / `SuppressedExistingPoll` / `SuppressedExistingScheduled` のすべてを通る）なので、「composition を cold にマークした段は warm を主張できない」という規則が呼び忘れようのない形で成立する。dispatcher に `note_stage_recovery` を書かないのは意図的である——そこに置くと**忘れたときに危険側（warm 誤申告）へ倒れる**。

`begin_stage()` を呼ぶのは `install_pending_tsf`（`tsf_warmup_coord.rs:195-212`）の**先頭**、`pending_tsf.borrow_mut()` を取る前。`stage` は別フィールド（`Cell`）なので二重借用は起きないが、`slot` を保持したまま他フィールドに触る形を残さないため位置を固定する。これにより、前版が見落としていた次の潜在バグが閉じる。

> **新規発見（BUG-85 として起票する）**: `pending_gji_warmup` は `cancel_probe()`（`output/mod.rs:1365-1369`）でも `install_pending_tsf` の上書きでもクリアされず、**段をまたいで残る**。段 A が `mark_warmup_pending()` を立てた直後に `CancelProbe` で machine が破棄されても bool は true のままで、段 B の最初の tick が `Done` に落ちると、**1文字も注入していない段 B の probe_id で `WarmupComplete` が出て `OnWarm` になる**。`step_probe` の `Done` アームだけが `take_warmup_pending()` を呼び、`Continue`/`LearnedTsf` アームは呼ばないことも同じ穴を広げている。

**なぜ `begin_stage` で probe_id を捕まえないか（根本設計案からの訂正）**: 段の開始時点では `current_gji_probe_id` はまだこの段のものではない。`send_romaji_as_tsf`（`output/vk_send.rs:294-318`）は `install_pending_tsf` を先に呼び、`GjiAction::StartProbe` → `Output::gji_store_probe_id`（`platform.rs:498`）はその後の `drain_output_post_send_effects`（`platform.rs:905-922`）で実行される。段の開始時に読むと**前の段の probe_id か `None`** を掴む。したがって probe_id は現行どおり**段末に `take_probe_id()` で読む**（`step_probe` の Done アームと同じ）。stale な id を掴んだ場合は `GjiFsm` 側の `running_probe_id()` 照合が弾く（`gji_fsm.rs:670-678`）。

### 4-e. 段末の後始末は `finish_probe_stage` ただ1箇所

```rust
/// output/mod.rs: 段の終わりに必ずやること（a)(b)(c) をこの順で行う唯一の場所。
/// `step_probe` の `Ended` アームからだけ呼ぶ。
fn finish_probe_stage(&mut self, end: StageEnd) -> Option<GjiResponse> {
    // (a) deferred VK の解放。所有権が raw literal 回収側にある間は触らない（INV-F）。
    if self.raw_recovery_owns_deferred() {
        log::debug!("[stage-end] {:?}: deferred の解放は raw recovery 側に委ねる", end.reason);
    } else {
        let n = self.flush_pending_deferred_vks();   // 既存関数をそのまま再利用
        if n > 0 { log::debug!("[stage-end] {:?}: deferred {n} VK(s) を flush", end.reason); }
    }
    // (c) TsfGate / OUTPUT_GATE ガード。deferred を送り切ってからゲートを開ける。
    self.on_tsf_probe_ready();
    self.gji_end_probe_guard();
    // (b) GjiFsm への通知。
    let rec = self.warmup_coord.take_stage_record();
    let probe_id = self.warmup_coord.take_probe_id()?;
    Some(self.gji_on_event(if rec.injected && !rec.recovered {
        GjiEvent::WarmupComplete { probe_id }
    } else {
        GjiEvent::WarmupAborted { probe_id, reason: end.reason }
    }))
}

/// deferred VK の解放権が raw literal 回収 / GJI reinit retry 側にあるか（INV-F）。
///
/// `flush_raw_tsf_literal_recovery`（`output/mod.rs:1504-1519`）は末尾で必ず
/// `flush_stale_deferred_vks_after_recovery` を通り、`WM_DRAIN_OUTPUT_QUEUE`
/// ハンドラ（`runtime/message_handlers.rs:1004-1009`）から無条件に呼ばれる。
/// BUG-38 の順序（backspace / romaji 再送 / reinit がすべて実送信されたあとで
/// なければ deferred を出してはいけない）はこの経路が守る。
fn raw_recovery_owns_deferred(&self) -> bool {
    RAW_TSF_LITERAL.backs.load(Relaxed) != 0
        || !RAW_TSF_LITERAL.romaji.lock()…is_empty()
        || self.pending_gji_reinit.borrow().is_some()
}
```

**なぜ「回収が所有しているか」を `StageEnd` のフィールドではなく段末の状態照会にするか（根本設計案からの訂正）**: 根本設計案は `StageEnd { recovery_owns_deferred: bool }` を提案していたが、この値を dispatch 中に計算すると**段が終わる tick と回収が予約された tick が食い違う経路で破綻する**。`emit_recovery_actions`（`tsf/warmup/literal_detect_fsm.rs:162-179`）は `[RawTsfLiteralRecovery, Done]` を必ずセットで yield するので現行コードでは同一 dispatch に収まるが、これは呼び出し側の慣行であって型が保証していない。所有権は**照会時点の事実**として読むのが正しく、そうすれば「回収予約と段末が別 tick に分かれても壊れない」。

`step_probe`（`output/mod.rs:1235-1332`）は3アームから2アームになる。

```rust
match dispatch {
    DispatchResult::Continue => {
        let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
        self.warmup_coord.restore_pending_tsf(machine);
        StepProbeResult { timer_cmd: Continue{..}, gji_response: None, .. }
    }
    DispatchResult::Ended(end) => {
        // machine はここで drop される（restore しない）＝段の終わり。
        let learned_tsf = end.reason == StageEndReason::UpgradedToTsf;
        let needs_gji_composition_reset = self.warmup_coord.take_composition_reset();
        let gji_response = self.finish_probe_stage(end);
        StepProbeResult {
            timer_cmd: Kill { id: TIMER_TSF_PROBE },
            gji_response, needs_gji_composition_reset, learned_tsf,
            completed_cold_seq: Some(cold_seq), literal_detect,
        }
    }
}
```

これにより、旧 `LearnedTsf` アーム（`:1316-1330`）が抱えていた次の3つの漏れが同時に閉じる。

1. `gji_end_probe_guard()` を呼んでいなかった（`Done` アームと `cancel_probe` だけが呼んでいた）。**新規発見**。
2. `take_probe_id()` を呼ばないため `current_gji_probe_id` が次の `StartProbe` まで残留していた。**新規発見**。
3. `take_warmup_pending()` を呼ばないため 4-d の bool が段をまたいでいた。

`on_tsf_probe_ready()` を `UpgradedToTsf` でも呼ぶことになるが、これは安全である: `TsfGateMachine::on_event` は `(Probing, ProbeComplete)` の1アームでしか状態を変えず、それ以外は `_ => Response::pass_through()`（`tsf/tsf_gate.rs:162-168`）で no-op である。Unicode 注入モード（`UpgradeToTsf` の唯一の発生源）で gate が `Probing` になっていることはない。

**`apply_transmit_skipped` は導入しない。** 前版が撤回した理由（`step_probe` は `Done`/`LearnedTsf` で machine を restore せず drop するので「tick され続けて二重に走る」は起こらない）はそのまま有効であり、本版では `Ended` アームが machine を drop することがコード上いっそう明示的になった。

### 4-f. `cancel_probe` / `install_pending_tsf` 上書きも同じ規則を通す

| 経路 | 段の記録 | deferred キュー |
| --- | --- | --- |
| `install_pending_tsf`（`tsf_warmup_coord.rs:195-212`、上書き含む） | `begin_stage()` で張り直す | **保持する**（新しい段が引き継いで flush する。既存テスト `deferred_vks_survive_probe_replacement` が固定している挙動） |
| `cancel_probe()`（`output/mod.rs:1365-1369`） | `take_stage_record()` で捨てる | **破棄する**（下記） |

`cancel_probe()` が発火するのは `GjiAction::CancelProbe`（`platform.rs:545-553`）、すなわち `ImeOff` / `FocusChange` / `handle_composition_reset` の3経路だけである。これは決定5-a が `GjiFsm` の `pending`（同じ打鍵の romaji 影）を破棄する経路と**完全に同じ集合**である。片方だけ残すと shadow と実体がずれ、残った VK は「誰にも所有されないまま、はるか後の無関係な回収でまとめて送られる」——BUG-27 の順序反転そのものになる。

```rust
pub(crate) fn cancel_probe(&self) {
    self.warmup_coord.clear_pending_tsf();
    self.gji_end_probe_guard();
    let _ = self.warmup_coord.take_probe_id();
    let _ = self.warmup_coord.take_stage_record();
    let n = self.warmup_coord.take_pending_deferred().len();   // 既存メソッド
    if n > 0 {
        log::warn!("[stage-cancel] deferred {n} VK(s) を破棄（宛先窓が変わった / エンジン停止）");
        // journal に件数を残す（ソーク項目）
    }
}
```

**これは awase が意図的に打鍵を捨てる数少ない場所である。** `FocusChange` ではそのまま送ると別ウィンドウへの誤送信になり（[ADR-101](101-bug74-giveup-retry-with-focus-guard.md) が focus 世代照合で塞いだのと同じ事故）、`ImeOff` では生の英字が出る。`discard_pending_deferred_after_stale_gji_reinit`（`output/mod.rs:1637-1644`）が同じ判断で既に存在する prior art である。件数をソークで測り、`CompositionReset` 由来の破棄が有意なら別 ADR で再検討する。

### 4-g. `GjiEvent::WarmupAborted` の意味論

```rust
GjiEvent::WarmupAborted { probe_id: ProbeId, reason: StageEndReason }
```

「この probe の段は、warm を主張できる形では終わらなかった。deferred VK キューは段末の規則（4-e）で処理済みであり、probe machine も drop 済みである」。`WarmupComplete` と同じく `running_probe_id()` との照合で stale を弾く。

- **`pending` の扱い**: `GjiAction::DiscardPending { count, reason: PendingDiscardReason::WarmupAborted }` を emit する（決定5-a の破棄アクションを共有する）。**`SendInput { pending }` は使わない**。前版はここを取り違えていた: `send_romaji_as_tsf` は `GjiEvent::KeyInput(PendingInput::new(romaji))` を**無条件に先頭で**dispatch し（`vk_send.rs:252-257`）、そのあとで defer するかを判定する。したがって `pending` の先頭要素は「この段を起動したローマ字」であり、`GateBypass` で中断した段ではそれは**一度も注入されていない**。「送った」と記録するのは INV-D 違反になる。
  - なお `pending` と coordinator の `pending_deferred` は**別物**である（`PendingInput` は `romaji: String` の1フィールド、`gji_fsm.rs:76-79`）。`DiscardPending` は「GjiFsm が持っていた romaji の影を捨てた」という意味であって、ユーザーの後続打鍵が失われたことは意味しない（後続打鍵の VK は 4-e で送信済み）。この非対称を doc コメントに明記する。
- **遷移先**:
  - `OnCold { kind, .. }` → `OnCold { kind, probe: ProbeStatus::NotStarted, pending: [] }`。**`OnWarm` には絶対に遷移しない。** `NotStarted` にすることで、次の `KeyInput` が `kind` から新しい probe を認可し直す（`gji_fsm.rs:639-649`）。死んだ `probe_id` を `Authorized` に残すと固着が再発する。`Short` + `NotStarted` は新しい状態ではない（`handle_composition_reset` の `OnCold` アーム `:472-487` が既に生成しうる）。
  - `OnComposing { warmup: AwaitingProbe { kind, .. } }` → `OnComposing` に留まり、`warmup` を `ComposingWarmup::AbortedCold { kind }` にする。後続の `EndComposition` がこの `kind` で `OnCold { kind, probe: NotStarted, pending: [] }` を再構築する（決定5-b と同じ材料・同じ規則）。
  - `AbortedCold` は `AlreadyWarm` と**別の variant** にする。理由は前版の記述（「丸めると `is_warm()` が warm になる」）ではない——`is_warm()`（`tsf/warmup/warmup_strategy.rs:76-82`）は `OnWarm | OnComposing` であり、`OnComposing` である限り warmup の中身に関わらず warm を返す。**正しい理由は `EndComposition` の再構築先が違うことである**: `AlreadyWarm` に丸めると `EndComposition` が `transition_to_warm()` を呼び（`gji_fsm.rs:769`）、1文字も注入していない probe を根拠に `OnWarm` へ落ちる。それが決定4の塞ぎたいリテラル漏れの経路である。
  - **`OnComposing` の間 `is_warm()` が true のままである点は意図的に変更しない**。`StartComposition` は `WM_IME_STARTCOMPOSITION`（GJI が実際に合成を始めた実観測）で入る状態であり、probe の中断はその観測を無効化しない。`AwaitingProbe` 中も現行は warm 扱いであり、`AbortedCold` を warm 扱いすることは新しいリスクを増やさない。

### 4-h. 中断の頻度は上がる（正しい挙動だが観測すること）

`WarmupAborted` は前版の想定より高頻度になる。コルーチン内部中断（`SuspectedLiteral`/`StaleConfirm`/`vk_sent` 未設定）も `:560` 経由で段末に到達し、`recovered=true` なら注入していても `WarmupAborted` になるためである。これは**今日 `OnCold(Authorized)` に固着して `prepend_f2_warmup` が毎打鍵 true になっている状態を、明示的な再認可に置き換えたもの**であり、GJI へ送る実際のキー列は変わらない。ただしソークで件数と `StartProbe` の再発行頻度を必ず観測すること（下記「未解決の疑問」）。

なお `gate_is_bypass()` が持続する窓（TSF 注入モードのまま gate が Bypass に落ちている窓）では、決定4は**新しいループを作らない**。今日も `send_romaji_as_tsf` は打鍵ごとに `GjiWarmupCoro` を install しており（`vk_send.rs:294-318` は `StartProbe` の有無ではなく `prepend_f2_warmup` で分岐する）、その段は毎回 `:574-577` で捨てられている。決定4はそこに「段が終わったという通知」を足すだけである。**この窓ではローマ字が1文字失われる**（`Transmit`/Tsf が送信されないまま段が終わる）という実害は今日から存在し、本 ADR では直さない——「未解決の疑問」に記録する。

**証拠義務**: warmup/cold-start ファミリー、かつ BUG-27 の未解決 follow-up。

- **dispatcher（`FakeProbeIo`、Linux 実行可）**: `gate_is_bypass()==true` の batch/Tsf と per-VK `idx==0`、`chars.is_empty()`、`UpgradeToTsf`、`ProbeAction::Done` の5系列で `DispatchResult::Ended` が返り、`reason` が期待どおりであること。**per-VK 列の `idx > 0` では gate が Bypass でも `Ended` にならず、`send_single_*_vk` が呼ばれて列が続くこと**（4-c、INV-E）。既存テスト `tsf_transmit_bypass_returns_true_without_transmit`（`probe_io.rs:1182-1201`）は `is_done()` 互換アクセサでそのまま通るが、`Ended(GateBypass)` を返すアサーションを追加する。
- **`architecture_guard`**: `dispatch_probe_actions` の本体に `return DispatchResult` が0件（4-b）。`note_stage_recovery` の呼び出し元が `Output::mark_cold_raw_tsf` の1箇所だけ（4-d）。`note_stage_injection` の呼び出し元が `impl ProbeIo for Output` の注入メソッド4つだけ。
- **`TsfWarmupCoordinator`（Linux 実行可）**: `begin_stage` → `note_stage_injection` → `take_stage_record` が `{injected:true, recovered:false}` を返し、2回目の `take` が `default()` を返すこと。**段 A で `note_stage_injection` した後 `cancel_probe` 相当（`clear_pending_tsf` + `take_stage_record`）を通すと、段 B の記録が `injected=false` から始まること**（BUG-85 の回帰テスト）。`install_pending_tsf` の上書きが記録を張り直し、かつ `pending_deferred` は保持すること。
- **`gji_fsm`（Linux 実行可）**: `WarmupAborted` 受信後の遷移（`OnCold(Authorized)` → `OnCold(NotStarted)` で `kind` 保存、`OnComposing(AwaitingProbe)` → `AbortedCold` → `EndComposition` → `OnCold(NotStarted, kind 保存)`、stale `probe_id` は無視、`DiscardPending { count>0 }` が emit されること）。
- **結合ケース**: `WarmupAborted` 後の次の `KeyInput` が `StartProbe { params: kind.probe_params() }` を emit すること（決定5-b の実害はここでしか露出しない）。
- `docs/known-bugs.md` に暫定 **BUG-85**（段の記録が段をまたいで残り、注入していない probe が `WarmupComplete` を得る／`LearnedTsf` が guard と probe_id を解放しない）を起票し、BUG-27 エントリに follow-up の到達状況（dispatcher 側・coro 側とも `:560` 合流で閉じた）を追記する。

---

## 決定5: FSM の pending は単一アクセサから読み、cold の種別は運ぶ

### 5-a. `pending` を単一アクセサから読み、破棄を明示的な行為にする

`ImeOff`（`gji_fsm.rs:559-562`）と `FocusChange`（`:586-589`）の pending 件数計算は `GjiState::OnCold { pending, .. }` しか見ないが、`OnComposing { warmup: AwaitingProbe { pending, .. } }` も pending を持つ。同じ関数内の `running_probe_id()`（`:311-323`）は両方を正しく見ている——片方だけが古い形のまま取り残されている。

```rust
/// probe_id と pending 件数を同じ match から返す。
/// running_probe_id() はこれの .0 を返す薄いラッパへ変える。
fn probe_and_pending(&self) -> (Option<ProbeId>, usize);
```

アクセサ統一だけでは足りない（両アームは直後に `self.state` を上書きするため bookkeeping が失われる）。`GjiAction::DiscardPending { count, reason }` を新設し、**pending を捨てるときは必ずこのアクションを emit してから状態を上書きする**。

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingDiscardReason {
    ImeOff,
    FocusChange,
    CompositionReset,
    /// 決定4-g: 段が warm を主張できる形で終わらなかった。
    WarmupAborted,
}
```

**破棄点の完全な一覧**:

| 位置 | 現状 | 変更後 |
| --- | --- | --- |
| `gji_fsm.rs:566-570`（`ImeOff`） | `OnCold` の pending にのみ `log::warn!` | `probe_and_pending()` で数え、`DiscardPending { reason: ImeOff }` を emit |
| `:596-600`（`FocusChange`） | 同上 | `DiscardPending { reason: FocusChange }` |
| `:472-487`（`handle_composition_reset` の `OnCold` アーム） | **無警告で `pending: vec![]`** | `DiscardPending { reason: CompositionReset }` |
| `:489-500`（同 `OnWarm`/`OnComposing` かつ `Short` → `transition_to_warm`） | **無警告**（`OnComposing(AwaitingProbe)` の pending が黙って消える） | 同上 |
| `:501-511`（同 Medium/Long → `OnCold`） | **無警告で `pending: vec![]`** | 同上 |
| （新設）決定4-g の `WarmupAborted` | — | `DiscardPending { reason: WarmupAborted }` |

`CompositionReset`/`NativeF2Consumed` は `ImeOff`/`FocusChange` より高頻度であり、こちらのほうが実害が出やすい。「破棄が起きても警告すら出ない」は `ImeOff`/`FocusChange` については不正確（既に `warn!` がある）で、**本当に無警告なのは `OnComposing(AwaitingProbe)` と `handle_composition_reset` の3箇所**である。

破棄そのものは維持する（`ImeOff` はエンジン停止、`FocusChange` は宛先ウィンドウが変わっており、そのまま送ると別ウィンドウへの誤送信になる）。`INPUT_DEFER` へ戻す案は採らない（`PendingInput` は `pub romaji: String` の1フィールドのみで `RawKeyEvent` を復元できず、順序保証の合流点に入れられる型ではない）。決定4-f の deferred VK 破棄とこの表が**同じ3イベント（+ 段の中断）で対になっている**ことが、shadow と実体の一致を保つ根拠である。

### 5-b. `EndComposition` は `ColdKind` を運ぶ。`ProbeParams` は `ColdKind` の純関数にする

`gji_fsm.rs:770-788` は `AwaitingProbe` から `OnCold` へ戻す際に `ColdKind::Short` と `ProbeParams { forces_prepend_f2: false, is_long_cold: false }` を**固定値で捏造**している。元の probe が Medium/Long 想定で認可されていた場合、composition 終了だけを理由に `kind` が `Short` へ書き換わり、`forces_prepend_f2` が黙って false になる。

```rust
impl ColdKind {
    /// ProbeParams は ColdKind の純関数である。構築点はここ1箇所だけにする（INV-C）。
    pub(crate) const fn probe_params(self) -> ProbeParams {
        ProbeParams { forces_prepend_f2: self.forces_prepend_f2(), is_long_cold: self.is_long() }
    }
}

pub(crate) enum ComposingWarmup {
    AlreadyWarm,
    AwaitingProbe { probe_id: ProbeId, kind: ColdKind, pending: Vec<PendingInput> },
    /// 決定4-g: probe の段が warm を主張できる形で終わらなかった。
    /// EndComposition で kind から OnCold(NotStarted) を再構築する。
    AbortedCold { kind: ColdKind },
}
```

- `StartComposition` が `OnCold(Authorized)` から `AwaitingProbe` へ遷移するとき（`:723-735`、`transition_cold_probe_to_composing` `:441-452`）に `kind` を持ち込む。
- `EndComposition` は持ち込んだ `kind` で `OnCold { kind, probe: Authorized { probe_id, params: kind.probe_params() }, pending }` を再構築する（`AwaitingProbe` の場合。probe はまだ飛行中なので `Authorized` のまま維持する）。
- `AbortedCold { kind }` の場合は `OnCold { kind, probe: NotStarted, pending: [] }`（決定4-g と同一規則）。
- `state_label`（`gji_fsm.rs:869-891`）に `OnComposing(AbortedCold)` のアームを足す。
- 既存の `ProbeParams` 構築点3箇所（`:363-366`、`:391-394`、`:642-645`）を `kind.probe_params()` へ置き換える。**運ぶべき確定値は `kind` ただ1つになり、`params` を別途運ぶ必要が消える**。
- `tests/architecture_guard.rs` に「`ProbeParams { .. }` のリテラル構築は `ColdKind::probe_params` の中だけ」の grep guard を追加する。

**`unwrap_or_default()` による捏造も同時に閉じる（INV-C の後半、レビュー指摘を採用）**: `TsfWarmupCoordinator::current_probe_params`（`tsf_warmup_coord.rs:120-125`）は `.unwrap_or_default()` で `ProbeParams { forces_prepend_f2: false, is_long_cold: false }`——**`EndComposition` が捏造していた値とビット単位で同じもの**——を返す。grep guard はこの経路を素通しするので、リテラル構築だけを禁じても INV-C は成立しない。

```rust
// tsf_warmup_coord.rs / output/mod.rs: いずれも Option のまま返す
pub(crate) fn current_probe_params(&self) -> Option<ProbeParams>;
pub(crate) fn gji_current_probe_params(&self) -> Option<ProbeParams>;
```

唯一の読み手である `send_romaji_as_tsf`（`vk_send.rs:304-315`）が明示的に決める:

```rust
let probe_params = self.gji_current_probe_params().unwrap_or_else(|| {
    // cold パスに入ったのに GjiFsm に認可済み probe が無い（OffCold 等）。
    // 値としては Short 相当だが、これは「認可の無い cold 送信」という観測すべき不整合。
    log::warn!("[tsf-send] cold パスだが GjiFsm に Authorized probe が無い（state={}）", …);
    crate::tsf::gji_fsm::ColdKind::Short.probe_params()
});
```

挙動は不変（`Short.probe_params() == ProbeParams::default()`）だが、暗黙の潰しが明示の分岐＋ログになる。

**なぜ `gji_idle_ms` を足して `ColdKind::classify()` を呼ばないか**:

- **(i) `classify` では復元できない値がある。** composition 直後は GJI I/O が活発なので idle は必ず小さく、`classify`（`gji_fsm.rs:119-127`）は事実上 `Short` を返す。すなわち `classify` は現在の捏造値と同じ答えを返すだけで、**元の probe が `Medium`/`Long` だったという事実（＝`forces_prepend_f2: true`）を復元できない**。実害は `kind` の表示名ではなく `params.forces_prepend_f2` のほうにある。
- **(ii) 認可済み probe の `kind` は確定値としてすでに存在する（INV-B）。** 復元できるかどうか以前に、運べる確定値があるなら再導出しない。

（[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の BUG-33 追補3・4 は `gji_idle_ms` の必須パラメータ化を**是正手法として推奨**しており、`handle_composition_reset` はその推奨に従った実装である。「ルール違反だから運ぶ」ではない。）

**実害の露出条件（テスト設計上の注意）**: 捏造された `params` を読むのは `gji_current_probe_params()` の1箇所だけで、`prepend_f2_warmup` かつ `defer_if_probe_in_flight` を通過しなかった打鍵でのみ効く。純 FSM テストだけでは実害を再現できないため、決定4の結合ケース（`WarmupAborted` → 次の `KeyInput` の `StartProbe` params）と併せて固定すること。

**証拠義務**: warmup/IME belief ファミリー。`gji_fsm` の状態遷移テスト（Linux 実行可）に「`AwaitingProbe(kind=Medium)` → `EndComposition` → `OnCold` の `kind` と `params.forces_prepend_f2=true` が保存される」「`OnComposing(AwaitingProbe, pending>0)` での `ImeOff`/`FocusChange`/`CompositionReset` が `DiscardPending { count>0 }` を emit する」「`handle_composition_reset` の3アームすべてが `DiscardPending` を emit する」を追加。`ProbeParams` 構築点の grep guard と、`current_probe_params` の戻り値が `Option` のままであることの型テスト。`docs/known-bugs.md` に暫定 **BUG-86** を起票する。

---

## 実装順序

**決定4＋決定5は同一 PR。** 決定4-g の `AbortedCold { kind }` が決定5-b の `kind` 保持に依存し、決定5-b の `ColdKind::probe_params()` 一元化が決定4の結合テストの前提になるため分割しない。同 PR に閉じる変更:

1. `tsf/gji_fsm.rs`: `StageEndReason` / `GjiEvent::WarmupAborted` / `GjiAction::DiscardPending` / `ComposingWarmup::AbortedCold` / `AwaitingProbe.kind` / `ColdKind::probe_params()` / `probe_and_pending()` / `current_probe_params` の `Option` 化。
2. `output/tsf_warmup_coord.rs`: `pending_gji_warmup: Cell<bool>` → `stage: Cell<StageRecord>`、`begin_stage`/`note_stage_*`/`take_stage_record`、`install_pending_tsf` への `begin_stage()` 追加。
3. `output/probe_io.rs`: `DispatchResult` の2アーム化、ラベル付きブロック化、`flush_deferred_and_mark_warmup`/`store_gji_warmup_if_probing` 削除、`ProbeIo` の `store_gji_warmup_result` → `note_stage_injection` 改名、per-VK gate の `idx == 0` 限定。
4. `output/mod.rs`: `finish_probe_stage` / `raw_recovery_owns_deferred` 新設、`step_probe` の2アーム化、`cancel_probe` の段末処理追加。
5. `output/vk_send.rs`: `gji_current_probe_params()` の `Option` 対応。
6. `tests/architecture_guard.rs`: 3件のガード追加。

`TickableFsm` は変更しない。`tuning.rs` は変更しない。

**決定3は着手可で、決定4+5とは独立。** 初版は「[ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-a の `deliver_key_event` が入った後でないと置き場所が無い」と書いていたが、**この依存は作業ツリーでは既に解消されている**: `runtime/message_handlers.rs:64-157` に `deliver_key_event`／`KeyOrigin`／`KeyDelivery` が実在し、`runtime/engine_window.rs`（`PumpContext`・`take_needs_engine_resync`）と `hook_channel.rs` も新規追加済みで、`handle_wm_drain_output_queue`（`:989-`）の `DRAIN_PENDING`（ADR-102 決定1-d）も入っている。一方 ADR-102/105 のステータス欄は「提案（未実装）」のままであり、**ドキュメントと作業ツリーが乖離している**。決定3の実装 PR で ADR-102/105 のステータスも実態に合わせて更新すること。

どちらも `develop` 上で行う（[main-develop-branch-flow](../../.claude/rules/main-develop-branch-flow.md)）。

**BUG 番号の先行確保（実装中に2回再採番した）**: 本 ADR が使う番号は、実装先ブランチの `docs/known-bugs.md` を実際に確認しながら都度決めた。(1) 実装着手時点の `fix/adr105-102-hwnd-delivery` 派生ブランチでは BUG-80/81/82 が別件（ADR-102/105、`docs/adr-102-review-findings-design` ブランチ側の採番 BUG-78/79/84 とは既に乖離——[.claude 運用メモ](../../.claude/rules/main-develop-branch-flow.md) が警告する「書き込み先の分岐が乖離を生む」の実例）に使用済みだったため、本 ADR が当初想定していた BUG-80/82/83 を BUG-83（決定3）・BUG-84（決定4）・BUG-85（決定5）へ採番し直した。(2) その後 `develop` へのリベース時点で、developには実装期間中に別セッションが `/code-review` で ADR-105/102 の追加是正を行い、それに **BUG-83 が既に使われていた**と判明したため、**BUG-84（決定3）・BUG-85（決定4、段の記録の段またぎ）・BUG-86（決定5）**へ再度採番し直した。**develop への合流後も番号の再衝突がないか改めて確認すること**（並行セッションが同じ番号を独立採番した事故はこれで4回目である）。

## 却下した代替案

- **19件を1件ずつ直す / 早期 return の直前に `flush_deferred_and_mark_warmup` を1行足す**: `[7]` は「共通関数に括ったのに呼び忘れた出口が残った」という既に一度失敗した対症療法の再演になる。加えて `store_gji_warmup_if_probing` は無条件に warmup 完了を立てるため、「送っていないのに warm」を新規に作る。
- **`finish_probe_stage(io, target, attempt) -> DispatchResult` を6箇所から呼ぶ（前版の決定4-d）**: 実コードの3箇所は出口ではなく flush 点であり、そこを `return` に置き換えると `apply_vk_sent` / `apply_transmit_done` が飛んで per-VK confirm が全モーラで壊れる（4-a）。「呼ぶ箇所を `architecture_guard` の grep で4件に固定する」という証拠義務も、表が6行・実体が8箇所という不整合のため最初から落ちる。
- **RAII（`Drop`）で段末フックを実装する**: (i) per-VK 列は「1 tick に1 VK」のコルーチンなので、guard を `dispatch_probe_actions` のスコープに置くと段末と無関係な場所で毎 tick 発火する。(ii) 段末にやることの1つ `TsfGate::on_ready` は `Output` のフィールドであって static ではなく、`Drop` から手が届かない。(iii) `install_pending_tsf` の上書きは `pending_tsf.borrow_mut()` を保持したまま旧値が drop されるため、guard の `Drop` が `has_pending_tsf()` を呼ぶと `RefCell` の二重借用で panic する。
- **`StageEnd { recovery_owns_deferred: bool }` を dispatch 中に計算する**: 回収予約と段末が別 tick に分かれる経路で古い値を運ぶ。所有権は段末の状態照会（`raw_recovery_owns_deferred()`）で読む（4-e）。
- **`TickableFsm` に `apply_transmit_skipped` を追加して probe FSM を終端へ落とす**: 根拠だった「早期 return 後も machine が tick され続けて二重に走る」が実コードと矛盾していた（`step_probe` は machine を restore せず drop する）。加えて、デフォルト実装なしでも「ラップ型が委譲を書き忘れる」リスクは残り（BUG-27 の直接原因）、この trait を太らせる正当化にならない。
- **per-VK 列の途中で gate を再確認し、輸送手段を降格して送り切る（前版の決定4-b）**: 「列に入る前に一度だけ評価する」に対応する場所が実コードに存在せず（1 tick 1 VK）、`degraded` の保持場所も一方向ラッチの規則も未定義だった。gate を段への入場条件に限定すれば、降格機構ごと不要になる（4-c）。
- **`TransmitSkip::sink_marker()` / `deferred_sink_marker()` で marker を一元化する**: 4-c で降格をやめたため、marker を選ぶ場所は `Output::flush_pending_deferred_vks`（`output/mod.rs:1622-1635`）の1箇所しか残らない。新しい関数を足す理由が消えた。
- **`WarmupAborted` 時に `GjiAction::SendInput { pending }` を emit する（前版）**: `pending` の先頭要素は「この段を起動したローマ字」であり、`GateBypass` 中断ではそれは一度も注入されていない（`vk_send.rs:252-257` が defer 判定より前に `KeyInput` を dispatch するため）。INV-D 違反。`DiscardPending { reason: WarmupAborted }` を使う。
- **`AwaitingProbe` に `kind` と `params` の両方を持たせる**: `params` は `kind` の純関数であり（INV-C）、両方持つと両者が食い違いうる新しい不変条件を作る。
- **post-bypass latch の評価を NonText 早期 return の前へ移す**: スコープが入れば実害（別プロセスへの誤適用）は消えるため不要であり、[ADR-102](102-startup-key-delivery-one-way-closure.md) 決定1-a の枝表と矛盾し、同一プロセスの NonText 子窓で latch を1回無駄に消費する代償だけが残る。
- **post-bypass latch のスコープを pid だけにする（前版）**: `ApplicationFrameHost.exe` が複数の UWP アプリの前景フレーム窓を同一 pid で所有しうるため、UWP アプリ間の切り替えで `[6]` の実害が残る。`ForegroundScope{pid, hwnd}` にすれば追加の Win32 呼び出しなしで閉じる。
- **post-bypass latch に TTL（時間定数）を持たせる**: 「ユーザーが prefix の次を押すまでの思考時間」は実測対象にできず、[tuning-constants](../../.claude/rules/tuning-constants.md) の要求を満たせない。
- **`ScopedOneShot` をリポジトリ内の他の5件のワンショットフラグへ同時に展開する**: BUG-36/49/58/74 が積み重なった領域であり、決定4+5 と同じリリースで触るのはリスクが実利を上回る。移行候補表として記録に留める（3-a）。
- **`pending` を `INPUT_DEFER` へ戻す**: `PendingInput` は `romaji: String` の1フィールドで `RawKeyEvent` を復元できない（5-a）。

## 未解決の疑問（実機ソークで確認すること）

- **`WarmupAborted` の発火頻度と、それに続く `StartProbe` の再発行頻度**（4-h）。コルーチン内部中断も合流するため件数は増える。`GjiFsm` の状態ラベルが `OnCold(Authorized)` に張り付く事象が消えることと、`StartProbe` が打鍵ごとに出るような窓が新たに生まれないことをログで確認する。
- **`cancel_probe()` による deferred VK 破棄の件数**（4-f）。特に `CompositionReset` 由来。有意なら「破棄ではなく probe を経由して再入する」（ADR-079 Stage2 相当）を別 ADR で検討する。
- **段末 flush が BUG-38 を再演しないこと**（4-e、INV-F）。`raw_recovery_owns_deferred()` が true の間は段末が flush しないこと、`flush_raw_tsf_literal_recovery` 側が最終的に必ず回収することを、`[stage-end]` と `[raw-tsf-literal]` のログ順で確認する。
- **`gate=Bypass` が持続する TSF 注入モード窓での1文字欠落**（4-h）。`send_romaji_dispatching_on_gate`（`output/mod.rs:1470-`）の doc が記録するとおり、`send_romaji_as_tsf` が `TransmitTarget::Tsf` の probe を張って gate=Bypass でスキップされる経路は今日から存在し、そのローマ字は再送されない。決定4は FSM の固着だけを直し、この欠落は直さない。ログで `Ended(GateBypass)` の件数を数え、実運用で発生するなら別途起票する。
- **`UpgradeToTsf` 経路の実害は演繹である**（4-e）。ただし前版が書いた「Unicode 経路で Tsf 昇格すると GjiFsm が `OnCold(Authorized)` に固着する」は**誤りだった**: `GjiAction::StartProbe` ハンドラは Unicode 注入モードでその場で `WarmupComplete` を dispatch して `OnWarm` へ遷移させる（`platform.rs:514-543`）。実在する漏れは `current_gji_probe_id` と `gji_probe_guard` の未解放（4-e の1・2）であり、いずれも小さい。`WarmupAborted { reason: UpgradedToTsf }` は `running_probe_id()` 照合で stale として無視される想定であり、それが実際にそうなっていることをログで確認する。
- **`ScopedOneShot` の `ForegroundScope` が UWP/`ApplicationFrameHost` 系アプリで期待どおりに一致し続けるか**（Linux では確認できない）。ミスマッチが恒常化すると `Expired` が毎回返り、そのアプリで post-bypass 機能が無言で止まる——ログで `Expired` の連発を検知できるようにしておく。
- 通知トーストが前景を奪った状態でのフック到達順序（3-e が「latch は生き残る」と主張する前提）。
- 決定5-a の `DiscardPending` は当面「明示化とカウント」までで、破棄自体は維持する。実機で `CompositionReset` 由来の破棄件数が有意なら、`FocusChange` 時に限って romaji を再送すべきかを別 ADR で検討する。
- **実装後の Opus コードレビューで見つかった、本 ADR のスコープ外の指摘**（実装コミット後・2026-08-26）:
  - `send_romaji_as_tsf_warm`（`output/vk_send.rs`）の `LiteralDetectFsm` install が、他3箇所の `install_pending_tsf` 呼び出しと異なり `defer_if_probe_in_flight` 相当のガードを持たず、飛行中の検出窓を無警告で上書きしうる。ADR-103 の diff はこの関数に触れておらず事前から存在するコードのため本 ADR では直さない。`docs/known-bugs.md` **BUG-87** として起票済み。
  - `note_stage_injection`/`note_stage_recovery` は `ProbeIo` トレイトのメンバーではなく `Output` の本番実装内の直接呼び出しであるため、`FakeProbeIo` を使う dispatcher レベルのテストではこの配線自体（呼び出し忘れ）を検証できない（`architecture_guard` の grep 件数保証はあるが、grep はコンパイル時型検査ではない）。トレイトメンバー化も選択肢だが、`ProbeIo` の呼び出し規約全体を変える大きめの変更になるため本 ADR では見送る。
  - `ScopedOneShot` を post-bypass 以外の同型ワンショットフラグ（`ms_ime_gate_give_up` 等5件）へ展開すべきという指摘は、3-a で述べたとおり意図的なスコープ限定であり指摘としては採用しない（BUG-36/49/58/74 が積み重なった領域を同じ PR で触るリスクが実利を上回るため）。

## 設計の経緯

Opus 2体でドラフト→敵対的レビューを4ラウンド実施し初版を収束させた。主な転換点: (1) 初版の「`TransmitAttempt` の値に関わらず `flush_deferred_and_mark_warmup` を通す」は `WarmupComplete` → `OnWarm` を意味し BUG-02 型のリテラル漏れを新規に開くと判明し破棄、`WarmupAborted` を新設。(2) post-bypass latch のスコープを `focus_epoch` にする初期案が通知 churn で tmux prefix を壊すと判明しプロセス同一性へ変更。

**ラウンド5（2026-08-26）: 全指摘を実コードに突き合わせて再検証し、決定4を全面改稿した。** `is_last` ゲートが意図的な順序保証（INV-E）であること、`apply_transmit_skipped` の唯一の根拠が誤認だったこと（machine は `Done` で drop される）、`finish_probe_stage` は `GjiFsm` へ直接イベントを送れないこと（`ProbeIo` に送出口が無い二段構え）、`AbortedCold` からの再構築先が未定義だったこと、`handle_composition_reset` の破棄点3箇所が漏れていたこと、`is_modifier_like` と `is_passthrough` が別物だったこと、`post_bypass_action` の5値のうち2値しか意味論が定義されていなかったこと、`armed_pid` と `now_pid` の採取元が構造的に食い違っていたこと、`PendingInput` が romaji 1フィールドだったこと、決定5-b の理由 (ii) が先例を逆に読んでいたこと、早期 return が3箇所ではなく4箇所だったこと、「輸送手段を落とす」に production 実績があったこと、決定3の依存が既に解消済みだったこと——を反映した。

**ラウンド6（2026-08-26、本版）: 決定4を「共通関数を6箇所から呼ぶ」から「型で出口を1つにする」根本設計へ差し替え、決定3の機構を汎用プリミティブへ分解した。** 独立した2件のレビュー（改訂版への敵対的再レビュー／根本設計の再検討）を統合し、全指摘を実コードで再検証した。

**採用した指摘（実装するとバグになる／設計として不十分だった）**:

1. **`finish_probe_stage` は「唯一の出口」になれなかった（致命）。** 表の6行のうち `:616`/`:646`/`:709-712` は出口ではなく flow 途中の flush 点であり、`return` に置き換えると `apply_transmit_done` / `apply_vk_sent` が飛ぶ。特に `:709-712` を出口にすると次 tick で `run_per_vk_confirm` の「`vk_sent` 未設定 → 無リカバリで中断」分岐（`probe_fsm.rs:478-492`、BUG-27 追補2 で msedge の実機破綻を確認済み）がモーラごとに毎回発火する。→ 「出口」と「flush 点」を型で分け、段末を `break 'stage <理由>` でしか表現できない形にした（4-b）。
2. **「呼ぶのはこの4箇所だけ」と言いながら表は6行・実体は8箇所だった。** `architecture_guard` で件数を固定すると最初からテストが落ちる。→ 件数固定ではなく「本体に `return DispatchResult` が0件」というガードに変えた。
3. **`probe_io.rs:560`（`ProbeAction::Done`）が数えられていない5つ目の早期 return であり、コルーチン内部中断の合流点だった。** `ProbeCoroState::tick`（`probe_coro_state.rs:82-85`）が `CoroStep::Complete => vec![Done]` に変換するため、`vk_sent` 未設定 / `SuspectedLiteral` / `StaleConfirm` の3経路がすべてここへ来る。前版が「coro 側はスコープ外」としていた BUG-27 follow-up の残り半分が、段末を `:560` に置くだけで同時に閉じる（4-a）。
4. **`degraded` の保持場所が無く、「idx==0 へ引き上げ」と「idx>0 で降格」が両立しない。** per-VK 列は 1 tick 1 VK。→ gate を段への入場条件に限定し、降格機構ごと撤去した（4-c）。あわせて `TransmitSkip` の置き場所問題（`tsf/` が `output/` に依存する新規結合）も、`StageEndReason` を `GjiEvent` と同居させることで解消した。
5. **`WarmupAborted` 時の `SendInput { pending }` は INV-D 違反。** `send_romaji_as_tsf` は defer 判定より前に `KeyInput` を無条件 dispatch する（`vk_send.rs:252-257`）ため、`pending` の先頭は「注入されなかったローマ字」である。→ `DiscardPending { reason: WarmupAborted }` にした（4-g）。あわせて「`pending`（romaji の影）と `pending_deferred`（VK キュー）は別物」という非対称を明文化した。
6. **`AbortedCold` を別 variant にする理由が誤っていた。** `is_warm()` は `OnWarm | OnComposing` なので、`OnComposing` である限り warmup の中身に関わらず warm を返す（`warmup_strategy.rs:76-82`）。→ 正しい理由（`EndComposition` の再構築先が違う）に差し替え、`OnComposing` 中の warm 扱いは実観測 `WM_IME_STARTCOMPOSITION` に基づくので変更しない、と明記した（4-g）。
7. **`Cell<Option<ProbeStageOutcome>>` の後勝ち上書き規則が未定義だった。** → 単調な2 bool（`injected`/`recovered`）にして合成規則を自明にした（4-d）。
8. **「`LearnedTsf` で GjiFsm が固着する」は成立しない。** Unicode 注入モードでは `StartProbe` ハンドラがその場で `WarmupComplete` を dispatch する（`platform.rs:514-543`）。→ 実在する漏れ（`gji_end_probe_guard` と `take_probe_id` の未実行）に主張を差し替えた。また「`UpgradeToTsf` 出口で deferred を flush すると生 VK を撃ち込む」という懸念は、Unicode 注入モードでは `defer_if_probe_in_flight` の呼び出し元が存在しない（`vk_send.rs:131`/`:295`/`:363` はいずれも Vk/Tsf/MS-IME 経路）ためキューが空であり、実害が無いことを確認した。
9. **`ApplicationFrameHost.exe` が複数 UWP アプリの前景窓を同一 pid で所有しうる。** → スコープを `ForegroundScope{pid, hwnd}` へ拡張した（追加の Win32 呼び出しゼロ、3-b）。
10. **`vk_is_modifier` は `vk_is_passthrough` の真部分集合であり、判定表は排他ではなかった。** → 順序依存であることを doc とテストで固定した（3-c）。
11. **「#1 の早期 return があるので毎打鍵 Win32 は増えない」は純関数の形では成立しない。** `now_pid` を引数で渡す設計では呼び出し側の遅延評価が本文の主張と食い違いうる。→ `is_armed()` で絞ってから `foreground_scope()` を呼ぶ形を疑似コードとして固定した（3-b/3-d）。
12. **grep guard は `ProbeParams::default()` を素通しし、INV-C は成立しなかった。** `current_probe_params()` の `unwrap_or_default()` が `EndComposition` の捏造値とビット単位で同じ値を返していた。→ `Option` のまま返し、唯一の読み手が明示的に決める形にした（5-b）。
13. **「BUG-81 は ADR-104 が起票済み」は誤り。** BUG-80/81/82/83 はいずれも未起票（当時参照していた main tree 側の番号）。→ 訂正した。**さらに本ラウンドで、実装先ブランチでは当初想定の BUG-80/82/83 自体が既に別件で使用済みと判明し、BUG-83/84/85 へ再採番した**（上記「実装順序」参照）。
14. **決定4-b「`apply_vk_sent` は変えない」と決定4-d「`:709-712` が出口」が同じ行について矛盾していた。** → 1 の改訂で解消。
15. **`pending_gji_warmup: Cell<bool>` が `cancel_probe()` でクリアされず段をまたぐ（未起票の潜在バグ）。** 段 A が立てた bool が段 B の最初の `Done` で消費され、1文字も注入していない段 B の probe が `WarmupComplete` を得る。→ `begin_stage()` を `install_pending_tsf` に置き、`cancel_probe` で `take_stage_record()` する形で閉じ、**BUG-85** として起票する（4-d）。
16. **`DispatchResult::LearnedTsf` アームが `gji_end_probe_guard()` を呼んでいなかった。** → `Ended` アームへの統合で閉じる（4-e）。

**本統合で新たに見つけ、両レビューの提案を訂正した点**:

- **`begin_stage()` で `probe_id` を捕まえる案は成立しない。** `install_pending_tsf`（`output/send_keys` の中）は `gji_store_probe_id`（`drain_output_post_send_effects` の中、`platform.rs:905-922`）より**先**に走るため、段の開始時に読める `current_gji_probe_id` は前の段のものか `None` である。→ probe_id は現行どおり段末に `take_probe_id()` で読む（4-d）。
- **`StageEnd { recovery_owns_deferred }` を dispatch 中に計算する案は tick をまたぐと壊れる。** → `raw_recovery_owns_deferred()` として段末に状態照会する形に変え、INV-F として明文化した（4-e）。
- **`note_stage_recovery` を dispatcher に書くと、忘れたときに危険側（warm 誤申告）へ倒れる。** → `RawTsfLiteralRecovery` アームの全分岐が無条件に通る `Output::mark_cold_raw_tsf`（`probe_io.rs:844` が唯一の呼び出し元）の中に置き、`architecture_guard` で呼び出し元数を固定した（4-d）。
- **`cancel_probe()` が deferred VK キューを放置していると、決定4が閉じたはずの順序反転が cancel 経路にだけ残る。** → `GjiFsm` の `pending` を破棄する3イベントと同じ集合で VK キューも破棄する形に揃えた（4-f）。件数観測をソーク項目にした。

**却下した指摘**:

- **`Skipped(GateBypass)` が毎打鍵の abort ループを新設する**: 前半は誤り。今日も `send_romaji_as_tsf` は `prepend_f2_warmup` で分岐して打鍵ごとに `GjiWarmupCoro` を install しており、その段は毎回 `:574-577` で捨てられている——churn は既に存在する。決定4はそこに通知を足すだけで、GJI へ送るキー列は変えない。ただし「件数を観測すべき」という含意は採用し、未解決の疑問に入れた。また、この窓ではキューが空なので「毎打鍵の deferred flush」も起きない。
- **`is_warm()` を `AbortedCold` で false にする / `WarmupAborted` 受信時に `OnComposing` を抜けて `OnCold` へ落とす**: `OnComposing` は `WM_IME_STARTCOMPOSITION` という実観測で入る状態であり、probe の中断はその観測を無効化しない。`AwaitingProbe` 中も現行は warm 扱いであり、ここを変えると「合成中なのに F2 を prepend する」という新しい経路を作る。指摘のうち「別 variant にする理由の記述が誤っている」という部分だけを採用した。
- **`ProbeIo` トレイトの破壊的変更を避けるべき、という含意**: `store_gji_warmup_result` → `note_stage_injection` の改名と `mark_cold_raw_tsf` の意味追加は必要である（bool では「中断」と「降格」を運べない）。`ProbeIo` は本番実装1つ＋テスト fake 1つで、`TickableFsm`（7実装＋ラップ型）とは委譲漏れのリスクが桁違いに違う。
- **`ScopedOneShot` をリポジトリ内の他5件へ同時展開する**: 移行表として記録するに留める（3-a）。
