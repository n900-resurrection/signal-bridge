# signal-bridge: Signal (presage) behind a local socket, for the native messaging integration (DOC/signal-native.md). Targets and conventions: ../../mk/app.mk
override APP     := signal-bridge
VERSION ?= 0.1.0
REV     ?= 1maemo1
DEPS    :=
include ../../mk/app.mk
