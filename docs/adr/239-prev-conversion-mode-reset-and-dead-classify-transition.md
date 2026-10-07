---
id: ADR-239
title: |-
  `prev_conversion_mode` が読み取りのたびに消える件と、本番で動いていない `classify_transition` の扱い
summary: |-
  `runtime/focus_tracking.rs::advance_focus_tracking` は、フォーカスが変わらなくても IME の読み取りのたびに走り(`ir_stage_focus` → `apply_focus_probe_result`)、末尾で `prev_conversion_mode` を毎回 `None` に戻す。元の意図(e1babb40)は「異なるウィンドウの conversion_mode を比較しない」(フォーカス変更時だけ)だった。結果、`input_mode_from_conversion`(`ConvMode::classify_transition`)は poll の経路で常に `prev=None` となり、手元の CI 754 run で `IME input method changed: conv=…` が 0 件、`[eisu-adopt]` の `prev_conv` は全件 `None`。この経路の 2 つの検出(英数への遷移、ROMAN ビットの変化)は、どちらも別の経路(`is_eisu_evidence`、`input_mode_from_romaji_flag`)と重なっている。
  案: A(リセットをフォーカス変更時だけにする=意図どおりに直す)・B(動いていない経路を消す)・C(そのままにして記録する)。推奨は B(挙動を変えない撤去)だが、起草段階で Opus レビュー前。
status: |-
  起草中(2026-10-06)。調査のみ、実装は未着手。Opus 敵対レビュー前。
related_adr:
  - "ADR-238"
  - "ADR-186"
---

# ADR-239: `prev_conversion_mode` の毎回リセットと、本番で動いていない `classify_transition`

## 背景(ADR-238 の実装中に見つかった)

ADR-238(BUG-190)の実装で、`[eisu-adopt]` の `prev_conv` が**全件 `None`** なのに気づいた。原因を調べた。

### 事実

1. `Runtime::apply_focus_probe_result`(`runtime/focus_tracking.rs:77`)は、IME の読み取りのたび(`ir_stage_focus`〈`runtime/ime_refresh.rs:85`〉→ `detect_and_update_focus`/`apply_focus_probe_result`)に走る。その中の `advance_focus_tracking`(361 行)は、フォーカス(プロセス/ウィンドウ)が変わっていなくても毎回、末尾で
   `self.platform_state.ime.set_prev_conversion_mode(None);` を実行する(445 行付近)。
2. この reset の元の意図は、e1babb40(2026-04-24、`ImeSnapshot: 3値意味論の導入`)の「prev_conversion_mode をリセット(異なるウィンドウの conversion_mode を比較しない)」。**フォーカス変更時だけ**のつもりだったが、条件が付いていない。
3. したがって poll の経路(`poll_and_classify_ime`/`classify_fetched_snapshot` → `classify_ime_snapshot`)では、`current_prev_conversion_mode` が常に `None`。`input_mode_from_conversion`(`observer/ime_observer.rs:115`、`let prev_conv = current_prev_conversion_mode?;`)は常に `None` を返し、`ConvMode::classify_transition`(`src/engine/conv.rs:141`)は**poll の経路では一度も評価されない**。
4. 裏取り(手元の CI ログ 754 run): `IME input method changed: conv=…→…`(`input_mode_from_conversion` が結果を返したときの info ログ)は **0 件**。`[eisu-adopt]` の `prev_conv` は全件 `None`。
5. `prev_conversion_mode` が `Some` になるのは、TsfNative の idle-conv-check(`runtime/key_pipeline.rs:752`、2810 行)が直接 `set_prev_conversion_mode(Some(conv))` を書く場合だけ。これは poll の経路ではない(`conv_raw` の診断〈`key_pipeline.rs:1774`〉でも読まれる)。

### `classify_transition` が検出するもの(2 つ)と、重なる別の経路

| 検出 | 条件 | 重なる経路 |
| --- | --- | --- |
| 英数への遷移 | 今回が英数・前回が非英数 → `ObservedEisu` | `ConvMode::is_eisu_evidence`(`classify_ime_snapshot` の (a))。ただし `ime_on == Some(false)` では (a) は conv=0 を無視する(BUG-57)。(b) は `ime_on` を見ないので、prev が生きていれば `ime_on=false` の conv=0 も ObservedEisu にする |
| ROMAN ビットの変化(NATIVE あり) | ひらがな↔ローマ字 → `ObservedRomaji`/`ObservedKana` | `input_mode_from_romaji_flag`(毎回の `is_romaji` と現在の mode を比べる) |

