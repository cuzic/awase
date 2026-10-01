---
id: ADR-208
title: |-
  明示的な IME キー(Ctrl+変換/無変換・かな・半角全角・漢字等)を押すと、内部状態が何であっても最大2回の押下で実 IME が一致する(固着ゼロの保証)
summary: |-
  所有者方針(2026-09-29 の「固着」の定義、2026-10-01 に保証として確認): モードずれ(awase の belief/予測/`applied` と実 IME の食い違い)が起きること自体は許容するが、明示キーを物理で押すと、内部状態がどうであっても
  『何度押しても変わらない』固着にならない。ADR-208 は当初 ADR-205 から切り出した草稿(GJI × Blind 窓で古い `applied` により絶対指定キーが省略され続ける件)だったが、Opus の網羅監査(`docs/tasks/adr208-liveness-audit-2026-10-01.md`、
  develop `042b5ee8`)で、固着は stale `applied` だけでなく `is_japanese_ime()==false`(S-2、全窓・全 IME で Engine のコンボと 0x16/0x1A が Unwarranted)等にもあると分かったため、保証として採用する。
  保証は1押下ごとの配送不変条件 INV-L1(物理が届くか awase が書くかのちょうど一方)と収束条件 INV-L2(絶対指定は1回、トグルは2回以内)。設計は新しい gate を足さず、既存の省略・授権(`applied` の already-matched・warrant)を緩める
  D1〜D4+押下 id による BUG-113 の二重送信防止。検証は純粋な決定関数の全列挙テスト(Linux)→ CI の drift × キー行列。L0〜L3 を v2 のブロッカーにする。
