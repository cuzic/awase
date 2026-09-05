//! アーキテクチャ境界の grep ベース回帰テスト。
//!
//! `.claude/rules/ime-belief-architecture.md` が定める
//! 「Observe → Pure(classify_*) → Apply(dispatch_event/reduce())」の3層分離を
//! 破る典型パターンをソースファイルの文字列走査で検知する。
//!
//! コンパイラや通常のユニットテストでは検出できない「型としては正しいが
//! 意味的に配線を間違えている」パターン（2026-07-05: cache-miss ヒューリスティックが
//! `UserImeSetIntent{source: IntentSource::Recovery}` でユーザー意図を偽装し、
//! confidence ガードを完全にバイパスして IME belief を直接破壊していたバグ）を、
//! 安価な第二の防衛線として stable Rust だけで検知する。
//!
//! この事故を受けて `IntentSource` は `UserIntentSource` に改名され
//! `Recovery` / `HwndCache` は列挙値として削除された（型で構築不能にする、最強の防衛線）。
//! 代わりに `PanicReset` / `HwndCacheRestored` という専用イベントが追加された。
//! このテストはその「専用イベントが専用の呼び出し元だけから発行され続けているか」を
//! 監視する第二の防衛線。第一の防衛線は dylint lint
//! (`lints/ime_event_guard`, `cargo dylint --lib ime_event_guard -p awase-windows` で実行)。
//!
//! この形式のテストは「壊れたら教えてくれる」ためのものであり、将来的に
//! 正当な理由で許可数が増える場合はこのファイルの定数を更新すること。

use std::fs;
use std::path::Path;

fn read_crate_file(rel_path: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let raw = fs::read_to_string(Path::new(manifest_dir).join(rel_path))
        .unwrap_or_else(|e| panic!("failed to read {rel_path}: {e}"));
    // Windows ランナーは git 既定の core.autocrlf=true でチェックアウト時に .rs
    // ファイルを CRLF 化する（`.gitattributes` の eol=lf 指定は `tests/golden/**`
    // のみが対象で、通常のソースファイルには効かない）。このファイル内の各種
    // ガードは `\n` を埋め込んだリテラル（例: `production_code_only` の
    // `"#[cfg(test)]\nmod tests"`）で境界検出しているため、CRLF のままだと
    // マッチに失敗し「本番コード」と「テストコード」の切り分けが機能しなくなる
    // （2026-08-04 実機CI: `user_intent_source_construction_is_limited_to_typed_writers`
    // が Windows ランナーでのみ count=5 相当で fail した）。読み込み時点で正規化する。
    raw.replace("\r\n", "\n")
}

/// `content` から `needle`（関数呼び出しの `fn_name(` 形）の**実呼び出し**箇所数を
/// 数える。行コメント（`//`/`///`/`//!`、trim 後に先頭一致）と、`fn `/`async fn `
/// 直後に続く関数定義そのものの行を除外する。
///
/// 素朴な `content.matches(needle).count()` だと、doc コメント中の
/// `` `set_ime_romaji_mode_with_target_async(None)` `` のような例示（実際に
/// `conv_classify.rs` に存在する）や、関数定義自身のシグネチャ行
/// （`pub async fn set_ime_romaji_mode_with_target_async(` in `ime.rs`）まで
/// 「呼び出し」として誤カウントしてしまう。
fn count_real_calls(content: &str, needle: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") // `//` / `///` / `//!` すべて除外
        })
        .filter(|line| line.contains(needle))
        .filter(|line| {
            let fn_name = needle.trim_end_matches('(');
            !line.contains(&format!("fn {fn_name}("))
        })
        .count()
}

/// `src/` 以下の全 `.rs` ファイルを再帰的に列挙し、crate ルートからの相対パス
/// （例: `"src/runtime/executor.rs"`）を返す。
///
/// 固定ファイルリストに対する grep だけでは「新しいファイルに呼び出しが追加された」
/// パターン（BUG-59 追補が `platform.rs` という当時どのリストにも無かったファイルに
/// 直接呼び出しを追加した実例）を検知できない。全ファイル走査が必須。
///
/// 走査自体は既存の `walk_rs_files`（元々3テストにそれぞれローカル関数として
/// 重複定義されていたもの、本ヘルパー新設時にトップレベルへ集約）を再利用する。
///
/// 実体は `list_rs_files_under("src")`（BUG-110/ADR-132 Phase 2 で追加、
/// 兄弟クレート横断走査にも使う汎用版）と完全に等価なので、そちらへ委譲する
/// （敵対的コードレビュー指摘: 重複実装の整理）。
fn list_src_files() -> Vec<String> {
    list_rs_files_under("src")
}

/// `dir` 以下の `.rs` ファイルを再帰的に `out` へ集める。
///
/// 3つのテスト（`user_ime_on_paths_are_paired_with_eisu_reset` /
/// `focus_probe_observation_is_limited_to_real_probe_path` /
/// `apply_ime_open_with_belief_call_sites_are_accounted_for`）がそれぞれ
/// ローカル関数として同一実装を持っていたため、トップレベルへ集約した。
fn walk_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir({}): {e}", dir.display())) {
        let entry = entry.unwrap_or_else(|e| panic!("read_dir entry: {e}"));
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `#[cfg(test)] mod tests {` より前の「本番コード」部分だけを取り出す。
/// テストコード内での使用（意図的な stale-intent シミュレーション等）は
/// このチェックの対象外とする。
///
/// # 改行コードに依存してはいけない（Windows CI で実際に壊れた）
///
/// 以前は `"#[cfg(test)]\nmod tests"` という**改行込みの固定リテラル**を
/// `find` していた。GitHub の windows ランナーは git 既定の
/// `core.autocrlf=true` で checkout するため `src/*.rs` は CRLF になり、
/// この needle は 1 件もマッチしない。`map_or(content, ..)` のフォールバックが
/// **ファイル全体を「本番コード」として返す**ため、
/// `production_code_only` を使うガード群が Windows で丸ごと誤判定していた。
/// `.gitattributes` の `eol=lf` 固定は `tests/golden/**` にしか掛かっていない。
///
/// 実害の顕在化: `any_observation_replay_door_is_not_used_in_production` が
/// `state/ime_model.rs` の `#[cfg(test)] mod tests` 内にある
/// `restored_from_journal(` 13 件を本番使用と誤検出して落ちた
/// （PR #59 の windows-build。それまでは同ジョブが手前の clippy ステップで
/// 落ちており、テストステップまで到達していなかったため露出しなかった）。
fn production_code_only(content: &str) -> &str {
    const MARKER: &str = "#[cfg(test)]";
    let mut from = 0;
    while let Some(rel) = content[from..].find(MARKER) {
        let idx = from + rel;
        // `#[cfg(test)]` と `mod tests` の間の空白/改行（LF でも CRLF でも）を跨ぐ。
        if content[idx + MARKER.len()..]
            .trim_start()
            .starts_with("mod tests")
        {
            return &content[..idx];
        }
        from = idx + MARKER.len();
    }
    content
}

/// `build_input_context` 自身が `left_thumb_down`/`right_thumb_down` を
/// リテラル `None` でハードコードしていないことを固定する。
///
/// ADR-097 決定0の欠落そのものの形（`runtime/mod.rs` の構造体リテラルが
/// `left_thumb_down: None, right_thumb_down: None,` と固定していた）を検出する。
/// インデント量に依存しないよう、フィールド名 + `: None,` の隣接だけを見る
/// （2026-08-20 の独立レビューで、旧テストがインデント12スペース決め打ちの
/// 部分文字列一致だったため、この最も再現させたくない退行そのものを
/// 素通ししていたと判明。修正）。
#[test]
fn build_input_context_does_not_hardcode_thumb_state() {
    let content = read_crate_file("src/runtime/mod.rs");
    assert!(
        !content.contains("left_thumb_down: None,"),
        "build_input_context must not hardcode left_thumb_down: None"
    );
    assert!(
        !content.contains("right_thumb_down: None,"),
        "build_input_context must not hardcode right_thumb_down: None"
    );
}

/// `build_input_context(...)` の呼び出し元が、引数としてリテラル `None, None`
/// （親指押下状態を引き継がない）を渡していないことを固定する。
///
/// インデント・改行幅に依存しないよう、空白を全て除去してから部分文字列一致を
/// 見る（2026-08-20 の独立レビューで、旧テストが「`build_input_context(\n`」＋
/// 「`None,\n            None,`」という改行・12スペースインデント決め打ちの
/// AND 一致だったため、`message_handlers.rs` のような別インデント幅の呼び出しや、
/// そもそも同一ファイル内に両方の文字列が別々の理由で存在するだけの偽陽性回避に
/// 弱く、実際に検出力が乏しいと判明。修正）。
#[test]
fn build_input_context_callers_do_not_drop_thumb_down_state() {
    for rel_path in [
        "src/runtime/key_pipeline.rs",
        "src/runtime/message_handlers.rs",
        "src/runtime/mod.rs",
    ] {
        let content = read_crate_file(rel_path);
        let squashed: String = content.split_whitespace().collect();
        // rustfmt は7引数の呼び出しを複数行に折り返すため末尾カンマが付く
        // （`,None,None,)`）が、1行に収まる将来の書き方も考慮して両方見る。
        assert!(
            !squashed.contains("build_input_context(")
                || (!squashed.contains(",None,None,)") && !squashed.contains(",None,None)")),
            "{rel_path} must not pass literal None, None to build_input_context"
        );
    }
}

fn non_comment_lines(content: &str) -> String {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_post_message_none_calls(content: &str) -> usize {
    let mut count = 0;
    let mut rest = content;
    while let Some(idx) = rest.find("PostMessageW") {
        rest = &rest[idx + "PostMessageW".len()..];
        let after_ws = rest.trim_start();
        let Some(after_paren) = after_ws.strip_prefix('(') else {
            continue;
        };
        let first_arg = after_paren.trim_start();
        if first_arg.starts_with("None")
            && first_arg["None".len()..]
                .chars()
                .next()
                .is_some_and(|c| c == ',' || c.is_whitespace())
        {
            count += 1;
        }
        rest = after_paren;
    }
    count
}

/// `production_code_only` が CRLF チェックアウトでも `#[cfg(test)] mod tests` を
/// 切り落とすことの回帰テスト（上の doc 参照）。
#[test]
fn production_code_only_strips_test_module_with_crlf() {
    let lf = "fn prod() { needle(); }\n#[cfg(test)]\nmod tests {\n    fn t() { needle(); }\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(production_code_only(lf).matches("needle(").count(), 1, "LF");
    assert_eq!(
        production_code_only(&crlf).matches("needle(").count(),
        1,
        "CRLF: windows ランナーの core.autocrlf=true でも本番コードだけを数えること"
    );
    // `#[cfg(test)]` が付いた別要素（`mod tests` ではない）では切らない。
    let other =
        "#[cfg(test)]\nfn helper() { needle(); }\n#[cfg(test)]\r\nmod tests {\n needle();\n}\n";
    assert_eq!(production_code_only(other).matches("needle(").count(), 1);
}

#[test]
fn output_and_tsf_production_code_do_not_reference_journal_directly() {
    let offenders: Vec<String> = list_src_files()
        .into_iter()
        .filter(|path| path.starts_with("src/output/") || path.starts_with("src/tsf/"))
        .filter_map(|path| {
            let content = read_crate_file(&path);
            let production = non_comment_lines(production_code_only(&content));
            let direct_journal_refs = production.matches("crate::journal").count()
                - production.matches("crate::journal_policy").count();
            (direct_journal_refs > 0).then_some(format!("{path}: {direct_journal_refs}"))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "output/ and tsf/ production code must pass literal-detect facts upward as data, \
         leaving JournalEntry conversion to platform.rs: {offenders:?}"
    );
}

/// `content` 内で `fn_signature_needle`（例: `"fn some_handler"`）が最初に
/// マッチした関数の本体（波括弧の対応を数えて閉じ括弧まで）を切り出す。
///
/// 特定の関数の中だけで「あるパターンが出現しないこと」を固定したい場合に使う
/// （ファイル全体には他の正当な用途で同じパターンが出現しうるため）。
fn extract_fn_body<'a>(content: &'a str, fn_signature_needle: &str) -> &'a str {
    let start = content
        .find(fn_signature_needle)
        .unwrap_or_else(|| panic!("function matching {fn_signature_needle:?} not found"));
    let open_brace = content[start..].find('{').map_or_else(
        || panic!("no opening brace found for {fn_signature_needle:?}"),
        |i| start + i,
    );
    let mut depth = 0i32;
    for (i, ch) in content[open_brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &content[open_brace..=open_brace + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces while extracting body of {fn_signature_needle:?}");
}

/// `content[open_brace..]` の `open_brace` に対応する閉じ括弧の絶対バイト位置を
/// 返す（波括弧の対応を数える）。**文字列リテラル（`"..."`、`\"` エスケープ考慮）
/// の中身は無視する** — `log::debug!("... {{ ... }}")` のような Rust の
/// format 文字列エスケープ（`{{`/`}}` はリテラルの `{`/`}` 1文字を表し、
/// コード構造上の波括弧ではない）が深さカウントを狂わせるのを防ぐため
/// （opus レビュー指摘、変異テストで実際に誤検知を確認済み、2026-08-08）。
/// 文字列リテラルの開始判定は簡易的（生文字列 `r"..."`/`r#"..."#` 等は未対応）
/// だが、このファイルが対象とする通常の Rust コードには十分。
fn find_balanced_close(content: &str, open_brace: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in content[open_brace..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_brace + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `extract_fn_body` の複数箇所版。`content` 内で `needle` が出現するたびに、
/// その直後の最初の `{` から波括弧の対応を数えて閉じ括弧までを切り出し、
/// 全出現分をベクタで返す（例: 同一ファイル内の複数の `spawn_local(async move {`
/// ブロックをそれぞれ独立に検査したい場合に使う）。**ネストした出現も再帰的に
/// 検出する**（例: 外側の `spawn_local` ブロックの中でさらに `spawn_local` が
/// 呼ばれている場合、両方を別々の要素として返す。外側の要素の中身には内側の
/// ブロックがそのまま含まれる点に注意 — 外側だけを解析したい場合は
/// [`mask_nested_needle_blocks`] で内側を除去してから使うこと）。
fn extract_all_balanced_blocks<'a>(content: &'a str, needle: &str) -> Vec<&'a str> {
    let mut blocks = Vec::new();
    collect_balanced_blocks(content, needle, &mut blocks);
    blocks
}

fn collect_balanced_blocks<'a>(content: &'a str, needle: &str, out: &mut Vec<&'a str>) {
    let mut search_from = 0usize;
    while let Some(rel_start) = content[search_from..].find(needle) {
        let start = search_from + rel_start;
        let Some(rel_open) = content[start..].find('{') else {
            break;
        };
        let open_brace = start + rel_open;
        let Some(end) = find_balanced_close(content, open_brace) else {
            panic!("unbalanced braces while extracting block for {needle:?} at byte {start}");
        };
        let block = &content[open_brace..=end];
        out.push(block);
        // ネストした出現を再帰的に探す（block 自身の '{'/'}' は含めず内側だけ）。
        if end > open_brace {
            collect_balanced_blocks(&content[open_brace + 1..end], needle, out);
        }
        search_from = end + 1;
    }
}

/// `block` 内にネストした `needle`（例: `"spawn_local(async"`）付きのブロックを
/// プレースホルダに置き換えて除去した文字列を返す。
///
/// 外側のブロック自身の「最初の await はどれか」を判定する際、独立して
/// スケジュールされるネストした `spawn_local` タスクの中身（別の実行タイミングで
/// 走る）を混入させないために使う。
fn mask_nested_needle_blocks(block: &str, needle: &str) -> String {
    let mut result = String::with_capacity(block.len());
    let mut search_from = 0usize;
    loop {
        let Some(rel_start) = block[search_from..].find(needle) else {
            result.push_str(&block[search_from..]);
            break;
        };
        let start = search_from + rel_start;
        let Some(rel_open) = block[start..].find('{') else {
            result.push_str(&block[search_from..]);
            break;
        };
        let open_brace = start + rel_open;
        let Some(end) = find_balanced_close(block, open_brace) else {
            result.push_str(&block[search_from..]);
            break;
        };
        result.push_str(&block[search_from..start]);
        result.push_str("/* nested spawn_local masked */");
        search_from = end + 1;
    }
    result
}

/// `ImeEvent::PanicReset` は `apply_panic_reset` のみが dispatch する。
///
/// `IntentSource::Recovery` は廃止され `UserIntentSource` に存在しない（型で強制済み）。
/// `ImeEvent::PanicReset` は `desired_open` を安全デフォルト値に戻すが `last_intent` を
/// 設定しない専用イベントであり、`apply_panic_reset` 以外から発行してはならない。
///
/// 観測が乏しい/存在しない状況でのヒューリスティックな推測は
/// `ObserverReported` + `ObservationConfidence::Low` を使うこと
/// (`reset_to_off_for_tsf_native_cache_miss` を参照)。
#[test]
fn panic_reset_event_is_limited_to_apply_panic_reset() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("ImeEvent::PanicReset {").count();
    assert_eq!(
        count, 1,
        "{path} 内で `ImeEvent::PanicReset` の本番コードでの使用箇所数が \
         想定(1 = apply_panic_reset のみ)と異なります(実際: {count})。\n\
         `ImeEvent::PanicReset` は全面リセット専用であり、ヒューリスティックな推測には \
         `ObserverReported` + `ObservationConfidence::Low` を使ってください。"
    );
}

/// `ImeEvent::HwndCacheRestored` は `apply_hwnd_cache_restore` のみが dispatch する。
///
/// `PanicReset` と対になる、キャッシュ復元専用の非ユーザー意図イベント。
/// `desired_open` を回復するが `last_intent` を設定しないため、ユーザーの能動的操作と
/// 区別され、後続の実観測が `effective_open()` を上書きできる。
#[test]
fn hwnd_cache_restored_event_is_limited_to_apply_hwnd_cache_restore() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("ImeEvent::HwndCacheRestored {").count();
    assert_eq!(
        count, 1,
        "{path} 内で `ImeEvent::HwndCacheRestored` の本番コードでの使用箇所数が \
         想定(1 = apply_hwnd_cache_restore のみ)と異なります(実際: {count})。"
    );
}

/// `ImeEvent::InputModeObserved` は必ず `confidence` を伴う（コンパイラが強制する）が、
/// 実際には外部 API/probe を呼んでいないのに「観測した」ことにして dispatch する
/// 偽装パターン（2026-07-05: SetOpen 直後の内部訂正が `source: ImmGetOpenStatus` を
/// 偽装していたバグ）を防ぐため、`InputModeObserved` の構築箇所数を固定する。
///
/// awase 自身の能動的な訂正（内部ロジックによる belief 書き換え）は
/// `InputModeApplied` を使うこと。
#[test]
fn input_mode_observed_construction_sites_are_accounted_for() {
    let known_sites: &[(&str, usize)] = &[
        ("src/state/platform_state.rs", 1), // apply_ime_update (ObserverPoll, Medium)
        // idle-conv-check / ImmCrossProbe。focus-conv-check は ALT+TAB 直後の conv 値で
        // belief を書き換えるバグの温床だったため撤去済み（フォーカス変更直後の読み取りは
        // ユーザー意図の signal ではない。conv_mode/prev_conversion_mode の追跡のみ残す）。
        ("src/runtime/key_pipeline.rs", 2),
        // GjiIoInference: Blacklist で GJI I/O 確認中の ObservedEisu 矛盾訂正
        // （フォーカス後の GJI プロセス I/O という真正の外部観測。Medium confidence、
        // ObservedEisu→AssumedRomaji の一方通行のみ）。
        ("src/runtime/ime_refresh.rs", 1),
    ];
    for (path, expected) in known_sites {
        let content = read_crate_file(path);
        let count = content.matches("ImeEvent::InputModeObserved {").count();
        assert_eq!(
            count, *expected,
            "{path} 内の `ImeEvent::InputModeObserved` 構築箇所数が想定({expected})と \
             異なります(実際: {count})。\n\
             新規箇所を追加した場合は、実際に外部 API/probe を呼んでいるか \
             (=偽装していないか)を確認した上で、このテストの期待値を更新してください。\n\
             awase 自身の能動的な訂正には `InputModeApplied` を使ってください。"
        );
    }
}

/// `ObservationSource::HeuristicDefault` は観測データが存在しない状況での安全デフォルト推測に限定される。
///
/// 現在の designated 使用箇所（すべて Low confidence で `desired_open` を書き換えない）:
/// - `reset_stale_ime_on_for_imm_broken`: Imm32Unavailable 入場時の安全デフォルト ON
///   (`reset_to_off_for_tsf_native_cache_miss` は 37883d0 で TsfNative SSOT 化に伴い削除済み)
///
/// Low confidence にすることで後続の実観測（Medium/High）で上書き可能にしている
/// （confidence は `Observed<HeuristicDefault>` 側で Low 固定、ADR-089 §2.2）。
///
/// **ADR-089 §7 はこのガードの削除を挙げているが、§9-2 の但し書き
/// （witness `ImePolicyProfile` は「起点を限定する」効果はあるが「起動時に
/// 限定する」効果は無い）に従い、needle を witness 構築子へ付け替えて残す。**
/// 型が守るのは「HeuristicDefault を名乗るには profile が要る」までであり、
/// 「起動直後の 1 箇所からしか呼ばない」はテキスト検査でしか守れない。
#[test]
fn heuristic_default_observation_is_limited_to_designated_methods() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("evidence::HeuristicDefault").count();
    assert_eq!(
        count, 1,
        "{path} 内の `evidence::HeuristicDefault` 使用箇所数が想定(1)と異なります(実際: {count})。\n\
         想定: reset_stale_ime_on_for_imm_broken (Imm32Unavailable entry → ON) の1箇所のみ。\n\
         (reset_to_off_for_tsf_native_cache_miss は 37883d0 で TsfNative SSOT 化に伴い削除済み)\n\
         新しい安全デフォルト推測を追加する場合は `UserImeSetIntent` を使わず \
         `Observed::<evidence::HeuristicDefault>::at_startup` を使い、このカウントを \
         更新してください。"
    );
}

