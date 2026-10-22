---
id: ADR-273
title: |-
  入力インジケーターの 2 つの証拠源(字形・右クリックメニュー)を belief へ入れる配線の入口条件と形(ADR-271 の第 3・第 4 段階)
summary: |-
  **【2026-10-20 改訂】右クリックメニューは belief に使わない(実機で信頼できず、所有者判断で断念。下の「メニューの配線を見送る」)。以降の配線の対象は字形だけで、メニューに関する記述は経緯として残す。**
  ADR-271 は設定の形(off/observe/unobserved/all)と実効値までを決め、belief への配線を別 ADR に残した。本 ADR はその配線の契約を決める。
  ①配線するのは IME の開閉だけ(入力モードは shadow のまま)。新しい ObservationSource(IndicatorMenu・TaskbarGlyph)を Medium・BeliefOnly で入れ、寿命は窓(フォーカス)の範囲とし、
  drift correction・read_back(Converged を含む)・reducer の drift 更新から明示的に外す(予測の照合は呼ぶ。遅れた指標が予測を消さないよう D4 の柵を通す)。
  ②「Medium が High を追い越さない」は順位で守る: `resolve_open_at` の中に、`derive_any` の後・`most_recent_trusted` の前の独立した分岐を足し、指標は、新鮮な Medium 以上(指標を除く)が無く、かつ指標より新しく失効していない Medium 以上も無いときだけ採る。
  帰結として、指標が効くのは **IMM が NULL を返して ObserverPoll が来ない窓**に限られ、IMM が嘘の値を返す窓・GJI の I/O 推論が ObserverPoll を書く窓では負ける(B1)。便益は小さく、その窓の型(一太郎)は未確定(ADR-261 D0-2)なので、段階 2 の前提にする。
  ③「観測の無い窓」の述語は `is_demoted_by_learning`(=NULL 型)を採る。④遅れ対策: 字形は最後のモード系打鍵から最大遅れ+余裕が経つまで報告しない(必須)。窓の同一性と FocusGen・前面の一致を照合する。
status: |-
  起草(2026-10-20)。**段階 1(核の実装、本番への配線なし)を実装中(2026-10-20、feat/adr273-indicator-source-core)**。Opus レビュー round 1(Blocker 3・Major 10・minor 10)・round 2(Blocker 1・Major 3・minor 5)・round 3(Blocker 1・Major 3・minor 4)・round 4(収束の確認: Blocker/Major なし、minor 6)を反映。**確定可**(Opus round 4、2026-10-20)。所有者判断 Q1・Q2・Q5 は 2026-10-20 に決定済み。**メニューの配線は 2026-10-20 に断念(Q3 の「メニュー先行」は取り下げ)、メニューの読み取り(ADR-264 段階 1 の shadow)そのものも 2026-10-22 に撤去**。決定は案。所有者判断 Q1〜Q3 は本文末尾。
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

## メニューの配線を見送る(2026-10-20、所有者判断)

右クリックメニュー(ADR-264)を belief の証拠にすることは断念する。`menu` の設定は `Observe`(journal・不具合報告への記録)までで、`Unobserved`/`All` へは上げない。理由は dragonflyg4(実機、Windows 11・日本語・GJI、1 台)での実況検証:

- **.NET の UIA(`System.Windows.Automation`、UIA2)では項目が読めなかった**: 手動で開いた `#32768`(約 236×266)の子要素は、検出の 0・0.5・1.5 秒後も、`MenuOpened`/`WindowOpened` を購読しても、RawView でも 0 件。ルートからの `MenuItem` 検索も 0 件。ADR-264 の CI の測定(PowerShell の UIA2 で `MenuItem`+`IsSelected`)に当たる読み方は、実機の GJI では成立しなかった。**ただし awase の実装(`observer/indicator_menu.rs`)が使うのはネイティブの `IUIAutomation`(UIA3)+`LegacyIAccessible` の状態で、この経路で実機が読めるかは未確認**(本測定の範囲外。断念の根拠は下の「読みが元の窓の状態と一致しない」で、API には依らない)。
- **MSAA(`IAccessible`)では読めた**: `AccessibleObjectFromWindow(hwnd, OBJID_CLIENT)` で 13 項目と `STATE_SYSTEM_CHECKED` が取れる。ただし `AutomationId` は無く、項目の位置(1=ひらがな 2=全角カタカナ 3=全角英数 4=半角カタカナ 5=半角英数 6=直接入力 …)で引くことになる。
- **読みが元の窓の状態と一致しない**: 自前の窓に GJI のモードを順に設定し、手動の右クリックでメニューを読んで IMM の実状態と比べた。1 回目は 6/6、2 回目は前面窓を記録して 4/12 一致。打鍵(`ka`)の確認を足した 3 回目は、正解が安定せず判定不能だったが、メニューは 12 回中 10 回が「ひらがな」を指した(打鍵が英数の回も同じ)。前面窓は自前の窓のままの回が多く、前面窓のずれでは説明できない。
  有力な仮説(未確認): インジケーターのクリックで Explorer 側の TSF コンテキストへフォーカスが移り、メニューのチェックが元の窓でなくタスクバー側の既定(ひらがな)を映す。
