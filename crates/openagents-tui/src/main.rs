mod catalog;
mod control;
mod detection;
mod runtime;
mod setup;
mod ui;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    runtime::dispatch(runtime::Cli::parse())
}
