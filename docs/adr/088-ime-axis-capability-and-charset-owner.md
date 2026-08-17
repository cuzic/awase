# ADR-088: IME 状態の軸分解（`AxisCapability`）と charset 軸の所有権（`CharsetOwner`）— および修飾キー汚染ハザードの未収束記録

## ステータス

**トラック A（`CharsetOwner`）は提案として撤回（[ADR-094](094-charset-axis-and-force-policy-removal.md)、
2026-08-17）。** charset 軸自体を awase が追跡しない方針が確定したため。
実装・実機ソークが未着手のままだったため、撤回に伴うコード削除は発生しない
（設計記録としてのみ残す）。トラック B〜D は変更なし。

**ドラフト。3つのトラックで到達点が大きく異なるため、まとめて「提案中」と
書かずにトラック別に明記する。**

| トラック | 内容 | 到達点 |
|---|---|---|
| **A. 軸モデル + `CharsetOwner`** | §2.4〜§2.9、§4 INV-29〜INV-35 | **設計として収束**（Fable（レビュアー）× Opus（設計者）の pre-mortem 往復5ラウンド、round5 で Fable が CONVERGED 判定）。**実装・実機ソークは未着手。コード変更は一切行っていない。** |
| **B. 修飾キー（Ctrl/Shift/Alt/Win）汚染ハザード** | §6 | **収束しなかった。** トラック A の設計にハザードを組み込む追加5ラウンドを実施したが、各ラウンドで発見される新規の具体的破綻シナリオが 5→4→3→3→3 件と**下げ止まり**、round5 でも棚卸し漏れの新しい送信口が発覚した。どの送信口をどう保護するか（shield / skip / defer / abort）のポリシー表は**未確定**。 |
| **C. VK モードキー送信口の棚卸し** | §5 | **完了**（本番コードの VK 送信口 **18 箇所**＝`send_input_safe` 呼び出し元 17 + `SendMessageTimeoutW` 経路 1。引き継ぎ要約にあった「62件」は算定基準が残っておらず再現できなかったため数え直した、§5 冒頭）。`architecture_guard` の判定設計案（§5.5）も確定。ただしこれは「どこから送っているか」の地図であって、「各セルをどう保護するか」（トラック B）ではない。 |
| **D. 実機実測** | §7 | **中断。** 合成キー入力（`SendInput` / `keybd_event` / `WH_JOURNALPLAYBACK`）が「API は成功を返すが実際には届かない」という原因不明の無効化に遭い、複数の仮説を検証したが特定に至らず、ユーザーの判断で打ち切った。トラック B が必要とする実測データはこのため得られていない。 |

**したがって本 ADR は「決定」ではなく、収束したトラック A の設計と、収束しな
かったトラック B・中断したトラック D の経緯を、再着手時に同じ壁へ正面から
ぶつからないために残す記録である。** `.claude/rules/experiment-logging.md` の
「なぜ前回それを捨てたのか（あるいは決められなかったのか）を辿れるようにする」
という規約を、revert コミットではなく ADR のレベルで適用する。

**実機検証の状態**: 本 ADR は新規実測を一切含まない（トラック D が中断した
ため）。§1 の因果はすべて既存の `docs/known-bugs.md` / `docs/experiments.md` /
実コード読解に由来する。トラック A の実装着手前に必要な実測は §8 の各 Phase に
明示する。

**invariant の採番**: ADR-084 が INV-1〜11、ADR-086 が INV-12〜19、ADR-087 が
INV-20〜28 を使用済みのため、本 ADR は **INV-29 から**採番する（採番理由は
ADR-086/087 と同型: 同一の名前空間に属し、後日の grep で一意に辿れることが
規約の実効性そのものであるため）。

**原則（P 番号）の採番と、設計セッション内部番号との衝突について**: ADR-084 が
P1〜P5、ADR-086 が P6〜P10、ADR-087 が P11〜P16 を使用済みのため、本 ADR は
**P17 から**採番する。**注意**: トラック A の設計セッション（Fable × Opus）は
そのセッション内部で独自に P1'〜P14 という採番を使っており、これは
ADR-084/086/087 の P1〜P16 とは**無関係**である。本 ADR に引き継がれたのは
そのうち新規に立てられた2件（セッション内番号 P13・P14）だけであり、本 ADR では
**P17・P18** として再採番した。セッション内 P1'〜P12 の本文は本 ADR に引き
継がれていない（引き継ぎ時点の要約にそれらの本文が含まれていなかった）——
これは既知の欠落であり、再着手時に「P1'〜P12 とは何だったか」を復元しようと
しないこと（復元不能な内部番号を追いかけるより、本 ADR §2 の原則と §4 の
invariant を出発点にするほうが速い）。

同様に、設計セッションの pre-mortem シナリオは S1〜S26 まで採番されているが、
本 ADR に本文が引き継がれたのは **S23〜S26 の4件のみ**である（§3.1）。

---

## 1. コンテキスト

### 1.1 IME 状態は1軸ではなく4軸である

このリポジトリは長らく IME 状態を「open/close（＋おまけで conv-mode）」として
扱ってきたが、実際に awase が壊したり壊されたりしている状態は **4つの独立した軸**
である。

| # | 軸 | 値域 | 現在の型・所在 | 観測手段 |
|---|---|---|---|---|
| 1 | **open/close** | bool | `ImeModel.desired_open`（private）/ `effective_open()`（`state/ime_model.rs`） | `ImmGetOpenStatus` / `ImmCrossProbe` / TSF / 推論 |
| 2 | **charset** | **5値** `Hiragana` / `ZenkakuKatakana` / `HankakuKatakana` / `ZenkakuAlpha` / `HankakuAlpha` | `awase::engine::conv::Charset`（`src/engine/conv.rs:19`） | `ImmGetConversionStatus` のビット合成（`NATIVE`/`KATAKANA`/`FULLSHAPE`） |
| 3 | **romaji**（入力方式: ローマ字入力 / JIS かな入力） | bool | `ConvMode.romaji`（同 `src/engine/conv.rs`、`IME_CMODE_ROMAN`） | 同上 |
| 4 | **Engine ON/OFF**（awase 自身の有効・無効） | bool | `ConvModeAuthority`（`state/conv_mode.rs:27`。実効ゲートは `Output::conv_mutation_allowed`。現状の駆動源は `EngineStateChanged` 単独＝`executor.rs:673` が唯一の書き込み点、§1.7） | awase 内部（観測ではない） |

軸 2 と 3 は IMM32 の同一 u32 ビットフィールド由来のため `ConvMode` に同居して
いるが、**性質はまったく違う**。charset は awase が日常的に書き換える軸である。

**romaji 軸の正確な状態（当初「書けない軸」と書いていたのを訂正）**: BUG-61 が
確定させたのは「**JIS かな入力へ固着した状態からローマ字入力へ戻す方向**が、
`ImmSetConversionStatus`（IMC write）でも `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` 注入でも
効かない」ことであり、実機で否定されたのは tray の「ローマ字」「かな」コマンド
（`set_ime_romaji_mode_state` / `_for_target`）と Ctrl+変換 のリセット経路である。
これらは撤去済み（`ime.rs:1754-1760` の撤去記録コメント）。

一方で **ROMAN ビットを立てる向きの IMC write は今も本番経路で生きている**:
`ime.rs:521` の `set_ime_romaji_mode()`（`conv | IME_CMODE_ROMAN`）が、IME を開く
直前の pre-mode として **2箇所**から呼ばれている——
`ime_controller.rs:78`（`ImmCrossProcessStrategy::apply` の MS-IME × ON 経路）と
`ime_controller.rs:188`（`MsImeDirectStrategy::apply` の ON 経路）。どちらも
「かな入力の conv=0x09 のまま IME ON すると JIS かな入力になる」のを防ぐための
先回りであり、`InputModeState::ObservedKana`（ユーザーが意図的にかな入力を選んだ
状態）のときは呼ばない。

したがって romaji 軸は「構造的に書けない軸」ではなく、**「片方向（→ローマ字）
だけ、しかも IMC write の形でしか触れず、復旧方向としては実機で無効と実証済み」
という非対称な軸**である。§2.4 の能力表と INV-33 はこの非対称を前提に読むこと。

**軸ごとに「読めるか」「書けるか」が違い、しかもその差がプロファイル
（`AppImeProfile` / `AppImePolicy`）ごとに違う。** ところが現状その差は
`AppImePolicy`（`state/app_ime_policy.rs`）の少数のフィールド
（`owns_physical_kanji` / `actuator_kind` / `focus_settle_ms` /
`default_feedback`）と、各所に散った `if profile.uses_kanji_toggle()` のような
分岐で表現されており、**「この軸はこのプロファイルでは書けない」という事実を
一覧できる場所がない**。

### 1.2 VK キーは「単一効果」と「複合効果」に分かれ、後者が事故の温床

| 分類 | キー | 効果 |
|---|---|---|
| **単一効果（安全）** | `VK_IME_ON`(0x16) / `VK_IME_OFF`(0x1A) | open 軸のみを**冪等に**動かす。ADR-067 が F21/F22 + `config1.db` バインドからこのキーへ全面移行した |
| **複合効果（危険）** | `VK_KANJI`(0x19) | open 軸を**トグル**（非冪等）。取りこぼし・二重送信がそのまま状態反転になる |
| **複合効果（危険）** | `VK_DBE_ALPHANUMERIC`(0xF0) / `VK_DBE_KATAKANA`(0xF1) / `VK_DBE_HIRAGANA`(0xF2) / `VK_DBE_SBCSCHAR`(0xF3) / `VK_DBE_DBCSCHAR`(0xF4) | charset 軸を動かすが、**同時に open 軸も動かす**。しかも IME が処理しない文脈では軸の外（CapsLock 等）を汚染する |
| **複合効果（危険）** | `VK_KANA`(0x15) | 単独では open 軸。MS-IME の公式ショートカット「Alt+かな」の**ラベル上の**当事者（実際に届く VK は下行、BUG-62 追補4） |
| **複合効果（危険・復旧不能）** | `VK_DBE_ROMAN`(0xF5) / `VK_DBE_NOROMAN`(0xF6) | **romaji 軸**（ローマ字入力 ⇔ JIS かな入力）を切り替える。物理 Alt+かな 押下時に **Windows のキーボードレイアウトドライバがこの2キーを合成して送ってくる**（BUG-62 追補4 で実機ログにより確定）。切り替わった後の復旧手段が存在しない（BUG-61）ため、`hook.rs:793` に専用の無条件 swallow 分岐がある（既定 ON、`GeneralConfig::swallow_alt_kana_input_method_switch` でオプトアウト可、BUG-62 追補5） |

この危険性は仮説ではなく、すべてリポジトリ内に実証記録がある。

**(a) open のつもりのキーが charset を動かす** — `platform.rs` の
`send_engine_state_ime_key()` のコメント（`platform.rs:757-779`）:

> MS-IME は IME 閉時に `VK_DBE_SBCSCHAR` を受け取ると半角英数モードで再オープン
> する挙動があり、Engine OFF / 実 IME ON の乖離を引き起こす。
> （…）OFF 時: `VK_KANJI` でクローズ直後に `VK_DBE_SBCSCHAR` が IME を再オープン
> する恐れがある。ON 時: `VK_KANJI` で開いた後に `VK_DBE_DBCSCHAR` を送ると
> 全角カタカナモードになりかねない。

**この関数の3つの early-return（`platform.rs:749` / `:764` / `:776`）は、
すべて「複合効果キーを送るべきでない状況」を個別に列挙したもの**であり、
本 ADR の能力表（§2.4）が一般形として吸収すべき対象である（§3.2 R4）。

**(b) IME が処理しない文脈では軸の外を汚染する** — `docs/experiments.md`
エントリ05（2026-07-07、Windows Terminal × MS-IME）:

> **CapsLock が点灯**。F0 は scan 0x3A（物理 CapsLock 位置）で、実 IME OFF の
> 文脈に着弾すると kbd106 の素の処理（CAPLOK）で CapsLock をトグルする
>
> **学び**: IME モードキー（F0/F2/F3 等、物理キー位置と scancode を共有）は
> 「実 IME が確実に ON」でない限り注入してはならない。（…）**belief は実状態の
> 保証にならない。**

同エントリ07（BUG-25 GJI entry の scan 付き `VK_DBE_ALPHANUMERIC` 注入）も
同じ CapsLock 汚染で即日撤回されている。

**(c) 修飾キーとの組み合わせが軸を破壊する** — BUG-62（2026-08-09）。物理 Alt を
押したまま「かな」キーを押すと MS-IME の公式ショートカット「Alt + かな
（カタカナ ひらがな ローマ字）」が発火し、ローマ字入力 → JIS かな入力へ
切り替わる。そして BUG-61 で「`ImmSetConversionStatus` も `VK_DBE_ROMAN` 注入も
復旧に効かない、Win32 に公式 API が存在しない」と**解決不能として完全クローズ**
されている。**修飾キー × IME モードキーの組み合わせは、awase が原理的に復旧
できない状態を作れる**という、この family でもっとも強い実証である。

**この BUG-62 の経緯は「特定方法」と「現在の状態」の両方を正確に引用すること**
（`docs/known-bugs.md` BUG-62 本文および追補4/5）:

- **ショートカットの存在の特定は Web 調査による**（`docs/known-bugs.md:7333`
  「**原因（Web 調査で特定）**」）。`git bisect` を使ったのは追補3 の別問題
  （「Alt+かな の後に入力不能」の原因コミット `b38d67f8` の同定）であって、
  JIS かな固着そのものの原因特定ではない。
