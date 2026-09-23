//! ADR-195段階6: 較正ウィザード(awase-settings)から学習プロセス
//! (`awase-keymap-learn-win`)を子プロセスとして起動し、標準出力の進捗行を
//! パースする。IPC(`calibration_ipc.rs`、ADR-176実装)は使わない——ペイロードが
//! 1ワード固定で表本体を運べないため。ここで運ぶのは進捗(現在何セル目/推定
//! 残り時間)と成否だけで、表本体(`KeymapCache`と同じfsスタンプ再チェックで
//! 反映される、ADR195-T4のスコープ)は一切運ばない。
//!
//! 対象プロセスの一時停止は不要: 学習プロセスの実行ファイル名は固定のため、
//! awase.exe側はADR195-T1のコード内定数照合(`is_keymap_learn_process_name`)で
//! 恒久的に無効化している。起動・終了のたびにawase.exeへ何かを要求する必要は
//! ない(動的バイパス要求・keepalive・タイムアウトいずれも無し)。

use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// 学習プロセスからの1行分の進捗。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LearnProgress {
    /// 1回以上測定済みのセル数。
    pub cell: u32,
    /// 全セル数(状態数×キー数)。
    pub total: u32,
    pub elapsed_ms: f64,
    /// 残り時間の単純な線形外挿。未進捗・全セル完了時は`None`。
    pub eta_ms: Option<f64>,
}

/// 学習プロセスの最終結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnOutcome {
    Success,
    /// 一部`decode_error`はあったが表は得られた(実機ドライバ側の一時的な観測失敗)。
    SuccessWithWarnings,
    Failure,
}

/// 学習プロセスの標準出力1行をパースした結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LearnLine {
    Progress(LearnProgress),
    Result(LearnOutcome),
}

/// `awase-keymap-learn-win`が標準出力へ書く
/// `progress cell=.. total=.. elapsed_ms=.. eta_ms=..` /
/// `result status=.. ..` 形式の1行をパースする。未知の行・壊れた行は`None`
/// (呼び出し側は黙って無視してよい——学習プロセスの出力フォーマットが
/// 先行して変わっても較正ウィザードをクラッシュさせないため)。
pub fn parse_learn_line(line: &str) -> Option<LearnLine> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    let fields: Vec<(&str, &str)> = parts.filter_map(|kv| kv.split_once('=')).collect();
    let field = |key: &str| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);

    match kind {
        "progress" => {
            let cell: u32 = field("cell")?.parse().ok()?;
            let total: u32 = field("total")?.parse().ok()?;
            let elapsed_ms: f64 = field("elapsed_ms")?.parse().ok()?;
            let eta_ms = field("eta_ms")
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|ms| *ms >= 0.0);
            Some(LearnLine::Progress(LearnProgress {
                cell,
                total,
                elapsed_ms,
                eta_ms,
            }))
        }
        "result" => match field("status")? {
            "success" => Some(LearnLine::Result(LearnOutcome::Success)),
            "success_with_warnings" => Some(LearnLine::Result(LearnOutcome::SuccessWithWarnings)),
            "failure" => Some(LearnLine::Result(LearnOutcome::Failure)),
            _ => None,
        },
        _ => None,
    }
}

/// 学習プロセスを子プロセスとして起動する。`exe_path`は
/// `awase-keymap-learn-win.exe`のパス。標準出力・標準エラーをパイプで受け取る。
pub fn spawn_learning_process(exe_path: &Path) -> io::Result<Child> {
    Command::new(exe_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// 起動した子プロセスの標準出力を1行ずつ読み、パースできた行だけ`on_line`へ
/// 渡す。子プロセスの終了(EOF)まで呼び出しスレッドをブロックするので、
/// 呼び出し側はUIスレッドとは別スレッドで呼ぶこと。
pub fn drain_learning_output(
    child: &mut Child,
    mut on_line: impl FnMut(LearnLine),
) -> io::Result<()> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("子プロセスのstdoutが取得できない(既に取得済み?)"))?;
    for line in BufReader::new(stdout).lines() {
        if let Some(parsed) = parse_learn_line(&line?) {
            on_line(parsed);
        }
    }
    Ok(())
}

/// ADR-195段階6決定5: 検出したキーマップ構成が同梱の3種(ATOK/
/// GJI+MS-IMEプリセット/Microsoft IME本体、いずれもカスタム設定なし)と
/// 一致する場合、awase-settingsは学習の実行を積極的に案内しない(実行自体は
/// 妨げないが、既定の導線に出さない)。
///
/// 構成の検出自体はこの関数の責務外——経路1〜3の統合は
/// [ADR195-T0](../../../../docs/tasks/adr195-t0-config-reading-integration.md)、
/// 実行時の「同梱表とのセル突き合わせ」による一致判定は
/// [ADR195-T4](../../../../docs/tasks/adr195-t4-runtime-loading.md)が持つ。
/// 本関数はそれらが確定させた「一致/不一致」の結果を受け取って、UI導線に
/// 出すかどうかだけを決める(検出ロジックの二重実装を避けるため)。
///
/// 呼び出し元は未配線(`#[allow(dead_code)]`): ADR195-T4の「同梱表とのセル
/// 突き合わせ」判定が実装されるまで、実際に渡せる`matches_bundled_preset`が
/// 存在しない。T4実装後、ここから`keymap_learn_wizard_ui`の表示条件へ配線する。
#[allow(dead_code)]
pub const fn should_recommend_learning(matches_bundled_preset: bool) -> bool {
    !matches_bundled_preset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_line() {
        let line = "progress cell=42 total=280 elapsed_ms=12345 eta_ms=6789";
        assert_eq!(
            parse_learn_line(line),
            Some(LearnLine::Progress(LearnProgress {
                cell: 42,
                total: 280,
                elapsed_ms: 12345.0,
                eta_ms: Some(6789.0),
            }))
        );
    }

    #[test]
    fn negative_eta_becomes_none() {
        let line = "progress cell=0 total=280 elapsed_ms=0 eta_ms=-1";
        let Some(LearnLine::Progress(p)) = parse_learn_line(line) else {
            panic!("progress行としてパースできるはず");
        };
        assert_eq!(p.eta_ms, None);
    }

    #[test]
    fn parses_result_lines() {
        assert_eq!(
            parse_learn_line("result status=success strategy=x elapsed_ms=1 presses=2 cells=3 total=4 decode_errors=0"),
            Some(LearnLine::Result(LearnOutcome::Success))
        );
        assert_eq!(
            parse_learn_line("result status=success_with_warnings decode_errors=3"),
            Some(LearnLine::Result(LearnOutcome::SuccessWithWarnings))
        );
        assert_eq!(
            parse_learn_line("result status=failure"),
            Some(LearnLine::Result(LearnOutcome::Failure))
        );
    }

    #[test]
    fn unknown_or_malformed_lines_are_ignored() {
        assert_eq!(parse_learn_line(""), None);
        assert_eq!(parse_learn_line("noise from stderr leaking in"), None);
        assert_eq!(parse_learn_line("progress cell=notanumber total=1"), None);
        assert_eq!(parse_learn_line("progress cell=1"), None, "totalが無ければNone");
        assert_eq!(parse_learn_line("result status=unknown_status"), None);
    }

    #[test]
    fn should_recommend_learning_only_for_non_bundled_configs() {
        assert!(should_recommend_learning(false));
        assert!(!should_recommend_learning(true));
    }
}
