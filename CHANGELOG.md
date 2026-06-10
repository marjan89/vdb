# Changelog

All notable changes to vdb are documented here.

## [Unreleased]

### Verified (Wave 7/8 of Epic E Phase 4, 2026-06-10)
- **Figma severity demotion** is intentional (`vdb/src/cmd/element_matrix.rs:269-282`): when one side of a pair is `figma` and `--strict-figma` is not set, severity is demoted one notch (Error→Warn, Warn→Info). Per ADR-012 D4 (figma-as-reference design): elements present in code but absent from design are looser drift than the reverse. `--strict-figma` lifts the clamp for audit-lane runs. Surfaced via the Wave-7 adversarial gate test in `tctl/docs/epic-e-gate-test-report.md`; closed-as-designed in `tctl/docs/tech-debt.md` TD-120.
- **Content-text drift** IS compared on matched element pairs (`vdb/src/cmd/diff.rs:394-409`): a `content` change between matched src/tgt yields a `WRONG_TEXT` Error diagnostic. Drift signal only fires for matched elements; unmatched elements yield only MISSING. Wave-7 TEST 2's apparent "content insensitivity" was an unmatched-element artifact. Closed-as-designed in TD-121.

(No code changes shipped in vdb during Wave-7/8 — verification only; behavior already correct.)

## [v0.2.0] — 2026-06-08

### Added
- **TD-62 element-matrix verb** (d59bc70): `vdb element-matrix` — cross-platform semantic drift aggregator. Walks `<catalogue-root>/<platform>/<screen>/semantic.yaml`, computes per-cell (matched / errors / warnings / infos), emits grid output (human-readable or `--json`). `--exit` tri-state (`strict` / `error-only` / `report-only`, default `report-only`) drives CI gates without per-consumer wrappers. See ADR-010.
- **Diff core refactor** (ea35129): `diff_schemas` / `Severity` / `Diagnostic` exposed as public API surface for reuse by `element-matrix` and future drift consumers (CI dashboards, audit tooling).
