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
    const ENTRY_POINTS: [(&str, usize); 5] = [
        // 内部委譲元(platform.rs 自身): ime_refresh.rs:740 / key_pipeline.rs:741 /
        // mod.rs:890（ADR-087 §5 item14 表 #11/#4/#7）+ apply_ime_open_with_applied
        // 内部からの委譲(platform.rs:1028) = 4。
        (".apply_ime_open_with_belief(", 4),
        // executor.rs:887 / mod.rs:733（表 #1/#6）+ apply_ime_open_with_belief
        // 内部からの委譲(platform.rs:1016) = 3。
        (".apply_ime_open_with_view(", 3),
        // ime_refresh.rs:499（表 #8）+ apply_ime_open 内部からの委譲(platform.rs:729) = 2。
        (".apply_ime_open_with_applied(", 2),
        // ime_refresh.rs:534/727（表 #9/#10）= 2。
        (".set_ime_open(", 2),
        // 呼び出し元ゼロ(死んだ入口、ADR-087 §5 item14 参照、Task #28 で対処)。
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
        ("src/output/conv_actuation.rs", 1), // actuate_conv_mode（ADR-084 INV-1 単一窓口、2026-08-08 Runtime→Output移設）
        ("src/tsf/warmup/cold_warmup.rs", 1), // ColdWarmupSequence::run_start
        ("src/runtime/executor.rs", 1),      // dispatch_ime_set_open（ImmCross async path）
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
        checked, 7,
        "ActuationTarget::capture を含む spawn_local ブロックの検査対象数が \
         想定(7)と異なります。新しい経路を追加/削除した場合は \
         actuation_target_capture_call_sites_are_accounted_for と合わせて \
         この期待値も更新すること。"
    );
}

/// ADR-086 §4 INV-15（2026-08-08、Phase 2/3 実装に伴い新設）: 生の `FocusChange`
/// イベントハンドラ自体が force-write（conv-mode/open-close の実書き込み）を
/// 起こしてはならない。`BUG-59` 追補（`9c102b02`、revert 済み）はまさにこの種の
/// 関数へ直接書き込みを追加して実機事故を起こした。
///
/// - `platform.rs::gji_on_focus_change`（conv 軸、生の FocusChange イベント
///   ハンドラ本体）は `Output::on_ime_mode_focus_changed`（武装のみ）を呼ぶことは
///   許されるが、`ActuationTarget::capture`/`actuate_conv_mode`/
///   `set_ime_conv_for_target` を**直接**呼んではならない（実際の書き込みは
///   `consume_force_pending_and_actuate` からのみ起きること）。
/// - `runtime/ime_refresh.rs::ir_post_focus_change_snapshot`（open/close 軸、
///   `gji_on_focus_change` 直後——`ime_mode_focus_gen` が今回のフォーカス変更分
///   だけ進んだ直後の単一集約点。`ir_notify_focus_changed` ではない——同関数の
///   実行時点では gen がまだ古いため）は `Runtime::arm_force_open_pending`
///   （武装のみ）を呼ぶことは許されるが、`apply_ime_open_with_belief`/
///   `apply_ime_open_with_view`/`send_ime_mode_key`/`on_ime_apply_complete` を
///   **直接**呼んではならない（実際の書き込みは Phase 3 item 1 の消費点
///   `kp_run_inner::consume_force_open_pending` からのみ起きること）。
///   `apply_ime_open_with_applied`〈GJI TsfNative VK_IME_ON 強制〉/
///   `set_ime_open`〈IME OFF 強制〉は force-write とは無関係の既存機構として
///   同関数内に実在するため、禁止リストには含めず出現数を固定する
///   （`ir_post_focus_change_snapshot_write_call_sites_are_accounted_for` 参照）
///   ——ホワイトリスト除外だと将来これらのラッパー経由で force-write が
///   紛れ込んでもガードをすり抜けるため（2026-08-08 2回目 opus レビュー M2）。
/// - `runtime/mod.rs::arm_force_open_pending`（武装専用に抽出した小関数）も
///   同じ禁止リストで走査する。代入以外を含まないため必ず通るはずだが、
///   将来ここに書き込みが追加されたら即座に検知する回帰ガードとして機能する。
#[test]
fn force_write_is_not_triggered_by_raw_focus_change() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "src/platform.rs",
            "fn gji_on_focus_change",
            &[
                "ActuationTarget::capture(",
                "actuate_conv_mode(",
                "set_ime_conv_for_target(",
            ],
        ),
        (
            "src/runtime/ime_refresh.rs",
            "fn ir_post_focus_change_snapshot",
            &[
                "apply_ime_open_with_belief(",
                "apply_ime_open_with_view(",
                "send_ime_mode_key(",
                "on_ime_apply_complete(",
            ],
        ),
        (
            "src/runtime/mod.rs",
            "fn arm_force_open_pending",
            &[
                "apply_ime_open_with_belief(",
                "apply_ime_open_with_view(",
                "apply_ime_open_with_applied(",
                "send_ime_mode_key(",
                "set_ime_open(",
                "on_ime_apply_complete(",
            ],
        ),
    ];

    for (path, fn_needle, forbidden_list) in cases {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let body = extract_fn_body(production, fn_needle);

        for forbidden in *forbidden_list {
            assert!(
                !body.contains(forbidden),
                "{path}::{fn_needle} 本体に {forbidden:?} が見つかりました。\
                 生の FocusChange イベントハンドラが force-write を直接起こしています \
                 （ADR-086 INV-15 違反、BUG-59 追補 `9c102b02` と同型の事故）。\
                 書き込みは武装（`force_pending`/`force_open_pending` フラグを \
                 立てるだけ）に留め、実際の書き込みは送信要求という入力意図に \
                 紐づく唯一の消費点からのみ行うこと。"
            );
        }
    }
}

