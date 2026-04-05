//! Authentication utilities shared across providers.
//!
//! Provides reusable building blocks:
//! - OAuth2 PKCE flow helpers
//! - GitHub device flow implementation
//! - Local file credential discovery
//! - Loopback callback server

pub mod credential;
pub mod device_flow;
pub mod error;
pub mod local_files;
