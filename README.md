# 繁化姬 Tauri 版

基於 [Fanhuaji-GUI-by-James1201](https://github.com/Fanhuaji/Fanhuaji-GUI-by-James1201) 使用 Tauri 2 重新撰寫的中文繁簡轉換桌面應用程式，使用 [zhconvert.org](https://zhconvert.org) API。

## 下載

前往 [Releases](https://github.com/7a6163/fanhuaji-tauri/releases) 下載最新版本：

| 平台 | 架構 | 格式 |
|------|------|------|
| macOS | Apple Silicon (aarch64) | `.dmg` |
| macOS | Intel (x86_64) | `.dmg` |
| Windows | x86_64 | `.msi` / `.exe` |
| Linux | x86_64 | `.AppImage` / `.deb` |
| Linux | ARM64 (aarch64) | `.AppImage` / `.deb` |

應用程式內建自動更新，啟動時會自動檢查新版本。

### macOS 首次開啟

由於應用程式未經 Apple 簽名，macOS 會顯示「無法打開」的警告。請依以下步驟操作：

1. 點擊 **Done**（完成）
2. 前往 **系統設定 → 隱私與安全性**
3. 往下滑找到「Fanhuaji was blocked」，點擊 **仍要打開**

或在終端機執行：

```bash
xattr -cr /Applications/Fanhuaji.app
```

## 功能

- 批次轉換多個檔案（支援 txt、srt、ass、lrc、vtt、csv、json、xml、html、md 等格式）
- 多種轉換模式：繁體化、簡體化、台灣化、香港化、中國化、注音化、拼音化等
- 詞語模組設定（自動偵測 / 啟用 / 停用）
- 自訂取代規則（轉換前取代、轉換後取代、保護詞彙）
- 檔案預覽與差異比較
- 彈性命名方式（自動命名、覆蓋原檔、加入後綴）
- 應用程式內自動更新

## 開發

### 系統需求

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://www.rust-lang.org/tools/install) >= 1.77
- Tauri 2 系統相依套件（參考 [Tauri 官方文件](https://v2.tauri.app/start/prerequisites/)）

### 指令

```bash
# 安裝前端相依套件
npm install

# 啟動開發模式（Tauri 視窗 + Vite HMR）
npm run tauri dev

# 正式建置
npm run tauri build
```

### 發布新版本

1. 更新版本號：

```bash
# 會同時更新 package.json、package-lock.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml
npm version <major|minor|patch>
```

2. 推送 tag 觸發 GitHub Actions 自動建置：

```bash
git push && git push --tags
```

GitHub Actions 會自動為所有平台建置並建立 Release（含 `latest.json` 供自動更新）。

### 手動發布

```bash
# 建置
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/fanhuaji.key)" npm run tauri build

# 建立 GitHub Release
gh release create v1.x.x src-tauri/target/release/bundle/dmg/*.dmg --title "v1.x.x"
```

## 技術架構

| 層級 | 技術 |
|------|------|
| 前端 | TypeScript + Vite（無框架，原生 DOM） |
| 後端 | Rust + Tauri 2 |
| API  | [zhconvert.org](https://api.zhconvert.org) |
| CI/CD | GitHub Actions（跨平台自動建置） |
| 更新 | tauri-plugin-updater（應用程式內自動更新） |

## 授權

本程式原始碼以 [MIT](LICENSE) 授權釋出。

本程式使用了[繁化姬](https://docs.zhconvert.org/)的 API 服務，其使用須遵守繁化姬的[服務條款](https://docs.zhconvert.org/license/)。商業使用請參閱繁化姬[授權條款](https://docs.zhconvert.org/license/)。
