#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Low,
    Original,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub detail: ImageDetail,
}
