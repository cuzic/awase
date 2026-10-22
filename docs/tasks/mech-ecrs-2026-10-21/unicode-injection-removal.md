---
title: Unicode 注入(InjectionMode::Unicode)の撤去 — 規模と影響の棚卸し
status: 調査のみ(2026-10-21、origin/develop 42e092ab)。cargo 未実行・読み取りのみ。行数はすべて見積り。「未確認」と書いたものは着手前に確かめること
created: 2026-10-21
related_adr: ["ADR-0004", "ADR-006", "ADR-023", "ADR-062", "ADR-083", "ADR-203", "ADR-212", "ADR-253"]
---

# Unicode 注入の撤去 — 規模と影響の棚卸し

## 0. 結論

### 0.1 どのくらい実際に使われているか(先に読むこと)

所有者の認識(「Unicode 注入は未学習クラスの初回接触と GJI が応答しないときの緊急避難だけ」)は、**コードと CI のログでは成り立たなかった**。

- **`force_tsf` に無い Win32/UWP の窓では、Unicode 注入は初回だけでなく毎回使われる。** 2 回目以降も Unicode のまま。
  - 事後昇格(ADR-062、`UnicodeLiteralObserverFsm`)は **`KeyAction::Romaji` を送ったときにしか動かない**(`output/mod.rs:780-809`、観測の設置は `KeyAction::Romaji` の腕の中だけ)。かなを出す打鍵はほぼすべて `KeyAction::Char` になる。`.yab` の値の kana が解決済みなら `Char` になり(`src/engine/nicola_fsm.rs:98-106`)、同梱の 6 配列(`layout/*.yab`)のローマ字のセルは**すべて**かな表(`src/kana_table.rs`、118 項目)で解決する(§U.2 の照合)。つまり同梱の配列では `Romaji` が出ず、事後昇格は起きない。学習済みの Tsf(`cache.toml` の `[injection_mode]`)が今の版で増えることも無い。
  - CI の実例: GJI(ATOK プリセット)× `Edit` で `send_keys: mode=Unicode` 16 件、観測 0 件、昇格 0 件(run 38665255261 `result-sc-eisu-km-edit-atok-1`、2026-10-15)。Mozc × `Edit`/`RICHEDIT50W` では 1 回あたり 744〜751 件が `mode=Unicode`、`→ Char(…) via Unicode` 481 件、`Romaji` 0 件、観測 0 件(run 39185861786、2026-10-19)。
  - 実機の例: LINE(`Qt663QWindowIcon`)× MS-IME で `Char('！') via Unicode`(`docs/experiments.md:533`、2026-08-09)。BUG-015 の記録も同じ。
  - **Unicode にならない窓**: `AppKind::TsfNative`(`Chrome_*`・`TeamsWebView`・`MozillaWindowClass`)→ Vk。既定 config の `force_tsf`(WezTerm・Windows Terminal と、その InputSite)→ Tsf。それ以外の Win32/UWP の窓(メモ帳・Word・Excel・LINE・awase-settings・ストアアプリなど)は **常に Unicode**。
  - 利用者の実機でどの窓が何割かは**未確認**。リポジトリに実利用のログは無い。所有者が普段 Chrome・VS Code・Windows Terminal を使っているなら、Unicode をほぼ見ないのは筋が通る(この 3 つは Unicode にならない)。
- **「GJI が応答しないときの緊急避難」は `InjectionMode::Unicode` ではなく、Tsf モード内の eager 経路**(§U.3、B 群)。**Windows Terminal・WezTerm など Tsf モードの窓だけ**で働く。条件は「GJI が 7 秒以上 idle(Medium/Long cold)」かつ「probe で GJI が応答した、または GJI の監視が不健全」。正確には「応答しないとき」ではなく、**監視が健全で応答しなかったとき(settled=false)は使わない**(`gji_warmup_coro.rs:87-107`)。Windows Terminal × GJI の利用者では、7 秒以上止まった後の最初の 1 文字のたびに起きうる。実利用の頻度は**未確認**(Mozc では修正前の実機ログで送信 506 回中 467 回、BUG-208)。
- **撤去したときの出力**(§U.4): Win32/UWP の窓は、確定済みのひらがなが入っていたところに、IME の composition(漢字変換できる)が入るようになる。IME が実際には閉じていればローマ字が入る。Tsf の eager は、かなが確定で入っていたところに VK ローマ字が入る。今の判定では、この場面は LiteralDetect を張らない(`needs_literal=false`)。GJI が準備できていなければ、リテラルのローマ字が補正されずに残る。撤去するなら、この場面で LiteralDetect を張る変更を一緒に入れる必要がある。

### 0.2 撤去の規模

- **消える行数の見積り: 合計で約 800 行(本番+テスト)。** 内訳は次の 2 つ。
  - **A. `InjectionMode::Unicode` 本体と、それがあるために存在する機構: 約 450 行。** Win32/UWP の既定を Vk にすれば到達しなくなる。
  - **B. Tsf モード内の「かな直接(unicode)送信」(eager 経路・`PendingGjiConfirm`): 約 350 行。** `InjectionMode::Unicode` とは別物で、Tsf モード(WezTerm・Windows Terminal など)の GJI long-cold 用の回避策。撤去の判断と検証は A と分ける。
  - これとは別に、e2e の判定器(Python)の `unicode`/`blind` 分類 約 40 行と、文書(README・config.toml・ADR)の記述が古くなる。
