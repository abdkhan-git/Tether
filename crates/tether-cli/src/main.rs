//! `tether`: control the Tether daemon on this computer. See `tether --help`.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use tether_cli::{run, term, Cli, Target};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = async {
        let target = Target::for_profile(&cli.profile)?;
        run(cli.command, &target, &mut std::io::stdout().lock(), &mut std::io::stdin().lock()).await
    }
    .await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Error text can quote the daemon; keep it inert too.
            eprintln!("error: {}", term::safe(&format!("{e:#}"), false));
            ExitCode::FAILURE
        }
    }
}
