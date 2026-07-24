//! literal 検出の共有コア ([`LiteralDetectCore`]) と warm パス用の薄いラッパー
//! ([`LiteralDetectFsm`])。
//!
//! ## literal 検出ロジックの単一所在地
//!
//! TSF 送信後に「composition が成立したか / リテラル化したか」を判定するロジックは、
//! 以前は本ファイルの [`LiteralDetectFsm`]（warm パス）と `gji_warmup_coro.rs` の
//! inline Phase 6（cold パス）に**重複**していた（ADR-053 は cold パスへの畳み込みを
//! 記載するが warm パスの実装が別に残り、判定コードが 2 箇所に分岐していた）。
//!
//! この重複を解消するため、判定タイミング・partial literal 判定・回収アクション生成を
//! [`LiteralDetectCore`] に集約した。cold パス（`GjiWarmupCoro` Phase 6）と warm パス
//! （`LiteralDetectFsm`）は同一の `LiteralDetectCore::poll` を呼ぶ。
//!
//! ## 使用場面
//!
//! - warm パス: [`LiteralDetectFsm`] を `pending_tsf` に install（TSF 送信後の composition 確認）
//! - cold パス: `GjiWarmupCoro` が coro 本体内で [`LiteralDetectCore`] を直接駆動
//!
//! ## 動作（`LiteralDetectCore::poll` / `LiteralDetectFsm::tick`）
//!
//! - 10ms 間隔の TIMER_TSF_PROBE ハンドラから駆動する。
//! - composition 確認 → `[ProbeAction::Done]`
//! - raw literal 疑い → `[ProbeAction::RawTsfLiteralRecovery { .. }, ProbeAction::Done]`
//!   （捨て駒キーには倒れない、2026-07-16 撤去。dispatcher が `consecutive_count()==0`
//!   のときだけ romaji 再送をスケジュールする）
//! - 判定待ち → `None`（`LiteralDetectFsm::tick` では `vec![]`、タイマー継続）

use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::TsfEnvSnapshot;
use crate::tsf::warmup::probe_fsm::{ProbeAction, ProbeObservations};

/// 部分リテラル検出時に、composition 破棄（ESC）の後に送るバックスペース数。
///
/// 部分リテラルの構造は「先頭 1 文字リテラル + 残りが 1 composition ユニット」。
/// 2026-07-10 以前は composition 側も「BS×1 で消せるはず」という推測に頼っていたが、
/// composition が実際に何文字分だったかは（candidate SHOW/HIDE や GJI I/O からは）
/// 分からないため、compose ユニットが 2 文字以上になるケースで消し過ぎ/消し残しが
/// 起きうる不安定さがあった。`VK_ESCAPE` は candidate 表示中の composition を
/// 文字数に関係なく 1 打鍵で確実に破棄できるため（`docs/windows-api-constraints.md`
/// 1-2 節で実機確認済み: 「VK_ESCAPE は composition をキャンセルして入力テキストが消える」）、
/// composition 側の推測を ESC に置き換え、BS はここに残る「先頭 literal プレフィックス」
/// の削除のみを担う。プレフィックスは経験的に 1 文字（cold→warm 遷移は通常 1 文字目の
/// 処理中に完了する）と仮定する。
/// `ze_bs_count`（= chars.len()）はローマ字文字数に等しいが、3 文字ローマ字（"ltu" など）
/// では ze_bs_count=3 となり literal プレフィックスの実数と食い違うため、部分リテラル
/// パスでは使わない。
pub(crate) const PARTIAL_LITERAL_BS: usize = 1;

