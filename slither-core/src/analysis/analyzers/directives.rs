use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

pub struct DirectivesAnalyzer;

impl Analyzer for DirectivesAnalyzer {
    fn name(&self) -> &str {
        "Directives"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Directives
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_conflicting_directives(ctx, &mut issues);
        self.check_noindex(ctx, &mut issues);
        self.check_nofollow(ctx, &mut issues);
        self.check_none(ctx, &mut issues);
        self.check_nosnippet(ctx, &mut issues);
        self.check_noimageindex(ctx, &mut issues);
        issues
    }
}

impl DirectivesAnalyzer {
    /// Directives that carry their own colon-separated value, so a leading
    /// `<token>:` on an X-Robots-Tag is part of the directive rather than a
    /// user-agent scope.
    const VALUED_DIRECTIVES: [&'static str; 4] = [
        "max-snippet",
        "max-image-preview",
        "max-video-preview",
        "unavailable_after",
    ];

    /// True if an `X-Robots-Tag` value is scoped to a specific crawler, e.g.
    /// `X-Robots-Tag: bingbot: noindex`.
    ///
    /// Every other check in this tool reports what Google sees, and a directive
    /// addressed to another crawler does not apply to Google. Treating the scope
    /// as an unscoped directive invented a noindex page and a maximum-severity
    /// "conflict" on a page that is fully indexable.
    fn is_ua_scoped(header: &str) -> bool {
        header
            .split(',')
            .any(|part| match part.trim().split_once(':') {
                Some((head, _)) => {
                    let head = head.trim().to_ascii_lowercase();
                    !head.is_empty()
                        && !Self::VALUED_DIRECTIVES.contains(&head.as_str())
                        && head
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                }
                None => false,
            })
    }

    /// Expand the aliases Google documents: `none` is `noindex, nofollow` and
    /// `all` is `index, follow`. Without this, "index,follow + none" and
    /// "all + noindex" — textbook contradictions — went undetected while the
    /// tool held both tokens in hand.
    fn expand_aliases(directives: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for d in directives {
            match d.as_str() {
                "none" => {
                    out.push("noindex".to_string());
                    out.push("nofollow".to_string());
                }
                "all" => {
                    out.push("index".to_string());
                    out.push("follow".to_string());
                }
                other => out.push(other.to_string()),
            }
        }
        out
    }

    /// Collect the effective directive tokens from both sources, or `None` when
    /// the header is scoped to another crawler.
    fn conflict_tokens(page: &crate::models::page::PageData) -> Option<Vec<String>> {
        let header = page.x_robots_tag.as_ref()?;
        if page.meta_robots_directives.is_empty() {
            return None;
        }
        if Self::is_ua_scoped(header) {
            return None;
        }
        let header_directives: Vec<String> = header
            .split(',')
            .flat_map(|s| s.split_whitespace())
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let meta_directives: Vec<String> = page
            .meta_robots_directives
            .iter()
            .map(|d| d.to_ascii_lowercase())
            .collect();
        let mut all = Self::expand_aliases(&meta_directives);
        all.extend(Self::expand_aliases(&header_directives));
        Some(all)
    }

