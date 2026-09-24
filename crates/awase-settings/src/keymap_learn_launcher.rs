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
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};

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

/// 学習プロセスの起動モード(ADR196-T4)。`awase-keymap-learn-win`のフラグに対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnMode {
    /// 通常の学習セッション。
    Learn,
    /// `--adopt-pending-judgement`: 要確認状態の表を採用へ書き換えるだけ(IME駆動なし)。
    AdoptPendingJudgement,
    /// `--revalidate`: 保存済みの表で自己検証だけを走らせる軽量再検証(ADR196-T5)。
    Revalidate,
}

impl LearnMode {
    /// この起動モードの子プロセスが出しうる行か。モードと合わない行(学習中に`adopt`行など)
    /// は無視する——別プロセスの出力の混入や、旧版の出力形式との取り違えで状態表示を
    /// 誤更新しないため。`Revalidate`は自己検証ウォークの進捗行を出しうるので`Progress`も許す。
    #[must_use]
    pub const fn accepts(self, line: &LearnLine) -> bool {
        matches!(
            (self, line),
            (Self::Learn, LearnLine::Progress(_) | LearnLine::Result(_))
                | (Self::AdoptPendingJudgement, LearnLine::Adopt(_))
                | (
                    Self::Revalidate,
                    LearnLine::Progress(_) | LearnLine::Revalidate(_)
                )
        )
    }

    const fn flag(self) -> Option<&'static str> {
        match self {
            Self::Learn => None,
            Self::AdoptPendingJudgement => Some("--adopt-pending-judgement"),
            Self::Revalidate => Some("--revalidate"),
        }
    }
}

/// `revalidate status=..`行の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevalidateOutcome {
    Passed,
    /// 自己検証が採否条件を割り、表が失効した。
    Invalidated,
    Failure,
}

/// 学習プロセスの標準出力1行をパースした結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LearnLine {
    Progress(LearnProgress),
    Result(LearnOutcome),
    /// `adopt status=success|failure`(判定書き換えモード)。`true`が成功。
    Adopt(bool),
    Revalidate(RevalidateOutcome),
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
        "adopt" => match field("status")? {
            "success" => Some(LearnLine::Adopt(true)),
            "failure" => Some(LearnLine::Adopt(false)),
            _ => None,
        },
        "revalidate" => match field("status")? {
            "passed" => Some(LearnLine::Revalidate(RevalidateOutcome::Passed)),
            "invalidated" => Some(LearnLine::Revalidate(RevalidateOutcome::Invalidated)),
            "failure" => Some(LearnLine::Revalidate(RevalidateOutcome::Failure)),
            _ => None,
        },
        _ => None,
    }
}