/// IME セッション最初の1文字専用の per-VK confirm ループ（BUG-24 追補）が、
/// ある VK で `SuspectedLiteral`/`StaleConfirm` になったときの回収パラメータを返す純関数。
///
/// `is_stale`: `DetectionResult::StaleConfirm`（epoch fencing 判定）由来なら `true`、
/// `SuspectedLiteral`（deadline 到達）由来なら `false`。
/// `failed_idx`: 何番目 (0-based) の VK が失敗判定になったか。
/// 戻り値: `(backs, escape_composition)`。
///
/// ## VK_BACK には「literal の positive な証拠」を要求する（2026-07-23、実機バグ2件で確認）
///
/// `VK_ESCAPE` の破壊スコープは pending composition 内に閉じている（実機確認済み、
/// `docs/windows-api-constraints.md` §1-2）のに対し、`VK_BACK` は「カーソル直前の
/// 1文字を無条件に消す」命令であり、composition スコープ**外**の既に確定済みの
/// テキストにも届く唯一の不可逆操作である。したがって BS は「本当に literal が
/// 出た」という積極的な証拠がある場合にのみ送る:
///
/// - `is_stale`（`StaleConfirm`）: 「confirm の根拠が古い」ことの検出であり
///   「literal である」ことの証拠では**ない**。idx に関わらず BS は送らない
///   （`backs=0`）。実機で2件確認済み: `escape_composition=true` の場合、ESC が
///   pending composition を破棄した後の BS が消すべき literal を失い、直前の
///   別スコープの確定済み文字（「リーク」の「ク」、「cold 」の末尾スペース）を
///   誤って消していた（`docs/known-bugs.md` BUG-33 追補3・4参照）。
/// - `!is_stale && failed_idx > 0`（`SuspectedLiteral`、直前の VK は fresh に
///   confirmed 済み）: GJI が「いま」この語を処理している直接証拠がある以上、
///   その直後の1VKだけが本当に IME をバイパスすることは実質考えにくい。BS は
///   送らず ESC のみに倒す（`escape_composition=true`、直前 VK の pending
///   composition を破棄）。
/// - `!is_stale && failed_idx == 0`（この語で一度も confirm 済みの証拠がない
///   唯一のケース）: 従来どおり `backs=1`（composition 自体が存在しないため
///   `escape_composition=false`）。
///
/// `escape_composition` は `failed_idx > 0` のときのみ `true`（BS の有無に関わらず、
/// それより前の VK が個別に `CompositionConfirmed` 済み＝pending composition が
/// 実在するため）。
pub(crate) const fn per_vk_recovery_params(is_stale: bool, failed_idx: usize) -> (usize, bool) {
    let backs = if is_stale || failed_idx > 0 { 0 } else { 1 };
    (backs, failed_idx > 0)
}

/// `CompositionConfirmed` 時に「先頭文字がリテラル化した partial literal」かどうかを判定する純関数。
///
/// WezTerm (TSF mode) では HIMC=NULL のため IMM32 composition 文字列との文字照合が
/// 不可能。代わりに以下の条件がすべて揃った場合を partial literal と判断する:
///   - `nc_fired=false` : fresh F2 に WezTerm が NAMECHANGE で応答しなかった
///     → TSF context が cold のまま送信した可能性が高い
///   - `is_tsf_mode` : WezTerm 等の TSF 専用アプリ（HIMC 照合不可）
///   - romaji 2 文字以上 : 1 文字なら partial にならない
pub(crate) fn is_partial_literal(
    observations: ProbeObservations,
    romaji: &str,
    env: TsfEnvSnapshot,
) -> bool {
    !observations.nc_fired && env.is_tsf_mode && romaji.chars().count() >= 2
}

/// literal 回収用アクション列を生成する（cold/warm 共通）。
///
/// 常に `RawTsfLiteralRecovery + Done`（backspace のみ、捨て駒キーには頼らない）。
/// `RawTsfLiteralRecovery` の dispatcher が `consecutive_count()==0` のときだけ
/// romaji の再送を `RAW_TSF_LITERAL` 経由でスケジュールする（`output/mod.rs::
/// record_raw_tsf_literal` → 次イベントで `send_romaji_as_tsf` を通常の cold パス
/// として再実行）。cold パスは per-VK confirm がデフォルトのため、この再送は
/// 自然に per-VK として実行される — 1文字失敗した後の再送も per-VK のままにする、
/// という設計（ユーザー方針、2026-07-16。以前あった「TSF mode かつ consecutive==0
/// → SendRecoveryBs + StartSacrificialWarmup」分岐は撤去した。捨て駒キー
/// （VK_A+BS/VK_IME_OFF→ON）は cold-start の予防用途としても失敗リカバリ用途
/// としても、もう本経路からは発行されない）。
///
/// `escape_composition`: `true` の場合、dispatcher はバックスペースの前に `VK_ESCAPE` を送って
/// composition を確実に破棄する（partial literal 専用、[`PARTIAL_LITERAL_BS`] のドキュメント参照）。
pub(crate) fn emit_recovery_actions(
    cold_seq: u32,
    romaji: String,
    backs: usize,
    escape_composition: bool,
) -> Vec<ProbeAction> {
    vec![
        ProbeAction::RawTsfLiteralRecovery {
            cold_seq,
            backs,
            romaji,
            escape_composition,
        },
        ProbeAction::Done,
    ]
}

