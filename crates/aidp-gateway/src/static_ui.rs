//! The web UI build served on `/` by `aidp gateway serve --ui-dir`.
//! `/api` stays first; a client route without a known extension falls back to `index.html`.

use crate::models::constants::static_ui::{
    API_PREFIX, ASSETS_SEGMENT, CACHE_IMMUTABLE, CACHE_REVALIDATE, CURRENT_SEGMENT,
    ENCODING_LIST_SEPARATOR, ENCODING_PARAM_SEPARATOR, EXTENSION_SEPARATOR,
    FORBIDDEN_SEGMENT_CHARS, INDEX_FILE, MIME_DEFAULT, MIME_TYPES, PARENT_SEGMENT, QUALITY_PREFIX,
    SEGMENT_SEPARATOR, UI_DIR_ENV, UI_DIR_NOT_A_DIRECTORY, UI_INDEX_MISSING,
};
use crate::models::enums::content_encoding::ContentEncoding;
use crate::models::enums::static_lookup::StaticLookup;
use crate::models::static_file::StaticFile;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tiny_http::Method;
use tracing::{info, warn};

const PERCENT: u8 = b'%';
const HEX_RADIX: u32 = 16;

/// The UI directory, canonical; empty when the daemon serves no UI.
#[derive(Debug, Default, Clone)]
pub struct StaticUi {
    root: Option<PathBuf>,
}

impl StaticUi {
    /// Opens `dir`, refusing a path that is not an existing directory; `None` serves no UI.
    pub fn open(dir: Option<&Path>) -> anyhow::Result<Self> {
        let Some(dir) = dir else {
            return Ok(Self::default());
        };
        let root = dir
            .canonicalize()
            .ok()
            .filter(|path| path.is_dir())
            .ok_or_else(|| anyhow::anyhow!("{UI_DIR_NOT_A_DIRECTORY}: {}", dir.display()))?;
        if root.join(INDEX_FILE).is_file() {
            info!(dir = %dir.display(), "serving the web UI on /");
        } else {
            warn!(dir = %dir.display(), "{UI_INDEX_MISSING}");
        }
        Ok(Self { root: Some(root) })
    }

    /// The canonical UI directory, when one is served.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Answers a request path, query excluded, from the UI directory.
    /// `accept_encoding` is the request header: a `.br` or `.gz` variant is preferred when accepted.
    pub fn lookup(&self, method: &Method, path: &str, accept_encoding: &str) -> StaticLookup {
        let Some(root) = self.root.as_deref() else {
            return StaticLookup::NotUi;
        };
        if !matches!(method, Method::Get | Method::Head) || is_api_path(path) {
            return StaticLookup::NotUi;
        }
        let Some(segments) = decode_segments(path) else {
            return StaticLookup::NotFound;
        };
        let accepted = accepted_encodings(accept_encoding);
        if let Some(file) = read_file(root, &segments, &accepted) {
            return StaticLookup::File(file);
        }
        if segments
            .last()
            .is_some_and(|segment| has_known_extension(segment))
        {
            return StaticLookup::NotFound;
        }
        read_file(root, &[INDEX_FILE.to_string()], &accepted)
            .map_or(StaticLookup::NotFound, StaticLookup::File)
    }
}

