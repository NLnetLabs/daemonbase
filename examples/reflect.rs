//! Example service that reflects data.

use clap::Parser;
use daemonbase::logging;
use log::{warn};

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    log: logging::Args,
}


fn main() {
    if logging::Config::init_logging().is_err() {
        return
    }
    warn!("Logging initialized.");

    let args = Args::parse();
    let log = logging::Config::from_args(&args.log);
    if log.switch_logging(false).is_err() {
        return
    }
    warn!("Switched logging.");

}
