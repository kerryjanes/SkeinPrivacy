@echo off
setlocal

set "WEFT_RAW_BASE=https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"
set "WEFT_DIR=%USERPROFILE%\.weft"
set "SCRIPT_DIR=%~dp0"
set "PS1_LOCAL=%SCRIPT_DIR%run-node.ps1"
set "PS1_CACHE=%WEFT_DIR%\run-node.ps1"

if not exist "%WEFT_DIR%" mkdir "%WEFT_DIR%" >NUL 2>NUL

if exist "%PS1_LOCAL%" (
  set "PS1=%PS1_LOCAL%"
) else (
  set "PS1=%PS1_CACHE%"
)

if not exist "%PS1%" (
  echo -^> downloading Windows runner...
  powershell -NoProfile -ExecutionPolicy Bypass -Command "[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -UseBasicParsing '%WEFT_RAW_BASE%/scripts/run-node.ps1' -OutFile '%PS1_CACHE%'"
  if errorlevel 1 exit /b %ERRORLEVEL%
  set "PS1=%PS1_CACHE%"
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%PS1%" %*
exit /b %ERRORLEVEL%
