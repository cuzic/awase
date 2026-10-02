---
title: 受動化後も残る能動的な書き込みの棚卸し（フォーカス変更時強制OFF・drift correction・conv軸）と回復手段の喪失
status: 一部実施済み（フォーカス変更時強制OFF・ROMAN補完系の撤去済み。T4 実機A/Bと能動書き込み増加方向の再設計が未了）。2026-09-29 に B5 として現状を同期
created: 2026-09-24
updated: 2026-09-29
related_adr: ["ADR-191", "ADR-090", "ADR-193", "ADR-098", "ADR-121", "ADR-199", "ADR-202"]
source_review: 俯瞰レビュー（2026-09-24）の B-5（起動時強制ON以外）/ C-2
---

# 残存する能動書き込みの棚卸し（俯瞰レビュー B-5 後半 / C-2）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。起動時の `desired_open=true` は [05](review-2026-09-24-05-startup-desired-open-forced-on.md)。
**2026-10-02 追記（B5 追補）**: 下の「## 2026-10-02 時点の現状（B5 追補: ADR-212/213/208）」が最新（その下の 2026-09-29 の節はその前の状態）。
**2026-09-29 追記（B5 同期）**: 下の「## 2026-09-29 時点の現状（B5 同期）」。それより下の各節は 2026-09-24 時点（基準 `5877f982`）の記述で、
撤去・再設計の決定により古くなった箇所には各所に「→ 2026-09-29」の注記を付けた。行番号は、`origin/develop`（`2c4d48b2`）で再確認したものにだけ新しい番号を書いた。
元の裏取り基準は `5877f982`（origin/develop）。`cbae84ff` 以降の差分（PR #293〜#296）で `crates/awase-windows/src` に入った変更は `tuning.rs`（`KEY_EFFECT_SETTLE_MS`）だけで、`lints/` は変わっていない。本文の行番号は `5877f982` で再確認した。
`.claude/rules/ime-belief-architecture.md`・`fix-requires-evidence.md`（IME actuation 合流点・warmup・focus 遷移・conv mode の各ファミリー）の対象領域。

旧称「ADR-178撤去プロジェクト領域A」（force-on / reassert の撤去）の「ADR-178」は、現 ADR-179 の旧番号（`docs/adr/index.md:186`「元178番…179へ採番し直し」）。現在の `docs/adr/178-*.md` は MSI アンインストールの別 ADR。ADR-179 本文には領域A の記述が無く（撤去の決定はどの ADR にも書かれていない）、[10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) の A-5 が ADR-179 に「領域A・C の撤去」節を追記する。本文書では以下「ADR-179（旧178）領域A」と書き、ADR-178 は関連 ADR に含めない。

## 現状（裏取り済み）

### 開閉軸の能動書き込み（ADR-191 決定5 指標3 = `set_ime_open_ordered` の呼び出し2箇所と一致）

| 経路 | 場所 | 条件・由来 |
|---|---|---|
| フォーカス変更時の強制OFF（**→ 2026-09-29: 撤去済み**、PR #313・`f2a875cd`、2026-09-25） | 旧 `runtime/ime_refresh.rs:599-609`。今は `ime_refresh.rs:589-593` に撤去を記したコメントだけが残る | 旧条件は `if !applied_ime_on && !new_profile_is_tsf_native`（非TsfNative だけ）。撤去根拠は CI 実測で撤去前後に差が出なかったこと（`docs/adr/191-calibration-experiments.md` A/B-1）。以下の T2・T3 参照 |
| drift correction | `ir_apply_drift_correction`（`ime_refresh.rs:658`）。`set_ime_open_ordered` が `:966`、`apply_ime_open_with_belief(order, None, belief)` が `:978-979`（2026-09-29 に `2c4d48b2` で再確認。呼び出し元は `ir_stage_notify`〈`:278`〉のみ） | 判定は `state/platform_state.rs` の `check_drift_correction`。BUG-020（TsfNative 救済）/ BUG-043（16回連続送信） |
| GJI reinit（打鍵時の事後回復。`set_ime_open_ordered` の外） | `output/probe_io.rs:186`（`send_chrome_gji_reinit_and_poll`。`:212-217` が VK_IME_OFF→ON の SendInput。give-up 時の予約は BUG-168/ADR-200 で否定的証拠が累計2回そろったときだけに限定済み） | GJI × TsfNative で打鍵時に literal を2回検出→give-up した後。**GJI 限定**（MS-IME に同等の経路は無い）。RichEdit 入力先の tsf × GJI では 30/30 で開け直したが、**実 Chrome × GJI では 0/10**（2026-09-29 追補）。この表は `set_ime_open_ordered` の2箇所（指標3）に対応するので、指標3には数えず参考行として載せる。**指標3の実数は、強制OFF撤去後は drift correction の1箇所**（`set_ime_open_ordered` の呼び出しは `ime_refresh.rs:966` のみ。`grep -rn "set_ime_open_ordered(" crates/awase-windows/src` で確認） |

起動時の強制ON（`desired_open=true` 初期値）は drift correction の経路で書かれる。これは [05](review-2026-09-24-05-startup-desired-open-forced-on.md) が扱い、ここでは別の行として数えない。

### フォーカス変更時の強制OFF（ADR-191 P1 の調査結果）

調査ブランチ `feat/adr191-p1-focus-forced-off-investigation`（先端 `49638d07`、2026-09-21）の状態を確認した:
- ローカルにだけあり、origin に push されていない（`git ls-remote origin` に該当なし）。worktree `/home/cuzic/rust-nicola-worktrees/adr191-p1` にチェックアウトされている。
- merge-base は `116eebdc`（2026-09-21、PR #233 のマージ）で、develop（`5877f982`）より471コミット遅れている。先行している5コミットの差分は docs の2ファイルだけ: `docs/teardown-verification-guide.md`（+207）、`docs/ime-passive-model-expected-results.md`（+229）（`git diff --stat 5877f982...<branch>` の三点 diff で確認）。二点 diff（`git diff 5877f982 <branch>`）では 213 ファイル・−37,517 行と出るが、これはブランチが古いだけで、ブランチ側の変更ではない。
- ガイド本文は `116eebdc` 時点のコードを見て書かれている。例えば §7.1 の「`PlatformRuntime::set_ime_open`」は、`5877f982` では `set_ime_open_ordered`（`platform.rs:1683`）が ADR-090 A-2 の授権チェック（`order.into_actuation().is_none()` なら書かずに `false`）を経てから呼ぶ。IMM 専用という結論は変わらない。

同ガイド §7.1 の要点（コードと突き合わせて確認済み）:
- 書き込みは `set_ime_open_ordered` → `PlatformRuntime::set_ime_open`（IMM32 専用）で行う。授権が下りなければ書かない。**効くのは ImmCross のアプリだけ**で、TsfNative には効かない（上表の条件とも一致）。
- 条件の `applied_ime_on` は、同じ関数の直前で非TsfNative のとき `record_confirmed(effective_open)` として書いた**前の窓の belief** で決まる。新しい窓の実IMEは読んでいない。観測していない値で書いているので、ADR-191 決定1に反する。
- 専用のテストも known-bugs も無い。裏付けは ADR-090 のインベントリ表の1行だけで、導入の意図は `git log -S` でも辿れない。
- 撤去は約10行で試作できるが、確認手段が無い（同ガイド §6 の表で L1/L2・L3 とも「無し」）。撤去すると、belief=OFF のまま新しい窓の実IMEが ON の場合に「Engine OFF なのに IME ON」のずれが観測で上書きされるまで残る。その持続時間は**未検証**。

