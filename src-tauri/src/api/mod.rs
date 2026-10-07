pub mod client;
pub mod error;
pub mod provider;
pub mod sse;

pub use client::{Fallback, MODELS, OpenRouterClient, OpenRouterProvider};
pub use error::{ApiError, ModelError};
pub use provider::{ChatProvider, ProbeStatus, ProviderKind, RemoteModel};
pub use sse::{ChatStream, SseParser};
