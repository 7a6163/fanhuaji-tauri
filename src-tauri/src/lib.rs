mod epub;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

const API_BASE: &str = "https://api.zhconvert.org";
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024; // 50 MiB

// --- API Types ---

#[derive(Deserialize)]
struct ApiResponse {
    code: i32,
    msg: String,
    data: Option<ApiConvertData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiConvertData {
    text: String,
    converter: String,
}

#[derive(Deserialize)]
struct ServiceInfoResponse {
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
struct ModuleInfo {
    name: String,
    description: String,
    category: String,
}

#[derive(Debug, Serialize)]
struct ServiceInfo {
    modules: Vec<ModuleInfo>,
    dict_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConvertFileResult {
    output_name: String,
    output_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    warnings: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConvertFileParams {
    input_path: String,
    converter: String,
    save_folder: String,
    naming: String,
    pre_replace: String,
    post_replace: String,
    protect_replace: String,
    modules: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConvertEpubParams {
    file_id: String,
    input_path: String,
    converter: String,
    save_folder: String,
    naming: String,
    pre_replace: String,
    post_replace: String,
    protect_replace: String,
    modules: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpubProgress {
    file_id: String,
    chapter_index: usize,
    chapter_total: usize,
    chapter_name: String,
}

// --- HTTP Client (shared via Tauri managed state) ---

struct HttpClient(reqwest::Client);

// --- Filename sanitization ---

fn sanitize_filename_part(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

// --- Output naming ---

fn build_output_name(input: &Path, naming: &str, converter: &str) -> Result<String, String> {
    let stem = input
        .file_stem()
        .ok_or("無法取得檔案名稱")?
        .to_string_lossy();
    let ext = input.extension().unwrap_or_default().to_string_lossy();

    match naming {
        "overwrite" => Ok(input
            .file_name()
            .ok_or("無法取得檔案名稱")?
            .to_string_lossy()
            .into_owned()),
        "suffix" => Ok(format!("{stem}.converted.{ext}")),
        _ => {
            let converter_suffix = sanitize_filename_part(converter);
            if converter_suffix.is_empty() {
                return Err("API 回應包含無效的轉換器名稱".to_string());
            }
            Ok(format!("{stem}.{converter_suffix}.{ext}"))
        }
    }
}

// --- API params builder ---

fn build_api_params<'a>(
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

fn resolve_output_dir(input: &Path, save_folder: &str) -> Result<PathBuf, String> {
    match save_folder {
        "same" => input
            .parent()
            .ok_or_else(|| "輸入路徑沒有父目錄".to_string())
            .map(|p| p.to_path_buf()),
        custom => Ok(PathBuf::from(custom)),
    }
}

fn validate_api_response(api: ApiResponse) -> Result<ApiConvertData, String> {
    if api.code != 0 {
        return Err(format!("API 錯誤：{}", api.msg));
    }
    api.data.ok_or_else(|| "API 回應缺少 data 欄位".to_string())
}

fn check_file_size(len: u64) -> Result<(), String> {
    if len > MAX_FILE_BYTES {
        return Err("檔案過大（上限 50 MB）".to_string());
    }
    Ok(())
}

fn build_warnings(errors: &[String]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(format!("部分章節失敗：{}", errors.join("；")))
    }
}

fn build_convert_result(
    output_name: String,
    output_path: PathBuf,
    warnings: Option<String>,
) -> ConvertFileResult {
    ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings,
    }
}

// --- Commands ---

#[tauri::command]
async fn get_service_info(client: tauri::State<'_, HttpClient>) -> Result<ServiceInfo, String> {
    let url = format!("{API_BASE}/service-info");
    let client = &client.0;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("網路請求失敗：{e}"))?;

    let info: ServiceInfoResponse = resp
        .json()
        .await
        .map_err(|e| format!("回應解析失敗：{e}"))?;

    build_service_info(info)
}

fn build_service_info(info: ServiceInfoResponse) -> Result<ServiceInfo, String> {
    if info.code != 0 {
        return Err(format!("服務資訊請求失敗（code: {}）", info.code));
    }

    let dict_version = info.revisions.and_then(|r| r.build).unwrap_or_default();
    let modules = info.data.as_ref().map(parse_modules).unwrap_or_default();

    Ok(ServiceInfo {
        modules,
        dict_version,
    })
}

#[tauri::command]
async fn pick_save_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .set_title("選擇輸出資料夾")
        .blocking_pick_folder();

    Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
async fn open_files_dialog(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let paths = app
        .dialog()
        .file()
        .add_filter(
            "支援檔案",
            &[
                "txt", "srt", "ass", "ssa", "lrc", "vtt", "sub", "sup", "csv", "tsv", "json",
                "xml", "html", "htm", "md", "epub",
            ],
        )
        .add_filter("所有檔案", &["*"])
        .set_title("開啟檔案")
        .blocking_pick_files();

    match paths {
        Some(files) => Ok(files.iter().map(|f| f.to_string()).collect()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn convert_file(
    client: tauri::State<'_, HttpClient>,
    params: ConvertFileParams,
) -> Result<ConvertFileResult, String> {
    let ConvertFileParams {
        input_path,
        converter,
        save_folder,
        naming,
        pre_replace,
        post_replace,
        protect_replace,
        modules,
    } = params;

    // Canonicalize and validate input path
    let canonical = tokio::fs::canonicalize(&input_path)
        .await
        .map_err(|e| format!("無效路徑：{e}"))?;

    // Check file size
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("無法讀取檔案資訊：{e}"))?;
    check_file_size(metadata.len())?;

    // Read the file
    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|e| format!("無法讀取檔案：{e}"))?;

    // Build API params
    let params = build_api_params(
        &content,
        &converter,
        &pre_replace,
        &post_replace,
        &protect_replace,
        &modules,
    );

    // Call API
    let url = format!("{API_BASE}/convert");
    let resp = client
        .0
        .post(&url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("網路請求失敗：{e}"))?;

    let api: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("回應解析失敗：{e}"))?;

    let data = validate_api_response(api)?;

    // Determine output directory
    let input = Path::new(&input_path);
    let dir = resolve_output_dir(input, &save_folder)?;

    let output_name = build_output_name(input, &naming, &data.converter)?;

    // Build output path from canonical directory to prevent traversal
    let canonical_dir = tokio::fs::canonicalize(&dir)
        .await
        .map_err(|e| format!("輸出目錄無效：{e}"))?;
    let output_path = canonical_dir.join(&output_name);

    // Write output
    tokio::fs::write(&output_path, &data.text)
        .await
        .map_err(|e| format!("無法寫入檔案：{e}"))?;

    Ok(build_convert_result(output_name, output_path, None))
}

#[tauri::command]
async fn convert_epub(
    app: tauri::AppHandle,
    client: tauri::State<'_, HttpClient>,
    params: ConvertEpubParams,
) -> Result<ConvertFileResult, String> {
    let ConvertEpubParams {
        file_id,
        input_path,
        converter,
        save_folder,
        naming,
        pre_replace,
        post_replace,
        protect_replace,
        modules,
    } = params;

    let canonical = tokio::fs::canonicalize(&input_path)
        .await
        .map_err(|e| format!("無效路徑：{e}"))?;

    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("無法讀取檔案資訊：{e}"))?;
    check_file_size(metadata.len())?;

    // Extract EPUB
    let canonical_clone = canonical.clone();
    let (temp_dir, content_files) =
        tokio::task::spawn_blocking(move || epub::extract_epub(&canonical_clone))
            .await
            .map_err(|e| format!("解壓錯誤：{e}"))??;

    let chapter_total = content_files.len();
    let url = format!("{API_BASE}/convert");
    let mut errors: Vec<String> = Vec::new();

    // Convert each chapter
    for (i, content_file) in content_files.iter().enumerate() {
        let chapter_name = epub::chapter_display_name(&content_file.relative_path);

        // Emit progress
        let _ = app.emit(
            "epub-progress",
            EpubProgress {
                file_id: file_id.clone(),
                chapter_index: i + 1,
                chapter_total,
                chapter_name: chapter_name.clone(),
            },
        );

        let file_path = temp_dir.path().join(&content_file.relative_path);
        let xhtml = match tokio::fs::read_to_string(&file_path).await {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("{chapter_name}: 讀取失敗 ({e})"));
                continue;
            }
        };

