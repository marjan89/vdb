mod compare;
mod diff;
mod overlay;
mod render;
mod validate;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "vdb", version, about = "Visual Debug Bridge — semantic diff, render, and compare")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Compare two semantic schemas and output mismatches
    Diff(diff::DiffArgs),
    /// Render semantic schema YAML as a PNG reconstruction
    Render(render::RenderArgs),
    /// Visual screenshot comparison (SSIM + pixel diff)
    Compare(compare::CompareArgs),
    /// Overlay semantic bounds on a device screenshot
    Overlay(overlay::OverlayArgs),
    /// Per-element djb2 color fingerprint validation
    Validate(validate::ValidateArgs),
}

pub fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Diff(args) => diff::run(args),
        Command::Render(args) => render::run(args),
        Command::Compare(args) => compare::run(args),
        Command::Overlay(args) => overlay::run(args),
        Command::Validate(args) => validate::run(args),
    }
}
