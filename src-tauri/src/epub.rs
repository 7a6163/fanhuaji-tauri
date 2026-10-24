use quick_xml::escape::resolve_html5_entity;
use quick_xml::events::{BytesRef, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// What kind of XML a content file holds; decides which text nodes are converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// Chapter or nav document: every text node is converted.
    Xhtml,
    /// Package document: only descriptive `dc:*` metadata is converted.
    Opf,
    /// NCX table of contents: every text node is converted.
    Ncx,
}

/// A convertible file found in the extracted EPUB.
#[derive(Debug)]
pub struct ContentFile {
    pub relative_path: String,
    pub kind: ContentKind,
}

/// Dublin Core elements in the OPF whose text is human-readable.
/// Matched by local name, ignoring case, so any prefix (or none) works.
const OPF_TEXT_ELEMENTS: &[&[u8]] = &[
    b"title",
    b"creator",
    b"contributor",
    b"description",
    b"subject",
    b"publisher",
];

/// Delimiter used to separate text nodes for batch API conversion.
const TEXT_DELIMITER: &str = "\x00\x01\x00";

/// Zip bomb protection: reject any single entry whose decompressed size
/// exceeds this limit. 100 MiB comfortably accommodates high-resolution
/// images and large chapters in legitimate EPUBs.
const MAX_ENTRY_BYTES: u64 = 100 * 1024 * 1024;

/// Zip bomb protection: reject archives whose cumulative decompressed
/// size exceeds this limit. 500 MiB is well above typical EPUB sizes
/// while blocking archives that expand into gigabytes.
const MAX_TOTAL_BYTES: u64 = 500 * 1024 * 1024;

/// Extract an EPUB ZIP to a temp directory and return content file paths.
pub fn extract_epub(epub_path: &Path) -> Result<(TempDir, Vec<ContentFile>), String> {
    let file = fs::File::open(epub_path).map_err(|e| format!("EPUB_OPEN_FAILED:{e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("EPUB_INVALID:{e}"))?;

    let temp_dir = TempDir::new().map_err(|e| format!("TEMPDIR_FAILED:{e}"))?;

    let mut total_extracted: u64 = 0;

    // Extract all files
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("EPUB_READ_ENTRY_FAILED:{e}"))?;
        let name = entry.name().to_string();

        // Prevent ZIP path traversal, for directory entries too. Path::starts_with
        // is component-based and treats "/tmp/x/../evil" as prefixed by "/tmp/x",
        // so it cannot catch ".." traversal on its own. Reject absolute paths and
        // any entry whose components contain ParentDir ("..") or RootDir ("/").
        let rel = Path::new(&name);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
        {
            return Err(format!("EPUB_UNSAFE_PATH:{name}"));
        }

        let out_path = temp_dir.path().join(rel);

        // Defence in depth: verify the joined path still sits under the
        // tempdir root (covers edge cases like Windows drive prefixes).
        if !out_path.starts_with(temp_dir.path()) {
            return Err(format!("EPUB_UNSAFE_PATH:{name}"));
        }

        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| format!("DIR_CREATE_FAILED:{e}"))?;
            continue;
        }

        // Reject entries whose declared uncompressed size exceeds the limit
        // without spending CPU on decompression. The header is untrusted, so
        // we also enforce the limit during the actual read below.
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(format!("EPUB_ENTRY_TOO_LARGE:{name}"));
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("DIR_CREATE_FAILED:{e}"))?;
        }

        // Cap the decompressed read to MAX_ENTRY_BYTES + 1 so we can detect
        // headers that lie about the true uncompressed size (zip bomb).
        let mut buf = Vec::new();
        let limit = MAX_ENTRY_BYTES + 1;
        (&mut entry)
            .take(limit)
            .read_to_end(&mut buf)
            .map_err(|e| format!("FILE_READ_FAILED:{e}"))?;
        if buf.len() as u64 > MAX_ENTRY_BYTES {
            return Err(format!("EPUB_ENTRY_TOO_LARGE:{name}"));
        }

        total_extracted = total_extracted.saturating_add(buf.len() as u64);
        if total_extracted > MAX_TOTAL_BYTES {
            return Err("EPUB_TOTAL_TOO_LARGE".to_string());
        }

        fs::write(&out_path, &buf).map_err(|e| format!("FILE_WRITE_FAILED:{e}"))?;
    }

    // Find content files by scanning for .xhtml/.html files
    let content_files = find_content_files(temp_dir.path())?;

    Ok((temp_dir, content_files))
}

