//! IME 変換モードの管理コンポーネント。
//!
//! `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）と `ConvModeAuthority`（所有権管理）を定義する。
//!
//! NOTE: `Charset`（かな形状軸）・`ConvModePolicy`（force ポリシー）・
//! `desired_mode`/`katakana_candidate`/`suppress_zenkata_until_ms`・
//! `should_reset_katakana_on_ime_on_combo`（BUG-50）は 2026-08-17、ADR-094 で
//! charset 軸の追跡自体を撤去したのに伴い削除した。詳細は ADR-094 参照。

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

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
}

impl Default for ConvModeMgr {
    fn default() -> Self {
        Self {
            mode: std::cell::Cell::new(None),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code, unused_variables))]
impl ConvModeMgr {
    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力し `true` を返す。
    pub(crate) fn update_from_conv(&self, conv: u32) -> bool {
        let new = ConvMode::from_u32(conv);
        let old = self.mode.get();

        if old == Some(new) {
            return false;
        }

        log::info!(
            "[conv-mode] {} → {} (conv=0x{conv:08X})",
            old.map_or_else(|| "None".to_string(), |m| m.to_string()),
            new,
        );
        self.mode.set(Some(new));
        true
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 実機ログで観測された値。CONV_HIRAGANA: NATIVE|FULLSHAPE|ROMAN。
    const CONV_HIRAGANA: u32 = 0x0019;

    #[test]
    fn update_from_conv_reports_change_and_get_reflects_it() {
        let mgr = ConvModeMgr::default();
        assert_eq!(mgr.get(), None);
        assert!(mgr.update_from_conv(CONV_HIRAGANA));
        assert_eq!(mgr.get(), Some(ConvMode::from_u32(CONV_HIRAGANA)));
        // 同じ値の再観測は変化なし扱い。
        assert!(!mgr.update_from_conv(CONV_HIRAGANA));
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
