---
id: ADR-273
title: |-
  入力インジケーターの 2 つの証拠源(字形・右クリックメニュー)を belief へ入れる配線の入口条件と形(ADR-271 の第 3・第 4 段階)
summary: |-
  ADR-271 は設定の形(off/observe/unobserved/all)と実効値までを決め、belief への配線を別 ADR に残した。本 ADR はその配線の契約を決める。
  ①新しい ObservationSource(IndicatorMenu・TaskbarGlyph)を Medium・BeliefOnly で入れ、drift correction と read_back の根拠から明示的に除外し、短い TTL で失効させる。
  ②「Medium が High を追い越さない」は時刻ではなく順位で守る: 観測ストアの競合解決(`most_recent_trusted`)は新しい Medium が古い High に勝つ(ADR-233)ので、指標源は競合時に他の Medium 以上より下に置く。
  これは `unobserved` では他の源が嘘をつく前提と衝突するため、`unobserved` では「他の Medium 以上が新鮮に無いとき」だけ、`all` でも同じ順位とする(`all` は便益が示せるまで画面に出さない)。
  ③「観測の無い窓」の述語は `is_demoted_by_learning` を採り(ADR-261 決定 4 の N 回失敗は不採用)、④遅れによる予測の上書きは observer 側の安定化と、メニューの選択可能性(非 0 の idChild)で塞ぐ。
  配線は証拠源ごとに別 PR で、`ui_max_level()` を上げる PR が入口条件の充足を golden で示す。
status: |-
  起草(2026-10-20)。実装なし。Opus レビュー前。決定は案。所有者判断 Q1〜Q3 は本文末尾。
related_adr:
  - "ADR-233"
  - "ADR-261"
  - "ADR-263"
  - "ADR-264"
  - "ADR-268"
  - "ADR-271"
---

# ADR-273: 入力インジケーターの証拠源を belief へ入れる配線の入口条件と形

## 背景

