#[cfg(windows)]
mod app {
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use awase_keymap_learn::anomaly::AnomalyPolicy;
    use awase_keymap_learn::cost::CostModel;
    use awase_keymap_learn::exec::{Executor, ImeDriver, ReadPolicy, Stats};
    use awase_keymap_learn::graph::Prior;
    use awase_keymap_learn::judgement::{
        combine, judge_self_verification, ReconciliationSummary, ScoredVerification,
        TableJudgement, ACCURACY_THRESHOLD, DEGENERATION_THRESHOLD, MIN_PREDICTED_STEPS,
        SYSTEMATIC_MISMATCH_THRESHOLD,
    };
    use awase_keymap_learn::model::KeyId;
    use awase_keymap_learn::persist::{PersistedCell, PersistedTable};
    use awase_keymap_learn::remeasure::{
        reconcile_with_bundled, MismatchedTarget, RemeasureParams,
    };
    use awase_keymap_learn::rng::Rng;
    use awase_keymap_learn::sample_models::atok_like;
    use awase_keymap_learn::strategy::{run, Req, Strategy};
    use awase_keymap_learn::table::Table;
    use awase_keymap_learn::verify::{
        classify_robust, predict, score_walk, ScoreReport, WalkObs, DEFAULT_MIN_MINORITY,
    };
    use awase_keymap_learn_win::RealImeDriver;
    use awase_windows::state::ime_kind::TipIdentity;

    const KEYS: [u32; 14] = [
        0x1D, 0x1C, 0xF2, 0xF1, 0xF0, 0xF3, 0x19, 0x16, 0x1A, 0x1B, 0x0D, 0x20, 0x08, 0x41,
    ];

    /// ADR-195段階6: 何押下ごとに標準出力へ進捗行を書き出すか。毎回書くと
    /// 子プロセス側(awase-settings)のパース負荷・パイプI/Oが無駄に増えるため間引く。
    const PROGRESS_EVERY_N_PRESSES: u32 = 10;

    /// ADR196-T2「1e前半」(opus-adversarial-consult 2026-09-23 C-2): 縮退が激しい表では
    /// [`MIN_PREDICTED_STEPS`](予測できたステップ数)に固定回数の押下では届かないことがある
    /// (縮退率20%の上限いっぱいなら300回押しても予測は約240歩)ため、予測300歩に達する
    /// まで押下を続ける。この定数は「それでも届かない」場合の安全弁の上限(暫定値)。
    const VERIFICATION_WALK_MAX_STEPS: usize = 1500;

    /// `<config dir>/keymap-learn-table.json`のパス。`awase.exe`/`awase-settings.exe`と
    /// 同じ探索規則(`awase::paths::resolve_relative_to_exe`、exeの隣→開発ビルドの
    /// ワークスペースルート→CWD相対の順)でconfig.tomlを探し、その親ディレクトリへ書く
    /// (`crates/awase-windows/src/state/key_effect_runtime.rs::table_file_path`と
    /// 同じ規約)。config.tomlが見つからなければ`None`(書き込み先を決められない)。
    fn table_file_path() -> Option<PathBuf> {
        let config_path = awase::paths::resolve_relative_to_exe("config.toml");
        if !config_path.exists() {
            return None;
        }
        config_path
            .parent()
            .map(|dir| dir.join("keymap-learn-table.json"))
    }

    /// ADR196-T2「1e前半」・不採用/要確認時の退避先。決定1eは「不採用でも表ファイルに
    /// 書き出す」というが、`keymap-learn-table.json`(採用済みの表、段階4読み手が直接読む)へ
    /// 上書きすると、以前`Accepted`だった良い表が今回の失敗で失われる。ユーザー判断
    /// (2026-09-23、opus-adversarial-consultのC-9)により、`Accepted`以外はこの別
    /// ファイルへ書き、`keymap-learn-table.json`はそのまま残す。
    fn last_attempt_file_path() -> Option<PathBuf> {
        let config_path = awase::paths::resolve_relative_to_exe("config.toml");
        if !config_path.exists() {
            return None;
        }
        config_path
            .parent()
            .map(|dir| dir.join("keymap-learn-last-attempt.json"))
    }

