# ADR-160: 非スコープを決定する会議体を持つ（C1: IME一本化／C2: アプリホワイトリスト化／C3: conv-mode追跡全廃）

## ステータス

**起票。[ADR-158](158-complexity-reduction-north-star.md)（北極星）の採用Cを分割・詳細化した
子ADR。C1〜C3は原則採択のみで、実施可否はユーザー確認待ちのまま未確定。2026-09-09、実機スパイク
の副産物としてC3の判断材料に予備実測を追加し、[ADR-161](161-single-source-spec-generation.md)
の実証実験から一般化した「宣言の強制とSSOT化」原則（[ADR-158](158-complexity-reduction-north-star.md)
参照）の適用可能性を将来検討として記録した（未実装）。本ADR単体としてのopus-adversarial-consult
は未実施。**

## 背景

[ADR-158](158-complexity-reduction-north-star.md)のRC1（TsfNativeでIME open状態の真値が読めず、
awaseがbeliefを持たざるを得ない構造）は、ドメインの性質として消去できない。しかしRC1が生む
複雑さの絶対量は「Win32×TsfNative×UWP」×「GJI×MS-IME×ATOK」×「cold/warm×idle×conv mode」と
いう直積に比例しており、この直積のどこかを意図的に切断すれば発生源自体を減らせる。

awaseのADR群（156本）の中に、「何をサポートしないか」を決めたADRは1本も見当たらない。
ADR-136/138のような却下はあるが、それは実装案の却下であって**要求の却下ではない**。awaseには
これまで非スコープの意思決定装置が存在せず、要求は一度も減っていない。本ADRはこの装置を持つ
こと自体を決定として採択する。

### 具体案

検討された具体案は以下の3つ。いずれも技術的には確実に成立するが、**技術判断ではなく製品判断**
であり、既存ユーザーへの実際の機能縮小を伴う。

| 案 | 内容 | 影響 |
|---|---|---|
| C1 | サポートIMEをGJI 1本に絞る（`ime_controller.rs`の4戦略ImmCross/GjiDirect/MsImeDirect/KanjiToggleを1〜2に削減） | MS-IMEユーザーを切る。`docs/experiments.md`に記録されたVK選択反転史の大半はIME間差異が原因であり、この複雑さも同時に減る |
| C2 | アプリ対応をホワイトリストに反転する（既知良好アプリ以外は明示的に無効化しトレイ表示） | 未知アプリでの動作を失う。`focus/`3,539行と学習キャッシュ、BUG-107型のプロセス間汚染が構造的に消える |
| C3 | conv mode追跡を全面放棄する（19〜20ファイル・約214箇所の読み取りと`conv_classify`/`conv_actuation`/`eisu_recovery`系を削除） | 日常操作の劣化。project memoryで既に「actuationのゲートに使わない」方針が確定済みであり、C3はその延長線上にある |

### 判断構造の設計（当初案の不備）

当初案は「別途ユーザーに個別確認の上で、それぞれ独立したADRとして起票する」とだけ記述して
いたが、これでは判断のトリガ・基準・期限のいずれも定義されず、C1〜C3が無期限に「確認待ち」に
留まれてしまう。これは[ADR-162](162-governance-reversal.md)が提案するADR TTL条項（期限内に
確定しないADRは自動クローズする）と自己矛盾する——RC4（加算のみのラチェット、決めないことを
決めてしまう）を問題視するADR群の中に、その典型例を作ってしまうことになる。本ADRでは各案に
判断材料と期限を明記することで、この不備を解消する。

## 決定

C1〜C3それぞれについて、判断材料と期限を以下のように定める。**判断材料が揃うまで、実施可否
そのものは本ADRでは決定しない**。

| 案 | 判断材料 | 期限 |
|---|---|---|
| C1（GJI一本化） | 不具合報告基盤（ADR-095）の集計でMS-IME利用者の実在規模を確認する | [ADR-162](162-governance-reversal.md)で四半期定例棚卸しが確立した時点から1サイクル以内、**かつ2026-12-31を超えない**（round4 TJ3 M1で追加） |
| C2（アプリホワイトリスト化） | 現行`focus/`学習キャッシュのカバレッジ実測と、BUG-107型プロセス間汚染の発生頻度を確認する | 同上 |
| C3（conv-mode追跡全廃） | 19〜20ファイル約214箇所の出現のうち、既に確定済みの「actuationゲートに使わない」方針を除いた残りの用途（診断ログ等）を棚卸しする | 同上 |

