# ADR-269 最終確認(round 3)

対象: `fix/bug-207-mozc-server-io-monitor` の HEAD `809930e1`。round 2 からの差分は `git diff 50fa63c2..809930e1`。

自分で確かめたこと(CI の成果物を取得して数え直した):

| run | 構成 | 結果 |
| --- | --- | --- |
| 39179709667(develop) | `ts-tsf-mozc-gji-{10,40}ms-{1,2}` | 4 本とも `verdict=INVALID`。`attached to GJI` 1・`raw TSF literal suspected` 1・`giving up` 2・`initial IME kind: GoogleJapaneseInput (Other)` |
| 39179706704(`107cbacd`) | 同じ 4 本 | 4 本とも `verdict=PASS`。attached 0・literal 0・giving up 0 |
| 39179706704 | `ts-tsf-mozc-{10,40}ms-{1,2}`(Mozc だけ) | 4 本とも PASS・literal 0 |

**ADR と BUG-207 の数字は正しい。**

結論: 必須の修正はない。直すとよい点は次の 4 つ。

1. a-5 のデバウンスの候補が残る小さな穴(1-1)。
2. 「条件の確認2」の判定が緩い(2-3)。
3. 既知の限界の「約 4 秒」が古いまま(3-1)。
4. CI が detach の経路を一度も通っていない(2-1・4-1)。

1 と 3 は数行で直る。4 は「未確認」と明記すればマージを止める理由にはならない。

---

## (1) a-3・a-5 の実装

### 1-1. 非対称デバウンス: 候補が残り、「付ける向きは 2 連続」が破れる(低〜中)
- `gji_monitor.rs:404-410`。`observed_target == NoEvidence` の分岐では `target_debounce` に触れない。そのため前の tick の候補が残る。
- 経過の例(current=NoEvidence、つまり Mozc がアクティブ):
  - tick1 で `Gji` を観測: `observe(Gji, NoEvidence)` が `candidate=Some(Gji)` にし、まだ確定しない。
  - tick2 で `NoEvidence` を観測: 即時の分岐に入る。`observed == current` なので何もしない。**candidate は Some(Gji) のまま**。
  - tick3 で `Gji` を観測: `candidate == Some(Gji)` なので、その場で確定して接続する。
- 2 連続でない 2 回の観測(間に Mozc の観測を挟む)で接続する。単発の誤読(`ImeKindDebounce` の doc が記録している型)が 2 回あると、Mozc がアクティブなのに GJI に付き、次のポーリング(約 2 秒)で外れるまで BUG-207 の症状が出うる。
- 直し方(2 行): 即時の分岐でも `target_debounce.observe(observed_target, current_target)` を呼んで結果を捨てる。`observed == current` なら candidate が消え、違えば `candidate=Some(NoEvidence)` になるだけで害はない。あるいは `Debounce` に `reset()` を足して呼ぶ。
- テスト: `Debounce` は純粋なので、gji_monitor.rs 内の `#[cfg(test)]` で「Gji, NoEvidence, Gji の順では確定しない」を固定できる。ただし `gji_monitor.rs` は `#[cfg(windows)]` の下にあるので Linux では走らない(CLAUDE.md の注意)。走らせたいなら、即時/2 連続の判断を `converter_target.rs` 側の純関数(`next_target(observed, current, candidate) -> (Option<ConverterTarget>, candidate)`)に移すと Linux で固定できる。

### 1-2. detach で 0 に戻す処理(問題なし、注記 1 つ)
- `gji_monitor_ok=false` を先に書き、その後に `gji_last_io_ms`・`gji_last_write_ms`・`gji_attach_ms`・`gji_write_bytes` を 0 にする順序。
- 主スレッドで判定の途中だった `LiteralDetector` は、`gji_write_bytes()=0` を読むと `saturating_sub` で 0 増加 → deadline で SuspectedLiteral になる。凍った場合と同じ結果で、既知の限界の 2 つ目に書かれているとおり。`gji_last_write_ms=0` は fencing を無効にする(`probe.rs:725`)が、write の確認は来ないので影響しない。
- 戻さない値: `gji_write_ops`・`gji_read_ops`・`gji_other_ops`・`gji_other_bytes`(診断専用)。`[shadow-toggle] ... gji_write_ops=117` のような診断の欄が、detach 後も GJI の最後の累積を出し続ける。判定には使われない(BUG-75)ので害は無いが、「Mozc 単独と同じ」を TSF_OBS の水準で言うなら、これらも 0 にするか、ADR の D1 の「値まで同じ」を「判定と idle に使う値は同じ」に弱める。
- 再接続(`NoEvidence → Gji`)は `gji_attach_ms` を新しく書き、`gji_last_io_ms` を接続時刻にする(`:448-455`)。BUG-176 の初回読みの扱いも従来どおり。判定は `gji_active` が真になってから作られるので、0 からの跳びを読む判定は無い(round 2 の c のとおり)。