- **`KEYEVENTF_UNICODE` そのものは無くならない。** Vk/Tsf モードでも、表に無い文字(`－` など)は `CharResolution::Unicode` で直接送る(BUG-066)。他アプリが注入した `VK_PACKET` の再注入も Unicode で送る(ADR-253、BUG-198)。撤去するのは「既定の注入モードとしての Unicode」と「Tsf モード内の eager かな送信」だけ。
- **IME OFF のときの出力: Unicode 注入は担っていない。** Engine は IME OFF のとき `Inactive(ImeOff)` になり(`src/engine/engine.rs:269-283` `compute_state`)、キーは物理キーのまま OS に渡る。撤去しても IME OFF のときの出力の別手段は要らない。ただし「belief は ON・実 IME は OFF」のずれがあるとき、今の Win32 窓では Unicode 注入がかなを出してずれを隠している(リスク 2)。
- **最大のリスク 3 つ**
  1. **Win32/UWP の窓が、Chrome・Windows Terminal 向けに調整した cold-start の機構へ初めて入る。** GJI は per-VK confirm(StaleConfirm→ESC、give-up の reinit)、MS-IME は BUG-13 のゲート(最大 400ms 待つ)、long-cold の閾値は Chrome の 5 秒。ADR-083 が統一を NO-GO にした理由(per-VK confirm の構造的欠陥 BUG-045、warm 時の待ち時間が未測定)がそのまま当てはまる。対象はメモ帳・Word・Excel・LINE(Qt)・PowerToys など。
  2. **belief のずれが見えるようになる。** 今は「Engine ON・実 IME 閉」でも Win32 窓にはかなが入る(BUG-166 の偽陽性、e2e の `typed_blind`)。撤去後は生のローマ字(`ka`)が入る。IME の無い窓・IME を無効にした入力欄(パスワード欄など)も、かなではなくローマ字になる。
  3. **アプリごとの VK の扱いの差。** ADR-023 は「全アプリ VK 方式」を「UWP で VK が正しく処理されない」として却下した(再現の記録は無い)。ADR-006 は PowerToys Command Palette で VK の取りこぼしがあり、Unicode で解決したと書く。TSF 専用で Win32 クラスの窓(`force_tsf` に無いもの)は、撤去後は Vk(重畳)になる(WezTerm は重畳で `mう` になった、ADR-0004)。今の版でも学習による Tsf への昇格は起きていないので、この点は撤去で悪くなるわけではない。MS-IME では小指シフト面の全角記号が半角になる(BUG-049 の型)。
- **使用頻度の訂正から出る副産物: 事後昇格の一式(A5・A6・A7 の書き込み側、約 240 行)は、Unicode の撤去を待たずに今でも消せる候補。** 同梱の配列では到達しない(§U.2)。ただし独自の `.yab` で、かな表に無いローマ字を書いた利用者だけには届きうる。
- **注意: ADR-083 は「`InjectionMode::Unicode` の廃止」「`UnicodeLiteralObserverFsm` の撤去」を「明示的にやってはいけないこと」に挙げている**(`docs/adr/083-injection-mode-per-vk-unification-investigation.md` の「明示的にやってはいけないこと」節)。ADR-023 は全アプリ VK を却下している。撤去の前に、この 2 つを覆す ADR を所有者の判断で起票する必要がある。

---

## U. 使用実態 — Unicode 注入になる条件・頻度・撤去後の出力

### U.1 `InjectionMode::Unicode` になる条件(全部)

注入モードはフォーカスの確定のたびに決まる(`runtime/focus_tracking.rs:91-96,135-138`、`runtime/mod.rs:1781-1792`)。

1. `injection_hint`(`focus/tracker.rs:97-124`)が `Default` を返す。条件は次の 3 つが全部当たらないこと。
   - config の `app_overrides.force_tsf` に、プロセス名とクラス名が一致する(InputSite の窓は前面窓のクラスでも照合、`focus/classifier.rs` `input_site_fallback_matches`)。
   - config の `app_overrides.force_vk` に一致する。
   - 学習済みの Tsf(`InjectionModeStore::has_tsf`、`cache.toml` の `[injection_mode]`)にクラス名がある。
2. かつ `detect_app_kind(class)` が `TsfNative` でない(`class_names.rs:273-288`)。

これで**毎回** Unicode になる。「初回だけ」という仕組みは無い。初回と 2 回目以降を分けうるのは、事後昇格(ADR-062)で上の 3 番目の条件が後から成り立つ場合だけ。

**事後昇格の条件**(`platform.rs:1002-1009` と `output/mod.rs:780-809`):
- 依頼: Unicode モードで、クラスが学習済みでない `send_keys` のたびにフラグを立てる。
- 設置: そのフラグを消費するのは **`KeyAction::Romaji` の送信だけ**。さらに `!has_pending_tsf()` かつ `ImeModeFsm` が Hiragana/Katakana のとき。
- 判定: 100ms 後に GJI プロセスの write bytes が増えていなければ Tsf に昇格し、`cache.toml` に保存する(`unicode_literal_observer.rs:70-91`)。GJI の検出(GJI か MS-IME か)は判定に入っていない。

### U.2 事後昇格が実際には起きないこと

- `KeyAction::Romaji` は、`.yab` のローマ字の値がかな表で 1 文字に解決できないときだけ作られる(`nicola_fsm.rs:98-106`、`yab/mod.rs:363-389` の `resolve_kana`)。
- 同梱の 6 配列のローマ字のセルをかな表(`src/kana_table.rs` の 118 項目)と照合すると、**表に無いものは 0 件**(全角英字を NFKC で正規化して照合。`nicola.yab`・`nicola_f.yab`・`nicola_kakutei.yab`・`nicola_kb232.yab`・`nicola_keytop.yab`・`nicola_us.yab` のすべて)。
- CI のログでも `Romaji` の送信は 0 件で、`[unicode-obs]` も 0 件。同梱の配列を使う限り、事後昇格は起きない。独自の `.yab` にかな表に無いローマ字を書いた利用者だけが例外。
- したがって `[injection_mode]` の学習は、今の版では増えない。既存の利用者の `cache.toml` に、昔の版(ADR-062 の実装は 2026-06-25)の学習が残っている可能性はある(**未確認**)。

### U.3 頻度の裏付け(ログ)

`[focus-sync] … → mode=X` の窓の種類と、打鍵の送り方を数えた。

