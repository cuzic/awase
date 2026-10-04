---
id: ADR-225
title: |-
  RawTsfLiteralRecovery の give-up を「実 IME が閉じている」証拠として扱い、文字の痕跡なき消失を止める(BUG-074)
summary: |-
  BUG-074: awase の見ていない経路で実 IME が閉じ belief が ON のままのとき、TsfNative×GJI の最初の打鍵は literal になり、回収(BS+再送)も literal になって give-up し、文字が消える(CI 10/10)。
  TsfNative は開閉を読めない(Blind)ため drift correction は発火できず、give-up の literal 判定が唯一の証拠になる。案: (C)give-up を Low ではなく Medium の観測として belief に入れ、既存の drift correction に再オープンを任せる。再送は D0 の測定後に判断する。
status: |-
  起草(2026-10-04)。Opus レビュー待ち。実装なし。
related_adr:
  - "ADR-080"
  - "ADR-100"
  - "ADR-101"
  - "ADR-191"
  - "ADR-212"
---

# ADR-225: give-up を「実 IME が閉じている」証拠として扱う

## 背景と事実(確認済みのもの)

- 再現(CI、2026-10-04、run 37188479610 `sc-driftrecovery-gji-tsf`): 実 IME を awase の外から閉じ(belief は ON)、`k`,`a` を打つ 10 試行が **10/10 で give-up**(`consecutive_before=1 gave_up=true backs=1`)、入力先は空。MS-IME×tsf は同条件で 0 件(GJI 固有)。
- 経路: `output/probe_io.rs` の `RawTsfLiteralRecovery`。`consecutive==0` は BS+romaji 再送、それ以外は再送なしで BS のみ(`set_raw_literal(backs, String::new(), …)`)。以前は give-up 後に reinit(VK_IME_OFF→ON)+再送をしていたが、ADR-212 P3 で撤去(実 Chrome×GJI で 0/10)。ただし ADR-212 P3 の表は「自前 RichEdit(tsf×GJI)では 30/30 効いた」とも記録している。
- ADR-100 決定3 は「retry を give-up に足す」案を、完了通知と focus 世代照合が無いこと等で却下し、案L(journal に romaji を残す)を採った(実装済み)。ADR-101 はその前提を整えて retry を実装したが、ADR-212 P3 で reinit ごと撤去された。
- TsfNative は IME の開閉を読み戻せない(`default_feedback` = `Blind`)。`ir_apply_drift_correction` は observed と desired の食い違いが前提のため、**belief が ON のまま・観測が無い状況では発火しない**。

## 未確認のこと(D0 で測る)

1. give-up 後、2 打鍵目以降は回復するか(IME は閉じたままか)。記録: 10 試行の以降の出力と、実 IME の開閉。
2. give-up の時点で belief/`desired_open`/observations はどうなっているか。
3. 実機(Chrome)の報告と同じ機序か(不明。本件の CI は自前の RichEdit 相当窓)。

## 決定案

- **D0(先に測る)**: `check_drift_recovery.py` に「give-up 後の追加打鍵 3 回の出力」と「awase.log の belief 遷移」を足す測定を CI で 1 回回す(観測のみ、挙動変更なし)。
- **D1(案C)**: give-up の確定時に、`ObserverReported { source: <新: LiteralGiveUp>, confidence: Medium, open: false }` を **観測**として `ImeStateHub` に記録する(`ime-belief-architecture.md` の Observe→classify→reduce に従う。`UserImeSetIntent`・`HeuristicDefault` の流用はしない)。`classify_*` の純粋関数で「give-up 連続カウント==1 かつ GJI×TsfNative」のときだけ観測を作る。これで `effective_open()=false` が `desired_open=ON` と食い違い、既存の drift correction が再オープンを試みる(Blind の `max_attempts` で打ち切り)。
- **D2(再送は保留)**: 失われた romaji の再送は D0 の結果が出るまで入れない。理由: ADR-100 決定3 の却下理由(完了通知なし・focus 世代)がそのまま残る。D1 で後続の文字が救われ、1 文字だけが失われるなら、案L の記録だけで足りるかを判断する。

## 代案

- **A. 受容**: 現状のまま、known-bugs の記録だけ。後続の文字が落ち続けるなら不採用。
- **B. 案J(Unicode 直接送信で romaji を出す)**: IME が閉じていると `ka` が英字で出る。意図(か)とは違うが「消える」よりまし、という立場。誤判定(実は IME ON)のときは二重出力になる。
- **C'. reinit の復活**: ADR-212 P3 の撤去根拠(実 Chrome 0/10・BUG-168 の副作用)を覆す測定がない限り不採用。

## 守る規約

- 再発ファミリー(warmup/IME belief/IME actuation 合流点)。回帰テスト(`classify_*` の純粋関数テスト、`journal_replay`)と `docs/known-bugs/BUG-074.md` の更新を同じ PR で。
- 新 gate/新入口を足す場合は ADR-119 の合流点表(`fix-requires-evidence.md`)を全て洗う。D1 は actuation の入口を足さず、既存の drift correction に任せる前提。
- belief 書き込みは `ObserverReported` 経由のみ。`ImeModel` の private フィールドに触れない。
- 設計の前に測る: D0 を先に回す。実機(Chrome)の同一性が確認できるまで「修正済み」とは書かない。

## リスク(起草者の見立て、Opus に突いてほしい点)

- give-up の literal 判定が、実 IME が開いているのに cold で早すぎただけの場合(BUG-074 の元の報告=Windows Terminal の cold)、belief を OFF に倒すと NICOLA を誤って止める。
- drift correction の再オープンが Blind で効かないとき、`max_attempts` 後は何も起きず、D1 は無駄な観測になる。
- BUG-27 追補2(無限 BS ループ)の再来にならないか。D1 は送信を増やさないが、drift correction 側の送信が増える。
