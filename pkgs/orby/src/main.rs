#[cfg(target_os = "macos")]
fn main() {
    if let Err(err) = run() {
        eprintln!("Orby: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn run() -> orby::Result<()> {
    use orby::{
        config::{RunnerConfig, STATE},
        machine::Machine,
        proxy::Proxy,
    };
    use std::{
        path::Path,
        sync::atomic::{AtomicBool, Ordering},
    };

    static SHUTDOWN: AtomicBool = AtomicBool::new(false);
    extern "C" fn terminate(_: libc::c_int) {
        SHUTDOWN.store(true, Ordering::Relaxed);
    }

    fn serve(shutdown: &AtomicBool, terminate: extern "C" fn(libc::c_int)) -> orby::Result<()> {
        let config = RunnerConfig::read(Path::new(STATE).join("runner.json").as_path())?;
        let listener = orby::launchd::listener(config.port)?;
        unsafe {
            libc::signal(libc::SIGTERM, terminate as libc::sighandler_t);
            libc::signal(libc::SIGINT, terminate as libc::sighandler_t);
        }
        Proxy::new(Machine::new(config)).serve(listener, shutdown)
    }

    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_] => serve(&SHUTDOWN, terminate),
        [_, command] if command == "serve" => serve(&SHUTDOWN, terminate),
        [_, command, config] if command == "activate" => {
            orby::activation::activate(Path::new(config))
        }
        [_, command] if command == "purge" => orby::activation::purge(),
        _ => Err(orby::error("usage: orby [serve | activate CONFIG | purge]")),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Orby runs only on Apple Silicon macOS");
    std::process::exit(1);
}
