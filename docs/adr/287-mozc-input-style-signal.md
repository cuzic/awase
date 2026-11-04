---
id: ADR-287
title: |-
  mozc-readable の共有メモリに「ローマ字入力/かな入力」の方式そのものを足し、OS のロックビット頼みを置き換える
summary: |-
  BUG-106（Mozc/GJI系でawaseの送信romajiがJISかな配列として解釈される）は、検知を`GetKeyState(VK_KANA)`というOS
  キーボードのロックビット直読みに頼っている（ADR-118）。これは「かなロック」というOS側のトグル状態であり、Mozc
  自身の`config.preedit_method`（ローマ字入力=ROMAN/かな入力=KANA、要確認・本文参照）という**入力方式そのもの**とは
  別軸で、ドライバ依存の遅延・エコー（BUG-14）を抱える。2026-11-04 11:52 JST、dragonflyg4の実機でWindows
  Terminal×Google日本語入力（Mozc Readable想定）が稼働中、awaseの送信romaji "to"を打った瞬間に`[kana-lock]`警告が
  発火し、直後の出力が「ｶﾗ」（JISかな配列でT→か、O→らに一致）に化けた実例を確認した（BUG-106追補7）。mozc-readable
  は既にIMEの開閉・CompositionMode（文字種）を共有メモリ経由で確実に読めている（ADR-265〜270）が、入力方式
  （ローマ字/かな）の軸は現在の共有メモリ（version 4、112バイト、実際にリリースされた最後の版）には含まれていない。
  決定: mozc-readable フォーク（別リポジトリ、本ADRの実装範囲外）に入力方式フィールドを追加してもらい、awase側は
  既存のバージョンゲート方式（ADR-267/268と同じ、先頭バイトは互換）で読み取る。段階1はObserve専用（journalのみ、
  beliefには入れない）で、OSロックビットとの食い違い・遅延を実機で集めてから、段階2（警告の即時化、または保留/
  送信停止）を別ADRで検討する。**2026-11-04 追記:** mozc-readable フォーク側の実装（[cuzic/mozc-readable](https://github.com/cuzic/mozc-readable)
  `exp/key-input-style`、`6afb89f29`、push済み・未ビルド・未マージ）で、反転のトリガーをソースから直接特定した
  ——Mozc自身の`prefer_kana_input`を反転させているのはADR-118が読む`VK_KANA`ではなく**別のVK**(`VK_DBE_ROMAN`/
  `VK_DBE_NOROMAN`)で、Mozcのソース自身のコメントが「スレッド/プロセス単位ではなくシステム/セッション全体の
  トグルで、他プロセスの打鍵でも変わりうる」と明記している（`use_keyboard_to_change_preedit_method`が有効な
  とき）。ADR-118のOSロックビットとこの軸は別物であり、一致するとは限らない（BUG-106追補8）。**2026-11-04 追記2:**
  mozc-readable側はversion 4が実際にリリースされた最後の版（どのGitHub Releaseにも5/6/7は出ていなかった）だと
  分かったため、最初のリリース前にversion 5/6/7を1つの**version 5**（160バイト）へ統合し、状態（pulseする
  `prefer_kana_input`等）を先・診断専用（pulseしないフォーカス時刻・キー数）を後に並べ直した（PR #16、
  `73e59c32d`）。以降、本ADRで「version 7」と書いていた箇所は「version 5」を指す。
status: |-
  起草（2026-11-04）。mozc-readable フォーク側は実装済み、readable ブランチへマージ済み(PR #13〜#16、version
  5/6/7の起案から最初のリリース前の単一version 5への統合まで)、ビルドCI実行中(cuzic/mozc-readable の
  readable-build、未完了)。awase側は段階1(Observe専用)まで実装済み(`mozc_shm.rs`の parse・`observer`/`tsf`の
  配線・`[mozc-input-style]`ログ、OSのかな入力ロックビットと並べて記録。オフセット・version番号は統合後の
  version 5に追随済み)。実機ログでの一致/食い違いの収集はmozc-readableのビルド完了後。段階2（belief/actuation
  への配線）は未決定。
related_adr:
  - "ADR-118"
  - "ADR-265"
  - "ADR-267"
  - "ADR-268"
---

# ADR-287: mozc-readable に入力方式（ローマ字/かな）の軸を足す

## 背景

[BUG-106](../known-bugs/BUG-106.md)・[BUG-008](../known-bugs/BUG-008.md)・[BUG-061](../known-bugs/BUG-061.md)は、
IME側で「ローマ字入力」から「JISかな直接入力」に入力方式そのものが反転し、awaseが送った romaji 相当の VK 列が
JIS かな配列として誤解釈される症状である。[ADR-118](118-teams-kana-lock-detection.md)で検知機構（`KanaLockHysteresis`、
`observer/kana_lock.rs`）を実装したが、これは`GetKeyState(VK_KANA)&1`という**OSのキーボードロックビット**を直読みする
もので、入力方式そのものを保持しているIME（Mozc/MS-IME）の内部状態とは別物である。ADR-118自身が「Google日本語入力は
今回の検知の対象外（実機テストでは言語バーからの手動切替自体が機能しなかった）」と明記している。

2026-11-04 11:52 JST、dragonflyg4の実機（Windows Terminal、`ime=GJI`のログラベルだがMozc Readable運用中）で、
ユーザーが「そのとおりですね」と入力した際に次のログが記録された（UTC、`awase.log`より抜粋）:

```
02:52:11.037 WARN [kana-lock] OS かな入力ロックを検知しました (reading=On streak=On(3) fg_class=CASCADIA_HOSTING_WINDOW_CLASS)
02:52:11.039 INFO [key-output] KeyInput(batched): romaji="ha" ime=GJI
02:52:11.292 INFO [key-output] KeyInput(batched): romaji="so" ime=GJI
02:52:11.387 INFO [key-output] KeyInput(batched): romaji="no" ime=GJI
02:52:12.212 INFO [key-output] KeyInput(batched): romaji="to" ime=GJI
02:52:12.341 INFO [key-output] KeyInput(batched): romaji="o"  ime=GJI
02:52:12.629 INFO [key-output] KeyInput(batched): romaji="ri" ime=GJI
...(de/su/ne/. と続く)
```

実際の出力は「そのｶﾗおりですね」——「と」(romaji "to") だけが「ｶﾗ」に化けた。JISかな配列ではTキー=「か」、
Oキー=「ら」であり、半角表記すればちょうど「ｶﾗ」になる。送信した romaji 列自体は正しく、入力方式がその一瞬だけ
JISかな直接入力になっていたと綴りレベルで完全に一致する。

同じログの中で`[kana-lock]`警告は3時間強のセッション中に何度も（00:41, 02:19, 02:22, 02:25, 02:52 UTC...）
`fg_class=CASCADIA_HOSTING_WINDOW_CLASS`で発火しており、単発ではなく繰り返し発生している。本ADR起票当初は
「何が`VK_KANA`トグルを反転させているかは未特定」としていたが、**mozc-readable側の実装時にソースを読んで
訂正した**（下記「現状の〜」節、BUG-106追補8）: `VK_KANA`自体はMozcの入力方式の反転とは無関係で、Mozc自身は
別のVK（`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`）で反転する。したがって「`[kana-lock]`警告(`VK_KANA`)が今回の実際の
反転原因だった」という前提自体を本ADRは証明していない——両者が同時に起きたことの確認でしかない。何が
`VK_KANA`トグル・`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`のどちらを実際に押し下げたか（ドライバ・他プロセス等）は
依然未特定（BUG-008/BUG-061と同じ「原因未確定、awase側に復旧手段なし」の系統）。

## 現状の mozc-readable 共有メモリでは検知できない理由

[ADR-265](265-mozc-readable-integration.md)〜[ADR-270](270-mozc-first-send-composition-check.md)で整備した
`TipSharedState`（`crates/awase-ime-core/src/state/mozc_shm.rs`。ADR起票時点の実際にリリースされた最後の版は
version 4、112バイト）が持つのは:

- `effective_open`/`tsf_open`: IMEの開閉
- `effective_mode`/`tsf_mode`: `CompositionMode`（ひらがな/全角カタカナ/半角カタカナ/半角英数/全角英数/直接入力—
  **文字種**の軸）
- version 3以降: `phase`・`candidate_category`・`preedit_*`・`key_seq`・`scope_forced_mode`・`server_ready`等
  （組成・確認用）

いずれも「ローマ字入力かJISかな直接入力か」という**入力方式**そのものは持たない。**2026-11-04、mozc-readable
フォーク側のソースを実際に読んで確認した**（`src/win32/base/input_state.h::InputBehavior::prefer_kana_input`、
`src/win32/base/config_snapshot.cc::ConfigSnapshot::Get`）: Mozcの設定の`config.proto::preedit_method`
（値`ROMAN`/`KANA`、デフォルト`ROMAN`）を起点に、`TipPrivateContext::EnsureInitialized()`が
`behavior->prefer_kana_input = (config->preedit_method() == Config::KANA)`として一度読み込み、以降は
**`src/win32/base/keyevent_handler.cc::UpdateBehaviorInImeProcessKey`が唯一の反転箇所**で、
`use_keyboard_to_change_preedit_method`（設定、デフォルトオフ）が有効なとき、`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`の
押下のたびに`behavior->prefer_kana_input = !behavior->prefer_kana_input`と反転する。ソース自身のコメントが
「この2つのVKの状態はスレッド単位・プロセス単位ではなくシステム/セッション全体で、別スレッド・別プロセスの
打鍵がいつでもこのグローバルなトグルを変えうる」と明記している——**OSのロックビット(`VK_KANA`)とは完全に
別のメカニズム**であり、`CompositionMode`とは独立した軸である。今回のBUG-106再現はこの軸（または、まだ別に
存在するかもしれないOS側の`VK_KANA`トグル）が反転したことが直接原因と推定されるが、本ADR時点ではどちらの
トグルが実際に動いたかは未確認（上記「背景」節）。したがって、現状の共有メモリをどれだけ読んでも「かな直接
入力に切り替わった」事実そのものは拾えず、ADR-118のOSロックビット頼みから抜け出せていない。

## 決定

1. **mozc-readable フォーク側の変更（本ADRの実装範囲外、別リポジトリ [cuzic/mozc-readable](https://github.com/cuzic/mozc-readable)
   の作業）**: `TipSharedState`に入力方式のフィールドを追加する。既存のバージョンゲート方式
   （version 2→3→4で先頭48バイトの意味を変えずに末尾へ追記してきたのと同じパターン）に従い、追記する。値はMozcの
   設定が持つ生の値をそのまま書く（awase側で意味づけをしない—ADR-265 D3の既存方針どおり）。**実装済み・マージ済み**
   （`exp/key-input-style`、`6afb89f29`、2026-11-04。未ビルド・未リリース）: `prefer_kana_input`・
   `use_romaji_key_to_toggle_input_style`の2フィールド（各u32）を追記。公開元は
   `TipPrivateContext::EnsureInitialized()`（config読み込み時の種まき）と`tip_keyevent_handler.cc::OnKey()`の
   `UpdateBehaviorInImeProcessKey`直後（唯一の反転箇所）の2箇所。`effective_mode`等と同様に状態として扱い、
   変化したら通知イベントをpulseする（他の診断用カウンタはpulseしない点と異なる）。**同日追記**: version 4が
   実際にリリースされた最後の版だと分かったため、最初のリリース前に本フィールドをフォーカス時刻・キー数
   （いずれも本ADR以前から`readable`に未リリースのまま存在）と合わせて**version 5**（160バイト）へ統合し、
   状態を先・診断専用を後に並べ直した（PR #16、`73e59c32d`、`readable/docs/shared-memory.md` 6-4節）。
2. **awase側（本ADRの実装範囲）**: `crates/awase-ime-core/src/state/mozc_shm.rs`に`V5_BLOCK_SIZE`(160)定数と対応する
   `parse`関数を追加し、`RawBlock`/`ShmState`とは別の追記構造体（`CompositionFields`・`FocusTicks`と同じ並び）
   として持つ。既存の受け入れ条件（`classify`）・`ShmUse`の判断には**混ぜない**——version 5が読めない環境
   （mozc-readable未更新・GJI公式版・MS-IME）では単に`None`のまま扱う。**実装済み**（2026-11-04）: `InputStyle`
   構造体（`prefer_kana_input`・`toggle_enabled`、オフセット112〜119）と単体テストを追加、
   `observer/mozc_shm.rs::read_input_style`（既存の`read_key_results`と同じ手順、`version >= 5`のときだけ160
   バイトまで読み直す）、`tsf/mod.rs::read_mozc_input_style`（Windows専用副作用への入口の既存パターン）を配線。
   mozc-readable側の統合（決定1の同日追記）に追随し、オフセット・version番号は統合後の値。
3. **段階1はObserve専用**（ADR-265 D3・ADR-268と同じ慎重さ）: 新フィールドの値は journal にだけ記録し、
   belief・actuation の判断には一切使わない。同時に、既存の`observer/kana_lock.rs`（`GetKeyState(VK_KANA)`）の
   読みも並行して記録し、両者が実機でどれだけ一致/食い違うか（タイミングのずれ、ドライバ由来のエコーで
   OSロックビットだけ動く場面の有無等）を集める。**実装済み**（2026-11-04）: `tsf/warmup/mozc_compose_check_fsm.rs`
   の既存の診断ログ（`[mozc-focus]`・`[mozc-keys]`、ADR-270の確認FSMが打鍵の直後に張る）に`[mozc-input-style]`を
   追加し、同じ1行に`prefer_kana_input`・`toggle_enabled`と、その場で読んだ`observer::kana_lock::read_kana_lock()`
   （`tsf::read_os_kana_lock()`経由）の値を並べて出す。`cargo check --target x86_64-pc-windows-msvc`・
   `cargo test --lib`（1135件）・`architecture_guard`（175件）・`layer_boundary_guard`（17件）は確認済み。
   **mozc-readableが未ビルドのため実機ログでの一致/食い違いの収集はまだ無い**（mozc-readable側のCI完了後）。
4. **段階2（本ADRでは決定しない、観測結果を見て別ADRを起票する）**: 段階1のデータで「mozc-readableの新フィールドが
   OSロックビットより速く・正確に反転を捉えられる」と確認できたら、検知後のアクション
   （即時警告への切り替え、打鍵の一時保留、ADR-270の確認機構との統合等）を検討する。自動復旧（入力方式を
   awase側から書き戻す）はBUG-61/BUG-62追補4で実機上不可能と確定済みのため、本ADRでもスコープ外のまま維持する。

## 適用範囲の限界

- 本ADRが効くのは mozc-readable を使っている窓だけ。公式 Google 日本語入力（mozc-readable 未適用）・MS-IME には
  この入力方式を読む経路が無く、BUG-106/BUG-061の本来の主戦場だったMS-IME（Windows Terminal + MS-IME）には
  効果がない。今回の実機ログは `ime=GJI` だったが、Mozc Readable運用中だったため本ADRの対象に入る——通常のGJI
  （mozc-readable非適用）だった場合はこの経路は使えない。
- 入力方式の反転そのものの原因（何が`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`、あるいはOS側の`VK_KANA`トグルを
  実際に押し下げているか）は本ADRでも未解決。本ADRは「反転を確実に知る」ところまでで、「反転を起こさせない」は
  依然スコープ外。
- 段階1がObserve専用である間は、ユーザーに見える挙動の変化は無い（`.claude/rules/fix-requires-evidence.md`の
  回帰テスト義務は段階2で実際に判断・actuationに使うときに効いてくる）。

## 関連

[BUG-106](../known-bugs/BUG-106.md)（追補7＝本ADRの引用元となった実機ログ、追補8＝mozc-readable実装時に
特定した反転トリガーの訂正）・[ADR-118](118-teams-kana-lock-detection.md)（現行のOSロックビット検知、本ADRが
補完する対象）・[ADR-265](265-mozc-readable-integration.md)〜[ADR-270](270-mozc-first-send-composition-check.md)
（既存の共有メモリ連携、本ADRが踏襲するバージョンゲート方式とObserve専用の段階分け）・
[cuzic/mozc-readable `exp/key-input-style`](https://github.com/cuzic/mozc-readable/tree/exp/key-input-style)
（本ADR決定1の実装、`6afb89f29`）。
