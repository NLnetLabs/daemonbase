//! Logging.

use std::{fmt, fs, io};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use clap::ArgAction;
use log::LevelFilter;
use log::error;
use crate::error::Failed;


//------------ Config --------------------------------------------------------

/// The configuration for logging.
#[derive(Clone, Debug)]
pub struct Config {
    /// The log levels to be logged.
    level: LevelFilter,

    /// The target to log to.
    target: Target,
}

impl Config {
    /// Initialize logging.
    ///
    /// Initializes the logging system so it can be used before having
    /// read the configuration. The function sets a maximum log level of
    /// `warn`, leading only printing important information, and directs all
    /// logging to stderr.
    pub fn init_logging() -> Result<(), Failed> {
        log::set_max_level(LevelFilter::Warn);
        if let Err(err) = log::set_logger(&GLOBAL_LOGGER) {
            eprintln!("Failed to initialize logger: {}.\nAborting.", err);
            return Err(Failed)
        }
        Ok(())
    }

    /// Creates the config from command line arguments only.
    pub fn from_args(args: &Args) -> Self {
        Self {
            level: if args.verbose > 1 {
                LevelFilter::Debug
            }
            else if args.verbose == 1 {
                LevelFilter::Info
            }
            else if args.quiet > 1 {
                LevelFilter::Off
            }
            else if args.quiet == 1 {
                LevelFilter::Error
            }
            else {
                LevelFilter::Warn
            },
            target: Target::from_args(args),
        }
    }

    /// Switches logging to the configured target.
    ///
    /// Once the configuration has been successfully loaded, logging should
    /// be switched to whatever the user asked for via this method.
    ///
    /// The `daemon` argument changes how the default log target is
    /// interpreted: If it is `true`, syslog is used on Unix systems if
    /// available via one of the standard Unix sockets. Otherwise, stderr is
    /// used.
    ///
    /// This method should only be called once. It returns an error if called
    /// again.
    pub fn switch_logging(
        &self,
        daemon: bool,
    ) -> Result<(), Failed> {
        let logger = Logger::new(self, daemon)?;
        GLOBAL_LOGGER.switch(logger);
        log::set_max_level(self.level);
        Ok(())
    }

    /// Rotates the log file if necessary.
    pub fn rotate_log(&self) -> Result<(), Failed> {
        GLOBAL_LOGGER.rotate()
    }
}


//------------ Args ----------------------------------------------------------

#[derive(Clone, Debug, clap::Args)]
#[group(id = "log-args")]
pub struct Args {
    /// Log more information, twice for even more
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Log less information, twice for no information
    #[arg(short, long, action = ArgAction::Count, conflicts_with = "verbose")]
    quiet: u8,

    /// Log to syslog
    #[cfg(unix)]
    #[arg(long, conflicts_with_all = ["stderr", "logfile"])]
    syslog: bool,

    /// Log to stderr
    #[arg(long, conflicts_with = "logfile")]
    stderr: bool,

    /// Log to this file
    #[arg(long, value_name = "PATH", conflicts_with = "stderr")]
    logfile: Option<PathBuf>,

    /// Facility to use for syslog logging
    #[cfg(unix)]
    #[arg(long, value_name = "FACILITY")]
    syslog_facility: Option<unix::FacilityArg>,
}


//------------ Target --------------------------------------------------------

/// The target to log to.
#[derive(Clone, Debug, Default)]
pub enum Target {
    /// Default.
    ///
    /// Logs to `Syslog(Facility::LOG_DAEMON)` on Unix in daemon mode and
    /// `Stderr` otherwise.
    #[default]
    Default,

    /// Syslog.
    ///
    /// The argument is the syslog facility to use.
    #[cfg(unix)]
    Syslog(syslog::Facility),

    /// Stderr.
    Stderr,

    /// A file.
    ///
    /// The argument is the file name.
    File(PathBuf)
}

impl Target {
    fn from_args(args: &Args) -> Self {
        #[cfg(unix)]
        if args.syslog {
            return Self::Syslog(
                args.syslog_facility.map(Into::into).unwrap_or(
                    syslog::Facility::LOG_DAEMON
                )
            )
        }

        if args.stderr {
            return Self::Stderr
        }

        if let Some(path) = args.logfile.as_ref() {
            return Self::File(path.clone())
        }

        Self::Default
    }
}


//--- PartialEq and Eq

impl PartialEq for Target {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Default, Self::Default) => true,
            #[cfg(unix)]
            (&Self::Syslog(s), &Self::Syslog(o)) => {
                (s as usize) == (o as usize)
            }
            (Self::Stderr, Self::Stderr) => true,
            (Self::File(s), Self::File(o)) => {
                s == o
            }
            _ => false
        }
    }
}

