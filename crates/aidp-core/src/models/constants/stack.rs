//! The `stack` input and the values [`crate::stack::detect`] answers for it.

/// Name of the input a template declares to route its language skills.
pub const INPUT: &str = "stack";
/// The value asking [`crate::stack::detect`] to choose the stack.
pub const AUTO: &str = "auto";
/// No marker matched: each stage keeps its multi-stack skill list.
pub const GENERIC: &str = "generic";
/// A Lambda TypeScript service (Serverless Framework).
pub const LAMBDA_TS: &str = "lambda-ts";
/// A C# / .NET service.
pub const CSHARP: &str = "csharp";
/// A Rust workspace.
pub const RUST: &str = "rust";
/// Root manifest of an Angular workspace.
pub const ANGULAR_JSON: &str = "angular.json";
/// AWS SAM template, an alternative Lambda marker beside `package.json`.
pub const SAM_TEMPLATE_YAML: &str = "template.yaml";

/// Root manifest of a Node project.
pub const PACKAGE_JSON: &str = "package.json";
/// Serverless Framework config, YAML form.
pub const SERVERLESS_YML: &str = "serverless.yml";
/// Serverless Framework config, alternate YAML extension.
pub const SERVERLESS_YAML: &str = "serverless.yaml";
/// Serverless Framework config, TypeScript form.
pub const SERVERLESS_TS: &str = "serverless.ts";
/// Serverless Framework configs, any of which marks a Lambda TS repo.
pub const SERVERLESS_FILES: &[&str] = &[SERVERLESS_YML, SERVERLESS_YAML, SERVERLESS_TS];
/// Manifest of a Rust crate or workspace.
pub const CARGO_TOML: &str = "Cargo.toml";
/// Extension of a .NET solution.
pub const SLN_EXT: &str = "sln";
/// Extension of a .NET project.
pub const CSPROJ_EXT: &str = "csproj";
/// Installed Node dependencies.
pub const NODE_MODULES_DIR: &str = "node_modules";
/// Cargo build output.
pub const TARGET_DIR: &str = "target";
/// .NET build output.
pub const BIN_DIR: &str = "bin";
/// .NET intermediate build output.
pub const OBJ_DIR: &str = "obj";
/// Bundled JavaScript output.
pub const DIST_DIR: &str = "dist";
/// Generic build output.
pub const OUT_DIR: &str = "out";
/// Directories never worth descending into when looking for markers.
pub const SKIP_DIRS: &[&str] = &[
    NODE_MODULES_DIR,
    TARGET_DIR,
    BIN_DIR,
    OBJ_DIR,
    DIST_DIR,
    OUT_DIR,
];