判断材料が揃わない、または[ADR-162](162-governance-reversal.md)の定例棚卸しサイクルが確立
しない場合、期限到来時点でC1〜C3は「見送り」として扱う。これにより、判断が無期限に先送りされる
ことを防ぐ。

**round4 TJ3 M1で訂正・バックストップの追加**: 上記「期限到来時点」は
[ADR-162](162-governance-reversal.md)のE3（四半期定例棚卸し）が確立した時点を起点に定義されて
おり、E3の確立自体がADR-159段階1/2の実績を前提とするため、E3が確立しなければこの期限は永遠に
発火しない——無期限放置を防ぐはずの条項が、無期限放置のケースでだけ無効になる自己参照だった。
**絶対日付のバックストップを追加する: 2026-12-31時点でADR-162のE3が未確立の場合、C1〜C3は
その時点で「見送り」確定とする**（E3確立を待たない）。「1サイクル以内」の語義も、暦上の3ヶ月
ではなく「棚卸しサイクル1回分の実施」を指す（round4 TJ3 S4）。

### C3の判断材料に関する予備実測（2026-09-09、[ADR-159](159-existing-io-boundary-inventory.md)の
実機スパイクの副産物）

`conv_mode`使用時の`effective_open`と、conv_modeが無かった場合のfallback式
（`shadow_on || candidate_visible || (!desired_open && candidate_was_seen)`——round4 TJ3 N2で
式を省略せず明記、`desired_open`に依存する項を含むため乖離条件の理解に直結する）の値を実際の
GJI利用セッション（Chrome/WezTerm/VS Code）で比較したところ、**乖離は1件も観測されなかった**
（初回セッションは計装の反映ミスにより無効だったため、コミット修正後の再測定セッションで確認、
詳細は[ADR-159](159-existing-io-boundary-inventory.md)「実機スパイク結果」節参照）。

**round4 TJ3 M3で訂正・分母の取り違え**: 「`conv_mode`を含むprobe読み取りは2192件」という
記述は、乖離比較の母数として引用していたが誤りだった。2192件は`WM_IME_CONTROL`の
`kind=probe`呼び出し数であり、乖離比較ログを置いた`reduce_open_belief`の呼び出し数とは
**別の母集団**である。さらに`reduce_open_belief`の実装（`ime_apply_planner.rs`）は
`conv_mode.map_or(fallback式, |conv| …)`であり、`conv_mode`が`None`のサンプルでは構造的に
乖離が発生しえない（None枝＝fallback式そのもの）。「乖離0件」の分母として本来報告すべきは
「`reduce_open_belief`の呼び出し数」と「そのうち`conv_mode.is_some()`だった件数」であり、
未計測のまま残っている。

**round4 TJ3 M4で追記・乖離が起きうる状態条件**: コード上、`desired_open=true`で乖離するのは
`conv & 0x1 == 0`かつfallbackがtrueの場合、すなわち**conv=0x10（ROMANのみ＝半角英数）または
conv=0で`shadow_on`**のときに限られる（既存ユニットテスト
`reduce_open_belief_roman_only_conv_is_not_hiragana`がこの状態をピン留めしている）。
「乖離0件」の最も素直な解釈は「当該セッション中に半角英数(ROMAN only)状態に一度も入らな
かった」であり、「conv_modeが冗長」ではない。**正式な判断材料として使うには、半角/全角キー等
で明示的に半角英数へ切り替える操作を含むセッションでの追加測定が必要**——これを欠いた測定は
「乖離条件を一度も踏まなかった」だけの結果になる。

