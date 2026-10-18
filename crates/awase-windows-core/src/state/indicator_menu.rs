//! ADR-264 段階 1: タスクバーの入力インジケーターの右クリックメニューを読んで、現在の入力モードを知る
//! **純粋な判定**（Win32 を呼ばない。Linux でテストできる）。
//!
//! メニューは IME 自身（TSF）が描く UI で、現在のモードの項目に UI Automation の `SelectionItemPattern.IsSelected`
//! が付く（CI の実測、ADR-264）。ここでは、読んだ項目（`AutomationId`・選択の有無）から、IME ごとの表で
//! **完全一致**したときだけ [`MenuReading`] を返す。**belief へは入れない**（段階 1 は journal に出すだけ。決定 5）。
//!
//! ## なぜ完全一致か
//!
//! Win32 メニューの `AutomationId` は `Item` + コマンド ID（コマンド ID を持たないサブメニュー項目は、区切り線を
//! 含む 1 始まりの位置）で、**IME 固有ではない**。実際、本物の MS-IME のメニューには `Item 10`（Add-on Dictionary）・
//! `Item 11`（Search）があり、GJI のモード項目（`Item 10`〜`15`）と衝突する。そこで、(1)呼び出し側が渡す IME の表で、
//! (2)モード項目の**全部**と、その IME 固有の非モード項目（マーカー）の**全部**が揃い、(3)ちょうど 1 つの
//! **モード項目**が選択されているときだけ採る。ほかのアプリの右クリックメニューや、別の IME の表で読んだメニューは `Miss`。
//!
//! ## 対応表の根拠
//!
//! [`Evidence::Observed`] は、静的に作った状態（IMM やキーで作った状態）でその項目が `IsSelected` になるのを CI で観測した行。
//! [`Evidence::Inferred`] は、項目名からの推定で、一度も静的に観測していない行（ADR-264 決定 3）。推定の行は、
//! 段階 1 の shadow で読める窓の直接の読みと突き合わせて一致を確かめるまで、belief の根拠にしてはならない。
//!
//! 本物の MS-IME は閉と開+半角英数（`0x00`）が同じ表示（`GlyphClass`、ADR-261）なので、`Item 105`（半角英数）は
//! **入力モード=半角英数、開閉は不明**（`open = None`）として読む。

use serde::Serialize;

/// 入力インジケーターのメニューを出す IME（段階 1 の対象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MenuIme {
    /// 本物の Microsoft IME
    MsIme,
    /// Google 日本語入力
    Gji,
}

impl MenuIme {
    /// journal・ログ用の短い名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MsIme => "ms-ime",
            Self::Gji => "gji",
        }
    }
}

/// メニューの項目が表す入力モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MenuMode {
    Hiragana,
    FullKatakana,
    FullAlnum,
    HalfKatakana,
    HalfAlnum,
    /// IME の閉（GJI の Direct Input）
    DirectInput,
}

impl MenuMode {
    /// journal・ログ用の短い名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hiragana => "hiragana",
            Self::FullKatakana => "full-katakana",
            Self::FullAlnum => "full-alnum",
            Self::HalfKatakana => "half-katakana",
            Self::HalfAlnum => "half-alnum",
            Self::DirectInput => "direct-input",
        }
    }
}

/// 対応表の行の根拠（ADR-264 決定 3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Evidence {
    /// 静的に作った状態で `IsSelected` になるのを CI で観測した
    Observed,
    /// 項目名からの推定（一度も静的に観測していない）
    Inferred,
}

impl Evidence {
    /// journal・ログ用の短い名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Inferred => "inferred",
        }
    }
}

/// メニューから読んだ 1 項目（`AutomationId` と選択の有無）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// UI Automation の `AutomationId`（例: `Item 101`・`Separator 6`）
    pub automation_id: String,
    /// `SelectionItemPattern.IsSelected`（パターンが無い項目は `false`）
    pub selected: bool,
}

/// `Item N` の `N`（コマンド ID またはサブメニュー項目の位置）。`Separator N` など別の形式は `None`。
#[must_use]
pub fn parse_command_id(automation_id: &str) -> Option<u32> {
    let n = automation_id.strip_prefix("Item ")?;
    // 先頭が数字で、全部が数字のときだけ（`12x`・`+1`・空は不可）。
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// 現在のモードの読み（完全一致したとき）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MenuReading {
    pub ime: MenuIme,
    pub mode: MenuMode,
    /// IME が開いているか。不明なら `None`（本物の MS-IME の半角英数は閉と同じ表示なので不明）
    pub open: Option<bool>,
    pub evidence: Evidence,
}

