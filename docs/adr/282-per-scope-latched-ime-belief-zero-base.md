---
id: ADR-282
title: |-
  IME の開閉と Engine の活性の belief をゼロベースで作り直す: IME の保持単位(スコープ)ごとに「最後に受け入れた証拠」を保持し、明示操作は ON・OFF とも同じ重さで、窓切替・観測の寿命切れで古い値へ戻さない(書き込みの根拠の寿命は今のまま分ける)
summary: |-
  BUG-223(GJI の tsf-native / imm32-unavailable 窓で、IME は ON なのに Engine が OFF のまま数秒〜数十秒続き、「こうしき」が「るしき」になった)と、
  所有者の方針(2026-10-26)「IME が ON でかなモード(ひらがな・カタカナ)なら必ず Engine ON。逆は OFF」を受けて、belief をゼロベースで設計し直す案。
  今の belief は、明示意図の保管場所が 5 つ(`desired_open`・`last_intent`・`IntentStore`・`persistent_explicit_off`・`key_effect.open`)あり、
  ひらがなキー(F2)の ON は予測、IME-OFF キーの OFF は意図、と経路で非対称で、窓切替は予測・観測を捨ててグローバルな `desired_open` を持ち越す。
  基準の測定(PR #857、擬似 IME・擬似窓のオラクル)では、IME の状態を全窓で共有するモデル(shared)の msime で I1 違反 405 本(主因 `desired-fallback` 297)、
  窓ごとに別スレッドのモデル(per-thread)では 9 本。実機がどちらに近いかは読めない窓では未測定で、設計の前提を CI 検証バッチ(§5.5)で確かめる。
  案の骨子: (1) belief を IME の保持単位(スコープ)ごとに持ち、スコープの粒度(スレッドごと/共有)はデータにする(前提つき)、
  (2) 各スコープは「最後に受け入れた主張」をラッチし、寿命切れで別の値へ後退しない、(3) 明示操作は ON・OFF・モード・修飾付き(BUG-221・ADR-281)を同じ型で記録し、
  新しい明示操作か受け入れ基準を満たす新しい観測だけが覆す、(4) 観測の信頼は「窓の種類 × IME × 証拠源」の表で決め、読めない窓の IMM の読みと MS-IME 本体の半角英数トグル中の閉の読みは反証に使わない、
  (5) 不確実なときは所有者の判断どおり、読めない窓は ON(かな)、読める窓は観測、(6) **belief のラッチと、書き込み(drift correction・授権)の根拠の寿命を分け**、
  書き込みの根拠は利用者の操作(`UserIntentSource` の範囲)だけ・今の寿命のままにする。IME への書き込みの方針は変えない(ADR-191・212 のまま)。
  局所修正(L1〜L3)を同じオラクルで測った結果(PR #868): 真が shared なら L1(予測で `desired_open` を書く、+7 行)で msime の I1 405 → 153・核の最小列 36 → 0。
  真が per-thread なら I1・I2 を同時に減らす局所修正は無い(L1 は I2 を 57 → 109)。残る 153 本は嘘の観測・外部変更で、どちらの案でも観測の手段と既定の設計が要る。
  保持単位の CI 測定(run 40327981441)では窓の種類で振る舞いが違い(読める窓・実 Chrome はスレッドごと、MS-IME × Windows Terminal は入場時に開を引き継ぐ、GJI × WT は未確定)、単一の局所修正は片方の窓で I2 を増やす。
  よって少なくとも「窓の種類ごとに持ち越すかの規則」(中間案 L4)が要り、それ以上のゼロベースが要るかは V0 で L4 と比べて決める(§13.2)。
status: |-
  起草(2026-10-26)→ Opus レビュー round 1(Blocker 2・Major 9・Minor 9)を反映した改訂(2026-10-26)。未確定(round 2 待ち)。
  所有者の判断 4 件(かなの範囲・不確実なときの既定・最後の明示操作が勝つ・窓ごとに独立)を決定として反映。BUG-221・ADR-281 の残りを範囲に含めた(所有者判断 2026-10-26)。
  依存する PR: #857(基準の測定)は 2026-10-26 にマージ済み(`gh pr view 857`)。#854(BUG-223)は OPEN。局所修正 L1〜L3 の測定は PR #868(OPEN、docs のみ)で出た(§13)。保持単位の CI 測定は run 40327981441 で一部出た(§1.4。読める窓・実 Chrome はスレッドごと、MS-IME × WT は入場時に開を引き継ぐ、GJI × WT は未確定)。
  別セッションの Opus コードレビューの入力 4 件を §4.14 に反映(コードで確認、症状は未確認)。CI 検証バッチ(§5.5)は未実施。未決 U1〜U13 は所有者の確認待ち。実装はまだ(コード・テストは不変)。
related_adr:
  - "ADR-082"
  - "ADR-087"
  - "ADR-089"
  - "ADR-098"
  - "ADR-106"
  - "ADR-165"
  - "ADR-179"
  - "ADR-187"
  - "ADR-188"
  - "ADR-191"
  - "ADR-199"
  - "ADR-205"
  - "ADR-208"
  - "ADR-212"
  - "ADR-213"
  - "ADR-214"
  - "ADR-233"
  - "ADR-245"
  - "ADR-255"
  - "ADR-260"
  - "ADR-262"
  - "ADR-263"
  - "ADR-268"
  - "ADR-273"
  - "ADR-277"
  - "ADR-279"
  - "ADR-280"
  - "ADR-281"
---

# ADR-282: IME の開閉と Engine の活性の belief をゼロベースで作り直す

調べた版: `origin/develop` `ebd744b6d`。行番号はこの版のもの。
棚卸し(PR #858、`origin/docs/belief-zero-base-inventory` の `docs/tasks/belief-zero-base-inventory-2026-10-26/`、以下「棚卸し」)は `ff75d5d58` で書かれており、
ここで引いたコードの行は本 ADR の起草・改訂時に `ebd744b6d` で読み直したもの(例: `compute_state` の本体は `src/engine/engine.rs:272`、269 は doc の行)。
基準の測定は PR #857(マージ済み。読んだのは `origin/test/belief-engine-invariant` の HEAD `37fc37e75` の `docs/tasks/belief-engine-invariant-baseline-2026-10-26.md` と golden
`crates/awase-windows/tests/golden/belief_engine_invariant_violations.txt`。以下「基準の測定」)。cargo はローカルで実行していない。実機のログは読んでいない。
Opus レビュー round 1 の指摘 ID(B1・B2・M1〜M9・m1〜m9)への対応は §14。

## 0. 決定の要約

**決定**(所有者の判断から直接導くもの・belief の形):

| # | 決定 | 今との違い |
|---|---|---|
| D2 | 各スコープは開閉軸・モード軸それぞれに **「最後に受け入れた主張(claim)」を 1 つだけ**持つ。主張は寿命切れで消えない(ラッチ。例外は §4.1 の行の寿命) | 今は 6 段の優先順位 + 鮮度 3 秒 + `DesiredFallback` で、寿命が切れると別の値へ後退する |
| D3 | **明示操作**(IME-ON/OFF キー・トグル・ひらがな/カタカナ/英数キー〈修飾付きを含む。§4.13〉・`keys.ime_*`・Engine の `SetOpen`)は、方向・キーの種類・修飾によらず同じ型 `ExplicitOp` で主張になる | 今は `UserImeSetIntent`(OFF キーなど)と `KeyEffectPredicted`(F2 など)に分かれ、F2 の ON だけ窓切替で消える(A1・A9、BUG-223) |
| D4 | 主張を覆せるのは、**より新しい明示操作**か、**受け入れ基準を満たす新しい観測**だけ。基準は今の `derive_any` と同じ(High 単独、または Medium 以上の無競合多数決〈**1 ソースでも可**〉)に、「窓の種類 × IME × 証拠源」の信頼表と柵(settle 170ms・字形 1300ms・フォーカス epoch+hwnd)を足す | 今は窓の種類を問わず `derive_any` が勝つ(BUG-219・223(a))。鮮度 3 秒の間は古い High が新しい Medium に勝つ(基準の測定の `derive-high` 型) |
| D5 | **不確実なとき**(スコープに開閉の主張が無い)の既定は、所有者の判断 O2 のとおり: **読めない窓は ON(かな)**、読める窓は観測(最初の読み)。実測の事前(新しいスレッドは閉)を先に使うかは U2、端末アプリを外すかは U10、InputRelay は U3 | 今は前窓の `desired_open` を持ち越す / ON の HeuristicDefault / OFF の HeuristicDefault が窓の種類ごとに混在(A5・A9) |
| D7 | Engine の活性 = `user_enabled ∧ 日本語 IME ∧ 開 ∧ かな(ひらがな・カタカナ、ローマ字入力)`。式は今の `compute_state` の 4 条件のまま、入力をフォーカス中のスコープの主張から作る | 入力の出所だけが変わる |
| D8 | **belief のラッチと、書き込みの根拠を分ける**(ADR-087 P11)。drift correction・授権に入るのは、利用者の操作(`OpOrigin ∈ {UserKey, UserCommand}`、今の `UserIntentSource` の `SyncKey`・`PhysicalImeKey`・`Command` の範囲)だけで、寿命は今のまま(drift は今の `last_intent` と同じく窓切替・予測・通過で消える、授権 Step 1 は `IntentStore` の 10/30 秒)。既定・予測・lang-detect・半角英数トグル・パニックリセットは入れない | 今の `OwnSsot` は `desired_open`(既定・持ち越しを含む)を読む(`open_warrant.rs:224`) |
| D9 | IME への書き込みの方針(何をいつ書くか)は変えない。ADR-191・199・212・213 のまま | 変更なし |

**前提つきの案**(実機の事実が未測定。§5.5 の CI 検証バッチの結果で確定・修正する):

| # | 案 | 前提 |
|---|---|---|
| P1 | belief を **IME の保持単位(スコープ)ごと**に持つ。粒度は (IME, 窓の種類, 軸) ごとのデータ `ScopeModel ∈ {PerThread, InheritOnEntry, Shared}`。PerThread の鍵は (pid, フォーカス先のスレッド) | CI 測定(run 40327981441、§1.4): 読める Win32 窓・実 Chrome は GJI・MS-IME とも PerThread、MS-IME × Windows Terminal の開閉は入場時に他の窓の開を引き継ぐ(InheritOnEntry)、GJI × WT・WezTerm・UWP は未確定(U4、§5.5 B1 の残り) |
| P6 | **ポーリングを明示意図から切り離す**(D6) | 今の早期 return が担う 3 つの役割(ADR-187 の通過マークの保護・ADR-279 P0 の起点・GJI の I/O 推論を drift の根拠にしない)と、「@」(BUG-113 系)との関係を置き換え・確認してから(§4.12、§5.5 B8)。満たせなければ D6 は採らず、予測の主張は止まったポーリングのまま保持する |

## 1. 背景

### 1.1 実機の事故(BUG-223)

`origin/fix/focus-desired-fallback-off:docs/known-bugs/BUG-223.md`(PR #854、OPEN)より。dragonflyg4(GJI、awase develop 43359222)で「こうしき」が「るしき」になった。
journal の `effective-open-flip → false` のうち、所有者の IME-OFF キー(`ExplicitIntent`)でない区間が 5 つ(03:48:28・03:49:31・04:33:48・04:34:27・04:37:54Z)。

- (a) 区間 1・4: 観測が false を出した。`DeriveMedium{ObserverPoll}` の単独、`DeriveHigh(ImmCrossProbe)`。GJI の tsf-native / imm32-unavailable 窓の IMM の読みが偽の閉を返した(BUG-219 と同型)。
- (b) 区間 2・3・5: 観測が無く `desired_open`(前窓の false)へ後退した(`DesiredFallback`)。区間 3 は直前の操作が F2(ON)だった。`KeyEffectPredicted` は `desired_open` を書かず、
  窓切替(`reduce_focus_changed`、`crates/awase-ime-core/src/state/ime_model.rs:1157`)が予測 `key_effect` を捨てると、前の OFF キーが書いた `desired_open=false` に戻る。
- BUG-223 自身が未確認としている点: (b) の各区間の直前に明示 OFF があったか。**実機で窓切替の後も IME が ON だった理由が「全窓で共有」か「移動先のスレッド自身の状態が ON」かも未確認**(どちらでも BUG-223 の記述と合う)。

### 1.2 基準の測定(PR #857)が示したこと: 405 本は shared、per-thread では 9 本

数値は §5.1.1 に表で再掲する。要点:

- **IME の状態を全窓で共有するモデル(shared)**: msime・全字母・長さ 3 で I1 違反 405 本、主因 `desired-fallback` 297 本、最小列 `key:hiragana > focus:wezterm`(OFF キー不要)。読めない窓で終わる違反は poll が届いても 100% 残る。BUG-223 の 4 本は shared でだけ違反。
- **窓ごとに別スレッドのモデル(per-thread)**: msime・核の字母・長さ 3 で **9 本**(poll 後 5 本)。原因は窓切替の持ち越しではなく、F0 で入力モードが半角英数の観測のまま残る軸と、戻った窓で起きる `desired-fallback`(`key:eisu > focus:wezterm > focus:explorer`)。BUG-223 の 4 本は違反にならない。
- 基準の測定は shared を主にした(理由: BUG-223 の「窓切替の後も IME は ON」に合う)。上の 1.1 のとおり、この記述は per-thread とも矛盾しない。
- **ゼロベースの規模の根拠の再検討(B1 の指摘)**: 実機が per-thread に近ければ、今のコードの I1 違反は 9 本で、構造を作り直す根拠は「違反の数」ではなく、(i) 明示意図の保管場所 5 つの非対称(X1・X3)が BUG-223 のような型を作り続けること、(ii) 所有者の判断 O3(寿命切れで戻らない)・O4(窓ごとに独立)を今の構造に入れると保管場所がさらに増えること、の 2 つになる。
  実機が shared に近ければ 405 本の大半(`desired-fallback`)が対象だが、その場合は局所修正(§9 の L1・L2)で多くが消える見込みもある。**どちらの場合も、局所修正 L1〜L3 を同じオラクルで数えてから、ゼロベースを採るかを V0 で判断する**(§5.4)。 測定の結果と結論は §13。
- 本 ADR の起草時に `crates/awase-windows/tests/support/harness.rs` の `key_mods` を読んで確かめたこと: ハーネスの物理キーは全部 `has_shadow_action: false`・`unreadable: false`・`observable_toggle_guard: false` で予測の経路に流す。
  本番の 0x16(IME-ON)・トグル解決キーは `UserImeSetIntent` の経路(棚卸し 01-map §4.1)なので、`key:ime-on`・`key:hankaku-zenkaku` で始まる最小列は本番と経路が違う(本番で同じ違反になるかは未確認)。F2 は本番でも予測の経路。

### 1.3 今の構造の問題(棚卸し 04-complexity の X1〜X11 のうち、本 ADR が解くもの)

| # | 問題 | 本 ADR での扱い |
|---|---|---|
| X1 | 明示意図の保管場所が 5 つで寿命がばらばら | belief 側は `ExplicitOp` 1 つ(スコープごと)に統合(D3)。書き込みの根拠(drift・授権)は今の `last_intent`・`IntentStore` の寿命のまま残す(D8)ので、**保管場所は 5 → 3(belief の主張・drift の根拠・授権の根拠)**で、0 にはならない |
| X2 | `desired_open` の意味が 8 つ混在 | `desired_open` を廃止し、意味ごとに置き場所を分ける(§4.8) |
| X3 | ON と OFF の経路・寿命の非対称(A1〜A9) | belief 側は方向で分けない(D3)。書き込みの根拠の非対称(TTL 10/30 秒)は残る(D8) |
| X4 | 窓切替の後処理が窓の種類ごとに別の書き方 | 窓切替はスコープの切り替えだけ(§4.5)。窓の種類の差は既定(D5)と信頼表(D4)に集める |
| X6 | 優先順位が 2 層(`resolve_open_at` の上に `IntentStore` の上書き) | belief 側は 1 層(スコープの主張) |
| X8 | 半角英数トグルの状態が 2 か所 | 表現はモード軸の主張に寄せる。出入りの条件は今の規則のまま(§4.10。取り下げ #6) |
| X10 | 初期値 `desired_open=true` の placeholder と特例 | スコープの「主張なし」で表す(D5) |

解かないもの: X5(外からの変化への追随の窓 4 つの統合)は観測の受け入れの入口が 1 つになるので尾部の重複は消えるが、窓の機構そのものは残す。X7(Engine を通さない入口)は U6。X9・X11 は範囲外。

### 1.4 所有者の方針と判断(2026-10-26、決定として扱う)

方針: IME が ON でかなモード(ひらがな・カタカナ)なら、必ず Engine ON。例外は `user_enabled` が偽のとき(ほかの例外候補は U1・U3)。逆に IME OFF・半角英数・直接入力なら Engine OFF。

| # | 所有者の判断 | 本 ADR で対応する決定 |
|---|---|---|
| O1 | 「かな」の範囲 = ひらがな・カタカナ。半角英数・直接入力は含まない | D7・§4.7 |
| O2 | 不確実なとき(観測が読めない窓で IME の状態が分からない)は窓の種類で分ける。読めない窓は ON 寄り、読める窓は観測を信じる | D5・§4.6(**中身は変えない**。適用範囲の確認は U2・U3・U10) |
| O3 | 最後の明示操作が勝つ。ON も OFF も対称。窓切替や観測の寿命切れで古い値に戻らない。観測が反証する場合の扱いは設計する | D2・D3・D4・§4.4 |
| O4 | 窓切替では IME の状態を窓ごとに独立させ、持ち越さない。新しい窓は、その窓の直前の記録、無ければ不確実の方針 | P1・§4.1・§4.5(「窓ごと」の読み方は U4。真が shared の構成との衝突は U11) |

**「保持単位はスレッド」の出典と範囲(B1 の指摘で限定した)**: `docs/adr/191-gji-state-scope-spec.md` §2・§2.1。CI run 35623301916(ブランチ `ci/e2e-gji-scope`、プローブ `gji_state_scope_probe`)。
測ったのは自前の **Win32 EDIT の窓**(`edit2`/`top2`/`thread2`/`thread2_new`/`proc2_new`、すべて IMM で読める窓)、環境は windows-latest(Windows Server)・`SPI_GETTHREADLOCALINPUTSETTINGS = 0`、
MS-IME の構成は TIP を `MicrosoftIme (Other)`(IMM32 HKL)と同定していた(同 §3 の 2)。**TSF ネイティブのアプリ(Chrome・VS Code・WezTerm)は同 §4 が未確認と書いている**。
生データの artifact は保持期限 7 日で失効しており、プローブのブランチも今は無い(`git branch -r` に無く、`git log --all -- crates/awase-windows/examples/gji_state_scope_probe.rs` も空)。
よって「GJI・MS-IME とも保持単位はスレッド」は上の範囲での事実で、BUG-223 の起きた読めない窓には未確認。

**保持単位の CI 測定(2026-10-26、ブランチ `ci/adr282-ime-scope`、run 40327981441〈success、head `ef6e3d4e0`〉、ジョブ `scope (msime)` 121589556404・`scope (gji)` 121589557165 のログの `SCOPE:` 行を `gh api .../jobs/<id>/logs` で読んで引いた。文書と PR は別担当が準備中)**:
窓 A で IME を設定(ひらがなで ON・半角英数・閉)し、別プロセスの窓 B で `ka` を打つ。対照: B 単独で `ka` は `ka`、IME ON キーの後は `か`。各セル 2 試行。環境は windows-latest(Windows Server)。

| IME | A > B | 開閉 | 変換モード | 備考(ログの値) |
|---|---|---|---|---|
| MS-IME 本体 | edit > edit・edit > chrome・chrome > edit・wt > edit | independent | independent | s1=`ka` 2/2 |
| MS-IME 本体 | **edit > wt**(B = Windows Terminal、tsf-native) | **shared-open** | independent | s1=`hira` 2/2(A を ON にしておくと B の WT で `ka` が `か`) |
| GJI | edit > edit・edit > chrome・chrome > edit | independent | independent | s1=`ka` 2/2 |
| GJI | wt > edit | independent | independent | s1=`ka` **1/1**(2 試行中 1 回だけ有効) |
| GJI | edit > wt | **inconclusive**(s1=mixed) | inconclusive(B で ON キーが効いていない、ctl_on=mixed 1/2) | GJI × WT の共有は未確定 |

読み取れること(小標本・Windows Server の範囲):
- 読める Win32 EDIT と**実 Chrome**(読めない窓、imm32-unavailable)の間では、GJI・MS-IME 本体とも開閉・変換モードは独立(スレッド/プロセスごと)。
- MS-IME 本体で、**WT(tsf-native)へ入ると、他の窓の開の状態が WT に効く**(edit > wt が shared-open)。ただし**逆向き(wt > edit)は independent** で、全窓で対称に共有されているわけではない(WT 側が入場時に他から開の状態を取り込む、のような一方向の振る舞いに見える。仕組みは未検証。TSF の keyboard open-close コンパートメントが候補)。
- GJI × WT、WezTerm、UWP/XAML、同じプロセスの別スレッド、マウス・タスクバーの切替、Windows 11 実機は未測定。
- BUG-223(GJI の tsf-native 窓を行き来して IME は ON のまま)は、MS-IME の WT の振る舞いと同じ型(入場先の tsf-native 窓が開を引き継ぐ)なら説明できるが、GJI × WT は inconclusive で確認できていない。


## 2. 不変条件(設計の目標)

検査の語彙は `crates/awase-windows/tests/support/harness.rs` と基準の測定のもの。「真」は擬似 IME の、いまフォーカスのあるスレッドの状態。
**すべての検査を shared と per-thread の両方の真のモデルで回し、合格条件をモデルごとに書く**(B1)。新設計の `ScopeModel` は、真のモデルと一致させた場合(粒度が正しい)と、食い違わせた場合(粒度の誤り)の両方を回す。

| # | 不変条件 | 検査の形 | 目標(合格条件) |
|---|---|---|---|
| I1 | 真が 開 ∧ かな(conv が NATIVE)∧ `user_enabled` ∧ 日本語 IME なら、Engine は Active | 基準の測定のオラクル(全列挙・長さ 3〜4・ランダム探索)。各事象の直後に `activation()` を見る | **粒度が正しい場合**: 「awase の操作・窓切替・正しい観測だけ」の列のうち、**予測の外れを含まない列**で違反 0(shared・per-thread とも)。予測の外れ(ハーネスの `PredictionRecord` の予測 ≠ 擬似 IME の真)を含む列は別の区分で数え、旧以下。**粒度の誤り**の場合は件数を報告(§4.1 の両立しない条件) |
| I2 | 真が 閉、または 開 ∧ 半角英数(NATIVE なし)なら、Engine は Inactive | 同じオラクルの I2 | 「awase の操作・窓切替・正しい観測だけ」の列で、既定の区分(D5 で ON に倒した列)以外は違反 0。**既定の区分を含めた I2 の総数は、モデルごと・窓の種類ごとに旧と並べ、旧より増える分を所有者が受け入れるかを判断する**(上限は所有者が決める。U10 の判断材料) |
| I3 | 最後の明示操作が勝つ(対称): スコープ S で明示操作 v をした後、S に対する新しい明示操作・受け入れた観測が無い限り、S にフォーカスがある間の belief は v | `op(v) @S` の後に {他スコープへの `focus_to`・`advance_ms`(3,500・60,000)・受け入れ基準を満たさない観測}を挟み、`focus_to(S)` の後の belief を見る。v を ON・OFF の両方で | 違反 0(S は `ScopeModel` で決まるスコープ。shared なら全窓が 1 つ) |
| I4 | スコープの独立: スコープ T の事象は S の主張を変えない | 変成関係: S にフォーカスがある時点の S の主張が、「S の事象と全体の事象だけ」を残した列で計算した主張と一致する | `ScopeModel=PerThread` で違反 0 |
| I5 | 古い観測で戻らない: 明示操作の時刻 + settle(`KEY_EFFECT_SETTLE_MS`=170)より前に読み始めた観測、字形の柵(`GLYPH_KEY_FENCE_MS`=1300)の内側、旧 epoch/hwnd の観測は主張を変えない | 読み取り開始時刻を op より前にした観測を流す。境界の前後(169/171ms) | 違反 0 |
| I6 | 寿命で後退しない: 主張は時間の経過だけでは変わらない | `advance_ms(N)` だけを流す。**N < `HWND_CACHE_MAX_AGE_MS`(3,600,000)の範囲**(最後にそのスコープにフォーカスがあってからの時間)。それ以上は行が捨てられ主張なしに戻る(§4.1) | N = 3,500・60,000・3,599,000 で違反 0 |
| I7 | 既定・予測・awase 起点の操作は書き込みを授権しない(D8) | §4.9 の授権の表の各入力で、`issue_open_warrant` の入力を検査。ハーネスの `WriteCommand` の件数 | 違反 0、書き込み件数は旧以下 |
| I8 | `user_enabled` が偽なら Inactive | `user_enabled` を切り替える事象を足す(§5.2) | 違反 0 |
| I9 | Engine が非活性になった瞬間に保留中のキーを flush する | Engine の単体テスト(ハーネスは `on_input` を呼ばない) | 今のテストを維持 |
| I10 | 修飾付きのモードキー(Ctrl・Shift・Alt + 0xF0〜0xF4)でも、効果が「開 ∧ かな」なら KeyDown の直後に Active(BUG-221)。効果が無い修飾付き(MS-IME 本体の Shift/Alt+F2 は開かない)で Active にしない | 格子の「届いた VK × 修飾」のセルごとに閉の状態から 1 回流す。**今の擬似 IME は修飾を知らない**(`harness.rs` の `key_ctrl` の doc、`grid-tables/*.json` に修飾付きのセルが無い)のでハーネス拡張が要る(§5.2) | 違反 0(team-lead の指示書の「I5」に当たる) |

### 2.1 0 にできない違反(弱点として明記する)

- **外部変更 × 読めない窓**: 言語バー・他アプリが読めない窓の IME を変えたとき、awase には証拠が届かない。主張は明示操作か既定のまま残る。直るのは次の明示操作、信頼できる証拠(MozcShm・ADR-205 の監視窓・字形を belief に入れる設定)、ADR-279 型の打鍵列による反証(未実装)。今の設計でも直らない。
- **嘘の観測 × 読める窓**: 信頼表で「反証可」にした源が嘘をつけば、主張は嘘に従う(BUG-189 の窓では新しい Medium を信じるのが正しかった)。
- **既定の区分**: 読めない窓・主張なしで ON に倒す。真が閉なら I2 違反。O2 の帰結。件数を旧と並べる(I2)。
- **予測の外れ**: F2 などの値は表の予測(ADR-191 決定 3。1 手で約 98.5%)。読めない窓では観測で訂正できず、新設計では窓切替をまたいで残る(今は窓切替で消えるので「偶然直る」ことが無くなる)。BUG-211・212 は未修正。I1 の合格条件からは外して別に数える(M4)。
- **粒度の誤り**: 真が shared なのに PerThread で持つ(またはその逆)と、§4.1 のとおり I1 と I3 は両立しない。

## 3. 新設計の全体像

```
            物理キー・Engine の要求・観測・フォーカス・タイマー(殻が集める事実)
                                │
                                ▼
   classify_*(今のまま)                       kp_predict_key_effect(入力をスコープの主張から作る。§4.13)
                                │
                                ▼
            BeliefEvent(値) ──► evolve(&mut BeliefStore, &BeliefEvent)   ← belief の唯一の書き込み点
                                │        (スコープ選択 → 受け入れ判定 → 主張の更新)
                                ▼
            decide(&BeliefStore, &BeliefEvent) -> Vec<BeliefCommand>       ← 命令は値で返す
              ・ScheduleRead { scope, at, reason }(P6 を採る場合)
              ・ActivationChanged { from, to }(殻が Engine の flush を回す)
              ・Journal(BeliefRecord)(根拠・却下の理由)
                                │
                                ▼
            derive(&BeliefStore, globals) -> BeliefView { open, mode, basis, activation }
              (Engine の ctx・lang-detect が読む。drift・授権は WriteBasis を読む〈D8〉)
```

- `evolve` は今の `ImeModel::reduce` の位置に入る(`.claude/rules/ime-belief-architecture.md` の「書き込み点は 1 か所」を引き継ぐ)。`decide`・`derive` は I/O を持たない純粋な関数(`docs/fcis-vocabulary.md` の Decider の形)。置き場所は `crates/awase-ime-core`。
- 書き込みの根拠(`WriteBasis`: drift の根拠・授権の根拠)は belief の主張とは別の状態として持ち、今の寿命の規則のまま(§4.9)。
- IME への書き込みは今の `kp_shadow_actuate`・`dispatch_ime_set_open`・drift correction の経路のまま。`decide` は IME への書き込みを返さない(D9)。

## 4. 設計

### 4.1 スコープ(P1)と、真のモデルが shared/per-thread のどちらでも成り立つか(B1)

**粒度をデータにする**: `ScopeModel` を (IME の種類, 窓の種類, `SPI_GETTHREADLOCALINPUTSETTINGS`) ごとの表で持つ。
- `PerThread`: スコープの鍵 = (pid, フォーカス先のスレッド)。
- `Shared`: スコープは 1 つ(全窓共通)。新設計は「グローバル 1 つの主張をラッチする」形になり、今の `desired_open` の持ち越しに近いが、予測の ON も主張になる点と寿命切れで後退しない点が違う。
- 初期値(§1.4 の測定 run 40327981441 による。小標本・Windows Server):

| 窓の種類 | IME | 開閉軸 | 変換モード軸 | 根拠 |
|---|---|---|---|---|
| 読める Win32 窓・実 Chrome(imm32-unavailable) | GJI・MS-IME 本体 | `PerThread` | `PerThread` | edit/chrome の 4 組が independent |
| tsf-native(Windows Terminal) | MS-IME 本体 | **`InheritOnEntry`**(新設): その窓に入ったとき、他のスコープの最新の開の主張を引き継ぐ。WT から出たときは相手に効かない | `PerThread` | edit > wt が shared-open、wt > edit が independent |
| tsf-native(Windows Terminal) | GJI | 未確定(U4) | 未確定 | edit > wt が inconclusive |
| WezTerm・UWP/XAML・その他の tsf-native | 任意 | 未確定 | 未確定 | 未測定 |

  `Shared`(全窓で 1 つ)は今の測定では当てはまる組が無い。未確定の組は、V0 で `PerThread`・`InheritOnEntry` の両方で回す。
  `InheritOnEntry` の窓に入ったときは、そのスコープの直前の記録より、**他のスコープの最新の開閉の主張が新しければそちらを採る**(時刻の新しいほう)。WT の中での明示操作はそのスコープの主張になるが、出た先には伝えない。

**両立するか(B1 の問い)**: 真のモデルと `ScopeModel` が一致すれば、I1(予測の外れ・外部変更を除く)と I3 は両立する。理由: I3 の「スコープ」は真が状態を共有する範囲そのものなので、最後の明示操作 = 真を最後に変えた操作になる。
- 例(レビューの例): `key:explicit-off @W(wezterm)` → `focus:A` → `key:hiragana @A` → `focus:W`。真が shared なら W も ON。`ScopeModel=Shared` なら主張は 1 つで、最後の操作(A の F2)の ON → I1・I3 とも満たす。
  `ScopeModel=PerThread` なら W の主張は OFF のまま → I3(PerThread の定義)は満たすが I1 違反。
- 測定(§1.4)の結果、窓の種類で振る舞いが違う(IMM32 系と実 Chrome はスレッドごと、MS-IME × WT は入場時に開を引き継ぐ一方向)。よって粒度は単一に決めず、窓の種類 × IME × 軸ごとのデータにする(上の表)。
- **一致しないとき、両方を満たす規則は無い**: 新しい窓に入ったとき、PerThread の真ではその窓自身の直前の状態が正しく、Shared の真では全体の最新の操作が正しい。両者は同じ入力(どちらの窓で何をしたか)から逆の結論を求めるので、粒度を知らずに両方を満たす規則は作れない。
  「別スコープの明示操作を弱い証拠として伝播する」案も検討したが、伝播した値が当たるのは Shared のときだけで、PerThread では I2/I1 の違反を作る(どちらに倒しても片方のモデルで外れる)。よって**粒度は測って決める**(B1 の CI 測定を V0 の前提条件にする)。
- **所有者の判断 O4 との関係(U11)**: 真が Shared の構成では、O4(窓ごとに独立、持ち越さない)そのものが真と食い違う(持ち越しが正しい)。その構成があると B1 で分かったら、O4 をその構成に適用するかを所有者に問う。

**スコープの鍵の取り方(M2: 帰属先と参照先を同じ関数にする)**:
- 鍵を求める関数を 1 つにする: `scope_of(foreground_tid) = (pid, tid of GetGUIThreadInfo(foreground_tid).hwndFocus)`。前例は `crates/awase-windows/src/focus/thread_scope.rs:365`(`GetGUIThreadInfo(window_tid, ..)` でフォーカス先を引いている)。
  明示操作の帰属(押下時点)と、フォーカス変化での参照(`derive` が読むスコープ)の両方でこの関数を使う。今の `win32::foreground_scope()`(`win32.rs:82`)は (pid, 前面の最上位 hwnd) で tid を持たないので、そのままでは使わない。

| 構成 | 前面の最上位窓 | フォーカス先(`hwndFocus`) | 鍵 | 未確認の点 |
|---|---|---|---|---|
| 普通の Win32 アプリ | アプリの窓 | 同じスレッドの子 | (pid, そのスレッド) | — |
| WebView2 を埋め込むアプリ | ホストアプリ | `msedgewebview2.exe` のスレッドの子 | (webview2 の pid, そのスレッド)。押下時点も参照時点も同じ関数なので食い違わない | IME の状態を持つのがホスト側か WebView2 側か(B1 で測る) |
| 別スレッドの子窓を持つアプリ | アプリの窓 | 別スレッドの子 | 子のスレッド | 同上 |
| `AttachThreadInput` で入力状態を共有するアプリ | — | 共有されたフォーカス | フォーカス先の hwnd 自身のスレッド | IME の状態がどちらのスレッドにあるか |
| UWP・TSF ネイティブ(`Windows.UI.Input.InputSite` 等) | `ApplicationFrameWindow` 等 | 入力面 | 入力面のスレッド | `hwndFocus` が真のフォーカスを返すか(BUG-91 は `hwndFocus` が真の最上位とは限らないと記録) |

- **tid が取れないとき**: `ScopeKey::Hwnd(hwndFocus)` に落とすが、同じ hwnd の tid が後で取れたら、`Hwnd` の行を (pid, tid) の行へ**移して 1 行にする**(正規化。2 行に割れない)。移すときに両方に主張があれば、時刻の新しいほうを残す。
- **スコープ表の行の寿命**: 主張の値は寿命で後退しないが、行は無限には持たない。プロセス終了・スレッド消滅は殻が知れば消す。知れない場合は、最後にフォーカスがあってから `HWND_CACHE_MAX_AGE_MS`(3,600,000、`tuning.rs:223`)で捨てる。
  **値を流用する理由と意味の違い(m6)**: 新しい時間の定数を足さない(取り下げ #8)ための流用で、意味は「キャッシュの鮮度」から「主張の最大持続時間」に変わる。`EXPLICIT_OFF_INTENT_TTL_MS` の doc(`tuning.rs:365-397`)は、`IntentStore` に 1 時間を流用したのを ADR-087 round4 M-A で「誤記録の最悪持続時間そのものになる」と指摘されて 30 秒へ縮めた経緯を書いている。
  本 ADR は belief 側だけで 1 時間を使い、書き込みの根拠(D8)には使わないことで、M-A が問題にした「drift が永久に再同期できない」を避ける(§4.9)。belief 側の誤りが最大 1 時間続きうること(読めない窓)はリスク 9。
- tid の再利用で別スレッドの行を読むおそれは、pid と組にすることで下げるが 0 ではない。

### 4.2 状態(D2)

```rust
// 擬似コード。名前は案。
struct BeliefStore {
    scopes: Map<ScopeKey, ScopeBelief>,
    focused: Option<(ScopeKey, FocusFence, WindowClass)>,
    observations: ObservationStore,   // 今の観測プール。受け入れ判定の材料。フォーカス切替で今どおり柵を進める
    globals: Globals,                 // user_enabled・is_japanese_ime・ime_kind・panic guard
    write_basis: WriteBasis,          // D8: drift の根拠(今の last_intent 相当)・授権の根拠(今の IntentStore)。寿命は今のまま
}

struct ScopeBelief {
    open: Option<Claim<bool>>,        // None = 主張なし(不確実)
    mode: Option<Claim<ModeClass>>,
    last_op: Option<ExplicitOp>,      // 最後の明示操作(診断・予測の入力)
    key_track: KeyTrack,              // 予測の追跡状態(今は窓切替で既定へ戻す。§4.5)
    half_width: HalfWidthLink,        // 半角英数トグル・戻り待ちとの結び付き(§4.10)
    last_focused_at: Tick,
}

struct Claim<T> { value: T, basis: ClaimBasis, at: Tick, fence: ClaimFence }

enum ClaimBasis {
    Explicit { origin: OpOrigin, certainty: Certainty, press: Option<PressId> },
    Observed { source: ObservationSource, confidence: ObservationConfidence },
    MeasuredPrior(PriorRule),         // U2 で所有者が認めた場合だけ使う
    UncertainDefault(WindowClass),    // D5 の既定
}

enum Certainty { Written, Predicted }   // Written = awase が書いた / Predicted = 生キーを通し効果を表から予測した
enum OpOrigin { UserKey, UserCommand, Engine, LangDetect, HalfWidthToggle, PanicReset }
enum ModeClass { Kana, Eisu, KanaInputMethod, Unknown }   // Kana = ひらがな・カタカナ(ローマ字入力)
```

- `ModeClass` と今の `InputModeState` の対応: `ObservedRomaji`・`AssumedRomaji{..}` → `Kana`、`ObservedEisu` → `Eisu`、`ObservedKana`(JIS かな入力方式、`src/engine/mode_state.rs:14`)→ `KanaInputMethod`(U5)、`Unknown` → `Unknown`。
  Engine の `InputContext` へは今の `InputModeState` に写して渡す(Engine 側は変えない)。
- **`applied` の置き場所(m2)**: `applied`(ADR-098、awase 自身の書き込みの記録)は belief の主張ではなく actuation 側の状態として、今どおりグローバルに 1 つ持ち、窓切替で `Unknown` に落とす(`reduce_focus_changed` の今の挙動)。
  `force_guards`・`observe_miss_monitor`・`focus_generation_watermark` も今どおり(`reduce_focus_changed` が窓切替で更新するものは、スコープの主張以外は変えない)。

### 4.3 更新: `evolve` の手順

1. **スコープの決定**: 観測は柵(ADR-106)が今のフォーカスと一致するときだけ今のスコープへ。明示操作は押下時点の `scope_of(foreground_tid)`(§4.1)へ。フォーカス変化は §4.5。
2. **受け入れ判定**(§4.4)。覆せなければ主張は変えず、却下の理由を journal に残す。
3. **主張の更新**: 受け入れたら、その軸の主張を置き換える。明示操作は `last_op` も更新する。`OpOrigin ∈ {UserKey, UserCommand}` の明示操作は、今と同じ規則で `write_basis` にも記録する(D8)。
4. **観測プール**: 観測は受け入れの可否によらず今どおり `ObservationStore` に記録する。プールの寿命切れは主張に影響しない(I6)。

### 4.4 優先順位と反証(D3・D4、O3)— ソース 1 つ単位の観測で成り立つ形(M1)

前提(`ebd744b6d` で確認): 観測プールは**ソースごとに最新 1 件**だけを持つ(`per_source`、`observation_store.rs`)。`derive_any` は「新鮮な High 単独 → Medium 以上の無競合多数決(1 ソースでも可)」
(`derive_actuating` との共通本体の doc、テスト `derive_open_medium_single_source`)。授権 Step 3 にも `MediumConsensus{second: None}` → `SingleIndirect` の分岐がある(`open_warrant.rs:205-210`)。
読める窓で届く Medium は通常 `ObserverPoll` の 1 ソースだけなので、「Medium が 2 つ」を条件にすると予測の外れを読める窓で覆せなくなる(退行)。よって条件は 1 ソースで成り立つ形にする。

新しい事象が今の主張を覆す条件:

| 新しい事象 | 今の主張の根拠 | 覆す条件 |
|---|---|---|
| 明示操作(ON・OFF・モード) | どれでも | **常に覆す**(O3) |
| 観測 High または Medium(信頼表で「反証可」) | 明示操作 | 読み取り開始時刻が主張の `fence`(op の時刻 + 170ms、字形は + 1300ms)より後で、プール全体で矛盾が無い(今の `derive_any` が値を返す)とき。**Medium 1 つでも覆す**(今の、settle 後の Medium 以上の観測で予測を照合して捨てる規則と同じ) |
| 同上 | 観測 | 読み取り開始時刻が新しいほうが勝つ(同時刻なら信頼度の高いほう。ADR-233 A' の順位キー)。**古い High が新しい Medium に勝つことはない**(BUG-189)。BUG-219 型(High の寿命切れ直後の嘘の Medium)は、信頼表で嘘をつく源を「反証不可」にすることで防ぐ(順位では防がない) |
| 同上 | 既定・実測の事前 | 1 つで覆す |
| 観測 Low(`FocusProbe`・`HeuristicDefault`) | 主張なし | 入る(§4.6-2) |
| 観測 Low | 既定 | 覆す(読める窓で、既定の ON より窓自身の読みを優先する) |
| 観測 Low | 明示操作・Medium 以上の観測 | **覆さない**(今の「推測は Medium 以上に負ける」と同じ。C-c6) |
| 信頼表で「反証不可」の観測 | どれでも | 覆さない。記録だけ |
| 時間の経過・観測の寿命切れ | どれでも | 覆さない(I6) |

**信頼表**(`TrustTable`、データ。初期値は今の挙動から作り、ハーネスと実機 CI で確かめる):

| 窓の種類 × IME | IMM 系(`ObserverPoll`・`ImmCrossProbe`・`FocusProbe`) | `MozcShm` | ADR-205 の監視窓(窓内の 1→0 の遷移) | ADR-188/244 の直接観測 | `TaskbarGlyph` |
|---|---|---|---|---|---|
| 読める窓 × GJI 系 | 反証可 | 反証可 | — | — | 設定が belief 配線のときだけ(今は `ui_max_level` が Observe なので不可) |
| 読める窓 × MS-IME 本体(IMC 経路) | 反証可。**ただし半角英数トグル中の開閉の読み(閉)は反証不可**(M3。`half_width_alnum.rs` の `plan_key_stage` のコメント: MS-IME 本体はトグル中の窓を `imm=C` と読む、CI run 39119247220) | — | — | — | 同上 |
| tsf-native・imm32-unavailable × GJI 系(Google 版、または Mozc 系で共有メモリの block がある) | **反証不可**(BUG-223(a)・BUG-219) | 反証可(ADR-268) | 反証可(開閉の遷移のみ。今は Mozc 系を外している) | モード軸のみ反証可(開閉軸は要確認。今は Mozc 系を外している) | 同上 |
| tsf-native・imm32-unavailable × Mozc 系で**共有メモリの block が無い**(公式 Mozc など、mozc-readable でないもの) | 反証不可(BUG-219 と同じ扱い。IMM の偽の閉の出所は未解明) | —(block が無い) | **未確定(U13)**。今は外れている(§4.14-2) | 未確定(U13) | 同上 |
| tsf-native・imm32-unavailable × MS-IME 本体 | 未確定(U7。今は TsfNative で poll が走らない) | — | 反証可 | モード軸のみ反証可 | 同上 |
| InputRelay | 反証不可(InputRelay は「この窓由来の open 観測も belief に取り込まない」、`crates/awase-windows-base/src/focus/class_names.rs:192-193`) | — | — | — | — |

- **共有メモリの `Held` の読み(§4.14-1)**: `decide_use` が `Held` を返した読み(awase の最後の操作から settle 未満で、IME の処理前かもしれない)は、今は何も報告しない。新設計では、`Held` の読みの開閉が**今の主張(MozcShm 由来)と同じとき**は観測プールの MozcShm の観測の年齢を延ばす(確認として扱う。値は変えない)。違うときは今どおり捨てる。主張はラッチなので、`Held` が続いても主張は後退しない(I6)が、観測プールを読む授権 Step 3・drift の材料のために年齢を延ばす。
- **M3 の経路**: MS-IME 本体の半角英数トグル中、読める窓では poll が「閉」を返す。これを受け入れると開閉の主張が閉にラッチされ、トグルを出る操作(モード軸だけの明示操作)の後も Engine が `ImeOff` のまま残る(I1 違反)。
  上の表の「トグル中は反証不可」で防ぐ。今は明示意図があるとポーリングが止まり、この経路に落ちないので、P6(ポーリングを止めない)を採るならこの行が必須。ハーネスにこの列を入れる(§5.2)。

**`Written` と `Predicted` の違い**は覆す条件には無い(対称)。drift・授権に入るかは `certainty` ではなく `origin` で決める(D8・§4.9)。

### 4.5 窓切替(P1、O4)

フォーカス変化で `evolve` がすること:

1. 前のスコープの行は**そのまま残す**。戻れば、その主張がそのまま効く(I3)。
2. 新しいスコープの行を引く。有ればその主張(「その窓の直前の記録」)。無ければ主張なしで §4.6。`ScopeModel=Shared` なら行は 1 つなので、主張はそのまま続く。
3. 観測プールの柵を進める(ADR-106・C-c7 のまま)。
4. 予測の追跡(`key_track`)はスコープの行に持つ(`191-gji-state-scope-spec.md` §3.4 の「GJI は同じスレッド内で閉→開でモードを保持」を予測の入力に残すため)。
5. `applied`・`force_guards` 等は今どおり(§4.2)。`InputBarrier::FocusTransition` も今のまま。
6. `write_basis` の drift の根拠は今の `last_intent` と同じく窓切替で捨てる(D8)。授権の根拠(`IntentStore`)は今のまま(hwnd キーで残る)。

消える処理: グローバルな `desired_open` の持ち越し、`plan_tsf_cache_restore`、`apply_hwnd_cache_restore`、`HwndCacheRestored`、`reset_stale_ime_on_for_imm_broken` を belief の観測として書くこと、退場時のキャッシュ保存と `MIN_FOCUS_DURATION_MS` の特例(belief 側)。

**BUG-128・BUG-218 を再発させない理由**:
- BUG-128: 粗い鍵(`(pid, class_name)`)で記録を引かない。ただし explorer の別 UWP 入力面が同じスレッドなら同じスコープ(U4、§5.5 B1・B7)。
- BUG-218: OFF の明示操作を押下時点の `scope_of(foreground_tid)` に記録する。追跡が遅れて後から入場しても、そのスコープの主張は OFF(新しいほう)。今の `plan_tsf_cache_restore` の「同じプロセスで押された、記録より新しい明示 OFF」(`tsf_cache_restore.rs:80`)を時刻の一般則に置き換える。押下時点の前面の取得がずれると戻る(リスク 4)。

### 4.6 不確実なときの既定(D5、O2)

スコープの開閉の主張が無いとき:

1. **読める窓**: フォーカス直後の同期の読み(`FocusProbe`、Low)・非同期の `ImmCrossProbe` が届けば主張にする。読みが失敗したときは 2。
2. **読めない窓、または 1 で読めなかった窓**: 開閉 = ON、モード = かな(`UncertainDefault`)。O2 のとおり。

実測の事前(「awase の起動後に作られたスレッドは閉で始まる」、`thread_scope.rs` の doc・191 spec §3.2。今の `assume_closed_for_new_thread` の条件)は、**この決定には入れない**。
この規則が当たるのは新しいスレッドの読めない窓(今の適用は Imm32Unavailable のキャッシュミス)で、O2 の対象そのものなので、使うと O2 を部分的に反転させる(M6)。使うかは U2 で所有者に問う。

**モードの主張が無いとき**: 開閉の主張が ON なら「かな」。これは今の `eisu_reset_on_ime_on`(`state/eisu_recovery.rs`)の役割を引き継ぐ。experiments エントリ 26 の E4 は、eisu reset が「Edge/TsfNative 向けの循環デッドロック対策を兼ねる」(EDIT では再読み取りが同じ役割を果たすので不要に見えるが、TsfNative/Chrome での要否は検証不能)と記録している。
新設計では、読めない窓の開閉 ON の主張に、モードが無い・古い根拠の `Eisu` だけのときは `Kana` を付けるので、「ObservedEisu が Engine を塞ぎ、IME-ON の経路が無くなる循環」は起きない。ただし GJI の同じスレッドでの閉→開は直前のモードを保持する(191 spec §3.3〜3.4)ので、スコープの追跡に変換モードがあればそれを使う。E4 と同じく、TsfNative での効果は検証できていない。

**既定 ON の失敗のモード(M7 で強めた)**:
- **端末アプリ**: Windows Terminal・WezTerm は tsf-native(読めない窓)で、長時間動くプロセスなので「新しいスレッド」にはほとんど当たらない。既定の ON は主に端末で効くことになる。端末ではコマンド入力(英字)が主で、
  実 IME が OFF のとき既定で ON に倒すと、`git status` が NICOLA の仮名のローマ字(例: 「g」の位置の文字のローマ字)になって IME に入り、IME が OFF なので英字の列として端末に入る(コマンドが壊れる)。
- 利用者が OFF キーを 1 回押せば、そのスコープに OFF の主張が残る(I3)。ただし行の寿命(1 時間)・パニックリセット・awase の再起動で既定に戻る。今は前窓の `desired_open` を持ち越すので「前の窓で OFF なら新しい窓も OFF」だったのが、新設計では「スコープごとに 1 回」になる。
- BUG-200(belief ON × 実 IME OFF で TSF 窓の最初の打鍵が raw TSF リテラル回収の Backspace で消える)の機会も増える。
- ON に倒した窓では ADR-262(最初の同時打鍵で IME ON を 1 回送る)が実 IME の OFF を直す一方、ADR-263/280 の誤判定の機会も増える。
- 所有者への確認(U10): 窓の種類・アプリ別の I2 の増加件数(ハーネス、§5.1)を示し、端末を ON 寄りの対象から外す選択肢を問う。
- 既定は授権に入れない(D8)ので、既定を根拠に IME へ書き込むことは無い。

### 4.7 Engine の活性の導出(D7、O1)

`Engine::compute_state(ctx)`(`src/engine/engine.rs:272`)の 4 条件をそのまま使い、`ctx` を新しい主張から作る:

| 条件 | 今の入力 | 新しい入力 |
|---|---|---|
| `adapter.is_enabled()` | 利用者の設定 | 同じ |
| `ctx.is_japanese_ime` | `ImeBelief::is_japanese_ime()` | 同じ |
| `ctx.ime_on` | `ImeStateHub::effective_open()` | フォーカス中のスコープの開閉の主張(無ければ §4.6) |
| `ctx.input_mode.is_romaji_capable()` | `ImeStateHub::input_mode()`(グローバル) | スコープのモードの主張を写したもの。`Kana` → `ObservedRomaji`/`AssumedRomaji`(根拠で選ぶ)。`Unknown` は下の扱い |

**読める窓の `Unknown`(m3)**: 窓切替の直後、読める窓で読みが来る前に「`Unknown` のまま」にすると、非同期の `ImmCrossProbe` が返るまで Engine が `NotRomajiInput` で非活性になり、最初の打鍵がローマ字のリテラルになりうる(今は持ち越した状態で活性のまま)。
よって、読める窓でも開閉が ON の主張(または既定)のときのモードの `Unknown` は `AssumedRomaji` として写す(§4.6 の「モードが無ければかな」と同じ)。
PR #817(マージ済み、「窓切替直後、refresh 前の最初の打鍵を保持して再生する」、BUG-214)が読める窓の最初の打鍵を保持するなら、この退行は起きにくいが、#817 の対象の窓は本 ADR の改訂では読んでいない(未確認。実装前に読む)。

### 4.8 今の構造との対応表

| 今の仕組み | 新設計での行き先 | 消える/残る |
|---|---|---|
| `ImeModel::desired_open`(グローバル・永続・初期値 true) | 明示意図の値 → スコープの主張と `last_op`、観測ゼロの既定 → §4.6、drift の比較 → `write_basis`(D8)、持ち越し → 無し(`ScopeModel=Shared` なら 1 つの主張が続く)、placeholder → 主張なし、授権 Step 4c の入力 → §4.9、TSF 復元の入力 → 無し、共有メモリの判断(`decide_use`、`platform_state.rs:2146-2149`)→ スコープの主張(実装前に読み方を確かめる) | 消える |
| `desired_is_placeholder`・`align_placeholder_desired`(BUG-163) | 主張なしのスコープに最初の観測が入ること(§4.6-1) | 消える |
| `ImeModel::last_intent` | belief 側: スコープの主張・`last_op`。書き込み側: `write_basis` の drift の根拠(**寿命は今のまま**: 窓切替・予測・通過で消える) | belief からは消え、drift の根拠として残る |
| `ImeStateHub::intent_store`(hwnd、ON 10 秒/OFF 30 秒) | belief 側: 使わない。書き込み側: 授権 Step 1 の根拠として**今のまま残す**(D8) | belief からは消え、授権の根拠として残る |
| `last_user_explicit_off_{ms,pid}`(`persistent_explicit_off`) | 明示操作の時刻と押下時点のスコープ。ON・OFF 対称 | 消える(時刻の一般則に吸収) |
| `ImeModel::key_effect.open`/`.mode`・`input_mode` | `ExplicitOp{certainty: Predicted}` の主張。照合は §4.4 | 統合。窓切替では消えない |
| `key_track` | スコープの行 | 残る(置き場所が変わる) |
| `ObservationStore` | 残る(受け入れ判定の材料・授権 Step 3 の入力)。`most_recent_trusted`・`DesiredFallback` の段は belief では不要 | プールは残る |
| `resolve_open_at` の 6 段 + `ForceGuard`、`resolve_effective` | 主張の参照 1 つ。パニックリセットは `OpOrigin::PanicReset` の明示操作(ON)として現スコープに入れ、他のスコープを主張なしにする。授権 Step 0(安全弁)は今のまま | 段は消える |
| `on_focus_process_changed` の窓種ごとの後処理 | スコープの切り替え + §4.6 | 縮む |
| `plan_tsf_cache_restore`・`apply_hwnd_cache_restore`・`HwndImeSnapshot` | スコープ表 | 消える |
| `HeuristicDefault` ON/OFF の観測 | belief 側では既定(D5)・実測の事前(U2)として読む。**授権 Step 4a の入力として、観測の書き込みは今のまま残す**(B2。消すと Step 4a の授権が変わる) | 残る(belief での意味だけ変わる) |
| `ModeKeyPassedThrough{align_desired}` | `desired_open` が無いので揃える必要が無い。通過マークは読み直しの予約と、drift の根拠(`last_intent`)を消す役割で残る | 役割が縮む |
| `reschedule_ime_refresh` の「明示意図があれば止める」 | P6 を採るなら止めない(3 つの役割の置き換えが条件、§4.12)。採らないなら今のまま | 前提つき |
| drift correction(`evaluate_drift`) | 入力を `write_basis`(利用者の操作だけ、寿命は今のまま)に。方針は ADR-212 P6 (a)(方向を問わず、明示意図があるときの再試行)のまま | 残る |
| `OpenWarrant` | §4.9 の表 | 残る |
| `ImeEvent` の列挙値 | 古い journal・不具合報告の再生のため残す(C-e6、ADR-082)。新しい事象は追加 | 残る |
| `PressLedger`・押下 ID(ADR-208) | `ExplicitOp::press` に持つ。二重書き込みの防止は今の actuation 側 | 残る |

### 4.9 書き込みの根拠と授権(D8、B2)

ADR-087 の原則(belief は誤りが可逆、書き込みの授権は不可逆。同じ bool を共有しない)により、belief のラッチ(O3)と書き込みの根拠の寿命は分けられる。

- **drift correction の根拠**: 今の `evaluate_drift`(`drift_correction.rs:127`)は `last_intent` と `desired_open` の一致を条件にする(`:133-140`、無ければ `NotExplicitIntent`)。`last_intent` は窓切替・予測・通過で消える。
  新設計では `write_basis.drift`(利用者の操作 `OpOrigin ∈ {UserKey, UserCommand}` の値と時刻)を同じ規則で持ち、同じ規則で消す。ADR-212 P6 (a) は「方向を問わず、明示意図があるときの再試行」(`docs/adr/212-*.md` の 65 行目「drift correction (a)〈明示操作の書き込みが届かなかったときの再試行〉は P6 まで残す」と 56 行目〈B1 の (a) の定義「明示意図があれば閾値0」〉)で、その範囲を変えない。
  lang-detect のひらがなキー(`lang_detect_shadow.rs` の `lang_detect_write_hiragana`、授権を通さない一回きりの送信で `applied` も書かない、ADR-262 D3)・半角英数トグル・パニックリセット・予測は drift の根拠に**入れない**。
  これで、BUG-114(experiments エントリ 20、drift correction の `FeedbackPolicy::Read` の無限再送)・ADR-087 round4 M-A(OFF の無期限化による固着)型を、belief のラッチが作ることは無い。
- **授権 Step ごとの入力と増減**(`open_warrant.rs:158-227`):

| Step | 今の入力 | 新しい入力 | 授権の増減 |
|---|---|---|---|
| 0 安全弁(`ForceGuardSet` の active な guard) | パニックリセットの guard | 同じ | 変わらない |
| 押下(ADR-208 D3、`ExplicitPress`) | この押下の向き | 同じ | 変わらない |
| 1 `IntentStore`(対象 hwnd・TTL 10/30 秒) | 利用者の明示意図 | 同じ(`IntentStore` を授権用に残す。予測・lang-detect は入れない。今も予測は `IntentStore` の行を消す〈`platform_state.rs:361`〉ので同じ) | 変わらない |
| 3 `derive_actuating`(Actuating プールの観測) | 観測プール | 同じ(プールを変えない) | 変わらない |
| 4a `HeuristicDefault` の観測 | `reset_stale_ime_on_for_imm_broken`(ON)・`assume_closed_for_new_thread`(OFF)が書く観測 | 同じ(観測の書き込みを残す。§4.8) | 変わらない |
| 4c `OwnSsot`(Blind のときだけ) | `desired_open`(既定・持ち越し・初期値 true を含む) | **案**: 利用者の操作の最後の値(`write_basis`、スコープ内)。無ければ「意図なし」(授権しない) | **減りうる**: 今は持ち越し・初期値 true で授権していた非押下の書き込みが授権されなくなる。Step 4c に届く本番の呼び出し元の列挙は未確認(V0 で旧新の授権を並べて数える) |

### 4.10 半角英数トグル・戻り待ち(ADR-107・244・245)

- **表現**: トグルに入る・出ることは、モード軸の明示操作(`OpOrigin::HalfWidthToggle`・`LangDetect`)。belief 側はモード軸の主張 1 つになる(X8)。
- **出入りの条件は今の規則のまま**。取り下げ #6(`024ca336` の「キー押下を根拠にトグルを手放す」は実 Chrome × MS-IME 本体で `か` のまま、GJI の MS-IME プリセットの Shift タップ後の無変換で `ro`、revert `6b1e91b8`)。
  トグル中の打鍵の予測は今どおり捨てる(ADR-244 M-2)。手放す条件は `half_width_alnum.rs`(`plan_leave`・`plan_key_stage`・`exit_owns_write`)と BUG-222 の修正(未マージ)の範囲に閉じる。
- **開閉軸を触らない**: トグルの出入りはモード軸だけを変える。MS-IME 本体のトグル中の「閉」の読みは開閉軸を覆さない(§4.4 の信頼表、M3)。
- **窓切替**: 離脱したスコープの行は `Eisu` と戻り待ちを持ったまま残り、新しいスコープは自分の主張を使う。戻った窓の最初の打鍵での復元(ADR-245)は今のまま書き、`Kana` の明示操作として記録する。BUG-220 の取り合いは `plan_rebuild_from_return_for_language_detect` のまま。
- **C-b3**(キャッシュの `ObservedEisu` が Engine を塞ぎ続けない): スコープの行の `Eisu` は、戻り待ち・明示操作・観測で決まったときだけ残す。根拠が古い観測だけなら、戻ったときに `Kana`(仮定)へ落とす。

### 4.11 lang-detect(ADR-263・280)・ADR-262・ADR-279 との接続

- lang-detect のひらがなキーは belief 側では `ExplicitOp{origin: LangDetect}`(開閉 ON・モード かな)の主張になるが、**書き込みの根拠には入れない**(§4.9)。誤判定の後は利用者の次の明示操作で覆る。利用者の操作より弱くするかは U8。
- 発火条件の belief はスコープの主張。既定 ON の窓が増えると発火も増える(§4.6)。
- InputRelay は lang-detect の対象外(`lang_detect_shadow.rs:315` の `lang_detect_window_in_scope`、「入力中継ツールの窓は常に対象外」)。
- ADR-262 の ON の送信は「belief・IntentStore は動かさない」(ADR-262 の summary)ので、主張として記録しない(今の挙動を保つ)。
- ADR-279(未実装)は受け入れ判定に「打鍵列による反証」の源を 1 つ足す形で載る。本 ADR では実装しない。

### 4.12 ポーリング(P6)と ADR-191 決定 3 の関係

ADR-191 決定 3 は、打鍵時の予測を `UserImeSetIntent` にしなかった理由を 2 つ書いている(「実装での単純化(round4 QB1・QB2)」): (1) `UserImeSetIntent` だと明示意図(`last_intent`)でポーリングが止まる(BUG-151 原因③)、
(2) `desired_open` への新イベントだと `resolve_open_at` で観測に負ける。(1) は `last_intent` を立てる場合の話、(2) は `desired_open` を書く場合の話で、別々の理由。

新設計では、F2 などの予測は belief の主張になるが `write_basis.drift`(今の `last_intent` 相当)は立てない(D8)。**したがって (1) のポーリングの停止は、P6 を採らなくても予測によっては起きない**(今と同じ)。
(2) は、主張がラッチされ settle 後の受け入れ可能な観測だけが覆す(§4.4)ので、鮮度の窓の内側の古い観測に負けない形で避ける。

**P6(明示意図があってもポーリングを止めない)は、別の目的(利用者の明示操作の後も読める窓で観測を続け、外部変更・書き込みの失敗に追随する)の案で、前提つき**。今の早期 return(`runtime/mod.rs:1187-1222`)は次を同時に担う(experiments エントリ 13 の教訓: 1 つのガードが複数の経路を守っている):
1. ADR-187 の通過マークの保護(「通過マークが有効な間は上書きしない」、読めない窓の分岐 `:1196`)。
2. ADR-279 P0 の `drift_follow_would_ensure_timer` の起点(`:1202`・`:1219`)。
3. コメント(`:1199-1201`)の「refresh を再開しない。GJI の I/O 推論が drift correction の根拠になるため」。ポーリングを再開すると、GJI の I/O 推論の観測が drift correction の材料に入る。
4. 未確認: 打鍵中のクロスプロセス読み取りによる「@」(BUG-113 系)の抑止を兼ねているか。
5. drift correction の発火の機会が周期 poll に結び付いて変わる(エントリ 13 は drift correction が周期チェーンに依存していたことの記録)。

P6 を採る条件: 1〜3・5 をそれぞれ別の仕組みで置き換え(通過マークの予約は今の分岐を残す、ADR-279 P0 の起点は専用タイマーのまま、GJI の I/O 推論は drift の材料から外す)、4 を §5.5 B8 で確かめる。
満たせないなら P6 は採らず、今の早期 return を残す。その場合も D2〜D5・D8 は成り立つ(予測の主張は止まったポーリングのまま保持され、読める窓での訂正は今と同じ settle 後の読み直し〈`may_change_ime` の 20ms・通過マークの読み直し〉に頼る)。
また P6 を採ると、MS-IME 本体のトグル中の閉の読み(M3)が届くので、§4.4 の信頼表の行が必須になる。

### 4.13 モードキーの効果の表(修飾付きを含む。BUG-221・ADR-281 を本 ADR に含める)

所有者の判断(2026-10-26、team-lead 経由): 「BUG-221 の残りは、ゼロベース設計の中でやる」。

**今の状態(`ebd744b6d`)**: BUG-221 は ADR-281 の A′ で修正済み・所有者の実機確認済み(Windows Terminal × GJI、Ctrl+F2)。A′ の範囲(ADR-281 D1): GJI/Mozc 系は 0xF0〜0xF4 × Ctrl・Shift・Alt(組み合わせ含む)を素のキーとして予測、
Microsoft IME 本体(CLSID で同定できたときだけ)は 0xF2 × Ctrl だけ、それ以外は抑止。Win 付きは抑止。判定は届いた VK。根拠は Mozc `keyevent_handler.cc:330-352` と CI(e2e-ime `sc-adr281-*`、run 40272737475・40274039585、ADR-281 の記載)。
未実施: MS-IME 本体の 0xF0・0xF1・0xF3・0xF4 × 修飾、D2'(GJI の Converter I/O による確認)。

**新設計での扱い**:
1. 修飾付きも、A′ の表で効果が決まるセルなら `ExplicitOp{certainty: Predicted}` で、「最後の明示操作が勝つ」の対象。決まらないセルは明示操作にしない(観測に委ねる)。
2. 効果の表の鍵は `(IME の種類, プリセット, 届いた VK, 修飾の組, 状態)`。今の述語(`plan_key_effect_track` の `ModeKeyModifiers::{Unknown, DroppedByGjiMozc, MsImeNativeCtrlOnHiragana}`)を、IME の種類ごとの写像表(修飾 → 素のセルへ写す/抑止)のデータにする。
   各行に根拠の種類を付ける: (a) 実測の格子(`sc-adr281-*`)、(b) IME の仕様の読み(Mozc のソース。GJI は (a) で裏付ける)、(c) IME の設定の読み取り(`config1.db`・MS-IME 本体のレジストリ、今の `key_effect_keymap`)、
   (d) 注入学習(ADR-195)は修飾付きには使わない(ADR-281 の却下案: Alt+F4・単語登録・ローマ字/かな切替を踏む)。(a)〜(c) が無いセルは抑止。
3. D2'(GJI の F2 の後の Converter I/O で予測を確定・取り消す)は、読めない窓 × GJI 系の開閉の観測の源の候補(信頼表の 1 列)。まず観測のみ(§5.5 B6)。
4. ADR-281 の Shift の注意(ADR-187 の追随 `mode_key_follow_admits_modifiers`〈`mode_key_pass.rs:265`〉は Shift で止まるので、読める窓の訂正は `may_change_ime` の 20ms の読み直しに頼る)を引き継ぐ。

**予測の入力の作り直し(別件の報告の扱い)**: 「`ObservedEisu` 中の F2 は no prediction で Engine が活性に戻らない」(BUG-221 とは別件。BUG-222〈未マージ〉のログにも「F2 6 回とも no prediction」)。起草時に読んで確かめたこと:
- `kp_predict_key_effect`(`key_pipeline.rs:1908`)の `PredictInput` は、`open` = `ime.effective_open()`(`IntentStore` の上書きを含むグローバル値)、`mode` = グローバルな `input_mode()`、`conv_raw` = `belief.prev_conversion_mode()`、
  `track` = `key_track()` を持続半角英数トグル中は変換モードを捨てて渡す(`without_conv_while_half_width_alnum`、ADR-244 M-2・BUG-192)、ほかに `unreadable`・`observable_toggle_guard`(ADR-260)。
- `predict_in_table`(`key_effect_predictor.rs:555`)は変換モードを `track.conv` → `conv_raw` → (`ObservedEisu` なら `C10`、でなければ `C19`)で決め、`observed_conv_is_unrepresentable_kana`(BUG-212、`:473`)なら `None`。段階 `Indeterminate`(BUG-211)は次のモードキーを予測しない。
- GJI の表(`grid-tables/atok.json`・`msime.json`)には `on-c10-none|hiragana → ON/0x19` のセルがある。`msime-native.json` には `on-c10-*|hiragana` が無い。
- よって GJI で `ObservedEisu`・開・段階なしなら予測が出るはずで、原因は `conv_raw`・`Indeterminate`・`observable_toggle_guard`・開閉の入力のどれか。**未確認**。
- 新設計では、予測の入力をフォーカス中のスコープの主張から作る(`open`・`mode`・`track`・`conv` をスコープの行から。他のスコープ・古い文脈の `prev_conversion_mode` を使わない)。トグル中の扱いは変えない。V0 で BUG-222 の列をハーネスに入れて原因を特定してから決める。

### 4.14 別セッションの Opus コードレビューからの入力 4 件(team-lead 経由、`origin/develop` `82e00b655` で確認した結果)

1. **共有メモリ観測が打鍵中に失効する**(確認: コードの読み): `report_mozc_shm`(`crates/awase-windows/src/state/platform_state.rs:2138-2182`)は `decide_use`(`crates/awase-ime-core/src/state/mozc_shm.rs:397-418`)が `Use` のときだけ `ObserverReported` を出し、
   `Held`・`Split` では何も報告しない。`Held` の条件は `read_is_after_baseline` が偽(読んだブロックの `update_tick_ms` が基準より古く、かつ基準から `KEY_EFFECT_SETTLE_MS` 未満)。
   基準は `hook::last_operation_tick_ms()` = `max(hook_alive_tick_ms, 最後の ImmCross の書き込み)`(`crates/awase-windows/src/hook.rs:345-350`、doc に「全キー〈自分の SendInput と KeyUp を含む〉」)。
   IME の状態が変わらない打鍵中はブロックの `update_tick_ms` が進まないので、170ms より短い間隔でキーが続くと `Held` が続き、`MOZC_SHM_OBSERVATION_TTL_MS`(1000)で観測が失効しうる。
   **打鍵中に実際にどのくらい `Held` が続くかは未実測**(§5.5 B10)。設計への反映: 主張のラッチ(D2・I6)で、失効しても主張は後退しない。加えて §4.4 の「`Held` の読みの開閉が今の主張と同じなら観測の年齢を延ばす」。
2. **`gji_evidence_absent()` が「共有メモリがある」ことを確かめていない**(確認: コードの読み): `direct_mode_key_watch_kind_for`・`external_change_watch_applies_for`(`crates/awase-windows/src/runtime/mod.rs:2501-2522`)は、
   `gji_evidence_absent()`(`crates/awase-windows/src/tsf/observer.rs:545`、`converter_target` が `NoEvidence` = GJI 系〈表示名の "Google"/"mozc" 一致〉で Google 版でない、`crates/awase-windows-base/src/tsf/converter_target.rs:30-35`)のとき、
   ADR-188 の直接読みと ADR-205 の外部変更の監視を外す。コメントの理由は「Mozc 系は開閉を共有メモリ(ADR-268、High)が担う」。共有メモリの block が実在するか(`MozcShmShadow::block_present()`、`runtime/mozc_shm_shadow.rs:34`)は見ていない。
   よって公式 Mozc など共有メモリを出さない Mozc 系では、読めない窓で外から IME を切り替えても追随する証拠が無い(**実機での症状は未確認**)。設計への反映: 信頼表に「共有メモリの block があるか」の軸を足した(§4.4)。block が無い行で直接読み・監視窓を戻すかは U13(§5.5 B9 で測る)。
3. **BUG-220 の修正が、戻った窓 A の実 IME は半角英数のままと仮定している**(確認: コードの読み): `lang_detect_shadow.rs:418-444` は、戻り待ちのある窓で英数と判定すると、`plan_rebuild_from_return_for_language_detect`(`crates/awase-hook-core/src/state/half_width_alnum.rs:351-366`)が真なら、
   IME へ送らずにトグルを立て直す(doc: 「窓 A の実 IME は、離脱で何も送っていないので半角英数のまま〈ADR-245 の事実〉」)。条件は入りの方針・`entry_ime_ok`・IME 経路の一致だけで、窓 B での操作や `hiragana_key_is_set` は見ていない。
   一方 `half_width_alnum.rs` の `plan_key_stage` のコメント(`:738-740`)は「偽を持ち越しとみなすのは TSF 系(GJI)の窓だけ」と書き、GJI の TSF 窓では窓をまたいで状態が持ち越されうる前提を持つ。
   **2 つの前提が食い違う**: A と B が IME の状態を共有するなら、B でひらがなにした後 A に戻ると実 IME はひらがなで、A のトグルの立て直しは Engine を非活性のまま素の英字をひらがなの IME へ流す(**実機で起きるかは未確認**。BUG-220.md も実機未確認)。
   設計への反映: これは §4.1 の粒度(PerThread か Shared か)の問題そのもの。PerThread なら B の操作は A のスコープに届かず、A のラッチ(`Eisu`)が正しい。Shared なら B の操作が A に効くので、A の `Eisu` は B の操作で上書きされているべき。
   新設計では、戻り待ち(ADR-245)の行もスコープの行に持たせ、`ScopeModel` が Shared の構成では B の明示操作が A の戻り待ちを無効にする(同じスコープなので)。BUG-220 の修正のコードは本 ADR では変えない。§5.5 B1 に「GJI の TSF 窓で B でひらがなにして A に戻る」セルを入れる。
4. **BUG-212 の早期 return が段階の追跡の更新まで飛ばす**(確認: コードの読み): `predict_in_table` の `observed_conv_is_unrepresentable_kana(input)` の `return None`(`crates/awase-key-effect/src/state/key_effect_predictor.rs:594-596`)は、
   `find_in`・`indeterminate_stage_without_cell`・Esc の段階の規則(BUG-162、`:610-633`)より前にある。よって半角カナ(conv 0x13 など)のまま Enter/Esc を押すと、段階の追跡(`track.stage`)が更新されず古い `Typing` が残りうる(**再現は未確認**。テストも未確認)。
   設計への反映: §4.13 の予測の入力の作り直しで、「開閉・モードを予測しない(表せない値)」と「段階の追跡を更新する」を分ける(BUG-162 の規則は表せない値でも働かせる)。スコープの行の `key_track` に入る値の正しさに関わる。

含めないもの(team-lead の指示どおり別経路): Zoom の key_pacer の Recovered 取りこぼし、lang-detect の保持破棄で Shift が押されたままになる件、パニックリセットが lang-detect の保持を解放しない件。

## 5. 検証計画

### 5.1 基準の測定(PR #857)との関係: shared と per-thread の両方を主にする

- オラクル(`belief_engine_invariant.rs`)を新の核にも当てる: 同じ事象列を旧(本物の `ImeStateHub`)・局所修正 L1〜L3(§9)・新(`ScopeModel` を真に一致/不一致の 2 通り)へ流し、I1・I2 を並べる。
- **合格条件(モデル別)**:

| 真のモデル | 新の `ScopeModel` | I1 の合格条件 | I2 の合格条件 |
|---|---|---|---|
| per-thread | PerThread(一致) | 核の列(予測の外れを除く)で 0。今の 9 本(核の字母)のうち予測の外れ・入力モード軸以外が 0 | 既定の区分以外 0。既定の区分を含めた総数を旧(57 本、核の字母)と並べて報告 |
| shared | Shared(一致) | 核の列(予測の外れを除く)で 0。BUG-223 の 4 本が違反でない | 同上(旧 35 本、msime 核の字母) |
| shared | PerThread(不一致) | 件数を報告(§4.1 の両立しない条件)。BUG-223 の 4 本の合否を記録 | 件数を報告 |
| per-thread | Shared(不一致) | 件数を報告 | 件数を報告 |

- 外部変更・嘘の観測を含む列は件数を並べて報告する(0 は目標にしない)。
- **ゼロベースを採る判断(V0 の出口)**: 局所修正 L1〜L3(と組み合わせ)が、上の「一致」の行の合格条件を満たすなら、ゼロベースの規模は不要で、局所修正 + 必要な部分(§4.13 の写像表など)だけを採る案を所有者に示す。満たさない部分が残るなら、その型を新設計が消すことを示して進む。

#### 5.1.1 基準の測定の数値(再確認した値)と、違反の型ごとの新設計の対応

数値は `origin/test/belief-engine-invariant`(HEAD `37fc37e75`、PR #857 マージ済み)の文書と golden を `git show` で読んで引いた。

| 設定 | 範囲 | I1 違反(T=0) | 2000ms 事象なしで残る | poll が 1 回届いても残る | 最小列 | うち「awase の操作・窓切替・正しい観測だけ」 |
|---|---|---:|---:|---:|---:|---:|
| atok shared | 全字母 19 種・長さ 3 | 315 | 315 | 290 | 31 | 18 |
| msime shared | 同 | 405 | 405 | 375 | 65 | 36 |
| msime-native shared | 同 | 405 | 405 | 375 | 65 | 36 |
| msime shared | 小さい字母 7 種・長さ 4 | 136 | 136 | 136 | 4 | 4 |
| msime per-thread | 核の字母 12 種・長さ 3 | 9 | 9 | 5 | 9 | 6 |

- I2(参考): 核の字母・長さ 3 で atok 43・msime 35・msime-native 35・msime per-thread 57。最小列の大半は嘘の観測(`obs:conv-infer-open`)を含む。
- 違反が出た時点の窓(msime shared): chrome 93・relay 93・wezterm 93・xaml 30・explorer(std) 84・explorer(imm32-unavailable) 12。読めない窓で終わる違反は poll が届いても 100% 残る。
- 「読めない窓で 100% 残る」違反を新設計が減らすのは、主に既定の ON(D5)による。同じ窓で I2(誤って ON)が既定の区分に移るので、I1 の改善と I2 の増加を必ず並べて示す(M4)。

違反の型(msime shared の分類、T=0)ごとの対応:

| 型 | 件数 | 何が起きているか | 新設計での扱い | 期待 |
|---|---:|---|---|---|
| `desired-fallback` | 297 | ON の予測が窓切替で捨てられ、前窓の `desired_open`(false)に戻る | `ScopeModel=Shared` なら 1 つの主張で F2 の ON が続く。PerThread なら新しいスコープの記録/既定(読めない窓は ON) | Shared(一致)で 0。PerThread(不一致)では読めない窓は既定 ON で消え、読める窓は件数を報告 |
| `derive-high` | 30 | 起動時の High(閉)が鮮度 3000ms の間、後の Medium より勝つ。最小列 `external-on` | 新しい受け入れ可能な観測が勝つ(古い High は新しい Medium に勝たない)。外部変更なので観測が届くまでは残る | poll が届いた後に 0 |
| `explicit-intent` | 23 | 明示意図の false が真の ON と食い違ったまま | 読める窓は settle 後の観測が覆す。読めない窓は残る | 読める窓で poll 後に 0 |
| `most-recent-trusted` | 21 | 鮮度切れのフォールバックが閉 | 鮮度切れで後退しない。外部変更なら残る | 外部変更を含まない列で 0 |
| `key-effect` | 18 | 予測の閉が真の ON を覆う(外部変更との組み合わせ、または予測の外れ) | 外部変更・予測の外れの区分で数える | 件数を報告 |
| `ObservedEisu` | 10 | belief は開だが入力モードが半角英数のまま | 予測の入力の作り直し(§4.13)で減るか V0 で確かめる | 件数を報告 |
| `derive-medium` | 6 | Poll の単独の嘘の false | 読める窓の Poll は反証可なので覆す(嘘の観測の区分) | 件数を報告 |

- 長さ 5〜6 の全列挙は基準の測定でも未実施。V0 では長さ 4(小さい字母)とランダム探索(長さ 8〜12)。

### 5.2 ハーネスの拡張(擬似窓が写していないもの)

| 写していないもの | 必要性 | 扱い |
|---|---|---|
| `on_focus_process_changed` の hwnd キャッシュ復元・TSF の ON キャッシュ復元・新スレッドの閉の仮定 | 旧が「戻る列」で救われる分を数える(基準の測定の偽陽性の疑い) | 旧の比較用に写す。§5.5 B7 の実測を基準線にする |
| per-thread で「awase の起動前からあるスレッド」(真の初期状態が開/閉どちらも) | 既定の区分の件数 | 窓の初期状態を開・閉の両方で列挙 |
| 押下時点の前面とフォーカスの追跡のずれ(BUG-218) | §4.3-1 の帰属 | `focus_to` の遅延 |
| 通過マーク・パニックリセット | 通過後の観測、パニックリセット | 事象として足す。`applied` の往復は実機 CI |
| lang-detect の保持・半角英数トグル・戻り待ち | BUG-186・193・220・222 の列。取り下げ #6 の失敗の列(実 Chrome × MS-IME 本体、GJI の MS-IME プリセットの Shift タップ後の無変換) | 拡張が要る |
| **MS-IME 本体の半角英数トグル中の「閉」の読み**(M3) | 信頼表の「トグル中は反証不可」 | 擬似 IME に「MS-IME 本体のトグル中は IMM の開閉の読みが閉」を足し、トグルを出た後の I1 を見る列を入れる |
| 周期の poll・settle 後の読み直し | P6 の効果 | 事象として足す |
| 読めない窓の IMM の嘘(BUG-223(a)) | 信頼表の「反証不可」 | 字母に足す |
| `user_enabled` の切り替え | I8 | 事象として足す |
| 修飾付きのモードキー | I10。MS-IME 本体の Shift/Alt+F2 は開かないのに、今の擬似 IME は開く | 擬似 IME の遷移表に「届いた VK × 修飾」の行。初期値は ADR-281 の CI の結果、未測定のセルは §5.5 B4 |
| 予測の入力(`unreadable`・`observable_toggle_guard`・`conv_raw`) | §4.13 の作り直しの検査 | 本番と同じ入力の作り方を写す。BUG-222 の列 |
| 授権の入力・`WriteCommand` | I7・§4.9 の表 | 授権 Step 4c に届く入力を写す |
| 窓の種類で混ぜた窓モデル(Win32・実 Chrome は per-thread、MS-IME の WT は入場時に開を引き継ぐ。§1.4 の測定) | V0 の評価、中間案 L4 の評価 | 擬似 IME の窓モデルに「入場時に開を引き継ぐ」窓の種類を足す |
| 長い時間の経過(`advance:11000ms`・`31000ms`・`3599000ms`) | TTL を持つ局所修正 L3 の比較・I6 | 字母に足す |
| `Engine::on_input`・flush | I9 | Engine の単体テスト |
| TSF の composition・cold/warm | 新設計は扱わない | 実機 CI と所有者の実機 |

### 5.3 その他の検査

- **参照モデルとの差分**: §4.4 の表だけから書いた小さな参照モデル(テスト内)とランダム列で突き合わせる。
- **I4 の変成関係**: ランダム探索で回す。
- **突然変異テスト**: `cargo mutants` を新の核に当て、受け入れ判定の各条件(柵の不等号・信頼表のセル・`origin` の分岐)が殺されることを確かめる(`git diff develop | cargo mutants --in-diff -`)。
- **台帳の回帰列**: §6 で「引き継ぐ」「代替で守る」にした各規則の事故の最小列をハーネスに足す。

### 5.4 段階 V0: 純粋な核を擬似 IME の上でだけ動かす(本番は触らない)

0. **前提条件**: §5.5 の B1・B2(保持単位・新スレッドの初期状態)の結果が出ていること(`ScopeModel` の初期値を決めるため。結果が出るまでは両方の値で回す)。局所修正 L1〜L3 の測定(belief-oracle)の結果があること。
1. `crates/awase-ime-core` に新の核(`BeliefStore`・`evolve`・`decide`・`derive`・`TrustTable`・`WriteBasis`)を足す。本番からは呼ばない。
2. ハーネスに新の核のアダプタを足し、§5.1 の旧・局所修正・新の比較を CI の `test` ジョブで回す。
3. **終わりの判定**: §5.1 の「一致」の行の合格条件、I3〜I8・I10 の検査、台帳の回帰列が全部通る。§5.2 の拡張のうち必要な行がそろっている。
   所有者が、既定の区分の I2 の増加(窓の種類別)・外部変更の件数・局所修正との比較を見て、ゼロベースを進めるか、局所修正を採るかを判断する。
4. 戻し方: 本番に配線していないので、モジュールを消すだけ。

### 5.5 CI 検証バッチ(設計が前提にしている実機の事実をまとめて確かめる。今は実施しない)

所有者の了承(2026-10-26、team-lead 経由): 「ゼロベース設計の文脈の中で、必要なら CI を使った検証をまとめてやる」。起草後に一括で実行する。優先度順。構成名は案。

| # | 何を測るか | 構成(既存のハーネス) | 合否・読み方の基準 | 結果で設計がどう変わるか | 実施に要る変更(粗い規模) |
|---|---|---|---|---|---|
| B1 | **(一部実施済み: run 40327981441。§1.4。残り: GJI × WT・WezTerm・UWP/XAML・同じプロセスの別スレッド・マウス/タスクバー・試行数の追加)** **IME の開閉・変換モードの保持単位**(スレッドか、窓か、全体か)を、**実アプリ**(Chrome・Edge・Windows Terminal・WezTerm・VS Code・WebView2 のホスト・UWP)と SPI=0/1 で。GJI(ATOK/MS-IME プリセット)・MS-IME 本体(**TSF で本体と同定された構成**)・Mozc。加えて、`scope_of(foreground_tid)`(§4.1)の鍵と IME の状態が切り替わる単位が一致するか | 既存の結果は 191 spec の CI(自前の Win32 EDIT、SPI=0、IMM32 同定の MS-IME)。プローブとブランチは今は無い。実アプリの窓の作り方は e2e-ime の `sc-bug149-chrome-*`(実 Chrome)・`wt-probe.yml`(Windows Terminal) | 各セル(IME × アプリ × SPI × 移動の種類)で、移動先・戻った後の開閉・変換モードを 2 回以上一致して読めること。読めない窓は TSF の compartment で読む | `ScopeModel` の表の初期値が決まる。Shared の構成があれば U11(O4 との衝突)を所有者に問う。鍵の関数が単位と合わなければ §4.1 を直す | プローブの作り直し(examples に 1 本、数百行)、ワークフロー 1 本、集計スクリプト 1 本。実アプリの起動は e2e-ime の部品を流用 |
| B2 | **新しいスレッドの窓は閉で始まるか**(U2)を、TSF で同定した MS-IME 本体 × 新スレッドと、実アプリ(Chrome の新しい窓)で | B1 と同じプローブ。191 spec §3.2 は「同定済み MS-IME 本体 × 新スレッド=閉の経路は CI で未検証」 | 各構成で 3/3 閉 | U2 の判断材料(実測の事前を使うか) | B1 に含める |
| B3 | **窓切替から IME の状態が確かになるまでの遅れ**(読める窓で最初の受け入れ可能な観測まで。読めない窓で MozcShm・監視窓・字形まで) | `sc-focusrestore-*`(`typing_stress --mode=focus-restore`)に、journal の「窓切替 → 最初の受け入れた観測」の時刻差を集計する判定を足す | 分布(中央値・最大)。測定 | 読める窓の既定(§4.6-1 の待ちの間)とリスク 9 の見積もりに使う。TTL の根拠にはしない | 判定スクリプト 1 本、構成の追加数行 |
| B4 | **BUG-221 の残り (1)**: MS-IME 本体の 0xF0・0xF1・0xF3・0xF4 × Ctrl・Shift・Alt の効果(閉と開から) | `sc-adr281-nat-*`(`--chord`、`tools/e2e/ime_key_matrix/check_modifier_mode_key.py`)をキーの数だけ広げる | 各セル 3/3 一致。素のセル(`msime-native.json`)と一致するか。Ctrl の KeyUp 後の競合(ADR-281 D2)が出ないこと | 一致したセルを §4.13 の写像表と擬似 IME の修飾の行に足す | `e2e-ime.yml` の構成の追加(十数行)、判定のキーの追加 |
| B5 | **BUG-221 の残り (2)**: 修飾付き F2 の直後の最初の NICOLA 打鍵が漏れないことを文字レベルで | `sc-adr281-awase-*` に F2 直後の NICOLA 打鍵を流し、窓の文字を読む判定を足す(Windows Terminal と自前の窓) | 3/3 で期待の仮名、素の `E` が出ない | 漏れるなら明示操作の反映の時点を見直す | 構成の追加と文字の判定(`typing_stress` の文字照合を流用) |
| B6 | **D2'(観測のみ)**: GJI で F2(素・修飾付き)の後の Converter I/O(`gji_last_write_ms`)の増加と実 IME の開閉の一致。閉のまま残った場合に増えないこと。変換(Reconvert)の I/O との区別 | GJI 構成で awase を起動し、journal に「予測・I/O の増分・実 IME の読み」を記録(belief は書かない) | 一致率と遅れの分布(待ち時間の定数の実測) | 信頼表に入れるか決める(§4.13-3) | 観測専用のログ(数十行)、判定スクリプト 1 本 |
| B7 | **TSF 窓の ON キャッシュ復元の効き方**(BUG-128・218。基準の測定の偽陽性の疑い) | 実 Chrome・Windows Terminal の構成にフォーカスの往復を足す | 旧(develop)の結果を記録(合否なし) | 旧で救われる列を基準線にする(§5.2) | 構成の追加 |
| B9 | **共有メモリの無い Mozc 系(公式 Mozc)× 読めない窓(実 Chrome)で、外から IME を切り替えたとき(タスクバー・マウス・注入キー)に追随する証拠があるか**(§4.14-2) | e2e-ime の Mozc 構成(`*-mozc-*`)に公式 Mozc を足し、`sc-bug149-chrome-*` の実 Chrome の窓で外部切替の手順を流す | 外部切替から Engine が追随するまでの時間(または追随しない)を記録 | 追随しないなら U13(block の無い行で直接読み・監視窓を戻すか)の判断材料。IMM の偽の閉が出るかも記録 | 構成の追加(数十行)、公式 Mozc の導入手順 |
| B10 | **打鍵中の共有メモリの読みが `Held` になる割合と、MozcShm の観測の失効の回数**(§4.14-1) | `mozc-shm-e2e.yml`(Mozc Readable)で打鍵ストレスを流し、`[mozc-shm]` の診断(`mozc_shm_shadow = true`)に `Held` の件数と失効を出す(観測専用のログを足す) | 割合と失効の回数を記録 | §4.4 の「同じ値の `Held` で年齢を延ばす」の効果の見積もり。旧(今の develop)で BUG-219 型の窓が開く頻度 | 観測専用のログ(数十行)、ワークフローの手順の追加 |
| B8 | **P6 の副作用**: 明示意図があっても読める窓でポーリングを続けたとき、打鍵中のクロスプロセス読み取りで「@」(BUG-113 系)が出ないか。GJI の I/O 推論が drift の材料に入らないか | P6 を V1 の shadow で有効にした版で、GJI × Windows Terminal の `sc-*` と打鍵ストレス `ts-*` | 「@」0 件、drift の送信件数が旧と同じ | 出るなら P6 を採らない(§4.12) | V1 の段階 |

- B1・B2 は P1・U2 の前提そのものなので最初に回し、**V0 の前提条件**にする。B4〜B6 は §4.13 の写像表を埋める。B3・B7 は基準線。B8 は V1 以降。
- いずれも GitHub Actions の windows-latest(Windows Server)での結果で、所有者の実機とは違いうる。

## 6. 台帳との照合(棚卸し 02-ledger の各規則)

「引き継ぐ」= 今の規則・実装をそのまま使う。「代替で守る」= 目的を別の仕組みで守る。「不要」= 起きる経路が無い(理由つき)。

| 出典 | 守っている規則 | 新設計 | 理由・どこで |
|---|---|---|---|
| BUG-46・052・116、ADR-191 | 物理 IME キーの配送(awase が書くトグルの KeyDown だけ Suppress) | 引き継ぐ | `physical_disposition.rs::plan_core` は belief の外 |
| BUG-110 | 明示意図が無い間は HeuristicDefault だけを根拠に drift correction を出さない | 引き継ぐ | drift の根拠は `write_basis`(利用者の操作だけ、寿命は今のまま)。既定・予測は入らない(§4.9) |
| BUG-110 追補1(ADR-279) | (未実装)残った明示 OFF を打鍵列で取り下げる | 接続のみ | §4.11 |
| BUG-113 | `shadow_on` を `Option<bool>` のまま扱う | 引き継ぐ。**ただし P6 との関係は未確認** | actuation 側は変えない。P6 を採ると打鍵中の読み取りが増え、「@」との関係は §5.5 B8 で確かめる(m8) |
| BUG-114(experiments エントリ 20) | drift correction の `FeedbackPolicy::Read` の無限再送を止める | 引き継ぐ | drift の根拠の寿命を今のままにする(§4.9)。belief のラッチは drift に入らない |
| BUG-128 | TSF 窓への ON 復元は hwnd 一致 | 代替で守る | 粗い鍵を使わない(§4.5)。explorer の UWP 入力面のスレッド構成は未確認(U4・B1) |
| BUG-151 | 通過したモードキー後は `applied` に依らず読み直す / 明示意図でポーリングが止まらない | 引き継ぐ | 予測は drift の根拠(`last_intent` 相当)を立てないので止まらない(§4.12)。`explicit_verify`・`mode_key_pass_live` は残す |
| BUG-157 | 通過キーの後、観測から `desired_open` を揃える | 不要 | `desired_open` が無い。drift の根拠は予測で立たないので、F2 の通過を drift が閉じ直す経路が無い |
| BUG-163 | placeholder の `desired_open` を最初の観測に揃える | 代替で守る | 主張なしのスコープに最初の観測が入る |
| BUG-186 | 直接観測を広げ、NATIVE の読みに追随したらトグルを書き込みなしで手放す | 引き継ぐ | §4.10 |
| BUG-189・ADR-233 | 新しい Medium が古い High に勝つ | 引き継ぐ | §4.4(1 ソースの新しい観測で覆す) |
| BUG-193・ADR-245 | 離脱時に書かず、戻った窓の最初の打鍵で復元 | 引き継ぐ | §4.10 |
| BUG-194・200・207 | raw TSF リテラル回収の宛先照合・Mozc 時の監視外し | 引き継ぐ | BUG-200 は既定 ON で機会が増えうる(§4.6) |
| BUG-205・211・212 | 予測器の段階規則・表 | 引き継ぐ | 外れが窓切替をまたいで残る(§2.1)。BUG-212 の早期 return が段階の追跡まで飛ばす点(§4.14-4)は、予測の入力の作り直しで「予測しない」と「追跡を更新する」を分ける |
| BUG-221・ADR-281 | A′(修飾付きを素のキーとして予測) | 引き継ぐ+範囲を本 ADR に含める | §4.13。残りは §5.5 B4〜B6 |
| (別件)`ObservedEisu` 中の F2 が no prediction | (原因未特定) | 確認の対象 | §4.13 の最後 |
| BUG-218 | 入場先と同じプロセスで押された、記録より新しい明示 OFF があれば復元しない | 代替で守る | 押下時点のスコープに記録し、新しいほうが勝つ(§4.5) |
| BUG-219 | Mozc 系を `[direct-follow]` から外す | 代替で守る | 信頼表で GJI 系の読めない窓の IMM を反証不可(順位では防がない)。外す条件は「Mozc 系」ではなく「共有メモリの block がある」に寄せる案(§4.14-2、U13) |
| (入力)MozcShm の `Held` の読みで観測が失効 | (規則なし) | 代替で守る | 主張のラッチ + 同じ値の `Held` で年齢を延ばす(§4.4・§4.14-1)。頻度は B10 |
| BUG-220 | 戻り待ちのある窓で英数と判定したら戻り待ちから立て直す | 引き継ぐ(前提を確認) | 「A の実 IME は半角英数のまま」という前提は粒度が PerThread のときだけ正しい(§4.14-3)。Shared の構成では B の操作で無効にする。コードは本 ADR では変えない |
| BUG-222(未マージ) | lang-detect が入れたトグルは、tsf-native × GJI 系で素の F2 で手放す | 引き継ぐ(マージされた形で) | 範囲を広げない |
| BUG-223 | (修正なし) | 新設計の目標 | §5.1(真のモデル別の合格条件) |
| C-b3 | キャッシュの `ObservedEisu` が Engine を塞ぎ続けない | 代替で守る | §4.10 |
| C-b4・experiments エントリ 26 E4 | IME-ON の経路に `ObservedEisu` の救済を対で置く(循環デッドロック対策) | 代替で守る | §4.6 の「モードが無い・古いならかな」。TsfNative での効果は E4 と同じく検証不能。ガードテストは新の経路に合わせて書き直す |
| ADR-087・089 | belief と授権の分離、観測プールの Actuating/BeliefOnly | 引き継ぐ | D8・§4.9 |
| ADR-098 | `applied` は actuation を試みた経路だけが書く | 引き継ぐ | §4.2 |
| ADR-106 | フォーカスの epoch+hwnd の柵 | 引き継ぐ | §4.1・§4.5 |
| ADR-165 | 時間だけで復元を抑制しない | 引き継ぐ | 取り下げ #5 |
| ADR-179・212・213 | 予防的・補正的な書き込みをしない | 引き継ぐ | D9 |
| ADR-187 | 通過後に読み直し、古い明示意図を捨てて観測に追随 | 引き継ぐ+代替 | 通過マークの読み直し・drift の根拠を消す役割は残す。belief 側は新しい観測が勝つ |
| ADR-188・244・205・268 | 外から変わったことへの追随 | 代替で守る | 信頼表の「反証可」の源 |
| ADR-191 決定 1・3 | IME が状態の正。表は予測、観測で確認・訂正 | 引き継ぐ | §4.12 |
| ADR-199 | 能動に書くのはトグルの役割のキーと `keys.ime_*` だけ | 引き継ぐ | D9 |
| ADR-208 | 押下 ID | 引き継ぐ | actuation 側 |
| ADR-212 P6 (a) | 明示意図があるときの再試行(方向を問わず) | 引き継ぐ | §4.9 |
| ADR-214 | (保留) | 関係なし | `applied` を変えない |
| ADR-255 | IME OFF のときだけ無変換/変換を Space に(Engine の belief で判定) | 引き継ぐ | belief の出所が変わるだけ |
| ADR-260 | 読める窓では開状態の英数トグルの入力モードを予測しない | 引き継ぐ | 予測器側 |
| ADR-262・263・280 | 観測の無い窓の補正・保持・言語判定 | 引き継ぐ | §4.11 |
| ADR-273 | 字形の柵 1300ms | 引き継ぐ | §4.4 |
| 定数(棚卸し 02-ledger §D) | `OBSERVATION_FRESH_WINDOW_MS`=3000 等 | 引き継ぐ | 値は変えない。3000 は観測の材料の鮮度として残り、主張の寿命には使わない。`EXPLICIT_ON/OFF_INTENT_TTL_MS` は授権の根拠に残る。`EXPLICIT_OFF_CACHE_SUPPRESS_MS` は belief では不要(書き込み側の読み手は実装前に確かめる) |

## 7. 取り下げた方式との照合(棚卸し 03-constraints (d) の 9 件 + experiments エントリ 13・20・26)

| # | 取り下げた方式 | 新設計は繰り返していないか |
|---|---|---|
| 1 | awase が観測・belief に合わせて IME へ書き直す補正 | 繰り返さない。`decide` は書き込みを返さない(D9)。drift・授権の根拠は利用者の操作だけで寿命は今のまま(D8)。lang-detect・予測・既定は書き込みの根拠に入らない(B2) |
| 2 | 観測が無いことを根拠に意図を偽装する | 繰り返さない。既定は `UncertainDefault` という自分の名前の根拠で持ち、明示操作とも観測とも名乗らない |
| 3 | 「この窓種には日本語 IME が無い」という推測を観測として書く | 繰り返さない。窓の種類から作るのは既定(ON)だけで、観測としては書かない |
| 4 | 信頼度優先で古い High を残す(BUG-189) | 繰り返さない。新しい観測が勝つ(§4.4)。BUG-219 型は信頼表で防ぐ |
| 5 | 時間だけで「OFF の後の復元」を抑制する | 繰り返さない。時刻の新しさで決める |
| 6 | キー押下を根拠にした半角英数トグルの手放し | 繰り返さない(§4.10) |
| 7 | IME OFF に送るキーを動かす | 繰り返さない |
| 8 | 実測なしの暫定定数を増やす | 繰り返さない。行の寿命は `HWND_CACHE_MAX_AGE_MS` を流用(§4.1、意味の違いを明記) |
| 9 | 窓切替で「ON へ戻す」方向の修正は BUG-128・218 を再発させうる | 粗い鍵を使わない・押下時点のスコープに記録する。**既定の ON(D5)は記録の無い読めない窓で実際に ON に倒す**(O2 の判断)。授権に入らないので force-ON は起きないが、Engine だけ ON の誤りは既定の区分として残る |
| E13 | `reschedule_ime_refresh` の例外を「1 つの目的だけ」と判断して撤去し、別の経路(drift の周期実行)を巻き添えにした | P6 は前提つきにし、早期 return が担う役割を 5 つ列挙して、置き換えてからでないと採らない(§4.12) |
| E20 | BUG-114(drift correction の `FeedbackPolicy::Read` の無限再送) | drift の根拠の寿命を今のままにし、belief のラッチを drift に入れない(§4.9) |
| E26 | E4: eisu reset は Edge/TsfNative の循環デッドロック対策を兼ねる(検証不能。統合しない) | §4.6 の「モードが無い・古いならかな」が役割を引き継ぐ。統合ではなく同じ規則を新の経路にも置く。TsfNative での効果は検証不能のまま |

## 8. 移行

| 段階 | 内容 | 終わりの判定 | 戻し方 |
|---|---|---|---|
| V0 | §5.4 | §5.4-3 | 新モジュールを消す |
| V1(shadow) | 本番で旧と新を並べて計算する。新は Engine にも授権にも効かせない。食い違いを journal に `[belief-v2]` として残す。殻は新の `evolve` に同じ事実を渡す(スコープの鍵、押下時点の鍵を足す) | 所有者の dragonflyg4 と CI の `sc-*` で食い違いを分類(旧が誤り/新が誤り/真が分からない)。「新が誤り」が既定の区分以外で 0、BUG-223 型の区間で新が正しい側。件数の基準は所有者が決める | shadow をやめる |
| V2(切替) | Engine の ctx・lang-detect の入力を新へ(drift・授権は `write_basis` で今と同じ値)。隠し設定(例: `belief_engine = "v2"`) | 所有者の実機で一定期間、BUG-223 型の報告が無く、既定の区分の誤 ON(端末など)が許容範囲 | 設定を旧へ戻す |
| V3(撤去) | §4.8 で「消える」の行を撤去。`ImeEvent` の列挙値は残す | CI 全部・golden・台帳の回帰列・journal の再生が通る | revert(`.claude/rules/experiment-logging.md` に従い失敗の条件を本文に書く) |

- 実機の確認は所有者の GJI 環境(dragonflyg4)。再発ファミリーに触れるので、各段階の PR は `.claude/rules/fix-requires-evidence.md` の (a) を満たす。

### 8.1 実装の見積もり(M8 で作り直した)

**参照の数**(`ebd744b6d`、`git grep -F -c <語> ebd744b6d -- '*.rs'` の合計): `desired_open` 402 か所・39 ファイル、`effective_open()` 161 か所・27 ファイル、`intent_store` 86 か所・14 ファイル、
`input_mode()` 83 か所・17 ファイル、`explicit_intent()` 76 か所・14 ファイル(レビュアーの数と `desired_open` が 1 か所・1 ファイル違う。数え方の範囲の違いと思われるが未確認)。
ファイルの大きさ: `platform_state.rs` 5,634 行、`key_pipeline.rs` 3,632 行、`ime_model.rs` 4,417 行、`focus_tracking.rs` 975 行(`wc -l`)。

| 場所 | 変更 | 規模の見当(根拠のない目安) |
|---|---|---|
| `crates/awase-ime-core/src/state/`(新モジュール、`scope_belief.rs`・`trust_table.rs`・`write_basis.rs` 等) | 核(V0) | 新規 800〜1,500 行 + 単体テスト |
| `crates/awase-windows/tests/support/{harness,pseudo_ime}.rs`・`belief_engine_invariant.rs`・golden | 新の核・局所修正のアダプタ、§5.2 の拡張(MS-IME のトグル中の閉・修飾・授権の入力・長い時間) | 千行規模 |
| `crates/awase-windows/src/state/platform_state.rs`(`ImeStateHub`、5,634 行) | V1 で全事象を新へも流す(殻が一時的に太る: 事象の分配・鍵の取得・食い違いの journal。数百行の見当)、V2 で読み口を切替、V3 で撤去 | 大 |
| `effective_open()`・`input_mode()` を読むが旧見積もりに無かったもの: `half_width_alnum.rs`・`conv_classify.rs`・`explicit_press.rs`・`force_guard.rs`・`focus_probe_plan.rs`・`output/conv_actuation.rs`・`output/mod.rs`・`runtime/ime_refresh.rs`・`indicator_glyph_shadow.rs`・`lang_detect_shadow.rs`・`message_handlers.rs`・`mozc_shm_shadow.rs` | 読み口を新の `BeliefView` へ(V2)。各ファイルの読み方の意味(belief か授権か)を 1 件ずつ分類する | 中〜大(27 ファイル・161 か所の分類が要る) |
| `runtime/focus_tracking.rs`・`runtime/key_pipeline.rs`・`runtime/mod.rs` | スコープの鍵・押下時点の鍵の取得、後処理の縮小、予測の入力の作り直し、(P6 を採るなら)早期 return の役割の置き換え | 中〜大 |
| `crates/awase-windows-plans/src/focus/tsf_cache_restore.rs`・`focus/hwnd_cache.rs` | V3 で撤去 | 撤去 |
| `ime_model.rs`・`ime_event.rs`・`intent_store.rs`・`open_warrant.rs`・`drift_correction.rs`・`drift_plan.rs`・`state/ime_decision_view.rs` | 新事象、`write_basis` への入力の差し替え、Step 4c の入力 | 中〜大 |
| `runtime/drift_follow_would.rs`(ADR-279 P0) | 起点(早期 return)を P6 で動かすなら移す | 小〜中 |
| journal の再生(ADR-082、`tests/journal_replay.rs`・`docs/journal-replay-guide.md`) | 古い `ImeEvent` 列(`HwndCacheRestored`・`KeyEffectPredicted`・`UserImeSetIntent` 等)を新の核でどう解釈するかの写像。古い不具合報告のスナップショットの読み方 | 中 |
| `crates/awase-windows/tests/architecture_guard.rs` | `desired_open`・belief の書き込み点・`KeyEffectPredicted` が `desired_open` を書かない・`user_ime_on_paths_are_paired_with_eisu_reset` 等の固定テスト群を新の構造に合わせる | 中 |
| dylint `lints/ime_event_guard`・`lints/observation_source_guard` | 対象 variant・構築点の変化(`HeuristicDefault` の意味の変化を含む) | 小〜中(nightly ピンの保守は別) |
| `crates/awase-key-effect/src/state/key_effect_predictor.rs` | 修飾の述語を写像表へ、予測の入力 | 中 |
| 不具合報告(ADR-095)のスナップショット | belief の内訳の項目を新の `ClaimBasis` に | 小〜中 |
| `src/engine/` | 変えない | 0 |
| CI 検証バッチ(§5.5) | プローブの作り直し・構成の追加・判定スクリプト・観測専用のログ | 中 |

## 9. 却下した案・比較する局所修正

**局所修正(V0 で同じオラクルで比べる。別担当 belief-oracle が使い捨てブランチで実装して数える。定義の要点)**:

| 案 | 変えること | 効く見込み | 副作用 |
|---|---|---|---|
| L1 | `KeyEffectPredicted{open: Some(v)}` の reduce で `desired_open = v` を書く(`last_intent` は今どおり `None`) | **測定(§13.1)**: msime shared の I1 405 → 153、核の最小列 36 → 0。per-thread は I1 185 → 185、I2 57 → 109。`desired-fallback` の主因に直接効く。`explicit_intent()` は `last_intent` だけを見る(`platform_state.rs:319-321`)ので、ポーリングは止まらない | 窓切替で v を持ち越す(per-thread の真の新しいスレッドでは I2 が増えうる)。授権 Step 4c が予測の方向へ変わる。`architecture_guard` の固定を外す要 |
| L2 | `reduce_focus_changed` の最後で、profile が TsfNative/Imm32Unavailable(InputRelay を除く)なら `desired_open = true` | **測定**: msime shared の I1 405 → 256、I2 35 → 99。per-thread の I2 57 → 165。O2 の最小形。shared の `desired-fallback` の大半に効く見込み | 読めない窓で真が閉のとき I2 が増える |
| L3 | `IntentStore` の鍵を窓のスレッドにし、`apply_key_effect_prediction` の `intent_store.remove` を予測値の記録(TTL 10/30 秒のまま)に置き換える | **測定**: msime shared の I1 405 → 165、長い静止の字母で核の最小列 36 → 80(TTL 切れで悪化)。戻った窓(同じスレッド)で予測が効く。shared では全窓で効く | TTL 切れで今と同じく後退する。授権 Step 1 に予測が入る(ADR-087 に反するので本番に入れるなら別の行の種類が要る) |

**却下した案**(§7 に加えて):

| 案 | 捨てた理由 |
|---|---|
| A. 局所修正だけで済ませる | **まだ捨てていない**(M5)。測定(§13.1)の結論は §13.2: 真が shared なら L1 で BUG-223 の核は消える。per-thread なら I1・I2 を同時に減らす局所修正は無い。旧版の却下理由「予測で `desired_open` を書くと BUG-151 原因③を広げる」は誤り(ポーリングの停止は `last_intent` だけを見る。ADR-191 QB1 の 2 つの理由を混同していた)。「窓切替で捨てた後の値が決まらない」も誤り(値は O2 で決まっている) |
| B. hwnd を鍵にする | 読める Win32 EDIT の窓の CI 実測では、同じスレッドの別 hwnd は状態を共有する(191 spec §3.1)。読めない窓では未測定なので、B1 の結果で見直す |
| C. グローバル 1 つの belief のまま、窓切替で既定へリセットする | O4 に合わない。戻った窓で利用者の明示 OFF を失う |
| D. belief の主張に TTL を付ける | O3 に合わない。書き込みの根拠には今の寿命を残す(D8)ので、M-A の固着は書き込み側では起きない |
| E. 不確実なときは常に OFF・または lang-detect の判定を待つ | O2 に反する / 判定まで最大 7 打鍵・2.5 秒保持する重さと誤判定(ADR-280) |
| F. 観測を窓ごとに保存して戻ったときに観測として使う | C-c7・ADR-106 に反する。戻ったときに使うのは主張 |
| G. 別スコープの明示操作を弱い証拠として伝播する(粒度を知らずに shared・per-thread の両方に合わせる) | §4.1 のとおり、どちらに倒しても片方のモデルで外れる。粒度は測る |
| H. `Written` と `Predicted` で覆す条件を変える | O3(同じ重さ)に合わない。違いは書き込みの根拠に入るか(`origin`)だけ |

## 10. リスク

1. **スコープの粒度・鍵の誤り**(P1・U4): 読めない窓の保持単位が未測定。粒度を誤ると §4.1 のとおり I1 と I3 が両立しない。V0 の前に B1。
2. **既定 ON の誤り**(§4.6): 端末アプリで英字のコマンドが壊れる。BUG-200 の機会が増える。I2 の増加を旧と並べて所有者に示す(U10)。
3. **IME の状態が全体共有の構成**: O4 と衝突する(U11)。
4. **押下時点の鍵の取得**(BUG-218 の代替): ずれると明示操作が別のスコープに付く。`hwndFocus` が UWP・TSF ネイティブで真のフォーカスを返すかは未確認(BUG-91)。
5. **P6 の副作用**: 早期 return の 5 つの役割(§4.12)。採るなら置き換えと B8。
6. **明示操作の重さが lang-detect の誤判定にも乗る**(belief 側のみ。書き込みには乗らない。U8)。
7. **予測の外れが長く残る**: 窓切替で偶然直っていた外れが残る。BUG-211・212 は未修正。
8. **移行の規模**(§8.1): 27 ファイル・161 か所の `effective_open()` の読み方の分類、journal の再生、ガード群。V1 で殻(`platform_state.rs`、ADR-275 で core へ移さないと決めた)が一時的に太る。局所修正で足りるなら、この規模は払わない(§5.4)。
9. **寿命の無い主張は、誤った記録の最悪の持続時間になる**(belief 側のみ): 読めない窓では、誤った帰属・予測の外れ・外部変更が、次の明示操作か行の寿命(1 時間)まで Engine の活性に効く。今は `IntentStore` の 30 秒後に `resolve_open_at` へ落ちる(落ちた先が正しい保証は無い、BUG-223)。
   書き込みの根拠には寿命を残す(D8)ので、ADR-087 round4 M-A の「drift が永久に再同期できない固着」は起きない。V0 で読めない窓の外部変更の列の「誤りが続いた時間」を旧新で比べる。
10. **MS-IME 本体のトグル中の閉の読み**(M3): 信頼表の行が漏れると、トグルを出た後に Engine が OFF のまま残る。
11. **ハーネスの仮定への過適合**: 擬似 IME が表せないクセで外れる。V1 の実機データで確かめる。

## 11. 未決(所有者の確認が要ること)

| # | 問い | 本 ADR の提案(決定ではない) |
|---|---|---|
| U1 | 方針の例外に `disable_apps`(BUG-78)・`FocusKind::NonText`(experiments エントリ 02)・日本語でない IME を含めるか | 3 つとも Engine OFF の例外(今の挙動と同じ。`disable_apps`・`NonText` は `compute_state` の外にあるので I1 の対象外) |
| U2 | 不確実なときの既定で、実測の事前(「awase の起動後に作られたスレッドは閉で始まる」)を O2 の ON 寄りより先に使ってよいか(使うと、新しいスレッドの読めない窓で O2 を部分的に反転させる) | 所有者の判断を待つ。判断材料: B2 の結果、ハーネスで「使う/使わない」の I1・I2 の差 |
| U3 | InputRelay(RDP・VM・PowerToys MWB 等)を O2 の「読めない窓」として ON 寄りにするか。**今の事実**: awase は actuation を所有しない(`decide_gate` が `NotOwned`、`ime_actuation_decision.rs:152`)、物理モードキーを Suppress しない、この窓由来の open 観測を belief に取り込まない(`class_names.rs:192-193`)、lang-detect の対象外(`lang_detect_shadow.rs:315`)、予測の `unreadable` からも除く(`key_pipeline.rs` の `kp_predict_key_effect`)。窓切替の後処理での扱いは本 ADR では読んでいない | ON 寄りの対象から外す(中継先の IME は awase から見えず、Engine を ON にする根拠が無い)。既定は「主張なし = 今と同じ扱い」 |
| U4 | O4 の「窓ごと」を「IME が状態を持つ単位ごと」と読んでよいか。測定(run 40327981441)では、読める窓・実 Chrome はスレッドごと、MS-IME × WT は入場時に他の窓の開を引き継ぐ(一方向)、GJI × WT は未確定 | 窓の種類ごとのデータ(§4.1 の表)にする。MS-IME × WT のように実 IME が開を引き継ぐ窓では、O4(持ち越さない)を開閉軸には適用しない(U11) |
| U5 | JIS かな入力方式(`ObservedKana`)のときの Engine | OFF のまま |
| U6 | `compute_state` の外の入口(X7)を本 ADR でそろえるか | そろえない |
| U7 | 信頼表の未確定のセル(読めない窓 × MS-IME 本体の IMM、直接観測の開閉軸) | V0 の全列挙と実機 CI で決める |
| U8 | lang-detect などの awase 起点の明示操作を、belief 側で利用者の操作と同じ重さにするか(書き込み側には入れない) | 同じ重さ |
| U9 | 修飾付きのモードキーの写像表に、IME の設定の読み取りで修飾付きの割り当てを足す範囲 | 今は扱わない。設定に修飾付きの割り当てがあれば、そのキーは抑止 |
| U10 | O2 の適用範囲: 端末アプリ(Windows Terminal・WezTerm、英字のコマンド入力が主)を ON 寄りの対象から外すか | 判断材料: 窓の種類・アプリ別の I2 の増加件数(V0)、§4.6 の端末での影響。提案はしない(所有者の判断 O2 の範囲の問題のため) |
| U11 | 実 IME が窓をまたいで開を引き継ぐ構成(測定で MS-IME × WT が該当。全体で対称に共有する構成は見つかっていない)に O4(持ち越さない)を適用するか | 適用しない(その窓の開閉軸は `InheritOnEntry`)。O4 の「持ち越さない」は実 IME が窓ごとに独立な組(読める窓・実 Chrome)に適用する |
| U13 | 共有メモリの block が無い Mozc 系(公式 Mozc など)で、読めない窓の ADR-188 の直接読み・ADR-205 の監視窓を戻すか(今は「Mozc 系」として外れていて、外部切替に追随する証拠が無い。§4.14-2) | B9 の結果を見て決める。IMM の偽の閉(BUG-219)の出所が未解明なので、戻すなら開閉の遷移だけ(ADR-205)から |
| U12 | V0 で局所修正(L1〜L3)が合格条件を満たした場合、ゼロベースではなく局所修正を採るか | 局所修正を採り、§4.13(写像表)・D8(書き込みの根拠の分離)など必要な部分だけを別に進める |

## 12. 所有者の判断(2026-10-26、決定)

1. 「かな」の範囲 = ひらがな・カタカナ(かな入力の全般)。半角英数・直接入力は含まない。→ D7・§4.7(JIS かな入力方式は U5)。
2. 不確実なときは窓の種類で分ける。観測が読めない窓(GJI の tsf-native など)は ON 寄り、読める窓は観測を信じる。→ D5・§4.6(中身は変えない。実測の事前は U2、InputRelay は U3、端末は U10)。
3. 最後の明示操作が勝つ(ON も OFF も対称)。窓切替や観測の寿命切れでその効果が消えて古い値に戻らない。観測が反証する場合の扱いは設計する。→ D2・D3・D4・§4.4(書き込みの根拠の寿命は分ける、D8)。
4. 窓切替では IME の状態(開閉)を窓ごとに独立させ、持ち越さない。→ P1・§4.1・§4.5(「窓ごと」の単位は U4・B1、共有の構成は U11)。

## 13. 基準の測定・局所修正の結果(V0 の比較表)

基準の測定(PR #857、マージ済み)の数値は §1.2・§5.1.1(`37fc37e75` の文書と golden、CI run 40275882156)。

### 13.1 局所修正 L1〜L3 の測定(PR #868、OPEN、`origin/docs/belief-local-fixes` HEAD `5efea3527` の `docs/tasks/belief-engine-invariant-local-fixes-2026-10-26.md` を読んで引いた)

測ったのは belief-oracle(別担当)。変種の定義は §9 の L1〜L3(本 ADR の定義どおり)。差分はブランチ `exp/belief-local-fixes`(draft PR #862)、CI run 40296295547 で決定的に一致。
同じオラクル・同じ字母(I1 は全字母 19 種・長さ 3、I2 は核の字母 12 種・長さ 3)。括弧は読める窓の poll が 1 回届いた後も残る本数。

**注意**: ここの per-thread の base(msime 185)は**全字母**の値で、§5.1.1 の 9 本(**核の字母** 12 種)とは字母が違う。

I1(T=0):

| 変種 | atok shared | msime shared | msime-native shared | atok per-thread | msime per-thread |
|---|---:|---:|---:|---:|---:|
| base | 315 (290) | 405 (375) | 405 (375) | 197 (172) | 185 (155) |
| L1 | 183 (164) | **153 (135)** | 153 (135) | 191 (172) | 185 (167) |
| L2 | 229 (208) | 256 (232) | 256 (232) | 201 (180) | 216 (192) |
| L3 | 189 (173) | 165 (148) | 165 (153) | 195 (179) | 181 (164) |
| L1+L2 | 191 (172) | 184 (164) | 184 (164) | 199 (180) | 216 (196) |
| L2+L3 | 197 (183) | 194 (177) | 194 (184) | 203 (189) | 220 (203) |

核の最小列(「awase の操作・窓切替・正しい観測だけ」): msime shared で base 36 → **L1 0**・L2 14・L3 16・L1+L2 6・L2+L3 22。atok shared 18 → L1 0。msime per-thread 20 → L1 **32**・L2 18・L3 20。
長い静止を足した字母(`advance:11000ms`・`31000ms`)では msime shared の核の最小列が base 36 → **L3 80**(TTL 切れ後に悪化)、L1 は 0 のまま。

I2(核の字母):

| 変種 | atok shared | msime shared | atok per-thread | msime per-thread |
|---|---:|---:|---:|---:|
| base | 43 | 35 | 43 | 57 |
| L1 | 43 | 31 | 43 | **109** |
| L2(= L2b = L1+L2) | 155 | **99** | 155 | **165** |
| L3 | 41 | 29 | 43 | 57 |
| L2+L3 | 123 | 67 | 155 | 165 |

- BUG-223 の 4 本 + `key:hiragana > focus:wezterm`: base の msime/msime-native shared で 5/5 違反、全変種で 0/5。
- T=2000ms(事象なし)では全変種で本数が変わらない。`WriteCommand` の件数は base 131〜136、全変種 131〜136(固定 seed のランダム列。差は 1〜5 件)。
- L1 の後に msime shared で残る 153 本は、全部「嘘の観測を含む」(32)か「外部変更を含む」(121)。
- L2 の I2 の増加は読めない窓(chrome・wezterm 各 46、xaml 7、msime shared)。per-thread では 57 → 165。
- 壊れる既存テスト: L1 は `ime_model` の 2 本(予測は `desired_open` を書かない)と `architecture_guard` の `key_effect_predicted_event_is_constructed_only_in_apply_key_effect_prediction`。L2 は `warmup_gate_focus_scope` の 2 本と `check_drift_correction_ignores_heuristic_default_alone_without_explicit_intent`。L3 は `architecture_guard` の `intent_store_record_call_sites_are_limited_to_explicit_user_actions`・`random_key_walks_keep_predictions_consistent_with_truth`・`prediction_exhaustive_golden`。
- 測定文書の限界: TSF 窓・hwnd キャッシュの復元を写していない(L2 は本物では `HwndCacheRestored` と重なる)。L3 の授権の参照は窓の鍵のまま。

### 13.2 結論: 局所修正で足りるか、ゼロベースが要るか(現時点。実機の粒度の測定待ち)

- **真が shared(全窓で共有)なら、BUG-223 の核は L1(+7 行)で消える**: msime shared で I1 405 → 153、核の最小列 36 → 0、I2 は 35 → 31 で悪化しない。新設計の `ScopeModel=Shared` と同じ効果を、今の構造の 1 か所の変更で得られる。
  この場合、ゼロベースを BUG-223 のために払う必要は無い(U12 の提案どおり L1 を採る)。ただし L1 は「予測は `desired_open` を書かない」という ADR-191 決定 3 の実装の固定(`architecture_guard`・単体テスト 2 本)を変える決定になる。
- **残る 153 本は、局所修正でもゼロベースの主張のラッチでも減らない種類**(嘘の観測 32・外部変更 121)。減らすのは観測の手段(信頼表、MozcShm・ADR-205 の監視窓・字形・ADR-279 型の打鍵列の反証)と、観測が無い・嘘のときの既定(D5)の設計で、これはゼロベースか局所修正かと独立に必要。
- **真が per-thread なら、I1 と I2 を同時に減らす局所修正は無い**: L1 は I1 を変えず(185 → 185、核の最小列 20 → 32)I2 を約 2 倍(57 → 109)、L2 は I2 を約 3 倍(57 → 165)、L3 は I1 をわずかに減らす(185 → 181)が TTL 切れで悪化する。
  per-thread の場合に要るのは、持ち越さない(O4)・窓ごとに直前の記録を持つ(新設計の `ScopeModel=PerThread` のスコープ表)・I2 を増やさない既定、で、これは局所修正の範囲を超える。**ゼロベース(少なくともスコープ表)が要るのはこの場合**。新設計がこの場合に I1・I2 を同時に減らせるかは V0 で示す(まだ示していない)。
- **L2(読めない窓の窓切替で ON)の I2 の増加は、O2(読めない窓は ON 寄り)を今の構造に入れたときのコストの実測値**: msime shared で 35 → 99、per-thread で 57 → 165。新設計の D5 も同じ向きのコストを持つ(§4.6)。U10(端末を外すか)の判断材料にする。
- **窓の種類別の結論(保持単位の測定 run 40327981441 を受けて)**:
  - **読める Win32 窓・実 Chrome(スレッドごと = per-thread)**: 局所修正は I1 を減らさず I2 を増やす(L1 で I2 57 → 109、per-thread・全字母)。ここで要るのは「持ち越さない・窓ごとの直前の記録」で、新設計のスコープ表(`PerThread`)の範囲。**L1 をこの窓に効かせてはいけない**(新しいスレッドの窓は閉で始まるので、持ち越した ON が I2 になる)。
  - **MS-IME × Windows Terminal(入場時に開を引き継ぐ)**: shared に近い振る舞いで、L1 の「予測の ON を持ち越す」が実 IME の振る舞いに合う。新設計では `InheritOnEntry` の窓だけ他のスコープの最新の開を採る。
  - **GJI × tsf-native(BUG-223 の窓)**: 未確定(inconclusive)。BUG-223 は shared 型に合うが、確認できていない。
  - よって、**単一の局所修正(L1 を全窓に)は、窓の種類が混ざる実機では片方の窓で I2 を増やす**。窓の種類で持ち越すか否かを分けること自体が、今の構造(グローバルな `desired_open` 1 つ)では表せず、スコープ表(少なくとも「窓の種類ごとに持ち越すかの規則」)が要る。
    最小の形は「L1 + 持ち越しを `InheritOnEntry` の窓への入場に限る(per-thread の窓へ入るときは `desired_open` を持ち越さず、窓の記録/既定を使う)」で、これは局所修正と新設計の中間(グローバルな値は残し、入場時の規則だけを窓の種類で分ける)。V0 でこの中間案(L4 と呼ぶ)も同じオラクルで測る。
  - V0 の評価は、基準の測定の字母を窓の種類で混ぜた窓モデル(Win32・Chrome は per-thread、MS-IME の WT は入場時に開を引き継ぐ)で行う(ハーネス拡張。§5.2)。
- 所有者の判断(2026-10-26、team-lead 経由): 「L1 は、shared か per-thread かを測ってから入れるか決める」。保持単位の測定(読めない窓を含む)は別担当が CI で実施中(結果は `docs/tasks/adr282-ime-state-scope-measurement-2026-10-26.md` の予定。本改訂の時点で、どのリモートブランチにもまだ無い)。
  P1(スコープの粒度)はその結果が出るまで確定しない。
  測定は一部出た(run 40327981441)が、GJI × WT は未確定なので、L1 の判断は窓の種類別の中間案 L4 の測定と GJI × WT の追加測定の後にする(提案)。

## 14. Opus レビュー round 1 への対応(2026-10-26)

| ID | 指摘の要旨 | 対応 |
|---|---|---|
| B1 | スレッドごとの前提と、基準の測定の shared が食い違い、合格条件が自己矛盾 | 反映: スレッドの出典の範囲を限定(§1.4)。D1 を決定から外し前提つきの P1 に、粒度を `ScopeModel` のデータに(§4.1)。shared/per-thread で I1・I3 が両立する条件と、粒度を誤ると両立しないことを明示。§5.1 を両モデル主・合格条件をモデル別に。B1 の CI 測定を V0 の前提条件に。405 は shared・per-thread は 9 本を §1.2 に出し、規模の根拠を再検討 |
| B2 | `Written` の範囲と TTL の撤去で drift・授権の対象が広がる | 反映: D8 で belief のラッチと書き込みの根拠を分離。drift・授権は `OpOrigin ∈ {UserKey, UserCommand}` だけ・寿命は今のまま。lang-detect のひらがなキーは入れない。授権 Step 0〜4c の表(§4.9)。drift (a) の記述を「方向を問わず明示意図があるとき」に訂正。Step 4a のため `HeuristicDefault` の観測の書き込みを残す |
| M1 | `derive_any` の基準の記述の誤り(Medium 1 つでも採る) | 反映: §4.4 を 1 ソースで成り立つ形に作り直し。U7 の「2 つそろうまで待つ」を撤回、BUG-219 は信頼表で防ぐ。Low の扱いを表に追加 |
| M2 | 明示操作の帰属先とスコープの鍵の食い違い | 反映: 鍵を求める関数を 1 つ(`GetGUIThreadInfo(foreground_tid).hwndFocus` のスレッド)にし、帰属と参照の両方で使う。WebView2・AttachThreadInput・子 hwnd・UWP の表。`Hwnd` の行を (pid, tid) へ正規化する規則 |
| M3 | MS-IME 本体のトグル中の閉の読み × D6 で Engine が OFF に残る | 反映: 信頼表に「トグル中の MS-IME の開閉の読みは反証不可」、§4.10 で開閉軸を触らないことを明記、§5.2 に列を追加 |
| M4 | I1 の目標と予測の外れ、I2 の既定の除外、I6 の行の寿命 | 反映: I1 から予測の外れを除き別区分で数える。I2 は既定の区分を含めた総数を旧と並べ、所有者が上限を決める。I6 の範囲を `HWND_CACHE_MAX_AGE_MS` 未満と明記 |
| M5 | §9 A の却下理由の誤り、局所修正との比較が無い | 反映: §9 A の理由を訂正し「まだ捨てていない」に。L1〜L3 を定義し V0 の比較対象に(定義は team-lead 経由で belief-oracle に送付)。U12 を追加 |
| M6 | O2 を実測の事前で部分的に反転、D1 を確定にした | 反映: 実測の事前を D5 から外して U2 に。D1 は前提つきの P1 に |
| M7 | 端末アプリでの誤った ON のコスト、U3 の事実 | 反映: §4.6 に端末のコスト、U10(端末を外す選択肢)。U3 に InputRelay の事実(actuation 非所有・観測を取り込まない・lang-detect 対象外)を記載 |
| M8 | 実装の見積もりが過小 | 反映: §8.1 を参照の数(自分で再計測、`desired_open` は 402/39 でレビュアーと 1 違い)・漏れていたファイル・移行作業(journal の再生・dylint・ガード群・`drift_follow_would`・`ime_decision_view.rs`・不具合報告)・殻の太り方で作り直し |
| M9 | D6 の撤去は experiments エントリ 13 の教訓に当たる | 反映: D6 を前提つきの P6 に。早期 return の役割を 5 つ列挙(GJI の I/O 推論を drift の根拠にしない役割を追加で確認)。§7 にエントリ 13・20・26、§4.6 に E4 の引き継ぎ |
| m1 | `compute_state` の行 | 反映: `engine.rs:272` |
| m2 | `applied` の置き場所 | 反映: §4.2(今どおりグローバル、窓切替で `Unknown`) |
| m3 | 読める窓の `Unknown` と PR #817 | 反映: §4.7。開閉 ON のときのモード `Unknown` は `AssumedRomaji` として写す。#817 の対象窓は未確認と明記 |
| m4 | 基準の測定の引用は一致 | 対応不要(確認のみ) |
| m5 | 主要な行・関数は実在 | 対応不要。`platform_state.rs:1429` は引用から外し `decide_use`(`:2146-2149`)に絞った |
| m6 | 行の寿命に値を流用する理由 | 反映: §4.1 に流用の理由と意味の違い、M-A の指摘との関係 |
| m7 | U3 は読めるので未確認のまま問わない | 反映: U3 に事実を記載(窓切替の後処理の扱いだけは未読と明記) |
| m8 | 台帳の BUG-113 の行 | 反映: 「P6 との関係は未確認」と記載 |
| m9 | 依存する PR の状態を status に | 反映: #857 マージ済み・#854 OPEN・局所修正の測定は結果待ち |
