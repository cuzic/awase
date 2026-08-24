# ADR-097: 親指シフト×小指シフトの複合面（左親指小指シフト面／右親指小指シフト面）

## ステータス

**実装済み（Phase0/0.5/1、2026-08-19）だが、UIタブは2026-08-23時点で
一時的に非表示。** 実機確認で、この面に何も割り当てていない状態だと
親指+小指+文字キーの同時打鍵でアルファベットがそのまま出力されてしまう
ことが判明した。やまぶきR公式配布物（v1.11.1）を実際にダウンロードして
確認したが、この2面（`[ローマ字小指左親指シフト]`/
`[ローマ字小指右親指シフト]`）の既定配列は同梱のどの `.yab` にも
含まれておらず、やまぶきR自体もユーザーが独自に設定する空の面として
提供している。「やまぶきR互換の既定配列」を実現する一次情報源が無く、
デフォルトで安全に使える完成度に達していないと判断し、
`crates/awase-settings/src/main.rs` の配列エディタからこの2面のタブを
一時的に隠した（`Face` enum・`YabLayout` フィールド・`.yab` の
パース/シリアライズ・エンジンの面解決ロジックは維持しており、`.yab`
ファイルを直接編集すればこれまで通り機能する）。やまぶきR互換の既定配列が
別途確定するか、ユーザーが独自配列で運用する前提を許容すると判断した
時点で、このフィルタを外せばよい。

草案（2026-08-19）。独立レビュー（Opus）を1巡し
「Go with fixes」の11項目を反映済み（同日改訂）。

改訂で入った最重要の変更は**決定0 の新設**である。起票時、
`InputContext.left_thumb_down`/`right_thumb_down` が実機でも追跡されていると
誤って前提していたが、実際には全経路で `None` 固定であり、**そのままでは
本 ADR の中核が実機で no-op になる**ことがレビューで判明した。決定0 と
Phase 0.5 はこの欠落を埋めるためのものであり、省略できない。

## コンテキスト

### 要求

ユーザーから「やまぶきR に実装されている3同時打鍵（左親指小指シフト、
右親指小指シフト）を実装したい」という要望があった。

やまぶきR は各文字キーに対して、現在 awase が実装している4面

- シフト無し（`Face::Normal` / `[ローマ字シフト無し]`）
- 小指シフト（`Face::Shift` / `[ローマ字小指シフト]`、物理 Shift）
- 左親指シフト（`Face::LeftThumb` / `[ローマ字左親指シフト]`）
- 右親指シフト（`Face::RightThumb` / `[ローマ字右親指シフト]`）

に加えて、**親指キー + 小指シフト(Shift) + 文字キー**の3キー同時打鍵による
2面（`[ローマ字小指左親指シフト]` / `[ローマ字小指右親指シフト]`）を持つ。
NICOLA 標準の4面だけでは足りない記号等の追加入力面として使える。

本 ADR のスコープはこの2面のみ。**両親指同時打鍵**（やまぶきRの
`[拡張親指シフト1]`/`[拡張親指シフト2]`）と**英数系6面**（`[英数シフト無し]`
以下）は対象外とし、現在と同じく `FaceKind::Ignored`（受理のみ・機能未実装）
のまま残す。

### 既に整っている土台

調査の結果、この機能のための土台は既に部分的に存在していた。

**1. `.yab` のセクション名は既に予約済み。**
`src/yab/mod.rs:482-502` の `classify_section` が、やまぶきR互換のため
`"ローマ字小指左親指シフト"` / `"ローマ字小指右親指シフト"` を
`FaceKind::Ignored` として認識し、パースエラーにせず読み飛ばしている。
`src/yab/tests.rs:637` の `test_parse_yamabuki_compat_sections_are_accepted_but_ignored`
がこの受理挙動を固定している。**セクション名を新規に決める必要はなく、
やまぶきR と同じ名前をそのまま昇格させればよい。**

**2. 修飾キー状態は実機でも継続追跡されている。**
`InputContext.modifiers`（したがって `PhysicalKeyState.modifiers.shift`）は
`build_input_context`（`crates/awase-windows/src/runtime/mod.rs:63`）が
`event.modifier_snapshot`（hook 時点でキャプチャした実値）から構築しており、
実機で正しい値が入る。**Shift のレベル判定に必要な材料は既に届いている。**

> **注意: 親指キーの押下状態はこれに含まれない。** 起票時、`PhysicalKeyState`
> の `left_thumb_down`/`right_thumb_down` も同様に届いていると誤って前提して
> いたが、実際には実機経路で常に `None` である。この欠落は本 ADR の中核を
> 無効化するため、**決定0**として独立に扱う。

**3. Shift は OS 修飾キー扱いされていない。**
`ModifierState::is_os_modifier_held()`（`src/types.rs`、テストは
`src/engine/fsm_types.rs:970` `modifier_state_is_os_modifier_held_shift_only_is_false`）
は ctrl/alt/win のみを見る。したがって Shift 押下中でも
`NicolaFsm::bypass_reason`（`nicola_fsm.rs:1726`）の `OsModifierHeld` には
落ちず、文字キー・親指キーはエンジン処理へ到達する。

**4. 空セクションのフォールバックが既にある。**
`parse_optional_face`（`yab/mod.rs:516`）はセクションが存在しなければ空の
`YabFace` を返す。新セクション未定義の既存 `.yab` は自動的に空面になる。

### 現状の面選択が持つ「順序依存の非対称性」

本 ADR の設計上、最も重要な発見。**現在、Shift+親指+文字の3キーは押下順に
よって異なる面に解決される。**

**順序A: Shift↓ → 親指↓ → 文字↓**

1. Shift↓ は `KeyClassification::Passthrough`（`vk.rs:206` の
   `is_passthrough` が `0x10`/`0xA0`/`0xA1` を含む）。`bypass_reason` が
   `BypassReason::Passthrough` を返し `handle_bypass`（`nicola_fsm.rs:1743`）
   へ。状態が Idle なら `Response::pass_through()` のみ。
2. 親指↓ は Idle で `classify_idle_intent`（`nicola_fsm.rs:844`）へ。
   `should_use_shift_plane`（`:763`）は `&& !ev.key_class.is_thumb()` の
   ため false。`IdleIntent::ConfirmMode` → `idle_wait` → `PendingThumb`。
3. 文字↓ は `decide_pending_thumb` → `step_pending_thumb_char`（`:1083`）。
   **この関数は `phys.modifiers.shift` を一切見ない。** 閾値内なら
   `thumb.face()`（= `LeftThumb`/`RightThumb`）で確定。

→ **親指面が出る。ただし「Shift が完全に無視される」のは親指面にその位置の
定義がある場合に限る。** 親指面に定義が無ければ `step_pending_thumb_char` の
`candidate` が `None` になり「時間超過 or 候補なし」分岐（`:1107`）へ落ちて
親指キーを単独確定 → 文字キーを `ReduceAndContinue` で再処理する。再処理は
Idle で `classify_idle_intent` に入るため、そこで `should_use_shift_plane` が
効いて **Shift 面**が出る。つまり順序Aは「親指面に定義があれば親指面、
無ければ Shift 面」という位置依存の挙動になっている。

**順序B: 親指↓ → Shift↓ → 文字↓**

1. 親指↓ → `PendingThumb`。
2. Shift↓ は Passthrough バイパス。状態が Idle でないため
   `handle_bypass` が `flush_pending(ContextChange::BypassKey)` を呼び、
   `PendingThumb` を `resolve_pending_thumb_as_single`（`:1216`）で単独確定
   → 無変換/変換の既定は `ModeKeyConfig{idle: Suppress, composing: Suppress}`
   なので**何も出力されずに** Idle へ。ただし `phys.left_thumb_down` と
   `left_thumb_consumed` は変わらないため、`active_thumb_face()`（`:1697`）
   は引き続きその親指面を返す。
3. 文字↓ は Idle で `classify_idle_intent` へ。分岐順は
   **`should_use_shift_plane` が `active_thumb_face()` より先**（`:854` vs
   `:858`）なので `IdleIntent::ShiftPlane` が勝つ。

→ **小指シフト面が出る。親指キーが完全に無視される。**

この非対称性は誰も意図して設計したものではなく、Shift 面ルーティング
（2026年3月、`72bd118`）と親指同時打鍵判定が独立に育った結果の副産物である。
**本 ADR の変更は、この2つの経路を単一の面解決関数へ寄せることで非対称性を
解消することが本体であり、新しい2面はその帰結として自然に入る。**

### 既存の Space/Enter リテラル特例

`is_space_thumb_shift_literal`（`nicola_fsm.rs:825`）/
`is_enter_thumb_shift_literal`（`:836`）は、親指キーが Space/Enter に
割り当てられている場合、Shift 同時押しなら同時打鍵判定を一切試みず即座に
リテラル送出する。doc コメントは根拠をこう書いている:

> NICOLA の小指シフト面（Shift 単独系）と親指シフト（同時打鍵系）はそもそも
> 組み合わせない設計のため、Shift 押下中の Space 親指キーを `PendingThumb` に
> 入れず即座に素通しにしても、通常の同時打鍵判定と衝突しない。

**本 ADR はこの「組み合わせない」という前提を明示的に破る拡張である。**
この2つの特例をどう温存するかが設計上の最大の論点（決定3）。

### Windows プラットフォーム層の risk（当初想定からの重要な訂正）

本 ADR の起票時、`kp_shift_conv_guard_key_down`
（`crates/awase-windows/src/runtime/key_pipeline.rs:1207`）が Shift 押下時点で
`actuate_conv_mode(HalfWidthAlnum)` を呼び conv を英数へ先書き込みすることが
BUG-49（`docs/known-bugs.md:5308`）/ BUG-58（同 `:7005`）と同型の罠を
再発させるリスクとして懸念されていた。

**このリスクは既に構造的に消えている。** 実コードを読んで確認した:

- 2026-08-09（known-bugs.md BUG-15 追補9）に**チョードに対する conv 先書き込み
  そのものが撤去された**。現在の `kp_shift_conv_guard_key_down`
  （`key_pipeline.rs:1207-1264`）が行うのは
  (a) `left_shift_tap_candidate` の設定、(b) `shift_conv_guard_pending` の設定、
  (c) かな入力コンテキストでなければ pending を落とす、の3つだけで、
  **`actuate_conv_mode` の呼び出しは存在しない**（`:1250-1263` の NOTE が
  撤去理由を記録している）。
