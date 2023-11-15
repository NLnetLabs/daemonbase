/// Error types used by multiple modules.
///
/// There are two error types that are used widely within the Routinator
/// library.
///
/// The most important is [`Failed`]. This error indicates that an
/// operation had to be canceled for some reason and callers can assume
/// that all diagnostic information has been logged and they need not do
/// anything further.
///
/// Secondly, [`ExitError`] is used when the program should be terminated. It
/// provides enough information to determine the exit code of the program.

use log::error;


//------------ Failed --------------------------------------------------------

/// An operation has failed to complete.
///
/// This error types is used to indicate that an operation has failed,
/// diagnostic information has been printed or logged, and the caller can’t
/// really do anything to recover.
#[derive(Clone, Copy, Debug)]
pub struct Failed;


//------------ ExitError -----------------------------------------------------

/// An error happened that should lead to terminating the program.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExitError(());

impl ExitError {
    pub fn exit() -> ! {
        std::process::exit(1);
    }
}

impl From<Failed> for ExitError {
    fn from(_: Failed) -> ExitError {
        error!("Fatal error. Exiting.");
        ExitError::default()
    }
}