**round4 TJ3 M2で追記・スコープの取り違え**: 上記の測定は`effective_open`という**読み取り側の
1消費者**のみを対象にしている。一方、決定表のC3定義は読み取り約214箇所に加え
**`conv_classify`/`conv_actuation`/`eisu_recovery`系の削除（書き込み側）**を含む。
`output/conv_actuation.rs`のモジュールdocが示す通り、conv-mode機構は実際に状態を書き換えて
系を「conv_modeとfallbackが一致する状態」へ引き戻す補正（`kp_stage_idle_conv_check`等）を
行っており、上記の測定はその補正が動いたままの条件下での**観測**であって、機構を削除した
場合の**反実仮想**ではない。「乖離0件」は書き込み側を削除した後の挙動を予測しない。**C3の
判断材料は読み取り側の用途棚卸しと書き込み側の実害棚卸しを別建てで進める**必要がある。

**round4 TJ3 M5で追記・C2判断材料の測定可能性**: C2の判断材料（`focus/`学習キャッシュの
カバレッジ実測、BUG-107型プロセス間汚染の発生頻度）は、いずれも**現状の計測手段では取得
できない**。不具合報告（ADR-095）のペイロードは`app_kind`（Win32/TsfNative/Uwp）のみ保持し
`process_name`/`class_name`を含まないため、フィールドデータから利用アプリの分布は出せない。
BUG-107型汚染は検出器が存在せず、症状が出て初めて人手で判明する性質のため頻度を測る手段が
ない。**材料を取るには先に計装を新設する必要があり、その新設コスト自体がC2の縮小効果に
見合うかどうかが判断の一部である**（判断材料の定義に含める）。TG1（判断材料収集タスク）は
この計装新設の要否検討から始める。

### TG1判断材料収集結果（2026-09-09）

**C1（GJI一本化）— 収集未着手・具体的にブロック**: `docs/bug-reports-triage.md`台帳には
`ime_kind`列が無く、実際の分布を得るには不具合報告のraw JSON（Cloudflare R2）を
`ime_kind`/`ime_product_name`フィールドで再集計する必要がある。本セッションでは
そのための取得手段（`bug-report-fetch`スキル等）にアクセスできず、着手できなかった。
次に着手するセッションは、まずこの取得手段の所在を確認すること。

**C2（アプリホワイトリスト化）— 収集未着手・計装新設がブロッカー（M5で既に判明済み）**:
上記の通り計装が存在しない。

**C3（conv-mode追跡全廃）— 書き込み側の棚卸しを実施**:
`grep`による静的棚卸しで、conv-modeに言及するファイルは25個（240箇所超）確認した。

**訂正（2026-09-09、opus code reviewで発見・M1）**: 当初「実際に書き込む
（`ImmSetConversionStatus`相当）経路は`output/conv_actuation.rs::actuate_conv_mode`
ただ1つに既に集約されている」「唯一の呼び出し元は`kp_shift_conv_guard_key_down`」と
記述していたが、これは誤りだった。`actuate_conv_mode`が呼ぶ
`crate::ime::set_ime_conv_for_target`自体に**5箇所**の呼び出し元がある
（`output/conv_actuation.rs:176`＝`actuate_conv_mode`自身、`runtime/key_pipeline.rs:1928`/
`:2489`/`:3079`、`tsf/warmup/cold_warmup.rs:94`）。さらに`IMC_SETCONVERSIONMODE`へ
`set_ime_conv_for_target`を経由せず到達する経路が**4系統**ある（いずれも
`ime.rs::modify_conv_mode`経由）:

- `runtime/message_handlers.rs:1308` → `set_ime_mode_for_target`
  （タスクトレイの「状態リセット」コマンド、**ユーザー向け機能**）
- `runtime/mod.rs:2072` → `set_ime_hiragana_mode_cross_process_async`（`panic_reset`、
  **ユーザー向け機能**）
- `runtime/open_chain.rs:187` → `set_ime_open_then_conv_for_target`（ImmCrossのopen+conv
  同時書き込み）
- `ime.rs:1318` → `set_ime_romaji_mode_for_target_blocking`

**実害の見積もりを訂正する**: C3実施の実害は「かな⇔半角英数トグル機能（Shift単独タップ）の
消失」1点ではなく、**タスクトレイの状態リセット・panic_reset・ImmCrossのopen+conv同時
actuationも巻き込む**。これはC3の決定表の記述「`conv_classify`/`conv_actuation`/
`eisu_recovery`系を削除」が示唆する「レガシーな余剰機構の削除」というイメージとも異なり、
**複数の現行機能にまたがる書き込み経路**である。C3を実施する場合、単なる削除ではなく、
上記9呼び出し元それぞれについて「この機能自体を廃止する」か「conv-modeに依存しない別の
実装（VK送信ベースのトグル等）へ置き換える」かの選択が個別に必要になる。読み取り側
（約214〜240箇所、診断ログ・`classify_conv_transition`によるbelief補正等が主）は、
project memoryが既に確立した「conv-modeはactuationのゲートに使わない」方針の対象であり、
書き込み側とは別に扱う。

