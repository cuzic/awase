//! ADR-265 段階 1: mozc-readable が書き出す共有メモリ（48 バイト、version 2）の**解釈**（純粋）。
//!
//! 仕様の出典は mozc-readable の `readable/docs/shared-memory.md`（awase 側に写しは無い）。ここは、読み取りの規約
//! （ADR-265 D3: 受け入れ条件）を、OS の呼び出しから切り離して固定する層である。
//!
//! - Win32 の読み取り（`OpenFileMappingW`／`NtOpenSection`、seqlock の再試行）は `observer/mozc_shm.rs`。
//! - この層は、読み取ったバイト列を解釈して「受け入れる／捨てる（理由つき）」を決め、journal の記録に変換するだけ。
//! - **belief には入れない**（段階 1 は journal のみ。信頼度の議論は段階 2。ADR-265 D3）。
//!
//! 状態を表す型に `ObservationSource` を足さない理由: 段階 1 は belief に入れないため、`ImeEvent` にも `ObservationSource`
//! にもならない。段階 2 で belief へ流すときに、別レビューで足す。

use crate::journal::MozcShmRecord;

/// 共有メモリのブロックの大きさ（バイト）。
pub const BLOCK_SIZE: usize = 48;

/// `magic`（メモリ上のバイト列は `4D 44 52 5A`、ASCII の `MDRZ`）。
pub const MAGIC: u32 = 0x5A52_444D;

/// 受け入れる最小の `version`。version 1 には `active` が無く、IME を切り替えた後も最後の状態が残るので捨てる。
pub const MIN_VERSION: u32 = 2;

/// ブロックの生の値（`TipSharedState`、リトルエンディアン）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawBlock {
    /// `magic`
    pub magic: u32,
    /// `version`
    pub version: u32,
    /// seqlock の番号（書き込み中は奇数、0 は未公開）
    pub seq: u32,
    /// Mozc のセッションから見た IME の開閉（入力スコープによる上書き後）
    pub effective_open: u32,
    /// Mozc のセッションから見た入力モード（`CompositionMode`）
    pub effective_mode: u32,
    /// TSF から見える IME の開閉（compartment 相当）
    pub tsf_open: u32,
    /// TSF から見える入力モード（`CompositionMode`）
    pub tsf_mode: u32,
    /// 書き出したスレッドの ID
    pub thread_id: u32,
    /// 書き出したプロセスの ID
    pub process_id: u32,
    /// TIP がそのスレッドで有効なら 1（version 2 以降）
    pub active: u32,
    /// 最後に変化を書いた時の `GetTickCount64()`（生存の判定には使わない）
    pub update_tick_ms: u64,
}

fn u32_at(bytes: &[u8; BLOCK_SIZE], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

impl RawBlock {
    /// 48 バイトを解釈する（検証はしない。検証は [`classify`]）。
    #[must_use]
    pub fn parse(bytes: &[u8; BLOCK_SIZE]) -> Self {
        let mut tick = [0u8; 8];
        tick.copy_from_slice(&bytes[40..48]);
        Self {
            magic: u32_at(bytes, 0),
            version: u32_at(bytes, 4),
            seq: u32_at(bytes, 8),
            effective_open: u32_at(bytes, 12),
            effective_mode: u32_at(bytes, 16),
            tsf_open: u32_at(bytes, 20),
            tsf_mode: u32_at(bytes, 24),
            thread_id: u32_at(bytes, 28),
            process_id: u32_at(bytes, 32),
            active: u32_at(bytes, 36),
            update_tick_ms: u64::from_le_bytes(tick),
        }
    }
}

/// Mozc の `commands::CompositionMode`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionMode {
    /// 直接入力（IME が閉じているときの値）
    Direct,
    /// ひらがな
    Hiragana,
    /// 全角カタカナ
    FullKatakana,
    /// 半角英数
    HalfAscii,
    /// 全角英数
    FullAscii,
    /// 半角カタカナ
    HalfKatakana,
}

