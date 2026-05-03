// core/src/providers/embedder.rs
use crate::error::{ProviderError, ProviderResult};
use crate::types::MessageId;
use async_trait::async_trait;

/// Trait for text embedding operations
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Generate embedding for text
    async fn embed(&self, text: &str) -> ProviderResult<Vec<f32>>;
    
    /// Get embedding dimensions
    fn dimensions(&self) -> usize;
    
    /// Get the name/model of this embedder
    fn name(&self) -> &str;
}

/// Null embedder (no embeddings)
#[derive(Debug)]
pub struct NullEmbedder;

impl NullEmbedder {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NullEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Embedder for NullEmbedder {
    async fn embed(&self, _text: &str) -> ProviderResult<Vec<f32>> {
        Err(ProviderError::ConfigError("Null embedder does not generate embeddings".to_string()))
    }
    
    fn dimensions(&self) -> usize {
        0
    }
    
    fn name(&self) -> &str {
        "null"
    }
}

/// OpenAI embedder (stub - not yet fully implemented)
#[derive(Debug)]
pub struct OpenAIEmbedder;

#[async_trait]
impl Embedder for OpenAIEmbedder {
    async fn embed(&self, _text: &str) -> ProviderResult<Vec<f32>> {
        Err(ProviderError::ConfigError("OpenAI embedder not yet implemented".to_string()))
    }
    
    fn dimensions(&self) -> usize {
        1536
    }
    
    fn name(&self) -> &str {
        "openai"
    }
}

/// Local embedder (stub - not yet fully implemented)
#[derive(Debug)]
pub struct LocalEmbedder;

#[async_trait]
impl Embedder for LocalEmbedder {
    async fn embed(&self, _text: &str) -> ProviderResult<Vec<f32>> {
        Err(ProviderError::ConfigError("Local embedder not yet implemented".to_string()))
    }
    
    fn dimensions(&self) -> usize {
        0
    }
    
    fn name(&self) -> &str {
        "local"
    }
}

/// Factory function to create appropriate embedder
pub fn create_embedder(
    provider: &str,
    _model: Option<String>,
    _base_url: Option<String>,
    _api_key: Option<String>,
    _dimensions: Option<usize>,
) -> ProviderResult<Box<dyn Embedder>> {
    match provider {
        "null" => Ok(Box::new(NullEmbedder::default())),
        "openai" => {
            // TODO: Implement OpenAI embedder
            Err(ProviderError::ConfigError("OpenAI embedder not yet implemented".to_string()))
        }
        "local" => {
            // TODO: Implement local embedder
            Err(ProviderError::ConfigError("Local embedder not yet implemented".to_string()))
        }
        _ => Err(ProviderError::ConfigError(format!("Unknown embedder provider: {}", provider))),
    }
}

/// Search result from vector store
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub message_id: MessageId,
    pub score: f32,
    pub message: Option<crate::types::Message>,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_null_embedder() {
        let embedder = NullEmbedder::default();
        assert_eq!(embedder.name(), "null");
        assert_eq!(embedder.dimensions(), 0);
        
        let result = embedder.embed("test").await;
        assert!(result.is_err());
    }
    
    #[test]
    fn test_factory() {
        let null_embedder = create_embedder("null", None, None, None, None).unwrap();
        assert_eq!(null_embedder.name(), "null");
        
        let result = create_embedder("unknown", None, None, None, None);
        assert!(result.is_err());
    }
}
