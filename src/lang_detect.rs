//! エピソード先頭の打鍵列から、日本語（NICOLA）入力かどうかを判定する純粋ロジック（ADR-263 PR 1）。
//!
//! 窓切替直後・長い無操作のあとの最初の数打鍵を、Engine の手前で保持して判定する。判定は次の 2 つだけで、
//! 打鍵の時刻は使わない（Down/Up の**順序**だけを見る）。
//!
//! 1. **親指と文字キーの重なり**: 親指の Down のときに文字キーが押されたまま、または文字キーの Down のときに
//!    親指が押されたまま。親指の単独タップ（重なりなし）は証拠にしない。無変換/変換・F13〜F24・Alt なりすましは
//!    英文の打鍵に現れないので重なりを証拠にできるが、Space を親指キーにした構成は英文のロールオーバーでも
//!    重なりが成立するので、呼び出し側（殻）がエピソードごと対象外にすること（ADR-263 D3）。
//! 2. **仮名 n-gram の対数尤度比**: 親指が出ていない間は、未シフト面でデコードした仮名列が日本語として自然かを、
//!    言語非依存（一様）の背景モデルと比べる。`LLR = Σ [ln P_ja(仮名_i | 仮名_i-1) − ln(1/N_keys)]`。
//!    英語専用のモデルは持たない（コーディング・他言語との併用のため）。
//!
//! 結果は次の 4 つ。**「日本語でない」と「決められない」は殻では同じ扱い**（保持した打鍵を今までと同じ流れに戻す）
//! なので、`en_threshold` は小さめ（英数側を早く流す）、`ja_threshold` は大きめ（日本語を慎重に）の非対称にする。
//!
//! 尤度比の単位は自然対数（nats）。背景の一様分布の台 `N_keys` は、配列の未シフト面（`YabFace`）で定義されているキーの数
//! （`nicola.yab` で 44）。仮名 n-gram の表は `score = log10(最大を 1 とした相対頻度)` の形（`data/ngram_hiragana`）を前提にする。
//!
//! 呼び出し側（殻）の約束: 判定に使う文字キーの Down を渡したら、その Up も必ず渡す（渡し損ねると、その文字が
//! 押下中のまま残り、後の親指の単独タップが重なりとして成立する）。Space を親指キーにした構成では
//! `ThumbOverlap::Disabled` を選ぶ（型で選ばせ、忘れないようにしている）。
//!
//! このモジュールは OS・時刻・生の VK を持たない。入力は分類済みの `PhysicalPos` だけ（ADR-019）。

use rustc_hash::{FxHashMap, FxHashSet};

use crate::engine::fsm_types::ThumbSide;
use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::yab::{YabFace, YabValue};

/// .yab の面の行数・列数の上限（`yab::MAX_ROWS`/`MAX_COLS` の値の写し。`yab` 側を変えたらここも合わせる）
const GRID_ROWS: u8 = 4;
const GRID_COLS: u8 = 13;

const fn thumb_index(side: ThumbSide) -> usize {
    match side {
        ThumbSide::Left => 0,
        ThumbSide::Right => 1,
    }
}

/// 親指と文字キーの重なりを日本語の証拠にするか。殻が必ず選ぶ（既定なし）。
///
/// Space を親指キーにした構成（`keyboard_layout = "us"` の代替）や ADR-255 の構成では、英文のロールオーバー
/// （`e` Down→Space Down→`e` Up）でも重なりが成立するので `Disabled` にする。`Disabled` のとき親指のイベントは無視され、
/// 日本語の確定は仮名 n-gram の尤度比だけになる（実質ほとんど確定しない。殻はエピソードごと対象外にしてよい）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbOverlap {
    Enabled,
    Disabled,
}

/// 判定器への入力（分類済みの打鍵イベント）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectEvent {
    /// 文字キーの Down（自動リピートの再送は無視される）
    CharDown(PhysicalPos),
    CharUp(PhysicalPos),
    ThumbDown(ThumbSide),
    ThumbUp(ThumbSide),
}

/// 日本語と判定した根拠
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JapaneseEvidence {
    /// 親指と文字キーの重なり
    ThumbOverlap,
    /// 仮名 n-gram の対数尤度比が `ja_threshold` に達した
    Likelihood,
}

/// 判定結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// まだ決まらない（保持を続ける）
    Pending,
    /// 日本語（NICOLA）入力
    Japanese(JapaneseEvidence),
    /// 対数尤度比が `-en_threshold` 以下（日本語として不自然）
    NotJapanese,
    /// `max_keys` 打鍵まで見ても決まらなかった
    Undecided,
}

