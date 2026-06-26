param(
  [Parameter(Position = 0)]
  [string]$NodeKey,
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Rest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Sk = Join-Path $HOME ".weft"
$StartupDir = [Environment]::GetFolderPath("Startup")
$StartupCmd = Join-Path $StartupDir "weft-node-start.cmd"

function Info([string]$Message) {
  Write-Host "-> $Message"
}

function Ensure-Dir([string]$Path) {
  if (!(Test-Path -LiteralPath $Path)) {
    New-Item -ItemType Directory -Path $Path | Out-Null
  }
}

function Download-File([string]$Url, [string]$OutFile) {
  Info "downloading $Url"
  Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $OutFile
}

function Test-Executable([string]$ExePath, [string]$Arg, [string]$Name) {
  if (!(Test-Path -LiteralPath $ExePath)) { return $false }
  $Stdout = Join-Path $Sk "$Name.check.out.log"
  $Stderr = Join-Path $Sk "$Name.check.err.log"
  try {
    $Process = Start-Process -FilePath $ExePath -ArgumentList $Arg -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput $Stdout -RedirectStandardError $Stderr
    return ($Process.ExitCode -eq 0)
  } catch {
    $_.Exception.Message | Set-Content -LiteralPath $Stderr -Encoding UTF8
    return $false
  }
}

function Expand-Zip([string]$ZipPath, [string]$Destination) {
  if (Test-Path -LiteralPath $Destination) {
    Remove-Item -LiteralPath $Destination -Recurse -Force
  }
  Ensure-Dir $Destination
  Expand-Archive -LiteralPath $ZipPath -DestinationPath $Destination -Force
}

function Stop-PidFile([string]$PidFile) {
  if (Test-Path -LiteralPath $PidFile) {
    $PidText = (Get-Content -LiteralPath $PidFile -Raw).Trim()
    if ($PidText -match '^\d+$') {
      try {
        & taskkill.exe /PID $PidText /T /F *> $null
      } catch {
        # Stale PID files are expected after crashes, reboots, or manual task kills.
      }
    }
    Remove-Item -LiteralPath $PidFile -Force -ErrorAction SilentlyContinue
  }
}

function Stop-WeftWorkspaceProcesses {
  if (!(Test-Path -LiteralPath $Sk)) { return }
  try {
    $Processes = Get-CimInstance Win32_Process | Where-Object {
      $_.CommandLine -and
      $_.CommandLine.Contains($Sk) -and
      ($_.Name -match '^(cmd\.exe|node\.exe|frpc\.exe|xray.*\.exe)$')
    }
    foreach ($Process in $Processes) {
      try {
        & taskkill.exe /PID $Process.ProcessId /T /F *> $null
      } catch {
        # Best-effort cleanup only.
      }
    }
  } catch {
    # Some locked-down Windows installs restrict process command-line reads.
  }
}

function Stop-WeftNode([bool]$Purge) {
  Info "stopping Weft node"
  Stop-PidFile (Join-Path $Sk "control-plane.pid")
  Stop-PidFile (Join-Path $Sk "frpc.pid")
  Stop-PidFile (Join-Path $Sk "xray.pid")
  Stop-WeftWorkspaceProcesses
  Start-Sleep -Milliseconds 500
  if ($Purge -and (Test-Path -LiteralPath $StartupCmd)) {
    Remove-Item -LiteralPath $StartupCmd -Force
  }
  if ($Purge -and (Test-Path -LiteralPath $Sk)) {
    Remove-Item -LiteralPath $Sk -Recurse -Force
    Write-Host "OK: node stopped and purged ($Sk removed)."
  } else {
    Write-Host "OK: node stopped. Config remains in $Sk; run weft-node.cmd to start again."
  }
}

function Start-CmdProcess([string]$CmdPath, [string]$PidFile) {
  Stop-PidFile $PidFile
  $Process = Start-Process -FilePath $env:ComSpec -ArgumentList "/c `"$CmdPath`"" -WindowStyle Hidden -PassThru
  Set-Content -LiteralPath $PidFile -Value $Process.Id -Encoding ASCII
}

function Assert-ProcessAlive([string]$PidFile, [string]$Name, [string]$LogFile) {
  if (!(Test-Path -LiteralPath $PidFile)) {
    throw "$Name did not start. Missing pid file. Log: $LogFile"
  }
  $PidText = (Get-Content -LiteralPath $PidFile -Raw).Trim()
  if (!($PidText -match '^\d+$')) {
    throw "$Name did not start. Bad pid file: $PidFile. Log: $LogFile"
  }
  $Process = Get-Process -Id ([int]$PidText) -ErrorAction SilentlyContinue
  if (!$Process) {
    Write-Host ""
    Write-Host "$Name failed to stay running. Last log lines:"
    if (Test-Path -LiteralPath $LogFile) {
      Get-Content -LiteralPath $LogFile -Tail 40
    } else {
      Write-Host "(missing log: $LogFile)"
    }
    throw "$Name is not running"
  }
}

function Ps-Literal([string]$Value) {
  return "'" + $Value.Replace("'", "''") + "'"
}

if ($NodeKey -eq "stop" -or $NodeKey -eq "--stop") {
  Stop-WeftNode ($Rest -contains "--purge")
  exit 0
}

Ensure-Dir $Sk

function Get-EnvOrDefault([string]$Name, [string]$Default) {
  $Value = [Environment]::GetEnvironmentVariable($Name)
  if ([string]::IsNullOrWhiteSpace($Value)) { return $Default }
  return $Value
}

function Decode-NodeKey([string]$Token) {
  $B64 = $Token.Replace("-", "+").Replace("_", "/")
  switch ($B64.Length % 4) {
    0 { }
    2 { $B64 += "==" }
    3 { $B64 += "=" }
    default { throw "node key has invalid base64url length" }
  }
  $Json = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($B64))
  $Data = $Json | ConvertFrom-Json
  foreach ($Key in @("realityPriv", "realityPub", "uuid", "sid", "port", "relay", "geo")) {
    if ($null -eq $Data.$Key -or "$($Data.$Key)" -eq "") {
      throw "node key is missing '$Key' - copy it again from cabinet -> deploy"
    }
  }
  return $Data
}

function Detect-PhysicalEgressIp {
  $ExplicitIp = [Environment]::GetEnvironmentVariable("WEFT_XRAY_SEND_THROUGH")
  if (![string]::IsNullOrWhiteSpace($ExplicitIp)) { return $ExplicitIp }

  $Interface = [Environment]::GetEnvironmentVariable("WEFT_EGRESS_INTERFACE")
  if ([string]::IsNullOrWhiteSpace($Interface)) { return "" }
  if ($Interface -eq "none") { return "" }

  try {
    $Configs = Get-NetIPConfiguration | Where-Object {
      $_.IPv4DefaultGateway -and
      $_.IPv4Address -and
      $_.NetAdapter.Status -eq "Up"
    }
    if (![string]::IsNullOrWhiteSpace($Interface) -and $Interface -ne "auto") {
      $Configs = $Configs | Where-Object { $_.InterfaceAlias -eq $Interface -or $_.InterfaceDescription -like "*$Interface*" }
    } else {
      $Configs = $Configs | Where-Object {
        $_.InterfaceAlias -notmatch "VPN|TAP|TUN|WireGuard|Tailscale|Loopback|vEthernet|Docker|Hyper-V"
      }
    }
    $First = $Configs | Select-Object -First 1
    if ($First) { return $First.IPv4Address.IPAddress }
  } catch {
    return ""
  }
  return ""
}

$NodeJson = Join-Path $Sk "node.json"
if (![string]::IsNullOrWhiteSpace($NodeKey)) {
  $Data = Decode-NodeKey $NodeKey
  $Data | ConvertTo-Json -Compress | Set-Content -LiteralPath $NodeJson -Encoding ASCII
} elseif (![string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable("WEFT_NODE_KEY"))) {
  $Data = Decode-NodeKey ([Environment]::GetEnvironmentVariable("WEFT_NODE_KEY"))
  $Data | ConvertTo-Json -Compress | Set-Content -LiteralPath $NodeJson -Encoding ASCII
} elseif (!(Test-Path -LiteralPath $NodeJson)) {
  Write-Host "No node key found. Register your device in cabinet -> deploy, copy the key, then run:"
  Write-Host "  weft-node.cmd <your-node-key>"
  exit 1
}

$Node = Get-Content -LiteralPath $NodeJson -Raw | ConvertFrom-Json

$Relay = Get-EnvOrDefault "WEFT_RELAY" "$($Node.relay)"
$RelayPort = Get-EnvOrDefault "WEFT_RELAY_PORT" "7000"
$RelayToken = Get-EnvOrDefault "WEFT_RELAY_TOKEN" "a40b1ab498a37ba6bbaa70791ac62287"
$Sni = Get-EnvOrDefault "WEFT_SNI" "ya.ru"
$Cluster = Get-EnvOrDefault "WEFT_CLUSTER" "devnet"
$Rpc = Get-EnvOrDefault "WEFT_RPC" "https://api.devnet.solana.com"
$Mint = Get-EnvOrDefault "WEFT_MINT" "Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy"
$Raw = Get-EnvOrDefault "WEFT_RAW_BASE" "https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"
$LocalHop1 = 14430
$FrpVer = Get-EnvOrDefault "WEFT_FRP_VERSION" "0.69.1"
$NodeVer = Get-EnvOrDefault "WEFT_NODE_VERSION" "22.13.1"

if ($Cluster.StartsWith("mainnet")) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable("WEFT_RPC"))) {
    throw "WEFT_RPC must be set explicitly for $Cluster"
  }
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable("WEFT_MINT"))) {
    throw "WEFT_MINT must be set explicitly for $Cluster"
  }
}

$NodeDir = Join-Path $Sk "node-v$NodeVer-win-x64"
$NodeExe = Join-Path $NodeDir "node.exe"
if (!(Test-Path -LiteralPath $NodeExe)) {
  $NodeZip = Join-Path $Sk "node.zip"
  $NodeExtract = Join-Path $Sk "node.extract"
  Download-File "https://nodejs.org/dist/v$NodeVer/node-v$NodeVer-win-x64.zip" $NodeZip
  Expand-Zip $NodeZip $NodeExtract
  if (Test-Path -LiteralPath $NodeDir) {
    Remove-Item -LiteralPath $NodeDir -Recurse -Force
  }
  Move-Item -LiteralPath (Join-Path $NodeExtract "node-v$NodeVer-win-x64") -Destination $NodeDir
  Remove-Item -LiteralPath $NodeZip -Force
  Remove-Item -LiteralPath $NodeExtract -Recurse -Force
}

$XrayExe = Join-Path $Sk "xray-1.8.24.exe"
if (!(Test-Path -LiteralPath $XrayExe)) {
  $XrayZip = Join-Path $Sk "xray.zip"
  $XrayExtract = Join-Path $Sk "xray-1.8.24.extract"
  Download-File "https://github.com/XTLS/Xray-core/releases/download/v1.8.24/Xray-windows-64.zip" $XrayZip
  Expand-Zip $XrayZip $XrayExtract
  Copy-Item -LiteralPath (Join-Path $XrayExtract "xray.exe") -Destination $XrayExe -Force
  Remove-Item -LiteralPath $XrayZip -Force
  Remove-Item -LiteralPath $XrayExtract -Recurse -Force
}

$FrpcExe = Join-Path $Sk "frpc.exe"
if ((Test-Path -LiteralPath $FrpcExe) -and !(Test-Executable $FrpcExe "--version" "frpc")) {
  Info "existing frpc.exe is not executable; reinstalling it"
  Remove-Item -LiteralPath $FrpcExe -Force -ErrorAction SilentlyContinue
}
if (!(Test-Path -LiteralPath $FrpcExe)) {
  $FrpZip = Join-Path $Sk "frp.zip"
  $FrpExtract = Join-Path $Sk "frp.extract"
  Download-File "https://github.com/fatedier/frp/releases/download/v$FrpVer/frp_${FrpVer}_windows_amd64.zip" $FrpZip
  try {
    Expand-Zip $FrpZip $FrpExtract
    Copy-Item -LiteralPath (Join-Path $FrpExtract "frp_${FrpVer}_windows_amd64\frpc.exe") -Destination $FrpcExe -Force
  } catch {
    Write-Host ""
    Write-Host "Windows blocked frpc.exe while extracting it."
    Write-Host "Open Windows Security -> Virus & threat protection -> Protection history and allow frpc.exe,"
    Write-Host "or stay in Command Prompt and run:"
    Write-Host "  weft-node.cmd allow-defender"
    Write-Host "  weft-node.cmd stop --purge"
    Write-Host "  weft-node.cmd <your-node-key>"
    throw
  } finally {
    if (Test-Path -LiteralPath $FrpZip) { Remove-Item -LiteralPath $FrpZip -Force }
    if (Test-Path -LiteralPath $FrpExtract) { Remove-Item -LiteralPath $FrpExtract -Recurse -Force }
  }
}
if (!(Test-Executable $FrpcExe "--version" "frpc")) {
  Write-Host ""
  Write-Host "frpc.exe is present but Windows cannot run it."
  Write-Host "Most common causes: Windows Defender quarantined it, the file is corrupted, or this Windows is not x64."
  Write-Host "Open Windows Security -> Virus & threat protection -> Protection history and allow frpc.exe,"
  Write-Host "or stay in Command Prompt and run:"
  Write-Host "  weft-node.cmd allow-defender"
  Write-Host "  weft-node.cmd stop --purge"
  Write-Host "  weft-node.cmd <your-node-key>"
  Write-Host "Diagnostic log: $Sk\frpc.check.err.log"
  throw "frpc.exe is not executable"
}

$ControlPlane = Join-Path $Sk "control-plane.mjs"
Download-File "$Raw/services/control-plane/dist/control-plane.mjs" $ControlPlane

$SendThrough = Detect-PhysicalEgressIp
if (![string]::IsNullOrWhiteSpace($SendThrough)) {
  Info "1-hop egress pinned to physical interface IP $SendThrough to bypass host VPN routes"
} else {
  Info "1-hop egress uses the system default route"
}

$FrpcToml = Join-Path $Sk "frpc.toml"
$XrayConfig = Join-Path $Sk "xray.json"
$NodeEnv = Join-Path $Sk "node.env"
$RunXray = Join-Path $Sk "run-xray.cmd"
$RunFrpc = Join-Path $Sk "run-frpc.cmd"
$RunCp = Join-Path $Sk "run-control-plane.cmd"
$RestartXray = Join-Path $Sk "restart-xray.ps1"
$XrayPid = Join-Path $Sk "xray.pid"
$FrpcPid = Join-Path $Sk "frpc.pid"
$CpPid = Join-Path $Sk "control-plane.pid"

@"
serverAddr = "$Relay"
serverPort = $RelayPort
auth.method = "token"
auth.token = "$RelayToken"
[[proxies]]
name = "weft-1hop-$($Node.uuid.Substring(0, 8))"
type = "tcp"
localIP = "127.0.0.1"
localPort = $LocalHop1
remotePort = $($Node.port)
"@ | Set-Content -LiteralPath $FrpcToml -Encoding ASCII

@"
WEFT_HOST=$Relay
WEFT_REALITY_PBK=$($Node.realityPub)
WEFT_REALITY_PRIV=$($Node.realityPriv)
WEFT_SID=$($Node.sid)
WEFT_SNI=$Sni
WEFT_HOP1_PORT=$LocalHop1
WEFT_HOPN_PORT=0
WEFT_PUBLIC_HOP1_PORT=$($Node.port)
WEFT_FOUNDER_UUID=$($Node.uuid)
WEFT_GEO=$($Node.geo)
WEFT_XRAY_CONFIG=$XrayConfig
WEFT_XRAY_BIN=$XrayExe
WEFT_XRAY_API=127.0.0.1:10085
WEFT_XRAY_RELOAD=powershell -NoProfile -ExecutionPolicy Bypass -File "$RestartXray"
WEFT_XRAY_SEND_THROUGH=$SendThrough
WEFT_STORE=$(Join-Path $Sk "users.json")
WEFT_PORT=8088
WEFT_CLUSTER=$Cluster
WEFT_RPC=$Rpc
WEFT_MINT=$Mint
WEFT_RELAY_TOKEN=$RelayToken
WEFT_RELAY_PROFILE_URL=https://${Relay}:8089/relay/node-profile
"@ | Set-Content -LiteralPath $NodeEnv -Encoding ASCII

@"
@echo off
"$XrayExe" run -c "$XrayConfig" >> "$Sk\xray.log" 2>&1
"@ | Set-Content -LiteralPath $RunXray -Encoding ASCII

@"
@echo off
"$FrpcExe" -c "$FrpcToml" >> "$Sk\frpc.log" 2>&1
"@ | Set-Content -LiteralPath $RunFrpc -Encoding ASCII

@"
@echo off
set WEFT_ENVFILE=$NodeEnv
"$NodeExe" "$ControlPlane" >> "$Sk\control-plane.log" 2>&1
"@ | Set-Content -LiteralPath $RunCp -Encoding ASCII

@"
`$ErrorActionPreference = "SilentlyContinue"
`$PidFile = $(Ps-Literal $XrayPid)
`$RunXray = $(Ps-Literal $RunXray)
if (Test-Path -LiteralPath `$PidFile) {
  `$PidText = (Get-Content -LiteralPath `$PidFile -Raw).Trim()
  if (`$PidText -match '^\d+`$') { & taskkill.exe /PID `$PidText /T /F *> `$null }
}
`$Args = '/c "' + `$RunXray + '"'
`$Process = Start-Process -FilePath `$env:ComSpec -ArgumentList `$Args -WindowStyle Hidden -PassThru
Set-Content -LiteralPath `$PidFile -Value `$Process.Id -Encoding ASCII
"@ | Set-Content -LiteralPath $RestartXray -Encoding ASCII

@"
@echo off
start "Weft Xray" /min cmd /c "$RunXray"
start "Weft frpc" /min cmd /c "$RunFrpc"
start "Weft control plane" /min cmd /c "$RunCp"
"@ | Set-Content -LiteralPath $StartupCmd -Encoding ASCII

Info "starting local services"
Start-CmdProcess $RunXray $XrayPid
Start-CmdProcess $RunFrpc $FrpcPid
Start-CmdProcess $RunCp $CpPid

Start-Sleep -Seconds 3
Assert-ProcessAlive $XrayPid "xray" "$Sk\xray.log"
Assert-ProcessAlive $FrpcPid "frpc" "$Sk\frpc.log"
Assert-ProcessAlive $CpPid "control-plane" "$Sk\control-plane.log"

Write-Host ""
Write-Host "OK: Weft 1-hop node is up (Windows background processes + Startup autostart)."
Write-Host "Public endpoint: ${Relay}:$($Node.port)   region: $($Node.geo)"
Write-Host "Logs: $Sk\xray.log, $Sk\frpc.log, $Sk\control-plane.log"
Write-Host "Stop: weft-node.cmd stop"
