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
  一部実装(L0・L1・L2・L3a 実装済み。L3 の Chrome〈Imm32Unavailable〉適用は L1+L3a で成立済みと確認〈2026-10-02、コード変更なし〉。L3b の CI sc-keymatrix-* 14構成は expect=pass へ昇格済み〈efa9621a〉、MS-IME×実 Chrome の OFF は ENV_EXCEPTION=既知の制限〈BUG-185〉)。v2.0.0 に含まれる。未実装/未検証: L3'(TsfNative 等 × GJI の S-1 残 1,728 件、v2 ブロッカーにしない)、L4(InputRelay)、実機 A/B。
  旧(2026-10-04 更新前):
  採用(2026-10-01、Opus round1 反映済み)。**L0 実装済み**(挙動不変の切り出し・差分 0 の確認・全列挙テスト・反例 golden。「L0 実装メモ」節)、**L1 実装済み**(押下 ID・applied の未知化・S-1 解消・BUG-113 の両立。「L1 実装メモ」節。実機・CI の drift × キー行列は未実施)、**L2 実装済み**(押下の授権と非リピートの shadow 昇格が `is_japanese_ime`・`current_focus` を問わない。S-2・S-4 を解消。「L2 実装メモ」節。実機・CI の非日本語構成は未実施)、**L3a(D4)実装済み**(shadow no-op で物理が Suppress される窓は書く。S-3 を解消。「L3a 実装メモ」節。実機・CI は未実施)、L3 の残り(Chrome への適用・CI の drift × キー行列・E2 の対照)から未実装。所有者決定: 全窓で必ず書く(物理押下ごとに1回、同一押下の二重送信のみ防ぐ)、『解消』は最大2回の押下、検証は決定表の網羅テスト→CI の行列、InputRelay は『awase が actuate しない窓では開閉キーを握りつぶさず素通し』、MS-IME × 実 Chrome の `VK_IME_OFF` が効かない件(BUG-172 対照)は例外として明記し別機構は後で検討、L0〜L3 を v2 のブロッカーにする(L3' は実機 A/B が条件で v2 のブロッカーにしない)。
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

(Opus レビュー round1〈2026-10-01〉の B-1・B-2・M-1〜M-8 を反映済み。)

1. **保証(仕様)**。
   - **INV-L1(配送の過不足なし。awase の決定の不変条件)**: 対象押下ごとに、awase の決定 `Delivery` は、**(i) 物理キーを配送する(Allow/PassThrough)か、(ii) awase が書く(Suppress/Consume + write)かの、ちょうど一方**。`Delivery` の型は両方を持てないようにし、二重(Allow と write の両方)は決定時点で静的に禁止する(実行時に IME の処理を検出する必要は無い)。IME が物理キーをどう処理するかは awase から観測できないので、INV-L1 には含めず、**キー意味表の前提 A1**として分離する。
   - **前提 A1(配送したキーを IME がキーの意味どおりに処理する)**: 成り立つキー=0x16/0x1A・F2・0xF0・GJI 学習表で開閉トグルとされる 0xF3/0xF4。成り立たないもの=任意の sync キー・未学習の構成。後者は「Allow で配送してよいキー」から外し、Suppress + write 側に寄せる。
   - **INV-L2(収束)**: 絶対指定キー(ON 専用/OFF 専用)は1押下で、トグルは2押下以内で、実 IME がキーの意味に一致する。トグルは belief が古いと1回目が逆向きになりうるが、各押下で INV-L1 が成り立ち、書いた後に belief が「書いた向き」になるので、2回目で一致する。前提は A1 と、環境の例外(決定4)を除くこと。
   - **対象押下**: 非注入(またはテスト目印つき)の**非リピート** KeyDown で、(a) shadow toggle を昇格させた/させうるキー(`shadow_action`/`sync_direction`)、または (b) Engine が `SetOpen(ExplicitUserAction)` を出した打鍵(コンボ・単独タップ・`keys.ime_*`)。**自動リピートは対象外**(Engine の特殊キー照合はリピートの Down でも一致するので、`press=None` として従来の `applied` 省略に任せる。押し続けた間 `VK_IME_ON` を約30回/秒送ることを防ぐ)。トレイ等の UI 操作は別に扱う。
   - **全窓で必ず書く**(所有者決定): 窓プロファイル・IME 種別によらず適用する(ただし v2 時点の保証範囲は決定6の表)。
2. **設計**(新しい gate は足さず、既存の省略・授権を**緩める**)。
   - **D1(押下 id と `applied` の未知化)**:
     - hook が非リピート KeyDown に単調増加の `PressId`(OS 非依存の不透明な値として `awase::types` に置く)を振る。**Engine を通る経路で id を運ぶ**: `ImeEffect::SetOpen { open, press: Option<PressId> }` とし、Engine の FSM は単独タップ(ADR-206)の保留開始 KeyDown の id を保持して、KeyUp/タイムアウト確定時の effect まで運ぶ(コア crate `src/engine/` の型変更。L2 の範囲)。`ActuationOrder` に `press: Option<PressId>` を載せる。
     - **未知化は `press.is_some()` の order だけ**(shadow toggle と同じく、view の `shadow_on` を `shadow_toggle_demotes_applied` を一般化した規則で未知にする。`applied` 自体は書き換えない)。`press=None`(リピート・drift correction 等)は従来どおり `applied` の already-matched で省く。
     - **1押下につき書き込みは1回(向きを問わない)**。同じ押下で向きが違う2経路(shadow が ON を、Engine が OFF を要求。sync キーが `keys.ime_off` にも割り当てられている等)は、書く前に優先順位で解く(**Engine の明示コンボ > shadow**。Engine 側はユーザーが設定した `keys.*`)。衝突はログと journal に残す。`ImeStateHub` の `last_written_press` を、**order を発行した時点で予約**する(ImmCross の async は完了が WM 経由で後から届くため。完了時に書くと、同じ打鍵の Engine の sync SetOpen が二重に送る)。`UnsafeToToggle`/Failed で書けなかったときも予約は解かない(同じ押下内の再試行はしない。次の押下で直る=INV-L2)。reinject・drain replay・Ctrl 救済の50ms保留は、capture 時のスナップショット id を使うので同じ id になる。KeyUp は対象外。
     - drift correction との競合: 押下の書き込みの直後に予約済みの `TIMER_IME_REFRESH` が走って同じ向きを重ねる(BUG-113 型)のを防ぐため、`kp_shadow_actuate` にも refresh の kill(または「直近 N ms に押下の書き込みがあったら drift は送らない」条件。N は実測が要る)を置く。
   - **D2(`is_japanese_ime` を押下の授権から外す)**: 対象押下の授権(`issue_open_warrant`)から `is_japanese_ime()` を外す。0xF3/0xF4・F13〜F24 の shadow 昇格の条件(非リピート)も外す(根拠: IME 未同定なら役割由来の `shadow_action` は `None`〈`enrich_key_role` の latch〉で、昇格の前提は既にそこで守られている)。**0x19 は外さず、`is_japanese_ime` の代わりに「TIP 同定済み」(`table_ime_kind().is_some()`、GJI または CLSID で同定できた MS-IME 本体)を条件にする**(所有者決定、2026-10-01。当初案の根拠は 0x19 には誤りだった: 0x19 は GJI 以外では `kanji_shadow_action` が `KeepStatic` で hook の静的 Toggle を返すので、IME 未同定でも `shadow_action` が付く。条件を外すと英語 IME・IME の無い窓・US 配列の Alt+`〈ADR-093〉の 0x19 で belief 反転と書き込みが起きる)。同定済みなら `is_japanese_ime` の誤判定でも昇格するので S-2 は解消する。未同定かつ `is_japanese_ime=false` の 0x19 は昇格しない。静的 Toggle のままだと昇格しないのに `plan` が Suppress して IME にも awase にも届かない(二重の空振り、Opus レビュー PR #421 M-1)ので、`enrich_key_role` の `kanji_shadow_action` が受動(`None`)にして物理を素通しにする(`key_effect_runtime::kanji_passive_when_unidentified`)。`is_japanese_ime` を上げない理由〈ADR-093〉は別。副作用: 英語 IME・IME の無い窓で `VK_IME_ON` が SendInput される(無害は推測、CI の非日本語構成で1本確認)。リモート(mstsc 等)では SendInput した 0x16 がリモート側の IME を開く(今は Unwarranted で止まっている挙動が変わる)。`is_japanese_ime=false` のままだと Engine は `NotJapaneseIme` で inactive になり「IME は開いたが NICOLA が効かない」が残る。対処案(別 PR): TIP で同定済み(`active_ime_kind` が GJI/MS-IME)の窓で押下の書き込みが `wrote_open_state()` を返したら `is_japanese_ime` を true に上げる。
   - **D3**: `current_focus==None` でも、押下の order は「この押下の意図(`order.open()`)」で授権する(`WarrantBasis::ExplicitPress`)。
   - **D4**: Win32 窓の shadow no-op で、**物理が Suppress される場合**は `kp_shadow_actuate(target)` を呼ぶ。Allow の場合は書かない(INV-L1)。`plan` と shadow 判断の循環は、**`plan(shadow_toggled=false)` を先に評価する固定点**で解く(`plan` が `shadow_toggled` を見るのは非 IC の分岐だけで、IC は `is_kanji_event` なら常に Suppress)。1. `plan(false)` を評価、2. Suppress なら書いて `shadow_toggled=true`、3. 後段の `plan(true)` は IC では同じく Suppress、非 IC の no-op は 1 で Allow なので書かない。
   - **合流点の配線**(`.claude/rules/fix-requires-evidence.md` の表): `ime_controller.rs::apply`(同期)・`open_chain.rs::run_open_chain_async`/`imm_cross_write`/`fallback_write`(非同期)・`executor.rs::dispatch_ime_set_open`・`key_pipeline.rs::kp_shadow_actuate` のすべてで、押下 id の予約と warrant を共有する。
3. **InputRelay**(所有者決定): InputRelay の窓(入力を別マシンへ中継するアプリ。ADR-119)では、開閉キーを **awase が握りつぶさず素通し**にする(INV-L1 の (i) 側。「toggle するキーは awase が actuate し、他は素通し」の原則の InputRelay への適用)。**実装は「NotOwned になった SetOpen の元の打鍵を reinject」ではなく、Engine の前で PassThrough にする**: Platform が Engine に純粋な問い合わせ(既存の `engine.matches_ime_off(&ctx, &event)` と同型の `matches_ime_combo`)をして、InputRelay の窓では `on_input` の前に PassThrough を返す(`kp_run_inner` の Ctrl 救済の判定と同じ位置)。reinject 案は、Engine の `prev_activation`/tray が進む、`handle_engine_set_open` が所有しない窓の hwnd に `desired_open`・`last_intent`・IntentStore を書く、消費した KeyDown に対応する KeyUp の義務(`up_duty`)が Consume される非対称、という3つの問題がある。InputRelay の窓では awase 側の Engine ON/OFF(NICOLA)は更新しない(素通しの打鍵は awase の操作ではない)。
4. **例外の明示**(保証の対象外。**閉じた列挙**で、追加は ADR の改訂を要する): (a) **(IME, 窓, 向き)のセル**として: **MS-IME × 実 Chrome の `VK_IME_OFF`**(awase が `VK_IME_OFF` を書く全ての OFF 系=トグルの OFF・単独タップの無変換・D4 を含む。CI `sc-driftrecovery-ctrlmuhenkan-msime-chrome` で物理 Ctrl+無変換の OFF が 9/10 で実 IME を閉じず、drift correction でも戻らなかった)。この環境で OFF にできるのは物理が Allow される英数 0xF0 等だけ。候補(別件で調査): OFF キーを Suppress+write でなく物理 Allow にして MS-IME 自身に処理させる(MS-IME が物理の Ctrl+無変換を OFF に割り当てているか次第、推測)、`ImmSetOpenStatus`・TSF compartment。**【2026-10-04 訂正】** この例外を「環境の制約」としていた説明は誤り: 原因は MS-IME の挙動(未確定の composition が残っている間の `VK_IME_OFF` は IME を閉じず、変換モードだけが半角英数に変わる。入力中の文字が無ければ 10/10 閉じる)と判明した。awase なしの OS 注入でも同じ。`IMC_SETOPENSTATUS(0)` なら閉じるが TSF 窓では未確定の文字が消える(BUG-184 の逆)ため補完は採用せず、**今後も対応しない既知の制限**とする(Google 日本語入力の利用を案内)。詳細は [BUG-185](../known-bugs/BUG-185.md)・[調査メモ](../tasks/msime-chrome-off-rca-2026-10-04.md)。例外としての扱い(閉じた列挙)は変えない。(b) Win キー押下中(`UnsafeToToggle`)、(c) Ctrl 救済の破棄。**環境の例外と認める条件**(機構の恒常的失敗と内部状態による固着の区別): **E1(決定表)**: 全列挙テストで、そのセルの対象押下が `Delivery::Write` を出している(INV-L1 は成立)。**E2(対照実験)**: CI で、同じ窓・同じキーを**内部状態を新鮮にして**(awase を再起動直後、belief・`applied` が Unknown)押しても同じ率で失敗する。E1 と E2 の両方を満たすセルだけを例外と認め、どちらかが崩れたら固着(バグ)として扱う。`ctrlmuhenkan-msime-chrome` の 9/10 は E2 の対照がまだ無いので、対照の構成を1本足すことを認定条件にする。
5. **検証**。
   - (a) **Linux の網羅テスト**: 純粋な決定関数 `state/explicit_press.rs::explicit_press_delivery(state, key) -> Delivery{physical, write, reason}`(`PhysicalKeyDisposition::plan`〈`runtime/transport.rs`、windows 型に依存しない値で受ける形へ移設。`plan` は薄い殻〉・shadow の昇格・warrant・`decide_attempt` の合成。ungated)を切り出し、内部状態(belief・`applied`・`is_japanese_ime`・窓プロファイル・IME 種別・観測・`current_focus`・Engine active・`was_down`〈リピート〉・同じ押下で2経路が異なる向きを要求)の全列挙(約83万通り以上)× キー種別を流し、INV-L1(Delivery が一方だけ、配送側なら A1 の表のキー)・絶対キー1回・トグル2回・不動点なし・BUG-113・リピートで書かないことを固定する。**オラクルは実物の遷移関数**を使う(`record_ime_apply_result` の generation=None 分岐〈`platform_state.rs:1084-1109`〉を ungated な純粋関数に切り出し、`ImeModel` の reduce〈BUG-156 の降格、`completion_can_update_applied`〉を実際に通す。テストが遷移を手で模倣して自己言及にならないようにする)。**L0 の『挙動を変えない』は、旧 `plan` + 旧 shadow 判断と新 `explicit_press_delivery` を同じ全列挙空間で比べて差分 0 を示すことで機械的に確かめる**。**現状の反例 golden** は、行の列挙(S-2 だけで状態空間の約半分が反例)ではなく、**反例の分類(S-1〜S-4・未知)× 件数 + 各分類の最小の代表例**にする。各段で該当分類の件数が 0 になることを確かめ、分類に当てはまらない反例が出たらテストを失敗させる(未知の固着の検出)。
   - (b) **CI の drift × キー行列**(実打鍵): ずれの作り方(ハーネスが実 IME を直接 close/open)× キー × 窓(EDIT/RichEdit/tsf 窓・実 Chrome)× IME(GJI/MS-IME)で、押下後 +0.5/+2 秒の実打鍵が最大2回の押下で一致するかを見る。物理 Ctrl は `TEST_INJECTION_MARKER` 付き注入で物理扱いにできる(debug ビルド+`AWASE_TEST_INJECTION=1`)。**CI で安定して作れるのは S-1 だけ**(awase に書かせた後、ハーネスが外から反転する)。S-2(`is_japanese_ime=false`)・S-3(観測が読めない/嘘)・S-4(`current_focus==None`)は CI では作れず、**Linux の全列挙テストだけが保証の根拠**になる。
6. **段階と v2 の保証範囲**(1段=1PR)。**L0〜L3 を v2 のブロッカーにする**(L3' は実機 A/B が条件で v2 のブロッカーにしない)。**v2 時点の保証範囲**: Chrome(Imm32Unavailable)と全窓の S-2 まで。**TsfNative×GJI の S-1(L3')、InputRelay の素通し(L4)、S-3・S-4(L5)は v2 時点で既知の制限として残る**(チェックリストの既知の制限にも同じ範囲を書く)。
   | 段 | 内容 |
   |---|---|
   | L0 | `explicit_press_delivery` と `record_ime_apply_result` の純粋部の切り出し(挙動は変えない。新旧の判定を全列挙で比べて差分 0)と全列挙テスト。現状の反例(S-1〜S-4)を、分類 × 件数 + 代表例で golden に固定 |
   | L1 | **押下 id**(`PressId`、`ImeEffect::SetOpen.press`〈コア crate の型変更、単独タップの id 運搬〉、`ActuationOrder.press`、`last_written_press` の発行時予約、衝突の優先順位)と `applied` の未知化(D1)。S-1 の解消、BUG-113 の両立。**D2 で書き込みが増える前に、同一押下の二重送信の防御を入れる** |
   | L2 | D2・D3(warrant、`is_japanese_ime` を押下の授権から外す)。S-2 の解消 |
   | L3 | D4(`plan(false)` 先行の固定点)と、Imm32Unavailable(Chrome)への適用。CI の drift × キー行列(S-1)と E2 の対照構成 |
   | L3' | TsfNative(WT×GJI)への拡張。**WT×GJI×PSReadLine の実機 A/B(押下ごとの `@` の発生率、develop と L3' 版、各 n≥30。BUG-175 の知見により、修飾キー〈Ctrl+Shift 等〉押下中の注入でも `@` が出ないかを含める)がマージ条件**(単発 `VK_IME_OFF` は BUG-124 の「@」を誘発しうる)。`@` が出る場合の代替は ADR-206 の案「同じ OFF キーを2回続けて押したときだけ送る」(「2回で一致」に収まる) |
   | L4 | InputRelay の Engine 前 PassThrough(決定3) |
   | L5 | 残る条件付き固着(S-3・S-4)の確認 |
7. **v2 ブロッカーの受け入れ条件**(L0〜L3 の完了条件): (1) 全列挙テスト: v2 範囲のセルで反例 0(分類外の反例も 0)。範囲外のセルの反例は、分類ごとの件数を golden に固定する(増えたら失敗)。(2) CI: S-1 の再現構成(Imm32Unavailable × GJI/MS-IME × Ctrl+変換/Ctrl+無変換/単独タップ × 外からの反転)が、各 n≥10 で、絶対キーは1押下、トグルは2押下で一致する。MS-IME×Chrome の OFF は例外の対照(E2)付き。(3) BUG-113: 既存の `@` 検出(`check_typing_stress.py`)の件数が、develop と同じ土台で増えていない。(4) 既存の `sc-*` の期待表が develop と同一。

### L0 実装メモ(2026-10-01、挙動不変。Opus round1 反映後)

- **切り出せた範囲**: 配送判断の核 `PhysicalKeyDisposition::plan` の本体を `state/physical_disposition.rs::plan_core`(ungated、`transport.rs` の `plan` は `ActiveImeKind` → `ImeKindId` の変換だけの殻)、shadow 昇格の選択を `state/explicit_press.rs::select_shadow_intent`、Engine の chord フィルタ条件を `engine_set_open_filtered_by_chord`、`record_ime_apply_result` の generation=None 分岐の純粋部を `ime_model::apply_result_effective_open` と `ImeModel::confirm_applied`(`ImeStateHub::record_confirmed` はこれを呼ぶだけ)。`issue_open_warrant`・`decide_*`・`shadow_toggle_demotes_applied`・`ShadowImeAction::resolve` は元から ungated でそのまま合成した。
- **`Delivery` の型**: 現状の違反(二重・空振り)も記録するため `Delivery{physical, write, reason, ..}` は両方を持てる形のままにし、INV-L1 を満たす決定だけの型 `Resolution { PassThrough | Write{physical, open} }` へ `Delivery::resolve()` で写す。写せないものが `Violation::{Both, Neither}`(反例)。前提 A1 は `ExplicitKey::a1_holds()`(0x16/0x1A・0xF0/F2・学習済み 0xF3/0xF4。InputRelay は中継先が処理するので別扱い)。
- **仕様との差**: 関数は `explicit_press_delivery_with(state, key, judge, mode)`。授権(`issue_open_warrant`)は `IntentStore`/`ObservationStore` を要するが、観測を入れる口(`AnyObservation::restored_from_journal`)を本番から呼ぶことを architecture_guard が禁じているため `WarrantJudge` として差し込む(テストは合成ストアで本物の `issue_open_warrant` を呼ぶ)。
- **循環は固定点で表現**: `plan(false)` を先に評価 → 書く決定 → 後段 `plan(write_wanted)` の手順を `DeliveryMode::{Legacy, FixedPoint}` で表す。Legacy は現状(belief が倒れたときだけ書く)、FixedPoint は D4(昇格した no-op で plan(false) が Suppress なら書く)。本番は本関数をまだ呼ばない。
- **挙動不変の確認**: 旧 `plan` + 旧 shadow 判断の合成(テスト内の `legacy_reference`)と新関数(Legacy)を、全列挙空間(状態 69,120 × キー 12 × 3 種の判定器 = 約 249 万通り)で比べて差分 0(`legacy_mode_matches_the_pre_extraction_composition_everywhere`)。Legacy と FixedPoint の差は「Suppress される no-op が書く」だけであることも固定した。Windows 側の `plan_tests` は Linux では走らないので windows-build CI が担う。
- **オラクル**: 書いた後の `applied` は手で模倣せず、shadow 経路は `apply_result_effective_open` + `ImeModel::confirm_applied`、Engine 経路は `ImeModel::reduce`(`ImeApplyRequested` → `ImeEvent::from_apply_outcome`、`completion_can_update_applied` を含む)を通す。BUG-156 の降格(予測の不一致)は押下の遷移に関わらないので含めない。
- **状態空間**: Opus round1 の指摘どおり `was_down`(リピートは対象外、P6)と、同一押下で shadow と Engine が異なる向きを要求する構成(P5)を追加した。
- **モデルの前提(推測を含む)**: 機構チェーンは先頭のみ、Engine の SetOpen は常に出る、実 IME は A1 のキーの配送を意味どおり処理し awase の書き込みはその向きに設定する、P5 は「Engine の executor は押下前の `applied` を見る」。押下 id(`PressId` 等)は L1 の範囲で、P5 の `#[ignore]` テストは現状のモデルとして置いた。
- **成果物**: `tests/explicit_press_exhaustive.rs`(69,120 状態 × 12 キー、debug ビルドで約 3 秒)、`tests/golden/explicit_press_counterexamples.txt`(P1〜P6 の反例を分類 × 件数 + 各分類の最小の代表例で固定。分類外の反例があるとテストが失敗する。`UPDATE_GOLDEN=1` で再生成)。

### L1 実装メモ(2026-10-01)

- **押下 ID の付与**: コア crate の `awase::types::PressId`(単調増加 `u64` の newtype)と `is_press_start(is_keydown, injected, was_down)`(非注入の非リピート KeyDown)。`RawKeyEvent::press_id: Option<PressId>`、hook(`hook.rs::assign_press_id`)が振る。KeyUp・自動リピート・注入は `None`。reinject・drain replay・Ctrl 救済の 50ms 保留は同じ `RawKeyEvent` を再処理するので同じ id。
- **Engine の運搬**(コア crate の型変更): `ImeEffect::SetOpen { open, press }`。コンボ・`keys.ime_*`・役割由来の開閉は `Engine::check_special_keys` がその打鍵の `press_id` を載せる(`Decision::stamp_set_open_press`)。単独タップ(ADR-206)は確定点(KeyUp/タイムアウト/次のキー)が KeyDown と別イベントなので、`ClassifiedEvent::press_id` → `PendingThumbData::press_id` が保留開始 KeyDown の id を保持し、`NicolaFsm::ime_open_requested`(`ImeOpenRequest { action, press }`)で確定時の effect まで運ぶ。自動リピートは `press=None`(特殊キー照合はリピートでも一致するが従来の `applied` 省略に任せる)。
- **予約と衝突の優先順位**: `state/press_ledger.rs::PressLedger::claim`(純粋)を `ImeStateHub::claim_press_write` が order の**発行直前**に呼ぶ(`last_written_press`。ImmCross の async は完了が後から届くので完了時では二重送信を防げない)。同じ押下で同じ向き=`Duplicate`(書かない)。衝突は info ログと `JournalEntry::PressWriteClaim`。
  - **衝突は書く前に静的に解く**(PR #419 Opus M-4): 衝突は設定の重なりからしか起きない(ある VK が shadow の昇格キーで、かつ Engine の `keys.ime_*` にも一致する)。`keys.ime_detect`(`sync_direction`)と重なるキーは `match_special_keys` が元から一致させない(二重処理の防止)ので、残るのは `shadow_action` 由来(0x16/0x1A・0x19・0xF3/F4 等)と `keys.ime_*` の重なりだけ。`kp_run_inner` が shadow の判断の**前**に `Engine::matches_ime_set_open(&ctx, &event)`(副作用なしの純粋な問い合わせ。`ctx` は shadow 前の belief で組むのでトグル型の向き `!ctx.ime_on` も解ける)を呼び、Engine が同じ打鍵の `SetOpen` を出すキーでは `kp_stage_shadow_ime_toggle` が昇格・belief・書き込みのいずれも行わず Engine に任せる(物理は Engine の Consume が握る)。単独タップ(確定が KeyUp)は、保留に入る無変換/変換が `sync_direction`・`shadow_action` を持たない(ADR-206: shadow を通らない)ので shadow との重なり自体が無く、保留開始 KeyDown の時点での判断は要らない。これで 1 押下の書き込みは 1 回(向きを問わない)になる。
  - `PressLedger` の衝突分岐(`ConflictEngineWins`〈Engine が逆向きなら上書き〉/`ConflictKept`)は、事前問い合わせが効かなかったときの**ほぼ到達しない防御線**として残す(全列挙の `P5-Ledger`。上書きは書き込み 2 回で、ImmCross が先頭の窓では両方 async なので勝ち負けは保証されない)。
  - **予約の解放**(M-2): **同期**の書き込み(先頭機構が ImmCross でない窓)が何も送らなかったとき(`outcome_sent_nothing`: UnsafeToToggle/NotOwned/Unwarranted)は `release_press_write` で予約を解き、同じ押下の次の経路が改めて書ける(「絶対指定は 1 回」)。`Failed`(一部送った可能性)・`AlreadyMatched` は解かない。**非同期**(ImmCross 先頭)は完了が後から届くので解けない。例外(決定4に準ずる): **ImmCross 先頭の窓で同一押下の 1 回目の書き込みが失敗した場合(フォーカス世代の変化・未授権を含む)は、同じ押下の後続の経路は省かれ、次の押下で直る**(INV-L2)。
  - **重複の完了**(M-1): 書かなかった重複の `SetOpen` は完了へ `AlreadyMatched` ではなく「送っていない」outcome(`press_ledger::DUPLICATE_OUTCOME` = UnsafeToToggle)を流す。`AlreadyMatched` だと書いていない押下が `applied=Confirmed` になり(先行の書き込みが失敗した場合に嘘の確認)、TsfNative では S-1 を L1 自身が作る。全列挙の `duplicate_completion_never_confirms_applied` が実物の `ImeModel` の遷移で固定する。
- **D1(applied の未知化)**: `ime_actuation_decision::explicit_press_shadow_on`/`explicit_press_applied_pair`(`shadow_toggle_demotes_applied` を一般化)。`ActuationOrder::with_press` が載せる `press.is_some()` の order だけ、`executor::dispatch_ime_set_open`(Engine)と `kp_shadow_actuate`(shadow)が view の `shadow_on` を未知にする。**Engine 経路は実質 TsfNative の窓だけ段階制御して未知化しない**(`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE = false`、`engine_press_unknowns_applied(profile.is_effectively_tsf_native(class_name))`。Windows Terminal の `CASCADIA_HOSTING_WINDOW_CLASS` は `Imm32Unavailable` に分類されるが実質 TSF なので、プロファイル値だけでなくクラス名も見る。M-3): リスク2・決定6 の L3'(TN×GJI の単発 `VK_IME_OFF` が BUG-124 型の「@」を誘発しうる)は WT×GJI×PSReadLine の実機 A/B(各 n≥30)がマージ条件で、v2 時点では既知の制限として残るため。L1 で TsfNative の Engine 経路まで未知化すると、Ctrl+無変換を IME OFF のまま押すたびに単発 `VK_IME_OFF` が出る。A/B の後でこの定数を `true` にすれば解禁される(モデル・golden・テストが追随する)。shadow 経路の降格(PR #408 から既にある)と押下の予約は TsfNative でも変えない。`applied` 自体は書き換えず、完了時の `record_ime_apply_result` が正しい値を書く(新しい `ImeEvent` は不要)。`press=None` は従来の already-matched 省略。shadow 経路は従来、リピートでも無条件に降格していたが、L1 でリピートは `applied` の省略に戻した。
- **kp_shadow_actuate の refresh kill**: P2c(ActivationSync 撤去)で消えた打鍵前の `TIMER_IME_REFRESH` の kill を戻した(kill を優先。「直近 N ms の押下の書き込みでは drift を送らない」条件は tuning 定数と実測が要るので採らない)。
- **ImmCross の書き込み時間切れ(項目6)は見送り**: `SendMessageTimeoutW(150ms)` の時間切れは取り消されず後で届くが、(1) 後続の機構(GjiDirect/MsImeDirect)は絶対指定の冪等な書き込みで、同じ向きに重なるだけ(逆向きの二重の「開閉」にならない)、(2) 止めると IME 窓が応答しない窓で VK へ落ちる収束経路(INV-L2)を失う、(3) 次の押下との順序逆転はフォールスルーの有無に関わらず起きる。時間切れを `ImmCrossOutcome::open_timed_out` として診断ログ(`[apply-ime] ImmCross failed (... timed_out=..)`)に出すだけにした。頻度を測って追い送りが害と分かったら `press.is_some()` の order だけ UnsafeToToggle(未確定)で止める案を再検討する。
- **L0 モデルの更新**: `DeliveryMode::PressId`(L1、**現在の本番**)と `PressIdFixedPoint`(L1+D4、L3 の本番)を足し、`explicit_press_delivery_after`(同一押下で先に予約された向き `claimed` を渡す)と `dual_route_writes_with`、`ElisionReason::AlreadyWrittenThisPress` を追加。モデルは本番と同じ純粋関数(`PressLedger`、`explicit_press_shadow_on`)を呼ぶ。全列挙(80,640 状態〈プロファイルに「Imm32Unavailable だが TSF 扱いのクラス」を加えた〉× 12 キー)で **S-1 は L1 前の 1,008 件(うち「起こりうる」396)から 672 件(同 264)へ減り、残りは実質 TsfNative×GJI の Engine 経路だけ**(L3' まで段階制御)。それを除けば 0 件。P5(事前問い合わせ後の書き込みは Engine の 1 回)と P5-Ledger(防御線の同じ向きの二重送信)は違反 0、P6 の `repeat_rewrites_gji_direct` は 0。`l1_changes_only_the_press_engine_already_matched_elision` が、L1 が L0 から変えるのは非リピートの Engine 経路(TsfNative を除く)の already-matched 省略だけであることを全列挙で固定。`s1_already_matched_is_resolved_by_l1_except_tsf_native` が、残る S-1 が実質 TsfNative×GJI だけであることを固定。モデルの合成順は本番と同じ(shadow は claim が gate〈`ImeController::apply`〉より前、Engine は gate〈executor 冒頭〉の後に claim)。S-2〜S-4・L5・L9 は L2 以降(件数は golden)。
- **本番との一致**: 押下の予約・applied の未知化の判断は、本番(`ImeStateHub::claim_press_write`・`dispatch_ime_set_open`・`kp_shadow_actuate`)とモデルが同じ純粋関数を呼ぶ(判断の二重実装なし)。ID の付与は `was_down`(フックが観測する物理キー状態)に依存し、KeyUp を取り逃すと次の物理押下が「リピート」と見なされて id が付かない(S-1 が戻る。`reset_physical_key_state` で救済される経路はある)。入口の配線は `tests/architecture_guard.rs::press_id_is_claimed_and_carried_at_every_order_issuing_entry` が固定(order を発行する 2 入口が予約・`with_press`・未知化・kill を持ち、drift correction〈`ime_refresh.rs`〉は press を持たない)。実機・CI の drift × キー行列は未実施。

### L2 実装メモ(2026-10-01)

- **授権(D2・D3)**: `open_warrant.rs` に `issue_press_warrant`(押下用)と `WarrantBasis::ExplicitPress` を足した。Step 0(真の安全弁)は押下でも先に効き、それ以外は `is_japanese_ime`・`current_focus`(IntentStore の Step 1)・観測の食い違いを見ず、押下の意図(`requested`)で授権する。`ActuationOrder::issue` は通常の授権(`issue_open_warrant`)と押下の授権の両方を発行時に評価し、`with_press(Some)` が載ったら押下の授権へ差し替える(押下 ID は order の発行後に載る〈L1〉ため。`issue` の構築経路〈INV-47〉と `.with_press(press)` の配線は不変)。`press=None`(リピート・drift correction 等)は従来どおり。合流点は order の `into_actuation`(同期 `ImeController::apply`、非同期 `run_open_chain_async`)が同じ warrant を共有するので、入口ごとの再判定は無い(`is_japanese_ime`/`current_focus` を見る別の gate は無いことを grep で確認)。`reassert_explicit_physical_key`・`force_on_and_correct_romaji`・drift correction は押下 ID を持たない(`press=None`)ので授権は従来どおり。
- **shadow 昇格(D2)**: `select_shadow_intent` の `shadow_action` 昇格は、非リピートの押下なら 0xF3/0xF4・F13〜F24 は `is_japanese_ime` を問わない。**0x19 は `is_japanese_ime` の代わりに TIP 同定済み(`ime_identified` = `tsf_obs().table_ime_kind().is_some()`)を条件に残す**(所有者決定。D2 当初案の根拠『IME 未同定なら `shadow_action` が `None`』は 0x19 に当たらず〈GJI 以外は静的 Toggle のまま〉、外すと英語 IME・IME 無しの窓・US 配列の Alt+` に副作用が出るため)。**未同定かつ `is_japanese_ime=false` の 0x19 は `kanji_shadow_action` が受動(`None`)にする**(M-1。`plan` が Down/Up とも Allow=素通し。英語 IME・IME 無しの窓・US 配列の Alt+` を飲み込まない。`is_japanese_ime` が真の未同定 IME〈ATOK 等〉・同定済みは従来どおり)。リピート(`was_down`)は従来どおり `is_japanese_ime` が真のときだけ(押下 ID を持たない order は授権が `is_japanese_ime` を問うので、昇格して belief だけ反転し書かれない食い違いを作らない)。
- **全列挙モデル**: `DeliveryMode::PressIdBeforeL2`(L1 まで、参考)を足し、`PressId`(本番)は L2 の授権(`WarrantRequest.explicit_press`)と昇格を適用する。`StoreJudge` は `explicit_press` なら本物の `issue_press_warrant`、でなければ `issue_open_warrant` を呼ぶ。`l2_changes_only_the_press_warrant_and_the_shadow_promotion` が、L2 が L1 から変えるのは非リピートの Unwarranted/NotPromoted の押下だけで、リピートは同一、押下の order は Unwarranted にならないことを全列挙で固定。`s2_and_s4_are_resolved_by_l2` が S-2・S-4 が 0 件であることを固定。`is_japanese_ime=false` の no-op を S-2 と誤分類していたのを A1_noop(別の穴)へ直した。
- **件数(全列挙 120,960 状態〈kind を Gji/MsIme 同定済み/MsIme 未同定の 3 通りにした〉× 12 キー、P1 の反例)**: L2 前 165,732 件 → L2 後 49,248 件。S-2(86,400)・S-4(29,430)・L-9(1,710)は **0 件**、P3(トグルの 2 押下収束)の違反も 0。未同定かつ `is_japanese_ime=false` の 0x19 は IME の開閉キーではなく、受動で素通しなので P1〜P4 の対象外(`kanji_is_promoted_when_identified_and_passed_through_when_unidentified` が昇格条件〈`is_japanese_ime || ime_identified`〉と、未同定時の物理 Allow・書き込み無しを全状態で固定)。残る反例: S-1 1,728(TsfNative/Gji 864 + 実質 TSF の ImmUnavailableTsfClass/Gji 864 のみ。L3' 待ち。golden の `s1_profile/kind` で固定。L2 前の 672 より増えたのは、S-2 で先に Unwarranted になっていた `is_japanese_ime=false` の状態が S-1 へ移ったため)、S-3 25,920(L3 の D4)、L5 InputRelay 8,640(L4)、A1 外の no-op 12,960(うち 4,320 は旧分類が S-2 と誤っていた分)。
- **見送り・残課題**: (1) `is_japanese_ime` を上げる対処(D2 の「TIP 同定済みの窓で押下の書き込みが `wrote_open_state()` を返したら true に上げる」)は別 PR。これが無いと `is_japanese_ime=false` の窓では Engine が `NotJapaneseIme` のままで「IME は開いたが NICOLA が効かない」。設計の注意点: (a) 上げる条件は「物理の押下(`press.is_some()`)× ON 方向 × TIP 同定済み(`table_ime_kind()` が `Some`)× 書き込み成功」に限る(OFF の押下では上げない)。(b) 「書き込み成功」は IME が開いた証拠ではない(BUG-141: `outcome:Applied` だけでは成功と判断できない)ので、同定済みを必須にする。(c) `is_japanese_ime` の消費者(約 10 箇所)の一覧を確認する(上げると drift correction 等、`press=None` の授権が通るようになる)。(d) probe による下げ(`apply_focus_probe` の grace、`platform_state.rs` の `is_japanese_ime = is_jp`)と振動しないようにする(同じフォーカスの間は probe で下げない等の優先)。(e) belief の規律(`.claude/rules/ime-belief-architecture.md`)と `architecture_guard` の `set_is_japanese_ime` 呼び出し元の件数を確認する。(2) 非日本語構成の CI と実機 A/B は未実施。(3) リモート(mstsc 等)で SendInput した 0x16 がリモート側の IME を開く挙動変化(今までは Unwarranted で止まっていた)は未確認(PR 本文に記載)。(4) 押下でも Step 0(`PanicReset`/`ProfilePolicy`)が ON を要求している間は OFF を授権しない。特に chord フィルタ中(`handle_engine_set_open` が `on_set_open_requested` の前に return するため PanicReset が解除されない)は、トレイの「状態をリセット」の直後に Ctrl+無変換の OFF が通らない(Ctrl を離せば直る。固着ではない、全列挙のモデルに PanicReset の次元は無い)。(5) `table_ime_kind()` は窓単位でなくシステム単位の同定なので、日本語 IME の窓から英語 IME の窓へ移った直後に同定が古い値のまま `Some` で残る区間がある(推測)。その間の英語窓の 0x19 は昇格する(書き込みは無害と推測)。起動直後はポーリング確定まで `None`(CI のログで最大約 3.6s)で、その間の 0x19 は `is_japanese_ime` が真なら昇格、偽なら受動。

### L3a 実装メモ(2026-10-01)

- **本番の変更(挙動が変わる箇所は1つ)**: `kp_stage_shadow_ime_toggle` の no-op 分岐(belief が既に押下の向きと一致)の終端が `return false` から `kp_shadow_noop_write` になった。`plan(shadow_toggled=false)` を先に評価(固定点の手順1)し、Suppress なら `kp_shadow_actuate(key_target, press)` を呼んで `true` を返す(手順2。後段の本物の `plan(true)` は IC では同じく Suppress)。Allow(IU/TN の 0x16/0x1A、sync キー、F13 等で物理が IME に届く窓)・リピート(`press_id=None`)・TsfNative は書かない。判断は `state/explicit_press.rs::shadow_noop_write_target`(ungated の純粋関数。**書く向きはキーの意味 `new_val`**で belief ではない。PanicReset ガード等で belief が動かず `current != new_val` のまま no-op に入っても逆向きを書かない〈Opus M-1〉。授権が通らなければ gate が止める)で、全列挙モデル(`DeliveryMode::PressIdFixedPoint`)と共有する。
- **L1 の規律**: 書き込みは `kp_shadow_actuate` の `claim_press_write`(order 発行直前)を通り、view の `shadow_on` は `explicit_press_applied_pair` で未知にする(belief==target でも実 IME が違えば書く、already-matched で省かれない)。同じ押下で Engine が SetOpen を出すキーは、呼び出し前の `engine_owns_open_key` で既に除外される(重複を作らない)。
- **TsfNative の段階制御**: `SHADOW_NOOP_WRITES_IN_TSF_NATIVE=false` は**現状どの状態でも効かない防御**。`plan(false)` が Suppress になる no-op は `Standard`(ImmCross)だけで、TSF 扱いのクラスは `Standard` にならず、TsfNative の no-op は物理が Allow で届く(Suppress されるのはトグルキーで、トグルは必ず belief を倒すため no-op にならない)。したがって定数を `true` にするだけでは TsfNative の no-op 書き込みは起きず、L3' で TsfNative にこれを広げるなら `plan` の Suppress 条件(TsfNative の no-op の Down を Suppress するか)を含む設計と BUG-124 の実機 A/B が要る。
- **BUG-113**: `plan(false)` が Allow なら書かない(物理が IME に届くので二重にならない)。同一押下の二重送信は予約が防ぐ(P5 は 0 件のまま)。
- **全列挙モデル・golden**: 本番の判断は `PressIdFixedPoint`(`PROD`)。`l3a_changes_only_the_suppressed_shadow_noop_write` が、L3a が L2(`PressId`)から変えるのは『Suppress される非リピートの no-op が書く』ことだけ(物理の配送は不変、Allow・リピート・Engine 経路は同一)を全列挙で固定する。`P1-FixedPoint` は `P1-PreL3a`(L2 まで)に置き換えた。
- **件数(120,960 状態 × 12 キー、P1 の反例)**: L3a 前 49,248 → L3a 後 23,328。S-3 は 25,920 → **0**(P2 の収束違反 49,248 → 23,328、P4 の繰り返し違反 92,160 → 40,320)。残る反例: S-1 1,728(TsfNative/Gji と実質 TSF の ImmUnavailableTsfClass/Gji、L3' 待ち)、L5 InputRelay 8,640(L4)、A1 外の no-op 12,960(Allow で配送するが IME が処理する保証が無いキー〈任意の sync キー等〉の no-op。モデルが『Allow なら IME が処理』としない部分で、D4 の範囲外)。
- **見送り・残課題**: F13〜F24 の役割キーは no-op で `plan(false)` が Allow(`shadow_toggled` を見る規則)なので書かない(A1 外の no-op に含まれる)。実機・CI の drift × キー行列(ハング窓・IME の偽報告で no-op 書き込みが実際に救済することの確認)は未実施。

### L3 の Chrome(Imm32Unavailable)適用の確認(2026-10-02、コード変更なし)

L3 の「Imm32Unavailable(Chrome)への適用」は、新しい実装なしで **L1+L3a により成立している**ことを確認した。

- **全列挙(`explicit_press_exhaustive`、18件 pass)**: `ImmUnavailable`(実 Chrome 相当)の S-1・S-3 は 0 件。S-1 の残りは `ImmUnavailableTsfClass`/`TsfNative` × GJI の 1,728 件(864 + 864、L3' 待ち)だけで、`ImmUnavailable` の反例は `A1_noop_pass_through`(A1 外の任意の sync キーの no-op を Allow で配送する。D4 の範囲外=二重配送を避けるため書かない)だけ。
- **理由**: `kp_stage_shadow_ime_toggle` の D4 は `plan(false)` が Suppress のときだけ書く。Chrome の絶対キー(0x16/0x1A、Ctrl+変換/無変換、単独タップ)は Engine 経路で、L1 の押下 ID と `applied` の未知化により already-matched で省かれず書かれる。物理 Allow のキーは INV-L1 により書かない。
- **CI(L3b、run 36951828252・36957521623)**: 実 Chrome × GJI は絶対キー1押下・トグル2押下以内で全セル収束。MS-IME × 実 Chrome は ON 方向が収束し、OFF 方向(絶対・トグルとも)は E2 の対照(fresh)も同率で失敗し ENV_EXCEPTION(決定4(a))。
- したがって L3 の残りは、`sc-keymatrix-*` を `expect=pass` へ昇格する判断と、受け入れ条件(決定7(3)(4)の `@` 件数・`sc-*` 期待表)の最終確認だけで、Chrome 向けの本番コード変更は要らない。

## リスク

1. D1 の押下 id が、同一押下の shadow 書き込みと Engine の SetOpen の二重送信(BUG-113)を取りこぼす(sync キーが `keys.ime_on` でもある構成など)。全列挙テストと `sc-kanji-*`/`sc-solotap-*` で固定する。
2. TN×GJI で、OFF キーごとに単発 `VK_IME_OFF` が出て BUG-124 型の「@」(L3')。実機 A/B が条件。
3. D2 で、英語 IME の窓の Ctrl+変換が `VK_IME_ON` を送る(無害と推測。非日本語構成の CI で1本確認)。L2 で実装済み(挙動変化あり、実機・CI 未確認)。0x19 は GJI 以外では静的 Toggle のまま IME 未同定でも `shadow_action` が付くため、TIP 同定済みのときだけ昇格させ、未同定かつ `is_japanese_ime=false` では受動にして素通しにする(英語 IME・US 配列の Alt+` を飲み込まない。所有者決定、「L2 実装メモ」参照)。0x16/0x1A・Engine のコンボは非日本語 IME でも `VK_IME_ON/OFF` を書く。
4. 網羅テストの状態空間が大きく、純粋関数の切り出しで挙動が変わるリスク(L0 は新旧の判定を全列挙で比べて差分 0 を示す。`plan` の循環は固定点で解けるので崩す変更ではない)。
5. 押下 id をコア crate の `ImeEffect::SetOpen` で運ぶ型変更(単独タップの確定点が KeyDown でないため)。

## 代替案

- **belief ベースの「すでに一致」で省略し続ける(現状)**: BUG-113 を防ぐが、stale `applied` で固着する(S-1)。却下。
- **観測で検証してから書く**: 読めない窓(Blind)では保証できない。却下。
- **トグルキーも絶対指定にする**: キーの意味が変わる。却下。

## 非目的

- モードずれ自体を防ぐこと(許容する)。drift correction の削減・撤去(ADR-212 P6)。
- MS-IME × 実 Chrome の書き込み機構の再設計(例外、別件)。

## 追記(2026-10-06): 衝突の info ログの撤去

本文(決定2 D1 など)の「衝突は info ログにも残す」は、ログと journal の重複の整理(第 1 段階、所有者承認 2026-10-06)で撤去した。`[press-ledger]` の手書きのログ 3 行は無くなり、衝突は journal の `PressWriteClaim`(verdict に出る)と、その派生の debug 行 `press write claim`(`awase::journal`)にだけ残る。既定(info)の awase.log には衝突の情報は出ない。報告の添付(`attach_log`)ありなら journal に残る。

