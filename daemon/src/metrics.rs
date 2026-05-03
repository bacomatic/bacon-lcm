// daemon/src/metrics.rs
use prometheus::{Counter, Histogram, HistogramOpts, IntGauge, Registry};

/// All Prometheus metrics for the daemon.
pub struct Metrics {
    pub registry: Registry,
    pub messages_stored_total: Counter,
    pub compactions_total: Counter,
    pub compaction_duration_seconds: Histogram,
    pub active_sessions: IntGauge,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let messages_stored_total = Counter::new(
            "lcm_messages_stored_total",
            "Total number of messages stored across all sessions",
        )?;

        let compactions_total = Counter::new(
            "lcm_compactions_total",
            "Total number of compaction operations triggered",
        )?;

        let compaction_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "lcm_compaction_duration_seconds",
                "Duration of compaction operations in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0]),
        )?;

        let active_sessions = IntGauge::new(
            "lcm_active_sessions",
            "Number of currently active LCM sessions",
        )?;

        registry.register(Box::new(messages_stored_total.clone()))?;
        registry.register(Box::new(compactions_total.clone()))?;
        registry.register(Box::new(compaction_duration_seconds.clone()))?;
        registry.register(Box::new(active_sessions.clone()))?;

        Ok(Self {
            registry,
            messages_stored_total,
            compactions_total,
            compaction_duration_seconds,
            active_sessions,
        })
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new().expect("failed to create Prometheus metrics registry")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry_constructs() {
        let m = Metrics::new();
        assert!(m.is_ok(), "metrics registry construction failed: {:?}", m.err());
    }

    #[test]
    fn test_metrics_increment_counter() {
        let m = Metrics::new().unwrap();
        m.messages_stored_total.inc();
        assert_eq!(m.messages_stored_total.get(), 1.0);
    }

    #[test]
    fn test_metrics_active_sessions_gauge() {
        let m = Metrics::new().unwrap();
        m.active_sessions.set(3);
        assert_eq!(m.active_sessions.get(), 3);
    }

    #[test]
    fn test_metrics_encode_returns_non_empty() {
        let m = Metrics::new().unwrap();
        m.messages_stored_total.inc_by(5.0);
        let mut buf = String::new();
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let mfs = m.registry.gather();
        encoder.encode_utf8(&mfs, &mut buf).unwrap();
        assert!(buf.contains("lcm_messages_stored_total"));
        assert!(buf.contains("5"));
    }
}
