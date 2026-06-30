# ADR-066: GJI CLSID ベース IME 種別検出（gji_write_idle_ms ヒューリスティック廃止）

## ステータス

採用済み（2026-06-27〜28 実装、commit a087553 → 70f65eb → 55233f0 → 005f0a6 → 2d5cfe9 → 699ab5f → 3ffbe66）

## コンテキスト

ADR-063 で TSF 共通層と IME 固有層を分離し、`active_ime_kind`（`GoogleJapaneseInput` / `MicrosoftIme`）で
GjiDirect / MsImeDirect の戦略を分岐させた。当初この `active_ime_kind` は `gji_monitor_ok()`
（= GJI プロセスが稼働しているか）から派生していた。

しかしこの派生には根本的な欠陥があった：

**GJI と MS-IME は共存できる。** GJI をインストールしていても、ユーザーが言語バーや
`Win+Space` で MS-IME をアクティブ IME に切り替えていることがある。このとき
GJI Converter プロセス（`GoogleIMEJaConverter.exe` 等）は常駐したままなので
`gji_monitor_ok == true` が維持され、実際には MS-IME が入力を処理しているのに
`active_ime_kind == GoogleJapaneseInput` と誤判定し、`GjiDirectStrategy` が発火していた。

この誤判定は以下の実害を生んだ：

- IME ON/OFF が GJI 固有の VK シーケンスで送られ、MS-IME には効かない／逆効果
- `LiteralDetectFsm` が GJI 前提で動作し、MS-IME アクティブ時に文字をリテラルと誤検出して
  BS（バックスペース）を連射する（commit 699ab5f が修正した症状）

「GJI プロセスが稼働しているか」と「GJI がいまアクティブ IME か」は別概念であり、
前者で後者を代用していたことが問題の本質である。

## 決定

**アクティブ TIP の CLSID を直接参照して IME 種別を判定する。** プロセス稼働の有無
（`gji_monitor_ok`）はインストール判定にとどめ、アクティブ種別の判定からは切り離す。

破壊的な一括置換を避け、観測軸を増やす中間ステップを挟んでから CLSID 一本化する
段階的移行を採った。

### Phase 1: write-idle ヒューリスティック（a087553 / 70f65eb）

最初のアプローチは、GJI が TSF へ書き込んでいない時間（`gji_write_idle_ms`）を
`ObservedState` に追加し、「GJI プロセスは生きているが一定時間 TSF へ書いていない＝
別 IME がアクティブ」と推定するものだった。

- `TsfObservations` に `gji_last_write_ms()` アクセサを追加（最後に GJI が TSF へ
  書き込んだ時刻からの経過 ms を返す）
- 10 秒以上アイドルなら `MsImeDirect` を選ぶ
- ただし候補窓表示中（`candidate_visible == true`）は GJI がアクティブなことが
  確実なので、write idle 判定をバイパスして `GjiDirect` を優先する（70f65eb）

これは確率的な推定にすぎず、しきい値（10 秒）の調整が本質的に不安定だった。
GJI で長時間入力していないだけのケースと、MS-IME に切り替えたケースを区別できない。

### Phase 2: CLSID ベース検出を併設（55233f0 / 005f0a6）

`ITfInputProcessorProfileMgr::GetActiveProfile` で現在アクティブな TIP の CLSID を取得し、
それが GJI の CLSID と一致するかで種別を判定する確定的な仕組みを導入した。

- `tsf/tip_detector.rs` を新設（198 行）
- `TsfObservations.tsf_active_kind`（`AtomicU8`）を追加。0=未取得、1=GJI、2=MS-IME
- `gji-io-monitor` スレッドが `GetActiveProfile` を **2 秒ごとにポーリング**して
  `set_tsf_active_kind()` を呼び、変化時のみ `WM_IME_KIND_CHANGED` を post する
- `active_ime_kind()` は `tsf_active_kind != 0` ならそれを優先し、未取得（0）の
  ときだけ従来の `gji_monitor_ok` 派生にフォールバックする

この段階では `gji_write_idle_ms` と CLSID の **2 系統が共存する過渡状態**だった。

GJI の CLSID はバージョンやインストール環境で変わりうるため**ハードコードしない**。
起動時に `EnumProfiles(0x0411 /* Japanese */)` で日本語 TIP を全列挙し、
display name（`GetLanguageProfileDescription`）に "Google" を含む TIP を GJI として
動的に発見し、`OnceLock<GUID>` にキャッシュする。

005f0a6 では発見した CLSID を `cache.toml` の `[tip_clsid]` セクションに永続化した。
2 回目以降の起動は `EnumProfiles`（COM 列挙）をスキップしてキャッシュから即読み込み、
高速起動する。`monitor_loop` 先頭で `set_base_dir()` を呼んでキャッシュ保存先（exe と
同じディレクトリ）を設定する。

### Phase 3: CLSID 一本化（2d5cfe9）

CLSID 判定が安定して動作することを確認し、`gji_write_idle_ms` ヒューリスティックを撤去した。

- `GjiDirectStrategy.is_applicable` / `MsImeDirectStrategy.is_applicable` を
  `active_ime_kind`（CLSID ベース）+ `gji_keybinds_ok` のみで判定するよう変更
- `ObservedState` から `gji_write_idle_ms` フィールドを削除
- write-idle に依存していたバイパスロジック（候補窓優先など）も不要になり除去

