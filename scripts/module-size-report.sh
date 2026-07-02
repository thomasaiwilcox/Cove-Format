#!/usr/bin/env sh
set -eu

limit="${1:-2000}"

find crates -path '*/src/*.rs' -type f -print0 |
    xargs -0 wc -l |
    awk -v limit="$limit" '$1 ~ /^[0-9]+$/ && $2 != "total" && $1 >= limit { print }' |
    sort -nr
