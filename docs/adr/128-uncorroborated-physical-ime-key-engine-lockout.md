# ADR-128: 確証の弱い物理IMEキー1回でEngineが無期限ロックアウトされる問題

## ステータス

**設計検討中（ドラフトv1、レビュー前）。**

## 背景

### 直接のきっかけ: 不具合報告 `01M1MMK8987NT5B2W73PCPZNZ1`

2026-09-03、Windows Terminal + PowerShell（GJI、JIS配列）で入力中に余分な
「@」が出力されるとの報告が届いた（`docs/bug-reports-triage.md` 該当行）。
journal と app_log の時系列突合で以下が確定した:

1. `21:51:55.702`、物理CapsLock位置（scan=0x3A）から `vk=0xF0`
   (`VK_DBE_ALPHANUMERIC`、いわゆる「英数」キー) の `KeyDown` が届いた。
   `kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs`）がこれを
   「物理IMEキーによる意図的なIME OFF」と解釈し、`desired_open=false` を
   即座に確定。同時に `Engine deactivated (ime=false...)` が発火し、
   awase自身のNICOLA変換エンジンがOFFになった。
2. しかし実際のGJI変換状態（`observed`）は `open=true` のまま変化せず、
   `21:51:57.796`〜`21:52:07.312` の間 `[drift] correction: observed=true ≠
   desired=false` → `apply_ime_open(false)`（`VK_IME_OFF` 送信）が
   14回超反復し、一度も収束しなかった。
3. **`Engine activated` のログが再度出るのは `21:52:24.474`
   ——不具合報告ボタンが押される瞬間まで、約29秒間ノーマル変換エンジンが
   OFFのままだった。**
4. journalのK,Y,O,Uキー入力（`decision: PassThrough`, `state: Idle`）は
   この空白期間中（wall-clock `21:52:06.8`〜`21:52:07.5` 相当）に発生して
   おり、NICOLA変換が一切かからず生のJIS配列文字がそのまま素通りしていた
   ことと符合する。「＠」は `VK_IME_OFF`/`VK_DBE_ALPHANUMERIC` というVK
   コードの値自体が化けたのではなく、**それらのイベントが誤って
   エンジンOFFを引き起こし、その約29秒間にユーザーが押した物理キー
   （NICOLA配列では別のかなにシフトされるはずのキー）がJIS配列本来の
   文字「＠」のまま素通りした**、という間接的な因果と結論した。

### なぜ `vk=0xF0 scan=0x3A` は信頼できないシグナルなのか（既知の事実）

`docs/known-bugs.md` BUG-15 追補7（2026-07-07 実機）で既に確認済み:

> `VK_DBE_ALPHANUMERIC`（scan 0x3A = 物理CapsLock位置）は、実IMEがOFFの
> 文脈に着弾すると kbd106 の素の英数キー処理（CAPLOCK）でCapsLockを
> トグルする（実機: belief ON × 実OFF の窓でShift押下のたびにCapsLock
> 点灯）。

これはawase自身が**注入**する場合の話だが、今回の報告は**物理キーボード
からの純粋な入力**として同じ `vk=0xF0 scan=0x3A` が届いたケースであり、
BUG-15が指摘した「この特定のVK×scan組み合わせは、OS/kbd106ドライバの
副産物として実IMEの意図を伴わずに発生しうる」という不安定性の別側面が
表面化したものと考えられる。今回の報告では `config.toml` の
`keymaps=[]` によりADR-126（Caps→Ctrlプリセット）は無効だったため、
この誤配線が直接の引き金ではない——CapsLock位置の英数キー自体が
本質的に持つ不安定性が、プリセット無しでも顕在化した。

### 現行実装: 単発イベント→即時belief確定、収束保証なしのリトライ

`crates/awase-windows/src/vk.rs::shadow_effect()`:

```rust
Self::ImeOff | Self::Alphanumeric | Self::Deactivate => ShadowImeEffect::TurnOff,
```

`Self::Alphanumeric`（`VK_DBE_ALPHANUMERIC`）は他の明示的なIME OFFキー
（`VK_IME_OFF`）と全く同列に扱われ、**scanコードによる区別も、
裏付けとなる2回目の観測を待つ猶予も一切ない**。`kp_stage_shadow_ime_toggle`
はこの1回のKeyDownだけで `write_physical_key()` を呼び `desired_open` を
確定する。

一方、`runtime/ime_refresh.rs` の drift correction は `desired` と
`observed` が食い違うたびに `FeedbackPolicy::Blind`（`max_attempts=5`）で
再送を試み、`GiveUp` した後も `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`
(`tuning.rs`, 3000ms) 経過後に新しい観測があれば無条件で再武装する
（`state/ime_actuation.rs::blind_rearm_cooldown_elapsed`、BUG-68）。
**「observedが一貫してdesiredに反し続けたら、desired側を観測に合わせて
訂正する」という経路は存在しない。** 今回のケースでは実際のGJI conv
状態（observed=true）の方が終始正しく、awase側の `desired_open=false`
が誤りだったが、機構はこの可能性を考慮せず無期限にリトライを続けた。

### 関連ADR/既知バグ