/// `Miss` の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MissReason {
    /// 項目が 1 つも無い（メニューが読めなかった）
    EmptyMenu,
    /// その IME のモード項目が全部は揃っていない（他のアプリのメニュー・別の IME の表で読んだメニュー）
    MissingModeItems,
    /// その IME 固有の非モード項目（マーカー）が全部は揃っていない
    MissingMarkerItems,
    /// 選択されている項目が無い
    NoSelection,
    /// 選択されている項目が 2 つ以上ある
    MultipleSelection,
    /// 選択されている項目がモード項目でない
    SelectedNotMode,
}

impl MissReason {
    /// journal・ログ用の短い名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::EmptyMenu => "empty-menu",
            Self::MissingModeItems => "missing-mode-items",
            Self::MissingMarkerItems => "missing-marker-items",
            Self::NoSelection => "no-selection",
            Self::MultipleSelection => "multiple-selection",
            Self::SelectedNotMode => "selected-not-mode",
        }
    }
}

/// 対応表の 1 行（モード項目）。
struct ModeRow {
    command_id: u32,
    mode: MenuMode,
    open: Option<bool>,
    evidence: Evidence,
}

const fn row(command_id: u32, mode: MenuMode, open: Option<bool>, evidence: Evidence) -> ModeRow {
    ModeRow {
        command_id,
        mode,
        open,
        evidence,
    }
}

/// 本物の MS-IME のモード項目（Windows Server 2025 build 26100、CI の実測）。
///
/// `105` は「入力モード=半角英数。開閉は不明」（閉と開+半角英数が同じ表示）。
const MSIME_MODES: [ModeRow; 5] = [
    row(101, MenuMode::Hiragana, Some(true), Evidence::Observed),
    row(102, MenuMode::FullKatakana, Some(true), Evidence::Observed),
    row(103, MenuMode::FullAlnum, Some(true), Evidence::Inferred),
    row(104, MenuMode::HalfKatakana, Some(true), Evidence::Inferred),
    row(105, MenuMode::HalfAlnum, None, Evidence::Observed),
];

/// 本物の MS-IME 固有の非モード項目。**`Item 536`（Troubleshoot problems）は新しい MS-IME で足された項目とみられ、
/// 古い build・Windows 10 には無い可能性があるので入れない**（ADR-264 決定 2。版をまたぐ確認は M0-5）。
const MSIME_MARKERS: [u32; 4] = [116, 118, 120, 122];

/// GJI のモード項目（CI の GJI）。`10`（Direct Input）は IME の閉。
const GJI_MODES: [ModeRow; 6] = [
    row(11, MenuMode::Hiragana, Some(true), Evidence::Observed),
    row(12, MenuMode::FullKatakana, Some(true), Evidence::Inferred),
    row(14, MenuMode::FullAlnum, Some(true), Evidence::Inferred),
    row(15, MenuMode::HalfKatakana, Some(true), Evidence::Inferred),
    row(13, MenuMode::HalfAlnum, Some(true), Evidence::Inferred),
    row(10, MenuMode::DirectInput, Some(false), Evidence::Observed),
];

/// GJI 固有の非モード項目（Dictionary Tool・Add Word・Properties・About・Help）。
const GJI_MARKERS: [u32; 5] = [20, 21, 22, 30, 31];

const fn table(ime: MenuIme) -> (&'static [ModeRow], &'static [u32]) {
    match ime {
        MenuIme::MsIme => (&MSIME_MODES, &MSIME_MARKERS),
        MenuIme::Gji => (&GJI_MODES, &GJI_MARKERS),
    }
}