impl Verdict {
    /// 確定したか（`Pending` 以外）
    #[must_use]
    pub const fn is_decided(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// 判定のパラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectParams {
    /// この値以上で日本語と確定する（nats）。大きいほど ON の誤確定が減る
    pub ja_threshold: f64,
    /// `-` この値以下で「日本語でない」とする（nats）。小さいほど英数側が早く流れるが、
    /// 親指がまだ出ていない日本語を取り逃す（日本語の確定が減る）
    pub en_threshold: f64,
    /// 判定に使う打鍵（Down）の最大数。超えたら `Undecided`
    pub max_keys: usize,
}

impl Default for DetectParams {
    /// 人工データのオフライン評価（2026-10-17。出典: `research-private/v3-eval/eval7.py` と
    /// `2026-10-17-v3-eval-first-run.md` の「第4回」）。`ja_threshold = 4` で他言語の日本語への誤確定 0.2%。
    /// `en_threshold` ごとの（日本語の確定, 他言語の保持の平均打鍵）は 4: (79.6%, 1.9)、8: (91.9%, 2.9)、
    /// 14: (95.1%, 4.0)、100: (98.2%, 6.7)。8 は日本語の取り逃し 8% と英数側の保持 3 打鍵弱の折り合い。
    /// 日本語の確定のほぼ全部は親指の重なりで、尤度比は『英数側を早く流す』ために効く。実ジャーナルで再校正する（PR 2a）。
    fn default() -> Self {
        Self {
            ja_threshold: 4.0,
            en_threshold: 8.0,
            max_keys: 7,
        }
    }
}

/// 未シフト面の位置→仮名と、仮名 2 連の条件付き確率（`data/ngram_hiragana` の生の表から作る）
#[derive(Debug)]
pub struct KanaModel {
    /// 未シフト面（`YabFace`）で仮名に読めるキー
    key_kana: FxHashMap<PhysicalPos, char>,
    /// 背景の一様分布の台（未シフト面で定義されているキーの数）
    key_count: f64,
    /// 2 連の相対頻度（`10^score`）
    freq: FxHashMap<(char, char), f64>,
    /// 左の文字ごとの頻度の和
    left: FxHashMap<char, f64>,
    /// 左の文字ごとの、表にある 2 連の数
    listed: FxHashMap<char, usize>,
    /// 表に現れる文字の集合
    vocab: FxHashSet<char>,
    /// 表に無い組の相対頻度（表の最小値の 1/10）
    floor: f64,
    /// 左の文字の頻度の総和
    total: f64,
    /// 表が使えるか（語彙が空でない）。使えないとき尤度比は計算せず、日本語を尤度だけで確定しない
    usable: bool,
}

impl KanaModel {
    /// 仮名 n-gram の生の表と、未シフト面（`normal`）から作る。
    ///
    /// 表が空でも作れるが、その場合 `usable()` が偽になり、判定器は尤度比を使わない（重なりだけで日本語を確定する）。
    /// 空の表では 1 打鍵ごとに `+ln(N_keys)` が積まれて日本語に確定してしまうため、必ず `usable()` で分岐する。
    #[must_use]
    pub fn new(ngram: &NgramModel, normal: &YabFace) -> Self {
        let mut key_kana = FxHashMap::default();
        let mut defined = 0_u32;
        for row in 0..GRID_ROWS {
            for col in 0..GRID_COLS {
                let pos = PhysicalPos::new(row, col);
                match normal.get(&pos) {
                    None | Some(YabValue::None) => {}
                    Some(YabValue::Romaji {
                        kana: Some(kana), ..
                    }) => {
                        defined += 1;
                        key_kana.insert(pos, *kana);
                    }
                    Some(_) => defined += 1,
                }
            }
        }

        let mut freq = FxHashMap::default();
        let mut left: FxHashMap<char, f64> = FxHashMap::default();
        let mut listed: FxHashMap<char, usize> = FxHashMap::default();
        let mut vocab = FxHashSet::default();
        let mut min = f64::INFINITY;
        for ((a, b), score) in ngram.bigram_scores() {
            let f = 10_f64.powf(f64::from(score));
            freq.insert((a, b), f);
            *left.entry(a).or_insert(0.0) += f;
            *listed.entry(a).or_insert(0) += 1;
            vocab.insert(a);
            vocab.insert(b);
            min = min.min(f);
        }
        let total: f64 = left.values().sum();
        let usable = !vocab.is_empty() && total > 0.0;
        let floor = if min.is_finite() { min * 0.1 } else { 1e-12 };

        Self {
            key_kana,
            key_count: f64::from(defined.max(1)),
            freq,
            left,
            listed,
            vocab,
            floor,
            total,
            usable,
        }
    }

    /// 仮名 n-gram の表が使えるか
    #[must_use]
    pub const fn usable(&self) -> bool {
        self.usable
    }