### drift correction（ADR-191 P2 の前提）

同ガイド §7.2 の要点:
- **判定側**は既存テストで守られている: `check_drift_correction` の単体テストと `crates/awase-windows/tests/drift_correction_replay.rs`（BUG-043 の16回連続送信を有界化）。どちらも Linux で走る。
- **書く側**（TsfNative で VK を実送信して回復するか）は純粋テストでは測れない。実機か、ADR-193 の RichEdit スーパークラス（`RICHEDIT50W` を `Chrome_RenderWidgetHostHWND` の名前で登録、awase は TsfNative として扱う）を使った CI E2E でしか測れない。ADR-193 の入力先の `e2e-ime.yml` への配線は未着手（同ガイド §8-2）。
- 明示意図の回復シナリオ（ユーザーが OFF にした直後に IME が ON へ戻る）を ATOK/GJI で作れるかは未確認。作れなければ、drift correction は撤去せず残す判断の根拠になる（同ガイド §8-3）。

### フォーカス変更時の eager warmup

（→ 2026-09-29: 強制OFF は撤去済みなので「直前」の相手は無い。呼び出しは `ime_refresh.rs:588` に移動。Ctrl↑ の eager warmup は BUG-174〈PR #358〉で撤去済みで、これとは別。確定キー warmup の全面削除は PR #360 が担うが未マージ）
`ime_refresh.rs`（旧 `:594`）の `self.platform.send_eager_warmup(warmup_ime_on)` がフォーカス変更ごとに ON 方向の warmup を送る。コメントに「トレイで半角英数へ切り替えた直後のフォーカス復帰で、一度だけひらがなへ戻る（既知の制限）」とあり、conv 軸にも作用する。ADR-191 決定1は warmup を既存の例外として撤去対象外にしているので、棚卸し表には「例外として維持」の行として載せる。

### conv 軸（変換モード）の書き込み

ADR-191 の線引き「awase が書いてよいのは開閉だけに作用するキー」（frontmatter summary の (2)。本文では決定1の線引き。本文の「決定2」は BUG-151 の最小修正で別物）に照らして棚卸しする対象。以下は**現時点で見つかった経路で、下限**（件数は棚卸しの成果物として確定させる）。

IMM 経由の書き込みは、最終的に `ime.rs:371` の `modify_conv_mode`（`ime.rs:393` で `actuate_ime_control(…, ActuateCmd::SetConversionMode(…))`）に集まる。`modify_conv_mode` を直接呼ぶのは `ime.rs:709`（`set_ime_romaji_mode_for_hwnd`）、`:750`（`set_ime_hiragana_mode_cross_process`）、`:1771`（`set_ime_mode_for_target`）の3箇所。そこから上へ辿った呼び出し元:

| 呼び出し | 場所 | 備考 |
|---|---|---|
| `set_ime_conv_for_target` | `runtime/key_pipeline.rs:1847`、`:2574`、`:3170` | |
| 同上 | `output/conv_actuation.rs:176` | |
| 同上 | `tsf/warmup/cold_warmup.rs:94` | warmup の一部。指標5の「warmup」に含まれると読める |
| `set_ime_open_then_conv_for_target` | `runtime/open_chain.rs:312` | 開の直後に conv を書く（`ConvAfterOpen::Write`）。`runtime/executor.rs:926` の `decide_dispatch_conv_after_open` が決める |
| `set_ime_romaji_mode_for_target_blocking` | `ime_controller.rs:439` | 同期経路の ROMAN ビット補完。呼び出し元は `architecture_guard.rs` の `sync_romaji_write_goes_through_a_captured_target` が固定済み |
| `set_ime_mode_for_target` | `runtime/message_handlers.rs:1291` | トレイからのリセット |
| `set_ime_hiragana_mode_cross_process_async` | `runtime/mod.rs:1794` | パニック時のリセット |

- トレイリセットの `set_ime_mode_for_target(hwnd, true, …)`（`message_handlers.rs:1291`）は、conv だけでなく先に `set_ime_open_for_target` で開閉も書く（`ime.rs:1757-1763`）。パニックリセットも `runtime/mod.rs:1791-1792` で `set_ime_open_cross_process_async(false/true)` を書く。どちらも `set_ime_open_ordered` の外なので、開閉軸の表（指標3）には入らない。T5 で「開閉軸・正当な例外」の行として載せる。
- `ime.rs:1742` の `set_ime_mode_for_target(` は独立した経路ではない。`pub unsafe fn set_ime_mode`（`ime.rs:1733`）の本体が委譲しているだけで、`set_ime_mode` の呼び出し元は `crates/` と `src/` にゼロ（`grep -rn "set_ime_mode(" crates src` で定義行のみ）。棚卸し表からは外し、デッドコードとして [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) の B-6（撤去後に使われなくなったコード）へ回す。
- **モードキー注入による conv 変更**も別系統としてある: `kp_restore_kana_from_half_width`（`key_pipeline.rs:1463`、`:1506`、`:1761`、`:2301`、`ime_refresh.rs:302`）や shift-conv-guard の `VK_DBE_HIRAGANA` 注入など。決定1の線引きは「キー」について述べているので、IMM 経由の書き込みに加えてこれを対象に含めるかを、棚卸しの最初に決める。

ADR-191 決定5の指標5（IME へ書く振る舞いの数）の列挙は「固定の例外・表駆動の追加・opt-in の単独タップ・`keys.ime_on/off/toggle`・EngineDecision・warmup」。conv 軸の書き込みは、`cold_warmup.rs:94` を除いて対応する項目が無い。

lint（`lints/actuation_call_guard/src/lib.rs:98-101`）は `actuate_ime_control` の許可呼び出し元として `set_ime_open_for_target` と `modify_conv_mode` を持つだけで、`set_ime_conv_for_target` / `set_ime_mode_for_target` / `set_ime_romaji_mode_for_hwnd` を呼ぶ側は数えていない（確認済み）。

## 2026-10-02 時点の現状（B5 追補: ADR-212/213/208）

`origin/develop`（`7119e808`）時点。**この節が最新で、下の 2026-09-29 の節・それ以前の記述と食い違う場合はこちらが正**。
ADR-212（予防的・補正的な IME 書き込みの段階撤去）・ADR-213（ActivationSync の撤去）・ADR-208（明示キーの固着ゼロの保証）の結果を反映する。
実測は [docs/experiments.md](../experiments.md) エントリ 30、棚卸しの元は [actuation-inventory-2026-09-30.md](actuation-inventory-2026-09-30.md)。

