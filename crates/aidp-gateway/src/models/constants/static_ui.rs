//! Constants of the web UI build served by `aidp gateway serve --ui-dir`.

/// Environment variable naming the UI directory when `--ui-dir` is absent.
pub const UI_DIR_ENV: &str = "AIDP_UI_DIR";
/// Path prefix that always routes to the API, never to the UI.
pub const API_PREFIX: &str = "/api";
/// Entry document of the UI, the fallback of every client-side route.
pub const INDEX_FILE: &str = "index.html";
/// First path segment of the hashed build assets, cached for a year.
pub const ASSETS_SEGMENT: &str = "assets";
/// Cache policy of a hashed asset: its name changes with its content.
pub const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Cache policy of `index.html` and of every unhashed file.
pub const CACHE_REVALIDATE: &str = "no-cache";
/// Content type of a file whose extension is not in [`MIME_TYPES`].
pub const MIME_DEFAULT: &str = "application/octet-stream";
/// Content type of `index.html`.
pub const MIME_HTML: &str = "text/html; charset=utf-8";
/// Extension separator of a file name.
pub const EXTENSION_SEPARATOR: char = '.';
/// Path segment separator of a URL.
pub const SEGMENT_SEPARATOR: char = '/';
/// Segment naming the current directory, ignored.
pub const CURRENT_SEGMENT: &str = ".";
/// Segment naming the parent directory, refused.
pub const PARENT_SEGMENT: &str = "..";
/// Characters a decoded segment may not hold: separators, drive marks and NUL.
pub const FORBIDDEN_SEGMENT_CHARS: &[char] = &['\\', ':', '\0'];
/// Request header listing the encodings the client accepts.
pub const HEADER_ACCEPT_ENCODING: &str = "Accept-Encoding";
/// Response header naming the encoding of a precompressed variant.
pub const HEADER_CONTENT_ENCODING: &str = "Content-Encoding";
/// Response header telling caches the body depends on `Accept-Encoding`.
pub const HEADER_VARY: &str = "Vary";
/// Brotli token of `Accept-Encoding` and `Content-Encoding`.
pub const BROTLI_TOKEN: &str = "br";
/// Gzip token of `Accept-Encoding` and `Content-Encoding`.
pub const GZIP_TOKEN: &str = "gzip";
/// File name suffix of a Brotli variant.
pub const BROTLI_SUFFIX: &str = ".br";
/// File name suffix of a gzip variant.
pub const GZIP_SUFFIX: &str = ".gz";
/// Separator of the `Accept-Encoding` list.
pub const ENCODING_LIST_SEPARATOR: char = ',';
/// Separator of the parameters of one `Accept-Encoding` item.
pub const ENCODING_PARAM_SEPARATOR: char = ';';
/// Quality parameter prefix of one `Accept-Encoding` item.
pub const QUALITY_PREFIX: &str = "q=";
/// Startup error when the UI directory is not an existing directory.
pub const UI_DIR_NOT_A_DIRECTORY: &str = "the UI directory is not an existing directory";
/// Startup warning when the UI directory holds no `index.html`.
pub const UI_INDEX_MISSING: &str = "the UI directory holds no index.html: client routes answer 404";

/// Known extensions and their content types; a missing file with one of them is a 404.
pub const MIME_TYPES: &[(&str, &str)] = &[
    ("html", MIME_HTML),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("json", "application/json"),
    ("map", "application/json"),
    ("webmanifest", "application/manifest+json"),
    ("txt", "text/plain; charset=utf-8"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("ico", "image/x-icon"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("wasm", "application/wasm"),
];
