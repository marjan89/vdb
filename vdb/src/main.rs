mod cmd;
mod schema;

use clap::Parser;
use cmd::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = cmd::run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
