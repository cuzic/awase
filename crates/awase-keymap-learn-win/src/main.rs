#[cfg(windows)]
mod app {
    use std::io::Write;

    use awase_keymap_learn::anomaly::AnomalyPolicy;
    use awase_keymap_learn::cost::CostModel;
    use awase_keymap_learn::exec::{Executor, ImeDriver, ReadPolicy, Stats};
    use awase_keymap_learn::graph::Prior;
    use awase_keymap_learn::judgement::{adopt_needs_confirmation, AdoptRejected};
    use awase_keymap_learn::model::KeyId;
    use awase_keymap_learn::persist::{from_json, LoadError, PersistedCell, PersistedTable};
    use awase_keymap_learn::rng::Rng;
    use awase_keymap_learn::sample_models::atok_like;
    use awase_keymap_learn::strategy::{run, Req, Strategy};
    use awase_keymap_learn::table::Table;
    use awase_keymap_learn::verify::{
        classify_robust, predict, score_walk, WalkObs, DEFAULT_MIN_MINORITY,
    };
    use awase_keymap_learn_win::RealImeDriver;

    const KEYS: [u32; 14] = [
        0x1D, 0x1C, 0xF2, 0xF1, 0xF0, 0xF3, 0x19, 0x16, 0x1A, 0x1B, 0x0D, 0x20, 0x08, 0x41,
    ];

    /// ADR-196決定1b-8: 判定書き換えモード起動フラグ。`awase-settings`の
    /// 「学習結果を使う」ボタン([ADR196-T4](../../../docs/tasks/adr196-t4-ui-status-and-adoption.md)、
    /// 未実装)が、実機のIME駆動を一切せずこのプロセスをこのフラグで再起動して、
    /// 要確認状態の判定だけをアトミックに採用へ書き換える(表ファイルの書き手は
    /// 学習プロセスのみという原則、決定3aを保つため)。
    const ADOPT_PENDING_JUDGEMENT_FLAG: &str = "--adopt-pending-judgement";

    /// ADR-195段階6: 何押下ごとに標準出力へ進捗行を書き出すか。毎回書くと
    /// 子プロセス側(awase-settings)のパース負荷・パイプI/Oが無駄に増えるため間引く。
    const PROGRESS_EVERY_N_PRESSES: u32 = 10;

    /// 段階2(自己検証)の独立ランダムウォークの長さ。ADR-195/ADR196-T2が挙げる
    /// 「正答率判定には最低300ステップ」の基準に合わせる。
    const VERIFICATION_WALK_STEPS: usize = 300;

