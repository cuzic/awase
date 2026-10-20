---
id: ADR-273
title: |-
  入力インジケーターの 2 つの証拠源(字形・右クリックメニュー)を belief へ入れる配線の入口条件と形(ADR-271 の第 3・第 4 段階)
summary: |-
  ADR-271 は設定の形(off/observe/unobserved/all)と実効値までを決め、belief への配線を別 ADR に残した。本 ADR はその配線の契約を決める。
  ①配線するのは IME の開閉だけ(入力モードは shadow のまま)。新しい ObservationSource(IndicatorMenu・TaskbarGlyph)を Medium・BeliefOnly で入れ、短い TTL(≤ 3 秒)で失効させ、
  drift correction・read_back(Converged を含む)・reducer の drift/予測の整合から明示的に外す。
  ②「Medium が High を追い越さない」は順位で守る: `resolve_open_at` の中に、`derive_any` の後・`most_recent_trusted` の前の独立した分岐を足し、指標は「新鮮な Medium 以上が 1 件も無い」ときだけ採る。
  帰結として、指標が効くのは **IMM が NULL を返して ObserverPoll が来ない窓**に限られ、IMM が嘘の値を返す窓・GJI の I/O 推論が ObserverPoll を書く窓では負ける(B1)。便益は小さく、その窓の型(一太郎)は未確定(ADR-261 D0-2)なので、段階 2 の前提にする。
  ③「観測の無い窓」の述語は `is_demoted_by_learning`(=NULL 型)を採る。④遅れ対策: 字形は最後のモード系打鍵から最大遅れ+余裕が経つまで報告しない(必須)。窓の同一性と FocusGen・前面の一致を照合する。
status: |-
  起草(2026-10-20)。実装なし。Opus レビュー round 1(Blocker 3・Major 10・minor 10)を反映して書き直した。再確認待ち。決定は案。所有者判断 Q1〜Q3 は本文末尾。
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

