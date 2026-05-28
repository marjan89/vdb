use clap::Args;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Args)]
pub struct MatrixArgs {
    /// Results directory containing run log YAMLs
    #[arg(long, default_value = "/Users/Shared/projects/Outdoors/catalogue/tests/results")]
    pub results: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

struct RunResult {
    tc_id: String,
    tc_name: String,
    platform: String,
    device: String,
    result: String,
    failure_step: Option<usize>,
    failure_reason: Option<String>,
    timestamp: String,
}

pub fn run(args: MatrixArgs) -> Result<(), String> {
    let dir = Path::new(&args.results);
    if !dir.exists() {
        return Err(format!("results dir not found: {}", args.results));
    }

    let mut results: Vec<RunResult> = Vec::new();

    for entry in std::fs::read_dir(dir).map_err(|e| format!("read dir: {e}"))? {
        let entry = entry.map_err(|e| format!("entry: {e}"))?;
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "yaml") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(r) = parse_run_result(&content, path.to_str().unwrap_or("")) {
                results.push(r);
            }
        }
    }

    // Group by TC × platform, keep latest per group
    let mut latest: BTreeMap<(String, String), RunResult> = BTreeMap::new();
    for r in results {
        let key = (r.tc_id.clone(), r.platform.clone());
        let is_newer = latest.get(&key).map_or(true, |prev| r.timestamp > prev.timestamp);
        if is_newer {
            latest.insert(key, r);
        }
    }

    let all_tcs: BTreeSet<String> = latest.keys().map(|(tc, _)| tc.clone()).collect();
    let all_platforms: BTreeSet<String> = latest.keys().map(|(_, p)| p.clone()).collect();

    if args.json {
        let entries: Vec<serde_json::Value> = latest.values().map(|r| {
            serde_json::json!({
                "tc_id": r.tc_id,
                "tc_name": r.tc_name,
                "platform": r.platform,
                "device": r.device,
                "result": r.result,
                "failure_step": r.failure_step,
                "failure_reason": r.failure_reason,
                "timestamp": r.timestamp,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&entries).unwrap_or_default());
        return Ok(());
    }

    // Print matrix table
    let platforms: Vec<&String> = all_platforms.iter().collect();
    print!("{:<12}", "TC");
    for p in &platforms {
        print!("  {:<10}", p);
    }
    println!();
    print!("{:<12}", "──────────");
    for _ in &platforms {
        print!("  {:<10}", "──────────");
    }
    println!();

    let mut total_pass = 0;
    let mut total_fail = 0;
    let mut total_missing = 0;

    for tc in &all_tcs {
        print!("{:<12}", tc);
        for p in &platforms {
            let key = (tc.clone(), p.to_string());
            if let Some(r) = latest.get(&key) {
                let symbol = if r.result == "PASS" { "✓ PASS" } else { "✗ FAIL" };
                print!("  {:<10}", symbol);
                if r.result == "PASS" { total_pass += 1; } else { total_fail += 1; }
            } else {
                print!("  {:<10}", "—");
                total_missing += 1;
            }
        }
        println!();
    }

    println!();
    println!("SUMMARY: {} pass, {} fail, {} missing", total_pass, total_fail, total_missing);
    println!();

    // Print failure details
    let failures: Vec<&RunResult> = latest.values().filter(|r| r.result != "PASS").collect();
    if !failures.is_empty() {
        println!("FAILURES:");
        for r in &failures {
            let step = r.failure_step.map(|s| format!("step {s}")).unwrap_or_default();
            let reason = r.failure_reason.as_deref().unwrap_or("unknown");
            println!("  {} ({}) — {} {}", r.tc_id, r.platform, step, reason);
        }
    }

    if total_fail > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn parse_run_result(content: &str, filename: &str) -> Option<RunResult> {
    // Handle both Android format (tc_id:) and iOS format (id:)
    let tc_id = extract_field(content, "tc_id:")
        .or_else(|| extract_field(content, "id:"))
        .unwrap_or_default();
    let tc_name = extract_field(content, "tc_name:")
        .or_else(|| extract_field(content, "name:"))
        .unwrap_or_default();
    let platform = extract_field(content, "platform:")
        .or_else(|| {
            if filename.contains("-android-") { Some("android".into()) }
            else if filename.contains("-ios-") { Some("ios".into()) }
            else { None }
        })
        .unwrap_or_default();
    let device = extract_field(content, "device:").unwrap_or_default();
    let result = extract_field(content, "result:")
        .unwrap_or_else(|| {
            // iOS format: check if any step FAILed
            if content.contains("result: FAIL") { "FAIL".into() } else { "PASS".into() }
        });
    let started = extract_field(content, "started:").unwrap_or_default();

    // Extract failure details
    let (failure_step, failure_reason) = if result == "FAIL" {
        extract_failure(content)
    } else {
        (None, None)
    };

    // Use file modification time as canonical timestamp (android clock is wrong)
    let timestamp = std::fs::metadata(filename)
        .and_then(|m| m.modified())
        .map(|t| format!("{:?}", t))
        .unwrap_or(started);

    if tc_id.is_empty() {
        return None;
    }

    Some(RunResult { tc_id, tc_name, platform, device, result, failure_step, failure_reason, timestamp })
}

fn extract_field(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) {
            let val = trimmed[key.len()..].trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_failure(content: &str) -> (Option<usize>, Option<String>) {
    let mut last_fail_step = None;
    let mut last_fail_error = None;
    let mut current_step = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("step:") || trimmed.starts_with("- step:") {
            if let Some(n) = trimmed.rsplit(':').next().and_then(|s| s.trim().parse::<usize>().ok()) {
                current_step = n;
            }
        }
        if trimmed == "result: FAIL" {
            last_fail_step = Some(current_step);
        }
        if trimmed.starts_with("error:") {
            let err = trimmed["error:".len()..].trim().trim_matches('"').trim_matches('\'');
            if !err.is_empty() {
                last_fail_error = Some(err.to_string());
            }
        }
    }

    (last_fail_step, last_fail_error)
}
