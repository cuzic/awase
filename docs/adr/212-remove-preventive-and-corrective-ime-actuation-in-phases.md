---
id: ADR-212
title: |-
  ユーザー操作を引き金にしない予防的・補正的な IME への書き込み(VK_IME_ON/OFF の先回り送信など)を、段階的に撤去する
summary: |-
  所有者方針(2026-09-30): awase は IME に書かない(actuation をなくす)。ADR-191(IME が真実、観測する・書かない)と ADR-199(能動制御の例外は『IME ON/OFF トグルの役割のキー』と awase 自身の
  `keys.ime_on/off` だけ)が原則だが、確定キー(Enter)ごとの eager `VK_IME_ON`(PR #398 で撤去)のように、ユーザー操作を引き金にしない予防的・補正的な書き込みが残っている。
  棚卸し(`docs/tasks/actuation-inventory-2026-09-30.md`、develop `ccc966b8` 時点)で、`VK_IME_ON` の送信点6箇所・`VK_IME_OFF` 3箇所・eager warmup の呼び出し元4つを確認し、
  未棚卸しだった **`ActivationSync` 起源の SetOpen**(Engine の ON/OFF の遷移が自動で IME の開閉を書く)を新しく発見した。
  本 ADR は、【許可】(ユーザーが押した/設定したキーへの直接の応答)と【出力】(文字を出すための注入)を対象外に置き、【予防的】【補正的】な経路を、優先順位つきの段階(P1〜P7)で撤去する計画を決める。
  各段は1PR・revert しやすい単位・実機A/B と CI で退行を確認し、`docs/experiments.md` に判定を残す。所有者決定: 左 Shift 単独タップの半角英数トグルは残す(対象外)、ActivationSync は実機で実送信の件数を測ってから止める。
status: |-
  草案(2026-09-30)。実装は P0(#398、確定キーの eager warmup)のみ済み。次に Opus レビュー。
related_adr:
  - "ADR-098"
  - "ADR-100"
  - "ADR-191"
  - "ADR-199"
  - "ADR-203"
  - "ADR-205"
  - "ADR-206"
  - "ADR-207"
  - "ADR-211"
---

# ADR-212: 予防的・補正的な IME actuation の段階的撤去

## 背景
**所有者の方針(2026-09-30)**: 「actuation をなくそう、と何度も伝えていた」。awase は、IME の状態を観測・予測して追随し、IME に書かない。書いてよいのは、ユーザーが押した/設定したキーへの直接の応答だけ
(ADR-199 決定1・2、ADR-206)。

しかし、確定キー(Enter)を通すたびに、awase がその場で `VK_IME_ON` を送る eager warmup が残っていた(実機 A/B: Enter 24回中24回。PR #398 で撤去、0回)。
「他にも予防的な `VK_IME_ON` が残っていないか。すべて撤去したい」という要望を受けて全経路を棚卸しした
(`docs/tasks/actuation-inventory-2026-09-30.md`。事実=コードを読んで確認、推測=未確認と明記)。

### 棚卸しの要点(develop `ccc966b8`)
- F2(0x71)を送る経路は現存しない。`VK_IME_ON`(0x16)の送信点は6箇所、`VK_IME_OFF`(0x1A)は3箇所。
- **予防的・補正的で、ユーザー操作を引き金にしないもの**(棚卸しの ID):
  A4 確定キー reinject の eager warmup(#398 で撤去済み)、A2 フォーカス変更時の eager warmup、A3 開く書き込みの後の随伴 eager warmup、
  A5 Unicode long-cold の `VK_IME_ON`+`VK_A`+`BS`、A6 Unicode long-cold の `VK_IME_OFF`→`VK_IME_ON` reinit(Actuation 起点のみ)、A6b Chrome/TSF リテラル2連続 give-up 後の reinit、
  B1 drift correction(GJI/TsfNative では実 VK 送信)、**C1 `ActivationSync` 起源の SetOpen**、D1/D2 ROMAN 補完、D3 cold 時の ROMAN 保護(conv 軸)。
- **デッドコード**: 記号 VK の生フォールバック(`vk_send.rs:692`)の `send_eager_tsf_warmup(WarmupImeOn::off(), …)` は、`WarmupImeOn::off()` を渡すので実送信されない(コメントも「理論上到達しない」)。
- **C1(新発見)**: Engine の active/inactive 遷移が、対称性のために自動で `SetOpen` を発行し、executor が origin を見ずに実 VK/`ImmSetOpenStatus` の書き込みへ流す(`engine.rs` の `check_active_transition`、`executor.rs` の `dispatch_effect`)。
  ADR-191/199 の例外(ユーザーが押したキーへの応答)に入らない。実送信の頻度は未測定(GJI の shadow が一致なら `AlreadyMatched` で吸収されるはずだが、TsfNative で belief が「未知」のときは吸収されない可能性がある=推測)。

## 決定

1. **範囲**。本 ADR が撤去の対象にするのは、次の【予防的】【補正的】な、IME への書き込み・IME キーの注入。
   - 【予防的】: ユーザー操作を引き金にせず、あとで起きうる不具合を先に防ぐために送る(warmup、cold 化対策、先回りの再主張)。
   - 【補正的】: belief と実状態のずれを直すために書く(drift correction、reinit、ROMAN 補完)。
2. **対象外**(残す)。
   - 【許可】: ユーザーが押した/設定したキーへの直接の応答(`keys.ime_on/off`・ADR-199/206 の役割トグル・ユーザーが押した IME キーの素通し・Ctrl+変換のリセット・トレイ/パニックリセット・`[[keymap]]` 一致時の composition キャンセル)。
   - 【出力】: 文字を出すための注入(文字・BS・ESC、Unicode 注入、リテラル掃除)。IME の状態を変えない。
   - **左 Shift 単独タップの半角英数トグル**(棚卸し A7/A8/D4/D6): IME キーではないが、ユーザーの明示操作による機能で、所有者決定(2026-09-30)で残す。
3. **段階**(優先順位。各段は1PR・revert しやすい単位。前の段の結果を見てから次へ):
   | 段 | 対象(棚卸し ID) | 内容 | 進め方 |
   |---|---|---|---|
   | P0 | A4 確定キー reinject の eager warmup | 済み(#398)。実機 24→0、退行なし | — |
   | P1 | 記号 VK フォールバックの `send_eager_tsf_warmup(off)`(`vk_send.rs`) | デッドコードの撤去。挙動は変わらない | コンパイル+`architecture_guard`。すぐやる |
   | P2 | **C1 ActivationSync の SetOpen** | Engine の遷移が実 actuation へ流れる経路を止める(belief 側の追随 `handle_engine_activation_sync` は残す) | **先に実機で実送信の件数を測る**(決定4)。0 に近ければ止める。0 でなければ、どの状況(窓・IME・belief の状態)で送っているかを特定してから決める(所有者決定) |
   | P3 | A6b Chrome/TSF give-up 後の reinit | 撤去。BS の掃除だけに縮退 | 実 Chrome×GJI で 0/10 と実測で効かない、BUG-168 で入力中文字を消す副作用も既知。CI(literal 連続検出の系)で `gave_up` 後の自己回復の有無を見る |
   | P4 | A2 フォーカス変更 eager warmup、A3 随伴 eager warmup | 撤去。InjectionMode::Tsf(WezTerm 等)+GJI だけに効く | experiments エントリ 10 の教訓どおり、環境変数フラグで無効化→ソーク(cold 60件超)→恒久化。**WezTerm+GJI は未検証**(実機で idle 後のフォーカス→最初の文字が `kおの` 化しないか、BUG-02) |
   | P5 | A6 Unicode long-cold の reinit(Actuation 起点)、A5 Unicode long-cold の `VK_IME_ON`+`VK_A`+`BS` | A6 は ADR-203 決定3(BeliefSync のみ抑止)を Actuation 起点にも広げる。A5 は撤去の可否を検証して決める | 実機: Windows Terminal+GJI で 10s 以上 idle 後の1文字目(`bあ` 型欠落)。A5 は高リスク(Unicode 注入は GJI の確認を迂回する) |
   | P6 | B1 drift correction の VK 実送信の分岐 | 撤去は最後 | **「ずれの持続時間」を先に測る**(review-2026-09-24-09 T3)。実 Chrome では観測が乗らず、回復するか自体が未確認(BUG-172)。観測経路を先に整備 |
   | P7 | D1/D2 ROMAN 補完、D3 cold 時の ROMAN 保護 | conv 軸。別に判断する | MS-IME 本体の実機(かな入力に落ちる症状)。D3 は ADR-191 決定1が warmup 例外として維持 |
4. **P2 の計測**(ActivationSync)。実機(GJI+Windows Terminal、GJI+Chrome/Edge、MS-IME+メモ帳の通常使用)で、journal の `ActuationDecision`(`caller=DispatchImeSetOpen`)から、`origin=ActivationSync` の件数と、そのうち実送信(`outcome=Applied`/`AppliedWithoutSendInput`)の件数を、窓の種類・belief の状態(既知/未知)別に数える。
   判断: 実送信が0件に近ければ、effect の発行元で落とす。非ゼロなら、その状況で ActivationSync が担っている役割(Engine が IME に書いて IME を実際に開閉する、例: 「IME OFF 後に Engine が勝手に ON へ戻る」の再発対策、2026-08-04)を確認してから決める。
5. **各段の共通の規約**。
   - 1段=1PR。撤去の内容だけを含める(実験フラグや CI 構成を混ぜない。#360 の教訓)。
   - `fix-requires-evidence.md`(warmup/focus/belief/conv/actuation 合流点の再発ファミリー): 回帰テスト(golden/journal replay)か `docs/known-bugs/` を添える。
   - 検証: 実機の A/B(develop と撤去版、同条件)と、CI(該当する `tsx-*`/`sc-*`)。**測るのは「撤去した送信の数」と「入力の欠落・リテラル化・`@`・StaleConfirm が増えないか」**。感度が低い CI だけで「差なし」と結論しない(実機で経路が通ることを、ログの目印で確認する。今回 #398 の A/B で、条件が経路を通っていなかったことを見つけた)。
   - `docs/experiments.md` に判定を追記する。撤去を revert するときは `experiment-logging.md` に従い、アプリ・IME・症状を本文に書く。
   - `complexity-budget.md`(未発効): `RESTRICTED_CALLS` の関数(`send_eager_warmup_vk_pair`・`send_chrome_gji_reinit_and_poll` など)や tuning 定数(`CHROME_GJI_REINIT_*` など)の削除は、差し引きとして記録する。
6. **完了条件**。`VK_IME_ON`/`VK_IME_OFF`/F2 を SendInput で送る経路、および `ImmSetOpenStatus`/`WM_IME_CONTROL` で開閉を書く経路が、【許可】(決定2)の関数だけになる。これを `lints/actuation_call_guard`(`RESTRICTED_CALLS`)と `architecture_guard` の件数ガードで固定する。

## 検証方針(実機 A/B の手順)
今回(#398)の実機 A/B で、clipwire のターゲット(`ab-*`、リポジトリ外の `targets.toml`)を作った。再利用できるよう、要点を `tools/e2e/ime_key_matrix/device/` に置く(実装時に決める)。要点:
- 同じ `config.toml`、`AWASE_TEST_INJECTION=1`(目印つき注入を物理扱い)、`RUST_LOG=debug`。awase は**標準エラーを記録する起動**にする(単純な `Start-Process` だと、終了する/止められることがあった)。
- A(develop)と B(撤去版)の exe を並べて置き、同じ注入手順で比べる。注入は、**前面の窓が意図した窓であること**を、窓のハンドルで確認してから行う(プロセス名だけの確認では、別の窓に入る)。
- ログの目印で、対象の経路が実際に通ったことを確認する(例: `[composition] reinject KeyDown … marking cold + eager warmup`)。通っていなければ、その条件は測定になっていない。

## 非目的
- 半角英数トグル(所有者決定で残す)。ADR-199/206 の役割トグル・`keys.ime_on/off`。文字の出力そのもの。
- 観測(読み取り)の追加・変更。

## 代替案
- **すべて一括で撤去**: 退行の原因の特定が難しくなる。過去に「IME OFF に何を送るか」が5日間で6回反転した(experiments エントリ 01)。段階的に、各段の実測を残す方が、戻すときの根拠になる。
- **測らずに ActivationSync を止める**: 原則(ADR-191)には合うが、「IME OFF 後に Engine が勝手に ON へ戻る」(2026-08-04)の再発対策を壊す恐れがある。所有者決定で、先に測る。

## リスク
1. 予防的な送信を外すと、cold 直後の最初の1文字のリテラル化・欠落(BUG-02 系、BUG-40)が戻りうる。段ごとに実機で見る。
2. 補正的な書き込み(drift correction)を外すと、belief と実 IME のずれが観測で上書きされるまで残る。P6 で持続時間を先に測る。
3. CI(打鍵ストレス)は失敗が0件の環境で感度が低い。実機の A/B と、ログの目印での経路確認を必須にする。