- 注入した右クリック(`mouse_event`、移動後 80 ms・押下 100〜150 ms・絶対座標)ではメニューが開かなかった(手動では開く)。自動の測定が成立しない。
- 実機の発見(字形の実装にも関わるので残す): ①名前が「トレイ入力インジケーター」で始まる要素が **2 つ**ある(「あ」のボタンは末尾が「右クリックで IME のオプションを表示します」、青い丸のボタンは名前に IME 名が入る別物)。「接頭辞が一致する最初の要素」は誤る。②DPI 非対応のプロセスでは UIA の矩形がずれる。③ホバーのツールチップ窓(`Xaml_WindowedPopupClass`)が出ている間は `Shell_TrayWnd` 配下の要素が見えなくなる(26 → 4)。④タスクバーの高さ(72 px/24 px)は状態で変わる。

これらにより、ADR-273 D1〜D5 のうちメニューに関する部分(`IndicatorMenu` の源・D4 のメニューの `idChild` 条件・段階 2 のメニュー行)は実装しない。字形の配線(`TaskbarGlyph`)は影響を受けない。ADR-264 の段階 2(belief への配線)も同じ理由で取り下げ、shadow(journal)は残す。

## 段階 1 の実装状況(2026-10-20、PR 〔核の実装・配線なし〕)

核(`awase-windows-core`)だけを実装した。**本番には配線していない**(`ui_max_level` は `Observe` のまま。`architecture_guard::taskbar_glyph_is_not_reported_from_production_yet` が、殻に `TaskbarGlyph`・`from_glyph(`・`fence_glyph(` が 0 件であることを固定する)。

- 実装は D1・D2 の設計を、**default-deny** の形にした: 字形の枠(`PerSourceObservations::taskbar_glyph`)を `iter()` に含めないので、`derive_any`・`most_recent_trusted*`・`read_back`(`Converged`・`AnyFreshEvidence` の両方)・`consensus`・`evaluate_drift` の全読み手から既定で除外される(除外リストを足すより漏れにくい。`evaluate_drift` の除外リストにも防御として明記した)。読み口は `ObservationStore::indicator_reading`(D2 の述語 (1)(2)・窓の同一性)と、`PerSourceObservations::indicator()` の 2 つだけ。
- 採用は `resolve_open_at` の独立した分岐(`BaseDecision::IndicatorGlyph { promoted_by_staleness }`)。鮮度切れで時刻の経過だけで繰り上がったとき `promoted_by_staleness` が付く(D2 d5・D5 ⑨)。`desired_open` は書かない。
- 寿命は書き込み時の削除: 指標以外の Medium 以上を**窓の同一性が合うときだけ**記録するとき・窓の変化(`update_focus_window`、hwnd が変わったときだけ)・`clear_on_focus_change`・`ImeEvent::IndicatorGlyphMissed`(字形が `Miss` になったとき)で消える。
- D4 の柵を核に入れた: **`FencedGlyph`**(構築子は `fence_glyph` だけ。モード系の打鍵から `GLYPH_KEY_FENCE_MS`=1300ms〔2026-10-21 に 1000ms から引き上げ。実機の押下から字形までの実測最大 0.885 秒+マージン 0.415 秒。`pending`〕・フォーカスの世代・前面の一致)が `Observed::<TaskbarGlyph>::from_glyph` の witness になる。**reducer も**、最後のモード系の入力(`UserImeSetIntent`・`ImeApplyRequested`・`KeyEffectPredicted`・`ModeKeyPassedThrough`。awase 自身の書き込みを含む)から `GLYPH_KEY_FENCE_MS` 以内の字形を記録も照合もしない(二重の柵。`ImeModel::last_mode_key_tick_ms` は予測の照合で消えないので、予測が 170ms 後に別の観測で消えた後でも効く〔Opus RM1〕。`FocusChanged` で消す。journal の再生でも決定的。`KEY_EFFECT_SETTLE_MS`=170ms では足りない〔B3〕)。`fence_glyph` の 3 つの事実は呼び出し側の自己申告なので、本当の防御は reducer 側。
- `promoted_by_staleness` は「字形が届いた時点では新鮮だった、字形と値の違う Medium 以上が、今は鮮度切れ」のときだけ付く(診断専用。GJI の窓で常に付いて過大に数えないため)。
- reducer は字形で `update_drift` を呼ばない(`reconcile_key_effect_open` は呼ぶ)。

