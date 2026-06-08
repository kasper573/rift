use rift::Metrics;

#[test]
fn renders_counters_histograms_and_labels() {
    let mut metrics = Metrics::default();
    metrics.ticks = 7;
    metrics.tick_duration.observe(0.002);
    metrics.close("normal");
    metrics.close("normal");
    metrics.close("unauthorized");
    let text = metrics.render();
    assert!(text.contains("rift_ticks_total 7\n"));
    assert!(text.contains("rift_tick_duration_seconds_bucket{le=\"0.0025\"} 1\n"));
    assert!(text.contains("rift_tick_duration_seconds_count 1\n"));
    assert!(text.contains("rift_client_connections_closed_total{code=\"normal\"} 2\n"));
    assert!(text.contains("rift_client_connections_closed_total{code=\"unauthorized\"} 1\n"));
}
