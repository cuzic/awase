//! 昇格ワーカーがペア集合を適用するときの、レジストリ I/O を伴わない純粋な判断（ADR-230 決定5）。
//!
//! 非昇格側（`awase-settings` の UI）は [`ApplyRequest`] を [`ApplyRequest::to_spec`] で1つの引数文字列にして
//! 昇格プロセスへ渡し、昇格側は [`ApplyRequest::from_spec`] で復元して [`decide`] に通す。昇格側は渡された値を信用せず、
//! 許可リスト・自己ペア・重複・衝突を [`compute_swap_write`] で再検証する。
//!
//! 守ること（ADR-230 決定5）:
//! - **比較交換**: UI が読んだエントリ列（`expected`）と、書く直前に読み直した値が食い違ったら、書かずに戻る
//!   （ユーザーが見て確認した差分と、実際に書く差分を一致させる）。
//! - **壊れた既存値は上書きしない**: [`parse_entries_strict`] が `None`（形式が壊れている）なら書かない。
//! - **承認フラグ**: 他ツールのエントリを消す（`displaced`）ときは、UI が確認した旨（`allow_displace`）が無ければ書かない。
//! - **読み戻し検証と巻き戻し**: 書いたあと読み戻して一致しなければ、書く前の生のバイト列（無ければ値の削除）へ戻す。

use crate::scancode_pairs::{compute_swap_write, Entry, Pair};

/// 昇格ワーカーのプロセス終了コード。`0` が成功、それ以外は書き込みを行わなかった、または元へ戻した。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExit {
    /// 書き込み・読み戻し検証まで成功（または変更が無く何もしなかった）。
    Ok,
    /// 想定外の失敗（レジストリの読み書きエラーなど）。
    Failed,
    /// UI が読んだ状態と、書く直前の状態が食い違った（他ツールなどが変更した）。書いていない。
    Changed,
    /// 検査に失敗した（許可リスト外・重複・衝突など）。書いていない。
    Invalid,
    /// 他ツールのエントリを消す必要があるが、UI の承認が無い。書いていない。
    DisplaceNotApproved,
    /// 既存の値の形式が壊れていて、上書きすると失われる。書いていない。
    ExistingCorrupt,
    /// 書いたが読み戻しが一致せず、元の値へ戻した。
    RolledBack,
    /// 書いたが読み戻しが一致せず、元の値へ戻すことにも失敗した。レジストリの中身を読み直して確認すること。
    RollbackFailed,
    /// 引数が解釈できない。書いていない。
    BadArguments,
}

impl WorkerExit {
    /// プロセス終了コード。
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Ok => 0,
            Self::Failed => 1,
            Self::Changed => 2,
            Self::Invalid => 3,
            Self::DisplaceNotApproved => 4,
            Self::ExistingCorrupt => 5,
            Self::RolledBack => 6,
            Self::RollbackFailed => 7,
            Self::BadArguments => 8,
        }
    }

    /// 終了コードから復元する（未知の値は `None`）。
    #[must_use]
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Ok,
            1 => Self::Failed,
            2 => Self::Changed,
            3 => Self::Invalid,
            4 => Self::DisplaceNotApproved,
            5 => Self::ExistingCorrupt,
            6 => Self::RolledBack,
            7 => Self::RollbackFailed,
            8 => Self::BadArguments,
            _ => return None,
        })
    }
}

/// 昇格ワーカーへ渡す適用要求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    /// 編集後のペア集合の全体（差分ではない）。
    pub pairs: Vec<Pair>,
    /// Caps 追加 Ctrl（ADR-126）を有効にするか。
    pub caps_extra_ctrl: bool,
    /// UI が読んだエントリ列（比較交換の期待値）。
    pub expected: Vec<Entry>,
    /// UI が、他ツールのエントリが消えることを確認済み。
    pub allow_displace: bool,
}

/// [`ApplyRequest::from_spec`] が失敗した理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    /// 必須のキーが無い、または重複している。
    MissingOrDuplicateKey(&'static str),
    /// 未知のキー。
    UnknownKey(String),
    /// 値が解釈できない。
    BadValue(&'static str),
}

fn hex4(v: u16) -> String {
    format!("{v:04X}")
}

fn parse_hex4(s: &str) -> Option<u16> {
    if s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        u16::from_str_radix(s, 16).ok()
    } else {
        None
    }
}

