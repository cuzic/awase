//! IME 変換モードの管理コンポーネント。
//!
//! `Charset` と `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）と `ConvModeAuthority`（所有権管理）を定義する。

pub(crate) use awase::config::ConvModePolicy;
pub(crate) use awase::engine::{Charset, ConvMode};

use super::TickMs;

// ─── 変換モード所有権 ────────────────────────────────────────────────────────────

/// IME 変換モードに対する awase の所有権状態。
///
/// awase エンジン ON/OFF・warmup 開始/終了など、conv mode 制御権の移譲時に更新される。
/// `executor` が `EngineStateChanged` で `WindowsPlatform::set_conv_mode_authority` を呼び、
/// `allows_conv_mutation()` の bool を conv mutation ゲートの唯一の実体である
/// `Output::conv_mutation_allowed`（Cell<bool>）へ push する。この enum 自体は状態を保持せず、
/// bool を導出するための分類器として使われる。
///
/// | 状態               | 説明                                                              |
/// |--------------------|-------------------------------------------------------------------|
/// | `Unknown`          | 初期状態。まだ所有権が確定していない。                            |
/// | `AwaseOwned`       | awase エンジン ON 中。conv mode を RomajiHiragana に lock する。  |
/// | `UserOwned`        | awase エンジン OFF / 非活性中。conv mode に一切触らない。         |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConvModeAuthority {
    /// 初期状態。所有権が誰にあるか不明（起動直後、エンジン状態未受信）。
    #[default]
    Unknown,
    /// awase エンジン ON 中。VK_DBE_HIRAGANA 等の conv mutation を許可する。
    AwaseOwned,
    /// awase 無効・エンジン OFF。IME conv mode に一切触らない。
    UserOwned,
    // 旧 TemporarilyUnowned（warmup 中の一時的な制御権返上）は 2026-07-06 の
    // 到達不能パス監査で撤去 — 構築サイトが一度も配線されなかった。
    // 必要になったら set_conv_mode_authority の呼び出し元とセットで再導入すること。
}

impl ConvModeAuthority {
    /// conv mode を変更する操作（VK_DBE_HIRAGANA 等・ImmSetConversionStatus）を許可するか。
    ///
    /// `AwaseOwned` のときのみ true。
    #[must_use]
    pub const fn allows_conv_mutation(self) -> bool {
        matches!(self, Self::AwaseOwned)
    }
}

// ─── conv-mode actuator（ADR-084 P1/INV-1）─────────────────────────────────────

/// `Output::actuate_conv_mode`（`output/conv_actuation.rs`）が受け取る書き込み目標。
///
/// [ADR-084](../../../../docs/adr/084-conv-mode-single-ownership-and-width-ssot.md) の
/// 提案する完全版（`Kana{katakana}`/`HalfWidthAlnum`/`Restore`）のうち、実際に
/// 移行済みの呼び出し元（`kp_shift_conv_guard_key_down` の MS-IME entry、
/// [ADR-086](../../../../docs/adr/086-force-write-trigger-and-target-identity.md)
/// Phase 2 の force-write）が必要とする variant のみを定義する。他の呼び出し元
/// （`kp_restore_kana_from_half_width` の復元リトライ、`cold_warmup.rs`/`executor.rs`/
/// idle-conv-check 側の romaji 復元）は未移行（`docs/known-bugs.md` ADR-084 追補参照）。
/// それらを移行する際に variant を追加すること — 使われない variant を先回りで
/// 用意しない（憶測での API 拡張を避ける）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ConvModeTarget {
    /// IME-ON 半角英数（`conv=0x0000`）。MS-IME が Shift 単独タップを英数切替と
    /// 誤認する前に、awase 側から先回りで同じ状態を書き込む安全網。
    HalfWidthAlnum,
    /// `conv_mode_policy = force` の目標値（`ConvModeMgr::desired_mode()`）。
    /// **`u32` の生値ではなく `ConvMode` で受け取る**（ADR-086 INV-17: force の目標値は
    /// `romaji == true` である限り必ず `IME_CMODE_ROMAN` を含まなければならず、
    /// `to_conv_bits()` を経由しない生の `u32` を渡せる形にすると、この保証を型で
    /// 強制できなくなる）。
    Desired(ConvMode),
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ConvModeTarget {
    /// `ImmSetConversionStatus`/IMC write に渡す raw conv 値。
    #[must_use]
    pub(crate) const fn imm_conv_value(self) -> u32 {
        match self {
            Self::HalfWidthAlnum => 0,
            Self::Desired(mode) => mode.to_conv_bits(),
        }
    }
}