/// `ImeEvent::InputModeApplied` は awase 自身の能動的な input_mode 更新に限定される。
///
/// 外部 API を呼んでいないのに `InputModeObserved` で「観測した体」を偽装するのを防ぐ。
/// `result: Applied` 固定のケース（7箇所、strategy/mode/tick_ms のみ違う）は
/// `runtime/mod.rs::Runtime::apply_input_mode_correction` に集約済み
/// （2026-07-27、下記 designated 呼び出し元は同ヘルパー経由）。`result: Skipped` を
/// 構築する経路は `state/ime_model.rs` 内の別経路専用でここには含まれない。
/// 現在の designated 使用箇所（各 strategy と対応）:
/// - `platform_state.rs::apply_panic_reset`        → `InputModeApplyStrategy::PanicReset`
///   （直接構築、`apply_input_mode_correction` 未経由）
/// - `platform_state.rs::apply_hwnd_cache_restore` → `InputModeApplyStrategy::CacheRestore`
///   （直接構築、`apply_input_mode_correction` 未経由）
/// - `runtime/mod.rs::apply_input_mode_correction` （唯一の構築箇所） 経由の呼び出し元:
///   - `key_pipeline.rs` (post-decision)             → `InputModeApplyStrategy::PostSetOpenEisuReset`
///   - `key_pipeline.rs` (shadow toggle OFF→ON)      → `InputModeApplyStrategy::UserImeOnEisuReset`
///   - `key_pipeline.rs` (shadow toggle no-op/TurnOn)→ `InputModeApplyStrategy::UserTurnOnEisuReset`
///   - `key_pipeline.rs` (左Shift単独タップ、トグルON)→ `InputModeApplyStrategy::UserHalfWidthAlnumToggle`
///     (`ObservedEisu` へ、`kp_shift_conv_guard_key_up`)
///   - `key_pipeline.rs` (半角英数トグルOFF共通ヘルパー)→ `InputModeApplyStrategy::UserHalfWidthAlnumToggle`
///     (`AssumedRomaji` へ、`kp_restore_kana_from_half_width`。B節のトグルOFF・E節の3競合
///     経路・F節のフォーカス変更安全策から共通で呼ばれる、2026-07-11)
///   - `ime_refresh.rs`                              → `InputModeApplyStrategy::ImmBrokenCorrection` (FocusChanged)
///   - `runtime/mod.rs`                              → `InputModeApplyStrategy::ImmBrokenCorrection` (Blacklist force-ON)
///
/// 新しい能動的訂正を追加する場合は `InputModeApplyStrategy` に専用 variant を追加し、
/// `apply_input_mode_correction` 経由で dispatch した上でこのカウントを更新すること。
/// 外部観測には必ず `InputModeObserved` を使うこと。
#[test]
fn input_mode_applied_construction_sites_are_accounted_for() {
    let known_sites: &[(&str, usize)] = &[
        ("src/state/platform_state.rs", 2), // PanicReset + CacheRestore（直接構築、対象外）
        // 7箇所すべて apply_input_mode_correction 経由になったため key_pipeline.rs / ime_refresh.rs はゼロ。
        ("src/runtime/key_pipeline.rs", 0),
        ("src/runtime/ime_refresh.rs", 0),
        // apply_input_mode_correction 自体の唯一の構築箇所（7箇所すべての共通呼び出し先）。
        ("src/runtime/mod.rs", 1),
    ];
    for (path, expected) in known_sites {
        let content = read_crate_file(path);
        let count = content.matches("ImeEvent::InputModeApplied {").count();
        assert_eq!(
            count, *expected,
            "{path} 内の `ImeEvent::InputModeApplied` 構築箇所数が想定({expected})と \
             異なります(実際: {count})。\n\
             新しい能動的訂正を追加する場合は `InputModeApplyStrategy` に専用 variant を追加し、\n\
             `runtime/mod.rs::Runtime::apply_input_mode_correction` 経由で dispatch した上で\n\
             このテストの期待値を更新してください。\n\
             外部 API 観測には `InputModeObserved` を使ってください（偽装厳禁）。"
        );
    }
}

/// `UserImeSetIntent` の dispatch は3つの typed writer 経由に限定される。
///
/// - `write_sync_key`        → `UserIntentSource::SyncKey`
/// - `write_physical_key`    → `UserIntentSource::PhysicalImeKey`
/// - `write_set_open_request`→ `UserIntentSource::Command`
///
/// 外部コードはこれらのメソッドを介して `UserImeSetIntent` を発行すること。
/// `dispatch_event(ImeEvent::UserImeSetIntent { .. })` を直接呼ぶのは
/// typed writer の実装内に限る。
/// 新しい `UserIntentSource` variant を追加して dispatch する場合は
/// 対応する typed writer メソッドを追加し、このカウントを更新すること。
/// user IME-ON 経路には stale `ObservedEisu` 救済が対で配線されていることを監視する。
///
/// 背景 (2026-07-06 MS Edge で実発生): `ObservedEisu` belief は engine activation を
/// `NotRomajiInput` で塞ぎ、activation 側の救済 (`PostSetOpenEisuReset`) は Decision 経由
/// `SetOpen(true)` 限定のため、救済のない IME-ON 経路が 1 本でもあると
/// Imm32Unavailable アプリ（観測経路なし）で engine が永久に inactive になる
/// 循環デッドロックを作る。経路×救済の対応表は `src/state/eisu_recovery.rs` の
/// module doc が SSOT。
///
/// このテストは typed writer（`write_sync_key` / `write_physical_key` /
/// `write_set_open_request`）の**呼び出し箇所**を src/ 全域で走査して固定する。
/// **新しい user IME-ON 経路（typed writer の新しい呼び出し元）を追加する場合は、
/// `state::eisu_recovery::eisu_reset_on_ime_on` による ObservedEisu 救済を対で配線し、
/// `eisu_recovery.rs` の対応表とこのテストの期待値を更新すること。**
#[test]
fn user_ime_on_paths_are_paired_with_eisu_reset() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    let patterns = [
        "write_sync_key(",
        "write_physical_key(",
        "write_set_open_request(",
    ];
    // (相対パス, 期待マッチ数, 説明)。ここに列挙されないファイルは 0 でなければならない。
    let expected: &[(&str, usize, &str)] = &[
        (
            "state/platform_state.rs",
            9,
            "typed writer 定義 3 + handle_engine_set_open 内部委譲 1 (Decision 経由 \
             SetOpen — 救済: kp_stage_post_decision の PostSetOpenEisuReset) + \
             BUG-51 追補 v3 の IntentStore 回帰テスト内での write_sync_key/\
             write_physical_key 直接呼び出し 5 件（新しい本番 IME-ON 経路ではなく \
             既存 typed writer をテストから呼んでいるだけなので eisu-reset の \
             追加配線は不要）",
        ),
        (
            "runtime/key_pipeline.rs",
            2,
            "kp_stage_shadow_ime_toggle の SyncKey/PhysicalImeKey (救済: 同関数内の \
             UserImeOnEisuReset + no-op 分岐の UserTurnOnEisuReset)",
        ),
    ];

    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(path).unwrap();
        let count: usize = patterns.iter().map(|p| content.matches(p).count()).sum();
        let expected_count = expected
            .iter()
            .find(|(f, _, _)| *f == rel)
            .map_or(0, |(_, n, _)| *n);
        assert_eq!(
            count, expected_count,
            "src/{rel} 内の typed writer (write_sync_key/write_physical_key/\
             write_set_open_request) 呼び出し箇所数が想定({expected_count})と異なります\
             (実際: {count})。\n\
             新しい user IME-ON 経路を追加した場合は、stale ObservedEisu の救済 \
             (state::eisu_recovery::eisu_reset_on_ime_on) を対で配線しないと、\
             Imm32Unavailable アプリで engine が永久 inactive になる循環デッドロックを\
             作ります。src/state/eisu_recovery.rs の経路×救済対応表と、このテストの \
             expected を更新してください。"
        );
    }

    // 救済側の実在確認: 対応表の 2 経路が実際に共通純関数を使っているか
    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    assert!(
        kp.matches("eisu_reset_on_ime_on(").count() >= 2,
        "key_pipeline.rs は PostSetOpenEisuReset / UserImeOnEisuReset の両経路で \
         eisu_recovery::eisu_reset_on_ime_on を使うこと（インライン再実装の禁止）"
    );
    assert!(
        kp.contains("InputModeApplyStrategy::UserImeOnEisuReset"),
        "shadow toggle 経路の救済 (UserImeOnEisuReset) が撤去されています。\
         撤去する場合は ObservedEisu 循環デッドロック (2026-07-06) の再発防止策を\
         代わりに用意してください。"
    );
    assert!(
        kp.contains("eisu_reset_on_turn_on_while_open(")
            && kp.contains("InputModeApplyStrategy::UserTurnOnEisuReset"),
        "shadow toggle no-op 分岐の救済 (UserTurnOnEisuReset) が撤去されています。\
         IME が既に open のまま conv だけ ObservedEisu に固着した場合、TurnOn 系キー \
         (ひらがな/かな 等) を押しても OFF→ON 遷移が起きないため UserImeOnEisuReset は \
         発火しません。撤去する場合は ObservedEisu 循環デッドロック (2026-07-09 MS Edge/\
         MS-IME で実発生) の再発防止策を代わりに用意してください。"
    );
}

/// `write_focus_probe` は実際に FocusProbe（first-key の `read_ime_state_fast`）を
/// 実行した経路のみが呼べる。
///
/// 2026-07-06: TsfGate の bypass 確定処理（`settle_tsf_gate_after_refresh`）が、probe を
/// 一切実行していないのに `write_focus_probe(false)` を毎リフレッシュ注入していた
/// （ce45b82、「非TSFウィンドウには日本語IMEが存在しない」という誤前提）。実観測経路を
/// 持たない Imm32Unavailable（Edge/Chrome）ではこの偽 Low false が `most_recent_trusted()`
/// 経由で belief を支配し、フォーカス約 500ms 後に Engine が必ず OFF になった
/// （docs/known-bugs.md BUG-07）。
///
/// TsfGate の状態確定（`bypass_tsf`/`confirm_tsf`）は injection 層の関心事であり、
/// IME open belief とは独立に行うこと。「この種のウィンドウに IME は無いはず」という
/// 推測を belief に書きたくなったら、それは観測の偽装である（`ObservationSource::FocusProbe`
/// は「実際に read_ime_state_fast を実行した」ことを意味する）。
#[test]
fn focus_probe_observation_is_limited_to_real_probe_path() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    // (相対パス, 期待マッチ数)。ここに列挙されないファイルは 0 でなければならない。
    let expected: &[(&str, usize)] = &[
        // apply_effective_ime — first-key FocusProbe（read_ime_state_fast 実行済み）の
        // 結果適用点。TsfNative/Imm32Unavailable は ADR-106 決定2 により観測不能として
        // 扱われ、この経路では write_focus_probe を呼ばない（shadow 値は guard 解除
        // 判定にのみ使う。代替観測としての記録は laundering として撤去済み、BUG-92）。
        ("runtime/key_pipeline.rs", 1),
    ];

    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(path).unwrap();
        let production = production_code_only(&content);
        let count = production.matches(".write_focus_probe(").count();
        let expected_count = expected
            .iter()
            .find(|(f, _)| *f == rel)
            .map_or(0, |(_, n)| *n);
        assert_eq!(
            count, expected_count,
            "src/{rel} 内の `.write_focus_probe(` 呼び出し箇所数が想定({expected_count})と\
             異なります(実際: {count})。\n\
             write_focus_probe は「実際に FocusProbe を実行した」経路専用です。probe を\
             実行していない場所から false を書くと、実観測経路を持たない Imm32Unavailable\
             （Edge/Chrome）で belief が偽 false に支配され、Engine が必ず OFF になります\
             （ce45b82 → BUG-07 の再発）。ヒューリスティックな推測なら \
             `ObserverReported + ObservationSource::HeuristicDefault + Low` を、\
             エンジンを keys に反応させたくないだけなら FocusKind::NonText 分類を使って\
             ください。"
        );
    }
}

/// `EngineSync::SetOpen` は `RomajiRecovered` 専用。`NativeToggleShadowOff` は
/// `EngineSync::ReportOpenInference` を使うこと。
///
/// 2026-07-08 BUG-19 再発: 当時 `KatakanaShadowOff` という別 variant（2026-08-17
/// ADR-094 で `NativeToggleShadowOff` へ統合）が `SetOpen(true)` 経由で
/// `handle_engine_set_open` → `UserImeSetIntent{Command}` を偽装し、`desired_open`
/// を直接書き換えていた。これによりユーザーが明示的に IME OFF にした直後でも、
/// conv の一発誤読（GJI 候補ポップアップへのフォーカス flicker 等）を理由に engine
/// が勝手に ON へ戻る再発バグを起こした。修正後は `NativeToggleShadowOff` を
/// `ReportOpenInference`（`ObserverReported` として記録するだけ、`desired_open` は
/// 変更しない、`PlatformState::report_conv_open_inference()` が唯一の消費経路）に
/// 分離した。この境界が将来再び崩れないよう、
/// `SetOpen(ConvSyncReason::NativeToggleShadowOff)` という組み合わせが本番コードに
/// 一切出現しないことを固定する。
#[test]
fn native_toggle_shadow_off_never_uses_set_open() {
    let path = "src/state/conv_classify.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let forbidden = "SetOpen(ConvSyncReason::NativeToggleShadowOff)";
    assert!(
        !production.contains(forbidden),
        "{path} に `{forbidden}` が出現しています。\n\
         NativeToggleShadowOff は SetOpen（engine を直接 actuate し、\
         UserImeSetIntent{{Command}} を偽装して desired_open を書き換える）を \
         使ってはならず、必ず ReportOpenInference（ObserverReported として記録する \
         だけ）を使うこと。さもないとユーザーの明示 IME OFF が conv の一発誤読で \
         上書きされる再発バグ（2026-07-08, BUG-19 再発）が戻ります。"
    );
}

/// conv ビット由来の open 推論を**構築**できるのは
/// `report_conv_open_inference()` の 1 箇所だけ（ADR-089 §2.1・§7、INV-40）。
///
/// 5a37333 で「型が subsume した」として一度削除したが、**それは早すぎた**ので
/// 復活させた（needle は `ObservationSource::ConvOpenInference` から
/// `evidence::ConvOpenInference` へ、期待値は 2 → 1 へ更新している）。型が
/// 守るのは「`ConvSyncReason` を持たないコードはこの観測を構築できない」
/// 「confidence の上限は Medium で呼び出し元は選べない」までであり、
/// **`ConvSyncReason` は普通の public enum なので誰でも構築できる**
/// （ADR-089 §9-11 の witness 強度の不均一）。したがって「conv 推論を名乗る
/// 経路が 1 本しかない」ことはテキスト検査でしか守れない。
///
/// なお `check_drift_correction()` の source-aware gate（BUG-19 再発対策）は
/// `ObservationSource::ConvOpenInference` を**読む**だけで観測を作らないため、
/// この needle には掛からない（旧テストの期待値 2 のうち 1 件がそれだった）。
#[test]
fn conv_open_inference_source_is_limited_to_report_and_gate() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("evidence::ConvOpenInference").count();
    assert_eq!(
        count, 1,
        "{path} 内の `evidence::ConvOpenInference` 構築箇所数が想定(1 = \
         report_conv_open_inference の dispatch)と異なります(実際: {count})。\n\
         conv ビット由来の open 推論の dispatch は必ず `report_conv_open_inference()` \
         経由にし、confidence の上限 (Medium) を勝手に上げないでください \
         (`Observed::<evidence::ConvOpenInference>::from_conv` が Medium を固定します)。"
    );
}

/// `IntentStore::record()` を直接呼んでよいのは `record_explicit_intent`
/// （本物のユーザー操作と確定できる3箇所からのみ呼ばれる）の内部だけ
/// （BUG-51 追補 v3）。`dispatch_event` の汎用フックから呼ぶと、conv 由来の
/// 内部同期（`EngineSync::DirectInput` 等が `UserImeSetIntent{Command}` を
/// dispatch する経路）まで「本物のユーザー操作」として永続化してしまう
/// （pre-mortem #1 角度2）。
#[test]
fn intent_store_record_call_sites_are_limited_to_explicit_user_actions() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    // 行コメントを除外する（`count_real_calls`）。素の `matches()` だと、
    // `record_explicit_intent` の doc が「どのガードが何を固定しているか」を
    // 説明するためにこの needle を引用しただけで落ちる（2026-08-13 実際に発生）。
    let count = count_real_calls(production, "self.intent_store.record(");
    assert_eq!(
        count, 1,
        "{path} 内で `self.intent_store.record(` の本番コードでの使用箇所数が \
         想定(1 = record_explicit_intent 内のみ)と異なります(実際: {count})。\n\
         新しい呼び出し元を足す前に、それが conv 由来の内部同期ではなく \
         本物のユーザー操作であることを確認し、record_explicit_intent 経由に \
         してください。"
    );
}

/// `record_explicit_intent()` を呼んでよい箇所を `src/` 全走査で固定する
/// （BUG-51 追補 v3、2026-08-13 に新設）。
///
/// # なぜ上の `intent_store_record_call_sites_are_limited_to_explicit_user_actions`
/// だけでは足りなかったか
///
/// 上のガードが固定しているのは `state/platform_state.rs` 内の
/// `self.intent_store.record(` の出現数（1 = `record_explicit_intent` の中だけ）で
/// あり、**`record_explicit_intent` 自身の呼び出し元の数は誰も固定していなかった**。
/// `record_explicit_intent` の doc は「呼び出してよいのは3箇所のみ
/// （`tests/architecture_guard.rs` で出現数を固定）」と書いていたが、3箇所目の
/// `runtime/key_pipeline.rs` は上のガードの走査対象（`platform_state.rs` 1 ファイル）
/// にすら入っていない。つまり `key_pipeline.rs` に4箇所目の
/// `record_explicit_intent(..)` を足しても、どのテストも落ちなかった。
///
/// 「conv 由来の内部同期を明示ユーザー意図として `IntentStore` に永続化しない」
/// （pre-mortem #1 角度2）という不変条件を守るには、**record の一次窓口
/// （＝上のガード）と、その窓口を叩ける入口の集合（＝本ガード）の両方**を
/// 固定する必要がある。新しい呼び出し元を足すときは、それが `IntentWitness`
/// （注入されていない実キーイベント）か `SetOpenOrigin::ExplicitUserAction` の
/// ように「本物のユーザー操作」であることが型/分岐で確定していることを
/// 確認してから known_sites を更新すること。
#[test]
fn record_explicit_intent_call_sites_are_limited_to_real_user_actions() {
    const NEEDLE: &str = "record_explicit_intent(";
    let known_sites: &[(&str, usize)] = &[
        // write_sync_key / write_physical_key（どちらも `IntentWitness` が
        // 「注入されていない実キーイベント」を型で要求する）。
        ("src/state/platform_state.rs", 2),
        // kp_stage_post_decision の `SetOpenOrigin::ExplicitUserAction` 分岐
        // （`applied == true` のときのみ）。
        ("src/runtime/key_pipeline.rs", 1),
    ];

    let all_files = list_src_files();
    let mut files_with_calls: Vec<(String, usize)> = Vec::new();
    for path in &all_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            files_with_calls.push((path.clone(), count));
        }
    }
    files_with_calls.sort();

    let mut expected: Vec<(String, usize)> = known_sites
        .iter()
        .map(|(p, c)| ((*p).to_string(), *c))
        .collect();
    expected.sort();

    assert_eq!(
        files_with_calls, expected,
        "`{NEEDLE}` を含むファイル集合/出現数が想定と異なります。\n\
         想定: {expected:?}\n実際: {files_with_calls:?}\n\
         IntentStore への記録は「本物のユーザー操作」に限定される（BUG-51 追補 v3、\
         pre-mortem #1 角度2）。conv 由来の内部同期（`EngineSync::DirectInput` 等が \
         `UserImeSetIntent{{Command}}` を dispatch する経路）からは呼ばないこと。"
    );
}

/// `ImeStateHub::effective_open()` が `IntentStore::resolve_effective_open()` を
/// 必ず通ることを固定する（BUG-51 追補 v3 の配線そのもの、2026-08-13 に新設）。
///
/// # なぜテキスト検査でしか守れないか
///
/// 判定本体（`state/intent_store.rs`、ungated）には Linux で走る回帰テスト
/// （`tests/intent_store_effective_open.rs`）があるが、**それを
/// `ImeStateHub::effective_open()` が実際に呼んでいるという配線自体**は
/// `state/platform_state.rs` が `#[cfg(windows)]` であるため Linux では
/// 1 行も実行されない（その中の `mod tests` も同様）。配線を外して
/// `shadow_model.effective_open()` を直接返す実装に戻しても、Linux CI は
/// 全緑のまま——BUG-51 追補の再発（明示 IME OFF が壊れた `ConvOpenInference`
/// 1 件で反転する）を誰も検知できない。
///
/// そこで「呼び出しが本番コードに 1 箇所だけ存在し、それが
/// `fn effective_open` の本体の中にある」ことを機械的に固定する。
#[test]
fn effective_open_is_wired_to_the_intent_store_decision() {
    const NEEDLE: &str = "resolve_effective_open(";
    let known_sites: &[(&str, usize)] = &[("src/state/platform_state.rs", 1)];

    let all_files = list_src_files();
    let mut files_with_calls: Vec<(String, usize)> = Vec::new();
    for path in &all_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            files_with_calls.push((path.clone(), count));
        }
    }
    files_with_calls.sort();

    let mut expected: Vec<(String, usize)> = known_sites
        .iter()
        .map(|(p, c)| ((*p).to_string(), *c))
        .collect();
    expected.sort();

    assert_eq!(
        files_with_calls, expected,
        "`{NEEDLE}` を含むファイル集合/出現数が想定と異なります。\n\
         想定: {expected:?}\n実際: {files_with_calls:?}"
    );

    // 呼び出しが `ImeStateHub::effective_open_at()`（判定本体）にあること。
    let content = read_crate_file("src/state/platform_state.rs");
    let production = production_code_only(&content);
    let bodies = extract_all_balanced_blocks(
        production,
        "fn effective_open_at(&self, now_ms: TickMs) -> bool",
    );
    assert_eq!(
        bodies.len(),
        1,
        "`fn effective_open_at(&self, now_ms: TickMs) -> bool` が {} 箇所あります（想定: 1）",
        bodies.len()
    );
    assert_eq!(
        count_real_calls(bodies[0], NEEDLE),
        1,
        "`ImeStateHub::effective_open_at()` の本体から `{NEEDLE}` が消えています。\n\
         belief（`Engine::compute_state` の `ctx.ime_on`）が IntentStore の \
         明示意図上書きを通らなくなると、BUG-51 追補の再現手順\n\
         （明示 IME OFF → プロセスを跨ぐフォーカス変更 → 壊れた \
         `ConvOpenInference` 1 件）で Engine だけが ON へ戻る退行が復活します。\n\
         判定本体の回帰は tests/intent_store_effective_open.rs にあります。"
    );

    // 引数なしの `effective_open()` は「壁時計を読んで `effective_open_at()` に
    // 委譲するだけ」であること（追補4）。本番の唯一の呼び出し口がこの 2 行を
    // 保つ限り、belief は必ず IntentStore 判定を通り、かつ TTL 判定に使う時刻は
    // record 側（`runtime/key_pipeline.rs` の `hook::current_tick_ms()`）と
    // 同じ時間軸に揃う。ここに合成 tick を持ち込むと、テストだけが通って
    // 実機では上書きが沈黙する 2026-08-13 windows-build 型の欠陥に戻る。
    let wrapper = extract_all_balanced_blocks(production, "fn effective_open(&self) -> bool");
    assert_eq!(
        wrapper.len(),
        1,
        "`fn effective_open(&self) -> bool` が {} 箇所あります（想定: 1）",
        wrapper.len()
    );
    assert_eq!(
        count_real_calls(wrapper[0], "effective_open_at("),
        1,
        "`ImeStateHub::effective_open()` が `effective_open_at()` へ委譲していません。"
    );
    assert_eq!(
        count_real_calls(wrapper[0], "current_tick_ms("),
        1,
        "`ImeStateHub::effective_open()` が `hook::current_tick_ms()`（record 側と \
         同じ時間軸）以外の時刻で IntentStore を評価しようとしています。"
    );
}