- **実際に届く VK は `VK_KANA` ではなかった。** 追補4（実機ログ）で、
  追補1〜3 が対象にしていた `VK_KANA`(0x15) 分岐は「Alt の押下有無に関わらず
  打鍵ごとに常時 swallow されており、そもそも今回の症状の引き金ではなかった」
  ことが判明した。真の引き金は、物理 Alt+かな 押下時に**キーボードレイアウト
  ドライバが合成して送る `VK_DBE_ROMAN`(0xF5) / `VK_DBE_NOROMAN`(0xF6)** で
  あり、`hook.rs` の swallow が `VK_KANA` しか見ていなかったため素通ししていた。
- **修正済みであり、実機で再検証されている。** 追補4 で 0xF5/0xF6 専用の
  swallow 分岐（`hook.rs:793`）を追加し、追補5 で
  `GeneralConfig::swallow_alt_kana_input_method_switch` によるオプトアウトを
  足した。ユーザーが `259aeaed` を実機（Windows Terminal + MS-IME）で再検証し
  **「再発しないようになりました」と確認済み**（`docs/known-bugs.md:7517-7519`）。
  したがって本 ADR は BUG-62 を「未解決の実害」としては引用しない——引用するのは
  **「対策が3回連続で的外れだった」という探索コストの実証**としてである。

さらに BUG-62 追補2 は、その対策（swallow）自体が別の副作用を生むことを示した:
「かな」キーを丸ごと OS へ渡さないと、OS 視点では Alt が単独タップされたことに
なり `SC_KEYMENU`（システムメニュー／アクセラレータ探索モード）が起動して
以後の入力がメニューナビゲーションに食われる。対策として AutoHotkey の
`#MenuMaskKey` と同じダミー Ctrl 注入（`inject_alt_menu_mask`、`hook.rs:282`）を
導入し、追補4 の 0xF5/0xF6 分岐にも同じマスクを適用している。
**「修飾キーを触る対策は、それ自体が新しい修飾キー汚染を生む」**——これが
トラック B が収束しなかった理由の中核である（§6）。

### 1.3 「IME OFF に何のキーを送るか」が5日間に6回反転した歴史

`docs/experiments.md` エントリ01（`534051a` → `098c663` → `adb856c` →
`b271aee` → … → `489cdf1`、前史 `d4d9e27`）。反転が繰り返された理由は、
**「このキーは open 軸だけを動かすのか、他の軸も動かすのか」を宣言する場所が
無かった**ことである。`VK_DBE_ALPHANUMERIC` は複数回 IME OFF キーとして採用・
撤回され、そのたびに「これは半角英数（IME ON）であって直接入力ではない」＝
**open 軸を閉じるのではなく charset 軸を動かすキーだった**、という同じ事実を
再発見していた。

現在この結論は `tests/architecture_guard.rs` の
`ime_open_close_functions_send_expected_vk_codes()`
（`crates/awase-windows/tests/architecture_guard.rs:859`）が
「`post_ime_on_direct` / `post_ime_off_direct` に `VK_KANJI` /
`VK_DBE_HIRAGANA` / `VK_DBE_ALPHANUMERIC` が混入したら fail」という形で固定して
いる。**本 ADR の能力表（§2.4）は、この単発のガードを「軸 × プロファイル」の
表として一般化したものである。**

### 1.4 BUG-50 — charset 軸に所有権が無いことの帰結

`docs/known-bugs.md` BUG-50（2026-08-05 ユーザー報告、「一度カタカナに入ると
IME-ON コンボを押しても永久に復旧できない」。現在の見出しは
**「デッドロック解消のみ対応済み、トリガー未確定」**）。ADR-084 INV-11
（2026-08-05 BUG-50 追補）はこれを次のように総括している:

> `ConvModeMgr` が保持する確定値は「観測された値」であって「awase が選んだ値」
> ではない。この2つを区別する情報（=帰属・provenance）を持たない限り、カタカナ等の
> 非既定 charset を検出したときに「ユーザーの意図的な選択だから壊さない」
> （`ime_controller.rs::MsImeDirectStrategy::apply` の `AlreadyMatched` スキップ）と
> 「内部の誤確定だから是正すべき」を両立できない。

**INV-11 との差分は「新規性」ではなく「保持場所」である（当初の記述を訂正）。**
INV-11 は帰属を要求しただけでなく、**3状態と判定規則まで既に定義していた**
（`docs/adr/084-...:325`: 「`actuate_conv_mode`（INV-1）による書き込みから一定
時間内に観測された変化は `Attributed{by: awase}`、それ以外は `UserOriginated`、
起動直後や tray 操作等の起点不明なものは `Unknown`」）。本 ADR の
`CharsetOwner`（§2.5）は**この3状態の再命名・精緻化**であり、加えた実質は
次の3点に限られる:

1. `Attributed{by: awase}` に**目標値そのもの**を持たせた（`Awase{target: Charset}`）
   ——T2 が `Hiragana` へ無条件リセットしないための必要条件（S23）。
2. `UserOriginated` に**掌握開始 tick** を持たせた（`User{since_ms}`）。
3. INV-11 が「`ConvModeMgr` の中で区別する」形（belief 側）で書いていたものを、
   **belief ではなく actuation 側の状態**として置き直した（`reduce()` は書かない、
   §2.1）。INV-11 の想定していた `update_from_conv` への epoch 追加は、本 ADR では
   T1 の confirm 窓（§2.5）として表現される。

**なお INV-11 が実害として挙げていた `AlreadyMatched` スキップは 2026-08-06 に
撤去済みである**（§1.5 参照）。したがって `CharsetOwner` の動機は「INV-11 が
挙げた実害の解消」ではなく、**§1.5 で述べる「所有権ゲートの不在」そのもの**で
ある。

### 1.5 ADR-087 の warrant は open 軸しか守っていない

ADR-087 は `OpenWarrant` / `WarrantBasis` / `issue_open_warrant()`
（`state/open_warrant.rs`）を導入し、Step 0〜4 の優先順位付きゲートを純粋関数と
して実装した（Phase 0〜2' 実装済み、Phase 3 配線は未着手）。しかしその対象は
**open 軸だけ**である。charset 軸のゲートは依然として
`ConvModeAuthority::allows_conv_mutation()`（`state/conv_mode.rs:45`）という
**bool 1個**であり、しかもこれは `AwaseOwned`（= awase エンジン ON 中）かどうかを
返すだけで、「今の charset 目標値を誰が決めたか」の情報を持たない。

```rust
// crates/awase-windows/src/state/conv_mode.rs:45
pub const fn allows_conv_mutation(self) -> bool {
    matches!(self, Self::AwaseOwned)
}
```

**エンジン ON 中はこのゲートが常に素通しになる**（`AwaseOwned` は awase エンジン
ON と同義であり、ユーザーが IME 側 UI で charset を選んだ事実を一切表現できない）。
その結果、`kp_reset_to_hiragana_romaji_capsoff` のような「ひらがなへ寄せる」経路や
ADR-085 の force-write が、ユーザーの選択した charset を上書きすることを止める
仕組みが**構造として存在しない**。

**BUG-50 との関係は正確に書く（当初「これが BUG-50 の構造的な土台である」と
書いていたのを訂正）。** BUG-50 の原因1として列挙された4つのガードのうち、
本節の記述が指していた「ガード3」——`MsImeDirectStrategy::apply` の
`AlreadyMatched` スキップ（実 conv に KATAKANA ビットが立っていたら
`VK_DBE_HIRAGANA` を送らない）——は、**2026-08-06 の
`VK_DBE_HIRAGANA` → `VK_IME_ON` 移行で撤去済みである**
（`ime_controller.rs:142-151` のコメントと `:168-170` の「ガード自体が不要になる」
記述、および `docs/known-bugs.md` BUG-50 追補〈2026-08-06〉）。`VK_IME_ON` は
conv-mode のどのビットにも触れないため、「送るとカタカナを壊す → だからスキップ
する → スキップ判定が帰属を区別できない」という連鎖の前提そのものが消えた。

現在 BUG-50 として開いているのは**原因2（なぜ最初にカタカナへ入ったか＝トリガー、
仮説A〜C）だけ**であり、これは `CharsetOwner` が解決する問題ではない
（`CharsetOwner` は「入った後に誰の所有物として扱うか」を決める型であって、
「なぜ入ったか」を突き止める型ではない）。**したがって本 ADR は BUG-50 を
`CharsetOwner` のゴールとして掲げない**（§8 Phase 1 参照）。

### 1.6 既存 ADR / ルールとの関係

本 ADR は既存 ADR を否定しない。ADR-087 が open 軸に対して定めた「根拠軸」の
規律を、**軸ジェネリックに一般化**し、charset 軸に固有の所有権概念を足す。
**ただし1件だけ「既存 ADR の決定が既に実装から失われている」ことを新たに
記録する**（ADR-072、§1.7）——これは否定ではなく事実の記録であり、復活させるか
どうかは本 ADR のスコープ外である。

| 既存 | 何を定めたか | 本 ADR との関係 |
|---|---|---|
| [ADR-064](064-conv-mode-policy-gate.md) | conv mutation ゲート `conv_mutation_allowed: Cell<bool>`（`output/mod.rs:160`）の導入。当時の型名は `ConvModePolicy{AwaseLocked, UserManaged}` だったが、**この型は現存せず `ConvModeAuthority{Unknown, AwaseOwned, UserOwned}`（`state/conv_mode.rs:27`）に改名・3値化されている**（`0803ac30`） | **維持**。`CharsetOwner` はこのゲートを置き換えず、**隣に並べて AND 評価**する（§3.2 の R4 訂正） |
| [ADR-067](067-vk-ime-on-off-migration.md) | F21/F22 + `config1.db` → `VK_IME_ON`/`VK_IME_OFF` 全面移行 | **維持**。本 ADR は §2.4 の能力表で「`VK_IME_ON`/`VK_IME_OFF` が唯一の単一効果キーである」ことを明文化し、ADR-067 の結論を軸モデルの語彙で再確認する。**廃止・変更しない** |
| [ADR-072](072-conv-mode-authority-apply-resync.md) | `conv_mode_authority` の `EngineStateChanged` 依存を撤廃し、**apply 完了ごと（`record_ime_apply_result`）に再同期する**（2026-07-01、`e2199e74`） | **前提を訂正**（下記「ADR-072 の再同期は現存しない」参照）。ADR-072 の再同期は `552414ec`（2026-07-05）で撤去済みで、現在の実効的な駆動源は `EngineStateChanged` 単独に戻っている |
| [ADR-078](078-ime-mode-belief-desired-effective-constraint.md) | conv/mode belief の3分割（Phase 1a のみ実装） | **一部継承**。`CharsetOwner` が先取りするのは **`DesiredMode { mode, source: UserIntentSource, sequence }`**（078 の 100-106 行）が持つはずだった「誰が選んだか」であって、`ModeConstraint` ではない——後者は「アプリの都合で一時的に要求している制約（パスワード欄の Eisu 等）」であり別物（078 の 118-124 行）。ADR-078 の再開ではない（ADR-087 §1.5.1 と同じ立場） |
| [ADR-081](081-per-profile-capability-driver-decomposition.md) | プロファイル別 capability 駆動ドライバへの分離（Phase 1a/1b/1c 試験実装・未配線、1d は実機ソーク必須で未着手） | **拡張**。§2.4 の `AxisCapability` は ADR-081 が目指す capability 表の「軸 × 能力」部分の先取りである。**ただし ADR-081 Phase 1d が未配線であるため、`AxisCapability` の置き場所は新設ドライバではなく既存の `AppImePolicy`（`state/app_ime_policy.rs`）とする**（実機なしで本番経路を書き足す ADR-081 Phase 1d の躓きを繰り返さないため） |
| [ADR-084](084-conv-mode-single-ownership-and-width-ssot.md) | conv 単一 actuator（P1/INV-1）、書き込みと belief 無効化の不可分性（INV-2）、**conv 帰属 provenance（INV-11）** | **精緻化と移設**（当初「INV-11 が定義しなかった型を新設する」と書いていたのを訂正）。INV-11 は既に `Attributed{by: awase}` / `UserOriginated` / `Unknown` の3状態と判定規則を定義済みであり、`CharsetOwner` はその再命名に「目標値の保持」「掌握 tick の保持」を足し、belief 側から actuation 側へ置き直したものである（§1.4）。ADR-084 の P1 単一 actuator はそのまま `AxisActuator` へ一般化する |
| [ADR-085](085-conv-mode-force-policy.md) | `ConvModePolicy{Observe, Force}`（`src/config.rs:34`）による force-write の武装。既定 `observe`。判定点は `Output::is_force_policy()`（`output/mod.rs:300`）。**ADR-064 の旧 `ConvModePolicy` とは同名の別物** | **意味論を拡張**。`CharsetOwner::User` の間は force も止まる（§2.5）。ADR-085 の「目標値の供給元」という位置づけは変えないが、「いつでも書ける」という含みは本 ADR が制限する。**`Force` を INV-30 の第3の AND 条件にはしない**（Force は全 conv 書き込みのゲートではなく特定トリガーの武装。INV-30 の注記を参照） |
| [ADR-086](086-force-write-trigger-and-target-identity.md) | force-write の定義（§2.1）、トリガー軸（INV-15）・空間軸（INV-14）、INV-12〜19 | **番号空間を継承**（INV-29〜）。force-write の定義はそのまま使う |
| [ADR-087](087-open-belief-actuation-warrant-separation.md) | 根拠軸。`OpenWarrant`/`WarrantBasis`/`issue_open_warrant()`、INV-20〜28 | **直接の親。全面置き換えではなく軸方向への一般化**。`WarrantBasis` の限定列挙（7 variant）は**そのまま維持**し、`ExplicitUserIntent` に fresh / standing の区別を**追補**する（§2.7）。ADR-087 の Step 0〜4 の意味論は charset 軸へそのまま延長する |
| `.claude/rules/ime-belief-architecture.md` | Observe → Pure → Apply の三層分離、confidence ガード、3段構えの強制 | **追記が必要**（§8 Phase 0）: standing intent と `CharsetOwner` は `reduce()` へ何も書かない（belief ではなく actuation 側の状態である）ことを明記する |
| `.claude/rules/experiment-logging.md` | revert には失敗条件（アプリ × IME × 再現手順）を書く | §6・§7 はこの規約を「収束しなかった設計」「実施できなかった実測」へ拡張適用したもの |

