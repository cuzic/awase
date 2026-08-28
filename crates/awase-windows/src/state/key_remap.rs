//! 物理キー1つを別の物理キーとして常時扱う単純リマップ（`key_remap` config、
//! ADR-110 参照）。
//!
//! `[[keymap]]`（`crate::keymap`）とは別物: keymap はコンボをアプリ限定で
//! インターセプトする機能で、`from`/`to` は「ある修飾状態でのキー」を表す。
//! こちらは修飾キーとしての役割そのものを恒久的に入れ替える
//! （Left Ctrl を CapsLock として扱う等）ための、より低レベルで単純な機構。
//! Alt なりすまし（`state::alt_impersonation`）と同じ「フックの最初期で vk を
//! 書き換え、以後の全パイプラインに新しい vk として流す」方式だが、対象は
//! エンジン ON/OFF に関わらず常時有効（NICOLA チョードと無関係な、秀Caps 的な
//! 一般キーリマップのため）。
//!
//! ADR-110 決定2（r3）: hold-state は「ルールのスロット」ではなく「物理 VK」で
//! インデックスし、bool ではなく解決済みの target VK 自体を latch する
//! （`decide_alt_impersonation` の bool ペア方式は、config reload で `to` が
//! 変わる／エントリが削除されるケースで破綻することが Opus レビューで判明した
//! ため採用しなかった）。

use awase::types::VkCode;

/// `key_remap` テーブルの最大エントリ数。`HookConfig` を経由せず、フック
/// スレッドが自前の lock-free atomics（ダブルバッファ、ADR-110 決定8）経由で
/// 読むための上限。
pub const MAX_KEY_REMAPS: usize = 8;

/// hold-state（`LATCHED_TARGET`）配列のサイズ。Windows の VK コードは全て
/// 1バイトに収まる（`PHYSICAL_KEY_STATE` と同じ前提）。
pub const LATCH_TABLE_SIZE: usize = 256;

/// 新規押下時点でのみ `configured_target` を読み直し、以後の auto-repeat/
/// KeyUp は latch された値をそのまま使う。KeyUp は必ず latch を 0（非リマップ）
/// にクリアする（`decide_alt_impersonation` と同じ「KeyUp 後は必ず false」
/// 不変条件、BUG-41 対策そのもの）。
///
/// `latched_target`/`configured_target`/戻り値の2要素目は 0 が「非リマップ」を
/// 表す（VK コード `0x00` は実在のキーに割り当てられないため番兵として使える）。
///
/// 戻り値: `(実際に使う vk, 次に latch すべき値)`
#[must_use]
pub const fn decide_simple_remap(
    original_vk: VkCode,
    is_keydown: bool,
    latched_target: u16,
    configured_target: u16,
) -> (VkCode, u16) {
    let is_fresh_press = is_keydown && latched_target == 0;
    let effective_target = if is_fresh_press {
        configured_target
    } else {
        latched_target
    };
    let vk = if effective_target != 0 {
        VkCode(effective_target)
    } else {
        original_vk
    };
    let next_latch = if is_keydown { effective_target } else { 0 };
    (vk, next_latch)
}

/// `key_remaps` の中から `to` が Ctrl 系（`VK_CONTROL`/`VK_LCONTROL`/
/// `VK_RCONTROL`）であるエントリの `from` が物理的に held されているかを
/// 合わせて見る、Ctrl 物理押下判定（ADR-110 決定5）。
///
/// `key_remap` で Ctrl⇔他キーの入れ替えを行うと、素の
/// `is_physical_key_down(VK_LCONTROL/RCONTROL)` だけでは「リマップ元の
/// キーで Ctrl として振る舞っている」ことを検出できない
/// （`from=VK_CAPITAL to=VK_LCONTROL` で物理 Ctrl は一切押されていないため）。
#[must_use]
pub fn effective_ctrl_physically_held(
    key_remaps: &[(VkCode, VkCode)],
    is_physical_key_down: impl Fn(VkCode) -> bool,
) -> bool {
    use crate::vk::{is_ctrl_variant, VK_LCONTROL, VK_RCONTROL};
    is_physical_key_down(VK_LCONTROL)
        || is_physical_key_down(VK_RCONTROL)
        || key_remaps
            .iter()
            .any(|&(from, to)| is_ctrl_variant(to) && is_physical_key_down(from))
}

