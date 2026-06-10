# Changelog

All notable changes to vdb are documented here.

## [v0.2.0] — 2026-06-08

### Added
- **TD-62 element-matrix verb** (d59bc70): `vdb element-matrix` — cross-platform semantic drift aggregator. Walks `<catalogue-root>/<platform>/<screen>/semantic.yaml`, computes per-cell (matched / errors / warnings / infos), emits grid output (human-readable or `--json`). `--exit` tri-state (`strict` / `error-only` / `report-only`, default `report-only`) drives CI gates without per-consumer wrappers. See ADR-010.
- **Diff core refactor** (ea35129): `diff_schemas` / `Severity` / `Diagnostic` exposed as public API surface for reuse by `element-matrix` and future drift consumers (CI dashboards, audit tooling).
