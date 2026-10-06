//! Scancode Map を「全単射の入れ替えペア `A⇄B`」の集合として編集するための純粋ロジック
//! （ADR-230）。レジストリ I/O・昇格・UI は含まない（`scancode_map`・`awase-settings` の責務）。
//!
//! 方針（ADR-230 決定1〜4）:
//! - 編集の単位は互いに素なペア。ペア `(A,B)` はレジストリ上 `A→B` と `B→A` の2エントリ。
//! - 所有の記録を持たない。レジストリのエントリ列を唯一の真実とし、[`detect_swap_pairs`] が
//!   「編集できるペア」と「他ツールのエントリ（読み取り専用）」に全件を分類する。
//! - 書き込み [`compute_swap_write`] は、読んだ状態との差分で全エントリ列を作る。検査は
//!   「書き込み後の写像が書き込み前より悪くならない（from の重複・多対一を新たに作らない）」こと。
//!   そのため**削除だけの操作は常に通る**。
//!
//! エントリは `(from, to)` の順（`scancode_map::parse_entries` と同じ）。

use crate::scancode_map::{SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL};

/// 左 Alt のスキャンコード（Set 1、非拡張）。
pub const SCANCODE_LEFT_ALT: u16 = 0x0038;
/// 右 Alt のスキャンコード（Set 1、E0 拡張）。上位バイトが E0。
pub const SCANCODE_RIGHT_ALT: u16 = 0xE038;
/// スペースのスキャンコード。
pub const SCANCODE_SPACE: u16 = 0x0039;
/// JIS「無変換」のスキャンコード。
pub const SCANCODE_MUHENKAN: u16 = 0x007B;
/// JIS「変換」のスキャンコード。
pub const SCANCODE_HENKAN: u16 = 0x0079;
/// JIS「かな（ひらがな）」のスキャンコード。
pub const SCANCODE_KANA: u16 = 0x0070;
/// JIS「半角/全角（漢字）」のスキャンコード（US 配列では `` ` `` の文字キー）。
pub const SCANCODE_HANKAKU_ZENKAKU: u16 = 0x0029;

/// 新規にペアへ使えるキー（ADR-230 決定2、初期範囲）。`u16` は E0 込みの1つの値で持つ。
/// 英数と Caps は同じ物理キー（`SCANCODE_CAPS_EISU`）の2つの呼び名なので1項目。
pub const ALLOWED_SCANCODES: &[u16] = &[
    SCANCODE_LEFT_CTRL,
    SCANCODE_LEFT_ALT,
    SCANCODE_RIGHT_ALT,
    SCANCODE_SPACE,
    SCANCODE_MUHENKAN,
    SCANCODE_HENKAN,
    SCANCODE_KANA,
    SCANCODE_HANKAKU_ZENKAKU,
    SCANCODE_CAPS_EISU,
];

/// レジストリの1エントリ `(from, to)`。
pub type Entry = (u16, u16);

/// 入れ替えペア。順序を持たない（`new(a,b) == new(b,a)`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pair {
    lo: u16,
    hi: u16,
}

impl Pair {
    /// 2つのスキャンコードからペアを作る（`a == b` の自己ペアも作れるが、書き込みは拒否される）。
    #[must_use]
    pub const fn new(a: u16, b: u16) -> Self {
        if a <= b {
            Self { lo: a, hi: b }
        } else {
            Self { lo: b, hi: a }
        }
    }

    /// ペアの2つのキー（小さい方が先）。
    #[must_use]
    pub const fn keys(self) -> (u16, u16) {
        (self.lo, self.hi)
    }

    /// このペアが書き込む2エントリ（`lo→hi`, `hi→lo`）。
    #[must_use]
    pub const fn entries(self) -> [Entry; 2] {
        [(self.lo, self.hi), (self.hi, self.lo)]
    }

    const fn contains(self, key: u16) -> bool {
        self.lo == key || self.hi == key
    }
}

