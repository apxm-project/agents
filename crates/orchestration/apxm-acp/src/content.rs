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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_block_serialization() {
        let block = ContentBlock::text("hello");
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");
    }

    #[test]
    fn image_block_serialization() {
        let block = ContentBlock::Image {
            mime_type: "image/png".into(),
            data: "base64data".into(),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["mimeType"], "image/png");
    }
}
