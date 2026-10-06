#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! `docs/layer-boundaries.md` のレイヤー境界ルールのうち、これまで検出手段が
//! 手動 grep のみだった 9 件 (A-2 / B-1 / B-2 / C-4 / C-5 / C-6 / D-1 / D-2 / E-1) を
//! ソースファイルのテキスト走査で自動化した回帰テスト。
//!
//! `architecture_guard.rs` と同じ「壊れたら教えてくれる」第二の防衛線であり、
//! stable Rust + std のみで動く (Windows ターゲット非依存。lib の本番コードは
//! `#[cfg(windows)]` でゲートされるが、このテストはファイルを *テキスト* として
//! 読むだけなのでどのホストでも実行できる)。
//!
//! 各テストは対応する `docs/layer-boundaries.md` の「検出」grep を Rust に翻訳したもの。
//! doc が定める 3 分類 (Violation / Transitional / Comment-only) のうち Comment-only を
//! 誤検知しないよう、走査前にコメント行・`#[cfg(test)]` ブロック・テスト専用ファイルを除外する。
//!
//! **ルールを弱めないこと**: 許可リスト (`ALLOW_*`) を安易に広げると防衛線が無力化する。
//! 新しい正当な例外を足すときは、なぜ doc の禁則に当たらないのかを一言添えること。

use std::fs;
use std::path::{Path, PathBuf};

// ───────────────────────── 共通ヘルパ ─────────────────────────

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `dir` 以下の `.rs` を再帰収集する。ファイル名に `test` を含むもの
/// (`test_support.rs` / `tests.rs` / `proptest_tests.rs` 等、`#[cfg(test)] mod` で
/// 宣言されるテスト専用ファイル) は本番コードではないので除外する。
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        panic!("failed to read dir {}", dir.display());
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.contains("test") {
                out.push(p);
            }
        }
    }
}

