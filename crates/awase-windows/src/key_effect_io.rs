//! キー効果の予測・学習済み表の「殻」: ファイル・レジストリ・実行ファイル位置を読む部分
//! （`key_effect_predictor`／`key_effect_runtime` の純粋な核から切り出した、ADR-229 段階 A-4）。
//!
//! 核（`key_effect_runtime`）は「テキスト/構造体 → 採否・セルへの変換」だけを持ち、
//! ファイルシステム・レジストリ・`crate::app` の設定パス解決は一切持たない。
//! 学習済み表を実際に読む入口（`load_runtime_table`・`read_persisted_table`）と、
//! `#[cfg(windows)]` の取得口（`get_gji`／`get_native`／`get_for_keymap`）はここにある。

use std::fs;
use std::path::Path;

#[cfg(windows)]
use awase_keymap_learn::persist::Fingerprint;
use awase_keymap_learn::persist::{self, LoadError, PersistedTable};
use awase_keymap_learn::staleness::FingerprintProbe;

use crate::state::key_effect_predictor::Cell;
#[cfg(windows)]
use crate::state::key_effect_predictor::{KeyEffectKeymap, KeymapCache};
#[cfg(windows)]
use crate::state::key_effect_runtime::RuntimeTableCache;
use crate::state::key_effect_runtime::{validate_and_convert, RejectReason, MAX_TABLE_FILE_BYTES};

/// `<config dir>/keymap-learn-table.json`のパス。`config dir`は`config.toml`の親ディレクトリ
/// （`crate::app::find_config_path()`と同じ解決順、見つからなければ`None`＝未学習として扱う）。
///
/// `crate::app`（実行ファイルの位置に基づく解決）は`#[cfg(windows)]`のため、この関数もそれに合わせる
/// （`state/`は原則OS非依存。この殻は crate 直下で、このファイルパス解決だけはWindows固有の起動時パス規則に依存する）。
#[cfg(windows)]
pub(crate) fn table_file_path() -> Option<std::path::PathBuf> {
    let config_path = crate::app::find_config_path().ok()?;
    Some(config_path.parent()?.join("keymap-learn-table.json"))
}

/// 学習プロセスが不採用/要確認の結果を退避する`keymap-learn-last-attempt.json`のパス
/// （`awase-keymap-learn-win`の`--last-attempt-path`既定と同じ場所）。
#[cfg(windows)]
pub(crate) fn last_attempt_file_path() -> Option<std::path::PathBuf> {
    let config_path = crate::app::find_config_path().ok()?;
    Some(config_path.parent()?.join("keymap-learn-last-attempt.json"))
}

