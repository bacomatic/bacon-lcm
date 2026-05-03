// core/src/storage/summary_dag.rs
use crate::error::{StorageError, StorageResult};
use crate::types::*;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::message_store::MessageStore;

/// Trait for summary DAG operations
#[async_trait]
pub trait SummaryDag: Send + Sync {
    /// Add a new summary node to the DAG
    async fn add_node(&self, node: SummaryNode) -> StorageResult<SummaryId>;

    /// Get a summary node by ID
    async fn get_node(&self, id: SummaryId) -> StorageResult<Option<SummaryNode>>;

    /// Get all summaries for a session
    async fn get_session_summaries(&self, session_id: SessionId) -> StorageResult<Vec<SummaryNode>>;

    /// Get the lineage (source pointers) for a summary
    async fn get_lineage(&self, id: SummaryId) -> StorageResult<Vec<LineagePointer>>;

    /// Expand a summary to get all original messages
    async fn expand(&self, id: SummaryId, message_store: &dyn MessageStore) -> StorageResult<Vec<Message>>;

    /// Get summaries at a specific compaction level
    async fn get_summaries_by_level(&self, session_id: SessionId, level: SummaryLevel) -> StorageResult<Vec<SummaryNode>>;

    /// Delete all summaries for a session
    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()>;

    /// Check for lineage cycles (should never happen in valid DAG)
    async fn detect_cycles(&self, session_id: SessionId) -> StorageResult<bool>;
}

/// In-memory summary DAG implementation
#[derive(Debug)]
pub struct InMemorySummaryDag {
    nodes: Arc<RwLock<HashMap<SummaryId, SummaryNode>>>,
    session_nodes: Arc<RwLock<HashMap<SessionId, Vec<SummaryId>>>>,
    lineage_cache: Arc<RwLock<HashMap<SummaryId, Vec<LineagePointer>>>>,
}

impl InMemorySummaryDag {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            session_nodes: Arc::new(RwLock::new(HashMap::new())),
            lineage_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Recursively collect all lineage pointers for a summary.
    /// Returns an error if a cycle is detected or if a summary reference points to an unknown node.
    async fn collect_lineage(&self, id: SummaryId, visited: &mut HashSet<SummaryId>) -> StorageResult<Vec<LineagePointer>> {
        if visited.contains(&id) {
            return Err(StorageError::ConstraintViolation("Cycle detected in lineage".to_string()));
        }

        visited.insert(id);

        let nodes = self.nodes.read().await;
        if let Some(node) = nodes.get(&id) {
            let lineage_items: Vec<LineagePointer> = node.lineage.clone();
            // Drop the lock before recursing
            drop(nodes);

            let mut lineage = Vec::new();

            for pointer in lineage_items {
                match pointer {
                    LineagePointer::Message(_) => lineage.push(pointer),
                    LineagePointer::Summary(summary_id) => {
                        // Verify the referenced summary exists before recursing
                        {
                            let nodes = self.nodes.read().await;
                            if !nodes.contains_key(&summary_id) {
                                return Err(StorageError::ConstraintViolation(
                                    format!("Lineage references unknown summary: {}", summary_id)
                                ));
                            }
                        }
                        // Box the future to allow recursion
                        let sub_lineage = Box::pin(self.collect_lineage(summary_id, visited)).await?;
                        lineage.extend(sub_lineage);
                    }
                }
            }

            Ok(lineage)
        } else {
            Ok(Vec::new())
        }
    }
}

impl Default for InMemorySummaryDag {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SummaryDag for InMemorySummaryDag {
    async fn add_node(&self, node: SummaryNode) -> StorageResult<SummaryId> {
        let id = node.id;

        // Check if the new node's lineage directly or transitively references itself
        // by temporarily inserting it and running collect_lineage
        {
            let mut nodes = self.nodes.write().await;
            nodes.insert(id, node.clone());
        }

        // collect_lineage will return an error if a cycle is detected
        let lineage_result = self.collect_lineage(id, &mut HashSet::new()).await;

        match lineage_result {
            Err(_) => {
                // Cycle detected — remove the node we just inserted and return error
                let mut nodes = self.nodes.write().await;
                nodes.remove(&id);
                return Err(StorageError::ConstraintViolation("Adding node would create cycle".to_string()));
            }
            Ok(lineage) => {
                // Update session index
                {
                    let mut session_nodes = self.session_nodes.write().await;
                    session_nodes
                        .entry(node.session_id)
                        .or_insert_with(Vec::new)
                        .push(id);
                }

                // Update lineage cache
                {
                    let mut lineage_cache = self.lineage_cache.write().await;
                    lineage_cache.insert(id, lineage);
                }

                Ok(id)
            }
        }
    }

    async fn get_node(&self, id: SummaryId) -> StorageResult<Option<SummaryNode>> {
        let nodes = self.nodes.read().await;
        Ok(nodes.get(&id).cloned())
    }

