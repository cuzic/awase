---
id: ADR-273
title: |-
  入力インジケーターの 2 つの証拠源(字形・右クリックメニュー)を belief へ入れる配線の入口条件と形(ADR-271 の第 3・第 4 段階)
summary: |-
  ADR-271 は設定の形(off/observe/unobserved/all)と実効値までを決め、belief への配線を別 ADR に残した。本 ADR はその配線の契約を決める。
  ①配線するのは IME の開閉だけ(入力モードは shadow のまま)。新しい ObservationSource(IndicatorMenu・TaskbarGlyph)を Medium・BeliefOnly で入れ、寿命は窓(フォーカス)の範囲とし、
  drift correction・read_back(Converged を含む)・reducer の drift 更新から明示的に外す(予測の照合は呼ぶ。遅れた指標が予測を消さないよう D4 の柵を通す)。
  ②「Medium が High を追い越さない」は順位で守る: `resolve_open_at` の中に、`derive_any` の後・`most_recent_trusted` の前の独立した分岐を足し、指標は、新鮮な Medium 以上(指標を除く)が無く、かつ指標より新しく失効していない Medium 以上も無いときだけ採る。
  帰結として、指標が効くのは **IMM が NULL を返して ObserverPoll が来ない窓**に限られ、IMM が嘘の値を返す窓・GJI の I/O 推論が ObserverPoll を書く窓では負ける(B1)。便益は小さく、その窓の型(一太郎)は未確定(ADR-261 D0-2)なので、段階 2 の前提にする。
  ③「観測の無い窓」の述語は `is_demoted_by_learning`(=NULL 型)を採る。④遅れ対策: 字形は最後のモード系打鍵から最大遅れ+余裕が経つまで報告しない(必須)。窓の同一性と FocusGen・前面の一致を照合する。
status: |-
  起草(2026-10-20)。実装なし。Opus レビュー round 1(Blocker 3・Major 10・minor 10)・round 2(Blocker 1・Major 3・minor 5)・round 3(Blocker 1・Major 3・minor 4)・round 4(収束の確認: Blocker/Major なし、minor 6)を反映。**確定可**(Opus round 4、2026-10-20)。所有者判断 Q1・Q2・Q3・Q5 は 2026-10-20 に決定済み。決定は案。所有者判断 Q1〜Q3 は本文末尾。
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
本 ADR は開閉だけを扱い、**入力モードへの配線は別 ADR**とする(入れる場合は同じ契約を書く: 順位・寿命・D4 の柵・`observation_source_guard`)。それまで、`unobserved`/`all` でもモードは journal の shadow にだけ出す。

### D1. 観測源の形と、読み手・reducer の除外

- 新しい `ObservationSource::IndicatorMenu` と `TaskbarGlyph`。`confidence = Medium`、`authority() = BeliefOnly`。`evidence.rs` の `declare_evidence!` に `BeliefPool` として足す(INV-38 の const assert)。`PerSourceObservations` の get/set/iter/`clear_all` も更新する。
- 入口は証拠源ごとに `ImeStateHub` の 1 メソッド。`classify_*` の純粋関数が「読み → 報告するか」を決め、`ImeEvent` → `reduce` を通す(`ObserverReported` を経由しない近道は `ime-belief-architecture.md` 違反)。`desired_open`・`IntentStore` は書かない。MozcShm の追随(ADR-268 D3)は Medium では採らない。
- **reducer の源分岐(M1・N1)**: `ime_model.rs` の `ObserverReported` アームは源を見ずに `update_drift(desired_open, observed.open())` と `reconcile_key_effect_open` を走らせる。指標源については **`update_drift` を呼ばない**分岐を `reduce` の中に足す(殻に近道を作らない)。呼ぶと、指標が drift を `None` に戻して正当な補正を遅らせる(`drift_duration` も巻き戻る)。同じ穴は `MozcShm` にも既にある(別 BUG として切り分ける)。
  **`reconcile_key_effect_open` は呼ぶ**(N1)。予測(`key_effect`)には鮮度も TTL も無く、消える経路は Medium 以上の観測による照合・フォーカスの変化・明示意図の 3 つだけである。NULL 型の窓にはもともと予測を照合する Medium 以上の観測が来ないので、指標まで照合しないと、モードキーを 1 回押した時点から予測が `resolve_open_at` の段 2 にフォーカス中ずっと居座り、指標が二度と効かない(外から変わったときに古い予測が勝つ)。
  代わりに「遅れた指標が正しい予測を消す」は D4 の柵を `classify_*` で必ず通してから報告することで防ぐ(柵の後なので押す前の状態を映した指標は届かない)。
