// core/src/types.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a message
pub type MessageId = Uuid;

/// Unique identifier for a session
pub type SessionId = Uuid;

/// Unique identifier for a summary node
pub type SummaryId = Uuid;

/// Message role in conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Individual message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub token_count: usize,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Summary node in the DAG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryNode {
    pub id: SummaryId,
    pub session_id: SessionId,
    pub level: SummaryLevel,
    pub content: String,
    pub token_count: usize,
    pub lineage: Vec<LineagePointer>,
    pub timestamp: DateTime<Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Summary compaction level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SummaryLevel {
    Leaf,
    Condensed,
    Emergency,
}

/// Pointer to source material for a summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LineagePointer {
    Message(MessageId),
    Summary(SummaryId),
}

/// Session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Item in the active context window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextItem {
    Message(Message),
    Summary(SummaryNode),
}

impl ContextItem {
    pub fn token_count(&self) -> usize {
        match self {
            ContextItem::Message(msg) => msg.token_count,
            ContextItem::Summary(summary) => summary.token_count,
        }
    }
    
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            ContextItem::Message(msg) => msg.timestamp,
            ContextItem::Summary(summary) => summary.timestamp,
        }
    }
}

/// Configuration for compaction thresholds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdConfig {
    pub model_max_tokens: usize,
    pub soft_limit: usize,
    pub hard_limit: usize,
}

/// Full compaction configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    pub thresholds: ThresholdConfig,
    pub fresh_tail_count: usize,
    pub leaf_group_size: usize,
    pub condensed_group_size: usize,
    pub parallel_compaction: bool,
    pub max_concurrent_compactions: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            thresholds: ThresholdConfig {
                model_max_tokens: 128000,
                soft_limit: 80000,
                hard_limit: 110000,
            },
            fresh_tail_count: 10,
            leaf_group_size: 20,
            condensed_group_size: 10,
            parallel_compaction: true,
            max_concurrent_compactions: 4,
        }
    }
}

/// Result of a compaction operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub level: SummaryLevel,
    pub summaries_created: Vec<SummaryId>,
    pub messages_compacted: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub duration_ms: u64,
}
