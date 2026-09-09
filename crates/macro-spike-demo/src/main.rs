//! ADR-161「機構の選定指針」実証実験3・4の利用側デモ（恒久化しない）。
//!
//! `ChokePoint` は、これまでの実証実験（`crates/xtask-spike`のsynスキャン、
//! `lints/actuation_call_guard_spike`のdylint試作）で実際に確認した実データを使う。

use macro_spike::{choke_points, MarkdownRow};

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
}