- conv=0x0000 の実書き込みは `kp_shift_conv_guard_key_up`（`:1266-1366`）の
  「本物の左Shift単独タップと確定した瞬間」に一本化された。
- BUG-58 の循環待ち（`OutputActiveGuard` × conv 復元）の直接の引き金は
  この先書き込みだったため、チョードに対しては原理的に発生しなくなっている。

したがって本 ADR が新たに検討すべき Windows 側の相互作用は、
**「Shift+親指+文字のチョードが誤って左Shift単独タップと判定されないか」**
という一点に絞られる（決定5でコード上の根拠付きで検証する）。

## 決定

### 決定0（前提条件・最優先）: `InputContext` に親指キーの押下状態を実際に配線する

**本 ADR の他のすべての決定は、この配線が済んでいることを前提にする。
配線しなければ、以下の設計は実機で完全な no-op になる。**

#### 事実（実コードで確認済み、2026-08-19）

`InputContext.left_thumb_down` / `right_thumb_down` は、**実機の全経路で
常に `None` に固定されている。**

- `crates/awase-windows/src/runtime/mod.rs:63-79` の `build_input_context` は
  `left_thumb_down: None, right_thumb_down: None` をリテラルで書いている
  （引数にすら取っていない）。呼び出し元は2箇所（`runtime/key_pipeline.rs:100`
  のホットパスと `runtime/mod.rs:239` の `build_ctx()`）で、どちらもこの
  `None` 固定の値を受け取る。
- `crates/awase-linux/src/main.rs:113-126` も `InputContext` を直接構築して
  おり、同じく `left_thumb_down: None, right_thumb_down: None`。
- `PhysicalKeyState::from_ctx`（`src/engine/input_tracker.rs:56-64`）は
  `ctx.left_thumb_down` をそのままコピーするだけなので、`Engine::on_input`
  （`src/engine/engine.rs:365`）が組み立てる `PhysicalKeyState` の親指状態も
  常に `None` になる。

`InputTracker`（`src/engine/input_tracker.rs:84`、`update_thumb_state` で
実際にタイムスタンプを記録する唯一の実装）は、**テストからしか使われていない**:
`src/engine/tests.rs`（`TestHarness`）、`src/engine/proptest_tests.rs:118`、
`src/engine/fsm_adapter.rs` のテストモジュール、
`crates/awase-windows/tests/e2e_windows.rs:60`。製品コードの呼び出し元は無い
（`crates/awase-macos/src/main.rs:60` は「InputTracker」という語をコメントで
使っているだけで、実際には `InputTracker::process` を呼んでいない）。

#### 帰結

`NicolaFsm` のうち、親指の**物理押下状態**に依存する次の機構は、
**現在すべて実機で不活性**である。

| 機構 | 実機での実際の挙動 |
| --- | --- |
| `active_thumb_face()`（`nicola_fsm.rs:1697`） | `phys.left_thumb_down.is_some()` が常に false → **常に `None`** |
| `IdleIntent::ActiveThumb`（`:859`） | 到達不能 |
| `reduce_active_thumb`（`:887`） | 到達不能 |
| `consume_thumb`（`:648`） | `self.phys.*_thumb_down`（=`None`）を代入するだけ |
| `is_thumb_consumed`（`:1687`） | `phys_down.is_some()` が false → **常に false** |

実機の親指シフトは、これらではなく `PendingThumb`/`PendingCharThumb` という
**FSM 内部の状態**だけで成立している（`PendingThumbData` はイベントから直接
構築されるため `InputContext` の欠落の影響を受けない）。つまり現状は
「動いているが、二重シフト防止の消費機構だけが死んでいる」状態である。

**したがって、決定2の中核（`classify_idle_intent` の分岐順逆転による順序A/B
の対称化）は、配線なしでは実機で一切効かない。** さらに悪いことに、
テスト計画は `TestHarness`（= `InputTracker` を使う）経由で書くため
**テストは全部緑になるのに製品の挙動は変わらない**。この乖離は本 ADR で
最も危険な失敗モードであり、決定0 として独立に切り出す理由である。

#### 決定

`build_input_context` に `left_thumb_down: Option<Timestamp>` /
`right_thumb_down: Option<Timestamp>` の2引数を追加し、実際の押下状態を渡す。

追跡そのものは hook 層に置く。`crates/awase-windows/src/hook.rs` は既に
`CACHED_THUMB_VKS`（`:141`）で親指 VK を保持し `hook_config()`（`:367`）で
返しているので、`ALT_L_WAS_DOWN`（`:82` 付近）と同じ静的アトミックの
パターンで押下タイムスタンプを持たせるのが既存様式に沿う。あるいは
`platform_state` にフィールドを置き `kp_stage_*` の早い段階で更新してもよい
（**どちらにするかは実装時に決めてよいが、`classify_key` が
`KeyClassification::LeftThumb`/`RightThumb` を返すのと同じ判定材料を使い、
Alt なりすまし（`apply_alt_impersonation`、`hook.rs:73`）で書き換わった後の
VK を見ること**——なりすまし中の Alt は親指キーとして振る舞うため）。

`crates/awase-linux/src/main.rs` にも同じ配線を入れる。macOS は現状
`InputContext` を構築する経路が確認できていないため、Phase 0.5 の作業時に
併せて調査する。

#### 副次的な効果（この配線自体が既存バグの修正である）

配線すると `is_thumb_consumed`/`consume_thumb` が初めて実機で機能する。これは
[ADR-008](008-physical-thumb-state-separation.md)（物理親指状態の分離）と
[ADR-010](010-thumb-consumption-timestamp.md)（消費タイムスタンプ）が意図した
「同じ親指の押下で後続キーが二重にシフトされるのを防ぐ」機構であり、現在は
実機で無効化されている。**Phase 0.5 は本 ADR の前提であると同時に、
ADR-008/010 の積み残しの解消でもある。**

ただしこれは**挙動変更を伴う**——親指を押しっぱなしにして複数の文字キーを
連続で打った場合、現在は（消費機構が死んでいるため）2文字目以降も
`PendingThumb` 経路には入らず通常面になるが、配線後は `active_thumb_face()`
が生きて2文字目が親指面になる可能性がある。**Phase 0.5 の実機確認では、
複合面とは無関係にこの「親指ホールド連打」の挙動を必ず確認すること。**
想定外の回帰が出た場合は、複合面の実装ではなく決定0 の配線側を疑う。

### 決定1: `Face` を 6 variant のフラット enum へ拡張し、解決を単一関数に集約する

`src/engine/fsm_types.rs:81` の `Face` を次の6値にする。

```rust
pub enum Face {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
    LeftThumbShift,   // 新規
    RightThumbShift,  // 新規
}
```

#### 検討した代替案と却下理由

**案B: `(thumb_side: Option<ThumbSide>, shift_held: bool)` の直交構造体。**

6面は確かに `(shift: 無/有) × (thumb: 無/左/右)` の完全な直積であり、
やまぶきRの `[英数…]` 6面も同じ構造を持つ。表現としては案Bが忠実である。
しかし却下する。

- **却下の主因: 網羅性検査を失う。** 現在 `Face` を `match` で受けている
  箇所は `get_face`（`nicola_fsm.rs:588`）、`consume_thumb`（`:648`）、
  `is_thumb_consumed`（`:1687`）の3つで、いずれも `Face::Normal | Face::Shift => …`
  という「親指面以外」のアームを持つ。フラット enum のまま variant を足すと
  **この3箇所が非網羅でコンパイルエラーになり、コンパイラが更新必須箇所を
  列挙してくれる**。構造体にして `.thumb` と `.shift` を別々に見る形に
  すると、この強制が消える。「新しい組み合わせを考慮し忘れる」ことこそ本
  リポジトリが繰り返してきた失敗様式（`.claude/rules/fix-requires-evidence.md`
  の「再発ファミリー」表に「キー選択」がある理由）であり、コンパイラ強制を
  手放す変更は割に合わない。
- **不正状態の排除という観点では両案に差が無い。** 案Bの
  `{thumb: None, shift: true}` は `Face::Shift` と同義で不正ではない。
  案Bの利点は「不正状態を作れないこと」ではなく「対称性の表現」だけである。

**案C: `Face::Thumb { side, shift }` + `Normal` + `Shift` の3 variant。**
`get_face` は素直になるが、`Face` が `Copy + PartialEq` のまま比較・保存
される既存コード（`IdleIntent::ActiveThumb(Face)`、`PendingThumbData::face()`）
の可読性が落ち、案Aに対する利得が小さい。却下。

#### 案Aに付随して導入するヘルパー

フラット enum の欠点（6アームの `match` が増える）は、意味のあるアクセサで
吸収する。

```rust
impl Face {
    /// この面が消費する親指キー（親指面でなければ None）。
    /// consume_thumb / is_thumb_consumed の 6 アーム match を潰す。
    pub const fn thumb_side(self) -> Option<ThumbSide> { … }

    /// 親指の押下側と小指シフトの押下状態から面を一意に決める。
    /// **面解決の唯一の入口**（決定2）。
    pub const fn resolve(thumb: Option<ThumbSide>, shift_held: bool) -> Self { … }
}
```

`ThumbSide` は `{ Left, Right }` の新規 enum。既存の `is_left: bool`
（`PendingThumbData`、`Face::from_thumb_bool`）はそのまま残す
（本 ADR のスコープを膨らませない。`ThumbSide` は `Face` 内部の表現に留め、
`from_thumb_bool` は `resolve` へ委譲する薄いラッパにする）。

#### ⚠️ コンパイラ強制が効かない2箇所（手作業で確実に更新すること）

案Aの利点は「`match` の網羅性検査が更新必須箇所を列挙してくれる」ことだが、
**`Face` を `match` せず `get_face` 経由でベタ列挙している箇所には効かない。**
variant を足してもコンパイルが通ってしまうため、実装時に必ず手で拾うこと。

