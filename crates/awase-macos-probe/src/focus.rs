//! NSWorkspace によるアプリアクティベーション（フォーカス）追跡。
//!
//! macOS固有のフォーカス設計は project_macos_port_strategy.md の
//! 「Windows版から転用できるもの / できないもの」を参照。
//!
//! # 何を提供するか
//!
//! - [`FocusSnapshot`] — ある時点のフォーカス状態のスナップショット
//!   （epoch / bundle identifier / process id）。
//! - [`FocusState`] — `Arc` 共有するライブなフォーカス状態。**tap callback から
//!   ロック待ちなしで**読める点が肝（下の「共有方式」参照）。
//! - macOS では [`install_focus_observer`] が `NSWorkspace` の
//!   `didActivateApplicationNotification` を購読する observer を登録し、通知の
//!   `userInfo[NSWorkspaceApplicationKey]`（= `NSRunningApplication`）から bundle
//!   identifier / process id を取り出して [`FocusState`] を更新する。
//! - [`run_focus_watch`] は `focus-watch` サブコマンドの実体（アプリ切替のたびに
//!   bundle identifier をライブ表示）。
//!
//! # 共有方式（設計レビューのホットパス・ロック競合指摘への対応）
//!
//! 読み手は 2 系統ある:
//!
//! 1. **CGEventTap callback（ホットパス、打鍵ごと）** — 各イベントに焼き込む
//!    `focus_epoch`（`u64`）と bundle side-table index（`u32`）だけが必要。これらは
//!    [`FocusState::focus_epoch`] / [`FocusState::current_bundle_index`] として
//!    **atomic ロード（ロックなし・割り当てなし）**で読む。tap callback は `RwLock`
//!    を一切触らないので、フォーカス更新中でも決してブロックしない（tap がブロック
//!    すると OS が tap を無効化しうるため、これは重要）。
//! 2. **診断 / focus-watch（低頻度）** — bundle identifier 文字列を含む完全な
//!    [`FocusSnapshot`] が欲しい。これは正規の格納先である `RwLock<FocusSnapshot>`
//!    から [`FocusState::snapshot`] で read ロックして clone する。書き込みは
//!    フォーカス変更時のみ・数フィールド代入のみで I/O もロング処理もない。
//!
//! `epoch` は `RwLock` 内 snapshot と atomic の両方に持つ（atomic 側はホットパス用の
//! ミラー）。実 API を叩くコードは `#[cfg(target_os = "macos")]` に隔離し、それ以外の
//! ホストでも [`FocusState`] のロジックはそのままコンパイル・単体テストできる。

// NSWorkspace notification observer 登録（objc2 ブロック/selector 呼び出し）に unsafe が必須。
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use crate::report::{EventLogger, EventRecord};

/// ある時点のフォーカス状態のスナップショット。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusSnapshot {
    /// フォーカス変更のたびに単調増加する世代。遅延到着イベントの並べ替え診断用。
    pub epoch: u64,
    /// アクティブなアプリの bundle identifier（取得不能なら `None`）。
    pub bundle_identifier: Option<String>,
    /// アクティブなアプリの process id（取得不能なら `None`）。
    pub process_id: Option<i32>,
}

/// `Arc` 共有するライブなフォーカス状態。共有方式はモジュール docstring 参照。
#[derive(Debug)]
pub struct FocusState {
    /// ホットパス用ミラー（`snapshot.epoch` と一致）。tap はここを atomic ロードする。
    epoch: AtomicU64,
    /// ホットパス用の bundle side-table index。tap はここを atomic ロードする。
    bundle_index: AtomicU32,
    /// 正規の完全スナップショット（低頻度の読み手用）。
    snapshot: RwLock<FocusSnapshot>,
    /// bundle identifier を intern して index 化する先。
    logger: Arc<EventLogger>,
}

impl FocusState {
    /// bundle intern 先の [`EventLogger`] を共有して、未観測状態の [`FocusState`] を作る。
    #[must_use]
    pub fn new(logger: Arc<EventLogger>) -> Arc<Self> {
        Arc::new(Self {
            epoch: AtomicU64::new(0),
            bundle_index: AtomicU32::new(EventRecord::NO_BUNDLE),
            snapshot: RwLock::new(FocusSnapshot::default()),
            logger,
        })
    }

    /// 現在の focus epoch。**tap callback からロック待ちなしで**呼べる（atomic ロード）。
    #[must_use]
    pub fn focus_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// 現在アクティブなアプリの bundle side-table index（未知は
    /// [`EventRecord::NO_BUNDLE`]）。**tap callback からロック待ちなしで**呼べ
    /// （atomic ロード）、返った index を [`EventRecord::bundle_index`] にそのまま入れる。
    #[must_use]
    pub fn current_bundle_index(&self) -> u32 {
        self.bundle_index.load(Ordering::SeqCst)
    }

