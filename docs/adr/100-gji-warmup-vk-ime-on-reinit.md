# ADR-100: GJI eager warmup キーの再選定と give-up 分岐の retry — 提案の却下・縮小版の実験登録・前提条件の切り出し

## ステータス

**決定2（`VK_IME_ON` 単発への置換）は2026-08-22、実機検証を経てユーザー判断により実装済み。決定1は却下のまま。決定3の案L（journal記録）は2026-08-23、BUG-74（ADR-100が予告していた「give-upによる文字消失の再報告」そのもの）への対応として実装済み。決定3で却下した「提案2」は、2026-08-24にADR-101で前提条件（完了通知、focus世代照合、送信後処理、deferred順序保証、retry上限）を満たす別設計として実装済み。決定5/F6もADR-101 Stage1で実装済み。** 決定2の群A/群Cとの厳密な比較・BUG-69依存の群C検討は今後の課題として残る。設計は premortem レビュー ラウンド1・ラウンド2（評決「N1〜N3 を反映すれば実装着手可、ラウンド3 の全面レビューは不要」）を経ている。ADR-098 決定3-c（「GJI の eager warmup キーを `VK_DBE_HIRAGANA` から `VK_IME_ON` へ置き換えられないか」、本 ADR では決定しないとして先送りされた宿題）と、[docs/experiments.md](../experiments.md) エントリ16（同件の事前登録）を正式に引き取り、ユーザー発案の2提案について結論を出す。

**ラウンド1 レビューで初稿の事実誤認4件が訂正された（F3・F5・F12・決定1 案C）。** うち2件は「本 ADR 自身が F10 で引用している BUG-45 の実機ログが、別の節の主張を否定していた」という自己矛盾であり、単独の事実誤りより重い。訂正内容は各 F の「【第2稿で訂正】」節に明記した。**訂正の結果、決定1・決定3 の却下という結論は維持されるが、却下の根拠のうち2本（「Tsf モードでは撃たれていない」「IMC が読めるか未検証」）は弱まった、または消えた。**

**ラウンド2 でさらに事実誤認1件（F3 の呼び出し元列挙、`flush_raw_tsf_literal_romaji` の欠落 = 3度目の「モード分割の言い切り」）と、案L の作業範囲・プライバシー方針の未確定が指摘され、反映した**（F3「【第2稿の第2訂正】」、F12 の `injection_mode` 列、決定3 の「案L の作業範囲」「案L のプライバシー方針」）。**ラウンド1・2 を通じて決定は一つも変わっていない**——変わったのは根拠の記述と実験設計の精度だけである。

本 ADR の初稿・改訂2版の調査は Linux 上のコード読解と既存の実機ログ記録（`docs/known-bugs.md`）に基づいていた。**2026-08-22、決定2 群C・群Bについて実機測定を実施した（F15〜F18、Windows Terminal / dragonflyg4）。** 群Cは第1回が診断ゲートの配置による交絡で無効、第2回（交絡解消後）は3シナリオそれぞれで最低1回の成功例を確認。群Bは決定4-f（`MapVirtualKeyW(VK_IME_ON)` の実機測定、F17）で送信形態を確定させたうえで2ラウンド実施し、`giving up`/literal 化は0件だった。いずれも群A（現行F2）との直接比較を含む決定2の合格基準には遠く届いていない。**Opus advisor によるレビューで、この結果（群C）を撤去の正式決定へ格上げするのは時期尚早であり、むしろ BUG-69（ADR-098）F1/F2 の未参照という別の穴が優先度として先に立つと指摘された（P6 参照）。** その BUG-69 修正自体は2026-08-22、実機で初回検証済み（`docs/known-bugs.md` BUG-69「Windows 実機での初回検証」節）。決定4 の残り項目・群A との直接比較実験は未実施のまま。

## コンテキスト

### 発端

ADR-098（BUG-69、`applied` の belief 偽装と到達不能 force-on ブロックの撤去）の実装完了直後、ユーザーから2点の提案が出た。

1. **提案1**: eager TSF warmup（`Output::send_eager_tsf_warmup`）が送っている `VK_DBE_HIRAGANA`（F2）を、`VK_IME_OFF → VK_IME_ON` のトグル送信（`ProbeIo::send_chrome_gji_reinit_and_poll` が既に持つ機構）に置き換える。
2. **提案2**: per-VK confirm（literal 検出）の give-up 分岐（2連続 literal 検出時、現状は backspace のみで romaji を再送しない）に、reinit 完了確認後の retry（romaji 再送）を追加する。

提案の動機は正当である。`VK_DBE_HIRAGANA` は「IME を開く」と「ひらがなに強制する」を1つの副作用に束ねており（BUG-50 デッドロックの直接の前提。MS-IME 側の ON キーは同じ理由で 2026-08-06 に `VK_IME_ON` へ移行済み）、BUG-15 追補7 は「IME モードキーの注入は実 IME が確実に ON でない限りしてはならない」とこの注入パターン自体を名指しで警告している。ADR-098 F4 はこれを「受容中の既知リスク」として明示的に残した。

### ADR-098 決定3-c との関係

ADR-098 決定3-c の原文は以下である（`docs/adr/098-*.md:380`）:

> MS-IME の ON キーは 2026-08-06 に `VK_DBE_HIRAGANA` から `VK_IME_ON` へ移行して conv 破壊を解消した。eager warmup で同じ置換ができれば F4 は構造的に消えるが、`VK_IME_ON` が GJI の TSF composition context 再初期化を BUG-02 と同等にトリガーするかは未検証であり実機実験が必要。

**重要な差**: 決定3-c が想定していたのは `VK_IME_ON` の**単発送信**（open 軸のみを触る、conv には触れない）である。提案1 はそれを `VK_IME_OFF → VK_IME_ON` の**トグル**へ拡張している。この差は本 ADR の結論を分ける（決定1・決定2）。

### この ADR が引き取る宿題

- ADR-098 決定3-c（未着手のまま残された）
- `docs/experiments.md` エントリ16（事前登録、未実施）
- BUG-32 の「未解決の残課題」: 「次に同種の報告があれば、per-VK confirm の give-up 分岐から `send_eager_tsf_warmup` を再試行する経路の追加を検討すること」

---

## 確定した事実（F1〜F14）

以下は全て本 ADR のためにこの worktree の実コードを読んで確認した。行番号は執筆時点（`feat/adr100-gji-warmup-vk-ime-on`、develop 分岐直後）のもの。**例外は F2 の1項目のみで、そこはコードコメント依拠であることを明記した（m12）。**

### F1: eager warmup は `InjectionMode::Tsf` でしか発火しない。**Chrome は対象外である。**

送信ゲートの最終段は `TsfReadiness::can_warmup()`（`crates/awase-windows/src/tsf/tsf_gate.rs:348-350`）:

```rust
pub const fn can_warmup(&self) -> bool {
    self.ime_on && self.is_tsf_mode
}
```

`is_tsf_mode` は `self.injection_mode == InjectionMode::Tsf`（`output/mod.rs:663-665`）。そして `InjectionMode` の決定表（`output/types.rs:16-30`）は:

| 優先 | 条件 | `InjectionMode` |
|---|---|---|
| 1 | `InjectionHint::ForceTsf` | **`Tsf`** |
| 2 | `InjectionHint::ForceVk` | `Vk` |
| 3 | `AppKind::TsfNative` | `Vk` |
| 4 | それ以外（Win32 / Uwp） | `Unicode` |

`InjectionHint::ForceTsf` になる経路は2つだけ（`focus/tracker.rs:80-93`）: (a) config の `app_overrides.force_tsf` エントリにマッチ（`src/config.rs:484`）、(b) `injection_mode_store.has_tsf(class_name)` — ADR-062 の実行時自己学習（`UnicodeLiteralObserverFsm` が「GJI write bytes が増えない」と判定したウィンドウクラスを `Tsf` へ昇格、`platform.rs:437-443` の `[injection-mode] ... → Tsf 事後昇格`、`cache.toml` の `[injection_mode]` に永続化）。

**帰結**:

- **Chrome/Edge（`Chrome_WidgetWin_1`）は `AppKind::TsfNative`（`focus/class_names.rs:220-228`、`chrome_` prefix）→ `InjectionMode::Vk` であり、eager warmup は 1 度も飛ばない。** ブリーフィング資料が「本提案が warmup 対象とするアプリ（Chrome/WezTerm/Windows Terminal、F2 cold-start 対策が要る全アプリ）」と書いていたのは誤りである。
- **さらに強く: Chrome は実行時学習でも `Tsf` へ昇格しない。** `UnicodeLiteralObserverFsm` の install 条件が `self.injection_mode == InjectionMode::Unicode`（`output/mod.rs:811-816`）であり、`Vk` モードのアプリは観測対象にすら入らない（ラウンド1 レビューによる補強）。**config `force_tsf` に明示登録しない限り、Chrome は永久に eager warmup 対象外である。**
- eager warmup の実対象は「config で `force_tsf` 指定されたアプリ」＋「実行時学習で `Unicode` → `Tsf` へ昇格したアプリ」という、**ユーザー環境ごとに異なる可変集合**である。ADR-083 の「調査で訂正された事実誤認」項目2 が既にこの点を明記している（「Tsf モードは実行時に増殖しうる」）。
- **未検証**: ユーザーの実機で WezTerm / Windows Terminal が実際に `Tsf` になっているかは `[injection-mode]` / `[tsf-eager-warmup]` ログで確認する必要がある（決定4-c）。

### F2: eager warmup は3ゲートを通り、打鍵駆動で高頻度に呼ばれる

`Output::send_eager_tsf_warmup`（`output/mod.rs:700-729`）のゲートは順に:

1. `self.conv_mutation_allowed.get()`（非 AwaseOwned ならスキップ）
2. `self.warmup_coord.needs_f2_probe()`（GJI 戦略でなければスキップ。`MsImeStrategy` は false）
3. `self.tsf_readiness(warmup_ime_on).can_warmup()`（= F1）

通過後は `crate::tsf::send::send_vk_dbe_hiragana_pair()`（`tsf/send.rs:20-38`）が `VK_DBE_HIRAGANA` の DOWN/UP ペアを1回 `SendInput` する。Win キー押下中は `None` を返して送信をスキップする（BUG-32 の修正形）。

本番の呼び出しサイトは6箇所（grep 実測）:

| 場所 | 属する関数 | 契機 |
|---|---|---|
| `platform.rs:265` | `send_eager_warmup`（`:264`） | `ime_refresh.rs:506` から（フォーカス変更 Stage3） |
| `platform.rs:582` | `dispatch_composition_response`（`:566`） | `CompositionAction::EmitWarmup`（`CompositionFsm` 駆動。物理 F2・確定キー・Ctrl 解放など） |
| `platform.rs:1164` | `on_ime_applied`（`:1089`） | 実 actuation 完了直後の2発目 |
| `platform.rs:1215` | `on_passthrough_key`（`:1173`） | 確定キー KeyDown |
| `platform.rs:1236` | `on_reinject_key`（`:1199`） | 再注入キー |
| `output/vk_send.rs:531` | — | `WarmupImeOn::off()` 固定。**「到達不能」という判定は同ファイル 518-528 行のコードコメント（「現状は理論上到達しない」）に依拠しており、本 ADR は独立検証していない**（m12。この1点だけ他の F と検証の質が異なる） |

ADR-098 決定1-b の付け替え表が同じ集合を別の粒度（`composition_confirm_key_up` / `composition_ctrl_up` / `composition_native_f2_down` を含む10サイト）で列挙している。**要点は「フォーカス変更時だけでなく、Enter/Escape 確定・Ctrl 解放・物理 F2・再注入キーのたびに撃たれうる」**ことである。コード上のコメントも「ROMAN ビット確保のみで冪等なため反復送信も無害」と、この高頻度性を前提に書かれている（`output/mod.rs:716-717`）。

### F3: `send_chrome_gji_reinit_and_poll` の本番呼び出し元は2つ。うち give-up 経路は **`Tsf` モードでも `Vk` モードでも発火する**

#### 【第2稿で訂正】

初稿は「本番呼び出し元は2つだけで、どちらも `Tsf` モードではない」「移設先は現在その機構が一度も使われていない `Tsf` モード」と書いた。**これは誤りである。** しかも初稿自身が F10 で引用した BUG-45 の実機ログ（`[h1-warmup]` / `[gji-coro]`）が、その反証をその場に含んでいた。訂正の根拠を以下に示す。

#### 呼び出し元（grep 実測、テスト stub を除く）

| # | 呼び出し元 | 発火条件 |
|---|---|---|
| 1 | `Output::send_f22_f21_reinit`（`output/mod.rs:897-899`） ← `platform.rs:521` | **`injection_mode == InjectionMode::Unicode`** かつ long-cold かつ deferred chars が空（`platform.rs:513-521`） |
| 2 | `Output::flush_raw_tsf_literal_recovery`（`output/mod.rs:1159-1164`） | give-up 分岐が `schedule_chrome_gji_reinit` で予約した `pending_gji_reinit_cold_seq` の消化（BUG-36）。**injection mode 非依存** |

#### give-up 分岐の emitter は2系統で、モードによって役割分担されている

`ProbeAction::RawTsfLiteralRecovery` を emit するのは以下の2つで、`emit_recovery_actions`（`tsf/warmup/literal_detect_fsm.rs:12-18`）を共有する:

| 系統 | 実装 | install サイト | モード |
|---|---|---|---|
| cold パス | `GjiWarmupCoro` Phase 6（`tsf/warmup/gji_warmup_coro.rs:217-232`、`LiteralDetectCore::new(..., DetectTarget::Tsf, ...)`） | `output/vk_send.rs:300-320`（`send_romaji_as_tsf` の `prepend_f2_warmup` 分岐内） | **主に `Tsf`。ただし `injection_mode` 限定ではない**（下記の呼び出し元表 #3 を参照） |
| warm パス | `LiteralDetectFsm` | `output/vk_send.rs:452-460` | **`Tsf` 以外**（install 条件に `&& !self.is_tsf_mode()` という明示ゲートがある、`vk_send.rs:460`） |

