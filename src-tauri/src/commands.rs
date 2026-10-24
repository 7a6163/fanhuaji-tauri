use crate::build_service_info;
use crate::epub;
use crate::{
    API_BASE, ApiConvertData, ApiResponse, Backups, ConvertFileResult, ConvertOptions,
    ConvertParams, EpubProgress, HttpClient, PREVIEW_CHAR_LIMIT, PreviewResult, ServiceInfo,
    build_api_params, build_output_name, build_preview, check_file_size, chunk_budget, decode_text,
    encode_text, epub_warnings, is_fatal_api_error, resolve_output_dir, split_text,
    trim_partial_utf8, truncate_chars, validate_api_response, write_atomic,
};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};
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
pub async fn pick_save_folder(
    app: tauri::AppHandle,
    title: String,
) -> Result<Option<String>, String> {
    let path = app.dialog().file().set_title(title).blocking_pick_folder();

    Ok(path.map(|p| p.to_string()))
}

/// Dialog strings come from the frontend so they follow the UI language.
#[tauri::command]
pub async fn open_files_dialog(
    app: tauri::AppHandle,
    title: String,
    supported_label: String,
    all_label: String,
) -> Result<Vec<String>, String> {
    let paths = app
        .dialog()
        .file()
        .add_filter(
            supported_label,
            &[
                "txt", "srt", "ass", "ssa", "lrc", "vtt", "sub", "csv", "tsv", "json", "xml",
                "html", "htm", "md", "epub",
            ],
        )
        .add_filter(all_label, &["*"])
        .set_title(title)
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
/// Emits an `epub-progress` event per chunk when there are 2+ chunks and
/// `file_id` is non-empty, so the UI
/// can show progress on a large single file. Returns the joined converted
/// text and the converter name reported by the first chunk.
async fn convert_in_chunks(
    app: Option<&tauri::AppHandle>,
    client: &reqwest::Client,
    file_id: &str,
    content: &str,
    opts: ConvertOptions<'_>,
) -> Result<(String, String), String> {
    let chunks = split_text(content, chunk_budget(opts)?);
    let total = chunks.len();
    let mut output = String::with_capacity(content.len());
    let mut result_converter = opts.converter.to_string();

    for (i, chunk) in chunks.iter().enumerate() {
        // Single-chunk files keep the plain "Converting…" label.
        if let Some(app) = app
            && total > 1
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

/// Canonicalize the input path and reject files over the size cap.
async fn resolve_input(input_path: &str) -> Result<PathBuf, String> {
    let canonical = tokio::fs::canonicalize(input_path)
        .await
        .map_err(|e| format!("INVALID_PATH:{e}"))?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("FILE_METADATA_FAILED:{e}"))?;
    check_file_size(metadata.len())?;
    Ok(canonical)
}

/// Output path and file name for a conversion. The path is built from the
/// canonical directory to prevent traversal.
async fn resolve_output(
    params: &ConvertParams,
    converter: &str,
) -> Result<(PathBuf, String), String> {
    let input = Path::new(&params.input_path);
    let dir = resolve_output_dir(input, &params.save_folder)?;
    let output_name = build_output_name(input, &params.naming, converter, &params.custom_suffix)?;
    let canonical_dir = tokio::fs::canonicalize(&dir)
        .await
        .map_err(|e| format!("OUTPUT_DIR_INVALID:{e}"))?;
    Ok((canonical_dir.join(&output_name), output_name))
}

/// Overwrite mode replaces the input: keep a copy first so the UI can undo.
fn backup_if_overwrite(
    backups: &Backups,
    file_id: &str,
    input: &Path,
    output: &Path,
) -> Result<Option<String>, String> {
    if input != output {
        return Ok(None);
    }
    let backup = backups.save(file_id, input)?;
    Ok(Some(backup.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn convert_file(
    app: tauri::AppHandle,
    client: tauri::State<'_, HttpClient>,
    backups: tauri::State<'_, Backups>,
    params: ConvertParams,
) -> Result<ConvertFileResult, String> {
    let canonical = resolve_input(&params.input_path).await?;

    // Read the file as raw bytes and decode with charset detection so that
    // non-UTF-8 subtitle files (Big5, GBK, Shift_JIS, UTF-16, ...) are handled.
    let raw = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("FILE_READ_FAILED:{e}"))?;
    let (content, source_encoding) = decode_text(&raw)?;

    // Convert in newline-aligned chunks (the API's front-end rejects request
    // bodies over ~1 MiB), streaming progress to the UI for large files.
    let (converted_text, result_converter) = convert_in_chunks(
        Some(&app),
        &client.0,
        &params.file_id,
        &content,
        params.options(),
    )
    .await?;

    let (output_path, output_name) = resolve_output(&params, &result_converter).await?;
    let backup_path = backup_if_overwrite(&backups, &params.file_id, &canonical, &output_path)?;

    // Write output in the source file's encoding
    let bytes = encode_text(&converted_text, source_encoding);
    let out_path = output_path.clone();
    tokio::task::spawn_blocking(move || {
        write_atomic(&out_path, |f| {
            f.write_all(&bytes)
                .map_err(|e| format!("FILE_WRITE_FAILED:{e}"))
        })
    })
    .await
    .map_err(|e| format!("FILE_WRITE_FAILED:{e}"))??;

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings: None,
        backup_path,
        // The UI shows this instead of asking the API for a separate preview.
        preview: Some(build_preview(&content, &converted_text)),
    })
}

/// Bytes read for a preview: a char is at most 4 bytes in any supported
/// encoding, so this always holds more than `PREVIEW_CHAR_LIMIT` chars.
const PREVIEW_READ_BYTES: u64 = 64 * 1024;

/// Convert a file via the API and return the original + converted text (capped)
/// for an on-screen diff preview. Unlike `convert_file`, nothing is written to disk.
#[tauri::command]
pub async fn preview_convert(
    client: tauri::State<'_, HttpClient>,
    params: ConvertParams,
) -> Result<PreviewResult, String> {
    // Only the start of the file is shown, so only the start is read: a large
    // file would cost a full read and decode on every click otherwise.
    let canonical = tokio::fs::canonicalize(&params.input_path)
        .await
        .map_err(|e| format!("INVALID_PATH:{e}"))?;
    let raw = tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        std::fs::File::open(&canonical)
            .and_then(|f| f.take(PREVIEW_READ_BYTES).read_to_end(&mut buf))
            .map(|_| buf)
    })
    .await
    .map_err(|e| format!("FILE_READ_FAILED:{e}"))?
    .map_err(|e| format!("FILE_READ_FAILED:{e}"))?;
    let (content, _) = decode_text(trim_partial_utf8(&raw))?;

    // Only the preview window is sent to the API — a large file would be
    // rejected for body size and the extra text would just be discarded here.
    let (original, _) = truncate_chars(&content, PREVIEW_CHAR_LIMIT);

    let data = convert_chunk(&client.0, original, params.options()).await?;

    Ok(build_preview(&content, &data.text))
}