| 出典 | 入力先 | `mode=Unicode` の窓 | 打鍵の経路 |
|---|---|---|---|
| run 39592330739(`ci/e2e-b2-bug18`、2026-10-21)`result-sc-startup-gji-tsf-on-1` ほか 3 件 | `--form=tsf`(`Chrome_RenderWidgetHostHWND` に見せた RichEdit) | `Windows.UI.Composition.DesktopWindowContentBridge`・`Static`(打鍵なし) | `→ Char via VK Batched (Chrome)` 1〜11 件。`mode=Unicode` の送信 0 件 |
| run 39185861786(`ci/bug-207-baseline-mozc`、2026-10-19)`ts-edit-mozc-*`・`ts-rich-mozc-*` 4 件 | `Edit`・`RICHEDIT50W` × Mozc | 入力先そのもの | 各 744〜751 件が `send_keys: mode=Unicode`、`→ Char via Unicode` 481 件、`Romaji` 0、`[unicode-obs]` 0、昇格 0 |
| run 38665255261(`ci/e2e-eisu-composite`、2026-10-15)`sc-eisu-km-edit-atok-1`・`sc-hwndcache-edit-refocus-works-2` | `Edit`・`TypingStressTop` × GJI(ATOK) | 入力先そのもの | `mode=Unicode` の `Char` 16 件・11 件、`[unicode-obs]` 0、昇格 0。A8 の `WarmupComplete(unicode)` 51 件・11 件 |
| `tools/e2e/ime_key_matrix/results/*/awase-full.log`(2026-09-19 ごろ、64 本。撤去済み、`git archive 675be51a` で展開して読む) | `Edit`・`ImeKeyMatrixSpikeWindowClass`・`RICHEDIT50W` × GJI | `Edit` 132・`ImeKeyMatrixSpikeWindowClass` 69・`RICHEDIT50W` 59・ほか | `send_keys: mode=Unicode` 377 件、`mode=Tsf` 53 件(Windows Terminal)。`→ Char via Unicode` 187 件。`[tsf-transmit]` は 30 件すべて `eager=false` |
| 実機(`docs/experiments.md:533`、BUG-015) | LINE × MS-IME | LINE | `Char('！') via Unicode` |

コマンド(例): `grep -o '\[focus-sync\] hwnd=[^ ]* class="[^"]*" app_kind=[A-Za-z0-9]* hint=[A-Za-z]* → mode=[A-Za-z]*' dist/awase.log | sed 's/hwnd=[^ ]* //' | sort | uniq -c`、`grep -c 'send_keys: mode=Unicode'`、`grep -c 'unicode-obs\]'`、`grep -c 'WarmupComplete(unicode)'`。artifact は `gh run download <run> -R cuzic/awase -n <name>`。

言えること: **CI の Win32 の入力先(EDIT・RichEdit・自前の窓)では、打鍵の 100% が Unicode で、昇格は 1 度も起きない。** 言えないこと: 利用者の実機で Win32 の窓にどれだけ打っているか(未確認。不具合報告の journal〈R2〉を集計すれば数えられる)。

### U.4 緊急避難(Tsf モードの eager)の条件と、撤去前後の出力

**eager の条件**(`gji_warmup_coro.rs:69-107`、`probe_fsm.rs:131-180` `decide_transmit_plan`、`gji_fsm.rs:107-132` `ColdKind`):

- 注入モードが Tsf(Windows Terminal・WezTerm と、`force_tsf` の窓)で、GjiWarmupCoro を通る cold の送信。
- `ColdKind` が Medium/Long(GJI の idle が `MEDIUM_IDLE_PROBE_MS`=7000ms 以上)。
- `nc_confirmed`。次のどれか:
  - 監視が健全で probe が settled、かつ cold の理由が settle を要しない。
  - 監視が不健全(このとき settled に関係なく真)。
  - 確定キーの後で settled。
- deferred が空。
- GJI の証拠が無いと確定していない(Mozc 系は除く、BUG-208)。

**`PendingGjiConfirm`**: eager で送った後、`gji_last_io_ms > last_unicode_transmit_ms` になるまで、次の warm のキーもかなで送る(`vk_send.rs:562-593`、`warm_send_plan.rs:74-88`)。

**監視が健全で GJI が応答しない(settled=false)ときは eager を使わない。** VK 経路で送り、LiteralDetect を張る(`needs_literal = !nc_confirmed && is_tsf_mode && gji_active`)。

| 場面 | 今の出力 | 撤去後の出力 |
|---|---|---|
| Win32/UWP の窓(`mode=Unicode`)× IME が実際に ON | 確定済みのひらがな(変換できない。IME の入力モードに関係なくひらがな) | VK ローマ字 → IME の composition(変換できる)。cold なら GJI は per-VK confirm、MS-IME は最大 400ms の保留を通る(遅れ・誤判定の可能性、§5 R1・R2) |
| Win32/UWP の窓 × belief は ON・実 IME は閉 | ひらがな(ずれが見えない) | ローマ字がそのまま入る(Chrome と同じ見え方) |
| Win32/UWP の窓 × IME の無い入力欄・IME 無効 | ひらがな | ローマ字 |
| Tsf × GJI × 7 秒以上 idle の最初の 1 文字(eager) | 確定済みのかな 1 文字(その文字は変換対象に入らない。triage `01M13EACMQ7D2VETW75N0BTZ9C` の「変換キーが効かない」はこれと推定) | VK ローマ字を即送る。`needs_literal=false` のままなら、GJI が準備できていないとリテラルのローマ字が補正されずに残る(`bあ` 型)。**撤去するなら eager の場面を `needs_literal=true`(LiteralDetect で回収)に変える必要がある** |
| Tsf × eager の後 GJI が I/O を返すまで(`PendingGjiConfirm`) | 後続もかなで確定 | VK run(warm パスの LiteralDetect の対象) |
| 表に無い文字(`－` など、全モード) | `KEYEVENTF_UNICODE` | 変わらない(残す) |

「文字が出ない」「VK のまま待つ」になる経路は、撤去後にも新しくは生まれない。待つのは、既にある per-VK confirm と MS-IME のゲートだけ。

---

## 1. Unicode 注入があるために存在する機構

確認: `git grep -n "InjectionMode::Unicode" -- '*.rs'` は 7 ファイル 15 件(gji_fsm.rs 4、platform.rs 3、output/mod.rs 3、sender.rs 2、key_pipeline.rs 1、vk_send.rs 1、types.rs 1)。

### A. `InjectionMode::Unicode` 本体(既定を Vk にすると到達しなくなる)

