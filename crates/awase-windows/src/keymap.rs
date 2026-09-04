//! `[[keymap]]` ルールのコンパイル済み表現とマッチング

use crate::vk::{ImeKeyKind, VkCodeExt};
use awase::config::{KeymapRule, ParsedKeyCombo};
use awase::engine::fsm_types::ModifierState;
use awase::types::{ModifierKey, VkCode};

/// `from`/`to` に指定できない vk か、指定できないなら理由を返す（ADR-114 決定5）。
///
/// 一般原則: awase の他のロジックが静的な VK 一覧ではなく `PHYSICAL_KEY_STATE`
/// ベース（または実行時に決まる）held 判定・専用処理を持つキー全般を禁止する。
/// `left_thumb_vk`/`right_thumb_vk` は実行時値（config 由来）のため引数で受け取る。
///
/// Alt/Win 系 VK の判定は `crate::vk::classify_modifier`（左右バリアントを
/// 全て吸収する唯一の分類関数）に委譲する——独自の VK 列挙を持つと、将来
/// `classify_modifier` 側に新しい別名 VK が追加されてもここには伝播せず、
/// ADR-114 決定5 が塞ごうとしている「PHYSICAL_KEY_STATE ベースの held 判定
/// キーとの二重管理」の穴が再び開く（実装レビュー指摘）。
#[must_use]
pub fn forbidden_target_vk_reason(
    vk: VkCode,
    left_thumb_vk: VkCode,
    right_thumb_vk: VkCode,
    is_to_side: bool,
) -> Option<&'static str> {
    if vk == left_thumb_vk || vk == right_thumb_vk {
        return Some("親指キー");
    }
    if ImeKeyKind::from_vk(vk).is_some() || (is_to_side && crate::vk::vk_may_mutate_conv(vk)) {
        return Some("IME 制御系 VK");
    }
    match crate::vk::classify_modifier(vk) {
        Some(ModifierKey::Alt) => return Some("Alt 系 VK"),
        Some(ModifierKey::Meta) => return Some("Win 系 VK"),
        Some(ModifierKey::Ctrl | ModifierKey::Shift) | None => {}
    }
    if vk == crate::vk::VK_CAPITAL {
        return Some("VK_CAPITAL（ADR-111 Scancode Map プリセットと二重介入しうる）");
    }
    None
}

/// `from` の主キー（`combo.vk`）として Ctrl/Shift を指定できるか（ADR-114 決定5）。
/// 修飾子側の Ctrl/Shift（`combo.ctrl`/`combo.shift`）は対象外——別途 combo.alt と
/// 同様のチェックを行う。`forbidden_target_vk_reason` と同じ理由で
/// `classify_modifier` に委譲する。
///
/// Ctrl も Shift と同様に禁止する（実装レビューで発見、MA-3）:
/// `vk.rs::is_non_shift_modifier` の doc が「Ctrl/Alt/Win は KeyDown/KeyUp
/// ペアの保証により Ctrl スタックを防止するため、Engine をバイパスして常に
/// OS に直接渡す」と明記している不変条件を、`[[keymap]]` のステップ2は
/// `process_key_event` より手前で consume するため素通りできてしまう。
/// `from = "Ctrl+VK_LCONTROL"` を許すと、LCtrl↓ が latch され OS に届かず、
/// `send_keymap_target` の `push_restore` が `is_physical_key_down` で
/// 物理 LCtrl が押下中と判定して Ctrl↓ を復元送信したあと、実際の物理 LCtrl
/// KeyUp は latch に consume され reinject されない——OS 側で Ctrl が
/// 永久に押されたままになる（BUG-78 と同じ stuck Ctrl family）。
fn is_forbidden_ctrl_or_shift_primary_key(vk: VkCode) -> bool {
    matches!(
        crate::vk::classify_modifier(vk),
        Some(ModifierKey::Ctrl | ModifierKey::Shift)
    )
}

