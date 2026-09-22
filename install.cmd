@echo off
rem Builds and installs NOBLE. Works while noble.exe is running: Windows
rem refuses to overwrite a running .exe but allows renaming it.
set BIN=%USERPROFILE%\.cargo\bin
del /q "%BIN%\noble.old.exe" 2>nul
if exist "%BIN%\noble.exe" move /y "%BIN%\noble.exe" "%BIN%\noble.old.exe" >nul
cargo install --path "%~dp0." --quiet
if errorlevel 1 (
  if exist "%BIN%\noble.old.exe" if not exist "%BIN%\noble.exe" move /y "%BIN%\noble.old.exe" "%BIN%\noble.exe" >nul
  echo NOBLE install failed.
  exit /b 1
)
for /f "delims=" %%v in ('"%BIN%\noble.exe" --version') do echo Installed: %%v
