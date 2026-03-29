use quick_xml::events::{BytesText, Event};
use quick_xml::{Reader, Writer};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// A content file found in the EPUB's OPF manifest.
pub struct ContentFile {
    pub relative_path: String,
}

/// Delimiter used to separate text nodes for batch API conversion.
const TEXT_DELIMITER: &str = "\x00\x01\x00";

/// Extract an EPUB ZIP to a temp directory and return content file paths.
pub fn extract_epub(epub_path: &Path) -> Result<(TempDir, Vec<ContentFile>), String> {
    let file = fs::File::open(epub_path).map_err(|e| format!("無法開啟 EPUB：{e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("無效的 EPUB 檔案：{e}"))?;

    let temp_dir = TempDir::new().map_err(|e| format!("無法建立暫存目錄：{e}"))?;

    // Extract all files
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("無法讀取 EPUB 內容：{e}"))?;
        let name = entry.name().to_string();

        if entry.is_dir() {
            fs::create_dir_all(temp_dir.path().join(&name))
                .map_err(|e| format!("無法建立目錄：{e}"))?;
            continue;
        }

        let out_path = temp_dir.path().join(&name);

        // Prevent ZIP path traversal
        if !out_path.starts_with(temp_dir.path()) {
            return Err(format!("EPUB 包含不安全的路徑：{name}"));
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("無法建立目錄：{e}"))?;
        }

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("無法讀取檔案：{e}"))?;
        fs::write(&out_path, &buf).map_err(|e| format!("無法寫入檔案：{e}"))?;
    }

    // Find content files by scanning for .xhtml/.html files
    let content_files = find_content_files(temp_dir.path())?;

    Ok((temp_dir, content_files))
}

/// Recursively find all .xhtml and .html files in the extracted EPUB.
fn find_content_files(dir: &Path) -> Result<Vec<ContentFile>, String> {
    let mut files = Vec::new();
    find_content_files_recursive(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn find_content_files_recursive(
    root: &Path,
    dir: &Path,
    files: &mut Vec<ContentFile>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("無法讀取目錄：{e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("無法讀取目錄項目：{e}"))?;
        let path = entry.path();
        if path.is_dir() {
            find_content_files_recursive(root, &path, files)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_lowercase();
            if ext_lower == "xhtml" || ext_lower == "html" || ext_lower == "htm" {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| format!("路徑錯誤：{e}"))?
                    .to_string_lossy()
                    .into_owned();
                files.push(ContentFile {
                    relative_path: relative,
                });
            }
        }
    }
    Ok(())
}

/// Extract all text content from XHTML, joining with a delimiter.
/// Returns the concatenated text and the count of text segments.
pub fn extract_text(xhtml: &str) -> Result<(String, usize), String> {
    let mut reader = Reader::from_str(xhtml);
    let mut texts = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Text(e)) => {
                let text = e
                    .unescape()
                    .map_err(|err| format!("XML 解碼錯誤：{err}"))?
                    .into_owned();
                if !text.trim().is_empty() {
                    texts.push(text);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML 解析錯誤：{e}")),
            _ => {}
        }
    }

    let count = texts.len();
    Ok((texts.join(TEXT_DELIMITER), count))
}

/// Replace text nodes in XHTML with converted text (split by delimiter).
pub fn replace_text(xhtml: &str, converted: &str) -> Result<String, String> {
    let segments: Vec<&str> = converted.split(TEXT_DELIMITER).collect();
    let mut seg_idx = 0;

    let mut reader = Reader::from_str(xhtml);
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    loop {
        match reader.read_event() {
            Ok(Event::Text(e)) => {
                let original = e.unescape().map_err(|err| format!("XML 解碼錯誤：{err}"))?;
                if !original.trim().is_empty() && seg_idx < segments.len() {
                    let new_text = BytesText::new(segments[seg_idx]);
                    writer
                        .write_event(Event::Text(new_text))
                        .map_err(|e| format!("XML 寫入錯誤：{e}"))?;
                    seg_idx += 1;
                } else {
                    writer
                        .write_event(Event::Text(e.into_owned()))
                        .map_err(|e| format!("XML 寫入錯誤：{e}"))?;
                }
            }
            Ok(Event::Eof) => break,
            Ok(e) => {
                writer
                    .write_event(e)
                    .map_err(|e| format!("XML 寫入錯誤：{e}"))?;
            }
            Err(e) => return Err(format!("XML 解析錯誤：{e}")),
        }
    }

    let buf = writer.into_inner().into_inner();
    String::from_utf8(buf).map_err(|e| format!("UTF-8 編碼錯誤：{e}"))
}