### 1.7 ADR-072 の再同期は現存しない（本 ADR の前提の訂正）

本 ADR は当初、`ConvModeAuthority` について「`EngineStateChanged` 駆動のまま
**維持・変更しない**」と書いていた。これは **ADR-072 が定めた内容と食い違う**
（ADR-072 の決定は「`EngineStateChanged`〈遷移エッジ〉への依存を撤廃し、
`record_ime_apply_result` で apply 完了ごとに再同期する」）。実コードを照合した
結果、**ADR-072 の再同期は既に撤去されており、現状が偶然 ADR-072 以前の形に
戻っていた**ことが分かったので、前提ごと訂正する。

**実コードの確認結果（`bf8727ac` 時点）:**

- `set_conv_mode_authority`（`platform.rs:125`）の**呼び出し元は
  `runtime/executor.rs:673` ただ1つ**であり、そのコンテキストは
  `Effect::Ui(UiEffect::EngineStateChanged { enabled, .. })` の分岐である
  （`executor.rs:667-673`）。
- `state/platform_state.rs:584` の `record_ime_apply_result` に conv authority の
  再同期は**無い**。
- 撤去したのは `552414ec`（2026-07-05、`refactor(ime): IME適用のSSOTを一本化…`）。
  その理由はコミット本文に明記されている: **`ImeModel` 側の
  `conv_mode_authority` は誰も読まない死コードであり、実ゲートは
  `Output::conv_mutation_allowed`（`Cell<bool>`）だった**——つまり ADR-072 が
  足した再同期は `ImeModel` 側の死んだフィールドを直しており、実ゲートには
  最初から届いていなかった（コミット本文の表現は「`record_ime_apply_result`
  内の的外れな再補正コード」）。

**帰結（P17 / INV-31 の結論は変わらない）**: P17 の論拠は「`ConvModeAuthority`
と `CharsetOwner` は**遷移駆動源が違う**から1つの enum に統合できない」である。
駆動源が `EngineStateChanged`（現状）であっても `apply` 完了（ADR-072 が
意図した形）であっても、**どちらも `CharsetOwner` の4遷移〈ユーザー操作 /
awase 自身の charset 書き込み〉とは 1:1 対応しない**。したがって前提の訂正は
P17・INV-31 の結論を動かさない。動くのは表現だけであり、「ADR-072 を維持する」
ではなく「**ADR-072 が意図した再同期は現存しない。復活させるかどうかは本 ADR の
スコープ外の別課題であり、どちらに転んでも `CharsetOwner` との AND 評価は
必要**」と書くべきである（§2.2・§3.2・INV-31 を同じ趣旨に修正済み）。

**未解決として §10 に送る論点**: ADR-072 が解こうとした「Engine が Active の
まま IME だけ再 apply する経路で authority が古いまま残る」問題は、実ゲートが
`Output::conv_mutation_allowed` に一本化された今も同じ形で残っているのか
（残っているなら ADR-072 の再同期を実ゲート側へ入れ直す必要がある）。本 ADR は
これを確認していない。

---

## 2. 決定（トラック A）

### 2.1 用語

**軸（axis）**: IME 状態を構成する独立に観測・書き込みできる次元。本 ADR では
§1.1 の4軸（open / charset / romaji / engine）を指す。

**`AxisCapability`**: 「あるプロファイルで、ある軸を読めるか（`read`）／書けるか
（`write`）」を宣言する静的な能力値。観測ではなく分類であり、
ADR-087 INV-25 が言う「スコープ判定」に該当する（＝ force-write の判断材料に
使ってよい）。

**`CharsetOwner`**: 「**今の charset 目標値を誰が決めたか**」を保持する
actuation 側の状態。3状態 `Unknown` / `Awase(target: Charset)` / `User(since_ms)`。
belief ではない（`reduce()` は書かない）。

**fresh / standing ExplicitUserIntent**: ADR-087 の `WarrantBasis::ExplicitUserIntent`
を2つに区別する。**fresh** = このディスパッチ tick で発生した明示操作（トレイ・
コマンド・物理 IME キー）。**standing** = `IntentStore`
（`state/intent_store.rs`、`RecordedTargetIntent` を `HwndId` キーで保持）に
記録され TTL 内で生き残っている過去の意図。

**force-write**: ADR-086 §2.1 の定義を継承する（「外部状態の観測結果を判断材料に
使わずに、awase 自身の意図を根拠として外部状態を書き換える操作」）。

### 2.2 責務の再配置（目標状態）

| 関心事 | 所有コンポーネント | 禁止事項 |
|---|---|---|
| 軸ごとの読み書き可否 | `AxisCapability` 表（`state/app_ime_policy.rs` に配置、プロファイルから派生） | 呼び出し側が `AppImeProfile` の raw な一致判定で「このアプリでは書けない」を再実装すること（ADR-087 INV-20 の禁止事項をそのまま継承） |
| 「engine が conv を書いてよいか」 | `ConvModeAuthority`（**本 ADR では触らない**。現状の駆動源は `EngineStateChanged` 単独＝ADR-072 の apply 完了再同期は撤去済み、§1.7） | `CharsetOwner` に統合・再解釈すること（§3.1 で却下、P17） |
| 「今の charset 目標値を誰が決めたか」 | **新設 `CharsetOwner`** | `ConvModeMgr` の観測値（`update_from_conv`）から直接導出すること |
| 外部書き込みの授権 | 単一 warrant 発行点（ADR-087 `issue_open_warrant()` を軸ジェネリック化） | 軸ごとに別のゲート関数を新設すること |
| 実際の書き込み | 単一 actuator（ADR-084 P1 の `actuate_conv_mode` を軸ジェネリック化） | 低レベル API（IMC write / VK 注入）を actuator の外から呼ぶこと（ADR-084 INV-1 の継承） |
| 進行中 actuation 中の観測の扱い | in-flight 窓（§2.6） | SafetyValve と fresh intent まで保留すること（P18） |

### 2.3 原則

（P1〜P5 は ADR-084、P6〜P10 は ADR-086、P11〜P16 は ADR-087 が使用済み。
本 ADR は P17 から採番する。§ステータスの注記のとおり、設計セッション内部の
P13/P14 を再採番したものである。）

#### P17: 「書いてよいか」と「誰が決めたか」は別の状態として並存させ、AND で評価する

「engine が conv を書いてよいか」（`ConvModeAuthority`、現状 `EngineStateChanged`
駆動）と「今の charset 目標値を誰が決めたか」（`CharsetOwner`）は、**遷移の
駆動源が違う**。前者は engine の ON/OFF（ADR-072 が意図した形なら apply の完了）
に、後者はユーザー操作と awase 自身の書き込みに駆動される。**どちらの駆動源で
あっても `CharsetOwner` の4遷移とは 1:1 対応しない**（§1.7）。

**既存 enum への統合・再解釈は却下する。** 遷移駆動源が違う2つを1つの enum に
詰めると、**engine の ON/OFF が起きるたびに「ユーザーが charset を掌握している」
という記憶が消える**。したがって warrant 発行は
`allows_conv_mutation() && CharsetOwner が許すこと` の **AND** で判定する。

> **経緯（重要）**: この原則は最初からこうだったのではない。設計の round1〜3 では
> 「`ConvModeAuthority` を廃止し `CharsetOwner` に一本化する」案を採っていたが、
> **round4 で実コード照合により反証された**（`ConvModeAuthority` は
> `EngineStateChanged` 駆動であり、`CharsetOwner` の4遷移と1:1 対応しない）。
> §3.1 に却下記録として残す。**「2つのゲートを1つに統合してすっきりさせる」
> 方向は再提案しないこと。**

#### P18: SafetyValve と fresh な明示ユーザー意図は、いかなる時間窓・所有権ゲートよりも先に評価する

ADR-087 の `issue_open_warrant()` は Step 0（override 権限を持つ真の安全弁）を
Step 1（明示意図）より先に置き、その2つを他のすべてより先に評価している
（`state/open_warrant.rs` の module doc、Step 0〜4）。**この意味論を charset 軸へ
そのまま延長する。**

具体的には、`CharsetOwner::User` によるゲートも、in-flight 窓による保留も、
SafetyValve と fresh ExplicitUserIntent には**適用しない**。これを守らないと、
ユーザーが Ctrl+Shift+変換 等を押しても「今 in-flight だから」「今 User 所有だ
から」と弾かれ、**復旧操作そのものが体感無効になる**（pre-mortem S24）。

### 2.4 `AxisCapability` — 軸ごとの能力表

```rust
// state/app_ime_policy.rs（配置先。ADR-081 Phase 1d が未配線のため、
// 新設ドライバではなく既存の AppImePolicy から派生させる）

/// ある軸に対して、このプロファイルで何ができるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisCapability {
    pub read: AxisRead,
    pub write: AxisWrite,
}

pub enum AxisRead {
    /// 直接 API で読める（ImmGetOpenStatus / ImmGetConversionStatus 等）
    Direct,
    /// 推論でしか得られない（ConvOpenInference / GjiIoInference 等）
    Inferred,
    /// 構造的に読めない（FeedbackPolicy::Blind 相当）
    Impossible,
}

pub enum AxisWrite {
    /// 単一効果キー／IMC write で冪等に書ける
    Idempotent,
    /// 書けるが複合効果（他軸を巻き込む）
    SideEffecting,
    /// 構造的に書けない
    Impossible,
}
```

**能力表の運用規則:**

1. **`write = Impossible` の軸は、SafetyValve 由来の書き込みも封じる**（INV-33）。
   送るキーが物理的に存在しない軸で SafetyValve だけ例外にしても、無効キーを
   送信するだけで何も直らない（むしろ §1.2(b) の軸外汚染を招く）。

2. **`MsIme × romaji` 行は `write = Impossible` にしない。** §1.1 で訂正した
   とおり、romaji 軸は「構造的に書けない」のではなく **方向によって扱いが違う
   非対称な軸**であり、`AxisWrite` の3値では表しきれない。能力表に載せる際は
   `write = SideEffecting` とし、**次の注記を必ず添えること**:
   > **→ ローマ字方向（`conv | IME_CMODE_ROMAN` の IMC write）は今も本番で
   > 生きている**: `ime.rs:521` `set_ime_romaji_mode()` が、IME を開く直前の
   > pre-mode として `ime_controller.rs:78`（ImmCross × MS-IME の ON 経路）と
   > `ime_controller.rs:188`（MsImeDirect の ON 経路）から呼ばれる。
   > `InputModeState::ObservedKana` のときは呼ばない。
   >
   > **→ JIS かな固着からの復旧方向は実機で否定済み**（BUG-61、Windows Terminal
   > × MS-IME）。`ImmSetConversionStatus` も `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` 注入も
   > 効かず、**解決不能クローズ済み**。tray の「ローマ字」「かな」コマンドと
   > その実体 `set_ime_romaji_mode_state` / `_for_target` は撤去済みで、
   > `ime.rs:1754-1760` に撤去記録コメントが残っている。再導入するときは
   > このコメントと BUG-61 を必ず引用すること。
   >
   > **→ JIS かな方向は「awase が書く」のではなく「OS ドライバが勝手に送る」**:
   > 物理 Alt+かな で `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` が届く（BUG-62 追補4）。
   > これに対する現在の唯一の防御は `hook.rs:793` の swallow である。

   この注記は `.claude/rules/experiment-logging.md` が対象とする「再導入ループ」
   対策そのものである（`VK_DBE_ALPHANUMERIC` が IME OFF キーとして何度も
   再浮上したのと同じ形が、romaji 軸でも起こりうる）。

3. **能力の判定に `AppImeProfile` の raw な一致判定を使ってはならない**
   （ADR-087 INV-20 をそのまま継承）。`focus/class_names.rs` が
   「2026-07-05 実機バグ」として明文で禁止している判定方法であり、
   Windows Terminal の外側ウィンドウ（`CASCADIA_HOSTING_WINDOW_CLASS`）で
   誤判定する。能力表は `AppImePolicy` のフィールド
   （`default_feedback` / `actuator_kind`）から派生させる。

### 2.5 `CharsetOwner` — charset 軸の所有権（新設）

```rust
// state/conv_mode.rs（ConvModeAuthority の隣に新設。統合はしない）

/// 「今の charset 目標値を誰が決めたか」。belief ではなく actuation 側の状態。
/// reduce() はこの型に一切書かない（.claude/rules/ime-belief-architecture.md）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharsetOwner {
    /// 起動直後など、起点不明。
    Unknown,
    /// awase が決めた。target は awase が最後に意図した charset。
    Awase { target: Charset },
    /// ユーザーが掌握している。since_ms は掌握を検出した tick。
    User { since_ms: TickMs },
}
```

#### 遷移表（**4本のみ**。これ以外の遷移を足さないこと）

| # | from | to | 条件 |
|---|---|---|---|
| T1 | `Awase{target: t}` | `User{since_ms: now}` | awase が charset 書き込みをしていない状態で、**hwnd が一致する観測**が **2連続一致**（既存 `ConvModeMgr` のデバウンスを流用、`state/conv_mode.rs:214` の `update_from_conv`、BUG-19 対策）で `t` と異なる値を示したとき。**ただし直近の awase 書き込みの confirm 窓（当該段の期限）内は判定しない** |
| T2 | `User{..}` | `Awase{target: u}` | **fresh** `ExplicitUserIntent(charset = u)` または SafetyValve。目標値は `u`（ユーザーが今指定した値）。**`Hiragana` への無条件リセットはしない** |
| T3 | `*` | `Awase{target: Hiragana}` | awase エンジンの **OFF → ON 遷移時のみ** |
| T4 | （状態は保持） | — | engine OFF 時は `CharsetOwner` を**保持したまま**、`ConvModeAuthority::UserOwned` が書き込みを止める |

