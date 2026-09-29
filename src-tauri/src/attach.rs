//! Builds a Run `input` from the typed text and dropped files. There is no upload endpoint on
//! the API server, so text files ride inline and images ride as `image_url` data-URL parts.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn build_input(text: &str, files: &[PathBuf]) -> Result<Value, String> {
    let mut message = text.to_owned();
    let mut images = Vec::new();
    for path in files {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let size = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?.len();
        if size > MAX_BYTES {
            return Err(format!("{name} is larger than 2 MB"));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
        if let Some(mime) = image_mime(path) {
            let url = format!("data:{mime};base64,{}", STANDARD.encode(&bytes));
            images.push(json!({ "type": "image_url", "image_url": { "url": url } }));
        } else if let Some(content) = String::from_utf8(bytes).ok().filter(|s| !s.contains('\0')) {
            let mut fence = "```".to_owned();
            while content.contains(&fence) {
                fence.push('`');
            }
            if !message.is_empty() {
                message.push_str("\n\n");
            }
            message.push_str(&format!("Attached file `{name}`:\n{fence}\n{content}\n{fence}"));
        } else {
            return Err(format!("{name}: only text and image files can be attached"));
        }
    }
    if images.is_empty() {
        return Ok(Value::String(message));
    }
    let mut content = vec![json!({ "type": "text", "text": message })];
    content.extend(images);
    Ok(json!([{ "role": "user", "content": content }]))
}

fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("hd-attach-test-{name}"));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn text_is_inlined_with_a_fence_longer_than_its_content() {
        let f = temp("notes.md", b"a ``` b");
        let input = build_input("see this", &[f]).unwrap();
        assert_eq!(input, json!("see this\n\nAttached file `hd-attach-test-notes.md`:\n````\na ``` b\n````"));
    }

    #[test]
    fn images_become_data_url_parts() {
        let f = temp("pic.PNG", &[0x89, b'P', b'N', b'G']);
        let input = build_input("look", &[f]).unwrap();
        assert_eq!(input[0]["content"][0], json!({ "type": "text", "text": "look" }));
        assert_eq!(input[0]["content"][1]["image_url"]["url"], json!("data:image/png;base64,iVBORw=="));
    }

    #[test]
    fn binary_and_oversized_files_are_refused() {
        assert!(build_input("", &[temp("blob.bin", &[0, 159, 146, 150])]).unwrap_err().contains("only text and image"));
        let big = temp("big.txt", &vec![b'a'; MAX_BYTES as usize + 1]);
        assert!(build_input("", &[big]).unwrap_err().contains("larger than 2 MB"));
    }
}
