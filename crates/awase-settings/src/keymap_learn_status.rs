//! ADR196-T4: 較正パネルの「使用中の予測表」状態表示(1行、事実ベース)。
//!
//! 表示文言の決定は純粋関数([`TableState::from_inputs`]・[`TableState::status_line`])に
//! 閉じ、ファイルI/O・版取得(ブロックしうる)は呼び出し側が行う。

use awase_keymap_learn::judgement::{NeedsConfirmationReason, RejectedReason, TableJudgement};
use awase_keymap_learn::persist::PersistedTable;
use awase_keymap_learn::revalidation::{
    EnvVersion, EnvVersionProbe, StoredEnvVersion, needs_revalidation,
};

/// 学習したが採用されなかった理由(表示用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAdoptedReason {
    /// 自己検証の正答率が閾値未満。`None`は採点結果が表ファイルに無い場合。
    LowAccuracy(Option<u8>),
    /// 縮退・標本数不足で予測できないキーが多い。
    ManyUnpredictable,
    /// 学習時にキーマップ設定を読み取れず、構成を識別する指紋を残せなかった。
    FingerprintUnavailable,
}

/// awase.exe(`key_effect_runtime::validate_and_convert`)が、判定`Accepted`の学習表を
/// それでも使わない理由(表示用)。呼び出し側が同じ関数を呼んで求める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRejection {
    /// 変換できたセルの割合(百分率)が閾値未満。
    CoverageTooLow { coverage_percent: u8 },
}

/// 表の状態(表示行の分類)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableState {
    /// 学習表を使っていない(未学習、または旧形式で判定が無く読み手が不採用にする表)。
    Bundled,
    Learned {
        /// `YYYY-MM-DD`(表ファイルの更新日、呼び出し側が渡す)。
        date: Option<String>,
        accuracy_percent: Option<u8>,
    },
    NeedsRevalidation {
        date: Option<String>,
        accuracy_percent: Option<u8>,
        stored: StoredEnvVersion,
        current: EnvVersionProbe,
    },
    NotAdopted(NotAdoptedReason),
    /// `general.use_learned_keymap_table=false`(opt-out)で、学習表があっても内蔵表を使う。
    LearnedDisabled,
    /// 学習時の判定は`Accepted`だが、awase.exeの読み込み時検証で棄却され内蔵表を使う。
    LearnedRejectedAtRuntime(RuntimeRejection),
    PendingSystematicMismatch {
        mismatch_percent: u8,
    },
    PendingUnverifiedMsImeNative,
    /// 未学習・カスタムキーマップで内蔵表の予測がない。
    NoPrediction,
}

/// 状態表示の入力(呼び出し側がファイル・環境から集めた事実)。
#[derive(Debug, Clone)]
pub struct StatusInputs<'a> {
    pub table: Option<&'a PersistedTable>,
    pub current_env: EnvVersionProbe,
    pub file_date: Option<String>,
    /// 内蔵表に予測が無いキーマップ構成(カスタムキーマップ)か。
    pub custom_keymap_without_prediction: bool,
    /// `general.use_learned_keymap_table`(既定`true`)。
    pub use_learned_keymap_table: bool,
    /// `judgement==Accepted`の表に対するawase.exe側の読み込み時検証の棄却理由
    /// (棄却されなければ`None`)。
    pub runtime_rejection: Option<RuntimeRejection>,
}

/// awase.exeと同じ`validate_and_convert`で読み込み時検証の棄却理由を求める。
///
/// 指紋照合(`Stale`)は、awase.exeが`KeyEffectKeymap`(GJI/MS-IME本体のキーマップ読み取り、
/// `pub(crate)`かつ`cfg(windows)`)から得る現在の指紋が要るため未対応で、ここでは表自身の
/// 指紋を「現在の指紋」として渡す(常に一致扱い)。カバレッジ不足だけを反映する。
#[must_use]
pub fn runtime_rejection_of(table: &PersistedTable) -> Option<RuntimeRejection> {
    use awase_keymap_learn::staleness::FingerprintProbe;
    use awase_windows::state::key_effect_runtime::{RejectReason, validate_and_convert};
    let current = table
        .fingerprint
        .map_or(FingerprintProbe::NotSupported, FingerprintProbe::Computed);
    match validate_and_convert(table, current) {
        Err(RejectReason::CoverageTooLow { coverage }) => {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "0.0..=1.0の割合を百分率へ丸める表示用"
            )]
            let coverage_percent = (coverage * 100.0).round().clamp(0.0, 100.0) as u8;
            Some(RuntimeRejection::CoverageTooLow { coverage_percent })
        }
        _ => None,
    }
}

