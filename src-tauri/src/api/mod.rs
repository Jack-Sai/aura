pub mod client;
pub mod error;

pub use client::{ChatStream, Fallback, MODELS, OpenRouterClient, OpenRouterProvider};
pub use error::{ApiError, ModelError};