**つまり「`Tsf` モードの literal 検出は `GjiWarmupCoro` が担当する」という役割分担が設計として存在する。** どちらの系統から give-up に落ちても `schedule_chrome_gji_reinit` → `flush_raw_tsf_literal_recovery` → `send_chrome_gji_reinit_and_poll` に至る。

#### 【第2稿の第2訂正】cold パスのモードは `injection_mode` だけでは決まらない

第2稿の初版はこの表の cold パス行に「**`Tsf` のみ**（`send_romaji_as_tsf` の到達経路は `TsfSender` と `Output::send_romaji` で、いずれも `InjectionMode::Tsf` 限定）」と書いた。**これも言い切りすぎであり訂正する**——`send_romaji_as_tsf` の本番呼び出し元は2つではなく**3つ**である（`grep -rn "send_romaji_as_tsf"` 実測、コメント行を除く）。

| # | 呼び出し元 | 分岐条件 |
|---|---|---|
| 1 | `TsfSender::send_romaji`（`output/sender.rs:52`） | `InjectionMode::Tsf` |
| 2 | `CompositionOutput::send_romaji`（`output/mod.rs:1053-1058`） | `InjectionMode::Tsf` |
| 3 | **`Output::flush_raw_tsf_literal_romaji`（`output/mod.rs:1135-1140`）** | **`tsf_gate.state()`。`injection_mode` を参照しない**（`Bypass` なら `send_romaji_batched`、それ以外なら `send_romaji_as_tsf`） |

`TsfGateState` は `injection_mode` とは独立した軸である（初期値 `Bypass`、`tsf/tsf_gate.rs:114-116`）。したがって **`Vk` / `Unicode` モードのアプリでも gate が `Bypass` 以外なら `send_romaji_as_tsf` に入り、`prepend_f2_warmup` 分岐で `GjiWarmupCoro` が install されうる。**

しかも呼び出し元 #3 は **literal recovery の再送そのもの**である。すなわち本節は「literal recovery の emitter のモード分割」を論じながら、literal recovery フロー自身に含まれる呼び出し元を落としていた。**初稿の C1（モード分割の言い切り）と同型の誤りが、C1 を修正した表の中で再発していた**という点で、これは本 ADR で3度目の同種の誤りである（experiments.md エントリ16 の「学び」に反映する）。

**付随して記録する穴**: `CompositionOutput::send_kana_char`（`output/mod.rs:1061-1063`）は `send_char_as_tsf(ch)` を **`injection_mode` の分岐なしに**呼ぶ（すぐ上の `send_romaji` が3分岐しているのと対照的）。トレイト宣言は `src/platform.rs:134`。**ただし本番呼び出し元はリポジトリ全体で 0 件**（grep のヒットは宣言・実装・無関係なコメントの3件のみ）なので現状は死んでいる。**誰かが `send_kana_char` を使い始めた瞬間に、モード分岐の無い経路が開く**ため記録しておく。

#### BUG-45 のログが `Tsf` モードだった証拠

F10 が引用する BUG-45 の再現ログには `[h1-warmup] reason=SetOpenTrue` と `[gji-coro] settle 必要` が含まれる。この2つのログ文字列の出力元は grep で一意に定まる: `[h1-warmup]` は `tsf/warmup/cold_warmup.rs:108` のみ、`[gji-coro]` は `tsf/warmup/gji_warmup_coro.rs:93/108/153` のみ。いずれも上表の cold パス（`Tsf` 限定）にしか無い。**したがって BUG-45 の Windows Terminal はそのとき `InjectionMode::Tsf` だった。** 傍証として `docs/known-bugs.md:2980` が「"gji-coro"（WezTerm 側）」と書いている。

#### 帰結（初稿からの変更点）

- 「移設先は当該機構が一度も使われていないモード」という論法は**消える**。give-up 経路の reinit は `Tsf` モードで現に発火している。
- 残るのは**頻度差**である: reinit は「literal 2連続失敗時」というエラー時のみ、eager warmup は「確定キー・Ctrl 解放・再注入・物理 F2 のたび」（F2）。決定1-(a) はこの頻度差のみに基づく論法へ書き直した。
- 初稿の「実績の文脈が違う」という表現は「モードが違う」ではなく「頻度が違う」に限定する。

なお関数名の `chrome_` は misnomer である（呼び出し元1は Unicode モード全般、呼び出し元2は injection mode 非依存）。`tuning.rs:106-119` の doc も「Chrome/Unicode-mode GJI 再初期化」と書いており、`output/mod.rs:895-898` の `send_f22_f21_reinit` の doc は「async IMC ポーリングは行わない」と書いているが実装は `send_chrome_gji_reinit_and_poll` を丸ごと呼んでおり**実際にはポーリングも走る**（決定7）。

### F4: `FeedbackPolicy::Blind` と IMC ポーリングは矛盾しない。ただし理由は「別軸だから型が矛盾を検出できていない」である

ブリーフィング資料が「★最重要、未解決の疑問」として挙げた点。結論は**矛盾しない**。根拠:

- `FeedbackPolicy`（`state/ime_actuation.rs:17-30`）の `Read` variant が持つ `source` は `ObservationSource::ImmGetOpenStatus`（`state/app_ime_policy.rs:73-79`）。つまり `Read`/`Blind` の区別は **open 軸（`ImmGetOpenStatus` / `IMC_GETOPENSTATUS`）の読み戻し可否のみ**を指す。
- `Blind` が実際にゲートしているのは3点だけである: (1) 試行回数の有界化（`decide_actuation_action`、`ime_actuation.rs:163-174`、`max_attempts=5`、`app_ime_policy.rs:26`）、(2) 収束確認クエリの種別（`ReadBackQuery::AnyFreshEvidence` vs `Converged{desired}`、`observation_store.rs:31-48`）、(3) `OpenWarrant` Step 4c（`OwnSsot`）の解禁（`state/open_warrant.rs:201-204`）。**`Blind` は特定の Win32 読み取り API の呼び出しを禁止するフラグではない。**
- `send_chrome_gji_reinit_and_poll` が読むのは conv 軸（`IMC_GETCONVERSIONMODE`）であり、その結果は `Output::update_ime_mode_from_imc` → `ImeModeFsm::on_conversion_mode_read` で終端する（`output/mod.rs:376-378`）。
- **`ImeModeFsm` は `ObservationStore` にも `ImeModel` にも一切書き込まない**（読み手を全数追跡して確認済み。読み手は `vk_send.rs:370`（MS-IME gate defer 判定）、`probe_io.rs:235`（このポーリングの打ち切り条件）、`probe_io.rs:306`（MS-IME ready poll）、`output/mod.rs:812-816`（Unicode literal observer の install 判定）、`output/mod.rs:909-916`（`TsfEnvSnapshot` 組み立て）、`ms_ime_ready_coro.rs:68-76` の6箇所で、いずれも送信タイミング判断に閉じている）。`Output` は `PlatformState`/`Runtime` を参照できないため、`record_observation` / `dispatch_event` / `set_user_explicit_intent` を呼ぶ経路を構造的に持たない。`TsfEnvSnapshot` 経由の間接読み手も追跡したが、`literal_detect_fsm.rs` は `env.ime_mode` を参照していない（grep 0 ヒット）ため、literal 判定へ belief が流れ込む経路も無い。
- 紛らわしい類似経路（別物）: `ObservationSource::ConvBitsInference` / `ConvOpenInference` を書く唯一の本番経路は `runtime/key_pipeline.rs:571-575` / `:612` の idle-conv-check であり、その conv 値は `key_pipeline.rs:388` の**独立した** `get_ime_conversion_mode_raw_timeout_async(10)` 由来。`ImeModeFsm` を経由していない。

**したがって、BUG-19/33/48/68/69 が繰り返し踏んできた「belief を evidence として書き戻す」パターンには該当しない。** `.claude/rules/ime-belief-architecture.md` の三層分離にも違反していない。

ただし残る問題は2つある:

1. **conv 軸の読み取りには profile ゲートが一切無い。** 対称な open 軸の `read_ime_state_fast`（`ime.rs:915-925`）は `profile.can_read_imm32_open_status()`（`focus/class_names.rs:186-188`、`Standard` のみ true）で Chrome/WezTerm を弾いて `ime_on=None` を返すのに対し、conv 読み取りは無条件に発行される。ただしこれは「未整理」ではなく別方針の設計である可能性が高い（決定6、m14）。
2. **`Blind` という語が open 軸限定であることがコード上から読み取れない。** 本 ADR の起票にあたってブリーフィング資料が「矛盾するのではないか」と疑問を持ったこと自体が、この読み取りにくさの実例である（決定6）。

### F5: IMC ポーリングは読めない環境で無言のまま 300ms 空回りする。**ただし TSF ネイティブアプリで読めた実機事例が 1 件記録されている**

#### 【第2稿で訂正】

初稿は「TSF ネイティブアプリで `ImmGetDefaultIMEWnd` が有効なウィンドウを返すかはコードからは判定できない。**これが本 ADR で最も重要な未測定量である**」と書いた。**これは、本 ADR 自身が F10 で引用している実機記録を評価しないまま「未測定」と宣言したものであり、訂正する。**

`docs/known-bugs.md:5052-5070`（BUG-45 の「**再現手順（ログで確認済み）**」ブロック、初稿が F10 で引用したのと同一ブロック）に次の一行がある:

> flush 時に backspace×1 → reinit(VK_IME_OFF→VK_IME_ON) 実行、**IMC poll で Hiragana 確認**

