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
    ///
    /// **仮名に読めないキー（記号・数字・未定義）は 0（中立）**。仮名 n-gram は仮名の並びの自然さしか測れず、
    /// 記号は日本語（`、` `。` `ー` 数字）にも英文にも現れるので、どちらの証拠にもならない。以前は表の最小値の 1/10 を
    /// 総和で割った下限（約 -33 nats、`en_threshold` の 4 倍）を課していて、親指なしの `¥` や `。` 1 打で
    /// 「日本語でない」と確定し、日本語の入力中に半角英数へ落ちた（2026-10-24、実機の journal で
    /// `llr=-43.1` と `llr=-33.2`）。評価（`research-private/v3-eval/eval8_symbols.py`）で、文中から打ち始めた
    /// NICOLA の親指なし区間の英数への誤判定は 22.1%（先頭が記号なら 100%）から 7.0% に下がる。
    #[allow(clippy::cast_precision_loss)] // 語彙の数（高々数百）なので精度は問題にならない
    fn delta(&self, prev: Option<char>, pos: PhysicalPos) -> f64 {
        let background = -self.key_count.ln();
        let Some(&token) = self.key_kana.get(&pos) else {
            return 0.0;
        };
        if !self.vocab.contains(&token) {
            return 0.0;
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

/// `EpisodeTracker::on_key_step` の結果
#[derive(Debug, Clone, PartialEq)]
pub struct KeyStep {
    /// この打鍵が来る前から進行中だったエピソードが、窓の変化・期限で閉じた（この打鍵はそのエピソードに属さない）
    pub closed_prev: Option<EpisodeOutcome>,
    /// この打鍵でエピソードが閉じた（この打鍵はそのエピソードに属する）
    pub closed_now: Option<EpisodeOutcome>,
    /// この打鍵が、進行中のエピソード（この打鍵で始まったもの、この打鍵で閉じたものを含む）に属するか
    pub in_episode: bool,
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

impl ActiveEpisode {
    /// エピソードを `stop` の理由で閉じて、結果にする。
    fn finish(self, stop: StopReason, now: u64) -> EpisodeOutcome {
        EpisodeOutcome {
            trigger: self.trigger,
            verdict: self.detector.verdict(),
            stop,
            presses: self.detector.presses(),
            llr: self.detector.llr(),
            llr_trace: self.detector.llr_trace().to_vec(),
            idle_ms: self.idle_ms,
            duration_ms: now.saturating_sub(self.started_ms),
        }
    }
}

/// エピソード追跡・保持の位相の種類（外から見える名前。テスト・ログ用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhaseKind {
    /// エピソードも武装も無い
    Idle,
    /// 窓切替・長い無操作を検出して、次の最初の Down を待っている
    Armed,
    /// エピソードを判定中（打鍵は保持していない）
    Observing,
    /// エピソードを判定中で、打鍵を保持している
    Holding,
}

/// 追跡器の位相。**`Armed` と `Observing` は同時に成り立たない**（武装はエピソードを始めるときに使い切る）ので、
/// `Option` を 2 つ持つ代わりに 1 つの enum にしている。
#[derive(Debug)]
enum TrackPhase {
    Idle,
    Armed { trigger: Trigger, idle_ms: u64 },
    Observing { active: ActiveEpisode },
}

impl TrackPhase {
    const fn kind(&self) -> PhaseKind {
        match self {
            Self::Idle => PhaseKind::Idle,
            Self::Armed { .. } => PhaseKind::Armed,
            Self::Observing { .. } => PhaseKind::Observing,
        }
    }
}

/// 位相とは別の、打鍵をまたいで持ち越す文脈（直前の窓と打鍵の時刻）。
#[derive(Debug, Clone, Copy, Default)]
struct TrackContext {
    last_scope: Option<u64>,
    last_key_ms: Option<u64>,
}

/// 追跡器の 1 打鍵ぶんの遷移（純粋。`evolve` に当たる）。新しい位相・文脈と、この打鍵の結果を返す。
fn track_step(
    cfg: &TrackerConfig,
    ctx: TrackContext,
    phase: TrackPhase,
    model: &KanaModel,
    key: ObservedKey,
    scope: u64,
) -> (TrackPhase, TrackContext, KeyStep) {
    let now = key.timestamp_ms;
    let (phase, closed_prev) = close_if_stale(cfg, phase, scope, now);
    let (phase, ctx) = arm(cfg, ctx, phase, scope, now);
    let phase = maybe_start(cfg, phase, key, scope);
    let in_episode = matches!(phase, TrackPhase::Observing { .. });
    let (phase, closed_now) = if in_episode {
        feed(model, phase, key, now)
    } else {
        (phase, None)
    };
    (
        phase,
        ctx,
        KeyStep {
            closed_prev,
            closed_now,
            in_episode,
        },
    )
}

/// 進行中のエピソードが、窓の変化・期限で閉じる。
fn close_if_stale(
    cfg: &TrackerConfig,
    phase: TrackPhase,
    scope: u64,
    now: u64,
) -> (TrackPhase, Option<EpisodeOutcome>) {
    match phase {
        TrackPhase::Observing { active } => {
            if active.scope != scope {
                (
                    TrackPhase::Idle,
                    Some(active.finish(StopReason::WindowChanged, now)),
                )
            } else if now.saturating_sub(active.started_ms) >= cfg.max_ms {
                (
                    TrackPhase::Idle,
                    Some(active.finish(StopReason::Timeout, now)),
                )
            } else {
                (TrackPhase::Observing { active }, None)
            }
        }
        other => (other, None),
    }
}

/// 窓切替・無操作を検出して武装する（エピソードが無いときだけ）。窓切替が無操作より優先される。
fn arm(
    cfg: &TrackerConfig,
    ctx: TrackContext,
    phase: TrackPhase,
    scope: u64,
    now: u64,
) -> (TrackPhase, TrackContext) {
    let idle = ctx.last_key_ms.map_or(0, |last| now.saturating_sub(last));
    let phase = match phase {
        observing @ TrackPhase::Observing { .. } => observing,
        other => {
            if ctx.last_scope != Some(scope) {
                TrackPhase::Armed {
                    trigger: Trigger::WindowSwitch,
                    idle_ms: idle,
                }
            } else if matches!(other, TrackPhase::Idle) && idle >= cfg.idle_threshold_ms {
                TrackPhase::Armed {
                    trigger: Trigger::Idle,
                    idle_ms: idle,
                }
            } else {
                other
            }
        }
    };
    (
        phase,
        TrackContext {
            last_scope: Some(scope),
            last_key_ms: Some(now),
        },
    )
}

/// 武装後の最初の Down で、エピソードを始めるか見送る（Shift 等の修飾キー自身は数えない）。
/// 武装は、始めても見送っても使い切る。
fn maybe_start(cfg: &TrackerConfig, phase: TrackPhase, key: ObservedKey, scope: u64) -> TrackPhase {
    if !key.down || key.role == KeyRole::Modifier {
        return phase;
    }
    match phase {
        TrackPhase::Armed { trigger, idle_ms } => {
            let material =
                !key.excluded && matches!(key.role, KeyRole::Char(_) | KeyRole::Thumb(_));
            if material {
                TrackPhase::Observing {
                    active: ActiveEpisode {
                        detector: EpisodeDetector::new(cfg.params, cfg.thumb_overlap, [false; 2]),
                        trigger,
                        idle_ms,
                        scope,
                        started_ms: key.timestamp_ms,
                    },
                }
            } else {
                TrackPhase::Idle
            }
        }
        other => other,
    }
}

/// 進行中のエピソードに打鍵を渡す。閉じたら結果を返し、位相は `Idle` になる。
fn feed(
    model: &KanaModel,
    phase: TrackPhase,
    key: ObservedKey,
    now: u64,
) -> (TrackPhase, Option<EpisodeOutcome>) {
    let mut active = match phase {
        TrackPhase::Observing { active } => active,
        other => return (other, None),
    };
    if key.down && key.excluded {
        return (
            TrackPhase::Idle,
            Some(active.finish(StopReason::ExcludedKey, now)),
        );
    }
    let event = match key.role {
        KeyRole::Modifier => return (TrackPhase::Observing { active }, None),
        KeyRole::Other => {
            return if key.down {
                (
                    TrackPhase::Idle,
                    Some(active.finish(StopReason::NonMaterialKey, now)),
                )
            } else {
                (TrackPhase::Observing { active }, None)
            };
        }
        KeyRole::Char(pos) if key.down => DetectEvent::CharDown(pos),
        KeyRole::Char(pos) => DetectEvent::CharUp(pos),
        KeyRole::Thumb(side) if key.down => DetectEvent::ThumbDown(side),
        KeyRole::Thumb(side) => DetectEvent::ThumbUp(side),
    };
    match active.detector.observe(model, event) {
        Verdict::Pending => (TrackPhase::Observing { active }, None),
        Verdict::Undecided => (
            TrackPhase::Idle,
            Some(active.finish(StopReason::MaxKeys, now)),
        ),
        Verdict::Japanese(_) | Verdict::NotJapanese => (
            TrackPhase::Idle,
            Some(active.finish(StopReason::Decided, now)),
        ),
    }
}

/// 窓切替直後・長い無操作のあとのエピソードを見つけ、判定器を回して結果を返す追跡器（挙動を変えない観測用）。
///
/// 殻は取り込み口（フックの打鍵が届いた順）で `on_key` を呼ぶ。`scope` は前面窓を表す識別子
/// （hwnd と pid から作る。窓が変わったかの比較にだけ使う）。
/// 中身は位相（`TrackPhase`）と文脈を持つだけで、遷移は純粋関数 `track_step`。
#[derive(Debug)]
pub struct EpisodeTracker {
    cfg: TrackerConfig,
    ctx: TrackContext,
    phase: TrackPhase,
}

impl EpisodeTracker {
    #[must_use]
    pub const fn new(cfg: TrackerConfig) -> Self {
        Self {
            cfg,
            ctx: TrackContext {
                last_scope: None,
                last_key_ms: None,
            },
            phase: TrackPhase::Idle,
        }
    }

    /// 打鍵を 1 つ渡す。エピソードが閉じたら結果を返す（観測用。保持しない呼び出し向けの簡易版）。
    ///
    /// 開始は、窓切替または長い無操作のあとの**最初の Down**が文字キーか親指のとき（修飾なし・注入でない）。
    /// 矢印・Tab・Enter 等で始まる再開はエピソードにしない。
    pub fn on_key(
        &mut self,
        model: &KanaModel,
        key: ObservedKey,
        scope: u64,
    ) -> Option<EpisodeOutcome> {
        let step = self.on_key_step(model, key, scope);
        step.closed_prev.or(step.closed_now)
    }

    /// [`Self::on_key`] の詳細版。閉じたエピソードがこの打鍵に属するか（`in_episode`）を返す。
    pub fn on_key_step(&mut self, model: &KanaModel, key: ObservedKey, scope: u64) -> KeyStep {
        let phase = std::mem::replace(&mut self.phase, TrackPhase::Idle);
        let (phase, ctx, step) = track_step(&self.cfg, self.ctx, phase, model, key, scope);
        self.phase = phase;
        self.ctx = ctx;
        step
    }

    /// 今の位相の種類
    #[must_use]
    pub const fn phase_kind(&self) -> PhaseKind {
        self.phase.kind()
    }

    /// 進行中のエピソードがあるか
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.phase, TrackPhase::Observing { .. })
    }

    /// 進行中のエピソードが始まった前面窓の識別子
    #[must_use]
    pub const fn active_scope(&self) -> Option<u64> {
        match &self.phase {
            TrackPhase::Observing { active } => Some(active.scope),
            _ => None,
        }
    }

    /// 進行中のエピソードの期限の時刻（開始 + `max_ms`）
    #[must_use]
    pub const fn active_deadline_ms(&self) -> Option<u64> {
        match &self.phase {
            TrackPhase::Observing { active } => {
                Some(active.started_ms.saturating_add(self.cfg.max_ms))
            }
            _ => None,
        }
    }

    /// 進行中のエピソードを、期限（タイマー）などの外部の理由で閉じる。
    pub fn force_close(&mut self, stop: StopReason, now_ms: u64) -> Option<EpisodeOutcome> {
        match std::mem::replace(&mut self.phase, TrackPhase::Idle) {
            TrackPhase::Observing { active } => Some(active.finish(stop, now_ms)),
            other => {
                self.phase = other;
                None
            }
        }
    }
}