つまり (b) は (a) と `input_mode_from_romaji_flag` に**実質的に包含**され、しかも prev が生きていれば BUG-57 の保護を破る向きに働く。**現在の「毎回 None」は、結果として BUG-57 の保護を守っている。**

## 決めること

`prev_conversion_mode` の毎回リセットと、poll 経路で動いていない `input_mode_from_conversion`/`classify_transition` を、どうするか。

## 案

| 案 | 内容 | 影響 |
| --- | --- | --- |
| A | リセットを**フォーカス(プロセス/ウィンドウ)が実際に変わったときだけ**にする(ADR-238 で候補の破棄に入れた条件 `process_changed \|\| prev_hwnd != new_hwnd` と同じ)=元の意図どおりに直す | (b) が本番で動き始める。`ime_on=Some(false)` の conv=0(閉じた窓、フォーカスが一瞬通る窓)を ObservedEisu にして BUG-57 を再発させる恐れ。`input_mode_from_romaji_flag` と重なる検出が増えるだけで得るものが小さい。ADR-238 の候補の確認とも相互作用(prev が生きると (b) 経由の ObservedEisu も候補のフィルタを通るが、`ime_on=Some(false)` を弾く条件は (a) 側にしか無く、フィルタは `ime_on=None` だけ弾く) |
| B | **動いていない経路を消す**: poll 経路の `input_mode_from_conversion`(と、それだけが使う `classify_transition` の英数遷移・ROMAN 変化、`current_prev_conversion_mode` の引数)を撤去する。TsfNative の idle-conv-check が使う `prev_conversion_mode` と `classify_idle` は残す | **挙動は変わらない**(現状 `prev=None` で常に None を返しているため)。コードが減り、ADR-238 の「(a)(b) の 2 生成箇所」が (a) 1 つになる。`ConvMode::classify_transition` が他から使われていなければ純粋関数ごと撤去(テストも)。BUG-57 の保護が「偶然」から「構造」に変わる |
| C | そのままにして、`prev_conversion_mode` の挙動(毎回リセット)と (b) が死んでいることを docs に記録する | 変更なし。(b) が死んでいることを知らない人が、prev を使う変更を足して意図せず BUG-57 を再発させる恐れが残る |

## 推奨(案、Opus で決める)

**B。** 動いていない経路は、直すより消す(「設計の転換は新機構の追加でなく場当たり的なパッチの撤去」の方針、ADR-217 と同じ流れ)。挙動が変わらないことは、(1) 手元の CI 754 run で `input_mode_from_conversion` の結果が 0 件、(2) 構造上 `prev` が poll の経路で常に `None`、の 2 つで示せる。実装時は journal リプレイ/閉ループで「撤去の前後で `new_input_mode` の列が同じ」を固定する。

## 評価に必要な測定(実装前)

1. `classify_transition` の呼び出し元を全件洗う(`ConvMode::classify_transition` が `ime_observer.rs:122` 以外から使われていないか。`classify_idle`/`classify_conv_transition` は別関数)。
2. poll の経路で `current_prev_conversion_mode` が `Some` になりうる経路が本当に無いか(`apply_ime_update` の `new_prev_conversion_mode` は書くが、次の読み取りの前に `advance_focus_tracking` が必ず消すか)。**順序の確認:** `ir_stage_focus`(reset)→ `ir_stage_observe`(`capture_poll_state` → classify)→ `apply_ime_update`(prev を書く)→ 次の tick の `ir_stage_focus`(reset)。ただし `process_deferred_keys`(`runtime/mod.rs:1378`、本番から到達しない)・ImmCrossProbe(`key_pipeline.rs:2868`)は `ir_stage_focus` を経ずに prev を読む。ImmCrossProbe は prev を書かない(`new_prev_conversion_mode` を捨てる)ので、直前の OsPoll が書いた prev を、次の `ir_stage_focus` が消す前に読む窓がありうる。**この窓で (b) が評価されるか**を、診断ログ(`[eisu-adopt]` の `branch=b` と `prev_conv!=None`)の件数で確認する(今のところ全件 `None`、(b) の件数 0)。
3. 撤去の前後で、`msime-native*`・`sc-dbe-*`・`sc-shift-msime-native`・`msime-stale-table`(ADR-238 と同じ構成)が同じ結果になること。

## 状態

起草中。調査のみ。Opus 敵対レビュー前。