**T1 の confirm 窓による除外は必須である**（pre-mortem S26）。これが無いと、
ADR-086 の force-write が「自分が書いた値の遅延反映」をユーザー操作と誤認して
`User` へ降格し、**force-write が自分の書き込みで自分を停止する**。突合条件は
「最後の書き込み目標値と**異なる**こと」を要件に含めること。

**T2 が `Hiragana` へリセットしない理由**（pre-mortem S23）: 既存の
「`RomajiHiragana` へ lock する」意味論をそのまま使うと、ユーザーが明示的に
カタカナを選んだ直後の tick でそのカタカナをひらがなへ戻してしまう。ユーザーの
意図した値をそのまま目標値にする。

#### warrant 発行への接続

warrant の発行は **`allows_conv_mutation() && CharsetOwner`** の両方を見る
（P17）。`CharsetOwner::User` の間は、次の basis を**すべて却下**する:

- `DirectRead` / `Corroborated` / `SingleIndirect` / `HeuristicGuess` / `OwnSsot`
- **standing** ExplicitUserIntent（`IntentStore` 由来の過去の意図）

通すのは **fresh** ExplicitUserIntent と SafetyValve のみ（P18）。

**これにより engine ON 中の IME UI 操作も止まる**（pre-mortem S25）。
S21（engine ON 中は `AwaseOwned` のままなのでゲートが素通しし、対策が実質
無効になる）は、この AND 評価によって初めて実効を持つ。

### 2.6 in-flight 窓

進行中の actuation（プランの実行中）に届いた観測は保留する。ただし
**保留対象を限定する**（P18、pre-mortem S24）:

| 種別 | in-flight 中の扱い |
|---|---|
| `DirectRead` / `Corroborated` / `SingleIndirect` / `HeuristicGuess` / `OwnSsot` / standing intent | **保留** |
| **SafetyValve** / **fresh** `ExplicitUserIntent` | **保留しない。即通し、進行中プランを中断させる**（設計セッション内部番号 P11' が定めた「再発行可能軸」の条件下。**この P11' は ADR-087 の P11 とは無関係のセッション内部番号であり、本文は本 ADR に引き継がれていない**——§ステータスの注記を参照） |

保留の上限時間は**当該段の期限と同値**とし、超過後は自動解除する（無期限保留は
作らない）。

### 2.7 basis 分類の追補（ADR-087 の `WarrantBasis` は増やさない）

`WarrantBasis` の限定列挙（`ExplicitUserIntent` / `DirectRead` / `SingleIndirect` /
`Corroborated` / `SafetyValve` / `HeuristicGuess` / `OwnSsot`、
`state/open_warrant.rs:60`）は **7つのまま変えない**。追補は1点のみ:

- **`ExplicitUserIntent` を fresh / standing に区別する**（§2.1）。fresh のみが
  P18 の最優先評価と in-flight 即通しの対象になる。

**staleness 上限の訂正**: staleness（鮮度）上限は **standing intent にのみ**
係る。`OwnSsot` の根拠である `desired_open` はタイムスタンプを持たず staleness が
定義不能なので、上限を課さない（課そうとすると「基準時刻が無いので常に失効する」
と「常に失効しない」のどちらにも読める曖昧な実装になる。ADR-087 INV-24(c) が
`FeedbackPolicy::Blind` で同じ曖昧さを避けたのと同型）。

### 2.8 フェーズ整合 — Phase 1 では `IntentStore` を経由しない

**Phase 1 では、トレイ／コマンド／物理キーのディスパッチ元から fresh charset
intent を warrant 判定へ直接渡す**（値と発生 tick だけを持つ引数）。
`IntentStore` は経由しない。

`RecordedTargetIntent`（`state/intent_store.rs:47`）の3軸拡張（open だけでなく
charset / romaji も保持する）は **Phase 2** のままとし、**standing intent のみが
それを必要とする**。

**この順序を守らないと、Phase 1 のゲートは「SafetyValve しか通れない空回り」に
なる**——`CharsetOwner::User` 中に通せるのは fresh intent と SafetyValve だけ
なのに（§2.5）、fresh intent の配線が無ければ実質 SafetyValve 専用ゲートに
なってしまう。

### 2.9 多軸トランザクションと観測入口

**多軸トランザクション**: 複数軸を1回の warrant でまとめて書けるのは
**Open 軸の `read = Direct` なプロファイルに限る**。`TsfNative` / `Blind` 系は
**Open 単軸のみ**とする（収束時の限定をそのまま維持）。

**観測入口の hwnd 必須化（スコープ縮小版）**: open 軸の `ImeObservation` は
**既に `hwnd: HwndId` を持つため対象外**。編集が必要なのは **conv 側の観測レコード
（idle-conv-check 系）の取得元 hwnd 必須化のみ**である（`CharsetOwner` の T1 が
「hwnd が一致する観測」を条件にするため）。

**新規テスト `focus_restore_warrant.rs` の前提条件**: このテストは
`crates/awase-windows/tests/` に**まだ存在しない**（新規作成対象）。合格条件に
**「フォーカス時 probe の結果が観測レコードとして載っていること」を前提条件として
明記する**こと。ADR-087 の Step 3 は FRESH 3s 窓を持ち、その挙動は
`state/open_warrant.rs` の `now_argument_gates_step3_via_fresh_window()`
（`open_warrant.rs:645`）で固定されているため、**probe が未配線だと `DirectRead`
が出ず、テストが「設計が悪い」ではなく「配線が無い」で落ちる**。

---

## 3. 代替案の比較・却下記録

`.claude/rules/experiment-logging.md` の教訓（「良いアイデアに見えるか」ではなく
「過去にどの条件で壊れたか（決められなかったか）」で評価する）に従う。

### 3.1 pre-mortem シナリオの継承と、round4 の反証

設計セッションは S1〜S22 を継承したうえで、round3〜4 で次を**受理**した
（本 ADR に本文が引き継がれたのは S23〜S26 の4件のみ、§ステータス参照）。

| # | 内容 | 本 ADR での対処 |
|---|---|---|
| S23 | fresh intent 通過 → `Awase` 復帰の際、既存の「`RomajiHiragana` へ lock」意味論が、**次の tick でユーザーのカタカナをひらがなへ戻す** | §2.5 T2（目標値はユーザー指定値 `u`。無条件リセットしない） |
| S24 | in-flight 窓で**全 basis を保留**すると、Ctrl+Shift+変換 等が体感無効になる | §2.6（SafetyValve と fresh intent は保留しない）、P18 |
| S25 | engine ON 中は `AwaseOwned` のままなのでゲートが素通しし、**S21 が実質未対策**になる | §2.5（`CharsetOwner` との AND 評価）、P17 |
| S26 | `CharsetOwner` の降格判定が、**awase 自身の force-write（ADR-086）の遅延反映をユーザー操作と誤認**して `User` へ降格し、force-write が自分の書き込みで自分を停止する | §2.5 T1（直近書き込みの confirm 窓内は判定しない。突合条件に「最後の書き込み目標値と異なること」を要件化） |

### 3.2 却下: `ConvModeAuthority` を廃止し `CharsetOwner` に一本化する（**再提案禁止**）

**round1〜3 の設計はこれを採っていた。round4 で実コード照合により反証された。**

- **反証の内容**: `ConvModeAuthority`（`state/conv_mode.rs:27`）の遷移駆動源は
  engine の ON/OFF（実装上は `EngineStateChanged`、`executor.rs:673` が唯一の
  書き込み点）であり、`CharsetOwner` の4遷移（§2.5 T1〜T4）と **1:1 対応
  しない**。1つの enum に統合すると、engine の ON/OFF のたびに「ユーザーが charset
  を掌握している」という記憶が消える。
- **駆動源が将来変わっても反証は生き残る**: ADR-072 が意図した「apply 完了ごとの
  再同期」は現存しない（§1.7）が、仮にそれを実ゲート側へ入れ直したとしても、
  駆動源は「apply の完了」であってユーザーの charset 操作ではない。**この却下は
  現在の実装の偶然に依存していない。**
- **却下したレビュー指摘**: 「既存 enum に同居させるなら状態数と遷移表を書き切れ」
  ——**同居自体を採らず2ゲート並存にしたため、`ConvModeAuthority` 側の遷移表は
  本 ADR の変更対象外とする**（現状は `EngineStateChanged` 単独駆動。ADR-072 の
  再同期を復活させるかは §10 の別論点）。
- **教訓**: 「2つのゲートが並んでいるのは設計が汚いから統合しよう」という
  リファクタ方向の直感は、**遷移駆動源が違う場合には必ず失敗する**。
  ADR-072 が「遷移エッジではなく apply 完了点で同期する」という同型の教訓を
  出しており、本件はその3例目である。**なお ADR-072 の実装自体は死んだ
  フィールドを直していたため後日撤去されている**（§1.7）——教訓は生きているが、
  「ADR-072 のとおりに動いている」と読まないこと。

### 3.3 R4 の訂正: 既存 early-return 3行との対応表

当初「`allows_conv_mutation()` を含む4行を能力表に置き換える」としていたが、
**3行 + 1新設**へ訂正する。

| # | 既存の実害コメント付き early-return | 移行先 |
|---|---|---|
| (i) | `suppress_engine_state_key`（`platform.rs:749`。ポーリング/フォーカス変化起因の遷移では VK を送らない＝無限ループ防止） | **in-flight RAII 窓**（§2.6）へ。時間スコープが同一のため素直に対応する。既存の `SuppressEngineStateKeyGuard`（`platform.rs:51-59`）が既に RAII である |
| (ii) | `applied` 整合スキップ（`platform.rs:764`。`last_applied == enabled` なら mode key を送らない） | **Open 段完了後の charset 再評価**へ |
| (iii) | `uses_kanji_toggle()`（`platform.rs:776`。`VK_KANJI` 済みプロファイルには `VK_DBE_*` を追加送信しない） | **能力表の `write = Impossible`**（§2.4）へ |
| **(新設)** | — | `allows_conv_mutation()` は**撤去も移設もせず現状のまま残し**、`CharsetOwner` を隣に新設して AND で評価する（§3.2 の反証の帰結） |

**(i)(ii)(iii) の3行を撤去する際は、実害コメントごと `docs/known-bugs.md` へ
転記してから撤去すること。** これらのコメントは「MS-IME は IME 閉時に
`VK_DBE_SBCSCHAR` を受け取ると半角英数モードで再オープンする」といった、
コード外に残っていない実機知見を含んでいる（§1.2(a)）。

---

## 4. 不変条件（invariant）

ADR-084 の INV-1〜11、ADR-086 の INV-12〜19、ADR-087 の INV-20〜28 を継承し、
**INV-29 から**採番する。

- **INV-29（軸能力の単一宣言）**: 「あるプロファイルであるIME状態軸を読めるか／
  書けるか」は `AxisCapability` 表（§2.4）ただ1箇所で宣言する。呼び出し側が
  `AppImeProfile` の raw な一致判定や個別の `if` でこれを再実装してはならない
  （ADR-087 INV-20 の禁止事項を軸方向へ延長）。能力表は `AppImePolicy` の
  フィールド（`default_feedback` / `actuator_kind`）から派生させること。

- **INV-30（charset 軸の二重ゲート）**: charset 軸への書き込み warrant は
  `ConvModeAuthority::allows_conv_mutation()`（実体は
  `Output::conv_mutation_allowed`、`output/conv_actuation.rs:129` が唯一の判定点）
  と `CharsetOwner` の **AND** で判定する。どちらか一方だけを見てはならない。
  `CharsetOwner::User` の間は `DirectRead` / `Corroborated` / `SingleIndirect` /
  `HeuristicGuess` / `OwnSsot` / **standing** `ExplicitUserIntent` をすべて却下する。

  **`ConvModePolicy::Force`（ADR-085）を第3の AND 条件にしないこと（意図的な
  対象外、実コードで確認済み）**: `ConvModePolicy{Observe, Force}`（`src/config.rs:34`）
  は「すべての conv 書き込みを許可するか」のゲートではなく、**force-write という
  特定のトリガーを武装するか**の設定である。判定点は
  `Output::is_force_policy()`（`output/mod.rs:300`、ADR-086 §6段3-4 が単一判定点
  として固定）で、その読み手は force-pending の武装／消費と
  `apply_force_on_for_imm_broken` の抑止であって `actuate_conv_mode` の入口では
  ない。実際 `conv_mode_policy = observe`（既定）でも shift-conv-guard の
  entry/restore 等の conv 書き込みは通常どおり行われる。したがって Force を AND
  条件に足すと、**既定設定で charset 書き込みが全滅する**。正しい関係は
  「Force は `CharsetOwner` の**上流にある1つの呼び出し元**であり、その呼び出しが
  INV-30 の2ゲートを通る」であり、§2.5 の「`CharsetOwner::User` の間は force も
  止まる」はこの形で実現される。

- **INV-31（`ConvModeAuthority` は統合しない、再提案禁止）**: `ConvModeAuthority`
  と `CharsetOwner` を1つの enum に統合・再解釈してはならない。遷移駆動源が
  異なり（前者は engine の ON/OFF＝現状 `EngineStateChanged`、`executor.rs:673`
  が唯一の書き込み点）、統合すると engine ON/OFF のたびにユーザー掌握の記憶が
  消える（§3.2）。`ConvModeAuthority` 側の遷移表は**本 ADR の変更対象外**とする
  ——ADR-072 が定めた apply 完了ごとの再同期は現存しないが（§1.7）、それを
  復活させるかどうかは本 invariant とは独立の別課題であり、どちらであっても
  統合禁止の理由は変わらない。

