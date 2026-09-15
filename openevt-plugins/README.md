# openevt-plugins

`openevt-plugins` provides dynamic plugin integrations for processing and streaming EVT3 event-based vision data.

## Devices

The crate builds to a single plugin library, `libopenevt_plugins.so`, providing
the devices most applications want on hand:

| Name | Description | Serial |
| --- | --- | --- |
| `openevt_simulator` | Synthesises CD events from a video file. | `EventSimulator` |
| `openevt_raw_file` | Replays a recorded EVT3 file as a device. | the file path |

The raw-file reader accepts any path as its serial, so a recording can be opened
directly without being advertised first. Set `OPENEVT_RAW_FILES` to a
platform-separated list of paths to have recordings show up in device
enumeration as well.

A shared library can only export one `abi_stable` root module, so both devices
are exported through one vtable and registered individually by the host. See
[`src/module.rs`](src/module.rs).

---

## 🛠️ Installation & Setup

Plugins are loaded dynamically at runtime using the `OPENEVT_PLUGIN_PATH` environment variable.

### 🐧 Linux

On Linux, shared library dependencies (such as FFmpeg) are **not bundled** with the release binaries because they are trivially installed via your system's package manager.

#### 1. Install FFmpeg Shared Libraries

Install the FFmpeg development / shared libraries for your distribution:

* **Arch Linux / Manjaro:**
```bash
sudo pacman -S ffmpeg

```


* **Ubuntu / Debian:**
```bash
sudo apt update
sudo apt install libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev

```


* **Fedora / RHEL:**
```bash
sudo dnf install ffmpeg-free-devel

```



#### 2. Place Plugin & Set Environment Variable

1. Download or build `libopenevt_plugins.so` and place it in your designated plugins directory (e.g., `~/.local/lib/openevt/plugins/`).
2. Export `OPENEVT_PLUGIN_PATH` pointing to that directory:

* **Temporary (Current Terminal Session):**
```bash
export OPENEVT_PLUGIN_PATH="$HOME/.local/lib/openevt/plugins"

```


* **Persistent (`~/.bashrc` or `~/.zshrc`):**
```bash
echo 'export OPENEVT_PLUGIN_PATH="$HOME/.local/lib/openevt/plugins"' >> ~/.bashrc
source ~/.bashrc

```



---

### 🪟 Windows

On Windows, `openevt_plugins.dll` must be placed in the same folder as the required FFmpeg 7.1 shared dynamic libraries (`.dll`), and the directory must be added to your system `PATH`.

#### 1. Download Required Files

1. Download `openevt_plugins.dll` from the [Releases](https://www.google.com/search?q=https://github.com/your-org/openevt-plugins/releases) page.
2. Download an **LGPL-licensed FFmpeg 7.1 Shared Release** archive (e.g., `ffmpeg-master-latest-win64-lgpl-shared.zip` from BtbN or Gyan.dev).
> **Note:** Ensure you download the **Shared** build containing `.dll` files (`avcodec-61.dll`, `avutil-59.dll`, etc.), not a static executable build (`.exe`).



#### 2. Directory Layout

Extract `openevt_plugins.dll` and the FFmpeg shared `.dll` files into a single dedicated folder (e.g., `C:\OpenEVT\plugins\`):

```text
C:\OpenEVT\plugins\
├── openevt_plugins.dll
├── avcodec-61.dll
├── avdevice-61.dll
├── avfilter-10.dll
├── avformat-61.dll
├── avutil-59.dll
├── swresample-5.dll
└── swscale-8.dll

```

#### 3. Configure Environment Variables

Set `OPENEVT_PLUGIN_PATH` to point to your plugin directory, and append that directory to your system `PATH` so the Windows dynamic loader can resolve `avcodec-61.dll` and related libraries:

* **PowerShell (Current Session):**
```powershell
$env:OPENEVT_PLUGIN_PATH="C:\OpenEVT\plugins"
$env:PATH="C:\OpenEVT\plugins;" + $env:PATH

```


* **System Properties (Persistent):**
1. Open **System Properties** → **Environment Variables**.
2. Under **System Variables**, create a new variable:
* **Variable Name:** `OPENEVT_PLUGIN_PATH`
* **Variable Value:** `C:\OpenEVT\plugins`


3. Select the existing `Path` variable, click **Edit**, and add: `C:\OpenEVT\plugins`



---

## 📄 License & Third-Party Notices

This repository is primary-licensed under the [MIT License](https://www.google.com/search?q=LICENSE).

When dynamically linked against FFmpeg binaries, interactions are governed by the **GNU Lesser General Public License (LGPL) v2.1**. See [THIRD_PARTY_LICENSES.md](https://www.google.com/search?q=THIRD_PARTY_LICENSES.md) for full compliance notices and license texts.
