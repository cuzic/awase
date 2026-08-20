# ADR-095: タスクトレイからの不具合報告機能 — Cloudflare Workers + R2 による非公開受付

## ステータス

実装済み・Cloudflare 側は実デプロイ済み（2026-08-19）。`report.awase.cc`
は実際に稼働しており、`POST /v1/reports` が R2 書き込みまで到達すること
をエンドツーエンドで確認済み（詳細は「実デプロイ」節）。**Windows 実機
での動作確認（タスクトレイ UI からの一連の操作）は未実施**。

## コンテキスト

これまで不具合報告はユーザーが手動で `docs/known-bugs.md` 相当の情報
（アプリ・IME・再現手順、[experiment-logging](../../.claude/rules/experiment-logging.md)
参照）を書き起こす前提だったが、一般ユーザーには負荷が高く報告が集まりにくい。
タスクトレイから直接、症状発生時の内部状態（app kind・IME 種別・
warm/cold・直近イベント列など）を自動添付して報告できる機能を検討した。

検討の過程で以下を決定した。

1. **受け口に GitHub Issues は使わない。** 公開リポジトリの Issue に
   投稿すると、添付ログに含まれうる内部状態が全世界に公開されてしまう。
   プライバシー上許容できないユーザーが一定数いると判断し、非公開の
   フォーム受付を別途用意する方針にした（round1 レビューで、実際に
   最も機微な添付物は composition テキストやウィンドウタイトルではなく
   `journal.rs` の生打鍵列であると判明した。詳細は決定4）。
2. **報告時点での AI（LLM）による自動要約・トリアージは行わない。**
   将来的な拡張候補ではあるが、今回のスコープには含めない。
3. **受付バックエンドは Cloudflare（Workers + R2）を GCP より優先する。**
   awase の CI 基盤（GitHub Actions self-hosted runner）は既に GCP
   （[project_gcp_windows_spot_ci]）上にあり、その意味では GCP に寄せる
   方が管理主体を一本化できる利点はあった。しかし本用途（低頻度・
   突発的なフォーム送信を非公開ストレージに保存するだけ）では経済性が
   決め手になった:
   - Cloudflare Workers/R2 の無料枠はこの規模のトラフィックなら実質
     恒久的に $0（正確なカード登録要否は実アカウントでの一次確認が
     必要、既知の限界参照）。
   - R2 はエグレス（ダウンロード）課金が無いため、後日ログを何度
     読み返しても追加コストが発生しない。GCS はエグレスに課金される
     （ただし本件の想定トラフィック規模では両者とも実質 $0 に収まり、
     決定的な差ではない）。
   - 本当の決め手は「無料枠を超えたときに fail-closed するか」の非対称性。
     Cloudflare は無料枠超過時にリクエストが弾かれるだけで課金が
     発生しない構成にできるのに対し、GCP の Cloud Run/Functions は
     自動スケールして請求が発生する。カード登録前提の GCP と異なり
     乱用時に「気づいたら課金されていた」事故が起きにくい。
   - **`awase.cc` の DNS 権威は既に Cloudflare にある**ため、
     `report.awase.cc` を Workers の Custom Domain として追加しても
     ゾーンの再委任は発生しない（round1 レビューで「DNS 権威の移管が
     必要では」と指摘されたが、ユーザー確認により該当しないと判明）。
4. **awase.cc 本体（GitHub Pages）は移行しない。** 「フォーム受付が
   必要 = 静的サイト全体を Cloudflare Pages 等へ移行しなければならない」
   と当初想定していたが、必要なのは POST を受けるエンドポイントだけ
   であり、`report.awase.cc` を Cloudflare Workers の Custom Domain として
   追加するだけで足りる（前述のとおり DNS は既に Cloudflare 配下）。
   awase.cc 本体の GitHub Pages ホスティングは変更しない。

### round1 レビュー（Opus）と round2 のユーザー判断

実装着手前に Opus へアドバーサリアルレビューを依頼し、8件の must-fix
指摘を得た。うち設計判断が必要だったものはユーザーに確認し、以下の
方針で確定した（各項目の詳細は「決定」節）。

