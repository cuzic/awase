---
id: ADR-271
title: |-
  タスクバーの入力インジケーターを読む 2 つの証拠源(字形・右クリックメニュー)の使い方を、設定画面の 4 段階(無効／観測のみ／読めないウィンドウだけ／すべて)で選べるようにする
summary: |-
  ADR-261(字形)と ADR-264(右クリックメニュー)は、観測の無いウィンドウの IME 状態を読む補助の証拠源で、どちらも「既定は無効のオプトイン」と決めてある。
  現状は、メニューは `config.toml` の `[diagnostics] indicator_menu_shadow`(真偽値・設定画面に無い)だけ、字形は設定が無い。
  本 ADR は、証拠源ごとに 1 つの 4 段階の選択肢(`off`/`observe`/`unobserved`/`all`)を置き、設定画面の上級者向けタブから選べるようにする。
  段階は累積する。**belief へ入れる段階(`unobserved`/`all`)の配線は ADR-261 決定 6・ADR-264 決定 5 の入口条件を満たす別 ADR で決める**ので、本 ADR は設定の形・意味・既定・実効値の決め方・画面までを決める。
  それまで第 3・第 4 段階は画面で選べず、設定ファイルの値は**保存を書き換えず、使う時点で観測のみに丸める**。
  `all` は ADR-261 決定 4 の「belief に入れるのは観測の無い窓だけ」を覆すので、その改訂として扱い、入口条件を足す。
status: |-
  起草(2026-10-20)。実装なし。Opus レビュー round 1(Blocker 1・Major 7・minor 9)を反映して書き直し、round 2 待ち。決定は案で、所有者判断を経ていない(Q1〜Q5)。
related_adr:
  - "ADR-148"
  - "ADR-207"
  - "ADR-217"
  - "ADR-233"
  - "ADR-261"
  - "ADR-263"
  - "ADR-264"
---

# ADR-271: 入力インジケーターの証拠源の使い方を 4 段階で選べるようにする

## ステータス

起草(2026-10-20)。実装なし。決定は案。Opus レビュー round 1 を反映(指摘は本文の該当箇所に取り込んだ)。

## 背景

