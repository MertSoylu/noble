@echo off
rem Builds the working tree and installs it as `noble-dev`, next to (never over)
rem a stable `noble` from `cargo install noble` or a release. Works while
rem noble-dev.exe is running: Windows refuses to overwrite a running .exe but
rem allows renaming it.
setlocal
if defined CARGO_HOME (set "BIN=%CARGO_HOME%\bin") else (set "BIN=%USERPROFILE%\.cargo\bin")
cargo build --release --quiet --manifest-path "%~dp0Cargo.toml"
if errorlevel 1 (
  echo NOBLE build failed.
  exit /b 1
)
rem Where cargo put it: CARGO_TARGET_DIR or build.target-dir may move it away from target\.
set "TARGET=%~dp0target"
for /f "usebackq delims=" %%t in (`powershell -NoProfile -Command "(cargo metadata --format-version 1 --no-deps --manifest-path '%~dp0Cargo.toml' | ConvertFrom-Json).target_directory"`) do set "TARGET=%%t"
if not exist "%BIN%" mkdir "%BIN%"
rem A free name to move the running binary aside to: an older one may still be running in another
rem window (it cannot be deleted then), so numbered names are tried, as `noble update` does.
set "OLD="
for %%n in (old old2 old3 old4 old5 old6 old7 old8 old9) do (
  if not defined OLD (
    del /q "%BIN%\noble-dev.%%n.exe" 2>nul
    if not exist "%BIN%\noble-dev.%%n.exe" set "OLD=%BIN%\noble-dev.%%n.exe"
  )
)
if not defined OLD (
  echo NOBLE install failed: close some noble-dev windows and try again.
  exit /b 1
)
if exist "%BIN%\noble-dev.exe" move /y "%BIN%\noble-dev.exe" "%OLD%" >nul
copy /y "%TARGET%\release\noble.exe" "%BIN%\noble-dev.exe" >nul
if errorlevel 1 (
  if exist "%OLD%" move /y "%OLD%" "%BIN%\noble-dev.exe" >nul
  echo NOBLE install failed.
  exit /b 1
)
for /f "delims=" %%v in ('"%BIN%\noble-dev.exe" --version') do echo Installed: %%v  ^(run: noble-dev^)
