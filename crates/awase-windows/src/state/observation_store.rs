//! 観測値ストア (Step 3)
//!
//! `ime_observations.rs` の `focus_probe` + `observer_poll` 等を、
//! **per-source の構造化ストア** に置換する。
//! 単一の `latest` スロットに圧縮するのではなく、reducer が
//! 「複数ソースの合意」「観測の鮮度」「ドリフト継続時間」を判断材料に
//! 使えるよう情報を保持する。
//!
//! ## 絶対ルール
//!
//! `observed.latest = Some(obs)` はする。
//! `desired_open = obs.open` は禁止。
//! Observer は health checker / drift detector の役割に徹する。

use std::time::{Duration, Instant};

use super::ime_event::{HwndId, ObservationConfidence, ObservationSource};

/// 単一の観測値レコード。reducer / actuator がこの完全な情報で判断する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeObservation {
    pub open: bool,
    pub source: ObservationSource,
    /// 観測タイムスタンプ (鮮度・経過時間計算用)
    pub at: Instant,
    /// どのウィンドウで観測したか (フォーカス変更後の stale 検出用)
    pub hwnd: HwndId,
    /// 観測の信頼度 (profile 別の judge に使う)
    pub confidence: ObservationConfidence,
    /// この観測値の有効期限 (フォーカス変更で expire させたい場合等)
    pub expires_at: Option<Instant>,
}

impl ImeObservation {
    /// 有効期限を過ぎていないか
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }

    /// `now` からの経過時間
    #[must_use]
    pub fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.at)
    }
}

/// ソース別の最新観測値。各ソースで独立に最新値を保持する。
#[derive(Debug, Default, Clone)]
pub struct PerSourceObservations {
    pub focus_probe: Option<ImeObservation>,
    pub observer_poll: Option<ImeObservation>,
    pub gji: Option<ImeObservation>,
    pub imm_get_open_status: Option<ImeObservation>,
    pub tsf: Option<ImeObservation>,
    pub hwnd_cache: Option<ImeObservation>,
}

impl PerSourceObservations {
    /// 指定ソースの最新値を返す
    #[must_use]
    pub const fn get(&self, source: ObservationSource) -> Option<&ImeObservation> {
        match source {
            ObservationSource::FocusProbe => self.focus_probe.as_ref(),
            ObservationSource::ObserverPoll => self.observer_poll.as_ref(),
            ObservationSource::Gji => self.gji.as_ref(),
            ObservationSource::ImmGetOpenStatus => self.imm_get_open_status.as_ref(),
            ObservationSource::Tsf => self.tsf.as_ref(),
            ObservationSource::HwndCache => self.hwnd_cache.as_ref(),
        }
    }

    /// 指定ソースの最新値をセットする
    pub const fn set(&mut self, source: ObservationSource, obs: ImeObservation) {
        match source {
            ObservationSource::FocusProbe => self.focus_probe = Some(obs),
            ObservationSource::ObserverPoll => self.observer_poll = Some(obs),
            ObservationSource::Gji => self.gji = Some(obs),
            ObservationSource::ImmGetOpenStatus => self.imm_get_open_status = Some(obs),
            ObservationSource::Tsf => self.tsf = Some(obs),
            ObservationSource::HwndCache => self.hwnd_cache = Some(obs),
        }
    }

    /// 全ソースの観測値を iter (Some のみ)
    pub fn iter(&self) -> impl Iterator<Item = &ImeObservation> {
        [
            self.focus_probe.as_ref(),
            self.observer_poll.as_ref(),
            self.gji.as_ref(),
            self.imm_get_open_status.as_ref(),
            self.tsf.as_ref(),
            self.hwnd_cache.as_ref(),
        ]
        .into_iter()
        .flatten()
    }

    /// 全ソースを clear する (フォーカス変更時用)
    pub const fn clear_all(&mut self) {
        self.focus_probe = None;
        self.observer_poll = None;
        self.gji = None;
        self.imm_get_open_status = None;
        self.tsf = None;
        self.hwnd_cache = None;
    }
}

/// desired と observed の乖離追跡 (DriftDetected event の根拠)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeDrift {
    pub started_at: Instant,
}

/// 観測値ストア (Step 3 の SSOT)。
///
/// reducer は以下のような問い合わせができる:
/// - `per_source.get(source)` — 特定ソースの最新値
/// - `most_recent_trusted()` — confidence + age で最も信頼できる観測
/// - `consensus(window)` — 直近 N 内の複数ソース合意
/// - `drift.is_some()` — desired と乖離しているか
/// - `is_source_flapping(source, window)` — 短期間で flapping しているか (今後実装)
#[derive(Debug, Default, Clone)]
pub struct ObservationStore {
    pub per_source: PerSourceObservations,
    /// desired との乖離追跡
    pub drift: Option<ImeDrift>,
}

impl ObservationStore {
    /// 観測値を per_source に記録する。
    #[allow(clippy::missing_const_for_fn)]
    pub fn record(&mut self, obs: ImeObservation) {
        self.per_source.set(obs.source, obs);
    }

