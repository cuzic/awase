# ADR-133: `send_ime_mode_key` が送る `SendInput` バッチの形状が GJI の「@」誤出力を左右する（BUG-113 恒久修正）

## ステータス

**再調査中（2026-09-05）。バッチ形状（候補V/A/B3/B4）・VK値
（`VK_IME_OFF` vs `VK_KANJI`）の両仮説は実機A/Bで反証、これらの候補は
不採用。ユーザー実機観測から PowerShell PSReadLine との相互作用が
トリガーの一部であることは確認したが、awase 側のどの送信方式・呼び出し
経路が「@」の必要十分条件かはまだ未特定——「相手（GJI/PSReadLine）の
実装が脆弱」であることは awase 側の送信動作が無関係であることを
意味しない（他の known-bugs エントリと同じ扱い）。
`set_ime_open_cross_process`（`ImmSetOpenStatus`）は TsfNative（Windows
Terminal 含む）に対してそもそも効果を持たないため候補から除外した。

呼び出し連鎖の全数調査で見つかった新候補（`kp_stage_idle_conv_check` が
spawn する GJI への cross-process 読み取りクエリが、`GjiDirectStrategy::
apply` の同期 `SendInput` と競合しうる）と、既知の「2重actuation」
（`shadow_toggle_off_sync`/`engine_decision_sync`）を切り分ける第2弾
診断スパイクを実装済み・実機投入待ち（`docs/known-bugs.md` BUG-113
「未解決・次にやること」参照）。**

D0-3診断コード一式（バッチ形状・VK値検証専用）は役目を終えたため撤去済み。

<details>
<summary>旧ステータス（設計段階、参考として保持）</summary>

設計承認（Opus 敵対的レビュー round4 で承認、v5）。
D0（実機での機構検証3件）に進める水準。

</details>

v1 は「裸2イベントバッチが原因」という仮説の下、対象VK自身の自己エコー
パディング（候補A）と偽Ctrlブラケット（候補B、mode=3相当）を提示したが、
round1 の敵対的レビューで両候補の設計に実装上の欠陥（Blocker 1・2）と
根拠の弱さ（Blocker 3）が見つかり、かつ「イベント数」仮説そのものを
相対化する対照データ（Major 4）と、より機構的な代替仮説（Major 5、
`VK_IME_OFF=0x1A` と JIS スキャンコード `@` の一致）が指摘された。
v2 では「原因不明のままパターンマッチで直す」前に、安価で決定的な
機構検証を先に行う方針へ転換し、D2 のパディング候補の安全性を
作り直した。v3 ではさらに round1 の残り（Major 8〜9 相当）を反映:
イベントを**増やさない**方向の代替案（候補V、`SendInput` の分割）を
見落としていたため新設して最優先候補に格上げ、案Y（`ImmSetOpenStatus`）
の却下根拠が存在しないコメントの引用だった点を訂正、統計的信頼性・
不感状態による交絡・撤退手順・hidden config の昇格条件・設定命名を
D3/D4 に反映した。v4 では round2 で新たに見つかった Blocker 2件
——候補B「対称形」案が mode key を Ctrl 押下中に配送してしまう
（既存の不変条件と衝突する）欠陥、および JIS スキャンコード 0x16 は
`6` ではなく `U` だったという round1 自身の誤りの訂正——を反映し、
安全弁引き上げの算術・GJI warmup 二重起動の回帰確認も追加した。v5 では
round3 で見つかった Major 指摘2件——D4 の判定基準が旧来の「パディング」
前提のままで新第一候補の候補V（バッチ分割）に適用できず、特に`@@`
（2個混入）の場合に誤って候補Bへフォールバックしてしまう欠陥、候補Vが
既存の「複数キーは同一バッチで配送する」設計原則と衝突する点への
反論不足——を候補ごとの判定基準・原則との関係の明文化で解消した。

## 背景・調査の経緯（要約）

詳細は [docs/known-bugs.md](../known-bugs.md) BUG-113/BUG-114、
[docs/experiments.md](../experiments.md) エントリ20 を参照。要点のみ:

1. Windows Terminal + Google 日本語入力（GJI）で awase Engine 有効時、物理
   半角/全角キー（この機体では `VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR` として
   届く）を押すと「@」が1文字余分に出力される。**TurnOff 方向でのみ発生。**
2. 当初仮説「`send_ime_mode_key` の `wScan=0` が原因」は実機 A/B で
   反証された（修正を全アプリ既定 on で投入したが「@」の再現に変化無し）。
   **この反証は「`wScan` フィールドを読む消費者」バージョンだけを反証して
   おり、後述 Major 5 の「`wVk` の値を scan として誤読する消費者」バージョン
   は反証していない**——`wVk` はどちらの試行でも `0x1A`（`VK_IME_OFF`）の
   まま変わっていない。
3. 上記 A/B のログ取得中に、無関係な別バグ（BUG-114、[ADR-134](134-drift-correction-feedback-policy-focus-snapshot-staleness.md)）
   を発見したが、単独では「@」の必要条件ではないと実機 A/B で判明した。
