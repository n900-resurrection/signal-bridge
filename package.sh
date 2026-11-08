#!/bin/bash
# Package signal-bridge as a .deb for Maemo 5. Run INSIDE the Workshop after build.sh: make -C apps/signal-bridge deb
# Output: /project/build/deb/signal-bridge_<VERSION>-<REV>_armel.deb
set -euo pipefail
PROJ=/project
. "$PROJ/scripts/debpack.sh"

PKG=signal-bridge
VERSION="${VERSION:-0.1.0}"
OUT="$PROJ/build/out/$PKG-$VERSION"
STAGE="$PROJ/build/deb/stage-$PKG"
HERE="$PROJ/apps/signal-bridge"
[ -x "$OUT/signal-bridge" ] || { echo "Missing build output in $OUT, run build.sh first (make build)"; exit 1; }

rm -rf "$STAGE"
mkdir -p "$STAGE/opt/signal-bridge/bin" "$STAGE/opt/signal-bridge/share/doc/signal-bridge" "$STAGE/usr/bin"
install -m 755 "$OUT/signal-bridge" "$STAGE/opt/signal-bridge/bin/signal-bridge"
install -m 755 "$HERE/signal-bridge-start" "$STAGE/opt/signal-bridge/bin/signal-bridge-start"
install -m 755 "$HERE/sb-client.py" "$STAGE/opt/signal-bridge/bin/sb-client"
install -m 644 "$HERE/LICENSE" "$STAGE/opt/signal-bridge/share/doc/signal-bridge/LICENSE"
install -m 644 "$HERE/README.md" "$STAGE/opt/signal-bridge/share/doc/signal-bridge/README.md"
ln -s ../../opt/signal-bridge/bin/signal-bridge "$STAGE/usr/bin/signal-bridge"
ln -s ../../opt/signal-bridge/bin/signal-bridge-start "$STAGE/usr/bin/signal-bridge-start"

SECTION=user/network
DEPENDS="libc6 (>= 2.5)"
DISPLAY_NAME="signal-bridge"
SHORT_DESC="Signal daemon (presage) with a local socket, for the messaging UI plugin"
LONG_DESC="signal-bridge $VERSION: a daemon built on presage that serves a local unix socket (~/.local/share/signal-bridge/bridge.sock) with a simple line protocol. The package libsignal-purple uses it to show Signal in the Maemo messaging UI. It opens the store of an already linked device (the one of gurk: link the device with gurk first); do not run gurk and the daemon at the same time. Start by hand: signal-bridge-start (log: ~/.local/share/signal-bridge/daemon.log); commands: signal-bridge contacts, groups, whoami, messages.
Static ARMv7 musl binary. Upstream presage: https://github.com/whisperfish/presage (AGPL-3.0)."
MAINTAINER="elPytel"
debpack_build