- **BUG-15 追補7**（`docs/known-bugs.md`）: `vk=0xF0 scan=0x3A` の
  不安定性の先例（awase自身の注入時のCapsLock汚染）。
- **BUG-14**: 注入イベント（`event.injected=true`）を物理IMEキー意図に
  昇格させない、という既存の「証拠の信頼度を区別する」前例。今回の
  イベントは `injected=false`（純粋な物理キー）であり、BUG-14のガードは
  効かない——BUG-14が区別するのは「注入か否か」の1軸のみで、「物理だが
  VK×scanの組み合わせ自体が信頼できるか」という軸は未区別。
- **ADR-093**: `VK_DBE_*`（0xF0-0xF4）をIME専用の合成VKコードとして
  扱う基盤。この5 VKの受信を `is_japanese_ime()` の即時true更新
  トリガーに使うが、false方向への確定には関与しない、という非対称設計
  が既にある（コメント参照）。今回問題にしているのは同じ5 VKのうち
  `Alphanumeric` の **shadow_effect（TurnOff）** 側であり、
  `is_japanese_ime()` の判定とは別軸。
- **ADR-121**（実装未着手）: 物理IMEキーが effective_open と同じ方向
  （no-op）だったときに実OS状態との乖離を訂正する経路が無い、という
  **逆方向**の欠落を扱う。本ADRは物理IMEキー1回の証拠強度が低い場合に
  belief を**誤った方向へ**確定させてしまう問題であり、対象は異なるが
  隣接する（どちらも「物理IMEキー→belief更新」経路の信頼性の話）。
- **BUG-68**（`state/ime_actuation.rs::blind_rearm_cooldown_elapsed`）:
  GiveUp後の即時再武装ループを防ぐcooldownを追加した先例。今回の課題は
  その一歩先——「再武装ループ自体に総量の上限や、observed側を信頼する
  reconciliationが無い」こと。

## 問題

以下の2つが独立に存在し、組み合わさって「1回の低信頼シグナル→約29秒間の
エンジンロックアウト」という実害を生んだ:

1. **証拠強度を区別しない即時確定**: `vk=0xF0 scan=0x3A` のような、
   既知の理由（kbd106ドライバの副産物）で実IMEの意図を伴わず発生しうる
   VK×scanの組み合わせが、他の明示的なIME OFFキーと全く同じ重みで
   即座に `desired_open` を確定させる。
2. **無期限リトライ、reconciliation経路なし**: 一度 `desired` と
   `observed` が乖離すると、drift correctionは3秒間隔で無期限に
   再武装・再送を繰り返すのみで、「observedが一貫して反対し続けている
   なら、desired側の方が誤っている可能性を考慮する」という上限付きの
   安全弁が存在しない。

## 論点（レビューで検討したい選択肢）

以下はまだ確定していない設計の方向性。Opusによる敵対的レビューを経て
決定を固める。

### (A) 証拠強度の分離

`vk=0xF0`（`VK_DBE_ALPHANUMERIC`）を `scan=0x3A`（CapsLock位置）で
受信した場合に限り、`PhysicalImeKey` 意図への即時昇格を止める、または
弱める。候補:
- A-1: `scan=0x3A` の `Alphanumeric` だけ `shadow_effect()` を
  `None`（意図に昇格させない）にし、`is_japanese_ime()` の
  upgrade判定（ADR-093、false方向には関与しない非対称設計）は維持する。
- A-2: 即時確定はせず、一定時間内に裏付けとなる2回目の証拠
  （同方向の観測、または同じキーの再送）があるまで保留する。

いずれも「JIS配列でCapsLock位置＝英数キーという構造そのものに起因する
既知の不安定性」を、config非依存でどこまで一般化して直すべきかが論点。
ADR-126（Caps→Ctrlプリセット）採用者が今後この位置を頻繁に押すように
なる（＝トリガー頻度が上がりうる）ことも影響評価に含める必要がある
（ただし今回の報告自体はプリセット未使用でも再現した点に注意）。

### (B) 有界reconciliation

drift correctionが一定回数のGiveUpサイクル、または一定の総経過時間、
observedと乖離し続けたら、`desired_open` をobservedに合わせて訂正する
安全弁を追加する。論点:
- 正当な理由（ユーザーが意図的にIMEをOFFにしていて、observed側の
  ポーリングが古い/別プロファイル由来で誤っている場合等）による長時間の
  乖離まで誤って上書きしてしまわないか。
- 「訂正」ではなく「エンジンを止めたまま無期限に待つのではなく、一定時間で
  一旦ユーザーに委ねる（例: 次の明示的なIME操作まで待つ）」という
  reconciliation以外の選択肢も検討に値するか。
- ADR-121が扱う「no-op側の欠落」と対称的に、「Send側が無期限に空振りする
  ケースの上限」として一体的に設計すべきか、別ADRのままにすべきか。

### (C) 両方

AとBは独立に価値があり、片方だけでも実害を減らせる可能性がある
（Aは今回の直接原因を塞ぐ、Bは原因が別のものであっても長時間ロック
アウトという実害の再発を防ぐ一般的な安全弁になる）。両方採用するか、
優先順位を付けて段階的に採用するかもレビュー対象。

## 決定

（レビュー未実施のため未記入）

## 実装状況

未着手。
