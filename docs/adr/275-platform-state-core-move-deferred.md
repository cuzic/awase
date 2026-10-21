---
id: ADR-275
title: |-
  platform_state.rs・sync_actuation.rs の core への移動は見送る(代わりに記録系の強制を強める小 PR を 3 本)
summary: |-
  殻 awase-windows の最大の塊 platform_state.rs(5,030 行)・sync_actuation.rs(757 行)を core へ移す案(ECRS R1)を起票し、Opus に敵対的レビューを依頼した。結論は見送り。
  理由: (1) ime_decision_view.rs は tsf::observer(Windows 専用)を読むので移せない。(2) INV-A97-1 で「コンパイラ強制」されているのは「他 crate から record_* を呼べない」ことだけで、呼び出し元ファイルの固定は今も CI(RECORDERS)。本番のハブは Runtime.platform_state が private なので外から届かず、移動しても変わらない。ただし (3) 移動すると pub(crate) 約 70 個が pub になり公開面の規律(PLATFORM_STATE_PUB_FNS)が崩れ、孤児規則で構築口(new・Default)が殻に書けなくなる。(4) 移動しないと得られない性質(Linux テスト・mutation testing)は無い: 今すでに Linux でテストでき、mutants の対象外コメントは古い。
  代わりに独立の小 PR を 3 本: mutants の対象に platform_state.rs を足す、dylint の RESTRICTED_CALLS に record_* を足す(UFCS の穴を塞ぐ)、RECORDERS の走査を workspace 全体に広げる。scope 注入(D1)も見送る(ADR-229 FCIS P2 の反転で、偽スコープが本番に漏れる危険がある)。
status: |-
  決定: 移動は見送り。Opus レビュー(docs/adr/review/275-opus-platform-state-move.md)反映済み。小 PR 3 本を実施する。
related_adr:
  - "ADR-229"
  - "ADR-241"
  - "ADR-272"
  - "ADR-274"
---

# ADR-275: platform_state.rs・sync_actuation.rs の core への移動は見送る

## 経緯

ADR-274 で core を 7 crate に分けた後、殻 awase-windows(本番 45,307 行)に残る最大の塊は `state/platform_state.rs`
(`ImeStateHub`・`PlatformState` ほか、5,030 行)と `state/sync_actuation.rs`(757 行)。OS を読まないので core へ移せるように見える
(ECRS R1、移動約 5,800 行で core 系 40.9% → 約 48%)。殻に残してきた理由は記録系 `record_*` の可視性 INV-A97-1
(所有者の決定、ADR-229 の Opus 案 (d))だった。

草案(前景スコープの注入 → 新 crate `awase-ime-state` への移動 → 記録系の強制を dylint へ)を書き、Opus に敵対的レビューを
依頼した(全文: [review/275-opus-platform-state-move.md](review/275-opus-platform-state-move.md))。

## 決定

**移動(段階 2)は見送る。scope 注入(D1)も見送る。** 代わりに、移動と無関係に価値のある小 PR を 3 本入れる。

## 理由(Opus レビューの要点)

1. **`ime_decision_view.rs` は移せない。** `tsf::observer`(`TsfObservations`・`ActiveImeKind`・`candidate_was_seen`)は殻の `cfg(windows)` モジュール。
   `platform_state.rs`・`sync_actuation.rs` は `ime_decision_view` を使っていないので、移す理由も無い。
2. **INV-A97-1 の「コンパイラ強制」は草案の思い込みだった。** `pub(crate)` が強制するのは「awase-windows の外から呼べない」ことだけ。
   呼び出し元ファイルの固定は今も CI(`RECORDERS`)。本番の安全性の根拠は `Runtime.platform_state` が private で、本番のハブに
   crate の外から届かないこと。移動しても変わらない。ただし閉ループのハーネスなど他 crate の自前インスタンスで `applied` を直接書ける
   ようになり、シナリオの忠実度が黙って落ちる恐れがある。
3. **移動の費用が大きい。** `pub(crate)` 約 70 個(`belief`・`journal`・`write_*` を含む)が `pub` になり `PLATFORM_STATE_PUB_FNS`
   (「pub はハーネスが呼ぶものだけ」)の規律が崩れる。孤児規則で `PlatformState::new`・`Default` が殻に書けなくなり構築口と時計のガード群を作り直す。
   `#[cfg(test)]` の口は crate をまたぐと消える(`testing` feature が要る)。新 crate の `-p` 足し忘れは dylint を黙らせる。
4. **移動しないと得られない性質は無い。** platform_state のテストは今すでに Linux で走る。mutation testing の対象にも今日からできる
   (`.cargo/mutants-awase-windows.toml` の「`cfg(windows)` なので対象外」は古いコメント)。得られるのは行数比率・増分ビルド・依存の向きの
   Cargo 保証だけで、違反の実績は 0。
5. **D1(scope 注入)は ADR-229 FCIS P2 の反転。** 核は scope を引数で受ける純粋な形で、「1 呼び出し 1 回読み」を `shell_shape_violation` が固定している。
   注入すると内側と外側で 2 回読み、`fn()` ポインタはテスト中の scope 切替ができず、既定の偽スコープが本番に漏れると ADR-260 が黙って無効になる。
   テストの書き換え 90 件、黙って外れる検査 4 件。

## 代わりに入れる小 PR(3 本)

1. `.cargo/mutants-awase-windows.toml` に `state/platform_state.rs` を足し、古いコメントを直す。
2. `lints/actuation_call_guard` の `RESTRICTED_CALLS` に `record_optimistic`・`record_confirmed`・`record_ime_apply_result` を足す
   (UFCS 呼び出しを拾う。`RECORDERS` は `.record_*(` で始まる文字列なので UFCS を見逃す)。許可は関数名で書く。件数は `RECORDERS` が見るので両方残す。
   `RECORDERS` の doc に残る古い関数名も直す。
3. `RECORDERS` の走査を workspace 全体の本番コードに広げ、ハーネス(`tests/support/harness.rs`)から `record_*` を名指さないガードを足す。

## 将来、段階 2 をやるなら(前提条件)

所有者が行数比率を目標として段階 2 を選ぶ場合は、先に次を ADR に書くこと: (a) `check_wiring.py` に「awase-windows の path 依存は
`core-crates.txt` に載ること」を足す、(b) `ime_decision_view.rs` を対象から外す、(c) 構築口の設計(孤児規則)、(d)
`production_hub_is_unreachable_from_outside_the_crate` の書き直し方針、(e) `#[cfg(test)]` の口の `testing` feature 化、(f) 新 crate は最上段
(`awase-ime-core` に入れると hook-core・plans を経由して循環する。依存に `tracing`・`quanta` が要る)。

## 起票時点の事実の訂正

- 草案の「dylint は CI の必須ジョブ」は誤り(develop にブランチ保護が無い)。赤い CI のままマージできる運用は今もこれからも一番弱い所で、移動の有無と無関係。
