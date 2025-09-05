//! Process management.

#[cfg(unix)]
pub use self::unix::{Args, Config, Process};

#[cfg(not(unix))]
pub use self::noop::{Args, Config, Process};


//============ unix ==========================================================

/// Implementation for normal Unix-style systems.
///
#[cfg(unix)]
mod unix {
    use std::io;
    use std::env::set_current_dir;
    use std::ffi::{CStr, CString};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::fd::{AsFd, AsRawFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf, StripPrefixError};
    use std::str::FromStr;
    use log::error;
    use nix::fcntl::{Flock, FlockArg, OFlag, open};
    use nix::sys::stat::Mode;
    use nix::sys::stat::umask;
    use nix::unistd::{Gid, Group, Uid, User};
    use nix::unistd::{
        close, chroot, dup2, fork, getpid, setsid,
    };
    use serde::{Deserialize, Serialize};
    use crate::config::{ConfigFile, ConfigPath};
    use crate::error::Failed;


    //-------- Process -------------------------------------------------------

    pub struct Process {
        /// All the configuration.
        config: Config,

        /// The pid file if requested.
        pid_file: Option<Flock<File>>,
    }

    impl Process {
        /// Creates the process from a config struct.
        pub fn from_config(config: Config) -> Self {
            Self { config, pid_file: None }
        }

        /// Adjusts a path for use after dropping privileges.
        ///
        /// Since [`drop_privileges`][Self::drop_privileges] may change the
        /// file system root, all absolute paths will need to be adjusted if
        /// they should be used after it is called.
        ///
        /// The method returns an error if the path is outside of what’s
        /// accessible to the process after dropping privileges.
        pub fn adjust_path(
            &self, path: PathBuf
        ) -> Result<PathBuf, StripPrefixError> {
            if let Some(chroot) = self.config.chroot.as_ref() {
                Ok(Path::new("/").join(
                    path.strip_prefix(chroot)?
                ))
            }
            else {
                Ok(path)
            }
        }

        /// Sets up the process as a daemon.
        ///
        /// If `background` is `true`, the daemon will be set up to run in
        /// the background which may involve forking.
        ///
        /// After the method returns, we will be running in the final process
        /// but still have the same privileges we were initially started with.
        /// This allows you to perform actions that require the original
        /// privileges in the forked process. Once you are done with that,
        /// call [`drop_privileges`][Self::drop_privileges] to conclude
        /// setting up the daemon.
        ///
        /// Because access to the standard streams may get lost during the
        /// method, it uses the logging facilities for any diagnostic output.
        /// You should therefore have set up your logging system prior to
        /// calling this method.
        pub fn setup_daemon(
            &mut self, background: bool
        ) -> Result<(), Failed> {
            self.create_pid_file()?;
            
            if background {
                // Fork to detach from terminal.
                self.perform_fork()?;

                // Create a new session.
                if let Err(err) = setsid() {
                    error!("Fatal: failed to crates new session: {err}");
                    return Err(Failed)
                }

                // Fork again to stop being the session leader so we can’t
                // acquire a controlling terminal on SVR4.
                self.perform_fork()?;

                // Change the working directory to either what’s configured
                // or / (so we don’t block a file system from being umounted).
                self.change_working_dir(true)?;

                // Set umask to 0 -- the mask is used “inverted,” that is,
                // everything set in the mask is removed from the actual
                // mode of a created file. Setting it to 0 allows everything.
                umask(Mode::empty());

                // Redirect the three standard streams to /dev/null.
                self.redirect_stdio()?;
            }
            else {
                self.change_working_dir(false)?;
            }

            // chown_pid_file

            Ok(())
        }


        /// Drops privileges.
        ///
        /// If requested via the config, this method will drop all potentially
        /// elevated privileges. This may include losing root or system
        /// administrator permissions and change the file system root.
        pub fn drop_privileges(&mut self) -> Result<(), Failed> {
            if let Some(path) = self.config.chroot.as_ref() {
                if let Err(err) = chroot(path.as_path()) {
                    error!("Fatal: cannot chroot to '{}': {}'",
                        path.display(), err
                    );
                    return Err(Failed)
                }
            }

            self.set_user_and_group()?;

            self.write_pid_file()?;

            Ok(())
        }

