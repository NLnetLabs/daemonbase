//! Example service that reflects data.

use clap::Parser;
use daemonbase::{logging, process};
use daemonbase::error::ExitError;
use daemonbase::logging::Logger;
use daemonbase::process::Process;
use log::{warn};

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    log: logging::Args,

    /// Detach from the terminal
    #[arg(short, long)]
    detach: bool,

    #[command(flatten)]
    process: process::Args,
}


fn _main() -> Result<(), ExitError> {
    Logger::init_logging()?;
    warn!("Logging initialized.");

    let args = Args::parse();
    let log = Logger::from_config(&args.log.to_config())?;
    let mut process = Process::from_config(args.process.into_config());

    log.switch_logging(args.detach)?;
    process.setup_daemon(args.detach)?;

    // This is where you create listener sockets so they can use privileged
    // ports.

    process.drop_privileges()?;

    warn!("Up and running.");

    // This is where we do something useful later.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }

    //Ok(())
}

fn main() {
    let _ = _main();
}
