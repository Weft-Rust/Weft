use std::{fmt::Display, path::PathBuf, str::FromStr};

use clap::{Args, Parser, Subcommand};

/// Weft is a voxel sandbox engine written in Rust.
/// The game itself is a mod, `mods/base`,
/// built only on the public SDK:
/// it has no private hooks into the engine,
/// so anything it does, another mod can do or change.
/// Weft is not a Minecraft clone and does not read Minecraft worlds;
/// it plays like one and borrows its numbers where that makes sense.
///
// TODO Make this better?
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The command to run.
    ///
    /// By default, it is `client`. See `weft client --help` for more details.
    #[command(subcommand)]
    pub command: Option<CliCommand>,
    // The client args since clap doesn't have a default subcommand feature
    #[command(flatten)]
    pub client_args: Option<ClientArgs>,
    /// The extra arguments passed to mods.
    ///
    /// How to use:
    ///
    /// Just put "--" in your argument and all the values will be passed as extra arguments.
    ///
    /// ```
    /// weft -- --test bed --test dog --mod gregtech
    /// ```
    //
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Args, Default)]
pub struct ClientArgs {
    #[command(flatten)]
    pub base: BaseArguments,
    /// Enable gpu debugging.
    // NOTE: I dunno what the `gpu_debug` does
    #[arg(long)]
    pub gpu_debug: Option<bool>,
    /// Disable vsync
    #[arg(long)]
    pub no_vsync: Option<bool>,
    /// Automation: Skip the menu and play this world (created if missing).
    #[arg(long)]
    pub world: Option<String>,
    /// The seed to use to use when creating a new automated `world`.
    ///
    /// Should goes in pair with `--world`.
    #[arg(long)]
    pub seed: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(flatten)]
    pub base: BaseArguments,
    /// The game port server.
    ///
    /// Use `host:port` to play in client.
    #[arg(long)]
    pub port: Option<u16>,
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Run weft as client
    Client(ClientArgs),
    /// Run Weft as a server
    Server(ServerArgs),
    /// Pack a mod.
    PackMod,
    /// Prepare a game directory
    Dist,
}

/// All the base arguments
#[derive(Debug, Args, Default)]
pub struct BaseArguments {
    /// The directory where weft should run
    #[arg(long)]
    pub run_dir: Option<PathBuf>,
    /// The mods directory.
    /// Can be repeated.
    #[arg(long)]
    pub mods: Vec<PathBuf>,
    /// Weft log spec.
    ///
    /// The format is like this: `target1=level,level,...`,
    /// where the `target` is log target (can be a rust native module or a weft mod)
    /// and the `level` (the log filter level) must be one of these values:
    /// - off: The target logs will not be shown
    /// - error: only shows "error" logs
    /// - warn: shows "error" and "warn" logs
    /// - info: shows "error", "warn" and "info" logs
    /// - debug: shows "error", "warn", "info" and debug logs
    /// - trace: All of the target logs will be shown
    ///
    /// If the target is `all` or `*` or not even set, the `level` will be the default filter.
    #[arg(long)]
    pub log: Option<String>,
    /// 'Verbose' weft long
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
    /// The size of the game window.
    ///
    /// The format is `width`x`height`.
    ///
    /// Example:
    /// ```
    /// weft --size 1200x698
    /// ```
    ///
    /// By default, it is `1600x900`.
    #[arg(long, default_value_t = size_default_value())]
    pub size: Option<Size>,
    /// Run weft in `dev` mode.
    ///
    /// Enabled by default if you are on rust `debug_assertions`.
    #[arg(long, default_value_t = dev_default_value())]
    pub dev: bool,
    /// Drives the game in automation mode: no pause on focus loss, settings not saved.
    #[arg(long, default_value_t = false)]
    pub automated: bool,
    /// Sets the "game view distance".
    #[arg(long)]
    pub view: Option<i32>,
}

fn dev_default_value() -> bool {
    cfg!(debug_assertions)
}

fn size_default_value() -> Option<Size> {
    Some(Size(1600, 900))
}

#[derive(Debug)]
pub struct Size(pub u32, pub u32);

#[derive(Debug)]
pub struct SizeParseError(());

impl Display for SizeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(&mut f, "Invalid size format. It should always be in `WidthxHeigth` format")
    }
}

impl core::error::Error for SizeParseError {}

impl FromStr for Size {
    type Err = SizeParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v: Vec<u32> = s.split('x').filter_map(|s| s.parse().ok()).collect();
        if v.len() == 2 { Ok(Self(v[0], v[1])) } else { Err(SizeParseError(())) }
    }
}
