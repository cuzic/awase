---
id: ADR-195
title: |-
  カスタムキーマップ対応のため、IMEキー効果の学習（awase-keymap-learn）を製品化する（設定読取→独立プロセスでの巡回学習→自己検証→永続化→実行時読込→ADR-176統合）
summary: |-
  ADR-191は「IMEが状態の正、awaseは書かず観測+打鍵時予測(KeyEffectPredicted)に追随する」を実現したが、予測表は現状コンパイル時に生成データ
  （`state/key_effect_table.rs`、予測器本体は`state/key_effect_predictor.rs`）として埋め込まれており、**カスタムキーマップのユーザーには
  対応できない**（これがADR-191決定3の目的そのもの）。本ADRは、この学習をユーザーの実機で製品として動かす計画を定める。
  **rev3の核心**: round1〜round2のopus-adversarial-consultで、rev2までの設計は2つの誤りに基づいていたと判明した。
  (A) 「予測（本ADRのスコープ）」と「actuation可否（ADR-189の固定セット+ユーザー明示config、本ADRの範囲外）」は develop の現状でも
  完全に別経路であり、**予測は一切IMEへ書き込まない**ため「書き込み対象への昇格ゲート」自体が不要——rev2の段階1aは、これを勘違いして
  「予測を落とす」方向に実装してしまい、ADR-189の固定セット（漢字0x19・半角/全角0xF3/0xF4）と`VK_IME_ON`/`OFF`（分類a、awaseが書いてよい
  代表）まで同梱表から29〜43%削る欠陥ゲートになっていた（round2 R-2）。本ADRはゲートを丸ごと削除し、代わりに「actuationの許可リストは
  ADR-189+ユーザー明示configのままで、学習結果から自動拡張しない」という1行の制約だけを置く。
  (B) 学習をawase.exe自身の非同期ランタイム（`winmsg-executor`）に統合する設計は、ADR-191決定3が要求する「awaseを完全バイパスした注入
  （A'）でなければ合成規則を学習してしまう」実測（A' 98.5% 対 B（awase経由）84.5%）と、`191-gji-state-scope-spec.md`が確定した
  「開閉・変換モードの保持単位はTSFスレッド」という2つの既決事項に構造的に矛盾していた（round2 N-1/N-2）。本ADRは、学習を
  **awase.exeとは別の独立した短命プロセス**（既存の`ime_key_matrix_spike.rs`/CI格子ツール一式の系譜）として実装し直す。注入・観測・
  窓の所有をすべて同一プロセスの単一スレッドに揃え、時間会計もシミュレータの仮想`CostModel`ではなく実時計で行う。
  段階構成: (0)設定の読み取り（既存3経路の統合、B-1で収束済み。ADR-192決定1の検出ロジックを再利用し重複実装しない）、(1)独立学習プロセス
  （`ImeDriver`トレイト+実機ドライバ、`SendInput`直叩き+`begin_calibration_bypass`型の窓バイパスでA'を確保、実時計）、(2)自己検証
  （独立ウォーク、実時間）、(3)永続化（表全体を別ファイルに分離）、(4)実行時読込（予測にのみ使う、actuationには使わない）、(5)隠れ状態を
  最小Mealy機械として持つ、(6)ADR-176ウィザードからの起動統合（子プロセス+IPC）、(7)安全対策、(8)陳腐化検出（全体フィンガープリント）。
  ADR-194（シミュレーション・リプレイのハーネス）とADR-192（状態依存キーの警告UI、検出ロジックは段階0で再利用）は範囲外（参照のみ）。
status: |-
  **草案 rev3（2026-09-22、opus-adversarial-consult round2の指摘〈Blocker R-2/R-2b/N-1/N-2、Major N-3/N-4/R-3/R-4、round1未対応分M-4/M-6〜M-9〉
  を反映し、team-lead確定の新設計方針〈書き込みゲート撤廃・独立プロセス化〉に沿って全面改訂。round3待ち）。**
  実装はまだ無い（`awase-keymap-learn`クレート自体は`feat/awase-calibration`ブランチにdevelop未マージで存在するが、巡回プランナと
  シミュレータのみで、本ADRが定める製品への組み込みは未着手）。
related_adr:
  - "ADR-019"
  - "ADR-090"
  - "ADR-153"
  - "ADR-162"
  - "ADR-176"
  - "ADR-189"
  - "ADR-190"
  - "ADR-191"
  - "ADR-192"
  - "ADR-194"
---

# ADR-195: IMEキー効果の学習（awase-keymap-learn）の製品化

## 背景

[ADR-191](191-ime-is-source-of-truth-observe-not-write.md)（撤去ブランチ`feat/adr191-remove-hardcoded-mode-keys`、develop未マージ）は、モードキー（ひらがな・カタカナ・英数・無変換・変換など）の
決め打ちを撤去し、打鍵の時点で`(状態, キー)`→効果の表を引いてbeliefを予測する方式に置き換えた。予測器（純粋関数）は
`state/key_effect_predictor.rs`、表の生成データは`state/key_effect_table.rs`にある。生成データは
`tools/e2e/ime_key_matrix/gen_key_effect_table.py`がCI実機の格子学習（`--grid`、awaseを完全にバイパスした注入）から**コンパイル時に**
生成する。対応するキーマップは、ATOK・GJIのMS-IMEプリセット・Microsoft IME本体の3種のみで、いずれもCI実機の既定のキーマップから作った。

