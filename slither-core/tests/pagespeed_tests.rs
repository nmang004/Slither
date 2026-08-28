use slither_core::pagespeed::models::{CwvStatus, CwvThresholds, PageSpeedResult};

#[test]
fn test_cwv_status_lcp_good() {
    assert_eq!(CwvThresholds::lcp_status(2000.0), CwvStatus::Good);
}

#[test]
fn test_cwv_status_lcp_needs_improvement() {
    assert_eq!(
        CwvThresholds::lcp_status(3000.0),
        CwvStatus::NeedsImprovement
    );
}

#[test]
fn test_cwv_status_lcp_poor() {
    assert_eq!(CwvThresholds::lcp_status(5000.0), CwvStatus::Poor);
}

#[test]
fn test_cwv_status_inp_good() {
    assert_eq!(CwvThresholds::inp_status(100.0), CwvStatus::Good);
}

#[test]
fn test_cwv_status_inp_poor() {
    assert_eq!(CwvThresholds::inp_status(600.0), CwvStatus::Poor);
}

#[test]
fn test_cwv_status_cls_good() {
    assert_eq!(CwvThresholds::cls_status(0.05), CwvStatus::Good);
}

#[test]
fn test_cwv_status_cls_poor() {
    assert_eq!(CwvThresholds::cls_status(0.3), CwvStatus::Poor);
}

#[test]
fn test_overall_status_all_good() {
    let result = PageSpeedResult {
        url: "https://example.com".to_string(),
        performance_score: 95,
        lcp_ms: 1500.0,
        inp_ms: Some(100.0),
        cls: 0.05,
        fcp_ms: 1200.0,
        ttfb_ms: 400.0,
        strategy: "mobile".to_string(),
        source: "api".to_string(),
    };
    assert_eq!(result.overall_cwv_status(), CwvStatus::Good);
}

#[test]
fn test_overall_status_ignores_missing_inp() {
    // When INP field data is absent (None), it must not be scored as "Good" —
    // the status should reflect only the metrics we actually measured.
    let result = PageSpeedResult {
        url: "https://example.com".to_string(),
        performance_score: 70,
        lcp_ms: 3000.0, // needs improvement
        inp_ms: None,
        cls: 0.05,
        fcp_ms: 1200.0,
        ttfb_ms: 400.0,
        strategy: "mobile".to_string(),
        source: "api".to_string(),
    };
    assert_eq!(result.overall_cwv_status(), CwvStatus::NeedsImprovement);
}

#[test]
fn test_overall_status_one_poor() {
    let result = PageSpeedResult {
        url: "https://example.com".to_string(),
        performance_score: 50,
        lcp_ms: 5000.0,
        inp_ms: Some(100.0),
        cls: 0.05,
        fcp_ms: 1200.0,
        ttfb_ms: 400.0,
        strategy: "mobile".to_string(),
        source: "api".to_string(),
    };
    assert_eq!(result.overall_cwv_status(), CwvStatus::Poor);
}

// ---------------------------------------------------------------------------
// Boundary value tests for CWV thresholds
// ---------------------------------------------------------------------------

#[test]
fn test_cwv_boundary_lcp_exactly_2500_is_good() {
    assert_eq!(CwvThresholds::lcp_status(2500.0), CwvStatus::Good);
}

#[test]
fn test_cwv_boundary_lcp_exactly_4000_is_needs_improvement() {
    assert_eq!(
        CwvThresholds::lcp_status(4000.0),
        CwvStatus::NeedsImprovement
    );
}

#[test]
fn test_cwv_boundary_inp_exactly_200_is_good() {
    assert_eq!(CwvThresholds::inp_status(200.0), CwvStatus::Good);
}

#[test]
fn test_cwv_boundary_inp_exactly_500_is_needs_improvement() {
    assert_eq!(
        CwvThresholds::inp_status(500.0),
        CwvStatus::NeedsImprovement
    );
}

#[test]
fn test_cwv_boundary_cls_exactly_0_1_is_good() {
    assert_eq!(CwvThresholds::cls_status(0.1), CwvStatus::Good);
}

#[test]
fn test_cwv_boundary_cls_exactly_0_25_is_needs_improvement() {
    assert_eq!(CwvThresholds::cls_status(0.25), CwvStatus::NeedsImprovement);
}

#[test]
fn test_cwv_boundary_fcp_exactly_1800_is_good() {
    assert_eq!(CwvThresholds::fcp_status(1800.0), CwvStatus::Good);
}

#[test]
fn test_cwv_boundary_ttfb_exactly_800_is_good() {
    assert_eq!(CwvThresholds::ttfb_status(800.0), CwvStatus::Good);
}