    /// 1 打鍵ぶんの対数尤度比の増分 `ln P_ja(token | prev) − ln(1/N_keys)`。
    ///
    /// `prev` は直前の打鍵の仮名（直前が仮名に読めなかった・先頭なら `None`）。
    #[allow(clippy::cast_precision_loss)] // 語彙の数（高々数百）なので精度は問題にならない
    fn delta(&self, prev: Option<char>, pos: PhysicalPos) -> f64 {
        let background = -self.key_count.ln();
        let low = (self.floor / self.total.max(self.floor)).ln();
        let Some(&token) = self.key_kana.get(&pos) else {
            return low - background;
        };
        if !self.vocab.contains(&token) {
            return low - background;
        }
        let vocab_len = self.vocab.len() as f64;
        let ja = match prev {
            Some(a) if self.vocab.contains(&a) => {
                let listed = self.listed.get(&a).copied().unwrap_or(0) as f64;
                let denominator = (vocab_len - listed)
                    .mul_add(self.floor, self.left.get(&a).copied().unwrap_or(0.0));
                let numerator = self.freq.get(&(a, token)).copied().unwrap_or(self.floor);
                (numerator / denominator).ln()
            }
            _ => {
                let left = self.left.get(&token).copied().unwrap_or(0.0);
                ((left + self.floor) / vocab_len.mul_add(self.floor, self.total)).ln()
            }
        };
        ja - background
    }
}

/// 1 エピソードぶんの判定器。`observe` に打鍵イベントを順に渡す。
///
/// 決まったあと（`Pending` 以外）は結果が変わらず、以降のイベントは無視される。
#[derive(Debug)]
pub struct EpisodeDetector {
    params: DetectParams,
    thumb_overlap: ThumbOverlap,
    held_chars: FxHashSet<PhysicalPos>,
    held_thumbs: [bool; 2],
    presses: usize,
    prev: Option<char>,
    llr: f64,
    /// 打鍵ごとの対数尤度比の増分（ログ用。キー位置は含まない）
    trace: Vec<f32>,
    verdict: Verdict,
}

impl EpisodeDetector {
    /// 判定器を作る。`thumbs_held` はエピソードの開始前から押されていた親指（左, 右。`RawKeyEvent` の
    /// `left/right_thumb_down_snapshot`）で、連続シフトの途中や窓切替の直前から押したままの親指の重なりを取り逃さないため。
    /// `KanaModel` は持たず、`observe` のたびに借りる（殻が `KanaModel` と判定器を同じ構造体に持てる。
    /// 設定のリロードでモデルが差し替わっても、進行中のエピソードが古いモデルを使い続けない）。
    #[must_use]
    pub fn new(params: DetectParams, thumb_overlap: ThumbOverlap, thumbs_held: [bool; 2]) -> Self {
        Self {
            params,
            thumb_overlap,
            held_chars: FxHashSet::default(),
            held_thumbs: thumbs_held,
            presses: 0,
            prev: None,
            llr: 0.0,
            trace: Vec::new(),
            verdict: Verdict::Pending,
        }
    }

    /// 現在の判定
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// これまでの対数尤度比（ログ用）
    #[must_use]
    pub const fn llr(&self) -> f64 {
        self.llr
    }

    /// 打鍵ごとの対数尤度比の増分の列（ログ用。表が使えないときは空）
    #[must_use]
    pub fn llr_trace(&self) -> &[f32] {
        &self.trace
    }

    /// 数えた文字キーの Down の数（自動リピートは数えない）
    #[must_use]
    pub const fn presses(&self) -> usize {
        self.presses
    }

    /// 打鍵イベントを 1 つ渡し、判定を返す。`model` は毎回同じものを渡すこと。
    pub fn observe(&mut self, model: &KanaModel, event: DetectEvent) -> Verdict {
        if self.verdict.is_decided() {
            return self.verdict;
        }
        match event {
            DetectEvent::ThumbDown(_) | DetectEvent::ThumbUp(_)
                if self.thumb_overlap == ThumbOverlap::Disabled => {}
            DetectEvent::ThumbDown(side) => {
                if self.held_chars.is_empty() {
                    self.held_thumbs[thumb_index(side)] = true;
                } else {
                    self.verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);
                }
            }
            DetectEvent::ThumbUp(side) => self.held_thumbs[thumb_index(side)] = false,
            DetectEvent::CharUp(pos) => {
                self.held_chars.remove(&pos);
            }
            DetectEvent::CharDown(pos) => self.on_char_down(model, pos),
        }
        self.verdict
    }

