//! IME 変換モードの管理コンポーネント。
//!
//! `Charset` と `ConvMode` の定義は platform 非依存の `nicola` クレートに移動済み。
//! このファイルは `ConvModeMgr`（状態管理ラッパー）と `ConvModeAuthority`（所有権管理）を定義する。

#[cfg(windows)]
pub(crate) use awase::engine::Charset;
pub(crate) use awase::engine::ConvMode;

use super::TickMs;

// ─── 変換モード所有権 ────────────────────────────────────────────────────────────

/// IME 変換モードに対する awase の所有権状態。
///
/// awase エンジン ON/OFF・warmup 開始/終了など、conv mode 制御権の移譲時に更新される。
/// `executor` が `EngineStateChanged` で `WindowsPlatform::set_conv_mode_authority` を呼び、
/// `allows_conv_mutation()` の bool を conv mutation ゲートの唯一の実体である
/// `Output::conv_mutation_allowed`（Cell<bool>）へ push する。この enum 自体は状態を保持せず、
/// bool を導出するための分類器として使われる。
///
/// | 状態               | 説明                                                              |
/// |--------------------|-------------------------------------------------------------------|
/// | `Unknown`          | 初期状態。まだ所有権が確定していない。                            |
/// | `AwaseOwned`       | awase エンジン ON 中。conv mode を RomajiHiragana に lock する。  |
/// | `UserOwned`        | awase エンジン OFF / 非活性中。conv mode に一切触らない。         |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConvModeAuthority {
    /// 初期状態。所有権が誰にあるか不明（起動直後、エンジン状態未受信）。
    #[default]
    Unknown,
    /// awase エンジン ON 中。VK_DBE_HIRAGANA 等の conv mutation を許可する。
    AwaseOwned,
    /// awase 無効・エンジン OFF。IME conv mode に一切触らない。
    UserOwned,
    // 旧 TemporarilyUnowned（warmup 中の一時的な制御権返上）は 2026-07-06 の
    // 到達不能パス監査で撤去 — 構築サイトが一度も配線されなかった。
    // 必要になったら set_conv_mode_authority の呼び出し元とセットで再導入すること。
}

impl ConvModeAuthority {
    /// conv mode を変更する操作（VK_DBE_HIRAGANA 等・ImmSetConversionStatus）を許可するか。
    ///
    /// `AwaseOwned` のときのみ true。
    #[must_use]
    pub const fn allows_conv_mutation(self) -> bool {
        matches!(self, Self::AwaseOwned)
    }
}

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
    /// HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する期限（`current_tick_ms` 値）。
    ///
    /// 0 = 抑制なし。以下の契機で更新される:
    ///
    /// - フォーカス変更後 1500ms: TsfNative でフォーカス直後に IMM conv が TSF を反映しない
    /// - HanKata warmup (F1+F3) 送信後 500ms: TsfNative では F3 が IMM conv の FULLSHAPE ビットを
    ///   変更しないため、F1+F3 後の IMM 読み取りが ZenKata (0x0B) を返す副作用を遮断する
    #[cfg(windows)]
    suppress_zenkata_until_ms: std::cell::Cell<u64>,
    /// 非カタカナ → カタカナ (Zenkaku/Hankaku) への遷移候補。2 回連続で同じ値を観測する
    /// まで `mode` を確定させない（BUG-19 の一発誤読ロック対策、下記 `update_from_conv` 参照）。
    katakana_candidate: std::cell::Cell<Option<ConvMode>>,
    /// 直近に real IME への復元書き込み（`set_ime_romaji_mode_with_target_async` 等）を
    /// 行った時点の `mode`。`needs_conv_restore_write` / `mark_conv_restore_written` 参照
    /// （ADR-078 Phase 1a: cold warmup 増幅ループ対策）。
    restore_written_for: std::cell::Cell<Option<ConvMode>>,
}

impl Default for ConvModeMgr {
    fn default() -> Self {
        Self {
            mode: std::cell::Cell::new(None),
            #[cfg(windows)]
            suppress_zenkata_until_ms: std::cell::Cell::new(0),
            katakana_candidate: std::cell::Cell::new(None),
            restore_written_for: std::cell::Cell::new(None),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code, unused_variables))]