/// `actuate_conv_mode` の呼び出し元が「なぜ conv-mode を変えるのか」を申告する理由。
///
/// [ADR-084](../../../../docs/adr/084-conv-mode-single-ownership-and-width-ssot.md) §2 の
/// 提案する完全版（`ShiftSoloTapCounter`/`HalfWidthAlnumToggle`/`WarmupRestore`/
/// `DriftCorrection`）のうち、移行済みの呼び出し元が使う variant のみを定義する
/// （`ConvModeTarget` と同じ理由）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ConvMutationReason {
    /// MS-IME が物理 Shift 単独タップを半角英数切替と誤認するのを打ち消す安全網
    /// （`kp_shift_conv_guard_key_down`、BUG-15/BUG-49）。
    ShiftSoloTapCounter,
    /// `conv_mode_policy = force` による force-write（ADR-086 Phase 2、INV-18: 観測に
    /// 基づく是正〈`DriftCorrection`/`ImmBrokenCorrection`〉と strategy variant を
    /// 共有しない）。
    ForcePolicy,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ConvMutationReason {
    /// `ImeModeFsm::unconfirm` に渡すログ用ラベル。
    #[must_use]
    pub(crate) const fn as_unconfirm_label(self) -> &'static str {
        match self {
            Self::ShiftSoloTapCounter => "shift-conv-guard entry",
            Self::ForcePolicy => "conv-mode-policy force",
        }
    }
}

/// `actuate_conv_mode` の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ConvActuationOutcome {
    /// ADR-064 の `conv_mutation_allowed`（`UserManaged` 中等）により却下され、
    /// 何も書き込まなかった。
    Rejected,
    /// gate を通過し、belief 無効化（INV-2）を同期的に行った上で、実際の
    /// Win32 書き込みを非同期 (`spawn_local`) に投げた。
    Actuated,
}

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
    /// HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する期限（`current_tick_ms` 値）。
    ///
    /// 0 = 抑制なし。以下の契機で更新される:
    ///
    /// - フォーカス変更後 1500ms: TsfNative でフォーカス直後に IMM conv が TSF を反映しない
    /// - HanKata warmup (F1+F3) 送信後 500ms: TsfNative では F3 が IMM conv の FULLSHAPE ビットを
    ///   変更しないため、F1+F3 後の IMM 読み取りが ZenKata (0x0B) を返す副作用を遮断する
    #[cfg(windows)]
    suppress_zenkata_until_ms: std::cell::Cell<u64>,
    /// 非カタカナ → カタカナ (Zenkaku/Hankaku) への遷移候補。2 回連続で同じ値を観測する
    /// まで `mode` を確定させない（BUG-19 の一発誤読ロック対策、下記 `update_from_conv` 参照）。
    katakana_candidate: std::cell::Cell<Option<ConvMode>>,
    /// `conv_mode_policy = force`（`config.toml` の `GeneralConfig::conv_mode_policy`）
    /// のときに cold 転換のたびに強制する目標モード。`IME ON/OFF`
    /// （`ImeModel::desired_open`）とは独立した別軸。
    ///
    /// awase 自身のトレイ（`ImeHiragana`/`ImeFullKatakana`/`ImeHalfKatakana`/
    /// `ImeFullAlpha`/`ImeHalfAlpha`）からのみ更新される。GJI/MS-IME 側の
    /// トレイやその他の経路で実 conv が変わっても、この値自体は変わらない
    /// （＝それらの変更は次の cold 転換で上書きされる、が設計意図）。
    desired_mode: std::cell::Cell<ConvMode>,
    /// `config.toml` の `GeneralConfig::conv_mode_policy` から反映されるポリシー。
    /// `bootstrap.rs`（起動時）と `apply_config_update`（reload 時）の両方から
    /// `set_policy` で更新される。デフォルトは `Observe`（従来動作）。
    policy: std::cell::Cell<ConvModePolicy>,
}

