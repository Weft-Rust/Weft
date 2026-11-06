mod cli;
// use std::option;
use std::path::PathBuf;
// use std::process::ExitCode;

use clap::Parser;
use weft_engine::Options;
use weft_engine::paths::{self, Dirs};
use weft_render::{Backend, BackendDesc, Error};

use crate::cli::{BaseArguments, Cli, ClientArgs, DistArgs, PackModArgs, ServerArgs};

#[global_allocator]
static ALLOC: weft_engine::diag::CountingAlloc = weft_engine::diag::CountingAlloc;

fn vulkan(desc: &BackendDesc) -> Result<Box<dyn Backend>, Error> {
    Ok(Box::new(weft_vulkan::VulkanBackend::new(desc)?))
}

/// Where `run` (and, in the source tree, `mods`) live: the working directory if it has either,
/// otherwise the nearest folder above the executable that does (a development build runs from
/// `target/<profile>/`).
fn game_root() -> PathBuf {
    let has = |d: &std::path::Path| d.join("run").is_dir() || d.join("mods").is_dir();
    let cwd = std::env::current_dir().unwrap_or_default();
    if has(&cwd) {
        return cwd;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.ancestors()
        .skip(1)
        .find(|d| has(d))
        .map(PathBuf::from)
        .unwrap_or(cwd)
}

// I prefer putting this here instead of using `From`
fn build_options_from_base_args(mut args: BaseArguments, extras: Vec<(String, String)>) -> Options {
    let root = game_root();

    // We build the game dir here
    let dirs = args
        .run_dir
        .map_or_else(|| paths::of_install(&root), |run| Dirs::under(&run));

    // Set config in advance
    paths::set_config(dirs.config);

    let mut opts = Options {
        run_dir: dirs.data,
        // Prioritize
        cache_dir: args.cache_dir.unwrap_or(dirs.cache),
        // Mod sources of the game itself; players' mods go into `<run dir>/mods`, added below.
        mod_dirs: {
            args.mods.push(root.join("mods"));
            args.mods
        },
        gpu_debug: false,
        world: None,
        seed: None,
        screen: None,
        size: (args.size.0, args.size.1),
        dev: args.dev,
        automated: args.automated,
        no_vsync: false,
        view: args.view,
    };
    // Run test jobs
    if let Some(code) = test_jobs(&extras, &opts.run_dir) {
        std::process::exit(code);
    }

    weft_engine::set_launch_options(extras);

    let player_mods = opts.run_dir.join("mods");

    // Init enging log
    weft_engine::log::init(Some(&opts.run_dir.join("logs").join("latest.log")), args.verbose, &args.log.unwrap_or_default());
    // crash handler
    weft_engine::diag::install_crash_handler(&opts.run_dir);
    log::info!("game folder {}, cache {}", opts.run_dir.display(), opts.cache_dir.display());
    // Release builds load the game's own mods the way players get them: each packed into one
    // file in `run/mods` (again only when it changed).
    if !cfg!(debug_assertions) && root.join("mods").is_dir() {
        for dir in subdirs(&root.join("mods")) {
            match weft_engine::modloader::pack_if_stale(&dir, &player_mods) {
                Ok((path, true)) => log::info!("packed {}", path.display()),
                Ok(_) => {}
                Err(e) => log::warn!("cannot pack {}: {e}", dir.display()),
            }
        }
        let sources = root.join("mods");
        opts.mod_dirs.retain(|d| *d != sources);
    }
    // What came with a shipped game stays with it, and a player's copy of a mod comes first.
    let shipped = root.join("run").join("mods");
    let separate = shipped != player_mods;
    opts.mod_dirs.push(player_mods);
    if separate {
        opts.mod_dirs.push(shipped);
    }

    weft_engine::init_workers();

    opts
}

fn client(args: ClientArgs, extras: Vec<(String, String)>) {
    let mut options = build_options_from_base_args(args.base, extras);
    options.gpu_debug = args.gpu_debug;
    options.no_vsync = args.no_vsync;
    options.world = args.world;
    options.seed = args.seed;
    options.screen = args.screen;
    if let Err(e) = weft_engine::run(vulkan, options) {
        log::error!("{e}");
        std::process::exit(1);
    }
    std::process::exit(weft_engine::exit_code());
}

fn server(args: ServerArgs, mut extras: Vec<(String, String)>) {
    extras.retain(|(k, _)| k != "port");
    if let Some(port) = args.port {
        extras.push(("port".into(), port.to_string()));
    }
    let mut options = build_options_from_base_args(args.base, extras);
    options.world = args.world;
    options.seed = args.seed;

    if let Err(e) = weft_engine::dedicated::run(options) {
        log::error!("{e}");
        std::process::exit(1);
    }
    std::process::exit(weft_engine::exit_code());
}

fn pack_mod(args: PackModArgs, extras: Vec<(String, String)>) {
    weft_engine::set_launch_options(extras);
    for to_pack in args.to_pack {
        match weft_engine::modloader::pack(&to_pack, &args.player_mods) {
            Ok(path) => println!("packed {}", path.display()),
            Err(e) => {
                eprintln!("cannot pack {}: {e}", to_pack.display());
                std::process::exit(1)
            }
        }
    }
}

fn dist(args: DistArgs, extras: Vec<(String, String)>) {
    weft_engine::set_launch_options(extras);
    match make_dist(&args.game_root, &args.out) {
        Ok(()) => println!("game ready in {}", args.out.display()),
        Err(e) => {
            eprintln!("cannot make {}: {e}", args.out.display());
            std::process::exit(1)
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let (command, extra) = cli.get_command();
    match command {
        cli::CliCommand::Client(client_args) => client(client_args, extra),
        cli::CliCommand::Server(server_args) => server(server_args, extra),
        cli::CliCommand::PackMod(pack_mod_args) => pack_mod(pack_mod_args, extra),
        cli::CliCommand::Dist(dist_args) => dist(dist_args, extra),
    }
}

/// `--test all` runs in several processes at once, each with its own run folder (`--jobs N`, 1 for
/// one after the other, also what naming tests does unless asked). Each takes every Nth test
/// (`testkit`, `--test-shard`); what they leave is put together as a single run would have it.
fn test_jobs(extra: &[(String, String)], run_dir: &std::path::Path) -> Option<i32> {
    let option = |name: &str| {
        extra
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    if option("test-shard").is_some() {
        return None;
    }
    let test = option("test")?;
    let auto = (std::thread::available_parallelism().map_or(1, |n| n.get()) / 4).clamp(1, 6);
    let jobs = option("jobs")
        .and_then(|j| j.parse().ok())
        .unwrap_or(if test == "all" { auto } else { 1 });
    if jobs < 2 || test == "list" {
        return None;
    }
    let started = std::time::Instant::now();
    let exe = std::env::current_exe().ok()?;
    let mut dirs: Vec<PathBuf> = (0..jobs)
        .map(|i| run_dir.join("test-jobs").join(i.to_string()))
        .collect();
    let spawn = |shard: &str, dir: &PathBuf| {
        let _ = std::fs::create_dir_all(dir);
        // Or a process that dies at once passes by the last run's.
        let _ = std::fs::remove_file(dir.join("test-reports").join("report.txt"));
        let log = std::fs::File::create(dir.join("output.log")).ok()?;
        let mut c = std::process::Command::new(&exe);
        c.args(["--test", test, "--test-shard", shard, "--run-dir"])
            .arg(dir)
            .stdout(log)
            .stderr(std::process::Stdio::null());
        for (k, v) in extra.iter().filter(|(k, _)| k != "test" && k != "jobs") {
            c.arg(format!("--{k}"));
            if !v.is_empty() {
                c.arg(v);
            }
        }
        c.spawn().ok()
    };
    let children: Vec<_> = dirs
        .iter()
        .enumerate()
        .map(|(i, dir)| spawn(&format!("{i}/{jobs}"), dir))
        .collect();
    let code = |c: Option<std::process::Child>| {
        c.and_then(|mut c| c.wait().ok())
            .and_then(|s| s.code())
            .unwrap_or(1)
    };
    let mut codes: Vec<i32> = children.into_iter().map(code).collect();
    // What measures how quick the game is can't share the machine: last, on its own.
    dirs.push(run_dir.join("test-jobs").join("alone"));
    codes.push(code(spawn("alone", &dirs[jobs])));
    // A crashed process has no report, and its tests didn't pass.
    let shots = run_dir.join("test-shots");
    let _ = std::fs::remove_dir_all(&shots);
    let _ = std::fs::create_dir_all(&shots);
    let (mut passed, mut failed, mut details, mut crashed) = (0, 0, String::new(), 0);
    for (i, dir) in dirs.iter().enumerate() {
        for e in std::fs::read_dir(dir.join("test-shots"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let _ = std::fs::rename(e.path(), shots.join(e.file_name()));
        }
        let report = std::fs::read_to_string(dir.join("test-reports/report.txt")).unwrap_or_default();
        let counts: Vec<i32> = report
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|w| w.parse().ok())
            .collect();
        if let ([p, f], true) = (counts.as_slice(), codes[i] >= 0 && !report.is_empty()) {
            (passed, failed) = (passed + p, failed + f);
            details.push_str(report.split_once('\n').map_or("", |r| r.1));
        } else {
            crashed += 1;
            details.push_str(&format!("\nFAIL process {i} ended without a report, see {}\n", dir.join("output.log").display()));
        }
    }
    let head = format!("testkit report: {passed} passed, {failed} failed, {:.1} s, {jobs} at once\n", started.elapsed().as_secs_f32());
    let reports = run_dir.join("test-reports");
    let _ = std::fs::create_dir_all(&reports);
    let _ = std::fs::write(reports.join("report.txt"), format!("{head}{details}"));
    print!("{head}");
    for l in details
        .lines()
        .scan(false, |in_fail, l| {
            *in_fail = if l.starts_with("FAIL") {
                true
            } else if l.starts_with("PASS") {
                false
            } else {
                *in_fail
            };
            Some(in_fail.then_some(l))
        })
        .flatten()
    {
        println!("{l}");
    }
    Some(failed + crashed)
}

fn subdirs(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// A folder to hand to someone: the executable, the engine's assets and every mod packed.
fn make_dist(root: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::copy(&exe, out.join(exe.file_name().unwrap())).map_err(|e| e.to_string())?;
    copy_dir(&root.join("crates").join("engine").join("assets"), &out.join("assets").join("core"))?;
    for dir in subdirs(&root.join("mods")) {
        // Development tools (the test kit) mark themselves `dist = false` in Cargo.toml.
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        if manifest.lines().any(|l| l.replace(' ', "") == "dist=false") {
            continue;
        }
        weft_engine::modloader::pack(&dir, &out.join("run").join("mods"))?;
    }
    Ok(())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from)
        .map_err(|e| format!("{}: {e}", from.display()))?
        .flatten()
    {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, to.join(e.file_name())).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
