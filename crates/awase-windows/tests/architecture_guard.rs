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

fn read_workspace_file(rel_path: &str) -> String {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("awase-windows must be under <workspace>/crates");
    fs::read_to_string(workspace.join(rel_path))
        .unwrap_or_else(|e| panic!("failed to read workspace {rel_path}: {e}"))
        .replace("\r\n", "\n")
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
/// `ime_open_actuation_entry_points_are_accounted_for`）がそれぞれ
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

/// ADR-129 (a-1): `hook::thumb_down_timestamps()`（親指ダウンタイムスタンプの
/// ライブクエリ）の呼び出し許可箇所を固定する。`key_pipeline.rs` は
/// capture 時点のスナップショット（`event.left_thumb_down_snapshot` /
/// `event.right_thumb_down_snapshot`）を読むだけで、ライブクエリを呼んでは
/// ならない——呼ぶと drain replay 中に「replay を実行している"今"」の値を
/// 誤って読み、無関係な親指押下と誤ってペアリングされる
/// （ADR-129 が扱った実インシデント）。
///
/// 許可箇所は3つ: `hook.rs`（capture 時点で1回呼び `RawKeyEvent` へ埋め込む
/// 本来の発生源）、`runtime/mod.rs::build_ctx`、
/// `runtime/message_handlers.rs`（タイマー直接発火経路の手書き複製、
/// `build_ctx` を経由しない。ADR-155 が指摘した既知の未解消の穴）。
#[test]
fn thumb_down_timestamps_live_query_is_limited_to_designated_call_sites() {
    let known_sites: &[(&str, usize)] = &[
        ("src/hook.rs", 1),        // capture 時点で1回呼び RawKeyEvent へ埋め込む
        ("src/runtime/mod.rs", 1), // build_ctx
        ("src/runtime/message_handlers.rs", 1), // タイマー直接発火経路（build_ctx 非経由）
    ];
    for (path, expected) in known_sites {
        let content = read_crate_file(path);
        let count = count_real_calls(&content, "thumb_down_timestamps(");
        assert_eq!(
            count, *expected,
            "{path} 内の `thumb_down_timestamps()` 呼び出し箇所数が想定({expected})と \
             異なります(実際: {count})。ADR-129 参照。"
        );
    }

    let key_pipeline = read_crate_file("src/runtime/key_pipeline.rs");
    let count = count_real_calls(&key_pipeline, "thumb_down_timestamps(");
    assert_eq!(
        count, 0,
        "src/runtime/key_pipeline.rs は `hook::thumb_down_timestamps()` の \
         ライブクエリを呼んではならない（ADR-129）。`event.left_thumb_down_snapshot` / \
         `event.right_thumb_down_snapshot` を読むこと。"
    );
}

/// ADR-129 (a-2-i): `key_pipeline.rs` が実際に capture-time スナップショット
/// （`event.left_thumb_down_snapshot` / `event.right_thumb_down_snapshot`）を
/// 読んでいることを固定する。
///
/// 上の負のガード（ライブクエリを呼ばない）だけでは、「ライブクエリを消したが
/// `None, None` を渡している」状態を通してしまう（opus-adversarial-consult
/// round2 critic 指摘、S1）。`build_input_context(` への引数文字列を厳密に
/// 一致させる形にはしない——`let (left_thumb_down, right_thumb_down) =
/// (event.left_thumb_down_snapshot, event.right_thumb_down_snapshot);` という
/// ローカル束縛経由の実装も正しいため、出現有無だけを見る。
#[test]
fn key_pipeline_reads_thumb_down_snapshot_from_event() {
    let content = read_crate_file("src/runtime/key_pipeline.rs");
    for field in [
        "event.left_thumb_down_snapshot",
        "event.right_thumb_down_snapshot",
    ] {
        assert!(
            content.contains(field),
            "src/runtime/key_pipeline.rs は `{field}` を読んでいる必要があります（ADR-129）。"
        );
    }
}

/// ADR-129 (a-2-ii): `key_pipeline.rs` が `build_ctx()` を呼ばないことを固定する。
///
/// 将来ここで `self.build_ctx()` を呼べば `runtime/mod.rs::build_ctx` 経由で
/// ライブクエリが**間接的に**復活しうるが、上の負のガードは
/// `hook::thumb_down_timestamps` という文字列を探しているだけなのでそれを
/// 検知できない（opus-adversarial-consult round2 critic 指摘、S5）。
#[test]
fn key_pipeline_does_not_call_build_ctx() {
    let content = read_crate_file("src/runtime/key_pipeline.rs");
    assert!(
        count_real_calls(&content, "build_ctx(") == 0,
        "src/runtime/key_pipeline.rs は `build_ctx()` を呼んではならない（ADR-129）。\
         呼ぶと `hook::thumb_down_timestamps()` のライブクエリが間接的に復活しうる。"
    );
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
/// の中身は無視する** — `tracing::debug!("... {{ ... }}")` のような Rust の
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
/// BUG-182: `panic_reset` は非 Imm32 窓（Chrome/Edge・TsfNative）では OFF→ON を直列実行しないので、
/// `apply_panic_reset` の後に `ImeEffect::SetOpen { open: true, press: None }` を executor 経路
/// （`execute_decision`）へ積まなければ実 IME が開かない。ADR-213 P2c で ActivationSync を撤去した際、
/// パニックがこの暗黙の利用者だったことを見落とした回帰の再発防止（テキスト照合）。
#[test]
fn panic_reset_non_imm32_branch_queues_set_open() {
    let content = read_crate_file("src/runtime/mod.rs");
    let production = production_code_only(&content);
    let start = production
        .find("pub fn panic_reset(")
        .expect("panic_reset が見つからない");
    let body = &production[start..];
    let end = body
        .find("\n    }\n")
        .expect("panic_reset の終端が見つからない");
    let body = &body[..end];
    let apply = body
        .find("apply_panic_reset(")
        .expect("panic_reset は apply_panic_reset を呼ぶこと");
    let branch = body
        .find("if !self.can_use_imm32_cross_process()")
        .expect("panic_reset に非 Imm32 分岐が必要（BUG-182）");
    assert!(
        branch > apply,
        "非 Imm32 分岐は apply_panic_reset（applied の未知化）の後に置くこと"
    );
    let tail = &body[branch..];
    assert!(
        tail.contains("ImeEffect::SetOpen")
            && tail.contains("open: true")
            && tail.contains("press: None"),
        "panic_reset の非 Imm32 分岐は ImeEffect::SetOpen {{ open: true, press: None }} を積むこと"
    );
    assert!(
        tail.contains("self.execute_decision("),
        "SetOpen は新しい起案口を作らず既存の executor 経路（execute_decision）へ積むこと"
    );
}

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
        // 1 = apply_ime_update (ObserverPoll, Medium)。
        // +1 (ADR-158 TF1、2026-09-09) =
        // `#[cfg(test)] mod tests`内の
        // `dispatch_event_journals_observation_source_without_new_journal_entry_variant`
        // （journal記録経路を検証する回帰テストのヘルパー。実際の外部API/probe観測では
        // ない——本ガードが対象とする「production codeでの偽装」ではなくテストフィクス
        // チャ）。
        ("src/state/platform_state.rs", 2),
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
/// - `assume_closed_for_new_thread`: awase 起動後に作られたスレッドの安全デフォルト OFF
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
        count, 2,
        "{path} 内の `evidence::HeuristicDefault` 使用箇所数が想定(2)と異なります(実際: {count})。\n\
         想定: reset_stale_ime_on_for_imm_broken (Imm32Unavailable entry → ON) と\n\
         assume_closed_for_new_thread (awase 起動後の新規スレッド → OFF) の2箇所。\n\
         (reset_to_off_for_tsf_native_cache_miss は 37883d0 で TsfNative SSOT 化に伴い削除済み)\n\
         新しい安全デフォルト推測を追加する場合は `UserImeSetIntent` を使わず \
         `Observed::<evidence::HeuristicDefault>::at_startup` を使い、このカウントを \
         更新してください。"
    );
    for designated in [
        "fn reset_stale_ime_on_for_imm_broken",
        "fn assume_closed_for_new_thread",
    ] {
        let body = extract_fn_body(production, designated);
        assert_eq!(
            body.matches("evidence::HeuristicDefault").count(),
            1,
            "`{designated}` が `HeuristicDefault` の designated 使用箇所であること"
        );
    }
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
            10,
            "typed writer 定義 3 + handle_engine_set_open 内部委譲 1 (Decision 経由 \
             SetOpen — 救済: kp_stage_post_decision の PostSetOpenEisuReset) + \
             BUG-51 追補 v3 の IntentStore 回帰テスト内での write_sync_key/\
             write_physical_key 直接呼び出し 6 件（BUG-110 追補9、issue #189: \
             check_drift_correction_ignores_heuristic_default_alone_without_\
             explicit_intent が明示 OFF を作るための write_sync_key 呼び出しを \
             1 件追加）（新しい本番 IME-ON 経路ではなく既存 typed writer を \
             テストから呼んでいるだけなので eisu-reset の追加配線は不要）",
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
    let engine = read_workspace_file("src/engine/nicola_fsm.rs");
    let eisu = read_crate_file("src/state/eisu_recovery.rs");
    assert!(
        engine.contains("forced_open_action")
            && eisu.contains("無変換/変換の開閉の役割")
            && eisu.contains("PostSetOpenEisuReset")
            && eisu.contains("eisu_reset_on_ime_on"),
        "ADR-192決定3bの user IME-ON 経路は Decision 経由の \
         PostSetOpenEisuReset/eisu_reset_on_ime_on と対で登録すること"
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
    let eisu_recovery = read_crate_file("src/state/eisu_recovery.rs");
    for needle in [
        "owned キーの shadow-toggle",
        "owned キーの Phase 3 delegate",
        "非owned キーの物理 IME キー / SyncKey shadow toggle",
    ] {
        assert!(
            eisu_recovery.contains(needle),
            "src/state/eisu_recovery.rs の user IME-ON 経路×ObservedEisu救済の \
             対応表に `{needle}` がありません。ADR-135 Phase 3 の owned/non-owned \
             分割に合わせてSSOTを更新してください。"
        );
    }
}

/// ADR-206: エンジン非活性側の無変換/変換の開閉（役割由来）は Windows パイプライン（旧ケース2/3改）ではなく
/// エンジンの特殊キー照合（S1 と同じ入口）に合流する。生キーの抑止は `Decision::Consume`（Down）と
/// `UpDuty::Consume`（Up）が対で負うので、旧マーカーや KeyUp の再評価が再導入されていないことと、
/// エンジン側の入口が必要なゲートを持つことを固定する。
#[test]
fn forced_thumb_path_lives_in_the_engine_special_key_match() {
    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    let kp_production = production_code_only(&kp);
    for banned in [
        "explicit_ime_action_target",
        "ExplicitImeActionOutcome",
        "explicit_ime_action_consumed",
        "explicit_action_for_pipeline",
    ] {
        assert!(
            !kp_production.contains(banned),
            "key_pipeline.rs に旧ケース2/3改の `{banned}` が再導入されています。無変換/変換の開閉は \
             エンジンの特殊キー照合（`Engine::thumb_open_role_action`）と FSM の単独タップ解決が担い、\
             生キーの抑止は Decision::Consume/UpDuty::Consume に任せる（ADR-206。旧経路は belief を書くだけで \
             実 IME へ ON を書かず、NotRomajiInput 等で「抑止したのに何も書かない」空振りになった）。"
        );
    }
    let shadow = extract_fn_body(kp_production, "fn kp_stage_shadow_ime_toggle(");
    assert!(
        !shadow.contains("forced_open_action") && !shadow.contains("VK_NONCONVERT"),
        "kp_stage_shadow_ime_toggle は無変換/変換の開閉を扱わない（ADR-206）"
    );

    let engine = read_workspace_file("src/engine/engine.rs");
    let engine_production = production_code_only(&engine);
    let body = extract_fn_body(engine_production, "fn thumb_open_role_action(");
    for gate in [
        "self.compute_active(ctx)",
        "ctx.is_japanese_ime",
        "self.adapter.is_enabled()",
        "Self::is_bare_thumb(event, ctx.modifiers)",
        "sync_direction.is_some()",
    ] {
        assert!(
            body.contains(gate),
            "Engine::thumb_open_role_action のゲート `{gate}` がありません（ADR-206 決定3）。\
             エンジン無効中・日本語 IME でない・修飾付き・ime_detect と重なるキーを能動にしてはならない。"
        );
    }
    let check = extract_fn_body(engine_production, "fn check_special_keys(");
    assert!(
        check.contains("event.was_down") && check.contains("Decision::consumed()"),
        "check_special_keys は自動リピートの Down で指令を作らず Consume だけ返すこと（ADR-206 不変条件）"
    );
    assert!(
        engine_production.contains("phase1_held"),
        "Phase 1 で消費した親指のリピートを FSM に渡さない印 `phase1_held` が必要です（ADR-206 決定3）"
    );
    assert!(
        !engine_production.contains("SetOpen {\n                open: action")
            && !body.contains("ImeEffect::SetOpen"),
        "thumb_open_role_action は SetOpen を直接積まない（`ime_set_open_effects` 経由。ADR-206）"
    );

    // 無変換/変換の VK 分岐は配送判断の核に残す（Allow を返す形。分岐ごと消すと将来 shadow_action が付いたとき
    // ImmCross で無条件 Suppress される）。核は ADR-208 L0 で `runtime/transport.rs` から
    // `state/physical_disposition.rs`（ungated）へ挙動を変えずに移した。
    let transport = read_crate_file("src/state/physical_disposition.rs");
    let disposition = extract_fn_body(
        production_code_only(&transport),
        "fn thumb_or_role_fkey_disposition(",
    );
    assert!(
        disposition.contains("VK_CONVERT | crate::vk::VK_NONCONVERT")
            && !disposition.contains("explicit_ime_action_consumed"),
        "physical_disposition.rs の無変換/変換の VK 分岐は Allow を返す形で残すこと（マーカーは撤去済み、ADR-206）"
    );

    // InputRelay の窓では役割を付けない（エンジンが Consume して actuation が NotOwned だと誰も書かない）。
    let rt = read_crate_file("src/runtime/mod.rs");
    let role = extract_fn_body(production_code_only(&rt), "fn enrich_thumb_key_role(");
    assert!(
        role.contains("AppImeProfile::InputRelay"),
        "enrich_thumb_key_role は InputRelay の窓で役割を付けないこと（ADR-206 決定3、ADR-119）"
    );
}

/// ADR-206 / BUG-174 の回帰ガード（所有者のマージ条件、2026-09-29）: **Ctrl を離したとき（Ctrl↑）に awase は
/// IME への actuation（SendInput・ImmSetOpenStatus・apply_ime_open_*）をしない。**
/// 旧 `CompositionEvent::CtrlUp` の eager warmup は Ctrl 押下中に `VK_IME_ON` を注入し、GJI + Windows Terminal で
/// 「@」の被疑箇所だった（BUG-174、`aa53eb4b` で撤去）。再導入と、Ctrl↑ 経路への actuation の混入を検知する。
/// 「@」そのものの再現は実機 A/B で、ここでは Ctrl↑ 経路に actuation 呼び出しが存在しないことをホストで固定する。
#[test]
fn ctrl_key_up_never_actuates_ime() {
    // 1. 旧 CtrlUp warmup の識別子が復活していない（crate 全体）。
    let workspace_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![workspace_src];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let content = fs::read_to_string(&path)
                    .expect("read")
                    .replace("\r\n", "\n");
                let production = production_code_only(&content);
                for banned in [
                    "CompositionEvent::CtrlUp",
                    "WarmupReason::CtrlUp",
                    "composition_ctrl_up",
                    "handle_ctrl_up_recovery",
                ] {
                    assert!(
                        !production.contains(banned),
                        "{} に `{banned}` が復活しています。Ctrl↑ で awase が VK_IME_ON を注入する経路は \
                         BUG-174（Windows Terminal + GJI の「@」被疑、`aa53eb4b` で撤去）です。",
                        path.display()
                    );
                }
            }
        }
    }

    // 2. Ctrl↑ 専用のハンドラ（`on_ctrl_key_up`）は belief のバリア解除だけで、actuation を呼ばない。
    let ps = read_crate_file("src/state/platform_state.rs");
    let body = extract_fn_body(production_code_only(&ps), "fn on_ctrl_key_up(");
    for banned in [
        "SendInput",
        "send_input",
        "send_ime_control",
        "set_ime_open",
        "apply_ime_open",
        "issue_actuation_order",
        "ImmSetOpenStatus",
        "send_eager_warmup",
    ] {
        assert!(
            !body.contains(banned),
            "on_ctrl_key_up が `{banned}` を含んでいます。Ctrl↑ で IME に actuation してはならない（BUG-174）。"
        );
    }

    // 3. パイプラインの Ctrl 系 KeyUp ブロックは `on_ctrl_key_up` を呼ぶだけ。
    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    let kp_prod = production_code_only(&kp);
    let start = kp_prod
        .find("is_ctrl_variant(event.vk_code)\n        {")
        .expect("Ctrl 系 KeyUp ブロック（is_ctrl_variant）が見つかりません");
    let block = &kp_prod[start..start + 400.min(kp_prod.len() - start)];
    assert!(
        block.contains("on_ctrl_key_up(") && !block.contains("apply_") && !block.contains("send_"),
        "Ctrl 系 KeyUp ブロックは on_ctrl_key_up の呼び出しだけであること（Ctrl↑ で actuation しない、BUG-174）: {block}"
    );
}

/// ADR-206 決定5: Decision 経由の `SetOpen(true)` の eisu 救済（`kp_stage_post_decision`）は GJI の英数保持
/// （`gji_retains_tracked_eisu`、BUG-159）を渡すこと。`false` 固定だと awase だけ AssumedRomaji に戻り、
/// GJI が英数を保持したまま NICOLA のローマ字がリテラルで出る。
#[test]
fn post_decision_eisu_reset_passes_gji_retained_mode() {
    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    let body = extract_fn_body(production_code_only(&kp), "fn kp_stage_post_decision(");
    assert!(
        body.contains("gji_retains_tracked_eisu("),
        "kp_stage_post_decision の eisu 救済に `gji_retains_tracked_eisu` の結果（mode_retained）を渡すこと \
         （BUG-159 の再発防止、ADR-206 決定5）"
    );
}

#[test]
fn ime_relevance_shadow_action_writes_are_accounted_for() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    let expected: &[(&str, usize, &str)] = &[
        (
            "hook.rs",
            1,
            "hook::classify_ime_relevance が静的 ImeKeyKind から初期値を書く",
        ),
        (
            "runtime/mod.rs",
            1,
            "Runtime::enrich_key_role が候補キーの役割(ADR-199決定8、T4の配線範囲は半角/全角0xF3/0xF4)から消費時に上書きする",
        ),
    ];

    for path in files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = read_crate_file(&format!("src/{rel}"));
        let production = production_code_only(&content);
        let count = if rel == "hook.rs" {
            production.matches("shadow_action,").count()
        } else {
            production.matches("ime_relevance.shadow_action =").count()
        };
        let expected_count = expected
            .iter()
            .find(|(f, _, _)| *f == rel)
            .map_or(0, |(_, n, _)| *n);
        assert_eq!(
            count, expected_count,
            "src/{rel} の event.ime_relevance.shadow_action 書き込み箇所数が想定\
             ({expected_count})と異なります(実際: {count})。本番の書き込み点は \
             hook::classify_ime_relevance と Runtime::enrich_key_role の2箇所に\
             限定してください（ADR-191）。"
        );
    }
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

/// ADR-213 P2d-2: settle 中の明示操作 `SetOpen` を落とす仕組みを復活させない。
///
/// `strip_ime_set_open_if_settling`（decision から SetOpen を除去）と
/// `handle_engine_set_open` の `focus_transition_was_pending` フィルタ（belief 側）は、
/// ActivationSync 撤去後は settle 中（フォーカス検知後 ImmCross 100ms・TsfNative 200ms）に
/// 押された Ctrl+変換等を黙って捨てるだけになった。Chrome×GJI・Chrome×MS-IME は settle 約20ms
/// 後の書き込みを受け付けると CI で実測済み（docs/experiments.md エントリ 30）。
/// strip と belief フィルタは対でしか意味が無い（片方だけだと belief と実書き込みが食い違う）ので、
/// どちらの再導入も本テストで止める。再導入するなら、窓の切り替え直後の明示操作で IME が
/// 逆向きに切り替わった等の実測（ADR-213 の revert 条件）を添えて本テストごと更新すること。
#[test]
fn settle_does_not_drop_explicit_set_open() {
    let executor_src = read_crate_file("src/runtime/executor.rs");
    let executor = production_code_only(&executor_src);
    assert!(
        !executor.contains("fn strip_ime_set_open_if_settling"),
        "runtime/executor.rs に strip_ime_set_open_if_settling が復活しています（ADR-213 P2d-2 で撤去済み）"
    );
    let key_pipeline_src = read_crate_file("src/runtime/key_pipeline.rs");
    let key_pipeline = production_code_only(&key_pipeline_src);
    assert!(
        !key_pipeline.contains("strip_ime_set_open_if_settling(")
            && !key_pipeline.contains("focus_transition_was_pending:")
            && !key_pipeline.contains("focus_transition_was_pending,")
            && !key_pipeline.contains("let focus_transition_was_pending"),
        "runtime/key_pipeline.rs が settle で SetOpen を落とす分岐を再導入しています（ADR-213 P2d-2）"
    );
    let platform_state_src = read_crate_file("src/state/platform_state.rs");
    let platform_state = production_code_only(&platform_state_src);
    assert!(
        !platform_state.contains("focus_transition_was_pending:")
            && !platform_state.contains("focus_transition_was_pending {")
            && !platform_state.contains("if focus_transition_was_pending"),
        "state/platform_state.rs の handle_engine_set_open に settle フィルタが復活しています（ADR-213 P2d-2）"
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
/// 内部同期（`EngineSync::DirectInput`（ADR-185で撤去済み） 等が `UserImeSetIntent{Command}` を
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
         pre-mortem #1 角度2）。conv 由来の内部同期（`EngineSync::DirectInput`（ADR-185で撤去済み） 等が \
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
        count_real_calls(wrapper[0], "self.clock.now_tick("),
        1,
        "`ImeStateHub::effective_open()` が `self.clock.now_tick()` 以外の時刻で \
         IntentStore を評価しようとしています。"
    );
    // 時間軸の同一性は「実機の HubClock が `hook::current_tick_ms` を読む」ことで保つ
    // （旧: effective_open() が current_tick_ms() を直接呼んでいた。仮想時計を差し込めるよう
    // HubClock 経由にした。実時計の tick の出所はここで固定する）。
    assert_eq!(
        count_real_calls(
            production_code_only(&read_crate_file("src/state/platform_state/shell.rs")),
            "HubClock::wall(crate::hook::current_tick_ms)"
        ),
        1,
        "`ImeStateHub` の時計が `hook::current_tick_ms` の実時計ではありません。\
         record 側（`runtime/key_pipeline.rs` の `hook::current_tick_ms()`）と TTL 判定の\
         時間軸が食い違い、実機で IntentStore の上書きが沈黙します。"
    );
}

