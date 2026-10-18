---
id: ADR-267
title: |-
  mozc-readable の共有メモリに「組成の段階・候補の種類・未確定文字列の長さ・処理したキーの通番・アプリ起因の遷移」を足す(version 3)
status: |-
  提案(2026-10-18、起草。Opus round1 の指摘〈致命3・重要6・軽微5〉を反映した第2版)。publisher・読み手とも未実装。round2 の再確認待ち
related_adr:
  - "ADR-191"
  - "ADR-265"
---

# ADR-267: mozc-readable の共有メモリに組成の状態を足す(version 3)

## 背景

ADR-265 の共有メモリ(`TipSharedState` version 2、48 バイト)が外に出す値は、IME の開閉・入力モード・`active` だけである。「いま未確定文字列(preedit)があるか」「変換中か」「候補が出ているか」「未確定文字列の長さ」「アプリの指示(入力スコープ)で打鍵なしに状態が変わったか」は分からない。

需要(ユーザー指摘 2026-10-18):

1. **QMK など自作キーボード**: 「組成中・変換中だけ別レイヤーにする」「候補が出ている間だけ Enter/矢印の意味を変える」「OLED に表示する」。
2. **awase**: 
   - 打鍵なしの状態遷移(パスワード欄など、アプリの指示で IME の状態が変わる)を、推測ではなく IME 自身の申告で知る。
   - 送ったキーを TIP が受け取ったかの直接の証拠(`key_seq`)。
   - 未確定文字列の長さ(参考値)。
   - **BackSpace 数の検算は需要から外す**。round1(R1-C1)で、`flush_raw_tsf_literal_backspaces` / `ze_bs_count` は「IME を素通りしたリテラル」を消す数で preedit の長さとは別の量であり、しかもその経路は `gji_active`(GoogleIMEJaConverter の I/O 観測)が前提で、mozc-readable の窓では走らないと分かった(`warm_send_plan.rs::plan_literal_detect` が `GjiNotActive` で Skip、`tsf/observer.rs::gji_is_active_ime`)。検算の対象は構造的に 0 件になる。mozc 系でリテラル検出を行う必要があるかは未確認で、必要になれば別 ADR とする。

## 決定

### D1. 構造体を version 3 にし、末尾に 32 バイト追記する(80 バイト)

オフセット 0〜47 は変えない(追記のみ。互換を壊すときだけ `version` を上げる、の取り決めに従う)。version 2 で `reserved` は `active` に使い切っており、version を上げずに済む空きは無い(R1-M4)。

| オフセット | 型 | 名前 | 意味 |
| --- | --- | --- | --- |
| 48 | u32 | `phase` | 0=組成なし、1=組成中(読みを入力中)、2=変換中(Space 後など。preedit に注目文節〈`annotation == HIGHLIGHT`〉がある) |
| 52 | u32 | `candidate_category` | 0=候補の窓なし、1=`SUGGESTION`、2=`PREDICTION`、3=`CONVERSION`。値は `commands::Category` に揃え `static_assert` で固定。`TipUiHandler` の表示判定〈`IsVisible`〉を通した結果で、UI-less モードでは 0 |
| 56 | u32 | `preedit_utf16_len` | 未確定文字列の UTF-16 の長さ(TIP が文書へ書く長さと同じ単位)。`phase=0` なら 0 |
| 60 | u32 | `preedit_cursor` | 未確定文字列内のカーソル位置。**UTF-32 のコードポイント単位**(Mozc の `output.preedit().cursor()` の単位) |
| 64 | u32 | `key_seq` | TIP が処理したキーダウンの通番(`TipKeyeventHandler::OnKeyDown` で eaten の有無に関わらず +1、ラップあり)。**打鍵ごとに毎回書く**(値が変わるため。pulse はしない) |
| 68 | u32 | `override_active` | 1 のとき、アプリの指示(入力スコープなど)で `effective_*` が Mozc セッション本来の値から上書きされている |
| 72 | u32 | `last_change_cause` | 直近の状態変化(開閉・モード・`override_active`)の原因。0=不明、1=打鍵(Mozc の応答)、2=アプリの指示(入力スコープ)、3=フォーカス移動、4=外部からの切り替え(Win+Space など) |
| 76 | u32 | `reserved` | 0 |

- `version` は `3`、サイズは 80 バイト。
- 変更点の一覧(publisher): `static_assert(sizeof(TipSharedState) == 48)` を 80 に、`Snapshot`・`operator==`・`Flush()` の書き込みに新フィールドを全部足す(足し忘れると ADR-265 R2-C2a と同じ「変化したのに書かれない」が起きる)。

### D2. 長さは UTF-16 で出す(R1-I5)