「IMC poll で Hiragana 確認」が成立するのは `probe_io.rs:232-241` の `confirmed`（`fsm.state().is_hiragana() && fsm.is_confirmed()`）が true になった場合だけであり、`is_confirmed()` が立つのは `on_conversion_mode_read` が `Some(mode)` を受け取ったときだけである（`tsf/ime_mode_fsm.rs:147-151` で `None` は早期 return）。**すなわち Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`、TsfNative、2026-07-29 実機）で `ImmGetDefaultIMEWnd` + `IMC_GETCONVERSIONMODE` は実値を返していた。**

さらに `get_ime_conversion_mode_raw_timeout` は BUG-55（2026-08-07）で `GetForegroundWindow` から `get_focused_hwnd()`（`GetGUIThreadInfo().hwndFocus`）へ変更されており、その理由コメント（`ime.rs:421-431`）が「トップレベルとは別の子ウィンドウが実際の TSF composition を持つアプリ」を名指ししている。**この API 経路は Windows Terminal を読むために調整された履歴を持つ。**

#### 訂正後の正しい理解

- **原理的に読めるか**: 少なくとも1件、TSF ネイティブアプリで読めた実機事例がある。「読めない」は既に反証されている。
- **なお未知なこと**: (i) 常に読めるか、(ii) どのクラス（WezTerm 本体・InputSite・CoreWindow）で読めるか、(iii) confirmed に到達する割合。**これらは「頻度・再現性の未測定」であって「原理の未検証」ではない。**
- したがって決定4-b は「最重要の未測定量」ではなく「**肯定事例 1 件あり、再現性と頻度が未測定**」と位置づける。

#### 劣化の機序（この部分は初稿のまま有効）

`get_ime_conversion_mode_raw_timeout_async(15)`（`ime.rs:847-851`）の実体は、ワーカースレッド上で `ImmGetDefaultIMEWnd(get_focused_hwnd())` → `SendMessageTimeoutW(WM_IME_CONTROL, IMC_GETCONVERSIONMODE, SMTO_ABORTIFHUNG, timeout_ms)` を呼ぶ（`ime.rs:421-451`、`imm.rs:105-166`）。**`ImmGetConversionStatus` ではない。**

失敗時の戻り値は全て `None`:

- `get_focused_hwnd()` が NULL → `ime.rs:432` の `.non_null()?`
- `ImmGetDefaultIMEWnd` が NULL → `ime.rs:448` の `let ime_wnd = ime_wnd?;`（**この経路における BUG-33 の "himc_null" の構造的等価物**）
- `SendMessageTimeoutW` がタイムアウト/失敗 → `imm.rs:166` の `(ok.0 != 0).then_some(result)`

`None` は `ImeModeFsm::on_conversion_mode_read` で belief を一切変更せず debug ログのみで早期 return する。したがって `confirmed` は false のまま、ポーリングは `max_retries = 300/10 = 30` 回まで空回りし、`[chrome-reinit] ... ポーリング完了` だけを出して終わる。

**帰結（強度を落とした形）**: 「confirm-then-transmit の方が fire-and-hope より頑健」という提案の前提は、**IMC が読める環境でのみ成立する**。読めない瞬間があれば、そのとき得られるのは「300ms 待った」という事実だけである。1 件の肯定事例はあるが、**常に読めるという保証は無い**。

### F6: `send_chrome_gji_reinit_and_poll` のポーリングには**フォーカス世代ガードが無い**

`ime_mode_focus_gen` を捕捉して後で照合する経路は3つある:

| # | 場所 | 照合の有無 | 用途 |
|---|---|---|---|
| 1 | `start_ms_ime_ready_poll`（`probe_io.rs:296` 捕捉 / `:302-304` 照合） | **あり**（`MsImePollStatus::Stale` で黙って終了） | MS-IME ready poll の stale 破棄 |
| 2 | hint ポーリング（`platform.rs:697-708`） | **あり** | 同上 |
| 3 | `cold_warmup.rs:66` 捕捉 → `:95` 付近で `with_app` 経由の再取得と照合 | **あり** | conv actuation の対象特定（`ActuationTarget::capture(focus_gen)` / `set_ime_conv_for_target`）。用途は上2つと異なるが、「フォーカス依存の非同期作業の前に世代を照合する」というパターンは共通（m9） |
| 4 | **`send_chrome_gji_reinit_and_poll`（`probe_io.rs:208-253`）** | **無し** | — |

かつ `update_ime_mode_from_imc` は無条件に `confirmed=true` を立てる（`update_ime_mode_hint_from_imc` と違い）。**ポーリング中にフォーカスが変わると、別ウィンドウの conv 値で `ImeModeFsm` を confirmed にしうる。**

`on_ime_mode_focus_changed`（`output/mod.rs:392-396`）が `ime_mode_focus_gen` をインクリメントする理由として doc に「以前の `spawn_local` IMC ポーリングが古いフォーカスの結果を書き込まないよう保護する」と明記されているにもかかわらず、この関数だけがその保護を使っていない。

これは本 ADR の提案とは独立した既存の欠陥であり、かつ提案2（retry の配線）の前提条件でもある（決定5）。

### F7: `VK_IME_OFF → VK_IME_ON` トグルには**実測された副作用が2件記録されている**

1. **IME 種別検出の単発フリップ（実測、2026-07-07 ユーザー提供ログ）**: `tsf/gji_monitor.rs:236-252` の `ImeKindDebounce` の doc が根拠を明記している——「`send_chrome_gji_reinit_and_poll` が送る実 `VK_IME_OFF→VK_IME_ON` トグル直後に 2146ms 間隔で2回 `[gji-fsm] StartComposition while engine off` が観測され、2秒ポーリング周期と一致」。同 doc はさらに、デバウンスを入れる前は `set_active_ime_kind` が warmup 戦略（`GjiFsm`/`MsImeStrategy`）を丸ごと新規生成して `OnWarm`/`OnComposing` を破棄するため、「Chrome cold-start reinit → 一時的な誤検出 → GjiFsm 再構築 → 次の単語も cold → 再度 reinit → …」という**自己増幅ループ**になったと記録している。
2. **未確定 preedit の commit（BUG-36、確定）**: reinit の `VK_IME_OFF` は未確定の preedit を commit してしまう。だから give-up 分岐では backspace の**後**に送らなければならず、`schedule_chrome_gji_reinit` による予約機構がわざわざ導入された（`output/probe_io.rs:75-81`、`output/mod.rs:1147-1164`）。**backspace → reinit の順序は絶対である。**

対して `VK_DBE_HIRAGANA` 単発は「開く + ひらがなに寄せる」の冪等な操作であり、composition を閉じる作用は持たない。**提案1 は、composition を閉じない操作を composition を閉じる操作へ置き換える。**

### F8: give-up 分岐は**既に reinit を予約している**。欠けているのは romaji 再送だけである

`ProbeAction::RawTsfLiteralRecovery` ハンドラ（`output/probe_io.rs:615-678`）:

- `consecutive == 0`: `io.set_raw_literal(backs, romaji, escape_composition)` — backspace + romaji 再送を予約
- `consecutive != 0`（give-up）: `io.set_raw_literal(backs, String::new(), escape_composition)` — **romaji を空文字にして再送を無効化** + `io.schedule_chrome_gji_reinit(cold_seq)`

したがって提案2 の「reinit を追加する」部分は既に実装済みである（BUG-33 で導入）。提案2 の新規部分は「reinit 完了確認後に `String::new()` ではなく元の romaji を再送する」ことのみ。

`probe_io.rs:1294-1303` に、この構造を固定するテスト assertion が存在する（「give-up 分岐は `send_chrome_gji_reinit_and_poll` を直接呼んではいけない」「`schedule_chrome_gji_reinit` で1回予約すべき」）。

**付随して確認した事実（決定3 の代替策 案L に直結）**: give-up 側の `log::warn!`（`probe_io.rs:664-669`）は `consecutive` と `backs` を出すが、**捨てた `romaji` の値を含んでいない**（`consecutive == 0` 側の warn には `{romaji:?}` がある）。つまり現状、give-up で失われた文字が何だったかはログからも journal からも復元できない。

### F9: 「give-up 後の再送」は過去に msedge で実機破壊し撤回された。**ただし真因は retry という発想ではなく別の検出器バグだった**（BUG-27 追補2 → 追補3）

- **追補2（2026-07-17、撤回）**: msedge（`Chrome_WidgetWin_1`、`Imm32Unavailable`、GJI）で「書いたそばから Backspace されて、まったく何も入力できません」。`vk_sent 未設定` が**打鍵のたびに毎回**発火し、`consecutive` が 6→7→…→12 と単調増加して一度も 0 に戻らず、常に give-up 分岐（backspace のみ）に固定された。撤回された修正は「`vk_sent 未設定` を `SuspectedLiteral` と同じ backspace+再送リカバリとして扱う」もの。
- **追補3（2026-07-17、根治）**: 真因は `ChromeProbe` が `apply_vk_sent` を内側コルーチンへ委譲し忘れていたこと。修正済み（回帰テスト `chrome_probe_apply_vk_sent_reaches_inner_coro` 追加）。
- **追補3 自身の結論**: 「撤回した backspace リカバリを**再度有効化する必要はない**——今回の根本修正で `vk_sent 未設定` の到達頻度自体が激減するはずなので、無リカバリの `return` のままで実害はほぼ無くなる見込み」。

**この事実の正しい読み方**: 撤回されたのは「`vk_sent 未設定` という信頼できないシグナルに基づく積極的リカバリ」であって、「本物の `SuspectedLiteral` に基づく resend」ではない。後者は `consecutive == 0` の分岐として今も本番で生きている。したがって提案2 は BUG-27 追補2 の再演ではない——**が、追補2 が示した失敗モード（`consecutive` が 0 に戻らないまま積み上がる状況で give-up 分岐の挙動が全打鍵に適用される）は依然として起こりうる**。BUG-27 追補4 が `CompositionConfirmed` でも `consecutive` をリセットするようにした（`probe_io.rs:697-702`）ことでリセット契機は増えたが、`CompositionConfirmed` 自体が届かない環境（＝まさに literal 化が続く環境）ではリセットされない。

### F10: BUG-45（未解決）が give-up → reinit 経路そのものを実機で問題視している

BUG-45（Windows Terminal、GJI、TsfNative、**F3 のとおり `InjectionMode::Tsf`**、2026-07-29 実機ログ）は「かきの」→ **"kaきの"**。ログ上の経路は本提案が触る場所と完全に一致する: 2連続 literal → give-up: backspace×1 のみ + `VK_IME_OFF→VK_IME_ON` reinit 予約 → flush 時に backspace → reinit 実行 → IMC poll で Hiragana 確認 → 以降正常。

BUG-45 の結論部が本 ADR にとって決定的である:

> 要するに、この経路には「actual にどう出力されたか」を確認してから次の一手を決める箇所が一つもない: suspected literal 判定も、backspace 数も、give-up 後の reinit も、すべて過去の代理指標から推測した belief の上に belief を積み重ねているだけで、どこかで一度でも belief が実態とズレると訂正する手段がない。

BUG-45 が挙げる恒久対策の方向性 (b) は「『合成成功の証拠切れ』を即 literal 確定にせず、**実際に画面へ literal 文字が出たことを確認してから backspace する**設計への変更」であり、**提案2（送信を1つ増やす）とは逆向き**である。

なお BUG-45 追補1 は「reinit の `VK_IME_OFF` が pending preedit を 'ka' として commit した」という筋を「推測」に格下げしている（3系統の独立解析で、`idx=1` の `vk=0x41`('A') が一度も SendInput されていないことが確認され、pending composition に 'a' が含まれる余地が無いため）。反証はされていないが、支持する具体証拠も無い。

**また、このログの「IMC poll で Hiragana 確認」の一行が F5 の訂正根拠である**（同じログが2つの F に効いている）。

### F11: reinit にはレート制限があり、連続 give-up では2回目以降が**skip されうる**

#### 【第2稿で緩和】

初稿の見出しは「2回目以降が skip される」と言い切っていたが、条件付きに緩める。

`probe_io.rs:169-180`: 前回の `last_gji_reinit_ms` から `CHROME_GJI_REINIT_CONFIRM_MS`（300ms、`tuning.rs:119`）経過していなければ**送信せずに return** する（BUG-33 で導入。理由は「OFF→ON→OFF→ON… の瞬間的な OFF ブリップが積み重なり GJI 側の composition/候補ウィンドウを無用に揺らす」）。

ただし literal 検出窓も `RAW_TSF_LITERAL_DETECT_MS = 300`（long idle は 500、`tuning.rs:72/80`）であり、give-up 間隔は概ね検出窓以上になる。**したがって skip するかは境界依存であり「必ず skip」ではない**（実際 BUG-45 のログでは reinit は実行されている）。実頻度は `[chrome-reinit] cold=N skip:` ログで測れる（決定4-g）。

**帰結**: 提案2 の「reinit 完了確認後に retry」は、reinit が skip された場合に何を待つのかが未定義になる。skip でも retry するなら、それは reinit と無関係な無条件 retry であり F9 の失敗モードに近づく。

### F12: give-up は実機で**少なくとも3回、ユーザー可視の文字欠落を伴って発生している**

#### 【第2稿で訂正】

初稿は「BUG-33 の実機ソーク（2026-07-22）で 0 件が最後の記録」とだけ書き、決定4-a で「0 件に近ければ提案2 の期待値もほぼ 0」と推論した。**リポジトリ内の記録を2件取りこぼしていた。**

| # | 日付 | アプリ（クラス） | `AppKind` → 既定 `injection_mode` | 出典 | 症状 |
|---|---|---|---|---|---|
| 1 | 2026-07-08 | UWP テキストフィールド（`Windows.UI.Input.InputSite.WindowClass`、`AppImeProfile` は TsfNative） | `AppKind::Uwp`（`class_names.rs:229-232`、`windows.ui.input.` prefix）→ **`Unicode`**。実行時学習で `Tsf` へ昇格していたかは**未確定** | BUG-16 追補3（`known-bugs.md:907-925`） | `GjiFsm` が `OffCold` 固着 → `StartComposition while engine off — ignored` → 2連続 literal → 「giving up ... no re-send」で当該文字が backspace のみで消失（「このせっけい」→「せっけい」） |
| 2 | 2026-07-23 | Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`）、GJI | **未確定**（ログに `[gji-coro]`/`[h1-warmup]` の記載が無く、row3 と同じ判定ができない） | BUG-38/39 追補2（`known-bugs.md:4310-4322`） | NICOLA 同時打鍵の高速入力で romaji "de" が `cold=45→46→47` の3世代連続 `epoch-fence-stale`。2世代目までは自己修復したが**3世代目は give-up に落ち、再送なしで「で」が完全に消失** |
| 3 | 2026-07-29 | Windows Terminal（同上）、GJI | **`Tsf`（確定）**——ログの `[h1-warmup]`/`[gji-coro]` から F3 のとおり | BUG-45 | "kaきの"（F10） |

**row1 の意味（重要）**: `Windows.UI.Input.InputSite.WindowClass` は `AppImeProfile` 上は TsfNative だが、`AppKind` は `Uwp` であり既定の `injection_mode` は **`Unicode`** である（ADR-083 の「訂正された事実誤認」項目1 が指摘した、`AppImeProfile` と `AppKind` の軸のズレそのもの）。実行時学習で `Tsf` へ昇格していなければ、**これは `Unicode` モードでの give-up 記録**になる。F3 の cold/warm 2系統のどちらが emit したのかも未確定である（`Unicode` モードなら warm パスの `LiteralDetectFsm` が `!is_tsf_mode()` を満たすので install されうるが、`send_romaji_as_unicode` 経由で literal 検出まで到達する経路の有無は本 ADR では追跡していない）。**決定4-a が「モード別に数える」ことを求める以上、既存3件のうち2件が未分類であること自体を記録に残す。**

BUG-33 のソーク結果（2026-07-22、0 件）は依然として事実だが、**それは「Chrome + GJI で、通常使用中の、時間を区切らない観察」であって、`Tsf` モードのアプリを対象にした計測ではない**（そして F3 のとおり `Tsf` モードでも give-up は起きる）。

**帰結**: 「give-up はほぼ起きないから提案2 の期待値も低い」という推論は成立しない。記録上の実績は「**give-up は起きる。起きるとユーザーは文字を失う**」である。決定3 の却下は、この実害を承知のうえで「提案2 の前提が成立しないから」という理由で行う——**したがって代替策を示す責任がある**（決定3 の「却下する代わりに何をするか」、premortem P5）。

### F13: **`VK_IME_ON` 単発を warmup 目的で送る本番経路が既に存在する**——しかもそれは犠牲キーを伴う

`Output::send_unicode_cold_warmup_keys`（`output/mod.rs:312-344`）は本番コードで、次の2段を送る:

1. `VK_IME_ON`（0x16）を `IME_KANJI_MARKER` 付き・**scan=0** で送信（`make_key_input_ex`）。ログ `[unicode-cold-warmup] cold=N VK_IME_ON 送信 (ひらがなモード切替)`。直後に `ime_mode_fsm.on_f21_sent()`。
2. `VK_A` + `VK_BACK` を `INJECTED_MARKER` 付きで同一バッチ送信（**犠牲キー**）。ログ `[unicode-cold-warmup] cold=N VK_A+BS 犠牲キー送信 (gji_write_bytes 上昇待ち)`。

稼働経路は `platform.rs:513-524` の `start_unicode_cold_warmup`（Unicode long-cold かつ deferred chars あり）。その後 `UnicodeColdWarmupFsm`（`tsf/warmup/unicode_cold_warmup_fsm.rs`）が 10ms ごとに `gji_write_bytes()` を監視し、上昇するか `WARMUP_TIMEOUT_MS = 200` 経過で deferred chars を送る。

**この事実の解釈は慎重を要する。2通りの読み方があり、コードだけでは決着しない:**

- **読み方A（否定寄り）**: `send_unicode_cold_warmup_keys` の doc は犠牲キーの目的を「**`VK_A` が GJI の hiragana composition を起動して `gji_write_bytes` を増やし**、BS が即キャンセルする」と書いている。これは「`VK_IME_ON` を送っただけでは GJI の composition は起動しない（write_bytes は上がらない）」という前提の上に立つ設計と読める。だとすれば案A（`VK_IME_ON` 単発で cold-start を解消できるか）に対する**リポジトリ内に既にある否定寄りの証拠**である。
- **読み方B（中立）**: 犠牲キーは「GJI が起動したことを**観測**するための刺激」であり（`UnicodeColdWarmupFsm` は write_bytes の上昇を readiness 信号として使う）、`VK_IME_ON` 単独の warm 効果の有無を直接主張してはいない。`gji_write_bytes` が上がらないことと composition context が温まっていないことは、厳密には別の命題である。

