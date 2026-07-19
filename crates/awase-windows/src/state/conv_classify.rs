//! idle-conv-check の conv ビット解釈を集約する純粋関数。
//!
//! `kp_stage_idle_conv_check` は IMM32 の変換モード (NATIVE/KATAKANA/ROMAN/FULLSHAPE)
//! を読み、belief の `input_mode` 更新と NICOLA engine の ON/OFF 同期を決めていた。
//! 従来この判断は手続きの中にインライン展開され、`handle_engine_set_open` が 5 箇所に
//! 散っていたため、ビット組合せの見落としバグ（ROMAN 見落とし `fc18cc7`、KATAKANA 喪失
//! `109b4c9`、HanKata→ZenKata 誤ダウングレード `1544d3f`、HanAlpha→Hiragana で Engine
//! OFF のまま `ea3da7f`）が繰り返し発生していた。
//!
//! この関数は分岐を 1 箇所に集約し、Win32 API・時刻取得・`with_app` 呼び出しを一切
//! 行わない純粋関数として全数テスト可能にする。時間依存のガード
//! (`should_run_idle_conv_check`) は呼び出し元で評価済みとする。

use awase::engine::{ConvMode, InputModeState};

/// engine ON 同期の理由。ログとテストの両方で「どの規則が発火したか」を固定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConvSyncReason {
    /// カタカナ (NATIVE+KATAKANA) を観測し shadow=OFF → engine ON 同期。
    KatakanaShadowOff,
    /// belief が romaji 不可→可 に回復 かつ shadow=ON → engine 再起動。
    RomajiRecovered,
    /// ひらがな/カタカナ (NATIVE) への切替を観測し shadow=OFF → engine ON 同期。
    NativeToggleShadowOff,
}

/// engine 同期アクション。従来 5 箇所に散っていた `handle_engine_set_open` 呼び出しを
/// 統一表現し、呼び出し元が 1 経路で dispatch できるようにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EngineSync {
    /// engine への働きかけなし。
    None,
    /// engine を ON にする (`handle_engine_set_open(true)`)。`RomajiRecovered` のみが
    /// この経路を使う: `effective_open` が既に true の状態での belief 再同期であり、
    /// shadow=OFF から新たに ON 意図を作り出すものではないため、ユーザー意図経路
    /// (`UserImeSetIntent{Command}`) の再利用を許容する。
    SetOpen(ConvSyncReason),
    /// `ObservedEisu` 観測 → engine OFF + DirectInput。conv の英数モードは IME-ON の
    /// 確証（conv=0x10 は ROMAN ビット付き半角英数）のため、`effective_open=true` の
    /// belief を直接注入して apply する。
    DirectInput,
    /// conv ビットが shadow=OFF 中に NATIVE/KATAKANA への切替を示した
    /// (`KatakanaShadowOff` / `NativeToggleShadowOff`)。
    ///
    /// かつては `SetOpen` として `handle_engine_set_open(true)` を直接呼び、
    /// `UserImeSetIntent{Command}` を偽装して `desired_open` を書き換えていた。
    /// これによりユーザーが明示的に IME OFF にした直後でも、engine が conv の
    /// 一発誤読（GJI 候補ポップアップへのフォーカス flicker 等）を理由に勝手に
    /// ON へ戻る再発バグを起こした（2026-07-08, BUG-19 再発）。
    ///
    /// この variant は engine を actuate せず、呼び出し元が
    /// `PlatformState::report_conv_open_inference()` 経由で `ObserverReported`
    /// として記録するだけにとどめる。`desired_open` は変更されないため、実際に
    /// 補正が必要かどうかの判断は既存の drift correction 経路
    /// (`check_drift_correction` / `ir_apply_drift_correction`、BUG-20 で OFF 方向も
    /// 修正済み) に委ねられる。
    ReportOpenInference(ConvSyncReason),
}

