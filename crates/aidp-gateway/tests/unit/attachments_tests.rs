use super::*;
use crate::models::enums::{ApiStatus, ImageMediaType};
use serde_json::json;

const PNG_WIRE_TYPE: &str = "image/png";
const IMAGE_WIRE_TYPE: &str = "image";
const BASE64_WIRE_TYPE: &str = "base64";
const UNKNOWN_WIRE_TYPE: &str = "application/pdf";
const SMALL_IMAGE_BYTES: usize = 200 * 1024;

fn attachment(media_type: ImageMediaType, bytes: usize) -> Attachment {
    Attachment {
        media_type,
        data_base64: STANDARD.encode(vec![7u8; bytes]),
    }
}

#[test]
fn accepts_small_png_and_serializes_image_block() {
    let blocks = validate_and_build(&[attachment(ImageMediaType::Png, SMALL_IMAGE_BYTES)]).unwrap();
    let value = serde_json::to_value(&blocks[0]).unwrap();
    assert_eq!(value["type"], IMAGE_WIRE_TYPE);
    assert_eq!(value["source"]["type"], BASE64_WIRE_TYPE);
    assert_eq!(value["source"]["media_type"], PNG_WIRE_TYPE);
    assert!(value["source"]["data"].is_string());
}

#[test]
fn accepts_exactly_the_size_limit() {
    assert!(validate_and_build(&[attachment(ImageMediaType::Png, MAX_DECODED_BYTES)]).is_ok());
}

#[test]
fn rejects_one_byte_over_the_limit() {
    let error =
        validate_and_build(&[attachment(ImageMediaType::Png, MAX_DECODED_BYTES + 1)]).unwrap_err();
    assert_eq!(error.status, ApiStatus::BadRequest);
    assert!(error.message.contains("attachments[0]"));
}

#[test]
fn rejects_unknown_media_type_naming_the_index() {
    let values = [
        json!({"mediaType": PNG_WIRE_TYPE, "dataBase64": "AAAA"}),
        json!({"mediaType": UNKNOWN_WIRE_TYPE, "dataBase64": "AAAA"}),
    ];
    let error = parse_attachments(&values).unwrap_err();
    assert_eq!(error.status, ApiStatus::BadRequest);
    assert_eq!(error.message, "attachments[1]: unsupported mediaType");
}

#[test]
fn rejects_invalid_base64() {
    let invalid = Attachment {
        media_type: ImageMediaType::Png,
        data_base64: "!!not base64!!".to_string(),
    };
    let error = validate_and_build(&[invalid]).unwrap_err();
    assert_eq!(error.status, ApiStatus::BadRequest);
}
