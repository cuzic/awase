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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}
