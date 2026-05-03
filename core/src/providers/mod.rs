// core/src/providers/mod.rs
//! Provider system for LLM integrations
//! 
//! Provides extensible traits and implementations for:
//! - Token counting (tiktoken, anthropic, naive)
//! - Text summarization (OpenAI, Anthropic, echo)
//! - Text embeddings (OpenAI, local, null)

pub mod tokenizer;
pub mod summarizer;
pub mod embedder;

use crate::error::LcmResult;
use crate::types::*;

// Re-export main traits and implementations
pub use tokenizer::{TokenCounter, create_token_counter, NaiveTokenCounter, TiktokenCounter, AnthropicTokenCounter};
pub use summarizer::{Summarizer, create_summarizer, EchoSummarizer, OpenAISummarizer, AnthropicSummarizer};
pub use embedder::{Embedder, create_embedder, NullEmbedder, OpenAIEmbedder, LocalEmbedder};

/// Provider registry for managing multiple providers
pub struct ProviderRegistry {
    pub token_counter: Box<dyn TokenCounter>,
    pub summarizer: Box<dyn Summarizer>,
    pub embedder: Box<dyn Embedder>,
}

impl ProviderRegistry {
    pub fn new(
        token_counter: Box<dyn TokenCounter>,
        summarizer: Box<dyn Summarizer>,
        embedder: Box<dyn Embedder>,
    ) -> Self {
        Self {
            token_counter,
            summarizer,
            embedder,
        }
    }
}
