# ADR-255 敵対的レビュー round2(Opus)

対象: `docs/adr/255-ime-off-thumb-key-space.md`(ワークツリー keymap-ime、HEAD `ebeb24b4`)。行番号はこの HEAD のもの。

## 総評

A1(エンジン内)への転換で、r1 の B1・M3・M4 は**構造的に解消している**。決定2-1 と 2-5 の AND は安全側になっている(下の「r1 対応の確認」)。

一方、コードを読むと新しい問題が三つ見つかった。

1. **TsfNative では `derive_actuating` が定常状態で `None` になる**。Chrome/VS Code/WezTerm では、外部変化を観測した直後の 3 秒だけ効く「時々効く」機能になる(Blocker)。
2. **条件4の「役割が無い」を `thumb_open_role_action` で判定すると、IME 設定由来の役割を見落とす**。役割があっても単独タップ設定が Suppress のときや、専用 Fn キーを設定しているときは `None` が返る。その場合 GJI が変換で IME を開けなくなる(Must)。
3. **InputRelay(RDP/VM)と Alt なりすましの古いキャッシュ**の 2 経路で、意図しない Space が出る(Must)。

---

## r1 対応の確認

| ID | 判定 | 根拠 |
| --- | --- | --- |
| B1 | 満たす | 決定2-1 は `compute_state(ctx)` を見る。`ctx.ime_on` は `effective_open()`(IntentStore の上書き込み、`platform_state.rs:971-980`)なので、上書きで「開」ならエンジンは活性になり、発動しない。2-5 は AND で条件を**足す**だけなので、エンジンが活性なのに発動する経路は無い。guard の上書き(ON 方向のみ)や `KeyEffectPrediction=open` も `ctx.ime_on=true` → 活性 → 不発動。未解決の疑問2 は「安全側」で閉じてよい。**ただし R2-S1 の Phase 1/Phase 2 の順序に注意** |
| B2 | ほぼ満たす | `!ctx.composing`(`ctx.composing` は `tsf::observer::ime_composition_active_now()`、`key_pipeline.rs` の ctx 構築)。キャンセル処理を通らないので、最悪でも Space が 1 個入るだけ。r1 で問題にした「未確定文字列の破棄」は消えた |
| M1 | 部分的 | `derive_actuating` の選び方は正しい。しかし実際に書かれる Actuating ソースの分布を見ていない(R2-B1) |
| M3 | 満たす(既存の前例と同じ窓が残る) | Phase 1 で Consume すると `lifecycle.on_key_down_consumed` と `phase1_held` が立つ(`engine.rs:499-505`)。ただし押している間に活性化したとき、文字キーは `ctx.left_thumb_down`(フックのスナップショット、`input_tracker.rs:56-63` → `PhysicalKeyState`)で親指シフト扱いになる。検証計画1 の「ならない/なる」は期待値を決めずに固定はできない(R2-S4) |
| M4 | 満たす | `[[keymap]]` の latch を使わない |
| M5 | 満たす | `is_bare_thumb` は `!event.injected`(`engine.rs:998-1006`)。ただし Alt なりすましは「物理」なので残る(R2-M2) |
| M6 | 不十分 | R2-M1 |
| M7 | 満たす | 表と summary に反映済み。baseline の「無変換・変換とも open は 0 のまま」は今回、成果物で裏取りしていない(未確認。run 38058313464 の KEY 行で確認できる) |

---

## Blocker

### R2-B1. TsfNative では `derive_actuating` の根拠がほぼ無い。予測では「効かない」ではなく「時々効く」になる

- 本番で open/close の観測として記録される Actuating ソースは、実質 `ObserverPoll`(Medium)と `ImmCrossProbe`(High、ImmCross アプリのフォーカス直後だけ)の 2 つ。`Observed::<...>` の構築箇所は `platform_state.rs:1427, 1604, 1636, 1681, 1802, 1826, 1859` で、`Gji`/`Tsf`/`ImmGetOpenStatus` の evidence を本番で作る箇所は見当たらない(`grep -rn "Observed::<" crates/awase-windows/src`)。
- `ObserverPoll` は TSF ネイティブ窓では書かれない。`observer/ime_observer.rs:52-63` の `classify_poll_outcome` で、`is_tsf_native` のときは `observer_poll: None` になる("IME detection skipped (TSF-native window)")。TsfNative で `write_observer_poll` が呼ばれるのは、外部変化の検出(`follow_external_change_in_scope`、`platform_state.rs:456-480`)と ADR-188 の窓内の直接読み(`platform_state.rs:530-556`)だけ。
- `derive_filtered` は `OBSERVATION_FRESH_WINDOW_MS = 3_000`(`tuning.rs:69`)より古い観測を捨てる(`observation_store.rs:832-835`)。
- 帰結: Chrome・VS Code・WezTerm で IME OFF のまま打っていると、`derive_actuating` は `None` になり発動しない。IME を OFF にした直後(外部変化として観測された直後)の 3 秒だけ発動する。**同じアプリで、押すタイミングによって Space になったりならなかったりする**。報告者からは「不安定」に見え、「効かない」より切り分けが難しい。
- 検証計画0 は「測る」としているが、コードからこの結果はほぼ予測できる。ADR に次のどれかを**決定として**書くこと。
  - (a) `cannot_verify_real_ime_state` 系のプロファイル(TsfNative 等)では、明示的に発動しない(決定的に「非対応」にする。設定画面にも書く)。
  - (b) 鮮度を問わない別の根拠を定義する。例: 同じフォーカスで最後に確定した Actuating 観測が `false` で、その後に開閉を変えうる打鍵(IME キー・役割キー)が無いこと。この場合は新しい判定になるので、r1 M1 と同じ審査が要る。
  - (c) 「時々効く」を受け入れ、debug ログの理由で切り分ける(非推奨)。