- 組成中の BackSpace は TIP が食べて Mozc の composer が処理するので、アプリの書記素の規則は効かない。長さを出す目的は、TSF/IMM の composition 文字列と突き合わせることで、TIP が文書に書く単位(UTF-16、`ComposePreeditText` → `SetText(..., preedit_text.size())`)が最も直接に使える。
- カーソルだけは Mozc が UTF-32 で持つので、そのまま出す。読み手は、同じブロック内で単位が違うことを仕様書で確かめること。
- コードポイント数・書記素数が要る読み手は少ない。要る需要が出たら追記する。

### D3. 未確定文字列の「中身」は出さない

有無・段階・長さ・カーソル位置だけを出す。パスワード欄の `composing` を 0 に落とすか(旧 Q1)は、**落とさない**。理由: Mozc はパスワードの入力スコープで入力モードを切り替えるため組成が起きにくく実害が小さい(R1-M3、`OnChangeInputScope`)。version 3 で新しく漏れるのは打鍵の時刻(`key_seq`・`update_tick_ms`)だが、同一ユーザーのプロセスは低レベルフックで打鍵そのものを取れるので、ADR-265 R1-M7 の「同一ユーザーは信頼境界の内側」の範囲内とする。

### D4. 候補の「数・選択位置」は足さない

対象は数と選択位置に限る。`candidate_category` は「有無」の判定に必要な最小の情報であり、除外しない(R1-M5)。

### D5. 値の鮮度は `key_seq` で判定する(旧 D5 を置き換え。R1-C3)

- 旧案の「`update_tick_ms` が直近の打鍵より新しいか」は成り立たない。publisher は変化が無ければ書かないので、「処理したが値が変わらなかった」と「まだ処理していない」の区別がつかず、ADR-265 D3 の読み取り規約(`update_tick_ms` による新しさの判定はしない)とも矛盾する。`seq` が進んだかも同じ理由で使えない。
- 読み手は、自分が送った(または観測した)キーダウンの数と `key_seq` を突き合わせ、TIP が追いついたかを判定する。`key_seq` は毎回書く唯一の項目である。
- 値は「TIP が編集セッションの中で公開した時点」の状態である。`OnOutputReceivedImpl` は Word の回避策で `TF_ES_ASYNC` に切り替えることがあり、受け取りと公開の間にアプリがロックを与えるまでの遅れが入る。読み手は、このずれを前提にする。

### D6. 書き込みと pulse を分ける(旧 Q2 を閉じる。R1-I6)

- `Flush()` の判定を 2 つに分ける。**書くかどうか**は全項目の比較(`key_seq` を含む)、**pulse するかどうか**は状態の項目(`active`・開閉・モード・`phase`・`candidate_category`・`override_active`・`last_change_cause`)の変化だけ。`preedit_utf16_len`・`preedit_cursor`・`key_seq` だけの変化では pulse しない。
- 書き込みのコスト(`InterlockedIncrement` 2 回と数個のストア)は、打鍵ごとに既にある mozc_server への IPC 往復(ms 単位)に比べて無視できる。測定は書き込みの遅さではなく、「連続打鍵で pulse の回数が状態の変化回数と一致する」ことの CI 確認だけにする。
- 通知イベントを 2 つに分ける案は、イベント名の増加が identity の置換表と AppContainer の名前空間(ADR-265 で通知イベントの名前空間は未確認)の両方に波及するため採らない。

### D7. 公開位置は編集セッションの末尾にする(R1-I1)

- 現行の `Publish` は `TipInputModeManager` の各 mutator の `PublishOnExit` からだけ呼ばれ、`Output` を受け取らない。引数に足すのではなく、`SetActive` と同じく別の setter(`SetComposition(...)`)を新設する。
- 呼ぶ場所は `TipEditSessionImpl::UpdateContext` / `OnCompositionTerminated` の末尾(`UpdateUI` の直前、または `TipUiHandler::Update` の中)とし、`last_output()` から値を作る。`UpdatePrivateContext` の中では呼ばない。理由: (1) `if (!output.has_status()) return S_FALSE;` の早期 return で、status の無い Output の更新を取りこぼす。(2) `UpdatePreeditAndComposition` より前に呼ばれるので、そこが `E_FAIL` でも `phase` が出てしまう。
- `candidate_category` は `TipUiHandler` の表示判定(`FillVisibility`・`TipUiElementManager::IsVisible`)を通した結果を同じ場所で取る(R1-C2)。

### D8. アプリ起因の遷移を申告する(ユーザー指摘 2026-10-18)

- 打鍵なしで状態が変わる(入力スコープでパスワード欄に入り、IME が閉じる/半角英数になる、など)ことを、`override_active` と `last_change_cause` で外に出す。version 2 でも `effective_*` と `tsf_*` の食い違いからは推測できるが、「いま食い違っている」ことしか分からず、遷移の瞬間と原因は分からない。
- awase は、これを belief の「打鍵由来ではない変化」として扱える(drift correction が推測で拾っていた類い)。**最初は journal に記録するだけの観測**とし、belief は動かさない(誤補正は本リポジトリで最も事故の多い領域のため)。
- 原因が特定できない変化は `last_change_cause=0`(不明)に倒す。どの原因を正確に出せるかは Mozc 側のコード次第で、**未確認**(Q1)。

