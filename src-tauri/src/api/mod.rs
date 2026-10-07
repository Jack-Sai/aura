pub mod client;
pub mod compat;
pub mod config;
pub mod error;
pub mod provider;
pub mod router;
pub mod sse;

pub use client::{Fallback, MODELS, OpenRouterClient, OpenRouterProvider};
pub use compat::{chat_endpoint, azure_chat_endpoint, CompatProvider};
pub use config::{model_key, parse_model_key, ModelConfig, ProviderConfig, RouterConfig};
pub use error::{ApiError, ModelError};
pub use provider::{ChatProvider, ProbeStatus, ProviderKind, RemoteModel};
pub use router::Router;
pub use sse::{ChatStream, SseParser};
