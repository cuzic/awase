//! IME 種別の ungated 表現（ADR-089 §2.8「K 軸の型」、INV-45）。
//!
//! `tsf::observer::ActiveImeKind` は `#[cfg(windows)]` の下にあり Linux から
//! 参照できない。`caps(p, k)` を Linux で全数テストできるようにするため、
//! state 層に同じ 2 値を ungated で置く。両者の変換は Windows 境界
//! （`tsf/observer.rs` の `From<ActiveImeKind> for ImeKindId`）1 箇所だけに置く。

/// フォアグラウンドで使用中の IME 種別。
///
/// **これは観測値ではなく推定値である**（ADR-089 §1.3(g)、INV-45）。
/// `MsIme` は「MS-IME を観測した」ではなく「**GJI を検出できなかった**」を
/// 意味する（`tsf/observer.rs` の `ActiveImeKind` doc）。GJI 起動直後・
/// フォーカス直後の未検出ウィンドウでは、GJI 環境でも `MsIme` になりうる。
///
/// したがって、この値で分岐してよいのは **誤っても被害が対称な選択** だけで
/// ある（原則 P20）。`GjiFsm` 同期義務のような「閉じ損ねると同期が落ちる」
/// ゲートに使ってはならない（ADR-089 §4.3、INV-42）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ImeKindId {
    /// Google 日本語入力を検出済み。
    Gji,
    /// GJI 非検出 — MS-IME（または互換 IME）と**推定**。
    MsIme,
}

impl ImeKindId {
    /// 全 variant。`caps(p, k)` の全数テスト用。
    pub const ALL: [Self; 2] = [Self::Gji, Self::MsIme];
}

/// Microsoft IME（日本語、TSF の TIP）の CLSID。`{03B5835F-F03C-411B-9CE2-AA23E1171E36}`。
///
/// 打鍵時予測の表（Microsoft IME本体用）とベリーフトグルを当てる相手を、「GJI ではない」ではなく
/// 「この CLSID の TIP」と**厳密に**同定するために使う（ADR-191、レビュー round2 NB1）。
pub const MS_IME_JA_TIP_CLSID: u128 = 0x03B5_835F_F03C_411B_9CE2_AA23_E117_1E36;

/// アクティブな入力方式（TSF の TIP または IMM32 HKL）の同定結果。
///
/// `ImeKindId::MsIme`（「GJI を検出できなかった」の意味）と違い、こちらは**明示的に同定できたか**を表す。
/// `tsf::tip_detector::query_active_kind` は GJI 以外の全 TIP と IMM32 HKL に `ActiveImeKind::MicrosoftIme`
/// を返す（ATOK・Japanist・WXG・他言語 TIP を含む）ので、`ActiveImeKind` だけでは区別できない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipIdentity {
    /// Google 日本語入力（CLSID 一致）。
    Gji,
    /// Microsoft IME 本体（CLSID 一致）。
    MsImeNative,
    /// それ以外（ATOK・Japanist・未知の TIP、IMM32 HKL のみ）。表を当てない。
    Other,
}

/// TIP の CLSID（`None` = TIP でない IMM32 HKL）から同定する。純関数。
#[must_use]
pub const fn identify_tip(clsid: Option<u128>, gji_clsid: Option<u128>) -> TipIdentity {
    let Some(c) = clsid else {
        return TipIdentity::Other;
    };
    if let Some(g) = gji_clsid {
        if c == g {
            return TipIdentity::Gji;
        }
    }
    if c == MS_IME_JA_TIP_CLSID {
        TipIdentity::MsImeNative
    } else {
        TipIdentity::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_covers_every_variant() {
        for k in ImeKindId::ALL {
            // match の網羅性で「ALL に載せ忘れた variant」を検出する。
            match k {
                ImeKindId::Gji | ImeKindId::MsIme => {}
            }
        }
        assert_eq!(ImeKindId::ALL.len(), 2);
    }

    #[test]
    fn identify_tip_distinguishes_gji_ms_ime_and_everything_else() {
        let gji = 0x1234_5678_9ABC_DEF0_1234_5678_9ABC_DEF0_u128;
        let atok = 0x0000_0001_0000_0002_0000_0003_0000_0004_u128; // ATOK 等の第三者 TIP（CLSID は任意の別値）
                                                                   // GJI（CLSID 一致）
        assert_eq!(identify_tip(Some(gji), Some(gji)), TipIdentity::Gji);
        // Microsoft IME 本体（CLSID 一致）。GJI の CLSID が未取得でも同定できる
        assert_eq!(
            identify_tip(Some(MS_IME_JA_TIP_CLSID), Some(gji)),
            TipIdentity::MsImeNative
        );
        assert_eq!(
            identify_tip(Some(MS_IME_JA_TIP_CLSID), None),
            TipIdentity::MsImeNative
        );
        // ATOK / Japanist / 未知の TIP は表を当てない
        assert_eq!(identify_tip(Some(atok), Some(gji)), TipIdentity::Other);
        assert_eq!(identify_tip(Some(atok), None), TipIdentity::Other);
        // IMM32 HKL のみ（TIP でない）も表を当てない
        assert_eq!(identify_tip(None, Some(gji)), TipIdentity::Other);
        assert_eq!(identify_tip(None, None), TipIdentity::Other);
    }
}