impl Eq for Target { }


//------------ Logger --------------------------------------------------------

/// Format and write log messages.
struct Logger {
    /// Where to write messages to.
    target: Mutex<LogBackend>,

    /// The maximum log level.
    level: LevelFilter,
}

/// The actual target for logging
enum LogBackend {
    #[cfg(unix)]
    Syslog(unix::SyslogLogger),
    File {
        file: fs::File,
        path: PathBuf,
    },
    Stderr {
        stderr: io::Stderr,
        timestamp: bool,
    }
}

impl Logger {
    /// Creates a new logger from config and additional information.
    fn new(
        config: &Config, daemon: bool,
    ) -> Result<Self, Failed> {
        let target = match config.target {
            #[cfg(unix)]
            Target::Default => {
                if daemon { 
                    Self::new_syslog_target(
                        syslog::Facility::LOG_DAEMON, false,
                    )?
                }
                else {
                    Self::new_stderr_target(false)
                }
            }
            #[cfg(not(unix))]
            Target::Default => {
                Self::new_stderr_target(false)
            }
            #[cfg(unix)]
            Target::Syslog(facility) => {
                Self::new_syslog_target(facility, true)?
            }
            Target::File(ref path) => {
                Self::new_file_target(path.clone())?
            }
            Target::Stderr => {
                Self::new_stderr_target(daemon)
            }
        };
        Ok(Self {
            target: Mutex::new(target),
            level: config.level,
        })
    }

    /// Creates a syslog target.
    ///
    /// If `use_inet` is `true`, also tries using the TCP and UDP options.
    #[cfg(unix)]
    fn new_syslog_target(
        facility: syslog::Facility,
        use_inet: bool,
    ) -> Result<LogBackend, Failed> {
        unix::SyslogLogger::new(facility, use_inet).map(LogBackend::Syslog)
    }

    fn new_file_target(path: PathBuf) -> Result<LogBackend, Failed> {
        Ok(LogBackend::File {
            file: match Self::open_log_file(&path) {
                Ok(file) => file,
                Err(err) => {
                    error!(
                        "Failed to open log file '{}': {}",
                        path.display(), err
                    );
                    return Err(Failed)
                }
            },
            path
        })
    }

    /// Opens a log file.
    fn open_log_file(path: &PathBuf) -> Result<fs::File, io::Error> {
        fs::OpenOptions::new().create(true).append(true).open(path)
    }

    /// Configures the stderr target.
    fn new_stderr_target(timestamp: bool) -> LogBackend {
        LogBackend::Stderr {
            stderr: io::stderr(),
            timestamp,
        }
    }

    /// Returns a mutex lock for the target
    fn target(&self) -> MutexGuard<LogBackend> {
        self.target.lock().expect("poisoned mutex")
    }

    /// Logs a message.
    ///
    /// This method may exit the whole process if logging fails.
    fn log(&self, record: &log::Record) {
        if self.should_ignore(record) {
            return;
        }

        if let Err(err) = self.try_log(record) {
            self.log_failure(err);
        }
    }

    /// Tries logging a message and returns an error if there is one.
    fn try_log(&self, record: &log::Record) -> Result<(), io::Error> {
        match self.target().deref_mut() {
            #[cfg(unix)]
            LogBackend::Syslog(ref mut logger) => logger.log(record),
            LogBackend::File { ref mut file, .. } => {
                writeln!(
                    file, "[{}] [{}] {}",
                    format_timestamp(),
                    record.level(),
                    record.args()
                )
            }
            LogBackend::Stderr{ ref mut stderr, timestamp } => {
                // We never fail when writing to stderr.
                if *timestamp {
                    let _ = writeln!(stderr, "[{}] [{}] {}",
                        format_timestamp(), record.level(), record.args()
                    );
                }
                else {
                    let _ = writeln!(
                        stderr, "[{}] {}", record.level(), record.args()
                    );
                }
                Ok(())
            }
        }
    }

    /// Handles an error that happened during logging.
    fn log_failure(&self, err: io::Error) -> ! {
        // We try to write a meaningful message to stderr and then abort.
        match self.target().deref() {
            #[cfg(unix)]
            LogBackend::Syslog(_) => {
                eprintln!("Logging to syslog failed: {}. Exiting.", err);
            }
            LogBackend::File { ref path, .. } => {
                eprintln!(
                    "Logging to file {} failed: {}. Exiting.",
                    path.display(),
                    err
                );
            }
            LogBackend::Stderr { ..  } => {
                // We never fail when writing to stderr.
            }
        }
        std::process::exit(1)
    }

