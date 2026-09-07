#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(msime_key_assignment.rsと同じ理由)
//! MS-IME「以前のバージョンのMicrosoft IMEを使う」互換モードの
//! 詳細キーカスタマイズ（`IMJPUEX.EXE`の「ユーザー定義」等）の起動時検出
//! （ADR-148 Phase 2）
//!
//! # 背景
//!
//! [`crate::msime_key_assignment`] が検出するのは「キーとタッチのカスタマイズ」
//! （新UI、`MSIME`直下のDWORD4値のみ）。互換モードでのみ到達できる旧UIの
//! 詳細キーカスタマイズは、無変換/変換キーを含む**任意のキー**に「IMEオン/オフ」
//! を割り当てられる別系統の設定で、新UI側の検出では見えない
//! （`docs/adr/148-bug-report-ime-keymap-attachment.md` Phase 2参照）。
//!
//! # レジストリ位置（2026-09-07 実機diffで確定）
//!
//! - `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME`の`keystyle`(REG_SZ) —
//!   現在有効な詳細キーカスタマイズのプリセット名。実測値の例:
//!   `"NATURAL"`（既定）、旧UIの「ユーザー定義」タブを開いて保存すると
//!   自動的に`"Custom"`に切り替わる。
//! - `HKCU\Software\Microsoft\IME\15.0\IMEJP\StyleList\<keystyle>\key`
//!   (REG_BINARY) — そのプリセットのキー割当てテーブル本体。
//!
//! # バイナリ形式（実機確認済み、Shift-JISテキスト）
//!
//! `hex:`（REG_BINARY）としてexportされるが、生バイト列は
//! `<キー名>=<コード1> <コード2> <コード3> <コード4> <コード5> <コード6>`
//! という記録がNUL区切りで連なり、リスト終端はNULがもう1つ多い、という
//! テキスト形式である。キー名はShift-JIS（「無変換」「変換」等の物理キー名、
//! `Ctrl+`/`Shift+`/`Alt+`修飾子付き表記もある）、コードは1バイトを
//! ASCII2文字の16進で表したもの。6個のコードは入力モード
//! （1列目=直接入力/IME OFF、2〜6列目=IME ON側の各モード、と推測）ごとに
//! 割り当てられた機能を表す列だと判明している。
//!
//! # 「IMEオン/オフ」トグルのコード（実機確認済み、限定的な範囲のみ）
//!
//! 旧UIの機能一覧には**単体のIME ON/単体のIME OFFという選択肢は存在せず**、
//! 「IMEオン/オフ」というトグル機能のみが選べる。これを無変換・変換キー
//! （修飾子なし）に割り当てたところ、2回とも同一パターンが再現した:
//! `無変換=CE CD CD CD CD CD` / `変換=CE CD CD CD CD CD`
//! （1列目=`CE`、2〜6列目=`CD`）。
//!
//! **実機での挙動確認（重要な非対称性）**: 直接入力中に変換キーを単独で
//! 押すとIME ONになった（1列目の`CE`は実際に効く）が、IME ON中に押しても
//! IME OFFにはならず、ネイティブの変換機能のままだった（2〜6列目の`CD`は
//! 実際には効かない——テーブル内に同名キーの重複行が生成され、後から
//! 追加された方〈2〜6列目が`00`＝「上書きなし」と見られる〉が優先される
//! ためと推測）。したがって本モジュールは**1列目（直接入力→ON方向）のみ**
//! を検出対象とする。2〜6列目（ON→OFF方向）は実効性が確認できていないため、
//! 検出・警告のいずれにも使わない（推測しない、実測した部分だけ使う）。
//!
//! # このモジュールが検出しないもの（既知の未解読範囲）
//!
//! - `Ctrl+`/`Shift+`/`Alt+`修飾子付きの無変換/変換（修飾子なしの単独タップ
//!   のみがawaseの親指シフト検出と衝突するため、範囲外とした）。
//! - `S1key`〜`SEkey`補助テーブル（本体`key`との重ね合わせ規則が未解読）。
//! - 「IMEオン/オフ」以外の機能（カタカナ/ひらがな切替等）のコード値。
//! - `key`テーブル内の重複行の一般的な優先順位規則（実機確認は変換キー
//!   1事例のみ）。
//!
//! レジストリは**読み取り専用**。

