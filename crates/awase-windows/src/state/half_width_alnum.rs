use awase::config::HalfWidthAlnumTogglePolicy;
use awase::types::VkCode;

use crate::state::foreground_scope::ForegroundScope;
use crate::state::TickMs;

/// 左Shift単独タップによる「IME-ON 半角英数」持続トグルの次アクション。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfWidthAlnumAction {
    None,
    Enter,
    Exit,
}

/// このKeyUpを起こしたShiftキーの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKeyUpKind {
    /// 左Shift、他の物理キーを一切介さない単独タップ。
    LeftTap,
    /// 左Shift、押下中に他の物理キーが挟まった（例: Shift+K のチョード）。
    LeftChord,
    /// 右Shift、他の物理キーを一切介さない単独タップ（緊急解除）。
    RightTap,
    /// 右Shift、押下中に他の物理キーが挟まった（例: Shift+K のチョード）。
    RightChord,
}

/// 半角英数持続トグルの entry/exit を純粋に計画する。
///
/// `toggle_active` が真のとき:
/// - 左右いずれかの**単独タップ**（`LeftTap`/`RightTap`）→ exit。左Shiftは
///   2回目タップとしてのトグルOFF、右Shiftは「緊急解除」——意味付けは違うが
///   どちらも exit する点は同じ。
/// - 左右いずれかの**チョード**（`LeftChord`/`RightChord`、例: Shift+K で
///   大文字を打つ）→ **exit しない**。半角英数トグルは「押しながらの他キー
///   入力」を大文字化するための一時的な Shift 修飾として使えるべきで、
///   Shift を離しただけでトグルが解除されてはユーザーが意図せず持続モード
///   から抜けてしまう（実機で報告された不具合。当初は左Shiftのみ対称に
///   修正していたが、右Shiftチョードで同じ不具合が再現することが分かり
///   左右対称に修正した）。
///
/// `toggle_active` の判定は `entry_supported` より**常に優先する**——
/// `entry_supported` は「新たに entry してよいか」だけを制御する条件であり、
/// 既に active な状態からの脱出をブロックしてはならない（entry 後に
/// IME 種別・belief・kill switch などが変化して `entry_supported` が
/// false に転じても、緊急解除で必ずかなへ戻れることを保証する）。
///
/// composition 中の entry ブロック（ADR-107 決定5の当初案）は撤去した
/// （known-bugs.md BUG-25追補5・追補10: 実機検証で preedit 非破壊・成功が
/// 再現し、ユーザーからもComposition中の発火を求める報告があったため）。
/// composition/候補ウィンドウ表示の状態はこの純粋関数の関知するところでは
/// なくなった。
#[must_use]
pub const fn plan_half_width_alnum_action(
    shift_up: ShiftKeyUpKind,
    toggle_active: bool,
    entry_supported: bool,
) -> HalfWidthAlnumAction {
    if toggle_active {
        if matches!(
            shift_up,
            ShiftKeyUpKind::LeftChord | ShiftKeyUpKind::RightChord
        ) {
            return HalfWidthAlnumAction::None;
        }
        return HalfWidthAlnumAction::Exit;
    }
    if entry_supported && matches!(shift_up, ShiftKeyUpKind::LeftTap) {
        return HalfWidthAlnumAction::Enter;
    }
    HalfWidthAlnumAction::None
}

/// このKeyUpを起こした物理Shiftの左右。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftSide {
    Left,
    Right,
}

/// [`HalfWidthAlnumState::on_shift_up`] が返す、呼び出し元が実行すべき副作用。
///
/// 実際の書き込み（IMC conv write / GJI SendInput / かな復元）は呼び出し元
/// （`runtime/key_pipeline.rs`）が担う。この型は「何をすべきか」の計画結果
/// のみを表し、副作用そのものは持たない（`plan_half_width_alnum_action` と
/// 同じ純粋計画の原則）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfWidthAlnumEffect {
    Nothing,
    EnterViaImcWrite,
    EnterViaGjiSendInput,
    /// 復元を実行する。`ownership` は「この exit が OS 書き込みの権利を誰から得たか」
    /// （ADR-245 決定4・6）。通常の左 Shift 2 回目タップ・右 Shift 緊急解除は
    /// `toggle_held == true` からなので [`ExitOwnership::FromToggle`]、離脱で戻り待ちへ
    /// 積まれたトグル（`toggle_held` は偽）を戻った窓で解く場合は
    /// [`ExitOwnership::FromResume`]。
    ExitRestoreKana {
        ownership: ExitOwnership,
    },
}

/// [`HalfWidthAlnumEffect::ExitRestoreKana`] の exit が OS 書き込みの権利をどこから得たか
/// （ADR-245 決定4）。`begin_restore_kana()` の旧値（= 直前の `toggle_held`）が偽だと
/// 復元本体は「already inactive」で送信を全部飛ばす（`key_pipeline.rs`）。戻り待ちから
/// 取り出した exit では `toggle_held` が既に偽なので、取り出しそのものが INV-B の
/// 「true→false の遷移 1 回」の役を担う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOwnership {
    /// `toggle_held` が真だった（`begin_restore_kana` の旧値が権利）。
    FromToggle,
    /// 戻り待ちのエントリを取り出した（取り出しが権利。`begin_restore_kana` の旧値は見ない）。
    FromResume,
}

/// この exit が OS 書き込みを持つか（ADR-245 決定4・検証 B-1）。殻はこの結果だけで分岐する。
/// `was_toggle_active` は `begin_restore_kana()` の戻り値。
#[must_use]
pub const fn exit_owns_write(ownership: ExitOwnership, was_toggle_active: bool) -> bool {
    match ownership {
        ExitOwnership::FromToggle => was_toggle_active,
        ExitOwnership::FromResume => true,
    }
}

/// 左右Shift単独タップによる「IME-ON 半角英数」持続トグルの全状態を1箇所に
/// 集約する。
///
/// 旧 `GateStore` の4フィールド（`left_shift_tap_candidate` /
/// `right_shift_tap_candidate` / `shift_conv_guard_pending` /
/// `half_width_alnum_toggle_active`）と、旧 `Runtime::
/// half_width_alnum_toggle_policy` フィールドを統合したもの。
///
/// フィールドは全て private。`.claude/rules/ime-belief-architecture.md` の
/// 「蓄積する値は書き込み経路を1箇所の関数に集約し、フィールドを private
/// 化する」という方針に従い、`GateStore`/`Runtime` を含む本モジュール外から
/// は以下のメソッド経由でのみ読み書きできる
/// （`crates/awase-windows/tests/architecture_guard.rs` が生フィールド名の
/// 本番コードからの出現数を 0 に固定する）。
#[derive(Debug, Default)]
pub struct HalfWidthAlnumState {
    /// 今回の左Shift downが単独タップ候補か。左Shift KeyDownでtrueにセット
    /// し、Shift保持中に他の非注入物理KeyDownが来たらfalseに倒す
    /// （チョード判定）。
    left_tap_armed: bool,
    /// `left_tap_armed` と対称の右Shift版。
    right_tap_armed: bool,
    /// 今回のShift downに対応する復元処理が必要か。`toggle_held`とは独立
    /// （トグルON中のShift downでも必ずtrueにする必要がある——立てないと
    /// KeyUp側でトグルOFF/右Shift緊急解除が発火しなくなる）。
    conv_guard_pending: bool,
    /// 左Shift単独タップによる「IME-ON半角英数」持続トグルが有効か。
    toggle_held: bool,
    /// `config.general.half_width_alnum_toggle` を反映するkill switch。
    entry_policy: HalfWidthAlnumTogglePolicy,
    /// 離脱時に IME へ何も送らず積んだ、戻り待ちのトグル（ADR-245 決定1・2）。
    return_pending: ReturnPending,
}

impl HalfWidthAlnumState {
    // ── 設定 ──────────────────────────────────────────────────────────

    /// `config.general.half_width_alnum_toggle` を反映する。起動時
    /// （`app/bootstrap.rs`）と設定リロード時（`apply_config_update`）の
    /// 両方から呼ぶこと（`Runtime::set_half_width_alnum_toggle_policy` 経由。
    /// `architecture_guard.rs` の reload guard テストがこの対称性を固定する）。
    pub fn set_policy(&mut self, policy: HalfWidthAlnumTogglePolicy) {
        self.entry_policy = policy;
    }

    // ── 物理キー観測 ──────────────────────────────────────────────────

