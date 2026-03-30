# ADR-005: Shadow IME 状態追跡と IME トグルキー検出

## ステータス
採用

## コンテキスト
Modern UI アプリ（Win11 メモ帳、Chrome 等）では CrossProcess IME 検出が不正確（常に open=0）。IME キャッシュは Unreliable/Unknown アプリでは shadow state にフォールバックするが、shadow を更新する仕組みがなかった。config の `ime_sync` キーは空で、shadow はデフォルト true のまま。

結果: IME を OFF にしても NICOLA 変換が継続する問題。

## 決定
フックコールバック内で日本語キーボード固有の IME 制御キーを検出し、shadow_ime_on を直接更新:

- `0xF2` (半角/全角 activate), `0x16` (VK_IME_ON) → shadow ON
- `0xF3`, `0xF4` (半角/全角 deactivate), `0x1A` (VK_IME_OFF) → shadow OFF
- `0x19` (VK_KANJI) → shadow トグル

加えて、これらのキーが検出されたら `PostMessageW(WM_IME_KEY_DETECTED)` でメッセージループに即時キャッシュ更新を要求。

## 結果
- Modern UI アプリでの IME OFF 検出が即座に反映
- config の `ime_sync` 設定は引き続き使用可能（ユーザーカスタマイズ用）
- Shadow 追跡はフック内（ノンブロッキング）、キャッシュ更新はメッセージループ上

## 関連コミット
`0055604`, `8132a45`