| # | 機構 | 場所 | 役割 | Unicode が無くなると | 行数 |
|---|---|---|---|---|---|
| A1 | `InjectionMode::Unicode` の値 | `crates/awase-windows-base/src/state/injection_mode.rs:13-20` | 3 つの注入モードの 1 つ | 値ごと消える | 2 |
| A2 | 既定の決定 `injection_mode_for` | `crates/awase-windows/src/output/types.rs:19-31` | `Default` のとき TsfNative→Vk、それ以外→Unicode | `Default`→Vk の 1 行に縮む | −6 |
| A3 | `UnicodeSender`・`OutputSession::is_vk_mode` | `crates/awase-windows/src/output/sender.rs:19-33,84,90-92` | モード別の送信口 | `UnicodeSender` は消え、`is_vk_mode` は常に真 | 約 18 |
| A4 | `send_romaji_as_unicode`・`send_romaji_per_key` | `output/key_injector.rs:134-151`、`output/vk_send.rs:361-368` | ローマ字→かなに変換して Unicode で送る。表に無いときは 1 文字ずつ VK | 呼び出し元が Unicode モードだけ(`git grep -n "send_romaji_per_key"` は定義と `send_romaji_as_unicode` からの 1 件だけ) | 約 17 |
| A5 | Unicode 送信後の GJI write 観測(ADR-062) `UnicodeLiteralObserverFsm` | `crates/awase-windows/src/tsf/warmup/unicode_literal_observer.rs`(95 行)、`output/mod.rs:167-172,310,340-346,781-809`(フラグ・依頼・設置)、`platform.rs:1002-1009`(依頼) | Unicode で送った後 100ms に GJI の write が無ければ、その窓のクラスを Tsf に上げる | Unicode で送らないので観測の対象が無い | 約 150 |
| A6 | Tsf への事後昇格の結果の受け渡し | `ProbeAction::UpgradeToTsf`(`tsf/warmup/probe_fsm.rs:253`)、`StageEndReason::UpgradedToTsf`(`tsf/gji_fsm.rs:71`)、`probe_io.rs:415-420,600-603,961-972`、`output/mod.rs:264,1068,1094,1101,1121`、`platform.rs:430,458-476`、`journal_policy.rs:66,78,321` | A5 の結果を focus 層と journal へ届ける | A5 が消えれば唯一の生産者が消える(`git grep -n "UpgradeToTsf"` は A5 の 1 か所だけが作る) | 約 45 |
| A7 | 学習の保存 `InjectionModeStore` | `focus/learning_store.rs:288-360`(+ BUG-108 のテスト `:461-495` の一部)、`focus/tracker.rs:26,46,53,107-109,120-122,330-341`、`app/bootstrap.rs:792` | `cache.toml` の `[injection_mode]` に Tsf を要するクラスを保存し、`injection_hint` が `ForceTsf` を返す | 書き手は A5 だけ。**読み手は残せる**(既存の利用者の学習済みクラスを Tsf のまま保つ移行)。消すかは所有者判断(§3 PR-5) | 約 110 |
| A8 | Unicode モードでの GjiFsm の即時 warm 化 | `platform.rs:539-560`(`StartProbe` を受けたら `WarmupComplete` を即送る。ログ `UnicodeImmediate`) | Unicode では probe を作らないので、GjiFsm を OnCold に留めないための代行 | Vk では probe を作るので不要 | 約 22 |
| A9 | ADR-203 (i) G2 の Unicode 除外 | `crates/awase-ime-core/src/state/gji_direct_mechanism.rs:291-310`(引数 `injection_is_unicode`)、テスト `:455-467`(2^5 全列挙)、呼び出し `platform.rs:989-995`、ablation `tools/e2e/ime_key_matrix/ablations/a8-no-gji-reopen-sync.sh:8-9` | Unicode では GjiFsm に KeyInput を送らないので同期しない | 引数が常に偽。引数ごと消し、全列挙は 2^4 になる | 約 10 |
| A10 | `reopen_obligation` の doc の Unicode の段落 | `gji_direct_mechanism.rs:334-340` | 「Unicode でも Reopen を出す」の経緯(baseline/atok-passthrough-cold の I4 FAIL) | 記述が意味を失う | 約 6 |
| A11 | long-idle の閾値の Unicode 腕 | `tsf/gji_fsm.rs:1022-1037`、既定値 `:328` | Unicode=`LONG_IDLE_MS`(10s)、Vk=`CHROME_LONG_IDLE_MS`(5s) | 腕が消える。**Win32 窓の long-cold は 10s→5s に変わる**(§2) | 約 6 |
| A12 | かなロックの警告の除外 | `runtime/key_pipeline.rs:3070-3077` | Unicode なら OS のかなロックを読まない | 条件の片側が消える | 2 |
| A13 | 診断のラベル・既定値 | `ime_diagnostic.rs:359-372`、`output/mod.rs:296`(`injection_mode: InjectionMode::Unicode` の初期値) | | 初期値を Vk に | 3 |
| A14 | フォーカス時(A1)と IME 種別の同期(G1)の GjiFsm への `ImeOn` | `runtime/focus_tracking.rs:197-207`、`runtime/message_handlers.rs:975-986`、`architecture_guard.rs:7424-7435` の一覧 | BUG-18 の対策。belief=ON なら GjiFsm へも ON を通知 | **PR #765(未マージ、`gh pr view 765` で OPEN を確認)のモデル検証: VK 注入だけの世界では G2 が全部拾い、A1・G1 は二重。Unicode があるうちは二重ではない。** Unicode 撤去後に消せる(A1 の `record_confirmed(true)` は別目的なので残す) | 約 20 |
| A15 | 古い doc(撤去済みの Unicode long-cold warmup の名残) | `output/mod.rs:66`(`unicode_cold_defer`)、`vk_send.rs:182`、`tsf/warmup/mod.rs:24`、`tickable_fsm.rs:13,70-75`、`vk.rs:68`(`send_unicode_cold_warmup_keys`)、`ime_mode_fsm.rs:165,282`、`ime_key_sequence_golden.rs:120`、`architecture_guard.rs:7512` | ADR-212 P5 で撤去済みの機構を指す記述 | Unicode 撤去と無関係に今でも古い。同じ PR で直す | 約 15 |

小計 約 450 行(A7 を残すなら約 340 行)。

### B. Tsf モード内の「かな直接送信」(eager 経路・`PendingGjiConfirm`)

`InjectionMode::Unicode` ではない。Tsf モード(WezTerm・Windows Terminal など)で GJI が long-cold のとき、最初の 1 文字を `KEYEVENTF_UNICODE` のかなで送る経路と、その後 GJI が I/O で応答するまで次の文字もかなで送る経路。導入は `598fbb1f`(2026-05-20、件名が `experiment:`)。