- **INV-32（安全弁と fresh 意図の最優先評価）**: SafetyValve と **fresh**
  `ExplicitUserIntent` は、いかなる時間窓（in-flight 窓・TTL）・所有権ゲート
  （`CharsetOwner`）よりも**先に**評価する。ADR-087 `issue_open_warrant()` の
  Step 0 / Step 1 の意味論（`state/open_warrant.rs`）を charset 軸へそのまま
  延長する。

- **INV-33（`write = Impossible` は安全弁も封じる）**: 能力表で
  `write = AxisWrite::Impossible` の軸には、SafetyValve 由来であっても書き込みを
  行わない。物理的に存在しないキーを送るだけで何も直らず、`docs/experiments.md`
  エントリ05/07 の軸外汚染（CapsLock 点灯）を招く。
  **適用対象の訂正**: romaji 軸をこの invariant の適用対象にしてはならない
  （当初案では romaji を `Impossible` に分類していたが、`ime.rs:521`
  `set_ime_romaji_mode()` が `ime_controller.rs:78` / `:188` から今も呼ばれて
  おり、`Impossible` にすると本番で生きている2経路が即座に違反になる。§1.1・
  §2.4 運用規則2 を参照）。本 invariant が現時点で実際に適用できる軸は、
  能力表を書いてみて `Impossible` が確定した組（`FeedbackPolicy::Blind` 系の
  読み取り不能プロファイル等）に限る。**「BUG-61 があるから romaji は
  Impossible」という短絡を、実装時に再び持ち込まないこと。**

- **INV-34（in-flight 保留対象の限定）**: in-flight 窓が保留してよいのは
  `DirectRead` / `Corroborated` / `SingleIndirect` / `HeuristicGuess` /
  `OwnSsot` / standing intent のみ。保留の上限時間は当該段の期限と同値とし、
  超過後は自動解除する（無期限保留を作らない）。

- **INV-35（conv 観測レコードの取得元 hwnd 必須化）**: `CharsetOwner` の T1
  （降格判定）は「hwnd が一致する観測」を条件とするため、conv 側の観測レコード
  （idle-conv-check 系）は取得元 hwnd を必ず保持する。open 軸の `ImeObservation`
  は既に `hwnd: HwndId` を持つため対象外。

- **INV-36（IME モード VK の送信口は単一関門を経由し、集合として固定する）**:
  IME モード VK（`VK_KANJI` / `VK_IME_ON` / `VK_IME_OFF` / `VK_DBE_*` /
  `VK_KANA`）を実際に送信する経路は `win32::send_input_safe`（§5.3）ただ1つの
  関門を経由する。送信口の集合は `tests/architecture_guard.rs` の許可リストと
  **集合として完全一致**で比較する（部分一致・件数のみの比較にしない、§5.5）。

- **INV-37（修飾キーポリシーは実測なしに確定しない、暫定）**: 各送信口が
  Ctrl / Shift / Alt / Win の押下中にどう振る舞うべきか（shield = 合成 KeyUp で
  一時解放 / skip = 送信を見送る / defer = 解放まで遅延 / そのまま送る）の
  ポリシー表は、**実測データが得られるまで確定しない**（§6・§7）。
  **現状の非対称（§5.2）を「意図された設計」として固定するテストを書いては
  ならない**——非対称の大半は意図ではなく棚卸し漏れである可能性が高い。
  暫定的に守るべきは INV-36（送信口が増えたら検知される）だけである。

**明示的に却下する方向（再提案禁止）**:

- `ConvModeAuthority` を廃止して `CharsetOwner` へ一本化すること（INV-31、§3.2）
- `CharsetOwner` の T2 で目標値を `Hiragana` に無条件リセットすること
  （pre-mortem S23）
- in-flight 窓で SafetyValve / fresh intent まで保留すること（S24、INV-34）
- `CharsetOwner` の降格判定（T1）を、直近 awase 書き込みの confirm 窓を無視して
  行うこと（S26。force-write が自分の書き込みで自分を止める）
- `WarrantBasis` に新 variant を足して軸を表現すること（§2.7。軸は basis ではなく
  能力表とゲートで表現する）
- `AxisCapability` の判定を `AppImeProfile` の raw な値で分岐すること
  （INV-29、ADR-087 INV-20 の継承）

---

## 5. VK モードキー送信口の棚卸し（トラック C、完了）

4方向の並行探索 + 詳細確認により、IME モード VK を実際に送信しうる箇所を
棚卸しした。

**件数の訂正（当初「62件」と書いていた）**: 引き継ぎ時点の要約にあった「62件」は
**本文から再現できず、算定基準も残っていない**（§5.1 の7件 + §5.3 の2群 + §5.4 の
14種 = 約26件しか列挙されていなかった）。「完了」を主張する以上、数えられる形に
直す。**本 ADR で実際に数え直した内訳は次のとおり**（すべて `bf8727ac` 時点、
本番コード＝`crates/awase-windows/src/` 配下のみ）:

| 分類 | 件数 | 数え方（再現手順） | 本文の場所 |
|---|---|---|---|
| `win32::send_input_safe` の呼び出し元 | **17** | `grep -rn "send_input_safe" crates/awase-windows/src/` から定義行 `win32.rs:95` を除く | §5.5 の表（全17件を列挙） |
| `send_input_safe` を通らない生の `SendMessageTimeoutW` 送信口 | **1** | `ime.rs:1419` `send_f2_via_sendmessage`（実質 dead、`ime.rs:1435`/`:1449`） | §5.3 |
| **VK 送信口 小計** | **18** | 上記2つの和。これが INV-36 が守るべき集合 | — |
| IMC / TSF 直接書き込み経路（VK を送らない＝修飾キー汚染の対象外） | 別枠 | §5.4 参照 | §5.4 |

**VK 送信口として管理すべきは 18 箇所**であり、そのうち「IME モード VK
（`VK_KANJI` / `VK_IME_ON` / `VK_IME_OFF` / `VK_DBE_*` / `VK_KANA`）を実際に
送りうる」ものは §5.1 の7件 + §5.3 の dead code 群である。残りは文字送信・犠牲
キー・修飾キー操作であり、§5.5 の表に「なぜ管理対象に含めるか」を注記した。

以下は主要な行の抜粋である（修飾キー汚染ハザードに関係する送信口と、管理対象
として残すべき dead code を挙げる）。

**行番号はすべて `bf8727ac` 時点の実ファイルで裏取り済み**（本 ADR 起草時に
独立に再検証し、引き継ぎ時点の要約にあった行番号ズレと2件の事実誤認を訂正した。
訂正内容は各項の注記に明示する）。**さらに 2026-08-12 のレビュー反映時に全行番号を
再検証し、`key_pipeline.rs` の6箇所・`ime.rs::HeldModifiers::push_restore` の
1箇所・`send_input_safe` の件数を追加で訂正した**（`state/open_warrant.rs:60` の
`WarrantBasis` は再確認の結果**正しかった**ため変更していない）。

### 5.1 主要な送信口

| # | 場所 | 関数 | 送る VK | Ctrl | Shift | Alt | Win |
|---|---|---|---|---|---|---|---|
| 1 | `ime.rs:202` | `post_kanji_toggle_to_focused` | `VK_KANJI` | 解放→復元 | 解放→復元 | 解放→復元 | **判定なし** |
| 2 | `ime.rs:356` | `send_ime_mode_key`（`engine_on_ime_vk` / `engine_off_ime_vk`） | `VK_IME_ON` / `VK_IME_OFF` | 解放→復元 | 解放→復元 | **意図的に非解放** | **skip**（押下中は即 `return false`、`ime.rs:364-369`） |
| 3 | `tsf/send.rs:20` | `send_vk_dbe_hiragana_pair`（TSF warmup 用） | `VK_DBE_HIRAGANA` | **無保護** | **無保護** | **無保護** | **skip**（`win_key_held()`、`tsf/send.rs:26-29`） |
| 4 | `output/probe_io.rs:159` | `send_chrome_gji_reinit_and_poll` | `VK_IME_OFF` → `VK_IME_ON` | **無保護** | **無保護** | **無保護** | **無保護** |
| 5 | `output/mod.rs:347` | `send_unicode_cold_warmup_keys` | `VK_IME_ON` + 犠牲キー | **無保護** | **無保護** | **無保護** | **無保護** |
| 6 | `lib.rs:318`（trait 宣言は `:312`、送信は `:344`） | `reinject`（deferred replay 専用、任意 VK） | 任意 | **無保護** | **無保護** | **無保護** | **無保護** |
| 7 | `runtime/key_pipeline.rs:1436` | `kp_restore_kana_from_half_width`（scan 付き） | `VK_DBE_HIRAGANA` | **無保護** | 合成 KeyUp | **無保護** | **無保護** |

補足（いずれも実コードで確認）:

- **#1 は `send_input_safe` の戻り値を確認している**（`ime.rs:267` で送信、
  不一致時に `:275-279` で warn）。#6 は確認していない（`lib.rs:344` の
  `let _ = win32::send_input_safe(&[input]);`）。
- **#2 の Alt 非解放は明示的な設計判断**。`ime.rs:371` で `HeldModifiers::read()`
  した直後、`ime.rs:374` で `let held_skip_alt = HeldModifiers { alt: false, ..held };`
  とし、`:372-373` のコメントに「ALT を解放すると ALT+TAB スイッチャーが確定して
  しまうため、ALT は解放しない。」と記されている。呼び出し元は
  `ime_controller.rs:120/194/209` と `platform.rs:788`。
- **#3 の唯一の呼び出し元**は `output/mod.rs:757`（関数宣言は `output/mod.rs:740`
  の `send_eager_tsf_warmup`）。#3 は `HeldModifiers` を一切使っていない。
- **#7 の「リトライ機構」は VK 再送ではない（引き継ぎ時点の要約の誤り、訂正）。**
  `key_pipeline.rs:1552` の `spawn_local` 内にある `RETRY_INTERVAL_MS = 160` /
  `MAX_TRIES = 4`（`:1557-1558`）が回すのは `set_ime_conv_for_target`
  （**IMC conv write**）であり、`VK_DBE_HIRAGANA` の再注入ではない。**VK 注入は
  1回だけ**である。したがって「リトライ各回で修飾キー状態を再判定する必要がある」
  という懸念は成立しない。合成 Shift KeyUp（`make_tsf_key_input(VK_SHIFT, true)`、
  `:1493-1496`）は `VK_DBE_HIRAGANA` ペア（`:1498-1505`）と**同一バッチの先頭**に
  積まれ、`:1506` で1回送信される。
- **#7 は無条件には発火しない**: `active_ime_kind == MicrosoftIme` かつ
  `effective_open()` が真の場合に限る（`:1458` / `:1485`）。

### 5.2 主要な発見（4点）

1. **Alt の扱いが2大送信経路で正反対**。`post_kanji_toggle_to_focused`（#1）は
   Alt を解放するが、`send_ime_mode_key`（#2）は **意図的に解放しない**
   （Alt+Tab スイッチャーが確定してしまうのを避けるため。コード上のコメントに
   明記されている）。**どちらが正しいかは実測されていない。**

2. **Win skip 保護があるのは 7 経路中 2 経路のみ**（#2 と #3）。ADR-061
   （Win キー押下中の IME キー注入スキップ）が定めた保護が、その後に追加された
   送信口へ波及していない。裏取り: `win_key_held()` の定義は `hook.rs:222`、
   **実呼び出しはリポジトリ全体で2箇所のみ**（`ime.rs:364` と `tsf/send.rs:26`）。
   他のヒット（`tuning.rs:234`、`state/win_key_guard.rs:3`、`hook.rs:213/236`）は
   すべて doc コメントである。

3. **完全に無保護の送信口が複数ある**（#4 `probe_io.rs`、#5 `output/mod.rs`、
   #6 `lib.rs::reinject`、および #3 の Ctrl/Shift/Alt）。#6 は `SendInput` の
   戻り値すら見ていない。

4. **実際の VK 送信はすべて `win32::send_input_safe` という単一の関門を経由して
   いる**。生の `SendInput(` 呼び出しは**本番コード（`crates/*/src/` および
   `src/`）では** `win32.rs:99`（`send_input_safe` の本体、関数定義は
   `win32.rs:95`）**ただ1箇所**であり、`keybd_event` の実呼び出しは存在しない
   （`hook.rs:16` のコメント言及のみ）。これは `architecture_guard` の土台に
   なりうる（§5.5、INV-36）。

   **「本番コード限定」という限定を明記すること（訂正）**: テストコードには生の
   `SendInput` が **2箇所**ある——`crates/awase-windows/tests/e2e_windows.rs:704`
   と `:2642`。§5.5 の `architecture_guard` は既存の `production_code_only`
   （`tests/architecture_guard.rs:111`）で走査対象を本番コードに絞るため、この
   2箇所は判定に影響しない。ただし**トラック D（§7）が再開して実機テスト
   ハーネスを増やす場合、この2箇所と同じ形で「関門を通らない送信」が増える**
   ことに注意する（テストハーネス自身の送信は INV-36 の対象外だが、それが本番
   経路の挙動を検証していると誤読しないこと）。

**修飾キー保護の実体（`HeldModifiers`）**: `ime.rs:107` に private struct として
定義され（`{ ctrl, shift, alt }`、`crate::ime` 内限定）、API は3つ——
`read()`（`:118`）/ `push_release()`（`:131`）/ `push_restore()`（`:149`）。
**Win はメンバーに無い**（Win は解放できないため、判定は `win_key_held()` 側の
別扱いになっている）。`read()` は `GetAsyncKeyState` ではなく
`crate::hook::is_physical_key_down`（`PHYSICAL_KEY_STATE`）を使う——合成 KeyUp に
よる自己汚染を避けるための既存の設計判断であり、能力表化のときも維持すること。