/// Ctrl 消費追跡（`CTRL_CONSUMED_SINCE_DOWN`）の reset 条件（ADR-110 決定5）。
///
/// 書き換え前の `original_vk` 自体が Ctrl 系だった場合（`from`=Ctrl系→非Ctrl
/// の物理キー自身の押下）と、書き換え後の `vk` が Ctrl 系になった場合
/// （`from`=非Ctrl→Ctrl系）の両方で reset する。`original_vk` 基準を欠くと、
/// `from`=Ctrl系→非Ctrl の押下がそれ自身を「Ctrl 押下中に別キーが押された」
/// と誤検出する（Opus レビュー r2 ラウンド F3 参照）。
#[must_use]
pub const fn is_ctrl_variant_either(original_vk: VkCode, vk: VkCode) -> bool {
    use crate::vk::is_ctrl_variant;
    is_ctrl_variant(original_vk) || is_ctrl_variant(vk)
}

/// config の `KeyRemapRule` リストをコンパイルし、`(from, to)` のテーブルに
/// 変換する（ADR-110 決定4・決定9）。以下の場合は該当エントリを skip し
/// `log::warn!` を出す（`KeymapTable::new` と同じ「警告して skip、エラーに
/// しない」方針）:
///
/// - `from`/`to` の名前解決に失敗（`VkCode::from_name`）
/// - `from == to`（無意味）
/// - `from` が既に採用済みの別エントリと重複（先勝ち）
/// - `MAX_KEY_REMAPS` を超えるエントリ
/// - `from`/`to` が Alt 系（`VK_MENU`/`VK_LMENU`/`VK_RMENU`）または Win 系
///   （`VK_LWIN`/`VK_RWIN`）——`is_alt_impersonation_active()`/`win_key_held()`
///   の held 判定が `key_remap` を認識しないため（decision4 参照）
///
/// `from` が `left_thumb_vk`/`right_thumb_vk`/`single_vk_hotkeys` のいずれかと
/// 一致する場合は skip せず警告のみ出す（decision9、NICOLA チョードや
/// ホットキーが機能しなくなる事故に気づけるようにするための注意喚起）。
#[must_use]
pub fn compile_key_remaps(
    rules: &[awase::config::KeyRemapRule],
    left_thumb_vk: VkCode,
    right_thumb_vk: VkCode,
    single_vk_hotkeys: &[VkCode],
) -> Vec<(VkCode, VkCode)> {
    use crate::vk::VkCodeExt;

    fn is_forbidden_modifier(vk: VkCode) -> bool {
        matches!(vk.0, 0x12 | 0xA4 | 0xA5 | 0x5B | 0x5C) // Alt系・Win系
    }

    let mut table: Vec<(VkCode, VkCode)> = Vec::new();
    for rule in rules {
        if table.len() >= MAX_KEY_REMAPS {
            log::warn!(
                "[key_remap] 上限 {MAX_KEY_REMAPS} 件を超えたため '{}' → '{}' を無視します",
                rule.from,
                rule.to
            );
            continue;
        }
        let Some(from) = VkCode::from_name(&rule.from) else {
            log::warn!("[key_remap] 'from' のパース失敗: {:?}", rule.from);
            continue;
        };
        let Some(to) = VkCode::from_name(&rule.to) else {
            log::warn!("[key_remap] 'to' のパース失敗: {:?}", rule.to);
            continue;
        };
        if from == to {
            log::warn!(
                "[key_remap] from == to（'{}'）は無意味なので無視します",
                rule.from
            );
            continue;
        }
        if is_forbidden_modifier(from) || is_forbidden_modifier(to) {
            log::warn!(
                "[key_remap] '{}' → '{}': Alt/Win 系キーは from/to に使用できません \
                 （ADR-110 決定4、held 判定との整合性が壊れるため）",
                rule.from,
                rule.to
            );
            continue;
        }
        if table
            .iter()
            .any(|&(existing_from, _)| existing_from == from)
        {
            log::warn!(
                "[key_remap] '{}' の重複ルールを無視します（最初の指定を優先）",
                rule.from
            );
            continue;
        }
        if from == left_thumb_vk || from == right_thumb_vk {
            log::warn!(
                "[key_remap] '{}' は現在の親指キー設定と衝突しています。NICOLA の \
                 同時打鍵チョードが機能しなくなります",
                rule.from
            );
        }
        if single_vk_hotkeys.contains(&from) {
            log::warn!(
                "[key_remap] '{}' は修飾キーなしのホットキーと衝突しています",
                rule.from
            );
        }
        table.push((from, to));
    }
    table
}

