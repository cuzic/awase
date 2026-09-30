//! MS-IME本体の再変換しうるセルを、予測から外す(ADR-210 Opusレビュー R6)。
//!
//! MS-IME本体は、EDITに文字があるとアイドル状態(入力中でない)の変換キー(0x1C)で再変換に入る
//! (入力中になる)。学習は測定前にEDITを消して「文書が空」の世界で測るため、そこで学んだ
//! 「アイドルで変換キー→入力中にならない」は、実行時に文字が残ったアプリでは成り立たない。
//! 実行時の予測を誤らせないよう、これらのセルの予測を空にする(実行時は同梱表へ縮退する)。
//!
//! OS非依存なのでLinuxでもユニットテストできる。

use awase_keymap_learn::persist::PersistedCell;

/// 変換キーのVK(`VK_CONVERT`)。
pub const HENKAN_VK: u16 = 0x1C;

/// アイドル状態(入力中でない)の変換キーのセルの予測を空にする。空にした件数を返す。
pub fn blank_idle_reconvert_predictions(cells: &mut [PersistedCell]) -> usize {
    let mut blanked = 0;
    for c in cells.iter_mut() {
        if c.key.0 == HENKAN_VK && !c.status.composing && c.prediction.is_some() {
            c.prediction = None;
            blanked += 1;
        }
    }
    blanked
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase_keymap_learn::model::{Disposition, KeyId, Outcome, Status};

    fn cell(vk: u16, composing: bool, predicted: bool) -> PersistedCell {
        let status = Status {
            open: true,
            mode: 0x09,
            composing,
        };
        PersistedCell {
            status,
            key: KeyId(vk),
            prediction: predicted.then_some(Outcome {
                status,
                disp: Disposition::None,
            }),
        }
    }

    #[test]
    fn blanks_only_idle_henkan_cells() {
        let mut cells = vec![
            cell(HENKAN_VK, false, true),  // 対象
            cell(HENKAN_VK, true, true),   // 入力中は再変換ではない
            cell(0x1D, false, true),       // 別のキー
            cell(HENKAN_VK, false, false), // 既に予測なし
        ];
        assert_eq!(blank_idle_reconvert_predictions(&mut cells), 1);
        assert_eq!(cells[0].prediction, None);
        assert!(cells[1].prediction.is_some());
        assert!(cells[2].prediction.is_some());
    }
}