| 指摘 | 内容 | round2 での決定 |
|---|---|---|
| A-1 | ネイティブのトレイダイアログから Cloudflare Turnstile（ブラウザ JS チャレンジ前提）は使えない | Turnstile は使わない。レート制限 + サイズ上限のみで乱用対策とする（決定6） |
| A-2 | `report.awase.cc` の Custom Domain 化には DNS 権威の Cloudflare への移管が必要で、「DNS は変更しない」という前提と矛盾するのでは | `awase.cc` は既に Cloudflare の権威 DNS 配下にあり、移管は不要と判明。前提の矛盾は解消 |
| B-1 | 自動添付予定の journal ログ（`vk_code`/`scan_code` 列）は composition テキストより機微で、実質キーロガー出力に近い | マスキングはせず生データのまま添付する。安全の担保は「送信前プレビューでの手動編集」に一本化する（決定4） |
| B-2 | 送信前プレビュー/編集が UI 詳細の一語のままで要件に昇格していない | 決定4 として要件に格上げ。B-1 の判断（マスキングなし）を成立させる唯一の安全弁のため必須項目とする |
| （追加確認） | ログ添付の既定 ON/OFF | 既定 ON（外すのはユーザー操作）。B-1/B-2 の組み合わせにより「生データを既定で送る」設計になるため、決定4 のプレビュー UI は省略不可の必須ステップとして実装すること |

B-1・既定ON・Turnstile なし、という3点は組み合わさるとリスクが積み上がる
（生データが既定で自動添付され、唯一の歯止めがユーザーのプレビュー確認
のみ）。ユーザーはこの組み合わせを認識した上で採用を決定しており、
本 ADR はこれを**受け入れたリスク**として明記する。実装時は決定4の
プレビュー UI を省略・簡略化しないこと。

A-3（送信主体の分離）・B-3（R2 アクセス制御）・B-5（送信ペイロードの
allowlist 化）・D-1（乱用対策の実装詳細）は、ユーザー判断を要する
プロダクト決定ではなくエンジニアリング判断として本 ADR 側で決定した
（決定3・決定5・決定6）。C-1（Cloudflare のカード登録要否）・B-3 の
保持期間日数など、実装前の事実確認や細部の数値は「既知の限界・
未決定事項」に残す。

### 実装（2026-08-19、codex CLI）

round2 決定に基づき、Worker バックエンドとタスクトレイ UI を codex CLI
（`codex exec`、write 権限）に実装させた。2つの独立した git worktree で
並行実装させ、完了後に Claude が差分を検証・修正してマージした。

- **`services/report-worker/`**（新設ディレクトリ、TypeScript + wrangler +
  vitest）: `POST /v1/reports` を実装。ボディサイズ上限 **512KiB**、
  レート制限 **20件/IP/日**（KV カウンタ、IP は SHA-256 ハッシュ化して
  キーにのみ使用し R2 本体には保存しない）、`report_id` は Worker 内で
  生成する ULID 相当、R2 書き込みは `put()` のみ（決定5 の書き込み専用
  方針を実装レベルで担保）。`pnpm install`/`pnpm test`/`pnpm run typecheck`
  で確認済み（vitest 7件 green）。実デプロイ・`wrangler login`・実際の
  DNS/R2/KV 作成は行っていない（`services/report-worker/README.md` に
  手作業手順を記載）。
- **タスクトレイ UI**（`crates/awase-windows/src/bug_report.rs` +
  `crates/awase-settings/src/bug_report.rs`）: トレイに「不具合を報告...」
  を追加。既存の journal ダンプ導線（`UnifiedJournal::dump_to_file`）と
  既存の設定画面起動パターン（`launch_settings`）を再利用し、
  `awase-settings --bug-report` を別プロセスとして起動する（決定3、
  awase.exe 本体からは送信しない）。送信は新規 crate を追加せず
  `windows` crate の `Win32_Networking_WinHttp` feature で実装。送信
  ペイロードは `JournalEntry` を直接 serialize せず専用の
  `BugReportPayload` allowlist 型に変換する（決定3/B-5）。送信前
  プレビューは省略不可のステップとして実装し、生成される JSON 全文を
  デフォルトで編集可能な状態のまま表示する（決定4）。送信失敗時は
  `%TEMP%/awase_bug_report_failed_<unix>.json` に内容を保存し手動送付
  導線を出す。`cargo test -p awase-windows --lib`（399件 green）・
  `cargo xwin build --target x86_64-pc-windows-msvc -p awase-settings`
  （リンク成功、新規警告なし）で確認済み。

