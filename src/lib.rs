//! Application core for Codex Dirigent.

pub mod app;
mod board;
pub mod codex;
pub mod cue;
pub mod review;
pub mod settings;
pub mod theme;
pub mod workspace;

/// Human-readable product name used in the UI and package metadata.
pub const PRODUCT_NAME: &str = "Codex Dirigent";

/// Current application version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn product_identity_is_stable() {
        assert_eq!(super::PRODUCT_NAME, "Codex Dirigent");
        assert!(!super::VERSION.is_empty());
    }
}