impl CompositionMode {
    /// 値（0〜5）から。範囲外なら `None`（将来の値や壊れた値を黙って別の値にしない）。
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Direct),
            1 => Some(Self::Hiragana),
            2 => Some(Self::FullKatakana),
            3 => Some(Self::HalfAscii),
            4 => Some(Self::FullAscii),
            5 => Some(Self::HalfKatakana),
            _ => None,
        }
    }

    /// journal 用のラベル。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Hiragana => "hiragana",
            Self::FullKatakana => "full-katakana",
            Self::HalfAscii => "half-ascii",
            Self::FullAscii => "full-ascii",
            Self::HalfKatakana => "half-katakana",
        }
    }
}

/// 受け入れたブロックの内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShmState {
    /// `version`
    pub version: u32,
    /// TSF から見える IME の開閉（段階 2 で belief に入れるなら既定で使う側。ADR-265 D3）
    pub tsf_open: bool,
    /// TSF から見える入力モード。範囲外の値なら `None`
    pub tsf_mode: Option<CompositionMode>,
    /// Mozc のセッションから見た IME の開閉
    pub effective_open: bool,
    /// Mozc のセッションから見た入力モード。範囲外の値なら `None`
    pub effective_mode: Option<CompositionMode>,
}

/// 捨てた理由（ADR-265 D3 の受け入れ条件のどれを満たさなかったか）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// `magic` が違う（別物のマッピング、または壊れている）
    BadMagic,
    /// `version` が `MIN_VERSION` 未満
    VersionTooOld,
    /// `seq` が奇数（書き込み中を写し取った）
    Torn,
    /// `seq == 0`（作った直後でまだ書かれていない）
    Unpublished,
    /// `process_id` が対象の窓のプロセスと違う（スレッド ID の再利用）
    PidMismatch,
    /// `active != 1`（その窓では mozc-readable が使われていない）
    Inactive,
}

impl Rejection {
    /// journal 用のラベル。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BadMagic => "bad-magic",
            Self::VersionTooOld => "version-too-old",
            Self::Torn => "torn",
            Self::Unpublished => "unpublished",
            Self::PidMismatch => "pid-mismatch",
            Self::Inactive => "inactive",
        }
    }
}

/// 読み取りの経路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// 通常のプロセス（`Local\mozc-readable-ime-<tid>`）
    Local,
    /// AppContainer のプロセス（`\Sessions\<n>\AppContainerNamedObjects\<SID>\mozc-readable-ime-<tid>`）
    AppContainer,
}

impl Route {
    /// journal 用のラベル。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::AppContainer => "app-container",
        }
    }
}

/// ブロックを読めなかった理由（OS の呼び出しの段階）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFailure {
    /// 開けなかった（そのスレッドでは mozc-readable が動いていない、または名前空間が違う）
    NotFound,
    /// AppContainer の経路を始められなかった（トークンを読めない・SID が取れない。段階を示す）
    Unresolved(&'static str),
    /// `NtOpenSection` が拒否した（NTSTATUS）
    Denied(u32),
    /// seqlock の再試行を使い切った
    Torn,
}

/// 1 回の読み取りの結果（OS 側の入力）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// 読んだ経路
    pub route: Route,
    /// 読んだ 48 バイト、または失敗の理由
    pub block: Result<RawBlock, ReadFailure>,
}

/// 読み取りの結論。変化の検出（journal に残すか）にも使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 受け入れた
    Accepted(ShmState),
    /// 捨てた（理由つき）
    Rejected(Rejection),
    /// 読めなかった
    Failed(ReadFailure),
}

