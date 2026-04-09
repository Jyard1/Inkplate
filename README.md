**THIS IS AI SLOP(claude code)**

# Inkplate

> # ⚠ **VIBE CODED WITH AI**
>
> **This codebase was generated end-to-end through AI assistance ("vibe
> coding"). Read the source, run the tests, and use your own judgement
> before trusting it for production separations — the author doesn't
> claim line-by-line authorship of this code.**

A screen-printing color separation tool built around a **continuous-tone
density-map engine**. Handles vector logos, cel-shaded illustration,
photoreal art, B&W halftone, pixel art, distressed, duotone, and mixed
media from a single unified pipeline.

> Each layer is a continuous-tone grayscale density map; halftoning happens
> at export time, so what you see in the preview is the clean mask, and what
> ships to the film is the rasterized dot.

## Features

### Engine

- **9 extractors** — spot-solid, spot anti-aliased (hard Voronoi with
  CIE94 distance), color range, HSB brightness inverted (the correct
  underbase recipe), LAB lightness inverted, GCR black, channel
  calculation DSL, luminance threshold, index assignment
- **11 workflow presets** — spot, cel-shaded, sim-process light/dark,
  single halftone, black only, stencil, duotone, tritone, index FS,
  index Bayer
- **Auto-detect** picks the right workflow for the image
- **LAB-space k-means palette** with CIE94 ΔE clustering. Preserves
  small saturated accents (yellow torch flames, red buttons) that
  naive RGB median-cut would merge away. `snap_extremes` forces
  near-black and near-white palette entries to pure `#000` / `#FFF`
  so plate swatches match the actual inks.
- **4 dither algorithms** — Floyd-Steinberg, Bayer, blue noise (true
  void-and-cluster), white noise
- **Halftone rasterizer** with rotated screens, 4 dot shapes,
  anti-aliased edges, per-layer LPI/angle/curve overrides
- **Background removal** via edge-seeded flood fill *or* alpha channel
  pass-through if the source PNG already has transparency

### GUI

- **Background processing worker** — `process_layer` runs on a
  dedicated thread, so slider drags stay snappy on multi-megapixel
  sources. Jobs coalesce (the latest one wins), and a generation
  counter drops results that got stale from layer reorders or
  workflow reruns.
- **Preview zoom & pan** — mouse-wheel zoom around the cursor,
  middle-drag pan, `F` to refit, `1` for 100%. Nearest-neighbour
  filtering so zoomed-in pixels stay crisp.
- **Undo / redo** — `Ctrl+Z` / `Ctrl+Y`, 64-entry history stack.
  Slider drags coalesce to one undo step; layer reorders, deletions,
  workflow reruns, brush strokes, and merge ops all snapshot.
- **Curve editor widget** — drag control points, double-click to
  add, right-click to delete. Endpoints stay at `x=0` and `x=255`;
  non-endpoint points can't leapfrog their neighbours.
- **Manual paint brush** — `Extractor::ManualPaint` layers have a
  per-layer stroke buffer; Primary-drag on the preview paints ink,
  toggleable to erase. The buffer serializes into `.inkplate`
  projects as raw bytes.
- **Ink color picker** — visible swatch + hex text input in the
  Identity section, so you can type a Pantone hex directly.
- **Multi-select + merge ops** — `Ctrl+click` layer names to build
  a blue-outlined multi-selection, then:
  - **Merge → shadow HT**: collapses the selected layers onto the
    lightest-ink plate and emits a new black halftone shadow plate
    whose density encodes the darker shades from a single extra ink.
  - **Merge same ink**: unions the masks of several same-ink plates
    (e.g. multiple black shadow plates) into one screen.
- **Per-plate Reach slider** on `spot_aa` layers — biases the CIE94
  distance for this plate so you can pull pixels onto it (or push
  them away) when the automatic Voronoi doesn't land right.
  Weights propagate across every layer's copy of the targets list
  so there's no double-coverage between plates.

### I/O and export

- **Project save/load** as `.inkplate` JSON files (versioned schema).
- **Film export** with embedded DPI metadata (pHYs chunk written
  directly through the `png` crate encoder), registration marks,
  film borders + captions, and contact sheets.
- **Headless engine** — the library crate (`inkplate`) is fully
  usable without the GUI for batch jobs and automation.
- **Two CLI binaries** — `extract` (run one extractor) and
  `separate` (run a full workflow against an image).

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
git clone https://github.com/Jyard1/Inkplate
cd Inkplate
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
- **Colorful illustration** — set the Max palette colors slider
  high (16–24 for multi-hue art) and leave "Merge same hue"
  unchecked by default. If the automatic Voronoi misassigns a
  shaded region, select that plate, scroll the inspector to
  Extractor, and nudge the **Reach** slider until it claims what
  it should.
- **Reducing plate count on the press** — after running the spot
  workflow, `Ctrl+click` several same-hue layers (all reds, or all
  teals) and click **Merge → shadow HT** to collapse them into one
  bright-color plate plus one black halftone shadow plate. Repeat
  for each hue family. Then `Ctrl+click` the resulting black
  shadow plates and click **Merge same ink** to put them all on a
  single screen. Classic two-ink shading trick.
- **Exact Pantone matches** — paste the hex value into the Ink
  field in the Inspector → Identity section. It updates the plate
  immediately and triggers a reprocess.

## License

[GNU General Public License v3.0 or later](LICENSE). Inkplate is free
software: you can redistribute it and/or modify it under the terms of
the GPL as published by the Free Software Foundation. See the LICENSE
file for the full text.
