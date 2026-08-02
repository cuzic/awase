//! Alt キーの親指キーなりすまし判定（純粋関数、ADR-082「決定1実施記録」の次の一歩）。
//!
//! `hook.rs` から移設した。`decide_alt_impersonation` は BUG-41（KeyUp でのなりすまし
//! フラグ stuck 化）の再発防止対象であり、実機で初めてテストが実行された 2026-07-25
//! まで検出されなかった。移設の目的は「この純粋判定を Linux の
//! `cargo test -p awase-windows --lib` から常時実行できるようにする」こと自体が
//! 再発防止の本体であり、以下のテストはその上乗せ。
//!
//! `vk` モジュール（`VK_LMENU` 等の定数）が ungated になったことで、この移設が
//! 可能になった（`parse_hotkey` のみ windows-gated、それ以外は Linux から使える）。

use awase::types::VkCode;

/// `vk`/`extended` から、この物理キーが Left Alt か Right Alt かを判定する。
///
/// `vk` が既に区別済みの VK_LMENU/VK_RMENU ならそのまま使う。汎用の VK_MENU
/// (0x12) で届いた場合は `extended`（`KBDLLHOOKSTRUCT.flags` の
/// `LLKHF_EXTENDED`）で判別する（Right Alt は拡張キー）。
#[must_use]
pub(crate) const fn classify_alt_side(vk: VkCode, extended: bool) -> (bool, bool) {
    let is_left = vk.0 == crate::vk::VK_LMENU.0 || (vk.0 == crate::vk::VK_MENU.0 && !extended);
    let is_right = vk.0 == crate::vk::VK_RMENU.0 || (vk.0 == crate::vk::VK_MENU.0 && extended);
    (is_left, is_right)
}

/// `left_thumb_key`/`right_thumb_key` 設定文字列を VkCode に解決し、
/// Alt なりすましが必要かどうかも同時に判定する。
///
/// `"Left Alt"`/`"Right Alt"`（`awase-settings` の `THUMB_KEY_OPTIONS` 参照）は
/// 物理キー名ではなく「エンジン ON 時のみ Left/Right Alt を親指キーとして扱う」
/// という特殊な指示であり、通常の VK 名パーサー（`VkCode::from_name`）には
/// 含めない。なりすまし先の VK は JIS の無変換(左)/変換(右)相当に固定する
/// （`config.rs` の `GeneralConfig::keyboard_model` doc 参照）。
///
/// 戻り値: `(親指キーとして使う VkCode, Alt なりすましを有効にするか)`。
/// 未知のキー名の場合は `None`。
#[must_use]
pub fn resolve_thumb_key(name: &str) -> Option<(VkCode, bool)> {
    use crate::vk::{VkCodeExt, VK_CONVERT, VK_NONCONVERT};
    match name {
        "Left Alt" => Some((VK_NONCONVERT, true)),
        "Right Alt" => Some((VK_CONVERT, true)),
        _ => VkCode::from_name(name).map(|vk| (vk, false)),
    }
}

