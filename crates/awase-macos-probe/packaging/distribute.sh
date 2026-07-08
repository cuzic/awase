#!/usr/bin/env bash
#
# distribute.sh — re-sign the awase-macos-probe .app for DISTRIBUTION and pack
# it into a .zip (and optionally a .dmg) distributable artifact.
#
# RUN THIS ON macOS ONLY, AND ONLY WITH A REAL APPLE DEVELOPER PROGRAM ACCOUNT.
# It invokes `codesign` with a "Developer ID Application" certificate, `ditto`,
# and `hdiutil` — none of which exist / are meaningful on the Linux dev machine
# where this file was authored. It has therefore only been syntax-checked
# (`bash -n`) on Linux. Real execution must be verified on Apple hardware with a
# valid Developer ID cert (project tracker task #19 / Phase M6). See README.md
# ("Distribution (Phase M6, not yet actionable)").
#
# WHY this is separate from build_probe_app.sh:
#   build_probe_app.sh (Task #13) produces a *development* .app with ad-hoc or a
#   local codesigning identity — good enough for TCC-grant stability during
#   iterative local testing, but NOT distributable: Gatekeeper on another Mac
#   would refuse to launch it. Public distribution requires (a) signing with a
#   "Developer ID Application: <name> (<TEAMID>)" certificate issued by the
#   Apple Developer Program, (b) the hardened runtime (--options runtime), and
#   (c) notarization (see notarize.sh). This script covers (a) and (b) and
#   produces the artifact that notarize.sh then submits for (c).
#
# Pipeline:
#   build_probe_app.sh  ->  distribute.sh  ->  notarize.sh
#   (dev .app)              (signed .zip/.dmg)   (notarized + stapled)
#
# Usage:
#   ./distribute.sh --identity "Developer ID Application: Jane Dev (AB12CD34EF)" \
#                   [--app <path to .app>] \
#                   [--out-dir <dir>] \
#                   [--dmg] \
#                   [--version <x.y.z>]
#
#   --identity  (required) Full "Developer ID Application: <name> (<TEAMID>)"
#               identity present in the login keychain. Distribution signing;
#               NOT ad-hoc.
#   --app       Path to the .app to re-sign. Default:
#               target/aarch64-apple-darwin/release/awase-macos-probe.app
#               (i.e. what `build_probe_app.sh --profile release` produced).
#   --out-dir   Where to write the .zip / .dmg. Default: <SCRIPT_DIR>/dist.
#   --dmg       Also produce a .dmg (via hdiutil) in addition to the .zip.
#   --version   Version string used in artifact filenames. Default: 0.1.0
#               (keep in sync with Cargo.toml / Info.plist).

set -euo pipefail

# --- resolve paths ----------------------------------------------------------
# This script lives at crates/awase-macos-probe/packaging/. The repo root is
# three levels up. Resolve relative to the script, not the caller's CWD.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

BIN_NAME="awase-macos-probe"
DEFAULT_APP="${REPO_ROOT}/target/aarch64-apple-darwin/release/${BIN_NAME}.app"

# --- defaults & arg parsing -------------------------------------------------
IDENTITY=""
APP_PATH="${DEFAULT_APP}"
OUT_DIR="${SCRIPT_DIR}/dist"
MAKE_DMG="false"
VERSION="0.1.0"

while [[ $# -gt 0 ]]; do
	case "$1" in
		--identity)
			IDENTITY="${2:?--identity requires a value}"
			shift 2
			;;
		--app)
			APP_PATH="${2:?--app requires a value}"
			shift 2
			;;
		--out-dir)
			OUT_DIR="${2:?--out-dir requires a value}"
			shift 2
			;;
		--version)
			VERSION="${2:?--version requires a value}"
			shift 2
			;;
		--dmg)
			MAKE_DMG="true"
			shift
			;;
		-h|--help)
			sed -n '2,45p' "${BASH_SOURCE[0]}"
			exit 0
			;;
		*)
			echo "error: unknown argument: $1" >&2
			exit 2
			;;
	esac
done

if [[ -z "${IDENTITY}" ]]; then
	echo "error: --identity is required (a 'Developer ID Application: <name> (<TEAMID>)' cert)." >&2
	echo "       Ad-hoc / local dev identities are NOT distributable — use build_probe_app.sh for those." >&2
	exit 2
fi

case "${IDENTITY}" in
	"Developer ID Application:"*) ;;
	*)
		# Not fatal (the user may know better), but distribution to other Macs
		# specifically requires a Developer ID Application cert; warn loudly.
		echo "warning: --identity does not start with 'Developer ID Application:' — Gatekeeper on" >&2
		echo "         other Macs will reject anything not signed with a Developer ID cert." >&2
		;;
esac

if [[ ! -d "${APP_PATH}" ]]; then
	echo "error: .app not found at ${APP_PATH}" >&2
	echo "       Run build_probe_app.sh --profile release first, or pass --app <path>." >&2
	exit 1
fi

APP_BASENAME="$(basename "${APP_PATH}")"

# --- re-sign for distribution (hardened runtime) ----------------------------
# --options runtime enables the hardened runtime, which notarization requires.
# --timestamp obtains a secure timestamp from Apple's TSA (also required for
# notarization). --force replaces the dev signature from build_probe_app.sh.
# --deep is used to reach any nested code; for a single-binary bundle it is
# effectively a no-op but is harmless and future-proofs against added dylibs.
echo "==> codesign (distribution, hardened runtime) with: ${IDENTITY}"
codesign --force --deep --options runtime --timestamp \
	--sign "${IDENTITY}" "${APP_PATH}"

echo "==> verifying signature (strict)"
codesign --verify --strict --verbose=2 "${APP_PATH}"

# --- produce artifacts ------------------------------------------------------
mkdir -p "${OUT_DIR}"
ZIP_PATH="${OUT_DIR}/${BIN_NAME}-${VERSION}.zip"

# ditto -c -k --keepParent is Apple's recommended way to zip a .app for
# notarytool submission: it preserves the bundle structure, symlinks, and
# extended attributes that a plain `zip` would corrupt.
echo "==> ditto -> ${ZIP_PATH}"
rm -f "${ZIP_PATH}"
ditto -c -k --keepParent "${APP_PATH}" "${ZIP_PATH}"

if [[ "${MAKE_DMG}" == "true" ]]; then
	DMG_PATH="${OUT_DIR}/${BIN_NAME}-${VERSION}.dmg"
	# Stage the .app alone in a temp dir so the .dmg contains just the bundle.
	STAGE_DIR="$(mktemp -d)"
	trap 'rm -rf "${STAGE_DIR}"' EXIT
	cp -R "${APP_PATH}" "${STAGE_DIR}/${APP_BASENAME}"

	echo "==> hdiutil create -> ${DMG_PATH}"
	rm -f "${DMG_PATH}"
	hdiutil create \
		-volname "${BIN_NAME}" \
		-srcfolder "${STAGE_DIR}" \
		-ov \
		-format UDZO \
		"${DMG_PATH}"

	# The .dmg itself should also be signed with the Developer ID cert so
	# Gatekeeper trusts the container, not only the .app inside it.
	echo "==> codesign the .dmg"
	codesign --force --timestamp --sign "${IDENTITY}" "${DMG_PATH}"
fi

echo ""
echo "OK: distributable artifacts in ${OUT_DIR}"
echo "    zip: ${ZIP_PATH}"
if [[ "${MAKE_DMG}" == "true" ]]; then
	echo "    dmg: ${DMG_PATH}"
fi
echo ""
echo "Next: notarize with ./notarize.sh (see README.md, Phase M6)."
