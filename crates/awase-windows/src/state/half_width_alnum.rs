use awase::config::HalfWidthAlnumTogglePolicy;
use awase::types::VkCode;

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
    /// `toggle_active == true` のときにしか発生しないため、直前の
    /// アクティブ状態を運ぶ `was_active` フィールドは持たない
    /// （常に true であることが自明なため）。
    ExitRestoreKana,
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
    pub fn on_shift_up(
        &mut self,
        side: ShiftSide,
        entry_ime_ok: bool,
        uses_imc_conv_write: bool,
    ) -> HalfWidthAlnumEffect {
        let shift_up_kind = self.take_shift_up_kind_disarming_both(side);
        let policy_allows_entry = match self.entry_policy {
            HalfWidthAlnumTogglePolicy::Off => false,
            HalfWidthAlnumTogglePolicy::MsImeOnly => uses_imc_conv_write,
            HalfWidthAlnumTogglePolicy::All => true,
        };
        let toggle_entry_supported = policy_allows_entry && entry_ime_ok;
        match plan_half_width_alnum_action(shift_up_kind, self.toggle_held, toggle_entry_supported)
        {
            HalfWidthAlnumAction::None => HalfWidthAlnumEffect::Nothing,
            HalfWidthAlnumAction::Enter => {
                if uses_imc_conv_write {
                    HalfWidthAlnumEffect::EnterViaImcWrite
                } else {
                    HalfWidthAlnumEffect::EnterViaGjiSendInput
                }
            }
            HalfWidthAlnumAction::Exit => HalfWidthAlnumEffect::ExitRestoreKana,
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

    // ── 状態照会 ──────────────────────────────────────────────────────

    #[must_use]
    pub const fn is_toggle_active(&self) -> bool {
        self.toggle_held
    }
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
        HalfWidthAlnumEffect as Effect, HalfWidthAlnumState, HalfWidthAlnumTogglePolicy, ShiftSide,
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
            let effect = state.on_shift_up(side, true, true);
            assert_eq!(effect, expect, "toggle inactive: side={side:?} arm={arm}");
        }

        // トグルアクティブ側: Tapはexit、Chordは何もしない（左右対称）。
        for (side, arm, expect) in [
            (ShiftSide::Left, true, Effect::ExitRestoreKana), // LeftTap
            (ShiftSide::Left, false, Effect::Nothing),        // LeftChord
            (ShiftSide::Right, true, Effect::ExitRestoreKana), // RightTap（緊急解除）
            (ShiftSide::Right, false, Effect::Nothing),       // RightChord
        ] {
            let mut state = HalfWidthAlnumState::default();
            state.set_policy(HalfWidthAlnumTogglePolicy::All);
            state.commit_enter_imc();
            if arm {
                state.arm_tap(side);
            }
            let effect = state.on_shift_up(side, true, true);
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

                            let effect = state.on_shift_up(side, entry_ime_ok, uses_imc);

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
                                HalfWidthAlnumAction::Exit => Effect::ExitRestoreKana,
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
            let effect = state.on_shift_up(side, true, false); // uses_imc=false = GJI
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
                let effect = state.on_shift_up(ShiftSide::Left, true, uses_imc);
                assert_eq!(
                    effect,
                    Effect::ExitRestoreKana,
                    "policy={policy:?} uses_imc={uses_imc}: トグルON中の緊急解除は \
                     policyに関係なく発火するべき"
                );
            }
        }
    }
}
