//! Example service that reflects data.

use clap::Parser;
use daemonbase::logging;
use daemonbase::logging::Logger;
use log::{warn};

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    log: logging::Args,
}


fn main() {
    if Logger::init_logging().is_err() {
        return
    }
    warn!("Logging initialized.");

    let args = Args::parse();
    let log = Logger::from_args(&args.log);
    if log.switch_logging(false).is_err() {
        return
    }
    warn!("Switched logging.");

}