| 経路 | 現状（2026-10-02） | 根拠・備考 |
|---|---|---|
| 確定キー（Enter）reinject の eager warmup | **撤去済み**（ADR-212 P0、PR #398） | 実機 24→0 回、入力の欠落・リテラル化は増えず。測った範囲は WT+GJI の MS-IME プリセット・IME ON・Engine OFF だけ（NICOLA ON・MS-IME 本体・Chrome の実機は未測定） |
| 記号 VK フォールバックの `send_eager_tsf_warmup(off)`（デッドコード） | **撤去済み**（ADR-212 P1、PR #399） | 挙動は変わらない |
| Chrome/TSF give-up 後の reinit（`VK_IME_OFF`→`VK_IME_ON`） | **撤去済み**（ADR-212 P3、PR #402）。BS/ESC の回収だけに縮退 | 実 Chrome×GJI で 0/10 と効かず、BUG-168 の副作用もあった。自前 RichEdit 窓（tsf×GJI、ADR-193）では撤去前 30/30 効いていた（撤去後の自己回復の変化は ADR-212 の P3 の項を参照） |
| フォーカス変更時・SetOpen(true) 随伴の eager warmup | **撤去済み**（ADR-212 P4、PR #401）。Ctrl↑ の eager warmup は BUG-173/174 で撤去済み | `InjectionMode::Tsf`（WezTerm 等）+ GJI だけに効いていた経路 |
| Unicode long-cold の reinit・`VK_IME_ON`+`VK_A`+BS | **撤去済み**（ADR-212 P5、PR #402・#403） | Unicode 注入は GJI の確認を迂回するため、判断は P2 の後の状態を前提に行った |
| drift correction の (b) 古い desired の補正・(c) HWND キャッシュ復元 | **撤去済み**（ADR-212 P6 の (b)(c)、PR #404）。(a) 明示意図後の補正は ADR-212 決定2 で別扱い | 実 Chrome では観測が乗らず判断に届かない（BUG-172、ADR-205 の監視窓が別途追随） |
| **ActivationSync 起源の `SetOpen`**（Engine の ON/OFF 遷移が自動で IME の開閉を書く経路） | **撤去済み**（ADR-213 P2a〜P2d-2、PR #408・#411・#412・#413 ほか） | 全面停止は CI で退行（sc-hz/sc-kanji）、gate による縮小は無効だったため、shadow toggle の OFF→ON を明示 actuation（`DecisionSite::ShadowToggleOn`）にしてから止めた。新スレッド=閉は belief 側の改善（ADR-212 P2 の後続、`4f28dd86`） |
| 明示キーの書き込み（`keys.ime_on/off`、Ctrl+変換/無変換、役割由来の Toggle） | **存続**。ただし ADR-208 L0〜L3 で省略条件を緩め、押下 ID（`PressId`）で 1 押下 1 回に限定した | 絶対指定は 1 回、トグルは 2 回以内で一致する保証（INV-L2）。CI の drift × キー行列（`sc-keymatrix-*`、16 構成）で確認。TsfNative×WT×GJI（L3'）・InputRelay（L4）・S-3/S-4（L5）は v2 では既知の制限 |
| shadow no-op（既に一致）で物理が Suppress される窓 | **書く**（ADR-208 L3a、D4 固定点）。Allow・リピート・TsfNative は書かない | 全列挙で S-3 25,920→0。BUG-113 型の二重送信は押下 ID の予約で防ぐ |
| `keys.engine_on_ime_key`/`engine_off_ime_key` | **撤去済み**（ADR-207、`279268f3`） | 旧 config に残っていればトレイで通知 |
| `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` | **撤去済み**（ADR-206、`669b784e`・`b28b7ca5`）。役割があれば Passthrough のときだけ絶対指定を 1 回送る | 「@」（WT+GJI）が出るかの実機確認は未実施 |
| MS-IME 本体の無変換/変換=値 2（トグル） | **役割由来の開閉として扱う**（ADR-199 T17 Phase 4、PR #379） | 実機（MS-IME 設定 UI・入力中/変換中の各状態）は未実施 |
| フォーカス変更時の強制 OFF、force-on/reassert | 撤去済み（2026-09-25 #313、2026-09-18）。変更なし | 上の 2026-09-29 の節 |

**未解決・次の判断**: ADR-212 P7（ROMAN 補完・conv 軸、MS-IME 本体の実機が条件）は別に判断する。MS-IME 本体 × 実 Chrome の `VK_IME_OFF` が効かない件は ADR-208 決定4(a) の例外（E1・E2 で認定、CI の fresh 対照で ENV_EXCEPTION を確認）。
実機でのみ確認できる項目（WT×GJI の「@」、MS-IME 本体の設定 UI と各状態、実 AutoHotkey での追随）は [v2-manual-verification-guide-2026-09-29.md](v2-manual-verification-guide-2026-09-29.md) に残る。

## 2026-09-29 時点の現状（B5 同期）

`origin/develop`（`2c4d48b2`）でコミット履歴・コードを突き合わせた。**この節が最新で、下の古い記述と食い違う場合はこちらが正**。

### 開閉軸の能動書き込みの一覧（現状列）

| 経路 | 現状（2026-09-29） | 根拠・備考 |
|---|---|---|
| フォーカス変更時の強制OFF | **撤去済み**（2026-09-25、PR #313 `f2a875cd`） | CI 実測で撤去前後に差なし。実機未検証は限界として ADR-191 に記録済み（T3） |
| 起動時の強制ON（`desired_open=true`） | 修正が develop に入った（BUG-163。詳細は [05](review-2026-09-24-05-startup-desired-open-forced-on.md)） | 本表の管轄外 |
| force-on / reassert | **撤去済み**（`621bf93c`/`f83084b3`、2026-09-18）。TsfNative の ON 方向救済は drift correction のみ、GJI は加えて reinit | reassert 側の撤去前対照は復元不能で未実施（BUG-172） |
| drift correction | **存続**（`ime_refresh.rs:658`、`set_ime_open_ordered` は `:966` の1箇所）。ただし **実 Chrome では判断に届かない** | 下の「BUG-172 の測定結果」。PR #360（未マージ）は `ConvOpenInference` 由来の drift 発火を撤去する内容 |
| GJI reinit（打鍵時） | 存続（`probe_io.rs:186`）。give-up からの予約は BUG-168/ADR-200 で「否定的証拠が累計2回」に限定済み | 実 Chrome × GJI では開け直せなかった（下） |
| フォーカス変更時の eager warmup | 存続（`ime_refresh.rs:588`）。Ctrl↑ の eager warmup は撤去済み（BUG-174、PR #358） | ADR-191 決定1で例外として維持 |
| GJI の 0x19（Alt+半角/全角）の役割由来 Toggle | **既定設定で初めて能動経路が動く**（PR #367、`d1456a4b`） | 下の「ime_toggle 既定を空にした影響」 |
| `keys.ime_on`/`ime_off`、役割由来の Toggle（0xF3/0xF4・F13〜F24・無変換/変換の単独タップ） | 存続（ADR-199 決定15。`ime_on`/`ime_off` の既定は残す） | 08 の結論 |
| `keys.engine_on_ime_key`/`engine_off_ime_key`（エンジン ON/OFF 時の IME モードキー送信） | **撤去決定**（所有者決定 2026-09-29。実装は `feat/v2-keys-cleanup`） | 配線は `app/bootstrap.rs:722-728` → `platform.rs:29-31,109-120`。実装後に本表から削除。actuation 合流点を減らす方向（複雑性予算では削除側）。詳細は [v2-a4 棚卸し](v2-a4-config-cleanup-inventory-2026-09-29.md) |
| `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` | **再設計決定**（所有者決定 2026-09-29。実装は `feat/v2-solo-tap-redesign`、ADR・敵対レビューが先）。**能動書き込みが増える方向** | 下の「撤去決定と再設計決定」。現行は `key_pipeline.rs:974` の `explicit_ime_action_target`（`SuppressOnly` は書かない） |