- **読み手の除外**: `evaluate_drift` の除外リスト、`read_back` の `AnyFreshEvidence`(`EXCLUDED_FROM_ANY_FRESH_EVIDENCE`)に加え、**`ReadBackQuery::Converged`(`most_recent_trusted_after`、除外が空)にも除外を持たせる**(M2。`_after` は private なので `ReadBackQuery` 側に除外を持たせる)。`Converged` を直さないと、遅れた字形の「開」が再送の収束(`Confirmed`)に使われる。
  `ime_event.rs` の doc が言う `platform_state.rs::check_drift_correction` の `matches!` は存在しない(`check_drift_correction` は `evaluate_drift(..).ok()` だけ)ので、doc 側を直す(m1)。
- **同一性(M8)**: `derive_filtered` の `is_identity_ok` に新しい源を足す(epoch+hwnd)。さらに、読んだ時点と適用時点で `FocusGen` が変わっていれば捨て(ADR-261 決定 5・ADR-264 決定 4)、前面窓が追跡中の窓と一致している(`lang_detect_focus_in_sync` と同じ型)ことを `classify_*` の入力にする。Alt+Tab 直後は `demoted_by_learning`・focus が前の窓の値のことがあり、場面①(フォーカス直後の字形)はちょうどそこに当たる。
- **寿命(N2。round 1 の「TTL ≤ 3 秒」は撤回)**: 指標の寿命は**窓(フォーカス)の範囲**とする(`clear_on_focus_change`。同じプロセスの中の窓の変化は下の消去で守る)。短い TTL にしない理由: 報告は散発的(字形はフォーカスと最初の打鍵まわり、メニューは 1 回)で、3 秒で消すと、指標が guess と違う値を出すたびに利用者の操作なしに `effective-open-flip` が入るとき・戻るときの 2 回起きる。直接 `desired_open` を書かないまま「次の確かな観測まで保つ」には、寿命を窓の範囲にするしかない。
  失効は**書き込み時の削除**で行う(決定的で journal の再生でも同じになる。読むときに無視する方式だと、MozcShm が失効したあとに数分前の指標が復活する): 字形の `Miss`、**指標以外の Medium 以上の観測を記録するとき**(`record_any` の中で指標の 2 枠を消す。そちらが新しいので時刻順でも勝つ。**指標を記録するときは、もう片方の指標の枠だけを消す**〈新しい方が勝つ。メニューと字形が食い違って同時に残らない。m2〉)、フォーカスの変化(`clear_on_focus_change`)、**同じプロセスの中の窓の変化**(`update_focus_window` が source を限定して指標の 2 枠を消す。`derive_filtered` の `is_identity_ok` は指標を読まないので、照合ではなく消去で同一性を守る。R2)。モード系の打鍵は予測が `resolve_open_at` の上位に載ることで置き換わる(指標は失効させない。予測が照合で消えたあと、指標が古ければ次の項の規則で負ける)。
  `derive_any` の鮮度窓(3 秒)は指標に対しては働かない前提で、指標は `derive_filtered` から外し、D2 の独立した分岐でだけ読む。`most_recent_trusted` には、下の D2 の述語の (1)(新鮮な Medium 以上が居る)を満たす間は指標を入れない(Medium 同士の衝突を、指標の新しさだけで決めさせない)。**Q4(戻り先)はこれで「戻さない」と閉じる**。