/// [`parse_legacy_key_table`]が返す1レコード。
///
/// `key_name_raw`はShift-JISの生バイト列のまま保持する（全キー名を汎用的に
/// デコードする実装を持たないため——[`is_muhenkan_label`]/[`is_henkan_label`]
/// による既知バイト列との一致判定にのみ使う）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyKeymapRecord {
    pub key_name_raw: Vec<u8>,
    /// 入力モード6列分の機能コード（1バイト）。
    pub codes: [u8; 6],
}

/// 無変換キーのShift-JISバイト列（修飾子なし）。
const MUHENKAN_LABEL: &[u8] = &[0x96, 0xb3, 0x95, 0xcf, 0x8a, 0xb7];
/// 変換キーのShift-JISバイト列（修飾子なし）。
const HENKAN_LABEL: &[u8] = &[0x95, 0xcf, 0x8a, 0xb7];
/// 「IMEオン/オフ」トグルを割り当てたとき、1列目（直接入力モード）に
/// 実測された機能コード。2026-09-07 dragonflyg4実機、無変換/変換の2キーで
/// 再現確認済み。
const IME_ON_TOGGLE_CODE_COL0: u8 = 0xCE;

/// `key`(REG_BINARY)の生バイト列を[`LegacyKeymapRecord`]のリストへ分解する。
///
/// 形式の詳細はモジュールdoc参照。壊れたレコード（`=`が無い、コードの
/// トークン数が6でない、16進として不正）はエラーにせず単に読み飛ばす
/// （1レコードの破損でテーブル全体を読めなくしないため）。
pub(crate) fn parse_legacy_key_table(bytes: &[u8]) -> Vec<LegacyKeymapRecord> {
    bytes
        .split(|&b| b == 0x00)
        .filter(|chunk| !chunk.is_empty())
        .filter_map(parse_one_record)
        .collect()
}

fn parse_one_record(chunk: &[u8]) -> Option<LegacyKeymapRecord> {
    let eq_pos = chunk.iter().position(|&b| b == b'=')?;
    let key_name_raw = chunk[..eq_pos].to_vec();
    let codes_str = &chunk[eq_pos + 1..];
    let tokens: Vec<&[u8]> = codes_str.split(|&b| b == b' ').collect();
    if tokens.len() != 6 {
        return None;
    }
    let mut codes = [0u8; 6];
    for (i, token) in tokens.iter().enumerate() {
        codes[i] = parse_hex_byte(token)?;
    }
    Some(LegacyKeymapRecord {
        key_name_raw,
        codes,
    })
}

fn parse_hex_byte(token: &[u8]) -> Option<u8> {
    if token.len() != 2 {
        return None;
    }
    let s = std::str::from_utf8(token).ok()?;
    u8::from_str_radix(s, 16).ok()
}

fn is_muhenkan_label(raw: &[u8]) -> bool {
    raw == MUHENKAN_LABEL
}

fn is_henkan_label(raw: &[u8]) -> bool {
    raw == HENKAN_LABEL
}

/// 指定ラベルの行（複数あれば重複行すべて）のうち、いずれか1つでも
/// 1列目が`IME_ON_TOGGLE_CODE_COL0`なら`true`。重複行は1列目の値が
/// 一致することを実機確認済み（変換キーの事例）なので、`any`で安全。
fn any_record_has_ime_on_toggle(
    records: &[LegacyKeymapRecord],
    is_target: impl Fn(&[u8]) -> bool,
) -> bool {
    records
        .iter()
        .any(|r| is_target(&r.key_name_raw) && r.codes[0] == IME_ON_TOGGLE_CODE_COL0)
}