    /// 巡回で得た表から、永続化するセル列を組み立てる(ADR-195段階1〜2の出力を
    /// 段階3の永続化フォーマットへ結合する、B1対応)。
    ///
    /// - B2対応: `key`は`Table`が内部で使う`KEYS`配列の**添字**ではなく、実際の
    ///   Windows VKコード(`KeyId(KEYS[idx] as u16)`)で書く。読み手
    ///   (`key_effect_runtime.rs::convert_cell`)は`TableKey::from_vk`で生VKとして
    ///   解釈するため、添字のまま書くと大半が「表に無いVK」として不採用になり、
    ///   偶然一致する添字(8→BS, 13→Enter等)は誤ったセルとして採用されてしまう。
    /// - M5対応: 訪問した(観測が1件以上ある)セルは、決定的と言えなくても
    ///   `prediction: None`で必ず1件書く。書き手が未測定セルを省略できると、
    ///   読み手側の縮退率チェック(`coverage_ratio`)の分母を書き手が恣意的に
    ///   操作でき、チェックの意味が無くなる。
    fn build_persisted_cells(table: &Table) -> Vec<PersistedCell> {
        table
            .cells()
            .map(|(&(status, key_idx), _)| PersistedCell {
                status,
                key: KeyId(KEYS[key_idx] as u16),
                prediction: predict(table, status, key_idx, DEFAULT_MIN_MINORITY),
            })
            .collect()
    }

    /// ADR-195段階2(round1 M-8、round3 m-3): 誤りに強い分類でも決定的と言えない
    /// セルが1つでもあれば、学習をもう一度実行する(呼び出し元がこの関数自体を
    /// 高々1回しか呼ばないため、やり直しは1回まで)。
    ///
    /// code-review指摘: `tour()`の再訪問条件は`table.count(status, key) < req.k`
    /// なので、非決定と判定されたセルは(その判定自体がmin_minority以上の観測を
    /// 前提とするため)既に元の`req.k`以上の観測数を持っている。同じ`req`のまま
    /// もう一度`run()`しても`need`が0のまま素通りし、観測が一切増えずに空振りする。
    /// 実際に観測を追加するため、非決定と判定されたセルの現在の観測数を上回るよう
    /// `k`を底上げしたリクエストで再実行する。
    fn retry_nondeterministic_cells_once<D: ImeDriver>(
        exec: &mut Executor<D>,
        strategy: Strategy,
        prior: &Prior,
        cost: &CostModel,
        suspects: &[usize],
        base_req: &Req,
        rng: &mut Rng,
    ) {
        // code-review指摘: ここでは「もう一度巡回すべきか」の判定だけが要る(RetryTrackerの
        // 状態は使い捨て、実際のやり直し回数の管理は行わない——このセッション全体で
        // やり直しは高々1回だけ)。decide_cell/RetryTrackerを使い捨てで呼ぶと、読み手に
        // 「複数回のやり直し管理をしている」と誤解させるため、declared_not_det()による
        // 直接判定に単純化した。
        let mut max_flagged_count = 0usize;
        for (&(status, key), obs) in exec.table.cells() {
            if classify_robust(&exec.table, status, key, DEFAULT_MIN_MINORITY).declared_not_det() {
                max_flagged_count = max_flagged_count.max(obs.len());
            }
        }
        if max_flagged_count == 0 {
            return;
        }
        eprintln!("非決定的なセルがあるため、学習をもう一度実行します(やり直しは1回まで)。");
        // code-review指摘(第三者の行単位差分スキャン): `k`は`run()`/`tour()`が
        // グラフ全セルへ一様に適用する単一のスカラーしきい値であり、非決定と判定
        // された一部のセルだけを狙い撃ちして再訪させる仕組みは無い。このため、
        // 1件でも非決定セルがあれば、既に十分な観測数を得ていたセルも含め
        // グラフ全体が新しいkまで再度巡回される(実機では巡回1周が数分単位)。
        // セル単位の狙い撃ち再訪を`tour()`に持たせるには戦略API自体の変更が
        // 要るため、今回はスコープ外とし、全体再巡回という単純だが確実な
        // 挙動のままにしている(スコープを絞る改善は将来課題)。
        let bumped_k = u32::try_from(max_flagged_count)
            .unwrap_or(u32::MAX)
            .saturating_add(2)
            .max(base_req.k);
        // code-review指摘: exec.stats.presses/elapsed_ms()は1回目のrun()からの累積値であり
        // リセットされない。base_reqのmax_presses/budget_msをそのまま使い回すと、1回目の
        // 実行で予算を(実機の異常再試行等で)使い切っていた場合、over()の最初のチェックで
        // 即座にtrueとなり、「もう一度実行します」とログに出すだけで実際には1回も
        // 押下せずに戻ってしまう。やり直しパスに、1回目とは独立した新しい予算を与える。
        let retry_req = Req {
            k: bumped_k,
            max_presses: exec.stats.presses.saturating_add(base_req.max_presses),
            budget_ms: exec.elapsed_ms() + base_req.budget_ms,
            ..*base_req
        };
        run(strategy, exec, prior, cost, suspects, &retry_req, rng);
    }

