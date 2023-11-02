//! Example service that reflects data.

use clap::Parser;
use daemonbase::log;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    log: daemonbase::log::Args,
}


fn main() {
    let args = Args::parse();

    eprintln!("log: {:?}", log::Config::from_args(&args.log));
}
