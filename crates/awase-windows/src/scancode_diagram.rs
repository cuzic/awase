//! 「キーの入れ替え」画面のキーボード図の純粋モデル（ADR-248 決定5。ペアの核の上、段階2）。
//!
//! 図の「位置」は物理キー、中身は「いまその位置で出る機能」。入れ替えは `A⇄B`（長さ2の循環）なので、位置 P の機能は
//! P が属するペアの相手（無ければ P 自身）。egui などの画面部品は含まず、`awase-settings` が描画だけを行う。
//!
//! 位置の状態（[`PositionState`]）と、ドロップの可否・結果（[`drop_function`]）は、書き込みの検査（`compute_swap_write`）と
//! 同じ関数で決める（判定と表示がずれないように）。

use crate::scancode_editor::is_jis_only;
use crate::scancode_map::{SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL};
use crate::scancode_pairs::{
    compute_swap_write, Detected, Entry, Pair, SwapError, ALLOWED_SCANCODES,
    SCANCODE_HANKAKU_ZENKAKU, SCANCODE_HENKAN, SCANCODE_KANA, SCANCODE_LEFT_ALT, SCANCODE_MUHENKAN,
    SCANCODE_RIGHT_ALT, SCANCODE_SPACE,
};

/// 位置が動かせない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// 他のツールが、このキーを from か to にして設定している。
    OtherToolSetting,
    /// 「Caps を Ctrl としても使う」がオンで、英数 / Caps か左 Ctrl。
    CapsExtraOn,
    /// awase の入れ替えに、他のツールのエントリが重なっている（警告つきのペア）。
    OtherToolOverlap,
    /// 許可リスト外のキーを含む入れ替え（図の下の一覧からも解除できる）。
    HasUnlistedKey,
    /// US 配列に物理キーが無い JIS 専用キー（過去に JIS 配列で書かれた入れ替え）。
    NoPhysicalKey,
}

impl LockReason {
    /// 利用者向けの理由の文。
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::OtherToolSetting => {
                "他のツールが設定しています。変えるときはそのツールで変更してください。"
            }
            Self::CapsExtraOn => {
                "「Caps を Ctrl としても使う」がオンです。オフにすると動かせます。"
            }
            Self::OtherToolOverlap => {
                "他のツールの設定と重なっているため動かせません。「戻す」で解除できます。"
            }
            Self::HasUnlistedKey => {
                "この画面で選べないキーを含む入れ替えです。「戻す」で解除できます。"
            }
            Self::NoPhysicalKey => "この配列に物理キーが無い入れ替えです。「戻す」で解除できます。",
        }
    }
}

/// 位置の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionState {
    /// ドラッグ元にもドロップ先にもできる。
    Editable,
    /// 動かせない。`releasable` なら、その入れ替えを「戻す」ことだけはできる（削除は常に通る）。
    Locked {
        reason: LockReason,
        releasable: bool,
    },
}

/// 図の状態を決めるための入力。
#[derive(Debug, Clone, Copy)]
pub struct DiagramContext<'a> {
    /// いまの編集内容のペア（`EditorState::pairs`）。
    pub pairs: &'a [Pair],
    /// 読み込み時の分類（他ツールのエントリ・警告つきペアの判定に使う）。
    pub detected: &'a Detected,
    /// 「Caps を Ctrl としても使う」のチェック。
    pub caps_extra: bool,
    /// JIS 配列か。
    pub jis: bool,
}

fn pair_containing(pairs: &[Pair], pos: u16) -> Option<Pair> {
    pairs.iter().copied().find(|p| {
        let (a, b) = p.keys();
        a == pos || b == pos
    })
}

/// 位置 `pos` で、いま出る機能（入れ替えのペアの相手。無ければ自分自身）。
#[must_use]
pub fn function_at(pairs: &[Pair], pos: u16) -> u16 {
    pair_containing(pairs, pos).map_or(pos, |p| {
        let (a, b) = p.keys();
        if a == pos {
            b
        } else {
            a
        }
    })
}

