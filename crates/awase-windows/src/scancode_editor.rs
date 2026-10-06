//! 設定画面の「キーの入れ替え」（Scancode Map のペア編集、ADR-230 段階4）の状態と判断を持つ純粋モデル。
//!
//! egui などの画面部品・レジストリ I/O・昇格は含まない（`awase-settings` が薄い接着コードで呼ぶ）。
//! 状態は「編集行」の並びと Caps 追加 Ctrl のチェックで、レジストリの現在値は [`EditorState::from_detected`] で行に直す。
//! 適用前の検査・確認の材料は [`EditorState::preview`] が返す（書き込む内容は [`compute_swap_write`] が決める）。

use crate::scancode_apply::WorkerExit;
use crate::scancode_map::{SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL};
use crate::scancode_pairs::{
    compute_swap_write, Detected, Entry, Pair, SwapError, WritePlan, ALLOWED_SCANCODES,
    SCANCODE_HANKAKU_ZENKAKU, SCANCODE_HENKAN, SCANCODE_KANA, SCANCODE_LEFT_ALT, SCANCODE_MUHENKAN,
    SCANCODE_RIGHT_ALT, SCANCODE_SPACE,
};

/// スキャンコードの表示名。許可リスト外（他ツールのエントリなど）は `0x003A` の形。
#[must_use]
pub fn key_label(scancode: u16) -> String {
    match scancode {
        SCANCODE_CAPS_EISU => "英数 / Caps".to_string(),
        SCANCODE_LEFT_CTRL => "左 Ctrl".to_string(),
        SCANCODE_LEFT_ALT => "左 Alt".to_string(),
        SCANCODE_RIGHT_ALT => "右 Alt".to_string(),
        SCANCODE_SPACE => "スペース".to_string(),
        SCANCODE_MUHENKAN => "無変換".to_string(),
        SCANCODE_HENKAN => "変換".to_string(),
        SCANCODE_KANA => "かな".to_string(),
        SCANCODE_HANKAKU_ZENKAKU => "半角/全角".to_string(),
        other => format!("0x{other:04X}"),
    }
}

/// JIS 配列にしか物理キーが無い（US 配列では候補に出さない）許可リストのキー。
#[must_use]
pub const fn is_jis_only(scancode: u16) -> bool {
    matches!(
        scancode,
        SCANCODE_MUHENKAN | SCANCODE_HENKAN | SCANCODE_KANA | SCANCODE_HANKAKU_ZENKAKU
    )
}

/// 新規にペアへ使えるキーの候補（許可リストの順）。US 配列では JIS 専用キーを除く。
#[must_use]
pub fn candidate_keys(jis: bool) -> Vec<u16> {
    ALLOWED_SCANCODES
        .iter()
        .copied()
        .filter(|&k| jis || !is_jis_only(k))
        .collect()
}

/// 「よくある入れ替え」のワンクリックボタン（所有者の決定 2026-10-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickPair {
    pub label: &'static str,
    pub a: u16,
    pub b: u16,
}

/// ワンクリックボタンの一覧。
pub const QUICK_PAIRS: [QuickPair; 2] = [
    QuickPair {
        label: "英数/Caps ⇄ 左 Ctrl",
        a: SCANCODE_CAPS_EISU,
        b: SCANCODE_LEFT_CTRL,
    },
    QuickPair {
        label: "変換 ⇄ スペース",
        a: SCANCODE_HENKAN,
        b: SCANCODE_SPACE,
    },
];

/// 行の左右。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// 編集行 1 つ（まだ片方しか選んでいない行もある）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Row {
    pub a: Option<u16>,
    pub b: Option<u16>,
}

impl Row {
    const fn uses(self, key: u16) -> bool {
        matches!(self.a, Some(a) if a == key) || matches!(self.b, Some(b) if b == key)
    }
}

/// ペア編集の状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorState {
    initial_pairs: Vec<Pair>,
    initial_caps: bool,
    rows: Vec<Row>,
    caps_extra: bool,
}