### ADR-178（現 ADR-179）領域A撤去後の状態

- 領域A（force-on・reassert）の撤去は `f83084b3`/`621bf93c`（2026-09-18）。その後に強制OFF（2026-09-25、#313）と conv 軸の経路9（焦点プローブの ROMAN 修正、2026-09-26、PR #329）も撤去された。
  conv 軸の経路の最新の分類・状態は [conv-write-paths-inventory.md](conv-write-paths-inventory.md) が正で、本文書の conv 軸の表（`5877f982` 時点の行番号）は**古い**（`ime.rs`・`key_pipeline.rs` の行番号がずれている。経路9は撤去済み）。
- 撤去後の ON 回復の CI 観測（`cal-driftrec-*`）と結果は、末尾の「CI での代替観測」「追補」の節にある。要点: 4構成（tsf/edit × GJI/MS-IME）すべてで awase は外部 close を観測せず（observed=0）、drift correction は一度も判断に届いていない。
  「drift correction は ON へ戻さない」とは言えず、「観測の経路に乗っていない」が事実。
- 撤去前ビルドとの対照は force-ON のみ復元して実施済み。reassert は後続変更と衝突して復元不能（未実施）。

### BUG-172 の測定結果（PR #365 マージ済み、その後の測定は未マージ）

[BUG-172](../known-bugs/BUG-172.md) の記録に沿う。

- **PR #365（マージ済み）**: 別窓方式のフォーカス変更を挟んだ実 Chrome 測定（`cal-driftrec-chrome-refocus-*`、run 36537797446）。実 Chrome × MS-IME 0/10・× GJI 0/10 で回復せず、
  **observed（Chrome の開閉を ImeModel が観測した回数）= 0**。Chrome 系は `profile=Imm32Unavailable` で開閉の観測を捨て、belief が古い ON のまま残る。撤去の有無（force-ON のみ復元）でも差なし。
- **`origin/docs/bug172-realistic-close-measurement`（f7a96923、未マージ）**: 実運用に近い外部 IME OFF（他プロセスの `SendInput` 注入、メモ帳経由）の実 Chrome 測定（run 36540419485）。
  - **GJI × 実 Chrome は外部注入の 半角/全角（0xF3）・VK_IME_OFF（0x1A）で 10/10 再現**。IME は閉じたまま、打鍵は `kiu`（Engine は ON のまま）。observed=0。
    awase は注入キーを hook で見ているが `[shadow-toggle] injected ... ユーザー意図に昇格させない (BUG-14)` で belief 追従を may_change_ime の refresh に委譲し、その refresh が Chrome では読めない。
  - awase の目印付き（物理キー相当）の 0xF3 は awase が Engine も OFF にするので症状ではない。**物理キー押下（awase 経由）では起きない**。
  - MS-IME × 実 Chrome は再現できず（外部注入の 0xF3/0x1A を MS-IME が効かせない、理由は未特定）。メモ帳経由は 0/10（IME 開閉は窓/スレッドごと）。
  - 影響: **C-2 の「TsfNative の ON 回復は drift correction だけ」は、実 Chrome では成立しない**（判断に届かない）。ゲート修正（msime-ready に開閉を要求）は実 Chrome の症状を直さないため見送り。
    次の一手（未着手）は、打鍵直前だけの読み取り専用照合、または `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` の通知購読（いずれも書き込みではなく belief の観測を増やす方向。判断は belief 更新の reduce 経由）。
- 限界: 1台の CI 実機、各10試行。ユーザーが遭遇するのは AutoHotkey 等の注入やモード切替アイコン操作だが、頻度は未確認。

### 撤去決定と再設計決定（所有者決定 2026-09-29、v2 A4）

出典: [v2-a4 棚卸し](v2-a4-config-cleanup-inventory-2026-09-29.md)（PR #368）。

- **`engine_on_ime_key`/`engine_off_ime_key` は撤去**。「受動が原則」に反する能動送信の残骸（`platform.rs` から SendInput）。実装は `feat/v2-keys-cleanup`。撤去で能動書き込みが1経路減る。
- **`keys.ime_detect.{on,off}` の既定は空にする**。これは belief の追随（受動）の既定を減らすもので、能動書き込みではない。`is_japanese_ime()` が偽の間の 0x16/0x1A の追随が一瞬効かなくなりうる（推論、実機未確認）。
- **`*_solo_tap_ime_action` は再設計**。Suppress/Passthrough の設定に従い、IME 側がトグルなら生キーを抑止して awase が **belief に従って ON/OFF を明示 inject** する。現行の `SuppressOnly`（書かない）に対して、
  **能動書き込みが増える**。この文書の観点（棚卸し・回復手段の喪失）で新たに生じる問題:
  - belief が実 IME とずれていると、ON にしたいのに OFF を送る等の逆動作になる。特に TsfNative（実 Chrome では belief が観測できず古い ON のまま残る〈BUG-172〉）と重なる。
    inject の前提となる belief の鮮度・確からしさを設計に含める（BUG-113 の `shadow_on` を `Option<bool>` のまま扱う教訓）。
  - IME 側が生キーでもトグルする場合、生キー抑止と inject を対にしないと二重トグルになる（BUG-46 型）。`transport.rs::PhysicalKeyDisposition::plan` と食い違わないこと。
  - inject は `apply_ime_open_with_view` 系の合流点を通す新しい呼び出し元になりうる。`lints/actuation_call_guard` の許可リストと `architecture_guard` の件数ガードへの影響を洗い出す（`complexity-budget.md` は未発効だが、`engine_on/off_ime_key` の撤去と対にして差し引きを説明できると望ましい）。
  - 指標3（`set_ime_open_ordered` の呼び出し）が1箇所のままか、増えるかは設計次第。増やす場合は ADR-191 決定5 の指標に反映する。

### `ime_toggle` 既定を空にした影響（PR #367）

- `keys.ime_toggle` の既定が `VK_KANJI` から空になった（`d1456a4b`、所有者決定 2026-09-29、ADR-199 決定15・ADR-202 T16-5）。
- 従来は既定の無修飾 `VK_KANJI` が `Engine::has_bare_ime_combo(0x19)` を真にし、GJI の 0x19 役割判定（`derive_key_shadow_action`）を `explicit_overlap` で常に無効化していた。
  つまり**既定設定の GJI では Alt+半角/全角が常に受動**（belief は実 IME の開閉の観測に追随）だった。
- 既定を空にしたので、**既定設定の GJI で ADR-202 の能動 `Derive` 経路（0x19 の行がトグルなら役割由来の `Toggle`）が初めて働く**。開閉軸の能動書き込み（役割由来の Toggle、上の表）に、GJI の 0x19 が既定で加わる。
- 未検証: 能動 `Derive` 経路の実機確認は未実施（ADR-202 T16-5。GJI の既定 `[keys]` での `sc-kanji-role-toggle`/`sc-kanji-role-nontoggle` の e2e）。既存 config の明示 `ime_toggle = ["VK_KANJI"]`（旧既定を GUI が書き出したもの）は尊重するので、その利用者は受動のまま。
- 変わらないもの: GJI 以外（MS-IME 本体・ATOK・未検出）の 0x19 は `hook.rs` の静的 `Toggle`（`KeepStatic`）のまま。互換モードの非対称は ADR-202 T16-7。物理の Alt+半角/全角は元から静的 `Toggle` が担っていた。

