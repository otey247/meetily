# Rebuilding the Meetily Windows Desktop App (.exe)

This document explains, end to end, how to rebuild the full Meetily desktop
application on Windows after making source changes (for example, the new
"chat with the transcript" feature). It covers prerequisites, the two sidecar
binaries the bundle depends on, the recommended one-command build, the manual
build steps it wraps, where the output `.exe` / installers land, and
troubleshooting.

> **TL;DR** — From the `frontend/` directory run **`build-gpu.bat`**. It builds
> the `llama-helper` sidecar, copies it into `src-tauri/binaries/`, then runs
> `pnpm run tauri:build`, which auto-downloads `ffmpeg`, compiles the Rust core
> and Next.js UI, and produces the NSIS installer and MSI under
> `src-tauri/target/release/bundle/`.

---

## 1. What "the full .exe" actually is

A production build produces three relevant artifacts:

| Artifact | Path (under `frontend/src-tauri/target/release/`) | Purpose |
| --- | --- | --- |
| Raw executable | `meetily.exe` | The app binary itself (not redistributable on its own — needs bundled resources) |
| NSIS installer | `bundle/nsis/meetily_0.4.0_x64-setup.exe` | The user-facing installer most people mean by "the .exe" |
| MSI installer | `bundle/msi/meetily_0.4.0_x64_en-US.msi` | Enterprise/MSI deployment |

The version number comes from `productName` / `version` in
[src-tauri/tauri.conf.json](../frontend/src-tauri/tauri.conf.json) (currently
`meetily` / `0.4.0`). The bundle `targets` are `msi` and `nsis` on Windows.

### The two bundled sidecar binaries (critical)

`tauri.conf.json` declares two **external binaries** that must exist before the
bundler runs:

```jsonc
"externalBin": [
    "binaries/llama-helper",
    "binaries/ffmpeg"
]
```

Tauri requires each external binary to be present **as a target-triple–suffixed
file** in `frontend/src-tauri/binaries/`, e.g. on a 64-bit machine:

```
frontend/src-tauri/binaries/llama-helper-x86_64-pc-windows-msvc.exe
frontend/src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe
```

> ⚠️ **`frontend/src-tauri/binaries/` is in `.gitignore`** (see line 79 of the
> repo `.gitignore`). On a fresh checkout the folder does **not** exist, and
> neither binary is committed. They are produced at build time:
>
> - **`ffmpeg`** is downloaded and verified automatically by the Cargo build
>   script [src-tauri/build/ffmpeg.rs](../frontend/src-tauri/build/ffmpeg.rs)
>   (pulled from the `Zackriya-Solutions/ffmpeg-binaries` GitHub release, cached
>   after the first build). **This requires network access on the first build.**
> - **`llama-helper`** must be compiled from the `llama-helper/` crate at the
>   **repo root** and copied into `binaries/`. The `build-gpu.bat` script does
>   this for you; a plain `tauri build` does **not**.
>
> This is why `clean_build_windows.bat` (which only runs `pnpm install` +
> `tauri build`) will **fail on a clean checkout** — nothing has built/copied
> `llama-helper` yet. Use `build-gpu.bat` instead, or build the sidecar manually
> (Section 5).

---

## 2. Prerequisites (one-time setup)

All four are required. The build scripts probe standard install locations for
Visual Studio and LLVM.

| Tool | Why | Install |
| --- | --- | --- |
| **Node.js 18+** | Builds the Next.js frontend, runs the Tauri CLI | <https://nodejs.org> |
| **Rust (rustup)** | Compiles the Tauri core, `llama-helper`, and `whisper-rs` | <https://www.rust-lang.org/tools/install> |
| **Visual Studio Build Tools** with the **"Desktop development with C++"** workload | MSVC compiler/linker + Windows SDK for the native crates | VS Installer → Modify → check the C++ workload |
| **CMake** | Builds `whisper.cpp` / GPU backends | <https://cmake.org/download/> (add to PATH) |
| **LLVM** (libclang) | `whisper-rs-sys` bindgen needs `libclang` | <https://releases.llvm.org/> — the scripts expect it at `C:\Program Files\LLVM\bin` |

`pnpm` is also used. If it is not installed globally you can run it on demand
with `npx -y pnpm@10 <cmd>` (the build scripts call `pnpm` directly, so for the
one-command path install it via `npm i -g pnpm` or `corepack enable`).

### Notes specific to this machine / repo