/// 受け入れ条件（ADR-265 D3、`shared-memory.md` の 4-2）を順に確かめる。
///
/// 1. `magic` が一致する。2. `version >= 2`。3. `seq` が偶数で 0 でない。4. `process_id` が対象の窓のプロセスと一致する。
/// 5. `active == 1`。**`update_tick_ms` の新しさは見ない**（変化が無ければ書かないので、生存の印にならない）。
///
/// # Errors
/// 条件を満たさなかった最初の理由を返す。
pub fn classify(raw: &RawBlock, expected_pid: u32) -> Result<ShmState, Rejection> {
    if raw.magic != MAGIC {
        return Err(Rejection::BadMagic);
    }
    if raw.version < MIN_VERSION {
        return Err(Rejection::VersionTooOld);
    }
    if raw.seq % 2 == 1 {
        return Err(Rejection::Torn);
    }
    if raw.seq == 0 {
        return Err(Rejection::Unpublished);
    }
    if raw.process_id != expected_pid {
        return Err(Rejection::PidMismatch);
    }
    if raw.active != 1 {
        return Err(Rejection::Inactive);
    }
    Ok(ShmState {
        version: raw.version,
        tsf_open: raw.tsf_open != 0,
        tsf_mode: CompositionMode::from_u32(raw.tsf_mode),
        effective_open: raw.effective_open != 0,
        effective_mode: CompositionMode::from_u32(raw.effective_mode),
    })
}

/// ADR-268 D3 の規則 **R3'**: この読みは、awase の最後の操作（`baseline_ms`）より**後の**事実か。
///
/// `update_tick_ms > baseline_ms`（操作より後に変化したことが確実）、または、操作から `settle_ms` 以上経っている
/// （操作を処理し終えた後の安定した状態。変化したなら新しい値、変化しなかった・書き込みが失われたなら古い値で、
/// どちらも事実）。`>=` にしない理由: `GetTickCount64` の分解能は約 15.6ms で、同じ目盛りの中の「直前の別の変化」と
/// 「操作」を区別できず、IME が処理する前の古い読みを通してしまう。
#[must_use]
pub const fn read_is_after_baseline(
    update_tick_ms: u64,
    baseline_ms: u64,
    now_ms: u64,
    settle_ms: u64,
) -> bool {
    update_tick_ms > baseline_ms || now_ms.saturating_sub(baseline_ms) >= settle_ms
}

/// 受け入れた読みを belief にどう使うか（ADR-268 D1・D3。判断だけで副作用は無い）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmUse {
    /// `tsf_open != effective_open`（入力スコープの上書き等）。割れている読みは belief に報告しない
    /// （どちらを IME の状態とするかが未測定）。
    Split,
    /// R3' を満たさない（操作の直後で、IME の処理前かもしれない）。belief に報告せず、追随もしない。
    Held,
    /// belief に報告する。`disagrees` が真なら、belief（`effective_open`）または `desired_open` と食い違うので、
    /// 外から変わったものとして追随する（`intent_store.remove` → `pass_through_observed(true, true)`）。
    Use { open: bool, disagrees: bool },
}

/// 受け入れた読み `shm` を、belief にどう使うかを決める。
///
/// `update_tick_ms` は読んだブロックの `update_tick_ms`（最後に**変化**を書いた時刻）。`ShmState` には持たせない:
/// 持たせると、時刻だけが違う読みが「別の結論」になり、診断の記録の重複除去（`Outcome` の等価比較）が崩れる。
/// `baseline_ms` は `max(hook_alive_tick_ms, 最後の ImmCross の書き込みの発行時刻)`（awase の最後の操作の時刻、
/// `GetTickCount64` の値）。`settle_ms` は `KEY_EFFECT_SETTLE_MS`。
#[must_use]
pub const fn decide_use(
    shm: &ShmState,
    update_tick_ms: u64,
    now_ms: u64,
    baseline_ms: u64,
    settle_ms: u64,
    effective_open: bool,
    desired_open: bool,
) -> ShmUse {
    if shm.tsf_open != shm.effective_open {
        return ShmUse::Split;
    }
    if !read_is_after_baseline(update_tick_ms, baseline_ms, now_ms, settle_ms) {
        return ShmUse::Held;
    }
    let open = shm.tsf_open;
    ShmUse::Use {
        open,
        disagrees: open != effective_open || open != desired_open,
    }
}

