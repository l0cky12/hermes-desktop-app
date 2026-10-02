//! Builds a Run `input` from the typed text and Attachments. There is no upload endpoint on the
//! API server, so text files ride inline and images ride as `image_url` data-URL parts.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

/// What all of a Turn's Attachments may weigh together. The API server refuses request bodies over
/// 10,000,000 bytes (`MAX_REQUEST_BYTES` in gateway/platforms/api_server.py) and images ride base64,
/// 4/3 their size: 7,000,000 bytes of images is 9,333,334 in the body, leaving ~0.67 MB for the typed
/// text and the JSON around it. SSH (ACP) has no cap; it gets the same budget so both modes behave alike.
pub const MAX_TOTAL_BYTES: u64 = 7_000_000;
/// A dropped image bigger than this is refused outright rather than read into memory to shrink.
pub const MAX_SHRINKABLE_BYTES: u64 = 50_000_000;

/// Where an Attachment's bytes come from: a path dropped on the window (checked against the
/// drop list), or bytes the webview already holds (the paperclip picker, a pasted image).
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Source {
    Dropped { path: PathBuf },
    Inline { name: String, data_url: String },
}

/// Reads one Attachment, refusing it if it is bigger than the `left` of the Turn's budget.
fn read(file: &Source, left: u64) -> Result<(String, Vec<u8>), String> {
    let too_big = |name: &str| format!("{name} takes the Attachments over the 7 MB a Turn can carry");
    match file {
        Source::Dropped { path } => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ok((name.clone(), read_file(path, left, &too_big(&name))?))
        }
        Source::Inline { name, data_url } => {
            let data = match data_url.split_once(',') {
                Some((head, data)) if head.starts_with("data:") && head.ends_with(";base64") => data,
                _ if data_url == "data:" => "",
                _ => return Err(format!("{name}: not a base64 data URL")),
            };
            if data.len() as u64 > left.div_ceil(3) * 4 {
                return Err(too_big(name));
            }
            let bytes = STANDARD.decode(data).map_err(|e| format!("{name}: {e}"))?;
            if bytes.len() as u64 > left {
                return Err(too_big(name));
            }
            Ok((name.clone(), bytes))
        }
    }
}

pub fn build_input(text: &str, files: &[Source]) -> Result<Value, String> {
    let mut message = text.to_owned();
    let mut images = Vec::new();
    let mut left = MAX_TOTAL_BYTES;
    for file in files {
        let (name, bytes) = read(file, left)?;
        left = left.saturating_sub(bytes.len() as u64);
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
            return Err(format!("{name}: only text and images can be attached"));
        }
    }
    if images.is_empty() {
        return Ok(Value::String(message));
    }
    let mut content = vec![json!({ "type": "text", "text": message })];
    content.extend(images);
    Ok(json!([{ "role": "user", "content": content }]))
}

/// A dropped image as a data URL, so the webview can shrink it to fit the budget; `None` if it
/// isn't an image.
pub fn dropped_image(path: &Path) -> Result<Option<String>, String> {
    let Some(mime) = image_mime(path) else { return Ok(None) };
    let bytes = read_file(path, MAX_SHRINKABLE_BYTES, &format!("{}: too large to shrink (over 50 MB)", path.display()))?;
    Ok(Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes))))
}

/// Metadata is only an early rejection: files can grow or report zero (e.g. procfs).
fn read_file(path: &Path, limit: u64, too_big: &str) -> Result<Vec<u8>, String> {
    let io_error = |e| format!("{}: {e}", path.display());
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Check the opened handle, without blocking if the path was replaced by a FIFO.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(format!("{}: only regular files can be attached", path.display()));
    }
    if metadata.len() > limit {
        return Err(too_big.to_owned());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(too_big.to_owned());
    }
    Ok(bytes)
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
    fn binary_files_are_refused() {
        assert!(build_input("", &[temp("blob.bin", &[0, 159, 146, 150])]).unwrap_err().ends_with("only text and images can be attached"));
    }

    #[test]
    fn the_budget_is_for_all_attachments_together() {
        let half = MAX_TOTAL_BYTES as usize / 2;
        let halves = || [temp("a.txt", &vec![b'a'; half]), inline("b.txt", &format!("data:text/plain;base64,{}", STANDARD.encode(vec![b'b'; half])))];
        assert!(build_input("", &halves()).is_ok());
        let [a, b] = halves();
        assert_eq!(build_input("", &[a, b, temp("c.txt", b"c")]).unwrap_err(), "hd-attach-test-c.txt takes the Attachments over the 7 MB a Turn can carry");
        // A single file over 2 MB (the old per-file cap) is fine.
        assert!(build_input("", &[temp("big.png", &vec![0; 3_000_000])]).is_ok());
    }

    #[test]
    fn dropped_images_are_read_for_the_webview_and_other_files_are_not() {
        let Source::Dropped { path } = temp("shot.png", &[0x89, b'P', b'N', b'G']) else { unreachable!() };
        assert_eq!(dropped_image(&path).unwrap().as_deref(), Some("data:image/png;base64,iVBORw=="));
        let Source::Dropped { path } = temp("notes2.md", b"x") else { unreachable!() };
        assert_eq!(dropped_image(&path).unwrap(), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dropped_reads_reject_devices_and_enforce_actual_bytes() {
        assert!(build_input("", &[Source::Dropped { path: "/dev/null".into() }]).unwrap_err().contains("regular files"));
        // procfs is a regular file with metadata length zero but a nonempty body.
        assert_eq!(std::fs::metadata("/proc/self/cmdline").unwrap().len(), 0);
        assert_eq!(read_file(Path::new("/proc/self/cmdline"), 1, "over budget").unwrap_err(), "over budget");
        let fifo = std::env::temp_dir().join(format!("hd-attach-fifo-{}", std::process::id()));
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let (tx, rx) = std::sync::mpsc::channel();
        let path = fifo.clone();
        std::thread::spawn(move || tx.send(read_file(&path, 1, "over budget")));
        let result = rx.recv_timeout(std::time::Duration::from_secs(1));
        if result.is_err() {
            // Unblock a regressed reader before reporting failure.
            let _ = std::fs::OpenOptions::new().read(true).write(true).open(&fifo);
        }
        std::fs::remove_file(fifo).unwrap();
        assert!(result.expect("opening a FIFO must not block").unwrap_err().contains("regular files"));
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
        let big = format!("data:text/plain;base64,{}", STANDARD.encode(vec![b'a'; MAX_TOTAL_BYTES as usize + 1]));
        assert!(build_input("", &[inline("big.txt", &big)]).unwrap_err().ends_with("over the 7 MB a Turn can carry"));
        assert!(build_input("", &[inline("x.txt", "not a data url")]).unwrap_err().contains("not a base64 data URL"));
    }
}
