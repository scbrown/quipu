//! Prometheus metrics for the Quipu server.

use prometheus::{Encoder, Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};

/// All Prometheus metrics exposed by the Quipu server.
pub struct Metrics {
    /// Total active facts in store (gauge, refreshed on scrape).
    pub facts_total: IntGauge,
    /// SPARQL query latency in seconds.
    pub query_duration_seconds: Histogram,
    /// Total episodes ingested.
    pub episode_ingest_total: IntCounter,
    /// Total SHACL validation failures.
    pub validation_failures_total: IntCounter,
    /// Vector search latency in seconds.
    pub vector_search_duration_seconds: Histogram,
    registry: Registry,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Create and register all metrics.
    pub fn new() -> Self {
        let registry = Registry::new();

        let facts_total = IntGauge::new(
            "quipu_facts_total",
            "Total active facts in the knowledge store",
        )
        .expect("metric creation");

        let query_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "quipu_query_duration_seconds",
                "SPARQL query latency in seconds",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]),
        )
        .expect("metric creation");

        let episode_ingest_total = IntCounter::new(
            "quipu_episode_ingest_total",
            "Total number of episodes ingested",
        )
        .expect("metric creation");

        let validation_failures_total = IntCounter::new(
            "quipu_validation_failures_total",
            "Total SHACL validation failures",
        )
        .expect("metric creation");

        let vector_search_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "quipu_vector_search_duration_seconds",
                "Vector/similarity search latency in seconds",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        )
        .expect("metric creation");

        registry
            .register(Box::new(facts_total.clone()))
            .expect("register");
        registry
            .register(Box::new(query_duration_seconds.clone()))
            .expect("register");
        registry
            .register(Box::new(episode_ingest_total.clone()))
            .expect("register");
        registry
            .register(Box::new(validation_failures_total.clone()))
            .expect("register");
        registry
            .register(Box::new(vector_search_duration_seconds.clone()))
            .expect("register");

        Self {
            facts_total,
            query_duration_seconds,
            episode_ingest_total,
            validation_failures_total,
            vector_search_duration_seconds,
            registry,
        }
    }

    /// Encode all metrics in Prometheus text exposition format.
    pub fn encode(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder
            .encode(&metric_families, &mut buffer)
            .expect("encode");
        String::from_utf8(buffer).expect("utf8")
    }
}
