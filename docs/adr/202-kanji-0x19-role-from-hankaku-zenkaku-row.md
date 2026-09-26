---
id: ADR-202
title: |-
  0x19（Alt+半角/全角）を、GJI では CUSTOM 表の `Hankaku/Zenkaku` 行から役割判定する（ADR-199 決定14 の実装設計）
summary: |-
  ADR-199 決定14 は「0x19 はユーザー設定で変えられない既知のトグル」を前提に、静的 `Toggle` を学習表で狭める移行（T14）を計画していた。
  T1(b) の実機確認（2026-09-26、GitHub Actions windows-latest、GJI の CUSTOM 表を protobuf 直接生成）で前提が逆と分かった:
  `Hankaku/Zenkaku` 行を `IMEOff` にした表では Alt+0x19 で IME が閉じ、行の無い表では閉じない（run 36242111739）。`Kanji` 行だけの表では閉じない
  （run 36242940343）。所有者決定（2026-09-26）: 0x19 を役割判定に入れる。本 ADR はその設計を定める。
  決定: (1) 対象は GJI だけ。0x19 の役割は `Hankaku/Zenkaku` 行（`VK_DBE_DBCSCHAR` 名）から求め、`Kanji` 行は見ない。プリセットでは半角/全角がトグルなので、
  プリセット利用者の挙動は変わらず、CUSTOM で行を変えた利用者だけが変わる。
  (2) 静的 `Toggle`（`hook.rs`）は残し、`enrich_key_role` が GJI のときだけ役割由来の値（`Toggle` か `None`）で上書きする。GJI 以外（MS-IME 本体・ATOK・未検出）は現行どおり。
  (3) Alt 付きは正常な入力として扱う（Ctrl/Shift/Win 付きは受動）。Down/Up の非対称を防ぐため既存の打鍵ごとのラッチを使う。
  (4) T14（学習表の `Kanji` セルで狭める移行）は撤回。学習表による狭め（決定6-2）は `Kanji` セルで従来どおり効かせる。
  (5) 実機検証を e2e に常設する（行を変えた CUSTOM 表で、awase 起動中に belief が実 IME とずれないこと）。
status: |-
  **草案（2026-09-26）。** 未決2件（`keys.ime_toggle` の既定、GJI 以外の 0x19）は下の「未決」を参照。実装は未着手。
related_adr:
  - "ADR-199"
  - "ADR-189"
  - "ADR-191"
  - "ADR-195"
---

# ADR-202: 0x19（Alt+半角/全角）を、GJI では CUSTOM 表の `Hankaku/Zenkaku` 行から役割判定する

## 背景

- 現行の awase は 0x19（`VK_KANJI`）を、IME 種別・キー設定に関わらず開閉トグルとして能動的に書く（`hook.rs` の `classify_ime_relevance` →
  `vk.rs::ImeKeyKind::Kanji.shadow_effect() = Toggle`。ADR-189）。0x19 は JIS 配列で Alt を押しながら半角/全角を押したときに届く
  （kbd106 の `T29`。ADR-199 背景1）。
- ADR-199 決定14 は、0x19 を「ユーザー設定で変えられない既知のトグル」として扱い、T1(b) で「TSF 経路で 0x19 が `Hankaku/Zenkaku` 行に従わない」と
  確認できたら学習表で狭める移行（T14）を行う、と決めていた。根拠は「IMM32 では OS 側が IME の割り当てに関係なく開閉する」（Mozc のコメント、
  `keyevent_handler.cc` L87-93）と「同梱表の実測が `Hankaku/Zenkaku` 行と食い違う」ことからの推定だった。
- **実機確認（2026-09-26、GitHub Actions windows-latest、GJI。`config1.db` の protobuf を直接生成して CUSTOM 表を与えた）**:

  | 表（`DirectInput Henkan IMEOn` で IME を開いたあと Alt+0x19） | 結果 | run |
  |---|---|---|
  | `Precomposition Hankaku/Zenkaku IMEOff` あり | 閉じる（`open` 1→0） | 36242111739 |
  | `Hankaku/Zenkaku` 行なし（対照） | 開いたまま | 36242111739 |
  | `Precomposition Kanji IMEOff` のみ（`Hankaku/Zenkaku` 行なし） | 開いたまま | 36242940343 |

  よって GJI の 0x19 は `Hankaku/Zenkaku` 行に従い、`Kanji` 行は見ない。推定は外れた。所有者決定（2026-09-26）で、0x19 を役割判定に入れる。
