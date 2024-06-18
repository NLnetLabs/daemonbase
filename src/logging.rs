//! Logging.

use std::{fmt, fs, io};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, OnceLock};
use clap::ArgAction;
use log::LevelFilter;
use log::error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use crate::config::{ConfigFile, ConfigPath};
use crate::error::{ExitError, Failed};


//------------ Logger --------------------------------------------------------

/// The configuration for logging.
#[derive(Clone, Debug)]
pub struct Logger {
    /// The log levels to be logged.
    level: LevelFilter,

    /// The target to log to.
    target: Target,
}

impl Logger {
    /// Initialize logging.
    ///
    /// Initializes the logging system so it can be used before having
    /// read the configuration. The function sets a maximum log level of
    /// `warn`, leading to only printing important information, and directs
    /// all log output to stderr.
    pub fn init_logging() -> Result<(), ExitError> {
        log::set_max_level(LevelFilter::Warn);
        if let Err(err) = log::set_logger(&GLOBAL_LOGGER) {
            eprintln!("Failed to initialize logger: {}.\nAborting.", err);
            return Err(ExitError::default())
        }
        Ok(())
    }

    /// Creates the logger from a config struct.
    pub fn from_config(config: &Config) -> Result<Self, Failed> {
        Ok(Self {
            level: config.log_level.0,
            target: match config.log_target {
                TargetName::Default => Target::Default,
                #[cfg(unix)]
                TargetName::Syslog => {
                    Target::Syslog(config.syslog_facility.into())
                }
                TargetName::Stderr => Target::Stderr,
                TargetName::File => {
                    match config.log_file.as_ref() {
                        Some(LogPath::Stderr) => Target::Stderr,
                        Some(LogPath::Path(ref file)) => {
                            Target::File(file.clone().into())
                        }
                        None => {
                            error!("Missing 'log-file' option in config.");
                            return Err(Failed)
                        }
                    }
                }
            },
        })
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
        let logger = Dispatch::new(self, daemon)?;
        GLOBAL_LOGGER.switch(logger);
        log::set_max_level(self.level);
        Ok(())
    }

    /// Rotates the log file if necessary.
    pub fn rotate_log(&self) -> Result<(), Failed> {
        GLOBAL_LOGGER.rotate()
    }
}


//------------ Config --------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(rename = "log-level", alias = "log_level", default)]
    log_level: LevelName,

    #[serde(rename = "log", alias = "log_target", default)]
    log_target: TargetName,

    #[cfg(unix)]
    #[serde(rename = "syslog-facility", alias = "log_facility", default)]
    syslog_facility: unix::FacilityArg,

    #[serde(rename = "log-file", alias = "log_file")]
    log_file: Option<LogPath>,
}

impl Config {
    /// Creates the logger from a config file.
    pub fn from_config_file(file: &mut ConfigFile) -> Result<Self, Failed> {
        Ok(Self {
            log_level: file.take_from_str::<LevelName>(
                "log-level"
            )?.unwrap_or_default(),
            log_target: file.take_from_str::<TargetName>(
                "log"
            )?.unwrap_or_default(),
            #[cfg(unix)]
            syslog_facility: file.take_from_str::<unix::FacilityArg>(
                "syslog-facility"
            )?.unwrap_or_default(),
            log_file: file.take_string("log-file")?.map(Into::into),
        })
    }

    pub fn from_args(args: &Args) -> Self {
        let mut res = Self::default();
        res.apply_args(args);
        res
    }

    /// Applies the arguments to the logger.
    pub fn apply_args(&mut self, args: &Args) {
        if let Some(level) = args.opt_level() {
            self.log_level = LevelName(level)
        }

        if args.stderr {
            self.log_target = TargetName::Stderr;
        }
        else if let Some(path) = args.logfile.as_ref() {
            self.log_target = TargetName::File;
            self.log_file = Some(path.clone());
        }
        else {
            #[cfg(unix)]
            if args.syslog {
                self.log_target = TargetName::Syslog;
            }
        }

        #[cfg(unix)]
        if let Some(facility) = args.syslog_facility {
            self.syslog_facility = facility;
        }
    }

    /// Adds the configuration a config file
    pub fn add_to_config_file(&self, config: &mut ConfigFile) {
        config.insert_string("log-level", self.log_level.as_str());
        config.insert_string("log", self.log_target.as_str());
        #[cfg(unix)]
        if !self.syslog_facility.is_default() {
            config.insert_string(
                "syslog-facility",
                self.syslog_facility.as_str()
            );
        }
        if let Some(path) = self.log_file.as_ref() {
            config.insert_string(
                "log-file", path
            );
        }
    }
}


//------------ TargetName ----------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(try_from = "String", into = "&'static str")]
enum TargetName {
    #[default]
    Default,

    #[cfg(unix)]
    Syslog,
    Stderr,
    File
}

impl TargetName {
    fn as_str(self) -> &'static str {
        match self {
            TargetName::Default => "default",
            #[cfg(unix)]
            TargetName::Syslog => "syslog",
            TargetName::Stderr => "stderr",
            TargetName::File => "file",
        }
    }
}

