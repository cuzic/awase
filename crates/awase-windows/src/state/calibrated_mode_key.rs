//! [ADR-176](../../../../docs/adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
//! 決定6: モードキー較正結果1件を表す純粋データ構造。
//!
//! `176-T1`。プラットフォーム非依存（`windows`crateに依存しない）な純粋
//! データ構造として、Linux上でも定義・テストできる場所に置く
//! （`state/ime_kind.rs`と同じ ungated パターン）。この構造体自体は
//! `ImeModel`のbeliefではない——`.claude/rules/ime-belief-architecture.md`の
//! Observe→`classify_*`→`reduce()`規律が対象とするIME belief状態
//! （ON/OFF・input_mode）とは別の、静的config分類を補完する較正記録
//! である。

use crate::gji_charset_autodetect::ImeToggleKind;
use crate::state::ime_kind::ImeKindId;
use crate::state::TickMs;
use crate::vk::VkCodeExt as _;
use awase::config::ImeDetectConfig;
use awase::types::VkCode;

/// GJI/MS-IMEの`config1.db`/レジストリの、較正時点でのフィンガープリント。
///
/// `is_stale`がこれを現在値と比較し、食い違えば較正結果を無効化する
/// （BUG-143の既知の限界——GUI実装のクリア漏れによる`custom_keymap_table`
/// 残留——を検出する手段としても機能する、ADR-176決定6）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigFingerprint {
    Gji {
        /// `session_keymap`フィールドの値（`awase-gji-config`のraw値）。
        session_keymap: Option<i64>,
        /// 較正対象キーに関連する`custom_keymap_table`の該当行
        /// （無ければ`None`）。行の生テキストをそのまま保持し、
        /// 意味解釈はここでは行わない。
        relevant_row: Option<String>,
    },
    MsIme {
        /// 較正に関連するレジストリ値のハッシュ。
        registry_value_hash: u64,
    },
}

/// 較正結果1件。対象VK・GJI/MS-IMEどちらの環境で較正したか・較正時点の
/// config指紋・確定時刻を持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CalibratedModeKey {
    pub(crate) vk: VkCode,
    /// v8時点のスコープ（OFF→ON方向のみ）では`ImeToggleKind::On`のみが
    /// 実際に保存される想定だが、型としては3値のまま持つ
    /// （decision4「変化なし/Toggle判別不能は保存しない」の判定は
    /// この型の外、確定ロジック側で行う）。
    pub(crate) result: ImeToggleKind,
    pub(crate) active_ime_kind: ImeKindId,
    pub(crate) config_fingerprint: ConfigFingerprint,
    /// 較正が確定した時刻（`TickMs`ではなく永続化を跨ぐため、
    /// プロセス再起動をまたいでも意味を持つUnix epoch msで保持する）。
    pub(crate) confirmed_at_epoch_ms: u64,
}

/// `record`の`config_fingerprint`が`current`と食い違っていれば`true`
/// （stale、静的分類へフォールバックすべき）。
#[must_use]
pub(crate) fn is_stale(record: &CalibratedModeKey, current: &ConfigFingerprint) -> bool {
    record.config_fingerprint != *current
}

/// `176-T12`（ADR-176決定6）: `record`が`current`に対してstaleでなければ
/// そのまま返し、staleなら`None`にする（`apply_calibration_override`への
/// 入力を「較正結果なし」に落とし、静的分類へフォールバックさせる）。
/// 呼び出し元（`gji_charset_autodetect.rs`/`message_handlers.rs`）は
/// `Runtime::calibrated_mode_key_for`が返した値をそのままここへ通すこと
/// ——staleかどうかの判定はこの関数の外で行わない。
#[must_use]
pub(crate) fn fresh_or_none<'a>(
    record: Option<&'a CalibratedModeKey>,
    current: &ConfigFingerprint,
) -> Option<&'a CalibratedModeKey> {
    record.filter(|r| !is_stale(r, current))
}

/// `176-T11`（ADR-176決定6）: `config.toml`への永続化用の文字列橋渡し。
/// `awase`本体（`src/config.rs`）はプラットフォーム非依存のため、
/// `ImeToggleKind`/`ImeKindId`/`ConfigFingerprint`を直接使えない
/// ——`KeysConfig`の`ime_on: Vec<String>`等と同じ「文字列で橋渡しする」
/// パターンに揃え、意味解釈はこのモジュール側で行う。
impl CalibratedModeKey {
    #[must_use]
    #[allow(dead_code)] // 176-T12（起動時保存）で呼び出し
    pub(crate) fn to_config_entry(&self) -> awase::config::CalibrationEntry {
        let result = ime_toggle_kind_to_str(self.result);
        let active_ime_kind = ime_kind_id_to_str(self.active_ime_kind);
        let (fingerprint_kind, gji_session_keymap, gji_relevant_row, ms_ime_registry_value_hash) =
            match &self.config_fingerprint {
                ConfigFingerprint::Gji {
                    session_keymap,
                    relevant_row,
                } => ("Gji", *session_keymap, relevant_row.clone(), None),
                ConfigFingerprint::MsIme {
                    registry_value_hash,
                } => ("MsIme", None, None, Some(*registry_value_hash)),
            };
        awase::config::CalibrationEntry {
            vk: self.vk,
            result: result.to_string(),
            active_ime_kind: active_ime_kind.to_string(),
            fingerprint_kind: fingerprint_kind.to_string(),
            gji_session_keymap,
            gji_relevant_row,
            ms_ime_registry_value_hash,
            confirmed_at_epoch_ms: self.confirmed_at_epoch_ms,
        }
    }
}