impl Default for ConvModeMgr {
    fn default() -> Self {
        Self {
            mode: std::cell::Cell::new(None),
            #[cfg(windows)]
            suppress_zenkata_until_ms: std::cell::Cell::new(0),
            katakana_candidate: std::cell::Cell::new(None),
            desired_mode: std::cell::Cell::new(ConvMode {
                charset: Charset::Hiragana,
                romaji: true,
            }),
            policy: std::cell::Cell::new(ConvModePolicy::Observe),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code, unused_variables))]
impl ConvModeMgr {
    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// 以後 1500ms 以内の HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する。
    /// TsfNative ウィンドウはフォーカス直後に IMM conv が TSF mode を反映しないため。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    #[cfg(windows)]
    pub(crate) fn on_focus_changed(&self, tick_ms: TickMs) {
        let until = tick_ms.0 + 1500;
        if until > self.suppress_zenkata_until_ms.get() {
            self.suppress_zenkata_until_ms.set(until);
        }
    }

    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力し `true` を返す。
    /// HankakuKatakana → ZenkakuKatakana のダウングレードは `suppress_zenkata_until_ms` 期限内
    /// であれば無視する（フォーカス後 1500ms または HanKata warmup 後 500ms）。
    ///
    /// 非カタカナ → カタカナ (Zenkaku/Hankaku) への遷移は、既存の確定モードがある場合に限り
    /// 2 回連続で同じ値を観測するまで確定させない（BUG-19: `GetForegroundWindow` 基準の
    /// conv 読み取りが、候補ウィンドウ等フォーカスが一瞬他ウィンドウに移った際に一発だけ
    /// 誤ったカタカナ conv を返すことがある。この値をここで確定させてしまうと、
    /// warmup の先頭 VK 選択がカタカナ用キーを実送信し、誤読が GJI の実状態として
    /// 定着してしまう — 詳細は `docs/known-bugs.md` BUG-19 参照）。
    ///
    /// `now_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn update_from_conv(&self, conv: u32, now_ms: TickMs) -> bool {
        let new = ConvMode::from_u32(conv);
        let old = self.mode.get();

        // 非カタカナ → カタカナ遷移候補の追跡。`new` が現在の確定モードと一致する
        // 場合（＝カタカナ候補と矛盾する読み取り）も含め、条件を満たさなければ
        // 必ず候補をクリアする。そうしないと、2回連続でなく「候補→矛盾する読み取り
        // →候補と同じ値」という間隔の空いた一致でも確定してしまう。
        let entering_katakana =
            new.charset.is_katakana() && old.is_some_and(|m| !m.charset.is_katakana());
        if entering_katakana {
            if self.katakana_candidate.get() != Some(new) {
                self.katakana_candidate.set(Some(new));
                log::debug!(
                    "[conv-mode] カタカナ遷移候補観測 (1回目、確定保留): \
                     {} → {} (conv=0x{conv:08X})",
                    old.map_or_else(|| "None".to_string(), |m| m.to_string()),
                    new,
                );
                return false;
            }
            // 2 回連続で同じカタカナ値を観測 — 確定へ進む。
        } else {
            self.katakana_candidate.set(None);
        }

        if old == Some(new) {
            return false;
        }