- 検証計画0 の計測は、(a)/(b) を選ぶ材料としてプロファイル別に「押下時点で `derive_actuating` が `Some(false)` だった割合」を数える形に直す。

---

## Must

### R2-M1. 条件4の「役割が無い」の判定元が狭い。GJI が自分で IME を開く変換を Space で奪う

- `NicolaFsm::thumb_open_role_action`(`nicola_fsm.rs:862-876`)は次の 2 点で `None` を返す。(i) 専用 Fn キーを設定した無変換、(ii) IME 設定由来の役割(`role_open_action`)があっても、単独タップの `ModeKeyConfig` が Passthrough でないとき(`solo_tap_is_passthrough`)。
- (ii) のとき、エンジン非活性では役割の SetOpen を書かず、生キーを通して **IME 自身に開かせている**(`Phase 2` の `pass_through`)。条件4 をこの関数で判定すると「役割なし」になり、Space に変えて GJI に届かなくなる。CI の `ctl-loaded` 構成(DirectInput の無変換 = IMEOn)はまさにこのケースで、その行がある人は IME を開く手段を失う。
- (i) も同じ。ADR の決定4 は `muhenkan_solo_tap_dedicated_fn_key` と重なるときは発動しないとしているが、`thumb_open_role_action` は専用 Fn キー設定済みで `None` を返すので、関数の結果だけを見ると逆に発動する。
- 修正案: 条件4 を「`bare_ime_action(vk)`(keys.ime_*)・`thumb_role_open_actions()` の該当側(単独タップ設定を問わない)・専用 Fn キー・`sync_direction` が**すべて None**」にする。純粋関数で表を固定し、`thumb_open_role_action` を流用しないこと。
- 学習表も IME 設定も無い構成(MS-IME の「キーとタッチのカスタマイズ」で無変換/変換に IME オン/オフを割り当てたが、awase がそれを知らない等)では、IME 自身の割り当てを Space で上書きする。決定5 の注記に「IME 側で無変換/変換に割り当てた機能より優先される」と書くこと。

### R2-M2. Alt なりすましで、IME OFF 直後の最初の Alt が Space になる

- フックは `cached_engine_enabled` が真のとき Alt を親指 vk(無変換)に書き換える(`hook.rs:228-253`、`alt_impersonation.rs:82-100`)。このキャッシュは `EngineStateChanged{enabled: now_active}`(`engine.rs:411-440`、executor.rs:623)でしか更新されない。そして `EngineStateChanged` は Phase 2 の `check_active_transition` が出す。
- GJI 自身(半角/全角・言語バー・アプリの自動切替)で IME が閉じた後、エンジンが遷移を出すのは次の打鍵の Phase 2。その打鍵が Alt なら、フック時点のキャッシュはまだ真 → Alt が無変換に書き換わる → 物理・非注入・bare なので `is_bare_thumb` を通る → Phase 1(Phase 2 より前)で `compute_state` は `Inactive(ImeOff)` → **Alt が Space になる**(Alt+Tab の Alt も含む。Tab より先に Alt Down が来るので、そのときは修飾なし)。
- 書き換え後の vk では区別できないが、scan code は残る(Alt は 0x38、無変換は 0x7B)。条件3 に「なりすまし由来でない」を足す(`RawKeyEvent` に印を足すか、scan code で判定)。検証計画3 の実機確認だけでなく、Linux 単体と CI(`left_alt_impersonates_thumb_key` + GJI 側から IME OFF → 最初の Alt)に入れる。

### R2-M3. InputRelay(RDP・VM・PowerToys MWB)では発動させない