    fn check_conflicting_directives(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // Split by what the contradiction actually costs. An index/noindex
        // contradiction decides whether the page exists in the index at all; a
        // follow/nofollow one only affects how link equity flows and leaves the
        // page fully indexed. Scoring them identically drove a completely
        // indexable site to 0/F.
        let mut index_conflicts: Vec<IssueUrl> = Vec::new();
        let mut follow_conflicts: Vec<IssueUrl> = Vec::new();

        for p in &ctx.pages {
            let Some(all) = Self::conflict_tokens(p) else {
                continue;
            };
            let has = |t: &str| all.iter().any(|d| d == t);
            let detail = || {
                Some(format!(
                    "Meta robots: [{}], X-Robots-Tag: {}",
                    p.meta_robots_directives.join(", "),
                    p.x_robots_tag.as_deref().unwrap_or("")
                ))
            };
            if has("index") && has("noindex") {
                index_conflicts.push(IssueUrl {
                    url: p.url.clone(),
                    detail: detail(),
                });
            } else if has("follow") && has("nofollow") {
                follow_conflicts.push(IssueUrl {
                    url: p.url.clone(),
                    detail: detail(),
                });
            }
        }

        if !index_conflicts.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Directives,
                check: "conflicting_directives".to_string(),
                display_name: "Conflicting Index Directives".to_string(),
                severity: Severity::Warning,
                description: "Pages declaring both index and noindex".to_string(),
                guidance: "The meta robots tag and the X-Robots-Tag header contradict each other about whether this page may be indexed. Search engines apply the most restrictive reading, so the page is likely dropped from the index. Align both sources.".to_string(),
                urls: index_conflicts,
            });
        }
        if !follow_conflicts.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Directives,
                check: "conflicting_follow_directives".to_string(),
                display_name: "Conflicting Follow Directives".to_string(),
                severity: Severity::Warning,
                description: "Pages declaring both follow and nofollow".to_string(),
                guidance: "The meta robots tag and the X-Robots-Tag header disagree about whether links on this page should be followed. The page stays indexable, but link discovery and equity flow become unpredictable. Align both sources.".to_string(),
                urls: follow_conflicts,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_directive(
        &self,
        ctx: &AnalysisContext,
        issues: &mut Vec<Issue>,
        directive: &str,
        check_name: &str,
        display_name: &str,
        description: &str,
        guidance: &str,
    ) {
        let directive_lower = directive.to_lowercase();
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                // Consider both the meta tag and the X-Robots-Tag header, but
                // not a header scoped to another crawler — `X-Robots-Tag:
                // bingbot: noindex` does not make the page noindex for Google,
                // and reporting it as such invented a noindexed page.
                if p.x_robots_tag.as_deref().is_some_and(Self::is_ua_scoped)
                    && !p
                        .meta_robots_directives
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(&directive_lower))
                {
                    return false;
                }
                p.effective_robots_directives()
                    .iter()
                    .any(|d| d == &directive_lower)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "Directives: [{}]",
                    p.effective_robots_directives().join(", ")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Directives,
                check: check_name.to_string(),
                display_name: display_name.to_string(),
                severity: Severity::Info,
                description: description.to_string(),
                guidance: guidance.to_string(),
                urls: affected,
            });
        }
    }

    fn check_noindex(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        self.check_directive(
            ctx,
            issues,
            "noindex",
            "noindex",
            "Noindex Pages",
            "Pages with noindex directive",
            "These pages are excluded from search engine indexes. Verify this is intentional, as noindexed pages will not appear in search results.",
        );
    }

    fn check_nofollow(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        self.check_directive(
            ctx,
            issues,
            "nofollow",
            "nofollow",
            "Nofollow Pages",
            "Pages with nofollow directive",
            "Search engines will not follow links on these pages for ranking purposes. Verify this is intentional, as it prevents link equity from flowing through these pages.",
        );
    }

    fn check_none(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        self.check_directive(
            ctx,
            issues,
            "none",
            "none",
            "None Directive (Noindex + Nofollow)",
            "Pages with none directive (equivalent to noindex, nofollow)",
            "The 'none' directive is equivalent to 'noindex, nofollow'. These pages are excluded from search indexes and their links are not followed. Verify this is intentional.",
        );
    }

    fn check_nosnippet(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        self.check_directive(
            ctx,
            issues,
            "nosnippet",
            "nosnippet",
            "Nosnippet Pages",
            "Pages with nosnippet directive",
            "Search engines will not show a text snippet or video preview for these pages in search results. This may reduce click-through rates.",
        );
    }

    fn check_noimageindex(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        self.check_directive(
            ctx,
            issues,
            "noimageindex",
            "noimageindex",
            "Noimageindex Pages",
            "Pages with noimageindex directive",
            "Images on these pages will not be indexed by search engines. Verify this is intentional if the page contains important visual content.",
        );
    }
}