/// The `--ui-dir` value, else a non-empty `AIDP_UI_DIR`.
pub fn ui_dir_or_env(flag: Option<PathBuf>) -> Option<PathBuf> {
    flag.or_else(|| {
        std::env::var_os(UI_DIR_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

/// The precompressed encodings an `Accept-Encoding` header allows, in server preference order.
/// An item with `q=0` is refused; `*` is ignored, so only named encodings are sent.
pub fn accepted_encodings(header: &str) -> Vec<ContentEncoding> {
    let allowed: Vec<String> = header
        .split(ENCODING_LIST_SEPARATOR)
        .filter_map(|item| {
            let mut parts = item.split(ENCODING_PARAM_SEPARATOR).map(str::trim);
            let name = parts.next()?.to_ascii_lowercase();
            let refused = parts
                .filter_map(|param| param.strip_prefix(QUALITY_PREFIX))
                .any(|quality| quality.parse::<f32>().is_ok_and(|value| value <= 0.0));
            (!name.is_empty() && !refused).then_some(name)
        })
        .collect();
    ContentEncoding::PREFERENCE
        .into_iter()
        .filter(|encoding| allowed.iter().any(|name| name == encoding.token()))
        .collect()
}

/// True for `/api` and every path under it: those always reach the API routes.
pub fn is_api_path(path: &str) -> bool {
    path.strip_prefix(API_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(SEGMENT_SEPARATOR))
}

/// The percent-decoded segments of `path`, without empty and `.` ones.
/// `None` for `..`, a segment of dots, an encoded separator, a drive mark or invalid UTF-8.
pub fn decode_segments(path: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    for raw in path.split(SEGMENT_SEPARATOR) {
        let segment = percent_decode(raw)?;
        if segment.is_empty() || segment == CURRENT_SEGMENT {
            continue;
        }
        let only_dots = segment.trim_matches(EXTENSION_SEPARATOR).is_empty();
        if segment == PARENT_SEGMENT
            || only_dots
            || segment.contains(SEGMENT_SEPARATOR)
            || segment.contains(FORBIDDEN_SEGMENT_CHARS)
        {
            return None;
        }
        segments.push(segment);
    }
    Some(segments)
}

/// The content type of an extension, case-insensitive; `None` when it is not a known one.
pub fn mime_for(extension: &str) -> Option<&'static str> {
    MIME_TYPES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        .map(|(_, mime)| *mime)
}

/// The cache policy of a file: a year for the hashed `assets/`, revalidation otherwise.
pub fn cache_control(segments: &[String]) -> &'static str {
    match segments {
        [first, _, ..] if first == ASSETS_SEGMENT => CACHE_IMMUTABLE,
        _ => CACHE_REVALIDATE,
    }
}

fn has_known_extension(segment: &str) -> bool {
    segment
        .rsplit_once(EXTENSION_SEPARATOR)
        .is_some_and(|(_, extension)| mime_for(extension).is_some())
}

/// Reads the regular file the segments name, only if it resolves inside `root`.
/// Its first accepted precompressed variant replaces it when the build wrote one.
fn read_file(root: &Path, segments: &[String], accepted: &[ContentEncoding]) -> Option<StaticFile> {
    let candidate = segments
        .iter()
        .fold(root.to_path_buf(), |path, segment| path.join(segment));
    let resolved = inside(root, &candidate)?;
    let content_type = resolved
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(mime_for)
        .unwrap_or(MIME_DEFAULT);
    let (bytes, content_encoding) = accepted
        .iter()
        .find_map(|encoding| {
            let mut variant = OsString::from(resolved.as_os_str());
            variant.push(encoding.suffix());
            let variant = inside(root, Path::new(&variant))?;
            Some((std::fs::read(variant).ok()?, Some(*encoding)))
        })
        .or_else(|| Some((std::fs::read(&resolved).ok()?, None)))?;
    Some(StaticFile {
        bytes,
        content_type,
        cache_control: cache_control(segments),
        content_encoding,
    })
}

/// The canonical path of `candidate` when it is a regular file inside `root`.
fn inside(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = candidate.canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

/// Percent-decoding of a path segment; `+` stays literal. `None` on a bad escape or bad UTF-8.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == PERCENT {
            let high = char::from(*bytes.get(index + 1)?).to_digit(HEX_RADIX)?;
            let low = char::from(*bytes.get(index + 2)?).to_digit(HEX_RADIX)?;
            decoded.push(u8::try_from(high * HEX_RADIX + low).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
#[path = "../tests/unit/static_ui_tests.rs"]
mod tests;
