#!/usr/bin/env python3
"""Make theme variants of the hand-authored gingham, preserving its geometry."""
from pathlib import Path

assets = Path(__file__).resolve().parent.parent
original = (assets / "gingham.svg").read_text()
source_colors = ("#ffd7e7", "#fff0f5", "#c96390", "#fffef7", "#e6ae75", "#fffdf4")
palettes = {
    "sepia": ("#ddc8a6", "#e9d9bc", "#997650", "#f0e2c7", "#ae8657", "#ead7b6"),
    "dark": ("#211a27", "#2a2130", "#89607c", "#5d455f", "#a87e9a", "#6c526d"),
}
for name, colors in palettes.items():
    svg = original
    for source, color in zip(source_colors, colors, strict=True):
        svg = svg.replace(source, color)
    (assets / f"gingham-{name}.svg").write_text(svg)