fn accuracy_percent(table: &PersistedTable) -> Option<u8> {
    table.verification.map(|v| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "0.0..=1.0の正答率を百分率へ丸める表示用"
        )]
        let pct = (v.score.accuracy() * 100.0).round().clamp(0.0, 100.0) as u8;
        pct
    })
}

impl TableState {
    #[must_use]
    pub fn from_inputs(inputs: &StatusInputs<'_>) -> Self {
        let Some(table) = inputs.table else {
            return if inputs.custom_keymap_without_prediction {
                Self::NoPrediction
            } else {
                Self::Bundled
            };
        };
        if !inputs.use_learned_keymap_table {
            // awase.exeは表ファイルを読まず、常に内蔵表を使う。
            return Self::LearnedDisabled;
        }
        match table.judgement {
            None => Self::Bundled,
            Some(TableJudgement::Rejected(reason)) => Self::NotAdopted(match reason {
                RejectedReason::LowAccuracy => {
                    NotAdoptedReason::LowAccuracy(accuracy_percent(table))
                }
                RejectedReason::HighDegeneration | RejectedReason::InsufficientSamples => {
                    NotAdoptedReason::ManyUnpredictable
                }
                RejectedReason::FingerprintUnavailable => NotAdoptedReason::FingerprintUnavailable,
            }),
            Some(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::SystematicMismatch { mismatch_percent },
            )) => Self::PendingSystematicMismatch { mismatch_percent },
            Some(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::UnverifiedMsImeNative,
            )) => Self::PendingUnverifiedMsImeNative,
            Some(TableJudgement::Accepted) => {
                if let Some(reason) = inputs.runtime_rejection {
                    return Self::LearnedRejectedAtRuntime(reason);
                }
                let date = inputs.file_date.clone();
                let accuracy_percent = accuracy_percent(table);
                match table.env_version {
                    Some(stored)
                        if needs_revalidation(
                            EnvVersionProbe::from(Some(stored)),
                            inputs.current_env,
                        ) =>
                    {
                        Self::NeedsRevalidation {
                            date,
                            accuracy_percent,
                            stored,
                            current: inputs.current_env,
                        }
                    }
                    _ => Self::Learned {
                        date,
                        accuracy_percent,
                    },
                }
            }
        }
    }

    /// 「学習結果を使う」ボタン(判定書き換えモード起動)を出す状態か。
    #[must_use]
    pub const fn can_adopt(&self) -> bool {
        matches!(
            self,
            Self::PendingSystematicMismatch { .. } | Self::PendingUnverifiedMsImeNative
        )
    }

    /// 「軽量再検証」ボタンを出す状態か。
    #[must_use]
    pub const fn can_revalidate(&self) -> bool {
        matches!(self, Self::NeedsRevalidation { .. })
    }

    /// 1行の状態表示。`bundled_env`は内蔵表の測定環境(ADR196-T3の版情報、
    /// 例: 「GJI 2.30.5000.0, Windows Build 26100」。不明なら`None`)。
    #[must_use]
    pub fn status_line(&self, bundled_env: Option<&str>) -> String {
        match self {
            Self::Bundled => bundled_env.map_or_else(
                || "使用中の予測表: 内蔵表".to_string(),
                |env| format!("使用中の予測表: 内蔵表（測定環境: {env}）"),
            ),
            Self::Learned {
                date,
                accuracy_percent,
            } => format!(
                "使用中: 学習表（{}）",
                learned_detail(date.as_deref(), *accuracy_percent)
            ),
            Self::NeedsRevalidation {
                stored, current, ..
            } => format!(
                "使用中: 学習表（要再検証: {}）",
                version_change(*stored, *current)
            ),
            Self::NotAdopted(reason) => {
                let why = match reason {
                    NotAdoptedReason::LowAccuracy(Some(pct)) => format!("自己検証 {pct}%"),
                    NotAdoptedReason::LowAccuracy(None) => "自己検証の正答率が不足".to_string(),
                    NotAdoptedReason::ManyUnpredictable => "予測できないキーが多い".to_string(),
                    NotAdoptedReason::FingerprintUnavailable => {
                        "キーマップ設定を読み取れず構成を記録できなかった".to_string()
                    }
                };
                format!("学習結果を採用しませんでした（理由: {why}）")
            }
            Self::LearnedDisabled => {
                "学習表は設定（use_learned_keymap_table=false）で無効化されており、内蔵表を使用中"
                    .to_string()
            }
            Self::LearnedRejectedAtRuntime(reason) => {
                let why = match reason {
                    RuntimeRejection::CoverageTooLow { coverage_percent } => {
                        format!("使えるセルが{coverage_percent}%しかありません")
                    }
                };
                format!("内蔵表を使用中（学習表は不採用: {why}）")
            }
            Self::PendingSystematicMismatch { mismatch_percent } => format!(
                "学習結果が内蔵表と大きく異なるため保留中（{mismatch_percent}%のセルが不一致）"
            ),
            Self::PendingUnverifiedMsImeNative => {
                "Microsoft IME本体は実機での精度検証待ちのため既定では使用しません".to_string()
            }
            Self::NoPrediction => "予測表なし（カスタムキーマップ）— 学習を推奨".to_string(),
        }
    }
}

