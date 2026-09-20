# ADR-186 実機E2E(ime_key_matrix)

無変換/変換/ひらがなキーの押下に対する、**実IMEの状態**と**awase の Engine 切り替え**を、
実機で自動検証するハーネス(RPA的なキー注入)。手順は10ステップ(ATOKプリセット):
`ひらがな→無変換→無変換→ひらがな→無変換→無変換→ひらがな→無変換→無変換→ひらがな`。

## 仕組み
- `crates/awase-windows/examples/ime_key_matrix_spike.rs --auto` が、手順のキーを `SendInput` で注入する
  (前提の状態にするための準備キーも自動)。実IME(ImmGet\* / WM_IME_CONTROL / TSF compartment)を
  押下前・+100/+400/+1500ms で記録する。
- 注入には `dwExtraInfo = 0x5350494B` の目印を付ける。awase は環境変数 `AWASE_TEST_INJECTION=1`
  のときだけ、この目印付き注入を物理キーとして扱う(`hook.rs::is_test_injection`)。本番は変わらない。
  (目印なしの注入は awase では「注入」扱いになり、押下時点の belief 追随を通らない。
  実測: Engine 追随が +1ms→+546ms/欠落。`check.py` が FAIL にする)
- `check.py` が、スパイクのログと awase のデバッグログを突き合わせ、`EXPECT` の期待表と照合する。

## 実行
1. Windows側で awase を `AWASE_TEST_INJECTION=1`・`RUST_LOG=debug` で起動する
   (ADR-186 の実装ブランチのビルド、`gji_thumb_key_ime_toggle=true`)。
2. `clipwire-targets.example.toml` のターゲットを登録・承認する。
3. `./run.sh`(実行中の約40秒は、Windows機のキーボード・マウスに触らない)。

## 判定の追加・変更
`check.py` の `EXPECT` を編集する。`engine=("none", 1500)` は「押下後1500msの間 Engine が activated に
ならない」、`("activated", 300)` は「300ms以内に activated になる」。
