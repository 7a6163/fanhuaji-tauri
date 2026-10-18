mod commands;
mod epub;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) const API_BASE: &str = "https://api.zhconvert.org";
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024; // 50 MiB

// --- API Types ---

#[derive(Deserialize)]
pub(crate) struct ApiResponse {
    code: i32,
    msg: String,
    data: Option<ApiConvertData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiConvertData {
    pub text: String,
    pub converter: String,
}

#[derive(Deserialize)]
pub(crate) struct ServiceInfoResponse {
    code: i32,
    data: Option<ServiceInfoData>,
    revisions: Option<Revisions>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceInfoData {
    modules: Option<serde_json::Value>,
    module_categories: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Revisions {
    build: Option<String>,
}

// --- Return Types ---

#[derive(Debug, Serialize)]
pub(crate) struct ModuleInfo {
    key: String,
    name: String,
    description: String,
    category: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServiceInfo {
    modules: Vec<ModuleInfo>,
    dict_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConvertFileResult {
    pub output_name: String,
    pub output_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConvertFileParams {
    #[serde(default)]
    pub file_id: String,
    pub input_path: String,
    pub converter: String,
    pub save_folder: String,
    pub naming: String,
    #[serde(default)]
    pub custom_suffix: String,
    pub pre_replace: String,
    pub post_replace: String,
    pub protect_replace: String,
    pub modules: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConvertEpubParams {
    pub file_id: String,
    pub input_path: String,
    pub converter: String,
    pub save_folder: String,
    pub naming: String,
    #[serde(default)]
    pub custom_suffix: String,
    pub pre_replace: String,
    pub post_replace: String,
    pub protect_replace: String,
    pub modules: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EpubProgress {
    pub file_id: String,
    pub chapter_index: usize,
    pub chapter_total: usize,
    pub chapter_name: String,
}

// --- HTTP Client (shared via Tauri managed state) ---

pub(crate) struct HttpClient(pub reqwest::Client);

// --- Filename sanitization ---

fn sanitize_filename_part(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

// --- Output naming ---

pub(crate) fn build_output_name(
    input: &Path,
    naming: &str,
    converter: &str,
    custom_suffix: &str,
) -> Result<String, String> {
    let stem = input
        .file_stem()
        .ok_or("FILENAME_UNAVAILABLE")?
        .to_string_lossy();
    let ext = input.extension().unwrap_or_default().to_string_lossy();

    let suffix = match naming {
        "overwrite" => {
            return Ok(input
                .file_name()
                .ok_or("FILENAME_UNAVAILABLE")?
                .to_string_lossy()
                .into_owned());
        }
        "suffix" => {
            let s = sanitize_filename_part(custom_suffix);
            if s.is_empty() {
                "converted".to_string()
            } else {
                s
            }
        }
        _ => {
            let s = sanitize_filename_part(converter);
            if s.is_empty() {
                return Err("API_INVALID_CONVERTER".to_string());
            }
            s
        }
    };

    if ext.is_empty() {
        Ok(format!("{stem}.{suffix}"))
    } else {
        Ok(format!("{stem}.{suffix}.{ext}"))
    }
}

/// Borrowed bundle of the user-tunable conversion options shared by the
/// file, preview and EPUB conversion paths.
#[derive(Clone, Copy)]
pub(crate) struct ConvertOptions<'a> {
    pub converter: &'a str,
    pub pre_replace: &'a str,
    pub post_replace: &'a str,
    pub protect_replace: &'a str,
    pub modules: &'a str,
}

// --- API params builder ---

pub(crate) fn build_api_params<'a>(
    text: &'a str,
    converter: &'a str,
    pre_replace: &'a str,
    post_replace: &'a str,
    protect_replace: &'a str,
    modules: &'a str,
) -> Vec<(&'a str, &'a str)> {
    let mut params = vec![("text", text), ("converter", converter)];
    if !pre_replace.is_empty() {
        params.push(("userPreReplace", pre_replace));
    }
    if !post_replace.is_empty() {
        params.push(("userPostReplace", post_replace));
    }
    if !protect_replace.is_empty() {
        params.push(("userProtectReplace", protect_replace));
    }
    if modules != "{}" && !modules.is_empty() {
        params.push(("modules", modules));
    }
    params
}

// --- Text chunking ---

/// Max source bytes per `/convert` request. The API's nginx front-end rejects
/// request bodies over 1 MiB; percent-encoding expands UTF-8 CJK bytes 3x, so
/// the source text has to stay well under 1 MiB / 3.
pub(crate) const MAX_CHUNK_BYTES: usize = 250 * 1024;

/// Split `text` into consecutive slices each at most `max_bytes` long.
///
/// Splits happen at `\n` boundaries where possible (the newline stays with the
/// preceding chunk, so `chunks.concat()` reproduces the input exactly). A single
/// line longer than `max_bytes` is broken at a UTF-8 char boundary. Empty input
/// yields an empty vec.
pub(crate) fn split_text(text: &str, max_bytes: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();

    while start < text.len() {
        let remaining = text.len() - start;
        if remaining <= max_bytes {
            chunks.push(&text[start..]);
            break;
        }

        let window_end = start + max_bytes;
        // Prefer breaking right after the last newline inside the window.
        let split = match bytes[start..window_end].iter().rposition(|&b| b == b'\n') {
            Some(rel) => start + rel + 1,
            None => {
                // No newline: fall back to the last char boundary <= window_end.
                let mut e = window_end;
                while e > start && !text.is_char_boundary(e) {
                    e -= 1;
                }
                // A single char wider than the window (shouldn't happen for
                // valid text, but stay safe): take at least one char.
                if e == start {
                    e = start + 1;
                    while e < text.len() && !text.is_char_boundary(e) {
                        e += 1;
                    }
                }
                e
            }
        };

        chunks.push(&text[start..split]);
        start = split;
    }

    chunks
}

// --- Module parsing ---

fn parse_modules(data: &ServiceInfoData) -> Vec<ModuleInfo> {
    let category_names: HashMap<String, String> = data
        .module_categories
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let mut modules = Vec::new();
    if let Some(mods) = &data.modules
        && let Some(obj) = mods.as_object()
    {
        for (key, val) in obj {
            let name = val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(key)
                .to_string();
            let desc = val
                .get("desc")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let cat_id = val.get("cat").and_then(|v| v.as_str()).unwrap_or("unknown");
            let category = category_names
                .get(cat_id)
                .cloned()
                .unwrap_or_else(|| "未知".to_string());
            modules.push(ModuleInfo {
                key: key.clone(),
                name,
                description: desc,
                category,
            });
        }
    }
    modules
}

// --- Pure helpers ---

pub(crate) fn resolve_output_dir(input: &Path, save_folder: &str) -> Result<PathBuf, String> {
    match save_folder {
        "same" => input
            .parent()
            .ok_or_else(|| "NO_PARENT_DIR".to_string())
            .map(|p| p.to_path_buf()),
        custom => Ok(PathBuf::from(custom)),
    }
}

pub(crate) fn validate_api_response(api: ApiResponse) -> Result<ApiConvertData, String> {
    if api.code != 0 {
        return Err(format!("API_ERROR:{}", api.msg));
    }
    api.data.ok_or_else(|| "API_NO_DATA".to_string())
}

/// Whether a conversion error means the API is unreachable or rejecting every
/// request, so converting further EPUB chapters would only fail the same way.
pub(crate) fn is_fatal_api_error(err: &str) -> bool {
    let code = err.split(':').next().unwrap_or_default();
    matches!(
        code,
        "NET_REQUEST_FAILED" | "HTTP_ERROR" | "PAYLOAD_TOO_LARGE" | "RESPONSE_PARSE_FAILED"
    )
}

/// Summarise per-chapter EPUB results: `None` when every chapter succeeded, an
/// `EPUB_PARTIAL_FAILED` warning when some did, or the last chapter error when
/// chapters failed and none converted (the output would be unconverted).
pub(crate) fn epub_warnings(
    failed: usize,
    converted: usize,
    total: usize,
    last_error: Option<String>,
) -> Result<Option<String>, String> {
    if failed == 0 {
        return Ok(None);
    }
    if converted == 0 {
        return Err(last_error.unwrap_or_else(|| format!("EPUB_PARTIAL_FAILED:{failed}/{total}")));
    }
    Ok(Some(format!("EPUB_PARTIAL_FAILED:{failed}/{total}")))
}

pub(crate) fn check_file_size(len: u64) -> Result<(), String> {
    if len > MAX_FILE_BYTES {
        return Err("FILE_TOO_LARGE".to_string());
    }
    Ok(())
}

/// How a text file was stored on disk, so the output can be written back the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextEncoding {
    encoding: &'static encoding_rs::Encoding,
    bom: bool,
}

/// Decode raw file bytes into a UTF-8 `String`, detecting the source encoding.
///
/// A leading BOM (UTF-8 / UTF-16) is honoured first; otherwise the charset is
/// guessed with `chardetng` (the detector Firefox uses), covering common
/// subtitle encodings such as Big5, GBK/GB18030, Shift_JIS and EUC. Returns the
/// decoded text along with how it was stored. Text containing NUL is rejected
/// as `NOT_TEXT_FILE`: it is a binary file (e.g. a `.sup` subtitle or a video).
pub(crate) fn decode_text(bytes: &[u8]) -> Result<(String, TextEncoding), String> {
    let (encoding, bom) = match encoding_rs::Encoding::for_bom(bytes) {
        Some((enc, _)) => (enc, true),
        None => {
            let mut detector = chardetng::EncodingDetector::new();
            detector.feed(bytes, true);
            (detector.guess(None, true), false)
        }
    };
    let (text, _, _) = encoding.decode(bytes);
    if text.contains('\0') {
        return Err("NOT_TEXT_FILE".to_string());
    }
    Ok((text.into_owned(), TextEncoding { encoding, bom }))
}

/// Encode converted text the way the source file was stored: same encoding,
/// with a BOM only if it had one. Falls back to UTF-8 with a BOM when the text
/// no longer fits a legacy encoding (e.g. simplified output into Big5), so no
/// characters are lost and players still detect the encoding.
pub(crate) fn encode_text(text: &str, source: TextEncoding) -> Vec<u8> {
    use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE};

    let enc = source.encoding;
    // encoding_rs can decode UTF-16 but not encode it.
    if enc == UTF_16LE || enc == UTF_16BE {
        let mut out = Vec::with_capacity(2 + text.len() * 2);
        for unit in std::iter::once(0xFEFF).chain(text.encode_utf16()) {
            let bytes = if enc == UTF_16LE {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            out.extend_from_slice(&bytes);
        }
        return out;
    }
    if enc != UTF_8 {
        let (bytes, _, had_errors) = enc.encode(text);
        if !had_errors {
            return bytes.into_owned();
        }
    }

    let mut out = Vec::with_capacity(3 + text.len());
    if source.bom || enc != UTF_8 {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    out.extend_from_slice(text.as_bytes());
    out
}

/// Write a file via a temp file in the same directory and a rename, so a
/// failure midway never leaves `path` truncated. This matters in overwrite
/// mode, where `path` is the input file. An existing file's permissions are kept.
pub(crate) fn write_atomic(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    let dir = path.parent().ok_or("NO_PARENT_DIR")?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("OUTPUT_CREATE_FAILED:{e}"))?;
    if let Ok(meta) = std::fs::metadata(path) {
        tmp.as_file()
            .set_permissions(meta.permissions())
            .map_err(|e| format!("FILE_WRITE_FAILED:{e}"))?;
    }
    write(tmp.as_file_mut())?;
    tmp.persist(path)
        .map_err(|e| format!("FILE_WRITE_FAILED:{}", e.error))?;
    Ok(())
}

pub(crate) fn build_service_info(info: ServiceInfoResponse) -> Result<ServiceInfo, String> {
    if info.code != 0 {
        return Err(format!("SERVICE_INFO_FAILED:{}", info.code));
    }

    let dict_version = info.revisions.and_then(|r| r.build).unwrap_or_default();
    let modules = info.data.as_ref().map(parse_modules).unwrap_or_default();

    Ok(ServiceInfo {
        modules,
        dict_version,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    commands::run();
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- sanitize_filename_part ---

    #[test]
    fn sanitize_keeps_alphanumeric() {
        assert_eq!(sanitize_filename_part("Taiwan"), "Taiwan");
    }

    #[test]
    fn sanitize_keeps_hyphens_and_underscores() {
        assert_eq!(sanitize_filename_part("my-file_name"), "my-file_name");
    }

    #[test]
    fn sanitize_removes_special_characters() {
        assert_eq!(sanitize_filename_part("a/b\\c:d"), "abcd");
    }

    #[test]
    fn sanitize_removes_spaces() {
        assert_eq!(sanitize_filename_part("hello world"), "helloworld");
    }

    #[test]
    fn sanitize_handles_empty_string() {
        assert_eq!(sanitize_filename_part(""), "");
    }

    #[test]
    fn sanitize_handles_chinese_characters() {
        assert_eq!(sanitize_filename_part("台灣化"), "台灣化");
    }

    #[test]
    fn sanitize_mixed_content() {
        assert_eq!(sanitize_filename_part("a!@#b$%^c"), "abc");
    }

    // --- decode_text ---

    #[test]
    fn decode_plain_utf8() {
        let (text, _) = decode_text("繁化姬".as_bytes()).unwrap();
        assert_eq!(text, "繁化姬");
    }

    #[test]
    fn decode_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("字幕".as_bytes());
        let (text, _) = decode_text(&bytes).unwrap();
        assert_eq!(text, "字幕");
    }

    #[test]
    fn decode_utf16le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "字幕".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let (text, _) = decode_text(&bytes).unwrap();
        assert_eq!(text, "字幕");
    }

    #[test]
    fn decode_big5() {
        let (bytes, _, had_errors) = encoding_rs::BIG5.encode("台灣繁體字幕測試內容夠長以利偵測");
        assert!(!had_errors);
        let (text, _) = decode_text(&bytes).unwrap();
        assert_eq!(text, "台灣繁體字幕測試內容夠長以利偵測");
    }

    #[test]
    fn decode_gbk() {
        let (bytes, _, had_errors) = encoding_rs::GBK.encode("简体中文字幕测试内容够长以利侦测");
        assert!(!had_errors);
        let (text, _) = decode_text(&bytes).unwrap();
        assert_eq!(text, "简体中文字幕测试内容够长以利侦测");
    }

    #[test]
    fn decode_rejects_binary() {
        let bytes = b"\x50\x47\x00\x00\x01\x2c\x00\x00";
        assert_eq!(decode_text(bytes), Err("NOT_TEXT_FILE".to_string()));
    }

    #[test]
    fn decode_utf16_with_bom_is_text() {
        // UTF-16 is full of NUL bytes but decodes to text without NUL.
        let bytes = encode_text(
            "ab",
            TextEncoding {
                encoding: encoding_rs::UTF_16LE,
                bom: true,
            },
        );
        assert_eq!(decode_text(&bytes).unwrap().0, "ab");
    }

    // --- encode_text ---

    fn round_trip(bytes: &[u8], converted: &str) -> Vec<u8> {
        let (_, source) = decode_text(bytes).unwrap();
        encode_text(converted, source)
    }

    #[test]
    fn encode_keeps_plain_utf8() {
        assert_eq!(round_trip("字幕".as_bytes(), "字幕"), "字幕".as_bytes());
    }

    #[test]
    fn encode_keeps_utf8_bom() {
        let input = b"\xEF\xBB\xBF\xE5\xAD\x97";
        assert_eq!(round_trip(input, "字"), input);
    }

    #[test]
    fn encode_keeps_utf16le() {
        let input = encode_text(
            "字幕",
            TextEncoding {
                encoding: encoding_rs::UTF_16LE,
                bom: true,
            },
        );
        assert_eq!(&input[..2], b"\xFF\xFE");
        assert_eq!(round_trip(&input, "字幕"), input);
    }

    #[test]
    fn encode_keeps_utf16be() {
        let input = encode_text(
            "字幕",
            TextEncoding {
                encoding: encoding_rs::UTF_16BE,
                bom: true,
            },
        );
        assert_eq!(&input[..2], b"\xFE\xFF");
        assert_eq!(round_trip(&input, "字幕"), input);
    }

    #[test]
    fn encode_keeps_big5() {
        let (input, _, _) = encoding_rs::BIG5.encode("台灣繁體字幕測試內容夠長以利偵測");
        let out = round_trip(&input, "臺灣繁體字幕測試內容夠長以利偵測");
        let (expected, _, _) = encoding_rs::BIG5.encode("臺灣繁體字幕測試內容夠長以利偵測");
        assert_eq!(out, expected.as_ref());
    }

    #[test]
    fn encode_falls_back_to_utf8_bom_when_unrepresentable() {
        let (input, _, _) = encoding_rs::BIG5.encode("台灣繁體字幕測試內容夠長以利偵測");
        // Simplified characters such as 体 and 测 are not in Big5.
        let out = round_trip(&input, "台湾繁体字幕测试内容够长以利侦测");
        let mut expected = b"\xEF\xBB\xBF".to_vec();
        expected.extend_from_slice("台湾繁体字幕测试内容够长以利侦测".as_bytes());
        assert_eq!(out, expected);
    }

    // --- write_atomic ---

    #[test]
    fn write_atomic_replaces_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.srt");
        std::fs::write(&path, "old").unwrap();
        write_atomic(&path, |f| {
            std::io::Write::write_all(f, b"new").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn write_atomic_failure_keeps_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.srt");
        std::fs::write(&path, "original").unwrap();
        let result = write_atomic(&path, |f| {
            std::io::Write::write_all(f, b"half").unwrap();
            Err("ZIP_WRITE_FAILED:disk full".to_string())
        });
        assert_eq!(result, Err("ZIP_WRITE_FAILED:disk full".to_string()));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        // The temp file is cleaned up.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.srt");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&path, |_| Ok(())).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }

    #[test]
    fn decode_empty_is_empty() {
        let (text, _) = decode_text(&[]).unwrap();
        assert_eq!(text, "");
    }

    // --- build_output_name ---

    #[test]
    fn output_name_overwrite_mode() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "overwrite", "Taiwan", "");
        assert_eq!(result.unwrap(), "test.srt");
    }

    #[test]
    fn output_name_suffix_default() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "suffix", "Taiwan", "");
        assert_eq!(result.unwrap(), "test.converted.srt");
    }

    #[test]
    fn output_name_suffix_custom() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "suffix", "Taiwan", "zh-tw");
        assert_eq!(result.unwrap(), "test.zh-tw.srt");
    }

    #[test]
    fn output_name_suffix_special_chars_sanitized() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "suffix", "Taiwan", "a/b:c");
        assert_eq!(result.unwrap(), "test.abc.srt");
    }