    fn on_char_down(&mut self, model: &KanaModel, pos: PhysicalPos) {
        // 押したままの自動リピート（Up より前の再送）は数えない
        if !self.held_chars.insert(pos) {
            return;
        }
        if self.thumb_overlap == ThumbOverlap::Enabled && self.held_thumbs.iter().any(|&down| down)
        {
            self.verdict = Verdict::Japanese(JapaneseEvidence::ThumbOverlap);
            return;
        }
        self.presses += 1;
        if model.usable {
            let delta = model.delta(self.prev, pos);
            self.llr += delta;
            #[allow(clippy::cast_possible_truncation)] // ログ用の増分。f32 の精度で足りる
            self.trace.push(delta as f32);
            self.prev = model.key_kana.get(&pos).copied();
        }

        let by_likelihood = if self.llr >= self.params.ja_threshold {
            Some(Verdict::Japanese(JapaneseEvidence::Likelihood))
        } else if self.llr <= -self.params.en_threshold {
            Some(Verdict::NotJapanese)
        } else {
            None
        }
        // 表が使えないときは尤度比を積んでいないので、閾値が 0 以下でも尤度では決めない
        .filter(|_| model.usable);
        self.verdict = by_likelihood.unwrap_or(if self.presses >= self.params.max_keys {
            Verdict::Undecided
        } else {
            Verdict::Pending
        });
    }
}

/// 1 打鍵の役割（殻が `RawKeyEvent` から作る）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRole {
    /// 文字キー（物理位置）
    Char(PhysicalPos),
    /// 親指キー
    Thumb(ThumbSide),
    /// Shift など修飾キー自身のイベント（エピソードに影響しない）
    Modifier,
    /// 判定の材料でない打鍵（矢印・Tab・Space〔親指でないとき〕・Enter・BackSpace・Esc 等）
    Other,
}

/// エピソードの追跡器に渡す 1 打鍵
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedKey {
    pub role: KeyRole,
    /// Down なら `true`
    pub down: bool,
    /// Ctrl/Alt/Win を伴う・awase の注入・IME キー（エピソードを打ち切る）
    pub excluded: bool,
    pub timestamp_ms: u64,
}

/// エピソードが始まった契機
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 前面窓が変わった直後
    WindowSwitch,
    /// 長い無操作のあと
    Idle,
}

impl Trigger {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WindowSwitch => "window-switch",
            Self::Idle => "idle",
        }
    }
}

/// エピソードを閉じた理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// 日本語/日本語でないが確定した
    Decided,
    /// `max_keys` 打鍵まで見ても決まらなかった
    MaxKeys,
    /// `max_ms` が経った
    Timeout,
    /// 前面窓が変わった
    WindowChanged,
    /// 修飾付き・注入・IME キーが来た
    ExcludedKey,
    /// 判定の材料でない打鍵（矢印・Tab・Enter 等）が来た
    NonMaterialKey,
}

impl StopReason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Decided => "decided",
            Self::MaxKeys => "max-keys",
            Self::Timeout => "timeout",
            Self::WindowChanged => "window-changed",
            Self::ExcludedKey => "excluded-key",
            Self::NonMaterialKey => "non-material-key",
        }
    }
}

/// 閉じたエピソードの結果（ログ用。キー位置の列は含まない）
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeOutcome {
    pub trigger: Trigger,
    pub verdict: Verdict,
    pub stop: StopReason,
    pub presses: usize,
    pub llr: f64,
    pub llr_trace: Vec<f32>,
    /// エピソード開始時の無操作の長さ（窓切替の契機でも、直前の打鍵からの経過）
    pub idle_ms: u64,
    pub duration_ms: u64,
}

/// 追跡器の設定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackerConfig {
    pub params: DetectParams,
    pub thumb_overlap: ThumbOverlap,
    /// この長さ以上の無操作のあとの最初の打鍵でエピソードを始める
    pub idle_threshold_ms: u64,
    /// エピソードの最大の長さ
    pub max_ms: u64,
}

#[derive(Debug)]
struct ActiveEpisode {
    detector: EpisodeDetector,
    trigger: Trigger,
    idle_ms: u64,
    scope: u64,
    started_ms: u64,
}

/// 窓切替直後・長い無操作のあとのエピソードを見つけ、判定器を回して結果を返す追跡器（挙動を変えない観測用）。
///
/// 殻は取り込み口（フックの打鍵が届いた順）で `on_key` を呼ぶ。`scope` は前面窓を表す識別子
/// （hwnd と pid から作る。窓が変わったかの比較にだけ使う）。
#[derive(Debug)]
pub struct EpisodeTracker {
    cfg: TrackerConfig,
    last_scope: Option<u64>,
    last_key_ms: Option<u64>,
    armed: Option<(Trigger, u64)>,
    active: Option<ActiveEpisode>,
}

impl EpisodeTracker {
    #[must_use]
    pub const fn new(cfg: TrackerConfig) -> Self {
        Self {
            cfg,
            last_scope: None,
            last_key_ms: None,
            armed: None,
            active: None,
        }
    }