/// [`detect_swap_pairs`] が読み取ったペア。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectedPair {
    pub pair: Pair,
    /// 他のエントリがこのペアのキーと交わっている（from の重複、または to が衝突）。
    /// 編集・削除はできるが、UI は警告を出す（ADR-230 決定3）。
    pub warning: bool,
}

/// ペアとして読めなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnclaimedReason {
    /// `A→0x0000`（キーの無効化）。
    Disabled,
    /// `A→A`（恒等）。
    Identity,
    /// 同じ from のエントリが複数ある（ペア側ではなくこのエントリを未解釈として扱う）。
    DuplicateFrom,
    /// 上記以外（片方向だけ、3巡回、既にペアで使われているキーとの重なりなど）。
    NotPair,
}

/// [`detect_swap_pairs`] の分類結果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Detected {
    pub pairs: Vec<DetectedPair>,
    /// `Caps/英数 → 左 Ctrl` の片方向だけがある（ADR-126 のプリセット）。
    pub caps_extra_ctrl: bool,
    /// ペアでもプリセットでもないエントリ（他ツールのもの。保持され、読み取り専用で表示する）。
    pub unclaimed: Vec<(Entry, UnclaimedReason)>,
}

/// エントリ列を、編集できるペア・Caps 追加 Ctrl プリセット・他ツールのエントリ（Unclaimed）に分類する
/// （ADR-230 決定3の分類表）。分類は配列にも許可リストにも依存しない。
#[must_use]
pub fn detect_swap_pairs(entries: &[Entry]) -> Detected {
    let mut used = vec![false; entries.len()];
    let mut taken_keys: Vec<u16> = Vec::new();
    let mut accepted: Vec<Pair> = Vec::new();

    // ペアは「(a,b) と (b,a) が両方ある、互いに別キー、キーが既存ペアと被らない」もの。入力順に最初のものを採る。
    for (i, &(a, b)) in entries.iter().enumerate() {
        if used[i] || a == b || b == 0 || taken_keys.contains(&a) || taken_keys.contains(&b) {
            continue;
        }
        let Some(j) = entries
            .iter()
            .enumerate()
            .position(|(j, &e)| j != i && !used[j] && e == (b, a))
        else {
            continue;
        };
        used[i] = true;
        used[j] = true;
        taken_keys.push(a);
        taken_keys.push(b);
        accepted.push(Pair::new(a, b));
    }

    // Caps 追加 Ctrl: `Caps→左 Ctrl` が単独（逆向きがペアに取られていない）で残っている。
    let caps_pos = entries
        .iter()
        .enumerate()
        .position(|(i, &e)| !used[i] && e == (SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL));
    if let Some(i) = caps_pos {
        used[i] = true;
    }
    let caps_extra_ctrl = caps_pos.is_some();

    // 残りは Unclaimed。from の重複は理由を分ける。
    let mut unclaimed = Vec::new();
    for (i, &e) in entries.iter().enumerate() {
        if used[i] {
            continue;
        }
        let (from, to) = e;
        let duplicated_from = entries
            .iter()
            .enumerate()
            .any(|(j, &(f, _))| j != i && f == from);
        let reason = if to == 0 {
            UnclaimedReason::Disabled
        } else if from == to {
            UnclaimedReason::Identity
        } else if duplicated_from {
            UnclaimedReason::DuplicateFrom
        } else {
            UnclaimedReason::NotPair
        };
        unclaimed.push((e, reason));
    }

    // 警告: ペアの2エントリ以外で、ペアのキーを from または to に持つエントリがある。
    let pairs = accepted
        .into_iter()
        .map(|pair| {
            let own = pair.entries();
            let warning = entries
                .iter()
                .any(|&e| !own.contains(&e) && (pair.contains(e.0) || pair.contains(e.1)));
            DetectedPair { pair, warning }
        })
        .collect();

    Detected {
        pairs,
        caps_extra_ctrl,
        unclaimed,
    }
}

