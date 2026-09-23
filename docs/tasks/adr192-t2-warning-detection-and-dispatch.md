# ADR-192 T2: 検出・警告の分岐と一度きり表示（決定2）を実装する

状態: 未着手（2026-09-22起票、[ADR192-T1](adr192-t1-state-dependent-key-classifier.md)完了後に着手）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)（rev8）
決定2は、T1が実装する判定関数の結果を使い、起動時・設定リロード時・IME種別確定時に検出し、
一度きり警告する。**このタスクは「検出・分岐・一度きり判定」ロジックまでを担当し、実際の
警告UI表示（awase-settings/トレイ通知等の見た目）は範囲外**——既存の`msime_key_assignment`
警告ポップアップの仕組みに乗せる想定だが、UI実装そのものは呼び出し側（既存のポップアップ
機構）に委ねる。

## 実装対象（詳細・根拠はADR-192決定2本文参照）

1. **親指キーか否かの分岐**: 対象VKが`general.left_thumb_key`/`right_thumb_key`
   （`muhenkan_vk`/`henkan_vk`ではない——ADR-192 round2 D-3参照、この2つは無変換/変換限定の
   内部値で分岐条件には使えない）に設定されているかで分岐する。
   - 親指キーとして使っている場合: 決定2の新規警告は**出さない**。既存の
     `msime_key_assignment::check_and_warn`と同型の判定をGJI側にも拡張する
     （新しい独立ダイアログは追加しない）。案内文は「IME側の割り当てを解除し、awaseの
     明示config（`*_solo_tap_ime_action`、またはADR192-T4の新経路）に委ねてください」。
   - 親指キーだが無変換/変換ではない場合: 決定2の新規警告もADR192-T4の救済も届かない
     ため、既存のT-16警告（`validate_thumb_key_in_ime_combos`）がその旨を案内する経路の
     まま残す（このタスクでは何もしない、無案内にしないことの確認のみ）。
   - 親指キーでない場合のみ、次の(A)/(B)/(ii)の警告を出す。
2. **入力モードキーは対象外**: `Eisu`/`Hiragana`/`Katakana`はT1の判定結果によらず警告を
   出さない（awaseは追随するだけで書かないキーのため）。
3. **警告文言の3分岐**（T1が返す`Classification`に対応）:
   - (A) 開閉軸の状態依存: 「モードがずれる可能性があるので冪等なキーに変更してください」
   - (B) 未確定文字列の行方の危険（`HankakuZenkaku`/`Kanji`のみ）: 「入力中に押すと変換中の
     文字が消える/確定してしまう場合があります（冪等なキーでも起こりうる、置き換えでは
     解決しません）」——**1回だけの情報通知**とし、該当キーごとに別々の警告は出さない。
   - `UserOverride`（T1の`CannotPredict(UserOverride)`）: 「awaseはこのキーの効果を
     追随できない可能性がある」
   - `AmbiguousKeymap`/`InsufficientData`は**沈黙**（警告を出さない）。
4. **一度きり判定（同一性キー）**:
   - (A)/`UserOverride`の警告: GJIは`KeymapCache`の`stamp`（`config1.db`のmtime+長さ）を
     流用する。MS-IME本体はレジストリなので既存のpacked-bits方式
     （`msime_key_assignment.rs:159-163`）で同一性を判定する。この判定は`KeymapCache`の
     `checked_at_ms`等の予測器側キャッシュ状態をリセットしない（読み取り専用の参照）。
   - (B)の警告: `msime_key_assignment.rs:154-158`の既存の前例に倣い、「実際に通知した
     内容そのもの（該当したキーの集合）」を同一性キーにする（プリセット種別だけをキーに
     しない——判定不能から警告対象へ変わる遷移等を見落とすため）。
5. **設定でオフにできる**: `warn_state_dependent_mode_keys`（既定on）で警告を止められる
   ようにする。警告はブロックしない（無視できる）。

## 完了条件・テスト

- `cargo test -p awase-windows`に単体テストを追加: 警告の一度きり・再警告・
  「警告しない」の動作、親指キー設定時に既存`check_and_warn`型の警告に分岐すること、
  (A)/(B)/`UserOverride`で警告文言が分かれること、入力モードキーには警告が出ないこと。
- T1の判定関数を呼び出すだけで、独自の解釈ロジックを再実装しないこと。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定2
- [ADR192-T1](adr192-t1-state-dependent-key-classifier.md)（前提）
- [ADR192-T3](adr192-t3-awase-settings-guided-replacement.md)（このタスクの警告結果を
  awase-settingsで表示・置き換え操作につなげる側）
