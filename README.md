# Inkplate

A screen-printing color separation tool built around a **continuous-tone
density-map engine**. Handles vector logos, cel-shaded illustration,
photoreal art, B&W halftone, pixel art, distressed, duotone, and mixed
media from a single unified pipeline.

> Each layer is a continuous-tone grayscale density map; halftoning happens
> at export time, so what you see in the preview is the clean mask, and what
> ships to the film is the rasterized dot.

## Features

- **9 extractors** — spot-solid, spot anti-aliased, color range, HSB
  brightness inverted (the correct underbase recipe), LAB lightness
  inverted, GCR black, channel calculation DSL, luminance threshold,
  index assignment
- **11 workflow presets** — spot, cel-shaded, sim-process light/dark,
  single halftone, black only, stencil, duotone, tritone, index FS,
  index Bayer
- **Auto-detect** picks the right workflow for the image
- **4 dither algorithms** — Floyd-Steinberg, Bayer, blue noise (true
  void-and-cluster), white noise
- **Halftone rasterizer** with rotated screens, 4 dot shapes, anti-aliased
  edges, per-layer LPI/angle/curve overrides
- **Background removal** via edge-seeded flood fill *or* alpha channel
  pass-through if the source PNG already has transparency
- **Project save/load** as `.inkplate` JSON files (versioned schema)
- **Film export** with embedded DPI metadata, registration marks,
  film borders + captions, and contact sheets
- **Headless engine** — the library crate (`inkplate`) is fully usable
  without the GUI for batch jobs and automation
- **Two CLI binaries** — `extract` (run one extractor) and `separate`
  (run a full workflow against an image)

## Design

```
SOURCE ─► EXTRACTOR ─► CURVES + LEVELS ─► MASK SHAPING ─► RENDER MODE
                                                                 │
                                              ┌──────────────────┴──────────┐
                                              ▼                             ▼
                                       _preview_mask                 _processed
                                       (smooth, composite)           (rasterized film)
```

Every workflow is built from the same nine extractors and four render
modes; what differs between workflows is which combinations and defaults
get applied, not the pipeline structure.

The density convention throughout is **0 = full ink, 255 = no ink**. A
foreground mask, when present, clamps every layer's output to no-ink
outside the art region — applied uniformly to underbase, color channels,
black plate, highlight white, and any future extractor type.

## Build

Requires Rust 1.75+. Install via [rustup](https://rustup.rs).

```sh
git clone https://github.com/adam/inkplate
cd inkplate
cargo run --release
```

For the headless CLIs:

```sh
cargo run --release --bin separate -- --image art.png --outdir films
cargo run --release --bin extract -- --image art.png --extractor color_range --target "#c43a2f" --out red.png
```

## Layout

```
src/
├── engine/        # headless image pipeline (testable without a GUI)
│   ├── color.rs           # sRGB <-> LAB, hex parsing, color naming
│   ├── palette.rs         # auto-palette, hue-family consolidation
│   ├── morphology.rs      # erode/dilate/open/close/smooth/feather
│   ├── tone.rs            # piecewise curve LUT, levels, density
│   ├── halftone.rs        # amplitude-modulated halftone dots
│   ├── dither/            # FS, Bayer, blue noise, white noise
│   ├── extractors/        # the 9 density-map builders
│   ├── workflows/         # the 11 presets + auto-detect
│   ├── foreground.rs      # background / foreground detection
│   ├── preprocess.rs      # desaturate, white/black bg variants
│   ├── layer.rs           # Layer struct + defaults
│   └── pipeline.rs        # process_layer orchestration
├── project/       # .inkplate save/load, schema versioning
├── export/        # PNG film output, registration marks, contact sheet
├── presets/       # garment / ink / Pantone approximation tables
├── bin/           # extract (one extractor), separate (full workflow)
└── gui/           # eframe/egui desktop app
```

The `engine` module is a library crate target (`inkplate`) so the full
pipeline is importable and testable without pulling in the GUI stack.

## Workflow tips

- **Pre-cut alpha in your source editor** for the most reliable
  background removal. Inkplate will auto-detect the alpha channel and
  use it as a foreground mask.
- For opaque sources, toggle **Remove background** in the top bar and
  tune the tolerance slider. The flood fill is edge-seeded, so it
  doesn't eat interior pixels that happen to match the background color.
- The **Composite preview** uses the smooth density masks; the **Export
  films** path re-runs the pipeline at the export DPI so halftone dots
  stay sharp instead of being upscaled.
- **Auto-detected workflow not great?** Override it from the dropdown.
  Sim-process workflows always halftone color channels; spot, cel-shaded,
  and index workflows render solid.

## Known issues

- No background processing worker — slider drags on very large images
  may stutter because every rerun happens on the UI thread.
- No curve editor widget yet. Tone curves come from workflow presets;
  the inspector shows the point count but isn't editable.
- No preview zoom/pan — fit-to-panel only.
- No undo/redo. Save your work as a `.inkplate` project before tweaking.
- No manual paint brush — the `Extractor::ManualPaint` variant is
  reserved for it.
- The PNG `pHYs` chunk (DPI metadata) is currently injected via a manual
  byte splice after the encoder. Plan is to switch to the `png` crate
  directly so the metadata is written cleanly.

All of these are tracked as `TODO(...)` markers in the source — `grep -r
TODO src/` to see what's open.

## License

[GNU General Public License v3.0 or later](LICENSE). Inkplate is free
software: you can redistribute it and/or modify it under the terms of
the GPL as published by the Free Software Foundation. See the LICENSE
file for the full text.