/// warm パス（`LiteralDetectFsm`）と cold パス（`GjiWarmupCoro` Phase 6）が共有する
/// literal 検出コア。
///
/// 検出タイミング（`LiteralDetector::check_now`）・partial literal 判定・回収アクション生成を
/// ここ 1 箇所に集約する。両パスは `poll` を 10ms ごとに呼ぶだけで、判定ロジックを重複させない。
pub(crate) struct LiteralDetectCore {
    /// ログ相関番号
    cold_seq: u32,
    /// 送信したローマ字（回収アクションのペイロード用）
    romaji: String,
    /// probe 中に観測した事実。部分リテラル判定に使用する。
    observations: ProbeObservations,
    /// composition 確認 / raw literal 検出器
    detector: LiteralDetector,
    /// LiteralDetect タイムアウト絶対時刻（ms）
    deadline_ms: u64,
    /// raw literal 検出時に送るバックスペース数
    ze_bs_count: usize,
    /// 構築時点の連続 raw-tsf-literal 回数（ログ用）。
    ///
    /// dispatcher（`probe_io.rs` の `RawTsfLiteralRecovery` ハンドラ）が
    /// `consecutive_count()==0` かどうかで再送 vs give-up を判定する。
    consecutive: u32,
    /// 候補ウィンドウ可視 veto の開始時刻（ms）。`None` は veto 未発動。
    ///
    /// `deadline_ms` 到達時点（`SuspectedLiteral`）で候補ウィンドウがまだ可視の場合、
    /// backspace を出さず hold する。可視である以上ほぼ確実に compose 成功しているため
    /// （BUG-27 追補5 と同型の regression を避ける）。[`GJI_CANDIDATE_VETO_CAP_MS`] を
    /// 超えても可視のまま確定しない異常系（候補ウィンドウの固着）に備え、hold には
    /// 上限を設ける。上限超過時も backspace はせず、無回収の `Done` で打ち切る。
    veto_started_at_ms: Option<u64>,
}

