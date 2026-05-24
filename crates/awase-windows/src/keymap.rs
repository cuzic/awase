//! `[[keymap]]` ルールのコンパイル済み表現とマッチング

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

/// config の `KeymapRule` リストをコンパイルする。
/// パース失敗したルールは警告ログを出して skip。
pub fn compile_keymaps(rules: &[KeymapRule]) -> Vec<CompiledKeymap> {
    let mut result = Vec::new();
    for rule in rules {
        let Some(combo) = crate::vk::parse_key_combo(&rule.from) else {
            log::warn!("[keymap] 'from' のパース失敗: {:?}", rule.from);
            continue;
        };
        let send_vk = if let Some(to) = &rule.to {
            let resolved = crate::vk::vk_name_to_code(to)
                .or_else(|| crate::vk::vk_name_to_code(&format!("VK_{to}")))
                .or_else(|| crate::vk::parse_key_combo(to).map(|c| c.vk));
            match resolved {
                Some(vk) => Some(vk),
                None => {
                    log::warn!("[keymap] 'to' のパース失敗: {:?}", to);
                    continue;
                }
            }
        } else {
            None
        };
        result.push(CompiledKeymap {
            app: rule.app.as_deref().map(str::to_lowercase),
            combo,
            send_vk,
        });
    }
    result
}

/// 現在のプロセスに適用されるルールをフィルタする。
/// `app = None` のルールは全アプリに適用。
pub fn filter_active_keymaps(all: &[CompiledKeymap], process_name: &str) -> Vec<CompiledKeymap> {
    let lower = process_name.to_lowercase();
    all.iter()
        .filter(|r| r.app.as_deref().map_or(true, |a| lower.starts_with(a) || lower == a))
        .cloned()
        .collect()
}

/// アクティブなルールから一致するものを探す。
/// 戻り値: None=マッチなし, Some(None)=消費のみ, Some(Some(vk))=送信キー
pub fn find_keymap_match(
    active: &[CompiledKeymap],
    vk: VkCode,
    mods: ModifierState,
) -> Option<Option<VkCode>> {
    active
        .iter()
        .find(|r| {
            r.combo.vk == vk
                && r.combo.ctrl == mods.ctrl
                && r.combo.shift == mods.shift
                && r.combo.alt == mods.alt
        })
        .map(|r| r.send_vk)
}