実装の過程で以下を確定・上書きした（未決定事項の一部を解消）:

- **送信データ形式**: JSON（multipart は不採用）。
- **レート制限・サイズ上限の具体値**: 20件/IP/日、ボディ512KiB、説明欄
  4,000文字、ログ抜粋256KiB（クライアント側でUTF-8境界を壊さず切り詰め）。
- **R2 保持期間**: 90日を暫定値として採用し、`wrangler r2 bucket lifecycle
  add` で実際に Cloudflare 上へ適用済み（後述「実デプロイ」節）。
- **送信元 IP の扱い**: レート制限のカウンタキー生成にのみ使用し
  （SHA-256ハッシュ化）、R2 に保存するレポート本体には含めない。
- **Worker のソース置き場所**: このリポジトリ内 `services/report-worker/`
  に同居（別リポジトリ化はしない）。
- **添付するログの範囲**: 新規の範囲指定は設けず、既存の journal ダンプ
  機構（直近最大2048エントリ、`UnifiedJournal::dump_to_file`）をそのまま
  使い、クライアント側で合計256KiBに切り詰める方針にした。

### 実デプロイ（2026-08-19）

`services/report-worker/README.md` の手順に沿って、ユーザーの Cloudflare
アカウント（`tomoya.kaw@gmail.com`、既に `wrangler` が OAuth 認証済み）に
実際にデプロイした。

1. **R2 の有効化（round1 C-1 の実地確認）**: `wrangler r2 bucket create`
   は初回 `Please enable R2 through the Cloudflare Dashboard. [code:
   10042]` で失敗した。ダッシュボードで R2 を明示的に有効化する操作が
   必要で、**クレジットカード登録は求められなかった**（無料枠の範囲内で
   完結した）。有効化後は `wrangler` から問題なくバケット作成できた。
2. **R2 バケット/KV namespace 作成**: `awase-report-bucket` と
   `RATE_LIMIT_KV`（id `9b1b037d92934d2595146c998edb5e08`）を作成し、
   `wrangler.toml` のプレースホルダ3箇所（`account_id`/`bucket_name`/
   `kv_namespaces[0].id`）を実値に置き換えた。
3. **90日 lifecycle 削除ルール**: `wrangler r2 bucket lifecycle add
   awase-report-bucket delete-old-reports reports/ --expire-days 90`
   を実行し適用済み。
4. **デプロイ**: `wrangler deploy` 成功。`wrangler.toml` の `routes`
   （`report.awase.cc/*`、zone_name `awase.cc`）も同時に登録された。
5. **DNS**: デプロイ直後は `report.awase.cc` の DNS レコードが存在せず
   （`NXDOMAIN`）、Worker route は登録されていても到達不能だった。
   Workers の「Custom Domain」機能ではなく、**ユーザーが手動で
   `report.awase.cc` の CNAME レコード（proxied）を作成**することで
   解決した。Custom Domain 機能は結果的に使わなかった（手動 CNAME +
   `wrangler.toml` の `routes` の組み合わせで十分に機能した）。
6. **エンドツーエンド疎通確認**: 有効なペイロードで
   `POST https://report.awase.cc/v1/reports` を実行し、`HTTP/2 201` と
   `{"report_id":"..."}` を確認。`wrangler r2 object get ... --remote`
   で該当オブジェクトが実際に R2 に書き込まれていることも確認した。
   確認用のテストオブジェクトはその後 `wrangler r2 object delete` で
   削除済み。