1. **`is_layout_key`（`nicola_fsm.rs:1713-1723`）** — `has_output(self.get_face(
   Face::Normal)) || … || has_output(self.get_face(Face::Shift))` の4連 OR。
   複合面を足し忘れると、**複合面にしか定義が無いキーが
   `IdleIntent::PassThrough`（`:867`）へ落ちて機能しない**。本 ADR で最も
   踏みやすい落とし穴。
2. **`idle_ngram`（`confirm_policy.rs:98-126`）** — Normal / LeftThumb /
   RightThumb の3面を個別に `lookup_face` して `should_speculate` に渡す。
   複合面を足し忘れても静かに「複合面を考慮しない投機判断」になるだけで、
   エラーにもテスト失敗にもなりにくい（決定6 Phase 0 の項目4で扱う）。

この2箇所を `tests/architecture_guard.rs` 相当のテキスト検査で固定する案も
考えたが、`src/` 側（`awase` crate）には同種の仕組みが無いため、代わりに
**この2箇所を直接対象にしたユニットテスト**（複合面にしか定義が無いキーが
`is_layout_key` で true になること）をテスト計画に入れる。

#### `YabLayout` / `.yab` フォーマット

`src/yab/mod.rs:236` の `YabLayout` に2フィールドを追加する。

```rust
pub struct YabLayout {
    pub name: String,
    pub normal: YabFace,
    pub left_thumb: YabFace,
    pub right_thumb: YabFace,
    pub shift: YabFace,
    pub left_thumb_shift: YabFace,   // [ローマ字小指左親指シフト]
    pub right_thumb_shift: YabFace,  // [ローマ字小指右親指シフト]
}
```

`FaceKind`（`yab/mod.rs:506`）に `LeftThumbShift` / `RightThumbShift` を
追加し、`classify_section`（`:482`）でその2つを `FaceKind::Ignored` から
昇格させる。`parse` は既存の `parse_optional_face` をそのまま使う。

**後方互換性:**

- **読み込み**: `parse_optional_face` が未定義セクションに対して空の
  `YabFace` を返すため、既存の `layout/nicola.yab`・`layout/nicola_us.yab`・
  `layout/nicola_f.yab` は変更なしでそのままパースでき、新2面は空になる。
- **書き出し**: `YabLayout::serialize`（`:646`）は新2面を
  **`YabFace::is_empty()` が false のときだけ出力する**。これにより、既存
  レイアウトを awase-settings で開いて保存しても余分な空セクションが
  書き込まれず、ラウンドトリップがバイト等価に保たれる。既存4面は従来通り
  常に出力する（空でも出力する現在の挙動を変えない）。
- **副次的な改善**: 現在 `sections: FxHashMap<FaceKind, Vec<String>>`
  （`yab/mod.rs:3`）は複数の `Ignored` セクションを同一キーに詰め込むため、
  最後の1つ以外が黙って捨てられている（読まれないので実害は無い）。2つを
  独立 variant へ昇格させることで、少なくともこの2面についてはこの取りこぼしが
  無くなる。

### 決定2: Shift は「ゲート条件（レベル信号）」として扱う。チョード判定の第3要素にはしない

`should_use_shift_plane` と同じ発想を採る。すなわち、**Shift は押しっぱなしで
保持される前提のレベル信号**であり、親指キー×文字キーのペアが確定する
「その瞬間」に `self.phys.modifiers.shift` を読んで面を決める。d1/d2 的な
タイミング仲裁の対象にはしない。

#### 根拠

**(a) 構造上、Shift をトークンにできない。**
Shift の VK は `is_passthrough()`（`vk.rs:206`）に含まれ、
`classify_key`（`hook.rs:36`）が `KeyClassification::Passthrough` を返す。
`bypass_reason` はこれを見て `ShiftReduceParser` のループに入る前に
バイパスする。Shift をチョードのトークンにするには `KeyClass` を作り替えて
バイパスを外す必要があり、そうすると Ctrl+Shift+○ 等の OS ショートカットが
壊れる。**代償が釣り合わない。**

**(b) Shift には解決すべき曖昧性が無い。**
`step_pending_char_thumb_3key`（`:1390`）の d1/d2 仲裁が存在するのは、
「文字1 → 親指 → 文字2」で**親指が文字1と文字2のどちらとペアになるか**が
本質的に曖昧だから（NICOLA 規格の核心、`docs/experiments.md`）。Shift には
この曖昧性が無い——Shift 自身は出力を持たず、「Shift が文字1に付くか文字2に
付くか」という競合が原理的に発生しない。押下中か否かの二値だけで決まる。
**タイミング仲裁を足しても解ける問題が増えない。**

**(c) ユーザー体感と一致する。**
実際のタイピングで小指シフトは先行して押され保持される。「ピアノの和音の
ように同時に押す」という NICOLA の設計思想が要求するのは親指と文字の同時性
であり、修飾キーの同時性ではない（NICOLA 規格が同時打鍵と呼ぶのは親指シフト
そのもので、小指シフトは規格外の拡張である）。

#### 実装位置: 面解決を4箇所から1関数へ集約する

親指面が決まる箇所は**9つ**ある。すべて `resolve_thumb_face`
（3段フォールバックチェイン、後述）経由に置き換える。

| # | 現在の箇所 | 現在の面決定 | 変更後 |
| --- | --- | --- | --- |
| 1 | `classify_idle_intent`（`:858-865`） | `active_thumb_face()` | `resolve_thumb_face(side, ev.pos)` |
| 2 | `step_pending_thumb_char`（`:1086`） | `thumb.face()` | `resolve_thumb_face(thumb.side(), ev.pos)` |
| 3 | `step_pending_char_thumb`（`:1044-1046`、閾値判定用の候補 kana） | `Face::from_thumb(ev.key_class)` | `resolve_thumb_face(side, pending.pos)` |
| 4 | `step_speculative_thumb`（`:1011`） | `Face::from_thumb(ev.key_class)` | `resolve_thumb_face(side, pending.pos)` |
| 5 | `compute_prefer_char1`（`:1354`） | `thumb.face()` | `resolve_thumb_face`（決定4） |
| 6 | `step_pending_char_thumb_3key`（`:1392`、char2 到着） | `thumb.face()` | `resolve_thumb_face` |
| 7 | `handle_key_up_pending_char_thumb`（`:1492`、char1/thumb の KeyUp） | `thumb.face()` | `resolve_thumb_face` |
| 8 | `timeout_pending_char_thumb`（`:1623`/呼び出し元 `:1827`） | `thumb.face()` | `resolve_thumb_face` |
| 9 | `flush_pending` の `PendingCharThumb` アーム（`:359`） | `thumb.face()` | 下記の通り**意図的に据え置く** |

#### `PendingCharThumb` には4つの出口がある（起票時に見落としていた）

`PendingCharThumb` 状態は、次の**4通りの終わり方**をする。当初の ADR は
このうち1つ（char2 到着 = #6）だけを複合面対応にしており、残り3つが
`thumb.face()` のままだと**同じ物理操作が「終わり方」によって別の面に解決
される**——本 ADR が解消しようとしているのと同型の、新しい非対称性を作って
しまう。

| 出口 | 契機 | 該当 |
| --- | --- | --- |
| char2 到着 | 3鍵目の文字キー | #6 |
| char1 または thumb の KeyUp | 指を離した | #7 |
| `TIMER_PENDING` 満了 | 閾値時間が経過 | #8 |
| バイパスキーによる flush | Ctrl/Alt/IME制御キー等の割り込み | #9 |

**#7・#8 は #6 と同じく `resolve_thumb_face` へ統一する**（実用上いちばん
多い終わり方は #7 の KeyUp である。ここを直さないと「ゆっくり打つと複合面、
素早く打つと親指面」のような時間依存の分岐になり、最悪の user experience に
なる）。

**#9（`flush_pending`）だけは意図的に据え置き、`thumb.face()`（＝ Shift を
見ない親指面）のままにする。** 理由:

- `flush_pending` は `ContextChange`（`ImeOff` / `FocusChanged` /
  `LayoutSwapped` / `EngineDisabled` / `BypassKey`）による**異常系の後始末**
  であり、「ユーザーが意図した和音」の解決ではない。同じ関数が
  `ComposingHint::Unknown` を受けたときに Space 例外まで捨てて無条件
  suppress する（`nicola_fsm.rs:339-345`）のと同じ「安全側に倒す」方針。
- フォーカス変更経由の flush では、`phys.modifiers.shift` は**切り替わった
  後の新しいウィンドウ**の状態を指しうる（`flush_pending` の doc が
  `composing` について同じ危険を長文で警告している、`:300-308`）。信頼
  できない Shift レベルで面を変えるのは、この警告に真っ向から反する。
- 実害が小さい。#9 に到達するのは Ctrl/Alt が割り込んだ場合等で、
  そのとき出るのが複合面か親指面かの差はユーザーには区別しにくい。

この据え置きは**意図的な非対称であり、忘れられた漏れではない**。
`flush_pending` の該当アームにその旨のコメントを必ず書くこと。

#### 変更後の列は `Face::resolve` ではなく `resolve_thumb_face`

上表の「変更後」列がすべて `resolve_thumb_face`（フォールバック込み）で
あって素の `Face::resolve` でないことは重要である。

> **不変条件: `Face::resolve` は `resolve_thumb_face` の内部からのみ呼ぶ。**
> 他の場所から直接呼んではならない。

素の `Face::resolve` に置き換えると、**複合面が部分定義**（一部のキーだけ
定義してある `.yab`）で後方互換性が壊れる。具体例:

`[ローマ字小指左親指シフト]` に記号キーだけ定義し、`k` の位置は未定義だと
する。ユーザーが Shift+左親指+`k` を打つと、素の `Face::resolve` は
`LeftThumbShift` を返す → `step_pending_thumb_char` の `candidate` が
`None` → 「時間超過 or 候補なし」分岐（`:1107`）へ落ちて親指キーを単独確定
し `k` を再処理 → Idle で `should_use_shift_plane` が効いて **Shift 面の
`Ｄ`** が出る。**期待は「複合面に無いなら親指面の `ｒｉ`」なのに、
まったく無関係な文字になる。** `resolve_thumb_face` を通せば手順2で
`LeftThumb` に落ちるためこれが起きない。

