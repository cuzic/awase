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
    /// 物理 F2 (VK_DBE_HIRAGANA) KeyDown（TSF モード）。物理 F2 は素通し（BUG-173）で、名前は歴史的経緯
    NativeF2Consumed,
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
    // 経由で F2 prepend バッチを駆動する別パスで実装されており、この ColdReason
    // variant で mark_composition_cold する経路は一度も配線されなかった。
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

    /// fresh F2 再送 + settle が必要かどうか（IME 初期化系 cold reason）
    #[must_use]
    pub const fn requires_settle(self) -> bool {
        matches!(
            self,
            Self::FocusChange | Self::NativeF2Consumed | Self::SetOpenTrue
        )
    }
}