/// 図に描く位置を、物理配置に近い行の並びで返す。許可リスト ∩ 配列に、いまの入れ替えに含まれる許可リスト内の位置を足す
/// （何も入れ替えていない JIS 専用キーだけを US 配列で隠す）。
#[must_use]
pub fn diagram_rows(pairs: &[Pair], jis: bool) -> Vec<Vec<u16>> {
    let shown = |k: u16| {
        ALLOWED_SCANCODES.contains(&k)
            && (jis || !is_jis_only(k) || pair_containing(pairs, k).is_some())
    };
    let layout: [&[u16]; 3] = [
        &[SCANCODE_HANKAKU_ZENKAKU],
        &[SCANCODE_CAPS_EISU],
        &[
            SCANCODE_LEFT_CTRL,
            SCANCODE_LEFT_ALT,
            SCANCODE_MUHENKAN,
            SCANCODE_SPACE,
            SCANCODE_HENKAN,
            SCANCODE_KANA,
            SCANCODE_RIGHT_ALT,
        ],
    ];
    layout
        .iter()
        .map(|row| {
            row.iter()
                .copied()
                .filter(|&k| shown(k))
                .collect::<Vec<u16>>()
        })
        .filter(|row| !row.is_empty())
        .collect()
}

/// 許可リスト外のキーを含むペア（図に描けないので、図の下の一覧に出して「解除」を付ける）。
#[must_use]
pub fn unlisted_pairs(pairs: &[Pair]) -> Vec<Pair> {
    pairs
        .iter()
        .copied()
        .filter(|p| {
            let (a, b) = p.keys();
            !ALLOWED_SCANCODES.contains(&a) || !ALLOWED_SCANCODES.contains(&b)
        })
        .collect()
}

/// 位置 `pos` の状態。
#[must_use]
pub fn position_state(ctx: &DiagramContext<'_>, pos: u16) -> PositionState {
    let locked = |reason, releasable| PositionState::Locked { reason, releasable };
    if ctx.caps_extra && (pos == SCANCODE_CAPS_EISU || pos == SCANCODE_LEFT_CTRL) {
        return locked(LockReason::CapsExtraOn, false);
    }
    if let Some(pair) = pair_containing(ctx.pairs, pos) {
        let (a, b) = pair.keys();
        let warned = ctx
            .detected
            .pairs
            .iter()
            .any(|d| d.pair == pair && d.warning);
        if warned {
            return locked(LockReason::OtherToolOverlap, true);
        }
        if !ALLOWED_SCANCODES.contains(&a) || !ALLOWED_SCANCODES.contains(&b) {
            return locked(LockReason::HasUnlistedKey, true);
        }
        if !ctx.jis && (is_jis_only(a) || is_jis_only(b)) {
            return locked(LockReason::NoPhysicalKey, true);
        }
        return PositionState::Editable;
    }
    // 自分の入れ替えに属さない位置: 他のツールが from か to にしているなら動かせない（解除する手段は awase に無い）。
    let foreign = ctx
        .detected
        .unclaimed
        .iter()
        .any(|&((from, to), _)| from == pos || (to != 0 && to == pos));
    if foreign {
        return locked(LockReason::OtherToolSetting, false);
    }
    PositionState::Editable
}

/// ドロップできなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropRefusal {
    /// ドラッグ元の位置が動かせない。
    SourceLocked(LockReason),
    /// ドロップ先の位置が動かせない。
    TargetLocked(LockReason),
    /// 書き込みの検査（許可リスト・Caps 追加 Ctrl・他ツールの to との衝突）に落ちた。
    Invalid(SwapError),
}

/// ドロップの結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropEffect {
    /// ドロップ後のペアの集合。
    pub pairs: Vec<Pair>,
    /// ドロップした2つの位置以外で、ドロップにより元に戻る位置（解いたペアの相手）。
    pub also_restored: Vec<u16>,
}