/// `awase::config::CalibrationEntry`から`CalibratedModeKey`へ変換する
/// （176-T11）。未知の`result`/`active_ime_kind`/`fingerprint_kind`文字列、
/// または`fingerprint_kind`が要求するフィールドが欠けている場合は`None`
/// を返す（呼び出し元がログへ警告を残し、そのエントリを無視することを
/// 想定——手書き編集された`config.toml`が壊れていても起動を落とさない）。
#[must_use]
#[allow(dead_code)] // 176-T12（起動時ロード）で呼び出し
pub(crate) fn calibrated_mode_key_from_config_entry(
    entry: &awase::config::CalibrationEntry,
) -> Option<CalibratedModeKey> {
    let result = ime_toggle_kind_from_str(&entry.result)?;
    let active_ime_kind = ime_kind_id_from_str(&entry.active_ime_kind)?;
    let config_fingerprint = match entry.fingerprint_kind.as_str() {
        "Gji" => ConfigFingerprint::Gji {
            session_keymap: entry.gji_session_keymap,
            relevant_row: entry.gji_relevant_row.clone(),
        },
        "MsIme" => ConfigFingerprint::MsIme {
            registry_value_hash: entry.ms_ime_registry_value_hash?,
        },
        _ => return None,
    };
    Some(CalibratedModeKey {
        vk: entry.vk,
        result,
        active_ime_kind,
        config_fingerprint,
        confirmed_at_epoch_ms: entry.confirmed_at_epoch_ms,
    })
}

const fn ime_toggle_kind_to_str(kind: ImeToggleKind) -> &'static str {
    match kind {
        ImeToggleKind::On => "On",
        ImeToggleKind::Off => "Off",
        ImeToggleKind::Toggle => "Toggle",
    }
}

fn ime_toggle_kind_from_str(s: &str) -> Option<ImeToggleKind> {
    match s {
        "On" => Some(ImeToggleKind::On),
        "Off" => Some(ImeToggleKind::Off),
        "Toggle" => Some(ImeToggleKind::Toggle),
        _ => None,
    }
}

const fn ime_kind_id_to_str(kind: ImeKindId) -> &'static str {
    match kind {
        ImeKindId::Gji => "Gji",
        ImeKindId::MsIme => "MsIme",
    }
}

fn ime_kind_id_from_str(s: &str) -> Option<ImeKindId> {
    match s {
        "Gji" => Some(ImeKindId::Gji),
        "MsIme" => Some(ImeKindId::MsIme),
        _ => None,
    }
}

/// `176-T2`（ADR-176決定5）: `gate_thumb_key_ime_actions`が返す
/// `wiring.henkan`/`wiring.muhenkan`（`static_result`）を、確定済み較正結果
/// （`calibrated`）があればそれで差し替える純粋関数。
///
/// `calibrated`はstale判定済みの値を渡すこと（stale/未較正なら呼び出し側で
/// `None`にしてから渡す——このモジュールの`is_stale`を使う）。
/// この関数自体は「較正結果があれば最優先」という単純な優先順位だけを持ち、
/// staleかどうかの判断はここでは行わない。
#[must_use]
pub(crate) fn apply_calibration_override(
    static_result: Option<ImeToggleKind>,
    calibrated: Option<&CalibratedModeKey>,
) -> Option<ImeToggleKind> {
    calibrated.map(|c| c.result).or(static_result)
}

/// `176-T6`（ADR-176決定1、round6 B3対応）: 較正モードのバイパスが
/// awase-settings側の応答無しに残り続けないためのタイムアウト判定。
/// `now`が`deadline`以降なら`true`（バイパスを自動解除すべき）。
/// `runtime/focus_tracking.rs::check_calibration_bypass_timeout`から呼ぶ。
/// 純粋関数のためLinux上でユニットテスト可能。
#[must_use]
pub(crate) const fn calibration_bypass_timed_out(now: TickMs, deadline: TickMs) -> bool {
    now.0 >= deadline.0
}

