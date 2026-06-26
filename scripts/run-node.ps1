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
$TaskPrefix = "WeftNode"
$XrayTask = "${TaskPrefix}Xray"
$FrpcTask = "${TaskPrefix}Frpc"
$CpTask = "${TaskPrefix}ControlPlane"

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

function Expand-Zip([string]$ZipPath, [string]$Destination) {
  if (Test-Path -LiteralPath $Destination) {
    Remove-Item -LiteralPath $Destination -Recurse -Force
  }
  Ensure-Dir $Destination
  Expand-Archive -LiteralPath $ZipPath -DestinationPath $Destination -Force
}

function Stop-TaskIfExists([string]$Name, [bool]$Delete) {
  & schtasks.exe /End /TN $Name *> $null
  if ($Delete) {
    & schtasks.exe /Delete /TN $Name /F *> $null
  }
}

function Stop-WeftNode([bool]$Purge) {
  Info "stopping Weft node"
  Stop-TaskIfExists $CpTask $Purge
  Stop-TaskIfExists $FrpcTask $Purge
  Stop-TaskIfExists $XrayTask $Purge
  if ($Purge -and (Test-Path -LiteralPath $Sk)) {
    Remove-Item -LiteralPath $Sk -Recurse -Force
    Write-Host "OK: node stopped and purged ($Sk removed)."
  } else {
    Write-Host "OK: node stopped. Config remains in $Sk; run .\weft-node.ps1 to start again."
  }
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
  Write-Host "  powershell -ExecutionPolicy Bypass -File .\weft-node.ps1 <your-node-key>"
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
    Write-Host "or run PowerShell as Administrator and add an exclusion:"
    Write-Host "  Add-MpPreference -ExclusionPath `"$Sk`""
    throw
  } finally {
    if (Test-Path -LiteralPath $FrpZip) { Remove-Item -LiteralPath $FrpZip -Force }
    if (Test-Path -LiteralPath $FrpExtract) { Remove-Item -LiteralPath $FrpExtract -Recurse -Force }
  }
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
$RestartXray = Join-Path $Sk "restart-xray.cmd"

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
WEFT_XRAY_RELOAD="$RestartXray"
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
@echo off
schtasks /End /TN "$XrayTask" >NUL 2>NUL
schtasks /Run /TN "$XrayTask" >NUL
"@ | Set-Content -LiteralPath $RestartXray -Encoding ASCII

Info "installing scheduled tasks"
foreach ($Task in @($CpTask, $FrpcTask, $XrayTask)) {
  & schtasks.exe /End /TN $Task *> $null
}
& schtasks.exe /Create /TN $XrayTask /SC ONLOGON /TR "`"$RunXray`"" /F | Out-Null
& schtasks.exe /Create /TN $FrpcTask /SC ONLOGON /TR "`"$RunFrpc`"" /F | Out-Null
& schtasks.exe /Create /TN $CpTask /SC ONLOGON /TR "`"$RunCp`"" /F | Out-Null

Info "starting relay tunnel and control plane"
& schtasks.exe /Run /TN $FrpcTask | Out-Null
& schtasks.exe /Run /TN $CpTask | Out-Null

Start-Sleep -Seconds 3

Write-Host ""
Write-Host "OK: Weft 1-hop node is up (Windows scheduled tasks)."
Write-Host "Public endpoint: ${Relay}:$($Node.port)   region: $($Node.geo)"
Write-Host "Logs: $Sk\xray.log, $Sk\frpc.log, $Sk\control-plane.log"
Write-Host "Stop: powershell -ExecutionPolicy Bypass -File .\weft-node.ps1 stop"