ユーザー方針（2026-09-20）の出発点は「カスタムキーマップに対応するというのがそもそもの目的です、必須ですね」であり、コンパイル時埋め込みの3種の表では、
この目的を達成していない。ユーザーが実際に使っているキーマップ（GJIの`custom_keymap_table`、MS-IMEのキー割り当て変更）に対しては、ADR-191の予測器は
「表に無い・カスタムが上書きしている」として`None`（予測しない、観測に任せる）を返す設計になっており、これは安全側の縮退であって対応ではない。

### 予測とactuationは別経路である（rev3の出発点、round2 R-2が指摘した誤りの根）

develop の現状の実装で、次の2つは**完全に別のデータ・別の経路**である:

- **予測（`KeyEffectPredicted`）**: `(状態, キー)`→効果の表を引いて、押した直後にbeliefを更新するだけの、**IMEへは一切書き込まない**機構
  （`state/key_effect_predictor.rs`）。誤って予測しても、後続の観測が100ms（`KEY_EFFECT_SETTLE_MS`）の猶予つきで訂正する。
- **actuation可否**: awaseが実際にIMEへキーを送ってよい対象は、ADR-189が固定する「beliefに基づく開閉トグルとして書いてよい」極少数のVK
  （漢字0x19、半角/全角0xF3/0xF4。`vk.rs::ImeKeyKind::is_open_toggle_for`が判定する）と、ユーザーが明示的にconfigへ書いた対応
  （`keys.ime_on`/`keys.ime_off`/`keys.ime_toggle`、親指キーの`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`）**だけ**であり、
  `runtime/mod.rs::enrich_ime_relevance`等はこの許可リストを見て`RawKeyEvent`にactuation可否のヒントを付けるだけで、学習結果や予測表を
  見て許可リストを広げる経路は存在しない。

**本ADRのスコープは前者（予測）だけ**であり、学習した内容が後者（actuation許可リスト）へ自動的に反映される経路を作らない限り、
「学習した結果を、開閉だけに作用するキーとそれ以外に仕分けてから書き込んでよいか判定する」というゲートは、そもそも要らない
（rev2の段階1aは、この区別を見誤って予測表そのものを削る方向にゲートを実装してしまっていた。詳細は「却下した設計（rev2からの変更点）」節）。

学習そのものの設計（巡回プランナ、待ちの短縮、状態の作り方の教訓）は、この会話の中で並行して積み上げてきた:
- **巡回学習の設計**: 文献調査（status message付きtransition tour、有向CPPの最適解、rural CPP、L\*/TTT/UIO/W法は不採用、「各セル2回」は統計的にほぼ
  無意味〈rule of three〉、部分1-switch）に基づき、`crates/awase-keymap-learn`（旧名`awase-calibration`。OS非依存の純粋Rust、`feat/awase-calibration`
  ブランチ、develop未マージ、現状+3,406行〈ADR-191決定5〉）でシミュレータを作り、戦略S0〜S9を比較した。**S6（部分1-switchの有向CPP）+
  イベント待ちを推奨**（理想条件でS0比、時間90%減）。
- **CI高速化**（PR #237、developマージ済み）: `--fast --notify --grid-adaptive`でATOK格子（132セル）4シャードの学習本体が
  s1 232秒・s2 224秒・s3 306秒・s4 263秒（[191-calibration-experiments.md:105](191-calibration-experiments.md)、run 35582339438、遅い版〈1,287〜
  1,693秒/シャード〉との差分0）。CIはこの4シャードを別々のジョブとして**並列**実行するため壁時計は最長シャード相当（約5分）だが、
  1台のユーザー実機で逐次実行すれば合計値に近い**約17分**が現実的な下限になる。**この時間は学習（格子巡回）単独であり、自己検証
  （段階2、独立ランダムウォークでの採点）の時間は含まない**——`score_walk.py`は別工程で、所要時間は計測されていない。
- **状態の作り方の教訓**: 格子は「全状態をキーだけで作る」（IMM書き込みで状態を作ると、実際にキーで到達した状態と乖離する。第1〜3版の変遷、
  [191-calibration-experiments.md](191-calibration-experiments.md)）。
- **GJIの状態保持の仕様**（PR #241、developマージ済み、[191-gji-state-scope-spec.md](191-gji-state-scope-spec.md)）: 開閉・変換モードの保持単位は
  **TSFスレッド**（結論1: hwndごとの記憶は不要で、むしろ誤りになる。同じスレッドの別HWNDは状態を共有し、**別スレッドの窓は独立する**）。
  新しいスレッド・プロセスは閉×ひらがなから始まる（Q3）。