/// 読み取りの結果を結論にする。
#[must_use]
pub fn outcome_of(reading: &Reading, expected_pid: u32) -> Outcome {
    match &reading.block {
        Ok(raw) => match classify(raw, expected_pid) {
            Ok(state) => Outcome::Accepted(state),
            Err(rejection) => Outcome::Rejected(rejection),
        },
        Err(failure) => Outcome::Failed(*failure),
    }
}

/// 何をきっかけに読んだか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// フォーカスが変わった tick
    Focus,
    /// 前回の記録から結論が変わった tick
    Change,
}

impl Trigger {
    /// journal 用のラベル。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Change => "change",
        }
    }
}

/// 記録に添える、読んだ時点の文脈。
///
/// `belief_open`・`belief_mode` は読んだ時点の belief（段階 1 は照合の材料としてだけ添える。belief へは入れない）。
/// `thread_id`・`process_id` は読んだ対象（`pid` はブロックの `process_id` の照合に使った値）。
#[derive(Debug, Clone)]
pub struct RecordContext {
    /// きっかけ
    pub trigger: Trigger,
    /// 読んだ経路
    pub route: Route,
    /// 読み取りの所要（マイクロ秒）
    pub read_us: u32,
    /// 読んだ時点の窓の IME プロファイル
    pub profile: &'static str,
    /// 読んだスレッドの ID
    pub thread_id: u32,
    /// 読んだプロセスの ID
    pub process_id: u32,
    /// 読んだ時点の belief（IME が開いているか）
    pub belief_open: Option<bool>,
    /// 読んだ時点の belief（入力モード、`Debug` 表記）
    pub belief_mode: Option<String>,
}

/// 結論を journal の記録に変換する（純粋）。
#[must_use]
pub fn to_record(outcome: &Outcome, ctx: RecordContext) -> MozcShmRecord {
    let (verdict, reason, nt_status, state) = match outcome {
        Outcome::Accepted(s) => ("accepted", None, None, Some(s)),
        Outcome::Rejected(r) => ("rejected", Some(r.label()), None, None),
        Outcome::Failed(ReadFailure::NotFound) => ("failed", Some("not-found"), None, None),
        Outcome::Failed(ReadFailure::Unresolved(stage)) => ("failed", Some(*stage), None, None),
        Outcome::Failed(ReadFailure::Denied(status)) => {
            ("failed", Some("denied"), Some(*status), None)
        }
        Outcome::Failed(ReadFailure::Torn) => ("failed", Some("retries-exhausted"), None, None),
    };
    MozcShmRecord {
        trigger: ctx.trigger.label(),
        route: ctx.route.label(),
        verdict,
        reason,
        nt_status,
        version: state.map(|s| s.version),
        tsf_open: state.map(|s| s.tsf_open),
        tsf_mode: state.and_then(|s| s.tsf_mode).map(CompositionMode::label),
        effective_open: state.map(|s| s.effective_open),
        effective_mode: state
            .and_then(|s| s.effective_mode)
            .map(CompositionMode::label),
        read_us: ctx.read_us,
        profile: ctx.profile,
        belief_open: ctx.belief_open,
        belief_mode: ctx.belief_mode,
        thread_id: ctx.thread_id,
        process_id: ctx.process_id,
    }
}