| # | 機構 | 場所 | 役割 | 撤去すると | 行数 |
|---|---|---|---|---|---|
| B1 | eager の決定 | `tsf/warmup/probe_fsm.rs:155-166`(`decide_transmit_plan`)、テスト `:857-919` 付近 | `nc_confirmed` かつ `forces_prepend_f2`(long cold)かつ deferred 無しなら eager。BUG-208 で Mozc 系は除外済み | 常に VK 経路になる | 約 60 |
| B2 | かな直接送信 | `output/vk_send.rs:117-178`(`TsfSendPipeline::transmit` の unicode 腕)、`tsf/output.rs:95-100`(`kana_for_romaji_static`) | eager のときかなを `TSF_MARKER` 付き Unicode で送る | VK run だけになる | 約 35 |
| B3 | `PendingGjiConfirm` | `output/probe_io.rs:83-93`、`output/vk_send.rs:562-593`、`tsf/probe.rs:83-85,106,138,156,233-240`(+ テスト `:1005-1013`)、`crates/awase-windows-plans/src/state/warm_send_plan.rs:62-88`(+ テスト `:188-230`) | unicode 送信の後、GJI が I/O で応答するまで次の warm キーも unicode で送る | 消える。B1 が無ければ唯一の書き手(`probe_io.rs:89`)に到達しない | 約 110 |
| B4 | `used_eager_path` の受け渡し | `output/mod.rs:271-272`(`WarmupOutcome`)、`vk_send.rs:425-431`(常に `false` の定数)、`gji_warmup_coro.rs:51,128,164,220-226`、`probe_io.rs:474,500-504`(ADR-270 の Mozc 確認の除外)、`ms_ime_ready_coro.rs:156` | | フィールドごと消える | 約 30 |
| B5 | テストとガード | `probe_io.rs:1040-1210` の eager 系テスト、`architecture_guard.rs:8350-8390`(BUG-208 の配線の固定) | | 書き直し・削除 | 約 110 |

小計 約 350 行。

### 残すもの(Unicode が名前に付くが撤去の対象外)

| 機構 | 場所 | 残す理由 |
|---|---|---|
| `KeyInjector::send_unicode_char`・`push_unicode_char_inputs` | `output/key_injector.rs:124-185`、`vk_send.rs:180-186,800-804` | Vk/Tsf の `CharResolution::Unicode` フォールバック(`vk_send.rs:718-723,778-783`)。`symbol_to_vk` に無い文字(`－` BUG-066、BUG-049 で外した全角記号など)はこれで送る |
| `VK_PACKET` の再注入 | `vk.rs:378-393`、`lib.rs:383-399`、`architecture_guard.rs:8775` | ADR-253・BUG-198。他アプリの注入文字を失わない |
| `win32.rs::input_may_mutate_conv` の Unicode 判定、journal の `SentKeyEvent.unicode` | `win32.rs:184-196,337`、`journal.rs:549-585` | 上の 2 つが残るので要る |
| `lints/actuation_call_guard` の `send_unicode_char` | `lib.rs:82` | 同上 |
| awase-settings の「Unicode 文字としてそのまま送信」 | `crates/awase-settings/src/main.rs:4050,4145` | `.yab` のリテラル値の説明。注入モードではない |

---

## 2. Unicode でなくなると挙動が変わる箇所

対象: `AppKind::Win32`(`detect_app_kind` で `Chrome_*`・`TeamsWebView`・`MozillaWindowClass`・UWP の 3 種以外すべて、`crates/awase-windows-base/src/focus/class_names.rs:273-288`)と `AppKind::Uwp`(`Windows.UI.Core.CoreWindow`・`ApplicationFrameWindow`・`Windows.UI.Input.*`)で、`force_tsf`/`force_vk` にも学習済みの Tsf にも当たらない窓。メモ帳・Word・Excel・LINE(Qt)・awase-settings(egui)・conhost・PowerToys・ストアアプリなど。

### 2.1 IME が OFF(直接入力)のとき

- **Engine は動かない。** `compute_state` は `!ctx.ime_on` で `Inactive(ImeOff)` を返し(`src/engine/engine.rs:269-283`)、キーは物理キーのまま渡る。Unicode 注入は IME OFF のときのかな・英数の出力を担っていない。撤去しても IME OFF のための別手段は要らない。
- **変わるのは「belief は ON・実 IME は OFF」のとき。** 今の Win32 窓では、Engine が出したかなが Unicode で入る(e2e の判定器自身が `check_reopen.py:17`「Unicode 注入では IME が閉じていても文字が入る。IME 状態の証拠にならない」と書く)。BUG-166 では Engine ON・MS-IME 閉のまま、Unicode の「か」でハーネスの準備確認が通っていた(偽陽性)。撤去後は同じ場面で `ka` が入る。Chrome では今もそうなっている。
- **IME の無い窓・IME を無効にした入力欄**(パスワード欄、`ImmAssociateContext(NULL)`、IME 非対応のアプリ): 今は belief が ON ならかなが入る。撤去後はローマ字が入る。belief がこれらの窓で ON になる頻度は未確認。

### 2.2 IME が ON のとき

