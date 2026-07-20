#!/usr/bin/env bash
# awase の winget マニフェストを最新(または指定)リリースの内容で更新し、
# winget validate / wingetcreate submit を実行する。
#
# MSYS2 bash（例: C:\Users\<user>\scoop\persist\msys2）上での実行を前提とする。
# winget.exe / wingetcreate.exe は MSYS2 の PATH に乗らないことが多く、
# また Invoke-WebRequest 等の一部処理は PowerShell 側で完結させたいため、
# 実際のコマンド呼び出しは powershell.exe 経由で行う。
#
# 使い方:
#   WINGET_CREATE_TOKEN=<public_repo スコープの GitHub PAT> ./submit.sh [VERSION]
#   VERSION省略時は cuzic/awase の最新リリースタグを使う。
set -euo pipefail

REPO="cuzic/awase"
IDENTIFIER="cuzic.awase"
MANIFEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WINGET_EXE='$env:LOCALAPPDATA\Microsoft\WindowsApps\winget.exe'
WINGETCREATE_EXE='$env:LOCALAPPDATA\Microsoft\WindowsApps\wingetcreate.exe'

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    VERSION=$(gh release list --repo "$REPO" --limit 1 --json tagName -q '.[0].tagName' | sed 's/^v//')
fi
[ -n "$VERSION" ] || { echo "VERSION を取得できませんでした（引数で指定してください）" >&2; exit 1; }

echo "==> awase v${VERSION} の winget マニフェストを準備します"

WORKDIR=$(mktemp -d)
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

MSI_NAME="awase-${VERSION}-x64.msi"
MSI_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${MSI_NAME}"

echo "==> ${MSI_URL} をダウンロードして SHA256 を計算します"
curl -fsSL -o "${WORKDIR}/${MSI_NAME}" "$MSI_URL"
SHA256=$(sha256sum "${WORKDIR}/${MSI_NAME}" | awk '{print toupper($1)}')
echo "    SHA256: ${SHA256}"
rm -f "${WORKDIR}/${MSI_NAME}"

echo "==> テンプレートから3ファイルを生成します"
sed -e "s/^PackageVersion: .*/PackageVersion: ${VERSION}/" \
    "${MANIFEST_DIR}/${IDENTIFIER}.yaml" \
    > "${WORKDIR}/${IDENTIFIER}.yaml"

sed -e "s/^PackageVersion: .*/PackageVersion: ${VERSION}/" \
    -e "s#    InstallerUrl: .*#    InstallerUrl: ${MSI_URL}#" \
    -e "s/^    InstallerSha256: .*/    InstallerSha256: ${SHA256}/" \
    "${MANIFEST_DIR}/${IDENTIFIER}.installer.yaml" \
    > "${WORKDIR}/${IDENTIFIER}.installer.yaml"

sed -e "s/^PackageVersion: .*/PackageVersion: ${VERSION}/" \
    -e "s#^ReleaseNotesUrl: .*#ReleaseNotesUrl: https://github.com/${REPO}/releases/tag/v${VERSION}#" \
    "${MANIFEST_DIR}/${IDENTIFIER}.locale.en-US.yaml" \
    > "${WORKDIR}/${IDENTIFIER}.locale.en-US.yaml"

WIN_WORKDIR=$(cygpath -w "$WORKDIR")

echo "==> winget validate を実行します"
powershell.exe -NoProfile -NonInteractive -Command \
    "& \"${WINGET_EXE}\" validate '${WIN_WORKDIR}'"

echo
read -r -p "検証OKでした。wingetcreate submit で PR を送信しますか？ [y/N] " ANSWER
if [ "$ANSWER" != "y" ] && [ "$ANSWER" != "Y" ]; then
    NOTRAP_DIR="${WORKDIR}"
    trap - EXIT
    echo "送信を中止しました。生成済みマニフェストは ${NOTRAP_DIR} に残しています（手動で削除してください）"
    exit 0
fi

: "${WINGET_CREATE_TOKEN:?WINGET_CREATE_TOKEN 環境変数に public_repo スコープの GitHub PAT を設定してください}"

echo "==> wingetcreate submit を実行します"
powershell.exe -NoProfile -NonInteractive -Command \
    "& \"${WINGETCREATE_EXE}\" submit --token '${WINGET_CREATE_TOKEN}' '${WIN_WORKDIR}'"

echo "==> 完了しました。packaging/winget/ 配下のテンプレート（バージョン以外の記述内容）に変更があれば、別途 git commit してください"
