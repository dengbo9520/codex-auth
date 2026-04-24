# Codex Auth GUI

Windows local desktop GUI for `codex-auth`.

Purpose: manage local `codex-auth` accounts with a clear visual interface, so users with legitimate ChatGPT/OpenAI accounts can switch accounts more conveniently.

## Usage Boundary

This project is for personal, learning, and non-commercial use only.

Requirements:

- Use your own genuine ChatGPT/OpenAI account.
- Use a valid subscription or access plan where required.
- Follow OpenAI and ChatGPT terms of service.

Not supported:

- Selling accounts or access.
- Sharing rented accounts.
- Bypassing subscription limits, usage limits, or risk controls.
- Commercial account-pool operation.
- Token theft, token cracking, or authentication bypass.

The GUI does not provide accounts, tokens, or any paid access. It only wraps local `codex-auth` commands and reads local `codex-auth` registry data.

## Features

- View local `codex-auth` accounts from `%USERPROFILE%\.codex\accounts\registry.json`.
- Switch accounts through `codex-auth switch`.
- Remove accounts through `codex-auth remove`.
- Set account aliases for duplicate emails.
- Import auth files, folders, or CPA token data.
- Launch web login or device-code login.
- Toggle `codex-auth` auto-switch and API usage mode.
- Show 5-hour and weekly usage windows when available.
- Auto-refresh account state in the background.
- Show command history, stdout, stderr, exit code, and diagnostics.

## Safety Model

- Account mutations call local `codex-auth` CLI.
- GUI reads local registry for display state.
- GUI does not reimplement `codex-auth` auth logic.
- GUI does not store plaintext tokens.
- Command logs are stored only in the Tauri app data directory.
- Alias editing writes `registry.json` and creates a backup first.

## Requirements

- Windows 11 recommended.
- Node.js 22+.
- Rust toolchain.
- Visual Studio C++ Build Tools.
- Microsoft WebView2 Runtime.
- Installed and working `codex-auth` CLI.
- Installed and working Codex CLI/App if you want to use the switched account there.

Check tools:

```powershell
node --version
rustc --version
codex-auth --version
codex --version
```

## Install

```powershell
npm install
```

## Development

```powershell
npm run tauri dev
```

## Build

Frontend build:

```powershell
npm run build
```

Rust tests:

```powershell
npm run test:rust
```

Windows installer bundle:

```powershell
npm run tauri -- build
```

Build output:

```text
src-tauri\target\release\codex-auth-gui.exe
src-tauri\target\release\bundle\nsis\*.exe
src-tauri\target\release\bundle\msi\*.msi
```

## How To Use

1. Install and configure `codex-auth`.
2. Start Codex Auth GUI.
3. Open Settings and use web login or device-code login when needed.
4. Open Import if you already have local auth files.
5. Open Accounts to view imported accounts.
6. Add aliases for accounts with duplicate email addresses.
7. Click Switch to activate an account.
8. Restart Codex CLI/App after switching, so the new active account is picked up.
9. Use Refresh if you want immediate state update. The app also refreshes in the background.

## Duplicate Email Accounts

`codex-auth switch <email>` can become interactive when two accounts share the same email. This GUI avoids interactive mode by resolving a unique selector.

Recommended fix:

- Set an alias, for example `melissa-plus`.
- Switch by alias through the GUI.

## Local Paths

Codex Auth GUI reads these local paths:

```text
%USERPROFILE%\.codex
%USERPROFILE%\.codex\accounts
%USERPROFILE%\.codex\accounts\registry.json
%USERPROFILE%\.codex\sessions
```

GUI command logs are stored under the Tauri app data directory:

```text
%APPDATA%\com.loongphy.codexauthgui\command-history.json
```

## Stack

- Tauri 2
- React
- TypeScript
- Vite
- Tailwind CSS v4
- shadcn/ui
- TanStack Query
- Zustand
- React Hook Form
- Zod
- Rust backend

## Project Structure

```text
src/                  React frontend
src/lib/api.ts         Tauri invoke API wrapper
src/lib/types.ts       Frontend DTO types
src/store/             UI state
src-tauri/src/         Rust backend
src-tauri/icons/       App icons
scripts/              Windows build helpers
```

## License / Terms

Non-commercial use only.

Use only with legitimate ChatGPT/OpenAI accounts that you own or are authorized to use. Do not use this project to resell access, pool accounts commercially, or bypass service limits.