| 観点 | 今(Unicode) | 撤去後(Vk) | 根拠 |
|---|---|---|---|
| 漢字変換 | できない(確定済みのひらがなが入る。IME の composition を経ない) | できる | ADR-006「IME を完全にバイパス」、triage `01M13EACMQ7D2VETW75N0BTZ9C`(UWP+GJI で変換キーが効かない報告。eager 経路の副作用と推定) |
| IME の入力モード | 無視して常にひらがな(カタカナモードでもひらがな) | IME のモードに従う | `send_romaji_as_unicode` は `kana_for_romaji` の結果を送るだけ(`key_injector.rs:144-151`) |
| IME のローマ字表のカスタマイズ | 影響しない | 影響する(AZIK などの独自表で違うかなになりうる) | 未確認(利用者数は不明) |
| 小指シフト面の全角記号(MS-IME) | Unicode で全角のまま | `symbol_to_vk` にある記号は VK 経由になり、shift-conv-guard と競合して半角になりうる | BUG-049(Vk 経路で `！`→`!`、Phase 1・2 対応済み・実機未検証) |
| cold-start の機構(GJI) | `GjiFsm` に KeyInput を送らず、`StartProbe` は即 `WarmupComplete`(A8) | `send_romaji_batched` が KeyInput を送り、cold なら Chrome 用の per-VK confirm(`TsfProbeCoro::new_chrome`)、StaleConfirm→ESC、give-up の reinit(`VK_IME_OFF`→`VK_IME_ON`)に入る | `vk_send.rs:215-345`。BUG-045(per-VK confirm は代理証拠のタイムアウト判定)、BUG-168・170・171 |
| cold-start の機構(MS-IME) | 待たない | IME ON の直後は `ImeModeFsm` の NATIVE 確認まで最大 `MS_IME_READY_CONFIRM_MS`=400ms 保留(BUG-13) | `vk_send.rs:487-552`、`tuning.rs:182` |
| long-cold の閾値 | `LONG_IDLE_MS`=10s | `CHROME_LONG_IDLE_MS`=5s | `gji_fsm.rs:1031-1037`、`tuning.rs:102,119`。Win32 窓での実測は無い(tuning-constants 規約の対象) |
| 送信の形 | かな 1 文字=2 イベント | ローマ字 2〜3 文字=4〜6 イベント(重畳順、1 回の `SendInput`) | `key_injector.rs`。PowerToys 等のフックとの干渉で取りこぼした記録がある(ADR-006) |
| 送信後の IME ポーリングのガード | 付かない | `mark_vk_output` が付く(「て→tえ」防止) | `output/mod.rs:868-872` |
| 送信順(重畳 vs 逐次) | — | Vk は重畳(M↓U↓M↑U↑)。TSF 専用の窓では `mう` になった記録 | ADR-0004(WezTerm)。Win32 の IMM32 窓での重畳の可否は未確認 |

### 2.3 設定と移行

- **`InjectionHint`(`focus/classifier.rs:58-65`)**: `ForceTsf`/`ForceVk`/`Default`。撤去後は `Default` と `ForceVk` がどの窓でも同じ Vk になり、**`force_vk` は意味を失う**。config(`src/config.rs:871-906`、`app_overrides.force_vk`)の読み込みは互換のため残し、警告を出す形が無難。awase-settings の「アプリ別」タブ(`main.rs:3646,4880`)で `force_vk` を編集できるので、文言か UI を直す。
- **学習済みの `[injection_mode]`(`cache.toml`)**: A7 を読み手だけ残せば、既存の利用者が Tsf に上がったクラスはそのまま Tsf。消すと Vk に戻る。
- **`config.toml:50-65` のコメント**(「Win32 モード(Unicode 直接注入)がデフォルト」)・**README.md:283-285 の表**(「メモ帳、Word、Excel | Unicode 直接注入」)・`docs/app-onboarding-checklist.md:34`・awase-settings `main.rs:2589` の説明文・`AppKind` の doc(`kinds.rs:9-12`)を直す。
- 移行の要否: 設定ファイルの書き換えは不要(`force_vk` を読み捨てにするだけ)。

### 2.4 実際にどれだけの窓が今 Unicode で送っているか

- 既定の config(`config.toml:59-62`)の `force_tsf` は WezTerm と Windows Terminal だけ。それ以外の Win32/UWP は既定で Unicode。
- 学習(A5)で Tsf に上がる窓は、同梱の配列では生まれない(§U.2)。CI のログでも `[unicode-obs]` は 0 件で、Win32 の入力先はすべて `mode=Unicode` のまま(§U.3)。

---

## 3. 撤去の順序案

