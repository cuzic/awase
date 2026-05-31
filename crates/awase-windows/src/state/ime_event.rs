//! IME 状態モデルの event 型定義 (Step 0)
//!
//! Reducer リファクタリングの足場として、IME 状態変更に関する全 event を表現する。
//! 現状 (Step 0) では event を log するのみで、本番判定には使わない。
//!
//! ## 設計原則
//!
//! - **event は immutable record**: 一度記録したら書き換えない。
//! - **時刻ではなく `seq` で順序を決める**: `GetTickCount` は wall clock 由来で
//!   逆転する可能性があるため、reducer の順序判断は必ず `EventTime::seq` を使う。
//! - **event の payload は早めに増やす**: 後で reducer が判断材料に使う情報
//!   (`hwnd`, `confidence`, `generation` 等) は最初から持たせる。

use std::time::Instant;

use crate::focus::class_names::AppImeProfile;

/// HWND の Send-safe な表現 (raw pointer 値を usize で保持)。
///
/// 実際の `HWND` は raw pointer を含むため Send/Sync ではない。
/// event log でクロススレッド伝搬される可能性があるため、ここでは値だけ保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HwndId(pub usize);

impl HwndId {
    pub const NULL: Self = Self(0);

    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl From<windows::Win32::Foundation::HWND> for HwndId {
    fn from(hwnd: windows::Win32::Foundation::HWND) -> Self {
        Self(hwnd.0 as usize)
    }
}

/// Event の時刻情報。reducer の順序判断は `seq` を使い、経過時間計算は
/// `monotonic` を使い、既存ログとの互換には `tick_ms` を使う。
#[derive(Debug, Clone, Copy)]
pub struct EventTime {
    /// 全 event を通じて単調増加する番号。順序判断はこれを使う。
    pub seq: u64,
    /// `Instant::now()` で取得した単調時刻。経過時間計算に使う。
    pub monotonic: Instant,
    /// `GetTickCount64()` 由来の ms。既存ログとの互換用。
    pub tick_ms: u64,
}

/// Intent のソース (ユーザー意図 / awase 内部判断 / 復旧措置 等)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentSource {
    /// 設定された同期キー (Shift+Space 等)
    SyncKey,
    /// 物理 KANJI 押下 (VK_F3/F4)
    PhysicalImeKey,
    /// awase 内部の判断 (Engine から SetOpen 要求等)
    Command,
    /// 復旧措置 (panic_reset 等)
    Recovery,
    /// per-HWND IME キャッシュ復元 (前回 focus 時の意図を再現)
    HwndCache,
}

/// Observation のソース (外部観測の種類)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationSource {
    /// フォーカス変更直後の同期プローブ
    FocusProbe,
    /// 500ms 周期のバックグラウンドポーリング
    ObserverPoll,
    /// GJI (GetGuiThreadInfo) 由来
    Gji,
    /// `ImmGetOpenStatus` 直接呼び出し
    ImmGetOpenStatus,
    /// TSF observer 由来
    Tsf,
    /// per-HWND IME キャッシュからの復元
    HwndCache,
}

/// 観測の信頼度。reducer が profile 別に judge する際に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObservationConfidence {
    /// 推測ベース (FocusProbe で blacklist 回避等)
    Low,
    /// 間接観測 (GJI / TSF observer)
    Medium,
    /// 直接 API 成功 (ImmGetOpenStatus 成功)
    High,
}

/// 入力 chord の種別 (Step 4 で使う、Step 0 では定義のみ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordKind {
    /// Ctrl + 無変換 → IME OFF
    CtrlMuhenkanImeOff,
    /// Ctrl + 変換 → IME ON
    CtrlHenkanImeOn,
}

/// Apply 失敗の種別 (Step 7 で使う、Step 0 では定義のみ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyError {
    /// タイムアウト
    Timeout,
    /// クロスプロセス IMM 呼び出し失敗
    CrossProcessFailed,
    /// トグル操作が unsafe（shadow 信頼度不足・focus 直後等）で送信しなかった
    UnsafeToToggle,
    /// その他
    Other,
}

/// IME 状態モデルへの全 event。
///
/// 時刻情報は `ImeEventEnvelope::time` に集約する (event 内に重複させない)。
#[derive(Debug, Clone)]
pub enum ImeEvent {
    /// ユーザー/awase が IME を toggle したい意図
    UserImeToggleIntent { source: IntentSource },

    /// ユーザー/awase が IME を ON/OFF に設定したい意図
    UserImeSetIntent { target: bool, source: IntentSource },

    /// OS への適用を開始した
    ImeApplyRequested { target: bool, generation: u64 },

    /// OS への適用が成功した (async 完了時、generation 照合必須)
    ImeApplySucceeded { target: bool, generation: u64 },

    /// OS への適用が失敗した
    ImeApplyFailed {
        target: bool,
        generation: u64,
        error: ApplyError,
    },

    /// 外部観測が値を報告した (desired を直接書き換えない)
    ObserverReported {
        open: bool,
        source: ObservationSource,
        hwnd: HwndId,
        confidence: ObservationConfidence,
    },

    /// フォーカスが変わった
    FocusChanged {
        from: Option<HwndId>,
        to: HwndId,
        profile: AppImeProfile,
    },

    /// Chord transaction の開始 (Ctrl+無変換 押下時等)
    ChordStarted { kind: ChordKind },

    /// Chord transaction の終了 (Ctrl KeyUp 等)
    ChordEnded { kind: ChordKind },

    /// desired と observed の乖離が一定時間続いた
    DriftDetected {
        desired: bool,
        observed: bool,
        duration_ms: u64,
    },
}

impl ImeEvent {
    /// `apply_ime_open` の outcome を Succeeded/Failed event に変換する。
    /// sync / async 両経路で使う single source of truth。
    #[must_use]
    pub fn from_apply_outcome(
        target: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: u64,
    ) -> Self {
        use awase::platform::ImeOpenOutcome;
        match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => Self::ImeApplySucceeded { target, generation },
            ImeOpenOutcome::Failed => Self::ImeApplyFailed {
                target,
                generation,
                error: ApplyError::CrossProcessFailed,
            },
            ImeOpenOutcome::UnsafeToToggle => Self::ImeApplyFailed {
                target,
                generation,
                error: ApplyError::UnsafeToToggle,
            },
        }
    }
}

/// Event log に積まれる envelope。時刻情報と event 本体をまとめる。
#[derive(Debug, Clone)]
pub struct ImeEventEnvelope {
    pub time: EventTime,
    pub event: ImeEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hwnd_id_null_check() {
        assert!(HwndId::NULL.is_null());
        assert!(!HwndId(0x1234).is_null());
    }

    #[test]
    fn confidence_ordering() {
        assert!(ObservationConfidence::Low < ObservationConfidence::Medium);
        assert!(ObservationConfidence::Medium < ObservationConfidence::High);
    }
}