    /// Shift以外の物理キーDownで、単独タップ候補を左右対称に折る。
    ///
    /// KeyDown/injected（BUG-14由来の自己注入除外）の判定は呼び出し側
    /// （`key_pipeline.rs::kp_stage_shift_conv_guard`）に残す。この関数は
    /// vkから「反対側の候補を折る」左右判定ロジックのみを持つ——engine層の
    /// イベント種別の意味論をstate層に持ち込まないため。
    pub fn note_physical_key_down(&mut self, vk: VkCode) {
        if vk != crate::vk::VK_LSHIFT {
            self.left_tap_armed = false;
        }
        if vk != crate::vk::VK_RSHIFT {
            self.right_tap_armed = false;
        }
    }

    pub fn arm_tap(&mut self, side: ShiftSide) {
        match side {
            ShiftSide::Left => self.left_tap_armed = true,
            ShiftSide::Right => self.right_tap_armed = true,
        }
    }

    pub fn arm_guard(&mut self) {
        self.conv_guard_pending = true;
    }

    pub fn disarm_guard(&mut self) {
        self.conv_guard_pending = false;
    }

    /// 旧 `mem::take(&mut gate.shift_conv_guard_pending)` 相当。
    pub fn take_guard(&mut self) -> bool {
        std::mem::take(&mut self.conv_guard_pending)
    }

    /// 読み取り専用（`kp_stage_shift_conv_guard`/`ir_decide_read_strategy` の
    /// `||` 左辺で使う）。
    #[must_use]
    pub const fn is_guard_pending(&self) -> bool {
        self.conv_guard_pending
    }

    // ── Shift KeyUp 判定 ──────────────────────────────────────────────

    /// このKeyUpが単独タップかチョードかを判定し、**左右両方の候補を
    /// disarmする**。
    ///
    /// 現行 `key_pipeline.rs` は KeyUp がどちらの Shift でも left/right
    /// 両方を `mem::take` している。`side` だけ disarm する実装に変えると
    /// BUG-25追補11の左右非対称（右Shiftチョードが常にExit扱いだった不具合）
    /// が再発するため、「両方disarmする」ことをメソッド名自体に刻む。
    ///
    /// モジュールprivate（`pub`ではない）: entry/exit判定は必ず`on_shift_up`
    /// を経由させ、この判定だけを外部から個別に呼んで`on_shift_up`の
    /// policy/entry_ime_ok判定を迂回できないようにする（テストは同一
    /// モジュール内の子`mod tests`からアクセスするため`pub`は不要）。
    fn take_shift_up_kind_disarming_both(&mut self, side: ShiftSide) -> ShiftKeyUpKind {
        let was_left = std::mem::take(&mut self.left_tap_armed);
        let was_right = std::mem::take(&mut self.right_tap_armed);
        match side {
            ShiftSide::Left => {
                if was_left {
                    ShiftKeyUpKind::LeftTap
                } else {
                    ShiftKeyUpKind::LeftChord
                }
            }
            ShiftSide::Right => {
                if was_right {
                    ShiftKeyUpKind::RightTap
                } else {
                    ShiftKeyUpKind::RightChord
                }
            }
        }
    }

    // ── Enter判定・確定 ───────────────────────────────────────────────

    /// Shift KeyUp を起点に「次に何をすべきか」を計画する。
    ///
    /// `entry_ime_ok` は `effective_open() && is_japanese_ime() &&
    /// is_user_enabled()` の3条件のみをまとめた1個のbool——このモジュールが
    /// 直接観測しない外部条件（`ImeStateHub`/`Engine`）を呼び出し元が事前に
    /// 集約したもの。**`Output::conv_mutation_allowed` は含まない**——
    /// conv書込権限はentry条件ではなく、呼び出し元
    /// `kp_stage_shift_conv_guard` 側の disarm_guard（一度 arm_guard した
    /// pending を降ろす側）にのみ効く、別軸の判定である。`uses_imc_conv_write`
    /// はアクティブIMEがMS-IME（IMC write可）かどうか——`entry_policy` が
    /// `MsImeOnly` のときの entry 可否と、Enter時にIMC書き込み経路とGJI
    /// SendInput経路のどちらを選ぶかの**両方**に使う。
    ///
    /// `pending_for_scope`（ADR-245 決定6）は「現在の前面スコープに一致する戻り待ちがある」
    /// 事実。真なら `toggle_held` が偽でも active として扱い、単独タップを Exit にする。
    /// 内部で `active = toggle_held || pending_for_scope` を
    /// [`plan_half_width_alnum_action`] へ渡す（純関数本体は変えない）。`Exit` を
    /// [`HalfWidthAlnumEffect::ExitRestoreKana`] に写すときだけ、`toggle_held` が偽で
    /// `pending_for_scope` が真なら [`ExitOwnership::FromResume`] にする。
    /// `toggle_held` と `pending_for_scope` が同時に真になる経路は無い。
    pub fn on_shift_up(
        &mut self,
        side: ShiftSide,
        entry_ime_ok: bool,
        uses_imc_conv_write: bool,
        pending_for_scope: bool,
    ) -> HalfWidthAlnumEffect {
        let shift_up_kind = self.take_shift_up_kind_disarming_both(side);
        let policy_allows_entry = match self.entry_policy {
            HalfWidthAlnumTogglePolicy::Off => false,
            HalfWidthAlnumTogglePolicy::MsImeOnly => uses_imc_conv_write,
            HalfWidthAlnumTogglePolicy::All => true,
        };
        let toggle_entry_supported = policy_allows_entry && entry_ime_ok;
        let toggle_held = self.toggle_held;
        let active = toggle_held || pending_for_scope;
        match plan_half_width_alnum_action(shift_up_kind, active, toggle_entry_supported) {
            HalfWidthAlnumAction::None => HalfWidthAlnumEffect::Nothing,
            HalfWidthAlnumAction::Enter => {
                if uses_imc_conv_write {
                    HalfWidthAlnumEffect::EnterViaImcWrite
                } else {
                    HalfWidthAlnumEffect::EnterViaGjiSendInput
                }
            }
            HalfWidthAlnumAction::Exit => HalfWidthAlnumEffect::ExitRestoreKana {
                ownership: if !toggle_held && pending_for_scope {
                    ExitOwnership::FromResume
                } else {
                    ExitOwnership::FromToggle
                },
            },
        }
    }

    /// IMC(MS-IME)経路のcommit。呼び出し元は現行`key_pipeline.rs`と同じく
    /// `actuate_conv_mode`呼び出しの**前**に無条件で呼ぶこと（順序を変えると
    /// 挙動変更になる、§3原則2）。
    pub fn commit_enter_imc(&mut self) {
        self.toggle_held = true;
    }

    /// GJI経路のcommit。呼び出し元は`send_gji_half_width_alnum_toggle`が
    /// `true`を返した場合のみ呼ぶこと（真のcommit-on-success、§3原則2）。
    pub fn commit_enter_gji(&mut self) {
        self.toggle_held = true;
    }

    // ── Exit/Restore ──────────────────────────────────────────────────

    /// 「IME-ON半角英数」からかな入力への復元を開始する。
    ///
    /// 旧 `mem::replace(&mut gate.half_width_alnum_toggle_active, false)`
    /// 相当。戻り値は直前の `toggle_held`（= 実際にOS書き込みが必要かの
    /// 判定に使う）。
    pub fn begin_restore_kana(&mut self) -> bool {
        std::mem::replace(&mut self.toggle_held, false)
    }

    /// GJI exitのSendInputが見送られた場合の巻き戻し。
    ///
    /// `begin_restore_kana` で false にした `toggle_held` を true に戻し、
    /// 次のタップ/緊急解除で再試行できるようにする（旧
    /// `kp_send_gji_restore_exit` の `gate.half_width_alnum_toggle_active =
    /// true` 相当）。
    pub fn rearm_after_failed_gji_exit(&mut self) {
        self.toggle_held = true;
    }

    /// 直接観測（ADR-188）が実状態の「かな」（NATIVE の読み）へ追随したとき、持続トグルを OS 書き込みなしで手放す
    /// （ADR-244 D3）。戻り値は直前の `toggle_held`。
    ///
    /// `begin_restore_kana` と違い復元の SendInput/IMC 書き込みを伴わない——IME 側のモードキーが次のモードを既に
    /// 決めている。`note_explicit_ime_action` も呼ばない（呼ぶと同じ窓の後続の読みが ADR-188 R3 で捨てられる）。
    /// 手放さないと凍結（`ShiftConvGuard`）が続き、次の左 Shift タップが「開始」でなく「解除」になる。
    pub fn abandon_on_observed_follow(&mut self) -> bool {
        std::mem::replace(&mut self.toggle_held, false)
    }