    /// 打鍵を 1 つ渡す。エピソードが閉じたら結果を返す。
    ///
    /// 開始は、窓切替または長い無操作のあとの**最初の Down**が文字キーか親指のとき（修飾なし・注入でない）。
    /// 矢印・Tab・Enter 等で始まる再開はエピソードにしない。
    pub fn on_key(
        &mut self,
        model: &KanaModel,
        key: ObservedKey,
        scope: u64,
    ) -> Option<EpisodeOutcome> {
        let now = key.timestamp_ms;
        let mut outcome = self.close_if_stale(scope, now);
        self.arm(scope, now);
        self.maybe_start(key, scope);
        if self.active.is_some() {
            let closed = self.feed(model, key, now);
            if outcome.is_none() {
                outcome = closed;
            }
        }
        outcome
    }

    /// 進行中のエピソードが、窓の変化・期限で閉じる。
    fn close_if_stale(&mut self, scope: u64, now: u64) -> Option<EpisodeOutcome> {
        let active = self.active.as_ref()?;
        if active.scope != scope {
            self.finish(StopReason::WindowChanged, now)
        } else if now.saturating_sub(active.started_ms) >= self.cfg.max_ms {
            self.finish(StopReason::Timeout, now)
        } else {
            None
        }
    }

    /// 窓切替・無操作を検出して武装する（エピソードが無いときだけ）。窓切替が無操作より優先される。
    fn arm(&mut self, scope: u64, now: u64) {
        let idle = self.last_key_ms.map_or(0, |last| now.saturating_sub(last));
        if self.active.is_none() {
            if self.last_scope != Some(scope) {
                self.armed = Some((Trigger::WindowSwitch, idle));
            } else if self.armed.is_none() && idle >= self.cfg.idle_threshold_ms {
                self.armed = Some((Trigger::Idle, idle));
            }
        }
        self.last_scope = Some(scope);
        self.last_key_ms = Some(now);
    }

    /// 武装後の最初の Down で、エピソードを始めるか見送る（Shift 等の修飾キー自身は数えない）。
    fn maybe_start(&mut self, key: ObservedKey, scope: u64) {
        if !key.down || self.active.is_some() || key.role == KeyRole::Modifier {
            return;
        }
        let Some((trigger, idle_ms)) = self.armed.take() else {
            return;
        };
        let material = !key.excluded && matches!(key.role, KeyRole::Char(_) | KeyRole::Thumb(_));
        if material {
            self.active = Some(ActiveEpisode {
                detector: EpisodeDetector::new(self.cfg.params, self.cfg.thumb_overlap, [false; 2]),
                trigger,
                idle_ms,
                scope,
                started_ms: key.timestamp_ms,
            });
        }
    }

    fn feed(&mut self, model: &KanaModel, key: ObservedKey, now: u64) -> Option<EpisodeOutcome> {
        if key.down && key.excluded {
            return self.finish(StopReason::ExcludedKey, now);
        }
        let event = match key.role {
            KeyRole::Modifier => return None,
            KeyRole::Other => {
                return key
                    .down
                    .then(|| self.finish(StopReason::NonMaterialKey, now))
                    .flatten();
            }
            KeyRole::Char(pos) if key.down => DetectEvent::CharDown(pos),
            KeyRole::Char(pos) => DetectEvent::CharUp(pos),
            KeyRole::Thumb(side) if key.down => DetectEvent::ThumbDown(side),
            KeyRole::Thumb(side) => DetectEvent::ThumbUp(side),
        };
        let active = self.active.as_mut()?;
        let verdict = active.detector.observe(model, event);
        match verdict {
            Verdict::Pending => None,
            Verdict::Undecided => self.finish(StopReason::MaxKeys, now),
            Verdict::Japanese(_) | Verdict::NotJapanese => self.finish(StopReason::Decided, now),
        }
    }

