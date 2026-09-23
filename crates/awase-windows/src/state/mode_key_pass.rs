//! 無変換/変換などの生キー通過マーク（ADR-187、BUG-157、BUG-158）の状態機械。
//!
//! 以前は `platform_state.rs`（`#[cfg(windows)]`）の `ImeStateHub` に直に載っており、判断ロジックが
//! Windows専用テストでしか検証できなかった。通過マークの寿命・意図の破棄・`desired_open` の揃え・
//! 読み直し間隔の判断は Win32 に依存しない（スコープ `S` を外から渡すだけ）ので、ここへ切り出す
//! （design-patterns-review.md 提案3、A1/A2）。`ImeStateHub` はこの型が返す
//! [`PassEffect`] を2行で適用する側に回り、`_in_scope` 版の手書きの依存性注入（`ForegroundScope` を
//! 引数で渡すためだけの重複メソッド）が要らなくなる。
//!
//! 破棄した試作ハーネス `feat/ime-sim-harness`（`state/mode_key_pass.rs`）を参考実装として移植した
//! （ハーネス自体はマージしていない）。ただしハーネス側は本PRの `readable_at_arm`（round2 A-N2、
//! BUG-151原因③対策）より前の版なので、`ModeKeyPassMark`・`should_drop_intents_for_mode_key_pass`・
//! `should_align_after_expired_mode_key_pass` はここで `readable_at_arm` を持つ現行版に揃えてある。

use super::scoped_latch::{ScopeCheck, ScopedOneShot};

/// 無変換/変換の生キー通過マーク。
///
/// **意図的に `platform_state.rs` ではなくここに置く**（緊急レビュー指摘: 破棄した試作ハーネス
/// `feat/ime-sim-harness` の同名関数は4引数〈`readable_at_arm` 無し〉のままで、本PRの5引数版と
/// 食い違っていた。フィールドを bool の羅列で関数に渡す形は、片方のブランチだけフィールドが増えても
/// コンパイルは通ってしまい、BUG-151原因③が黙って再発しうる。この構造体を唯一のデータソースにし、
/// 判定関数は `&ModeKeyPassMark` を受け取ることで、フィールド不足・順序違いを型エラーとして検出する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModeKeyPassMark {
    pub(crate) armed_at_ms: u64,
    /// 意図の破棄は通過ごとに1回だけ(最初の観測の直後)。窓の間の再読み取りでは、通過より後に記録された意図を捨てない。
    pub(crate) invalidated: bool,
    /// `desired_open`を観測へ揃えた（`ModeKeyPassedThrough`をdispatchした）ことがあるか。窓が切れた後の最初の
    /// 成功観測での揃えを通過につき1回に絞る（BUG-158追補2）。
    pub(crate) aligned: bool,
    /// 通過より後に、awase自身が実際にIMEへ書いた（`record_optimistic`/`record_confirmed`）か。書いたなら実IMEが
    /// 書いた値と違っても信用せず、揃えずにdrift correctionへ任せる。
    /// **過大に数える**: `record_confirmed` の呼び出し元には actuation を伴わない belief ミラー（フォーカス変更時の
    /// ミラー等、ADR-098決定5）も含まれ、それらも `true` にする。安全側（揃えない）にだけ倒れるので実害は薄いが、
    /// 追補4の揃えが「awaseが書いた」とは無関係な理由で効かなくなりうる（round2 A-N6）。
    pub(crate) awase_wrote: bool,
    /// 通過を立てた時点で、その窓が読める窓（`can_use_imm32_cross_process`）だったか。窓の途中で`imm-learning`が
    /// 降格させても、立てた時点で読めたなら窓の終了時に古い意図を捨てる（BUG-151原因③、レビュー round2 A-N2）。
    pub(crate) readable_at_arm: bool,
}

/// 通過マークの判断が呼び出し元へ求める副作用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PassEffect {
    /// 対象の明示意図（`IntentStore`）を削除する（最初の観測の直後の1回だけ）。
    pub(crate) remove_intent: bool,
    /// `ImeEvent::ModeKeyPassedThrough` を dispatch する（reducerが`last_intent`を捨てる。
    /// `desired_open` を揃えるかは呼び出し元が `on_expiry` から `align_desired = !on_expiry` を計算する。
    /// `ModeKeyPassMark` にも `PassEffect` にも持たせない — dispatch する `ImeEvent` の構築は
    /// `ImeStateHub` 側の責務のまま残す）。
    pub(crate) pass_through: bool,
}