/// `#[cfg(test)]` が付いた item (mod / impl / fn 等) の本体を丸ごと覆う真偽マスク。
/// ブレースの深さを数えて item の閉じ `}` までを test 領域として印付ける。
fn test_block_mask(lines: &[&str]) -> Vec<bool> {
    let n = lines.len();
    let mut mask = vec![false; n];
    let mut i = 0;
    while i < n {
        if lines[i].trim_start().starts_with("#[cfg(test)]") {
            let mut depth: i32 = 0;
            let mut started = false;
            let mut k = i;
            while k < n {
                mask[k] = true;
                for ch in lines[k].chars() {
                    match ch {
                        '{' => {
                            depth += 1;
                            started = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if started && depth <= 0 {
                    break;
                }
                if !started && lines[k].contains(';') {
                    // `#[cfg(test)] mod tests;` のようなブロックを持たない item
                    break;
                }
                k += 1;
            }
            i = k + 1;
        } else {
            i += 1;
        }
    }
    mask
}

/// 行からコメント部分を落とした「コード部分」を返す。
/// 行コメント (`//` / `///` / `//!`) とブロックコメント継続行 (`*` 始まり) は空にする。
/// 行末コメントは最初の `//` で切り落とす。
fn code_part(line: &str) -> String {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
        return String::new();
    }
    match line.find("//") {
        Some(i) => line[..i].to_string(),
        None => line.to_string(),
    }
}

/// test ブロック外・コメント外の (1 始まり行番号, コード部分) を返す。
fn code_lines(content: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let mask = test_block_mask(&lines);
    lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !mask[*i])
        .map(|(i, l)| (i + 1, code_part(l)))
        .filter(|(_, c)| !c.trim().is_empty())
        .collect()
}

fn rel(path: &Path) -> String {
    // ALLOW リストはフォワードスラッシュ固定で書かれているため、Windows の
    // `\` 区切り表示に引きずられないよう常に `/` へ正規化する。
    path.strip_prefix(manifest())
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

/// `dirs` 以下の本番コードから `pred` に該当する行を「path:line: code」形式で集める。
fn scan<F: Fn(&str) -> bool>(dirs: &[PathBuf], pred: F) -> Vec<String> {
    let mut files = Vec::new();
    for d in dirs {
        if d.is_dir() {
            collect_rs(d, &mut files);
        } else if d.is_file() {
            files.push(d.clone());
        }
    }
    files.sort();
    let mut hits = Vec::new();
    for f in &files {
        let content = fs::read_to_string(f).unwrap_or_default();
        for (line, code) in code_lines(&content) {
            if pred(&code) {
                hits.push(format!("{}:{line}: {}", rel(f), code.trim()));
            }
        }
    }
    hits
}

/// `needle` の直後が識別子文字 (`[A-Za-z0-9_]`) でない箇所があるか
/// (grep の `\b` 相当)。`SendMessage\b` を `SendMessageTimeoutW` と区別するのに使う。
fn contains_word_boundary(code: &str, needle: &str) -> bool {
    let bytes = code.as_bytes();
    let mut start = 0;
    while let Some(pos) = code[start..].find(needle) {
        let abs = start + pos;
        let after = abs + needle.len();
        let boundary = bytes
            .get(after)
            .is_none_or(|&b| !(b.is_ascii_alphanumeric() || b == b'_'));
        if boundary {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

fn assert_empty(rule: &str, hits: &[String], why: &str) {
    assert!(
        hits.is_empty(),
        "layer-boundaries.md {rule} 違反を検出しました。\n{why}\n\n該当箇所:\n  {}",
        hits.join("\n  ")
    );
}

// ───────────────────────── カテゴリ A ─────────────────────────

/// layer-boundaries.md A-2: Engine は事前分類のみ参照 (vk_code は等値比較のみ)。
/// Why: ADR-019 事前分類アーキテクチャ。Engine が vk hex を分類し始めると
/// プラットフォーム独立性 (macOS/Linux 対応) が壊れる。
///
/// doc の grep は `vk_code\.0|VK_[A-Z]+` と粗いが、その「期待: 等値比較とフィールド参照のみ」
/// = 禁則は (1) hex 比較 (2) 範囲 match (3) `is_*` 分類メソッド の 3 つ。ここではその
/// 具体的禁則パターンだけを検出する (named 定数との等値比較や Debug 整形は許容)。
#[test]
fn a2_engine_no_vk_hex_classification() {
    let engine = manifest().join("../../src/engine");
    let hits = scan(&[engine], |code| {
        // (1) vk_code.0 への hex 比較
        (code.contains("vk_code.0")
            && code.contains("0x")
            && (code.contains("== 0x") || code.contains("==0x")))
            // (3) vk_code.is_xxx() 分類メソッド
            || code.contains("vk_code.is_")
            // (2) hex の範囲 arm (0x..=0x) — Engine に現れれば vk 範囲分岐の疑い
            || code.contains("..=0x")
            || code.contains("..= 0x")
    });
    assert_empty(
        "A-2",
        &hits,
        "Engine (src/engine/) では vk_code の hex 比較・範囲 match・is_* 分類メソッドを\
         書かないこと。分類はプラットフォーム層が行い、Engine は KeyClassification 等の\
         事前分類フィールドを読むだけにする。",
    );
}

// ───────────────────────── カテゴリ B ─────────────────────────

/// layer-boundaries.md B-1: `crate::APP` / `with_app` は限定モジュールのみ。
/// Why: ADR-004 AppState orchestrator。observer/focus/output/ime/state が state に
/// こっそり触れる経路を塞ぎ、読み書きを Runtime に集約する。
///
/// 禁則対象は observer/ focus/ output/ ime.rs state/ の 5 領域。唯一の例外は
/// spawn_local closure 内 (async path での再入回避に必須)。現状その例外は
/// `output/probe_io.rs` の 1 箇所のみ (line 309 の `spawn_local(async move {...})` 内)。
#[test]
fn b1_with_app_confined_to_orchestrator_modules() {
    // (path 接尾辞, コード断片) — spawn_local 内で正当に with_app を呼ぶ既知の例外
    // (layer-boundaries.md B-1: 「spawn_local closure 内 (async path で再エントリ
    // 回避のため必須)」は許容モジュール)。
    const ALLOW: &[(&str, &str)] = &[
        (
            "output/probe_io.rs",
            "crate::with_app(|runtime|", // line 309 の spawn_local(async move) 内
        ),
        (
            "output/conv_actuation.rs",
            "crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())",
            // actuate_conv_mode の spawn_local(async move) 内、
            // set_ime_conv_for_target の verify_still_current クロージャ。
            // ime_mode_focus_gen は Runtime/Output 非依存な ime.rs から
            // 読めないため with_app 経由が必須（ADR-086 INV-14）。
        ),
    ];
    let src = manifest().join("src");
    let dirs = [
        src.join("observer"),
        src.join("focus"),
        src.join("output"),
        src.join("state"),
    ];
    let mut hits = scan(&dirs, |code| {
        code.contains("with_app(")
            || code.contains("with_app_ref(")
            || code.contains("with_app_or_repost")
            || code.contains("crate::APP")
            || code.contains("APP.with(")
    });
    // ime.rs (単一ファイル) も 5 領域の一部。
    let ime = manifest().join("src/ime.rs");
    if ime.exists() {
        let content = fs::read_to_string(&ime).unwrap_or_default();
        for (line, code) in code_lines(&content) {
            if code.contains("with_app(") || code.contains("crate::APP") {
                hits.push(format!("src/ime.rs:{line}: {}", code.trim()));
            }
        }
    }
    hits.retain(|h| {
        !ALLOW
            .iter()
            .any(|(p, needle)| h.contains(p) && h.contains(needle))
    });
    assert_empty(
        "B-1",
        &hits,
        "observer/ focus/ output/ ime.rs state/ 内で with_app / crate::APP を直接呼ばないこと。\
         Runtime メソッド経由で間接アクセスするか、どうしても必要なら spawn_local closure に\
         出して ALLOW に登録すること。",
    );
}

/// layer-boundaries.md B-2: `output/` は named API のみ (tsf_obs() 直接呼出禁止)。
/// Why: ADR-030。観測の意図を型 (gji_last_io_ms() 等の named API) に表現する。
#[test]
fn b2_output_uses_named_tsf_observation_api() {
    let hits = scan(&[manifest().join("src/output")], |code| {
        code.contains("tsf_obs()")
    });
    assert_empty(
        "B-2",
        &hits,
        "output/ から TSF observation atomic に触れるときは tsf::observer の named API\
         (gji_last_io_ms() / namechange_baseline() 等) を使い、tsf_obs() を直接呼ばないこと。",
    );
}

// ───────────────────────── カテゴリ C ─────────────────────────

/// layer-boundaries.md C-4: App 固有分岐は AppImePolicy / classifier のみ。
/// Why: ADR-032 設計原則 4。reducer (ime_model.rs::reduce) に app 分岐を漏らさない。
///
/// doc の grep は crate 全体を走査し classifier 群を allowlist で除くが、その真の禁則は
/// 「reducer 内に AppKind:: / class_name 分岐を書かない」。よって reducer を持つ
/// `state/ime_model.rs` に絞って app 分岐がゼロであることを検査する (classifier 側は正当)。
#[test]
fn c4_reducer_has_no_app_specific_branches() {
    let hits = scan(&[manifest().join("src/state/ime_model.rs")], |code| {
        code.contains("AppKind::")
            || code.contains("class_name ==")
            || code.contains("class_name.contains")
            || code.contains("app_kind ==")
    });
    assert_empty(
        "C-4",
        &hits,
        "reducer (state/ime_model.rs) 内で AppKind:: / class_name 分岐を書かないこと。\
         app 固有判断は state/app_ime_policy.rs か focus/classifier.rs に置く。",
    );
}

/// layer-boundaries.md C-5: 旧 boolean guard 残骸ゼロ。
/// Why: ADR-032 設計原則 5。ctrl_bypass_hold 等の sideband guard 積み増しが複雑度の
/// 温床になった履歴。新 API `is_focus_transition_pending()` (InputBarrier ベース) は別物。
///
/// doc の期待は「撤去済みコメントのみ or 完全ゼロ」なのでコメントは許容し、本番コード側で
/// 旧 guard 名の *識別子* 使用がゼロであることを検査する。新 API との誤検知を避けるため
/// `is_focus_transition_pending` は除外する。
#[test]
fn c5_no_legacy_boolean_guard_remnants() {
    let mut files = Vec::new();
    collect_rs(&manifest().join("src"), &mut files);
    files.sort();
    let mut hits = Vec::new();
    for f in &files {
        let content = fs::read_to_string(f).unwrap_or_default();
        for (line, code) in code_lines(&content) {
            // 新 API `is_focus_transition_pending` を除いてから旧 field 名を探す。
            let stripped = code.replace("is_focus_transition_pending", "");
            let hit = stripped.contains("ctrl_bypass_hold")
                || stripped.contains("focus_transition_pending")
                || stripped.contains("shadow_toggle_suppressed")
                || stripped.contains("ImeRecoveryState");
            if hit {
                hits.push(format!("{}:{line}: {}", rel(f), code.trim()));
            }
        }
    }
    assert_empty(
        "C-5",
        &hits,
        "旧 boolean guard (ctrl_bypass_hold / focus_transition_pending / \
         shadow_toggle_suppressed / ImeRecoveryState) を本番コードで参照しないこと。\
         新規 edge case は InputBarrier / ForceGuardSet で表現する。",
    );
}

/// layer-boundaries.md C-6: reduce() 呼出は 1 箇所 (全 event に seq 付与を強制)。
/// Why: ADR-032 設計原則 6。全 ImeEvent を event_log 経由 (reduce_with_envelope) に
/// 通し、壁時計非依存・リプレイ可能性を確保する。
///
/// 本番コードでの `.reduce(` on model は `state/platform_state.rs` の 1 箇所のみ
/// (`reduce_with_envelope` 内)。ime_model.rs の直接 reduce 呼出は reducer 自体の
/// ユニットテスト (`#[cfg(test)]`) で、test ブロック除外により対象外になる。
#[test]
fn c6_single_reduce_call_site() {
    let hits = scan(&[manifest().join("src")], |code| {
        code.contains("model.reduce(")
    });
    assert_eq!(
        hits.len(),
        1,
        "layer-boundaries.md C-6: 本番コードでの model.reduce() 呼出は 1 箇所\
         (platform_state.rs::reduce_with_envelope) のみのはずが {} 箇所ありました。\n\
         全 ImeEvent は event_log.record() 経由で seq を付与すること。\n該当箇所:\n  {}",
        hits.len(),
        hits.join("\n  ")
    );
    assert!(
        hits[0].contains("platform_state.rs"),
        "C-6: 唯一の reduce 呼出は platform_state.rs のはずが {} でした。",
        hits[0]
    );
}

// ADR-170 決定1(reduce() の大きい分岐の private ヘルパー抽出)が
// 「ヘルパーは reduce() 本体からのみ呼ばれる」ことを固定する count guard は
// `tests/architecture_guard.rs::reduce_helpers_are_called_only_from_reduce_body`
// にある(本体スコープの二重固定に `extract_fn_body` を使うため、それが既に
// あるファイル側に置いた。opus-adversarial-consult round2 R2-2/R2-3)。

// ───────────────────────── カテゴリ D ─────────────────────────

/// layer-boundaries.md D-1: magic hex を vk.rs 外で書かない。
/// Why: feedback_vk_encapsulation。VK 定数の意図を helper / 定数名で表現する。
///
/// doc の grep は全 hex を拾い「VK 以外 (HRESULT/タイミング定数等) のみ残る」ことを期待
/// する粗いものだが、その禁則の具体形は「vk.rs 外での VkCode(0x..) リテラル」。よって
/// `VkCode(0x..)` の本番コード出現を検査する (construction / 無名比較の両方を捕捉)。
///
/// 既知の例外 1 件: cold-start warmup の犠牲キー 'A' (`VkCode(0x41)`)。vk.rs に named 定数が
/// ない letter key のため暫定的に許容 (本来は vk.rs へ移すのが望ましい既存の借り)。
#[test]
fn d1_no_vk_magic_hex_outside_vk_rs() {
    const ALLOW: &[(&str, &str)] = &[];
    let mut files = Vec::new();
    collect_rs(&manifest().join("src"), &mut files);
    files.retain(|f| f.file_name().and_then(|n| n.to_str()) != Some("vk.rs"));
    files.sort();
    let mut hits = Vec::new();
    for f in &files {
        let content = fs::read_to_string(f).unwrap_or_default();
        for (line, code) in code_lines(&content) {
            if code.contains("VkCode(0x") {
                hits.push(format!("{}:{line}: {}", rel(f), code.trim()));
            }
        }
    }
    hits.retain(|h| {
        !ALLOW
            .iter()
            .any(|(p, needle)| h.contains(p) && h.contains(needle))
    });
    assert_empty(
        "D-1",
        &hits,
        "vk.rs 外で VkCode(0x..) リテラルを書かないこと。分類は vk.rs の helper、\
         log は UpperHex impl を使う。letter key を犠牲キーに使う場合は vk.rs に named 定数を\
         足すか、理由を添えて ALLOW に登録すること。",
    );
}

// ───────────────────────── カテゴリ D-2 (既存テストでカバー) ─────────────────────────
//
// layer-boundaries.md D-2: ImmCross アプリには物理 IME キーを見せない
// (VK_KANJI 等を KeyDown/KeyUp 両方 Consume する)。
//
// これは grep で表現しづらい *挙動* ルールであり、既存のユニットテストで十分カバー済み:
//   crates/awase-windows/src/state/physical_disposition.rs (`mod tests`。旧 runtime/transport.rs の
//   plan_tests、ADR-229 T1 で移動。Linux でも走る)
//     fn immcross_suppresses_kanji_down_and_up_regardless_of_shadow_toggled()
//   — AppImeProfile::Standard (=ImmCross) で KeyDown/KeyUp × shadow_toggled 全組合せに対し
//     PhysicalKeyDisposition::plan_core() が常に Suppress を返すことを検証している (08b8661)。
//   同モジュールの input_relay_*・injected_*・non_kanji_event_always_allowed・plan_matrix_covers_all_branches_without_panicking も併せて
//   PhysicalKeyDisposition の全プロファイル挙動を固定している。
// よって D-2 はこのファイルに新規テストを追加せず、既存テストでカバー済みと記録する。

// ───────────────────────── カテゴリ E ─────────────────────────

/// layer-boundaries.md E-1: SendMessageTimeoutW は spawn_local 経由 (imm.rs/ime.rs 内のみ)。
/// Why: project_in_with_app_removal。同期 SendMessage がメッセージポンプを回すと hook が
/// 再入し crate::with_app の再入ガードに引っかかる。低レベルラッパは imm.rs/ime.rs に隔離。
///
/// doc の grep semantics (`SendMessageTimeoutW|SendMessage\b`) を忠実に再現する。
/// vk_send.rs の log 文字列 "SendMessageTimeout" (末尾 W 無し) は境界規則で除外される。
#[test]
fn e1_send_message_confined_to_low_level_wrappers() {
    let mut files = Vec::new();
    collect_rs(&manifest().join("src"), &mut files);
    files.retain(|f| {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        name != "imm.rs" && name != "ime.rs"
    });
    files.sort();
    let mut hits = Vec::new();
    for f in &files {
        let content = fs::read_to_string(f).unwrap_or_default();
        for (line, code) in code_lines(&content) {
            if code.contains("SendMessageTimeoutW") || contains_word_boundary(&code, "SendMessage")
            {
                hits.push(format!("{}:{line}: {}", rel(f), code.trim()));
            }
        }
    }
    assert_empty(
        "E-1",
        &hits,
        "SendMessageTimeoutW / 同期 SendMessage は imm.rs (send_ime_control) または ime.rs の\
         async wrapper 内に隔離し、with_app 内で直接呼ばず spawn_local 経由にすること。",
    );
}

// ───────────────────── Tier-2「pure core」ガード (FCIS P0) ─────────────────────
//
// `state/` のうち「Win32・壁時計・グローバル状態・FS を一切持たない」ungated ファイルを
// `CORE_MODULES` として固定し、本番コード（`#[cfg(test)]` の item・コメント・文字列リテラルを除く）が
// 次の 4 規則に違反しないことを確かめる。許可リストは持たない（違反するファイルは載せない）。
//
// 1. 壁時計の直接読み取り（`Instant::now()` 等）。時刻は引数で受ける。
// 2. `static`（不変も含む）と `thread_local!`。
// 3. ファイル内の `#[cfg(windows)]` 項目（`cfg`/`cfg_attr` の中の `windows` の語）。
//    例外は 3 つだけ: `mod` 宣言の直前の `#[cfg(windows)]`、`#[cfg(any(windows, test))]`、
//    lint を抑えるだけの `#[cfg_attr(not(windows), allow(...))]`（動作を切り替えない）。
// 4. FS / 環境変数の読み取り。`env!`/`option_env!`/`include_str!` はコンパイル時展開なので対象外。

/// Tier-2 の対象（`state/<名前>.rs`）。`state/mod.rs` 自体は含めない（`#[cfg(windows)]` の再公開を持つ）。
const CORE_MODULES: &[&str] = &[
    "actuation_chain",
    "actuation_decision_record",
    "alt_impersonation",
    "app_ime_policy",
    "app_suppression",
    "belief",
    "conv_after_open",
    "conv_classify",
    "conv_mode",
    "drift_correction",
    "eisu_recovery",
    "event_origin",
    "evidence",
    "explicit_press",
    "external_change_watch",
    "focus_probe_plan",
    "focus_resync_policy",
    "force_guard",
    "generation",
    "gji_direct_mechanism",
    "half_width_alnum",
    "hook_state",
    "hook_watchdog",
    "ime_actuation",
    "ime_actuation_decision",
    "ime_kind",
    "imm_evidence",
    "injection_mode",
    "input_barrier",
    "intent_store",
    "key_effect_table",
    "key_sequence_policy",
    "keymap_initial_hypothesis",
    "keymap_latch",
    "layout_language",
    "mode_key_pass",
    "observation_store",
    "open_warrant",
    "physical_disposition",
    "post_bypass",
    "press_ledger",
    "scoped_latch",
    "state_dependent_key_warning",
    "transition",
    "win_key_guard",
];

/// ungated だが現状 Tier-2 の規則に違反するファイルと、その理由。直したら `CORE_MODULES` へ移す
/// （`core_modules_violation_list_is_not_stale` が、違反が消えたのに残っているものを失敗させる）。
const NOT_CORE_MODULES: &[(&str, &str)] = &[
    (
        "hub_clock",
        "時計の実装そのもの。Instant::now() を持つ（恒久的に Tier-2 の外）",
    ),
    (
        "ime_event",
        "#[cfg(windows)] impl HwndId / From<HWND>（殻へ出す候補）",
    ),
    (
        "ime_event_log",
        "Instant::now()（record_at を使う側へ寄せれば解消）",
    ),
    (
        "ime_model",
        "effective_open() などの Instant::now()（effective_open_at を呼ぶ側へ）",
    ),
    (
        "ime_profile_driver",
        "不変の static 3 つ（const の &'static dyn へ置き換えられる見込み）",
    ),
    (
        "key_effect_predictor",
        "#[cfg(windows)] の get_gji/get_native（FS/レジストリ。殻へ）",
    ),
    (
        "key_effect_runtime",
        "#[cfg(windows)] と fs::metadata（学習済み表の読み込み。殻へ）",
    ),
    (
        "probe_admission",
        "可変の static カウンタと #[cfg(windows)] の関数（カウンタは殻へ）",
    ),
];

/// 文字列リテラルの中身を落とす（`"..."` → `""`）。ログ文言に `std::fs` 等が出ても誤検出しない。
/// 生文字列・複数行文字列は扱わない。
fn strip_string_literals(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' && chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'\'') {
            out.push_str("' '");
            i += 3;
        } else if chars[i] == '"' {
            out.push_str("\"\"");
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// `needle` の直前が識別子文字でない箇所があるか（`fs::` を `ifs::` と区別する）。
fn contains_word_start(code: &str, needle: &str) -> bool {
    let bytes = code.as_bytes();
    let mut start = 0;
    while let Some(pos) = code[start..].find(needle) {
        let abs = start + pos;
        if abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_') {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

/// 行頭の `pub` / `pub(crate)` などの可視性を落とす。
fn strip_visibility(code: &str) -> &str {
    let t = code.trim();
    if let Some(rest) = t.strip_prefix("pub(") {
        rest.split_once(')').map_or("", |(_, r)| r.trim_start())
    } else if let Some(rest) = t.strip_prefix("pub ") {
        rest.trim_start()
    } else {
        t
    }
}

fn is_static_item(code: &str) -> bool {
    code.contains("thread_local!") || strip_visibility(code).starts_with("static ")
}

fn is_mod_decl(code: &str) -> bool {
    strip_visibility(code).starts_with("mod ")
}

/// 規則 3 の対象か: `cfg`/`cfg_attr` 属性で `windows` の語を含み、かつ例外の 3 形でないもの。
/// `next` は次のコード行（`mod` 宣言の直前の `#[cfg(windows)]` を許すため）。
fn is_forbidden_cfg_windows(attr: &str, next: Option<&str>) -> bool {
    let t = attr.trim();
    if !(t.starts_with("#[cfg(") || t.starts_with("#[cfg_attr(") || t.starts_with("#![cfg")) {
        return false;
    }
    if !contains_word_start(t, "windows") {
        return false;
    }
    let norm: String = t.split_whitespace().collect();
    if norm == "#[cfg(any(windows,test))]" {
        return false;
    }
    if norm.starts_with("#[cfg_attr(not(windows),allow(") && norm.ends_with("))]") {
        return false;
    }
    if norm == "#[cfg(windows)]" && next.is_some_and(is_mod_decl) {
        return false;
    }
    true
}

/// ファイル内容から Tier-2 の違反を (行番号, 規則名, コード) で返す。
fn core_violations(content: &str) -> Vec<(usize, &'static str, String)> {
    let raw = code_lines(content);
    let mut out = Vec::new();
    for (idx, (line, code)) in raw.iter().enumerate() {
        let s = strip_string_literals(code);
        let t = code.trim().to_string();
        if [
            "Instant::now(",
            "SystemTime::now(",
            "quanta::",
            "MonotonicClock",
        ]
        .iter()
        .any(|p| s.contains(p))
        {
            out.push((*line, "wall-clock", t.clone()));
        }
        if is_static_item(&s) {
            out.push((*line, "static", t.clone()));
        }
        // cfg は `target_os = "windows"` を見るため、文字列を落とす前のコードで判定する。
        if is_forbidden_cfg_windows(code, raw.get(idx + 1).map(|(_, c)| c.as_str())) {
            out.push((*line, "cfg-windows", t.clone()));
        }
        if [
            "std::fs",
            "std::env",
            ".exists()",
            ".metadata(",
            "read_to_string(",
            "read_dir(",
        ]
        .iter()
        .any(|p| s.contains(p))
            || ["fs::", "env::", "File::open"]
                .iter()
                .any(|p| contains_word_start(&s, p))
        {
            out.push((*line, "fs-env", t));
        }
    }
    out
}

fn state_dir() -> PathBuf {
    manifest().join("src/state")
}

fn state_file(name: &str) -> PathBuf {
    state_dir().join(format!("{name}.rs"))
}

/// `state/mod.rs` の `mod X;` のうち、直前の属性に `#[cfg(windows)]` が無い（ungated な）ものの名前。
fn ungated_state_modules() -> Vec<String> {
    let content = fs::read_to_string(state_dir().join("mod.rs")).expect("state/mod.rs");
    let lines: Vec<&str> = content.lines().collect();
    let mut names = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let Some(rest) = strip_visibility(l).strip_prefix("mod ") else {
            continue;
        };
        let Some(name) = rest.strip_suffix(';') else {
            continue;
        };
        let mut gated = false;
        let mut j = i;
        while j > 0 {
            j -= 1;
            let p = lines[j].trim();
            if p.starts_with("#[") {
                gated |= p.starts_with("#[cfg(windows)]");
            } else if !p.starts_with("//") {
                break;
            }
        }
        if !gated {
            names.push(name.trim().to_string());
        }
    }
    names
}

/// FCIS P0: `CORE_MODULES` の全ファイルの本番コードが Tier-2 の 4 規則に違反しない。
#[test]
fn core_modules_have_no_tier2_violations() {
    let mut hits = Vec::new();
    for name in CORE_MODULES {
        let path = state_file(name);
        let content = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("CORE_MODULES の {name} が実在しません: {}", path.display())
        });
        for (line, rule, code) in core_violations(&content) {
            hits.push(format!("{}:{line}: [{rule}] {code}", rel(&path)));
        }
    }
    assert_empty(
        "Tier-2 (FCIS P0)",
        &hits,
        "pure core（CORE_MODULES）は壁時計・static・thread_local・#[cfg(windows)] 項目・FS/環境変数を持たない。\
         時刻は引数で受け、Win32/FS に触る部分は殻（runtime/ など）へ出すこと。\
         直せないなら、そのファイルを CORE_MODULES から外して NOT_CORE_MODULES に理由つきで移す。",
    );
}

/// 新しい ungated な `state/` のファイルは、`CORE_MODULES` か `NOT_CORE_MODULES` のどちらかに分類させる。
#[test]
fn core_modules_classify_every_ungated_state_module() {
    let ungated = ungated_state_modules();
    let mut unclassified = Vec::new();
    for name in &ungated {
        let in_core = CORE_MODULES.contains(&name.as_str());
        let in_not = NOT_CORE_MODULES.iter().any(|(n, _)| n == name);
        assert!(
            !(in_core && in_not),
            "{name} が CORE_MODULES と NOT_CORE_MODULES の両方にあります"
        );
        if !in_core && !in_not {
            unclassified.push(name.clone());
        }
    }
    for n in CORE_MODULES {
        assert!(
            ungated.iter().any(|m| m == n),
            "CORE_MODULES の {n} は state/mod.rs の ungated な mod ではありません（gated になった/削除された）"
        );
    }
    assert!(
        unclassified.is_empty(),
        "新しい ungated な state/ のファイルを CORE_MODULES（違反 0）か NOT_CORE_MODULES（理由つき）に分類してください: {unclassified:?}"
    );
}

/// `NOT_CORE_MODULES` の各ファイルには、今も実際に違反がある（直したら CORE_MODULES へ移す）。
#[test]
fn core_modules_violation_list_is_not_stale() {
    let mut stale = Vec::new();
    for (name, _) in NOT_CORE_MODULES {
        let content = fs::read_to_string(state_file(name))
            .unwrap_or_else(|_| panic!("NOT_CORE_MODULES の {name} が実在しません"));
        if core_violations(&content).is_empty() {
            stale.push(*name);
        }
    }
    assert!(
        stale.is_empty(),
        "違反が無くなったファイルは NOT_CORE_MODULES から CORE_MODULES へ移してください: {stale:?}"
    );
}

#[cfg(test)]
mod core_guard_helper_tests {
    use super::*;

    fn rules(src: &str) -> Vec<&'static str> {
        core_violations(src)
            .into_iter()
            .map(|(_, r, _)| r)
            .collect()
    }

    #[test]
    fn detects_wall_clock_and_ignores_comments_and_strings() {
        assert_eq!(rules("let t = Instant::now();\n"), ["wall-clock"]);
        assert_eq!(
            rules("let t = std::time::SystemTime::now();\n"),
            ["wall-clock"]
        );
        assert_eq!(rules("let c = quanta::Clock::new();\n"), ["wall-clock"]);
        assert!(rules("let a = 1; // Instant::now()\n").is_empty());
        assert!(rules("log!(\"std::fs Instant::now()\");\n").is_empty());
        assert!(rules("#[cfg(test)]\nmod t { fn f() { Instant::now(); } }\n").is_empty());
    }

    #[test]
    fn detects_static_items_but_not_static_lifetimes() {
        assert_eq!(rules("static FOO: u32 = 1;\n"), ["static"]);
        assert_eq!(rules("pub(crate) static mut X: u8 = 0;\n"), ["static"]);
        assert_eq!(rules("thread_local! { static A: u8 = 1; }\n"), ["static"]);
        assert!(rules("fn f(x: &'static str) -> &'static str { x }\n").is_empty());
        assert!(rules("const NAMES: &[&'static str] = &[];\n").is_empty());
    }

    #[test]
    fn detects_cfg_windows_variants_with_exceptions() {
        assert_eq!(rules("#[cfg(windows)]\nfn f() {}\n"), ["cfg-windows"]);
        assert_eq!(
            rules("#[cfg(all(windows, unix))]\nfn f() {}\n"),
            ["cfg-windows"]
        );
        assert_eq!(
            rules("#[cfg(target_os = \"windows\")]\nfn f() {}\n"),
            ["cfg-windows"]
        );
        assert_eq!(
            rules("#[cfg_attr(windows, derive(Debug))]\nstruct S;\n"),
            ["cfg-windows"]
        );
        assert!(rules("#[cfg(windows)]\nmod shell;\n").is_empty());
        assert!(rules("#[cfg(windows)]\npub(crate) mod shell;\n").is_empty());
        assert!(rules("#[cfg(any(windows, test))]\nfn f() {}\n").is_empty());
        assert!(rules("#[cfg_attr(not(windows), allow(dead_code))]\nfn f() {}\n").is_empty());
        assert!(rules("#[cfg(unix)]\nfn f() {}\n").is_empty());
    }

    #[test]
    fn detects_fs_and_env_but_allows_compile_time_macros() {
        assert_eq!(rules("let s = std::fs::read(p);\n"), ["fs-env"]);
        assert_eq!(rules("use std::fs;\n"), ["fs-env"]);
        assert_eq!(rules("let m = fs::metadata(p);\n"), ["fs-env"]);
        assert_eq!(rules("let v = std::env::var(\"A\");\n"), ["fs-env"]);
        assert_eq!(rules("let f = File::open(p);\n"), ["fs-env"]);
        assert_eq!(rules("if p.exists() {}\n"), ["fs-env"]);
        assert!(rules("const V: &str = env!(\"CARGO_PKG_VERSION\");\n").is_empty());
        assert!(rules("const T: &str = include_str!(\"x.txt\");\n").is_empty());
        assert!(rules("let x = ifs::foo();\n").is_empty());
    }
}