/// 現在有効な詳細キーカスタマイズプリセット名（`keystyle`の実測値の既知集合）。
///
/// 未知の値は`Other`に潰す——`imjpuexc.exe SETKEYTEMPLATE`が受け付ける名前は
/// `Microsoft_IME`/`IME_Standard`/`ATOK`/`VJE`/`WX`のみだが、レジストリの
/// 実測値表記はこれと異なる（`NATURAL`/`MS-IME2000`等）ため、実測した文字列
/// のみを既知値として扱う（推測しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyKeyStyle {
    Atok,
    Custom,
    MsIme2000,
    Natural,
    Vje,
    Wx,
    /// 実測していない値。生の文字列は保持しない
    /// （ADR-148 F7と同じ理由: 自由文字列を安全弁の外に漏らさない）。
    Other,
}

/// [`LegacyKeyStyle::from_registry_value`]/[`LegacyKeyStyle::as_str`]共通の
/// 唯一の対応表（コードレビュー指摘: 2つのmatch文に同じ6値を重複させると
/// 将来値追加時に片方だけ更新して静かにズレる）。
const KNOWN_STYLES: &[(LegacyKeyStyle, &str)] = &[
    (LegacyKeyStyle::Atok, "ATOK"),
    (LegacyKeyStyle::Custom, "Custom"),
    (LegacyKeyStyle::MsIme2000, "MS-IME2000"),
    (LegacyKeyStyle::Natural, "NATURAL"),
    (LegacyKeyStyle::Vje, "VJE"),
    (LegacyKeyStyle::Wx, "WX"),
];

impl LegacyKeyStyle {
    fn from_registry_value(s: &str) -> Self {
        KNOWN_STYLES
            .iter()
            .find(|(_, name)| *name == s)
            .map_or(Self::Other, |(style, _)| *style)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        KNOWN_STYLES
            .iter()
            .find(|(style, _)| *style == self)
            .map_or("Other", |(_, name)| name)
    }
}

/// 旧UI（詳細キーカスタマイズ）で無変換/変換キーに「IMEオン/オフ」
/// トグルが割り当てられているかの検出結果。
///
/// `muhenkan_ime_on_toggle`/`henkan_ime_on_toggle`は`Option<bool>`——
/// `None`は「判定できなかった」（未知プリセット・レジストリエラー等）を
/// 意味し、「割当てなしと確認できた」（`Some(false)`）とは区別する
/// （コードレビュー指摘: この区別が無いと、判定不能を「安全」と誤読する
/// 静かな偽陰性になる）。`Some(true)`でも実機確認済みなのは「直接入力中に
/// 押すと予期せずIME ONになる」方向のみで、「IME ON中に押すとIME OFFに
/// なる」方向は実効性未確認（モジュールdoc参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LegacyMsImeToggleAssignment {
    pub active_style: Option<LegacyKeyStyle>,
    pub muhenkan_ime_on_toggle: Option<bool>,
    pub henkan_ime_on_toggle: Option<bool>,
}

impl LegacyMsImeToggleAssignment {
    /// 判定不能（未知プリセット・レジストリエラー等）。
    fn unknown(active_style: Option<LegacyKeyStyle>) -> Self {
        Self {
            active_style,
            muhenkan_ime_on_toggle: None,
            henkan_ime_on_toggle: None,
        }
    }

    /// 既知プリセットだが`key`値自体が存在しない（＝一度も詳細カスタマイズ
    /// されていない）ことをレジストリの「値なし」応答から確認できた場合。
    /// 「判定不能」ではなく確定した`Some(false)`として扱ってよい。
    fn confirmed_absent(active_style: LegacyKeyStyle) -> Self {
        Self {
            active_style: Some(active_style),
            muhenkan_ime_on_toggle: Some(false),
            henkan_ime_on_toggle: Some(false),
        }
    }

