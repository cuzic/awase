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
fn list_src_files() -> Vec<String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src_root = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src_root, &mut files);
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

/// `#[cfg(test)]\nmod tests {` より前の「本番コード」部分だけを取り出す。
/// テストコード内での使用（意図的な stale-intent シミュレーション等）は
/// このチェックの対象外とする。
fn production_code_only(content: &str) -> &str {
    content
        .find("#[cfg(test)]\nmod tests")
        .map_or(content, |idx| &content[..idx])
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
/// (`reset_to_off_for_tsf_native_cache_miss` は 37883d0 で TsfNative SSOT 化に伴い削除済み)
///
/// Low confidence にすることで後続の実観測（Medium/High）で上書き可能にしている。
/// 「観測がない」状況を `UserImeSetIntent` で偽装することは禁止（confidence ガードをバイパスするため）。
/// 新しい使用箇所を追加する場合は、本当に「観測データが存在しない」状況かを確認し、
/// `UserImeSetIntent` ではなく `ObserverReported` + Low confidence を使う理由を明記すること。
#[test]
fn heuristic_default_observation_is_limited_to_designated_methods() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production
        .matches("ObservationSource::HeuristicDefault")
        .count();
    assert_eq!(
        count, 1,
        "{path} 内の `ObservationSource::HeuristicDefault` 使用箇所数が想定(1)と異なります(実際: {count})。\n\
         想定: reset_stale_ime_on_for_imm_broken (Imm32Unavailable entry → ON) の1箇所のみ。\n\
         (reset_to_off_for_tsf_native_cache_miss は 37883d0 で TsfNative SSOT 化に伴い削除済み)\n\
         新しい安全デフォルト推測を追加する場合は `UserImeSetIntent` を使わず \
         `ObserverReported + ObservationConfidence::Low` を使い、このカウントを更新してください。"
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
            4,
            "typed writer 定義 3 + handle_engine_set_open 内部委譲 1 (Decision 経由 \
             SetOpen — 救済: kp_stage_post_decision の PostSetOpenEisuReset)",
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
        // 結果適用点。TsfNative/Imm32Unavailable の shadow 代替観測もここに集約される。
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

/// `EngineSync::SetOpen` は `RomajiRecovered` 専用。`KatakanaShadowOff`/
/// `NativeToggleShadowOff` は `EngineSync::ReportOpenInference` を使うこと。
///
/// 2026-07-08 BUG-19 再発: `KatakanaShadowOff` が `SetOpen(true)` 経由で
/// `handle_engine_set_open` → `UserImeSetIntent{Command}` を偽装し、`desired_open`
/// を直接書き換えていた。これによりユーザーが明示的に IME OFF にした直後でも、
/// conv の一発誤読（GJI 候補ポップアップへのフォーカス flicker 等）を理由に engine
/// が勝手に ON へ戻る再発バグを起こした。修正後は `KatakanaShadowOff`/
/// `NativeToggleShadowOff` を `ReportOpenInference`（`ObserverReported` として
/// 記録するだけ、`desired_open` は変更しない、`PlatformState::
/// report_conv_open_inference()` が唯一の消費経路）に分離した。この境界が将来
/// 再び崩れないよう、`SetOpen(ConvSyncReason::KatakanaShadowOff)` /
/// `SetOpen(ConvSyncReason::NativeToggleShadowOff)` という組み合わせが本番コードに
/// 一切出現しないことを固定する。
#[test]
fn katakana_and_native_toggle_shadow_off_never_use_set_open() {
    let path = "src/state/conv_classify.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    for forbidden in [
        "SetOpen(ConvSyncReason::KatakanaShadowOff)",
        "SetOpen(ConvSyncReason::NativeToggleShadowOff)",
    ] {
        assert!(
            !production.contains(forbidden),
            "{path} に `{forbidden}` が出現しています。\n\
             KatakanaShadowOff/NativeToggleShadowOff は SetOpen（engine を直接 \
             actuate し、UserImeSetIntent{{Command}} を偽装して desired_open を \
             書き換える）を使ってはならず、必ず ReportOpenInference（ObserverReported \
             として記録するだけ）を使うこと。さもないとユーザーの明示 IME OFF が \
             conv の一発誤読で上書きされる再発バグ（2026-07-08, BUG-19 再発）が戻ります。"
        );
    }
}

/// `ObservationSource::ConvOpenInference` への参照は2箇所のみに限定される。
///
/// - `report_conv_open_inference()`: `ObserverReported` の dispatch（唯一の書き込み点）。
///   他の箇所がこの source を直接名乗って `ObserverReported` を dispatch すると、
///   実際には conv ビットからの間接推論ではない値を「conv 推論」と偽装できてしまう
///   （`ime-belief-architecture.md` が禁じる観測偽装パターンの一種）。confidence の
///   上限（Medium）も `report_conv_open_inference()` 内で固定されているため、
///   新しい呼び出し元を増やす場合はこの関数を経由すること。
/// - `check_drift_correction()`: 明示意図が無い間はこの source 単独で drift
///   correction を発火させない source-aware gate（BUG-19 再発対策）。
#[test]
fn conv_open_inference_source_is_limited_to_report_and_gate() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production
        .matches("ObservationSource::ConvOpenInference")
        .count();
    assert_eq!(
        count, 2,
        "{path} 内の `ObservationSource::ConvOpenInference` 参照箇所数が想定(2 = \
         report_conv_open_inference の dispatch + check_drift_correction の \
         source-aware gate)と異なります(実際: {count})。\n\
         conv ビット由来の open 推論の dispatch は必ず `report_conv_open_inference()` \
         経由にし、confidence の上限 (Medium) を勝手に上げないでください。"
    );
}

