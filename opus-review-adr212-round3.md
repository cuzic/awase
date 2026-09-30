# ADR-212 v3 Opus レビュー round3(2026-09-30、対象 `69595d75`、PR #400)

## 結論: 収束(新しい Major なし)

| 指摘 | v3 | 判定 |
|---|---|---|
| M-N1 `set_ime_open` はデッドではない | 背景を訂正し、P1 から外した(P1 の対象は `send_eager_tsf_warmup(off)`・`WarmupImeOn::off()`・`WarmupOrigin::Off`) | 満たす |
| M-N2 #400 の不足 | `[set-open] origin open generation outcome`。sync は outcome を同じ行に、async は `outcome=async`+generation。settle で落とした SetOpen と C2 は既存のログで数える | 満たす(下の m9 は運用上の注意) |
| M-N3 止める場所 | A(Engine で出さない、variant ごと削除)を採用し、B を採らない理由も記載。決定7を型での固定+件数ガードに | 満たす |
| M-N4 C2 の性質 | 「書かないのに pending と抑制窓を立てる経路」に訂正 | 満たす |
| m7 件数ガードの限界 | 「検出の仕掛けで証明ではない。(a) は `drift_correction.rs` の条件を単体テストで固定」 | 満たす |
| m8 (c) は所有者に確認 | P6 の行に追記 | 満たす |

#400 の差分を確認した。`dispatch_ime_set_open` が `None` を返すのは async の分岐(`executor.rs` の `spawn_local` の後)だけで、NotOwned の早期 return も `Some` で outcome が出る。key 経路と RefreshState 経路の両方が `dispatch_effect` を通る。計測には足りる。

## Minor(実装・計測のときの注意。ADR の修正は任意)

- **m9. async の突き合わせが効かない場合がある**: `execute_from_loop` が渡す `generation` は `ime.model().pending_generation()`(`executor.rs:248`)。RefreshState 経路の ActivationSync は `handle_engine_activation_sync` を通らず、`ImeApplyRequested` を立てない。そのため `generation` が `None`、または**直前の別の要求の値**になりうる。
  該当するのは ImmCross が先の窓(Standard=読める窓)の async だけで、P2 が一番気にしている TsfNative × GJI(sync の GjiDirect)には影響しない。数えるときは「`generation=None` または重複する async の行は、時刻順に `open_chain` の完了ログと手で合わせる」と、計測の手順メモに書いておけば足りる。
- **m10. C3 の「不要になる」の前提**: `strip_ime_set_open_if_settling` は origin を見ずに**全ての** `SetOpen` を落とす(ExplicitUserAction も含む)。したがって P2 の後も、settle 中に押されたユーザーの IME キー由来の `SetOpen` のために `schedule_settle_retry` が要りうる。
  決定5の「他の用途を確認してから」で押さえられているので、P2 の PR で「ExplicitUserAction が settle で落とされたときの扱い」を確認項目に入れること。

実装に進んでよい: P1(#399)、P2 の計測(#400)、P2 の実装(決定5の A、計測結果を見てから)、P3 以降(v3 の段と条件のとおり)。
