use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::Value;

use crate::models::enums::{ApiStatus, ImageBlockType, ImageSourceType};
use crate::models::{ApiError, Attachment, ImageBlock, ImageSource};

const MAX_DECODED_BYTES: usize = 5 * 1024 * 1024;
const UNSUPPORTED_MEDIA_TYPE_DETAIL: &str = "unsupported mediaType";

/// Validate each attachment and build the image blocks to send to claude.
/// A failure is a 400 naming the offending index.
pub fn validate_and_build(attachments: &[Attachment]) -> Result<Vec<ImageBlock>, ApiError> {
    attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| build_block(index, attachment))
        .collect()
}

fn build_block(index: usize, attachment: &Attachment) -> Result<ImageBlock, ApiError> {
    let decoded = STANDARD
        .decode(attachment.data_base64.as_bytes())
        .map_err(|_| rejection(index, "dataBase64 is not valid base64"))?;
    if decoded.len() > MAX_DECODED_BYTES {
        return Err(rejection(
            index,
            &format!("image exceeds {MAX_DECODED_BYTES} bytes"),
        ));
    }
    Ok(ImageBlock {
        block_type: ImageBlockType::Image,
        source: ImageSource {
            source_type: ImageSourceType::Base64,
            media_type: attachment.media_type,
            data: attachment.data_base64.clone(),
        },
    })
}

/// Deserialize a JSON array of attachments; an unknown mediaType is a 400 naming its index.
pub fn parse_attachments(values: &[Value]) -> Result<Vec<Attachment>, ApiError> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            serde_json::from_value(value.clone())
                .map_err(|_| rejection(index, UNSUPPORTED_MEDIA_TYPE_DETAIL))
        })
        .collect()
}

fn rejection(index: usize, detail: &str) -> ApiError {
    ApiError {
        status: ApiStatus::BadRequest,
        message: format!("attachments[{index}]: {detail}"),
    }
}

#[cfg(test)]
#[path = "../tests/unit/attachments_tests.rs"]
mod attachments_tests;