- **awaseを完全バイパスした注入（A'）でなければIMEの表にならない**（ADR-191決定3・実測2）: A'（完全バイパス）98.5%に対しB（awase経由、
  `AWASE_TEST_INJECTION=1`でawase自身のフックに物理キー扱いさせ、awaseの通常のactuationパイプラインを通す条件）は84.5%で、外れ31件は
  すべて**awase自身の書き込み規則**だった。既存の`Runtime::begin_calibration_bypass`/`end_calibration_bypass`（ADR-176 176-T6）は、
  較正窓（現状は`awase-settings.exe`固定）に対してこのA'相当（自己操作0、実測済み）を、awase.exeを終了させずに達成する既存の窓単位
  バイパス機構である。
- **実運用のタイミング問題**: 打鍵時予測の実装過程で、BUG-153〜159（`docs/known-bugs/`、観測とactuationのタイミングに関する一般的な
  備え〈fence、通過マーク相当〉が要ることを示した一群）を発見・修正した。BUG-160（親指モードキー×Shiftのpassthrough優先順位の欠落）は
  同じ番号帯だが、内容は打鍵時タイミングではなくキー選択の優先順位の話で、本ADRの学習セッションのタイミング問題とは別系統である。
- **既存の設定読み取り経路は3つ、いずれも部分的**（round1指摘B-1で洗い出し、round2で全項目を develop で裏取り済み）:
  1. `gji_charset_autodetect.rs`（544行、ADR-191後も残存。ADR-191が撤去したのは「読んだ値でbeliefを上書きする」配線であって、読み取り自体では
     ない）: `read_config1_db()`（生バイト列）・`config1_db_stamp()`（mtime+len、キャッシュ無効化用）・`read_key_effect_keymap()`
     （バイト列→`key_effect_predictor.rs::KeyEffectKeymap::from_config`への橋渡し）。`from_config`は`session_keymap`の値でATOK・MS-IMEの
     **内蔵プリセット**を選ぶだけで、`custom_keymap_table`の中身そのものは効果表に反映していない（CUSTOM/MOBILEは`None`＝予測なしに縮退）。
  2. `crates/awase-gji-config::keymap`: `extract_ime_keys`/`extract_mode_keys`（`custom_keymap_table`のTSVを解析し、キーごとにON/OFF/トグルや
     `SetMode`/`ToggleAlphanumericMode`/`ToggleKanaType`/`Other`を分類）——**`custom_keymap_table`の中身を実際に読んで分類する唯一の経路**。
     [ADR-192](192-state-dependent-mode-key-warning-and-guided-override.md)決定1が、同じ`extract_mode_keys`ベースの検出を「状態依存キーの
     警告」目的で既に定義している（Mozc公開キーマップTSVの展開も含む、より広い検出）。
  3. `msime_key_assignment.rs`: レジストリからMicrosoft IME本体のキー再割り当てを検出する（GJIとは無関係、単純な「割り当て変更の有無」）。

本ADRは、この学習を「CI実機の格子」から「ユーザーの実機」へ、独立プロセスとして持ち出すための段階的な導入計画を定める。

## 目的

カスタムキーマップ（GJIの`custom_keymap_table`、MS-IMEのキー割り当て変更、将来的な他IME）に対応する。ADR-191の3段階ラウンド
（設定の読み取り→学習→検証）を、ユーザーの実機で実行できる形にする。**本ADRは予測（`KeyEffectPredicted`）の対応範囲拡大のみを扱い、
awaseが実際に書き込んでよいキーの集合（actuation許可リスト）は一切変更しない**（次節「決定」の前提）。

## 非目的（このADRで扱わないこと）

- **actuation許可リストの拡張**: ADR-189の固定セットとユーザー明示config以外に、awaseが実際にキーを送信してよい対象を増やすことは
  本ADRでは行わない。学習結果は予測にのみ使う。許可リストを増やす提案は、必要になった時点で別ADRとする。
- **ADR-194（シミュレーション・リプレイのハーネス）**: 試作はOpusレビューで「既存の単体テストでは捕まえられなかったバグの実証」に失敗し、
  ユーザー判断で破棄した（ブランチ`feat/ime-sim-harness`のみに存在し、developには未マージ・未存在）。
- **[ADR-192](192-state-dependent-mode-key-warning-and-guided-override.md)（状態依存キーの検出・警告UI）**: 検出ロジック（決定1）は本ADRの
  段階0が呼び出して再利用する（下記）。警告文言・置き換えの案内UIの詳細はADR-192側が引き続き所有する。
- **他IME（ATOK単体アプリ、Google日本語入力以外の変換エンジン）への一般化**: 対象はGJI（ATOK/MS-IMEプリセット/カスタム）とMicrosoft IME本体に限る。

## 決定

### 段階0: 設定の読み取りラウンド（既存3経路の統合。ADR-192決定1の検出を再利用）

**round1指摘B-1・round2確認**: develop に既に3つの部分的な読み取り経路がある（背景節）。段階0は**新規構築ではなく統合**であり、
見積もりは薄い集約関数（100行未満）+`extract_mode_keys`への小拡張（50行未満）。

1. **統合**: 経路1（`from_config`のプリセット選択）・経路2（`extract_ime_keys`/`extract_mode_keys`、`custom_keymap_table`実測）・経路3
   （`msime_key_assignment.rs`、MS-IME本体のレジストリ再割り当て）の出力を、キー単位の1つの構造体（初期仮説表S）にまとめる。
