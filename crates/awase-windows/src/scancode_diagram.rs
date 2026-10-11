//! 「キーの入れ替え」画面のキーボード図の純粋モデル（ADR-248 決定5・決定6）。
//!
//! 図の「位置」は物理キー、中身は「いまその位置で出る機能」。入れ替えは置換で、ペア（長さ2の循環）と3つ以上の循環でできている。
//! 位置 P の機能 σ(P) は、P が属する単位の「次のキー」（無ければ P 自身）。egui などの画面部品は含まず、`awase-settings` が
//! 描画だけを行う。
//!
//! 位置の状態（[`PositionState`]）と、ドロップの可否・結果（[`drop_function`]）は、書き込みの検査
//! （`compute_swap_write_with_cycles`）と同じ関数で決める（判定と表示がずれないように）。

use crate::scancode_editor::is_jis_only;
use crate::scancode_map::{SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL};
use crate::scancode_pairs::{
    compute_swap_write_with_cycles, Cycle, Detected, Entry, Pair, SwapError, ALLOWED_SCANCODES,
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
    /// Caps を Ctrl にするキーボードフィルタドライバ（Ctrl2cap など）が入っていて、設定すると二重になる。
    CapsFilterDriver,
    /// 親指キーが IME の ON/OFF キーも兼ねていて、入れ替えると IME の切り替えが働かなくなる。
    ThumbImeRole,
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
            Self::CapsFilterDriver => {
                "Caps を Ctrl にするドライバ（Ctrl2cap など）が入っているため、二重になります。ドライバをアンインストールして再起動すると設定できます。"
            }
            Self::ThumbImeRole => {
                "親指キーが IME の ON/OFF キーも兼ねているため、入れ替えられません。「キー設定」の IME ON/OFF から外すと設定できます。"
            }
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
    /// いまの編集内容の、3つ以上の循環（`EditorState::cycles`）。
    pub cycles: &'a [Cycle],
    /// 読み込み時の分類（他ツールのエントリ・警告つきペアの判定に使う）。
    pub detected: &'a Detected,
    /// 「Caps を Ctrl としても使う」のチェック。
    pub caps_extra: bool,
    /// JIS 配列か。
    pub jis: bool,
    /// Caps を Ctrl にするキーボードフィルタドライバ（Ctrl2cap など）が入っている。
    pub caps_filter: bool,
    /// IME の ON/OFF キーも兼ねている親指キーのスキャンコード。
    pub ime_role_thumbs: &'a [u16],
}

/// 編集の単位（ペアまたは3つ以上の循環）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditUnit {
    Pair(Pair),
    Cycle(Cycle),
}

impl EditUnit {
    /// この単位のキー。
    #[must_use]
    pub fn keys(&self) -> Vec<u16> {
        match self {
            Self::Pair(p) => {
                let (a, b) = p.keys();
                vec![a, b]
            }
            Self::Cycle(c) => c.keys().to_vec(),
        }
    }
}

/// ペアと循環を、編集の単位として並べる。
#[must_use]
pub fn units_of(pairs: &[Pair], cycles: &[Cycle]) -> Vec<EditUnit> {
    pairs
        .iter()
        .map(|&p| EditUnit::Pair(p))
        .chain(cycles.iter().cloned().map(EditUnit::Cycle))
        .collect()
}

fn unit_containing(pairs: &[Pair], cycles: &[Cycle], pos: u16) -> Option<EditUnit> {
    units_of(pairs, cycles)
        .into_iter()
        .find(|u| u.keys().contains(&pos))
}

/// 位置 → 機能の対応（恒等でない位置だけ）。
fn sigma(pairs: &[Pair], cycles: &[Cycle]) -> Vec<Entry> {
    let mut out: Vec<Entry> = pairs.iter().flat_map(|p| p.entries()).collect();
    out.extend(cycles.iter().flat_map(Cycle::entries));
    out
}

/// 置換を、ペア（長さ2の循環）と3つ以上の循環に分ける。入力は「位置 → 機能」の対応で、恒等の位置は含まない。
fn decompose(sig: &[Entry]) -> (Vec<Pair>, Vec<Cycle>) {
    let image = |k: u16| sig.iter().find(|e| e.0 == k).map(|e| e.1);
    let mut seen: Vec<u16> = Vec::new();
    let (mut pairs, mut cycles) = (Vec::new(), Vec::new());
    for &(start, _) in sig {
        if seen.contains(&start) {
            continue;
        }
        let mut chain = vec![start];
        let mut current = image(start);
        while let Some(next) = current {
            if next == start || chain.contains(&next) {
                break;
            }
            chain.push(next);
            current = image(next);
        }
        seen.extend(chain.iter().copied());
        match chain.len() {
            0 | 1 => {}
            2 => pairs.push(Pair::new(chain[0], chain[1])),
            _ => {
                if let Some(cycle) = Cycle::new(&chain) {
                    cycles.push(cycle);
                }
            }
        }
    }
    (pairs, cycles)
}