- メニュー: 段階 1(shadow)が journal にだけ出す。belief には入らない。字形: コード無し(ADR-261 段階 0 の D0-2〈一太郎の実機〉が未測定)。
- 観測ストア(`state/observation_store.rs`)。一時的な読みは `per_source` に 1 ソース 1 枠で、`expires_at` を付ければ全読み手が同時に失効する(ADR-268 D1 が `MozcShm` で使った仕組み)。
- `resolve_open_at`(`state/ime_model.rs`)の順は、(`platform_state.rs` の `IntentStore` が最上位)→ 明示意図 → 打鍵予測 → `derive_any` → `most_recent_trusted`。
  - `derive_any`: **新鮮(3 秒)な High があれば最新の 1 件で即決**。無ければ Medium 以上の無競合多数決。Medium 同士が食い違うと `None`。
  - `most_recent_trusted`(ADR-233 の A'): キーは `(confidence >= Medium, at, confidence)`。**新しい Medium が古い(新鮮でない)High に勝つ**(単体テスト `most_recent_trusted_prefers_newer_medium_over_older_high`)。
- `ObservationSource` を読む箇所の全体(grep、worktree c3d76dda): `derive_any`(呼び出し元は `resolve_open_at`・`fallback_shadow`・`ModeKeyPassedThrough{align_desired}`・`align_placeholder_desired`)、`derive_actuating`(authority で絞る)、
  `most_recent_trusted`(`resolve_open_at` のみ)、`most_recent_trusted_excluding`(`evaluate_drift`・診断)、`read_back` の 2 分岐(`AnyFreshEvidence` と `Converged`)、`heuristic_default`、`consensus`(本番の呼び出し無し)、`observation(source)`。

## 決定

### D0. 範囲: 配線するのは IME の開閉だけ。入力モードは shadow のまま

字形・メニューはモード(ひらがな/半角英数/カタカナ等)も読むが、入力モードの belief(`InputModeObserved`)には源ごとの枠・順位・TTL・fence が無く、新しい Medium が直接の読みを無条件に上書きする(`ime_model.rs` の `InputModeObserved` アーム)。
本 ADR は開閉だけを扱い、**入力モードへの配線は別 ADR**とする(入れる場合は同じ契約を書く: 順位・TTL・D4 の柵・`observation_source_guard`)。それまで、`unobserved`/`all` でもモードは journal の shadow にだけ出す。

### D1. 観測源の形と、読み手・reducer の除外

- 新しい `ObservationSource::IndicatorMenu` と `TaskbarGlyph`。`confidence = Medium`、`authority() = BeliefOnly`。`evidence.rs` の `declare_evidence!` に `BeliefPool` として足す(INV-38 の const assert)。`PerSourceObservations` の get/set/iter/`clear_all` も更新する。
- 入口は証拠源ごとに `ImeStateHub` の 1 メソッド。`classify_*` の純粋関数が「読み → 報告するか」を決め、`ImeEvent` → `reduce` を通す(`ObserverReported` を経由しない近道は `ime-belief-architecture.md` 違反)。`desired_open`・`IntentStore` は書かない。MozcShm の追随(ADR-268 D3)は Medium では採らない。
- **reducer の源分岐(M1)**: `ime_model.rs` の `ObserverReported` アームは源を見ずに `update_drift(desired_open, observed.open())` と `reconcile_key_effect_open` を走らせる。指標源については**両方を呼ばない**分岐を `reduce` の中に足す(殻に近道を作らない)。呼ぶと、指標が drift を `None` に戻して正当な補正を遅らせ(`drift_duration` も巻き戻る)、`KEY_EFFECT_SETTLE_MS`(170ms)を過ぎた Medium が正しい予測を消す(D4)。同じ穴は `MozcShm` にも既にある(別 BUG として切り分ける)。
- **読み手の除外**: `evaluate_drift` の除外リスト、`read_back` の `AnyFreshEvidence`(`EXCLUDED_FROM_ANY_FRESH_EVIDENCE`)に加え、**`ReadBackQuery::Converged`(`most_recent_trusted_after`、除外が空)にも除外を持たせる**(M2。`_after` は private なので `ReadBackQuery` 側に除外を持たせる)。`Converged` を直さないと、遅れた字形の「開」が再送の収束(`Confirmed`)に使われる。
  `ime_event.rs` の doc が言う `platform_state.rs::check_drift_correction` の `matches!` は存在しない(`check_drift_correction` は `evaluate_drift(..).ok()` だけ)ので、doc 側を直す(m1)。
- **同一性(M8)**: `derive_filtered` の `is_identity_ok` に新しい源を足す(epoch+hwnd)。さらに、読んだ時点と適用時点で `FocusGen` が変わっていれば捨て(ADR-261 決定 5・ADR-264 決定 4)、前面窓が追跡中の窓と一致している(`lang_detect_focus_in_sync` と同じ型)ことを `classify_*` の入力にする。Alt+Tab 直後は `demoted_by_learning`・focus が前の窓の値のことがあり、場面①(フォーカス直後の字形)はちょうどそこに当たる。
- **TTL**: `record_any` が `expires_at` を付ける。**制約: TTL ≤ `OBSERVATION_FRESH_WINDOW_MS`(3 秒)**。超えると指標が `derive_any` の鮮度を外れた後も `most_recent_trusted` に残り、何十秒も前の ObserverPoll にさえ負けるか、逆に古い指標が残る。字形は `Miss` が来たら前の `Hit` を即失効させる。
  TTL は短いほど安全ではない: 報告は散発的(字形はフォーカスと最初の打鍵まわり、メニューは 1 回)で、TTL が切れると belief は `most_recent_trusted` の古い値か `DesiredFallback` に戻り、利用者の操作なしに Engine が切り替わる(`effective-open-flip`)。D0 で測るのは**遅れだけでなく報告の間隔の分布**で、TTL が切れたときの戻り先を決める(Q4 として配線 PR に残す)。
- **HWND キャッシュ経由の残り(M4)**: `runtime/focus_tracking.rs` が窓を離れるときに `effective_open()` を HWND キャッシュへ保存し、戻ると `HwndCacheRestored` が `desired_open` に書く。指標で決まった belief は、TTL が切れた後も窓を往復すると**キャッシュ経由で残る**。「失効後は全読み手から消える」はキャッシュには及ばない。既存の源にも共通の性質なので許容するが、journal に「指標由来」を残して追えるようにする。

### D2. 「Medium が High を追い越さない」は順位で守る。段の置き場と勝ち負けの表

「High」は**新鮮な High**(3 秒以内、`derive_any` が即決する)を指す。新鮮でない High(フォーカス時の `ImmCrossProbe` は `expires_at: None` で残る)は、ADR-233 のとおり新しい Medium に負ける。指標が負けるべき相手は「新鮮な Medium 以上」であって、古い High ではない(M10)。

- **配置(M3)**: `resolve_open_at` の中に、`derive_any` と `most_recent_trusted` の間の**独立した分岐**として置く。`derive_any`(`derive_filtered`)の中には入れない: `derive_any` は `ModeKeyPassedThrough{align_desired}` と `align_placeholder_desired` が `desired_open` に書く根拠なので、中に入れると指標が `desired_open` を書き、D1 の「書かない」が破れる。
  この分岐は新しい `BaseDecision` の種類(`[effective-open-flip]` の出力と journal)と、`fallback_shadow` の早期 return の条件の更新を伴う(指標で決まった場面を「フォールバック」と数えない)。
- 指標は、新鮮な Medium 以上が **1 件も無い**ときだけ採る。

| 同じ窓に居る相手 | 指標の勝ち負け | 理由・備考 |
| --- | --- | --- |
| 新鮮な High(`ImmCrossProbe`・`MozcShm`・直接の読み) | 負ける | `derive_any` が即決。指標は段に降りてこない |
| 新鮮な ObserverPoll(IMM の読み。値を返す窓) | 負ける | 嘘の値を返す窓でも、読み値が新鮮なら指標は採られない(**嘘型は救えない**、D3) |
| 新鮮な ObserverPoll(GJI の I/O 推論。`observe_gji_after_focus` が `Some(true)` の向きだけ書く) | 負ける | GJI の窓では I/O 推論が ObserverPoll を書き続けるので、指標は閉の読みでも負ける。BUG-203 型には効かない(B1) |
| `ConvOpenInference`(Medium、TsfNative の idle-conv-check) | 負ける | 新鮮な間は無競合多数決に参加する |
| ObserverPoll が来ない(IMM が NULL・タイムアウト) | **勝つ** | 指標が効く場面。これが NULL 型の窓(D3) |
| 打鍵予測・明示意図・`IntentStore` | 負ける | `resolve_open_at` の上位。利用者が awase のキーで切り替えた窓では、その後外から変わっても `IntentStore` の TTL(ON 10 秒・OFF 30 秒)の間、指標は効かない(M6) |
| `HeuristicDefault`・`FocusProbe`(Low) | 勝つ | Low は Medium 以上に負ける既存の規則 |

- **帰結(B1)**: 指標が効くのは「IMM が NULL を返して ObserverPoll が来ず、新鮮な他の Medium 以上も無い窓」に限られる。ADR-271 の `unobserved` が想定した「嘘をつく窓」の一部(IMM が値を返す嘘型、GJI の I/O 推論が走る窓)は**救えない**。これを承知で順位を守るのは、Medium が High と Medium を上書きする害(遅れた古い指標が正しい読みを潰す)が、効く窓を増やす便益より大きいと見るため。Q1 で所有者に確認する。
- **`all` の便益(M9・m10)**: 読める窓でも ObserverPoll が `None` の窓(TSF ネイティブ・静的な Imm32Unavailable の Chrome・Edge・Windows Terminal・VS Code・WezTerm。`classify_poll_outcome` の `is_tsf_native` 分岐)では指標が belief の主な源になる。打鍵量の大半を占める窓で害の面積が大きい。
  したがって **D3 の TSF ネイティブ除外は `all` にも掛ける**(Chrome 系で便益を示せるまで)。タイムアウトが続く間に指標を入れることは、BUG-158 追補の「タイムアウトは証拠ではないので belief を保つ」の改訂になるので、その ADR にも追記する。配線 PR は `all` が効く場面を journal で 1 つ以上示す(示せなければ `ui_max_level` を `Unobserved` までにとどめ、画面に `all` を出さない)。
- journal の「採った/採らなかった」の理由に `ExplicitIntent`・`IntentStore`・他の Medium・TTL 外を入れ、効く場面の割合を測る(M6)。

### D3. 「観測の無いウィンドウ」の述語は `is_demoted_by_learning`。これは NULL 型の窓を選ぶ

ADR-261 決定 4 は「この `FocusGen` で `direct-follow` の読みが N 回失敗」、ADR-263 は `lang_detect_window_in_scope`。ADR-271 D2 は前者を採らず、`focus::is_demoted_by_learning` と `InputRelay` 除外を使うと決めた。本 ADR で確定する。

- 採る理由: N が未測定で、既に実装・テストされた述語がある。降格は学習結果としてキャッシュされ、フォーカスを往復しても安定する。
- **述語が選ぶ窓の種類(B2)**: 降格の経路は `record_null_probe`(`ImmGetDefaultIMEWnd` が NULL を `UNAVAILABLE_CONFIRM_THRESHOLD` 回)と、`learn_imm_capability_from_miss` の miss 閾値超え(`ime_on == None` かつタイムアウトでない)の 2 つだけ。
  つまり述語が真になるのは **IMM が値を返さない窓(NULL 型)**で、**値を返すが嘘の窓(嘘型)は降格せず、対象外**。一太郎がどちらの型かは ADR-261 D0-2 で未確定。**嘘型だった場合、本 ADR の配線は #623 に効かない**。
- **段階 2 の前提**: 一太郎(または報告者の環境)の型を D0-2 で確かめる。NULL 型と分かるまでは `ui_max_level` を上げない。CI(ADR-264 の近似窓)は `cache.toml` で `unavailable` を強制して再現しているので、実環境の学習経路で降格する状態であることを CI で別に確かめる。
- 弱点: 学習が済む前の最初の窓は対象外。誤った降格(一時窓の NULL が連続)は `cache.toml` に永続する。誤った降格は安全側(ObserverPoll が来れば指標は負ける)なので害は小さい。
- **TSF ネイティブ窓の除外**: `is_effectively_tsf_native`(クラス名の表)に当たる窓と `InputRelay` を、`unobserved` でも `all` でも除外する。誤った belief が Engine の活性を変える面が、BUG-124 型の影響で広がるのを避ける。この除外は `unobserved` ではほとんど空振りする(降格は `Standard` からの降格でしか起きない)ので、効くのは `all` の側。実行時に TSF と学習した窓(`injection_mode_store.has_tsf`)は表に無く除外されないので、配線 PR で扱いを決める(Q2)。

### D4. 遅れによる誤った上書きを塞ぐ

- **字形(B3)**: ADR-261 決定 6-1(b)の安定化(同じ字形が最大遅れ以上続いたとき)は、**それだけでは足りない**。`reconcile_key_effect_open` は届いた時刻と予測時刻の差が 170ms 以上の Medium で予測を消すので、押す**前**から変わらない字形(最大 0.68 秒の遅れで押下後も旧状態のまま)は安定化を通って、正しい予測を消す。
  したがって次を**必須**にする: **最後のモード系の打鍵(`KeyEffectPredicted`)から、最大遅れ+余裕(≥ 0.68 秒+α、α は D0 で測る)が経つまで字形を報告しない**。安定化の窓の起点は最後の打鍵より後とする。170ms 程度の柵では足りない。
- **メニュー**: 報告するのは、`MENUSTART`〜`MENUEND` の間に非 0 の `idChild` が 1 度も出ていないときに限る(出ていれば選択された可能性があり読みは古い)。残る穴は ADR-264 のまま未解決として引き継ぐ: ①閉じるクリックがインジケーターへの左クリックとして届き、`idChild` なしで切り替わる(M0-10)、②キーボード選択で `idChild` が出るか(M0-3)、
  ③カーソルを動かせばほぼ必ず非 0 の `idChild` が出るので、報告が通る頻度は小さい(Q3)、④BUG-202 修正(`#32768` を窓の切替から外した)後も `idChild` が同じイベントで取れるか。メニューにも、打鍵時刻との前後関係の条件を入れる(読んだ時刻と届いた時刻の差)。1 回限りなので、柵で落とした報告は捨てられ、何も残らない。
- **GJI の I/O 推論との順序(BUG-203)は本 ADR の範囲外**。ObserverPoll(Actuating)の挙動を変える修正で、実 Chrome+GJI で再現せず優先度を下げた件(2026-10-18)。BUG-203 の側で扱う。

### D5. 回帰テストと確認

- `observation_store.rs`: ①新鮮な High+新しい指標 → High、②新しい指標+新鮮な ObserverPoll で食い違い → ObserverPoll、③ObserverPoll が来ない・失効、指標が新鮮 → 指標、④TTL 後に全読み手から消える(TTL ≤ 3 秒の制約も)、⑤`evaluate_drift`・`read_back` の**両分岐**(`AnyFreshEvidence`・`Converged`)が指標を根拠にしない、⑥古い(新鮮でない)High+新しい指標 → 指標(ADR-233 との整合を固定)。
- `ime_model.rs`: reducer が指標源について `update_drift`・`reconcile_key_effect_open` を呼ばない。打鍵予測が新鮮な間に届いた指標が予測を消さない。字形は最後のモード系打鍵から柵が経つまで報告されない。
- `architecture_guard.rs`: 指標源の報告口の生成点の件数固定(`mozc_shm_reader_touches_belief_only_through_report_mozc_shm` と同じ型)。カウンタ名に `belief_` を使わない。`observation_source_guard`(dylint)は D0 により入力モードを扱わないので更新しない。
- 新しい tuning 定数(TTL・字形の打鍵後の柵の α・必要なら他)は `#[measured_macro::measured]` と実測 ms を添える(tuning-constants)。D0 の測定項目に「測るもの」を書く。
- journal・報告: `ObservationSource` は journal の `AnyObservation` と `DriftCorrectionFixture` に乗るので、新しい種類で古い再生ツールが読めなくなる点を CHANGELOG に書く。ADR-271 D5 の `indicator_evidence` に件数を足すなら `bug_report.rs` の `SCHEMA_VERSION` と report worker を同じ PR で更新する。
- CI(Windows): NULL 型の窓(`unavailable` 強制ではなく、実学習で降格する状態)で、ObserverPoll が来ない間に指標が belief を決めること、読める窓では belief が変わらないこと、GJI の窓で I/O 推論の ObserverPoll が居るとき指標が負けること(B1 を期待どおりに固定)。

## 段階

1. 核(`awase-windows-core`): D1 の源・除外・reducer の分岐・同一性、D2 の分岐、`classify_*`、単体テストと golden。**`ui_max_level` は上げない**。
2. **前提**: 一太郎(または報告者)の窓が NULL 型であることの確認(ADR-261 D0-2)。確認できなければ以降は止める。
3. メニュー: 報告口と D4 の `idChild` 条件。CI 通過後に `menu` の `ui_max_level` を `Unobserved` へ。
4. 字形: ADR-261 の段階 0〜1 が済んでから(別 PR)。D4 の柵を入れる。
5. `all`: 便益の場面を journal で示せたら `All` へ。示せなければ上げない(ADR-271 D2b)。

## 未解決(所有者)

- **Q1**: D2 の順位(指標は新鮮な Medium 以上に必ず負ける)で始めてよいか。代案は、`unobserved` に限り指標を Medium と同順位にして「新しい方が勝つ」とする。GJI の I/O 推論や嘘型の窓にも効くが、遅れた古い指標が正しい読みを潰す害を許す。
- **Q2**: TSF ネイティブ窓の除外(D3)を外してよい条件(便益の示し方)。実行時に TSF と学習した窓の扱い。
- **Q3**: メニューを先に `Unobserved` へ上げる順序の価値(開いた瞬間の 1 回だけで、`idChild` 条件のため報告が通る頻度が小さい)。
- **Q4(配線 PR で決める)**: TTL が切れたときの belief の戻り先(戻すか、次の確かな観測まで保つか)。

## 関連

[ADR-271](271-indicator-evidence-settings-levels.md)(設定と入口条件)・[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md)・[ADR-264](264-indicator-menu-mode-evidence.md)(証拠源)・
[ADR-233](233-stale-high-observation-beats-newer-in-most-recent-trusted.md)(`most_recent_trusted` の順位)・[ADR-268](268-mozc-shm-belief-connection.md)(High の源を入れた前例。TTL・除外・identity)・
[ADR-263](263-episode-start-key-hold-and-language-decision.md)(述語の出所)。
