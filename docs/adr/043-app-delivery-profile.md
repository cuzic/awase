# ADR-043: AppDeliveryProfile — アプリ固有出力動作の宣言的集約

**状態:** 提案（未実装）  
**調査日:** 2026-06-04  
**関連:** ADR-033 (app-ime-profile), ADR-004 (injection-mode-design)

---

## 背景

現在、アプリ固有の出力動作に関する知識が複数ファイルに条件として散在している。

| ワークアラウンド | 場所 | 埋め込まれた知識 |
|---|---|---|
| 6-F（Chrome F2 重複スキップ）| `output/vk_send.rs:140-144` | `cold_reason == ColdReason::F2NonTsf` |
| 6-F（Chrome probe タイミング）| `output/vk_send.rs:164-179` + `tuning.rs` | `CHROME_PROBE_*` 定数6個 |
| 6-G（WezTerm Unicode+VK 混在防止）| `output/probe_io.rs:165` | `deferred_vks.is_empty()` 条件 |

新しいアプリへの対応時、どのファイルを変更すべきか自明でなく、変更漏れのリスクがある。

---

## 決定

**このADRは未実装。** 設計を記録しておき、新しいアプリ対応の機会に実装する。

### 新設型: `AppDeliveryProfile`（`output/delivery_profile.rs`）

```rust
/// フォーカス中アプリの出力動作特性。フォーカス変更時に確定し、以降不変。
///
/// 出力アルゴリズムはアプリ名・クラス名を知らずこのフィールドのみを参照する。
/// 新規アプリへの対応は `from_focus()` への追記だけで完結する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct AppDeliveryProfile {
    /// キー注入方式
    pub mode: InjectionMode,

    /// 物理 F2 が composition context を初期化してから有効な期間 (ms)。
    /// この期間内なら programmatic F2 をスキップする。0 = スキップしない。
    ///
    /// Chrome/Edge: 物理 F2 で TSF context を自己初期化するため、awase が
    /// 二重に F2 を送ると composition がリセットされる（workaround 6-F）。
    pub physical_f2_valid_ms: u64,

    /// KEYEVENTF_UNICODE 直後に VK ストロークを続けて安全か。
    ///
    /// WezTerm (TSF mode): Unicode 後の VK を literal 'n' として扱う（workaround 6-G）。
    /// false の場合、deferred VK がある局面では Unicode kana パスを使わない。
    pub unicode_vk_interleave_safe: bool,

    /// Vk モード (Chrome/Edge) の probe 待機時間。
    /// 通常 / 長期idle / 物理F2+GJI長期idle の3状況を宣言的に保持する。
    pub vk_probe: VkProbeParams,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VkProbeParams {
    pub normal:      (u64, u64),   // (min_ms, max_ms)
    pub long_idle:   (u64, u64),
    pub f2_gji_idle: (u64, u64),   // 物理F2 + GJI長期idle の特殊ケース
}
```

### 構築（`InjectionMode` から決定）

```rust
impl AppDeliveryProfile {
    pub fn from_focus(mode: InjectionMode, _ime_profile: AppImeProfile) -> Self {
        match mode {
            InjectionMode::Tsf => Self {
                mode,
                physical_f2_valid_ms: 0,
                unicode_vk_interleave_safe: false,   // WezTerm: 6-G
                vk_probe: VkProbeParams::default(),
            },
            InjectionMode::Vk => Self {
                mode,
                physical_f2_valid_ms: F2_STALE_MS,  // Chrome: 6-F
                unicode_vk_interleave_safe: true,
                vk_probe: VkProbeParams {
                    normal:      (CHROME_PROBE_MIN_MS,             CHROME_PROBE_MAX_MS),
                    long_idle:   (CHROME_PROBE_LONG_IDLE_MIN_MS,   CHROME_PROBE_LONG_IDLE_MAX_MS),
                    f2_gji_idle: (CHROME_PROBE_F2_GJI_IDLE_MIN_MS, CHROME_PROBE_LONG_IDLE_MAX_MS),
                },
            },
            InjectionMode::Unicode => Self {
                mode,
                physical_f2_valid_ms: 0,
                unicode_vk_interleave_safe: true,
                vk_probe: VkProbeParams::default(),
            },
        }
    }

    pub fn physical_f2_still_valid(&self, elapsed_ms: u64) -> bool {
        self.physical_f2_valid_ms > 0 && elapsed_ms < self.physical_f2_valid_ms
    }

    pub fn resolve_vk_probe(&self, long_idle: bool, f2_gji_idle: bool) -> (u64, u64) {
        if long_idle        { self.vk_probe.long_idle   }
        else if f2_gji_idle { self.vk_probe.f2_gji_idle }
        else                { self.vk_probe.normal      }
    }
}
```