これにより round1 C-1（Cloudflare のカード登録要否）は解消し、「既知の
限界・未決定事項」の実デプロイ関連項目も解消した（詳細は同節）。

### schema v2 への再デプロイ（2026-08-19、決定7・決定8）

決定7（症状カテゴリ）・決定8（IME・キーボード環境情報）の実装後、
`wrangler deploy` で Worker を再デプロイし、`schema_version: 2` の
有効なペイロードで疎通確認した。デプロイ直後の数秒はエッジへの伝播
待ちで `unsupported_schema_version`（400）が返ったが、15〜20秒程度で
解消し `HTTP/2 201` + `report_id` を確認。`wrangler r2 object get`
で決定7・決定8の全フィールド（`symptom_category`/`ime_product_name`/
`keyboard_model`/`windows_keyboard_layout`/`competing_software`）が
R2オブジェクトに正しく反映されていることも確認した。確認用オブジェクト
は削除済み。まだ実利用者がいない機能のため、v1→v2 の移行期間や
後方互換は設けていない。

## 決定

### 1. 受け口の非公開化

GitHub Issues ではなく非公開フォームで受け付ける（前述、変更なし）。

### 2. ホスティング / DNS

```
タスクトレイ「不具合を報告...」
  → ダイアログ（自由記述 + 自動添付ログのプレビュー/編集、既定で添付ON）
  → report.awase.cc (Cloudflare Workers, Custom Domain) へ POST
  → Cloudflare R2 に非公開保存（一般公開しない）
```

- `awase.cc`（GitHub Pages）: ホスティング・DNS とも変更なし。
- `report.awase.cc`（新設）: Cloudflare Workers の Custom Domain として
  追加。`awase.cc` は既に Cloudflare の権威 DNS 配下にあるため、
  ゾーンの再委任は発生しない。
- 保存先: Cloudflare R2（非公開バケット）。閲覧はメンテナ側の手動確認を
  前提とし、この ADR の時点では自動処理（AI 要約等）を行わない。

### 3. 送信主体とペイロード（エンジニアリング判断）

- HTTP 送信は awase.exe（低レベルキーボードフックの常駐プロセス）本体
  では行わない。`crates/awase-windows/Cargo.toml` には現状 HTTP/TLS
  スタックが無く、常時稼働のフックプロセスに新規の攻撃面・依存監査
  対象を持ち込みたくない（`webbrowser` の audit 対応 `b89dfdf6` の
  実費を踏まえた判断）。送信は `awase-settings` 相当の別プロセス、
  または `windows` crate の WinHTTP バインディング（追加 crate 不要）
  経由で行う。
- 送信ペイロードは `journal.rs::JournalEntry` 等の内部型を直接
  `Serialize` しない。専用の `BugReportPayload` allowlist 型へ明示的に
  変換してから送信する。内部型に将来フィールドが増えても報告内容へ
  無自覚に混入しない設計にする（B-5 対応）。

### 4. ログ内容と送信前プレビュー（必須要件）

- journal ログの打鍵内容（`vk_code`/`scan_code`）はマスキングせず生データ
  のまま添付候補にする（ユーザー決定。再現性を優先）。
- その代わり、送信前プレビュー/編集画面を**省略不可の必須ステップ**と
  する。最低限満たす要件:
  1. 送信はユーザーの明示操作（「送信」ボタン等）でのみ発生し、暗黙
     送信は無い。
  2. 送信する全文（自由記述 + 添付ログ）をダイアログ内で表示する
     （折りたたみ・非表示状態がデフォルトにならないこと）。
  3. 添付ログは個別に外せる（自由記述のみの送信も可能）。
  4. 送信先ホスト名（`report.awase.cc`）と保存期間の目安をダイアログ内に
     明示する。

### 5. R2 への保存とアクセス制御（エンジニアリング判断）

- Worker に付与する R2 バインディングは書き込み専用（PutObject 相当）
  に限定する。List/Get 権限は与えず、閲覧はメンテナが別トークンで行う。
  Worker のトークンが漏洩しても他ユーザーの報告を読み出せないように
  するため。
