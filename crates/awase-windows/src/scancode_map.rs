#![cfg_attr(windows, allow(unsafe_code))]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Windows Scancode Map（`HKLM\SYSTEM\CurrentControlSet\Control\Keyboard
//! Layout\Scancode Map`）の読み書きと、Caps(英数)⇔Left Ctrl 入れ替え /
//! Caps(英数)→Left Ctrl 片方向複製プリセット用のバイト列生成・
//! マージロジック（ADR-111 / ADR-126）。
//!
//! バイナリ形式は Windows のドキュメント化された固定フォーマット
//! （4バイトヘッダ×2 + エントリ数(null終端込み) + エントリ配列 + null終端）
//! に従う。エントリはリトルエンディアン `u16` ペア `[to_scancode,
//! from_scancode]` の順。パース・生成・マージのロジックは純粋関数として
//! 全プラットフォームでコンパイル・テストできる（`awase-settings` から
//! Linux上でも単体テストするため）。レジストリ I/O のみ `#[cfg(windows)]`。

/// JIS「英数」キー / US「CapsLock」キーの物理スキャンコード（Set 1）。
/// 両者は物理的に同一の位置・同一のスキャンコードを共有し、レイアウト
/// ドライバが Shift 状態で異なる VK に翻訳する（ADR-111 決定1）。
pub const SCANCODE_CAPS_EISU: u16 = 0x003A;
/// Left Ctrl のスキャンコード（Set 1、非拡張）。
pub const SCANCODE_LEFT_CTRL: u16 = 0x001D;

/// Scancode Map に書き込む具体的な内容を持つプリセット。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScancodeMapPreset {
    /// ADR-111: Caps(英数)⇔Left Ctrl 双方向入れ替え。
    Swap,
    /// ADR-126: Caps(英数)→Left Ctrl 片方向のみ。
    CapsAsExtraCtrl,
}

impl ScancodeMapPreset {
    /// このプリセットが書き込むエントリ（`(from, to)` の順）。
    #[must_use]
    pub const fn entries(self) -> &'static [(u16, u16)] {
        match self {
            Self::Swap => &[
                (SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL),
                (SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU),
            ],
            Self::CapsAsExtraCtrl => &[(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL)],
        }
    }

    /// このプリセットが有効なとき「自分が書いた」と主張できる `from` 側
    /// scancode の集合。
    const fn owned_from_codes(self) -> &'static [u16] {
        match self {
            Self::Swap => &[SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL],
            Self::CapsAsExtraCtrl => &[SCANCODE_CAPS_EISU],
        }
    }
}

/// GUI のラジオボタン・CLI 引数が表すユーザー選択。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScancodeMapSelection {
    Off,
    Swap,
    CapsAsExtraCtrl,
}

impl ScancodeMapSelection {
    /// 選択に対応するプリセット。`Off` はレジストリ値の無効化を表す。
    #[must_use]
    pub const fn preset(self) -> Option<ScancodeMapPreset> {
        match self {
            Self::Off => None,
            Self::Swap => Some(ScancodeMapPreset::Swap),
            Self::CapsAsExtraCtrl => Some(ScancodeMapPreset::CapsAsExtraCtrl),
        }
    }

    /// 昇格ワーカーへ渡す CLI 引数値。
    #[must_use]
    pub const fn as_cli_arg(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Swap => "swap",
            Self::CapsAsExtraCtrl => "caps-extra-ctrl",
        }
    }

    /// 昇格ワーカー側で CLI 引数値を解釈する。
    #[must_use]
    pub fn from_cli_arg(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "swap" => Some(Self::Swap),
            "caps-extra-ctrl" => Some(Self::CapsAsExtraCtrl),
            _ => None,
        }
    }
}

/// Scancode Map の `REG_BINARY` 値をパースし `(from, to)` のリストを返す。
/// 不正な形式（短すぎる等）は空リストを返す（既存値が壊れていた場合に
/// 安全側で「未設定」扱いにするため、エラーにはしない）。
#[must_use]
pub fn parse_entries(bytes: &[u8]) -> Vec<(u16, u16)> {
    if bytes.len() < 12 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let mut out = Vec::new();
    let mut offset = 12;
    // count にはnull終端エントリ自身も含まれるので count-1 件読む。
    for _ in 0..count.saturating_sub(1) {
        let Some(chunk) = bytes.get(offset..offset + 4) else {
            break;
        };
        let to = u16::from_le_bytes([chunk[0], chunk[1]]);
        let from = u16::from_le_bytes([chunk[2], chunk[3]]);
        if from == 0 && to == 0 {
            break;
        }
        out.push((from, to));
        offset += 4;
    }
    out
}

