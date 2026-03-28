use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tauri_plugin_dialog::DialogExt;

const API_BASE: &str = "https://api.zhconvert.org";

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
    used_modules: Vec<String>,
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
struct ConvertFileResult {
    output_name: String,
    output_path: String,
}

// --- Commands ---

#[tauri::command]
async fn get_service_info() -> Result<ServiceInfo, String> {
    let url = format!("{API_BASE}/service-info");
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("網路請求失敗：{e}"))?;

    let info: ServiceInfoResponse = resp
        .json()
        .await
        .map_err(|e| format!("回應解析失敗：{e}"))?;

    let dict_version = info
        .revisions
        .and_then(|r| r.build)
        .unwrap_or_default();

    let mut modules = Vec::new();

    if let Some(data) = &info.data {
        // Parse module categories
        let categories: HashMap<String, Vec<String>> = data
            .module_categories
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Build category lookup: module_name -> category_name
        let mut module_to_category: HashMap<String, String> = HashMap::new();
        for (cat_name, cat_modules) in &categories {
            for m in cat_modules {
                module_to_category.insert(m.clone(), cat_name.clone());
            }
        }

        // Parse modules
        if let Some(mods) = &data.modules {
            if let Some(obj) = mods.as_object() {
                for (name, val) in obj {
                    let desc = val
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let category = module_to_category
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| "未知".to_string());
                    modules.push(ModuleInfo {
                        name: name.clone(),
                        description: desc,
                        category,
                    });
                }
            }
        }
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
            &["txt", "srt", "ass", "ssa", "lrc", "vtt", "sub", "sup", "csv", "tsv", "json", "xml", "html", "htm", "md"],
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
    input_path: String,
    converter: String,
    save_folder: String,
    naming: String,
    pre_replace: String,
    post_replace: String,
    protect_replace: String,
    modules: String,
) -> Result<ConvertFileResult, String> {
    // Read the file
    let content = tokio::fs::read_to_string(&input_path)
        .await
        .map_err(|e| format!("無法讀取檔案：{e}"))?;

    // Build API params
    let mut params: Vec<(&str, String)> = vec![
        ("text", content),
        ("converter", converter.clone()),
    ];

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

    // Call API
    let client = reqwest::Client::new();
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

    // Determine output path
    let input = Path::new(&input_path);
    let dir = match save_folder.as_str() {
        "same" | _ => input.parent().unwrap_or(Path::new(".")),
    };

    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let ext = input.extension().unwrap_or_default().to_string_lossy();

    let output_name = match naming.as_str() {
        "overwrite" => input.file_name().unwrap_or_default().to_string_lossy().to_string(),
        "suffix" => format!("{stem}.converted.{ext}"),
        _ => {
            // Auto naming: add converter name
            let converter_suffix = &data.converter;
            format!("{stem}.{converter_suffix}.{ext}")
        }
    };

    let output_path = dir.join(&output_name);

    // Write output
    tokio::fs::write(&output_path, &data.text)
        .await
        .map_err(|e| format!("無法寫入檔案：{e}"))?;

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().to_string(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_service_info,
            open_files_dialog,
            convert_file,
        ])
        .run(tauri::generate_context!())
        .expect("啟動應用程式時發生錯誤");
}
