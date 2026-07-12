use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, logs::SdkLoggerProvider, trace::SdkTracerProvider};
use std::sync::OnceLock;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

const SERVICE_NAME: &str = "triumvirate-daemon-v2";

/// The logger provider owns the batch processor that ships logs. If it is dropped at the
/// end of `init_tracing`, the processor shuts down and every log is silently discarded.
/// Pin it for the life of the process.
static LOGGER_PROVIDER: OnceLock<SdkLoggerProvider> = OnceLock::new();

pub(crate) fn init_tracing() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            "triumvirate=info,daemon_core=info,daemon_http=info,agent_worker=info,agent_adapter=info,mcp_bridge=info,mcp_tools=info,fallback_outbox=info,shared_types=info,warn".into()
        });
    let otel_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    let use_stderr = should_write_logs_to_stderr();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_target(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(move || {
            if use_stderr {
                Box::new(std::io::stderr()) as Box<dyn std::io::Write + Send>
            } else {
                Box::new(std::io::stdout()) as Box<dyn std::io::Write + Send>
            }
        });
    match otel_endpoint {
        Some(endpoint) => {
            let resource = Resource::builder().with_service_name(SERVICE_NAME).build();

            // --- traces (OTLP /v1/traces) ---
            // NOTE: opentelemetry-otlp resolves OTEL_EXPORTER_OTLP_ENDPOINT as a *base* and
            // appends the signal path ("/v1/traces", "/v1/logs"). Pass the base, not the
            // full signal URL, or you get "/v1/traces/v1/traces" and a silent 404.
            let span_exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint.clone())
                .build()?;
            let tracer_provider = SdkTracerProvider::builder()
                .with_batch_exporter(span_exporter)
                .with_resource(resource.clone())
                .build();
            let tracer = tracer_provider.tracer(SERVICE_NAME);
            opentelemetry::global::set_tracer_provider(tracer_provider);

            // --- logs (OTLP /v1/logs) ---
            let log_exporter = opentelemetry_otlp::LogExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .build()?;
            let logger_provider = SdkLoggerProvider::builder()
                .with_batch_exporter(log_exporter)
                .with_resource(resource)
                .build();
            let logger_provider = LOGGER_PROVIDER.get_or_init(|| logger_provider);
            let otel_log_layer = OpenTelemetryTracingBridge::new(logger_provider);

            // A span costs money to ship and store, and dilutes every query written against
            // it. Without a filter of its own this layer exports EVERY #[instrument] in the
            // tree: a sample run produced 74 `triumvirate_home_dir` and 48 `unix_time_ms`
            // spans (0ms each, unable to answer any question anyone would ask) against 3
            // `ask_agent` spans -- the only ones that carry agent/outcome/duration. Export
            // this crate's spans; leave the sub-crate plumbing to the stderr layer, which
            // keeps its own, chattier filter.
            let otel_span_filter = tracing_subscriber::EnvFilter::try_from_env("OTEL_SPAN_FILTER")
                .unwrap_or_else(|_| "triumvirate=info".into());

            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .with(
                    tracing_opentelemetry::layer()
                        .with_tracer(tracer)
                        .with_filter(otel_span_filter),
                )
                .with(otel_log_layer)
                .init();
        }
        None => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
    }
    Ok(())
}

fn should_write_logs_to_stderr() -> bool {
    // Keep MCP/proxy stdout reserved for JSON-RPC frames only.
    matches!(
        std::env::args().nth(1).as_deref(),
        Some("mcp") | Some("proxy")
    )
}