/// Recursively find all convertible files (.xhtml/.html/.htm, .opf, .ncx) in the extracted EPUB.
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
    let entries = fs::read_dir(dir).map_err(|e| format!("DIR_READ_FAILED:{e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("DIR_ENTRY_READ_FAILED:{e}"))?;
        let path = entry.path();
        if path.is_dir() {
            find_content_files_recursive(root, &path, files)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_lowercase();
            let kind = match ext_lower.as_str() {
                "xhtml" | "html" | "htm" => Some(ContentKind::Xhtml),
                "opf" => Some(ContentKind::Opf),
                "ncx" => Some(ContentKind::Ncx),
                _ => None,
            };
            if let Some(kind) = kind {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| format!("PATH_STRIP_FAILED:{e}"))?
                    .to_string_lossy()
                    .into_owned();
                files.push(ContentFile {
                    relative_path: relative,
                    kind,
                });
            }
        }
    }
    Ok(())
}

/// Elements whose text is code, not prose: converting `<style>` would turn
/// font names such as "宋体" into "宋體" and break the CSS.
const XHTML_SKIP_ELEMENTS: &[&[u8]] = &[b"style", b"script"];

/// Tracks whether the reader is inside an element whose text should be
/// converted: inside one of `include` (or anywhere when it is `None`) and not
/// inside any of `exclude`.
struct TextScope {
    include: Option<&'static [&'static [u8]]>,
    exclude: &'static [&'static [u8]],
    include_depth: usize,
    exclude_depth: usize,
}

/// Depth bookkeeping for "inside a matching element": once inside, every
/// nested start tag deepens it; otherwise only a matching tag starts it.
fn nest(depth: &mut usize, matches: bool) {
    if *depth > 0 {
        *depth += 1;
    } else if matches {
        *depth = 1;
    }
}

fn matches_any(elements: &[&[u8]], name: &[u8]) -> bool {
    elements.iter().any(|el| el.eq_ignore_ascii_case(name))
}

impl TextScope {
    fn new(include: Option<&'static [&'static [u8]]>, exclude: &'static [&'static [u8]]) -> Self {
        Self {
            include,
            exclude,
            include_depth: 0,
            exclude_depth: 0,
        }
    }

    fn enter(&mut self, name: &[u8]) {
        let included = self.include.is_some_and(|els| matches_any(els, name));
        nest(&mut self.include_depth, included);
        nest(&mut self.exclude_depth, matches_any(self.exclude, name));
    }

    fn leave(&mut self) {
        self.include_depth = self.include_depth.saturating_sub(1);
        self.exclude_depth = self.exclude_depth.saturating_sub(1);
    }

    fn active(&self) -> bool {
        (self.include.is_none() || self.include_depth > 0) && self.exclude_depth == 0
    }
}

fn scope_for(kind: ContentKind) -> TextScope {
    match kind {
        ContentKind::Opf => TextScope::new(Some(OPF_TEXT_ELEMENTS), &[]),
        ContentKind::Xhtml => TextScope::new(None, XHTML_SKIP_ELEMENTS),
        ContentKind::Ncx => TextScope::new(None, &[]),
    }
}

/// A text node as quick-xml (0.38+) reports it: `Text` events split at every
/// entity reference (`&amp;`, `&nbsp;`, `&#x4E2D;`), each reference its own
/// `GeneralRef` event. The run is collected so the node is converted as one piece.
#[derive(Default)]
struct TextRun {
    text: String,
    events: Vec<Event<'static>>,
}