4. ユーザーの指摘（「Ctrl+無変換 も同じ `send_ime_mode_key(VK_IME_OFF)` を
   送っているのでは」）を機に、Ctrl+無変換 と物理半角/全角キーが**まったく
   同じ**コードパスを通るのに前者では「@」が一度も出ないことを確認し、
   両者の唯一の違い（後述）に真因候補を絞り込んだ。

## 問題（現状の確定事実と、確定していないこと）

`crates/awase-windows/src/ime.rs::send_ime_mode_key()` は次の形で
`SendInput` バッチを組み立てる（`held_skip_alt` は Alt を除く物理修飾キー
状態）:

```rust
let held_skip_alt = HeldModifiers { alt: false, ..HeldModifiers::read() };
let mut inputs: Vec<INPUT> = Vec::with_capacity(6);
held_skip_alt.push_release(&mut inputs, IME_KANJI_MARKER); // Ctrl/Shift 押下中のみ追加
inputs.push(make_key_input_ex(vk, false, IME_KANJI_MARKER)); // vk down
inputs.push(make_key_input_ex(vk, true, IME_KANJI_MARKER));  // vk up
let still = held_skip_alt.push_restore(&mut inputs, IME_KANJI_MARKER); // Ctrl/Shift 押下中のみ追加
```

`push_release`（`output/held_modifiers.rs:59-61`）は `self`（読み取り時の
物理状態）を見て、Ctrl/Shift が実際に押下中でなければ何も追加しない。
一方 `push_restore`（同 68-94行）は**呼び出し時点で改めて物理キー状態を
再読み取り**し、`self` と AND を取ってから復元イベントを追加する
（synthetic Ctrl↑ 注入後にユーザーが Ctrl を離した場合に誤って復元しない
ための安全策）。この非対称性から、実際に発生しうるバッチ形状は次の
3通りである（v1 は2通りとして扱っていたが不正確だった、Major 6）:

- **Ctrl+無変換、かつ `read()`〜`push_restore()` の間 Ctrl を押し続けた場合**:
  `[Ctrl↑, vk↓, vk↑, Ctrl↓]` の4イベントバッチ。
- **Ctrl+無変換だが、その間に Ctrl を離した場合**（「押して即離す」操作
  なので現実的）: `[Ctrl↑, vk↓, vk↑]` の3イベントバッチ（`still.ctrl==false`
  で復元イベントが付かない）。
- **物理半角/全角キー単独**（修飾キー無し）: `[vk↓, vk↑]` の裸2イベント
  バッチ。

`config.toml` の `diag_bug113_mode_cycle_enabled` による実機診断
（`ime_controller.rs`、複数ラウンドで再現確認済み）:

| mode | 送り方 | イベント数 | 「@」 |
| --- | --- | --- | --- |
| 0（baseline） | 単体2イベント（現状） | 2 | **出る** |
| 1（imm-cross） | `ImmSetOpenStatus`（`SendInput` 不使用） | 0 | 出ない |
| 2（keystate-clear） | 別バッチで synthetic KeyUp 後、単体2イベント | 2（別バッチ含め4） | **出る** |
| 3（fake-ctrl-bracket） | 偽 Ctrl release/restore で1バッチにまとめる | 4 | 出ない |

**未確定点1（Major 6）**: 実機で「@」が出なかった Ctrl+無変換 の実行時、
実際に送信されたバッチが4イベントだったか3イベントだったかを、
`send_ime_mode_key` 自身が出す `[ime-mode] SendInput ... total={} events`
ログ（`ime.rs:299-309`）で確認していない。もし3イベントで「@」が出て
いないなら、必要なパディングは1イベントで足り、後述 Blocker 1 の
「stuck Ctrl」問題が構造的に発生しない、より安全な最小修正に到達できる。
**D4 の実機検証より前に、既存ログの再確認だけで分かることなので先に行う。**

**未確定点2（Major 4）**: 「イベント数」だけが弁別因子とは断定できない。
GJI eager warmup 用の `tsf/send.rs::send_eager_warmup_vk_pair()` は
`VK_IME_ON` を実 scan 付きの裸2イベントバッチで日常的に送信しており
（ADR-100 決定2で本採用済み、Windows Terminal + GJI でも通る）、
「@」や他の余分な文字は一度も報告されていない。イベント数2・実scanという
条件を揃えても `VK_IME_ON` では出ず `VK_IME_OFF` では出ることになり、
方向（あるいは VK 値そのもの）も同じ強さの共変量である。mode 0/3 の
対比だけから「イベント数が唯一の弁別因子」と結論するのは過剰主張
だった（v1 72-73行の記述を撤回する）。

**未確定点3（Major 5、新規。round2 で数値を訂正）**: 機構仮説の候補が
1つ見つかっている。`VK_IME_OFF = 0x1A`（`vk.rs:32`）であり、**JIS 106
配列における `@` キーのスキャンコード（set 1）は 0x1A**（実機は
JIS 106、known-bugs.md 記載）。参考（set 1 QWERTY 行、round1 レビュー
自身の誤りを round2 で訂正: `VK_IME_ON = 0x16` → JIS scan 0x16 は
**`U`**、`6`（数字、scan 0x07）ではない）: `VK_IME_ON = 0x16` → `U`、
`VK_KANJI = 0x19` → `P`。「どこかの層が `wVk` の値をスキャンコードとして
解釈している」という仮説が成り立つ。上記「背景」2. の反証はこの仮説の
「`wScan` を読む」版だけを排除しており、「`wVk` を scan として誤読する」
版は未検証のまま残っている。ただしこの仮説は mode=3（4イベントで「@」が
出ない）を単独では説明しない——Ctrl↑ の先行がなぜ VK 値の誤読を止める
のかは不明なままであり、正直に残る謎として明記する。