- The Windows build scripts (`build.bat`, `build-gpu.bat`) **hardcode Visual
  Studio 2022 paths first**, then fall back to a VS 2019 Build Tools path. If you
  only have **VS 2019**, the fallback branch is used — confirm the build output
  prints "Using Visual Studio 2019 Build Tools". If you hit linker/SDK errors,
  installing the **VS 2022 Build Tools C++ workload** matches the scripts'
  primary path and is the most reliable option.
- The repo's `pnpm-lock.yaml` has an overrides mismatch, so a strict
  `--frozen-lockfile` install fails. The build scripts use a normal `pnpm
  install`, which is fine. If you install manually, use
  `npx -y pnpm@10 install --no-frozen-lockfile` and then
  `git checkout -- pnpm-lock.yaml` to avoid committing lockfile churn.

---

## 3. Recommended: one-command full build

From the **`frontend/`** directory (or the repo root — the script locates
`package.json` either way):

```bat
build-gpu.bat
```

What it does, in order:

1. Frees port 3118 if something is listening.
2. Sets `LIBCLANG_PATH=C:\Program Files\LLVM\bin` and configures the Visual
   Studio / Windows SDK environment.
3. Auto-detects a GPU feature via `scripts/auto-detect-gpu.js` (NVIDIA → `cuda`,
   AMD/Intel → `vulkan`, otherwise CPU). You can force it:
   ```bat
   set TAURI_GPU_FEATURE=vulkan
   build-gpu.bat
   ```
   Use `set TAURI_GPU_FEATURE=` (empty) to force CPU-only.
4. Builds the `llama-helper` sidecar in release mode from the repo-root
   `llama-helper/` crate with the detected feature.
5. Detects the host target triple (`rustc -vV`) and copies the sidecar to
   `src-tauri/binaries/llama-helper-<triple>.exe`.
6. Runs `pnpm run tauri:build`, which:
   - runs `pnpm build` (Next.js production export to `frontend/out`),
   - executes `build/ffmpeg.rs` to download/verify `ffmpeg` into `binaries/`,
   - compiles the Rust core (applies the SQLite migrations' schema at first run,
     not build time),
   - bundles the NSIS + MSI installers.

When it finishes you'll see "Build completed successfully!" and the artifacts in
Section 1 will exist.

> **First build is slow.** Compiling `whisper.cpp` + all Rust crates from cold
> can take 20–40+ minutes. Subsequent incremental builds are much faster because
> Cargo and the ffmpeg cache are reused.

---

## 4. Quick iterate: dev build (hot reload)

For testing changes without producing installers, use the dev build. It still
requires the sidecar binaries to exist, so run a full `build-gpu.bat` once first
(or build the sidecar manually per Section 5), then:

```bat
clean_run_windows.bat
```

or directly:

```bat
pnpm run tauri:dev
```

This launches the app with the Next.js dev server on port 3118 and Rust hot
recompile. Enable verbose logging with:

```powershell
$env:RUST_LOG="debug"; pnpm run tauri:dev
```

---

## 5. Manual build (understanding / customizing each step)

If you want to run the steps yourself (or `build-gpu.bat` fails midway), this is
the equivalent sequence. Run from `frontend/`.

**5.1 Set up the toolchain environment** (in the same shell):

```bat
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
```
(Adjust the path to your VS edition/year. For VS 2019 use the `...\2019\BuildTools\...` path.)

**5.2 Install JS dependencies:**

```bat
pnpm install
```

**5.3 Build the `llama-helper` sidecar** (from the repo-root crate):

```bat
pushd ..\llama-helper
cargo build --release
:: For GPU: cargo build --release --features cuda   (or vulkan)
popd
```

**5.4 Copy the sidecar into `binaries/` with the host target triple.** Find your
triple with `rustc -vV` (look at the `host:` line, typically
`x86_64-pc-windows-msvc`), then:

```bat
mkdir src-tauri\binaries 2>nul
copy /Y ..\target\release\llama-helper.exe src-tauri\binaries\llama-helper-x86_64-pc-windows-msvc.exe
```

**5.5 Produce the installers** (this also auto-downloads `ffmpeg` on first run):

```bat
pnpm run tauri:build
```

GPU-specific variants are available as npm scripts if you prefer to pin the
feature instead of relying on auto-detection:

```bat
pnpm run tauri:build:cuda
pnpm run tauri:build:vulkan
pnpm run tauri:build:cpu
```

