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
            // Without a registered propagator, `inject_context` writes NOTHING and the daemon
            // never learns it is a child of the MCP bridge's span, the two processes would keep
            // producing two unrelated traces for one call. This is the line that makes the
            // traceparent header in daemon-http actually carry something.
            opentelemetry::global::set_text_map_propagator(
                opentelemetry_sdk::propagation::TraceContextPropagator::new(),
            );

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
            // D-002: the exporter's OWN telemetry must not travel through the exporter.
            //
            // The OTel SDK reports its export failures as tracing events (`otel_error!` emits
            // `target: env!("CARGO_PKG_NAME")`, i.e. `opentelemetry_sdk`, with a hardcoded empty
            // message and the cause in an `error` field). This layer had no filter, so each
            // failure was queued into the very batch exporter that had just failed. Measured
            // 2026-09-19: ~4,266 such events a day in the local log, 30 ever reaching PostHog.
            // They arrive, when they arrive at all, only after the outage they describe.
            //
            // The original D-002 entry said the cause "does not survive into PostHog". That was
            // wrong: the 30 that landed carry it in `attributes.error`; only `body` is empty, by
            // the SDK's design. The real defects were the loop and the loss, and both end here.
            // These events stay in the LOCAL log in full, cause included, which is where they
            // can be read during the outage; "is telemetry arriving at all" is answered by the
            // delivery round trip in `mcp_bridge::telemetry_delivery`, not by this stream.
            let otel_log_layer = exporter_safe_log_layer(logger_provider);

            // A span costs money to ship and store, and dilutes every query written against it.
            // Without a filter of its own this layer exports EVERY #[instrument] in the tree: a
            // sample run shipped 74 `triumvirate_home_dir` and 48 `unix_time_ms` spans (0ms each,
            // unable to answer any question anyone would ask) against 3 `ask_agent` spans.
            //
            // But the filter must not be so tight that it severs the tree. Spans DO nest correctly
            // (verified: `ask_agent` parents `acquire_worker` under one trace_id) -- and
            // `acquire_worker` lives in `agent_worker`, so a `triumvirate`-only filter would keep
            // the root and throw away its children, leaving a lone span pretending to be a trace.
            //
            // So: keep this crate plus the worker lifecycle (acquire/update/dismiss_worker, which
            // is exactly where session and cost bugs hide). Drop `daemon_core` -- that is where the
            // home_dir/unix_time_ms/persist_json_file noise comes from -- and the `mcp_bridge`
            // string helpers. The stderr layer keeps its own, chattier filter regardless.
            // `daemon_http` must stay in: it owns `daemon_ask_agent`, the span that adopts the
            // MCP bridge's traceparent. Filter it out and the daemon's `ask_agent` inherits a
            // parent that was never exported, a dangling edge, which renders worse than no
            // parent at all.
            let otel_span_filter = tracing_subscriber::EnvFilter::try_from_env("OTEL_SPAN_FILTER")
                .unwrap_or_else(|_| "triumvirate=info,agent_worker=info,daemon_http=info".into());

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

/// The OTLP log layer, with the exporter's own telemetry kept out of it (D-002).
///
/// A named builder, and the ONLY way production constructs this layer, so the test below can
/// exercise the real wiring. Testing `is_exporter_self_telemetry` alone would stay green if the
/// `.with_filter(...)` call were deleted, which is the helper-instead-of-call-site trap this
/// repo has already fallen into more than once.
pub(crate) fn exporter_safe_log_layer<S, P, L>(
    provider: &P,
) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    P: opentelemetry::logs::LoggerProvider<Logger = L> + Send + Sync + 'static,
    L: opentelemetry::logs::Logger + Send + Sync + 'static,
{
    OpenTelemetryTracingBridge::new(provider).with_filter(tracing_subscriber::filter::filter_fn(
        |meta| !is_exporter_self_telemetry(meta.target()),
    ))
}

/// Is this event the telemetry pipeline talking about ITSELF? Pure, so the rule is testable.
///
/// `opentelemetry*` covers the SDK and the OTLP exporter. `reqwest`, `hyper` and `h2` are the
/// HTTP stack the exporter rides on: their events during an export are the same loop one layer
/// down. Matched on the target's leading path segment, so `hyperlocal` or `reqwest_retry` are
/// not swept up by accident.
pub(crate) fn is_exporter_self_telemetry(target: &str) -> bool {
    let root = target.split("::").next().unwrap_or(target);
    root.starts_with("opentelemetry") || matches!(root, "reqwest" | "hyper" | "h2")
}

fn should_write_logs_to_stderr() -> bool {
    // Keep MCP/proxy stdout reserved for JSON-RPC frames only.
    matches!(
        std::env::args().nth(1).as_deref(),
        Some("mcp") | Some("proxy")
    )
}

#[cfg(test)]
mod exporter_self_telemetry_tests {
    use super::is_exporter_self_telemetry;

    /// RED IF: the SDK's export-failure events flow back into the OTLP exporter. The target is
    /// verified from source: `otel_error!` uses `env!("CARGO_PKG_NAME")`, so the SDK's
    /// `BatchLogProcessor.ExportError` arrives as `opentelemetry_sdk`.
    #[test]
    fn the_pipelines_own_events_are_kept_out_of_the_pipeline() {
        for t in ["opentelemetry_sdk", "opentelemetry_otlp", "opentelemetry", "opentelemetry_sdk::logs",
                  "reqwest", "reqwest::connect", "hyper", "hyper::proto::h1", "h2"] {
            assert!(is_exporter_self_telemetry(t), "{t} must not be exported through itself");
        }
    }

    /// THE WIRING, through the same builder production calls. An event from the SDK's own
    /// target must not reach the exporter; the daemon's own event must.
    /// RED IF: `.with_filter(...)` is removed from `exporter_safe_log_layer`.
    #[test]
    fn the_production_layer_drops_the_sdk_event_and_keeps_the_daemons() {
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
        use tracing_subscriber::layer::SubscriberExt;

        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(super::exporter_safe_log_layer(&provider));
        tracing::subscriber::with_default(subscriber, || {
            // Verbatim in shape to what `otel_error!` emits for a failed batch export.
            tracing::error!(target: "opentelemetry_sdk", name = "BatchLogProcessor.ExportError", error = "boom", "");
            tracing::error!(target: "triumvirate::agent_exec", "the daemon's own event");
        });

        let logs = exporter.get_emitted_logs().expect("in-memory exporter");
        let bodies: Vec<String> = logs
            .iter()
            .map(|l| format!("{:?}", l.record.body()))
            .collect();
        assert_eq!(logs.len(), 1, "exactly the daemon's event may ship; got {bodies:?}");
        assert!(bodies[0].contains("the daemon's own event"), "got {bodies:?}");
    }

    /// RED IF: the filter widens into the daemon's own events. Those are the entire point of the
    /// log export; dropping them would trade a loop for a blackout.
    #[test]
    fn the_daemons_own_events_still_ship() {
        for t in ["triumvirate", "triumvirate::agent_exec", "mcp_bridge::posthog", "agent_worker",
                  "daemon_http", "hyperlocal", "reqwest_retry", "fleet"] {
            assert!(!is_exporter_self_telemetry(t), "{t} is the daemon's own and must ship");
        }
    }
}
