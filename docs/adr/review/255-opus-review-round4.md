# ADR-255 敵対的レビュー round4(Opus)

対象: `docs/adr/255-ime-off-thumb-key-space.md`(ワークツリー keymap-ime、HEAD `01a98ccb`)。行番号はこの HEAD のもの。

## 総評

**新規の Blocker はなし。新規の Must は 3 件。** いずれも安全側の問題で、誤って Space を出す穴ではない。

- R3-B1 の対応(`KeyOpenEffect` の三値、発動は `NoEffect` のときだけ)で、IME を開く手段を奪う経路は閉じた。決定3 の述語の確定(R3-M1)と CI の入力先の指定(R3-M2)も意図どおり。
- 残る問題は「**報告者の構成で実際に発動するか**」の見込みが ADR の記述より悪いこと。
  - 中心は R4-M1・R4-M2: 既定のプリセットでは変換が発動しない。報告者自身が足した CUSTOM 行が発動を止める。
  - R4-M3: 決定3b の確認事項(2)は、タスクバーの表示では答えられない。

---

## r3 対応の確認

| ID | 判定 | 根拠 |
| --- | --- | --- |
| R3-B1 | 満たす(安全性)。発動範囲の見込みは R4-M1/M2 | 発動は `NoEffect` のときだけ。不明は `Unknown`(MS-IME 本体・TIP 未同定・表が読めない)。オーバーレイは `Opens` |
| R3-M1 | 満たす | 述語を `cannot_verify_real_ime_state`(`class_names.rs:277-286`)に確定した。範囲の記述も正しい |
| R3-M2 | 満たす | 計画0(ii) で入力先の分類を確かめ、計画2 の入力先を Standard にした。対照(d) も足した |
| R3-S1 | 満たす | フックが `impersonated` の印を付ける |
| R3-S2 | **未対応** | 前置する遷移の effects に含まれる `SetOpen(false)` の扱いが ADR に書かれていない(下の R4-S5) |
| R3-S3 | 満たす | 計画1 の軸と計画2(c) を足した |
| R3-S4 | 満たす | 計画0(v) に run・ジョブ・ログの名前を添えた |

---

## Must

### R4-M1. プリセットの `DirectInput` 行は本番のコードに無い。既定の MS-IME プリセットでは変換が発動しない

実コードの裏取り:

- `role.rs` の判定は、プリセットについて「トグルを持つキーの名前の一覧」(`Preset::toggle_vk_names`、`crates/awase-gji-config/src/role.rs:68-75`)しか持っていない。Mozc の TSV 本体は**同梱しない**(ADR-199 決定4。role.rs のテストの注記、:405-408)。各プリセットの `DirectInput` 行は**テストの定数**(`MS_IME_TSV`/`ATOK_TSV`/`KOTOERI_TSV` と `*_DIRECT_INPUT`、:410-505)にしか無い。
  - 決定2-4(ii) の「プリセット……の `DirectInput` 行にそのキーの行があれば `Opens`」を実装するには、4 プリセットの変換/無変換の `DirectInput` 行を**本番の表として新たに持つ**必要がある。ADR に、この表を足すこと・その出典(Mozc `b4bbc42f`)・Mozc の更新で古くなる性質(テスト `passive_open_vk_names_outside_table_match_mozc_direct_input_rows` と同じく手で取り直す)を書くこと。前例は ADR-211 の `passive_open_vk_names_outside_table`(:110-119)で、プリセットごとの定数を `source()` で選ぶ形がそのまま使える。
- その表で判定した結果(テストの定数より):

| GJI のキー設定 | 無変換 | 変換 |
| --- | --- | --- |
| MS-IME(**既定**。未設定・NONE・空の CUSTOM もこれ、`source()` :86-98) | `NoEffect` → 発動 | `DirectInput\tHenkan\tReconvert` → **機能あり、発動しない** |
| ATOK | `IMEOn` → 発動しない | `IMEOn` → 発動しない |
| KOTOERI | `NoEffect` → 発動 | `NoEffect` → 発動 |
| MOBILE | MS-IME と同じ | MS-IME と同じ |
| CUSTOM | 表しだい | 表しだい |