/// `UserIntentSource` をリテラルで名乗れるのは `write_set_open_request`
/// （`Command`）の 1 箇所だけ（ADR-089 §2.2・§7、INV-40）。
///
/// `SyncKey` / `PhysicalImeKey` は `IntentWitness::from_sync_key` /
/// `from_physical` が運ぶようになったため、リテラルは残っていない——
/// 「注入されていない実キーイベント」（`&RawKeyEvent`, `injected == false`）が
/// 無ければ意図を名乗れない（BUG-14 の型化）。
///
/// **`Command` は engine 内部判断であり、引数の型で起点を限定できる外部事実が
/// 無いため witness 化できない**（ADR-089 §9-8）。したがってこのガードは
/// 削除せず、期待値 1 で残す。**ゼロにする変更は単独で行わないこと**——
/// BUG-19 の再発条件（間接推測が `Command` を名乗って `desired_open` を
/// 書き換える）に直接関係する。
#[test]
fn user_intent_source_construction_is_limited_to_typed_writers() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("source: UserIntentSource::").count();
    assert_eq!(
        count, 1,
        "{path} 内の `source: UserIntentSource::` リテラル構築箇所数が想定(1)と異なります(実際: {count})。\n\
         想定: write_set_open_request (`Command`) の1箇所のみ。\n\
         `SyncKey` / `PhysicalImeKey` は `IntentWitness` が source を運ぶため、\n\
         リテラルで名乗ってはいけません（ADR-089 §2.2）。\n\
         新しい UserIntentSource variant を追加する場合は、witness に載せられる \n\
         外部事実があるかをまず検討してください（ADR-089 §9-8）。"
    );
}

/// `AnyObservation::restored_from_journal` は journal / fixture 復元専用の口で
/// あり、本番コードから呼んではならない（ADR-089 §2.1）。
///
/// 本番の観測は必ず `Observed<E>` の witness 構築子（`from_probe` /
/// `from_cross_probe` / `from_poll` / `at_startup` / `from_conv`）を通す。
/// この口を本番から使うと、witness を持たないコードが任意の
/// `ObservationSource` と `ObservationConfidence` を名乗れてしまい、
/// §2.2 のデータ witness が丸ごと迂回される。
#[test]
fn any_observation_replay_door_is_not_used_in_production() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(path).unwrap();
        let production = production_code_only(&content);
        let count = production.matches("restored_from_journal(").count();
        // 定義そのもの（`pub const fn restored_from_journal(`）は evidence.rs に 1 件。
        let expected = usize::from(rel == "state/evidence.rs");
        assert_eq!(
            count, expected,
            "src/{rel} が `restored_from_journal(` を本番コードで使っています\
             (実際: {count}, 想定: {expected})。観測は `Observed<E>` の witness \
             構築子を通してください（ADR-089 §2.1・§2.2、INV-40）。"
        );
    }
}

/// 実 IME actuation 入口 6 種（`apply_ime_open_with_belief` / `_with_view` /
/// `_with_applied` / `set_ime_open` / `apply_ime_open`）の
/// **呼び出し箇所数**を、入口ごとに crate 全域で固定する
/// （各関数の定義行 `fn ...(` は数えない）。
///
/// これは「唯一の窓口」への統合テストではなく、**新しい未レビューの呼び出し元が
/// 増えたら気づく**ための count guard である。旧版（`apply_ime_open_with_belief`
/// 単体のみ）は ADR-080 / `docs/known-bugs.md` BUG-43（drift correction ループが
/// raw actuation を observe tick ごとに無限再送した設計欠陥）への対策として作られたが、
/// `apply_ime_open_with_belief(` の**部分文字列一致**でカウントしていたため
/// `runtime/mod.rs` の doc コメント中の同名文字列を1件誤って呼び出しとして数えており
/// （実呼び出しは4件、旧 `EXPECTED_TOTAL=5` の内訳に1件のコメントが混入していた）、
/// かつ `_with_view`/`_with_applied`/`set_ime_open`/`apply_skipping_imm` 経由の入口は
/// 対象外だった（ADR-087 §5 Phase 3 item14、2026-08-10 棚卸しで判明）。
///
/// **2026-08-12（ADR-089 Phase B）**: `apply_skipping_imm` は撤去した。ImmCross が
/// 機構チェーンの要素になったことで、`Failed` 後のフォールスルーは
/// `state/actuation_chain.rs::run_chain_async` が行う（`runtime/open_chain.rs`）。
/// 非同期経路の入口は `run_open_chain_async` に一本化されており、その件数は
/// `open_chain_is_the_only_async_actuation_entry` が固定する。
///
/// 実 actuation 入口 11 経路の全数棚卸し（force-write / observation-based
/// correction / Engine intent の分類、`shadow_on`/`origin` の扱い）は
/// `docs/adr/087-open-belief-actuation-warrant-separation.md` §5 item14 の表を
/// 参照。新しい呼び出し元を追加した場合はこの表を更新し、
/// `ir_apply_drift_correction` と同じ `Actuation` ベースのゲーティングが必要か
/// （ADR-080 / BUG-43 参照）を検討した上で、このカウントを更新すること。
#[test]
fn ime_open_actuation_entry_points_are_accounted_for() {
    // needle は先頭に `.` を付けたメソッド呼び出し形にする。定義行
    // (`fn apply_ime_open_with_belief(` 等) は `.` を伴わないため自動的に除外され、
    // `log::info!("... apply_ime_open({open}) ...")` のような人間可読ログ文字列
    // （`.` を伴わない）も除外される（後者は `apply_ime_open(` の素の部分文字列
    // 一致だと 6 箇所誤検出することを実際に確認した上でこの形にした）。
    //
    // **2026-08-12（ADR-090 §2.A A-1）**: 実 actuation 入口を `ActuationOrder`
    // 経由へ移した（§6 ステップ 5 item 20）。件数の変化は次の 2 つだけで、
    // **入口の数そのものは変わっていない**:
    //
    // - `.set_ime_open(` 2 → **0**。トレイトメソッド（`src/platform.rs` の
    //   トレイト定義）には引数を足せないため、外部 2 件
    //   （`ime_refresh.rs` の focus change 強制 OFF と drift correction の
    //   ImmCross 分岐）を inherent な `set_ime_open_ordered` へ移した。
    //   **トレイトメソッド側はガードとして残す**（ゼロになったことが可視化
    //   される。ADR-090 §2.A 設計案 3）。
    // - `.apply_ime_open_with_applied(` 2 → **1**。呼び出し元ゼロの死んだ
    //   trait オーバーライド `WindowsPlatform::apply_ime_open` を削除したため、
    //   その内部委譲 1 件が消えた（`awase` 側のトレイト既定実装が残る）。
    //
    // **2026-08-21（ADR-098 決定2、BUG-69）**: `ir_post_focus_change_snapshot`
    // の TsfNative force-on ブロック（`apply_ime_open_with_applied(order, None)`
    // の唯一の本番呼び出し元）を撤去した。`apply_ime_open_with_applied` 自体も
    // 呼び出し元ゼロになったためメソッドごと削除し（未使用の force-write API を
    // 残さない方針、`.claude/rules/experiment-logging.md`）、その内部委譲だった
    // `.apply_ime_open_with_belief(` の1件も連鎖して消える。
    // - `.apply_ime_open_with_belief(` 3 → **2**（`apply_ime_open_with_applied`
    //   内部委譲の消滅）。
    // - `.apply_ime_open_with_applied(` 1 → **0**（メソッドごと削除。ガードは
    //   残す——0 でなくなったら死んだ API が復活したことを意味する）。
    const ENTRY_POINTS: [(&str, usize); 6] = [
        // 外部 2（ime_refresh.rs drift correction / key_pipeline.rs idle-conv-check、
        // ADR-087 §5 item14 表 #11/#4）。ADR-098 決定2 で内部委譲元
        // （apply_ime_open_with_applied）が消えたため 3→2。
        //
        // **2026-08-19（BUG-34 横展開 D）**: 表 #7 の mod.rs try_force_on_bootstrap は
        // ここから外れた。同期 ImmCrossProcessStrategy::apply（150ms 宣言
        // タイムアウトの SendMessageTimeoutW をエンジンスレッドで直接ブロックする
        // 経路）を経由しなくなり、executor.rs の ImmCross async path と同じ
        // run_open_chain_async へ委譲するようになったため
        // （`async_imm_cross_actuation_goes_through_the_single_chain_entry` 参照）。
        (".apply_ime_open_with_belief(", 2),
        // 外部 2（executor.rs engine decision / mod.rs force_on_and_correct_romaji、
        // 表 #1/#6）+ apply_ime_open_with_belief 内部からの委譲 1 = 3。
        // （`apply_ime_open_with_belief` からの委譲であって `apply_ime_open_with_applied`
        // からではないため ADR-098 決定2 の影響を受けない。）
        (".apply_ime_open_with_view(", 3),
        // ADR-098 決定2（BUG-69）: 唯一の呼び出し元（ime_refresh.rs の GJI
        // TsfNative 強制 ON ブロック）を撤去し、メソッド自体も削除した。
        (".apply_ime_open_with_applied(", 0),
        // ADR-090 A-1 で `set_ime_open_ordered` へ移したため本番呼び出しゼロ。
        // **ガードは残す**——ここが 0 でなくなったら、warrant を通さない
        // actuation 入口が復活したことを意味する。
        (".set_ime_open(", 0),
        // 外部 2（ime_refresh.rs:534 focus change 強制 OFF / :752 drift correction
        // の ImmCross 分岐）。**旧コメントは `:727` と書いていたが実在しない**
        // ——近いのは `log::warn!` の文字列（`:725`）で、先頭に `.` が無いため
        // そもそも needle に一致しない（ADR-090 §2.A.2(3) 脚注）。
        (".set_ime_open_ordered(", 2),
        // 呼び出し元ゼロ(死んだ入口)。`WindowsPlatform` のオーバーライドは
        // ADR-090 A-1 で削除し、`awase` 側のトレイト既定実装だけが残る。
        (".apply_ime_open(", 0),
    ];

    let files = list_src_files();
    for (needle, expected) in ENTRY_POINTS {
        let mut total = 0usize;
        let mut breakdown: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            let count = count_real_calls(production, needle);
            if count > 0 {
                total += count;
                breakdown.push((path.clone(), count));
            }
        }
        assert_eq!(
            total, expected,
            "`{needle}` の呼び出し箇所数が想定({expected})と異なります(実際: {total})。\
             内訳: {breakdown:?}\n\
             docs/adr/087-open-belief-actuation-warrant-separation.md §5 item14 の\
             実 actuation 入口棚卸し表を更新し、新しい呼び出し元が force-write / \
             observation-based correction のどちらに分類され warrant 必須化の対象と\
             すべきか検討した上でこの期待値を更新してください。"
        );
    }
}

/// `ImeStateHub::record_optimistic`/`record_confirmed`（ADR-098 決定6-a、BUG-69）の
/// crate 全域の呼び出し箇所数を固定する。
///
/// `mirror_applied_open`/`mirror_applied_open_with_ts`（旧 API、`ts==0` センチネルで
/// `Optimistic`/`Confirmed` を選んでいた）は、決定6-a でこの2メソッドに置き換える
/// までの間、**architecture_guard 34本のうち1本にも守られていなかった**
/// （`rg mirror_applied_open crates/awase-windows/tests/` が no match、設計討議で
/// 実測確認済み）。`applied` への書き込みは BUG-16/BUG-20/BUG-69 が繰り返し
/// 踏んできた「belief を actuation の記録として書く」誤用の温床であり
/// （INV-A97-1）、新しい呼び出し元が無審査で増えたら気づけるよう、旧 API と
/// 同じ「呼び出し元ゼロの穴」を新 API で再現しないためのガードとして追加する。
///
/// 期待値の内訳（すべて `docs/adr/098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md`
/// F6 の6サイトに対応。新しいサイトを追加した場合は、それが実 actuation の記録
/// （`record_confirmed`/`record_optimistic`）か belief の書き戻し（INV-A97-1 違反）
/// かを判定した上でこの期待値を更新すること）:
///
/// - `.record_optimistic(` = 1（`ir_apply_drift_correction`、ImmCross 分岐）。
/// - `.record_confirmed(` = 5（`record_ime_apply_result` 内部/`ir_post_focus_change_snapshot`
///   〈決定1-a、TsfNative ではスキップされ非 TsfNative のみ到達〉/
///   `kp_stage_shadow_ime_toggle`/`focus_tracking.rs` hard pre-sync/
///   `process_deferred_keys`〈本番到達不能なデッドコード、決定5参照〉）。
#[test]
fn applied_state_recorders_call_sites_are_accounted_for() {
    const RECORDERS: [(&str, usize); 2] = [(".record_optimistic(", 1), (".record_confirmed(", 5)];

    let files = list_src_files();
    for (needle, expected) in RECORDERS {
        let mut total = 0usize;
        let mut breakdown: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            let count = count_real_calls(production, needle);
            if count > 0 {
                total += count;
                breakdown.push((path.clone(), count));
            }
        }
        assert_eq!(
            total, expected,
            "`{needle}` の呼び出し箇所数が想定({expected})と異なります(実際: {total})。\
             内訳: {breakdown:?}\n\
             ADR-098 決定0 INV-A97-1（`ImeModel.applied` は実際に OS への actuation を\
             試みた経路だけが書いてよい）を確認し、新しい呼び出しがそれに違反しないか\
             （belief を actuation の記録として書いていないか）確認した上でこの期待値を\
             更新してください。既存の5箇所のうち3箇所（`ir_post_focus_change_snapshot`\
             の非TsfNative分岐・`focus_tracking.rs` の hard pre-sync・\
             `process_deferred_keys`〈dead code〉）は actuation を伴わない belief\
             ミラーとして ADR-098 決定5 が明示的に許容した既知の例外です\
             （`state/platform_state.rs` の `record_optimistic` doc 参照）。"
        );
    }
}

/// ADR-108 証拠義務(a-2): `ImeModel.applied` はまだ `pub` のため、reducer 外からの
/// 直接代入をテキスト走査で固定する。`record_confirmed`/`record_optimistic` の既知の
/// 例外と、`ImeModel::reduce` 内の正規書き込み以外が増えた場合は、`applied` を
/// private 化してアクセサへ寄せる本筋の修正を検討すること。
///
/// このガードは暫定的な文字列検査であり、`ImeModel { applied: some_state, .. }` の
/// ような変数・関数呼び出しを使う構造体リテラル、`.applied  =` のような空白違い、
/// `&mut` 経由の別名書き込みは検出できない。通り抜ける書き方が存在するため、
/// 本筋は ADR-108 に書いた `applied` の private 化と専用アクセサ化である。
#[test]
fn applied_direct_assignments_are_accounted_for() {
    const DIRECT_ASSIGNMENTS: [(&str, usize); 2] = [
        ("src/state/ime_model.rs", 5),
        ("src/state/platform_state.rs", 2),
    ];
    const STRUCT_LITERAL_FIELDS: [(&str, usize); 1] = [("src/state/ime_model.rs", 1)];

    for path in list_src_files() {
        let content = read_crate_file(&path);
        let production = non_comment_lines(production_code_only(&content));
        let assignment_count = production.matches(".applied = ").count();
        let literal_count = production.matches("applied: AppliedImeState::").count()
            + production
                .matches("applied: crate::state::AppliedImeState::")
                .count();
        let expected_assignments = DIRECT_ASSIGNMENTS
            .iter()
            .find(|(p, _)| *p == path)
            .map_or(0, |(_, n)| *n);
        let expected_literals = STRUCT_LITERAL_FIELDS
            .iter()
            .find(|(p, _)| *p == path)
            .map_or(0, |(_, n)| *n);

        assert_eq!(
            assignment_count, expected_assignments,
            "{path} の `.applied = ` 直接代入数が想定({expected_assignments})と異なります\
             (実際: {assignment_count})。ADR-108 決定3の `applied` 書き込み点集約を\
             破っていないか確認してください。"
        );
        assert_eq!(
            literal_count, expected_literals,
            "{path} の `applied: AppliedImeState::...` 構造体リテラル数が想定\
             ({expected_literals})と異なります(実際: {literal_count})。`ImeModel` を\
             reducer 外で直接構築していないか確認してください。"
        );
    }
}

/// ADR-098 決定1-c: `apply_force_on_for_imm_broken` の 20ms 無限再試行ループ封鎖
/// （BUG-69）が `force_on_attempt_allowed`/`note_force_on_attempt` を経由し続けている
/// ことを固定する。0 になるとループ封鎖そのものが外れる（実装記録「実装順序・
/// テスト コミット1」の必須回帰テスト）。
#[test]
fn force_on_retry_cooldown_gate_call_sites_are_accounted_for() {
    const GATES: [(&str, usize); 2] = [
        (".force_on_attempt_allowed(", 1),
        (".note_force_on_attempt(", 1),
    ];

    let files = list_src_files();
    for (needle, expected) in GATES {
        let mut total = 0usize;
        let mut breakdown: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            let count = count_real_calls(production, needle);
            if count > 0 {
                total += count;
                breakdown.push((path.clone(), count));
            }
        }
        assert_eq!(
            total, expected,
            "`{needle}` の呼び出し箇所数が想定({expected})と異なります(実際: {total})。\
             内訳: {breakdown:?}\n\
             ADR-098 決定1-c（BUG-69 の 20ms 無限再試行ループ封鎖）が\
             `apply_force_on_for_imm_broken` 内で経由し続けているか確認してください。\
             0 になるとクールダウンが外れ、TsfNative で cold-mark を伴う実効 50Hz の\
             再試行ループが再発します。"
        );
    }
}

/// `handle_wm_focus_kind_update`（UIA 非同期分類結果のハンドラ、BUG-12 対策）が
/// belief/state への書き込みを一切行わないことを固定する。
///
/// この handler は UIA の非同期分類結果を受け取るが、hwnd 粒度とウィンドウ内
/// フォーカス要素追跡の設計が未解決のため、結果を意図的に破棄しログのみに
/// とどめている（`let _ = app;` で書き込み手段自体を放棄、関数内コメント参照）。
/// この no-op はコンパイラで強制されておらず「コメントのみの防御」であるため、
/// 将来この handler に belief 書き込みロジックが足された場合、GjiFsm の
/// `CompositionReset`/`NativeF2Consumed`（BUG-33 追補3・4）と同型の「弱い
/// 非同期シグナルだけで belief を破壊し確定済み文字が消える」バグが構造的に
/// 再発しうる。それを検知するため、この関数の本体に belief/state 書き込みと
/// 思われる呼び出しパターンが一切出現しないことを固定する。
#[test]
fn uia_async_focus_kind_handler_does_not_write_belief() {
    let path = "src/runtime/message_handlers.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "fn handle_wm_focus_kind_update");
    for forbidden in [
        "dispatch_event(",
        "reduce(",
        "ImeEvent::",
        ".shadow_model.",
        "force_guards",
        ".belief.",
        "learn_injection_mode",
        "update_injection_mode",
        "gji_on_",
    ] {
        assert!(
            !body.contains(forbidden),
            "{path} の handle_wm_focus_kind_update 内に belief/state 書き込みと \
             思われるパターン `{forbidden}` が見つかりました。UIA 非同期結果は \
             BUG-12 により意図的に適用しない設計です（hwnd 粒度・ウィンドウ内 \
             フォーカス要素追跡の設計が未解決のため）。意図的に適用するよう \
             変更したのであれば、関数内コメントの課題が解決されたことを確認した \
             上でこのテストの期待値を更新してください。"
        );
    }
}

/// `match act_signature` 部分の開始マーカー。`ir_apply_drift_correction` の中で
/// `FeedbackPolicy` を分岐する `match act_policy { ... }` ブロックの先頭。
const DRIFT_MATCH_MARKER: &str = "match act_policy {";
/// 実送信ブロックの先頭にある `log::warn!` のメッセージ接頭辞。この直前で
/// `match act_policy { ... }`（早期 return 分岐）が終わる。
const DRIFT_SEND_LOG_MARKER: &str = "[drift] correction: observed=";

