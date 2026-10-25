#!/usr/bin/env bash
# Tulpar тест жүгіртушісі: tests/*.tlp салыстырады tests/*.out-пен, tests/errors/*.tlp қателерін тексереді.
cd "$(dirname "$0")/.."
T=./target/release/tulpar
[ -x "$T" ] || { echo "алдымен: cargo build --release"; exit 2; }
pass=0; fail=0
for src in tests/*.tlp tests/progs/*.tlp; do
    exp="${src%.tlp}.out"
    [ -f "$exp" ] || continue
    for mode in "" "-O0"; do
        got=$($T run "$src" $mode 2>&1)
        if [ "$got" == "$(cat "$exp")" ]; then pass=$((pass+1)); else
            fail=$((fail+1)); echo "ҚАТЕ: $src $mode"; diff <(echo "$got") "$exp" | head -5
        fi
    done
done
# интерпретатор (ендірілген режим): компиляцияланған нәтижемен бірдей болуы керек
# (файл, желі және қолмен жад қолжетімсіз, сондықтан бұл тесттер өткізіледі)
for src in tests/*.tlp tests/progs/*.tlp; do
    exp="${src%.tlp}.out"
    [ -f "$exp" ] || continue
    case "$src" in tests/files_net.tlp|tests/manual_refs.tlp|tests/stdlib.tlp) continue;; esac
    got=$($T interp "$src" 2>&1)
    if [ "$got" == "$(cat "$exp")" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "ҚАТЕ (интерпретатор): $src"; diff <(echo "$got") "$exp" | head -5
    fi
done
for src in tests/errors/*.tlp; do
    want=$(head -1 "$src" | sed 's/^# ШЫҒЫС: //')
    got=$($T run "$src" 2>&1); code=$?
    if [ $code -ne 0 ] && [[ "$got" == *"$want"* ]]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "ҚАТЕ: $src (код $code) күтілген: «$want»"; echo "$got" | head -3
    fi
done
echo "өтті: $pass, құлады: $fail"
[ $fail -eq 0 ]