- **HWND キャッシュ経由の残り(M4)**: `runtime/focus_tracking.rs` が窓を離れるときに `effective_open()` を HWND キャッシュへ保存し、戻ると `HwndCacheRestored` が `desired_open` に書く。指標で決まった belief は、窓を往復するとキャッシュ経由で `desired_open` に**残る**(指標の寿命は窓の範囲なので、戻った窓では指標自体は消えている)。窓を離れるときの `effective_open()` はほぼ必ず指標の値になるので、「`desired_open` は書かない」は事実上「**直接は書かない**」に弱まる。既存の源にも共通の性質なので許容する。journal で追うには、`HwndImeSnapshot` が bool しか持たないので、保存時に「指標由来」を journal へ出す別の行を足す(キャッシュ側に源を持たせる変更は採らない。n3)。

### D2. 「Medium が High を追い越さない」は順位で守る。段の置き場と勝ち負けの表

「High」は**新鮮な High**(3 秒以内、`derive_any` が即決する)を指す。新鮮でない High(フォーカス時の `ImmCrossProbe` は `expires_at: None` で残る)は、ADR-233 のとおり新しい Medium に負ける。指標が負けるべき相手は「新鮮な Medium 以上」であって、古い High ではない(M10)。

- **配置(M3)**: `resolve_open_at` の中に、`derive_any` と `most_recent_trusted` の間の**独立した分岐**として置く。`derive_any`(`derive_filtered`)の中には入れない: `derive_any` は `ModeKeyPassedThrough{align_desired}` と `align_placeholder_desired` が `desired_open` に書く根拠なので、中に入れると指標が `desired_open` を書き、D1 の「書かない」が破れる。
  この分岐は新しい `BaseDecision` の種類(`[effective-open-flip]` の出力と journal)と、`fallback_shadow` の早期 return の条件の更新を伴う(指標で決まった場面を「フォールバック」と数えない)。
- **指標を採る述語は 1 本**: 指標を採るのは、**(1)新鮮な Medium 以上(指標を除く)が 1 件も無く、かつ (2)指標より新しく、失効していない Medium 以上(指標を除く)が無い**とき。(1) が Medium 同士の衝突(`derive_any` が `None`)を段 3 から外し、(2) が「新鮮でなくても指標より新しい観測には負ける」(ADR-233 の時刻順)を残す。失効済みの観測は数えない。`most_recent_trusted` に指標を入れない条件も (1) と同じ述語を使い、段 3 と同じ境界を共有する。書き込み時の削除があるので (2) は常に真で冗長だが、削除の実装が漏れたときの防御として残す(実装で `most_recent_trusted` が指標を常に除く形にして述語を 1 箇所にしてもよい。m1)。Low(`HeuristicDefault`・`FocusProbe`)は Medium 以上に入らないので、指標が勝つ。
- **時刻の経過で belief が変わる(d5)**: t=10 に ObserverPoll(GJI の I/O 推論で開)、t=11 に柵を通った指標(閉)が来ると、t=13 までは ObserverPoll が新鮮で開、t=13 に ObserverPoll が新鮮でなくなると (1)(2) を満たし、入力が何も無いのに閉へ変わる(`effective-open-flip`)。向きは正しいことが多いが、journal と `[effective-open-flip]` に「鮮度切れで繰り上がった」と理由を出す。