**本 ADR はどちらとも断定しない。** ただし少なくとも「`VK_IME_ON` 単発では `gji_write_bytes` の上昇という観測可能な反応が得られなかったため、設計者は刺激をもう1つ足した」という事実は確実であり、**案A を「完全に未検証」と扱うのは誤りである**（初稿の誤り）。決定2 はこれを織り込み、決定4-e は既存ログでこの点を実測する。

### F14: eager warmup と reinit は**送信形態（`wScan` / `dwExtraInfo`）が異なる**——「キーを差し替えるだけ」ではない

2つの INPUT 生成関数は別物である。

| | eager warmup | reinit / unicode-cold-warmup |
|---|---|---|
| 生成関数 | `make_tsf_key_input`（`tsf/output.rs:110-112`）→ `make_scan_key_input`（`:83-105`） | `make_key_input_ex`（`tsf/output.rs:115-136`） |
| `wScan` | `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` の**実行時実値** | **`0` 固定** |
| `dwExtraInfo` | `TSF_MARKER` | `IME_KANJI_MARKER` |
| `KEYEVENTF_SCANCODE` | **付けない** | （scan=0 なので無関係） |

`make_scan_key_input` の doc（`tsf/output.rs:76-83`）は、この組み合わせが `99f56a2` → `2d4d85c` の**2段階の実機トライで決まった**ものであることを明記している（`KEYEVENTF_SCANCODE` を付けると WezTerm が `LLKHF_SCANCODE` 付きキーとして検出し IME をバイパスする）。

一方 scan=0 注入については、`docs/experiments.md` エントリ09（BUG-25、GJI entry の scan=0 `VK_DBE_ALPHANUMERIC` 注入）が「**フックにすら届かず反証**」として実験を1本まるごと潰している。

加えて `VK_IME_ON`（`vk.rs:25`、0x16）は物理キーに対応しないため、`MapVirtualKeyW(0x16, MAPVK_VK_TO_VSC)` が 0 を返す可能性がある（**Linux では検証不能、決定4-f で実測する**）。0 を返すなら「scan 付きの `VK_IME_ON`」は原理的に作れない。

**帰結**: 決定2 の実験の独立変数は VK 単独ではなく **(VK, `wScan`, `dwExtraInfo`) の組**である。これを事前に固定せずに実験すると、否定的結果が出たときに「`VK_IME_ON` が効かない」のか「scan=0 だから届かなかった」のかを事後に分離できない（エントリ09 の再演）。

### F15: 決定2 群C（eager warmup 完全無効化）の実機第1回測定（2026-08-22、dragonflyg4）——**不確定、参考記録**

`Output::send_eager_tsf_warmup` 冒頭に環境変数 `AWASE_DIAG_DISABLE_EAGER_WARMUP` による診断ゲートを追加し（3ゲート通過の**前**に配置）、Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、TsfNative、GJI）で実機テストした。cold=1〜8（`gji_idle_ms` 最大 85687ms=約85.7秒）全件で per-VK confirm が正常に confirmed へ到達し、`giving up` は0件だった。

**この回だけでは決定2に使えない**: ゲートを3ゲートの**前**に置いたため、`[diag] ... スキップ` ログは「3ゲートを通過して本来なら送信していたはずの送信を止めた」場合と「元々 `conv_mutation_allowed`/`needs_f2_probe()`/`can_warmup()` のいずれかで弾かれ、フラグが無くても送らなかったはずの呼び出し」の両方で同一に出力される（交絡）。したがって8回のうち何回が「本当に eager warmup を阻止した」ケースだったかを事後に分離できない。この欠陥は実機テストで発覚した。

### F16: 決定2 群C の実機第2回測定（2026-08-22、dragonflyg4）——診断ゲートを3ゲートの**後**へ移動し交絡を解消

`send_eager_tsf_warmup` の診断ゲートを `can_warmup()` チェックの**直後**（3ゲート通過後）へ移動し、`[diag]` ログの文言も「3ゲート通過後に…本来なら送信していたはず」へ変更した（`output/mod.rs:707-720`）。以後の `[diag]` ログは全て「本当に F2 送信を阻止したケース」のみを表す。

以下3シナリオそれぞれで最低1回、交絡のない成功例を確認した（いずれも Windows Terminal、GJI、TsfNative、`giving up`/`SuspectedLiteral` は全セッション通じて0件）:

| シナリオ | 該当する cold イベント | 結果 |
|---|---|---|
| IME OFF の状態からフォーカス復帰直後の入力（BUG-69 F3 が指摘する「唯一生きている actuation を止めた場合に一番危険」なケースを狙ったもの） | `cold=15`（restart後）、直前に `FocusChange` で `ime_on=false(shadow)` の `Windows.UI.Input.InputSite.WindowClass` へ復帰、直後 `reason=SetOpenTrue` | per-VK[0/0] confirmed → セッション確認 |
| 63秒放置後の入力 | `cold=20`、`gji_idle_ms=63094` | per-VK 2/2 confirmed → セッション確認 |
| 高速連続打鍵（BUG-45 が実際に踏んだ「NICOLA 同時打鍵の高速連続入力」を意識） | `cold=29`〜`cold=31`（14秒間に3回連続）、加えて偶発的に `cold=32`（`gji_idle_ms=82437`=約82.4秒）も同セッションで発生 | 4件とも全 VK confirmed → セッション確認 |

**この第2回の評価**: 3つの狙ったシナリオそれぞれで最低1回、交絡のないクリーンな成功データが得られた。ただし各シナリオ1〜4回に過ぎず、「決定2 項目1〜5 の合格基準」（3群比較、各シナリオ5回以上）には遠く届いていない。**未実施のまま残る**: 群A（現行F2）・群B（`VK_IME_ON` 単発）との比較そのもの（今回試したのは群Cのみ）、Chrome/Edge を含む `Vk` モードアプリでの検証（F1 のとおりそもそも対象外）、force_tsf 指定/学習昇格した別アプリでの検証。

**この経験から得た教訓**（`docs/experiments.md` エントリ16へも転記）: **対照群を作るための無効化ゲートは、既存ゲート（判定条件）の後に置くこと。** 前に置くと「意図的に止めた」と「元々発火しない」が同じログ行になり、事後に分離できない交絡を生む。無効化する前に、その機構が「今日・この環境で・本当に発火する」ことを同一ログで確認してから止める。

### F17: 決定4-f を実機測定した——`MapVirtualKeyW(VK_IME_ON=0x16, MAPVK_VK_TO_VSC)` は非ゼロ（0xF2）を返す

2026-08-22、dragonflyg4 で2通りの方法で測定した。

1. **standalone PowerShell から直接 P/Invoke**（awase を介さない）: `MapVirtualKeyW(0x16, MAPVK_VK_TO_VSC) = 0xF2 (242)`。
2. **`send_ime_mode_key` 内に診断ログを1行追加し、実際の送信直前（awase.exe 自身のスレッド・実行時キーボードレイアウト文脈）で測定**: GJI の IME ON/OFF 切替を複数回発生させ、`[diag-mapvk] MapVirtualKeyW(vk=0x16, MAPVK_VK_TO_VSC) = 0xF2 (242)` を複数回・一貫して観測。`VK_IME_OFF (0x1A)` は `0xF1 (241)`。

両方法で結果が一致した（0xF2）。**ただし方法1だけでは信頼できないことも同時に判明した**——同じ standalone テストで `VK_DBE_HIRAGANA (0xF2)` を引くと `MapVirtualKeyW(0xF2, MAPVK_VK_TO_VSC) = 0` を返したが、これは実際の hook ログが一貫して示す `scan=0x70`（`make_scan_key_input` 経由の自己注入送信、`extra=TSF_MARKER` で確認済み）と矛盾する。`MapVirtualKeyW`（Ex でない版）は呼び出しスレッドの実行時キーボードレイアウトに依存するため、standalone プロセスの文脈と awase.exe 本体の文脈で異なる値を返しうる。**VK_IME_ON については両文脈で一致したため今回は事なきを得たが、一般には standalone 測定だけを信頼してはならない。**

**帰結**: 決定2 の第0ステップ（N5）が要求する分岐が確定した。**第1候補（`VK_IME_ON`, scan=0xF2, `TSF_MARKER`）が成立する。** 群B実験はこの形態で組む。第2候補（scan=0, `IME_KANJI_MARKER`）へフォールバックする必要はない。

**傍証（未検証の偶然の一致、深追いしない）**: `VK_IME_ON` の scan 値 (0xF2) が `VK_DBE_HIRAGANA` 自身の仮想キー値 (0xF2) と数値的に一致している。これが Windows の日本語キーボードレイアウトテーブル内で IME 制御系仮想キーが相互参照されているためか、単なる偶然かは未確認。決定2 の実験結果の解釈に影響する可能性は低いと考えるが、群B が予期しない挙動を示した場合はこの一致を疑ってよい。

### F18: 群B（`VK_IME_ON` 単発、候補1の形態）を実機第1回測定（2026-08-22、dragonflyg4）——有望、サンプル数は依然不足

`send_vk_dbe_hiragana_pair` に環境変数 `AWASE_DIAG_EAGER_WARMUP_VK_IME_ON` による診断ゲートを追加し、送信 VK を `VK_DBE_HIRAGANA` から `VK_IME_ON` へ差し替えた（送信形態は F17 で確定した候補1のまま——`make_tsf_key_input` が都度 `MapVirtualKeyW` で scan を算出、`TSF_MARKER`。他のコードパス（`send_ime_mode_key`・`send_unicode_cold_warmup_keys` 等）は変更していない）。Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、TsfNative、GJI）で2ラウンド実施した。

| ラウンド | 該当 cold イベント | `gji_idle_ms` | 結果 | 画面表示（目視） |
|---|---|---|---|---|
| 1 | `cold=8`（`reason=SetOpenTrue`） | 15610（約15.6秒） | per-VK[0/0] confirmed → セッション確認 | 正しいひらがな（ユーザー目視確認） |
| 2 | `cold=9`（`reason=SetOpenTrue`） | 30250（約30.3秒） | per-VK[1/1]×2 confirmed → セッション確認 | 正しいひらがな（ユーザー目視確認） |

2ラウンドを通じて `cold=1`〜`cold=13`（`reason` は `SetOpenTrue`/`NativeF2Consumed`/`CtrlKeyBypass`/`ReinjectConfirmKey` の4種）が発生し、**`giving up`・`SuspectedLiteral` は一件も無かった**。`[diag-groupB] eager warmup sent vk=VkCode(22)`（0x16=`VK_IME_ON`）が計30回出現し、実際に `VK_DBE_HIRAGANA` ではなく `VK_IME_ON` が送られ続けたことも確認済み。

**傍証（F13/決定2測定項目6 に関連、未確定）**: `cold=7` の `VK_IME_ON` 送信直後（約1.9秒後）に `[gji-io] WRITE: w_ops=+3 w_KB=+584.0` という、通常の打鍵1回あたり（0.0〜1.3KB程度）から大きく外れる書き込みバーストを1件観測した。GJI 側の何らかの初期化処理に対応する可能性があるが、`VK_IME_ON` 送信との因果を確定する追加計測（同条件を複数回再現し再現性を見る等）は行っていない。

**評価**: 2026年5月に一度試されて撤回された `VK_IME_ON` warmup 実験（`48d25f2`→`3d49109`、当時「TSF composition context の初期化をトリガーしない」という実機観測で撤回）とは異なる結果になっている。ただし当時と今回で送信形態（scan の有無・付け方）が同一だったかは未確認のため、単純に「5月の結論が古かった」とは言えない。**サンプル数は決定2 の合格基準（各条件5試行以上、群A との対照比較）には届いていない**——今回は群Bのみを2回試しただけで、同一セッション・同一条件での群Aとの直接比較は無い。F16（群C）と同じ限界を持つ：交絡はないが、統計的に有意と言える段階にはまだ遠い。

---

## 決定

### 決定1: 提案1（eager warmup の `VK_DBE_HIRAGANA` → `VK_IME_OFF→VK_IME_ON` トグル置換）を**却下する**

理由は4点。**第2稿で (a) を頻度差のみの論法へ書き直し、(c) の強度を落とした。**

**(a) 「エラー時にだけ撃たれる回復手段」を「打鍵駆動の高頻度パス」へ移すことの危険が、実測された副作用で裏付けられている。**
F3 のとおり reinit は今日、Unicode long-cold と literal give-up という**エラー時・稀な契機**でしか撃たれていない（give-up 経路は `Tsf` モードでも発火するので「モードが違う」という初稿の論法は取り下げる。**残るのは頻度差である**）。対して eager warmup は確定キー・Ctrl 解放・再注入・物理 F2・フォーカス変更のたびに撃たれうる（F2）。そこへ移設すると、F7-1 の自己増幅ループ（`GetActiveProfile` 単発フリップ → warmup 戦略の丸ごと再生成 → cold 再突入 → 再 reinit）の発火機会が桁で増える。今日 `ImeKindDebounce` が塞いでいるのは「reinit が稀である」という前提の下でのフリップであり、その前提を外したときにデバウンス（同一種別2回連続で確定）が持ちこたえるかは未検証である。

**(b) 意味論が違う（この理由が最も強い）。**
`VK_DBE_HIRAGANA` 単発は composition を閉じない冪等操作（コード自身が「反復送信も無害」と書いている、`output/mod.rs:716-717`）。`VK_IME_OFF→VK_IME_ON` は composition を一度閉じる破壊的遷移であり、未確定 preedit を commit する（F7-2、BUG-36 で**確定**）。eager warmup の呼び出しサイトには `composition_confirm_key_up` / `composition_ctrl_up` / `on_reinject_key` のように **composition の直後でありうる**ものが含まれる（F2）。「まだ何も composition していないはずだから閉じても無害」という暗黙の前提が成り立たない。

**(c) confirm の実効性に保証が無い（強度を落とした）。**
提案の中核の主張は「confirm-then-transmit の方が fire-and-hope より頑健」だが、F5 のとおり IMC が読めない瞬間には confirm は無言で 300ms の空回りに劣化する。**BUG-45 に肯定事例が 1 件あるので「読めない」とは言えないが、「常に読める」保証も無い（再現性・頻度が未測定）。** 読めない瞬間の差分は「F2 を 1 発」→「OFF/ON を 2 発 + 300ms の非同期ポーリング + preedit commit リスク」という悪化になる。

