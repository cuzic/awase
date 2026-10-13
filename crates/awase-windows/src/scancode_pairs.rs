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

/// 3つ以上のキーの循環（ADR-248 決定6）。`keys[i]` の位置が `keys[i+1]`（最後は `keys[0]`）の機能を出す。
/// エントリは `(keys[i], keys[i+1])`。長さ2の循環は [`Pair`]。最小のキーが先頭になるよう回転して正規化する
/// （向きは区別する: 逆向きの循環は別の置換）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cycle {
    keys: Vec<u16>,
}

impl Cycle {
    /// 3つ以上・重複なし・0 を含まないときだけ作れる。
    #[must_use]
    pub fn new(keys: &[u16]) -> Option<Self> {
        if keys.len() < 3 || keys.contains(&0) {
            return None;
        }
        let mut sorted = keys.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != keys.len() {
            return None;
        }
        let start = keys.iter().enumerate().min_by_key(|&(_, &k)| k)?.0;
        let mut rotated = keys[start..].to_vec();
        rotated.extend_from_slice(&keys[..start]);
        Some(Self { keys: rotated })
    }

    /// 循環のキー（最小のキーが先頭）。
    #[must_use]
    pub fn keys(&self) -> &[u16] {
        &self.keys
    }

    /// この循環が書き込むエントリ `(位置, その位置が出す機能)`。
    #[must_use]
    pub fn entries(&self) -> Vec<Entry> {
        let n = self.keys.len();
        (0..n)
            .map(|i| (self.keys[i], self.keys[(i + 1) % n]))
            .collect()
    }

    /// `key` を含むか。
    #[must_use]
    pub fn contains(&self, key: u16) -> bool {
        self.keys.contains(&key)
    }
}

/// ペアと循環を「位置 → 機能」のエントリにする（恒等の位置は含まない）。
#[must_use]
pub fn permutation_entries(pairs: &[Pair], cycles: &[Cycle]) -> Vec<Entry> {
    let mut out: Vec<Entry> = pairs.iter().flat_map(|p| p.entries()).collect();
    out.extend(cycles.iter().flat_map(Cycle::entries));
    out
}

