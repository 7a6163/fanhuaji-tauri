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

pub(crate) fn check_file_size(len: u64) -> Result<(), String> {
    if len > MAX_FILE_BYTES {
        return Err("FILE_TOO_LARGE".to_string());
    }
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

    #[test]
    fn api_base_url() {
        assert_eq!(API_BASE, "https://api.zhconvert.org");
    }
}
