//! フォーカス時の IMM32 制御能力の学習（`ImmGetDefaultIMEWnd` の初回判定）の決定（純粋）。
//!
//! `focus/imm_learning.rs::learn_imm_capability_on_focus` に埋め込まれていた判断（学習するか／しないか、
//! プローブ結果をどう記録するか）を、OS・キャッシュを読まない純粋関数として切り出したもの
//! （FCIS F5d、ADR-229）。殻は `observe`（Win32 のときだけ `process_name` を評価 → 空でなければ学習済みか
//! を引く。読む順序と回数、`process_name` の評価が**ちょうど 1 回・Win32 判定の直後**であることは元のまま）→
//! [`plan_imm_learning`] → （`Probe` のときだけ）`ImmGetDefaultIMEWnd` → [`plan_probe_result`] →
//! `execute`（`record_imm_null_probe` / `clear_imm_pending_unavailable`）。
//!
//! 学習キーを `(process_name, class_name)` にして、空のプロセス名では学習しない（BUG-56 の単発 NULL
//! 誤確定と BUG-107 の `class_name` 単独キーの汚染を避ける）規則は、ここでは変えずに固定している。
//! 閾値回連続で確定する処理自体は `ImmCapabilityStore::record_null_probe`（殻の先）が持つ。

use crate::focus::AppKind;

/// 学習しない理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImmLearnSkip {
    /// `Win32` 以外の AppKind（TSF/UWP は IMM32 の学習対象外）
    NotWin32,
    /// プロセス名を解決できなかった。空文字列を共有バケツにすると BUG-107 を別の軸で再現するため諦める。
    EmptyProcessName,
    /// `(process_name, class_name)` は学習済み（確定済み）
    AlreadyLearned,
}

/// [`plan_imm_learning`] の決定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImmLearnPlan {
    Skip(ImmLearnSkip),
    /// `ImmGetDefaultIMEWnd` を呼んで結果を [`plan_probe_result`] に渡す。
    Probe,
}

/// 学習するかを決める。判定順は `Win32` か → プロセス名が空か → 学習済みか。
///
/// 殻は `process_name` を `Win32` のときだけ評価し、`already_learned` はプロセス名が空でないときだけ
/// 引く（読まなかった分は空文字列 / `false` のまま渡す。優先順位で無視される）。
pub(crate) const fn plan_imm_learning(
    app_kind: AppKind,
    process_name_is_empty: bool,
    already_learned: bool,
) -> ImmLearnPlan {
    if !matches!(app_kind, AppKind::Win32) {
        return ImmLearnPlan::Skip(ImmLearnSkip::NotWin32);
    }
    if process_name_is_empty {
        return ImmLearnPlan::Skip(ImmLearnSkip::EmptyProcessName);
    }
    if already_learned {
        return ImmLearnPlan::Skip(ImmLearnSkip::AlreadyLearned);
    }
    ImmLearnPlan::Probe
}

/// プローブ結果の記録方法
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImmProbeRecord {
    /// `ImmGetDefaultIMEWnd` = NULL: 疑いを 1 回分記録（閾値回連続で確定、BUG-56）
    RecordNullProbe,
    /// 非 NULL: 疑いのカウントをクリア
    ClearPending,
}

/// `ImmGetDefaultIMEWnd` の結果（NULL か）から記録方法を決める。
pub(crate) const fn plan_probe_result(ime_wnd_is_null: bool) -> ImmProbeRecord {
    if ime_wnd_is_null {
        ImmProbeRecord::RecordNullProbe
    } else {
        ImmProbeRecord::ClearPending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [AppKind; 3] = [AppKind::Win32, AppKind::TsfNative, AppKind::Uwp];

    /// 元の早期 return 3 連（kind 3 × 空名 2 × 学習済み 2 = 12 通り）を独立に書いた期待値で固定する。
    #[test]
    fn exhaustive_plan_table() {
        for kind in KINDS {
            for empty in [false, true] {
                for learned in [false, true] {
                    let got = plan_imm_learning(kind, empty, learned);
                    let want = if kind != AppKind::Win32 {
                        ImmLearnPlan::Skip(ImmLearnSkip::NotWin32)
                    } else if empty {
                        ImmLearnPlan::Skip(ImmLearnSkip::EmptyProcessName)
                    } else if learned {
                        ImmLearnPlan::Skip(ImmLearnSkip::AlreadyLearned)
                    } else {
                        ImmLearnPlan::Probe
                    };
                    assert_eq!(got, want, "{kind:?} empty={empty} learned={learned}");
                }
            }
        }
    }

    /// BUG-107: 空のプロセス名では（学習済みかどうかに関わらず）決して Probe しない。
    #[test]
    fn empty_process_name_never_probes() {
        for kind in KINDS {
            for learned in [false, true] {
                assert_ne!(plan_imm_learning(kind, true, learned), ImmLearnPlan::Probe);
            }
        }
    }

    /// BUG-56: NULL は「確定」ではなく「疑いの記録」、非 NULL はカウントのクリア。
    #[test]
    fn probe_result_maps_null_to_suspicion_and_non_null_to_clear() {
        assert_eq!(plan_probe_result(true), ImmProbeRecord::RecordNullProbe);
        assert_eq!(plan_probe_result(false), ImmProbeRecord::ClearPending);
    }
}
