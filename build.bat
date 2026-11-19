@echo off
rem Builds Raketo for Windows: installer, portable zip and the raketo command. Needs Rust (MSVC) and Node.js.
setlocal
cd /d "%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if not exist node_modules call npm ci || exit /b 1
cargo build -p raketo-cmd --release || exit /b 1
if not exist src-tauri\binaries mkdir src-tauri\binaries
copy /y target\release\raketo-cmd.exe src-tauri\binaries\raketo-cmd-x86_64-pc-windows-msvc.exe >nul
call npx tauri build --bundles nsis || exit /b 1
copy /y target\release\raketo-cmd.exe target\release\raketo.com >nul
powershell -NoProfile -Command "Compress-Archive -Force -Path target\release\raketo.exe,target\release\raketo.com -DestinationPath target\release\Raketo_portable.zip"
echo.
echo Installer: target\release\bundle\nsis\
echo Portable:  target\release\Raketo_portable.zip