- 決定2-4 の末尾「報告(GJI)の構成には合う」は、この表を見ると言い過ぎ。報告は「変換/無変換」の両方だが、**既定のプリセットでは無変換しか Space にならない**。ATOK プリセットでは両方ならない。決定3b の確認事項に「GJI のキー設定(プリセット名、または CUSTOM)」を足し、上の表を ADR に載せること。
- `Reconvert`(再変換)を「機能あり」に数える判断そのものは安全側で妥当。ただし直接入力での再変換は、選択した文字列があるときしか効かない。変換を Space にしたい人が「再変換を捨ててよい」と選べる余地を残すかを、未解決の疑問に足すこと(R4-S1)。

### R4-M2. 報告者自身の CUSTOM 行(InsertSpace 等)が発動を止める

- 報告者は GJI で「直接入力の無変換/変換を空白入力」に割り当てようとした。CUSTOM 表には `DirectInput\tMuhenkan\tInsertSpace` 等の行が**残っている**可能性が高い。決定2-4(ii) は `DirectInput` 行があれば(コマンドを問わず)機能ありとするので、**報告者の構成では両キーとも発動しない**。
- 対応は次のどちらかを決定として書く。
  - (a) 報告者への案内に「GJI の CUSTOM 表から、直接入力の無変換/変換の行(InsertSpace 等)を消す」を含める。決定3b の返答内容と設定画面の注記に入れる。
  - (b) `DirectInput` 行のコマンドが `InsertSpace`/`InsertHalfSpace`/`InsertFullSpace` のときは `NoEffect` に数える。CI では効かなかったが、仮説 a〜c が未分離なので、効く環境で Space が二重になる危険が残る。
  - 推奨は (a)。(b) は仮説 a の確認(設定ダイアログで直接入力行に InsertSpace を選べるか)が済んでから。
- 計画1 の表に「CUSTOM の `DirectInput` 行が InsertSpace 系」の行を足し、期待値を (a)/(b) のどちらかで固定する。

### R4-M3. 決定3b の確認事項(2)はタスクバーでは答えられない。報告機能で集める

- 「タスクバーの表示が『A』(直接入力)か『A』の半角英数か」は、どちらも「A」と表示される(MS-IME 本体も GJI も、直接入力と半角英数の両方で「A」を出す構成がある)ので、利用者には答えようがない。答えがずれると、ゲートの判定(決定7 の `NotRomajiInput` を範囲外にした判断)がずれる。
- awase の「不具合を報告」機能(ADR-095、トレイ)には、フォーカス窓のクラス・プロファイル・belief(open・input mode)が入る。確認事項 (1)(2) は、**「問題の起きるアプリで、IME OFF の状態のまま無変換を押した直後に『不具合を報告』を送ってもらう」**の 1 手で客観的に集められる。決定3b の (1)(2) をこれに置き換えること。手で確かめてもらうなら、GJI のツールバー(言語バー)の入力モード表示で「直接入力」か「半角英数」かを見てもらう。
- 確認事項は 4 つになる: (1)(2) 報告機能、(3) 押し続けたときの期待、(4) GJI のキー設定のプリセット名/CUSTOM(R4-M1)と、CUSTOM なら直接入力の無変換/変換の行(R4-M2)。

---

## Should

### R4-S1. `Reconvert` を Space で上書きする選択肢

- 既定プリセットの変換は `Reconvert` で、R4-M1 により発動しない。報告者が「変換も Space」にしたい場合の道は、(i) GJI のキー設定を CUSTOM にして該当行を消す、(ii) awase 側に「再変換を上書きしてよい」を足す、の 2 つ。今回は (i) の案内で足りるはずなので、未解決の疑問に記録し、(ii) は需要が出てから扱う。

### R4-S2. 未知のオーバーレイは `Unknown`