/// 学習プロセスを`mode`で子プロセスとして起動する。`exe_path`は
/// `awase-keymap-learn-win.exe`のパス。標準出力・標準エラーをパイプで受け取る。
pub fn spawn_learning_process(exe_path: &Path, mode: LearnMode) -> io::Result<Child> {
    let mut command = Command::new(exe_path);
    if let Some(flag) = mode.flag() {
        command.arg(flag);
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// 子プロセスの標準出力を1行ずつ読み、パースできた行だけ`on_line`へ渡す。
/// 子プロセスの終了(EOF)まで呼び出しスレッドをブロックするので、呼び出し側は
/// UIスレッドとは別スレッドで呼ぶこと。
///
/// `Child`本体ではなく`ChildStdout`(`child.stdout.take()`した後の値)を受け取る
/// ことで、呼び出し側は読み取りをブロックしているあいだも`Child`本体
/// (`kill()`/`wait()`用)を別スレッド(UIのキャンセル操作)と共有できる。
///
/// 1行のデコードに失敗しても(非UTF-8等)、その行だけ読み飛ばして読み取りを
/// 継続する——子プロセスの出力全体を1行の乱れだけで諦めない。それ以外の
/// I/Oエラー(パイプの異常切断等)は呼び出し側へ伝える。
pub fn drain_learning_output(
    stdout: ChildStdout,
    mut on_line: impl FnMut(LearnLine),
) -> io::Result<()> {
    for line in BufReader::new(stdout).lines() {
        match line {
            Ok(line) => {
                if let Some(parsed) = parse_learn_line(&line) {
                    on_line(parsed);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 起動済みの子プロセスから、`drain_learning_output`が読める`ChildStdout`を
/// 取り出す。標準出力がパイプ済み(`spawn_learning_process`)でなければエラー。
pub fn take_learning_stdout(child: &mut Child) -> io::Result<ChildStdout> {
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("子プロセスのstdoutが取得できない(既に取得済み?)"))
}

/// [`take_learning_stdout`]の標準エラー版。`awase-keymap-learn-win`は
/// 書き込み失敗時の理由(`print_result_line`のErr分岐)や`decode_errors`警告を
/// 標準エラーへ書くが、標準出力の`result`行にはこの理由が含まれない
/// (code-review指摘: 従来`spawn_learning_process`は標準エラーを`Stdio::null()`で
/// 捨てており、このモジュールのdocコメントが謳う「標準出力・標準エラーをパイプで
/// 受け取る」と実装が食い違っていた)。呼び出し側は失敗理由をユーザーに提示するため、
/// これを別スレッドで読み切ってから使う([`drain_learning_stderr_lines`]参照)。
pub fn take_learning_stderr(child: &mut Child) -> io::Result<ChildStderr> {
    child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("子プロセスのstderrが取得できない(既に取得済み?)"))
}

/// `stderr`を1行ずつ読み、空でない最後の行を返す(無ければ`None`)。
/// `result status=failure`の直前に`print_result_line`が書く1行の理由
/// メッセージをそのままUIへ出すのが目的で、複数行のログを蓄積・解析する
/// 用途は想定しない。
#[must_use]
pub fn drain_learning_stderr_lines(stderr: ChildStderr) -> Option<String> {
    let mut last = None;
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            last = Some(line);
        }
    }
    last
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
            parse_learn_line(
                "result status=success strategy=x elapsed_ms=1 presses=2 cells=3 total=4 decode_errors=0"
            ),
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
    fn parses_adopt_and_revalidate_lines() {
        assert_eq!(
            parse_learn_line("adopt status=success"),
            Some(LearnLine::Adopt(true))
        );
        assert_eq!(
            parse_learn_line("adopt status=failure reason=not_pending"),
            Some(LearnLine::Adopt(false))
        );
        assert_eq!(
            parse_learn_line("revalidate status=passed accuracy=0.970 predicted=310"),
            Some(LearnLine::Revalidate(RevalidateOutcome::Passed))
        );
        assert_eq!(
            parse_learn_line(
                "revalidate status=invalidated reason=LowAccuracy accuracy=0.5 predicted=300"
            ),
            Some(LearnLine::Revalidate(RevalidateOutcome::Invalidated))
        );
        assert_eq!(
            parse_learn_line("revalidate status=failure reason=x"),
            Some(LearnLine::Revalidate(RevalidateOutcome::Failure))
        );
        assert_eq!(parse_learn_line("adopt status=maybe"), None);
    }

    #[test]
    fn each_mode_accepts_only_its_own_lines() {
        let progress = LearnLine::Progress(LearnProgress {
            cell: 1,
            total: 2,
            elapsed_ms: 0.0,
            eta_ms: None,
        });
        let result = LearnLine::Result(LearnOutcome::Success);
        let adopt = LearnLine::Adopt(true);
        let reval = LearnLine::Revalidate(RevalidateOutcome::Passed);
        assert!(LearnMode::Learn.accepts(&progress) && LearnMode::Learn.accepts(&result));
        assert!(!LearnMode::Learn.accepts(&adopt) && !LearnMode::Learn.accepts(&reval));
        assert!(LearnMode::AdoptPendingJudgement.accepts(&adopt));
        assert!(!LearnMode::AdoptPendingJudgement.accepts(&result));
        assert!(!LearnMode::AdoptPendingJudgement.accepts(&reval));
        assert!(LearnMode::Revalidate.accepts(&reval) && LearnMode::Revalidate.accepts(&progress));
        assert!(!LearnMode::Revalidate.accepts(&result) && !LearnMode::Revalidate.accepts(&adopt));
    }

    #[test]
    fn mode_flags_match_learn_win_cli() {
        assert_eq!(LearnMode::Learn.flag(), None);
        assert_eq!(
            LearnMode::AdoptPendingJudgement.flag(),
            Some("--adopt-pending-judgement")
        );
        assert_eq!(LearnMode::Revalidate.flag(), Some("--revalidate"));
    }

    #[test]
    fn unknown_or_malformed_lines_are_ignored() {
        assert_eq!(parse_learn_line(""), None);
        assert_eq!(parse_learn_line("noise from stderr leaking in"), None);
        assert_eq!(parse_learn_line("progress cell=notanumber total=1"), None);
        assert_eq!(
            parse_learn_line("progress cell=1"),
            None,
            "totalが無ければNone"
        );
        assert_eq!(parse_learn_line("result status=unknown_status"), None);
    }

    /// `take_learning_stdout`/`take_learning_stderr`/`drain_learning_output`/
    /// `drain_learning_stderr_lines`をモック子プロセス相手に実際に動かす統合テスト
    /// (docs/tasks/adr195-t6-adr176-wizard-integration.mdの完了条件「子プロセス起動・
    /// 標準出力パースのテスト(Windows実機またはモックプロセスでの検証)」に対応)。
    ///
    /// モック子プロセスとして本物の`awase-keymap-learn-win.exe`は使わず、この
    /// テストバイナリ自身(`std::env::current_exe()`)を、環境変数
    /// `AWASE_MOCK_KEYMAP_LEARN_LINES`/`AWASE_MOCK_KEYMAP_LEARN_STDERR`を渡した上で
    /// `mock_child_entrypoint`という1テストだけをフィルタ実行する形で再起動する。
    /// Windows/Linux両方で「実在するexeパスを渡して実プロセスを起動し、実パイプ経由で
    /// 標準出力/標準エラーを読む」という配管そのものを検証できる(シェルスクリプトや
    /// .batだと`Command::new`がプラットフォームによって解釈を変えるため使わない)。
    #[test]
    fn spawns_real_child_process_and_parses_its_stdout() {
        let exe = std::env::current_exe().expect("current_exe");
        let mock_lines = [
            "progress cell=1 total=2 elapsed_ms=10 eta_ms=5",
            "result status=success strategy=x elapsed_ms=20 presses=1 cells=1 total=2 decode_errors=0",
        ]
        .join("\n");

        let mut child = Command::new(&exe)
            .args([
                "keymap_learn_launcher::tests::mock_child_entrypoint",
                "--exact",
                "--nocapture",
            ])
            .env("AWASE_MOCK_KEYMAP_LEARN_LINES", mock_lines)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("モック子プロセスの起動に失敗");

        let stdout = take_learning_stdout(&mut child).expect("stdoutパイプの取得に失敗");
        let mut lines = Vec::new();
        drain_learning_output(stdout, |line| lines.push(line)).expect("stdoutの読み取りに失敗");
        let status = child.wait().expect("子プロセスの終了待ちに失敗");

        assert!(
            status.success(),
            "モック子プロセスが異常終了した: {status:?}"
        );
        assert_eq!(
            lines,
            vec![
                LearnLine::Progress(LearnProgress {
                    cell: 1,
                    total: 2,
                    elapsed_ms: 10.0,
                    eta_ms: Some(5.0),
                }),
                LearnLine::Result(LearnOutcome::Success),
            ]
        );
    }

    /// `drain_learning_stderr_lines`(失敗理由の抽出)を同じ自己再起動トリックで検証する。
    #[test]
    fn spawns_real_child_process_and_parses_its_stderr_reason() {
        let exe = std::env::current_exe().expect("current_exe");

        let mut child = Command::new(&exe)
            .args([
                "keymap_learn_launcher::tests::mock_child_entrypoint",
                "--exact",
                "--nocapture",
            ])
            .env("AWASE_MOCK_KEYMAP_LEARN_LINES", "result status=failure")
            .env(
                "AWASE_MOCK_KEYMAP_LEARN_STDERR",
                "観測に失敗しました: decode_errors=999",
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("モック子プロセスの起動に失敗");

        let stdout = take_learning_stdout(&mut child).expect("stdoutパイプの取得に失敗");
        let stderr = take_learning_stderr(&mut child).expect("stderrパイプの取得に失敗");
        let mut lines = Vec::new();
        drain_learning_output(stdout, |line| lines.push(line)).expect("stdoutの読み取りに失敗");
        let reason = drain_learning_stderr_lines(stderr);
        let _ = child.wait();

        assert_eq!(lines, vec![LearnLine::Result(LearnOutcome::Failure)]);
        assert_eq!(
            reason.as_deref(),
            Some("観測に失敗しました: decode_errors=999")
        );
    }

    /// 上記2テストの「モック子プロセス」役。`AWASE_MOCK_KEYMAP_LEARN_LINES`が
    /// 設定されているときだけ動作する——素の`cargo test`/`cargo nextest run`で通常
    /// 実行されたときは何もせず即座に成功する、無害な1テストとして振る舞う。
    #[test]
    fn mock_child_entrypoint() {
        let Ok(lines) = std::env::var("AWASE_MOCK_KEYMAP_LEARN_LINES") else {
            return;
        };
        for line in lines.split('\n') {
            println!("{line}");
        }
        if let Ok(stderr_line) = std::env::var("AWASE_MOCK_KEYMAP_LEARN_STDERR") {
            eprintln!("{stderr_line}");
        }
    }
}