/// スコープ `S`（本番では `crate::win32::ForegroundScope`）ごとの一回マーク。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModeKeyPassLatch<S: Copy + PartialEq> {
    latch: ScopedOneShot<S, ModeKeyPassMark>,
}

impl<S: Copy + PartialEq> ModeKeyPassLatch<S> {
    pub(crate) const fn new() -> Self {
        Self {
            latch: ScopedOneShot::new(),
        }
    }

    /// 物理モードキーを通過させた直後に呼ぶ（ADR-187）。現在のスコープに対する一回マークを立てる。
    /// `readable`: 立てた時点でこの窓が読める窓だったか（`readable_at_arm`）。
    pub(crate) fn arm(&mut self, scope: S, now_ms: u64, readable: bool) {
        self.latch.arm(
            scope,
            ModeKeyPassMark {
                armed_at_ms: now_ms,
                invalidated: false,
                aligned: false,
                awase_wrote: false,
                readable_at_arm: readable,
            },
        );
    }

    /// awaseが実際にIMEへ書いた（`applied`を更新した）ことを、有効なマークへ記録する（BUG-158追補2）。
    pub(crate) fn note_awase_write(&mut self, scope: S) {
        if let ScopeCheck::Live(mark) = self.latch.peek(scope) {
            if !mark.awase_wrote {
                self.latch.arm(
                    scope,
                    ModeKeyPassMark {
                        awase_wrote: true,
                        ..mark
                    },
                );
            }
        }
    }

    /// マークが有効で、窓（`window_ms`）の間か（消費しない）。スコープが変わっていれば`peek`が失効させる。
    pub(crate) fn live(&mut self, now_ms: u64, scope: S, window_ms: u64) -> bool {
        matches!(
            self.latch.peek(scope),
            ScopeCheck::Live(mark) if now_ms.saturating_sub(mark.armed_at_ms) < window_ms
        )
    }

    /// 窓が切れるまでの残り時間(ms)。マークが無い/スコープが変わった/窓が切れていれば`None`。
    pub(crate) fn window_remaining_ms(
        &mut self,
        now_ms: u64,
        scope: S,
        window_ms: u64,
    ) -> Option<u64> {
        let ScopeCheck::Live(mark) = self.latch.peek(scope) else {
            return None;
        };
        mode_key_pass_window_remaining_ms(now_ms.saturating_sub(mark.armed_at_ms), window_ms)
    }

    /// 立てた時点で読める窓だった（`readable_at_arm`）マークが、窓の終了を待っているとき、その残り時間(ms)。
    /// 窓の途中で読めなくなった（降格した）場合でも、立てた時点で読めた窓なら窓の終了時に起こすための
    /// 起床時刻に使う（BUG-151原因③、レビュー round2 A-N2）。既に観測成功で無効化済み（`invalidated`）なら`None`。
    pub(crate) fn expiry_wait_ms(&mut self, now_ms: u64, scope: S, window_ms: u64) -> Option<u64> {
        let ScopeCheck::Live(mark) = self.latch.peek(scope) else {
            return None;
        };
        if !mark.readable_at_arm || mark.invalidated {
            return None;
        }
        mode_key_pass_window_remaining_ms(now_ms.saturating_sub(mark.armed_at_ms), window_ms)
    }

    /// 通過マークに対して、古い明示意図を捨てて`desired_open`を揃えるかを判断する（BUG-157/158）。
    ///
    /// - `on_expiry == false`: 観測が成功したとき（窓の間だけ）。
    /// - `on_expiry == true`: 窓の終了時（観測が一度も成功しなかったときだけ、かつ立てた時点で読める窓のときだけ）。
    ///
    /// 捨てるべきでなければ`None`。捨てるなら、マークを更新し、呼び出し元が適用する副作用を返す。
    pub(crate) fn drop_decision(
        &mut self,
        now_ms: u64,
        scope: S,
        on_expiry: bool,
        has_last_intent: bool,
        window_ms: u64,
    ) -> Option<PassEffect> {
        let ScopeCheck::Live(mark) = self.latch.peek(scope) else {
            return None;
        };
        let age_ms = now_ms.saturating_sub(mark.armed_at_ms);
        if !should_drop_intents_for_mode_key_pass(&mark, age_ms, on_expiry, window_ms) {
            return None;
        }
        let first = !mark.invalidated;
        // reducerは`last_intent`を捨て、`desired_open`を観測から導ける開閉へ揃える（BUG-157）。
        // 最初の観測はGJIがキーを処理する前の古い状態のことがあるので、窓の間は観測が届くたびに
        // 揃え直す。ただし通過より後に記録された明示意図（`last_intent`）は、2回目以降では捨てない。
        let align = first || !has_last_intent;
        if first || (align && !mark.aligned) {
            self.latch.arm(
                scope,
                ModeKeyPassMark {
                    invalidated: true,
                    // 窓の終了時の破棄（`on_expiry`）は観測を得ていないので、揃えたことにしない。
                    aligned: mark.aligned || (align && !on_expiry),
                    ..mark
                },
            );
        }
        Some(PassEffect {
            remove_intent: first,
            pass_through: align,
        })
    }

