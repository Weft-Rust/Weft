use std::{fmt::Display, path::PathBuf, str::FromStr};

use clap::{Args, Parser, Subcommand};

/// Weft is a voxel sandbox engine written in Rust.
///
/// The game itself is a mod, `mods/base`,
/// built only on the public SDK:
/// it has no private hooks into the engine,
/// so anything it does, another mod can do or change.
///
/// Weft is not a Minecraft clone and does not read Minecraft worlds;
/// it plays like one and borrows its numbers where that makes sense.
// TODO Make this better?
#[derive(Debug, Parser)]
#[command(version, about, long_about, override_usage = "weft [COMMAND] [OPTIONS] [-- [EXTRAS_ARGS]...]")]
pub struct Cli {
    // The client args since clap doesn't have a default subcommand feature
    #[command(flatten)]
    pub client_args: ClientArgs,
    /// The command to run.
    ///
    /// By default, it is `client`. See `weft client --help` for more details.
    #[command(subcommand)]
    pub command: Option<CliCommand>,
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
    #[arg(last = true, global = true)]
    pub extra_args: Vec<String>,
}

impl Cli {
    pub fn get_command(self) -> (CliCommand, Vec<(String, String)>) {
        let command = self
            .command
            .unwrap_or_else(|| CliCommand::Client(self.client_args));
        let extras = {
            let mut extras = Vec::<(String, String)>::new();
            let mut args = self.extra_args.iter().peekable();
            while let Some(arg) = args.next() {
                match arg.strip_prefix("--") {
                    Some(name) => {
                        let v: String = if args.peek().is_some_and(|n| !n.starts_with("--")) {
                            args.next().cloned().unwrap_or_default()
                        } else {
                            "1".into()
                        };
                        extras.push((name.to_string(), v));
                    }
                    None => eprintln!("unknown argument {arg}"),
                }
            }
            extras
        };
        (command, extras)
    }
}

#[derive(Debug, Args, Default)]
pub struct ClientArgs {
    #[command(flatten)]
    pub base: BaseArguments,
    /// Enable gpu debugging.
    // NOTE: I dunno what the `gpu_debug` does
    #[arg(long)]
    pub gpu_debug: bool,
    /// Disable vsync
    #[arg(long)]
    pub no_vsync: bool,
    /// Automation: Skip the menu and play this world (created if missing).
    #[arg(long)]
    pub world: Option<String>,
    /// The seed to use to use when creating a new automated `world`.
    ///
    /// Should goes in pair with `--world`.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Open the built-in screen
    #[arg(long)]
    pub screen: Option<String>,
    /// The renderer to use.
    ///
    /// Possible values:
    /// `vulkan`
    /// `wgpu`
    #[arg(long)]
    pub renderer: Option<String>,
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(flatten)]
    pub base: BaseArguments,
    /// The game port server.
    ///
    /// Use `host:port` to play in client.
    #[arg(long, short)]
    pub port: Option<u16>,
    /// Play the given world (created if missing).
    ///
    /// By default, it is `world`.
    #[arg(long)]
    pub world: Option<String>,
    /// The seed to use to use when creating a new `world`.
    ///
    /// Should goes in pair with `--world`.
    #[arg(long)]
    pub seed: Option<u64>,
}

/// `weft` will run on `client` by default if no command is set.
#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Run weft as client
    Client(ClientArgs),
    /// Run Weft as a server
    Server(ServerArgs),
    /// Pack a mod.
    PackMod(PackModArgs),
    /// Prepare a folder to hand to someone:
    /// the executable, the engine's assets and every mod packed.
    Dist(DistArgs),
    /// Sends a mod's pushed commit to the mod index as
    /// one new file, `mods/<id>/<version>.toml`. The index builds the mod itself from that commit, so
    /// nothing is uploaded from here. With the GitHub CLI (`gh`) logged in the pull request is opened
    /// from here; otherwise the browser opens GitHub's page for the new file, filled in.
    SubmitMod(SubmitModArgs),
    /// A shortcut to `weft --automated -- --test all`
    TestAll(TestAllArgs),
}