impl ConvModeMgr {
    /// フォーカスウィンドウが変わったことを通知する。
    ///
    /// 以後 1500ms 以内の HankakuKatakana → ZenkakuKatakana ダウングレードを抑制する。
    /// TsfNative ウィンドウはフォーカス直後に IMM conv が TSF mode を反映しないため。
    ///
    /// `tick_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    #[cfg(windows)]
    pub(crate) fn on_focus_changed(&self, tick_ms: TickMs) {
        let until = tick_ms.0 + 1500;
        if until > self.suppress_zenkata_until_ms.get() {
            self.suppress_zenkata_until_ms.set(until);
        }
    }

    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力し `true` を返す。
    /// HankakuKatakana → ZenkakuKatakana のダウングレードは `suppress_zenkata_until_ms` 期限内
    /// であれば無視する（フォーカス後 1500ms または HanKata warmup 後 500ms）。
    ///
    /// 非カタカナ → カタカナ (Zenkaku/Hankaku) への遷移は、既存の確定モードがある場合に限り
    /// 2 回連続で同じ値を観測するまで確定させない（BUG-19: `GetForegroundWindow` 基準の
    /// conv 読み取りが、候補ウィンドウ等フォーカスが一瞬他ウィンドウに移った際に一発だけ
    /// 誤ったカタカナ conv を返すことがある。この値をここで確定させてしまうと、
    /// warmup の先頭 VK 選択がカタカナ用キーを実送信し、誤読が GJI の実状態として
    /// 定着してしまう — 詳細は `docs/known-bugs.md` BUG-19 参照）。
    ///
    /// `now_ms`: 呼び出し元が取得した現在時刻（`GetTickCount64` 由来）。
    pub(crate) fn update_from_conv(&self, conv: u32, now_ms: TickMs) -> bool {
        let new = ConvMode::from_u32(conv);
        let old = self.mode.get();

        // 非カタカナ → カタカナ遷移候補の追跡。`new` が現在の確定モードと一致する
        // 場合（＝カタカナ候補と矛盾する読み取り）も含め、条件を満たさなければ
        // 必ず候補をクリアする。そうしないと、2回連続でなく「候補→矛盾する読み取り
        // →候補と同じ値」という間隔の空いた一致でも確定してしまう。
        let entering_katakana =
            new.charset.is_katakana() && old.is_some_and(|m| !m.charset.is_katakana());
        if entering_katakana {
            if self.katakana_candidate.get() != Some(new) {
                self.katakana_candidate.set(Some(new));
                log::debug!(
                    "[conv-mode] カタカナ遷移候補観測 (1回目、確定保留): \
                     {} → {} (conv=0x{conv:08X})",
                    old.map_or_else(|| "None".to_string(), |m| m.to_string()),
                    new,
                );
                return false;
            }
            // 2 回連続で同じカタカナ値を観測 — 確定へ進む。
        } else {
            self.katakana_candidate.set(None);
        }

        if old == Some(new) {
            return false;
        }

        #[cfg(windows)]
        if old.is_some_and(|m| m.charset == Charset::HankakuKatakana)
            && new.charset == Charset::ZenkakuKatakana
        {
            let now = now_ms.0;
            let until = self.suppress_zenkata_until_ms.get();
            if now < until {
                log::debug!(
                    "[conv-mode] HanKata→ZenKata ダウングレード抑制 \
                     (残り{}ms, conv=0x{conv:08X})",
                    until.saturating_sub(now)
                );
                return false;
            }
        }
        log::info!(
            "[conv-mode] {} → {} (conv=0x{conv:08X})",
            old.map_or_else(|| "None".to_string(), |m| m.to_string()),
            new,
        );
        self.mode.set(Some(new));
        self.katakana_candidate.set(None);
        true
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }

    /// warmup/送信ロジックが実際に追従すべき charset を返す。
    ///
    /// `tuning::DIAG_FORCE_HIRAGANA_CHARSET` が有効な間は、観測値に関わらず常に
    /// `Charset::Hiragana` を返す（カタカナ/英数への charset-aware warmup・VK
    /// 前置ロジックを実験的に無効化する診断フラグ）。`get()`/`update_from_conv()`
    /// 自体は影響を受けない — 観測・ログは通常通り継続され、この関数を経由する
    /// 「実際に charset に追従して動くかどうか」の消費側だけが無効化される。
    #[cfg(windows)]
    pub(crate) fn effective_charset(&self) -> Charset {
        if crate::tuning::DIAG_FORCE_HIRAGANA_CHARSET {
            return Charset::Hiragana;
        }
        self.get().map_or(Charset::Hiragana, |m| m.charset)
    }

    /// 現在の `mode` に対する real IME への復元書き込みがまだ済んでいなければ `true`。
    ///
    /// `mode` が変わらない限り、同じ belief 値に対する復元書き込みは 1 回だけに制限する
    /// （ADR-078 Phase 1a）。`cold_warmup.rs::preamble()` は cold warmup（＝
    /// `GjiFsm::FocusChange` のたびに再突入し得る）ごとに `conv_mode.get()` を real IME へ
    /// 書き戻していたため、一度誤って確定した belief がフォーカス往復のたびに
    /// 再アサートされ続けて自己増幅する経路になっていた（詳細は
    /// `docs/known-bugs.md` BUG-19、`docs/adr/078-*.md` 参照）。`mode` が本当に
    /// 変化した場合（新しい `update_from_conv` が新値を確定した場合）は、その新しい
    /// `mode` に対しては改めて `true` を返す — 復元自体を止めるのではなく、
    /// 「同じ belief への書き込みの反復」だけを止める。
    ///
    /// `mode` が `None`（まだ何も確定していない）のときは書き込む対象がないため `false`。
    pub(crate) fn needs_conv_restore_write(&self) -> bool {
        self.mode
            .get()
            .is_some_and(|current| self.restore_written_for.get() != Some(current))
    }

