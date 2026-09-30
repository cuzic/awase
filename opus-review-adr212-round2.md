# ADR-212 v2 Opus レビュー round2(2026-09-30、対象 `9544ac10`、PR #400 の差分)

## 結論

**収束は保留(新しい Major が1つ。原因は私の round1 の誤り)**。
- **M-N1**: P1 に足した「`Platform::set_ime_open` はデッドコード」は誤りで、**B1 drift correction の ImmCross の書き込み経路そのもの**。P1 から外すこと。
- **中身**: それ以外の round1 の指摘は、意図どおり反映されている。残りは PR #400 の計測の不足(M-N2)と、決定5の「どこで止めるか」の未決(M-N3)。どちらも ADR の方針を変えるものではない。
- **進め方**: M-N1 を直せば、P1(set_ime_open を除く)は進めてよい。#400 は M-N2 の2点を足してから計測に使うこと。P2 の実装は M-N3 を決めてから。
- **次のラウンド**: 私の側で確認し直すのは M-N1〜N3 の反映だけなので、次は短い確認で足りる。

---

## (1) round1 の反映状況

| 指摘 | v2 | 判定 |
|---|---|---|
| B1 計測が取れない | 決定4で、今の journal では取れないと明記。#400 で `[set-open] origin=` を出し、outcome 別と SendInput の目印で数える | 方針は満たす。#400 の実装は不足(M-N2) |
| B2 belief 側を残すと危ない | 決定5で pending・抑制窓・`on_set_open_requested` をまとめて落とす | 満たす。ただし「どこで」が未決(M-N3) |
| B3 BUG-170 型の取り残し | 決定5の検証に入れた。`Reopen` の発火元を足す判断も含めた | 満たす |
| M1 ADR-191 の改訂 | 決定6に明記 | 満たす |
| M2 idle-conv-check の直接呼び出し | C2 として記載 | 記載は満たす。性質の記述が違う(M-N4) |
| M3 settle の再試行 | C3 として記載 | 満たす |
| M4・M5 B1 の3分類 | (a) 許可の再試行 (b) 古い desired (c) キャッシュの押し付けに分け、(a) を P6 まで残す | 満たす。(a) と (b)(c) は、既存の閾値の条件(`explicit_intent == Some(desired) && last_intent.is_some()`、`drift_correction.rs`)で実装上も分けられる |
| M6 2026-08-04 の誤読 | 背景と代替案で訂正。(i) nonaiyo の補正、(ii) 予測の自己成就を明記 | 満たす |
| M7 P4 の運用 | フラグの PR と恒久化の PR の2つ。目印の件数を合格条件に入れた。P2 の後に行う | 満たす |
| M8 完了条件の固定 | 関数名の lint の限界を明記し、architecture_guard の件数で固定 | 満たす。M-N3 と連動する(下) |
| m1 `Platform::set_ime_open` | P1 に追加 | **誤り(M-N1)**。これは私の round1 の誤り |
| m2 ESC+BS の回収 | 決定2に理由と境界を明記 | 満たす |
| m3 強制 Exit | 決定2で「残す」と明記 | 満たす |
| m4 P5 は P2 の後の前提で | 記載 | 満たす |
| m5 P0 の測った範囲 | 記載 | 満たす |
| m6 完了条件の範囲 | 開閉軸だけと明記 | 満たす |

---

## (2) v2 で新しく入った誤り・矛盾

### M-N1(Major). `Platform::set_ime_open` はデッドコードではない(round1 m1 の撤回)
`crates/awase-windows/src/platform.rs:1522-1536` の `set_ime_open_ordered` は、warrant を通した後に **`PlatformRuntime::set_ime_open(self, open)`** を完全修飾の構文で呼ぶ(L1536)。
この `set_ime_open_ordered` は、B1 drift correction の ImmCross の分岐から呼ばれる(`ime_refresh.rs:952`。`architecture_guard.rs` の `.set_ime_open_ordered(` は1件と固定)。
`architecture_guard.rs:1458` の `.set_ime_open(` の0件は**メソッド呼び出しの形だけ**を数えている。完全修飾の呼び出しを見落としていることは、`lints/actuation_call_guard/src/lib.rs:60-66` のコメントが明記しており、lint 側は `("set_ime_open", &["set_ime_open_ordered"])` として許可している。
私は round1 でこのガードの0件を根拠に「デッド」と書いたが、誤りだった。