/// ADR-086 §4 INV-15（2026-08-08、2回目 opus アドバーサリアルレビュー M2）:
/// `ir_post_focus_change_snapshot` に実在する既存の open 書き込み
/// （`apply_ime_open_with_applied`〈GJI TsfNative VK_IME_ON 強制〉/
/// `set_ime_open`〈IME OFF 強制〉、force-write とは無関係）の出現数を固定する。
///
/// `force_write_is_not_triggered_by_raw_focus_change` の禁止リストからこの
/// 2 つを除外する代わりに、ここで出現数を固定することで「新しい force-write
/// 経路がこれらのラッパー経由で紛れ込んでも検知できない」という穴を塞ぐ
/// （`apply_ime_open_with_applied` は `apply_ime_open_with_belief` の薄い
/// ラッパーであり、禁止リストへの単純な追加では素通りしてしまう）。
#[test]
fn ir_post_focus_change_snapshot_write_call_sites_are_accounted_for() {
    let path = "src/runtime/ime_refresh.rs";
    let content = read_crate_file(path);
    let production = production_code_only(&content);
    let body = extract_fn_body(production, "fn ir_post_focus_change_snapshot");

    let applied_count = count_real_calls(body, "apply_ime_open_with_applied(");
    assert_eq!(
        applied_count, 1,
        "{path}::ir_post_focus_change_snapshot 内の `apply_ime_open_with_applied(` \
         出現数が想定(1 = GJI TsfNative VK_IME_ON 強制)と異なります(実際: \
         {applied_count})。新しい呼び出しを追加した場合はこの期待値を更新し、\
         それが force-write（ADR-086 INV-15 の対象）でないことを確認すること。"
    );

    // `set_ime_open(` は実呼び出し1件（IME OFF 強制）+ ログメッセージ1件
    // （`log::debug!("... set_ime_open(false) called ...")`）で計2件。
    let set_ime_open_count = count_real_calls(body, "set_ime_open(");
    assert_eq!(
        set_ime_open_count, 2,
        "{path}::ir_post_focus_change_snapshot 内の `set_ime_open(` 出現数が \
         想定(2 = 実呼び出し1件 + ログメッセージ1件)と異なります(実際: \
         {set_ime_open_count})。新しい呼び出しを追加した場合はこの期待値を \
         更新し、それが force-write でないことを確認すること。"
    );
}

