//! Michelangelo Core — headless application layer.
//!
//! Responsible for command routing, service coordination, and
//! exposing the core API to transport layers (stdio, HTTP, etc.).

pub mod service;

pub use service::CoreService;

/// Core crate version string.
pub fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_version_is_not_empty() {
        assert!(!core_version().is_empty());
    }
}