impl TextRun {
    fn push(&mut self, event: Event<'_>) -> Result<(), String> {
        match &event {
            Event::Text(t) => self.text.push_str(t),
            Event::GeneralRef(r) => self.text.push_str(&resolve_ref(r)?),
            _ => unreachable!("TextRun only takes Text and GeneralRef events"),
        }
        self.events.push(event.into_owned());
        Ok(())
    }
}

fn resolve_ref(r: &BytesRef) -> Result<String, String> {
    if let Some(c) = r
        .resolve_char_ref()
        .map_err(|e| format!("XML_DECODE_FAILED:{e}"))?
    {
        return Ok(c.to_string());
    }
    // HTML5 names cover the XML predefined ones; chapters often use &nbsp; or &hellip;.
    resolve_html5_entity(r)
        .map(str::to_string)
        .ok_or_else(|| format!("XML_DECODE_FAILED:unknown entity &{};", &**r))
}

fn is_text(event: &Event) -> bool {
    matches!(event, Event::Text(_) | Event::GeneralRef(_))
}

/// Extract the text nodes relevant for `kind`, joining with a delimiter.
/// Returns the concatenated text and the count of text segments.
pub fn extract_text(kind: ContentKind, xml: &str) -> Result<(String, usize), String> {
    let mut reader = Reader::from_str(xml);
    let mut scope = scope_for(kind);
    let mut texts = Vec::new();
    let mut run = TextRun::default();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("XML_PARSE_FAILED:{e}"))?;
        if scope.active() && is_text(&event) {
            run.push(event)?;
            continue;
        }
        // Any other event ends the text node, and must be flushed before the scope changes.
        let text = std::mem::take(&mut run).text;
        if !text.trim().is_empty() {
            texts.push(text);
        }
        match event {
            Event::Start(e) => scope.enter(e.local_name().as_ref().as_bytes()),
            Event::End(_) => scope.leave(),
            Event::Eof => break,
            _ => {}
        }
    }

    let count = texts.len();
    Ok((texts.join(TEXT_DELIMITER), count))
}

/// Replace the text nodes `extract_text` yields with converted text
/// (split by delimiter). Fails with `EPUB_SEGMENT_MISMATCH` when the number of
/// segments differs from the number of text nodes, since the text would then
/// land in the wrong elements.
pub fn replace_text(kind: ContentKind, xml: &str, converted: &str) -> Result<String, String> {
    let segments: Vec<&str> = converted.split(TEXT_DELIMITER).collect();
    let mut nodes = 0;

    let mut reader = Reader::from_str(xml);
    let mut scope = scope_for(kind);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut write = |e: Event| {
        writer
            .write_event(e)
            .map_err(|e| format!("XML_WRITE_FAILED:{e}"))
    };
    let mut run = TextRun::default();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("XML_PARSE_FAILED:{e}"))?;
        if scope.active() && is_text(&event) {
            run.push(event)?;
            continue;
        }
        let done = std::mem::take(&mut run);
        if !done.events.is_empty() {
            let segment = if done.text.trim().is_empty() {
                None
            } else {
                nodes += 1;
                segments.get(nodes - 1)
            };
            match segment {
                Some(segment) => write(Event::Text(BytesText::new(segment)))?,
                // Whitespace-only nodes (and any surplus) are written back untouched.
                None => {
                    for e in done.events {
                        write(e)?;
                    }
                }
            }
        }
        match &event {
            Event::Start(s) => scope.enter(s.local_name().as_ref().as_bytes()),
            Event::End(_) => scope.leave(),
            Event::Eof => break,
            _ => {}
        }
        write(event)?;
    }

    if nodes != segments.len() {
        return Err(format!("EPUB_SEGMENT_MISMATCH:{}/{nodes}", segments.len()));
    }

    let buf = writer.into_inner().into_inner();
    String::from_utf8(buf).map_err(|e| format!("UTF8_ENCODE_FAILED:{e}"))
}