    /// Flushes the logging backend.
    fn flush(&self) {
        match self.target().deref_mut() {
            #[cfg(unix)]
            LogBackend::Syslog(ref mut logger) => logger.flush(),
            LogBackend::File { ref mut file, .. } => {
                let _ = file.flush();
            }
            LogBackend::Stderr { ref mut stderr, .. } => {
                let _  = stderr.lock().flush();
            }
        }
    }

    /// Determines whether a log record should be ignored.
    ///
    /// This filters out messages by libraries that we don’t really want to
    /// see.
    fn should_ignore(&self, record: &log::Record) -> bool {
        let module = match record.module_path() {
            Some(module) => module,
            None => return false,
        };

        // log::Level sorts more important first.

        if record.level() > log::Level::Error {
            // From rustls, only log errors.
            if module.starts_with("rustls") {
                return true
            }
        }
        if self.level >= log::LevelFilter::Debug {
            // Don’t filter anything else if we are in debug or trace.
            return false
        }

        // Ignore these modules unless INFO or more important.
        record.level() > log::Level::Info && (
               module.starts_with("tokio_reactor")
            || module.starts_with("hyper")
            || module.starts_with("reqwest")
            || module.starts_with("h2")
        )
    }

    /// Rotates the log target if necessary.
    ///
    /// This method exits the whole process when rotating fails.
    fn rotate(&self) -> Result<(), Failed> {
        if let LogBackend::File {
            ref mut file, ref path
        } = self.target().deref_mut() {
            // This tries to open the file. If this fails, it writes a
            // message to both the old file and stderr and then exits.
            *file = match Self::open_log_file(path) {
                Ok(file) => file,
                Err(err) => {
                    let _ = writeln!(file,
                        "Re-opening log file {} failed: {}. Exiting.",
                        path.display(), err
                    );
                    eprintln!(
                        "Re-opening log file {} failed: {}. Exiting.",
                        path.display(), err
                    );
                    return Err(Failed)
                }
            }
        }
        Ok(())
    }
}


//------------ SyslogLogger --------------------------------------------------

#[cfg(unix)]
mod unix {
    use super::*;
    use clap::builder::PossibleValue;

    /// A syslog logger.
    ///
    /// This is essentially [`syslog::BasicLogger`] but that one keeps the
    /// logger behind a mutex – which we already do – and doesn’t return
    /// error – which we do want to see.
    pub struct SyslogLogger(
        syslog::Logger<syslog::LoggerBackend, syslog::Formatter3164>
    );

    impl SyslogLogger {
        /// Creates a new syslog logger.
        pub fn new(
            facility: syslog::Facility,
            use_inet: bool,
        ) -> Result<Self, Failed> {
            let process = std::env::current_exe().ok().and_then(|path|
                path.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .map(ToString::to_string)
            ).unwrap_or_else(|| String::from("routinator"));
            let formatter = syslog::Formatter3164 {
                facility,
                hostname: None,
                process,
                pid: std::process::id(),
            };

            match syslog::unix(formatter.clone()) {
                Ok(logger) => return Ok(Self(logger)),
                Err(err) => {
                    if !use_inet {
                        error!("Cannot connect to syslog: {}", err);
                        return Err(Failed)
                    }
                }
            }

            let logger = syslog::tcp(
                formatter.clone(), ("127.0.0.1", 601)
            ).or_else(|_| {
                syslog::udp(formatter, ("127.0.0.1", 0), ("127.0.0.1", 514))
            });
            match logger {
                Ok(logger) => Ok(Self(logger)),
                Err(err) => {
                    error!("Cannot connect to syslog: {}", err);
                    Err(Failed)
                }
            }
        }

        /// Tries logging.
        pub fn log(&mut self, record: &log::Record) -> Result<(), io::Error> {
            match record.level() {
                log::Level::Error => self.0.err(record.args()),
                log::Level::Warn => self.0.warning(record.args()),
                log::Level::Info => self.0.info(record.args()),
                log::Level::Debug => self.0.debug(record.args()),
                log::Level::Trace => {
                    // Syslog doesn’t have trace, use debug instead.
                    self.0.debug(record.args())
                }
            }.map_err(|err| {
                match err.0 {
                    syslog::ErrorKind::Io(err) => err,
                    syslog::ErrorKind::Msg(err) => {
                        io::Error::new(io::ErrorKind::Other, err)
                    }
                    err => {
                        io::Error::new(io::ErrorKind::Other, format!("{}", err))
                    }
                }
            })
        }

        /// Flushes the logger.
        ///
        /// Ignores any errors.
        pub fn flush(&mut self) {
            let _ = self.0.backend.flush();
        }
    }

    /// Helper type to use the facility with a clap parser.
    #[derive(Clone, Copy, Debug)]
    pub struct FacilityArg(syslog::Facility);

