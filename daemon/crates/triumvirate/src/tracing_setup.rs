use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) fn init_tracing() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            "triumvirate=info,daemon_core=info,daemon_http=info,agent_worker=info,agent_adapter=info,mcp_bridge=info,mcp_tools=info,fallback_outbox=info,shared_types=info,warn".into()
        });
    let otel_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    match otel_endpoint {
        Some(endpoint) => {
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
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_target(false)
                        .with_span_events(FmtSpan::CLOSE),
                )
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        None => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_target(false)
                        .with_span_events(FmtSpan::CLOSE),
                )
                .init();
        }
    }
    Ok(())
}