GJI 自身は closed-source のため機構の最終確認はできないが、上記の
未確定点1〜3は**いずれも安価に検証可能**であり、D4 で最優先事項とする。

## 決定

### D0（新規・最優先）: バッチ形状の修正方針を確定する前に、安価な機構検証を3件行う

以下は実機での確認のみで、コード変更を必要としない、または最小限の
一時的な観測で済む（v1 は「機構は不明」で思考を止めていたが、
`.claude/rules/tuning-constants.md` が禁じる「効かないので増やした」型の
対症療法と同じ形（測る前に動かす）になっていたため、これを是正する）。

1. **既存ログの再確認**: Ctrl+無変換 実行時の `[ime-mode] SendInput ...
   total={} events` ログを見て、実際に3イベントだったか4イベントだったか
   を確認する（未確定点1）。
2. **ON方向の対照実験（round2 で対象・観測文字を訂正）**: v1/v2 は
   `MsImeDirectStrategy::apply(true)` を対照として挙げていたが、この
   戦略は `ms_ime_direct_applicable(kind, profile)` が
   `ActiveImeKind::MicrosoftIme` を要求するため**BUG-113 の再現環境
   （GJI）では一度も走らない**——GJI 環境の対照にはならない。GJI +
   Windows Terminal で裸2イベントの `VK_IME_ON` を本番で送っている
   経路は `tsf/send.rs::send_eager_warmup_vk_pair`（eager warmup）のみ
   である。Windows Terminal + GJI で long idle（warmup が発火する
   状態）を作ってから最初の打鍵を行い、warmup 発火の瞬間に**「@」だけ
   でなく `u`/`U`（未確定点3、JIS scan 0x16）が混入しないかを能動的に
   観測する**（「@ が出ないことの確認」ではなく、Major 5 仮説が予測する
   具体的な文字を積極的に探す）。混入しなければ方向依存が確定し、
   VK 値の scan 誤読仮説も同時に弱まる。**v1 の D4 が提案していた
   「`AlreadyMatched` ガードを一時的に外す」変更は不要**——GJI 環境での
   裸2イベント ON は warmup 経路で既に本番稼働している（BUG-50
   デッドロック周辺のガードを触るリスクを避けられる）。
3. **VK値そのものの寄与を切り分ける**: `KanjiToggleStrategy`
   （`post_kanji_toggle_to_focused`、`VK_KANJI = 0x19`、`send_ime_mode_key`
   とは別の送信機構）を Windows Terminal + GJI で発火させ、`p`
   （JIS scan 0x19）が混入するか確認する。混入すれば原因は
   `send_ime_mode_key` のバッチ形状ではなく VK 値の解釈そのものにあり、
   本 ADR のスコープ設定が丸ごと外れていたことになる。混入しなければ
   `send_ime_mode_key` 経路限定の現象であるとの確証が強まる。
   **どちらに転んでも情報量が大きい、最優先の実験。**

3の結果次第では、D1〜D4 は全面的に作り直しになる可能性がある。
D0 は D4 の実機ラウンドを消費する前に済ませる。

### D1: パディングを入れる条件は「裸の2イベントバッチになる場合のみ」に限定する

`held_skip_alt.ctrl == false && held_skip_alt.shift == false`（`push_release`
の判定条件と一致することを確認済み、Alt は既に条件から除外されており
紛れ込みは無い、Major 6 で確認）のとき（＝ real な修飾キー由来の
パディングが無い場合）に限り、後述 D2 のパディングを追加する。

### D1'（新規・Blocker 2 対応）: パディング対象は `VK_IME_ON`/`VK_IME_OFF` の場合のみに限定し、呼び出し元列挙を訂正する

**v1 の「`send_ime_mode_key` は単一呼び出し元」という前提は誤りだった。**
実際の呼び出し元は3つある（`docs/experiments.md` エントリ20の表に既に
正しく列挙されていたものを v1 は見落としていた）:

| # | 場所 | 送る VK |
| --- | --- | --- |
| 1 | `ime_controller.rs:132`（`GjiDirectStrategy`） | `VK_IME_ON`/`VK_IME_OFF` 固定 |
| 2 | `ime_controller.rs:204/229`（`MsImeDirectStrategy`） | 同上 |
| 3 | `platform.rs:1276`（`send_engine_state_ime_key`） | **ユーザーが `config.toml` の `engine_on_ime_key`/`engine_off_ime_key` で任意指定した VK** |

`vk.rs:180-184` が明記する通り、`send_ime_mode_key` は
「open-only にも conv-mutating にもなりうる」ため、**呼び出し元単位では
なく実際に送信する VK の値で判定する必要がある**。`VkCode::from_name`
はユーザー設定に `"VK_KANJI"` のような**非冪等なトグルキー**を許容し
（バリデーションは無い）、`ime_toggle` の既定値自体が `"VK_KANJI"` である
ため、この設定は現実的にありうる。

