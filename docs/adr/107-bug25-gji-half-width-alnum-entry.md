# ADR-107: BUG-25 GJI 半角英数 entry の実現機構（自己注入の識別・修飾キー文脈・一度きりのトグル）

## ステータス

**提案（未実装、2026-08-27）。** [`docs/known-bugs.md`](../known-bugs.md) BUG-25 追補4（同日、mozc
ソース調査 + 6経路の実機検証）を受けて、3回撤回されている GJI 向け entry 機構を4回目として
どう作るかを定める。

本ADRは追補4の結論を**全面的には引き継がない**。追補4が特定した真因
（`transport::PhysicalKeyDisposition::plan` の `dbe_mode_key_policy=Suppress`）は
**経路5（外部プロセスからの unmarked `SendInput`）を完全に説明する**が、
**追補1・追補3（awase 自身の注入）の失敗は構造的に説明できない**ことが、本ADRの
作成過程で `hook.rs` のソース照合により判明した（後述「原因B」）。この訂正により、
最優先の実装案は追補4が示唆した「transport にバイパスを設ける」ではなく、
**「awase 自身の注入形態を、既に本番で動いている `ime::send_ime_mode_key` と同じ形に
揃え、かつ修飾キー文脈を補正する」**（決定2）に変わる。transport バイパス（決定3）は
決定2が実機で不発だった場合にのみ実装する条件付き決定として完全設計だけ与える。

**追記（2026-08-27、決定0 の実機計測結果。[`docs/known-bugs.md`](../known-bugs.md)
BUG-25 追補5）:** M1（`none`/停止）・M2（`none`/起動）・M3（`ime_kanji`/起動）の
3セルを実機で確認した。**M3 は成功**（2回連続の再トグルも成功）——決定2が
transport バイパス無しで有効であることを実機で裏付けた。ただし計測中に
**決定0 が想定していなかった新しい構造的欠陥**を発見した:
`SendInput` で送った `VK_DBE_ALPHANUMERIC` の **KeyUp が awase 自身の低レベル
フックに一度も届いていない**（KeyDown は毎回届く）。DOWN/UP を同一バッチで
送っても、別々の `SendInput` 呼び出しに分割し実時間50ms空けても同じ結果で、
タイミングは原因ではない。実害として (a) 成功直後に続けて打鍵すると1文字目
だけ全角英数になるレース、(b) 短時間の連続トグルで awase 自身の drift
correction が介入したと見られる予測不能な状態遷移（一瞬全角英数→IME オフ・
直接入力、タスクトレイの手動操作でのみ復旧）を観測した。詳細・生ログ・
再現条件は BUG-25 追補5 を参照。**決定2 の実装には KeyUp 欠落を織り込んだ
追加のセーフガード（settle 待ち・連続発火の最小間隔）が必要**という形で
下記の決定2/決定9に反映した。

**さらに追記（同日、M4 実施——原因B を実機で確定）:** `ime_kanji`/起動/
Shift 押下中はひらがなのまま変化せず**失敗した**。ログを確認したところ、
M1〜M3 では毎回記録されていた `[hook] IME-mode vk=0xF0 down self_injected=
true ... extra=0x4B45594A` が M4 では**一度も記録されなかった**——この
debug ログは `ImeKeyKind::from_vk(0xF0)` が `Some` を返す限り
`self_injected` 判定より前で無条件に発火するため、ログに出ないことは
「イベントが awase を含むいかなるフックにも到達していない」ことを意味する。
原因B は当初の仮説（GJI が `Shift+Eisu` を未定義として無視する）より
根深く、**Shift 同時押下時は `VK_DBE_ALPHANUMERIC` の KeyDown 自体が
OS レベルで配送されない**らしいことが確定した（正確な OS 側メカニズムは
未確認、後述「未解決の疑問」7 参照）。これにより**決定2 の synthetic
Shift↑ 前置は「望ましい」ではなく entry が構造的に成立するための必須要件
である**ことが実機で確定した。詳細は BUG-25 追補5 を参照。

## コンテキスト

### これまでの経緯 — 同じ機能で3回の撤回

| 追補 | 日付 | 試した entry 経路 | 結果 |
|---|---|---|---|
| 追補1 | 2026-07-11 | scan 付き `VK_DBE_ALPHANUMERIC`（`send_vk_dbe_alpha_warmup` 流用、scan=0x3A） | CapsLock 汚染。`[hook] IME-mode vk=0xF0` ログが一度も出ず |
| 追補2 | 2026-07-11 | IMC write 一本化 | 「読み返すと成功」の偽の成功。mozc の compartment write は UI ミラーで実コンポーザに届かない |
| 追補3 | 2026-07-11 | scan=0 `VK_DBE_ALPHANUMERIC`（`make_key_input_ex(.., TSF_MARKER)`） | 同じく `[hook]` ログ不出現。**GJI では entry を一切試みない**方針へ後退（`toggle_entry_supported = active_ime_kind == MicrosoftIme`） |
| 追補4 | 2026-08-27 | 6経路（OnMenuSelect×2 / PostMessage×2 / SendInput×2） | 経路6（`SendInput` scan=0、**awase 停止中**）のみ成功 |

追補3が「entry が機能しないまま `half_width_alnum_toggle_active` を立てると、生ローマ字キーが
GJI の未切替のひらがな変換エンジンへ素通しされ**かな入力が壊れる**」という新規の実害を
生んだことは、本ADRの設計制約として最重要である。**機構が実証されるまで機能自体を
無効化する方が安全側**という追補3の判断は本ADRでも維持する（決定7）。

### 追補4が確定させたこと（そのまま引き継ぐ事実）

1. **アクチュエーションの唯一の確認済み経路は `SendInput` の
   `VK_DBE_ALPHANUMERIC`(0xF0) DOWN+UP、`wScan=0`。** scan=0 を選ぶのは BUG-15 追補7 /
   BUG-25 追補1 が記録した CapsLock scan(0x3A) 衝突を避けるため。
2. mozc（`win32/base/keyevent_handler.cc`）は `VK_DBE_ALPHANUMERIC` を **VK 値のみ**で
   `KeyEvent::EISU` に変換し、キーマップコマンド `ToggleAlphanumericMode`
   （`session/session.cc::Session::ToggleAlphanumericMode` → `composer->ToggleInputMode()`）に
   落とす。これは**無条件トグルであり冪等ではない**。
3. 真に冪等な `Session::CompositionModeHalfASCII`
   （`SessionCommand::SWITCH_COMPOSITION_MODE` 経由）は存在するが、外部から到達する
   試みは `ITfLangBarItemButton::OnMenuSelect`（候補 GUID 2種）で3回とも `S_OK` を返しながら
   実機で一切実効しなかった。**原因不明のまま行き止まりとして扱う**（追補4で closed）。
4. `composer/composer.cc::Composer::SetInputMode` は `composition_.SetInputMode(...)` と
   `is_new_input_ = true` を設定するだけで**既存の未確定文字列（preedit）を書き換えない**。
   Composition 中に送っても非破壊であることをソースと実機（3回）の両方で確認済み。
5. ただし **Composition 中に送って「以後に打つ文字」のモードが実際に変わるか**は、
   6経路のいずれでも肯定的に確認できていない（Precomposition 状態でのみ end-to-end に成功）。