- InputRelay の窓では、ローカルの IME belief はリモート側の IME 状態を表さない。ローカルが OFF と観測されても、リモート側が ON で親指シフトしていることがある。ADR-206 も InputRelay では役割を付けない(`runtime/mod.rs::enrich_thumb_key_role` の `input_relay` 分岐、:815-818)。
- 条件に `profile != InputRelay` を足す。これは Platform 側の事実なので、`ime_off_confirmed` を立てる側(シェル)で偽にするのが筋。`disable_apps` の窓はフックが素通しするので対象外。

### R2-M4. 新しい `SpecialKeyMatch` の置き場所と、Platform 側の述語の整合

- 置き場所は `match_special_keys` の `or_else` 連鎖(`engine.rs:917-936`)の**末尾**(`thumb_open_role_action` の後)にすること。`engine_on` コンボ・`keys.ime_*`・自動検出トグルより後でないと、緊急復帰経路やユーザー設定を奪う。
- `match_special_keys` は Platform からも呼ばれる。`matches_ime_set_open`(`engine.rs:896-903`、shadow の書き込みを抑止する判断 `engine_owns_open_key`、`key_pipeline.rs:113-125`)と `matches_ime_off`(`engine.rs:880-885`、Ctrl+無変換 救済窓)で、新しい種別は**開閉ではない**ので `None`/`false` になる。これを明記し、網羅 match のテストで固定する。
- 自動リピートの扱い。既存の `check_special_keys` 先頭のガード(`engine.rs:1105-1107`)は `thumb_open_role_action` 専用で、新しい種別には効かない。初回の Down は `phase1_held`(:502-504)が立つのでリピートは :493 で Consume される。ただし `phase1_held` は flush 等で消えうる(コメント :1103)。その後のリピート Down が `!event.was_down` 条件で弾かれるかを、`match_special_keys` の連鎖(:932 の `!event.was_down` は役割側にだけある)で確かめ、新しい種別にも同じ `!was_down` を付ける。付けないと、リピートで Space が連射される経路が残る。
- ADR-019: エンジンは生の VK 定数を持たない。Space の VK は `set_space_thumb_config`(`engine.rs:112-124`)と同じく Platform から渡すこと。

### R2-M5. 再発ファミリーの指定

- `src/engine/engine.rs::thumb_open_role_action` は `fix-requires-evidence.md` の「キー選択(IME ON/OFF に送る VK)」ファミリーに、エンジン非活性側の入口として明記されている。物理キー押下ラッチ(Down/Up 非対称)ファミリーにも触れる。ADR の影響範囲に、両ファミリーと (a) 回帰テストの置き場所(`src/engine/tests.rs`、`cargo test --lib`)を書くこと。

---

## Should

### R2-S1. Phase 1 の早期 return で、活性→非活性の遷移の副作用が 1 打鍵遅れる

- `on_input_body` は Phase 1(特殊キー)で Consume すると、Phase 2 の `check_active_transition`(`engine.rs:511`)を通らずに return する(:498-507)。GJI 側で IME が閉じた直後の最初の打鍵が無変換だと、次の 3 つが次の打鍵まで遅れる: `EngineStateChanged{false}`(トレイ表示・Alt なりすましのキャッシュ、R2-M2 の窓が延びる)、`release_pending_and_reinject`(保留中の出力の解放)、FSM の flush。
- 既存の役割経路も同じ形だが、役割経路は IME を開閉する打鍵なので、直後の遷移で整合が取れる。Space 経路は IME を変えないので、遅れたままになる。新しい種別では、Space の前に `check_active_transition` の effects を前置する(`prepend_effects`)ことを決定3 に書く。

### R2-S2. ADR-245(左 Shift 半角英数の持続トグル)との順序

- ADR-245 のトグルは「IME-ON 半角英数」(`half_width_alnum.rs` 冒頭の doc)なので、トグル中はエンジンが `NotRomajiInput` になり発動しない。決定7 と整合する。
- 問題は「戻り待ち」(離脱時に IME へ送らず保留し、戻った窓の最初の打鍵で復元する)。戻った窓の最初の打鍵が無変換のとき、復元(IME を開いて英数にする)が ctx 構築の前か後かで結果が変わる。前なら発動しない。後なら Space を出してから IME が開く。ADR-245 PR 2(殻の配線)は未マージなので、両 ADR のどちらかに「戻り待ちが立っている間は `ime_off_confirmed=false`」と書いて順序の依存を消す。

### R2-S3. `ctx.composing` の有効範囲

- `ime_composition_active_now()` は TSF observer のフラグ。ADR-114 の実装レビュー(MA-1)は、`[[keymap]]` で `is_composition_warm_in_tsf()` との OR を必要とした。R2-B1 の結果、発動しうるのは主に Win32/IMM 窓になる。その窓(Notepad + MS-IME/GJI)で composition 中にこのフラグが立つかを、検証計画2 の負の対照「composition 中は入らない」で、Win32 の窓を使って確かめること(Chrome で確かめても意味が無い)。

