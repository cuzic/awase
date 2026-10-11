# CI のデバッグセッション(Tailscale + SSH)

GitHub ホスト型の Windows ランナーを Tailscale につなぎ、SSH で入って試行錯誤するための手順。
`.github/workflows/ci-debug-session.yml`(ブランチ `ci/debug-session` への push で起動)。

## 用意するもの(所有者)

1. Tailscale の管理画面で OAuth クライアントを作る。権限は `auth_keys` の書き込み(Devices Core / Auth Keys: write)、タグは `tag:ci`。
2. GitHub のリポジトリのシークレットに、`TS_OAUTH_CLIENT_ID` と `TS_OAUTH_SECRET` を登録する。
3. tailnet の ACL に、タグの所有者と、開発マシンからランナーへの SSH を許す規則を足す:

```json
"tagOwners": { "tag:ci": ["autogroup:admin"] },
"acls": [
  { "action": "accept", "src": ["autogroup:member"], "dst": ["tag:ci:22"] }
]
```

(既存の ACL がある場合は、その `acls` に1行足す。ランナー → 開発マシンの通信は許さない。)

## 使い方

1. `git commit --allow-empty -m "debug session" && git push origin ci/debug-session` で起動する。
2. ジョブのログ(`Print how to connect, then wait`)に、ホスト名 `gh-win-<run_id>` と tailnet の IP が出る。
3. 開発マシンから `ssh -i ~/.ssh/awase_ci_debug runneradmin@gh-win-<run_id>`(公開鍵はワークフローに書いてある。秘密鍵は開発マシンにだけある)。
4. 終わったらランナー上で `New-Item C:\ci-debug-stop`。放置しても、最大 240 分で終わる。

## 注意

- 起動できるのはリポジトリの所有者だけ(`github.actor == github.repository_owner`)。
- パスワード認証は切ってある。公開鍵のみ。ランナーは一時的なノードで、ジョブが終わると消える。
- 鍵は使い捨て。漏れた疑いがあれば、鍵を作り直して、ワークフローの公開鍵を差し替える。
- ランナーの分数を消費する(公開リポジトリなら無料。非公開では Windows は課金の倍率が高い)。