### Phase 4: LiteralDetect 連動の修正（699ab5f / 3ffbe66）

種別判定の修正に合わせて、`LiteralDetectFsm` のインストール判定も
「プロセス稼働」から「CLSID ベースのアクティブ判定」へ切り替えた。

- `observer.rs` に `gji_is_active_ime()` を追加。
  `gji_monitor_ok`（プロセス稼働）**AND** `tsf_active_kind == GoogleJapaneseInput`
  （CLSID 判定）の両方が真のときだけ true を返す
- `TsfEnvSnapshot.gji_active` と `LiteralDetectFsm` のインストール判定を
  この新関数に切り替え。MS-IME アクティブ時は LiteralDetect が動かず BS 連射が止まる
- 3ffbe66: WezTerm で Enter 後の TSF mode における LiteralDetect 誤検出を抑制。
  `is_confirm_key && is_tsf_mode && !gji_resumed` のとき `nc_fired` を true に昇格させ、
  `needs_literal` の第 2 項を抑制する

## 設計詳細

### tip_detector.rs の仕組み

すべての関数は COM STA 初期化済みの `gji-io-monitor` スレッドから呼ぶ
（COM インターフェースは STA アパートメントに束縛されるため生成スレッド以外で
使ってはいけない）。

- `create_profile_ctx()`: `monitor_loop` 先頭で `ITfInputProcessorProfileMgr` /
  `ITfInputProcessorProfiles` を生成。失敗しても `None` を返すだけで既存の GJI
  モニタリングは継続する（CLSID 判定だけが無効化される degrade）
- `discover_and_cache_gji_clsid()`: 冪等。キャッシュ済みなら即返却。なければ
  `cache.toml` を試し、それも無ければ `EnumProfiles` で発見して永続化
- `query_active_kind()`: `GetActiveProfile` の結果が `TF_PROFILETYPE_INPUTPROCESSOR`
  でなければ（IMM32 ベースの HKL）`MicrosoftIme` とみなす。TIP かつ CLSID が
  キャッシュ済み GJI CLSID と一致すれば `GoogleJapaneseInput`、それ以外は `MicrosoftIme`
- `dump_profiles()`: 起動時診断。日本語 TIP を全列挙して CLSID・名称を info ログ出力。
  新環境での CLSID 確認用

### cache.toml 永続化

ADR-058（injection-mode cache.toml）と同じファイルを共有し、`[tip_clsid]` セクションに
`gji = "{XXXXXXXX-...}"` を保存する。GUID 文字列は `fmt_guid` / `parse_guid` で
相互変換する。

### gji_is_active_ime() の意味

「GJI がいまユーザーの入力を処理しているか」を表す唯一の真偽値。
プロセス稼働だけでも、CLSID 一致だけでも不十分で、両方が揃って初めて true。
LiteralDetect のような GJI 前提の機構の起動可否はこの関数で判断する。

## 検討した代替案

**CLSID をソースにハードコードする**
→ 採用しなかった。GJI の TIP CLSID はバージョン・配布形態で変わりうる。
  `EnumProfiles` + display name マッチングなら将来のバージョン変更にも追従できる。

**gji_write_idle_ms ヒューリスティックを残してチューニングする（Phase 1 を維持）**
→ 採用しなかった。しきい値ベースの推定は「長く入力していない GJI」と
  「切り替えられた MS-IME」を原理的に区別できない。CLSID は確定的で
  チューニング不要。

**毎フレーム GetActiveProfile を呼ぶ**
→ 採用しなかった。COM 呼び出しのコストがある。2 秒ポーリング + `WM_IME_KIND_CHANGED`
  通知で十分な追従性が得られる（IME 切り替えは人間操作なので秒オーダーで足りる）。

## 変更ファイル

| ファイル | 変更内容 |
|---------|---------|
| `tsf/tip_detector.rs` | 新設。CLSID 発見・GetActiveProfile クエリ・cache.toml 永続化・診断ダンプ |
| `tsf/observer.rs` | `tsf_active_kind`(AtomicU8)、`set_tsf_active_kind`、`active_ime_kind` の CLSID 優先化、`gji_is_active_ime()`、monitor_loop の 2 秒ポーリング + `set_base_dir`/`create_profile_ctx`/`discover_and_cache_gji_clsid` 呼び出し |
| `state/ime_decision_view.rs` | `gji_write_idle_ms` 追加 → 後に削除、`TsfEnvSnapshot.gji_active` を `gji_is_active_ime()` 起点に切替 |
| `ime_controller.rs` | `GjiDirect`/`MsImeDirect` の `is_applicable` を `active_ime_kind` + `gji_keybinds_ok` のみに簡素化 |
| `LiteralDetectFsm`（detect 系） | インストール判定を `gji_is_active_ime()` に切替、Enter 後 TSF mode の誤検出抑制（`nc_fired` 昇格） |

## 関連 ADR

- ADR-063: TSF 共通層と IME 固有層の分離 + MS-IME 対応（本 ADR の基盤。`active_ime_kind` の導入元）
- ADR-058: injection-mode の cache.toml 永続化（同じキャッシュファイルを共有）
- ADR-049: TSF mode における LiteralDetect（WezTerm warm）
- ADR-048: SacrificialWarmup（GJI cold-start probe）