/// 位置 `pos` で、いま出る機能（入れ替えの単位の次のキー。無ければ自分自身）。ペアだけの版。
#[must_use]
pub fn function_at(pairs: &[Pair], pos: u16) -> u16 {
    function_at_units(pairs, &[], pos)
}

/// 位置 `pos` で、いま出る機能（ペアと循環の両方を見る）。
#[must_use]
pub fn function_at_units(pairs: &[Pair], cycles: &[Cycle], pos: u16) -> u16 {
    sigma(pairs, cycles)
        .iter()
        .find(|e| e.0 == pos)
        .map_or(pos, |e| e.1)
}

/// 機能 `function` を、いま出している位置（無ければ `function` 自身）。
fn position_of(pairs: &[Pair], cycles: &[Cycle], function: u16) -> u16 {
    sigma(pairs, cycles)
        .iter()
        .find(|e| e.1 == function)
        .map_or(function, |e| e.0)
}

/// 図に表示する、位置 `pos` で働く機能。[`function_at_units`] に加えて、「Caps を Ctrl としても使う」がオンのとき、
/// 英数 / Caps の位置が左 Ctrl として働くこと（多対一で、置換では表せない）を表す。
#[must_use]
pub fn displayed_function(pairs: &[Pair], cycles: &[Cycle], caps_extra: bool, pos: u16) -> u16 {
    if caps_extra && pos == SCANCODE_CAPS_EISU {
        SCANCODE_LEFT_CTRL
    } else {
        function_at_units(pairs, cycles, pos)
    }
}

/// 動かせない位置の下に出す短い注記。
#[must_use]
pub fn locked_note(reason: LockReason, pos: u16) -> &'static str {
    match reason {
        LockReason::CapsExtraOn if pos == SCANCODE_CAPS_EISU => "Ctrl として動作中",
        LockReason::CapsFilterDriver => "ドライバが入っています",
        LockReason::ThumbImeRole => "IME キーも兼ねています",
        _ => "動かせません",
    }
}