### R2-S4. 押下中に活性化したときの期待値を先に決める

- 既存の役割経路(変換 = IME ON の単独押下)にも、押したまま次の文字を打つと親指シフト扱いになる同じ窓がある。そちらの現挙動を前例として調べ、Space 経路も同じにするか、違えるならその理由を書く。検証計画1 の「ならない/なる」は、どちらかに決めてから固定すること。KeyUp は `UpDuty::Consume` で回収され、活性化後は FSM に Down なしの親指 Up が届く(`engine.rs:513` は非活性時だけ `release_only`)。この挙動も単体テストで固定する。

### R2-S5. 検証計画の抜け

1. 計画0: R2-B1 のとおり、プロファイル別に「押下時点で `derive_actuating == Some(false)` の割合」を数える。報告者のアプリのプロファイル(TsfNative か)を先に確認する。
2. 計画2 の負の対照に足す: (i) CUSTOM 表に `DirectInput,Henkan,IMEOn` があり単独タップ設定が既定の構成で、変換が IME を開き Space にならない(R2-M1)。(ii) Alt なりすまし + GJI 側からの IME OFF → 最初の Alt が Alt のまま(R2-M2)。(iii) Ctrl+無変換(救済窓)と Shift+無変換が従来どおり。(iv) 長押しで Space が 1 個だけ(リピートしない)。
3. InputRelay(R2-M3)は CI で作りにくいので、Linux 単体で「InputRelay なら `ime_off_confirmed=false`」を固定する。
4. 計画1 の表に、`was_down`・なりすまし由来・InputRelay・戻り待ちの各軸を足す。
5. journal: 発動・不発動の理由を debug ログに出すとしているが、ADR-250(境界 journal)の流れに合わせ、エンジンの判断を記録として残すかを決める(報告 journal で「なぜ Space にならなかったか」を追えるように)。

### R2-S6. 設定名

- 既存に `set_space_thumb_config`/`space_thumb_vk`(Space を親指キーにする構成のフォールバック)がある。`ime_off_thumb_key = "space"` はそれと混同しやすい。値が `"pass"` なのも、単独タップの Suppress/Passthrough(`muhenkan_solo_tap_*`)の語と紛れる。候補: `thumb_key_when_ime_off = "unchanged" | "space"`、または真偽値の `ime_off_thumb_key_as_space = false`。
- 決定1 の「親指キーが無変換/変換のときだけ」と Alt なりすまし(親指は無変換、物理は Alt)の関係は、R2-M2 と合わせて書く。

### R2-S7. 事実関係

- 「エンジンは非活性のとき bare の親指キーを `thumb_open_role_action` で扱う。入口の条件に…`is_bare_thumb`(物理・無修飾・非注入)…が揃う」とある。正確には `is_bare_thumb` は Shift と OS 修飾(Ctrl/Alt/Win)を除き、`key_classification` が LeftThumb/RightThumb のものを通す(`engine.rs:998-1006`)。Alt なりすましの打鍵は「物理」として通る(R2-M2)。本文の「物理」に注記を足すこと。
- 決定4 の「panic 検出は計数が増減しない」は正しい(`app/mod.rs:626-637` で `deliver_key_event` より前に数える)。ただし、重なる構成では条件4 で発動しないので、「重なる構成での速い交互押下」は Space 機能と無関係に今と同じ。検証計画4 は削ってよい。

---

## 確認に使ったコマンド(HEAD `ebeb24b4`)

- `grep -rn "Observed::<" crates/awase-windows/src --include=*.rs | grep -v test`(本番で作られる観測の種類)
- `sed -n 28,90p crates/awase-windows/src/observer/ime_observer.rs`(TsfNative で `observer_poll: None`)
- `sed -n 827,900p crates/awase-windows-core/src/state/observation_store.rs`、`grep -n OBSERVATION_FRESH_WINDOW_MS crates/awase-windows-core/src/tuning.rs`
- `sed -n 455,546p src/engine/engine.rs`(on_input の Phase 1/2)、`sed -n 896-1006`(match_special_keys・thumb_open_role_action・is_bare_thumb)
- `sed -n 840,880p src/engine/nicola_fsm.rs`(役割の Passthrough フィルタと専用 Fn キー)
- `sed -n 58,150p crates/awase-windows/src/runtime/key_pipeline.rs`(ctx 構築と shadow 段の順序)
- `sed -n 400,440p src/engine/engine.rs` と `hook.rs:228-253`(Alt なりすましのキャッシュの更新元)
- 未確認: CI 成果物(baseline の open 遷移)。TsfNative で `follow_external_change_in_scope` が実際に何秒おきに観測を書くか(コードからは「変化したときだけ」と読めるが、実測していない)。
