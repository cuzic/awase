#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Phase 2: MSAA (IAccessible) によるロールベース判定

use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};

use super::classify::{ClassifyReason, ClassifyResult};
use crate::state::msaa_role_plan::{decide_msaa_role, MsaaRoleDecision};

/// `OBJID_CLIENT` — クライアント領域のアクセシブルオブジェクト
const OBJID_CLIENT: i32 = -4;

/// MSAA ロールに基づくフォーカス判定
///
/// テキスト入力ロール（Text, Document）なら TextInput、
/// 非テキストロール（ツールバー、メニュー等）なら NonText、
/// 判定不能なら Undetermined を返す。
#[must_use]
#[tracing::instrument(level = "debug", skip_all)]
pub fn msaa_classify(hwnd: HWND) -> ClassifyResult {
    // observe → decide（純粋、state/msaa_role_plan.rs）→ execute（ログと ClassifyResult 化）
    let decision = decide_msaa_role(read_msaa_role_id(hwnd));
    match decision {
        MsaaRoleDecision::TextInput(role) => {
            tracing::debug!("MSAA: {role:?} → TextInput");
            ClassifyResult {
                kind: decision.kind(),
                reason: ClassifyReason::MsaaRole(format!("{role:?}")),
            }
        }
        MsaaRoleDecision::NonText(role) => {
            tracing::debug!("MSAA: {role:?} → NonText");
            ClassifyResult {
                kind: decision.kind(),
                reason: ClassifyReason::MsaaRole(format!("{role:?}")),
            }
        }
        MsaaRoleDecision::UndeterminedUnlisted(role_id) => {
            tracing::debug!("MSAA: role={role_id} → Undetermined (not in allow/deny list)");
            ClassifyResult {
                kind: decision.kind(),
                reason: ClassifyReason::Undetermined,
            }
        }
        // 判定不能 → Undetermined
        MsaaRoleDecision::UndeterminedUnread => ClassifyResult {
            kind: decision.kind(),
            reason: ClassifyReason::Undetermined,
        },
    }
}

/// observe: MSAA でフォーカス窓のロール値を読む（失敗なら `None`）。
fn read_msaa_role_id(hwnd: HWND) -> Option<u32> {
    let mut acc: *mut std::ffi::c_void = std::ptr::null_mut();
    #[expect(clippy::cast_sign_loss)] // OBJID_CLIENT (-4) is a Windows API convention
    let objid = OBJID_CLIENT as u32;
    // SAFETY: hwnd は呼出元から渡された有効なウィンドウハンドル。
    //         acc は AccessibleObjectFromWindow の出力ポインタであり、成功時に非 null が保証される。
    let ok = unsafe { AccessibleObjectFromWindow(hwnd, objid, &IAccessible::IID, &raw mut acc) };
    if ok.is_err() || acc.is_null() {
        return None;
    }
    // SAFETY: AccessibleObjectFromWindow が成功し acc が非 null であることを直前で確認済み。
    //         IAccessible::from_raw は COM の AddRef 済みポインタをラップする。
    let accessible: IAccessible = unsafe { IAccessible::from_raw(acc) };
    let child_self = VARIANT::from(0i32); // CHILDID_SELF
                                          // SAFETY: accessible は有効な IAccessible COM インターフェース。
                                          //         child_self は CHILDID_SELF (0) で IAccessible の規約に従った有効な引数。
    let role = unsafe { accessible.get_accRole(&child_self) }.ok()?;
    #[expect(clippy::cast_sign_loss)] // MSAA role values are non-negative
    // SAFETY: role は get_accRole が返した有効な VARIANT。
    //         lVal フィールドへのアクセスは MSAA ロール値が VT_I4 型であることが仕様で保証される。
    let role_id = unsafe { role.Anonymous.Anonymous.Anonymous.lVal as u32 };
    Some(role_id)
}