/// `(from, to)` のリストから Scancode Map の `REG_BINARY` 値を構築する。
/// `entries` が空なら `None`（値自体を削除すべきことを示す）。
#[must_use]
pub fn build_bytes(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    if entries.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(12 + entries.len() * 4 + 4);
    out.extend_from_slice(&0u32.to_le_bytes()); // Header: Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Header: Flags
    let count = u32::try_from(entries.len() + 1).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for &(from, to) in entries {
        out.extend_from_slice(&to.to_le_bytes());
        out.extend_from_slice(&from.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // null 終端エントリ
    Some(out)
}

/// 現在のエントリ列から、有効なプリセットを検出する。
/// Swap は CapsAsExtraCtrl の1エントリを包含するため先に判定する。
#[must_use]
pub fn current_preset(entries: &[(u16, u16)]) -> Option<ScancodeMapPreset> {
    if ScancodeMapPreset::Swap
        .entries()
        .iter()
        .all(|e| entries.contains(e))
    {
        Some(ScancodeMapPreset::Swap)
    } else if ScancodeMapPreset::CapsAsExtraCtrl
        .entries()
        .iter()
        .all(|e| entries.contains(e))
    {
        Some(ScancodeMapPreset::CapsAsExtraCtrl)
    } else {
        None
    }
}

/// 有効化・無効化・切替後に書き込むエントリ列を計算する。
///
/// 掃除対象は「検出された現行プリセットの所有 `from` scancode」と
/// 「目標プリセットの所有 `from` scancode」の和集合だけに限定する。
///
/// 既知の限界（決定3「原理的限界」参照）: Swap の2エントリと「CapsAsExtraCtrl
/// と、値が偶然 Swap の逆方向エントリと一致する第三者エントリ」の組は、
/// レジストリのバイト列だけでは区別できない。この曖昧な状態は
/// `current_preset` の Swap優先判定により Swap として扱われるため、
/// 無効化するとその第三者エントリも一緒に消える。
///
/// 実装時に一度、書き込み順序を手がかりにこの2状態を判別しようとする
/// ヒューリスティックが追加されたが、Opus敵対的コードレビューで撤回した。
/// 撤回理由は次の2点: 対象を無効化時の遷移のみに限定していたため
/// `target=CapsAsExtraCtrl` への切替ではB1が別経路で再発する点、および
/// 順序ヒューリスティックの前提（第三者エントリの間に別エントリが
/// 挟まる・レジストリ全体が書き直される等）は容易に崩れ、崩れると
/// 真の Swap を無効化しても消し残りが発生し再試行しても直らない点。
///
/// この曖昧性はレジストリ内容のみを真実の情報源にしている以上、原理的に
/// 解消できない（真に解決するには awase が最後に書いたプリセットを
/// 別の場所に記録するマーカーが必要で、別ADR相当のスコープ）。
#[must_use]
pub fn compute_new_entries(
    existing: &[(u16, u16)],
    target: ScancodeMapSelection,
) -> Vec<(u16, u16)> {
    let current = current_preset(existing);
    let target_preset = target.preset();
    let mut clear: Vec<u16> = Vec::new();
    if let Some(preset) = current {
        clear.extend_from_slice(preset.owned_from_codes());
    }
    if let Some(preset) = target_preset {
        clear.extend_from_slice(preset.owned_from_codes());
    }

    let mut out: Vec<_> = existing
        .iter()
        .copied()
        .filter(|&(from, _)| !clear.contains(&from))
        .collect();
    if let Some(preset) = target_preset {
        out.extend_from_slice(preset.entries());
    }
    out
}

/// エントリ列から検出したプリセットと、無関係な余剰エントリ数を返す。
#[must_use]
pub fn detect_status(entries: &[(u16, u16)]) -> (Option<ScancodeMapPreset>, usize) {
    let preset = current_preset(entries);
    let owned_len = preset.map_or(0, |p| p.entries().len());
    (preset, entries.len() - owned_len)
}

#[cfg(windows)]
mod registry {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_LOCAL_MACHINE, REG_BINARY,
        RRF_RT_REG_BINARY,
    };

    const SUBKEY: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Control\\Keyboard Layout");
    const VALUE_NAME: PCWSTR = w!("Scancode Map");

    /// 現在の Scancode Map の値を読み取る。値が存在しなければ `Ok(None)`。
    /// 昇格不要（`HKEY_LOCAL_MACHINE` は既定で誰でも読める）。
    pub fn read() -> Result<Option<Vec<u8>>, String> {
        let mut size: u32 = 0;
        // SAFETY: 出力バッファ引数は None（サイズ取得のみ）。他の引数は
        // 静的な NUL 終端済み UTF-16 文字列。
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                SUBKEY,
                VALUE_NAME,
                RRF_RT_REG_BINARY,
                None,
                None,
                Some(&raw mut size),
            )
        };
        if result != windows::Win32::Foundation::ERROR_SUCCESS {
            if result == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            return Err(format!("Scancode Map 読み取り失敗(サイズ取得): {result:?}"));
        }
        if size == 0 {
            return Ok(Some(Vec::new()));
        }
        let mut buf = vec![0u8; size as usize];
        let mut actual_size = size;
        // SAFETY: buf は size バイト確保済みで、呼び出し中有効。
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                SUBKEY,
                VALUE_NAME,
                RRF_RT_REG_BINARY,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&raw mut actual_size),
            )
        };
        if result == windows::Win32::Foundation::ERROR_SUCCESS {
            buf.truncate(actual_size as usize);
            Ok(Some(buf))
        } else {
            Err(format!("Scancode Map 読み取り失敗: {result:?}"))
        }
    }

    /// Scancode Map に値を書き込む。管理者権限が必要（呼び出し元は
    /// 昇格済みであること、`awase-settings` の自己昇格フロー参照）。
    pub fn write(bytes: &[u8]) -> Result<(), String> {
        // SAFETY: bytes は呼び出し中有効なスライス。
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_LOCAL_MACHINE,
                SUBKEY,
                VALUE_NAME,
                REG_BINARY.0,
                Some(bytes.as_ptr().cast()),
                u32::try_from(bytes.len()).unwrap_or(u32::MAX),
            )
        };
        if result == windows::Win32::Foundation::ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Scancode Map 書き込み失敗: {result:?}"))
        }
    }

    /// Scancode Map の値自体を削除する。管理者権限が必要。
    pub fn delete() -> Result<(), String> {
        // SAFETY: `HKEY_LOCAL_MACHINE` は擬似ハンドルで CloseHandle 不要。
        //         SUBKEY/VALUE_NAME は静的な NUL 終端済み UTF-16 文字列。
        let result = unsafe { RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, SUBKEY, VALUE_NAME) };
        if result == windows::Win32::Foundation::ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("Scancode Map 削除失敗: {result:?}"))
        }
    }
}

