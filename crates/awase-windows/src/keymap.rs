//! `[[keymap]]` ルールのコンパイル済み表現とマッチング

use crate::vk::VkCodeExt;
use awase::config::{KeymapRule, ParsedKeyCombo};
use awase::engine::fsm_types::ModifierState;
use awase::types::VkCode;

/// `[[keymap]]` ルールの実行時表現
#[derive(Debug, Clone)]
pub struct CompiledKeymap {
    /// マッチ対象プロセス名（lowercase, None=全アプリ）
    pub app: Option<String>,
    /// インターセプトするキーコンボ
    pub combo: ParsedKeyCombo,
    /// 再注入するキー（None=消費のみ）
    pub send_vk: Option<VkCode>,
}

/// コンパイル済みキーマップのテーブル。
///
/// `[[keymap]]` ルールをコンパイルし、フォーカス変更時のフィルタリングと
/// キーイベントのマッチングを提供する。
#[derive(Debug, Clone, Default)]
pub struct KeymapTable(Vec<CompiledKeymap>);

impl KeymapTable {
    /// config の `KeymapRule` リストをコンパイルする。
    /// パース失敗したルールは警告ログを出して skip。
    pub fn new(rules: &[KeymapRule]) -> Self {
        let mut result = Vec::new();
        for rule in rules {
            let Some(combo) = crate::vk::parse_key_combo(&rule.from) else {
                log::warn!("[keymap] 'from' のパース失敗: {:?}", rule.from);
                continue;
            };
            let send_vk = if let Some(to) = &rule.to {
                let resolved = VkCode::from_name(to)
                    .or_else(|| VkCode::from_name(&format!("VK_{to}")))
                    .or_else(|| crate::vk::parse_key_combo(to).map(|c| c.vk));
                let Some(vk) = resolved else {
                    log::warn!("[keymap] 'to' のパース失敗: {to:?}");
                    continue;
                };
                Some(vk)
            } else {
                None
            };
            result.push(CompiledKeymap {
                app: rule.app.as_deref().map(str::to_lowercase),
                combo,
                send_vk,
            });
        }
        Self(result)
    }

    /// 現在のプロセスに適用されるルールをフィルタして新しい `KeymapTable` を返す。
    /// `app = None` のルールは全アプリに適用。
    ///
    /// プロセス名の比較は完全一致（大文字小文字無視 + 末尾 `.exe` の有無不問）。
    /// 前方一致はしない（`"code"` が `"codeblocks.exe"` に誤爆する類の事故を防ぐ、
    /// `state::app_suppression::matches_disabled_app` と同じ理由・同じ正規化関数）。
    #[must_use]
    pub fn filter_active(&self, process_name: &str) -> Self {
        let normalized = crate::state::app_suppression::normalize_process_name(process_name);
        Self(
            self.0
                .iter()
                .filter(|r| {
                    r.app.as_deref().is_none_or(|a| {
                        crate::state::app_suppression::normalize_process_name(a) == normalized
                    })
                })
                .cloned()
                .collect(),
        )
    }

    /// アクティブなルールから一致するものを探す。
    /// 戻り値: None=マッチなし, Some(None)=消費のみ, Some(Some(vk))=送信キー
    #[must_use]
    pub fn find_match(&self, vk: VkCode, mods: ModifierState) -> Option<Option<VkCode>> {
        self.0
            .iter()
            .find(|r| {
                r.combo.vk == vk
                    && r.combo.ctrl == mods.ctrl
                    && r.combo.shift == mods.shift
                    && r.combo.alt == mods.alt
                    && !mods.win
            })
            .map(|r| r.send_vk)
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app: Option<&str>, from: &str, to: Option<&str>) -> KeymapRule {
        KeymapRule {
            app: app.map(str::to_string),
            from: from.to_string(),
            to: to.map(str::to_string),
        }
    }

    fn mods(ctrl: bool, shift: bool, alt: bool, win: bool) -> ModifierState {
        ModifierState {
            ctrl,
            shift,
            alt,
            win,
        }
    }

    #[test]
    fn find_match_rejects_win_modifier() {
        let table = KeymapTable::new(&[rule(None, "Ctrl+VK_I", Some("F7"))]);
        let vk_i = VkCode::from_name("VK_I").expect("VK_I resolves");

        assert!(table
            .find_match(vk_i, mods(true, false, false, false))
            .is_some());
        assert!(
            table
                .find_match(vk_i, mods(true, false, false, true))
                .is_none(),
            "Win+Ctrl+I must not match a Ctrl+I rule"
        );
    }

    #[test]
    fn filter_active_uses_exact_match_not_prefix() {
        let table = KeymapTable::new(&[rule(Some("code"), "Ctrl+VK_I", Some("F7"))]);

        // 前方一致なら "codeblocks.exe" も誤って拾ってしまう。完全一致では拾わない。
        assert!(table.filter_active("codeblocks.exe").is_empty());
        // 大文字小文字無視・.exe の有無を問わず完全一致するケースは拾う。
        assert_eq!(table.filter_active("Code.exe").len(), 1);
        assert_eq!(table.filter_active("code").len(), 1);
    }
}