同じ理由で、**閾値判定に使う `candidate_kana` もフォールバック後の面から
引くこと**（表の #3）。`step_pending_char_thumb`（`:1045-1050`）と
`step_pending_thumb_char`（`:1087-1092`）は候補仮名を
`TimingJudge::is_simultaneous` に渡して n-gram で閾値を動的調整している。
未定義の複合面から `None` を引くと `adjusted_threshold` が静かに変わり、
**新しいタイミング定数を1つも足していないのに同時打鍵の判定時間が変化する**
という追跡困難な回帰になる。フォールバック後の面から引けばこれは起きない
（決定4 の「定数は増やさない」を実質的に担保しているのはこの一点である）。

#### `classify_idle_intent` の分岐順を入れ替える

現在の `should_use_shift_plane`（`:854`）→ `active_thumb_face()`（`:858`）
の順を、**`active_thumb_face()` → `should_use_shift_plane` の順に逆転させる。**

```rust
fn classify_idle_intent(&self, ev: &ClassifiedEvent) -> IdleIntent {
    // Space/Enter の Shift リテラル特例は据え置きで最優先（決定3）
    if self.is_space_thumb_shift_literal(ev) { return IdleIntent::PassThrough; }
    if self.is_enter_thumb_shift_literal(ev) { return IdleIntent::PassThrough; }

    // 未消費の親指キーが押下中なら、Shift の有無を含めて面を決める。
    // ここを should_use_shift_plane より先に置くのが本 ADR の変更点。
    if !ev.key_class.is_thumb() {
        if let Some(side) = self.active_thumb_side() {
            if let Some(face) = self.resolve_thumb_face(side, ev.pos) {
                return IdleIntent::ActiveThumb(face);
            }
        }
    }
    if self.should_use_shift_plane(ev) { return IdleIntent::ShiftPlane; }
    …
}
```

これにより**順序A・順序Bの両方が同じ `LeftThumbShift` 面に解決される**
（コンテキスト節の非対称性の解消）。順序Bで Shift↓ が `PendingThumb` を
flush しても、`phys.left_thumb_down` と `left_thumb_consumed` は変化しない
ため `active_thumb_side()` が生き残る点が効いている。

#### 面解決チェイン（後方互換性の要）

`resolve_thumb_face(side, pos)` は次の順で最初に**定義がある**面を返す。

1. `Face::resolve(Some(side), shift_held)`（Shift 押下時は複合面、非押下時は
   従来の親指面）に `pos` の定義があればそれ。
2. 無ければ `Face::resolve(Some(side), false)`（= 従来の親指面）。
3. それも無ければ `None`（呼び出し元が従来通り Shift 面 → 確定モードへ
   フォールスルー）。

「定義がある」は `YabFace::contains_key`（`yab/mod.rs:168`）の意味、すなわち
**`無`（`YabValue::None`）は「定義あり・出力なし」として扱う**。これは
`lookup_face` が `Some(KeyAction::Suppress)` を返す既存の区別
（`yab/mod.rs:467-471` の「`YabValue::None`（'無'）も格納する」コメント）と
一致する。ユーザーは複合面に `無` と書くことでフォールバックを明示的に
遮断できる。

**このチェインが後方互換性の全体である。** 新セクション未定義の既存 `.yab`
では手順1が必ず外れて手順2に落ち、順序Aは現在と完全に同一の出力になる。
順序Bだけが「小指シフト面 → 親指面」に変わるが、これはコンテキスト節で
示した意図せぬ非対称性の是正であり、意図的な挙動変更として記録する。

### 決定3: Space/Enter リテラル特例は据え置き、新面は `KeyClass::Char` のみに適用

`is_space_thumb_shift_literal` / `is_enter_thumb_shift_literal` は
**一切変更しない**。`classify_idle_intent` の先頭2分岐という現在の位置も
維持する（上のコード片参照）。

**帰結**: 親指キーが Space または Enter に割り当てられていて、かつ
`space_thumb_shift_literal` / `enter_thumb_shift_literal` が `true`（既定）
の場合、**その側の複合面は事実上到達不能になる**。Shift+Space はリテラルな
スペース、Shift+Enter はソフト改行のままである。

これは意図的な制限とする。理由:

- Space/Enter は IME の正規機能キー（変換候補送り・変換確定）であり、
  `TextKeyConfig` の doc（`fsm_types.rs:555-570`）が記録する通り、これらを
  抑制すると通常の変換操作そのものが壊れる。「Shift+Space で明示的に半角
  スペースを打つ」はユーザーが日常的に使うエスケープハッチである。
- 既に `src/engine/tests.rs:1000` `test_shift_space_literal_passthrough_when_enabled` /
  `:1121` `test_shift_enter_literal_passthrough_when_enabled` が固定している
  挙動であり、複合面のために壊す価値がない。
- **逃げ道は既にある**: `space_thumb_shift_literal = false`（`GeneralConfig`）
  にすれば従来通り `PendingThumb` に入り（`tests.rs:1022`
  `test_shift_space_enters_pending_when_literal_disabled` が固定）、複合面に
  到達する。Space を親指キーにしたまま複合面も使いたいユーザーはこの設定を
  切ればよい。

**`KeyClass::Char` 限定の境界は構造的に保証される。** 面解決が走る4箇所は
いずれも文字キー側が `KeyClass::Char` である経路にしか存在しない:

- `classify_idle_intent` の該当分岐は `!ev.key_class.is_thumb()` でガード済み。
  `KeyClass::Passthrough` は `bypass_reason`（`:1727`）が `on_key_down` の
  時点で弾いており、パーサループに到達しない。したがって残るのは `Char` のみ。
- `step_pending_thumb_char` は `decide_pending_thumb`（`:926`）の
  `KeyClass::Char` アームからしか呼ばれない。
- `resolve_char_thumb_as_simultaneous` は `PendingCharThumb` 状態からのみ
  呼ばれ、その状態は `step_pending_char_thumb`（`:1041`、`PendingChar` +
  親指）経由でしか作られない。`PendingChar` に入るのは文字キーだけ。
- `step_speculative_thumb` の `pending` は `SpeculativeChar`、すなわち文字キー。

**新しい `if ev.key_class == KeyClass::Char` チェックを足す必要は無い。**
この不変条件を `Face::resolve` の doc コメントに明記し、
`src/engine/tests.rs` に「Passthrough キーが複合面に到達しない」ことを
固定するテストを1本置く（テスト計画参照）。

#### 親指キーが Shift 修飾キーに割り当てられている場合の罠

`src/engine/tests.rs:867` が示す通り、**親指キーに Shift 自身を割り当てる
ことは現在可能**である（`modifier_key: Some(ModifierKey::Shift)`、単独タップは
`resolve_pending_thumb_as_single`（`:1225`）が無条件 suppress する）。

この設定では、その親指キーを押した瞬間に `phys.modifiers.shift` が true に
なるため、**すべての親指+文字チョードが複合面に解決されてしまう**。これは
明白な回帰である。

**対策**: `NicolaFsm` に `thumb_shift_faces_enabled: bool` を持たせ、
左右いずれかの親指キーが Shift 修飾キーにバインドされている場合は
`false` にする。`Face::resolve` の呼び出し側で false なら `shift_held` を
強制的に `false` として扱う（= 完全に従来挙動）。

判定材料は既に手元にある——`ClassifiedEvent.modifier_key`
（`fsm_types.rs:60`）と `PendingThumbData.modifier_key`（`:405`）。ただし
`active_thumb_face()` 経路には `PendingThumbData` が無いため、**設定時に
一度だけ計算する専用の setter**（`set_thumb_shift_faces_enabled(bool)`）を
新設し、Platform 層の bootstrap が親指キー VK の分類結果から渡す形にする。
`set_thumb_key_solo_tap_config` 等と同じ「Platform 層が VK を判定して core に
渡す」パターン（`nicola_fsm.rs:126` の doc 参照）に従う。

**このフラグは左右まとめて1つである。** 片方の親指キーだけを Shift に
割り当てているユーザーは、問題の無いもう片方の複合面まで無効になる。左右
独立の2フラグにもできるが、v1 では単一フラグを採る——この設定自体が極めて
稀（親指キーに Shift を割り当てると単独タップが常時 suppress される、
`resolve_pending_thumb_as_single`（`:1225`）ため実用性が低い）であり、
状態を2つに増やす価値が無い。ユーザーから要望が出たら分割する。

#### 左右の対称性について（明示的に決めておく）

- **右Shift でも複合面は有効。** `should_use_shift_plane` が見る
  `phys.modifiers.shift` は `ModifierState::update`（`src/types.rs:126`）が
  `ModifierKey::Shift` で更新する値であり、`classify_modifier`
  （`vk.rs`）は `VK_SHIFT`/`VK_LSHIFT`/`VK_RSHIFT` を等しく `Shift` に
  分類する。左右の区別は無い。
  **左右が非対称なのは `shift-conv-guard` の単独タップ判定だけ**である
  （`kp_shift_conv_guard_key_down`（`:1209`）が `VK_LSHIFT` のみを
  `left_shift_tap_candidate` の対象にする。右Shift はトグルの緊急解除専用）。
  これは conv 制御の都合であって面選択とは無関係なので、複合面は左右どちらの
  Shift でも同じように使える。
- **両親指を同時に押した場合は左を優先する。** `active_thumb_face()`
  （`:1697-1706`）が `if left … else if right …` の順で見ている既存の
  優先順位をそのまま引き継ぐ（`active_thumb_side()` も同じ順にする）。
  両親指同時打鍵を独立の面として扱うのはスコープ外（未解決の論点4）なので、
  ここで新しい規則を作らない。

### 決定4: 3鍵逐次仲裁（`step_pending_char_thumb_3key`）はスコープに含める。ただし新しい軸は足さない

「文字1 → 親指 → 文字2」の d1/d2 仲裁（`nicola_fsm.rs:1390`,
`compute_prefer_char1`（`:1344`））は、**面の差し替えだけ行い、仲裁ロジック
そのものには手を入れない。**

具体的には `compute_prefer_char1` が n-gram スコアリングに使う3つの候補仮名