    #[test]
    fn output_name_auto_mode() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "auto", "Taiwan", "");
        assert_eq!(result.unwrap(), "test.Taiwan.srt");
    }

    #[test]
    fn output_name_auto_with_special_converter() {
        let result = build_output_name(Path::new("/tmp/test.txt"), "auto", "Wiki/Traditional", "");
        assert_eq!(result.unwrap(), "test.WikiTraditional.txt");
    }

    #[test]
    fn output_name_auto_empty_converter_fails() {
        let result = build_output_name(Path::new("/tmp/test.txt"), "auto", "!@#$", "");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "API_INVALID_CONVERTER");
    }

    #[test]
    fn output_name_no_extension_suffix() {
        let result = build_output_name(Path::new("/tmp/README"), "suffix", "Taiwan", "");
        assert_eq!(result.unwrap(), "README.converted");
    }

    #[test]
    fn output_name_no_extension_auto() {
        let result = build_output_name(Path::new("/tmp/README"), "auto", "Taiwan", "");
        assert_eq!(result.unwrap(), "README.Taiwan");
    }

    #[test]
    fn output_name_chinese_filename() {
        let result = build_output_name(Path::new("/tmp/字幕.srt"), "auto", "Taiwan", "");
        assert_eq!(result.unwrap(), "字幕.Taiwan.srt");
    }

    // --- build_api_params ---

    #[test]
    fn api_params_basic() {
        let params = build_api_params("hello", "Taiwan", "", "", "", "{}");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("text", "hello"));
        assert_eq!(params[1], ("converter", "Taiwan"));
    }

    #[test]
    fn api_params_with_pre_replace() {
        let params = build_api_params("hello", "Taiwan", "a=b", "", "", "{}");
        assert_eq!(params.len(), 3);
        assert_eq!(params[2], ("userPreReplace", "a=b"));
    }

    #[test]
    fn api_params_with_post_replace() {
        let params = build_api_params("hello", "Taiwan", "", "c=d", "", "{}");
        assert_eq!(params.len(), 3);
        assert_eq!(params[2], ("userPostReplace", "c=d"));
    }

    #[test]
    fn api_params_with_protect_replace() {
        let params = build_api_params("hello", "Taiwan", "", "", "word", "{}");
        assert_eq!(params.len(), 3);
        assert_eq!(params[2], ("userProtectReplace", "word"));
    }

    #[test]
    fn api_params_with_modules() {
        let params = build_api_params("hello", "Taiwan", "", "", "", r#"{"Naruto":1}"#);
        assert_eq!(params.len(), 3);
        assert_eq!(params[2], ("modules", r#"{"Naruto":1}"#));
    }

    #[test]
    fn api_params_empty_modules_skipped() {
        let params = build_api_params("hello", "Taiwan", "", "", "", "{}");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn api_params_empty_string_modules_skipped() {
        let params = build_api_params("hello", "Taiwan", "", "", "", "");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn api_params_all_options() {
        let params = build_api_params("text", "Simplified", "a=b", "c=d", "protect", r#"{"X":1}"#);
        assert_eq!(params.len(), 6);
    }

    // --- split_text ---

    #[test]
    fn split_text_empty_is_empty() {
        assert!(split_text("", 100).is_empty());
    }

    #[test]
    fn split_text_shorter_than_limit_is_single_chunk() {
        assert_eq!(split_text("hello world", 100), vec!["hello world"]);
    }

    #[test]
    fn split_text_breaks_at_newline_boundaries() {
        let text = "aaaa\nbbbb\ncccc\n";
        let chunks = split_text(text, 6);
        assert_eq!(chunks, vec!["aaaa\n", "bbbb\n", "cccc\n"]);
    }

    #[test]
    fn split_text_roundtrips() {
        let text = "第一行內容\n第二行內容\n第三行更長一些的內容\n第四行\n";
        let chunks = split_text(text, 20);
        assert_eq!(chunks.concat(), text);
        for c in &chunks {
            assert!(c.len() <= 20 || c.chars().count() == 1);
        }
    }

    #[test]
    fn split_text_never_splits_mid_char() {
        // Each CJK char is 3 bytes; a 7-byte window can't align to 3.
        let text = "字字字字字字字字";
        let chunks = split_text(text, 7);
        assert_eq!(chunks.concat(), text);
        for c in &chunks {
            assert!(c.is_char_boundary(0) && c.is_char_boundary(c.len()));
            assert!(std::str::from_utf8(c.as_bytes()).is_ok());
        }
    }

    #[test]
    fn split_text_oversized_single_line_falls_back_to_char_boundary() {
        let text = "這是一整行沒有換行符號的長內容需要被切開";
        let chunks = split_text(text, 9);
        assert_eq!(chunks.concat(), text);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn split_text_uses_real_chunk_size() {
        let line = "測試內容\n";
        let text = line.repeat(1000);
        let chunks = split_text(&text, MAX_CHUNK_BYTES);
        assert_eq!(chunks.concat(), text);
        assert_eq!(chunks.len(), 1);
    }

    // --- parse_modules ---

    #[test]
    fn parse_modules_with_categories() {
        let data = ServiceInfoData {
            modules: Some(serde_json::json!({
                "Naruto": {"name": "火影忍者", "desc": "日本動畫", "cat": "anime"},
                "Typo": {"name": "錯字修正", "desc": "修正常見錯字", "cat": "func"}
            })),
            module_categories: Some(serde_json::json!({
                "anime": "動畫",
                "func": "功能性"
            })),
        };
        let modules = parse_modules(&data);
        assert_eq!(modules.len(), 2);
        let naruto = modules.iter().find(|m| m.name == "火影忍者").unwrap();
        assert_eq!(naruto.key, "Naruto");
        assert_eq!(naruto.description, "日本動畫");
        assert_eq!(naruto.category, "動畫");
    }

    #[test]
    fn parse_modules_missing_category_defaults_to_unknown() {
        let data = ServiceInfoData {
            modules: Some(serde_json::json!({
                "Test": {"name": "Test", "desc": "desc", "cat": "nonexistent"}
            })),
            module_categories: Some(serde_json::json!({})),
        };
        let modules = parse_modules(&data);
        assert_eq!(modules[0].category, "未知");
    }

    #[test]
    fn parse_modules_missing_name_uses_key() {
        let data = ServiceInfoData {
            modules: Some(serde_json::json!({
                "MyModule": {"desc": "description"}
            })),
            module_categories: None,
        };
        let modules = parse_modules(&data);
        assert_eq!(modules[0].key, "MyModule");
        assert_eq!(modules[0].name, "MyModule");
    }

    #[test]
    fn parse_modules_empty() {
        let data = ServiceInfoData {
            modules: Some(serde_json::json!({})),
            module_categories: None,
        };
        assert!(parse_modules(&data).is_empty());
    }

    #[test]
    fn parse_modules_none() {
        let data = ServiceInfoData {
            modules: None,
            module_categories: None,
        };
        assert!(parse_modules(&data).is_empty());
    }

    // --- Deserialization ---

    #[test]
    fn api_response_deserializes_success() {
        let json = r#"{"code":0,"msg":"","data":{"text":"測試","converter":"Taiwan"}}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.text, "測試");
        assert_eq!(data.converter, "Taiwan");
    }

    #[test]
    fn api_response_deserializes_error() {
        let json = r#"{"code":1,"msg":"error occurred","data":null}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 1);
        assert_eq!(resp.msg, "error occurred");
        assert!(resp.data.is_none());
    }

    #[test]
    fn service_info_response_deserializes() {
        let json = r#"{"code":0,"data":{"modules":{},"moduleCategories":{}},"revisions":{"build":"dict-abc123-r100"}}"#;
        let resp: ServiceInfoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!(resp.data.is_some());
        assert_eq!(resp.revisions.unwrap().build.unwrap(), "dict-abc123-r100");
    }

    #[test]
    fn service_info_response_missing_revisions() {
        let json = r#"{"code":0,"data":null,"revisions":null}"#;
        let resp: ServiceInfoResponse = serde_json::from_str(json).unwrap();
        assert!(resp.revisions.is_none());
    }

    #[test]
    fn convert_file_params_deserializes() {
        let json = r#"{"inputPath":"/tmp/test.txt","converter":"Taiwan","saveFolder":"same","naming":"auto","preReplace":"","postReplace":"","protectReplace":"","modules":"{}"}"#;
        let params: ConvertFileParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.input_path, "/tmp/test.txt");
        assert_eq!(params.converter, "Taiwan");
        assert_eq!(params.save_folder, "same");
        assert_eq!(params.naming, "auto");
        assert!(params.pre_replace.is_empty());
        assert!(params.post_replace.is_empty());
        assert!(params.protect_replace.is_empty());
        assert_eq!(params.modules, "{}");
    }

    #[test]
    fn convert_file_result_serializes_camel_case() {
        let result = ConvertFileResult {
            output_name: "test.Taiwan.txt".to_string(),
            output_path: "/tmp/test.Taiwan.txt".to_string(),
            warnings: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["outputName"], "test.Taiwan.txt");
        assert_eq!(json["outputPath"], "/tmp/test.Taiwan.txt");
        assert!(json.get("output_name").is_none());
    }

    // --- resolve_output_dir ---

    #[test]
    fn resolve_output_dir_same_returns_parent() {
        let dir = resolve_output_dir(Path::new("/tmp/test.txt"), "same").unwrap();
        assert_eq!(dir, PathBuf::from("/tmp"));
    }

    #[test]
    fn resolve_output_dir_same_no_parent_errors() {
        let result = resolve_output_dir(Path::new("test.txt"), "same");
        let result2 = resolve_output_dir(Path::new("/"), "same");
        assert!(result.is_ok() || result.is_err());
        assert!(result2.is_ok() || result2.is_err());
    }

    #[test]
    fn resolve_output_dir_custom_path() {
        let dir = resolve_output_dir(Path::new("/tmp/test.txt"), "/output/dir").unwrap();
        assert_eq!(dir, PathBuf::from("/output/dir"));
    }

    // --- validate_api_response ---

    #[test]
    fn validate_api_response_success() {
        let api = ApiResponse {
            code: 0,
            msg: String::new(),
            data: Some(ApiConvertData {
                text: "converted".to_string(),
                converter: "Taiwan".to_string(),
            }),
        };
        let data = validate_api_response(api).unwrap();
        assert_eq!(data.text, "converted");
        assert_eq!(data.converter, "Taiwan");
    }

    #[test]
    fn validate_api_response_error_code() {
        let api = ApiResponse {
            code: 1,
            msg: "some error".to_string(),
            data: None,
        };
        let err = validate_api_response(api).unwrap_err();
        assert_eq!(err, "API_ERROR:some error");
    }

    #[test]
    fn validate_api_response_missing_data() {
        let api = ApiResponse {
            code: 0,
            msg: String::new(),
            data: None,
        };
        let err = validate_api_response(api).unwrap_err();
        assert_eq!(err, "API_NO_DATA");
    }

    // --- check_file_size ---

    #[test]
    fn check_file_size_within_limit() {
        assert!(check_file_size(1024).is_ok());
        assert!(check_file_size(MAX_FILE_BYTES).is_ok());
    }

    #[test]
    fn check_file_size_exceeds_limit() {
        let err = check_file_size(MAX_FILE_BYTES + 1).unwrap_err();
        assert_eq!(err, "FILE_TOO_LARGE");
    }

    // --- build_service_info ---

    #[test]
    fn build_service_info_success() {
        let info = ServiceInfoResponse {
            code: 0,
            data: Some(ServiceInfoData {
                modules: Some(serde_json::json!({
                    "Test": {"name": "TestMod", "desc": "A test", "cat": "func"}
                })),
                module_categories: Some(serde_json::json!({"func": "功能性"})),
            }),
            revisions: Some(Revisions {
                build: Some("dict-v1".to_string()),
            }),
        };
        let result = build_service_info(info).unwrap();
        assert_eq!(result.dict_version, "dict-v1");
        assert_eq!(result.modules.len(), 1);
        assert_eq!(result.modules[0].name, "TestMod");
    }

    #[test]
    fn build_service_info_error_code() {
        let info = ServiceInfoResponse {
            code: 500,
            data: None,
            revisions: None,
        };
        let err = build_service_info(info).unwrap_err();
        assert_eq!(err, "SERVICE_INFO_FAILED:500");
    }

    #[test]
    fn build_service_info_no_data_no_revisions() {
        let info = ServiceInfoResponse {
            code: 0,
            data: None,
            revisions: None,
        };
        let result = build_service_info(info).unwrap();
        assert!(result.modules.is_empty());
        assert!(result.dict_version.is_empty());
    }

    // --- Serialization ---

    #[test]
    fn epub_progress_serializes_camel_case() {
        let progress = EpubProgress {
            file_id: "abc".to_string(),
            chapter_index: 1,
            chapter_total: 10,
            chapter_name: "chapter1".to_string(),
        };
        let json = serde_json::to_value(&progress).unwrap();
        assert_eq!(json["fileId"], "abc");
        assert_eq!(json["chapterIndex"], 1);
        assert_eq!(json["chapterTotal"], 10);
        assert_eq!(json["chapterName"], "chapter1");
        assert!(json.get("file_id").is_none());
    }

    #[test]
    fn convert_epub_params_deserializes() {
        let json = r#"{"fileId":"f1","inputPath":"/tmp/book.epub","converter":"Taiwan","saveFolder":"same","naming":"auto","preReplace":"","postReplace":"","protectReplace":"","modules":"{}"}"#;
        let params: ConvertEpubParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.file_id, "f1");
        assert_eq!(params.input_path, "/tmp/book.epub");
        assert_eq!(params.converter, "Taiwan");
    }

    #[test]
    fn convert_file_result_skips_none_warnings() {
        let result = ConvertFileResult {
            output_name: "t.txt".to_string(),
            output_path: "/tmp/t.txt".to_string(),
            warnings: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("warnings").is_none());
    }

    #[test]
    fn convert_file_result_includes_warnings() {
        let result = ConvertFileResult {
            output_name: "t.txt".to_string(),
            output_path: "/tmp/t.txt".to_string(),
            warnings: Some("warn".to_string()),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["warnings"], "warn");
    }

    // --- Constants ---

    #[test]
    fn max_file_bytes_is_50mb() {
        assert_eq!(MAX_FILE_BYTES, 50 * 1024 * 1024);
    }

    // --- is_fatal_api_error ---

    #[test]
    fn fatal_api_errors_abort() {
        assert!(is_fatal_api_error("NET_REQUEST_FAILED:timed out"));
        assert!(is_fatal_api_error("HTTP_ERROR:502"));
        assert!(is_fatal_api_error("PAYLOAD_TOO_LARGE"));
        assert!(is_fatal_api_error("RESPONSE_PARSE_FAILED:eof"));
    }

    #[test]
    fn chapter_errors_are_not_fatal() {
        assert!(!is_fatal_api_error("XML_DECODE_FAILED:bad entity"));
        assert!(!is_fatal_api_error("API_ERROR:some error"));
        assert!(!is_fatal_api_error("HTTP_ERROR_LIKE:1"));
        assert!(!is_fatal_api_error(""));
    }

    // --- epub_warnings ---

    #[test]
    fn epub_warnings_none_when_all_succeed() {
        assert_eq!(epub_warnings(0, 5, 5, None), Ok(None));
        assert_eq!(epub_warnings(0, 0, 3, None), Ok(None));
    }

    #[test]
    fn epub_warnings_partial_failure() {
        let warn = epub_warnings(3, 37, 40, Some("XML_PARSE_FAILED:x".into()));
        assert_eq!(warn, Ok(Some("EPUB_PARTIAL_FAILED:3/40".to_string())));
    }

    #[test]
    fn epub_warnings_errors_when_nothing_converted() {
        let err = epub_warnings(2, 0, 4, Some("XML_DECODE_FAILED:x".into()));
        assert_eq!(err, Err("XML_DECODE_FAILED:x".to_string()));
    }

    #[test]
    fn api_base_url() {
        assert_eq!(API_BASE, "https://api.zhconvert.org");
    }
}
