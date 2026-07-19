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
    // (path 接尾辞, コード断片) — spawn_local 内で正当に with_app を呼ぶ既知の例外。
    const ALLOW: &[(&str, &str)] = &[(
        "output/probe_io.rs",
        "crate::with_app(|runtime|", // line 309 の spawn_local(async move) 内
    )];
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
    const ALLOW: &[(&str, &str)] = &[
        ("output/mod.rs", "const VK_A: VkCode = VkCode(0x41);"), // send_unicode_cold_warmup_keys
    ];
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
//   crates/awase-windows/src/runtime/transport.rs
//     fn immcross_suppresses_kanji_down_and_up_regardless_of_shadow_toggled()
//   — AppImeProfile::Standard (=ImmCross) で KeyDown/KeyUp × shadow_toggled 全組合せに対し
//     PhysicalKeyDisposition::plan() が常に Suppress を返すことを検証している (08b8661)。
//   同ファイルの imm32_unavailable_* / non_kanji_event_always_allowed も併せて
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