| 同じ窓に居る相手 | 指標の勝ち負け | 理由・備考 |
| --- | --- | --- |
| 新鮮な High(`ImmCrossProbe`・`MozcShm`・直接の読み) | 負ける | `derive_any` が即決。指標は段に降りてこない |
| 新鮮な ObserverPoll(IMM の読み。値を返す窓) | 負ける | 嘘の値を返す窓でも、読み値が新鮮なら指標は採られない(**嘘型は救えない**、D3) |
| 新鮮な ObserverPoll(GJI の I/O 推論。`observe_gji_after_focus` が `Some(true)` の向きだけ書く) | 負ける | GJI の窓では I/O 推論が ObserverPoll を書き続けるので、指標は閉の読みでも負ける。BUG-203 型には効かない(B1) |
| `ConvOpenInference`(Medium) | 負ける | 新鮮な間は無競合多数決に参加する。conv ビットは IMM が読める窓でしか取れないので NULL 型の窓にはそもそも居ない(n1) |
| ObserverPoll(日本語 IME でない。`classify_poll_outcome` が `is_japanese_ime == Some(false)` で毎回 false を記録) | 負ける | 日本語 IME でないときに指標が負けるのは正しい(n2) |
| ObserverPoll が来ない(IMM が NULL・タイムアウト) | **勝つ** | 指標が効く場面。これが NULL 型の窓(D3) |
| ADR-263 の言語判定(窓の切替直後の打鍵の保持・ひらがなキーの送信) | **指標は ADR-263 の書き込みの前提(`belief_open`・`lang_detect_engine_active`)になる** | ADR-263 の対象の窓(降格した窓)は本 ADR の D3 と同じ窓で、belief を直接読んで IME へキーを送る。`BeliefOnly` が守るのは `derive_actuating`・`open_warrant` だけで、この経路は保証の外にある。指標が開なら送信の前提が成り立ち、閉なら保持も送信も止まる。許すかは所有者判断(Q5) |
| 打鍵予測・明示意図・`IntentStore` | 負ける | `resolve_open_at` の上位。利用者が awase のキーで切り替えた窓では、その後外から変わっても `IntentStore` の TTL(ON 10 秒・OFF 30 秒)の間、指標は効かない(M6) |
| `HeuristicDefault`・`FocusProbe`(Low) | 勝つ | Low は Medium 以上に負ける既存の規則 |

- **帰結(B1・N4)**: 指標が効くのは「IMM が NULL を返して ObserverPoll が来ず、新鮮な他の Medium 以上も無い窓」に限られる。重ねると、(a)NULL 型に降格した窓、(b)新鮮な ObserverPoll が無い(GJI では I/O 推論が打鍵ごとに書くので最後の打鍵から 3 秒以上)、(c)`last_intent` が無い(awase の親指キーで切り替える利用者ではフォーカス中ずっと残る)、(d)`IntentStore` の項目が切れている、(e)予測が無い、をすべて満たす場面で、実質は「NULL 型の MS-IME の窓に入った直後、最初の IME 切替キーを押す前」に近い。**配線 PR の前に**、段階 1 の shadow の journal で(a)〜(e)を満たす読みの件数と、そのうち belief と食い違った件数を数える。件数と同時に、その間の belief との食い違いの件数、食い違った後に利用者が打ち直した件数も出す。0 に近ければ、段階 2 そのものを見送る判断を Q1 の材料として所有者に出す。ADR-271 の `unobserved` が想定した「嘘をつく窓」の一部(IMM が値を返す嘘型、GJI の I/O 推論が走る窓)は**救えない**。これを承知で順位を守るのは、Medium が High と Medium を上書きする害(遅れた古い指標が正しい読みを潰す)が、効く窓を増やす便益より大きいと見るため。Q1 で所有者に確認する。
- **`all` の便益(M9・m10)**: 読める窓でも ObserverPoll が `None` の窓(TSF ネイティブ・静的な Imm32Unavailable の Chrome・Edge・Windows Terminal・VS Code・WezTerm。`classify_poll_outcome` の `is_tsf_native` 分岐)では指標が belief の主な源になる。打鍵量の大半を占める窓で害の面積が大きい。
  ところが D3 の `is_effectively_tsf_native` の表は `CoreWindow`・`XamlExplorerHostIslandWindow`・`InputSite`・`CASCADIA_HOSTING_WINDOW_CLASS`・`org.wezfurlong.wezterm` の 5 つで、`Chrome_WidgetWin_1`(Chrome・Edge・VS Code・Electron)を含まず(`class_names.rs` のテストが明示的に固定)、TSF ネイティブ除外では Chrome 系を覆えない(N3)。したがって **`all` の対象は、静的に `Standard` のプロファイルの窓(降格したものを含む)に限り、静的に `Imm32Unavailable` の窓は除外する**(Chrome 系で便益を示せるまで)。タイムアウトが続く間に指標を入れることは、BUG-158 追補の「タイムアウトは証拠ではないので belief を保つ」の改訂になるので、その ADR にも追記する。配線 PR は `all` が効く場面を journal で 1 つ以上示す(示せなければ `ui_max_level` を `Unobserved` までにとどめ、画面に `all` を出さない)。
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
- **字形を読む場面を 1 つ足す(R3。ADR-261 決定 5 の改訂)**: 決定 5 の場面(①フォーカス・前面の変化、②その窓の最初の打鍵まわり)だけだと、モード系の打鍵(予測が付く)は「最初の打鍵」の後に来ることがほとんどで、柵の経った字形を読む場面が無く、NULL 型の窓では予測を照合する報告が来ない(予測がフォーカス中ずっと居座る)。
  そこで③**モード系の打鍵の後、柵が経ったあとの最初の打鍵(またはタイマー 1 回)**で読む。読む回数・抑止(決定 5)の対象に入れる。ただし決定 5 の「同一 `FocusGen` で結果が変わらなければ再読しない」は、モード系の打鍵では `FocusGen` が変わらないので③と衝突する。**モード系の打鍵(`KeyEffectPredicted`)は再読の抑止を解く**例外を決定 5 の改訂に書く(m3)。③の「最初の打鍵」はモード系でない打鍵に限り(その打鍵自体がモード系なら柵は計り直し)、取り込みはフックの経路の外で行い結果は次の判断から効く(m4)。③は画面の取り込みを打鍵に結びつけ、ADR-261 決定 7 の「キーロガー+スクリーングラバー」の型に①②より近づくので、決定 7 の改訂として 1 行触れ、打鍵との時間的な相関を弱める「タイマー 1 回」の側を既定にする(m5)。足さない場合の帰結は「字形は予測を照合しない(予測が残る間は効かない)」で、D2 の(e)の条件にそう書く。
