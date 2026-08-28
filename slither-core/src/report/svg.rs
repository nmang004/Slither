use std::fmt::Write;

/// Render a circular score gauge as inline SVG.
pub fn render_score_gauge(score: u32, grade: &str, color: &str) -> String {
    let radius = 80.0_f64;
    let circumference = 2.0 * std::f64::consts::PI * radius;
    let progress = score as f64 / 100.0;
    let dash_offset = circumference * (1.0 - progress);

    format!(
        r##"<svg width="200" height="220" viewBox="0 0 200 220" xmlns="http://www.w3.org/2000/svg">
  <style>
    @keyframes gaugeArc {{ from {{ stroke-dashoffset: {circumference}; }} to {{ stroke-dashoffset: {dash_offset}; }} }}
  </style>
  <circle cx="100" cy="100" r="{radius}" fill="none" stroke="var(--track)" stroke-width="12"/>
  <circle cx="100" cy="100" r="{radius}" fill="none" stroke="{color}" stroke-width="12"
    stroke-dasharray="{circumference}" stroke-dashoffset="{circumference}"
    stroke-linecap="round" transform="rotate(-90 100 100)"
    style="animation: gaugeArc 1.2s ease-out forwards;"/>
  <text x="100" y="95" text-anchor="middle" fill="var(--text)" font-size="42" font-weight="800" font-family="var(--font)">{score}</text>
  <text x="100" y="125" text-anchor="middle" fill="{color}" font-size="28" font-weight="700" font-family="var(--font)">{grade}</text>
  <text x="100" y="205" text-anchor="middle" fill="var(--text-dim)" font-size="13" font-weight="600" letter-spacing="0.08em" font-family="var(--font)">HEALTH SCORE</text>
</svg>"##,
        radius = radius,
        circumference = circumference,
        dash_offset = dash_offset,
        color = color,
        score = score,
        grade = grade,
    )
}

/// Render a donut chart from percentage segments.
/// Each segment: (percentage, label, color).
pub fn render_donut_chart(segments: &[(f64, &str, &str)], total: u32) -> String {
    let radius = 70.0_f64;
    let circumference = 2.0 * std::f64::consts::PI * radius;
    let mut svg = String::from(
        r#"<svg width="300" height="200" viewBox="0 0 300 200" xmlns="http://www.w3.org/2000/svg">"#,
    );

    let mut offset = 0.0;
    for (pct, _label, color) in segments {
        let dash = circumference * pct / 100.0;
        let gap = circumference - dash;
        let _ = write!(
            svg,
            r##"<circle cx="100" cy="100" r="{}" fill="none" stroke="{}" stroke-width="24" stroke-dasharray="{} {}" stroke-dashoffset="{}" transform="rotate(-90 100 100)"/>"##,
            radius, color, dash, gap, -offset
        );
        offset += dash;
    }

    // Center text
    let _ = write!(
        svg,
        r##"<text x="100" y="105" text-anchor="middle" fill="var(--text)" font-size="24" font-weight="800" font-family="var(--font)">{}</text>"##,
        total
    );

    // Legend
    let mut y = 30;
    for (pct, label, color) in segments {
        let _ = write!(
            svg,
            r##"<rect x="210" y="{}" width="12" height="12" rx="3" fill="{}"/><text x="228" y="{}" fill="var(--text)" font-size="12" font-weight="500" font-family="var(--font)">{} <tspan fill="var(--text-dim)">({:.0}%)</tspan></text>"##,
            y,
            color,
            y + 10,
            label,
            pct
        );
        y += 24;
    }

    svg.push_str("</svg>");
    svg
}

/// Render a horizontal bar chart.
/// Each bucket: (label, count, color).
pub fn render_bar_chart(buckets: &[(String, u32, &str)]) -> String {
    if buckets.is_empty() {
        return r#"<svg width="540" height="0" viewBox="0 0 540 0" xmlns="http://www.w3.org/2000/svg"></svg>"#.to_string();
    }

    let max_count = buckets.iter().map(|(_, c, _)| *c).max().unwrap_or(1).max(1);
    let bar_height: u32 = 28;
    let gap: u32 = 8;
    let total_height = buckets.len() as u32 * (bar_height + gap);
    let chart_width: u32 = 400;
    let label_width: u32 = 80;
    let svg_width = chart_width + label_width + 60;

    let mut svg = format!(
        r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" xmlns="http://www.w3.org/2000/svg">"#,
        svg_width, total_height, svg_width, total_height
    );

    for (i, (label, count, color)) in buckets.iter().enumerate() {
        let y = i as u32 * (bar_height + gap);
        let bar_width = (*count as f64 / max_count as f64 * chart_width as f64) as u32;

        // Label
        let _ = write!(
            svg,
            r##"<text x="{}" y="{}" fill="var(--text-dim)" font-size="12" font-weight="500" font-family="var(--font)" text-anchor="end">{}</text>"##,
            label_width - 8,
            y + bar_height / 2 + 4,
            label
        );

        // Background bar
        let _ = write!(
            svg,
            r##"<rect x="{}" y="{}" width="{}" height="{}" rx="6" fill="var(--track)"/>"##,
            label_width, y, chart_width, bar_height,
        );

        // Bar
        let _ = write!(
            svg,
            r##"<rect x="{}" y="{}" width="{}" height="{}" rx="4" fill="{}" opacity="0.8"/>"##,
            label_width,
            y,
            bar_width.max(2),
            bar_height,
            color
        );

        // Count label
        let _ = write!(
            svg,
            r##"<text x="{}" y="{}" fill="var(--text)" font-size="12" font-weight="700" font-family="var(--font)">{}</text>"##,
            label_width + bar_width + 8,
            y + bar_height / 2 + 4,
            count
        );
    }

    svg.push_str("</svg>");
    svg
}