    fn finish(&mut self, stop: StopReason, now: u64) -> Option<EpisodeOutcome> {
        let active = self.active.take()?;
        Some(EpisodeOutcome {
            trigger: active.trigger,
            verdict: active.detector.verdict(),
            stop,
            presses: active.detector.presses(),
            llr: active.detector.llr(),
            llr_trace: active.detector.llr_trace().to_vec(),
            idle_ms: active.idle_ms,
            duration_ms: now.saturating_sub(active.started_ms),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanmap::KeyboardModel;
    use crate::yab::YabLayout;
    use std::path::Path;

    fn pos(row: u8, col: u8) -> PhysicalPos {
        PhysicalPos::new(row, col)
    }

    /// 小さな合成モデル: か(1,1) た(1,2) こ(1,3) さ(1,4)。「かた」「たこ」「こさ」が自然、他は表に無い。
    fn tiny_model() -> KanaModel {
        let ngram =
            NgramModel::from_csv("2,かた,0.0\n2,たこ,0.0\n2,こさ,0.0\n2,かこ,-3.0\n", 0, 0, 0)
                .expect("csv");
        let mut face = YabFace::new();
        for (col, (romaji, kana)) in [("ka", 'か'), ("ta", 'た'), ("ko", 'こ'), ("sa", 'さ')]
            .into_iter()
            .enumerate()
        {
            face.insert(
                pos(1, u8::try_from(col + 1).expect("col")),
                YabValue::Romaji {
                    romaji: romaji.to_string(),
                    kana: Some(kana),
                },
            );
        }
        // 仮名に読めないが定義されているキー（記号）
        face.insert(pos(0, 0), YabValue::Literal("1".to_string()));
        KanaModel::new(&ngram, &face)
    }

    fn params(ja: f64, en: f64, max_keys: usize) -> DetectParams {
        DetectParams {
            ja_threshold: ja,
            en_threshold: en,
            max_keys,
        }
    }

    #[test]
    fn thumb_down_while_char_held_is_overlap() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        assert_eq!(
            d.observe(&model, DetectEvent::CharDown(pos(1, 1))),
            Verdict::Pending
        );
        assert_eq!(
            d.observe(&model, DetectEvent::ThumbDown(ThumbSide::Left)),
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        );
    }

    #[test]
    fn char_down_while_thumb_held_is_overlap() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        assert_eq!(
            d.observe(&model, DetectEvent::ThumbDown(ThumbSide::Right)),
            Verdict::Pending
        );
        assert_eq!(
            d.observe(&model, DetectEvent::CharDown(pos(1, 2))),
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        );
    }

    #[test]
    fn thumb_tap_alone_is_not_evidence() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        d.observe(&model, DetectEvent::ThumbDown(ThumbSide::Left));
        d.observe(&model, DetectEvent::ThumbUp(ThumbSide::Left));
        assert_eq!(
            d.observe(&model, DetectEvent::CharDown(pos(1, 1))),
            Verdict::Pending
        );
    }