/// `ir_apply_drift_correction` の `match act_policy { ... }` ブロック（＝ `Blind`/`GaveUp`
/// と `Read`/`Confirmed` の早期 return 分岐）だけを切り出す。
///
/// 開始は `match act_policy {`、終了は実送信ブロックの先頭にある
/// `log::warn!("[drift] correction: observed=...")` の直前。この `log::warn!` より後は
/// ADR-080 不変条件6 のスコープ外（乖離が確定して実際に `set_ime_open` する正規経路であり、
/// そこで `dispatch_event(ImeEvent::DriftDetected {..})` を呼ぶのは正当）。したがって
/// **関数全体ではなく match ブロックだけ**を検査対象にする。行番号ではなくマーカー文字列で
/// 境界を求めるため、周辺のコードが動いても壊れにくい。
fn extract_drift_correction_match_block(content: &str) -> &str {
    let start = content
        .find(DRIFT_MATCH_MARKER)
        .unwrap_or_else(|| panic!("marker {DRIFT_MATCH_MARKER:?} not found in ime_refresh.rs"));
    let send_marker = content.find(DRIFT_SEND_LOG_MARKER).unwrap_or_else(|| {
        panic!("send-path marker {DRIFT_SEND_LOG_MARKER:?} not found in ime_refresh.rs")
    });
    // match ブロック内は `log::debug!` のみ。実送信は `log::warn!` で始まる唯一の箇所。
    let send_log = content[start..send_marker]
        .rfind("log::warn!(")
        .map_or_else(
            || panic!("no `log::warn!(` found between match block and send-path marker"),
            |i| start + i,
        );
    assert!(
        send_log > start,
        "抽出範囲が不正: match ブロック開始 ({start}) より前に送信 log ({send_log}) がある"
    );
    &content[start..send_log]
}

/// ADR-080 不変条件6 の回帰ガード: `Resolution::GaveUp`（Blind の max_attempts 到達）
/// および `Read` の未収束・deadline 超過による早期 return は、いかなる場合も
/// `observations` ストアへの書き込み（`ObserverReported` 等の dispatch）を発生させない。
///
/// これに違反すると BUG-33 と同型の「収束偽装」が再発する。BUG-33 では、ある機構が
/// **自分の belief をそのまま観測ストアに「観測」として書き戻していた**ため、書き戻した
/// 値が構造上つねに一致してしまい、drift 検知が二度と発火しなくなっていた。ここで
/// もし GaveUp/Confirmed の早期 return が `desired` を観測として書き込めば、次 tick 以降の
/// `check_drift_correction` が「観測 == desired」で乖離なしと誤認し、本来まだ実現できて
/// いない目標を「達成済み」と勘違いする（＝同じ失敗モード）。
///
/// 注意: `match act_policy { ... }` ブロックの**後**にある正規の実送信経路は
/// `dispatch_event(ImeEvent::DriftDetected {..})` を正当に呼ぶ。それは不変条件6の
/// スコープ外なので、関数全体ではなく match ブロックのテキストだけを検査する
/// (`extract_drift_correction_match_block` 参照)。仮にその `dispatch_event` を match
/// ブロック内（早期 return より前）へ移動させれば、このテストは fail する。
#[test]
fn drift_correction_giveup_and_confirmed_do_not_write_observations() {
    let path = "src/runtime/ime_refresh.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let match_block = extract_drift_correction_match_block(production);
    // 注意: `observations.record(` に限定する（`.record(` だけだと `UnifiedJournal::record`
    // ‐ ADR-082 Phase 0.5 で追加された `self.platform_state.ime.journal.record(..)`（監査用
    // ジャーナルへの書き込み、`observations` とは無関係）にも誤って一致してしまう。
    // `journal` は書き込み専用の監査ログで、drift 検知の収束判定（`check_drift_correction`/
    // `most_recent_trusted`）が読み取ることは一切無い。`record`/`absorb`/`stamper` は
    // 監査ログへの書き込み・採番用、`dump_to_file`/`dump_to_file_capped` は診断出力用であり、
    // いずれも `observations` とは無関係なので、不変条件6のスコープ外。
    for forbidden in [
        "dispatch_event(",
        "ObserverReported",
        "observations.record(",
        "write_focus_probe",
        "write_observer_poll",
        "write_imm_cross_probe",
    ] {
        assert!(
            !match_block.contains(forbidden),
            "{path} の ir_apply_drift_correction 内 `match act_policy {{ ... }}` \
             （Blind/GaveUp・Read/Confirmed の早期 return 分岐）に、観測ストアへの \
             書き込みと思われるパターン `{forbidden}` が見つかりました。\n\
             ADR-080 不変条件6 により、GaveUp（および Read の deadline 超過/未収束）は \
             `observations` への書き込み（`ObserverReported` 等の dispatch）を \
             一切発生させてはなりません。違反すると docs/known-bugs.md BUG-33 と同型の \
             収束偽装（自分の belief を観測として書き戻し、drift 検知が二度と発火しない）\
             が再発します。実送信は match ブロックの後（`log::warn!(\"[drift] correction: \
             observed=...\")` 以降）でのみ行い、そこでの `DriftDetected` dispatch は \
             不変条件6 のスコープ外です。"
        );
    }
}

#[test]
fn bug_report_journal_truncation_does_not_slice_from_the_front() {
    let content = read_crate_file("src/bug_report.rs");
    let production = production_code_only(&content);
    for forbidden in ["[..max_bytes]", "[..end]", "input[.."] {
        assert!(
            !production.contains(forbidden),
            "bug_report.rs の journal 添付切り詰めで `{forbidden}` が見つかりました。\
             添付ログは古い先頭ではなく、症状直前の末尾 entry を JSON 配列として妥当に残す必要があります。"
        );
    }
}

/// GJI/MS-IME の IME ON/OFF/フォールバックが送信する VK コードを、実装ソースの
/// テキスト走査で固定する。
///
/// `docs/experiments.md` エントリ01: 「IME OFF に何のキーを送るか」で5日間に6回、
/// 採用と撤回が反転した（`534051a` → `098c663` → `adb856c` → `b271aee` → … →
/// `489cdf1`）。最終結論は「GJI/MS-IME いずれも IME ON/OFF は冪等 VK_IME_ON (0x16) /
/// VK_IME_OFF (0x1A) を送る `post_ime_on_direct()`/`post_ime_off_direct()` 経由
/// （VK_KANJI トグルには戻さない）」（根拠: `489cdf1`, `48a667a`、ON 側は
/// `2026-08-06` に BUG-50 根治として同じキーへ統一）。
///
/// `tests/ime_key_sequence_golden.rs` の `KEY_DOC` はこの結論をコメントとして固定して
/// いるが、その本文はハードコードされた定数文字列同士の突き合わせ（自己参照）であり、
/// 各 `post_*` 関数の実装が別の VK コードに戻ってもゴールデンは通ってしまう。この
/// テストは `src/ime.rs` の実関数本体を直接検査し、送信 VK コードの回帰を検知する。
/// Win32 呼び出しを伴わないテキスト走査のみのため Linux 上でもそのまま実行できる。
#[test]
fn ime_open_close_functions_send_expected_vk_codes() {
    let path = "src/ime.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);

    // 冪等 IME ON: VK_IME_ON。VK_KANJI（非冪等トグル）・VK_DBE_HIRAGANA（MS-IME専用）が
    // 混入すると shadow desync や環境依存の不具合を再導入する。
    let on_direct = extract_fn_body(production, "pub unsafe fn post_ime_on_direct(");
    assert!(
        on_direct.contains("VK_IME_ON"),
        "{path} の post_ime_on_direct が VK_IME_ON を送っていません。"
    );
    for forbidden in ["VK_KANJI", "VK_DBE_HIRAGANA", "VK_DBE_ALPHANUMERIC"] {
        assert!(
            !on_direct.contains(forbidden),
            "{path} の post_ime_on_direct に {forbidden} が混入しています。冪等 IME ON は \
             VK_IME_ON 単独であるべきです。"
        );
    }

    // 冪等 IME OFF: VK_IME_OFF。docs/experiments.md エントリ01「IME OFF に何のキーを
    // 送るか」で5日間に6回反転した最終結論（534051a→098c663→adb856c→b271aee→…→
    // 489cdf1、根拠48a667a）。
    let off_direct = extract_fn_body(production, "pub unsafe fn post_ime_off_direct(");
    assert!(
        off_direct.contains("VK_IME_OFF"),
        "{path} の post_ime_off_direct が VK_IME_OFF を送っていません。docs/experiments.md \
         エントリ01 の6回反転（534051a→098c663→adb856c→b271aee→…→489cdf1）の最終結論から \
         の回帰です。"
    );
    for forbidden in ["VK_KANJI", "VK_DBE_ALPHANUMERIC", "VK_DBE_HIRAGANA"] {
        assert!(
            !off_direct.contains(forbidden),
            "{path} の post_ime_off_direct に {forbidden} が混入しています。docs/experiments.md \
             エントリ01 で撤回済みの選択肢への回帰の可能性があります。"
        );
    }

    // GJI/MS-IME 共用エイリアスは post_ime_on_direct/post_ime_off_direct への委譲のみである
    // こと。独自の VK 送信を再実装すると、6回反転の教訓を踏まえない別経路が生まれる。
    // （2026-08-06: MsImeDirectStrategy の ON も VK_DBE_HIRAGANA → VK_IME_ON へ移行し
    // GjiDirectStrategy と同じキーになったため、MS-IME 専用の post_ms_ime_on/off は
    // 呼び出し元を失い削除した。BUG-50 参照）
    let gji_on = extract_fn_body(production, "pub unsafe fn post_gji_ime_on(");
    assert!(
        gji_on.contains("post_ime_on_direct()"),
        "{path} の post_gji_ime_on が post_ime_on_direct() に委譲していません。"
    );
    let gji_off = extract_fn_body(production, "pub unsafe fn post_gji_ime_off(");
    assert!(
        gji_off.contains("post_ime_off_direct()"),
        "{path} の post_gji_ime_off が post_ime_off_direct() に委譲していません。"
    );

    // 最終フォールバック: VK_KANJI トグルを down/up ちょうど1回ずつ送る。
    // （関数本体には import 文・診断ログ・コメントにも `VK_KANJI` という部分文字列が
    // 複数回出現するため、実際に SendInput へ push する `make_key_input_ex(VK_KANJI, ..)`
    // の down/up 引数だけを数える。ヘルパー名のリネームにも意味論的に頑健。）
    let kanji_toggle = extract_fn_body(production, "pub unsafe fn post_kanji_toggle_to_focused(");
    let down_count = kanji_toggle.matches("VK_KANJI, false").count();
    let up_count = kanji_toggle.matches("VK_KANJI, true").count();
    assert_eq!(
        (down_count, up_count),
        (1, 1),
        "{path} の post_kanji_toggle_to_focused 内 VK_KANJI down/up 送信回数が想定(1, 1)と \
         異なります(実際: ({down_count}, {up_count}))。"
    );
}

/// ADR-086 §4 INV-14/INV-19（2026-08-08、全 6 経路の移行完了に伴い更新）:
/// `set_ime_romaji_mode_with_target`/`_async`（実行時に `get_focused_hwnd()` を
/// ライブクエリして書き込み先を決める、ターゲット同一性を持たない低レベル API）
/// は `ime.rs` から**削除済み**。このテストは再導入されないことを固定する
/// tripwire として残す（`known_sites` は空 = 出現数 0 が唯一の正しい状態）。
///
/// この関数は起案時点と実行時点で書き込み先ウィンドウが変わっても検知できない
/// （ADR-086 §1.2 欠陥1）。BUG-59 追補（`9c102b02`）は `platform.rs` に7番目の
/// 直接呼び出しを追加したが、当時このテストが存在せず検知できなかった（実機で
/// LINE の全打鍵が「い」になる等の実害が出て revert 済み、`docs/known-bugs.md`
/// BUG-59 追補参照）。このテストが失敗したら、新しい呼び出しは
/// `ActuationTarget::capture` → `set_ime_conv_for_target`/
/// `set_ime_open_then_conv_for_target` 経由に置き換えること（低レベル関数を
/// 再実装しないこと）。
#[test]
fn conv_write_call_sites_are_target_explicit() {
    const NEEDLE: &str = "set_ime_romaji_mode_with_target_async(";
    let known_sites: &[(&str, usize)] = &[];

    // BUG-59 追補（`9c102b02`）は当時の known_sites のどのファイルにも無かった
    // `platform.rs` に直接呼び出しを追加した。固定リストへの grep だけでは
    // 「新しいファイルに呼び出しが増えた」ケースを検知できないため、
    // `src/` 全体を走査して実際に呼び出しを含むファイル集合を求め、
    // known_sites のキー集合と完全一致することも別途検証する。
    let all_files = list_src_files();
    let mut files_with_calls: Vec<(String, usize)> = Vec::new();
    for path in &all_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            files_with_calls.push((path.clone(), count));
        }
    }
    files_with_calls.sort();

    let mut expected: Vec<(String, usize)> = known_sites
        .iter()
        .map(|(p, c)| ((*p).to_string(), *c))
        .collect();
    expected.sort();

    assert_eq!(
        files_with_calls, expected,
        "`{NEEDLE}` を含むファイル集合/出現数が想定（空）と異なります。\n\
         想定: {expected:?}\n実際: {files_with_calls:?}\n\
         `set_ime_romaji_mode_with_target(_async)` は ADR-086 §5 Phase1b step6 \
         で削除済みの低レベル API です。再実装せず、`ActuationTarget::capture` \
         → `set_ime_conv_for_target`/`set_ime_open_then_conv_for_target` \
         経由で書き込むこと。"
    );
}

/// ADR-086 §4 INV-14/INV-19（2026-08-08、全 6 経路の移行完了に伴い新設）:
/// `ActuationTarget::capture` の呼び出し箇所数をファイルごとに固定する。
///
/// 旧 `set_ime_romaji_mode_with_target_async` の出現数チェック
/// （`conv_write_call_sites_are_target_explicit`）は、そのライブクエリ版
/// 自体が削除された今、「新しい force-write 経路が追加されたこと」を検知する
/// 力を失った（呼び出す対象が無いので誰も呼べない）。代わりに
/// `ActuationTarget::capture` — 全ての target-aware 書き込みが必ず通る
/// 唯一の入口 — の呼び出し箇所数を固定することで、同じ役割
/// （BUG-59 追補のような「新しい経路が未追跡のまま増える」検知）を引き継ぐ。
#[test]
fn actuation_target_capture_call_sites_are_accounted_for() {
    const NEEDLE: &str = "ActuationTarget::capture(";
    let known_sites: &[(&str, usize)] = &[
        ("src/output/conv_actuation.rs", 1), // actuate_conv_mode（ADR-084 INV-1 単一窓口、2026-08-08 Runtime→Output移設）
        ("src/tsf/warmup/cold_warmup.rs", 1), // ColdWarmupSequence::run_start
        ("src/runtime/executor.rs", 1),      // dispatch_ime_set_open（ImmCross async path）
        ("src/runtime/key_pipeline.rs", 3), // kp_reset_to_hiragana_romaji_capsoff / kp_restore_kana_from_half_width / apply_focus_probe(ImmCrossProbe kana修正)（apply_idle_conv_check の restore_roman(BUG-08 Apply(3))経路は2026-08-17 BUG-61に伴い撤去）
        ("src/runtime/mod.rs", 1), // try_force_on_bootstrap（BUG-34 横展開 D、2026-08-19: 同期 ImmCrossProcessStrategy::apply 経由の force-on を run_open_chain_async へ移行）
    ];

    let all_files = list_src_files();
    let mut files_with_calls: Vec<(String, usize)> = Vec::new();
    for path in &all_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            files_with_calls.push((path.clone(), count));
        }
    }
    files_with_calls.sort();

    let mut expected: Vec<(String, usize)> = known_sites
        .iter()
        .map(|(p, c)| ((*p).to_string(), *c))
        .collect();
    expected.sort();

    assert_eq!(
        files_with_calls, expected,
        "`{NEEDLE}` を含むファイル集合/出現数が想定と異なります。\n\
         想定: {expected:?}\n実際: {files_with_calls:?}\n\
         新しい force-write 経路を追加する場合は ActuationTarget::capture を \
         起案時点（spawn_local ブロック先頭、他の await より前）で1回呼び、\
         この known_sites を更新すること。毎試行 capture するループは検証を \
         事実上 no-op 化するため避けること（opus アドバーサリアルレビュー \
         2026-08-08、key_pipeline.rs::kp_restore_kana_from_half_width 参照）。"
    );
}

/// ADR-086 §1.2 欠陥1 / opus レビュー指摘（2026-08-08）: `ActuationTarget::capture`
/// は spawn した async ブロックの先頭、いかなる他の await よりも前に置かれて
/// いなければならない。executor.rs（open と同じウィンドウへ ROMAN を補完する
/// はずが、open 完了を待つ間にフォーカスが動くと別ウィンドウへ誤爆しうる）・
/// cold_warmup.rs（診断 read 待機中に abort 率が自ら上がる）で、この順序が
/// 守られていなかった実装バグが見つかり個別に修正された（#19〜#21、
/// `conv_actuation.rs::actuate_conv_mode` だけが最初から正しい順序だった）。
/// 同じ退行を機械的に検知する。
///
/// 判定方法: `spawn_local(async move { ... })`（または `async { ... }`）ブロックを
/// 対象ファイルから抽出し、`ActuationTarget::capture(` を含むブロックについて、
/// ブロック内で最初に出現する `.await` が capture 自身の await であることを
/// 確認する（capture 呼び出しの前に他の await が無いことと等価）。
#[test]
fn actuation_target_capture_is_first_await_in_spawn_local_block() {
    let target_files = [
        "src/output/conv_actuation.rs",
        "src/tsf/warmup/cold_warmup.rs",
        "src/runtime/executor.rs",
        "src/runtime/key_pipeline.rs",
    ];
    let mut checked = 0usize;
    for path in target_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        // 行コメントを除去してから抽出する。`.await` という文字列がコメント中に
        // 出現すると、ブロック内の実コードより前に「最初の await」と誤認しうる。
        let stripped: String = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for block in extract_all_balanced_blocks(&stripped, "spawn_local(async") {
            // ネストした spawn_local（独立してスケジュールされ、外側ブロックとは
            // 別の実行タイミングで走る）の中身は、外側ブロック自身の「最初の
            // await」判定に混入させない。extract_all_balanced_blocks は入れ子も
            // 別要素として返すため、ネストしたブロック自身は別途このループの
            // 後続イテレーションで独立に検査される。
            let masked = mask_nested_needle_blocks(block, "spawn_local(async");
            if !masked.contains("ActuationTarget::capture(") {
                continue; // この spawn_local（自身のスコープ内）は conv write と無関係
            }
            checked += 1;
            let first_await = masked.find(".await").unwrap_or_else(|| {
                panic!(
                    "{path}: ActuationTarget::capture を含む spawn_local ブロックに \
                     .await が見つかりません"
                )
            });
            let prefix = &masked[..first_await];
            assert!(
                prefix.contains("ActuationTarget::capture("),
                "{path}: spawn_local ブロック内で最初の .await が \
                 ActuationTarget::capture ではありません（他の await が先に \
                 実行されています）。capture を await するより前に他の await が \
                 あると、focus_gen 更新の遅延窓で verify_still_current が空虚に \
                 一致してしまい ADR-086 INV-14 の検証が効かなくなります \
                 （executor.rs/cold_warmup.rs で実際に踏んだバグ、2026-08-08）。\
                 ブロック冒頭:\n{}",
                &masked[..masked.len().min(300)]
            );
        }
    }
    assert_eq!(
        checked, 6,
        "ActuationTarget::capture を含む spawn_local ブロックの検査対象数が \
         想定(6)と異なります。新しい経路を追加/削除した場合は \
         actuation_target_capture_call_sites_are_accounted_for と合わせて \
         この期待値も更新すること。"
    );
}

/// ADR-086 §4 INV-15（2026-08-08、Phase 2/3 実装に伴い新設）: 生の `FocusChange`
/// イベントハンドラ自体が force-write（conv-mode の実書き込み）を起こしては
/// ならない。`BUG-59` 追補（`9c102b02`、revert 済み）はまさにこの種の関数へ
/// 直接書き込みを追加して実機事故を起こした。
///
/// - `platform.rs::gji_on_focus_change`（conv 軸、生の FocusChange イベント
///   ハンドラ本体）は `Output::on_ime_mode_focus_changed`（武装のみ）を呼ぶことは
///   許されるが、`ActuationTarget::capture`/`actuate_conv_mode`/
///   `set_ime_conv_for_target` を**直接**呼んではならない。
///
/// NOTE: open/close 軸の force-write（`arm_force_open_pending`/
/// `consume_force_open_pending`、ADR-086 Phase 3）は 2026-08-17、ADR-094 で
/// force ポリシー自体を撤去したのに伴い削除した。この2つ目の走査対象
/// （`runtime/ime_refresh.rs::ir_post_focus_change_snapshot` と
/// `runtime/mod.rs::arm_force_open_pending`）も同時に削除した。
#[test]
fn force_write_is_not_triggered_by_raw_focus_change() {
    let path = "src/platform.rs";
    let fn_needle = "fn gji_on_focus_change";
    let forbidden_list: &[&str] = &[
        "ActuationTarget::capture(",
        "actuate_conv_mode(",
        "set_ime_conv_for_target(",
    ];

    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let body = extract_fn_body(production, fn_needle);

    for forbidden in forbidden_list {
        assert!(
            !body.contains(forbidden),
            "{path}::{fn_needle} 本体に {forbidden:?} が見つかりました。\
             生の FocusChange イベントハンドラが force-write を直接起こしています \
             （ADR-086 INV-15 違反、BUG-59 追補 `9c102b02` と同型の事故）。\
             書き込みは武装（`force_pending` フラグを立てるだけ）に留め、\
             実際の書き込みは送信要求という入力意図に紐づく唯一の消費点からのみ \
             行うこと。"
        );
    }
}