/// コメント（行コメント・ネストするブロックコメント）だけを除く小さな字句走査。
/// 文字列リテラル（`"..."`・`r#"..."#`）と文字リテラルの中の `//`・`/*` はコメント扱いしない
/// （コメントや文字列の中の glob `tests/*.json` で、その後ろの本番コードが消えないように）。
/// 除いたあと、空行を落として各行の末尾の空白も落とす。
fn strip_comments(code: &str) -> String {
    let b: Vec<char> = code.replace('\r', "").chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if b[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
        } else if c == 'r'
            && (i == 0
                || !ident(b[i - 1])
                // `br#".."#`・`cr#".."#`（接頭辞つきの生文字列）
                || (matches!(b[i - 1], 'b' | 'c') && (i == 1 || !ident(b[i - 2]))))
            && {
                let mut j = i + 1;
                while b.get(j) == Some(&'#') {
                    j += 1;
                }
                b.get(j) == Some(&'"')
            }
        {
            // 生文字列 r##"..."##
            let mut j = i + 1;
            let mut hashes = 0;
            while b[j] == '#' {
                hashes += 1;
                j += 1;
            }
            j += 1; // 開き `"`
            while j < b.len() {
                if b[j] == '"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&'#')) {
                    j += 1 + hashes;
                    break;
                }
                j += 1;
            }
            out.extend(&b[i..j.min(b.len())]);
            i = j;
        } else if c == '"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != '"' {
                j += if b[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            out.extend(&b[i..end]);
            i = end;
        } else if c == '\'' {
            // 文字リテラル（`'"'`・`'\n'`）はまとめて写す。lifetime（`'a`）は `'` だけ写す。
            let end = if next == Some('\\') {
                // `'\''` の2つ目の `'` を閉じと誤認しないよう、エスケープ文字の次から探す。
                b.get(i + 3..)
                    .and_then(|rest| rest.iter().position(|&x| x == '\'').map(|k| i + 3 + k + 1))
            } else if b.get(i + 2) == Some(&'\'') {
                Some(i + 3)
            } else {
                None
            };
            let end = end.unwrap_or(i + 1).min(b.len());
            out.extend(&b[i..end]);
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `with_clock(` の呼び出し数。`new_with_clock(` など識別子の一部と、`fn with_clock(`（定義）は数えない。
fn count_with_clock_calls(code: &str) -> usize {
    code.lines()
        .filter(|l| !l.contains("fn with_clock("))
        .map(|l| {
            l.match_indices("with_clock(")
                .filter(|(i, _)| {
                    l[..*i]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                })
                .count()
        })
        .sum()
}

/// `ImeStateHub.clock` への書き込みの数。private でも子モジュール（`platform_state/` 配下）と核の
/// 本番メソッドからは書けるため数える。拾うもの: `.clock = ..`・`*clock = ..`（分配束縛）・
/// `&mut <path>.clock`／`&mut (<path>.clock)`／`ref mut clock`（`mem::replace`・`mem::swap`・`mem::take`
/// は `&mut` で拾う）・`clock.clone_from(..)`（`HubClock` は `Clone` なので今すでに書き換えの手段）・
/// 行内の分配束縛／リテラル `{ clock, .. }`・`{ clock: c, .. }`・`, clock: c`。
///
/// 限界（拾えない）:
/// - 別名に束縛し直したあとの書き込み（`let c = &mut self.clock;` は `&mut` で拾えるが、
///   `let Self { clock: c, .. } = self;` が複数行に分かれて `{` と同じ行に無い形、`let c = &mut *(..)` の
///   ような経由）。
/// - `&mut self` を取るメソッド呼び出し（`self.clock.advance_ms(n)` は今すでに存在する。実時計の `Wall` では
///   何もしないが、実装が変われば抜け道）。`clone_from` 以外の `Clone`/`Default` 由来の書き換えも同様。
/// - `Cell`/`RefCell` 経由（今の `clock` は内部可変性を持たない enum で、型を変えれば diff に出る）。
fn count_clock_writes(code: &str) -> usize {
    let assign_after = |rest: &str| {
        let t = rest.trim_start();
        t.starts_with('=') && !t.starts_with("==")
    };
    let mut n = 0;
    for l in code.lines() {
        for (k, _) in l.match_indices("clock") {
            let before = &l[..k];
            let after = &l[k + "clock".len()..];
            if after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                continue; // `clocks` など
            }
            let dotted = before.ends_with('.');
            let deref = before.ends_with('*');
            if (dotted || deref) && assign_after(after) {
                n += 1;
            } else if before.trim_end().ends_with("ref mut") {
                n += 1;
            } else if let Some(m) = before.rfind("&mut ") {
                // `&mut (self.clock)`・`&mut self.clock`・`&mut clock`
                let mid = before[m + 5..].trim_start_matches('(');
                if mid
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                {
                    n += 1;
                }
            }
            if after.starts_with(".clone_from(") {
                n += 1;
            }
            let bt = before.trim_end();
            let at = after.trim_start();
            if (bt.ends_with('{') || bt.ends_with(','))
                && (at.starts_with(':') || at.starts_with(',') || at.starts_with('}'))
            {
                n += 1;
            }
        }
    }
    n
}

/// 行頭の `clock:`・`clock,`（構造体リテラルの値・短縮形、フィールド宣言）の数。
/// 期待値は `platform_state.rs`（フィールド宣言 1 + `with_clock` 本体の短縮形 1）の 2 件だけで、
/// 他のファイルは 0 件。別の構築関数（`clock: c,` など）が増えたら件数が増えて落ちる。
fn count_clock_field_lines(code: &str) -> usize {
    code.lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("clock:") || t.starts_with("clock,")
        })
        .count()
}

/// 殻の本番コードが渡す実時計の式（`effective_open_at` を検査する
/// `effective_open_is_wired_to_the_intent_store_decision`の `HubClock::wall(..)` 件数の固定と同じ式）。
const REAL_CLOCK_CALL: &str = "with_clock(HubClock::wall(crate::hook::current_tick_ms))";

/// `with_clock(` の呼び出し元を、本番（`mod tests` を除く）では次の2か所だけに固定する:
/// - `state/platform_state/shell.rs`（`new()` の殻。実時計）
/// - `state/platform_state.rs` の `#[cfg(test)] impl PlatformState`（`for_test`）
///
/// 加えて、この2ファイルの本番コードで `clock` への書き込みが 0 件であること。
/// すべての照合はコメントを除いてから行う。違反を説明で返す。
fn with_clock_call_violations(sources: &[(String, String)]) -> Vec<String> {
    const SHELL: &str = "src/state/platform_state/shell.rs";
    const CORE: &str = "src/state/platform_state.rs";
    let mut out = Vec::new();
    for (path, content) in sources {
        let production = strip_comments(production_code_only(content));
        let calls = count_with_clock_calls(&production);
        let allowed = path == SHELL || path == CORE;
        if calls > 0 && !allowed {
            out.push(format!(
                "{path}: with_clock( の本番呼び出し {calls} 件（許可リスト外）"
            ));
        } else if allowed && calls != 1 {
            out.push(format!(
                "{path}: with_clock( の呼び出しが {calls} 件（1件であること）"
            ));
        }
        // `clock` は private だが、`platform_state/` 配下の子モジュールには親の private が見える。
        if path == CORE || path.starts_with("src/state/platform_state/") {
            let writes = count_clock_writes(&production);
            if writes != 0 {
                out.push(format!(
                    "{path}: clock への書き込み（代入・&mut・分配束縛）が {writes} 件（0件であること）"
                ));
            }
            let want = usize::from(path == CORE) * 2;
            let fields = count_clock_field_lines(&production);
            if fields != want {
                out.push(format!(
                    "{path}: clock のフィールド行（宣言・リテラル・短縮形）が {fields} 件（{want}件であること）"
                ));
            }
        }
        if path == CORE && calls == 1 {
            // `#[cfg(test)] impl PlatformState { .. }` ブロックの**中身**に呼び出しがあること
            // （空の gated `impl` を残して外へ移すと、gate 文字列が前にあるだけでは通ってしまう）。
            let gated: usize =
                extract_all_balanced_blocks(&production, "#[cfg(test)]\nimpl PlatformState {")
                    .iter()
                    .map(|b| count_with_clock_calls(b))
                    .sum();
            if gated != 1 {
                out.push(format!(
                    "{path}: with_clock( の呼び出しが `#[cfg(test)] impl PlatformState`（for_test）の中にありません"
                ));
            }
        }
        if path == SHELL {
            // 殻の1件は実時計を渡すこと（空白を除いて照合）。
            let squeezed: String = production.chars().filter(|c| !c.is_whitespace()).collect();
            if squeezed.matches(REAL_CLOCK_CALL).count() != 1 {
                out.push(format!(
                    "{path}: with_clock の引数が `{REAL_CLOCK_CALL}`（実時計）ではありません"
                ));
            }
        }
    }
    out
}

#[test]
fn with_clock_is_called_only_by_real_clock_shell_and_for_test() {
    let sources: Vec<(String, String)> = list_src_files()
        .into_iter()
        .map(|p| {
            let c = read_crate_file(&p);
            (p, c)
        })
        .collect();
    let v = with_clock_call_violations(&sources);
    assert!(
        v.is_empty(),
        "`with_clock(` の本番呼び出し元が想定と異なります: {v:?}。本番で `ImeStateHub` に任意の時計を\
         渡す入口を増やすと、TTL 判定の時間軸（`hook::current_tick_ms`）が食い違う恐れがあります。\
         実時計は `state/platform_state/shell.rs` の `new()` だけです。"
    );
}

#[test]
fn with_clock_guard_detects_new_production_entry() {
    const SHELL: &str = "src/state/platform_state/shell.rs";
    const CORE: &str = "src/state/platform_state.rs";
    let shell_ok = "fn new() { Self::with_clock(HubClock::wall(crate::hook::current_tick_ms)) }";
    // フィールド宣言 1 + `with_clock` 本体の短縮形 1（行頭の `clock:`・`clock,`）を持つ最小の核。
    let core_ok = "struct ImeStateHub {\n    clock: C,\n}\nimpl ImeStateHub {\n fn with_clock(clock: C) -> Self {\n Self {\n clock,\n }\n }\n}\n#[cfg(test)]\nimpl PlatformState {\n fn for_test() { ImeStateHub::with_clock(c) }\n}\n";
    let build = |shell: &str, core: &str| {
        vec![
            (SHELL.to_string(), shell.to_string()),
            (CORE.to_string(), core.to_string()),
        ]
    };
    let has = |srcs: &[(String, String)], needle: &str| {
        with_clock_call_violations(srcs)
            .iter()
            .any(|m| m.contains(needle))
    };
    assert!(with_clock_call_violations(&build(shell_ok, core_ok)).is_empty());

    // 新しい本番の入口（別ファイル）
    let mut other = build(shell_ok, core_ok);
    other.push((
        "src/runtime/mod.rs".to_string(),
        "fn x() { ImeStateHub::with_clock(HubClock::manual()) }".to_string(),
    ));
    assert!(has(&other, "許可リスト外"));
    // 殻が実時計以外を渡す
    assert!(has(
        &build(
            "fn new() { Self::with_clock(HubClock::manual(0)) }",
            core_ok
        ),
        "実時計"
    ));
    // 実時計の式がコメントにしか無い（行コメント・ブロックコメント）
    for c in [
        "// with_clock(HubClock::wall(crate::hook::current_tick_ms))\nSelf::with_clock(HubClock::manual(0))",
        "/* with_clock(HubClock::wall(crate::hook::current_tick_ms)) */ Self::with_clock(HubClock::manual(0))",
    ] {
        assert!(has(&build(c, core_ok), "実時計"), "{c}");
    }
    // 空の gated `impl` を残し、`for_test` を外へ移す
    assert!(has(
        &build(
            shell_ok,
            "#[cfg(test)]\nimpl PlatformState {\n}\nimpl PlatformState {\n fn for_test() { ImeStateHub::with_clock(c) }\n}\n"
        ),
        "for_test"
    ));
    // gated ブロックの doc コメントにだけ `ImeStateHub::with_clock(` がある（実呼び出しは外）
    assert!(has(
        &build(
            shell_ok,
            "#[cfg(test)]\nimpl PlatformState {\n /// ImeStateHub::with_clock(clock) と同じ\n fn helper() {}\n}\nimpl PlatformState {\n fn for_test() { ImeStateHub::with_clock(c) }\n}\n"
        ),
        "for_test"
    ));
    // 殻にもう1件
    assert!(has(
        &build(
            &format!("{shell_ok}\nfn y() {{ Self::with_clock(c) }}"),
            core_ok
        ),
        "1件であること"
    ));
    // for_test が `#[cfg(test)]` の外
    assert!(has(
        &build(
            shell_ok,
            "impl PlatformState {\n fn for_test() { ImeStateHub::with_clock(c) }\n}\n"
        ),
        "for_test"
    ));
    // clock への書き込み（代入・setter・&mut・分配束縛・括弧つき &mut）
    for (shell, core) in [
        ("fn new() { let mut s = Self::with_clock(HubClock::wall(crate::hook::current_tick_ms)); s.clock = HubClock::manual(0); s }", core_ok.to_string()),
        (shell_ok, format!("{core_ok}impl ImeStateHub {{ fn set(&mut self, c: C) {{ self.clock = c; }} }}")),
        (shell_ok, format!("{core_ok}impl ImeStateHub {{ fn m(&mut self) {{ std::mem::swap(&mut self.clock, &mut o); }} }}")),
        (shell_ok, format!("{core_ok}impl ImeStateHub {{ fn m(&mut self) {{ let r = &mut (self.clock); }} }}")),
        (shell_ok, format!("{core_ok}impl ImeStateHub {{ fn m(&mut self) {{ let Self {{ clock, .. }} = self; *clock = c; }} }}")),
        (shell_ok, format!("{core_ok}impl ImeStateHub {{ fn m(&mut self) {{ let Self {{ ref mut clock, .. }} = *self; }} }}")),
    ] {
        assert!(has(&build(shell, &core), "clock への書き込み"), "{shell} / {core}");
    }
    // 名前を変えた分配束縛・clone_from・接頭辞つき生文字列の後ろ
    for body in [
        "let Self { clock: c, .. } = self; *c = HubClock::manual(0);",
        "self.clock.clone_from(&o);",
        "let r = br#\"a\"/*\"#; self.clock = c;",
        "let r = cr#\"a\"/*\"#; self.clock = c;",
        "std::mem::replace(&mut self.clock, c);",
        "std::mem::take(&mut self.clock);",
    ] {
        assert!(
            has(
                &build(shell_ok, &format!("{core_ok}fn m(&mut self) {{ {body} }}")),
                "clock への書き込み"
            ),
            "{body}"
        );
    }
    // 構造体リテラルの値が `HubClock::` で始まらない・短縮形（別の構築関数）
    for lit in [
        "clock: c,",
        "clock: make_clock(),",
        "clock,",
        "clock: HubClock::manual(0),",
    ] {
        assert!(
            has(
                &build(
                    shell_ok,
                    &format!("{core_ok}fn z() {{ Self {{\n{lit}\n }} }}")
                ),
                "フィールド行"
            ),
            "{lit}"
        );
    }
    // `platform_state/` 配下の別の子モジュールでの書き込み
    let mut child = build(shell_ok, core_ok);
    child.push((
        "src/state/platform_state/other.rs".to_string(),
        "fn f(h: &mut ImeStateHub) { h.clock = c; }".to_string(),
    ));
    assert!(has(&child, "other.rs: clock への書き込み"));
    // M1: コメント内の glob `/*`（閉じない）の後ろの本番コード・文字列の `"/*"` の後ろが消えない
    let mut glob = build(shell_ok, core_ok);
    glob.push((
        "src/state/other.rs".to_string(),
        "/// tests/journals/*.json を使う\nfn x() { ImeStateHub::with_clock(HubClock::manual(0)) }"
            .to_string(),
    ));
    assert!(has(&glob, "許可リスト外"));
    let in_string = build(
        shell_ok,
        &format!("{core_ok}fn x(&mut self) {{ let g = \"/*\"; self.clock = c; }}"),
    );
    assert!(has(&in_string, "clock への書き込み"));
    // 文字列内の `//`・生文字列・ネストしたブロックコメント・文字リテラルの `'"'`
    let tricky = build(
        shell_ok,
        &format!(
            "{core_ok}fn x() {{ let u = \"http://a\"; let r = r#\"/* \"#; let q = '\"'; /* a /* b */ c */ self.clock = c; }}"
        ),
    );
    assert!(has(&tricky, "clock への書き込み"));
    // `==` やコメント内の代入は書き込みではない。`new_with_clock(` や `fn with_clock(` 定義は数えない
    assert!(with_clock_call_violations(&build(
        shell_ok,
        &format!("{core_ok}// self.clock = c;\nfn q() {{ a.clock == b }}")
    ))
    .is_empty());
    let neutral = vec![(
        "src/journal.rs".to_string(),
        "fn with_clock(c: C) {}\nlet j = UnifiedJournal::new_with_clock(1, c);".to_string(),
    )];
    assert!(with_clock_call_violations(&neutral).is_empty());
}

/// M1: コメント内の glob（`tests/journals/*.json`）を持つ実在の 3 ファイルで、`strip_comments` が
/// 本番コードを消さないこと（関数の定義行が、除去の前後で同じ数だけ残る）。
#[test]
fn strip_comments_keeps_production_code_in_files_with_glob_comments() {
    for path in [
        "src/state/actuation_decision_record.rs",
        "src/state/conv_classify.rs",
        "src/state/ime_actuation.rs",
    ] {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let fn_lines = |text: &str| {
            text.lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !t.starts_with("//")
                        && (t.starts_with("fn ")
                            || t.starts_with("pub fn ")
                            || t.starts_with("pub(crate) fn "))
                })
                .count()
        };
        assert!(
            fn_lines(production) > 0,
            "{path}: 本番の関数が見つかりません"
        );
        assert_eq!(
            fn_lines(&strip_comments(production)),
            fn_lines(production),
            "{path}: strip_comments が本番コードを消しています"
        );
    }
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
/// `_with_applied` / `set_ime_open` / `set_ime_open_ordered` / `apply_ime_open`）の
/// **呼び出し箇所数**を、入口ごとに crate 全域で固定する（`_with_belief` と
/// `_with_applied` は ADR-216 R3 / ADR-179 で呼び出し元がなくなり、0 のまま
/// 「復活したら気づく」ために残している）
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
    // `tracing::info!("... apply_ime_open({open}) ...")` のような人間可読ログ文字列
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
        //
        // **2026-09-08（ADR-153決定1実装）**: 表 #12（`key_pipeline.rs::
        // kp_stage_shadow_ime_toggle`のケース3、無変換/変換単独タップの
        // 明示config`"off"`×belief既にOFF）が新規追加され 2→3。
        //
        // **2026-09-08（同日、ケース3撤回）**: 実機A/B実験で「@」再現の
        // 直接原因と確定し撤回したため、表 #12 の入口が消えて 3→2 に戻った
        // （`docs/known-bugs.md` BUG-113節・`docs/experiments.md`エントリ25）。
        // **2026-09-19（ADR-185）**: `key_pipeline.rs::kp_apply_conv_engine_sync`の`DirectInput`分岐
        // （半角英数検出時のIME OFF実送信、BUG-146）を撤去したため 2→1（残りは`ime_refresh.rs`の
        // drift correction のみ）。
        // ADR-216 R3: 唯一の呼び出し元にインライン化し、1→0。
        // 死んだ API が復活したら気づくためガードは残す。
        (".apply_ime_open_with_belief(", 0),
        // 外部 2（executor.rs engine decision / ime_refresh.rs drift correction）。
        //
        // **2026-09-08（ADR-121 D3）**: `mod.rs::reassert_explicit_physical_key`
        // （物理IMEキーno-op時の冪等再送、BUG-37部分対策）が新規追加され 3→4。
        //
        // **2026-09-19（領域A撤去、ユーザー指示）**: TsfNative向けON方向救済
        // 4系統（force-on/drift correction/warmup/reassert）のうち reassert
        // （`reassert_explicit_physical_key`）を撤去し、4→3に戻った。
        //
        // **2026-09-19（同日、force-on撤去）**: `mod.rs::force_on_and_correct_romaji`
        // （表 #6、force-ON 実送信の内部委譲元）も撤去し、3→2に戻った。
        // **2026-10-02（ADR-216 R3）**: drift correction の薄いラッパーを
        // インライン化したため、直接呼び出し元の内訳だけが変わった。
        (".apply_ime_open_with_view(", 2),
        // ADR-098 決定2（BUG-69）: 唯一の呼び出し元（ime_refresh.rs の GJI
        // TsfNative 強制 ON ブロック）を撤去し、メソッド自体も削除した。
        (".apply_ime_open_with_applied(", 0),
        // ADR-090 A-1 で `set_ime_open_ordered` へ移したため本番呼び出しゼロ。
        // **ガードは残す**——ここが 0 でなくなったら、warrant を通さない
        // actuation 入口が復活したことを意味する。
        (".set_ime_open(", 0),
        // 外部 1（ime_refresh.rs drift correction の ImmCross 分岐）。
        // **2026-09-25**: focus change 強制 OFF（`focus_change_enforce_off`）を撤去したため
        // 2→1（docs/adr/191-calibration-experiments.md「A/B-1」）。
        (".set_ime_open_ordered(", 1),
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
    // FCIS P2: 殻（`state/platform_state/shell.rs`）も走査し、ファイルごとの件数で固定する。
    // 殻を除外すると、殻へ `.record_*_in_scope(` の呼び出しを足しても気づけない。
    // 核の `record_ime_apply_result_in_scope` → `record_confirmed_in_scope` が従来の
    // `record_ime_apply_result` → `record_confirmed` の1件、殻は各 `_in_scope` をちょうど1件。
    // 殻を除く本番の呼び出し元の合計は分割前と同じ 1 / 5。
    const SHELL: &str = "src/state/platform_state/shell.rs";
    type Expected = &'static [(&'static str, usize)];
    const RECORDERS: [(&[&str], Expected); 3] = [
        // `record_ime_apply_result` の呼び出し元（核の `_in_scope` を呼ぶのは殻の1件だけ）。
        (
            &[
                ".record_ime_apply_result(",
                ".record_ime_apply_result_in_scope(",
            ],
            &[("src/runtime/mod.rs", 1), (SHELL, 1)],
        ),
        (
            &[".record_optimistic(", ".record_optimistic_in_scope("],
            &[("src/runtime/ime_refresh.rs", 1), (SHELL, 1)],
        ),
        (
            &[".record_confirmed(", ".record_confirmed_in_scope("],
            &[
                ("src/runtime/focus_tracking.rs", 1),
                ("src/runtime/ime_refresh.rs", 1),
                ("src/runtime/key_pipeline.rs", 1),
                ("src/runtime/mod.rs", 1),
                ("src/state/platform_state.rs", 1),
                (SHELL, 1),
            ],
        ),
    ];

    let files = list_src_files();
    for (needles, expected) in RECORDERS {
        let mut breakdown: Vec<(String, usize)> = Vec::new();
        for path in &files {
            let content = read_crate_file(path);
            let production = production_code_only(&content);
            let count: usize = needles
                .iter()
                .map(|n| count_real_calls(production, n))
                .sum();
            if count > 0 {
                breakdown.push((path.clone(), count));
            }
        }
        breakdown.sort();
        let want: Vec<(String, usize)> = expected
            .iter()
            .map(|(p, c)| ((*p).to_string(), *c))
            .collect();
        let non_shell: usize = breakdown
            .iter()
            .filter(|(p, _)| p != SHELL)
            .map(|(_, c)| c)
            .sum();
        assert_eq!(
            breakdown, want,
            "`{}` の呼び出し箇所(ファイルごとの件数)が想定と異なります(殻を除く合計: {non_shell})。\
             ADR-098 決定0 INV-A97-1（`ImeModel.applied` は実際に OS への actuation を\
             試みた経路だけが書いてよい）を確認し、新しい呼び出しがそれに違反しないか\
             （belief を actuation の記録として書いていないか）確認した上でこの期待値を\
             更新してください。既存の呼び出しのうち3箇所は actuation を伴わない belief\
             ミラーとして ADR-098 決定5 が明示的に許容した既知の例外です\
             （`state/platform_state.rs` の `record_optimistic_in_scope` doc 参照）。",
            needles[0]
        );
    }
}