impl LiteralDetectCore {
    /// `LiteralDetectCore` を生成する。`detector` と `deadline_ms` は呼び出し側が用意する
    /// （cold パスは transmit 完了時、warm パスは送信直後に確定させる）。
    pub(crate) const fn new(
        cold_seq: u32,
        romaji: String,
        observations: ProbeObservations,
        detector: LiteralDetector,
        deadline_ms: u64,
        ze_bs_count: usize,
        consecutive: u32,
    ) -> Self {
        Self {
            cold_seq,
            romaji,
            observations,
            detector,
            deadline_ms,
            ze_bs_count,
            consecutive,
            veto_started_at_ms: None,
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// - `None` → まだ待機中（タイマー継続）
    /// - `Some([Done])` → composition 確認（タイマー停止）
    /// - `Some([RawTsfLiteralRecovery { .. }, Done])` → raw literal 検出（タイマー停止）
    pub(crate) fn poll(&mut self, env: TsfEnvSnapshot) -> Option<Vec<ProbeAction>> {
        use crate::tsf::probe::DetectionResult;

        // BUG-24 追補: このIMEセッション（打鍵開始〜候補ウィンドウHIDE）で既に
        // CompositionConfirmedを確認済みなら、literal-detect自体をスキップして
        // 即送信する。is_partial_literalが送信前の無関係な代理指標(nc_fired)
        // に頼っているため、cold直後は毎回誤検知しうる — セッション内2文字目以降は
        // 「今回のセッションで実際にcomposeが機能した」という直接の事実だけで
        // 十分と判断し、無駄な確認・訂正の反復を避ける（反応速度優先）。
        if crate::tsf::observer::literal_session_confirmed(self.cold_seq) {
            log::debug!(
                "[literal-detect] cold={} セッション確認済み → スキップ",
                self.cold_seq
            );
            return Some(vec![ProbeAction::Done]);
        }

        let detection = self.detector.check_now(self.deadline_ms)?;

        match detection {
            DetectionResult::CompositionConfirmed => {
                if is_partial_literal(self.observations, &self.romaji, env) {
                    // ze_bs_count (= chars.len()) は「全部リテラル」向けの値であり、
                    // 部分リテラルには使えない。composition 側は VK_ESCAPE で文字数に
                    // 関係なく確実に破棄し（dispatcher 側で実行）、BS は先頭 literal
                    // プレフィックス分（PARTIAL_LITERAL_BS）のみを担う。
                    // 例: "ltu" → 'l' リテラル + 'tu'→'と' composition
                    //     → ESC (composition 破棄) + BS×1 ('l' 削除) が正しい。
                    log::debug!(
                        "[literal-detect] cold={} partial literal (nc=false tsf romaji={:?} escape+backs={} consecutive={} real_gji_idle_ms={})",
                        self.cold_seq,
                        self.romaji,
                        PARTIAL_LITERAL_BS,
                        self.consecutive,
                        crate::tsf::observer::gji_idle_ms(),
                    );
                    crate::ime_diagnostic::log_composition_probe(self.cold_seq, "partial-literal");
                    return Some(self.recovery(PARTIAL_LITERAL_BS, true));
                }

                log::debug!(
                    "[literal-detect] cold={} composition confirmed real_gji_idle_ms={}",
                    self.cold_seq,
                    crate::tsf::observer::gji_idle_ms(),
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "confirmed");
                // BUG-27 追補4: consecutive_count リセットを dispatcher の
                // CompositionConfirmed ハンドラに一元化する（mark_literal_session_confirmed
                // の直接呼び出しをやめ、ProbeAction 経由にする）。
                Some(vec![
                    ProbeAction::CompositionConfirmed {
                        cold_seq: self.cold_seq,
                        mark_literal_session: true,
                    },
                    ProbeAction::Done,
                ])
            }
            DetectionResult::SuspectedLiteral => match self.veto_decision() {
                VetoDecision::Hold => {
                    log::debug!(
                        "[literal-detect] cold={} candidate window可視のため回収を保留 (real_gji_idle_ms={})",
                        self.cold_seq,
                        crate::tsf::observer::gji_idle_ms(),
                    );
                    None
                }
                VetoDecision::Expired => {
                    log::warn!(
                        "[literal-detect] cold={} candidate window可視のまま veto 上限 {}ms 超過 → 無回収で打ち切り",
                        self.cold_seq,
                        crate::tuning::GJI_CANDIDATE_VETO_CAP_MS,
                    );
                    crate::ime_diagnostic::log_composition_probe(self.cold_seq, "veto-expired");
                    Some(vec![ProbeAction::Done])
                }
                VetoDecision::NotApplicable => {
                    log::debug!(
                        "[literal-detect] cold={} suspected literal (backs={} consecutive={} real_gji_idle_ms={})",
                        self.cold_seq,
                        self.ze_bs_count,
                        self.consecutive,
                        crate::tsf::observer::gji_idle_ms(),
                    );
                    crate::ime_diagnostic::log_composition_probe(self.cold_seq, "suspected");
                    Some(self.recovery(self.ze_bs_count, false))
                }
            },
            DetectionResult::StaleConfirm => {
                // BUG-33 追補4: StaleConfirm は「confirm 根拠が古い」ことの検出で
                // あって「literal である」証拠ではない。backspace は送らず
                // （backs=0）、romaji 再送のみスケジュールする
                // （`per_vk_recovery_params` のドキュメント参照）。
                log::warn!(
                    "[literal-detect] cold={} stale confirm 検出 (romaji={:?}) → \
                     backspace は送らず romaji 再送のみ行う",
                    self.cold_seq,
                    self.romaji,
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "epoch-fence-stale");
                Some(self.recovery(0, false))
            }
        }
    }

    fn recovery(&mut self, backs: usize, escape_composition: bool) -> Vec<ProbeAction> {
        emit_recovery_actions(
            self.cold_seq,
            std::mem::take(&mut self.romaji),
            backs,
            escape_composition,
        )
    }

    /// `SuspectedLiteral`（deadline 到達）時点で、候補ウィンドウ可視性による
    /// backspace veto を適用すべきか判定する。
    ///
    /// veto 対象外（per-VK Chrome パス、または候補ウィンドウが可視でない）なら
    /// [`VetoDecision::NotApplicable`] を返し、呼び出し側は従来通り回収する。
    fn veto_decision(&mut self) -> VetoDecision {
        if !self.detector.veto_eligible() || !crate::tsf::observer::gji_candidate_visible_now() {
            self.veto_started_at_ms = None;
            return VetoDecision::NotApplicable;
        }
        let now = crate::hook::current_tick_ms();
        let started_at = *self.veto_started_at_ms.get_or_insert(now);
        if now < started_at.saturating_add(crate::tuning::GJI_CANDIDATE_VETO_CAP_MS) {
            VetoDecision::Hold
        } else {
            VetoDecision::Expired
        }
    }
}

/// [`LiteralDetectCore::veto_decision`] の判定結果。
#[derive(Debug, PartialEq, Eq)]
enum VetoDecision {
    /// veto 対象外（per-VK パス、または候補ウィンドウが可視でない）→ 通常の回収に進む。
    NotApplicable,
    /// 候補ウィンドウ可視 かつ 上限未到達 → backspace を出さず hold（ポーリング継続）。
    Hold,
    /// 候補ウィンドウ可視のまま上限到達 → backspace はせず無回収で打ち切る。
    Expired,
}

/// warm パスの post-transmit composition 確認 FSM。[`LiteralDetectCore`] の薄いラッパー。
///
/// 構築後は 10ms ごとに [`tick`](LiteralDetectFsm::tick) を呼ぶ。
/// `Done` を含む Vec が返ったらタイマーを停止する。
pub(crate) struct LiteralDetectFsm {
    core: LiteralDetectCore,
    /// RAII guard — drop で `OUTPUT_GATE.active=false`
    _guard: OutputActiveGuard,
}

impl LiteralDetectFsm {
    /// `LiteralDetectFsm` を生成する。
    ///
    /// `literal_detect_ms` はタイムアウト期間（ms）。`OutputActiveGuard::begin()` を内部で
    /// 呼び出し、`LiteralDetector::new(true)`（単語単位のバッチ確認のため veto 有効）と
    /// deadline（`current_tick_ms() + literal_detect_ms`）を確定して `LiteralDetectCore` を
    /// 組み立てる。
    ///
    /// `consecutive` は現在の連続 raw-tsf-literal 回数。0 かつ TSF mode のとき sacr warmup を起動する。
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        observations: ProbeObservations,
        ze_bs_count: usize,
        literal_detect_ms: u64,
        consecutive: u32,
    ) -> Self {
        let guard = OutputActiveGuard::begin();
        let detector = LiteralDetector::new(true);
        let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
        Self {
            core: LiteralDetectCore::new(
                cold_seq,
                romaji,
                observations,
                detector,
                deadline_ms,
                ze_bs_count,
                consecutive,
            ),
            _guard: guard,
        }
    }
}

impl crate::tsf::warmup::tickable_fsm::TickableFsm for LiteralDetectFsm {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.core.poll(env).unwrap_or_default()
    }