- オブジェクトキーはサーバ側で生成する（クライアント指定のパスは
  使わない）。
- 保持期間・自動削除ルールの具体日数、送信元 IP の扱いは未確定
  （「既知の限界・未決定事項」参照）。

### 6. 乱用対策（Turnstile を使わない、ユーザー決定）

- Cloudflare Turnstile は導入しない（ネイティブのトレイダイアログは
  ブラウザ JS 実行環境を持たず、素朴には組み込めないため）。
- 乱用対策はレート制限（IP/日次上限）とリクエストサイズ上限、および
  JSON スキーマ検証の組み合わせのみで行う。具体的な閾値は実装時に
  決定する。

### 7. 症状カテゴリによる最小送信フロー（2026-08-19 追加、ユーザー決定）

決定4は「自由記述（必須）+ 添付ログ」の2要素を前提にしていたが、一般的な
クラッシュレポーターの多く（Windows Error Reporting・Sentry系・ブラウザの
「送信しますか」ダイアログ）はユーザー入力がほぼ無い（Yes/No 程度）。
その違いは、クラッシュレポーターは例外/クラッシュという明確なトリガーが
あり原因をシステム側が既に持っているのに対し、awase のバグは**クラッシュ
せずサイレントに違う動作をする**ため、「何が起きるべきだったか」を知って
いるのはユーザーだけである点にある。この情報の非対称性を踏まえ、
自由記述という主観的情報の価値は残しつつ、**最小操作での送信**も
できるようにする。

- ダイアログに症状カテゴリの選択肢（単一選択、10択）を追加する:
  1. 入力した文字と違う文字が出た（変換ミス）
  2. 一部の文字が消えた／出力されなかった
  3. ローマ字のまま出る／ひらがなに戻らない
  4. 全角・半角やカタカナが意図せず切り替わった
  5. 日本語入力（IME）が勝手にON/OFFになった
  6. 親指キー（無変換・変換など）が効かない、誤動作する
  7. 別のアプリに切り替えた直後におかしくなった
  8. しばらく操作しなかった後、最初の入力がおかしい
  9. キーを押しても反応しない
  10. その他
- 自由記述欄は**任意**に変更する（決定4の「説明（必須）」を修正）。
  ただし症状カテゴリで「その他」を選んだ場合は、カテゴリ自体が情報を
  持たないため自由記述を必須にする。
- **最小フロー**: カテゴリを1つ選ぶだけで送信可能（自由記述もログ添付も
  触らなくてよい）。送信前プレビュー（決定4の要件）は維持する。
- 送信ペイロードのスキーマを変更するため `schema_version` を **2** に
  上げる（`symptom_category` を必須フィールドとして追加、`description`
  の必須制約を緩和する破壊的変更のため）。Worker 側 (`services/report-worker/`)
  とタスクトレイ側の両方を同時に更新し、ズレなく同期デプロイする
  （このリポジトリでは1クライアント実装のみで後方互換を気にする実利用者
  がまだいないため、バージョン分岐実装は行わない）。

### 8. IME・キーボード環境情報の追加（2026-08-19 追加、ユーザー決定）

決定7と同じ `schema_version: 2`（未デプロイの機能なのでバージョンを
分けず1本にまとめた）に、以下4フィールドを追加した:

- `ime_product_name: string | null` — `GetLanguageProfileDescription` に
  基づく IME の実際の製品名（例: 「Google 日本語入力」）。既存の
  `ime_kind`（Gji/MsIme/Unknown の粗い3値）を補う。**新規の COM/TSF 呼び出しは
  追加しない**: `tsf/tip_detector.rs::dump_profiles()` が既に取得している
  説明文字列を `(clsid, langid, profile_guid)` キーでキャッシュし、
  `query_active_kind()` がアクティブプロファイルと照合して
  `tsf/observer.rs::TSF_OBS` の `RwLock<Option<String>>` に反映する。
  不具合報告生成時はこのキャッシュを読むだけ（取得できなければ `null`）。