    #[test]
    fn char_released_before_thumb_is_not_overlap() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        d.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        d.observe(&model, DetectEvent::CharUp(pos(1, 1)));
        assert_eq!(
            d.observe(&model, DetectEvent::ThumbDown(ThumbSide::Left)),
            Verdict::Pending
        );
    }

    #[test]
    fn auto_repeat_is_not_counted() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        d.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        d.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        d.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        assert_eq!(d.presses(), 1);
    }

    #[test]
    fn natural_kana_sequence_reaches_japanese_by_likelihood() {
        let model = tiny_model();
        // か→た→こ→さ は表で自然。閾値を低くして、尤度だけで日本語に届くことを確かめる。
        let mut d = EpisodeDetector::new(params(1.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        let mut last = Verdict::Pending;
        for col in 1..=4 {
            last = d.observe(&model, DetectEvent::CharDown(pos(1, col)));
            d.observe(&model, DetectEvent::CharUp(pos(1, col)));
        }
        assert_eq!(last, Verdict::Japanese(JapaneseEvidence::Likelihood));
        assert!(d.llr() >= 1.0);
    }

    #[test]
    fn unnatural_sequence_becomes_not_japanese() {
        let model = tiny_model();
        // 記号キー(0,0)は仮名に読めず、尤度が下限になる。en_threshold を低くして早く「日本語でない」になる。
        let mut d = EpisodeDetector::new(params(100.0, 5.0, 7), ThumbOverlap::Enabled, [false; 2]);
        let mut last = Verdict::Pending;
        for _ in 0..3 {
            last = d.observe(&model, DetectEvent::CharDown(pos(0, 0)));
            d.observe(&model, DetectEvent::CharUp(pos(0, 0)));
        }
        assert_eq!(last, Verdict::NotJapanese);
    }

    #[test]
    fn max_keys_without_a_decision_is_undecided() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(1000.0, 1000.0, 3), ThumbOverlap::Enabled, [false; 2]);
        let mut last = Verdict::Pending;
        for col in 1..=3 {
            last = d.observe(&model, DetectEvent::CharDown(pos(1, col)));
            d.observe(&model, DetectEvent::CharUp(pos(1, col)));
        }
        assert_eq!(last, Verdict::Undecided);
    }

    #[test]
    fn verdict_is_sticky_after_a_decision() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        d.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        let decided = d.observe(&model, DetectEvent::ThumbDown(ThumbSide::Left));
        assert_eq!(
            d.observe(&model, DetectEvent::ThumbUp(ThumbSide::Left)),
            decided,
            "決まったあとはイベントを無視する"
        );
    }

    #[test]
    fn empty_ngram_is_unusable_and_never_decides_japanese_by_likelihood() {
        // 面は複数キー（N>=2）にする。1 キーだと背景の増分が 0 になり、空の表のバグを見逃す。
        let ngram = NgramModel::new(0, 0, 0);
        let mut face = YabFace::new();
        for (col, (romaji, kana)) in [("ka", 'か'), ("ta", 'た'), ("ko", 'こ')]
            .into_iter()
            .enumerate()
        {
            face.insert(
                pos(1, u8::try_from(col + 1).expect("col")),
                YabValue::Romaji {
                    romaji: romaji.to_string(),
                    kana: Some(kana),
                },
            );
        }
        let model = KanaModel::new(&ngram, &face);
        assert!(!model.usable());
        let mut d =
            EpisodeDetector::new(DetectParams::default(), ThumbOverlap::Enabled, [false; 2]);
        let mut last = Verdict::Pending;
        for col in 1..=3 {
            last = d.observe(&model, DetectEvent::CharDown(pos(1, col)));
            d.observe(&model, DetectEvent::CharUp(pos(1, col)));
            assert!(
                !matches!(last, Verdict::Japanese(_)),
                "表が空なのに日本語と確定した（{col} 打鍵目）"
            );
        }
        assert!(
            d.llr().abs() < f64::EPSILON,
            "表が使えないとき尤度比は積まない"
        );
        // max_keys（7）まで見ても尤度では決まらず、重なりだけが日本語の根拠になる。
        assert_eq!(last, Verdict::Pending);
    }

    #[test]
    fn disabled_thumb_overlap_ignores_thumb_events() {
        let model = tiny_model();
        let mut d =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Disabled, [false; 2]);
        d.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        assert_eq!(
            d.observe(&model, DetectEvent::ThumbDown(ThumbSide::Left)),
            Verdict::Pending,
            "Space を親指にした構成では重なりを証拠にしない"
        );
        assert_eq!(
            d.observe(&model, DetectEvent::CharDown(pos(1, 2))),
            Verdict::Pending
        );
    }

    #[test]
    fn thumb_held_before_the_episode_counts_as_overlap() {
        let model = tiny_model();
        let mut d = EpisodeDetector::new(
            params(100.0, 100.0, 7),
            ThumbOverlap::Enabled,
            [true, false],
        );
        assert_eq!(
            d.observe(&model, DetectEvent::CharDown(pos(1, 1))),
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        );
    }

    // ---- EpisodeTracker ----

    fn tracker_cfg() -> TrackerConfig {
        TrackerConfig {
            params: params(100.0, 100.0, 7),
            thumb_overlap: ThumbOverlap::Enabled,
            idle_threshold_ms: 3000,
            max_ms: 2500,
        }
    }

    fn key(role: KeyRole, down: bool, t: u64) -> ObservedKey {
        ObservedKey {
            role,
            down,
            excluded: false,
            timestamp_ms: t,
        }
    }

    #[test]
    fn window_switch_starts_an_episode_and_overlap_closes_it() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        // 最初の打鍵（窓 1）: 文字 Down でエピソード開始。決まらない。
        assert!(t
            .on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1)
            .is_none());
        // 親指 Down（文字が押下中）で日本語（重なり）。
        let out = t
            .on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), true, 20), 1)
            .expect("closed");
        assert_eq!(out.trigger, Trigger::WindowSwitch);
        assert_eq!(
            out.verdict,
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap)
        );
        assert_eq!(out.stop, StopReason::Decided);
        assert_eq!(out.presses, 1);
    }

    #[test]
    fn same_window_short_idle_does_not_start_a_new_episode() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        t.on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), true, 20), 1);
        // 決定後、同じ窓で 100ms 後の打鍵ではエピソードを始めない。
        assert!(t
            .on_key(&model, key(KeyRole::Char(pos(1, 2)), true, 120), 1)
            .is_none());
        assert!(t
            .on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), true, 130), 1)
            .is_none());
    }

    #[test]
    fn long_idle_starts_an_episode_with_idle_trigger() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        t.on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), true, 20), 1);
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), false, 30), 1);
        t.on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), false, 40), 1);
        // 10 秒後（同じ窓）: 無操作の契機。
        t.on_key(&model, key(KeyRole::Char(pos(1, 2)), true, 10_040), 1);
        let out = t
            .on_key(
                &model,
                key(KeyRole::Thumb(ThumbSide::Right), true, 10_050),
                1,
            )
            .expect("closed");
        assert_eq!(out.trigger, Trigger::Idle);
        assert_eq!(out.idle_ms, 10_000);
    }

    #[test]
    fn episode_is_not_started_by_a_non_material_key() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        // 窓切替の最初の Down が矢印（Other）なら、エピソードにしない。
        assert!(t.on_key(&model, key(KeyRole::Other, true, 10), 1).is_none());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 20), 1);
        assert!(t
            .on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), true, 30), 1)
            .is_none());
    }

    #[test]
    fn modifier_key_events_do_not_consume_the_arm() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Modifier, true, 5), 1);
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        let out = t
            .on_key(&model, key(KeyRole::Thumb(ThumbSide::Left), true, 20), 1)
            .expect("closed");
        assert_eq!(out.stop, StopReason::Decided);
    }

    #[test]
    fn non_material_or_excluded_key_closes_the_episode() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        let out = t
            .on_key(&model, key(KeyRole::Other, true, 20), 1)
            .expect("closed");
        assert_eq!(out.stop, StopReason::NonMaterialKey);

        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        let mut excluded = key(KeyRole::Char(pos(1, 2)), true, 20);
        excluded.excluded = true;
        let out = t.on_key(&model, excluded, 1).expect("closed");
        assert_eq!(out.stop, StopReason::ExcludedKey);
    }

    #[test]
    fn window_change_and_timeout_close_the_episode() {
        let model = tiny_model();
        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        let out = t
            .on_key(&model, key(KeyRole::Char(pos(1, 2)), true, 20), 2)
            .expect("closed");
        assert_eq!(out.stop, StopReason::WindowChanged);

        let mut t = EpisodeTracker::new(tracker_cfg());
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1);
        t.on_key(&model, key(KeyRole::Char(pos(1, 1)), false, 20), 1);
        let out = t
            .on_key(&model, key(KeyRole::Char(pos(1, 2)), true, 2_600), 1)
            .expect("closed");
        assert_eq!(out.stop, StopReason::Timeout);
    }

    #[test]
    fn episode_outcome_carries_the_likelihood_trace() {
        let model = tiny_model();
        let mut cfg = tracker_cfg();
        cfg.params = params(1.0, 100.0, 7);
        let mut t = EpisodeTracker::new(cfg);
        let mut last = None;
        for (i, col) in [1_u8, 2, 3].into_iter().enumerate() {
            let ts = 10 + u64::try_from(i).expect("i") * 10;
            let down = t.on_key(&model, key(KeyRole::Char(pos(1, col)), true, ts), 1);
            let up = t.on_key(&model, key(KeyRole::Char(pos(1, col)), false, ts + 1), 1);
            last = last.or(down).or(up);
        }
        let out = last.expect("closed by likelihood");
        assert_eq!(out.verdict, Verdict::Japanese(JapaneseEvidence::Likelihood));
        assert_eq!(out.llr_trace.len(), out.presses);
    }

    // ---- 実データ（data/ngram_hiragana.csv.gz と layout/nicola.yab）での回帰 ----

    fn real_model() -> KanaModel {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ngram = NgramModel::from_file(&root.join("data/ngram_hiragana.csv.gz"), 0, 0, 0)
            .expect("ngram file");
        let yab = std::fs::read_to_string(root.join("layout/nicola.yab")).expect("yab");
        let layout = YabLayout::parse(&yab, KeyboardModel::Jis).expect("layout");
        KanaModel::new(&ngram, &layout.normal)
    }

    /// QWERTY の物理位置（yab の行・列。JIS）
    fn qwerty(c: char) -> PhysicalPos {
        const ROWS: [&str; 4] = ["1234567890-", "qwertyuiop@[", "asdfghjkl;:]", "zxcvbnm,./"];
        for (r, row) in ROWS.iter().enumerate() {
            if let Some(col) = row.find(c) {
                return pos(
                    u8::try_from(r).expect("row"),
                    u8::try_from(col).expect("col"),
                );
            }
        }
        panic!("QWERTY に無い文字: {c}");
    }

    fn run(model: &KanaModel, text: &str) -> (Verdict, usize) {
        let mut d =
            EpisodeDetector::new(DetectParams::default(), ThumbOverlap::Enabled, [false; 2]);
        for c in text.chars() {
            d.observe(model, DetectEvent::CharDown(qwerty(c)));
            d.observe(model, DetectEvent::CharUp(qwerty(c)));
            if d.verdict().is_decided() {
                break;
            }
        }
        (d.verdict(), d.presses())
    }

    #[test]
    fn english_and_code_are_not_decided_japanese_with_default_params() {
        let model = real_model();
        for text in [
            "thequickbrown",
            "gitstatus",
            "importos",
            "constx",
            "cargotest",
        ] {
            let (verdict, _) = run(&model, text);
            assert!(
                !matches!(verdict, Verdict::Japanese(_)),
                "{text} が日本語と判定された: {verdict:?}"
            );
        }
    }

    #[test]
    fn english_code_is_released_early_as_not_japanese() {
        let model = real_model();
        // 「thequickbrown」は 2 打鍵目で尤度比が -9.3（1 打鍵目 -2.2）となり、en_threshold = 8 を超える。7 打鍵を待たずに解放できる。
        let (verdict, presses) = run(&model, "thequickbrown");
        assert_eq!(verdict, Verdict::NotJapanese);
        assert!(presses <= 3, "保持が長すぎる: {presses} 打鍵");
    }
}