2. **ADR-192決定1との所有権（round1 M-6への対応）**: 「キーマップから状態依存キーを機械的に検出する」ロジックそのものは
   **ADR-192決定1が所有・実装する**（`extract_mode_keys`ベース、Mozc公開キーマップTSVの展開も含む、より広い検出）。本ADRの段階0は、
   ADR-192決定1が実装した検出関数の出力を**消費するだけ**で、独自に検出ロジックを再実装しない。ADR-192が本ADRより先に実装されていなければ、
   段階0は`extract_mode_keys`を直接呼ぶ暫定実装とし、ADR-192実装後にその出力へ差し替える（どちらが先に実装されても、検出ロジックの
   実装箇所は1つに保つ）。
3. **状態別束縛の小拡張**: `extract_mode_keys`は現状、キーに割り当てられた`GjiModeCommand`の**種別**だけを見て、どのstatus行
   （`Precomposition`/`Composition`/`Conversion`/`DirectInput`）に束縛されているかは捨てている。しかし`SetMode`（絶対設定系）の
   コマンドがComposition/Conversion（入力中）のstatusに束縛されていれば、**それは押した結果の変換モードそのもの**であり、
   コマンド名だけから初期仮説の予測値を直接組み立てられる（学習を省略できる）。相対トグル系（`ToggleAlphanumericMode`/
   `ToggleKanaType`）は現在のモード依存で一意に定まらないため、初期仮説は「不明、要学習」に留める。この区別に
   **書き込み可否の意味は無い**（前節の通り、本ADRは予測しか扱わない）——単に「学習を省略できる確信度」の話である
   （`KeymapRow`が既に`status`を持つため、新しい解析は不要。見積もり: 50行未満+テスト）。
4. **Microsoft IME本体には経路2に相当するものが無い**（レジストリはキー再割り当ての有無だけ）。したがってMicrosoft IME本体では
   初期仮説Sは常に空（=全キー要学習）であり、段階1（学習）を省略できない。
5. **既存資産の再利用**: 上記以外の新しい検出機構は作らない。charset自動検出や設定への書き戻しは作らない（ADR-191決定5「削る・
   見送るもの」を踏襲）。
6. 段階0の出力は、段階1（学習）が測定すべきキーの集合（初期仮説が「不明」に留まったキー）と、初期仮説の表S。

### 段階1: 独立プロセスでの学習ラウンド（rev2からの最大の変更点）

**round2 N-1/N-2が指摘した構造的矛盾（awaseの単一スレッド非同期ランタイムに学習ループを埋め込む設計は、A'バイパス要件とTSFスレッド単位の
状態保持のどちらとも両立しない）を受けて、学習を`awase.exe`とは別の独立した短命プロセスとして実装し直す**。

- **プロセス構成**: 学習は、既存の`crates/awase-windows/examples/ime_key_matrix_spike.rs`（CI格子ツール）の系譜を継ぐ、独立したバイナリ
  （`awase-keymap-learn`クレート配下の新しい実行可能ターゲット、例: `crates/awase-keymap-learn/src/bin/learn.rs`、または
  `awase-windows`側の新しい`examples/`エントリ）として実装する。プロセス内は**単一の同期メインループ**（`awase-keymap-learn::Executor`
  をそのまま、シミュレータと同じ同期呼び出しで動かす。`spawn_local`・非同期executorは一切要らない）。**これがN-2を構造的に解消する
  理由**: 注入（`SendInput`）・観測（IMM/TSF読み取り）・専用窓の所有をすべて**この1プロセスの1スレッド**に置くため、
  「別スレッドのTSF状態を読む」という取り違えがそもそも発生しない（`191-gji-state-scope-spec.md`の「保持単位はスレッド」を、
  複数スレッドを跨がない設計で満たす）。
- **`ImeDriver`トレイト（round2 N-3対応込み）**: `Executor`が`SimIme`に要求している`read_status`/`reread_status`/`press`/`reset`の4メソッドを
  `pub trait ImeDriver`として切り出し、`Executor<D: ImeDriver>`にジェネリック化する。**round2 N-3の指摘どおり**、`Executor::new`が
  `sim.machine().initial_status()`（真のモデル、実機ドライバには存在しない）を呼んでいる箇所は、`ImeDriver::read_status()`から初期状態を
  取る形にシグネチャを変える（「既存テスト無変更」は主張しない。既存の`SimIme`ベースのテストは呼び出し側を1行直すだけで、期待値自体は
  変わらない）。**時間会計（round2 N-4対応）**: `ImeDriver`に`elapsed_ms(&self) -> f64`を追加し、`SimIme`は従来どおり仮想`CostModel`の値を
  返す（シミュレータ比較実験に影響しない）。実機ドライバは`Instant::now()`起点の実測msを返す。`Executor`の予算判定
  （`strategy::run`の`over()`）はこの値で行うため、B-5の20分/40分基準は**実時間**で判定できる。
  見積もり: トレイト切り出し+シグネチャ調整120行未満（awase-keymap-learn、既存テストは呼び出し1行修正）。