### 1-3. GJI だけの環境
- 変わらない(round 2 の a-1)。即時の分岐は `NoEvidence` を観測したときだけ働く。GJI だけの環境で `(GJI 系, Other)` が観測されるのは、GJI 系の CLSID で `TipIdentity::Gji` でない TIP がアクティブなときに限られ、その TIP は存在しない。
- `query_active_kind` が IMM32 の HKL を返す単発の誤読では、`identify_tip(None)=Other` だが kind が MicrosoftIme なので `converter_target=Gji`。即時に外れることはない。
- GJI だけの退行の CI は、`e2e-ime-smoke`(961effea、GJI の baseline と atok-passthrough-cold の 2 構成)しか回っていない。per-VK の literal 判定の経路(`ts-tsf-gji-*`・実 Chrome の `tsx-chrome*`)は修正後に未実行。コード上は変わらないが、`only='ts-tsf-gji-*'` で一度回せば安く言い切れる(4-3)。

---

## (2) ダミープロセス方式は BUG-207 の機構を代表できているか

**代表できているもの(develop の失敗の機構そのもの):**
- `find_gji_pid` は名前の接頭辞だけで探す。`OpenProcess(PROCESS_QUERY_INFORMATION)` も同じセッションの同じ権限で成功する。そのため `gji_monitor_ok=true` → `gji_is_active_ime()=true`(kind は Mozc で GJI 系)→ per-VK の literal 判定が有効になる。
- ダミーの累積 I/O は増えない → 350B に届かない → SuspectedLiteral → Escape/BS → 諦め。develop の 4/4 INVALID(attached 1・疑い 1・諦め 2)は、実機の BUG-207 のログの形(`attached to GJI process`・`suspected`・`giving up` の連続)と一致する。

**代表できないもの(U6 に足すとよい):**

### 2-1. detach の経路を一度も通っていない(中、いちばん大事)
- 修正後の 4 本は `attached to GJI` 0・`detaching` 0(上の数え直し)。awase の起動時から `initial IME kind: GoogleJapaneseInput (Other)` なので、最初から `NoEvidence` で、一度も接続しない。
- round 2 で足した 3 つの処理は、どれも CI で一度も実行されていない。
  - detach のブロック
  - 0 に戻す処理(a-3)
  - 非対称デバウンス(a-5)
- 実機の利用者が「awase の起動後に GJI → Mozc へ切り替えた」なら、通るのはこの未検証の経路。
- ADR の順序 2 の本文「修正後(`detaching`)を awase.log で比べる」は、実際の結果(detaching 0)と食い違う(3-2)。
- 経路を通す案(ダミーのまま作れる): 起動時はアクティブな TIP を MS-IME(GJI 系でない → target=Gji → ダミーに接続)にしておく。awase の起動後に `Set-WinDefaultInputMethodOverride` や `ITfInputProcessorProfileMgr::ActivateProfile` で Mozc に切り替えて打鍵する。awase.log に `attached to GJI` → `converter target → NoEvidence` → `detaching` が並び、`gji_io=` が 0 起点になることを確かめる。順序 5(切替の窓)と同じ構成で兼ねられる。

### 2-2. 実 GJI のバックグラウンド I/O(低)
- ダミーは起動直後(PowerShell の起動の I/O)を除いて静止している。実 GJI の変換サーバーは、別アプリが GJI を使えば I/O が動く。
- develop では、実 GJI の I/O が偶然 350B を越えて confirm になったり、`observe_gji_after_focus` で Mozc の belief を ON に書いたりしていた(round 2 の a-4)。この汚染と、偶然の confirm による `consecutive` の reset はダミーでは出ない。修正後はどちらも接続しないので関係ない。develop の再現が「実機より素直」になっている、という向きの差。

### 2-3. 「条件の確認2」は TIP が Mozc であることを確かめきれていない(低〜中)
- `e2e-ime.yml` の「条件の確認2」は `initial IME kind: (\S+) \((\w+)\)` の **group 2 が `Other`** であることだけを見る。
- `Other` は ATOK や未知の TIP、IMM32 の HKL でも出る。group 1 が `GoogleJapaneseInput` であることも確かめないと、「Mozc のインストールが失敗して別の IME がアクティブ」を条件成立と誤読する。その場合は develop も修正後も判定が走らず、両方 PASS になる。
- 1 行足す: `$kind.Matches[0].Groups[1].Value -ne 'GoogleJapaneseInput'` でも失敗にする。

### 2-4. 起動時の 1 回しか確かめていない(低)
- 条件の確認は `initial IME kind` だけを見る。試験の途中で TIP が変わっても検出しない(例: ハーネスの VK_CONVERT。ただし Mozc は ON にしないので kind は変わらない)。
- `[tip-detect] IME kind →` と `converter target →` の行が試験中に出ていないことも確かめると堅い。