impl EditorState {
    /// レジストリの現在値の分類（[`crate::scancode_pairs::detect_swap_pairs`] の結果）から編集状態を作る。
    #[must_use]
    pub fn from_detected(detected: &Detected) -> Self {
        let pairs: Vec<Pair> = detected.pairs.iter().map(|d| d.pair).collect();
        let rows = pairs
            .iter()
            .map(|p| {
                let (a, b) = p.keys();
                Row {
                    a: Some(a),
                    b: Some(b),
                }
            })
            .collect();
        Self {
            initial_pairs: pairs,
            initial_caps: detected.caps_extra_ctrl,
            rows,
            caps_extra: detected.caps_extra_ctrl,
        }
    }

    /// 編集行。
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// 「Caps を追加の Ctrl にする」のチェック。
    #[must_use]
    pub const fn caps_extra(&self) -> bool {
        self.caps_extra
    }

    /// 空の行を足す。
    pub fn add_row(&mut self) {
        self.rows.push(Row::default());
    }

    /// 行を消す（範囲外は無視）。
    pub fn remove_row(&mut self, index: usize) {
        if index < self.rows.len() {
            self.rows.remove(index);
        }
    }

    /// 行の片側のキーを設定する（範囲外は無視）。候補にないキーは [`Self::candidates`] で選ばせないこと。
    pub fn set_key(&mut self, index: usize, side: Side, key: Option<u16>) {
        if let Some(row) = self.rows.get_mut(index) {
            match side {
                Side::A => row.a = key,
                Side::B => row.b = key,
            }
        }
    }

    /// ワンクリックボタンで行を足せるか（2つのキーのどちらも他の行で使われておらず、Caps 追加 Ctrl と衝突しない）。
    #[must_use]
    pub fn quick_available(&self, quick: &QuickPair) -> bool {
        let blocked = |k: u16| {
            self.rows.iter().any(|r| r.uses(k))
                || (self.caps_extra && (k == SCANCODE_CAPS_EISU || k == SCANCODE_LEFT_CTRL))
        };
        !blocked(quick.a) && !blocked(quick.b)
    }

    /// ワンクリックボタン。[`Self::quick_available`] のときだけ行を足して `true`。
    pub fn add_quick(&mut self, quick: &QuickPair) -> bool {
        if !self.quick_available(quick) {
            return false;
        }
        self.rows.push(Row {
            a: Some(quick.a),
            b: Some(quick.b),
        });
        true
    }

    /// 「Caps を追加の Ctrl にする」をオンにできるか（Caps か左 Ctrl を使う行があるとオンにできない）。
    #[must_use]
    pub fn caps_extra_available(&self) -> bool {
        !self
            .rows
            .iter()
            .any(|r| r.uses(SCANCODE_CAPS_EISU) || r.uses(SCANCODE_LEFT_CTRL))
    }

    /// チェックを変える。オンにできないとき（[`Self::caps_extra_available`] が偽）は変えずに `false`。
    pub fn set_caps_extra(&mut self, on: bool) -> bool {
        if on && !self.caps_extra_available() {
            return false;
        }
        self.caps_extra = on;
        true
    }

    /// 行 `index` の `side` のドロップダウンに出す候補。許可リスト（`jis` でない配列では JIS 専用キーを除く）から、
    /// 他の行・同じ行の反対側で使われているキーと、Caps 追加 Ctrl が有効なときの Caps/左 Ctrl を除く。
    /// 今そこに入っているキー（許可リスト外の既存ペアを含む）は常に含める。
    #[must_use]
    pub fn candidates(&self, index: usize, side: Side, jis: bool) -> Vec<u16> {
        let current = self.rows.get(index).and_then(|r| match side {
            Side::A => r.a,
            Side::B => r.b,
        });
        let other = self.rows.get(index).and_then(|r| match side {
            Side::A => r.b,
            Side::B => r.a,
        });
        let mut out: Vec<u16> = candidate_keys(jis)
            .into_iter()
            .filter(|&k| {
                Some(k) != other
                    && !self
                        .rows
                        .iter()
                        .enumerate()
                        .any(|(i, r)| i != index && r.uses(k))
                    && !(self.caps_extra && (k == SCANCODE_CAPS_EISU || k == SCANCODE_LEFT_CTRL))
            })
            .collect();
        if let Some(c) = current {
            if !out.contains(&c) {
                out.insert(0, c);
            }
        }
        out
    }

