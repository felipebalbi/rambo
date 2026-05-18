//! Diagnostic: survey the post-reset state of any chip's SRAM via SWD.
use clap::Parser;
use color_eyre::Result;
use rambo::Cli;

fn main() -> Result<()> {
    color_eyre::install()?;
    Cli::parse().run()
}