        #[cfg(windows)]
        if old.is_some_and(|m| m.charset == Charset::HankakuKatakana)
            && new.charset == Charset::ZenkakuKatakana
        {
            let now = now_ms.0;
            let until = self.suppress_zenkata_until_ms.get();
            if now < until {
                log::debug!(
                    "[conv-mode] HanKata→ZenKata ダウングレード抑制 \
                     (残り{}ms, conv=0x{conv:08X})",
                    until.saturating_sub(now)
                );
                return false;
            }
        }
        log::info!(
            "[conv-mode] {} → {} (conv=0x{conv:08X})",
            old.map_or_else(|| "None".to_string(), |m| m.to_string()),
            new,
        );
        self.mode.set(Some(new));
        self.katakana_candidate.set(None);
        true
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }

    /// `conv_mode_policy = force` のときに cold 転換のたびに強制する目標モードを返す。
    /// デフォルトは全角ひらがな（`Default::default()` 参照）。
    pub(crate) fn desired_mode(&self) -> ConvMode {
        self.desired_mode.get()
    }

    /// 目標モードを更新する。awase 自身のトレイ（`ImeHiragana` 等）からのみ
    /// 呼ぶこと。GJI/MS-IME 側の観測（`update_from_conv`）からは呼ばない —
    /// 「awase のトレイから変更したときだけ目標が変わる」という設計上の
    /// 唯一の書き込み点。
    pub(crate) fn set_desired_mode(&self, mode: ConvMode) {
        self.desired_mode.set(mode);
    }

    /// 現在の conv モードポリシーを返す。
    pub(crate) fn policy(&self) -> ConvModePolicy {
        self.policy.get()
    }

    /// ポリシーを更新する。`bootstrap.rs`（起動時）と `apply_config_update`
    /// （config reload 時）から呼ぶ。
    pub(crate) fn set_policy(&self, policy: ConvModePolicy) {
        self.policy.set(policy);
    }
}

/// IME-ON コンボ（既定: Ctrl+変換）押下時に、ひらがな＋ローマ字＋CapsLock OFF への
/// リセット（`kp_reset_to_hiragana_romaji_capsoff`）を起動すべきかどうかの判定。
///
/// `was_open_before`（IME open の belief、`effective_open()`）に加えて
/// `observed_katakana`（`ConvModeMgr` が現に観測しているカタカナかどうか）の
/// いずれかが真ならリセット対象にする。
///
/// # 背景（BUG-50、2026-08-05）
///
/// 従来は `was_open_before` 単独で判定していたが、belief が drift で誤って
/// `false` になっている（IME は既にカタカナへ入っているのに belief は「まだ
/// 閉じている」と誤認している）ケースでは、ユーザーが IME-ON コンボを押しても
/// このリセットが起動せず、カタカナから永久に復旧できないデッドロックになって
/// いた（`docs/known-bugs.md` BUG-50 参照）。`observed_katakana` を追加条件に
/// することで、belief が誤っていても実際に観測されたカタカナを起点にリセット
/// できるようにする。`was_open_before=true` のときの既存の破壊的リセット挙動
/// （カタカナであっても問答無用でひらがなへ寄せる）はそのまま維持されるため、
/// 新しい破壊的動作のクラスは増えない。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const fn should_reset_katakana_on_ime_on_combo(
    was_open_before: bool,
    observed_katakana: bool,
) -> bool {
    was_open_before || observed_katakana
}

#[cfg(test)]
mod tests {
    use super::*;

    // 実機ログで観測された値。CONV_HIRAGANA: NATIVE|FULLSHAPE|ROMAN。
    // CONV_ZENKATA: NATIVE|KATAKANA|FULLSHAPE|ROMAN。
    const CONV_HIRAGANA: u32 = 0x0019;
    const CONV_ZENKATA: u32 = 0x001B;

    fn t(ms: u64) -> TickMs {
        TickMs(ms)
    }

    // ── desired_mode ────────────────────────────────────────────────────────

    #[test]
    fn desired_mode_defaults_to_zenkaku_hiragana() {
        let mgr = ConvModeMgr::default();
        let d = mgr.desired_mode();
        assert_eq!(d.charset, Charset::Hiragana);
        assert!(d.romaji);
    }

