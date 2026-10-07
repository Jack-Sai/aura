pub mod client;
pub mod error;
pub mod sse;

pub use client::{Fallback, MODELS, OpenRouterClient, OpenRouterProvider};
pub use error::{ApiError, ModelError};
pub use sse::{ChatStream, SseParser};