- `keyboard_model: "Jis" | "Us"` — awase 自身の `GeneralConfig.keyboard_model`
  設定。
- `windows_keyboard_layout: string` — `GetKeyboardLayout` による Windows
  自身の認識するキーボードレイアウト（`LANGID=0x.... (Japanese=..)`）。
  報告生成時にその場で呼び出す副作用のない読み取り専用 API のため
  キャッシュ不要。
- `competing_software: string[]` — ADR-060 の競合ソフトウェア検出
  （やまぶき/やまぶきR/紅皿、`WH_KEYBOARD_LL` フックが衝突しうる他の
  親指シフトエミュレータ）を、起動時の一度きりの警告からも報告生成時
  からも呼べる関数 `detect_conflicting_software()` として抽出し再利用。

「キーボードドライバ」情報として物理ハードウェアドライバ名までは
含めない（Windows は簡単に取得できる標準 API を持たず、取得コストが
見合わないと判断）。最も近い概念である「競合するキーボードフック
ソフトウェア」の検出で代替する。

新規の COM/TSF 呼び出しをタスクトレイの常駐プロセス（`awase.exe`）側の
既存スレッド・既存タイミング以外から追加しないことを徹底した
（`tsf/observer.rs` は過去のバグの多くの震源地であり、新規の呼び出し元
追加はそれ自体がリスクのため）。

### 9. 内部状態スナップショット・設定ファイル・配列ファイルの添付（2026-08-19 追加、ユーザー決定）

決定7・決定8までで journal ログ（何が起きたか）と静的な環境情報は
送信できるようになったが、**その瞬間 awase が何を信じて・どう設定されて
動いていたか**は欠けていた。awase のバグの多くは例外を出さずサイレントに
違う挙動をするため（決定7の背景と同じ理由）、症状発生時の内部状態
（IME belief・conv 状態）と実際の設定内容が診断に必須になる場面が多い。
[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md)・
[experiment-logging](../../.claude/rules/experiment-logging.md) が
コミット本文に実測・失敗条件を残すことを求めているのと同じ精神を、
ユーザーからの不具合報告にも適用する。

`schema_version` を **3** に上げ、以下3つを追加した。いずれも決定4の
journal ログと同じ設計（**マスキングしない生データ、既定 ON、個別に
外せる、送信前プレビューで必ず全文表示**）を踏襲する。新規のマスキング
ロジックは作らず、安全弁は決定4のプレビュー UI に一本化する。

- **`state_snapshot`**（`attach_state_snapshot: bool` で個別 ON/OFF）:
  報告生成時点での IME belief/conv 状態のスナップショット。すべて
  `with_app` クロージャ内の既存インメモリ状態の読み取りのみで、
  新規の COM/TSF 呼び出しは追加しない（決定8の原則を踏襲）。
  - `desired_open` / `effective_open`（`ImeModel`/`ImeStateHub` の
    既存アクセサ）
  - `input_mode`（`InputModeState` の文字列表現。
    `ObservedRomaji`/`ObservedKana`/`ObservedEisu`/`AssumedRomaji{reason}`/
    `Unknown` — 実質的な conv 状態を表す）
  - `applied`（`AppliedImeState` の文字列表現）
  - `app_kind` / `focus_kind`（`FocusStore` の分類結果）
  - `gji_state`（`WindowsPlatform::gji_state_label()` の既存の
    フォーマット済み文字列をそのまま流用）

  **既知の限界**: 現在アクティブな `WriteMechanism`
  （ImmCross/GjiDirect/MsImeDirect/KanjiToggle）を返すライブ関数は
  存在しない（`ime_controller.rs::characterize_strategy` はテスト専用
  ヘルパー）。今回のスコープには含めず、将来課題として残す。

- **`config_toml`**（`attach_config: bool`）: 現在使用中の `config.toml`
  の生テキスト。`Runtime` はブート時に `AppConfig` 全体を保持せず個別
  フィールドに分解済みのため、シリアライズではなく
  `app::find_config_path()`（既存）でパスを特定してディスクから
  再読み込みする。副作用のない読み取り専用 I/O であり、決定8の
  `windows_keyboard_layout`（`GetKeyboardLayout` を都度呼ぶ）と同じ
  「キャッシュ不要な読み取り専用 API」の扱いに準じる。