/// メニューの項目から、現在のモードを読む。**IME の表で完全一致したときだけ** `Ok`。
///
/// # Errors
/// 完全一致しなかったとき、その理由を [`MissReason`] で返す（`Miss`）。
pub fn classify_menu(ime: MenuIme, items: &[MenuItem]) -> Result<MenuReading, MissReason> {
    if items.is_empty() {
        return Err(MissReason::EmptyMenu);
    }
    let (modes, markers) = table(ime);
    let has_command = |id: u32| {
        items
            .iter()
            .any(|it| parse_command_id(&it.automation_id) == Some(id))
    };
    if !modes.iter().all(|m| has_command(m.command_id)) {
        return Err(MissReason::MissingModeItems);
    }
    if !markers.iter().all(|&id| has_command(id)) {
        return Err(MissReason::MissingMarkerItems);
    }
    let mut selected = items.iter().filter(|it| it.selected);
    let first = selected.next().ok_or(MissReason::NoSelection)?;
    if selected.next().is_some() {
        return Err(MissReason::MultipleSelection);
    }
    let selected_id = parse_command_id(&first.automation_id);
    let row = modes
        .iter()
        .find(|m| Some(m.command_id) == selected_id)
        .ok_or(MissReason::SelectedNotMode)?;
    Ok(MenuReading {
        ime,
        mode: row.mode,
        open: row.open,
        evidence: row.evidence,
    })
}

/// 読み取りワーカー（別スレッド）がメインスレッドへ返す結果。`Win32` のメッセージ（`wParam`/`lParam`）に載せるため、
/// ヒープを使わない固定長の符号化を持つ（[`MenuOutcome::pack`]・[`MenuOutcome::unpack`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuOutcome {
    pub ime: MenuIme,
    pub result: Result<MenuReading, MissReason>,
    /// 読んだ `MenuItem` の数（`u16` に飽和）
    pub item_count: u16,
    /// 読み取りの所要（ミリ秒。`u15` に飽和）
    pub read_ms: u16,
}

const MODE_ALL: [MenuMode; 6] = [
    MenuMode::Hiragana,
    MenuMode::FullKatakana,
    MenuMode::FullAlnum,
    MenuMode::HalfKatakana,
    MenuMode::HalfAlnum,
    MenuMode::DirectInput,
];

const MISS_ALL: [MissReason; 6] = [
    MissReason::EmptyMenu,
    MissReason::MissingModeItems,
    MissReason::MissingMarkerItems,
    MissReason::NoSelection,
    MissReason::MultipleSelection,
    MissReason::SelectedNotMode,
];

/// `read_ms` の上限（`lParam` の下位 16 bit の最上位を符号に使わないため 15 bit）。
const READ_MS_MAX: u16 = 0x7FFF;

impl MenuOutcome {
    /// 符号化（`wParam`, `lParam`）。32 bit 環境でも収まるように、`wParam` は 12 bit、`lParam` は 31 bit だけ使う。
    ///
    /// - `wParam`: bit0 = IME（0=MS-IME・1=GJI）、bit1 = 結果（0=Hit・1=Miss）、bit2〜4 = モード/理由の番号、
    ///   bit5〜6 = 開閉（0=閉・1=開・2=不明、Hit のみ）、bit7 = 根拠（0=観測済み・1=推定、Hit のみ）
    /// - `lParam`: 下位 16 bit = 項目数、その上 15 bit = 読み取りの所要（ms）
    #[must_use]
    pub fn pack(&self) -> (usize, isize) {
        let ime_bit = usize::from(matches!(self.ime, MenuIme::Gji));
        let w = match self.result {
            Ok(r) => {
                let mode = MODE_ALL.iter().position(|m| *m == r.mode).unwrap_or(0);
                let open = match r.open {
                    Some(false) => 0usize,
                    Some(true) => 1,
                    None => 2,
                };
                let ev = usize::from(matches!(r.evidence, Evidence::Inferred));
                ime_bit | (mode << 2) | (open << 5) | (ev << 7)
            }
            Err(reason) => {
                let idx = MISS_ALL.iter().position(|m| *m == reason).unwrap_or(0);
                ime_bit | (1 << 1) | (idx << 2)
            }
        };
        let ms = self.read_ms.min(READ_MS_MAX);
        let l = usize::from(self.item_count) | (usize::from(ms) << 16);
        // `l` は最大 31 bit なので、32 bit 環境の `isize` でも負にならない。
        #[allow(clippy::cast_possible_wrap)]
        let l = l as isize;
        (w, l)
    }

