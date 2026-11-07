//! Lookup of the web UI build: index fallback, MIME types, cache policy and traversal refusal.

use super::{accepted_encodings, cache_control, decode_segments, is_api_path, mime_for, StaticUi};
use crate::models::constants::static_ui::{
    ASSETS_SEGMENT, CACHE_IMMUTABLE, CACHE_REVALIDATE, INDEX_FILE, MIME_DEFAULT, MIME_HTML,
};
use crate::models::enums::content_encoding::ContentEncoding;
use crate::models::enums::static_lookup::StaticLookup;
use crate::models::static_file::StaticFile;
use aidp_testkit::TempWorkspace;
use std::fs;
use tiny_http::Method;

const UI_DIR: &str = "dist";
const ASSETS_DIR: &str = "dist/assets";
const INDEX_BODY: &str = "<!doctype html><div id=\"root\"></div>";
const SCRIPT_FILE: &str = "dist/assets/index-3f9a1c.js";
const SCRIPT_PATH: &str = "/assets/index-3f9a1c.js";
const SCRIPT_BODY: &str = "console.log(1)";
const STYLE_FILE: &str = "dist/assets/index-77aa01.css";
const STYLE_PATH: &str = "/assets/index-77aa01.css";
const ICON_FILE: &str = "dist/favicon.svg";
const ICON_PATH: &str = "/favicon.svg";
const UPPER_FILE: &str = "dist/LOGO.PNG";
const UPPER_PATH: &str = "/LOGO.PNG";
const UNKNOWN_FILE: &str = "dist/notes.xyz";
const UNKNOWN_PATH: &str = "/notes.xyz";
const SECRET_FILE: &str = "secret.txt";
const SECRET_BODY: &str = "outside the UI directory";
const SCRIPT_NAME: &str = "index-3f9a1c.js";
const ENCODED_PATH: &str = "/a%20b/./c+d//e/";
const DECODED_SEGMENTS: &[&str] = &["a b", "c+d", "e"];
const UPPER_JS_EXTENSION: &str = "JS";
const HTML_EXTENSION: &str = "html";
const UNKNOWN_EXTENSION: &str = "exe";
const MIME_JS: &str = "text/javascript; charset=utf-8";
const MIME_CSS: &str = "text/css; charset=utf-8";
const MIME_SVG: &str = "image/svg+xml";
const MIME_PNG: &str = "image/png";
const ROOT_PATH: &str = "/";
const NO_ENCODING: &str = "";
const SCRIPT_BROTLI_FILE: &str = "dist/assets/index-3f9a1c.js.br";
const SCRIPT_GZIP_FILE: &str = "dist/assets/index-3f9a1c.js.gz";
const INDEX_GZIP_FILE: &str = "dist/index.html.gz";
const BROTLI_BODY: &str = "brotli bytes";
const GZIP_BODY: &str = "gzip bytes";
const INDEX_GZIP_BODY: &str = "gzip index";
const BROWSER_ENCODINGS: &str = "gzip, deflate, br, zstd";
const GZIP_ONLY: &str = "gzip";
const BROTLI_REFUSED: &str = "br;q=0, gzip;q=0.8";
const BOTH_REFUSED: &str = "br; q=0.0, gzip;q=0";
const UPPER_CASE_ENCODINGS: &str = " GZIP ,BR ";
const WILDCARD: &str = "*";
const DEEP_ROUTES: &[&str] = &["/board", "/board/GH-12", "/inbox/", "/issues/GH-3.5"];
const MISSING_ASSETS: &[&str] = &["/assets/missing.js", "/missing.css", "/assets/gone.woff2"];
const API_PATHS: &[&str] = &["/api", "/api/", "/api/health", "/api/events"];
const NOT_API_PATHS: &[&str] = &["/", "/apiary", "/board/api", "//api/health"];
const TRAVERSALS: &[&str] = &[
    "/../secret.txt",
    "/assets/../../secret.txt",
    "/%2e%2e/secret.txt",
    "/assets/%2E%2E/%2e%2e/secret.txt",
    "/..%2fsecret.txt",
    "/..%5csecret.txt",
    "/%5c..%5csecret.txt",
    "/.../secret.txt",
    "/C:%5cWindows",
    "/index.html::$DATA",
    "/index%00.html",
    "/bad%zzescape",
    "/half%2",
];

