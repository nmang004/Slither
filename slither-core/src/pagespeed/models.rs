use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CwvStatus {
    Good,
    NeedsImprovement,
    Poor,
}

impl std::fmt::Display for CwvStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Good => write!(f, "good"),
            Self::NeedsImprovement => write!(f, "needs_improvement"),
            Self::Poor => write!(f, "poor"),
        }
    }
}

pub struct CwvThresholds;

impl CwvThresholds {
    pub fn lcp_status(ms: f64) -> CwvStatus {
        if ms <= 2500.0 {
            CwvStatus::Good
        } else if ms <= 4000.0 {
            CwvStatus::NeedsImprovement
        } else {
            CwvStatus::Poor
        }
    }

    pub fn inp_status(ms: f64) -> CwvStatus {
        if ms <= 200.0 {
            CwvStatus::Good
        } else if ms <= 500.0 {
            CwvStatus::NeedsImprovement
        } else {
            CwvStatus::Poor
        }
    }

    pub fn cls_status(value: f64) -> CwvStatus {
        if value <= 0.1 {
            CwvStatus::Good
        } else if value <= 0.25 {
            CwvStatus::NeedsImprovement
        } else {
            CwvStatus::Poor
        }
    }

    pub fn fcp_status(ms: f64) -> CwvStatus {
        if ms <= 1800.0 {
            CwvStatus::Good
        } else if ms <= 3000.0 {
            CwvStatus::NeedsImprovement
        } else {
            CwvStatus::Poor
        }
    }

    pub fn ttfb_status(ms: f64) -> CwvStatus {
        if ms <= 800.0 {
            CwvStatus::Good
        } else if ms <= 1800.0 {
            CwvStatus::NeedsImprovement
        } else {
            CwvStatus::Poor
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSpeedResult {
    pub url: String,
    pub performance_score: u32,
    pub lcp_ms: f64,
    /// INP is only available from CrUX field data (PSI navigation-mode
    /// Lighthouse emits no INP audit), so it is optional. `None` means "not
    /// measured" rather than a fabricated 0.0/"Good".
    pub inp_ms: Option<f64>,
    pub cls: f64,
    pub fcp_ms: f64,
    pub ttfb_ms: f64,
    pub strategy: String,
    pub source: String,
}

impl PageSpeedResult {
    pub fn overall_cwv_status(&self) -> CwvStatus {
        let mut statuses = vec![
            CwvThresholds::lcp_status(self.lcp_ms),
            CwvThresholds::cls_status(self.cls),
        ];
        // Only factor INP in when we actually have field data for it.
        if let Some(inp) = self.inp_ms {
            statuses.push(CwvThresholds::inp_status(inp));
        }
        if statuses.contains(&CwvStatus::Poor) {
            CwvStatus::Poor
        } else if statuses.contains(&CwvStatus::NeedsImprovement) {
            CwvStatus::NeedsImprovement
        } else {
            CwvStatus::Good
        }
    }
}
