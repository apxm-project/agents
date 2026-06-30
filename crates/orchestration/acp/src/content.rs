use serde::{Deserialize, Serialize};

/// ACP content block types for prompt input.
///
/// v1 only sends `Text` blocks; other variants are defined for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: String,
    },
    ResourceLink {
        uri: String,
        title: Option<String>,
        name: Option<String>,
    },
    Resource {
        resource: ResourceContent,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    pub text: Option<String>,
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text { text: s.into() }
    }
}
