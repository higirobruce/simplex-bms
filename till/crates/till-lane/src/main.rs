// Task 5 will call these formatting functions; until then they are unused.
#[allow(dead_code)]
mod fmt;

// Later tasks consume the full token set; until then parts of it are unused.
#[allow(dead_code)]
mod theme;

// Task 3: Device traits and simulated implementations.
#[allow(dead_code)]
mod device;

// Task 4: ESC/POS byte encoder. Task 5 composes the tax receipt on top of it.
#[allow(dead_code)]
mod escpos;

// Task 5: composes the RRA tax receipt.
#[allow(dead_code)]
mod receipt;

// Task 6: fiscal port trait and simulated VSDC. Phase 3 will write the real client.
#[allow(dead_code)]
mod fiscal;

// Task 7: the sale itself.
#[allow(dead_code)]
mod lane;

// Task 10: latency budgets and the timings `lane` records against them.
#[allow(dead_code)]
mod latency;

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

/// Opens the lane. Any failure here is a startup failure and never a hard
/// stop: the hard stop means "the VSDC is unreachable", and blaming it for a
/// database fault would send a shop to check a network that is fine.
async fn start(demo: bool) -> Result<LaneApp, String> {
    if !demo {
        // Phase 3 writes the real VSDC client. Until then the only fiscal port
        // is a simulator that invents receipts, which must not run a real shop.
        return Err("This build has no VSDC client yet, so it cannot sell for real. Run it with --demo to try the lane.".into());
    }
    let store = till_store::Store::open_in_memory()
        .await
        .map_err(|e| format!("The demo database could not be opened ({e})."))?;
    store
        .sync_catalogue(&demo::products())
        .await
        .map_err(|e| format!("The demo catalogue could not be loaded ({e})."))?;
    let state = LaneState::new(store, demo::config())
        .await
        .map_err(|e| e.to_string())?;
    eprintln!("{}", demo::cheat_sheet());
    Ok(LaneApp::Running {
        state: Box::new(state),
        devices: Box::new(Devices::demo()),
    })
}

fn main() -> eframe::Result {
    let demo = std::env::args().any(|a| a == "--demo");

    // The UI owns the runtime. `LaneState`'s methods are `async` because the
    // store's API is, but egui's update loop is synchronous, so `ui` blocks on
    // them. They are local SQLite calls (sub-millisecond), so blocking the UI
    // thread is acceptable. Entering the runtime lets `ui` find it with
    // `Handle::current()` on this (the UI) thread.
    let runtime = tokio::runtime::Runtime::new().expect("the async runtime starts");
    let _entered = runtime.enter();

    let app = runtime.block_on(start(demo)).unwrap_or_else(|message| {
        eprintln!("lane could not start: {message}");
        LaneApp::Fatal(message)
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Simplex Lane")
            .with_inner_size([1280.0, 1024.0]),
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
                    cashier: "M. Uwase".into(),
                    shift: "Shift 3 · open 06:58".into(),
                },
            );
            Ok(Box::new(app))
        }),
    )
}