fn learned_detail(date: Option<&str>, accuracy_percent: Option<u8>) -> String {
    match (date, accuracy_percent) {
        (Some(d), Some(p)) => format!("{d}学習、自己検証 {p}%"),
        (Some(d), None) => format!("{d}学習"),
        (None, Some(p)) => format!("自己検証 {p}%"),
        (None, None) => "学習済み".to_string(),
    }
}

fn dotted(v: EnvVersion) -> String {
    let [major, minor, build, rev] = v.0;
    format!("{major}.{minor}.{build}.{rev}")
}

fn version_change(stored: StoredEnvVersion, current: EnvVersionProbe) -> String {
    let from = match stored {
        StoredEnvVersion::Known(v) => format!("GJI {}", dotted(v)),
        StoredEnvVersion::Unconfirmed => "学習時の版が未確認".to_string(),
    };
    match current {
        EnvVersionProbe::Known(v) => format!("{from} → {}", dotted(v)),
        EnvVersionProbe::Unconfirmed | EnvVersionProbe::Unknown => {
            format!("{from} → 現在の版が未確認")
        }
    }
}

/// UNIX秒を`utc_offset_secs`(ローカル時刻のUTCからの差、JSTなら`32400`)ずらして
/// `YYYY-MM-DD`に整形する(表ファイルの更新日表示用、日付ライブラリを引かないための
/// 最小実装。Howard Hinnantのcivil_from_days)。
#[must_use]
pub fn format_ymd(unix_secs: u64, utc_offset_secs: i64) -> String {
    let local = i64::try_from(unix_secs).unwrap_or(0) + utc_offset_secs;
    let days = local.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// 現在のローカルタイムゾーンのUTCからの差(秒)。取得できない環境は0(UTC)。
#[cfg(windows)]
#[must_use]
pub fn local_utc_offset_secs() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    // GetTimeZoneInformationの戻り値(TIME_ZONE_ID_STANDARD=1, TIME_ZONE_ID_DAYLIGHT=2)。
    const STANDARD: u32 = 1;
    const DAYLIGHT: u32 = 2;
    let mut tzi = TIME_ZONE_INFORMATION::default();
    // SAFETY: `tzi`は有効な出力先。
    let id = unsafe { GetTimeZoneInformation(&raw mut tzi) };
    let extra = match id {
        STANDARD => tzi.StandardBias,
        DAYLIGHT => tzi.DaylightBias,
        _ => 0,
    };
    // Biasは「UTC = ローカル + Bias」(分)なので符号を反転する。
    -i64::from(tzi.Bias + extra) * 60
}

#[cfg(not(windows))]
#[must_use]
pub const fn local_utc_offset_secs() -> i64 {
    0
}

/// 別スレッドで取得した現在の環境の事実（IME本体版と、使用中IMEのキーマップ構成）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvSnapshot {
    pub version: EnvVersionProbe,
    /// 使用中のIMEがGJIで、そのキーマップが内蔵表を持たない構成（カスタムキーマップ等）か。
    /// GJI以外のIMEや`config1.db`が読めない場合は`false`。
    pub custom_keymap_without_prediction: bool,
}

impl EnvSnapshot {
    pub const UNKNOWN: Self = Self {
        version: EnvVersionProbe::Unknown,
        custom_keymap_without_prediction: false,
    };
}