### D9. 読み手の規約(R1-I2、R1-I4)

- **map サイズ**: 長寿命のプロセスは旧 DLL のまま 48 バイトのセクションを作り続けるので、新旧が同時に存在する。読み手は「サイズ 0(セクション全体)で map する」か、「48 で map して `version` を読み、`version >= 3` のときだけ 80 で map し直す」。80 固定で map すると、version 2 のブロックの開閉・モードまで読めなくなる。
- **書き手の `ERROR_ALREADY_EXISTS`**: 同名のセクションが旧サイズ(48)で残っていると、`MapViewOfFile(..., 80)` が失敗し、`tried_ = true` でそのスレッドは二度と書かない。この場合は 48 だけ書く(version 2 として振る舞う)ことにする。
- **`active == 0` のときは他の項目を無視する**ことを必須として仕様書に書く。`Deactivate` は組成の項目を触らないので、IME を切り替えた後も `phase` が残りうる。あわせて `Deactivate` で `phase`・`candidate_category`・`preedit_utf16_len` を 0 に落とす。
- **context とスレッドのずれ**: 組成の状態は `ITfContext` ごと(`TipPrivateContext::last_output`)だが、共有メモリはスレッドごとに 1 スロット(`thread_local ThreadSlot`)。同じスレッドの別 context へフォーカスが移っても、キーを押すまで更新されないことがある。段階 1 の CI で何が出るかを確かめ、仕様書に書く。

## 却下した代案

- **別のマッピング名**(例 `Local\mozc-readable-comp-<tid>`)に分ける案: D9 の互換問題は消え、更新頻度の違う値を分けられるが、AppContainer 経由の読み取り(`NtOpenSection`)と identity の置換表が 2 倍になる。同じマッピング + D9 の読み手規約の方が安い。
- **`composing` / `candidates_visible` の 0/1**: 打鍵中のサジェストでも `has_candidate_window` が真になり、Space の前後を区別できない(R1-C2)。変換中の BackSpace は変換の取り消しで、読みの文字数とは別の動きになる(R1-I3)。QMK の Enter の意味も、組成中(読みを確定)と変換中(候補を確定)で違う。

## 実装の段階

1. **mozc-readable**: `readable/docs/shared-memory.md` に version 3 を書く(仕様が正)。D7 の位置に `SetComposition` を配線し、`Snapshot`・`Flush()` を直す。CI で別プロセスから次を確かめる。
   - `phase`: 入力前=0、`k`=1、Space=2、Enter=0。
   - `candidate_category`: `ka` の時点は 0 または `SUGGESTION`、Space 後は `CONVERSION`(確定後に予測を出す設定は固定して外す)。
   - `key_seq`: 値が変わらない打鍵でも進む。
   - 組成中に同じ窓の別 context へ移る/Win+Space で別 IME に切り替える、のときに何が出るか。
   - パスワード欄(入力スコープ)に入ったとき `override_active=1`・`last_change_cause=2` か。
   - 連続打鍵で pulse の回数が状態の変化回数と一致する。
2. **awase**: ADR-265 段階 1 の Observe が入った後で、journal に `phase`・`candidate_category`・`key_seq`・`override_active`・`last_change_cause` を記録する(観測のみ)。
3. 記録の結果を見て、belief への反映(D8)や `key_seq` を使った「TIP が追いついたか」の判定を、別の ADR にする。
4. **QMK 向け**: `readable/docs/examples/read_ime_state.py` を version 3 対応にする(D9 の map 規約に注意。現状の `assert STRUCT.size == 48` と `MapViewOfFile(..., STRUCT.size)` は書き換えが要る)。`phase`・`candidate_category`・`override_active` を Raw HID で送る例を足す(スケッチのまま。実機未検証と明記)。

## 未決

- **Q1**: `last_change_cause` の各原因を、Mozc の実コードのどの地点で正確に区別できるか(特に「外部からの切り替え」と「フォーカス移動」)。取れない原因は 0 に倒す。
- **Q2**: Mozc の `Category` の値(`commands::Category`)と `static_assert` で固定できる範囲の確認。

## 確認していないこと

- `candidate_category` を `IsVisible` 越しに取れるか(`TipUiHandler::Update` の中で取る前提。UI-less モードの扱いを含む)。
- AppContainer 内の窓での読み取り(80 バイトでも名前空間の扱いは変わらない見込みだが、CI で確かめる)。
- 組成中の context 切り替え・IME 切り替え時に、どのフィールドがどう残るか(D9、段階 1 の CI で確かめる)。