    /// `<config dir>/keymap-learn-table.json`のパス。`awase.exe`/`awase-settings.exe`と
    /// 同じ探索規則(`awase::paths::resolve_relative_to_exe`、exeの隣→開発ビルドの
    /// ワークスペースルート→CWD相対の順)でconfig.tomlを探し、その親ディレクトリへ書く
    /// (`crates/awase-windows/src/state/key_effect_runtime.rs::table_file_path`と
    /// 同じ規約)。config.tomlが見つからなければ`None`(書き込み先を決められない)。
    fn table_file_path() -> Option<std::path::PathBuf> {
        let config_path = awase::paths::resolve_relative_to_exe("config.toml");
        if !config_path.exists() {
            return None;
        }
        config_path
            .parent()
            .map(|dir| dir.join("keymap-learn-table.json"))
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
    fn run_verification_walk<D: ImeDriver>(
        exec: &mut Executor<D>,
        rng: &mut Rng,
    ) -> awase_keymap_learn::verify::ScoreReport {
        exec.set_progress_sink(|_, _| {});
        exec.set_recording(false);
        let mut walk = Vec::with_capacity(VERIFICATION_WALK_STEPS);
        for _ in 0..VERIFICATION_WALK_STEPS {
            // opus-adversarial-consult round2 N3対応: セッション監視が既に
            // 失敗と判定していたら、採点にならない押下を続けない。
            if exec.driver.should_abort() {
                break;
            }
            let key = rng.below(KEYS.len());
            if let Some(info) = exec.press(key) {
                // round2 N1対応: 汚染された観測(外部からの書き込み・物理入力・
                // フォーカス喪失)は採点に使わない。学習フェーズは
                // `Executor::press`が表への記録を見送るが、このウォークは
                // `recording=false`で呼ばれるため同じ保護がかかっておらず、
                // 汚染された観測がそのまま`verify_accuracy`の採点対象に
                // なっていた(表そのものは正しいのに不当にスコアが下がる)。
                if info.contaminated {
                    continue;
                }
                walk.push(WalkObs {
                    status: info.before,
                    key,
                    outcome: info.outcome,
                });
            }
        }
        exec.set_recording(true);
        score_walk(&exec.table, DEFAULT_MIN_MINORITY, &walk)
    }

    /// ADR-195段階3〜4への結合(B1対応): 表を永続化フォーマットへ変換し、一時ファイル+
    /// renameで原子的に書き込む。指紋(ADR-195段階8)は、その計算方式自体がADR-196決定3で
    /// 再設計中のため、ここでは`None`のまま残す(ADR196-T5が実配線する)。
    ///
    /// 戻り値は(書き込もうとしたセル数, 書き込み結果)。
    fn persist_learned_table(table: &Table) -> (usize, Result<(), String>) {
        let cells = build_persisted_cells(table);
        let cell_count = cells.len();
        let persisted = PersistedTable::new(cells);
        let write_result = table_file_path()
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

    /// 判定書き換えモードの失敗理由。`code()`は標準出力の`reason=`欄へ載せる
    /// 空白・コロンを含まない固定トークン(code-review指摘: パス・OSエラー文言を
    /// 含む自由形式の理由をそのまま`reason=`へ埋めると、awase-settings側の
    /// `split_whitespace()`+`key=value`パース〈`keymap_learn_launcher::parse_learn_line`、
    /// 既存の`result`行と同じ規約〉が壊れる)。詳細はこの型の`Display`でeprintln専用に持つ
    /// (既存の`print_result_line`が失敗時に`eprintln!`で詳細を逃がすのと同じ流儀)。
    #[derive(Debug)]
    enum AdoptFailure {
        NoConfig,
        ReadFailed(std::path::PathBuf, std::io::Error),
        ParseFailed(LoadError),
        Rejected(AdoptRejected),
        SerializeFailed(serde_json::Error),
        WriteFailed(std::path::PathBuf, anyhow::Error),
    }

    impl AdoptFailure {
        const fn code(&self) -> &'static str {
            match self {
                Self::NoConfig => "no_config",
                Self::ReadFailed(..) => "read_failed",
                Self::ParseFailed(..) => "parse_failed",
                Self::Rejected(AdoptRejected::NoJudgement) => "no_judgement",
                Self::Rejected(AdoptRejected::Rejected) => "rejected",
                Self::SerializeFailed(..) => "serialize_failed",
                Self::WriteFailed(..) => "write_failed",
            }
        }
    }

    impl std::fmt::Display for AdoptFailure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::NoConfig => write!(f, "config.tomlが見つかりません"),
                Self::ReadFailed(path, e) => write!(f, "{}への読み込みに失敗: {e}", path.display()),
                Self::ParseFailed(e) => write!(f, "{e}"),
                Self::Rejected(reason) => write!(f, "{reason}"),
                Self::SerializeFailed(e) => write!(f, "表のシリアライズに失敗: {e}"),
                Self::WriteFailed(path, e) => {
                    write!(f, "{}への書き込みに失敗: {e:#}", path.display())
                }
            }
        }
    }

    /// 決定1b-8の中核: 既存の`keymap-learn-table.json`を読み、要確認状態の判定を
    /// アトミックに採用へ書き換える。純粋な採否ロジック(`adopt_needs_confirmation`)は
    /// `awase-keymap-learn::judgement`が持つ(ホストでユニットテスト済み)——本関数は
    /// ファイルI/Oの糊付けのみ。パスを引数化しているのはテスト容易性のため
    /// (`table_file_path()`自体はexe相対探索でテストで差し替えられない)。
    fn adopt_pending_judgement_at(path: &std::path::Path) -> Result<(), AdoptFailure> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| AdoptFailure::ReadFailed(path.to_path_buf(), e))?;
        let mut table = from_json(&json).map_err(AdoptFailure::ParseFailed)?;
        table.judgement =
            Some(adopt_needs_confirmation(table.judgement).map_err(AdoptFailure::Rejected)?);
        let rewritten = table.to_json().map_err(AdoptFailure::SerializeFailed)?;
        awase::fs_atomic::write_atomic(path, rewritten.as_bytes())
            .map_err(|e| AdoptFailure::WriteFailed(path.to_path_buf(), e))
    }

    fn adopt_pending_judgement() -> Result<(), AdoptFailure> {
        let path = table_file_path().ok_or(AdoptFailure::NoConfig)?;
        adopt_pending_judgement_at(&path)
    }

    /// 判定書き換えモードのエントリポイント。成否を標準出力へ運ぶ(awase-settings側の
    /// パース対象、`result`行とは別の`adopt`行——学習セッションの結果ではないため)。
    /// 失敗の詳細(パス・OSエラー文言)はstderrへ、stdoutには空白を含まない
    /// 理由コードのみを載せる(code-review指摘、上記`AdoptFailure`のdoc参照)。
    fn run_adopt_mode() {
        match adopt_pending_judgement() {
            Ok(()) => println!("adopt status=success"),
            Err(failure) => {
                eprintln!("学習表の判定書き換えに失敗しました: {failure}");
                println!("adopt status=failure reason={}", failure.code());
            }
        }
        let _ = std::io::stdout().flush();
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

    pub fn run_main() {
        if std::env::args().any(|arg| arg == ADOPT_PENDING_JUDGEMENT_FLAG) {
            run_adopt_mode();
            return;
        }
        let strategy = if std::env::args().any(|arg| arg == "--strategy=s0") {
            Strategy::S0
        } else {
            Strategy::S6
        };
        let driver = build_driver(strategy);
        let initial = driver.initial_status();
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

        let score = run_verification_walk(&mut executor, &mut rng);

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

        let (cell_count, write_result) = persist_learned_table(&executor.table);
        print_result_line(ResultLineArgs {
            strategy,
            training_elapsed_ms,
            training_presses,
            covered1: executor.table.covered1(),
            total_cells,
            decode_errors,
            cell_count,
            score,
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
        score: awase_keymap_learn::verify::ScoreReport,
        write_result: &'a Result<(), String>,
    }

    /// ADR-195段階6決定5(項目5): result行は書き込みに成功してから出す
    /// (失敗したのに"success"を名乗らない)。
    fn print_result_line(args: ResultLineArgs) {
        match args.write_result {
            Ok(()) => {
                println!(
                    "result status={} strategy={} elapsed_ms={:.0} presses={} cells={} total={} \
                     decode_errors={} persisted_cells={} verify_accuracy={:.3} verify_confidence={:.3}",
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
        use awase_keymap_learn::model::{Disposition, Outcome, Status};

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

        use awase_keymap_learn::judgement::{
            NeedsConfirmationReason, RejectedReason, TableJudgement,
        };
        use awase_keymap_learn::model::KeyId;

        /// テストごとに衝突しない一時ファイルパスを作る(`std::env::temp_dir()`+
        /// テスト名+スレッドIDの規約、`awase-settings::bug_report`テストと同型)。
        fn temp_table_path(label: &str) -> std::path::PathBuf {
            std::env::temp_dir().join(format!(
                "awase_keymap_learn_win_adopt_test_{label}_{:?}.json",
                std::thread::current().id()
            ))
        }

        fn sample_table(judgement: Option<TableJudgement>) -> PersistedTable {
            let mut table = PersistedTable::new(vec![PersistedCell {
                status: st(true, 0x09),
                key: KeyId(0x1D),
                prediction: None,
            }]);
            table.judgement = judgement;
            table
        }

        /// 決定1b-8: 要確認状態のファイルは採用へ書き換わり、ディスク上にも反映される。
        #[test]
        fn adopt_pending_judgement_at_accepts_needs_confirmation_on_disk() {
            let path = temp_table_path("accepts");
            let table = sample_table(Some(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::SystematicMismatch {
                    mismatch_percent: 40,
                },
            )));
            std::fs::write(&path, table.to_json().unwrap()).unwrap();

            let result = adopt_pending_judgement_at(&path);

            assert!(result.is_ok(), "expected success, got {result:?}");
            let reloaded = from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(reloaded.judgement, Some(TableJudgement::Accepted));
            let _ = std::fs::remove_file(&path);
        }

        /// 安全弁: 不採用(低正答率)のファイルは、書き換え要求があっても変更されない
        /// (エラーを返し、ディスク上の内容もそのまま)。
        #[test]
        fn adopt_pending_judgement_at_leaves_rejected_file_untouched() {
            let path = temp_table_path("rejected");
            let table = sample_table(Some(TableJudgement::Rejected(RejectedReason::LowAccuracy)));
            std::fs::write(&path, table.to_json().unwrap()).unwrap();

            let result = adopt_pending_judgement_at(&path);

            match result {
                Err(failure) => assert_eq!(failure.code(), "rejected"),
                Ok(()) => panic!("expected rejection for a low-accuracy table"),
            }
            let reloaded = from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(reloaded, table, "拒否時はファイルを一切書き換えない");
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn adopt_pending_judgement_at_reports_missing_file() {
            let path = temp_table_path("missing_never_created");
            let _ = std::fs::remove_file(&path); // 前回の残骸があれば消す

            let result = adopt_pending_judgement_at(&path);

            match result {
                Err(failure) => assert_eq!(failure.code(), "read_failed"),
                Ok(()) => panic!("expected a read failure for a missing file"),
            }
        }

        /// code-review指摘の回帰テスト: 標準出力の`reason=`欄は空白・コロンを含む
        /// 自由形式の文言(パス・OSエラー文言)であってはならない
        /// (`keymap_learn_launcher::parse_learn_line`のsplit_whitespace()+key=value
        /// パースを壊すため)。`code()`が返す全トークンがこの制約を満たすことを固定する。
        #[test]
        fn adopt_failure_codes_are_single_whitespace_free_tokens() {
            let path = std::path::PathBuf::from("dummy");
            let samples = [
                AdoptFailure::NoConfig,
                AdoptFailure::ReadFailed(
                    path.clone(),
                    std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
                ),
                AdoptFailure::Rejected(AdoptRejected::NoJudgement),
                AdoptFailure::Rejected(AdoptRejected::Rejected),
            ];
            for sample in &samples {
                let code = sample.code();
                assert!(
                    code.split_whitespace().count() == 1 && !code.contains(':'),
                    "code {code:?} must be a single whitespace/colon-free token"
                );
            }
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
