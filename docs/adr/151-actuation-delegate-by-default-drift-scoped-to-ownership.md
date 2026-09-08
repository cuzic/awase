# ADR-151: delegate 対象キーの actuation を belief 追随のみに倒す方向（[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)「案D」の分離・保留）

## ステータス

**将来構想として保留（未実装）。** [ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
が実装した決定（随伴 warmup の outcome gating）とは別に、同 ADR の r3
レビューが検証した「案D」を独立の構想として分離したもの。

**本ファイルの成り立ちについての注記（2026-09-08 追記）**: このADR番号
（151）は index.md 上では以前から存在し、[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
「ADR-151/152 との違い」節や [ADR-156](156-unify-deferred-execution-queues.md)
「代替案B」節が「ADR-151」として具体的な Blocker（後述）を引用してきたが、
**本文ファイル自体はどのコミットにも一度も存在しなかった**
（`git log --all --follow -- docs/adr/151-*.md` が 0 件、2026-09-08 に確認）。
index.md の1行要約と、他 ADR からの引用・[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
r3 節「検討し棄却した代替案・案D」（一次資料）を突き合わせて本文を再構成した。
**再構成であり、当時実際に検討されたすべての言い回しやニュアンスを保証するもの
ではない**——特に「決定」節以降は index.md の一行要約と ADR-149/153/156 の
引用範囲を出ない。

## 背景

[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md) の
r3 レビュー中、ユーザーから次の設計提案があった: 「GJI/MS-IME 自身の
キーマップ検出（`IntentKind::PhysicalImeKey`）で判明した IME ON/OFF キーは、
GJI 自身のネイティブ passthrough 処理に完全に委ね、awase 自身は
`SendInput` を一切送らない。actuation が必要なのは awase 自身の明示設定
（`IntentKind::SyncKey`、Ctrl+変換等）の場合のみ」。

opus-adversarial-consult（r3）による検証の結論（ADR-149 案D節から要約）:

1. 「belief 追随と actuation の分離」という切り分け自体は既存コード構造
   （`write_physical_key`/`handle_engine_activation_sync` は Win32 呼び出しを
   一切伴わず、実 `SendInput` は `GjiDirectStrategy::apply` の
   `send_ime_mode_key` 呼び出し1点に閉じている）と整合する。
2. 配線（`IntentKind` を actuation 地点まで届ける経路）は新規のグローバル
   状態なしで実現可能（`ImeModel.last_intent.source` が既にこの情報を
   保持している）。ただし `last_intent` は sticky なため、無関係な後続
   actuation まで誤って巻き込まないよう `EventSource::SelfActuated` の
   起案元文字列との二重条件が必要になる。
3. 「self-actuation の方が信頼性が高い」という非対称性は confirmability
   ではなく「状態カバレッジ」の軸で実在する——`classify_and_push`
   （`crates/awase-gji-config/src/keymap.rs`）は TurnOn 分類に
   `on_statuses` が `"DirectInput"` を含むことを要求しないため、
   「TurnOn と分類されたが、IME OFF 状態では何も起きないキー」が
   構造的に作れる。

## 決定（保留のまま）

**不採用（Blocker により保留）。**

`Applied` を詐称する（実際には送っていないのに送った扱いにする）と、
TsfNative における唯一の ON 方向救済機構
`apply_force_on_for_imm_broken`（`state/ime_actuation.rs::
force_on_attempt_allowed` が `applied` が ON 確定済みの場合に早期
return する）が**構造的に永久停止する**。belief 乖離が検知できない
どころか、救済経路自体が構造的に閉じてしまう。

さらに、この設計を採用しても随伴 eager warmup（`platform.rs` の
`send_eager_tsf_warmup` 呼び出し）は `outcome` を見ないため、送信2回が
別途残ってしまう——[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
本体の決定（随伴 warmup の outcome gating）はどちらの設計でも前提条件
であり、それを先に入れた時点で残り送信は1回になり、BUG-113 の確立済み
機構（重複 SendInput）はもう成立しない。本ADRの設計が追加で削れるのは
残り最後の1回であり、限界効用はほぼゼロと判断された。

### 再検討の条件（両方揃った時点）

1. TsfNative でも IME open 状態を読める観測手段が手に入ったとき
   （force-ON 救済の代替が用意できる）。
2. `classify_and_push` に「`on_statuses` が `DirectInput` を含むこと」を
   TurnOn 分類の必要条件として追加し、状態カバレッジの穴を塞いだとき。

いずれも現時点（2026-09-08）で未着手。

## ADR-152 との関係

[ADR-152](152-keystroke-step-source-sink-pipeline.md) は打鍵1回を
source→sink パイプラインとして再構成する、より広い構想。[ADR-156](156-unify-deferred-execution-queues.md)
「代替案B」節によれば、ADR-152 の決定3が実装されれば `pending_deferred`
等の解放条件は `StepOwnership::resolve()` の一部として再設計される見込み
だが、**ADR-152 自身が本ADRの Blocker（force-ON 救済の永久停止）を
共有しており、着手できないままである**（ADR-156 の記述）。本ADRの
Blocker が解消しない限り、ADR-152 が「belief 追随のみで actuate しない」
ステップ種別を第一級の機能として持つ設計にも同じ壁が立ちはだかる。

## 関連

[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
（本ADRの一次資料、「検討し棄却した代替案・案D」節）、
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（隣接する広い構想、
本ADRの Blocker を共有）、
[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
（方向性の違いを明記——「送らない」ではなく「送るものを変える」ため
同じ Blocker を踏まない設計）、
[ADR-156](156-unify-deferred-execution-queues.md)（本ADRを引用する
「代替案B」節）、BUG-113。
