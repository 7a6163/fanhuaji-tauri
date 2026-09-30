use crate::build_service_info;
use crate::epub;
use crate::{
    API_BASE, ApiConvertData, ApiResponse, ConvertEpubParams, ConvertFileParams, ConvertFileResult,
    ConvertOptions, EpubProgress, HttpClient, MAX_CHUNK_BYTES, ServiceInfo, build_api_params,
    build_output_name, check_file_size, decode_text, resolve_output_dir, split_text,
    validate_api_response,
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
        .map_err(|e| format!("NET_REQUEST_FAILED:{e}"))?;

    let info = resp
        .json()
        .await
        .map_err(|e| format!("RESPONSE_PARSE_FAILED:{e}"))?;

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

/// POST one text chunk to the `/convert` endpoint and return the converted data.
///
/// The HTTP status is checked before the body is parsed, so an nginx `413` page
/// or a `5xx` HTML error surfaces as `PAYLOAD_TOO_LARGE` / `HTTP_ERROR:{status}`
/// rather than a misleading `RESPONSE_PARSE_FAILED`.
async fn convert_chunk(
    client: &reqwest::Client,
    text: &str,
    opts: ConvertOptions<'_>,
) -> Result<ApiConvertData, String> {
    let params = build_api_params(
        text,
        opts.converter,
        opts.pre_replace,
        opts.post_replace,
        opts.protect_replace,
        opts.modules,
    );

    let url = format!("{API_BASE}/convert");
    let resp = client
        .post(&url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("NET_REQUEST_FAILED:{e}"))?;

    let status = resp.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            return Err("PAYLOAD_TOO_LARGE".to_string());
        }
        return Err(format!("HTTP_ERROR:{}", status.as_u16()));
    }

    let api: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("RESPONSE_PARSE_FAILED:{e}"))?;

    validate_api_response(api)
}

/// Convert `content` in newline-aligned chunks, concatenating the results.
///
/// Emits an `epub-progress` event per chunk when `file_id` is non-empty so the
/// UI can show progress on a large single file. Returns the joined converted
/// text and the converter name reported by the first chunk.
async fn convert_in_chunks(
    app: Option<&tauri::AppHandle>,
    client: &reqwest::Client,
    file_id: &str,
    content: &str,
    opts: ConvertOptions<'_>,
) -> Result<(String, String), String> {
    let chunks = split_text(content, MAX_CHUNK_BYTES);
    let total = chunks.len();
    let mut output = String::with_capacity(content.len());
    let mut result_converter = opts.converter.to_string();

    for (i, chunk) in chunks.iter().enumerate() {
        if let Some(app) = app
            && !file_id.is_empty()
        {
            let _ = app.emit(
                "epub-progress",
                EpubProgress {
                    file_id: file_id.to_string(),
                    chapter_index: i + 1,
                    chapter_total: total,
                    chapter_name: String::new(),
                },
            );
        }

        let data = convert_chunk(client, chunk, opts).await?;

        if i == 0 {
            result_converter = data.converter;
        }
        output.push_str(&data.text);

        if i + 1 < total {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    Ok((output, result_converter))
}

#[tauri::command]
pub async fn convert_file(
    app: tauri::AppHandle,
    client: tauri::State<'_, HttpClient>,
    params: ConvertFileParams,
) -> Result<ConvertFileResult, String> {
    let ConvertFileParams {
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

    // Canonicalize and validate input path
    let canonical = tokio::fs::canonicalize(&input_path)
        .await
        .map_err(|e| format!("INVALID_PATH:{e}"))?;

    // Check file size
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("FILE_METADATA_FAILED:{e}"))?;
    check_file_size(metadata.len())?;

    // Read the file as raw bytes and decode with charset detection so that
    // non-UTF-8 subtitle files (Big5, GBK, Shift_JIS, UTF-16, ...) are handled.
    let raw = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("FILE_READ_FAILED:{e}"))?;
    let (content, _encoding) = decode_text(&raw);

    // Convert in newline-aligned chunks (the API's front-end rejects request
    // bodies over ~1 MiB), streaming progress to the UI for large files.
    let (converted_text, result_converter) = convert_in_chunks(
        Some(&app),
        &client.0,
        &file_id,
        &content,
        ConvertOptions {
            converter: &converter,
            pre_replace: &pre_replace,
            post_replace: &post_replace,
            protect_replace: &protect_replace,
            modules: &modules,
        },
    )
    .await?;

    // Determine output directory
    let input = Path::new(&input_path);
    let dir = resolve_output_dir(input, &save_folder)?;

    let output_name = build_output_name(input, &naming, &result_converter, &custom_suffix)?;

    // Build output path from canonical directory to prevent traversal
    let canonical_dir = tokio::fs::canonicalize(&dir)
        .await
        .map_err(|e| format!("OUTPUT_DIR_INVALID:{e}"))?;
    let output_path = canonical_dir.join(&output_name);

    // Write output
    tokio::fs::write(&output_path, &converted_text)
        .await
        .map_err(|e| format!("FILE_WRITE_FAILED:{e}"))?;

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings: None,
    })
}

