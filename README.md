# DM Voice

Simple Push-to-Talk - Speech-to-Text app for macOS and Windows. Hold the hotkey → speak → release → text gets injected into the currently active text field. Local, offline, powered by Whisper.

![DM Voice — hold the hotkey, speak, watch the text appear](assets/demo.svg)

## Interface

| Tray menu | Settings |
|:---:|:---:|
| <img src="assets/tray-menu.svg" alt="Tray menu with model picker, autostart toggle and settings" width="320"> | <img src="assets/settings.svg" alt="Settings window with hotkey, permissions and model management" width="320"> |

> These mockups mirror the actual UI in structure and behavior. Minor visual differences from the real app may stem from the macOS renderer and the user's accent color choice.

## Features

- **Push-to-talk** with a freely configurable hotkey (default: `Alt+Space`)
- **Whisper models** to choose from (tiny / small / medium / large-v3-turbo / large-v3) — switchable directly from the tray menu
- **Local & offline** — no cloud upload. GPU-accelerated via **Metal** on Apple Silicon and **Vulkan** on Windows (NVIDIA, AMD, Intel Arc/Xe)
- **Text injection** into any active app — Cmd+V on macOS (CGEvent + AX API), Ctrl+V on Windows
- **Launch at login** as a tray-menu toggle
- **Auto-updates** via the built-in Tauri updater (tray → "Check for updates…")
- **Recording limit**: 60 seconds per push-to-talk

## Installation

Grab the latest build for your platform from [Releases](https://github.com/m0nji/DM_Voice/releases).

### macOS (Apple Silicon)

1. Download `DM.Voice_<version>_aarch64.dmg`
2. Open the DMG and drag `DM Voice.app` into `/Applications` (or `~/Applications`)
3. Launch the app — it is Apple-notarized, so Gatekeeper opens it without the "right-click → Open" workaround
4. **Grant permissions** on first run:
   - **Microphone** — to capture audio
   - **Accessibility** — to inject text via Cmd+V

> Intel Macs are not built (long queue times on `macos-13` runners; low demand). Build from source if needed.

### Windows (x86_64)

1. Download `DM.Voice_<version>_x64-setup.exe`
2. Run the NSIS installer. On first launch Windows SmartScreen may show a warning — click "More info → Run anyway"
3. GPU acceleration works out-of-the-box on NVIDIA, AMD and Intel Arc/Xe (Vulkan ships with the GPU driver — no separate SDK needed for end users)

> Windows ARM64 (Snapdragon X) is not built yet — pending Vulkan-SDK validation on `aarch64-pc-windows-msvc`.

On first launch the `large-v3-turbo` model (~874 MB) is downloaded automatically. Other models can be fetched afterward from the tray menu.

## Usage

- **Recording**: hold the hotkey (default `Alt+Space`), speak, release → text is inserted into the active field
- **Click the tray icon**: opens the menu with model picker, autostart toggle, settings, quit
- **Settings** → change hotkey, check permissions, download/delete models

## Building from source

Prerequisites (both platforms): Rust toolchain, Tauri CLI (`cargo install tauri-cli`).

### macOS

Additional: macOS 13+, Xcode Command Line Tools.

```bash
git clone https://github.com/m0nji/DM_Voice.git
cd DM_Voice/src-tauri
cargo tauri build --bundles app,dmg
```

Output: `src-tauri/target/release/bundle/macos/DM Voice.app` and the DMG under `bundle/dmg/`.

**Note**: If you sign the build yourself, omit the hardened-runtime flag
(`codesign` without `--options runtime`) — otherwise macOS silently suppresses
the microphone TCC prompt for locally self-signed certs.

### Windows

Additional: Visual Studio 2022 with the "Desktop development with C++" workload (for MSVC + Windows SDK), [Ninja](https://github.com/ninja-build/ninja/releases), and the [LunarG Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows) (1.3.290+). Enable Windows long-path support if you hit `C1041` errors during the `whisper.cpp` Vulkan-shaders build.

```powershell
git clone https://github.com/m0nji/DM_Voice.git
cd DM_Voice\src-tauri
cargo tauri build --bundles nsis
```

Output: NSIS installer at `src-tauri\target\release\bundle\nsis\`.

For the exact CI recipe (MSVC env, ccache workaround, etc.) see [`.github/workflows/release.yml`](.github/workflows/release.yml).

## License

DM Voice is licensed under the **GNU General Public License v3.0 or later** (GPL-3.0+).
See [LICENSE](LICENSE) for the full text.

In short:
- You may use, modify and redistribute the app — including commercially.
- If you redistribute a modified version, you must also publish its complete
  source code under the same (or a compatible later) GPL license.
  Closed-source forks are not permitted.
- No warranty, no liability — see LICENSE sections 15 and 16.

© 2026 DM Apps.

---

<p align="center">
  <a href="https://www.buymeacoffee.com/m0nji" target="_blank">
    <img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" height="50">
  </a>
  &nbsp;&nbsp;
  <a href="https://www.paypal.com/pool/9p2cITSKXm?sr=wccr" target="_blank">
    <img src="https://www.paypalobjects.com/en_US/i/btn/btn_donate_LG.gif" alt="Donate via PayPal" height="50">
  </a>
</p>
