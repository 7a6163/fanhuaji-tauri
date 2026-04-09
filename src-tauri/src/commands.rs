use crate::build_service_info;
use crate::epub;
use crate::{
    API_BASE, ApiResponse, ConvertEpubParams, ConvertFileParams, ConvertFileResult, EpubProgress,
    HttpClient, ServiceInfo, build_api_params, build_output_name, check_file_size,
    resolve_output_dir, validate_api_response,
};
use std::path::Path;
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn get_service_info(client: tauri::State<'_, HttpClient>) -> Result<ServiceInfo, String> {
    let url = format!("{API_BASE}/service-info");
    let client = &client.0;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("網路請求失敗：{e}"))?;

    let info = resp
        .json()
        .await
        .map_err(|e| format!("回應解析失敗：{e}"))?;

    build_service_info(info)
}

#[tauri::command]
pub async fn pick_save_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .set_title("選擇輸出資料夾")
        .blocking_pick_folder();

    Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
pub async fn open_files_dialog(app: tauri::AppHandle) -> Result<Vec<String>, String> {
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
pub async fn convert_file(
    client: tauri::State<'_, HttpClient>,
    params: ConvertFileParams,
) -> Result<ConvertFileResult, String> {
    let ConvertFileParams {
        input_path,
        converter,
        save_folder,
        naming,
        custom_suffix,
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

    let output_name = build_output_name(input, &naming, &data.converter, &custom_suffix)?;

    // Build output path from canonical directory to prevent traversal
    let canonical_dir = tokio::fs::canonicalize(&dir)
        .await
        .map_err(|e| format!("輸出目錄無效：{e}"))?;
    let output_path = canonical_dir.join(&output_name);

    // Write output
    tokio::fs::write(&output_path, &data.text)
        .await
        .map_err(|e| format!("無法寫入檔案：{e}"))?;

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings: None,
    })
}

#[tauri::command]
pub async fn convert_epub(
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
        custom_suffix,
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

    let output_name = build_output_name(input, &naming, &converter, &custom_suffix)?;
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

    let warnings = if errors.is_empty() {
        None
    } else {
        Some(format!("部分章節失敗：{}", errors.join("；")))
    };

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings,
    })
}

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
