//! アプリ単位の awase 無効化（`disable_apps`）の純粋判定（`hook.rs` から移設）。
//!
//! `hook.rs` はフォーカス中プロセス名と `config.app_overrides.disable_apps` を
//! 突き合わせて awase を丸ごとパススルーさせる。この突き合わせロジック自体は
//! `alt_impersonation.rs`/`ime_kind.rs` と同じ「純粋判定を Linux で常時テストできる
//! ようにする」移設パターンに従い、ここに切り出す。
//!
//! 動機（2026-08-25 不具合報告）: リモートデスクトップ(mstsc.exe)接続中、接続元
//! (ローカル)側の awase で Ctrl キーが押しっぱなし状態になり、Excel/iTunes 等で
//! 入力がおかしくなる。`docs/known-bugs.md` BUG-78 参照。

/// `entries` の中に `process_name` にマッチするものがあるか。
///
/// 大文字小文字を無視し、末尾の `.exe` の有無どちらでも一致する
/// （`process = "mstsc"` でも `process = "mstsc.exe"` でも同じ意味になる）。
/// 前方一致は使わない — `keymap.rs::filter_active` のような `starts_with` 方式は
/// 予期しない過剰マッチ（例: `"note"` が `"notepad.exe"` に誤爆）を招く。
///
/// `process_name` が空文字列の場合は常に `false` を返す。`get_process_name` が
/// 失敗して空文字列を返すケースがあり、これが空文字列エントリと一致すると
/// 全アプリが無効化される事故になるため、明示的にガードする。
#[must_use]
pub fn matches_disabled_app(entries: &[String], process_name: &str) -> bool {
    if process_name.is_empty() {
        return false;
    }
    let normalized = normalize_process_name(process_name);
    entries
        .iter()
        .any(|entry| !entry.is_empty() && normalize_process_name(entry) == normalized)
}

/// プロセス名を比較用に正規化する（小文字化 + 末尾 `.exe` 除去）。
fn normalize_process_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// フォーカス変更前後で「無効化対象アプリへの出入り」のどちらが起きたかを表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionEdge {
    /// 変化なし（無効化対象アプリの内側に留まる／外側に留まる）。
    None,
    /// 無効化対象アプリへ入った（後始末の Enter 処理が必要）。
    Enter,
    /// 無効化対象アプリから出た（後始末の Leave 処理が必要）。
    Leave,
}

/// 前後の無効化状態からエッジを判定する。
///
/// フォーカス変更のたびに呼ばれる想定のため、`prev == next` の場合は必ず
/// `SuppressionEdge::None` を返し、呼び出し側が「エッジでのみ後始末する」
/// 契約を守れるようにする。
#[must_use]
pub const fn edge(prev: bool, next: bool) -> SuppressionEdge {
    match (prev, next) {
        (false, true) => SuppressionEdge::Enter,
        (true, false) => SuppressionEdge::Leave,
        _ => SuppressionEdge::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_ignores_case() {
        let entries = vec!["mstsc.exe".to_string()];
        assert!(matches_disabled_app(&entries, "MSTSC.EXE"));
        assert!(matches_disabled_app(&entries, "MsTsc.exe"));
    }

    #[test]
    fn matches_with_or_without_exe_suffix() {
        let with_suffix = vec!["mstsc.exe".to_string()];
        let without_suffix = vec!["mstsc".to_string()];
        assert!(matches_disabled_app(&with_suffix, "mstsc"));
        assert!(matches_disabled_app(&without_suffix, "mstsc.exe"));
    }

    #[test]
    fn empty_list_never_matches() {
        assert!(!matches_disabled_app(&[], "mstsc.exe"));
    }

    #[test]
    fn empty_process_name_never_matches() {
        // get_process_name 失敗時の空文字列が、空エントリと一致して
        // 全アプリ無効化になる事故を防ぐガード。
        let entries = vec!["mstsc.exe".to_string(), String::new()];
        assert!(!matches_disabled_app(&entries, ""));
    }

    #[test]
    fn empty_entry_never_matches() {
        let entries = vec![String::new()];
        assert!(!matches_disabled_app(&entries, "explorer.exe"));
    }

    #[test]
    fn non_matching_process_returns_false() {
        let entries = vec!["mstsc.exe".to_string()];
        assert!(!matches_disabled_app(&entries, "notepad.exe"));
    }

    #[test]
    fn edge_detects_enter_and_leave_only() {
        assert_eq!(edge(false, true), SuppressionEdge::Enter);
        assert_eq!(edge(true, false), SuppressionEdge::Leave);
        assert_eq!(edge(false, false), SuppressionEdge::None);
        assert_eq!(edge(true, true), SuppressionEdge::None);
    }
}