- P1 から `Platform::set_ime_open` を外すこと。背景の「デッドコード」の行と P1 の行も直すこと。
- **PR #399 には入れないこと**。入れると ImmCross の窓(Standard)の drift correction が書けなくなる。P6 で (a) を残す以上、この経路は (a) の再試行にも使われうる。
- 整理したいなら、P6 で B1 の形が決まった後に、`set_ime_open_ordered` へ中身を移す**リファクタ**として扱う(挙動は変えない)。あわせて `architecture_guard` の `.set_ime_open(` の0件ガードに「完全修飾の呼び出しは数えない」ことを注記すると、同じ誤読を防げる。

### M-N2(Minor〜Major、計測の信頼性). PR #400 のログだけでは突き合わせが不安定
#400 は `dispatch_effect` の入口(`executor.rs:661`)に `[set-open] origin={origin:?} open={open}` を足すだけ。key 経路と RefreshState 経路の両方がここを通るのは確認した(`execute_from_loop` → `dispatch_effect`)。足りないのは次の3点。
- **(i) 結果との結び付け**: 決定4は「直後の `actuation decision`/`[apply-ime]` の行」で結果を取ると書く。しかし ImmCross が先の窓(Standard)は async(`dispatch_ime_set_open` の `spawn_local`、`executor.rs:824-`)で、完了(`on_ime_apply_complete`、`generation`・`outcome` を span に持つ。debug)は後で別の行として届く。隣の行で結び付けると、打鍵や refresh の行が挟まって誤る。
  **`[set-open]` の行に `generation={generation:?}` を足すこと**(この関数の引数にある)。これで async の完了とも突き合わせられる。
- **(ii) sync の結果を同じ行に出す**: sync の経路は `dispatch_ime_set_open` が `Option<(bool, ImeOpenOutcome)>` を返すので、戻り値を受けて `[set-open] origin=… generation=… outcome=…` を出せば、数えるのが一行で済む。NotOwned の早期 return(`executor.rs:781`)は journal に残るが、ログの行が無い。
- **(iii) 範囲外の2つは、別のログで数えることを決定4に書く**:
  - settle で落とされた `SetOpen`(`strip_ime_set_open_if_settling`、`executor.rs:166` の debug「`[focus-settle] SetOpen(..) effect stripped`」)は、`dispatch_effect` に来ない。C3 の件数としては、このログで数える。
  - C2 も `dispatch_effect` を通らない(M-N4)。

### M-N3(Major、P2 の実装を始める前に). 決定5は「どこで止めるか」を決めていない。決め方によって決定7の固定方法が変わる
決定5は、止める場所を「key 経路 `key_pipeline.rs`、`RefreshState` 経路」と書く。実際に選べる場所は2つで、影響の範囲が違う。
- **A. Engine(コア)で ActivationSync の `SetOpen` を出さない**(`engine.rs::transition_activation` で、`origin == ActivationSync` のときは `SetOpen` を push せず、`EngineStateChanged` だけを出す)。
  - 良い点1: effect が無くなるので、key 経路の origin の分岐(`key_pipeline.rs:1325-1333`)にも、`handle_engine_activation_sync` の pending や抑制窓にも、自動で到達しなくなる。B2 がまとめて片付く。
  - 良い点2: `SetOpenOrigin::ActivationSync` の構築箇所がゼロになるので、**列挙の variant ごと削除できる**。そうすれば、決定7の件数のガードより強い、型の固定になる(コンパイラが保証する)。
  - 注意1: `transition_activation` は ExplicitUserAction の経路(`apply_active_transition`・`ime_set_open_effects` 等)と共有なので、origin で分岐すること。
  - 注意2: コアの `src/engine` のテストと、`tests/support/harness.rs:552-557` を更新すること。
  - 注意3: ADR-019(コアの OS 非依存)には影響しない。
- **B. executor で落とす**(`dispatch_effect` で origin を見て捨てる)。effect は key_pipeline の post_decision に届くので、`handle_engine_activation_sync` の側も別に止める必要がある(2か所の変更)。ActivationSync は構築され続けるので、決定7の「構築箇所の件数を0」とは両立しない。