/// [`HoldController::on_key`]・[`HoldController::on_timer`] が返す命令（defunctionalization）。
///
/// 殻は返された**列を先頭から順に**実行する。実行の順序は列の並びが表す（殻のコードの順序に暗黙に頼らない）。
/// 並びは常に次の順で、該当する命令だけが出る:
///
/// 1. [`Self::DiscardHeld`]（保持列を捨てた）
/// 2. [`Self::WriteHiragana`]／[`Self::EnterHalfWidthAlnum`]（IME への書き込み。**解放の前**）
/// 3. [`Self::Record`]（閉じたエピソードの記録。古いものが先。書き込みの結果を記録に使うので、書き込みの後）
/// 4. [`Self::Replay`]（保持していた打鍵の解放。書き込みの**後**に順序どおりに流す）
/// 5. [`Self::ArmDeadline`]／[`Self::DisarmDeadline`]（期限のタイマー）
/// 6. [`Self::Pass`]／[`Self::Consume`]（この打鍵の行き先。`on_key` だけ）
#[derive(Debug, Clone, PartialEq)]
pub enum HoldCommand<T> {
    /// 保持列を捨てた（保持中に前面窓が変わった）。捨てた打鍵の数（1 以上）
    DiscardHeld(usize),
    /// **保持していたエピソード**で日本語と確定した。ひらがなキーを送る（`release` を流す**前**に、IME への書き込みの
    /// 対象にしてよい）。保持しなかったエピソード（観測だけ）では出ない（判定は `Record` の `verdict` で見る）
    WriteHiragana,
    /// **保持していたエピソード**で日本語でないと確定した（`NotJapanese`）。`ime_on_eisu` の半角英数への切替の対象に
    /// してよい。未決（`Undecided`）や打ち切りでは出ない
    EnterHalfWidthAlnum,
    /// 閉じたエピソードの結果を記録する
    Record(EpisodeOutcome),
    /// 保持していた打鍵（と、解放の契機になった打鍵）を、順序どおりに流す。空のときは出ない
    Replay(Vec<T>),
    /// このあとも保持が続き、**この打鍵で保持が始まった**: 期限のタイマーを起動する
    ArmDeadline,
    /// 保持が続いていない: 期限のタイマーを外す（同じ打鍵で始まって確定した場合を含む）
    DisarmDeadline,
    /// この打鍵は保持も解放もせず、通常どおり流す（エピソードに属さない、または保持しないエピソードの打鍵）
    Pass(T),
    /// この打鍵は元の経路へ流さない（保持列に入り、`Replay` で流れる、または `DiscardHeld` で捨てる）
    Consume,
}