```rust
let char1_thumb_kana  = self.lookup_kana_at(pending.pos, thumb_face);
let char1_single_kana = self.lookup_kana_at(pending.pos, Face::Normal);
let char2_thumb_kana  = self.lookup_kana_at(ev.pos, thumb_face);
```

の `thumb_face` を **`resolve_thumb_face` 由来**のものに差し替える（素の
`Face::resolve` ではない。決定2の不変条件。複合面が部分定義のとき
`lookup_kana_at` が `None` を返して n-gram スコアが静かに歪むのを防ぐ）。
`Face::Normal` 側は変更しない——Shift 面は「親指を使わなかった場合」の
候補ではない。親指を使わない場合の対抗候補は決定2の分岐順により Shift 面
ではなく通常面のままである。

#### v1 で対象外とすること

- **Shift レベルの変化を仲裁の材料にしない。** 「文字1 → 親指 → Shift↓ →
  文字2」のように途中で Shift が押される混在ケースは、v1 では
  **安全側に倒して対象外**とする。この順序では Shift↓ が
  `bypass_reason` → `handle_bypass` → `flush_pending` を通り、
  `PendingCharThumb` が「文字1+親指の同時打鍵」として**その場で確定**される
  （`flush_pending` の `PendingCharThumb` アーム、`:353-367`）。**決定2の表
  #9 でこのアームを `thumb.face()` のまま据え置くと決めた**ので、文字1+親指は
  Shift 無しの親指面、文字2 は Shift 面（または複合面）となる。
  これは「ユーザーが和音を作り終えてから Shift を押した」という素直な解釈と
  一致するため、**追加のガードは不要**。この挙動をテストで固定する
  （テスト計画ケース15。決定2の表 #9 と期待値が一致していることを、
  実装前に必ず突き合わせること——起票時の草案では表と矛盾していた）。
- **Shift の押下タイミングを閾値判定に混ぜない。** `is_simultaneous` /
  `three_key_pairing`（`src/engine/timing.rs`）のシグネチャは変更しない。

#### 新しいタイミング定数は導入しない

本 ADR は `crates/awase-windows/src/tuning.rs` に定数を1つも追加しない。
同時打鍵の閾値は既存の `threshold_us` / n-gram 判定をそのまま再利用する
（Shift はレベル信号でありタイミング要素ではないため、待つべき新しい時間が
存在しない）。したがって `.claude/rules/tuning-constants.md` の実測義務は
本 ADR には**適用されない**。

**将来、実機で「Shift 保持中のチョードだけ閾値が足りない」という症状が出た
場合**は、値を上げる前に同ルールに従って次を実測すること: (1) Shift 押下
から文字キー到達までの実測 ms、(2) Shift 非保持時の同じチョードの d1 実測
との差分。差が有意でなければ閾値の問題ではなく面解決の問題である。

### 決定5: Windows プラットフォーム層 — ガードの誤発火は既に構造的に防がれている。順序不変条件のみ守る

コンテキスト節で述べた通り、チョードに対する conv 先書き込みは 2026-08-09 に
撤去済みで、BUG-49/BUG-58 の機序は再現しない。残る検証対象は
「Shift+親指+文字が誤って**左Shift単独タップ**と判定され、
`half_width_alnum_toggle_active`（IME-ON 半角英数の持続トグル）が立たないか」
の一点。

**コード上の根拠**（`key_pipeline.rs:1190-1195`）:

```rust
if matches!(event.event_type, KeyEventType::KeyDown)
    && !event.injected
    && event.vk_code != crate::vk::VK_LSHIFT
{
    self.platform_state.gate.left_shift_tap_candidate = false;
}
```

親指キーの KeyDown も文字キーの KeyDown も、非注入かつ `VK_LSHIFT` ではない
ので、この時点で候補が折られる。`kp_shift_conv_guard_key_up`（`:1277`）の
`is_left_shift_tap` は `take(left_shift_tap_candidate)` が false になるため
成立せず、トグルは立たない。**Shift+親指+文字のチョードは、押下順に
関わらず（Shift が先でも親指が先でも）持続トグルを誤発火しない。**

ただしこの保証は次の呼び出し順に依存している。**これを壊してはならない。**

> `kp_stage_post_decision`（呼び出しは `key_pipeline.rs:215`、定義は `:921`、
> 末尾 `:1082` で `kp_stage_shift_conv_guard` を呼ぶ）は、`kp_stage_execute`
> （呼び出し `:228`、定義 `:1649`）**より必ず先に**実行される。物理イベントが
> `INPUT_DEFER` へ退避されても、ガードの候補折り・pending 設定は defer の
> 有無に関係なく発火する。

これは BUG-58 の修正（案E、`38b5a4ee`）が明示的に依拠している既存の不変条件
（known-bugs.md `:7095-7103`：「**この呼び出し順を変更する場合は本バグが
再発しないか必ず確認すること。**」）と同一である。本 ADR はこの不変条件に
新しい依存先を1つ増やすだけで、条件自体を変更しない。

#### ガード自体を親指押下中に抑制すべきか → しない

「親指キー押下中は `kp_stage_shift_conv_guard` 全体を抑制する」案を検討したが
**却下する**。上記の通り候補折りが既に効いているため抑制は冗長であり、一方で
`shift_conv_guard_pending` を立てそこねると、`kp_shift_conv_guard_key_up` の
`take()` が false になって**2回目の左Shiftタップによるトグル解除も、右Shiftに
よる緊急解除も一切発火しなくなる**（`key_pipeline.rs:1225-1229` が 2026-07-11
の codex レビューで発覚したものとして記録している既知の罠）。触らないのが
正しい。

#### トグルの「exit」側は tap/chord を区別しない（既存挙動・退行ではないが頻度が上がる）

上の分析は**トグルが立つ（entry）側**についてのもの。**解除（exit）側は
非対称**である。`kp_shift_conv_guard_key_up`（`:1346-1361`）の

```rust
if self.platform_state.gate.half_width_alnum_toggle_active {
    // 2回目の左Shiftタップ（トグルOFF）・右Shift（トグルの緊急解除）
    …
    self.kp_restore_kana_from_half_width(true);
}
```

は、**`left_shift_tap_candidate` を一切見ない**。すなわち
`half_width_alnum_toggle_active` が立っている間は、**任意の Shift 解放**
（単独タップでもチョードの一部でも、左でも右でも）でトグルが解除される。

これは本 ADR による新規の退行ではない——**現在の `[ローマ字小指シフト]` 面の
チョード（Shift+1 で `！` 等）でもまったく同じことが起きる**。トグル ON 中に
小指シフト面を打てば、その Shift 解放でトグルが解けている。

しかし本 ADR は**Shift を併用する機会を増やす**（複合面を使うユーザーは
Shift+親指+文字を日常的に打つ）ため、**この挙動に遭遇する頻度は確実に
上がる**。「半角英数トグルを ON にしたまま複合面を1回打ったらトグルが
解けた」という報告が来たときに、それが仕様（既存挙動）なのか新規バグなのかを
即座に判別できるよう、ここに記録しておく。

**exit 側を tap 限定にする変更は本 ADR では行わない。** entry と exit の
対称性を崩す変更であり、右Shift による「緊急解除」（`:1347` のコメントが
明示する意図的な機能）を壊す。もし将来この頻度が実害になったら、
別 ADR として「exit も `left_shift_tap_candidate` を見る + 右Shift の緊急
解除は残す」を検討すること。

#### 全角記号の出力経路は既存の小指シフト面と同一

複合面から全角記号（`'！'` 等）を出力する経路は、現在の `[ローマ字小指シフト]`
面が通っている経路（`shift_face_reduce` → `lookup_face` → `KeyAction` →
`vk_pair_to_ascii` / `send_romaji_batched`）と同一であり、**物理 Shift が
押されたまま注入されるという条件も同じ**である。新しい露出は生まれない。
（`crates/awase-windows/src/ime.rs:133` 付近の modifier 中和はそのまま効く。）

### 決定6: 段階的実装計画

本リポジトリの `.claude/rules/main-develop-branch-flow.md` に従い、
**全フェーズを `develop` 経由で行う**。`main` への直接コミットは禁止。
feature ブランチ（例: `feat/thumb-pinky-shift-chord`）は `develop` の先端から
切り、`develop` へマージする。`main` の更新は
`release-develop-to-main` スキルによるリリース操作のときだけ。
複数セッションで並行作業する場合は `.claude/rules/worktree-per-session.md` に
従い worktree を分離する。

**この ADR ファイル自体も同ルールの対象である。** 同ルールは
「ドキュメントのみの変更（例: `docs/known-bugs.md` の追記、ADR のステータス
同期）であっても、このルールの対象とする。『コードじゃないから直接 `main` で
いい』という例外は設けない」と明記している。起票時、本ファイルは `main` の
作業ツリーに untracked のまま置かれていた。**`develop` 起点のブランチへ移して
からコミットすること。**

#### Phase 0: コアエンジンの純粋ロジック + `.yab` フォーマット（Linux 上で検証可能）

> **⚠️ Phase 0 だけでは実機の挙動は 1 mm も変わらない。** 決定0 の配線が
> 無い限り `active_thumb_face()` は実機で常に `None` を返し、
> `classify_idle_intent` の分岐順逆転も複合面の解決も到達しない。
> **`.yab` にセクションを書いても Windows 版・Linux 版では効かない**
> （macOS は `InputContext` 構築経路が未確認）。Phase 0 の成果物は
> 「エンジン内部の純粋ロジックとテスト」までであり、**機能としては
> Phase 0.5 の完了をもって初めて成立する。**

対象は `src/` 配下のみ。プラットフォーム非依存の純粋ロジックに限定する。

1. `src/engine/fsm_types.rs`: `ThumbSide` 新設、`Face` に2 variant 追加、
   `Face::resolve` / `Face::thumb_side()` 実装。`from_thumb`/`from_thumb_bool`
   を `resolve` への委譲に書き換え。
2. `src/yab/mod.rs`: `FaceKind` に2 variant、`classify_section` の昇格、
   `YabLayout` に2フィールド、`parse` / `serialize`（非空時のみ出力）/
   `resolve_kana` の更新。
