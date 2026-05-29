//! 現在フォーカス中のウィンドウ情報を集約する構造体。

use crate::focus::class_names::AppImeProfile;

/// 現在フォーカス中のウィンドウに関する情報。
///
/// フォーカスが変化するまで有効な状態を一箇所に集約する。
/// `pid == 0` はフォーカス未取得（起動直後等）を表す。
#[derive(Debug)]
pub struct CurrentFocus {
    pub pid: u32,
    pub class_name: String,
    /// フォーカス中アプリの IME 制御プロファイル（`class_name` から導出してキャッシュ）。
    pub app_profile: AppImeProfile,
    /// フォーカス中プロセス名（小文字、キーマップマッチング用）。
    pub process_name: String,
}

impl CurrentFocus {
    pub fn unfocused() -> Self {
        Self {
            pid: 0,
            class_name: String::new(),
            app_profile: AppImeProfile::Standard,
            process_name: String::new(),
        }
    }

    /// フォーカス情報をアトミックに更新する。
    /// `app_profile` は `class_name` から導出してキャッシュする。
    pub fn update(&mut self, pid: u32, class_name: String) {
        self.process_name = super::classify::get_process_name(pid).to_lowercase();
        self.app_profile = AppImeProfile::from_class_name(&class_name);
        self.pid = pid;
        self.class_name = class_name;
    }

    /// フォーカスが確立されているか（`pid != 0`）。
    pub const fn is_focused(&self) -> bool {
        self.pid != 0
    }
}