    /// 両側が選ばれている行のペア。
    #[must_use]
    pub fn pairs(&self) -> Vec<Pair> {
        self.rows
            .iter()
            .filter_map(|r| Some(Pair::new(r.a?, r.b?)))
            .collect()
    }

    /// 片方しか選んでいない行がある（適用できない）。
    #[must_use]
    pub fn has_incomplete(&self) -> bool {
        self.rows.iter().any(|r| r.a.is_some() != r.b.is_some())
    }

    /// 読み込み時から変わっているか（空の行は無視する）。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        let mut now = self.pairs();
        let mut then = self.initial_pairs.clone();
        now.sort_unstable();
        then.sort_unstable();
        now != then || self.caps_extra != self.initial_caps
    }

    /// 読み込み時の状態へ戻す（行は初期のペアから作り直す）。
    pub fn reset(&mut self) {
        self.rows = self
            .initial_pairs
            .iter()
            .map(|p| {
                let (a, b) = p.keys();
                Row {
                    a: Some(a),
                    b: Some(b),
                }
            })
            .collect();
        self.caps_extra = self.initial_caps;
    }

    /// 適用前の検査と確認の材料。`existing` は UI が読んだ生のエントリ列（比較交換の期待値と同じもの）、
    /// `thumb_scancodes` は設定の親指キーに当たるスキャンコード。
    #[must_use]
    pub fn preview(&self, existing: &[Entry], thumb_scancodes: &[u16]) -> Preview {
        let pairs = self.pairs();
        Preview {
            cautions: cautions(&pairs, thumb_scancodes),
            plan: compute_swap_write(existing, &pairs, self.caps_extra),
        }
    }
}

/// 注意が要る組（適用時の確認ダイアログに出す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caution {
    /// 英数/Caps と親指キーのペア。英数位置の二重人格（Shift+英数 = CapsLock）が親指キーの位置へ移る。
    CapsWithThumbKey { thumb: u16 },
    /// 親指キーに設定されているキーを含む。親指シフトの物理位置が動く。
    ThumbKeyMoved { thumb: u16 },
    /// スペースを含む。スペースでの変換操作が入れ替え先のキーへ移る。
    SpaceMoved,
}

/// ペアの集合から、注意が要る組を挙げる（重複なし、出現順）。
#[must_use]
pub fn cautions(pairs: &[Pair], thumb_scancodes: &[u16]) -> Vec<Caution> {
    let mut out: Vec<Caution> = Vec::new();
    let mut push = |c: Caution| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    for pair in pairs {
        let (a, b) = pair.keys();
        for (key, other) in [(a, b), (b, a)] {
            if thumb_scancodes.contains(&key) {
                if other == SCANCODE_CAPS_EISU {
                    push(Caution::CapsWithThumbKey { thumb: key });
                } else {
                    push(Caution::ThumbKeyMoved { thumb: key });
                }
            }
        }
        if a == SCANCODE_SPACE || b == SCANCODE_SPACE {
            push(Caution::SpaceMoved);
        }
    }
    out
}

/// 確認ダイアログの注意書き（1項目1行）。
#[must_use]
pub fn caution_text(caution: Caution) -> String {
    match caution {
        Caution::CapsWithThumbKey { thumb } => format!(
            "「英数 / Caps」と、親指キーに設定している「{}」を入れ替えます。英数キーは Shift と一緒に押すと CapsLock になるため、\
             その動きが「{}」の位置へ移り、親指キーと Shift を一緒に押すと CapsLock が点灯します。",
            key_label(thumb),
            key_label(thumb)
        ),
        Caution::ThumbKeyMoved { thumb } => format!(
            "親指キーに設定している「{}」を入れ替えます。親指シフトを押す物理的な位置が変わります。",
            key_label(thumb)
        ),
        Caution::SpaceMoved => "「スペース」を入れ替えます。スペースで変換する操作が、入れ替え先のキーに移ります。".to_string(),
    }
}