/// A UI build in a temp workspace, with one file next to it, outside the UI directory.
fn build() -> (TempWorkspace, StaticUi) {
    let workspace = TempWorkspace::new("static-ui");
    fs::create_dir_all(workspace.child(ASSETS_DIR)).expect("assets dir");
    fs::write(workspace.child(UI_DIR).join(INDEX_FILE), INDEX_BODY).expect("index");
    fs::write(workspace.child(SCRIPT_FILE), SCRIPT_BODY).expect("script");
    fs::write(workspace.child(STYLE_FILE), SCRIPT_BODY).expect("style");
    fs::write(workspace.child(ICON_FILE), SCRIPT_BODY).expect("icon");
    fs::write(workspace.child(UPPER_FILE), SCRIPT_BODY).expect("upper");
    fs::write(workspace.child(UNKNOWN_FILE), SCRIPT_BODY).expect("unknown");
    fs::write(workspace.child(SECRET_FILE), SECRET_BODY).expect("secret");
    fs::write(workspace.child(SCRIPT_BROTLI_FILE), BROTLI_BODY).expect("brotli variant");
    fs::write(workspace.child(SCRIPT_GZIP_FILE), GZIP_BODY).expect("gzip variant");
    fs::write(workspace.child(INDEX_GZIP_FILE), INDEX_GZIP_BODY).expect("index gzip variant");
    let ui = StaticUi::open(Some(&workspace.child(UI_DIR))).expect("open the UI directory");
    (workspace, ui)
}

fn file(ui: &StaticUi, path: &str) -> StaticFile {
    encoded(ui, path, NO_ENCODING)
}

fn encoded(ui: &StaticUi, path: &str, accept_encoding: &str) -> StaticFile {
    match ui.lookup(&Method::Get, path, accept_encoding) {
        StaticLookup::File(file) => file,
        other => panic!("{path}: expected a file, got {other:?}"),
    }
}

fn index() -> StaticFile {
    StaticFile {
        bytes: INDEX_BODY.as_bytes().to_vec(),
        content_type: MIME_HTML,
        cache_control: CACHE_REVALIDATE,
        content_encoding: None,
    }
}

#[test]
fn root_serves_index_without_cache() {
    let (_workspace, ui) = build();
    assert_eq!(file(&ui, ROOT_PATH), index());
}

#[test]
fn client_routes_fall_back_to_index() {
    let (_workspace, ui) = build();
    for route in DEEP_ROUTES {
        assert_eq!(file(&ui, route), index(), "{route}");
    }
}

#[test]
fn hashed_assets_are_immutable_with_their_mime_type() {
    let (_workspace, ui) = build();
    let script = file(&ui, SCRIPT_PATH);
    assert_eq!(script.bytes, SCRIPT_BODY.as_bytes());
    assert_eq!(script.content_type, MIME_JS);
    assert_eq!(script.cache_control, CACHE_IMMUTABLE);
    let style = file(&ui, STYLE_PATH);
    assert_eq!(style.content_type, MIME_CSS);
    assert_eq!(style.cache_control, CACHE_IMMUTABLE);
}

#[test]
fn files_outside_assets_revalidate() {
    let (_workspace, ui) = build();
    let icon = file(&ui, ICON_PATH);
    assert_eq!(icon.content_type, MIME_SVG);
    assert_eq!(icon.cache_control, CACHE_REVALIDATE);
    assert_eq!(file(&ui, UPPER_PATH).content_type, MIME_PNG);
    assert_eq!(file(&ui, UNKNOWN_PATH).content_type, MIME_DEFAULT);
}

#[test]
fn missing_file_with_a_known_extension_is_not_found() {
    let (_workspace, ui) = build();
    for path in MISSING_ASSETS {
        assert_eq!(
            ui.lookup(&Method::Get, path, NO_ENCODING),
            StaticLookup::NotFound,
            "{path}"
        );
    }
}

#[test]
fn traversal_never_leaves_the_ui_directory() {
    let (_workspace, ui) = build();
    for path in TRAVERSALS {
        assert_eq!(
            ui.lookup(&Method::Get, path, NO_ENCODING),
            StaticLookup::NotFound,
            "{path}"
        );
    }
}

#[test]
fn api_paths_and_writes_are_left_to_the_api() {
    let (_workspace, ui) = build();
    for path in API_PATHS {
        assert_eq!(
            ui.lookup(&Method::Get, path, NO_ENCODING),
            StaticLookup::NotUi,
            "{path}"
        );
    }
    for method in [Method::Post, Method::Put, Method::Patch, Method::Delete] {
        assert_eq!(
            ui.lookup(&method, ROOT_PATH, NO_ENCODING),
            StaticLookup::NotUi
        );
    }
}

#[test]
fn head_is_served_like_get() {
    let (_workspace, ui) = build();
    assert_eq!(
        ui.lookup(&Method::Head, ROOT_PATH, NO_ENCODING),
        StaticLookup::File(index())
    );
}

#[test]
fn no_ui_directory_leaves_everything_to_the_api() {
    let ui = StaticUi::default();
    assert!(ui.root().is_none());
    assert_eq!(
        ui.lookup(&Method::Get, ROOT_PATH, NO_ENCODING),
        StaticLookup::NotUi
    );
    assert_eq!(
        ui.lookup(&Method::Get, SCRIPT_PATH, NO_ENCODING),
        StaticLookup::NotUi
    );
}