    impl FacilityArg {
        pub fn as_str(self) -> &'static str {
            use syslog::Facility::*;

            match self.0 {
                LOG_KERN => "kern",
                LOG_USER => "user",
                LOG_MAIL => "mail",
                LOG_DAEMON => "daemon",
                LOG_AUTH => "auth",
                LOG_SYSLOG => "syslog",
                LOG_LPR => "lpr",
                LOG_NEWS => "news",
                LOG_UUCP => "uucp",
                LOG_CRON => "cron",
                LOG_AUTHPRIV => "authpriv",
                LOG_FTP => "ftp",
                LOG_LOCAL0 => "local0",
                LOG_LOCAL1 => "local1",
                LOG_LOCAL2 => "local2",
                LOG_LOCAL3 => "local3",
                LOG_LOCAL4 => "local4",
                LOG_LOCAL5 => "local5",
                LOG_LOCAL6 => "local6",
                LOG_LOCAL7 => "local7",
            }
        }
    }

    impl From<FacilityArg> for syslog::Facility {
        fn from(arg: FacilityArg) -> Self {
            arg.0
        }
    }

    impl clap::ValueEnum for FacilityArg {
        fn value_variants<'a>() -> &'a [Self] {
            &[
                Self(syslog::Facility::LOG_KERN),
                Self(syslog::Facility::LOG_USER),
                Self(syslog::Facility::LOG_MAIL),
                Self(syslog::Facility::LOG_DAEMON),
                Self(syslog::Facility::LOG_AUTH),
                Self(syslog::Facility::LOG_SYSLOG),
                Self(syslog::Facility::LOG_LPR),
                Self(syslog::Facility::LOG_NEWS),
                Self(syslog::Facility::LOG_UUCP),
                Self(syslog::Facility::LOG_CRON),
                Self(syslog::Facility::LOG_AUTHPRIV),
                Self(syslog::Facility::LOG_FTP),
                Self(syslog::Facility::LOG_LOCAL0),
                Self(syslog::Facility::LOG_LOCAL1),
                Self(syslog::Facility::LOG_LOCAL2),
                Self(syslog::Facility::LOG_LOCAL3),
                Self(syslog::Facility::LOG_LOCAL4),
                Self(syslog::Facility::LOG_LOCAL5),
                Self(syslog::Facility::LOG_LOCAL6),
                Self(syslog::Facility::LOG_LOCAL7),
            ]
        }

        fn to_possible_value(&self) -> Option<PossibleValue> {
            Some(PossibleValue::new(self.as_str()))
        }
    }
}


//------------ GlobalLogger --------------------------------------------------

/// The global logger.
///
/// A value of this type can go into a static. Until a proper logger is
/// installed, it just writes all log output to stderr.
struct GlobalLogger {
    /// The real logger. Can only be set once.
    inner: OnceLock<Logger>,
}

/// The static for the log crate.
static GLOBAL_LOGGER: GlobalLogger = GlobalLogger::new();

impl GlobalLogger {
    /// Creates a new provisional logger.
    const fn new() -> Self {
        GlobalLogger { inner: OnceLock::new() }
    }

    /// Switches to the proper logger.
    fn switch(&self, logger: Logger) {
        if self.inner.set(logger).is_err() {
            panic!("Tried to switch logger more than once.")
        }
    }

    /// Performs a log rotation.
    fn rotate(&self) -> Result<(), Failed> {
        match self.inner.get() {
            Some(logger) => logger.rotate(),
            None => Ok(()),
        }
    }
}


impl log::Log for GlobalLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        match self.inner.get() {
            Some(logger) => logger.log(record),
            None => {
                let _ = writeln!(
                    io::stderr().lock(), "[{}] {}",
                    record.level(), record.args()
                );
            }
        }
    }

    fn flush(&self) {
        if let Some(logger) = self.inner.get() {
            logger.flush()
        }
    }
}


//------------ Formatting dates ----------------------------------------------

pub fn format_timestamp() -> impl fmt::Display {
    use chrono::Local;
    use chrono::format::{Item, Numeric, Pad};

    const LOCAL_ISO_DATE: &[Item<'static>] = &[
        Item::Numeric(Numeric::Year, Pad::Zero),
        Item::Literal("-"),
        Item::Numeric(Numeric::Month, Pad::Zero),
        Item::Literal("-"),
        Item::Numeric(Numeric::Day, Pad::Zero),
        Item::Literal("T"),
        Item::Numeric(Numeric::Hour, Pad::Zero),
        Item::Literal(":"),
        Item::Numeric(Numeric::Minute, Pad::Zero),
        Item::Literal(":"),
        Item::Numeric(Numeric::Second, Pad::Zero),
    ];

    Local::now().format_with_items(LOCAL_ISO_DATE.iter())
}
