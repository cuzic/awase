//! IMEキー効果の学習(状態×キー→効果の表を作る)の巡回プランナとシミュレータ(ADR-191)。
//! ADR-176 の較正UI(ユーザーが押して確定する)とは別物。
//!
//! 実機のIMEを「観測できる部分(status)と観測できない部分(隠れ状態)を持つMealy機械」とみなし、
//! (status, キー)→結果 の表を少ない押下・少ない待ちで埋める巡回を計画する。
//! OS非依存の純粋ロジック(Windows APIもVKコードも持たない)で、実機なしにLinuxでテストできる。
//! キーは抽象ID(`KeyId`)で表し、実機のVKとの対応は呼び出し側(ドライバ)が持つ。
//!
//! 構成: `model`(Mealy機械の型) / `sim`(実機の代わり) / `cost`(待ちモデル) / `anomaly`(異常とリセット段階) /
//! `graph`(プランナ用グラフ・巡回計画) / `table`(観測表) / `exec`(実行器) / `strategy`(S0〜S9、定義は同モジュールのdoc) /
//! `metrics`(指標) / `sample_models`(具体的なモデル例=合成ランダムモデルとATOK風モデル) /
//! `verify`(段階2: 独立ランダムウォークでの自己検証、誤りに強い分類) /
//! `persist`(学習結果の永続化フォーマット、ADR-195段階3) / `staleness`(学習済み表の陳腐化検出、
//! ADR-195段階8) / `minimize`(段階5: 隠れ状態を最小のMealy機械として求めるpartition refinement) /
//! `external_write`(学習窓への外部からの書き込みの直接観測、ADR-196決定1b) /
//! `judgement`(学習結果の採否判定、ADR-196決定1a・1b項目7〜9・1e) /
//! `remeasure`(内蔵表と食い違ったセルの再測定、ADR-196決定1b項目7〜8)。

pub mod anomaly;
pub mod cost;
pub mod exec;
pub mod external_write;
pub mod graph;
pub mod judgement;
pub mod metrics;
pub mod minimize;
pub mod model;
pub mod persist;
pub mod remeasure;
pub mod rng;
pub mod sample_models;
pub mod sim;
pub mod staleness;
pub mod strategy;
pub mod table;
pub mod verify;