もし D2 のパディング（対象VK自身を2回打鍵）を `send_ime_mode_key` 内部で
一律に行うと、ユーザーがトグルキーを設定している場合に**2回送信して
差し引きゼロになり、Engine OFF/ON 時に IME が一切切り替わらなくなる**
（Blocker 2、深刻な実害）。

**決定**: パディングは `send_ime_mode_key(vk)` の引数 `vk` が
`VK_IME_ON` または `VK_IME_OFF`（Windows 標準の真の開閉キー、呼び出し元
1・2が常に送る値）のときにのみ適用する。呼び出し元3（ユーザー設定 VK）
は、その VK が偶然 `VK_IME_ON`/`VK_IME_OFF` と一致する場合を除き対象外
とする。

**スコープ外として明記する関連関数**: 姉妹関数
`send_ime_mode_key_with_shift_release_prefix`（`ime.rs:332`、BUG-25 GJI
半角英数 entry/exit 用、`output/mod.rs:1231` から呼び出し）も、
`prepend_synthetic_shift_up == false` かつ修飾キー無しなら同じ裸2イベント
バッチになりうる。「バッチの形が原因」という仮説が正しいなら理論上
同じ症状を出しうるが、この関数は `VK_DBE_HIRAGANA`/`VK_DBE_ALPHANUMERIC`
という異なる意味論のキーを扱い、BUG-25/BUG-50 という別の複雑な経緯を
持つため、**本 ADR のスコープには含めない**。`docs/known-bugs.md` の
「未解決の疑問」に、この経路が同型のリスクを持つ可能性を明記するに
留める。

### D2: 候補は「バッチ分割」を第一候補、「自己エコー」を第二候補とし、フォールバックは実機で作り直す

**round1 レビューで追加（見落とし）**: v1/v2 は「パディングを足す」方向
（イベント数を増やす）しか検討しておらず、却下案にすら挙げていなかった。
72-77行の表の仮説は「`SendInput` 1呼び出しに含まれるイベント数」なので、
2 → 1 に**減らす**方向の変位も、2 → 4 に増やすのと同じだけ弁別力がある。
こちらを先に検討する。

#### 候補 V（新規・第一候補、未検証）: `SendInput` を down / up の2回に分割する

```text
1回目の SendInput: [vk↓]
2回目の SendInput: [vk↑]
```

修飾キーの release/restore が無い場合（D1 の条件）に限り、現状1回の
`SendInput` にまとめている `[vk↓, vk↑]` を、2回の独立した `SendInput`
呼び出しに分割する。新しいキーイベントを一切持ち込まないため:

- Blocker 2（`platform.rs:1276` 経由でユーザー設定トグル VK を二重送信して
  IME 切替が無効化される問題）が構造的に発生しない——同じキーを1回しか
  送らないため、D1'（VK 種別によるスコープ限定）を必須の前提にする必要が
  無くなる（ただし D1' 自体は引き続き有効な設計として残す）。
- Blocker 3（「冪等」への依存）も不要——同じキーを1回しか送らないので、
  冪等性の議論自体が要らない。
- 候補B の stuck Ctrl とも無縁。

リスクは down と up が別バッチになることで、その隙間に他のイベントが
割り込む可能性があるが、`send_ime_mode_key` は同一スレッドの同期呼び出し
であり実質ゼロに近い。**候補Aより先に実機で検証する。**

**既存の設計原則との関係（round3 で追加）**: `output/held_modifiers.rs:105-107`
（`send_keymap_target` の doc、ADR-114 決定3/ADR-130 決定3・4）は
「複数ステップの列全体を同一 `SendInput` バッチで送信する（描画前に
完結させ、中間状態を外部に見せない。Chrome cold-start 検出の VK_A+BS
アトミックバッチと同じ原則）」と明記しており、候補Vはこの原則を意図的に
破る。ただしこの原則は「**複数キーの列**の中間状態を外部に見せない」
ことが趣旨であり、単一キーの down/up には本来当てはまらない。加えて
本 ADR では「バッチとして一体に配送されること自体が症状の引き金」と
疑っているため、原則を破ること自体が検証したい介入そのものである。
この2点により、候補Vが原則の例外であることは妥当と判断する
（`send_keymap_target` 側を分割してよいという意味ではない——あちらは
複数キーの列であり本原則がそのまま適用される）。

**実装メモ（round3 で追加、Minor）**: `send_input_safe`
（`win32.rs:186-194`）の `conv_mutation::bump()` は `SendInput` 呼び出し
1回につき1回発火するため、候補Vは conv-mutating VK に対して bump が
2回になりうる。現行スコープ（D1' により `vk` は `VK_IME_ON`/
`VK_IME_OFF` に限定、`vk_may_mutate_conv` には含まれない）では無害だが、
D3 が呼び出し元3（ユーザー設定 VK、`VK_DBE_*` になりうる）を除外する
理由に「bump の二重化」も併記しておく（D3 参照）。また
`ime.rs:299-309` の `total=N events` ログは候補Vでは1回だけでなく2回
出るため、実装時はログに `split=true` と付記し、D0-1 が読む既存ログ
（1バッチの総イベント数という意味）を汚さないようにする。