    /// 通過マークの窓が**切れた後**の最初の成功観測で、`desired_open`を観測へ揃えるかを判断する（BUG-158追補2）。
    /// 揃えるなら（通過につき1回）マークを更新して`true`。
    pub(crate) fn align_after_expired(
        &mut self,
        now_ms: u64,
        scope: S,
        has_last_intent: bool,
        window_ms: u64,
    ) -> bool {
        let ScopeCheck::Live(mark) = self.latch.peek(scope) else {
            return false;
        };
        let age_ms = now_ms.saturating_sub(mark.armed_at_ms);
        if !should_align_after_expired_mode_key_pass(&mark, age_ms, window_ms, has_last_intent) {
            return false;
        }
        self.latch.arm(
            scope,
            ModeKeyPassMark {
                aligned: true,
                ..mark
            },
        );
        true
    }
}

impl<S: Copy + PartialEq> Default for ModeKeyPassLatch<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// 通過マーク（ADR-187）に対して、古い明示意図を捨ててよいか（`age_ms` = 通過からの経過）。
///
/// - `on_expiry == false`（観測が成功したとき）: 窓の間（`age_ms < window_ms`）だけ。
/// - `on_expiry == true`（窓の終了時、BUG-158）: 窓が切れて（`age_ms >= window_ms`）、まだ一度も観測で
///   捨てていない（`!mark.invalidated`）、かつ**通過を立てた時点で読める窓だった**（`mark.readable_at_arm`）ときだけ。
///   窓の間・捨て済みは何もしない。通過の途中で`imm-learning`が窓を降格させても、立てた時点で読める窓なら
///   破棄する（「今読めるか」で判定すると、降格を跨いだ通過の後始末をする者がいなくなり、古い意図が残って
///   ポーリングが止まる＝BUG-151原因③、レビュー round2 A-N2）。読めない窓（立てた時点から読めない）は
///   意図がbeliefの唯一の手がかりなので捨てない（BUG-158の見直し）。
///
/// 判定を純関数にして、`#[cfg(windows)]`配下の`platform_state`のテストに頼らずLinuxで固定する。
/// `mark`（`ModeKeyPassMark`）を引数にすることで、フィールドを bool の羅列で渡す形と違い、
/// フィールドの過不足・順序違いがコンパイルエラーになる（緊急レビュー指摘、sim-harness ブランチとの食い違い対策）。
#[must_use]
pub(crate) const fn should_drop_intents_for_mode_key_pass(
    mark: &ModeKeyPassMark,
    age_ms: u64,
    on_expiry: bool,
    window_ms: u64,
) -> bool {
    let expired = age_ms >= window_ms;
    if on_expiry {
        expired && !mark.invalidated && mark.readable_at_arm
    } else {
        !expired
    }
}

/// 通過マークの窓が**切れた後**の最初の成功観測で、`desired_open`を観測へ揃えるか（BUG-157の揃えの延長、BUG-158追補2）。
///
/// 窓の間の観測が全て時間切れ・空振りだった通過（MS-IME本体のCI）では、揃える機会が無く、`desired_open`が古いまま
/// `observed ≠ desired`が続いて、drift correctionが実IMEへ書き戻す（書き込みが効く環境ではユーザーのモードキーを閉じ直す）。
/// 揃える条件（全て満たすとき、通過につき1回だけ）:
/// - 窓が切れている（窓の間は既存の揃えが担当）
/// - まだ一度も揃えていない（`!mark.aligned`）
/// - 通過より後に、awaseが実際にIMEへ書いていない（`!mark.awase_wrote`。書いたなら実IMEに届かなかったのかもしれず、
///   drift correctionが訂正すべきなので、実IMEを信用しない）
/// - 通過より後に記録された明示意図が無い（`!has_intent`。`ModeKeyPassedThrough`は`last_intent`を捨てるので、
///   新しい意図を巻き添えにしない。`ModeKeyPassMark`には持たない値〈`shadow_model.last_intent`由来〉なので
///   引数のまま残す）
///
/// 揃えた後は通常のdrift correctionに戻る（永続的に無効化しない）。`mark`を引数にする理由は
/// `should_drop_intents_for_mode_key_pass`と同じ（緊急レビュー指摘、sim-harness ブランチとの食い違い対策）。
#[must_use]
pub(crate) const fn should_align_after_expired_mode_key_pass(
    mark: &ModeKeyPassMark,
    age_ms: u64,
    window_ms: u64,
    has_intent: bool,
) -> bool {
    age_ms >= window_ms && !mark.aligned && !mark.awase_wrote && !has_intent
}