fn parse_list<T>(value: &str, parse_item: impl Fn(&str) -> Option<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        return Some(Vec::new());
    }
    value.split(',').map(parse_item).collect()
}

fn set<T>(slot: &mut Option<T>, name: &'static str, parsed: Option<T>) -> Result<(), SpecError> {
    let value = parsed.ok_or(SpecError::BadValue(name))?;
    if slot.replace(value).is_some() {
        return Err(SpecError::MissingOrDuplicateKey(name));
    }
    Ok(())
}

impl ApplyRequest {
    /// 1つのコマンドライン引数へ直列化する。形式: `pairs=007B-0038,0079-0039;caps=0;expect=003A>001D,001D>003A;displace=0`
    /// （スキャンコードは常に4桁の16進。空の一覧は空文字列）。
    #[must_use]
    pub fn to_spec(&self) -> String {
        let pairs = self
            .pairs
            .iter()
            .map(|p| {
                let (a, b) = p.keys();
                format!("{}-{}", hex4(a), hex4(b))
            })
            .collect::<Vec<_>>()
            .join(",");
        let expect = self
            .expected
            .iter()
            .map(|&(from, to)| format!("{}>{}", hex4(from), hex4(to)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "pairs={pairs};caps={};expect={expect};displace={}",
            u8::from(self.caps_extra_ctrl),
            u8::from(self.allow_displace)
        )
    }

    /// [`Self::to_spec`] の逆。
    ///
    /// # Errors
    /// [`SpecError`] を参照。
    pub fn from_spec(spec: &str) -> Result<Self, SpecError> {
        let mut pairs = None;
        let mut caps = None;
        let mut expect = None;
        let mut displace = None;
        for field in spec.split(';') {
            let (key, value) = field.split_once('=').ok_or(SpecError::BadValue("field"))?;
            match key {
                "pairs" => set(&mut pairs, "pairs", parse_pairs(value))?,
                "caps" => set(&mut caps, "caps", parse_flag(value))?,
                "expect" => set(&mut expect, "expect", parse_entries(value))?,
                "displace" => set(&mut displace, "displace", parse_flag(value))?,
                other => return Err(SpecError::UnknownKey(other.to_string())),
            }
        }
        Ok(Self {
            pairs: pairs.ok_or(SpecError::MissingOrDuplicateKey("pairs"))?,
            caps_extra_ctrl: caps.ok_or(SpecError::MissingOrDuplicateKey("caps"))?,
            expected: expect.ok_or(SpecError::MissingOrDuplicateKey("expect"))?,
            allow_displace: displace.ok_or(SpecError::MissingOrDuplicateKey("displace"))?,
        })
    }
}

fn parse_flag(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn parse_pairs(value: &str) -> Option<Vec<Pair>> {
    parse_list(value, |item| {
        let (a, b) = item.split_once('-')?;
        Some(Pair::new(parse_hex4(a)?, parse_hex4(b)?))
    })
}

fn parse_entries(value: &str) -> Option<Vec<Entry>> {
    parse_list(value, |item| {
        let (from, to) = item.split_once('>')?;
        Some((parse_hex4(from)?, parse_hex4(to)?))
    })
}

/// Scancode Map の `REG_BINARY` 値を厳密にパースする。形式が壊れている（短い・エントリ数と長さが合わない・
/// null 終端が無い）なら `None`。`scancode_map::parse_entries` は壊れた値を空リストにするので、上書きの判断には使えない。
#[must_use]
pub fn parse_entries_strict(bytes: &[u8]) -> Option<Vec<Entry>> {
    if bytes.len() < 12 {
        return None;
    }
    let count = usize::try_from(u32::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11],
    ]))
    .ok()?;
    // count は null 終端エントリ自身を含む。
    if count == 0 || bytes.len() < 12 + count.checked_mul(4)? {
        return None;
    }
    let mut out = Vec::with_capacity(count - 1);
    for i in 0..count - 1 {
        let offset = 12 + i * 4;
        let to = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        let from = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
        if from == 0 && to == 0 {
            return None; // 途中の null 終端は壊れた値
        }
        out.push((from, to));
    }
    let end = 12 + (count - 1) * 4;
    if bytes[end..end + 4] != [0, 0, 0, 0] {
        return None; // 終端の null が無い
    }
    Some(out)
}

