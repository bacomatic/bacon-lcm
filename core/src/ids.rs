// core/src/ids.rs
use uuid::Uuid;

/// Create a new message ID
pub fn new_message_id() -> crate::types::MessageId {
    Uuid::new_v4()
}

/// Create a new session ID
pub fn new_session_id() -> crate::types::SessionId {
    Uuid::new_v4()
}

/// Create a new summary ID
pub fn new_summary_id() -> crate::types::SummaryId {
    Uuid::new_v4()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_generation() {
        let msg_id = new_message_id();
        let session_id = new_session_id();
        let summary_id = new_summary_id();

        // All should be different
        assert_ne!(msg_id, session_id);
        assert_ne!(session_id, summary_id);
        assert_ne!(msg_id, summary_id);

        // All should be valid UUIDs
        assert!(msg_id.get_version() == Some(uuid::Version::Random));
        assert!(session_id.get_version() == Some(uuid::Version::Random));
        assert!(summary_id.get_version() == Some(uuid::Version::Random));
    }
}