/// 保持コントローラの位相。**保持列 `held` は `Holding` の中だけにある**（保持していないのに打鍵が溜まっている、
/// 保持中なのにエピソードが無い、を型で作れない）。追跡器の位相（`TrackPhase`）に `Holding` を足したもの。
#[derive(Debug)]
enum EpisodePhase<T> {
    Idle,
    Armed { trigger: Trigger, idle_ms: u64 },
    Observing { active: ActiveEpisode },
    Holding { active: ActiveEpisode, held: Vec<T> },
}

impl<T> EpisodePhase<T> {
    const fn kind(&self) -> PhaseKind {
        match self {
            Self::Idle => PhaseKind::Idle,
            Self::Armed { .. } => PhaseKind::Armed,
            Self::Observing { .. } => PhaseKind::Observing,
            Self::Holding { .. } => PhaseKind::Holding,
        }
    }

    /// 追跡器の位相と、保持列（`Holding` のときだけ `Some`）に分ける。
    fn split(self) -> (TrackPhase, Option<Vec<T>>) {
        match self {
            Self::Idle => (TrackPhase::Idle, None),
            Self::Armed { trigger, idle_ms } => (TrackPhase::Armed { trigger, idle_ms }, None),
            Self::Observing { active } => (TrackPhase::Observing { active }, None),
            Self::Holding { active, held } => (TrackPhase::Observing { active }, Some(held)),
        }
    }