3. `src/engine/nicola_fsm.rs`:
   - `get_face`（`:588`）に2アーム
   - `consume_thumb`（`:648`）/ `is_thumb_consumed`（`:1687`）を
     `thumb_side()` 経由に書き換え（`match` の 6 アーム化を回避）
   - `is_layout_key`（`:1713`）に2面を追加（**これを忘れると、複合面にしか
     定義が無いキーが `IdleIntent::PassThrough` へ落ちて機能しない**）
   - `resolve_thumb_face` 新設、`classify_idle_intent`（`:844`）の分岐順逆転
   - `step_pending_thumb_char`（`:1083`）/ `step_pending_char_thumb`（`:1041`）/
     `step_speculative_thumb`（`:1009`）/ `resolve_char_thumb_as_simultaneous`
     （`:624`）の呼び出し元 / `compute_prefer_char1`（`:1344`）を `resolve` 経由に
   - `thumb_shift_faces_enabled` フィールドと `set_thumb_shift_faces_enabled`
     setter（決定3）
4. `src/engine/confirm_policy.rs`: **`idle_ngram`（`:98`）は、Shift 押下中は
   `idle_wait` に倒す**（決定済み。起票時は「実装時に決める」としていたが
   確定させた）。

   理由: `idle_ngram` は Normal / LeftThumb / RightThumb の3面の候補仮名を
   `should_speculate` に渡し、「通常面が明らかに有利なら投機出力する」と
   判断する。ここに複合面を足そうとすると、**複合面の値は記号が中心で
   n-gram モデル（`NgramModel`、仮名の連接確率）の学習対象外**であるため
   `lookup_kana_at` が `None` を返しやすく、スコア比較の入力が欠けた状態で
   「通常面が有利」と誤判定して投機出力してしまう。投機が外れると
   `retract_and_replace`（`nicola_fsm.rs:981`）が BACKSPACE を送るので、
   記号入力のたびに画面がちらつく。Shift 押下中は投機せず素直に待つほうが
   得られるものが多い。

   `ConfirmMode::Speculative` / `TwoPhase` も同様に、Shift 押下中は
   `idle_wait` へ倒す（`idle_speculative`（`:50`）/ `idle_two_phase`（`:77`）
   の先頭でガードする）。**帰結: これらの確定モードを使っているユーザーは、
   Shift チョードだけレイテンシ特性が `Wait` 相当になる。** 記号入力の頻度は
   低いので許容する。
5. テスト（後述のテスト計画）。

**検証**: `cargo test`（Linux でそのまま全緑になること）、`cargo clippy
--all-targets -- -D warnings`、`cargo fmt --check`。実機は不要。
**ただし前述の通り、Phase 0 完了時点で製品の挙動は変わらない。**

#### Phase 0.5（必須）: `InputContext` への親指押下状態の配線

**決定0 の実装。これが無いと本機能は実機で成立しない。** Phase 0 と Phase 1
の間に必ず挟む。順序を入れ替えたり省略したりしないこと。

1. `crates/awase-windows/src/runtime/mod.rs:63`: `build_input_context` に
   `left_thumb_down` / `right_thumb_down` の2引数を追加し、`None` リテラルを
   置き換える。呼び出し元2箇所（`runtime/key_pipeline.rs:100`、
   `runtime/mod.rs:239` の `build_ctx()`）を更新する。
2. 親指押下タイムスタンプの追跡実装（決定0 参照）。Alt なりすまし
   （`hook.rs:73` `apply_alt_impersonation`）**適用後**の VK を見ること。
3. `crates/awase-linux/src/main.rs:113-126`: 同じ配線。
4. macOS: `InputContext` の構築経路を調査してから配線（未確認）。

**検証**:

- `crates/awase-windows/tests/` 側に「`build_input_context` が親指押下状態を
  落とさない」ことを固定するガードテストを置く（テスト計画参照）。
  **エンジン内テストでは原理的に検出できない**——`TestHarness` は
  `InputTracker` を使うため、この配線が壊れていても常に緑になる。これが
  起票時に欠落を見逃した直接の原因である。
- **実機確認が必須**（決定0 の「副次的な効果」参照）: 親指キーを押しっぱなしに
  して複数の文字キーを連続で打ったときの挙動。`is_thumb_consumed` /
  `consume_thumb` が初めて実機で機能するため、複合面とは無関係に挙動が
  変わりうる。想定外の回帰が出たら、複合面ではなくこの配線を疑うこと。

#### Phase 1: Windows / GUI 配線（Linux 上でクロスコンパイル確認まで）

1. `crates/awase-settings/src/main.rs`: 独自の `Face` enum（`:34`）に2値、
   `FACES` テーブル（`:41`）を6要素に、`layout_face` / `layout_face_mut`
   （`:372`,`:381`）に2アーム。配列エディタのタブが6面になる。
   **awase-settings の `Face` は `engine::fsm_types::Face` とは別型**なので
   独立に更新が要る（型としては疎結合のまま維持してよい）。
2. `crates/awase-windows/src/app/bootstrap.rs`: 親指キー VK の分類結果から
   `set_thumb_shift_faces_enabled` を呼ぶ配線（決定3）。
   **この配線自体に最低限のガードを用意する**——テスト計画のケース12は
   `set_thumb_shift_faces_enabled(false)` を直接呼ぶだけで、bootstrap が
   実際にそのフラグを渡していることを何も検証していない。決定0 と同型の
   「エンジン内テストは緑だが配線が無い」失敗を繰り返さないよう、
   bootstrap が親指キー VK から `ModifierKey::Shift` を判定してフラグを
   渡す部分を純粋関数（例: `thumb_shift_faces_enabled_for(left_vk, right_vk)`）
   に切り出し、それを直接テストする。
3. `crates/awase-linux` / `crates/awase-macos`: 同じ setter の配線。両者とも
   `hook::set_thumb_keycodes` で親指キーを設定している箇所（`main.rs:42` /
   `:43`）の近傍。
4. `key_pipeline.rs::kp_stage_shift_conv_guard` の doc コメントに、本 ADR が
   その候補折りロジックに依存していることを追記（決定5の順序不変条件を
   壊されないようにする）。

**検証**: Linux 上で `cargo xwin clippy -p awase-windows` によるクロス
コンパイル確認まで（`project_main_ci_broken_windows_build_2026_07_19` の
知見）。GUI の見た目確認は Phase 2 と合わせる。**注意: ローカル rustfmt が
古いと CI が落ちる。`rustup update stable` を先に走らせること。**

#### Phase 2: Windows 実機検証（自動化不可）

`.yab` に複合面を実際に定義し、以下を確認する。

| 確認項目 | 期待 |
| --- | --- |
| Shift↓ → 無変換↓ → k↓（順序A） | 複合面の値が出る |
| 無変換↓ → Shift↓ → k↓（順序B） | 同じ値が出る（非対称性の解消確認） |
| 複合面未定義位置で同じ操作 | 従来の親指面の値が出る（フォールバック） |
| 複合面が**部分定義**の `.yab` で未定義キーを打つ | 親指面の値が出る（Shift 面の値が出たら決定2のフォールバックが壊れている） |
| 右Shift + 親指 + 文字 | 左Shift と同じ複合面が出る（決定3） |
| 両親指同時 + Shift + 文字 | 左の複合面が出る（決定3） |
| チョードを KeyUp で終える / タイムアウトで終える | **同じ面が出る**（決定2 の出口 #7/#8） |
| チョード後の Shift↑ | 半角英数トグルが**立たない**（決定5、ログで `[shift-conv-guard]` が出ないこと） |
| **トグル ON 中に複合面チョードを打つ** | **トグルが解除される（既存挙動・退行ではない、決定5 の exit 側分析参照）** |
| 同じチョードを連打 | BUG-58 型の数秒フリーズが起きない |
| 複合面の全角記号（`'！'` 等） | 半角化しない（BUG-49 型の退行が無いこと） |
| 左Shift単独タップ | 従来通り半角英数トグルが立つ（既存機能の非破壊） |
| **親指ホールドで文字を連打**（複合面と無関係） | Phase 0.5 の配線で挙動が変わりうる。決定0 参照 |

対象は MS-IME / Google 日本語入力 × Chrome / Windows Terminal / LINE の
組み合わせ。特に **LINE（Qt/ImmCross）** は BUG-49 で全角記号の半角化が
実害として出たアプリなので必ず含める。

### 決定7: 今後この機能を revert する場合の規約

本機能は `.claude/rules/experiment-logging.md` の適用範囲
（`ime_controller.rs` / `output/vk_send.rs` = キー選択ファミリー、
`src/engine/nicola_fsm.rs` は列挙されていないが、Windows 側の Shift ガードと
相互作用する以上、実質的に同ファミリーである）に隣接する。

**本機能の全部または一部を revert するコミットは、本文に
アプリ × IME × 再現手順の3点を必ず書くこと。** 「ADR-097 を revert」だけの
本文は禁止する。特に「決定2（Shift をレベル信号として扱う）」と
「決定3（Space/Enter 特例の据え置き）」は、後日別セッションが独立に
「直交フラグにすべき」「Space も複合面に入れるべき」と再提案しやすい形を
しているため、撤回する場合は**なぜ撤回したのか**を証拠付きで残すこと
（`docs/experiments.md` にも1行追記する）。

### 決定8: デフォルト配列は追加しない

`layout/nicola.yab`・`layout/nicola_us.yab`・`layout/nicola_f.yab` に
複合面のセクションを**追加しない**。新2面は「ユーザーが `.yab` で任意に
定義できる空の面」として実装する。

理由:

- やまぶきR の実際のデフォルト配列（左親指小指シフト面／右親指小指シフト面に
  何が割り当てられているか）は Web 調査だけでは断定できなかった。
  推測で埋めた値は「やまぶきR互換」を名乗れない。
- 本リポジトリには**未定義の独自拡張を配列に無断追加して後の監査で発覚した
  前例**がある（NICOLA 規格との全キー監査、2026-07-28）。同じ轍は踏まない。
- **消費ロジックの無い予備プロビジョニングを避ける**という既存の教訓
  （2026-08-15、F22-F24 予備バインドを即日 revert）とも整合する。ただし本件は
  逆向き——ロジックだけ先に入れてデータを入れない——であり、こちらは安全側
  （空面はフォールバックで従来挙動に落ちる）。