**(d) 目的（BUG-50 系リスクの構造的除去）は正しいが、より安い手段が残っている。** conv 副作用を消すのに OFF は不要である（決定2）。

#### 検討したが見送った案

| 案 | 内容 | 判断 |
|---|---|---|
| A | `VK_IME_ON` の**単発**へ置換（OFF なし） | **見送らない。決定2 で実験対象とする。** これが ADR-098 決定3-c の原案そのものであり、(b)(d) の問題を持たない。ただし F13 のとおり「完全に未検証」ではなく、既存経路の設計が否定寄りの示唆を与えている。実機データ無しでは実装しない |
| **A'** | **`VK_IME_ON` + 犠牲キー（`VK_A`+`VK_BACK` 同一バッチ）** | **案A が不足だった場合の次手として保持する。** `send_unicode_cold_warmup_keys`（F13）に既存の形があり、ADR-048 の SacrificialWarmup（Chrome cold-start 対策、アトミックバッチで文字フラッシュを防ぐ）と同型。**案A が不合格でも「`VK_IME_ON` 系は駄目」と一般化してはならない**——この案が残る |
| B | `VK_DBE_HIRAGANA` と `VK_IME_ON` の両方を送る | 目的（`VK_DBE_HIRAGANA` の conv 副作用の除去）を達成しない。送信数だけ増える |
| C | eager warmup 自体を撤去する | ADR-098 決定3 が KEEP と決定済み。撤去すると `Tsf` モードのアプリ（`force_tsf` 指定 + 学習昇格分）で cold-start リテラル化が再燃する。**なお ADR-098 決定3 の原文（`098-*.md:377`）が被害例として挙げた「BUG-02（Chrome）のリテラル化」は誤りである**——F1 のとおり Chrome は eager warmup 対象外であり、撤去しても Chrome には影響しない。本 ADR でこの点を訂正する（初稿はこの誤りを ADR-098 から継承していた） |
| D | feature flag / 環境変数で新旧キーを切り替えて実機比較する | 消費ロジックの無い予備実装を避ける方針（撤回済み `CharsetSlot` / F22-F24 予備バインドと同型）。実験は一時ビルドで行い、結論が出た側だけを本流に入れる |
| E | eager warmup の直前に実 IME 状態を読んで BUG-15 追補7 の安全則を満たす | ADR-098 決定3 が既に否定（TsfNative では `FeedbackPolicy::Blind` により open 軸を読めない、F4）。加えて同期読み取りは BUG-34 が撤去対象としている |
| **F'** | **eager warmup の送信頻度自体を下げる（デバウンス／クールダウン）** | **却下するが理由を記録する。** (a) が「頻度が高いから危険なキーへ変えられない」と論じている以上、頻度を下げるという直交した選択肢は検討に値する。`eager_warmup_sent_ms` は既に記録されているので「前回送信から N ms 以内ならスキップ」は小さな変更で試せる。**却下理由**: (i) 現行キー（`VK_DBE_HIRAGANA` 単発）は冪等で、頻度が高いこと自体の実害が観測されていない——実害が無いものにレート制限を足すのは、BUG-16 が踏んだ「スパムガードが本来必要な再試行まで殺す」失敗の再現リスクがある。(ii) N の値に実測根拠が無く、`.claude/rules/tuning-constants.md` の実測義務を満たせない（何 ms なら安全かを測る手段が無い）。(iii) F4 の残存ハザード（実 IME が ON でないときの注入）の露出回数は減るが、露出そのものは消えない——構造的解決は案A/A' 側にある。**ただし案A/A' が採用され、かつ新キーの副作用が頻度依存だった場合には、この案が再浮上する**（そのとき初めて N の実測を行う） |

#### なぜ却下が安全か

現状維持である。eager warmup は ADR-098 決定3 が KEEP と決めた形（3ゲート + `VK_DBE_HIRAGANA` 単発）のまま動き続ける。F4 の残存ハザードは ADR-098 が既に「受容中の既知リスク」として明示的に引き受けている。

### 決定2: 案A（`VK_IME_ON` 単発への置換）を**採用する**（2026-08-22、実装済み）

ADR-098 決定3-c と `docs/experiments.md` エントリ16 を、より具体的な測定計画へ格上げしたうえで、群Bの実機検証結果（F17・F18）をもってユーザー判断により正式採用へ格上げした。

**実施状況（2026-08-22 追記、F15〜F18）**: 下記の群A/B/C比較のうち**群C（eager warmup 完全無効化、F15・F16）と群B（`VK_IME_ON` 単発、F17・F18）**を実機（dragonflyg4、Windows Terminal）で測定した。群Cは第1回が診断ゲートの位置による交絡で無効（F15）、第2回（交絡解消後、F16）で3シナリオそれぞれ最低1回のクリーンな成功例を確認。群Bは候補1の形態（scan実値+`TSF_MARKER`、F17で確定）で2ラウンド実施し（F18）、`giving up`/literal 化は一件も無かった。**いずれも決定当時は「有望」の段階で、下表が要求する「各条件5試行以上・3群を同一セッションで比較」には届いていなかった。群A（現行F2）を基準にした直接比較も行っていない。**

**採用判断（2026-08-22）**: 上記の限界を認識したうえで、ユーザー判断により群Bの結果（15.6秒・30.3秒放置を含む cold=1〜13、`giving up`/literal 化0件、画面表示も目視確認）を根拠として `send_eager_warmup_vk_pair`（旧 `send_vk_dbe_hiragana_pair`）の送信 VK を `VK_DBE_HIRAGANA` から `VK_IME_ON` へ本採用した（`crates/awase-windows/src/tsf/send.rs`・`output/mod.rs`）。目的（BUG-50 型副作用の構造的除去）は達成される。**厳密な3群比較・大サンプルでの統計的検証は行っていないことを明記する**——将来 cold-start 系の不具合が再発した場合、この採用判断を疑ってよい経路として記録しておく（`docs/known-bugs.md` BUG-50 追補2 参照）。群C（eager warmup 完全無効化）は本採用の対象外——BUG-69 依存の懸念が残るため、別途の検討課題として ADR-100 に残す。

#### 実験の独立変数を先に固定する（F14）

差し替えるのは VK だけではない。**(VK, `wScan`, `dwExtraInfo`) の組を1つに固定してから実験する。**

- **第1候補**: `(VK_IME_ON, MapVirtualKeyW 実値, TSF_MARKER)` — 現行 eager warmup と**送信形態を揃え**、VK だけを変える。独立変数が1つになるので結果の解釈が一意になる。**ただし `MapVirtualKeyW(0x16)` が 0 を返すならこの候補は成立しない**（決定4-f で先に測る）。
- **第2候補（第1が不成立の場合）**: `(VK_IME_ON, 0, IME_KANJI_MARKER)` — `send_unicode_cold_warmup_keys` / reinit と同じ形態。**この場合、否定的結果が出ても「`VK_IME_ON` が効かない」とは結論できない**（scan=0 で届かなかった可能性が残る。エントリ09 の前例）。実験ログにその限界を明記すること。

#### 測定項目（3群比較にする）

初稿の「置換前後の2群」では、「置換して literal 化しなかった」から「`VK_IME_ON` が効いた」と「そもそもその条件では warmup 無しでも literal 化しなかった」を区別できない。**対照群 (C) を入れる。**

| 群 | 対象コードパス | 送るもの |
|---|---|---|
| A | eager warmup（`send_vk_dbe_hiragana_pair`） | 現行 `VK_DBE_HIRAGANA`（scan 実値 + `TSF_MARKER`） |
| B | 同上 | `VK_IME_ON`（第0ステップで固定した形態） |
| C | 同上 | **何も送らない**（eager warmup を無効化） |
| **B'** | **unicode-cold-warmup（`send_unicode_cold_warmup_keys`）** | **`VK_IME_ON` のみ（犠牲キー `VK_A`+BS を外す）**。決定4-e が求める「`gji_write_bytes` の上昇が `VK_IME_ON` 由来か `VK_A` 由来か」の分離用（N8）。**群A/B/C とは別のコードパスであり、同じ一時ビルドで測れるが測定手順は独立している** |

**第0ステップ（N5）**: 実験ビルドを立てたら、群A/B/C を回す前に **決定4-f（`MapVirtualKeyW(VK_IME_ON=0x16, MAPVK_VK_TO_VSC)` の戻り値）を1行ログで測り、その場で第1候補（scan 実値）か第2候補（scan=0）かを確定させる**。これを後回しにすると、群B を第1候補のつもりで組んだのに実際は scan=0 が入っていた、という取り違えが起きる。

**群C の実施条件（N4）**: 群C は eager warmup を無効化した一時ビルドであり、**実験中は意図的な退行状態**である（ADR-098 決定3 が eager warmup を cold-start 対策として KEEP している以上、群C の実行中ユーザーは BUG-02 系リテラル化に晒される）。**短時間・意図した条件下でのみ実施し、通常使用のまま放置しないこと。** 推奨プロトコル: 各群について「long idle（10秒以上）→ 1文字打鍵 → 出力を記録」を1試行とし、idle 秒数を変えて各条件 5 試行以上。群C は連続使用ではなくこの試行単位でのみ動かす。

各群で次を記録する:

| # | 測ること | 合格基準 |
|---|---|---|
| 1 | アプリ・ウィンドウクラス・`injection_mode`・long idle 秒数 | 記録必須（比較の前提。群 A/B/C で条件が揃っていること） |
| 2 | **初回打鍵の出力文字列**（conv ビットではなく実際に出た文字） | 群 A と群 B が一致すること。**「conv が変わらないこと」ではなく「入力結果の文字列が一致すること」で取る**（P1 参照。「今まで寄せてくれていたものが寄らなくなる」という形の退行を conv 観測では捉えられない） |
| 3 | リテラル化の発生有無 | 群 B が群 A と同等。**群 C との差が無ければ、その条件では eager warmup 自体が効いていない**（＝ ADR-098 決定3 の KEEP 判断の前提も同時に検証される） |
| 4 | conv モード（かな/ローマ字/カタカナ）への影響 | 群 B で `VK_DBE_HIRAGANA` の「ひらがなに強制する」副作用が消えること（置換の本来の目的） |
| 5 | BUG-50 系デッドロック（カタカナロックイン）の再現有無 | 群 B で再現しない |
| 6 | `VK_IME_ON` 送信後に `gji_write_bytes` が上昇するか | F13 の読み方A/B を決着させる。上昇しないなら案A' へ進む |

群 C を入れる意義は ADR-100 の範囲を超える: **ADR-098 決定3 の「eager warmup は唯一生きている実効的な cold-start 対策」という判断も、対照群を持たないまま述べられている。** 本 ADR が実験計画へ格上げする以上、ここで対照を入れるのが自然である。

#### 記録義務

`.claude/rules/experiment-logging.md` に従い、アプリ（クラス名込み）・IME（GJI、Engine ON/OFF、cold/warm、idle 秒数）・再現手順を記録する。項目3 が不合格で撤回する場合は revert コミット本文にこの3点を書く。

#### タイミング定数について

**新規のタイミング定数は導入しない。** 初稿は「`VK_IME_ON` 単発は `VK_DBE_HIRAGANA` 単発と送信形態が同じなので定数変更は発生しない」と書いたが、**F14 のとおり送信形態は同じではない**ので、この理由付けは撤回する。正しい理由は「本実験は待機時間を一切変更しないから」である。もし項目3 で「`VK_IME_ON` は効くが反映が遅い」という結果になり待機の追加が必要になった場合は、**その待機が何 ms 必要かを実測してから**別途起票する（実測なしのエスカレーションは禁止）。

### 決定3: 提案2（give-up 分岐に reinit 完了確認後の retry を追加）を**却下する。ただし代替として案L（journal 記録）を実施する**

**2026-08-24追補:** この却下は「当時の `send_chrome_gji_reinit_and_poll` に完了通知・
focus世代照合・送信後処理・順序保証が無い状態で retry する案」への却下である。
BUG-74対応の [ADR-101](101-bug74-giveup-retry-with-focus-guard.md) は、決定5/F6を
先に修正し、`WM_GJI_REINIT_RETRY_COMPLETE`、`PendingGjiReinit`、Polling中の
`pending_deferred` 抑止、`SuppressedExistingPoll` の raw cleanup 抑止、retry上限を
追加したうえで、通常送信経路による1回だけの retry を実装した。

「retry という発想が悪い」からではない。**「reinit 完了確認」という前提が現状のコードでは成立しないから**である。4点。**第2稿で (b) の強度を落とし、代替策を追加した。**

**(a) 完了通知の経路が存在しない（この理由が最も強い。コードで確認済み、反論の余地なし）。**
`send_chrome_gji_reinit_and_poll` のポーリングは `win32_async::spawn_local` の fire-and-forget であり（`probe_io.rs:208-253`）、Hiragana を確認できても `break` してログを出すだけで、誰にも通知しない。retry を配線するには (i) 完了コールバック、(ii) `cold_seq` の照合、(iii) フォーカス世代の照合（**F6 の欠落を先に埋める必要がある**）、(iv) 「確認できた」と「タイムアウトした」の呼び分け、の4点を新設する必要がある。これは「既存機構の再利用」ではなく新機構の追加である。

**(b) 「確認できない」瞬間が黙って「300ms 経過」に劣化する（強度を落とした）。**
IMC が読めない瞬間には confirmed が立たないままポーリングが終わる（F5）。retry のトリガーがそのとき実質「300ms タイマー」になるなら、それは confirm-then-retry ではなく、BUG-27 追補2 が msedge で実機破壊した無条件 retry に時間差をつけただけである。**BUG-45 に肯定事例が 1 件あるので「読めない」と決めつけることはできないが、読める割合が未測定である以上、confirm を retry のゲートとして使う設計は成立を保証できない。**

**(c) reinit 自体が skip されうる（F11）。** 連続 give-up の間隔が 300ms 未満なら reinit が送信されない。「reinit 完了確認後」の定義が skip 時に未定義になる。skip でも retry するなら (b) と同じ問題に帰着する。

