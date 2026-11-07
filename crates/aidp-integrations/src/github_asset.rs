//! Images and videos of pull requests and comments, read on the viewer's behalf: a private
//! repository's attachments answer a login page to anyone not signed in to GitHub.

use crate::gh::gh_stdout;
use crate::models::constants::github_asset::{
    AMPERSAND, ANONYMOUS_HOSTS, ATTACHMENTS_PATH_PREFIX, ATTRIBUTE_END, AUTHORIZATION_HEADER,
    BEARER_PREFIX, GH_API, GH_AUTH, GH_FIELD_FLAG, GH_TOKEN, GH_TOKEN_LABEL, GITHUB_HOST,
    HTML_AMPERSAND, HTTPS_SCHEME, MARKDOWN_CONTEXT_FIELD, MARKDOWN_MODE_FIELD, MARKDOWN_PATH,
    MARKDOWN_TEXT_FIELD, MAX_ASSET_BYTES, MEDIA_PREFIXES, MESSAGE_NOT_MEDIA, MESSAGE_NO_SIGNED_URL,
    MESSAGE_REPO_REQUIRED, MESSAGE_TOO_LARGE, MESSAGE_URL_NOT_ALLOWED, PATH_SEPARATOR, RAW_HOST,
    REPO_IMAGE_SEGMENTS, REPO_IMAGE_SEGMENT_INDEX, SLUG_EXTRA_CHARS, SLUG_SEPARATOR, SRC_ATTRIBUTE,
    TOKEN_TTL,
};
use crate::models::github_asset::GithubAsset;
use crate::net::asset_agent;
use crate::squad::GhRunner;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use url::Url;

fn is_github_media_path(path: &str) -> bool {
    path.starts_with(ATTACHMENTS_PATH_PREFIX)
        || path
            .split(PATH_SEPARATOR)
            .nth(REPO_IMAGE_SEGMENT_INDEX)
            .is_some_and(|segment| REPO_IMAGE_SEGMENTS.contains(&segment))
}

/// `raw` parsed when it names a file GitHub hosts, over HTTPS on its default port.
pub fn allowed_asset_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| MESSAGE_URL_NOT_ALLOWED.to_string())?;
    let host = url.host_str().unwrap_or_default();
    let hosted = ANONYMOUS_HOSTS.contains(&host)
        || host == RAW_HOST
        || (host == GITHUB_HOST && is_github_media_path(url.path()));
    if url.scheme() == HTTPS_SCHEME && url.port().is_none() && hosted {
        Ok(url)
    } else {
        Err(MESSAGE_URL_NOT_ALLOWED.to_string())
    }
}

/// True for a file served by `github.com` itself: it is read through a signed address.
pub fn is_attachment(url: &Url) -> bool {
    url.host_str() == Some(GITHUB_HOST)
}

/// True for a raw repository file, which the `gh` token opens.
pub fn needs_token(url: &Url) -> bool {
    url.host_str() == Some(RAW_HOST)
}

/// `repo` when it is a plain `owner/name` slug.
pub fn checked_repo(repo: Option<&str>) -> Result<&str, String> {
    let valid_part = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || SLUG_EXTRA_CHARS.contains(&c))
    };
    repo.filter(|slug| {
        slug.split_once(SLUG_SEPARATOR)
            .is_some_and(|(owner, name)| valid_part(owner) && valid_part(name))
    })
    .ok_or_else(|| MESSAGE_REPO_REQUIRED.to_string())
}

/// The short-lived signed address of an attachment: GitHub's Markdown API renders it for a
/// member of `repo`, the only way an API client can read a private repository's attachment.
pub fn signed_attachment_url(url: &Url, repo: &str, runner: &dyn GhRunner) -> Result<Url, String> {
    let args = [
        GH_API.to_string(),
        MARKDOWN_PATH.to_string(),
        GH_FIELD_FLAG.to_string(),
        format!("{MARKDOWN_TEXT_FIELD}![]({url})"),
        GH_FIELD_FLAG.to_string(),
        MARKDOWN_MODE_FIELD.to_string(),
        GH_FIELD_FLAG.to_string(),
        format!("{MARKDOWN_CONTEXT_FIELD}{repo}"),
    ];
    let html = runner.run(&args)?;
    let source = html
        .split_once(SRC_ATTRIBUTE)
        .and_then(|(_, rest)| rest.split(ATTRIBUTE_END).next())
        .ok_or_else(|| MESSAGE_NO_SIGNED_URL.to_string())?;
    let signed = allowed_asset_url(&source.replace(HTML_AMPERSAND, AMPERSAND))?;
    if is_attachment(&signed) {
        return Err(MESSAGE_NO_SIGNED_URL.to_string());
    }
    Ok(signed)
}

/// The `gh` user's token, asked to `gh` at most once per [`TOKEN_TTL`].
pub fn gh_token() -> Result<String, String> {
    static TOKEN: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    let cell = TOKEN.get_or_init(|| Mutex::new(None));
    let mut cached = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((token, read_at)) = cached.as_ref() {
        if read_at.elapsed() < TOKEN_TTL {
            return Ok(token.clone());
        }
    }
    let token = gh_stdout(GH_TOKEN_LABEL, &[GH_AUTH, GH_TOKEN])
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    *cached = Some((token.clone(), Instant::now()));
    Ok(token)
}

fn is_media(content_type: &str) -> bool {
    MEDIA_PREFIXES
        .iter()
        .any(|prefix| content_type.starts_with(prefix))
}

/// Reads the image or video at `url`, with `token` when given; anything else is refused.
pub fn fetch_asset(url: &Url, token: Option<&str>) -> Result<GithubAsset, String> {
    let mut request = asset_agent().get(url.as_str());
    if let Some(token) = token {
        request = request.set(AUTHORIZATION_HEADER, &format!("{BEARER_PREFIX}{token}"));
    }
    let response = request.call().map_err(|error| error.to_string())?;
    let content_type = response.content_type().to_string();
    if !is_media(&content_type) {
        return Err(MESSAGE_NOT_MEDIA.to_string());
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_ASSET_BYTES {
        return Err(MESSAGE_TOO_LARGE.to_string());
    }
    Ok(GithubAsset {
        bytes,
        content_type,
    })
}

#[cfg(test)]
#[path = "../tests/unit/github_asset_tests.rs"]
mod tests;
