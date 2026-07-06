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
    fs::read_to_string(Path::new(manifest_dir).join(rel_path))
        .unwrap_or_else(|e| panic!("failed to read {rel_path}: {e}"))
}

/// `#[cfg(test)]\nmod tests {` より前の「本番コード」部分だけを取り出す。
/// テストコード内での使用（意図的な stale-intent シミュレーション等）は
/// このチェックの対象外とする。
fn production_code_only(content: &str) -> &str {
    match content.find("#[cfg(test)]\nmod tests") {
        Some(idx) => &content[..idx],
        None => content,
    }
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
    let count = production.matches("ObservationSource::HeuristicDefault").count();
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
/// 現在の designated 使用箇所（各 strategy と対応）:
/// - `platform_state.rs::apply_panic_reset`        → `InputModeApplyStrategy::PanicReset`
/// - `platform_state.rs::apply_hwnd_cache_restore` → `InputModeApplyStrategy::CacheRestore`
/// - `key_pipeline.rs` (post-decision)             → `InputModeApplyStrategy::PostSetOpenEisuReset`
/// - `key_pipeline.rs` (shadow toggle OFF→ON)      → `InputModeApplyStrategy::UserImeOnEisuReset`
/// - `ime_refresh.rs`                              → `InputModeApplyStrategy::ImmBrokenCorrection` (FocusChanged)
/// - `runtime/mod.rs`                              → `InputModeApplyStrategy::ImmBrokenCorrection` (Blacklist force-ON)
///
/// 新しい能動的訂正を追加する場合は `InputModeApplyStrategy` に専用 variant を追加し
/// このカウントを更新すること。外部観測には必ず `InputModeObserved` を使うこと。
#[test]
fn input_mode_applied_construction_sites_are_accounted_for() {
    let known_sites: &[(&str, usize)] = &[
        ("src/state/platform_state.rs", 2), // PanicReset + CacheRestore
        ("src/runtime/key_pipeline.rs", 2), // PostSetOpenEisuReset + UserImeOnEisuReset
        ("src/runtime/ime_refresh.rs", 1),  // ImmBrokenCorrection (FocusChanged)
        ("src/runtime/mod.rs", 1),          // ImmBrokenCorrection (Blacklist force-ON)
    ];
    for (path, expected) in known_sites {
        let content = read_crate_file(path);
        let count = content.matches("ImeEvent::InputModeApplied {").count();
        assert_eq!(
            count, *expected,
            "{path} 内の `ImeEvent::InputModeApplied` 構築箇所数が想定({expected})と \
             異なります(実際: {count})。\n\
             新しい能動的訂正を追加する場合は `InputModeApplyStrategy` に専用 variant を追加し、\n\
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
    fn walk_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}")) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk_rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest_dir).join("src");
    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);

    let patterns = ["write_sync_key(", "write_physical_key(", "write_set_open_request("];
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
             UserImeOnEisuReset)",
        ),
    ];

    for path in &files {
        let rel = path.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
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
    fn walk_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}")) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk_rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

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
        let rel = path.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
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
