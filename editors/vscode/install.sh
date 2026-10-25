#!/usr/bin/env bash
# Tulpar синтаксис бояуын VS Code-ға орнату (Code - OSS / Code / VSCodium)
set -e
cd "$(dirname "$0")"
for d in ~/.vscode-oss/extensions ~/.vscode/extensions ~/.vscode-insiders/extensions; do
    [ -d "$d" ] || continue
    t="$d/tulpar.tulpar-0.1.0"
    rm -rf "$t"; mkdir -p "$t"
    cp -r package.json language-configuration.json syntaxes "$t/"
    echo "орнатылды: $t"
done
echo "VS Code-ты қайта жүктеңіз: Ctrl+Shift+P -> Developer: Reload Window"
