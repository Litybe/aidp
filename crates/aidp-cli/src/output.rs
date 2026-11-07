use crate::commands::{CommandOutput, Payload};
use crate::models::constants::embedded_command::{ERROR_PREFIX, LINE_ENDING};
use aidp_core::Format;
use std::io::Write;

/// Renders a JSON value in the requested format.
/// Delegates to the shared renderer in `aidp-core`.
pub fn render_value(format: Format, value: &serde_json::Value) -> anyhow::Result<String> {
    Ok(aidp_core::render_value(format, value)?)
}

/// Writes a command's diagnostics to stderr, then its payload to stdout.
pub fn emit(output: &CommandOutput) -> anyhow::Result<()> {
    write_streams(
        output,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    )
}

/// Captures the same payload, diagnostics and failure formatting as process emission.
pub fn capture(output: &CommandOutput) -> anyhow::Result<(String, String)> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    write_streams(output, &mut stdout, &mut stderr)?;
    Ok((String::from_utf8(stdout)?, String::from_utf8(stderr)?))
}

fn write_streams(
    output: &CommandOutput,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> anyhow::Result<()> {
    for line in &output.diagnostics {
        stderr.write_all(format!("{line}{LINE_ENDING}").as_bytes())?;
    }
    match &output.payload {
        Payload::Empty => {}
        Payload::Text(text) => stdout.write_all(format!("{text}{LINE_ENDING}").as_bytes())?,
        Payload::Json { value, format } => {
            stdout.write_all(
                format!(
                    "{}{LINE_ENDING}",
                    render_value(format.unwrap_or_default(), value)?
                )
                .as_bytes(),
            )?;
        }
    }
    if let Some(failure) = &output.failure {
        stderr.write_all(format!("{ERROR_PREFIX}{failure:?}{LINE_ENDING}").as_bytes())?;
    }
    Ok(())
}

/// Serializes with `": "` and `", "` between object members.
/// Array elements stay compact. The driver scripts read this exact layout.
pub fn to_spaced_json<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    let mut buffer = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, SpacedObjects);
    value.serialize(&mut serializer)?;
    Ok(String::from_utf8(buffer)?)
}

struct SpacedObjects;

impl serde_json::ser::Formatter for SpacedObjects {
    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        writer.write_all(b": ")
    }
}

#[cfg(test)]
#[path = "../tests/unit/output_tests.rs"]
mod tests;