/// ADR-086 §6段3-4（2026-08-08、Phase 3 設計調査に伴い新設）:
/// `ConvModePolicy::Force` の直接読み取りは `Output::is_force_policy()` の
/// 定義内 1 箇所のみに限定される。
///
/// conv 軸（`on_ime_mode_focus_changed`）と open 軸
/// （`runtime/mod.rs::apply_force_on_for_imm_broken`/`::reschedule_ime_refresh`）が
/// それぞれ独自に `matches!(.., ConvModePolicy::Force)` を書くと、INV-13
/// （軸の対称性）が要求する「同じ policy 判定関数」が構造的に保証されなくなる
/// （条件が将来ズレても気づけない）。`is_force_policy()` を唯一の判定点にする
/// ことで、新しい force-write 経路を追加するときも自然にこの1点を経由させる。
#[test]
fn force_policy_is_read_from_a_single_decision_point() {
    const NEEDLE: &str = "ConvModePolicy::Force";
    let all_files = list_src_files();
    let mut total = 0usize;
    let mut sites: Vec<(String, usize)> = Vec::new();
    for path in &all_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            total += count;
            sites.push((path.clone(), count));
        }
    }
    assert_eq!(
        total, 1,
        "`{NEEDLE}` の本番コードでの直接参照が想定(1 = \
         output/mod.rs::is_force_policy の定義のみ)と異なります。\n実際: {sites:?}\n\
         新しい force-write 経路を追加する場合は Output::is_force_policy() 経由で \
         判定し、`ConvModePolicy::Force` を直接 matches! しないこと \
         （ADR-086 §6段3-4、INV-13 の軸対称性）。"
    );
    assert_eq!(
        sites,
        vec![("src/output/mod.rs".to_string(), 1)],
        "`{NEEDLE}` の唯一の直接参照は src/output/mod.rs（is_force_policy の定義）に \
         あるはずです。実際: {sites:?}"
    );
}

/// ADR-086 §7-12（2026-08-08、2回目 opus アドバーサリアルレビュー M5）:
/// `Output::is_force_policy()` の呼び出し箇所数を固定する。
///
/// `runtime/mod.rs::reschedule_ime_refresh` は「`apply_force_on_for_imm_broken`
/// が `is_force_policy()` で即 return するため、周期リフレッシュ連鎖が復活しても
/// force-ON の周期スパムは再発しない」という**別関数の早期 return に依存する
/// 暗黙の前提**の上に成り立っている。呼び出し箇所数を固定することで、
/// 片方だけが変更されて前提が崩れることを検知する（2026-08-06〜2026-08-08 で
/// この関係が一度崩れかけた経緯があるため）。
///
/// 現在の呼び出し箇所（4）: `output/mod.rs::on_ime_mode_focus_changed`（conv 軸
/// 武装）、`runtime/mod.rs::arm_force_open_pending`（open 軸武装）、
/// `runtime/mod.rs::apply_force_on_for_imm_broken`（force policy 時は
/// 周期経路を使わない早期 return）、`runtime/mod.rs::reschedule_ime_refresh`
/// （drift correction 用の周期継続例外）。
#[test]
fn is_force_policy_call_sites_are_accounted_for() {
    const NEEDLE: &str = "is_force_policy()";
    let all_files = list_src_files();
    let mut total = 0usize;
    let mut sites: Vec<(String, usize)> = Vec::new();
    for path in &all_files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, NEEDLE);
        if count > 0 {
            total += count;
            sites.push((path.clone(), count));
        }
    }
    assert_eq!(
        total, 4,
        "`{NEEDLE}` の呼び出し箇所数が想定(4)と異なります。\n実際: {sites:?}\n\
         `runtime/mod.rs::reschedule_ime_refresh` の force policy 例外は \
         `apply_force_on_for_imm_broken` が同条件で早期 return することに \
         暗黙で依存している（ADR-086 §7-12）。呼び出し箇所を変更した場合は \
         この依存関係が崩れていないか確認し、この期待値を更新すること。"
    );
}