/// SID（`SID` 構造体のバイト列）を `S-R-I-S-S-…` の文字列にする。
///
/// 並び: リビジョン（1 バイト）、下位権限の数（1 バイト）、識別子権限（6 バイト、ビッグエンディアン）、
/// 下位権限（4 バイトずつ、リトルエンディアン）。AppContainer の SID（`S-1-15-2-…`）の名前空間のパスを作るために使う。
/// 形式が合わなければ `None`。
#[must_use]
pub fn format_sid(bytes: &[u8]) -> Option<String> {
    let revision = *bytes.first()?;
    let count = usize::from(*bytes.get(1)?);
    if count > 15 || bytes.len() < 8 + 4 * count {
        return None;
    }
    let mut authority = 0u64;
    for b in &bytes[2..8] {
        authority = (authority << 8) | u64::from(*b);
    }
    let mut out = format!("S-{revision}-{authority}");
    for i in 0..count {
        let at = 8 + 4 * i;
        let sub = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        out.push('-');
        out.push_str(&sub.to_string());
    }
    Some(out)
}

/// AppContainer の名前空間の、共有メモリのパス（`NtOpenSection` に渡す完全なパス）。
#[must_use]
pub fn app_container_section_path(session_id: u32, sid: &str, thread_id: u32) -> String {
    format!(
        "\\Sessions\\{session_id}\\AppContainerNamedObjects\\{sid}\\mozc-readable-ime-{thread_id}"
    )
}