/// `[[keymap]]` ルールの実行時表現
#[derive(Debug, Clone)]
pub struct CompiledKeymap {
    /// マッチ対象プロセス名（lowercase, None=全アプリ）
    pub app: Option<String>,
    /// インターセプトするキーコンボ
    pub combo: ParsedKeyCombo,
    /// 再注入するキー列（空=消費のみ）。各ステップは Down+Up ペアで即時完結する。
    pub send_vks: Vec<VkCode>,
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
        'rules: for rule in rules {
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
            if is_forbidden_ctrl_or_shift_primary_key(combo.vk) {
                log::warn!(
                    "[keymap] 'from' の主キーに Ctrl/Shift は指定できません（ADR-114 決定5）: {:?}",
                    rule.from
                );
                continue;
            }
            if let Some(reason) =
                forbidden_target_vk_reason(combo.vk, left_thumb_vk, right_thumb_vk, false)
            {
                log::warn!(
                    "[keymap] 'from' に {reason} は指定できません（ADR-114 決定5）: {:?}",
                    rule.from
                );
                continue;
            }
            let mut send_vks = Vec::with_capacity(rule.to.len());
            for to in &rule.to {
                let resolved =
                    VkCode::from_name(to).or_else(|| VkCode::from_name(&format!("VK_{to}")));
                let Some(vk) = resolved else {
                    if to.contains('+') {
                        log::warn!(
                            "[keymap] 'to' に修飾キーは指定できません（ADR-130 決定2）: {to:?}"
                        );
                    } else {
                        log::warn!("[keymap] 'to' のパース失敗: {to:?}");
                    }
                    continue 'rules;
                };
                if let Some(reason) =
                    forbidden_target_vk_reason(vk, left_thumb_vk, right_thumb_vk, true)
                {
                    log::warn!(
                        "[keymap] 'to' に {reason} は指定できません（ADR-114 決定5）: {to:?}"
                    );
                    continue 'rules;
                }
                send_vks.push(vk);
            }
            result.push(CompiledKeymap {
                app: rule.app.as_deref().map(str::to_lowercase),
                combo,
                send_vks,
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
    ///
    /// `process_name` が空文字列の場合は `app = None` のルールにしか一致しない
    /// （`app_suppression::matches_disabled_app` と同じ理由——`get_process_name`
    /// がフォーカス取得失敗時に空文字列を返すケースがあり、`app = ""` という
    /// TOML 上の空文字列ルールと偶然一致してしまう事故を防ぐガード）。
    #[must_use]
    pub fn filter_active(&self, process_name: &str) -> Self {
        let normalized = crate::state::app_suppression::normalize_process_name(process_name);
        Self(
            self.0
                .iter()
                .filter(|r| match r.app.as_deref() {
                    None => true,
                    Some(_) if normalized.is_empty() => false,
                    Some(a) => {
                        crate::state::app_suppression::normalize_process_name(a) == normalized
                    }
                })
                .cloned()
                .collect(),
        )
    }

    /// アクティブなルールから一致するものを探す。
    /// 戻り値: None=マッチなし, Some(vec)=マッチあり（空 vec は消費のみ）。
    #[must_use]
    pub fn find_match(&self, vk: VkCode, mods: ModifierState) -> Option<Vec<VkCode>> {
        self.0
            .iter()
            .find(|r| {
                r.combo.vk == vk
                    && r.combo.ctrl == mods.ctrl
                    && r.combo.shift == mods.shift
                    && r.combo.alt == mods.alt
                    && !mods.win
            })
            .map(|r| r.send_vks.clone())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `vk` を主キーまたは `to` ターゲットとして使うアクティブなルールがあれば
    /// `level` で警告する（ADR-114「未解決の疑問」5、`muhenkan_solo_tap_dedicated_fn_key`
    /// のような実行時にしか確定しない vk との衝突用）。**警告のみで動作は
    /// 変えない**。戻り値は衝突の有無（テスト用）。
    ///
    /// `level` を呼び出し元に選ばせる理由（実装レビュー指摘 m-4）:
    /// `recompute_active_keymaps()` はフォーカス変更のたびに呼ばれるため、
    /// 衝突が存在する環境では `Level::Warn` 固定だと全フォーカス変更で
    /// `log::warn!` が出続け、`app.log`（不具合報告に添付される）が
    /// スパムで埋もれる。setter（`set_muhenkan_dedicated_fn_key_*`、vk が
    /// 実際に変化した瞬間のみ呼ばれる）は `Warn`、`recompute_active_keymaps()`
    /// は `Debug` を渡す。
    pub(crate) fn warn_if_vk_conflicts(
        &self,
        vk: VkCode,
        context: &str,
        level: log::Level,
    ) -> bool {
        let conflicts = self
            .0
            .iter()
            .any(|rule| rule.combo.vk == vk || rule.send_vks.contains(&vk));
        if conflicts {
            log::log!(
                level,
                "[keymap] アクティブな [[keymap]] ルールが {context}（vk=0x{:02X}）と \
                 同じキーを使っています。両方のロジックが同じ物理キーに反応します \
                 （ADR-114）",
                vk.0
            );
        }
        conflicts
    }
}

/// ADR-130 決定3: `[[keymap]]` の `to` 各ステップを独立した Down+Up ペアに展開する。
#[must_use]
pub(crate) fn keymap_target_tap_pairs(steps: &[VkCode]) -> Vec<(VkCode, bool)> {
    let mut pairs = Vec::with_capacity(steps.len() * 2);
    for &vk in steps {
        pairs.push((vk, false));
        pairs.push((vk, true));
    }
    pairs
}

/// `[[keymap]]` ルールがエンジン制御系ホットキーと同じキーコンボを奪っていないか
/// 警告する（ADR-114「未解決の疑問」4）。**警告のみで動作は変えない**——
/// ユーザー設定を勝手に無効化しない。
///
/// 決定2 の順序により `[[keymap]]` が先に消費すると、同じコンボの
/// `engine_on`/`engine_off`/`ime_on`/`ime_off`/`ime_toggle`/
/// `engine_toggle_hotkey` は黙って発火しなくなる（`engine_toggle_hotkey` は
/// `RegisterHotKey` によるグローバルホットキーだが、awase 自身の
/// `WH_KEYBOARD_LL` フックが全打鍵を無条件で一旦 swallow し PassThrough
/// 判定時のみ reinject する構造上、`[[keymap]]` が consume すれば OS 側に
/// キー自体が届かず `WM_HOTKEY` も発火しない）。
///
/// `sync_ime_toggle_auto_detect`（MS-IME レジストリ由来の自動追加分）は
/// 対象外（スコープ外、実行時の任意タイミングで追加されるため呼び出し元を
/// 増やしすぎない判断）。
pub(crate) fn warn_on_engine_hotkey_collision(
    keymaps: &[KeymapRule],
    engine_on: &[ParsedKeyCombo],
    engine_off: &[ParsedKeyCombo],
    ime_on: &[ParsedKeyCombo],
    ime_off: &[ParsedKeyCombo],
    ime_toggle: &[ParsedKeyCombo],
    engine_toggle_hotkey: Option<&str>,
) {
    // `ParsedKeyCombo` は `PartialEq` を derive 済みなので `==` で比較できる。
    //
    // `crate::vk::parse_hotkey` は Windows 専用（`windows` クレートの
    // MOD_CONTROL 等を使う）のためここでは使えない（この関数は Linux でも
    // ビルド・テストできるよう ungated にしている）。`parse_key_combo` は
    // 最後のトークンに `VK_` 接頭辞が必要だが `engine_toggle_hotkey` は
    // "Ctrl+Shift+F12" のように接頭辞なしで書く（`parse_hotkey` と同じ
    // 慣習）ため、ここで補って `parse_key_combo` に委譲する。
    let hotkey_combo = engine_toggle_hotkey.and_then(|s| {
        let prefixed = s.rfind('+').map_or_else(
            || format!("VK_{s}"),
            |idx| format!("{}+VK_{}", &s[..idx], &s[idx + 1..]),
        );
        crate::vk::parse_key_combo(&prefixed)
    });

    for rule in keymaps {
        let Some(combo) = crate::vk::parse_key_combo(&rule.from) else {
            continue;
        };
        for (label, combos) in [
            ("engine_on", engine_on),
            ("engine_off", engine_off),
            ("ime_on", ime_on),
            ("ime_off", ime_off),
            ("ime_toggle", ime_toggle),
        ] {
            if combos.contains(&combo) {
                log::warn!(
                    "[keymap] 'from' = {:?} は keys.{label} と同じキーコンボです。\
                     [[keymap]] が先に消費するため {label} が発火しなくなります \
                     （ADR-114）",
                    rule.from
                );
            }
        }
        if let Some(hotkey) = &hotkey_combo {
            if *hotkey == combo {
                log::warn!(
                    "[keymap] 'from' = {:?} は general.engine_toggle_hotkey と \
                     同じキーコンボです。[[keymap]] が先に消費するため \
                     engine_toggle_hotkey が発火しなくなります（ADR-114）",
                    rule.from
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app: Option<&str>, from: &str, to: Option<&str>) -> KeymapRule {
        KeymapRule {
            app: app.map(str::to_string),
            from: from.to_string(),
            to: to.into_iter().map(str::to_string).collect(),
        }
    }

    fn rule_steps(app: Option<&str>, from: &str, to: &[&str]) -> KeymapRule {
        KeymapRule {
            app: app.map(str::to_string),
            from: from.to_string(),
            to: to.iter().map(|s| (*s).to_string()).collect(),
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
    fn filter_active_empty_process_name_does_not_match_empty_app_rule() {
        // `app = ""`（TOML の空文字列）というルールが、get_process_name 失敗時の
        // 空文字列 process_name と偶然一致して全アプリに適用されてしまう事故を防ぐ。
        let table = new_table(&[rule(Some(""), "Ctrl+VK_I", Some("F7"))]);
        assert!(table.filter_active("").is_empty());
        // app = None のルールは空文字列 process_name でも引き続きマッチする。
        let table_none_app = new_table(&[rule(None, "Ctrl+VK_I", Some("F7"))]);
        assert_eq!(table_none_app.filter_active("").len(), 1);
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
    fn new_skips_ctrl_as_primary_key_in_from() {
        // MA-3: Ctrl+VK_LCONTROL のような from を許すと、LCtrl の物理 KeyDown が
        // consume され latch されたまま OS に届かず、send_keymap_target の
        // push_restore が物理押下を見て Ctrl↓ を復元送信したあと、実際の
        // 物理 KeyUp は latch に食われて reinject されない stuck Ctrl になる
        // （BUG-78 と同じ family）。
        let table = new_table(&[rule(None, "Ctrl+VK_LCONTROL", Some("F7"))]);
        assert!(
            table.is_empty(),
            "Ctrl 単体主キーの from ルールは skip されるべき（stuck Ctrl 防止）"
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

    #[test]
    fn new_accepts_multiple_to_steps() {
        let table = new_table(&[rule_steps(None, "Ctrl+VK_I", &["F7", "F8"])]);
        let vk_i = VkCode::from_name("VK_I").expect("VK_I resolves");
        let vk_f7 = VkCode::from_name("VK_F7").expect("VK_F7 resolves");
        let vk_f8 = VkCode::from_name("VK_F8").expect("VK_F8 resolves");

        assert_eq!(
            table.find_match(vk_i, mods(true, false, false, false)),
            Some(vec![vk_f7, vk_f8])
        );
    }

    #[test]
    fn new_skips_modifier_combo_to_step() {
        let table = new_table(&[rule_steps(None, "Ctrl+VK_I", &["Ctrl+M", "F7"])]);
        assert!(
            table.is_empty(),
            "ADR-130 決定2: to の修飾子付きステップはルール全体を skip する"
        );
    }

    #[test]
    fn to_side_rejects_conv_mutating_vk_but_from_side_does_not() {
        let (left, _right) = thumb_vks();
        assert_eq!(
            forbidden_target_vk_reason(crate::vk::VK_CONVERT, left, crate::vk::VK_SPACE, false),
            None,
            "ADR-130 IME除外決定: vk_may_mutate_conv の OR 判定は from 側には適用しない"
        );
        assert!(
            forbidden_target_vk_reason(crate::vk::VK_CONVERT, left, crate::vk::VK_SPACE, true)
                .is_some(),
            "ADR-130 IME除外決定: to 側では VK_CONVERT を禁止する"
        );
    }

    /// `to_side_rejects_conv_mutating_vk_but_from_side_does_not` は
    /// `forbidden_target_vk_reason` を直接呼ぶだけで `is_to_side` の
    /// **呼び出し側**（`KeymapTable::new` の2箇所）を検証しない——`:113`/`:130`
    /// の `is_to_side` 引数の true/false を取り違えても全テストが通ってしまう
    /// （コードレビュー指摘 M1）。以下3件は `KeymapTable::new` 経由で検証する。
    ///
    /// 親指キーを既定（無変換/変換）から離しておく——`VK_CONVERT` が
    /// 親指キー由来で禁止されているだけだと、conv-mutating OR
    /// （ADR-130 IME除外決定）の効果と区別できない。
    fn thumb_vks_away_from_convert() -> (VkCode, VkCode) {
        (crate::vk::VK_SPACE, crate::vk::VK_RETURN)
    }

    #[test]
    fn new_forbids_conv_mutating_vk_as_to_target_via_keymap_table() {
        let (left, right) = thumb_vks_away_from_convert();
        let table = KeymapTable::new(&[rule(None, "Ctrl+VK_I", Some("VK_CONVERT"))], left, right);
        assert!(
            table.is_empty(),
            "`is_to_side=true` の OR 判定により、親指キーでなくても VK_CONVERT を \
             to に指定したルールは skip されるべき（`keymap.rs:130` の呼び出し）"
        );
    }

    #[test]
    fn new_accepts_conv_mutating_vk_as_from_primary_key_when_not_thumb_key() {
        let (left, right) = thumb_vks_away_from_convert();
        let table = KeymapTable::new(&[rule(None, "VK_CONVERT", Some("F7"))], left, right);
        assert_eq!(
            table.len(),
            1,
            "ADR-130 R2-b: OR 判定は to 側限定であり、親指キーでない \
             VK_CONVERT を from の主キーにするルールまで過剰禁止しては \
             ならない（`keymap.rs:113` の呼び出し）"
        );
    }

    #[test]
    fn new_forbids_vk_dbe_roman_and_noroman_as_to_target() {
        let table_roman = new_table(&[rule(None, "Ctrl+VK_I", Some("VK_DBE_ROMAN"))]);
        let table_noroman = new_table(&[rule(None, "Ctrl+VK_J", Some("VK_DBE_NOROMAN"))]);
        assert!(
            table_roman.is_empty(),
            "VK_DBE_ROMAN は ImeKeyKind::from_vk 対象外だが vk_may_mutate_conv \
             には含まれる——これが OR 判定を追加した理由そのもの（BUG-61、\
             一度 JIS かな側へ切り替わると復旧不能）"
        );
        assert!(
            table_noroman.is_empty(),
            "VK_DBE_NOROMAN も同様に to 側で禁止されるべき"
        );
    }

    #[test]
    fn decision1_deserializes_legacy_single_to_new_list_and_omitted_to() {
        let cfg: awase::config::AppConfig = toml::from_str(
            r#"
[[keymaps]]
from = "Ctrl+VK_I"
to = "F7"

[[keymaps]]
from = "Ctrl+VK_J"
to = ["F7", "F8"]

[[keymaps]]
from = "Ctrl+VK_K"
"#,
        )
        .expect("keymap to compatibility forms deserialize");

        assert_eq!(cfg.keymaps[0].to, vec!["F7"]);
        assert_eq!(cfg.keymaps[1].to, vec!["F7", "F8"]);
        assert!(cfg.keymaps[2].to.is_empty());
    }

    #[test]
    fn decision3_expands_each_step_to_independent_down_up_pair() {
        let vk_f7 = VkCode::from_name("VK_F7").expect("VK_F7 resolves");
        let vk_f8 = VkCode::from_name("VK_F8").expect("VK_F8 resolves");

        assert_eq!(
            keymap_target_tap_pairs(&[vk_f7, vk_f8]),
            vec![(vk_f7, false), (vk_f7, true), (vk_f8, false), (vk_f8, true)]
        );
    }

    #[test]
    fn warn_if_vk_conflicts_detects_primary_and_target_vk() {
        let table = new_table(&[rule(None, "Ctrl+VK_I", Some("F7"))]);
        let vk_i = VkCode::from_name("VK_I").expect("VK_I resolves");
        let vk_f7 = VkCode::from_name("VK_F7").expect("VK_F7 resolves");
        let vk_unrelated = VkCode::from_name("VK_Z").expect("VK_Z resolves");

        assert!(
            table.warn_if_vk_conflicts(vk_i, "test", log::Level::Debug),
            "主キーとの衝突"
        );
        assert!(
            table.warn_if_vk_conflicts(vk_f7, "test", log::Level::Debug),
            "toターゲットとの衝突"
        );
        assert!(
            !table.warn_if_vk_conflicts(vk_unrelated, "test", log::Level::Debug),
            "無関係な vk は衝突しない"
        );
    }

    #[test]
    fn warn_on_engine_hotkey_collision_does_not_panic_on_collision() {
        let combo_ctrl_i = ParsedKeyCombo {
            vk: VkCode::from_name("VK_I").expect("VK_I resolves"),
            ctrl: true,
            shift: false,
            alt: false,
        };
        // engine_on と同じコンボの [[keymap]] ルール。警告が出るだけでパニックしない
        // ことを確認する（ログ内容の検証はこのリポジトリの既存慣習に無いため行わない）。
        warn_on_engine_hotkey_collision(
            &[rule(None, "Ctrl+VK_I", Some("F7"))],
            &[combo_ctrl_i],
            &[],
            &[],
            &[],
            &[],
            Some("Ctrl+Shift+VK_F12"),
        );
    }

    #[test]
    fn warn_on_engine_hotkey_collision_does_not_panic_without_collision() {
        warn_on_engine_hotkey_collision(
            &[rule(None, "Ctrl+VK_I", Some("F7"))],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
        );
    }
}