/// [`decide`] の判断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 書かずに終わる（拒否、または変更が無い）。
    Stop(WorkerExit),
    /// このエントリ列を書く。空なら値そのものを削除する。
    Write { entries: Vec<Entry> },
}

/// 書く直前に読み直した生の値 `existing_raw`（値が無ければ `None`）と要求から、書くか止めるかを決める。
#[must_use]
pub fn decide(existing_raw: Option<&[u8]>, request: &ApplyRequest) -> Decision {
    let existing = match existing_raw {
        // 値が無い、または長さ 0（失うものが無い）。
        None | Some([]) => Vec::new(),
        Some(bytes) => match parse_entries_strict(bytes) {
            Some(entries) => entries,
            None => return Decision::Stop(WorkerExit::ExistingCorrupt),
        },
    };
    if existing != request.expected {
        return Decision::Stop(WorkerExit::Changed);
    }
    let Ok(plan) = compute_swap_write(&existing, &request.pairs, request.caps_extra_ctrl) else {
        return Decision::Stop(WorkerExit::Invalid);
    };
    if !plan.displaced.is_empty() && !request.allow_displace {
        return Decision::Stop(WorkerExit::DisplaceNotApproved);
    }
    if plan.entries == existing {
        return Decision::Stop(WorkerExit::Ok);
    }
    Decision::Write {
        entries: plan.entries,
    }
}

/// 書いた直後に読み戻した生の値 `raw`（値が無ければ `None`）が、書いたエントリ列と一致するか（順序込み）。
/// 書いたエントリ列が空なら、値が無い（削除された）か、形式が正しく空であること。
#[must_use]
pub fn read_back_matches(written: &[Entry], raw: Option<&[u8]>) -> bool {
    raw.map_or(written.is_empty(), |bytes| {
        parse_entries_strict(bytes).is_some_and(|entries| entries == written)
    })
}

/// 書いた直後の読み戻しの分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadBack {
    /// 書いた値と一致した。
    Matches,
    /// 書いた値でも元の値でもない、正しく読める値だった。書き込みと読み戻しの間に他の書き手が書いたとみなし、
    /// 巻き戻さない（巻き戻すと他の書き手の変更を消す）。画面は今の値を読み直すこと。
    ForeignWrite,
    /// それ以外（値が無い・読めない・元の値のまま）。元へ戻す。
    Mismatch,
}