    /// `pack` の逆。範囲外の値（壊れたメッセージ）は `None`。
    #[must_use]
    pub fn unpack(wparam: usize, lparam: isize) -> Option<Self> {
        if wparam >> 8 != 0 || lparam < 0 {
            return None;
        }
        let ime = if wparam & 1 == 0 {
            MenuIme::MsIme
        } else {
            MenuIme::Gji
        };
        let index = (wparam >> 2) & 0b111;
        let result = if (wparam >> 1) & 1 == 1 {
            if (wparam >> 5) != 0 {
                return None;
            }
            Err(*MISS_ALL.get(index)?)
        } else {
            let open = match (wparam >> 5) & 0b11 {
                0 => Some(false),
                1 => Some(true),
                2 => None,
                _ => return None,
            };
            let evidence = if (wparam >> 7) & 1 == 1 {
                Evidence::Inferred
            } else {
                Evidence::Observed
            };
            Ok(MenuReading {
                ime,
                mode: *MODE_ALL.get(index)?,
                open,
                evidence,
            })
        };
        #[allow(clippy::cast_sign_loss)] // 上で `lparam >= 0` を確かめた
        let l = lparam as usize;
        let item_count = u16::try_from(l & 0xFFFF).ok()?;
        let read_ms = u16::try_from((l >> 16) & 0x7FFF).ok()?;
        Some(Self {
            ime,
            result,
            item_count,
            read_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Item N` の項目列を作る。`selected` に入れた ID の項目だけ選択済みにする。
    fn menu(ids: &[&str], selected: &[&str]) -> Vec<MenuItem> {
        ids.iter()
            .map(|id| MenuItem {
                automation_id: (*id).to_owned(),
                selected: selected.contains(id),
            })
            .collect()
    }

    /// CI（windows-latest、MS-IME 本体）で読んだメニュー窓配下の項目 ID（区切り線を除く 16 件）と区切り線。
    const MSIME_IDS: [&str; 19] = [
        "Item 101",
        "Item 102",
        "Item 103",
        "Item 104",
        "Item 105",
        "Separator 6",
        "Item 118",
        "Item 120",
        "Item 122",
        "Item 10",
        "Item 11",
        "Item 124",
        "Item 116",
        "Separator 14",
        "Item 15",
        "Item 16",
        "Item 17",
        "Separator 18",
        "Item 536",
    ];

    /// CI（GJI）で読んだメニュー窓配下の項目 ID（11 件）と区切り線。
    const GJI_IDS: [&str; 13] = [
        "Item 11",
        "Item 12",
        "Item 14",
        "Item 15",
        "Item 13",
        "Item 10",
        "Separator 7",
        "Item 21",
        "Item 22",
        "Item 20",
        "Separator 11",
        "Item 31",
        "Item 30",
    ];

    fn msime(selected: &str) -> Vec<MenuItem> {
        menu(&MSIME_IDS, &[selected])
    }

    fn gji(selected: &str) -> Vec<MenuItem> {
        menu(&GJI_IDS, &[selected])
    }

    // ---- parse_command_id ----

    #[test]
    fn parse_command_id_reads_only_item_numbers() {
        assert_eq!(parse_command_id("Item 101"), Some(101));
        assert_eq!(parse_command_id("Item 0"), Some(0));
        assert_eq!(parse_command_id("Separator 6"), None);
        assert_eq!(parse_command_id("Item"), None);
        assert_eq!(parse_command_id("Item "), None);
        assert_eq!(parse_command_id("Item 12x"), None);
        assert_eq!(parse_command_id("Item +1"), None);
        assert_eq!(parse_command_id("Item -1"), None);
        assert_eq!(parse_command_id("item 1"), None);
        assert_eq!(parse_command_id(" Item 1"), None);
        assert_eq!(parse_command_id(""), None);
        // u32 を超える数は読めない
        assert_eq!(parse_command_id("Item 99999999999999999999"), None);
    }

    // ---- 実測した状態(Observed) ----

    #[test]
    fn msime_hiragana_is_observed_open() {
        let r = classify_menu(MenuIme::MsIme, &msime("Item 101")).unwrap();
        assert_eq!(r.mode, MenuMode::Hiragana);
        assert_eq!(r.open, Some(true));
        assert_eq!(r.evidence, Evidence::Observed);
    }

    #[test]
    fn msime_katakana_is_observed_open() {
        let r = classify_menu(MenuIme::MsIme, &msime("Item 102")).unwrap();
        assert_eq!(r.mode, MenuMode::FullKatakana);
        assert_eq!(r.open, Some(true));
        assert_eq!(r.evidence, Evidence::Observed);
    }

    /// 本物の MS-IME は閉と開+半角英数が同じ表示。`105` を「閉」と読んではならない（開閉は不明）。
    #[test]
    fn msime_half_alnum_leaves_open_unknown() {
        let r = classify_menu(MenuIme::MsIme, &msime("Item 105")).unwrap();
        assert_eq!(r.mode, MenuMode::HalfAlnum);
        assert_eq!(r.open, None, "半角英数=閉とは読まない");
        assert_eq!(r.evidence, Evidence::Observed);
    }

    #[test]
    fn gji_hiragana_and_direct_input_are_observed() {
        let on = classify_menu(MenuIme::Gji, &gji("Item 11")).unwrap();
        assert_eq!((on.mode, on.open), (MenuMode::Hiragana, Some(true)));
        assert_eq!(on.evidence, Evidence::Observed);
        let off = classify_menu(MenuIme::Gji, &gji("Item 10")).unwrap();
        assert_eq!((off.mode, off.open), (MenuMode::DirectInput, Some(false)));
        assert_eq!(off.evidence, Evidence::Observed);
    }

    // ---- 推定の行(Inferred) ----

    #[test]
    fn rows_never_observed_statically_are_marked_inferred() {
        for (ime, ids, selected) in [
            (MenuIme::MsIme, &MSIME_IDS[..], "Item 103"),
            (MenuIme::MsIme, &MSIME_IDS[..], "Item 104"),
            (MenuIme::Gji, &GJI_IDS[..], "Item 12"),
            (MenuIme::Gji, &GJI_IDS[..], "Item 14"),
            (MenuIme::Gji, &GJI_IDS[..], "Item 15"),
            (MenuIme::Gji, &GJI_IDS[..], "Item 13"),
        ] {
            let r = classify_menu(ime, &menu(ids, &[selected])).unwrap();
            assert_eq!(r.evidence, Evidence::Inferred, "{ime:?} {selected}");
        }
    }

    // ---- 完全一致(誤同定を避ける) ----

    /// MS-IME のメニューには `Item 10`・`Item 11`(Add-on Dictionary・Search)があり、GJI のモード項目 ID と衝突する。
    /// MS-IME のメニューを GJI の表で読むと Miss(`12`・`13`・`14` が無い)。
    #[test]
    fn msime_menu_read_with_gji_table_is_a_miss() {
        assert_eq!(
            classify_menu(MenuIme::Gji, &msime("Item 11")),
            Err(MissReason::MissingModeItems)
        );
    }

    #[test]
    fn gji_menu_read_with_msime_table_is_a_miss() {
        assert_eq!(
            classify_menu(MenuIme::MsIme, &gji("Item 11")),
            Err(MissReason::MissingModeItems)
        );
    }

    /// ほかのアプリの右クリックメニュー（コマンド ID が偶然 GJI のモード項目に当たる）は採らない。
    #[test]
    fn another_apps_menu_is_a_miss_even_if_ids_collide() {
        let other = menu(
            &[
                "Item 10", "Item 11", "Item 12", "Item 13", "Item 14", "Item 15", "Item 100",
            ],
            &["Item 11"],
        );
        // モード項目は揃うが、GJI 固有の非モード項目(20/21/22/30/31)が無い
        assert_eq!(
            classify_menu(MenuIme::Gji, &other),
            Err(MissReason::MissingMarkerItems)
        );
        // MS-IME の表では、モード項目(101〜105)が無い
        assert_eq!(
            classify_menu(MenuIme::MsIme, &other),
            Err(MissReason::MissingModeItems)
        );
    }

    #[test]
    fn missing_marker_item_is_a_miss() {
        // `Item 116`(Properties)を欠く
        let ids: Vec<&str> = MSIME_IDS
            .iter()
            .copied()
            .filter(|id| *id != "Item 116")
            .collect();
        assert_eq!(
            classify_menu(MenuIme::MsIme, &menu(&ids, &["Item 101"])),
            Err(MissReason::MissingMarkerItems)
        );
    }

    /// `Item 536`(Troubleshoot problems)は新しい MS-IME だけの項目なので、無くても Hit する(要件に入れない)。
    #[test]
    fn msime_without_item_536_still_hits() {
        let ids: Vec<&str> = MSIME_IDS
            .iter()
            .copied()
            .filter(|id| *id != "Item 536")
            .collect();
        let r = classify_menu(MenuIme::MsIme, &menu(&ids, &["Item 101"])).unwrap();
        assert_eq!(r.mode, MenuMode::Hiragana);
    }

    // ---- 選択の数 ----

    #[test]
    fn no_selection_multiple_selection_and_non_mode_selection_are_misses() {
        assert_eq!(
            classify_menu(MenuIme::MsIme, &menu(&MSIME_IDS, &[])),
            Err(MissReason::NoSelection)
        );
        assert_eq!(
            classify_menu(MenuIme::MsIme, &menu(&MSIME_IDS, &["Item 101", "Item 102"])),
            Err(MissReason::MultipleSelection)
        );
        // 非モード項目(IME Pad)が選択されている
        assert_eq!(
            classify_menu(MenuIme::MsIme, &msime("Item 118")),
            Err(MissReason::SelectedNotMode)
        );
    }

    #[test]
    fn empty_menu_is_a_miss() {
        assert_eq!(
            classify_menu(MenuIme::MsIme, &[]),
            Err(MissReason::EmptyMenu)
        );
        assert_eq!(classify_menu(MenuIme::Gji, &[]), Err(MissReason::EmptyMenu));
    }

    // ---- 符号化 ----

    fn outcome(
        ime: MenuIme,
        result: Result<MenuReading, MissReason>,
        item_count: u16,
        read_ms: u16,
    ) -> MenuOutcome {
        MenuOutcome {
            ime,
            result,
            item_count,
            read_ms,
        }
    }

    #[test]
    fn pack_roundtrips_every_hit_and_miss() {
        for ime in [MenuIme::MsIme, MenuIme::Gji] {
            for mode in MODE_ALL {
                for open in [Some(false), Some(true), None] {
                    for evidence in [Evidence::Observed, Evidence::Inferred] {
                        let o = outcome(
                            ime,
                            Ok(MenuReading {
                                ime,
                                mode,
                                open,
                                evidence,
                            }),
                            16,
                            84,
                        );
                        let (w, l) = o.pack();
                        assert_eq!(MenuOutcome::unpack(w, l), Some(o), "{o:?}");
                    }
                }
            }
            for reason in MISS_ALL {
                let o = outcome(ime, Err(reason), 11, 109);
                let (w, l) = o.pack();
                assert_eq!(MenuOutcome::unpack(w, l), Some(o), "{o:?}");
            }
        }
    }

    #[test]
    fn pack_fits_in_32_bit_message_params_and_saturates() {
        let o = outcome(
            MenuIme::Gji,
            Err(MissReason::SelectedNotMode),
            u16::MAX,
            u16::MAX,
        );
        let (w, l) = o.pack();
        assert!(w < 1 << 8, "wParam は 8 bit に収まる");
        assert!(l >= 0, "lParam は負にならない(32 bit の isize でも)");
        assert!(l <= 0x7FFF_FFFF, "lParam は 31 bit に収まる");
        let back = MenuOutcome::unpack(w, l).unwrap();
        assert_eq!(back.item_count, u16::MAX);
        assert_eq!(back.read_ms, 0x7FFF, "read_ms は 15 bit に飽和");
    }

    #[test]
    fn unpack_rejects_corrupt_messages() {
        // 上位ビットが立っている
        assert_eq!(MenuOutcome::unpack(1 << 8, 0), None);
        // lParam が負
        assert_eq!(MenuOutcome::unpack(0, -1), None);
        // モードの番号が範囲外(6, 7)
        assert_eq!(MenuOutcome::unpack(6 << 2, 0), None);
        assert_eq!(MenuOutcome::unpack(7 << 2, 0), None);
        // 理由の番号が範囲外
        assert_eq!(MenuOutcome::unpack((1 << 1) | (6 << 2), 0), None);
        // Miss なのに開閉のビットが立っている
        assert_eq!(MenuOutcome::unpack((1 << 1) | (1 << 5), 0), None);
        // 開閉が 3(未定義)
        assert_eq!(MenuOutcome::unpack(3 << 5, 0), None);
    }

    #[test]
    fn labels_are_stable_journal_names() {
        assert_eq!(MenuIme::MsIme.label(), "ms-ime");
        assert_eq!(MenuIme::Gji.label(), "gji");
        assert_eq!(MenuMode::DirectInput.label(), "direct-input");
        assert_eq!(Evidence::Inferred.label(), "inferred");
        assert_eq!(
            MissReason::MissingMarkerItems.label(),
            "missing-marker-items"
        );
    }
}