status: |-
  採用(2026-10-01)。**L0 実装済み**(挙動不変の切り出しと全列挙テスト・反例 golden。詳細は「L0 実装メモ」節)、L1 から未実装。所有者決定: 全窓で必ず書く(物理押下ごとに1回、同一押下の二重送信のみ防ぐ)、『解消』は最大2回の押下、検証は決定表の網羅テスト→CI の行列、InputRelay は『awase が actuate しない窓では開閉キーを握りつぶさず素通し』、MS-IME × 実 Chrome の `VK_IME_OFF` が効かない件(BUG-172 対照)は例外として明記し別機構は後で検討、L0〜L3 を v2 のブロッカーにする(L3' は実機 A/B が条件で v2 のブロッカーにしない)。
related_adr:
  - "ADR-205"
  - "ADR-206"
  - "ADR-098"
  - "ADR-108"
  - "ADR-119"
  - "ADR-213"
---

# ADR-208: 明示キーの固着ゼロの保証

## 背景: 固着の定義(所有者、2026-09-29)

固着 = **何度モードキーを押しても状態が変わらないこと**。トグル系で belief が古く、2回押しで直るのは許容する。モードずれが起きること自体は許容する(設計思想: awase は IME の状態を完全には追えない。ADR-191・ADR-212)。許容しないのは、明示キーを押しても解消されない状態が残ることである。

当初の草稿は、GJI × Blind 窓(Imm32Unavailable/TsfNative)で、絶対指定キー(Ctrl+変換=ON 等)が `gji_direct_already_matches(shadow_on, open)` の `applied` で `AlreadyMatched` として握りつぶされ続ける件(S-1)だけを扱っていた。2026-10-01 の Opus の網羅監査(根拠資料: [docs/tasks/adr208-liveness-audit-2026-10-01.md](../tasks/adr208-liveness-audit-2026-10-01.md))で、固着は次の4つあると分かった。

- **S-1(確定、既知)**: GJI × Blind × Engine 経由の絶対キー(Ctrl+変換/無変換・F13・単独タップ)× 古い `applied`。shadow 経路は PR #408(ADR-213 P2a)で解消済み。
- **S-2(確定、新規)**: `is_japanese_ime()==false`(実際は日本語 IME)の間、warrant(`state/open_warrant.rs:136`)が全書き込みを Unwarranted にする。Engine のコンボと 0x16/0x1A が全窓・全 IME で効かない(Win32 窓では漢字 0x19 も同じ根)。Engine は `NotJapaneseIme` で inactive になるので、「NICOLA が効かない」状態で Ctrl+変換を押しても救済されない。
- **S-3(条件付き)**: Win32 窓で belief が既にキーの意味と一致しているときの shadow no-op は、物理を Suppress して書かず、drift correction(観測が読めないと発火しない)頼み。
- **S-4(条件付き、推測)**: Win32 窓で `current_focus==None` かつ観測が食い違う向きのとき、Unwarranted と空書きを交互に繰り返す。

## 決定

1. **保証(仕様)**。対象押下(下記)ごとに次を保証する。
   - **INV-L1(配送の過不足なし)**: 明示キーの非リピート物理 KeyDown 1回につき、IME へ届く開閉の作用は**ちょうど1つ**。物理キーそのものが IME に届く(Allow、IME が自分で処理)か、awase が書く(Suppress/Consume して `VK_IME_ON/OFF`・`ImmSetOpenStatus`)かのどちらか一方。両方は二重 actuation(BUG-46/52/113)、どちらも無しは「二重の空振り」(固着の素)。awase が書く場合、**belief・`applied`・`is_japanese_ime` を理由に省略・棄却しない**。省略してよいのは「同じ押下で既に送った」場合だけ(BUG-113)。
   - **INV-L2(収束)**: 絶対指定キー(ON 専用/OFF 専用)は1押下で、トグルは2押下以内で、実 IME がキーの意味に一致する。トグルは belief が古いと1回目が逆向きになりうるが、各押下で INV-L1 が成り立ち、書いた後に belief が「書いた向き」になるので、2回目で一致する。
   - **対象押下**: 非注入(またはテスト目印つき)の非リピート KeyDown で、(a) shadow toggle を昇格させた/させうるキー(`shadow_action`/`sync_direction`)、または (b) Engine が `SetOpen(ExplicitUserAction)` を出した打鍵(コンボ・単独タップ・`keys.ime_*`)。トレイ等の UI 操作は別に扱う。
   - **全窓で必ず書く**(所有者決定): 窓プロファイル(Win32 ImmCross/TsfNative/Imm32Unavailable)・IME 種別(GJI/MS-IME)によらず適用する。
2. **設計**(新しい gate は足さず、既存の省略・授権を**緩める**)。
   - **D1**: Engine の明示 SetOpen でも、shadow toggle と同じく「押下の書き込みでは `applied` を省略の根拠にしない」(`executor.rs::dispatch_ime_set_open` で view の `shadow_on` を、`shadow_toggle_demotes_applied` を一般化した規則で未知にする。`applied` 自体は書き換えず、完了時の `record_ime_apply_result` が正しい値を書く。新しい ImeEvent は不要)。**BUG-113 の二重送信は押下 id で防ぐ**: hook が非リピート KeyDown に単調増加の `press_id` を振り、`ActuationOrder` に `press: Option<PressId>` を載せ、`ImeStateHub` が `last_written_press` を持ち、`decide_attempt` の前に「同じ press_id・同じ向きで既に書いた」なら `AlreadyMatched` で省く。drift correction 等の押下由来でない書き込みは `press=None` で従来どおり。省略の根拠は「`applied` が一致」ではなく「この押下で送った」。
   - **D2**: 対象押下の授権(`issue_open_warrant`)と shadow の昇格から `is_japanese_ime()` を外す(押下は明示操作)。0x19 の昇格条件の `is_japanese_ime()` を外す(US 配列の Alt+` でも出るため、ADR-093 の「IME の証拠」では扱わない)。英語 IME の窓で `VK_IME_ON` を送ることは無害(冪等、推測。CI の非日本語構成で1本確かめる)。
   - **D3**: `current_focus==None` でも、押下の order は「この押下の意図(`order.open()`)」で授権する(`WarrantBasis::ExplicitPress`)。押下は ADR-090 の warrant が防ぎたい「推測による書き込み」ではない。
   - **D4**: Win32 窓の shadow no-op で、**物理が Suppress される場合**(`plan` が Suppress を返す条件)は `kp_shadow_actuate(target)` を呼ぶ。Allow の場合は物理が届くので書かない(INV-L1)。これには `PhysicalKeyDisposition::plan` と shadow 判断の循環(`shadow_toggled` を入力に取る)を、`explicit_press_delivery` への一本化で解く。
   - **合流点の配線**(`.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」の表): `ime_controller.rs::apply`(同期)・`open_chain.rs::run_open_chain_async`/`imm_cross_write`/`fallback_write`(非同期)・`executor.rs::dispatch_ime_set_open`・`key_pipeline.rs::kp_shadow_actuate` のすべてで、押下 id の判定と warrant を共有する。drift correction は変更しない(`press=None`)。
3. **InputRelay**(所有者決定、2026-10-01): InputRelay の窓(PowerToys「境界線のないマウス」等、入力を別マシンへ中継するアプリ。IME の実体は向こう側で、awase は actuate を所有しない。ADR-119)では、開閉キーは **awase が握りつぶさず、そのまま素通し**にする(INV-L1 の「物理が届く」側)。「awase が開閉を決める(toggle する)キーは awase が actuate し、それ以外は素通し」という原則の、InputRelay への適用。現状は Engine のコンボが Consume され、届かず awase も書かない(二重の空振り)。実装は、`NotOwned` になった SetOpen の元の打鍵を Platform 側で reinject する形(Engine は profile を知らない、layer の規則)。実装の可否は L1 以降で検討する。
4. **例外の明示**(保証の対象外): (a) **MS-IME × 実 Chrome で awase の `VK_IME_OFF` が効かない**(BUG-172 の対照。2026-10-01 の CI `sc-driftrecovery-ctrlmuhenkan-msime-chrome` で、物理 Ctrl+無変換の OFF が 9/10 で実 IME を閉じなかった、drift correction でも戻らない)。機構の恒常的失敗なので前提から除き、別の書き込み機構(別キー・`ImmSetOpenStatus`・TSF compartment 等)の調査は別件にする。(b) Win キー押下中(`UnsafeToToggle`)、(c) Ctrl 救済の破棄。
5. **検証**。
   - (a) **Linux の網羅テスト**: 純粋な決定関数 `state/explicit_press.rs::explicit_press_delivery(state, key) -> Delivery{physical, write, reason}`(`PhysicalKeyDisposition::plan`・shadow の昇格・warrant・`decide_attempt` の合成。ungated)を切り出し、内部状態(belief・`applied`・`is_japanese_ime`・窓プロファイル・IME 種別・観測・`current_focus`・Engine active・press 履歴)の全組み合わせ(約83万通り、全列挙)× キー種別を流し、INV-L1・絶対キー1回・トグル2回・不動点なし・BUG-113(同一押下の二重送信なし)を固定する。まず**現状の反例を golden に固定して穴を見えるようにする**。
   - (b) **CI の drift × キー行列**(実打鍵): ずれの作り方(ハーネスが実 IME を直接 close/open、フォーカス変更、古い `applied`)× キー(Ctrl+無変換・Ctrl+変換・かな・半角全角・漢字・F13・単独タップ)× 窓(EDIT/RichEdit/tsf 窓・実 Chrome)× IME(GJI/MS-IME)で、押下後 +0.5/+2 秒の実打鍵が最大2回の押下で一致するかを見る。物理 Ctrl は `TEST_INJECTION_MARKER` 付き注入で物理扱いにできる(`hook.rs::is_test_injection`、debug ビルド+`AWASE_TEST_INJECTION=1`)。既存の `sc-driftrecovery-*`・`sc-kanji-*`・`sc-solotap-*` を拡張する。
6. **段階**(1段=1PR)。**L0〜L3 を v2 のブロッカーにする**(L3' は実機 A/B が条件で v2 のブロッカーにしない)。
   | 段 | 内容 |
   |---|---|
   | L0 | `explicit_press_delivery` の切り出し(挙動は変えない)と全列挙テスト。現状の反例(S-1〜S-4)を golden に固定 |
   | L1 | D2・D3(warrant、`is_japanese_ime` を明示押下の授権から外す)。S-2 の解消 |
   | L2 | D1 の押下 id(`press_id`、`ActuationOrder.press`、`last_written_press`)と `applied` の未知化。S-1 の解消、BUG-113 の両立 |
   | L3 | D4 と、Imm32Unavailable(Chrome)への適用。CI の drift × キー行列 |
   | L3' | TsfNative(WT×GJI)への拡張。**WT×GJI×PSReadLine の実機 A/B(押下ごとの `@` の発生率、develop と L3' 版、各 n≥30)がマージ条件**(単発 `VK_IME_OFF` は BUG-124 の「@」を誘発しうる)。`@` が出る場合の代替は ADR-206 の案「同じ OFF キーを2回続けて押したときだけ送る」(「2回で一致」に収まる) |
   | L4・L5 | InputRelay の素通し(決定3)、残る条件付き固着(S-3・S-4)の確認 |

### L0 実装メモ(2026-10-01、挙動不変)

- **切り出せた範囲**: 配送判断の核 `PhysicalKeyDisposition::plan` の本体を `state/physical_disposition.rs::plan_core`(ungated、`transport.rs` の `plan` は `ActiveImeKind` → `ImeKindId` の変換だけの殻)、shadow 昇格の intent 選択を `state/explicit_press.rs::select_shadow_intent`(`kp_stage_shadow_ime_toggle` が呼ぶ)、Engine の chord フィルタ条件を `engine_set_open_filtered_by_chord`(`handle_engine_set_open` が呼ぶ)。`issue_open_warrant`・`decide_gate`/`decide_chain`/`decide_attempt`・`shadow_toggle_demotes_applied`・`ShadowImeAction::resolve` は元から ungated で、そのまま合成した。
- **関数の形が仕様と違う点**: `explicit_press_delivery_with(state, key, judge)`。授権(`issue_open_warrant`)は `IntentStore`/`ObservationStore` を要するが、観測を入れる口(`AnyObservation::restored_from_journal`)を本番コードから呼ぶことは architecture_guard が禁じているため、`WarrantJudge` として差し込む(テストは合成ストアで本物の `issue_open_warrant` を呼ぶ)。L1 以降で本番が呼ぶときは live の `WarrantContext` から判定する実装を渡す。
- **循環は崩していない**: `kp_run_inner` の「shadow 昇格 → `plan`」の順を `explicit_press_delivery_with` が同じ順序で再現する。本番は本関数を呼ばない(D4 の一本化は L3)。
- **モデルの前提(推測を含む)**: 機構チェーンは先頭のみ、Engine の SetOpen は常に出る、完了後の `applied` は `record_ime_apply_result` の意味論、実 IME は Allow で届いた物理キーを意味どおり処理し awase の書き込みはその向きに設定する。P5(BUG-113)は「shadow 書き込みの後の Engine SetOpen は押下前の `applied` を見る」という現状モデルでの件数で、押下 id(L2)まで `#[ignore]`。
- **成果物**: `tests/explicit_press_exhaustive.rs`(全 34,560 状態 × 12 キー、デバッグビルドで約1秒)、`tests/golden/explicit_press_counterexamples.txt`(P1〜P5 の反例をクラス別の件数と代表例で固定。`UPDATE_GOLDEN=1` で再生成。L1〜L3 で件数が減る)。

## リスク

1. D1 の押下 id が、同一押下の shadow 書き込みと Engine の SetOpen の二重送信(BUG-113)を取りこぼす(sync キーが `keys.ime_on` でもある構成など)。全列挙テストと `sc-kanji-*`/`sc-solotap-*` で固定する。
2. TN×GJI で、OFF キーごとに単発 `VK_IME_OFF` が出て BUG-124 型の「@」(L3')。実機 A/B が条件。
3. D2 で、英語 IME の窓の Ctrl+変換が `VK_IME_ON` を送る(無害と推測。非日本語構成の CI で1本確認)。
4. 網羅テストの状態空間が大きく、純粋関数の切り出しが `PhysicalKeyDisposition::plan` の循環を崩す変更になる(L0 は挙動を変えない前提で、golden で差分ゼロを確認)。

## 代替案

- **belief ベースの「すでに一致」で省略し続ける(現状)**: BUG-113 を防ぐが、stale `applied` で固着する(S-1)。却下。
- **観測で検証してから書く**: 読めない窓(Blind)では保証できない。却下。
- **トグルキーも絶対指定にする**: キーの意味が変わる。却下。

## 非目的

- モードずれ自体を防ぐこと(許容する)。drift correction の削減・撤去(ADR-212 P6)。
- MS-IME × 実 Chrome の書き込み機構の再設計(例外、別件)。