- 実機で GJI が書いた CUSTOM 表には `Hankaku/Zenkaku` 行が4状態とも残る（プリセット既定と同値。ADR-186 の実測 `config1-custom-keymap-table-ignored.tsv`。
  ADR-199 T1(a)）ので、行を変えていない CUSTOM 利用者では従来どおりトグルと判定される。

## 決定

### 決定1: 対象は GJI だけ。役割は `Hankaku/Zenkaku` 行から求める

- 役割の判定は既存の `awase_gji_config::role::key_role` をそのまま使い、0x19 の引き先を `VK_DBE_DBCSCHAR`（`Hankaku/Zenkaku` 行のキー名）にする。
  `Kanji` 行は見ない（`Kanji` 行だけの表で閉じない、上の実機結果。ADR-199 T2 の「`Kanji` 行は 0x19 に写さない」と一致）。
- 候補集合（`ROLE_CANDIDATE_VK_NAMES`）は変えない。0x19 は Alt 付きで届き、決定4 の無修飾ガードを通らないので、候補集合には入れず専用の分岐にする
  （ADR-199 決定4・決定14 の記述どおり）。
- 効くのは CUSTOM で `Hankaku/Zenkaku` 行を変えた利用者だけ。プリセット（ATOK/MS-IME/KOTOERI/MOBILE）は4種とも半角/全角がトグルなので、
  0x19 も従来どおり `Toggle`（挙動不変）。

### 決定2: 静的 `Toggle` は残し、GJI のときだけ上書きする

- `classify_ime_relevance`（hook、IME 非依存）の静的 `Toggle` は変えない。`Runtime::enrich_key_role` が 0x19 を扱い、`ImeKindId::Gji` のときだけ
  `shadow_action` を役割由来の値（`Some(Toggle)` か `None`）で置き換える。GJI 以外（MS-IME 本体・ATOK 本体・未検出・第三者 IME）は静的 `Toggle` のまま（現行と同じ）。
- 理由: 最小の変更で「行を変えた GJI 利用者が、IME は動かないのに awase が belief を反転する」不整合（現状の 0x19）だけを直せる。
  静的値を撤去して全 IME を役割判定に載せると、MS-IME 本体（0x19 は未確認、決定17 のとおり互換モード等の差もある）まで巻き込む。
- 合流点は増やさない。`shadow_action` を付ける場所は `enrich_key_role` の1箇所のまま（`architecture_guard` の代入箇所固定を維持）。

### 決定3: Alt 付きは正常、他の修飾は受動。ラッチで Down/Up を対称にする

- 0x19 は物理的に Alt 付きで届くので、Alt だけの修飾は受動にしない。Ctrl・Shift・Win が付くときは役割を付けない（`None`）。
  判定は純関数（例 `kanji_modifier_passive(ctrl, shift, win)`）に出し、ホストテストで固める。
- Down=Allow・Up=Suppress の非対称（BUG-131/132 型）を防ぐため、既存の打鍵ごとのラッチ（`key_role_latch`、`latch_step`）を使う。
  識別は scan_code（半角/全角キー 0x29 は 0xF3/0xF4 と同じ物理キー）。Alt を先に離すと KeyUp 時の修飾が変わるので、修飾ではなくラッチの記録で Up を決める。

### 決定4: T14 は撤回。学習表による狭めは `Kanji` セルで効かせる

- 静的 `Toggle` を学習表で狭める移行（T14）は不要（0x19 は「変えられない既知のトグル」ではない）。撤回する。
- 学習表の矛盾セルによる狭め（ADR-199 決定6-2、狭める方向だけ）は、0x19 では `TableKey::Kanji` のセルを見る（実機で測ったセルが `Kanji` なので）。
  これは既存の `derive_key_shadow_action` の `TableKey::from_vk` が 0x19 を `Kanji` に写す挙動のまま使える。

### 決定5: 実機検証を e2e に常設する