**推奨は A**。決定5と決定7に、A を選ぶこと、そして variant を削除して型で固定すること(architecture_guard の件数は、他の呼び出し元の件数だけにする)を書くこと。
「`EngineActivationSync` の記録だけを残す」(決定5)は、A では effect が無いので記録も出ない。その記録には belief への作用が無い(`ime_model.rs:735-751`、reducer は何もしない)ので、残す意味も無い。BUG-48 の対策(echo を明示意図にしない)は、echo そのものが無くなることで自動的に満たされる。この1文は削除するか、「A では不要になる」と書き換えること。

### M-N4(Minor). C2 は「第2の actuation 経路」ではなく、「書かないのに pending を立てる経路」
`kp_apply_conv_engine_sync`(`key_pipeline.rs:856-914`)の `EngineSync::SetOpen(RomajiRecovered)` の分岐は、`handle_engine_activation_sync` を呼ぶだけで、`SetOpen` の effect を出さない。
`handle_engine_activation_sync` 自体は belief のイベント(`EngineActivationSync`・`ImeApplyRequested`)を出すだけで、実 IME には書かない(`platform_state.rs:631-690`)。つまり C2 は、**今すでに、完了の来ない pending transition を立て、書いていない抑制窓を開けている**。コメントの「actuation は同一」は実装と食い違う。
- C2 は #400 のログには出ない(書かないので、数える対象でもない)。件数は既存の info ログ「`[idle-conv-check] TsfNative: engine ON 同期`」(`key_pipeline.rs:~902`)で分かる。
- 背景の C2 の説明を「Engine を経由せず `handle_engine_activation_sync` を直接呼び、pending と抑制窓だけを立てる(実送信は無い)」に直すこと。P2 で A を選べば、C2 は「`handle_engine_activation_sync` の呼び出しを外す」だけの変更になる。
- 既存の副作用(完了の来ない pending)が今どう効いているかは、P2 の後の比較で見ればよい。先に別の不具合として扱う必要は無さそう(pending にはタイムアウトがある)。

### m7. 決定7の件数ガードは「検出の仕掛け」であって証明ではない、と書く
`dispatch_ime_set_open`/`apply_ime_open_with_belief`/`apply_ime_open_with_view` の呼び出し元の件数は、P6 の後も (a)(明示意図の再試行)のために drift correction からの呼び出しが残る。そのため「残っているのが (a) だけであること」は件数では示せない。
(a) だけを残すことは、`drift_correction.rs` の条件(明示意図があるときだけ発火)を単体テスト(`tests/closed_loop_scenarios.rs` 等、Linux で走る)で固定するのが確かな方法。決定7に1行足すこと。

### m8. P6 (c) は利用者に見える挙動の変更かを所有者に確認する
HWND キャッシュの復元は、実 IME の状態を窓ごとに戻す効果を持つ(awase が書いている場合)。Windows の IME は、もともとスレッド/窓ごとに開閉を保持する。したがって (c) の書き込みは、多くの場合「実 IME が既にそうなっている状態への重ね書き」か「キャッシュが古い場合の上書き」のはず。
ただし、「awase が窓ごとの IME 状態を覚えて戻す」ことを所有者が機能として期待していないかを、P6 の前に確認しておくこと(期待していなければ、そのまま外してよい)。

---

## (3) P1 に `Platform::set_ime_open` を足した件(PR #399 に未反映)
M-N1 のとおり、**足さないこと**。P1(#399)は記号 VK フォールバックの `send_eager_tsf_warmup(off)` の撤去だけで完結させる。

## (4) 実装に進んでよいか
- **P1(#399、set_ime_open を除く)**: 進めてよい。
- **P2 の計測(#400)**: M-N2 の (i)(ii) を足してから計測に使うこと(小さな変更で済む)。(iii) の2つは既存のログで数えることを、決定4に書くこと。
- **P2 の実装**: 計測の結果に加えて、M-N3(A か B か)を決定5・7に書いてから始めること。
- **P3 以降**: v2 の段と条件のとおりでよい。

M-N1 を外して M-N3 を決めれば、残りは Minor だけになる。その時点で収束とみなしてよい。