### 段階 2a: 殻の入口を作る(2026-10-21、配線 PR 1。**まだ動かない**)

`ImeStateHub::report_indicator_glyph`(`state/platform_state.rs`)を、字形を belief に報告する**唯一の入口**として作った。`Hit` は `fence_glyph`(柵)を通したものだけ `ObserverReported(TaskbarGlyph, Medium)` にし、`Miss` は `IndicatorGlyphMissed` で枠を消す。呼び出し元は `runtime/indicator_glyph_shadow.rs` の 1 件だけで、`glyph_belief_in_scope()` が真のときだけ呼ぶ。**`EvidenceSource::ui_max_level()` は `Observe` のまま**なので、実効の段階が `unobserved` 以上にならず、本番では動かない(挙動は変わらない)。

- 対象の窓(`glyph_belief_in_scope`): 入力中継ツール・実質 TSF ネイティブの窓・**mozc-readable の共有メモリがある窓(R1)**は除外。`unobserved` は実測学習で降格された窓(D3)、`all` は静的に `Standard` の窓(Q2)。
- **R2・R6**: `belief_is_decided_by_glyph`(hub の層の最終的な理由。`IntentStore` の上書きを字形由来と数えない)が真のとき、見本のラベルを付けない(収集も一致の記録もしない)。
- `FenceCtx::focus_gen_unchanged`: 前面窓の変化の通知はフォーカス追跡より先に届くので、依頼の時点の同一性は切り替える前の窓のものになりうる。**1 回目(0.9 秒後)は処理した時点の同一性**を基準にする(追跡が追いついている=前面が一致のときだけ。`first_read_fence`)。2 回目(2.5 秒後)の基準は 1 回目の基準で、変わっていれば捨てる(Opus RE1)。
- **窓の述語は核の純粋関数**(`glyph_window_in_scope`・`glyph_scope_label`、Linux の単体テストで D3・Q2 の表を固定)。`current_app_profile()` は降格後の実効値を返すので、`all` は「実効 `Standard` **または**降格された窓」とする(降格は `Standard` からしか起きない。静的な `Imm32Unavailable` だけが除外される)。`all` は `unobserved` の対象を含む(Opus E1)。
- **窓の同一性は取り込みを処理した時点の `FocusFence`(epoch + hwnd)**: 1 回目は処理した時点(追跡が追いついているときだけ)、2 回目は 1 回目の基準の `ImeStateHub::focus_fence()` を `report_indicator_glyph` に渡し、今と違えば捨てる。同じプロセスの中の別の欄への移動(hwnd)も見る。`from_glyph` には報告の時点の fence でなく、この読みの fence を付ける(Opus E2)。1 回目が取り込めなかった・追跡が追いついていなかったときは基準が無く、2 回目は報告しない。
- **`Miss` の取り消し**は `TooFar`/`Ambiguous`(字形が表に無い状態に変わった)で、窓の判定を通ったときだけ。画像の不具合・一様な画像・表が空は何もしない。
- **R1 は二重**: 殻は共有メモリのブロックがある窓を除外し、hub も `MozcShm` の観測がある窓(失効したものを含む)では報告しない(遷移の穴)。
- **`observe` のままでも件数を数える**: journal の `IndicatorGlyphRecord` に `would_scope`(`unobserved`/`all`/`none`)・`would_report`(`reported`/`miss-changed`/`ignored`/`mozc-window`/`fenced:…`。`miss-changed` は取り消しの条件を満たしたの意味で、observe では枠が空)・`would_decide`/`would_flip`(柵を通る読みを今記録したら字形が belief を**決めるか**〔明示意図・予測・新鮮な Medium 以上が勝てば偽〕、決めるなら今の belief と**違うか**。`ImeModel::would_glyph_decide`。複製に本物と同じ `ObserverReported` を `reduce` で通す〔柵を過ぎた字形は予測を照合して消すので、D2 の (e)「予測が無い」は条件にならない〕。hub が `IntentStore` の上書きを掛ける〔D2 の (d)〕。基準が作れなかった読みも `would_scope` は出し、`would_report = fenced:no-baseline` とする)を出す(`preview_indicator_glyph`。belief には入れない)。**`ui_max_level` を上げる入口条件**: 実機の journal で `would_decide` が真の件数(D2 の (a)〜(e))を数え、0 に近ければ見送る。
- **見本に無い字形の形をログに出す診断**(2026-10-21): `TooFar`/`Ambiguous` の `Miss` のとき、前処理後の 2 値画像（20×20 の 2 枚。**元の画素ではない**。ADR-261 決定 7 の「画素は保存しない」を守る）を `awase::glyph_miss` の info に `[indicator-glyph] miss-shape dots=… m0=<20 行> m1=<20 行>` として出す。同じ形は 1 度だけ・起動ごとに最大 24 件。実機で曖昧になる窓（Chrome の非テキスト部など、見本に無い字形）の形を後から確かめ、見本に足すか決める材料。
- R1 の主は殻の `block_present`(共有メモリのブロックの有無)で、hub の `MozcShm` 観測の判定は遷移の補助(失効したものを含むので、同じ窓で TIP を替えてもフォーカスが変わるまで字形を使わない。安全側)。
- 未決: 実行時に TSF と学習した窓(`injection_mode_store.has_tsf`)の扱い(Q2)。R3(HWND キャッシュ経由の R2 の穴)は Q5 の実装時に一緒に閉じる。
- 残る入口条件(`ui_max_level` を上げる前): 一太郎(NULL 型)の確認(段階 2)、D2 の(a)〜(e)の件数を shadow の journal で数える、R3(HWND キャッシュ)・R4(hwnd の意味)の扱いを決める。