    /// 全ソースを clear する (フォーカス変更時用)。drift も clear。
    pub fn clear_on_focus_change(&mut self) {
        self.per_source.clear_all();
        self.drift = None;
    }

    /// desired と observed の乖離を更新する。
    ///
    /// `observed` が `desired` と一致する場合は drift を clear。
    /// 不一致が継続するなら drift を保持し続ける (started_at は更新しない)。
    pub const fn update_drift(&mut self, desired: bool, observed: bool, now: Instant) {
        if desired == observed {
            self.drift = None;
            return;
        }
        if self.drift.is_none() {
            self.drift = Some(ImeDrift { started_at: now });
        }
    }

    /// 乖離継続時間を返す
    #[must_use]
    pub fn drift_duration(&self, now: Instant) -> Option<Duration> {
        self.drift
            .map(|d| now.saturating_duration_since(d.started_at))
    }

    /// 最も信頼できる観測値を返す (confidence 優先、同 confidence なら新しい方)。
    ///
    /// expire 済みの観測は除外する。
    #[must_use]
    pub fn most_recent_trusted(&self, now: Instant) -> Option<&ImeObservation> {
        self.per_source
            .iter()
            .filter(|o| !o.is_expired(now))
            .max_by(|a, b| a.confidence.cmp(&b.confidence).then(a.at.cmp(&b.at)))
    }

    /// 直近 `window` 内に複数ソースが同じ値で合意しているか。
    ///
    /// 2 ソース以上が同じ値を見ていれば `Some(value)` を返す。
    /// 値が分かれる、または 1 ソースしかない場合は `None`。
    #[must_use]
    pub fn consensus(&self, window: Duration, now: Instant) -> Option<bool> {
        let mut votes_true = 0;
        let mut votes_false = 0;
        for obs in self.per_source.iter() {
            if obs.age(now) > window || obs.is_expired(now) {
                continue;
            }
            if obs.open {
                votes_true += 1;
            } else {
                votes_false += 1;
            }
        }
        if votes_true >= 2 && votes_false == 0 {
            Some(true)
        } else if votes_false >= 2 && votes_true == 0 {
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(open: bool, source: ObservationSource, at: Instant) -> ImeObservation {
        ImeObservation {
            open,
            source,
            at,
            hwnd: HwndId::NULL,
            confidence: ObservationConfidence::Medium,
            expires_at: None,
        }
    }

    #[test]
    fn per_source_get_and_set() {
        let mut p = PerSourceObservations::default();
        let now = Instant::now();
        let o = obs(true, ObservationSource::Gji, now);
        p.set(ObservationSource::Gji, o);
        assert_eq!(p.get(ObservationSource::Gji).map(|x| x.open), Some(true));
        assert_eq!(p.get(ObservationSource::Tsf), None);
    }

    #[test]
    fn store_record_and_clear() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        s.record(obs(true, ObservationSource::ObserverPoll, now));
        assert!(s.per_source.observer_poll.is_some());
        s.clear_on_focus_change();
        assert!(s.per_source.observer_poll.is_none());
    }

    #[test]
    fn drift_tracking() {
        let mut s = ObservationStore::default();
        let t0 = Instant::now();
        // desired=true, observed=false → drift 開始
        s.update_drift(true, false, t0);
        assert!(s.drift.is_some());
        assert_eq!(s.drift.unwrap().started_at, t0);

        // 同じ desired/observed で再 update → started_at 維持
        let t1 = t0 + Duration::from_millis(50);
        s.update_drift(true, false, t1);
        assert_eq!(s.drift.unwrap().started_at, t0, "started_at 維持");

        // desired と observed が一致 → drift clear
        s.update_drift(true, true, t1);
        assert!(s.drift.is_none());
    }

    #[test]
    fn most_recent_trusted_by_confidence() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut low = obs(true, ObservationSource::FocusProbe, now);
        low.confidence = ObservationConfidence::Low;
        let mut high = obs(false, ObservationSource::ImmGetOpenStatus, now);
        high.confidence = ObservationConfidence::High;
        s.record(low);
        s.record(high);
        assert_eq!(
            s.most_recent_trusted(now).map(|o| o.open),
            Some(false),
            "High confidence が勝つ"
        );
    }

    #[test]
    fn consensus_requires_two_sources() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let window = Duration::from_millis(500);

        s.record(obs(true, ObservationSource::ObserverPoll, now));
        assert_eq!(s.consensus(window, now), None, "1 ソースでは合意なし");

        s.record(obs(true, ObservationSource::Gji, now));
        assert_eq!(s.consensus(window, now), Some(true), "2 ソース合意");

        s.record(obs(false, ObservationSource::Tsf, now));
        assert_eq!(s.consensus(window, now), None, "意見が分かれたら合意なし");
    }

    #[test]
    fn expired_observation_excluded() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut o = obs(true, ObservationSource::Gji, now);
        o.expires_at = Some(now);
        s.record(o);
        assert_eq!(s.most_recent_trusted(now), None, "expire 済みは除外");
    }
}
