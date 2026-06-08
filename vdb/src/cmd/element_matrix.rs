//! `vdb element-matrix` — cross-platform semantic drift aggregator.
//!
//! Walks `<root>/<platform>/<screen>/semantic.yaml`, runs pairwise drift across
//! every (platform-A, platform-B) pair per screen, and emits a screen × pair grid
//! with worst-severity per cell. Reuses `diff::diff_schemas` as the per-pair
//! primitive — this verb is the aggregation shell.
//!
//! Figma is treated as MIXED-MODE: any pair involving figma has its severity
//! dropped one notch (ERROR → WARN, WARN → INFO). Disable with `--strict-figma`.

use clap::Args;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use semantic_schema::SemanticSchema;

use super::diff::{diff_schemas, Severity};

const DEFAULT_CATALOGUE: &str = "/Users/Shared/projects/Outdoors/catalogue";

#[derive(Args)]
pub struct ElementMatrixArgs {
    /// Catalogue root containing <platform>/<screen>/semantic.yaml.
    /// Defaults to $TCTL_CATALOGUE_ROOT or /Users/Shared/projects/Outdoors/catalogue.
    #[arg(long)]
    pub root: Option<String>,

    /// Exit semantics: strict (any diag fails), error-only (errors fail),
    /// report-only (always exit 0). Default: report-only.
    #[arg(long, default_value = "report-only")]
    pub exit: ExitMode,

    /// Comma-separated screen filter (e.g. "discover,site-detail").
    #[arg(long)]
    pub screens: Option<String>,

    /// Comma-separated platform filter (e.g. "ios,android").
    #[arg(long)]
    pub platforms: Option<String>,

    /// Treat figma as a peer platform (no severity drop on figma pairs).
    #[arg(long)]
    pub strict_figma: bool,

    /// Emit JSON instead of human-readable grid.
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ExitMode {
    Strict,
    #[clap(name = "error-only")]
    ErrorOnly,
    #[clap(name = "report-only")]
    ReportOnly,
}

/// Outcome for a single (screen, platform-a, platform-b) cell.
#[derive(Debug, Clone)]
pub struct CellResult {
    pub screen: String,
    pub pair: (String, String),
    pub worst: Option<Severity>,
    pub matched: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub top_reason: Option<String>,
}

pub fn run(args: ElementMatrixArgs) -> Result<(), String> {
    let root = resolve_root(args.root.as_deref());
    let root_path = Path::new(&root);
    if !root_path.exists() {
        return Err(format!("catalogue root not found: {root}"));
    }

    let screen_filter: Option<BTreeSet<String>> = args
        .screens
        .as_deref()
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
    let platform_filter: Option<BTreeSet<String>> = args
        .platforms
        .as_deref()
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect());

    // Discover platforms and screens.
    let platforms = discover_platforms(root_path, platform_filter.as_ref())?;
    let screens = discover_screens(root_path, &platforms, screen_filter.as_ref())?;

    if platforms.is_empty() {
        return Err(format!("no platforms found under {root}"));
    }
    if screens.is_empty() {
        return Err(format!("no screens found under {root}"));
    }

    // Compute all cells.
    let cells = compute_matrix(root_path, &platforms, &screens, args.strict_figma);

    if args.json {
        emit_json(&platforms, &screens, &cells);
    } else {
        emit_grid(&platforms, &screens, &cells);
    }

