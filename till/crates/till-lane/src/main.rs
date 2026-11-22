// Money, quantity and tax formatting for the screen and the receipt.
mod fmt;

// Design tokens for the egui theme. Individual unused tokens are marked where they are declared.
mod theme;

// Device traits and simulated implementations.
mod device;

// ESC/POS byte encoder, which `receipt` composes the tax receipt on top of.
mod escpos;

// Composes the RRA tax receipt.
mod receipt;

// Composes the shift reports (internal documents, not fiscal receipts, and not
// RRA's daily X and Z; see `report.rs`).
mod report;

// Fiscal port trait and simulated VSDC. Phase 3 will write the real client.
mod fiscal;

// The sale itself.
mod lane;

// Latency budgets and the timings `lane` records against them.
mod latency;

// Where the cloud address and lane token come from.
mod config;

// Drains the outbound queue to the cloud and pulls the cashier list.
mod sync;
mod sync_status;

mod demo;
mod devices;
mod ui;

use devices::Devices;
use eframe::egui;
use lane::LaneState;

/// What the window shows: a running lane, or why there is none.
enum LaneApp {
    Running {
        state: Box<LaneState>,
        devices: Box<Devices>,
    },
    Fatal(String),
}

impl eframe::App for LaneApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        match self {
            LaneApp::Running { state, devices } => ui::lane(ctx, state, devices),
            LaneApp::Fatal(message) => ui::fatal(ctx, message),
        }
    }
}

/// How the lane was asked to run.
#[derive(Debug)]
struct Args {
    /// `--demo`: run against the simulated VSDC and the demo catalogue.
    demo: bool,
    /// `--db <path>`: keep the till's data in this file instead of in memory,
    /// so a shift, its sales and its invoice sequence survive a restart.
    db: Option<std::path::PathBuf>,
    /// `--vsdc-offline`: start with the simulated VSDC unreachable, so the
    /// first sale hard stops. Shift+F12 switches it while the lane runs.
    vsdc_offline: bool,
    /// `--set-token`: read a lane token from standard input, save it to the OS
    /// credential store, and exit.
    set_token: bool,
}

/// Reads the command line. An unknown flag is refused rather than ignored: a
/// mistyped `--db` that silently fell back to memory would look like data loss.
fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Result<Args, String> {
    let mut args = Args {
        demo: false,
        db: None,
        vsdc_offline: false,
        set_token: false,
    };
    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--demo" => args.demo = true,
            "--vsdc-offline" => args.vsdc_offline = true,
            "--set-token" => args.set_token = true,
            "--db" => {
                let path = it
                    .next()
                    .ok_or_else(|| "--db needs a file path after it.".to_owned())?;
                args.db = Some(std::path::PathBuf::from(path));
            }
            other => {
                return Err(format!(
                    "Unknown option {other}. Known: --demo, --db <path>, --vsdc-offline, --set-token."
                ));
            }
        }
    }
    Ok(args)
}

/// Opens the lane. Any failure here is a startup failure and never a hard
/// stop: the hard stop means "the VSDC is unreachable", and blaming it for a
/// database fault would send a shop to check a network that is fine.
async fn start(args: &Args) -> Result<LaneApp, String> {
    if !args.demo {
        // Phase 3 writes the real VSDC client. Until then the only fiscal port
        // is a simulator that invents receipts, which must not run a real shop.
        return Err("This build has no VSDC client yet, so it cannot sell for real. Run it with --demo to try the lane.".into());
    }
    let store = match &args.db {
        Some(path) => till_store::Store::open(path).await.map_err(|e| {
            format!(
                "The database at {} could not be opened ({e}).",
                path.display()
            )
        })?,
        None => till_store::Store::open_in_memory()
            .await
            .map_err(|e| format!("The demo database could not be opened ({e})."))?,
    };
    seed_demo(&store, args.db.is_some()).await?;
    let mut state = LaneState::new(store.clone(), demo::config())
        .await
        .map_err(|e| e.to_string())?;
    start_sync(store, &mut state);
    eprintln!("{}", demo::cheat_sheet());
    if let Some(path) = &args.db {
        eprintln!("Data is kept in {} and survives a restart.", path.display());
    }
    if args.vsdc_offline {
        eprintln!("The simulated VSDC starts UNREACHABLE: the first sale will hard stop.");
    }
    Ok(LaneApp::Running {
        state: Box::new(state),
        devices: Box::new(Devices::demo(args.vsdc_offline)),
    })
}

/// Loads the demo catalogue and cashiers into an in-memory demo store only.
///
/// A store on disk (`--db`) is a till whose cashiers and catalogue come from
/// the cloud. `sync_cashiers` DELETES every cashier absent from the list it is
/// handed, so seeding the demo cashier at each start would wipe the real ones
/// and open shifts as `demo-cashier`, which the cloud refuses, withholding
/// every sale of the shift. `sync_catalogue` never deletes, but seeding demo
/// products into a real catalogue lets a cashier sell items the cloud has
/// never heard of (their lines would be refused), so it is skipped too.
async fn seed_demo(store: &till_store::Store, persistent: bool) -> Result<(), String> {
    if persistent {
        return Ok(());
    }
    store
        .sync_catalogue(&demo::products())
        .await
        .map_err(|e| format!("The demo catalogue could not be loaded ({e})."))?;
    store
        .sync_cashiers(&demo::cashiers())
        .await
        .map_err(|e| format!("The demo cashiers could not be loaded ({e})."))
}