/// idle-conv-check の判断結果。input_mode belief の更新と engine 同期を分離して表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConvTransition {
    /// belief.input_mode の更新 (`None` = 変更なし / ダウングレード抑制)。
    /// `Some` の場合、呼び出し元が `InputModeObserved` を dispatch する。
    pub input_mode_update: Option<InputModeState>,
    /// engine ON/OFF 同期アクション。
    pub engine: EngineSync,
    /// JISかな化（ひらがな conv で ROMAN ビットが無い）を観測 → ローマ字入力を
    /// 復元すべきか。awase の engine は romaji VK を出力するため、engine が open の間に
    /// IME がかな入力になると出力が壊滅する（外部注入 VK_KANA によるかなロック
    /// トグルで実発生、BUG-08）。呼び出し元は conv 権限 (conv_mutation_allowed) と
    /// レート制限を確認した上で `set_ime_romaji_mode_with_target_async(None)` を送る。
    ///
    /// steady-state（conv 不変）でも true を返す: 当初は `conv_mode_changed` 遷移時のみ
    /// 発火させていたが、roma→kana の変化検出はフォーカス変更時の refresh 等
    /// **別経路の `update_from_conv` が先に消費する**ため、idle-conv-check から見ると
    /// 常に「変化なしの steady kana」になり一度も発火しなかった（2026-07-06 実機:
    /// Windows Terminal がセッション中ずっと conv=0x0009 のまま）。送信スパム防止は
    /// 純粋関数の責務ではなく、呼び出し元のレート制限（`kp_stage_idle_conv_check`）が担う。
    #[serde(default)]
    pub restore_roman: bool,
}

/// 現在確定している `ConvMode`・belief・engine 状態から idle-conv-check の同期判断を導く。
///
/// I/O・時刻取得・`with_app` 呼び出しを一切行わない純粋関数。
///
/// # 引数
/// - `cm`: 現在確定している `ConvMode`。呼び出し元は `ConvModeMgr::get()`
///   （= `ConvModeMgr::update_from_conv` 済みの値）を渡すこと。`ImmGetConversionStatus`
///   の生値を直接 `ConvMode::from_u32` してここに渡してはならない — `ConvModeMgr` は
///   非カタカナ→カタカナ遷移を2回連続観測するまで確定させないデバウンスを持つ（BUG-19）。
///   この関数が生値を直接受け取ると、`ConvModeMgr` 側（warmup のキー選択）だけが保護され、
///   ここ（belief 更新・engine 同期）は一発誤読に無防備なままになってしまう。
/// - `current`: 現在の `input_mode` belief。
/// - `is_cold`: `output_in_flight_ms() == u64::MAX`（ROMAN ビット未確定期間）。
/// - `effective_open`: 現在の engine open 状態 (`effective_open()`)。
/// - `conv_mode_changed`: `ConvModeMgr::update_from_conv` が変化を検出したか。
/// - `is_roman_reliable`: ROMAN ビット (0x10) が信頼できるか。TsfNative の idle 経路では
///   常に `false`。
#[must_use]
pub fn classify_conv_transition(
    cm: ConvMode,
    current: InputModeState,
    is_cold: bool,
    effective_open: bool,
    conv_mode_changed: bool,
    is_roman_reliable: bool,
) -> ConvTransition {
    let input_mode_update = cm.classify_idle(is_cold, current, is_roman_reliable);
    // NATIVE=0 ⟺ 英数モード (is_eisu)。KATAKANA は Charset で判定する。
    let has_native = !cm.is_eisu();
    let has_katakana = cm.charset.is_katakana();
    let was_romaji_capable = current.is_romaji_capable();

    // belief 変化なし (None) の場合:
    // - NATIVE(ひらがな/カタカナ)+shadow=OFF なら conv 不変・変化を問わず engine ON
    //   同期を試みる（BUG-26: かつて conv 不変の場合はカタカナのみを回復対象とし、
    //   非カタカナ NATIVE を無条件で無視していた。FocusChanged 直後の最初の
    //   idle-conv-check が「既に NATIVE」を steady-state として読む場合
    //   （ConvModeMgr が focus 変更前から同じ値を保持しており conv_mode_changed が
    //   一度も true にならない）、この経路だけが唯一の回復手段なのに永久に
    //   EngineSync::None を返し続け、shadow=OFF が実際の Hiragana conv と乖離した
    //   まま engine が romaji パススルーに固定される（2026-07-17, Windows Terminal /
    //   Windows.UI.Input.InputSite.WindowClass で実機再現、docs/known-bugs.md
    //   BUG-26 参照）。
    // - AssumedRomaji は常に classify_idle=None を返すため、この分岐が None
    //   ケースでの唯一の回復経路になる。
    //
    // belief を更新する (Some) 場合は、更新後の new_mode を見て engine を同期する。
    // 従来コードは複数の if を順に評価していたが、発火するアクションは互いに排他
    // （対象 open が衝突しない）なので単一アクションに集約できる。ObservedEisu
    // (NATIVE=0) は NativeToggle 系と、`!effective_open` を要求する分岐は
    // `effective_open` を要求する romaji 回復分岐と排他になる。
    let engine = input_mode_update.map_or(
        if has_native && !effective_open {
            let reason = if has_katakana {
                ConvSyncReason::KatakanaShadowOff
            } else {
                ConvSyncReason::NativeToggleShadowOff
            };
            EngineSync::ReportOpenInference(reason)
        } else {
            EngineSync::None
        },
        |new_mode| {
            if matches!(new_mode, InputModeState::ObservedEisu) {
                EngineSync::DirectInput
            } else if matches!(new_mode, InputModeState::ObservedRomaji)
                && has_katakana
                && !effective_open
            {
                EngineSync::ReportOpenInference(ConvSyncReason::KatakanaShadowOff)
            } else if !was_romaji_capable && new_mode.is_romaji_capable() && effective_open {
                EngineSync::SetOpen(ConvSyncReason::RomajiRecovered)
            } else if conv_mode_changed && has_native && !effective_open {
                EngineSync::ReportOpenInference(ConvSyncReason::NativeToggleShadowOff)
            } else {
                EngineSync::None
            }
        },
    );

    // JISかな化検出: ひらがな（NATIVE、非カタカナ）conv で ROMAN ビットが無い状態を
    // engine open 中に観測したらローマ字入力の復元を要求する。
    // - is_roman_reliable 必須: MS-IME × TsfNative では closed/idle 時の conv 読み取りが
    //   ROMAN ビットを落として報告する（偽陽性）。ここで復元書き込みをすると MS-IME が
    //   数秒で 0x09 に戻し、conv が 0x19⇄0x09 を往復して他の conv ベースルール
    //   （ObservedEisu / NativeToggleShadowOff）を誤発火させ、直接入力中に spurious な
    //   Engine ON + IME ON を引き起こした（2026-07-06T05:28 実機、BUG-08 追補2）。
    //   is_roman_reliable=false（TsfNative idle 経路）では発火しない。
    // - !is_cold 必須: コールドスタート期間の ROMAN ビットは未確定のため無視。
    // - steady-state でも要求する（変化検出は別経路が先に消費するため頼れない）。
    //   送信頻度の抑制は呼び出し元のレート制限が担う。
    let restore_roman = is_roman_reliable
        && !is_cold
        && has_native
        && !has_katakana
        && !cm.romaji
        && effective_open;

    ConvTransition {
        input_mode_update,
        engine,
        restore_roman,
    }
}