/// 現在のIME本体版(フィンガープリント)の取得状態。ブロックしうるWin32呼び出しを
/// UIスレッドから外すため別スレッドで走らせ、結果をチャネルで受け取る。
/// `process_start`は取得側プロセス(awase-settings)の起動時刻で、Converterの更新時刻が
/// これより新しいと「学習中に版が変わった可能性」として`Unconfirmed`になる
/// ([`awase_keymap_learn::revalidation::classify_converter_version`])。呼び出しごとの
/// `now()`ではなく固定のプロセス起動時刻を使うことで、settings起動後に更新された
/// Converterを正しく検出する。
#[derive(Debug)]
pub struct EnvProbe {
    current: EnvSnapshot,
    rx: Option<std::sync::mpsc::Receiver<EnvSnapshot>>,
    started: bool,
    pub process_start: std::time::SystemTime,
}

impl EnvProbe {
    #[must_use]
    pub fn new(process_start: std::time::SystemTime) -> Self {
        Self {
            current: EnvSnapshot::UNKNOWN,
            rx: None,
            started: false,
            process_start,
        }
    }

    /// 取得を開始すべきか(未開始のときだけ`true`)。
    #[must_use]
    pub const fn needs_start(&self) -> bool {
        !self.started
    }

    /// 取得スレッドの受信側を登録する。
    pub fn attach(&mut self, rx: std::sync::mpsc::Receiver<EnvSnapshot>) {
        self.started = true;
        self.rx = Some(rx);
    }

    /// 学習・再検証・採用の完了後に呼ぶ。次の表示時に版を取り直す。取得が完了するまで
    /// [`Self::is_pending`]が真で、呼び出し側は古い版で状態を再計算してはならない。
    pub fn request_reprobe(&mut self) {
        self.started = false;
        self.rx = None;
    }

    /// 取得中(開始前を含む)か。
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.rx.is_some() || !self.started
    }

    /// 結果が届いていれば取り込み、更新があったら`true`。
    pub fn poll(&mut self) -> bool {
        let Some(probe) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.current = probe;
        self.rx = None;
        true
    }

    #[must_use]
    pub const fn current(&self) -> EnvVersionProbe {
        self.current.version
    }

    #[must_use]
    pub const fn custom_keymap_without_prediction(&self) -> bool {
        self.current.custom_keymap_without_prediction
    }
}