/// 通常のプロセスの共有メモリの名前（`OpenFileMappingW` に渡す）。
#[must_use]
pub fn local_mapping_name(thread_id: u32) -> String {
    format!("Local\\mozc-readable-ime-{thread_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(fields: [u32; 10], tick: u64) -> [u8; BLOCK_SIZE] {
        let mut b = [0u8; BLOCK_SIZE];
        for (i, f) in fields.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        b[40..48].copy_from_slice(&tick.to_le_bytes());
        b
    }

    /// magic, version, seq, eff_open, eff_mode, tsf_open, tsf_mode, tid, pid, active
    fn good() -> RawBlock {
        RawBlock::parse(&bytes([MAGIC, 2, 8, 1, 1, 1, 2, 1908, 8284, 1], 1234))
    }

    #[test]
    fn parse_reads_every_field_little_endian() {
        let raw = RawBlock::parse(&bytes(
            [MAGIC, 2, 8, 1, 3, 0, 5, 1908, 8284, 1],
            0x0102_0304_0506,
        ));
        assert_eq!(raw.magic, 0x5A52_444D);
        assert_eq!(raw.version, 2);
        assert_eq!(raw.seq, 8);
        assert_eq!(raw.effective_open, 1);
        assert_eq!(raw.effective_mode, 3);
        assert_eq!(raw.tsf_open, 0);
        assert_eq!(raw.tsf_mode, 5);
        assert_eq!(raw.thread_id, 1908);
        assert_eq!(raw.process_id, 8284);
        assert_eq!(raw.active, 1);
        assert_eq!(raw.update_tick_ms, 0x0102_0304_0506);
    }

    #[test]
    fn magic_bytes_in_memory_are_mdrz() {
        let b = bytes([MAGIC, 2, 8, 0, 0, 0, 0, 0, 0, 1], 0);
        assert_eq!(&b[0..4], b"MDRZ");
    }

    #[test]
    fn a_published_active_block_of_the_same_process_is_accepted() {
        let s = classify(&good(), 8284).unwrap();
        assert_eq!(s.version, 2);
        assert!(s.tsf_open);
        assert_eq!(s.tsf_mode, Some(CompositionMode::FullKatakana));
        assert!(s.effective_open);
        assert_eq!(s.effective_mode, Some(CompositionMode::Hiragana));
    }

    #[test]
    fn each_acceptance_condition_rejects_with_its_own_reason() {
        let mut r = good();
        r.magic = 0;
        assert_eq!(classify(&r, 8284), Err(Rejection::BadMagic));

        let mut r = good();
        r.version = 1;
        assert_eq!(classify(&r, 8284), Err(Rejection::VersionTooOld));

        let mut r = good();
        r.seq = 9;
        assert_eq!(classify(&r, 8284), Err(Rejection::Torn));

        let mut r = good();
        r.seq = 0;
        assert_eq!(classify(&r, 8284), Err(Rejection::Unpublished));

        assert_eq!(classify(&good(), 1), Err(Rejection::PidMismatch));

        let mut r = good();
        r.active = 0;
        assert_eq!(classify(&r, 8284), Err(Rejection::Inactive));
    }

    #[test]
    fn conditions_are_checked_in_the_documented_order() {
        // magic が違えば、他が全部ダメでも bad-magic（先に見る）
        let mut r = good();
        r.magic = 0;
        r.version = 0;
        r.active = 0;
        assert_eq!(classify(&r, 1), Err(Rejection::BadMagic));
        // version 1 の古いブロックは、active が無い（0）ので version で先に捨てる
        let mut r = good();
        r.version = 1;
        r.active = 0;
        assert_eq!(classify(&r, 8284), Err(Rejection::VersionTooOld));
        // 未公開（seq == 0）は pid・active より先
        let mut r = good();
        r.seq = 0;
        r.active = 0;
        assert_eq!(classify(&r, 1), Err(Rejection::Unpublished));
    }

    #[test]
    fn stale_update_tick_does_not_reject() {
        let mut r = good();
        r.update_tick_ms = 0;
        assert!(classify(&r, 8284).is_ok());
    }

    #[test]
    fn out_of_range_mode_value_is_kept_as_unknown_not_guessed() {
        let mut r = good();
        r.tsf_mode = 9;
        r.effective_mode = 6;
        let s = classify(&r, 8284).unwrap();
        assert_eq!(s.tsf_mode, None);
        assert_eq!(s.effective_mode, None);
    }

    #[test]
    fn composition_mode_values_and_labels() {
        let all = [
            (0, CompositionMode::Direct, "direct"),
            (1, CompositionMode::Hiragana, "hiragana"),
            (2, CompositionMode::FullKatakana, "full-katakana"),
            (3, CompositionMode::HalfAscii, "half-ascii"),
            (4, CompositionMode::FullAscii, "full-ascii"),
            (5, CompositionMode::HalfKatakana, "half-katakana"),
        ];
        for (v, m, label) in all {
            assert_eq!(CompositionMode::from_u32(v), Some(m));
            assert_eq!(m.label(), label);
        }
        assert_eq!(CompositionMode::from_u32(6), None);
    }

    #[test]
    fn outcome_of_maps_read_failures_and_rejections() {
        let ok = Reading {
            route: Route::Local,
            block: Ok(good()),
        };
        assert!(matches!(outcome_of(&ok, 8284), Outcome::Accepted(_)));
        assert_eq!(
            outcome_of(&ok, 1),
            Outcome::Rejected(Rejection::PidMismatch)
        );
        let nf = Reading {
            route: Route::Local,
            block: Err(ReadFailure::NotFound),
        };
        assert_eq!(
            outcome_of(&nf, 8284),
            Outcome::Failed(ReadFailure::NotFound)
        );
    }

    #[test]
    fn identical_conclusions_compare_equal_for_change_detection() {
        let a = outcome_of(
            &Reading {
                route: Route::Local,
                block: Ok(good()),
            },
            8284,
        );
        let mut raw = good();
        raw.update_tick_ms += 1; // 時刻だけの違いは結論を変えない
        raw.seq += 2;
        let b = outcome_of(
            &Reading {
                route: Route::Local,
                block: Ok(raw),
            },
            8284,
        );
        assert_eq!(a, b);
        let mut raw = good();
        raw.tsf_open = 0;
        let c = outcome_of(
            &Reading {
                route: Route::Local,
                block: Ok(raw),
            },
            8284,
        );
        assert_ne!(a, c);
    }

    fn ctx(trigger: Trigger, route: Route) -> RecordContext {
        RecordContext {
            trigger,
            route,
            read_us: 1,
            profile: "standard",
            thread_id: 1908,
            process_id: 8284,
            belief_open: None,
            belief_mode: None,
        }
    }

    #[test]
    fn record_for_an_accepted_block_carries_state_belief_and_target() {
        let o = Outcome::Accepted(classify(&good(), 8284).unwrap());
        let rec = to_record(
            &o,
            RecordContext {
                read_us: 42,
                profile: "tsf-native",
                belief_open: Some(false),
                belief_mode: Some("Hiragana".to_string()),
                ..ctx(Trigger::Focus, Route::AppContainer)
            },
        );
        assert_eq!(rec.thread_id, 1908);
        assert_eq!(rec.process_id, 8284);
        assert_eq!(rec.trigger, "focus");
        assert_eq!(rec.route, "app-container");
        assert_eq!(rec.verdict, "accepted");
        assert_eq!(rec.reason, None);
        assert_eq!(rec.tsf_open, Some(true));
        assert_eq!(rec.tsf_mode, Some("full-katakana"));
        assert_eq!(rec.effective_mode, Some("hiragana"));
        assert_eq!(rec.version, Some(2));
        assert_eq!(rec.read_us, 42);
        assert_eq!(rec.belief_open, Some(false));
        assert_eq!(rec.belief_mode.as_deref(), Some("Hiragana"));
    }

    #[test]
    fn record_for_a_rejection_or_failure_has_a_reason_and_no_state() {
        let rec = to_record(
            &Outcome::Rejected(Rejection::Inactive),
            ctx(Trigger::Change, Route::Local),
        );
        assert_eq!(rec.verdict, "rejected");
        assert_eq!(rec.reason, Some("inactive"));
        assert_eq!(rec.tsf_open, None);
        let rec = to_record(
            &Outcome::Failed(ReadFailure::Denied(0xC000_0022)),
            ctx(Trigger::Change, Route::AppContainer),
        );
        assert_eq!(rec.verdict, "failed");
        assert_eq!(rec.reason, Some("denied"));
        assert_eq!(rec.nt_status, Some(0xC000_0022));
        let rec = to_record(
            &Outcome::Failed(ReadFailure::Unresolved("sid")),
            ctx(Trigger::Change, Route::AppContainer),
        );
        assert_eq!(rec.reason, Some("sid"));
    }

    #[test]
    fn format_sid_builds_the_app_container_form() {
        // S-1-15-2-1642035244-3194355056: 実測した AppContainer の SID の先頭（run 39097062197）と同じ並び
        let mut b = vec![1u8, 3, 0, 0, 0, 0, 0, 15];
        for sub in [2u32, 1_642_035_244, 3_194_355_056] {
            b.extend_from_slice(&sub.to_le_bytes());
        }
        assert_eq!(
            format_sid(&b).as_deref(),
            Some("S-1-15-2-1642035244-3194355056")
        );
    }

    #[test]
    fn format_sid_rejects_malformed_input() {
        assert_eq!(format_sid(&[]), None);
        assert_eq!(format_sid(&[1]), None);
        assert_eq!(format_sid(&[1, 2, 0, 0, 0, 0, 0, 5, 1, 0, 0, 0]), None); // 下位権限が足りない
        assert_eq!(format_sid(&[1, 16, 0, 0, 0, 0, 0, 5]), None); // 15 を超える
    }

    #[test]
    fn section_path_and_mapping_name_match_the_spec() {
        assert_eq!(local_mapping_name(1908), "Local\\mozc-readable-ime-1908");
        assert_eq!(
            app_container_section_path(2, "S-1-15-2-1", 1908),
            "\\Sessions\\2\\AppContainerNamedObjects\\S-1-15-2-1\\mozc-readable-ime-1908"
        );
    }

    // ── ADR-268 D3: R3' と decide_use ──

    fn shm(tsf_open: bool, effective_open: bool) -> ShmState {
        ShmState {
            version: 2,
            tsf_open,
            tsf_mode: Some(CompositionMode::Hiragana),
            effective_open,
            effective_mode: Some(CompositionMode::Hiragana),
        }
    }

    /// R3' の 2 項: strict の `>`、または操作から settle 以上。
    #[test]
    fn read_is_after_baseline_table() {
        // (update_tick, baseline, now, settle) -> 期待
        let table: [(u64, u64, u64, u64, bool); 7] = [
            (1001, 1000, 1010, 170, true),  // 操作より後に変化(strict)
            (1000, 1000, 1010, 170, false), // 同じ目盛り + settle 未満 → 処理前かもしれない
            (990, 1000, 1010, 170, false),  // 操作より前 + settle 未満 → まだ処理前
            (990, 1000, 1170, 170, true),   // 操作より前でも settle 経過 → 書き込みが失われた(事実)
            (1000, 1000, 1170, 170, true),  // 同じ目盛りでも settle 経過 → 安定した状態
            (990, 1000, 1169, 170, false),  // settle の 1ms 手前
            (5, 0, 0, 170, true),           // 基準が 0(操作の記録なし)なら update_tick > 0 で通る
        ];
        for (u, b, n, st, want) in table {
            assert_eq!(
                read_is_after_baseline(u, b, n, st),
                want,
                "update={u} baseline={b} now={n} settle={st}"
            );
        }
    }

    /// 基準より now が小さい(時計の巻き戻しや初期値)でも落ちない。
    #[test]
    fn read_is_after_baseline_never_underflows() {
        assert!(!read_is_after_baseline(0, 100, 50, 170));
    }

    #[test]
    fn decide_use_split_when_tsf_and_effective_differ() {
        let st = shm(true, false);
        let tick = 5000;
        assert_eq!(
            decide_use(&st, tick, 6000, 1000, 170, true, true),
            ShmUse::Split
        );
        let st = shm(false, true);
        let tick = 5000;
        assert_eq!(
            decide_use(&st, tick, 6000, 1000, 170, true, true),
            ShmUse::Split
        );
    }

    #[test]
    fn decide_use_holds_a_read_right_after_an_operation() {
        // 操作(基準 1000)の直後(now 1010)で、update_tick が基準以下 → 報告しない・追随しない
        let st = shm(false, false);
        let tick = 1000;
        assert_eq!(
            decide_use(&st, tick, 1010, 1000, 170, true, true),
            ShmUse::Held
        );
    }

    #[test]
    fn decide_use_reports_and_marks_disagreement_with_belief_or_desired() {
        // 一致: 報告するが追随しない
        let st = shm(true, true);
        let tick = 5000;
        assert_eq!(
            decide_use(&st, tick, 6000, 1000, 170, true, true),
            ShmUse::Use {
                open: true,
                disagrees: false
            }
        );
        // belief(effective_open)と食い違う → 追随
        assert_eq!(
            decide_use(&st, tick, 6000, 1000, 170, false, true),
            ShmUse::Use {
                open: true,
                disagrees: true
            }
        );
        // desired_open と食い違う(belief は共有メモリに従っているが desired が古い、N8)→ 追随
        assert_eq!(
            decide_use(&st, tick, 6000, 1000, 170, true, false),
            ShmUse::Use {
                open: true,
                disagrees: true
            }
        );
    }

    /// 書き込みが IME に届かなかった場面(N2): 共有メモリは変わらず update_tick は操作より前のまま。
    /// settle 経過後は「書き込みが効かなかった」という事実として追随する(belief が開のまま IME は閉で固定されない)。
    #[test]
    fn decide_use_follows_a_lost_write_after_settle() {
        let st = shm(false, false);
        let tick = 900; // 操作(1000)より前のまま
        assert_eq!(
            decide_use(&st, tick, 1100, 1000, 170, true, true),
            ShmUse::Held
        );
        assert_eq!(
            decide_use(&st, tick, 1170, 1000, 170, true, true),
            ShmUse::Use {
                open: false,
                disagrees: true
            }
        );
    }
}
