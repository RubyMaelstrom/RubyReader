#!/usr/bin/env bash
# Run after building packaging/wayland-window-probe.c (see packaging/README.md).
# Only a new isolated smoke library and its uniquely titled window are touched.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
renderer="${1:-cpu}"
probe="$PWD/artifacts/wayland-probe/probe"
test -x "$probe"
run_dir="$(mktemp -d "$PWD/artifacts/tray-${renderer}-XXXXXX")"
RUBY_READER_DECORA_SEED=42 ./target/release/ruby-reader --data-dir "$run_dir" --sample --smoke --renderer "$renderer" >"$run_dir/run.log" 2>&1 &
reader_pid=$!
title="Ruby Reader test $reader_pid"
tray="org.kde.StatusNotifierItem-$reader_pid-1"
visible() { "$probe" | rg -Fxq "$title"; }
for attempt in {1..150}; do
    if visible; then break; fi
    kill -0 "$reader_pid"
    sleep 0.2
done
visible
printf 'Compositor: test window mapped (%s)\n' "$renderer"
# The native smoke closes its actual native window at step six.
for attempt in {1..150}; do
    if ! visible; then break; fi
    kill -0 "$reader_pid"
    sleep 0.2
done
if visible; then printf 'FAIL: close left the toplevel mapped\n'; exit 1; fi
printf 'Compositor: close removed the toplevel\n'
for attempt in {1..150}; do
    if visible; then break; fi
    kill -0 "$reader_pid"
    sleep 0.1
done
visible
printf 'Compositor: restored window mapped\n'
# Also exercise a real compositor close request, then the real tray activation.
"$probe" --close "$title"
for attempt in {1..50}; do
    if ! visible; then break; fi
    sleep 0.1
done
if visible; then printf 'FAIL: compositor close left the toplevel mapped\n'; exit 1; fi
gdbus call --session --dest "$tray" --object-path /StatusNotifierItem --method org.kde.StatusNotifierItem.Activate 0 0 >/dev/null
for attempt in {1..100}; do
    if visible; then break; fi
    sleep 0.1
done
visible
printf 'Compositor: real close request and tray click passed\n'
wait "$reader_pid"
rg 'Native smoke passed' "$run_dir/run.log"
if visible; then printf 'FAIL: smoke quit left a window\n'; exit 1; fi
printf 'Evidence: %s\n' "$run_dir"