### 今後の作業（この文書の未了）

- T4 の実機 A/B（既定チョードでの外部 close 回復）。ただし BUG-172 の測定で、実 Chrome では drift correction が判断に届かないことが分かったため、A/B の意味は「届く条件を作る」ことから先に変わった。
- 上の再設計（solo-tap）が入ったら、開閉軸の表と指標3を更新する。
- conv 軸の表を [conv-write-paths-inventory.md](conv-write-paths-inventory.md) に一本化するか、本文書の表を削る（重複と行番号の陳腐化を避ける。判断は所有者）。

## C-2: 回復手段の喪失と非対称

- force-on と reassert は撤去済み（`f83084b3` / `621bf93c`、`5877f982` に含まれる。2026-09-18。旧称「ADR-178 領域A」＝ADR-179〈旧178〉領域A）。TsfNative の ON 方向の救済は drift correction だけ（**GJI については打鍵時の GJI reinit も ON 方向に働く**が、RichEdit 入力先でのみで実 Chrome では回復しなかった。下の「追補 2026-09-29」参照）。記憶メモによれば、撤去時点で「drift 単独で代替できるか」の実機 A/B は未実施で、その後の実施記録もリポジトリ内で見つからない（**未確認**）。
- 一方で、状態を押し付ける書き込みは残る: 起動時の強制ON（[05](review-2026-09-24-05-startup-desired-open-forced-on.md)）と、ImmCross へのフォーカス変更時の強制OFF。「救済のための書き込みは消したのに、押し付ける書き込みは残っている」形。
- 注意（→ 2026-09-29: 強制OFF は撤去済みなので、「押し付ける書き込み」は起動時の強制ON〈05〉だけになった）: 強制OFFは TsfNative では発火しないので、TsfNative の「回復手段の喪失」は drift correction だけの問題。強制OFFの撤去で確かめるべきなのは ImmCross 側のずれの持続時間。

## タスク

- [x] **T1 P1 調査結果の取り込み**（実施済み。`docs/teardown-verification-guide.md` と `docs/ime-passive-model-expected-results.md` が develop に存在する。以下は当時の手順）:
  - (a) worktree `adr191-p1` を使っているセッションを確認する（`worktree-per-session`。他セッションの作業中ブランチを勝手にマージしない）。
  - (b) `docs/teardown-verification-guide.md` と `docs/ime-passive-model-expected-results.md` を develop に入れる（docs のみ、`main-develop-branch-flow` に従い develop へ直接マージ可）。手段は先行5コミットの cherry-pick か2ファイルのチェックアウト（ブランチは471コミット遅れているので、ブランチごとのマージや二点 diff での確認はしない）。取り込むとき、ガイド中のコード参照（関数名・行番号）を `5877f982` 以降の develop で再確認して直す（例: §7.1 の `set_ime_open` → `set_ime_open_ordered`）。
  - (c) ADR-191 決定5の P1 行から `teardown-verification-guide.md` §7.1 へリンクする。
- [x] **T2 強制OFFの確認手段を先に作る**（同ガイド §8-1）: `ime_key_matrix_spike` に2窓のフォーカス切替モードを足し、片方を IME ON にしてから belief OFF のままもう片方へ移り、移動後 +100/+400/+1500ms で実IMEと Engine の一致を記録する。撤去前のビルドでは、`focus_change_enforce_off` が実際に書いたか（`set_ime_open_ordered` の戻り値 `sent`、`ime_refresh.rs:603` 以降のログ）を各試行で記録する。授権が下りずに書いていない試行は、撤去前後の差がゼロでも「撤去しても影響なし」の証拠にならないため分けて数える。対象は ImmCross（CI の GJI 構成は Win32 `Edit` が入力先なのでそのまま測れる）。 → 2026-09-25 CIで4通り試行し、いずれも差が出ず判定不能（記録: `docs/adr/191-calibration-experiments.md`）。
- [x] **T3 強制OFFの撤去要否を ADR-191 で決める**: T2 で撤去前後の「不一致が続く時間」を比べる。撤去するなら撤去コミットに T2 のモードを CI の構成として含める。 → 2026-09-25 ユーザー判断で「CI結果（現developでは実質no-op、`sent=false`はwarrant拒否）を根拠に撤去」と決定。実機未検証は限界として記録。
- [ ] **T4 drift correction（P2 の前提）**（→ 2026-09-29: BUG-172 の測定で、実 Chrome では drift correction が判断に届かない〈observed=0〉と分かった。上の B5 節参照）:
  - 判定側: 既存テスト（`check_drift_correction` の単体テスト、`drift_correction_replay.rs`）で足りる。追加は不要。
  - 書く側: ADR-193 の入力先を `e2e-ime.yml` に配線し（同ガイド §8-2、GJI 有効化と `awase=true/false` の対照が要る）、明示意図の回復シナリオ（§8-3）を作る。
    → **2026-09-27 CI配線完了**（`cal-drift-tsf-{gji-atok,msime-native}`、`typing_stress --mode=drift`、
    `feat/adr191-t4-drift-e2e-wiring`、developへは未マージ）。ただし `keys.ime_off` を単一キー(`VK_NONCONVERT`)へ
    上書きした代替検証(既定の`Ctrl+無変換`チョードはSendInputでは駆動できない)。結果:60試行すべて復帰なし・
    drift correction発火0件(`docs/adr/191-calibration-experiments.md` A/B-3)。`awase=true/false`の対照はまだ無い。
  - **終了条件**: シナリオが ATOK/GJI で作れなければ、drift correction は撤去せず残すと ADR-191 に記録して終える。作れたら、TsfNative の ON 回復を drift 単独で代替できるかを A/B する。
    → CIでの単一キーOFFでは作れなかったが、これは既定チョードでの結論ではない(限界としてA/B-3に明記)。
    終了条件を満たすには、実機での既定チョード(`Ctrl+無変換`)によるA/B(下記「実機A/B手順」)がなお必要。
  - A/B は [05](review-2026-09-24-05-startup-desired-open-forced-on.md) の修正前か後か、どちらのビルドで行うかを固定する（起動時の強制ONを直すと、起動直後の drift 発火が減るため）。
