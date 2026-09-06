#!/usr/bin/env bash
set -euo pipefail
# Stages a portable local directory. Never installs or changes desktop settings.
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
cargo build --release --locked -p ruby-reader
reader_stage="artifacts/package/ruby-reader"
mkdir -p "$reader_stage/share/applications" "$reader_stage/share/icons/hicolor/128x128/apps" "$reader_stage/share/doc/ruby-reader"
install -m 0755 target/release/ruby-reader "$reader_stage/ruby-reader"
install -m 0644 packaging/ruby-reader.desktop "$reader_stage/share/applications/ruby-reader.desktop"
install -m 0644 assets/icon.png "$reader_stage/share/icons/hicolor/128x128/apps/ruby-reader.png"
install -m 0644 README.md assets/fonts/LICENSE.txt "$reader_stage/share/doc/ruby-reader/"
printf 'Staged Ruby Reader at %s\n' "$PWD/$reader_stage/ruby-reader"
