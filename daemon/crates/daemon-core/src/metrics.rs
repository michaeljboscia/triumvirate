use prometheus::{
    Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts,
    Registry,
};

#[derive(Debug, Clone)]
pub struct DaemonMetrics {
    pub registry: Registry,
    pub agent_requests_total: IntCounter,
    pub agent_duration_seconds: Histogram,
    pub agent_tokens_total: IntCounter,
    pub ledger_events_ingested_total: IntCounter,
    pub ledger_queue_lag_seconds: Gauge,
    pub ledger_spool_size_bytes: IntGauge,
    pub fleet_active_total: IntGauge,
    pub reviews_total: IntCounter,
    pub marker_parse_success_rate: Gauge,
    pub http_requests_total: IntCounterVec,
    pub http_request_duration_seconds: HistogramVec,
    pub http_in_flight_requests: IntGauge,
}

impl DaemonMetrics {
    pub fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();
        let agent_requests_total = IntCounter::new(
            "triumvirate_agent_requests_total",
            "Total ask_agent requests handled by daemon",
        )?;
        let agent_duration_seconds = Histogram::with_opts(HistogramOpts::new(
            "triumvirate_agent_duration_seconds",
            "Duration of ask_agent requests in seconds",
        ))?;
        let agent_tokens_total = IntCounter::new(
            "triumvirate_agent_tokens_total",
            "Total tokens reported by ask_agent requests",
        )?;
        let ledger_events_ingested_total = IntCounter::new(
            "triumvirate_ledger_events_ingested_total",
            "Total ledger events ingested",
        )?;
        let ledger_queue_lag_seconds = Gauge::new(
            "triumvirate_ledger_queue_lag_seconds",
            "Ledger queue lag in seconds",
        )?;
        let ledger_spool_size_bytes = IntGauge::new(
            "triumvirate_ledger_spool_size_bytes",
            "Current ledger spool directory size in bytes",
        )?;
        let fleet_active_total = IntGauge::new("triumvirate_fleet_active_total", "Active fleet count")?;
        let reviews_total = IntCounter::new("triumvirate_reviews_total", "Total reviews completed")?;
        let marker_parse_success_rate = Gauge::new(
            "triumvirate_marker_parse_success_rate",
            "Marker parse success rate",
        )?;
        let http_requests_total = IntCounterVec::new(
            Opts::new(
                "triumvirate_http_requests_total",
                "HTTP requests by route and status",
            ),
            &["route", "status"],
        )?;
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "triumvirate_http_request_duration_seconds",
                "HTTP request durations by route",
            ),
            &["route"],
        )?;
        let http_in_flight_requests =
            IntGauge::new("triumvirate_http_requests_in_flight", "In-flight HTTP requests")?;
        registry.register(Box::new(agent_requests_total.clone()))?;
        registry.register(Box::new(agent_duration_seconds.clone()))?;
        registry.register(Box::new(agent_tokens_total.clone()))?;
        registry.register(Box::new(ledger_events_ingested_total.clone()))?;
        registry.register(Box::new(ledger_queue_lag_seconds.clone()))?;
        registry.register(Box::new(ledger_spool_size_bytes.clone()))?;
        registry.register(Box::new(fleet_active_total.clone()))?;
        registry.register(Box::new(reviews_total.clone()))?;
        registry.register(Box::new(marker_parse_success_rate.clone()))?;
        registry.register(Box::new(http_requests_total.clone()))?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;
        registry.register(Box::new(http_in_flight_requests.clone()))?;
        marker_parse_success_rate.set(1.0);
        Ok(Self {
            registry,
            agent_requests_total,
            agent_duration_seconds,
            agent_tokens_total,
            ledger_events_ingested_total,
            ledger_queue_lag_seconds,
            ledger_spool_size_bytes,
            fleet_active_total,
            reviews_total,
            marker_parse_success_rate,
            http_requests_total,
            http_request_duration_seconds,
            http_in_flight_requests,
        })
    }

    pub fn snapshot_keepalive(&self) {
        let _ = self.agent_tokens_total.get();
        let _ = self.ledger_events_ingested_total.get();
        let _ = self.ledger_queue_lag_seconds.get();
        let _ = self.ledger_spool_size_bytes.get();
        let _ = self.fleet_active_total.get();
        let _ = self.reviews_total.get();
        let _ = self.marker_parse_success_rate.get();
    }
}