- [x] **T5 conv 軸の棚卸し**（実施済み: [conv-write-paths-inventory.md](conv-write-paths-inventory.md)。conv 軸11経路: 撤去候補A 4・例外B 6・warmup C 1）: 起点を「3関数の grep」ではなく「`modify_conv_mode` と `ActuateCmd::SetConversionMode` に到達する全経路（呼び出し元を逆に辿る）＋ conv を変えるモードキー注入」にする。上表は下限。各経路を次の3つに分類する: 決定1の線引き違反（撤去候補）／正当な例外（パニック・トレイのリセット）／撤去対象外の warmup。フォーカス変更時の eager warmup（`ime_refresh.rs:594`）も「例外として維持」の行で載せる。パニック・トレイのリセットが書く開閉（`runtime/mod.rs:1791-1792`、`message_handlers.rs:1291` 経由の `set_ime_open_for_target`）も「開閉軸・正当な例外」の行で載せる。[08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md) の固定セットの結論を分類に反映する。表は `docs/tasks/actuation-confluence-inventory.md` と同じ粒度（1経路1行、根拠の行番号付き）にする。ファイル書式や分類軸（あちらは統合候補/構造上必要/ロジック共有候補）は合わせない。結果を ADR-191 決定5の指標5に加算する。
- [x] **T6 呼び出し元の固定**（実施済み: `architecture_guard.rs::conv_write_call_sites_are_fixed_to_the_inventory`。`modify_conv_mode(` は `ime.rs` の3入口、`set_ime_conv_for_target(` は5か所に固定）: 棚卸しで経路が確定するまで lint（`RESTRICTED_CALLS`）には追加しない。ADR-191 決定5は `RESTRICTED_CALLS` の行数を「補助に留める」としており、撤去を成功基準とする ADR で許可リストのエントリを増やすのは逆向き（`complexity-budget.md` は未発効だが方向は同じ）。既存ガードで既に固定されているもの: `set_ime_open_then_conv_for_target(` は `async_imm_cross_actuation_goes_through_the_single_chain_entry`（`architecture_guard.rs:2128`）が `open_chain.rs` の1件に固定、`set_ime_conv_for_target(` は `force_write_is_not_triggered_by_raw_focus_change`（`:1874`）が `gji_on_focus_change` からの呼び出しを禁止、`set_ime_romaji_mode_for_target_blocking` は `sync_romaji_write_goes_through_a_captured_target`（`:2557`）。新たに足すのは `set_ime_conv_for_target` の呼び出し元件数ガード（同じ方式）だけに絞る。lint 化は撤去が頭打ちになってから判断する。

## 受け入れ条件

- **ドキュメント（T1・T5）**: ADR-191 に棚卸し表が追記されている。中身は開閉軸2箇所（強制OFF `ime_refresh.rs:603`、drift 補正 `:934`）、eager warmup、conv 軸の全経路で、各行に分類と撤去/維持の判断がある。docs のみの段階ではコードを変えない（`git diff --stat` が `docs/` だけ）。
- **A/B 結果の記録先**: ADR-191 の補助資料 `docs/adr/191-calibration-experiments.md`（または取り込んだ `teardown-verification-guide.md` §7.1/§7.2）。撤去を取り下げた（revert した）ときだけ、`experiment-logging` 規約に従い `docs/experiments.md` にも1行足す（同ファイルは取り下げたアプローチの記録なので）。A/B の結果は [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) A-5 で作る領域A撤去の記録にも反映する。
- **強制OFFの撤去（T2・T3）**: windows-build / e2e-ime CI で、T2 のフォーカス切替モードの撤去前後の不一致時間が記録されている（ImmCross、Linux では走らない）。撤去コミットは `fix-requires-evidence` を満たす（T2 のモードを回帰の確認として残す、または known-bugs を足す）。
- **drift correction の撤去（T4）**: 判定側の既存テスト（Linux: `cargo nextest run -p awase-windows --test drift_correction_replay` と `check_drift_correction` の単体テスト）が通る。書く側は ADR-193 の入力先を使う CI E2E、または実機（Chrome / VS Code / WezTerm 等の TsfNative 環境で実タイピング）で、明示意図の回復が撤去前と同じく働くことを確認する。シナリオが作れない場合は「撤去しない」の記録で完了とする。

## 実機 A/B 手順（T4: drift correction 単独での ON 回復・未実施）

強制OFF（T2・T3）は #313 で撤去済みのため、旧 A/B-1（撤去前後比較）は不要になった。残るのは T4 の判断材料だけ。
実行者はユーザー（実機と物理キー押下が要る。SendInput 注入では物理キー状態を作れない）。
BUG-163（起動時 `desired_open=true` の強制ON）は修正が develop に入っているため、**現 develop 先端のビルド1本**で測る（修正前との比較は不要）。

**準備**: develop 先端を **push してから** `awase-build` スキル（`clipwire exec awase-build`）でビルドし、Windows 側チェックアウトのブランチとコミット（`git log -1`）が push したものか毎回確認する。
ログは `awase.log`（`tracing`）。drift の結果は `info!`（`ime_refresh.rs` の `Blacklist drift correction: apply_ime_open(...)`）。解釈する前に、押したキー・config のパス・awase の PID/コミットを記録する。

**A/B-2（対象は TsfNative）**: Chrome または VS Code の入力欄で、reassert/force-on は撤去済みで比較対象が無いので、撤去済み状態での回復可否だけを測る。
1. IME ON で日本語入力できる状態から、Ctrl+無変換（または設定中の OFF キー）で OFF にし、直後に IME が ON へ戻る/固定されるかを見る（2026-07-08 の症状）。10回。
2. ずれが作れない → 「作れないので drift correction は撤去せず残す」と ADR-191 に記録して T4 終了。
3. ずれが作れた場合: `Blacklist drift correction` ログの発火有無と、発火後に**実タイピング**で正しく ON/OFF になったかを記録する（API の成功表示だけで判断しない）。

**結果の記録先**: `docs/adr/191-calibration-experiments.md`。1試行=1行（日時・アプリ・IME・押したキー・+100/+400/+1500ms の一致・drift 発火の有無）。

## 他ファイルとの依存

- [05](review-2026-09-24-05-startup-desired-open-forced-on.md): 05 は 09 に依存しない。09 の A/B（T4）は 05 の修正の有無を前提条件として持つ（どちらのビルドで測るかを固定する）。起動時に ON を書き、ImmCross の窓へフォーカスが移ると強制OFFが OFF を書く往復は、05 と T3 の両方に関わる。
- [08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md) → 09: 08 の開閉書き込み固定セットの結論を、T5 の棚卸し表の分類に反映する。
  - **反映（2026-09-28）**: 08 の結論は ADR-199 で確定済み。開閉軸の能動書き込みは、(1) 役割が Toggle と判定されたキー（0xF3/0xF4・F13〜F24・無変換/変換の単独タップ・GJI の 0x19）、(2) awase 自身の `keys.ime_on/ime_off` 設定のキー、(3) GJI 以外の 0x19（`hook.rs` の静的 Toggle、MS-IME 本体は固定トグルと実機確認済み、互換モードの非対称は ADR-202 T16-7 で別途）の3系統に整理された。固定セットを VK で無条件に書く経路は撤去済み（`is_open_toggle_for` 撤去、`architecture_guard` が再出現を監視）。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md): A-5（領域A撤去の記録が ADR-179 に無い）が C-2 の「未確認」の原因。09 は 10 A-5 が ADR-179 に書く「領域A・C の撤去」節を参照し、09 の A/B 結果はその節へ戻す（双方向）。`set_ime_mode`（`ime.rs:1733`）のデッドコードは 10 の B-6 へ渡す。**要追随（09 の担当外）**: 10 B-6 には現在 `set_ime_mode` の行が無いので1行加える。10 `:155` の「09 の `related_adr: ADR-178`」は 09 側で既に外しているので 10 側で削る。
- [11](review-2026-09-24-11-low-priority-backlog.md): **要追随（09 の担当外）**: 11 `:57` の「conv 軸の書き込みは `ime.rs:1742` の `set_ime_mode_for_target` 呼び出しも含む（09）」は 09 の結論と逆。「`ime.rs:1742` は呼び出し元ゼロの `set_ime_mode` 内の委譲で独立経路ではない（09、デッドコードとして 10 B-6 へ）」に直す。

## 未確認点

