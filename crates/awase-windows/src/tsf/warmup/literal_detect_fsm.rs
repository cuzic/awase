//! literal 回収のパラメータと回収アクションの生成（per-VK confirm 用）。
//!
//! 使うのは `probe_fsm.rs::run_per_vk_confirm`（cold パスの per-VK confirm）だけ。
//!
//! かつてはここに語単位の literal 検出コア（`LiteralDetectCore`）と warm パス用のラッパー
//! （`LiteralDetectFsm`）があったが、ECRS MB-01（cold パスの inline LiteralDetect）と MB-02
//! （warm パスの設置。条件「`TsfGate` が Probing かつ非 Tsf モード」が GitHub の Windows CI で 1 度も
//! 成立しなかった）で使う所が無くなったため撤去した
//! （docs/tasks/mech-ecrs-2026-10-21/probe-mb0203-result.md）。ファイル名は呼び出し元のパスを変えないために残している。

use crate::state::event_origin::Generation;
use crate::tsf::literal_facts::LiteralDetectFacts;
use crate::tsf::warmup::probe_fsm::ProbeAction;

/// IME セッション最初の1文字専用の per-VK confirm ループ（BUG-24 追補）が、
/// ある VK で `SuspectedLiteral`/`StaleConfirm` になったときの回収パラメータを返す純関数。
///
/// `is_stale`: `DetectionResult::StaleConfirm`（epoch fencing 判定）由来なら `true`、
/// `SuspectedLiteral`（deadline 到達）由来なら `false`。
/// `failed_idx`: 何番目 (0-based) の VK が失敗判定になったか。
/// 戻り値: `(backs, escape_composition)`。
///
/// ## VK_BACK には「literal の positive な証拠」を要求する（2026-07-23、実機バグ2件で確認）
///
/// `VK_ESCAPE` の破壊スコープは pending composition 内に閉じている（実機確認済み、
/// `docs/windows-api-constraints.md` §1-2）のに対し、`VK_BACK` は「カーソル直前の
/// 1文字を無条件に消す」命令であり、composition スコープ**外**の既に確定済みの
/// テキストにも届く唯一の不可逆操作である。したがって BS は「本当に literal が
/// 出た」という積極的な証拠がある場合にのみ送る:
///
/// - `is_stale`（`StaleConfirm`）: 「confirm の根拠が古い」ことの検出であり
///   「literal である」ことの証拠では**ない**。idx に関わらず BS は送らない
///   （`backs=0`）。実機で2件確認済み: `escape_composition=true` の場合、ESC が
///   pending composition を破棄した後の BS が消すべき literal を失い、直前の
///   別スコープの確定済み文字（「リーク」の「ク」、「cold 」の末尾スペース）を
///   誤って消していた（`docs/known-bugs.md` BUG-33 追補3・4参照）。
/// - `!is_stale && failed_idx > 0`（`SuspectedLiteral`、直前の VK は fresh に
///   confirmed 済み）: GJI が「いま」この語を処理している直接証拠がある以上、
///   その直後の1VKだけが本当に IME をバイパスすることは実質考えにくい。BS は
///   送らず ESC のみに倒す（`escape_composition=true`、直前 VK の pending
///   composition を破棄）。
/// - `!is_stale && failed_idx == 0`（この語で一度も confirm 済みの証拠がない
///   唯一のケース）: 従来どおり `backs=1`（composition 自体が存在しないため
///   `escape_composition=false`）。
///
/// `escape_composition` は `failed_idx > 0` のときのみ `true`（BS の有無に関わらず、
/// それより前の VK が個別に `CompositionConfirmed` 済み＝pending composition が
/// 実在するため）。
pub(crate) const fn per_vk_recovery_params(is_stale: bool, failed_idx: usize) -> (usize, bool) {
    let backs = if is_stale || failed_idx > 0 { 0 } else { 1 };
    (backs, failed_idx > 0)
}

/// literal 回収用アクション列を生成する（per-VK confirm 用）。
///
/// 常に `RawTsfLiteralRecovery + Done`（backspace のみ、捨て駒キーには頼らない）。
/// `RawTsfLiteralRecovery` の dispatcher が `consecutive_count()==0` のときだけ
/// romaji の再送を `RAW_TSF_LITERAL` 経由でスケジュールする（`output/mod.rs::
/// record_raw_tsf_literal` → 次イベントで `send_romaji_batched_bypass_gate` を通常の cold パス
/// として再実行）。cold パスは per-VK confirm がデフォルトのため、この再送は
/// 自然に per-VK として実行される — 1文字失敗した後の再送も per-VK のままにする、
/// という設計（ユーザー方針、2026-07-16。以前あった「TSF mode かつ consecutive==0
/// → SendRecoveryBs + StartSacrificialWarmup」分岐は撤去した。捨て駒キー
/// （VK_A+BS/VK_IME_OFF→ON）は cold-start の予防用途としても失敗リカバリ用途
/// としても、もう本経路からは発行されない）。
///
/// `escape_composition`: `true` の場合、dispatcher はバックスペースの前に `VK_ESCAPE` を送って
/// composition を確実に破棄する（[`per_vk_recovery_params`] の `escape_composition` 参照）。
pub(crate) fn emit_recovery_actions(
    cold_seq: Generation,
    romaji: String,
    backs: usize,
    escape_composition: bool,
    facts: LiteralDetectFacts,
) -> Vec<ProbeAction> {
    vec![
        ProbeAction::RawTsfLiteralRecovery {
            cold_seq,
            backs,
            romaji,
            escape_composition,
            facts,
        },
        ProbeAction::Done,
    ]
}

