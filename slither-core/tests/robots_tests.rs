use slither_core::crawler::robots::RobotsChecker;

#[tokio::test]
async fn test_robots_allows_path() {
    let robots_txt = "User-agent: *\nDisallow: /private/\nDisallow: /admin/\n";
    let checker = RobotsChecker::from_str(robots_txt, "Slither/0.1.0");
    assert!(checker.is_allowed("/public/page"));
    assert!(checker.is_allowed("/about"));
    assert!(!checker.is_allowed("/private/secret"));
    assert!(!checker.is_allowed("/admin/dashboard"));
}

#[tokio::test]
async fn test_robots_crawl_delay() {
    let robots_txt = "User-agent: *\nCrawl-delay: 5\nDisallow: /private/\n";
    let checker = RobotsChecker::from_str(robots_txt, "Slither/0.1.0");
    assert_eq!(checker.crawl_delay(), Some(5));
}

#[tokio::test]
async fn test_robots_empty() {
    let checker = RobotsChecker::from_str("", "Slither/0.1.0");
    assert!(checker.is_allowed("/anything"));
    assert_eq!(checker.crawl_delay(), None);
}

#[tokio::test]
async fn test_robots_disallow_all() {
    let robots_txt = "User-agent: *\nDisallow: /\n";
    let checker = RobotsChecker::from_str(robots_txt, "Slither/0.1.0");
    assert!(!checker.is_allowed("/anything"));
    // Root should also be disallowed
    assert!(!checker.is_allowed("/"));
}
