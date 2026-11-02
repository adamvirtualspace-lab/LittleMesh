# xatlas (vendored)

- Upstream: https://github.com/jpcy/xatlas
- Commit: `f700c7790aaa030e794b52ba7791a05c085faf0c` (2022-07-25)
- Files: `source/xatlas/xatlas.cpp`, `source/xatlas/xatlas.h`, `LICENSE` (MIT)

## Local patches

Each change is marked `LittleMesh patch` in the source.

1. **`PackOptions::keepChartAspect`** (default `false`, so stock behaviour is unchanged).
   After scaling a chart uniformly, `PackCharts` stretches it separately along x and y
   so its extents land on whole texels. That is under 0.5% on large charts but up to
   about 33% on charts a few texels wide, which breaks exact island shapes and
   relative texel density. With the option on, that per-axis stretch (and block
   alignment) is skipped. LittleMesh turns it on when packing its own charts.
