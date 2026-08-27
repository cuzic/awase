//! IME 変換モードの管理コンポーネント。
//!
//! `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）と `ConvModeAuthority`（所有権管理）を定義する。
//!
//! NOTE: `Charset`（かな形状軸）・`ConvModePolicy`（force ポリシー）・
//! `desired_mode`/`katakana_candidate`/`suppress_zenkata_until_ms`・
//! `should_reset_katakana_on_ime_on_combo`（BUG-50）は 2026-08-17、ADR-094 で
//! charset 軸の追跡自体を撤去したのに伴い削除した。詳細は ADR-094 参照。

#[cfg(test)]
use super::ime_event::HwndId;
#[cfg(test)]
use super::probe_admission::FocusEpoch;
use super::probe_admission::FocusFence;
use super::TickMs;
use awase::engine::ConvMode;

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
/// 移行済みの呼び出し元（`kp_shift_conv_guard_key_down` の MS-IME entry）が必要と
/// する variant のみを定義する。他の呼び出し元（`kp_restore_kana_from_half_width`
/// の復元リトライ、`cold_warmup.rs`/`executor.rs`/idle-conv-check 側の romaji 復元）
/// は未移行（`docs/known-bugs.md` ADR-084 追補参照）。それらを移行する際に variant
/// を追加すること — 使われない variant を先回りで用意しない（憶測での API 拡張を
/// 避ける）。`Desired`（`conv_mode_policy = force` 用）は 2026-08-17、ADR-094 で
/// force ポリシー自体を撤去したのに伴い削除した。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ConvModeTarget {
    /// IME-ON 半角英数（`conv=0x0000`）。MS-IME が Shift 単独タップを英数切替と
    /// 誤認する前に、awase 側から先回りで同じ状態を書き込む安全網。
    HalfWidthAlnum,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ConvModeTarget {
    /// `ImmSetConversionStatus`/IMC write に渡す raw conv 値。
    #[must_use]
    pub(crate) const fn imm_conv_value(self) -> u32 {
        match self {
            Self::HalfWidthAlnum => 0,
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
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ConvMutationReason {
    /// `ImeModeFsm::unconfirm` に渡すログ用ラベル。
    #[must_use]
    pub(crate) const fn as_unconfirm_label(self) -> &'static str {
        match self {
            Self::ShiftSoloTapCounter => "shift-conv-guard entry",
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

// ─── 観測 (ADR-106 決定4) ────────────────────────────────────────────────────

/// `ConvModeMgr::observe` に渡す conv 観測 1 件。
///
/// `read_at`（時間軸）・`fence`（空間軸、epoch+hwnd）を観測自身に持たせることで、
/// `ConvModeMgr::observe` が「届いた順序」ではなく「観測された時点」を基準に
/// 採否を判定できるようにする。旧 `update_from_conv(u32)` は値だけを受け取り、
/// idle-conv-check と focus-conv-check という 2 つの独立した非同期/同期経路が
/// 同じ `Cell` へ書き込んでいたため、`ConvModeMgr` 自身には「後から完了した方が
/// 常に勝つ」という暗黙の前提しかなかった（BUG-34 追補が見送った decision7
/// 「focus-conv-check の非同期 offload」を安全に行うための前提工事）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConvObservation {
    pub(crate) mode: ConvMode,
    pub(crate) read_at: TickMs,
    pub(crate) fence: FocusFence,
    pub(crate) source: ConvReadSource,
}

/// [`ConvObservation`] の発生源。ログ・診断用（採否判定には使わない）。
///
/// `FocusCheck` の唯一の構築元は `#[cfg(windows)]` の `key_pipeline.rs` のため、
/// 非 Windows では未使用になる（`state/mod.rs` の ungated モジュール群と同じ局所抑制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ConvReadSource {
    /// `kp_stage_idle_conv_check_inner`（タイピング停止後、または focus-resync トリガー）。
    IdleCheck,
    /// `apply_focus_probe` 内の TsfNative フォーカス復帰同期読み取り。
    FocusCheck,
}

// `fence` は保持しない: monotonic guard は呼び出し時点の
// `current`（呼び出し元が持つ生きた FocusFence）と観測自身の
// フィールドだけで判定でき、直近採用済み観測の fence を別途覚えておく
// 必要が無い。
#[derive(Debug, Clone, Copy)]
struct ConvModeRecord {
    mode: ConvMode,
    read_at: TickMs,
}

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check`/`apply_focus_probe` が `observe` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ConvModeMgr {
    last: std::cell::Cell<Option<ConvModeRecord>>,
}

impl Default for ConvModeMgr {
    fn default() -> Self {
        Self {
            last: std::cell::Cell::new(None),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code, unused_variables))]
impl ConvModeMgr {
    /// 新しい観測を取り込む（ADR-106 決定4）。
    ///
    /// `current` は呼び出し時点の実際のフォーカス同一性——観測が捕まえた
    /// `obs.fence` と一致しない場合、フォーカスが変わった
    /// 後に届いた stale な読み取りとして棄却する。さらに `read_at` が既に採用済み
    /// の観測より古い場合も、到着順序に関わらず棄却する（monotonic guard）。
    ///
    /// 採用の上でモードが変化した場合のみ `true` を返す（呼び出し元は info ログを出す）。
    pub(crate) fn observe(&self, obs: ConvObservation, current: FocusFence) -> bool {
        if obs.fence != current {
            log::debug!(
                "[conv-mode] stale focus context の観測を棄却: obs.epoch={} current.epoch={} \
                 obs.hwnd={:?} current.hwnd={:?} source={:?}",
                obs.fence.epoch,
                current.epoch,
                obs.fence.hwnd,
                current.hwnd,
                obs.source,
            );
            return false;
        }
        if let Some(last) = self.last.get() {
            if obs.read_at < last.read_at {
                log::debug!(
                    "[conv-mode] stale read_at の観測を棄却: obs={:?} last={:?} source={:?}",
                    obs.read_at,
                    last.read_at,
                    obs.source,
                );
                return false;
            }
        }

        let old = self.last.get().map(|r| r.mode);
        let new = obs.mode;
        self.last.set(Some(ConvModeRecord {
            mode: new,
            read_at: obs.read_at,
        }));

        if old == Some(new) {
            return false;
        }

        log::info!(
            "[conv-mode] {} → {} (source={:?})",
            old.map_or_else(|| "None".to_string(), |m| m.to_string()),
            new,
            obs.source,
        );
        true
    }

    /// 現在のモードを返す。`None` = まだ `observe` が一度も採用されていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.last.get().map(|r| r.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 実機ログで観測された値。CONV_HIRAGANA: NATIVE|FULLSHAPE|ROMAN。
    const CONV_HIRAGANA: u32 = 0x0019;

    // 実機ログで観測された別の値。CONV_ROMAN: NATIVE|FULLSHAPE|ROMAN と異なる組み合わせ。
    const CONV_ROMAN_OFF: u32 = 0x0001;

    const EPOCH: FocusEpoch = 7;
    const HWND: HwndId = HwndId(0x1234);
    const FENCE: FocusFence = FocusFence {
        epoch: EPOCH,
        hwnd: HWND,
    };

    fn obs(mode_conv: u32, read_at_ms: u64, fence: FocusFence) -> ConvObservation {
        ConvObservation {
            mode: ConvMode::from_u32(mode_conv),
            read_at: TickMs(read_at_ms),
            fence,
            source: ConvReadSource::IdleCheck,
        }
    }

    #[test]
    fn observe_reports_change_and_get_reflects_it() {
        let mgr = ConvModeMgr::default();
        assert_eq!(mgr.get(), None);
        assert!(mgr.observe(obs(CONV_HIRAGANA, 100, FENCE), FENCE));
        assert_eq!(mgr.get(), Some(ConvMode::from_u32(CONV_HIRAGANA)));
        // 同じ値の再観測は変化なし扱い。
        assert!(!mgr.observe(obs(CONV_HIRAGANA, 200, FENCE), FENCE));
    }

    // ── ADR-106 決定4: monotonic guard 全数テスト ──────────────────────────
    //
    // 軸: read_at（古い/新しい）× focus_epoch（一致/不一致）× hwnd（一致/不一致）。
    // 「棄却」= self.last が変化しない（get() が棄却前の値のまま）。

    #[test]
    fn observe_accepts_newer_read_at_matching_epoch_and_hwnd() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 100, FENCE), FENCE));
        assert!(mgr.observe(obs(CONV_ROMAN_OFF, 200, FENCE), FENCE));
        assert_eq!(mgr.get(), Some(ConvMode::from_u32(CONV_ROMAN_OFF)));
    }

    #[test]
    fn observe_rejects_older_read_at_even_with_matching_epoch_and_hwnd() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 200, FENCE), FENCE));
        // read_at がより古い観測は、後から届いても棄却する
        // （idle-conv-check と focus-conv-check が非同期に完了する順序に依存しない）。
        assert!(!mgr.observe(obs(CONV_ROMAN_OFF, 100, FENCE), FENCE));
        assert_eq!(
            mgr.get(),
            Some(ConvMode::from_u32(CONV_HIRAGANA)),
            "古い read_at の観測は last を上書きしない"
        );
    }

    #[test]
    fn observe_rejects_mismatched_focus_epoch_even_with_newer_read_at() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 100, FENCE), FENCE));
        let other_fence = FocusFence {
            epoch: EPOCH + 1,
            hwnd: HWND,
        };
        assert!(!mgr.observe(obs(CONV_ROMAN_OFF, 200, FENCE), other_fence));
        assert_eq!(
            mgr.get(),
            Some(ConvMode::from_u32(CONV_HIRAGANA)),
            "観測の focus_epoch が現在と異なるなら read_at が新しくても棄却する"
        );
    }

    #[test]
    fn observe_rejects_mismatched_hwnd_even_with_newer_read_at() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 100, FENCE), FENCE));
        let other_fence = FocusFence {
            epoch: EPOCH,
            hwnd: HwndId(HWND.0 + 1),
        };
        assert!(!mgr.observe(obs(CONV_ROMAN_OFF, 200, FENCE), other_fence));
        assert_eq!(
            mgr.get(),
            Some(ConvMode::from_u32(CONV_HIRAGANA)),
            "観測の hwnd が現在と異なるなら read_at が新しくても棄却する"
        );
    }

    #[test]
    fn observe_rejects_mismatched_epoch_and_hwnd_and_older_read_at_simultaneously() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 200, FENCE), FENCE));
        let other_fence = FocusFence {
            epoch: EPOCH + 1,
            hwnd: HwndId(HWND.0 + 1),
        };
        assert!(!mgr.observe(obs(CONV_ROMAN_OFF, 100, FENCE), other_fence));
        assert_eq!(mgr.get(), Some(ConvMode::from_u32(CONV_HIRAGANA)));
    }

    #[test]
    fn observe_accepts_equal_read_at_with_matching_epoch_and_hwnd() {
        // read_at が「厳密に古い」ときのみ棄却する（`<` であり `<=` ではない）。
        // 同一 tick 内の2回目の観測（同じ read_at）は許可する。
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 100, FENCE), FENCE));
        assert!(mgr.observe(obs(CONV_ROMAN_OFF, 100, FENCE), FENCE));
        assert_eq!(mgr.get(), Some(ConvMode::from_u32(CONV_ROMAN_OFF)));
    }

    #[test]
    fn observe_records_read_at_focus_and_hwnd_even_when_mode_unchanged() {
        // モードが変化しなくても last の read_at/focus_epoch/hwnd 自体は更新される
        // （次回 stale 判定の基準点が進む）ことを固定する。
        let mgr = ConvModeMgr::default();
        assert!(mgr.observe(obs(CONV_HIRAGANA, 100, FENCE), FENCE));
        assert!(!mgr.observe(obs(CONV_HIRAGANA, 200, FENCE), FENCE));
        // read_at=100 に戻る観測は、last が既に 200 まで進んでいるため棄却される。
        assert!(!mgr.observe(obs(CONV_ROMAN_OFF, 150, FENCE), FENCE));
        assert_eq!(mgr.get(), Some(ConvMode::from_u32(CONV_HIRAGANA)));
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
}