**兄弟関数 `alt_key_held()`（`hook.rs:256`）は VK 送信保護ではない。**
呼び出し元は `hook.rs:742` / `hook.rs:802` の2箇所で、いずれも BUG-62 の
「かな」キー swallow 判定用である。**「Alt 保護なら既に `alt_key_held()` がある」
と誤読しないこと**——送信側には一度も配線されていない。

### 5.3 dead code だが管理対象に残すべき送信口

**2026-08-17追記（`send_f2_via_sendmessage` は撤去済み）:** BUG-67
（`docs/known-bugs.md`）の調査で `VK_DBE_HIRAGANA` の合成送信箇所を
棚卸しした際、本項が dead のまま残すべきとしていた `send_f2_via_sendmessage`/
`send_f2_via_sendmessage_async` を実際に削除した。「復活しうる送信口として
管理対象に残す」という当時の判断を覆す新しい理由があったわけではなく、
今回の棚卸しの機会に合わせてユーザーが撤去を選択した。以下の表・
「単一関門の唯一の例外」の記述は削除前の状態の記録として残す。

| 場所 | 関数 | 状態 |
|---|---|---|
| ~~`ime.rs:1419`~~ | ~~`send_f2_via_sendmessage`~~（撤去済み） | **呼び出し元は「ゼロ」ではなく1つ**（`ime.rs:883`、`send_f2_via_sendmessage_async`（宣言 `:880`）が `offload_unsafe` 経由で呼ぶ）。**その async ラッパー自身の呼び出し元がゼロ**なので、結果として実質 dead code である（引き継ぎ時点の要約「呼び出し元ゼロ」は1段浅い観測だったので訂正）。~~**復活しうる送信口として管理対象には残す**~~ |
| `ime.rs:290` / `:303` / `:312` / `:320` | `post_ime_on_direct` / `post_ime_off_direct` / `post_gji_ime_on` / `post_gji_ime_off` | production 呼び出し元ゼロ。**「テストのみ参照」も厳密には誤り**（訂正）——`tests/architecture_guard.rs:866/882/902/907` は `extract_fn_body(production, "pub unsafe fn post_ime_on_direct(")` で `src/ime.rs` を**テキストとして走査**しているだけ、`tests/ime_key_sequence_golden.rs:75/76/83/88` と `state/key_sequence_policy.rs:110/113/114` はコメント中の言及のみ。**コンパイル上のリンクは一切存在しない。** それでも削除してはならない: `tests/architecture_guard.rs:859` の `ime_open_close_functions_send_expected_vk_codes()` がこれらの**本体テキスト**を検査対象にしており、削除するとエントリ01（5日間6回反転）の回帰検知が消える |

**`send_f2_via_sendmessage` は §5.2 発見4（単一関門）の唯一の例外でもある**:
この関数は `SendMessageTimeoutW(WM_KEYDOWN/WM_KEYUP)`（`ime.rs:1435` / `:1449`）で
キーメッセージを直接 wndproc へ届けるため `send_input_safe` を通らない。
**dead であるうちは実害が無いが、復活させると INV-36 の「単一関門」が破れる。**
§5.5 の `architecture_guard` は `SendMessageTimeoutW` も判定条件に含めること。

### 5.4 IMC 直接書き込み経路（VK 不使用、修飾キー汚染の対象外）

IMC / TSF 経由の直接書き込み経路も **14 種**（ラッパー関数の数）確認した
（`imm.rs:121` `send_ime_control`、`ime.rs:71` `set_ime_open_for_target`、
`ime.rs:1206` `set_ime_conv_for_target`、`ime.rs:1308`
`set_ime_open_then_conv_for_target` 等）。

**romaji 軸の IMC 経路をここに明記する（当初の抜けを補う）**:

| 関数 | 何を書くか | 呼び出し元 |
|---|---|---|
| `ime.rs:521` `set_ime_romaji_mode()` | 現在の conv に `IME_CMODE_ROMAN` を OR（→ローマ字方向のみ） | `ime_controller.rs:78`（`ImmCrossProcessStrategy::apply`、MS-IME × ON × 非 `ObservedKana`）、`ime_controller.rs:188`（`MsImeDirectStrategy::apply`、ON × 非 `ObservedKana`）。async 版 `set_ime_romaji_mode_async`（`ime.rs:782`）もある |
| `ime.rs:806` `set_ime_romaji_mode_for_hwnd()` | hwnd 指定版。`target_conv` が `Some` ならその値をそのまま設定 | `set_ime_conv_for_target`（`ime.rs:1206`）/ `set_ime_open_then_conv_for_target`（`ime.rs:1308`）の実体 |
| （撤去済み）`set_ime_romaji_mode_state` / `_for_target` | ローマ字 ⇔ JIS かな の**双方向**切替 | **BUG-61 で撤去済み**（tray の「ローマ字」「かな」コマンドが唯一の呼び出し元だった）。撤去記録は `ime.rs:1754-1760` のコメント |

**書き込みの実体は2箇所しかない**: `send_ime_control(ime_wnd, IMC_SETOPENSTATUS, …)`
（`ime.rs:87`）と `send_ime_control(ime_wnd, IMC_SETCONVERSIONMODE, …)`
（`ime.rs:504`、`modify_conv_mode` の中）。14 種はいずれもこの2箇所へ収束する
ラッパーである。

これらは **VK を送らないため修飾キー汚染の対象外**である——この事実は
「修飾キーが危険なら IMC write に寄せればよい」という誘惑を生むが、
**BUG-25 で IMC write が mozc の TIP では UI 表示専用の一方向ミラーであり実
コンポーザに伝播しないことが実測済み**であり、逃げ道にはならない（ADR-084 §3 案1）。

### 5.5 `architecture_guard` の設計案（未実装）

**ファイル名ベースの許可リストにしない。** 代わりに次の2条件の**同一関数ブロック
内での同居**で「送信口」と判定する:

1. **VK 値**が現れること — 定数名（`VK_KANJI` / `VK_IME_ON` / `VK_IME_OFF` /
   `VK_DBE_*`〈`VK_DBE_ROMAN` / `VK_DBE_NOROMAN` を含む〉/ `VK_KANA`）**および
   生の16進**（`0x19` / `0x16` / `0x1A` / **`0xF0`〜`0xF6`** / `0x15`）の両方を
   対象にする。
   **`0xF5`/`0xF6` を必ず含めること（当初 `0xF0`〜`0xF4` 止まりだったのを訂正）**:
   この2キーは romaji 軸を切り替え、BUG-61 で復旧不能と確定している——
   awase 側から**送る**経路は現時点で存在しない（`tray_inject_romaji_mode_vk` は
   BUG-61 で撤去済み）が、だからこそ「誰かがまた送ろうとしたら検知する」ことに
   価値がある。実際 BUG-61 の対応・第1段では一度この注入を実装しており
   （`Runtime::tray_inject_romaji_mode_vk`）、再導入ループが既に1周している。
   なお `vk.rs:156` の `may_change_ime()` は既に `0xF0..=0xF6` を範囲としており、
   本リポジトリの既存分類と整合する。
2. **INPUT 組み立て/送信関数**が現れること — `make_key_input_ex` /
   `make_tsf_key_input` / `KEYBDINPUT` / `send_input_safe` /
   `SendMessageTimeoutW`。

判定した送信口の集合を、テスト内の許可リストと **集合として完全一致**で比較する
（件数だけの比較や部分一致にしない。ADR-087 §5 Phase 3 item14 で
`EXPECTED_TOTAL=5` が doc コメント中の同名文字列を1件誤カウントしていた前例が
ある）。既存の `extract_all_balanced_blocks` / `production_code_only`
（`tests/architecture_guard.rs:192` / `:111`）がそのまま使える。

**原理的に閉じられる根拠と、その数え上げ結果**: §5.2 の発見4のとおり、実際の
VK 送信は `win32::send_input_safe`（定義 `win32.rs:95`、生の `SendInput(` は
その本体 `win32.rs:99` ただ1箇所）という単一の関門を通る。したがって
`send_input_safe` の呼び出し元を数え切れば送信口の集合は閉じる。
**本 ADR 起草時に数え上げた結果は 17 箇所**（`bf8727ac` 時点。当初「18 箇所」と
書いていたのは誤りで、下表を数えると 17 行分にしかならない——
`ime.rs:1695`（送信行）と `:1667`（`toggle_caps_lock` の定義行）を別々に数えて
いた。`grep -rn "send_input_safe" crates/awase-windows/src/` の結果から定義行
`win32.rs:95` を除くと 17 件で下表と一致する）:

| ファイル:行 | 備考 |
|---|---|
| `lib.rs:344` | `reinject`（§5.1 #6） |
| `ime.rs:267` | `post_kanji_toggle_to_focused`（#1） |
| `ime.rs:393` | `send_ime_mode_key`（#2） |
| `ime.rs:1695` | `toggle_caps_lock`（定義 `:1667`）— **IME モード VK ではないが、`docs/experiments.md` エントリ05 の CapsLock 汚染と同じグローバル状態を触る。能力表化の際は「軸外の状態を触る送信口」として別枠で扱うこと** |
| `hook.rs:287` | `inject_alt_menu_mask`（定義 `hook.rs:282`。BUG-62 追補2 の `SC_KEYMENU` マスク用ダミー Ctrl 注入。追補4 で `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` の swallow 分岐〈`hook.rs:793`〉からも呼ばれるようになった）— **修飾キーそのものを注入する唯一の送信口**。§6 のポリシーを決めるときはここが相互作用の中心になる |
| `tsf/send.rs:35` | `send_vk_dbe_hiragana_pair`（#3） |
| `tsf/output.rs:179` | TSF 経路の文字送信 |
| `output/mod.rs:359` / `:372` | `send_unicode_cold_warmup_keys`（#5、VK_IME_ON ペアと犠牲キー VK_A+VK_BACK で2回） |
| `output/probe_io.rs:195` | `send_chrome_gji_reinit_and_poll`（#4） |
| `output/vk_send.rs:69` | 文字送信（ローマ字 VK） |
| `output/key_injector.rs:112` / `:128` / `:196` / `:215` | 文字・犠牲キー注入 |
| `runtime/key_pipeline.rs:1506` | `kp_restore_kana_from_half_width`（#7） |
| `runtime/mod.rs:1462` | `send_all_modifier_key_ups`（定義 `runtime/mod.rs:1414`）— **全修飾キーを強制解放する送信口。§6 のポリシー表を作るときの既存資産であり、無関係な文脈で発火しないかの確認対象でもある** |

これに `send_input_safe` を通らない `ime.rs:1419` `send_f2_via_sendmessage`
（`SendMessageTimeoutW`、§5.3。実質 dead）を加えた **18 箇所**が、INV-36 が
集合として固定すべき対象である。

**したがって「送信口の集合は閉じている」ことは論証済みである**（§6.5 item2 の
前提はこれで満たされた）。残る未確定は「各セルをどう保護するか」（§6.3）だけで
ある。

---

## 6. 修飾キー汚染ハザード — 収束しなかった記録（トラック B）

### 6.1 何をやったか

トラック A の設計（§2）に「修飾キー（Ctrl / Shift / Alt / Win）が押されている
最中に IME モード VK を送ると何が起きるか」というハザードを組み込むため、
Fable（レビュアー）× Opus（設計者）の pre-mortem 往復を**追加で5ラウンド**
実施した。

### 6.2 なぜ収束しなかったと判定したか

**各ラウンドで新しく発見された「具体的な破綻シナリオ」の件数が下げ止まった。**

| ラウンド | 新規の具体的破綻シナリオ |
|---|---|
| round1 | 5 件 |
| round2 | 4 件 |
| round3 | 3 件 |
| round4 | 3 件 |
| round5 | 3 件（**加えて、棚卸し漏れの新しい送信口が発覚**） |

トラック A（§2）は round5 で新規シナリオが尽き、レビュアーが CONVERGED 判定を
出した。トラック B は **3件で下げ止まり、しかも round5 に至っても設計の前提
（送信口の集合）が閉じていなかった**。この状態で「決定」を書くと、
実装した瞬間に未知の送信口が反例になる。

**この「下げ止まり」自体が本 ADR の成果物である**: 新規シナリオが単調減少して
ゼロに向かうなら机上の往復を続ける価値があるが、3件で平らになったということは
**残りのシナリオは机上では潰せない（実機の挙動を知らないと決められない）**
ことを意味する。トラック D（§7）が中断した以上、往復を6ラウンド目に進めても
同じ3件が形を変えて出続けるだけである。

**この表の限界（正直に書く）**: 上表は件数だけで、**各ラウンドで挙がった
シナリオそのものの本文は本 ADR に1件も引き継がれていない**（トラック A 側の
S23〜S26 だけが §3.1 に残っている、§ステータス参照）。同様に、round5 で
「棚卸し漏れの新しい送信口が発覚した」と書いてあるが、**それが §5.5 の表の
どの行だったかは引き継ぎ時点の要約に残っておらず、本 ADR 起草セッションからは
復元できなかった**。§5 の棚卸しは本 ADR 起草時に `send_input_safe` の呼び出し元を
機械的に数え直す形でやり直しており（結果 17 箇所 + `SendMessageTimeoutW` 1 箇所）、
**round5 時点の「漏れ」がその 18 箇所に含まれていることは確かだが、どれかは
特定できない**。したがってこの表は「収束しなかったという判定の根拠」としては
読めるが、「何が未解決か」の目録としては使えない——後者は §6.3 の4つの問いを
参照すること。再着手時に round1〜5 のシナリオ本文を探しにいかないこと（存在
しない）。

### 6.3 何が決まっていないか