6. **経路5と経路6の差は「awase が起動しているか」だけ**であり、A/B で再現を確定した。
   経路5の失敗は `transport::PhysicalKeyDisposition::plan` の
   `is_dbe_mode_key_down` 条件（`dbe_mode_key_policy=Suppress` 既定、BUG-52 対策）で
   完全に説明できる——spike ツールの `SendInput` は `dwExtraInfo=0` であり、awase から見れば
   **外部プロセスの生の DBE キー**そのものだからである。
7. MS-IME 側の entry（IMC write）は今日も動いており、本ADRの対象外。

### 原因A: 外部プロセス由来の DBE モードキーは awase 自身が握り潰す（確定・維持する）

`runtime/transport.rs::PhysicalKeyDisposition::plan` は、GJI が active な場合
（`key_sequence_policy::gji_direct_applicable`）に `VK_DBE_ALPHANUMERIC` /
`VK_DBE_KATAKANA` / `VK_DBE_SBCSCHAR` / `VK_DBE_DBCSCHAR` の KeyDown を
`shadow_toggled` に関わらず常に `Suppress` する（`is_dbe_mode_key_down`）。
KeyUp も `ime_actuation_owned` 下では常に `Suppress`。

これは BUG-52（2026-08-05 実機、NICOLA の物理「IME ON」キーが 0xF2 ではなく 0xF0/0xF1 を
生成し、素通しすると実 IME が能動的に英数/カタカナへ切り替わる）への対策であり、
BUG-90（PowerToys Mouse Without Borders 経由の「英数」キーが効かない）で journal からも
確認された、**現に効いている安全機構**である。**本ADRはこの保護を外部・物理由来の
DBE キーに対しては一切弱めない**（決定3の設計制約）。

### 原因B: awase 自身の注入は `transport::plan` に到達し得ない（追補4の真因記述の訂正）

`crates/awase-windows/src/hook.rs::hook_callback` のソースを照合した結果、以下が確定した。

1. `is_self_injected(extra_info)` は `INJECTED_MARKER`(`0x4B45_594D`) /
   `TSF_MARKER`(`0x4B45_5946`) / `IME_KANJI_MARKER`(`0x4B45_594A`) の3値のいずれかに
   一致するかを見る（`tsf/output.rs` で定義）。
2. `[hook] IME-mode vk=...` の診断ログは `ImeKeyKind::from_vk(vk).is_some()` で発火し、
   `vk.rs::ImeKeyKind::from_vk` は `0xF0 => Some(Self::Alphanumeric)` を**含む**。
   このログは `self_injected` の早期 return **より前**にあり、追補1が「フックが 0xF0
   イベントを一切受け取っていない」と読んだ根拠自体は正しい。
3. 直後の

   ```rust
   // 自己注入キー（SendInput with INJECTED_MARKER 等）は OS にそのまま通す
   if self_injected {
       return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
   }
   ```

   により、**マーカ付きの自己注入イベントは `build_raw_key_event` にすら到達せず、
   `HOOK_KEYS` にも積まれず、当然 `key_pipeline::kp_run_inner` の
   `PhysicalKeyDisposition::plan`（本番の唯一の呼び出し点）にも到達しない。**

したがって:

- **追補1・追補3 は `TSF_MARKER` 付きで注入していた**（追補2 の記述:
  `make_key_input_ex(VK_DBE_ALPHANUMERIC, is_keyup, TSF_MARKER)`）。これらは
  awase 自身のフックによって `CallNextHookEx` で OS へ素通しされる側であり、
  **`plan` の Suppress ポリシーに握り潰されることは構造的に起こり得ない。**
- 傍証: `tsf/send.rs::send_eager_warmup_vk_pair`（`TSF_MARKER` + scan 付き `VK_IME_ON`）は
  本番で機能している。その doc は自らを「`PhysicalKeyDisposition::plan` が物理キーを
  Suppress した埋め合わせ」と説明しており、**awase 自身のマーカ付き注入が実 IME に
  届くことが前提**になっている。もし自己注入が `plan` の対象なら、この warmup 自体が
  成立しない。
- ゆえに **追補4の「真因の特定」は経路5に対しては正しいが、追補1・追補3の失敗の説明
  としては誤りである。** 追補1・追補3 の 0xF0 は（フックログに現れなかった理由が
  未解明のまま）少なくとも awase の transport 層に握り潰されてはいない。

**では追補1・追補3 は何が違ったのか。** 確認済みで動く形（経路6の spike、および本番で
動いている `ime.rs::send_ime_mode_key`）と、追補2/3 が書いた形の差分は次の3点である。

| 差分 | 追補2/3 の entry | 本番 `ime::send_ime_mode_key` / 経路6 spike |
|---|---|---|
| マーカ | `TSF_MARKER`（TSF Sequential モード識別用） | `IME_KANJI_MARKER`（IME モード/KANJI キー専用。doc に「shadow toggle を一切行わない純パススルー」と明記）／ spike は `0`（マーカなし） |
| 修飾キーの解放 | **無し**（0xF0 の DOWN+UP だけを送る） | `HeldModifiers::read()` → `push_release`/`push_restore` で Ctrl/Shift を解放・復元（Alt/Win は解放しない） |
| 送信関数 | `make_key_input_ex` を直接叩く | `send_ime_mode_key`（Win 押下中スキップ・送信数検証・診断ログ込み） |

このうち**修飾キー文脈が決定的に怪しい**。entry が発火するのは
`key_pipeline.rs::kp_shift_conv_guard_key_up` の「左Shift単独タップ確定」の瞬間であり、
この時点で:

- `HeldModifiers::read()` は `PHYSICAL_KEY_STATE`（ハードウェア由来イベントのみで更新、
  `hook.rs` の `if !is_injected { ... slot.store(is_keydown, ..) }`）を読むため、
  今まさに処理中の**物理 LShift KeyUp は既に反映済み**で `shift=false` を返す。
  → `push_release` は Shift↑ を**一切出さない**。
- しかし OS / 実 IME から見た Shift はまだ**押下中**である。awase はこの物理 Shift KeyUp を
  まだ reinject していない（だからこそ `kp_restore_kana_from_half_width` には
  `prepend_synthetic_shift_up: bool` という引数が存在し、その doc は
  「呼び出し元がまだ物理 Shift up の reinject を行っていない（＝ OS 視点でまだ Shift 押下中）
  場合は true にする」「Shift+ひらがなキー = カタカナ切替に化けないよう、synthetic Shift up を
  同一バッチの先頭に入れる」と明記している）。

つまり **追補2/3 の entry 注入は、OS/IME から見れば `Shift + VK_DBE_ALPHANUMERIC` として
届いていた可能性が高い。** mozc の `keyevent_handler.cc` は修飾キー状態込みで
`KeyEvent` を構築するため、`Shift+Eisu` は `Eisu` とは別のキーであり、出荷キーマップに
束縛が無ければ**無反応（no-op）**になる。復元側（`kp_restore_kana_from_half_width`）は
この罠を既に知っていて synthetic Shift↑ を前置しているのに、**entry 側だけがその対策を
持っていなかった**。

`[hook] IME-mode vk=0xF0` ログが出なかった件はこの仮説では説明できず、**未解明のまま残る**
（「未解決の疑問」1）。ただしログの不出現は「GJI に届いたか」とは独立の問題である
——自己注入は `CallNextHookEx` で先へ進むため、フックで観測されようがされまいが OS には
渡る。設計判断の根拠としてはログ不出現より「実 IME が反応しなかった」という一次事実の方を
採る。