| PR | 内容 | 規模 | 検証 |
|---|---|---|---|
| **PR-00**(Unicode の撤去と独立、今でもできる) | 事後昇格の書き込み側(A5・A6、`InjectionModeStore::learn_tsf`)の撤去。同梱の配列では到達しない(§U.2)。ADR-083 が「撤去するな」に挙げているので ADR の追記が先。`[injection_mode]` の読み手は残す | 約 −240 行 | Linux の単体(`probe_io` の dispatch テストの `UpgradeToTsf` を削除)、`cargo check --target x86_64-pc-windows-msvc` |
| **PR-0**(文書) | ADR を起票: ADR-023「全アプリ VK 方式は不採用」と ADR-083「明示的にやってはいけないこと」を覆す根拠と、Win32/UWP の既定を **Vk にするか Tsf にするか** の決定(所有者判断。Vk は Chrome 用の重畳順と per-VK confirm、Tsf は逐次順と GjiWarmupCoro。どちらが IMM32 の窓に合うかは未確認) | 文書のみ | opus-adversarial-consult |
| **PR-1**(測定、本番コード変更なし) | **既存の `force_vk` 設定で、e2e の Win32 窓を Vk にした構成を足す**(例: `force_vk = [{ process = "typing_stress.exe", class = "Edit" }]`、同様に `RICHEDIT50W`・`ImeKeyMatrixSpikeWindowClass`・awase-settings)。既定の Unicode の構成と並べて比べる。Tsf を候補にするなら `force_tsf` で同じことをする | e2e-ime.yml に構成を追加 | GitHub の Windows ランナー: `ts-edit-*`・`ts-rich-*`・`tsx-edit-*/tsx-rich-*-20ms-cold`・`tsx-{edit,rich}-gji-10ms-idle12s`・`sc-startup-{gji,msime}-edit-{on,off}`・`tsx-bugreport-*`(egui の窓)、e2e-ime-smoke の `baseline`・`atok-passthrough-cold`(窓は `Edit`)。**フォーマットは「入力先のテキスト」と不変条件(I1〜I5)**。実機: メモ帳・Word・Excel・LINE・PowerToys Command Palette・UWP(設定の検索欄など)・`force_tsf` の無い Windows Terminal で、GJI と MS-IME、cold(起動直後・10 秒以上の idle)と warm |
| **PR-2**(挙動を変える。小さく、revert しやすく) | `injection_mode_for` の `Default` を Vk(または PR-0 の決定)にする。A2・A13 だけ。A5 は自然に到達しなくなる | 約 10 行 | PR-1 と同じ構成を既定のままで回す。`experiment-logging.md` の対象(revert するなら本文にアプリ×IME×症状)。fix-requires-evidence のキー選択・warmup ファミリーなので (a)/(b) |
| **PR-3**(到達しないコードの撤去) | A1・A3〜A6・A8〜A12・A15 | 約 −330 行 | Linux: `cargo nextest run --workspace --lib`(`needs_belief_sync_on` の全列挙を 2^4 に、`probe_io` の dispatch テスト、`gji_fsm` の `reopen_updates_injection_mode_even_when_ignored` を Vk に)、`architecture_guard`。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`(`output/`・`platform.rs` は cfg(windows)) |
| **PR-4** | A14(A1・G1 の GjiFsm への `ImeOn` の GJI 分岐)。PR #765 のモデル検証(VK 注入だけの世界で A1・G1 を外しても「送信時に OffCold」の状態が増えない)を回帰テストとして入れる | 約 −20 行 | PR #765 の到達可能性の全列挙(Linux)。e2e: `sc-reopen-tsf-gji-gap{300,600,900}`(`--require-sync`)、負の対照 `sc-reopen-tsf-gji-gap600-nofix`(ablation a8 は A9 の引数が消えるので書き直す) |
| **PR-5**(所有者判断) | A7 `InjectionModeStore` を消すか、読み手だけ残すか。消すなら `cache.toml` の `[injection_mode]` は読み捨て | 約 −110 行 | Linux の `learning_store` テスト(BUG-108 のテストは `[injection_mode]` を例に使うので直す) |
| **PR-6**(別系統、A と独立) | B(eager・`PendingGjiConfirm`)。Tsf モードの GJI long-cold の最初の文字を VK 経路と LiteralDetect に任せる。今の `decide_transmit_plan` は eager の場面(`nc_confirmed`)で `needs_literal=false` なので、そのまま eager を外すとリテラルが補正されない。`needs_literal` の条件も同じ PR で変える(§U.4) | 約 −350 行 | e2e: `tsx-tsf-{gji,msime}-20ms-cold`、`sc-reopen-tsf-gji-*`(`--require-cold`)、`bug171-tsf-gji-*`、Mozc の構成(ADR-269・270)。**実機: Windows Terminal × GJI の 10 秒以上 idle の後の最初の文字**(`bあ`・`nお` 型のリテラル化が戻らないか、BUG-002 の系統)。warmup ファミリー |
| **PR-7**(e2e と文書の後片付け) | 判定器の `unicode`/`typed_blind`/`blind`(`check_reopen.py`・`check_drift_recovery.py`・`check_run_validity.py`・`e2e_common.py` の `UNICODE_RE`・`test_check_*.py`)を外す。e2e-ime.yml の「素の EDIT は Unicode 注入なので構成にしない」(`:517`、README `tools/e2e/ime_key_matrix/README.md:142`)を外し、EDIT を drift/reopen の入力先に足せるようにする(撤去の利点: EDIT が IME 状態の証拠になる)。README・config.toml・ADR-0004/006/023/062/083 の status | Python 約 −40 行 | `invariants-unit`(Linux) |

- tests/・golden で Unicode を前提にしているもの: `ime_key_sequence_golden.rs` は送信キー列の golden で Unicode 送信を含まない(`:120` のコメントだけ)。`e2e_windows.rs` の Unicode は検証側の `send_unicode_string` で無関係。`gji_fsm.rs:2656-2677` のテストは `InjectionMode::Unicode` を入力に使うので直す。e2e-ime-smoke の 2 構成(`baseline`・`atok-passthrough-cold`)は窓が `Edit` で今は Unicode。PR-2 の後は Vk になり、**`invariant_limits.json` の上限を超える可能性がある**(per-VK confirm の I5 など。未確認)。PR-1 で先に数を見る。
- e2e-ime.yml の構成のうち入力先が Win32 窓のもの: `--form=edit` を含む行 21、`rich` 6、`bugreport` 4(`grep -c` の行数。構成数ではない)。

---

## 4. 撤去で連鎖して不要になる既存の機構

| 機構 | 不要になる理由 | 行数 |
|---|---|---|
| G2 の Unicode 除外(A9) | 引数が常に偽 | 約 10 |
| A1・G1 の GjiFsm `ImeOn`(A14) | PR #765: VK 注入だけなら G2 と二重 | 約 20 |
| Unicode での GjiFsm 即時 warm 化(A8) | Unicode が無い | 約 22 |
| `UnicodeLiteralObserverFsm` と事後昇格(A5・A6) | Unicode が無い | 約 195 |
| `InjectionModeStore`(A7) | 書き手が A5 だけ(所有者判断) | 約 110 |
| `UnicodeSender`・`send_romaji_as_unicode`・`send_romaji_per_key`(A3・A4) | 呼び出し元が無い | 約 35 |
| long-idle の Unicode 腕(A11)・かなロックの除外(A12)・診断(A13) | | 約 11 |
| 古い doc(A15) | ADR-212 P5 で撤去済みの `UnicodeColdWarmupFsm` 等への言及 | 約 15 |
| `force_vk` 設定(§2.3) | 既定と同じになる(互換のため読み込みは残す) | 小 |
| eager・`PendingGjiConfirm`(B) | Unicode 撤去の連鎖ではない。所有者の「Unicode 注入は害」の方針で同時に検討するもの(BUG-208 が Mozc で実害を記録) | 約 350 |
| tuning 定数 | **連鎖して消える `tuning.rs` の定数は無い**(`LONG_IDLE_MS` は Tsf が、`CHROME_LONG_IDLE_MS` は Vk が使い続ける。Unicode long-cold の定数は ADR-212 P5 で撤去済み) | 0 |
| e2e 判定器の `unicode`/`blind` 分類 | 該当する送信が無くなる | 約 40(Python) |

合計 約 800 行(A 約 450 + B 約 350)、ほかに Python 約 40 行。

---

## 5. リスクと、撤去してはいけない理由の候補

具体的に「どのアプリ×IME×状態で何が壊れうるか」が言えるものだけを挙げる。

| # | アプリ × IME × 状態 | 壊れうること | 根拠 | 確かめ方 |
|---|---|---|---|---|
| R1 | メモ帳・Word・Excel・LINE × GJI × cold(起動直後・5 秒以上の idle) | per-VK confirm が StaleConfirm→ESC に倒れて未確定の文字が消える、give-up の reinit(`VK_IME_OFF`→`VK_IME_ON`)が走る、最初の文字がリテラル化する | ADR-083 NO-GO 理由 1・3(BUG-045 の構造的欠陥、warm 時の待ち時間が未測定)、BUG-168・170・171 は Chrome/TSF での実例 | PR-1 の `tsx-{edit,rich}-gji-*-cold`・`idle12s`、実機 |
| R2 | 同 × MS-IME × IME ON の直後 | 最初の文字が最大 400ms 保留される(打鍵の遅れ)。保留の期限切れで give-up がラッチされる | BUG-13、BUG-049 の却下案の記述(`ms_ime_gate_give_up` のラッチ) | PR-1 の `sc-startup-msime-edit-*`、`ts-edit-msime-*` |
| R3 | Win32/UWP の窓 × 任意の IME × belief=ON・実 IME=閉 | かなではなく `ka` が入る | BUG-166(EDIT×MS-IME 起動直後に Engine ON・実 IME 閉)、`check_reopen.py:17` | 撤去すれば Chrome と同じ見え方になるだけで、新しい不具合ではない。drift correction の追随が Win32 で働くかを `sc-driftrecovery-*` 型の構成で EDIT にも足して見る |
| R4 | パスワード欄・IME 非対応のアプリ × belief=ON | かなではなくローマ字 | 推論(`compute_state` が belief で決まる)。頻度は未確認 | 実機 |
| R5 | UWP(ストアアプリ・設定の検索欄など) × 任意 | VK が正しく処理されない | ADR-023「全アプリ VK 方式: UWP で VK が正しく処理されない」(再現の記録・アプリ名は無い)。一方、Windows Terminal の InputSite(`Windows.UI.Input.InputSite.WindowClass`)は `force_tsf` の InputSite フォールバックで VK を受けて動いている(`focus/classifier.rs:80-100`) | 実機。CI は「UWP は安定して動かせない」(`typing_stress/main.rs:21`) |
| R6 | PowerToys Command Palette ほか、他のキーボードフックがある環境 | VK の取りこぼし | ADR-006(当時の batched/per_key で取りこぼし、Unicode で解決)。現在の Vk の送り方(1 回の `SendInput` で重畳)で再発するかは未確認 | 実機 |
| R7 | TSF 専用で Win32 クラスの窓(`force_tsf` に無いもの) × GJI | 撤去後は Vk(重畳)。重畳で `mう` 型の崩れ。今は Unicode のまま(A5 の学習は同梱の配列では起きない、§U.2)なので、Unicode でかなが入っていたものが変わる | ADR-0004(WezTerm)。ADR-083 訂正 2 は「Tsf は学習で増える」と書くが、今の版では増えない | A7 を読み手だけ残す(昔の版の学習を守る)。新しい窓は PR-1 の実機 |
| R8 | 任意の Win32 窓 × MS-IME × 小指シフト面の全角記号 | `！` が `!` になる | BUG-049(Vk 経路での実例、対応済み・実機未検証) | `sc-charthumb` 型の構成を EDIT で |
| R9 | 任意 × IME のローマ字表を独自にしている利用者 | 違うかなになる | 推論。Chrome で既に同じ | 報告待ち |
| R10 | Tsf モード × GJI × long-cold(PR-6 のみ) | eager を外すと最初の文字がリテラル化する(`bあ` 型) | eager は `598fbb1f` の実験として入った回避策。BUG-002 の系統 | `tsx-tsf-gji-20ms-cold`、Windows Terminal の実機 |

**撤去してはいけない理由として確定したものは見つからなかった。** 記録として残っている反対の根拠は、ADR-023(UWP、再現の記録なし)、ADR-006(PowerToys、当時の送り方)、ADR-083(per-VK confirm の欠陥と待ち時間が未測定。統一は NO-GO・「Unicode を廃止するな」)の 3 つで、いずれも「測っていない」ことが理由。撤去を支える記録は、ADR-006 自身の「IME を完全にバイパス」(漢字変換できない)、BUG-208(Tsf モードの eager unicode で変換候補ができない実害)、triage `01M13EACMQ7D2VETW75N0BTZ9C`(UWP で変換キーが効かない報告)、e2e が EDIT を IME 状態の証拠に使えないこと(`e2e-ime.yml:517`)。

---

## 確認に使ったコマンド(抜粋、origin/develop 42e092ab)

- `git grep -n "InjectionMode::Unicode" -- '*.rs'` → 7 ファイル 15 件
- `git grep -n "send_romaji_per_key\|kana_for_romaji_static" -- '*.rs'` → `send_romaji_per_key` は定義と `send_romaji_as_unicode` からだけ
- `git grep -n "UpgradeToTsf\|learned_tsf\|learn_injection_mode_tsf" -- '*.rs'` → 生産者は `unicode_literal_observer.rs:81` だけ
- `git grep -n "last_unicode_transmit\|set_last_unicode_transmit_ms" -- '*.rs'` → 書き手は `probe_io.rs:89` だけ
- `git grep -n "gji_on_ime_on(" -- '*.rs'` → `platform.rs:1088`(receipt)、`focus_tracking.rs:204`(A1)、`message_handlers.rs:982`(G1)
- `git grep -n "force_vk\|force_tsf" -- config.toml` → 既定は `force_tsf` に WezTerm・Windows Terminal、`force_vk` はコメントのみ
- `gh pr view 765 --json state` → OPEN(PR #765 のモデル検証の結論は本文から引用。マージ前なので数値は未確認)
- `grep -l "unicode-obs\]" tools/e2e/ime_key_matrix/results/*/awase-full.log | wc -l` → 0
- 同梱の配列とかな表の照合: `layout/*.yab` の引用符の無い値を NFKC で正規化し、`src/kana_table.rs` の `("…", '…')` 118 項目に無いものを数えた(Python の 1 回限りのスクリプト、6 ファイルとも 0 件)
- CI の artifact: run 39592330739・39185861786・38665255261・38659208245(§U.3 の表)

## 追記(2026-10-21): Unicode 注入されたかなも composition になる

`sc-preedit-*` の実測で、Unicode 注入されたかなは IME に入力として受け取られ composition になった(GJI・MS-IME・RichEdit・tsf)。撤去しても漢字変換の可否は変わらない。詳細と決定は ADR-276。