- **実機ドライバ（`SendInput`直叩き、awaseの通常actuation経路は一切通さない。round2 N-1対応）**: 実機ドライバの`press`は、
  awaseの`output/vk_send.rs`等の通常actuationパイプライン（`DeferGate`・belief更新を伴う）を**一切通さず**、生の`SendInput`を叩く
  薄い注入関数だけを使う（ADR-191決定3が要求するA'そのもの。「学習専用の新しい注入経路は作らない」というrev2の方針は撤回する——
  A'を満たすには、それ以外に道が無い）。観測（IMM/TSF読み取り、TSFスレッドcompartment通知の購読）も同一プロセス・同一スレッドで行う
  （`191-gji-state-scope-spec.md`の手法T、素のWin32 EDIT窓で`CoCreateInstance(CLSID_TF_ThreadMgr)+Activate()`を踏襲）。
- **`awase.exe`との共存（A'の確保）**: 学習プロセスの窓へ向かう`SendInput`注入キーは、`awase.exe`が並行して動いていると、
  そのシステム全体のLLフックに（対象窓のプロセスに関係なく）見られてしまう。A'（自己操作0の実測条件）を確保する手段は次の
  いずれかとし、**どちらを採るかは実装時にA'相当が実測で確認できるかで決める**（`191-calibration-experiments.md`と同じ
  「まず実測、そのうえで採否を決める」手法を踏襲。事前にコミットしない）:
  1. **単純案**: 学習セッション中は`awase.exe`を終了しておく（ADR-191決定3のA'実測そのものと同一条件——CIの格子学習はそもそも
     `awase.exe`を起動していない）。ウィザード（段階6）が学習プロセスの起動前に`awase.exe`の終了を確認する。
  2. **拡張案**: 既存の`Runtime::begin_calibration_bypass`（ADR-176 176-T6、較正窓に対してA'相当・自己操作0を実測済み）を、
     対象プロセス名を`"awase-settings.exe"`固定から引数化し、学習プロセスの窓にも適用できるよう小さく一般化する。
     `awase.exe`を終了せずに済む代わりに、この一般化そのものにA'相当（自己操作0）の実機再測定が要る。
- **異常への対処**: `awase-keymap-learn::anomaly`の分類（キー未着・観測経路の不一致・想定外の状態・リセット失敗）とリセット段階
  （Soft/Mode/Hard）を実機ドライバに接続する。BUG-153〜159の教訓（観測の時間切れを異常の証拠に数えない、通過マーク相当の仕組みが要る）を、
  ドライバ層の設計に反映する。
- **出力**: 観測表（決定的/履歴依存/非決定/矛盾のセル分類）。段階0の初期仮説Sと食い違うセルは、設定の読み取り側の直すべき箇所として
  記録する。**「誤りに強い分類」（少数派が一定数以上のときだけ非決定と宣言する等）は、`awase-keymap-learn`の`README.md`に未実装と
  明記された前提機能であり、round1 M-8の指摘どおり本ADRの実装着手前提として明示する**——これが無いままだと、単発の観測誤り（BUG-153系の
  タイミング問題起因を含む）を非決定セルと誤判定し、予測なしに縮退するセルが不必要に増える。

### 段階2: 自己検証（独立ランダムウォークでの信頼度算出、実時間）

学習した表を、学習に使っていない独立のキー列（ランダムウォーク）で採点し、一段予測の正答率と信頼度を算出する。CI実機の格子で確立した手法
（`score_walk.py`相当）を、段階1と同じ独立プロセス・実時間の枠組みへ移植する。**ATOKは99.6%（[191-calibration-experiments.md](191-calibration-experiments.md)、
一致278・不一致1・表に無い19）の実測があるが、Microsoft IME本体は独立walkでの採点自体が未実施であり「未確認」のまま**。信頼度が低い
（段階1の「誤りに強い分類」で弾かれる）場合は、学習をやり直すか、当該セルを予測しない。

### 段階3: 学習結果の永続化

[ADR-176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)決定6（176-T11）が定めた`config.toml`の`[[calibration]]`スキーマ（1件=1モードキーの較正結果）は
「1キー1件」の粒度で、本ADRの表は`(状態, キー)`→効果のセル単位なので、**粒度が違う**。`[[calibration]]`を無理に拡張せず、**別ファイル
（例: `<config dir>/keymap-learn-table.json`のような、表全体を1ファイルに持つ独立フォーマット）を新設する**方針を第一候補とする
（`[[calibration]]`はADR-176が定めるユーザー向け・人間可読な少数キーの上書き設定のままにする）。

### 段階4: 予測器の実行時読込（予測にのみ使う、actuationには使わない）

`state/key_effect_predictor.rs`の予測器を、コンパイル時埋め込みの`key_effect_table.rs`から、実行時に読み込んだ学習結果（段階3の
永続化データ）を使う形に変える。**同梱の3種の表は未学習時の既定値として残す**（ADR-191決定3「表が空にならない」の踏襲）。
学習済みの表があれば、それを優先する。**確認**: ここで読み込んだ表は`KeyEffectPredicted`（belief更新）にのみ使い、awase自身が
IMEへキーを送るactuationの判定（ADR-189の固定セット、ユーザー明示config）には一切使わない——本ADRのスコープを超える変更をしない。