/// 適用前の検査結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// 書き込む内容、または書き込めない理由。
    pub plan: Result<WritePlan, SwapError>,
    /// 注意が要る組。
    pub cautions: Vec<Caution>,
}

impl Preview {
    /// 確認ダイアログが要るか（注意が要る組・他ツールのエントリが消える・隠れたエントリが効き出しうる）。
    #[must_use]
    pub fn needs_confirmation(&self) -> bool {
        !self.cautions.is_empty()
            || self
                .plan
                .as_ref()
                .is_ok_and(|p| !p.displaced.is_empty() || !p.revealed.is_empty())
    }
}

/// 確認ダイアログに出す文面（1項目1行）。書き込めない場合は空。
#[must_use]
pub fn confirmation_lines(preview: &Preview) -> Vec<String> {
    let mut lines: Vec<String> = preview.cautions.iter().map(|&c| caution_text(c)).collect();
    if let Ok(plan) = &preview.plan {
        for &(from, to) in &plan.displaced {
            lines.push(format!(
                "他のツールが設定した「{} → {}」を消します。",
                key_label(from),
                key_label(to)
            ));
        }
        for &(from, to) in &plan.revealed {
            lines.push(format!(
                "他のツールが設定した「{} → {}」が、これまで隠れていたため、効き出す可能性があります。",
                key_label(from),
                key_label(to)
            ));
        }
    }
    lines
}

/// 書き込めない理由の利用者向け文言。
#[must_use]
pub fn swap_error_text(error: SwapError) -> String {
    match error {
        SwapError::SelfPair { key } => format!("「{}」を自分自身と入れ替えることはできません。", key_label(key)),
        SwapError::DuplicateKey { key } => format!("「{}」が複数の入れ替えで使われています。", key_label(key)),
        SwapError::NotAllowed { key } => format!("「{}」は入れ替えに使えないキーです。", key_label(key)),
        SwapError::CapsExtraConflict { key } => format!(
            "「Caps を追加の Ctrl にする」と同時に、「{}」の入れ替えは作れません。",
            key_label(key)
        ),
        SwapError::CollidesWithForeignTarget { entry: (from, to) } => format!(
            "他のツールの設定と重なります（「{} → {}」を作ると、同じキーを出すキーが2つになります）。",
            key_label(from),
            key_label(to)
        ),
    }
}

