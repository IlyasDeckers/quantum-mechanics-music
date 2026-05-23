//! Command-line interface — minimal after the TOML config refactor.
//!
//! Most former CLI fields now live in the config file. What's left is
//! session-specific (which MIDI port) or about the config file itself.

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "crystallized_time", version, about)]
pub struct Cli {
    /// Path to the TOML config file. Falls back to `config.toml`
    /// in the working directory if the flag is absent.
    #[arg(long, default_value = "config.toml")]
    pub config: PathBuf,

    /// Print available MIDI output ports and exit.
    #[arg(long)]
    pub list_ports: bool,

    /// Which MIDI output port to open. Session-specific (varies by
    /// machine), so stays on the CLI rather than the config file.
    #[arg(short, long, default_value_t = 0)]
    pub port: usize,

    /// Print available MIDI input ports and exit.
    #[arg(long)]
    pub list_input_ports: bool,

    /// Which MIDI input port to open. Absence runs the chain without input.
    #[arg(long)]
    pub input_port: Option<usize>,

    /// Enable the TUI monitor (ratatui-based terminal dashboard).
    #[arg(short = 't', long, default_value_t = false)]
    pub tui: bool,
}