ADR-176のIPC（`calibration_ipc.rs`の`pack`/`pack_result`）はペイロードが1ワード（`usize`）固定で、数十〜数百セルの表本体を運ぶ設計には
なっていない。したがって段階4の「実行時読込」はIPC経由ではなく、**段階3の永続化ファイルを直接読む**（`KeymapCache`と同様のfs読み取り+
スタンプ比較キャッシュ）方式に限定する。既存のIPCは「学習セッションの開始/終了/成否」という制御信号のみを運ぶ。

**破損ファイルへの縮退（round1 M-10後半、未対応分）**: 永続化ファイルのスキーマ検証に失敗した場合は同梱の既定表へフォールバックする、
未知のセル・パースできない行は「予測なし」に読み替える、ファイルサイズに上限を設ける——`KeymapCache`が同種の既定値フォールバックを
既に持つ設計（`from_config`のCUSTOM/MOBILE→`None`）に揃える。

### 段階5: 隠れ状態を学習した最小のMealy機械として持つ

現状の`state/key_effect_predictor.rs`は、入力中の段階（なし/入力中/変換中〈Space・変換キー・無変換で入る〉）を、固定の`Stage`/`Conv`型と、
打鍵履歴からの追跡規則（`KeyTrack`、`next_stage()`）で表現している（ADR-191決定3「実装での単純化」、暫定と明記。round2 R-1が指摘した
「`key_effect_table.rs`」表記の誤りをここで訂正済み——`Stage`/`Conv`/`KeyTrack`/`next_stage()`はすべて`key_effect_predictor.rs`にある）。
本段階は、この固定の名前・規則を、学習結果から作る**最小のMealy機械**に置き換える。

**状態併合アルゴリズム（round1 M-7への対応）**: 巡回学習（段階1）は既に全キー×全到達可能状態を訪問しているため、L\*のような能動的な
クエリ生成は不要（背景節で不採用と明記済み）。代わりに、有限個の識別プローブ（Esc/Enter/BS/Space）への応答ベクトル（`(押下後の
open/mode/composing相当, Disposition)`の組）が完全一致する状態どうしを同一クラスへ併合する、単純な等価類分割（観測表ベースの
Moore/Mealy機械最小化の標準手法）で足りる。応答ベクトルが1件でも異なれば別クラスとする（過剰併合よりは状態数が多く残る方を安全側とする）。
**後方互換**: 同梱の初期仮説表（段階4）は、固定の`Stage`/`Conv`のまま残してよい（学習していないユーザーへの既定値としての役割のみ）。

### 段階6: ADR-176較正ウィザードとの統合（子プロセス起動+IPC）

ADR-191は`apply_calibrated_mode_keys`設定を撤去済み（`src/config.rs`に撤去確認テストあり）——「較正結果を適用する」という独立した
動作モードはもう存在しない。本段階の統合は、「較正結果の適用」ではなく、**ADR-176の較正ウィザード（awase-settings、UI導線）から、
段階1の独立学習プロセスを子プロセスとして起動できるようにする**ことだけを指す。awase-settingsは学習プロセスの標準出力またはIPC
（既存の`calibration_ipc.rs`のパターンを参考に、進捗（現在何セル目/推定残り時間）と成否だけを運ぶ、表本体は運ばない）で進捗を受け取り
UI表示する。学習が完了すると段階3の永続化ファイルが更新され、段階4の実行時読込（次回のfsスタンプ再チェック時、または完了通知を
トリガにした即時再読込）で反映される——「適用」という別のユーザー操作は要らない。

### 段階7: 安全対策

- **専用窓への注入に限る**（他アプリへは送らない。段階1のA'確保策と同じ窓を使う）。
- **ユーザー入力の混入検出**: 学習プロセスが自分の注入以外のキー・フォーカス変更を検出したら当該試行を無効化する。
- **他アプリへの副作用を作らない**: TsfNativeアプリへ生キーが届く経路が無いので、BUG-113/124の「@」の機序は成立しない。管理者権限の窓は
  対象外。短時間の大量注入がセキュリティソフトに検知されない範囲に押下数を抑える（S6+イベント待ちで既に大幅な削減がある）。
- **永続化ファイルの検証**（段階4参照）。

### 段階8: 陳腐化検出

キーマップ（`config1.db`、レジストリのキー割り当て）が変わったら、学習済みの表を失効させ、段階4は既定値へフォールバックする。
**フィンガープリントの粒度（round1 m-4への対応）**: 段階3の永続化が「1キー1件」ではなく表全体の1ファイルであるため、ADR-176 176-T12の
`relevant_rows_for_vk`（1VKごとの部分文字列）ではなく、`config1_db_stamp()`（`config1.db`全体のmtime+len）または
`session_keymap`/`custom_keymap_table`/`overlay_keymaps`3値のハッシュを、**表ファイル全体の1つのフィンガープリント**として使う。

## 成功基準・中止基準（IME別に分離。実測と未計測部分を明示）

- **学習時間（GJI）**: 段階0の静的仮説（背景節・段階0の項目3）で、`Composition`/`Conversion`に束縛が無い大半のキーは学習を省略できる
  見込みのため、典型的なカスタムキーマップでの段階1所要時間はATOK全数学習（実測17分、`191-calibration-experiments.md:105`のシャード別
  合計）より短くなる想定。段階1+段階2の合計が実用に耐える時間として、暫定を**20分以内**とする。