/// 図に描く位置を、物理配置に近い行の並びで返す。許可リスト ∩ 配列に、いまの入れ替えに含まれる許可リスト内の位置を足す
/// （何も入れ替えていない JIS 専用キーだけを US 配列で隠す）。
#[must_use]
pub fn diagram_rows(pairs: &[Pair], cycles: &[Cycle], jis: bool) -> Vec<Vec<u16>> {
    let shown = |k: u16| {
        ALLOWED_SCANCODES.contains(&k)
            && (jis || !is_jis_only(k) || unit_containing(pairs, cycles, k).is_some())
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

/// 許可リスト外のキーを含む単位（図に描けないので、図の下の一覧に出して「解除」を付ける）。
#[must_use]
pub fn unlisted_units(pairs: &[Pair], cycles: &[Cycle]) -> Vec<EditUnit> {
    units_of(pairs, cycles)
        .into_iter()
        .filter(|u| u.keys().iter().any(|k| !ALLOWED_SCANCODES.contains(k)))
        .collect()
}

/// 位置 `pos` の状態。
#[must_use]
pub fn position_state(ctx: &DiagramContext<'_>, pos: u16) -> PositionState {
    let locked = |reason, releasable| PositionState::Locked { reason, releasable };
    let own = unit_containing(ctx.pairs, ctx.cycles, pos);
    if ctx.caps_extra && (pos == SCANCODE_CAPS_EISU || pos == SCANCODE_LEFT_CTRL) {
        return locked(LockReason::CapsExtraOn, false);
    }
    if ctx.caps_filter && (pos == SCANCODE_CAPS_EISU || pos == SCANCODE_LEFT_CTRL) {
        // すでに入れ替えがあるなら「戻す」だけできる。新しくは作れない。
        return locked(LockReason::CapsFilterDriver, own.is_some());
    }
    if ctx.ime_role_thumbs.contains(&pos) {
        return locked(LockReason::ThumbImeRole, own.is_some());
    }
    if let Some(unit) = own {
        let keys = unit.keys();
        let warned = matches!(&unit, EditUnit::Pair(pair)
            if ctx.detected.pairs.iter().any(|d| d.pair == *pair && d.warning));
        if warned {
            return locked(LockReason::OtherToolOverlap, true);
        }
        if keys.iter().any(|k| !ALLOWED_SCANCODES.contains(k)) {
            return locked(LockReason::HasUnlistedKey, true);
        }
        if !ctx.jis && keys.iter().any(|&k| is_jis_only(k)) {
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
    /// ドロップ後の、3つ以上の循環の集合。
    pub cycles: Vec<Cycle>,
    /// 機能をドラッグした元の位置（その機能をいま出している位置）。
    pub source: u16,
    /// ドロップ後、ドロップ先の位置が属する単位の長さ（元に戻って恒等なら 0、ペアなら 2、循環ならその長さ）。
    pub unit_len: usize,
}

/// 機能 `function` を位置 `pos` へドロップしたときの結果を決める。
///
/// 機能 `function` をいま出している位置 `s` と `pos` の中身を入れ替える（置換に互換 `(s pos)` を掛ける:
/// `σ'(s) = σ(pos)`、`σ'(pos) = function`。変わるのは `s` と `pos` の2つの位置だけ）。結果の置換をペアと循環に分け、
/// 書き込みの検査（`compute_swap_write_with_cycles`）に通して、落ちたら何も変えない。`function == pos`（元の位置へ戻す）は、
/// `pos` を恒等に戻して、その単位から外す（循環は1つ短くなる）。`s == pos`（すでにそこ）は何も変えない。
///
/// # Errors
/// [`DropRefusal`] を参照。
pub fn drop_function(
    ctx: &DiagramContext<'_>,
    existing: &[Entry],
    function: u16,
    pos: u16,
) -> Result<DropEffect, DropRefusal> {
    let source = position_of(ctx.pairs, ctx.cycles, function);
    if let PositionState::Locked { reason, .. } = position_state(ctx, source) {
        return Err(DropRefusal::SourceLocked(reason));
    }
    if let PositionState::Locked { reason, .. } = position_state(ctx, pos) {
        return Err(DropRefusal::TargetLocked(reason));
    }
    let current = sigma(ctx.pairs, ctx.cycles);
    let image = |k: u16| current.iter().find(|e| e.0 == k).map_or(k, |e| e.1);
    let next: Vec<Entry> = if source == pos {
        // すでにそこにある（何も変えない）。
        current.clone()
    } else {
        // σ'(source) = σ(pos)、σ'(pos) = function（= σ(source)）。恒等になった位置は入れない。
        let mut n: Vec<Entry> = current
            .iter()
            .copied()
            .filter(|e| e.0 != source && e.0 != pos)
            .collect();
        let new_source = image(pos);
        if new_source != source {
            n.push((source, new_source));
        }
        if function != pos {
            n.push((pos, function));
        }
        n
    };
    let (pairs, cycles) = decompose(&next);
    compute_swap_write_with_cycles(existing, &pairs, &cycles, ctx.caps_extra)
        .map_err(DropRefusal::Invalid)?;
    let unit_len = unit_containing(&pairs, &cycles, pos).map_or(0, |u| u.keys().len());
    Ok(DropEffect {
        pairs,
        cycles,
        source,
        unit_len,
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

/// ドラッグ中に、ドロップ先の下へ出す結果の説明（ドロップ後の状態は変えない）。`pos` はドロップ先の位置。
#[must_use]
pub fn drop_hint(
    result: &Result<DropEffect, DropRefusal>,
    pos: u16,
    label: impl Fn(u16) -> String,
) -> String {
    match result {
        Ok(effect) if effect.unit_len == 0 => {
            format!("ここへ落とすと、「{}」の位置が元に戻ります。", label(pos))
        }
        Ok(effect) if effect.source == pos => "ここへ落としても、変わりません。".to_string(),
        Ok(effect) if effect.unit_len > 2 => format!(
            "ここへ落とすと、「{}」と「{}」の位置の中身が入れ替わります（{}個のキーの循環になります）。",
            label(effect.source),
            label(pos),
            effect.unit_len
        ),
        Ok(effect) => format!(
            "ここへ落とすと、「{}」と「{}」の位置の中身が入れ替わります。",
            label(effect.source),
            label(pos)
        ),
        Err(refusal) => refusal_text(*refusal),
    }
}

/// 位置 `pos` が属する単位（ペアまたは循環）を丸ごと解く（「この位置を戻す」）。削除なので書き込みの検査に常に通る。
#[must_use]
pub fn release_position(pairs: &[Pair], cycles: &[Cycle], pos: u16) -> (Vec<Pair>, Vec<Cycle>) {
    (
        pairs
            .iter()
            .copied()
            .filter(|p| {
                let (a, b) = p.keys();
                a != pos && b != pos
            })
            .collect(),
        cycles
            .iter()
            .filter(|c| !c.contains(pos))
            .cloned()
            .collect(),
    )
}

/// 単位 `unit` を解く（図の下の一覧の「解除」）。
#[must_use]
pub fn release_unit(pairs: &[Pair], cycles: &[Cycle], unit: &EditUnit) -> (Vec<Pair>, Vec<Cycle>) {
    (
        pairs
            .iter()
            .copied()
            .filter(|p| *unit != EditUnit::Pair(*p))
            .collect(),
        cycles
            .iter()
            .filter(|c| *unit != EditUnit::Cycle((*c).clone()))
            .cloned()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scancode_pairs::{compute_swap_write, detect_swap_pairs};

    const CAPS: u16 = SCANCODE_CAPS_EISU;
    const LCTRL: u16 = SCANCODE_LEFT_CTRL;
    const LALT: u16 = SCANCODE_LEFT_ALT;
    const RALT: u16 = SCANCODE_RIGHT_ALT;
    const SPC: u16 = SCANCODE_SPACE;
    const MUH: u16 = SCANCODE_MUHENKAN;
    const HEN: u16 = SCANCODE_HENKAN;
    const KANA: u16 = SCANCODE_KANA;

    fn entries_of(pairs: &[Pair], cycles: &[Cycle]) -> Vec<Entry> {
        sigma(pairs, cycles)
    }

    /// ペアと循環から、レジストリの現在値と分類を作る。
    fn registry(pairs: &[Pair], cycles: &[Cycle]) -> (Vec<Entry>, Detected) {
        let existing = entries_of(pairs, cycles);
        let detected = detect_swap_pairs(&existing);
        (existing, detected)
    }

    fn ctx<'a>(
        pairs: &'a [Pair],
        cycles: &'a [Cycle],
        detected: &'a Detected,
        caps_extra: bool,
        jis: bool,
    ) -> DiagramContext<'a> {
        DiagramContext {
            pairs,
            cycles,
            detected,
            caps_extra,
            jis,
            caps_filter: false,
            ime_role_thumbs: &[],
        }
    }

    fn cycle(keys: &[u16]) -> Cycle {
        Cycle::new(keys).unwrap()
    }

    #[test]
    fn function_at_returns_the_next_key_of_the_unit_or_itself() {
        let pairs = [Pair::new(CAPS, LCTRL)];
        assert_eq!(function_at(&pairs, CAPS), LCTRL);
        assert_eq!(function_at(&pairs, LCTRL), CAPS);
        assert_eq!(function_at(&pairs, SPC), SPC);
        // 循環では向きがある: MUH の位置は HEN を出し、HEN の位置は SPC、SPC の位置は MUH を出す。
        let cycles = [cycle(&[MUH, HEN, SPC])];
        assert_eq!(function_at_units(&[], &cycles, MUH), HEN);
        assert_eq!(function_at_units(&[], &cycles, HEN), SPC);
        assert_eq!(function_at_units(&[], &cycles, SPC), MUH);
        assert_eq!(function_at_units(&[], &cycles, LALT), LALT);
        // 機能から位置を逆引きする。HEN を出しているのは MUH の位置。
        assert_eq!(position_of(&[], &cycles, HEN), MUH);
        assert_eq!(position_of(&[], &cycles, LALT), LALT);
    }

    #[test]
    fn decompose_splits_a_permutation_into_pairs_and_cycles() {
        let sig = [
            (MUH, HEN),
            (HEN, SPC),
            (SPC, MUH),
            (CAPS, LCTRL),
            (LCTRL, CAPS),
        ];
        let (pairs, cycles) = decompose(&sig);
        assert_eq!(pairs, vec![Pair::new(CAPS, LCTRL)]);
        assert_eq!(cycles, vec![cycle(&[MUH, HEN, SPC])]);
        assert!(decompose(&[]).0.is_empty());
    }

    #[test]
    fn rows_hide_jis_only_keys_on_us_unless_they_are_in_a_unit() {
        let flat = |rows: Vec<Vec<u16>>| rows.into_iter().flatten().collect::<Vec<u16>>();
        let jis = flat(diagram_rows(&[], &[], true));
        assert!(jis.contains(&MUH) && jis.contains(&HEN) && jis.contains(&KANA));
        assert!(jis.contains(&SCANCODE_HANKAKU_ZENKAKU));
        let us = flat(diagram_rows(&[], &[], false));
        for k in [MUH, HEN, KANA, SCANCODE_HANKAKU_ZENKAKU] {
            assert!(!us.contains(&k), "{k:04X}");
        }
        assert!(us.contains(&SPC) && us.contains(&CAPS) && us.contains(&RALT));
        // 過去に JIS 配列で書かれた入れ替え（ペアも循環も）の位置は、US でも描いて解除できるようにする。
        let pairs = [Pair::new(MUH, LALT)];
        assert!(flat(diagram_rows(&pairs, &[], false)).contains(&MUH));
        let cycles = [cycle(&[KANA, SPC, LALT])];
        assert!(flat(diagram_rows(&[], &cycles, false)).contains(&KANA));
    }

    #[test]
    fn a_plain_drop_creates_a_pair_and_a_drop_on_home_releases() {
        let (existing, detected) = registry(&[], &[]);
        let c = ctx(&[], &[], &detected, false, true);
        let effect = drop_function(&c, &existing, LCTRL, CAPS).unwrap();
        assert_eq!(effect.pairs, vec![Pair::new(CAPS, LCTRL)]);
        assert!(effect.cycles.is_empty());
        assert_eq!((effect.source, effect.unit_len), (LCTRL, 2));

        let (existing, detected) = registry(&effect.pairs, &[]);
        let c = ctx(&effect.pairs, &[], &detected, false, true);
        // 位置 CAPS に出ている機能(左 Ctrl)を、左 Ctrl の位置へ戻す。
        let back = drop_function(&c, &existing, LCTRL, LCTRL).unwrap();
        assert!(back.pairs.is_empty() && back.cycles.is_empty());
        assert_eq!(back.unit_len, 0);
    }

    #[test]
    fn dropping_onto_a_swapped_position_makes_a_three_cycle() {
        // CAPS⇄LCTRL がある状態で、スペースの機能を CAPS の位置へ落とす。CAPS と SPC の位置の中身が入れ替わる。
        let pairs = [Pair::new(CAPS, LCTRL)];
        let (existing, detected) = registry(&pairs, &[]);
        let c = ctx(&pairs, &[], &detected, false, true);
        let effect = drop_function(&c, &existing, SPC, CAPS).unwrap();
        assert!(effect.pairs.is_empty());
        assert_eq!(effect.cycles, vec![cycle(&[CAPS, SPC, LCTRL])]);
        assert_eq!((effect.source, effect.unit_len), (SPC, 3));
        // CAPS の位置はスペースを、SPC の位置は左 Ctrl（CAPS がそれまで出していた機能）を、LCTRL の位置は CAPS を出す。
        let f = |k| function_at_units(&effect.pairs, &effect.cycles, k);
        assert_eq!((f(CAPS), f(SPC), f(LCTRL)), (SPC, LCTRL, CAPS));
    }

    #[test]
    fn dropping_a_swapped_function_elsewhere_also_makes_a_three_cycle() {
        // CAPS⇄LCTRL。位置 CAPS に出ている機能(左 Ctrl)を、スペースの位置へ落とす。
        let pairs = [Pair::new(CAPS, LCTRL)];
        let (existing, detected) = registry(&pairs, &[]);
        let c = ctx(&pairs, &[], &detected, false, true);
        let effect = drop_function(&c, &existing, LCTRL, SPC).unwrap();
        assert_eq!(effect.source, CAPS);
        let f = |k| function_at_units(&effect.pairs, &effect.cycles, k);
        assert_eq!((f(SPC), f(CAPS), f(LCTRL)), (LCTRL, SPC, CAPS));
    }

    #[test]
    fn dropping_a_function_home_shrinks_a_cycle() {
        // MUH→HEN→SPC→MUH（MUH の位置は HEN を出す）。HEN の機能を HEN の位置へ戻すと、MUH⇄SPC のペアに縮む。
        let cycles = [cycle(&[MUH, HEN, SPC])];
        let (existing, detected) = registry(&[], &cycles);
        let c = ctx(&[], &cycles, &detected, false, true);
        let effect = drop_function(&c, &existing, HEN, HEN).unwrap();
        assert_eq!(effect.pairs, vec![Pair::new(MUH, SPC)]);
        assert!(effect.cycles.is_empty());
        assert_eq!(effect.unit_len, 0);
        // すでにそこにある機能を落としても変わらない。
        let same = drop_function(&c, &existing, HEN, MUH).unwrap();
        assert_eq!(same.cycles, cycles.to_vec());
        assert_eq!(same.source, MUH);
    }

    #[test]
    fn two_unrelated_pairs_can_be_merged_into_a_four_cycle() {
        // MUH⇄HEN と SPC⇄LALT。SPC の機能(SPC の位置は LALT を出しているので LALT)…ここでは SPC を MUH の位置へ落とす。
        let pairs = [Pair::new(MUH, HEN), Pair::new(SPC, LALT)];
        let (existing, detected) = registry(&pairs, &[]);
        let c = ctx(&pairs, &[], &detected, false, true);
        let effect = drop_function(&c, &existing, SPC, MUH).unwrap();
        // SPC の機能を出しているのは LALT の位置。LALT と MUH の位置の中身が入れ替わる。
        assert_eq!(effect.source, LALT);
        assert_eq!(effect.cycles.len() + effect.pairs.len(), 1);
        assert_eq!(effect.unit_len, 4);
        let f = |k| function_at_units(&effect.pairs, &effect.cycles, k);
        assert_eq!(f(MUH), SPC);
        assert_eq!(f(LALT), HEN);
    }

    #[test]
    fn other_tool_entries_lock_positions_and_refuse_drops() {
        // 他ツールの `F1→スペース` がある。スペースを動かすと `X→スペース` の多対一になるので、位置ごと動かせない。
        let existing = [(0x003B, SPC)];
        let detected = detect_swap_pairs(&existing);
        let c = ctx(&[], &[], &detected, false, true);
        assert_eq!(
            position_state(&c, SPC),
            PositionState::Locked {
                reason: LockReason::OtherToolSetting,
                releasable: false
            }
        );
        assert!(matches!(
            drop_function(&c, &existing, SPC, HEN),
            Err(DropRefusal::SourceLocked(LockReason::OtherToolSetting))
        ));
        assert!(matches!(
            drop_function(&c, &existing, HEN, SPC),
            Err(DropRefusal::TargetLocked(LockReason::OtherToolSetting))
        ));
    }

    #[test]
    fn unlisted_keys_are_refused_by_the_write_check() {
        let (existing, detected) = registry(&[], &[]);
        let c = ctx(&[], &[], &detected, false, true);
        assert!(matches!(
            drop_function(&c, &existing, 0x0010, SPC),
            Err(DropRefusal::Invalid(SwapError::NotAllowed { key: 0x0010 }))
        ));
    }

    #[test]
    fn caps_extra_locks_the_caps_and_ctrl_positions() {
        let detected = detect_swap_pairs(&[(CAPS, LCTRL)]);
        let c = ctx(&[], &[], &detected, true, true);
        for k in [CAPS, LCTRL] {
            assert_eq!(
                position_state(&c, k),
                PositionState::Locked {
                    reason: LockReason::CapsExtraOn,
                    releasable: false
                }
            );
        }
        assert_eq!(position_state(&c, SPC), PositionState::Editable);
    }

    #[test]
    fn caps_extra_shows_the_caps_position_acting_as_ctrl() {
        assert_eq!(displayed_function(&[], &[], true, CAPS), LCTRL);
        assert_eq!(displayed_function(&[], &[], true, LCTRL), LCTRL);
        assert_eq!(displayed_function(&[], &[], false, CAPS), CAPS);
        let pairs = [Pair::new(MUH, LALT)];
        assert_eq!(displayed_function(&pairs, &[], true, MUH), LALT);
        let cycles = [cycle(&[MUH, HEN, SPC])];
        assert_eq!(displayed_function(&[], &cycles, false, MUH), HEN);
        assert_eq!(
            locked_note(LockReason::CapsExtraOn, CAPS),
            "Ctrl として動作中"
        );
        assert_eq!(locked_note(LockReason::CapsExtraOn, LCTRL), "動かせません");
        assert_eq!(
            locked_note(LockReason::OtherToolSetting, CAPS),
            "動かせません"
        );
    }

    #[test]
    fn warned_pairs_lock_both_positions_but_stay_releasable() {
        // 自分のペア MUH⇄LALT に、他ツールの `F1→MUH`（to が重なる）がある。
        let existing = [(MUH, LALT), (LALT, MUH), (0x003B, MUH)];
        let detected = detect_swap_pairs(&existing);
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        assert!(detected.pairs.iter().any(|d| d.warning));
        let c = ctx(&pairs, &[], &detected, false, true);
        for k in [MUH, LALT] {
            assert_eq!(
                position_state(&c, k),
                PositionState::Locked {
                    reason: LockReason::OtherToolOverlap,
                    releasable: true
                },
                "{k:04X}"
            );
        }
        // 戻す(解除)は常に通る。
        assert!(release_position(&pairs, &[], MUH).0.is_empty());
        assert!(matches!(
            drop_function(&c, &existing, MUH, SPC),
            Err(DropRefusal::SourceLocked(_))
        ));
        assert!(matches!(
            drop_function(&c, &existing, SPC, LALT),
            Err(DropRefusal::TargetLocked(_))
        ));
    }

    #[test]
    fn unlisted_units_are_listed_and_their_listed_position_is_locked_releasable() {
        // ペアと循環の両方。
        let existing = [
            (MUH, 0x0010),
            (0x0010, MUH),
            (0x0020, 0x0021),
            (0x0021, 0x0022),
            (0x0022, 0x0020),
        ];
        let detected = detect_swap_pairs(&existing);
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        assert_eq!(pairs.len(), 1);
        assert_eq!(detected.cycles.len(), 1);
        let unlisted = unlisted_units(&pairs, &detected.cycles);
        assert_eq!(unlisted.len(), 2);
        let c = ctx(&pairs, &detected.cycles, &detected, false, true);
        assert_eq!(
            position_state(&c, MUH),
            PositionState::Locked {
                reason: LockReason::HasUnlistedKey,
                releasable: true
            }
        );
        // 単位ごとに解除できる。
        let (p, cy) = release_unit(&pairs, &detected.cycles, &unlisted[1]);
        assert_eq!((p.len(), cy.len()), (1, 0));
    }

    #[test]
    fn jis_only_units_on_us_are_release_only() {
        let (existing, detected) = registry(&[Pair::new(MUH, LALT)], &[]);
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        let c = ctx(&pairs, &[], &detected, false, false);
        for k in [MUH, LALT] {
            assert_eq!(
                position_state(&c, k),
                PositionState::Locked {
                    reason: LockReason::NoPhysicalKey,
                    releasable: true
                }
            );
        }
        let _ = existing;
        let cycles = [cycle(&[KANA, SPC, LALT])];
        let (_, detected) = registry(&[], &cycles);
        let c = ctx(&[], &cycles, &detected, false, false);
        assert!(matches!(
            position_state(&c, SPC),
            PositionState::Locked {
                reason: LockReason::NoPhysicalKey,
                releasable: true
            }
        ));
    }

    #[test]
    fn a_caps_filter_driver_locks_caps_and_ctrl_positions_and_refuses_new_settings() {
        let (existing, detected) = registry(&[], &[]);
        let mut c = ctx(&[], &[], &detected, false, true);
        c.caps_filter = true;
        for k in [CAPS, LCTRL] {
            assert_eq!(
                position_state(&c, k),
                PositionState::Locked {
                    reason: LockReason::CapsFilterDriver,
                    releasable: false
                }
            );
        }
        assert_eq!(position_state(&c, SPC), PositionState::Editable);
        assert!(matches!(
            drop_function(&c, &existing, SPC, CAPS),
            Err(DropRefusal::TargetLocked(LockReason::CapsFilterDriver))
        ));
        assert!(matches!(
            drop_function(&c, &existing, CAPS, SPC),
            Err(DropRefusal::SourceLocked(LockReason::CapsFilterDriver))
        ));
        assert!(drop_function(&c, &existing, HEN, SPC).is_ok());
        // すでに Caps↔Ctrl の入れ替えがあるときは、「戻す」(解除)だけ許す。
        let pairs = [Pair::new(CAPS, LCTRL)];
        let (_, detected) = registry(&pairs, &[]);
        let mut c = ctx(&pairs, &[], &detected, false, true);
        c.caps_filter = true;
        assert_eq!(
            position_state(&c, CAPS),
            PositionState::Locked {
                reason: LockReason::CapsFilterDriver,
                releasable: true
            }
        );
        assert!(release_position(&pairs, &[], CAPS).0.is_empty());
    }

    #[test]
    fn a_thumb_key_that_is_also_an_ime_key_is_locked_in_the_diagram() {
        let thumbs = [HEN];
        let (existing, detected) = registry(&[], &[]);
        let mut c = ctx(&[], &[], &detected, false, true);
        c.ime_role_thumbs = &thumbs;
        assert_eq!(
            position_state(&c, HEN),
            PositionState::Locked {
                reason: LockReason::ThumbImeRole,
                releasable: false
            }
        );
        assert_eq!(position_state(&c, MUH), PositionState::Editable);
        assert!(matches!(
            drop_function(&c, &existing, SPC, HEN),
            Err(DropRefusal::TargetLocked(LockReason::ThumbImeRole))
        ));
        assert!(matches!(
            drop_function(&c, &existing, HEN, SPC),
            Err(DropRefusal::SourceLocked(LockReason::ThumbImeRole))
        ));
        assert!(drop_function(&c, &existing, MUH, SPC).is_ok());
        let pairs = [Pair::new(HEN, SPC)];
        let (_, detected) = registry(&pairs, &[]);
        let mut c = ctx(&pairs, &[], &detected, false, true);
        c.ime_role_thumbs = &thumbs;
        assert_eq!(
            position_state(&c, HEN),
            PositionState::Locked {
                reason: LockReason::ThumbImeRole,
                releasable: true
            }
        );
    }

    #[test]
    fn hint_and_refusal_texts_are_never_empty_and_describe_the_effect() {
        let (existing, detected) = registry(&[], &[]);
        let c = ctx(&[], &[], &detected, false, true);
        let ok = drop_function(&c, &existing, LCTRL, CAPS);
        let h = drop_hint(&ok, CAPS, |k| format!("<{k}>"));
        assert!(h.contains("入れ替わります") && !h.contains("循環"));
        let pairs = [Pair::new(CAPS, LCTRL)];
        let (existing, detected) = registry(&pairs, &[]);
        let c = ctx(&pairs, &[], &detected, false, true);
        let cyc = drop_function(&c, &existing, SPC, CAPS);
        assert!(drop_hint(&cyc, CAPS, |k| format!("<{k}>")).contains("3個のキーの循環"));
        let home = drop_function(&c, &existing, LCTRL, LCTRL);
        assert!(drop_hint(&home, LCTRL, |k| format!("<{k}>")).contains("元に戻ります"));
        let same = drop_function(&c, &existing, LCTRL, CAPS);
        assert!(drop_hint(&same, CAPS, |k| format!("<{k}>")).contains("変わりません"));
        let bad = drop_function(&c, &existing, 0x0010, SPC);
        assert!(!drop_hint(&bad, SPC, |k| format!("{k}")).is_empty());
        for reason in [
            LockReason::OtherToolSetting,
            LockReason::CapsExtraOn,
            LockReason::OtherToolOverlap,
            LockReason::HasUnlistedKey,
            LockReason::NoPhysicalKey,
            LockReason::CapsFilterDriver,
            LockReason::ThumbImeRole,
        ] {
            assert!(!refusal_text(DropRefusal::TargetLocked(reason)).is_empty());
        }
    }

    fn all_permutations(keys: &[u16]) -> Vec<Vec<u16>> {
        fn rec(rest: &mut Vec<u16>, cur: &mut Vec<u16>, out: &mut Vec<Vec<u16>>) {
            if rest.is_empty() {
                out.push(cur.clone());
                return;
            }
            for i in 0..rest.len() {
                let k = rest.remove(i);
                cur.push(k);
                rec(rest, cur, out);
                cur.pop();
                rest.insert(i, k);
            }
        }
        let mut out = Vec::new();
        rec(&mut keys.to_vec(), &mut Vec::new(), &mut out);
        out
    }

    #[test]
    fn small_universe_every_drop_is_a_valid_transposition_and_releases_always_pass() {
        // 4キーの置換24通りの全部から、全位置×全機能のドロップ。結果が Ok なら、(1) 落とした位置に機能が来る、(2) 変わる位置は
        // 元の位置と落とした位置の2つだけ、(3) 書き込みの検査に通る。「戻す」は常に通る。
        let keys = [LALT, SPC, MUH, HEN];
        let mut oks = 0;
        for image in all_permutations(&keys) {
            let sig: Vec<Entry> = keys
                .iter()
                .zip(&image)
                .filter(|(k, v)| k != v)
                .map(|(&k, &v)| (k, v))
                .collect();
            let (pairs, cycles) = decompose(&sig);
            let existing = entries_of(&pairs, &cycles);
            let detected = detect_swap_pairs(&existing);
            let c = ctx(&pairs, &cycles, &detected, false, true);
            for &f in &keys {
                for &p in &keys {
                    let Ok(effect) = drop_function(&c, &existing, f, p) else {
                        continue;
                    };
                    oks += 1;
                    let after = |k| function_at_units(&effect.pairs, &effect.cycles, k);
                    let before = |k| function_at_units(&pairs, &cycles, k);
                    assert_eq!(after(p), f, "{image:?} f={f:04X} p={p:04X}");
                    for &k in &keys {
                        if k != p && k != effect.source {
                            assert_eq!(after(k), before(k), "{image:?} k={k:04X}");
                        }
                    }
                    compute_swap_write_with_cycles(&existing, &effect.pairs, &effect.cycles, false)
                        .unwrap();
                    // 結果は置換のまま（機能の重複が無い）。
                    let mut outs: Vec<u16> = keys.iter().map(|&k| after(k)).collect();
                    outs.sort_unstable();
                    let mut want = keys.to_vec();
                    want.sort_unstable();
                    assert_eq!(outs, want, "{image:?}");
                }
                // 「戻す」は常に通る。
                let (rp, rc) = release_position(&pairs, &cycles, f);
                compute_swap_write_with_cycles(&existing, &rp, &rc, false).unwrap();
                assert!(unit_containing(&rp, &rc, f).is_none());
            }
        }
        assert!(oks > 300, "{oks}");
        // ペアだけの検査関数も、結果をそのまま受け付ける（互換）。
        compute_swap_write(&[], &[], false).unwrap();
    }
}
