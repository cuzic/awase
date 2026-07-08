//! 3 分割の権限モデル（listen_events / post_events / accessibility_client）。
//!
//! 型定義と API 選択は project_macos_probe_interfaces.md の「権限モデル（確定版）」
//! に従う。特に:
//! - `post_events` は `CGPreflightPostEventAccess`/`CGRequestPostEventAccess` を使う
//!   （`AXIsProcessTrusted` ではない。監視/送出/AX は別権限）。
//! - `accessibility_client` のみ `AXIsProcessTrusted`（objc2-application-services）。
//! - `Denied` バリアントは持たない。Core Graphics の preflight は bool しか返さず
//!   「未要求」と「拒否済み」を区別できないため `NotGranted` に統一する。
//!
//! macOS 実 API を叩くコードは全て `#[cfg(target_os = "macos")]` で隔離し、それ以外の
//! ホスト（Linux 開発機など）では `Unsupported` を返してコンパイルを通す。

// 公開デリゲート関数は non-macos の fallback body だけ見ると const 化できるが、macOS
// では非 const な FFI（CGPreflight* / AXIsProcessTrusted 等）を呼ぶため const にできない。
// プラットフォーム分岐由来なので missing_const_for_fn を許可する。
#![allow(clippy::missing_const_for_fn)]
// IOKit (IOHIDCheckAccess) 自前 FFI と CFDictionary 構築（AXIsProcessTrustedWithOptions
// 用）に unsafe が必須。
#![allow(unsafe_code)]

/// 各権限の確認結果。`Denied` を持たない理由はモジュール docstring 参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    Granted,
    NotGranted,
    /// non-macOS ホストの fallback 専用。macOS ターゲットビルドでは構築されない
    /// （実 API は常に Granted/NotGranted のどちらかを返すため）。
    #[allow(dead_code)]
    Unsupported,
}

/// 権限要求（TCC プロンプトを伴いうる）の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRequestOutcome {
    Granted,
    StillNotGranted,
    /// non-macOS ホストの fallback 専用。macOS ターゲットビルドでは構築されない。
    #[allow(dead_code)]
    Unsupported,
}

/// 3 系統の権限を一括で確認した結果。
#[derive(Debug, Clone, Copy)]
pub struct MacPermissions {
    /// `CGPreflightListenEventAccess`（イベント監視）。
    pub listen_events: PermissionStatus,
    /// `CGPreflightPostEventAccess`（イベント送出）。`AXIsProcessTrusted` ではない。
    pub post_events: PermissionStatus,
    /// `AXIsProcessTrusted`（AX クライアント）。Phase M0-M4 では未使用見込み。
    pub accessibility_client: PermissionStatus,
}

/// 同じ「イベント監視」権限を Core Graphics と IOKit HID の 2 系統で確認して並べた
/// 比較診断結果。OS バージョン差で両者が食い違うことがあるため `permissions status`
/// 出力で並記する用途（主権限確認手段ではない）。
// フィールドは `{:?}` (Debug) 経由でのみ読まれるが、その使用が dead_code 解析に
// 認識されないケースがあるため明示的に allow する。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticPermissionComparison {
    /// `CGPreflightListenEventAccess` の結果。
    pub cg_preflight_listen: PermissionStatus,
    /// `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)` の結果。
    pub iohid_check_listen: PermissionStatus,
}

/// 3 系統の権限を確認する。TCC プロンプトは出さない（preflight のみ）。
#[must_use]
pub fn check_all() -> MacPermissions {
    #[cfg(target_os = "macos")]
    {
        macos::check_all()
    }
    #[cfg(not(target_os = "macos"))]
    {
        MacPermissions {
            listen_events: PermissionStatus::Unsupported,
            post_events: PermissionStatus::Unsupported,
            accessibility_client: PermissionStatus::Unsupported,
        }
    }
}

/// イベント監視権限を要求する（`CGRequestListenEventAccess`、TCC プロンプトを伴いうる）。
#[must_use]
pub fn request_listen_events() -> PermissionRequestOutcome {
    #[cfg(target_os = "macos")]
    {
        macos::request_listen_events()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionRequestOutcome::Unsupported
    }
}

/// イベント送出権限を要求する（`CGRequestPostEventAccess`、TCC プロンプトを伴いうる）。
#[must_use]
pub fn request_post_events() -> PermissionRequestOutcome {
    #[cfg(target_os = "macos")]
    {
        macos::request_post_events()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionRequestOutcome::Unsupported
    }
}

/// AX クライアント権限を要求する（`AXIsProcessTrustedWithOptions` に
/// `kAXTrustedCheckOptionPrompt = true` を渡してプロンプト表示）。
#[must_use]
pub fn request_accessibility() -> PermissionRequestOutcome {
    #[cfg(target_os = "macos")]
    {
        macos::request_accessibility()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionRequestOutcome::Unsupported
    }
}