/// [`RuntimeTableCache::get`]の`stamp`引数（更新時刻+長さ）。ファイルが無い/読めなければ`None`
/// （`KeymapCache`の「GJI未導入」と同じ規則: 版が変わらない限り読み直さない）。
#[cfg(windows)]
pub(crate) fn table_file_stamp() -> Option<(u64, u64)> {
    let path = table_file_path()?;
    let meta = fs::metadata(&path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((mtime, meta.len()))
}

/// [`RuntimeTableCache::get`]の`load`引数。採用できなければ理由をログに残して`None`を返す
/// （呼び出し側は同梱表へフォールバックする）。
#[cfg(windows)]
pub(crate) fn load_and_log(fingerprint: Fingerprint) -> Option<Vec<Cell>> {
    let path = table_file_path()?;
    match load_runtime_table(&path, FingerprintProbe::Computed(fingerprint)) {
        Ok(cells) => {
            tracing::info!(
                "[key-effect-runtime] 学習済み表を採用: {} セル (path={})",
                cells.len(),
                path.display()
            );
            Some(cells)
        }
        Err(RejectReason::NotFound) => {
            // ファイル未学習（存在しない）は正常系、ログしない。
            None
        }
        Err(reason) => {
            tracing::warn!(
                "[key-effect-runtime] 学習済み表を不採用、同梱表へフォールバック: {reason} (path={})",
                path.display()
            );
            None
        }
    }
}

/// 学習プロセスが使う「今のキーマップの指紋」。
///
/// `awase-keymap-learn-win`が、学習時点の指紋を書き込む/再検証で照合するために使う。awase.exeの読込（`kp_predict_key_effect`）が
/// 使う指紋と同じ関数（`KeyEffectKeymap::fingerprint`）から作るので、書き手と読み手で
/// 計算方式がずれない。GJI/Microsoft IME本体以外（`Other`）は指紋方式が無い（`NotSupported`、
/// 実行時もその構成では予測しない）。GJIで`config1.db`が読めない/解析できないときは`Unavailable`。
#[cfg(windows)]
#[must_use]
pub fn current_fingerprint_probe(tip: crate::state::ime_kind::TipIdentity) -> FingerprintProbe {
    use crate::state::ime_kind::TipIdentity;
    match tip {
        TipIdentity::Gji => crate::gji_charset_autodetect::read_key_effect_keymap()
            .map_or(FingerprintProbe::Unavailable, |k| {
                FingerprintProbe::Computed(k.fingerprint())
            }),
        TipIdentity::MsImeNative => FingerprintProbe::Computed(
            crate::msime_key_assignment::read_key_effect_keymap_native().fingerprint(),
        ),
        TipIdentity::Other => FingerprintProbe::NotSupported,
    }
}

/// `std::io::Error`を、ファイル不在(正常系)とそれ以外の読み取り失敗(異常系、ログすべき)
/// とを区別できる[`RejectReason`]へ変換する。
fn io_reject_reason(e: &std::io::Error) -> RejectReason {
    if e.kind() == std::io::ErrorKind::NotFound {
        RejectReason::NotFound
    } else {
        RejectReason::Io
    }
}

/// ファイルを読み、パース・スキーマ検証・採否判定・指紋照合・縮退率チェックまで行う
/// （同梱表との突き合わせはしない。ADR-196決定1e）。
///
/// # Errors
/// 採用できない理由を[`RejectReason`]で返す。
pub fn load_runtime_table(
    path: &Path,
    current_fingerprint: FingerprintProbe,
) -> Result<Vec<Cell>, RejectReason> {
    let table = read_persisted_table(path)?;
    validate_and_convert(&table, current_fingerprint)
}

/// ファイルを読んで`PersistedTable`へパースするところまで（採否判定・変換はしない）。
/// [`load_runtime_table`]と不具合報告（`bug_report::BugReportKeymapLearnSummary`）が共有する。
///
/// # Errors
/// 読めなかった/パースできなかった理由を[`RejectReason`]で返す（採否判定由来の理由は返さない）。
pub fn read_persisted_table(path: &Path) -> Result<PersistedTable, RejectReason> {
    let meta = fs::metadata(path).map_err(|e| io_reject_reason(&e))?;
    if meta.len() > MAX_TABLE_FILE_BYTES {
        return Err(RejectReason::TooLarge);
    }
    // 0バイトは未学習と同じ扱い（scoopのpersistは、まだ存在しない永続化対象のファイルを
    // 空ファイルとして作ることがあり、壊れたファイル扱い＝パース失敗の警告にしないため）。
    // （Windowsではディレクトリの`len`も0なので、通常ファイルに限る。ディレクトリは読み取り失敗のまま）
    if meta.is_file() && meta.len() == 0 {
        return Err(RejectReason::NotFound);
    }
    let text = fs::read_to_string(path).map_err(|e| io_reject_reason(&e))?;
    persist::from_json(&text).map_err(|e| match e {
        LoadError::Parse(_) => RejectReason::Parse,
        LoadError::SchemaVersionMismatch { .. } => RejectReason::SchemaVersionMismatch,
        LoadError::DuplicateCell { .. } => RejectReason::DuplicateCell,
    })
}

/// `RuntimeTableCache` の取得口（`#[cfg(windows)]`。ファイルのスタンプ・読込を殻で束ねる）。
#[cfg(windows)]
pub trait RuntimeTableCacheShellExt {
    /// `RuntimeTableCache::get` を、予測器・警告・(B)判定で共通の検証キー（`preset`・同梱表そのままか・指紋）で呼ぶ。
    fn get_for_keymap(&mut self, now_ms: u64, keymap: &KeyEffectKeymap) -> Option<&[Cell]>;
}

#[cfg(windows)]
impl RuntimeTableCacheShellExt for RuntimeTableCache {
    fn get_for_keymap(&mut self, now_ms: u64, keymap: &KeyEffectKeymap) -> Option<&[Cell]> {
        let fingerprint = keymap.fingerprint();
        self.get(
            now_ms,
            (
                keymap.preset(),
                keymap.is_unmodified_bundled_config(),
                fingerprint,
            ),
            table_file_stamp,
            || load_and_log(fingerprint),
        )
    }
}

/// `KeymapCache` の取得口（`#[cfg(windows)]`。`config1.db`・レジストリを読む）。
#[cfg(windows)]
pub trait KeymapCacheShellExt {
    /// GJI の`config1.db`用の `KeymapCache::get`。予測（`kp_predict_key_effect`）と役割判定
    /// （`enrich_key_role`、ADR-199決定8）が**同じインスタンス・同じ引数**で呼ぶための取得部分。
    fn get_gji(&mut self, now_ms: u64) -> Option<&KeyEffectKeymap>;
    /// Microsoft IME 本体のレジストリ割り当て用（[`Self::get_gji`]と同じ趣旨）。
    fn get_native(&mut self, now_ms: u64) -> Option<&KeyEffectKeymap>;
}

#[cfg(windows)]
impl KeymapCacheShellExt for KeymapCache {
    fn get_gji(&mut self, now_ms: u64) -> Option<&KeyEffectKeymap> {
        self.get(
            now_ms,
            crate::gji_charset_autodetect::config1_db_stamp,
            crate::gji_charset_autodetect::read_key_effect_keymap,
        )
    }

    fn get_native(&mut self, now_ms: u64) -> Option<&KeyEffectKeymap> {
        self.get(
            now_ms,
            || Some(crate::msime_key_assignment::native_assignment_stamp()),
            || {
                let keymap = crate::msime_key_assignment::read_key_effect_keymap_native();
                if keymap.legacy_table_unknown() {
                    // ADR-254: 止めた理由を journal(tracing)に残す(この副作用を受けた人の報告を、
                    // ほかの原因と区別するため)。キーマップの読み直し(版が変わったとき)にだけ出る。
                    tracing::info!(
                        "[msime-legacy] 旧UIのキーテンプレート(keystyle)が既定でないため、同梱表 MSIME_NATIVE の打鍵時予測を止めます(互換 ON の Custom は無変換/変換だけ Custom の表から予測) (ADR-254)"
                    );
                }
                Some(keymap)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::key_effect_runtime::{
        convert_cells, coverage_ratio, coverage_slot_count, MIN_COVERAGE_RATIO,
    };

    const NS: FingerprintProbe = FingerprintProbe::NotSupported;

    /// CI専用(`--ignored`、環境変数`KL_TABLE_PATH`): 実機の学習プロセスが書いた
    /// `keymap-learn-table.json`を実行時の採否判定(`load_runtime_table`)に通し、
    /// 生セル数・変換できたセル数・カバレッジの分母（畳んだ後に変換対象になりえた枠数）と
    /// 採否を出力する。学習表が同梱表と違っていても棄却されない（ADR-196決定1e）ことの実機確認用。
    #[test]
    #[ignore = "CI用: 実機の学習表(KL_TABLE_PATH)が要る"]
    fn ci_real_learned_table_is_adopted() {
        let path = std::env::var("KL_TABLE_PATH").expect("KL_TABLE_PATH");
        let path = Path::new(&path);
        let persisted = read_persisted_table(path).expect("読める");
        let converted = convert_cells(&persisted.cells);
        println!(
            "CI-RESULT raw={} converted={} slots={} coverage={:.3} limit={MIN_COVERAGE_RATIO}",
            persisted.cells.len(),
            converted.len(),
            coverage_slot_count(&persisted.cells),
            coverage_ratio(&persisted.cells, converted.len()),
        );
        let result = load_runtime_table(path, NS);
        println!(
            "CI-RESULT load_runtime_table={:?}",
            result.as_ref().map(Vec::len)
        );
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn load_runtime_table_distinguishes_not_found_from_real_io_errors() {
        // ファイル不在(正常系、ログしない)と、存在するが読み取れない(異常系、ログすべき)を
        // 混同しない。存在しないパスはNotFound。
        let missing = std::env::temp_dir().join("awase_keymap_learn_table_does_not_exist.json");
        let _ = fs::remove_file(&missing);
        assert_eq!(
            load_runtime_table(&missing, NS),
            Err(RejectReason::NotFound)
        );

        // ディレクトリをファイルとして開こうとすると(存在はするが読めない)、
        // NotFoundではなくIoになる。
        let dir = std::env::temp_dir().join(format!(
            "awase_keymap_learn_table_dir_{}",
            unique_test_suffix()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).expect("create test dir");
        let result = load_runtime_table(&dir, NS);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result, Err(RejectReason::Io));
    }

    #[test]
    fn empty_file_is_treated_as_not_learned() {
        let path = std::env::temp_dir().join(format!(
            "awase_keymap_learn_table_empty_{}.json",
            unique_test_suffix()
        ));
        fs::write(&path, b"").expect("write empty file");
        let result = load_runtime_table(&path, NS);
        let _ = fs::remove_file(&path);
        assert_eq!(result, Err(RejectReason::NotFound));
    }

    fn unique_test_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    }
}