#[cfg(windows)]
pub use registry::{delete, read, write};

#[cfg(test)]
mod tests {
    use super::{
        build_bytes, compute_new_entries, current_preset, detect_status, parse_entries,
        ScancodeMapPreset, ScancodeMapSelection, SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL,
    };

    #[test]
    fn build_then_parse_roundtrips_swap_preset() {
        let entries = ScancodeMapPreset::Swap.entries();
        let bytes = build_bytes(entries).expect("non-empty entries");
        assert_eq!(parse_entries(&bytes), entries.to_vec());
    }

    #[test]
    fn build_then_parse_roundtrips_caps_as_extra_ctrl_preset() {
        let entries = ScancodeMapPreset::CapsAsExtraCtrl.entries();
        let bytes = build_bytes(entries).expect("non-empty entries");
        assert_eq!(parse_entries(&bytes), entries.to_vec());
    }

    #[test]
    fn build_bytes_matches_documented_layout_for_swap_preset() {
        let bytes = build_bytes(ScancodeMapPreset::Swap.entries()).unwrap();
        // ヘッダ(Version=0, Flags=0) + count=3(2エントリ+null終端)
        assert_eq!(&bytes[0..4], &0u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &3u32.to_le_bytes());
        // エントリ1: to=0x001D, from=0x003A
        assert_eq!(&bytes[12..14], &0x001Du16.to_le_bytes());
        assert_eq!(&bytes[14..16], &0x003Au16.to_le_bytes());
        // エントリ2: to=0x003A, from=0x001D
        assert_eq!(&bytes[16..18], &0x003Au16.to_le_bytes());
        assert_eq!(&bytes[18..20], &0x001Du16.to_le_bytes());
        // null終端
        assert_eq!(&bytes[20..24], &0u32.to_le_bytes());
        assert_eq!(bytes.len(), 24);
    }

    #[test]
    fn build_bytes_returns_none_for_empty() {
        assert!(build_bytes(&[]).is_none());
    }

