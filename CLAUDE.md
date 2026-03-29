# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

繁化姬 (Fanhuaji) is a Tauri 2 desktop app for batch Chinese text conversion (Traditional/Simplified/localized variants). It uses the [zhconvert.org API](https://api.zhconvert.org) for conversion. The UI is in Traditional Chinese (zh-Hant).

## Build & Dev Commands

```bash
# Frontend dev server (Vite on port 1420)
npm run dev

# Full Tauri dev (launches native window + Vite HMR)
npm run tauri dev

# Production build (TypeScript check + Vite build + Rust compile)
npm run tauri build

# Rust-only build (from src-tauri/)
cd src-tauri && cargo build

# TypeScript type check
npx tsc --noEmit
```

## Architecture

**Two-layer app: Vite frontend + Rust/Tauri backend.**

### Frontend (`src/`)
- `main.ts` — Single-file vanilla TypeScript app (no framework). Manages all UI state, DOM rendering, and Tauri IPC via `invoke()`.
- `styles.css` — All styling. macOS-native look with CSS custom properties.
- `index.html` — Tab-based UI: file list, save options, module settings, custom replacements, file preview.

### Backend (`src-tauri/src/`)
- `lib.rs` — All application logic. Three Tauri commands:
  - `get_service_info` — Fetches available conversion modules and dict version from zhconvert API
  - `open_files_dialog` — Native file picker via `tauri-plugin-dialog`
  - `convert_file` — Reads file, sends content to zhconvert `/convert` API, writes result
- `main.rs` — Entry point, calls `fanhuaji_lib::run()`

### IPC Contract
Frontend calls backend via `invoke("command_name", { params })`. The Rust commands use `#[tauri::command]` and return `Result<T, String>`. Params use camelCase on the JS side, snake_case on the Rust side (Tauri auto-converts).

## Key Dependencies
- **Tauri 2** with plugins: `tauri-plugin-dialog`, `tauri-plugin-opener`
- **reqwest** (HTTP client for zhconvert API)
- **Vite 6** (frontend bundling, dev server on port 1420)

## Conventions
- UI text and error messages are in Traditional Chinese
- API base URL is hardcoded as `https://api.zhconvert.org` in `lib.rs`
- No frontend framework — plain DOM manipulation with innerHTML templates
- State is managed as module-level `let` variables in `main.ts`
