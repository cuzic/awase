#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! フォーカス検出・注入モード決定に関する型定義モジュール。
//!
//! 以前は `runtime::mod` に置かれていたが、focus 層に移動した（逆依存解消）。

use awase::config::AppOverrides;
use std::sync::{OnceLock, RwLock};

/// `app_overrides.input_relay_apps`（issue #136 / BUG-90 決定4）の
/// `ime.rs::read_ime_state_fast` 専用スナップショット。
///
/// **新しい呼び出し元を足さないこと。** `Runtime`/`FocusTracker` に到達できる
/// 呼び出し元は `FocusTracker::input_relay_apps()`（正規ルート、ロック不要）を
/// 使う（`runtime/mod.rs::on_window_focus_event` 参照）。このプロセスグローバル
/// が存在するのは `read_ime_state_fast` が `self` を持たない `pub unsafe fn`
/// であるという1点のためだけ。
///
/// `RwLock` を使う理由: このリポジトリは単一スレッド・メッセージループ駆動で
/// ロック不要が原則（CLAUDE.md「Concurrency model」）だが、
/// `read_ime_state_fast` は `read_ime_state_fast_async` 経由で
/// `offload_unsafe`（ワーカースレッド）からも直接（`open_chain.rs`、
/// メインスレッドの `spawn_local` 内）からも呼ばれる。config リロード時の
/// 書き込み（`ForceOverrides::new`）はメインスレッドから、読み取りは両方の
/// スレッドから起こりうるため、この `RwLock` は正当。共有 `RwLock` は現在3箇所:
/// ここ、TSF プロファイル列挙結果の `PROFILE_DESCRIPTIONS`、BugReport 用 IME 製品名
/// キャッシュの `TSF_OBS.ime_product_name`。
static INPUT_RELAY_APPS: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

fn input_relay_apps_cell() -> &'static RwLock<Vec<String>> {
    INPUT_RELAY_APPS.get_or_init(|| RwLock::new(Vec::new()))
}

/// `RwLock` が poison していても中身は捨てない（`/code-review` 指摘: 以前は
/// `unwrap_or_default()` で空配列にフォールバックしていたが、これだと poison
/// 後は `input_relay_apps` 設定が永続的に無視される＝ issue #136 の再発防止が
/// 静かに無効化される。`clone_from` は単純な `Vec<String>` 代入で複合的な
/// 不変条件を持たないため、poison 直前の中身をそのまま使い続けて安全）。
pub(crate) fn input_relay_apps_snapshot() -> Vec<String> {
    input_relay_apps_cell()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

// ── IMM capability / 学習ストア ──
// `ImmCapability`・`ImmCapabilityStore` は Win32 を呼ばない純粋な学習・永続化なので、
// Linux でもビルド・テストされる `learning_store.rs` に置く(ADR-272 / ECRS F3)。従来の import 経路を保つため再エクスポートする。
pub use super::learning_store::{ImmCapability, ImmCapabilityStore};

// ── ForceOverrides ──

/// アプリごとのフォーカス種別オーバーライド設定を保持し、
/// 判断ロジックを提供する構造体。
///
/// `AppOverrides` をラップし、check_app_override 等を
/// メソッドとして集約することで呼び出し側 API を統一する。
///
/// 注入モードのオーバーライド（`force_tsf`・`InjectionHint`）は Tsf 注入経路の撤去
/// （2026-10-22、ADR-0004 追記）で削除した。すべて Batched VK で送る。
#[derive(Debug)]
pub struct ForceOverrides {
    inner: AppOverrides,
}

impl ForceOverrides {
    /// **副作用**: `input_relay_apps` をプロセスグローバル
    /// （`INPUT_RELAY_APPS`）へ複製する（`ime.rs::read_ime_state_fast` が
    /// `self` を持たないため、issue #136 / BUG-90 決定4）。設定リロードで
    /// 複数回呼ばれた場合は最後の呼び出しが勝つ。
    #[must_use]
    pub fn new(overrides: AppOverrides) -> Self {
        // poison していても書き込み自体は諦めない（`input_relay_apps_snapshot`
        // の doc コメント参照。`if let Ok` で握り潰すと config リロード後も
        // 古い/空の値が読まれ続けるリグレッションになる）。
        let mut apps = input_relay_apps_cell()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        apps.clone_from(&overrides.input_relay_apps);
        drop(apps);
        Self { inner: overrides }
    }

    /// `force_text` / `force_bypass` オーバーライドをチェックする。
    pub(crate) fn check_app_override(
        &self,
        process_id: u32,
        class_name: &str,
    ) -> Option<crate::focus::FocusKind> {
        if self.inner.force_text.is_empty() && self.inner.force_bypass.is_empty() {
            return None;
        }
        let process_name = super::classify::get_process_name(process_id);
        for entry in &self.inner.force_text {
            if entry.process.eq_ignore_ascii_case(&process_name)
                && entry.class.eq_ignore_ascii_case(class_name)
            {
                return Some(crate::focus::FocusKind::TextInput);
            }
        }
        for entry in &self.inner.force_bypass {
            if entry.process.eq_ignore_ascii_case(&process_name)
                && entry.class.eq_ignore_ascii_case(class_name)
            {
                return Some(crate::focus::FocusKind::NonText);
            }
        }
        None
    }

    /// `disable_apps` にマッチするプロセス名か（class 不問）。
    ///
    /// `force_bypass` と異なりウィンドウクラス名を問わない — mstsc.exe のように
    /// 複数のウィンドウクラスを取りうるアプリを丸ごと無効化するための入り口。
    #[must_use]
    pub(crate) fn is_app_disabled(&self, process_name: &str) -> bool {
        crate::state::app_suppression::matches_disabled_app(&self.inner.disable_apps, process_name)
    }

    #[must_use]
    pub(crate) fn input_relay_apps(&self) -> &[String] {
        &self.inner.input_relay_apps
    }
}
