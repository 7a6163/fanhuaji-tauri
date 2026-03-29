use quick_xml::events::{BytesText, Event};
use quick_xml::{Reader, Writer};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// A content file found in the EPUB's OPF manifest.
#[derive(Debug)]
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
    use std::io::Write;
    use tempfile::NamedTempFile;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Build a minimal but valid EPUB ZIP in memory and return its bytes.
    /// `extra_entries` is a list of (name, content) pairs written after the
    /// required EPUB skeleton entries.
    fn build_epub_bytes(extra_entries: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);

        // mimetype must be first and stored (uncompressed)
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflated =
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        // Minimal META-INF/container.xml
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )
        .unwrap();

        // Minimal OPF
        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata/>
  <manifest>
    <item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#,
        )
        .unwrap();

        // Default chapter
        zip.start_file("OEBPS/chapter1.xhtml", deflated).unwrap();
        zip.write_all(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>測試章節</p></body>
</html>"#
            .as_bytes(),
        )
        .unwrap();

        // Caller-supplied extras
        for (name, content) in extra_entries {
            zip.start_file(*name, deflated).unwrap();
            zip.write_all(content).unwrap();
        }

        zip.finish().unwrap().into_inner()
    }

    /// Write EPUB bytes to a `NamedTempFile` and return it (keep alive to
    /// prevent deletion while the test runs).
    fn epub_tempfile(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    // -------------------------------------------------------------------------
    // extract_text
    // -------------------------------------------------------------------------

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
    fn extract_text_with_xhtml_namespace() {
        // Namespace declarations on the root element must not confuse the parser.
        let xhtml = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><p>繁體中文</p></body>
</html>"#;
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 1);
        assert_eq!(text, "繁體中文");
    }

    #[test]
    fn extract_text_mixed_inline_content() {
        // Text nodes split by inline elements (<em>, <strong>) should each be
        // extracted as separate segments.
        let xhtml =
            r#"<html><body><p>前面<em>強調</em>後面</p></body></html>"#;
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 3);
        let parts: Vec<&str> = text.split(TEXT_DELIMITER).collect();
        assert_eq!(parts, vec!["前面", "強調", "後面"]);
    }

    #[test]
    fn extract_text_only_whitespace_nodes_skipped() {
        // Newlines and spaces between tags should not produce segments.
        let xhtml = "<html>\n  <body>\n    <p>唯一文字</p>\n  </body>\n</html>";
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 1);
        assert_eq!(text, "唯一文字");
    }

    #[test]
    fn extract_text_single_segment_no_delimiter() {
        let xhtml = r#"<html><body><p>孤獨段落</p></body></html>"#;
        let (text, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 1);
        assert!(!text.contains(TEXT_DELIMITER));
        assert_eq!(text, "孤獨段落");
    }

    // -------------------------------------------------------------------------
    // replace_text
    // -------------------------------------------------------------------------

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
    fn replace_text_nested_tags() {
        // Replacement must leave all wrapper elements intact.
        let xhtml =
            r#"<html><body><div><section><p>深層文字</p></section></div></body></html>"#;
        let result = replace_text(xhtml, "Deep text").unwrap();
        assert!(result.contains("<div>"));
        assert!(result.contains("<section>"));
        assert!(result.contains("<p>"));
        assert!(result.contains("Deep text"));
        assert!(!result.contains("深層文字"));
    }

    #[test]
    fn replace_text_self_closing_tags_preserved() {
        // Self-closing / void elements like <br/> and <img/> must survive unchanged.
        let xhtml =
            r#"<html><body><p>行一<br/>行二</p><img src="cover.jpg"/></body></html>"#;
        let converted = format!("Line one{TEXT_DELIMITER}Line two");
        let result = replace_text(xhtml, &converted).unwrap();
        assert!(result.contains("Line one"));
        assert!(result.contains("Line two"));
        // The self-closing elements should still be present in some form.
        assert!(result.contains("br"));
        assert!(result.contains("img"));
        assert!(result.contains("cover.jpg"));
    }

    #[test]
    fn replace_text_xml_entities_in_source() {
        // Entities like &amp; and &lt; in the original must round-trip correctly.
        let xhtml = r#"<html><body><p>a &amp; b &lt; c</p></body></html>"#;
        // extract_text unescapes, so the single segment is "a & b < c"
        let (extracted, count) = extract_text(xhtml).unwrap();
        assert_eq!(count, 1);
        // Replace with same content — result should still be valid XML with entities re-escaped.
        let result = replace_text(xhtml, &extracted).unwrap();
        // quick-xml re-encodes '&' and '<' in text nodes
        assert!(result.contains("&amp;") || result.contains("& b"));
        // The tag structure must remain valid
        assert!(result.contains("<p>"));
        assert!(result.contains("</p>"));
    }

    #[test]
    fn replace_text_whitespace_only_nodes_pass_through() {
        // Whitespace-only text nodes must not consume a converted segment.
        let xhtml = "<html>\n<body>\n<p>文字</p>\n</body>\n</html>";
        let result = replace_text(xhtml, "replacement").unwrap();
        assert!(result.contains("replacement"));
        // Surrounding whitespace nodes should still be present
        assert!(result.contains('\n'));
    }

    #[test]
    fn replace_text_empty_converted_string() {
        // If there are no converted segments, whitespace-only nodes pass through
        // and non-whitespace nodes are left as-is (seg_idx stays 0, segments
        // is [""] which has length 1, so the first real text node gets "").
        let xhtml = r#"<html><body><p>原文</p></body></html>"#;
        let result = replace_text(xhtml, "").unwrap();
        // Should produce valid XML without panicking
        assert!(result.contains("<p>"));
        assert!(result.contains("</p>"));
    }

    #[test]
    fn roundtrip_extract_replace() {
        let xhtml = r#"<html><body><h1>標題</h1><p>段落一</p><p>段落二</p></body></html>"#;
        let (text, _) = extract_text(xhtml).unwrap();
        let result = replace_text(xhtml, &text).unwrap();
        assert!(result.contains("標題"));
        assert!(result.contains("段落一"));
        assert!(result.contains("段落二"));
    }

    // -------------------------------------------------------------------------
    // find_content_files (via a temp directory built manually)
    // -------------------------------------------------------------------------

    #[test]
    fn find_content_files_returns_only_content_types() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create a nested directory layout
        fs::create_dir_all(root.join("OEBPS/Text")).unwrap();
        fs::create_dir_all(root.join("OEBPS/Images")).unwrap();

        fs::write(root.join("OEBPS/Text/chapter1.xhtml"), b"<html/>").unwrap();
        fs::write(root.join("OEBPS/Text/chapter2.XHTML"), b"<html/>").unwrap(); // uppercase extension
        fs::write(root.join("OEBPS/Text/appendix.html"), b"<html/>").unwrap();
        fs::write(root.join("OEBPS/Text/intro.htm"), b"<html/>").unwrap();
        fs::write(root.join("OEBPS/Images/cover.jpg"), b"JFIF").unwrap();
        fs::write(root.join("OEBPS/content.opf"), b"<opf/>").unwrap();
        fs::write(root.join("mimetype"), b"application/epub+zip").unwrap();

        let files = find_content_files(root).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

        // Only .xhtml, .XHTML, .html, .htm should appear
        assert_eq!(files.len(), 4, "expected 4 content files, got: {names:?}");

        // Results must be sorted
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "results should be sorted");

        // Non-content files must be absent
        for name in &names {
            assert!(
                !name.ends_with(".jpg") && !name.ends_with(".opf") && *name != "mimetype",
                "unexpected file in results: {name}"
            );
        }
    }

    #[test]
    fn find_content_files_empty_directory() {
        let dir = TempDir::new().unwrap();
        let files = find_content_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn find_content_files_no_content_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("mimetype"), b"application/epub+zip").unwrap();
        fs::write(dir.path().join("cover.png"), b"PNG").unwrap();
        let files = find_content_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    // -------------------------------------------------------------------------
    // extract_epub
    // -------------------------------------------------------------------------

    #[test]
    fn extract_epub_extracts_files_and_finds_content() {
        let bytes = build_epub_bytes(&[]);
        let tmp = epub_tempfile(&bytes);

        let (dir, content_files) = extract_epub(tmp.path()).unwrap();

        // mimetype must have been extracted
        assert!(dir.path().join("mimetype").exists());
        // chapter1.xhtml must have been extracted under OEBPS/
        assert!(dir.path().join("OEBPS/chapter1.xhtml").exists());
        // The content file list must contain chapter1.xhtml
        assert_eq!(content_files.len(), 1);
        assert!(content_files[0].relative_path.contains("chapter1.xhtml"));
    }

    #[test]
    fn extract_epub_finds_multiple_content_files_sorted() {
        let ch2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><p>第二章</p></body></html>"#
            .as_bytes();
        let bytes = build_epub_bytes(&[("OEBPS/chapter2.xhtml", ch2)]);
        let tmp = epub_tempfile(&bytes);

        let (_dir, content_files) = extract_epub(tmp.path()).unwrap();

        assert_eq!(content_files.len(), 2);
        // Must be sorted: chapter1 before chapter2
        assert!(content_files[0].relative_path < content_files[1].relative_path);
    }

    #[test]
    fn extract_epub_ignores_non_content_files() {
        let bytes = build_epub_bytes(&[("OEBPS/cover.jpg", b"JFIF")]);
        let tmp = epub_tempfile(&bytes);

        let (_dir, content_files) = extract_epub(tmp.path()).unwrap();

        // cover.jpg must not appear in content_files
        for cf in &content_files {
            assert!(
                !cf.relative_path.ends_with(".jpg"),
                "jpg should not be a content file"
            );
        }
    }

    #[test]
    fn extract_epub_invalid_zip_returns_error() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"this is not a zip file").unwrap();
        f.flush().unwrap();

        let result = extract_epub(f.path());
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("無效的 EPUB") || msg.contains("EPUB"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn extract_epub_missing_file_returns_error() {
        let result = extract_epub(Path::new("/nonexistent/path/book.epub"));
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("無法開啟 EPUB") || msg.contains("EPUB"));
    }

    // -------------------------------------------------------------------------
    // repack_epub
    // -------------------------------------------------------------------------

    #[test]
    fn repack_epub_mimetype_is_first_and_stored() {
        // Build and extract a minimal EPUB, then repack it and inspect the result.
        let bytes = build_epub_bytes(&[]);
        let tmp = epub_tempfile(&bytes);
        let (extracted_dir, _) = extract_epub(tmp.path()).unwrap();

        let output = NamedTempFile::new().unwrap();
        repack_epub(extracted_dir.path(), output.path()).unwrap();

        // Open the repacked ZIP and verify mimetype position and compression.
        let repacked_file = fs::File::open(output.path()).unwrap();
        let mut archive = ZipArchive::new(repacked_file).unwrap();

        assert!(archive.len() > 0, "repacked ZIP must not be empty");

        // Entry at index 0 must be mimetype
        let entry0 = archive.by_index(0).unwrap();
        assert_eq!(entry0.name(), "mimetype");
        assert_eq!(
            entry0.compression(),
            CompressionMethod::Stored,
            "mimetype must be stored (uncompressed)"
        );
        drop(entry0);

        // mimetype content must be correct
        let mut mime_entry = archive.by_name("mimetype").unwrap();
        let mut content = String::new();
        mime_entry.read_to_string(&mut content).unwrap();
        assert_eq!(content, "application/epub+zip");
    }

    #[test]
    fn repack_epub_contains_all_files() {
        let ch2 = r#"<html><body><p>第二章</p></body></html>"#.as_bytes();
        let bytes = build_epub_bytes(&[("OEBPS/chapter2.xhtml", ch2)]);
        let tmp = epub_tempfile(&bytes);
        let (extracted_dir, _) = extract_epub(tmp.path()).unwrap();

        let output = NamedTempFile::new().unwrap();
        repack_epub(extracted_dir.path(), output.path()).unwrap();

        let repacked_file = fs::File::open(output.path()).unwrap();
        let mut archive = ZipArchive::new(repacked_file).unwrap();

        // Collect all entry names
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();

        assert!(names.contains(&"mimetype".to_string()));
        assert!(
            names.iter().any(|n| n.contains("chapter1.xhtml")),
            "chapter1.xhtml missing from repacked archive"
        );
        assert!(
            names.iter().any(|n| n.contains("chapter2.xhtml")),
            "chapter2.xhtml missing from repacked archive"
        );
    }

    #[test]
    fn repack_epub_missing_mimetype_still_succeeds() {
        // If the temp directory has no mimetype file, repack should succeed
        // (the mimetype block is guarded by an `if exists` check).
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("content.xhtml"), b"<html/>").unwrap();

        let output = NamedTempFile::new().unwrap();
        let result = repack_epub(dir.path(), output.path());
        assert!(result.is_ok(), "repack without mimetype should not error");

        // The output must still be a valid ZIP containing content.xhtml
        let repacked_file = fs::File::open(output.path()).unwrap();
        let mut archive = ZipArchive::new(repacked_file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.iter().any(|n| n.contains("content.xhtml")));
    }

    // -------------------------------------------------------------------------
    // chapter_display_name
    // -------------------------------------------------------------------------

    #[test]
    fn chapter_display_name_extracts_stem() {
        assert_eq!(chapter_display_name("OEBPS/chapter1.xhtml"), "chapter1");
        assert_eq!(chapter_display_name("content.html"), "content");
    }

    #[test]
    fn chapter_display_name_no_extension() {
        assert_eq!(chapter_display_name("README"), "README");
    }

    #[test]
    fn chapter_display_name_deeply_nested() {
        assert_eq!(
            chapter_display_name("OEBPS/Text/part1/section2.xhtml"),
            "section2"
        );
    }

    #[test]
    fn chapter_display_name_empty_string() {
        // An empty path has no stem, so the fallback returns the input unchanged.
        assert_eq!(chapter_display_name(""), "");
    }
}