**(d) BUG-45 が未解決のまま、同じ経路の送信を増やすことになる。** BUG-45 は「この経路には actual を確認してから次の一手を決める箇所が一つもない」ことを問題の核心として記録しており、推奨する方向 (b) は「実際に画面へ literal 文字が出たことを確認してから backspace する」＝**確認を増やす**方向である。提案2 は同じ belief の連鎖の上に送信をもう1段積む。

#### 却下する代わりに何をするか（案L を実施する）

F12 のとおり、give-up 到達時に romaji が失われるのは推測ではなく**3件記録済みの実害**である。「却下」だけで終えると、次に同じ報告が来た担当者は代替手段を持たないまま「とりあえず再送してみる」パッチ（案F そのもの、BUG-27 追補2 の再演）に手を伸ばす（premortem P5）。

**案L: give-up 分岐で捨てた `romaji` を journal（ADR-096 のレーン）へ記録する。**

- 送信を1つも増やさないので BUG-45 の推奨方向 (b) と競合しない。
- F8 のとおり、現状 give-up 側の `log::warn!`（`probe_io.rs:664-669`）は捨てた `romaji` の値を含んでいない（`consecutive == 0` 側にだけ `{romaji:?}` がある）。**失われた文字が実機で何だったかを、ログからも journal からも復元できない。**
- ADR-096 の literal-detect journal 記録 `LiteralDetectRecord`（定義は `tsf/literal_facts.rs:58-66`、フィールドは `cold_seq` / `facts` / `consecutive_before` / `gave_up` / `backs` / `escape_composition` / `session_marked`）には既に give-up の事実が入っている。journal への配線は `platform.rs:204/248` → `journal.rs:210-214`。**ここに捨てた romaji を1フィールド足すのが最小の変更である。**
- **成立条件は確認済み**: `LiteralDetectRecord` は `#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]`（`tsf/literal_facts.rs:57`）で **`Copy` ではない**（同ファイルの他5型は `Copy` を derive しているので、ここは確認する価値があった）ため `String` フィールドを足せる。journal 側 `JournalEvent::LiteralDetect { record, .. }`（`journal.rs:210-214`）は record を**所有**で持つので可変長データでも壊れない。ADR-096 のレーンが固定長レコードを要求する構造だったら案L は成立しなかった。

#### 案L の作業範囲（ラウンド2 N2 を受けて精密化。着手前に設計判断を確定させておく）

**フィールドを1つ足すと、`LiteralDetectRecord` の構築サイト全数がコンパイルエラーになる。** grep 実測で**本番5箇所 + テストヘルパー1箇所**（ラウンド2 レビューは4箇所と見積もったが、実際にはこれより2箇所多い）:

| # | 場所 | verdict / 用途 | romaji を持つか |
|---|---|---|---|
| 1 | `output/probe_io.rs:629`（`RawTsfLiteralRecovery` ハンドラ） | give-up / 初回疑い | **持つ**（唯一の記録元） |
| 2 | `output/probe_io.rs:688`（`CompositionConfirmed` ハンドラ） | 合成成功 | 持たない |
| 3 | `output/probe_io.rs:711`（`LiteralDetectNote` ハンドラ） | 記録のみ | 持たない |
| 4 | `output/probe_io.rs:374`（`plan_skipped_record`） | `PlanSkippedLiteral` | 持たない |
| 5 | **`crates/awase-windows/src/platform.rs:204-210`**（`flush_pending_literal_vk_as_aborted`） | `AbortedNoVerdict` | 持たない。**`probe_io.rs` の外にあるため grep しないと気づかない** |
| 6 | `journal_policy.rs:233`（`literal_record` テストヘルパー、`#[cfg(test)] mod tests` 内） | テスト | 持たない |

**設計判断（着手前に確定させる）**:

- **型は `Option<String>`** とする。`String::new()` を5箇所に撒くと「空文字＝記録し忘れ」と「空文字＝そもそも romaji を持たない verdict」が区別できなくなり、集計時に誤読を招く。`None` なら「この verdict は romaji を持たない」が型で表現される。
- **push 時に `clone` が要る。** give-up 分岐の trace push は `if consecutive == 0` の**前**（`probe_io.rs:627-637`）にあり、その直後の `io.set_raw_literal(backs, romaji, escape_composition)`（`:645`）が `romaji` を **move** する。両分岐（初回疑い / give-up）で記録するなら push 時点で `romaji.clone()` が必要。romaji は1文字分のローマ字（数バイト）なので clone のコストは無視できる。

**「最小の変更」という評価自体は維持する**——型・journal・テストの3条件をすべて満たし、送信も挙動も一切増やさない。変わったのは作業範囲の見積もりだけである。

#### 案L のプライバシー方針（ラウンド2 N3。**実装前に決めておく**）

案L が journal に載せるのは、literal 化して失われた romaji ——すなわち**ユーザーが実際に打とうとした文字列**である。journal は ADR-095/096 の不具合報告機構で Cloudflare Worker へ送信されうる。

**採る方針: (1) 生の romaji を入れる。ADR-095 の既存 opt-in に委ねる。**

根拠:

- **journal は既に `attach_log` チェックボックスの opt-in 配下にある。** `bug_report.rs:214-220` で `log_excerpt`（＝構造化 journal の `journal_json`）は `if input.attach_log` でのみ payload に載り、同じチェックボックスが生ログ `app_log_excerpt`（`:221-227`）も制御する（`bug_report.rs:142-145` の doc に「`attach_log` チェックボックスで両方まとめて制御する」と明記）。**案L は新しい送信チャネルを開かない。**
- **同じ情報は既に生ログ側に出ている。** `consecutive == 0` 側の `log::warn!`（`probe_io.rs:639-644`）が `{romaji:?}` を出力しており、`awase.log` の tail は同じ `attach_log` で添付される。案L が変えるのは「give-up 側でも記録されること」と「機械可読になること」であって、打鍵内容が報告に含まれうるという性質そのものは既存である。
- **(2) 長さと文字種だけ**では決定3 の目的を満たさない。BUG-45 の "kaきの" 級の解析は「どの romaji が、どの文字に化けたか」の対応が要る。`len` + `is_ascii` では復元できない。
- **(3) 生を入れて報告組み立て時にマスク**は、`attach_log` が既に同じ生 romaji を含む `app_log_excerpt` を通しているため、journal だけマスクしても実効的な保護にならない（一貫しない防御は誤った安心を生む）。

**この方針が変わる条件**: journal が `attach_log` とは別の常時送信チャネル（テレメトリ等）に載る設計変更が起きた場合、本判断は無効になる。そのときは (3) を再検討すること。**「journal は opt-in の添付物である」という前提に依存した判断である**ことを明記しておく。
- ADR-095 の不具合報告に自動で載るため、次回の報告時に解析材料が揃う。

**これは決定3 の一部として実施する**（実装を伴うが、挙動変更ゼロ・送信ゼロ）。

#### 検討したが見送った案

| 案 | 内容 | 判断 |
|---|---|---|
| F | reinit 完了を待たず give-up 分岐で無条件に romaji を再送する | 却下。F9 の失敗モードそのもの。`consecutive` が 0 に戻らない環境で全打鍵に適用される |
| G | give-up 分岐から `send_eager_tsf_warmup` を再試行する（BUG-32 の残課題そのもの） | スコープ外。BUG-32 の残課題は「Win キー押下中で warmup がスキップされ IME-ON 信号が一度も届かない」という**特定の**失敗モードの回収であり、`send_vk_dbe_hiragana_pair` が `None` を返したことを記録して後で再試行する方が的確（give-up 分岐は無関係な失敗モードも通る）。別 ADR / 別 BUG として扱う |
| H | give-up の閾値（`consecutive != 0`）を緩め、3回連続まで再送を続ける | 却下。「何回まで再送するか」を実測なしに動かすことになる。BUG-27 追補2 の失敗は閾値ではなく検出器の信頼性の問題だった |
| I | `RAW_TSF_LITERAL_DETECT_MS`（300ms / long idle 500ms）を延ばして give-up 自体を減らす | 却下。対症療法。`.claude/rules/tuning-constants.md` が名指しで戒める「同じ定数ファミリーの盲目的エスカレーション」に該当する。延ばす前に「何 ms 必要か」の実測が要る |
| **J** | **give-up 時に、その文字だけ Unicode 直接送信へ退避する** | **却下しない。次に検討すべき第1候補として保持する。** `send_unicode_char_direct`（`probe_io.rs:259`）と `FlushDeferredUnicodeChars` の機構が既にある。**BUG-45 のログ自身が「reinit 後 "ki" は `gji_settled=true` で unicode transmit 経由、正常に「き」として出力」と記録しており、literal 化が続く状況で unicode 経路が機能した実例がある。** VK 再送を増やさず、confirm も要らず、BUG-45 の推奨方向とも競合しない。**本 ADR で採用しない理由**: (i) Unicode 直接送信は GJI の composition を経由しないため、「1文字だけ IME を迂回する」ことが変換候補の文脈（未確定文字列）とどう干渉するかが未検証（`.claude/rules` に記録された「Unicode 注入は GJI 確認を迂回する」という既知の性質）。(ii) 決定4-a の頻度データが無い段階で新経路を足す判断はできない。**決定4 のデータが揃った時点で最優先で評価すること** |
| **K** | **give-up で backspace も打たない（無回収 `return`）** | **却下しない。案J と並ぶ候補として保持する。** BUG-27 追補3 が「無リカバリの `return` のままで実害はほぼ無くなる見込み」と結論した路線の give-up 版であり、BUG-45 の恒久対策 (b)（「実際に画面へ literal 文字が出たことを確認してから backspace する」＝確認できないなら backspace しない）と方向が一致する。初稿は「送信を増やす案」だけを検討し「**送信を減らす**」方向を1つも挙げていなかった。**本 ADR で採用しない理由**: backspace を打たないと literal 文字が画面に残る（BUG-45 の "ka" がまさにそれ）。「消えるより残る方がマシか」はユーザー体験の判断であり、実機での頻度と見え方のデータ無しには決められない。決定4-a と併せて評価する |
| **L** | **give-up で捨てた romaji を journal に記録する** | **採用**（上記） |

### 決定4: 実装の前に**先に測る**（大半は実装 0 行で今日から取れるデータ）

決定1・決定3 がいずれも未測定量に依存して強度が変わるため、既存ログで取れるものを先に取る。

- **4-a: give-up の実機発生頻度を、アプリ別・`injection_mode` 別に数える。**
  `[raw-tsf-literal] ... giving up` の出現回数と、そのときのアプリ・クラス名・`injection_mode`・IME・idle 秒数。**初稿の「0 件に近ければ提案2 の期待値もほぼ 0」という判定基準は F12 の訂正により撤回する。** 記録上 give-up は起きるので、測るべきは「起きるかどうか」ではなく「どのアプリ・どのモードで、どの頻度で起きるか」である。`Vk`（Chrome/Edge）と `Tsf`（WezTerm / Windows Terminal）を分けて数えること。これが案J・案K のどちらを先に評価するかの判断材料になる。
- **4-b: reinit 後の IMC poll が confirmed へ到達する割合（「原理の検証」ではなく「頻度の測定」）。**
  既存ログで判別できる: `[chrome-reinit] cold=N Hiragana 確認 → ポーリング終了` が出れば読めている、`[chrome-reinit] cold=N ポーリング完了: total_write_delta=...` だけで終われば 30 回空回りしている。**F5 のとおり肯定事例は 1 件既にあるので、測るのは到達率とアプリ依存性である。** 到達率が高ければ決定1-(c) / 決定3-(b) は弱まるが、決定1 は (b)、決定3 は (a) で維持される。
- **4-c: eager warmup の実対象アプリ。** `[tsf-eager-warmup] VK_DBE_HIRAGANA 送信` と `[injection-mode] ... → Tsf 事後昇格` の出現。決定2 の前提（項目1）。**warmup が実際に飛んでいるアプリが無ければ、決定2 の実験自体が空振りになる。**
- **4-d: `[chrome-reinit] ... GJI write_bytes 上昇検出` の出現。** OFF→ON トグルが GJI 側に実際に届いているかの独立証拠（IMC が読めなくても取れる）。
- **【4-b / 4-d の集計上の注意（N7）】`cold_seq` では reinit の由来を区別できない。** `send_f22_f21_reinit` は `send_chrome_gji_reinit_and_poll(Generation::INITIAL)` を渡す（`output/mod.rs:899`）ため、**Unicode long-cold 由来の reinit ログは `cold=` が常に INITIAL 値になる**。give-up 由来（実 `cold_seq` を渡す）と混ざるので、集計時は `cold` 値ではなく直前に出ている `[unicode-cold-warmup]` / `[raw-tsf-literal] ... giving up` のどちらが先行するかで由来を判別すること。
- **4-e: 既存の `VK_IME_ON` 単発経路の効き（F13 の読み方A/B の決着）。**
  `[unicode-cold-warmup] cold=N VK_IME_ON 送信` の後、`UnicodeColdWarmupFsm` が `gji_write_bytes` 上昇を検出するまでに何 ms かかったか（`WARMUP_TIMEOUT_MS = 200` でタイムアウトしたか）。**ただしこの経路は `VK_IME_ON` の直後に犠牲キー `VK_A` も送るため、上昇が `VK_IME_ON` によるものか `VK_A` によるものかは現行ログでは分離できない。** 現行ログから分かるのは「両方送った場合に何 ms で上がるか」までである。**分離は決定2 の群B'（`send_unicode_cold_warmup_keys` から犠牲キーを外す）で行う**——群A/B/C とは別のコードパス（eager warmup ではなく unicode-cold-warmup）なので、同じ一時ビルドで測れるが測定手順は独立している（N8）。
- **4-f: `MapVirtualKeyW(VK_IME_ON=0x16, MAPVK_VK_TO_VSC)` の戻り値。** 1行のログで取れる。0 なら決定2 の第1候補（scan 付き `VK_IME_ON`）が成立しないので、第2候補へ切り替える。**これは「あとで測ればいい」項目ではなく、決定2 の実験ビルドの第0ステップである**（N5）——群A/B/C を回す前に測り、その場で候補を確定させること。
- **4-g: reinit のレート制限 skip 頻度（F11）。** `[chrome-reinit] cold=N skip: 前回 reinit から ...` の出現回数。連続 give-up で実際に skip が起きているかを確認する。