/// `kp_stage_mode_key_follow`（無変換/変換等の生キー通過後の追随＝通過マーク＋読み直し予約）を、
/// この打鍵で立ててよいか（修飾キーの判定だけ、レビュー round3 N8）。
///
/// Shift 押下中（ATOK ではかな⇔半角英数、ADR-186 残る問題2）だけ追随を止める。Ctrl/Alt/Win は見ない
/// ——Ctrl+無変換→Ctrl+変換（Ctrl 保持のまま、spike `--resync` のリセット操作）はこれまでどおり追随する
/// 必要がある。`kp_stage_key_effect_track` の予測抑止（`modifiers_suppress_prediction`、
/// Ctrl/Alt/Win も含めて表のセルの予測を止める）とは理由が別物なので、述語を共有しない。
#[must_use]
pub(crate) const fn mode_key_follow_admits_modifiers(shift: bool) -> bool {
    !shift
}

/// 通過マークの窓が切れるまでの残り時間(ms)。窓が切れていれば`None`。
#[must_use]
pub(crate) const fn mode_key_pass_window_remaining_ms(age_ms: u64, window_ms: u64) -> Option<u64> {
    if age_ms >= window_ms {
        None
    } else {
        Some(window_ms - age_ms)
    }
}

/// 通過マークの窓の間、次のIME読み取りを何ms後に予約するか（BUG-158）。
///
/// - 直前の読み取りが**成功**した（連続失敗カウント0）: ADR-187どおり`reread_ms`ごとに読み直す
///   （最初の観測はGJI/IMEがキーを処理する前の古い状態のことがある）。
/// - 直前の読み取りが**失敗**した（`ime_on=None`等）: 読み直しを窓の終了時の1回に絞る（`remaining_ms + 1`）。
///   失敗する環境（MS-IME本体のIMMクロスプロセスprobeが50〜100ms）で60msごとに読み直すと、probeが重なって
///   連続失敗を積み上げ、`IME_DETECT_MISS_THRESHOLD`(3)で`imm-learning`が窓を`Imm32Unavailable`へ誤って降格する
///   （CI `ci/e2e-msime-native-e`）。窓の終了時の読み取りの後、`ir_stage_notify`が古い意図を捨てて通常の
///   ポーリングへ戻る。
#[must_use]
pub(crate) const fn mode_key_pass_next_read_ms(
    last_read_succeeded: bool,
    remaining_ms: u64,
    reread_ms: u64,
) -> u64 {
    if last_read_succeeded {
        reread_ms
    } else {
        remaining_ms.saturating_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の `ModeKeyPassMark`。`armed_at_ms`（age_msを別引数で渡すため0固定）・`aligned`/`awase_wrote`
    /// （このテストでは無関係）はダミー値、`invalidated`/`readable_at_arm`だけ指定する。
    const fn mark_for_drop_test(invalidated: bool, readable_at_arm: bool) -> ModeKeyPassMark {
        ModeKeyPassMark {
            armed_at_ms: 0,
            invalidated,
            aligned: false,
            awase_wrote: false,
            readable_at_arm,
        }
    }

    /// 緊急レビュー指摘(3): 窓の途中で`imm-learning`が降格しても（＝呼び出し時点で読めない窓になっていても）、
    /// `readable_at_arm`（立てた時点で読めた）が`true`のままなら、窓の終了時に古い意図を捨てる
    /// （BUG-151原因③、round2 A-N2）。この述語は「今読めるか」を引数に取らないので、呼び出し側が
    /// 降格後の値を渡しても`mark.readable_at_arm`（立てた時点の値のまま変わらない）だけで判定されることを固定する。
    #[test]
    fn should_drop_intents_for_mode_key_pass_uses_readable_at_arm_even_after_mid_window_demotion() {
        let w = 300;
        let armed_readable_now_demoted = mark_for_drop_test(false, true);
        // 窓が切れた時点で on_expiry=true が呼ばれるのは「今読めるかに関わらず」（round2 A-N2 で
        // can_use_imm32_cross_process() ゲートを外した）。mark.readable_at_arm=true のままなので捨てる。
        assert!(should_drop_intents_for_mode_key_pass(
            &armed_readable_now_demoted,
            w,
            true,
            w
        ));
    }

    /// BUG-158: 意図の破棄の判定。観測成功時は窓の間だけ、窓の終了時は窓が切れて未破棄のときだけ。
    #[test]
    fn should_drop_intents_for_mode_key_pass_distinguishes_observation_and_expiry() {
        let w = 300;
        let not_invalidated_readable = mark_for_drop_test(false, true);
        let invalidated_readable = mark_for_drop_test(true, true);
        let not_invalidated_unreadable = mark_for_drop_test(false, false);
        // 観測が成功したとき: 窓の間だけ捨てる。
        assert!(should_drop_intents_for_mode_key_pass(
            &not_invalidated_readable,
            0,
            false,
            w
        ));
        assert!(
            should_drop_intents_for_mode_key_pass(&invalidated_readable, 299, false, w),
            "2回目以降の観測でも(desired揃え)"
        );
        assert!(!should_drop_intents_for_mode_key_pass(
            &not_invalidated_readable,
            300,
            false,
            w
        ));
        // 窓の終了時: 窓の間は何もしない(最初のtickで早すぎる破棄をしない。CIで実際に起きたバグ)。
        assert!(
            !should_drop_intents_for_mode_key_pass(&not_invalidated_readable, 142, true, w),
            "窓の間は捨てない"
        );
        assert!(!should_drop_intents_for_mode_key_pass(
            &not_invalidated_readable,
            299,
            true,
            w
        ));
        // 窓が切れて未破棄なら捨てる。
        assert!(should_drop_intents_for_mode_key_pass(
            &not_invalidated_readable,
            300,
            true,
            w
        ));
        assert!(should_drop_intents_for_mode_key_pass(
            &not_invalidated_readable,
            5000,
            true,
            w
        ));
        // 観測の成功で既に捨てたなら、窓が切れても捨てない(通過より後の明示意図を守る)。
        assert!(!should_drop_intents_for_mode_key_pass(
            &invalidated_readable,
            300,
            true,
            w
        ));
        // レビュー round2 A-N2: 立てた時点で読めない窓（blind）は、窓が切れても意図を捨てない（beliefの唯一の手がかり）。
        assert!(!should_drop_intents_for_mode_key_pass(
            &not_invalidated_unreadable,
            300,
            true,
            w
        ));
        assert!(!should_drop_intents_for_mode_key_pass(
            &not_invalidated_unreadable,
            5000,
            true,
            w
        ));
        // 観測成功時の破棄（窓の間）は、立てた時点の読める/読めないに依らない（既存の挙動）。
        assert!(should_drop_intents_for_mode_key_pass(
            &not_invalidated_unreadable,
            0,
            false,
            w
        ));
    }

    /// BUG-158: 通過マークの窓の間の読み直し間隔。成功なら再読み取り間隔、失敗なら窓の終了時の1回だけ。
    #[test]
    fn mode_key_pass_next_read_ms_backs_off_after_failed_read() {
        assert_eq!(mode_key_pass_next_read_ms(true, 280, 60), 60);
        assert_eq!(mode_key_pass_next_read_ms(false, 280, 60), 281);
        assert_eq!(mode_key_pass_window_remaining_ms(20, 300), Some(280));
        assert_eq!(mode_key_pass_window_remaining_ms(300, 300), None);
        assert_eq!(mode_key_pass_window_remaining_ms(301, 300), None);
    }

    /// テスト用の `ModeKeyPassMark`。`invalidated`/`readable_at_arm`（このテストでは無関係）はダミー値。
    const fn mark_for_align_test(aligned: bool, awase_wrote: bool) -> ModeKeyPassMark {
        ModeKeyPassMark {
            armed_at_ms: 0,
            invalidated: true,
            aligned,
            awase_wrote,
            readable_at_arm: true,
        }
    }

    /// BUG-158追補2: 窓が切れた後の最初の成功観測での揃え。通過→全観測が時間切れ→窓切れ→最初の成功観測で揃い、
    /// 揃った後・awase自身の書き込み後・新しい明示意図があるときは揃えない。
    #[test]
    fn should_align_after_expired_mode_key_pass_only_once_and_not_after_awase_write() {
        let w = 300;
        let fresh = mark_for_align_test(false, false);
        let already_aligned = mark_for_align_test(true, false);
        let awase_wrote = mark_for_align_test(false, true);
        // 窓の間は既存の揃えが担当（ここでは揃えない）。
        assert!(!should_align_after_expired_mode_key_pass(
            &fresh, 299, w, false
        ));
        // 窓が切れた後の最初の成功観測で揃える。
        assert!(should_align_after_expired_mode_key_pass(
            &fresh, 300, w, false
        ));
        assert!(should_align_after_expired_mode_key_pass(
            &fresh, 9000, w, false
        ));
        // 揃えた後は通常のdrift correctionへ戻る（2回目以降は揃えない）。
        assert!(!should_align_after_expired_mode_key_pass(
            &already_aligned,
            9500,
            w,
            false
        ));
        // 通過以降にawase自身が書いたなら、実IMEを信用しない（drift correctionが訂正すべき）。
        assert!(!should_align_after_expired_mode_key_pass(
            &awase_wrote,
            9000,
            w,
            false
        ));
        // 新しい明示意図があるなら巻き添えにしない。
        assert!(!should_align_after_expired_mode_key_pass(
            &fresh, 9000, w, true
        ));
    }

    /// レビュー round3 N8: Ctrl+無変換→Ctrl+変換（Ctrl 保持のまま、spike `--resync`「リセット操作」）は
    /// 追随（通過マーク＋読み直し）を止めてはならない。Shift+無変換/変換（ATOK ではかな⇔半角英数、
    /// ADR-186 残る問題2）だけを止める。両者を混同すると `atok-resync*` の追随が構造的に効かなくなる
    /// （`kp_stage_key_effect_track` の予測抑止〈`modifiers_suppress_prediction`、Ctrl/Alt/Win 含む〉と
    /// 同じ述語を追随側でも使ってしまったのが原因。理由が別物なので述語も分ける）。
    #[test]
    fn mode_key_follow_admits_ctrl_but_not_shift() {
        // 判定は shift だけを見る。ctrl/alt/win は Ctrl+無変換→Ctrl+変換（--resync）のためあえて見ない
        // （modifier_snapshot.shift=false であれば ctrl/alt/win の保持に関わらず追随する）。
        assert!(
            mode_key_follow_admits_modifiers(false),
            "無修飾・Ctrl保持は追随する"
        );
        assert!(
            !mode_key_follow_admits_modifiers(true),
            "Shift は追随しない"
        );
    }

    /// `ModeKeyPassLatch`（`ImeStateHub`が委譲する側）自身の振る舞い。スコープを跨ぐと失効すること、
    /// `arm`→`drop_decision`（窓の間・観測成功）→`PassEffect`の内容を最小限固定する
    /// （個々の判定の全パターンは上の純関数のテストが担当するので、ここは委譲の配線だけを見る）。
    #[test]
    fn mode_key_pass_latch_arms_and_drops_within_window() {
        let mut latch: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        latch.arm(1, 100, true);
        assert!(latch.live(150, 1, 300), "窓の間は有効");

        let effect = latch
            .drop_decision(150, 1, false, false, 300)
            .expect("観測成功・明示意図なしなら破棄する");
        assert!(effect.remove_intent, "最初の観測は意図を削除する");
        assert!(effect.pass_through, "最初の観測は揃える");

        // 同一スコープでの2回目（意図なし=align継続）は remove_intent が立たない。
        let effect2 = latch
            .drop_decision(160, 1, false, false, 300)
            .expect("2回目以降の観測でも揃え直す(BUG-157)");
        assert!(!effect2.remove_intent, "2回目は意図を消さない(既に消した)");
        assert!(effect2.pass_through);

        // スコープが変わると`peek`が失効させる（`ScopedOneShot`の一回性、別テストで固定済み）。
        latch.arm(1, 200, true);
        assert!(!latch.live(210, 2, 300), "スコープが変わると失効");
        assert!(!latch.live(210, 1, 300), "失効後は同じスコープでも無効");
    }

    /// cargo-mutants実測（docs/tasks/mode-key-pass-latch-mutation-coverage.md、やること1）:
    /// `note_awase_write`は「有効なマークのawase_wroteをfalse→trueへ一度だけ立てる」グルーコードで、
    /// この構造体メソッド自身を直接呼ぶテストが無かった。`awase_wrote=true`が以後の
    /// `align_after_expired`の判定を変えることをブラックボックスで対比させる。
    #[test]
    fn note_awase_write_marks_awase_wrote_and_blocks_align_after_expired() {
        let w = 300;
        let mut with_write: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        with_write.arm(1, 0, true);
        with_write.note_awase_write(1);
        assert!(
            !with_write.align_after_expired(w, 1, false, w),
            "note_awase_writeの後は実IMEを信用せず揃えない"
        );

        // 対照群: note_awase_writeを呼ばなければ同条件で揃える。
        let mut without_write: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        without_write.arm(1, 0, true);
        assert!(
            without_write.align_after_expired(w, 1, false, w),
            "awaseが書いていなければ窓切れ後の最初の観測で揃える"
        );
    }

    /// cargo-mutants実測（同docs、やること2）: `window_remaining_ms`自身を直接呼ぶテストが無く、
    /// 「関数本体をNone/Some(0)/Some(1)に差し替える」変異が生存していた。0でも1でもない具体値
    /// （250）を確認することで、内部委譲先の純関数テストだけでは潰せなかった構造体メソッド自身の
    /// グルーコードを固定する。
    #[test]
    fn window_remaining_ms_returns_concrete_remaining_time() {
        let mut latch: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        latch.arm(1, 100, true);
        assert_eq!(latch.window_remaining_ms(150, 1, 300), Some(250));

        // マークが無ければNone。
        let mut empty: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        assert_eq!(empty.window_remaining_ms(150, 1, 300), None);

        // スコープが変わっていればNone（`peek`が失効させる）。
        let mut other_scope: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        other_scope.arm(1, 100, true);
        assert_eq!(other_scope.window_remaining_ms(150, 2, 300), None);
    }

    /// cargo-mutants実測（同docs、やること3）: `expiry_wait_ms`自身を直接呼ぶテストが無かった。
    /// `readable_at_arm × invalidated`の組み合わせを決定表として固定する
    /// （`!mark.readable_at_arm || mark.invalidated`の`||`↔`&&`・`!`削除、および
    /// 関数本体のNone/Some(0)/Some(1)差し替えを同時に潰す）。
    #[test]
    fn expiry_wait_ms_requires_readable_at_arm_and_not_invalidated() {
        let w = 300;
        // readable_at_arm=true, invalidated=false: 具体的な残り時間を返す。
        let mut readable_fresh: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        readable_fresh.arm(1, 100, true);
        assert_eq!(readable_fresh.expiry_wait_ms(150, 1, w), Some(250));

        // readable_at_arm=false: 立てた時点で読めない窓は、窓の間でもNone（BUG-158の見直し）。
        let mut unreadable: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        unreadable.arm(1, 100, false);
        assert_eq!(unreadable.expiry_wait_ms(150, 1, w), None);

        // invalidated=true（観測成功でdrop_decisionが破棄した後）: None。
        let mut invalidated: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        invalidated.arm(1, 100, true);
        invalidated
            .drop_decision(150, 1, false, false, w)
            .expect("観測成功・明示意図なしなら破棄する");
        assert_eq!(invalidated.expiry_wait_ms(160, 1, w), None);
    }

    /// cargo-mutants実測（同docs、やること4）: `drop_decision`の唯一の既存テスト
    /// （`mode_key_pass_latch_arms_and_drops_within_window`）は毎回`has_last_intent=false`固定・
    /// 1回目の呼び出しで`align`と`aligned`が同時にtrueになるため、「2回目以降・`mark.aligned`が
    /// 既にfalseのまま`first=false`に到達する」経路が通っていなかった。1回目を`on_expiry=true`
    /// （窓の終了時、観測なし。`aligned`は更新されない）で呼び、2回目をより大きい`window_ms`を渡した
    /// 観測成功（`on_expiry=false`）で呼ぶことで、`first=false かつ align=true かつ
    /// mark.aligned=false`の組み合わせに到達させ、`aligned`が実際に`false→true`へ書き変わることを
    /// `align_after_expired`で観測する。
    #[test]
    fn drop_decision_aligns_on_first_full_window_observation_after_expiry_miss() {
        let mut latch: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        latch.arm(1, 0, true);

        // 1回目: 窓の終了時(on_expiry=true)、観測なし。first=trueなのでalign=trueだが、
        // on_expiry=trueなのでalignedはfalseのまま更新されない（BUG-158の仕様）。
        let expired_effect = latch
            .drop_decision(300, 1, true, false, 300)
            .expect("読める窓が切れたのに未破棄なら破棄する");
        assert!(expired_effect.remove_intent, "初回の破棄は意図を削除する");
        assert!(
            expired_effect.pass_through,
            "初回はalign=trueでdispatchする"
        );

        // 2回目: より大きい窓を渡した観測成功(on_expiry=false)。invalidated済みなのでfirst=false、
        // has_last_intent=falseでalign=true、mark.alignedはまだfalseなので
        // `first || (align && !mark.aligned)` が効いて実際にaligned=false→trueへ書き変わる。
        let observed_effect = latch
            .drop_decision(320, 1, false, false, 1000)
            .expect("2回目: より大きい窓での観測成功");
        assert!(
            !observed_effect.remove_intent,
            "2回目は意図を消さない(既に消した)"
        );
        assert!(observed_effect.pass_through, "2回目の観測でも揃える");

        // aligned=trueへ実際に書き変わったことを align_after_expired で観測する
        // (既にtrueなら以後は揃えない=should_align_after_expired_mode_key_passがfalseを返す)。
        assert!(
            !latch.align_after_expired(9000, 1, false, 1),
            "2回目の観測でaligned=trueになったので、以後は揃えない"
        );
    }

    /// cargo-mutants実測（同docs、やること4の補助）: 上のテストは2回目呼び出し後の最終状態しか
    /// 見ないため、`mark.aligned || (align && !on_expiry)`の内側`align && !on_expiry`部分の
    /// `&&`↔`||`・`!`削除変異は、1回目(on_expiry=true)単体の直後状態を見ないと潰せない
    /// （block内では`align`は常にtrueになる不変条件があり、内側が`||`化すると
    /// `on_expiry`の値に関わらず常にtrueへ壊れる）。1回目単体で`aligned`がfalseのままであることを
    /// 別インスタンスで確認する。
    #[test]
    fn drop_decision_first_expiry_miss_alone_does_not_align_yet() {
        let w = 300;
        let mut expiry_only: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        expiry_only.arm(1, 0, true);
        expiry_only
            .drop_decision(w, 1, true, false, w)
            .expect("読める窓が切れたのに未破棄なら破棄する");
        assert!(
            expiry_only.align_after_expired(w + 1, 1, false, w),
            "1回目(on_expiry=true)の後もalignedはfalseのまま=まだ揃えられる"
        );
    }

    /// cargo-mutants実測（同docs、やること4の補助）: `aligned: mark.aligned || (...)`構築式の
    /// `mark.aligned`参照が削除される変異は、既に`align_after_expired`で揃った（`invalidated`は
    /// まだfalse）マークへ初めての`drop_decision`（`first=true`）が来た場合にだけ観測できる
    /// （それ以外では`!mark.aligned`ガードにより`mark.aligned`は常にfalseの状態でしか
    /// block内へ到達しないため、参照削除が無害化してしまう）。
    #[test]
    fn drop_decision_preserves_prior_alignment_from_align_after_expired_on_first_observation() {
        let w = 100;
        let mut latch: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        latch.arm(1, 0, true);
        // 窓の間、一度もdrop_decisionを呼ばずに窓が切れ、align_after_expiredが先に揃える。
        assert!(
            latch.align_after_expired(w, 1, false, w),
            "先にalign_after_expiredで揃う"
        );

        // その後に初めてのdrop_decision(on_expiry=true、観測なし、invalidatedはまだfalse)が来ても、
        // 既に揃っている(mark.aligned=true)ことを維持する(`mark.aligned || (...)`のmark.aligned項)。
        let effect = latch
            .drop_decision(w, 1, true, false, w)
            .expect("読める窓が切れたのに未破棄なら破棄する");
        assert!(effect.remove_intent, "初回のdrop_decisionは意図を削除する");

        // alignedがtrueのまま維持されている(false化していない)ことを確認する
        // (既にtrueなら以後は揃えない=falseを返す)。
        assert!(
            !latch.align_after_expired(w + 1, 1, false, w),
            "既に揃っているので二重に揃えない"
        );
    }

    /// cargo-mutants実測（同docs、やること5）: `align_after_expired`自身を直接呼ぶテストが
    /// 無かった（内部で委譲する純関数`should_align_after_expired_mode_key_pass`は既にテスト済み）。
    /// 「窓の間は揃えない→窓が切れたら揃える→二重に揃えない」の3段階を直接固定することで、
    /// 関数本体のtrue/false差し替え・`!`削除・`aligned`フィールド構築式の削除を一度に潰す。
    #[test]
    fn align_after_expired_aligns_once_and_persists_the_flag() {
        let w = 300;
        let mut latch: ModeKeyPassLatch<u8> = ModeKeyPassLatch::new();
        latch.arm(1, 0, true);
        assert!(
            !latch.align_after_expired(200, 1, false, w),
            "窓の間はalign_after_expiredの担当ではない"
        );
        assert!(
            latch.align_after_expired(300, 1, false, w),
            "窓が切れた後の最初の成功観測で揃える"
        );
        assert!(
            !latch.align_after_expired(9000, 1, false, w),
            "揃えた後は二重に揃えない(alignedフィールドが実際にtrueへ書き変わっている)"
        );
    }
}