/// Repack extracted files into a valid EPUB ZIP, written atomically so an
/// overwritten input survives a failed repack.
/// The mimetype file must be first and uncompressed per EPUB spec.
pub fn repack_epub(temp_dir: &Path, output_path: &Path) -> Result<(), String> {
    crate::write_atomic(output_path, |file| write_epub_zip(temp_dir, file))
}

fn write_epub_zip(temp_dir: &Path, file: &mut fs::File) -> Result<(), String> {
    let mut zip = ZipWriter::new(file);

    // Write mimetype first (uncompressed, no extra field)
    let mimetype_path = temp_dir.join("mimetype");
    if mimetype_path.exists() {
        let content =
            fs::read_to_string(&mimetype_path).map_err(|e| format!("MIMETYPE_READ_FAILED:{e}"))?;
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", options)
            .map_err(|e| format!("ZIP_WRITE_FAILED:{e}"))?;
        zip.write_all(content.as_bytes())
            .map_err(|e| format!("ZIP_WRITE_FAILED:{e}"))?;
    }

    // Add all other files
    let all_files = collect_files(temp_dir)?;
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for relative in &all_files {
        if relative == "mimetype" {
            continue;
        }
        let full_path = temp_dir.join(relative);
        let content =
            fs::read(&full_path).map_err(|e| format!("FILE_READ_FAILED:{relative}: {e}"))?;
        zip.start_file(relative, options)
            .map_err(|e| format!("ZIP_WRITE_FAILED:{e}"))?;
        zip.write_all(&content)
            .map_err(|e| format!("ZIP_WRITE_FAILED:{e}"))?;
    }

    zip.finish().map_err(|e| format!("ZIP_FINISH_FAILED:{e}"))?;
    Ok(())
}

fn collect_files(dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_files_recursive(dir, dir, &mut files)?;
    Ok(files)
}

