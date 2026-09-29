//! Builds a Run `input` from the typed text and Attachments. There is no upload endpoint on the
//! API server, so text files ride inline and images ride as `image_url` data-URL parts.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Where an Attachment's bytes come from: a path dropped on the window (checked against the
/// drop list), or bytes the webview already holds (the paperclip picker, a pasted image).
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Source {
    Dropped { path: PathBuf },
    Inline { name: String, data_url: String },
}

fn read(file: &Source) -> Result<(String, Vec<u8>), String> {
    let too_big = |name: &str| format!("{name} is larger than 2 MB");
    match file {
        Source::Dropped { path } => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let size = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?.len();
            if size > MAX_BYTES {
                return Err(too_big(&name));
            }
            Ok((name.clone(), std::fs::read(path).map_err(|e| format!("{name}: {e}"))?))
        }
        Source::Inline { name, data_url } => {
            let data = match data_url.split_once(',') {
                Some((head, data)) if head.starts_with("data:") && head.ends_with(";base64") => data,
                _ if data_url == "data:" => "",
                _ => return Err(format!("{name}: not a base64 data URL")),
            };
            let bytes = STANDARD.decode(data).map_err(|e| format!("{name}: {e}"))?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(too_big(name));
            }
            Ok((name.clone(), bytes))
        }
    }
}

pub fn build_input(text: &str, files: &[Source]) -> Result<Value, String> {
    let mut message = text.to_owned();
    let mut images = Vec::new();
    for file in files {
        let (name, bytes) = read(file)?;
        if let Some(mime) = image_mime(Path::new(&name)) {
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

    fn temp(name: &str, bytes: &[u8]) -> Source {
        let path = std::env::temp_dir().join(format!("hd-attach-test-{name}"));
        std::fs::write(&path, bytes).unwrap();
        Source::Dropped { path }
    }

    fn inline(name: &str, data_url: &str) -> Source {
        Source::Inline { name: name.into(), data_url: data_url.into() }
    }

    #[test]
    fn text_is_inlined_with_a_fence_longer_than_its_content() {
        let input = build_input("see this", &[temp("notes.md", b"a ``` b")]).unwrap();
        assert_eq!(input, json!("see this\n\nAttached file `hd-attach-test-notes.md`:\n````\na ``` b\n````"));
    }

    #[test]
    fn images_become_data_url_parts() {
        let input = build_input("look", &[temp("pic.PNG", &[0x89, b'P', b'N', b'G'])]).unwrap();
        assert_eq!(input[0]["content"][0], json!({ "type": "text", "text": "look" }));
        assert_eq!(input[0]["content"][1]["image_url"]["url"], json!("data:image/png;base64,iVBORw=="));
    }

    #[test]
    fn binary_and_oversized_files_are_refused() {
        assert!(build_input("", &[temp("blob.bin", &[0, 159, 146, 150])]).unwrap_err().contains("only text and image"));
        let big = temp("big.txt", &vec![b'a'; MAX_BYTES as usize + 1]);
        assert!(build_input("", &[big]).unwrap_err().contains("larger than 2 MB"));
    }

    #[test]
    fn inline_files_follow_the_same_rules() {
        let pasted = inline("Pasted image 1.png", "data:image/png;base64,iVBORw==");
        let picked = inline("todo.txt", &format!("data:text/plain;base64,{}", STANDARD.encode("buy milk")));
        let input = build_input("", &[picked, pasted]).unwrap();
        assert_eq!(input[0]["content"][0]["text"], json!("Attached file `todo.txt`:\n```\nbuy milk\n```"));
        assert_eq!(input[0]["content"][1]["image_url"]["url"], json!("data:image/png;base64,iVBORw=="));
        // FileReader writes an empty file as a bare "data:".
        assert_eq!(build_input("", &[inline("empty.txt", "data:")]).unwrap(), json!("Attached file `empty.txt`:\n```\n\n```"));
        let big = format!("data:text/plain;base64,{}", STANDARD.encode(vec![b'a'; MAX_BYTES as usize + 1]));
        assert!(build_input("", &[inline("big.txt", &big)]).unwrap_err().contains("larger than 2 MB"));
        assert!(build_input("", &[inline("x.txt", "not a data url")]).unwrap_err().contains("not a base64 data URL"));
    }
}
