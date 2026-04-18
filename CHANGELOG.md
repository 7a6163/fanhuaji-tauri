# Changelog

本檔案記錄繁化姬 Tauri 版的所有重要變更。格式基於 [Keep a Changelog](https://keepachangelog.com/)。

## [2.2.1] - 2026-04-17

### Security

- 修復 rustls-webpki TLS 憑證驗證繞過漏洞（RUSTSEC-2026-0098、RUSTSEC-2026-0099）：升級 `rustls-webpki` 至 0.103.12，防止同網段攻擊者透過憑證名稱限制繞過進行更新通道 MitM
- 修復 EPUB 解壓縮炸彈（Zip Bomb）漏洞：新增單檔 100 MiB 與累計 500 MiB 解壓上限，防止惡意 EPUB 耗盡磁碟或記憶體
- 修復 EPUB ZIP slip 路徑穿越漏洞：原先的 `Path::starts_with` 為 component 結構比對，無法偵測 `..` 穿越（如 `/tmp/x/../evil`）；新增 component 層級檢查，拒絕絕對路徑與含 `..` component 的 entry
- 更新 `rand` 至 0.9.4（RUSTSEC-2026-0097）

## [2.2.0] - 2026-04-09

### Added

- 多語言支援（i18n）：正體中文、簡體中文、English
- 語言自動偵測（依系統語言設定，支援手動切換）
- 語言選擇器（設定 → 一般 → 語言）
- 語言偏好持久化（重啟後保留選擇）

## [2.1.0] - 2026-04-09

### Added

- 所有設定持久化（轉換模式、命名方式、取代規則、模組設定重啟後保留）
- 自動轉換開關（設定 → 一般 → 行為，可關閉拖入後自動轉換）
- 自訂後綴命名（命名方式選擇「自訂後綴」可輸入自定義後綴）
- 設定抽屜滑入/滑出動畫
- Escape 鍵關閉設定抽屜

### Changed

- API 授權聲明從主畫面移至設定 → 關於
- 移除標題列重複的應用名稱（macOS 標題列已顯示）
- 無副檔名時輸出檔名不再產生尾隨句點（Windows 相容）

## [2.0.4] - 2026-03-30

### Fixed

- Windows 桌面捷徑圖示模糊（ICO 檔從單一 16x16 改為包含 16~256 多解析度）

### Changed

- 更新專案描述，移除舊版 GUI 參考

## [2.0.3] - 2026-03-30

### Added

- 自訂輸出資料夾功能（設定中選擇，持久化儲存）
- Windows portable ZIP 版本（免安裝）

### Fixed

- 主題切換按鈕在淺色/深色模式下配色不一致
- CI 升級 codecov-action v6（Node.js 24）
- CI 同一分支重複 push 時自動取消上一個 run

### Changed

- HTTP client 改為 Tauri managed state 共享（連線池複用，效能提升）
- Rust edition 升級至 2024
- Linux Wayland EGL workaround 自動設定

## [2.0.0] - 2026-03-29

### Added

- 全新極簡 UI：拖入檔案即自動轉換，零點擊操作
- EPUB 電子書轉換支援（逐章轉換，保留結構/CSS/圖片）
- EPUB 轉換章節進度即時顯示
- 設定 drawer（側邊抽屜）分頁：轉換、取代、模組、一般
- 跨平台字型統一：Inter + Noto Sans TC
- 跨平台 select 控制項樣式統一（appearance: none）
- 進度條顯示批次轉換進度
- 空狀態拖放區引導（支援格式提示）
- Windows portable ZIP 版本
- Linux Wayland EGL workaround 自動設定

### Changed

- UI 重新設計：移除 6 個 tab，改為單一主畫面 + 設定 drawer
- 檔案列表簡化為：檔名 + 狀態圖示 + 訊息
- 轉換模式移入設定（預設台灣化）
- 拖放/開啟檔案後自動開始轉換
- NSIS 安裝模式改為 currentUser（不需管理員權限）
- CSS 全面改用 CSS 變數，支援 light/dark/system 主題

### Security

- EPUB ZIP 解壓加入路徑穿越防護
- EPUB 轉換改用 async I/O，避免阻塞 Tokio 執行緒
- 部分章節失敗改用 warnings 欄位回報，不再污染 output_path

## [1.1.0] - 2026-03-29

### Added

- 設定 tab：主題切換（系統 / 淺色 / 深色）、檢查更新、關於資訊
- 拖放檔案支援（拖放到視窗自動加入檔案清單）
- 應用程式內自動更新（tauri-plugin-updater）
- 動態版本號顯示（標題欄 + 設定頁從 Tauri config 讀取）
- CI/CD：GitHub Actions 跨平台自動建置（macOS universal、Windows、Linux x86_64/ARM64）
- 測試：87 個測試（52 TypeScript + 35 Rust），TypeScript 97% 行覆蓋率
- Codecov 覆蓋率追蹤整合
- CI 和 Codecov badge 加入 README

### Fixed

- 檔案清單表格不再擠壓右側操作按鈕
- 主題切換按鈕在淺色/深色模式下正確顯示
- 轉換功能參數傳遞修正（convert_file params wrapping）
- URL 白名單驗證，防止 open-redirect 攻擊
- 路徑穿越防護改進：從 canonical 目錄建構輸出路徑
- escHtml 加入單引號轉義
- Theme data-attribute 加入 runtime 驗證（移除不安全的 as cast）
- initVersion 加入錯誤處理
- 浮動 Promise 標記 void

### Changed

- 商業使用提示文字改為「商業使用請參閱繁化姬授權條款」
- productName 改為 ASCII（Fanhuaji）確保跨平台檔名正確
- countByStatus 改為單次遍歷（效能優化）
- Rust 程式碼提取純函式（build_output_name、build_api_params、parse_modules）
- tab 切換邏輯抽出 activateTab 工具函式
- GitHub Actions 升級至 v5 + Node.js 24

### Security

- 加入 URL 白名單（isSafeUrl），僅允許 zhconvert.org、docs.zhconvert.org、github.com
- 輸出路徑改為從 canonical 目錄建構，fail closed
- escHtml 補上單引號轉義，防止屬性注入

## [1.0.0] - 2026-03-28

### Added

- 批次檔案轉換功能，支援 txt、srt、ass、ssa、lrc、vtt、csv、json、xml、html、md 等格式
- 多種轉換模式：繁體化、簡體化、台灣化、香港化、中國化、注音化、拼音化、火星化
- 第三方服務支援：維基簡體化、維基繁體化
- App 限定模式：僅轉換字符編碼、僅修改檔案名稱
- 詞語模組設定，依分類瀏覽，支援自動偵測 / 啟用 / 停用
- 自訂取代規則：轉換前取代、轉換後取代、保護詞彙
- 檔案預覽面板（檔案資訊、輸入預覽、輸出預覽、差異比較、轉換資訊）
- 彈性存檔選項：自動命名、覆蓋原檔、加入後綴
- 狀態列即時顯示全部 / 待轉換 / 成功 / 錯誤數量
- 原生檔案選擇對話框（tauri-plugin-dialog）
