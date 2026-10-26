#!/usr/bin/env bash
# Tulpar орнату: ~/.local/bin/tulpar және ~/.local/lib/libtulpar_rt.a, libtulpar_kernel.a, libtulpar_win.a
set -e
cd "$(dirname "$0")"
cargo build --release
mkdir -p ~/.local/bin ~/.local/lib
cp target/release/tulpar ~/.local/bin/tulpar
cp target/release/libtulpar_rt.a ~/.local/lib/libtulpar_rt.a
cp target/release/libtulpar_kernel.a ~/.local/lib/libtulpar_kernel.a
cp target/release/libtulpar_win.a ~/.local/lib/libtulpar_win.a
echo "орнатылды: ~/.local/bin/tulpar  (PATH-қа ~/.local/bin қосылғанын тексеріңіз)"
