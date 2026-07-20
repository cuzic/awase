#!/usr/bin/env bash
# awase の Chocolatey パッケージを最新(または指定)リリースの内容で更新し、
# choco pack / (任意で)ローカルインストールテスト / choco push を実行する。
#
# 使い方:
#   CHOCO_API_KEY=<community.chocolatey.org のAPIキー> ./submit.sh [VERSION]
#   VERSION省略時は cuzic/awase の最新リリースタグを使う。
set -euo pipefail

REPO="cuzic/awase"
PKG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    VERSION=$(gh release list --repo "$REPO" --limit 1 --json tagName -q '.[0].tagName' | sed 's/^v//')
fi
[ -n "$VERSION" ] || { echo "VERSION を取得できませんでした（引数で指定してください）" >&2; exit 1; }

echo "==> awase v${VERSION} の Chocolatey パッケージを準備します"

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

echo "==> テンプレートから nuspec / install script を生成します"
mkdir -p "${WORKDIR}/tools"

sed -e "s#<version>.*</version>#<version>${VERSION}</version>#" \
    "${PKG_DIR}/awase.nuspec" \
    > "${WORKDIR}/awase.nuspec"

sed -e "s/^\$version = .*/\$version = '${VERSION}'/" \
    -e "s#^  checksum64     = .*#  checksum64     = '${SHA256}'#" \
    "${PKG_DIR}/tools/chocolateyinstall.ps1" \
    > "${WORKDIR}/tools/chocolateyinstall.ps1"

cp "${PKG_DIR}/tools/chocolateyuninstall.ps1" "${WORKDIR}/tools/"

echo "==> choco pack を実行します"
(cd "$WORKDIR" && choco pack)

NUPKG="${WORKDIR}/awase.${VERSION}.nupkg"
[ -f "$NUPKG" ] || { echo "nupkg が生成されませんでした: $NUPKG" >&2; exit 1; }

echo
read -r -p "ローカルインストールテストをしますか？（管理者権限のシェル推奨） [y/N] " TESTANS
if [ "$TESTANS" = "y" ] || [ "$TESTANS" = "Y" ]; then
    choco install awase -s "$WORKDIR" -y
    echo "動作確認後、'choco uninstall awase -y' で片付けてから続行してください"
fi

echo
read -r -p "community.chocolatey.org に push しますか？ [y/N] " ANSWER
if [ "$ANSWER" != "y" ] && [ "$ANSWER" != "Y" ]; then
    NOTRAP_DIR="${WORKDIR}"
    trap - EXIT
    echo "push を中止しました。生成済み nupkg は ${NOTRAP_DIR} に残しています（手動で削除してください）"
    exit 0
fi

: "${CHOCO_API_KEY:?CHOCO_API_KEY 環境変数に community.chocolatey.org のAPIキーを設定してください}"

echo "==> choco push を実行します"
choco push "$NUPKG" --source https://push.chocolatey.org/ --api-key "$CHOCO_API_KEY"

echo "==> 完了しました。packaging/choco/ 配下のテンプレート（バージョン以外の記述内容）に変更があれば、別途 git commit してください"