- **柵は `classify_*` で必ず通す(N1)**: D1 が `reconcile_key_effect_open` を呼ぶ前提なので、柵の後でない指標は報告しない。
- **メニュー**: 報告するのは、`MENUSTART`〜`MENUEND` の間に非 0 の `idChild` が 1 度も出ておらず、かつ **`MENUEND` が最後のモード系打鍵より柵の分だけ後**のときに限る(出ていれば選択された可能性があり読みは古い)。残る穴は ADR-264 のまま未解決として引き継ぐ: ①閉じるクリックがインジケーターへの左クリックとして届き、`idChild` なしで切り替わる(M0-10)、②キーボード選択で `idChild` が出るか(M0-3)、
  ③カーソルを動かせばほぼ必ず非 0 の `idChild` が出るので、報告が通る頻度は小さい(Q3)、④BUG-202 修正(`#32768` を窓の切替から外した)後も `idChild` が同じイベントで取れるか。1 回限りなので、柵で落とした報告は捨てられ、何も残らない。
- **GJI の I/O 推論との順序(BUG-203)は本 ADR の範囲外**。ObserverPoll(Actuating)の挙動を変える修正で、実 Chrome+GJI で再現せず優先度を下げた件(2026-10-18)。BUG-203 の側で扱う。

### D5. 回帰テストと確認