### 呼び出し側の変化

**`output/vk_send.rs`（6-F）:**
```rust
// Before
let skip_f2_send = cold_reason == ColdReason::F2NonTsf && !f2_stale;
let (probe_min_ms, probe_max_ms) = if long_idle { ... } else if f2_gji_long_idle { ... } else { ... };

// After
let skip_f2_send = cold_reason == ColdReason::F2NonTsf
    && self.delivery_profile.physical_f2_still_valid(elapsed);
let (probe_min_ms, probe_max_ms) =
    self.delivery_profile.resolve_vk_probe(long_idle, f2_gji_long_idle);
```

**`output/probe_io.rs`（6-G）:**
```rust
// Before
used_eager_path: used_eager_path && deferred_vks.is_empty(),

// After
used_eager_path: used_eager_path
    && (self.delivery_profile.unicode_vk_interleave_safe || deferred_vks.is_empty()),
```

**`Output` 構造体:**
```rust
// Before: injection_mode: InjectionMode,
// After:  delivery_profile: AppDeliveryProfile,
// injection_mode() アクセサは delivery_profile.mode に委譲
```

---

## 評価

### メリット

- **新規アプリ対応の変更箇所が一箇所になる**: `from_focus()` への追記のみ
- **「なぜこう動くか」の所在が明確**: アルゴリズムのコメントでなく型のフィールドに記述
- **`CHROME_` 接頭辞定数の意味が正確になる**: Brave/Edge にも適用されるのに Chrome 名のままだった定数が消える

### デメリット・限界

- **バグは一つも減らない**: 6-F と 6-G が再配置されるだけで動作は同一
- **17 個のワークアラウンドのうち 2 個しか対象外**: 他の 15 個（タイミング系・スレッド安全系・IME 状態管理系）は無関係
- **`ColdReason::F2NonTsf` は残る**: イベントのトリガー（物理F2消費）とアプリ能力（`physical_f2_valid_ms`）は直交するため両方存在し続ける
- **`InjectionMode` への結合が粗い**: 将来 Vk モードだが F2 挙動が異なるアプリが出ると `from_focus` の引数が増える

### 実装タイミング

新しいアプリ（Electron 系など）の対応で `vk_send.rs` や `probe_io.rs` を触る機会に「ついでに」行うのが適切。このリファクタリングのためだけに工数を割く優先度は低い。

---

## 調査過程で否定された代替案

### 案1: WM_IME_STARTCOMPOSITION フック

`WH_CALLWNDPROC` で `WM_IME_STARTCOMPOSITION` を傍受し composition の warm/cold 状態を直接観測する。

**否定理由:**
- DLL インジェクションが必須（AV 誤検知リスク）
- `SetWinEventHook(WINEVENT_OUTOFCONTEXT)` での `EVENT_OBJECT_IME_*` は GJI TSF モードでは発火しないことを実機確認済み（コミット `817c9bb`）

### 案2: ImmGetCompositionString クロスプロセスクエリ

フォアグラウンドウィンドウの HIMC から composition 文字列長を読み取り warm/cold を判定する。

**否定理由:**
- WezTerm は TSF native app のため `ImmGetCompositionStringW` が常に 0 を返す
- probe-and-retry として実装・実験済みで最悪 952ms 遅延が発生した（コミット `b643bac`）

### 案3: GJI named pipe 盗聴

GJI プロセス ↔ TSF DLL 間の独自 IPC を傍受して composition 状態を取得する。

**否定理由:**
- プロトコルは proprietary バイナリ形式でリバースエンジニアリングが必要
- GJI バージョンアップで即死するリスクが高い
- 現在の `GetProcessIoCounters` ベースの `TsfReadinessProbe` が実質的に同等の観測を DLL なしで実現済み