### 2-5. 実 GJI の TIP が登録されている環境との差(低)
- 実機は GJI の TIP が登録されていた可能性が高い(未確認)。その場合は `GJI_CLSID` が Some になり、`GJI_FAMILY_CLSIDS` に Google と Mozc の両方が入る。CI のダミーでは `GJI_CLSID=None` のまま。
- Mozc の identity は `Other` で同じなので、`converter_target` の結果は変わらない。ただし「Google 版がアクティブなのに `Other` と誤読される」側(round 1 の 14、OnceLock の取りこぼし)は CI では試せない。

---

## (3) ADR・BUG の記述

### 3-1. 既知の限界の 1 つ目が古い(低、要修正)
- ADR の既知の限界の先頭行は「デバウンス(2 秒ポーリング × 2 連続、最大約 4 秒)の間は旧ターゲットを見続ける。Google 版 GJI → Mozc では、その間だけ BUG-207 の症状が出うる」のまま。
- D1 の 3 つ目の点(外す向きは即時、約 2 秒)と矛盾する。
- 直し方: GJI → Mozc は「次のポーリングまで最大約 2 秒」、Mozc → GJI は「2 連続で最大約 4 秒、その間は GJI の literal 判定が無効(未検出の向きで安全側)」と書き分ける。
- BUG-207 の「未確認」の最後の文(付ける向きは最大約 4 秒)は Mozc → GJI の向きで、BUG-207 の害の向きではない。害の向き(GJI → Mozc、約 2 秒)を書く。

### 3-2. 順序 2 の本文と結果の食い違い(低)
- ADR の順序 2 は「develop(`attached to GJI process`…)と修正後(`detaching`)を awase.log で比べる」。実際の修正後は `detaching` 0(起動時から NoEvidence)。
- 「修正後は接続そのものが 0(起動時から Mozc 系なので detach の経路は通らない。detach は順序 5 で確かめる)」に直す。結果の節の「未確認」にも「detach の経路(起動後の TIP 切替)」を足す。

### 3-3. fix_commits(低)
- BUG-207 は `["50fa63c2"]` だけ。0 に戻す処理と非対称デバウンスの `961effea` も修正の一部なので足す(CI の構成の `7b545a6b`・`107cbacd` は本文の参照で足りる)。

### 3-4. 「値まで develop と同じ」(低)
- 1-2 のとおり、診断専用の `gji_*_ops`・`gji_other_bytes` は戻していない。D1 の「Mozc 単独環境: TSF_OBS の値まで develop と同じ」は Mozc 単独については正しい(一度も接続しない)。
- 「GJI と Mozc の両方がある環境: … も Mozc 単独の develop と同じになる」は、読み手を列挙した範囲では正しい。値の全部ではない。「判定と idle に使う値は」と限定するか、残りも戻す。

### 3-5. 結果の表の見出し(低)
- 「修正後(`107cbacd`)」は CI の構成の変更のコミット。awase のコードは `961effea` 以降変わっていないので誤りではない。「コードは 961effea、構成は 107cbacd」と書くと、後で読む人が迷わない。

### 3-6. 正しく直っていることを確かめた点
- B の「未確認」、C の約 50ms、A の 0.4KB の出どころの分離、「探す仕組みも無かった」、順序 1 が退行の確認にすぎないこと。
- 既知の限界の 2 つ目(凍る→消す、StaleConfirm は D1 では起きない)。
- 症状の食い違いを未確認にしたこと。
- experiments.md の 1 行。
- BUG-207 の「回帰テスト」節は、Mozc 単独の構成が再発防止にならないことまで明記していて妥当。

---

## (4) まだ足りない実測

1. **detach の経路の CI**(2-1)。起動時は MS-IME → awase の起動後に Mozc へ切り替え、静止したダミーを置いたまま打鍵する。
   - 確かめること: `attached` → `detaching` の順、`gji_io=` が 0 起点になること、切替から detach までの時間(2 秒以下か)、その窓での `raw-tsf-literal` の有無。順序 5 と同じ構成で兼ねる。
   - これが無いと、round 2 で足したコードは「コンパイルが通る」以上の確認が無い。
2. **「条件の確認2」に group 1 の検査を足して再実行**(2-3)。今の結果を疑う理由は無い(4 本とも `GoogleJapaneseInput (Other)` を自分で確かめた)ので、次の run からでよい。
3. **GJI だけの per-VK 経路の退行確認**(1-3)。`only='ts-tsf-gji-*'`(必要なら `tsx-chrome*` も)を修正後の HEAD で 1 回。
4. 既存の順序 3(段階 2 の観測)・順序 4(Mozc の本物のリテラル)・順序 6(実機)はそのまま。順序 6 の確認項目「`detaching` が 1 回出る」は、利用者が awase を Mozc で起動するなら出ない。「起動時から Mozc なら `attached` が 0、起動後に切り替えたなら `detaching` が 1」と場合を分ける。