/// `176-T5`（ADR-176決定7、B4対応）: 較正対象VKが明示configに既に
/// 登録されているかを判定し、登録されていれば較正UI（`176-T10`）が
/// 表示すべき警告理由を返す。`None`なら較正してよい。
///
/// BUG-140（`docs/known-bugs/BUG-140.md`）と同じ「優先順位ではなく
/// 構造的除外」の方針を取る: `apply_calibration_override`は較正結果を
/// 無条件に最優先採用する（`176-T2`）ため、既に明示config済みのVKを
/// 較正すると、ユーザーが意図して書いた設定を較正UIが無断で上書きする
/// ことになる。値の優劣や後勝ちで解決するのではなく、較正の実行自体を
/// 拒否して構造的に衝突を起こさせない。
///
/// `awase-settings`から呼び出せるよう`pub`（`awase_windows`クレートの
/// 公開関数）。Windows APIには依存しない純粋関数のため、Linux上で
/// ユニットテストできる。
#[must_use]
pub fn explicit_config_conflict_reason(
    vk: VkCode,
    ime_detect: &ImeDetectConfig,
    ime_on: &[String],
    ime_off: &[String],
    ime_toggle: &[String],
) -> Option<&'static str> {
    let in_ime_detect = [&ime_detect.toggle, &ime_detect.on, &ime_detect.off]
        .into_iter()
        .flatten()
        .filter_map(|s| VkCode::from_name(s))
        .any(|registered| registered == vk);
    if in_ime_detect {
        return Some("keys.ime_detect（IME検出用シャドウ追跡キー）に既に登録されているキーです");
    }

    let bare_vk_registered = [ime_on, ime_off, ime_toggle]
        .into_iter()
        .flatten()
        .filter_map(|s| crate::vk::parse_key_combo(s))
        .any(|combo| combo.vk == vk && !combo.ctrl && !combo.shift && !combo.alt);
    if bare_vk_registered {
        return Some("keys.ime_on/ime_off/ime_toggle に修飾キー無しで既に登録されているキーです");
    }

    None
}

// ── 176-T9a: 押下起点の試行(trial)構築と2回一致確定 ─────────────────────

/// `176-T9a`（ADR-176決定4）: 較正probeの押下単位試行1件の分類。
/// `pre`=押下直前の最後の有効open値、`post`=押下からSETTLE_WINDOW_MS
/// 以内の最終有効open値。
///
/// opus-adversarial-consultレビュー（round9）指摘B2: 単純に「OFF→ON遷移を
/// 2回観測した」だけでは`On`と`Toggle`を区別できない——`Toggle`キーも
/// OFFから押せば必ずONになるため、`(false, true)`は両仮説と矛盾しない。
/// `Toggle`と確定的に区別できるのは以下の2パターンのみ:
/// - `(true, false)`: ONの状態で押すとOFFになった。これは`Toggle`の
///   決定的証拠であり、このキーを`On`として確定してはならない
///   （決定4「矛盾する観測パターンは保存しない」）。
/// - `(true, true)`: ONの状態で押してもONのまま。これは「押しても切れ
///   ない」＝単純トグルではない、という`On`の決定的証拠。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrialOutcome {
    /// `(false, false)`: このキーが今回の試行では何の効果も起こしていない
    /// （フォーカスがズレていた等）。証拠として扱わない。
    NoChange,
    /// `(false, true)`: OFF→ON。`On`/`Toggle`いずれの仮説とも矛盾しない
    /// （単独では確定できない、支持的証拠止まり）。
    TurnsOn,
    /// `(true, false)`: ON→OFF。`Toggle`の決定的証拠。
    TurnsOff,
    /// `(true, true)`: ONのまま変化なし。`On`の決定的証拠。
    StaysOn,
}

impl TrialOutcome {
    #[must_use]
    pub(crate) const fn classify(pre: bool, post: bool) -> Self {
        match (pre, post) {
            (false, false) => Self::NoChange,
            (false, true) => Self::TurnsOn,
            (true, false) => Self::TurnsOff,
            (true, true) => Self::StaysOn,
        }
    }
}

/// `176-T9a`確定状態機械の判定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalibrationVerdict {
    /// まだ確定にも却下にも至っていない（試行を継続する）。
    Undetermined,
    /// `Toggle`の決定的証拠（`(true, false)`試行）を観測したため、
    /// このキーは`On`として確定できない（決定4「矛盾する観測パターンは
    /// 保存しない」）。一度却下されたら以後も却下のまま
    /// （`saw_turns_off`はリセットされない、`CalibrationConfirmState`の
    /// doc参照）。
    Rejected,
    /// `ImeToggleKind::On`として確定した。
    ConfirmedOn,
}