/// Repack extracted files into a valid EPUB ZIP.
/// The mimetype file must be first and uncompressed per EPUB spec.
pub fn repack_epub(temp_dir: &Path, output_path: &Path) -> Result<(), String> {
    let file = fs::File::create(output_path).map_err(|e| format!("無法建立輸出檔案：{e}"))?;
    let mut zip = ZipWriter::new(file);

    // Write mimetype first (uncompressed, no extra field)
    let mimetype_path = temp_dir.join("mimetype");
    if mimetype_path.exists() {
        let content =
            fs::read_to_string(&mimetype_path).map_err(|e| format!("無法讀取 mimetype：{e}"))?;
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", options)
            .map_err(|e| format!("ZIP 寫入錯誤：{e}"))?;
        zip.write_all(content.as_bytes())
            .map_err(|e| format!("ZIP 寫入錯誤：{e}"))?;
    }

    // Add all other files
    let all_files = collect_files(temp_dir)?;
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for relative in &all_files {
        if relative == "mimetype" {
            continue;
        }
        let full_path = temp_dir.join(relative);
        let content = fs::read(&full_path).map_err(|e| format!("無法讀取 {relative}：{e}"))?;
        zip.start_file(relative, options)
            .map_err(|e| format!("ZIP 寫入錯誤：{e}"))?;
        zip.write_all(&content)
            .map_err(|e| format!("ZIP 寫入錯誤：{e}"))?;
    }

    zip.finish().map_err(|e| format!("ZIP 完成錯誤：{e}"))?;
    Ok(())
}

fn collect_files(dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_files_recursive(dir, dir, &mut files)?;
    Ok(files)
}

fn collect_files_recursive(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("無法讀取目錄：{e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("無法讀取目錄項目：{e}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, files)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| format!("路徑錯誤：{e}"))?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(())
}

/// Get the chapter name from a file path for progress display.
pub fn chapter_display_name(relative_path: &str) -> String {
    PathBuf::from(relative_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| relative_path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_basic() {
        let xhtml = r#"<html><body><p>你好</p><p>世界</p></body></html>"#;
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 2);
        assert!(text.contains("你好"));
        assert!(text.contains("世界"));
        assert!(text.contains(TEXT_DELIMITER));
    }

    #[test]
    fn extract_text_preserves_whitespace_only_skips() {
        let xhtml = r#"<html><body><p>文字</p>  <p>內容</p></body></html>"#;
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 2);
        assert!(!text.starts_with(TEXT_DELIMITER));
    }

    #[test]
    fn extract_text_empty_body() {
        let xhtml = r#"<html><body></body></html>"#;
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 0);
        assert!(text.is_empty());
    }

    #[test]
    fn replace_text_basic() {
        let xhtml = r#"<html><body><p>你好</p><p>世界</p></body></html>"#;
        let converted = format!("Hello{TEXT_DELIMITER}World");
        let result = replace_text(xhtml, &converted).unwrap();
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
        assert!(!result.contains("你好"));
    }

    #[test]
    fn replace_text_preserves_tags() {
        let xhtml = r#"<html><body><div class="test"><p>文字</p></div></body></html>"#;
        let result = replace_text(xhtml, "text").unwrap();
        assert!(result.contains(r#"class="test""#));
        assert!(result.contains("text"));
    }

    #[test]
    fn roundtrip_extract_replace() {
        let xhtml = r#"<html><body><h1>標題</h1><p>段落一</p><p>段落二</p></body></html>"#;
        let (text, _) = extract_text(xhtml).unwrap();
        let result = replace_text(xhtml, &text).unwrap();
        // Should contain all original text
        assert!(result.contains("標題"));
        assert!(result.contains("段落一"));
        assert!(result.contains("段落二"));
    }

    #[test]
    fn chapter_display_name_extracts_stem() {
        assert_eq!(chapter_display_name("OEBPS/chapter1.xhtml"), "chapter1");
        assert_eq!(chapter_display_name("content.html"), "content");
    }

    #[test]
    fn chapter_display_name_no_extension() {
        assert_eq!(chapter_display_name("README"), "README");
    }
}
