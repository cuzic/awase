//! `[[keymap]]` ルールのコンパイル済み表現とマッチング

use crate::vk::{ImeKeyKind, VkCodeExt};
use awase::config::{KeymapRule, ParsedKeyCombo};
use awase::engine::fsm_types::ModifierState;
use awase::types::VkCode;

/// `from`/`to` に指定できない vk か、指定できないなら理由を返す（ADR-114 決定5）。
///
/// 一般原則: awase の他のロジックが静的な VK 一覧ではなく `PHYSICAL_KEY_STATE`
/// ベース（または実行時に決まる）held 判定・専用処理を持つキー全般を禁止する。
/// `left_thumb_vk`/`right_thumb_vk` は実行時値（config 由来）のため引数で受け取る。
fn forbidden_target_vk_reason(
    vk: VkCode,
    left_thumb_vk: VkCode,
    right_thumb_vk: VkCode,
) -> Option<&'static str> {
    if vk == left_thumb_vk || vk == right_thumb_vk {
        return Some("親指キー");
    }
    if ImeKeyKind::from_vk(vk).is_some() {
        return Some("IME 制御系 VK");
    }
    if matches!(
        vk,
        crate::vk::VK_MENU | crate::vk::VK_LMENU | crate::vk::VK_RMENU
    ) {
        return Some("Alt 系 VK");
    }
    if matches!(vk, crate::vk::VK_LWIN | crate::vk::VK_RWIN) {
        return Some("Win 系 VK");
    }
    if vk == crate::vk::VK_CAPITAL {
        return Some("VK_CAPITAL（ADR-111 Scancode Map プリセットと二重介入しうる）");
    }
    None
}