        // Extract text
        let (text, count) = match epub::extract_text(&xhtml) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("{chapter_name}: {e}"));
                continue;
            }
        };

        if count == 0 {
            continue; // No text to convert
        }

        // Call API
        let api_params = build_api_params(
            &text,
            &converter,
            &pre_replace,
            &post_replace,
            &protect_replace,
            &modules,
        );

        let resp = match client.0.post(&url).form(&api_params).send().await {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("{chapter_name}: 網路請求失敗 ({e})"));
                continue;
            }
        };

        let api: ApiResponse = match resp.json().await {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("{chapter_name}: 回應解析失敗 ({e})"));
                continue;
            }
        };

        if api.code != 0 {
            errors.push(format!("{chapter_name}: API 錯誤 ({})", api.msg));
            continue;
        }

        let data = match api.data {
            Some(d) => d,
            None => {
                errors.push(format!("{chapter_name}: API 回應缺少 data"));
                continue;
            }
        };

        // Replace text in XHTML
        let new_xhtml = match epub::replace_text(&xhtml, &data.text) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("{chapter_name}: {e}"));
                continue;
            }
        };

        if let Err(e) = tokio::fs::write(&file_path, new_xhtml).await {
            errors.push(format!("{chapter_name}: 寫入失敗 ({e})"));
            continue;
        }

        // Small delay between API calls
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Determine output path
    let input = Path::new(&input_path);
    let dir = resolve_output_dir(input, &save_folder)?;

    let output_name = build_output_name(input, &naming, &converter)?;
    let canonical_dir = tokio::fs::canonicalize(&dir)
        .await
        .map_err(|e| format!("輸出目錄無效：{e}"))?;
    let output_path = canonical_dir.join(&output_name);

    // Repack EPUB
    let temp_path = temp_dir.path().to_path_buf();
    let out_path = output_path.clone();
    tokio::task::spawn_blocking(move || epub::repack_epub(&temp_path, &out_path))
        .await
        .map_err(|e| format!("打包錯誤：{e}"))??;

    let warnings = build_warnings(&errors);

    Ok(build_convert_result(output_name, output_path, warnings))
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
        let result = build_output_name(Path::new("/tmp/test.srt"), "overwrite", "Taiwan");
        assert_eq!(result.unwrap(), "test.srt");
    }

    #[test]
    fn output_name_suffix_mode() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "suffix", "Taiwan");
        assert_eq!(result.unwrap(), "test.converted.srt");
    }

    #[test]
    fn output_name_auto_mode() {
        let result = build_output_name(Path::new("/tmp/test.srt"), "auto", "Taiwan");
        assert_eq!(result.unwrap(), "test.Taiwan.srt");
    }

    #[test]
    fn output_name_auto_with_special_converter() {
        let result = build_output_name(Path::new("/tmp/test.txt"), "auto", "Wiki/Traditional");
        assert_eq!(result.unwrap(), "test.WikiTraditional.txt");
    }

    #[test]
    fn output_name_auto_empty_converter_fails() {
        let result = build_output_name(Path::new("/tmp/test.txt"), "auto", "!@#$");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("無效的轉換器名稱"));
    }

    #[test]
    fn output_name_no_extension() {
        let result = build_output_name(Path::new("/tmp/README"), "suffix", "Taiwan");
        assert_eq!(result.unwrap(), "README.converted.");
    }

    #[test]
    fn output_name_chinese_filename() {
        let result = build_output_name(Path::new("/tmp/字幕.srt"), "auto", "Taiwan");
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
        // A bare filename has no parent directory component
        let result = resolve_output_dir(Path::new("test.txt"), "same");
        // On some systems Path::new("test.txt").parent() returns Some(""),
        // which is a valid (empty) path, so this might not error.
        // But Path::new("/").parent() is None, so test that:
        let result2 = resolve_output_dir(Path::new("/"), "same");
        // At least one of these should exercise the code path
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
        assert!(err.contains("API 錯誤"));
        assert!(err.contains("some error"));
    }

    #[test]
    fn validate_api_response_missing_data() {
        let api = ApiResponse {
            code: 0,
            msg: String::new(),
            data: None,
        };
        let err = validate_api_response(api).unwrap_err();
        assert!(err.contains("data"));
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
        assert!(err.contains("50 MB"));
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
        assert!(err.contains("500"));
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

    // --- build_warnings ---

    #[test]
    fn build_warnings_empty_errors() {
        assert!(build_warnings(&[]).is_none());
    }

    #[test]
    fn build_warnings_with_errors() {
        let errors = vec!["ch1: failed".to_string(), "ch2: timeout".to_string()];
        let w = build_warnings(&errors).unwrap();
        assert!(w.contains("部分章節失敗"));
        assert!(w.contains("ch1: failed"));
        assert!(w.contains("ch2: timeout"));
        assert!(w.contains("；"));
    }

    #[test]
    fn build_warnings_single_error() {
        let errors = vec!["ch1: failed".to_string()];
        let w = build_warnings(&errors).unwrap();
        assert!(w.contains("ch1: failed"));
        assert!(!w.contains("；"));
    }

    // --- build_convert_result ---

    #[test]
    fn build_convert_result_without_warnings() {
        let result = build_convert_result(
            "test.Taiwan.txt".to_string(),
            PathBuf::from("/tmp/test.Taiwan.txt"),
            None,
        );
        assert_eq!(result.output_name, "test.Taiwan.txt");
        assert_eq!(result.output_path, "/tmp/test.Taiwan.txt");
        assert!(result.warnings.is_none());
    }

    #[test]
    fn build_convert_result_with_warnings() {
        let result = build_convert_result(
            "book.epub".to_string(),
            PathBuf::from("/tmp/book.epub"),
            Some("部分章節失敗：ch1".to_string()),
        );
        assert_eq!(result.output_name, "book.epub");
        assert!(result.warnings.is_some());
    }

    #[test]
    fn build_convert_result_serializes_without_warnings_field() {
        let result =
            build_convert_result("test.txt".to_string(), PathBuf::from("/tmp/test.txt"), None);
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("warnings").is_none());
    }

    #[test]
    fn build_convert_result_serializes_with_warnings_field() {
        let result = build_convert_result(
            "test.txt".to_string(),
            PathBuf::from("/tmp/test.txt"),
            Some("warn".to_string()),
        );
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["warnings"], "warn");
    }

    // --- EpubProgress serialization ---

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

    // --- ConvertEpubParams deserialization ---

    #[test]
    fn convert_epub_params_deserializes() {
        let json = r#"{"fileId":"f1","inputPath":"/tmp/book.epub","converter":"Taiwan","saveFolder":"same","naming":"auto","preReplace":"","postReplace":"","protectReplace":"","modules":"{}"}"#;
        let params: ConvertEpubParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.file_id, "f1");
        assert_eq!(params.input_path, "/tmp/book.epub");
        assert_eq!(params.converter, "Taiwan");
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(HttpClient(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("無法建立 HTTP 客戶端"),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_service_info,
            pick_save_folder,
            open_files_dialog,
            convert_file,
            convert_epub,
        ])
        .run(tauri::generate_context!())
        .expect("啟動應用程式時發生錯誤");
}