- **学習時間（Microsoft IME本体）**: 段階0に静的仮説が無い（初期仮説Sが常に空）ため、段階1は実測17分相当の全数学習が確定的に発生し、
  これに段階2（**未計測**）が加わる。**したがって20分基準はMicrosoft IME本体には適用しない**——実機プロトタイプで段階1・段階2を
  分けて実測してから、Microsoft IME本体専用の基準を別途定める（暫定は置かない。round2 R-3が指摘した「基準が自分の本文で反例を
  持つ」状態を、基準そのものを分けることで解消する）。
- **予測精度**: 段階2の自己検証で、一段予測の正答率が、コンパイル時埋め込みの表の実測水準（ATOK 99.6%。MS-IME本体は未確認、上記参照）を
  大きく下回らないこと（暫定: 95%以上）。
- **中止基準**: 段階1の実機プロトタイプで、異常（キー未着・観測不一致）の発生率が、シミュレータの想定（キー欠落2%・観測誤り3%）を
  大幅に超える（暫定: 10%超）場合、実機ドライバの設計を見直す。GJIで段階1+段階2の合計時間が実用に耐えない（暫定: 40分超）場合、
  S6以外の戦略（貪欲、rural CPP）や、段階0のショートカットが実際に削れるセル数（ATOK 132セルのうち`Stage == None`のセル数として
  机上で先に見積もれる）の再検証を行う。
- ADR-191決定5の複雑性の収支（指標1）の対象**外**とする（ADR-191決定5に明記済み）。本ADRでは代わりに、段階ごとに触る既存モジュールの
  変更行数を分けて数える（次節「単一ディレクトリに閉じる、の実際の範囲（round2 R-4）」参照）。

## 単一ディレクトリに閉じる、の実際の範囲（round2 R-4への対応）

rev2までは「較正基盤は単一のディレクトリに閉じる」（ADR-191決定5）を掲げつつ、実際には段階1が`win32-worker`・`output/vk_send.rs`・
`lints/actuation_call_guard`まで触る設計だった。**独立プロセス化（本rev3）により、段階1の実装物のほぼ全てが新しい独立バイナリ
（`awase-keymap-learn`クレート配下）に閉じ、`awase-windows`の既存runtimeモジュールに触らない**（`output/vk_send.rs`・`win32-worker`・
`actuation_call_guard`のいずれも不要になった）。段階ごとに残る「既存モジュールへの接触」を明示する:

| 段階 | 触る既存モジュール | 見積もり |
|---|---|---|
| 0 | `crates/awase-gji-config/src/keymap.rs`（`extract_mode_keys`拡張） | 50行未満+テスト |
| 1 | `crates/awase-keymap-learn`（トレイト化）、`Runtime::begin_calibration_bypass`（拡張案を採る場合のみ、対象プロセス名の引数化） | 120行未満+（拡張案時のみ）30行程度 |
| 4 | `state/key_effect_predictor.rs`（実行時読込の入口、1,198行のうち追加分） | 未見積もり（実装時に確定） |
| 6 | `calibration_ipc.rs`、`crates/awase-settings/`（子プロセス起動+進捗表示） | 未見積もり（実装時に確定） |

`awase-keymap-learn`本体（現状+3,406行、develop未マージ）は、本ADRの段階1（独立プロセスとしての実装）と**同時に**developへマージする
（このタイミングなら、実際に消費者が使う形で入るため、「消費者の無い先行実装」というADR-158/162が問題視する非対称にはならない）。

## layer-boundaries / ime-belief-architectureとの整合

- **コアは引き続きOS非依存**（[docs/layer-boundaries.md](../layer-boundaries.md)、ADR-019）。`awase-keymap-learn`もOS非依存の純粋Rustクレートで、
  VKコードを持たない（`KeyId`は抽象ID）。段階1の独立プロセス（`awase-windows`側にドライバを持つ）が、`KeyId`⇔実VKの対応とWin32 APIを持つ。
- **belief書き込みは`reduce()`経由のみ**（`.claude/rules/ime-belief-architecture.md`）。学習セッションは`awase.exe`とは別プロセスで完結し、
  `awase.exe`のbeliefパイプラインには一切触れない（段階1のA'確保策そのものが、この境界を物理的に保証する）。学習結果（段階3の
  永続化データ）を段階4で予測器が読み込む経路は、既存の`KeyEffectPredicted`の生成元をコンパイル時データから実行時データへ差し替えるだけで、
  belief書き込みの規約自体は変わらない。
- **actuation合流点への影響ゼロ**（`.claude/rules/fix-requires-evidence.md`「IME actuation合流点」表）: 本ADRはactuation許可リストを
  変更しないため、`ime_controller.rs::apply`・`runtime/open_chain.rs`等の合流点を一切変更しない。
- **[ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md)（型化/dylint境界）との関係**: 学習結果の永続化データの型が、既存の
  `ImeEvent`/`ObservationSource`の型化方針と衝突しないか、実装時に確認する。

## 却下した設計（rev2からの変更点）