/// 「イベント監視」を Core Graphics と IOKit HID の 2 系統で確認して並記する。
#[must_use]
pub fn compare_listen_event_checks() -> DiagnosticPermissionComparison {
    #[cfg(target_os = "macos")]
    {
        macos::compare_listen_event_checks()
    }
    #[cfg(not(target_os = "macos"))]
    {
        DiagnosticPermissionComparison {
            cg_preflight_listen: PermissionStatus::Unsupported,
            iohid_check_listen: PermissionStatus::Unsupported,
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core::ffi::c_void;

    use objc2_application_services::{
        kAXTrustedCheckOptionPrompt, AXIsProcessTrusted, AXIsProcessTrustedWithOptions,
    };
    use objc2_core_foundation::{
        kCFBooleanTrue, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
        CFDictionary, CFRetained,
    };
    use objc2_core_graphics::{
        CGPreflightListenEventAccess, CGPreflightPostEventAccess, CGRequestListenEventAccess,
        CGRequestPostEventAccess,
    };

    use super::{
        DiagnosticPermissionComparison, MacPermissions, PermissionRequestOutcome, PermissionStatus,
    };

    // IOHIDRequestType / IOHIDAccessType（<IOKit/hid/IOHIDLib.h>）。plain uint32。
    const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 0;
    const K_IOHID_ACCESS_TYPE_GRANTED: u32 = 0;

    // objc2 系クレートに IOKit HID の request-access API ラッパが無いため自前宣言。
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        // extern IOHIDAccessType IOHIDCheckAccess(IOHIDRequestType requestType);
        fn IOHIDCheckAccess(request: u32) -> u32;
    }

    const fn status(granted: bool) -> PermissionStatus {
        if granted {
            PermissionStatus::Granted
        } else {
            PermissionStatus::NotGranted
        }
    }

    const fn request_outcome(granted: bool) -> PermissionRequestOutcome {
        if granted {
            PermissionRequestOutcome::Granted
        } else {
            PermissionRequestOutcome::StillNotGranted
        }
    }

    pub(super) fn check_all() -> MacPermissions {
        MacPermissions {
            listen_events: status(CGPreflightListenEventAccess()),
            post_events: status(CGPreflightPostEventAccess()),
            // SAFETY: AXIsProcessTrusted は引数を取らず TCC 状態を読むだけ。
            accessibility_client: status(unsafe { AXIsProcessTrusted() }),
        }
    }

    pub(super) fn request_listen_events() -> PermissionRequestOutcome {
        request_outcome(CGRequestListenEventAccess())
    }

    pub(super) fn request_post_events() -> PermissionRequestOutcome {
        request_outcome(CGRequestPostEventAccess())
    }

    pub(super) fn request_accessibility() -> PermissionRequestOutcome {
        let options = prompt_options();
        // SAFETY: options は None か、kAXTrustedCheckOptionPrompt をキーに持つ有効な
        // CFDictionary。AXIsProcessTrustedWithOptions は options を借用するのみ。
        let trusted = unsafe { AXIsProcessTrustedWithOptions(options.as_deref()) };
        request_outcome(trusted)
    }

    pub(super) fn compare_listen_event_checks() -> DiagnosticPermissionComparison {
        DiagnosticPermissionComparison {
            cg_preflight_listen: status(CGPreflightListenEventAccess()),
            iohid_check_listen: iohid_listen_status(),
        }
    }

    fn iohid_listen_status() -> PermissionStatus {
        // SAFETY: IOHIDCheckAccess は IOHIDRequestType の plain uint32 を取り
        // IOHIDAccessType の plain uint32 を返すだけ。ポインタ授受も CF 所有権もない。
        let access = unsafe { IOHIDCheckAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) };
        // Granted 以外（Denied / Unknown）は preflight と同じく NotGranted に潰す。
        status(access == K_IOHID_ACCESS_TYPE_GRANTED)
    }

    /// `{ kAXTrustedCheckOptionPrompt: kCFBooleanTrue }` の CFDictionary を作る。
    fn prompt_options() -> Option<CFRetained<CFDictionary>> {
        // SAFETY: 参照する extern static（kCFBooleanTrue / kAXTrustedCheckOptionPrompt /
        // kCF*DictionaryCallBacks）はいずれも CoreFoundation / HIServices が公開する
        // 有効な静的 CF オブジェクト・コールバック。keys/values は呼び出し中有効で、
        // CFDictionary が内部 retain するため呼び出し後の生存維持は不要。キー/値とも
        // CF 型なので標準 CF 型コールバックが正しい。
        unsafe {
            let prompt_true = kCFBooleanTrue?;
            let mut keys: [*const c_void; 1] =
                [core::ptr::from_ref(kAXTrustedCheckOptionPrompt).cast()];
            let mut values: [*const c_void; 1] = [core::ptr::from_ref(prompt_true).cast()];
            CFDictionary::new(
                None,
                keys.as_mut_ptr(),
                values.as_mut_ptr(),
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
        }
    }
}
