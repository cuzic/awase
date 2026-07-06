# ジャーナル・リプレイ回帰基盤ガイド

## なぜこれが必要か

awase の再発バグ（fc18cc7 / 109b4c9 / 1544d3f / ea3da7f 等）の多くは、実機の特定アプリ・特定IME・特定タイミングでしか観測できない conv ビットの組合せが原因だった。手書きのユニットテストは開発者が「思いつく」組合せしかカバーできず、実機で偶然踏んだ組合せは記録されなければ再発防止に繋がらない。

`journal.rs`（統合イベントジャーナル）は既に存在し、ホットキー（Alt+変換 → Alt+無変換 を2回連続）で `%TEMP%/awase_journal_<tick_ms>.json` にダンプできる。この基盤は「実機で観測した瞬間にその入力を永久固定する」ためのものであり、journal.rs というダンプ機能と `tests/journal_replay.rs` という回帰テストを繋ぐのが本ガイドの対象範囲。

## 現在のスコープ（MVP）

フォーカス遷移・タイマー・キー入力すべてを統合リプレイするのは巨大すぎるため、現時点では `state/conv_classify.rs::classify_conv_transition`（idle-conv-check の conv ビット解釈と engine 同期判断）のみを対象にしている。この関数は純粋関数（I/O・時刻取得・`with_app` 呼び出しなし）であり、入力6値から出力を一意に決定するため、リプレイに最も適している。

将来的に他の純粋関数（`classify_idle`, `classify_fetched_snapshot` 等）にも同様の仕組みを広げられる。

## 仕組み

1. `journal.rs::JournalEntry::ConvClassifyCall` — `classify_conv_transition` の実引数と戻り値を構造化して記録する専用エントリ。`kp_stage_idle_conv_check`（`runtime/key_pipeline.rs`）が呼び出しのたびに記録する。
2. `state/conv_classify.rs::ConvClassifyFixture` — リプレイ専用の独立フォーマット（`Serialize`/`Deserialize` 両対応）。`JournalEntry` 全体は `KeyEventSummary` に `&'static str` を含み単純には `Deserialize` できないため、リプレイに必要なフィールドだけを持つ別構造体として定義している。
3. `tests/journals/*.json` — `ConvClassifyFixture` の配列を保存したフィクスチャファイル群。
4. `tests/journal_replay.rs` — `tests/journals/` 配下の全 JSON を読み込み、記録された入力で `classify_conv_transition` を再実行し、`expected` と一致するかを assert する。`conv_classify` モジュールは `#[cfg(windows)]` でゲートされていないため、**このテストは Linux ホストでもそのまま実行できる**（CI で常時実行可能）。

## バグに気づいたときの手順

1. **修正する前に**、ホットキーでジャーナルをダンプする（Alt+変換 → Alt+無変換 を2回連続）。ダンプ先は `%TEMP%/awase_journal_<tick_ms>.json`。
2. ダンプ JSON から、バグが起きた直前の `ConvClassifyCall` エントリを探す（`elapsed_ms` とログのタイムスタンプを突き合わせる）。
3. `crates/awase-windows/tests/journals/` に新しい JSON ファイル（または既存ファイルへの追記）として、以下の形式で転記する:

```json
[
  {
    "name": "短い識別名（英数字とアンダースコア推奨）",
    "note": "何が起きたか・関連コミット等の説明",
    "conv": 9,
    "current": "ObservedKana",
    "is_cold": false,
    "effective_open": true,
    "conv_mode_changed": true,
    "is_roman_reliable": false,
    "expected": {
      "input_mode_update": { "AssumedRomaji": { "reason": "ImmBridgeBroken" } },
      "engine": { "SetOpen": "RomajiRecovered" }
    }
  }
]
```

`InputModeState`/`EngineSync`/`ConvSyncReason` の JSON 表現は serde のデフォルト（externally tagged）。unit variant はそのまま文字列（例: `"ObservedKana"`、`"None"`）、フィールド付き variant はオブジェクト（例: `{"AssumedRomaji": {"reason": "ImmBridgeBroken"}}`、`{"SetOpen": "RomajiRecovered"}`）。`input_mode_update` が `None`（belief 変化なし）の場合は JSON の `null` を書く。迷ったら `crates/awase-windows/src/state/conv_classify.rs` の `#[cfg(test)] mod tests` にある定数（`CONV_JISKANA` 等）や、`tests/journals/example-jiskana-recovery.json` を参照する。

4. **重要**: 転記した直後の `expected` は「実際に起きたバグの出力」であることが多い。バグ修正のロジックを実装したら、`expected` を**手で「あるべき出力」に書き換える**こと。書き換えずに放置すると、このテストはバグを固定化してしまい、修正の意味がなくなる。
5. `cargo test -p awase-windows --test journal_replay` を実行し、通ることを確認してからコミットする（P5規約「fix にはテストか記録を添える」も参照）。

## 制約・既知の限界

- 現状は `classify_conv_transition` 単体のリプレイのみ。フォーカス遷移・タイマー・実際の Win32 API 呼び出しシーケンスは再現しない。
- `ConvClassifyFixture` は JSON を手で編集する前提。将来的には実機ダンプ JSON から `ConvClassifyCall` エントリを自動抽出してフィクスチャに変換するスクリプトを用意すると転記の手間が減る（未実装）。
- フィクスチャの `expected` は開発者が正しいと判断した値であり、テスト自体は「その判断からの退行」を検知するだけで「その判断が正しいかどうか」は保証しない。