- **`layout_yab`**（`attach_layout: bool`）: 現在有効な `.yab` 配列
  ファイルの生テキスト。`config_toml` から `general.layouts_dir` を
  取り出して解決し、`app.platform.tray.current_layout_name()`
  （トレイから保存せず切り替えた最新のレイアウト名を返す既存アクセサ）
  と結合して読み込む。`config.general.default_layout`（保存済みの値）
  ではなく実行時の現在値を使うことで、「設定を保存せずレイアウトだけ
  切り替えた状態で発生したバグ」も正しく再現できるようにする。

**プライバシー注記**: `layouts_dir`/`default_layout` にユーザーが絶対
パスを入力していた場合、Windows のユーザー名が含まれる可能性がある。
決定4の journal ログ同様マスキングはせず、送信前プレビューでの手動編集
のみを安全弁とする。実測サイズは典型的な `config.toml` で数 KB、`.yab`
で数 KB 程度（既存の `layout/*.yab` 実例は 1.4〜1.9KB）であり、
`log_excerpt` の上限 `LOG_EXCERPT_MAX_BYTES`（256KiB）や全体の
`MAX_BODY_BYTES`（512KiB）に対して十分小さいため、journal ログのような
末尾切り詰めロジックは設けない。ただし合計サイズが `MAX_BODY_BYTES` を
超える極端なケース（巨大な journal ログ＋大きい `config.toml`/`.yab` の
組み合わせ）に備え、クライアント側で送信前に本文サイズを見て
「添付を外してください」と案内する軽量なガードのみ入れている
（サーバ側の実際の上限はサーバ側定数がSSOT、クライアント側の値は
早期に分かりやすく警告するためだけの複製）。

**既知の限界（レビューで指摘、2026-08-19）**: `config_toml`/`layout_yab`
は報告生成時点でディスクから読み直すため、`Runtime` が実際にロード済み
の設定（起動時または最後の `WM_RELOAD_CONFIG` 時点の値）とズレる場合が
ある。具体的には、ユーザーが `config.toml` を手編集したが設定リロード
（トレイの「設定を再読み込み」またはアプリ再起動）をしていない状態で
不具合報告すると、添付される `config_toml` は編集後の内容になり、
実際に動いていた設定とは異なる。この場合、「その設定で動いていた」と
調査側が誤読するリスクがある。`layouts_dir`（config由来）は
`AppConfig::validate()` を通した値を使うため決定9初版にあった正規化漏れ
（`layouts_dir` に `..` を含む値が生のまま使われるバグ）は修正済みだが、
「設定リロード前後のズレ」自体は残る既知の限界であり、今回のスコープでは
対処しない（`state_snapshot` 側に「設定ファイルの mtime とロード時刻の
一致フラグ」を足す等の対策は将来課題）。

### 選ばなかった選択肢

- **GitHub Issues**: 公開性がプライバシー要件と相容れないため不採用。
- **GCP Cloud Functions + GCS**: 技術的には成立するが、無料枠超過時の
  fail-closed 特性で Cloudflare に劣ると判断し不採用。CI 基盤との
  管理主体一本化という利点はあったが、それを上回る決め手にはならなかった。
- **報告時 AI トリアージ（Claude API 等）**: 今回のスコープ外。将来
  追加する場合も、Workers から Claude API を叩く形で疎に追加できる
  はずで、今の設計を阻害しない。
- **ブラウザ経由の Turnstile 検証**（ループバック HTTP/カスタム URI
  スキームでトークンをアプリへ受け渡す、または報告フォーム自体を
  ブラウザで開く）: bot 対策としては最も堅牢だが、実装複雑度と
  ネイティブ UX の劣化に見合わないとユーザーが判断し不採用。

## 保持するもの（変更しないもの）

