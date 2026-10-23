//! IME composition context がコールド状態になった理由(ADR-272 段階 A-2)。
//!
//! `tsf/output.rs` は Windows 専用のため、Linux でビルドする `tsf/probe.rs` などが使う
//! この列挙だけをここへ置き、`tsf::output`・`output` から再エクスポートする。

/// IME composition context がコールド状態になった理由（診断ログ用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColdReason {
    /// フォーカス変更
    #[default]
    FocusChange,
    /// `ImeEffect::SetOpen(true)` 実行後
    SetOpenTrue,
    /// `ImeEffect::SetOpen(false)` 実行後（IME OFF → composition context 無効化）
    SetOpenFalse,
    /// Space/Enter/Escape のパススルー
    PassthroughConfirmKey,
    /// Space/Enter/Escape の reinject
    ReinjectConfirmKey,
    /// 記号 VK 送信後（TSF context リセット可能性あり）
    SymbolVkSent,
    /// F2 non-TSF mode passthrough
    F2NonTsf,
    // 旧 SessionExpired は 2026-07-06 の到達不能パス監査で撤去 — production の
    // 構築サイトゼロ。session-expiry は `assess_warmth` の bool（session_expired）
    // 経由の別パスで実装されていたが、それも経過時間が常に ~0ms で成立しないため
    // 撤去した（MB-05、2026-10-22）。
    /// raw TSF literal 検出後のリカバリ（バックスペース後に再 cold 扱い）
    RawTsfLiteralRecovery,
    /// Ctrl+key パススルー時の composition キャンセル（IME ショートカット横取り防止）
    CtrlKeyBypass,
    /// `[[keymap]]` の `target_vk` 送信前の composition キャンセル（IME
    /// ショートカット横取り防止、ADR-114 決定3・実装レビュー m-2）。
    /// `CtrlKeyBypass` と目的は同じだが、journal・診断で cold-start の連鎖を
    /// 追う際に「Ctrl bypass が原因」と誤誘導しないよう区別する。
    KeymapTarget,
}

impl ColdReason {
    /// confirmation キー（composition 確定/キャンセル）かどうか
    #[must_use]
    pub const fn is_confirm_key(self) -> bool {
        matches!(self, Self::PassthroughConfirmKey | Self::ReinjectConfirmKey)
    }
}

/// 物理 F2(ひらがな)の KeyDown で composition を cold にするか(`platform.rs::composition_native_f2_down` の判断)。
///
/// - warm でない: cold にする(従来どおり)。
/// - warm で GJI 系(GJI の I/O の証拠がある): cold にしない(BUG-31: warm 中の無関係な物理 IME キーで cold 化すると、
///   直後のタイピングが cold-start 経路に落ちて候補ウィンドウ可視性のレースで文字が消える)。
/// - warm でも、ADR-270 の確認を張る場合(`mozc_check_applies` = Mozc 系〈`gji_evidence_absent`〉かつ前景窓が
///   Windows Terminal・WezTerm、`focus::class_names::mozc_compose_check_applies`): cold にする。ADR-270 の確認(Mozc の最初の
///   送信の後に入力中の文字ができたか)は cold の送信でしか張られず、BUG-209(「をもと」→「wo もと」)の再現経路は
///   ひらがなキーの直後だった。旧 Tsf 注入モードは warm でも `NativeF2Consumed` で cold にしてこの確認を張っていたが、
///   Tsf 注入経路の撤去(2026-10-22)で無くなったので、Mozc 系に限ってここで同じ効果を残す(PR #809 レビュー M2)。
#[must_use]
pub(crate) const fn physical_f2_marks_cold(warm: bool, mozc_check_applies: bool) -> bool {
    !warm || mozc_check_applies
}

#[cfg(test)]
mod tests {
    use super::physical_f2_marks_cold;

    /// BUG-209 の再発防止: warm のまま Mozc 系でひらがなキーを押したら cold にする(ADR-270 の確認を張らせる)。
    /// GJI 系は BUG-31 の方針どおり warm なら cold にしない。
    #[test]
    fn physical_f2_cold_decision_table() {
        // (warm, mozc_check_applies) -> cold
        assert!(physical_f2_marks_cold(false, false), "cold のまま(GJI)");
        assert!(physical_f2_marks_cold(false, true), "cold のまま(Mozc)");
        assert!(
            !physical_f2_marks_cold(true, false),
            "BUG-31: warm の GJI は cold にしない"
        );
        assert!(
            physical_f2_marks_cold(true, true),
            "BUG-209: warm でも Mozc は cold にする"
        );
    }
}
