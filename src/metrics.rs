use opentelemetry::{metrics::{Counter, Gauge, Meter}};

/// Struct for consolidating all metrics into one place
pub struct Metrics {
    pub active_connections_gauge: Gauge<u64>,
    pub connections_counter: Counter<u64>,
    pub proxied_bytes_counter: Counter<u64>
}

/// Implementation for Metrics
impl Metrics {

    /// Creates new Metrics struct based on a parsed Meter Provider
    pub fn new(meter: &Meter) -> Self {
        Self {
            active_connections_gauge: meter.u64_gauge("active_connections.gauge").build(),
            connections_counter: meter.u64_counter("connections.counter").build(),
            proxied_bytes_counter: meter.u64_counter("bytes_proxied.counter").build(),
        }
    }
}