@echo off
rem Builds Raketo for Windows: portable exe and installer. Needs Rust (MSVC) and Node.js.
setlocal
cd /d "%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if not exist node_modules call npm ci || exit /b 1
call npx tauri build --bundles nsis || exit /b 1
echo.
echo Portable:  target\release\raketo.exe
echo Installer: target\release\bundle\nsis\
