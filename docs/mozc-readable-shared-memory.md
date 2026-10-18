# mozc-readable 共有メモリ仕様(外部の読み手向け)

[mozc-readable](https://github.com/cuzic/mozc-readable) の TIP(Mozc の TSF 入力方式)が書き出す、IME の開閉・入力モードの共有メモリの仕様です。awase を含め、外部のプロセスが「いまの IME の状態」を読むための取り決めです。方針と経緯は [ADR-265](adr/265-mozc-readable-integration.md) を参照してください。

- **版**: 現行の実装は **version 1**。**version 2**(`active` の追加など)は ADR-265 で決めた予定で、**まだ実装されていません**。本書では version 1 を「現行」、version 2 を「予定」と区別して書きます。
- 出典: mozc-readable の `src/win32/tip/tip_state_publisher.{h,cc}`(`readable` ブランチ)。食い違いがあれば実装が正です。
- 効く IME は mozc-readable だけです(公式の Mozc・Google 日本語入力・Microsoft IME には書き出し側が無い)。

## 想定読者と使い道: 自作キーボード(QMK など)

主な想定読者は、**自分でキーボードのファームウェアを書く人**(QMK など)です。IME の状態をキーボード側へ伝えると、たとえば次ができます。

- OLED に「あ / A」を出す。LED の色で IME の開閉を示す。
- IME が開いているときだけ、日本語入力用のレイヤーへ自動で切り替える(閉じたら戻す)。
- 「いま日本語か英数か分からず、打ってから気づく」を減らす。

Windows の標準の API では、他のアプリの IME の状態を外から確実に読めません(アプリごとに答えが違い、嘘をつくこともあります)。mozc-readable は、IME 自身がその値を共有メモリへ出すので、ここから読めば推測が要りません。

### 全体像

```text
[mozc-readable の TIP] --共有メモリ--> [PC 側の小さなブリッジ] --USB(HID)--> [キーボードのファーム]
   IME の中の値を書く                   読んで、変わったら送る               受け取って OLED/レイヤーに反映
```

キーボードのファームは、PC の IME の中を直接は見られないので、**PC 側にブリッジ(常駐する小さなプログラム)が要ります**。ブリッジの読み取り部分の最小例が [`examples/mozc-readable/read_ime_state.py`](examples/mozc-readable/read_ime_state.py) です(標準ライブラリだけ・約 100 行)。表示する代わりに、変わったときに HID でキーボードへ送れば完成です。

### キーボードへ送る(スケッチ。未検証)

QMK なら、PC からキーボードへ小さなデータを送る **Raw HID** が使えます(`rules.mk` の `RAW_ENABLE = yes`)。既定では usage page `0xFF60`・usage `0x61`、1 回 32 バイトです(自分のファームの設定で確認してください)。ブリッジ側は `hidapi` などで、たとえば次の 3 バイトを送ります。

| バイト | 内容 |
| --- | --- |
| 0 | 0(Raw HID のレポート ID。環境によっては要らない) |
| 1 | IME の開閉(0=閉、1=開、2=不明) |
| 2 | 入力モード(0〜5、不明は 255。値は 2 章の表) |

ファーム側(QMK)は `raw_hid_receive()` で受け取って、OLED の描画やレイヤー切り替えに使います。

```c
static uint8_t ime_open = 2, ime_mode = 255;   // 2=不明、255=不明

void raw_hid_receive(uint8_t *data, uint8_t length) {
    ime_open = data[1];
    ime_mode = data[2];
    // ここでレイヤーを切り替える・OLED を更新する、など
}
```

ZMK など Raw HID の仕組みが標準で無いファームについては、筆者は確認していません。PC → キーボードの経路は各自で用意してください(この仕様書の範囲外です)。

### 使う前に知っておくこと

- **Windows 専用**で、**mozc-readable を入力方式にしている窓だけ**で値が出ます。公式の Mozc・Google 日本語入力・Microsoft IME には書き出し側がありません。読めない窓は **「不明」** として扱い、キーボード側で「閉」と決め打ちしないでください(誤表示になります)。
- **現行は version 1** で、IME を Win+Space などで別の IME に切り替えても、最後の状態が残る見込みです(5-1。未実測)。version 2 で `active` を足して直す予定です。それまでは、複数の IME を切り替えて使う人は誤表示があり得ます。
- 反映の遅さは、CI の実測で、キーを押してから共有メモリに反映されるまで約 61 ms でした。ブリッジの送信と、キーボード側の描画は別にかかります。
- AppContainer で動くアプリ(UWP など)の窓では読めない見込みです(1 章。未確認)。
- インストーラー(MSI)は署名がなく、SmartScreen の警告が出ます。配布は最初の Release が出てからです(ADR-265)。

## 1. オブジェクト名

名前はすべて、対話ユーザーセッションの `Local\` 名前空間にあります。

| 種別 | 名前 | 中身 |
| --- | --- | --- |
| 共有メモリ(ページファイル裏付けのファイルマッピング) | `Local\mozc-readable-ime-<TID>` | 下記の `TipSharedState`(48 バイト)。`<TID>` は TIP が動いているスレッドの ID(10 進数) |
| 通知イベント(manual-reset) | `Local\mozc-readable-ime-changed` | 状態が変わるたびに pulse(`SetEvent` の直後に `ResetEvent`) |

- マッピングは **スレッドごと**です(TSF の状態はスレッドごとに独立。ADR-191)。読み手は、読みたい窓の**キーボードフォーカスを持つスレッド**の ID で開きます(前面窓のスレッドとは限りません。UWP は前面が `ApplicationFrameHost` で、入力は別プロセスの `CoreWindow` のスレッドです)。
- マッピングの DACL は、LocalSystem・Administrators・現在のユーザーに全権、`ALL APPLICATION PACKAGES` に読み取りです(`WinSandbox::kSharableFileForRead`)。
- **AppContainer 内のプロセス(UWP など)が作る `Local\` は、`\Sessions\<n>\AppContainerNamedObjects\<AppContainer SID>\` に振り分けられます**。そのため、通常のプロセスの読み手は `Local\mozc-readable-ime-<TID>` の名前では開けない見込みです(未確認、ADR-265 の段階 1 の前に実測する)。開けなければ「その窓では読めない」として扱ってください。

## 2. `TipSharedState`(version 1、48 バイト)

リトルエンディアン。構造体の配置は外部インターフェースの一部で、**追記のみ・互換を壊すときは `version` を上げる**取り決めです。

| オフセット | 型 | 名前 | 意味 |
| --- | --- | --- | --- |
| 0 | u32 | `magic` | `0x5A52444D`(メモリ上のバイト列は `4D 44 52 5A`、ASCII の `MDRZ`) |
| 4 | u32 | `version` | 現行は `1` |
| 8 | u32 | `seq` | seqlock の番号。**書き込み中は奇数** |
| 12 | u32 | `effective_open` | 0/1。Mozc のセッションから見た IME の開閉(入力スコープによる上書き後) |
| 16 | u32 | `effective_mode` | `commands::CompositionMode`(Mozc のセッション) |
| 20 | u32 | `tsf_open` | 0/1。TSF から見える開閉(compartment 相当) |
| 24 | u32 | `tsf_mode` | `commands::CompositionMode`(TSF) |
| 28 | u32 | `thread_id` | 書き出したスレッドの ID(マッピング名の `<TID>` と同じ) |
| 32 | u32 | `process_id` | 書き出したプロセスの ID |
| 36 | u32 | `reserved` | 0 |
| 40 | u64 | `update_tick_ms` | 最後に**変化を書いた**時の `GetTickCount64()` |

### 入力モードの値(`CompositionMode`、Mozc の `protocol/commands.proto`)

| 値 | 名前 | 意味 |
| --- | --- | --- |
| 0 | `DIRECT` | 直接入力(IME が閉じているときの値) |
| 1 | `HIRAGANA` | ひらがな |
| 2 | `FULL_KATAKANA` | 全角カタカナ |
| 3 | `HALF_ASCII` | 半角英数 |
| 4 | `FULL_ASCII` | 全角英数 |
| 5 | `HALF_KATAKANA` | 半角カタカナ |

### どちらの `*_open` / `*_mode` を使うか

- `effective_*` は、入力スコープ(パスワード欄など)による上書きの**後**の値です。
- `tsf_*` は、TSF の compartment 相当の値です。awase の belief は compartment 相当なので、awase は `tsf_*` を既定にする予定です(ADR-265 D3)。両者が食い違うとき(パスワード欄など)の扱いは、実測してから決めます。

## 3. 書き込み側の動作(現行)

- マッピングは、そのスレッドで最初に `Publish` が呼ばれたときに作られます(`magic`・`version`・`thread_id`・`process_id` を書く。この時点の `seq` は 0、状態の 4 項目も 0)。
- 書き出しは、`effective_open`・`effective_mode`・`tsf_open`・`tsf_mode` の **4 項目のどれかが変わったときだけ**です。変化が無ければ `seq` も進めず、イベントも pulse しません(フォーカス移動やクリックのたびに呼ばれるが、ほとんどは無変更のため)。したがって **`update_tick_ms` は「最後に変化した時刻」であり、生存の印ではありません**。
- 書き込みの手順: `seq` を +1(奇数)→ 4 項目と `update_tick_ms` を書く → `seq` を +1(偶数)。どちらの +1 も全バリア(`InterlockedIncrement`)。
- スレッドが終わるか DLL が外れるとビューとハンドルが閉じられ、**最後のハンドルが閉じるとマッピングは消えます**。

## 4. 読み手の規約

### 4-1. 読み方(seqlock)

```text
open  = OpenFileMappingW(FILE_MAP_READ, FALSE, "Local\mozc-readable-ime-" + tid)
view  = MapViewOfFile(open, FILE_MAP_READ, 0, 0, 48)
loop (数回まで):
    seq1 = acquire_load(view.seq)
    if seq1 is odd: retry                  # 書き込み中
    copy = volatile_read(view[0..48])       # 中身は 1 回だけ読む
    fence(acquire)
    seq2 = load(view.seq)
    if seq1 == seq2: accept copy; break     # 一致したときだけ採る
    retry
UnmapViewOfFile; CloseHandle                # 読むたびに閉じる(ハンドルを持ち続けない)
```

- **ハンドルを持ち続けない**: 読むたびに Open → Map → 読む → Unmap → Close します。持ち続けると、書き出し側のプロセスが終わった後も古いブロックが残ります。
- Rust では `read_volatile` と `fence(Ordering::Acquire)` が要ります。

### 4-2. 受け入れ条件(すべて満たしたときだけ「読めた」とする)

1. `magic == 0x5A52444D`。
2. `version >= 2`(予定。**version 1 のブロックは捨てる**。version 1 には `active` が無く、IME を切り替えた後も最後の状態が残るため。下の 5-1)。version 2 が出るまでの間は、version 1 を「記録のみ」にとどめ、belief には使わない(ADR-265 D3)。
3. `seq` が偶数で、seqlock の読みが一致した。`seq == 0` は「未公開」(作った直後で、まだ最初の書き込みが無い)なので捨てる。
4. `process_id` が、対象の窓のプロセスの PID と一致する(**スレッド ID は再利用される**ので、別プロセスの新しいスレッドが同じ番号になると古いブロックを読んでしまう)。
5. (version 2 の予定)`active == 1`。

### 4-3. 通知イベント

- `Local\mozc-readable-ime-changed` は **pulse** です。待っている間に別の処理で忙しかった読み手は取りこぼします。**起きるたびに、読みたいスレッドのブロックを読み直してください**(イベントは「読み直す合図」にだけ使う)。
- manual-reset のイベントを `SetEvent` の直後に `ResetEvent` するので、待っている全員が起きます。

### 4-4. 古い値への備え

- `update_tick_ms` の新しさでブロックの生死を判定しないでください(変化が無ければ書かないため、長く変わらない窓が「古い」と誤判定される)。生存は `process_id` の照合と、(予定の)`active` で見ます。

## 5. 既知の限界

### 5-1. IME を切り替えても状態が残る(ソース上。未実測)

書き出し側は、TIP の無効化(Win+Space で同じスレッドの IME を MS-IME などへ切り替える)で何も書きません(`Deactivate` から呼ばれていない)。ソースの読み取りでは、マッピングはスレッドが生きている限り残り、最後の値(例: 開・ひらがな)のままです。version 1 の読み手は、**いま mozc-readable が使われているか**を判別できません。

→ version 2 で `active`(Activate で 1、Deactivate で 0)を足す予定です。`active` も「変化したときだけ書く」判定の比較対象に入れます(入れないと、再 Activate のときに状態が前回と同じだと `active=1` が書かれない)。

### 5-2. 最初の書き込みの前

マッピングを作った直後は `seq == 0` で、中身も 0(閉・`DIRECT`)です。version 1 では `magic` が先に書かれるので、この状態を「閉・直接入力」として読んでしまいます。→ 4-2 の 3(`seq == 0` は捨てる)で防ぎ、version 2 で書き出し側が `magic` を最後に書くよう直す予定です。

### 5-3. 信頼境界は同一ユーザー

同じセッションのどのプロセスでも、TIP より先に同名のマッピングを作れば値を偽れます(TIP は既存のものを開いて書きます)。同一ユーザーの攻撃者は他の手段もあるため、脅威としては許容します。

### 5-4. 取れる情報

- 開閉と、上の 6 つの入力モードだけです。変換中かどうか・候補・読みなどは出しません。
- 新しいプロセスの最初の打鍵が落ちることがある件は、この仕様とは別の、mozc-readable 側の既知の問題です(原因は未特定)。

## 6. version 2(予定)

ADR-265 D3・段階 1 で決めた変更です。**未実装**。

- `active`(u32、0/1)を追加する(`reserved` を使うか末尾へ追記するかは実装時に決め、本書を更新する)。`TipTextService` の Activate で 1、Deactivate で 0 を書き、`seq` を進める。
- `Unchanged()` の比較に `active` を含める。
- 最初の書き込みで `magic` を最後に書く(`seq == 0` の未公開状態が正しい値に見えないようにする)。
- `version` を 2 に上げる。読み手は `version >= 2` だけを採る。

## 7. 変更履歴

| 日付 | 内容 |
| --- | --- |
| 2026-10-18 | 初版。version 1(現行実装)と version 2(予定)を記述。自作キーボード向けの使い道と最小例を追加 |