/// ADR-086 §4 INV-15（2026-08-08、2回目 opus アドバーサリアルレビュー M2）:
/// `ir_post_focus_change_snapshot` に実在する既存の open 書き込み
/// （`set_ime_open`〈IME OFF 強制〉、force-write とは無関係）の出現数を固定する。
///
/// `force_write_is_not_triggered_by_raw_focus_change` の禁止リストからこれを
/// 除外する代わりに、ここで出現数を固定することで「新しい force-write
/// 経路がこのラッパー経由で紛れ込んでも検知できない」という穴を塞ぐ。
///
/// **2026-08-21（ADR-098 決定2、BUG-69）**: `apply_ime_open_with_applied(`
/// のガード（旧: 1 = GJI TsfNative VK_IME_ON 強制）は撤去した。この関数内の
/// 唯一の呼び出し元だった TsfNative force-on ブロック自体を削除し、
/// `apply_ime_open_with_applied` メソッドごと削除したため、`.apply_ime_open_with_applied(`
/// の出現数ゼロは `ime_open_actuation_entry_points_are_accounted_for`
/// （crate 全域の入口カウント）が固定する。本関数専用のガードとしては
/// 二重になるため撤去する。
#[test]
fn ir_post_focus_change_snapshot_write_call_sites_are_accounted_for() {
    let path = "src/runtime/ime_refresh.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "fn ir_post_focus_change_snapshot");

    // **ADR-090 A-1**: 実呼び出しは `set_ime_open_ordered(` へ移った
    // （トレイトメソッドには `ActuationOrder` 引数を足せないため、
    // §2.A 設計案 3）。`set_ime_open(` に残るのはログメッセージ 1 件
    // （`log::debug!("... set_ime_open(false) called ...")`）だけ。
    let set_ime_open_count = count_real_calls(body, "set_ime_open(");
    assert_eq!(
        set_ime_open_count, 1,
        "{path}::ir_post_focus_change_snapshot 内の `set_ime_open(` 出現数が \
         想定(1 = ログメッセージのみ。実呼び出しは set_ime_open_ordered へ移行)と\
         異なります(実際: {set_ime_open_count})。トレイトメソッド \
         `set_ime_open` を直接呼ぶと warrant を通さない actuation 入口が\
         復活します（ADR-090 §2.A・INV-47）。"
    );
    let ordered_count = count_real_calls(body, "set_ime_open_ordered(");
    assert_eq!(
        ordered_count, 1,
        "{path}::ir_post_focus_change_snapshot 内の `set_ime_open_ordered(` \
         出現数が想定(1 = IME OFF 強制)と異なります(実際: {ordered_count})。\
         新しい呼び出しを追加した場合はこの期待値を更新し、それが force-write \
         でないことを確認すること。"
    );
}

// NOTE: `force_policy_is_read_from_a_single_decision_point` と
// `is_force_policy_call_sites_are_accounted_for`（ADR-086 §6段3-4/§7-12）は
// 2026-08-17、ADR-094 で `conv_mode_policy`/`Output::is_force_policy()` 自体を
// 撤去したのに伴い削除した。

/// ADR-087 INV-28（実装記録 §8.10、item16(a)）:
/// force-write 経路（`force_on_and_correct_romaji` / GJI TsfNative 強制ON）は
/// `applied` に `None` を渡すことで `GjiDirectStrategy::apply`
/// （`gji_direct_already_matches`、`shadow_on == Some(true)` のとき
/// `VK_IME_ON` を no-op skip する）を最初から bypass する設計になっている
/// （`build_ime_control_view(None)` → `control.shadow_on = None`、
/// `platform.rs::build_ime_control_view` 参照。`None` は `Some(true)` とも
/// `Some(false)` とも一致しないため、ON方向・OFF方向のどちらの
/// already-matched判定もbypassする——BUG-113 修正後もこの性質は保たれる、
/// `docs/known-bugs.md` BUG-113 参照）。
///
/// この不変条件が崩れる（`None` の代わりに実 `applied` 値を渡すよう変更される）と、
/// force-ON 経路が古い shadow_on=ON を見て no-op に阻まれ、BUG-16 が実装レベルで
/// 再発しうる。「`applied` を `None` にして bypass する」という意図はコメントでしか
/// 表現されておらず、コンパイラは強制しないため、テキスト走査で固定する。
///
/// **2026-08-21（ADR-098 決定2、BUG-69）**: 旧第2 assertion（`ir_post_focus_change_snapshot`
/// の `apply_ime_open_with_applied(order, None)` 1件を固定）は撤去した。
/// TsfNative force-on ブロック（唯一の呼び出し元）を削除したため。決定1適用後は
/// `shadow_on=false` になった通常 strategy chain と、決定1-c で有界化された
/// `apply_force_on_for_imm_broken`（本テストが固定する `force_on_and_correct_romaji`
/// 経由）の両方が INV-28 の bypass を担う——**この関数（`force_on_and_correct_romaji`）
/// だけが INV-28 の唯一の enforcement 拠点**であることに注意。
#[test]
fn force_write_paths_bypass_gji_shadow_on_via_none_applied() {
    // `.contains()` は文字列リテラル（コメント含む）にもマッチし、
    // 呼び出し箇所を実際に書き換えても壊れなければ vacuous になる
    // （2026-08-10 Opus レビュー M1: `ime_refresh.rs:481` の行コメントだけで
    // 2つ目の assertion が偽陽性に通っていた）。`count_real_calls`
    // （コメント行除外・`fn` 定義行除外）を使い、かつ関数本体スコープに
    // 限定することで、実際の呼び出しが変更されたときにだけ検知する。
    let mod_rs = read_crate_file("src/runtime/mod.rs");
    let mod_production = production_code_only(&mod_rs);
    let force_on_body = extract_fn_body(mod_production, "fn force_on_and_correct_romaji");
    assert_eq!(
        count_real_calls(force_on_body, "build_ime_control_view(None)"),
        1,
        "force_on_and_correct_romaji は build_ime_control_view(None) を経由して \
         shadow_on=false を作ることで GJI の no-op skip を bypass する設計。\
         `None` 以外の値を渡すよう変更された場合、ADR-087 INV-28 の前提が崩れる。"
    );
}

/// BUG-113 追補（Opus 敵対的レビューで発見・修正）: `open_chain.rs::fallback_write`
/// は先行機構（ImmCross）が実際に OS を読み戻して「まだ desired 状態でない」
/// ことを確認した`Failed`の後にしか呼ばれない（`imm_cross_write`参照）。
/// この時点で`shadow_ime_control_view()`が返す実`applied`をそのまま使うと、
/// `key_pipeline.rs::kp_stage_shadow_ime_toggle`のImmCross経路がactuationの
/// **前**に書く`record_confirmed(false)`（pre-actuation write）を読み返す
/// 循環になり、`GjiDirectStrategy`の`gji_direct_already_matches`が誤って
/// `AlreadyMatched`を返し、実際にはOSがまだON なのに`VK_IME_OFF`が送られない
/// 回帰を作り込む。`fallback_write`は`view.control.shadow_on`を明示的に
/// `None`へ上書きしてこの循環をbypassする設計（`force_on_and_correct_romaji`
/// のINV-28と同じ語彙）。テキスト走査でこの1行を固定する。
#[test]
fn fallback_write_bypasses_gji_shadow_on_via_none_override() {
    let open_chain_rs = read_crate_file("src/runtime/open_chain.rs");
    let production = production_code_only(&open_chain_rs);
    let fallback_write_body = extract_fn_body(production, "fn fallback_write");
    assert_eq!(
        count_real_calls(fallback_write_body, "view.control.shadow_on = None"),
        1,
        "fallback_write は view.control.shadow_on = None で GjiDirect/MsImeDirect の \
         already-matched skip を bypass する設計（BUG-113 追補）。この上書きが \
         削除・変更されると、ImmCross Failed 後のフォールバックが \
         pre-actuation write を読み返して自分の送信を握り潰す回帰が再発する。"
    );
}

// ── ADR-089 Phase B（§2.3・§6 item 6/7）─────────────────────────────────────

/// 実 actuation の起案が `ActuationOrder::issue()` 1 本を通ることを固定する
/// （ADR-090 §2.A A-1、INV-47）。
///
/// # 何が変わったか（ADR-089 Phase B → ADR-090 A-1）
///
/// Phase B の時点では `issue_open_warrant()`（ADR-087）の本番呼び出し元が
/// ゼロで、既存の apply 経路は `OpenWarrant` を持たなかった。そのため
/// warrant を素通しする暫定入口 `warrant_pending_adr087()` を 2 箇所
/// （同期チェーン / 非同期チェーン）が通っており、本テストはその件数
/// （2）を固定していた。
///
/// **ADR-090 A-1 で `warrant_pending_adr087()` は削除した。**
/// `Requested → Warranted` の経路は
/// (a) `Actuation::warrant(OpenWarrant)`（実 warrant を要求）と
/// (b) `ActuationOrder::into_actuation_shadow()` / `into_actuation()` だけで
/// あり、`ActuationOrder` の唯一の構築経路 `issue()` は
/// `issue_open_warrant()` の戻り値をそのまま受ける。したがって
/// **「warrant を発行せずに actuation を起案する」ことが型として書けない**。
///
/// 本テストは残った実行時の抜け道——`Actuation::request(` を
/// `ActuationOrder` の外で呼ぶこと——を件数で塞ぐ。
///
/// # 【重要】本テストが今固定しているのは「死んだコードが 2 箇所」である
///
/// 下で内訳 `[("src/state/actuation_chain.rs", 2)]` に固定している 2 箇所
/// （`ActuationOrder::into_actuation` / `DriftEpisode::next_attempt`）は、
/// **どちらも本番から到達不能**である（2026-08-12 の PR 最終レビューで確認）:
///
/// - `into_actuation` の参照は定義自体・`actuation_chain.rs` のモジュール doc・
///   本コメントだけで、本番呼び出し元はゼロ。
/// - `DriftEpisode::new` の呼び出しは `actuation_chain.rs` の
///   `#[cfg(test)] mod tests` にしか無く、`DriftEpisode` 型ごと本番未配線。
///
/// A-1 後に本番で生きている `Requested → Warranted` 経路は
/// **`into_actuation_shadow`（`ime_controller.rs` / `runtime/open_chain.rs`）
/// の 1 本だけ**であり、それは warrant の有無に関わらず `Warranted` へ進める
/// （授権が無ければ `Authorization::LegacyUnwarranted { would_have_blocked: true }`
/// を載せるだけで**書き込みは止めない**）。つまり **A-1 の時点では
/// `Warranted` は「実 `OpenWarrant` がある」ことを意味しない**。
/// ADR-089 §2.3 が意図した型による保証が効き始めるのは、**A-2 で入口ごとに
/// `into_actuation_shadow` → `into_actuation` へ差し替え終えたとき**である
/// （入口ごとに実機ソークが必須、ADR-090 §6 ステップ 7 / §2.A A-5'）。
/// **本テストが緑であることを「INV-47 は守られている」と読まないこと。**
///
/// # なぜ型で閉じないのか
///
/// `Actuation::request` を `pub(crate)` 未満にはできない
/// （`state/actuation_chain.rs` のモジュール doc の compile_fail doctest が
/// crate 外から `Actuation::request(..).warrant(..)` を組み立てており、
/// それは**正規経路の説明**として必要）。
#[test]
fn actuation_is_only_requested_through_actuation_order() {
    // どちらも `state/actuation_chain.rs` の中で、**実 `OpenWarrant` を伴う**
    // 構築だけ（ただし**2 箇所とも本番未配線の死んだコード**。上の doc 参照）:
    //   1. `ActuationOrder::into_actuation`（A-2 用。本番呼び出し元ゼロ）
    //   2. `DriftEpisode::next_attempt`（同一 warrant からの再試行。回数制限は
    //      `decide_actuation_action` が持つ、INV-41。`DriftEpisode::new` は
    //      テストからしか呼ばれておらず、これも本番未配線）
    // **`state/actuation_chain.rs` の外に出たら、それは warrant を持たない
    // 起案経路が復活したということ。**
    let files = list_src_files();
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "Actuation::request(");
        if count > 0 {
            breakdown.push((path.clone(), count));
        }
    }
    assert_eq!(
        breakdown,
        vec![("src/state/actuation_chain.rs".to_string(), 2)],
        "`Actuation::request(` の本番呼び出しは `state/actuation_chain.rs` の \
         2 箇所（`ActuationOrder::into_actuation` / `DriftEpisode::next_attempt`、\
         どちらも実 `OpenWarrant` を伴い、どちらも A-2 まで本番未配線）\
         だけにすること。実際: {breakdown:?}\n\
         実 actuation は `ActuationOrder::issue()`（= `issue_open_warrant()` を\
         必ず通る）から起案してください（ADR-090 §2.A・INV-47）。"
    );
    // 素通し入口が復活していないこと（ADR-090 A-1 で削除済み）。
    // コメント行は除外する——`state/actuation_chain.rs` のモジュール doc は
    // 「Phase B ではこの入口があった / A-1 で削除した」という経緯を
    // 名前付きで残しており（`.claude/rules/experiment-logging.md` の
    // 「なぜ前回それを捨てたのかを辿れるようにする」規約）、それは残すべき記録
    // である。塞ぎたいのは**実際の呼び出しと定義**の復活だけ。
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let live = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains("warrant_pending_adr087"))
            .count();
        assert_eq!(
            live, 0,
            "{path}: `warrant_pending_adr087` は ADR-090 A-1 で削除した。\
             warrant を素通しする入口を再導入しないこと（INV-47）。"
        );
    }
}

/// `WarrantContext` の組み立てが `ImeStateHub::warrant_context()` 1 箇所に
/// 限られることを固定する（ADR-090 §2.A A-R3、INV-48）。
///
/// `WarrantContext` は 8 フィールドで、うち `intent_store` は `ImeStateHub` の
/// **private フィールド**である。実 actuation 入口は外部 8 経路あるので、
/// 各入口がリテラルで組み立てると (a) `intent_store` の private を崩すか、
/// (b) 同じ組み立てが 8 箇所に散る（ADR-087 §7 round4 N-A が
/// `WarrantContext` を導入して避けたかったもの）。
#[test]
fn warrant_context_is_built_in_one_place() {
    let files = list_src_files();
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains("WarrantContext {"))
            .count();
        if count > 0 {
            breakdown.push((path.clone(), count));
        }
    }
    assert_eq!(
        breakdown,
        vec![("src/state/platform_state.rs".to_string(), 1)],
        "`WarrantContext {{` のリテラル構築は `ImeStateHub::warrant_context()` の\
         1 箇所だけにすること（ADR-090 INV-48）。実際: {breakdown:?}"
    );
}

/// ImmCross を含む非同期 actuation の入口が `run_open_chain_async` 1 本である
/// ことを固定する（ADR-089 §6 Phase B item 6、二重経路の解消）。
///
/// 旧 `apply_skipping_imm`（async IMM が `Failed` を返した後の 2 本目の走査
/// 入口）は撤去済み。`spawn_local` の中で ImmCross の書き込みを直接呼ぶコードを
/// 足すと、フォールスルー規則（`state/actuation_chain.rs::falls_through`）を
/// 迂回する 2 本目の経路が復活する。
#[test]
fn async_imm_cross_actuation_goes_through_the_single_chain_entry() {
    // ImmCross の実書き込み API を、機構チェーン外から呼ぶ既知の箇所（後述）。
    const IMM_WRITE_SITES: [(&str, &[(&str, usize)]); 2] = [
        (
            "set_ime_open_then_conv_for_target(",
            &[("src/runtime/open_chain.rs", 1)],
        ),
        (
            "set_ime_open_cross_process_async(",
            &[
                ("src/runtime/open_chain.rs", 1),
                ("src/platform.rs", 1),
                ("src/runtime/mod.rs", 2),
            ],
        ),
    ];
    let files = list_src_files();

    // 1. `apply_skipping_imm` は完全に消えていること。
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        assert_eq!(
            count_real_calls(production, "apply_skipping_imm("),
            0,
            "{path} に `apply_skipping_imm(` が残っています（ADR-089 Phase B で撤去済み）"
        );
    }

    // 2. ImmCross の実書き込み API を、機構チェーン外から呼ぶ箇所を固定する。
    //
    // `set_ime_open_then_conv_for_target`（ADR-086 INV-14 準拠の open+conv 書き込み）は
    // チェーン専用。`set_ime_open_cross_process_async` は「open を 1 回書く」だけの
    // 低レベル API で、チェーン以外にも **actuation ではない**既知の用途がある:
    //
    // - `platform.rs::set_ime_open`（fire-and-forget。outcome を呼び出し元へ返さず
    //   フォールバックも持たないため、そもそもチェーンの対象ではない）
    // - `runtime/mod.rs::panic_reset`（OFF → ON を 1 タスク内で直列化する復旧手順。
    //   ADR-087 の SafetyValve 相当であり、戦略選択の対象ではない）
    //
    // ここを増やす＝フォールスルー規則を迂回する経路を増やす、なので件数で固定する。
    for (needle, expected_sites) in IMM_WRITE_SITES {
        let mut breakdown: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            // 定義元（`ime.rs`）は `fn ...(` 行が除外されるが、内部委譲があるため除く。
            if path == "src/ime.rs" {
                continue;
            }
            let count = count_real_calls(production, needle);
            if count > 0 {
                breakdown.push((path.clone(), count));
            }
        }
        breakdown.sort();
        let mut expected: Vec<(String, usize)> = expected_sites
            .iter()
            .map(|(p, n)| ((*p).to_string(), *n))
            .collect();
        expected.sort();
        assert_eq!(
            breakdown, expected,
            "`{needle}` の呼び出し箇所が想定と異なります。ImmCross を機構チェーンの\
             外で書くとフォールスルー規則（`state/actuation_chain.rs::falls_through`）を\
             迂回する 2 本目の経路になります（ADR-089 §2.3）。"
        );
    }

    // 3. 非同期チェーンの入口は 1 本（定義 1 + 呼び出し 3）。
    //
    // **2026-08-19（BUG-34 横展開 D）**: mod.rs::try_force_on_bootstrap が
    // 3本目の呼び出し元として加わった（executor.rs / key_pipeline.rs は既存）。
    // 同期 ImmCrossProcessStrategy::apply（エンジンスレッドを直接ブロックする
    // SendMessageTimeoutW 経路）から、この単一チェーン入口へ移行したもの。
    let mut entry_calls = 0usize;
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        entry_calls += count_real_calls(production, "run_open_chain_async(");
    }
    assert_eq!(
        entry_calls, 3,
        "`run_open_chain_async(` の呼び出し箇所数が想定(3: executor.rs / \
         key_pipeline.rs / runtime/mod.rs)と異なります(実際: {entry_calls})。"
    );
}

/// `PerSourceObservations::set` の本番呼び出し元を `ObservationStore` 内の
/// 1 箇所（`record_replayed`）に固定する（ADR-089 §9-11 の「裏口」封じ）。
///
/// Phase A の時点では `set` が `pub` で、`store.per_source.set(ImeObservation { .. })`
/// と書けば witness も `record`/`record_belief` も経由せずに観測を注入できた。
/// Phase B で `pub(crate)` へ縮小したうえで、crate 内の呼び出し元数もここで固定する。
#[test]
fn per_source_set_is_confined_to_the_store() {
    let files = list_src_files();
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "per_source.set(");
        if count > 0 {
            breakdown.push((path.clone(), count));
        }
    }
    assert_eq!(
        breakdown,
        vec![("src/state/observation_store.rs".to_string(), 1)],
        "`per_source.set(` は `ObservationStore::record_replayed` からのみ呼ぶこと\
         （ADR-089 §2.1・§9-11）。実際: {breakdown:?}"
    );
}