    fn from_table(active_style: LegacyKeyStyle, records: &[LegacyKeymapRecord]) -> Self {
        Self {
            active_style: Some(active_style),
            muhenkan_ime_on_toggle: Some(any_record_has_ime_on_toggle(records, is_muhenkan_label)),
            henkan_ime_on_toggle: Some(any_record_has_ime_on_toggle(records, is_henkan_label)),
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;

    use super::{parse_legacy_key_table, LegacyKeyStyle, LegacyMsImeToggleAssignment};

    const IMEJP_BASE: &str = "Software\\Microsoft\\IME\\15.0\\IMEJP";

    /// `RegGetValueW`の2回呼び出し（サイズ取得→本読み）で可変長の値を読む。
    /// `flags`は`RRF_RT_REG_SZ`/`RRF_RT_REG_BINARY`いずれかを渡す。
    ///
    /// `scancode_map.rs::registry::read()`と同じ形（コードレビュー指摘）:
    /// 値が存在しない（`ERROR_FILE_NOT_FOUND`）ことと、それ以外の失敗
    /// （TOCTOU競合等の一時的なエラーを含む）を区別して返す——前者は
    /// 「確認できた事実」、後者は「判定できなかった」であり、呼び出し元は
    /// この2つを別の意味として扱う必要がある。
    fn read_raw_value(
        subkey: &str,
        value_name: &str,
        flags: u32,
    ) -> Result<Option<Vec<u8>>, String> {
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER};
        let subkey_wide = crate::win32::to_wide(subkey);
        let value_wide = crate::win32::to_wide(value_name);
        let subkey_pcwstr = PCWSTR(subkey_wide.as_ptr());
        let value_pcwstr = PCWSTR(value_wide.as_ptr());
        let mut size: u32 = 0;
        // SAFETY: subkey_wide/value_wide はNUL終端済みUTF-16で呼び出し中有効。
        //         最初の呼び出しはサイズ取得のみでバッファを書き込まない。
        let probe = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey_pcwstr,
                value_pcwstr,
                windows::Win32::System::Registry::REG_ROUTINE_FLAGS(flags),
                None,
                None,
                Some(&raw mut size),
            )
        };
        if probe.is_err() {
            if probe == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            return Err(format!(
                "registry read failed (size probe) for {subkey}\\{value_name}: {probe:?}"
            ));
        }
        if size == 0 {
            return Ok(Some(Vec::new()));
        }
        let mut buf = vec![0u8; size as usize];
        let mut size2 = size;
        // SAFETY: buf は size バイト確保済みで呼び出し中有効。size2 は
        //         書き込まれた実バイト数を受け取る。
        let fill = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey_pcwstr,
                value_pcwstr,
                windows::Win32::System::Registry::REG_ROUTINE_FLAGS(flags),
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&raw mut size2),
            )
        };
        if fill.is_err() {
            if fill == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            return Err(format!(
                "registry read failed (fill) for {subkey}\\{value_name}: {fill:?}"
            ));
        }
        buf.truncate(size2 as usize);
        Ok(Some(buf))
    }

    /// `keystyle`(REG_SZ)を読む。`Ok(None)`=値が存在しない、`Err`=読み取り
    /// 自体に失敗（TOCTOU競合・不正なUTF-16長等、コードレビュー指摘で
    /// 追加——以前は末尾半端バイトを`chunks_exact`で無言破棄していた）。
    fn read_active_style() -> Result<Option<LegacyKeyStyle>, String> {
        use windows::Win32::System::Registry::RRF_RT_REG_SZ;
        let Some(bytes) =
            read_raw_value(&format!("{IMEJP_BASE}\\MSIME"), "keystyle", RRF_RT_REG_SZ.0)?
        else {
            return Ok(None);
        };
        if bytes.len() % 2 != 0 {
            return Err(format!(
                "keystyle registry value has odd byte length: {}",
                bytes.len()
            ));
        }
        // REG_SZ はUTF-16LE、末尾NULを含む。
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let s = String::from_utf16(&units)
            .map_err(|e| format!("keystyle registry value is not valid UTF-16: {e}"))?;
        Ok(Some(LegacyKeyStyle::from_registry_value(&s)))
    }

    fn read_key_table_bytes(style: LegacyKeyStyle) -> Result<Option<Vec<u8>>, String> {
        use windows::Win32::System::Registry::RRF_RT_REG_BINARY;
        let subkey = format!("{IMEJP_BASE}\\StyleList\\{}", style.as_str());
        read_raw_value(&subkey, "key", RRF_RT_REG_BINARY.0)
    }

    /// 現在有効な詳細キーカスタマイズプリセットを読み、無変換/変換キーへの
    /// 「IMEオン/オフ」割当てを検出する（ADR-148 Phase 2）。
    ///
    /// `LegacyKeyStyle::Other`（未知プリセット）は「判定不能」を返す
    /// （コードレビュー指摘: `StyleList\Other\key`という実在しないパスを
    /// 読みに行って「値なし」＝「割当てなし」と誤判定していた）。
    #[must_use]
    pub(crate) fn read_legacy_toggle_assignment() -> LegacyMsImeToggleAssignment {
        let Ok(Some(style)) = read_active_style() else {
            return LegacyMsImeToggleAssignment::unknown(None);
        };
        if style == LegacyKeyStyle::Other {
            return LegacyMsImeToggleAssignment::unknown(Some(style));
        }
        match read_key_table_bytes(style) {
            Ok(Some(table_bytes)) => {
                let records = parse_legacy_key_table(&table_bytes);
                LegacyMsImeToggleAssignment::from_table(style, &records)
            }
            Ok(None) => LegacyMsImeToggleAssignment::confirmed_absent(style),
            Err(_) => LegacyMsImeToggleAssignment::unknown(Some(style)),
        }
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::read_legacy_toggle_assignment;

#[cfg(test)]
mod tests {
    use super::*;

    /// 「無変換=CE CD CD CD CD CD」（2026-09-07 dragonflyg4実機、
    /// `StyleList\Custom\key`に実際に書き込まれたバイト列そのもの）。
    fn muhenkan_ime_on_toggle_record() -> Vec<u8> {
        let mut v = MUHENKAN_LABEL.to_vec();
        v.extend_from_slice(b"=CE CD CD CD CD CD\0");
        v
    }

    /// ATOKプリセットの既定値「無変換=A2 A2 A2 A2 A2 A2」（割当てなし相当）。
    fn muhenkan_default_record() -> Vec<u8> {
        let mut v = MUHENKAN_LABEL.to_vec();
        v.extend_from_slice(b"=A2 A2 A2 A2 A2 A2\0");
        v
    }

    #[test]
    fn parses_single_record() {
        let mut bytes = muhenkan_ime_on_toggle_record();
        bytes.push(0x00); // リスト終端の追加NUL
        let records = parse_legacy_key_table(&bytes);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key_name_raw, MUHENKAN_LABEL);
        assert_eq!(records[0].codes, [0xCE, 0xCD, 0xCD, 0xCD, 0xCD, 0xCD]);
    }

    #[test]
    fn detects_ime_on_toggle_on_muhenkan() {
        let mut bytes = muhenkan_ime_on_toggle_record();
        bytes.push(0x00);
        let records = parse_legacy_key_table(&bytes);
        let result = LegacyMsImeToggleAssignment::from_table(LegacyKeyStyle::Custom, &records);
        assert_eq!(result.muhenkan_ime_on_toggle, Some(true));
        assert_eq!(result.henkan_ime_on_toggle, Some(false));
    }

    #[test]
    fn default_assignment_is_not_detected_as_toggle() {
        let mut bytes = muhenkan_default_record();
        bytes.push(0x00);
        let records = parse_legacy_key_table(&bytes);
        let result = LegacyMsImeToggleAssignment::from_table(LegacyKeyStyle::Atok, &records);
        assert_eq!(result.muhenkan_ime_on_toggle, Some(false));
    }

    /// 変換キーで実機確認した「重複行」ケース: 同名の行が2つあり、後の方は
    /// 2〜6列目が`00`(上書きなしと見られる値)。1列目はどちらも`CE`で一致
    /// するため、`any`判定なら重複の有無に関わらず正しく検出できる。
    #[test]
    fn detects_ime_on_toggle_even_with_conflicting_duplicate_row() {
        let mut bytes = HENKAN_LABEL.to_vec();
        bytes.extend_from_slice(b"=CE CD CD CD CD CD\0");
        bytes.extend_from_slice(HENKAN_LABEL);
        bytes.extend_from_slice(b"=CE 00 00 00 00 00\0");
        bytes.push(0x00);
        let records = parse_legacy_key_table(&bytes);
        assert_eq!(records.len(), 2);
        let result = LegacyMsImeToggleAssignment::from_table(LegacyKeyStyle::Custom, &records);
        assert_eq!(result.henkan_ime_on_toggle, Some(true));
    }

    #[test]
    fn modifier_prefixed_key_is_not_confused_with_bare_key() {
        // "Ctrl+変換=CE 00 00 00 00 00" は範囲外(修飾子付き)。
        let mut bytes = b"Ctrl+".to_vec();
        bytes.extend_from_slice(HENKAN_LABEL);
        bytes.extend_from_slice(b"=CE 00 00 00 00 00\0");
        bytes.push(0x00);
        let records = parse_legacy_key_table(&bytes);
        assert_eq!(records.len(), 1);
        let result = LegacyMsImeToggleAssignment::from_table(LegacyKeyStyle::Custom, &records);
        assert_eq!(result.henkan_ime_on_toggle, Some(false));
    }

    #[test]
    fn unknown_style_is_reported_as_undetermined_not_absent() {
        // コードレビュー指摘の回帰: 未知プリセットは「割当てなし」
        // (Some(false))ではなく「判定不能」(None)であるべき。
        let result = LegacyMsImeToggleAssignment::unknown(Some(LegacyKeyStyle::Other));
        assert_eq!(result.muhenkan_ime_on_toggle, None);
        assert_eq!(result.henkan_ime_on_toggle, None);
        assert_eq!(result.active_style, Some(LegacyKeyStyle::Other));
    }

    #[test]
    fn confirmed_absent_reports_false_not_unknown() {
        // 既知プリセットで`key`値自体が存在しない場合は確定した
        // Some(false)（「判定不能」ではない）。
        let result = LegacyMsImeToggleAssignment::confirmed_absent(LegacyKeyStyle::Natural);
        assert_eq!(result.muhenkan_ime_on_toggle, Some(false));
        assert_eq!(result.henkan_ime_on_toggle, Some(false));
    }

    #[test]
    fn malformed_record_is_skipped_not_panicking() {
        let mut bytes = b"BrokenNoEquals".to_vec();
        bytes.push(0x00);
        bytes.extend_from_slice(&muhenkan_ime_on_toggle_record());
        bytes.push(0x00);
        let records = parse_legacy_key_table(&bytes);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key_name_raw, MUHENKAN_LABEL);
    }

    #[test]
    fn wrong_token_count_is_skipped() {
        let mut bytes = MUHENKAN_LABEL.to_vec();
        bytes.extend_from_slice(b"=CE CD CD\0"); // 6個ではなく3個
        bytes.push(0x00);
        let records = parse_legacy_key_table(&bytes);
        assert!(records.is_empty());
    }

    #[test]
    fn legacy_key_style_round_trips_known_values() {
        for style in [
            LegacyKeyStyle::Atok,
            LegacyKeyStyle::Custom,
            LegacyKeyStyle::MsIme2000,
            LegacyKeyStyle::Natural,
            LegacyKeyStyle::Vje,
            LegacyKeyStyle::Wx,
        ] {
            assert_eq!(LegacyKeyStyle::from_registry_value(style.as_str()), style);
        }
    }

    #[test]
    fn unknown_style_value_becomes_other() {
        assert_eq!(
            LegacyKeyStyle::from_registry_value("SomeFutureStyle"),
            LegacyKeyStyle::Other
        );
    }
}
