//! IMEキー効果の学習(較正)の巡回プランナとシミュレータ(ADR-191)。
//!
//! 実機のIMEを「観測できる部分(status)と観測できない部分(隠れ状態)を持つMealy機械」とみなし、
//! (status, キー)→結果 の表を少ない押下・少ない待ちで埋める巡回を計画する。
//! OS非依存の純粋ロジック(Windows APIもVKコードも持たない)で、実機なしにLinuxでテストできる。
//! キーは抽象ID(`KeyId`)で表し、実機のVKとの対応は呼び出し側(ドライバ)が持つ。
//!
//! 構成: `model`(真のモデル) / `sim`(実機の代わり) / `cost`(待ちモデル) / `anomaly`(異常とリセット段階) /
//! `graph`(プランナ用グラフ・巡回計画) / `table`(観測表) / `exec`(実行器) / `strategy`(S0〜S7) /
//! `metrics`(指標) / `models`(合成モデルとATOK風モデル)。

pub mod anomaly;
pub mod cost;
pub mod exec;
pub mod graph;
pub mod metrics;
pub mod model;
pub mod models;
pub mod rng;
pub mod sim;
pub mod strategy;
pub mod table;