    fn cold_seq_hint(&self) -> u32 {
        self.core.cold_seq
    }
}

// ── テスト ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_vk_recovery_params_first_vk_suspected_literal_backs_1_no_escape() {
        assert_eq!(
            per_vk_recovery_params(false, 0),
            (1, false),
            "先頭 VK の SuspectedLiteral（この語で一度も confirm 済みの証拠がない \
             唯一のケース）は composition が存在しないため ESC 不要、literal の \
             positive な証拠として backs=1"
        );
    }

    // BUG-33 追補4: 2番目以降の VK の SuspectedLiteral は「直前の VK が fresh に
    // confirmed 済み」という正の証拠があるため、もはや backspace を送らない
    // （以前は backs=1 で送っていたが、実機で「cold 」の末尾スペースを誤って
    // 消す実害が確認された）。ESC のみで pending composition を破棄する。
    #[test]
    fn per_vk_recovery_params_later_vk_suspected_literal_escapes_without_backspace() {
        assert_eq!(
            per_vk_recovery_params(false, 1),
            (0, true),
            "2番目以降の VK の SuspectedLiteral は先行 VK による composition を \
             ESC で破棄するが、backspace は送らない"
        );
        assert_eq!(per_vk_recovery_params(false, 2), (0, true));
    }

    // BUG-33 追補4: StaleConfirm は「confirm 根拠が古い」ことの検出であって
    // 「literal である」証拠ではないため、idx に関わらず backspace を送らない。
    #[test]
    fn per_vk_recovery_params_stale_confirm_never_backspaces() {
        assert_eq!(
            per_vk_recovery_params(true, 0),
            (0, false),
            "先頭 VK の StaleConfirm は composition も存在しないため ESC も不要"
        );
        assert_eq!(
            per_vk_recovery_params(true, 1),
            (0, true),
            "2番目以降の VK の StaleConfirm は ESC のみ、backspace は送らない"
        );
    }

    fn obs(nc_fired: bool) -> ProbeObservations {
        ProbeObservations {
            nc_fired,
            ..Default::default()
        }
    }

    fn tsf_env() -> TsfEnvSnapshot {
        TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        }
    }

    // CompositionConfirmed が partial literal 条件を満たす場合 → RawTsfLiteralRecovery
    #[test]
    fn composition_confirmed_tsf_nc_false_multi_char_forces_recovery() {
        // 条件充足: nc=false, is_tsf_mode=true, romaji.chars()=2
        assert!(
            is_partial_literal(obs(false), "ni", tsf_env()),
            "部分リテラル条件がすべて揃っているべき"
        );
    }

    // nc_fired=true の場合は強制 recovery しない
    #[test]
    fn composition_confirmed_nc_fired_does_not_force_recovery() {
        assert!(
            !is_partial_literal(obs(true), "ni", tsf_env()),
            "nc_fired=true → 強制 recovery 不要"
        );
    }

    // 1 文字ローマ字は部分リテラルにならない
    #[test]
    fn composition_confirmed_single_char_romaji_no_recovery() {
        assert!(
            !is_partial_literal(obs(false), "n", tsf_env()),
            "1 文字ローマ字 → 部分リテラルにならない"
        );
    }

    // TSF モードでない場合は強制 recovery しない
    #[test]
    fn composition_confirmed_non_tsf_no_recovery() {
        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            ..Default::default()
        };
        assert!(
            !is_partial_literal(obs(false), "ni", env),
            "non-TSF mode → 強制 recovery 不要"
        );
    }

    // 3 文字ローマ字 (っ = "ltu") でも BS は 1 固定（composition 側は ESC が担当）
    #[test]
    fn partial_literal_bs_count_is_always_1_regardless_of_romaji_length() {
        // "ltu" → 'l' リテラル + 'tu'→'と' composition
        // → ESC (composition 破棄、文字数不問) + BS×1 ('l' 削除) が正しい。
        // BS×3 (= chars.len()) を送ると挿入点前の無関係な文字を消してしまう。
        assert!(
            is_partial_literal(obs(false), "ltu", tsf_env()),
            "ltu: 部分リテラル条件が揃っているべき"
        );
        assert_eq!(
            PARTIAL_LITERAL_BS, 1,
            "PARTIAL_LITERAL_BS は常に 1 (先頭 literal プレフィックスのみ、composition は ESC で破棄)"
        );
    }

    // partial literal 検出時、emit される recovery アクションが escape_composition=true を
    // 持つことを確認する（2026-07-10 追加: ESC-based composition 回収）。
    // 2026-07-16: 捨て駒キー撤去に伴い emit_recovery_actions は常に RawTsfLiteralRecovery を
    // 返すようになった（consecutive による分岐は dispatcher 側 `probe_io.rs` に一本化）。
    #[test]
    fn emit_recovery_actions_partial_literal_sets_escape_composition_true() {
        let actions = emit_recovery_actions(0, "ltu".to_string(), PARTIAL_LITERAL_BS, true);
        match &actions[0] {
            ProbeAction::RawTsfLiteralRecovery {
                escape_composition, ..
            } => assert!(
                *escape_composition,
                "partial literal 回収は escape_composition=true であるべき"
            ),
            other => panic!("expected RawTsfLiteralRecovery, got {other:?}"),
        }
    }

    // SuspectedLiteral（全部 literal 化）は escape_composition=false のままであるべき
    // （composition が存在しないため ESC は不要、既存の chars.len() ベース BS のみ）。
    #[test]
    fn emit_recovery_actions_suspected_literal_keeps_escape_composition_false() {
        let actions = emit_recovery_actions(0, "ko".to_string(), 2, false);
        match &actions[0] {
            ProbeAction::RawTsfLiteralRecovery {
                escape_composition, ..
            } => assert!(
                !*escape_composition,
                "SuspectedLiteral 回収は escape_composition=false であるべき"
            ),
            other => panic!("expected RawTsfLiteralRecovery, got {other:?}"),
        }
    }

    // ── veto: 候補ウィンドウ可視時の backspace 抑制 ─────────────────────────────
    //
    // 候補ウィンドウの SHOW/HIDE と GJI I/O は別々のセンサーであり、SuspectedLiteral
    // （deadline 到達）の瞬間に候補ウィンドウが可視なら、ほぼ確実に compose は成功して
    // いる（BUG-27 追補5 と同型の regression を避ける）。この veto の poll() 内での
    // 実装を検証する。

    use crate::tsf::observer::TSF_OBS;
    use std::sync::atomic::Ordering::SeqCst;

    /// `TSF_OBS` はプロセス全体のグローバル状態のため、テスト間の競合を防ぐロック
    /// (`observer.rs`/`probe.rs`と共有、`TSF_OBS_TEST_LOCK`参照)。
    use crate::tsf::observer::TSF_OBS_TEST_LOCK as VETO_TEST_LOCK;

    fn reset_tsf_obs_for_veto_test() {
        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        crate::tsf::observer::reset_literal_session_confirmed();
    }

    // SuspectedLiteral 到達時点で候補ウィンドウが可視なら、backspace を出さず
    // hold（None を返してポーリング継続）すべき。
    #[test]
    fn poll_vetoes_backspace_while_candidate_visible() {
        let _g = VETO_TEST_LOCK.lock().unwrap();
        reset_tsf_obs_for_veto_test();

        // 送信直前（可視になる前）に detector のベースラインを取る。veto_eligible=true
        // （単語単位のバッチ確認を模擬）。
        let detector = LiteralDetector::new(true);
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        let now_ms = crate::hook::current_tick_ms();
        let mut core =
            LiteralDetectCore::new(0, "ko".to_string(), obs(true), detector, now_ms, 2, 0);

        let result = core.poll(tsf_env());
        assert!(
            result.is_none(),
            "候補ウィンドウ可視時は backspace を出さず hold すべき: {result:?}"
        );
    }

    // hold が GJI_CANDIDATE_VETO_CAP_MS を超えても候補ウィンドウが可視のままなら、
    // backspace はせず無回収の Done で打ち切るべき（固着ウィンドウに対する安全弁）。
    #[test]
    fn poll_gives_up_without_backspace_after_veto_cap_expires() {
        let _g = VETO_TEST_LOCK.lock().unwrap();
        reset_tsf_obs_for_veto_test();

        let detector = LiteralDetector::new(true);
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        let now_ms = crate::hook::current_tick_ms();
        let mut core =
            LiteralDetectCore::new(0, "ko".to_string(), obs(true), detector, now_ms, 2, 0);

        // 1 回目: hold に入る（veto_started_at_ms が確定する）。
        assert!(core.poll(tsf_env()).is_none());

        // 上限を超えるまで実時間で待機する（候補ウィンドウ固着を模擬）。
        std::thread::sleep(std::time::Duration::from_millis(
            crate::tuning::GJI_CANDIDATE_VETO_CAP_MS + 50,
        ));

        let actions = core
            .poll(tsf_env())
            .expect("上限超過後は Some(..) で確定するべき");
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { .. })),
            "上限超過時も backspace（RawTsfLiteralRecovery）は出さないべき: {actions:?}"
        );
        assert!(
            actions.iter().any(|a| matches!(a, ProbeAction::Done)),
            "無回収で Done を返すべき: {actions:?}"
        );
    }

    // per-VK 単体確認（veto_eligible=false）では前モーラ由来の誤 veto を避けるため、
    // 候補ウィンドウが可視でも veto を適用せず従来通り backspace 回収するべき。
    #[test]
    fn poll_does_not_veto_on_per_vk_confirm_path() {
        let _g = VETO_TEST_LOCK.lock().unwrap();
        reset_tsf_obs_for_veto_test();

        TSF_OBS.gji_write_bytes.store(5_000, SeqCst);
        let detector = LiteralDetector::new_with_pre_send_baseline(5_000, false);
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        let now_ms = crate::hook::current_tick_ms();
        let mut core =
            LiteralDetectCore::new(0, "s".to_string(), obs(true), detector, now_ms, 1, 0);

        let actions = core
            .poll(tsf_env())
            .expect("per-VK パスは veto を無効化し即座に回収するべき");
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { .. })),
            "per-VK パスでは候補ウィンドウ可視でも backspace 回収すべき: {actions:?}"
        );
    }

    // ── ADR-079: epoch fencing（StaleConfirm）の回帰テスト ────────────────────

    /// confirm 根拠（write-bytes 閾値超過）が今回の送信より前の GJI I/O にしか
    /// 裏付けられていない場合、`LiteralDetectCore::poll` は `StaleConfirm` を
    /// 受けて `SuspectedLiteral` と同じ回収（backspace + romaji 再送）を行う
    /// ことを確認する。
    ///
    /// 当初は「検出のみ・recovery なし（ただの Done）」だったが、これは
    /// per-VK confirm の1文字目でこの経路が発火した場合に、まだ送信していない
    /// 後続 VK が一切送られないまま処理が終了する実害を実機で引き起こした
    /// （2026-07-22 実機報告: 「これでできる」→「kれでできる」、
    /// docs/known-bugs.md BUG-33 追補、`probe_fsm.rs` 側の同種修正コメント参照）。
    #[test]
    fn poll_recovers_like_suspected_literal_when_stale_confirm_detected() {
        let _g = VETO_TEST_LOCK.lock().unwrap();
        reset_tsf_obs_for_veto_test();

        TSF_OBS.gji_write_bytes.store(9_000, SeqCst);
        let detector = LiteralDetector::new_with_pre_send_baseline(9_000, true); // epoch > stale_write_ms
        // detector構築時刻(epoch_send_ms)より確実に前のwrite根拠を模擬する。
        // GetTickCount64は解像度が粗い(既定~15.6ms)ため、実時間sleep(5ms)で
        // 「epochより後」を作ろうとすると同一tickに丸められることがあり、
        // evidence_is_fresh(tieをfresh扱いする`>=`比較)が意図せずtrueになって
        // flakyに失敗する(2026-07-25、probe.rs/probe_fsm.rsの同種修正時に
        // このテストへの適用を見落としていたが、Windows実機の再実行で
        // 単独の真の失敗として顕在化し発見)。saturating_subで明示的に確実な
        // 過去時刻を作ることで解像度に依存しないようにする。
        let stale_write_ms = crate::hook::current_tick_ms().saturating_sub(50);
        TSF_OBS.gji_last_write_ms.store(stale_write_ms, SeqCst);
        TSF_OBS.gji_write_bytes.store(9_400, SeqCst); // 閾値超過だが根拠は stale

        let now_ms = crate::hook::current_tick_ms();
        let mut core =
            LiteralDetectCore::new(0, "fu".to_string(), obs(true), detector, now_ms, 2, 0);

        let actions = core
            .poll(tsf_env())
            .expect("stale confirm は即座に確定するはず");
        // BUG-33 追補4: romaji 再送はスケジュールするが、backspace は送らない
        // （StaleConfirm は literal の証拠ではないため）。
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { backs: 0, .. })),
            "stale confirm 検出時は romaji 再送はスケジュールするが backspace は \
             送らないはず（未送信 VK を残したまま無回収で終わると生文字が残存する \
             ため再送自体は必要）: {actions:?}"
        );

        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }
}