    #[test]
    fn desired_mode_only_changes_via_set_desired_mode() {
        let mgr = ConvModeMgr::default();
        let zenkata = ConvMode::from_u32(CONV_ZENKATA);
        // update_from_conv（GJI/MS-IME 側の観測）を流しても、desired_mode は
        // 一切変化しない（awase トレイ経由の set_desired_mode だけが唯一の
        // 書き込み点、という設計の回帰テスト）。
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0))); // 初回観測（非カタカナ）で確定
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10))); // 1回目のカタカナ観測は保留
        assert!(mgr.update_from_conv(CONV_ZENKATA, t(20))); // 2回連続一致で確定
        assert_eq!(mgr.get(), Some(zenkata));
        assert_eq!(mgr.desired_mode().charset, Charset::Hiragana);

        mgr.set_desired_mode(zenkata);
        assert_eq!(mgr.desired_mode(), zenkata);
    }

    #[test]
    fn policy_defaults_to_observe_and_updates_via_set_policy() {
        let mgr = ConvModeMgr::default();
        assert_eq!(mgr.policy(), ConvModePolicy::Observe);
        mgr.set_policy(ConvModePolicy::Force);
        assert_eq!(mgr.policy(), ConvModePolicy::Force);
    }

    // ── ADR-084 P1 conv-mode actuator ─────────────────────────────────────────

    /// ADR-084 P1: `ConvModeTarget::HalfWidthAlnum` は `conv=0x0000`（IME-ON 半角英数）
    /// に対応する。値を変えると `actuate_conv_mode` が誤った conv を書き込む。
    #[test]
    fn half_width_alnum_target_maps_to_zero() {
        assert_eq!(ConvModeTarget::HalfWidthAlnum.imm_conv_value(), 0);
    }

    /// `ConvMutationReason` のログラベルは `ImeModeFsm::unconfirm` の既存ログ文言
    /// （`"shift-conv-guard entry"`）と一致させる。移行前のインライン呼び出しと
    /// ログの見た目を変えないための固定。
    #[test]
    fn shift_solo_tap_counter_uses_existing_unconfirm_label() {
        assert_eq!(
            ConvMutationReason::ShiftSoloTapCounter.as_unconfirm_label(),
            "shift-conv-guard entry"
        );
    }

    /// `allows_conv_mutation` は `AwaseOwned` のときのみ true。反転すると
    /// `UserOwned`（エンジン OFF 中）でも awase が conv mode を書き換えてしまい、
    /// ユーザーが選択した IME 設定を破壊する（conv mode は再発ファミリーの一つ）。
    #[test]
    fn allows_conv_mutation_only_when_awase_owned() {
        assert!(ConvModeAuthority::AwaseOwned.allows_conv_mutation());
        assert!(!ConvModeAuthority::UserOwned.allows_conv_mutation());
        assert!(!ConvModeAuthority::Unknown.allows_conv_mutation());
    }

    /// ADR-086 INV-17 の回帰テスト: `ConvModeTarget::Desired` の `imm_conv_value()` は
    /// `romaji == true` である限り、charset に関わらず必ず `IME_CMODE_ROMAN`（0x10）を
    /// 含む。含まなくなると romaji 入力中に engine が非対応と誤認し BUG-08 系の
    /// 文字化けを再発する。全 charset を網羅する（1つだけ確認すると、その charset
    /// だけ特別扱いする実装ミスを見逃す）。
    #[test]
    fn desired_target_preserves_roman_bit_for_all_charsets() {
        const IME_CMODE_ROMAN: u32 = 0x10;
        for charset in [
            Charset::Hiragana,
            Charset::ZenkakuKatakana,
            Charset::HankakuKatakana,
            Charset::ZenkakuAlpha,
            Charset::HankakuAlpha,
        ] {
            let mode = ConvMode {
                charset,
                romaji: true,
            };
            let raw = ConvModeTarget::Desired(mode).imm_conv_value();
            assert_eq!(
                raw & IME_CMODE_ROMAN,
                IME_CMODE_ROMAN,
                "charset={charset:?} で ROMAN ビットが落ちている (raw=0x{raw:08X})"
            );
        }
    }

    /// force-write の `ConvMutationReason::ForcePolicy` は観測是正系の reason
    /// （`ShiftSoloTapCounter` は投機的安全網だが force ではない）とログラベルを
    /// 共有しない（INV-18: force と observation-based correction を型で区別する）。
    #[test]
    fn force_policy_uses_distinct_unconfirm_label() {
        assert_eq!(
            ConvMutationReason::ForcePolicy.as_unconfirm_label(),
            "conv-mode-policy force"
        );
        assert_ne!(
            ConvMutationReason::ForcePolicy.as_unconfirm_label(),
            ConvMutationReason::ShiftSoloTapCounter.as_unconfirm_label()
        );
    }

    /// BUG-19: 一発だけのカタカナ観測は確定させない（`GetForegroundWindow` 基準の
    /// conv 読み取りが候補ウィンドウ等から誤ってカタカナ conv を拾うケースの再現）。
    #[test]
    fn single_spurious_katakana_reading_is_not_committed() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        assert_eq!(mgr.get().unwrap().charset.to_string(), "Hiragana");

        let changed = mgr.update_from_conv(CONV_ZENKATA, t(10));
        assert!(!changed, "1回目のカタカナ観測は確定してはいけない");
        assert_eq!(
            mgr.get().unwrap().charset.to_string(),
            "Hiragana",
            "1回目の観測では mode が書き換わってはいけない"
        );
    }

    /// 2回連続で同じカタカナ値を観測したら確定する。
    #[test]
    fn katakana_reading_confirmed_after_two_consecutive_observations() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10)));

        let changed = mgr.update_from_conv(CONV_ZENKATA, t(20));
        assert!(changed, "2回連続で一致したら確定するべき");
        assert_eq!(mgr.get().unwrap().charset.to_string(), "ZenKata");
    }

    /// 候補観測の直後に元の値へ戻る読み取りが入った場合、候補はクリアされ、
    /// その後に同じカタカナ値が来ても「1回目」として扱われる（間隔の空いた
    /// 一致で確定してしまわないことの回帰テスト）。
    #[test]
    fn intervening_reading_resets_katakana_candidate() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10)), "1回目: 保留");
        assert!(
            !mgr.update_from_conv(CONV_HIRAGANA, t(20)),
            "現状維持の再観測（変化なし）"
        );

        // 直前に矛盾する読み取り(Hiragana)があったため、これは改めて「1回目」。
        let changed = mgr.update_from_conv(CONV_ZENKATA, t(30));
        assert!(
            !changed,
            "間に矛盾する読み取りを挟んだ場合、確定までもう一度連続一致が必要"
        );
        assert_eq!(mgr.get().unwrap().charset.to_string(), "Hiragana");

        assert!(
            mgr.update_from_conv(CONV_ZENKATA, t(40)),
            "改めて2回連続で確定"
        );
        assert_eq!(mgr.get().unwrap().charset.to_string(), "ZenKata");
    }

    /// 初回観測（`old` が `None`）はデバウンス対象外 — 起動直後にカタカナの
    /// アプリへフォーカスした場合等、正当なケースを即座に反映する。
    #[test]
    fn first_ever_observation_is_not_debounced_even_if_katakana() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_ZENKATA, t(0)));
        assert_eq!(mgr.get().unwrap().charset.to_string(), "ZenKata");
    }

    /// カタカナ以外への遷移（英数化等）は従来通りデバウンスなしで即確定する。
    #[test]
    fn non_katakana_transitions_are_unaffected() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        // 半角英数 (conv=0)
        assert!(mgr.update_from_conv(0x0000, t(10)));
        assert_eq!(mgr.get().unwrap().charset.to_string(), "HanAlpha");
    }

    // ── should_reset_katakana_on_ime_on_combo (BUG-50) ─────────────────────

    /// 既存挙動: belief が「既に ON」なら、観測に関わらずリセットする
    /// （カタカナであっても問答無用でひらがなへ寄せる従来の破壊的挙動を維持）。
    #[test]
    fn resets_when_belief_says_already_open() {
        assert!(should_reset_katakana_on_ime_on_combo(true, false));
        assert!(should_reset_katakana_on_ime_on_combo(true, true));
    }

    /// BUG-50 の核心: belief が誤って Off でも、実際に観測された conv が
    /// カタカナならリセットする（belief 単独判定だと発火せず永久に戻れなかった）。
    #[test]
    fn resets_when_belief_is_off_but_katakana_is_observed() {
        assert!(should_reset_katakana_on_ime_on_combo(false, true));
    }

    /// 通常の OFF→ON（カタカナ観測なし）ではリセットしない
    /// （素の IME-ON として振る舞う従来挙動を変えない）。
    #[test]
    fn no_reset_for_plain_off_to_on_without_katakana_observation() {
        assert!(!should_reset_katakana_on_ime_on_combo(false, false));
    }
}