    // ── 戻り待ち（ADR-245） ───────────────────────────────────────────

    /// 離脱で、持続トグルを IME へ何も送らずに戻り待ちへ積む（ADR-245 決定1・2）。
    ///
    /// `toggle_held` を下ろし（belief の補正は殻の仕事）、`scope`（トグルに入った窓。離脱の時点の
    /// 前面窓は既に移動先なので殻が Enter 時に控えた値を渡す）・IME 種別・時刻のエントリを積む。
    /// トグルが立っていなければ何もせず `None`。積んだときは容量超過で捨てた古いエントリの件数
    /// （ログは殻が出す）を `Some` で返す。
    pub fn suspend_toggle_for_return(
        &mut self,
        scope: ForegroundScope,
        uses_imc_conv_write: bool,
        now: TickMs,
    ) -> Option<usize> {
        if !self.begin_restore_kana() {
            return None;
        }
        Some(self.return_pending.push(ReturnPendingEntry {
            scope,
            uses_imc_conv_write,
            at: now,
        }))
    }

    /// 戻り待ちが 1 件でもあるか。空のときは殻が打鍵ごとの `foreground_scope()` を呼ばない（ADR-245 決定2）。
    #[must_use]
    pub const fn has_return_pending(&self) -> bool {
        !self.return_pending.is_empty()
    }

    /// `scope` と完全一致（pid と hwnd の両方）する戻り待ちを取り出さずに見る。
    #[must_use]
    pub fn find_return_pending(&self, scope: ForegroundScope) -> Option<ReturnPendingEntry> {
        self.return_pending.find_for_scope(scope)
    }

    /// `scope` と完全一致する戻り待ちを取り出す（INV-B の「1 回」の役。取り出しと同時にエントリが消える）。
    pub fn take_return_pending(&mut self, scope: ForegroundScope) -> Option<ReturnPendingEntry> {
        self.return_pending.take_for_scope(scope)
    }

    /// 寿命（`max_age_ms`。殻は `tuning::HWND_CACHE_MAX_AGE_MS` を渡す）を超えた戻り待ちを捨て、捨てた件数を返す。
    pub fn prune_return_pending(&mut self, now: TickMs, max_age_ms: u64) -> usize {
        self.return_pending.prune_expired(now, max_age_ms)
    }

    // ── 状態照会 ──────────────────────────────────────────────────────

    #[must_use]
    pub const fn is_toggle_active(&self) -> bool {
        self.toggle_held
    }
}

/// 直接観測の追随の結果から、持続トグルを手放すべきか（ADR-244 D3・D4）。
///
/// - 手放すのは **Microsoft IME 本体で、観測された読みが NATIVE（`eisu == Some(false)`）で、追随の後に belief が
///   英数でなくなっているとき**だけ。`belief_left_eisu` は追随（`InputModeObserved`）が reducer の予測の fence
///   （`KEY_EFFECT_SETTLE_MS` 以内の予測が優先）に捨てられなかったか。捨てられて belief が英数のまま残ったのに
///   トグルだけ手放すと、「belief は英数・トグルは無し」になり、次の Shift タップが「開始」になる。
/// - キー押下を根拠にはしない（`024ca336`〜`6b1e91b8` の「素通しのキーが来たら無条件に手放す」案は、読みが届かず
///   `か` のままで、GJI の MS-IME プリセットに `ro` を出して戻した）。
/// - GJI は手放さない（挙動を変えない。GJI の追随で `toggle_held` が残る潜在課題は BUG-186 の範囲外）。
/// - 英数の読み（`Some(true)`）や追随なしでは手放さない（全角英数 `conv=0x18` は NATIVE が無く英数のまま）。
#[must_use]
pub fn should_abandon_on_observed_follow(
    kind: crate::state::ime_kind::ImeKindId,
    eisu: Option<bool>,
    belief_left_eisu: bool,
) -> bool {
    matches!(kind, crate::state::ime_kind::ImeKindId::MsIme)
        && eisu == Some(false)
        && belief_left_eisu
}

// ── 戻り待ち（ADR-245、BUG-193） ──────────────────────────────────────
//
// 窓 A で持続半角英数にしたまま別プロセスの窓 B へ移ると、離脱時の強制復元は世代の bump で
// 必ず中断し、F2 等は移動先 B に届く。ADR-245 は離脱時に IME へ何も送らず、A を覚えておいて、
// A に戻って最初の打鍵の手前で復元する。このブロックはその判断の核（純関数と小さな型）。
// 事実の収集（前面スコープ・物理の修飾キー・予測の答え等）と OS への書き込みは殻（PR 2）の仕事。

/// 戻り待ちの最大件数（ADR-245 決定2）。超えたら古いものから捨てる。
pub const RETURN_PENDING_CAPACITY: usize = 4;

/// 戻り待ちの 1 件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnPendingEntry {
    /// トグルに入った窓。照合は `win32::foreground_scope()` 1 本で作った値同士（pid と hwnd の完全一致）。
    pub scope: ForegroundScope,
    /// Enter 時の IME 種別（`on_shift_up` の `uses_imc_conv_write` と同じ。真 = MS-IME 本体、偽 = GJI）。
    pub uses_imc_conv_write: bool,
    /// 積んだ時刻。寿命の判定に使う。
    pub at: TickMs,
}

/// 戻り待ちの集合（容量 [`RETURN_PENDING_CAPACITY`]、古い順）。
#[derive(Debug, Default)]
pub struct ReturnPending {
    entries: Vec<ReturnPendingEntry>,
}

impl ReturnPending {
    /// 積む。同じスコープの古いエントリは置き換える（同じ窓が二重に待たない）。容量を超えたら古いものから
    /// 捨て、**捨てた件数**を返す（置き換えは数えない。ログは殻が出す）。
    pub fn push(&mut self, entry: ReturnPendingEntry) -> usize {
        self.entries.retain(|e| e.scope != entry.scope);
        self.entries.push(entry);
        let excess = self.entries.len().saturating_sub(RETURN_PENDING_CAPACITY);
        drop(self.entries.drain(..excess));
        excess
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// `scope` と完全一致するエントリ。pid だけが一致する別窓は対象にしない。
    #[must_use]
    pub fn find_for_scope(&self, scope: ForegroundScope) -> Option<ReturnPendingEntry> {
        self.entries.iter().find(|e| e.scope == scope).copied()
    }

    /// `scope` と完全一致するエントリを取り出す。他のエントリは保持する（別窓＝保持）。
    pub fn take_for_scope(&mut self, scope: ForegroundScope) -> Option<ReturnPendingEntry> {
        let idx = self.entries.iter().position(|e| e.scope == scope)?;
        Some(self.entries.remove(idx))
    }

    /// 寿命切れのエントリを捨て、捨てた件数を返す。
    pub fn prune_expired(&mut self, now: TickMs, max_age_ms: u64) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|e| !return_pending_expired(e.at, now, max_age_ms));
        before - self.entries.len()
    }
}

/// 戻り待ちが寿命を超えたか。`max_age_ms` は殻が `tuning::HWND_CACHE_MAX_AGE_MS` を渡す
/// （新しい時間定数は作らない、ADR-245 決定2）。ちょうど `max_age_ms` は有効。
#[must_use]
pub const fn return_pending_expired(at: TickMs, now: TickMs, max_age_ms: u64) -> bool {
    now.saturating_sub(at.0) > max_age_ms
}

/// 判断点 (a): 離脱（`ir_notify_focus_changed`）で何をするか（ADR-245 決定1・10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeavePlan {
    /// IME へは何も送らず、トグルを戻り待ちへ積む（belief を戻すのは従来どおり殻）。
    Suspend,
    /// 何もしない。
    Nothing,
}

/// 離脱の計画。トグルが立っていて、戻りの照合に使える（有効な）スコープがあるときだけ `Suspend`。
/// 無効なスコープ（取得失敗）は実在のスコープと決して一致しないので、積んでも戻れない。
#[must_use]
pub const fn plan_leave(toggle_active: bool, entry_scope: ForegroundScope) -> LeavePlan {
    if toggle_active && entry_scope.is_valid() {
        LeavePlan::Suspend
    } else {
        LeavePlan::Nothing
    }
}

