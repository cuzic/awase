# タイミング定数の変更規約（`tuning.rs`）

paths: `crates/awase-windows/src/tuning.rs`

## ルール

`crates/awase-windows/src/tuning.rs` の **タイミング定数**（probe の min/max、idle 閾値、
warmup 待機、settle grace など、`_MS` 系の定数）を変更するコミットは、本文に
**実測値** を含めること。

必須で書く内容:

1. **測ったもの**: 何を計測したのか（例:「fresh F2 送信から GJI I/O 開始までの時間」
   「Chrome の TSF composition context 再初期化にかかる時間」）
2. **数値**: 実測 ms（可能なら idle 秒数などの条件付き。例:「~8.7s idle 後 cold=7 で 325ms」）
3. **導出**: その実測から定数値をどう決めたか（例:「実測最大 ~180ms + マージン 170ms = 350ms」）

「効かないので増やした」だけの変更は禁止。値を動かす前に、その定数が支配している
待機／期限が **実際に何 ms 必要か** を実機で測る。

## 良い前例（実在コミット）

- **`79134f5`**（`fix(tuning): Chrome F2NonTsf + GJI long idle で「という→toいう」`）:
  「Chrome の TSF composition context 再初期化に **~326ms** かかることを実測で確認。
  keyboard idle が 1000ms と短く long_idle=false → probe_min_ms=20ms が選ばれ、
  probe 初回 tick(~305ms) で max_deadline 超過 → 308ms 時点で T+O 送信 → Chrome 未準備
  (ready=326ms) でリテラル `to` 出力」と、**症状・実測・因果・対策定数（350ms）** が
  すべて本文にある。さらに [docs/known-bugs.md](../../docs/known-bugs.md)（BUG-02 修正履歴）
  にも実測 ~326ms 付きで残してある。この水準が目標。
- **`b101153`**（`こ→ko` 修正）: `CHROME_PROBE_LONG_IDLE_MIN_MS` 100→200ms。本文に
  「Chrome の再初期化に ~114ms、probe 起点が F2 送信より ~7ms 早いため実効 ~102ms で
  間に合わず、200ms で実効 ~193ms を確保」と、なぜその値かの実測根拠がある。
- **`9a7e699`**（`bあ` 部分リテラル修正）: `GJI_LONG_IDLE_PROBE_TOTAL_MS` 150→350ms。
  「F2×2 後 GJI が VK 受付可能になるまで **実測 181ms**、150ms タイムアウトが早すぎた。
  実測最大 ~180ms + マージン 170ms = 350ms」と導出付き。

## 避けるべきパターン: 同じ定数ファミリーの盲目的エスカレーション

Chrome probe の最小待機は、別々のバグ修正のたびに **同じ役割の定数が段階的に釣り上がって
きた**（`git log --follow -- crates/awase-windows/src/tuning.rs` で確認）:

```
CHROME_PROBE_MIN_MS = 20        （c74a7ba, timing.rs 統合時）
CHROME_PROBE_LONG_IDLE_MIN_MS = 100   （6fd0ca2, らいねんも→raいねんも）
                            → 200      （b101153, こ→ko）
CHROME_PROBE_F2_GJI_IDLE_MIN_MS = 350  （79134f5, という→toいう）
```

上記コミットは幸い **各段で実測を添えていた**（守るべき良い習慣）。それでもなお、
同じ「Chrome が準備できるまで待つ」目的の定数が **20→100→200→350ms** と 5 週間で
4 段釣り上がった事実は注意信号である。値を上げる前に必ず問うこと:

- 今回の不足は本当に「待ち時間が足りない」のか、それとも **probe の起点や発火条件が
  ずれている** のか（`79134f5` は probe 起点が F2 送信より早い＝計測の基準点ズレが真因で、
  値を上げるのは対症だった。`3c275a7` は context 失効を検出して F2 を再送する＝別軸の修正）。
- マージンを足すだけで別の環境（別アプリ・別 idle）に副作用が出ないか。
- 実測せずに「とりあえず倍にする」形の変更（実測なしのエスカレーション）は禁止。
  レビューで本文に ms の実測値が無ければ差し戻す。

## なぜこのルールが必要か（背景）

タイミング定数は「増やせば大抵直る（が、レイテンシが悪化し、別の spurious を誘発しうる）」
ため、実測なしに雪だるま式に膨らみやすい。実際に IME OFF キー選択では、レイテンシを
短縮する変更（`534051a`）が spurious OFF を露出させ revert された（`098c663`,
[docs/experiments.md](../../docs/experiments.md) 参照）。値そのものではなく **何 ms 必要かの
実測** をコミットに残すことで、後から「この 350ms はどの計測に基づくのか」を検証・再調整
できる。関連: [experiment-logging](./experiment-logging.md)、[fix-requires-evidence](./fix-requires-evidence.md)。
