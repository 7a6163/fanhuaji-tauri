#!/usr/bin/env node
/**
 * Syncs the version from package.json to:
 * - src-tauri/tauri.conf.json
 * - src-tauri/Cargo.toml
 *
 * Run automatically via npm's "version" lifecycle hook.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const pkg = JSON.parse(readFileSync(resolve(root, "package.json"), "utf-8"));
const version = pkg.version;

// Update tauri.conf.json
const tauriConfPath = resolve(root, "src-tauri/tauri.conf.json");
const tauriConf = JSON.parse(readFileSync(tauriConfPath, "utf-8"));
tauriConf.version = version;
writeFileSync(tauriConfPath, `${JSON.stringify(tauriConf, null, 2)}\n`);

// Update Cargo.toml
const cargoPath = resolve(root, "src-tauri/Cargo.toml");
let cargo = readFileSync(cargoPath, "utf-8");
cargo = cargo.replace(
  /^version\s*=\s*"[^"]*"/m,
  `version = "${version}"`,
);
writeFileSync(cargoPath, cargo);

console.log(`Version synced to ${version}`);