        /// Changes the user and group IDs.
        fn set_user_and_group(&self) -> Result<(), Failed> {
            // Unfortunately, this isn’t quite as portable as we want it to
            // be as most of the function we use are not available on some
            // platforms. Instead of copying the cfg attributes from the nix
            // crate, we define fallback functions and overwrite their symbol
            // if possible using a glob import.
            //
            // For setting uid and gid, we need to cascade: Use `setresuid`
            // if available, otherwise use `setreuid` if available, otherwise
            // use `setuid`; analogous for gid. We achieve this by having
            // the fallback call the next step which may itself be a fallback.

            /// Dummy fallback function for `nix::unistd::initgroups`.
            #[allow(dead_code)]
            fn initgroups(
                _user: &CStr, _group: Gid
            ) -> Result<(), nix::errno::Errno> {
                Ok(())
            }

            /// Fallback function for `nix::unistd::setresgid`.
            #[allow(dead_code)]
            fn setresgid(
                rgid: Gid, egid: Gid, _sgid: Gid
            ) -> Result<(), nix::errno::Errno> {
                use nix::libc::{c_int, gid_t};

                #[allow(dead_code)]
                unsafe fn setregid(rgid: gid_t, _egid: gid_t) -> c_int {
                    unsafe { nix::libc::setgid(rgid) }
                }

                {
                    #[allow(unused_imports)]
                    use nix::libc::*;

                    if unsafe { setregid(rgid.as_raw(), egid.as_raw()) } != 0 {
                        return Err(nix::errno::Errno::last());
                    }
                }

                Ok(())
            }

            /// Fallback function for `nix::unistd::setresuid`.
            #[allow(dead_code)]
            fn setresuid(
                ruid: Uid, euid: Uid, _suid: Uid
            ) -> Result<(), nix::errno::Errno> {
                use nix::libc::{c_int, uid_t};

                #[allow(dead_code)]
                unsafe fn setreuid(ruid: uid_t, _euid: uid_t) -> c_int {
                    unsafe { nix::libc::setuid(ruid) }
                }

                {
                    #[allow(unused_imports)]
                    use nix::libc::*;

                    if unsafe { setreuid(ruid.as_raw(), euid.as_raw()) } != 0 {
                        return Err(nix::errno::Errno::last());
                    }
                }

                Ok(())
            }

            let Some(user) = self.config.user.as_ref() else {
                return Ok(())
            };

            // If we don’t have an explicit group, we use the user’s group.
            let gid = self.config.group.as_ref().map(|g| {
                g.gid
            }).unwrap_or_else(|| {
                user.gid
            });

            // Let the system load the supplemental groups for the user.
            {
                #[allow(unused_imports)]
                use nix::unistd::*;

                initgroups(&user.c_name, gid).map_err(|err| {
                    error!(
                        "failed to initialize the group access list: {err}",
                    );
                    Failed
                })?;
            }

            // Set the group ID.
            {
                #[allow(unused_imports)]
                use nix::unistd::*;

                setresgid(gid, gid, gid).map_err(|err| {
                    error!(
                        "failed to set group ID: {err}"
                    );
                    Failed
                })?;
            }

            // Set the user ID.
            {
                #[allow(unused_imports)]
                use nix::unistd::*;

                setresuid(user.uid, user.uid, user.uid).map_err(|err| {
                    error!(
                        "failed to set user ID: {err}"
                    );
                    Failed
                })?;
            }

            Ok(())
        }