**各送信口 × 各修飾キーのセルを shield / skip / defer / そのまま のどれにするか、
というポリシー表が未確定。** §5.1 の表は「現状こうなっている」の記述であって、
「こうあるべき」ではない。

未決の具体的な問い（すべて実機実測が必要）:

1. **Alt 押下中の `VK_IME_ON` / `VK_IME_OFF` は、Alt+ショートカットとして解釈
   されるか。** これが決まらないと §5.2 発見1（`post_kanji_toggle_to_focused` は
   Alt を解放し、`send_ime_mode_key` は解放しない）のどちらが正しいかを決められない。
2. **無保護の4経路（#3 の Ctrl/Shift/Alt、#4、#5、#6）で、Ctrl を押しっぱなしに
   したまま送信が起きると何が起こるか。**
3. **`prepend_synthetic_shift_up`（#7 の合成 Shift KeyUp）のバッチ順序は OS の
   キューで保たれるか。** ADR-048 は「同一 `SendInput` バッチは連続キューに
   積まれる保証がある」としており、#7 の合成 Shift KeyUp と
   `VK_DBE_HIRAGANA` ペアは実際に同一バッチ（`key_pipeline.rs:1493-1506`）に
   積まれている。**したがってこの問いは ADR-048 の主張が正しい限り理論的には
   決着しているが、`VK_SHIFT` の KeyUp が「IME に対して」意図どおり先行して
   解釈されるか（キューの順序と IME 側の処理順序が一致するか）は別問題であり
   未実測である。**
   なお引き継ぎ時点では「#7 はリトライ（0/160/320/480ms）に分かれるためバッチ
   保証が及ばない」という懸念が挙げられていたが、**これは誤りだった**——
   リトライの対象は IMC conv write であり VK 再注入ではない（§5.1 補足）。
4. **Win skip の経路ごとの非対称（§5.2 発見2）に実害はあるか。** ADR-061 が
   Win skip を入れた理由（Win+A でスタートメニューが開く）が、#4/#5/#6 でも
   同様に起きるのか。

### 6.4 既知の実害（決まっていないことの重み）

このポリシー表が未確定であることは「まだ手を付けていない領域」ではなく、
**既に実害が出ている領域**である:

- **BUG-62**（§1.2(c)）: 物理 Alt + かな → ドライバが `VK_DBE_ROMAN`/
  `VK_DBE_NOROMAN` を合成 → JIS かな固着 → BUG-61 で**復旧不能と確定**。
  修飾キー × IME モードキーは awase が復旧できない状態を作れる。
  **現在は対策済み**（`hook.rs:793` の swallow、`259aeaed` で実機再検証
  「再発しない」）。ここで引用しているのは**残っている実害ではなく、
  「どこを直せばよいかを当てるのが難しい」という探索コストの実証**である。
- **対策を3回連続で外している**（BUG-62 追補1〜3 → 追補4）。追補1〜3 は
  `VK_KANA`(0x15) だけを見ており、実機ログを直接読むまで「その分岐は打鍵ごとに
  常時 swallow されており今回の症状とは無関係だった」ことに気づけなかった
  （`docs/known-bugs.md` 追補4）。**修飾キー × IME モードキーの領域では、
  コード読解と履歴の二分探索だけでは真因に届かない**ことの実証である。
- **BUG-62 追補2/3**: その対策（swallow）自体が `SC_KEYMENU` を起動させ、
  「Alt+かな の後に入力不能」という別の不具合を生んだ。追補3 はこの症状の
  原因コミットを `git bisect` で `b38d67f8`（2026-07-05）と特定したが、
  **その修正でも症状は再発した**（追補4 の冒頭）——`git bisect` が当てたのは
  「いつから壊れたか」であって「何が壊しているか」ではなかった。
  **対策の投入から発覚までに約1ヶ月かかっている。**
- **`docs/experiments.md` エントリ05/07**: IME モードキーの scan が物理
  CapsLock 位置（0x3A）と共有されているため、IME が処理しない文脈では
  CapsLock を汚染する。**2回、別々のセッションで同じ失敗を踏んでいる。**

### 6.5 再着手するときの出発点

1. **§5.5 の `architecture_guard`（INV-36）を先に入れる。** ポリシーが決まる前でも
   「送信口が増えたら検知される」状態は作れる。round5 で棚卸し漏れが発覚した
   のと同じことが実装中に起きるのを防ぐ。
2. ~~`send_input_safe` の全呼び出し元を数え切る~~ → **本 ADR 起草時に実施済み
   （17 箇所、§5.5 の表。`SendMessageTimeoutW` 経路 1 を足して計 18 送信口）。
   送信口の集合は閉じている。**
   このとき見つかった2つの「修飾キーそのものを触る送信口」——
   `hook.rs:287` `inject_alt_menu_mask`（ダミー Ctrl 注入）と
   `runtime/mod.rs:1462` `send_all_modifier_key_ups`（全修飾キー強制解放）——は、
   ポリシー表を作るときの既存資産であると同時に、**ポリシーと相互作用して新しい
   汚染を生みうる箇所**でもある（BUG-62 追補2 がまさにその実例）。
3. **§6.3 の4つの問いのうち1つだけを実機で測る。** 全部を一度に決めようとした
   ことが5ラウンドの往復が収束しなかった一因である。もっとも実害が大きいのは
   問い1（Alt × `VK_IME_ON`/`OFF`）——§5.2 発見1 の矛盾がそこにあり、
   BUG-62 の family でもある。
4. **`.claude/rules/experiment-logging.md` に従い、1問1コミットで測定結果を残す。**

---

## 7. 実機実測トラックの中断記録（トラック D）

**なぜこれを書くか**: 将来同じ手法（合成キー入力による IME 実機自動テスト）を
再試行したときに、同じ壁に何も知らないままぶつからないため。
`.claude/rules/experiment-logging.md` の「失敗の証拠を捨てない」という規約を、
コード変更を伴わない実測の試みにも適用する。

### 7.1 何をやろうとしたか

§6.3 の問いに答えるため、Windows 実機上で修飾キーを押した状態を作り、その状態で
IME モード VK を送って結果（IME の open / charset / romaji、CapsLock 状態、
`SC_KEYMENU` の発火有無）を観測しようとした。

### 7.2 何が起きたか

**合成キー入力が「API は成功を返すが実際には届かない」という原因不明の無効化を
起こした。** `SendInput` / `keybd_event` / `WH_JOURNALPLAYBACK` の3方式すべてで
同じ現象が出た。

投入経路も2通り試した:

- **clipwire 経由**（このサンドボックスから Tailscale 越しに Windows 側で
  コマンドを実行する経路）
- **ユーザーが Windows 側で直接操作**

**どちらでも再現した。**

### 7.3 検証した仮説と、それぞれの結末

| 仮説 | 結末 |
|---|---|
| clipd（clipwire の実行側）の実行コンテキストが原因（サービス／別セッションで動いており、対話セッションへ入力を送れない） | 完全には否定も肯定もできず。ユーザー直接操作でも再現したことと整合しない |
| awase 自身のフック（`WH_KEYBOARD_LL`）が干渉している | awase を止めても改善せず（＝主因ではない）。ただし完全な切り分けには至らず |
| ウィンドウステーション／デスクトップの分離（`WinSta0\Default` 以外で動いている） | 有力だが確証は取れず |
| メッセージポンプの方式（テストハーネス側がメッセージループを回していない） | 一部の方式では関係しうるが、全方式で同じ症状が出たことを説明できない |

**いずれも完全には特定できなかった。ユーザーの判断でこの実測トラックを中断した。**

### 7.4 記録すべき教訓

**P/Invoke（および同等の Win32 直接呼び出し）ベースの IME 実機自動テストは、
原因不明の理由で「API は成功するが入力が届かない」状態に陥りうる。**

この脆さ自体が、今後の CI / テスト基盤の設計における前提条件である:

- **「実機自動テストを整備すれば §6 のポリシー表を機械的に決められる」という
  計画は立てないこと。** 実機自動化そのものが未解決の課題である。
- ADR-087 §8 が採った方針——**純粋関数として切り出して Linux 上で全数テストする**
  （`issue_open_warrant()` の Step 0〜4 を独立オラクルとの 4608 通り網羅比較で
  固定した）——は、この制約下での正しい回避策である。トラック A の実装も同じ
  方針を採るべきである（§8 Phase 1）。
- 逆に、**「修飾キーが押されている最中に VK を送ると OS が何をするか」は純粋関数
  に切り出せない**。ここがトラック A（純粋ロジックに落とせる）とトラック B
  （落とせない）の本質的な違いであり、**トラック B が収束しなかったことの構造的な
  理由**でもある。
- 代替手段の候補（いずれも未検証）: 実機での手動観測 + `docs/experiments.md` への
  記録（従来どおり）、`cargo-ci-gcp-spot-instance` の Windows self-hosted runner 上
  での対話セッション付き実行、GUI 自動化ツール（AutoHotkey 等）の外部プロセス化。

---

## 8. 移行計画（トラック A のみ。トラック B は計画を立てない）

**トラック B（修飾キーポリシー）は §6.5 の出発点だけを残し、Phase を切らない。**
未確定の設計に Phase 番号を振ると「Phase が切ってあるから着手してよい」と
誤読される（ADR-081 Phase 1d / ADR-078 Phase 1 が「起票したが実機制約で完走
できなかった」前例を2件持つ、ADR-087 §1.5.1）。

各 Phase は独立してリリース可能で、後の Phase が実機で否定されても前の Phase は
残る。

### Phase 0（記録のみ、Linux で完結、コード変更なし）

1. 本 ADR を `docs/adr/088-*.md` として追加し、`docs/adr/index.md` に登録する
   （**本セッションで実施**）。
2. `docs/known-bugs.md` の BUG-50 に「**デッドロック（原因1）は解消済み。
   残る原因2〈トリガー〉は ADR-088 の対象外だが、charset 軸の所有権ゲートは
   ADR-088 `CharsetOwner` が担う**」と追記する。**未実施。**
   ——「BUG-50 の恒久対応方針は ADR-088」という書き方をしないこと。BUG-50 が
   今なお開いているのは原因2（トリガー未確定）だけであり、`CharsetOwner` は
   それを解決しない（§1.5）。
3. `.claude/rules/ime-belief-architecture.md` に「standing intent と
   `CharsetOwner` は `reduce()` へ何も書かない（belief ではなく actuation 側の
   状態である）」旨を明記する。**未実施。**
4. `docs/experiments.md` に、トラック B が収束しなかったこと・トラック D が
   中断したことを1行ずつ追記する。**未実施。**

### Phase 1（`AxisCapability` + `CharsetOwner` の純粋ロジック、Linux で完結）

5. `AxisCapability` を `state/app_ime_policy.rs` に新設し、`AppImePolicy` から
   派生させる（INV-29）。`MsIme × romaji` 行は **`write = SideEffecting`** とし、
   §2.4 運用規則2 の3ブロックの注記（→ローマ字方向は生きている / 復旧方向は
   BUG-61 で否定済み / JIS かな方向は OS ドライバが送ってくる）を置く。
   **`Impossible` にしないこと**（INV-33 の適用対象の訂正を参照）。
6. `CharsetOwner` を `state/conv_mode.rs` の `ConvModeAuthority` の**隣に**新設
   する（INV-31: 統合しない）。§2.5 の遷移表 T1〜T4 を実装する。
7. warrant 発行を `allows_conv_mutation() && CharsetOwner` の AND へ拡張する
   （INV-30）。**fresh charset intent はディスパッチ元から直接渡す**
   （§2.8。`IntentStore` は経由しない）。
8. in-flight 窓（§2.6、INV-34）を実装する。既存の
   `SuppressEngineStateKeyGuard`（`platform.rs:51-59`）が RAII の前例。
9. **ADR-087 §8 と同じ方針で、独立に書いたオラクルとの網羅比較テストを書く**
   （§7.4）。`CharsetOwner` の遷移は 3 状態 × 4 遷移条件 × charset 5 値なので
   全数列挙が可能である。

**この Phase は純粋ロジックのみで、ランタイムへの配線を含まない。**

**ゴールの訂正（既に解消済みの問題を再ゴール化しない）**: 当初「BUG-50 の
デッドロックが構造的に起きえないことをテストで示す」と書いていたが、**BUG-50 の
デッドロック（原因1）は 2026-08-06 の `VK_DBE_HIRAGANA` → `VK_IME_ON` 移行で
既に解消済みである**（§1.5、`docs/known-bugs.md` BUG-50 追補）。既に消えた
デッドロックを Phase のゴールに据えると、**達成済みの状態を「本 Phase の成果」と
誤って主張する**ことになる。

本 Phase の正しいゴールは次の2点:

1. **`CharsetOwner::User` の間、awase 側の自動経路（`DirectRead` /
   `Corroborated` / `SingleIndirect` / `HeuristicGuess` / `OwnSsot` / standing
   intent、および ADR-085 の force-write）が charset を書けないことを、全数
   列挙テストで固定する。** これは現在**存在しない**保護である
   （`allows_conv_mutation()` は engine ON 中は常に true、§1.5）。
2. **fresh `ExplicitUserIntent` と SafetyValve は `CharsetOwner::User` 中でも
   必ず通ることを固定する**（P18 / INV-32。これが破れると復旧操作そのものが
   体感無効になる、S24）。

**BUG-50 の残存部分（原因2＝なぜ最初にカタカナへ入ったか、仮説A〜C）は本 Phase の
対象外である。** `CharsetOwner` はトリガーを特定しない。

### Phase 2（standing intent の3軸化）

10. `RecordedTargetIntent`（`state/intent_store.rs:47`）を open だけでなく
    charset / romaji も保持できるよう拡張する（§2.8）。standing intent のみが
    これを必要とする。