// ── ジャーナル・リプレイ回帰基盤（P1）───────────────────────────────────────────

/// 実機ジャーナル由来（または手作り）の `classify_conv_transition` 呼び出し1件を
/// 表す固定フィクスチャ。`tests/journals/*.json` に配列として保存し、
/// `tests/journal_replay.rs` が読み込んで再実行・照合する。
///
/// `journal.rs::JournalEntry::ConvClassifyCall` が実機ダンプで記録する
/// フィールドと同じ形（conv/current/is_cold/effective_open/conv_mode_changed/
/// is_roman_reliable → result）だが、こちらは往復可能な独立フォーマットとして
/// 定義する（`JournalEntry` 全体は `KeyEventSummary` に `&'static str` を含み
/// 単純には `Deserialize` できないため、リプレイ専用に切り出している）。
///
/// `conv` フィールドは実機で観測された生の `ImmGetConversionStatus` 値。
/// `classify_conv_transition` は `ConvMode` を受け取るため、リプレイ側
/// (`tests/journal_replay.rs`) が `ConvMode::from_u32(fixture.conv)` に変換してから
/// 呼び出す。このフィクスチャ基盤の目的は conv ビット解釈ロジック自体の回帰検出であり
/// （モジュール冒頭のコメント参照）、`ConvModeMgr` のデバウンス（BUG-19）とのやり取り
/// までは対象にしない — デバウンス自体の回帰は `state/conv_mode.rs` 側の単体テストが担う。
///
/// フィクスチャの追加手順は `docs/journal-replay-guide.md` を参照。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvClassifyFixture {
    /// 何が起きたバグ/シナリオの記録か（人間可読な短い説明）。
    pub name: String,
    /// 参考: 実機で発生した既知のバグの説明・関連コミット等（任意）。
    #[serde(default)]
    pub note: String,
    pub conv: u32,
    pub current: InputModeState,
    pub is_cold: bool,
    pub effective_open: bool,
    pub conv_mode_changed: bool,
    pub is_roman_reliable: bool,
    /// 期待される `ConvTransition`。実機ダンプをそのまま転記した直後はバグを
    /// 含む「実際の」出力になっていることがあるため、修正後は必ず「あるべき」
    /// 出力に手で書き換えてからコミットすること。
    pub expected: ConvTransition,
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase::engine::{AssumedReason, InputModeState};

    // ── conv ビット定数（IMM32 変換モード）─────────────────────────────────────
    const NATIVE: u32 = 0x0001;
    const KATAKANA: u32 = 0x0002;
    const FULLSHAPE: u32 = 0x0008;
    const ROMAN: u32 = 0x0010;

    // 代表的な conv 値
    const CONV_HANALPHA: u32 = 0x0000; // 半角英数 (全ビット 0)
    const CONV_EISU_ROMAN: u32 = ROMAN; // 0x0010: MS-IME 半角英数 (ROMAN 付き) fc18cc7
    const CONV_ZENALPHA: u32 = FULLSHAPE; // 0x0008: 全角英数
    const CONV_HIRAGANA: u32 = NATIVE | FULLSHAPE | ROMAN; // 0x0019: ひらがなローマ字
    const CONV_JISKANA: u32 = NATIVE | FULLSHAPE; // 0x0009: JISかな (ROMAN なし)
    const CONV_ZENKATA: u32 = NATIVE | KATAKANA | FULLSHAPE; // 0x000B: 全角カタカナ
    const CONV_HANKATA: u32 = NATIVE | KATAKANA; // 0x0003: 半角カタカナ 1544d3f

    fn assumed() -> InputModeState {
        InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        }
    }

    /// idle 経路のデフォルト引数で分類する（is_cold=false, is_roman_reliable=false）。
    /// テストの可読性のため raw conv (`u32`) を受け取り、ここで `ConvMode` に変換する
    /// （本番の呼び出し元は `ConvModeMgr::get()` のデバウンス済み値を渡す。BUG-19 参照）。
    fn classify(
        conv: u32,
        current: InputModeState,
        effective_open: bool,
        conv_mode_changed: bool,
    ) -> ConvTransition {
        classify_conv_transition(
            ConvMode::from_u32(conv),
            current,
            false,
            effective_open,
            conv_mode_changed,
            false,
        )
    }

    // ── 英数モード検出（ObservedEisu → DirectInput）──────────────────────────────

    #[test]
    fn hanalpha_detected_as_eisu_direct_input() {
        let t = classify(CONV_HANALPHA, assumed(), true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    /// fc18cc7 回帰: ROMAN ビット付き半角英数 (conv=0x0010) も英数モードとして扱う。
    #[test]
    fn eisu_with_roman_bit_0x10_is_still_eisu() {
        let t = classify(CONV_EISU_ROMAN, assumed(), true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    #[test]
    fn zenalpha_detected_as_eisu_direct_input() {
        let t = classify(CONV_ZENALPHA, assumed(), false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        // ObservedEisu は NATIVE=0 なので NativeToggle 系とは排他 → DirectInput のみ。
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    #[test]
    fn eisu_when_belief_already_eisu_no_input_mode_update_but_still_direct_input() {
        // classify_idle は既に ObservedEisu の場合 None を返すが、それは belief 変化なし
        // であって engine 同期の必要性とは別。conv 不変なら engine も触らない。
        let t = classify(CONV_HANALPHA, InputModeState::ObservedEisu, false, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    // ── カタカナ（NATIVE+KATAKANA）──────────────────────────────────────────────

    /// 1544d3f 回帰: 半角カタカナ (HanKata, conv=0x0003) を認識する。
    /// TsfNative では ROMAN=0 だが KATAKANA は romaji-capable 扱い → ObservedRomaji。
    #[test]
    fn hankata_from_non_romaji_recovers_to_observed_romaji() {
        let t = classify(CONV_HANKATA, InputModeState::ObservedKana, false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        // shadow=OFF + カタカナ検出 → ObserverReported として記録するだけ (engine は actuate しない)
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::KatakanaShadowOff)
        );
    }

    #[test]
    fn zenkata_shadow_off_engine_on() {
        let t = classify(CONV_ZENKATA, InputModeState::ObservedKana, false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::KatakanaShadowOff)
        );
    }

    #[test]
    fn zenkata_shadow_on_no_engine_change() {
        // 既に romaji_capable なら input_mode 変化なし、effective_open=true なら engine も不変。
        let t = classify(CONV_ZENKATA, assumed(), true, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    /// 0f75b5b 回帰: カタカナ + shadow=OFF + conv 不変でも engine を復帰させる唯一の経路。
    #[test]
    fn katakana_shadow_off_conv_unchanged_still_recovers_engine() {
        let t = classify(CONV_ZENKATA, assumed(), false, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::KatakanaShadowOff)
        );
    }

    // ── ひらがな（NATIVE, ROMAN 有無）──────────────────────────────────────────

    /// ひらがなローマ字 (ROMAN 付き, 0x19) は belief が非 romaji_capable のとき
    /// ObservedRomaji に訂正され、shadow=ON なら engine 再起動 (RomajiRecovered)。
    #[test]
    fn hiragana_roman_recovers_romaji_and_restarts_engine_when_shadow_on() {
        let t = classify(CONV_HIRAGANA, InputModeState::ObservedKana, true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::RomajiRecovered)
        );
    }

    /// 同上だが shadow=OFF: RomajiRecovered は effective_open を要求するため発火せず、
    /// 代わりに NativeToggle (NATIVE+shadow=OFF) で engine ON 同期する。
    #[test]
    fn hiragana_roman_recovers_romaji_and_syncs_engine_on_when_shadow_off() {
        let t = classify(CONV_HIRAGANA, InputModeState::ObservedKana, false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    /// ea3da7f 回帰: HanAlpha→Hiragana(ROMAN なし, JISかな conv) で belief が非
    /// romaji_capable のとき、TsfNative (is_roman_reliable=false) では ObservedKana への
    /// downgrade をせず AssumedRomaji { ImmBridgeBroken } に回復する。
    /// shadow=ON なら engine 再起動 (RomajiRecovered)。
    #[test]
    fn jiskana_recovers_to_assumed_romaji_and_restarts_engine_when_shadow_on() {
        let t = classify(CONV_JISKANA, InputModeState::ObservedKana, true, true);
        assert_eq!(
            t.input_mode_update,
            Some(InputModeState::AssumedRomaji {
                reason: AssumedReason::ImmBridgeBroken
            })
        );
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::RomajiRecovered)
        );
    }

    #[test]
    fn hiragana_belief_already_romaji_capable_no_change() {
        // AssumedRomaji は romaji_capable → classify_idle=None。
        // effective_open=true なら engine も不変。
        let t = classify(CONV_HIRAGANA, assumed(), true, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    #[test]
    fn hiragana_belief_romaji_capable_shadow_off_syncs_engine() {
        // input_mode 変化なし (None) だが conv 変化 + NATIVE + shadow=OFF → engine ON。
        let t = classify(CONV_HIRAGANA, assumed(), false, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    /// BUG-26 回帰: 上と同じ conv=0x19 (ひらがなローマ字, 非カタカナ) だが
    /// conv_mode_changed=false（steady-state — FocusChanged 直後の最初の
    /// idle-conv-check で ConvModeMgr が既にこの値を保持しており「変化」を
    /// 検出しない場合に相当）。かつては非カタカナ NATIVE は conv_mode_changed=true
    /// の場合のみ回復対象とされ、この steady-state ケースは無条件で
    /// EngineSync::None を返し続けていた。shadow=OFF が実際の Hiragana conv と
    /// 乖離したまま、engine が romaji パススルーに永久に固定される
    /// （Windows Terminal / InputSite.WindowClass で実機再現、docs/known-bugs.md
    /// BUG-26）。conv_mode_changed の有無に関わらず回復するのが正しい。
    #[test]
    fn hiragana_belief_romaji_capable_shadow_off_steady_state_still_syncs_engine() {
        let t = classify(CONV_HIRAGANA, InputModeState::ObservedRomaji, false, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    /// d41ba86 回帰: JISかな (ROMAN なし) でも NATIVE 切替として engine ON 同期する
    /// (is_roman_reliable=false のためひらがなへの downgrade はしない)。
    #[test]
    fn jiskana_native_toggle_shadow_off_syncs_engine() {
        let t = classify(CONV_JISKANA, assumed(), false, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::ReportOpenInference(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    // ── ダウングレード抑制 (ed862bb) ───────────────────────────────────────────

    /// ed862bb: TsfNative (is_roman_reliable=false) では AssumedRomaji → ObservedKana の
    /// downgrade を抑制する。classify_idle は None を返すため belief は維持される。
    #[test]
    fn tsf_native_suppresses_romaji_to_kana_downgrade() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_JISKANA),
            assumed(),
            false,
            true,
            false,
            false,
        );
        assert_eq!(t.input_mode_update, None);
    }

    /// 逆に is_roman_reliable=true（通常 IMM32）ではひらがな conv で ObservedKana に訂正する。
    #[test]
    fn roman_reliable_downgrades_to_kana() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_JISKANA),
            assumed(),
            false,
            false,
            true,
            true,
        );
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedKana));
    }

    // ── cold start ─────────────────────────────────────────────────────────────

    /// cold start 中 (is_cold=true) は ROMAN ビットが信頼できないため、ひらがなローマ字
    /// conv でも belief を変更しない（英数モードのみ確実に判定）。
    #[test]
    fn cold_start_hiragana_roman_no_input_mode_change() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_HIRAGANA),
            InputModeState::Unknown,
            true,
            false,
            false,
            false,
        );
        assert_eq!(t.input_mode_update, None);
    }

    #[test]
    fn cold_start_eisu_still_detected() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_HANALPHA),
            InputModeState::Unknown,
            true,
            false,
            true,
            false,
        );
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    // ── engine None（何も同期しない）ケース ─────────────────────────────────────

    #[test]
    fn no_conv_change_no_belief_change_is_noop() {
        let t = classify(CONV_HIRAGANA, assumed(), true, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    #[test]
    fn native_toggle_but_already_open_no_engine_change() {
        // conv 変化 + NATIVE だが effective_open=true → NativeToggle は !effective_open を要求 → None。
        let t = classify(CONV_JISKANA, assumed(), true, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    // ── 全数に近い網羅: 代表 conv × belief × (open, changed) の組合せ ───────────

    /// 主要 conv 値 × 代表 belief で panic せず一貫した結果を返すことを確認する
    /// スモークテスト（enum 化により全組合せが型上網羅されていることの担保）。
    #[test]
    fn smoke_all_major_conv_belief_combinations() {
        let convs = [
            CONV_HANALPHA,
            CONV_EISU_ROMAN,
            CONV_ZENALPHA,
            CONV_HIRAGANA,
            CONV_JISKANA,
            CONV_ZENKATA,
            CONV_HANKATA,
        ];
        let beliefs = [
            InputModeState::ObservedRomaji,
            InputModeState::ObservedKana,
            InputModeState::ObservedEisu,
            assumed(),
            InputModeState::Unknown,
        ];
        for &conv in &convs {
            for &belief in &beliefs {
                for &open in &[false, true] {
                    for &changed in &[false, true] {
                        let t = classify(conv, belief, open, changed);
                        // 英数モードは常に ObservedEisu → DirectInput（belief が既に Eisu の
                        // 場合を除く）を返すという不変条件。
                        if ConvMode::from_u32(conv).is_eisu() {
                            match t.input_mode_update {
                                Some(m) => {
                                    assert_eq!(m, InputModeState::ObservedEisu);
                                    assert_eq!(t.engine, EngineSync::DirectInput);
                                }
                                None => {
                                    // belief が既に ObservedEisu のケースのみ。
                                    assert_eq!(belief, InputModeState::ObservedEisu);
                                }
                            }
                        }
                        // SetOpen は RomajiRecovered 専用で、effective_open (engine 既に ON)
                        // の belief 再同期にのみ使う — shadow=OFF から新規に ON 意図を
                        // 作り出す KatakanaShadowOff/NativeToggleShadowOff はここには来ない
                        // (ReportOpenInference に分離済み、BUG-19 再発対策)。
                        if let EngineSync::SetOpen(reason) = t.engine {
                            assert_eq!(reason, ConvSyncReason::RomajiRecovered);
                            assert!(open);
                        }
                        // ReportOpenInference (KatakanaShadowOff/NativeToggleShadowOff) は
                        // !effective_open でのみ発火し、desired_open は変更しない
                        // (ObserverReported として記録するだけ)。
                        if let EngineSync::ReportOpenInference(reason) = t.engine {
                            assert!(matches!(
                                reason,
                                ConvSyncReason::KatakanaShadowOff
                                    | ConvSyncReason::NativeToggleShadowOff
                            ));
                            assert!(!open);
                        }
                        // restore_roman は「ひらがな・非カタカナ・ROMANなし・engine open」
                        // でのみ発火する不変条件（conv 変化の有無には依存しない）。
                        if t.restore_roman {
                            assert!(open);
                            assert_eq!(conv & ROMAN, 0);
                            assert!(!ConvMode::from_u32(conv).is_eisu());
                            assert!(!ConvMode::from_u32(conv).charset.is_katakana());
                        }
                    }
                }
            }
        }
    }

    // ── JISかな化 → ローマ字入力復元（restore_roman, BUG-08）───────────────────────
    //
    // 反転の記録（docs/experiments.md エントリ 03）:
    // 当初 is_roman_reliable=false（TsfNative idle）でも発火させたが、MS-IME × TsfNative
    // では ROMAN=0 が偽陽性（closed/idle 時に ROMAN を落として報告）であり、復元書き込みが
    // conv を 0x19⇄0x09 で往復させ、他の conv ベースルールを誤発火させて直接入力中の
    // spurious Engine/IME ON を引き起こした（2026-07-06T05:28 実機）。
    // 現仕様: is_roman_reliable=true の文脈でのみ発火する。

    /// ROMAN ビットが信頼できる文脈での JISかな検出 → 復元要求。
    #[test]
    fn jiskana_with_reliable_roman_requests_restore() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_JISKANA),
            InputModeState::ObservedRomaji,
            false, // is_cold
            true,  // effective_open
            true,  // conv_mode_changed
            true,  // is_roman_reliable
        );
        assert!(
            t.restore_roman,
            "reliable ROMAN + JISかな + engine open → 復元"
        );
    }

    /// steady-state（conv 変化なし）でも reliable なら復元を要求する
    /// （変化検出は別経路が先に消費するため頼れない。スパム防止は呼び出し元のレート制限）。
    #[test]
    fn jiskana_steady_state_with_reliable_roman_still_requests_restore() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_JISKANA),
            InputModeState::ObservedKana,
            false,
            true,
            false, // conv_mode_changed
            true,
        );
        assert!(t.restore_roman);
    }

    /// TsfNative idle 経路（is_roman_reliable=false）では**決して**発火しない。
    /// ROMAN=0 が偽陽性のため、復元書き込みは conv を荒らすだけで有害（BUG-08 追補2）。
    #[test]
    fn tsfnative_unreliable_roman_never_restores() {
        for changed in [false, true] {
            let t = classify(CONV_JISKANA, InputModeState::ObservedRomaji, true, changed);
            assert!(
                !t.restore_roman,
                "is_roman_reliable=false では復元しない (changed={changed})"
            );
        }
    }

    /// engine が閉じている（IME OFF 相当の belief）なら復元しない。
    #[test]
    fn jiskana_while_closed_does_not_restore() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_JISKANA),
            InputModeState::ObservedRomaji,
            false,
            false, // effective_open
            true,
            true,
        );
        assert!(!t.restore_roman);
    }

    /// コールドスタート期間（ROMAN ビット未確定）は復元しない。
    #[test]
    fn jiskana_cold_start_does_not_restore() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_JISKANA),
            InputModeState::ObservedRomaji,
            true, // is_cold
            true,
            true,
            true,
        );
        assert!(!t.restore_roman);
    }

    /// ROMAN 付きひらがな（正常状態）では発火しない。
    #[test]
    fn hiragana_with_roman_does_not_restore() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_HIRAGANA),
            InputModeState::ObservedRomaji,
            false,
            true,
            true,
            true,
        );
        assert!(!t.restore_roman);
    }

    /// カタカナ conv は restore_roman の対象外（imm_conv_target 系の warmup が担当）。
    #[test]
    fn katakana_without_roman_does_not_restore() {
        let t = classify_conv_transition(
            ConvMode::from_u32(CONV_ZENKATA),
            InputModeState::ObservedRomaji,
            false,
            true,
            true,
            true,
        );
        assert!(!t.restore_roman);
    }
}