---

## 6. Verifying the new chat feature compiled in

The "chat with the transcript" feature added a Rust `chat` module, five new
Tauri commands, and a SQLite migration. After building:

- **Compile check only** (no bundle), from `frontend/`:
  ```bat
  build.bat check
  ```
  This runs `cargo check --no-default-features` in `src-tauri/` and is the
  fastest way to confirm the Rust changes compile before committing to a full
  build.
- **Migration**: the `chat_messages` table is created automatically by the sqlx
  migrator at app startup (migration
  `migrations/20260611000000_add_chat_messages.sql`). No build-time step is
  needed. To confirm, launch the app and check the chat panel (bottom-right
  button); on a saved meeting, asking a question should stream a response and
  the history should survive an app restart.

---

## 7. Build output reference

After a successful production build, under `frontend/src-tauri/target/release/`:

```
meetily.exe                                  ← raw app binary
binaries/llama-helper-<triple>.exe           ← LLM sidecar (built in step 5.3/4)
binaries/ffmpeg-<triple>.exe                 ← auto-downloaded by build.rs
bundle/nsis/meetily_0.4.0_x64-setup.exe      ← installer (distribute this)
bundle/msi/meetily_0.4.0_x64_en-US.msi       ← MSI installer
```

Install the app by running the NSIS `*-setup.exe`. To distribute, share that
installer — it carries the executable, the sidecars, templates, and icons.

---

## 8. Code signing (optional)

`tauri.conf.json` references a Windows `signCommand` that calls
`scripts/sign-windows.ps1`. That script **no-ops unless** the
`DIGICERT_KEYPAIR_ALIAS` environment variable is set (it uses DigiCert `smctl`).
For local/test builds you can ignore signing entirely; the unsigned installer
works but will show a SmartScreen warning. To sign, set the alias and have the
DigiCert KeyLocker tooling installed before building.

---

## 9. Troubleshooting

| Symptom | Cause / Fix |
| --- | --- |
| `failed to bundle ... binaries/llama-helper-<triple>.exe not found` | The sidecar wasn't built/copied. Run `build-gpu.bat`, or do Section 5.3–5.4 manually. Confirm the triple in the filename matches `rustc -vV` host. |
| Build hangs/fails downloading ffmpeg | `build/ffmpeg.rs` needs network on first build. Re-run with connectivity; the binary is cached in `binaries/` afterward. To skip the download, drop a working `ffmpeg-<triple>.exe` into `binaries/` yourself. |
| `libclang.dll not found` / bindgen errors | LLVM missing or `LIBCLANG_PATH` not set. Install LLVM to `C:\Program Files\LLVM` and set `LIBCLANG_PATH=C:\Program Files\LLVM\bin`. |
| `kernel32.lib` / `msvcrt.lib` not found, or linker errors | Visual Studio C++ workload or Windows SDK missing. Install "Desktop development with C++". The scripts prefer VS 2022; with only VS 2019 use the 2019 fallback or install the VS 2022 Build Tools. |
| `cmake` not recognized | Install CMake and add it to PATH; restart the shell. |
| `cargo` not recognized | Install Rust via rustup; restart the shell so PATH updates. |
| `pnpm install` fails on frozen lockfile | Use `pnpm install` (non-frozen) or `npx -y pnpm@10 install --no-frozen-lockfile`, then `git checkout -- pnpm-lock.yaml`. |
| Port 3118 already in use (dev) | The scripts kill it automatically; otherwise `taskkill` the PID from `netstat -aon | findstr :3118`. |
| GPU not being used | Auto-detection needs the GPU **SDK**, not just drivers (CUDA Toolkit / Vulkan SDK). Force with `set TAURI_GPU_FEATURE=cuda` (or `vulkan`). See [GPU_ACCELERATION.md](GPU_ACCELERATION.md). |

---

## 10. Quick reference

```bat
:: Full production build (installers) — recommended
cd frontend
build-gpu.bat

:: Force a GPU/CPU feature
set TAURI_GPU_FEATURE=vulkan & build-gpu.bat
set TAURI_GPU_FEATURE=        & build-gpu.bat   :: CPU-only

:: Dev build with hot reload (sidecars must already exist)
clean_run_windows.bat

:: Rust compile check only (fast)
build.bat check
```

Output installer: `frontend/src-tauri/target/release/bundle/nsis/meetily_0.4.0_x64-setup.exe`
</content>
</invoke>