    /// 現在のフォーカス状態の完全なスナップショット。`RwLock` を read ロックして
    /// clone する（低頻度の表示/診断用。tap ホットパスからは呼ばないこと）。
    #[allow(dead_code)] // まだ呼ぶ CLI/診断コードが無い（focus-watch は live 表示のみ）
    #[must_use]
    pub fn snapshot(&self) -> FocusSnapshot {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// アクティベーションを記録し、新しい epoch を返す。observer から低頻度で呼ぶ。
    ///
    /// 正規 snapshot を write ロック下で更新してから、ホットパス用 atomic を
    /// bundle_index → epoch の順で発行する。これにより、tap が新しい epoch を観測した
    /// ときには対応する bundle_index も既に見えていることを `SeqCst` 順序で保証する。
    fn record_activation(&self, bundle: Option<String>, pid: Option<i32>) -> u64 {
        let idx = bundle
            .as_deref()
            .map_or(EventRecord::NO_BUNDLE, |b| self.logger.intern_bundle(b));
        let epoch = {
            let mut snap = self
                .snapshot
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            snap.epoch = snap.epoch.wrapping_add(1);
            snap.bundle_identifier = bundle;
            snap.process_id = pid;
            snap.epoch
        };
        self.bundle_index.store(idx, Ordering::SeqCst);
        self.epoch.store(epoch, Ordering::SeqCst);
        epoch
    }
}

/// `focus-watch` サブコマンドの実体。共有 `NSWorkspace` のアクティベーション通知を
/// 購読し、アプリが切り替わるたびに bundle identifier をライブ表示する。
///
/// メインスレッドで `NSApplication` のイベントループに入り、terminate されるまで
/// ブロックする。
///
/// # Errors
///
/// メインスレッド以外から呼ばれた場合（`NSApplication` セットアップ失敗）に `Err`。
#[cfg(target_os = "macos")]
pub fn run_focus_watch() -> anyhow::Result<()> {
    let app = crate::runtime::setup_main_application()?;
    let logger = Arc::new(EventLogger::new(256));
    let state = FocusState::new(logger);
    let _guard = install_focus_observer(Arc::clone(&state), true);
    log::info!("focus-watch: NSWorkspace アクティベーション監視を開始（Ctrl-C で終了）");
    println!("focus-watch: waiting for application activation events... (Ctrl-C to stop)");
    app.run();
    Ok(())
}

/// [`run_focus_watch`] の非 macOS スタブ。
///
/// # Errors
///
/// 非 macOS では常に `Err`（unsupported platform）を返す。
#[cfg(not(target_os = "macos"))]
pub fn run_focus_watch() -> anyhow::Result<()> {
    anyhow::bail!("focus-watch is only supported on macOS")
}

// FocusObserverGuard: 型自体は install_focus_observer の戻り値として使われるが、
// 変数に束縛されず即座に drop されない用途がまだ無いため名指しでの import は未使用扱い。
#[allow(unused_imports)]
#[cfg(target_os = "macos")]
pub use mac::{install_focus_observer, FocusObserverGuard};

#[cfg(target_os = "macos")]
mod mac {
    use std::io::Write;
    use std::sync::Arc;

    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass};
    use objc2_app_kit::{
        NSRunningApplication, NSWorkspaceApplicationKey,
        NSWorkspaceDidActivateApplicationNotification,
    };
    use objc2_foundation::NSNotification;

    use crate::runtime::{with_autorelease_pool, workspace_notification_center};

    use super::FocusState;

    struct Ivars {
        state: Arc<FocusState>,
        live: bool,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "AwaseProbeFocusObserver"]
        #[ivars = Ivars]
        struct FocusObserver;

        impl FocusObserver {
            #[unsafe(method(workspaceDidActivate:))]
            fn workspace_did_activate(&self, notification: &NSNotification) {
                with_autorelease_pool(|| self.handle_activation(notification));
            }
        }

