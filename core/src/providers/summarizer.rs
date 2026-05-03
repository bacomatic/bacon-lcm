// core/src/providers/summarizer.rs
use crate::error::{ProviderError, ProviderResult};
use crate::types::Message;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Trait for text summarization operations
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Summarize a collection of messages
    async fn summarize(&self, messages: &[Message]) -> ProviderResult<String>;
    
    /// Get the name/model of this summarizer
    fn name(&self) -> &'static str;
    
    /// Get maximum context length for this summarizer
    fn max_context_length(&self) -> usize;
}

/// Echo summarizer for testing (just concatenates messages)
#[derive(Debug)]
pub struct EchoSummarizer {
    max_context_length: usize,
}

impl EchoSummarizer {
    pub fn new(max_context_length: usize) -> Self {
        Self { max_context_length }
    }
}

impl Default for EchoSummarizer {
    fn default() -> Self {
        Self::new(100000) // Large default for testing
    }
}

#[async_trait]
impl Summarizer for EchoSummarizer {
    async fn summarize(&self, messages: &[Message]) -> ProviderResult<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }
        
        let mut summary = String::new();
        summary.push_str("=== SUMMARY ===\n\n");
        
        for (i, message) in messages.iter().enumerate() {
            let role_str = match message.role {
                crate::types::MessageRole::User => "user",
                crate::types::MessageRole::Assistant => "assistant",
                crate::types::MessageRole::System => "system",
                crate::types::MessageRole::Tool => "tool",
            };
            summary.push_str(&format!("{}. [{}] {}\n", i + 1, role_str, message.content));
        }
        
        summary.push_str("\n=== END SUMMARY ===");
        
        Ok(summary)
    }
    
    fn name(&self) -> &'static str {
        "echo"
    }
    
    fn max_context_length(&self) -> usize {
        self.max_context_length
    }
}

/// OpenAI-compatible summarizer
#[derive(Debug)]
pub struct OpenAISummarizer {
    client: reqwest::Client,
    model: String,
    base_url: String,
    api_key: String,
    max_tokens: usize,
    temperature: f32,
    max_context_length: usize,
}

impl OpenAISummarizer {
    pub fn new(
        model: String,
        base_url: Option<String>,
        api_key: String,
        max_tokens: Option<usize>,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model,
            api_key,
            max_tokens: max_tokens.unwrap_or(1024),
            temperature: temperature.unwrap_or(0.3),
            max_context_length: 128000, // Default for GPT-4
        }
    }
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    max_tokens: usize,
    temperature: f32,
}

#[derive(Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[async_trait]
impl Summarizer for OpenAISummarizer {
    async fn summarize(&self, messages: &[Message]) -> ProviderResult<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }
        
        let openai_messages: Vec<OpenAIMessage> = messages
            .iter()
            .map(|m| OpenAIMessage {
                role: match m.role {
                    crate::types::MessageRole::User => "user".to_string(),
                    crate::types::MessageRole::Assistant => "assistant".to_string(),
                    crate::types::MessageRole::System => "system".to_string(),
                    crate::types::MessageRole::Tool => "user".to_string(), // Map tool to user for summarization
                },
                content: m.content.clone(),
            })
            .collect();
        
        let request = OpenAIRequest {
            model: self.model.clone(),
            messages: openai_messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };
        
        let response = self
            .client
            .post(&format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(ProviderError::ApiError(format!("OpenAI API error: {}", error_text)));
        }
        
        let openai_response: OpenAIResponse = response.json().await?;
        
        if let Some(choice) = openai_response.choices.first() {
            Ok(choice.message.content.clone())
        } else {
            Err(ProviderError::InvalidResponse("No choices in OpenAI response".to_string()))
        }
    }
    
    fn name(&self) -> &'static str {
        "openai"
    }
    
    fn max_context_length(&self) -> usize {
        self.max_context_length
    }
}

/// Anthropic summarizer
#[derive(Debug)]
pub struct AnthropicSummarizer {
    client: reqwest::Client,
    model: String,
    api_key: String,
    max_tokens: usize,
    max_context_length: usize,
}

impl AnthropicSummarizer {
    pub fn new(
        model: String,
        api_key: String,
        max_tokens: Option<usize>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            model,
            api_key,
            max_tokens: max_tokens.unwrap_or(1024),
            max_context_length: 200000, // Default for Claude 3
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

#[async_trait]
impl Summarizer for AnthropicSummarizer {
    async fn summarize(&self, messages: &[Message]) -> ProviderResult<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }
        
        let anthropic_messages: Vec<AnthropicMessage> = messages
            .iter()
            .map(|m| AnthropicMessage {
                role: match m.role {
                    crate::types::MessageRole::User => "user".to_string(),
                    crate::types::MessageRole::Assistant => "assistant".to_string(),
                    crate::types::MessageRole::System => "user".to_string(), // Map system to user
                    crate::types::MessageRole::Tool => "user".to_string(), // Map tool to user
                },
                content: m.content.clone(),
            })
            .collect();
        
        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            messages: anthropic_messages,
        };
        
        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .json(&request)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(ProviderError::ApiError(format!("Anthropic API error: {}", error_text)));
        }
        
        let anthropic_response: AnthropicResponse = response.json().await?;
        
        if let Some(content) = anthropic_response.content.first() {
            Ok(content.text.clone())
        } else {
            Err(ProviderError::InvalidResponse("No content in Anthropic response".to_string()))
        }
    }
    
    fn name(&self) -> &'static str {
        "anthropic"
    }
    
    fn max_context_length(&self) -> usize {
        self.max_context_length
    }
}

/// Factory function to create appropriate summarizer
pub fn create_summarizer(
    provider: &str,
    model: String,
    base_url: Option<String>,
    api_key: Option<String>,
    max_tokens: Option<usize>,
    temperature: Option<f32>,
) -> ProviderResult<Box<dyn Summarizer>> {
    match provider {
        "echo" => Ok(Box::new(EchoSummarizer::default())),
        "openai" => {
            let api_key = api_key.ok_or_else(|| ProviderError::ConfigError("API key required for OpenAI".to_string()))?;
            Ok(Box::new(OpenAISummarizer::new(model, base_url, api_key, max_tokens, temperature)))
        }
        "anthropic" => {
            let api_key = api_key.ok_or_else(|| ProviderError::ConfigError("API key required for Anthropic".to_string()))?;
            Ok(Box::new(AnthropicSummarizer::new(model, api_key, max_tokens)))
        }
        _ => Err(ProviderError::ConfigError(format!("Unknown summarizer provider: {}", provider))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{new_message_id, new_session_id};
    use crate::types::MessageRole;
    use chrono::Utc;
    
    fn create_test_message(role: MessageRole, content: &str) -> Message {
        Message {
            id: new_message_id(),
            session_id: new_session_id(),
            role,
            content: content.to_string(),
            timestamp: Utc::now(),
            token_count: content.len() / 4,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    #[tokio::test]
    async fn test_echo_summarizer() {
        let summarizer = EchoSummarizer::default();
        let messages = vec![
            create_test_message(MessageRole::User, "Hello"),
            create_test_message(MessageRole::Assistant, "Hi there!"),
        ];
        
        let summary = summarizer.summarize(&messages).await.unwrap();
        assert!(summary.contains("Hello"));
        assert!(summary.contains("Hi there!"));
        assert!(summary.contains("SUMMARY"));
    }
    
    #[test]
    fn test_factory() {
        let echo = create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
        assert_eq!(echo.name(), "echo");
        
        let result = create_summarizer("openai", "gpt-4".to_string(), None, None, None, None);
        assert!(result.is_err()); // Should fail without API key
    }
}