fn collect_files_recursive(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("DIR_READ_FAILED:{e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("DIR_ENTRY_READ_FAILED:{e}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, files)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| format!("PATH_STRIP_FAILED:{e}"))?
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

        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

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
  <metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Book Title</dc:title></metadata>
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
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 2);
        assert!(text.contains("你好"));
        assert!(text.contains("世界"));
        assert!(text.contains(TEXT_DELIMITER));
    }

    #[test]
    fn extract_text_preserves_whitespace_only_skips() {
        let xhtml = r#"<html><body><p>文字</p>  <p>內容</p></body></html>"#;
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 2);
        assert!(!text.starts_with(TEXT_DELIMITER));
    }

    #[test]
    fn extract_text_empty_body() {
        let xhtml = r#"<html><body></body></html>"#;
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
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
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 1);
        assert_eq!(text, "繁體中文");
    }

    #[test]
    fn extract_text_mixed_inline_content() {
        // Text nodes split by inline elements (<em>, <strong>) should each be
        // extracted as separate segments.
        let xhtml = r#"<html><body><p>前面<em>強調</em>後面</p></body></html>"#;
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 3);
        let parts: Vec<&str> = text.split(TEXT_DELIMITER).collect();
        assert_eq!(parts, vec!["前面", "強調", "後面"]);
    }

    #[test]
    fn extract_text_only_whitespace_nodes_skipped() {
        // Newlines and spaces between tags should not produce segments.
        let xhtml = "<html>\n  <body>\n    <p>唯一文字</p>\n  </body>\n</html>";
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 1);
        assert_eq!(text, "唯一文字");
    }

    #[test]
    fn extract_text_malformed_xml_returns_parse_error() {
        // Unclosed attribute quote forces quick-xml to emit an Err event.
        let xhtml = r#"<html><body><p attr="unclosed><</p></body></html>"#;
        let result = extract_text(ContentKind::Xhtml, xhtml);
        assert!(result.is_err(), "malformed XML must produce an error");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("XML_PARSE_FAILED:") || msg.starts_with("XML_DECODE_FAILED:"),
            "expected XML parse/decode error, got: {msg}"
        );
    }

    #[test]
    fn replace_text_malformed_xml_returns_parse_error() {
        let xhtml = r#"<html><body><p attr="unclosed><</p></body></html>"#;
        let result = replace_text(ContentKind::Xhtml, xhtml, "replacement");
        assert!(result.is_err(), "malformed XML must produce an error");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("XML_PARSE_FAILED:") || msg.starts_with("XML_DECODE_FAILED:"),
            "expected XML parse/decode error, got: {msg}"
        );
    }

    #[test]
    fn extract_text_single_segment_no_delimiter() {
        let xhtml = r#"<html><body><p>孤獨段落</p></body></html>"#;
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
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
        let result = replace_text(ContentKind::Xhtml, xhtml, &converted).unwrap();
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
        assert!(!result.contains("你好"));
    }

    #[test]
    fn replace_text_preserves_tags() {
        let xhtml = r#"<html><body><div class="test"><p>文字</p></div></body></html>"#;
        let result = replace_text(ContentKind::Xhtml, xhtml, "text").unwrap();
        assert!(result.contains(r#"class="test""#));
        assert!(result.contains("text"));
    }

    #[test]
    fn replace_text_nested_tags() {
        // Replacement must leave all wrapper elements intact.
        let xhtml = r#"<html><body><div><section><p>深層文字</p></section></div></body></html>"#;
        let result = replace_text(ContentKind::Xhtml, xhtml, "Deep text").unwrap();
        assert!(result.contains("<div>"));
        assert!(result.contains("<section>"));
        assert!(result.contains("<p>"));
        assert!(result.contains("Deep text"));
        assert!(!result.contains("深層文字"));
    }

    #[test]
    fn replace_text_self_closing_tags_preserved() {
        // Self-closing / void elements like <br/> and <img/> must survive unchanged.
        let xhtml = r#"<html><body><p>行一<br/>行二</p><img src="cover.jpg"/></body></html>"#;
        let converted = format!("Line one{TEXT_DELIMITER}Line two");
        let result = replace_text(ContentKind::Xhtml, xhtml, &converted).unwrap();
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
        let (extracted, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 1);
        // Replace with same content — result should still be valid XML with entities re-escaped.
        let result = replace_text(ContentKind::Xhtml, xhtml, &extracted).unwrap();
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
        let result = replace_text(ContentKind::Xhtml, xhtml, "replacement").unwrap();
        assert!(result.contains("replacement"));
        // Surrounding whitespace nodes should still be present
        assert!(result.contains('\n'));
    }

    #[test]
    fn replace_text_empty_converted_string() {
        // "" splits into one empty segment, which replaces the single text node.
        let xhtml = r#"<html><body><p>原文</p></body></html>"#;
        let result = replace_text(ContentKind::Xhtml, xhtml, "").unwrap();
        // Should produce valid XML without panicking
        assert!(result.contains("<p>"));
        assert!(result.contains("</p>"));
    }

    #[test]
    fn replace_text_rejects_too_few_segments() {
        let xhtml = r#"<html><body><p>一</p><p>二</p><p>三</p></body></html>"#;
        let converted = format!("壹{TEXT_DELIMITER}貳");
        let err = replace_text(ContentKind::Xhtml, xhtml, &converted).unwrap_err();
        assert_eq!(err, "EPUB_SEGMENT_MISMATCH:2/3");
    }

    #[test]
    fn replace_text_rejects_too_many_segments() {
        let xhtml = r#"<html><body><p>一</p></body></html>"#;
        let converted = format!("壹{TEXT_DELIMITER}貳");
        let err = replace_text(ContentKind::Xhtml, xhtml, &converted).unwrap_err();
        assert_eq!(err, "EPUB_SEGMENT_MISMATCH:2/1");
    }

    #[test]
    fn xhtml_skips_style_and_script() {
        let xhtml = r#"<html><head><title>标题</title><style>p{font-family:"宋体";}</style><script>var s="简体";</script></head><body><p>正文</p></body></html>"#;
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            text.split(TEXT_DELIMITER).collect::<Vec<_>>(),
            vec!["标题", "正文"]
        );

        let converted = format!("標題{TEXT_DELIMITER}正文");
        let result = replace_text(ContentKind::Xhtml, xhtml, &converted).unwrap();
        assert_eq!(
            result,
            xhtml.replace("<title>标题", "<title>標題"),
            "style and script must be untouched"
        );
    }

    #[test]
    fn ncx_still_converts_every_text_node() {
        // The style/script exclusion is XHTML-only.
        let ncx = r#"<ncx><docTitle><text>书名</text></docTitle><style>不是样式</style></ncx>"#;
        let (_, count) = extract_text(ContentKind::Ncx, ncx).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn roundtrip_extract_replace() {
        let xhtml = r#"<html><body><h1>標題</h1><p>段落一</p><p>段落二</p></body></html>"#;
        let (text, _) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        let result = replace_text(ContentKind::Xhtml, xhtml, &text).unwrap();
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
        fs::write(root.join("OEBPS/toc.ncx"), b"<ncx/>").unwrap();
        fs::write(root.join("mimetype"), b"application/epub+zip").unwrap();

        let files = find_content_files(root).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

        // .xhtml, .XHTML, .html, .htm plus the .opf and .ncx metadata files
        assert_eq!(files.len(), 6, "expected 6 content files, got: {names:?}");

        // Results must be sorted
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "results should be sorted");

        // Non-content files must be absent
        for name in &names {
            assert!(
                !name.ends_with(".jpg") && *name != "mimetype",
                "unexpected file in results: {name}"
            );
        }
    }

    #[test]
    fn find_content_files_tags_kinds() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.xhtml"), b"<html/>").unwrap();
        fs::write(dir.path().join("b.opf"), b"<package/>").unwrap();
        fs::write(dir.path().join("c.NCX"), b"<ncx/>").unwrap();
        let kinds: Vec<ContentKind> = find_content_files(dir.path())
            .unwrap()
            .iter()
            .map(|f| f.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![ContentKind::Xhtml, ContentKind::Opf, ContentKind::Ncx]
        );
    }

    #[test]
    fn opf_converts_only_descriptive_metadata() {
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <metadata>
    <dc:identifier id="id">abc-123</dc:identifier>
    <dc:title>测试书名</dc:title>
    <dc:creator opf:role="aut">测试作者</dc:creator>
    <dc:description>简介：<b>示例</b>内容</dc:description>
    <dc:language>zh</dc:language>
    <dc:date>2026-08-09</dc:date>
  </metadata>
  <manifest><item id="c1" href="a.xhtml"/></manifest>
</package>"#;
        let (text, count) = extract_text(ContentKind::Opf, opf).unwrap();
        assert_eq!(count, 5);
        assert_eq!(
            text.split(TEXT_DELIMITER).collect::<Vec<_>>(),
            vec!["测试书名", "测试作者", "简介：", "示例", "内容"]
        );
    }

    #[test]
    fn opf_replace_leaves_identifiers_untouched() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:identifier>abc-123</dc:identifier><dc:title>书名</dc:title><dc:language>zh</dc:language></metadata></package>"#;
        let result = replace_text(ContentKind::Opf, opf, "書名").unwrap();
        assert_eq!(
            result,
            opf.replace("书名", "書名"),
            "only the title text may change"
        );
    }

    #[test]
    fn opf_ignores_bad_entity_outside_scope() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:rights>&bogus; 2026</dc:rights><dc:title>书名</dc:title></metadata></package>"#;
        let (text, count) = extract_text(ContentKind::Opf, opf).unwrap();
        assert_eq!((text.as_str(), count), ("书名", 1));
        let result = replace_text(ContentKind::Opf, opf, "書名").unwrap();
        assert_eq!(result, opf.replace("书名", "書名"));
    }

    #[test]
    fn xhtml_html_entities_are_converted() {
        let xhtml = "<html><body><p>第一章&nbsp;开始&hellip;</p><p>&copy; 作者</p></body></html>";
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            text.split(TEXT_DELIMITER).collect::<Vec<_>>(),
            vec!["第一章\u{a0}开始\u{2026}", "\u{a9} 作者"]
        );

        let converted = format!("第一章\u{a0}開始\u{2026}{TEXT_DELIMITER}\u{a9} 作者");
        let result = replace_text(ContentKind::Xhtml, xhtml, &converted).unwrap();
        assert_eq!(
            result,
            "<html><body><p>第一章\u{a0}開始\u{2026}</p><p>\u{a9} 作者</p></body></html>"
        );
    }

    #[test]
    fn char_refs_stay_in_one_text_node() {
        let xhtml = "<html><body><p>&#x4E2D;&#25991; 字幕</p><p>  </p></body></html>";
        let (text, count) = extract_text(ContentKind::Xhtml, xhtml).unwrap();
        assert_eq!((text.as_str(), count), ("中文 字幕", 1));

        let result = replace_text(ContentKind::Xhtml, xhtml, "中文 字幕 & 歌詞").unwrap();
        assert_eq!(
            result,
            "<html><body><p>中文 字幕 &amp; 歌詞</p><p>  </p></body></html>"
        );
    }

    #[test]
    fn opf_matches_any_dc_prefix_and_case() {
        let opf = r#"<package><metadata xmlns:dcterms="http://purl.org/dc/elements/1.1/"><dcterms:title>一</dcterms:title><title xmlns="http://purl.org/dc/elements/1.1/">二</title><dc:Title>三</dc:Title></metadata></package>"#;
        let (text, count) = extract_text(ContentKind::Opf, opf).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            text.split(TEXT_DELIMITER).collect::<Vec<_>>(),
            vec!["一", "二", "三"]
        );
    }

    #[test]
    fn ncx_replaces_nav_labels() {
        let ncx = r#"<ncx><docTitle><text>测试书名</text></docTitle><navMap><navPoint><navLabel><text>第1章 测试</text></navLabel><content src="a.html"/></navPoint></navMap></ncx>"#;
        let (text, count) = extract_text(ContentKind::Ncx, ncx).unwrap();
        assert_eq!(count, 2);
        let converted = text.replace("测试书名", "測試書名").replace("测试", "測試");
        let result = replace_text(ContentKind::Ncx, ncx, &converted).unwrap();
        assert!(result.contains("<text>測試書名</text>"));
        assert!(result.contains("<text>第1章 測試</text>"));
        assert!(result.contains(r#"src="a.html""#));
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
        // The content file list must contain chapter1.xhtml and the OPF
        let chapters: Vec<_> = content_files
            .iter()
            .filter(|f| f.kind == ContentKind::Xhtml)
            .collect();
        assert_eq!(chapters.len(), 1);
        assert!(chapters[0].relative_path.contains("chapter1.xhtml"));
        assert!(content_files.iter().any(|f| f.kind == ContentKind::Opf));
    }

    #[test]
    fn extract_epub_finds_multiple_content_files_sorted() {
        let ch2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><p>第二章</p></body></html>"#
            .as_bytes();
        let bytes = build_epub_bytes(&[("OEBPS/chapter2.xhtml", ch2)]);
        let tmp = epub_tempfile(&bytes);

        let (_dir, content_files) = extract_epub(tmp.path()).unwrap();

        let chapters: Vec<_> = content_files
            .iter()
            .filter(|f| f.kind == ContentKind::Xhtml)
            .collect();
        assert_eq!(chapters.len(), 2);
        // Must be sorted: chapter1 before chapter2
        assert!(chapters[0].relative_path < chapters[1].relative_path);
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
            msg.starts_with("EPUB_INVALID:") || msg.starts_with("EPUB_"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn extract_epub_missing_file_returns_error() {
        let result = extract_epub(Path::new("/nonexistent/path/book.epub"));
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.starts_with("EPUB_OPEN_FAILED:"));
    }

    #[test]
    fn extract_epub_rejects_oversized_entry() {
        // Build an EPUB with one entry that decompresses to > MAX_ENTRY_BYTES.
        // Repeating 'A' compresses extremely well, so a 101 MiB payload fits
        // comfortably under the 50 MiB upstream size check.
        let oversized: Vec<u8> = vec![b'A'; (MAX_ENTRY_BYTES as usize) + 1024];
        let bytes = build_epub_bytes(&[("OEBPS/huge.xhtml", &oversized)]);
        let tmp = epub_tempfile(&bytes);

        let result = extract_epub(tmp.path());
        assert!(result.is_err(), "oversized entry must be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("EPUB_ENTRY_TOO_LARGE:"),
            "expected size-limit error, got: {msg}"
        );
    }

    #[test]
    fn extract_epub_creates_directory_entries() {
        // Build an EPUB that includes an explicit directory entry
        // (trailing slash). extract_epub must call fs::create_dir_all
        // for the is_dir() branch.
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.add_directory("OEBPS/Text/", deflated).unwrap();
        zip.start_file("OEBPS/Text/chapter.xhtml", deflated)
            .unwrap();
        zip.write_all(b"<html><body><p>dir test</p></body></html>")
            .unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let tmp = epub_tempfile(&bytes);
        let (dir, _) = extract_epub(tmp.path()).expect("extract should succeed");

        assert!(
            dir.path().join("OEBPS/Text").is_dir(),
            "directory entry must be created on disk"
        );
        assert!(dir.path().join("OEBPS/Text/chapter.xhtml").exists());
    }

    #[test]
    fn extract_epub_rejects_path_traversal() {
        // Build an EPUB with an entry whose name contains ".." to escape
        // the extraction root. Path::starts_with is structural and does
        // not catch this on its own — the component-level check must.
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("../evil.xhtml", deflated).unwrap();
        zip.write_all(b"<html/>").unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let tmp = epub_tempfile(&bytes);
        let result = extract_epub(tmp.path());
        assert!(result.is_err(), "path traversal must be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("EPUB_UNSAFE_PATH:"),
            "expected EPUB_UNSAFE_PATH error, got: {msg}"
        );
    }

    #[test]
    fn extract_epub_rejects_directory_traversal() {
        // Directory entries were created before the traversal check ran.
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("pwned");
        let name = format!("../../../../../../../..{}/", target.display());

        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.add_directory(name, stored).unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let tmp = epub_tempfile(&bytes);
        let msg = extract_epub(tmp.path()).unwrap_err();
        assert!(msg.starts_with("EPUB_UNSAFE_PATH:"), "got: {msg}");
        assert!(
            !target.exists(),
            "directory was created outside the temp dir"
        );
    }

    #[test]
    fn extract_epub_rejects_absolute_path() {
        // An absolute path like /etc/passwd must also be rejected.
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("/etc/evil.xhtml", deflated).unwrap();
        zip.write_all(b"<html/>").unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let tmp = epub_tempfile(&bytes);
        let result = extract_epub(tmp.path());
        assert!(result.is_err(), "absolute path must be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("EPUB_UNSAFE_PATH:"),
            "expected EPUB_UNSAFE_PATH error, got: {msg}"
        );
    }

    #[test]
    fn extract_epub_rejects_cumulative_size_overflow() {
        // Build an EPUB with many medium-sized entries whose combined size
        // exceeds MAX_TOTAL_BYTES but whose individual sizes stay below
        // MAX_ENTRY_BYTES. 6 × 90 MiB = 540 MiB > 500 MiB limit.
        let medium: Vec<u8> = vec![b'B'; 90 * 1024 * 1024];
        let extras: Vec<(String, Vec<u8>)> = (0..6)
            .map(|i| (format!("OEBPS/ch{i}.bin"), medium.clone()))
            .collect();
        let extras_ref: Vec<(&str, &[u8])> = extras
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_slice()))
            .collect();
        let bytes = build_epub_bytes(&extras_ref);
        let tmp = epub_tempfile(&bytes);

        let result = extract_epub(tmp.path());
        assert!(result.is_err(), "cumulative overflow must be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg == "EPUB_TOTAL_TOO_LARGE" || msg.starts_with("EPUB_ENTRY_TOO_LARGE:"),
            "expected size-limit error, got: {msg}"
        );
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

        assert!(!archive.is_empty(), "repacked ZIP must not be empty");

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
