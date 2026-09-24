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
        adopt_needs_confirmation, combine, gate_on_fingerprint, judge_self_verification,
        AdoptRejected, ReconciliationSummary, ScoredVerification, TableJudgement,
        ACCURACY_THRESHOLD, DEGENERATION_THRESHOLD, MIN_PREDICTED_STEPS,
        SYSTEMATIC_MISMATCH_THRESHOLD,
    };
    use awase_keymap_learn::model::KeyId;
    use awase_keymap_learn::persist::{from_json, LoadError, PersistedCell, PersistedTable};
    use awase_keymap_learn::remeasure::{
        reconcile_with_bundled, MismatchedTarget, RemeasureParams,
    };
    use awase_keymap_learn::revalidation::{
        apply_revalidation, outcome_of_revalidation, table_from_persisted, RevalidationOutcome,
        StoredEnvVersion,
    };
    use awase_keymap_learn::rng::Rng;
    use awase_keymap_learn::sample_models::{atok_like, atok_like_with_modes};
    use awase_keymap_learn::staleness::{self, FingerprintProbe};
    use awase_keymap_learn::strategy::{run, Req, Strategy};
    use awase_keymap_learn::table::Table;
    use awase_keymap_learn::verify::{
        classify_robust, predict, score_walk, ScoreReport, WalkObs, DEFAULT_MIN_MINORITY,
    };
    use awase_keymap_learn_win::RealImeDriver;
    use awase_windows::state::ime_kind::TipIdentity;
    use awase_windows::state::key_effect_predictor::TableKey;
    use awase_windows::state::key_effect_runtime::current_fingerprint_probe;

    const KEYS: [u32; 14] = [
        0x1D, 0x1C, 0xF2, 0xF1, 0xF0, 0xF3, 0x19, 0x16, 0x1A, 0x1B, 0x0D, 0x20, 0x08, 0x41,
    ];

    /// ADR-196決定1b-8: 判定書き換えモード起動フラグ。`awase-settings`の
    /// 「学習結果を使う」ボタン([ADR196-T4](../../../docs/tasks/adr196-t4-ui-status-and-adoption.md)、
    /// 未実装)が、実機のIME駆動を一切せずこのプロセスをこのフラグで再起動して、
    /// 要確認状態の判定だけをアトミックに採用へ書き換える(表ファイルの書き手は
    /// 学習プロセスのみという原則、決定3aを保つため)。
    const ADOPT_PENDING_JUDGEMENT_FLAG: &str = "--adopt-pending-judgement";

    /// ADR196-T5決定3a: 軽量再検証モード起動フラグ。段階1(格子学習)を飛ばし、保存済みの表を
    /// 予測として使う段階2(自己検証ウォーク)だけを実行する。合格なら指紋・採点を書き直し、
    /// 不合格(正答率などが採否条件を割った)のときに初めて表を失効させる。
    const REVALIDATE_FLAG: &str = "--revalidate";
    /// 検証ウォークの1歩ごとの記録をstderrへ出す(正答率のばらつきの調査用、標準出力の`result`行には影響しない)。
    const TRACE_WALK_FLAG: &str = "--trace-walk";

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
    /// - B-5対応: 読み手が**そもそも表現できないキー**(`TableKey::from_vk`が`None`。
    ///   学習が入力中状態へ入るために使う文字キー0x41)のセルは書かない。書くと全セルの
    ///   1/14(約7.1%)が「変換できないセル」として`coverage_ratio`の分母に入り、
    ///   `MIN_COVERAGE_RATIO`(80%)の実余裕が約13%まで削られる。これは測定できた/できない
    ///   の判断ではなく読み手の語彙の外にあるキーなので、M5の懸念(測ったセルを省いて分母を
    ///   操作する)には当たらない。
    fn build_persisted_cells(table: &Table) -> Vec<PersistedCell> {
        table
            .cells()
            .filter(|&(&(_, key_idx), _)| {
                u16::try_from(KEYS[key_idx]).is_ok_and(|vk| TableKey::from_vk(vk).is_some())
            })
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
    /// `--trace-walk`用: 検証ウォークの1歩を、採点と同じ規則(`predict`)で整形する。
    fn trace_walk_step<D: ImeDriver>(
        exec: &Executor<D>,
        index: usize,
        info: awase_keymap_learn::exec::PressInfo,
        key: usize,
    ) -> String {
        use awase_keymap_learn::walk_trace::{
            format_contaminated_step, format_walk_step, CellInfo,
        };
        let vk = KEYS[key];
        if info.contaminated {
            return format_contaminated_step(index, info.before, vk);
        }
        let observations = exec.table.observations(info.before, key);
        let mut distinct: Vec<_> = observations.iter().map(|o| o.outcome).collect();
        distinct.sort_by_key(|o| format!("{o:?}"));
        distinct.dedup();
        format_walk_step(
            index,
            info.before,
            vk,
            info.outcome,
            predict(&exec.table, info.before, key, DEFAULT_MIN_MINORITY),
            CellInfo {
                class: classify_robust(&exec.table, info.before, key, DEFAULT_MIN_MINORITY),
                n_obs: observations.len(),
                n_distinct: distinct.len(),
            },
        )
    }

    /// 進捗sinkは学習の巡回にだけ意味があるので、ここでは無効化する(有効なままだと
    /// `recording=false`の間もpressごとに呼ばれ、cell数が増えないのにelapsed_msだけ
    /// 伸びる不審な進捗行が出る)。
    ///
    /// C-2対応: 固定回数ではなく、予測できたステップ数([`ScoreReport::predicted`])が
    /// [`MIN_PREDICTED_STEPS`]に達するまで押下を続ける。[`VERIFICATION_WALK_MAX_STEPS`]
    /// (押下の試行回数)に達しても届かなければ打ち切って返す(`judge_self_verification`が
    /// `InsufficientSamples`として不採用にする)。
    fn run_verification_walk<D: ImeDriver>(
        exec: &mut Executor<D>,
        rng: &mut Rng,
        trace: bool,
    ) -> ScoreReport {
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
                if trace {
                    eprintln!("{}", trace_walk_step(exec, attempts, info, key));
                }
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
    /// renameで原子的に書き込む。キーマップ設定の指紋(ADR-195段階8、`awase_windows::state::
    /// key_effect_runtime::current_fingerprint_probe`)と、IME本体の版(`env_version`、
    /// ADR-196決定3b)を書く。指紋を計算できなかった(`Unavailable`)場合の判定の格下げ
    /// (`gate_on_fingerprint`)は呼び出し側が`judgement`へ反映済みであること。
    ///
    /// C-4/C-9対応: `judgement`が`Accepted`なら本体(`keymap-learn-table.json`)へ、
    /// それ以外は[`last_attempt_file_path`]へ書く。
    ///
    /// 戻り値は(書き込もうとしたセル数, 書き込み結果)。
    fn persist_judged_table(
        cells: Vec<PersistedCell>,
        verification: ScoredVerification,
        judgement: TableJudgement,
        env_version: Option<StoredEnvVersion>,
        fingerprint: FingerprintProbe,
    ) -> (usize, Result<(), String>) {
        let cell_count = cells.len();
        let mut persisted = PersistedTable::new(cells)
            .with_env_version(env_version)
            .with_verification(verification)
            .with_judgement(judgement);
        if let FingerprintProbe::Computed(fp) = fingerprint {
            persisted = persisted.with_fingerprint(fp);
        }
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
                Self::Rejected(AdoptRejected::NoFingerprint) => "no_fingerprint",
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

    /// 決定1b-8の中核: 保留中の表を採用済みへ書き換えて`keymap-learn-table.json`へ置く。
    /// 要確認/不採用の表は`keymap-learn-last-attempt.json`(`pending`)へ退避される(C-9)ので、
    /// それがあればそれを読み、採用後に`table_path`へアトミックに昇格させて`pending`を消す
    /// (実機windows-latestで、退避先とは別の`table_path`だけを読んで常に`read_failed`になる
    /// 不具合が見つかった)。`pending`が無ければ従来どおり`table_path`を読んで書き換える
    /// (採用済みへの再実行は冪等成功)。純粋な採否ロジック(`adopt_needs_confirmation`)は
    /// `awase-keymap-learn::judgement`が持つ——本関数はファイルI/Oの糊付けのみ。
    /// パスを引数化しているのはテスト容易性のため(exe相対探索はテストで差し替えられない)。
    fn adopt_pending_judgement_at(
        pending: &std::path::Path,
        table_path: &std::path::Path,
    ) -> Result<(), AdoptFailure> {
        let source = if pending.exists() {
            pending
        } else {
            table_path
        };
        let json = std::fs::read_to_string(source)
            .map_err(|e| AdoptFailure::ReadFailed(source.to_path_buf(), e))?;
        let mut table = from_json(&json).map_err(AdoptFailure::ParseFailed)?;
        table.judgement = Some(
            adopt_needs_confirmation(table.judgement, table.fingerprint.is_some())
                .map_err(AdoptFailure::Rejected)?,
        );
        let rewritten = table.to_json().map_err(AdoptFailure::SerializeFailed)?;
        awase::fs_atomic::write_atomic(table_path, rewritten.as_bytes())
            .map_err(|e| AdoptFailure::WriteFailed(table_path.to_path_buf(), e))?;
        if source == pending {
            // 昇格済み。残しても次回は同じ内容を昇格するだけ(冪等)なので、消せなくても成功。
            let _ = std::fs::remove_file(pending);
        }
        Ok(())
    }

    fn adopt_pending_judgement() -> Result<(), AdoptFailure> {
        let table_path = table_file_path().ok_or(AdoptFailure::NoConfig)?;
        let pending = last_attempt_file_path().ok_or(AdoptFailure::NoConfig)?;
        adopt_pending_judgement_at(&pending, &table_path)
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

    /// 軽量再検証モード(ADR196-T5決定3a)。標準出力の`revalidate`行は`adopt`行と同じく
    /// `key=value`の空白区切りで、失敗理由は空白を含まない固定トークンだけを載せる。
    fn run_revalidate_mode() {
        let process_start = SystemTime::now();
        let result = revalidate_table(process_start);
        let line = match &result {
            Ok((RevalidationOutcome::Passed, score)) => format!(
                "revalidate status=passed accuracy={:.3} predicted={}",
                score.accuracy(),
                score.predicted()
            ),
            Ok((RevalidationOutcome::Invalidated(reason), score)) => format!(
                "revalidate status=invalidated reason={reason:?} accuracy={:.3} predicted={}",
                score.accuracy(),
                score.predicted()
            ),
            Err(reason) => format!("revalidate status=failure reason={reason}"),
        };
        println!("{line}");
        let _ = std::io::stdout().flush();
        if result.is_err() {
            std::process::exit(1);
        }
    }

    /// 保存済みの表を読み、実機で自己検証ウォークだけを走らせ、結果を表ファイルへ
    /// アトミックに書き戻す。ウォーク中の外部書き込み・フォーカス喪失・IME切り替え等で
    /// 何を測ったか確定できない場合は、表を触らず`Err`(固定トークン)を返す。
    fn revalidate_table(
        process_start: SystemTime,
    ) -> Result<(RevalidationOutcome, ScoreReport), &'static str> {
        let path = table_file_path().ok_or("no_config")?;
        let json = std::fs::read_to_string(&path).map_err(|_| "read_failed")?;
        let persisted = from_json(&json).map_err(|_| "parse_failed")?;
        // 失効済み(Rejected)の表は、再検証に合格しても復活させない(再学習が必要)。
        if matches!(persisted.judgement, Some(TableJudgement::Rejected(_))) {
            return Err("already_rejected");
        }
        let driver = build_driver(Strategy::S6);
        let tip = driver.tip_identity();
        // 表が測った構成と今の構成が違えば、再検証に合格しても復活させない(GJIの表を
        // Microsoft IME本体の下で、あるいはキーマップ変更後に「再検証合格」させない)。
        // 指紋を持たない旧形式の表は`check`がFreshにする(従来どおり)。
        let fingerprint_at_start = current_fingerprint_probe(tip);
        if staleness::check(&persisted, fingerprint_at_start).is_stale() {
            return Err("keymap_changed");
        }
        let config1_db_at_start = (tip == TipIdentity::Gji)
            .then(awase_windows::gji_charset_autodetect::read_config1_db)
            .flatten();
        let mut executor = Executor::new(driver, AnomalyPolicy::default(), ReadPolicy::Single);
        executor.table = table_from_persisted(&persisted, |k| {
            KEYS.iter().position(|&vk| vk == u32::from(k.0))
        });
        executor.reset();
        let seed = fresh_walk_seed();
        let score = run_verification_walk(&mut executor, &mut Rng::new(seed), false);
        if executor.driver.session_failed() {
            return Err("interference");
        }
        if let Some(reason) =
            end_of_session_abort_reason(&executor.driver, tip, config1_db_at_start.as_deref())
        {
            return Err(reason);
        }
        if current_fingerprint_probe(tip) != fingerprint_at_start {
            return Err("keymap_changed");
        }
        let outcome = outcome_of_revalidation(judge_score(&score, tip, None));
        let rewritten = apply_revalidation(
            persisted,
            outcome,
            probe_env_version(tip, process_start),
            ScoredVerification { score, seed },
        );
        let json = rewritten.to_json().map_err(|_| "serialize_failed")?;
        awase::fs_atomic::write_atomic(&path, json.as_bytes()).map_err(|_| "write_failed")?;
        Ok((outcome, score))
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
                let total_cells_estimate =
                    atok_like().distinct_status_count() as u32 * KEYS.len() as u32;
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

    /// 学習の初期仮説モデル(ATOK風モデルの抽象modeを実機の変換モード値へ対応づけ、
    /// 開始状態を実機の`initial`へ合わせる)。Microsoft IME本体は全角/半角カタカナ・全角英数にも
    /// 到達する(windows-latest実測: 0x13が683回、0x18が187回)ので5モード、
    /// GJI(ATOK/MS-IMEプリセット)は従来の2モード。
    fn build_model(
        initial: awase_keymap_learn::model::Status,
        tip: TipIdentity,
    ) -> awase_keymap_learn::model::Machine {
        let native_modes = tip == TipIdentity::MsImeNative;
        let mut model = atok_like_with_modes(if native_modes { 5 } else { 2 });
        for state in &mut model.states {
            // ヒューリスティックな初期仮説として、抽象modeを実機の変換モード値
            // (`Status::mode_from_raw_conv`の出力)へ対応づける。
            // 2モード: 0/1=0x09/0x00、5モード: 0〜4=ひらがな/半角英数/全角カタカナ/半角カタカナ/全角英数。
            state.status.mode = match state.status.mode {
                0 => 0x09,
                1 => 0x00,
                2 => 0x0B,
                3 => 0x03,
                _ => 0x08,
            };
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
    /// 上限。windows-latest実GJI(ATOKプリセット)で全80セル×20回=1600件の到達所要押下数を
    /// 実測: 中央値11・p95≈100・p99≈150・最大230(60超は10.3%、240超は0件)。実測最大230+
    /// 約9%の余裕=250(60では10%が「確認できず」で予測を落とされていた)。RESET_EVERY=24だけ
    /// (n=240)でも最大223・p99=140で、押下数はRESET_EVERYに依存しない(下記)。
    const REMEASURE_MAX_SETUP_PRESSES: usize = 250;
    /// セットアップ押下がこの回数続けて目的のstatusに出会えなければリセットして歩き直す。
    /// 同構成・全80セル×3回で3/6/12/24/48を比較: 到達押下数の平均は22〜26で差が無く
    /// (歩き直しは到達に効かない)、所要時間だけが変わった(全80セルの再測定で
    /// 3:約635s・6:約344s・12:約226s・24:約186s・48:約159s)。24以上は逓減。
    /// 24と48の優劣は測定では決まらない(判断): 汚染された状態から抜けるリセットの機会を
    /// 残す意味で、逓減の入口の24とする。
    const REMEASURE_RESET_EVERY: usize = 24;

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
        // 診断: 内蔵表と食い違ったセルごとの再測定結果(内蔵表側の版ずれ等の判断材料)。
        for (target, outcome) in &result.cells {
            let vk = KeyId(KEYS[target.key] as u16);
            let probe = PersistedCell {
                status: target.status,
                key: vk,
                prediction: Some(target.learned),
            };
            let bundled =
                awase_windows::state::key_effect_runtime::describe_bundled_cell(&probe, preset)
                    .unwrap_or_else(|| "(取得不可)".to_string());
            eprintln!(
                "再測定: {outcome:?} vk={:#04x} status={:?} learned={:?} bundled={bundled}",
                KEYS[target.key], target.status, target.learned
            );
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

    /// プロセスのエントリポイント。判定書き換えモード(実機のIME駆動なし)か、
    /// 通常の学習セッション(`run_main`)かをフラグで振り分ける。
    pub fn entry() {
        if std::env::args().any(|arg| arg == ADOPT_PENDING_JUDGEMENT_FLAG) {
            run_adopt_mode();
        } else if std::env::args().any(|arg| arg == REVALIDATE_FLAG) {
            run_revalidate_mode();
        } else {
            run_main();
        }
    }

    /// 版取得(Toolhelp・`GetFileVersionInfoW`)がブロックした場合に学習の記録を止めない上限。
    const ENV_VERSION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

    /// 学習時点のIME本体の版(ADR-196決定3b)。GJIのときだけConverterのファイル版を取る
    /// (Microsoft IME側の4値はADR-197待ちで未実装のため`None`)。
    fn probe_env_version(tip: TipIdentity, process_start: SystemTime) -> Option<StoredEnvVersion> {
        if tip != TipIdentity::Gji {
            return None;
        }
        StoredEnvVersion::from_probe(awase_keymap_learn_win::probe_gji_env_version_with_timeout(
            process_start,
            ENV_VERSION_PROBE_TIMEOUT,
        ))
    }

    #[allow(clippy::too_many_lines)] // 学習の各段階を直列に並べる入口で、段階ごとの分割はしない
    fn run_main() {
        let process_start = SystemTime::now();
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
        // 学習時点のキーマップ指紋(表へ書く)。終了時に再計算して、学習中に構成が変わって
        // いたら`Unavailable`扱い(何を測ったか確定できない)にする。
        let fingerprint_at_start = current_fingerprint_probe(tip_at_start);
        let model = build_model(initial, tip_at_start);

        let mut rng = Rng::new(195);
        let prior = Prior::from_machine(&model, 0.0, &mut rng);
        let cost = CostModel::event();
        let mut executor = Executor::new(driver, AnomalyPolicy::default(), ReadPolicy::Single);

        let total_cells = model.distinct_status_count() as u32 * KEYS.len() as u32;
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
        let trace_walk = std::env::args().any(|arg| arg == TRACE_WALK_FLAG);
        let score = run_verification_walk(&mut executor, &mut walk_rng, trace_walk);

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

        let fingerprint = if current_fingerprint_probe(tip_at_start) == fingerprint_at_start {
            fingerprint_at_start
        } else {
            FingerprintProbe::Unavailable
        };
        // 指紋を計算できなかった表は採用へ進ませない(指紋`None`の表は陳腐化検出で
        // 永久に保護されないため、`Rejected(FingerprintUnavailable)`にして退避ファイルへ書く)。
        let judgement = gate_on_fingerprint(
            judge_score(&score, tip_at_start, reconciliation.as_ref()),
            fingerprint,
        );
        let (cell_count, write_result) = persist_judged_table(
            cells,
            ScoredVerification {
                score,
                seed: walk_seed,
            },
            judgement,
            probe_env_version(tip_at_start, process_start),
            fingerprint,
        );
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
        // 書き込み失敗時は失敗理由が標準エラー最終行に残るよう、警告で上書きしない
        // (awase-settingsは標準エラーの最後の非空行を失敗理由として表示する)。
        if decode_errors > 0 && write_result.is_ok() {
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

        /// B-5回帰テスト: 読み手が表現できないキー(文字キー0x41、`KEYS`の添字13)の
        /// セルは書かない。書くと`coverage_ratio`の分母の約1/14が常に変換不能になる。
        #[test]
        fn build_persisted_cells_omits_keys_the_reader_cannot_represent() {
            let text_key_idx = KEYS.iter().position(|&vk| vk == 0x41).expect("KEYSに0x41");
            let mut table = Table::new();
            table.record(st(true, 0x09), text_key_idx, None, out(true, 0x09));
            table.record(st(true, 0x09), 1, None, out(false, 0));
            let cells = build_persisted_cells(&table);
            assert_eq!(cells.len(), 1, "0x41のセルは書かない");
            assert_eq!(cells[0].key, KeyId(0x1C));
        }

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
            // 指紋配線後の学習プロセスが書く表(指紋あり)。指紋なしは専用テストで扱う。
            table.fingerprint = Some(awase_keymap_learn::persist::Fingerprint(1, 2));
            table
        }

        /// 06以前の学習プロセスが書いた退避ファイル(指紋None・要確認)は、採用操作だけで
        /// 本体へ昇格しない。ファイルも書き換えない。
        #[test]
        fn adopt_pending_judgement_at_refuses_needs_confirmation_without_fingerprint() {
            let pending = temp_table_path("nofp_pending");
            let table_path = temp_table_path("nofp_table");
            let _ = std::fs::remove_file(&table_path);
            let mut table = sample_table(Some(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::UnverifiedMsImeNative,
            )));
            table.fingerprint = None;
            std::fs::write(&pending, table.to_json().unwrap()).unwrap();

            let result = adopt_pending_judgement_at(&pending, &table_path);

            assert_eq!(result.unwrap_err().code(), "no_fingerprint");
            assert!(!table_path.exists(), "本体へ昇格しない");
            assert!(pending.exists(), "退避ファイルは残す");
            let _ = std::fs::remove_file(&pending);
        }

        /// 採用は指紋を保つ(書き換えるのは判定だけ)。
        #[test]
        fn adopt_pending_judgement_at_keeps_the_fingerprint() {
            let path = temp_table_path("keeps_fp");
            let table = sample_table(Some(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::UnverifiedMsImeNative,
            )));
            std::fs::write(&path, table.to_json().unwrap()).unwrap();
            let none = temp_table_path("keeps_fp_no_pending");
            let _ = std::fs::remove_file(&none);

            adopt_pending_judgement_at(&none, &path).unwrap();

            let reloaded = from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(reloaded.fingerprint, table.fingerprint);
            let _ = std::fs::remove_file(&path);
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

            let none = temp_table_path("accepts_no_pending");
            let _ = std::fs::remove_file(&none);
            let result = adopt_pending_judgement_at(&none, &path);

            assert!(result.is_ok(), "expected success, got {result:?}");
            let reloaded = from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(reloaded.judgement, Some(TableJudgement::Accepted));
            let _ = std::fs::remove_file(&path);
        }

        /// 実機windows-latestで見つかった不具合の回帰: 要確認の表は退避ファイル(pending)にあり、
        /// `table_path`は無い。採用すると`table_path`へ採用済みで昇格し、pendingは消える。
        /// もう一度実行しても(pendingは無く`table_path`が採用済み)成功する。
        #[test]
        fn adopt_pending_judgement_at_promotes_pending_attempt_to_table() {
            let pending = temp_table_path("promote_pending");
            let table_path = temp_table_path("promote_table");
            let _ = std::fs::remove_file(&table_path);
            let table = sample_table(Some(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::UnverifiedMsImeNative,
            )));
            std::fs::write(&pending, table.to_json().unwrap()).unwrap();

            let first = adopt_pending_judgement_at(&pending, &table_path);
            assert!(first.is_ok(), "expected success, got {first:?}");
            let promoted = from_json(&std::fs::read_to_string(&table_path).unwrap()).unwrap();
            assert_eq!(promoted.judgement, Some(TableJudgement::Accepted));
            assert!(!pending.exists(), "昇格後はpendingを消す");

            let second = adopt_pending_judgement_at(&pending, &table_path);
            assert!(second.is_ok(), "再実行は冪等に成功する: {second:?}");
            let _ = std::fs::remove_file(&table_path);
        }

        /// pendingが不採用の表なら、`table_path`(以前の良い表)は書き換えず、pendingも残す。
        #[test]
        fn adopt_pending_judgement_at_rejected_pending_keeps_existing_table() {
            let pending = temp_table_path("rejected_pending");
            let table_path = temp_table_path("rejected_pending_table");
            let good = sample_table(Some(TableJudgement::Accepted));
            std::fs::write(&table_path, good.to_json().unwrap()).unwrap();
            let bad = sample_table(Some(TableJudgement::Rejected(RejectedReason::LowAccuracy)));
            std::fs::write(&pending, bad.to_json().unwrap()).unwrap();

            let result = adopt_pending_judgement_at(&pending, &table_path);

            assert_eq!(result.unwrap_err().code(), "rejected");
            let reloaded = from_json(&std::fs::read_to_string(&table_path).unwrap()).unwrap();
            assert_eq!(reloaded, good, "以前の良い表は失われない");
            assert!(pending.exists(), "不採用のpendingは残す");
            let _ = std::fs::remove_file(&table_path);
            let _ = std::fs::remove_file(&pending);
        }

        /// 安全弁: 不採用(低正答率)のファイルは、書き換え要求があっても変更されない
        /// (エラーを返し、ディスク上の内容もそのまま)。
        #[test]
        fn adopt_pending_judgement_at_leaves_rejected_file_untouched() {
            let path = temp_table_path("rejected");
            let table = sample_table(Some(TableJudgement::Rejected(RejectedReason::LowAccuracy)));
            std::fs::write(&path, table.to_json().unwrap()).unwrap();

            let none = temp_table_path("rejected_no_pending");
            let _ = std::fs::remove_file(&none);
            let result = adopt_pending_judgement_at(&none, &path);

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

            let none = temp_table_path("missing_pending_never_created");
            let _ = std::fs::remove_file(&none);
            let result = adopt_pending_judgement_at(&none, &path);

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
                AdoptFailure::Rejected(AdoptRejected::NoFingerprint),
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
    app::entry();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("awase-keymap-learn-win is Windows-only");
}