    #[test]
    fn parse_entries_handles_short_or_garbage_input_safely() {
        assert_eq!(parse_entries(&[]), Vec::new());
        assert_eq!(parse_entries(&[0u8; 11]), Vec::new());
        assert_eq!(parse_entries(&[0u8; 12]), Vec::new());
    }

    #[test]
    fn build_bytes_matches_documented_layout_for_caps_as_extra_ctrl_preset() {
        let bytes = build_bytes(ScancodeMapPreset::CapsAsExtraCtrl.entries()).unwrap();
        // ヘッダ(Version=0, Flags=0) + count=2(1エントリ+null終端)
        assert_eq!(&bytes[0..4], &0u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &2u32.to_le_bytes());
        // エントリ1: to=0x001D, from=0x003A
        assert_eq!(&bytes[12..14], &0x001Du16.to_le_bytes());
        assert_eq!(&bytes[14..16], &0x003Au16.to_le_bytes());
        // null終端
        assert_eq!(&bytes[16..20], &0u32.to_le_bytes());
        assert_eq!(bytes.len(), 20);
    }

    #[test]
    fn current_preset_detects_swap_when_both_entries_present_among_others() {
        let mut entries = vec![(0x0010, 0x0011)];
        entries.extend_from_slice(ScancodeMapPreset::Swap.entries());
        assert_eq!(current_preset(&entries), Some(ScancodeMapPreset::Swap));
    }

    #[test]
    fn current_preset_detects_caps_as_extra_ctrl_when_only_one_direction_present() {
        let entries = vec![(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL)];
        assert_eq!(
            current_preset(&entries),
            Some(ScancodeMapPreset::CapsAsExtraCtrl)
        );
    }

    #[test]
    fn current_preset_prefers_swap_over_caps_as_extra_ctrl() {
        let entries = ScancodeMapPreset::Swap.entries().to_vec();
        assert_eq!(current_preset(&entries), Some(ScancodeMapPreset::Swap));
    }

    #[test]
    fn current_preset_returns_none_for_empty_or_unrelated_entries() {
        assert_eq!(current_preset(&[]), None);
        assert_eq!(current_preset(&[(0x0010, 0x0011)]), None);
    }

    #[test]
    fn compute_new_entries_off_to_swap() {
        let existing = vec![
            (0x0010, 0x0011),             // 無関係、保持されるべき
            (SCANCODE_CAPS_EISU, 0x0099), // 衝突、置き換えられるべき
            (SCANCODE_LEFT_CTRL, 0x0088), // 衝突、置き換えられるべき
        ];
        let mut expected = vec![(0x0010, 0x0011)];
        expected.extend_from_slice(ScancodeMapPreset::Swap.entries());
        assert_eq!(
            compute_new_entries(&existing, ScancodeMapSelection::Swap),
            expected
        );
    }

    #[test]
    fn compute_new_entries_off_to_caps_as_extra_ctrl() {
        let existing = vec![
            (0x0010, 0x0011),
            (SCANCODE_CAPS_EISU, 0x0099),
            (SCANCODE_LEFT_CTRL, 0x0088),
        ];
        let mut expected = vec![(0x0010, 0x0011), (SCANCODE_LEFT_CTRL, 0x0088)];
        expected.extend_from_slice(ScancodeMapPreset::CapsAsExtraCtrl.entries());
        assert_eq!(
            compute_new_entries(&existing, ScancodeMapSelection::CapsAsExtraCtrl),
            expected
        );
    }

    #[test]
    fn compute_new_entries_swap_to_caps_as_extra_ctrl() {
        let mut existing = vec![(0x0010, 0x0011)];
        existing.extend_from_slice(ScancodeMapPreset::Swap.entries());
        let mut expected = vec![(0x0010, 0x0011)];
        expected.extend_from_slice(ScancodeMapPreset::CapsAsExtraCtrl.entries());
        assert_eq!(
            compute_new_entries(&existing, ScancodeMapSelection::CapsAsExtraCtrl),
            expected
        );
    }

    #[test]
    fn compute_new_entries_caps_as_extra_ctrl_to_swap() {
        let mut existing = vec![(0x0010, 0x0011), (SCANCODE_LEFT_CTRL, 0x0088)];
        existing.extend_from_slice(ScancodeMapPreset::CapsAsExtraCtrl.entries());
        let mut expected = vec![(0x0010, 0x0011)];
        expected.extend_from_slice(ScancodeMapPreset::Swap.entries());
        assert_eq!(
            compute_new_entries(&existing, ScancodeMapSelection::Swap),
            expected
        );
    }

