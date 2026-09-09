//! ADR-161「機構の選定指針」実証実験3〜6の利用側デモ（恒久化しない）。
//!
//! `ChokePoint` は、これまでの実証実験（`crates/xtask-spike`のsynスキャン、
//! `lints/actuation_call_guard_spike`のdylint試作）で実際に確認した実データを使う。

use macro_spike::{actuation_choke_point, choke_points, measured, MarkdownRow};

#[derive(Debug, MarkdownRow)]
pub struct ChokePoint {
    callee: &'static str,
    callers: &'static [&'static str],
    adr: &'static str,
}

// 実験4: 関数形式手続きマクロによるDSL宣言。
// callee/callersの値は、xtask-spike(synスキャン)とactuation_call_guard_spike(dylint)の
// 実測結果そのもの。
choke_points! {
    "apply_ime_open_with_view" => ["platform.rs::apply_ime_open_with_belief", "runtime/executor.rs", "runtime/mod.rs::force_on_and_correct_romaji", "runtime/mod.rs::reassert_explicit_physical_key"], adr: "ADR-119, ADR-121";
    "apply_ime_open_with_belief" => ["runtime/key_pipeline.rs", "runtime/ime_refresh.rs"], adr: "ADR-090";
    "set_ime_open" => ["set_ime_open_ordered", "PlatformRuntime::apply_ime_open(デフォルト実装、ADR-087 Phase3判断待ち)"], adr: "ADR-090 A-1, ADR-159実証実験";
}

// 実験5: 属性マクロ(実行時記録版)。set_ime_openの実際の許可呼び出し元
// (set_ime_open_ordered)を模した関数に付ける。dylintのように呼び出しを禁止はしないが、
// 呼ばれるたびに呼び出し元のfile:lineを記録する。
#[actuation_choke_point(callers = "set_ime_open_ordered")]
fn set_ime_open_demo(open: bool) -> bool {
    open
}

// 実験6: 属性マクロ(メタデータ強制版)。value_ms/commitが両方揃っているので
// コンパイルが通るはずの正常系。
#[measured(value_ms = 181, margin_ms = 169, commit = "9a7e699")]
#[allow(dead_code)]
const GJI_LONG_IDLE_PROBE_TOTAL_MS_DEMO: u64 = 350;

// 実験6(異常系、意図的にコメントアウトで保持): value_msを欠いた場合の挙動を
// 一度コンパイルして確認済み。エラーメッセージ:
//   error: #[measured(...)] には value_ms が必須です
//   (.claude/rules/tuning-constants.md: 実測msを書かずに定数を変更してはならない)
// #[measured(commit = "abc123")]
// const BAD_CONST_MISSING_VALUE_MS: u64 = 100;

fn main() {
    println!("=== 実験4(関数形式手続きマクロ): CHOKE_POINTS が実際に構築できたか ===");
    println!("エントリ数: {}", CHOKE_POINTS.len());

    println!();
    println!("=== 実験3(deriveマクロ): to_markdown_row() で生成したMarkdown表 ===");
    println!("| callee | callers | adr |");
    println!("|---|---|---|");
    for cp in &CHOKE_POINTS {
        println!("{}", cp.to_markdown_row());
    }

    println!();
    println!("=== 実験5(属性マクロ・実行時記録版): 呼び出し元のfile:lineを記録 ===");
    // 「許可された」呼び出し元からの1回
    let _ = set_ime_open_demo(true);
    // 「許可されていない」呼び出し元からの1回(実際にはdylintならここでコンパイルエラーになる)
    let _ = rogue_caller();

    println!();
    println!("=== 実験6(属性マクロ・メタデータ強制版): 正常系がコンパイルを通ったことを確認 ===");
    println!(
        "GJI_LONG_IDLE_PROBE_TOTAL_MS_DEMO = {GJI_LONG_IDLE_PROBE_TOTAL_MS_DEMO} \
         (#[measured(value_ms=181, margin_ms=169, commit=\"9a7e699\")] が正常に展開された)"
    );
}

fn rogue_caller() -> bool {
    // 許可リストに無い場所からの呼び出し。
    set_ime_open_demo(false)
}