#### 候補 V-2（V の変種、V が効かなかった場合のみ検討）: down と up の間に数ms の遅延を入れる

`.claude/rules/tuning-constants.md` の実測義務がかかる（新しい待機時間を
導入するため）。候補 V で十分なら不要。候補 V が「@」を止めるが別の
退行（例: 分割によるタイミング競合）を出した場合の調整弁として記録する
に留め、本 ADR では実装しない。

#### 候補 A（第二候補、未検証）: 対象 VK 自身を2度打鍵する自己エコーパディング

```text
vk↓(decoy) vk↑(decoy) vk↓(real) vk↑(real)
```

**Blocker 3（round1 レビュー）への対応**: v1 は `MsImeDirectStrategy` doc
の「Windows 標準の冪等な開閉キー」という記述を根拠に「バッチ内2回送信は
意味論上1回と同じ」としていたが、これは**論点先取**だった。この
「冪等」は「既に目的の open 状態なら no-op」という **open 軸に関する
状態冪等性**を指すのみであり、BUG-113 が観測している「@」は**まさに
これらのキーが open 軸以外に観測可能な副作用（余分な1文字の混入）を
持つことの証拠**である。known-bugs.md が記録する通り、mode 0 で「@」が
出た回でも直後の "a" は正しく半角で出力されている——つまり open 軸は
完璧に冪等に振る舞いながら、同時に文字を1個吐いている。「open 軸で
冪等だからバッチ内2回送信も安全」という推論は、いま反証されている
当の性質（このキーの作用が open 軸に閉じていること）を前提にしており
成立しない。**候補Aは「新しい VK も偽の修飾キーも持ち込まない」という
利点はあるが、それ自体は「安全である」ことの証明にはならない、
純粋に未検証の候補である**と明記する。

#### 候補 B（フォールバック）: 偽 Ctrl release/restore ブラケット — 現行設計は不採用、再設計が必要

**Blocker 1（round1 レビュー）: mode=3 の実装は Ctrl を「押しっぱなし」で
残す欠陥がある。実機で「@」が出なかった実績は、この欠陥がバレなかった
だけの可能性が高い。** 診断スパイクの実装（`ime.rs:361-377` 相当）は

```rust
make_key_input_ex(VK_CONTROL, true,  IME_KANJI_MARKER),  // KEYEVENTF_KEYUP
make_key_input_ex(VK_IME_OFF, false, IME_KANJI_MARKER),
make_key_input_ex(VK_IME_OFF, true,  IME_KANJI_MARKER),
make_key_input_ex(VK_CONTROL, false, IME_KANJI_MARKER),  // KeyDown、対になる Up が無い
```

という形で、実 Ctrl が押されていない状況でこのバッチを送ると、
**バッチ終了時点で OS のキー状態は Ctrl=DOWN のまま**になる
（`push_restore` が物理キー状態を再確認してから復元するのは、まさに
この事故を防ぐガードであり、mode=3 はそれを意図的に迂回している）。

失敗シナリオ: Windows Terminal で半角/全角キー1回押下の直後にユーザーが
`c` を打つと、アプリには **Ctrl+C**（実行中プロセスへの割り込み相当）
として届きうる。`HeldModifiers::read()` は awase 自身の物理キー状態を
読むため、この stuck Ctrl は awase 側から一切観測できない。Alt が
物理的に押されている状態（D1 の条件は ctrl/shift のみを見るため Alt
押下中でも裸2イベント判定になる）で発火すると Ctrl+Alt が居座る。
このリポジトリには `project_ctrl_mismatch_stuck_modifier`
（VcXsrv 等の stuck Ctrl が Chrome の IME-OFF を壊す既知問題）という
前例があり、同型のリスクである。

**さらに深刻な懸念**: BUG-114（[ADR-134](134-drift-correction-feedback-policy-focus-snapshot-staleness.md)）
の drift correction 暴走（20〜90ms 間隔の高頻度再送）が実機検証環境に
存在した状態で mode=3 を検証していたため、次に mode=3 が発火すれば
先頭の Ctrl↑ で stuck Ctrl が自己回復し、**stuck Ctrl が最長でも数十ms
しか持続せず観測されなかった可能性がある**。ADR-134 の修正で再送が
有界化された後（＝次の mode=3 相当の発火が来るまでの間隔が長くなった
後）に、初めてこの実害が顕在化するという最悪の順序になりうる。

**決定（round2 で「対称形」案を削除・一本化）**: 候補 B は現行の実装の
まま「実機検証済みの安全なフォールバック」として扱わない。