/// Convert one extracted EPUB content file in place. Returns `Ok(false)` when
/// the file has no text to convert.
async fn convert_epub_chapter(
    client: &reqwest::Client,
    kind: epub::ContentKind,
    file_path: &Path,
    opts: ConvertOptions<'_>,
) -> Result<bool, String> {
    let xml = tokio::fs::read_to_string(file_path)
        .await
        .map_err(|e| format!("FILE_READ_FAILED:{e}"))?;

    let (text, count) = epub::extract_text(kind, &xml)?;
    if count == 0 {
        return Ok(false);
    }

    // Chunked, so a chapter over the request-body limit still converts.
    let (converted, _) = convert_in_chunks(None, client, "", &text, opts).await?;
    let new_xml = epub::replace_text(kind, &xml, &converted)?;
    tokio::fs::write(file_path, new_xml)
        .await
        .map_err(|e| format!("FILE_WRITE_FAILED:{e}"))?;

    // Small delay between API calls
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    Ok(true)
}

#[tauri::command]
pub async fn convert_epub(
    app: tauri::AppHandle,
    client: tauri::State<'_, HttpClient>,
    backups: tauri::State<'_, Backups>,
    params: ConvertParams,
) -> Result<ConvertFileResult, String> {
    let canonical = resolve_input(&params.input_path).await?;

    // Extract EPUB
    let input = canonical.clone();
    let (temp_dir, content_files) = tokio::task::spawn_blocking(move || epub::extract_epub(&input))
        .await
        .map_err(|e| format!("EPUB_EXTRACT_FAILED:{e}"))??;

    let chapter_total = content_files.len();
    let mut failed_chapters: usize = 0;
    let mut converted_chapters: usize = 0;
    let mut last_error = None;
    let opts = params.options();

    for (i, content_file) in content_files.iter().enumerate() {
        let _ = app.emit(
            "epub-progress",
            EpubProgress {
                file_id: params.file_id.clone(),
                chapter_index: i + 1,
                chapter_total,
                chapter_name: epub::chapter_display_name(&content_file.relative_path),
            },
        );

        let file_path = temp_dir.path().join(&content_file.relative_path);
        match convert_epub_chapter(&client.0, content_file.kind, &file_path, opts).await {
            Ok(true) => converted_chapters += 1,
            Ok(false) => {}
            // Offline or the API is rejecting every request: the remaining
            // chapters would fail the same way, each after its own timeout.
            Err(e) if is_fatal_api_error(&e) => return Err(e),
            Err(e) => {
                failed_chapters += 1;
                last_error = Some(e);
            }
        }
    }

    // Decided before repacking so a book where nothing converted is not written out.
    let warnings = epub_warnings(
        failed_chapters,
        converted_chapters,
        chapter_total,
        last_error,
    )?;

    let (output_path, output_name) = resolve_output(&params, &params.converter).await?;
    let backup_path = backup_if_overwrite(&backups, &params.file_id, &canonical, &output_path)?;

    // Repack EPUB
    let temp_path = temp_dir.path().to_path_buf();
    let out_path = output_path.clone();
    tokio::task::spawn_blocking(move || epub::repack_epub(&temp_path, &out_path))
        .await
        .map_err(|e| format!("EPUB_REPACK_FAILED:{e}"))??;

    Ok(ConvertFileResult {
        output_name,
        output_path: output_path.to_string_lossy().into_owned(),
        warnings,
        backup_path,
        preview: None,
    })
}

/// Put back the original that overwrite mode replaced for `file_id`.
#[tauri::command]
pub async fn restore_backup(
    backups: tauri::State<'_, Backups>,
    file_id: String,
) -> Result<(), String> {
    backups.restore(&file_id)
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
            let backup_dir = app.path().app_cache_dir()?.join("backups");
            app.manage(Backups::new(backup_dir)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_service_info,
            pick_save_folder,
            open_files_dialog,
            convert_file,
            convert_epub,
            preview_convert,
            restore_backup,
            is_portable,
        ])
        .run(tauri::generate_context!())
        .expect("啟動應用程式時發生錯誤");
}
