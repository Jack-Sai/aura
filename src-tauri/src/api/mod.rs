pub mod client;
pub mod compat;
pub mod config;
pub mod error;
pub mod provider;
pub mod router;
pub mod sse;

pub use client::{Fallback, MODELS};
pub use compat::{azure_chat_endpoint, chat_endpoint, CompatProvider};
pub use config::{model_key, parse_model_key, ModelConfig, ProviderConfig, RouterConfig};
pub use error::{ApiError, ModelError};
pub use provider::{ChatProvider, ProbeStatus, ProviderKind, RemoteModel};
pub(crate) use router::build_executor;
pub use router::Router;
pub use sse::{ChatStream, SseParser};