- **却下**: rev2の段階1a（学習セルを`composing`時の`mode`/`composing`変化で「書き込み対象から除外」するゲート）。develop同梱表を
  機械的に数えたところ、ATOK 29%・GJIのMS-IMEプリセット32%・Microsoft IME本体33%（`mode`変化のみ）〜43%（`Disposition`込み）のセルが
  除外され、その中にADR-189の固定セット（`Kanji`/`HankakuZenkaku`）と分類(a)の`ImeOn`/`ImeOff`まで含まれていた（round2 R-2の実測）。
  根本原因は「予測を落とすゲート」と「actuationへの昇格を止めるゲート」を混同したことで、本ADRは前節の通り予測しか扱わないため
  ゲート自体が不要と判明した（round1が推奨していた「分類a/bへの昇格は別ADRの範囲」と同じ結論に、遠回りして到達した）。
- **却下**: 学習をawase.exeの`spawn_local`ランタイムに統合する設計（rev2の段階1）。ADR-191決定3のA'実測要件と
  `191-gji-state-scope-spec.md`のTSFスレッド単位の状態保持のどちらとも構造的に矛盾する（round2 N-1/N-2）。独立プロセス化がこれを
  構造的に解消する。
- **却下した代替案（rev1から継続）**: 「学習を一切せず、ADR-192の警告UIだけでカスタムキーマップのユーザーに冪等キーへの変更を促す」——
  ユーザー指摘に反する。「`awase-keymap-learn`のロジックを`awase-windows`側に直接書く（クレートを分けない）」——OS非依存のテスト
  容易性を失う。「段階4の実行時読込もADR-176のIPCで表本体を送る」——1ワード固定ペイロードでは表本体が入らない。

## リスク

- **リスク**: 独立プロセスの`awase.exe`との共存（A'確保策1・2のいずれか）が、実機で自己操作0を再現しない可能性。段階1の実機
  プロトタイプで、まず単純案（`awase.exe`終了）から検証し、A'が確認できてから拡張案（バイパス一般化）を検討する。
- **リスク**: 実機での学習セッションの異常処理が、シミュレータの想定を超える複雑さになる（GJIとMicrosoft IME本体で、異常の現れ方が
  違う可能性）。段階1を小さいプロトタイプ（GJI・ATOKプリセットのみ、少数キー）から始め、異常率を実測してから対象を広げる。
- **リスク**: ADR-176の`[[calibration]]`スキーマ（1キー1件）と、本ADRの表（セル単位）の粒度の違いが、永続化データの形式を複雑にする。
  段階3で別ファイルに分離する方針を第一候補とし、実装前に確定する。
- **リスク**: 「誤りに強い分類」（段階1の前提、`awase-keymap-learn`未実装）が無いまま実装に進むと、単発の観測誤りを非決定と誤判定し、
  予測なしに縮退するセルが不必要に増える。段階1着手前にこの分類ロジックを実装することを、本ADRの前提条件とする。

## 検証計画

- 段階0〜2を、GJI・ATOKプリセットの小さいプロトタイプ（既知のCI実機の格子と同じキー集合）で実装し、CI実機の格子学習の結果と
  一致するかを確認する（独立プロセスの正しさの検証。特にA'確保策が自己操作0を再現するかをここで実測する）。
- 段階1の実機プロトタイプで、学習時間・異常率・段階2単独の所要時間を分けて実測し、GJI/Microsoft IME本体それぞれの成功基準・
  中止基準を実測値で更新する。
- カスタムキーマップ（`custom_keymap_table`に変換・無変換以外の割り当てがある実機）で、段階0〜4を通しで実行し、予測がATOK/MS-IME
  プリセットの既定表と違う結果になることを確認する。
- **回帰テストの置き場所**（`.claude/rules/fix-requires-evidence.md`のテスト置き場所規約に沿う）: `awase-keymap-learn`クレート自身の
  `cargo test`（`ImeDriver`トレイト・`Executor`の予算判定ロジック、Linuxで実行可）、段階0の拡張は
  `crates/awase-gji-config/src/keymap.rs`のユニットテスト、段階4の実行時読込は`ime_key_sequence_golden.rs`型のgoldenへ既定値
  フォールバックのケースを追加、段階8のstale検出は`state/calibrated_mode_key.rs`の既存テストパターンを踏襲する。

## 関連

ADR-019（layer-boundaries、コアのOS非依存）、ADR-090（型化/dylint境界）、ADR-153（明示config、ADR-176の較正結果適用の前身）、
ADR-162（複雑性予算）、ADR-176（較正UI、IPC、`[[calibration]]`永続化、`begin_calibration_bypass`）、ADR-189（awaseが書いてよい
固定セット、予測とactuationの境界線そのもの）、ADR-190（Microsoft IME本体のactuationフォールバック、MSIME_NATIVE表の背景）、
ADR-191（本ADRの前提、実測・3段階ラウンドの初出、A'実測、`apply_calibrated_mode_keys`撤去）、ADR-192（状態依存キーの検出〈決定1を
本ADR段階0が再利用〉・警告、範囲外）、ADR-194（シミュレーション・リプレイのハーネス、`feat/ime-sim-harness`ブランチのみに存在・
develop未マージ、範囲外・参照のみ、破棄の経緯）。
