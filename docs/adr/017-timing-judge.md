# ADR-017: TimingJudge によるタイミング判定の集中化

## ステータス

採用

## コンテキスト

同時打鍵のタイミング判定（`elapsed < threshold`）が NicolaFsm の複数の step_* メソッドに散在していた。n-gram による閾値調整、3キー仲裁のスコアリング、投機出力の判定がそれぞれ別の場所にあり、「どういう計算式でどう判定しているか」がコードを追わないとわからなかった。

## 決定

`TimingJudge` 構造体に全タイミング判定を集約する。

### 3つの判定メソッド

```rust
struct TimingJudge<'a> {
    threshold_us: u64,
    ngram_model: Option<&'a NgramModel>,
    recent_kana: Vec<char>,
}
```

**`is_simultaneous(pending_ts, new_ts, candidate_kana) → bool`**

2キー判定。elapsed が閾値内かを返す。n-gram モデルがあれば閾値を動的調整する。

**`three_key_pairing(char1_ts, thumb_ts, char2_ts, ...) → ThreeKeyResult`**

3キー仲裁。以下の3段階で判定:
1. n-gram なし → 純粋なタイミング比較（d1 < d2）
2. タイミング差が大きい（30%マージン超）→ タイミング優先
3. タイミングが接近 → n-gram スコアで判定（パターン A vs B）

**`should_speculate(normal_kana, left_kana, right_kana) → bool`**

投機出力判定。通常面と親指面の n-gram スコア差が 0.5 を超えたら投機する。

### 名前付き定数

```rust
TIMING_MARGIN_PERCENT = 30      // 3キー仲裁のマージン
SPECULATIVE_SCORE_THRESHOLD = 0.5  // 投機出力のスコア差閾値
NGRAM_CONTEXT_SIZE = 3          // n-gram コンテキスト長
```

### n-gram 閾値調整の計算式

```
threshold = clamp(base + tanh(score) * range, min, max)
```

- 正のスコア（よく出現するペア）→ 閾値を広げる（同時打鍵と認識しやすい）
- 負のスコア（まれなペア）→ 閾値を狭める
- tanh で調整量を ±range に収束させる（極端なスコアで暴走しない）

## 結果

- NicolaFsm の step_* メソッドは `self.timing_judge().is_simultaneous(...)` と1行で判定
- 計算式と定数が timing.rs に集約（読み解きやすい）
- `adjusted_threshold_us()` と `should_pair_with_char1()` を NicolaFsm から削除
