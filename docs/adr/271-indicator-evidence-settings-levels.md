---
id: ADR-271
title: |-
  タスクバーの入力インジケーターを読む 2 つの証拠源(字形・右クリックメニュー)の使い方を、設定画面の 4 段階(無効／観測のみ／読めないウィンドウだけ／すべて)で選べるようにする
summary: |-
  ADR-261(字形)と ADR-264(右クリックメニュー)は、観測の無いウィンドウの IME 状態を読む補助の証拠源で、どちらも「既定は無効のオプトイン」と決めてある。
  ところが現状は、メニューは `config.toml` の `[diagnostics] indicator_menu_shadow`(真偽値・設定画面に無い)だけ、字形は設定が無い。
  本 ADR は、証拠源ごとに 1 つの 4 段階の選択肢を置き、設定画面の上級者向けタブから選べるようにする。
  **無効**(何も読まない)／**観測のみ**(読んで journal に残す。不具合報告に添付される。belief・打鍵は変えない)／
  **読めないウィンドウだけ**(窓内の IME 状態が読めないと学習されたウィンドウでだけ、読みを belief の証拠にする)／**すべてのウィンドウ**(読めるウィンドウでも証拠にする。利用者の責任)。
  段階は累積する(上の段階は下の段階の動作を含む)。
  **本 ADR が決めるのは設定の形・意味・既定・移行・画面の出し方まで**。第 3・第 4 段階が実際に belief へ入れる配線は、ADR-261 決定 6・ADR-264 決定 5 の入口条件を満たす別 ADR/追記で決める。
  それまで第 3・第 4 段階は画面で選べず、設定ファイルに書かれていても観測のみに丸めて警告する(「選べるのに効かない」を作らない)。
status: |-
  起草(2026-10-20)。実装なし。Opus レビュー待ち。決定は案で、所有者判断を経ていない。
related_adr:
  - "ADR-261"
  - "ADR-263"
  - "ADR-264"
  - "ADR-265"
  - "ADR-268"
---

# ADR-271: 入力インジケーターの証拠源の使い方を 4 段階で選べるようにする

## ステータス

起草(2026-10-20)。実装なし。決定は案。

## 背景