- 決定2-4(ii) は既知のオーバーレイ(`SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`)を `Opens` にすると書くだけで、未知のオーバーレイに触れていない。`key_role` も、未知のオーバーレイは「書き換える行が分からないので全キーを受動」にしている(role.rs:121-141)。三値でも、未知のオーバーレイがあれば `Unknown` と明記すること。
- `source()` が `(Some(_), _) => Source::Unknown`(未知の `session_keymap`)を返す場合も `Unknown` に入れる(表に書かれているのは「表が読めない」だけ)。

### R4-S3. 修飾付きの行と無修飾の行を分ける

- `ATOK_DIRECT_INPUT` には `Shift Henkan\tReconvert` がある(role.rs:498)。発動条件は無修飾(`is_bare_thumb` が Shift を除く)なので、`DirectInput` 行の照合は**無修飾のキー名の完全一致**にすること。`Shift Henkan` を変換の行と数えると、ATOK 以外の CUSTOM でも誤って `Opens` になる(安全側ではあるが、発動範囲が理由なく狭まる)。`KeyStates::of`(:241-)が既にこの区別をしているかを実装時に確かめ、テストの 1 行で固定する。

### R4-S4. `KeyOpenEffect` をエンジンへ運ぶ経路

- 判定に要る GJI の表は、シェル側の `key_effect_keymap.get_gji(now_ms)`(I/O と間引きあり、`runtime/mod.rs::derive_key_shadow_action`)にある。エンジン(OS 非依存)は自分で読めない。既存の `enrich_thumb_key_role` が押下ごとに `set_thumb_role_open_actions` で押した側だけ書く形(`runtime/mod.rs:796-836`)に揃え、同じ場所で `KeyOpenEffect` も押した側だけ書く(古い値を残さない。決定8「古い役割を残さない」と同じ理由)。ADR の影響範囲にこの経路を書く。

### R4-S5. 前置する遷移の `SetOpen(false)`(R3-S2 の再掲)

- 決定4 の「`check_active_transition` の effects を前置する」で、活性→非活性の遷移に `SetOpen{open:false}` が含まれうる(`transition_activation`、`engine.rs:411-440`。`emit_set_open` しだい)。IME が既に閉じている窓へ 1 回の actuation になる。どの `emit_set_open` で呼ばれるか、IME actuation 合流点ファミリーに触れるか、`press=None` で良いかを一行書くこと。前置するのを `EngineStateChanged` と保留出力の解放だけにして `SetOpen` を外す選択肢もある。

### R4-S6. 細部

- 決定3 の注記の「Chrome・Edge・VS Code・Windows Terminal・UWP など」は正しい。計画0(ii) で Windows 11 のメモ帳が対象外と分かった場合、「メモ帳でも動かない」と注記に足すこと。報告者が試しに使う最初のアプリになりやすい。
- 決定2-4 の「決定5の注記『IME 側の割り当てより優先される』が当てはまるのは……GJI の設定ファイル外に限る」は、MS-IME 本体を `Unknown`(発動しない)にしたので、GJI ではほぼ空になる。注記そのものを「IME の割り当てが読めないときは何もしない」に書き換えてよい。

---

## 確認に使ったコマンド(HEAD `01a98ccb`)

- `sed -n 56,141p crates/awase-gji-config/src/role.rs`(プリセットはトグルの名前一覧だけ、`source()`、`passive_open_vk_names_outside_table`、オーバーレイの扱い)
- `sed -n 405,505p crates/awase-gji-config/src/role.rs`(Mozc `b4bbc42f` の各プリセットの行。テストの定数)
- `grep -rn "Henkan\|Muhenkan" crates/awase-gji-config/src/*.rs | grep -i directinput`
- `sed -n 796,836p crates/awase-windows/src/runtime/mod.rs`(押下ごとの役割の書き込み)
- 未確認: `KeyStates::of` が修飾付きの行を別扱いするか(R4-S3)。GJI のツールバーで直接入力と半角英数の表示が実際に分かれるか(R4-M3)。