4-a〜4-d・4-g は既存のログ出力であり**新規コードは不要**。4-e の一部と 4-f は一時ビルドが要る。いずれも ADR-095/096 の不具合報告機構（journal スナップショット）でも収集できる。

### 決定5: F6（`send_chrome_gji_reinit_and_poll` のフォーカス世代ガード欠落）を独立した欠陥として `docs/known-bugs.md` に記録する

**2026-08-24追補:** F6はADR-101 Stage1で実装済み。`send_chrome_gji_reinit_and_poll`
はreinit予約時点の `ime_mode_focus_gen` を受け取り、IMC poll の判定時に現在世代と
照合する。不一致または `with_app` 再入失敗は `Stale` として扱い、IME FSM を汚染せず
retryも行わない。

本 ADR の提案とは独立に存在する既存の穴であり、かつ提案2 を将来配線する場合の前提条件でもある。

記録すべき内容:

- `ime_mode_focus_gen` を捕捉・照合する3経路（F6 の表）のうち、`send_chrome_gji_reinit_and_poll` だけが照合しないこと。
- `update_ime_mode_from_imc` が無条件に `confirmed=true` を立てること（`update_ime_mode_hint_from_imc` との差）。
- `on_ime_mode_focus_changed`（`output/mod.rs:392-396`）の doc がまさにこの保護を目的として世代を上げていること。
- **誰が誤導されうるか（m10）**: F4 で全数列挙した `ImeModeFsm` 読み手のうち、stale な `confirmed` で誤動作しうるのは次の4つ。具体的な誤導シナリオは「GJI アプリで reinit ポーリング中に MS-IME アプリへフォーカスが移り、旧ウィンドウの conv 値で `confirmed` が立って MS-IME gate の defer が早期解除される」。

  | 読み手 | 誤動作の形 |
  |---|---|
  | `vk_send.rs:370`（`ms_ime_gate_defer` 判定） | `is_native_ready()` が偽で true になり、MS-IME が未準備のまま romaji を送る（BUG-13 の再燃） |
  | `probe_io.rs:306`（MS-IME ready poll） | `MsImePollStatus::Ready` の誤判定 |
  | `output/mod.rs:812-816`（Unicode literal observer の install 条件 `Hiragana \| Katakana`） | observer が install される／されないの誤り |
  | `ms_ime_ready_coro.rs:68-76`（`TsfEnvSnapshot.ime_mode_confirmed` 経由） | コルーチンの待機解除が早すぎる |
- **retry を配線する場合は本欠陥の修正が前提条件である**旨（premortem P2）。

**本 ADR では修正しない**（挙動変更を伴い実機ソークが必要。かつ「ポーリング中のフォーカス変更で confirmed が立つ」ことが実害を出した実機ログはまだ無い）。修正するなら `start_ms_ime_ready_poll` と同型の gen 照合を入れて `Stale` で黙って終了する形になる。

### 決定6: `FeedbackPolicy` の doc コメントに「open 軸限定」であることを明記する（挙動変更ゼロ）

F4 のとおり `Read`/`Blind` は open 軸（`ImmGetOpenStatus`）の読み戻し可否のみを指し、conv 軸（`IMC_GETCONVERSIONMODE`）の読み取りには一切言及も制約もしていない。この読み取りにくさが実際に誤読を生んだ（本 ADR 起票時のブリーフィング資料が「矛盾するのではないか」と疑問を立てた）。

`state/ime_actuation.rs` の `FeedbackPolicy` 定義に以下の主旨を追記する:

- `Blind` は「open 軸の読み戻し手段が構造的に無い」宣言であり、**Win32 読み取り API の呼び出しを禁止するフラグではない**。実際にゲートしているのは (1) 試行回数の有界化、(2) `ReadBackQuery` の種別、(3) `OpenWarrant` Step 4c の解禁の3点だけである。
- conv 軸（`IMC_GETCONVERSIONMODE`）の読み取りは `Blind` プロファイルでも行われている（`send_chrome_gji_reinit_and_poll` の IMC ポーリング、idle-conv-check）。これらは `ImeModeFsm` / `ConvBitsInference` という別系統へ流れ、open 軸の belief には触れない。
- **open 軸と conv 軸の非対称は「未整理」ではなく「別の実測に基づく別設計」である（m14）。** open 軸は `can_read_imm32_open_status`（profile で弾く）方式、conv 軸は BUG-55（2026-08-07）で `GetForegroundWindow` → `get_focused_hwnd()`（`GetGUIThreadInfo().hwndFocus`）へ変更し、**hwnd の取り方で TSF ネイティブに対応する**方式を採った（理由コメント `ime.rs:421-431` が Windows Terminal の InputSite 子ウィンドウを名指ししている）。この経緯を doc に添える。

### 決定7: `send_f22_f21_reinit` の doc と実装の乖離を訂正する（挙動変更ゼロ）

`output/mod.rs:895-898` の doc は「Chrome の `send_chrome_gji_reinit_and_poll` と同じ VK_IME_OFF→VK_IME_ON シーケンスだが、async IMC ポーリングは行わない」と書いているが、実装（`:899`）は `send_chrome_gji_reinit_and_poll(Generation::INITIAL)` を丸ごと呼んでおりポーリングも走る。doc を実装に合わせる。

あわせて次の2点を doc に記す:

- `send_chrome_gji_reinit_and_poll` の `chrome_` という名前が misnomer であること（本番呼び出し元は Unicode モードと literal recovery の2つで、Chrome 固有ではない。**give-up 経路は `Tsf` モードでも発火する**、F3）。**改名はしない**——`tuning.rs` / `known-bugs.md` BUG-33/36/45 / テスト assertion が全てこの名前で相互参照しており、改名は grep 追跡性を一時的に壊すコストの方が大きい。
- **`send_f22_f21_reinit` は `send_chrome_gji_reinit_and_poll` を呼ぶため、`last_gji_reinit_ms` によるレート制限（`probe_io.rs:169-180`、300ms）を give-up 経路と共有する（m13）。** すなわち Unicode long-cold の reinit と literal give-up の reinit は互いを 300ms 抑止しうる。F11 の理解にも効く。
- **`send_f22_f21_reinit` は `cold_seq` に `Generation::INITIAL` を固定で渡す（`output/mod.rs:899`）ため、この経路の `[chrome-reinit] cold=N ...` ログは常に同じ値になる（N7）。** give-up 由来の reinit（実 `cold_seq` を渡す）とログ上で `cold` 値では区別できない。決定4-b / 4-d でログを集計する際の注意点として doc に残す。

---

## `docs/experiments.md` エントリ16 の扱い

**提案: エントリ16 は置き換えず、追記で拡張する。新エントリ17 は作らない。**

理由: エントリ16 は「事前登録」であり、その事前登録の中核（案A = `VK_IME_ON` 単発が cold-start 対策として代替になるか）は本 ADR で却下されていない——むしろ決定2 で正式な実験計画へ格上げされる。ここで新エントリを立てて OFF→ON トグル版の却下だけを別扱いにすると、次に誰かが「eager warmup を IME_ON 系のキーに変えよう」と考えたときに、エントリ16 だけを読んで「OFF→ON トグルは検討済みで却下された」ことに辿り着けない。`.claude/rules/experiment-logging.md` が戒める「なぜ前回それを捨てたのか がコミット/記録から辿れない」状態そのものである。

追記する内容:

1. **合格基準表の対象アプリ欄の訂正（F1）**: 「WezTerm/Windows Terminal で」は正確には「`InjectionMode::Tsf` になっているアプリで」であり、それは config `force_tsf` + 実行時学習で決まる可変集合である。**Chrome は eager warmup の対象外**（`AppKind::TsfNative` → `InjectionMode::Vk`。しかも `UnicodeLiteralObserverFsm` が `Unicode` モード限定なので実行時学習でも昇格しない）。実験前に `[tsf-eager-warmup]` ログで実対象を特定すること。
2. **背景欄の訂正（M5）**: エントリ16 の背景は ADR-098 決定3 由来だが、**ADR-098 決定3 が eager warmup 撤去の被害例として挙げた「BUG-02（Chrome）のリテラル化」自体が誤りである**（項目1 と同じ理由）。放置すると同じ誤解がエントリ16 経由で再生産される。
3. **提案1（`VK_IME_OFF→VK_IME_ON` トグル版）の却下記録**: 本 ADR 決定1 の4理由（頻度差 × 実測副作用、composition を閉じる意味論の差、confirm の実効性に保証が無い、より安い代替がある）を要約し、ADR-100 へリンクする。
4. **提案2（give-up 分岐の confirm 後 retry）の却下記録と代替策**: 本 ADR 決定3 の4理由を要約。特に「完了通知経路が存在しない」（最も強い理由）。あわせて採用した案L（journal 記録）と、保持した案J（Unicode 退避）・案K（backspace も打たない）を記す。
5. **新規: 送信形態（`wScan` / `dwExtraInfo` マーカー）が独立変数である（F14）。** 実験の第1候補・第2候補と、第2候補では否定的結果を「`VK_IME_ON` が効かない」と解釈できないこと。`docs/experiments.md` エントリ09（scan=0 注入がフックにすら届かず反証された）へリンクする。
6. **新規: 既存の `VK_IME_ON` 経路（`send_unicode_cold_warmup_keys`）とその犠牲キー設計（F13）。** 合格基準表に「`VK_IME_ON` 単発で `gji_write_bytes` が上がるか」を1行追加する。**案A' が案A の次手として存在すること**を明記し、案A の不合格を「`VK_IME_ON` 系は駄目」と一般化しないよう釘を刺す。
7. **新規: 対照群 (C)「warmup を送らない」（決定2）。** 合格基準表の1列目を「群」にして A/B/C の3群化する。これにより eager warmup 自体の実効性（ADR-098 決定3 の前提）も同時に検証できる。
8. **学び（暫定）の更新**: 現在の「『唯一生きている機構だから触らない』と『触るなら安全な代替キーに変えたい』は両立する」に加えて:

   > **既存機構の再利用に見える提案でも、その機構が今日どの頻度で撃たれているかを先に数えること。そして「使われていない」と結論する前に、その機構が出すログ文字列の唯一の出力元を grep で確かめること。**
   > （ADR-100 の初稿は `[gji-coro]` / `[h1-warmup]` の出力元を確かめずに「`Tsf` モードでは一度も撃たれていない」と書き、**自分が同じ ADR 内で引用していた BUG-45 のログと矛盾した**。同様に「TSF ネイティブで IMC が読めるかは未検証」と書いた直後の節で、読めた実例を含むログを引用していた。**手元の記録を引用する前に、その記録が自分の別の主張を否定していないか読み返すこと。**）
   >
   > **さらに: 「この経路は必ずモード X 限定である」と書く前に、その関数の呼び出し元を grep で全数数えること。分岐条件が `injection_mode` 以外の軸（`tsf_gate` 等）に載っている呼び出し元が混じっていることがある。**
   > （ADR-100 は同じ「モード分割の言い切り」を**3回**間違えた: 初稿の F3、それを訂正した第2稿の F3 表（`flush_raw_tsf_literal_romaji` が `tsf_gate` で分岐するのを落としていた）、および F12 row1（`Windows.UI.Input.InputSite.WindowClass` は `AppImeProfile` では TsfNative だが `AppKind::Uwp` → 既定 `Unicode`）。**`AppImeProfile` / `AppKind` / `InjectionMode` / `TsfGateState` は4つの独立した軸であり**、どれか1つで語ると必ずどこかが漏れる。ADR-083 の「訂正された事実誤認」項目1 が既に同じ罠を記録している。）

---

## premortem: この決定を実装して3ヶ月後に BUG-XX が起きるとしたら

**P1（決定2 の実験が本流に入った後）: `VK_IME_ON` は IME を開くが conv がローマ字入力のままで、かな入力ユーザーの初回打鍵が英字になる。**
`VK_DBE_HIRAGANA` は「開く + ひらがなに寄せる」であり、後者が消える。かな入力（`restore_roman` 系、BUG-08）の環境では、この「寄せる」作用が実は誰かの前提になっている可能性がある。決定2 の測定項目4 は「conv への影響が無いこと」を合格基準にしているが、**「影響が無い」が「今まで寄せてくれていたものが寄らなくなる」という形で現れる可能性**を見落とすと、実験は合格判定されたのに実運用で壊れる。**対策**: 測定項目2 のとおり「conv が変わらないこと」ではなく「置換前と置換後で入力結果の文字列が一致すること」で取る。これを3群すべてに適用する。

**P2（決定5 を記録だけして修正しないまま、別の作業が retry を配線した）: フォーカス跨ぎの stale confirm で誤った romaji が別ウィンドウへ再送される。**
F6 のガード欠落は既知として記録されるが未修正のまま残る。将来「reinit 完了確認後の retry」を実装する誰かが、その完了通知に世代照合を入れ忘れると、Alt+Tab 直後に旧ウィンドウ向けの romaji が新ウィンドウへ送られる。BUG-35（stale confirm 誤帰属、ADR-079 Stage1 epoch fencing）と同型。**対策**: known-bugs.md の記録に「retry を配線する場合は F6 の修正が前提条件である」と明記する（決定5）。

**P3（決定4-b の測定が「reinit の IMC poll がほとんど confirmed に到達していない」を示した場合）: BUG-33 の修正自体の実効性が疑わしくなる。**
BUG-45 に肯定事例が 1 件ある（F5）ので「一度も到達しない」ことは無いが、到達率が低ければ、give-up 分岐の reinit は多くの場合「OFF→ON を送って 300ms 空回りしていただけ」になる。その場合、BUG-33 が「修正済み」としている状態の根拠（実機ソークで give-up ログが 0 件になった）は、reinit が効いたからではなく別要因（BUG-27 追補3 の `apply_vk_sent` 委譲漏れ修正など、同時期の別修正）だった可能性が出る。**これは本 ADR の決定を覆す情報ではないが、BUG-33 の記録に追補として残す価値がある。** 4-d（`write_bytes 上昇検出`）が独立の証拠になる。