C1〜C3のいずれかを実施する判断が下された場合、実施内容・影響範囲・移行手順は別途独立した
ADRとして起票する。

### 「宣言の強制とSSOT化」原則の適用（将来検討、[ADR-158](158-complexity-reduction-north-star.md)
参照）

[ADR-161](161-single-source-spec-generation.md)の実証実験から一般化した原則（散文ではなく
機械可読な宣言を単一の真実source-of-truthとする）を本ADRに適用する場合、上記の判断材料・期限
テーブルは現状Markdownの散文表であり、「期限が来たかどうか」を人間が読み返さない限り判定
できない——これは round1 M1（期限の起点がADR-162のE3に依存し、E3が確立しなければ判定不能）が
指摘した問題とも関係する。判断材料・期限を構造化データ（例: 各案の判断基準・期限・現在の
判断材料の値を持つ小さな宣言）として持たせ、[ADR-162](162-governance-reversal.md)のE3
（四半期定例棚卸し）がそれを機械的に読み込んで「今回判断すべき案はどれか」を報告する、という
形にできれば、期限管理を散文から切り離せる可能性がある。ただしこれはADR-159/161のような
実証実験を経ていない未検証のアイデアであり、本ADRの決定には含めない。C1〜C3の判断材料が
最初に埋まった時点（[ADR-162](162-governance-reversal.md)のE3稼働後）で、構造化する価値が
あるか改めて判断する。

## 検討した代替案

### 代替案（棄却）: C節を採択リストから外し「現時点で決定しない」と明示する

opus-adversarial-consultによる[ADR-158](158-complexity-reduction-north-star.md)のround1
レビューでは、判断材料・期限を書けないなら採択リストから外す方が誠実、という指摘があった。
しかし「非スコープを決定する会議体を持つこと」自体はコストが低く、判断材料・期限を明記
できたため（本ADRの「決定」節）、この代替案は不採用とする。

### 代替案（棄却）: C1〜C3を今すぐ全て実施する

技術的には可能だが、いずれも実際のユーザーへの機能縮小を伴う製品判断であり、判断材料
（利用実態の実測）を欠いたまま実施することは、RC2（正しさを判定できる装置が実機にしか
ない）と同種の性急さになる。判断材料を集めてから決めることとし、今すぐの実施は不採用とする。

## 今後の議論

1. C1の判断材料（ADR-095集計）を実際に取得する手順を確認する。
2. C2の判断材料（`focus/`学習キャッシュのカバレッジ実測、BUG-107型汚染の発生頻度）を計測する
   方法を確認する。
3. C3の判断材料（conv-mode読み取り約214箇所の用途棚卸し）に着手する。
4. [ADR-162](162-governance-reversal.md)の四半期定例棚卸しサイクルが確立した時点で、上記
   判断材料をレビューし、C1〜C3それぞれの実施可否をユーザーとともに確定する。
5. 判断材料・期限テーブルの構造化（宣言の強制とSSOT化原則の適用）を、C1〜C3の判断材料が
   最初に埋まった時点で改めて検討する（「決定」節の該当サブセクション参照）。

## 関連

[ADR-158](158-complexity-reduction-north-star.md)（北極星、本ADRの親。「設計原則: 宣言の
強制とSSOT化」節）、
[ADR-161](161-single-source-spec-generation.md)（原則の実証実験元）、
[ADR-162](162-governance-reversal.md)（判断期限の基準となる四半期定例棚卸しサイクルの
提供元）、
`docs/bug-reports-triage.md`（不具合報告基盤ADR-095の集計対象）、
project memory `feedback_conv_mode_unreliable_dont_gate_actuation_on_it`
（conv-modeをactuationのゲートに使わない、という既に確定した方針。C3はこの延長）。