/// ADR-087 INV-28（実装記録 §8.10、item16(a)）:
/// force-write 経路（`force_on_and_correct_romaji` / GJI TsfNative 強制ON）は
/// `applied` に `None` を渡すことで `GjiDirectStrategy::apply`
/// （`ime_controller.rs:110`、`shadow_on == true` のとき `VK_IME_ON` を
/// no-op skip する）を最初から bypass する設計になっている
/// （`build_ime_control_view(None)` → `applied.unwrap_or((false, 0))` →
/// `control.shadow_on = false`、`platform.rs::build_ime_control_view` 参照）。
///
/// この不変条件が崩れる（`None` の代わりに実 `applied` 値を渡すよう変更される）と、
/// force-ON 経路が古い shadow_on=ON を見て no-op に阻まれ、BUG-16 が実装レベルで
/// 再発しうる。「`applied` を `None` にして bypass する」という意図はコメントでしか
/// 表現されておらず、コンパイラは強制しないため、テキスト走査で固定する。
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

    let ime_refresh_rs = read_crate_file("src/runtime/ime_refresh.rs");
    let ime_refresh_production = production_code_only(&ime_refresh_rs);
    let focus_change_body =
        extract_fn_body(ime_refresh_production, "fn ir_post_focus_change_snapshot");
    assert_eq!(
        count_real_calls(
            focus_change_body,
            ".apply_ime_open_with_applied(true, None)"
        ),
        1,
        "GJI TsfNative 入場時の強制ON（ir_post_focus_change_snapshot）は \
         apply_ime_open_with_applied(true, None) 経由で shadow_on=false を作ることで \
         GJI の no-op skip を bypass する設計。`None` 以外の値を渡すよう変更された場合、\
         ADR-087 INV-28 の前提が崩れる。"
    );
}

// ── ADR-089 Phase B（§2.3・§6 item 6/7）─────────────────────────────────────

/// `Actuation<Warranted>` を **warrant なしで**作る暫定入口
/// （`warrant_pending_adr087`）の呼び出し箇所数を固定する。
///
/// ADR-087 の `issue_open_warrant()` は本番未配線（呼び出し元ゼロ、2026-08-12
/// 確認）であり、既存の apply 経路は `OpenWarrant` を持たない。Phase B は
/// ADR-087 Phase 3 を巻き込まないため、warrant を持たない経路のための
/// 名前付きの入口を分けた（`state/actuation_chain.rs` モジュール doc の
/// 「差分」3）。
///
/// **この件数は増やさないこと。** ADR-087 Phase 3 の配線が進むたびに、この
/// 期待値は減っていくのが正しい方向である。
#[test]
fn legacy_unwarranted_actuation_sites_are_accounted_for() {
    // ime_controller.rs（同期チェーン）と runtime/open_chain.rs（非同期チェーン）
    // の 2 箇所のみ。
    const EXPECTED: usize = 2;
    let files = list_src_files();
    let mut total = 0usize;
    let mut breakdown: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let content = read_crate_file(path);
        let production = production_code_only(&content);
        let count = count_real_calls(production, "warrant_pending_adr087(");
        if count > 0 {
            total += count;
            breakdown.push((path.clone(), count));
        }
    }
    assert_eq!(
        total, EXPECTED,
        "`warrant_pending_adr087(` の呼び出し箇所数が想定({EXPECTED})と異なります\
         (実際: {total})。内訳: {breakdown:?}\n\
         ADR-087 の `issue_open_warrant()` を通さずに actuation を起こす経路を\
         増やさないでください（ADR-089 §2.3、`state/actuation_chain.rs` 参照）。"
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
