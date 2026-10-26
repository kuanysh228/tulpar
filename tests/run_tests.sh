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
# ядро режимі (QEMU қажет): tests/os/*.tlp -> tests/os/*.out, tests/os/errors/*.tlp -> күтілген қате мәтіні
if command -v qemu-system-x86_64 >/dev/null && command -v objcopy >/dev/null && [ -f target/release/libtulpar_kernel.a ]; then
    for src in tests/os/*.tlp; do
        exp="${src%.tlp}.out"
        got=$(timeout 120 $T os run "$src" 2>&1)
        if [ "$got" == "$(cat "$exp")" ]; then pass=$((pass+1)); else
            fail=$((fail+1)); echo "ҚАТЕ (ядро): $src"; diff <(echo "$got") "$exp" | head -5
        fi
    done
    for src in tests/os/errors/*.tlp; do
        want=$(head -1 "$src" | sed 's/^# ШЫҒЫС: //')
        got=$(timeout 120 $T os run "$src" 2>&1); code=$?
        if [ $code -ne 0 ] && [[ "$got" == *"$want"* ]]; then pass=$((pass+1)); else
            fail=$((fail+1)); echo "ҚАТЕ (ядро): $src (код $code) күтілген: «$want»"; echo "$got" | head -3
        fi
    done
else
    echo "(ядро тесттері өткізілді: qemu-system-x86_64 жоқ)"
fi
# Windows .exe (wine қажет): барлық тесттер .exe ретінде құрастырылып wine-да орындалады; TULPAR_TEST_WINE=0 өшіреді
if command -v wine >/dev/null && [ -f target/release/libtulpar_win.a ] && [ "${TULPAR_TEST_WINE:-1}" != 0 ]; then
    export WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml=d"
    wexe=$(mktemp -u /tmp/tulpar-wine-XXXXXX.exe)
    for src in tests/*.tlp tests/progs/*.tlp; do
        exp="${src%.tlp}.out"
        [ -f "$exp" ] || continue
        $T build "$src" -o "$wexe" >/dev/null 2>&1 || { fail=$((fail+1)); echo "ҚАТЕ (windows құрастыру): $src"; continue; }
        got=$(timeout 120 wine "$wexe" 2>/dev/null | tr -d '\r')
        if [ "$got" == "$(cat "$exp")" ]; then pass=$((pass+1)); else
            fail=$((fail+1)); echo "ҚАТЕ (windows): $src"; diff <(echo "$got") "$exp" | head -5
        fi
    done
    for src in tests/errors/*.tlp; do
        want=$(head -1 "$src" | sed 's/^# ШЫҒЫС: //')
        if ! got=$($T build "$src" -o "$wexe" 2>&1); then :; else got=$(timeout 120 wine "$wexe" 2>&1 | tr -d '\r'); fi
        if [[ "$got" == *"$want"* ]]; then pass=$((pass+1)); else
            fail=$((fail+1)); echo "ҚАТЕ (windows): $src күтілген: «$want»"; echo "$got" | head -3
        fi
    done
    rm -f "$wexe"
else
    echo "(Windows тесттері өткізілді: wine жоқ)"
fi
echo "өтті: $pass, құлады: $fail"
[ $fail -eq 0 ]
