#!/usr/bin/env bash
# A11(ADR-247): A9 と A10 の両方(F13〜F24 の追随を無効にした、修正前相当)。
set -e
bash "$(dirname "$0")/a9-no-fkey-observe.sh"
bash "$(dirname "$0")/a10-no-fkey-predict.sh"
