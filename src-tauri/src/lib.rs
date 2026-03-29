use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

#[derive(Deserialize)]
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

#[derive(Serialize)]
struct ModuleInfo {
    name: String,
    description: String,
    category: String,
}

#[derive(Serialize)]
struct ServiceInfo {
    modules: Vec<ModuleInfo>,
    dict_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConvertFileResult {
    output_name: String,
    output_path: String,
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

// --- HTTP Client ---

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("無法建立 HTTP 客戶端：{e}"))
}

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
            .to_string()),
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
    if let Some(mods) = &data.modules {
        if let Some(obj) = mods.as_object() {
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
    }
    modules
}

// --- Commands ---

#[tauri::command]
async fn get_service_info() -> Result<ServiceInfo, String> {
    let url = format!("{API_BASE}/service-info");
    let client = http_client()?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("網路請求失敗：{e}"))?;

    let info: ServiceInfoResponse = resp
        .json()
        .await
        .map_err(|e| format!("回應解析失敗：{e}"))?;

    if info.code != 0 {
        return Err(format!("服務資訊請求失敗（code: {}）", info.code));
    }

    let dict_version = info.revisions.and_then(|r| r.build).unwrap_or_default();

    let mut modules = Vec::new();

    if let Some(data) = &info.data {
        modules = parse_modules(data);
    }

    Ok(ServiceInfo {
        modules,
        dict_version,
    })
}

#[tauri::command]
async fn open_files_dialog(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let paths = app
        .dialog()
        .file()
        .add_filter(
            "文字檔案",
            &[
                "txt", "srt", "ass", "ssa", "lrc", "vtt", "sub", "sup", "csv", "tsv", "json",
                "xml", "html", "htm", "md",
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
async fn convert_file(params: ConvertFileParams) -> Result<ConvertFileResult, String> {
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
    if metadata.len() > MAX_FILE_BYTES {
        return Err("檔案過大（上限 50 MB）".to_string());
    }

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
    let client = http_client()?;
    let url = format!("{API_BASE}/convert");
    let resp = client
        .post(&url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("網路請求失敗：{e}"))?;

    let api: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("回應解析失敗：{e}"))?;

    if api.code != 0 {
        return Err(format!("API 錯誤：{}", api.msg));
    }

    let data = api.data.ok_or("API 回應缺少 data 欄位")?;

    // Determine output directory
    let input = Path::new(&input_path);
    let dir_buf: PathBuf;
    let dir: &Path = match save_folder.as_str() {
        "same" => input.parent().unwrap_or(Path::new(".")),
        custom => {
            dir_buf = PathBuf::from(custom);
            &dir_buf
        }
    };

    let output_name = build_output_name(input, &naming, &data.converter)?;

    let output_path = dir.join(&output_name);

    // Validate output path stays within intended directory
    let canonical_dir = tokio::fs::canonicalize(dir)
        .await
        .map_err(|e| format!("輸出目錄無效：{e}"))?;
    let canonical_out_parent = output_path.parent().ok_or("無法取得輸出目錄")?;
    let canonical_out_parent = tokio::fs::canonicalize(canonical_out_parent)
        .await
        .unwrap_or_else(|_| canonical_out_parent.to_path_buf());
    if !canonical_out_parent.starts_with(&canonical_dir) {
        return Err("輸出路徑超出預期目錄".to_string());
    }

    // Write output
    tokio::fs::write(&output_path, &data.text)
        .await
        .map_err(|e| format!("無法寫入檔案：{e}"))?;

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().to_string(),
    })
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

    // --- http_client ---

    #[test]
    fn http_client_creates_successfully() {
        assert!(http_client().is_ok());
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
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["outputName"], "test.Taiwan.txt");
        assert_eq!(json["outputPath"], "/tmp/test.Taiwan.txt");
        assert!(json.get("output_name").is_none());
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
            open_files_dialog,
            convert_file,
        ])
        .run(tauri::generate_context!())
        .expect("啟動應用程式時發生錯誤");
}