/// 殻（`shell.rs`）の各メソッドが「`foreground_scope()` を1回読んで、1つの `_in_scope` に委譲するだけ」
/// であることを、メソッドごとに確かめる。違反があれば説明を返す。
fn shell_shape_violation(code: &str) -> Option<String> {
    let mut parts = code.split("pub(crate) fn ");
    parts.next(); // 最初のメソッドより前（`use`・`impl` の行）
    let mut methods = 0usize;
    for body in parts {
        methods += 1;
        let name = body.split('(').next().unwrap_or("?").trim();
        let reads = body.matches("foreground_scope()").count();
        if reads != 1 {
            return Some(format!(
                "`{name}`: foreground_scope() が {reads} 回（1回であること）"
            ));
        }
        let calls = body.matches("_in_scope(").count();
        if calls != 1 {
            return Some(format!(
                "`{name}`: `_in_scope(` の呼び出しが {calls} 個（1つであること）"
            ));
        }
        // 式の中の判断（`&&`・`||`・クロージャの `|`・`?`・`=>`）も含め、委譲以外を書かせない。
        for banned in [
            "if ", "match ", "for ", "while ", "loop ", "let ", "?", "&&", "|", "=>",
        ] {
            if body.contains(banned) {
                return Some(format!(
                    "`{name}`: `{banned}` を含む（委譲するだけであること）"
                ));
            }
        }
    }
    (methods < 13).then(|| format!("殻のメソッド数が13未満: {methods}"))
}

#[test]
fn shell_methods_only_read_scope_once_and_delegate() {
    let content = read_crate_file("src/state/platform_state/shell.rs");
    let code = non_comment_lines(production_code_only(&content));
    if let Some(v) = shell_shape_violation(&code) {
        panic!("殻の形の違反: {v}");
    }
    // 最初の `pub(crate) fn` より前（形の検査の対象外）に置けるのは、構築口の 3 つだけ
    // （`ImeStateHub::new`・`PlatformState::new`・`Default::default`）。ここに判断を書く抜け道を塞ぐ。
    let preamble = code.split("pub(crate) fn ").next().unwrap_or("");
    let fns: Vec<&str> = preamble
        .split("fn ")
        .skip(1)
        .map(|b| b.split('(').next().unwrap_or("?").trim())
        .collect();
    assert_eq!(
        fns,
        ["new", "new", "default"],
        "殻の先頭（形の検査の対象外）に構築口以外の関数があります"
    );
    assert_eq!(
        preamble.matches("foreground_scope()").count(),
        0,
        "構築口が foreground_scope() を読んでいます"
    );
}

/// `shell_shape_violation` 自体が、`if`/`match`/`for` で検出できない形も拾えることの確認。
#[test]
fn shell_shape_violation_detects_expression_level_decisions() {
    let ok = "pub(crate) fn a(&mut self) -> bool {\n self.a_in_scope(crate::win32::foreground_scope())\n}\n";
    let fourteen = ok.repeat(13);
    // 13 個の正しい殻は通る（`pub(crate) fn ` で区切るため先頭に前置きを足す）。
    assert_eq!(
        shell_shape_violation(&format!("impl X {{\n{fourteen}")),
        None
    );
    for bad_body in [
        "self.a_in_scope(foreground_scope()) && self.b",
        "self.a_in_scope(foreground_scope()) || true",
        "self.a_in_scope(foreground_scope()).then(|| 1)",
        "self.a_in_scope(foreground_scope())?",
        "self.a_in_scope(foreground_scope()); self.b_in_scope(foreground_scope())",
        "self.a_in_scope(foreground_scope()); self.b_in_scope(s)",
        "self.a_in_scope(s)",
        "self.a_in_scope(foreground_scope(), foreground_scope())",
    ] {
        let bad = format!("pub(crate) fn bad(&mut self) {{ {bad_body} }}\n{fourteen}");
        let code = format!("impl X {{\n{bad}");
        assert!(shell_shape_violation(&code).is_some(), "{bad_body}");
    }
}

// ADR-121 D3のreassert_ime_apply_complete_without_belief_write専用テスト
// （reassert_ime_apply_complete_skips_belief_write）は、2026-09-19に
// reassert機構自体（`reassert_explicit_physical_key`、TsfNative向けON方向
// 救済4系統の1つ）を撤去したため削除した。詳細はdocs/known-bugs/参照。