### 配線 PR の注意(Opus レビュー、2026-10-20)

- **R1 MozcShm が割れたとき**: ADR-268 は `tsf_open != effective_open` の間 MozcShm を報告しない。共有メモリが止まって TTL が切れたあとに字形(TIP の実状態を映す)が belief を決めると、ADR-268 が避けた向きへ倒れる。mozc 系の窓で割れている間(`AppliedDemoted{MozcShmSplit}` の間)は字形を報告しない。
- **R2 見本の自己学習のループ**: 殻の shadow(`indicator_glyph_shadow`)は `effective_open()` から見本のラベルを作る。字形が belief を決めるようになると、字形が決めた belief で字形の見本にラベルを付け、誤読が `glyph_samples.json` に永続化して自分を強化する(`agree_open` の集計も同語反復になる)。**`decided_by.base == IndicatorGlyph` のときは収集も一致の記録もしない**。`current_profile()` が降格を反映するかも確かめる。
- **R3 Q5 が HWND キャッシュ経由で抜ける**: 窓を離れるときの `effective_open()`(ほぼ字形の値)が HWND キャッシュに入り、戻ると `HwndCacheRestored` が `desired_open` を書く。その窓の `BaseDecision` は `DesiredFallback` になり、「字形由来の `BaseDecision` を持つ間は ADR-263 を止める」をすり抜ける。「指標由来」の印を HWND キャッシュ側で持つかを、Q5 の実装時に決める。
- **R4 hwnd の意味**: 字形はキーボードフォーカスのスレッド(`hwndFocus`)に追随する(ADR-261 D0-10)。報告の `hwnd` は `current_fence.hwnd` と同じ意味で取ること。`hwndFocus` の子窓が頻繁に変わるアプリでは、`FocusHwndUpdated` のたびに字形が消えて効く時間がほぼ無くなる。shadow の journal で回数を測る。
- **R5 柵**: 殻の `fence_glyph` だけでなく reducer にも二重に置いた(上記)。殻の `FenceCtx::since_last_mode_key_ms` の起点も reducer と同じ「最後のモード系の入力または awase の書き込み」に揃える。
- **R7 柵の値(D4 の入口条件)**: 2026-10-21 に dragonflyg4(GJI・自前の窓)で押下から字形が最初に変わるまでを 72 件測った(awase 停止 36 件=中央 0.53 秒・最大 0.885 秒、起動 36 件=中央 0.42 秒・最大 0.713 秒。前日の同手順は 0.09〜0.25 秒で日によって約 3 倍ぶれる)。旧値 1000ms は余裕 0.115 秒だったので 1300ms に上げた。**実 Chrome は測定済み**(同日、自前の Chrome・使い捨てプロファイル・textarea、Chrome の割り当てを避けて F13〜F18、awase 停止、36 件=中央 0.215 秒・最大 0.35 秒、外れ値なし。`glyph_latency_chrome.ps1`、ブランチ `ci/menu-match-live`)。ADR-261 D0-1 の外れ値(1.35〜2.3 秒)はフォーカス切替後の遅れで、押下からの遅れでは再現しなかった。**未測定は TSF のみの窓(UWP 型)での押下からの遅れ**(実機用のドライバが無い。CI の ADR-261 の測定では実 Chrome・tsf フォームで最大 0.68 秒)。配線の前に測るか、測れないなら 1300ms のまま `pending` で配線し、shadow の journal で押下から字形までの遅れの分布を見る。
- **R6 `IntentStore` の層**: `platform_state` の `effective_open` は `IntentStore` で上書きする。`resolve_open_at` の `decided_by` だけで「字形由来」と判定すると、`IntentStore` が上書きしている場面を誤って字形由来と数える。R2・R3・Q5 の判定は hub の層の最終的な理由で行う。