/// [`KeyStagePlan::Drop`] の理由。ログ・`basis` で区別する（ADR-245 決定5 の S4-1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// 「IME モードの役割」のキー。ユーザーが選んだモードを上書きしない。
    ImeModeRole,
    /// `effective_open()` が偽（B から持ち越された偽。belief が閉なのにトグルを立て直すと辻褄が合わない）。
    EffectiveOpenFalse,
    /// Enter 時と照合時の IME 種別が違う。
    ImeKindMismatch,
    /// 現在の窓が `app_disabled`。
    AppDisabled,
    /// 寿命切れ。
    Expired,
}

impl DropReason {
    /// ログ・`basis` 用の固定文字列。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ImeModeRole => "IME モードの役割のキー",
            Self::EffectiveOpenFalse => "effective_open 偽(B から持ち越し)",
            Self::ImeKindMismatch => "IME 種別不一致",
            Self::AppDisabled => "app_disabled",
            Self::Expired => "寿命切れ",
        }
    }
}

/// 判断点 (b): 戻り待ちがスコープと一致した窓での、打鍵の段の結果（ADR-245 決定5、優先順位は上から）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStagePlan {
    /// 何もしない（エントリは残す）。修飾キー自身の KeyDown/KeyUp。
    Nothing,
    /// 送信なしでエントリを捨てる。
    Drop(DropReason),
    /// 注入せずにトグルを立て直し、belief を ObservedEisu にする（GJI でひらがなキーが SET と確かめられないとき）。
    RebuildToggle,
    /// 物理の修飾キーが押されている。エントリを残して次の打鍵で判断する。
    Deferred,
    /// 復元を実行する（`ExitOwnership::FromResume`）。
    Resume,
}

impl KeyStagePlan {
    /// この結果でエントリを消費する（戻り待ちから取り出す）か。`Nothing`・`Deferred` は残す。
    #[must_use]
    pub const fn consumes_entry(self) -> bool {
        matches!(self, Self::Drop(_) | Self::RebuildToggle | Self::Resume)
    }
}

/// [`plan_key_stage`] に渡す事実。殻が集める。
#[derive(Debug, Clone, Copy)]
pub struct KeyStageFacts {
    /// スコープが一致した戻り待ちのエントリ。
    pub entry: ReturnPendingEntry,
    /// 打鍵の時刻。
    pub now: TickMs,
    /// 寿命（`tuning::HWND_CACHE_MAX_AGE_MS`）。
    pub max_age_ms: u64,
    /// 現在の IME 種別（`entry.uses_imc_conv_write` と同じ意味）。
    pub uses_imc_conv_write_now: bool,
    /// 修飾キー（Shift/Ctrl/Alt/Win）自身の KeyDown/KeyUp か。
    pub is_modifier_key: bool,
    /// `enrich_key_role` が IME モードの役割を付けた、または `matches_ime_set_open`/`matches_ime_off` が真のキー。
    /// 修飾キーの有無を問わない。
    pub is_ime_mode_role_key: bool,
    /// 親指キーの役割が付いたキー（`is_ime_mode_role_key` から除く）。
    pub is_thumb_key: bool,
    pub effective_open: bool,
    pub app_disabled: bool,
    /// GJI のひらがなキー（F2）が SET か。`hiragana_key_is_set` の答え（予測なしは `None`）。MS-IME 本体では見ない。
    pub hiragana_key_is_set: Option<bool>,
    /// Shift/Ctrl/Alt/Win のいずれかを物理的に押している。
    pub physical_modifier_down: bool,
}

/// 打鍵の段の判定（ADR-245 決定5）。次の優先順位で上から決める。Drop・RebuildToggle を Deferred より先に
/// 判定するのは、Ctrl+変換でひらがなに戻した A に、後から RebuildToggle や F2 を重ねないため。
///
/// 1. 修飾キー自身 → `Nothing`
/// 2. IME モードの役割のキー（親指キーを除く） → `Drop(ImeModeRole)`
/// 3. effective_open 偽・IME 種別不一致・app_disabled・寿命切れ → `Drop(..)`
/// 4. GJI で F2 が SET と確かめられない → `RebuildToggle`（修飾キーの有無を問わない）
/// 5. 物理の修飾キー押下中 → `Deferred`
/// 6. それ以外（親指キー+文字を含む） → `Resume`
#[must_use]
pub const fn plan_key_stage(f: &KeyStageFacts) -> KeyStagePlan {
    if f.is_modifier_key {
        return KeyStagePlan::Nothing;
    }
    if f.is_ime_mode_role_key && !f.is_thumb_key {
        return KeyStagePlan::Drop(DropReason::ImeModeRole);
    }
    if !f.effective_open {
        return KeyStagePlan::Drop(DropReason::EffectiveOpenFalse);
    }
    if f.entry.uses_imc_conv_write != f.uses_imc_conv_write_now {
        return KeyStagePlan::Drop(DropReason::ImeKindMismatch);
    }
    if f.app_disabled {
        return KeyStagePlan::Drop(DropReason::AppDisabled);
    }
    if return_pending_expired(f.entry.at, f.now, f.max_age_ms) {
        return KeyStagePlan::Drop(DropReason::Expired);
    }
    // GJI（MS-IME 本体は IMC の SET と VK_DBE_HIRAGANA の SET なので判定は不要）。
    if !f.entry.uses_imc_conv_write && !matches!(f.hiragana_key_is_set, Some(true)) {
        return KeyStagePlan::RebuildToggle;
    }
    if f.physical_modifier_down {
        return KeyStagePlan::Deferred;
    }
    KeyStagePlan::Resume
}

/// 判断点 (c) の KeyDown 側（ADR-245 決定6 の R4-1）: Shift の KeyDown で単独タップの候補（ガード）を
/// 落とすか。トグル中、または現在の前面スコープに戻り待ちがあるときは落とさない（KeyUp の
/// ExitOnTap に届かせる。エンジン OFF など `entry_context_ok` が偽の構成でも）。
/// `entry_context_ok` は `effective_open && is_japanese_ime && is_user_enabled && conv_mutation_allowed`。
#[must_use]
pub const fn shift_key_down_disarms_guard(
    toggle_active: bool,
    pending_for_scope: bool,
    entry_context_ok: bool,
) -> bool {
    if toggle_active || pending_for_scope {
        return false;
    }
    !entry_context_ok
}

#[cfg(test)]
mod tests {
    use super::{plan_half_width_alnum_action as plan, HalfWidthAlnumAction, ShiftKeyUpKind};