/// `per_source` の**フィールドへの直接代入**が本番コードに存在しないことを固定する
/// （ADR-090 §2.C 設計案 3、INV-49）。
///
/// # なぜ `per_source_set_is_confined_to_the_store` だけでは足りないのか
///
/// ADR-089 Phase B が縮小したのは `PerSourceObservations::set` だけだったが、
/// `set` は「フィールド代入の便利メソッド」であって唯一の入口ではなかった。
///
/// ```ignore
/// store.per_source.observer_poll = Some(ImeObservation { source: .., .. });
/// ```
///
/// と書けば `set` を通らずに観測を注入できる。crate 外からの経路は
/// ADR-090 §2.C が `per_source` の `pub(crate)` 化 + `ImeObservation` の
/// `#[non_exhaustive]` で構造的に塞いだが、**crate 内では依然として書ける**。
/// 型で消せない残余なので、本番コードでの件数をここで 0 に固定する。
///
/// テストコード（`#[cfg(test)] mod tests` 以降）は対象外——`platform_state.rs` の
/// stale 観測シミュレーション（`.at = stale_at`）のように、状態を人為的に作る
/// 必要がある。
#[test]
fn per_source_fields_are_not_assigned_directly() {
    // `PerSourceObservations` の 9 フィールド（`observation_store.rs`）。
    const FIELDS: [&str; 9] = [
        "focus_probe",
        "observer_poll",
        "gji",
        "imm_get_open_status",
        "tsf",
        "hwnd_cache",
        "imm_cross_probe",
        "heuristic_default",
        "conv_open_inference",
    ];
    let files = list_src_files();
    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        for (lineno, line) in production.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || !trimmed.contains("per_source") {
                continue;
            }
            for field in FIELDS {
                // `per_source.<field> =` / `per_source\n  .<field> = ` の
                // 素朴な形。複数行に割れた代入は検出できないが、
                // `per_source` を含む行自体が本番に 0 行であることを
                // 別途この走査が示すので実害は無い。
                if trimmed.contains(&format!("per_source.{field}")) {
                    hits.push(format!("{path}:{}: {}", lineno + 1, trimmed));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "`per_source` の各フィールドへ本番コードから直接触らないこと\
         （ADR-090 §2.C / INV-49）。観測の書き込みは `record` / `record_belief` / \
         `record_replayed` の 3 口のみ、読み取りは `ObservationStore::observation()` \
         または `PerSourceObservations::get()` を使う。実際: {hits:?}"
    );
}

/// 機構 1 つ分の実 write（`ime_controller::apply_mechanism`）の呼び出し元を、
/// **チェーンの writer 実装 2 つだけ**に固定する（ADR-089 §2.3、Phase B 追随）。
///
/// # なぜ必要か
///
/// `legacy_unwarranted_actuation_sites_are_accounted_for`（`Actuation` の起案数）と
/// `async_imm_cross_actuation_goes_through_the_single_chain_entry`（非同期入口数）は
/// **チェーンの入口だけ**を数えており、`apply_mechanism` の呼び出し元は誰も
/// 数えていなかった。`apply_mechanism` は `Actuation` 型状態チェーンを一切構築せずに
/// `SendInput` / `post_kanji_toggle_to_focused` / `ImmSetOpenStatus` を起こせる。
/// ここに 3 本目の呼び出し元が生えると、`falls_through` 規則（次へ進むのは `Failed`
/// のときだけ、特に `UnsafeToToggle` で `VK_KANJI` へ落ちない）も `Actuation` の
/// アフィン性（1 値 = 高々 1 回の成功 write、INV-41）も通らない write 経路になる。
///
/// # なぜ型で閉じないのか
///
/// 現在の 2 箇所はどちらも `MechanismWriter` / `AsyncMechanismWriter` の `write`
/// 実装、すなわち `run_chain` / `run_chain_async` が駆動する **write ステップ
/// そのもの**である。「チェーンを経由させる」ことが定義上できない（実装の中で
/// チェーンを再度張ると再帰する）ため、可視性の縮小でも解けない
/// （`runtime/open_chain.rs` は別モジュールなので `pub(crate)` 未満にできない）。
/// 恒久策は `run_chain` だけが構築できる authorization トークンを
/// `MechanismWriter::write` の引数に通すこと（ADR-089 §9-15）で、Phase C 送り。
#[test]
fn raw_mechanism_write_sites_are_confined_to_chain_writers() {
    let files = list_src_files();

    // 1. `apply_mechanism(` の本番呼び出し元はこの 2 箇所だけ。
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "apply_mechanism(");
        if count > 0 {
            breakdown.push((path.clone(), count));
        }
    }
    breakdown.sort();
    assert_eq!(
        breakdown,
        vec![
            ("src/ime_controller.rs".to_string(), 1),
            ("src/runtime/open_chain.rs".to_string(), 1),
        ],
        "`apply_mechanism(` の呼び出し元は機構チェーンの writer 実装 2 つだけに\
         固定されています（ADR-089 §2.3）。実際: {breakdown:?}\n\
         チェーン外から 1 機構分の実 write を起こす経路を増やさないでください。"
    );

    // 2. 同期側の 1 件は `impl MechanismWriter for SyncChainWriter` の中にある。
    let controller = read_crate_file("src/ime_controller.rs");
    let sync_writer = extract_fn_body(&controller, "impl MechanismWriter for SyncChainWriter");
    assert_eq!(
        count_real_calls(sync_writer, "apply_mechanism("),
        1,
        "`ime_controller.rs` の `apply_mechanism(` は \
         `impl MechanismWriter for SyncChainWriter` の中にあること（ADR-089 §2.3）"
    );

    // 3. 非同期側の 1 件は `fallback_write` の中にあり、その `fallback_write` は
    //    `impl AsyncMechanismWriter for AsyncChainWriter` からのみ呼ばれる。
    let open_chain = read_crate_file("src/runtime/open_chain.rs");
    let open_chain_production = production_code_only(&open_chain);
    let fallback = extract_fn_body(&open_chain, "fn fallback_write");
    assert_eq!(
        count_real_calls(fallback, "apply_mechanism("),
        1,
        "`runtime/open_chain.rs` の `apply_mechanism(` は `fallback_write` の中に\
         あること（ADR-089 §2.3）"
    );
    let async_writer = extract_fn_body(
        &open_chain,
        "impl AsyncMechanismWriter for AsyncChainWriter",
    );
    assert_eq!(
        count_real_calls(open_chain_production, "fallback_write("),
        count_real_calls(async_writer, "fallback_write("),
        "`fallback_write(` は `impl AsyncMechanismWriter for AsyncChainWriter` の\
         外から呼ばないこと（ADR-089 §2.3）"
    );

    // 4. 並行する裏口（`ImeOpenStrategy::apply` の直接呼び出し）が塞がれていること。
    //    `pub(crate) struct GjiDirectStrategy` のままだと、crate 内のどこからでも
    //    `GjiDirectStrategy.apply(open, &view)` と書けば `apply_mechanism` を
    //    経由せずに同じ実 write を起こせる。可視性はコンパイラが強制するので、
    //    ここで固定するのは「宣言を再び `pub` へ広げないこと」だけでよい。
    for decl in [
        "trait ImeOpenStrategy",
        "struct ImmCrossProcessStrategy",
        "struct GjiDirectStrategy",
        "struct MsImeDirectStrategy",
        "struct KanjiToggleStrategy",
    ] {
        let line = controller
            .lines()
            .find(|line| line.contains(decl) && !line.trim_start().starts_with("//"))
            .unwrap_or_else(|| panic!("`{decl}` の宣言が `ime_controller.rs` に見つかりません"));
        assert!(
            !line.trim_start().starts_with("pub"),
            "`{decl}` は `ime_controller.rs` の外へ出さないこと（ADR-089 §2.3）。\
             実際の宣言: {line}"
        );
    }
}

/// ADR-089 §6 Phase C item 12（= ADR-086 INV-14 の未移行分の是正）:
/// **同期経路の ROMAN 補完 IMC write は、捕獲済み `ActuationTarget` を必ず通る。**
///
/// Phase C 以前は `ImmCrossProcessStrategy::apply` と `MsImeDirectStrategy::apply`
/// が `crate::ime::set_ime_romaji_mode()`（宛先をライブクエリで write 時点に
/// 自己決定する低レベル API）を**別々に**呼んでいた。`output/conv_actuation.rs`
/// の doc が「ADR-086 Phase 1〜2 の『7 経路』の数え漏れ」と書いていた 2 経路が
/// これである。Phase C で書き込み口を `ime_controller::romaji_pre_write` の
/// 1 箇所へ統合し、`ActuationTarget::capture_blocking` →
/// `set_ime_romaji_mode_for_target_blocking` を通す形にした。
///
/// 本テストが守るのは次の 3 点:
///
/// 1. 削除したライブクエリ版（`set_ime_romaji_mode()` / `_async()`）が
///    本番コードに復活していないこと。
/// 2. 同期捕獲（`ActuationTarget::capture_blocking`）と同期 ROMAN write の
///    呼び出し元が `ime_controller.rs` の 1 箇所ずつであること。
/// 3. その 1 箇所が `romaji_pre_write` の中にあること
///    （= `needs_romaji_pre_write` の条件判定を必ず通ること）。
#[test]
fn sync_romaji_write_goes_through_a_captured_target() {
    let files = list_src_files();

    // 1. 削除済みライブクエリ版の復活検知。
    for removed in ["set_ime_romaji_mode()", "set_ime_romaji_mode_async("] {
        let mut sites: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            let count = production
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .filter(|line| line.contains(removed))
                .count();
            if count > 0 {
                sites.push((path.clone(), count));
            }
        }
        assert!(
            sites.is_empty(),
            "`{removed}`（宛先をライブクエリで自己決定する同期 IMC write）は \
             ADR-089 §6 Phase C item 12 で削除済みです。再実装せず、\
             `ActuationTarget::capture_blocking` → \
             `set_ime_romaji_mode_for_target_blocking` 経由で書き込むこと。\n\
             実際: {sites:?}"
        );
    }

    // 2. 同期捕獲と同期 ROMAN write の呼び出し元。
    for needle in [
        "ActuationTarget::capture_blocking(",
        "set_ime_romaji_mode_for_target_blocking(",
    ] {
        let mut sites: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            let count = count_real_calls(production, needle);
            if count > 0 {
                sites.push((path.clone(), count));
            }
        }
        sites.sort();
        assert_eq!(
            sites,
            vec![("src/ime_controller.rs".to_string(), 1)],
            "`{needle}` の本番呼び出し元は `ime_controller.rs` の \
             `romaji_pre_write` 1 箇所だけに固定されています（ADR-089 Phase C item 12）。\
             実際: {sites:?}"
        );
    }

    // 3. その 1 箇所が `romaji_pre_write` の中にあること。
    let controller = read_crate_file("src/ime_controller.rs");
    let pre_write = extract_fn_body(&controller, "fn romaji_pre_write");
    for needle in [
        "ActuationTarget::capture_blocking(",
        "set_ime_romaji_mode_for_target_blocking(",
    ] {
        assert_eq!(
            count_real_calls(pre_write, needle),
            1,
            "`{needle}` は `romaji_pre_write` の中で呼ぶこと（条件判定 \
             `needs_romaji_pre_write` を迂回させないため、ADR-089 Phase C item 12）"
        );
    }
}

// ── BUG-78: disable_apps（アプリ単位の awase 無効化 + Ctrl/Shift スタック復旧） ──

/// `disable_apps` の早期 return（`hook_callback` 内、`FOCUS_APP_DISABLED` を見る分岐）は
/// ちょうど 1 箇所だけ存在し、`PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS` 更新ブロック
/// より**後**、`VK_KANA` swallow ブロックより**前**に置かれていること。
///
/// 設計上の理由（`.claude/plans` の premortem 参照）: 更新ブロックより前に早期 return する
/// と、無効アプリに入る直前から押していたキーの KeyUp が記録されず、対策したい
/// Ctrl スタック自体をこの分岐が新規に生む。VK_KANA/Alt なりすまし等の変換系ロジックより
/// 前に置くことで、無効化中はそれらの介入も一切効かなくする（ユーザー判断により例外なし）。
#[test]
fn disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana() {
    let content = read_crate_file("src/hook.rs");
    let production = production_code_only(&content);

    let early_return_needle = "FOCUS_APP_DISABLED.load(Ordering::Relaxed)";
    let count = production.matches(early_return_needle).count();
    assert_eq!(
        count, 1,
        "src/hook.rs 内で `{early_return_needle}` の出現数が想定(1)と異なります \
         (実際: {count})。disable_apps の早期 return は hook_callback 内の1箇所に \
         限定すること。"
    );

    let update_block_pos = production
        .find("if let Some(slot) = PHYSICAL_KEY_STATE.get(vk.0 as usize) {")
        .expect("PHYSICAL_KEY_STATE update block not found in src/hook.rs");
    let early_return_pos = production
        .find(early_return_needle)
        .expect("early return needle not found (checked above)");
    let vk_kana_pos = production
        .find("if vk == crate::vk::VK_KANA {")
        .expect("VK_KANA swallow block not found in src/hook.rs");

    assert!(
        update_block_pos < early_return_pos,
        "disable_apps の早期 return は PHYSICAL_KEY_STATE 更新ブロックより後に \
         置くこと（前に置くと無効アプリ突入直前の KeyUp が記録されず、対策したい \
         Ctrl スタックをこの分岐自体が新規に生む）。"
    );
    assert!(
        early_return_pos < vk_kana_pos,
        "disable_apps の早期 return は VK_KANA swallow ブロックより前に置くこと \
         （無効化中は変換系ロジックの介入を一切効かなくする設計）。"
    );
}

/// `clear_hook_latches_for_app_disable` の Leave 分岐は `PHYSICAL_KEY_STATE` の
/// うち Ctrl/Shift の 6 スロットだけを force-false し、**Alt/Win には一切触れない**こと。
///
/// 設計上の理由: Alt+Tab で無効アプリへ出入りする瞬間は Alt が物理押下中であることが
/// 多く、ここで Alt/Win の `PHYSICAL_KEY_STATE` を force-false すると `alt_key_held()` が
/// 偽って BUG-62（Alt+かな で JIS かな直接入力へ不可逆に切り替わる）の保護が外れる。
/// Ctrl/Shift はこのリスクが小さい（Alt+Tab 中に押されていることが稀で、誤ってクリア
/// しても次の物理 KeyDown/KeyUp で自己修復する安全側の誤り）ため対象にする。
#[test]
fn app_disable_leave_edge_clears_only_ctrl_and_shift_not_alt_or_win() {
    let content = read_crate_file("src/hook.rs");
    let body = extract_fn_body(&content, "fn clear_hook_latches_for_app_disable");

    for must_contain in [
        "VK_CONTROL",
        "VK_LCONTROL",
        "VK_RCONTROL",
        "VK_SHIFT",
        "VK_LSHIFT",
        "VK_RSHIFT",
    ] {
        assert!(
            body.contains(must_contain),
            "clear_hook_latches_for_app_disable は {must_contain} をクリア対象に \
             含むこと（BUG-78 対策）。"
        );
    }

    for must_not_contain in ["VK_MENU", "VK_LMENU", "VK_RMENU", "VK_LWIN", "VK_RWIN"] {
        assert!(
            !body.contains(must_not_contain),
            "clear_hook_latches_for_app_disable は {must_not_contain} に触れては \
             いけない（Alt+Tab 離脱時に alt_key_held()/win_key_held() を偽らせ、\
             BUG-62 の Alt+かな 保護を壊すリスクがあるため、設計段階の premortem で \
             除外が決まった）。"
        );
    }
}

#[test]
fn engine_thread_posts_go_through_win32_chokepoint() {
    let mut post_thread_sites = Vec::new();
    let mut post_none_sites = Vec::new();
    for path in list_src_files() {
        let content = read_crate_file(&path);
        let production = production_code_only(&content);
        let uncommented = non_comment_lines(production);
        let post_thread = uncommented.matches("PostThreadMessageW(").count();
        if post_thread > 0 {
            post_thread_sites.push((path.clone(), post_thread));
        }
        let post_none = count_post_message_none_calls(&uncommented);
        if post_none > 0 {
            post_none_sites.push((path, post_none));
        }
    }
    post_thread_sites.sort();
    post_none_sites.sort();
    assert_eq!(
        post_thread_sites,
        vec![("src/hook.rs".to_string(), 1)],
        "PostThreadMessageW direct calls are limited to hook-thread WM_QUIT"
    );
    assert_eq!(
        post_none_sites,
        vec![("src/win32.rs".to_string(), 1)],
        "PostMessageW(None, ..) is only allowed inside win32::post_to_main_thread_with fallback"
    );
}

#[test]
fn key_events_reach_engine_only_via_deliver_key_event() {
    let mut sites = Vec::new();
    for path in list_src_files() {
        let content = read_crate_file(&path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "process_key_event(");
        if count > 0 {
            sites.push((path, count));
        }
    }
    sites.sort();
    assert_eq!(
        sites,
        vec![("src/runtime/message_handlers.rs".to_string(), 1)],
        "Runtime::process_key_event must be called only by deliver_key_event"
    );
}

#[test]
fn enqueue_reinject_call_sites_are_accounted_for() {
    let mut sites = Vec::new();
    for path in list_src_files() {
        let content = read_crate_file(&path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "enqueue_reinject(");
        if count > 0 {
            sites.push((path, count));
        }
    }
    sites.sort();
    assert_eq!(
        sites,
        vec![
            ("src/runtime/executor.rs".to_string(), 2),
            ("src/runtime/key_pipeline.rs".to_string(), 1),
            // TIMER_IME_OFF_RESCUE の再処理が deliver_key_event 経由に統合された分、
            // message_handlers.rs 側の直接呼び出しが1件減った(5→4、追加発見E)。
            ("src/runtime/message_handlers.rs".to_string(), 4),
        ],
        "enqueue_reinject call sites are limited to deliver_key_event plus the documented pending-replay exceptions"
    );
}

/// `WM_EXECUTE_EFFECTS` の post を `message_handlers.rs` 内の箇所数で固定する
/// （コードレビュー指摘8、指摘10）。
///
/// 以前は `deliver_key_event` の各 `Reinjected` 早期return分岐（Nested pump・
/// NonText・consume_post_bypass・process_key_event PassThrough の4箇所）が
/// それぞれ個別に post していたため、drain で複数キーをまとめて処理する際に
/// `WM_EXECUTE_EFFECTS` が N 回投函されうる構造だった。`deliver_key_event`
/// （と `consume_post_bypass`）は post を一切行わず `KeyDelivery` を返すだけに
/// し、post は呼び出し元の責務にした（`deliver_key_event` の doc 参照）。
///
/// 呼び出し元側は2種類のパターンに集約されている:
/// - `post_effects_if_reinjected(delivery)` ヘルパー（指摘10で
///   `handle_wm_key_from_hook` と `handle_wm_timer`(TIMER_IME_OFF_RESCUE) の
///   重複3行パターンを共通化）: `deliver_key_event` の戻り値
///   （`handle_wm_timer` 側は `KeyOrigin::ImeOffRescueReplay` として通した
///   戻り値、以前の `replay_ime_off_rescue_event` 直接呼び出し+`PassThrough`
///   判定から統合済み）が `Reinjected` なら1回。
/// - `handle_wm_drain_output_queue`: ループ後 `any_reinject` なら1回
///   （バッチ内の複数キーをまとめて1回だけ判定するため、ヘルパーとは
///   別パターンのまま）。
///
/// 新しい早期return分岐を追加する場合、個別に post せず
/// `post_effects_if_reinjected` か `handle_wm_drain_output_queue` の
/// いずれかへ集約すること。集約できない正当な理由があるならこのテストの
/// 期待値を更新すること。
#[test]
fn wm_execute_effects_post_sites_are_limited_to_batch_boundaries() {
    let path = "src/runtime/message_handlers.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let raw_post_count = count_real_calls(production, "post_to_main_thread(WM_EXECUTE_EFFECTS)");
    assert_eq!(
        raw_post_count, 2,
        "{path} 内の `post_to_main_thread(WM_EXECUTE_EFFECTS)` 呼び出し箇所数が \
         想定(2)と異なります(実際: {raw_post_count})。\n\
         想定: post_effects_if_reinjected ヘルパー内で1回 / \
         handle_wm_drain_output_queue で1回の計2箇所のみ。\n\
         deliver_key_event・consume_post_bypass は post を行わず KeyDelivery を \
         返すだけにすること（バッチ内で post が N 回重複するのを防ぐため）。"
    );
    let helper_call_count = count_real_calls(production, "post_effects_if_reinjected(");
    assert_eq!(
        helper_call_count, 2,
        "{path} 内の `post_effects_if_reinjected(` 呼び出し箇所数が想定(2)と \
         異なります(実際: {helper_call_count})。\n\
         想定: handle_wm_key_from_hook / handle_wm_timer(TIMER_IME_OFF_RESCUE) の \
         2箇所のみ。新しい呼び出し元を追加する場合、個別に \
         post_to_main_thread(WM_EXECUTE_EFFECTS) せずこのヘルパーへ集約すること。"
    );
}

/// コードレビュー指摘3の回帰テスト: `deliver_key_event` の `FocusKind::NonText`
/// 早期returnが `KeyOrigin::ImeOffRescueReplay` を対象外にしていること。
///
/// `TIMER_IME_OFF_RESCUE` が `deliver_key_event` 経由に統合された結果
/// （追加発見E）、50ms 救済窓満了時に focus_kind が（フォーカス遷移中等で
/// 一時的・誤って）`NonText` と分類されていると、この早期returnがユーザーの
/// 明示的な IME OFF ジェスチャーをリトライなしで黙って無効化してしまう
/// 回帰があった。early-return の条件式が origin を除外していることを
/// ソース走査で固定する。
#[test]
fn deliver_key_event_nontext_early_return_excludes_ime_off_rescue_replay() {
    let content = read_crate_file("src/runtime/message_handlers.rs");
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "pub(crate) fn deliver_key_event(");
    let nontext_idx = body
        .find("FocusKind::NonText")
        .expect("deliver_key_event must check FocusKind::NonText");
    let block_start = body[nontext_idx..]
        .find('{')
        .map(|i| nontext_idx + i)
        .expect("NonText check must be followed by a block");
    let condition = &body[nontext_idx..block_start];
    assert!(
        condition.contains("KeyOrigin::ImeOffRescueReplay"),
        "deliver_key_event の FocusKind::NonText 早期returnは \
         KeyOrigin::ImeOffRescueReplay を対象外にすること（コードレビュー指摘3、\
         さもなくば IME OFF 救済リプレイが focus_kind の一時的誤判定で \
         黙って無効化されうる）。\n条件式: {condition:?}"
    );
}

/// コードレビュー指摘4の回帰テスト: `TIMER_IME_OFF_RESCUE` 分岐が
/// `deliver_key_event` を呼ぶ前に `begin_key_batch(app)` を呼んでいること。
///
/// `begin_key_batch` の doc が定める「バッチ境界で1回だけ resync する」契約の
/// 呼び出し元（`handle_wm_key_from_hook`・`handle_wm_drain_output_queue`）に、
/// この TIMER 分岐が含まれていなかった回帰。
#[test]
fn timer_ime_off_rescue_calls_begin_key_batch_before_deliver_key_event() {
    let content = read_crate_file("src/runtime/message_handlers.rs");
    let production = production_code_only(&content);
    let timer_body = extract_fn_body(production, "pub(crate) unsafe fn handle_wm_timer(");
    let branch_start = timer_body
        .find("crate::TIMER_IME_OFF_RESCUE")
        .expect("handle_wm_timer must handle TIMER_IME_OFF_RESCUE");
    let deliver_idx = timer_body[branch_start..]
        .find("deliver_key_event(app, pending_event, KeyOrigin::ImeOffRescueReplay)")
        .map(|i| branch_start + i)
        .expect("TIMER_IME_OFF_RESCUE branch must call deliver_key_event with ImeOffRescueReplay");
    let begin_idx = timer_body[branch_start..deliver_idx].find("begin_key_batch(app)");
    assert!(
        begin_idx.is_some(),
        "TIMER_IME_OFF_RESCUE 分岐は deliver_key_event 呼び出し前に \
         begin_key_batch(app) を呼ぶこと（コードレビュー指摘4）。"
    );
}

#[test]
fn bootstrap_initial_focus_scope_precedes_ime_cache_initialization() {
    let content = read_crate_file("src/app/bootstrap.rs");
    let run_all = extract_fn_body(&content, "fn run_all");
    let focus_idx = run_all
        .find("Runtime::establish_initial_focus_scope")
        .expect("run_all must establish initial focus scope");
    let ime_idx = run_all
        .find("initialize_ime_cache()")
        .expect("run_all must initialize IME cache");
    assert!(
        focus_idx < ime_idx,
        "startup must establish initial focus scope before initialize_ime_cache"
    );
    assert_eq!(
        run_all
            .matches("Runtime::establish_initial_focus_scope")
            .count(),
        1,
        "startup must establish the initial focus scope exactly once"
    );
}

#[test]
fn establish_initial_focus_scope_advances_focus_epoch_once() {
    let content = read_crate_file("src/runtime/focus_tracking.rs");
    // establish_initial_focus_scope は共通ヘルパー enter_focus_scope 経由で
    // focus_epoch を進める（コードレビュー指摘9で on_focus_process_changed と共通化）。
    let body = extract_fn_body(&content, "fn establish_initial_focus_scope");
    assert_eq!(
        count_real_calls(body, "self.enter_focus_scope("),
        1,
        "establish_initial_focus_scope must call enter_focus_scope exactly once"
    );
    let helper_body = extract_fn_body(&content, "fn enter_focus_scope");
    assert_eq!(
        helper_body.matches("focus_epoch.wrapping_add(1)").count(),
        1,
        "enter_focus_scope must advance focus_epoch exactly once"
    );
}