### 制約

- [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の
  Observe → 純粋 `classify_*` → `reduce()` を破らない。entry/exit は観測ではなく
  **awase 自身の能動的訂正**であり、既存どおり
  `ImeEvent::InputModeApplied { strategy: InputModeApplyStrategy::UserHalfWidthAlnumToggle }`
  で表現する（`InputModeObserved` の偽装は禁止）。
- [layer-boundaries](../layer-boundaries.md) A-1: コア `awase` クレートは OS 非依存を保つ。
  注入は全面的に `crates/awase-windows` の問題。B-2: `output/` は named API のみで呼ぶ。
  D-1: magic hex を `vk.rs` 外に書かない。
- 原因A の保護（外部・物理由来 DBE キーの Suppress）を弱めない。
- BUG-15 追補7: 実 IME が確実に ON でない限り IME モードキーを注入しない。
- GJI に対して **IMC read/write を成否判定に使わない**（追補2の教訓）。実効性の検証は
  **実際に打鍵した文字**でのみ行う。

## 不変条件

- **INV-A（entry は一度きり）**: 非冪等な `ToggleAlphanumericMode` は、awase 側の
  `half_width_alnum_toggle_active` の **false→true の遷移**に対してのみ、ちょうど1回
  送る。`true` のまま再送する経路を作らない。
- **INV-B（exit も一度きり）**: 同様に **true→false の遷移**に対してのみ1回。
  「復元は何度呼んでも安全」という MS-IME 時代の前提（IMC write は冪等な SET）は
  GJI では成立しない——トグルの二重送信は「かなへ戻す」ではなく「英数へ戻す」になる。
- **INV-C（自己注入の識別は二要素）**: transport 層で DBE Suppress を迂回してよいのは、
  「awase 自身がこの機能のために発行した」ことが **識別子（マーカ）と意図（ワンショット
  通行証）の両方**で裏付けられる場合に限る。片方だけでは足りない（決定3）。
- **INV-D（実証されるまで機能を無効化する）**: entry の実効性が実機で確認できていない
  IME 種別・状態では、`half_width_alnum_toggle_active` を**立てない**。belief だけ
  進めて engine を pass-through にすることは、何も起きないより悪い（追補3）。

---

## 決定0: 実装着手前に、残置した spike で 2×2 の切り分け計測を行う

**根拠**: 原因B により、追補4 の真因記述は追補1/3 の失敗を説明していない。何が犯人か
分からないまま実装案を選ぶと、4回目の撤回になる。幸い判別は安く、道具は既にある
（`crates/awase-windows/examples/spike_langbar_input_mode.rs`、追補4 で再利用可能な形で残置）。

spike に `--sendinput-marker=<none|tsf|ime_kanji>` と `--sendinput-shift-held`
（注入前に Shift↓ を送り、注入後に Shift↑ を送る）の2フラグを足し、次を実機で埋める。

| # | マーカ | awase | Shift | 期待/仮説 | 実機結果（2026-08-27） |
|---|---|---|---|---|---|
| M1 | none | 停止 | 解放 | 成功（＝追補4 経路6 の再確認、基準線） | **成功** |
| M2 | none | 起動 | 解放 | 失敗（＝追補4 経路5 の再確認、原因A の再現） | **失敗**（想定通り） |
| M3 | `IME_KANJI_MARKER` | 起動 | 解放 | **成功なら決定2で足りる**（原因A を構造的に迂回できている） | **成功**（Composition 中の再検証含め4回とも成功） |
| M4 | `IME_KANJI_MARKER` | 起動 | **押下中** | **失敗なら「原因B＝修飾キー文脈」が確定**（追補1/3 の説明が付く） | **失敗、原因B 確定**（KeyDown 自体がフックに届かず） |

**結論（決定0 完了）**: 全セルが仮説どおりの結果となり、**分岐は「M3 成功
→ 決定2 のみ実装、決定3 は実装しない」に確定した。** ただし M3/M4 の過程で
決定0 策定時には想定していなかった `SendInput` の KeyUp 欠落という新しい
構造的制約が見つかったため、決定2 に settle 待ち・連続発火の最小間隔という
追加のセーフガードが必要になった（詳細は決定2 の追記、BUG-25 追補5 を参照）。

同時に、各セルで `[hook] IME-mode vk=0xF0 ... self_injected=` 行が出るか（未解決の疑問1）と
CapsLock 状態を記録する。判定は必ず**実際に打鍵した文字**で行う（追補2の教訓）。

**分岐**:

- M3 成功 → **決定2 のみ実装。決定3（transport バイパス）は実装しない。**
- M3 失敗 かつ M2 失敗・M1 成功 → 決定2 に加えて **決定3 を実装**（unmarked 注入 + 通行証）。
- M1 まで失敗 → 前提が崩れている。実装せず追補5 に記録して中断する。

**証拠義務**: 結果は `docs/known-bugs.md` BUG-25 追補5 に上表の形で記録する。spike の
2フラグ追加は診断ツールの拡張であり本体の挙動を変えないため、テストは不要。

---

## 決定1: entry は `VK_DBE_ALPHANUMERIC`(0xF0) の scan=0 DOWN+UP、`SendInput` に固定する

追補4 の示唆1 をそのまま採用する。`PostMessageW`（経路3/4）は Precomposition では
成功するが Composition 中に実効しないことが確認済みで、かつ OS のキーボード状態を
経由しないため mozc が修飾キー込みで `KeyEvent` を組む前提と噛み合わない。
`OnMenuSelect`（経路1/2）は closed。

scan は **0**（`make_key_input_ex`）。`MapVirtualKeyW(0xF0, MAPVK_VK_TO_VSC)` が返す
0x3A は物理 CapsLock 位置であり、BUG-15 追補7 / BUG-25 追補1 が記録した汚染の入口。
**scan 付きヘルパー（`make_tsf_key_input`）をこの経路に使ってはならない。**

追補1 の教訓「warmup 用ヘルパーを standalone トグルへ転用しない」を守り、
`tsf/send.rs::send_eager_warmup_vk_pair`（warmup 専用、`TSF_MARKER`+scan）は流用しない。

---

## 決定2: 注入は `IME_KANJI_MARKER` を使い、**synthetic Shift↑ を同一バッチに前置する**

**これが本ADRの最優先の実装案であり、原因B に対する直接の対策である。**

送るバッチ（1回の `SendInput`）:

```
[ synthetic VK_SHIFT ↑ (IME_KANJI_MARKER) ]   ← prepend_synthetic_shift_up が真のときのみ
  VK_DBE_ALPHANUMERIC ↓ (IME_KANJI_MARKER, wScan=0)
  VK_DBE_ALPHANUMERIC ↑ (IME_KANJI_MARKER, wScan=0)
```

**マーカに `IME_KANJI_MARKER` を選ぶ理由**: `tsf/output.rs` の doc が
「IME モード/KANJI トグル注入（`VK_KANJI`, `VK_IME_ON/OFF`, `VK_DBE_*`）用。フックは
再処理も **shadow toggle も一切行わない**純パススルー」と定義しており、本用途の意味論と
完全に一致する。`TSF_MARKER`（追補2/3 が使った値）は TSF Sequential モードの識別子で
意味論がずれている。さらに重要なのは、`VK_DBE_ALPHANUMERIC` の
`ImeKeyKind::shadow_effect()` は `ShadowImeEffect::TurnOff` であり、**もし自分の注入が
shadow toggle 経路に乗ると「IME が OFF になった」と belief を汚染する**。
`IME_KANJI_MARKER` はこれを構造的に防ぐ。

**synthetic Shift↑ を前置する理由**（原因B 参照）: `kp_shift_conv_guard_key_up` の時点で
`HeldModifiers::read()`（`PHYSICAL_KEY_STATE` 由来）は既に `shift=false` を返すため、
`ime::send_ime_mode_key` の `push_release` は Shift↑ を出さない。一方 OS/実 IME から
見た Shift はまだ押下中である。したがって **`send_ime_mode_key` をそのまま呼ぶのでは
不十分**であり、`kp_restore_kana_from_half_width` が既に採っているのと同じ
`prepend_synthetic_shift_up` を entry 側にも入れる必要がある。物理 Shift は既に解放済み
なので restore は不要（後続の本物の Shift↑ reinject と重複するが KeyUp の重複は無害、
という復元側と同じ根拠）。

**Win/Alt 押下中はスキップする**（`hook::win_key_held() || hook::alt_key_held()`）。
根拠は `kp_restore_kana_from_half_width` の既存ガードと同一（Win+F2 でスタートメニュー、
Alt+かな で JIS かな直接入力へ切替、2026-08-17 実機診断）。スキップした場合は
**INV-D により `half_width_alnum_toggle_active` を立てない**（belief だけ進めない）。

**`effective_open()` ガード**を entry にも掛ける。BUG-15 追補7 の「実 IME が確実に ON で
ない限り IME モードキー注入禁止」は方向（英数化／かな化）を問わない。

**追記（2026-08-27、決定0 の実機結果を受けた追加要件）: `SendInput` の KeyUp が
awase 自身のフックに構造的に届かないことが判明した**（`docs/known-bugs.md`
BUG-25 追補5）。DOWN/UP を同一バッチで送っても、別々の `SendInput` 呼び出しに
分割し実時間を空けても再現し、タイミングは原因ではない。モード切替自体は
毎回成功しているため決定2 の実装方針は変えないが、**KeyUp 欠落による GJI
側の潜在的なキー状態不整合を前提に、以下2点を追加する**:

1. **settle 待ち**: entry の `SendInput` 発行後、`half_width_alnum_toggle_active`
   を立てて engine を pass-through 状態にする前に、短い猶予を挟む
   （既存の warmup/cold-start 系パターンと同種）。追補5 で観測した「1文字目
   だけ全角英数になるレース」は `--sendinput-up-delay-ms=50` を挟んだ再試行
   では再現しなかった（サンプル数は少なく確定的ではないが、方向性としては
   一致する）。具体的な待機時間は
   [tuning-constants](../../.claude/rules/tuning-constants.md) の実測義務に
   従い、決定0 と同じ spike（`--sendinput-up-delay-ms`）で複数回の再現実験を
   行ってから決めること——現時点で単一サンプルの 50ms を定数化しない。
2. **連続発火の最小間隔**: 追補5 で M3 を3回連続実行した際、3回目で
   「一瞬全角英数→IME オフ・直接入力」という予測不能な状態遷移を観測した
   （awase 自身の drift correction / idle-conv-check が短時間の連続トグルに
   反応して介入したと推定、ログの完全な相関分析は未実施）。INV-A（entry は
   一度きり）が単一の左Shiftタップに対しては既に二重発火を防いでいるが、
   **ユーザーが短時間に複数回タップした場合**（連打）に同じ不安定化が
   起きないよう、entry 呼び出し自体に最小間隔のクールダウンを設けることを
   検討する。

---

## 決定3: 決定2 が実機で不発の場合に限り、unmarked 注入 + `DbeSelfInjectionPass` 通行証

**条件付き決定。決定0 の M3 が失敗した場合にのみ実装する。** 実装しない場合、本節は
「なぜその設計を選ばなかったか」の記録として残す。

M3 が失敗し M1 が成功するなら、「実際に GJI へ届く唯一の形」は経路6 と同じ
**マーカなし**の `SendInput` だということになる。その形で送ると awase 自身のフックは
これを外部の生 DBE キーとして扱い、`transport::plan` が原因A で握り潰す。よって
**その1回の注入だけを通す**仕組みが要る。

### 3-a: 二要素の通行証（INV-C）

`output/` に、この機能専用のワンショット通行証を持たせる。

```rust
// state/dbe_self_injection.rs（ungated、Win32 非依存 ＝ Linux で単体テスト可能）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbeSelfInjectionPass {
    vk: VkCode,          // この機能が送る VK に固定（0xF0）
    remaining: u8,       // DOWN+UP の 2。0 になったら失効
    expires_at_ms: u64,  // 期限。過ぎたら失効
}

impl DbeSelfInjectionPass {
    /// 通行証が当該イベントを許可するか判定し、許可なら1枚消費する純粋関数。
    pub fn admit(slot: &mut Option<Self>, vk: VkCode, marker: usize, now_ms: u64) -> bool;
}
```

`plan` のシグネチャに `self_injection_admitted: bool` を1つ足す（`plan` 自身は純粋関数の
まま保つ。通行証の消費は呼び出し元 `kp_run_inner` で行い、`plan` には結果だけ渡す）。
`plan` 側の変更は `is_dbe_mode_key_down` の条件に `&& !self_injection_admitted` を足す
1行に留める。

**二要素であることが本質**（INV-C）:

1. **識別子**: `event.extra_info` が本機能専用の新マーカ
   `GJI_ALNUM_ENTRY_MARKER`（`tsf/output.rs` に4つ目の値として追加、例 `0x4B45_5941`）に
   一致すること。**この値を `hook.rs::is_self_injected()` に足してはならない**——足すと
   フックが早期 return して `plan` に到達せず、それは決定2（マーカ付き）と同じ形に
   戻ってしまい、M3 が失敗した前提と矛盾する。この「足さない」という反直感的な契約は
   guard test で固定する（決定8）。
2. **意図**: 通行証が armed で、VK が一致し、残数があり、期限内であること。

片方だけでは不足する理由:

- マーカだけ → `dwExtraInfo` は誰でも任意値を書ける。他ツールが偶然/意図的に同値を
  書いた DBE キーが常時素通しになり、BUG-52 の保護が恒久的に穴になる。
- 通行証だけ → armed の窓の間に**物理**の DBE キー（BUG-52 が想定する、NICOLA の
  物理 IME ON キーが生成する 0xF0/0xF1）が到着すると、それが通行証を食って素通しする。
  窓は短いがゼロではない。

### 3-b: 期限定数の扱い

`expires_at_ms` の猶予は `tuning.rs` に置く（例 `GJI_ALNUM_ENTRY_PASS_TTL_MS`）。
[tuning-constants](../../.claude/rules/tuning-constants.md) の実測義務が掛かる。
**測るもの**: 「`SendInput` 発行から、対応する LL フックコールバックが awase に到達する
までの遅延」。`spike` に計測モードを足すか、実装時に一時的な `log::info!` で往復を測る。
**導出**: 実測最大 + マージン。これは「機能が動くまで待つ」種類の定数ではなく
**失効境界**であり、大きすぎると通行証が物理キーに食われる窓が伸びる——
tuning-constants.md が警告する「効かないから増やす」エスカレーションとは逆に、
**小さい方が安全側**である点を実装コミット本文に明記すること。

### 3-c: arm と inject を分離不能にする

通行証を arm する関数と注入する関数を分けない。`Output` の**単一のメソッドの中で**
「arm → `SendInput`」を行い、`Output` の外から通行証を arm する API を公開しない
（ADR-106 決定1 が `GenerationAllocator::allocate(&mut self)` で「読むだけで進まない」を
型で潰したのと同じ考え方）。arm したのに送らない／送ったのに arm し忘れる、という
型で守られない契約を作らない。

---

## 決定4: 冪等性は awase 側の遷移ゲートで担保する（mozc 側の冪等コマンドは使わない）

追補4 の示唆3 を採用する。到達可能な mozc 側コマンドは非冪等なトグルだけなので、
**「ちょうど1回」は awase の状態機械が保証する**。

### 4-a: entry の発火点（INV-A）

`runtime/key_pipeline.rs::kp_shift_conv_guard_key_up` の

```
is_left_shift_tap && toggle_entry_supported && !half_width_alnum_toggle_active
```

の分岐そのもの。ここは `half_width_alnum_toggle_active` を `false` から `true` に
書き換える**唯一の場所**であり（`GateStore` の他フィールドと違い書き込み点が1つしかない）、
`shift_conv_guard_pending` と `left_shift_tap_candidate` はいずれも
`std::mem::take` で消費されるため、同一の物理タップで2回入ることはない。
`toggle_entry_supported` の定義を
`active_ime_kind == MicrosoftIme` から **設定 + IME 種別 + 実効性ガードの合成**へ広げる
（決定6・決定7）。

**残余リスク**: entry の `SendInput` が OS に受理されたが GJI に無視された場合、belief は
`ObservedEisu` に進み実モードはひらがなのまま——追補3 の実害そのものになる。
これは `SendInput` に完了通知が無い以上、構造的に消せない。緩和は
(i) 決定7 の実効性ゲート（実証済みの条件でしか `toggle_entry_supported` を真にしない）、
(ii) 決定6 のキルスイッチ、(iii) 右Shift単独タップによる緊急解除（既存）の3つ。
**残余リスクの大きさ**: 発火は「左Shift単独タップ」という明示的なユーザー操作に限られ、
1タップにつき高々1回。暴走的に繰り返す経路は無い。

### 4-b: exit の発火点と、既存コードに潜む二重送信リスク（INV-B）

`kp_restore_kana_from_half_width` は現在**無条件に** `half_width_alnum_toggle_active = false`
を代入し、その後の OS 書き込みへ進む。呼び出し元は4系統ある
（`kp_shift_conv_guard_key_up` の2回目タップ／右Shift緊急解除、
`ir_notify_focus_changed` のフォーカス変更、`kp_stage_shadow_ime_toggle` の
`UserImeOnEisuReset`/`UserTurnOnEisuReset`、`kp_stage_post_decision` の
`PostSetOpenEisuReset`）。

MS-IME ではこれで安全だった——復元は IMC write（冪等な SET）だからである。
**GJI で復元にトグルを使うと、同一 tick 内で2系統から呼ばれた瞬間に「かなへ戻す」が
「英数へ戻す」に反転する。** よって:

- `half_width_alnum_toggle_active` の読み書きを
  `std::mem::replace(&mut ..., false)` に変え、**旧値が `true` だったときにだけ**
  OS 書き込み（GJI 向け注入）を行う。belief 更新
  （`apply_input_mode_correction`）は現状どおり分岐の外で無条件に行ってよい
  （冪等な代入であり、二重呼び出しで壊れない）。
- ADR-084 INV-7 が記録している「entry が MS-IME 限定なのだから復元も MS-IME 限定」
  という**対称性の根拠が、本ADRで変わる**。`kp_restore_kana_from_half_width` の
  `active_ime_kind == MicrosoftIme` 分岐を、GJI にも entry がある前提へ書き換えること
  （同関数の doc コメントも合わせて更新する。前提が変わったのにコメントだけ残ると、
  次の担当者が「GJI には entry が無い」と誤読する）。

### 4-c: exit のキーは非対称に選ぶ（トグルではなく SET を狙う）

exit では `VK_DBE_ALPHANUMERIC` を再送するのではなく、**`VK_DBE_HIRAGANA`(0xF2) を送る**
案を推す。理由:

- 0xF2 は awase が本番で GJI へ送り続けている実績のある VK であり（warmup・復元経路）、
  TSF ルーティングで到達することが分かっている。
- mozc 側で `KeyEvent::HIRAGANA` が**冪等な SET**（`InputModeHiragana`）に落ちるなら、
  entry のトグルが何らかの理由で二重発火していても exit で状態が収束する
  ——非対称にすることで「入口は一度きり、出口は必ず正しい状態へ倒す」という
  自己修復性が得られる。
- **要確認**: `KeyEvent::HIRAGANA` → `Session::InputModeHiragana` が冪等な SET である
  ことは、追補4 の mozc 調査範囲に**含まれていない**。決定0 のついでに
  `session/session.cc` / `session/keymap.h` で確認すること。冪等でなければ
  exit も `VK_DBE_ALPHANUMERIC` の再送（対称なトグル）に落とし、INV-B の遵守が
  唯一の安全弁になる。

既存の `effective_open()` ガード・Win/Alt スキップ・`prepend_synthetic_shift_up` は
exit 側にも同じく適用する（復元経路には既に全部ある）。

---

## 決定5: Composition / 候補ウィンドウ表示中は、発火せず**ラッチもしない**

追補4 の示唆4 は「ガードを設ける方が無難」と書いているが、本ADRはより強く
**「発火しない、かつ `half_width_alnum_toggle_active` を立てない」**を決定する。
「後で発火させる（defer）」も採らない。

**推奨と理由**:

1. **defer は採らない。** ユーザーの1タップという明示的な操作の効果が、composition が
   閉じた不定の時点で遅れて現れるのは、何も起きないより驚きが大きい。さらに
   「composition が閉じた」ことを知る信頼できる単一のイベントは無く
   （`take_pending_end_composition` は drain-once のブリッジフラグで、他の消費者と
   競合する）、defer は新しい状態と新しい競合を1つずつ増やす。
2. **「発火はしないがラッチはする」も採らない**（これが本節の核心）。追補4 で
   Composition 中の実効性は**6経路すべてで肯定的に確認できていない**。実効しない
   可能性のある状態で belief だけ `ObservedEisu` に進めると、engine が pass-through に
   なり生ローマ字キーが GJI の未切替のかな変換エンジンへ流れ込む——**追補3 が実際に
   起こした「かな入力が壊れる」実害そのもの**である。INV-D をここに適用する。
3. **no-op は観測可能で回復可能。** ユーザーは確定（Enter）か Esc の後にもう一度
   タップすればよい。ログ（`log::info!`）で「composition 中のため半角英数トグルを
   見送った」と残せば、実機ログからも判別できる。

**判定に使う観測**: `tsf/observer.rs::ime_composition_active_now()`（IME 非依存、
`EVENT_OBJECT_IME_SHOW`/`HIDE` 由来）**または** `gji_candidate_visible_now()`
（GJI 固有の候補ウィンドウ）。前者だけでは候補なしの preedit を、後者だけでは
候補が出ていない composition を取りこぼすため OR で取る。どちらも
`tsf_obs()` 経由の named アクセサであり、layer rule B-3 に適合する。

**例外**: **exit（右Shift緊急解除・2回目タップ・フォーカス変更由来の復元）は
composition 状態に関わらず実行する。** exit は「安全な既定状態へ戻す」操作であり、
見送ると半角英数状態が他アプリへ持ち越される方が実害が大きい（BUG-25 本体の
フォーカス変更安全策と同じ理屈）。ガードは entry 側だけの非対称なものにする。

**追記（2026-08-27、決定0 の実機結果で本決定の前提が古くなった。
[`docs/known-bugs.md`](../known-bugs.md) BUG-25 追補5）:** 上記「2.」の根拠
（「Composition 中の実効性は6経路すべてで肯定的に確認できていない」）は
**追補4 時点の記述であり、その6経路はいずれも後に効かないと判明した経路
（`PostMessageW`、マーカなし `SendInput`）だった。** 決定2 が定めた「正しい
注入方式」（`IME_KANJI_MARKER` 付き `SendInput`）で Composition 中に発火
させたのは追補5 が初めてで、2回とも**非破壊・成功**（プリエディット無傷、
続けて打った文字が正しく半角英数）した。

ただしサンプル数が2回のみであり、本決定を撤回するには不十分と判断する。
**本ADRでは決定5（発火せずラッチもしない）を実装方針として維持するが**、
理由を「実効性が全く確認できていないため」から「サンプル数が少なく安定性
（特に KeyUp 欠落との相互作用、`docs/known-bugs.md` 追補5 参照）を確認しき
れていないため」に差し替える。決定0 と同じ spike
（`--sendinput-marker=ime_kanji`）を使い、Composition 中の複数回再試行
（10回程度、preedit 内容・カーソル位置を変えたパターンを含む）で安定性が
確認できれば、決定5 を緩和し composition 中も発火させる設計へ次回セッション
で見直す価値がある。

## 決定6: 注入の置き場所 — `Output` の gated メソッド1つに閉じ、arm と分離できなくする

**低レベル送信**は既存の `crates/awase-windows/src/ime.rs::send_ime_mode_key` と同じ
形（`make_key_input_ex` + `IME_KANJI_MARKER` + scan=0 + Win 押下スキップ + 送信数検証）
を採るが、決定2 の synthetic Shift↑ 前置が必要なため**そのままは呼べない**。
`send_ime_mode_key` に `prepend_synthetic_shift_up: bool` を足すか、隣に
`send_ime_mode_key_with_shift_release_prefix` を置く（既存 6 呼び出し元の挙動を
変えないよう、既定 `false` の別関数にする方を推す）。

**gated な入口**は `crates/awase-windows/src/output/mod.rs` に
`Output::send_gji_half_width_alnum_toggle(&self, prepend_synthetic_shift_up: bool) -> bool`
として置く。`send_eager_tsf_warmup` / `send_f22_f21_reinit` / `send_unicode_cold_warmup_keys`
と同じ「ゲート判定 + 委譲」の並びに入る。

**なぜ `output/mod.rs` か**（他候補を実コードの責務と突き合わせて棄却した結果）:

- `output/vk_send.rs`: 中身は `TsfSendPipeline` と `send_romaji_*`/`send_char_*` で、
  責務は**テキスト（ローマ字/かな）の送信**。IME モードキーは1つも無い。
  [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の再発ファミリー表が
  「キー選択（IME ON/OFF に送る VK）」の主なファイルとして `vk_send.rs` を挙げているのは
  事実だが、**現在の実装の責務とは合っていない**（表の方が実態に遅れている）。
  ファミリーの証拠義務は決定8 で満たすので、置き場所を実態に合わせない理由は無い。
- `output/conv_actuation.rs`: 「conv-mode を変更する唯一の窓口」(ADR-084 P1/INV-1)。
  ただしその実体は **IMM32 の conversion-status write** であり、ADR-084 は
  「DBE VK の `SendInput` は ROMAN ビットに効かないと実証済みで撤去した」(BUG-61) と
  記録している。**GJI 向けトグルは conv write ではない**ので、ここに混ぜると
  INV-1 の「唯一の窓口」の意味が濁る。
- `runtime/conv_actuation.rs`: `Output::actuate_conv_mode` への1行の委譲だけを持つ層。
  新しいロジックの置き場所ではない（同ファイルの doc が明記）。
- `tsf/send.rs`: doc が「TSF warmup VK 送信ヘルパー」と自称し、関数も1つ
  （`send_eager_warmup_vk_pair`）だけ。**追補1 の教訓（warmup ヘルパーを standalone
  トグルへ転用しない）に真正面から反する。**

`key_pipeline.rs` からは `self.platform.output.send_gji_half_width_alnum_toggle(..)`
という named API 越しにのみ呼ぶ（layer rule B-2）。決定3 を実装する場合、通行証の arm は
このメソッドの内部で行い、外部に arm API を出さない（決定3-c）。

---

## 決定7: `toggle_entry_supported` を「IME 種別」から「実証済みの発火条件」へ広げる

現在の

```rust
let toggle_entry_supported = tsf_obs().active_ime_kind() == ActiveImeKind::MicrosoftIme;
```

を、次の合成に置き換える。**この式が INV-D の唯一の実装点**になる。

1. 設定（決定8 のキルスイッチ）が当該 IME 種別を許可している。
2. `active_ime_kind` が `MicrosoftIme`（従来どおり IMC write）または
   `GoogleJapaneseInput`（本ADRの新経路）である。
3. `effective_open() && belief.is_japanese_ime() && engine.is_user_enabled()`
   （`kp_shift_conv_guard_key_down` が既に `shift_conv_guard_pending` を折る条件と同じ）。
4. GJI の場合のみ: composition/候補が出ていない（決定5）、かつ Win/Alt が押されていない
   （決定2）。

**この判定を純粋関数へ切り出す**ことを強く推奨する。`state/` に

```rust
pub enum HalfWidthAlnumAction { None, Enter, Exit }

pub fn plan_half_width_alnum_action(
    is_left_shift_tap: bool,
    is_right_shift: bool,
    toggle_active: bool,
    entry_supported: bool,
    composing: bool,
) -> HalfWidthAlnumAction;
```

を置けば、決定4（一度きり）・決定5（composition ガード）・INV-D が **Linux 上の
単体テストで機械的に固定できる**。現状これらは `kp_shift_conv_guard_key_up` の
中に散らばった `if` であり、Windows 実機フック依存を理由に一切テストされていない
（BUG-25 本体の「テスト」節が「自動テスト不可」と認めている）。この切り出しは
本ADRで**テスト可能になる範囲を最大化するための中心的な工事**である。

---

## 決定8: キルスイッチは `half_width_alnum_toggle`（`off` / `ms_ime_only`（既定）/ `all`）

同じ機能で3回撤回している以上、4回目は**コード revert なしで止められる**必要がある。

```rust
// src/config.rs（コア awase クレート、GeneralConfig）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HalfWidthAlnumTogglePolicy {
    Off,
    #[default]
    MsImeOnly,
    All,
}
```

- **既定 `MsImeOnly` は今日の挙動そのもの。** つまり本ADRの実装をマージした直後の
  既定動作は無変更であり、GJI 向け新経路は `all` を明示したユーザーだけが踏む
  （オプトイン）。追補3 の実害が「かな入力が壊れる」という重いものである以上、
  実機ソークが積み上がるまで既定で配るべきではない。
- `off` は MS-IME 側も含めて機能全体を止める。今日この機能には off スイッチが無く、
  StickyKeys ユーザー（BUG-25 未検証項目3、今も未解決）や「左Shift をタップする癖が
  ある」ユーザーが逃げられない。この穴も同時に塞ぐ。
- 置き場所は `GeneralConfig`、`dbe_mode_key_policy` /
  `muhenkan_solo_tap_dedicated_fn_key` と同じ**隠し設定**（config.toml のみ、
  設定 GUI には出さない）。`all` の実機評価が固まった段階で GUI 露出と既定変更を
  別コミットで検討する。
- 運搬は `dbe_mode_key_policy` と同じ経路（`Runtime` のフィールド + `set_*` +
  `apply_config_update`）に倣う。既にリロード機構があるため再起動不要で止められる。

**`dbe_mode_key_policy` の再利用は却下する**（「却下した代替案」参照）。

---

## 決定9: 証拠義務 — 何をテストし、何を記録で代替するか

[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の再発ファミリーのうち
**「キー選択（IME ON/OFF に送る VK）」** と **「IME belief」** に触れる。加えて決定3 を
実装する場合は `tuning.rs` に定数が増えるため
[tuning-constants](../../.claude/rules/tuning-constants.md) の実測義務が、
将来この機能を再び撤回する場合は
[experiment-logging](../../.claude/rules/experiment-logging.md) の
「アプリ × IME × 再現手順」記載義務が掛かる。

### (a) Linux で回せる自動テスト

| 対象 | 置き場所 | 固定する内容 |
|---|---|---|
| `plan_half_width_alnum_action`（決定7） | `state/` の当該モジュール `#[cfg(test)]` | entry は false→true の1回だけ／composing 中は `None`／右Shift は toggle_active のときだけ `Exit`／`entry_supported=false` なら `None`（INV-A/D） |
| `DbeSelfInjectionPass::admit`（決定3、実装する場合のみ） | `state/dbe_self_injection.rs` `#[cfg(test)]` | 残数2で DOWN+UP が通る／3枚目は通らない／VK 不一致は通らない／期限切れは通らない／マーカ不一致は通らない |
| `PhysicalKeyDisposition::plan`（決定3、同上） | `runtime/transport.rs::plan_tests`（既存） | `self_injection_admitted=true` で `Allow`／`false` なら従来どおり `Suppress`／`dbe_mode_key_policy=Passthrough` 側の既存4テストが不変であること（BUG-52 の保護が無傷） |
| belief 遷移 | `tests/golden_scenarios.rs` | 既存 `scenario_15_half_width_alnum_toggle_keeps_ime_open_while_engine_goes_inactive` に GJI 版を追加。**さらに「復元イベントを2回流しても belief が英数へ戻らない」を追加**（INV-B の belief 側の対応物） |
| 構築点の件数 | `tests/architecture_guard.rs` | `input_mode_applied_construction_sites_are_accounted_for` の `key_pipeline.rs` 期待値を更新 |
| 反直感的な契約 | `tests/architecture_guard.rs`（新規、決定3 実装時のみ） | **`hook.rs::is_self_injected()` の本体に `GJI_ALNUM_ENTRY_MARKER` が出現しないこと。** 足すと設計が静かに壊れる（フックが早期 return して `plan` に届かなくなる）ため、テキスト走査で固定する |

### (b) 自動テストで代替できないもの（記録で担保する）

実 IME のモードが本当に切り替わったか、CapsLock が汚染されないか、Composition 中の
挙動——いずれも実機依存。`docs/known-bugs.md` **BUG-25 追補5** に次を記録する。

- 決定0 の 2×2 結果表。
- 実機検証チェックリスト（追補2 が定めた3点を踏襲・拡張。実行可能な手順書は
  [bug25-gji-entry-verification-checklist.md](../bug25-gji-entry-verification-checklist.md)）:
  1. `[hook] IME-mode vk=0xF0 ... self_injected=` 行の出現有無。
  2. CapsLock 状態が変化していないこと。
  3. **実際に打鍵した文字**が半角英数になること（IMC の read-back を成功判定に使わない）。
  4. トグルON→フォーカス変更→戻る、を往復しても英数状態が持ち越されないこと。
  5. トグルON→右Shift緊急解除、とトグルON→2回目左Shiftタップ、の両方でかなに戻ること
     （INV-B の実機側の確認。**二重復元の反転が起きないこと**）。
- `docs/experiments.md` に**着手前**に1行の事前登録エントリを立てる（ADR-100 が
  エントリ16 で採った方式）。4回目の挑戦であり、成否どちらでも次の担当者に残る形にする。

---

## 実施順序

| 順 | 内容 | 前提 |
|---|---|---|
| 1 | 決定0（spike に2フラグ追加 → 2×2 実機計測 → 追補5 記録） | 無し。**ここを飛ばして実装に入らない** |
| 2 | 決定7 の純粋関数切り出し（挙動変更ゼロのリファクタ + テスト） | 無し。1と並行可 |
| 3 | 決定8（設定追加、既定 `ms_ime_only` ＝ 挙動変更ゼロ） | 無し。1と並行可 |
| 4 | 決定2 + 決定4 + 決定5 + 決定6（GJI entry/exit 本体） | 決定0 が M3 成功 or M1 成功 |
| 5 | 決定3（unmarked + 通行証） | **決定0 の M3 が失敗した場合のみ** |
| 6 | 実機ソーク（`all` を明示設定して常用）→ 既定変更/GUI 露出の判断 | 4（または5）完了後 |

順序2・3 を先に単独で入れておくと、4 が失敗して撤回する場合でも
「純粋関数化」「off スイッチ」という独立に価値のある成果が残る。

---

## 却下した代替案

- **`hook.rs::is_self_injected()` に本機能用のマーカを追加する（決定3 の変種）**:
  追加すると `CallNextHookEx` の早期 return に乗る＝決定2 とまったく同じ形になる。
  決定3 は「決定2 の形では届かなかった」場合の案なので、自己矛盾する。
- **`event.injected`（`LLKHF_INJECTED`）でバイパスする**: 他プロセスの synthetic DBE キーが
  すべて素通しになる。BUG-90 は文字どおり **PowerToys Mouse Without Borders が VK を
  再構成して送る**ケースであり、BUG-14 は「1打鍵ごとに foreign-injected `VK_KANA` が届く」
  MS-IME 自身の注入を記録している。区別に使える情報ではない。
- **GJI のときだけ `dbe_mode_key_policy` を `Passthrough` 相当に倒す**: BUG-52 の保護
  （NICOLA の物理 IME ON キーが 0xF0/0xF1 を生成して実 IME が能動的にモードを変える）を
  GJI 全体で恒久的に外すことになる。バイパスは「awase 自身が今この瞬間に送った1回」に
  限定されなければならない（INV-C）。
- **キルスイッチとして `dbe_mode_key_policy` を再利用する**: これは「**外部の** DBE キーを
  どう扱うか」の軸であり、「**自分の** entry 機能を使うか」とは別軸。相乗りさせると
  「半角英数トグルを使いたいユーザーが BUG-52 の保護も同時に外す」ことになる。
  軸を混ぜない（ADR-088 以降の「軸」の扱いに倣う）。
- **`config1.db` のカスタムキーマップで `COMPOSITION_MODE_HALF_ALPHANUMERIC` に
  キーを割り当て、真に冪等な SET を使う**: 技術的には最も筋が良いが、
  `config1.db` 書き込み編集は
  [[project_ime_key_danger_classification_and_roadmap_2026_08_11]] で「復活させない」と
  既に決定済み。本ADRのスコープ外。
- **`ITfLangBarItemButton::OnMenuSelect` の再挑戦**: 2つの候補 GUID で計3回、
  `S_OK` を返しながら実効しない。edit session 完了待ちのメッセージポンプを 3 秒回しても
  不変。原因不明のまま closed（追補4）。新しい根拠が出るまで再試行しない。
- **`PostMessageW` を本番経路にする**: Precomposition では成功する（経路3）が
  Composition 中は実効せず（経路3/4）、しかも OS のキーボード状態を経由しないため
  mozc が修飾キー込みで `KeyEvent` を組む前提と噛み合わない。加えて他プロセスの
  入力キューへ直接投げる形は BUG-09 / ADR-105 が繰り返し警告している配送経路の罠に
  近い。診断ツール（spike）に留める。
- **entry の前に conv を読んで「既に英数なら送らない」ことで冪等にする**: 追補2 の
  教訓に真正面から反する。GJI に対する IMC read は「awase 自身が直前に書いた値を
  読み返しているだけ」になりうる、構造的に信用できない情報源である。冪等性は
  awase 側の遷移（INV-A/B）だけで担保する。
- **`tsf/send.rs` の warmup ヘルパーを流用する**: 追補1 がまさにこれをやって
  CapsLock 汚染を起こし、「warmup 用ヘルパー（直後に文字送信が続く前提で設計されたもの）を
  無関係な standalone トグル用途へ転用しない」という教訓を残している。

---

## 未解決の疑問（実機で確認すること）

1. ~~**なぜ追補1・追補3 で `[hook] IME-mode vk=0xF0` が一度も出なかったのか。**~~
   **決定0 の M3（2026-08-27）で部分的に解消**: マーカ付き（`IME_KANJI_MARKER`）
   `SendInput` の **KeyDown は毎回 `self_injected=true` として正しくログに出た**
   （`docs/known-bugs.md` BUG-25 追補5）。ただし別の未解決の疑問1a が新たに
   判明した（下記）。
   1a. **KeyUp だけが構造的にログへ出ない。** 同じ M3 計測で、KeyDown は毎回
       ログに出たのに対応する KeyUp が一度も出なかった。DOWN/UP を同一
       `SendInput` バッチで送っても、別々の呼び出しに分割し実時間50ms空けても
       再現し、タイミングは原因ではない。`SendInput` 自体は毎回成功
       （`sent=1/1`or`2/2`）を報告している。候補: (a) `wScan=0` の `VK_DBE_*`
       系は Windows 内部のキー状態追跡が機能せず、KeyUp が低レベルフック
       チェーンへ配送される前に握り潰される、(b) 別 VK へ変換されて
       `ImeKeyKind::from_vk` の対象外になっている（未確認）。
       **この疑問が未解決でも決定2 の実装は妨げられない**（モード切替自体は
       毎回成功しているため）が、決定2 に「settle 待ち」「連続発火の最小
       間隔」を追加する根拠になっている。
2. **Composition 中のトグルは「以後に打つ文字」のモードを実際に変えるのか。**
   6経路すべてで否定的/不確定。決定5 でこの状態を回避する設計にしたため実装の前提には
   ならないが、判明すれば決定5 を緩められる。
3. **mozc の `KeyEvent::HIRAGANA` は冪等な SET（`InputModeHiragana`）か。**
   決定4-c の非対称 exit の前提。`session/session.cc` / `session/keymap.h` で確認する。
4. **通行証（決定3）が物理 DBE キーに食われる窓は実測でどれくらいか。**
   決定3-b の定数導出と同じ計測で得られる。0 にはできないので、大きさを知った上で
   受け入れる。
5. **StickyKeys との相互作用**（BUG-25 未検証項目3、2026-07-11 から未解決）。
   StickyKeys 自体が「Shift 単独タップ」を検出してラッチするため、本機能とセマンティクスが
   競合する。決定8 の `off` はこのユーザー向けの逃げ道でもある。
6. **決定2 の synthetic Shift↑ が、実 IME 側で「Shift 単独タップ」と誤認されないか。**
   BUG-15 は MS-IME の Shift 単独タップ誤検知が出発点だった。GJI 側に同種の検知が
   あるかは未調査。実機で entry 直後にかな入力が乱れないかを見る。
7. ~~決定0 の M4（`ime_kanji`/起動/Shift 押下中）は未実施のまま。~~
   **解消（2026-08-27）: M4 実施済み、失敗（原因B 確定）。** ただし
   「なぜ Shift 押下中だけ KeyDown 自体がフックに届かないのか」という
   OS 側の正確なメカニズムは未確認のまま残る。候補: (a) Windows が
   Shift+特定 VK の組み合わせをシステムアクセラレータ/メニューキー処理系
   へ振り分け、低レベルフックチェーンに載せる前に横取りしている、
   (b) `wScan=0` の VK に対する内部状態追跡が Shift 修飾との組み合わせで
   さらに不安定になる、(c) 決定0 で確認できなかった何らかの別要因。
   決定2 の synthetic Shift↑ 前置が必須であることは実機で確定したため、
   この OS 側メカニズムの解明自体は実装の前提にはならない。
8. **「1文字目だけ全角英数になるレース」は `--sendinput-up-delay-ms` で
   本当に緩和されるのか。** 追補5 ではサンプル数1で緩和したように見えたが、
   複数回の再現実験で確認していない。
9. **短時間連続トグルでの不安定化は本当に drift correction / idle-conv-check
   由来か。** `[shadow-toggle]`/`[warrant-shadow]`/`[idle-conv-check]` の
   タイムラインを完全に相関させるまで確定しない（追補5）。

---

## 付録: 今後の調査（観測側、本ADRのスコープ外）

追補4 の示唆5 が残した宿題。アクチュエーションが決着したあとに着手する価値がある。

- `ITfInputProcessorProfileActivationSink`（spike の `--watch-profile`）による
  プロファイル切替の push 購読。現在 `tsf/observer.rs` が2秒ポーリングで読んでいる
  `ITfInputProcessorProfileMgr::GetActiveProfile` を push 化できる可能性。
- `ITfLangBarItemSink`（spike の `--watch-langbar`）による入力モードボタンの
  push 購読。言語バーのボタン表示は実 composition mode のミラーであるため、
  **ひらがな⇔英数の切替を外部プロセスから肯定的に観測できる**可能性がある。
  これが成立すれば、本ADRが「`SendInput` に完了通知が無いので消せない」と認めた
  残余リスク（決定4-a）に、初めて事後確認の手段が付く。
- `ITfCompartmentEventSink`: spike の module doc が記録しているとおり、
  「`ITfThreadMgr::QueryInterface` から取る」という訂正によって**再浮上した**未検証の
  候補。ただし「無関係な外部プロセスの `ITfThreadMgr` を自分で `Activate` した場合に、
  他プロセスの実 compartment 値まで見えるか」は別問題として未確認。

これらはいずれも Windows XP 時代からある公開・文書化された TSF COM インタフェースであり、
[[feedback_no_private_ipc_reverse_engineering]]（他社 IME の私的 IPC を覗かない）には
抵触しない——タスクバーの言語バー/IME インジケータ UI 自体がこれらを使っている。
