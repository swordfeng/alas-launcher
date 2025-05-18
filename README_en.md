**| English | [简体中文](README.md) |**
ALAS Launcher: A New Type of [AzurLaneAutoScript](https://github.com/LmeSzinc/AzurLaneAutoScript) Launcher
===
Background: Since getting a Mac Mini, I've been too lazy to press the power button on my PC. But it feels wrong not running ALAS...

This [blog post](https://www.binss.me/blog/run-azurlaneautoscript-on-arm64/) by binss was very inspiring,
but the methods used either rely on translation layer or Docker containers. As a native purist, I really don't want to run user applications in containers, nor do I want to mess up my system environment. So why not run ALAS natively on MacOS, on Apple Silicon?

Thus this repo was born.

Simple Usage Instructions
---
Go to Releases on the right, download the archive for your system and CPU architecture, and extract it.
- Windows: Run `alas-launcher.exe`
- MacOS: Open `AzurLaneAutoScript.app`. If there's an error, open Terminal and run `xattr -dr com.apple.quarantine AzurLaneAutoScript.app` (because I don't have an Apple developer certificate to sign the program)
- Linux: Run `alas-launcher`. Note that the program depends on `libwebkit2gtk-4.1` and a recent `glibc` (CI runs on Ubuntu 22.04). If you don't have these, the launcher might not work, but ALAS itself should run fine

License
---
Since ALAS uses GPLv3, we use GPLv3 too. Most dependencies use Apache2, BSD3, etc. - please check upstream repos for details.

Differences from Original Version
---
1. Cross-platform, of course.
2. The original launcher updates git repo, kills existing processes, updates pip, updates electron resources, and restarts adb on startup. This version only updates the repo. If launched multiple times, it only refocuses the existing window.
3. Python package versions differ from original, but it works fine. Automatic pip updates are disabled. If upstream adds a requirements file, pip updates can be implemented.
4. Restarting and replacing adb is tricky, not implemented.
5. Directory structure has been modified slightly.

Technical Details
---
1. Compiled MXNet. Since PyPI versions don't work, had to compile it myself. MXNet's CMake is... challenging, and I had to add some patches. Achieved backwards-compatible builds for all platforms. See https://github.com/swordfeng/mxnet-build.
2. Used `uv` to download portable Python, so it can run anywhere.
3. Updated many Python package versions since lots of packages can't compile on arm64 Mac. See `requirements.in`.
4. Following binss's blog, chose MXNet 1.9.1 and a newer NumPy version. Interestingly, this NumPy version removed `np.bool`, so monkey-patched MXNet to add this type back.
5. Since cnocr only accepts mxnet \[1.5.0, 1.7.0\), modified the version when packaging.
6. Used Tauri for the shell. Original GUI's Electron could probably work on Mac, but it looked messy so I gave up after brief research.
7. Packaging scripts, all on GitHub Actions, see `.github/workflows`.
8. Removed some duplicate files. Not sure why *-nix symlinks were all packed as copies, or if it was due to `cp` with hardlinks? Anyway, just deduped with hardlinks. Too lazy to investigate deeper compression.

Directory Structure
---
ALAS Root Directory
* Windows: AzurLaneAutoScript
* MacOS: AzurLaneAutoScript.app/Contents/AzurLaneAutoScript
* Linux: AzurLaneAutoScript
ALAS Launcher
* Windows: AzurLaneAutoScript/alas-launcher.exe
* MacOS: AzurLaneAutoScript.app/Contents/MacOS/alas-launcher
* Linux: AzurLaneAutoScript/alas-launcher
Python
* All systems: toolkit (similar to venv structure)
Git
* Unix: Installed with Unix directory structure to toolkit
* Windows: MinGit extracted to toolkit/git
Adb
* Unix: toolkit/bin/adb
* Windows: toolkit/adb.exe
Environment Variables Added by Launcher
* Unix:
  - toolbox/bin
  - toolbox/libexec/git-core
  - toolbox/lib (LD_LIBRARY_PATH)
* Windows:
  - toolbox
  - toolbox/Scripts
  - toolbox/git/cmd