/// Alt キー1個ぶんの「なりすまし」判定(純粋関数、テスト対象)。
///
/// 新規押下(`was_down=false` の `KeyDown`)時点でのみ `engine_enabled` を見て
/// 判定し直す。auto-repeat の `KeyDown`(`was_down=true`)は、直前の新規押下
/// 時点の判定(`was_impersonating`)をそのまま使う。これにより、同一の
/// 押しっぱなしセッション中に設定変更やエンジン ON/OFF 切替が起きても、途中で
/// 判定がズレて Alt が stuck modifier になることを防ぐ。
///
/// `KeyUp` は vk 翻訳(戻り値1要素目)だけは直前の判定と対称にする(押下時に
/// thumb_vk を送っていれば、離す側も同じ thumb_vk を送る)。一方、以後保持
/// すべき状態(戻り値2要素目、`ALT_L_IMPERSONATING`/`ALT_R_IMPERSONATING` に
/// 格納され `is_alt_impersonation_active()` が参照する)は、物理キーが離れた
/// 時点で必ず false に戻す。ここを `was_impersonating` のまま持ち越すと、
/// キーを離した後も「なりすまし発動中」のフラグが stuck true になり、後続の
/// 無関係な(なりすましでない)Alt 押下まで `modifiers.alt` を誤って false
/// 補正してしまう(`cec4da9` が修正した bypass 誤爆と同種の再発。2026-07-25、
/// Windows実機で初めてこのテストを実行して発見。BUG-41）。
///
/// 戻り値: `(書き換え後の vk, 次に保持すべき is_impersonating 状態)`
#[must_use]
pub(crate) fn decide_alt_impersonation(
    original_vk: VkCode,
    thumb_vk: VkCode,
    is_keydown: bool,
    was_down: bool,
    was_impersonating: bool,
    engine_enabled: bool,
) -> (VkCode, bool) {
    let is_fresh_press = is_keydown && !was_down;
    let currently_impersonating = if is_fresh_press {
        engine_enabled
    } else {
        was_impersonating
    };
    let vk = if currently_impersonating {
        thumb_vk
    } else {
        original_vk
    };
    // KeyUpの瞬間、物理キーはもう押下中ではないため、以後保持する状態は
    // 必ずfalseにする(vk翻訳自体はcurrently_impersonatingを使い、対称性を保つ)。
    let next_impersonating = if is_keydown {
        currently_impersonating
    } else {
        false
    };
    (vk, next_impersonating)
}

#[cfg(test)]
mod tests {
    use super::{classify_alt_side, decide_alt_impersonation, resolve_thumb_key};
    use crate::vk::{VK_CONVERT, VK_LMENU, VK_MENU, VK_NONCONVERT, VK_RMENU, VK_SPACE};

    const LEFT_THUMB: awase::types::VkCode = VK_NONCONVERT;

    // ── classify_alt_side / resolve_thumb_key（旧 hook.rs から無変更で移設） ──

    /// vk が既に区別済みの VK_LMENU/VK_RMENU で届く環境では、extended フラグに
    /// 関わらずそのまま Left/Right と判定する。
    #[test]
    fn classify_alt_side_specific_vk() {
        assert_eq!(classify_alt_side(VK_LMENU, false), (true, false));
        assert_eq!(classify_alt_side(VK_LMENU, true), (true, false));
        assert_eq!(classify_alt_side(VK_RMENU, false), (false, true));
        assert_eq!(classify_alt_side(VK_RMENU, true), (false, true));
    }

    /// vk が汎用の VK_MENU (0x12) で届く環境（実機で確認された挙動。
    /// `vk.rs` の `classify_modifier`/`is_ctrl_variant` が汎用形も含めて
    /// 防御的にマッチしているのと同じ理由）では、LLKHF_EXTENDED で
    /// Left/Right を判別する（Right Alt が拡張キー）。
    ///
    /// このテストは実機で「Left Alt がなりすまし機能として全く発動しない」
    /// という回帰の再発防止用（vk == VK_LMENU 直接比較のみだと、この
    /// ケースを取りこぼして常に false になっていた）。
    #[test]
    fn classify_alt_side_generic_vk_menu() {
        assert_eq!(classify_alt_side(VK_MENU, false), (true, false));
        assert_eq!(classify_alt_side(VK_MENU, true), (false, true));
    }

    /// Alt 以外の VK はどちらにも該当しない。
    #[test]
    fn classify_alt_side_unrelated_vk_is_neither() {
        assert_eq!(classify_alt_side(VK_SPACE, false), (false, false));
        assert_eq!(classify_alt_side(VK_SPACE, true), (false, false));
    }

    /// "Left Alt"/"Right Alt" は無変換/変換相当の VK に解決され、
    /// なりすましフラグが立つ。
    #[test]
    fn resolve_thumb_key_alt_sentinels() {
        assert_eq!(resolve_thumb_key("Left Alt"), Some((VK_NONCONVERT, true)));
        assert_eq!(resolve_thumb_key("Right Alt"), Some((VK_CONVERT, true)));
    }

    /// 通常の VK 名は従来通り解決され、なりすましフラグは立たない。
    #[test]
    fn resolve_thumb_key_normal_vk_name() {
        assert_eq!(resolve_thumb_key("VK_SPACE"), Some((VK_SPACE, false)));
        assert_eq!(
            resolve_thumb_key("VK_NONCONVERT"),
            Some((VK_NONCONVERT, false))
        );
    }