    /// [`Self::split`] の逆。エピソードが閉じた位相（`Observing` でない）に保持列が残っていてはならない。
    fn join(track: TrackPhase, held: Option<Vec<T>>) -> Self {
        match (track, held) {
            (TrackPhase::Observing { active }, Some(held)) => Self::Holding { active, held },
            (TrackPhase::Observing { active }, None) => Self::Observing { active },
            (TrackPhase::Armed { trigger, idle_ms }, held) => {
                debug_assert!(held.is_none(), "武装中に保持列が残った");
                Self::Armed { trigger, idle_ms }
            }
            (TrackPhase::Idle, held) => {
                debug_assert!(held.is_none(), "エピソードが無いのに保持列が残った");
                Self::Idle
            }
        }
    }
}

/// エピソード先頭の打鍵を保持し、判定が出たら順序どおりに解放するコントローラ（ADR-263 D2）。
///
/// 保持するのは**エピソードの全打鍵**（Down も Up も、判定の材料でない打鍵も）。保持しないエピソード
/// （`may_hold = false`）でも判定と結果の記録は行う（観測）。窓が変わっていたら保持した打鍵は捨てる。
/// 結果は [`HoldCommand`] の列で返す。殻は `WriteHiragana` を実行してから `Replay` を流す。
///
/// 中身は位相（`EpisodePhase`）と文脈を持つだけの薄い殻で、遷移は新しい位相を返す純粋関数
/// （`hold_key_step`・`hold_timer_step`。Decider の `decide`/`evolve` に当たる）。
#[derive(Debug)]
pub struct HoldController<T> {
    cfg: TrackerConfig,
    ctx: TrackContext,
    phase: EpisodePhase<T>,
}

/// 1 ステップで起きたことの集計（命令の列を組み立てる前の途中結果）。
struct StepFacts<T> {
    pass: Option<T>,
    release: Vec<T>,
    discarded: usize,
    outcomes: Vec<EpisodeOutcome>,
    /// IME への書き込みの命令（`WriteHiragana`/`EnterHalfWidthAlnum`）
    write: Option<HoldCommand<T>>,
    /// 期限のタイマーの命令（`ArmDeadline`/`DisarmDeadline`）
    deadline: Option<HoldCommand<T>>,
}

impl<T> StepFacts<T> {
    const fn empty() -> Self {
        Self {
            pass: None,
            release: Vec::new(),
            discarded: 0,
            outcomes: Vec::new(),
            write: None,
            deadline: None,
        }
    }

    /// ステップの終わりに、保持が続くか・この打鍵で保持が始まったかからタイマーの命令を決める。
    fn set_deadline(&mut self, holding: bool, started_holding: bool) {
        self.deadline = if !holding {
            Some(HoldCommand::DisarmDeadline)
        } else if started_holding {
            Some(HoldCommand::ArmDeadline)
        } else {
            None
        };
    }

    /// 命令の列にする。順序は [`HoldCommand`] の doc のとおり。`consume` は `on_key` のときだけ真。
    fn into_commands(self, consume: bool) -> Vec<HoldCommand<T>> {
        let mut out = Vec::new();
        if self.discarded > 0 {
            out.push(HoldCommand::DiscardHeld(self.discarded));
        }
        out.extend(self.write);
        out.extend(self.outcomes.into_iter().map(HoldCommand::Record));
        if !self.release.is_empty() {
            out.push(HoldCommand::Replay(self.release));
        }
        out.extend(self.deadline);
        if let Some(event) = self.pass {
            out.push(HoldCommand::Pass(event));
        } else if consume {
            out.push(HoldCommand::Consume);
        }
        out
    }
}

/// 打鍵 1 つの遷移（純粋）。新しい位相・文脈と、殻が順に実行する命令の列を返す。
#[allow(clippy::too_many_arguments)] // 状態（cfg・文脈・位相）と入力（モデル・打鍵・事象・窓・保持の可否）を 1:1 で渡す
fn hold_key_step<T>(
    cfg: &TrackerConfig,
    ctx: TrackContext,
    phase: EpisodePhase<T>,
    model: &KanaModel,
    key: ObservedKey,
    event: T,
    scope: u64,
    may_hold: bool,
) -> (EpisodePhase<T>, TrackContext, Vec<HoldCommand<T>>) {
    let (track, mut held) = phase.split();
    let was_active = matches!(track, TrackPhase::Observing { .. });
    let (track, ctx, step) = track_step(cfg, ctx, track, model, key, scope);
    let mut out = StepFacts::empty();
    let mut closed_prev = false;
    let mut started_holding = false;

    if let Some(prev) = step.closed_prev {
        closed_prev = true;
        let prev_held = held.take().unwrap_or_default();
        if prev.stop == StopReason::WindowChanged {
            out.discarded += prev_held.len();
        } else {
            out.release.extend(prev_held);
        }
        out.outcomes.push(prev);
    }

    if step.in_episode {
        // この打鍵でエピソードが始まった: 進行中のエピソードが無かった、またはいま古いものを閉じた。
        if !was_active || closed_prev {
            held = may_hold.then(Vec::new);
            started_holding = may_hold;
        }
        match held.as_mut() {
            Some(buffer) => buffer.push(event),
            None => out.pass = Some(event),
        }
        if let Some(now) = step.closed_now {
            let holding = held.is_some();
            if holding && matches!(now.verdict, Verdict::Japanese(_)) {
                out.write = Some(HoldCommand::WriteHiragana);
            } else if holding && now.verdict == Verdict::NotJapanese {
                out.write = Some(HoldCommand::EnterHalfWidthAlnum);
            }
            out.release.extend(held.take().unwrap_or_default());
            out.outcomes.push(now);
        }
    } else {
        out.pass = Some(event);
    }
    out.set_deadline(held.is_some(), started_holding);
    (
        EpisodePhase::join(track, held),
        ctx,
        out.into_commands(true),
    )
}

