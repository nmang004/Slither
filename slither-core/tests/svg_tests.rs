use slither_core::report::svg;

#[test]
fn test_score_gauge_contains_svg() {
    let result = svg::render_score_gauge(85, "B", "#55efc4");
    assert!(result.contains("<svg"));
    assert!(result.contains("85"));
    assert!(result.contains("B"));
}

#[test]
fn test_score_gauge_zero() {
    let result = svg::render_score_gauge(0, "F", "#e17055");
    assert!(result.contains("0"));
    assert!(result.contains("F"));
}

#[test]
fn test_score_gauge_perfect() {
    let result = svg::render_score_gauge(100, "A", "#00b894");
    assert!(result.contains("100"));
    assert!(result.contains("A"));
    assert!(result.to_lowercase().contains("health score"));
}

#[test]
fn test_donut_chart() {
    let segments = vec![
        (75.0, "2xx", "#00b894"),
        (15.0, "3xx", "#fdcb6e"),
        (10.0, "4xx", "#e17055"),
    ];
    let result = svg::render_donut_chart(&segments, 100);
    assert!(result.contains("<svg"));
    assert!(result.contains("2xx"));
    assert!(result.contains("100"));
}

#[test]
fn test_donut_chart_single_segment() {
    let segments = vec![(100.0, "2xx", "#00b894")];
    let result = svg::render_donut_chart(&segments, 42);
    assert!(result.contains("<svg"));
    assert!(result.contains("42"));
}

#[test]
fn test_bar_chart() {
    let buckets = vec![
        ("0-200ms".to_string(), 30, "#00b894"),
        ("200-500ms".to_string(), 15, "#fdcb6e"),
        ("500ms+".to_string(), 5, "#e17055"),
    ];
    let result = svg::render_bar_chart(&buckets);
    assert!(result.contains("<svg"));
    assert!(result.contains("0-200ms"));
    assert!(result.contains("30"));
}

#[test]
fn test_bar_chart_empty() {
    let buckets: Vec<(String, u32, &str)> = vec![];
    let result = svg::render_bar_chart(&buckets);
    assert!(result.contains("<svg"));
}