#[test]
fn establish_initial_focus_scope_does_not_write_ime_belief() {
    // (関数名, 禁止語) の組で例外を明示する。件数と中身は下の専用 assert が縛る。
    const EXEMPT: &[(&str, &str)] = &[
        ("sync_initial_focus_fence", "dispatch_event("),
        // BUG-114 根本原因1（ADR-134 D1c）で追加した app_policy 初期化ヘルパー。
        // `sync_initial_focus_fence` と同じ理由で dispatch_event(` 1件だけ例外化する。
        ("sync_initial_app_policy", "dispatch_event("),
    ];

    let content = read_crate_file("src/runtime/focus_tracking.rs");
    let bodies = [
        (
            "establish_initial_focus_scope",
            extract_fn_body(&content, "fn establish_initial_focus_scope"),
        ),
        (
            "classify_focus_probe",
            extract_fn_body(&content, "fn classify_focus_probe"),
        ),
        (
            "advance_focus_tracking",
            extract_fn_body(&content, "fn advance_focus_tracking"),
        ),
        (
            "apply_app_disable_transition",
            extract_fn_body(&content, "fn apply_app_disable_transition"),
        ),
        // establish_initial_focus_scope が呼ぶ共通ヘルパー（コードレビュー指摘9で
        // on_focus_process_changed と共通化）。ここに処理が移った分、上の直接
        // テキスト検査から漏れないよう対象関数リストへ明示的に加える
        // （検査範囲を広げ忘れるとテストは緑のまま防御が消えるため）。
        (
            "enter_focus_scope",
            extract_fn_body(&content, "fn enter_focus_scope"),
        ),
        // BUG-102 で追加した fence 同期ヘルパー。`dispatch_event(` を1件だけ
        // 持つため下の EXEMPT で除外するが、**残りの禁止語は他と同じく効かせる**
        // ——対象リストへ載せないと、この関数に belief 書き込みを足しても
        // どのテストも落ちない（2026-08-31 敵対的レビュー指摘3-a）。
        (
            "sync_initial_focus_fence",
            extract_fn_body(&content, "fn sync_initial_focus_fence"),
        ),
        // BUG-114 根本原因1（ADR-134 D1c）で追加した app_policy 初期化ヘルパー。
        // 同じ理由（対象リストへ載せないと belief 書き込みを足しても検知
        // できない）で明示的に加える。
        (
            "sync_initial_app_policy",
            extract_fn_body(&content, "fn sync_initial_app_policy"),
        ),
    ];
    for forbidden in [
        "dispatch_event(",
        "apply_hwnd_cache_restore(",
        "record_confirmed(",
        "reset_stale_ime_on_for_imm_broken(",
        "EngineCommand::FocusChanged",
    ] {
        for (name, body) in bodies {
            if EXEMPT.contains(&(name, forbidden)) {
                continue;
            }
            assert!(
                !body.contains(forbidden),
                "establish_initial_focus_scope indirect path `{name}` must not write IME belief via `{forbidden}`"
            );
        }
    }

    // 例外を認めた `sync_initial_focus_fence` の `dispatch_event` は、fence 同期
    // イベントちょうど1件でなければならない（BUG-102）。件数を縛らないと、
    // 2つ目の dispatch（`FocusChanged` 等）をここに足しても既存テストが全て
    // 緑のまま通ってしまう。
    let sync_body = extract_fn_body(&content, "fn sync_initial_focus_fence");
    assert_eq!(
        count_real_calls(sync_body, "dispatch_event("),
        1,
        "sync_initial_focus_fence の dispatch_event はちょうど1件（fence 同期のみ）"
    );
    assert!(
        non_comment_lines(sync_body).contains("ImeEvent::InitialFocusFenceEstablished"),
        "sync_initial_focus_fence の唯一の dispatch は \
         ImeEvent::InitialFocusFenceEstablished であること"
    );

    // BUG-114 根本原因1（ADR-134 D1c）: `sync_initial_app_policy` も同様に
    // dispatch_event ちょうど1件、`InitialAppPolicyEstablished` のみであること。
    let app_policy_sync_body = extract_fn_body(&content, "fn sync_initial_app_policy");
    assert_eq!(
        count_real_calls(app_policy_sync_body, "dispatch_event("),
        1,
        "sync_initial_app_policy の dispatch_event はちょうど1件（app_policy 初期化のみ）"
    );
    assert!(
        non_comment_lines(app_policy_sync_body).contains("ImeEvent::InitialAppPolicyEstablished"),
        "sync_initial_app_policy の唯一の dispatch は \
         ImeEvent::InitialAppPolicyEstablished であること"
    );

    // `establish_initial_focus_scope` は `sync_initial_app_policy` をちょうど1回、
    // かつ `advance_focus_tracking`（`current_app_profile()` が正しい値を返す
    // ようになる箇所）より後に呼ぶこと（ADR-134 D1c の実装位置要件）。
    let bootstrap_body = extract_fn_body(&content, "fn establish_initial_focus_scope");
    assert_eq!(
        count_real_calls(bootstrap_body, "self.sync_initial_app_policy("),
        1,
        "establish_initial_focus_scope は sync_initial_app_policy をちょうど1回呼ぶこと"
    );
    let bootstrap_code = non_comment_lines(bootstrap_body);
    let advance_idx = bootstrap_code
        .find("self.advance_focus_tracking(")
        .expect("establish_initial_focus_scope must call advance_focus_tracking");
    let app_policy_idx = bootstrap_code
        .find("self.sync_initial_app_policy(")
        .expect("establish_initial_focus_scope must call sync_initial_app_policy");
    assert!(
        advance_idx < app_policy_idx,
        "sync_initial_app_policy は advance_focus_tracking の後に呼ぶこと \
         (先に呼ぶと current_app_profile() がまだ正しい値を返さない、ADR-134 D1c)"
    );
}

/// `apply_app_disable_transition` の `invalidate_engine_context`（engine decision の
/// 実行を伴う唯一の副作用）は、bootstrap経路（`establish_initial_focus_scope`、まだ
/// 一度もIMEを観測していない）では呼ばれてはならない。engine生成直後はflushすべき
/// pendingが存在しないため意味を持たない一方、ADR-102決定3-bの「最初のIME観測より
/// 前にbeliefを書き換えない」という構造的保証を、"今は何も起きないはず"という前提
/// ではなく経路自体の遮断で満たすため（Opus敵対的レビュー指摘、2026-08-26）。
///
/// `establish_initial_focus_scope_does_not_write_ime_belief` は関数本体の直接テキスト
/// しか見ないため、`apply_app_disable_transition`が呼ぶ`invalidate_engine_context`
/// （それ自体は`dispatch_event`等の禁止リストに載らない）までは検知できない。
/// このテストはその1段先の呼び出しチェーンを明示的に固定する。
#[test]
fn app_disable_invalidate_engine_context_is_skipped_during_bootstrap() {
    let content = read_crate_file("src/runtime/focus_tracking.rs");

    let apply_body = extract_fn_body(&content, "fn apply_app_disable_transition");
    assert!(
        apply_body.contains("invalidate_engine_context") && apply_body.contains("!is_bootstrap"),
        "apply_app_disable_transition の invalidate_engine_context 呼び出しは \
         `is_bootstrap` でガードされていること（bootstrap時に engine decision を \
         実行させないため）"
    );

    let bootstrap_body = extract_fn_body(&content, "fn establish_initial_focus_scope");
    assert!(
        bootstrap_body.contains("advance_focus_tracking(&classified, true)"),
        "establish_initial_focus_scope は advance_focus_tracking を \
         is_bootstrap=true で呼ぶこと"
    );

    let probe_result_body = extract_fn_body(&content, "fn apply_focus_probe_result");
    assert!(
        probe_result_body.contains("advance_focus_tracking(&classified, false)"),
        "apply_focus_probe_result（定常経路）は advance_focus_tracking を \
         is_bootstrap=false で呼ぶこと"
    );
}

/// `establish_initial_focus_scope_does_not_write_ime_belief` は関数本体の直接テキスト
/// しか見ないため、`advance_focus_tracking` が呼ぶ
/// `notify_focus_hwnd_updated_if_needed`（それ自体は `dispatch_event` 等の禁止
/// リストに載らない）までは検知できない。このテストはその1段先の呼び出しチェーン
/// を明示的に固定する（ADR-106 決定3、PR 109 コードレビュー是正）。
#[test]
fn focus_hwnd_updated_dispatch_is_skipped_during_bootstrap() {
    let content = read_crate_file("src/runtime/focus_tracking.rs");

    let advance_body = extract_fn_body(&content, "fn advance_focus_tracking");
    assert!(
        advance_body.contains("notify_focus_hwnd_updated_if_needed"),
        "advance_focus_tracking は notify_focus_hwnd_updated_if_needed 経由で \
         FocusHwndUpdated を dispatch すること"
    );

    let notify_body = extract_fn_body(&content, "fn notify_focus_hwnd_updated_if_needed");
    assert!(
        notify_body.contains("dispatch_event(") && notify_body.contains("if is_bootstrap"),
        "notify_focus_hwnd_updated_if_needed の dispatch_event 呼び出しは \
         `is_bootstrap` でガードされていること（bootstrap時は ObservationStore の \
         fence がまだ FocusChanged で初期化されておらず、belief 層へ書き込むと \
         establish_initial_focus_scope の不変条件を破るため）"
    );
}

/// BUG-102: bootstrap の `establish_initial_focus_scope` は、live 側フェンス
/// （`Runtime::focus_fence()` = `enter_focus_scope` 後の epoch + `update_focus_info`
/// 後の hwnd）を `ObservationStore::current_fence` へ同期しなければならない。
///
/// 同期が無いと、起動時にフォーカスされていたアプリで発生する `ImmCrossProbe`
/// 観測（High / `ActuatingPool`）が `derive_filtered` の `is_identity_ok` で
/// stale 扱いされ、ユーザーが別プロセスへ切り替えて戻る（= `FocusChanged`）まで
/// 恒久的に導出から外れ続ける。
///
/// 呼び出し順序も固定する。`sync_initial_focus_fence` が読む `focus_fence()` の
/// 2 軸は別々の場所で確定するため、**両方の後**でなければならない:
/// epoch は `enter_focus_scope`、hwnd は `advance_focus_tracking`
/// （→ `update_focus_info`）。どちらか一方でも前に置くと、確定前の古い値を
/// fence として焼き付ける。
#[test]
fn establish_initial_focus_scope_syncs_the_observation_fence() {
    let content = read_crate_file("src/runtime/focus_tracking.rs");
    let body = extract_fn_body(&content, "fn establish_initial_focus_scope");
    assert_eq!(
        count_real_calls(body, "self.sync_initial_focus_fence("),
        1,
        "establish_initial_focus_scope は sync_initial_focus_fence をちょうど1回呼ぶこと \
         (BUG-102: ObservationStore 側の fence が既定値のまま残ると、起動直後の \
         アプリの高信頼観測が次のプロセス変更まで導出から外れ続ける)"
    );
    // 順序判定もコメントを落としたテキストに対して行う（doc コメント中の関数名
    // 言及が `find` に先に当たると偽陽性/偽陰性になるため、件数カウント側の
    // `count_real_calls` と揃える）。
    let body_code = non_comment_lines(body);
    let idx = |needle: &str| {
        body_code
            .find(needle)
            .unwrap_or_else(|| panic!("establish_initial_focus_scope must call `{needle}`"))
    };
    let advance_idx = idx("self.advance_focus_tracking(");
    let enter_idx = idx("self.enter_focus_scope(");
    let sync_idx = idx("self.sync_initial_focus_fence(");
    assert!(
        enter_idx < sync_idx,
        "sync_initial_focus_fence は enter_focus_scope の後に呼ぶこと \
         (先に呼ぶと epoch インクリメント前の古い fence を焼き付ける)"
    );
    assert!(
        advance_idx < sync_idx,
        "sync_initial_focus_fence は advance_focus_tracking の後に呼ぶこと \
         (先に呼ぶと update_focus_info 前の hwnd=NULL を fence に焼き付ける)"
    );
}

/// BUG-102: `ImeEvent::InitialFocusFenceEstablished` は bootstrap 専用であり、
/// dispatch 元は `sync_initial_focus_fence` の1箇所だけ。reducer 側のアームは
/// `ObservationStore::establish_initial_fence()`（fence 1フィールドの差し替え）
/// しか行わない。
///
/// このイベントは「まだ一度も IME を観測していない時点で dispatch される」という、
/// 他のどのイベントも持たない性質を持つ（ADR-102 決定3-b）。belief を書く処理が
/// このアームや新しい呼び出し元に紛れ込むと、その不変条件が静かに壊れる。
/// アーム本体が belief に触れないことは
/// `state::ime_model::tests::initial_focus_fence_established_touches_only_the_fence`
/// が実行時に固定し、ここでは「増えていないこと」だけを見る。
#[test]
fn initial_focus_fence_event_only_touches_the_fence() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    // (needle, [(相対パス, 期待マッチ数)])。列挙されないファイルは 0 でなければ
    // ならない。**どちらの needle も固定ファイルへの grep ではなく全ファイル走査に
    // 乗せる** ——固定リストへの grep は「新しいファイルに呼び出しが追加された」
    // パターンを検知できない（本ファイル冒頭 `list_src_files` の doc 参照）。
    let checks: &[(&str, &[(&str, usize)])] = &[
        (
            "InitialFocusFenceEstablished",
            &[
                // sync_initial_focus_fence（bootstrap 専用の唯一の dispatch 元）。
                ("runtime/focus_tracking.rs", 1),
                // reducer のアーム。
                ("state/ime_model.rs", 1),
                // variant 定義そのもの。
                ("state/ime_event.rs", 1),
            ],
        ),
        (
            // reducer のアームが fence の差し替え以外をしていないこと（呼び先の限定）。
            // 先頭のドットにより `pub fn establish_initial_fence(`（定義）は数えない。
            ".establish_initial_fence(",
            &[("state/ime_model.rs", 1)],
        ),
    ];
    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(path).unwrap();
        // doc コメントでこのイベント名に言及しているファイル（`probe_admission.rs` の
        // `FocusFence` 説明等）を数えないよう、コメント行を落としてから数える。
        let production = non_comment_lines(production_code_only(&content));
        for (needle, expected) in checks {
            let count = production.matches(needle).count();
            let expected_count = expected
                .iter()
                .find(|(f, _)| *f == rel)
                .map_or(0, |(_, n)| *n);
            assert_eq!(
                count, expected_count,
                "src/{rel} 内の `{needle}` の出現数が想定\
                 ({expected_count})と異なります(実際: {count})。\n\
                 このイベントは bootstrap（最初の IME 観測より前）でのみ dispatch される\
                 専用イベントです。新しい呼び出し元を足す前に、それが本当に「起動時の\
                 初回フォーカススコープ確立」なのかを確認してください（ADR-102 決定3-b）。"
            );
        }
    }
}

/// BUG-114 根本原因1（ADR-134 D1c）: `ImeEvent::InitialAppPolicyEstablished` は
/// bootstrap 専用であり、dispatch 元は `sync_initial_app_policy` の1箇所だけ。
/// reducer 側のアームは `self.app_policy = AppImePolicy::from_profile(profile)`
/// （app_policy 1フィールドの差し替え）しか行わない。
///
/// `initial_focus_fence_event_only_touches_the_fence` と同じ構造の監視テスト。
/// アーム本体が app_policy 以外に触れないことは
/// `state::ime_model::tests::initial_app_policy_established_touches_only_app_policy`
/// が実行時に固定し、ここでは「増えていないこと」だけを見る。
#[test]
fn initial_app_policy_event_only_touches_app_policy() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    let checks: &[(&str, &[(&str, usize)])] = &[(
        "InitialAppPolicyEstablished",
        &[
            ("runtime/focus_tracking.rs", 1),
            ("state/ime_model.rs", 1),
            ("state/ime_event.rs", 1),
        ],
    )];
    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(path).unwrap();
        let production = non_comment_lines(production_code_only(&content));
        for (needle, expected) in checks {
            let count = production.matches(needle).count();
            let expected_count = expected
                .iter()
                .find(|(f, _)| *f == rel)
                .map_or(0, |(_, n)| *n);
            assert_eq!(
                count, expected_count,
                "src/{rel} 内の {needle} の出現数が想定と異なります(期待: \
                 {expected_count}, 実際: {count})。ADR-134 D1c 参照。"
            );
        }
    }
}

// ── ADR-103 決定4: probe 段の唯一の出口 ────────────────────────────────────

/// `dispatch_probe_actions` の本体から `return DispatchResult` を1件残らず消す
/// （ADR-103 決定4-b）。段が終わる出口は `break 'stage <StageEndReason>` という
/// 形でしか書けないようにし、「呼び忘れられる出口」を型検査で強制する。この guard
/// は grep による第二の防衛線であり、将来 `return DispatchResult` を書く新しい
/// 早期脱出が追加されたことを機械的に検知する。
#[test]
fn dispatch_probe_actions_has_no_early_return() {
    let path = "src/output/probe_io.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "fn dispatch_probe_actions");
    let count = body.matches("return DispatchResult").count();
    assert_eq!(
        count, 0,
        "{path} の dispatch_probe_actions 本体に `return DispatchResult` が \
         {count} 件見つかりました。段の終わりは `break 'stage <理由>` でのみ表現し、\
         早期 return を書かないでください（ADR-103 決定4-b）。"
    );
}

/// `note_stage_recovery` を呼ぶのは `mark_cold_raw_tsf` の本番実装ただ1箇所
/// （ADR-103 決定4-d）。`mark_cold_raw_tsf` は `RawTsfLiteralRecovery` アームの
/// 全分岐で無条件に呼ばれるため、「composition を cold にマークした段は warm を
/// 主張できない」という規則が呼び忘れようのない形で成立する。dispatcher 側に
/// 同じ呼び出しを書くと、忘れたときに危険側（warm 誤申告）へ倒れるため書かない。
#[test]
fn note_stage_recovery_is_called_only_from_mark_cold_raw_tsf() {
    let path = "src/output/probe_io.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production
        .matches(".warmup_coord.note_stage_recovery()")
        .count();
    assert_eq!(
        count, 1,
        "{path} 内で `note_stage_recovery` の呼び出し箇所数が想定(1 = \
         mark_cold_raw_tsf のみ)と異なります(実際: {count})。"
    );
}

/// `note_stage_injection` を呼ぶのは `impl ProbeIo for Output` の注入メソッド4つ
/// （`transmit_tsf`/`transmit_chrome`/`send_single_tsf_vk`/`send_single_chrome_vk`）
/// だけ（ADR-103 決定4-d）。dispatcher からは1行も呼ばない——「実際に注入したか」
/// は注入した関数自身が記録することで、呼び忘れを構造的に防ぐ。
#[test]
fn note_stage_injection_is_called_only_from_the_four_injection_methods() {
    let path = "src/output/probe_io.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production
        .matches(".warmup_coord.note_stage_injection()")
        .count();
    assert_eq!(
        count, 4,
        "{path} 内で `note_stage_injection` の呼び出し箇所数が想定(4 = \
         transmit_tsf/transmit_chrome/send_single_tsf_vk/send_single_chrome_vk)と \
         異なります(実際: {count})。dispatch_probe_actions からは呼ばないこと。"
    );
}

// ── ADR-103 決定5: ProbeParams は ColdKind の純関数（INV-C）────────────────

