#!/usr/bin/env bash
#
# notarize.sh — submit a distributable artifact to Apple's notary service and
# staple the resulting ticket onto it.
#
# RUN THIS ON macOS ONLY, AND ONLY WITH A REAL APPLE DEVELOPER PROGRAM ACCOUNT.
# It invokes `xcrun notarytool` and `xcrun stapler`, which exist only inside a
# macOS + Xcode command-line-tools install and require valid Apple credentials.
# This file was authored on Linux and has only been syntax-checked (`bash -n`).
# Real execution must be verified on Apple hardware (project tracker task #19 /
# Phase M6). See README.md ("Distribution (Phase M6, not yet actionable)").
#
# WHERE THIS FITS:
#   build_probe_app.sh  ->  distribute.sh  ->  notarize.sh   (this script)
#   (dev .app)              (signed .zip/.dmg)   (notarized + stapled artifact)
#
# WHY notarization: since macOS 10.15, Gatekeeper refuses to launch downloaded
# software from an identified developer unless Apple has *notarized* it (an
# automated malware scan of the signed artifact). notarytool uploads the
# artifact, waits for the verdict, and — on success — Apple issues a ticket.
# `stapler staple` then attaches that ticket to the artifact so it validates
# even offline. The artifact MUST already be signed with a Developer ID cert
# and hardened runtime (distribute.sh does that) or notarization is rejected.
#
# ---------------------------------------------------------------------------
# AUTHENTICATION (choose ONE; passed to `notarytool` — this script does not
# hardcode any secret and reads everything from the environment):
#
#   The exact auth mechanism to standardize on for this project is NOT decided
#   here — that is a call for whoever runs this on real hardware with real Apple
#   Developer credentials. `notarytool` supports two forms; both are wired below
#   and selected by which env vars are set:
#
#   (A) App Store Connect API key (recommended for CI — no interactive 2FA):
#         AC_API_KEY_ID       — the key's Key ID
#         AC_API_ISSUER_ID    — the issuer UUID
#         AC_API_KEY_PATH     — path to the downloaded AuthKey_<KeyID>.p8 file
#
#   (B) Apple ID + app-specific password:
#         AC_APPLE_ID         — the developer Apple ID (email)
#         AC_TEAM_ID          — the 10-char Team ID
#         AC_APP_PASSWORD     — an app-specific password (appleid.apple.com),
#                               NOT the account's real password
#
#   NEVER commit these. Prefer a keychain profile
#   (`xcrun notarytool store-credentials`) on a real dev machine; the env-var
#   forms below exist so this can also run in CI once secrets are configured.
# ---------------------------------------------------------------------------
#
# Usage:
#   ./notarize.sh <path-to-.zip-or-.dmg>
#
#   The argument is the artifact produced by distribute.sh
#   (e.g. dist/awase-macos-probe-0.1.0.zip or .dmg).

set -euo pipefail

# --- arg parsing ------------------------------------------------------------
if [[ $# -ne 1 || "$1" == "-h" || "$1" == "--help" ]]; then
	sed -n '2,60p' "${BASH_SOURCE[0]}"
	[[ $# -ne 1 ]] && exit 2 || exit 0
fi

ARTIFACT="$1"
if [[ ! -f "${ARTIFACT}" ]]; then
	echo "error: artifact not found: ${ARTIFACT}" >&2
	exit 1
fi

case "${ARTIFACT}" in
	*.zip|*.dmg) ;;
	*)
		echo "error: artifact must be a .zip or .dmg (got '${ARTIFACT}')." >&2
		echo "       Produce one with ./distribute.sh first." >&2
		exit 2
		;;
esac

# --- select auth form from environment --------------------------------------
NOTARY_AUTH_ARGS=()
if [[ -n "${AC_API_KEY_ID:-}" && -n "${AC_API_ISSUER_ID:-}" && -n "${AC_API_KEY_PATH:-}" ]]; then
	echo "==> auth: App Store Connect API key (key-id ${AC_API_KEY_ID})"
	if [[ ! -f "${AC_API_KEY_PATH}" ]]; then
		echo "error: AC_API_KEY_PATH points to a missing file: ${AC_API_KEY_PATH}" >&2
		exit 1
	fi
	NOTARY_AUTH_ARGS=(
		--key "${AC_API_KEY_PATH}"
		--key-id "${AC_API_KEY_ID}"
		--issuer "${AC_API_ISSUER_ID}"
	)
elif [[ -n "${AC_APPLE_ID:-}" && -n "${AC_TEAM_ID:-}" && -n "${AC_APP_PASSWORD:-}" ]]; then
	echo "==> auth: Apple ID + app-specific password (${AC_APPLE_ID})"
	NOTARY_AUTH_ARGS=(
		--apple-id "${AC_APPLE_ID}"
		--team-id "${AC_TEAM_ID}"
		--password "${AC_APP_PASSWORD}"
	)
else
	echo "error: no notarization credentials in the environment." >&2
	echo "       Set EITHER the API-key trio (AC_API_KEY_ID / AC_API_ISSUER_ID / AC_API_KEY_PATH)" >&2
	echo "       OR the Apple-ID trio (AC_APPLE_ID / AC_TEAM_ID / AC_APP_PASSWORD)." >&2
	echo "       See the header of this script for details." >&2
	exit 2
fi

# --- submit & wait ----------------------------------------------------------
# --wait blocks until Apple returns Accepted/Invalid, so the exit status of
# this script reflects the notarization verdict (set -e aborts on rejection).
echo "==> notarytool submit ${ARTIFACT} --wait"
xcrun notarytool submit "${ARTIFACT}" "${NOTARY_AUTH_ARGS[@]}" --wait

# --- staple -----------------------------------------------------------------
# Attach the notarization ticket to the artifact so Gatekeeper validates it
# even without a network round-trip. For a .zip, stapling is done on the .app
# after unzip (tickets can't attach to a plain zip); for a .dmg the ticket
# attaches to the .dmg directly.
case "${ARTIFACT}" in
	*.dmg)
		echo "==> stapler staple ${ARTIFACT}"
		xcrun stapler staple "${ARTIFACT}"
		xcrun stapler validate "${ARTIFACT}"
		;;
	*.zip)
		# A .zip cannot itself be stapled. The ticket is fetched at launch from
		# Apple, so the zip is already distributable once Accepted above. To
		# ship a *stapled* .app, staple the unzipped bundle and re-zip it.
		echo "note: a .zip cannot be stapled directly."
		echo "      The submission was Accepted, so the .app inside is notarized"
		echo "      and Gatekeeper will fetch the ticket online at first launch."
		echo "      For an offline-validatable artifact, unzip the .app, run"
		echo "      'xcrun stapler staple <app>' on it, then re-zip with"
		echo "      'ditto -c -k --keepParent'. (Prefer --dmg in distribute.sh"
		echo "      for a directly-stapleable artifact.)"
		;;
esac

echo ""
echo "OK: notarization complete for ${ARTIFACT}"
