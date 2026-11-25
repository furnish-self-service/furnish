//! furnish: self-service app catalog. Shared by the CLI (`src/main.rs`) and the GUI (`gui/`).
pub mod auth;
pub mod catalog;
pub mod check;
mod conditions;
mod diagnostics;
pub mod endpoint;
pub mod engine;
mod installer_download;
pub mod load;
pub mod locale;
pub mod nixos;
pub mod nixos_apps;
pub mod options;
mod nix_profile;
mod package_names;
mod package_receipts;
mod script_managers;
pub(crate) mod platform;
pub mod policy;
pub mod reports;
pub mod resources;
pub mod required;
pub mod selfupdate;
pub mod resource_env;
pub mod service;
pub mod settings;
pub mod versions;

/// Print a line of CLI output. A reader such as `head` can close stdout while the
/// command still has work to finish or an operation error to report on stderr.
#[macro_export]
macro_rules! cli_println {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(err) = writeln!(std::io::stdout().lock(), $($arg)*)
            && err.kind() != std::io::ErrorKind::BrokenPipe
        {
            panic!("failed printing to stdout: {err}");
        }
    }};
}