/// Starts the cloud sync on the runtime (never the UI thread), if the lane has
/// an address and a token. Without them the till still sells: sync is off, the
/// queue just grows, and the reason goes to stderr (not to the cashier).
fn start_sync(store: till_store::Store, state: &mut LaneState) {
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cloud sync is off: {e}.");
            return;
        }
    };
    match sync::CloudSync::new(store, cfg) {
        Ok(cloud) => {
            state.set_sync_wake(cloud.waker());
            state.set_sync_status(cloud.status_handle());
            tokio::spawn(std::sync::Arc::new(cloud).run());
        }
        Err(e) => eprintln!("Cloud sync is off: {e}."),
    }
}

/// `--set-token`: the token is read from stdin so it never lands in shell history.
fn set_token_from_stdin() -> i32 {
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        eprintln!("Could not read the token from standard input.");
        return 1;
    }
    match config::store_token(&line) {
        Ok(()) => {
            eprintln!("Lane token saved to the OS credential store.");
            0
        }
        Err(e) => {
            eprintln!("Could not save the token: {e}.");
            1
        }
    }
}

fn main() -> eframe::Result {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if args.set_token {
        std::process::exit(set_token_from_stdin());
    }

    // The UI owns the runtime. `LaneState`'s methods are `async` because the
    // store's API is, but egui's update loop is synchronous, so `ui` blocks on
    // them. They are local SQLite calls (sub-millisecond), so blocking the UI
    // thread is acceptable. Entering the runtime lets `ui` find it with
    // `Handle::current()` on this (the UI) thread.
    let runtime = tokio::runtime::Runtime::new().expect("the async runtime starts");
    let _entered = runtime.enter();

    let app = runtime.block_on(start(&args)).unwrap_or_else(|message| {
        eprintln!("lane could not start: {message}");
        LaneApp::Fatal(message)
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Simplex Lane")
            .with_inner_size([1280.0, 1024.0])
            // A till owns its screen. Maximised also keeps the footer (totals
            // and tender) on-screen on a short laptop panel, where a
            // 1024-tall window would put it under the taskbar.
            .with_maximized(true)
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Simplex Lane",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            ui::set_session(
                &cc.egui_ctx,
                ui::Session {
                    lane: "Lane 02".into(),
                },
            );
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn no_arguments_means_no_demo_no_file_and_a_reachable_vsdc() {
        let a = parse(&[]).unwrap();
        assert!(!a.demo);
        assert!(a.db.is_none());
        assert!(!a.vsdc_offline);
    }

    #[test]
    fn the_flags_parse_in_any_order() {
        for argv in [
            vec!["--demo", "--db", "till.db", "--vsdc-offline"],
            vec!["--vsdc-offline", "--db", "till.db", "--demo"],
            vec!["--db", "till.db", "--vsdc-offline", "--demo"],
        ] {
            let a = parse(&argv).unwrap();
            assert!(a.demo, "{argv:?}");
            assert!(a.vsdc_offline, "{argv:?}");
            assert_eq!(a.db.as_deref(), Some(std::path::Path::new("till.db")));
        }
    }

    #[test]
    fn db_without_a_path_is_refused() {
        let e = parse(&["--demo", "--db"]).unwrap_err();
        assert!(e.contains("needs a file path"), "{e}");
    }

    /// A mistyped flag must not be ignored. `--db` silently falling back to an
    /// in-memory store would read as data loss at the end of a shift.
    #[test]
    fn an_unknown_flag_is_refused_rather_than_ignored() {
        for bad in ["--dbb", "--vsdc_offline", "-demo", "till.db"] {
            let e = parse(&[bad]).unwrap_err();
            assert!(e.contains(bad), "{bad}: {e}");
            assert!(
                e.contains("--db <path>"),
                "the error must list the real flags: {e}"
            );
        }
    }

    /// A path that looks like a flag is still a path: `--db --demo` must not
    /// quietly consume `--demo` as the filename and then not be a demo.
    #[test]
    fn a_flag_shaped_db_path_is_taken_literally() {
        let a = parse(&["--db", "--demo"]).unwrap();
        assert_eq!(a.db.as_deref(), Some(std::path::Path::new("--demo")));
        assert!(!a.demo, "--demo was consumed as the path, so it is not set");
    }

    fn real_cashier(id: &str) -> till_store::Cashier {
        till_store::Cashier {
            id: id.into(),
            name: "Real".into(),
            pin_hash: "x".into(),
            active: true,
        }
    }

    /// Startup on a `--db` store must leave the cloud's cashiers and catalogue
    /// alone: `sync_cashiers` deletes whatever the list omits.
    #[tokio::test]
    async fn a_db_stores_cashiers_survive_startup() {
        let path = std::env::temp_dir().join(format!(
            "till-lane-seed-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let store = till_store::Store::open(&path).await.unwrap();
        store
            .sync_cashiers(&[real_cashier("real-1")])
            .await
            .unwrap();
        seed_demo(&store, true).await.unwrap();
        let ids: Vec<String> = store
            .active_cashiers()
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec!["real-1".to_string()]);
        drop(store);
        for ext in ["", "-wal", "-shm"] {
            let mut f = path.clone().into_os_string();
            f.push(ext);
            let _ = std::fs::remove_file(f);
        }
    }

    #[tokio::test]
    async fn the_in_memory_demo_is_seeded_with_the_demo_cashier() {
        let store = till_store::Store::open_in_memory().await.unwrap();
        seed_demo(&store, false).await.unwrap();
        let ids: Vec<String> = store
            .active_cashiers()
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec!["demo-cashier".to_string()]);
    }
}
