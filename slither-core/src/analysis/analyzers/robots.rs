use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use texting_robots::Robot;

/// Reports how the site's robots.txt treats major AI / LLM crawlers, and flags
/// AI opt-out signals (`noai`). This is a 2026-relevant concern: many sites now
/// deliberately allow or block training/answer-engine crawlers, and an SEO
/// wants that policy surfaced rather than buried.
pub struct RobotsAnalyzer;

/// (product token, human label) for the AI crawlers we report on.
const AI_CRAWLERS: &[(&str, &str)] = &[
    ("GPTBot", "OpenAI GPTBot (training)"),
    ("OAI-SearchBot", "OpenAI SearchBot (ChatGPT search)"),
    ("ChatGPT-User", "OpenAI ChatGPT-User (browsing)"),
    (
        "Google-Extended",
        "Google-Extended (Gemini/Vertex training)",
    ),
    ("ClaudeBot", "Anthropic ClaudeBot"),
    ("anthropic-ai", "Anthropic anthropic-ai"),
    ("PerplexityBot", "Perplexity"),
    ("CCBot", "Common Crawl (CCBot)"),
    ("Bytespider", "ByteDance Bytespider"),
    ("Amazonbot", "Amazon"),
    ("meta-externalagent", "Meta AI"),
];

impl Analyzer for RobotsAnalyzer {
    fn name(&self) -> &str {
        "Robots / AI crawlers"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Directives
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_ai_crawler_policy(ctx, &mut issues);
        self.check_noai_meta(ctx, &mut issues);
        issues
    }
}

impl RobotsAnalyzer {
    fn check_ai_crawler_policy(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let Some(robots_txt) = ctx.robots_txt.as_deref() else {
            return;
        };
        if robots_txt.trim().is_empty() {
            return;
        }

        // A crawler is "blocked" when it is disallowed from the site root. We
        // only report bots that the robots.txt explicitly addresses (a named
        // group), since a site with only `User-agent: *` is not making an
        // AI-specific decision.
        let mut blocked: Vec<String> = Vec::new();
        for (token, label) in AI_CRAWLERS {
            if !robots_txt_mentions(robots_txt, token) {
                continue;
            }
            if let Ok(robot) = Robot::new(token, robots_txt.as_bytes()) {
                if !robot.allowed("/") {
                    blocked.push((*label).to_string());
                }
            }
        }

        if !blocked.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Directives,
                check: "ai_crawlers_blocked".to_string(),
                display_name: "AI Crawlers Blocked in robots.txt".to_string(),
                severity: Severity::Info,
                description: "robots.txt disallows one or more AI/LLM crawlers".to_string(),
                guidance: format!(
                    "These AI crawlers are disallowed at the site root: {}. This is a policy \
                     choice — blocking training crawlers protects content but may reduce \
                     visibility in AI answer engines. Confirm it matches your intent.",
                    blocked.join(", ")
                ),
                urls: Vec::new(),
            });
        }
    }

    /// Flag pages that opt out of AI use via `<meta name="robots" content="noai">`
    /// or `noimageai` — an emerging signal search/AI crawlers honor.
    fn check_noai_meta(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.effective_robots_directives()
                    .iter()
                    .any(|d| d == "noai" || d == "noimageai")
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
                check: "noai_directive".to_string(),
                display_name: "AI Opt-Out (noai) Directive".to_string(),
                severity: Severity::Info,
                description: "Pages carrying a noai / noimageai directive".to_string(),
                guidance: "These pages ask AI systems not to use their content for training or \
                     generation. Verify this is intentional for the affected pages."
                    .to_string(),
                urls: affected,
            });
        }
    }
}

/// True if robots.txt has a `User-agent` line naming `token` (case-insensitive).
///
/// RFC 9309 §2.2: everything from `#` to end of line is a comment, so
/// `User-agent: GPTBot  # no AI training please` names GPTBot. Comparing the
/// whole raw value made the AI-crawler report vanish for any file that
/// annotates its user-agent lines — precisely the files whose authors are
/// documenting an AI-blocking decision. The crawl engine (texting_robots)
/// strips comments correctly, so this hand-rolled scan disagreed with it.
fn robots_txt_mentions(robots_txt: &str, token: &str) -> bool {
    let token_lower = token.to_ascii_lowercase();
    robots_txt.lines().any(|line| {
        let line = line
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        line.strip_prefix("user-agent:")
            .map(|v| v.trim() == token_lower)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(robots: Option<&str>) -> AnalysisContext {
        AnalysisContext {
            seed_url: "https://e.com".to_string(),
            domain: "e.com".to_string(),
            sitemap_data: None,
            pages: Vec::new(),
            robots_txt: robots.map(String::from),
        }
    }

    #[test]
    fn flags_blocked_ai_crawler() {
        let robots = "User-agent: GPTBot\nDisallow: /\n\nUser-agent: *\nAllow: /\n";
        let issues = RobotsAnalyzer.analyze(&ctx(Some(robots)));
        assert!(issues.iter().any(|i| i.check == "ai_crawlers_blocked"));
    }

    #[test]
    fn no_issue_when_ai_crawler_allowed() {
        let robots = "User-agent: *\nAllow: /\n";
        let issues = RobotsAnalyzer.analyze(&ctx(Some(robots)));
        assert!(!issues.iter().any(|i| i.check == "ai_crawlers_blocked"));
    }

    #[test]
    fn no_robots_no_issue() {
        let issues = RobotsAnalyzer.analyze(&ctx(None));
        assert!(issues.is_empty());
    }
}