/// [`compute_swap_write`] が書き込めない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    /// `A⇄A`。
    SelfPair { key: u16 },
    /// 同じキーが複数のペアに現れる。
    DuplicateKey { key: u16 },
    /// 新規に作るペアのキーが許可リストに無い（既存のペアの保持には適用しない）。
    NotAllowed { key: u16 },
    /// Caps 追加 Ctrl のとき、`Caps/英数` か左 Ctrl を含むペアは作れない（Caps と相手が両方 Ctrl を出す多対一になる）。
    CapsExtraConflict { key: u16 },
    /// 新規に作るエントリのキーが、保持する他ツールのエントリの to に現れる（多対一を新たに作る）。
    CollidesWithForeignTarget { entry: Entry },
}

/// [`compute_swap_write`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePlan {
    /// レジストリへ書く全エントリ列。出力順は「既存の保持分を既存の順、新規ペアを末尾・入力順、
    /// 新規の Caps 追加 Ctrl をその後」に固定する（読み戻し検証が順序込みの一致比較のため）。
    pub entries: Vec<Entry>,
    /// 上書きで消える他ツールのエントリ。UI が事前に確認する（ADR-230 決定3）。
    pub displaced: Vec<Entry>,
}

/// 編集後のペア集合 `pairs`（差分ではなく全体）と Caps 追加 Ctrl の有無から、書き込む全エントリ列を作る。
///
/// 内部で [`detect_swap_pairs`] を呼び、既存のペアのうち `pairs` に無いものを削除し、新しいペアを追加する。
/// 他ツールのエントリ（Unclaimed）は保持する。検査は書き込み前より悪くならないこと
/// （from の重複・多対一を新たに作らない）なので、削除だけの操作は常に成功する。
///
/// # Errors
/// [`SwapError`] を参照。
pub fn compute_swap_write(
    existing: &[Entry],
    pairs: &[Pair],
    caps_extra_ctrl: bool,
) -> Result<WritePlan, SwapError> {
    // 1. `pairs` 自体の検査（キー単位）。
    let mut seen: Vec<u16> = Vec::new();
    for pair in pairs {
        let (a, b) = pair.keys();
        if a == b {
            return Err(SwapError::SelfPair { key: a });
        }
        for key in [a, b] {
            if seen.contains(&key) {
                return Err(SwapError::DuplicateKey { key });
            }
            seen.push(key);
        }
    }

    let detected = detect_swap_pairs(existing);
    let existing_pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
    let new_pairs: Vec<Pair> = pairs
        .iter()
        .copied()
        .filter(|p| !existing_pairs.contains(p))
        .collect();
    let adds_caps = caps_extra_ctrl && !detected.caps_extra_ctrl;

    // 2. 新規に作るものだけに、許可リストと Caps 追加 Ctrl との衝突を適用する。
    for pair in &new_pairs {
        for key in [pair.keys().0, pair.keys().1] {
            if !ALLOWED_SCANCODES.contains(&key) {
                return Err(SwapError::NotAllowed { key });
            }
        }
    }
    // Caps 追加 Ctrl を新たに足すときは全ペアを、既に有効なときは新規ペアだけを検査する
    // （既存の状態が既に同居していても、何も変えない操作や削除は通す）。
    if caps_extra_ctrl {
        let checked: &[Pair] = if adds_caps { pairs } else { &new_pairs };
        for pair in checked {
            for key in [pair.keys().0, pair.keys().1] {
                if key == SCANCODE_CAPS_EISU || key == SCANCODE_LEFT_CTRL {
                    return Err(SwapError::CapsExtraConflict { key });
                }
            }
        }
    }

    // 3. 新規に作るエントリ。
    let mut added: Vec<Entry> = new_pairs.iter().flat_map(|p| p.entries()).collect();
    if adds_caps {
        added.push((SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL));
    }

    // 4. 上書きで消える他ツールのエントリ（from が新規エントリの from と同じ）。
    // Caps 追加 Ctrl を足すとき、他ツールの `左 Ctrl→Caps` が残ると組が入れ替え（ペア）に読み替わるので、これも消す。
    let added_from: Vec<u16> = added.iter().map(|e| e.0).collect();
    let displaced: Vec<Entry> = detected
        .unclaimed
        .iter()
        .map(|&(e, _)| e)
        .filter(|e| {
            added_from.contains(&e.0)
                || (adds_caps && *e == (SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU))
        })
        .collect();

    // 5. 保持する他ツールのエントリと、新規エントリの to が同じなら、2つのキーが同じキーを出す多対一を新たに作る。
    for &(entry, _) in &detected.unclaimed {
        if displaced.contains(&entry) {
            continue;
        }
        if let Some(&culprit) = added.iter().find(|&&(_, to)| to == entry.1) {
            return Err(SwapError::CollidesWithForeignTarget { entry: culprit });
        }
    }

    // 6. 出力: 既存の順のまま、削除対象（外されたペア・外された Caps 追加 Ctrl・displaced）を除き、末尾に新規を足す。
    let kept_pairs: Vec<Pair> = existing_pairs
        .iter()
        .copied()
        .filter(|p| pairs.contains(p))
        .collect();
    let mut removed: Vec<Entry> = Vec::new();
    for d in &detected.pairs {
        if !kept_pairs.contains(&d.pair) {
            removed.extend(d.pair.entries());
        }
    }
    if detected.caps_extra_ctrl && !caps_extra_ctrl {
        removed.push((SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL));
    }
    removed.extend(displaced.iter().copied());

    // 同じ写像の複製（完全に同じエントリ）は、1つを消すなら全部消す（片方だけ残ると意図しない設定が残るため）。
    let mut out: Vec<Entry> = existing
        .iter()
        .copied()
        .filter(|e| !removed.contains(e))
        .collect();
    out.extend(added);
    Ok(WritePlan {
        entries: out,
        displaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: u16 = SCANCODE_CAPS_EISU;
    const LCTRL: u16 = SCANCODE_LEFT_CTRL;
    const LALT: u16 = SCANCODE_LEFT_ALT;
    const MUH: u16 = SCANCODE_MUHENKAN;
    const HEN: u16 = SCANCODE_HENKAN;
    const SPC: u16 = SCANCODE_SPACE;

    fn pair(a: u16, b: u16) -> Pair {
        Pair::new(a, b)
    }

    /// ペア `a⇄b` が書き込む2エントリ（`Pair::entries` の順）。
    fn pe(a: u16, b: u16) -> Vec<Entry> {
        Pair::new(a, b).entries().to_vec()
    }

    // ---- detect_swap_pairs: ADR-230 決定3の分類表 ----

    #[test]
    fn complete_swap_is_a_pair() {
        let d = detect_swap_pairs(&[(CAPS, LCTRL), (LCTRL, CAPS)]);
        assert_eq!(
            d.pairs,
            vec![DetectedPair {
                pair: pair(CAPS, LCTRL),
                warning: false
            }]
        );
        assert!(!d.caps_extra_ctrl);
        assert!(d.unclaimed.is_empty());
    }

    #[test]
    fn pair_outside_allowlist_is_still_a_pair() {
        let d = detect_swap_pairs(&[(0x0010, 0x0011), (0x0011, 0x0010)]);
        assert_eq!(d.pairs.len(), 1);
        assert!(d.unclaimed.is_empty());
    }

    #[test]
    fn caps_to_ctrl_alone_is_caps_extra_ctrl() {
        let d = detect_swap_pairs(&[(CAPS, LCTRL)]);
        assert!(d.caps_extra_ctrl);
        assert!(d.pairs.is_empty());
        assert!(d.unclaimed.is_empty());
    }

    #[test]
    fn one_way_entry_other_than_caps_is_unclaimed() {
        let d = detect_swap_pairs(&[(MUH, LALT)]);
        assert_eq!(d.unclaimed, vec![((MUH, LALT), UnclaimedReason::NotPair)]);
    }

    #[test]
    fn pair_crossed_by_a_third_entry_is_a_warned_pair_and_the_third_is_unclaimed() {
        // A→B, B→A, C→A
        let d = detect_swap_pairs(&[(MUH, LALT), (LALT, MUH), (HEN, MUH)]);
        assert_eq!(
            d.pairs,
            vec![DetectedPair {
                pair: pair(MUH, LALT),
                warning: true
            }]
        );
        assert_eq!(d.unclaimed, vec![((HEN, MUH), UnclaimedReason::NotPair)]);
    }

    #[test]
    fn three_cycle_is_all_unclaimed() {
        let d = detect_swap_pairs(&[(MUH, HEN), (HEN, SPC), (SPC, MUH)]);
        assert!(d.pairs.is_empty());
        assert_eq!(d.unclaimed.len(), 3);
    }

    #[test]
    fn pair_with_extra_entry_on_same_from_is_warned_and_extra_is_duplicate_from() {
        // A→B, B→A に A→Z が追記された（ADR-230 S13）。
        let d = detect_swap_pairs(&[(MUH, LALT), (LALT, MUH), (MUH, SPC)]);
        assert_eq!(
            d.pairs,
            vec![DetectedPair {
                pair: pair(MUH, LALT),
                warning: true
            }]
        );
        assert_eq!(
            d.unclaimed,
            vec![((MUH, SPC), UnclaimedReason::DuplicateFrom)]
        );
    }

    #[test]
    fn duplicate_from_without_a_pair_marks_only_those_entries() {
        let d = detect_swap_pairs(&[(MUH, LALT), (MUH, SPC), (HEN, 0x0030), (0x0030, HEN)]);
        assert_eq!(d.pairs.len(), 1);
        assert_eq!(
            d.unclaimed,
            vec![
                ((MUH, LALT), UnclaimedReason::DuplicateFrom),
                ((MUH, SPC), UnclaimedReason::DuplicateFrom),
            ]
        );
    }

    #[test]
    fn disabled_and_identity_entries_are_unclaimed() {
        let d = detect_swap_pairs(&[(MUH, 0x0000), (HEN, HEN)]);
        assert_eq!(
            d.unclaimed,
            vec![
                ((MUH, 0x0000), UnclaimedReason::Disabled),
                ((HEN, HEN), UnclaimedReason::Identity),
            ]
        );
    }

    #[test]
    fn overlapping_pairs_keep_the_first_and_leave_the_rest_unclaimed() {
        // A⇄B と A⇄C は A が重なるので、最初だけがペア。
        let d = detect_swap_pairs(&[(MUH, LALT), (LALT, MUH), (MUH, SPC), (SPC, MUH)]);
        assert_eq!(d.pairs.len(), 1);
        assert_eq!(d.unclaimed.len(), 2);
    }

    #[test]
    fn swap_plus_unrelated_third_party_is_kept_as_unclaimed() {
        // 既存テスト `third_party_left_ctrl_remap...` に当たる構成: Caps→LCtrl 単独と他ツールの 0x1D→0x99。
        let d = detect_swap_pairs(&[(CAPS, LCTRL), (LCTRL, 0x0099)]);
        assert!(d.caps_extra_ctrl);
        assert_eq!(
            d.unclaimed,
            vec![((LCTRL, 0x0099), UnclaimedReason::NotPair)]
        );
    }

    #[test]
    fn ambiguous_third_party_entry_colliding_with_swap_reverse_value_reads_as_pair() {
        // Caps→LCtrl（プリセット）に、値が偶然 Swap の逆向きと一致する第三者の LCtrl→Caps。従来の Swap 優先判定と同じ結果。
        let d = detect_swap_pairs(&[(CAPS, LCTRL), (LCTRL, CAPS)]);
        assert_eq!(d.pairs.len(), 1);
        assert!(!d.caps_extra_ctrl);
    }

    // ---- compute_swap_write ----

    #[test]
    fn adds_a_new_pair_at_the_end_in_input_order() {
        let existing = vec![(0x0010, 0x0011)];
        let plan = compute_swap_write(&existing, &[pair(MUH, LALT)], false).unwrap();
        let mut expected = existing;
        expected.extend(pe(MUH, LALT));
        assert_eq!(plan.entries, expected);
        assert!(plan.displaced.is_empty());
    }

    #[test]
    fn rejects_new_pair_with_key_outside_allowlist() {
        assert_eq!(
            compute_swap_write(&[], &[pair(0x0010, MUH)], false),
            Err(SwapError::NotAllowed { key: 0x0010 })
        );
    }

    #[test]
    fn keeps_existing_pair_outside_allowlist_untouched() {
        let existing = vec![(0x0010, 0x0011), (0x0011, 0x0010)];
        let plan = compute_swap_write(&existing, &[pair(0x0010, 0x0011)], false).unwrap();
        assert_eq!(plan.entries, existing);
    }

    #[test]
    fn rejects_self_pair_and_duplicate_keys() {
        assert_eq!(
            compute_swap_write(&[], &[pair(MUH, MUH)], false),
            Err(SwapError::SelfPair { key: MUH })
        );
        assert_eq!(
            compute_swap_write(&[], &[pair(MUH, HEN), pair(MUH, SPC)], false),
            Err(SwapError::DuplicateKey { key: MUH })
        );
    }

    #[test]
    fn removing_a_pair_deletes_exactly_its_two_entries() {
        let existing = vec![(0x0010, 0x0011), (MUH, LALT), (LALT, MUH)];
        let plan = compute_swap_write(&existing, &[], false).unwrap();
        assert_eq!(plan.entries, vec![(0x0010, 0x0011)]);
    }

    #[test]
    fn can_remove_a_pair_crossed_by_a_foreign_entry_and_the_foreign_entry_stays() {
        // 他ツールが 0x7B→0x3A のようなエントリを足していても、awase が書いた入れ替えを解除できる（現行から後退しない）。
        let existing = vec![(CAPS, LCTRL), (LCTRL, CAPS), (MUH, CAPS)];
        let plan = compute_swap_write(&existing, &[], false).unwrap();
        assert_eq!(plan.entries, vec![(MUH, CAPS)]);
    }

    #[test]
    fn can_remove_a_pair_when_a_duplicate_from_entry_was_appended() {
        // ADR-230 S13: A→B, B→A のあとに A→Z。削除は A→B, B→A の2件だけを消す。
        let existing = vec![(MUH, LALT), (LALT, MUH), (MUH, SPC)];
        let plan = compute_swap_write(&existing, &[], false).unwrap();
        assert_eq!(plan.entries, vec![(MUH, SPC)]);
    }

    #[test]
    fn new_pair_displaces_foreign_entry_with_same_from() {
        let existing = vec![(MUH, SPC)];
        let plan = compute_swap_write(&existing, &[pair(MUH, LALT)], false).unwrap();
        assert_eq!(plan.entries, pe(MUH, LALT));
        assert_eq!(plan.displaced, vec![(MUH, SPC)]);
    }

    #[test]
    fn new_pair_colliding_with_foreign_target_is_rejected() {
        // 他ツールの HEN→MUH があるところへ MUH⇄LALT を作ると、HEN と LALT が両方 MUH を出す。
        let existing = vec![(HEN, MUH)];
        assert_eq!(
            compute_swap_write(&existing, &[pair(MUH, LALT)], false),
            Err(SwapError::CollidesWithForeignTarget {
                entry: pair(MUH, LALT).entries()[0]
            })
        );
    }

    #[test]
    fn keeping_an_already_crossed_pair_is_not_an_error() {
        let existing = vec![(MUH, LALT), (LALT, MUH), (HEN, MUH)];
        let plan = compute_swap_write(&existing, &[pair(MUH, LALT)], false).unwrap();
        assert_eq!(plan.entries, existing);
    }

    #[test]
    fn swap_to_caps_extra_ctrl_is_one_write() {
        // Swap をやめて「Caps を追加の Ctrl」にするのが、1回の呼び出し（UAC 1回）で書ける。
        let existing = vec![(CAPS, LCTRL), (LCTRL, CAPS)];
        let plan = compute_swap_write(&existing, &[], true).unwrap();
        // Swap の2エントリは削除、Caps→Ctrl は新規に追加される（Caps→LCtrl 自体は旧エントリと同値だが旧エントリはペア扱い）。
        assert_eq!(plan.entries, vec![(CAPS, LCTRL)]);
    }

    #[test]
    fn caps_extra_ctrl_blocks_pairs_that_use_caps_or_left_ctrl() {
        assert_eq!(
            compute_swap_write(&[], &[pair(LCTRL, MUH)], true),
            Err(SwapError::CapsExtraConflict { key: LCTRL })
        );
        // それ以外のペアは同時に使える。
        let plan = compute_swap_write(&[], &[pair(MUH, LALT)], true).unwrap();
        let mut expected = pe(MUH, LALT);
        expected.push((CAPS, LCTRL));
        assert_eq!(plan.entries, expected);
    }

    #[test]
    fn caps_extra_ctrl_with_foreign_caps_entry_displaces_it() {
        let existing = vec![(CAPS, 0x0099)];
        let plan = compute_swap_write(&existing, &[], true).unwrap();
        assert_eq!(plan.entries, vec![(CAPS, LCTRL)]);
        assert_eq!(plan.displaced, vec![(CAPS, 0x0099)]);
    }

    #[test]
    fn caps_extra_ctrl_with_foreign_entry_targeting_left_ctrl_is_rejected() {
        let existing = vec![(MUH, LCTRL)];
        assert_eq!(
            compute_swap_write(&existing, &[], true),
            Err(SwapError::CollidesWithForeignTarget {
                entry: (CAPS, LCTRL)
            })
        );
    }

    #[test]
    fn third_party_entry_survives_caps_extra_ctrl_enable_and_disable() {
        // 既存テスト `third_party_left_ctrl_remap_to_unrelated_key_survives_caps_extra_ctrl_enable_and_disable` の引き継ぎ。
        let third = (LCTRL, 0x0099);
        let on = compute_swap_write(&[third], &[], true).unwrap();
        assert!(on.entries.contains(&third));
        assert!(on.entries.contains(&(CAPS, LCTRL)));
        let off = compute_swap_write(&on.entries, &[], false).unwrap();
        assert_eq!(off.entries, vec![third]);
    }

    // ---- 性質テスト（決定論的な疑似乱数、依存なし） ----

    struct Rng(u64);
    impl Rng {
        fn step(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.step() % n as u64).unwrap_or(0)
        }
    }

    const UNIVERSE: [u16; 7] = [CAPS, LCTRL, LALT, SPC, MUH, HEN, 0x0030];

    fn random_entries(rng: &mut Rng) -> Vec<Entry> {
        let n = rng.below(7);
        (0..n)
            .map(|_| {
                let from = UNIVERSE[rng.below(UNIVERSE.len())];
                let to = if rng.below(10) == 0 {
                    0
                } else {
                    UNIVERSE[rng.below(UNIVERSE.len())]
                };
                (from, to)
            })
            .collect()
    }

    fn random_pairs(rng: &mut Rng) -> Vec<Pair> {
        let n = rng.below(4);
        (0..n)
            .map(|_| {
                Pair::new(
                    UNIVERSE[rng.below(UNIVERSE.len())],
                    UNIVERSE[rng.below(UNIVERSE.len())],
                )
            })
            .collect()
    }

    fn dup_from(entries: &[Entry]) -> usize {
        let mut froms: Vec<u16> = entries.iter().map(|e| e.0).collect();
        froms.sort_unstable();
        froms.dedup();
        entries.len() - froms.len()
    }

    fn dup_to(entries: &[Entry]) -> usize {
        let mut tos: Vec<u16> = entries.iter().map(|e| e.1).collect();
        tos.sort_unstable();
        tos.dedup();
        entries.len() - tos.len()
    }

    #[test]
    fn property_removal_identity_round_trip_and_no_new_collisions() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut ok_count = 0;
        for _ in 0..20_000 {
            let mut existing = random_entries(&mut rng);
            existing.dedup();
            let pairs = random_pairs(&mut rng);
            let caps = rng.below(2) == 0;
            let before = detect_swap_pairs(&existing);
            let before_pairs: Vec<Pair> = before.pairs.iter().map(|d| d.pair).collect();

            // 削除だけ（既存の部分集合）は常に成功し、何も増やさない。
            let kept: Vec<Pair> = before_pairs
                .iter()
                .copied()
                .filter(|_| rng.below(2) == 0)
                .collect();
            let keep_caps = before.caps_extra_ctrl && rng.below(2) == 0;
            let removal =
                compute_swap_write(&existing, &kept, keep_caps).expect("削除だけの操作は常に通る");
            assert!(removal.entries.len() <= existing.len());
            assert!(removal.displaced.is_empty());

            // 何も変えない呼び出しは恒等。
            let same = compute_swap_write(&existing, &before_pairs, before.caps_extra_ctrl)
                .expect("恒等は常に通る");
            assert_eq!(same.entries, existing);

            let Ok(plan) = compute_swap_write(&existing, &pairs, caps) else {
                continue;
            };
            ok_count += 1;
            let after = detect_swap_pairs(&plan.entries);
            let got: Vec<Pair> = after.pairs.iter().map(|d| d.pair).collect();
            // 指定したペアはすべて読める（除去で別のペアが現れることはある）。
            for p in &pairs {
                assert!(
                    got.contains(p),
                    "existing={existing:?} pairs={pairs:?} caps={caps}"
                );
            }
            // 現れた余分なペアは、書き込み前に読み取り専用だったエントリ（かプリセットのエントリ）だけでできている。
            for p in got.iter().filter(|p| !pairs.contains(p)) {
                for e in p.entries() {
                    let was_unclaimed = before.unclaimed.iter().any(|&(u, _)| u == e);
                    let was_caps = before.caps_extra_ctrl && e == (CAPS, LCTRL);
                    assert!(was_unclaimed || was_caps, "invented pair {p:?}");
                }
            }
            // Caps 追加 Ctrl は指定どおりに読める（現れた余分なペアに吸われた場合を除く）。
            assert!(
                after.caps_extra_ctrl == caps || (caps && got.contains(&Pair::new(CAPS, LCTRL))),
                "existing={existing:?} pairs={pairs:?} caps={caps}"
            );
            // 他ツールのエントリは、上書きされたもの・消したペアやプリセットと同じ写像の複製を除いて保持される。
            for &(e, _) in &before.unclaimed {
                let same_as_removed = before_pairs.iter().any(|p| p.entries().contains(&e))
                    || (before.caps_extra_ctrl && e == (CAPS, LCTRL));
                assert!(
                    plan.displaced.contains(&e) || plan.entries.contains(&e) || same_as_removed,
                    "lost foreign entry {e:?}: existing={existing:?} pairs={pairs:?} caps={caps}"
                );
            }
            // 書き込み後の写像が書き込み前より悪くならない。
            assert!(dup_from(&plan.entries) <= dup_from(&existing));
            assert!(dup_to(&plan.entries) <= dup_to(&existing));
        }
        assert!(
            ok_count > 500,
            "成功する入力が少なすぎて性質を検査できていない: {ok_count}"
        );
    }
}