/// ADR-170 決定1: `ImeModel::reduce()` の大きい分岐を private ヘルパー
/// (`reduce_*`) へ抽出した。`.claude/rules/ime-belief-architecture.md` の
/// 「belief を書くのは reduce() だけ」という前提は、Rust の private が
/// モジュールスコープでしかない以上コンパイラでは強制されない
/// (opus-adversarial-consult round1 F2)。ヘルパーが `reduce()` の**本体内**
/// からのみ呼ばれることを、`reduce()` 本体スコープでの出現数とファイル全体
/// での出現数を突き合わせる二重固定で検証する——「ファイル内で件数1」だけの
/// 検証では、`reduce()` を経由しない別の呼び出し元1件を見逃せてしまう
/// (round2 R2-2)。ヘルパー名は `fn reduce_` 定義をファイルから自動抽出する
/// ため、新しいヘルパーを追加してもこのテスト自体の更新は不要
/// (round2 R2-3)。
///
/// 抽出条件は可視性修飾子(`pub`/`pub(crate)`)を剥がしてから`fn reduce_`と
/// 照合する——剥がさないと、ヘルパーに可視性を付けた瞬間そのヘルパーだけが
/// 自動抽出から静かに漏れてガード対象外になる(まさにこのガードが検知
/// すべき「reduce()以外から呼べるようになった」瞬間に自分が無効化される、
/// round3 R3-1)。
#[test]
fn reduce_helpers_are_called_only_from_reduce_body() {
    let path = "src/state/ime_model.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let reduce_body = extract_fn_body(production, "pub fn reduce(");

    let helper_names: Vec<String> = production
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let without_vis = trimmed
                .strip_prefix("pub(crate) ")
                .or_else(|| trimmed.strip_prefix("pub "))
                .unwrap_or(trimmed);
            let rest = without_vis.strip_prefix("fn reduce_")?;
            let end = rest.find('(')?;
            Some(format!("reduce_{}", &rest[..end]))
        })
        .collect();
    assert!(
        !helper_names.is_empty(),
        "{path} に ADR-170 の reduce_* ヘルパーが1つも見つかりません \
         (命名規約 `fn reduce_*` が変わった場合はこのテストの抽出条件も \
         見直してください)。"
    );

    for helper in &helper_names {
        let needle = format!("self.{helper}(");
        let in_body = count_real_calls(reduce_body, &needle);
        let in_whole_file = count_real_calls(production, &needle);
        assert_eq!(
            in_body, 1,
            "ADR-170: {helper} は reduce() 本体内から1回呼ばれるはずですが \
             {in_body} 回でした。"
        );
        assert_eq!(
            in_whole_file, in_body,
            "ADR-170: {helper} が reduce() の外からも呼ばれています \
             (本体内 {in_body} 回 / ファイル全体 {in_whole_file} 回)。belief を \
             書くヘルパーは reduce() 本体からのみ呼ぶこと。"
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
        // ime_model.rs: 5→6。`KeyEffectPredicted`のreduce内で、予測がappliedと食い違う向きへ開閉を動かしたとき
        // appliedを`Unknown`へ落とす1件を追加（BUG-156、`reduce()`内の正規書き込み）。
        // 6→7。`ModeKeyPassedThrough`のreduce内で、揃えた観測がappliedと食い違うときappliedを`Unknown`へ落とす
        // 1件を追加（ADR-205 D6、BUG-172。`reduce()`内の正規書き込み）。
        // 7→8 / platform_state 2→1（ADR-208 L0）。`ImeStateHub::record_confirmed` の `applied` 書き込み
        // （generation=None の完了記録）を、全列挙テストが本物の遷移を通せるよう `ImeModel::confirm_applied`
        // へ移した（挙動不変。`record_confirmed` はこれを呼ぶだけ）。書き込み点の総数は変わらない。
        // 8→9。`PanicReset`のreduce内で、全面リセット時にappliedを`Unknown`へ落とす1件を追加
        // （BUG-182。`reduce()`内の正規書き込み）。
        ("src/state/ime_model.rs", 9),
        ("src/state/platform_state.rs", 1),
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

// `force_on_retry_cooldown_gate_call_sites_are_accounted_for`（ADR-098 決定1-c、
// BUG-69 の 20ms 無限再試行ループ封鎖ガード）は削除した。2026-09-19、領域A撤去
// （ユーザー指示）で `apply_force_on_for_imm_broken`/`force_on_attempt_allowed`/
// `note_force_on_attempt`/`ForceOnRetryState` を丸ごと撤去したため、このテストが
// 固定していた「呼び出し箇所数1」という前提自体が意味を失った。

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

/// 早期 return 分岐の開始マーカー。`ir_apply_drift_correction`（FCIS F4 で observe→`decide_drift_plan`→
/// execute に分けた後の execute）の中で、計画の `DriftStep` を分岐する `match act.step { ... }` の先頭。
/// 旧 `match act_policy {`（`FeedbackPolicy` の分岐）に相当する。`Blind`/`GiveUp`・`Read`/`Confirmed` の
/// 判断は `state/drift_plan.rs` に移り、ここは計画の実行だけをする。
const DRIFT_MATCH_MARKER: &str = "let send_path = match act.step {";
/// 実送信ブロックの先頭にある `tracing::warn!` のメッセージ接頭辞。この直前で
/// `match act.step { ... }`（早期 return 分岐）が終わる。
const DRIFT_SEND_LOG_MARKER: &str = "[drift] correction: observed=";

/// `ir_apply_drift_correction`（execute）の `let send_path = match act.step { ... }` ブロック（＝
/// `GiveUp`/`Confirmed`/`SkipWarrantWouldBlock` の早期 return 腕）だけを切り出す。
///
/// 開始は `let send_path = match act.step {`、終了は実送信ブロックの先頭にある
/// `tracing::warn!("[drift] correction: observed=...")` の直前。この `tracing::warn!` より後は
/// ADR-080 不変条件6 のスコープ外（乖離が確定して実際に `set_ime_open` する正規経路であり、
/// そこで `dispatch_event(ImeEvent::DriftDetected {..})` を呼ぶのは正当）。したがって
/// **関数全体ではなく match ブロックだけ**を検査対象にする。行番号ではなくマーカー文字列で
/// 境界を求めるため、周辺のコードが動いても壊れにくい。
///
/// ADR-139 決定1: `log::warn!` から `tracing::warn!` への機械置換に伴い、この関数が
/// 探すマーカー文字列も同一コミットで更新した（更新を怠ると `ime_refresh.rs` の
/// 置換直後にこの関数が必ず panic する — 実際にタスク分解レビューで検出された）。
fn extract_drift_correction_match_block(content: &str) -> &str {
    let start = content
        .find(DRIFT_MATCH_MARKER)
        .unwrap_or_else(|| panic!("marker {DRIFT_MATCH_MARKER:?} not found in ime_refresh.rs"));
    let send_marker = content.find(DRIFT_SEND_LOG_MARKER).unwrap_or_else(|| {
        panic!("send-path marker {DRIFT_SEND_LOG_MARKER:?} not found in ime_refresh.rs")
    });
    // match ブロック内は `tracing::debug!` のみ。実送信は `tracing::warn!` で始まる唯一の箇所。
    let send_log = content[start..send_marker]
        .rfind("tracing::warn!(")
        .map_or_else(
            || panic!("no `tracing::warn!(` found between match block and send-path marker"),
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
/// 注意: `match act.step { ... }` ブロックの**後**にある正規の実送信経路は
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
            "{path} の ir_apply_drift_correction 内 `match act.step {{ ... }}` \
             （Blind/GaveUp・Read/Confirmed の早期 return 分岐）に、観測ストアへの \
             書き込みと思われるパターン `{forbidden}` が見つかりました。\n\
             ADR-080 不変条件6 により、GaveUp（および Read の deadline 超過/未収束）は \
             `observations` への書き込み（`ObserverReported` 等の dispatch）を \
             一切発生させてはなりません。違反すると docs/known-bugs.md BUG-33 と同型の \
             収束偽装（自分の belief を観測として書き戻し、drift 検知が二度と発火しない）\
             が再発します。実送信は match ブロックの後（`tracing::warn!(\"[drift] correction: \
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
        ("src/runtime/key_pipeline.rs", 3), // kp_shadow_actuate（ADR-213 決定1、OFF→ON の Targeted 書き込み） / kp_reset_to_hiragana_romaji_capsoff / kp_restore_kana_from_half_width（apply_focus_probe の ImmCrossProbe kana修正は2026-09-26に撤去、docs/adr/191-calibration-experiments.md A/B-2。apply_idle_conv_check の restore_roman(BUG-08 Apply(3))経路は2026-08-17 BUG-61に伴い撤去）
                                            // 2026-09-19（領域A撤去、ユーザー指示）: `src/runtime/mod.rs` の
                                            // try_force_on_bootstrap（force-ON bootstrap）を撤去したため、
                                            // mod.rs のエントリ（1）が消えた。
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

    // **2026-09-25**: focus change 強制 OFF（`focus_change_enforce_off`）を撤去したため、
    // この関数内の IME open 書き込み呼び出しはゼロ（旧: `set_ime_open_ordered(` 1 件）。
    // 復活したら warrant 経由の actuation 入口が増えたことを意味するので、
    // 意図的な変更ならこの期待値と `ime_open_actuation_entry_points_are_accounted_for` を更新すること。
    for needle in ["set_ime_open(", "set_ime_open_ordered("] {
        let count = count_real_calls(body, needle);
        assert_eq!(
            count, 0,
            "{path}::ir_post_focus_change_snapshot 内の `{needle}` 出現数が想定(0)と\
             異なります(実際: {count})。フォーカス変更時の強制 OFF は 2026-09-25 に撤去済み\
             （docs/adr/191-calibration-experiments.md「A/B-1」）。"
        );
    }
}

// NOTE: `force_policy_is_read_from_a_single_decision_point` と
// `is_force_policy_call_sites_are_accounted_for`（ADR-086 §6段3-4/§7-12）は
// 2026-08-17、ADR-094 で `conv_mode_policy`/`Output::is_force_policy()` 自体を
// 撤去したのに伴い削除した。

// `force_write_paths_bypass_gji_shadow_on_via_none_applied`（ADR-087 INV-28、
// force_on_and_correct_romaji の build_ime_control_view(None) bypassを固定）は
// 削除した。2026-09-19、領域A撤去（ユーザー指示）で `force_on_and_correct_romaji`
// 自体を丸ごと撤去したため、このテストが固定していた「唯一の enforcement 拠点」が
// 消滅した。INV-28 bypass のもう一方の担い手（`fallback_write`）は次の
// `fallback_write_bypasses_gji_shadow_on_via_none_override` が引き続き固定する。

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
        "fallback_write は view.control.shadow_on = None で GjiDirect の \
         already-matched skip を bypass する設計（BUG-113 追補、MsImeDirectは\
         shadow_onをskip判定に使わないため無関係）。この上書きが \
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
    //      `FeedbackPolicy::decide_action` が持つ、INV-41。`DriftEpisode::new` は
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

    // 3. 非同期チェーンの入口は 1 本（定義 1 + 呼び出し 2）。
    //
    // **2026-08-19（BUG-34 横展開 D）**: mod.rs::try_force_on_bootstrap が
    // 3本目の呼び出し元として加わった（executor.rs / key_pipeline.rs は既存）。
    // 同期 ImmCrossProcessStrategy::apply（エンジンスレッドを直接ブロックする
    // SendMessageTimeoutW 経路）から、この単一チェーン入口へ移行したもの。
    //
    // **2026-09-19（領域A撤去、ユーザー指示）**: try_force_on_bootstrap を
    // 丸ごと撤去したため、3本目の呼び出し元が消えて 3→2 に戻った
    // （executor.rs / key_pipeline.rs のみ）。
    let mut entry_calls = 0usize;
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        entry_calls += count_real_calls(production, "run_open_chain_async(");
    }
    assert_eq!(
        entry_calls, 2,
        "`run_open_chain_async(` の呼び出し箇所数が想定(2: executor.rs / \
         key_pipeline.rs)と異なります(実際: {entry_calls})。"
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
/// `SendInput` / `ImmSetOpenStatus` を起こせる。
/// ここに 3 本目の呼び出し元が生えると、`falls_through` 規則（次へ進むのは `Failed`
/// のときだけ、特に `UnsafeToToggle` で次の機構へ落ちない）も `Actuation` の
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

/// ADR-171: `candidate_was_seen` を消費する呼び出し箇所を固定する。
///
/// ADR-171 対象は2箇所: `platform.rs::on_ime_applied_inner` の既存リセットと、
/// `ime_controller.rs::apply_mechanism` の GjiDirect OFF 方向 override 送信直後の
/// リセット。`runtime/focus_tracking.rs` にもフォーカス変更時のキャリーオーバー
/// 防止用リセットが1箇所あるが、これは本ADRのスコープ外なので意図的に除外する。
#[test]
fn candidate_was_seen_consumption_sites_are_pinned_for_adr171() {
    const NEEDLE: &str = "crate::tsf::observer::reset_candidate_was_seen(";
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in list_src_files() {
        if path == "src/runtime/focus_tracking.rs" {
            continue;
        }
        let content = read_crate_file(&path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            breakdown.push((path, count));
        }
    }
    breakdown.sort();
    assert_eq!(
        breakdown,
        vec![
            ("src/ime_controller.rs".to_string(), 1),
            ("src/platform.rs".to_string(), 1),
        ],
        "`reset_candidate_was_seen(` の ADR-171 対象呼び出し箇所は \
         `platform.rs` と `ime_controller.rs` の2箇所に固定されています。\
         実際: {breakdown:?}"
    );
}

/// `count_real_calls` に加えて、tracing フォーマット文字列中の言及
/// （例: `tracing::debug!("... SendInput(...) ...")`）と、行末コメント中の
/// 言及（例: `foo(); // SendInput(...) の説明`）を除外する。
///
/// `SendInput`/`SendMessageTimeoutW` は関数定義ではなく Win32 API 名なので
/// `fn xxx(` 形の自己定義除外は不要な一方、これらのシンボル名はログメッセージ
/// （`ime.rs`/`held_modifiers.rs` 等）や doc コメント・行末コメント中の説明で
/// 頻出する。同じ行の needle 出現位置より前に `"`（文字列リテラル開始）または
/// `//`（行末コメント開始）があれば、コード上の実呼び出しではないとみなして
/// 除外する。
///
/// 既知の未対応ケース（実装レビュー指摘m2、現在のコードベースには該当なし
/// だが将来のfalse positive/negativeとして記録しておく）: ブロックコメント
/// （`/* ... SendInput( ... */`）、同一行に文字列リテラルが needle より
/// **前**にある実呼び出し（`line[..pos]`に`"`が誤って含まれ見逃す）、
/// 1行に needle が複数回出現するケース（行単位でしか数えない）。
/// `production_code_only` 自体の限界（`#[cfg(test)] mod tests` 以外の名前の
/// テストモジュールは本番扱いになる）もこの関数固有ではなく本ファイル全体の
/// 既存の制約を継承する。
fn count_real_calls_excluding_string_literals(content: &str, needle: &str) -> usize {
    content
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .filter(|line| {
            line.find(needle).is_some_and(|pos| {
                let prefix = &line[..pos];
                !prefix.contains('"') && !prefix.contains("//")
            })
        })
        .count()
}

/// ADR-140 Step1 決定G: `SendInput(` / `SendMessageTimeoutW(` の生産コード
/// 呼び出しは、OS に到達する唯一の物理境界（`win32.rs::send_input_safe` /
/// `imm.rs::send_ime_control`）にそれぞれ1箇所だけであることを固定する。
///
/// # なぜ必要か
///
/// ADR-140 決定B（`crate::probe_actuation_fence`）は「物理境界は単一
/// チョークポイント」という前提の上に、bump 地点をここ2箇所だけに置くことで
/// 未発見の第4・第5 actuation 経路（確定事実5が「少なくとも3つ確認、全てとは
/// 限らない」と明記）を気にせず済ませている。この前提が崩れる（新しい
/// `SendInput`/`SendMessageTimeoutW` 直接呼び出しが別ファイルに増える）と、
/// フェンスが一部の actuation を検出できなくなり、issue #136 型の
/// 「1箇所塞いで別箇所に穴」を再演する。
///
/// `imm.rs:127-130` のコメントは既に「本クレートの全 `SendMessageTimeoutW`
/// 呼び出しは本関数を経由する唯一のチョークポイント」と宣言しているが、
/// それを固定するテストは無かった（本テストが最初）。
#[test]
fn send_input_and_send_message_timeout_w_have_single_production_call_site() {
    let files = list_src_files();

    let mut send_input_sites: Vec<(String, usize)> = Vec::new();
    let mut send_message_timeout_w_sites: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let send_input_count = count_real_calls_excluding_string_literals(production, "SendInput(");
        if send_input_count > 0 {
            send_input_sites.push((path.clone(), send_input_count));
        }
        let send_message_timeout_w_count =
            count_real_calls_excluding_string_literals(production, "SendMessageTimeoutW(");
        if send_message_timeout_w_count > 0 {
            send_message_timeout_w_sites.push((path.clone(), send_message_timeout_w_count));
        }
    }
    send_input_sites.sort();
    send_message_timeout_w_sites.sort();

    assert_eq!(
        send_input_sites,
        vec![("src/win32.rs".to_string(), 1)],
        "`SendInput(` の生産コード呼び出しは `win32.rs::send_input_safe` の\
         1箇所のみに固定されています（ADR-140 決定B/G）。実際: {send_input_sites:?}\n\
         新しい呼び出しを追加する場合は `send_input_safe` 経由にすること\
         （さもないと probe_actuation_fence の bump がその actuation を検出できない）。"
    );
    assert_eq!(
        send_message_timeout_w_sites,
        vec![("src/imm.rs".to_string(), 1)],
        "`SendMessageTimeoutW(` の生産コード呼び出しは `imm.rs::send_ime_control` の\
         1箇所のみに固定されています（ADR-140 決定B/G）。実際: {send_message_timeout_w_sites:?}\n\
         新しい呼び出しを追加する場合は `send_ime_control` 経由にすること\
         （さもないと probe_actuation_fence の bump がその actuation/probe を検出できない）。"
    );
}

/// conv 軸の書き込み経路の件数を固定する（09 T6、`docs/tasks/conv-write-paths-inventory.md`）。
///
/// T5 の棚卸しで、conv 軸を書く経路は次の2つの入口に集約されると確認した。新しい呼び出し元を
/// 足すと、棚卸しの表（A 撤去候補・B 正当な例外・C warmup）に載らない書き込みが増える。
/// 撤去が目的の ADR-191 決定5に反する追加を、件数の増加で気づけるようにする。
/// 意図した追加・撤去のときは、この件数と棚卸しの表を同じコミットで更新すること。
///
/// - `modify_conv_mode(`（`IMC_SETCONVERSIONMODE` の唯一の書き手）: `ime.rs` の3入口のみ。
/// - `set_ime_conv_for_target(`: 5か所（cold-start の ROMAN 保護、`actuate_conv_mode`、
///   Ctrl+変換のリセット、半角英数トグルの復元、焦点プローブのかなモード修正）。
#[test]
fn conv_write_call_sites_are_fixed_to_the_inventory() {
    let files = list_src_files();
    let count_sites = |needle: &str| -> Vec<(String, usize)> {
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
        sites
    };

    assert_eq!(
        count_sites("modify_conv_mode("),
        vec![("src/ime.rs".to_string(), 3)],
        "`modify_conv_mode(` の本番呼び出し元は `ime.rs` の3入口（`set_ime_romaji_mode_for_hwnd`・\
         `set_ime_hiragana_mode_cross_process`・`set_ime_mode_for_target`）に固定されています。\
         新しい入口を足すなら `docs/tasks/conv-write-paths-inventory.md` の表を更新すること。"
    );

    assert_eq!(
        count_sites("set_ime_conv_for_target("),
        vec![
            ("src/output/conv_actuation.rs".to_string(), 1),
            ("src/runtime/key_pipeline.rs".to_string(), 2),
            ("src/tsf/warmup/cold_warmup.rs".to_string(), 1),
        ],
        "`set_ime_conv_for_target(` の本番呼び出し元は4か所に固定されています\
         （`docs/tasks/conv-write-paths-inventory.md` の経路3・4・5・8。経路9=焦点プローブの\
         ROMAN 修正は 2026-09-26 に撤去済みで、ここに戻さないこと）。\
         増やすなら棚卸しの表に分類（A 撤去候補／B 例外／C warmup）を書いて、この件数を更新すること。\
         撤去したなら件数を減らすこと。"
    );
}

/// BUG-163: 授権（warrant）が下りない drift 補正は、「検知」（journal・`DriftDetected`・バルーン通知）へ進めない。
///
/// `ir_apply_drift_correction` は、ImmCross の書き込み経路（`set_ime_open_ordered`、ADR-090 A-2 で
/// `Unwarranted` を拒否する）で書けない補正を、journal・`DriftDetected`（`applied` を `Optimistic` に
/// 偽装する）・バルーン通知へ流していた。
///
/// FCIS F4 で判断が `state/drift_plan.rs::decide_drift_plan` へ移った。固定する性質は 3 つ:
/// 1. observe が `.would_have_blocked()` を読む（授権の事実が `DriftFacts` に載る）。
/// 2. `decide_drift_plan` は、授権が下りない ImmCross を `SkipWarrantWouldBlock`（診断なし）で返す
///    分岐を、診断の決定（`should_notify_diagnostic`）・方針ごとの打ち切り/収束/送信（`SendPath`）より**前**に持つ。
/// 3. execute は `SkipWarrantWouldBlock` の腕で return し、journal・`DriftDetected`・`set_ime_open_ordered` は
///    その腕より後ろにしか無い。
#[test]
fn drift_correction_does_not_detect_when_the_warrant_would_block() {
    let content = read_crate_file("src/runtime/ime_refresh.rs");
    let production = production_code_only(&content);
    let observe = extract_fn_body(production, "fn ir_observe_drift_facts");
    assert!(
        observe.contains(".would_have_blocked()"),
        "`ir_observe_drift_facts` に `would_have_blocked()` の読み取りが必要（BUG-163）"
    );

    let plan_src = read_crate_file("src/state/drift_plan.rs");
    let plan_prod = production_code_only(&plan_src);
    let decide = extract_fn_body(plan_prod, "pub fn decide_drift_plan");
    let guard = decide
        .find("f.imm_cross && f.warrant_would_block")
        .expect("`decide_drift_plan` に授権が下りない ImmCross の早期分岐が必要（BUG-163）");
    for later in [
        "should_notify_diagnostic(",
        "SendPath::ImmCross",
        "FeedbackPolicy::Blind",
    ] {
        let at = decide
            .find(later)
            .unwrap_or_else(|| panic!("`{later}` が `decide_drift_plan` に無い"));
        assert!(
            guard < at,
            "授権の早期分岐は `{later}` より前になければならない（BUG-163）"
        );
    }
    assert!(
        decide[guard..].contains("notify_diagnostic: false")
            && decide[guard..].contains("DriftStep::SkipWarrantWouldBlock"),
        "授権が下りない補正は診断なしの `SkipWarrantWouldBlock` にする（BUG-163）"
    );

    let body = extract_fn_body(production, "fn ir_apply_drift_correction");
    let skip_arm = body
        .find("DriftStep::SkipWarrantWouldBlock =>")
        .expect("execute に `SkipWarrantWouldBlock` の腕が必要（BUG-163）");
    for later in [
        "ImeEvent::DriftDetected",
        "JournalEntry::ImeActuation",
        "set_ime_open_ordered(",
    ] {
        let at = body
            .find(later)
            .unwrap_or_else(|| panic!("`{later}` が `ir_apply_drift_correction` に無い"));
        assert!(
            skip_arm < at,
            "`SkipWarrantWouldBlock` の腕（return）は `{later}` より前になければならない（BUG-163）"
        );
    }
    assert!(
        drift_diagnostic_call_is_guarded(body),
        "診断バルーンの呼び出しは `if act.notify_diagnostic {{` の内側になければならない\
         （外すと毎 tick バルーンが出る。ADR-132 / BUG-163）"
    );
}

/// `ir_notify_drift_giveup_diagnostic(` の呼び出しが、直前の `if act.notify_diagnostic {` の
/// 直下（空白のみを挟む）にあるか。継続時間・通知済みの判定は関数側から `decide_drift_plan` へ移ったので、
/// この `if` が唯一の防波堤である。
fn drift_diagnostic_call_is_guarded(body: &str) -> bool {
    let Some(at) = body.find("ir_notify_drift_giveup_diagnostic(") else {
        return false;
    };
    let head = body[..at].trim_end();
    // `self.` の前置きを除いてから、直前が `if` の開き括弧であることを見る。
    let head = head.strip_suffix("self.").unwrap_or(head).trim_end();
    head.ends_with("if act.notify_diagnostic {")
}

/// 診断バルーンの呼び出し（`.ir_notify_drift_giveup_diagnostic(`、定義の `fn ` を除く）の件数。
fn count_drift_diagnostic_calls(text: &str) -> usize {
    text.matches(".ir_notify_drift_giveup_diagnostic(").count()
}

/// 診断の呼び出しは全域で 1 件（`ir_apply_drift_correction` の `if act.notify_diagnostic` の内側だけ）。
/// 2 件目や別関数からの呼び出しは、上の「最初の 1 件が守られているか」の照合では見えない。
#[test]
fn drift_diagnostic_is_called_from_exactly_one_site() {
    let mut files = Vec::new();
    walk_rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut sites = Vec::new();
    for f in files {
        let content = production_code_only(&fs::read_to_string(&f).unwrap_or_default()).to_string();
        let n = count_drift_diagnostic_calls(&content);
        if n > 0 {
            sites.push((f.display().to_string(), n));
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "診断の呼び出し元は ir_apply_drift_correction の 1 箇所だけ: {sites:?}"
    );
    assert!(
        sites[0].0.ends_with("ime_refresh.rs") && sites[0].1 == 1,
        "{sites:?}"
    );
}

#[test]
fn drift_diagnostic_call_count_detects_a_second_call() {
    assert_eq!(
        count_drift_diagnostic_calls("self.ir_notify_drift_giveup_diagnostic(a);"),
        1
    );
    assert_eq!(
        count_drift_diagnostic_calls(
            "self.ir_notify_drift_giveup_diagnostic(a);\nself.ir_notify_drift_giveup_diagnostic(b);"
        ),
        2
    );
    assert_eq!(
        count_drift_diagnostic_calls("fn ir_notify_drift_giveup_diagnostic("),
        0
    );
}

#[test]
fn drift_diagnostic_guard_detects_an_unguarded_call() {
    let guarded =
        "if act.notify_diagnostic {\n    self.ir_notify_drift_giveup_diagnostic(a, b);\n}";
    assert!(drift_diagnostic_call_is_guarded(guarded));
    let unguarded = "self.ir_notify_drift_giveup_diagnostic(a, b);";
    assert!(!drift_diagnostic_call_is_guarded(unguarded));
    let wrong_cond = "if other {\n    self.ir_notify_drift_giveup_diagnostic(a, b);\n}";
    assert!(!drift_diagnostic_call_is_guarded(wrong_cond));
    let after_if =
        "if act.notify_diagnostic {\n    x();\n}\nself.ir_notify_drift_giveup_diagnostic(a, b);";
    assert!(!drift_diagnostic_call_is_guarded(after_if));
    assert!(!drift_diagnostic_call_is_guarded("nothing"));
}

/// BUG-163（代案A）: 起動時の初期値のままの `desired_open` は、awase の意図ではない。
///
/// - フォーカス時の先同期（`applied` の `record_confirmed(true)` と GJI への ImeOn 通知＝long-cold の
///   `VK_IME_OFF→VK_IME_ON`）は、`desired_is_placeholder` の間は行わない（IME を閉じて起動したとき awase が開けない）。
///   代わりに、最初の成功観測が「開」だったとき `ir_align_placeholder_desired` が同じ処理（`presync_applied_open_on`）を行う。
/// - 揃え（`ir_align_placeholder_desired`）は、drift 補正（`ir_apply_drift_correction`）より前に呼ぶ。
#[test]
fn startup_placeholder_desired_is_not_treated_as_intent() {
    let focus = read_crate_file("src/runtime/focus_tracking.rs");
    let focus_prod = production_code_only(&focus);
    let calls: Vec<usize> = focus_prod
        .match_indices("self.presync_applied_open_on(")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "フォーカス時の先同期は `presync_applied_open_on` 経由の1箇所だけ"
    );
    let head = &focus_prod[..calls[0]];
    let guard_at = head
        .rfind("desired_is_placeholder()")
        .expect("先同期の直前に `desired_is_placeholder()` の判定が必要（BUG-163）");
    assert!(
        calls[0] - guard_at < 300,
        "先同期は `desired_is_placeholder()` の間は行わない（BUG-163）。判定が先同期の直前にあること"
    );

    let refresh = read_crate_file("src/runtime/ime_refresh.rs");
    let refresh_prod = production_code_only(&refresh);
    let align = refresh_prod
        .find("self.ir_align_placeholder_desired();")
        .expect("ir_stage で `ir_align_placeholder_desired` を呼ぶこと（BUG-163）");
    let drift = refresh_prod
        .find("self.ir_apply_drift_correction();")
        .expect("ir_stage で `ir_apply_drift_correction` を呼ぶ");
    assert!(
        align < drift,
        "`ir_align_placeholder_desired` は `ir_apply_drift_correction` より前に呼ぶこと（BUG-163）"
    );
    let body = extract_fn_body(refresh_prod, "fn ir_align_placeholder_desired");
    assert!(
        body.contains("if open && !tsf_native"),
        "揃えた値が「開」の非 TsfNative だけ、スキップした先同期を行う（閉なら行わない）"
    );
}

/// BUG-163: awase 自身のウィンドウ（警告ダイアログ等）へのフォーカスでは、
/// `reset_stale_ime_on_for_imm_broken`（安全デフォルト ON）も `assume_closed_for_new_thread`
/// （安全デフォルト OFF）も呼ばない。ON 側を呼ぶと、先同期と GJI への ImeOn 通知
/// （long-cold の `VK_IME_OFF→VK_IME_ON` reinit）へ進み、IME を閉じて起動したとき起動直後に awase が IME を開ける。
#[test]
fn stale_ime_on_heuristic_skips_awase_own_windows() {
    let content = read_crate_file("src/runtime/focus_tracking.rs");
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "fn on_focus_process_changed(");
    let guard_at = body
        .find("self.platform.focus.pid() == std::process::id()")
        .expect("呼び出しの前に awase 自身のウィンドウの除外が必要（BUG-163）");
    for method in [
        "assume_closed_for_new_thread(",
        "reset_stale_ime_on_for_imm_broken(",
    ] {
        assert_eq!(
            count_real_calls(production, method),
            1,
            "`{method}` の呼び出しは focus_tracking.rs 全体で1箇所だけ"
        );
        let calls: Vec<usize> = body.match_indices(method).map(|(i, _)| i).collect();
        assert_eq!(
            calls.len(),
            1,
            "`{method}` の呼び出しは on_focus_process_changed の1箇所だけ"
        );
        assert!(
            guard_at < calls[0]
                && calls[0] - guard_at < 1_500
                && body[guard_at..calls[0]].contains("} else {"),
            "`{method}` は awase 自身のウィンドウを除外する分岐の直近の else 側でのみ呼ぶこと（BUG-163）"
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
///    （= `decide_needs_romaji_pre_write` の条件判定を必ず通ること）。
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
             `decide_needs_romaji_pre_write` を迂回させないため、ADR-089 Phase C item 12）"
        );
    }
}

// ── BUG-78: disable_apps（アプリ単位の awase 無効化 + Ctrl/Shift スタック復旧） ──

/// `disable_apps` の早期 return（`hook_callback` 内、`HOOK_STATE.focus_app_disabled`
/// を見る分岐）はちょうど 1 箇所だけ存在し、`HOOK_STATE.physical_key_state`/
/// `HOOK_STATE.physical_key_down_at_ms` 更新ブロックより**後**、`VK_KANA` swallow
/// ブロックより**前**に置かれていること（ADR-164 フェーズ4で20静的を`HookState`
/// 構造体へ集約したが、フィールドの意味論・配置順は不変）。
///
/// 設計上の理由（`.claude/plans` の premortem 参照）: 更新ブロックより前に早期 return する
/// と、無効アプリに入る直前から押していたキーの KeyUp が記録されず、対策したい
/// Ctrl スタック自体をこの分岐が新規に生む。VK_KANA/Alt なりすまし等の変換系ロジックより
/// 前に置くことで、無効化中はそれらの介入も一切効かなくする（ユーザー判断により例外なし）。
#[test]
fn disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana() {
    let content = read_crate_file("src/hook.rs");
    let production = production_code_only(&content);

    let early_return_needle = "HOOK_STATE.focus_app_disabled.load(Ordering::Relaxed)";
    let count = production.matches(early_return_needle).count();
    assert_eq!(
        count, 1,
        "src/hook.rs 内で `{early_return_needle}` の出現数が想定(1)と異なります \
         (実際: {count})。disable_apps の早期 return は hook_callback 内の1箇所に \
         限定すること。"
    );

    let update_block_pos = production
        .find("if let Some(slot) = HOOK_STATE.physical_key_state.get(vk.0 as usize) {")
        .expect("HOOK_STATE.physical_key_state update block not found in src/hook.rs");
    let early_return_pos = production
        .find(early_return_needle)
        .expect("early return needle not found (checked above)");
    let vk_kana_pos = production
        .find("if vk == crate::vk::VK_KANA {")
        .expect("VK_KANA swallow block not found in src/hook.rs");

    assert!(
        update_block_pos < early_return_pos,
        "disable_apps の早期 return は HOOK_STATE.physical_key_state 更新ブロックより後に \
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
        // BUG-148/ADR-186: current_focus 初期化ヘルパー（同上、dispatch_event( 1件だけ例外）。
        ("sync_initial_focus_hwnd", "dispatch_event("),
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
        // BUG-148/ADR-186 で追加した current_focus 初期化ヘルパー。同じ理由で対象に加える。
        (
            "sync_initial_focus_hwnd",
            extract_fn_body(&content, "fn sync_initial_focus_hwnd"),
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

    // BUG-148/ADR-186: `sync_initial_focus_hwnd` も dispatch_event ちょうど1件、
    // `InitialFocusHwndEstablished` のみであること。
    let focus_hwnd_sync_body = extract_fn_body(&content, "fn sync_initial_focus_hwnd");
    assert_eq!(
        count_real_calls(focus_hwnd_sync_body, "dispatch_event("),
        1,
        "sync_initial_focus_hwnd の dispatch_event はちょうど1件（current_focus 初期化のみ）"
    );
    assert!(
        non_comment_lines(focus_hwnd_sync_body).contains("ImeEvent::InitialFocusHwndEstablished"),
        "sync_initial_focus_hwnd の唯一の dispatch は \
         ImeEvent::InitialFocusHwndEstablished であること"
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

/// ADR-187/191: `ImeEvent::ModeKeyPassedThrough` は `ImeStateHub::pass_through_observed` の1箇所だけが
/// dispatch する（呼び出し元は、観測成功時の `invalidate_intents_if_mode_key_pass_live`、窓の終了時の
/// `expire_mode_key_pass_mark`、窓が切れた後の最初の成功観測での `align_after_expired_mode_key_pass`）。
/// reducer は `last_intent` を捨て、`align_desired` かつ観測から導ける開閉があるときだけ `desired_open` を
/// それへ揃える（BUG-157。観測が成功しないまま窓が切れた破棄は `desired_open` を書かない）。
/// つまり `desired_open` を書ける口の1つなので、dylint `ime_event_guard` の designated 関数にも登録してある。
#[test]
fn mode_key_passed_through_event_is_dispatched_from_one_place() {
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
        let production = non_comment_lines(production_code_only(&content));
        let count = production.matches("ModeKeyPassedThrough").count();
        let expected = match rel.as_str() {
            "state/platform_state.rs" | "state/ime_model.rs" | "state/ime_event.rs" => 1,
            _ => 0,
        };
        assert_eq!(
            count, expected,
            "src/{rel} 内の ModeKeyPassedThrough の出現数が想定と異なります(期待: \
             {expected}, 実際: {count})。ADR-187 の dispatch 元は1箇所に限定すること。"
        );
    }
}

/// ADR-205（BUG-172）・ADR-227: 外部変化の監視窓は、arm が `kp_stage_post_decision` と `ir_follow_after_literal_giveup`（ADR-227 (i)）の各1箇所、追随（`follow_external_change`）が
/// `ir_follow_external_change` の1箇所だけ。追随は `ObserverPoll` の記録 + 意図削除 + `ModeKeyPassedThrough` で、
/// awase は IME を書かない（`apply_ime_open_*`/`set_ime_open`/`send_ime` 系をこのファイル群から呼ばない）。
#[test]
fn external_change_watch_has_single_arm_and_follow_sites() {
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
        let production = non_comment_lines(production_code_only(&content));
        // arm は 2 箇所: 外部注入の IME キー直後(`kp_arm_external_change_watch`、ADR-205)と、
        // give-up を契機にした読み直し(`ir_follow_after_literal_giveup`、ADR-227 (i))。追随は 1 箇所のまま
        // (give-up 契機の読みも既存の `ir_follow_external_change` に載る=新しい追随の入口は作らない)。
        for (needle, allowed) in [
            (
                ".arm_external_change_watch(",
                &["runtime/key_pipeline.rs", "runtime/ime_refresh.rs"][..],
            ),
            (".follow_external_change(", &["runtime/ime_refresh.rs"][..]),
            // FCIS P2: 殻は `_in_scope` 版をちょうど1回ずつ呼ぶ（殻を除外しない）。
            (
                ".arm_external_change_watch_in_scope(",
                &["state/platform_state/shell.rs"][..],
            ),
            (
                ".follow_external_change_in_scope(",
                &["state/platform_state/shell.rs"][..],
            ),
        ] {
            let count = production.matches(needle).count();
            let expected = usize::from(allowed.contains(&rel.as_str()));
            assert_eq!(
                count, expected,
                "src/{rel} 内の {needle} の出現数が想定({expected})と異なります。ADR-205/227: 呼び出し元は {allowed:?} の各1箇所に限定すること。"
            );
        }
    }
}

/// ADR-205（PR #377 Opus レビュー 1・2）: 外部変化の監視窓は Imm32Unavailable かつ GJI の窓だけに適用する。
/// arm 側（`kp_arm_external_change_watch`）と追随側（`ir_follow_external_change`）の両方が
/// `external_change_watch_applies` を通ること、その述語が両条件を持つことを固定する。
#[test]
fn external_change_watch_is_limited_to_imm32_unavailable_and_gji() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let read = |rel: &str| {
        non_comment_lines(production_code_only(
            &fs::read_to_string(Path::new(manifest_dir).join("src").join(rel)).unwrap(),
        ))
    };
    let mod_rs = read("runtime/mod.rs");
    let pred = mod_rs
        .split("fn external_change_watch_applies")
        .nth(1)
        .expect("述語が無い");
    let pred = &pred[..pred.find("\n    }\n").unwrap_or(pred.len())];
    assert!(pred.contains("AppImeProfile::Imm32Unavailable"), "{pred}");
    assert!(
        pred.contains("ActiveImeKind::GoogleJapaneseInput"),
        "{pred}"
    );
    assert!(read("runtime/key_pipeline.rs").contains("self.external_change_watch_applies()"));
    assert!(read("runtime/ime_refresh.rs").contains("self.external_change_watch_applies()"));
}

/// ADR-158 TE3 / PR #377 レビュー M6-1: `Runtime::can_use_imm32_cross_process` は `#[track_caller]` を持つ。
/// 直前に別の関数を挿入すると属性と doc だけが新しい関数へ移り、呼び出し元の棚卸しが黙って壊れる。
#[test]
fn can_use_imm32_cross_process_wrapper_keeps_track_caller() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = fs::read_to_string(Path::new(manifest_dir).join("src/runtime/mod.rs")).unwrap();
    let lines: Vec<&str> = src.lines().collect();
    let idx = lines
        .iter()
        .position(|l| l.contains("pub fn can_use_imm32_cross_process(&self)"))
        .expect("ラッパが無い");
    let prev = lines[idx - 1].trim();
    let prev2 = lines[idx - 2].trim();
    assert_eq!(prev, "#[track_caller]", "直前の行: {prev}");
    assert_eq!(prev2, "#[must_use]", "その前の行: {prev2}");
}

/// BUG-148/ADR-186: `ImeEvent::InitialFocusHwndEstablished` は bootstrap 専用であり、
/// dispatch 元は `sync_initial_focus_hwnd` の1箇所だけ。reducer 側のアームは
/// `self.current_focus = Some(hwnd)`（current_focus 1フィールドの差し替え）しか行わない。
///
/// `initial_app_policy_event_only_touches_app_policy` と同じ構造の監視テスト。
/// アーム本体が current_focus 以外に触れないことは
/// `state::ime_model::tests::initial_focus_hwnd_established_touches_only_current_focus`
/// が実行時に固定し、ここでは「増えていないこと」だけを見る。
#[test]
fn initial_focus_hwnd_event_only_touches_current_focus() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    let checks: &[(&str, &[(&str, usize)])] = &[(
        "InitialFocusHwndEstablished",
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
                 {expected_count}, 実際: {count})。BUG-148/ADR-186 参照。"
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
/// の4述語、`AppImeProfile::is_effectively_tsf_native` /
/// `AppImeProfile::cannot_verify_real_ime_state` /
/// `AppImeProfile::should_reprime_on_lightweight_focus_sync` の3メソッド
/// （2026-09-10、自由関数から`impl AppImeProfile`のメソッドへ移動）、
/// `From<AppImeProfile> for ImePolicyProfile`、`from_class_and_process` のいずれかで `InputRelay` の
/// 分岐が欠落していないか確認すること（欠落すると condition (b)/(c) が
/// 別経路から迂回されうる、査読で指摘された最重要ポイント）。
/// `production_code_only` は `#[cfg(test)]` の直後が文字どおり `mod tests` の
/// ときしか test module を切り落とせない。`runtime/transport.rs` は
/// `mod plan_shell_tests`（ADR-229 T1 前は `plan_tests`）という別名を使っているため、共有ヘルパーのままだと
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

/// ADR-153 決定1「ケース3」（"off"×belief既にOFFの強制actuate）は、ADR-206 で `*_solo_tap_ime_action` ごと撤去した。
/// 旧ケース3専用のアクチュエーション理由タグが復活しないことだけを固定する（強制 actuate が「@」の直接原因と
/// 確定した経緯は BUG-113/BUG-124、`docs/experiments.md` エントリ25）。ADR-206 の OFF 方向は、
/// エンジンの `SetOpen(false)`（`applied` の陽性証拠があれば `already_matches` で省略）であり、
/// `shadow_on: None` バイパスの毎回強制 actuate ではない。
#[test]
fn kp_stage_shadow_ime_toggle_never_reintroduces_case3_forced_actuate() {
    let content = read_crate_file("src/runtime/key_pipeline.rs");
    let production = production_code_only(&content);
    assert!(
        !production.contains("explicit_ime_action_case3_off"),
        "旧ケース3専用のアクチュエーション理由タグ`explicit_ime_action_case3_off`が再導入されています。\
         この設計は「beliefが変化しなくても毎回強制actuateする」ことが「@」再現の直接原因と確定済みです。"
    );
}

#[test]
fn input_relay_profile_wiring_occurrence_counts_are_pinned() {
    let expectations: &[(&str, usize)] = &[
        ("src/focus/class_names.rs", 12),
        // ADR-208 L0: `plan` の本体（InputRelay の早期 return）は `state/physical_disposition.rs::plan_core` へ
        // 挙動を変えずに移した。`transport.rs` の `plan` は殻（`InputRelay` を名指ししない）。合計は 2 のまま。
        ("src/runtime/transport.rs", 0),
        ("src/state/physical_disposition.rs", 2),
        // ADR-163 TH1b-2a: `executor.rs::dispatch_ime_set_open` の InputRelay
        // ゲートは、5箇所（この関数 + `ime_controller.rs::apply` +
        // `open_chain.rs`の3関数）に重複していた同一条件のリテラル比較を
        // `state::ime_actuation_decision::decide_gate`（ungated、TH1b-1で
        // 全数テスト済み）への呼び出しに置き換えた。このファイルの本番コード
        // からは `InputRelay` という識別子が消えるため 0 に更新する
        // （gate自体が消えたわけではないことは、直後の
        // `decide_gate_wiring_occurrence_counts_are_pinned` が
        // `decide_gate(` 呼び出しの残存を別途固定する）。
        ("src/runtime/executor.rs", 0),
        // `ime_controller.rs`/`open_chain.rs` も同様に `decide_gate` 経由に
        // 置き換わったが、周辺コメント（issue #136/BUG-90決定4の説明文）に
        // `InputRelay` の記述が残っているため件数は変化しない。
        // condition (a) を実際に担保しているのはこちらの2ファイル
        // （`ImeController::apply` / `run_open_chain_async` /
        // `fallback_write` の3箇所、コードレビューで gate 取りこぼしが
        // 見つかった経緯は ADR-119 参照）。ここが欠けると、今回踏んだのと
        // 同じクラスの退行（gate の一部消失）を検知できない。
        //
        // ADR-163 TH1b-2a: `ImeController::apply`冒頭のリテラル比較
        // （`view.focus.profile == AppImeProfile::InputRelay`）も
        // `decide_gate`呼び出しに置き換えたため、本番コードの`InputRelay`
        // 出現は直前の説明コメント（551行目付近）1件のみになる。
        ("src/ime_controller.rs", 1),
        // 同じく`imm_cross_write`/`fallback_write`/`run_open_chain_async`の
        // 3箇所のリテラル比較を`decide_gate`呼び出しへ置き換えたため、
        // 本番コードの`InputRelay`出現は7から4（周辺コメント分）に減る。
        ("src/runtime/open_chain.rs", 4),
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

#[test]
fn cross_thread_shared_lock_declarations_are_accounted_for() {
    // 裸の `static X: Mutex<...>`（`OnceLock`/`Arc`でラップされていない生の
    // ロック、例: hook.rs::HOOK_IME_MODE_DIAGNOSTICS）も対象に含める。ただし
    // `#[cfg(test)]`直後の宣言（OUTPUT_GATE_TEST_LOCK/TSF_OBS_TEST_LOCK等、
    // テストビルドにしか存在しないロック）は本番のクロススレッド共有状態
    // ではないため除外する。
    fn shared_lock_declaration_count(production: &str) -> usize {
        let mut count = 0;
        let mut prev_was_cfg_test = false;
        for line in production.lines().map(str::trim_start) {
            let is_static_decl =
                line.starts_with("static ") || line.starts_with("pub ") || line.starts_with("pub(");
            if is_static_decl
                && !prev_was_cfg_test
                && (line.contains("OnceLock<RwLock<")
                    || line.contains("OnceLock<Arc<Mutex<")
                    || line.contains(": RwLock<")
                    || line.contains(": Mutex<"))
            {
                count += 1;
            }
            if !line.is_empty() {
                prev_was_cfg_test = line == "#[cfg(test)]";
            }
        }
        count
    }

    let mut actual: Vec<(String, usize)> = list_src_files()
        .into_iter()
        .filter_map(|path| {
            let content = read_crate_file(&path);
            let production = production_code_only(&content);
            let count = shared_lock_declaration_count(production);
            (count > 0).then_some((path, count))
        })
        .collect();
    actual.sort();

    // ADR-164 フェーズ4（2026-09-12）: `src/hook.rs` の
    // `HOOK_IME_MODE_DIAGNOSTICS: Mutex<...>` は裸の top-level static から
    // `HookState` 構造体のフィールド（`ime_mode_diagnostics: Mutex<...>`）へ
    // 移行し、このテストが検出する「裸の `static X: Mutex<...>`」パターンには
    // もう一致しない（意図した変化——20静的を1つの singleton へ集約したことの
    // 直接の結果）。Mutex 自体が消えたわけではないことは、直後の
    // `hook_state_struct_has_exactly_one_mutex_field` が別途固定する。
    let mut expected: Vec<(String, usize)> = vec![
        ("src/app/logging.rs".to_string(), 1),
        ("src/focus/classifier.rs".to_string(), 1),
        ("src/tsf/observer.rs".to_string(), 1),
        ("src/tsf/tip_detector.rs".to_string(), 1),
    ];
    expected.sort();

    assert_eq!(
        actual, expected,
        "クロススレッド共有ロック宣言の出現箇所が想定と異なります。\
         意図した変更なら期待値を更新してください。"
    );

    for (path, checks) in [
        (
            "src/app/logging.rs",
            &[("static LOG_WRITER_STATE: OnceLock<Arc<Mutex<", 1)][..],
        ),
        (
            "src/focus/classifier.rs",
            &[("static INPUT_RELAY_APPS: OnceLock<RwLock<", 1)][..],
        ),
        (
            "src/tsf/observer.rs",
            &[("ime_product_name: RwLock<", 1)][..],
        ),
        (
            "src/tsf/tip_detector.rs",
            &[("static PROFILE_DESCRIPTIONS: RwLock<", 1)][..],
        ),
    ] {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        for (needle, expected) in checks {
            let count = production.matches(needle).count();
            assert_eq!(
                count, *expected,
                "{path} 内の `{needle}` 出現数が想定({expected})と異なります(実際: {count})。\
                 クロススレッド共有ロックを増減する場合は理由を確認し、この期待値を更新してください。"
            );
        }
    }
}

/// ADR-164 フェーズ4: `src/hook.rs` の20静的（`HookState`構造体、上記テストの
/// module doc参照）は Mutex を`ime_mode_diagnostics`フィールド1件に限定し、
/// 残り19フィールドはロックフリー struct-of-atomics であること。
///
/// `cross_thread_shared_lock_declarations_are_accounted_for` は「裸の
/// top-level static」しか見ないため、`HookState`構造体の**内部**にMutexが
/// いくつあるかは別途固定する必要がある——`HOOK_IME_MODE_DIAGNOSTICS`が
/// `HookState`へ集約された際にこの検出力の欠落が判明した（同テストのコメント
/// 参照）。将来誰かが `HookState` へ2件目の `Mutex` フィールドを追加した場合
/// （`WH_KEYBOARD_LL`のホットパスでロックを増やすと`LowLevelHooksTimeout`
/// サイレント解除のリスクが増える、ADR-164フェーズ4「訂正3」参照）、この
/// テストが検知する。
#[test]
fn hook_state_struct_has_exactly_one_mutex_field() {
    let content = read_crate_file("src/hook.rs");
    let production = production_code_only(&content);
    let body = extract_fn_body(&content, "struct HookState");

    let struct_mutex_count = body.matches("Mutex<").count();
    assert_eq!(
        struct_mutex_count, 1,
        "src/hook.rs::HookState 構造体内の `Mutex<` 出現数が想定(1)と異なります \
         (実際: {struct_mutex_count})。ホットパスで新たな Mutex フィールドを \
         追加していないか確認すること（ADR-164フェーズ4「訂正3」: \
         WH_KEYBOARD_LLはLowLevelHooksTimeout内に返らないとフックがサイレントに \
         外れるため、ime_mode_diagnostics以外へのMutex追加は原則禁止）。"
    );

    let total_mutex_count = production.matches("Mutex<").count();
    assert_eq!(
        total_mutex_count, 1,
        "src/hook.rs 全体の `Mutex<` 出現数が想定(1)と異なります(実際: \
         {total_mutex_count})。HookState外に新たなMutexを追加していないか確認すること。"
    );
}

/// ADR-163 TH1b-2a: 上記テストが`executor.rs`で追えなくなった
/// InputRelayゲートの存在を、`decide_gate(`呼び出し箇所の件数で改めて固定する。
///
/// `state::ime_actuation_decision::decide_gate`はissue #136/BUG-90決定4の
/// InputRelayゲートを1箇所に集約した purely 関数（TH1b-1）。5箇所の呼び出し元
/// （`ImeController::apply`・`dispatch_ime_set_open`・`open_chain.rs`の
/// `imm_cross_write`/`fallback_write`/`run_open_chain_async`）のうち1つでも
/// 削除されると、上記テストの`InputRelay`文字列カウント（コメント由来で
/// 見かけ上は変化しないファイルもある）だけでは検知できない
/// ——本テストが呼び出し件数そのものを見て埋め合わせる。
///
/// **ADR-180決定1（2026-09-19）**: `open_chain.rs`の3箇所は`decide_gate(`を
/// 直接呼ぶ代わりに、共有ヘルパー`ime_actuation_decision::is_input_relay(`を
/// 呼ぶ形へ統合した（`with_app`を内包しない、round1 E2形）。ファイル別の
/// `decide_gate(`出現数だけを見ると`open_chain.rs`が3→0になり、3つの
/// `.await`境界のうち1つがgate呼び出しを失っても検知できなくなる
/// （round1 C6が指摘した退行）。そのため`open_chain.rs`側は関数別に
/// `is_input_relay(`の出現数を固定する形へ作り替えた。
#[test]
fn decide_gate_wiring_occurrence_counts_are_pinned() {
    let direct_decide_gate: &[(&str, usize)] =
        &[("src/ime_controller.rs", 1), ("src/runtime/executor.rs", 1)];
    for (path, expected) in direct_decide_gate {
        let content = read_crate_file(path);
        let production = strip_any_test_module(&content);
        let count = production.matches("decide_gate(").count();
        assert_eq!(
            count, *expected,
            "{path} 内で `ime_actuation_decision::decide_gate(` 呼び出しの \
             本番コードでの出現数が想定({expected})と異なります(実際: {count})。\
             InputRelayゲート（issue #136/BUG-90決定4）の呼び出し元が \
             増減していないか確認すること。意図した変更ならこのテストの \
             期待値を更新すること。"
        );
    }

    // open_chain.rs: 3つの`.await`境界それぞれが`is_input_relay(`を
    // 関数本体内でちょうど1回呼んでいることを固定する（ADR-180決定1）。
    let open_chain_rs = read_crate_file("src/runtime/open_chain.rs");
    let production = strip_any_test_module(&open_chain_rs);
    let per_fn_expectations: &[(&str, usize)] = &[
        ("fn imm_cross_write", 1),
        ("fn fallback_write", 1),
        ("fn run_open_chain_async", 1),
    ];
    for (fn_signature_needle, expected) in per_fn_expectations {
        let body = extract_fn_body(production, fn_signature_needle);
        let count = body.matches("is_input_relay(").count();
        assert_eq!(
            count, *expected,
            "src/runtime/open_chain.rs の `{fn_signature_needle}` 内で \
             `is_input_relay(` 呼び出しの出現数が想定({expected})と異なります \
             (実際: {count})。この関数の`.await`境界でInputRelayゲートの \
             再検証が失われていないか確認すること。"
        );
    }
    // 上記3関数以外にis_input_relay(が漏れ出していないかも固定する
    // （合計4件: 上記3 + `ime_actuation_decision.rs`自身の定義1件）。
    let total_in_open_chain = production.matches("is_input_relay(").count();
    assert_eq!(
        total_in_open_chain, 3,
        "src/runtime/open_chain.rs 全体での `is_input_relay(` 呼び出し数が \
         想定(3)と異なります(実際: {total_in_open_chain})。新しい呼び出し元が \
         増えた場合は上記per-fn期待値にも追加すること。"
    );
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
// ── `half_width_alnum` 機能カプセル化（旧 `GateStore` 4フィールド + 旧
//    `Runtime::half_width_alnum_toggle_policy` の統合） ──────────────────

/// `HalfWidthAlnumState` の5フィールドへの本番コードからの直接アクセスが
/// 0件であること、および宣言側で再び `pub` 化されていないことを固定する。
///
/// # なぜ `per_source_fields_are_not_assigned_directly` のような行単位一致を
/// そのまま使わないのか
///
/// `runtime/ime_refresh.rs` の journal record 構築のように、rustfmt が
/// フィールド初期化子を
///
/// ```ignore
/// half_width_alnum_toggle_active: self
///     .platform_state
///     .gate
///     .half_width_alnum_toggle_active,
/// ```
///
/// のように複数行へ折り返す実例が既にあった（本PRで修正済みだが、退行検知の
/// ためにこの形も拾えるようにする）。`build_input_context_callers_do_not_drop_thumb_down_state`
/// と同じ手法（`split_whitespace().collect()` で空白を全て除去してから
/// 部分文字列一致を見る）を使うことで、インデント幅・改行位置に依存せず
/// `.half_width_alnum.left_tap_armed` のような生アクセスを検出する。
#[test]
fn half_width_alnum_state_fields_are_not_accessed_directly() {
    const FIELDS: [&str; 5] = [
        "left_tap_armed",
        "right_tap_armed",
        "conv_guard_pending",
        "toggle_held",
        "entry_policy",
    ];

    // 1. 使用箇所走査: 本番コード全体（`state/half_width_alnum.rs` 自身の
    //    実装は除く——フィールドを実際に読み書きしてよいのはこのファイルの
    //    メソッド本体だけ）。
    let files = list_src_files();
    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        if path == "src/state/half_width_alnum.rs" {
            continue;
        }
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let squashed: String = production.split_whitespace().collect();
        for field in FIELDS {
            let needle = format!(".half_width_alnum.{field}");
            let count = squashed.matches(&needle).count();
            if count > 0 {
                hits.push(format!("{path}: {needle} x{count}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "`HalfWidthAlnumState` のフィールドへ本番コードから直接触らないこと \
         （state/half_width_alnum.rs のメソッド経由に限定すること）。実際: {hits:?}"
    );

    // 2. 宣言側走査（再pub化検知）。`pub field` の素朴なリテラル一致だと
    //    `pub(crate) field` のような可視性修飾子付きの再宣言を素通りして
    //    しまうため、修飾子の有無を問わず検出する `declares_pub_field` を使う。
    let source = read_crate_file("src/state/half_width_alnum.rs");
    for field in FIELDS {
        assert!(
            !declares_pub_field(&source, field),
            "`HalfWidthAlnumState::{field}` が `pub`（`pub(crate)` 等の \
             可視性修飾子付きを含む）フィールドとして宣言されています。\
             private のまま維持すること。"
        );
    }
}

/// `source` の中でフィールド `field` が `pub`（修飾子なし）または
/// `pub(crate)`/`pub(super)`/`pub(in ...)` のような可視性修飾子付きの
/// `pub` として宣言されているかを判定する。
///
/// 空白の量・改行位置に依存しないよう、比較前に全ての空白を除去する
/// （`half_width_alnum_state_fields_are_not_accessed_directly` の使用箇所
/// 走査、および `build_input_context_callers_do_not_drop_thumb_down_state`
/// と同じ手法）。素朴な `contains("pub {field}")` は `pub(crate) {field}`
/// のような修飾子付き再宣言を検出できない（M4: Opus敵対的レビュー指摘）。
fn declares_pub_field(source: &str, field: &str) -> bool {
    let squashed: String = source.split_whitespace().collect();
    // 修飾子なし: `pub left_tap_armed` → squash後 `publeft_tap_armed`。
    if squashed.contains(&format!("pub{field}")) {
        return true;
    }
    // `pub(crate)`/`pub(super)`/`pub(in a::b)` 等の修飾子付き:
    // squash後は `pub(...)left_tap_armed` の形になる。`pub(` に対応する
    // `)` までをスキップしてから直後が `field` かを見る。
    let mut rest = squashed.as_str();
    while let Some(idx) = rest.find("pub(") {
        let after_open = &rest[idx + "pub(".len()..];
        let Some(close_idx) = after_open.find(')') else {
            break;
        };
        let after_close = &after_open[close_idx + 1..];
        if after_close.starts_with(field) {
            return true;
        }
        rest = after_close;
    }
    false
}

/// `declares_pub_field` 自体の回帰テスト（M4: 素朴な `contains("pub {field}")`
/// は `pub(crate)` 修飾子付きの再宣言を検出できなかった、という指摘の再発防止）。
#[test]
fn declares_pub_field_detects_qualified_visibility() {
    assert!(declares_pub_field(
        "pub left_tap_armed: bool,",
        "left_tap_armed"
    ));
    assert!(declares_pub_field(
        "pub(crate) toggle_held: bool,",
        "toggle_held"
    ));
    assert!(declares_pub_field(
        "pub(super) entry_policy: Policy,",
        "entry_policy"
    ));
    assert!(!declares_pub_field(
        "toggle_held: bool, // not pub",
        "toggle_held"
    ));
    assert!(!declares_pub_field("pub other_field: bool,", "toggle_held"));
}

/// `config.general.use_learned_keymap_table`・`predict_henkan_open_in_unreadable_windows` の反映が、起動時（`app/bootstrap.rs`）と
/// 設定リロード時（`Runtime::apply_config_update`）の**両方**から setter 経由で呼ばれていることを固定する。
/// 以前は再読込でしか反映されず、起動時は既定の true のままだった（opt-out が効かない。ADR-209 の CI の対照が PASS してしまい発覚）。
#[test]
fn general_keymap_prediction_flags_are_wired_at_bootstrap_and_reload() {
    let bootstrap = read_crate_file("src/app/bootstrap.rs");
    let bootstrap_production = non_comment_lines(production_code_only(&bootstrap));
    let runtime_mod = read_crate_file("src/runtime/mod.rs");
    let reload_body = extract_fn_body(&runtime_mod, "pub(crate) fn apply_config_update(");
    for setter in [
        "set_use_learned_keymap_table(",
        "set_predict_henkan_open_in_unreadable_windows(",
    ] {
        assert_eq!(
            count_real_calls(&bootstrap_production, setter),
            1,
            "src/app/bootstrap.rs は起動時に `{setter}...)` をちょうど1回呼ぶこと"
        );
        assert_eq!(
            count_real_calls(reload_body, setter),
            1,
            "Runtime::apply_config_update は設定リロード時に `{setter}...)` をちょうど1回呼ぶこと"
        );
    }
}

/// `config.general.half_width_alnum_toggle` の反映（`Runtime::
/// set_half_width_alnum_toggle_policy`）が、起動時（`app/bootstrap.rs`）と
/// 設定リロード時（`Runtime::apply_config_update`）の**両方**から呼ばれて
/// いることを固定する（BUG-103と同型の「片方の経路だけ配線し忘れる」
/// 再発ファミリー対策）。
///
/// `every_platform_entry_point_calls_apply_general_config_after_nicola_fsm_new`
/// と同じ手法（呼び出し回数を数える）を使う。
#[test]
fn half_width_alnum_toggle_policy_is_wired_at_bootstrap_and_reload() {
    let bootstrap = read_crate_file("src/app/bootstrap.rs");
    let bootstrap_production = non_comment_lines(production_code_only(&bootstrap));
    let bootstrap_count =
        count_real_calls(&bootstrap_production, "set_half_width_alnum_toggle_policy(");
    assert_eq!(
        bootstrap_count, 1,
        "src/app/bootstrap.rs は起動時に \
         `set_half_width_alnum_toggle_policy(...)` をちょうど1回呼ぶこと \
         （実際: {bootstrap_count}）"
    );

    let runtime_mod = read_crate_file("src/runtime/mod.rs");
    // `apply_config_update` の本体内で呼ばれていることまで確認する
    // （定義とは別の箇所に呼び出しがあるだけでは reload 時の反映を保証しない）。
    let apply_config_update_body =
        extract_fn_body(&runtime_mod, "pub(crate) fn apply_config_update(");
    let reload_count = count_real_calls(
        apply_config_update_body,
        "set_half_width_alnum_toggle_policy(",
    );
    assert_eq!(
        reload_count, 1,
        "Runtime::apply_config_update は設定リロード時に \
         `set_half_width_alnum_toggle_policy(...)` をちょうど1回呼ぶこと \
         （実際: {reload_count}）"
    );

    // `set_half_width_alnum_toggle_policy` メソッド自体は
    // `HalfWidthAlnumState::set_policy` へのデリゲートであること
    // （B1対応: メソッド自体を削除せず残す）。
    let setter_body = extract_fn_body(
        &runtime_mod,
        "pub(crate) fn set_half_width_alnum_toggle_policy(",
    );
    assert_eq!(
        count_real_calls(setter_body, "half_width_alnum.set_policy("),
        1,
        "Runtime::set_half_width_alnum_toggle_policy は \
         `self.platform_state.gate.half_width_alnum.set_policy(policy)` へ \
         デリゲートすること（実際の本体: {setter_body:?}）"
    );
}

/// ADR-191/199: `plan()` の DBE 分岐が KeyDown を無条件に握りつぶすのは「awase が実際に書くキー」だけ
/// （`enrich_key_role` が役割から `Some(Toggle)` を付けた 0xF3/0xF4。旧 `is_open_toggle_for`）であること、および
/// BUG-116/ADR-137 決定2 の安全ガードが本番コードから消えていないことを固定する。
/// 配送判断の決定表(BUG-116 の Shift+かな を含む)は `state/physical_disposition.rs` の `mod tests` が Linux でも
/// 検査する(ADR-229 T1)。このスキャンは、その決定表が頼る本番側のトークン(役割由来の `Toggle` と 0xF3/0xF4 の組など)が
/// 本番コードに残っていることと、`key_pipeline.rs`(`runtime/` は `#[cfg(windows)]` で、そのユニットテストは Linux に
/// 存在しない、CLAUDE.md 参照)側の安全ガードを固定する補助。
#[test]
fn bug116_shift_katakana_guards_are_present_in_production_code() {
    // 配送判断の核は ADR-208 L0 で `state/physical_disposition.rs` へ移した（`transport.rs` の `plan` は殻）。
    // 両方を連結して走査する（トークンの有無を見るだけなので、どちらにあってもよい）。
    let transport = format!(
        "{}\n{}",
        strip_any_test_module(&read_crate_file("src/runtime/transport.rs")),
        strip_any_test_module(&read_crate_file("src/state/physical_disposition.rs")),
    );
    let transport = transport.as_str();
    // Suppress の根拠は「役割由来の `Some(Toggle)` と 0xF3/0xF4 の組」（ADR-199 T4）。どちらかが消えると
    // VK だけ（または shadow_action だけ）で握りつぶす形に退行し、awase が書かないキーを Suppress しうる。
    for token in [
        "ShadowImeAction::Toggle",
        "ImeKeyKind::DbeSbcsChar",
        "ImeKeyKind::DbeDbcsChar",
        // ADR-199 決定18(iii): F13〜F24 は「最初の Down で実際に書いた打鍵だけ」Suppress する専用の分岐
        // （ImmCross の無条件 Suppress より前）。消えると書かない打鍵が二重の空振り・Up 欠落になる。
        "Self::thumb_or_role_fkey_disposition(event, shadow_toggled)",
        "role-fkey",
    ] {
        assert!(
            transport.contains(token),
            "runtime/transport.rs の本番コードから `{token}` が消えています。\
             Suppress の対象は「awase が書くキー（役割由来の `Some(Toggle)` の 0xF3/0xF4）」だけに\
             する（ADR-191/199）"
        );
    }
    assert!(
        !transport.contains("is_open_toggle_for"),
        "runtime/transport.rs の本番コードに撤去済みの `is_open_toggle_for` が再び現れています（ADR-199 T4）"
    );
    // 撤去後は awase が書かない英数(0xF0)・カタカナ(0xF1)を VK で列挙して Suppress してはならない
    // （握りつぶすと OS にも awase にも誰も何もしない二重の空振りになる）。BUG-116 の
    // Shift+0xF1 の特例（`shift_katakana_passthrough`）も、0xF1 が常に Allow になったため撤去済み。
    for token in [
        "VK_DBE_ALPHANUMERIC",
        "VK_DBE_KATAKANA",
        "fn shift_katakana_passthrough",
        "DbeModeKeyContext",
        // 設定 `dbe_mode_key_policy` は撤去済み（Passthrough が実質死んでいたため、B-M3）。
        // 復活させるなら 0xF3/0xF4 の `shadow_toggled` Suppress との関係を決め直すこと。
        "DbeModeKeyPolicy",
    ] {
        assert!(
            !transport.contains(token),
            "runtime/transport.rs の本番コードに `{token}` が再び現れています（ADR-191: \
             Suppress の対象は役割由来の `shadow_action` で決め、awase が書かないキーを VK 列挙で \
             握りつぶさない）"
        );
    }

    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    let kp = strip_any_test_module(&kp);
    for token in [
        "is_configured_thumb_key",
        "read_kana_lock",
        "conv_mutation_allowed",
        // ADR-199 T4: 役割由来の `shadow_action` は `kp_run_inner` の冒頭（`kp_stage_shadow_ime_toggle`・
        // `plan()` より前）で付く。この呼び出しが消えると 0xF3/0xF4 が `shadow_action` なしで
        // Allow され、awase の書き込みと生キーの二重 actuation（BUG-46/BUG-52）に退行する。
        "self.enrich_key_role(&mut event)",
        // ADR-199 決定16: 無変換/変換の役割由来の open 軸操作は、`engine.on_input` より前に打鍵ごとに設定し直す。
        // 消えると GJI CUSTOM で無変換/変換をトグルにしたユーザーの単独タップが受動のまま（または古い役割が残る）。
        // key_pipeline 自体は `forced_open_action` を名指ししない（下の別ガード）ので、呼び出し名だけを固定する。
        "self.enrich_thumb_key_role(&event)",
        // ADR-199 決定18(i)(ii): F13〜F24 のラッチを「実際に書いたか」で確定する呼び出しと、リピートで昇格させない条件。
        "self.settle_fkey_role_latch(&event, shadow_toggled)",
        "is_role_fkey(event.vk_code)",
    ] {
        assert!(
            kp.contains(token),
            "runtime/key_pipeline.rs の本番コードから `{token}` が消えています \
             （BUG-116/ADR-137 決定2のガード）"
        );
    }
    // 順序も固定する: `enrich_key_role` が `kp_stage_shadow_ime_toggle`・`plan()` より後ろに動くと、
    // それらが `shadow_action` の付く前のイベントを読み、0xF3/0xF4 が Allow のまま二重 actuation になる。
    let thumb_role_at = kp
        .find("self.enrich_thumb_key_role(&event)")
        .expect("enrich_thumb_key_role の呼び出し");
    let on_input_at = kp
        .find("self.engine.on_input(event, &ctx)")
        .expect("engine.on_input の呼び出し");
    assert!(
        thumb_role_at < on_input_at,
        "runtime/key_pipeline.rs: `enrich_thumb_key_role` は `engine.on_input` より前に呼ぶこと（ADR-199 決定16。\
         KeyDown 時点で `defers_solo_until_release` が役割由来の操作を見る）"
    );
    // `settle_fkey_role_latch` は `kp_stage_shadow_ime_toggle` の結果（`shadow_toggled`）を受けるので直後、`plan()` より前。
    let toggle_at = kp
        .find("self.kp_stage_shadow_ime_toggle(&event, engine_owns_open_key)")
        .expect("kp_stage_shadow_ime_toggle の呼び出し");
    let settle_at = kp
        .find("self.settle_fkey_role_latch(&event, shadow_toggled)")
        .expect("settle_fkey_role_latch の呼び出し");
    let plan_at = kp
        .find("PhysicalKeyDisposition::plan(")
        .expect("plan の呼び出し");
    assert!(
        toggle_at < settle_at && settle_at < plan_at,
        "runtime/key_pipeline.rs: `settle_fkey_role_latch` は `kp_stage_shadow_ime_toggle` の後・`plan()` の前に呼ぶこと\
         （ADR-199 決定18(i)）"
    );
    let enrich = kp
        .find("self.enrich_key_role(&mut event)")
        .expect("enrich_key_role の呼び出し");
    for later in [
        "self.kp_stage_shadow_ime_toggle(&event, engine_owns_open_key)",
        "PhysicalKeyDisposition::plan(",
    ] {
        let at = kp
            .find(later)
            .unwrap_or_else(|| panic!("`{later}` が見つかりません"));
        assert!(
            enrich < at,
            "runtime/key_pipeline.rs: `enrich_key_role` は `{later}` より前に呼ぶこと（ADR-199 T4）"
        );
    }
}

/// `hook_callback`（`WH_KEYBOARD_LL` フックプロシージャ本体）内のログ/tracing
/// マクロ呼び出し数を固定する（ADR-139 決定2）。
///
/// `hook_channel.rs:183-199` の不変条件「フックコールバック上ではロック取得・
/// アロケーション・ブロッキング呼び出し・ログ出力を一切行わない」により、
/// 通常のキー打鍵経路はログ呼び出しゼロで抜ける。現在ある7箇所は全て
/// IME モードキー・`VK_KANA`・`VK_DBE_ROMAN`/`NOROMAN`・Alt 系 vk という
/// **稀な分岐内のみ**（`hook.rs` のコメントに「VK_KANA は稀なキーなので
/// ログコストは無視できる」と評価済み）。`log`→`tracing` 移行（決定1）で
/// `tracing-appender::non_blocking` 等を安易に導入すると「non_blocking なら
/// フックコールバックで自由にログしてよい」という誤読を招きかねないため、
/// この数が増えていないことをテストで固定し、invariant が緩む方向の変更を
/// 機械的に検知する。意図した追加ならこのテストの期待値を更新すること。
#[test]
fn hook_callback_log_call_count_is_pinned() {
    const START_MARKER: &str = "unsafe extern \"system\" fn hook_callback(";
    const END_MARKER: &str = "pub fn now_timestamp_us";

    let content = read_crate_file("src/hook.rs");
    let start = content
        .find(START_MARKER)
        .unwrap_or_else(|| panic!("marker {START_MARKER:?} not found in hook.rs"));
    let end = content[start..].find(END_MARKER).map_or_else(
        || panic!("marker {END_MARKER:?} not found after hook_callback"),
        |i| start + i,
    );
    let body = &content[start..end];

    let count = body.matches("tracing::trace!").count()
        + body.matches("tracing::debug!").count()
        + body.matches("tracing::info!").count()
        + body.matches("tracing::warn!").count()
        + body.matches("tracing::error!").count();
    assert_eq!(
        count, 7,
        "hook_callback 内のログ/tracing マクロ呼び出し数が想定(7)と異なります \
         (実際: {count})。hook_channel.rs:183-199 の不変条件\
         （フックコールバック上でログ出力を一切行わない）が緩んでいないか、\
         増えた呼び出しが本当に稀な分岐内に限定されているかを確認すること。\
         意図した変更ならこのテストの期待値を更新すること。"
    );
}

/// BUG-181: `hook_callback` は物理 Up で `stale_down_vk_on_up` を呼び、Down 時に
/// 記録した VK の押下枠を落とす（Down=0xF2/Up=0xF0 の物理ひらがなキーで
/// `was_down` が固着し押下 ID を失うのを防ぐ）。この呼び出しが消えると再発する。
#[test]
fn hook_callback_clears_stale_down_vk_on_up() {
    let content = read_crate_file("src/hook.rs");
    let start = content
        .find("unsafe extern \"system\" fn hook_callback(")
        .expect("hook_callback not found");
    let body = &content[start..];
    for needle in [
        "crate::vk::stale_down_vk_on_up(",
        "crate::vk::physical_identity_slot(",
        "physical_down_vk_by_identity",
    ] {
        assert!(
            body.contains(needle),
            "hook_callback から {needle} の呼び出しが消えています（BUG-181）"
        );
    }
}

/// リポジトリルート相対のファイルを読む（`read_crate_file` は crate ルート
/// 相対専用のため、`.claude/`・`.githooks/` はこちらを使う）。
fn read_repo_root_file(rel_path: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    // crates/awase-windows/ から見てリポジトリルートは2段上。
    let repo_root = Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("failed to resolve repo root from {manifest_dir}"));
    let raw = fs::read_to_string(repo_root.join(rel_path))
        .unwrap_or_else(|e| panic!("failed to read {rel_path}: {e}"));
    raw.replace("\r\n", "\n")
}

/// ADR-139 決定3の `#[instrument]` 対象ファイルが、
/// `.claude/rules/fix-requires-evidence.md` の再発ファミリー表、または
/// `.githooks/pre-push` の正規表現のいずれかでカバーされていることを保証する。
///
/// **この assert は一方向のみ**（決定3 ⊆ 表 ∪ 正規表現）。逆方向——表/正規表現が
/// 守るべきファイルを決定3側が instrument し忘れていないか——は検知できない
/// （表・正規表現・決定3の3者は完全な集合一致にならない: `tuning.rs` は表・
/// 正規表現には現れるが決定3では定数のみのため明示的に対象外、正規表現は
/// ディレクトリ/パスのプレフィックス単位でありファイル単位の決定3リストとは
/// 粒度が異なる。タスク分解レビューで判明、ADR-139 決定3参照）。
/// 決定3の対象ファイルが増減したらこの定数リストも更新すること。
#[test]
fn decision3_instrument_targets_are_covered_by_reincidence_family_docs() {
    const DECISION3_FILES: &[&str] = &[
        "ime_controller.rs",
        "runtime/open_chain.rs",
        "runtime/executor.rs",
        "runtime/conv_actuation.rs",
        "output/conv_actuation.rs",
        "runtime/transport.rs",
        "output/tsf_warmup_coord.rs",
        "output/probe_io.rs",
        "state/ime_model.rs",
        "state/observation_store.rs",
        "runtime/ime_coordinator.rs",
        "focus/classifier.rs",
        "focus/classify.rs",
        "focus/uia.rs",
        "focus/msaa.rs",
        "runtime/focus_tracking.rs",
        "state/conv_mode.rs",
        "ime.rs",
        "output/vk_send.rs",
        "platform.rs",
        "runtime/ime_refresh.rs",
        "runtime/key_pipeline.rs",
        "tsf/probe.rs",
        "tsf/observer.rs",
        "tsf/output.rs",
    ];

    // `#[tracing::instrument]` を1つも持たない対象（PRコードレビューで、
    // このリストに載っているのに計装されていないことが検出された）。
    // 中身を確認した上での意図的な除外のみここに載せ、理由を書くこと。
    const NO_INSTRUMENT_EXCEPTIONS: &[(&str, &str)] = &[
        (
            "runtime/ime_coordinator.rs",
            "29行、ImeCoordinator::new()のみ。実際のIME適用結果の集約は\
             runtime/mod.rs::on_ime_apply_completeが担い、そちらに#[instrument]済み。",
        ),
        (
            "tsf/observer.rs",
            "ほぼ全てatomicのアクセサ（notify/baseline/has_changed/reset/value等）で、\
             相関情報を持つ意味のある処理単位が無い。",
        ),
    ];

    let table = read_repo_root_file(".claude/rules/fix-requires-evidence.md");
    let hook = read_repo_root_file(".githooks/pre-push");

    for file in DECISION3_FILES {
        let file_stem = file.rsplit('/').next().unwrap_or(file);
        let module_stem = file_stem.trim_end_matches(".rs");
        // 表・正規表現とも、個別ファイル名ではなくディレクトリ単位
        // （`focus/`・`output/`・`tsf/`等）で再発ファミリーを指す行がある
        // （例: 「focus 遷移」行は `focus/` とだけ書き、`focus/classifier.rs`
        // を個別列挙しない、`.githooks/pre-push`の正規表現も同様）ため、
        // ディレクトリプレフィックスでの一致も許容する。**この包含チェックは
        // 一方向かつ緩い**（決定3が表/正規表現の範囲を逸脱していないかしか
        // 見ない。表/正規表現がカバーすべきファイルを決定3が計装し忘れて
        // いないかは、下の「実際に#[instrument]があるか」チェックが担う）。
        let dir_prefix = file.rsplit_once('/').map(|(dir, _)| format!("{dir}/"));
        let dir_hit = dir_prefix
            .as_deref()
            .is_some_and(|d| table.contains(d) || hook.contains(d));
        let in_table = table.contains(file_stem) || table.contains(module_stem);
        let in_hook = hook.contains(file_stem) || hook.contains(module_stem);
        assert!(
            in_table || in_hook || dir_hit,
            "ADR-139決定3の対象 `{file}` が fix-requires-evidence.md の再発ファミリー表にも \
             .githooks/pre-push の正規表現にも見つかりません。決定3がホットスポット表の \
             範囲外へ逸脱していないか（本当に再発ファミリー領域か）確認すること。\
             意図した対象追加なら表または正規表現側も更新するか、このテストの \
             コメントに除外理由を明記すること。"
        );

        // 逆方向: リストに載っているのに実際は計装されていない、という
        // このPR自身が作った状態（B4/B-4、PRコードレビューで検出）を検知する。
        if let Some((_, reason)) = NO_INSTRUMENT_EXCEPTIONS.iter().find(|(f, _)| f == file) {
            let _ = reason; // 理由はコメント/定数として保持するのみ、assertはしない
            continue;
        }
        let content = read_crate_file(&format!("src/{file}"));
        // コメント中の説明的な言及（例: 本テスト自身の存在を解説する
        // `runtime/ime_refresh.rs` のコメントが偶然 `#[tracing::instrument]`
        // という文字列を含む）に誤って一致しないよう、コメント行を除去してから
        // 実際の属性出現を数える（PRコードレビュー指摘）。
        let production = non_comment_lines(production_code_only(&content));
        assert!(
            production.contains("#[tracing::instrument"),
            "ADR-139決定3の対象 `{file}` に #[tracing::instrument] が1つもありません。\
             リストに載せたなら実際に計装すること。計装すべき関数が無いファイルなら \
             NO_INSTRUMENT_EXCEPTIONS に理由付きで追加すること。"
        );
    }
}

/// ADR-139 決定4 必須条件2・3: `journal.rs` の `emit_tracing` 実装
/// （`JournalEntry::emit_tracing`／判別子文字列ヘルパー群／
/// `JournalEnvelope::emit_tracing`）に `?`/`%` シギル（Debug/Display
/// フォーマット）と `_ =>`/`.. =>` ワイルドカードアームが出現しないことを保証する。
///
/// `?`/`%` はDebug文字列化であり、ADR-082決定1（`ImeEvent`等の型を保った
/// journal記録）を実質的に巻き戻す。ワイルドカードは`match`の網羅性検査
/// （将来variantが増えたときにコンパイルエラーで検知する、この機構の唯一の
/// 安全装置）を破壊する。コメント中の `` `?`/`%` `` `` `_ =>` `` という説明的な
/// 言及は対象外にするため、コメント行を除去してから走査する。
#[test]
fn journal_emit_tracing_has_no_debug_display_sigils_or_wildcards() {
    const START_MARKER: &str = "fn decision_kind_shape(";
    const END_MARKER: &str = "/// 統合イベントジャーナル。";

    let content = read_crate_file("src/journal.rs");
    let start = content
        .find(START_MARKER)
        .unwrap_or_else(|| panic!("marker {START_MARKER:?} not found in journal.rs"));
    let end = content[start..].find(END_MARKER).map_or_else(
        || panic!("marker {END_MARKER:?} not found after {START_MARKER:?}"),
        |i| start + i,
    );
    let block = non_comment_lines(&content[start..end]);

    assert!(
        !block.contains('?'),
        "journal.rs の emit_tracing 実装に `?`（Debug フォーマット）が含まれています。\
         ADR-139決定4はDebug文字列化を禁止しています（ADR-082決定1の巻き戻し防止）。\
         判別子文字列は `strum::IntoStaticStr` + `variant_name` で取ること。"
    );
    assert!(
        !block.contains('%'),
        "journal.rs の emit_tracing 実装に `%`（Display フォーマット）が含まれています。\
         ADR-139決定4はDisplayフォーマットも禁止しています（理由は `?` と同じ）。"
    );
    assert!(
        !block.contains("_ =>") && !block.contains(".. =>"),
        "journal.rs の emit_tracing 実装（またはその判別子文字列ヘルパー）に \
         ワイルドカードアームが含まれています。将来 variant が増えたときの \
         コンパイルエラー検知（この機構の唯一の安全装置）が失われるため、\
         全 variant を明示的に列挙すること。"
    );
}

/// ADR-169: `UnifiedJournal::record_key_input` の OS auto-repeat 畳み込みは
/// 「`key_input` レーンの `buffer.back()` は直前に記録した `KeyInput` である」
/// という不変条件に依存する。この不変条件は `JournalEntry::KeyInput {` の
/// 本番構築点が `runtime/key_pipeline.rs` の1箇所だけであることが前提
/// （複数箇所から構築される、または `absorb()` 経由の遅延 envelope が
/// このレーンに混ざると、`back()` が「直前の KeyInput」でなくなり、
/// 無関係なエントリへ `repeat_count` が誤って加算される——時系列の
/// 捏造）。新しい構築箇所を追加する前に、`record_key_input` の doc comment
/// （`journal.rs`）を読み、この不変条件への影響を確認すること。
#[test]
fn journal_key_input_construction_is_limited_to_key_pipeline() {
    // フルパス（`crate::journal::` 修飾）でのみ数える: `journal.rs` 自身の
    // 内部コードは同モジュール内なので `JournalEntry::KeyInput` を無修飾で
    // 参照する（`record_key_input` 内部の分解パターン等、これらは新規
    // construction ではなく既存エントリの読み取りであり対象外）。
    // 外部（他モジュール）からの construction は必ずこのフルパス表記に
    // なるため、これで実質的に「外部からの構築箇所」だけを数えられる。
    const NEEDLE: &str = "crate::journal::JournalEntry::KeyInput {";
    const EXPECTED_PATH: &str = "src/runtime/key_pipeline.rs";

    let files = list_src_files();
    let mut total = 0usize;
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = production.matches(NEEDLE).count();
        if count > 0 {
            total += count;
            breakdown.push((path.clone(), count));
        }
    }
    assert_eq!(
        total, 1,
        "`{NEEDLE}` の本番コードでの構築箇所数が想定(1)と異なります(実際: {total})。\
         内訳: {breakdown:?}\n\
         想定される唯一の構築箇所は `{EXPECTED_PATH}` の `kp_run_inner` です。\
         この不変条件が崩れると `UnifiedJournal::record_key_input`（ADR-169）の \
         auto-repeat 畳み込みが無関係なエントリへ誤って合流します。"
    );
    assert_eq!(
        breakdown,
        vec![(EXPECTED_PATH.to_owned(), 1)],
        "`{NEEDLE}` の構築箇所は `{EXPECTED_PATH}` である想定でしたが、\
         実際の内訳は {breakdown:?} でした。"
    );
}

/// Windows Defenderの`Behavior:Win32/Persistence.A!.ml`誤検知対策
/// （`docs/known-bugs.md` BUG-120、2026-09-07）: HKCU Runキーへの登録/解除
/// (`autostart::register()`/`autostart::unregister()`)は、ユーザーの
/// クリックに対する直接の同期的な応答からのみ発生させる方針にした
/// （`bootstrap.rs::handle_auto_start`からの自動再登録を撤去）。
///
/// この方針は現状プローズ（コードコメント）だけで守られており、
/// 型やコンパイラでは強制されていない。将来、新しいバックグラウンド
/// メンテナンス/自動修復処理がうっかり`autostart::register()`を呼ぶと、
/// 本PRが除去したのと同じ「無操作でのRunキー書き込み」パターンを
/// 静かに再導入してしまう（`fix-requires-evidence.md`が記録するissue
/// #136と同型の「1箇所直しても別経路が迂回する」問題）。せめて
/// 呼び出し箇所数をテキスト走査で固定し、想定外の増加を検知する
/// （`awase-windows`クレート内のみ対象。`awase-settings`側の呼び出しは
/// `crates/awase-settings/src/main.rs::apply_autostart_toggle`が唯一の
/// 呼び出し元であることをコードレビューで確認済み、こちらは別crateの
/// ためこのテストのスキャン対象外）。
#[test]
fn autostart_register_call_sites_are_limited_to_tray_click_handler() {
    const NEEDLES: [(&str, usize); 2] =
        [("autostart::register(", 1), ("autostart::unregister(", 1)];

    let files = list_src_files();
    for (needle, expected) in NEEDLES {
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
             唯一の想定呼び出し元は `src/tray.rs::handle_autostart_toggle`\
             （トレイメニュークリックへの直接の同期応答）です。新しい呼び出しを\
             追加する前に、それがユーザーのクリックに対する直接の同期的な応答か\
             確認してください——起動時やタイマー等、無操作の経路から呼ぶと\
             Windows Defenderの`Behavior:Win32/Persistence.A!.ml`誤検知の\
             再発要因になります（`docs/known-bugs.md` BUG-120参照）。"
        );
        assert_eq!(
            breakdown,
            vec![("src/tray.rs".to_string(), expected)],
            "`{needle}` は src/tray.rs 以外からも呼ばれています: {breakdown:?}\n\
             上記assert_eqのメッセージ参照。"
        );
    }
}

/// ADR-158 TE1（opus code review M2で追加）: `tuning.rs`の全`pub const`に
/// `#[measured_macro::measured(...)]`が付いていることを確認する。
///
/// `#[measured]`自体は`value_ms`と定数の実値が一致するかは検証するが、
/// 「そもそも属性が付いているか」は検証しない（属性が無ければマクロは実行されず、
/// 静かに素通りする）。新しい定数を無属性で追加する退行を、このガードで検出する。
#[test]
fn tuning_constants_all_have_measured_attribute() {
    let content = read_crate_file("src/tuning.rs");
    let lines: Vec<&str> = content.lines().collect();
    let mut missing = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("pub const ") {
            continue;
        }
        // 直前の非空行が #[measured_macro::measured(...)] であることを確認する。
        let mut j = i;
        let mut found = false;
        while j > 0 {
            j -= 1;
            let prev = lines[j].trim();
            if prev.is_empty() {
                continue;
            }
            found = prev.starts_with("#[measured_macro::measured(");
            break;
        }
        if !found {
            let const_name = trimmed
                .trim_start_matches("pub const ")
                .split(':')
                .next()
                .unwrap_or(trimmed);
            missing.push(format!("{const_name} (line {})", i + 1));
        }
    }
    assert!(
        missing.is_empty(),
        "tuning.rsに#[measured_macro::measured(...)]の付いていないpub constがあります: \
         {missing:?}\n\
         新しい定数を追加した場合は、実測済みなら#[measured(value_ms=.., commit=\"..\")]、\
         未実測ならせめて#[measured(pending = true)]を付けること \
         (.claude/rules/tuning-constants.md)。"
    );
}

/// ADR-178（MSIアンインストール時のユーザーデータ喪失をPermanent化+自己修復で
/// 防ぐ）v14 opus敵対的レビュー Blocker B1対応。
///
/// `wix/main.wxs`の`Permanent="yes"`（7コンポーネント）は
/// `wix_installer_guard.rs::config_file_and_nicola_yab_components_have_permanent`
/// で固定されているが、その**唯一の解毒剤**である自己修復の配線
/// （`ensure_default_config_exists`/`ensure_default_layouts_exist`の
/// 呼び出しと、生成先を書き込み先として固定する不変条件）を守るテストが
/// これまで存在しなかった。`Permanent`は不可逆であり、この配線が壊れると
/// round1 B1の最悪シナリオ（MSI管理外のレジストリKeyPathが残る環境で
/// 再インストールしてもconfig.tomlが再配置されず起動不能になる）が
/// 二度と直せない形で復活する。
mod adr178_self_heal_wiring {
    use super::{extract_fn_body, read_crate_file};

    /// `app/mod.rs::load_config`本体に`ensure_default_config_exists()`が
    /// 含まれること。これが消えると、起動時にconfig.tomlが自動生成されず
    /// round1 B1のシナリオがそのまま復活する。
    #[test]
    fn load_config_calls_ensure_default_config_exists() {
        let content = read_crate_file("src/app/mod.rs");
        let body = extract_fn_body(&content, "fn load_config() -> Result<AppConfig> {");
        assert!(
            body.contains("ensure_default_config_exists();"),
            "app/mod.rs::load_config()の本体にensure_default_config_exists()の\
             呼び出しが見つからない。これが無いとPermanent=\"yes\"で保護している\
             config.tomlが万一消えたとき、再生成されず起動不能になる \
             （round1 B1が不可逆な形で復活する、ADR-178 v14レビューB1対応）。"
        );
    }

    /// `find_config_path`は副作用を持たない設計にした（v14レビューM1対応）
    /// ——`read_bug_report_attachments`等の観測経路から誤って自己修復を
    /// 発火させないため。この関数本体にensure系の呼び出しが紛れ込んで
    /// いないことを固定する。
    #[test]
    fn find_config_path_has_no_self_heal_side_effect() {
        let content = read_crate_file("src/app/mod.rs");
        let body = extract_fn_body(
            &content,
            "pub(crate) fn find_config_path() -> Result<PathBuf> {",
        );
        assert!(
            !body.contains("ensure_default_config_exists"),
            "app/mod.rs::find_config_path()にensure_default_config_exists()の\
             呼び出しが紛れ込んでいる。この関数はread_bug_report_attachments等\
             複数の観測経路から呼ばれる副作用のないヘルパーであるべき \
             （ADR-178 v14レビューM1対応）——不具合報告を開いただけで\
             ユーザー環境のconfig.tomlが生成されてしまう回帰を防ぐ。"
        );
    }

    /// `app/bootstrap.rs::init_engine_validated`内で、`.yab`の自己修復
    /// （`ensure_default_layouts_exist`）の呼び出しが、読み取り先を解決する
    /// `resolve_relative`より**前**にあること。順序が入れ替わると、
    /// 2026-09-17に実機で踏んだバグ（`resolve_relative`がCWD相対の裸パスへ
    /// フォールバックし、生成先もそこに引きずられて`%LOCALAPPDATA%\awase\layout`
    /// が生成されない）が再発する。
    #[test]
    fn layouts_self_heal_runs_before_resolve_relative() {
        let content = read_crate_file("src/app/bootstrap.rs");
        let ensure_pos = content
            .find("ensure_default_layouts_exist(&config.general.layouts_dir)")
            .expect("ensure_default_layouts_exist call not found in bootstrap.rs");
        let resolve_pos = content
            .find("let layouts_dir = resolve_relative(&config.general.layouts_dir)")
            .expect("resolve_relative(&config.general.layouts_dir) call not found in bootstrap.rs");
        assert!(
            ensure_pos < resolve_pos,
            "ensure_default_layouts_exist()の呼び出し（位置={ensure_pos}）が\
             resolve_relative()の呼び出し（位置={resolve_pos}）より後にある。\
             resolve_relativeは存在依存のフォールバック（exe隣に無ければCWD相対の\
             裸パスを返す）を持つため、先に呼ぶと自己修復の生成先を\
             汚染する。2026-09-17実機検証で発見した`.yab`未生成バグが\
             再発する（ADR-178 v14レビューBlocker B1対応、無警告で起きる\
             ため気づきにくい）。"
        );
    }

    /// `ensure_default_config_exists`/`ensure_default_layouts_exist`の本体に
    /// `resolve_relative`系の解決関数が出現しないこと（＝生成先を存在依存の
    /// 解決結果に委ねない、という不変条件そのもの）。v2〜v13で7回、v14でも
    /// 1回（`3d7a7ece`）再発した「読み取り先/書き込み先」問題の根を、
    /// これ以上形を変えて再発させないための機械的なガード。
    #[test]
    fn ensure_functions_never_use_resolve_relative_as_write_target() {
        for (file, fn_signature) in [
            ("src/app/mod.rs", "fn ensure_default_config_exists() {"),
            (
                "src/app/mod.rs",
                "pub(super) fn ensure_default_layouts_exist(layouts_dir_raw: &str) {",
            ),
        ] {
            let content = read_crate_file(file);
            let body = extract_fn_body(&content, fn_signature);
            for forbidden in [
                "resolve_relative(",
                "resolve_relative_to_exe(",
                "resolve_layouts_dir(",
            ] {
                assert!(
                    !body.contains(forbidden),
                    "{file}の`{fn_signature}`の本体に{forbidden}が出現する。\
                     これらの関数は存在に依存したフォールバック（exe隣に無ければ\
                     CWD相対の裸パスを返す）を持つため、生成先として使うと\
                     意図しない場所への書き込みが発生する \
                     （ADR-178 v14レビューBlocker B1・M6が名指しした不変条件）。\
                     生成先は常にexe_dir.join(生文字列)で明示的に組み立てること。"
                );
            }
        }
    }

    /// `crates/awase-settings/src/main.rs::SettingsApp::new`に、config.toml・
    /// `.yab`両方の自己修復呼び出しが含まれること（別クレートだが
    /// `CARGO_MANIFEST_DIR`からの相対パスで読める。専用の`tests/`を持たない
    /// `awase-settings`側の唯一の配線ガードをここに置く）。
    #[test]
    fn settings_app_new_calls_both_ensure_functions() {
        let content = read_crate_file("../awase-settings/src/main.rs");
        let body = extract_fn_body(
            &content,
            "fn new(cc: &eframe::CreationContext<'_>, adr192_warning_context: bool) -> Self {",
        );
        assert!(
            body.contains("ensure_default_config_exists();"),
            "crates/awase-settings/src/main.rs::SettingsApp::new()にensure_default_config_exists()\
             の呼び出しが見つからない（ADR-178 v14レビューB1対応）。"
        );
        assert!(
            body.contains("ensure_default_layouts_exist(&config.general.layouts_dir);"),
            "crates/awase-settings/src/main.rs::SettingsApp::new()にensure_default_layouts_exist()\
             の呼び出しが見つからない（ADR-178 v14レビューB1対応）。"
        );
    }
}

/// ADR-191 決定3: `ImeEvent::KeyEffectPredicted`（打鍵時点の予測を belief へ反映する、観測でも意図でもない
/// 専用イベント）の構築は `ImeStateHub::apply_key_effect_prediction` の1箇所に限る。ここが増えると
/// 「観測の偽装」や「ユーザー意図の偽装」への近道になりうる（`ime-belief-architecture.md`）。
/// また `desired_open` を書かない（ドリフト補正がIMEへ書き戻して「awaseは書かない」に反する）ことを
/// reduce 側のアームで固定する。
#[test]
fn key_effect_predicted_event_is_constructed_only_in_apply_key_effect_prediction() {
    let platform_state = read_crate_file("src/state/platform_state.rs");
    let production = production_code_only(&platform_state);
    assert_eq!(
        count_real_calls(production, "ImeEvent::KeyEffectPredicted {"),
        1,
        "KeyEffectPredicted の構築は platform_state.rs::apply_key_effect_prediction の1箇所だけ"
    );
    for path in [
        "src/runtime/key_pipeline.rs",
        "src/runtime/ime_refresh.rs",
        "src/runtime/mod.rs",
        "src/runtime/executor.rs",
    ] {
        let content = read_crate_file(path);
        assert_eq!(
            count_real_calls(production_code_only(&content), "ImeEvent::KeyEffectPredicted"),
            0,
            "{path} から KeyEffectPredicted を直接 dispatch しない（apply_key_effect_prediction 経由）"
        );
    }
    // reduce のアームは desired_open を書かない。
    let model = read_crate_file("src/state/ime_model.rs");
    let arm = model
        .split("ImeEvent::KeyEffectPredicted { open, mode, track } => {")
        .nth(1)
        .expect("reduce に KeyEffectPredicted のアームがある")
        .split("ImeEvent::ModeKeyPassedThrough")
        .next()
        .unwrap();
    assert!(
        !arm.contains("desired_open"),
        "KeyEffectPredicted は desired_open を書かない"
    );
}

/// ADR-191 決定3・4（レビュー指摘C-M3）: `state/key_effect_table.rs`（打鍵時予測の表）は
/// `tools/e2e/ime_key_matrix/gen_key_effect_table.py` が `grid-tables/*.json` から生成する
/// 「手で編集しない」ファイルである。生成元 JSON・スクリプトを変えて再生成し忘れる、または
/// 生成物を手で編集すると、表が黙って学習結果と食い違う（読めないアプリでは観測で訂正されない）。
/// スクリプトの `--check` でコミット済みの生成物と一致することを機械的に検査する。
///
/// `python3` が無い環境では失敗する（`AWASE_ALLOW_SKIP_GENERATED_CHECK` を立てたときだけスキップ）。CI（ubuntu・windows）には有る。
#[test]
fn key_effect_table_matches_generator() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let script = repo.join("tools/e2e/ime_key_matrix/gen_key_effect_table.py");
    assert!(
        script.exists(),
        "生成スクリプトが見つかりません: {script:?}"
    );
    let out = match std::process::Command::new("python3")
        .arg(&script)
        .arg("--check")
        .env("PYTHONUTF8", "1")
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // 無言でスキップすると、ランナーから python3 が消えたときに緑のまま検査が消える（round2 C-N7）。
            // 明示的に許可した環境（ローカルの最小構成）でだけスキップし、CI では失敗させる。
            assert!(
                std::env::var_os("AWASE_ALLOW_SKIP_GENERATED_CHECK").is_some(),
                "python3 が見つからないため key_effect_table.rs の生成物検査ができません。python3 を入れるか、\
                 ローカルだけ AWASE_ALLOW_SKIP_GENERATED_CHECK=1 でスキップしてください"
            );
            eprintln!("python3 が無いため key_effect_table.rs の生成物検査をスキップします");
            return;
        }
        Err(e) => panic!("python3 の起動に失敗: {e}"),
    };
    assert!(
        out.status.success(),
        "state/key_effect_table.rs が生成結果と一致しません:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// ADR-202: 0x19（Alt+半角/全角）の `shadow_action` は、GJI のときだけ `Hankaku/Zenkaku` 行から求める。
/// `runtime/` は Linux でテスト実行できない（CLAUDE.md）ので、配線の形を静的スキャンで固定する:
/// (1) 同じ `latch_step` の中で決める（別の代入を増やすと Down/Up の非対称〈BUG-131/132 型〉と、
///     `shadow_action` 書き込み箇所固定の両方が崩れる）、(2) injected の 0x19 は付けない（静的値のまま）、
/// (3) GJI 以外は `KanjiRolePlan::KeepStatic`（現行維持、決定2）。
#[test]
fn kanji_0x19_role_goes_through_the_shared_latch_and_only_overrides_gji() {
    let rt_src = read_crate_file("src/runtime/mod.rs");
    let rt = production_code_only(&rt_src);
    for token in [
        "fn kanji_shadow_action",
        "kanji_role_plan(is_gji, m.ctrl, m.shift, m.win)",
        "KanjiRolePlan::KeepStatic => static_action",
        "KanjiRolePlan::Passive => None",
        "KanjiRolePlan::Derive => self.derive_key_shadow_action(ImeKindId::Gji, vk)",
        "if is_kanji && event.injected",
        "return self.kanji_shadow_action(vk, static_action, m);",
    ] {
        assert!(
            rt.contains(token),
            "runtime/mod.rs の本番コードから `{token}` が消えています（ADR-202 の配線）"
        );
    }
    // 0x19 の分岐は `latch_step` のクロージャ内（ラッチの前に判定しない）。
    let latch_at = rt.find("latch_step(").expect("latch_step の呼び出し");
    let kanji_at = rt
        .find("return self.kanji_shadow_action(vk, static_action, m);")
        .expect("0x19 の分岐");
    assert!(
        latch_at < kanji_at,
        "runtime/mod.rs: 0x19 の `kanji_shadow_action` は `latch_step` のクロージャの中で呼ぶこと（ADR-202 決定3）"
    );
    // `shadow_action` の代入は1箇所のまま（`ime_relevance_shadow_action_writes_are_accounted_for`）。
    assert_eq!(rt.matches("ime_relevance.shadow_action =").count(), 1);
}

/// ADR-199 決定15（2026-09-29 所有者決定）: `keys.ime_toggle` の既定は空（「IME の設定に従う」原則）。
/// 既定に無修飾の `VK_KANJI` を戻すと、`Engine::has_bare_ime_combo(0x19)` が常に真になり、
/// GJI の 0x19 役割判定（ADR-202、`derive_key_shadow_action` の `explicit_overlap`）が既定で無効化される。
/// `KeysConfig::default()` の書式と、設定 GUI の JIS 切替書き込みが既定へ揃っていることを固定する
/// （`ime_on`/`ime_off` の既定は変えない）。
#[test]
fn keys_ime_toggle_default_stays_empty_and_gui_jis_switch_follows_default() {
    let cfg = read_workspace_file("src/config.rs");
    let cfg = production_code_only(&cfg);
    assert!(
        cfg.contains("ime_toggle: Vec::new(),"),
        "src/config.rs: `KeysConfig::default()` の `ime_toggle` は空 (`Vec::new()`) のままにすること（ADR-199 決定15）"
    );
    assert!(
        !cfg.contains("ime_toggle: vec![\"VK_KANJI\""),
        "src/config.rs: `keys.ime_toggle` の既定に `VK_KANJI` を戻さないこと（ADR-199 決定15・ADR-202）"
    );
    assert!(
        cfg.contains("ime_on: vec![\"Ctrl+変換\".to_string()],")
            && cfg.contains("ime_off: vec![\"Ctrl+無変換\".to_string()],"),
        "src/config.rs: `keys.ime_on`/`ime_off` の既定（Ctrl+変換/Ctrl+無変換）は変えないこと（決定15）"
    );
    let gui = read_workspace_file("crates/awase-settings/src/main.rs");
    assert!(
        !gui.contains("keys.ime_toggle = vec![\"VK_KANJI\""),
        "awase-settings: JIS 切替で `keys.ime_toggle` に `VK_KANJI` を書かないこと（既定は空、決定15）"
    );
    assert!(
        gui.contains(
            "self.config.keys.ime_toggle = awase::config::KeysConfig::default().ime_toggle;"
        ),
        "awase-settings: JIS 切替の `ime_toggle` は `KeysConfig::default()` に揃えること"
    );
}

/// BUG-173（Opus レビュー D1）: 物理 F2 を Suppress/握りつぶす経路が再導入されないこと、および
/// KeyUp ラッチが `plan()` の直後・journal 記録と実配送の前に呼ばれることを固定する。
/// F2 の常時 Allow は `plan_core` 側にあり、`state/physical_disposition.rs` の `mod tests` が Linux でも検査する
/// (ADR-229 T1)。`handle_reinject` と KeyUp ラッチの呼び出し順は runtime/ 側で、Linux でテスト実行できない
/// （CLAUDE.md）ため、この静的スキャンが唯一の検知手段。
#[test]
fn bug173_physical_f2_is_never_suppressed_and_keyup_latch_order_is_fixed() {
    // 1. plan() の F2 分岐は常に Allow（VK だけで決まり、TSF/warmup の状態で Suppress を返さない）
    // `plan` の本体（F2 分岐を含む）は ADR-208 L0 で `state/physical_disposition.rs::plan_core` へ移した。
    let transport = read_crate_file("src/state/physical_disposition.rs");
    let transport = strip_any_test_module(&transport);
    let f2 = transport
        .find("if event.vk_code == crate::vk::VK_DBE_HIRAGANA {")
        .expect("plan() の F2 分岐が見つかりません（BUG-173）");
    let f2_branch = &transport[f2..];
    let f2_end = f2_branch.find("\n        }\n").expect("F2 分岐の終端");
    assert_eq!(
        f2_branch[..f2_end]
            .lines()
            .skip(1)
            .map(str::trim)
            .collect::<Vec<_>>(),
        vec!["return Self::Allow;"],
        "plan() の F2 分岐が `return Self::Allow;` 以外になっています（BUG-173: ADR-100 決定2 で warmup が \
         VK_IME_ON 単発になり、物理 F2 の代替 F2 再送の契約は無い。Suppress を戻すと物理ひらがなキーが無反応になる）"
    );

    // 2. handle_reinject に VK_DBE_HIRAGANA の特例（TSF での握りつぶし）を戻さない
    let executor = read_crate_file("src/runtime/executor.rs");
    let executor = strip_any_test_module(&executor);
    let start = executor
        .find("fn handle_reinject")
        .expect("handle_reinject が見つかりません");
    let body = &executor[start..];
    let end = body[10..].find("\n    fn ").map_or(body.len(), |e| e + 10);
    assert!(
        !body[..end].contains("VK_DBE_HIRAGANA"),
        "executor.rs::handle_reinject に VK_DBE_HIRAGANA の特例が再び現れています（BUG-173）"
    );
    // F2 の握りつぶしを platform 側の reinject フックへ移し替えることも禁じる。
    let platform_src = read_crate_file("src/platform.rs");
    let platform_src = strip_any_test_module(&platform_src);
    if let Some(at) = platform_src.find("fn on_reinject_key") {
        let body = &platform_src[at..];
        let end = body.find("\n    }\n").map_or(body.len(), |e| e + 7);
        assert!(
            !body[..end].contains("VK_DBE_HIRAGANA"),
            "platform.rs::on_reinject_key に VK_DBE_HIRAGANA の特例が現れています（BUG-173）"
        );
    }

    // 2b. キー打鍵を契機とする eager warmup の送信は撤去済み（BUG-173 追補2）。reinject 段は cold 化と GjiFsm reset だけ。
    let platform = read_crate_file("src/platform.rs");
    let platform = strip_any_test_module(&platform);
    let reinject = platform
        .find("fn on_reinject_key")
        .expect("on_reinject_key が見つかりません");
    let reinject_body = &platform[reinject..];
    let reinject_end = reinject_body
        .find("\n    }\n")
        .map_or(reinject_body.len(), |e| e + 7);
    assert!(
        !reinject_body[..reinject_end].contains("send_eager_tsf_warmup"),
        "platform.rs::on_reinject_key が `send_eager_tsf_warmup` を呼んでいます（BUG-173 追補2: 確定キー reinject 時の \
         VK_IME_ON 送信は撤去済み。Enter1回で2発出ていた発火の再導入になる）"
    );
    // eager warmup（`send_eager_tsf_warmup`）は ADR-212 P4 で全て撤去した（確定キー〈#398〉・フォーカス変更・随伴・vk_send の Off 固定〈P1〉）。
    // 再導入されたら（Enter 1回で2発・F2 併走の再発）ここで落ちる。
    let mut sends = 0;
    for f in [
        "src/platform.rs",
        "src/runtime/key_pipeline.rs",
        "src/runtime/executor.rs",
        "src/output/vk_send.rs",
        "src/runtime/ime_refresh.rs",
        "src/runtime/message_handlers.rs",
    ] {
        let src = read_crate_file(f);
        sends += strip_any_test_module(&src)
            .matches(".send_eager_tsf_warmup(")
            .count();
    }
    assert_eq!(
        sends, 0,
        "`send_eager_tsf_warmup(` の本番呼び出し箇所が0以外です（BUG-173 追補2: キー打鍵契機の warmup 送信は撤去済み。\
         意図した追加なら ADR-191 の warmup 節と BUG-173.md を更新してこの数を直すこと）"
    );
    assert!(
        !platform.contains("fn on_passthrough_key"),
        "platform.rs に `on_passthrough_key`（確定キー D 段の warmup 後処理）が戻っています（BUG-173 追補2）"
    );

    // 3. KeyUp ラッチの呼び出し順: plan() → latch → record_key_input → kp_stage_execute
    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    let kp = strip_any_test_module(&kp);
    let plan = kp
        .find("PhysicalKeyDisposition::plan(")
        .expect("plan( 呼び出し");
    let latch = kp
        .find("self.kp_latch_keyup_to_keydown_disposition(&event, physical)")
        .expect("ラッチ呼び出し");
    let record = kp.find("record_key_input(").expect("record_key_input(");
    let execute = kp
        .find("self.kp_stage_execute(decision, &event, profile, physical)")
        .expect("kp_stage_execute 呼び出し");
    assert!(
        plan < latch && latch < record && record < execute,
        "kp_run_inner の順序が壊れています: plan → kp_latch_keyup_to_keydown_disposition → \
         record_key_input(journal) → kp_stage_execute（BUG-173追補: journal の physical と実配送を一致させる）"
    );
}

/// ADR-203 決定9 / BUG-170: `GjiFsm` へ ON/開き直しを同期する入口の一覧を固定する。
///
/// BUG-170 は「belief だけが ON になり `GjiFsm` が `OffCold` に取り残される」同期漏れで、
/// 入口ごとに点パッチを足すたびに別の入口が漏れる再発ファミリー（BUG-18/22/170）だった。
/// 入口を宣言して件数を固定しておけば、新しい入口（または既存入口の削除）が黙って増減しない。
/// 入口を足す/消すときは、ADR-203 の決定表（`docs/adr/203-*.md`）と一緒にこの表を更新すること。
///
/// - `gji_on_ime_on(`: receipt（`sync_gji` の `OnImeOn` 腕）・フォーカス復帰の presync
///   （`focus_tracking.rs`、BUG-18）・IME 種別同期（`message_handlers.rs`）
/// - `gji_on_ime_off(`: receipt（`sync_gji` の `OnImeOff` 腕）のみ（OFF は awase の actuation 由来だけ）
/// - `sync_gji(`（ungated 側の `state/gji_direct_mechanism.rs` を除く）: (i) `send_keys` の level 突合
///   （`platform.rs`）、(ii) `kp_reopen_gji_fsm`（`key_pipeline.rs`）
/// - `kp_reopen_gji_fsm(`: 物理キー予測 ON・shadow toggle の既に ON（no-op 分岐）・OFF→ON に倒した瞬間の3か所
#[test]
fn gji_fsm_sync_entry_points_are_accounted_for() {
    let table: &[(&str, &[(&str, usize)])] = &[
        (
            "gji_on_ime_on(",
            &[
                ("src/platform.rs", 1),
                ("src/runtime/focus_tracking.rs", 1),
                ("src/runtime/message_handlers.rs", 1),
            ],
        ),
        ("gji_on_ime_off(", &[("src/platform.rs", 1)]),
        (
            "sync_gji(",
            &[("src/platform.rs", 1), ("src/runtime/key_pipeline.rs", 1)],
        ),
        ("kp_reopen_gji_fsm(", &[("src/runtime/key_pipeline.rs", 3)]),
    ];
    for (needle, expected) in table {
        for rel in list_src_files() {
            if rel == "src/state/gji_direct_mechanism.rs" && *needle == "sync_gji(" {
                continue; // receipt.settle の sink 呼び出し（INV-43）
            }
            let content = read_crate_file(&rel);
            let count = count_real_calls(production_code_only(&content), needle);
            let want = expected
                .iter()
                .find(|(f, _)| *f == rel)
                .map_or(0, |(_, n)| *n);
            assert_eq!(
                count, want,
                "{rel}: `{needle}` の呼び出し数が {count}（期待 {want}）。GjiFsm 同期の入口を増減したら \
                 ADR-203 の決定表とこの表を更新すること（点パッチの再発防止、BUG-18/22/170）"
            );
        }
    }
}

/// ADR-203 (ii): ユーザーの IME-ON 経路（`write_sync_key`/`write_physical_key`、既存の
/// `user_ime_on_paths_are_paired_with_eisu_reset` が数える2か所）は、`GjiFsm` の開き直し
/// （`kp_reopen_gji_fsm`）とも対で配線されていること。eisu 救済と同じく「新しい user IME-ON 経路を
/// 足したら対で配線し忘れる」ことの検出（BUG-170 の入口漏れの再発防止）。
#[test]
fn user_ime_on_paths_are_paired_with_gji_reopen() {
    let content = read_crate_file("src/runtime/key_pipeline.rs");
    let prod = production_code_only(&content);
    let on_paths =
        count_real_calls(prod, "write_sync_key(") + count_real_calls(prod, "write_physical_key(");
    let reopens = count_real_calls(prod, "kp_reopen_gji_fsm(");
    assert_eq!(
        on_paths, 2,
        "user IME-ON 書き込み経路の数が変わった（eisu 救済ガードと同時に更新）"
    );
    // shadow toggle: no-op 分岐 + 実際に倒した瞬間の2か所、加えて予測経路の1か所。
    assert!(
        reopens >= 3,
        "kp_reopen_gji_fsm の呼び出しが {reopens} 件。shadow toggle の2分岐と予測経路に必要"
    );
    // 3つの入口はそれぞれ別の発生元(journal の trigger で区別、PR #354 コードレビュー L1)を渡す。
    for src in [
        "ReopenSource::Predict",
        "ReopenSource::ShadowNoop",
        "ReopenSource::ShadowToggle",
    ] {
        assert_eq!(
            count_real_calls(prod, src),
            1,
            "{src} は kp_reopen_gji_fsm の入口ごとに1か所だけで使うこと"
        );
    }
}

/// ADR-203 決定3（/code-review 指摘）: `GjiSyncOrigin` は `GjiFsmSync::origin()` が唯一の出所。
/// `platform.rs` が `GjiSyncOrigin::BeliefSync` を直書きしてよいのは、`gji_sync_from_belief` の
/// `debug_assert!`（belief 起点専用であることの表明）の1か所だけ（Unicode long-cold の reinit を抑止する判定は
/// ADR-212 P3/P5 で reinit ごと撤去した）。同期の呼び出し側
/// （`gji_sync_from_belief`）は `sync.origin()` を渡す。直書きに戻ると、新しい variant の起点の
/// 取り違え（`origin()` のユニットテストは実経路を通らない）がテストで検出できなくなる。
#[test]
fn gji_sync_origin_comes_from_the_sync_variant() {
    let content = read_crate_file("src/platform.rs");
    let prod = production_code_only(&content);
    assert_eq!(
        count_real_calls(prod, "GjiSyncOrigin::BeliefSync"),
        1,
        "platform.rs の `GjiSyncOrigin::BeliefSync` 直書きは debug_assert の1か所だけ"
    );
    assert!(
        count_real_calls(prod, "sync.origin()") >= 2,
        "gji_sync_from_belief は sync.origin() を dispatch_gji_response_from に渡すこと"
    );
}

/// ADR-199 T17 Phase 4: 役割判定の「つなぎ目」を固定する。`runtime/mod.rs` は `#[cfg(windows)]` で Linux のホストテストに現れず、
/// `derive_key_shadow_action` の `ImeKindId::MsIme` 腕が `msime_native_key_role` 以外（例えば `None`）へ差し替わっても
/// 他のテストは全て通ってしまう。空白を除いて照合するので rustfmt の整形に依存しない。
/// 判定そのものの網羅は `key_effect_predictor.rs` の単体テストが持つ（ここでは重複させない）。
#[test]
fn derive_key_shadow_action_routes_ms_ime_to_msime_native_key_role() {
    let rt: String = read_crate_file("src/runtime/mod.rs")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    for token in [
        "ImeKindId::Gji=>k.gji_key_role(vk.0)",
        "ImeKindId::MsIme=>k.msime_native_key_role(vk.0)",
        "ime.and_then(|ime|self.derive_key_shadow_action(ime,vk))",
    ] {
        assert!(
            rt.contains(token),
            "runtime/mod.rs: `{token}` が無い。MS-IME 本体の役割判定（ADR-199 T17 Phase 4）のつなぎ目が外れている"
        );
    }
    let warn = read_crate_file("src/msime_key_assignment.rs");
    assert!(
        !warn.contains("トグル、awase未対応"),
        "msime_key_assignment.rs: 値2（トグル）は Phase 4 で awase が肩代わりするので競合警告に含めない"
    );
}

/// `kp_apply_conv_engine_sync`（idle-conv-check の conv 観測由来の engine 同期）は、
/// 書き込みと対の副作用（`ImeApplyRequested` の dispatch・イベント世代の確保・
/// `TIMER_IME_REFRESH` の kill）を持たない（ADR-213 P2d-1、旧 C2）。
///
/// 旧実装はこれらを書いたが、conv 観測は書き込みではないので対の副作用は何も守らず、
/// 世代だけが孤立 pending として残り、TsfNative で drift 補正タイマーを止めていた。
/// `ReportOpenInference` 分岐の `schedule_ime_refresh(20)` は別物（kill ではなく予約）なので対象外。
#[test]
fn conv_engine_sync_has_no_apply_requested_generation_or_timer_kill() {
    let path = "src/runtime/key_pipeline.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "fn kp_apply_conv_engine_sync(");
    let code = non_comment_lines(body);
    for forbidden in [
        "ImeApplyRequested",
        "allocate_event_generation",
        "timer.kill(TIMER_IME_REFRESH)",
        "handle_conv_engine_on_sync",
    ] {
        assert!(
            !code.contains(forbidden),
            "{path} の kp_apply_conv_engine_sync に `{forbidden}` が出現しています。\n\
             conv 観測由来の engine 同期は PanicReset ガード解除\
             （`release_panic_reset_guard_on_positive_evidence`）以外の副作用を持たないこと\
             （ADR-213 P2d-1）。"
        );
    }
}

/// ADR-208 L1: 明示キー押下の書き込みを起案する入口（order を発行する 2 入口）は、order の**発行前**に押下 ID を
/// 予約し（`claim_press_write`、`last_written_press`）、押下 ID を order に載せる（`with_press`）。
/// どちらか 1 入口だけに足して満足しない（`fix-requires-evidence.md` の「IME actuation 合流点」）。
/// drift correction（`runtime/ime_refresh.rs`）は押下に由来しないので `press=None` のまま（`with_press` を呼ばない）。
#[test]
fn press_id_is_claimed_and_carried_at_every_order_issuing_entry() {
    // Engine 経由（executor）。async/sync の 2 order すべてが press を載せる。
    let executor = read_crate_file("src/runtime/executor.rs");
    let body = extract_fn_body(production_code_only(&executor), "fn dispatch_ime_set_open(");
    let code = non_comment_lines(body);
    assert_eq!(
        code.matches("claim_press_write(").count(),
        1,
        "dispatch_ime_set_open は order の発行前に `claim_press_write` を 1 回だけ呼ぶこと（ADR-208 D1）"
    );
    assert_eq!(
        code.matches(".with_press(press)").count(),
        2,
        "dispatch_ime_set_open の async/sync 両方の order に `.with_press(press)` を載せること（ADR-208 D1）"
    );
    assert!(
        code.contains("explicit_press_applied_pair("),
        "dispatch_ime_set_open は view の shadow_on を `explicit_press_applied_pair` で未知にすること（ADR-208 D1）"
    );
    // shadow toggle（key_pipeline）。
    let kp = read_crate_file("src/runtime/key_pipeline.rs");
    let body = extract_fn_body(production_code_only(&kp), "fn kp_shadow_actuate(");
    let code = non_comment_lines(body);
    assert_eq!(
        code.matches("claim_press_write(").count(),
        1,
        "kp_shadow_actuate は order の発行前に `claim_press_write` を 1 回だけ呼ぶこと（ADR-208 D1）"
    );
    assert_eq!(
        code.matches(".with_press(press)").count(),
        2,
        "kp_shadow_actuate の async/sync 両方の order に `.with_press(press)` を載せること（ADR-208 D1）"
    );
    assert!(
        code.contains("explicit_press_applied_pair("),
        "kp_shadow_actuate は view の shadow_on を `explicit_press_applied_pair` で未知にすること（ADR-208 D1）"
    );
    // M-4: Engine が同じ打鍵で SetOpen を出すキーでは、shadow の判断の前に Engine へ純粋な問い合わせをして shadow を抑止する。
    let run = non_comment_lines(production_code_only(&kp));
    assert!(
        run.contains("matches_ime_set_open(") && run.contains("kp_stage_shadow_ime_toggle(&event, engine_owns_open_key)"),
        "kp_run_inner は shadow の判断の前に `engine.matches_ime_set_open` を問い合わせ、`engine_owns_open_key` を渡すこと（ADR-208 D1）"
    );
    let shadow_body = non_comment_lines(extract_fn_body(
        production_code_only(&kp),
        "fn kp_stage_shadow_ime_toggle(",
    ));
    assert!(
        shadow_body.contains("if engine_owns_open_key {"),
        "kp_stage_shadow_ime_toggle は `engine_owns_open_key` で昇格・書き込みを抑止すること（ADR-208 D1）"
    );
    // 押下の書き込みの直後に予約済みの refresh → drift correction が同じ向きを重ねない（BUG-113 型）。
    assert!(
        code.contains("timer.kill(TIMER_IME_REFRESH)"),
        "kp_shadow_actuate は書き込み前に打鍵前の `TIMER_IME_REFRESH` 予約を kill すること（P2c で ActivationSync の kill が消えた穴）"
    );
    // drift correction は押下に由来しない（press=None）。
    let refresh = read_crate_file("src/runtime/ime_refresh.rs");
    let refresh_prod = non_comment_lines(production_code_only(&refresh));
    assert!(
        !refresh_prod.contains("with_press(") && !refresh_prod.contains("claim_press_write("),
        "ime_refresh.rs（drift correction）は押下 ID を持たない: `with_press`/`claim_press_write` を呼んではならない"
    );
}

// ── FCIS P5a-1: `ImeStateHub` / `PlatformState` の公開面を固定する ─────────────────────
//
// `ImeStateHub` を `pub` にしたのは、閉ループのハーネス（`tests/support/harness.rs`）が本物を呼ぶため。
// 公開する面は「ハーネスが実際に呼ぶものだけ」に絞り、名前の集合をここで固定する。記録系
// （`record_confirmed`・`record_optimistic`・`record_ime_apply_result` とその `_in_scope` 版。INV-A97-1）は
// `pub` にしない。ただしこれは**入口を名前で絞る規律**であって、安全性の根拠ではない: `pub fn dispatch_event` が
// あれば、`ImeApplyRequested → ImeApplySucceeded` や `ModeKeyPassedThrough{demote_applied}` を流して crate の外から
// `applied` を書ける。本番に実害が無いのは、**本番のハブに crate の外から届く口が無い**から
// （`Runtime.platform_state` は private、`PlatformState.ime` は `pub(crate)`）。この到達不能性が本当の不変条件で、
// 下の `production_hub_is_unreachable_from_outside_the_crate` が固定する。ハーネスの呼び出しが増えて `pub` を
// 足すときは、`PLATFORM_STATE_PUB_FNS` と `harness.rs` の使い方を同じ PR で更新すること。

const PLATFORM_STATE_PUB_FNS: &[&str] = &[
    "advance_clock_ms",
    "apply_key_effect_prediction",
    "arm_external_change_watch_in_scope",
    "clock",
    "dispatch_event",
    "effective_open_at",
    "follow_external_change_in_scope",
    "model",
    "new", // `PlatformState::new`（`shell.rs`、実時計の構築口）
    "record_explicit_intent",
    "set_is_japanese_ime",
    "warrant_context",
    "with_clock",
];

/// `pub` の直後の修飾子（`const`・`async`・`unsafe`・`extern "..."`）を読み飛ばして、`fn ` の後ろを返す。
/// `pub(crate)` などの制限付きは対象外（`None`）。
fn pub_fn_rest(line: &str) -> Option<&str> {
    let mut rest = line.trim_start().strip_prefix("pub ")?;
    loop {
        if let Some(r) = rest.strip_prefix("fn ") {
            return Some(r);
        }
        if let Some(r) = rest
            .strip_prefix("const ")
            .or_else(|| rest.strip_prefix("async "))
            .or_else(|| rest.strip_prefix("unsafe "))
        {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("extern ") {
            // `extern "C" fn`
            rest = r.trim_start();
            if let Some(q) = rest.strip_prefix('"') {
                rest = q.split_once('"').map_or("", |(_, t)| t).trim_start();
            }
        } else {
            return None;
        }
    }
}

/// `pub fn`（修飾子付きを含む。`pub(crate)` などは含まない）の名前を集める。
fn pub_fn_names(code: &str) -> Vec<String> {
    let mut names: Vec<String> = code
        .lines()
        .filter_map(pub_fn_rest)
        .map(|rest| {
            rest.split(['(', '<'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .collect();
    names.sort();
    names
}

/// `pub` な戻り値（シグネチャの `->` 以降）に `needles` のどれかを名乗る `pub fn` の名前。
fn pub_fns_returning(code: &str, needles: &[&str]) -> Vec<String> {
    let lines: Vec<&str> = code.lines().collect();
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = pub_fn_rest(line) else {
            continue;
        };
        let name = rest.split(['(', '<']).next().unwrap_or("").trim();
        let mut sig = String::new();
        for l in &lines[i..] {
            sig.push_str(l);
            sig.push(' ');
            if l.contains('{') || l.trim_end().ends_with(';') {
                break;
            }
        }
        if let Some((_, ret)) = sig.split_once("->") {
            if needles.iter().any(|n| ret.contains(n)) {
                hits.push(name.to_string());
            }
        }
    }
    hits
}

/// 構造体のブロック内の `pub` フィールド（`pub(crate)` などは含まない）。
fn pub_field_lines(block: &str) -> Vec<String> {
    block
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub ") && l.contains(':') && !l.contains("fn "))
        .map(str::to_string)
        .collect()
}

/// 記録系の名前（`pub` にしてはならないもの）。
fn recorder_pub_violation(names: &[String]) -> Option<String> {
    names
        .iter()
        .find(|n| {
            n.starts_with("record_confirmed")
                || n.starts_with("record_optimistic")
                || n.starts_with("record_ime_apply_result")
        })
        .map(|n| format!("記録系 `{n}` が pub fn になっています（INV-A97-1）"))
}

#[test]
fn platform_state_pub_fns_are_fixed_and_exclude_recorders() {
    let mut names = Vec::new();
    for rel in list_src_files() {
        let content = read_crate_file(&rel);
        let code = non_comment_lines(production_code_only(&content));
        // `impl ImeStateHub {` / `impl PlatformState {` はどのファイルにあっても拾う（`runtime/executor.rs` など）。
        for needle in ["impl ImeStateHub {", "impl PlatformState {"] {
            for block in extract_all_balanced_blocks(&code, needle) {
                names.extend(pub_fn_names(block));
            }
        }
        // フィールド: `ImeStateHub`・`PlatformState` は `pub` フィールドを持たない（`shadow_model` 等を直接書かせない）。
        for needle in ["pub struct ImeStateHub {", "pub struct PlatformState {"] {
            for block in extract_all_balanced_blocks(&code, needle) {
                let fields = pub_field_lines(block);
                assert!(
                    fields.is_empty(),
                    "`{rel}`: `{needle}` に pub フィールドがあります（{fields:?}）。外から状態を直接書き換えられます"
                );
            }
        }
    }
    names.sort();
    if let Some(v) = recorder_pub_violation(&names) {
        panic!("{v}");
    }
    let expected: Vec<String> = PLATFORM_STATE_PUB_FNS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        names, expected,
        "`ImeStateHub`/`PlatformState` の `pub fn` の集合が想定と異なります。ハーネスが呼ぶものだけを pub にし、\
         増減したら `PLATFORM_STATE_PUB_FNS` を更新してください。"
    );
}

/// 本番のハブに crate の外から届く口が無いこと（`pub dispatch_event` が安全である前提）。
#[test]
fn production_hub_is_unreachable_from_outside_the_crate() {
    // (1) `PlatformState.ime` は `pub(crate)` のまま（上のテストが `pub` フィールドを禁じる）。
    let ps = non_comment_lines(production_code_only(&read_crate_file(
        "src/state/platform_state.rs",
    )));
    assert!(
        ps.contains("pub(crate) ime: ImeStateHub"),
        "`PlatformState.ime` が `pub(crate)` ではありません"
    );
    // (2) `Runtime.platform_state` は private。
    let rt = non_comment_lines(production_code_only(&read_crate_file("src/runtime/mod.rs")));
    assert!(
        !rt.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("pub") && t.contains("platform_state:")
        }),
        "`Runtime.platform_state` が pub になっています"
    );
    // (3) `ImeStateHub`・`PlatformState` を名乗って返す `pub fn` が無い（`Self` は対象外）。
    for rel in list_src_files() {
        let content = read_crate_file(&rel);
        let code = non_comment_lines(production_code_only(&content));
        let hits = pub_fns_returning(&code, &["PlatformState", "ImeStateHub"]);
        assert!(
            hits.is_empty(),
            "`{rel}`: 本番のハブへの口になりうる pub fn があります: {hits:?}"
        );
    }
}

/// 上のガード自体が、`pub fn` の追加・修飾子付き・記録系の公開・pub フィールド・ハブを返す口を検出できることの確認。
#[test]
fn platform_state_pub_fn_guard_detects_additions_and_recorders() {
    let code = "    pub fn a(&self) {}\n    pub const fn b(&self) {}\n    pub(crate) fn c(&self) {}\n    fn d(&self) {}\n    pub unsafe fn e(&self) {}\n    pub async fn f(&self) {}\n    pub extern \"C\" fn g(&self) {}\n    pub const unsafe fn h(&self) {}\n    pub fn record_confirmed_in_scope(&mut self) {}\n";
    let names = pub_fn_names(code);
    assert_eq!(
        names,
        ["a", "b", "e", "f", "g", "h", "record_confirmed_in_scope"]
    );
    assert!(recorder_pub_violation(&names).is_some());
    assert!(
        recorder_pub_violation(&["a".to_string(), "record_explicit_intent".to_string()]).is_none()
    );
    let unsafe_rec = pub_fn_names("    pub unsafe fn record_confirmed(&mut self) {}\n");
    assert!(recorder_pub_violation(&unsafe_rec).is_some());

    let block = "{\n    pub(crate) belief: B,\n    pub shadow_model: M,\n    clock: C,\n    pub fn x(&self) {}\n}";
    assert_eq!(pub_field_lines(block), ["pub shadow_model: M,"]);

    let code = "    pub fn hub(&mut self) -> &mut ImeStateHub {\n        &mut self.ime\n    }\n    pub fn ok(&self) -> Self {\n        todo!()\n    }\n    pub fn multi(\n        &mut self,\n    ) -> &PlatformState {\n        todo!()\n    }\n";
    assert_eq!(
        pub_fns_returning(code, &["PlatformState", "ImeStateHub"]),
        ["hub", "multi"]
    );
}

/// `runtime/ime_refresh.rs` の本番コードが、打鍵中判定を `is_typing`(`state/ime_read_strategy.rs`)経由で行い、
/// `TYPING_IDLE_MS` を直接比較していないこと。`runtime/` は Windows 限定で単体テストが observe を通らないため、
/// `idle_ms < TYPING_IDLE_MS` に戻されても全数表(decide 側)は落ちない。それを文字列照合で固定する。
fn ime_refresh_uses_is_typing_only(src: &str) -> bool {
    let code = non_comment_lines(production_code_only(src));
    !code.contains("TYPING_IDLE_MS") && code.contains("is_typing(")
}

#[test]
fn read_strategy_observe_goes_through_is_typing() {
    let src = read_crate_file("src/runtime/ime_refresh.rs");
    assert!(
        ime_refresh_uses_is_typing_only(&src),
        "runtime/ime_refresh.rs が打鍵中判定を is_typing 経由で行っていません(TYPING_IDLE_MS を直接比較していないか、\
         is_typing( を呼んでいるかを確認)。decide 側(state/ime_read_strategy.rs)と式がずれると、\
         通過マークを読まないまま SkipTyping に落ちます。"
    );
}

#[test]
fn ime_refresh_is_typing_guard_detects_violations() {
    assert!(ime_refresh_uses_is_typing_only(
        "let m = is_typing(idle_ms) && live(now);\n// TYPING_IDLE_MS はコメント\n"
    ));
    assert!(!ime_refresh_uses_is_typing_only(
        "let m = idle_ms < TYPING_IDLE_MS && live(now);\n"
    ));
    assert!(!ime_refresh_uses_is_typing_only("let m = live(now);\n"));
    assert!(!ime_refresh_uses_is_typing_only(
        "let m = is_typing(idle_ms);\nlet t = idle_ms <= crate::tuning::TYPING_IDLE_MS;\n"
    ));
}


/// FCIS F3: relay の「即時/キュー/ガード」の判断は `state/relay_plan.rs` に出した。触る辺を固定する。
/// - e12: `execute_relay` の Consume は Timer だけを即時に実行する（`plan_consume_effect` 経由。Immediate が Queue より先に実行される位置にある）。
/// - e13: `handle_reinject` は `OutputActiveGuard::begin()` を `spawn_local` の**前**に取る。
/// - c21・c22 の対（ADR-156）: defer 側（`run_passthrough_pipeline`）と drain 側（`reinject_wait_remaining`）が
///   どちらも `relay_plan` の同じ閾値判断（`output_guard_remaining_ms` 系）を使う。片方だけ条件を足さないこと。
#[test]
fn relay_plan_edges_e12_e13_and_defer_drain_pair_are_pinned() {
    let executor = read_crate_file("src/runtime/executor.rs");
    let prod = production_code_only(&executor);

    let relay = non_comment_lines(extract_fn_body(prod, "fn execute_relay("));
    assert!(
        relay.contains("plan_relay(") && relay.contains("plan_consume_effect("),
        "execute_relay は判断を relay_plan::plan_relay / plan_consume_effect に委ねること（e12）"
    );
    assert!(
        relay.contains("matches!(effect, Effect::Timer(_))"),
        "execute_relay は Timer だけを即時の対象にすること（e12）"
    );
    let imm = relay.find("EffectRoute::Immediate").expect("Immediate arm");
    let queue = relay.find("EffectRoute::Queue").expect("Queue arm");
    assert!(
        imm < queue,
        "Immediate（Timer の即時実行）が Queue より先であること（e12）"
    );

    let reinject = non_comment_lines(extract_fn_body(prod, "fn handle_reinject("));
    let begin = reinject
        .find("OutputActiveGuard::begin()")
        .expect("OutputActiveGuard::begin()");
    let spawn = reinject.find("spawn_local(").expect("spawn_local(");
    assert!(
        begin < spawn,
        "OutputActiveGuard::begin() は spawn_local の前に取ること（e13）"
    );

    let defer = non_comment_lines(extract_fn_body(prod, "fn run_passthrough_pipeline("));
    let drain = non_comment_lines(extract_fn_body(prod, "fn reinject_wait_remaining("));
    assert!(
        defer.contains("relay_plan::output_in_flight("),
        "defer 側（run_passthrough_pipeline）は relay_plan::output_in_flight を使うこと（c21・c22）"
    );
    assert!(
        drain.contains("relay_plan::reinject_wait_remaining("),
        "drain 側（reinject_wait_remaining）は relay_plan::reinject_wait_remaining を使うこと（c21・c22）"
    );
    assert!(
        !defer.contains("< crate::tuning::OUTPUT_GUARD_MS") && !drain.contains("OUTPUT_GUARD_MS -"),
        "出力ガードの閾値判断を executor.rs にインラインで書き戻さないこと（relay_plan に一本化）"
    );
}
