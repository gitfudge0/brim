//! Core domain models for Brim.
//!
//! This crate defines the shared types used across all other crates:
//! provider identity, quota/usage snapshots, confidence labels,
//! time windows, and the provider trait.

pub mod confidence;
pub mod error;
pub mod history;
pub mod models;
pub mod provider;
pub mod time_window;