/// 2つの置換（[`permutation_entries`] の形）の間で、位置または機能が変わるキー。
/// 位置 P の機能が変わるなら、P と、変わる前後の機能を含む。単位の中身が変わっても、位置も機能も変わらないキーは含まない。
#[must_use]
pub fn touched_keys(before: &[Entry], after: &[Entry]) -> Vec<u16> {
    let image = |sig: &[Entry], k: u16| sig.iter().find(|e| e.0 == k).map_or(k, |e| e.1);
    let mut positions: Vec<u16> = before.iter().chain(after).map(|e| e.0).collect();
    positions.sort_unstable();
    positions.dedup();
    let mut out: Vec<u16> = Vec::new();
    for p in positions {
        let (b, a) = (image(before, p), image(after, p));
        if b != a {
            out.extend([p, b, a]);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// [`touched_keys`] のうち、変更後の置換にまだ入っている（位置か機能として入れ替えに残っている）キー。恒等へ戻って入れ替えから
/// 外れたキーは含まない。「動かせないキーを入れ替えに新しく入れた・動かした」の判定に使う（解除は含まない）。
#[must_use]
pub fn moved_keys(before: &[Entry], after: &[Entry]) -> Vec<u16> {
    touched_keys(before, after)
        .into_iter()
        .filter(|&k| after.iter().any(|e| e.0 == k || e.1 == k))
        .collect()
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
    /// 3つ以上の閉じた循環（from も to も他のエントリと重ならないもの）。編集できる入れ替えとして読む（ADR-248 決定6）。
    pub cycles: Vec<Cycle>,
    /// `Caps/英数 → 左 Ctrl` の片方向だけがある（ADR-126 のプリセット）。
    pub caps_extra_ctrl: bool,
    /// ペアでもプリセットでもないエントリ（他ツールのもの。保持され、読み取り専用で表示する）。
    pub unclaimed: Vec<(Entry, UnclaimedReason)>,
}

/// 3つ以上の閉じた循環を探す（ADR-248 決定6）。採用済みペアのキーと被らず、from も to も全エントリの中で1回ずつだけの
/// エントリをたどって元へ戻るもの。許可リストには関係なく読む（ペアと同じ。作る・変えるときだけ許可リストを当てる）。
/// 見つけた循環のエントリは `used` に印を付ける。
fn find_cycles(entries: &[Entry], used: &mut [bool], taken_keys: &[u16]) -> Vec<Cycle> {
    let candidate: Vec<usize> = (0..entries.len())
        .filter(|&i| {
            let (a, b) = entries[i];
            !used[i]
                && a != b
                && a != 0
                && b != 0
                && !taken_keys.contains(&a)
                && !taken_keys.contains(&b)
                && entries.iter().filter(|e| e.0 == a).count() == 1
                && entries.iter().filter(|e| e.1 == b).count() == 1
        })
        .collect();
    let mut cycles: Vec<Cycle> = Vec::new();
    let mut in_cycle = vec![false; entries.len()];
    for &start in &candidate {
        if in_cycle[start] {
            continue;
        }
        let mut chain = vec![start];
        let mut current = start;
        let closed = loop {
            let want = entries[current].1;
            let Some(&next) = candidate.iter().find(|&&j| entries[j].0 == want) else {
                break false;
            };
            if next == start {
                break true;
            }
            if chain.contains(&next) {
                break false;
            }
            chain.push(next);
            current = next;
        };
        if !closed {
            continue;
        }
        let keys: Vec<u16> = chain.iter().map(|&i| entries[i].0).collect();
        if let Some(cycle) = Cycle::new(&keys) {
            for &i in &chain {
                in_cycle[i] = true;
                used[i] = true;
            }
            cycles.push(cycle);
        }
    }
    cycles
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
        if used[i]
            || a == b
            || a == 0
            || b == 0
            || taken_keys.contains(&a)
            || taken_keys.contains(&b)
        {
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

    // 3つ以上の閉じた循環（ADR-248 決定6）。Caps 追加 Ctrl より先に見る:
    // `Caps→左Ctrl, 左Ctrl→X, X→Caps` は、プリセット+他ツール2件ではなく1つの循環。
    let cycles = find_cycles(entries, &mut used, &taken_keys);

    // Caps 追加 Ctrl: `Caps→左 Ctrl` が単独（逆向きがペアに取られていない）で残っている。
    // ただし Caps が採用済みペアのキーなら、同じ `Caps→左 Ctrl`（ペアの複製や、ペア `Caps⇄X` に重なる from 重複）は
    // プリセットではなく from 重複の Unclaimed にする（ペアと Caps 追加 Ctrl は排他、ADR-230 決定1・決定3の表）。
    let caps_pos = if taken_keys.contains(&SCANCODE_CAPS_EISU) {
        None
    } else {
        entries
            .iter()
            .enumerate()
            .position(|(i, &e)| !used[i] && e == (SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL))
    };
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
        cycles,
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
    /// 書いた結果を読み戻すと、要求した入れ替えがそのまま読めない、または「Caps を Ctrl としても使う」が別の入れ替えに
    /// 読み替わる（他ツールの開いた鎖が、足したエントリで閉じて循環になる、など）。他ツールのエントリを無断で awase の入れ替えにしないため拒否する。
    Reclassified,
}

/// [`compute_swap_write`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePlan {
    /// レジストリへ書く全エントリ列。出力順は「既存の保持分を既存の順、新規ペアを末尾・入力順、
    /// 新規の Caps 追加 Ctrl をその後」に固定する（読み戻し検証が順序込みの一致比較のため）。
    pub entries: Vec<Entry>,
    /// 上書きで消える他ツールのエントリ。UI が事前に確認する（ADR-230 決定3）。追加するエントリと同じ値のものは含まない
    /// （書く値が変わらないので確認は要らない）。
    pub displaced: Vec<Entry>,
    /// この書き込みで、隠れていたかもしれないエントリが効き出す可能性があるもの。消すペア・Caps 追加 Ctrl と同じ from を持つ
    /// 他ツールのエントリ（from が重複していたとき、OS は先頭のエントリを効かせる=先勝ち〈WinPE の VM で確認、ADR-257 段階2〉ので、先頭を消すと残った方が効き出しうる）。
    /// 書き込みを止める理由にはならず、UI が警告する。
    pub revealed: Vec<Entry>,
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
    compute_swap_write_with_cycles(existing, pairs, &[], caps_extra_ctrl)
}

/// 編集の単位（ペア＝長さ2の循環、または3つ以上の循環）。キーとエントリを持つ。
#[derive(Debug, Clone)]
struct Unit {
    keys: Vec<u16>,
    entries: Vec<Entry>,
}

impl Unit {
    fn from_pair(pair: Pair) -> Self {
        let (a, b) = pair.keys();
        Self {
            keys: vec![a, b],
            entries: pair.entries().to_vec(),
        }
    }

    fn from_cycle(cycle: &Cycle) -> Self {
        Self {
            keys: cycle.keys().to_vec(),
            entries: cycle.entries(),
        }
    }

    /// 書き込むエントリの集合が同じか（順序は問わない）。
    fn same(&self, other: &Self) -> bool {
        let sorted = |u: &Self| {
            let mut e = u.entries.clone();
            e.sort_unstable();
            e
        };
        sorted(self) == sorted(other)
    }

    /// `A⇄A`（自分自身とのペア）。
    fn is_self_pair(&self) -> bool {
        self.keys.len() == 2 && self.keys[0] == self.keys[1]
    }
}

/// [`compute_swap_write`] のペアに、3つ以上の循環 `cycles`（ADR-248 決定6）を加えたもの。
///
/// ペアと循環は同じ「単位」として検査する:
/// 同じキーが複数の単位に現れてはならず、新規に作る単位のキーは許可リスト内で、Caps 追加 Ctrl・他ツールの to と衝突しない。
/// 「残す・消す」の単位は、ペアも循環も、エントリがすべて値として一致するかどうか（1つでも変われば、元の単位を消して新規に作る）。
///
/// # Errors
/// [`SwapError`] を参照。
pub fn compute_swap_write_with_cycles(
    existing: &[Entry],
    pairs: &[Pair],
    cycles: &[Cycle],
    caps_extra_ctrl: bool,
) -> Result<WritePlan, SwapError> {
    // 1. `pairs`・`cycles` 自体の検査（キー単位）。
    let units: Vec<Unit> = pairs
        .iter()
        .map(|&p| Unit::from_pair(p))
        .chain(cycles.iter().map(Unit::from_cycle))
        .collect();
    let mut seen: Vec<u16> = Vec::new();
    for unit in &units {
        if unit.is_self_pair() {
            return Err(SwapError::SelfPair { key: unit.keys[0] });
        }
        for &key in &unit.keys {
            if seen.contains(&key) {
                return Err(SwapError::DuplicateKey { key });
            }
            seen.push(key);
        }
    }

    let detected = detect_swap_pairs(existing);
    let existing_units: Vec<Unit> = detected
        .pairs
        .iter()
        .map(|d| Unit::from_pair(d.pair))
        .chain(detected.cycles.iter().map(Unit::from_cycle))
        .collect();
    let new_units: Vec<&Unit> = units
        .iter()
        .filter(|u| !existing_units.iter().any(|e| e.same(u)))
        .collect();
    let adds_caps = caps_extra_ctrl && !detected.caps_extra_ctrl;

    // 2. 新規に作るものだけに、許可リストと Caps 追加 Ctrl との衝突を適用する。
    for unit in &new_units {
        for &key in &unit.keys {
            if !ALLOWED_SCANCODES.contains(&key) {
                return Err(SwapError::NotAllowed { key });
            }
        }
    }
    // Caps 追加 Ctrl を新たに足すときは全単位を、既に有効なときは新規の単位だけを検査する
    // （既存の状態が既に同居していても、何も変えない操作や削除は通す）。
    if caps_extra_ctrl {
        let checked: Vec<&Unit> = if adds_caps {
            units.iter().collect()
        } else {
            new_units.clone()
        };
        for unit in checked {
            for &key in &unit.keys {
                if key == SCANCODE_CAPS_EISU || key == SCANCODE_LEFT_CTRL {
                    return Err(SwapError::CapsExtraConflict { key });
                }
            }
        }
    }

    // 3. 新規に作るエントリ。
    let mut added: Vec<Entry> = new_units
        .iter()
        .flat_map(|u| u.entries.iter().copied())
        .collect();
    if adds_caps {
        added.push((SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL));
    }

    // 4. 消すものを決める。外された単位・外された Caps 追加 Ctrl のエントリ（removed_own）と、新規エントリの from と同じ
    //    from を持つ他ツールのエントリ（replaced。Caps 追加 Ctrl を足すとき、他ツールの `左 Ctrl→Caps` が残ると組が
    //    入れ替えに読み替わるので、これも消す）。いずれも**値**で消す（同じ写像の複製は1つを消すなら全部消す）。
    let mut removed_own: Vec<Entry> = Vec::new();
    for existing_unit in &existing_units {
        if !units.iter().any(|u| u.same(existing_unit)) {
            removed_own.extend(existing_unit.entries.iter().copied());
        }
    }
    if detected.caps_extra_ctrl && !caps_extra_ctrl {
        removed_own.push((SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL));
    }
    let added_from: Vec<u16> = added.iter().map(|e| e.0).collect();
    let replaced: Vec<Entry> = detected
        .unclaimed
        .iter()
        .map(|&(e, _)| e)
        .filter(|e| {
            added_from.contains(&e.0)
                || (adds_caps && *e == (SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU))
        })
        .collect();
    let displaced: Vec<Entry> = replaced
        .iter()
        .copied()
        .filter(|e| !added.contains(e))
        .collect();
    let is_removed = |e: &Entry| removed_own.contains(e) || replaced.contains(e);

    // 5. 残る他ツールのエントリの to と、新規エントリの to が同じなら、2つのキーが同じキーを出す多対一を新たに作る。
    for &(entry, _) in &detected.unclaimed {
        if is_removed(&entry) {
            continue;
        }
        if let Some(&culprit) = added.iter().find(|&&(_, to)| to == entry.1) {
            return Err(SwapError::CollidesWithForeignTarget { entry: culprit });
        }
    }

    // 6. 出力: 既存の順のまま、消すものを除き、末尾に新規を足す。
    let out_existing: Vec<Entry> = existing
        .iter()
        .copied()
        .filter(|e| !is_removed(e))
        .collect();
    let revealed: Vec<Entry> = detected
        .unclaimed
        .iter()
        .map(|&(e, _)| e)
        .filter(|e| {
            !is_removed(e)
                && !added_from.contains(&e.0)
                && (removed_own.iter().any(|r| r.0 == e.0) || replaced.iter().any(|r| r.0 == e.0))
        })
        .collect();
    let mut out = out_existing;
    out.extend(added);
    // 事後条件（新しく足すものだけに当てる）: 書いた結果を読み戻すと、新しく作る単位がそのまま読め、Caps 追加 Ctrl を新たに
    // オンにするならそのまま読める。他ツールの開いた鎖が、足したエントリで閉じて循環に読み替わると、他ツールの設定が確認なしで
    // awase の入れ替えになり、以後の「戻す」「すべて解除」で消えてしまう（Opus コードレビュー S1）。
    // 残す単位・すでにオンの Caps 追加 Ctrl には当てない: 削除で他のエントリが露出して読み替わるのは ADR-230 で認めた現象
    // （`revealed`）で、書き込みを止めると「削除だけの操作は常に通る」が崩れる（Opus コードレビュー round2 B1）。
    let read = detect_swap_pairs(&out);
    let read_units: Vec<Unit> = read
        .pairs
        .iter()
        .map(|d| Unit::from_pair(d.pair))
        .chain(read.cycles.iter().map(Unit::from_cycle))
        .collect();
    let new_units_read = new_units
        .iter()
        .all(|u| read_units.iter().any(|r| r.same(u)));
    if !new_units_read || (adds_caps && !read.caps_extra_ctrl) {
        return Err(SwapError::Reclassified);
    }
    Ok(WritePlan {
        entries: out,
        displaced,
        revealed,
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
    fn three_cycle_is_read_as_a_cycle_not_unclaimed() {
        let d = detect_swap_pairs(&[(MUH, HEN), (HEN, SPC), (SPC, MUH)]);
        assert!(d.pairs.is_empty());
        assert!(d.unclaimed.is_empty());
        assert_eq!(d.cycles, vec![Cycle::new(&[MUH, HEN, SPC]).unwrap()]);
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

    // ---- Opus コードレビュー round1(PR #501)の反例 ----

    #[test]
    fn b1_duplicate_of_a_pair_entry_is_not_taken_as_caps_extra_ctrl() {
        // ペア Caps⇄LCtrl に同値の複製があっても、Caps 追加 Ctrl とは読まず from 重複の Unclaimed にする。
        let existing = vec![(CAPS, LCTRL), (LCTRL, CAPS), (CAPS, LCTRL)];
        let d = detect_swap_pairs(&existing);
        assert_eq!(d.pairs.len(), 1);
        assert!(!d.caps_extra_ctrl);
        assert_eq!(
            d.unclaimed,
            vec![((CAPS, LCTRL), UnclaimedReason::DuplicateFrom)]
        );
        // ペアを残してチェックを外すだけ: 何も変わらない（左 Ctrl が消えて多対一になったりしない）。
        let plan = compute_swap_write(&existing, &[pair(CAPS, LCTRL)], false).unwrap();
        assert_eq!(plan.entries, existing);
        // ペアを外して Caps 追加 Ctrl を入れる: 複製も含めて、Caps→左 Ctrl が1つだけ残る。
        let plan = compute_swap_write(&existing, &[], true).unwrap();
        assert_eq!(plan.entries, vec![(CAPS, LCTRL)]);
        // ペアを残したまま Caps 追加 Ctrl を入れるのは排他で拒否する。
        assert!(matches!(
            compute_swap_write(&existing, &[pair(CAPS, LCTRL)], true),
            Err(SwapError::CapsExtraConflict { .. })
        ));
    }

    #[test]
    fn s1_caps_to_ctrl_overlapping_a_caps_pair_is_duplicate_from_not_caps_extra_ctrl() {
        let d = detect_swap_pairs(&[(CAPS, MUH), (MUH, CAPS), (CAPS, LCTRL)]);
        assert_eq!(
            d.pairs,
            vec![DetectedPair {
                pair: pair(CAPS, MUH),
                warning: true
            }]
        );
        assert!(!d.caps_extra_ctrl);
        assert_eq!(
            d.unclaimed,
            vec![((CAPS, LCTRL), UnclaimedReason::DuplicateFrom)]
        );
    }

    #[test]
    fn n2_replacing_a_pair_that_has_a_same_value_duplicate_is_not_a_false_collision() {
        // 無変換⇄左Alt（複製つき）を外して、変換⇄左Alt にする、が1回でできる。
        let existing = vec![(MUH, LALT), (LALT, MUH), (MUH, LALT)];
        let plan = compute_swap_write(&existing, &[pair(HEN, LALT)], false).unwrap();
        assert_eq!(plan.entries, pe(HEN, LALT));
    }

    #[test]
    fn n3_displaced_excludes_entries_with_the_same_value_as_an_added_entry() {
        // 無変換⇄変換 を、別の並びで読まれた同値のエントリがある状態から作る。書く値は変わらないので承認は要らない。
        let existing = vec![(MUH, LALT), (LALT, MUH), (MUH, HEN), (HEN, MUH)];
        // 先頭の無変換⇄左Alt が採用され、(MUH,HEN),(HEN,MUH) は Unclaimed。
        let plan = compute_swap_write(&existing, &[pair(MUH, HEN)], false).unwrap();
        assert_eq!(plan.entries, pe(MUH, HEN));
        assert!(plan.displaced.is_empty());
    }

    #[test]
    fn n4_pair_with_zero_key_is_not_a_pair() {
        let d = detect_swap_pairs(&[(MUH, 0), (0, MUH)]);
        assert!(d.pairs.is_empty());
        assert_eq!(d.unclaimed.len(), 2);
    }

    #[test]
    fn s2_removal_reports_entries_that_may_start_to_take_effect() {
        // Caps 追加 Ctrl を外すと、同じ from の隠れていた `Caps→変換` が効き出しうる（同じ from のエントリは OS が先頭のものを効かせる=先勝ち。WinPE の VM で確認、ADR-257・2026-10-11）。
        let existing = vec![
            (CAPS, LCTRL),
            (LALT, HEN),
            (LALT, LCTRL),
            (HEN, 0),
            (CAPS, HEN),
        ];
        let plan = compute_swap_write(&existing, &[], false).unwrap();
        assert_eq!(plan.revealed, vec![(CAPS, HEN)]);
        assert!(plan.entries.contains(&(CAPS, HEN)));
        // 何も消さない操作では何も報告しない。
        let same = compute_swap_write(&existing, &[], true).unwrap();
        assert!(same.revealed.is_empty());
    }

    #[test]
    fn s2_1_caps_extra_ctrl_removing_a_left_ctrl_to_caps_entry_reports_same_from_entries() {
        // Caps 追加 Ctrl を足すとき、他ツールの `左 Ctrl→Caps` は消える（組が入れ替えに読み替わるため）。
        // 同じ from の `左 Ctrl→右 Alt` が残り、効き出しうるので revealed に出す。
        let existing = vec![(LCTRL, CAPS), (LCTRL, SCANCODE_RIGHT_ALT)];
        let plan = compute_swap_write(&existing, &[], true).unwrap();
        assert_eq!(plan.displaced, vec![(LCTRL, CAPS)]);
        assert_eq!(plan.revealed, vec![(LCTRL, SCANCODE_RIGHT_ALT)]);
    }

    // ---- 全列挙（小さな宇宙、E0 キーと 0 を含む） ----

    #[test]
    fn exhaustive_small_universe_keeps_the_invariants() {
        const KEYS: [u16; 4] = [CAPS, LCTRL, MUH, SCANCODE_RIGHT_ALT];
        const TOS: [u16; 5] = [CAPS, LCTRL, MUH, SCANCODE_RIGHT_ALT, 0];
        let all_entries: Vec<Entry> = KEYS
            .iter()
            .chain(std::iter::once(&0))
            .flat_map(|&f| TOS.iter().map(move |&t| (f, t)))
            .collect();
        // 許可リストの4キーで作れる、互いに素なペア集合（空、1組、2組）。
        let mut pair_sets: Vec<Vec<Pair>> = vec![Vec::new()];
        for (i, &a) in KEYS.iter().enumerate() {
            for &b in &KEYS[i + 1..] {
                pair_sets.push(vec![Pair::new(a, b)]);
            }
        }
        pair_sets.push(vec![
            Pair::new(CAPS, LCTRL),
            Pair::new(MUH, SCANCODE_RIGHT_ALT),
        ]);
        pair_sets.push(vec![
            Pair::new(CAPS, MUH),
            Pair::new(LCTRL, SCANCODE_RIGHT_ALT),
        ]);
        pair_sets.push(vec![
            Pair::new(CAPS, SCANCODE_RIGHT_ALT),
            Pair::new(LCTRL, MUH),
        ]);

        let mut lists: Vec<Vec<Entry>> = vec![Vec::new()];
        for &a in &all_entries {
            lists.push(vec![a]);
            for &b in &all_entries {
                lists.push(vec![a, b]);
                for &c in &all_entries {
                    lists.push(vec![a, b, c]);
                }
            }
        }
        let mut ok = 0usize;
        for existing in &lists {
            let before = detect_swap_pairs(existing);
            // 何も変えない呼び出しは恒等、削除だけは常に成功する。
            assert_identity_write(existing, &before);
            let removal = compute_swap_write(existing, &[], false).expect("全削除は常に通る");
            assert!(removal.entries.len() <= existing.len());
            for pairs in &pair_sets {
                for caps in [false, true] {
                    let Ok(plan) = compute_swap_write(existing, pairs, caps) else {
                        continue;
                    };
                    ok += 1;
                    let after = detect_swap_pairs(&plan.entries);
                    let got: Vec<Pair> = after.pairs.iter().map(|d| d.pair).collect();
                    for p in pairs {
                        assert!(
                            got.contains(p),
                            "existing={existing:?} pairs={pairs:?} caps={caps}"
                        );
                    }
                    // 指定どおりの Caps 追加 Ctrl（ペアが Caps⇄左Ctrl に読み替わった場合と、重なっていた他ツールの
                    // `Caps→左 Ctrl` が単独になった場合を除く）。
                    assert!(
                        after.caps_extra_ctrl == caps
                            || (caps && got.contains(&Pair::new(CAPS, LCTRL)))
                            || (after.caps_extra_ctrl
                                && before.unclaimed.iter().any(|&(e, _)| e == (CAPS, LCTRL))),
                        "existing={existing:?} pairs={pairs:?} caps={caps}"
                    );
                }
            }
        }
        assert!(ok > 10_000, "検査した組が少なすぎる: {ok}");
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

    /// 読んだ分類をそのまま渡した書き込みは恒等（何も変えない）。
    fn assert_identity_write(existing: &[Entry], before: &Detected) {
        let pairs: Vec<Pair> = before.pairs.iter().map(|d| d.pair).collect();
        let same = compute_swap_write_with_cycles(
            existing,
            &pairs,
            &before.cycles,
            before.caps_extra_ctrl,
        )
        .expect("恒等は常に通る");
        assert_eq!(same.entries, existing, "identity: {existing:?}");
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
            assert_identity_write(&existing, &before);

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
                after.caps_extra_ctrl == caps
                    || (caps && got.contains(&Pair::new(CAPS, LCTRL)))
                    // 消したペアと重なっていた他ツールの `Caps→左 Ctrl` が、単独になって Caps 追加 Ctrl に読めるようになった。
                    || (after.caps_extra_ctrl
                        && before.unclaimed.iter().any(|&(e, _)| e == (CAPS, LCTRL))),
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

    // ---- 循環（ADR-248 決定6） ----

    fn all_permutations(keys: &[u16]) -> Vec<Vec<u16>> {
        // keys の並べ替え全部（σ(keys[i]) = result[i]）。
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

    /// 置換 σ（`keys[i]` の位置が `image[i]` を出す）を、長さ2の循環（ペア）と3つ以上の循環に分ける。
    fn decompose(keys: &[u16], image: &[u16]) -> (Vec<Pair>, Vec<Cycle>) {
        let sigma = |k: u16| image[keys.iter().position(|&x| x == k).unwrap()];
        let mut seen: Vec<u16> = Vec::new();
        let (mut pairs, mut cycles) = (Vec::new(), Vec::new());
        for &start in keys {
            if seen.contains(&start) || sigma(start) == start {
                continue;
            }
            let mut chain = vec![start];
            let mut cur = sigma(start);
            while cur != start {
                chain.push(cur);
                cur = sigma(cur);
            }
            seen.extend(chain.iter().copied());
            if chain.len() == 2 {
                pairs.push(Pair::new(chain[0], chain[1]));
            } else {
                cycles.push(Cycle::new(&chain).unwrap());
            }
        }
        (pairs, cycles)
    }

    #[test]
    fn cycle_new_normalizes_rotation_and_rejects_bad_input() {
        let c = Cycle::new(&[HEN, SPC, MUH]).unwrap();
        assert_eq!(c.keys()[0], c.keys().iter().copied().min().unwrap());
        assert_eq!(c, Cycle::new(&[SPC, MUH, HEN]).unwrap());
        assert_eq!(c, Cycle::new(&[MUH, HEN, SPC]).unwrap());
        // 向きは区別する。
        assert_ne!(
            Cycle::new(&[MUH, HEN, SPC]).unwrap(),
            Cycle::new(&[MUH, SPC, HEN]).unwrap()
        );
        assert!(Cycle::new(&[MUH, HEN]).is_none());
        assert!(Cycle::new(&[MUH, HEN, MUH]).is_none());
        assert!(Cycle::new(&[MUH, HEN, 0]).is_none());
        // エントリは「位置 → その位置が出す機能」。
        let c = Cycle::new(&[MUH, HEN, SPC]).unwrap();
        let mut e = c.entries();
        e.sort_unstable();
        let mut want = vec![(MUH, HEN), (HEN, SPC), (SPC, MUH)];
        want.sort_unstable();
        assert_eq!(e, want);
    }

    #[test]
    fn a_cycle_with_an_overlapping_foreign_entry_is_not_read_as_a_cycle() {
        // from の重複
        let d = detect_swap_pairs(&[(MUH, HEN), (HEN, SPC), (SPC, MUH), (MUH, LALT)]);
        assert!(d.cycles.is_empty());
        assert_eq!(d.unclaimed.len(), 4);
        // to の重複（別のキーが同じ機能を出す）
        let d = detect_swap_pairs(&[(MUH, HEN), (HEN, SPC), (SPC, MUH), (LALT, MUH)]);
        assert!(d.cycles.is_empty());
        // 閉じていない鎖
        let d = detect_swap_pairs(&[(MUH, HEN), (HEN, SPC)]);
        assert!(d.cycles.is_empty());
        assert_eq!(d.unclaimed.len(), 2);
    }

    #[test]
    fn a_cycle_outside_the_allowlist_is_still_read_as_a_cycle() {
        let d = detect_swap_pairs(&[(0x0010, 0x0011), (0x0011, 0x0012), (0x0012, 0x0010)]);
        assert_eq!(d.cycles.len(), 1);
        assert!(d.unclaimed.is_empty());
    }

    #[test]
    fn a_cycle_through_caps_and_ctrl_wins_over_the_caps_extra_ctrl_preset() {
        let d = detect_swap_pairs(&[(CAPS, LCTRL), (LCTRL, MUH), (MUH, CAPS)]);
        assert!(!d.caps_extra_ctrl);
        assert_eq!(d.cycles.len(), 1);
        assert!(d.unclaimed.is_empty());
        // 単独の Caps→左 Ctrl は、これまでどおりプリセット。
        let d = detect_swap_pairs(&[(CAPS, LCTRL)]);
        assert!(d.caps_extra_ctrl && d.cycles.is_empty());
    }

    #[test]
    fn creating_keeping_editing_and_removing_a_cycle() {
        let c1 = Cycle::new(&[MUH, HEN, SPC]).unwrap();
        // 作る。
        let plan =
            compute_swap_write_with_cycles(&[], &[], std::slice::from_ref(&c1), false).unwrap();
        let mut e = plan.entries.clone();
        e.sort_unstable();
        let mut want = c1.entries();
        want.sort_unstable();
        assert_eq!(e, want);
        // 何も変えない呼び出しは恒等。
        let same = compute_swap_write_with_cycles(&plan.entries, &[], &[c1], false).unwrap();
        assert_eq!(same.entries, plan.entries);
        // 向きを変えた循環に置き換える: 元のエントリは消え、新しいエントリが入る。
        let c2 = Cycle::new(&[MUH, SPC, HEN]).unwrap();
        let edited =
            compute_swap_write_with_cycles(&plan.entries, &[], std::slice::from_ref(&c2), false)
                .unwrap();
        let mut got = edited.entries;
        got.sort_unstable();
        let mut want = c2.entries();
        want.sort_unstable();
        assert_eq!(got, want);
        // 削除は常に通る。
        let removed = compute_swap_write_with_cycles(&plan.entries, &[], &[], false).unwrap();
        assert!(removed.entries.is_empty());
        // ペアに縮める（3巡回 → ペア）。
        let to_pair =
            compute_swap_write_with_cycles(&plan.entries, &[Pair::new(MUH, HEN)], &[], false)
                .unwrap();
        let mut got = to_pair.entries;
        got.sort_unstable();
        let mut want = Pair::new(MUH, HEN).entries().to_vec();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    #[test]
    fn a_new_cycle_gets_the_new_unit_checks() {
        // 許可リスト外のキーを含む新しい循環は作れない。
        let bad = Cycle::new(&[MUH, HEN, 0x0010]).unwrap();
        assert_eq!(
            compute_swap_write_with_cycles(&[], &[], &[bad], false),
            Err(SwapError::NotAllowed { key: 0x0010 })
        );
        // Caps 追加 Ctrl と Caps/左 Ctrl を含む循環は同居できない。
        let with_caps = Cycle::new(&[CAPS, MUH, HEN]).unwrap();
        assert_eq!(
            compute_swap_write_with_cycles(&[], &[], &[with_caps], true),
            Err(SwapError::CapsExtraConflict { key: CAPS })
        );
        // 同じキーがペアと循環の両方に現れてはならない。
        let c = Cycle::new(&[MUH, HEN, SPC]).unwrap();
        assert_eq!(
            compute_swap_write_with_cycles(&[], &[Pair::new(MUH, LALT)], &[c], false),
            Err(SwapError::DuplicateKey { key: MUH })
        );
        // 他ツールのエントリの to と同じ機能を出すエントリは新たに作れない。
        let c = Cycle::new(&[MUH, HEN, SPC]).unwrap();
        assert!(matches!(
            compute_swap_write_with_cycles(&[(LALT, HEN)], &[], &[c], false),
            Err(SwapError::CollidesWithForeignTarget { .. })
        ));
    }

    #[test]
    fn an_existing_cycle_outside_the_allowlist_can_be_kept_and_removed() {
        let existing = [(0x0010, 0x0011), (0x0011, 0x0012), (0x0012, 0x0010)];
        let d = detect_swap_pairs(&existing);
        let same = compute_swap_write_with_cycles(&existing, &[], &d.cycles, false).unwrap();
        assert_eq!(same.entries, existing);
        let removed = compute_swap_write_with_cycles(&existing, &[], &[], false).unwrap();
        assert!(removed.entries.is_empty());
    }

    #[test]
    fn every_permutation_of_four_keys_round_trips_through_write_and_read() {
        let keys = [LALT, SPC, MUH, HEN];
        let mut checked = 0;
        for image in all_permutations(&keys) {
            let (pairs, cycles) = decompose(&keys, &image);
            let plan = compute_swap_write_with_cycles(&[], &pairs, &cycles, false)
                .expect("許可リスト内の置換は書ける");
            // 書いたエントリは、恒等でない位置ごとに1つだけで、「位置 → 機能」が σ と一致する。
            let non_fixed = keys.iter().zip(&image).filter(|(k, v)| k != v).count();
            assert_eq!(plan.entries.len(), non_fixed, "{image:?}");
            for (k, v) in keys.iter().zip(&image) {
                if k != v {
                    assert!(plan.entries.contains(&(*k, *v)), "{image:?}");
                }
            }
            // 読み戻すと、同じ単位（ペアと循環）に分類される。
            let read = detect_swap_pairs(&plan.entries);
            let mut read_pairs: Vec<Pair> = read.pairs.iter().map(|d| d.pair).collect();
            let mut want_pairs = pairs.clone();
            read_pairs.sort_unstable();
            want_pairs.sort_unstable();
            assert_eq!(read_pairs, want_pairs, "{image:?}");
            let mut read_cycles = read.cycles.clone();
            let mut want_cycles = cycles.clone();
            read_cycles.sort();
            want_cycles.sort();
            assert_eq!(read_cycles, want_cycles, "{image:?}");
            assert!(read.unclaimed.is_empty(), "{image:?}");
            // 何も変えない呼び出しは恒等、全削除は常に通る。
            let same =
                compute_swap_write_with_cycles(&plan.entries, &pairs, &cycles, false).unwrap();
            assert_eq!(same.entries, plan.entries, "{image:?}");
            let none = compute_swap_write_with_cycles(&plan.entries, &[], &[], false).unwrap();
            assert!(none.entries.is_empty(), "{image:?}");
            checked += 1;
        }
        assert_eq!(checked, 24);
    }

    #[test]
    fn caps_extra_ctrl_that_would_close_a_foreign_chain_is_rejected_not_reclassified() {
        // 他ツールの `左Ctrl→無変換`・`無変換→Caps`。ここへ `Caps→左Ctrl`(Caps を Ctrl としても使う)を足すと3つのエントリが
        // 閉じた循環になり、他ツールの2件が確認なしで awase の入れ替えに読み替わる（Opus コードレビュー S1）。
        let existing = [(LCTRL, MUH), (MUH, CAPS)];
        assert_eq!(
            compute_swap_write_with_cycles(&existing, &[], &[], true),
            Err(SwapError::Reclassified)
        );
        // オフのままなら、他ツールのエントリは保持される。
        let same = compute_swap_write_with_cycles(&existing, &[], &[], false).unwrap();
        assert_eq!(same.entries, existing);
        // 許可リスト外のキーを経由する鎖も同じ。
        let existing = [(LCTRL, 0x0010), (0x0010, CAPS)];
        assert_eq!(
            compute_swap_write_with_cycles(&existing, &[], &[], true),
            Err(SwapError::Reclassified)
        );
        // 閉じない鎖なら通る。
        let existing = [(LCTRL, MUH)];
        assert!(compute_swap_write_with_cycles(&existing, &[], &[], true).is_ok());
    }

    #[test]
    fn every_transition_between_permutations_of_four_keys_leaves_exactly_the_target() {
        let keys = [LALT, SPC, MUH, HEN];
        let perms = all_permutations(&keys);
        let mut checked = 0;
        for from in &perms {
            let (fp, fc) = decompose(&keys, from);
            let start = compute_swap_write_with_cycles(&[], &fp, &fc, false)
                .unwrap()
                .entries;
            for to in &perms {
                let (tp, tc) = decompose(&keys, to);
                let plan = compute_swap_write_with_cycles(&start, &tp, &tc, false)
                    .expect("許可リスト内の置換どうしの遷移は通る");
                let mut got = plan.entries.clone();
                got.sort_unstable();
                let mut want: Vec<Entry> = keys
                    .iter()
                    .zip(to)
                    .filter(|(k, v)| k != v)
                    .map(|(&k, &v)| (k, v))
                    .collect();
                want.sort_unstable();
                assert_eq!(got, want, "{from:?} -> {to:?}");
                assert!(plan.displaced.is_empty(), "{from:?} -> {to:?}");
                checked += 1;
            }
        }
        assert_eq!(checked, 24 * 24);
    }

    #[test]
    fn random_registries_and_permutation_requests_keep_the_invariants() {
        let keys = [LALT, SPC, MUH, HEN];
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let mut ok = 0;
        for _ in 0..30_000 {
            let mut existing = random_entries(&mut rng);
            existing.dedup();
            let before = detect_swap_pairs(&existing);
            let before_pairs: Vec<Pair> = before.pairs.iter().map(|d| d.pair).collect();
            // 何も変えない呼び出しは恒等、全削除は常に通る。
            let same = compute_swap_write_with_cycles(
                &existing,
                &before_pairs,
                &before.cycles,
                before.caps_extra_ctrl,
            )
            .expect("恒等は常に通る");
            assert_eq!(same.entries, existing);
            assert_every_kept_subset_is_ok(&existing, &before);
            assert!(compute_swap_write_with_cycles(&existing, &[], &[], false).is_ok());
            // 4キーのランダムな置換 + Caps 追加 Ctrl を要求する。通ったら、要求した単位と Caps 追加 Ctrl は読み戻せる。
            let mut image = keys.to_vec();
            for i in (1..image.len()).rev() {
                image.swap(i, rng.below(i + 1));
            }
            let (pairs, cycles) = decompose(&keys, &image);
            let caps = rng.below(2) == 0;
            let Ok(plan) = compute_swap_write_with_cycles(&existing, &pairs, &cycles, caps) else {
                continue;
            };
            ok += 1;
            let after = detect_swap_pairs(&plan.entries);
            let got_pairs: Vec<Pair> = after.pairs.iter().map(|d| d.pair).collect();
            for p in &pairs {
                assert!(got_pairs.contains(p), "{existing:?} {image:?}");
            }
            for c in &cycles {
                assert!(after.cycles.contains(c), "{existing:?} {image:?}");
            }
            assert!(
                !caps || after.caps_extra_ctrl,
                "{existing:?} {image:?} caps"
            );
        }
        assert!(ok > 5_000, "{ok}");
    }

    /// 既存の単位（ペア・循環）の任意の部分集合を残す要求（Caps 追加 Ctrl は、あるならそのまま残すかオフ）は、必ず通る。
    /// 削除で他のエントリが露出して読み替わっても、書き込みは止めない（ADR-230 の `revealed` の扱い）。
    fn assert_every_kept_subset_is_ok(existing: &[Entry], before: &Detected) {
        let pairs: Vec<Pair> = before.pairs.iter().map(|d| d.pair).collect();
        let n = pairs.len() + before.cycles.len();
        for mask in 0..(1u32 << n) {
            let keep = |i: usize| mask & (1 << i) != 0;
            let kept_pairs: Vec<Pair> = pairs
                .iter()
                .enumerate()
                .filter(|&(i, _)| keep(i))
                .map(|(_, &p)| p)
                .collect();
            let kept_cycles: Vec<Cycle> = before
                .cycles
                .iter()
                .enumerate()
                .filter(|&(i, _)| keep(pairs.len() + i))
                .map(|(_, c)| c.clone())
                .collect();
            let caps_options: &[bool] = if before.caps_extra_ctrl {
                &[true, false]
            } else {
                &[false]
            };
            for &caps in caps_options {
                assert!(
                    compute_swap_write_with_cycles(existing, &kept_pairs, &kept_cycles, caps)
                        .is_ok(),
                    "削除だけの要求が通らない: {existing:?} mask={mask:b} caps={caps}"
                );
            }
        }
    }

    #[test]
    fn removing_a_unit_is_always_allowed_even_when_it_exposes_other_entries() {
        // round2 B1 の q1: 「スペース」の戻す（スペースを含むペアを外す）と、他ツールのエントリ `無変換→スペース`・`スペース→無変換` が
        // 露出して先に別のペアに読まれ、残す `無変換⇄変換` が読めなくなる。それでも書き込みは通る（露出は revealed で扱う）。
        let existing = [
            (SPC, LALT),
            (LALT, SPC),
            (MUH, SPC),
            (SPC, MUH),
            (MUH, HEN),
            (HEN, MUH),
        ];
        let d = detect_swap_pairs(&existing);
        let kept: Vec<Pair> = d
            .pairs
            .iter()
            .map(|x| x.pair)
            .filter(|p| !p.contains(SPC))
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(compute_swap_write_with_cycles(&existing, &kept, &d.cycles, false).is_ok());
        assert_every_kept_subset_is_ok(&existing, &d);
        // q4: 「Caps を Ctrl としても使う」を残したまま `無変換⇄変換` を外すと、他ツールの2件と `Caps→左Ctrl` が閉じた循環として
        // 読める。残す側なので止めない（確認ダイアログの revealed に出る）。
        let existing = [
            (CAPS, LCTRL),
            (LCTRL, MUH),
            (MUH, CAPS),
            (MUH, HEN),
            (HEN, MUH),
        ];
        let d = detect_swap_pairs(&existing);
        assert!(d.caps_extra_ctrl);
        let plan = compute_swap_write_with_cycles(&existing, &[], &[], true).unwrap();
        assert!(!plan.revealed.is_empty());
        assert_every_kept_subset_is_ok(&existing, &d);
        // 新しくオンにするときだけ止める（S1）。
        let existing = [(LCTRL, MUH), (MUH, CAPS)];
        assert_eq!(
            compute_swap_write_with_cycles(&existing, &[], &[], true),
            Err(SwapError::Reclassified)
        );
    }
}
