use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueCategory {
    ResponseCodes,
    Security,
    Url,
    PageTitles,
    MetaDescription,
    Headings,
    Content,
    Images,
    Canonicals,
    Directives,
    Hreflang,
    Links,
    StructuredData,
    Sitemaps,
    Performance,
    JavaScript,
}

impl fmt::Display for IssueCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResponseCodes => write!(f, "Response Codes"),
            Self::Security => write!(f, "Security"),
            Self::Url => write!(f, "URL"),
            Self::PageTitles => write!(f, "Page Titles"),
            Self::MetaDescription => write!(f, "Meta Description"),
            Self::Headings => write!(f, "Headings"),
            Self::Content => write!(f, "Content"),
            Self::Images => write!(f, "Images"),
            Self::Canonicals => write!(f, "Canonicals"),
            Self::Directives => write!(f, "Directives"),
            Self::Hreflang => write!(f, "Hreflang"),
            Self::Links => write!(f, "Links"),
            Self::StructuredData => write!(f, "Structured Data"),
            Self::Sitemaps => write!(f, "Sitemaps"),
            Self::Performance => write!(f, "Performance"),
            Self::JavaScript => write!(f, "JavaScript"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical = 0,
    Warning = 1,
    Info = 2,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Critical => write!(f, "Critical"),
            Self::Warning => write!(f, "Warning"),
            Self::Info => write!(f, "Info"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub category: IssueCategory,
    pub check: String,
    pub display_name: String,
    pub severity: Severity,
    pub description: String,
    pub guidance: String,
    pub urls: Vec<IssueUrl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
