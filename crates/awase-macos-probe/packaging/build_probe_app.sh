#!/usr/bin/env bash
#
# build_probe_app.sh — assemble & code-sign a development .app bundle for the
# awase-macos-probe diagnostic binary.
#
# RUN THIS ON macOS ONLY. It invokes `cargo build` for an Apple target and
# `codesign`, neither of which is meaningful (or `codesign` even present) on
# the Linux dev machine where this file was authored. It has therefore only
# been syntax-checked (`bash -n`) on Linux — real execution must be verified on
# actual Apple hardware (project tracker task #19) before Task #4 (permissions)
# real-device verification is meaningful.
#
# WHY a .app + stable signing identity, not just `cargo run`:
#   macOS's TCC (privacy permission) database keys grants — Input Monitoring,
#   Accessibility, etc. — off the *code identity* of the signed binary. A bare
#   ad-hoc sign (`codesign --sign -` with no explicit identifier) can produce a
#   different code identity on each rebuild, so a permission a human granted
#   once gets silently revoked after the next `cargo build`. Signing with a
#   stable local identity (or, weaker, ad-hoc + a fixed --identifier) keeps the
#   identity constant across rebuilds so the grant persists. See
#   crates/awase-macos-probe/packaging/README.md and the project design memory
#   (macos_probe_interfaces, ".app identity" bullet).
#
# Usage:
#   ./build_probe_app.sh [--target <triple>] [--profile debug|release] \
#                        [--identity <codesign identity name>]
#
#   --target    aarch64-apple-darwin (default) | x86_64-apple-darwin
#   --profile   debug (default) | release
#   --identity  Name of a persistent local codesigning identity in your login
#               keychain (recommended, most stable). If omitted, falls back to
#               plain ad-hoc signing with a fixed --identifier (weaker; TCC may
#               still need re-granting occasionally).

set -euo pipefail

# --- resolve paths ----------------------------------------------------------
# This script lives at crates/awase-macos-probe/packaging/. The repo root is
# three levels up. Resolve relative to the script, not the caller's CWD.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
INFO_PLIST="${SCRIPT_DIR}/Info.plist"
CODESIGN_REPORT="${SCRIPT_DIR}/last_codesign_report.txt"

BUNDLE_ID="tools.awase.macos-probe"
BIN_NAME="awase-macos-probe"

# --- defaults & arg parsing -------------------------------------------------
TARGET="aarch64-apple-darwin"
PROFILE="debug"
IDENTITY=""

while [[ $# -gt 0 ]]; do
	case "$1" in
		--target)
			TARGET="${2:?--target requires a value}"
			shift 2
			;;
		--profile)
			PROFILE="${2:?--profile requires a value}"
			shift 2
			;;
		--identity)
			IDENTITY="${2:?--identity requires a value}"
			shift 2
			;;
		-h|--help)
			sed -n '2,40p' "${BASH_SOURCE[0]}"
			exit 0
			;;
		*)
			echo "error: unknown argument: $1" >&2
			exit 2
			;;
	esac
done

case "${TARGET}" in
	aarch64-apple-darwin|x86_64-apple-darwin) ;;
	*)
		echo "error: --target must be aarch64-apple-darwin or x86_64-apple-darwin (got '${TARGET}')" >&2
		exit 2
		;;
esac

case "${PROFILE}" in
	debug|release) ;;
	*)
		echo "error: --profile must be debug or release (got '${PROFILE}')" >&2
		exit 2
		;;
esac

# --- build ------------------------------------------------------------------
CARGO_ARGS=(build -p "${BIN_NAME}" --target "${TARGET}")
if [[ "${PROFILE}" == "release" ]]; then
	CARGO_ARGS+=(--release)
fi

echo "==> cargo ${CARGO_ARGS[*]}"
( cd "${REPO_ROOT}" && cargo "${CARGO_ARGS[@]}" )

BUILT_BIN="${REPO_ROOT}/target/${TARGET}/${PROFILE}/${BIN_NAME}"
if [[ ! -f "${BUILT_BIN}" ]]; then
	echo "error: expected built binary not found at ${BUILT_BIN}" >&2
	exit 1
fi

# --- assemble .app ----------------------------------------------------------
APP_DIR="${REPO_ROOT}/target/${TARGET}/${PROFILE}/${BIN_NAME}.app"
CONTENTS="${APP_DIR}/Contents"

echo "==> assembling ${APP_DIR}"
rm -rf "${APP_DIR}"
mkdir -p "${CONTENTS}/MacOS" "${CONTENTS}/Resources"
cp "${INFO_PLIST}" "${CONTENTS}/Info.plist"
cp "${BUILT_BIN}" "${CONTENTS}/MacOS/${BIN_NAME}"
chmod +x "${CONTENTS}/MacOS/${BIN_NAME}"

# --- code-sign --------------------------------------------------------------
if [[ -n "${IDENTITY}" ]]; then
	echo "==> codesign with local identity: ${IDENTITY}"
	codesign --force --deep --sign "${IDENTITY}" "${APP_DIR}"
else
	# Weaker fallback: plain ad-hoc signing. Setting an explicit --identifier
	# to the fixed bundle id is documented by Apple as making the code
	# requirement more stable across rebuilds than bare ad-hoc with no
	# identifier, but this is still weaker than a real local signing identity:
	# TCC grants may occasionally need re-granting. Pass --identity to avoid.
	echo "==> codesign ad-hoc (no --identity given; weaker TCC stability)"
	codesign --force --deep --sign - --identifier "${BUNDLE_ID}" "${APP_DIR}"
fi

# --- record codesign identity for cross-rebuild diffing ---------------------
# A human on real hardware can diff this file across rebuilds to confirm the
# code identity (CDHash / Identifier / TeamIdentifier) stays stable — that
# stability is exactly what keeps TCC grants alive.
echo "==> codesign --display --verbose=4 -> ${CODESIGN_REPORT}"
codesign --display --verbose=4 "${APP_DIR}" >"${CODESIGN_REPORT}" 2>&1 || true

echo ""
echo "OK: ${APP_DIR}"