    #[test]
    fn entry_only_on_inactive_left_shift_tap() {
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, false, true),
            HalfWidthAlnumAction::Enter
        );
        assert_eq!(
            plan(ShiftKeyUpKind::RightTap, false, true),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, false, true),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::RightChord, false, true),
            HalfWidthAlnumAction::None
        );
    }

    #[test]
    fn active_toggle_taps_exit_but_chords_persist_symmetrically() {
        // 2回目の左Shiftタップ・右Shift単独タップ（緊急解除）は exit。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, true, true),
            HalfWidthAlnumAction::Exit
        );
        assert_eq!(
            plan(ShiftKeyUpKind::RightTap, true, true),
            HalfWidthAlnumAction::Exit
        );
        // 左右どちらのチョード（Shift+文字キーで大文字を打つ用途）も
        // exit しない — トグル中に Shift を離しただけで持続モードから
        // 抜けてしまう不具合の修正（実機報告、左右対称）。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, true, true),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::RightChord, true, true),
            HalfWidthAlnumAction::None
        );
    }

    #[test]
    fn unsupported_entry_blocks_enter_but_never_blocks_tap_exit() {
        // entry_supported=false は新規 entry を止めるだけで、既に active な
        // トグルからの脱出（緊急解除）はブロックしない — entry 後に IME種別
        // 変化・kill switch・belief 変化等で entry_supported が false に
        // 転じても、ユーザーは必ずかなへ戻れる。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, false, false),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, true, false),
            HalfWidthAlnumAction::Exit
        );
        assert_eq!(
            plan(ShiftKeyUpKind::RightTap, true, false),
            HalfWidthAlnumAction::Exit
        );
        // チョードは entry_supported の値に関わらず常に None（exitしない
        // という結論自体は entry 可否の設定と無関係）。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, true, false),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::RightChord, true, false),
            HalfWidthAlnumAction::None
        );
    }

    // ── `HalfWidthAlnumState` の遷移テーブルテスト（§9） ──────────────

    use super::{
        ExitOwnership, HalfWidthAlnumEffect as Effect, HalfWidthAlnumState,
        HalfWidthAlnumTogglePolicy, ShiftSide,
    };

    /// 左右対称性: `LeftTap`/`LeftChord`/`RightTap`/`RightChord` の4パターンを
    /// トグルON/OFF双方で確認する。トグルOFF側は既存
    /// `entry_only_on_inactive_left_shift_tap` と同じ非対称（`RightTap` は
    /// 緊急解除専用でEnterしない）を state 経由でも保つことを固定する。
    #[test]
    fn on_shift_up_left_right_symmetry_across_toggle_states() {
        // トグル非アクティブ側。
        for (side, arm, expect) in [
            (ShiftSide::Left, true, Effect::EnterViaImcWrite), // LeftTap
            (ShiftSide::Left, false, Effect::Nothing),         // LeftChord
            (ShiftSide::Right, true, Effect::Nothing), // RightTap（緊急解除は非アクティブ時は無効）
            (ShiftSide::Right, false, Effect::Nothing), // RightChord
        ] {
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(HalfWidthAlnumTogglePolicy::All);
            if arm {
                state.arm_tap(side);
            }
            let effect = state.on_shift_up(side, true, true, false);
            assert_eq!(effect, expect, "toggle inactive: side={side:?} arm={arm}");
        }

        // トグルアクティブ側: Tapはexit、Chordは何もしない（左右対称）。
        for (side, arm, expect) in [
            (
                ShiftSide::Left,
                true,
                Effect::ExitRestoreKana {
                    ownership: ExitOwnership::FromToggle,
                },
            ), // LeftTap
            (ShiftSide::Left, false, Effect::Nothing), // LeftChord
            (
                ShiftSide::Right,
                true,
                Effect::ExitRestoreKana {
                    ownership: ExitOwnership::FromToggle,
                },
            ), // RightTap（緊急解除）
            (ShiftSide::Right, false, Effect::Nothing), // RightChord
        ] {
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(HalfWidthAlnumTogglePolicy::All);
            state.commit_enter_imc();
            if arm {
                state.arm_tap(side);
            }
            let effect = state.on_shift_up(side, true, true, false);
            assert_eq!(effect, expect, "toggle active: side={side:?} arm={arm}");
        }
    }

    /// 左右非対称の再発防止テスト（BUG-25追補11型）: `note_physical_key_down`
    /// は反対側の候補**だけ**を折り、自分自身の候補は生き残る。
    /// `take_shift_up_kind_disarming_both` は呼んだ側に関わらず**必ず両方**を
    /// disarmする。
    ///
    /// m2（Opus敵対的レビュー指摘）: 旧実装は `arm_tap(Right)` を
    /// `note_physical_key_down(VK_RSHIFT)` の**後**に呼んでいたため、
    /// `note_physical_key_down` を「無条件に両方false」へ変異させても
    /// このテストは通ってしまっていた（ミューテーション耐性ゼロ）。
    /// `arm_tap` を検証対象の呼び出しより**前**に置き、直後に
    /// `take_shift_up_kind_disarming_both` で確認する順序に組み替える。
    #[test]
    fn note_physical_key_down_folds_only_the_opposite_side_bug25_addendum11() {
        // ケース1: VK_RSHIFT の物理KeyDownは左候補を折る（`vk != VK_LSHIFT`）。
        {
            let mut state = HalfWidthAlnumState::default();
            state.arm_tap(ShiftSide::Left);
            state.note_physical_key_down(crate::vk::VK_RSHIFT);
            assert_eq!(
                state.take_shift_up_kind_disarming_both(ShiftSide::Left),
                ShiftKeyUpKind::LeftChord,
                "note_physical_key_down(VK_RSHIFT) は左候補を折るはずなので、\
                 左Shiftのkeyupはチョード扱いになるべき"
            );
        }
        // ケース2: VK_RSHIFT 自身の物理KeyDownは右候補を折らない
        // （`vk != VK_RSHIFT` が false になるため）。arm_tap を
        // note_physical_key_down より前に置くことで、「無条件に両方false」
        // という変異にもこのテストが反応する（変異があればここが
        // RightChordになり失敗する）。
        {
            let mut state = HalfWidthAlnumState::default();
            state.arm_tap(ShiftSide::Right);
            state.note_physical_key_down(crate::vk::VK_RSHIFT);
            assert_eq!(
                state.take_shift_up_kind_disarming_both(ShiftSide::Right),
                ShiftKeyUpKind::RightTap,
                "note_physical_key_down(VK_RSHIFT) は右候補（自分自身の\
                 KeyDown）を折ってはならない"
            );
        }
        // ケース3（新規）: 対称に、VK_LSHIFT 自身の物理KeyDownは左候補を
        // 折らない。
        {
            let mut state = HalfWidthAlnumState::default();
            state.arm_tap(ShiftSide::Left);
            state.note_physical_key_down(crate::vk::VK_LSHIFT);
            assert_eq!(
                state.take_shift_up_kind_disarming_both(ShiftSide::Left),
                ShiftKeyUpKind::LeftTap,
                "note_physical_key_down(VK_LSHIFT) は左候補（自分自身の\
                 KeyDown）を折ってはならない"
            );
        }
        // ケース4: take_shift_up_kind_disarming_both は呼んだ側に関係なく
        // 必ず両方をdisarmする（左を取った直後に右を取るとチョード扱いに
        // なる——`side` だけ disarm する実装に変えると `RightTap` になって
        // しまい退行を検知できなくなる）。
        {
            let mut state = HalfWidthAlnumState::default();
            state.arm_tap(ShiftSide::Left);
            state.arm_tap(ShiftSide::Right);
            let _ = state.take_shift_up_kind_disarming_both(ShiftSide::Left);
            assert_eq!(
                state.take_shift_up_kind_disarming_both(ShiftSide::Right),
                ShiftKeyUpKind::RightChord,
                "take_shift_up_kind_disarming_both は呼んだ側に関係なく\
                 両方をdisarmするべき（直前のLeft呼び出しで既にdisarm済み\
                 のはず）"
            );
        }
    }

    /// GJI失敗時の巻き戻し: `begin_restore_kana` → `rearm_after_failed_gji_exit`
    /// → 次の `begin_restore_kana` が再び `true`（直前active）を返すこと。
    #[test]
    fn failed_gji_exit_rearms_toggle_for_retry() {
        let mut state = HalfWidthAlnumState::default();
        state.commit_enter_gji();
        assert!(state.is_toggle_active());

        assert!(
            state.begin_restore_kana(),
            "commit_enter_gji 直後の begin_restore_kana は直前activeとしてtrueを返すべき"
        );
        assert!(!state.is_toggle_active());

        state.rearm_after_failed_gji_exit();
        assert!(
            state.is_toggle_active(),
            "GJI SendInput見送り後は再試行に備えてtoggle_heldをtrueへ戻すべき"
        );
        assert!(
            state.begin_restore_kana(),
            "巻き戻し後の再試行でも begin_restore_kana は直前activeとしてtrueを返すべき"
        );
    }

    /// Enter Effectの真理値表: `entry_policy`(`Off`/`MsImeOnly`/`All`) ×
    /// `uses_imc_conv_write`(true/false) × `toggle_held`(true/false) ×
    /// `entry_ime_ok`(true/false) × `ShiftKeyUpKind`4種の全組み合わせで
    /// `on_shift_up` の戻り値が `plan_half_width_alnum_action` + policy 判定
    /// から導出される期待値と一致することを確認する。
    ///
    /// m3（Opus敵対的レビュー指摘）: 旧実装は `entry_ime_ok` を常に `true`
    /// 固定していたため、`on_shift_up` 内の
    /// `policy_allows_entry && entry_ime_ok` の `&&` を `||` へ変異させても
    /// 全パターンが通ってしまっていた（ミューテーション耐性ゼロ）。
    /// `entry_ime_ok` を5軸目としてtrue/false両方回し、期待値側の
    /// `toggle_entry_supported` 計算にも同じ `&&` を反映する。
    #[test]
    fn enter_effect_truth_table_across_policy_ime_and_toggle_state() {
        for policy in [
            HalfWidthAlnumTogglePolicy::Off,
            HalfWidthAlnumTogglePolicy::MsImeOnly,
            HalfWidthAlnumTogglePolicy::All,
        ] {
            for uses_imc in [true, false] {
                for toggle_active in [true, false] {
                    for entry_ime_ok in [true, false] {
                        for kind in [
                            ShiftKeyUpKind::LeftTap,
                            ShiftKeyUpKind::LeftChord,
                            ShiftKeyUpKind::RightTap,
                            ShiftKeyUpKind::RightChord,
                        ] {
                            let side = match kind {
                                ShiftKeyUpKind::LeftTap | ShiftKeyUpKind::LeftChord => {
                                    ShiftSide::Left
                                }
                                ShiftKeyUpKind::RightTap | ShiftKeyUpKind::RightChord => {
                                    ShiftSide::Right
                                }
                            };
                            let mut state = HalfWidthAlnumState::default();
                            state.set_policy(policy);
                            if toggle_active {
                                state.commit_enter_imc();
                            }
                            if matches!(kind, ShiftKeyUpKind::LeftTap | ShiftKeyUpKind::RightTap) {
                                state.arm_tap(side);
                            }

                            let effect = state.on_shift_up(side, entry_ime_ok, uses_imc, false);

                            let policy_allows_entry = match policy {
                                HalfWidthAlnumTogglePolicy::Off => false,
                                HalfWidthAlnumTogglePolicy::MsImeOnly => uses_imc,
                                HalfWidthAlnumTogglePolicy::All => true,
                            };
                            let toggle_entry_supported = policy_allows_entry && entry_ime_ok;
                            let action = plan(kind, toggle_active, toggle_entry_supported);
                            let expected = match action {
                                HalfWidthAlnumAction::None => Effect::Nothing,
                                HalfWidthAlnumAction::Enter => {
                                    if uses_imc {
                                        Effect::EnterViaImcWrite
                                    } else {
                                        Effect::EnterViaGjiSendInput
                                    }
                                }
                                HalfWidthAlnumAction::Exit => Effect::ExitRestoreKana {
                                    ownership: ExitOwnership::FromToggle,
                                },
                            };
                            assert_eq!(
                                effect, expected,
                                "policy={policy:?} uses_imc={uses_imc} \
                                 toggle_active={toggle_active} \
                                 entry_ime_ok={entry_ime_ok} kind={kind:?}"
                            );
                        }
                    }
                }
            }
        }

        // MsImeOnly + GJI環境: Enter系Effectは絶対に出ない
        // （policy=MsImeOnlyの存在意義そのもの）。
        for kind in [ShiftKeyUpKind::LeftTap, ShiftKeyUpKind::RightTap] {
            let side = if matches!(kind, ShiftKeyUpKind::LeftTap) {
                ShiftSide::Left
            } else {
                ShiftSide::Right
            };
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(HalfWidthAlnumTogglePolicy::MsImeOnly);
            state.arm_tap(side);
            let effect = state.on_shift_up(side, true, false, false); // uses_imc=false = GJI
            assert_ne!(
                effect,
                Effect::EnterViaGjiSendInput,
                "MsImeOnly policy下でGJI環境のEnterが発火してはならない (kind={kind:?})"
            );
            assert_ne!(effect, Effect::EnterViaImcWrite);
        }

        // toggle_held=true からの Exit は policy に関わらず必ず出る
        // （緊急解除はkill switchの対象外）。
        for policy in [
            HalfWidthAlnumTogglePolicy::Off,
            HalfWidthAlnumTogglePolicy::MsImeOnly,
            HalfWidthAlnumTogglePolicy::All,
        ] {
            for uses_imc in [true, false] {
                let mut state = HalfWidthAlnumState::default();
                state.set_policy(policy);
                state.commit_enter_imc();
                state.arm_tap(ShiftSide::Left);
                let effect = state.on_shift_up(ShiftSide::Left, true, uses_imc, false);
                assert_eq!(
                    effect,
                    Effect::ExitRestoreKana {
                        ownership: ExitOwnership::FromToggle,
                    },
                    "policy={policy:?} uses_imc={uses_imc}: トグルON中の緊急解除は \
                     policyに関係なく発火するべき"
                );
            }
        }
    }

    /// ADR-244 D3: 手放すのは MS-IME 本体 × NATIVE の読み(eisu=Some(false)) × 追随後に belief が英数でないときだけ。
    #[test]
    fn abandon_only_for_ms_ime_native_read_that_took_effect() {
        use super::should_abandon_on_observed_follow as f;
        use crate::state::ime_kind::ImeKindId::{Gji, MsIme};
        assert!(f(MsIme, Some(false), true));
        // 予測の fence に追随を捨てられ、belief が英数のまま残ったときは手放さない（belief は英数・トグルは無し、を作らない）。
        assert!(!f(MsIme, Some(false), false));
        // 英数の読み・追随なしでは手放さない(全角英数 conv=0x18 は eisu=Some(true) のまま)。
        assert!(!f(MsIme, Some(true), true));
        assert!(!f(MsIme, None, true));
        // GJI は従来どおり手放さない。
        assert!(!f(Gji, Some(false), true));
        assert!(!f(Gji, Some(true), true));
        assert!(!f(Gji, None, true));
    }

    /// ADR-244 D3: 手放しは OS 書き込みを伴わず、直前の toggle_held を返す。手放した後の復元(begin_restore_kana)は不要。
    #[test]
    fn abandon_on_observed_follow_clears_toggle_without_a_restore_request() {
        use super::HalfWidthAlnumState;
        let mut st = HalfWidthAlnumState::default();
        st.commit_enter_imc();
        assert!(st.is_toggle_active());
        assert!(st.abandon_on_observed_follow());
        assert!(!st.is_toggle_active());
        // 二重呼び出しは何も起こさず、復元の取りこぼしも誤って立てることもない。
        assert!(!st.abandon_on_observed_follow());
        assert!(!st.begin_restore_kana());
    }

    // ── ADR-245: 戻り待ち・離脱/打鍵の段/Shift KeyUp の判断 ──────────────

    use super::{
        exit_owns_write, plan_key_stage, plan_leave, return_pending_expired,
        shift_key_down_disarms_guard, DropReason, KeyStageFacts, KeyStagePlan, LeavePlan,
        ReturnPending, ReturnPendingEntry, RETURN_PENDING_CAPACITY,
    };
    use crate::state::foreground_scope::ForegroundScope;
    use crate::state::TickMs;

    const MAX_AGE: u64 = 3_600_000;

    fn scope(pid: u32, hwnd: isize) -> ForegroundScope {
        ForegroundScope { pid, hwnd }
    }

    fn entry(pid: u32, hwnd: isize, uses_imc: bool, at: u64) -> ReturnPendingEntry {
        ReturnPendingEntry {
            scope: scope(pid, hwnd),
            uses_imc_conv_write: uses_imc,
            at: TickMs(at),
        }
    }

    /// 既定は「MS-IME 本体、開、通常の文字キー、修飾キーなし、寿命内」= `Resume` になる事実。
    fn facts() -> KeyStageFacts {
        KeyStageFacts {
            entry: entry(10, 0x100, true, 1_000),
            now: TickMs(2_000),
            max_age_ms: MAX_AGE,
            uses_imc_conv_write_now: true,
            is_modifier_key: false,
            is_ime_mode_role_key: false,
            is_thumb_key: false,
            effective_open: true,
            app_disabled: false,
            hiragana_key_is_set: None,
            physical_modifier_down: false,
        }
    }

    /// GJI（`uses_imc_conv_write == false`）版の既定事実。F2 は SET と確かめられている。
    fn gji_facts() -> KeyStageFacts {
        KeyStageFacts {
            entry: entry(10, 0x100, false, 1_000),
            uses_imc_conv_write_now: false,
            hiragana_key_is_set: Some(true),
            ..facts()
        }
    }

    /// ADR-245 決定4・検証 B-1: `FromResume` は `begin_restore_kana` の旧値（偽）に関わらず OS 書き込みを持つ。
    #[test]
    fn exit_owns_write_from_resume_ignores_the_old_toggle_value() {
        assert!(exit_owns_write(ExitOwnership::FromResume, false));
        assert!(exit_owns_write(ExitOwnership::FromResume, true));
        assert!(exit_owns_write(ExitOwnership::FromToggle, true));
        assert!(!exit_owns_write(ExitOwnership::FromToggle, false));
    }

    /// 決定6: 戻り待ちがあるとき、`toggle_held` が偽でも単独タップは Exit（`FromResume`）になる。
    #[test]
    fn on_shift_up_with_pending_for_scope_exits_from_resume() {
        for side in [ShiftSide::Left, ShiftSide::Right] {
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(HalfWidthAlnumTogglePolicy::All);
            state.arm_tap(side);
            assert_eq!(
                state.on_shift_up(side, true, true, true),
                Effect::ExitRestoreKana {
                    ownership: ExitOwnership::FromResume
                },
                "side={side:?}: 戻り待ちあり + toggle_held 偽 = FromResume"
            );
        }
    }

    /// 決定6: エンジン OFF など `entry_ime_ok` が偽・policy が Off でも、戻り待ちからの Exit は止まらない
    /// （緊急解除と同じく entry 条件の対象外）。
    #[test]
    fn on_shift_up_pending_exit_ignores_entry_conditions() {
        for policy in [
            HalfWidthAlnumTogglePolicy::Off,
            HalfWidthAlnumTogglePolicy::MsImeOnly,
            HalfWidthAlnumTogglePolicy::All,
        ] {
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(policy);
            state.arm_tap(ShiftSide::Left);
            assert_eq!(
                state.on_shift_up(ShiftSide::Left, false, false, true),
                Effect::ExitRestoreKana {
                    ownership: ExitOwnership::FromResume
                },
                "policy={policy:?}"
            );
        }
    }

    /// 決定6: 戻り待ちがあってもチョード（Shift+文字）は Exit しない。
    #[test]
    fn on_shift_up_with_pending_chord_does_nothing() {
        for side in [ShiftSide::Left, ShiftSide::Right] {
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(HalfWidthAlnumTogglePolicy::All);
            // arm しない = チョード。
            assert_eq!(state.on_shift_up(side, true, true, true), Effect::Nothing);
        }
    }

    /// `toggle_held` が真なら `pending_for_scope` に関わらず従来どおり `FromToggle`、どちらも偽なら従来どおり Enter。
    #[test]
    fn on_shift_up_without_pending_keeps_existing_ownership_and_enter() {
        let mut state = HalfWidthAlnumState::default();
        state.set_policy(HalfWidthAlnumTogglePolicy::All);
        state.commit_enter_imc();
        state.arm_tap(ShiftSide::Left);
        assert_eq!(
            state.on_shift_up(ShiftSide::Left, true, true, false),
            Effect::ExitRestoreKana {
                ownership: ExitOwnership::FromToggle
            }
        );
        let mut state = HalfWidthAlnumState::default();
        state.set_policy(HalfWidthAlnumTogglePolicy::All);
        state.arm_tap(ShiftSide::Left);
        assert_eq!(
            state.on_shift_up(ShiftSide::Left, true, true, false),
            Effect::EnterViaImcWrite
        );
    }

    /// 決定6 の R4-1: トグル中・戻り待ちあり（エンジン OFF 等で文脈が偽でも）はガードを落とさない。
    #[test]
    fn shift_key_down_keeps_guard_for_toggle_or_pending_even_with_engine_off() {
        // 通常: 文脈が偽なら落とす、真なら落とさない。
        assert!(shift_key_down_disarms_guard(false, false, false));
        assert!(!shift_key_down_disarms_guard(false, false, true));
        // トグル中・戻り待ちあり: 文脈が偽でも落とさない。
        assert!(!shift_key_down_disarms_guard(true, false, false));
        assert!(!shift_key_down_disarms_guard(false, true, false));
        assert!(!shift_key_down_disarms_guard(false, true, true));
    }

    #[test]
    fn return_pending_expired_boundary() {
        assert!(!return_pending_expired(TickMs(100), TickMs(100), 50));
        assert!(!return_pending_expired(TickMs(100), TickMs(150), 50));
        assert!(return_pending_expired(TickMs(100), TickMs(151), 50));
        // now が at より前（時計の巻き戻り）でも飽和して期限切れにしない。
        assert!(!return_pending_expired(TickMs(100), TickMs(50), 0));
    }

    /// 離脱: トグル中で有効なスコープがあるときだけ Suspend。
    #[test]
    fn plan_leave_suspends_only_for_active_toggle_with_valid_scope() {
        assert_eq!(plan_leave(true, scope(1, 2)), LeavePlan::Suspend);
        assert_eq!(plan_leave(false, scope(1, 2)), LeavePlan::Nothing);
        assert_eq!(
            plan_leave(true, ForegroundScope::INVALID),
            LeavePlan::Nothing
        );
        assert_eq!(plan_leave(true, scope(0, 2)), LeavePlan::Nothing);
        assert_eq!(plan_leave(true, scope(1, 0)), LeavePlan::Nothing);
    }

    /// 離脱=Suspend: toggle_held が下りて戻り待ちへ積まれ、IME へは何も送らない（状態遷移のみ）。
    #[test]
    fn suspend_lowers_toggle_and_queues_the_entry() {
        let mut state = HalfWidthAlnumState::default();
        state.commit_enter_imc();
        assert!(!state.has_return_pending());
        assert_eq!(
            state.suspend_toggle_for_return(scope(10, 0x100), true, TickMs(5)),
            Some(0)
        );
        assert!(!state.is_toggle_active());
        assert!(state.has_return_pending());
        assert_eq!(
            state.find_return_pending(scope(10, 0x100)),
            Some(entry(10, 0x100, true, 5))
        );
        // トグルが立っていなければ何も積まない。
        let mut idle = HalfWidthAlnumState::default();
        assert_eq!(
            idle.suspend_toggle_for_return(scope(10, 0x100), true, TickMs(5)),
            None
        );
        assert!(!idle.has_return_pending());
    }

    /// 戻り+文字キー: 一致したエントリを取り出すと消える（INV-B の「1 回」）。別窓のエントリは保持する。
    #[test]
    fn take_returns_the_matching_entry_once_and_keeps_other_windows() {
        let mut state = HalfWidthAlnumState::default();
        for (pid, hwnd) in [(10, 0x100), (20, 0x200)] {
            state.commit_enter_gji();
            state.suspend_toggle_for_return(scope(pid, hwnd), false, TickMs(1));
        }
        // 別窓（C）では取り出せず、どちらも保持。
        assert_eq!(state.take_return_pending(scope(30, 0x300)), None);
        assert!(state.has_return_pending());
        // A に戻る。
        assert_eq!(
            state.take_return_pending(scope(10, 0x100)),
            Some(entry(10, 0x100, false, 1))
        );
        assert_eq!(
            state.take_return_pending(scope(10, 0x100)),
            None,
            "二重に取れない"
        );
        // B の分は残る。
        assert_eq!(
            state.find_return_pending(scope(20, 0x200)),
            Some(entry(20, 0x200, false, 1))
        );
    }

    /// pid だけ一致（別 hwnd）は Resume の対象にしない。
    #[test]
    fn same_pid_different_hwnd_is_not_a_match() {
        let mut rp = ReturnPending::default();
        rp.push(entry(10, 0x100, true, 1));
        assert_eq!(rp.find_for_scope(scope(10, 0x101)), None);
        assert_eq!(rp.take_for_scope(scope(10, 0x101)), None);
        assert_eq!(rp.len(), 1);
        // hwnd だけ一致（別 pid）も対象外。
        assert_eq!(rp.find_for_scope(scope(11, 0x100)), None);
    }

    /// 容量超過: 古いものから捨て、捨てた件数を返す。
    #[test]
    fn push_over_capacity_drops_oldest_first_and_reports_count() {
        let mut rp = ReturnPending::default();
        for hwnd in [0x100, 0x101, 0x102, 0x103] {
            assert_eq!(rp.push(entry(1, hwnd, true, 1)), 0);
        }
        assert_eq!(rp.len(), RETURN_PENDING_CAPACITY);
        assert_eq!(rp.push(entry(2, 0x900, true, 99)), 1);
        assert_eq!(rp.len(), RETURN_PENDING_CAPACITY);
        assert_eq!(
            rp.find_for_scope(scope(1, 0x100)),
            None,
            "最も古いものが落ちる"
        );
        assert!(rp.find_for_scope(scope(1, 0x101)).is_some());
        assert!(rp.find_for_scope(scope(2, 0x900)).is_some());
        assert_eq!(RETURN_PENDING_CAPACITY, 4, "ADR-245 決定2: 容量 4");
    }

    /// 同じスコープの再 push は置き換え（二重に待たない。捨てた件数には数えない）。
    #[test]
    fn push_same_scope_replaces_without_counting_as_dropped() {
        let mut rp = ReturnPending::default();
        rp.push(entry(1, 0x100, true, 1));
        assert_eq!(rp.push(entry(1, 0x100, false, 7)), 0);
        assert_eq!(rp.len(), 1);
        assert_eq!(
            rp.find_for_scope(scope(1, 0x100)),
            Some(entry(1, 0x100, false, 7))
        );
    }

    /// 寿命切れの除去: 期限内は残し、超えたものだけ捨てて件数を返す。
    #[test]
    fn prune_expired_removes_only_entries_older_than_max_age() {
        let mut rp = ReturnPending::default();
        rp.push(entry(1, 0x1, true, 0));
        rp.push(entry(2, 0x2, true, 600));
        assert_eq!(rp.prune_expired(TickMs(1_000), 500), 1);
        assert!(rp.find_for_scope(scope(1, 0x1)).is_none());
        assert!(rp.find_for_scope(scope(2, 0x2)).is_some());
        assert_eq!(rp.prune_expired(TickMs(1_000), 500), 0);
        // 状態経由でも同じ。
        let mut state = HalfWidthAlnumState::default();
        state.commit_enter_imc();
        state.suspend_toggle_for_return(scope(1, 1), true, TickMs(0));
        assert_eq!(state.prune_return_pending(TickMs(MAX_AGE + 1), MAX_AGE), 1);
        assert!(!state.has_return_pending());
    }

    #[test]
    fn key_stage_default_facts_resume() {
        assert_eq!(plan_key_stage(&facts()), KeyStagePlan::Resume);
        assert_eq!(plan_key_stage(&gji_facts()), KeyStagePlan::Resume);
    }

    /// 行 1: 修飾キー自身の KeyDown/KeyUp は何もしない（他の条件より先。Drop 条件が揃っていても）。
    #[test]
    fn key_stage_modifier_key_itself_is_nothing() {
        let f = KeyStageFacts {
            is_modifier_key: true,
            effective_open: false,
            is_ime_mode_role_key: true,
            physical_modifier_down: true,
            ..facts()
        };
        assert_eq!(plan_key_stage(&f), KeyStagePlan::Nothing);
        assert!(!KeyStagePlan::Nothing.consumes_entry());
    }

    /// 行 3: Drop の理由は個別に区別できる。
    #[test]
    fn key_stage_drop_reasons_are_distinguished() {
        let cases = [
            (
                KeyStageFacts {
                    effective_open: false,
                    ..facts()
                },
                DropReason::EffectiveOpenFalse,
            ),
            (
                KeyStageFacts {
                    uses_imc_conv_write_now: false,
                    ..facts()
                },
                DropReason::ImeKindMismatch,
            ),
            (
                KeyStageFacts {
                    app_disabled: true,
                    ..facts()
                },
                DropReason::AppDisabled,
            ),
            (
                KeyStageFacts {
                    now: TickMs(1_000 + MAX_AGE + 1),
                    ..facts()
                },
                DropReason::Expired,
            ),
        ];
        let mut labels = Vec::new();
        for (f, reason) in cases {
            assert_eq!(plan_key_stage(&f), KeyStagePlan::Drop(reason));
            assert!(KeyStagePlan::Drop(reason).consumes_entry());
            labels.push(reason.label());
        }
        labels.push(DropReason::ImeModeRole.label());
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), 5, "理由はログで区別できること");
    }

    /// 寿命はちょうど上限なら有効（Resume）。
    #[test]
    fn key_stage_entry_at_exact_max_age_is_still_valid() {
        let f = KeyStageFacts {
            now: TickMs(1_000 + MAX_AGE),
            ..facts()
        };
        assert_eq!(plan_key_stage(&f), KeyStagePlan::Resume);
    }

    /// 行 2: IME モードの役割のキー（修飾キーの有無を問わない）は Drop。親指キーは除く。
    /// Ctrl+変換（`keys.ime_on` の組み合わせ）= IME モードの役割 + 物理 Ctrl 押下中でも Drop。
    #[test]
    fn key_stage_ime_mode_role_key_drops_but_thumb_key_resumes() {
        let role = KeyStageFacts {
            is_ime_mode_role_key: true,
            ..facts()
        };
        assert_eq!(
            plan_key_stage(&role),
            KeyStagePlan::Drop(DropReason::ImeModeRole)
        );
        let ctrl_henkan = KeyStageFacts {
            is_ime_mode_role_key: true,
            physical_modifier_down: true,
            ..facts()
        };
        assert_eq!(
            plan_key_stage(&ctrl_henkan),
            KeyStagePlan::Drop(DropReason::ImeModeRole),
            "Ctrl+変換は Deferred でなく Drop（行 2 は行 5 より先）"
        );
        let thumb = KeyStageFacts {
            is_ime_mode_role_key: true,
            is_thumb_key: true,
            ..facts()
        };
        assert_eq!(plan_key_stage(&thumb), KeyStagePlan::Resume);
        // 親指キー+文字（Shift 押下なし）も Resume。
        let thumb_plain = KeyStageFacts {
            is_thumb_key: true,
            ..facts()
        };
        assert_eq!(plan_key_stage(&thumb_plain), KeyStagePlan::Resume);
    }

    /// 行 2 は行 3 より先（ImeModeRole の理由が優先して残る）。
    #[test]
    fn key_stage_role_drop_precedes_state_drops() {
        let f = KeyStageFacts {
            is_ime_mode_role_key: true,
            effective_open: false,
            app_disabled: true,
            ..facts()
        };
        assert_eq!(
            plan_key_stage(&f),
            KeyStagePlan::Drop(DropReason::ImeModeRole)
        );
    }

    /// 行 5: Shift/Ctrl 押下中の文字キーは Deferred（エントリは残す）。
    #[test]
    fn key_stage_physical_modifier_defers_and_keeps_entry() {
        let f = KeyStageFacts {
            physical_modifier_down: true,
            ..facts()
        };
        assert_eq!(plan_key_stage(&f), KeyStagePlan::Deferred);
        assert!(!KeyStagePlan::Deferred.consumes_entry());
        let g = KeyStageFacts {
            physical_modifier_down: true,
            ..gji_facts()
        };
        assert_eq!(plan_key_stage(&g), KeyStagePlan::Deferred);
    }

    /// 行 4: GJI で F2 が SET と確かめられない（トグル/予測なし）は RebuildToggle。修飾キーの有無を問わない。
    /// Ctrl 押下中で予測がトグルでも RebuildToggle（行 5 の Deferred より先）。
    #[test]
    fn key_stage_gji_non_set_rebuilds_toggle_regardless_of_modifiers() {
        for set in [Some(false), None] {
            for modifier_down in [false, true] {
                let f = KeyStageFacts {
                    hiragana_key_is_set: set,
                    physical_modifier_down: modifier_down,
                    ..gji_facts()
                };
                assert_eq!(
                    plan_key_stage(&f),
                    KeyStagePlan::RebuildToggle,
                    "set={set:?} modifier_down={modifier_down}"
                );
            }
        }
        assert!(KeyStagePlan::RebuildToggle.consumes_entry());
        // 予測が SET なら Resume。
        assert_eq!(plan_key_stage(&gji_facts()), KeyStagePlan::Resume);
    }

    /// MS-IME 本体では F2 の予測を見ない（IMC の SET と VK_DBE_HIRAGANA の SET）。
    #[test]
    fn key_stage_ms_ime_ignores_hiragana_prediction() {
        for set in [None, Some(false), Some(true)] {
            let f = KeyStageFacts {
                hiragana_key_is_set: set,
                ..facts()
            };
            assert_eq!(plan_key_stage(&f), KeyStagePlan::Resume, "set={set:?}");
        }
    }

    /// 行 3 は行 4 より先: effective_open 偽の GJI は RebuildToggle でなく Drop。
    #[test]
    fn key_stage_drop_precedes_rebuild_toggle() {
        let f = KeyStageFacts {
            effective_open: false,
            hiragana_key_is_set: None,
            ..gji_facts()
        };
        assert_eq!(
            plan_key_stage(&f),
            KeyStagePlan::Drop(DropReason::EffectiveOpenFalse)
        );
    }

    #[test]
    fn only_resume_drop_and_rebuild_consume_the_entry() {
        assert!(KeyStagePlan::Resume.consumes_entry());
        assert!(!KeyStagePlan::Nothing.consumes_entry());
        assert!(!KeyStagePlan::Deferred.consumes_entry());
    }
}