/// 書いた直後に読み戻した生の値 `raw` を、書いたエントリ列 `written` と書く前の生の値 `original` に照らして分類する。
#[must_use]
pub fn classify_read_back(
    written: &[Entry],
    original: Option<&[u8]>,
    raw: Option<&[u8]>,
) -> ReadBack {
    if read_back_matches(written, raw) {
        ReadBack::Matches
    } else if raw != original && raw.is_some_and(|bytes| parse_entries_strict(bytes).is_some()) {
        ReadBack::ForeignWrite
    } else {
        ReadBack::Mismatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scancode_map::{
        build_bytes, SCANCODE_CAPS_EISU as CAPS, SCANCODE_LEFT_CTRL as LCTRL,
    };
    use crate::scancode_pairs::{SCANCODE_LEFT_ALT as LALT, SCANCODE_MUHENKAN as MUH};

    fn request(pairs: &[Pair], caps: bool, expected: &[Entry], displace: bool) -> ApplyRequest {
        ApplyRequest {
            pairs: pairs.to_vec(),
            caps_extra_ctrl: caps,
            expected: expected.to_vec(),
            allow_displace: displace,
        }
    }

    fn raw(entries: &[Entry]) -> Option<Vec<u8>> {
        build_bytes(entries)
    }

    #[test]
    fn exit_codes_round_trip_and_are_distinct() {
        let all = [
            WorkerExit::Ok,
            WorkerExit::Failed,
            WorkerExit::Changed,
            WorkerExit::Invalid,
            WorkerExit::DisplaceNotApproved,
            WorkerExit::ExistingCorrupt,
            WorkerExit::RolledBack,
            WorkerExit::RollbackFailed,
            WorkerExit::BadArguments,
        ];
        for (i, e) in all.iter().enumerate() {
            assert_eq!(WorkerExit::from_code(e.code()), Some(*e));
            for other in &all[i + 1..] {
                assert_ne!(e.code(), other.code());
            }
        }
        assert_eq!(WorkerExit::from_code(99), None);
        assert_eq!(WorkerExit::Ok.code(), 0);
    }

    #[test]
    fn spec_round_trips() {
        let req = request(
            &[Pair::new(MUH, LALT), Pair::new(0xE038, 0x0039)],
            true,
            &[(CAPS, LCTRL), (LCTRL, 0x0099)],
            true,
        );
        let spec = req.to_spec();
        assert_eq!(
            spec,
            "pairs=0038-007B,0039-E038;caps=1;expect=003A>001D,001D>0099;displace=1"
        );
        assert_eq!(ApplyRequest::from_spec(&spec), Ok(req));
    }

    #[test]
    fn spec_with_empty_lists_round_trips() {
        let req = request(&[], false, &[], false);
        assert_eq!(req.to_spec(), "pairs=;caps=0;expect=;displace=0");
        assert_eq!(ApplyRequest::from_spec(&req.to_spec()), Ok(req));
    }

    #[test]
    fn spec_rejects_malformed_input() {
        for bad in [
            "",
            "pairs=;caps=0;expect=",
            "pairs=;caps=2;expect=;displace=0",
            "pairs=007B;caps=0;expect=;displace=0",
            "pairs=007B-00;caps=0;expect=;displace=0",
            "pairs=007B-00ZZ;caps=0;expect=;displace=0",
            "pairs=;caps=0;expect=003A-001D;displace=0",
            "pairs=;caps=0;expect=;displace=0;extra=1",
            "pairs=;pairs=;caps=0;expect=;displace=0",
            "pairs=;caps=0;expect=;displace=0;",
        ] {
            assert!(
                ApplyRequest::from_spec(bad).is_err(),
                "{bad:?} が通ってしまった"
            );
        }
    }

    #[test]
    fn strict_parse_accepts_wellformed_and_rejects_broken_values() {
        let bytes = raw(&[(CAPS, LCTRL), (LCTRL, CAPS)]).unwrap();
        assert_eq!(
            parse_entries_strict(&bytes),
            Some(vec![(CAPS, LCTRL), (LCTRL, CAPS)])
        );
        // 値が空の形式（count=1、null 終端のみ）は正しく空。
        let mut empty = vec![0u8; 8];
        empty.extend_from_slice(&1u32.to_le_bytes());
        empty.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(parse_entries_strict(&empty), Some(Vec::new()));
        // 壊れた値。
        assert_eq!(parse_entries_strict(&[]), None);
        assert_eq!(parse_entries_strict(&[0u8; 11]), None);
        let mut truncated = bytes.clone();
        truncated.truncate(truncated.len() - 2);
        assert_eq!(parse_entries_strict(&truncated), None);
        let mut no_terminator = bytes.clone();
        let n = no_terminator.len();
        no_terminator[n - 1] = 1;
        assert_eq!(parse_entries_strict(&no_terminator), None);
        let mut zero_count = bytes;
        zero_count[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_entries_strict(&zero_count), None);
    }

    #[test]
    fn writes_when_state_matches_and_plan_is_valid() {
        let existing = [(0x0010, 0x0011)];
        let req = request(&[Pair::new(MUH, LALT)], false, &existing, false);
        let bytes = raw(&existing).unwrap();
        let Decision::Write { entries } = decide(Some(&bytes), &req) else {
            panic!("書くはず");
        };
        assert_eq!(entries[0], (0x0010, 0x0011));
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn value_missing_is_the_same_as_empty() {
        let req = request(&[Pair::new(MUH, LALT)], false, &[], false);
        assert!(matches!(decide(None, &req), Decision::Write { .. }));
    }

    #[test]
    fn stops_with_changed_when_registry_differs_from_what_the_ui_read() {
        let req = request(&[Pair::new(MUH, LALT)], false, &[(0x0010, 0x0011)], false);
        let other = raw(&[(0x0010, 0x0022)]).unwrap();
        assert_eq!(
            decide(Some(&other), &req),
            Decision::Stop(WorkerExit::Changed)
        );
        // UI は「無い」と読んだが、書く直前には値がある。
        let req = request(&[Pair::new(MUH, LALT)], false, &[], false);
        assert_eq!(
            decide(Some(&other), &req),
            Decision::Stop(WorkerExit::Changed)
        );
    }

    #[test]
    fn zero_length_value_is_treated_as_empty_not_corrupt() {
        let req = request(&[Pair::new(MUH, LALT)], false, &[], false);
        assert!(matches!(decide(Some(&[]), &req), Decision::Write { .. }));
    }

    #[test]
    fn stops_with_existing_corrupt_and_never_overwrites_a_broken_value() {
        let req = request(&[Pair::new(MUH, LALT)], false, &[], false);
        assert_eq!(
            decide(Some(&[1, 2, 3]), &req),
            Decision::Stop(WorkerExit::ExistingCorrupt)
        );
    }

    #[test]
    fn stops_with_invalid_when_the_plan_is_rejected_again_on_the_elevated_side() {
        // 許可リスト外のキー（昇格側でも再検証する）。
        let req = request(&[Pair::new(0x0010, MUH)], false, &[], false);
        assert_eq!(decide(None, &req), Decision::Stop(WorkerExit::Invalid));
        // 他ツールのエントリと衝突する。
        let existing = [(0x0079, MUH)];
        let req = request(&[Pair::new(MUH, LALT)], false, &existing, false);
        let bytes = raw(&existing).unwrap();
        assert_eq!(
            decide(Some(&bytes), &req),
            Decision::Stop(WorkerExit::Invalid)
        );
    }

    #[test]
    fn displacing_foreign_entries_needs_the_ui_approval_flag() {
        let existing = [(MUH, 0x0039)];
        let bytes = raw(&existing).unwrap();
        let req = request(&[Pair::new(MUH, LALT)], false, &existing, false);
        assert_eq!(
            decide(Some(&bytes), &req),
            Decision::Stop(WorkerExit::DisplaceNotApproved)
        );
        let approved = request(&[Pair::new(MUH, LALT)], false, &existing, true);
        assert!(matches!(
            decide(Some(&bytes), &approved),
            Decision::Write { .. }
        ));
    }

    #[test]
    fn nothing_to_change_stops_with_ok_without_writing() {
        let existing = [(CAPS, LCTRL), (LCTRL, CAPS)];
        let bytes = raw(&existing).unwrap();
        let req = request(&[Pair::new(CAPS, LCTRL)], false, &existing, false);
        assert_eq!(decide(Some(&bytes), &req), Decision::Stop(WorkerExit::Ok));
    }

    #[test]
    fn removing_the_last_pair_writes_an_empty_list_which_means_delete() {
        let existing = [(CAPS, LCTRL), (LCTRL, CAPS)];
        let bytes = raw(&existing).unwrap();
        let req = request(&[], false, &existing, false);
        assert_eq!(
            decide(Some(&bytes), &req),
            Decision::Write {
                entries: Vec::new()
            }
        );
    }

    #[test]
    fn classify_read_back_separates_our_write_a_foreign_write_and_a_mismatch() {
        let original = raw(&[(0x0010, 0x0011)]);
        let written = [(MUH, LALT), (LALT, MUH)];
        let ours = raw(&written);
        let foreign = raw(&[(0x0010, 0x0022)]);
        assert_eq!(
            classify_read_back(&written, original.as_deref(), ours.as_deref()),
            ReadBack::Matches
        );
        // 書いた値でも元の値でもない、正しく読める値は他の書き手。
        assert_eq!(
            classify_read_back(&written, original.as_deref(), foreign.as_deref()),
            ReadBack::ForeignWrite
        );
        // 元の値のまま（書き込みが効いていない）・値が無い・壊れた値は巻き戻しへ。
        assert_eq!(
            classify_read_back(&written, original.as_deref(), original.as_deref()),
            ReadBack::Mismatch
        );
        assert_eq!(
            classify_read_back(&written, original.as_deref(), None),
            ReadBack::Mismatch
        );
        assert_eq!(
            classify_read_back(&written, original.as_deref(), Some(&[1, 2, 3])),
            ReadBack::Mismatch
        );
    }

    #[test]
    fn read_back_matches_compares_order_and_handles_delete() {
        let written = [(MUH, LALT), (LALT, MUH)];
        let bytes = raw(&written).unwrap();
        assert!(read_back_matches(&written, Some(&bytes)));
        assert!(!read_back_matches(
            &[(LALT, MUH), (MUH, LALT)],
            Some(&bytes)
        ));
        assert!(!read_back_matches(&written, None));
        assert!(read_back_matches(&[], None));
        assert!(!read_back_matches(&[], Some(&bytes)));
        assert!(!read_back_matches(&written, Some(&[1, 2, 3])));
    }
}