/// 学習を勧める文言(症状ベース、ADR196-T4実装対象4)。
pub const LEARNING_RECOMMENDATION: &str =
    "IME状態の表示やNICOLA入力の開閉が実際の入力とずれることがある場合、学習を実行してください。";

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted_table(env: Option<StoredEnvVersion>) -> PersistedTable {
        let mut t = PersistedTable::new(Vec::new())
            .with_judgement(TableJudgement::Accepted)
            .with_env_version(env);
        t.verification = None;
        t
    }

    fn inputs(table: Option<&PersistedTable>, env: EnvVersionProbe) -> StatusInputs<'_> {
        StatusInputs {
            table,
            current_env: env,
            file_date: Some("2026-09-23".to_string()),
            custom_keymap_without_prediction: false,
            use_learned_keymap_table: true,
            runtime_rejection: None,
        }
    }

    fn pcell(key: u16, predicted: bool) -> awase_keymap_learn::persist::PersistedCell {
        use awase_keymap_learn::model::{Disposition, KeyId, Outcome, Status};
        let status = Status {
            open: true,
            mode: 0x09,
            composing: false,
        };
        awase_keymap_learn::persist::PersistedCell {
            status,
            key: KeyId(key),
            prediction: predicted.then_some(Outcome {
                status,
                disp: Disposition::Kept,
            }),
        }
    }

    /// 判定`Accepted`でも、awase.exeの`validate_and_convert`がカバレッジ不足で棄却する表は
    /// 「使用中: 学習表」にならない(俯瞰レビューA-2条件2、awase.exeと同じ関数を呼ぶ)。
    #[test]
    fn low_coverage_table_is_reported_as_rejected_at_runtime() {
        // 表に無いVK(0x99)のセルは変換不能: 10セル中2セットだけ使える(20% < 80%)。
        let mut cells = vec![pcell(0xF2, true), pcell(0xF3, true)];
        cells.extend((0..8).map(|_| pcell(0x99, false)));
        let t = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        let rej = runtime_rejection_of(&t);
        assert!(
            matches!(rej, Some(RuntimeRejection::CoverageTooLow { .. })),
            "{rej:?}"
        );
        let mut i = inputs(Some(&t), EnvVersionProbe::Unknown);
        i.runtime_rejection = rej;
        let line = TableState::from_inputs(&i).status_line(None);
        assert!(line.starts_with("内蔵表を使用中"), "{line}");
    }

    #[test]
    fn well_covered_table_has_no_runtime_rejection() {
        let cells: Vec<_> = (0..10).map(|_| pcell(0xF2, true)).collect();
        let t = PersistedTable::new(cells).with_judgement(TableJudgement::Accepted);
        assert_eq!(runtime_rejection_of(&t), None);
    }

    #[test]
    fn runtime_rejection_is_not_reported_as_learned() {
        let t = accepted_table(Some(StoredEnvVersion::Known(EnvVersion([2, 30, 1, 0]))));
        let rej = RuntimeRejection::CoverageTooLow {
            coverage_percent: 60,
        };
        let mut i = inputs(Some(&t), EnvVersionProbe::Known(EnvVersion([2, 30, 2, 0])));
        i.runtime_rejection = Some(rej);
        let s = TableState::from_inputs(&i);
        assert_eq!(s, TableState::LearnedRejectedAtRuntime(rej));
        assert!(!s.can_revalidate() && !s.can_adopt());
        let line = s.status_line(None);
        assert!(line.starts_with("内蔵表を使用中"), "{line}");
        assert!(!line.contains("使用中: 学習表"), "{line}");
    }

    #[test]
    fn opt_out_is_not_reported_as_learned() {
        let t = accepted_table(None);
        let mut i = inputs(Some(&t), EnvVersionProbe::Unknown);
        i.use_learned_keymap_table = false;
        let s = TableState::from_inputs(&i);
        assert_eq!(s, TableState::LearnedDisabled);
        assert!(!s.status_line(None).contains("使用中: 学習表"));
        // 要再検証になる版差があってもopt-outが優先(awase.exeは表を読まない)。
        let t = accepted_table(Some(StoredEnvVersion::Known(EnvVersion([2, 30, 1, 0]))));
        let mut i = inputs(Some(&t), EnvVersionProbe::Known(EnvVersion([2, 30, 2, 0])));
        i.use_learned_keymap_table = false;
        assert_eq!(TableState::from_inputs(&i), TableState::LearnedDisabled);
    }

    #[test]
    fn formats_unix_secs_as_date() {
        assert_eq!(format_ymd(0, 0), "1970-01-01");
        assert_eq!(format_ymd(1_789_000_000, 0), "2026-09-10");
        assert_eq!(format_ymd(1_709_164_800, 0), "2024-02-29");
    }

    #[test]
    fn date_uses_local_offset_across_midnight() {
        // 2026-09-22 20:00:00 UTC は JST(+9h) では 2026-09-23 05:00。
        let utc_evening = 1_789_000_000 + 86_400 * 12 + 20 * 3600 - (1_789_000_000 % 86_400);
        assert_eq!(format_ymd(utc_evening, 0), "2026-09-22");
        assert_eq!(format_ymd(utc_evening, 9 * 3600), "2026-09-23");
        assert_eq!(format_ymd(utc_evening, -9 * 3600), "2026-09-22");
    }

    fn snap(version: EnvVersionProbe) -> EnvSnapshot {
        EnvSnapshot {
            version,
            custom_keymap_without_prediction: true,
        }
    }

    #[test]
    fn env_probe_reprobe_blocks_stale_state_until_result() {
        let mut probe = EnvProbe::new(std::time::SystemTime::UNIX_EPOCH);
        assert!(probe.needs_start() && probe.is_pending());
        let (tx, rx) = std::sync::mpsc::channel();
        probe.attach(rx);
        assert!(!probe.needs_start() && probe.is_pending());
        assert!(!probe.poll(), "結果が来るまで更新なし");
        let v1 = EnvVersionProbe::Known(EnvVersion([1, 0, 0, 0]));
        tx.send(snap(v1)).unwrap();
        assert!(probe.poll());
        assert!(!probe.is_pending());
        assert_eq!(probe.current(), v1);
        assert!(probe.custom_keymap_without_prediction());

        probe.request_reprobe();
        assert!(
            probe.needs_start() && probe.is_pending(),
            "完了後は再取得が必要"
        );
        assert_eq!(probe.current(), v1, "新しい結果が来るまで旧値は保持");
        let (tx, rx) = std::sync::mpsc::channel();
        probe.attach(rx);
        let v2 = EnvVersionProbe::Known(EnvVersion([2, 0, 0, 0]));
        tx.send(snap(v2)).unwrap();
        assert!(probe.poll());
        assert_eq!(probe.current(), v2);
    }

    #[test]
    fn bundled_shows_measurement_env() {
        let s = TableState::from_inputs(&inputs(None, EnvVersionProbe::Unknown));
        assert_eq!(s, TableState::Bundled);
        assert_eq!(
            s.status_line(Some("GJI 2.30.1.0, Windows Build 26100")),
            "使用中の予測表: 内蔵表（測定環境: GJI 2.30.1.0, Windows Build 26100）"
        );
        assert_eq!(s.status_line(None), "使用中の予測表: 内蔵表");
    }

    #[test]
    fn learned_shows_date() {
        let t = accepted_table(None);
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert_eq!(s.status_line(None), "使用中: 学習表（2026-09-23学習）");
        assert!(!s.can_revalidate() && !s.can_adopt());
    }

    #[test]
    fn same_version_is_not_revalidation() {
        let v = EnvVersion([2, 30, 1, 0]);
        let t = accepted_table(Some(StoredEnvVersion::Known(v)));
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Known(v)));
        assert!(matches!(s, TableState::Learned { .. }));
    }

    #[test]
    fn version_change_needs_revalidation() {
        let t = accepted_table(Some(StoredEnvVersion::Known(EnvVersion([2, 30, 1, 0]))));
        let s = TableState::from_inputs(&inputs(
            Some(&t),
            EnvVersionProbe::Known(EnvVersion([2, 30, 2, 0])),
        ));
        assert!(s.can_revalidate());
        assert_eq!(
            s.status_line(None),
            "使用中: 学習表（要再検証: GJI 2.30.1.0 → 2.30.2.0）"
        );
    }

    #[test]
    fn rejected_reasons() {
        let mk = |r| PersistedTable::new(Vec::new()).with_judgement(TableJudgement::Rejected(r));
        let t = mk(RejectedReason::LowAccuracy);
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert_eq!(
            s.status_line(None),
            "学習結果を採用しませんでした（理由: 自己検証の正答率が不足）"
        );
        let t = mk(RejectedReason::HighDegeneration);
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert_eq!(
            s.status_line(None),
            "学習結果を採用しませんでした（理由: 予測できないキーが多い）"
        );
        let t = mk(RejectedReason::FingerprintUnavailable);
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert_eq!(
            s.status_line(None),
            "学習結果を採用しませんでした（理由: キーマップ設定を読み取れず構成を記録できなかった）"
        );
        assert_eq!(
            TableState::NotAdopted(NotAdoptedReason::LowAccuracy(Some(82))).status_line(None),
            "学習結果を採用しませんでした（理由: 自己検証 82%）"
        );
    }

    #[test]
    fn pending_states_differ_and_allow_adoption() {
        let t = PersistedTable::new(Vec::new()).with_judgement(TableJudgement::NeedsConfirmation(
            NeedsConfirmationReason::SystematicMismatch {
                mismatch_percent: 42,
            },
        ));
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert!(s.can_adopt());
        assert_eq!(
            s.status_line(None),
            "学習結果が内蔵表と大きく異なるため保留中（42%のセルが不一致）"
        );
        let t = PersistedTable::new(Vec::new()).with_judgement(TableJudgement::NeedsConfirmation(
            NeedsConfirmationReason::UnverifiedMsImeNative,
        ));
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert!(s.can_adopt());
        let line = s.status_line(None);
        assert_eq!(
            line,
            "Microsoft IME本体は実機での精度検証待ちのため既定では使用しません"
        );
        assert!(!line.contains('%'), "不一致率の数字は出さない");
    }

    #[test]
    fn no_prediction_for_custom_keymap() {
        let mut i = inputs(None, EnvVersionProbe::Unknown);
        i.custom_keymap_without_prediction = true;
        let s = TableState::from_inputs(&i);
        assert_eq!(s, TableState::NoPrediction);
        assert_eq!(
            s.status_line(None),
            "予測表なし（カスタムキーマップ）— 学習を推奨"
        );
    }

    #[test]
    fn legacy_table_without_judgement_is_bundled() {
        let t = PersistedTable::new(Vec::new());
        let s = TableState::from_inputs(&inputs(Some(&t), EnvVersionProbe::Unknown));
        assert_eq!(s, TableState::Bundled);
    }
}