    /// ADR-195段階2: 学習に使っていない独立のランダムウォークで一段予測を採点する。
    /// 進捗sinkは学習の巡回にだけ意味があるので、ここでは無効化する(有効なままだと
    /// `recording=false`の間もpressごとに呼ばれ、cell数が増えないのにelapsed_msだけ
    /// 伸びる不審な進捗行が出る)。
    ///
    /// C-2対応: 固定回数ではなく、予測できたステップ数([`ScoreReport::predicted`])が
    /// [`MIN_PREDICTED_STEPS`]に達するまで押下を続ける。[`VERIFICATION_WALK_MAX_STEPS`]
    /// (押下の試行回数)に達しても届かなければ打ち切って返す(`judge_self_verification`が
    /// `InsufficientSamples`として不採用にする)。
    fn run_verification_walk<D: ImeDriver>(exec: &mut Executor<D>, rng: &mut Rng) -> ScoreReport {
        exec.set_progress_sink(|_, _| {});
        exec.set_recording(false);
        let mut walk = Vec::new();
        let mut attempts = 0usize;
        let report = loop {
            // opus-adversarial-consult round2 N3対応: セッション監視が既に
            // 失敗と判定していたら、採点にならない押下を続けない。
            if exec.driver.should_abort() {
                break score_walk(&exec.table, DEFAULT_MIN_MINORITY, &walk);
            }
            let key = rng.below(KEYS.len());
            attempts += 1;
            if let Some(info) = exec.press(key) {
                // round2 N1対応: 汚染された観測(外部からの書き込み・物理入力・
                // フォーカス喪失)は採点に使わない。
                if !info.contaminated {
                    walk.push(WalkObs {
                        status: info.before,
                        key,
                        outcome: info.outcome,
                    });
                }
            }
            let report = score_walk(&exec.table, DEFAULT_MIN_MINORITY, &walk);
            if report.predicted() >= MIN_PREDICTED_STEPS || attempts >= VERIFICATION_WALK_MAX_STEPS
            {
                break report;
            }
        };
        exec.set_recording(true);
        report
    }

