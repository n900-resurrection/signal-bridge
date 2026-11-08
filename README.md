# signal-bridge

Signal for the Nokia N900 (Maemo 5): a small daemon built on [presage](https://github.com/whisperfish/presage) (the library behind gurk-rs) that serves a local
unix socket with a simple line protocol. A libpurple plugin (`libsignal-purple`, in the N900 project) talks to it, so that Signal appears as an ordinary account
in the stock Maemo messaging UI. Licence: AGPL-3.0 (presage is AGPL).

```
signal-bridge daemon                 # receive messages and serve ~/.local/share/signal-bridge/bridge.sock
signal-bridge contacts | groups | whoami
signal-bridge messages THREAD [--since MS]   # stored messages of a contact UUID or a group key
```

It opens the presage store of an already linked device (default: the store of gurk-rs, `~/.local/share/gurk/signal.sqlite`, passphrase from
`~/.config/gurk/gurk.toml`; override with `--store` and `--gurk-config`). **Do not run it together with gurk** (one connection per device). With
`--allow UUID,...` only these recipients can be written to (a guard for tests). The protocol is described in the header of `src/main.rs`; `sb-client.py` is a tiny test client.

Build (in the N900 project, Rust toolchain and musl cross gcc of `apps/gurk`): `make -C apps/signal-bridge build` produces a static ARMv7 musl binary.
