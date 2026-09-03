# ADR-123: `pending_deferred` の flush ガードが GJI reinit-retry 完了しか見ていないため、reinit 完了を待つ間に到着した別モーラが独立 probe で追い越し、deferred VK が確定済みセッションの後ろに取り残されて出力順が入れ替わる

## ステータス

**round 1・round 2（Opus 2体、architect/premortem）完了。round 2 終了時点では
「decision を選ぶ段階に到達していない」という結論だったが、architect が
round 2 の最後に提示した2点の検証項目（app.log の reinit 発火有無 / deferred
flush と「ば」probe 完了の前後関係）を、`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`
の `app_log_excerpt`（`log::` 生ログ、journal では追えなかった層）を直接読む
ことで両方とも確定させた。これにより根本原因は「推測」から「確定」へ格上げ
され、decision も具体化した。ユーザー指示の2ラウンド後の追加確認として本版
（round 2.5）を記録する。**

[GitHub issue #148](https://github.com/cuzic/awase/issues/148) として追跡。

## 確定した根本原因（app.log 生ログのタイムスタンプ順再構成、`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`）

以下は `wrangler r2 object get` で取得した report JSON の `payload.app_log_excerpt`
（`log::` 出力そのもの、journal とは別フィールド）を時刻順に読んだ結果で、
推測を含まない一次証拠である（該当行はタイムスタンプ `02:42:53.606`〜
`02:42:54.129`、約523ms の窓）。

1. **`02:42:53.668`** 「た」の1文字目('T', cold=235) が per-VK confirm で
   `suspected literal` → backspace ×1 + 再送 "ta" が予約される
   （`[raw-tsf-literal] cold=235 raw TSF literal suspected → backspace ×1
   + re-send "ta" scheduled`）。
2. **`02:42:53.670`** probe(442→443) の切替中、drain キュー（`queue_len=6`）
   がまとめてリプレイされる。backspace 送信 → "ta" 再送（cold=236 の新しい
   probe443 を開始）。
3. **`02:42:53.680`**「と」が engine を通過し `send_keys` で romaji "to"
   に変換されるが、**probe443 が in-flight のため** `[tsf] probe in flight
   → deferred 2 VK(s) for "to"` — **`pending_deferred` に T,O の2VKが積まれる**。
4. **`02:42:53.681`**「え」も同様に engine を通過し romaji "e" に変換される
   が、probe443 がまだ in-flight のため `[tsf] probe in flight → deferred
   1 VK(s) for "e"` — **`pending_deferred` に E が追加され、計3VK
   （T,O,E）が滞留する。**
5. **`02:42:54.006`**「た」の再送（cold=236）も per-VK confirm で
   **2回目の** `suspected literal`（`consecutive=2`）→ **give-up**
   （`giving up, backs=1 cleanup only (no re-send)`）。
6. **`02:42:54.007`** probe443 が終了（`ProbeDone`）→ backspace ×1 送信。
7. **`02:42:54.013`** **`[chrome-reinit] cold=236 VK_IME_OFF→VK_IME_ON
   強制リセット送信 + IMC ポーリング開始`** — give-up は実際に GJI reinit
   を送信していた（round 1 版の「reinit 不発火」判定は誤り、round 2 の
   訂正が正しかったことが確定）。
8. **`02:42:54.019`** **`[raw-tsf-literal] skip stale deferred flush while
   GJI reinit retry is polling: result=StartedRetryPolling { poll_token: 7 }`**
   — reinit のポーリングが終わるまで、`pending_deferred`（T,O,E の3VK）の
   flush は意図的にスキップされる。**この時点で `discard_raw_recovery_if_focus_stale`
   は発火していない**（focus は変化していない）。round 2 の「focus churn で
   全破棄」仮説（第一仮説としていた `discard_raw_recovery_if_focus_stale`）は
   **本インシデントでは不成立と確定した**。
9. **`02:42:54.019`〜`.022`**「ば」の物理キーが drain される
   （314/278/181/158ms の追加遅延を伴う、事実2で測定した遅延バーストの
   正体）。
10. **`02:42:54.022`**「ば」が engine を通過し romaji "ba" に変換される。
    **ここで `[tsf] probe in flight → deferred` ログが出ない** ——
    probe443 は既に `ProbeDone` で終了しており `has_pending_tsf()` が
    false のため、「ば」は deferred されず**新しい独立 probe444（cold=237）
    を開始する。**
11. **`02:42:54.083`** probe444 で「ば」の1文字目('B') が
    `candidate SHOW` を伴って正常に confirm される。
12. **`02:42:54.086`** reinit のポーリング（手順7で開始）が完了
    （`IMC poll #1: conv=0x00000019 NATIVE=true write_delta=+315B` →
    `status=Confirmed`、`origin_focus_gen=431 == current_focus_gen=431`）。
    完了トリガーで **「た」の再送が `retry romaji via normal path: "ta"`
    として発行される**（3回目の "ta" 送信）。
13. **`02:42:54.086`〜`.089`** この「た」再送は、**ちょうど今 in-flight な
    probe444（元は「ば」用）に相乗りする形**で `vks=[54,41]`（'T','A'）
    として送信・confirm される（`[tsf-transmit] cold=237 romaji="ta" →
    vk-run`）。
14. **`02:42:54.103`**「ば」の2文字目('A') も confirm され、
    `per-VK: 全 2 VK 確認済み → セッション確認`（session_marked）。
    **この直後**、`[tsf-probe] deferred 3 VK(s) を romaji 直後に送出 (Tsf)`
    ——手順3-4で滞留していた `pending_deferred`（T,O,E）が、**「ば」と
    「た」(3回目)がどちらも確定した後に**、ようやく flush される。

**確定した機序**: 「た」の give-up が予約した GJI reinit のポーリング完了を
待つ間、`pending_deferred`（と・え、3VK）は正しく flush を保留された
（手順8、これ自体は意図通りの安全策）。しかし**この保留期間中に到着した
「ば」は、`pending_deferred` の存在を一切考慮しない別経路（`has_pending_tsf()`
のみを見る `defer_if_probe_in_flight`）を通り、独立した新しい probe を
起動して先に確定してしまう**。さらに reinit 完了トリガーで「た」の再送が
その「ば」の probe に相乗りして追加確定される。結果、`pending_deferred` に
先に入っていた「と」「え」が、後から来た「ば」「た」に**追い越され**、
最終的な flush（手順14）は両者が確定した後になる。**この「追い越し」こそが
文字脱落・順序入替の直接原因であり、GJI 内部の解釈やモーラ融合の推測は
不要だった。**

出力の最終形（「ばたと」、「え」消失）は、この追い越し順序（ば→た→と+え）と
整合する。「え」が完全に消えた理由は本ログ窓の外（`02:42:54.129` 以降）に
あると見られ、`pending_deferred` の flush（手順14、raw な生 VK 送信で probe
を経由しない）が、既にセッション確認済みの composition に対してどう作用
したかは GJI 内部の挙動に依存し確定できないが、**出現順序そのものはこれで
完全に説明できる。**

### round 1/2 の各仮説との決着

| 仮説 | 判定 |
|---|---|
| round 0: 「え+ば が `pending_deferred` で融合し単一の巨大合成イベントになった」 | **不成立（確定）**。融合ではなく「追い越し」。write_delta=315 は「ば」自身の confirm 値（正しい解釈だった、`[chrome-reinit] cold=236 ... write_delta=+315B` のログは実は reinit の IMC polling 側の値であり、`LiteralDetect` の write_delta=315 とは別物と判明——両者はたまたま近い値だが無関係） |
| round 1 F-1: cold-start は word-level `LiteralDetectFsm` ではなく per-VK confirm | **確定**（`path=PerVk, target=Tsf`） |
| round 2 R2-2: 「journal に reinit イベントが無い」は false negative | **確定的に正しかった。reinit は実際に発火した**（手順7） |
| round 2 R2-4: `discard_raw_recovery_if_focus_stale` が第一仮説 | **不成立と確定**。focus_gen は変化しておらず discard 経路は通っていない |
| round 2 R2-3: 「deferred flush は『ば』の probe 完了より後に来る」 | **確定（architect の予測が正確に的中）**。ただし機序は「backspace/reinit 待ちで単純に遅延した」のではなく「`pending_deferred` の存在を無視して『ば』が追い越した」 |

## 確定した真因

**`defer_if_probe_in_flight`（`output/mod.rs:1277-1290`）およびそれに準ずる
gating が `has_pending_tsf()`（TSF probe オブジェクトの生存）だけを見ており、
`pending_deferred` が非空である（＝まだ flush されていない先行モーラの VK が
存在する）ことを考慮しない。** そのため、probe が一旦終了（give-up 等で
`ProbeDone`）した後、その `pending_deferred` がまだ flush されていない
（reinit retry 等の後続処理待ちで保留中の）状態でも、次に届いたモーラは
「probe は in-flight ではない」と判定されて即座に独立した新しい probe を
開始でき、結果として `pending_deferred` の中身を追い越して先に確定する。

### 関係ファイル・関数一覧（確定版）

| 役割 | file:line |
|---|---|
| gating の欠落箇所（`has_pending_tsf()` のみで `pending_deferred`/`raw_recovery_owns_deferred()` を見ない） | `crates/awase-windows/src/output/mod.rs:1277-1290`（`defer_if_probe_in_flight`） |
| `pending_deferred` 実体 | `crates/awase-windows/src/output/tsf_warmup_coord.rs:51-57,296-337` |
| `raw_recovery_owns_deferred`（reinit 予約中かの判定、既存だが `defer_if_probe_in_flight` からは参照されていない） | `crates/awase-windows/src/output/mod.rs:1400-1417` |
| give-up → GJI reinit 予約・ポーリング | `crates/awase-windows/src/output/probe_io.rs:766-782`, `crates/awase-windows/src/output/probe_io.rs:167-` |
| reinit ポーリング完了 → romaji 再送のトリガー（既存 probe への相乗り） | `chrome-reinit-retry completion` 経路（`platform.rs` 内、ログ行 `[chrome-reinit-retry] retry romaji via normal path`） |
| deferred flush 実行（`[tsf-probe] deferred N VK(s) を romaji 直後に送出`） | `crates/awase-windows/src/output/key_injector.rs:302-313` |
| TSF cold-start の per-VK confirm 経路 | `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs:172-181` |

## BUG-38 との異同

**BUG-38 とは異なる、しかし近縁の欠陥。** BUG-38 は「同一 probe の give-up
サイクル内で `pending_deferred` を flush し忘れる」ことを塞いだ。本件は
「`pending_deferred` が flush 待ちの間、**別の独立した send がそれを一切
考慮せず追い越せる**」という、flush 忘れとは異なる gating の抜け穴。
BUG-38 の修正（`flush_stale_deferred_vks_after_recovery`）自体は正しく
機能している（手順14で実際に flush されている）。抜けているのは
「flush されるまでの間、新規 send を止める」側のガードである。

## 決定

**`defer_if_probe_in_flight`（および同種の gating 判定）の条件を
`has_pending_tsf() || raw_recovery_owns_deferred() || !pending_deferred.is_empty()`
に拡張し、`pending_deferred` に未flushの VK が残っている間は新しいモーラも
同じキューに追記する（独立した新規 probe を開始させない）。** これにより
「ば」のようなモーラが `pending_deferred` を追い越すこと自体を構造的に
防ぐ。

- **長所**: 症状の直接原因（追い越し）をピンポイントで塞ぐ。`pending_deferred`
  の flush 順序自体（BUG-38 が保証した部分）は変更不要。選択肢D（round 2 で
  却下）のような probe 連鎖の抑制や、選択肢A（モーラ境界の導入）のような
  `SendInput` 分割も不要——`pending_deferred` に積む条件を直すだけなので、
  round 2 で懸念された「単一 SendInput のメッセージポンプ保護喪失」
  （BUG-02 系 race 再燃リスク）を新設しない。
- **短所/リスク**: gating を強めることで、`pending_deferred` が長時間
  flush されないケース（reinit retry が何らかの理由で長引く等）では、
  後続モーラの入力が今まで以上に長く待たされる可能性がある。実測が必要
  （`tuning-constants.md` 規約対象ではないが、レイテンシ影響は要確認）。
  また、この変更が `pending_deferred` の生存期間そのものを延ばすわけではない
  ため、`discard_raw_recovery_if_focus_stale`（focus 変化時の全破棄）が
  **他のインシデントで**発火した場合の挙動（本件では発火しなかったが、
  経路自体は実在する）への影響は別途確認が必要。
- **選択肢E（順序トークンによる検出）との関係**: 本 decision は「追い越しを
  未然に防ぐ」予防策であり、E が提案した「検出してログに残す」観測策とは
  補完関係にある。E の最小版（`pending_deferred` に順序情報を持たせ、
  flush 時に破れがあれば `log::error!` する）は、本 decision の実装後も
  「万一 gating の見落としがあった場合の安全網」として価値があるため、
  **軽量な形で併設することを推奨**する。

## 未決定事項

1. **実装後のレイテンシ影響の実測**: gating 強化で後続モーラの待ち時間が
   伸びるケースがどの程度あるか。
2. **`FOCUS_RESYNC` gate が2打鍵目で事実上解除される件**（round 1 F-2）は
   本ADRのスコープ外の別欠陥として引き続き切り出しを推奨。
3. **`discard_raw_recovery_if_focus_stale` 自体の経路**（本件では不発火
   だったが実在する）は、focus churn が実際に reinit retry 中に起きる
   別インシデントで別途検証が必要——本 decision の gating 拡張後も、
   `pending_deferred` ごと破棄されるケース自体は残るため、根絶ではなく
   緩和である点に注意。
4. Unicode 側の同型フラットバッファ2箇所（`UnicodeColdWarmupFsm::deferred_chars`、
   `Output::unicode_cold_deferred`）は「同型だが未観測」として
   `docs/known-bugs.md` に別記録し、本ADRのスコープからは外す。
5. **回帰テストの置き場所**: `defer_if_probe_in_flight` は純粋関数に近い
   判定ロジックを含むため、`output/tsf_warmup_coord.rs` の既存ユニット
   テスト、または `.claude/rules/fix-requires-evidence.md` に従い
   `crates/awase-windows/tests/golden_scenarios.rs` への追加を検討する。

## 関連

BUG-38、BUG-89（`FOCUS_RESYNC`/`OUTPUT_GATE` の deferred replay 経路）、
BUG-45/BUG-75/ADR-122（per-VK confirm の代理指標ベース判定の構造的欠陥）、
[docs/bug-reports-triage.md](../bug-reports-triage.md)
（`01M1JJD54XQXSEJTHHFKV1WKA1` 該当行）。