impl From<TargetName> for &'static str {
    fn from(target: TargetName) -> Self {
        target.as_str()
    }
}

impl TryFrom<String> for TargetName {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl FromStr for TargetName {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "default" => Ok(TargetName::Default),
            #[cfg(unix)]
            "syslog" => Ok(TargetName::Syslog),
            "stderr" => Ok(TargetName::Stderr),
            "file" => Ok(TargetName::File),
            _ => Err("invalid log target")
        }
    }
}


//------------ LevelName -----------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(try_from = "String", into = "&'static str")]
struct LevelName(LevelFilter);

impl Default for LevelName {
    fn default() -> Self {
        LevelName(LevelFilter::Warn)
    }
}

impl LevelName {
    fn as_str(self) -> &'static str {
        match self.0 {
            LevelFilter::Off => "off",
            LevelFilter::Error => "error",
            LevelFilter::Warn => "warn",
            LevelFilter::Info => "info",
            LevelFilter::Debug => "debug",
            LevelFilter::Trace => "trace",
        }
    }
}

impl From<LevelName> for &'static str {
    fn from(level: LevelName) -> Self {
        level.as_str()
    }
}

impl TryFrom<String> for LevelName {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl FromStr for LevelName {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LevelFilter::from_str(s).map(Self).map_err(|_| "invalid log level")
    }
}


//------------ LogPath -------------------------------------------------------

/// A path that is either "-" for stderr or an actual path.
#[derive(Clone, Debug)]
pub enum LogPath {
    /// Standard error designated as a "-"
    Stderr,

    /// An actual path.
    Path(ConfigPath),
}

impl From<String> for LogPath {
    fn from(src: String) -> Self {
        if src == "-" {
            Self::Stderr
        }
        else {
            Self::Path(src.into())
        }
    }
}

impl<'de> Deserialize<'de> for LogPath {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D
    ) -> Result<Self, D::Error> {
        let path = String::deserialize(deserializer)?;
        if path == "-" {
            Ok(Self::Stderr)
        }
        else {
            Ok(Self::Path(path.into()))
        }
    }
}

impl Serialize for LogPath {
    fn serialize<S: Serializer>(
        &self, serializer: S
    ) -> Result<S::Ok, S::Error> {
        match self {
            Self::Stderr => "-".serialize(serializer),
            Self::Path(ref path) => path.serialize(serializer),
        }
    }
}

impl fmt::Display for LogPath {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Stderr => f.write_str("-"),
            Self::Path(ref path) => write!(f, "{}", path.display()),
        }
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
    logfile: Option<LogPath>,

    /// Facility to use for syslog logging
    #[cfg(unix)]
    #[arg(long, value_name = "FACILITY")]
    syslog_facility: Option<unix::FacilityArg>,
}

impl Args {
    pub fn to_config(&self) -> Config {
        Config::from_args(self)
    }

    fn opt_level(&self) -> Option<LevelFilter> {
        if self.verbose > 1 {
            Some(LevelFilter::Debug)
        }
        else if self.verbose == 1 {
            Some(LevelFilter::Info)
        }
        else if self.quiet > 1 {
            Some(LevelFilter::Off)
        }
        else if self.quiet == 1 {
            Some(LevelFilter::Error)
        }
        else {
            None
        }
    }
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


//------------ Dispatch ------------------------------------------------------

/// Format and write log messages.
struct Dispatch {
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

impl Dispatch {
    /// Creates a new logger from config and additional information.
    fn new(
        config: &Logger, daemon: bool,
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
    #[derive(Clone, Copy, Debug, Deserialize, Serialize)]
    #[serde(try_from = "String", into = "&'static str")]
    pub struct FacilityArg(syslog::Facility);

    impl FacilityArg {
        pub fn is_default(self) -> bool {
            matches!(self.0, syslog::Facility::LOG_DAEMON)
        }

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

    impl Default for FacilityArg {
        fn default() -> Self {
            Self(syslog::Facility::LOG_DAEMON)
        }
    }

    impl From<syslog::Facility> for FacilityArg {
        fn from(f: syslog::Facility) -> Self {
            Self(f)
        }
    }

    impl From<FacilityArg> for syslog::Facility {
        fn from(arg: FacilityArg) -> Self {
            arg.0
        }
    }

    impl From<FacilityArg> for &'static str {
        fn from(arg: FacilityArg) -> Self {
            arg.as_str()
        }
    }

    impl TryFrom<String> for FacilityArg {
        type Error = &'static str;

        fn try_from(s: String) -> Result<Self, Self::Error> {
            Self::from_str(&s)
        }
    }

    impl FromStr for FacilityArg {
        type Err = &'static str;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            syslog::Facility::from_str(s).map(Self).map_err(|_| {
                "invalid syslog facility"
            })
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
    inner: OnceLock<Dispatch>,
}

/// The static for the log crate.
static GLOBAL_LOGGER: GlobalLogger = GlobalLogger::new();

impl GlobalLogger {
    /// Creates a new provisional logger.
    const fn new() -> Self {
        GlobalLogger { inner: OnceLock::new() }
    }

    /// Switches to the proper logger.
    fn switch(&self, logger: Dispatch) {
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