/// 期限のタイマーの遷移（純粋）。保持中のエピソードを閉じる。`scope` は今の前面窓の識別子。
fn hold_timer_step<T>(
    cfg: &TrackerConfig,
    ctx: TrackContext,
    phase: EpisodePhase<T>,
    scope: u64,
) -> (EpisodePhase<T>, TrackContext, Vec<HoldCommand<T>>) {
    let (track, held) = phase.split();
    let mut out = StepFacts::empty();
    let track = match track {
        TrackPhase::Observing { active } => {
            let deadline = active.started_ms.saturating_add(cfg.max_ms);
            let same_window = active.scope == scope;
            let stop = if same_window {
                StopReason::Timeout
            } else {
                StopReason::WindowChanged
            };
            let held = held.unwrap_or_default();
            if same_window {
                out.release.extend(held);
            } else {
                out.discarded += held.len();
            }
            out.outcomes.push(active.finish(stop, deadline));
            TrackPhase::Idle
        }
        other => other,
    };
    out.set_deadline(false, false);
    (
        EpisodePhase::join(track, None),
        ctx,
        out.into_commands(false),
    )
}

impl<T> HoldController<T> {
    #[must_use]
    pub const fn new(cfg: TrackerConfig) -> Self {
        Self {
            cfg,
            ctx: TrackContext {
                last_scope: None,
                last_key_ms: None,
            },
            phase: EpisodePhase::Idle,
        }
    }

    /// 保持中か
    #[must_use]
    pub const fn is_holding(&self) -> bool {
        matches!(self.phase, EpisodePhase::Holding { .. })
    }

    /// 保持している打鍵の数
    #[must_use]
    pub const fn held_len(&self) -> usize {
        match &self.phase {
            EpisodePhase::Holding { held, .. } => held.len(),
            _ => 0,
        }
    }

    /// 今の位相の種類
    #[must_use]
    pub const fn phase_kind(&self) -> PhaseKind {
        self.phase.kind()
    }

    /// 取り込み口の打鍵を 1 つ渡す。`may_hold` は、この打鍵で**エピソードが始まる場合に**保持するか
    /// （対象の窓・設定による。進行中のエピソードの保持は変わらない）。
    pub fn on_key(
        &mut self,
        model: &KanaModel,
        key: ObservedKey,
        event: T,
        scope: u64,
        may_hold: bool,
    ) -> Vec<HoldCommand<T>> {
        let phase = std::mem::replace(&mut self.phase, EpisodePhase::Idle);
        let (phase, ctx, commands) = hold_key_step(
            &self.cfg, self.ctx, phase, model, key, event, scope, may_hold,
        );
        self.phase = phase;
        self.ctx = ctx;
        commands
    }

    /// 期限のタイマーが発火した。保持中のエピソードを閉じる。`scope` は今の前面窓の識別子。
    ///
    /// 前面窓が保持の開始時と違えば（マウスで窓を変えたあと次の打鍵が来なかった場合）、保持した打鍵は**捨てる**
    /// （元の窓の打鍵が新しい窓へ出る誤送信を避ける）。同じなら解放する。
    pub fn on_timer(&mut self, scope: u64) -> Vec<HoldCommand<T>> {
        let phase = std::mem::replace(&mut self.phase, EpisodePhase::Idle);
        let (phase, ctx, commands) = hold_timer_step(&self.cfg, self.ctx, phase, scope);
        self.phase = phase;
        self.ctx = ctx;
        commands
    }
}

/// ひらがなキーを送ってよいかの判断に使う事実（殻が集める）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // 独立した前提条件を 1:1 で表している
pub struct WriteFacts {
    /// belief が IME ON
    pub belief_open: bool,
    /// 観測の無い窓（実測学習で降格された窓）
    pub unobserved_window: bool,
    /// NonText（タスクバー等）
    pub non_text: bool,
    /// 前面窓と awase のフォーカス追跡が同じプロセスを指している（Alt+Tab 直後は追跡が遅れる）
    pub focus_in_sync: bool,
    /// 利用者が直近に、これから書く向きと**逆**の明示操作をしている（その意図を尊重して書かない。
    /// ひらがなキー〔ON〕なら明示 OFF、IME OFF キーなら明示 ON）
    pub explicit_opposite_intent: bool,
    /// Alt/Win が押されている（Alt 中の合成かなキーは JIS かな入力への切替に化ける。BUG-008）
    pub modifier_held: bool,
}

