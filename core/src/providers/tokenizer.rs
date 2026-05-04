// core/src/providers/tokenizer.rs
use crate::error::{ProviderError, ProviderResult};
use async_trait::async_trait;

/// Trait for token counting operations
#[async_trait]
pub trait TokenCounter: Send + Sync {
    /// Count tokens in a text string
    async fn count(&self, text: &str) -> ProviderResult<usize>;
    
    /// Get the name/model of this tokenizer
    fn name(&self) -> &str;
}

/// Naive token counter (rough approximation)
#[derive(Debug)]
pub struct NaiveTokenCounter {
    chars_per_token: f32,
}

impl NaiveTokenCounter {
    pub fn new(chars_per_token: f32) -> Self {
        Self { chars_per_token }
    }
}

impl Default for NaiveTokenCounter {
    fn default() -> Self {
        Self::new(4.0) // Standard approximation
    }
}

#[async_trait]
impl TokenCounter for NaiveTokenCounter {
    async fn count(&self, text: &str) -> ProviderResult<usize> {
        Ok((text.len() as f32 / self.chars_per_token).ceil() as usize)
    }
    
    fn name(&self) -> &str {
        "naive"
    }
}

/// Tiktoken-based token counter for OpenAI models
#[derive(Debug)]
pub struct TiktokenCounter {
    encoding: tiktoken_rs::CoreBPE,
    model_name: String,
}

impl TiktokenCounter {
    pub fn new(model_name: &str) -> ProviderResult<Self> {
        // Try as an encoding name first (e.g. "cl100k_base"), then as a model name
        let encoding = match model_name {
            "cl100k_base" => tiktoken_rs::cl100k_base()
                .map_err(|e| ProviderError::ConfigError(format!("Failed to load tiktoken encoding: {}", e)))?,
            "r50k_base" => tiktoken_rs::r50k_base()
                .map_err(|e| ProviderError::ConfigError(format!("Failed to load tiktoken encoding: {}", e)))?,
            "p50k_base" => tiktoken_rs::p50k_base()
                .map_err(|e| ProviderError::ConfigError(format!("Failed to load tiktoken encoding: {}", e)))?,
            "o200k_base" => tiktoken_rs::o200k_base()
                .map_err(|e| ProviderError::ConfigError(format!("Failed to load tiktoken encoding: {}", e)))?,
            other => tiktoken_rs::get_bpe_from_model(other)
                .map_err(|e| ProviderError::ConfigError(format!("Failed to load tiktoken encoding for '{}': {}", other, e)))?,
        };
        
        Ok(Self {
            encoding,
            model_name: model_name.to_string(),
        })
    }
    
    pub fn for_model(model: &str) -> ProviderResult<Self> {
        let encoding = tiktoken_rs::get_bpe_from_model(model)
            .map_err(|e| ProviderError::ConfigError(format!("Failed to get encoding for model {}: {}", model, e)))?;
        
        Ok(Self {
            encoding,
            model_name: model.to_string(),
        })
    }
}

#[async_trait]
impl TokenCounter for TiktokenCounter {
    async fn count(&self, text: &str) -> ProviderResult<usize> {
        Ok(self.encoding.encode_with_special_tokens(text).len())
    }
    
    fn name(&self) -> &str {
        &self.model_name
    }
}

/// Anthropic-calibrated token counter
#[derive(Debug)]
pub struct AnthropicTokenCounter {
    chars_per_token: f32,
}

impl AnthropicTokenCounter {
    pub fn new() -> Self {
        Self { chars_per_token: 3.4 } // Anthropic's approximate ratio
    }
}

impl Default for AnthropicTokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TokenCounter for AnthropicTokenCounter {
    async fn count(&self, text: &str) -> ProviderResult<usize> {
        Ok((text.len() as f32 / self.chars_per_token).ceil() as usize)
    }
    
    fn name(&self) -> &str {
        "anthropic"
    }
}