        /// Creates the pid file if requested.
        fn create_pid_file(&mut self) -> Result<(), Failed> {
            let path = match self.config.pid_file.as_ref() {
                Some(path) => path,
                None => return Ok(())
            };

            let file = OpenOptions::new()
                .read(false).write(true)
                .create(true).truncate(true)
                .mode(0o666)
                .open(path);
            let file = match file {
                Ok(file) => file,
                Err(err) => {
                    error!("Fatal: failed to create PID file {}: {}",
                        path.display(), err
                    );
                    return Err(Failed)
                }
            };
            let file = match Flock::lock(
                file, FlockArg::LockExclusiveNonblock
            ) {
                Ok(file) => file,
                Err((_, err)) => {
                    error!("Fatal: cannot lock PID file {}: {}",
                        path.display(), err
                    );
                    return Err(Failed)
                }
            };
            self.pid_file = Some(file);
            Ok(())
        }

        /// Updates the pid in the pid file after forking.
        fn write_pid_file(&mut self) -> Result<(), Failed> {
            if let Some(pid_file) = self.pid_file.as_mut() {
                let pid = format!("{}", getpid());
                if let Err(err) = pid_file.write_all(pid.as_bytes()) {
                    error!(
                        "Fatal: failed to write PID to PID file: {err}"
                    );
                    return Err(Failed)
                }
            }
            Ok(())
        }

        /// Peforms a fork and exits the parent process.
        fn perform_fork(&self) -> Result<(), Failed> {
            match unsafe { fork() } {
                Ok(res) => {
                    if res.is_parent() {
                        std::process::exit(0)
                    }
                    Ok(())
                }
                Err(err) => {
                    error!("Fatal: failed to detach: {err}");
                    Err(Failed)
                }
            }
        }

        /// Changes the current working directory in necessary.
        fn change_working_dir(&self, background: bool) -> Result<(), Failed> {
            let mut path = self.config.working_dir.as_ref().or(
                self.config.chroot.as_ref()
            ).map(ConfigPath::as_path);
            if background {
                path = path.or(Some(Path::new("/")));
            }
            if let Some(path) = path {
                if let Err(err) = set_current_dir(path) {
                    error!("Fatal: failed to set working directory {}: {}",
                        path.display(), err
                    );
                    return Err(Failed)
                }
            }

            Ok(())
        }

        /// Changes the stdio streams to /dev/null.
        fn redirect_stdio(&self) -> Result<(), Failed> {
            let dev_null = match open(
                "/dev/null", OFlag::O_RDWR,
                Mode::empty()
            ) {
                Ok(fd) => fd,
                Err(err) => {
                    error!("Fatal: failed to open /dev/null: {err}");
                    return Err(Failed)
                }
            };

            if let Err(err) = dup2(dev_null, io::stdin().as_fd().as_raw_fd()) {
                error!(
                    "Fatal: failed to redirect stdio to /dev/null: {err}"
                );
                return Err(Failed)
            }
            if let Err(err) = dup2(dev_null, io::stdout().as_fd().as_raw_fd()) {
                error!(
                    "Fatal: failed to redirect stdout to /dev/null: {err}"
                );
                return Err(Failed)
            }
            if let Err(err) = dup2(dev_null, io::stderr().as_fd().as_raw_fd()) {
                error!(
                    "Fatal: failed to redirect stderr to /dev/null: {err}"
                );
                return Err(Failed)
            }

            if let Err(err) = close(dev_null) {
                error!(
                    "Fatal: failed to close /dev/null: {err}"
                );
                return Err(Failed)
            }

            Ok(())
        }
    }


    //-------- Config --------------------------------------------------------

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct Config {
        /// The optional PID file for server mode.
        #[serde(rename = "pid-file")]
        pid_file: Option<ConfigPath>,

        /// The optional working directory for server mode.
        #[serde(rename = "working-dir")]
        working_dir: Option<ConfigPath>,

        /// The optional directory to chroot to in server mode.
        chroot: Option<ConfigPath>,

        /// The name of the user to change to in server mode.
        user: Option<UserId>,

        /// The name of the group to change to in server mode.
        group: Option<GroupId>,
    }

    impl Config {
        pub fn from_config_file(
            file: &mut ConfigFile
        ) -> Result<Self, Failed> {
            Ok(Config {
                pid_file: file.take_path("pid-file")?,
                working_dir: file.take_path("working-dir")?,
                chroot: file.take_path("chroot")?,
                user: file.take_from_str("user")?,
                group: file.take_from_str("group")?,
            })
        }