/// 書き込み（ひらがなキー/IME OFF キー）を送れない共通の理由。判断の順序が優先順位。純粋。
/// `explicit_label` は逆向きの明示操作があるときの理由の名前。
const fn write_skip_reason(f: WriteFacts, explicit_label: &'static str) -> Option<&'static str> {
    if !f.belief_open {
        Some("skipped-belief-off")
    } else if !f.unobserved_window || f.non_text {
        Some("skipped-observed-window")
    } else if !f.focus_in_sync {
        Some("skipped-focus-lag")
    } else if f.explicit_opposite_intent {
        Some(explicit_label)
    } else if f.modifier_held {
        Some("skipped-modifier-held")
    } else {
        None
    }
}

/// ひらがなキー（日本語と判定したときの IME ON）を送れない理由（送ってよければ `None`）。純粋。
#[must_use]
pub const fn hiragana_write_skip_reason(f: WriteFacts) -> Option<&'static str> {
    write_skip_reason(f, "skipped-explicit-off")
}

/// 半角英数トグルへ入れない理由（`ime_on_eisu` で英数と判定したとき）（送ってよければ `None`）。純粋。
/// `belief_open` は『awase が IME ON とみなしている』こと（OFF とみなしているなら、入る意味が無い）。
#[must_use]
pub const fn eisu_write_skip_reason(f: WriteFacts) -> Option<&'static str> {
    write_skip_reason(f, "skipped-explicit-on")
}