    /// C-7対応: 検証ウォーク専用の乱数シードを実行のたびに変える(時刻由来)。学習本体の
    /// 乱数(固定シード195)と共有すると、記録した`seed`だけではウォークを再現できない。
    #[allow(clippy::cast_possible_truncation)]
    fn fresh_walk_seed() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    }

    /// ADR-195段階3〜4への結合(B1対応): 表を永続化フォーマットへ変換し、一時ファイル+
    /// renameで原子的に書き込む。指紋(ADR-195段階8)は、その計算方式自体がADR-196決定3で
    /// 再設計中のため、ここでは`None`のまま残す(ADR196-T5が実配線する)。
    ///
    /// C-4/C-9対応: `judgement`が`Accepted`なら本体(`keymap-learn-table.json`)へ、
    /// それ以外は[`last_attempt_file_path`]へ書く。
    ///
    /// 戻り値は(書き込もうとしたセル数, 書き込み結果)。
    fn persist_judged_table(
        cells: Vec<PersistedCell>,
        verification: ScoredVerification,
        judgement: TableJudgement,
    ) -> (usize, Result<(), String>) {
        let cell_count = cells.len();
        let persisted = PersistedTable::new(cells)
            .with_verification(verification)
            .with_judgement(judgement);
        // C-9: `Accepted`以外は本体を上書きせず退避ファイルへ書く。
        let path_resolver: fn() -> Option<PathBuf> = if judgement == TableJudgement::Accepted {
            table_file_path
        } else {
            last_attempt_file_path
        };
        let write_result = path_resolver()
            .ok_or_else(|| "config.tomlが見つからないため書き込み先を決められない".to_string())
            .and_then(|path| {
                persisted
                    .to_json()
                    .map_err(|e| format!("表のシリアライズに失敗: {e}"))
                    .map(|json| (path, json))
            })
            .and_then(|(path, json)| {
                awase::fs_atomic::write_atomic(&path, json.as_bytes())
                    .map_err(|e| format!("{}への書き込みに失敗: {e:#}", path.display()))
            });
        (cell_count, write_result)
    }

    /// `run_main`のうち、セッション監視が失敗と判定していないかを確認する
    /// 部分（学習フェーズ直後・検証ウォーク直後の2箇所から呼ぶ、round2 N1
    /// 対応で複製されていたブロックの共通化）。失敗していたら専用result行を
    /// 出して`std::process::exit(1)`で終了する（round3 R3対応: 失敗時に
    /// 終了コードを非0にする。result行を必ずflushしてから終了すること）。
    /// 失敗していなければ何もせず戻る。
    fn exit_if_session_failed(
        executor: &Executor<RealImeDriver>,
        strategy: Strategy,
        training_elapsed_ms: f64,
        training_presses: u32,
        total_cells: u32,
        decode_errors: u32,
    ) {
        if !executor.driver.session_failed() {
            return;
        }
        print_interference_failure_line(InterferenceFailureArgs {
            strategy,
            training_elapsed_ms,
            training_presses,
            covered1: executor.table.covered1(),
            total_cells,
            decode_errors,
            contaminated_trials: executor.stats.contaminated_trials,
            invalidated_trials: executor.driver.session_invalidated_trials(),
        });
        std::process::exit(1);
    }

    /// C-1/A-6/B-3: 学習・検証ウォーク完了後の、表を書かずに失敗とすべき理由
    /// (フック断絶・学習中のIME切り替え・GJI設定変更)。`None`なら継続してよい。
    fn end_of_session_abort_reason(
        driver: &RealImeDriver,
        tip_at_start: TipIdentity,
        config1_db_at_start: Option<&[u8]>,
    ) -> Option<&'static str> {
        if !driver.hook_alive() {
            return Some("hook_lost");
        }
        if driver.query_tip_identity() != Some(tip_at_start) {
            return Some("ime_unidentified_or_switched");
        }
        let config1_db_at_end = (tip_at_start == TipIdentity::Gji)
            .then(awase_windows::gji_charset_autodetect::read_config1_db)
            .flatten();
        (config1_db_at_start != config1_db_at_end.as_deref()).then_some("gji_config_changed")
    }

    /// ADR196-T2「1e前半」(C-1/A-6/B-3): `reason`があれば「何を測ったか確定できない」
    /// セッション失敗として専用result行を出し、`exit_if_session_failed`と同じく
    /// `std::process::exit(1)`で終了する(表は書かない、決定1b項目5)。
    fn exit_if_skipped(
        reason: Option<&'static str>,
        executor: &Executor<RealImeDriver>,
        strategy: Strategy,
        training_elapsed_ms: f64,
        training_presses: u32,
        total_cells: u32,
        decode_errors: u32,
    ) {
        let Some(reason) = reason else {
            return;
        };
        eprintln!("学習セッションを失敗として終了しました(reason={reason}): 表は書き出しません");
        println!(
            "result status=failure strategy={} elapsed_ms={:.0} presses={} cells={} total={} \
             decode_errors={} reason={}",
            strategy.name(),
            training_elapsed_ms,
            training_presses,
            executor.table.covered1(),
            total_cells,
            decode_errors,
            reason,
        );
        let _ = std::io::stdout().flush();
        std::process::exit(1);
    }

    /// round2 N2対応: `RealImeDriver::new()`はquiet window判定(外部からの
    /// 書き込み・物理入力・フォーカス喪失、round1 M1/M3対応で発火条件が
    /// 広がった)や、それ以外の初期化失敗(COM初期化・TSF起動・窓作成・
    /// フック登録等)で`Err`を返すことがある。以前は呼び出し元が`?`でそのまま
    /// プロセスの異常終了に委ねていたため、result行が出ず、awase-settings側の
    /// 較正パネルには「結果を送らずに終了した」としか表示されなかった。他の
    /// 失敗経路と同じresult行の形式で理由を伝えた上で、`std::process::exit(1)`
    /// で終了する(round3 R3対応)。round3 R2対応:
    /// `RealImeDriver::is_quiet_window_error`で原因を区別し、result行の
    /// `reason`をquiet window判定によるものとそれ以外とで出し分ける。
    fn build_driver(strategy: Strategy) -> RealImeDriver {
        match RealImeDriver::new(KEYS.to_vec()) {
            Ok(driver) => driver,
            Err(err) => {
                let total_cells_estimate = atok_like().states.len() as u32 * KEYS.len() as u32;
                let reason = if RealImeDriver::is_quiet_window_error(&err) {
                    "quiet_window"
                } else {
                    "init"
                };
                print_driver_init_failure_line(strategy, total_cells_estimate, &err, reason);
                std::process::exit(1);
            }
        }
    }

    /// 進捗(現在何セル目/推定残り時間)を標準出力へ運ぶsinkを作る(ADR-195段階6)。
    /// awase-settings(較正ウィザード)はこの行をパースしてUI表示する。IPCは
    /// 使わない(ペイロードが1ワード固定で表本体を運べないため、詳細はADR本文
    /// 「段階6」節参照)。表本体はここでは一切標準出力へ出さない。
    fn make_progress_sink(total_cells: u32) -> impl FnMut(&Stats, &Table) {
        move |stats, table| {
            if stats.presses % PROGRESS_EVERY_N_PRESSES != 0 {
                return;
            }
            let cell = table.covered1() as u32;
            let elapsed_ms = stats.timeline.last().map_or(0.0, |&(ms, _, _)| ms);
            // 経過時間からの単純な線形外挿。0除算・未進捗時はeta不明(-1)を返す。
            let eta_ms = if cell == 0 || cell >= total_cells {
                -1.0
            } else {
                elapsed_ms / f64::from(cell) * f64::from(total_cells - cell)
            };
            println!(
                "progress cell={cell} total={total_cells} elapsed_ms={elapsed_ms:.0} eta_ms={eta_ms:.0}"
            );
            let _ = std::io::stdout().flush();
        }
    }

    /// 学習の初期仮説モデル(ATOK風モデルの抽象modeを実機のConv値へ対応づけ、
    /// 開始状態を実機の`initial`へ合わせる)。
    fn build_model(
        initial: awase_keymap_learn::model::Status,
    ) -> awase_keymap_learn::model::Machine {
        let mut model = atok_like();
        for state in &mut model.states {
            // ヒューリスティックな初期仮説として、抽象mode 0/1を実機のConv値0x09/0x00へ対応づける。
            state.status.mode = if state.status.mode == 0 { 0x09 } else { 0x00 };
        }
        if let Some(index) = model
            .states
            .iter()
            .position(|state| state.status == initial)
        {
            model.initial = index;
        }

        model
    }

    /// 決定1a: Microsoft IME本体なら既定で要確認。決定1b項目7〜8: 既知構成で内蔵表との
    /// 再測定後の突き合わせ結果(`reconciliation`)があれば`judgement::combine`で合成する
    /// (系統的不一致なら`Accepted`を要確認へ下げる、これは系統的バグへの安全弁であって
    /// 「内蔵表が正しい」という前提ではない)。
    fn judge_score(
        score: &ScoreReport,
        tip: TipIdentity,
        reconciliation: Option<&ReconciliationSummary>,
    ) -> TableJudgement {
        combine(
            judge_self_verification(
                score,
                tip == TipIdentity::MsImeNative,
                ACCURACY_THRESHOLD,
                DEGENERATION_THRESHOLD,
                MIN_PREDICTED_STEPS,
            ),
            reconciliation,
            SYSTEMATIC_MISMATCH_THRESHOLD,
        )
    }

    /// 再測定1セルあたりの、目的のstatusへ到達しようとして押してよいセットアップ押下数の
    /// 上限(暫定値、実機での到達所要押下数の実測待ち)。
    const REMEASURE_MAX_SETUP_PRESSES: usize = 60;
    /// セットアップ押下がこの回数続けて目的のstatusに出会えなければリセットして歩き直す。
    const REMEASURE_RESET_EVERY: usize = 12;

    /// ADR196-T2決定1b項目7〜9: 既知構成なら、学習表を内蔵表と突き合わせ、食い違ったセルを
    /// (学習本体とは別のセットアップ経路で)再測定する。再現しなかった(確認できなかった
    /// ものを含む)セルは`cells`の`prediction`を`None`へ落とす。既知構成でない・`config1.db`が
    /// 読めない場合は`None`(突き合わせ自体を行わない)。
    ///
    /// 項目9のうち「不一致の分布タグ」は未実装(残作業)。
    fn reconcile_against_bundled(
        executor: &mut Executor<RealImeDriver>,
        tip: TipIdentity,
        cells: &mut [PersistedCell],
        rng: &mut Rng,
    ) -> Option<ReconciliationSummary> {
        use awase_windows::gji_charset_autodetect::{
            bundled_preset_for_adjudication, BundledPresetLookup,
        };
        let preset = match bundled_preset_for_adjudication(tip) {
            BundledPresetLookup::Known(preset) => preset,
            BundledPresetLookup::NotKnown => return None,
            BundledPresetLookup::ConfigUnreadable => {
                eprintln!("警告: config1.dbを読めないため内蔵表との突き合わせをスキップ。");
                return None;
            }
        };
        let diff = awase_windows::state::key_effect_runtime::diff_against_bundled(cells, preset);
        let mut only_in_one_table = diff.only_in_one_table;
        let mut targets = Vec::new();
        for m in &diff.mismatched {
            let learned = cells
                .iter()
                .find(|c| c.status == m.status && c.key == m.key)
                .and_then(|c| c.prediction);
            let key = KEYS.iter().position(|&vk| vk == u32::from(m.key.0));
            match (learned, key) {
                (Some(learned), Some(key)) => targets.push(MismatchedTarget {
                    status: m.status,
                    key,
                    learned,
                }),
                // 学習値が無い/KEYSに無いキーは再測定できない(分母に含めず「片側のみ」扱い)。
                _ => only_in_one_table += 1,
            }
        }
        let params = RemeasureParams {
            max_setup_presses: REMEASURE_MAX_SETUP_PRESSES,
            reset_every: REMEASURE_RESET_EVERY,
            key_count: KEYS.len(),
        };
        executor.set_recording(false);
        let result = reconcile_with_bundled(
            executor,
            diff.matched,
            only_in_one_table,
            &targets,
            rng,
            &params,
        );
        executor.set_recording(true);
        for (status, key) in &result.dropped {
            let vk = KeyId(KEYS[*key] as u16);
            for cell in cells.iter_mut() {
                if cell.status == *status && cell.key == vk {
                    cell.prediction = None;
                }
            }
        }
        eprintln!(
            "内蔵表との突き合わせ: 一致{}・再測定で再現{}・再現せず{}・片側のみ{}",
            result.summary.matched,
            result.summary.reconfirmed,
            result.summary.not_reproduced,
            result.summary.only_in_one_table,
        );
        Some(result.summary)
    }

    pub fn run_main() {
        let strategy = if std::env::args().any(|arg| arg == "--strategy=s0") {
            Strategy::S0
        } else {
            Strategy::S6
        };
        let driver = build_driver(strategy);
        let initial = driver.initial_status();
        // A-6/B-3: 開始時点のTIP・(GJIのときだけ)config1.dbを記録し、終了時に再取得して
        // 比較する(学習中のIME/GJI設定の切り替え検出)。
        let tip_at_start = driver.tip_identity();
        let config1_db_at_start = (tip_at_start == TipIdentity::Gji)
            .then(awase_windows::gji_charset_autodetect::read_config1_db)
            .flatten();
        let model = build_model(initial);

        let mut rng = Rng::new(195);
        let prior = Prior::from_machine(&model, 0.0, &mut rng);
        let cost = CostModel::event();
        let mut executor = Executor::new(driver, AnomalyPolicy::default(), ReadPolicy::Single);

        let total_cells = model.states.len() as u32 * KEYS.len() as u32;
        executor.set_progress_sink(make_progress_sink(total_cells));

        let req = Req::default();
        run(
            strategy,
            &mut executor,
            &prior,
            &cost,
            &model.history_suspects,
            &req,
            &mut rng,
        );
        retry_nondeterministic_cells_once(
            &mut executor,
            strategy,
            &prior,
            &cost,
            &model.history_suspects,
            &req,
            &mut rng,
        );

        // code-review指摘: 学習(+やり直し)の直後、独立ウォーク(段階2)を走らせる前に
        // 統計をここで確定させる。ウォーク後に読むと、`stats.presses`/`elapsed_ms`
        // (Executor::pressが記録の有無に関わらず無条件に更新するため)にウォーク分
        // (固定300+リトライの可変分)が混入し、戦略比較(presses/elapsed_ms)の指標として
        // 意味を持たなくなる。`decode_errors`も同様に、学習に無関係な検証ウォーク中の
        // 一時的な観測失敗が「学習表に信頼できない観測が混じっている」という誤った
        // 警告を生む(実際にはtable自体はウォーク中recording=falseで変化しない)。
        let training_elapsed_ms = executor.elapsed_ms();
        let training_presses = executor.stats.presses;
        let decode_errors = executor.driver.decode_error_count();

        // [ADR195-T7](../../../docs/tasks/adr195-t7-safety-measures.md)項目2
        // (opus-adversarial-consult round1 M1対応): セッション監視
        // (`RealImeDriver::check_session_interference`)が無効化上限を超えて
        // いたら、検証ウォーク・表の書き出しへ進まずここで失敗として終了する。
        // 汚染された観測(外部からの書き込み・物理入力・フォーカス喪失)は
        // `Executor::press`が表への記録を既に見送っているが、無効化が多発した
        // セッションは表の残りのセルの信頼性も疑わしいため、書き出さない。
        exit_if_session_failed(
            &executor,
            strategy,
            training_elapsed_ms,
            training_presses,
            total_cells,
            decode_errors,
        );

        // C-7: 検証ウォーク専用の乱数(学習本体とは独立、時刻由来のシード)。
        let walk_seed = fresh_walk_seed();
        let mut walk_rng = Rng::new(walk_seed);
        let score = run_verification_walk(&mut executor, &mut walk_rng);

        // ADR196-T2決定1b項目7〜8: 既知構成なら内蔵表との突き合わせ→再測定。学習・検証と
        // 同じセッション監視の下で行うため、後続のセッション失敗判定より前に実行する。
        let mut cells = build_persisted_cells(&executor.table);
        let reconciliation =
            reconcile_against_bundled(&mut executor, tip_at_start, &mut cells, &mut walk_rng);

        // round2 N1対応: 検証ウォーク中にセッション監視が失敗と判定していたら、
        // (学習フェーズ直後のチェックだけでは検証ウォーク中の汚染を見逃すため)
        // ここでも確認し、表を書き出さない。
        exit_if_session_failed(
            &executor,
            strategy,
            training_elapsed_ms,
            training_presses,
            total_cells,
            decode_errors,
        );

        // C-1/A-6/B-3: フック断絶・学習中のIME切り替え・GJI設定変更のいずれかなら
        // 何を測ったか確定できないため、表を書かずに失敗として終了する。
        let abort_reason = end_of_session_abort_reason(
            &executor.driver,
            tip_at_start,
            config1_db_at_start.as_deref(),
        );
        exit_if_skipped(
            abort_reason,
            &executor,
            strategy,
            training_elapsed_ms,
            training_presses,
            total_cells,
            decode_errors,
        );

        let judgement = judge_score(&score, tip_at_start, reconciliation.as_ref());
        let verification = ScoredVerification {
            score,
            seed: walk_seed,
        };
        let (cell_count, write_result) = persist_judged_table(cells, verification, judgement);
        print_result_line(ResultLineArgs {
            strategy,
            training_elapsed_ms,
            training_presses,
            covered1: executor.table.covered1(),
            total_cells,
            decode_errors,
            cell_count,
            score,
            judgement,
            write_result: &write_result,
        });
        if decode_errors > 0 {
            eprintln!(
                "警告: observe_imm失敗によるフォールバックが{decode_errors}回発生。学習表に信頼できない観測が混じっている可能性がある。"
            );
        }
    }

    /// [`print_interference_failure_line`]の引数。
    #[derive(Clone, Copy)]
    struct InterferenceFailureArgs {
        strategy: Strategy,
        training_elapsed_ms: f64,
        training_presses: u32,
        covered1: usize,
        total_cells: u32,
        decode_errors: u32,
        contaminated_trials: u32,
        invalidated_trials: u32,
    }

    /// [ADR195-T7](../../../docs/tasks/adr195-t7-safety-measures.md)項目2
    /// (round1 M1対応): セッション監視の無効化上限を超えたときの専用result行。
    /// `print_result_line`と同じ`result status=... strategy=...`の形式を保ち
    /// `reason=interference`を足す——awase-settings(較正ウィザード)がこの行を
    /// パースする前提(ADR-195段階6)を崩さないため。
    fn print_interference_failure_line(args: InterferenceFailureArgs) {
        eprintln!(
            "学習セッションを失敗として終了します: 外部からの書き込み・物理入力・\
             フォーカス喪失により{}回の試行が無効化上限を超えました(汚染された観測{}件)。\
             学習表は書き出しません。",
            args.invalidated_trials, args.contaminated_trials
        );
        println!(
            "result status=failure strategy={} elapsed_ms={:.0} presses={} cells={} total={} \
             decode_errors={} contaminated_trials={} invalidated_trials={} reason=interference",
            args.strategy.name(),
            args.training_elapsed_ms,
            args.training_presses,
            args.covered1,
            args.total_cells,
            args.decode_errors,
            args.contaminated_trials,
            args.invalidated_trials,
        );
        let _ = std::io::stdout().flush();
    }

    /// [ADR195-T7](../../../docs/tasks/adr195-t7-safety-measures.md)項目2
    /// (opus-adversarial-consult round2 N2対応): `RealImeDriver::new()`の
    /// 初期化が失敗したときの専用result行。他のresult status=failure行と
    /// 同じ形式にし、`awase-settings`側が結果を確実にパースできるようにする
    /// (N2以前はプロセスが`Err`のまま終了し、result行が一切出ず「結果を
    /// 送らずに終了しました」としか表示されなかった)。`reason`は
    /// quiet window判定によるものかそれ以外かを呼び出し側
    /// (`build_driver`、round3 R2対応)が区別して渡す。
    fn print_driver_init_failure_line(
        strategy: Strategy,
        total_cells: u32,
        err: &windows::core::Error,
        reason: &str,
    ) {
        eprintln!("学習プロセスの初期化に失敗しました: {err}");
        println!(
            "result status=failure strategy={} elapsed_ms=0 presses=0 cells=0 total={total_cells} \
             decode_errors=0 reason={reason}",
            strategy.name(),
        );
        let _ = std::io::stdout().flush();
    }

    /// [`print_result_line`]の引数(clippyの`too_many_arguments`回避のため構造体にまとめる)。
    #[derive(Clone, Copy)]
    struct ResultLineArgs<'a> {
        strategy: Strategy,
        training_elapsed_ms: f64,
        training_presses: u32,
        covered1: usize,
        total_cells: u32,
        decode_errors: u32,
        cell_count: usize,
        score: ScoreReport,
        judgement: TableJudgement,
        write_result: &'a Result<(), String>,
    }

    /// `judgement`を`result`行に載せる大分類(詳細な理由は表ファイルのJSONに残る。
    /// `parse_learn_line`は未知のフィールドを無視するのでawase-settingsは壊れない)。
    fn judgement_tag(judgement: TableJudgement) -> &'static str {
        match judgement {
            TableJudgement::Accepted => "accepted",
            TableJudgement::NeedsConfirmation(_) => "needs_confirmation",
            TableJudgement::Rejected(_) => "rejected",
        }
    }

    /// ADR-195段階6決定5(項目5): result行は書き込みに成功してから出す
    /// (失敗したのに"success"を名乗らない)。`status=success`は「(採否に関わらず)表を
    /// 書けた」の意味のまま残す——`Rejected`/`NeedsConfirmation`でも退避ファイルへの
    /// 書き込みが成功していれば`success`になる(awase-settings側は`judgement=`を見て
    /// 「学習完了」と誤解させない表示にすること)。
    fn print_result_line(args: ResultLineArgs) {
        match args.write_result {
            Ok(()) => {
                println!(
                    "result status={} strategy={} elapsed_ms={:.0} presses={} cells={} total={} \
                     decode_errors={} persisted_cells={} verify_accuracy={:.3} verify_confidence={:.3} \
                     judgement={}",
                    if args.decode_errors == 0 {
                        "success"
                    } else {
                        "success_with_warnings"
                    },
                    args.strategy.name(),
                    args.training_elapsed_ms,
                    args.training_presses,
                    args.covered1,
                    args.total_cells,
                    args.decode_errors,
                    args.cell_count,
                    args.score.accuracy(),
                    args.score.confidence(),
                    judgement_tag(args.judgement),
                );
            }
            Err(reason) => {
                eprintln!("学習表の書き込みに失敗しました: {reason}");
                println!(
                    "result status=failure strategy={} elapsed_ms={:.0} presses={} cells={} total={} decode_errors={}",
                    args.strategy.name(),
                    args.training_elapsed_ms,
                    args.training_presses,
                    args.covered1,
                    args.total_cells,
                    args.decode_errors
                );
            }
        }
        let _ = std::io::stdout().flush();
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use awase_keymap_learn::judgement::{NeedsConfirmationReason, RejectedReason};
        use awase_keymap_learn::model::{Disposition, Outcome, Status};

        #[test]
        fn judgement_tag_covers_every_variant() {
            assert_eq!(judgement_tag(TableJudgement::Accepted), "accepted");
            assert_eq!(
                judgement_tag(TableJudgement::NeedsConfirmation(
                    NeedsConfirmationReason::UnverifiedMsImeNative
                )),
                "needs_confirmation"
            );
            assert_eq!(
                judgement_tag(TableJudgement::Rejected(RejectedReason::LowAccuracy)),
                "rejected"
            );
        }

        fn st(open: bool, mode: u8) -> Status {
            Status {
                open,
                mode,
                composing: false,
            }
        }

        fn out(open: bool, mode: u8) -> Outcome {
            Outcome {
                status: st(open, mode),
                disp: Disposition::None,
            }
        }

        /// B2回帰テスト: `build_persisted_cells`は`Table`が内部で使う`KEYS`配列の
        /// **添字**ではなく、実際のWindows VKコードでセルを書く。添字1(=`KEYS[1]`=
        /// `0x1C`=Henkan)を、生VKの1(存在しないVK値)と混同していないことを固定する。
        #[test]
        fn build_persisted_cells_uses_real_vk_codes_not_key_array_indices() {
            let mut table = Table::new();
            // 添字1 = KEYS[1] = 0x1C(Henkan)。もし添字のまま書くと`KeyId(1)`になり、
            // 実際には無変換(0x1D=KEYS[0])のVKと衝突する誤りを検出できない。
            table.record(st(true, 0x09), 1, None, out(false, 0));
            let cells = build_persisted_cells(&table);
            assert_eq!(cells.len(), 1);
            assert_eq!(
                cells[0].key,
                KeyId(0x1C),
                "添字1は実VK 0x1C(Henkan)であるべき"
            );
            assert_ne!(
                cells[0].key,
                KeyId(1),
                "添字をそのままKeyIdにしてはいけない(B2)"
            );
        }

        /// M5回帰テスト: 訪問したが決定的でないセル(観測が食い違う)も、省略せず
        /// `prediction: None`で書く。書き手が未測定/非決定セルを省略できると、
        /// 読み手側の縮退率チェックの分母を書き手が恣意的に操作できてしまう。
        #[test]
        fn build_persisted_cells_keeps_visited_nondeterministic_cells_with_none_prediction() {
            let mut table = Table::new();
            // 同じ文脈(ctx)・同じ(status, key)に3対2で食い違う観測を記録する。
            // 少数派2件はDEFAULT_MIN_MINORITY(2)に達するため、誤りに強い分類でも
            // 本物の非決定として扱われる(verify.rs::
            // classify_robust_declares_nondet_when_minority_reaches_thresholdと同じ形。
            // 1対1のタイでは多数派の先着優先でDet扱いになってしまい、このテストの
            // 意図〈訪問したが決定的でないセル〉を検証できない)。
            table.record(st(true, 0x09), 0, Some(1), out(true, 0x09));
            table.record(st(true, 0x09), 0, Some(1), out(true, 0x09));
            table.record(st(true, 0x09), 0, Some(1), out(true, 0x09));
            table.record(st(true, 0x09), 0, Some(1), out(false, 0));
            table.record(st(true, 0x09), 0, Some(1), out(false, 0));
            let cells = build_persisted_cells(&table);
            assert_eq!(cells.len(), 1, "訪問したセルは省略せず1件書くべき");
            assert_eq!(
                cells[0].prediction, None,
                "決定的と言えないセルはNoneで書くべき(省略ではない)"
            );
        }
    }
}

#[cfg(windows)]
fn main() {
    app::run_main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("awase-keymap-learn-win is Windows-only");
}