        /// Creates the process from command line arguments only.
        pub fn from_args(args: Args) -> Self {
            Config {
                pid_file: args.pid_file,
                working_dir: args.working_dir,
                chroot: args.chroot,
                user: args.user,
                group: args.group,
            }
        }

        /// Applies the arguments to the process.
        pub fn apply_args(&mut self, args: Args) {
            if let Some(pid_file) = args.pid_file {
                self.pid_file = Some(pid_file)
            }
            if let Some(working_dir) = args.working_dir {
                self.working_dir = Some(working_dir)
            }
            if let Some(chroot) = args.chroot {
                self.chroot = Some(chroot)
            }
            if let Some(user) = args.user {
                self.user = Some(user)
            }
            if let Some(group) = args.group {
                self.group = Some(group)
            }
        }

        pub fn with_pid_file(mut self, v: ConfigPath) -> Self {
            self.pid_file = Some(v);
            self
        }

        pub fn with_working_dir(mut self, v: ConfigPath) -> Self {
            self.working_dir = Some(v);
            self
        }

        pub fn with_chroot(mut self, v: ConfigPath) -> Self {
            self.chroot = Some(v);
            self
        }

        pub fn with_user(mut self, v: &str) -> Result<Self, String> {
            self.user = Some(UserId::from_str(v)?);
            Ok(self)
        }

        pub fn with_group(mut self, v: &str) -> Result<Self, String> {
            self.group = Some(GroupId::from_str(v)?);
            Ok(self)
        }
    }


    //-------- Args ----------------------------------------------------------

    #[derive(Clone, Debug, clap::Args)]
    #[group(id = "process-args")]
    pub struct Args {
        /// The file for keep the daemon process's PID in
        #[arg(long, value_name = "PATH")]
        pid_file: Option<ConfigPath>,

        /// The working directory of the daemon process
        #[arg(long, value_name = "PATH")]
        working_dir: Option<ConfigPath>,

        /// Root directory for the daemon process
        #[arg(long, value_name = "PATH")]
        chroot: Option<ConfigPath>,

        /// User for the daemon process
        #[arg(long, value_name = "UID")]
        user: Option<UserId>,

        /// Group for the daemon process
        #[arg(long, value_name = "GID")]
        group: Option<GroupId>,
    }

    impl Args {
        pub fn into_config(self) -> Config {
            Config::from_args(self)
        }
    }


    //-------- UserId --------------------------------------------------------

    /// A user ID in configuration.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(try_from = "String", into = "String", expecting = "a user name")]
    struct UserId {
        /// The user name.
        ///
        /// This is used for error reporting.
        name: String,

        /// The user name as a C string.
        ///
        /// This is used internally. We keep both the string and C string
        /// versions because conversion can cause errors, so it best happens
        /// already when creating an object.
        c_name: CString,

        /// The numerical user ID.
        uid: Uid,

        /// The numerical group ID of the user.
        gid: Gid,

    }

    impl TryFrom<String> for UserId {
        type Error = String;

        fn try_from(name: String) -> Result<Self, Self::Error> {
            let Ok(c_name) = CString::new(name.clone()) else {
                return Err(format!("invalid user name '{name}'"))
            };
            match User::from_name(&name) {
                Ok(Some(user)) => {
                    Ok(UserId {
                        name, c_name,
                        gid: user.gid, uid: user.uid
                    })
                }
                Ok(None) => {
                    Err(format!("unknown user '{name}'"))
                }
                Err(err) => {
                    Err(format!("failed to resolve user '{name}': {err}"))
                }
            }
        }
    }

    impl FromStr for UserId {
        type Err = String;

        fn from_str(name: &str) -> Result<Self, Self::Err> {
            String::from(name).try_into()
        }
    }

    impl From<UserId> for String {
        fn from(user: UserId) -> Self {
            user.name
        }
    }


    //-------- GroupId -------------------------------------------------------