v2 は再設計案として対称形（`Ctrl↓(fake) vk↓ vk↑ Ctrl↑(fake)`、先に偽の
押下、最後に必ず偽の解放で終える）と3イベント版（`[Ctrl↑(fake), vk↓,
vk↑]`）の2案を並記していたが、**round2 レビューで対称形案自体に新規の
欠陥が見つかったため削除する**: 対称形は `vk`（`VK_IME_OFF`/`VK_IME_ON`）
を **Ctrl 押下中に配送する**。`send_ime_mode_key` の doc コメント
（`ime.rs:251-255`）は「Ctrl/Shift/Alt が押下中の場合…先に KeyUp を注入
して修飾なしで mode key を届ける…これを行わないと OS/IME/アプリが
`Ctrl+<mode key>` の組み合わせとして解釈し、想定外のショートカット発火を
招く」と明記しており、`push_release`→送信→`push_restore` という順序は
偶然ではなくこの不変条件を守るための設計である。対称形はこれを反転させ、
実機で「@」が出なかった条件（vk は Ctrl 非押下で届く）とはバッチ形状も
意味論も別物になる上、`Ctrl+VK_IME_OFF`/`Ctrl+VK_IME_ON` が Windows
Terminal や GJI 側で何を起こすか未知数という、新たなリスクを持ち込む。

**採用する再設計は3イベント版 `[Ctrl↑(fake), vk↓, vk↑]` に一本化する**
（D0-1 で Ctrl+無変換 の実バッチが3イベントで「@」が出ていないと確認
できた場合の設計に対応）。この形は偽の KeyDown を一切送らないため
stuck Ctrl が構造的に発生せず、vk も Ctrl 非押下で届くため上記の不変
条件を守る。D0-1 の結果が「4イベントでないと『@』が出ない」だった場合の
予備として、**末尾も KeyUp にする** `[Ctrl↑(fake), vk↓, vk↑, Ctrl↑(fake)]`
（KeyUp の重複は無害——`ime.rs:365-370` の左右 Shift 二重 KeyUp 送信と
同じ根拠）を第二候補として記録する。**どちらの形も stuck Ctrl を作らず、
vk を Ctrl 非押下で届ける**という2つの不変条件を満たす。

#### 決定

候補 V → 候補 A → 候補 B（再設計版）の順で実機検証する（D4）。候補 V が
「@」を再現しなくなることを確認できれば、新しい VK も偽の修飾キーも
二重送信のリスクも持たない候補 V を採用する。候補 V が効かない場合に
候補 A を検証し、候補 A も効かない場合に限り、上記の再設計を経た候補 B
へフォールバックする。**現行実装のままの mode=3をそのまま採用すること
はしない。**

### D3: 適用範囲は既定 off の hidden config で開始し、明示的な条件で既定 on へ昇格する

- 初期実装は既定 off の hidden config とする。`docs/experiments.md`
  エントリ20 の教訓を踏まえる。**設定名は機構を正確に表すものにする**
  （round1 レビュー指摘: v2 の `gji_off_batch_padding_enabled` は
  `gji`・`off` のいずれも不正確——実際には `send_ime_mode_key` の全経路
  〈GJI/MS-IME 共通、呼び出し元1・2〉に適用され、方向も ON/OFF 両方が
  対象になりうる。`ime_mode_key_batch_shape_fix_enabled` のように機構名
  で命名する）。
- 有効時のスコープは D1'（`vk` が `VK_IME_ON`/`VK_IME_OFF` の場合のみ）
  かつ D1（裸の2イベントバッチになる場合のみ）の両条件を満たす場合に
  一律に適用する。**呼び出し元3（`platform.rs::send_engine_state_ime_key`、
  ユーザー設定 VK）は明示的にスコープ外とする**（D1' 参照。理由は
  ユーザー設定トグルキーの二重送信/差し引きゼロ化に加え、候補V採用時は
  `conv_mutation::bump()` の二重発火リスクもある——`vk_may_mutate_conv`
  に含まれる `VK_DBE_*` 系がユーザー設定されうるため、将来スコープを
  広げる際はこの2点を再確認すること）。
- **既定 on への昇格条件を数値で定義する**（round1 レビュー指摘:
  「D4 で問題が無いことを確認できた場合」だけでは期限も条件も無く、
  ADR-081 Phase1d/1e が「ソーク待ち」のまま恒久凍結された前例と同じ
  形骸化リスクがある。しかも今回は既定 off のままだと BUG-113 が
  直らない——「hidden config で修正済み」に見えるが実際にはユーザーが
  有効化しない限り症状が残る、という誤解を招きやすい構造）。D4 の
  受け入れ基準（下記）を満たした時点で既定 on へ切り替える、と本 ADR
  自身が判定条件を持つ。それでも判定できない場合は、hidden config を
  作らず「診断スパイクのまま実機で詰めきってから一発で既定 on の修正を
  入れる」方を優先する。

### D4: 検証計画

D0（機構検証3件）を最初に行う。その後、既存の
`diag_bug113_mode_cycle_enabled` 診断スパイクを拡張し、以下を実機比較
する。**判定基準を事前に、候補ごとに定義してから実験する**（v1/v3は
「@」が出るか出ないかの二値でしか判定基準を書いておらず、`@@`（2個
混入）や「1個のまま残る」等の中間結果が出た場合に何を意味するかが
未定義だった、Blocker 3。round3 でさらに、候補が「パディング」から
「バッチ分割」（候補V）に変わったことで、既存の判定基準が候補Vには
そのまま適用できないと判明したため、候補ごとに分けて書き直す）:

**候補V（分割）の判定基準**:
- 「@」が0個: 「1回の `SendInput` に down/up が同居していること」が
  引き金と確定。仮説が「イベント数」から「呼び出し単位の同居」へ
  精密化される。候補Vを採用する。
- 「@」が2個（`@@`）: **`SendInput` 呼び出し1回につき「@」が1個生成
  される、という強い証拠**。仮説は「イベント数」でも「バッチ形状」でも
  なく「呼び出し回数」になり、パディング方向（候補A・候補B、いずれも
  1回のバッチに複数イベントを積む＝1呼び出し）は**全部逆効果**と判明
  する。**候補Bへのフォールバックは論理的に誤り**——候補Bも4イベント
  ＝1呼び出しであり、この観測下では進む理由が無い。D0-3 の VK 値仮説
  （Major 5）へ戻る。
- 「@」が1個のまま: 呼び出し形状は無関係。D0-3 の VK 値仮説へ。

**候補A（自己エコー）の判定基準**（候補Vが効かなかった場合のみ実施）:
- 「@」が0個: パディングが有効、候補Aを採用する。
- 「@」が2個以上（`@@`）: パディングが原因を解消せず悪化させた——候補Aの
  4イベント自体が新たな「@」の起点になっている可能性。候補Aを棄却し
  候補B（再設計版）へ。
- 「@」が1個のまま: パディングの形が不十分——イベント数を増やす方向
  ではなく、D0-3 の VK 値仮説（Major 5）に立ち返って再検討する。

0. **前提条件（round1 レビューで must に格上げ）**: [ADR-134](134-drift-correction-feedback-policy-focus-snapshot-staleness.md)
   の D1c（bootstrap 初期化）を先に実機ソークし、drift correction の
   暴走が解消されていることを確認してから本 D4 を開始する。理由:
   (a) 診断スパイクの安全弁 `DIAG_BUG113_MAX_INVOCATIONS = 32` は
   グローバルカウンタであり、暴走中の `apply(false)` 連打がこれを
   数秒で食い潰しうる（実際に known-bugs.md がこれを報告済み）。
   (b) 前回の実機検証終盤に発生した「物理半角/全角キーが `TurnOn`
   （no-op）としてしか認識されなくなる」不感状態（known-bugs.md
   BUG-113 参照）は、モード巡回が 0→1→2→3 の順であることと合わせると
   **候補 B（当時 mode=3、巡回の最後）を最も汚染しやすい位置**に
   置いていた。これは「本修正の検証と独立」な現象ではなく、実機データ
   の妥当性そのものを脅かす交絡要因である。**この不感状態が再度発生
   した場合、そのラウンド以降のデータは破棄する。**
1. **診断モードを絞る**: 6候補（0/1/2/3/V/A）を巡回すると1候補あたりの
   試行回数がさらに減る。mode=1/2 は既に結論が出ているため巡回から
   外し、**「0（対照）／候補V」→（V が失敗したら）「0（対照）／候補A」**
   の2択巡回に絞る。
2. **統計的信頼性の確保・安全弁の引き上げ（round2 で算術を明記）**:
   `GjiDirectStrategy::apply(open=false)` は `shadow_toggle_off_sync` と
   `engine_decision_sync` の2経路から同一の物理押下に対して連続で2回
   呼ばれる（`ime_controller.rs` コメント）ため、1 episode = 2
   invocation を消費する。**1候補あたり最低10回以上の連続無発生**を
   要求するなら、2択巡回（0/候補）×10 episode×2 invocation =
   **最低40、余裕を見て安全弁 `DIAG_BUG113_MAX_INVOCATIONS` を48程度に
   引き上げる**。既存の32のままでは1候補あたり最大8回にしかならず、
   要求回数に届かない。
3. **送信事実をログで裏取りする**: 「@」が出なかった判定の必要条件として、
   `[ime-mode] SendInput ... total=N events` ログが実際に出ていることを
   併せて確認する（前提条件0(b)の不感状態を検出する直接的な手段でもある）。
   また `set_diag_bug113_mode_cycle_enabled`（spike
   `ime_controller.rs:187-196`）は有効化のたびに invocation カウンタは
   リセットするが **`DIAG_BUG113_MODE_COUNTER` はリセットしない**ため、
   複数セッションに分けて試行回数を稼ぐ場合、再有効化後にどのモードから
   始まるかは実装側では予測できない。操作者はログの `mode=N` を必ず
   読んで現在のモードを確認すること。
4. Windows Terminal + GJI: 物理半角/全角キー単独押下で「@」が出ないこと
   （上記の拡充した試行回数で）。
5. Windows Terminal + MS-IME: 同上（MS-IME 側は未検証のため新規に確認）。
6. WezTerm（他の TsfNative）、Chrome/Edge/LINE/Teams（Imm32Unavailable）
   で IME ON/OFF・かな入力・変換候補が壊れないことの回帰確認。**候補A
   （自己エコー）を検証する場合、D1'によりVK_IME_ONにも`vk`の2回送信が
   入る**ため、GJI eager warmup（`send_eager_warmup_vk_pair`、ADR-100
   決定2で`VK_IME_ON`をTSF再初期化トリガーとして使用）が想定外に二重
   起動され cold-start リテラル化（BUG-02型）が新たに起きないことも
   確認する（候補VはVKを複製しないためこの懸念は無い）。
