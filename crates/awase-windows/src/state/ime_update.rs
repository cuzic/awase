//! `classify_ime_snapshot()` が返す状態更新命令と観測値(純粋なデータ型)。

use awase::engine::InputModeState;

/// Observer が返す単一観測 (値 + タイムスタンプ)。
#[derive(Debug, Clone, Copy)]
pub struct ImeObs {
    pub value: bool,
    pub ms: u64,
}

/// `classify_ime_snapshot()` が返す状態更新命令。
///
/// 副作用なし・純粋変換の結果を表す。
/// 呼び出し元（`PlatformState::apply_ime_update()`）が状態に反映する。
#[derive(Debug)]
pub struct ImeUpdate {
    /// 検出された is_japanese_ime（`Some` のときのみ更新すべき）
    pub is_japanese_ime: Option<bool>,
    /// `observer_poll` スロットに書くべき値（`Some` のときのみ書く）
    pub observer_poll: Option<ImeObs>,
    /// miss_count を 1 インクリメントすべきか
    pub increment_miss_count: bool,
    /// `force_on_panic_reset` フラグと miss_count をリセットすべきか（検出成功時）
    pub clear_force_on_panic_reset: bool,
    /// `input_mode` に適用すべき新しい値（`Some` のときのみ更新すべき）
    pub new_input_mode: Option<InputModeState>,
    /// `prev_conversion_mode`(直近に観測した conv。読み手は予測の入力 `conv_raw` だけ、ADR-239)に書くべき値
    /// （`Some` のときのみ更新すべき）
    pub new_prev_conversion_mode: Option<u32>,
    /// 英数モードの候補の更新(ADR-238、BUG-190)。
    pub eisu_candidate: crate::state::eisu_candidate::CandidateUpdate,
}
