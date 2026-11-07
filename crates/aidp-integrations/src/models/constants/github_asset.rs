//! Reading the images and videos of pull requests and comments: GitHub hosts only.

use std::time::Duration;

pub const HTTPS_SCHEME: &str = "https";
pub const GITHUB_HOST: &str = "github.com";
pub const RAW_HOST: &str = "raw.githubusercontent.com";
/// Hosts serving public or pre-signed files: read without a token.
pub const ANONYMOUS_HOSTS: [&str; 5] = [
    "private-user-images.githubusercontent.com",
    "user-images.githubusercontent.com",
    "objects.githubusercontent.com",
    "avatars.githubusercontent.com",
    "camo.githubusercontent.com",
];
/// `github.com` path of the files dropped in a description or comment.
pub const ATTACHMENTS_PATH_PREFIX: &str = "/user-attachments/";
/// Third path segment of a repository image on `github.com`: `/<owner>/<repo>/assets/...`.
pub const REPO_IMAGE_SEGMENTS: [&str; 2] = ["assets", "raw"];
pub const REPO_IMAGE_SEGMENT_INDEX: usize = 3;
pub const PATH_SEPARATOR: char = '/';
pub const SLUG_SEPARATOR: char = '/';
pub const SLUG_EXTRA_CHARS: [char; 3] = ['-', '_', '.'];

/// Media types served back to the viewer: images, and the videos GitHub accepts as attachments.
pub const MEDIA_PREFIXES: [&str; 2] = ["image/", "video/"];
pub const MAX_ASSET_BYTES: u64 = 30 * 1024 * 1024;
pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const BEARER_PREFIX: &str = "Bearer ";
pub const GH_API: &str = "api";
pub const GH_AUTH: &str = "auth";
pub const GH_TOKEN: &str = "token";
pub const GH_TOKEN_LABEL: &str = "gh auth token";
pub const GH_FIELD_FLAG: &str = "-f";
pub const MARKDOWN_PATH: &str = "markdown";
pub const MARKDOWN_MODE_FIELD: &str = "mode=gfm";
pub const MARKDOWN_TEXT_FIELD: &str = "text=";
pub const MARKDOWN_CONTEXT_FIELD: &str = "context=";
/// The rendered attachment's signed address sits in the first `src` attribute.
pub const SRC_ATTRIBUTE: &str = "src=\"";
pub const ATTRIBUTE_END: char = '"';
pub const HTML_AMPERSAND: &str = "&amp;";
pub const AMPERSAND: &str = "&";
/// How long the `gh` token is reused before `gh` is asked again.
pub const TOKEN_TTL: Duration = Duration::from_secs(600);

pub const MESSAGE_URL_NOT_ALLOWED: &str = "only images and videos hosted by GitHub can be read";
pub const MESSAGE_REPO_REQUIRED: &str =
    "repo=<owner>/<name> is required to read a GitHub attachment";
pub const MESSAGE_NO_SIGNED_URL: &str = "GitHub returned no signed address for the attachment";
pub const MESSAGE_NOT_MEDIA: &str = "the URL does not point to an image or a video";
pub const MESSAGE_TOO_LARGE: &str = "the file is larger than 30 MB";