/// 機能 `function` を位置 `pos` へドロップしたときの結果を決める（ペアの核の間のドロップの意味）。
///
/// `pos` または `function` を含む既存のペアを解いてから、`pos ⇄ function` を作る。解く部分と作る部分は1回の
/// `compute_swap_write` で検査し、作る部分が落ちたら何も変えない。`function == pos`（元の位置へ戻す）は、その位置を含むペアを
/// 解くだけで、書き込みの検査に常に通る（削除）。
///
/// # Errors
/// [`DropRefusal`] を参照。
pub fn drop_function(
    ctx: &DiagramContext<'_>,
    existing: &[Entry],
    function: u16,
    pos: u16,
) -> Result<DropEffect, DropRefusal> {
    // 機能 F を今出している位置 s（入れ替え済みならペアの相手、そうでなければ F 自身）。
    let source = function_at(ctx.pairs, function);
    if let PositionState::Locked { reason, .. } = position_state(ctx, source) {
        return Err(DropRefusal::SourceLocked(reason));
    }
    if let PositionState::Locked { reason, .. } = position_state(ctx, pos) {
        return Err(DropRefusal::TargetLocked(reason));
    }
    let mut dissolved: Vec<u16> = Vec::new();
    let mut pairs: Vec<Pair> = Vec::new();
    for &p in ctx.pairs {
        let (a, b) = p.keys();
        if a == pos || b == pos || a == function || b == function {
            dissolved.extend([a, b]);
        } else {
            pairs.push(p);
        }
    }
    if function != pos {
        pairs.push(Pair::new(pos, function));
    }
    compute_swap_write(existing, &pairs, ctx.caps_extra).map_err(DropRefusal::Invalid)?;
    let mut also_restored: Vec<u16> = dissolved
        .into_iter()
        .filter(|&k| k != pos && k != function)
        .collect();
    also_restored.sort_unstable();
    also_restored.dedup();
    Ok(DropEffect {
        pairs,
        also_restored,
    })
}

/// ドロップできなかった理由の利用者向けの文。
#[must_use]
pub fn refusal_text(refusal: DropRefusal) -> String {
    match refusal {
        DropRefusal::SourceLocked(reason) | DropRefusal::TargetLocked(reason) => {
            reason.text().to_string()
        }
        DropRefusal::Invalid(error) => crate::scancode_editor::swap_error_text(error),
    }
}

/// ドラッグ中に、ドロップ先の下へ出す結果の説明（ドロップ後の状態は変えない）。
#[must_use]
pub fn drop_hint(
    result: &Result<DropEffect, DropRefusal>,
    label: impl Fn(u16) -> String,
) -> String {
    match result {
        Ok(effect) if effect.also_restored.is_empty() => {
            "ここへ落とすと、2つのキーの位置が入れ替わります。".to_string()
        }
        Ok(effect) => {
            let names: Vec<String> = effect.also_restored.iter().map(|&k| label(k)).collect();
            format!(
                "ここへ落とすと、2つのキーの位置が入れ替わります（「{}」の位置も元に戻ります）。",
                names.join("」「")
            )
        }
        Err(refusal) => refusal_text(*refusal),
    }
}

