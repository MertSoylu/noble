@echo off
rem Builds the working tree and installs it as `noble-dev`, next to (never over)
rem a stable `noble` from `cargo install noble` or a release. Works while
rem noble-dev.exe is running: Windows refuses to overwrite a running .exe but
rem allows renaming it.
setlocal
set BIN=%USERPROFILE%\.cargo\bin
cargo build --release --quiet --manifest-path "%~dp0Cargo.toml"
if errorlevel 1 (
  echo NOBLE build failed.
  exit /b 1
)
if not exist "%BIN%" mkdir "%BIN%"
del /q "%BIN%\noble-dev.old.exe" 2>nul
if exist "%BIN%\noble-dev.exe" move /y "%BIN%\noble-dev.exe" "%BIN%\noble-dev.old.exe" >nul
copy /y "%~dp0target\release\noble.exe" "%BIN%\noble-dev.exe" >nul
if errorlevel 1 (
  if exist "%BIN%\noble-dev.old.exe" move /y "%BIN%\noble-dev.old.exe" "%BIN%\noble-dev.exe" >nul
  echo NOBLE install failed.
  exit /b 1
)
for /f "delims=" %%v in ('"%BIN%\noble-dev.exe" --version') do echo Installed: %%v  ^(run: noble-dev^)