#[test]
fn a_directory_without_index_answers_not_found_for_client_routes() {
    let workspace = TempWorkspace::new("static-ui-empty");
    fs::create_dir_all(workspace.child(UI_DIR)).expect("ui dir");
    let ui = StaticUi::open(Some(&workspace.child(UI_DIR))).expect("open");
    assert_eq!(
        ui.lookup(&Method::Get, ROOT_PATH, NO_ENCODING),
        StaticLookup::NotFound
    );
    assert_eq!(
        ui.lookup(&Method::Get, DEEP_ROUTES[0], NO_ENCODING),
        StaticLookup::NotFound
    );
}

#[test]
fn open_refuses_a_missing_directory_and_a_file() {
    let workspace = TempWorkspace::new("static-ui-refused");
    fs::write(workspace.child(SECRET_FILE), SECRET_BODY).expect("file");
    assert!(StaticUi::open(Some(&workspace.child(UI_DIR))).is_err());
    assert!(StaticUi::open(Some(&workspace.child(SECRET_FILE))).is_err());
    assert!(StaticUi::open(None).expect("no UI").root().is_none());
}

#[test]
fn api_prefix_is_matched_on_whole_segments() {
    for path in API_PATHS {
        assert!(is_api_path(path), "{path}");
    }
    for path in NOT_API_PATHS {
        assert!(!is_api_path(path), "{path}");
    }
}

#[test]
fn segments_are_decoded_and_cleaned() {
    let expected: Vec<String> = DECODED_SEGMENTS.iter().map(ToString::to_string).collect();
    assert_eq!(decode_segments(ENCODED_PATH), Some(expected));
    assert_eq!(decode_segments(ROOT_PATH), Some(Vec::new()));
    for path in TRAVERSALS {
        assert_eq!(decode_segments(path), None, "{path}");
    }
}

#[test]
fn mime_and_cache_tables() {
    assert_eq!(mime_for(UPPER_JS_EXTENSION), Some(MIME_JS));
    assert_eq!(mime_for(HTML_EXTENSION), Some(MIME_HTML));
    assert_eq!(mime_for(UNKNOWN_EXTENSION), None);
    let asset = vec![ASSETS_SEGMENT.to_string(), SCRIPT_NAME.to_string()];
    let top = vec![ASSETS_SEGMENT.to_string()];
    assert_eq!(cache_control(&asset), CACHE_IMMUTABLE);
    assert_eq!(cache_control(&top), CACHE_REVALIDATE);
    assert_eq!(cache_control(&[INDEX_FILE.to_string()]), CACHE_REVALIDATE);
}

#[test]
fn precompressed_variant_is_sent_when_accepted() {
    let (_workspace, ui) = build();
    let brotli = encoded(&ui, SCRIPT_PATH, BROWSER_ENCODINGS);
    assert_eq!(brotli.bytes, BROTLI_BODY.as_bytes());
    assert_eq!(brotli.content_encoding, Some(ContentEncoding::Brotli));
    assert_eq!(brotli.content_type, MIME_JS);
    assert_eq!(brotli.cache_control, CACHE_IMMUTABLE);
    let gzip = encoded(&ui, SCRIPT_PATH, BROTLI_REFUSED);
    assert_eq!(gzip.bytes, GZIP_BODY.as_bytes());
    assert_eq!(gzip.content_encoding, Some(ContentEncoding::Gzip));
    let plain = encoded(&ui, SCRIPT_PATH, BOTH_REFUSED);
    assert_eq!(plain.bytes, SCRIPT_BODY.as_bytes());
    assert_eq!(plain.content_encoding, None);
}

#[test]
fn fallback_index_and_files_without_variant() {
    let (_workspace, ui) = build();
    let index_gzip = encoded(&ui, DEEP_ROUTES[1], BROWSER_ENCODINGS);
    assert_eq!(index_gzip.bytes, INDEX_GZIP_BODY.as_bytes());
    assert_eq!(index_gzip.content_type, MIME_HTML);
    assert_eq!(index_gzip.content_encoding, Some(ContentEncoding::Gzip));
    let style = encoded(&ui, STYLE_PATH, BROWSER_ENCODINGS);
    assert_eq!(style.bytes, SCRIPT_BODY.as_bytes());
    assert_eq!(style.content_encoding, None);
}

#[test]
fn accept_encoding_is_parsed_in_server_preference_order() {
    let both = vec![ContentEncoding::Brotli, ContentEncoding::Gzip];
    assert_eq!(accepted_encodings(BROWSER_ENCODINGS), both);
    assert_eq!(accepted_encodings(UPPER_CASE_ENCODINGS), both);
    assert_eq!(accepted_encodings(GZIP_ONLY), vec![ContentEncoding::Gzip]);
    assert_eq!(
        accepted_encodings(BROTLI_REFUSED),
        vec![ContentEncoding::Gzip]
    );
    assert!(accepted_encodings(BOTH_REFUSED).is_empty());
    assert!(accepted_encodings(WILDCARD).is_empty());
    assert!(accepted_encodings(NO_ENCODING).is_empty());
}