/// `KeysConfig` のホットキーのうち、修飾キーなし（bare な単一 VK）で設定されて
/// いるものの VK 一覧を返す（ADR-110 決定9: `key_remap` の `from` がこれらと
/// 衝突する場合の警告用、`compile_key_remaps` の `single_vk_hotkeys` 引数に渡す）。
#[must_use]
pub fn modifier_free_hotkey_vks(keys: &awase::config::KeysConfig) -> Vec<VkCode> {
    use crate::vk::parse_key_combo;

    let mut out = Vec::new();
    let mut consider = |combos: &[String]| {
        for s in combos {
            if let Some(combo) = parse_key_combo(s) {
                if !combo.ctrl && !combo.shift && !combo.alt {
                    out.push(combo.vk);
                }
            }
        }
    };
    consider(&keys.engine_on);
    consider(&keys.engine_off);
    consider(&keys.ime_on);
    consider(&keys.ime_off);
    consider(&keys.ime_toggle);
    if let Some(vk) = keys.engine_off_solo_repeat.as_deref().and_then(|s| {
        use crate::vk::VkCodeExt;
        VkCode::from_name(s)
    }) {
        out.push(vk);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        compile_key_remaps, decide_simple_remap, effective_ctrl_physically_held,
        is_ctrl_variant_either,
    };
    use crate::vk::{VK_CAPITAL, VK_LCONTROL, VK_NONCONVERT, VK_RCONTROL};
    use awase::config::KeyRemapRule;
    use awase::types::VkCode;

    // ── decide_simple_remap ──

    #[test]
    fn fresh_press_with_configured_target_remaps() {
        let (vk, next_latch) = decide_simple_remap(VK_CAPITAL, true, 0, VK_LCONTROL.0);
        assert_eq!(vk, VK_LCONTROL);
        assert_eq!(next_latch, VK_LCONTROL.0);
    }

    #[test]
    fn fresh_press_without_configured_target_passes_through() {
        let (vk, next_latch) = decide_simple_remap(VK_CAPITAL, true, 0, 0);
        assert_eq!(vk, VK_CAPITAL);
        assert_eq!(next_latch, 0);
    }

    #[test]
    fn repeat_keydown_uses_latched_target_even_if_configured_target_changed() {
        // 新規押下時点で LCONTROL に latch。
        let (_, latch) = decide_simple_remap(VK_CAPITAL, true, 0, VK_LCONTROL.0);
        assert_eq!(latch, VK_LCONTROL.0);
        // config reload で configured_target が変わっても、repeat は latch を使う
        // （ADR-110 決定2 r3、Opus レビュー R1 の再発防止テスト）。
        let (vk, next_latch) = decide_simple_remap(VK_CAPITAL, true, latch, VK_RCONTROL.0);
        assert_eq!(vk, VK_LCONTROL);
        assert_eq!(next_latch, VK_LCONTROL.0);
    }

    #[test]
    fn keyup_uses_latched_target_and_always_clears_to_zero() {
        let (_, latch) = decide_simple_remap(VK_CAPITAL, true, 0, VK_LCONTROL.0);
        let (vk_up, next_latch) = decide_simple_remap(VK_CAPITAL, false, latch, VK_RCONTROL.0);
        assert_eq!(
            vk_up, VK_LCONTROL,
            "KeyUp は新規押下時点でlatchされたtargetを使うべき"
        );
        assert_eq!(next_latch, 0, "KeyUp後は必ずlatchが0にクリアされるべき");
    }

    #[test]
    fn keyup_with_configured_target_removed_still_releases_the_latched_target() {
        let (_, latch) = decide_simple_remap(VK_CAPITAL, true, 0, VK_LCONTROL.0);
        // reload でエントリごと削除された(configured_target=0)場合でも、
        // KeyUp はlatch済みのLCONTROLを使う。
        let (vk_up, next_latch) = decide_simple_remap(VK_CAPITAL, false, latch, 0);
        assert_eq!(vk_up, VK_LCONTROL);
        assert_eq!(next_latch, 0);
    }

    #[test]
    fn unmapped_key_never_latches() {
        for is_keydown in [true, false] {
            let (vk, next_latch) = decide_simple_remap(VK_CAPITAL, is_keydown, 0, 0);
            assert_eq!(vk, VK_CAPITAL);
            assert_eq!(next_latch, 0);
        }
    }

    #[test]
    fn exhaustive_keyup_always_clears_latch() {
        for latched_target in [0u16, VK_LCONTROL.0] {
            for configured_target in [0u16, VK_RCONTROL.0] {
                let (_, next_latch) =
                    decide_simple_remap(VK_CAPITAL, false, latched_target, configured_target);
                assert_eq!(
                    next_latch, 0,
                    "KeyUp後は常に0であるべき: latched={latched_target} configured={configured_target}"
                );
            }
        }
    }

    // ── effective_ctrl_physically_held ──

    #[test]
    fn effective_ctrl_held_true_when_real_ctrl_physically_down() {
        let table: [(VkCode, VkCode); 0] = [];
        assert!(effective_ctrl_physically_held(&table, |vk| vk == VK_LCONTROL));
    }

    #[test]
    fn effective_ctrl_held_true_when_remapped_source_is_physically_down() {
        let table = [(VK_CAPITAL, VK_LCONTROL)];
        assert!(effective_ctrl_physically_held(&table, |vk| vk == VK_CAPITAL));
    }

    #[test]
    fn effective_ctrl_held_false_when_neither_real_nor_remapped_ctrl_is_down() {
        let table = [(VK_CAPITAL, VK_LCONTROL)];
        assert!(!effective_ctrl_physically_held(&table, |_| false));
    }

    #[test]
    fn effective_ctrl_held_ignores_non_ctrl_targets() {
        let table = [(VK_CAPITAL, VK_NONCONVERT)];
        assert!(!effective_ctrl_physically_held(&table, |vk| vk == VK_CAPITAL));
    }

    // ── is_ctrl_variant_either ──

    #[test]
    fn resets_when_original_vk_is_ctrl_even_if_rewritten_to_non_ctrl() {
        assert!(is_ctrl_variant_either(VK_LCONTROL, VK_CAPITAL));
    }

    #[test]
    fn resets_when_rewritten_vk_is_ctrl_even_if_original_was_not() {
        assert!(is_ctrl_variant_either(VK_CAPITAL, VK_LCONTROL));
    }

    #[test]
    fn does_not_reset_when_neither_is_ctrl() {
        assert!(!is_ctrl_variant_either(VK_CAPITAL, VK_NONCONVERT));
    }

    // ── compile_key_remaps ──

    fn rule(from: &str, to: &str) -> KeyRemapRule {
        KeyRemapRule {
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    #[test]
    fn valid_rule_compiles() {
        let rules = [rule("VK_CAPITAL", "VK_LCONTROL")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert_eq!(table, vec![(VK_CAPITAL, VK_LCONTROL)]);
    }

    #[test]
    fn unresolvable_name_is_skipped() {
        let rules = [rule("NOT_A_REAL_KEY", "VK_LCONTROL")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert!(table.is_empty());
    }

    #[test]
    fn from_equal_to_is_skipped() {
        let rules = [rule("VK_CAPITAL", "VK_CAPITAL")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert!(table.is_empty());
    }

    #[test]
    fn duplicate_from_keeps_first_only() {
        let rules = [
            rule("VK_CAPITAL", "VK_LCONTROL"),
            rule("VK_CAPITAL", "VK_RCONTROL"),
        ];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert_eq!(table, vec![(VK_CAPITAL, VK_LCONTROL)]);
    }

    #[test]
    fn over_capacity_entries_are_skipped() {
        let rules: Vec<KeyRemapRule> = (0..super::MAX_KEY_REMAPS + 2)
            .map(|i| {
                let from = format!("VK_F{}", i + 1);
                rule(&from, "VK_LCONTROL")
            })
            .collect();
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert_eq!(table.len(), super::MAX_KEY_REMAPS);
    }

    #[test]
    fn alt_variant_from_is_rejected() {
        let rules = [rule("VK_LMENU", "VK_CAPITAL")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert!(table.is_empty());
    }

    #[test]
    fn alt_variant_to_is_rejected() {
        let rules = [rule("VK_CAPITAL", "VK_LMENU")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert!(table.is_empty());
    }

    #[test]
    fn win_variant_is_rejected() {
        let rules = [rule("VK_LWIN", "VK_CAPITAL")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert!(table.is_empty());
    }

    #[test]
    fn collision_with_thumb_key_is_warned_but_not_skipped() {
        let rules = [rule("VK_NONCONVERT", "VK_CAPITAL")];
        let table = compile_key_remaps(&rules, VK_NONCONVERT, VkCode(0x1C), &[]);
        assert_eq!(table, vec![(VK_NONCONVERT, VK_CAPITAL)]);
    }
}
