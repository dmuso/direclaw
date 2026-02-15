use serde::Deserialize;

pub fn cmd_update(args: &[String]) -> Result<String, String> {
    if args.is_empty() || args[0] == "check" {
        return cmd_update_check();
    }
    if args[0] == "apply" {
        return Err(
            "update apply is unsupported in this build to avoid unsafe in-place upgrades. remediation: visit GitHub Releases, download the target archive, verify SHA256, and replace the binary manually".to_string(),
        );
    }
    Err("usage: update [check|apply]".to_string())
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubLatestRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    prerelease: bool,
    draft: bool,
    assets: Vec<GithubReleaseAsset>,
}

fn normalize_version_for_compare(raw: &str) -> String {
    raw.trim().trim_start_matches('v').to_ascii_lowercase()
}

fn parse_version_numbers(raw: &str) -> Option<Vec<u64>> {
    let trimmed = normalize_version_for_compare(raw);
    let core = trimmed
        .split_once('-')
        .map(|(left, _)| left)
        .unwrap_or(&trimmed);
    if core.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    for part in core.split('.') {
        if part.is_empty() {
            return None;
        }
        out.push(part.parse::<u64>().ok()?);
    }
    Some(out)
}

fn is_update_available(current: &str, latest: &str) -> bool {
    if let (Some(mut current_parts), Some(mut latest_parts)) = (
        parse_version_numbers(current),
        parse_version_numbers(latest),
    ) {
        let max_len = current_parts.len().max(latest_parts.len());
        current_parts.resize(max_len, 0);
        latest_parts.resize(max_len, 0);
        return latest_parts > current_parts;
    }
    normalize_version_for_compare(current) != normalize_version_for_compare(latest)
}

fn update_repo() -> String {
    std::env::var("DIRECLAW_UPDATE_REPO").unwrap_or_else(|_| "dharper/rustyclaw".to_string())
}

fn update_api_base() -> String {
    std::env::var("DIRECLAW_UPDATE_API_URL")
        .unwrap_or_else(|_| "https://api.github.com".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn load_latest_release(repo: &str) -> Result<GithubLatestRelease, String> {
    let (owner, name) = repo.split_once('/').ok_or_else(|| {
        format!("update check failed: repository `{repo}` must use `owner/name` format")
    })?;
    let url = format!(
        "{}/repos/{}/{}/releases/latest",
        update_api_base(),
        urlencoding::encode(owner),
        urlencoding::encode(name)
    );
    let response = ureq::get(&url)
        .set("accept", "application/vnd.github+json")
        .set(
            "user-agent",
            concat!("direclaw/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| format!("update check failed to query {url}: {e}"))?;

    let status = response.status();
    if status != 200 {
        return Err(format!(
            "update check failed to query {url}: unexpected status {status}"
        ));
    }

    response
        .into_json::<GithubLatestRelease>()
        .map_err(|e| format!("update check failed to parse release metadata: {e}"))
}

fn cmd_update_check() -> Result<String, String> {
    let repo = update_repo();
    let release = load_latest_release(&repo).map_err(|err| {
        format!(
            "{err}. remediation: verify network access and set DIRECLAW_UPDATE_REPO/DIRECLAW_UPDATE_API_URL if needed"
        )
    })?;
    if release.draft {
        return Err(
            "update check failed: latest release is a draft and cannot be used for updates"
                .to_string(),
        );
    }

    let current_version = env!("CARGO_PKG_VERSION");
    let latest_version = normalize_version_for_compare(&release.tag_name);
    let update_available = is_update_available(current_version, &latest_version);

    let mut lines = vec![
        "update_check=ok".to_string(),
        format!("repository={repo}"),
        format!("current_version={current_version}"),
        format!("latest_version={latest_version}"),
        format!("release_tag={}", release.tag_name),
        format!("release_url={}", release.html_url),
        format!("prerelease={}", release.prerelease),
        format!("update_available={update_available}"),
    ];
    if let Some(published_at) = release.published_at {
        lines.push(format!("published_at={published_at}"));
    }
    if !release.assets.is_empty() {
        let mut names: Vec<String> = release.assets.into_iter().map(|asset| asset.name).collect();
        names.sort();
        lines.push(format!("assets={}", names.join(",")));
    }
    if update_available {
        lines.push(
            "remediation=download release archive, verify SHA256 from checksums.txt, replace binary manually"
                .to_string(),
        );
    }
    Ok(lines.join("\n"))
}