観測の無いウィンドウ(窓内の IMM が読めない。一太郎など、issue #623)の IME 状態を知る補助として、
タスクバーの入力インジケーターを読む証拠源が 2 つ起草されている。

| 証拠源 | ADR | 何を読むか | 実装の現状(origin/develop 49667cc3) |
| --- | --- | --- | --- |
| 字形 | [ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md) | インジケーターの小さな画面矩形を取り込み、字形の類似度で状態を引く | **コード無し**(段階 0 の D0-2〈一太郎の実機〉が未測定) |
| 右クリックメニュー | [ADR-264](264-indicator-menu-mode-evidence.md) | 利用者がメニューを開いた瞬間に、現在のモードを UI Automation で 1 回読む | 段階 1(shadow)実装済み・v2.2.0 に同梱。`[diagnostics] indicator_menu_shadow = true` でだけ journal に残す。設定画面には無い |

両 ADR とも「既定は無効のオプトイン」(ADR-261 所有者判断 Q2・ADR-264 決定 6。画面取り込み+低レベルフックはキーロガー+スクリーングラバーに見え、
EDR/ウイルス対策の行動検出に当たりうる)と決めてあり、belief への配線(段階 2)は未決である。

所有者の要望(2026-10-20): 両者を設定画面に出し、**①単に無効にする、②観測して不具合報告に添付することだけできる、
③IME の状態を見られないウィンドウでだけ有効にする、④すべてのウィンドウで有効にする**、を選べるようにしたい。

[ADR-263](263-episode-start-key-hold-and-language-decision.md)は同じ問題を `mode`(off/observe/ime_on/ime_on_eisu)と `scope`(unobserved/all)の
2 軸の設定にし、画面では意味のある組み合わせの 1 つのプルダウンに集約した(`AUTO_DETECT_PRESETS`)。本 ADR の 4 段階はその型に揃える。

## 決定

### D1. 設定の形: 証拠源ごとに 1 つの `EvidenceLevel`

```toml
[indicator_evidence]
glyph = "off"   # "off" | "observe" | "unobserved" | "all"
menu  = "off"
```

- `src/config.rs` に `EvidenceLevel`(`Off`〈既定〉/`Observe`/`Unobserved`/`All`、`serde(rename_all = "snake_case")`、`Ord` 派生で累積を比較に使う)と
  `IndicatorEvidenceConfig { glyph, menu }` を置く。ルート `awase` クレートは OS 非依存のままで、段階の意味づけ(下の D2)は `awase-windows` 側の殻が持つ。
- 2 軸(`mode`×`scope`)にしないのは、ここでは「観測だけ」に範囲の軸が無く、組み合わせが 4 つしかないから(ADR-263 は 5 つの組み合わせ)。1 本の列挙で足りる。
- 画面は証拠源ごとに 1 つの閉じたコンボボックスで、選択中の段階の説明を閉じた状態でも出す(ADR-263 の画面と同じ作り。`626a08a6`)。

### D2. 4 段階の意味(累積)

| 段階 | 読む | journal に残す | 不具合報告に添付される | belief の証拠にする | 対象のウィンドウ |
| --- | --- | --- | --- | --- | --- |
| `off`(既定) | しない | しない | 一(config.toml の段階の値だけ) | しない | 一 |
| `observe` | する | する | する(既存の journal 添付経路) | **しない** | 読み取りはすべてのウィンドウ(結果の突き合わせは読めるウィンドウの直接の読みと) |
| `unobserved` | する | する | する | **する**(Medium 以下・`BeliefOnly`) | 窓内の IME 状態が読めないと学習されたウィンドウだけ |
| `all` | する | する | する | する | 上に加え、読めるウィンドウも。**利用者の責任** |

- `observe`: 挙動は変えない(ADR-261 決定 6 段階 1・ADR-264 決定 5 の shadow そのもの)。journal は不具合報告に直近の分が添付される(ADR-095/148 の既存経路)ので、
  「観測して不具合報告に添付する」は新しい送信経路を足さずに成り立つ。字形の `glyph_probe` の要約は ADR-261 決定 6 の allowlist に従う。
- `unobserved` の「読めない」の述語は、ADR-263 の `AutoDetectScope::Unobserved` と**同じ関数**を使う(実測学習で降格された窓。二重定義しない)。
  ADR-261 決定 4 の「`direct-follow` の読みが N 回失敗」の述語を ADR-263 が実装した後は、両者がそれに一本化される。
- `all` でも、入力中継ツールのウィンドウ(`AppImeProfile::InputRelay`。インジケーターは手元の IME でありリモート側の状態ではない)は対象外にする(ADR-263 と同じ)。
  読めるウィンドウでは直接の読み(High)が優先で、インジケーターの読みは Medium 以下に留め、新しい `ObservationSource` は `authority() = BeliefOnly`(drift correction を起こさない。ADR-261 決定 6 の入口条件 2)。
- どの段階でも**IME へは何も書かない**(ADR-261 決定 0・ADR-264 決定 0。`actuation_choke_point` を足さない)。

### D3. 第 3・第 4 段階は、belief への配線 ADR が決まるまで選べない

`unobserved`/`all` が実際に belief へ入れる配線は、ADR-261 決定 6 の入口条件(遅れによる予測の上書きの防止・drift correction 経由の書き込み禁止・`Miss` 後の Hit の失効・`Medium`)と
ADR-264 決定 5 の未決 5 項目(観測ストアの新鮮さ・GJI の I/O 推論との順序〈BUG-203〉・新しい `ObservationSource` の作り方・confidence・`BeliefOnly`)を満たす別 ADR/追記で決める。**本 ADR では配線を決めない**。そのため:

- `awase` クレートに `EvidenceSource::max_level() -> EvidenceLevel`(現状: `menu` = `Observe`、`glyph` = `Off`)を置く。配線を実装した PR が定数を上げる唯一の場所にする。
- **設定画面**: `max_level()` を超える段階はグレーアウトし、理由(「belief への反映は未対応です」)を出す。`glyph` の行は `max_level() >= Observe` になるまで画面に出さない(コードが無いものを選ばせない)。
- **読み込み**: 設定ファイルの値が `max_level()` を超えるときは `max_level()` に丸め、起動時に `warn!` を 1 回出す(`config.validate()` の正規化の仕組みに乗せ、画面のバナーにも出す)。丸めずに黙って無視すると「選んだのに効いていない」ことを利用者が知る手段が無い。
- `max_level()` を上げるときは、同じ PR で ADR-261/264 の該当入口条件の充足を示す(`fix-requires-evidence` の再発ファミリー〈IME belief〉に当たるので golden/journal リプレイか BUG ファイル)。

### D4. 既定・移行・反映

- **既定は両方 `off`**(ADR-261 Q2・ADR-264 決定 6 を維持)。`observe` を既定にしない理由: 字形は周期的な画面取り込み、メニューは UI Automation の読み取りで、どちらも EDR の行動検出に当たりうる(ADR-261 決定 7)。
  ADR-263 の `auto_detect.mode` が既定 `observe` なのは、打鍵列の統計処理でプロセス外の画面や UI を読まないからで、同列には扱わない。
- **`[diagnostics] indicator_menu_shadow` の移行**: v2.2.0 に同梱済みなので、新キー `indicator_evidence.menu` が未指定で `indicator_menu_shadow = true` のときは `menu = "observe"` として読み、`warn!` で新キーへの移行を促す。
  新キーがあればそちらが優先。**この読み替えは 1 リリース(次のマイナー版)で削除する**(ADR-217 の「名目だけの後方互換を残さない」に従い、削除する版を `config.rs` の doc に書く)。設定画面は旧キーに書き戻さない。
- **反映は再起動後**(フック・読み取りスレッド・表の収集は起動時に 1 度だけ作る。現行の `indicator_menu_shadow` と同じ。`Off`→`Observe` 以上のときだけ再起動が実質必要で、
  `Observe`/`Unobserved`/`All` の間の切り替えは、読み取りを起動したまま判定の閾値だけを変えるので将来ホットリロードにできるが、本 ADR では一律「再起動後」にする)。画面にも「反映は awase の再起動後」と出す。
- 不具合報告に添付する config.toml には段階の値が含まれる(既存。`bug_report.rs` の `config_toml`)ので、「この報告は証拠源が有効だったか」は報告から分かる。

### D5. 画面(`awase-settings`、上級者向けタブ。ADR-263 の節の下)

- 見出し「入力インジケーターを読む補助(上級者向け)」。冒頭の説明に、**awase が画面の一部(字形)または UI の項目(メニュー)を読むこと**・**既定が無効である理由(ウイルス対策ソフトが行動を検出する場合がある)**・**IME へは何も書かないこと**を平易に書く。
- 「右クリックメニュー」の行(字形は D3 のとおり、コードが入るまで非表示):

| 段階 | 画面の名前 | 説明(閉じた状態でも出す) |
| --- | --- | --- |
| `off` | 使わない | メニューを読みません。 |
| `observe` | 記録だけする | 入力インジケーターを右クリックしてメニューを開いたときだけ、現在のモードを記録します。動作は変わりません。不具合報告に含まれます。 |
| `unobserved` | IME の状態が読めないウィンドウで使う | (D3 により準備中・選べない) |
| `all` | すべてのウィンドウで使う | (D3 により準備中・選べない) |

- 「記録だけする」の説明には、報告に含まれる内容(モードの識別子と時刻のみ。他のアプリのメニューの内容は読まない〈ADR-264 決定 6〉)を書く。

## 検討した案

- **A. 真偽値の `*_shadow` を証拠源ごとに増やす**: 「読めないウィンドウだけ」「すべて」を表せない。却下。
- **B. 証拠源をまとめた 1 つのスイッチ**(ADR-261 Q2 の「収集と読み取りで同じスイッチ」を字義どおり): 字形(画面取り込み)とメニュー(UIA)はリスクの質が違い、
  メニューだけ試したい利用者にも字形の取り込みを許すことになる。Q2 の趣旨(収集と読み取りを別スイッチにしない)は**証拠源ごと**に守られている。却下。
- **C. 第 3・第 4 段階を今は画面に出さない**: 将来足すときに画面と設定の形を変えることになり、移行が増える。形だけ先に決めて丸める(D3)方が、利用者の設定が後で壊れない。
- **D. `unobserved` を既定の `observe` に含める(読み取りも観測の無い窓だけにする)**: `observe` の価値は、読める窓での直接の読みとの突き合わせ(正解のある一致率。ADR-261 決定 6・ADR-264 決定 5)で、これが無いと不具合報告から証拠源の精度が評価できない。却下。

## 影響

- `src/config.rs`(型・読み替え・丸め・テスト)、`config.toml` の例(`example-begin: config-indicator-evidence`)、`awase-settings`(節・プリセット表・グレーアウト)、
  `awase-windows/src/app/bootstrap.rs`(`indicator_menu_shadow` の分岐を `menu >= Observe` に置換)。
- ADR-261 決定 7・ADR-264 決定 6 の「既定は無効のオプトイン」は本 ADR の `off` に当たる。両 ADR の段階 3(設定画面)は本 ADR に置き換わる。

## 未解決

- Q1(所有者): `observe` の「不具合報告に添付」は、journal の直近分の既存添付(全打鍵を含むため既に利用者の同意の上で送られる)で足りるか、字形の見本の画像(最大 3 件、ADR-261 決定 6)を別に同意させるか。
- Q2(所有者): `all` で入力中継ツール以外に除外するウィンドウ(TSF ネイティブのウィンドウ(Windows Terminal 等)は ADR-263 では除外している)を揃えるか。ADR-263 に揃える案を既定とする。
- 旧キー `indicator_menu_shadow` の削除版。

## 段階

1. 型・読み替え・丸め・テスト(`awase` クレート、Linux で検証可)。`bootstrap.rs` の分岐の置換。
2. 設定画面(`menu` の行のみ。`glyph` は ADR-261 段階 1 の実装 PR で行を出す)。
3. (別 ADR)第 3・第 4 段階の配線。`max_level()` を上げる。

## 関連

[ADR-261](261-taskbar-indicator-glyph-observation-for-unreadable-windows.md)・[ADR-264](264-indicator-menu-mode-evidence.md)(証拠源)・
[ADR-263](263-episode-start-key-hold-and-language-decision.md)(設定の型と「読めない」の述語を揃える)・[ADR-265](265-mozc-readable-integration.md)/[ADR-268](268-mozc-shm-belief-connection.md)(belief への接続の先例)・
[ADR-212](212-remove-preventive-and-corrective-ime-actuation-in-phases.md)(書かない)・[ADR-217](217-remove-dead-compat-and-unused-parameters.md)(互換の削除)。issue #623。