- `awase.cc` の GitHub Pages ホスティング。DNS は既に Cloudflare 配下
  であり、本 ADR による変更もない。
- 既存の `docs/known-bugs.md` ベースの手動記録運用（フォーム経由の報告は
  これを置き換えるのではなく補完する）。

## 既知の限界・未決定事項

実装（前述「実装」節）で解消した項目は取り消し線で示す。

- ~~Cloudflare R2 の有効化にカード登録が必要かどうか~~ → 2026-08-19
  にユーザーの実アカウントで確認済み（round1 C-1）。ダッシュボードでの
  明示的な有効化操作は必要だったが、**カード登録は求められなかった**
  （無料枠の範囲内）。詳細は「実デプロイ」節。
- ~~R2 の保持期間（自動削除までの日数）・送信元 IP を保存するか否か~~
  → 実装で決定（90日、IP は非保存）し、**実際の Cloudflare 上にも
  `wrangler r2 bucket lifecycle add` で適用済み**（2026-08-19）。
- ~~添付するログ/内部状態の範囲~~ → 既存の journal ダンプ機構をそのまま
  流用する方針で実装（新規の範囲指定ロジックは作らなかった）。ただし
  これが [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md)
  の「アプリ×IME×再現手順」を機械的に満たすのに十分な粒度かは、実際に
  収集された報告を見てから判断する必要がある。
- ~~送信データの形式~~ → JSON に決定・実装済み。
- ~~レート制限・サイズ上限の具体的な閾値~~ → 20件/IP/日・512KiB・
  説明4,000文字・ログ256KiB で実装済み（実運用で調整が必要になる可能性
  はある）。
- R2 に溜まった報告をメンテナがどう閲覧・トリアージするか（ダッシュボード
  無しで手動 DL するのか、簡易ビューアを別途作るか）、報告到着の通知
  手段（webhook 等）は依然として未確定。R2 の閲覧用トークン（決定5、
  Worker のトークンとは別）もまだ発行されていない。
- `main.rs` に既存の「awase.log を添えて GitHub Issue でご報告ください」
  という案内文言があり、本 ADR の決定（非公開フォームへの一本化 or
  併存）と整理が必要。**この文言は今回の実装で変更していない**
  （round1 D-4、未対応のまま）。
- ~~awase バージョン・Windows ビルド番号・IME 種別など、再現に必須の
  メタデータを送信に含めることを不変条件にするか~~ → スキーマの必須
  フィールド（`app_version`/`os_version`/`ime_kind`）として実装済み。
- プライバシーポリシーの掲示場所（`awase.cc` への追加ページ等）は未確定
  のまま（round1 B-4、未対応）。
- ~~Worker のソースコード置き場所・デプロイ手段~~ → このリポジトリ内
  `services/report-worker/` に同居する方針で実装済み。CI 組み込み
  （GitHub Actions からの `wrangler deploy`、Secrets 管理）は未着手。
- ~~タスクトレイ側 UI（ダイアログの詳細レイアウト）~~ → egui ベースで
  実装済み（`crates/awase-settings/src/bug_report.rs`）。
- **Windows 実機での動作確認は未実施。** `cargo xwin build`/`cargo test`
  （Linux）では検証したが、実際に awase.exe から「不具合を報告」を選び、
  journal ダンプ→別プロセス起動→ WinHTTP 送信という一連の経路が実機で
  動くかは未確認。**`report.awase.cc` 自体は実際に稼働している**ため
  （前述「実デプロイ」節）、実機確認では実際に送信されるところまで
  確認できるはず。
- ~~実デプロイ一式~~ → 2026-08-19 に完了（前述「実デプロイ」節）。
  `report.awase.cc` は Workers の「Custom Domain」機能ではなく、
  ユーザーが手動作成した CNAME レコード（proxied）+ `wrangler.toml` の
  `routes` で疎通している。R2 バケット・KV namespace・90日 lifecycle
  ルールも実際に作成・適用済み。エンドツーエンドの疎通確認（有効な
  ペイロードで 201 応答・R2 書き込みまで）も実施済み。