#[cfg(test)]
mod model_check;

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
        // さ(1,4) は行頭にも「さ」の後にも表に無く、尤度が低い。en_threshold を低くして早く「日本語でない」になる。
        let mut d = EpisodeDetector::new(params(100.0, 5.0, 7), ThumbOverlap::Enabled, [false; 2]);
        let mut last = Verdict::Pending;
        for _ in 0..3 {
            last = d.observe(&model, DetectEvent::CharDown(pos(1, 4)));
            d.observe(&model, DetectEvent::CharUp(pos(1, 4)));
        }
        assert_eq!(last, Verdict::NotJapanese);
    }

    #[test]
    fn symbol_keys_are_neutral_evidence() {
        // 記号キー(0,0)は仮名に読めない。どちらの証拠にもならないので、何度押しても llr は動かず、
        // 英数とも日本語とも確定しない（以前は 1 打で約 -33 nats になり NotJapanese に確定した）。
        let model = tiny_model();
        let mut d = EpisodeDetector::new(params(4.0, 8.0, 7), ThumbOverlap::Enabled, [false; 2]);
        for _ in 0..6 {
            assert_eq!(
                d.observe(&model, DetectEvent::CharDown(pos(0, 0))),
                Verdict::Pending
            );
            d.observe(&model, DetectEvent::CharUp(pos(0, 0)));
        }
        assert!(d.llr().abs() < f64::EPSILON);
        assert_eq!(d.presses(), 6);
    }

    #[test]
    fn symbol_key_does_not_hide_the_kana_after_it() {
        // 「、」のような記号のあとの仮名は、行頭と同じに数える（直前の仮名の文脈は切れる）。
        let model = tiny_model();
        let mut with_symbol =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        with_symbol.observe(&model, DetectEvent::CharDown(pos(0, 0)));
        with_symbol.observe(&model, DetectEvent::CharUp(pos(0, 0)));
        with_symbol.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        let mut head =
            EpisodeDetector::new(params(100.0, 100.0, 7), ThumbOverlap::Enabled, [false; 2]);
        head.observe(&model, DetectEvent::CharDown(pos(1, 1)));
        assert!((with_symbol.llr() - head.llr()).abs() < 1e-12);
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

    // ---- HoldController ----

    /// 命令の列を、旧 `HoldStep` と同じ意味のフィールドに読み替えた見え方（既存の単体テスト用）。
    /// 命令の並びそのものの固定は `crates/awase-windows/tests/lang_detect_decision_exhaustive.rs` が持つ。
    struct View {
        pass: Option<u32>,
        release: Vec<u32>,
        discarded: usize,
        outcomes: Vec<EpisodeOutcome>,
        japanese: bool,
        english: bool,
        holding: bool,
        started_holding: bool,
    }

    fn view(cmds: Vec<HoldCommand<u32>>) -> View {
        let mut v = View {
            pass: None,
            release: Vec::new(),
            discarded: 0,
            outcomes: Vec::new(),
            japanese: false,
            english: false,
            // 保持が続くのは、DisarmDeadline が出ないとき（保持が続かないステップでは必ず出る）。
            holding: true,
            started_holding: false,
        };
        for cmd in cmds {
            match cmd {
                HoldCommand::DiscardHeld(n) => v.discarded = n,
                HoldCommand::WriteHiragana => v.japanese = true,
                HoldCommand::EnterHalfWidthAlnum => v.english = true,
                HoldCommand::Record(o) => v.outcomes.push(o),
                HoldCommand::Replay(keys) => v.release = keys,
                HoldCommand::ArmDeadline => v.started_holding = true,
                HoldCommand::DisarmDeadline => v.holding = false,
                HoldCommand::Pass(e) => v.pass = Some(e),
                HoldCommand::Consume => {}
            }
        }
        v
    }

    struct Ctl(HoldController<u32>);

    impl Ctl {
        fn new(cfg: TrackerConfig) -> Self {
            Self(HoldController::new(cfg))
        }

        fn on_key(
            &mut self,
            model: &KanaModel,
            key: ObservedKey,
            event: u32,
            scope: u64,
            may_hold: bool,
        ) -> View {
            view(self.0.on_key(model, key, event, scope, may_hold))
        }

        fn on_timer(&mut self, scope: u64) -> View {
            view(self.0.on_timer(scope))
        }

        fn is_holding(&self) -> bool {
            self.0.is_holding()
        }
    }

    fn hold_controller() -> Ctl {
        Ctl::new(tracker_cfg())
    }

    #[test]
    fn episode_keys_are_held_and_released_in_order_on_overlap() {
        let model = tiny_model();
        let mut c = hold_controller();
        let s1 = c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        assert!(s1.pass.is_none() && s1.release.is_empty());
        assert!(s1.holding && s1.started_holding);
        // 修飾キー自身のイベントも保持列に入る（順序を保つため）。エピソードには影響しない。
        let s2 = c.on_key(&model, key(KeyRole::Modifier, true, 15), 2, 1, true);
        assert!(s2.pass.is_none() && s2.holding && !s2.started_holding);
        // 文字がまだ押されたまま親指を押す = 重なり。
        let s3 = c.on_key(
            &model,
            key(KeyRole::Thumb(ThumbSide::Left), true, 20),
            3,
            1,
            true,
        );
        assert!(s3.japanese);
        assert_eq!(s3.release, vec![1, 2, 3]);
        assert!(!s3.holding && s3.pass.is_none());
        assert_eq!(s3.outcomes.len(), 1);
        assert!(!c.is_holding());
    }

    #[test]
    fn keys_outside_an_episode_pass_through() {
        let model = tiny_model();
        let mut c = hold_controller();
        // 武装された最初の Down が矢印（Other）なら、エピソードにせず通す。
        let s = c.on_key(&model, key(KeyRole::Other, true, 10), 7, 1, true);
        assert_eq!(s.pass, Some(7));
        assert!(s.release.is_empty() && !s.holding);
    }

    #[test]
    fn episode_without_may_hold_is_observed_but_never_held() {
        let model = tiny_model();
        let mut c = hold_controller();
        let s1 = c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, false);
        assert_eq!(s1.pass, Some(1));
        assert!(!s1.holding && !s1.started_holding);
        let s2 = c.on_key(
            &model,
            key(KeyRole::Thumb(ThumbSide::Left), true, 20),
            2,
            1,
            false,
        );
        assert_eq!(s2.pass, Some(2));
        assert!(s2.release.is_empty());
        assert!(
            !s2.japanese,
            "保持していないエピソードは書き込みの対象にしない"
        );
        assert_eq!(s2.outcomes.len(), 1);
        assert_eq!(
            s2.outcomes[0].verdict,
            Verdict::Japanese(JapaneseEvidence::ThumbOverlap),
            "判定は outcome に残る"
        );
    }

    #[test]
    fn window_change_discards_the_held_keys() {
        let model = tiny_model();
        let mut c = hold_controller();
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), false, 15), 2, 1, true);
        // 窓 2 の打鍵: 古いエピソードは WindowChanged で閉じ、保持した 2 打鍵は捨てる。新しいエピソードはこの打鍵で始まる。
        let s = c.on_key(&model, key(KeyRole::Char(pos(1, 2)), true, 20), 3, 2, true);
        assert_eq!(s.discarded, 2);
        assert!(s.release.is_empty());
        assert_eq!(s.outcomes[0].stop, StopReason::WindowChanged);
        assert!(
            s.holding && s.started_holding,
            "新しいエピソードで保持を始める"
        );
        assert!(s.pass.is_none());
    }

    #[test]
    fn non_material_key_releases_held_keys_with_the_trigger_last() {
        let model = tiny_model();
        let mut c = hold_controller();
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        let s = c.on_key(&model, key(KeyRole::Other, true, 20), 2, 1, true);
        assert_eq!(s.release, vec![1, 2], "契機の打鍵は保持列の最後");
        assert!(!s.japanese && !s.holding);
        assert_eq!(s.outcomes[0].stop, StopReason::NonMaterialKey);
    }

    #[test]
    fn not_japanese_verdict_releases_without_the_write_flag() {
        let model = tiny_model();
        let mut cfg = tracker_cfg();
        cfg.params = params(100.0, 5.0, 7);
        let mut c = Ctl::new(cfg);
        // さ(1,4) は行頭の尤度が低く、1 打鍵目で NotJapanese になる。
        let s = c.on_key(&model, key(KeyRole::Char(pos(1, 4)), true, 10), 1, 1, true);
        assert_eq!(s.release, vec![1]);
        assert!(!s.japanese);
        assert_eq!(s.outcomes[0].verdict, Verdict::NotJapanese);
    }

    #[test]
    fn timer_releases_the_held_keys() {
        let model = tiny_model();
        let mut c = hold_controller();
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), false, 15), 2, 1, true);
        let s = c.on_timer(1);
        assert_eq!(s.release, vec![1, 2]);
        assert!(!s.holding && !s.japanese);
        assert_eq!(s.outcomes[0].stop, StopReason::Timeout);
        // タイマーが二重に発火しても何も起きない。
        assert!(c.on_timer(1).release.is_empty());
    }

    #[test]
    fn stale_timeout_on_the_next_key_releases_and_starts_a_new_episode() {
        let model = tiny_model();
        let mut c = hold_controller();
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        // タイマーが間に合わず、期限後に同じ窓の次の打鍵が来た: 古い保持を解放（捨てない）して、新しい判定へ。
        let s = c.on_key(
            &model,
            key(KeyRole::Char(pos(1, 2)), true, 3_000),
            2,
            1,
            true,
        );
        assert_eq!(s.release, vec![1]);
        assert_eq!(s.discarded, 0);
        assert_eq!(s.outcomes[0].stop, StopReason::Timeout);
    }

    #[test]
    fn ime_off_write_uses_the_same_gate_with_its_own_explicit_label() {
        assert_eq!(eisu_write_skip_reason(all_clear()), None);
        let opposite = WriteFacts {
            explicit_opposite_intent: true,
            ..all_clear()
        };
        assert_eq!(
            eisu_write_skip_reason(opposite),
            Some("skipped-explicit-on"),
            "IME OFF の書き込みで見送る逆向きの明示操作は、明示 ON"
        );
        assert_eq!(
            hiragana_write_skip_reason(opposite),
            Some("skipped-explicit-off")
        );
        let modifier = WriteFacts {
            modifier_held: true,
            ..all_clear()
        };
        assert_eq!(
            eisu_write_skip_reason(modifier),
            Some("skipped-modifier-held")
        );
    }

    #[test]
    fn english_flag_is_set_only_for_a_held_not_japanese_episode() {
        let model = tiny_model();
        let mut cfg = tracker_cfg();
        cfg.params = params(100.0, 5.0, 7);
        // 保持するエピソードで NotJapanese に確定: english = true。
        let mut c = Ctl::new(cfg);
        let s = c.on_key(&model, key(KeyRole::Char(pos(1, 4)), true, 10), 1, 1, true);
        assert!(s.english && !s.japanese);
        // 保持しないエピソード（観測だけ）では立たない。
        let mut c = Ctl::new(cfg);
        let s = c.on_key(&model, key(KeyRole::Char(pos(1, 4)), true, 10), 1, 1, false);
        assert!(!s.english);
        // 未決（max_keys）や打ち切りでは立たない。
        let mut c = Ctl::new(tracker_cfg());
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        let s = c.on_key(&model, key(KeyRole::Other, true, 20), 2, 1, true);
        assert!(!s.english && !s.japanese);
    }

    #[test]
    fn timer_after_the_window_changed_discards_the_held_keys() {
        let model = tiny_model();
        let mut c = hold_controller();
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), true, 10), 1, 1, true);
        c.on_key(&model, key(KeyRole::Char(pos(1, 1)), false, 15), 2, 1, true);
        // マウスで窓を変えた（scope 2）あと、次の打鍵が来ないまま期限のタイマーが発火した。
        let s = c.on_timer(2);
        assert!(s.release.is_empty());
        assert_eq!(s.discarded, 2);
        assert_eq!(s.outcomes[0].stop, StopReason::WindowChanged);
        assert!(!s.holding);
    }

    fn all_clear() -> WriteFacts {
        WriteFacts {
            belief_open: true,
            unobserved_window: true,
            non_text: false,
            focus_in_sync: true,
            explicit_opposite_intent: false,
            modifier_held: false,
        }
    }

    #[test]
    fn hiragana_is_written_only_when_every_precondition_holds() {
        assert_eq!(hiragana_write_skip_reason(all_clear()), None);
    }

    #[test]
    fn hiragana_write_is_skipped_with_the_first_failing_reason() {
        let cases = [
            (
                WriteFacts {
                    belief_open: false,
                    ..all_clear()
                },
                "skipped-belief-off",
            ),
            (
                WriteFacts {
                    unobserved_window: false,
                    ..all_clear()
                },
                "skipped-observed-window",
            ),
            (
                WriteFacts {
                    non_text: true,
                    ..all_clear()
                },
                "skipped-observed-window",
            ),
            (
                WriteFacts {
                    focus_in_sync: false,
                    ..all_clear()
                },
                "skipped-focus-lag",
            ),
            (
                WriteFacts {
                    explicit_opposite_intent: true,
                    ..all_clear()
                },
                "skipped-explicit-off",
            ),
            (
                WriteFacts {
                    modifier_held: true,
                    ..all_clear()
                },
                "skipped-modifier-held",
            ),
        ];
        for (facts, reason) in cases {
            assert_eq!(hiragana_write_skip_reason(facts), Some(reason), "{facts:?}");
        }
        // 複数が同時に偽でも、belief OFF が最優先。
        let many = WriteFacts {
            belief_open: false,
            modifier_held: true,
            explicit_opposite_intent: true,
            ..all_clear()
        };
        assert_eq!(hiragana_write_skip_reason(many), Some("skipped-belief-off"));
    }

    // ---- 実データ（data/ngram_hiragana.csv.gz と layout/nicola.yab）での回帰 ----

    fn real_model() -> KanaModel {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ngram = NgramModel::from_file(&root.join("data/ngram_hiragana.csv.gz"), 0, 0, 0)
            .expect("ngram file");
        let yab = std::fs::read_to_string(root.join("layout/nicola.yab")).expect("yab");
        // 本番（bootstrap）と同じく、読み込み直後に仮名を解決する。これを忘れると全キーが「仮名に読めない」扱いになる。
        let layout = YabLayout::parse(&yab, KeyboardModel::Jis)
            .expect("layout")
            .resolve_kana();
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
    fn real_model_reads_kana_keys_and_treats_symbols_as_neutral() {
        // w=か は仮名に読める（増分が 0 でない）。q=。 と数字キーは仮名に読めず中立（0）。
        let model = real_model();
        assert!(model.delta(None, qwerty('w')).abs() > 0.1);
        assert!(model.delta(None, qwerty('q')).abs() < f64::EPSILON);
        assert!(model.delta(None, qwerty('1')).abs() < f64::EPSILON);
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