- ADR-179（旧178）領域A撤去後に drift correction だけで TsfNative の ON 回復を代替できるかの実機 A/B の結果（リポジトリ内に記録なし）。
- 強制OFFを撤去したとき、ImmCross の新しい窓で「Engine OFF なのに IME ON」が観測で上書きされるまでの時間（P1 調査も未検証と記載）。
- 明示意図の回復シナリオを ATOK/GJI で作れるか。
- worktree `adr191-p1` を現在どのセッションが所有しているか。

## レビュー反映メモ（2026-09-24、Opus 批判的レビューへの対応）

指摘はすべて `5877f982` のコード・git で裏取りしてから反映した。反映しなかった指摘は無い。
- 1（強制OFFは TsfNative で発火しない）: `ime_refresh.rs:599` の条件で確認。T2/T3 と受け入れ条件を ImmCross 向けに直し、TsfNative の実機確認は T4 へ移した。
- 2（conv 7箇所の数え方）: `set_ime_mode` の呼び出し元ゼロ、`open_chain.rs:312`・`ime_controller.rs:439`・`modify_conv_mode` の3呼び出し元・`kp_restore_kana_from_half_width` の注入を確認。件数を確定値として扱うのをやめ、下限の表にした。
- 3（ADR-178 は別件）: `docs/adr/178-msi-uninstall-preserve-userdata.md` を確認。`related_adr` から外し、ADR-090（`ime_refresh.rs:600` のコメント）と ADR-193 を加えた。BUG-020/BUG-043 は本文の表で参照した（シリーズの frontmatter に BUG 用のキーが無いため）。
- 4〜6: ブランチ状態（`49638d07`、origin に無い、docs 2ファイル）とガイド §7.1/§7.2/§8 の内容を `git show` で確認して取り込んだ。
- 7〜9: ADR-191 決定5の指標3（2箇所）・指標5の列挙を本文で確認し、「3箇所」を2箇所の列挙に、「指標5に入っていない」を「warmup の1件を除き」に直した。eager warmup（`:594`）を追加した。
- 10: `lints/actuation_call_guard/src/lib.rs:98-101` で確認し、未確認点から外した。
- 11: 代案（`architecture_guard.rs` の件数ガード）を採用した。同方式の既存ガード `sync_romaji_write_goes_through_a_captured_target` があることも確認した。
- 12・13: 記録先を ADR-191 補助資料に変え、「cargo check が通る」を「`git diff --stat` が `docs/` だけ」に置き換えた。
- 14〜16: 05 の依存節（「09 の A/B 計画は 05 の修正の有無を前提条件として持つ」）と向きを合わせ、10 との双方向の依存を書いた。行番号を `:602-603` と併記し、分類表は書式ではなく粒度を合わせると書き分けた。

## レビュー反映メモ（2026-09-24、再確認レビューへの対応）

指摘 A〜G はすべて `5877f982` のコード・git で裏取りし、反映した。反映しなかった指摘は無い。
- A: merge-base `116eebdc`、develop より471遅れ・先行5、三点 diff は docs 2ファイル +436、二点 diff は 213 ファイル −37,517 行を確認。`platform.rs:1683` の `set_ime_open_ordered` と授権チェックも確認。
- B: 11 `:57` と 10 B-6 の記載を確認。どちらも担当外なので依存節に要追随として書いた。
- C: `docs/adr/index.md:186` と 10 A-5（`:57`・`:64`・`:155`）を確認。冒頭と依存節を直し、表記を「ADR-179（旧178）」にそろえた。
- D: ADR-191 本文の「### 決定2」が `:182` の BUG-151 最小修正であることを確認。
- E: `set_ime_open_ordered` が授権なしで `false` を返すことを確認し、T2 に記録項目を足した。
- F: `architecture_guard.rs:1874`・`:2128` の既存ガードを確認し、T6 の範囲を絞った。
- G: `runtime/mod.rs:1791-1792` に加え、トレイリセットの `set_ime_mode_for_target(hwnd, true, …)` も `ime.rs:1763` で開閉を書くことを確認した（レビューが挙げていない点）。08 → 09 の向きと中身も依存節に書いた。

### CI での代替観測（2026-09-29、`cal-driftrec-*`。A/B-2 の代替ではない）

**位置づけ:** 09 の A/B-2 手順1は「明示意図 OFF・実 ON」の向きで、この観測は逆向き（明示意図 ON・実 OFF）。T4 の終了条件
（明示意図の回復シナリオが作れるか）にはまだ答えていない。ここで測ったのは「**外部から閉じられた IME を awase が観測するか／観測した後に戻すか**」。

方法: `typing_stress --mode=drift-on`。`VK_IME_ON`（awase の明示意図 ON になるキー）で ON にそろえ、ハーネスが自プロセスの入力欄の IME へ
`WM_IME_CONTROL(IMC_SETOPENSTATUS,0)` を送って awase を経由せず閉じ（別スレッドからの `ImmSetOpenStatus` は失敗する）、+500/+1500/+3000ms の
API 開閉とかな単打の実打鍵結果を記録する。`check_drift_recovery.py` が awase.log を2つの時間窓で突合せる。
**閉→打鍵直前**: observed（ImeModel への開閉観測）・drift（drift correction 発火）。**打鍵中〜確定後**: conv_read・reinit・unicode。
前提（`on_key=VK_IME_ON`、同試行の ON 操作〜close の最後の `explicit_intent=Some(true)`）が成り立たない試行は invalid。
構成: tsf（TsfNative 相当、ADR-193）と edit（Win32 対照）× GJI/MS-IME、各 10 試行×3 回。