#[test]
fn user_intent_source_construction_is_limited_to_typed_writers() {
    let path = "src/state/platform_state.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let count = production.matches("source: UserIntentSource::").count();
    assert_eq!(
        count, 3,
        "{path} 内の `source: UserIntentSource::` リテラル構築箇所数が想定(3)と異なります(実際: {count})。\n\
         想定: write_sync_key / write_physical_key / write_set_open_request の3箇所のみ。\n\
         `UserImeSetIntent` は typed writer 経由で発行し、直接 dispatch_event() を呼ばないこと。\n\
         新しい UserIntentSource variant を追加する場合は typed writer メソッドを追加してください。"
    );
}

/// `apply_ime_open_with_belief(` の**呼び出し箇所数**を crate 全域で固定する
/// （関数定義 `fn apply_ime_open_with_belief(` は数えない）。
///
/// これは「唯一の窓口」への統合テストではなく、**新しい未レビューの呼び出し元が
/// 増えたら気づく**ための count guard である。ADR-080 / `docs/known-bugs.md` BUG-43
/// の根本原因は、raw actuation（IME open の実 actuate）を drift correction ループが
/// observe tick ごとに無限再送していたことだった。修正（タスク #14）は
/// `ir_apply_drift_correction` の actuation を `Actuation`/`FeedbackPolicy`
/// ステートマシンでゲートしたが、**raw send 呼び出し自体は同関数内にインラインで
/// 意図的に残した**（Phase 1 のスコープは「送るか否か・頻度」の制御であって、
/// raw send を別モジュールの「単一窓口」に物理的に集約することではない。全呼び出し元の
/// 棚卸しと統合は Phase 2）。
///
/// したがってこのテストは「`ir_apply_drift_correction` が raw actuation を直接
/// 呼ばないこと」は**検証しない**（それは現状の正しいコードに対して偽であり、
/// 即座に fail する）。代わりに、BUG-43 の設計欠陥（同じ actuate 呼び出しが
/// 無自覚に増殖した）と同型の増殖を検知するため、呼び出し元の総数を凍結する。
///
/// 現在の既知の呼び出し元（file : function、行番号はドリフトするため記載しない）:
/// - `platform.rs` : `apply_ime_open_with_applied`（shadow のみから belief を作る後方互換ラッパー）
/// - `runtime/mod.rs` : `apply_force_on_for_imm_broken` / `try_force_on_bootstrap`（Blacklist force-ON、2箇所）
/// - `runtime/key_pipeline.rs` : ObservedEisu 検出時の DirectInput 補正（false 送信）
/// - `runtime/ime_refresh.rs` : `ir_apply_drift_correction`（Blacklist/TsfNative の drift 訂正、ADR-080 の直接対象）
///
/// 新しい呼び出し元を追加した場合は、`ir_apply_drift_correction` と同じ
/// `Actuation` ベースのゲーティングが必要かどうか（ADR-080 / BUG-43 参照）を
/// 検討した上で、このカウントと上記一覧を更新すること。呼び出し元を削除した
/// 場合は単にカウントを更新すること。
#[test]
fn apply_ime_open_with_belief_call_sites_are_accounted_for() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    const EXPECTED_TOTAL: usize = 5;
    let mut total = 0usize;
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(path).unwrap();
        let production = production_code_only(&content);
        // 呼び出し箇所のみ数える（定義 `fn apply_ime_open_with_belief(` は除外）。
        let calls = production.matches("apply_ime_open_with_belief(").count()
            - production.matches("fn apply_ime_open_with_belief(").count();
        if calls > 0 {
            total += calls;
            breakdown.push((rel, calls));
        }
    }

    assert_eq!(
        total, EXPECTED_TOTAL,
        "`apply_ime_open_with_belief(` の呼び出し箇所数が想定({EXPECTED_TOTAL})と\
         異なります(実際: {total})。内訳: {breakdown:?}\n\
         新しい呼び出し元を追加した場合は、`ir_apply_drift_correction` と同じ \
         Actuation ベースのゲーティングが必要かどうか（ADR-080 / \
         docs/known-bugs.md BUG-43 参照 — raw actuation が observe tick ごとに\
         無限再送された設計欠陥）を検討した上で、このカウントを更新してください。\n\
         呼び出し元を削除した場合は単にこのカウントを更新してください。"
    );
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
    // `most_recent_trusted`）が読み取ることは一切無い（`grep -rn '\.journal\b'` で確認済み、
    // 全呼び出しが `.record(..)` か `.dump_to_file()` のみ）ため、不変条件6のスコープ外。
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
        ("src/runtime/conv_actuation.rs", 1), // actuate_conv_mode（ADR-084 INV-1 単一窓口）
        ("src/tsf/warmup/cold_warmup.rs", 1), // ColdWarmupSequence::run_start
        ("src/runtime/executor.rs", 1),       // dispatch_ime_set_open（ImmCross async path）
        ("src/runtime/key_pipeline.rs", 4), // kp_stage_idle_conv_check(BUG-08) / kp_reset_to_hiragana_romaji_capsoff / kp_restore_kana_from_half_width / apply_focus_probe(ImmCrossProbe kana修正)
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
        "src/runtime/conv_actuation.rs",
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
        checked, 7,
        "ActuationTarget::capture を含む spawn_local ブロックの検査対象数が \
         想定(7)と異なります。新しい経路を追加/削除した場合は \
         actuation_target_capture_call_sites_are_accounted_for と合わせて \
         この期待値も更新すること。"
    );
}