観測の無いウィンドウ(窓内の IMM が読めない。一太郎など、issue #623)の IME 状態を知る補助として、タスクバーの入力インジケーターを読む証拠源が 2 つ起草されている。

| 証拠源 | ADR | 何を読むか | 実装の現状(origin/develop 49667cc3) |
| --- | --- | --- | --- |
| 字形 | [ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md) | インジケーターの小さな画面矩形を取り込み、字形の類似度で状態を引く | **コード無し**(段階 0 の D0-2〈一太郎の実機〉が未測定) |
| 右クリックメニュー | [ADR-264](264-indicator-menu-mode-evidence.md) | 利用者がメニューを開いた瞬間に、現在のモードを UI Automation で 1 回読む | 段階 1(shadow)実装済み・v2.2.0 に同梱。`[diagnostics] indicator_menu_shadow = true` でだけ journal に残す。設定画面には無い |

両 ADR とも「既定は無効のオプトイン」(ADR-261 所有者判断 Q2・ADR-264 決定 6。画面取り込み+低レベルフックはキーロガー+スクリーングラバーに見え、EDR/ウイルス対策の行動検出に当たりうる)と決めてあり、belief への配線(段階 2)は未決である。

所有者の要望(2026-10-20): 両者を設定画面に出し、**①単に無効にする、②観測して不具合報告に添付することだけできる、③IME の状態を見られないウィンドウでだけ有効にする、④すべてのウィンドウで有効にする**、を選べるようにしたい。

[ADR-263](263-episode-start-key-hold-and-language-decision.md)は同じ問題を `mode`×`scope` の 2 軸にし、画面では意味のある組み合わせの 1 つのプルダウンに集約した(`AUTO_DETECT_PRESETS`)。画面の作りはそれに揃える。

## 決定

### D1. 設定の形: 証拠源ごとに 1 つの `EvidenceLevel`

```toml
[indicator_evidence]
glyph = "off"   # "off" | "observe" | "unobserved" | "all"
menu  = "off"
```

- `src/config.rs` に `EvidenceLevel`(`Off`〈既定〉/`Observe`/`Unobserved`/`All`、`serde(rename_all = "snake_case")`、`Ord` 派生)と `IndicatorEvidenceConfig { glyph, menu }` を置く(OS 非依存。ADR-019)。
- 2 軸にしないのは、観測のみに範囲の軸が無く、組み合わせが 4 つで足りるから。
- **証拠源ごとの独立スイッチ**にする。これは ADR-264 決定 6 の「ADR-261 と同じスイッチに揃える案(所有者判断 Q)」を**別スイッチに決める**ことで、ADR-264 決定 6・段階 5 の該当記述を本 ADR で置き換える(Q3)。字形(画面の取り込み)とメニュー(UI Automation)は EDR 上のリスクの質が違い、メニューだけ試したい利用者に字形の取り込みを許すべきでないため。
  ADR-261 Q2 の趣旨(収集と読み取りを別スイッチにしない)は**証拠源ごと**に守る。
- **組み合わせの規則**:
  - `menu ≥ observe` のとき journal に出す「メニュー読み × 字形 × belief の三つ組」(ADR-264 決定 5)の字形は、**`glyph ≥ observe` のときだけ**取り込む。`glyph = off` なら字形欄は空(取り込まない)。字形のオプトアウトを menu が破らない。
  - メニューの読みを字形の表の正解ラベルにするのは、`glyph ≥ observe` **かつ** `menu ≥ observe` のときだけ。
  - どちらかが `off` でも、もう一方は単独で動く。

### D2. 4 段階の意味(累積)

| 段階 | 読む範囲 | journal・不具合報告 | 見本の収集(字形のみ) | belief の証拠にする範囲 |
| --- | --- | --- | --- | --- |
| `off`(既定) | しない(メニューの内容・字形を読まない) | 出さない | しない | しない |
| `observe` | すべてのウィンドウ | する | する(受け身。D2c) | **しない** |
| `unobserved` | すべてのウィンドウ(`observe` と同じ) | する | する | **観測の無いウィンドウだけ** |
| `all` | すべてのウィンドウ | する | する | 読めるウィンドウにも。**利用者の責任** |

- 読む範囲は `observe` 以上で同じ(読める窓での直接の読みとの一致率が `observe` の価値であり、それを `unobserved` で失わない。ADR-261 決定 6・ADR-264 決定 5)。段階で変わるのは belief の証拠にする範囲だけ。
- どの段階でも**IME へは何も書かない**(ADR-261 決定 0・ADR-264 決定 0)。`off` は「メニューの内容・字形を読まない」の意味で、ADR-264 の `MENUSTART`〜`MENUEND` の WinEvent 登録(BUG-202/203 の抑止に要る共通の前提)は `off` でも別の理由で動きうる(画面の文言に「何も監視しない」と書かない)。
- **「観測の無いウィンドウ」の述語**: 実測学習で `Standard` から `Imm32Unavailable` に降格した窓(`focus::is_demoted_by_learning`。ADR-263 の `lang_detect_window_in_scope` の中身のうち降格の事実)。
  ADR-261 決定 4 の述語(この `FocusGen` で `direct-follow` が N 回失敗)とは**別物**であり、どちらを採るかは配線 ADR で決める(未解決)。ADR-263 の `lang_detect_window_in_scope` は使わず(TSF ネイティブ除外と scope を含むため)、`is_demoted_by_learning` と `InputRelay` 除外を直接使う。学習降格は `Standard` からだけ起きるので `InputRelay` 除外は `unobserved` では空振りし、効くのは `all` だけ。
  ADR-263 の TSF ネイティブ窓の除外は**書き込み**(BUG-124 型の誤出力)の理由によるもので、読むだけの証拠源には当てはまらないため継承しない。除外するのは入力中継ツールのウィンドウ(`AppImeProfile::InputRelay`。インジケーターは手元の IME でありリモート側の状態ではない)だけ。
- **belief の証拠にするときの種別**: 新しい `ObservationSource` で `authority() = BeliefOnly`(drift correction を起こさない)、`confidence` は Medium 以下(詳細は配線 ADR)。

#### D2b. `all` は ADR-261 決定 4・ADR-264 決定 2 の改訂である

ADR-261 決定 4 は「段階 2 で belief に入れるのは観測の無い窓だけ。読める窓では直接の読みが優先で、二重に解釈しない」、ADR-264 決定 2 は「読める窓では直接の読みがあるので不要」と決めている。`all` はこれを覆す。
さらに、観測ストアの `most_recent_trusted`(ADR-233)は**新しい Medium が古い High に勝つ**ので、「直接の読み(High)が優先」は自動では成り立たない。字形の遅れ(最大 0.68 秒)で前の状態を映した Medium が、正しい High と `KeyEffectPredicted` を上書きしうる。
読める窓は打鍵量の大半で、`all` の害の面積は `unobserved` より桁で大きい。したがって:

- **この「Medium が High を追い越さない」は `unobserved` を含む第 3 段階以上の入口条件**(D3 末尾)にする。`unobserved` の述語は IMM が読めないことだけを見て他の High 源の有無を見ないので、`unobserved` の窓にも High の観測がある: ADR-268 の `MozcShm`(High・BeliefOnly。一太郎 + mozc-readable で `is_demoted_by_learning` が真のまま High を持つ)、TSF ネイティブのクラス(Windows Terminal・WezTerm 等。TSF 側の観測の信頼度は未確認)。D2b の `all` 固有の条件は便益の場面だけ。ADR-233 の順位キーで、インジケーターの読みが新しい時刻で入っても直接の読み・予測を上書きしないことを golden/journal リプレイで固定する。
- **`all` の便益の場面は未実証**。読める窓には直接の読みが既にあり、`all` が足すのは「直接の読みが外れているとき」だけ。所有者の要望(④)なので枠は残すが、**配線の PR 時点で便益の場面を 1 つ以上示せなければ `all` は画面から外す**(Q2)。外すと決まるまでは `All` を列挙に残す(画面でグレーアウトするだけ。alias は不要)。外すときは variant を消して `#[serde(alias = "all")]` を `Unobserved` に付け、警告は `from_toml_str` が生の表を見て積む既存の作法(`confirm_mode` の旧値の警告と同じ)で出す(variant だけ消すと `"all"` が型エラーで設定全体が `ConfigLoadState::Dangerous` になり、ADR-217 の旧値の受理にも反する)。読み替えた値は保存で書き戻さない。

#### D2c. 字形の見本の収集・永続化(字形のみ)

ADR-261 決定 2 の受け身の収集(読める窓で直接の読みと組にし、見本を別ファイルに永続化)は `observe` 以上で行う。`off` にすると収集を止め、**既存の見本ファイルは残すが使わない**(見本は 2 値化した小画像で個人情報を含まない。ADR-261 決定 7)。
削除は、**本 ADR が ADR-261 段階 3(設定画面の収集状況)に加える要件**の「収集した見本を削除」ボタンで利用者が行う。`off` で見本が残っているときは画面にそう表示する(オプトアウト後に画面由来のデータが残ると後で気づくのを避ける)。`off` への切り替えで勝手に消さないのは、再度有効にしたときに環境指紋が合えば表を再利用できるため。

### D3. 第 3・第 4 段階は、belief への配線 ADR が決まるまで選べない。保存は書き換えない

- `awase-windows-core` に `EvidenceSource::ui_max_level() -> EvidenceLevel`(現状: `menu` = `Observe`、`glyph` = `Off`)を「画面で選べる上限=Windows の殻の実装の到達度」として **`awase-windows-core`** に置く(Windows 固有の事実なのでルートの `awase` コアには置かない。Linux で単体テストでき、`awase-settings` は `awase-windows` の re-export 経由で参照する。`awase` コアには `EvidenceLevel`・`IndicatorEvidenceConfig` の型だけが残る)。配線 PR が上げる唯一の場所にする。
- **実効値は使う時点で決める**(`awase-windows` の殻が `effective = min(configured, ui_max_level())` を起動時に 1 度計算して Runtime に固定する)。`validate()` と設定画面の保存は、**段階の丸めについては値を変えない**(旧キーの読み替えは D4 のとおり読み込み時に行う)。
  理由: 設定画面の保存(`awase-settings/src/main.rs` の三者比較: `base` と `validate()` 後の差分を書く)は、`validate()` が値を丸めると、利用者が無関係な項目を変えて「適用」しただけで `all` が `observe` として恒久的に書かれる(Opus B1)。
  `validate()` は丸めの警告を出さない。警告は起動時のログに 1 回と、画面の「設定値: all(この版では観測のみとして動作)」の表示だけにする(適用のたびの状態行に出すと、直した不具合〈`main.rs` の警告の再表示〉の再発になる)。
- **設定画面**: `ui_max_level()` を超える段階はグレーアウトし、理由(「belief への反映は未対応です」)を出す。ファイルの値が上限を超えているときは「設定値: all(この版では観測のみとして動作)」と表示し、保存しない限り書き換えない。`glyph` の行は `ui_max_level() >= Observe` になるまで出さない(コードが無いものを選ばせない)。
- **反映は再起動後**。起動時の実効値を Runtime に固定し、`WM_RELOAD_CONFIG`(ホットリロード)で来た値は使わない(`off` で起動してワーカーが無いまま `unobserved` を判定に使う半端な状態を作らない)。画面にも「反映は awase の再起動後」と出す。
- `ui_max_level()` を上げる PR は、同じ PR で ADR-261/264 の該当入口条件の充足と、D2b の「Medium が High を追い越さない」ことを示す(`fix-requires-evidence` の IME belief ファミリー。golden/journal リプレイか BUG ファイル)。

### D4. 既定と旧キー

- **既定は両方 `off`**(ADR-261 Q2・ADR-264 決定 6 を維持)。ADR-263 の `auto_detect.mode` が既定 `observe` なのは、打鍵列の統計処理で画面や UI を読まないからで、同列には扱わない。
- **旧キーの読み替えは `AppConfig::from_toml_str`(読み込みの唯一の入口)で行う**。殻の使う時点ではなく読み込み時に行うので、設定画面の `base` にも反映され、画面は実態どおり「記録だけする」を表示する(殻で読み替えると、画面は「使わない」、awase は記録、という最悪の向きの食い違いになる)。読み込み後は `base` も保存対象も `menu = Observe` で差分が無いので、**新キーは書かれない**(三者比較)。旧キーはディスクに残り、毎回の読み込みで読み替えられる(害は無い)。Q4 で旧キーの受理をやめるなら、その前に旧キーが `true` のファイルへ新キーを書き足す移行が要る。
- **`[diagnostics] indicator_menu_shadow`**: v2.2.0 に同梱済み。ADR-217 は「config.rs の旧キーの受理は全て残す」(外部の既存 `config.toml` を守る)と決めているので、**読み替えを残す**: 新キー `indicator_evidence.menu` が未指定で旧キーが `true` なら `menu = "observe"` として扱い、`warn!` で新キーへの移行を促す。新キーがあればそちらが優先。
  フィールドは `#[serde(skip_serializing)]`(画面の全体書き出しが旧キーを書き戻さない)。v2.2.0 の CHANGELOG はこのキーを案内していない(利用者は所有者・CI・#623 の報告者程度)ので、読み替えを省き撤去キーの通知(ADR-207 の `config_load_diag::removed_notice`)だけにする案もある(Q4)。
- `architecture_guard.rs::indicator_menu_shadow_is_observe_only_and_opt_in`(bootstrap の `config.diagnostics.indicator_menu_shadow` の文字列を探す)は、新しい分岐に合わせて更新する。

### D5. 不具合報告への添付

所有者要望②「観測して不具合報告に添付」を満たすため、`observe` 以上の添付を次と定義する(ADR-261 決定 6 の allowlist を正とする):

1. journal の行(`IndicatorMenu`、字形の記録)。ただし journal は添付の可否(`attach_log`)に従い、末尾に切り詰められる。メニューを開く頻度は低いので、行が残っている保証は無い。
2. **ADR-148 型の要約**(診断フィールド、新しい送信内容。`off` でも実効の段階だけは常に出す。旧版〈機能が無い〉との区別のため。報告のスキーマ〈`bug_report.rs` の `SCHEMA_VERSION` と受け側の report worker〉の更新を伴う): `indicator_evidence { glyph: {実効の段階, env_match, hits, misses, reason 別件数}, menu: {実効の段階, 読んだ回数, 同定の成否の件数, 読める窓: 直接の読み(High)との一致/不一致の件数、観測の無い窓: belief との一致/不一致の件数(意味が違うので分けて数える)} }`。件数だけで内容は含まない。
   **設定値ではなく実効の段階**を出す(`config_toml` は `attach_config` が偽なら添付されず、既定値はファイルに書かれないことも多いため、config.toml だけでは「有効だったか」が分からない。丸め後の値とも食い違いうる)。
3. 見本に近くない字形の画像(最大 3 件、ADR-261 決定 6)は別扱い(Q1)。

ADR-261 決定 6 の「`glyph_probe` の要約」は本 D5 の `indicator_evidence.glyph` に当たる(新しい診断フィールドを足すことは両 ADR で変わらない。「新しい送信経路を足さない」とは言わない)。

### D6. 画面(`awase-settings`、上級者向けタブ。ADR-263 の節の下)

- 見出し「入力インジケーターを読む補助(上級者向け)」。冒頭に、**awase が画面の一部(字形)または UI の項目(メニュー)を読むこと**・**既定が無効である理由(ウイルス対策ソフトが行動を検出する場合がある)**・**IME へは何も書かないこと**を平易に書く。
- 証拠源ごとの閉じたコンボボックスで、選択中の段階の説明をホバー(`on_hover_text`)で出すほか、閉じた状態でも 1 行の説明を出す。字形の行は D3 のとおり、コードが入るまで非表示。
- 「右クリックメニュー」の行:

| 段階 | 画面の名前 | 説明 |
| --- | --- | --- |
| `off` | 使わない | メニューの内容を読みません。 |
| `observe` | 記録だけする | 入力インジケーターを右クリックしてメニューを開いたときだけ、現在のモードを記録します。動作は変わりません。不具合報告に件数が含まれます。 |
| `unobserved` | IME の状態が読めないウィンドウで使う | 準備中(選べない) |
| `all` | すべてのウィンドウで使う | 準備中(選べない) |

## 検討した案

- **A. 真偽値の `*_shadow` を証拠源ごとに増やす**: 「読めないウィンドウだけ」「すべて」を表せない。却下。
- **B. 証拠源をまとめた 1 つのスイッチ**(ADR-264 決定 6 の案): 字形とメニューはリスクの質が違い、メニューだけ試したい利用者にも字形の取り込みを許すことになる。D1 の組み合わせ規則で字形のオプトアウトを menu が破らないようにして、別スイッチにする。却下(Q3 で所有者確認)。
- **C. 第 3・第 4 段階を今は画面に出さない**: 将来足すときに画面と設定の形を変えることになる。形だけ先に決め、保存は書き換えず実効値で丸める(D3)方が、利用者の設定が後で壊れない。
- **D. 読み取りも `unobserved` では観測の無い窓だけにする**: `observe` の価値は読める窓での直接の読みとの一致率で、これを失う。却下(D2)。
- **E. メニューは `off/observe` の 2 段階だけにする**: メニューは利用者が開いたときに 1 回読むだけで、ADR-264 決定 5 の未決 1(観測ストアの 3 秒の新鮮さ)から第 3・第 4 段階の効く時間が数秒しかない可能性がある。配線の設計次第なので、今は 4 段階の形を保ち、メニューの上限を `Observe` に据える(Q5)。

## 影響

- `src/config.rs`(型・旧キーの読み替え・`skip_serializing`・警告・テスト)、`config.toml` の例、`awase-settings`(節・プリセット表・グレーアウト・「設定値」の表示)、
  `awase-windows-core`(`ui_max_level`)、`awase-windows`(`bootstrap.rs` の分岐を実効値に置換、`ui_max_level` との min)、`architecture_guard.rs` の更新、
  不具合報告の診断フィールド `indicator_evidence`(`awase-windows` 側の生成と `awase-settings/src/bug_report.rs` の載せ替え)。
- ADR-264 決定 6・段階 5(同じスイッチ・設定画面)は本 ADR に置き換わる。ADR-261 段階 3 は残る(収集状況の表示と「見本を削除」)。ADR-261 決定 4 と ADR-264 決定 2 は改訂される: `all` が読める窓でも belief の証拠にする点(D2b)、`unobserved` 以上でも全ウィンドウを読む点(ADR-264 決定 2 の「shadow 以外は観測の無い窓でだけ読む」を、読み取りは全窓・belief への反映だけを絞る、に変える)。

## 未解決(所有者)

- Q1: 見本の画像(最大 3 件)の添付を、ADR-261 決定 6 のとおり既定の添付内容にするか、別の同意を取るか。
- Q2: `all` の便益の場面を示せなければ外す(D2b)。示せるまで枠だけ残してよいか。
- Q3: 別スイッチ(D1)でよいか(ADR-264 決定 6 の「同じスイッチ」案の置き換え)。
- Q4: 旧キーは読み替えを残す(ADR-217)か、通知だけにするか。
- Q5: メニューの第 3・第 4 段階を画面に予告する(D6)か、メニューは 2 段階だけ見せるか。
- 「観測の無いウィンドウ」の述語を ADR-261 決定 4 と ADR-263 のどちらに統一するか(配線 ADR で決める)。

## 段階

1. 型・旧キー・警告・実効値(`awase` と殻、Linux で検証可)。`bootstrap.rs` の分岐と `architecture_guard.rs` の更新。
2. 設定画面(`menu` の行)。不具合報告の `indicator_evidence`。
3. (別 ADR)第 3・第 4 段階の配線。`ui_max_level()` を上げる。字形は ADR-261 段階 1 の実装 PR で行を出す。

## 関連

[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md)・[ADR-264](264-indicator-menu-mode-evidence.md)(証拠源)・
[ADR-263](263-episode-start-key-hold-and-language-decision.md)(設定の型。「観測の無い」の述語の出所)・[ADR-233](233-stale-high-observation-beats-newer-in-most-recent-trusted.md)(`most_recent_trusted` の順位)・
[ADR-148](148-bug-report-ime-keymap-attachment.md)(不具合報告の診断)・[ADR-217](217-remove-dead-compat-and-unused-parameters.md)(旧キーの受理)・[ADR-207](207-keys-ime-detect-default-empty-and-remove-engine-ime-keys.md)(撤去キーの通知)・
[ADR-212](212-remove-preventive-and-corrective-ime-actuation-in-phases.md)(書かない)。issue #623。
