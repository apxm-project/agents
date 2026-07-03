//! W3C `traceparent` extraction and propagation.
//!
//! Two levels are provided, matching the two call sites that already exist
//! in the codebase (`os-listeners::relay_http`, HOST WS-B, already landed):
//!
//! 1. [`traceparent_from_headers`] — read the raw header string, for
//!    services that only want to attach it to a span/log line as an
//!    attribute (the lightweight pattern `os` already uses).
//! 2. [`extract_traceparent`] / [`inject_current_traceparent`] — full W3C
//!    trace-context propagation: parse the header into an OTel `Context`,
//!    set it as the current span's parent, and inject the *current* span's
//!    context back into outgoing request headers. This makes a
//!    server -> os -> host (or studio -> server/auth/os) hop one continuous
//!    trace instead of independently-rooted spans that merely log the same
//!    string.

use opentelemetry::propagation::{Extractor, Injector};

const TRACEPARENT: &str = "traceparent";

struct HeaderExtractor<'a>(&'a http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

struct HeaderInjector<'a>(&'a mut http::HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(val)) = (
            http::HeaderName::from_bytes(key.as_bytes()),
            http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, val);
        }
    }
}

/// Read the raw `traceparent` header value, if present. Does not parse or
/// validate the W3C format — for the lightweight "attach as a span/log
/// attribute" pattern.
pub fn traceparent_from_headers(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get(TRACEPARENT)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string)
}

/// Parse an inbound `traceparent` (via the globally-registered
/// `TraceContextPropagator`, set by [`crate::init`]) into an OTel `Context`
/// and set it as the given span's parent, so this span — and every child
/// span/log emitted under it — is joined to the caller's trace.
///
/// Returns the raw header value too (for logging/back-compat with the
/// existing `os` span-attribute pattern).
pub fn extract_traceparent(headers: &http::HeaderMap, span: &tracing::Span) -> Option<String> {
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let raw = traceparent_from_headers(headers);
    if raw.is_some() {
        let cx = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(headers))
        });
        span.set_parent(cx);
    }
    raw
}

/// Inject the *current* span's OTel context into outgoing request headers as
/// a W3C `traceparent`, so the next hop can [`extract_traceparent`] it and
/// continue the same trace.
pub fn inject_current_traceparent(headers: &mut http::HeaderMap) {
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(headers));
    });
}

/// [`inject_current_traceparent`] convenience for `reqwest` callers — sets
/// the header on the request builder directly.
#[cfg(feature = "reqwest")]
pub fn inject_traceparent_reqwest(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let mut headers = http::HeaderMap::new();
    inject_current_traceparent(&mut headers);
    builder.headers(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TracerProvider as _;
    use tracing_subscriber::layer::SubscriberExt;

    /// Real span-linking (`set_parent`/`context`) only does anything once a
    /// `tracing-opentelemetry` layer backed by a real SDK tracer is active —
    /// mirrors what [`crate::init`] installs in production, scoped to the
    /// current thread for the duration of `f` so tests don't race a global
    /// subscriber.
    fn with_test_subscriber<F: FnOnce()>(f: F) {
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
        let provider = opentelemetry_sdk::trace::TracerProvider::builder().build();
        let tracer = provider.tracer("test");
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        let subscriber = tracing_subscriber::registry().with(telemetry);
        tracing::subscriber::with_default(subscriber, f);
    }

    #[test]
    fn traceparent_from_headers_reads_raw_value() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            traceparent_from_headers(&headers).as_deref(),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
    }

    #[test]
    fn traceparent_from_headers_absent_is_none() {
        let headers = http::HeaderMap::new();
        assert_eq!(traceparent_from_headers(&headers), None);
    }

    #[test]
    fn extract_traceparent_joins_the_incoming_trace_id() {
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        with_test_subscriber(|| {
            let mut headers = http::HeaderMap::new();
            let incoming_trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
            headers.insert(
                "traceparent",
                format!("00-{incoming_trace_id}-00f067aa0ba902b7-01")
                    .parse()
                    .unwrap(),
            );

            let span = tracing::info_span!("test.request");
            let raw = extract_traceparent(&headers, &span);
            assert!(raw.is_some());

            let _enter = span.enter();
            let cx = tracing::Span::current().context();
            let span_ref = opentelemetry::trace::TraceContextExt::span(&cx);
            let trace_id = span_ref.span_context().trace_id().to_string();
            assert_eq!(trace_id, incoming_trace_id);
        });
    }

    #[test]
    fn inject_current_traceparent_round_trips_through_extract() {
        with_test_subscriber(|| {
            let mut inbound = http::HeaderMap::new();
            let incoming_trace_id = "0af7651916cd43dd8448eb211c80319c";
            inbound.insert(
                "traceparent",
                format!("00-{incoming_trace_id}-b7ad6b7169203331-01")
                    .parse()
                    .unwrap(),
            );

            let span = tracing::info_span!("test.boundary");
            extract_traceparent(&inbound, &span);
            let _enter = span.enter();

            let mut outbound = http::HeaderMap::new();
            inject_current_traceparent(&mut outbound);

            let propagated = traceparent_from_headers(&outbound).expect("traceparent set");
            assert!(
                propagated.contains(incoming_trace_id),
                "expected outbound traceparent {propagated:?} to carry trace id {incoming_trace_id}"
            );
        });
    }

    #[test]
    fn traceparent_survives_a_second_hop() {
        // server -> os -> host: verify the trace id set at hop 1 is still
        // present after being extracted and re-injected at hop 2.
        with_test_subscriber(|| {
            let mut hop0 = http::HeaderMap::new();
            let trace_id = "5b8aa5a2d2c872e8321cf37308d69df2";
            hop0.insert(
                "traceparent",
                format!("00-{trace_id}-051581bf3cb55c13-01").parse().unwrap(),
            );

            let span1 = tracing::info_span!("hop1");
            extract_traceparent(&hop0, &span1);
            let mut hop1 = http::HeaderMap::new();
            {
                let _e = span1.enter();
                inject_current_traceparent(&mut hop1);
            }

            let span2 = tracing::info_span!("hop2");
            extract_traceparent(&hop1, &span2);
            let mut hop2 = http::HeaderMap::new();
            {
                let _e = span2.enter();
                inject_current_traceparent(&mut hop2);
            }

            let final_header = traceparent_from_headers(&hop2).expect("hop2 traceparent");
            assert!(final_header.contains(trace_id));
        });
    }
}