    let exit_code = exit_code_for(&cells, args.exit);
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

pub fn resolve_root(arg: Option<&str>) -> String {
    if let Some(r) = arg {
        return r.to_string();
    }
    std::env::var("TCTL_CATALOGUE_ROOT").unwrap_or_else(|_| DEFAULT_CATALOGUE.to_string())
}

fn discover_platforms(
    root: &Path,
    filter: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(root).map_err(|e| format!("read root: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.starts_with('.') || name == "tests" || name == "docs" {
            continue;
        }
        if let Some(f) = filter {
            if !f.contains(&name) {
                continue;
            }
        }
        // Must contain at least one <name>/<screen>/semantic.yaml below.
        if has_any_semantic(&path) {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

fn has_any_semantic(platform_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(platform_dir) else {
        return false;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("semantic.yaml").is_file() {
            return true;
        }
    }
    false
}

fn discover_screens(
    root: &Path,
    platforms: &[String],
    filter: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for p in platforms {
        let pdir = root.join(p);
        let Ok(entries) = std::fs::read_dir(&pdir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            if !path.join("semantic.yaml").is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if let Some(f) = filter {
                if !f.contains(&name) {
                    continue;
                }
            }
            set.insert(name);
        }
    }
    Ok(set.into_iter().collect())
}

fn compute_matrix(
    root: &Path,
    platforms: &[String],
    screens: &[String],
    strict_figma: bool,
) -> Vec<CellResult> {
    // Eagerly load every (platform, screen) once.
    let mut cache: BTreeMap<(String, String), Option<SemanticSchema>> = BTreeMap::new();
    for screen in screens {
        for platform in platforms {
            let key = (platform.clone(), screen.clone());
            let path = root.join(platform).join(screen).join("semantic.yaml");
            cache.insert(key, load_schema(&path).ok());
        }
    }

    let mut cells = Vec::new();
    for screen in screens {
        for i in 0..platforms.len() {
            for j in (i + 1)..platforms.len() {
                let pa = &platforms[i];
                let pb = &platforms[j];
                let a = cache.get(&(pa.clone(), screen.clone())).and_then(|o| o.as_ref());
                let b = cache.get(&(pb.clone(), screen.clone())).and_then(|o| o.as_ref());
                let cell = compute_cell(screen, pa, pb, a, b, strict_figma);
                cells.push(cell);
            }
        }
    }
    cells
}

fn load_schema(path: &PathBuf) -> Result<SemanticSchema, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_yaml::from_str(&content).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn compute_cell(
    screen: &str,
    pa: &str,
    pb: &str,
    a: Option<&SemanticSchema>,
    b: Option<&SemanticSchema>,
    strict_figma: bool,
) -> CellResult {
    let pair = (pa.to_string(), pb.to_string());
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            // One or both sides missing → treat as ERROR-severity hole.
            return CellResult {
                screen: screen.to_string(),
                pair,
                worst: Some(Severity::Error),
                matched: 0,
                errors: 1,
                warnings: 0,
                infos: 0,
                top_reason: Some(format!(
                    "missing semantic.yaml for {}",
                    if a.is_none() { pa } else { pb }
                )),
            };
        }
    };

    let report = diff_schemas(a, b, None, false);
    let is_figma_pair = pa == "figma" || pb == "figma";
    let demote = is_figma_pair && !strict_figma;

    let (errors, warnings, infos) = if demote {
        // Drop everything one notch: errors → warnings, warnings → infos, infos → infos
        (
            Vec::<String>::new(),
            report.errors.clone(),
            [report.warnings.clone(), report.infos.clone()].concat(),
        )
    } else {
        (report.errors.clone(), report.warnings.clone(), report.infos.clone())
    };

    let worst = derive_worst(&errors, &warnings, &infos);
    let top_reason = pick_top_reason(&errors, &warnings, &infos);

    CellResult {
        screen: screen.to_string(),
        pair,
        worst,
        matched: report.matched,
        errors: errors.len(),
        warnings: warnings.len(),
        infos: infos.len(),
        top_reason,
    }
}

fn derive_worst(errors: &[String], warnings: &[String], infos: &[String]) -> Option<Severity> {
    if !errors.is_empty() {
        Some(Severity::Error)
    } else if warnings.iter().any(|w| !w.contains("within tolerance")) {
        Some(Severity::Warning)
    } else if !warnings.is_empty() || !infos.is_empty() {
        Some(Severity::Info)
    } else {
        None
    }
}

fn pick_top_reason(
    errors: &[String],
    warnings: &[String],
    infos: &[String],
) -> Option<String> {
    if let Some(e) = errors.first() {
        return Some(short_reason(e));
    }
    if let Some(w) = warnings.iter().find(|w| !w.contains("within tolerance")) {
        return Some(short_reason(w));
    }
    if let Some(w) = warnings.first() {
        return Some(short_reason(w));
    }
    infos.first().map(|i| short_reason(i))
}

/// Extract the diagnostic tag (e.g. "MISSING", "WRONG_TEXT") for compact display.
fn short_reason(msg: &str) -> String {
    msg.split(':').next().unwrap_or(msg).to_string()
}

fn cell_symbol(c: &CellResult) -> String {
    match c.worst {
        None => "PASS".to_string(),
        Some(Severity::Info) => format!("INFO({})", c.infos),
        Some(Severity::Warning) => match &c.top_reason {
            Some(r) => format!("WARN({r})"),
            None => format!("WARN({})", c.warnings),
        },
        Some(Severity::Error) => match &c.top_reason {
            Some(r) => format!("ERROR({r})"),
            None => format!("ERROR({})", c.errors),
        },
    }
}

fn emit_grid(platforms: &[String], screens: &[String], cells: &[CellResult]) {
    println!("element-matrix — {} screens × {} platforms", screens.len(), platforms.len());
    println!();
    // Group by screen.
    for screen in screens {
        let mut row: Vec<&CellResult> =
            cells.iter().filter(|c| c.screen == *screen).collect();
        row.sort_by(|a, b| a.pair.cmp(&b.pair));
        let mut line = format!("screen={:<20}", screen);
        for c in &row {
            line.push_str(&format!(
                "  {}-{}: {}",
                c.pair.0,
                c.pair.1,
                cell_symbol(c)
            ));
        }
        println!("{line}");
    }
    println!();
    let totals = summarize(cells);
    println!(
        "SUMMARY: {} pass, {} info, {} warn, {} error  ({} cells total)",
        totals.pass, totals.info, totals.warn, totals.error, cells.len()
    );
}

fn emit_json(platforms: &[String], screens: &[String], cells: &[CellResult]) {
    let cells_json: Vec<serde_json::Value> = cells
        .iter()
        .map(|c| {
            serde_json::json!({
                "screen": c.screen,
                "pair": [c.pair.0, c.pair.1],
                "worst": match c.worst {
                    None => "pass",
                    Some(Severity::Info) => "info",
                    Some(Severity::Warning) => "warn",
                    Some(Severity::Error) => "error",
                },
                "matched": c.matched,
                "errors": c.errors,
                "warnings": c.warnings,
                "infos": c.infos,
                "top_reason": c.top_reason,
            })
        })
        .collect();
    let totals = summarize(cells);
    let report = serde_json::json!({
        "platforms": platforms,
        "screens": screens,
        "cells": cells_json,
        "summary": {
            "pass": totals.pass,
            "info": totals.info,
            "warn": totals.warn,
            "error": totals.error,
            "total": cells.len(),
        }
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
}

#[derive(Default)]
struct Totals {
    pass: usize,
    info: usize,
    warn: usize,
    error: usize,
}

fn summarize(cells: &[CellResult]) -> Totals {
    let mut t = Totals::default();
    for c in cells {
        match c.worst {
            None => t.pass += 1,
            Some(Severity::Info) => t.info += 1,
            Some(Severity::Warning) => t.warn += 1,
            Some(Severity::Error) => t.error += 1,
        }
    }
    t
}

pub fn exit_code_for(cells: &[CellResult], mode: ExitMode) -> i32 {
    let totals = summarize(cells);
    match mode {
        ExitMode::ReportOnly => 0,
        ExitMode::ErrorOnly => if totals.error > 0 { 1 } else { 0 },
        ExitMode::Strict => {
            if totals.error > 0 || totals.warn > 0 || totals.info > 0 { 1 } else { 0 }
        }
    }
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_yaml(dir: &Path, platform: &str, screen: &str, body: &str) {
        let d = dir.join(platform).join(screen);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("semantic.yaml"), body).unwrap();
    }

    fn ios_min(content: &str) -> String {
        format!(
            "screen: test\ndevice: iPhone\nplatform: ios\ntimestamp: 2026-01-01T00:00:00Z\nviewport:\n  width: 400\n  height: 800\n  density: 3.0\nelements:\n- id: hello_label\n  type: text\n  content: {content}\n  bounds:\n    x: 10\n    y: 10\n    w: 100\n    h: 20\n  clickable: false\n  accessible: true\n"
        )
    }

    fn android_min(content: &str) -> String {
        format!(
            "screen: test\ndevice: Pixel\nplatform: android\ntimestamp: 2026-01-01T00:00:00Z\nviewport:\n  width: 400\n  height: 800\n  density: 2.625\nelements:\n- id: hello_label\n  type: text\n  content: {content}\n  bounds:\n    x: 10\n    y: 10\n    w: 100\n    h: 20\n  clickable: false\n  accessible: true\n"
        )
    }

    #[test]
    fn empty_root_returns_error() {
        let tmp = tempdir();
        // No platform dirs at all.
        let args = ElementMatrixArgs {
            root: Some(tmp.to_string_lossy().to_string()),
            exit: ExitMode::ReportOnly,
            screens: None,
            platforms: None,
            strict_figma: false,
            json: false,
        };
        // discover_platforms returns empty → run returns Err.
        let result = run(args);
        assert!(result.is_err(), "expected error on empty root, got {:?}", result);
    }

    #[test]
    fn single_screen_all_pass() {
        let tmp = tempdir();
        write_yaml(&tmp, "ios", "discover", &ios_min("Hello"));
        write_yaml(&tmp, "android", "discover", &android_min("Hello"));
        let platforms = vec!["android".to_string(), "ios".to_string()];
        let screens = vec!["discover".to_string()];
        let cells = compute_matrix(&tmp, &platforms, &screens, false);
        assert_eq!(cells.len(), 1, "expected 1 pair-cell");
        let c = &cells[0];
        assert_eq!(c.errors, 0);
        assert_eq!(c.warnings, 0);
        assert_eq!(c.worst, None, "expected PASS, got {:?} top={:?}", c.worst, c.top_reason);
    }

    #[test]
    fn mixed_severity_aggregation_and_figma_demotion() {
        let tmp = tempdir();
        // ios vs android: different text -> ERROR (WRONG_TEXT)
        write_yaml(&tmp, "ios", "discover", &ios_min("Hello"));
        write_yaml(&tmp, "android", "discover", &android_min("World"));
        // figma vs both: same id+content as ios -> would PASS, but figma is mixed-mode
        write_yaml(
            &tmp,
            "figma",
            "discover",
            &ios_min("Hello").replace("platform: ios", "platform: figma"),
        );

        let platforms = vec!["android".to_string(), "figma".to_string(), "ios".to_string()];
        let screens = vec!["discover".to_string()];
        let cells = compute_matrix(&tmp, &platforms, &screens, false);
        assert_eq!(cells.len(), 3, "expected 3 pairs for 3 platforms");

        // android-ios: WRONG_TEXT → real error
        let ai = cells.iter().find(|c| c.pair == ("android".into(), "ios".into())).unwrap();
        assert_eq!(ai.worst, Some(Severity::Error), "android-ios should be ERROR");

        // android-figma: WRONG_TEXT but demoted to WARN
        let af = cells.iter().find(|c| c.pair == ("android".into(), "figma".into())).unwrap();
        assert!(
            matches!(af.worst, Some(Severity::Warning)),
            "android-figma should be demoted to WARN, got {:?}",
            af.worst
        );

        // figma-ios: identical content → PASS (or INFO after demotion of any EXTRAs)
        let fi = cells.iter().find(|c| c.pair == ("figma".into(), "ios".into())).unwrap();
        assert!(
            fi.worst.is_none() || fi.worst == Some(Severity::Info),
            "figma-ios should be PASS/INFO, got {:?}",
            fi.worst
        );

        // Exit semantics on mixed grid.
        assert_eq!(exit_code_for(&cells, ExitMode::ReportOnly), 0);
        assert_eq!(exit_code_for(&cells, ExitMode::ErrorOnly), 1);
        assert_eq!(exit_code_for(&cells, ExitMode::Strict), 1);
    }

    /// Tiny in-process tempdir helper (no extra crate dep). Uses an atomic
    /// counter plus nanos to keep parallel test runs from colliding.
    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        p.push(format!("vdb-emtest-{nanos}-{seq}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