- `observation_store.rs`: ①新鮮な High+新しい指標 → High、②新しい指標+新鮮な ObserverPoll で食い違い → ObserverPoll、③ObserverPoll が来ない・失効、指標が新鮮 → 指標、④フォーカスの変化・字形の `Miss`・より新しい Medium 以上で失効する。新鮮な Medium 以上が居る間は `most_recent_trusted` に指標が入らない、⑤`evaluate_drift`・`read_back` の**両分岐**(`AnyFreshEvidence`・`Converged`)が指標を根拠にしない、⑥古い(新鮮でない)High+新しい指標 → 指標(ADR-233 との整合を固定)、⑦′メニューと字形の食い違いで新しい方が勝つ(指標同士は互いの枠を消す)、⑦新鮮な Medium 同士の衝突のとき指標は衝突を決めない、⑧同じプロセスの中の窓の変化・Medium 以上の記録で指標の 2 枠が消える、⑨鮮度切れで繰り上がった場合の理由が journal に出る。
- `ime_model.rs`: reducer が指標源について `update_drift` を呼ばない。指標源でも `reconcile_key_effect_open` が呼ばれ、柵を通した指標は予測を正しく照合する。柵の前の指標は報告されず予測を消さない。字形は最後のモード系打鍵から柵が経つまで報告されない。reducer の分岐は `ObserverReported` のアーム内で源を見て分け、`reduce_` 接頭辞のヘルパーに出す場合は名前をそろえる(`reduce_helpers_are_called_only_from_reduce_body`。n5)。
- `architecture_guard.rs`: 指標源の報告口の生成点の件数固定(`mozc_shm_reader_touches_belief_only_through_report_mozc_shm` と同じ型)。カウンタ名に `belief_` を使わない。`observation_source_guard`(dylint)は D0 により入力モードを扱わないので更新しない。
- 新しい tuning 定数(字形の打鍵後の柵の α・必要なら他。寿命は窓の範囲なので TTL 定数は無い)は `#[measured_macro::measured]` と実測 ms を添える(tuning-constants)。D0 の測定項目に「測るもの」を書く。
- journal・報告: `ObservationSource` は journal の `AnyObservation` と `DriftCorrectionFixture` に乗るので、新しい種類で古い再生ツールが読めなくなる点を CHANGELOG に書く。ADR-271 D5 の `indicator_evidence` に件数を足すなら `bug_report.rs` の `SCHEMA_VERSION` と report worker を同じ PR で更新する。
- CI(Windows): NULL 型の窓(`unavailable` 強制ではなく、実学習で降格する状態)で、ObserverPoll が来ない間に指標が belief を決めること、読める窓では belief が変わらないこと、GJI の窓で I/O 推論の ObserverPoll が居るとき指標が負けること(B1 を期待どおりに固定)。

## 段階

1. 核(`awase-windows-core`): D1 の源・除外・reducer の分岐・同一性、D2 の分岐、`classify_*`、単体テストと golden。**`ui_max_level` は上げない**。
2. **前提(字形のみ。所有者判断 Q3 によりメニューには課さない)**: 一太郎(または報告者)の窓が NULL 型であることの確認(ADR-261 D0-2)。確認できなければ字形は止める。メニューでは D2 の(a)〜(e)の件数を journal に出し続けて事後に評価する(ゲートにしない。0 に近ければ `menu` を `Observe` へ戻す判断を所有者に出す)。
3. メニュー(**先行**): 報告口と D4 の `idChild` 条件。CI 通過後に `menu` の `ui_max_level` を `Unobserved` へ。便益が小さい可能性は承知のうえで(Q3)、測定を待たずに進める。
4. 字形: ADR-261 の段階 0〜1 が済んでから(別 PR)。D4 の柵を入れる。
5. `all`: 便益の場面を journal で示せたら `All` へ。示せなければ上げない(ADR-271 D2b)。

## 未解決(所有者)

- **Q1(決定済み 2026-10-20)**: 指標は新鮮な Medium 以上に必ず負ける順位で進める(代案の同順位は採らない)。
- **Q2(決定済み 2026-10-20)**: TSF ネイティブ窓は当面除外し、便益を示せたら外す。`all` は静的に `Standard` の窓に限る。
- **Q3(決定済み 2026-10-20)**: メニューを先に `Unobserved` へ上げる。測定を待たない(字形のみ D0-2 が前提)。
- **Q5(決定済み 2026-10-20)**: 指標由来の belief のとき、ADR-263 の書き込みは**止める**(既定のとおり)。指標由来の `BaseDecision` を持つ間は ADR-263 のひらがなキー送信と保持を行わない。
- ~~Q4(TTL 切れの戻り先)~~: 寿命を窓の範囲にして「戻さない」と閉じた(D1)。

## 関連

[ADR-271](271-indicator-evidence-settings-levels.md)(設定と入口条件)・[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md)・[ADR-264](264-indicator-menu-mode-evidence.md)(証拠源)・
[ADR-233](233-stale-high-observation-beats-newer-in-most-recent-trusted.md)(`most_recent_trusted` の順位)・[ADR-268](268-mozc-shm-belief-connection.md)(High の源を入れた前例。TTL・除外・identity)・
[ADR-263](263-episode-start-key-hold-and-language-decision.md)(述語の出所)。