    #[test]
    fn compute_new_entries_caps_as_extra_ctrl_to_off() {
        let mut existing = vec![(0x0010, 0x0011)];
        existing.extend_from_slice(ScancodeMapPreset::CapsAsExtraCtrl.entries());
        assert_eq!(
            compute_new_entries(&existing, ScancodeMapSelection::Off),
            vec![(0x0010, 0x0011)]
        );
    }

    #[test]
    fn compute_new_entries_swap_to_off() {
        // ADR-111由来のSwapプリセットを無効化する、最も基本的な遷移
        // （/code-review指摘: リファクタ前はremove_preset_*テストで
        // カバーされていたが、compute_new_entriesへの統合後どのテストにも
        // 現れていなかった）。
        let mut existing = vec![(0x0010, 0x0011)];
        existing.extend_from_slice(ScancodeMapPreset::Swap.entries());
        assert_eq!(
            compute_new_entries(&existing, ScancodeMapSelection::Off),
            vec![(0x0010, 0x0011)]
        );
    }

    #[test]
    fn third_party_left_ctrl_remap_to_unrelated_key_survives_caps_extra_ctrl_enable_and_disable() {
        // 決定3が実際に保護する、現実的な多数派のケース: 第三者ツールが
        // Left Ctrl を awase のプリセットと無関係な値にリマップしている
        // （Swap の逆方向エントリの値 SCANCODE_CAPS_EISU とは異なる）。
        // CapsAsExtraCtrl は 0x1D を所有しないため、有効化・無効化の
        // どちらを通しても触れられない。
        let existing = vec![(SCANCODE_LEFT_CTRL, 0x0099)];
        let enabled = compute_new_entries(&existing, ScancodeMapSelection::CapsAsExtraCtrl);
        assert!(enabled.contains(&(SCANCODE_LEFT_CTRL, 0x0099)));
        assert!(enabled.contains(&(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL)));
        let disabled = compute_new_entries(&enabled, ScancodeMapSelection::Off);
        assert_eq!(disabled, existing);
    }

    #[test]
    fn ambiguous_third_party_entry_colliding_with_swap_reverse_value_is_not_preserved() {
        // 既知の限界（決定3「原理的限界」参照）: 第三者エントリの値が
        // たまたま Swap の逆方向エントリ (0x1D→0x3A) と完全に一致する場合、
        // CapsAsExtraCtrl 有効化後の状態はレジストリのバイト列だけからは
        // 真の Swap と区別できない。current_preset は Swap を優先判定する
        // ため、このケースを無効化すると第三者エントリごと消える
        // ——バグではなく、レジストリ内容のみを情報源にすることの
        // 原理的な限界として受容し、ここでその挙動を固定する。
        let existing = vec![(SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU)];
        let enabled = compute_new_entries(&existing, ScancodeMapSelection::CapsAsExtraCtrl);
        assert_eq!(
            current_preset(&enabled),
            Some(ScancodeMapPreset::Swap),
            "曖昧な状態はSwapとして検出される(既知の限界)"
        );
        let disabled = compute_new_entries(&enabled, ScancodeMapSelection::Off);
        assert!(
            disabled.is_empty(),
            "第三者エントリもろとも消える(既知の限界、B1の対象外)"
        );
    }

    #[test]
    fn detect_status_counts_extra_entries_for_caps_as_extra_ctrl() {
        let mut entries = vec![(0x0010, 0x0011), (0x0012, 0x0013)];
        entries.extend_from_slice(ScancodeMapPreset::CapsAsExtraCtrl.entries());
        assert_eq!(
            detect_status(&entries),
            (Some(ScancodeMapPreset::CapsAsExtraCtrl), 2)
        );
    }

    // `registry::delete()` の `ERROR_FILE_NOT_FOUND → Ok(())` 分岐は、
    // 実際の `HKLM\...\Scancode Map` に対して2回連続で削除を試みないと
    // 検証できない。Opus敵対的コードレビューで「これを単体テストとして
    // 実行すると、開発者/CIランナーの実際の Scancode Map 設定を書き換え
    // うる（非昇格ならACCESS_DENIEDで失敗、昇格なら実害）」と指摘され
    // 削除した。ロジック自体は `result == ERROR_SUCCESS ||
    // result == ERROR_FILE_NOT_FOUND` という1行の比較追加のみで、
    // コードレビューで足りる複雑度と判断する。
}