実測（run [36511231753](https://github.com/cuzic/awase/actions/runs/36511231753)、windows-latest、`ci/adr178-tsfnative-on-recovery` の `10b5ed51`、
判定スクリプトは同コミットの版。3 回とも同じ結果。数は「その現象があった試行数」、10 試行あたり）:

| 入力先 × IME | verdict | 明示意図 ON | observed | drift 補正 | conv_read | reinit | 実打鍵 |
|---|---|---|---|---|---|---|---|
| tsf × GJI | REOPENED_BY_OTHER_PATH | 10 | **0** | 0 | 10 | 10 | `か`（reopened_by_typing 10/10、各 run） |
| tsf × MS-IME | NOT_OBSERVED | 10 | **0** | 0 | 10 | 0 | 生ローマ字 `ka`（not_recovered 10/10） |
| edit × GJI | UNDETERMINED | 10 | **0** | 0 | 0 | 0 | `か`（typed_blind 10/10、unicode 10） |
| edit × MS-IME | UNDETERMINED | 10 | **0** | 0 | 0 | 0 | `か`（typed_blind 10/10、unicode 10） |

言えること:
- 4構成すべてで、閉じてから打鍵直前まで、awase は **ImeModel へ開閉を観測しなかった**（observed=0。窓内に `[stage-observe]` が無く、`ir_apply_drift_correction` の
  唯一の呼び出し元 `ir_stage_notify`〈`ime_refresh.rs:278`〉に届いていない）。したがって drift correction の判断（`check_drift_correction`、授権、鮮度上限）は
  **一度も走っていない**。「drift correction は ON へ戻さない」とは**言えない**（測れていない）。verdict の NOT_OBSERVED はこの意味。
- 観測しない理由（コード上のコメントによる。`runtime/mod.rs`）: tsf は TsfNative の早期 return（`:1102-1108`、定期ポーリングを予約しない）。edit は明示意図が
  あるときポーリングを止める条項（`:1146-1148`）。再開の契機は、同コメント（`:1094-1095`）によればフォーカス変更・may_change_ime キー・`ReportOpenInference` だけで、
  この測定ではそれ以外は起きなかった。なお edit は ON キーと無関係に、awase が **Unicode 注入**するので、打鍵結果は IME の開閉の証拠にならない
  （今回は明示意図 ON のまま Engine が ON で残ったので、閉じた IME にも `か` が入った）。
- 打鍵時の送信前チェックは conv を読む（conv_read）。ただし ImeModel の開閉観測ではない。**tsf × MS-IME では、この送信前チェック（`output/probe_io.rs` の msime-ready）が
  conv の NATIVE を「ON 確認」と扱い、閉じた IME へ "ka" を送って生ローマ字になった**（30/30。`state=Hiragana confirmed=false` → `NATIVE 確認 → 送信 "ka"`、
  run 36510380572 の `result-cal-driftrec-tsf-msime-native-1` の awase.log。run 36511231753 でも conv_read 10/10 で再現）。conv は閉じても NATIVE のまま残る（`ime_refresh.rs:862-866`）。開閉ではなく conv で
  送信可否を決めていることが、ログで確認できた。conv mode ファミリーの再発として [BUG-172](../known-bugs/BUG-172.md) に起票済み（2026-09-29。実 Chrome では別経路と判明し、ゲート修正は見送り。追補参照）。
- **C-2「TsfNative の ON 方向の救済は drift correction だけ」は、GJI × TsfNative については反証された**: 打鍵して literal を 2 回検出（count=2）→ give-up →
  **GJI reinit（VK_IME_OFF→ON 注入、`probe_io.rs:186`）**が ON 方向の能動書き込みとして開け直す（tsf × GJI、30/30、打鍵後の API は各 run 10/10 で開）。これは
  ずれの検知ではなく打鍵時の事後回復。確定テキストは 30/30 で `か` だが、最初は一時的にリテラルが入り、BS と再送で補正される。MS-IME には同等の経路が無い。この reinit は開閉軸の棚卸し（上表）に載っていない → 追記が要る。

言えないこと（未確認のまま）:
- 撤去前（reassert/force-on あり）のビルドでの同シナリオの対照は無く、領域A撤去で回復力が落ちたかは不明。
- 観測が発生した後（フォーカス変更など）に drift correction が戻すか、BUG-163 1段目の「授権が下りない補正は検知へ進めない」が働くか。ログに
  `[drift] 授権が下りないため補正を見送る` は 0 件（判断に届いていないため）。
- edit 構成（GJI・MS-IME）は Unicode 注入のため、`ime_ready` の前提確認も含め、打鍵結果から IME の開閉は原理的に判別できない。

実行: `gh workflow run e2e-ime.yml --ref <branch> -f only='cal-driftrec-*'`（cal-* は only 指定時だけ走る）。観測のみで合否には含めない。
次の一手の候補: ずれを作った後にフォーカス変更（観測を1回起こす）を挟み、drift correction 自体の判断まで届く条件を作る。

### 追補 2026-09-29: フォーカス変更を挟んだ測定と実 Chrome（計画2・3）

**実 Chrome（`chrome_probe --close-ime=10`、run [36524071258](https://github.com/cuzic/awase/actions/runs/36524071258)〈MS-IME、`--msime` でプロファイルを Chrome 起動前に有効化〉・
[36518453739](https://github.com/cuzic/awase/actions/runs/36518453739)〈GJI〉）**: IME を `WM_IME_CONTROL` で閉じ、3秒後にかな単打。
- 結果: MS-IME 9/10 と GJI 10/10 が `kiu`（閉じたままローマ字。MS-IME の残り1回はフォーカス外れで INVALID）。**GJI reinit も実 Chrome では回復しなかった**
  （RichEdit の tsf × GJI 30/30 との差。reinit は打鍵1回だけを見ており、2回目以降は未確認）。
- MS-IME でも msime-ready ゲートは**経由していない**（`[msime-ready]` は最初の準備打鍵の3件のみ）。BUG-172 の「NATIVE 誤認」は RichEdit 入力先の現象で、実 Chrome では別原因（次項）。
  詳細は [BUG-172](../known-bugs/BUG-172.md)。
- `IMC_GETOPENSTATUS` 自体は実 Chrome の窓でも 1→0 と読める。awase が読まないだけ（次項）。

**フォーカス変更を挟んだ測定（`--refocus`、run [36530903798](https://github.com/cuzic/awase/actions/runs/36530903798)、`cal-driftrec-refocus-*`、各10試行）**:
閉じた直後にタスクバーへフォーカスを外して戻し（`drift_on_refocus` の `away_ok=true`）、awase の FocusChange 経路を通してから打鍵した。

| 入力先 × IME | verdict | observed | drift | 補足 |
|---|---|---|---|---|
| tsf × GJI | REOPENED_BY_OTHER_PATH | 0 | 0 | 打鍵時 reinit で 10/10 回復（reopened_by_typing） |
| tsf × MS-IME | NOT_OBSERVED | 0 | 0 | 10/10 生ローマ字 |
| edit × GJI | NOT_RECOVERED | 10 | 0 | 10/10 未回復（Unicode 注入の窓は無く、実打鍵がローマ字） |
| edit × MS-IME | NOT_RECOVERED | 10 | 0 | 同上 |

言えること（awase.log で確認）:
- **Chrome 系クラス（`Chrome_RenderWidgetHostHWND`）は FocusChange の分類が `profile=Imm32Unavailable`** で、`Skipping IMM query for known-broken class (shadow state SSOT)`。
  フォーカス変更後も開閉を観測せず、drift correction の判断に**届かない**（observed=0）。前回の「届いていない」は、フォーカス変更を挟んでも変わらない。
- **edit（ImmCross）は観測が届く**（`ObserverPoll ime_on true → false`）が、フォーカス変更で **`explicit_intent=None`**（明示意図 ON が消える）になり、awase は閉じた状態を新しい belief として採用する。
  戻す理由が無いので drift=0。これは設計どおりで、バグではない。
- したがって「閉じられた IME を drift correction が ON へ戻す」経路は、フォーカス変更を挟んでも成立しない（Chrome 系は観測不能、ImmCross は意図が消える）。ON への回復は GJI reinit だけで、実 Chrome では効かなかった。

限界:
- 実 Chrome に対する `--refocus`（chrome_probe）は `SetForegroundWindow`/`SwitchToThisWindow` がタスクバーに拒否され（`away=false`。2026-09-29 に別窓方式へ修正し `away=true`、測定結果は BUG-172）、**フォーカス変更は起きていない**（FAIL 10/10 は refocus 無しと同じ）。tsf の結果は同じクラス名の RichEdit での代用で、分類の理由がクラス名なので実 Chrome でも同じと推定しているが未確認。
- 閉じ方は `WM_IME_CONTROL`（外部要因の再現）で、実運用の閉じ方との対応は未確認。撤去前ビルドとの対照は force-ON のみ復元して実施済み(reassert は復元不能、[BUG-172](../known-bugs/BUG-172.md) 参照)。

次の一手の候補: 実 Chrome で観測できないこと（`Imm32Unavailable`）が「外部から閉じられた」ケースの本質的な限界かを、実運用の経路（他アプリ・OS による IME OFF）の頻度から判断する。頻度が低ければ対処しない選択もある。
