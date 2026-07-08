//! awase-macos-probe: macOS API 契約検証用の診断バイナリのエントリポイント。
//!
//! ここではサブコマンドのディスパッチのみを行う。各サブコマンドの実処理は
//! 対応するモジュール（permissions.rs / tap.rs / output.rs / input_source.rs /
//! focus.rs / keys.rs / runtime.rs）側に置く方針とし、このファイルの構造は
//! 安定させる（他モジュールの実装追加のために編集し直す必要がないようにする）。
//!
//! 設計の背景は以下のメモリを参照:
//! - project_macos_probe_interfaces.md（確定インタフェース）
//! - project_macos_port_strategy.md（実装方針・Phase M0検証項目）

mod focus;
mod input_source;
mod keys;
mod layout_probe;
mod output;
mod permissions;
mod report;
mod runtime;
mod sleep_wake;
mod synthetic;
mod tap;
mod tis_sys;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log_environment();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let parts: Vec<&str> = args.iter().map(String::as_str).collect();
    dispatch(&parts);
}

/// 起動のたびに実行環境（OS version/build/architecture/bundle id 等）を JSONL で
/// 1 行ログする。複数実機の結果を後から突き合わせるための最小限の環境記録。
/// tap 固有フィールドはサブコマンドごとに異なり起動時点では未確定なため `"n/a"`。
fn log_environment() {
    let env = report::ProbeEnvironment::capture(report::ProbeTapConfig {
        tap_location: "n/a".to_owned(),
        tap_placement: "n/a".to_owned(),
        tap_options: "n/a".to_owned(),
        event_mask: 0,
        post_location: "n/a".to_owned(),
    });
    log::info!("{}", env.to_json_line());
}

/// サブコマンド名から各モジュールの実処理へのディスパッチ。
fn dispatch(parts: &[&str]) {
    match parts {
        ["permissions", "status"] => {
            log::info!("{:?}", permissions::check_all());
            log::info!("{:?}", permissions::compare_listen_event_checks());
        }
        ["permissions", "request", "listen"] => {
            log::info!("listen: {:?}", permissions::request_listen_events());
        }
        ["permissions", "request", "post"] => {
            log::info!("post: {:?}", permissions::request_post_events());
        }
        ["permissions", "request", "accessibility"] => {
            log::info!("accessibility: {:?}", permissions::request_accessibility());
        }
        ["permissions", "request", "all"] => {
            log::info!("listen: {:?}", permissions::request_listen_events());
            log::info!("post: {:?}", permissions::request_post_events());
            log::info!("accessibility: {:?}", permissions::request_accessibility());
        }
        ["permissions", ..] => {
            log::error!(
                "usage: permissions status | permissions request <listen|post|accessibility|all>"
            );
        }
        ["tap-dump", rest @ ..] => report_result("tap-dump", tap::run_tap_dump(rest)),
        ["tap-recover", rest @ ..] => report_result("tap-recover", tap::run_tap_recover(rest)),
        ["output-test", rest @ ..] => run_output_test(rest),
        ["input-sources"] => {
            report_result("input-sources", input_source::run_input_sources_cli());
        }
        ["layout-probe"] => layout_probe::run(),
        ["input-watch"] => report_result("input-watch", input_source::run_input_watch_cli()),
        ["focus-watch"] => report_result("focus-watch", focus::run_focus_watch()),
        ["sleep-wake", rest @ ..] => report_result("sleep-wake", sleep_wake::run_sleep_wake(rest)),
        [] => print_usage(),
        other => {
            log::error!("unknown subcommand: {}", other.join(" "));
            print_usage();
        }
    }
}

/// `output-test <mode>` の `<mode>` を解決して `output::run_output_test` へ渡す。
/// `unicode:` モードは空白を含みうるため、残り引数を空白区切りで結合してから解析する。
fn run_output_test(rest: &[&str]) {
    if rest.is_empty() {
        log::error!("usage: output-test <passthrough|suppress|substitute:<key>|unicode:<str>>");
        return;
    }
    let mode_str = rest.join(" ");
    match mode_str.parse() {
        Ok(mode) => report_result("output-test", output::run_output_test(mode)),
        Err(e) => log::error!("output-test: invalid mode {mode_str:?}: {e}"),
    }
}

/// サブコマンドの `anyhow::Result` を統一形式でログする。
fn report_result(name: &str, result: anyhow::Result<()>) {
    if let Err(e) = result {
        log::error!("{name} failed: {e:#}");
    }
}

fn print_usage() {
    log::info!("usage: awase-macos-probe <subcommand> [args]");
    log::info!("subcommands:");
    log::info!("  permissions status");
    log::info!("  permissions request <listen|post|accessibility|all>");
    log::info!("  tap-dump");
    log::info!("  tap-recover");
    log::info!("  output-test");
    log::info!("  input-sources");
    log::info!("  layout-probe");
    log::info!("  input-watch");
    log::info!("  focus-watch");
    log::info!("  sleep-wake");
}