/// Factory function to create appropriate token counter
pub fn create_token_counter(provider: &str, model: Option<&str>) -> ProviderResult<Box<dyn TokenCounter>> {
    match provider {
        "tiktoken" => {
            let model_name = model.unwrap_or("cl100k_base");
            Ok(Box::new(TiktokenCounter::new(model_name)?))
        }
        "anthropic" => Ok(Box::new(AnthropicTokenCounter::default())),
        "naive" => Ok(Box::new(NaiveTokenCounter::default())),
        "auto" => {
            // Auto-select based on model if provided
            if let Some(model_name) = model {
                if model_name.starts_with("gpt-") || model_name.contains("openai") {
                    Ok(Box::new(TiktokenCounter::for_model(model_name)?))
                } else if model_name.starts_with("claude-") {
                    Ok(Box::new(AnthropicTokenCounter::default()))
                } else {
                    Ok(Box::new(NaiveTokenCounter::default()))
                }
            } else {
                Ok(Box::new(NaiveTokenCounter::default()))
            }
        }
        _ => Err(ProviderError::ConfigError(format!("Unknown tokenizer provider: {}", provider))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_naive_token_counter() {
        let counter = NaiveTokenCounter::new(4.0);
        let count = counter.count("Hello world!").await.unwrap();
        assert_eq!(count, 3); // "Hello world!" is 12 chars / 4 = 3
    }
    
    #[tokio::test]
    async fn test_anthropic_token_counter() {
        let counter = AnthropicTokenCounter::new();
        let count = counter.count("Hello world!").await.unwrap();
        assert!(count > 0);
    }
    
    #[tokio::test]
    async fn test_tiktoken_counter() {
        let counter = TiktokenCounter::new("cl100k_base").unwrap();
        let count = counter.count("Hello world!").await.unwrap();
        assert!(count > 0);
    }
    
    #[tokio::test]
    async fn test_tiktoken_counter_name_returns_model() {
        let counter = TiktokenCounter::new("cl100k_base").unwrap();
        assert_eq!(counter.name(), "cl100k_base");
        
        let counter = TiktokenCounter::for_model("gpt-4").unwrap();
        assert_eq!(counter.name(), "gpt-4");
    }
    
    #[tokio::test]
    async fn test_empty_text() {
        let naive = NaiveTokenCounter::default();
        assert_eq!(naive.count("").await.unwrap(), 0);
        
        let tiktoken = TiktokenCounter::new("cl100k_base").unwrap();
        assert_eq!(tiktoken.count("").await.unwrap(), 0);
        
        let anthropic = AnthropicTokenCounter::new();
        assert_eq!(anthropic.count("").await.unwrap(), 0);
    }
    
    #[tokio::test]
    async fn test_factory() {
        let naive = create_token_counter("naive", None).unwrap();
        assert_eq!(naive.name(), "naive");
        
        let anthropic = create_token_counter("anthropic", None).unwrap();
        assert_eq!(anthropic.name(), "anthropic");
        
        let auto_gpt = create_token_counter("auto", Some("gpt-4")).unwrap();
        assert_eq!(auto_gpt.name(), "gpt-4");
        
        let auto_claude = create_token_counter("auto", Some("claude-3")).unwrap();
        assert_eq!(auto_claude.name(), "anthropic");
    }
    
    #[tokio::test]
    async fn test_factory_tiktoken_path() {
        // Explicit tiktoken with default encoding
        let counter = create_token_counter("tiktoken", None).unwrap();
        assert_eq!(counter.name(), "cl100k_base");
        
        // Explicit tiktoken with specific encoding
        let counter = create_token_counter("tiktoken", Some("o200k_base")).unwrap();
        assert_eq!(counter.name(), "o200k_base");
        
        // Explicit tiktoken with model name
        let counter = create_token_counter("tiktoken", Some("gpt-4")).unwrap();
        assert_eq!(counter.name(), "gpt-4");
    }
    
    #[tokio::test]
    async fn test_factory_auto_fallback() {
        // Auto with no model falls back to naive
        let counter = create_token_counter("auto", None).unwrap();
        assert_eq!(counter.name(), "naive");
        
        // Auto with unknown model falls back to naive
        let counter = create_token_counter("auto", Some("unknown-model")).unwrap();
        assert_eq!(counter.name(), "naive");
    }
    
    #[test]
    fn test_factory_unknown_provider() {
        let result = create_token_counter("nonexistent", None);
        assert!(result.is_err());
    }
}