7. 候補 B（再設計版）を使う場合は、追加でグローバルホットキーアプリや
   アクセシビリティ系ソフトが誤動作しないことに加え、**半角/全角キーを
   押した直後に Windows Terminal で `c`/`a` を打鍵し、そのまま文字が
   出力されること（Ctrl+C として解釈されない＝stuck Ctrl が無いこと）
   を明示的に確認する**（round1 レビュー指摘: 目視確認の対象がグローバル
   ホットキー/アクセシビリティだけでは stuck Ctrl を検出できない）。

**撤退手順**: D3 で既定 off の hidden config として実装している間は、
新たな退行が出た場合 **config を off に戻すだけで撤退が完了する**
（コードの revert は不要）。既定 on へ昇格した後に退行が見つかった
場合は、[experiment-logging](../../.claude/rules/experiment-logging.md)
に従ってアプリ・IME・再現手順を revert コミットに記録する。

## 却下した代替案

### 案 X: 実機未検証のまま全アプリ・既定 on で投入する

`docs/experiments.md` エントリ20 で一度失敗した進め方そのものであり、
反証されたときの後始末が大きくなる。採用しない。

### 案 Y: mode=1（`ImmSetOpenStatus`、`SendInput` を使わないメッセージベース制御）

**却下根拠を訂正（round1 レビュー Major 8）**: v1/v2 は「`GjiDirectStrategy`
の設計原則と衝突する」「TsfNative では歴史的に不安定・ハングしうる」と
書いていたが、`GjiDirectStrategy` の doc（`ime_controller.rs:102-115`）
にはそのような記述が無い。存在しない根拠を引用していた。

正しい却下根拠は次の2点である。(1) `platform.rs:1209-1214` が明記する
実装済みのガード:「IMM32 API で直接 open/close できないアプリ
（Imm32Unavailable/TSF-native）では `get_gui_thread_info` +
`send_ime_control` が ~200ms タイムアウトしてブロックする」ため、
`can_use_imm32_cross_process() == false` なら早期 return する設計に
なっている。(2) 実機診断で、mode=1 を診断ローテーションに含めると
**drift correction が収束を確認できず 20〜90ms 間隔で `SendInput`/
`ImmSetOpenStatus` を数百回連続発火させる暴走が実際に発生した**
（`ImmSetOpenStatus` 自体は成功と記録されるが、プロファイルは TsfNative
＝ IMM 不可と分類されているため、drift correction 側が収束を観測できず
無条件に送り続ける——ADR-134 が扱う SSOT 分裂と同型の非対称）。

「API 呼び出しは成功するが状態の読み返しができず収束判定できない」と
いう本質的な問題があるため、恒久採用するなら「TsfNative でも IMM を
使ってよい」というプロファイル設計自体の再定義（ADR-119 の InputRelay
ゲート新設に匹敵する規模）が必要になる。優先度を下げ、候補 V/A/B が
いずれも実機で失敗した場合の最終手段として残すが、**採用する場合は
プロファイルゲートの扱いと収束判定の獲得手段を新たに設計すること**を
条件として明記する。

### 案 Z: mode=2（別バッチでの keystate-clear 前処理）

実機で「@」の再現に変化が無かった（「残留キー状態」仮説の反証）。不採用。

### 案 W: VK_KANA/VK_KANJI 自体の置換（旧 D1〜D6）

この機体では到達不能コードだったと確定済み。他のキーボード/ドライバ
環境で `VK_KANA`(0x15) が実際に届く可能性は排除できていないため、旧
実装（`spike/adr133-wt-vk-kana-dbe-hiragana` ブランチ）は削除せず残すが、
develop へマージするかは別途ユーザー判断とする（本 ADR のスコープ外）。

## 未解決の疑問

- D0-3（`KanjiToggleStrategy` での `p` 混入テスト）の結果次第では、
  本 ADR のスコープ設定（`send_ime_mode_key` のバッチ形状）自体が
  誤っている可能性が残る。
- `send_ime_mode_key_with_shift_release_prefix`（BUG-25 GJI半角英数
  entry/exit）が同型の裸2イベントバッチ問題を抱えている可能性がある
  （D1' 参照）。本 ADR のスコープ外だが、将来の調査対象として記録する。
- 検証終盤に発生した「新規に開いた Windows Terminal ウィンドウで物理
  半角/全角キーが `TurnOn`（no-op）としてしか認識されなくなる」現象
  （known-bugs.md BUG-113 参照）の原因そのものは未調査のまま。ただし
  この現象が実機データの妥当性に及ぼす影響は D4 前提条件0(b)で対処済み
  （検出したらそのラウンド以降のデータを破棄する）。
- BUG-114（[ADR-134](134-drift-correction-feedback-policy-focus-snapshot-staleness.md)）
  は独立の原因として別途修正する。D4 の実機検証は ADR-134 の D1c
  （bootstrap 初期化）を先に安定させてから行うこと（**D4 前提条件0
  参照、must**——BUG-114 の暴走が残ったままだと、上記「Blocker 1」で
  指摘したような「別バグの副作用で問題が隠れる」事故が起きうる）。