/// `176-T9a`（ADR-176決定4）: 較正probeの押下単位試行列から
/// `ImeToggleKind::On`確定/矛盾棄却/未確定を判定する状態機械。
///
/// 確定条件（opus-adversarial-consultレビューround9 B1/B2の結論——
/// 決定4の他条項「変化なし/矛盾する観測パターンは保存しない」と両立する
/// 唯一の読み方として採用）: `TurnsOff`（`Toggle`の決定的証拠）を一度も
/// 観測しないまま、`StaysOn`（`On`の決定的証拠）を少なくとも1回観測し、
/// かつ`TurnsOn`/`StaysOn`の合計試行数が2以上。「2回連続で一致」を
/// 「Toggleではないと分かる決定的証拠を含む、2件の支持的試行」と解釈する。
///
/// `NoChange`試行は証拠として一切カウントしない（このキーが今回は
/// 何の効果も起こさなかっただけであり、`On`/`Toggle`どちらの判定にも
/// 使えないため）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CalibrationConfirmState {
    saw_turns_off: bool,
    stays_on_count: u32,
    supporting_count: u32,
}

impl CalibrationConfirmState {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            saw_turns_off: false,
            stays_on_count: 0,
            supporting_count: 0,
        }
    }

    /// 試行1件`(pre, post)`を反映し、現時点の判定を返す。`Rejected`/
    /// `ConfirmedOn`が返った後にさらに`record_trial`を呼んでも、
    /// `Rejected`は変わらない（`saw_turns_off`はリセットされない）。
    /// `ConfirmedOn`確定後の呼び出し元での扱いは呼び出し側の責務
    /// （このメソッド自体は確定後も引き続き`ConfirmedOn`を返し続ける）。
    pub(crate) fn record_trial(&mut self, pre: bool, post: bool) -> CalibrationVerdict {
        match TrialOutcome::classify(pre, post) {
            TrialOutcome::NoChange => {}
            TrialOutcome::TurnsOff => self.saw_turns_off = true,
            TrialOutcome::TurnsOn => self.supporting_count += 1,
            TrialOutcome::StaysOn => {
                self.stays_on_count += 1;
                self.supporting_count += 1;
            }
        }
        if self.saw_turns_off {
            return CalibrationVerdict::Rejected;
        }
        if self.stays_on_count >= 1 && self.supporting_count >= 2 {
            return CalibrationVerdict::ConfirmedOn;
        }
        CalibrationVerdict::Undetermined
    }
}

/// `176-T9a`確定済み試行1件、または未確定の状態を表す。`TrialTracker::tick`
/// の戻り値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrialTick {
    /// このtickでは試行が確定しなかった（試行継続中、または非アクティブ）。
    Pending,
    /// 試行が1件確定した。呼び出し元は`CalibrationConfirmState::
    /// record_trial`へ`(pre, post)`をそのまま渡すこと。
    Completed { pre: bool, post: bool },
    /// 試行が破棄された（`post`が一度もprobeで取得できなかった、または
    /// 新しい押下でsettle windowが上書きされた）。この試行は
    /// `CalibrationConfirmState`へは渡さない。
    Discarded,
}