#[derive(Debug, Args)]
pub struct TestAllArgs {
    /// The directory where weft should run
    #[arg(long)]
    pub run_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct SubmitModArgs {
    /// The mod to submit
    #[arg(short, long)]
    pub r#mod: String,
    /// The owner of the mod.
    ///
    /// Mostly the Github login credentials
    #[arg(long)]
    pub owner: Option<String>,
}

#[derive(Debug, Args)]
pub struct PackModArgs {
    /// The directory where we put the mods.
    #[arg(long, short)]
    pub out_dir: Option<PathBuf>,
    #[arg(long, short)]
    /// The mod to pack.
    pub r#mod: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct DistArgs {
    /// The game root directory to copy
    pub game_root: PathBuf,
    /// The target directory
    pub out: PathBuf,
}

/// All the base arguments
#[derive(Debug, Args, Default)]
pub struct BaseArguments {
    /// The directory where weft should run
    #[arg(long)]
    pub run_dir: Option<PathBuf>,
    /// The game cache directory
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    /// The mods directory.
    /// Can be repeated.
    #[arg(long)]
    pub mods: Vec<PathBuf>,
    /// Weft log spec.
    ///
    /// The format is like this: `target1=level,level,...`,
    /// where the `target` is log target (can be a rust native module or a weft mod)
    /// and the `level` (the log filter level) must be one of these values:
    ///
    /// - off: The target logs will not be shown
    ///
    /// - error: only shows "error" logs
    ///
    /// - warn: shows "error" and "warn" logs
    ///
    /// - info: shows "error", "warn" and "info" logs
    ///
    /// - debug: shows "error", "warn", "info" and debug logs
    ///
    /// - trace: All of the target logs will be shown
    ///
    /// If the target is `all` or `*` or not even set, the `level` will be the default filter.
    #[arg(long)]
    pub log: Option<String>,
    /// 'Verbose' weft logs
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
    #[arg(long, default_value_t = Default::default())]
    pub size: Size,
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
    /// Mods to disable
    #[arg(long)]
    pub off: Vec<String>,
}

fn dev_default_value() -> bool {
    cfg!(debug_assertions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Size(pub u32, pub u32);

impl Default for Size {
    fn default() -> Self {
        Self(1600, 900)
    }
}

impl Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.0, self.1)
    }
}

#[derive(Debug)]
pub struct SizeParseError(());

impl Display for SizeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Invalid size format. It should always be in `WidthxHeigth` format")
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

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn test_no_arg() {
        let (command, extras) = Cli::parse_from(["weft"]).get_command();
        assert!(matches!(command, CliCommand::Client(_)));
        assert!(extras.is_empty());
    }

    #[test]
    fn test_no_arg_with_extra() {
        let (command, extras) = Cli::parse_from(["weft", "--", "--klee"]).get_command();
        assert!(matches!(command, CliCommand::Client(_)));
        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0], ("klee".into(), "1".into()))
    }

    #[test]
    fn test_client() {
        let (command, extras) = Cli::parse_from(["weft", "client"]).get_command();
        assert!(matches!(command, CliCommand::Client(_)));
        assert!(extras.is_empty());
    }

    #[test]
    fn test_client_size_with_extra() {
        let (command, extras) = Cli::try_parse_from(["weft", "client", "--size", "1234x234", "--", "--test", "zhong"])
            .unwrap()
            .get_command();

        if let CliCommand::Client(client_arg) = command {
            assert_eq!(client_arg.base.size, Size(1234, 234));
        } else {
            unreachable!("This should be `CliCommand::Client`")
        }

        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0], ("test".into(), "zhong".into()));
    }

    #[test]
    fn test_server() {
        let (command, extras) = Cli::parse_from(["weft", "server"]).get_command();
        assert!(matches!(command, CliCommand::Server(_)));
        assert!(extras.is_empty());
    }

    #[test]
    fn test_server_with_extra() {
        let (command, extras) =
            Cli::parse_from(["weft", "server", "--port", "7680", "--size", "123x45", "--", "--op", "plus"]).get_command();
        if let CliCommand::Server(arg) = command {
            assert_eq!(arg.base.size, Size(123, 45));
            assert_eq!(arg.port, Some(7680));
        } else {
            unreachable!("The parsed command should be `CliCommand::Server`");
        }
        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0], ("op".into(), "plus".into()));
    }

    #[test]
    fn test_pack_mods() {
        let (command, extras) = Cli::parse_from(["weft", "pack-mod", "-o", "some/dir/aaaa"]).get_command();
        if let CliCommand::PackMod(a) = command {
            assert_eq!(a.out_dir.unwrap().as_path(), "some/dir/aaaa");
            assert!(a.r#mod.is_empty());
        } else {
            unreachable!("This should parse to `CliCommand::PackMod`")
        }
        assert!(extras.is_empty());
    }

    #[test]
    fn test_dist() {
        let (command, extras) = Cli::parse_from(["weft", "dist", "/home/alice/.weft", "/home/nicole/.weft-alice"]).get_command();
        if let CliCommand::Dist(a) = command {
            assert_eq!(a.game_root.as_path(), "/home/alice/.weft");
            assert_eq!(a.out.as_path(), "/home/nicole/.weft-alice");
        } else {
            unreachable!("This should parse to `CliCommand::Dist`")
        }
        assert!(extras.is_empty());
    }

    #[test]
    fn test_run_dir() {
        let (command, extras) = Cli::parse_from([
            "weft",
            "--automated",
            "--run-dir",
            "/home/user/devel/Weft/run/test-jobs/1",
            "--world",
            "w",
            "--",
            "--test",
            "all",
            "--test-shard",
            "0/2",
        ])
        .get_command();
        if let CliCommand::Client(a) = command {
            assert_eq!(a.world.expect("World should be set"), "w");
            assert!(a.base.automated);
            assert_eq!(
                a.base
                    .run_dir
                    .expect("The run dir path should be available"),
                String::from("/home/user/devel/Weft/run/test-jobs/1")
            );
        } else {
            unreachable!("This should parse to CliCommand::Dist");
        }
        assert_eq!(extras[0], ("test".into(), "all".into()));
        assert_eq!(extras[1], ("test-shard".into(), "0/2".into()));
    }

    #[test]
    fn dbg_assert_clap() {
        Cli::command().debug_assert();
    }
}