/// `from` の主キー（`combo.vk`）として Shift を指定できるか（ADR-114 決定5）。
/// 修飾子側の Shift（`combo.shift`）は対象外——別途 combo.alt と同様のチェックを行う。
fn is_forbidden_shift_primary_key(vk: VkCode) -> bool {
    matches!(
        vk,
        crate::vk::VK_SHIFT | crate::vk::VK_LSHIFT | crate::vk::VK_RSHIFT
    )
}

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
    /// パース失敗・禁止 VK 使用ルールは警告ログを出して skip（ADR-114 決定5）。
    ///
    /// `left_thumb_vk`/`right_thumb_vk` は実行時値（`config.general` 由来）。
    /// `muhenkan_solo_tap_dedicated_fn_key`（GJI 専用 Fn キー、実行時にしか
    /// 確定しない）とエンジン制御系コンボとの衝突検出はここでは行わない
    /// （ADR-114「未解決の疑問」4・5、`Runtime::recompute_active_keymaps()` 側で扱う）。
    pub fn new(rules: &[KeymapRule], left_thumb_vk: VkCode, right_thumb_vk: VkCode) -> Self {
        let mut result = Vec::new();
        for rule in rules {
            let Some(combo) = crate::vk::parse_key_combo(&rule.from) else {
                log::warn!("[keymap] 'from' のパース失敗: {:?}", rule.from);
                continue;
            };
            if combo.alt {
                log::warn!(
                    "[keymap] 'from' の Alt 修飾は使用できません（ADR-114 決定5）: {:?}",
                    rule.from
                );
                continue;
            }
            if is_forbidden_shift_primary_key(combo.vk) {
                log::warn!(
                    "[keymap] 'from' の主キーに Shift は指定できません（ADR-114 決定5）: {:?}",
                    rule.from
                );
                continue;
            }
            if let Some(reason) =
                forbidden_target_vk_reason(combo.vk, left_thumb_vk, right_thumb_vk)
            {
                log::warn!(
                    "[keymap] 'from' に {reason} は指定できません（ADR-114 決定5）: {:?}",
                    rule.from
                );
                continue;
            }
            let send_vk = if let Some(to) = &rule.to {
                let resolved = VkCode::from_name(to)
                    .or_else(|| VkCode::from_name(&format!("VK_{to}")))
                    .or_else(|| crate::vk::parse_key_combo(to).map(|c| c.vk));
                let Some(vk) = resolved else {
                    log::warn!("[keymap] 'to' のパース失敗: {to:?}");
                    continue;
                };
                if let Some(reason) = forbidden_target_vk_reason(vk, left_thumb_vk, right_thumb_vk)
                {
                    log::warn!(
                        "[keymap] 'to' に {reason} は指定できません（ADR-114 決定5）: {to:?}"
                    );
                    continue;
                }
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

    /// テスト用の親指キー割り当て（無変換/変換、実運用の既定値と同じ組）。
    fn thumb_vks() -> (VkCode, VkCode) {
        (crate::vk::VK_NONCONVERT, crate::vk::VK_CONVERT)
    }

    fn new_table(rules: &[KeymapRule]) -> KeymapTable {
        let (left, right) = thumb_vks();
        KeymapTable::new(rules, left, right)
    }

    #[test]
    fn find_match_rejects_win_modifier() {
        let table = new_table(&[rule(None, "Ctrl+VK_I", Some("F7"))]);
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
        let table = new_table(&[rule(Some("code"), "Ctrl+VK_I", Some("F7"))]);

        // 前方一致なら "codeblocks.exe" も誤って拾ってしまう。完全一致では拾わない。
        assert!(table.filter_active("codeblocks.exe").is_empty());
        // 大文字小文字無視・.exe の有無を問わず完全一致するケースは拾う。
        assert_eq!(table.filter_active("Code.exe").len(), 1);
        assert_eq!(table.filter_active("code").len(), 1);
    }

    #[test]
    fn new_skips_alt_modifier_in_from() {
        let table = new_table(&[rule(None, "Alt+VK_I", Some("F7"))]);
        assert!(table.is_empty(), "Alt+X の from ルールは skip されるべき");
    }

    #[test]
    fn new_skips_shift_as_primary_key_in_from() {
        let table = new_table(&[rule(None, "VK_LSHIFT", Some("F7"))]);
        assert!(
            table.is_empty(),
            "Shift 単体主キーの from ルールは skip されるべき"
        );
    }

    #[test]
    fn new_skips_ime_control_target_vk() {
        // VK_KANJI (半角/全角) は ImeKeyKind::from_vk が Some を返す IME 制御系 VK。
        let table = new_table(&[rule(None, "Ctrl+VK_KANJI", None)]);
        assert!(
            table.is_empty(),
            "IME 制御系 VK を主キーにした from は skip されるべき"
        );
    }

    #[test]
    fn new_skips_alt_target_vk() {
        let table = new_table(&[rule(None, "Ctrl+VK_I", Some("VK_LMENU"))]);
        assert!(table.is_empty(), "Alt 系 VK への to は skip されるべき");
    }

    #[test]
    fn new_skips_win_target_vk() {
        let table = new_table(&[rule(None, "Ctrl+VK_I", Some("VK_LWIN"))]);
        assert!(table.is_empty(), "Win 系 VK への to は skip されるべき");
    }

    #[test]
    fn new_skips_capital_target_vk() {
        let table = new_table(&[rule(None, "Ctrl+VK_I", Some("VK_CAPITAL"))]);
        assert!(table.is_empty(), "VK_CAPITAL への to は skip されるべき");
    }

    #[test]
    fn new_skips_thumb_key_target_vk() {
        let (left, _right) = thumb_vks();
        // to に親指キー(無変換)を指定するルールは skip される。
        let table = KeymapTable::new(
            &[rule(None, "Ctrl+VK_I", Some("VK_NONCONVERT"))],
            left,
            crate::vk::VK_CONVERT,
        );
        assert!(table.is_empty(), "親指キーへの to は skip されるべき");
    }

    #[test]
    fn new_accepts_valid_rule() {
        let table = new_table(&[rule(None, "Ctrl+VK_I", Some("F7"))]);
        assert_eq!(
            table.len(),
            1,
            "禁止対象に該当しない通常ルールは skip されない"
        );
    }
}
