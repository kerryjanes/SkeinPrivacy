@echo off
setlocal

set "WEFT_RAW_BASE=https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"
set "WEFT_DIR=%USERPROFILE%\.weft"
set "SCRIPT_DIR=%~dp0"
set "PS1_LOCAL=%SCRIPT_DIR%run-node.ps1"
set "PS1_CACHE=%WEFT_DIR%\run-node.ps1"

if not exist "%WEFT_DIR%" mkdir "%WEFT_DIR%" >NUL 2>NUL

if /I "%~1"=="allow-defender-admin" (
  echo -^> adding Windows Defender exclusion for %WEFT_DIR%
  powershell -NoProfile -ExecutionPolicy Bypass -Command "Add-MpPreference -ExclusionPath '%WEFT_DIR%'"
  if errorlevel 1 (
    echo Failed to add the Defender exclusion.
    echo Open Command Prompt as Administrator and run: weft-node.cmd allow-defender
    exit /b %ERRORLEVEL%
  )
  echo OK>"%WEFT_DIR%\defender-exclusion.ok"
  echo OK: Windows Defender exclusion added for %WEFT_DIR%
  exit /b 0
)

if /I "%~1"=="allow-defender" (
  net session >NUL 2>NUL
  if not errorlevel 1 (
    call "%~f0" allow-defender-admin
    exit /b %ERRORLEVEL%
  )
  echo -^> opening elevated Command Prompt for Defender exclusion
  echo -^> approve the UAC prompt; the elevated window closes automatically
  powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath $env:ComSpec -Verb RunAs -Wait -ArgumentList '/c ""%~f0"" allow-defender-admin'"
  exit /b %ERRORLEVEL%
)

if /I not "%~1"=="stop" if /I not "%~1"=="--stop" if not exist "%WEFT_DIR%\defender-exclusion.ok" (
  call "%~f0" allow-defender
  if errorlevel 1 (
    echo WARNING: Defender exclusion was not added automatically; continuing anyway.
  )
)

if exist "%PS1_LOCAL%" (
  set "PS1=%PS1_LOCAL%"
) else (
  set "PS1=%PS1_CACHE%"
)

if not exist "%PS1_LOCAL%" (
  echo -^> downloading Windows runner...
  powershell -NoProfile -ExecutionPolicy Bypass -Command "[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -UseBasicParsing '%WEFT_RAW_BASE%/scripts/run-node.ps1' -OutFile '%PS1_CACHE%'"
  if errorlevel 1 exit /b %ERRORLEVEL%
  set "PS1=%PS1_CACHE%"
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%PS1%" %*
exit /b %ERRORLEVEL%
