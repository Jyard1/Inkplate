# TODO

Consolidated list of open work, mirrored from `TODO(...)` markers in the
source. Run `grep -rn 'TODO(' src/` to see them in context.

The tags in parentheses are the landing they were originally scheduled
for; many have slipped past their landing because the work was
deprioritised, not because it was lost. Treat the tags as "this is the
phase it conceptually belongs to," not as a deadline.

## GUI / UX

- **Background processing worker** — slider drags currently rerun
  `process_layer` on the UI thread, which stutters on large images.
  Should move to a rayon-backed worker that publishes results back to
  the UI via a channel. (`src/gui/mod.rs`)
- **Curve editor widget** — the inspector shows tone curves as "N
  points" but doesn't let you drag them. Engine already supports
  arbitrary piecewise curves (`engine::tone::CurvePoint`); just needs
  a custom egui widget on a small canvas. (`src/gui/panels/inspector.rs`)
- **Preview zoom/pan** — the center panel is fit-to-panel only. Add
  scroll-wheel zoom + middle-drag pan, plus 100% / fit hotkeys.
  (`src/gui/panels/preview.rs`)
- **Undo/redo** — `GuiState` has no history stack. Snapshot the layer
  list before each mutation, cap at 64 entries, bind Ctrl+Z/Y.
- **Menu + hotkeys** — `Ctrl+O` / `Ctrl+S` / `Ctrl+E` / etc. None of
  the standard shortcuts are wired yet.
- **Manual paint brush** — `Extractor::ManualPaint` exists in the model
  but there's no UI for painting on/off pixels.

## Engine

- **Index-assignment dither caching** — running N layers against the
  same palette currently re-dithers the whole image N times. Cache the
  assignment array keyed on `(image-hash, palette-hash, dither-kind)`
  with a small LRU. (`src/engine/extractors/index_assignment.rs`)
- **Morphology rolling histogram** — the separable max/min filters
  are O(w · h · r) per pixel. Fine for the radii we use today (≤ 8 px),
  but a rolling histogram would make them O(1) in `r` for the day we
  need bigger blurs. (`src/engine/morphology.rs`)

## Export

- **PNG `pHYs` chunk** — currently injected by manually splicing bytes
  into the encoded PNG output, after the IHDR. Works but is fragile.
  Switch to the `png` crate directly so the chunk is written cleanly
  by the encoder. (`src/export/film.rs`)
- **Bundled fallback font** — film border captions try Segoe UI →
  Arial → DejaVu Sans from system paths and skip the caption if none
  is found. Bundle a small open font (Inter, DejaVu, etc.) so captions
  always render regardless of platform. (`src/export/border.rs`)
- **Contact sheet captions** — the grid currently shows just the
  thumbnails; add the layer number / name / ink hex under each cell
  using the same font helper. (`src/export/contact_sheet.rs`)
- **Foreground mask resampling on export resize** — when
  `width_inches` triggers a source resample, the cached foreground
  mask is at the wrong size and gets dropped. Resize it with
  nearest-neighbor (to keep edges crisp) before passing into the
  pipeline. (`src/export/film.rs`)

## Presets / libraries

- **GARMENT_PRESETS** — table of common shirt colors with display
  names. (`src/presets/mod.rs`)
- **INK_PRESETS** — common plastisol colors with opacity hints, used
  by the underbase boost-under-darks decision.
- **PANTONE_APPROX** — small subset (~200 common Pantone solid coated
  entries) plus a `nearest_pantone(Rgb) -> (name, Rgb)` lookup. Not
  color-managed; document that in the GUI tooltip when surfaced.

## Project / I/O

- **Schema migration steps** — `project::migrate` is a stub. Add
  branches keyed on `version` if/when `CURRENT_VERSION` bumps so
  older files load forward. (`src/project/mod.rs`)