// ── テスト ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::literal_facts::{
        DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralVerdict,
    };

    #[test]
    fn per_vk_recovery_params_first_vk_suspected_literal_backs_1_no_escape() {
        assert_eq!(
            per_vk_recovery_params(false, 0),
            (1, false),
            "先頭 VK の SuspectedLiteral（この語で一度も confirm 済みの証拠がない \
             唯一のケース）は composition が存在しないため ESC 不要、literal の \
             positive な証拠として backs=1"
        );
    }

    // BUG-33 追補4: 2番目以降の VK の SuspectedLiteral は「直前の VK が fresh に
    // confirmed 済み」という正の証拠があるため、もはや backspace を送らない
    // （以前は backs=1 で送っていたが、実機で「cold 」の末尾スペースを誤って
    // 消す実害が確認された）。ESC のみで pending composition を破棄する。
    #[test]
    fn per_vk_recovery_params_later_vk_suspected_literal_escapes_without_backspace() {
        assert_eq!(
            per_vk_recovery_params(false, 1),
            (0, true),
            "2番目以降の VK の SuspectedLiteral は先行 VK による composition を \
             ESC で破棄するが、backspace は送らない"
        );
        assert_eq!(per_vk_recovery_params(false, 2), (0, true));
    }

    // BUG-33 追補4: StaleConfirm は「confirm 根拠が古い」ことの検出であって
    // 「literal である」証拠ではないため、idx に関わらず backspace を送らない。
    #[test]
    fn per_vk_recovery_params_stale_confirm_never_backspaces() {
        assert_eq!(
            per_vk_recovery_params(true, 0),
            (0, false),
            "先頭 VK の StaleConfirm は composition も存在しないため ESC も不要"
        );
        assert_eq!(
            per_vk_recovery_params(true, 1),
            (0, true),
            "2番目以降の VK の StaleConfirm は ESC のみ、backspace は送らない"
        );
    }

    fn test_facts() -> LiteralDetectFacts {
        LiteralDetectFacts {
            verdict: LiteralVerdict::SuspectedLiteral,
            route: DetectRoute::CheckNow,
            path: DetectPath::PerVk,
            target: DetectTarget::Tsf,
            vk: None,
            idx: 0,
            last_idx: 0,
            evidence: DetectEvidence::default(),
        }
    }

    // escape_composition=true で emit した recovery アクションがその値を持つことを確認する
    // （2026-07-10 追加: ESC-based composition 回収）。
    // 2026-07-16: 捨て駒キー撤去に伴い emit_recovery_actions は常に RawTsfLiteralRecovery を
    // 返すようになった（consecutive による分岐は dispatcher 側 `probe_io.rs` に一本化）。
    #[test]
    fn emit_recovery_actions_keeps_escape_composition_true() {
        let actions = emit_recovery_actions(
            Generation::INITIAL,
            "ltu".to_string(),
            0,
            true,
            test_facts(),
        );
        match &actions[0] {
            ProbeAction::RawTsfLiteralRecovery {
                escape_composition, ..
            } => assert!(
                *escape_composition,
                "escape_composition=true が保たれるべき"
            ),
            other => panic!("expected RawTsfLiteralRecovery, got {other:?}"),
        }
    }

    // SuspectedLiteral（全部 literal 化）は escape_composition=false のままであるべき
    // （composition が存在しないため ESC は不要、既存の chars.len() ベース BS のみ）。
    #[test]
    fn emit_recovery_actions_suspected_literal_keeps_escape_composition_false() {
        let actions = emit_recovery_actions(
            Generation::INITIAL,
            "ko".to_string(),
            2,
            false,
            test_facts(),
        );
        match &actions[0] {
            ProbeAction::RawTsfLiteralRecovery {
                escape_composition, ..
            } => assert!(
                !*escape_composition,
                "SuspectedLiteral 回収は escape_composition=false であるべき"
            ),
            other => panic!("expected RawTsfLiteralRecovery, got {other:?}"),
        }
    }
}