## 却下案

- **トグル系のキーの後、awase がメニューを不可視で開いて読む案**(所有者判断 2026-10-20 で却下): 他プロセス(IME 側)が出すメニューに DWM の cloak は使えず、画面外への移動・透明化はメニュー表示の後の対処で数フレーム見える。
  見た目を消しても、メニュー窓へのフォーカス移動(BUG-202)・選択直後の GJI の I/O 推論(BUG-203)・開くための右クリック注入またはカーソル移動は残る。ADR-261 が既に「背景で読むものではない」と却下した理由と同じ。
  トグル系のキーの後に読む用途は、侵襲の小さい字形の読む場面③(D4。モード系の打鍵の後、柵が経ったあと)で満たす。UIA の `Invoke` でカーソルを動かさずに開けるかは測っていない(再検討するなら先に測る)。

## 段階

1. 核(`awase-windows-core`): D1 の源・除外・reducer の分岐・同一性、D2 の分岐、`classify_*`、単体テストと golden。**`ui_max_level` は上げない**。
2. **前提**: 一太郎(または報告者)の窓が NULL 型であることの確認(ADR-261 D0-2)。確認できなければ字形は止める。D2 の(a)〜(e)の件数を shadow の journal で数える(0 に近ければ見送り)。
3. ~~メニュー~~: **断念**(下の「メニューの配線を見送る」)。`menu` の `ui_max_level` は `Observe` のまま上げない。
4. 字形: ADR-261 の段階 0〜1 が済んでから(別 PR)。D4 の柵を入れる。`glyph` の行を出し、`Unobserved` へ。
5. `all`: 便益の場面を journal で示せたら `All` へ。示せなければ上げない(ADR-271 D2b)。

## 未解決(所有者)

- **Q1(決定済み 2026-10-20)**: 指標は新鮮な Medium 以上に必ず負ける順位で進める(代案の同順位は採らない)。
- **Q2(決定済み 2026-10-20)**: TSF ネイティブ窓は当面除外し、便益を示せたら外す。`all` は静的に `Standard` の窓に限る。
- ~~Q3(メニューを先に上げる)~~: **取り下げ**。実機の測定でメニューが信頼できなかったため、メニューの配線自体を断念した。
- **Q5(決定済み 2026-10-20)**: 指標由来の belief のとき、ADR-263 の書き込みは**止める**(既定のとおり)。指標由来の `BaseDecision` を持つ間は ADR-263 のひらがなキー送信と保持を行わない。
- ~~Q4(TTL 切れの戻り先)~~: 寿命を窓の範囲にして「戻さない」と閉じた(D1)。

## 関連

[ADR-271](271-indicator-evidence-settings-levels.md)(設定と入口条件)・[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md)・[ADR-264](264-indicator-menu-mode-evidence.md)(証拠源)・
[ADR-233](233-stale-high-observation-beats-newer-in-most-recent-trusted.md)(`most_recent_trusted` の順位)・[ADR-268](268-mozc-shm-belief-connection.md)(High の源を入れた前例。TTL・除外・identity)・
[ADR-263](263-episode-start-key-hold-and-language-decision.md)(述語の出所)。