    /// A user ID in configuration.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(try_from = "String", into = "String", expecting = "a user name")]
    struct GroupId {
        /// The group name.
        name: String,

        /// The numerical group ID.
        gid: Gid,
    }

    impl TryFrom<String> for GroupId {
        type Error = String;

        fn try_from(name: String) -> Result<Self, Self::Error> {
            match Group::from_name(&name) {
                Ok(Some(group)) => {
                    Ok(GroupId { gid: group.gid, name })
                }
                Ok(None) => {
                    Err(format!("unknown group '{name}'"))
                }
                Err(err) => {
                    Err(format!("failed to resolve group '{name}': {err}"))
                }
            }
        }
    }

    impl FromStr for GroupId {
        type Err = String;

        fn from_str(name: &str) -> Result<Self, Self::Err> {
            String::from(name).try_into()
        }
    }

    impl From<GroupId> for String {
        fn from(user: GroupId) -> Self {
            user.name
        }
    }
}


//============ noop ==========================================================

/// ‘Empty’ implementation for systems we don’t really support.
///
#[cfg(not(unix))]
mod noop {
    use std::path::{PathBuf, StripPrefixError};
    use serde::{Deserialize, Serialize};
    use crate::config::ConfigFile;
    use crate::error::Failed;


    //-------- Process -------------------------------------------------------

    pub struct Process;

    impl Process {
        /// Creates the process from a config struct.
        pub fn from_config(config: Config) -> Self {
            let _ = config;
            Self
        }

        /// Adjusts a path for use after dropping privileges.
        ///
        /// Since [`drop_privileges`][Self::drop_privileges] may change the
        /// file system root, all absolute paths will need to be adjusted if
        /// they should be used after it is called.
        ///
        /// The method returns an error if the path is outside of what’s
        /// accessible to the process after dropping privileges.
        pub fn adjust_path(
            &self, path: PathBuf
        ) -> Result<PathBuf, StripPrefixError> {
            Ok(path)
        }

        /// Sets up the process as a daemon.
        ///
        /// If `background` is `true`, the daemon will be set up to run in
        /// the background which may involve forking.
        ///
        /// After the method returns, we will be running in the final process
        /// but still have the same privileges we were initially started with.
        ///
        /// Because access to the standard streams may get lost during the
        /// method, it uses the logging facilities for any diagnostic output.
        /// You should therefore have set up your logging system prioir to
        /// calling this method.
        pub fn setup_daemon(
            &mut self, background: bool
        ) -> Result<(), Failed> {
            let _ = background;
            Ok(())
        }

        /// Drops privileges.
        ///
        /// If requested via the config, this method will drop all potentially
        /// elevated privileges. This may include loosing root or system
        /// administrator permissions and change the file system root.
        pub fn drop_privileges(&mut self) -> Result<(), Failed> {
            Ok(())
        }
    }


    //-------- Config --------------------------------------------------------

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct Config;

    impl Config {
        /// Creates the proces from a config file.
        pub fn from_config_file(
            file: &mut ConfigFile
        ) -> Result<Self, Failed> {
            let _ = file;
            Ok(Self)
        }

        /// Creates the process from command line arguments only.
        pub fn from_args(args: Args) -> Self {
            let _ = args;
            Self
        }

        /// Applies the arguments to the process.
        pub fn apply_args(&mut self, args: Args) {
            let _ = args;
        }

        pub fn with_pid_file(self, _: ConfigPath) -> Self {
            self
        }

        pub fn with_working_dir(self, _: ConfigPath) -> Self {
            self
        }

        pub fn with_chroot(self, _: ConfigPath) -> Self {
            self
        }

        pub fn with_user(self, _: &str) -> Result<Self, String> {
            Ok(self)
        }

        pub fn with_group(self, _: &str) -> Result<Self, String> {
            Ok(self)
        }
    }

    //-------- Args ----------------------------------------------------------

    #[derive(Clone, Debug, clap::Args)]
    #[group(id = "process-args")]
    pub struct Args;

    impl Args {
        pub fn into_config(&self) -> Config {
            Config
        }
    }
}

