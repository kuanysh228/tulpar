#!/usr/bin/env bash
# Tulpar орнату: ~/.local/bin/tulpar және ~/.local/lib/libtulpar_rt.a
set -e
cd "$(dirname "$0")"
cargo build --release
mkdir -p ~/.local/bin ~/.local/lib
cp target/release/tulpar ~/.local/bin/tulpar
cp target/release/libtulpar_rt.a ~/.local/lib/libtulpar_rt.a
echo "орнатылды: ~/.local/bin/tulpar  (PATH-қа ~/.local/bin қосылғанын тексеріңіз)"