/// 昇格側の終了コードの利用者向け文言（成功は呼ばない）。
#[must_use]
pub const fn worker_exit_text(exit: WorkerExit) -> &'static str {
    match exit {
        WorkerExit::Ok => "適用しました。",
        WorkerExit::Failed => "処理に失敗しました。",
        WorkerExit::Changed => {
            "読み込んだあとに、設定が他のツールなどで変更されました。今の状態を読み直しました。もう一度操作してください。"
        }
        WorkerExit::Invalid => "この組み合わせは適用できません。",
        WorkerExit::DisplaceNotApproved => "他のツールの設定を消す確認が取れていないため、適用しませんでした。",
        WorkerExit::ExistingCorrupt => {
            "レジストリの Scancode Map の形式が壊れているため、変更できません。手動で削除するか、他のツールで直してください。"
        }
        WorkerExit::RolledBack => "書き込みを確認できなかったため、元の設定へ戻しました。",
        WorkerExit::RollbackFailed => {
            "書き込みを確認できず、元へ戻すことにも失敗しました。今の状態を読み直しているので、内容を確認してください。"
        }
        WorkerExit::BadArguments => "内部エラーです（引数を解釈できませんでした）。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scancode_pairs::detect_swap_pairs;

    const CAPS: u16 = SCANCODE_CAPS_EISU;
    const LCTRL: u16 = SCANCODE_LEFT_CTRL;
    const MUH: u16 = SCANCODE_MUHENKAN;
    const HEN: u16 = SCANCODE_HENKAN;
    const SPC: u16 = SCANCODE_SPACE;
    const LALT: u16 = SCANCODE_LEFT_ALT;

    fn editor(entries: &[Entry]) -> EditorState {
        EditorState::from_detected(&detect_swap_pairs(entries))
    }

    #[test]
    fn labels_cover_the_allowed_keys_and_fall_back_to_hex() {
        for &k in ALLOWED_SCANCODES {
            assert!(!key_label(k).starts_with("0x"), "{k:04X}");
        }
        assert_eq!(key_label(0x0010), "0x0010");
        assert_eq!(key_label(0xE05B), "0xE05B");
    }

    #[test]
    fn candidates_drop_jis_only_keys_for_non_jis_layouts() {
        let jis = candidate_keys(true);
        let us = candidate_keys(false);
        assert_eq!(jis.len(), ALLOWED_SCANCODES.len());
        for k in [MUH, HEN, SCANCODE_KANA, SCANCODE_HANKAKU_ZENKAKU] {
            assert!(jis.contains(&k));
            assert!(!us.contains(&k));
        }
        assert!(us.contains(&SPC) && us.contains(&CAPS) && us.contains(&SCANCODE_RIGHT_ALT));
    }

    #[test]
    fn existing_pairs_become_rows_and_untouched_state_is_not_dirty() {
        let e = editor(&[(CAPS, LCTRL), (LCTRL, CAPS)]);
        assert_eq!(e.rows().len(), 1);
        assert!(!e.is_dirty());
        assert!(!e.caps_extra());
        let e = editor(&[(CAPS, LCTRL)]);
        assert!(e.caps_extra());
        assert!(e.rows().is_empty());
        assert!(!e.is_dirty());
    }

    #[test]
    fn candidates_exclude_keys_used_elsewhere_but_keep_the_current_one() {
        let mut e = editor(&[]);
        e.add_row();
        e.add_row();
        e.set_key(0, Side::A, Some(MUH));
        e.set_key(0, Side::B, Some(LALT));
        // 行1の候補に、行0で使っているキーは出ない。
        let c = e.candidates(1, Side::A, true);
        assert!(!c.contains(&MUH) && !c.contains(&LALT));
        // 行0のA側の候補には、今の無変換が含まれ、反対側の左 Alt は含まれない。
        let c = e.candidates(0, Side::A, true);
        assert!(c.contains(&MUH) && !c.contains(&LALT));
    }

    #[test]
    fn candidates_keep_a_current_key_outside_the_allowlist() {
        let e = editor(&[(0x0010, 0x0011), (0x0011, 0x0010)]);
        let c = e.candidates(0, Side::A, true);
        assert_eq!(c.first().copied(), Some(0x0010));
    }

    #[test]
    fn caps_extra_is_exclusive_with_rows_using_caps_or_left_ctrl() {
        let mut e = editor(&[]);
        assert!(e.add_quick(&QUICK_PAIRS[0]));
        assert!(!e.caps_extra_available());
        assert!(!e.set_caps_extra(true));
        e.remove_row(0);
        assert!(e.set_caps_extra(true));
        // オンのとき、Caps/左 Ctrl は候補から消え、ワンクリックでも足せない。
        e.add_row();
        let c = e.candidates(0, Side::A, true);
        assert!(!c.contains(&CAPS) && !c.contains(&LCTRL));
        assert!(!e.add_quick(&QUICK_PAIRS[0]));
        assert!(e.add_quick(&QUICK_PAIRS[1]));
    }

    #[test]
    fn quick_pair_is_rejected_when_a_key_is_already_used() {
        let mut e = editor(&[]);
        assert!(e.quick_available(&QUICK_PAIRS[1]));
        assert!(e.add_quick(&QUICK_PAIRS[1])); // 変換 ⇄ スペース
        assert!(!e.quick_available(&QUICK_PAIRS[1]));
        assert!(!e.add_quick(&QUICK_PAIRS[1]));
        assert_eq!(e.rows().len(), 1);
    }

    #[test]
    fn dirty_follows_the_pairs_not_the_row_order_or_empty_rows() {
        let mut e = editor(&[(MUH, LALT), (LALT, MUH), (HEN, SPC), (SPC, HEN)]);
        assert!(!e.is_dirty());
        e.add_row(); // 空の行は変更ではない
        assert!(!e.is_dirty());
        assert!(!e.has_incomplete());
        e.set_key(2, Side::A, Some(SCANCODE_KANA));
        assert!(e.has_incomplete());
        e.remove_row(0);
        assert!(e.is_dirty());
        e.reset();
        assert!(!e.is_dirty());
        assert_eq!(e.rows().len(), 2);
    }

    #[test]
    fn preview_reports_the_write_and_flags_cautions() {
        let existing = [(MUH, 0x0099)];
        let mut e = editor(&existing);
        assert!(e.add_quick(&QUICK_PAIRS[1])); // 変換 ⇄ スペース
        let thumb = [HEN];
        let p = e.preview(&existing, &thumb);
        let plan = p.plan.as_ref().expect("書ける");
        assert_eq!(plan.entries.len(), 3);
        assert_eq!(
            p.cautions,
            vec![Caution::ThumbKeyMoved { thumb: HEN }, Caution::SpaceMoved]
        );
        assert!(p.needs_confirmation());
    }

    #[test]
    fn preview_asks_for_confirmation_when_foreign_entries_are_displaced_or_revealed() {
        let existing = [(MUH, SPC)];
        let mut e = editor(&existing);
        e.add_row();
        e.set_key(0, Side::A, Some(MUH));
        e.set_key(0, Side::B, Some(LALT));
        let p = e.preview(&existing, &[]);
        assert!(p.cautions.is_empty());
        assert!(p.needs_confirmation());
        let lines = confirmation_lines(&p);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("無変換 → スペース"));
        // 何も確認が要らない入れ替え。
        let mut e = editor(&[]);
        e.add_row();
        e.set_key(0, Side::A, Some(MUH));
        e.set_key(0, Side::B, Some(LALT));
        assert!(!e.preview(&[], &[]).needs_confirmation());
    }

    #[test]
    fn caps_with_a_thumb_key_is_a_stronger_caution_than_a_moved_thumb_key() {
        let thumb = [MUH];
        let c = cautions(&[Pair::new(CAPS, MUH)], &thumb);
        assert_eq!(c, vec![Caution::CapsWithThumbKey { thumb: MUH }]);
        assert!(caution_text(c[0]).contains("CapsLock"));
        // 親指キーでないなら、Caps との入れ替えに警告は無い。
        assert!(cautions(&[Pair::new(CAPS, MUH)], &[]).is_empty());
    }

    #[test]
    fn preview_returns_the_error_when_the_plan_cannot_be_written() {
        let mut e = editor(&[]);
        e.add_row();
        e.set_key(0, Side::A, Some(0x0010)); // 許可リスト外
        e.set_key(0, Side::B, Some(MUH));
        let p = e.preview(&[], &[]);
        assert_eq!(p.plan, Err(SwapError::NotAllowed { key: 0x0010 }));
        assert!(confirmation_lines(&p).iter().all(|l| !l.is_empty()));
        assert!(swap_error_text(SwapError::NotAllowed { key: 0x0010 }).contains("0x0010"));
    }

    #[test]
    fn every_error_and_exit_has_a_user_facing_text() {
        for e in [
            SwapError::SelfPair { key: MUH },
            SwapError::DuplicateKey { key: MUH },
            SwapError::NotAllowed { key: MUH },
            SwapError::CapsExtraConflict { key: CAPS },
            SwapError::CollidesWithForeignTarget { entry: (MUH, HEN) },
        ] {
            assert!(!swap_error_text(e).is_empty());
        }
        for code in 0..=8 {
            let exit = WorkerExit::from_code(code).expect("定義済みの終了コード");
            assert!(!worker_exit_text(exit).is_empty());
        }
    }
}