[ADR-271](271-indicator-evidence-settings-levels.md) は、字形([ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md))と右クリックメニュー([ADR-264](264-indicator-menu-mode-evidence.md))の使い方を 4 段階で選ぶ設定と、実効値 `min(configured, ui_max_level())` を実装した(PR #698)。
`ui_max_level()` は現状 `menu = Observe`・`glyph = Off` で、`unobserved`/`all` は画面に出ず、設定ファイルに書いても観測のみに丸められる。本 ADR は、この上限を上げるときに満たすべき配線の契約を決める。

実装の現状(origin/develop 33fd4b04):

- メニュー: 段階 1(shadow)が journal にだけ出す。belief には入らない。
- 字形: コード無し。ADR-261 の段階 0(D0-2、一太郎の実機)が未測定。
- 観測ストア(`state/observation_store.rs`)の解決順は、明示意図 → 打鍵予測 → `derive_any` → `most_recent_trusted`。
  - `derive_any`: **新鮮(3 秒)な High があれば最新の 1 件で即決**。無ければ Medium 以上の無競合多数決。Medium 同士が食い違うと `None`。
  - `None` のとき `most_recent_trusted`(ADR-233 の A')。キーは `(confidence >= Medium, at, confidence)` で、**新しい Medium が古い High に勝つ**。単体テスト `most_recent_trusted_prefers_newer_medium_over_older_high` が固定している。
  - 一時的な読みは `per_source` に 1 ソース 1 枠。`expires_at` を付ければ全読み手が同時に失効する(ADR-268 D1 が `MozcShm` で使った仕組み)。

## 決定

### D1. 観測源の形

- 新しい `ObservationSource::IndicatorMenu` と `ObservationSource::TaskbarGlyph`。`confidence = Medium`、`authority() = BeliefOnly`。
- 入口は証拠源ごとに `ImeStateHub` の 1 メソッド。`classify_*` の純粋関数が「読み → 報告するか」を決め、`ImeEvent`/`reduce` を通す(`ime-belief-architecture.md`)。`desired_open`・`IntentStore` は書かない。
  `MozcShm` の追随(ADR-268 D3: 食い違いで `intent_store.remove` と `pass_through_observed`)は **Medium では採らない**。外から変わったことの追随は High の源(共有メモリ・直接の読み)の仕事で、指標は「他に決め手が無いときの belief」に限る。
- **drift correction と `read_back` の根拠から明示的に除外する**。`evaluate_drift` と `read_back` は authority を見ない(ADR-268 D2 の教訓)ので、`BeliefOnly` だけでは止まらない。除外しないと、指標が明示意図と食い違うたびに IME を開け直す(決定 0「書かない」を破る)。`EXCLUDED_FROM_ANY_FRESH_EVIDENCE` と `evaluate_drift` の除外リストに足し、`check_drift_correction` の `matches!` も更新する。
- **短い TTL**(`record_any` が `expires_at` を付ける。値は D0 で実測して tuning 定数に。仮置きなし)。失効後は指標が `per_source` から全読み手に対して消え、`most_recent_trusted` にも降りてこない。これにより「3 秒後に古い読みがフォールバックで復活する」(ADR-233 の BUG-189 型)を構造的に防ぐ。
  字形は `Miss` が来たら前の `Hit` を即失効させる(ADR-261 決定 6-3)。

### D2. 「Medium が High を追い越さない」は時刻ではなく順位で守る

ADR-271 D2b が入口条件にした項目。次の 2 つの場面がある。

1. **新鮮な High がある**: `derive_any` が High を即決するので、指標(Medium)は決まりに加わらない。ここは既に守られている(`MozcShm` が居る窓、直接の読みが取れた窓)。
2. **High が無く、Medium が競合する**: `derive_any` が `None` になり `most_recent_trusted` へ降りる。指標が ObserverPoll などより新しいと、**遅れて届いた古い状態の指標が勝つ**。`all`(読める窓にも入れる)ではこれが主な害になる。

対策: 競合解決(`most_recent_trusted` と `derive_filtered` の多数決)で、指標源を **他の Medium 以上より下**に置く。

- `derive_filtered` の Medium 無競合多数決から指標源を外し、指標源は「他の Medium 以上が新鮮に 1 件も無いとき」だけ、第 3 の段(High・Medium 多数決の後、`most_recent_trusted` の前)で採る。
- 指標の読みが採られるのは、**観測の無い窓で、他の Medium 以上が失効しているとき**に限る。`unobserved` はこの条件が自然に成立する場面(IMM が読めない窓では ObserverPoll が `None` を返し続ける)で、`all` の読める窓では ObserverPoll が居るので指標は決まりに入らない。
- **`all` の便益**: この規則では `all` は読める窓で、ObserverPoll の `ime_on=None`(タイムアウト)が続く間だけ効く。ADR-271 D2b の「便益の場面は未実証、所有者判断で `all` は残す」と整合し、**配線 PR は `all` が効く場面を 1 つ以上 journal で示す**(示せなければ `ui_max_level` を `Unobserved` までにとどめ、画面に `all` を出さない)。
- 守りの固定: 「新しい指標が古い High・新しい Medium を上書きしない」を `observation_store.rs` の単体テストと golden で固定する(`most_recent_trusted_prefers_newer_medium_over_older_high` は ADR-233 のために残す。指標源だけ例外)。

### D3. 「観測の無いウィンドウ」の述語は `is_demoted_by_learning` に統一する

ADR-261 決定 4 は「この `FocusGen` で `direct-follow` の読みが N 回失敗」、ADR-263 は `lang_detect_window_in_scope`(降格 or `scope=all`、TSF ネイティブ・`InputRelay` 除外)。ADR-271 D2 は前者を採らず、`focus::is_demoted_by_learning` と `InputRelay` 除外を使うと決めた。本 ADR でこれを確定する。

- 採る理由: N の値が未測定(ADR-261 D0)で、既に実装・テストされた述語がある。降格は学習結果としてキャッシュされ、フォーカスを往復しても安定する。
- 弱点: 学習が済む前の最初の窓は対象外(その間は従来の belief)。許容する(読めない窓だと学習する打鍵数は少ない)。
- **TSF ネイティブ窓の扱い**(読むだけの ADR-271 では継承しなかった除外): belief に入れるときは、誤った belief が Engine の活性(親指キーを通すか)を変えるので、TSF ネイティブ(`is_effectively_tsf_native`)は**最初の配線では除外する**。理由は、TSF の窓には TSF 側の観測と `MozcShm` の経路が既にあり、指標を足す便益が不明なまま BUG-124 型の影響面を広げるため。便益が示せたら別 PR で外す(Q2)。

### D4. 遅れによる誤った上書きを塞ぐ

- **字形**: ADR-261 決定 6-1 の(b)を採る。同じ字形が最大遅れ(0.68 秒)以上続いたときだけ報告する(observer 側の安定化。`reduce` と時刻の扱いは変えない)。打鍵後 N ms 以内は報告しない(併用、N は D0-9 の実測)。
- **メニュー**: 読みを報告するのは、`MENUSTART`〜`MENUEND` の間に**非 0 の `idChild` が 1 度も出ていない**とき、かつメニューが閉じる前に読んだ値に限る。選択された可能性があれば読みは古い(ADR-264 決定 5)。
  GJI の I/O 推論との順序(BUG-203): 非 0 の `idChild` が出たメニューの `MENUEND` から N ms は、GJI の I/O を開閉の証拠にしない(ADR-264 決定 5 未決 2 を本 ADR で確定。BUG-203 の修正は独立に済ませ、本 ADR の証拠には依存させない)。
- **キー押下との競合**: 予測(`KeyEffectPredicted`)が新鮮な間は、`reconcile` の `KEY_EFFECT_SETTLE_MS`(170 ms)が過ぎるまで指標を `per_source` に入れない(予測を消さないため)。

### D5. 回帰テストと実機・CI での確認

- `observation_store.rs`: ①新しい指標(Medium)+古い High → High、②新しい指標+新しい ObserverPoll(Medium)で食い違い → ObserverPoll、③ObserverPoll が失効し指標だけ新鮮 → 指標、④TTL 後は全読み手から消える、⑤`evaluate_drift`・`read_back` が指標を根拠にしない。
- `architecture_guard.rs`: 指標源の `ObservationSource` と報告口の生成点の件数固定。`observation_source_guard`(dylint)の更新。カウンタの名前に `belief_` を使わない(「&mut self.belief」走査に当たる、PR #698 の教訓)。
- journal の `IndicatorEvidence` 行に「採った/採らなかった」と理由(他に Medium 以上が居た・TTL 外・`idChild`)を残し、`indicator_evidence` の要約(ADR-271 D5)に件数を足す。
- CI(Windows): ①メニュー: GJI・MS-IME の IMM 偽装窓(ADR-264 の近似窓)で `unobserved` にして、ObserverPoll が嘘をつく窓で belief が指標に従うこと、読める窓(`unobserved`)では belief が変わらないこと。②GJI の閉じる操作直後に I/O 推論が ON を立てないこと(BUG-203)。

## 段階

1. 核(`awase-windows-core`): D1 の源・除外・TTL、D2 の順位、`classify_*`、単体テストと golden。**`ui_max_level` は上げない**。
2. メニュー: 報告口と D4 の `idChild` 条件。CI 通過後に `menu` の `ui_max_level` を `Unobserved` へ。画面に第 3 段階を出す。
3. 字形: ADR-261 の段階 0〜1 が済んでから(別 PR)。`glyph` の行を出し、`Unobserved` へ。
4. `all`: 便益の場面を journal で示せたら `ui_max_level` を `All` へ。示せなければ上げない(ADR-271 D2b)。

## 未解決(所有者)

- **Q1**: D2 の順位(指標は他の Medium 以上より下)で `unobserved` を始めてよいか。代案は、`unobserved` に限り指標を Medium と同順位にして「新しい方が勝つ」とする(ObserverPoll が嘘をつく窓で素早く効くが、遅れた古い指標が勝つ害を許す)。
- **Q2**: TSF ネイティブ窓の除外(D3)を最初の配線から外してよい条件(便益の示し方)。
- **Q3**: メニューを先に `Unobserved` へ上げる順序(メニューは開いた瞬間の 1 回だけなので便益は小さい。字形は一太郎の実機測定が先で遅い)。メニューだけ先に出す価値があるか、字形を待つか。

## 関連

[ADR-271](271-indicator-evidence-settings-levels.md)(設定と入口条件)・[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md)・[ADR-264](264-indicator-menu-mode-evidence.md)(証拠源)・
[ADR-233](233-stale-high-observation-beats-newer-in-most-recent-trusted.md)(`most_recent_trusted` の順位)・[ADR-268](268-mozc-shm-belief-connection.md)(High の源を入れた前例。TTL・除外・identity)・
[ADR-263](263-episode-start-key-hold-and-language-decision.md)(述語の出所)。