11. conv 側の観測レコード（idle-conv-check 系）に取得元 hwnd を必須化する
    （INV-35）。open 軸の `ImeObservation` は既に `hwnd: HwndId` を持つため
    対象外。

### Phase 3（配線、実機ソーク必須）

12. Phase 1 のロジックを実際の actuation 経路へ配線する。ADR-087 Phase 3
    （`OpenWarrant` の配線）と**同じセッションで行わないこと**——open 軸と
    charset 軸の副作用を切り分けられなくなる（ADR-086 §ステータスが Phase 2/3 の
    ソークを別セッションに分けた理由と同じ）。
13. §3.3 の (i)(ii)(iii) を、実害コメントごと `docs/known-bugs.md` へ転記して
    から撤去する。

**実測義務**（`.claude/rules/tuning-constants.md`）: §2.5 T1 の「直近 awase 書き
込みの confirm 窓」は新しいタイミング定数を要する。**「`actuate_conv_mode` の
書き込み完了から、その値が観測に現れるまでの実測 ms」を Windows 実機で計測し、
コミット本文に記載すること。** ADR-084 §5 Phase 1 が既に同じ実測を要求しており
（`MS_IME_READY_CONFIRM_MS` の 400ms は IME ON/OFF 遷移の実測であって conv 書き
換えの実測ではない）、**この実測はまだ行われていない**。既存値を流用しないこと。

### revert する場合の義務

`.claude/rules/experiment-logging.md` に従い、本 ADR 由来の変更を revert する
コミットは本文に **アプリ / IME（種別と状態）/ 再現手順と症状** を必ず記載する。
本 ADR が対象とする領域（`ime.rs` / `platform.rs` / `state/conv_mode.rs` /
`output/conv_actuation.rs` / `runtime/conv_actuation.rs`）は同ルールの適用範囲に
明示的に含まれている。

---

## 9. 強制メカニズム

`.claude/rules/ime-belief-architecture.md` 末尾の3段構えに倣う。同ルールの判断
基準に従い、**dylint の新設は「型では防げない意味論的偽装」にのみ投資する**
（ADR-087 が同じ理由で新規 dylint crate を却下している）。

### 段1: コンパイラ

- **INV-30**: `CharsetOwner` のフィールドを private にし、遷移は
  `state/conv_mode.rs` の4メソッド（T1〜T4 に対応）からのみ行えるようにする
  （`ForceGuardSet.guards` を private 化して `clear()` を唯一の口にしたのと
  同じ手法）。
- **INV-33**: `AxisWrite::Impossible` の軸に対する書き込み要求が型として
  構築できないようにする（`AxisCapability` を通さないと actuator の引数が
  作れない形にする）。

### 段2: dylint（既存 crate の拡張のみ、新規 crate は作らない）

- `lints/observation_source_guard` を拡張し、`CharsetOwner` の遷移メソッドが
  `state/conv_mode.rs` 外から呼ばれたら warning。`lints/ime_event_guard` が
  `PanicReset` / `HwndCacheRestored` を designated 関数に限定しているのと同型。

### 段3: CI テスト（Linux で実行可能、`tests/architecture_guard.rs`）

1. `ime_mode_vk_send_sites_are_accounted_for` — §5.5 の判定（VK 値 × INPUT
   組み立て関数の同一ブロック内同居）で送信口を列挙し、許可リストと**集合として
   完全一致**で比較する。**INV-36**
2. `charset_owner_transitions_are_limited_to_four` — `CharsetOwner` を書く箇所が
   §2.5 の T1〜T4 に対応する4メソッドのみであることを固定する。**INV-30/31**
3. `conv_mutation_gate_evaluates_both_authority_and_owner` — warrant 発行点で
   `allows_conv_mutation()` と `CharsetOwner` の両方が参照されることを固定する。
   **INV-30**
4. `axis_capability_is_not_derived_from_raw_profile` — `AxisCapability` の派生
   コードに `AppImeProfile` の raw な一致判定（`matches!(profile, ...)` 等）が
   現れないことを固定する。**INV-29**（ADR-087 INV-20 の機械化でもある）

### 段4: golden / 網羅比較テスト

- **`CharsetOwner` の遷移は全数列挙が可能**（3 状態 × 4 遷移条件 × charset 5 値）。
  ADR-087 §8.6/§8.8 が `issue_open_warrant()` に対して行った「独立に書いた
  オラクルとの網羅比較」と同じ手法を適用する（§7.4 のとおり、実機自動テストが
  使えない以上これが最も強い防御である）。
- `tests/ime_key_sequence_golden.rs` は**触らない**。§5.3 のとおり
  `tests/architecture_guard.rs:859` の
  `ime_open_close_functions_send_expected_vk_codes()` が実関数本体を検査する形で
  エントリ01（5日間6回反転）を守っており、こちらが SSOT である。

---

## 10. 未解決の論点

1. **トラック B（修飾キーポリシー）全体。** §6.3 の4つの問いはすべて実機実測が
   必要で、トラック D の中断により未着手。再着手の出発点は §6.5。

2. **実機自動テストの実現手段そのもの。** §7 のとおり、合成キー入力が原因不明で
   無効化される。この問題を解かない限り、IME 制御の実機回帰テストは
   「手動 + `docs/experiments.md` への記録」以上には進めない。

3. **`AxisCapability` と ADR-081 の関係の最終形。** 本 ADR は暫定的に
   `AppImePolicy` に置くと決めたが（§1.6）、ADR-081 Phase 1d（strangler-fig
   配線）が実機ソークを経て入ったら、能力表はドライバ側へ移すべきか。
   **移すなら二重定義の期間を作らないこと**が条件になる。

4. **romaji 軸を `AxisWrite` の3値でどう表すか。** §1.1 で訂正したとおり、
   romaji は「→ローマ字方向の IMC write は本番で生きている（`ime.rs:521`）が、
   JIS かな固着からの復旧方向は実機で否定済み（BUG-61）」という**方向依存の
   非対称**を持ち、`Idempotent` / `SideEffecting` / `Impossible` のどれにも
   きれいに収まらない。暫定的に `SideEffecting` + 注記（§2.4 運用規則2）と
   したが、`AxisWrite` に方向の概念（`write_to(value)` ごとの可否）を入れるべき
   かは実装時の論点として残る。**入れない場合、注記が唯一の防壁になる**ため、
   注記を削らないこと。

5. **`CharsetOwner::User` の解除条件に TTL を設けるか。** 現在の設計（§2.5）では
   `User` からの復帰は T2（fresh intent / SafetyValve）のみで、時間経過では
   復帰しない。ADR-087 INV-24(a) が open 軸の明示意図に ON/OFF 非対称な TTL を
   設けた（`EXPLICIT_ON_INTENT_TTL_MS` / `EXPLICIT_OFF_INTENT_TTL_MS`）のと
   非対称になる。**意図的な非対称**（charset の掌握は open の意図より長く尊重
   すべき）と考えているが、実機でユーザーが「カタカナから戻れない」と感じないか
   は未検証。

6. **多軸トランザクションの適用範囲**（§2.9）。Open 軸 `read = Direct` の
   プロファイル限定という制限は収束時のものだが、実装してみると
   「charset だけを書きたいが Open 段の完了を待つ必要があるか」という順序の
   問題が残る。§3.3 (ii)（`applied` 整合スキップ → Open 段完了後の charset
   再評価）がここに関係する。

7. **本 ADR §2 の設計セッション内部番号（P1'〜P12、S1〜S22）の本文が失われて
   いること。** §ステータスに記載のとおり。復元を試みるより、本 ADR の §2/§4 を
   出発点にすること。§6.2 のトラック B 側ラウンド別シナリオ（round1〜5）も
   同様に本文が残っていない。

8. **ADR-072 の「apply 完了ごとの conv authority 再同期」を実ゲート側へ入れ直す
   必要があるか**（§1.7）。ADR-072 が解こうとした「Engine が Active のまま IME
   だけ再 apply する経路で authority が古いまま残る」問題が、実ゲートが
   `Output::conv_mutation_allowed` に一本化された今も同じ形で残っているかを
   本 ADR は確認していない。**本 ADR のスコープ外だが、INV-30 の AND の片側が
   古い値のまま固まると `CharsetOwner` を足しても意味が無くなる**ため、Phase 3
   の配線前には確認すること。

---

## 11. 関連

- [ADR-064](064-conv-mode-policy-gate.md): conv mutation ゲート
  `Output::conv_mutation_allowed`（`Cell<bool>`）の導入。**型名の帰属に注意**——
  ADR-064 当時の型 `ConvModePolicy{AwaseLocked, UserManaged}` は現存せず、
  役割は `ConvModeAuthority{Unknown, AwaseOwned, UserOwned}`
  （`state/conv_mode.rs:27`、`allows_conv_mutation()` は `:45`）に改名・3値化
  されている（`0803ac30`）。**`ConvModePolicy` という名前は現在 ADR-085 が
  `{Observe, Force}` として再利用しており（`src/config.rs:34`）、ADR-064 とは
  無関係の別物である。** 本 ADR は前者を**置き換えず AND で並存**する
- [ADR-067](067-vk-ime-on-off-migration.md): `VK_IME_ON`/`VK_IME_OFF` への全面
  移行（§2.4 能力表の「単一効果キー」の唯一の実例。**維持**）
- [ADR-072](072-conv-mode-authority-apply-resync.md): `conv_mode_authority` の
  apply 完了時再同期。**この再同期は `552414ec`（2026-07-05）で撤去済みで現存
  しない**（§1.7）。現在の駆動源は `EngineStateChanged`（`executor.rs:673` が
  唯一の書き込み点）。本 ADR は `ConvModeAuthority` 側を変更しないが、
  「ADR-072 が維持されている」とも書かない
- [ADR-078](078-ime-mode-belief-desired-effective-constraint.md): conv-mode
  belief の3分割。`CharsetOwner` が先取りするのは **`DesiredMode`**
  （`{mode, source: UserIntentSource, sequence}`、078 の 100-106 行）の
  「誰が選んだか」であって、`ModeConstraint`（アプリ都合の一時的制約、
  078 の 118-124 行）ではない
- [ADR-081](081-per-profile-capability-driver-decomposition.md): プロファイル別
  capability ドライバ（`AxisCapability` の将来の置き場所候補。**Phase 1d 未配線の
  ため今は `AppImePolicy` に置く**）
- [ADR-084](084-conv-mode-single-ownership-and-width-ssot.md): conv 単一 actuator
  と **INV-11（conv 帰属 provenance、`084-...:325`）**。INV-11 は
  `Attributed{by: awase}` / `UserOriginated` / `Unknown` の**3状態と判定規則を
  既に定義済み**であり、`CharsetOwner` はその**再命名・精緻化**である
  （新規性は目標値・掌握 tick の保持と、belief から actuation 側への置き直しの
  3点。§1.4）
- [ADR-085](085-conv-mode-force-policy.md): `ConvModePolicy{Observe, Force}`
  （`src/config.rs:34`。名前は ADR-064 の旧 `ConvModePolicy` と同じだが**別物**）。
  判定点は `Output::is_force_policy()`（`output/mod.rs:300`）。`CharsetOwner::User`
  中は force も止まる（§1.6）が、**Force は INV-30 の AND 条件ではない**
  （INV-30 の注記を参照）
- [ADR-086](086-force-write-trigger-and-target-identity.md): force-write の定義
  （§2.1）とトリガー軸／空間軸。**INV 番号空間を継承**
- [ADR-087](087-open-belief-actuation-warrant-separation.md): 根拠軸
  （`OpenWarrant`/`WarrantBasis`/`issue_open_warrant()`）。**本 ADR の直接の親。
  全面置き換えではなく軸方向への一般化**
- [ADR-061](061-win-key-ime-injection-skip.md): Win キー押下中の IME キー注入
  スキップ（§5.2 発見2 の「7経路中2経路にしか波及していない」保護）
- [ADR-048](048-sacrificial-warmup-chrome-coldstart.md): SacrificialWarmup /
  アトミックバッチ送信（同一 `SendInput` バッチは連続キューに積まれる、という
  §6.3 の問い3 の前提）
- `docs/known-bugs.md`: **BUG-50**（カタカナ復旧不能、`CharsetOwner` の発端。
  **デッドロック〈原因1〉は 2026-08-06 に解消済み、開いているのはトリガー
  〈原因2〉のみ**、§1.5）、**BUG-61**（JIS かな固着からの**復旧方向**が実機で
  否定され解決不能クローズ。romaji 軸の非対称の根拠。**「romaji は書けない」の
  根拠ではない**、§1.1）、**BUG-62**（物理 Alt+かな →
  `VK_DBE_ROMAN`/`VK_DBE_NOROMAN`。原因の特定は **Web 調査**〈`:7333`〉、
  真の引き金の特定は**実機ログ**〈追補4〉。`git bisect` を使ったのは追補3 の別
  問題の原因コミット同定であり、しかもその修正は効かなかった。**追補4/5 で修正
  済み・`259aeaed` で実機再検証済み**〈`:7517-7519`〉、§1.2(c)）、BUG-19（conv
  確定の2連続デバウンス、§2.5 T1 が流用）、BUG-25（IMC write は mozc TIP では
  UI ミラー、§5.4）、BUG-13/15/16/26/33/43/48/63
- `docs/experiments.md`: **エントリ01**（IME OFF キー 5日間6回反転、§1.3）、
  **エントリ05/07**（IME モードキーの scan による CapsLock 汚染、§1.2(b)）、
  エントリ09（GJI entry の scan=0 注入はフックにすら届かず反証）
- `.claude/rules/`: `experiment-logging.md`（§6/§7 の記録義務の根拠）、
  `tuning-constants.md`（§8 Phase 3 の実測義務）、
  `fix-requires-evidence.md`、`ime-belief-architecture.md`（§8 Phase 0 item3 で
  追記が必要）