**P4: 決定1 の却下を根拠に、後から誰かが「じゃあ F2 のままでいい」と結論して BUG-50 リスクを忘れる。**
決定1 は「OFF→ON トグルは却下」であって「F2 のままで良い」ではない。ADR-098 F4 が「受容中の既知リスク」と呼んだものは今も残っている。**対策**: 決定2（案A の実験登録）と experiments.md エントリ16 の拡張により、宿題が消えないようにする。本 ADR のステータス欄と `docs/adr/index.md` にも「決定2 は未実施」と明記する。

**P5（第2稿で追加）: give-up による文字消失が再報告されたが、ADR-100 に「検討して却下した」とだけ書かれており、代替策が用意されていない。**
F12 のとおり give-up 到達時の文字消失は 3 件記録済みの実害である。担当者は ADR-100 を読み、決定3 の4理由が今も有効であることを確認し、しかし目の前のユーザーは文字を失い続けているので、結局「とりあえず再送してみる」パッチ（案F そのもの、BUG-27 追補2 の再演）に手を伸ばす。**対策**: 決定3 で案L（journal 記録）を採用し、案J（Unicode 退避）・案K（backspace も打たない）を「却下ではなく保持」として表に残した。次の報告時には、案L が集めたデータ（実際に何の文字が失われたか）と決定4-a の頻度データを使って、案J / 案K のどちらを取るかを判断できる。**「却下」と「代替なし」を混同しないこと。**

**P6（F15/F16 実機測定後に追加、2026-08-22 訂正）: eager warmup 撤去を正式決定した3ヶ月後、TsfNative+GJI アプリへフォーカスを戻した最初の数文字が直接入力（英字）で出る。**
BUG-69（ADR-098）F1/F2/F3 は「TsfNative+GJI のフォーカス復帰時、実際に機能する IME actuation は eager warmup だけであり、他の2機構（force-on ブロック・`mirror_applied_open` 経由の訂正）は別のバグで到達不能」と結論している。**ADR-100 はこの結論をこれまで一度も参照していなかった。** F16 の実機テストは Windows Terminal に対して群Cを試したが、3シナリオとも「実IMEがONのまま」推移した可能性が高く、「実IMEがOFFの状態でフォーカスが戻る」（外部IMEトグル直後、Ctrl+無変換の救済直後等）という、BUG-69 が指摘する本当に危険な条件を明示的には踏めていない（シナリオ1は`ime_on=false(shadow)`を経由したが、`SetOpenTrue`自体は正常に発火しており、eager warmupが唯一のactuationとして機能しなければならない場面を必ずしも意味しない）。

**訂正（2026-08-22、コード調査で確認）**: 「BUG-69 F1/F2 が未修正」という前提は誤りだった。ADR-098 決定1-a/1-b/1-c/2 は本 worktree の元になった `origin/develop`（コミット `034bfa2a`）に**既に実装済み**である（`src/platform.rs` の `WarmupImeOn` 型、`state/ime_actuation.rs` の `ForceOnRetryState`/`force_on_attempt_allowed`、`runtime/ime_refresh.rs` の `is_effectively_tsf_native` によるmirror分岐、到達不能だった force-on ブロックの撤去、いずれもコードで確認済み）。`docs/known-bugs.md` BUG-69 も「実装済み（2026-08-21 追記）」と明記している。**残る真の未実施は「Windows 実機での検証・ソークが一度も行われていないこと」**（BUG-69 自体の症状も「実機ログなし・コード読解で構築した想定シナリオ」のまま、修正の実機確認も同様に0件）。P6 の対策は「修正を先に入れる」ではなく「**この既存の修正を実機で検証してから eager warmup 撤去の議論を進める**」に修正する。

**追記（2026-08-22、実機第1回検証実施）**: dragonflyg4 で Windows Terminal に対し「IME を明示的に OFF にした状態で他ウィンドウへ切り替え→放置→フォーカスを戻し、物理キーには一切触れず待機する」手順を実施した。ログ上で `[focus-settle] apply_force_on_for_imm_broken skipped (settling) → 550ms 後に refresh で再試行` の直後に `force-ON (ImmBrokenForceOn): apply_ime_open(true) → Applied` が2回発火し、いずれも直前88秒以上・物理キー操作なしのタイミングで自律的に IME を正しく補正したことを確認した（詳細は `docs/known-bugs.md` BUG-69 の「Windows 実機での初回検証」節）。**これは BUG-69 の核心（force-on が TsfNative で恒久的に無効化されていた）が実機で解消されていることを示す一次証拠であり、P6 が要求していた「eager warmup 撤去の前提条件」の実機側の確認が1回分進んだことを意味する。** ただし1セッション・1アプリでの限定的な確認であり、ソーク（長時間・複数アプリでの継続検証）はまだ無い。P6 自体は「解消済み」に格上げしない——この1回のデータだけで premortem シナリオを閉じるのは、まさに F15 が踏んだ「少ないサンプルで結論を急ぐ」失敗の再演になる。

---

## 未解決の疑問・実機検証が必要な項目

正直に「わからない」ことを列挙する。**第2稿で項目1・3 の性格が変わり、項目7 に F13 の示唆を追記した。**

1. **【F5、性格が変わった】TSF ネイティブアプリで `IMC_GETCONVERSIONMODE` が読める「割合」。** 原理的に読めることは BUG-45 の 1 件で確認済み。未知なのは (i) 常に読めるか、(ii) どのクラス（WezTerm 本体・InputSite・CoreWindow）で読めるか、(iii) confirmed 到達率。決定4-b がこれを測る。**初稿はこれを「最重要の未測定量」と誤って位置づけていた。**
2. **【F1】ユーザーの実機で eager warmup が実際にどのアプリへ飛んでいるか。** `force_tsf` の既定エントリの有無と、実行時学習（`cache.toml` の `[injection_mode]`）で昇格したクラスは環境依存。決定4-c。**warmup が飛んでいるアプリが存在しなければ、決定2 の実験は空振りになる。**
3. **【F12、訂正済み】give-up の実機発生「頻度」（アプリ別・モード別）。** 発生すること自体は 3 件の記録で確定。未知なのは頻度分布。決定4-a。
4. **【F7-1】`ImeKindDebounce` が高頻度 reinit に耐えるか。** 提案1 を却下したので当面問題にならないが、決定2 の案A/A' が採用され、かつ `VK_IME_ON` が同様のフリップを誘発する場合には同じ問いが再燃する。`VK_IME_ON` 単発が `GetActiveProfile` フリップを起こすかは未測定。
5. **【F10】BUG-45 の "kaきの" の真の因果。** 追補1 で reinit-commits-preedit 説が「推測」へ格下げされ、以後進展していない。BUG-45 が挙げる次の一手（backspace flush 直後・reinit 直後の画面/バッファ内容の診断ログ、および「か」単独の最小再現）は未実施。**決定3 の案L はこの解析に必要なデータの一部（失われた romaji の値）を供給する。**
6. **【F4-1、m14 で性格が変わった】conv 軸に profile ゲートが無い非対称。** BUG-55 の経緯から「別方針の設計」であることが分かった（決定6）。残る疑問は「hwnd の取り方で対応する方式が、全 TSF ネイティブクラスで機能するか」——これは項目1 と同じ問いに帰着する。
7. **【F13】`VK_IME_ON` が GJI の TSF composition context 再初期化を `VK_DBE_HIRAGANA` と同等にトリガーするか**（ADR-098 決定3-c の原問、決定2 測定項目3）。**本 ADR で最終的に答えを出せなかった中心の問い。** ただし完全な未知ではない: `send_unicode_cold_warmup_keys` の設計が「`VK_IME_ON` の後に犠牲キー `VK_A` を足す」形になっていることは、少なくとも「`VK_IME_ON` 単発では `gji_write_bytes` の上昇という観測可能な反応が得られない」ことを示唆する（読み方A）。ただし犠牲キーが warm 手段なのか観測手段なのかはコードからは決着しない（読み方B）。決定4-e / 決定2 測定項目6 が決着させる。
8. **【F14、F17で解決】`MapVirtualKeyW(VK_IME_ON=0x16, MAPVK_VK_TO_VSC)` の戻り値。** 実機測定済み: `0xF2`（非ゼロ）。第1候補（scan実値）が成立することを確認した。
9. **【F2、m12】`output/vk_send.rs:531` が本当に到達不能か。** 本 ADR はコードコメント依拠で独立検証していない唯一の項目。決定2 の実験で eager warmup の送信キーを変える場合、この経路も同時に変わるので、到達しうるなら影響を受ける。

---

## 実装時に満たすべき規約（後続作業向け）

本 ADR 自体は設計文書であり `.claude/rules/fix-requires-evidence.md` の直接の対象ではないが、決定から派生する実装コミットについて事前に決めておく。

- **決定3 の案L（journal 記録）**: 実装を伴う唯一の決定。`tsf/literal_facts.rs:58-66` の `LiteralDetectRecord` に `Option<String>` フィールドを1つ追加し、`probe_io.rs:629` の give-up/初回疑い側で `romaji.clone()` を詰める。**構築サイトは本番5箇所 + テストヘルパー1箇所あり、全部がコンパイルエラーになる**（決定3「案L の作業範囲」の表を参照。特に `platform.rs:204-210` は `probe_io.rs` の外なので見落としやすい）。他5箇所は `None`。journal への配線（`platform.rs:204/248` → `journal.rs:210-214`）は既存のまま流れる。挙動変更ゼロ・送信ゼロ。**プライバシー方針は決定3 の該当節で確定済み**（生 romaji を入れる、`attach_log` の既存 opt-in に委ねる）。
  **(a) 回帰テスト**として、`probe_io.rs:1270-1305` の既存 give-up テスト（`gave_up=true` の Verdict が trace に残ることを検証している）に「捨てた romaji が `LiteralDetectRecord` に入っていること」の assertion を足せる（Linux で実行可能）。したがって `fix-requires-evidence.md` は (a) で満たせる。
- **決定2 の実験ビルド**: 一時ブランチで行い、結論が出た側だけを本流へ入れる。撤回する場合は `.claude/rules/experiment-logging.md` に従い revert コミット本文にアプリ・IME 状態・再現手順の3点を書き、`docs/experiments.md` エントリ16 に1行追記する。本採用する場合、`crates/awase-windows/tests/ime_key_sequence_golden.rs`（キー選択の golden、`ime_controller.rs::characterize_strategy`（`ime_controller.rs:568`）が SSOT）で守れるかを検討する——ただし eager warmup は `characterize_strategy` の経路ではないため golden で守れない可能性が高い。その場合は (b)（`docs/known-bugs.md` への記録）で代替する。
- **決定5（known-bugs.md への F6 記録）**: 記録のみ・コード変更なし。
- **決定6・決定7（doc コメント訂正）**: 挙動変更ゼロ。`cargo xwin clippy --lib -D warnings` が通ることのみ確認すればよい。決定3 の案L と同一コミットで入れてよい。
- **タイミング定数**: 本 ADR の決定は新規のタイミング定数を導入しない。決定2 の実験の結果として待機が必要になった場合は、`.claude/rules/tuning-constants.md` の実測義務（測ったもの・数値・導出）を満たしてから起票する。特に `CHROME_GJI_REINIT_CONFIRM_MS`（300ms）は「実測」ではなく「GJI は VK_IME_ON 受信後 ~50-100ms 以内に移行する実測値が多い。300ms あれば十分な余裕」という導出であり、かつ**レート制限にも兼用され、さらに Unicode long-cold 経路と give-up 経路で共有されている**（決定7、m13）。この定数を動かす場合は3つの役割すべてに影響することを明記すること。

## 関連 ADR / BUG への影響

- **ADR-098**: 決定3-c を本 ADR が引き取り、案A（`VK_IME_ON` 単発）は決定2 で実験登録、拡張版（OFF→ON トグル）は決定1 で却下。決定3（eager warmup の KEEP）と F4（受容中の既知リスク）は変更なし。**ただし決定3 の原文が eager warmup 撤去の被害例として挙げた「BUG-02（Chrome）のリテラル化」は誤りであり、本 ADR F1 / 決定1 案C で訂正する**（Chrome は eager warmup 対象外）。ADR-098 側にもこの訂正を追補として反映すること。また決定3 の「唯一生きている実効的な cold-start 対策」という評価は対照群を持たない判断であり、決定2 の群 C がこれを初めて検証する。
- **BUG-32**: 「give-up 分岐から `send_eager_tsf_warmup` を再試行する経路」という残課題は、本 ADR 決定3 の案G として検討したうえでスコープ外とした（別の失敗モードの回収であり、`send_vk_dbe_hiragana_pair` の `None` を記録して再試行する方が的確）。BUG-32 の残課題欄にこの判断を追記すること。
- **BUG-33**: 決定4-b の測定結果次第では、reinit の実効性（＝BUG-33 修正の効き）の再評価が必要になる（P3）。
- **BUG-38/39・BUG-16 追補3・BUG-45**: F12 の give-up 実害 3 件。決定3 の案L（journal 記録）はこれらの続報が来たときの解析材料を供給するためのものである。
- **BUG-45**: 未解決のまま。本 ADR 決定3 は BUG-45 の推奨方向 (b) と競合しないよう提案2 を却下し、案K（backspace も打たない）を同方向の候補として保持した。
- **BUG-50**: 決定2 の実験の直接の目的（`VK_DBE_HIRAGANA` の「ひらがなに強制する」副作用の除去）。
- **BUG-55**: 決定6 の m14（conv 軸が profile ゲートではなく hwnd の取り方で TSF ネイティブに対応している経緯）の出典。
- **ADR-048（SacrificialWarmup）**: 決定1 案A' の先行事例（`VK_A`+BS のアトミックバッチ）。
- **ADR-096**: 決定3 案L の実装先（literal-detect journal レーンの `LiteralDetectRecord`）。
- **`docs/experiments.md` エントリ16**: 置換ではなく拡張（上記「エントリ16 の扱い」、追記項目 1〜8）。
- **`docs/experiments.md` エントリ09**: 決定2 の第2候補（scan=0 注入）の限界の根拠。
