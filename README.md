# 繁化姬 Tauri 版

中文繁簡轉換桌面應用程式，基於 [zhconvert.org](https://zhconvert.org) API，使用 Tauri 2 建構。

## 功能

- 批次轉換多個檔案（支援 txt、srt、ass、lrc、vtt、csv、json、xml、html、md 等格式）
- 多種轉換模式：繁體化、簡體化、台灣化、香港化、中國化、注音化、拼音化等
- 詞語模組設定（自動偵測 / 啟用 / 停用）
- 自訂取代規則（轉換前取代、轉換後取代、保護詞彙）
- 檔案預覽與差異比較
- 彈性命名方式（自動命名、覆蓋原檔、加入後綴）

## 系統需求

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://www.rust-lang.org/tools/install) >= 1.70
- Tauri 2 系統相依套件（參考 [Tauri 官方文件](https://v2.tauri.app/start/prerequisites/)）

## 開發

```bash
# 安裝前端相依套件
npm install

# 啟動開發模式（Tauri 視窗 + Vite HMR）
npm run tauri dev

# 正式建置
npm run tauri build
```

## 技術架構

| 層級 | 技術 |
|------|------|
| 前端 | TypeScript + Vite（無框架，原生 DOM） |
| 後端 | Rust + Tauri 2 |
| API  | [zhconvert.org](https://api.zhconvert.org) |

## 授權

MIT