    /// 未知のキー名は `None`（呼び出し元が `.context(...)` でエラーにする）。
    #[test]
    fn resolve_thumb_key_unknown_name_returns_none() {
        assert_eq!(resolve_thumb_key("Not A Real Key"), None);
    }

    // ── decide_alt_impersonation（旧 hook.rs から無変更で移設、5件） ──

    /// エンジン ON・新規押下 → なりすまし発動、vk が親指キーに書き換わる。
    #[test]
    fn fresh_press_engine_on_impersonates() {
        let (vk, impersonating) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, true);
        assert_eq!(vk, LEFT_THUMB);
        assert!(impersonating);
    }

    /// エンジン OFF・新規押下 → なりすましなし、vk は元の Alt のまま。
    #[test]
    fn fresh_press_engine_off_does_not_impersonate() {
        let (vk, impersonating) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, false);
        assert_eq!(vk, VK_LMENU);
        assert!(!impersonating);
    }

    /// 押しっぱなし中（auto-repeat KeyDown）にエンジンが OFF に切り替わっても、
    /// 新規押下時点の判定（なりすまし中）を維持する。
    #[test]
    fn repeat_keydown_keeps_original_decision_even_if_engine_toggled_off() {
        // 新規押下時点: エンジン ON → なりすまし発動
        let (_, impersonating_after_fresh) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, true);
        assert!(impersonating_after_fresh);

        // repeat KeyDown 時点: エンジンが OFF に切り替わっていても was_down=true なので
        // 新規押下時点の判定（なりすまし中）を維持する。
        let (vk, impersonating) = decide_alt_impersonation(
            VK_LMENU,
            LEFT_THUMB,
            true, // is_keydown (repeat)
            true, // was_down
            impersonating_after_fresh,
            false, // engine now OFF
        );
        assert_eq!(
            vk, LEFT_THUMB,
            "repeat KeyDown はなりすまし継続すべき（途中でズレると Alt が stuck する）"
        );
        assert!(impersonating);
    }

    /// KeyUp は新規押下時点の判定をそのまま使う（KeyUp 時点でエンジン状態が
    /// 変わっていても、対応する KeyDown と対称的に扱われる）。
    #[test]
    fn keyup_uses_the_decision_recorded_at_keydown() {
        // KeyDown 時点: エンジン ON → なりすまし発動
        let (_, impersonating_after_down) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, true);

        // KeyUp 時点: エンジンが OFF に切り替わっていても、KeyDown 時点の判定を使う。
        let (vk_up, impersonating_after_up) = decide_alt_impersonation(
            VK_LMENU,
            LEFT_THUMB,
            false, // is_keydown = false (KeyUp)
            true,  // was_down (直前は押下中だった)
            impersonating_after_down,
            false, // engine now OFF
        );
        assert_eq!(
            vk_up, LEFT_THUMB,
            "KeyUp は対応する KeyDown のなりすまし判定と対称であるべき"
        );
        assert!(
            !impersonating_after_up,
            "KeyUp 後は押下状態ではないため false"
        );
    }

    /// 押していない状態から始まる通常の Alt 単体タップは、エンジン OFF なら
    /// KeyDown/KeyUp とも通常の Alt のまま（回帰: 常時なりすましにならないこと）。
    #[test]
    fn normal_alt_tap_when_engine_off_stays_as_alt_through_down_and_up() {
        let (vk_down, imp_down) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, false);
        assert_eq!(vk_down, VK_LMENU);
        assert!(!imp_down);

        let (vk_up, imp_up) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, false, true, imp_down, false);
        assert_eq!(vk_up, VK_LMENU);
        assert!(!imp_up);
    }

    // ── 網羅テーブルテスト（新規、ADR-082 レビューで JSON fixture の代替として採用） ──
    //
    // decide_alt_impersonation は (is_keydown, was_down, was_impersonating,
    // engine_enabled) の bool 4 個のみに依存する純粋関数であり、入力空間は
    // 2^4=16 通りで有限。BUG-41 は「実機で journal ダンプが取れる系列があった」
    // 種類のバグではなく「既存の決定論的ユニットテストが Windows 実機で初めて
    // 実行され失敗した」種類のバグ（ADR-082 レビュー参照）であるため、journal
    // フィクスチャではなく網羅テーブル + 不変条件アサーションで固定化する。
    //
    // オラクル値は関数 doc コメント（上記）の仕様から手で導出し、実装をコピーした
    // 別実装は行わない（トートロジー防止）。

    /// `decide_alt_impersonation` の期待値を仕様から導出する参照実装（トートロジー
    /// を避けるため、本体実装をそのままコピーしない）。
    fn expected(is_keydown: bool, was_down: bool, was_impersonating: bool, engine: bool) -> bool {
        // 「新規押下時点でのみ engine を見て判定し直す。それ以外(repeat/KeyUp)は
        // was_impersonating を維持」という仕様通りの素直な場合分け。
        let is_fresh_press = is_keydown && !was_down;
        if is_fresh_press {
            engine
        } else {
            was_impersonating
        }
    }

    #[test]
    fn decide_alt_impersonation_exhaustive_16_combinations() {
        for is_keydown in [false, true] {
            for was_down in [false, true] {
                for was_impersonating in [false, true] {
                    for engine_enabled in [false, true] {
                        let (vk, next_impersonating) = decide_alt_impersonation(
                            VK_LMENU,
                            LEFT_THUMB,
                            is_keydown,
                            was_down,
                            was_impersonating,
                            engine_enabled,
                        );
                        let currently_impersonating =
                            expected(is_keydown, was_down, was_impersonating, engine_enabled);

                        // vk 翻訳は「現在なりすまし中か」に一致する。
                        assert_eq!(
                            vk == LEFT_THUMB,
                            currently_impersonating,
                            "vk 翻訳の不一致: is_keydown={is_keydown} was_down={was_down} \
                             was_impersonating={was_impersonating} engine_enabled={engine_enabled}"
                        );

                        // 以後保持する状態は、KeyDown なら currently_impersonating を
                        // そのまま、KeyUp なら必ず false（BUG-41 の直接の不変条件）。
                        let expected_next = is_keydown && currently_impersonating;
                        assert_eq!(
                            next_impersonating, expected_next,
                            "next_impersonating の不一致: is_keydown={is_keydown} was_down={was_down} \
                             was_impersonating={was_impersonating} engine_enabled={engine_enabled}"
                        );
                    }
                }
            }
        }
    }

    /// BUG-41 の不変条件そのもの: KeyUp 後は「以後保持する状態」が必ず false。
    /// これが崩れると、なりすましフラグが stuck true のまま残り、後続の無関係な
    /// Alt 押下まで誤って `modifiers.alt` を false 補正してしまう。
    #[test]
    fn keyup_always_clears_next_impersonating_regardless_of_prior_state() {
        for was_down in [false, true] {
            for was_impersonating in [false, true] {
                for engine_enabled in [false, true] {
                    let (_, next_impersonating) = decide_alt_impersonation(
                        VK_LMENU,
                        LEFT_THUMB,
                        false, // is_keydown = false (KeyUp)
                        was_down,
                        was_impersonating,
                        engine_enabled,
                    );
                    assert!(
                        !next_impersonating,
                        "KeyUp 後は next_impersonating が必ず false であるべき: \
                         was_down={was_down} was_impersonating={was_impersonating} \
                         engine_enabled={engine_enabled}"
                    );
                }
            }
        }
    }

    /// 新規押下(was_down=false の KeyDown)は常に engine_enabled と一致する
    /// なりすまし判定になる（押しっぱなし中の判定固定の起点）。
    #[test]
    fn fresh_press_always_matches_engine_enabled() {
        for was_impersonating in [false, true] {
            for engine_enabled in [false, true] {
                let (vk, next_impersonating) = decide_alt_impersonation(
                    VK_LMENU,
                    LEFT_THUMB,
                    true,  // is_keydown = true
                    false, // was_down = false (新規押下)
                    was_impersonating,
                    engine_enabled,
                );
                assert_eq!(vk == LEFT_THUMB, engine_enabled);
                assert_eq!(next_impersonating, engine_enabled);
            }
        }
    }
}