    /// 現在の `mode` に対する復元書き込みを実行したことを記録する。
    ///
    /// 呼び出し元が実際に `set_ime_romaji_mode_with_target_async` 等を発行した直後に呼ぶこと。
    pub(crate) fn mark_conv_restore_written(&self) {
        self.restore_written_for.set(self.mode.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 実機ログで観測された値。CONV_HIRAGANA: NATIVE|FULLSHAPE|ROMAN。
    // CONV_ZENKATA: NATIVE|KATAKANA|FULLSHAPE|ROMAN。
    const CONV_HIRAGANA: u32 = 0x0019;
    const CONV_ZENKATA: u32 = 0x001B;

    fn t(ms: u64) -> TickMs {
        TickMs(ms)
    }

    /// BUG-19: 一発だけのカタカナ観測は確定させない（`GetForegroundWindow` 基準の
    /// conv 読み取りが候補ウィンドウ等から誤ってカタカナ conv を拾うケースの再現）。
    #[test]
    fn single_spurious_katakana_reading_is_not_committed() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        assert_eq!(mgr.get().unwrap().charset.to_string(), "Hiragana");

        let changed = mgr.update_from_conv(CONV_ZENKATA, t(10));
        assert!(!changed, "1回目のカタカナ観測は確定してはいけない");
        assert_eq!(
            mgr.get().unwrap().charset.to_string(),
            "Hiragana",
            "1回目の観測では mode が書き換わってはいけない"
        );
    }

    /// 2回連続で同じカタカナ値を観測したら確定する。
    #[test]
    fn katakana_reading_confirmed_after_two_consecutive_observations() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10)));

        let changed = mgr.update_from_conv(CONV_ZENKATA, t(20));
        assert!(changed, "2回連続で一致したら確定するべき");
        assert_eq!(mgr.get().unwrap().charset.to_string(), "ZenKata");
    }

    /// 候補観測の直後に元の値へ戻る読み取りが入った場合、候補はクリアされ、
    /// その後に同じカタカナ値が来ても「1回目」として扱われる（間隔の空いた
    /// 一致で確定してしまわないことの回帰テスト）。
    #[test]
    fn intervening_reading_resets_katakana_candidate() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10)), "1回目: 保留");
        assert!(
            !mgr.update_from_conv(CONV_HIRAGANA, t(20)),
            "現状維持の再観測（変化なし）"
        );

        // 直前に矛盾する読み取り(Hiragana)があったため、これは改めて「1回目」。
        let changed = mgr.update_from_conv(CONV_ZENKATA, t(30));
        assert!(
            !changed,
            "間に矛盾する読み取りを挟んだ場合、確定までもう一度連続一致が必要"
        );
        assert_eq!(mgr.get().unwrap().charset.to_string(), "Hiragana");

        assert!(
            mgr.update_from_conv(CONV_ZENKATA, t(40)),
            "改めて2回連続で確定"
        );
        assert_eq!(mgr.get().unwrap().charset.to_string(), "ZenKata");
    }

    /// 初回観測（`old` が `None`）はデバウンス対象外 — 起動直後にカタカナの
    /// アプリへフォーカスした場合等、正当なケースを即座に反映する。
    #[test]
    fn first_ever_observation_is_not_debounced_even_if_katakana() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_ZENKATA, t(0)));
        assert_eq!(mgr.get().unwrap().charset.to_string(), "ZenKata");
    }

    /// カタカナ以外への遷移（英数化等）は従来通りデバウンスなしで即確定する。
    #[test]
    fn non_katakana_transitions_are_unaffected() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        // 半角英数 (conv=0)
        assert!(mgr.update_from_conv(0x0000, t(10)));
        assert_eq!(mgr.get().unwrap().charset.to_string(), "HanAlpha");
    }

    // ── needs_conv_restore_write / mark_conv_restore_written (ADR-078 Phase 1a) ──

    /// mode が未確定（`None`）のうちは復元書き込みの対象がない。
    #[test]
    fn restore_write_not_needed_before_any_mode_is_confirmed() {
        let mgr = ConvModeMgr::default();
        assert!(!mgr.needs_conv_restore_write());
    }

    /// mode が確定した直後は復元書き込みが必要。書き込み後は同じ mode に対しては
    /// 不要になる（cold warmup のたびに同じ belief を書き戻す増幅ループの対策）。
    #[test]
    fn restore_write_needed_once_then_suppressed_for_same_mode() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_ZENKATA, t(0)));
        assert!(
            mgr.needs_conv_restore_write(),
            "確定直後は復元書き込みが必要"
        );

        mgr.mark_conv_restore_written();
        assert!(
            !mgr.needs_conv_restore_write(),
            "書き込み済みマーク後は同じ mode に対して不要"
        );

        // 同じ mode を再観測（変化なし）しても、書き込み済みのままなので不要。
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10)));
        assert!(!mgr.needs_conv_restore_write());
    }

    /// mode が本当に別の値へ変化したら、その新しい mode に対しては改めて必要になる
    /// （復元自体を止めるのではなく、同じ belief への反復書き込みだけを止める）。
    #[test]
    fn restore_write_needed_again_after_mode_genuinely_changes() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        mgr.mark_conv_restore_written();
        assert!(!mgr.needs_conv_restore_write());

        // 半角英数へ変化（カタカナ以外へのデバウンスなし遷移）
        assert!(mgr.update_from_conv(0x0000, t(10)));
        assert!(
            mgr.needs_conv_restore_write(),
            "mode が変わったら新しい mode に対して改めて必要"
        );
    }

    /// カタカナ debounce の 1 回目（未確定）の間は、まだ古い mode のままなので
    /// 復元書き込みの要否はその古い mode の記録状態に従う（新しい候補値では判断しない）。
    #[test]
    fn restore_write_unaffected_by_pending_katakana_candidate() {
        let mgr = ConvModeMgr::default();
        assert!(mgr.update_from_conv(CONV_HIRAGANA, t(0)));
        mgr.mark_conv_restore_written();
        assert!(!mgr.needs_conv_restore_write());

        // 1回目のカタカナ観測（確定保留、mode はまだ Hiragana のまま）
        assert!(!mgr.update_from_conv(CONV_ZENKATA, t(10)));
        assert!(
            !mgr.needs_conv_restore_write(),
            "mode 自体はまだ Hiragana のまま書き込み済みなので不要"
        );

        // 2回目で確定 → 新しい mode (ZenKata) に対して改めて必要
        assert!(mgr.update_from_conv(CONV_ZENKATA, t(20)));
        assert!(mgr.needs_conv_restore_write());
    }
}
