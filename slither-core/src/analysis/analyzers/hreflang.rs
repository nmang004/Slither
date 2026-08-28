use crate::analysis::{norm_url as norm, AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use std::collections::{HashMap, HashSet};

pub struct HreflangAnalyzer;

impl Analyzer for HreflangAnalyzer {
    fn name(&self) -> &str {
        "Hreflang"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Hreflang
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_missing_return_links(ctx, &mut issues);
        self.check_incorrect_lang_codes(ctx, &mut issues);
        self.check_non_200_urls(ctx, &mut issues);
        self.check_non_canonical_urls(ctx, &mut issues);
        self.check_noindex_urls(ctx, &mut issues);
        self.check_multiple_entries(ctx, &mut issues);
        self.check_conflicting_lang_declarations(ctx, &mut issues);
        self.check_missing_self_reference(ctx, &mut issues);
        self.check_missing_x_default(ctx, &mut issues);
        issues
    }
}

/// Validate an hreflang value as a BCP-47 language tag (the subset Google
/// accepts): `x-default`, a 2–3 letter language subtag, an optional 4-letter
/// script subtag, and an optional region subtag that is either a 2-letter
/// country code or a 3-digit UN M.49 code. Case-insensitive, since hreflang
/// matching is case-insensitive.
fn is_valid_lang_code(code: &str) -> bool {
    if code.eq_ignore_ascii_case("x-default") {
        return true;
    }
    // ISO 639-1 (2-letter) language codes; used to reject reversed forms like
    // `us-en` where "us" is a region, not a language.
    const ISO_639_1: &str = " aa ab ae af ak am an ar as av ay az ba be bg bh bi bm bn bo br bs ca ce ch co cr cs cu cv cy da de dv dz ee el en eo es et eu fa ff fi fj fo fr fy ga gd gl gn gu gv ha he hi ho hr ht hu hy hz ia id ie ig ii ik io is it iu ja jv ka kg ki kj kk kl km kn ko kr ks ku kv kw ky la lb lg li ln lo lt lu lv mg mh mi mk ml mn mr ms mt my na nb nd ne ng nl nn no nr nv ny oc oj om or os pa pi pl ps pt qu rm rn ro ru rw sa sc sd se sg si sk sl sm sn so sq sr ss st su sv sw ta te tg th ti tk tl tn to tr ts tt tw ty ug uk ur uz ve vi vo wa wo xh yi yo za zh zu ";

    // ISO 3166-1 alpha-2 country codes (plus the UK/EU style errors are absent
    // by construction, which is the point).
    const ISO_3166_1: &str = " AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW ";
    // The UN M.49 area codes Google documents as usable for targeting.
    const UN_M49: &str = " 001 002 005 009 011 013 014 015 017 018 019 021 029 030 034 035 039 053 054 057 061 142 143 145 150 151 154 155 202 419 ";

    let mut parts = code.split('-').peekable();

    // Language subtag: 2–3 ASCII letters (reject a leading region like "us-en").
    match parts.next() {
        Some(lang)
            if (2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            // A 2-letter subtag must be a real ISO 639-1 language.
            if lang.len() == 2 {
                let padded = format!(" {} ", lang.to_ascii_lowercase());
                if !ISO_639_1.contains(&padded) {
                    return false;
                }
            }
        }
        _ => return false,
    }

    // Optional script subtag: exactly 4 letters (e.g. Hant, Hans, Latn).
    if let Some(next) = parts.peek() {
        if next.len() == 4 && next.chars().all(|c| c.is_ascii_alphabetic()) {
            parts.next();
        }
    }

    // Optional region subtag. Google: "Only language codes listed in ISO 639-1
    // and region codes listed in ISO 3166-1 Alpha 2 are supported; other codes
    // aren't supported" — an unsupported code means the annotation is ignored.
    // Shape-checking alone let `en-UK` through, which is the single most common
    // real-world hreflang error (the country code for the UK is GB).
    if let Some(region) = parts.next() {
        let is_country = region.len() == 2
            && region.chars().all(|c| c.is_ascii_alphabetic())
            && ISO_3166_1.contains(&format!(" {} ", region.to_ascii_uppercase()));
        // UN M.49 area codes are also accepted (e.g. 419 for Latin America).
        let is_m49 = region.len() == 3
            && region.chars().all(|c| c.is_ascii_digit())
            && UN_M49.contains(&format!(" {} ", region));
        if !(is_country || is_m49) {
            return false;
        }
    }

    // No further subtags supported.
    parts.next().is_none()
}

impl HreflangAnalyzer {
    fn check_missing_return_links(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // Both sides of the join are normalized: an href written with a
        // different trailing slash or host casing than the crawled URL is the
        // same page, and comparing raw strings reports phantom missing return
        // links (and silently skips genuinely broken ones).
        let page_map: HashMap<String, &crate::models::page::PageData> =
            ctx.pages.iter().map(|p| (norm(&p.url), p)).collect();

        fn has_return_link(target: &crate::models::page::PageData, from: &str) -> bool {
            target.hreflang_tags.iter().any(|t| norm(&t.url) == from)
        }

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.hreflang_tags.is_empty())
            .filter(|p| {
                let self_url = norm(&p.url);
                p.hreflang_tags.iter().any(|tag| {
                    let target_url = norm(&tag.url);
                    if target_url == self_url {
                        return false; // skip self-references
                    }
                    match page_map.get(&target_url) {
                        // Check if target page has a hreflang pointing back
                        Some(target_page) => !has_return_link(target_page, &self_url),
                        None => false, // target not crawled, can't verify
                    }
                })
            })
            .map(|p| {
                let self_url = norm(&p.url);
                let missing: Vec<String> = p
                    .hreflang_tags
                    .iter()
                    .filter(|tag| {
                        let target_url = norm(&tag.url);
                        if target_url == self_url {
                            return false;
                        }
                        match page_map.get(&target_url) {
                            Some(target_page) => !has_return_link(target_page, &self_url),
                            None => false,
                        }
                    })
                    .map(|tag| format!("{} ({})", tag.url, tag.lang))
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Missing return links from: {}",
                        crate::analysis::detail_sample(&missing, missing.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "missing_return_links".to_string(),
                display_name: "Missing Return Links".to_string(),
                severity: Severity::Warning,
                description: "Hreflang tags without reciprocal return links".to_string(),
                guidance: "Hreflang annotations must be reciprocal. If page A declares page B as an alternate language version, page B must also declare page A. Add the missing return hreflang tags.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_incorrect_lang_codes(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.hreflang_tags
                    .iter()
                    .any(|tag| !is_valid_lang_code(&tag.lang))
            })
            .map(|p| {
                let invalid: Vec<String> = p
                    .hreflang_tags
                    .iter()
                    .filter(|tag| !is_valid_lang_code(&tag.lang))
                    .map(|tag| format!("\"{}\"", tag.lang))
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Invalid language codes: {}",
                        crate::analysis::detail_sample(&invalid, invalid.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "incorrect_lang_codes".to_string(),
                display_name: "Invalid Language Codes".to_string(),
                severity: Severity::Warning,
                description: "Hreflang tags with invalid language codes".to_string(),
                guidance: "Language codes must be a valid ISO 639-1 code (e.g., 'en'), optionally followed by an ISO 3166-1 alpha-2 region code (e.g., 'en-US'), or 'x-default'. Fix the invalid language codes.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_non_200_urls(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let status_map: HashMap<String, u16> =
            ctx.pages.iter().map(|p| (norm(&p.url), p.status)).collect();

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.hreflang_tags.is_empty())
            .filter(|p| {
                p.hreflang_tags
                    .iter()
                    .any(|tag| status_map.get(&norm(&tag.url)).is_some_and(|&s| s != 200))
            })
            .map(|p| {
                let bad: Vec<String> = p
                    .hreflang_tags
                    .iter()
                    .filter_map(|tag| {
                        status_map.get(&norm(&tag.url)).and_then(|&s| {
                            if s != 200 {
                                Some(format!("{} (status {})", tag.url, s))
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Non-200 targets: {}",
                        crate::analysis::detail_sample(&bad, bad.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "non_200_urls".to_string(),
                display_name: "Hreflang to Non-200 Pages".to_string(),
                severity: Severity::Warning,
                description: "Hreflang tags pointing to non-200 status pages".to_string(),
                guidance: "Hreflang alternate URLs should return a 200 status code. Non-200 pages (redirects, errors) confuse search engines about the correct alternate version. Update hreflang tags to point to the final, accessible URLs.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_non_canonical_urls(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let page_map: HashMap<String, &crate::models::page::PageData> =
            ctx.pages.iter().map(|p| (norm(&p.url), p)).collect();

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.hreflang_tags.is_empty())
            .filter(|p| {
                let self_url = norm(&p.url);
                p.hreflang_tags.iter().any(|tag| {
                    // Google requires each version to list itself, so a page's
                    // own self-reference is never the actionable finding — the
                    // page that points AT it is, and that page is reported
                    // separately. Counting both inflated affected_urls past the
                    // crawl size.
                    if norm(&tag.url) == self_url {
                        return false;
                    }
                    if let Some(target) = page_map.get(&norm(&tag.url)) {
                        // Target has a canonical that points elsewhere
                        target
                            .canonical
                            .as_ref()
                            .is_some_and(|c| norm(c) != norm(&target.url))
                    } else {
                        false
                    }
                })
            })
            .map(|p| {
                let self_url = norm(&p.url);
                let bad: Vec<String> = p
                    .hreflang_tags
                    .iter()
                    .filter(|tag| norm(&tag.url) != self_url)
                    .filter_map(|tag| {
                        page_map.get(&norm(&tag.url)).and_then(|target| {
                            target.canonical.as_ref().and_then(|c| {
                                if norm(c) != norm(&target.url) {
                                    Some(format!("{} (canonical: {})", tag.url, c))
                                } else {
                                    None
                                }
                            })
                        })
                    })
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Non-canonical targets: {}",
                        crate::analysis::detail_sample(&bad, bad.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "non_canonical_urls".to_string(),
                display_name: "Hreflang to Non-Canonical".to_string(),
                severity: Severity::Warning,
                description: "Hreflang tags pointing to non-canonical pages".to_string(),
                guidance: "Hreflang tags should point to canonical URLs. If the target page has a different canonical URL, search engines may ignore the hreflang annotation. Update hreflang tags to use the canonical versions.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_noindex_urls(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let page_map: HashMap<String, &crate::models::page::PageData> =
            ctx.pages.iter().map(|p| (norm(&p.url), p)).collect();

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.hreflang_tags.is_empty())
            .filter(|p| {
                let self_url = norm(&p.url);
                p.hreflang_tags.iter().any(|tag| {
                    // See check_non_canonical_urls: a required self-reference is
                    // not a bad target.
                    if norm(&tag.url) == self_url {
                        return false;
                    }
                    if let Some(target) = page_map.get(&norm(&tag.url)) {
                        target.is_noindex()
                    } else {
                        false
                    }
                })
            })
            .map(|p| {
                let self_url = norm(&p.url);
                let bad: Vec<String> = p
                    .hreflang_tags
                    .iter()
                    .filter(|tag| norm(&tag.url) != self_url)
                    .filter(|tag| {
                        page_map
                            .get(&norm(&tag.url))
                            .is_some_and(|target| target.is_noindex())
                    })
                    .map(|tag| tag.url.clone())
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Noindexed targets: {}",
                        crate::analysis::detail_sample(&bad, bad.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "noindex_urls".to_string(),
                display_name: "Hreflang to Noindex Pages".to_string(),
                severity: Severity::Warning,
                description: "Hreflang tags pointing to noindexed pages".to_string(),
                guidance: "Hreflang alternate URLs should be indexable. Pointing to noindexed pages means the alternate version won't appear in search results. Either remove the noindex directive from the target or remove the hreflang annotation.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_multiple_entries(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                let mut seen = HashSet::new();
                p.hreflang_tags
                    .iter()
                    .any(|tag| !seen.insert(tag.lang.to_lowercase()))
            })
            .map(|p| {
                let mut counts: HashMap<String, u32> = HashMap::new();
                for tag in &p.hreflang_tags {
                    *counts.entry(tag.lang.to_lowercase()).or_insert(0) += 1;
                }
                let dupes: Vec<String> = counts
                    .iter()
                    .filter(|(_, &count)| count > 1)
                    .map(|(lang, count)| format!("{} ({}x)", lang, count))
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Duplicate language codes: {}",
                        crate::analysis::detail_sample(&dupes, dupes.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "multiple_entries".to_string(),
                display_name: "Duplicate Language Entries".to_string(),
                severity: Severity::Warning,
                description: "Pages with duplicate hreflang language codes".to_string(),
                guidance: "Each language/region code should appear only once in a page's hreflang annotations. Duplicate entries confuse search engines about which URL to use for that language. Remove duplicate entries.".to_string(),
                urls: affected,
            });
        }
    }

    /// Two pages that reciprocate correctly but disagree about what language
    /// the other one is in.
    ///
    /// `has_return_link` only compares URLs, so a cluster where A calls B German
    /// while B calls itself French satisfies reciprocity and passes silently.
    /// The annotation is self-contradictory and search engines discard clusters
    /// they cannot resolve.
    fn check_conflicting_lang_declarations(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let page_map: HashMap<String, &crate::models::page::PageData> =
            ctx.pages.iter().map(|p| (norm(&p.url), p)).collect();

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.hreflang_tags.is_empty())
            .filter_map(|p| {
                let self_url = norm(&p.url);
                let conflicts: Vec<String> = p
                    .hreflang_tags
                    .iter()
                    .filter(|tag| norm(&tag.url) != self_url)
                    .filter_map(|tag| {
                        let target_url = norm(&tag.url);
                        let target = page_map.get(&target_url)?;
                        // What does the target call itself?
                        let own = target
                            .hreflang_tags
                            .iter()
                            .find(|t| norm(&t.url) == target_url)?;
                        if own.lang.eq_ignore_ascii_case(&tag.lang) {
                            None
                        } else {
                            Some(format!(
                                "{} declared as \"{}\" here but self-declares \"{}\"",
                                tag.url, tag.lang, own.lang
                            ))
                        }
                    })
                    .collect();
                if conflicts.is_empty() {
                    return None;
                }
                Some(IssueUrl {
                    url: p.url.clone(),
                    detail: Some(crate::analysis::detail_sample(&conflicts, conflicts.len())),
                })
            })
            .collect();

        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "conflicting_lang_declarations".to_string(),
                display_name: "Conflicting Hreflang Languages".to_string(),
                severity: Severity::Warning,
                description: "Pages disagree about an alternate version's language".to_string(),
                guidance: "Two pages in the same hreflang cluster declare different language codes for the same URL. Search engines cannot resolve a contradictory cluster and may ignore the annotations. Make every page use the same language code for a given alternate.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_self_reference(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                let self_url = norm(&p.url);
                !p.hreflang_tags.is_empty()
                    && !p.hreflang_tags.iter().any(|tag| norm(&tag.url) == self_url)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: None,
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "missing_self_reference".to_string(),
                display_name: "Missing Self-Reference".to_string(),
                severity: Severity::Info,
                description: "Pages with hreflang tags but no self-referencing entry".to_string(),
                guidance: "Each page with hreflang annotations should include a self-referencing entry for its own language/region. This helps search engines confirm the page's language assignment.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_x_default(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                !p.hreflang_tags.is_empty()
                    && !p
                        .hreflang_tags
                        .iter()
                        .any(|tag| tag.lang.to_lowercase() == "x-default")
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: None,
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Hreflang,
                check: "missing_x_default".to_string(),
                display_name: "Missing x-default".to_string(),
                severity: Severity::Info,
                description: "Pages with hreflang tags but no x-default entry".to_string(),
                guidance: "Include an x-default hreflang entry to specify the fallback page for users whose language/region doesn't match any of the declared alternates.".to_string(),
                urls: affected,
            });
        }
    }
}

#[cfg(test)]
mod lang_code_tests {
    use super::is_valid_lang_code;

    #[test]
    fn accepts_valid_bcp47() {
        for c in [
            "en",
            "x-default",
            "en-GB",
            "en-us",
            "zh-Hant",
            "zh-Hant-TW",
            "es-419",
            "pt-BR",
        ] {
            assert!(is_valid_lang_code(c), "{c} should be valid");
        }
    }

    #[test]
    fn rejects_invalid() {
        for c in ["us-en", "en-1a", "toolongtag", "e", "en-XYZ"] {
            assert!(!is_valid_lang_code(c), "{c} should be invalid");
        }
    }
}