/// 位置 `pos` を含むペアを解く（「この位置を戻す」）。削除なので書き込みの検査に常に通る。
#[must_use]
pub fn release_position(pairs: &[Pair], pos: u16) -> Vec<Pair> {
    pairs
        .iter()
        .copied()
        .filter(|p| {
            let (a, b) = p.keys();
            a != pos && b != pos
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scancode_pairs::detect_swap_pairs;

    const CAPS: u16 = SCANCODE_CAPS_EISU;
    const LCTRL: u16 = SCANCODE_LEFT_CTRL;
    const LALT: u16 = SCANCODE_LEFT_ALT;
    const RALT: u16 = SCANCODE_RIGHT_ALT;
    const SPC: u16 = SCANCODE_SPACE;
    const MUH: u16 = SCANCODE_MUHENKAN;
    const HEN: u16 = SCANCODE_HENKAN;
    const KANA: u16 = SCANCODE_KANA;

    fn ctx_for<'a>(
        pairs: &'a [Pair],
        detected: &'a Detected,
        caps_extra: bool,
        jis: bool,
    ) -> DiagramContext<'a> {
        DiagramContext {
            pairs,
            detected,
            caps_extra,
            jis,
        }
    }

    fn entries_of(pairs: &[Pair]) -> Vec<Entry> {
        pairs.iter().flat_map(|p| p.entries()).collect()
    }

    #[test]
    fn function_at_returns_the_partner_or_itself() {
        let pairs = [Pair::new(CAPS, LCTRL)];
        assert_eq!(function_at(&pairs, CAPS), LCTRL);
        assert_eq!(function_at(&pairs, LCTRL), CAPS);
        assert_eq!(function_at(&pairs, SPC), SPC);
    }

    #[test]
    fn rows_hide_jis_only_keys_on_us_unless_they_are_in_a_pair() {
        let flat = |rows: Vec<Vec<u16>>| rows.into_iter().flatten().collect::<Vec<u16>>();
        let jis = flat(diagram_rows(&[], true));
        assert!(jis.contains(&MUH) && jis.contains(&HEN) && jis.contains(&KANA));
        assert!(jis.contains(&SCANCODE_HANKAKU_ZENKAKU));
        let us = flat(diagram_rows(&[], false));
        for k in [MUH, HEN, KANA, SCANCODE_HANKAKU_ZENKAKU] {
            assert!(!us.contains(&k), "{k:04X}");
        }
        assert!(us.contains(&SPC) && us.contains(&CAPS) && us.contains(&RALT));
        // 過去に JIS 配列で書かれた入れ替えの位置は、US でも描いて解除できるようにする。
        let pairs = [Pair::new(MUH, LALT)];
        assert!(flat(diagram_rows(&pairs, false)).contains(&MUH));
    }

    #[test]
    fn a_plain_drop_creates_a_pair_and_a_drop_on_home_releases() {
        let detected = detect_swap_pairs(&[]);
        let ctx = ctx_for(&[], &detected, false, true);
        let effect = drop_function(&ctx, &[], LCTRL, CAPS).unwrap();
        assert_eq!(effect.pairs, vec![Pair::new(CAPS, LCTRL)]);
        assert!(effect.also_restored.is_empty());

        let existing = entries_of(&effect.pairs);
        let detected = detect_swap_pairs(&existing);
        let ctx = ctx_for(&effect.pairs, &detected, false, true);
        // 位置 CAPS に出ている機能(左 Ctrl)を、左 Ctrl の位置へ戻す。
        let back = drop_function(&ctx, &existing, LCTRL, LCTRL).unwrap();
        assert!(back.pairs.is_empty());
    }

    #[test]
    fn dropping_onto_a_swapped_position_dissolves_the_old_pair_and_reports_it() {
        // CAPS⇄LCTRL がある状態で、スペースの機能を CAPS の位置へ落とす。
        let pairs = [Pair::new(CAPS, LCTRL)];
        let existing = entries_of(&pairs);
        let detected = detect_swap_pairs(&existing);
        let ctx = ctx_for(&pairs, &detected, false, true);
        let effect = drop_function(&ctx, &existing, SPC, CAPS).unwrap();
        assert_eq!(effect.pairs, vec![Pair::new(CAPS, SPC)]);
        // 左 Ctrl の位置は、触っていないのに元に戻る。
        assert_eq!(effect.also_restored, vec![LCTRL]);
    }

    #[test]
    fn dropping_a_swapped_function_elsewhere_restores_its_old_partner() {
        // CAPS⇄LCTRL。位置 CAPS に出ている機能(左 Ctrl)を、スペースの位置へ落とす。
        let pairs = [Pair::new(CAPS, LCTRL)];
        let existing = entries_of(&pairs);
        let detected = detect_swap_pairs(&existing);
        let ctx = ctx_for(&pairs, &detected, false, true);
        let effect = drop_function(&ctx, &existing, LCTRL, SPC).unwrap();
        assert_eq!(effect.pairs, vec![Pair::new(SPC, LCTRL)]);
        assert_eq!(effect.also_restored, vec![CAPS]);
    }

    #[test]
    fn a_failing_create_part_changes_nothing() {
        // 他ツールの `F1→スペース` がある。スペースを動かすと `X→スペース` の多対一になるので、ドロップは位置ごと動かせない。
        let existing = [(0x003B, SPC)];
        let detected = detect_swap_pairs(&existing);
        let ctx = ctx_for(&[], &detected, false, true);
        assert_eq!(
            position_state(&ctx, SPC),
            PositionState::Locked {
                reason: LockReason::OtherToolSetting,
                releasable: false
            }
        );
        assert!(matches!(
            drop_function(&ctx, &existing, SPC, HEN),
            Err(DropRefusal::SourceLocked(LockReason::OtherToolSetting))
        ));
        assert!(matches!(
            drop_function(&ctx, &existing, HEN, SPC),
            Err(DropRefusal::TargetLocked(LockReason::OtherToolSetting))
        ));
    }

    #[test]
    fn unlisted_keys_are_refused_by_the_write_check() {
        // 許可リスト外のキーを含むペアは作れない。ドロップの元・先が許可リスト外でも、検査（NotAllowed）で落ちる。
        let detected = detect_swap_pairs(&[]);
        let ctx = ctx_for(&[], &detected, false, true);
        assert!(matches!(
            drop_function(&ctx, &[], 0x0010, SPC),
            Err(DropRefusal::Invalid(SwapError::NotAllowed { key: 0x0010 }))
        ));
    }

    #[test]
    fn hint_and_refusal_texts_are_never_empty() {
        let detected = detect_swap_pairs(&[]);
        let ctx = ctx_for(&[], &detected, false, true);
        let ok = drop_function(&ctx, &[], LCTRL, CAPS);
        assert!(drop_hint(&ok, |k| format!("{k}")).contains("入れ替わります"));
        let pairs = [Pair::new(CAPS, LCTRL)];
        let existing = entries_of(&pairs);
        let detected = detect_swap_pairs(&existing);
        let ctx = ctx_for(&pairs, &detected, false, true);
        let restored = drop_function(&ctx, &existing, SPC, CAPS);
        assert!(drop_hint(&restored, |k| format!("<{k}>")).contains("元に戻ります"));
        let bad = drop_function(&ctx, &existing, 0x0010, SPC);
        assert!(!drop_hint(&bad, |k| format!("{k}")).is_empty());
        for reason in [
            LockReason::OtherToolSetting,
            LockReason::CapsExtraOn,
            LockReason::OtherToolOverlap,
            LockReason::HasUnlistedKey,
            LockReason::NoPhysicalKey,
        ] {
            assert!(!refusal_text(DropRefusal::TargetLocked(reason)).is_empty());
        }
    }

    #[test]
    fn caps_extra_locks_the_caps_and_ctrl_positions() {
        let detected = detect_swap_pairs(&[(CAPS, LCTRL)]);
        let ctx = ctx_for(&[], &detected, true, true);
        for k in [CAPS, LCTRL] {
            assert_eq!(
                position_state(&ctx, k),
                PositionState::Locked {
                    reason: LockReason::CapsExtraOn,
                    releasable: false
                }
            );
        }
        assert_eq!(position_state(&ctx, SPC), PositionState::Editable);
    }

    #[test]
    fn warned_pairs_lock_both_positions_but_stay_releasable() {
        // 自分のペア MUH⇄LALT に、他ツールの `F1→MUH`（to が重なる）がある。
        let existing = [(MUH, LALT), (LALT, MUH), (0x003B, MUH)];
        let detected = detect_swap_pairs(&existing);
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        assert!(detected.pairs.iter().any(|d| d.warning));
        let ctx = ctx_for(&pairs, &detected, false, true);
        for k in [MUH, LALT] {
            assert_eq!(
                position_state(&ctx, k),
                PositionState::Locked {
                    reason: LockReason::OtherToolOverlap,
                    releasable: true
                },
                "{k:04X}"
            );
        }
        // 戻す(解除)は常に通る。
        assert!(release_position(&pairs, MUH).is_empty());
        // ドラッグ元・先には使えない。
        assert!(matches!(
            drop_function(&ctx, &existing, MUH, SPC),
            Err(DropRefusal::SourceLocked(_))
        ));
        assert!(matches!(
            drop_function(&ctx, &existing, SPC, LALT),
            Err(DropRefusal::TargetLocked(_))
        ));
    }

    #[test]
    fn unlisted_pairs_are_listed_and_their_listed_position_is_locked_releasable() {
        let existing = [(MUH, 0x0010), (0x0010, MUH)];
        let detected = detect_swap_pairs(&existing);
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        assert_eq!(unlisted_pairs(&pairs), pairs);
        let ctx = ctx_for(&pairs, &detected, false, true);
        assert_eq!(
            position_state(&ctx, MUH),
            PositionState::Locked {
                reason: LockReason::HasUnlistedKey,
                releasable: true
            }
        );
    }

    #[test]
    fn jis_only_pair_on_us_is_release_only() {
        let existing = [(MUH, LALT), (LALT, MUH)];
        let detected = detect_swap_pairs(&existing);
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        let ctx = ctx_for(&pairs, &detected, false, false);
        assert_eq!(
            position_state(&ctx, MUH),
            PositionState::Locked {
                reason: LockReason::NoPhysicalKey,
                releasable: true
            }
        );
        assert_eq!(
            position_state(&ctx, LALT),
            PositionState::Locked {
                reason: LockReason::NoPhysicalKey,
                releasable: true
            }
        );
    }

    #[test]
    fn small_universe_every_drop_keeps_a_valid_involution_and_releases_always_pass() {
        // 許可リストの全位置・全機能の組で、ドロップが Ok なら結果が書き込みの検査に通り、ペアの集合が互いに素(involution)で、
        // 「解除」は常に通る。
        let starts: Vec<Vec<Pair>> = vec![
            vec![],
            vec![Pair::new(CAPS, LCTRL)],
            vec![Pair::new(MUH, LALT), Pair::new(HEN, SPC)],
        ];
        for start in &starts {
            let existing = entries_of(start);
            let detected = detect_swap_pairs(&existing);
            let ctx = ctx_for(start, &detected, false, true);
            for &f in ALLOWED_SCANCODES {
                for &p in ALLOWED_SCANCODES {
                    if let Ok(effect) = drop_function(&ctx, &existing, f, p) {
                        compute_swap_write(&existing, &effect.pairs, false).unwrap();
                        let mut keys: Vec<u16> = effect
                            .pairs
                            .iter()
                            .flat_map(|q| [q.keys().0, q.keys().1])
                            .collect();
                        let before = keys.len();
                        keys.sort_unstable();
                        keys.dedup();
                        assert_eq!(before, keys.len(), "{start:?} f={f:04X} p={p:04X}");
                        if f != p {
                            assert_eq!(function_at(&effect.pairs, p), f);
                            assert_eq!(function_at(&effect.pairs, f), p);
                        }
                    }
                }
            }
            for &k in ALLOWED_SCANCODES {
                let released = release_position(start, k);
                compute_swap_write(&existing, &released, false).unwrap();
            }
        }
    }
}