        unsafe impl NSObjectProtocol for FocusObserver {}
    );

    impl FocusObserver {
        fn new(state: Arc<FocusState>, live: bool) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { state, live });
            unsafe { msg_send![super(this), init] }
        }

        fn handle_activation(&self, notification: &NSNotification) {
            let ivars = self.ivars();
            // アクティブになったアプリはこの通知の userInfo[NSWorkspaceApplicationKey]
            // に入っている（= NSRunningApplication）。frontmostApplication を引くより
            // 通知固有で、高速切替時の取り違えが起きにくい。
            let (bundle, pid) = notification
                .userInfo()
                .and_then(|info| {
                    // SAFETY: NSWorkspaceApplicationKey は AppKit が公開する有効な
                    //         extern static NSString キー。
                    info.objectForKey(unsafe { NSWorkspaceApplicationKey })
                })
                .and_then(|obj| obj.downcast::<NSRunningApplication>().ok())
                .map_or((None, None), |app| {
                    (
                        app.bundleIdentifier().map(|s| s.to_string()),
                        Some(app.processIdentifier()),
                    )
                });
            let epoch = ivars.state.record_activation(bundle.clone(), pid);
            if ivars.live {
                println!(
                    "[focus] epoch={epoch} bundle={} pid={}",
                    bundle.as_deref().unwrap_or("<unknown>"),
                    pid.map_or_else(|| "?".to_owned(), |p| p.to_string()),
                );
                let _ = std::io::stdout().flush();
            }
            log::info!("focus activate epoch={epoch} bundle={bundle:?} pid={pid:?}");
        }
    }

    /// 登録した observer を保持し、drop 時に登録解除するガード。生かしている間だけ
    /// アクティベーション通知が届く（`addObserver:...` は observer を retain しないため）。
    pub struct FocusObserverGuard {
        observer: Retained<FocusObserver>,
    }

    impl std::fmt::Debug for FocusObserverGuard {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FocusObserverGuard").finish_non_exhaustive()
        }
    }

    impl Drop for FocusObserverGuard {
        fn drop(&mut self) {
            let center = workspace_notification_center();
            // SAFETY: 自分が登録した observer を外すだけ。observer はこの呼び出し中有効。
            unsafe { center.removeObserver(&self.observer) };
        }
    }

    /// `NSWorkspaceDidActivateApplicationNotification` を購読する observer を登録し、
    /// 生存を管理するガードを返す。メインスレッドから呼ぶこと（notification center は
    /// `!Send`）。`live=true` で切替のたびに標準出力へライブ表示する。
    #[must_use]
    pub fn install_focus_observer(state: Arc<FocusState>, live: bool) -> FocusObserverGuard {
        let observer = FocusObserver::new(state, live);
        let center = workspace_notification_center();
        // SAFETY: selector は FocusObserver が NSNotification を取る形で定義した
        // workspaceDidActivate: と一致。name はワークスペースのアクティベーション通知の
        // extern static。observer は返り値のガードが登録中ずっと生かし続ける。
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(workspaceDidActivate:),
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
            );
        }
        FocusObserverGuard { observer }
    }
}

#[cfg(test)]
mod tests {
    use super::{EventLogger, EventRecord, FocusSnapshot, FocusState};
    use std::sync::Arc;

    fn logger() -> Arc<EventLogger> {
        Arc::new(EventLogger::new(16))
    }

    #[test]
    fn new_state_is_empty() {
        let s = FocusState::new(logger());
        assert_eq!(s.focus_epoch(), 0);
        assert_eq!(s.current_bundle_index(), EventRecord::NO_BUNDLE);
        assert_eq!(s.snapshot(), FocusSnapshot::default());
    }

    #[test]
    fn record_activation_updates_snapshot_and_epoch() {
        let s = FocusState::new(logger());
        let epoch = s.record_activation(Some("com.apple.Safari".to_owned()), Some(501));
        assert_eq!(epoch, 1);
        assert_eq!(s.focus_epoch(), 1);
        let snap = s.snapshot();
        assert_eq!(snap.epoch, 1);
        assert_eq!(snap.bundle_identifier.as_deref(), Some("com.apple.Safari"));
        assert_eq!(snap.process_id, Some(501));
        assert_ne!(s.current_bundle_index(), EventRecord::NO_BUNDLE);
    }

    #[test]
    fn record_activation_none_bundle_and_pid() {
        let s = FocusState::new(logger());
        let _ = s.record_activation(None, None);
        let snap = s.snapshot();
        assert_eq!(snap.bundle_identifier, None);
        assert_eq!(snap.process_id, None);
        assert_eq!(s.current_bundle_index(), EventRecord::NO_BUNDLE);
    }

    #[test]
    fn epoch_increments_monotonically() {
        let s = FocusState::new(logger());
        let _ = s.record_activation(Some("a".to_owned()), Some(1));
        let _ = s.record_activation(Some("b".to_owned()), Some(2));
        assert_eq!(s.focus_epoch(), 2);
        assert_eq!(s.snapshot().epoch, 2);
    }

    #[test]
    fn hot_path_atomics_mirror_snapshot() {
        let s = FocusState::new(logger());
        let _ = s.record_activation(Some("com.z".to_owned()), Some(9));
        // ホットパス atomic ミラーと正規 snapshot の epoch が一致すること。
        assert_eq!(s.focus_epoch(), s.snapshot().epoch);
    }

    #[test]
    fn same_bundle_reuses_index() {
        let s = FocusState::new(logger());
        let _ = s.record_activation(Some("com.x".to_owned()), Some(1));
        let first = s.current_bundle_index();
        let _ = s.record_activation(Some("com.y".to_owned()), Some(2));
        let _ = s.record_activation(Some("com.x".to_owned()), Some(1));
        let third = s.current_bundle_index();
        assert_eq!(first, third, "same bundle id must intern to the same index");
    }
}
