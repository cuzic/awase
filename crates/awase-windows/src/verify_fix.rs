//! CI 検証スパイク専用（ブランチ `ci/send-confirm-verify`、develop にはマージしない）。
//!
//! 環境変数 `AWASE_VERIFY_FIXES`（カンマ区切り）で修正案 A/B を実行時に切り替える。
//! `debug_assertions` が無い（release）ビルドでは常に `false` を返し、挙動は変わらない。

/// 指定フラグが有効か。release ビルドでは常に `false`。
#[must_use]
pub fn on(name: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        active().iter().any(|f| f == name)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        false
    }
}

/// 有効なフラグを返す。初回呼び出し時に1回だけ環境変数をパースする。
#[cfg(debug_assertions)]
fn active() -> &'static [String] {
    static FLAGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FLAGS.get_or_init(|| {
        std::env::var("AWASE_VERIFY_FIXES")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// 起動時に有効フラグを1回ログへ出す。
pub fn log_active() {
    #[cfg(debug_assertions)]
    tracing::warn!("[verify:fix-flags] active={}", active().join(","));
    #[cfg(not(debug_assertions))]
    tracing::warn!("[verify:fix-flags] active=");
}

/// フラグの効果が実際に発火したときに呼ぶ。
pub fn fired(name: &str) {
    tracing::warn!("[verify:fix-applied] flag={name}");
}

static LAST_APPLIED_KIND: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static LAST_APPLIED_AT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `build_ime_control_view` が最後に見た `applied`（`to_pair` 形式）を覚える。
/// `at_ms == 0` は `Optimistic`、それ以外は `Confirmed`、`None` は `Unknown`。
pub fn note_applied_seen(applied: Option<(bool, u64)>) {
    use std::sync::atomic::Ordering::Relaxed;
    match applied {
        None => LAST_APPLIED_KIND.store(0, Relaxed),
        Some((_, 0)) => LAST_APPLIED_KIND.store(1, Relaxed),
        Some((_, at_ms)) => {
            LAST_APPLIED_AT_MS.store(at_ms, Relaxed);
            LAST_APPLIED_KIND.store(2, Relaxed);
        }
    }
}

/// `(applied_kind, applied_age_ms)` を返す（`[verify:skip-gji-direct]` 用）。
#[must_use]
pub fn last_applied_for_log(now_ms: u64) -> (&'static str, Option<u64>) {
    use std::sync::atomic::Ordering::Relaxed;
    match LAST_APPLIED_KIND.load(Relaxed) {
        1 => ("optimistic", None),
        2 => (
            "confirmed",
            Some(now_ms.saturating_sub(LAST_APPLIED_AT_MS.load(Relaxed))),
        ),
        _ => ("unknown", None),
    }
}
