TEST_NAME=signal-bridge
need_pkg signal-bridge
ok binary test -x /opt/signal-bridge/bin/signal-bridge
ok starter test -x /opt/signal-bridge/bin/signal-bridge-start
ok in-path sh -c 'command -v signal-bridge && command -v signal-bridge-start'
contains version "signal-bridge" signal-bridge --version
contains help "daemon" sh -c 'signal-bridge --help 2>&1'
# nothing of the Signal account is touched: only --help and --version run here
summary