/// `ProbeParams { .. }` のリテラル構築は `ColdKind::probe_params` の中だけ
/// （ADR-103 決定5-b、INV-C）。`EndComposition` 等が固定値で再構築する退行を防ぐ。
#[test]
fn probe_params_construction_is_limited_to_cold_kind_probe_params() {
    let path = "src/tsf/gji_fsm.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    // "ProbeParams {" は構造体定義(`struct ProbeParams {`)と関数シグネチャの
    // 戻り値直後の開き波括弧(`-> ProbeParams {`)にも偶然一致するため両方除く。
    let false_positives = production.matches("struct ProbeParams {").count()
        + production.matches("-> ProbeParams {").count();
    let total = production.matches("ProbeParams {").count();
    let construction_count = total - false_positives;
    assert_eq!(
        construction_count, 1,
        "{path} 内で `ProbeParams {{ .. }}` のリテラル構築箇所数が想定(1 = \
         ColdKind::probe_params の中だけ)と異なります(実際: {construction_count})。\
         ProbeParams は ColdKind の純関数として一元化されている(INV-C)。"
    );
}

/// `GjiAction::DiscardPending { .. }` のリテラル構築は `discard_pending_action`
/// の中だけ（ADR-103 決定5-a）。他の箇所で直接構築すると、`count`/`reason` の
/// 対応関係（破棄点の完全な一覧、5-a）を経由せずに任意の値で emit できてしまい、
/// 「破棄を明示的な行為にする」という決定の前提が崩れる。
#[test]
fn discard_pending_construction_is_limited_to_discard_pending_action() {
    let path = "src/tsf/gji_fsm.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("GjiAction::DiscardPending {").count();
    assert_eq!(
        count, 1,
        "{path} 内で `GjiAction::DiscardPending {{ .. }}` のリテラル構築箇所数が \
         想定(1 = discard_pending_action の中だけ)と異なります(実際: {count})。"
    );
}

/// `raw_recovery_owns_deferred` の呼び出し箇所は `finish_probe_stage`
/// （ADR-103 決定4-e、INV-F: 段末の deferred 解放判断）と
/// `defer_if_probe_in_flight`（ADR-123 変更A: 新規モーラを defer すべきか
/// の判断、report_id `01M1KEGZ081YHJ1T2NC765SYYH`）の2箇所に限定する。
/// 前者は「pending_deferred を今 flush してよいか」、後者は「新しい入力を
/// pending_deferred に積むべきか」という別の問いに答えており、いずれも
/// raw recovery が deferred キューの所有権を握っている間は手を出さない、
/// という同じ原則の異なる適用箇所である。3箇所目が増えた場合は、本当に
/// 同じ原則の適用か（さもなくば別の状態表現を検討すべきでないか）を確認
/// すること。
#[test]
fn raw_recovery_owns_deferred_call_sites_are_accounted_for() {
    let path = "src/output/mod.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production
        .matches("self.raw_recovery_owns_deferred()")
        .count();
    assert_eq!(
        count, 2,
        "{path} 内で `raw_recovery_owns_deferred` の呼び出し箇所数が想定(2 = \
         finish_probe_stage + defer_if_probe_in_flight)と異なります(実際: {count})。"
    );
}

/// `deliver_key_event` 内の早期return順序を固定する（ADR-114 決定2 r4収束版）。
///
/// 「latch チェック（KeyUp 解放 + KeyDown repeat 抑制）→ `PumpContext::Nested`
/// 早期return → `FocusKind::NonText` パススルー → `[[keymap]]` KeyDown 新規照合
/// （`active_keymaps.find_match`）→ `[[post_bypass]]` 消費」の順序がソース上の
/// 出現順そのものであることを固定する。この順序を崩すと、latch が残った vk の
/// KeyDown/KeyUp が Nested/NonText 早期returnで素通りしてしまう構造的リーク
/// （BUG-100 の経路1・2 と同型）が復活する。`deliver_key_event` 自体は
/// Windows 依存（`FocusKind`/`SendInput`）が強く実行時テストが難しいため、
/// ソーステキスト走査で順序を固定する（`architecture_guard.rs` の他のテストと
/// 同じ手法）。
#[test]
fn deliver_key_event_keymap_latch_check_precedes_nested_and_nontext_early_returns() {
    let content = read_crate_file("src/runtime/message_handlers.rs");
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "pub(crate) fn deliver_key_event(");

    // `keymap_latch` を検索キーにする（`.is_latched(...)` は cargo fmt で
    // メソッドチェーンが複数行に折り返されうるため、改行を跨がない単一
    // identifier で検索する）。
    let activity_idx = body
        .find("last_hook_activity_ms")
        .expect("deliver_key_event must update last_hook_activity_ms");
    let latch_idx = body
        .find("keymap_latch")
        .expect("deliver_key_event must check keymap_latch");
    assert!(
        activity_idx < latch_idx,
        "keymap_latch のチェックは last_hook_activity_ms の更新より後に \
         置くこと（ADR-114 実装レビュー MA-A）。latch チェックを \
         last_hook_activity_ms 更新より前に置くと、latch 中のキー（＝実際に \
         ユーザーが打鍵中）が hook activity として記録されず、\
         runtime/ime_refresh.rs の keyboard idle 判定（GJI/Chrome long-idle \
         分岐の起点）が誤って idle に倒れる。"
    );
    let nested_idx = body
        .find("KeyOrigin::Hook(PumpContext::Nested)")
        .expect("deliver_key_event must check KeyOrigin::Hook(PumpContext::Nested)");
    let nontext_idx = body
        .find("FocusKind::NonText")
        .expect("deliver_key_event must check FocusKind::NonText");
    // `find_match`/`active_keymaps` の呼び出し自体は cognitive complexity 対策で
    // `consume_keymap_match` ヘルパーへ切り出されている（`cancel_composition_
    // and_arm_post_bypass_on_ctrl` と同じパターン）。`deliver_key_event` の本体
    // には呼び出し箇所（`consume_keymap_match(app, event)`）だけが残る。
    let find_match_idx = body
        .find("consume_keymap_match(app, event)")
        .expect("deliver_key_event must call consume_keymap_match");
    let post_bypass_idx = body
        .find("consume_post_bypass(app, event, is_key_down)")
        .expect("deliver_key_event must call consume_post_bypass");

    assert!(
        latch_idx < nested_idx,
        "keymap_latch のチェックは PumpContext::Nested 早期returnより前に \
         置くこと（ADR-114 決定2）。さもないと latch 中の vk が Nested \
         早期returnで素通りし latch が解放されない。"
    );
    assert!(
        latch_idx < nontext_idx,
        "keymap_latch のチェックは FocusKind::NonText 早期returnより前に \
         置くこと（ADR-114 決定2）。さもないと latch 中の vk が NonText \
         早期returnで素通りし latch が解放されない。"
    );
    assert!(
        nontext_idx < find_match_idx,
        "[[keymap]] の新規照合（find_match）は FocusKind::NonText 早期returnの \
         後に置くこと（ADR-114 決定2、v1 スコープでは NonText では効かない \
         既知の限界）。"
    );
    assert!(
        find_match_idx < post_bypass_idx,
        "[[keymap]] の新規照合は [[post_bypass]] 消費より前に置くこと \
         （ADR-114 決定2、[[keymap]] が NICOLA エンジンに一切見せないため）。"
    );
}

// ── PR #127: プラットフォームエントリポイントの配線漏れ ──────────────

/// `NicolaFsm::new` はコンストラクタ引数を持つが、`timing_margin_percent`/
/// `min_overlap_margin_percent`（`GeneralConfig` 由来のユーザー設定値）は
/// コンストラクタ直後の `apply_general_config` 呼び出しで別途反映する設計に
/// なっている（`src/engine/nicola_fsm.rs` のフィールド doc 参照）。コンパイラは
/// この呼び出し漏れを検知できない——`NicolaFsm::new` はコンストラクタ既定値
/// だけで黙って動き続ける。
///
/// PR #127（`feat/confirm-mode-simplify`）のコードレビューで、この呼び出しが
/// `awase-linux`/`awase-macos` の両方で実際に漏れていたことが発覚した
/// （config.toml で値を設定してもFSMに一切反映されず無反応だった）。3プラット
/// フォームそれぞれに個別の `set_timing_margins` 呼び出しをコピペしていたのが
/// 原因の一つだったため、`NicolaFsm::apply_general_config`/
/// `Engine::apply_general_config` へ一本化した（同コードレビュー7回目）。
/// 同種の見落としを新しいプラットフォームエントリポイントが追加された際にも
/// 機械的に検知する第二の防衛線として、このガードテストを維持する。
///
/// `apply_general_config` の呼び出しが `NicolaFsm::new` より**後**（テキスト
/// 上のオフセットが大きい）にあることまで確認する（同7回目指摘: 単純な
/// 部分文字列の有無だけだと、無関係な別インスタンスへの呼び出しやコメント中の
/// 言及でも素通りしてしまう）。
///
/// 相対パスは `crates/awase-windows`（このクレートの `CARGO_MANIFEST_DIR`）
/// 基準。`awase-linux`/`awase-macos` は兄弟クレートのため `../` で辿る。
#[test]
fn every_platform_entry_point_calls_apply_general_config_after_nicola_fsm_new() {
    const PLATFORM_ENTRY_POINTS: &[&str] = &[
        "src/app/bootstrap.rs",
        "../awase-linux/src/main.rs",
        "../awase-macos/src/main.rs",
    ];
    for path in PLATFORM_ENTRY_POINTS {
        let content = read_crate_file(path);
        // /code-review指摘（PR #127、8回目）: 単純な部分文字列一致だと、
        // NicolaFsm::new より後にあるコメント（例: 削除済みの呼び出しに
        // 言及するTODOや過去のレビュー指摘コメント）が偶然
        // `.apply_general_config(` を含むだけで素通りしてしまう。
        // 他のガードテストと同じく `//` 行コメントを除いた本文だけを見る。
        let production = non_comment_lines(production_code_only(&content));
        let Some(construct_pos) = production.find("NicolaFsm::new(") else {
            continue;
        };
        let wires_margins_after = production
            .find(".apply_general_config(")
            .is_some_and(|pos| pos > construct_pos);
        assert!(
            wires_margins_after,
            "{path} は NicolaFsm::new(...) を呼んでいるが、その後に \
             apply_general_config(...) を呼んでいない（見つからない、または \
             construct より前にしかない）。GeneralConfig の \
             timing_margin_percent/min_overlap_margin_percent（config.toml 由来の \
             ユーザー設定値）がこのプラットフォームでは無反応になる \
             （PR #127 コードレビュー: awase-linux/awase-macos の両方で\
             実際に起きた見落とし）。"
        );
        // /code-review指摘（PR #127、9回目）: 上の位置チェックは「最初の
        // NicolaFsm::new より後にapply_general_configが1回でもあるか」
        // しか見ておらず、同一ファイルに将来2つ目の独立した構築箇所（例:
        // 診断用の別経路）が追加され、そちらだけ配線を忘れても検知できない。
        // 構築回数と配線回数が一致することも合わせて確認する（完全な
        // 「どの構築がどの配線に対応するか」までは検証しないが、本ファイルの
        // 他のガードテストと同じ粒度のヒューリスティックとしては十分）。
        let construct_count = production.matches("NicolaFsm::new(").count();
        let wire_count = production.matches(".apply_general_config(").count();
        assert_eq!(
            construct_count, wire_count,
            "{path}: NicolaFsm::new(...) の出現回数({construct_count})と \
             apply_general_config(...) の出現回数({wire_count})が一致しません。\
             同一ファイル内に複数の構築箇所がある場合、そのうちどれかが \
             apply_general_config を呼び忘れている可能性があります。"
        );
    }
}

// ── issue #136 / BUG-90 決定4: AppImeProfile::InputRelay の配線を固定 ─────────

/// `AppImeProfile::InputRelay`（PowerToys Mouse Without Borders 等の入力中継
/// ツール向け、issue #136 / BUG-90 決定4）の本番コード中の出現数をファイルごとに
/// 固定する。
///
/// この variant は「唯一の構築箇所」を強制する PanicReset 型の規約ではなく、
/// 「消費箇所（述語・分岐）が今後の変更で静かに減らないこと」を守るための
/// スナップショットガード（`.claude/rules/ime-belief-architecture.md` の
/// 判断基準(c)）。件数が変わった場合、それが意図した変更（新しい消費箇所の追加等）
/// なら定数を更新すればよい。意図せず減っていた場合は、`can_use_imm32_cross_process`
/// / `uses_kanji_toggle` / `should_pass_physical_key` / `can_read_imm32_open_status`
/// の4述語、`is_effectively_tsf_native` / `cannot_verify_real_ime_state` /
/// `should_reprime_on_lightweight_focus_sync` の3自由関数、`From<AppImeProfile>
/// for ImePolicyProfile`、`from_class_and_process` のいずれかで `InputRelay` の
/// 分岐が欠落していないか確認すること（欠落すると condition (b)/(c) が
/// 別経路から迂回されうる、査読で指摘された最重要ポイント）。
/// `production_code_only` は `#[cfg(test)]` の直後が文字どおり `mod tests` の
/// ときしか test module を切り落とせない。`runtime/transport.rs` は
/// `mod plan_tests` という別名を使っているため、共有ヘルパーのままだと
/// テストコード中の `InputRelay` 出現（回帰テストの引数等）まで「本番コード」
/// として誤カウントする（レビュー指摘）。ここでは `#[cfg(test)]` の直後に
/// 続く `mod <任意の識別子> {` を汎用的に検出して切り落とす、より厳密な版を
/// 本テスト専用に使う。
fn strip_any_test_module(content: &str) -> &str {
    const MARKER: &str = "#[cfg(test)]";
    let mut from = 0;
    while let Some(rel) = content[from..].find(MARKER) {
        let idx = from + rel;
        let rest = content[idx + MARKER.len()..].trim_start();
        if rest.starts_with("mod ") {
            return &content[..idx];
        }
        from = idx + MARKER.len();
    }
    content
}

#[test]
fn input_relay_profile_wiring_occurrence_counts_are_pinned() {
    let expectations: &[(&str, usize)] = &[
        ("src/focus/class_names.rs", 12),
        ("src/runtime/transport.rs", 4),
        ("src/runtime/executor.rs", 1),
        // `executor.rs` の gate は早期 exit の最適化（重複するが無害）。
        // condition (a) を実際に担保しているのはこちらの2ファイル
        // （`ImeController::apply` / `run_open_chain_async` /
        // `fallback_write` の3箇所、コードレビューで gate 取りこぼしが
        // 見つかった経緯は ADR-119 参照）。ここが欠けると、今回踏んだのと
        // 同じクラスの退行（gate の一部消失）を検知できない。
        ("src/ime_controller.rs", 2),
        ("src/runtime/open_chain.rs", 7),
    ];
    for (path, expected) in expectations {
        let content = read_crate_file(path);
        let production = strip_any_test_module(&content);
        let count = production.matches("InputRelay").count();
        assert_eq!(
            count, *expected,
            "{path} 内で `AppImeProfile::InputRelay`（`InputRelay` を含む識別子）の \
             本番コードでの出現数が想定({expected})と異なります(実際: {count})。\
             issue #136 / BUG-90 決定4の配線箇所が増減していないか確認すること。\
             意図した変更ならこのテストの期待値を更新すること。"
        );
    }
}

/// `DeferredOrigin::RecoveryResend` の本番構築箇所は
/// `DeferGate::deferred_origin`（`src/output/vk_send.rs`）1箇所に限定する。
///
/// ADR-123 変更A+C 決定4-2（gate免除入口）が実装され、`RecoveryResend` が
/// 初めて本番コードから構築されるようになった。構築箇所が増えた場合、
/// `discard_raw_recovery_if_focus_stale`等の破棄経路が想定と異なる由来の
/// VKを巻き込んでいないか確認すること。
#[test]
fn deferred_origin_recovery_resend_construction_is_limited_to_gate_bypass() {
    let known_sites: &[(&str, usize)] = &[
        ("src/output/mod.rs", 0),
        ("src/output/tsf_warmup_coord.rs", 0),
        // DeferGate::deferred_origin の1箇所のみ。
        ("src/output/vk_send.rs", 1),
    ];
    for (path, expected) in known_sites {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = production.matches("DeferredOrigin::RecoveryResend").count();
        assert_eq!(
            count, *expected,
            "{path} 内で `DeferredOrigin::RecoveryResend` の本番コードでの構築箇所数が \
             想定({expected})と異なります(実際: {count})。意図した変更ならこのテストの \
             期待値を更新してください。"
        );
    }
}

// ── BUG-110/ADR-132 Phase 2: WarmupImeOn の gate 迂回防止 ──────────────

/// `manifest_dir` 相対の `rel_root` 以下の `.rs` ファイルを再帰的に列挙し、
/// `read_crate_file` が使える形（`CARGO_MANIFEST_DIR` 相対パス文字列）で返す。
///
/// `WarmupImeOn::from_applied_or_belief` は core クレート（`../../src/`、
/// `crates/awase-windows` から見て2階層上）の `pub const fn` であり、
/// `list_src_files()`（`awase-windows` 自身の `src/` のみ走査）では取りこぼす。
/// 兄弟クレート `awase-linux`/`awase-macos` も含めて漏れなく走査する
/// （PR #127 のコードレビューで実際に見落とされた前例、上記
/// `every_platform_entry_point_calls_apply_general_config_after_nicola_fsm_new`
/// と同じ理由）。
fn list_rs_files_under(rel_root: &str) -> Vec<String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest_dir).join(rel_root);
    let mut files = Vec::new();
    walk_rs_files(&root, &mut files);
    files
        .iter()
        .map(|path| {
            path.strip_prefix(manifest_dir)
                .unwrap_or_else(|e| panic!("strip_prefix: {e}"))
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

/// `WarmupImeOn::from_applied_or_belief`（`applied` が既知の間は belief に
/// フォールバックしない、という単調性を持つ生のコンストラクタ）の本番コード
/// からの実呼び出しは、BUG-110/ADR-132 Phase 2 のゲート版
/// `from_applied_or_belief_unless_off_drift` の内部1箇所に限定する。
///
/// このゲートを迂回して `from_applied_or_belief` を直接呼ぶ新しい呼び出し元が
/// 増えると、OFF 方向 drift correction と逆向きに warmup（`VK_IME_ON`）を
/// 送る経路が黙って復活する（BUG-110 で実際に競合した経路そのもの）。
/// `needle` の末尾に `(` を含めることで、4つ目のコンストラクタ
/// `from_applied_or_belief_unless_off_drift(` を誤ってカウントしない
/// （`from_applied_or_belief` は後者の**接頭辞**だが、直後に続く文字が
/// `_unless_off_drift` であって `(` ではないため区別できる）。
///
/// # このガードが対象としない既知の別経路（敵対的コードレビュー指摘）
///
/// `WarmupImeOn::from_actuated`（`platform.rs::on_ime_applied` — 実
/// actuation 直後の随伴 warmup）はこのガードの対象外で、`off_drift_active`
/// ゲートを通らない。これは4つ目のコンストラクタとは別の、意図的な
/// 未ゲート経路（ADR-132「Phase 2」節「実装上の既知の限界」参照）であり、
/// `send_eager_tsf_warmup` へ渡す `origin` 引数（`"gated"`/`"actuated"`）で
/// ログ上区別する対応を別途行った。
#[test]
fn warmup_ime_on_from_applied_or_belief_is_called_only_from_the_gated_constructor() {
    let mut files = list_rs_files_under("../../src");
    files.extend(list_src_files());
    files.extend(list_rs_files_under("../awase-linux/src"));
    files.extend(list_rs_files_under("../awase-macos/src"));

    let mut total = 0usize;
    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "from_applied_or_belief(");
        if count > 0 {
            total += count;
            hits.push(format!("{path} ({count})"));
        }
    }
    assert_eq!(
        total, 1,
        "`WarmupImeOn::from_applied_or_belief(` の本番コードでの実呼び出しが \
         1箇所（`from_applied_or_belief_unless_off_drift` 内部）以外に \
         見つかりました: {hits:?}。BUG-110/ADR-132 Phase 2 のゲートを \
         迂回する新しい呼び出し元が追加されていないか確認してください。"
    );
}

/// `needle` の実呼び出し（`fn {name}(` という定義行、および行コメント
/// `//`/`///`/`//!` は除外——`non_comment_lines` を内部で適用する。ADR等の
/// doc コメントに引用されたコード片や `#[cfg(test)]` 内の正当なリテラル
/// 使用が「本番コードの迂回」として誤検知されるのを防ぐ、敵対的コード
/// レビュー N3 指摘）ごとに、対応する閉じ括弧までの引数リスト文字列
/// （深さカウントを尊重した「トップレベル」の `,` で分割、トリム済み）を
/// 返す。文字列リテラル内の括弧・カンマは非対応（本テストが対象とする
/// 呼び出しはいずれも識別子/真偽値リテラルのみの単純な引数なので十分）。
/// 対応する閉じ括弧が見つからない場合（構文的に不完全な部分一致等）は
/// その出現を明示的にスキップする（N4 指摘: 見つからない場合に
/// `args_start` を終端扱いすると、続く走査が非 UTF-8 境界で panic しうる
/// バグがあった）。
fn extract_call_arg_lists(content: &str, needle: &str) -> Vec<Vec<String>> {
    let content = non_comment_lines(content);
    let content = content.as_str();
    let fn_name = needle.trim_end_matches('(');
    let def_needle = format!("fn {fn_name}(");
    let mut results = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = content[search_from..].find(needle) {
        let call_start = search_from + rel;
        let args_start = call_start + needle.len();
        let is_definition = content[..call_start]
            .rfind('\n')
            .map(|i| i + 1)
            .is_some_and(|line_start| content[line_start..args_start].contains(&def_needle));
        if is_definition {
            search_from = args_start;
            continue;
        }
        let mut depth: i32 = 1;
        let mut args_end = None;
        for (offset, ch) in content[args_start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        args_end = Some(args_start + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(args_end) = args_end else {
            // 対応する閉じ括弧が見つからなかった（構文的に不完全）。
            // これ以上この occurrence を安全に解釈できないため、次の
            // needle 検索へ進む（args_start から1バイトだけ進めると
            // マルチバイト文字の途中を指す可能性があるため、次の
            // needle 出現へ直接ジャンプする）。
            search_from = args_start;
            continue;
        };
        let args_text = &content[args_start..args_end];
        // トップレベルの `,` でのみ分割する（N5 指摘: 深さカウントを
        // 終端検出だけでなく分割自体にも使う。ネストした関数呼び出しや
        // クロージャの引数内カンマで誤分割しない）。
        let mut args: Vec<String> = Vec::new();
        let mut arg_depth: i32 = 0;
        let mut current_start = 0usize;
        for (offset, ch) in args_text.char_indices() {
            match ch {
                '(' | '[' | '{' => arg_depth += 1,
                ')' | ']' | '}' => arg_depth -= 1,
                ',' if arg_depth == 0 => {
                    args.push(args_text[current_start..offset].trim().to_owned());
                    current_start = offset + 1;
                }
                _ => {}
            }
        }
        let tail = args_text[current_start..].trim();
        if !tail.is_empty() {
            args.push(tail.to_owned());
        }
        results.push(args);
        search_from = args_end + 1;
    }
    results
}

/// `WarmupImeOn::from_applied_or_belief_unless_off_drift` の第3引数
/// （`off_drift_active`）に本番コードがリテラル `true`/`false` を直書きして
/// いないことを確認する（敵対的コードレビュー指摘）。
///
/// 上の `warmup_ime_on_from_applied_or_belief_is_called_only_from_the_gated_constructor`
/// は `from_applied_or_belief(` の呼び出し件数をゲート版内部の1箇所に固定するが、
/// その1箇所自体が迂回された場合——新しい呼び出し元がゲート版へ第3引数として
/// リテラル `false` を直書きし、実際には `check_drift_correction` 由来の判定を
/// 一切行わない——は検出できない。これが「ゲートを迂回する新しい呼び出し元を
/// 防ぐ」という宣言目的に対して最も安直な迂回手段であるため、引数が bool 型
/// リテラルでないことを機械的に確認する。`#[cfg(test)]` 側（`src/platform.rs`
/// の12通り全数テスト）はリテラルを渡すのが正しい用途なので対象外とする。
#[test]
fn warmup_gate_third_arg_is_never_a_bare_literal_in_production_code() {
    let mut files = list_rs_files_under("../../src");
    files.extend(list_src_files());
    files.extend(list_rs_files_under("../awase-linux/src"));
    files.extend(list_rs_files_under("../awase-macos/src"));

    let mut violations: Vec<String> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        for args in extract_call_arg_lists(production, "from_applied_or_belief_unless_off_drift(") {
            let Some(third) = args.get(2) else {
                violations.push(format!(
                    "{path}: from_applied_or_belief_unless_off_drift の引数が \
                     3個未満 ({args:?})"
                ));
                continue;
            };
            if third == "true" || third == "false" {
                violations.push(format!(
                    "{path}: 第3引数(off_drift_active)にリテラル `{third}` を \
                     直書き（引数全体: {args:?}）"
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "`from_applied_or_belief_unless_off_drift` の第3引数にリテラル \
         `true`/`false` を直書きする呼び出しが本番コードに見つかりました: \
         {violations:?}。BUG-110/ADR-132 Phase 2 のゲートを実質的に無効化 \
         （常に開く/常に閉じる）する迂回であり、`check_drift_correction` \
         由来の判定変数を渡すべきです。"
    );
}