- 既存の検証構成（`ci/t1b-alt-kanji` の `sc-t1b-row-imeoff`/`sc-t1b-no-row`/`sc-t1b-kanji-row-only`）を、develop の `e2e-ime.yml` に昇格する（`tsv` 引数と
  `--chord-at`/`--chord-prep` も同時に入れる）。
- **awase 起動中**の構成を足す: `Hankaku/Zenkaku` 行を「閉じない」コマンドに変えた CUSTOM 表で Alt+0x19 を押し、実 IME の開閉と Engine の追随（belief）が
  ずれないこと。今の静的 `Toggle` ではここでずれる（IME は動かず belief だけ反転）。判定は `check_consistency.py` 系。
- ADR-199 の学習・検証の枠組みにならい、修正前（develop）で失敗・修正後で通ることを同じ構成で示す（PR #333 の手順）。

## 検討して採らなかった案

- **静的 `Toggle` を撤去し、全 IME を役割判定に載せる**: MS-IME 本体の 0x19 が未確認で、ATOK 等の役割は表が読めない。GJI だけの不整合を直す目的に対して
  影響範囲が広い。決定2 のとおり GJI だけ上書きする。
- **0x19 を候補集合に入れる**: 候補集合は無修飾で評価する前提（決定4）。0x19 は Alt 付きなので、集合に入れるとガードを外す例外が増える。専用分岐のほうが小さい。
- **`Kanji` 行も見て両方が揃ったときだけトグル**: 実機で `Kanji` 行は 0x19 に効かないと分かったので、見ると誤判定の元になる。

## 未決（所有者判断）

1. **`keys.ime_toggle` の既定（`VK_KANJI`）を空にするか。** ADR-199 決定15 は T14 と同時に空にすると決めたが、T14 を撤回したので前提が変わる。
   既定 `VK_KANJI`（無修飾）は、無修飾の 0x19 を出す構成（キーリマッパー等）にだけ一致する。GJI では役割由来の値と重なるときは
   `explicit_overlap` で役割を付けない（二重処理はしない）。推奨: **当面は空にしない**（GJI 以外で静的 `Toggle` が残るため、既定を空にすると
   ホットキー側の既存利用者だけ挙動が変わる）。空にするなら別 PR で、`awase-settings/src/main.rs:2593`（JIS 切替の書き込み）も揃える。
2. **GJI 以外の 0x19 をどうするか。** 決定2 は現行維持（静的 `Toggle`）。MS-IME 本体は Alt+半角/全角が仕様上のトグルと推定されるが未確認。
   確認するなら、GJI と同じ手順の CI 構成（MS-IME 本体で Alt+0x19）を足し、結果で MS-IME 本体だけ「固定トグル」と確定できる。推奨: **本 ADR の実装後に別途確認**。

## 実装タスク

- T16-1: 純関数（0x19 の修飾判定、GJI のときの `shadow_action` 決定）とホストテスト。`gji_key_role` の vk 名引きに 0x19→`VK_DBE_DBCSCHAR` を足す。
- T16-2: `enrich_key_role` に 0x19 の分岐（GJI のみ上書き、ラッチ共用、`architecture_guard` 維持）。`transport.rs` の物理配送は `shadow_action` の有無で従来どおり決まる。
- T16-3: e2e 常設（決定5）。修正前後の比較を PR に残す。
- T16-4: ADR-199 の決定14・T16・影響表（ADR-189 固定セット行）を本 ADR 参照に更新。未決の結論が出たら T16-5 として `keys.ime_toggle`・設定 GUI を扱う。

## 影響

- 変わるのは **GJI で `Hankaku/Zenkaku` 行を変えた CUSTOM 利用者の Alt+半角/全角** だけ。IME が動かないのに belief を反転する不整合が直る。
- 変わらない: プリセット利用者、GJI 以外の IME、無変換/変換・F13〜F24・半角/全角（0xF3/0xF4）の役割判定。
- リスク: 0x19 の Down/Up 非対称（決定3 のラッチで防ぐ、`transport.rs` を通る family なので `fix-requires-evidence` の回帰テストが要る）。
  実機で確認したのは標準 EDIT コントロール上の GJI で、TsfNative（Chrome 等）での Alt+半角/全角は未確認（T16-3 で拡げる）。