やまぶきR の実配列が特定でき、かつユーザーが「デフォルトに入れたい」と
明示的に判断した時点で、別コミットとして追加すればよい。

## テスト計画

`.claude/rules/fix-requires-evidence.md` の要求（(a) 回帰テスト または
(b) `docs/known-bugs.md` 追記）は、**(a) を主として満たす**。すべて Linux 上の
`cargo test` で実行できる。

> **⚠️ エンジン内テストだけでは本 ADR の正しさを保証できない。**
> `src/engine/tests.rs` の `TestHarness` は `InputTracker` を使うため、
> 決定0 の配線が無くても以下のケースは**全部緑になる**。実機で機能する
> ことの保証は「`crates/awase-windows/tests/` 側のガード」（下記 A 群）と
> Phase 0.5 / Phase 2 の実機確認だけが担う。この非対称性を忘れないこと。

### A 群: 配線を守るガード（`crates/awase-windows/tests/`、必須）

エンジン内テストでは原理的に検出できない領域。決定0 の欠落を二度と
起こさないための最重要テスト。

- **A-1**: `build_input_context`（`runtime/mod.rs:63`）が渡された親指押下
  状態をそのまま `InputContext` に載せることを固定する。`Some(ts)` を渡して
  `Some(ts)` が返ることを確認するだけの単純なテストでよい——**重要なのは、
  誰かが再び `None` リテラルに戻したら落ちること**。
- **A-2**: `build_input_context` の呼び出し元が親指状態を落としていないこと。
  関数がハードコードに戻される以外に、呼び出し元が常に `None` を渡す形でも
  同じ欠陥が再現するため、`architecture_guard.rs` 相当のテキスト検査で
  「`build_input_context(` の呼び出しに `None, None` が直接書かれていない」
  ことを固定するのが現実的（`tests/architecture_guard.rs` の既存様式に倣う）。
- **A-3**: 決定3 の `thumb_shift_faces_enabled_for(left_vk, right_vk)`
  純粋関数（Phase 1 項目2で切り出す）の真理値表を固定する。

### B 群: エンジン内テスト（`src/engine/tests.rs` ほか）

### `crates/awase-windows/tests/ime_key_sequence_golden.rs` について（重要な訂正）

**このゴールデンは本 ADR と無関係であり、変更してはならない。**

このファイル（224行、`#![cfg(windows)]`）が固定しているのは
`ImeOpenStrategy` の選択（ImmCross → GjiDirect → MsImeDirect → KanjiToggle）と
IME を ON/OFF するために送る VK であって、**`.yab` の面選択ではない**。
`characterize_strategy(active_gji, profile, skip_imm)` の入力に配列の面は
一切登場しない。本 ADR の変更後も
`crates/awase-windows/tests/golden/ime_key_sequences.txt` は**1バイトも
変わらない**。

「キー選択ファミリーだからこの golden に追記する」という誘導に従って
無理に差分を作らないこと。本 ADR に対応する機械可読な回帰防止は
`src/engine/tests.rs`（面選択の SSOT）である。

### `src/engine/tests.rs` に追加するケース

既存の `make_engine_with_shift()`（`:1368`）と
`make_engine_with_space_thumb()`（`:888`）に倣って
`make_engine_with_thumb_shift_faces()` を用意し、新モジュール
「親指小指シフト面」を追加する。

**順序の対称性（本 ADR の中核）**

1. `Shift↓ → 左親指↓ → k↓` → 複合面の値
2. `左親指↓ → Shift↓ → k↓` → **同じ値**（現状は小指シフト面の値になる
   ——このテストが現状で落ちることを確認してから実装すること）
3. `Shift↓ → 右親指↓ → k↓` → 右複合面の値

3b. 右Shift（`VK_RSHIFT`）で 1. と同じ操作 → 同じ複合面の値（決定3 の左右
    対称性。左Shift だけでテストすると `shift-conv-guard` の左右非対称と
    混同したまま気付けない）
3c. 両親指を同時に押下 + Shift + 文字 → **左**の複合面（決定3）

**フォールバック**

4. 複合面のその位置が未定義 → 親指面の値（順序A・B 両方で）
4b. **複合面が部分定義**（同じ面の別の位置には定義がある）で、未定義の位置を
    打つ → 親指面の値。**順序A・順序B の両方で明示する。**
    決定2 の「素の `Face::resolve` に置き換えると壊れる」具体例そのもの
    （素の resolve だと Shift 面の値が出て落ちる）。ケース4 が「面が丸ごと
    空」を見るのに対し、こちらは「面はあるがそのキーが無い」を見る——
    実装ミスの出方が違うので両方要る。
4c. 複合面が部分定義で未定義の位置に対し、同時打鍵の**閾値**が Shift 非保持時と
    変わらないこと（決定2 の `candidate_kana` の出所。閾値が変わると
    「新しい定数を足していないのに判定時間が変わる」回帰になる）
5. 複合面に明示的な `無` → `KeyAction::Suppress`（親指面へフォールバック
   **しない**）
6. 複合面・親指面とも未定義 → 従来通り Shift 面 → それも無ければパススルー
6b. 複合面にしか定義が無いキーが `is_layout_key` で `true` になる
    （決定1 の「コンパイラ強制が効かない箇所 1」の直接テスト。これが無いと
    複合面が丸ごと `PassThrough` に落ちる実装ミスを検出できない）

**Shift レベルの動的な変化**

7. `Shift↓ → 左親指↓ → Shift↑ → k↓` → 親指面（複合面ではない）
8. `左親指↓ → k↓`（Shift 無し）→ 親指面（既存挙動の非破壊）

**既存特例の非破壊（回帰ガード）**

9. Space 親指 + `shift_literal=true` + Shift 押下 → 依然としてリテラル
   スペースで PassThrough（`test_shift_space_literal_passthrough_when_enabled`
   の複合面版）
10. Enter 親指 + `shift_literal=true` → 同上
11. Space 親指 + `shift_literal=false` + Shift 押下 → 複合面に到達する
12. 親指キーが Shift 修飾キーにバインドされている（`tests.rs:867` の設定）
    → `thumb_shift_faces_enabled=false` により複合面に**行かない**
    （**このケースは `set_thumb_shift_faces_enabled(false)` を直接呼ぶだけで、
    bootstrap が実際にそのフラグを渡すことは検証しない。** そちらは A-3 が
    担当する。両方無いと「エンジンは正しいが配線が無い」状態を見逃す）
13. Shift 面のみ定義・親指未押下 → 従来通り Shift 面
    （`test_shift_held_uses_shift_face`（`:1386`）が無変更で通ること）

**`PendingCharThumb` の4つの出口が一致すること（決定2）**

13b. `Shift↓ → k↓ → 左親指↓` の後 **char1 の KeyUp** で確定 → 複合面
     （出口 #7、`handle_key_up_pending_char_thumb`）
13c. `Shift↓ → k↓ → 左親指↓` の後 **thumb の KeyUp** で確定 → 複合面
     （出口 #7 の別分岐）
13d. `Shift↓ → k↓ → 左親指↓` の後 **タイムアウト**で確定 → 複合面
     （出口 #8、`timeout_pending_char_thumb`）
13e. 13b〜13d と ケース1（char2 到着 = 出口 #6）が**すべて同じ面**を出すこと
     を1つのテストで並べて固定する。**「終わり方によって面が変わらない」
     ことが本 ADR の中核なので、個別に緑になるだけでなく一致を明示的に
     assert すること。**

**3鍵仲裁との相互作用**

14. `Shift↓ → k↓ → 左親指↓ → j↓`（3鍵逐次仲裁）。
    **「いずれかに正しく解決する」では常に真になりアサーションにならない。**
    次の形で書くこと: 複合面と親指面で n-gram スコアが**逆転する**具体的な
    かな組み合わせを `.yab` に仕込み、
    (a) Shift 非保持なら `PairWithChar1`、
    (b) Shift 保持なら `PairWithChar2`（またはその逆）
    となる境界を選ぶ。こうすると「`compute_prefer_char1` が複合面から候補
    仮名を引いている」ことが**出力の違いとして観測できる**。
    素の親指面から引いたままでも通ってしまうテストにしない。
    n-gram モデルを差し込む方法は `NicolaFsm::set_ngram_model`（`:560`）と
    既存の n-gram テスト（`src/engine/tests.rs` 内）を参照。
15. `k↓ → 左親指↓ → Shift↓ → j↓` → Shift↓ の flush により
    「k+親指（**親指面**、Shift を見ない）」+「j は Shift 面」に分解される。
    **これは決定2 の表 #9（`flush_pending` は `thumb.face()` のまま据え置き）
    と一致していなければならない。** 起票時の草案では表が
    `resolve_char_thumb_as_simultaneous` を一律 `resolve` 経由にすると
    書いており、このケースと矛盾していた（表を #9 の据え置きへ修正済み）。
    実装時に表とこのケースの期待値が一致していることを再確認すること。

**境界**

16. `KeyClass::Passthrough` のキー（例: F5）が複合面に到達しない
    （`bypass_reason` が先に弾く）

### `src/yab/tests.rs` に追加するケース

- `[ローマ字小指左親指シフト]` / `[ローマ字小指右親指シフト]` が
  `YabLayout.left_thumb_shift` / `.right_thumb_shift` に**実際にパースされる**
  （既存の `test_parse_yamabuki_compat_sections_are_accepted_but_ignored`
  （`:637`）は「無視される」ことを前提にしているので、**この2面の分を
  切り出して別テストに移す**。英数系6面と拡張親指シフトの `Ignored` 検証は
  残す）
- 新2面が空のレイアウトを `serialize` すると、既存4面のみが出力される
  （バイト等価のラウンドトリップ）
- 新2面が非空なら `serialize` に含まれ、再 `parse` で復元できる
- `layout/nicola.yab`・`nicola_us.yab`・`nicola_f.yab` が変更なしでパースでき、
  新2面が `is_empty()` であること（実ファイルを読むテストが既にあれば
  そこにアサートを足す）

### `src/engine/proptest_tests.rs` に追加する観点

