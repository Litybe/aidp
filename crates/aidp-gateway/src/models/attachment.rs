use serde::Deserialize;

use super::enums::ImageMediaType;

/// One image attached to a reply, as sent by the client.
#[derive(Debug, Clone, Deserialize)]
pub struct Attachment {
    #[serde(rename = "mediaType")]
    pub media_type: ImageMediaType,
    #[serde(rename = "dataBase64")]
    pub data_base64: String,
}
