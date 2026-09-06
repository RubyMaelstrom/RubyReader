# Packaging and native checks

`bash packaging/stage.sh` builds the optimized, locked release and stages it
under `artifacts/package/ruby-reader`. It never installs or changes desktop
settings. All runtime art and fonts are embedded in the executable.

## Wayland close-to-tray check

The optional compositor probe requires a compositor exposing wlr foreign
toplevel management, `wayland-scanner`, a C compiler, libwayland-client, and the
`wlr-foreign-toplevel-management-unstable-v1.xml` protocol definition. That XML
is also bundled in Cargo's downloaded `wayland-protocols-wlr` source package.
Set `reader_protocol` to its absolute path, then run:

```bash
mkdir -p artifacts/wayland-probe
wayland-scanner client-header "$reader_protocol" artifacts/wayland-probe/wlr-toplevel.h
wayland-scanner private-code "$reader_protocol" artifacts/wayland-probe/wlr-toplevel.c
cc -Wall -Wextra -Werror -Iartifacts/wayland-probe packaging/wayland-window-probe.c artifacts/wayland-probe/wlr-toplevel.c -lwayland-client -o artifacts/wayland-probe/probe
cargo build --release --locked -p ruby-reader
bash packaging/check-tray.sh cpu
bash packaging/check-tray.sh hybrid
```

The scripts create isolated sample libraries, inspect actual window removal,
send a compositor close request only to the uniquely titled test window, and
activate the real tray icon. Screenshots and logs stay in ignored `artifacts/`
directories. The normal personal library is never used.
