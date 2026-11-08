#!/bin/bash
# Cross-build signal-bridge (Rust, static musl) for Maemo 5. Run INSIDE the Workshop (make -C apps/signal-bridge build).
# Needs the Rust toolchain and the musl cross gcc of apps/gurk (make -C apps/gurk build once). The crates are compiled
# into the target directory of gurk, so the first build reuses everything gurk already built.
# Output: build/out/signal-bridge-$VERSION/signal-bridge
set -euo pipefail
ROOT=/project
D="$ROOT/build/rust"
VERSION="${VERSION:-0.1.0}"
OUT="$ROOT/build/out/signal-bridge-$VERSION"
export RUSTUP_HOME="$D/rustup" CARGO_HOME="$D/cargo" PATH="$D/cargo/bin:$PATH"
[ -x "$D/cargo/bin/cargo" ] || { echo "Missing Rust, run: make -C apps/rust build"; exit 1; }
T="$D/musl-cross"
[ -x "$T/bin/arm-linux-musleabi-gcc" ] || { echo "Missing the musl cross gcc, run: make -C apps/gurk build"; exit 1; }
mkdir -p "$OUT"
cd "$ROOT/apps/signal-bridge"
# the same dependency versions as gurk (Cargo.lock of the gurk checkout; cargo adds what is missing)
[ -f Cargo.lock ] || cp "$D/gurk-rs/Cargo.lock" Cargo.lock
TRIPLE=armv7-unknown-linux-musleabi
export CARGO_TARGET_DIR="$D/target-gurk"
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABI_LINKER="$T/bin/arm-linux-musleabi-gcc" \
       CC_armv7_unknown_linux_musleabi="$T/bin/arm-linux-musleabi-gcc" \
       CXX_armv7_unknown_linux_musleabi="$T/bin/arm-linux-musleabi-g++" \
       AR_armv7_unknown_linux_musleabi="$T/bin/arm-linux-musleabi-ar" \
       RUSTFLAGS="-C target-cpu=cortex-a8"
cargo build --release --target "$TRIPLE" 2>&1 | tee "$OUT/build.log" | grep -E "^(error|warning: unused)|^ +--> |Compiling signal-bridge|Finished|could not" | head -60
test "${PIPESTATUS[0]}" = 0 || { tail -40 "$OUT/build.log"; exit 1; }
cp "$D/target-gurk/$TRIPLE/release/signal-bridge" "$OUT/signal-bridge"
arm-linux-gnueabi-strip "$OUT/signal-bridge" 2>/dev/null || true
file "$OUT/signal-bridge" | cut -c1-140
ls -l "$OUT/signal-bridge"
