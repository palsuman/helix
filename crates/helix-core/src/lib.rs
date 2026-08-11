//! `helix-core` — shared types and traits used across the Helix kernel.
//!
//! This crate has no dependency on Tauri, the frontend, or IPC transport. It
//! defines the vocabulary (error types, service traits, health model) that
//! every other crate in the workspace builds on.

pub mod container;
pub mod error;
pub mod health;

pub use container::{
    HealthCheck, Lifetime, ManagedHandle, ManagedService, MockServiceProvider, Service,
    ServiceContainer, ServiceContext, ServiceError, ServiceProvider,
};
pub use error::{AppError, ErrorCategory};
pub use health::{ServiceHealth, ServiceMetrics};