/// `176-T9a`: T8の`press_seq`（押下検知）とprobeサンプル列から、
/// 押下単位の試行`(pre, post)`ペアを1件ずつ切り出す純粋な状態機械。
/// Win32/非同期処理からは完全に独立しており、Linux上でユニットテスト
/// できる（実際のWin32 probe呼び出し・タイマー駆動は呼び出し元
/// （`runtime/focus_tracking.rs`の較正probeループ、`hook.rs`の
/// `calibration_press_seq()`）の責務）。
///
/// - `pre`が取れない（押下前に有効サンプルが一度も無い）押下は
///   試行を作らず無視する（opus-adversarial-consultレビューround9
///   B1「preが取れない試行は捨てる」）。
/// - settle window中に新しい押下（`press_seq`のさらなる増加）が来た
///   場合、進行中の試行を`Discarded`として破棄し、新しい押下で
///   settle windowを仕切り直す（重なった試行を両方とも中途半端に
///   確定させない）。
/// - `probe_result`が`None`（probe失敗）のtickは`last_good_open`/
///   進行中の試行の`post`候補、どちらの更新にも使わない
///   （決定3 round6 M4「`None`は変化なしと区別し再試行」対応）。
#[derive(Debug, Clone)]
pub(crate) struct TrialTracker {
    last_seen_press_seq: u32,
    last_good_open: Option<bool>,
    active: Option<ActiveTrial>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveTrial {
    pre: bool,
    deadline_ms: u64,
    last_observed_post: Option<bool>,
}

impl TrialTracker {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            last_seen_press_seq: 0,
            last_good_open: None,
            active: None,
        }
    }

    /// 毎tick呼ぶ。`press_seq`/`now_ms`はライブな値
    /// （`hook::calibration_press_seq()`/`current_tick_ms()`）、
    /// `probe_result`はこのtickで取得できたprobe結果
    /// （`None`=probe失敗、無視）。`settle_window_ms`は
    /// `tuning::CALIBRATION_TRIAL_SETTLE_WINDOW_MS`を渡すこと。
    pub(crate) fn tick(
        &mut self,
        press_seq: u32,
        now_ms: u64,
        probe_result: Option<bool>,
        settle_window_ms: u64,
    ) -> TrialTick {
        let mut discarded = false;

        // 新しい押下を検知（press_seqの増加）。
        if press_seq != self.last_seen_press_seq {
            self.last_seen_press_seq = press_seq;
            if self.active.is_some() {
                // settle window中の重複押下——進行中の試行を破棄し仕切り直す
                // （opus-adversarial-consultレビューround9 B1）。
                self.active = None;
                discarded = true;
            }
            if let Some(pre) = self.last_good_open {
                self.active = Some(ActiveTrial {
                    pre,
                    deadline_ms: now_ms + settle_window_ms,
                    last_observed_post: None,
                });
            }
            // pre が取れない（まだ一度も有効サンプルが無い）場合、この押下は
            // 試行を作らず無視する（round9 B1）。
        }

        if let Some(open) = probe_result {
            if let Some(active) = &mut self.active {
                active.last_observed_post = Some(open);
            } else {
                self.last_good_open = Some(open);
            }
        }

        if let Some(active) = self.active {
            if now_ms >= active.deadline_ms {
                self.active = None;
                // settle window中、probeが一度も成功しなかった場合は
                // 判定材料が無いため試行ごと破棄する。
                return active
                    .last_observed_post
                    .map_or(TrialTick::Discarded, |post| TrialTick::Completed {
                        pre: active.pre,
                        post,
                    });
            }
        }

        if discarded {
            return TrialTick::Discarded;
        }
        TrialTick::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(fingerprint: ConfigFingerprint) -> CalibratedModeKey {
        CalibratedModeKey {
            vk: VkCode::from(0x1C),
            result: ImeToggleKind::On,
            active_ime_kind: ImeKindId::Gji,
            config_fingerprint: fingerprint,
            confirmed_at_epoch_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn matching_fingerprint_is_not_stale() {
        let fp = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOn".to_string()),
        };
        let record = sample(fp.clone());
        assert!(!is_stale(&record, &fp));
    }

    #[test]
    fn differing_session_keymap_is_stale() {
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let current = ConfigFingerprint::Gji {
            session_keymap: Some(2),
            relevant_row: None,
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    #[test]
    fn differing_relevant_row_is_stale_even_with_same_session_keymap() {
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOn".to_string()),
        };
        let current = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOff".to_string()),
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    #[test]
    fn ms_ime_hash_mismatch_is_stale() {
        let recorded = ConfigFingerprint::MsIme {
            registry_value_hash: 111,
        };
        let current = ConfigFingerprint::MsIme {
            registry_value_hash: 222,
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    #[test]
    fn fresh_or_none_passes_through_matching_fingerprint() {
        let fp = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let record = sample(fp.clone());
        assert_eq!(fresh_or_none(Some(&record), &fp), Some(&record));
    }

    #[test]
    fn fresh_or_none_drops_stale_fingerprint() {
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let current = ConfigFingerprint::Gji {
            session_keymap: Some(2),
            relevant_row: None,
        };
        let record = sample(recorded);
        assert_eq!(fresh_or_none(Some(&record), &current), None);
    }

    #[test]
    fn fresh_or_none_passes_through_none() {
        let current = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        assert_eq!(fresh_or_none(None, &current), None);
    }

    // ── 176-T11: config.toml永続化のラウンドトリップ ────────────────────

    #[test]
    fn gji_entry_round_trips_through_config_entry() {
        let fp = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: Some("DirectInput\tHenkan\tIMEOn".to_string()),
        };
        let record = sample(fp);
        let entry = record.to_config_entry();
        assert_eq!(entry.vk, record.vk);
        assert_eq!(entry.result, "On");
        assert_eq!(entry.active_ime_kind, "Gji");
        assert_eq!(entry.fingerprint_kind, "Gji");
        assert_eq!(entry.gji_session_keymap, Some(1));
        assert_eq!(
            entry.gji_relevant_row.as_deref(),
            Some("DirectInput\tHenkan\tIMEOn")
        );
        assert_eq!(entry.ms_ime_registry_value_hash, None);
        assert_eq!(
            calibrated_mode_key_from_config_entry(&entry).as_ref(),
            Some(&record)
        );
    }

    #[test]
    fn ms_ime_entry_round_trips_through_config_entry() {
        let mut record = sample(ConfigFingerprint::MsIme {
            registry_value_hash: 0xDEAD_BEEF,
        });
        record.active_ime_kind = ImeKindId::MsIme;
        let entry = record.to_config_entry();
        assert_eq!(entry.active_ime_kind, "MsIme");
        assert_eq!(entry.fingerprint_kind, "MsIme");
        assert_eq!(entry.gji_session_keymap, None);
        assert_eq!(entry.gji_relevant_row, None);
        assert_eq!(entry.ms_ime_registry_value_hash, Some(0xDEAD_BEEF));
        assert_eq!(
            calibrated_mode_key_from_config_entry(&entry).as_ref(),
            Some(&record)
        );
    }

    #[test]
    fn entry_round_trips_through_actual_toml_serialization() {
        let record = sample(ConfigFingerprint::Gji {
            session_keymap: None,
            relevant_row: None,
        });
        let entry = record.to_config_entry();
        let mut config = awase::config::AppConfig::default();
        config.calibration.push(entry);
        let toml_str = toml::to_string(&config).expect("serialize AppConfig");
        let parsed: awase::config::AppConfig =
            toml::from_str(&toml_str).expect("deserialize AppConfig");
        assert_eq!(parsed.calibration.len(), 1);
        assert_eq!(
            calibrated_mode_key_from_config_entry(&parsed.calibration[0]),
            Some(record)
        );
    }

    #[test]
    fn unknown_result_string_is_rejected() {
        let mut entry = sample(ConfigFingerprint::Gji {
            session_keymap: None,
            relevant_row: None,
        })
        .to_config_entry();
        entry.result = "Unknown".to_string();
        assert_eq!(calibrated_mode_key_from_config_entry(&entry), None);
    }

    #[test]
    fn ms_ime_fingerprint_without_hash_is_rejected() {
        let mut entry = sample(ConfigFingerprint::MsIme {
            registry_value_hash: 1,
        })
        .to_config_entry();
        entry.ms_ime_registry_value_hash = None;
        assert_eq!(calibrated_mode_key_from_config_entry(&entry), None);
    }

    #[test]
    fn override_falls_back_to_static_when_no_calibration() {
        let result = apply_calibration_override(Some(ImeToggleKind::Off), None);
        assert_eq!(result, Some(ImeToggleKind::Off));
    }

    #[test]
    fn override_falls_back_to_static_when_calibration_absent_and_static_none() {
        let result = apply_calibration_override(None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn override_prefers_calibration_over_static() {
        let fp = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let calibrated = sample(fp);
        // static_resultが較正結果と異なっていても、較正結果が優先される。
        let result = apply_calibration_override(Some(ImeToggleKind::Off), Some(&calibrated));
        assert_eq!(result, Some(ImeToggleKind::On));
    }

    #[test]
    fn override_prefers_calibration_even_without_static_result() {
        let fp = ConfigFingerprint::MsIme {
            registry_value_hash: 1,
        };
        let calibrated = sample(fp);
        let result = apply_calibration_override(None, Some(&calibrated));
        assert_eq!(result, Some(ImeToggleKind::On));
    }

    #[test]
    fn mismatched_ime_kind_variant_is_stale() {
        // GJIで較正したレコードを、MS-IMEのfingerprintと突き合わせる
        // （較正時と反映時でactive_ime_kindが変わった異常系）。
        let recorded = ConfigFingerprint::Gji {
            session_keymap: Some(1),
            relevant_row: None,
        };
        let current = ConfigFingerprint::MsIme {
            registry_value_hash: 0,
        };
        let record = sample(recorded);
        assert!(is_stale(&record, &current));
    }

    // ── 176-T5: explicit_config_conflict_reason ─────────────────────────

    fn muhenkan() -> VkCode {
        VkCode::from_name("無変換").expect("VK_NONCONVERT should parse")
    }

    fn empty_ime_detect() -> ImeDetectConfig {
        ImeDetectConfig {
            toggle: vec![],
            on: vec![],
            off: vec![],
        }
    }

    #[test]
    fn no_conflict_when_vk_absent_from_all_lists() {
        let ime_detect = ImeDetectConfig {
            toggle: vec![],
            on: vec!["IMEオン".to_string()],
            off: vec!["IMEオフ".to_string()],
        };
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &[]);
        assert_eq!(result, None);
    }

    #[test]
    fn conflicts_when_vk_registered_in_ime_detect_on() {
        let ime_detect = ImeDetectConfig {
            toggle: vec![],
            on: vec!["IMEオン".to_string(), "無変換".to_string()],
            off: vec![],
        };
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &[]);
        assert!(result.is_some(), "keys.ime_detect.on との衝突を検出すべき");
    }

    #[test]
    fn conflicts_when_vk_registered_in_ime_detect_toggle() {
        let ime_detect = ImeDetectConfig {
            toggle: vec!["無変換".to_string()],
            on: vec![],
            off: vec![],
        };
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &[]);
        assert!(
            result.is_some(),
            "keys.ime_detect.toggle との衝突を検出すべき"
        );
    }

    #[test]
    fn conflicts_when_vk_registered_bare_in_ime_on() {
        let ime_detect = empty_ime_detect();
        let ime_on = vec!["無変換".to_string()];
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &ime_on, &[], &[]);
        assert!(
            result.is_some(),
            "keys.ime_on への修飾キー無し登録との衝突を検出すべき（src/config.rs:601-609の実害と同型）"
        );
    }

    #[test]
    fn conflicts_when_vk_registered_bare_in_ime_toggle() {
        let ime_detect = empty_ime_detect();
        let ime_toggle = vec!["無変換".to_string()];
        let result =
            explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &[], &ime_toggle);
        assert!(result.is_some());
    }

    #[test]
    fn no_conflict_when_vk_registered_with_modifier_in_ime_off() {
        // 修飾キー付き（例: Ctrl+無変換）は Phase 1 で無条件消費しないため、
        // BUG-140/実害報告と同種の衝突ではない。構造的除外の対象外。
        let ime_detect = empty_ime_detect();
        let ime_off = vec!["Ctrl+無変換".to_string()];
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &[], &ime_off, &[]);
        assert_eq!(
            result, None,
            "修飾キー付き登録は構造的除外の対象外（優先順位の問題であり本判定の対象外）"
        );
    }

    #[test]
    fn no_conflict_when_different_vk_registered() {
        let ime_detect = empty_ime_detect();
        let ime_on = vec!["変換".to_string()];
        let result = explicit_config_conflict_reason(muhenkan(), &ime_detect, &ime_on, &[], &[]);
        assert_eq!(result, None);
    }

    // ── 176-T6: calibration_bypass_timed_out ────────────────────────────

    #[test]
    fn calibration_bypass_not_timed_out_before_deadline() {
        assert!(!calibration_bypass_timed_out(TickMs(999), TickMs(1_000)));
    }

    #[test]
    fn calibration_bypass_timed_out_at_deadline() {
        assert!(calibration_bypass_timed_out(TickMs(1_000), TickMs(1_000)));
    }

    #[test]
    fn calibration_bypass_timed_out_after_deadline() {
        assert!(calibration_bypass_timed_out(TickMs(1_001), TickMs(1_000)));
    }

    // ── 176-T9a: TrialOutcome::classify ──────────────────────────────────

    #[test]
    fn trial_outcome_classifies_all_four_combinations() {
        assert_eq!(TrialOutcome::classify(false, false), TrialOutcome::NoChange);
        assert_eq!(TrialOutcome::classify(false, true), TrialOutcome::TurnsOn);
        assert_eq!(TrialOutcome::classify(true, false), TrialOutcome::TurnsOff);
        assert_eq!(TrialOutcome::classify(true, true), TrialOutcome::StaysOn);
    }

    // ── 176-T9a: CalibrationConfirmState ─────────────────────────────────

    #[test]
    fn two_turns_on_trials_alone_do_not_confirm() {
        // opus-adversarial-consultレビューround9 B2: (false, true) だけでは
        // Toggleと区別できないため、2回では確定しない。
        let mut state = CalibrationConfirmState::new();
        assert_eq!(
            state.record_trial(false, true),
            CalibrationVerdict::Undetermined
        );
        assert_eq!(
            state.record_trial(false, true),
            CalibrationVerdict::Undetermined
        );
    }

    #[test]
    fn turns_off_once_rejects_permanently() {
        let mut state = CalibrationConfirmState::new();
        assert_eq!(
            state.record_trial(true, false),
            CalibrationVerdict::Rejected
        );
        // 後から On の決定的証拠(StaysOn)が2回来ても却下のまま。
        assert_eq!(state.record_trial(true, true), CalibrationVerdict::Rejected);
        assert_eq!(state.record_trial(true, true), CalibrationVerdict::Rejected);
    }

    #[test]
    fn two_stays_on_trials_confirm_on() {
        let mut state = CalibrationConfirmState::new();
        assert_eq!(
            state.record_trial(true, true),
            CalibrationVerdict::Undetermined
        );
        assert_eq!(
            state.record_trial(true, true),
            CalibrationVerdict::ConfirmedOn
        );
    }

    #[test]
    fn one_stays_on_plus_one_turns_on_confirms_on() {
        let mut state = CalibrationConfirmState::new();
        assert_eq!(
            state.record_trial(false, true),
            CalibrationVerdict::Undetermined
        );
        assert_eq!(
            state.record_trial(true, true),
            CalibrationVerdict::ConfirmedOn
        );
    }

    #[test]
    fn no_change_trials_never_contribute_to_confirmation() {
        let mut state = CalibrationConfirmState::new();
        for _ in 0..10 {
            assert_eq!(
                state.record_trial(false, false),
                CalibrationVerdict::Undetermined
            );
        }
    }

    #[test]
    fn single_stays_on_is_not_enough_without_second_supporting_trial() {
        let mut state = CalibrationConfirmState::new();
        assert_eq!(
            state.record_trial(true, true),
            CalibrationVerdict::Undetermined
        );
        assert_eq!(
            state.record_trial(false, false),
            CalibrationVerdict::Undetermined
        );
    }

    // ── 176-T9a: TrialTracker ─────────────────────────────────────────────

    const SETTLE_MS: u64 = 3_000;

    #[test]
    fn no_press_never_produces_a_trial() {
        let mut tracker = TrialTracker::new();
        for t in (0..10_000).step_by(100) {
            assert_eq!(
                tracker.tick(0, t, Some(false), SETTLE_MS),
                TrialTick::Pending
            );
        }
    }

    #[test]
    fn press_without_prior_good_open_is_ignored() {
        // pre が取れない（まだ一度も有効サンプルが無い）押下は試行を作らない。
        let mut tracker = TrialTracker::new();
        assert_eq!(tracker.tick(1, 0, None, SETTLE_MS), TrialTick::Pending);
        // その後もサンプルが無いまま長時間経過しても Completed/Discarded は出ない
        // （そもそも active trial が無いため）。
        assert_eq!(
            tracker.tick(1, SETTLE_MS + 1, None, SETTLE_MS),
            TrialTick::Pending
        );
    }

    #[test]
    fn press_with_known_pre_completes_with_last_observed_post_at_deadline() {
        let mut tracker = TrialTracker::new();
        // baseline: open=false, at t=0.
        assert_eq!(
            tracker.tick(0, 0, Some(false), SETTLE_MS),
            TrialTick::Pending
        );
        // 押下検知 (press_seq 0->1) at t=100.
        assert_eq!(
            tracker.tick(1, 100, Some(false), SETTLE_MS),
            TrialTick::Pending
        );
        // settle window中に open が true に変化するサンプルが届く。
        assert_eq!(
            tracker.tick(1, 500, Some(true), SETTLE_MS),
            TrialTick::Pending
        );
        // deadline (100 + SETTLE_MS) 到達で確定。
        assert_eq!(
            tracker.tick(1, 100 + SETTLE_MS, Some(true), SETTLE_MS),
            TrialTick::Completed {
                pre: false,
                post: true
            }
        );
    }

    #[test]
    fn press_with_no_successful_probe_during_window_is_discarded() {
        let mut tracker = TrialTracker::new();
        assert_eq!(
            tracker.tick(0, 0, Some(false), SETTLE_MS),
            TrialTick::Pending
        );
        assert_eq!(tracker.tick(1, 100, None, SETTLE_MS), TrialTick::Pending);
        // settle window中ずっとprobe失敗。
        assert_eq!(
            tracker.tick(1, 100 + SETTLE_MS, None, SETTLE_MS),
            TrialTick::Discarded
        );
    }

    #[test]
    fn overlapping_press_discards_in_progress_trial_and_restarts_window() {
        let mut tracker = TrialTracker::new();
        assert_eq!(
            tracker.tick(0, 0, Some(false), SETTLE_MS),
            TrialTick::Pending
        );
        // 1回目の押下。
        assert_eq!(
            tracker.tick(1, 100, Some(false), SETTLE_MS),
            TrialTick::Pending
        );
        // settle window中に2回目の押下（重複）——1回目の試行は破棄される。
        assert_eq!(
            tracker.tick(2, 200, Some(true), SETTLE_MS),
            TrialTick::Discarded
        );
        // 2回目の試行のdeadlineは 200 + SETTLE_MS。
        assert_eq!(
            tracker.tick(2, 200 + SETTLE_MS, Some(true), SETTLE_MS),
            TrialTick::Completed {
                pre: false,
                post: true
            }
        );
    }

    #[test]
    fn probe_failures_do_not_update_baseline_outside_active_trial() {
        let mut tracker = TrialTracker::new();
        assert_eq!(
            tracker.tick(0, 0, Some(false), SETTLE_MS),
            TrialTick::Pending
        );
        // probe失敗はbaselineを更新しない。
        assert_eq!(tracker.tick(0, 50, None, SETTLE_MS), TrialTick::Pending);
        assert_eq!(
            tracker.tick(1, 100, Some(true), SETTLE_MS),
            TrialTick::Pending
        );
        // pre は直近の有効サンプル(false)のまま——Noneの介在に影響されない。
        assert_eq!(
            tracker.tick(1, 100 + SETTLE_MS, Some(true), SETTLE_MS),
            TrialTick::Completed {
                pre: false,
                post: true
            }
        );
    }
}