- `arb_vk()`（`:218`）の候補に `VK_LSHIFT` を追加し、
  `never_panics_on_arbitrary_events` / `state_always_valid` /
  `keydown_keyup_balance` が Shift を含むランダム列でも通ることを固定する。
  **Shift の KeyDown が `handle_bypass` → `flush_pending` を通る経路が
  ランダム列に混ざるため、面解決の分岐順逆転で不変条件が壊れていないかの
  検出力が上がる。**
- `make_layout()`（`:68`）に複合面の定義を数キー追加する。
- `classify_modifier`（`:190`）が `VK_LSHIFT` に `ModifierKey::Shift` を
  返すようにする（`tests.rs:232` に既にある分類と揃える）。

### `docs/known-bugs.md`

新規バグではないので追記は必須ではないが、**BUG-49 と BUG-58 の項に
「2026-08-19: ADR-097 でこの領域に隣接する変更を入れたが、チョードへの
conv 先書き込みが撤去済み（BUG-15 追補9）のため機序は再現しない。
`kp_stage_post_decision` → `kp_stage_execute` の順序不変条件に
新しい依存先が増えた」旨を1〜2行追記する**ことを推奨する。次に
この領域を触る担当者が ADR-097 に辿り着けるようにするため。

## 影響（Consequences）

### 正の影響

- Shift+親指+文字の押下順による面の食い違い（コンテキスト節）が解消する。
  これは本 ADR が無くても直すべき既存の不整合だった。
- 面解決が `resolve_thumb_face` の1関数に集約され、「どの面が出るか」を
  1箇所で読めるようになる。現在は `classify_idle_intent` の分岐順・
  `step_pending_thumb_char` の無関心・`should_use_shift_plane` の
  `!is_thumb()` ガード・`PendingCharThumb` の4出口という複数箇所に暗黙に
  散っている。
- **決定0 の配線により、`is_thumb_consumed`/`consume_thumb`（ADR-008/010 が
  設計した親指の二重シフト防止）が初めて実機で機能するようになる。**
  本 ADR とは独立の、積み残しの解消である。
- やまぶきR の `.yab` を（この2面については）そのまま読めるようになる。

### 負の影響・受け入れるトレードオフ

- **順序Bの挙動が変わる。** 「親指↓ → Shift↓ → 文字↓」は現在 Shift 面の値を
  出すが、変更後は親指面（複合面が未定義なら）の値を出す。既存 `.yab` 利用者に
  とっては挙動変更である。意図的な是正だが、リリースノートに明記すること。
- **Space/Enter 親指では複合面が既定で使えない**（決定3）。設定で
  `*_shift_literal = false` にすれば使えるが、その場合 Shift+Space による
  明示的な半角スペース入力を失う。
- **親指キーに Shift を割り当てているユーザーは複合面を使えない**（決定3）。
  これは正しい制限だが、設定画面で理由が分かるようにするのが望ましい
  （Phase 1 で hover text を足すかは実装時判断）。
- `Face` の variant 追加により `match` を持つ3箇所が必ず更新対象になる。
  これはコストではなく**意図した安全装置**（決定1）。ただし
  `is_layout_key` / `idle_ngram` の2箇所には効かない（決定1 の警告）。
- **決定0 の配線は、複合面とは無関係に「親指ホールド中の連打」の挙動を
  変えうる。** これまで死んでいた消費機構が生き返るため。回帰が出た場合に
  「複合面のせいだ」と誤診しないこと（Phase 0.5 の実機確認項目）。
- **`ConfirmMode::Speculative`/`TwoPhase`/`NgramPredictive` を使っている
  ユーザーは、Shift 押下中だけレイテンシ特性が `Wait` 相当になる**
  （決定6 Phase 0 項目4）。記号入力の頻度は低いので許容する。
- 配列エディタのタブが4→6に増え、横幅を消費する。

### 中立

- Windows 側のロジック変更は `build_input_context` への親指状態の配線
  （決定0）と `set_thumb_shift_faces_enabled`（決定3）だけで、
  IME 制御・warmup・conv mode には一切触れない。したがって
  `.claude/rules/tuning-constants.md` の実測義務は発生しない（決定4）。

## 未解決の論点・要ユーザー判断事項

1. **やまぶきR のデフォルト配列が不明。** 左親指小指シフト面／右親指小指
   シフト面に標準で何が割り当てられているかは Web 調査で断定できなかった。
   決定8では「空のまま実装する」としたが、この判断で良いか。
   （ユーザーがやまぶきRの実 `.yab` を持っているなら、それを読めば確定する。）
2. **順序Bの挙動変更を受け入れるか。** 現在「親指↓ → Shift↓ → 文字↓」で
   小指シフト面を出す操作を、無意識に使っているケースがあるか。
   もし「この順序では Shift 面が出てほしい」なら、決定2の分岐順逆転を
   やめて「Shift 面優先のまま、複合面は Shift↓ が先の場合だけ有効」という
   非対称な仕様にもできる（ただし説明しづらくなる）。
3. **決定0 の配線が既存挙動に与える影響。** 親指ホールド連打の挙動が
   変わりうる（`is_thumb_consumed` が実機で初めて機能する）。これは
   ADR-008/010 が意図した正しい挙動への是正だが、現在の（消費機構が
   死んだ）挙動に慣れているユーザーには回帰に見える可能性がある。
   Phase 0.5 の実機確認で違和感が出た場合、複合面を諦めるのではなく
   「配線はするが `active_thumb_face()` の利用は複合面判定に限る」という
   中間案もありうる。**実機で確かめてから判断する。**
4. **`idle_ngram` / 投機モードの扱い**（Phase 0 の項目4）。決定6 で
   「Shift 押下中は `idle_wait` に倒す」と確定させたが、
   `ConfirmMode::NgramPredictive` / `Speculative` / `TwoPhase` を使っている
   ユーザーにとっては Shift チョードだけレイテンシ特性が変わる。
   実機で体感差が問題になるようなら再検討する。
5. **両親指同時打鍵**（`[拡張親指シフト1]`/`[拡張親指シフト2]`）を
   将来やるか。note.com 記事の実例では両親指同時+文字キーで `_`/`*`/`(`
   等を出す拡張がある。本 ADR ではスコープ外としたが、`Face::resolve` の
   シグネチャを `(Option<ThumbSide>, bool)` にしておくと「両親指」を
   表現できない。将来やるなら `ThumbState { None, Left, Right, Both }` へ
   広げる必要がある。**今この可能性を織り込んで設計を膨らませるべきか、
   必要になってから広げるべきか。**（本 ADR の推奨は後者。`Face::resolve`
   は enum を返す純粋関数なので、後から入力型を広げるコストは小さい。）
6. **英数系6面**（`[英数シフト無し]` 等）。やまぶきR は IME OFF 時の面を
   別に持つ。awase は現在これを `Ignored` にしており、本 ADR も変更しない。
   ユーザーが必要としているか。
7. **設定画面での説明。** 決定3の2つの制限（Space/Enter 親指、Shift 親指）
   はユーザーから見ると「なぜか複合面が効かない」に見える。設定画面に
   注記を出すか、起動時ログに警告を出すか。

## 参照

**決定0（配線）関連 — 実装前に必ず読むこと**

- `crates/awase-windows/src/runtime/mod.rs:63-79` — `build_input_context`
  （`left_thumb_down`/`right_thumb_down` を `None` 固定にしている当の箇所）
- `crates/awase-windows/src/runtime/key_pipeline.rs:100` /
  `crates/awase-windows/src/runtime/mod.rs:239` — 呼び出し元2箇所
- `crates/awase-linux/src/main.rs:113-126` — Linux 側の `InputContext` 直接構築
- `src/engine/input_tracker.rs:56-64` — `PhysicalKeyState::from_ctx`
  （`ctx` の `None` をそのまま伝播する）
- `src/engine/input_tracker.rs:84-188` — `InputTracker`（テスト専用）
- `crates/awase-windows/src/hook.rs:141` — `CACHED_THUMB_VKS`（追跡の材料）
- [ADR-008](008-physical-thumb-state-separation.md) /
  [ADR-010](010-thumb-consumption-timestamp.md) — この配線が本来満たすはずの設計

**面選択関連**

- `src/engine/fsm_types.rs:81-107` — `Face` enum
- `src/engine/nicola_fsm.rs:763` — `should_use_shift_plane`
- `src/engine/nicola_fsm.rs:825-841` — Space/Enter リテラル特例
- `src/engine/nicola_fsm.rs:844-901` — `classify_idle_intent` / `decide_idle`
- `src/engine/nicola_fsm.rs:353-367` — `flush_pending` の `PendingCharThumb`
  アーム（決定2 の出口 #9、意図的に据え置く箇所）
- `src/engine/nicola_fsm.rs:1340-1422` — 3鍵逐次仲裁
- `src/engine/nicola_fsm.rs:1476-1511` — `handle_key_up_pending_char_thumb`（出口 #7）
- `src/engine/nicola_fsm.rs:1623-1641` — `timeout_pending_char_thumb`（出口 #8）
- `src/engine/nicola_fsm.rs:1687-1723` — `active_thumb_face` / `is_layout_key`
- `src/engine/confirm_policy.rs:98-126` — `idle_ngram`
- `src/yab/mod.rs:482-513` — `classify_section` / `FaceKind`

**Windows Shift ガード関連**

- `crates/awase-windows/src/runtime/key_pipeline.rs:1182-1366` — Shift ガード
  （`kp_stage_shift_conv_guard` `:1182` / `_key_down` `:1207` /
  `_key_up` `:1266`、`is_left_shift_tap` は `:1277`）
- `crates/awase-windows/src/runtime/key_pipeline.rs:215`/`:228` —
  `kp_stage_post_decision` → `kp_stage_execute` の順序不変条件（決定5）
- `docs/known-bugs.md:5308` — BUG-49（小指シフト面の全角記号が半角化）
- `docs/known-bugs.md:7005` — BUG-58（チョードで ~5 秒フリーズ、案E で修正）

**その他**

- [ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)
  — 親指キー単独タップの `ModeKeyConfig`/`TextKeyConfig` 再編
- `.claude/rules/main-develop-branch-flow.md` / `fix-requires-evidence.md` /
  `experiment-logging.md` / `tuning-constants.md`