/// Cap on characters returned to the UI for a preview, to keep payloads small.
const PREVIEW_CHAR_LIMIT: usize = 8000;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    original: String,
    converted: String,
    truncated: bool,
}

/// Convert a file via the API and return the original + converted text (capped)
/// for an on-screen diff preview. Unlike `convert_file`, nothing is written to disk.
#[tauri::command]
pub async fn preview_convert(
    client: tauri::State<'_, HttpClient>,
    params: ConvertFileParams,
) -> Result<PreviewResult, String> {
    // Canonicalize, validate, and read the input — same guards as convert_file.
    let canonical = tokio::fs::canonicalize(&params.input_path)
        .await
        .map_err(|e| format!("INVALID_PATH:{e}"))?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("FILE_METADATA_FAILED:{e}"))?;
    check_file_size(metadata.len())?;
    let raw = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("FILE_READ_FAILED:{e}"))?;
    let (content, _encoding) = decode_text(&raw);

    // Only the preview window is sent to the API — a large file would be
    // rejected for body size and the extra text would just be discarded here.
    let truncated = content.chars().count() > PREVIEW_CHAR_LIMIT;
    let original: String = content.chars().take(PREVIEW_CHAR_LIMIT).collect();

    let data = convert_chunk(
        &client.0,
        &original,
        ConvertOptions {
            converter: &params.converter,
            pre_replace: &params.pre_replace,
            post_replace: &params.post_replace,
            protect_replace: &params.protect_replace,
            modules: &params.modules,
        },
    )
    .await?;

    let converted: String = data.text.chars().take(PREVIEW_CHAR_LIMIT).collect();

    Ok(PreviewResult {
        original,
        converted,
        truncated,
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
        .map_err(|e| format!("INVALID_PATH:{e}"))?;

    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("FILE_METADATA_FAILED:{e}"))?;
    check_file_size(metadata.len())?;

    // Extract EPUB
    let canonical_clone = canonical.clone();
    let (temp_dir, content_files) =
        tokio::task::spawn_blocking(move || epub::extract_epub(&canonical_clone))
            .await
            .map_err(|e| format!("EPUB_EXTRACT_FAILED:{e}"))??;

    let chapter_total = content_files.len();
    let mut failed_chapters: usize = 0;

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
            Err(_) => {
                failed_chapters += 1;
                continue;
            }
        };

        // Extract text
        let (text, count) = match epub::extract_text(&xhtml) {
            Ok(r) => r,
            Err(_) => {
                failed_chapters += 1;
                continue;
            }
        };

        if count == 0 {
            continue; // No text to convert
        }

        // Call API — chunked, so a chapter over the request-body limit still
        // converts instead of being counted as a failure.
        let converted = match convert_in_chunks(
            None,
            &client.0,
            "",
            &text,
            ConvertOptions {
                converter: &converter,
                pre_replace: &pre_replace,
                post_replace: &post_replace,
                protect_replace: &protect_replace,
                modules: &modules,
            },
        )
        .await
        {
            Ok((converted, _)) => converted,
            Err(_) => {
                failed_chapters += 1;
                continue;
            }
        };

        // Replace text in XHTML
        let new_xhtml = match epub::replace_text(&xhtml, &converted) {
            Ok(r) => r,
            Err(_) => {
                failed_chapters += 1;
                continue;
            }
        };

        if tokio::fs::write(&file_path, new_xhtml).await.is_err() {
            failed_chapters += 1;
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
        .map_err(|e| format!("OUTPUT_DIR_INVALID:{e}"))?;
    let output_path = canonical_dir.join(&output_name);

    // Repack EPUB
    let temp_path = temp_dir.path().to_path_buf();
    let out_path = output_path.clone();
    tokio::task::spawn_blocking(move || epub::repack_epub(&temp_path, &out_path))
        .await
        .map_err(|e| format!("EPUB_REPACK_FAILED:{e}"))??;

    let warnings = if failed_chapters == 0 {
        None
    } else {
        Some(format!(
            "EPUB_PARTIAL_FAILED:{failed_chapters}/{chapter_total}"
        ))
    };

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings,
    })
}

/// Returns `true` when this build is the Windows portable distribution, which
/// is detected by a `portable` marker file shipped next to the executable.
/// Portable builds cannot self-update in place (the Tauri Windows updater only
/// runs an installer), so the frontend uses this to switch to a notify-only flow.
#[tauri::command]
pub fn is_portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("portable").exists()))
        .unwrap_or(false)
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
            preview_convert,
            is_portable,
        ])
        .run(tauri::generate_context!())
        .expect("啟動應用程式時發生錯誤");
}