    async fn get_session_summaries(&self, session_id: SessionId) -> StorageResult<Vec<SummaryNode>> {
        let session_nodes = self.session_nodes.read().await;
        let nodes = self.nodes.read().await;

        if let Some(node_ids) = session_nodes.get(&session_id) {
            let mut result = Vec::with_capacity(node_ids.len());

            for &id in node_ids {
                if let Some(node) = nodes.get(&id) {
                    result.push(node.clone());
                }
            }

            // Sort by timestamp
            result.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    async fn get_lineage(&self, id: SummaryId) -> StorageResult<Vec<LineagePointer>> {
        let lineage_cache = self.lineage_cache.read().await;
        Ok(lineage_cache.get(&id).cloned().unwrap_or_default())
    }

    async fn expand(&self, id: SummaryId, message_store: &dyn MessageStore) -> StorageResult<Vec<Message>> {
        let lineage = self.get_lineage(id).await?;
        let mut messages = Vec::new();

        for pointer in lineage {
            match pointer {
                LineagePointer::Message(message_id) => {
                    if let Some(message) = message_store.get(message_id).await? {
                        messages.push(message);
                    }
                }
                LineagePointer::Summary(_) => {
                    // Recursively expand nested summaries
                    // This would require careful handling to avoid infinite loops
                    // For now, we'll skip nested summary expansion
                }
            }
        }

        // Sort by timestamp
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(messages)
    }

    async fn get_summaries_by_level(&self, session_id: SessionId, level: SummaryLevel) -> StorageResult<Vec<SummaryNode>> {
        let all_summaries = self.get_session_summaries(session_id).await?;
        Ok(all_summaries.into_iter().filter(|s| s.level == level).collect())
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        let mut session_nodes = self.session_nodes.write().await;
        let mut nodes = self.nodes.write().await;
        let mut lineage_cache = self.lineage_cache.write().await;

        if let Some(node_ids) = session_nodes.remove(&session_id) {
            for id in node_ids {
                nodes.remove(&id);
                lineage_cache.remove(&id);
            }
        }

        Ok(())
    }

    async fn detect_cycles(&self, session_id: SessionId) -> StorageResult<bool> {
        let session_nodes = self.session_nodes.read().await;
        let nodes = self.nodes.read().await;

        if let Some(node_ids) = session_nodes.get(&session_id) {
            for &id in node_ids {
                let mut visited = HashSet::new();
                if self.has_cycle_from_node(id, &nodes, &mut visited) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}

impl InMemorySummaryDag {
    /// Helper method to detect cycles from a specific node
    fn has_cycle_from_node(
        &self,
        node_id: SummaryId,
        nodes: &HashMap<SummaryId, SummaryNode>,
        visited: &mut HashSet<SummaryId>,
    ) -> bool {
        if visited.contains(&node_id) {
            return true;
        }

        visited.insert(node_id);

        if let Some(node) = nodes.get(&node_id) {
            for pointer in &node.lineage {
                if let LineagePointer::Summary(summary_id) = pointer {
                    if self.has_cycle_from_node(*summary_id, nodes, visited) {
                        return true;
                    }
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{new_session_id, new_summary_id};
    use chrono::Utc;

    fn create_test_summary(session_id: SessionId, level: SummaryLevel, lineage: Vec<LineagePointer>) -> SummaryNode {
        SummaryNode {
            id: new_summary_id(),
            session_id,
            level,
            content: "Test summary".to_string(),
            token_count: 50,
            lineage,
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_add_and_get_summary() {
        let dag = InMemorySummaryDag::new();
        let session_id = new_session_id();
        let summary = create_test_summary(session_id, SummaryLevel::Leaf, vec![]);

        let stored_id = dag.add_node(summary.clone()).await.unwrap();
        assert_eq!(stored_id, summary.id);

        let retrieved = dag.get_node(summary.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "Test summary");
    }

    #[tokio::test]
    async fn test_session_summaries() {
        let dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        let summary1 = create_test_summary(session_id, SummaryLevel::Leaf, vec![]);
        let summary2 = create_test_summary(session_id, SummaryLevel::Condensed, vec![]);

        dag.add_node(summary1).await.unwrap();
        dag.add_node(summary2).await.unwrap();

        let summaries = dag.get_session_summaries(session_id).await.unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        let dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Create summaries that would form a cycle
        let summary1 = create_test_summary(session_id, SummaryLevel::Leaf, vec![]);
        let summary2 = create_test_summary(session_id, SummaryLevel::Leaf, vec![LineagePointer::Summary(summary1.id)]);
        let summary3 = create_test_summary(session_id, SummaryLevel::Leaf, vec![LineagePointer::Summary(summary2.id)]);

        // Modify summary1 to point to summary3, creating a cycle
        let mut summary1_with_cycle = summary1.clone();
        summary1_with_cycle.lineage = vec![LineagePointer::Summary(summary3.id)];

        // This should fail due to cycle detection
        let result = dag.add_node(summary1_with_cycle).await;
        assert!(result.is_err());
    }
}
