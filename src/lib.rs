//! Foundational utilities for system daemons.
//!
//! Daemons (as they are known on Unix-based systems) are background processes
//! that provide continuous services, e.g. monitoring the system, playing
//! music, and serving websites. This class of programs have many shared
//! concerns: they usually load configuration files, interact with system
//! privileges, and log events. `daemonbase` offers simple but opinionated
//! solutions to these needs.
//
// TODO: Document usage.
// TODO: Document portability.

pub mod config;
pub mod error;
pub mod logging;
pub mod process;
