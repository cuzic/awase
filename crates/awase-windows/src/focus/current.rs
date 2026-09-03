//! 現在フォーカス中のウィンドウ情報を集約する構造体。

use crate::focus::class_names::AppImeProfile;
use crate::focus::{AppKind, FocusKind};

/// 現在フォーカス中のウィンドウに関する情報。
///
/// フォーカスが変化するまで有効な状態を一箇所に集約する。
/// `pid == 0` はフォーカス未取得（起動直後等）を表す。
#[derive(Debug)]
pub struct CurrentFocus {
    pub hwnd: usize,
    /// `hwnd` の top-level 祖先ウィンドウ（`GetAncestor(hwnd, GA_ROOT)`）。
    ///
    /// 追跡している `hwnd` はフォーカス中コントロール（`hwndFocus` 等）であり、
    /// 必ずしも top-level ウィンドウとは限らない（ネイティブ Win32 マルチ
    /// フィールドダイアログでの Tab 移動等、同一 top-level ウィンドウ内でコントロール
    /// 間フォーカスが移動するケース）。`root_hwnd` はこの区別を計測するための値
    /// であり、ADR-106 決定3 の判定ロジック（`FocusFence`/`is_identity_ok`/`admit()`）
    /// には使わない——観測側 (`ImeObservation`) は `root_hwnd` を持たないため、
    /// 混ぜると構造体比較が恒常的に不一致になる（PR 109 コードレビュー指摘1
    /// Step1、known-bugs.md 参照）。
    pub root_hwnd: usize,
    pub pid: u32,
    pub class_name: String,
    /// フォーカス中アプリの IME 制御プロファイル（`class_name` から導出してキャッシュ）。
    pub app_profile: AppImeProfile,
    /// フォーカス中プロセス名（小文字、キーマップマッチング用）。
    pub process_name: String,
}

impl CurrentFocus {
    #[must_use]
    pub const fn unfocused() -> Self {
        Self {
            hwnd: 0,
            root_hwnd: 0,
            pid: 0,
            class_name: String::new(),
            app_profile: AppImeProfile::Standard,
            process_name: String::new(),
        }
    }

    /// フォーカス情報をアトミックに更新する。
    /// `app_profile` は `class_name` と `process_name` から導出してキャッシュする。
    pub fn update(&mut self, pid: u32, class_name: String, hwnd: usize, relay_apps: &[String]) {
        self.update_with_process_name(pid, class_name, hwnd, relay_apps, None);
    }

    /// 既に同一フォーカスプローブ内で取得済みの process_name があれば再利用して更新する。
    pub fn update_with_process_name(
        &mut self,
        pid: u32,
        class_name: String,
        hwnd: usize,
        relay_apps: &[String],
        process_name: Option<String>,
    ) {
        self.hwnd = hwnd;
        #[cfg(windows)]
        {
            self.process_name = process_name
                .unwrap_or_else(|| super::classify::get_process_name(pid).to_lowercase());
            self.root_hwnd = super::classify::root_hwnd_of(hwnd);
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            let _ = process_name;
            self.process_name.clear();
            self.root_hwnd = hwnd;
        }
        self.app_profile =
            AppImeProfile::from_class_and_process(&class_name, &self.process_name, relay_apps);
        self.pid = pid;
        self.class_name = class_name;
    }

    /// フォーカスが確立されているか（`pid != 0`）。
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.pid != 0
    }
}

/// journal 用に現在フォーカス先を 1 個の値として固めたスナップショット。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusIdentity {
    pub hwnd: usize,
    pub pid: u32,
    pub class_name: String,
    pub process_name: String,
    pub app_profile: AppImeProfile,
    pub app_kind: AppKind,
    pub focus_kind: FocusKind,
}

impl Default for FocusIdentity {
    fn default() -> Self {
        Self {
            hwnd: 0,
            pid: 0,
            class_name: String::new(),
            process_name: String::new(),
            app_profile: AppImeProfile::Standard,
            app_kind: AppKind::Win32,
            focus_kind: FocusKind::Undetermined,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct FocusChangedAxes {
    pub process: bool,
    pub window: bool,
    pub app_kind: bool,
    pub focus_kind: bool,
}

impl FocusIdentity {
    #[must_use]
    pub fn changed_axes(&self, next: &Self) -> FocusChangedAxes {
        FocusChangedAxes {
            process: self.pid != next.pid || self.process_name != next.process_name,
            window: self.hwnd != next.hwnd,
            app_kind: self.app_kind != next.app_kind,
            focus_kind: self.focus_kind != next.focus_kind,
        }
    }
}

impl FocusChangedAxes {
    #[must_use]
    pub const fn any(self) -> bool {
        self.process || self.window || self.app_kind || self.focus_kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> FocusIdentity {
        FocusIdentity {
            hwnd: 1,
            pid: 10,
            class_name: "A".to_owned(),
            process_name: "a.exe".to_owned(),
            app_profile: AppImeProfile::Standard,
            app_kind: AppKind::Win32,
            focus_kind: FocusKind::TextInput,
        }
    }

    #[test]
    fn changed_axes_reports_no_change() {
        let prev = identity();
        assert_eq!(prev.changed_axes(&prev), FocusChangedAxes::default());
        assert!(!prev.changed_axes(&prev).any());
    }

    #[test]
    fn changed_axes_reports_each_axis() {
        let prev = identity();
        let mut next = prev.clone();
        next.pid = 11;
        assert_eq!(
            prev.changed_axes(&next),
            FocusChangedAxes {
                process: true,
                ..FocusChangedAxes::default()
            }
        );

        let mut next = prev.clone();
        next.hwnd = 2;
        assert_eq!(
            prev.changed_axes(&next),
            FocusChangedAxes {
                window: true,
                ..FocusChangedAxes::default()
            }
        );

        let mut next = prev.clone();
        next.app_kind = AppKind::TsfNative;
        assert_eq!(
            prev.changed_axes(&next),
            FocusChangedAxes {
                app_kind: true,
                ..FocusChangedAxes::default()
            }
        );

        let mut next = prev.clone();
        next.focus_kind = FocusKind::NonText;
        assert_eq!(
            prev.changed_axes(&next),
            FocusChangedAxes {
                focus_kind: true,
                ..FocusChangedAxes::default()
            }
        );
    }

    #[test]
    fn changed_axes_reports_all_axis_combinations() {
        let prev = identity();
        for mask in 0u8..16 {
            let mut next = prev.clone();
            if mask & 0b0001 != 0 {
                next.pid = 11;
            }
            if mask & 0b0010 != 0 {
                next.hwnd = 2;
            }
            if mask & 0b0100 != 0 {
                next.app_kind = AppKind::TsfNative;
            }
            if mask & 0b1000 != 0 {
                next.focus_kind = FocusKind::NonText;
            }
            assert_eq!(
                prev.changed_axes(&next),
                FocusChangedAxes {
                    process: mask & 0b0001 != 0,
                    window: mask & 0b0010 != 0,
                    app_kind: mask & 0b0100 != 0,
                    focus_kind: mask & 0b1000 != 0,
                },
                "mask={mask:04b}"
            );
        }
    }
}
