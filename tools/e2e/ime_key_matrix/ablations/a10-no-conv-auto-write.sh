#!/usr/bin/env bash
# A10: A8(経路9) と A9(経路1+2) を同時に撤去する(conv 軸の自動書き込みを全部止めた構成)。
d="$(dirname "$0")"
bash "$d/a8-no-focus-probe-roman.sh" && bash "$d/a9-no-roman-completion.sh"
