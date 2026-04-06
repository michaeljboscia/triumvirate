use mcp_bridge::should_use_daemon_proxy;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) fn init_tracing() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "triumvirate=info".into());
    let json_logs = should_use_daemon_proxy(std::env::var("TRIUMVIRATE_JSON_LOGS").ok().as_deref());
    let otel_endpoint = std::env::var("TRIUMVIRATE_OTEL_EXPORTER_OTLP_ENDPOINT").ok();

    match (json_logs, otel_endpoint) {
        (true, Some(endpoint)) => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .build()?;
            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(Resource::builder().with_service_name("triumvirate-daemon-v2").build())
                .build();
            let tracer = provider.tracer("triumvirate-daemon-v2");
            opentelemetry::global::set_tracer_provider(provider);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().json().with_target(false))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        (false, Some(endpoint)) => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .build()?;
            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(Resource::builder().with_service_name("triumvirate-daemon-v2").build())
                .build();
            let tracer = provider.tracer("triumvirate-daemon-v2");
            opentelemetry::global::set_tracer_provider(provider);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().with_target(false))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        (true, None) => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().json().with_target(false))
                .init();
        }
        (false, None) => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().with_target(false))
                .init();
        }
    }
    Ok(())
}
