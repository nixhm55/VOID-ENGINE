$ErrorActionPreference = 'Stop'
$appName = if ([string]::IsNullOrWhiteSpace($env:APP_NAME)) { 'VOID Engine' } else { $env:APP_NAME.Trim() }
$appSlug = if ([string]::IsNullOrWhiteSpace($env:APP_SLUG)) { 'void' } else { $env:APP_SLUG.Trim() }
$version = if ([string]::IsNullOrWhiteSpace($env:APP_VERSION)) { '1.0.0' } else { $env:APP_VERSION.Trim() }
if ($appSlug -notmatch '^[a-z0-9][a-z0-9_+-]*$') { throw "Invalid package name: $appSlug" }
$wixVersion = ($version -split '[-+]', 2)[0]
if ($wixVersion -notmatch '^\d{1,5}\.\d{1,5}\.\d{1,5}$') { throw "Invalid MSI version: $version" }

$md5 = [System.Security.Cryptography.MD5]::Create()
try {
  $upgradeCode = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($appSlug))).ToString().ToUpperInvariant() + '}'
  $productCode = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:${wixVersion}:product"))).ToString().ToUpperInvariant() + '}'
  $packageCode = '{' + [Guid]::NewGuid().ToString().ToUpperInvariant() + '}'
  $exeCompGuid = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:exe"))).ToString().ToUpperInvariant() + '}'
  $cfgCompGuid = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:config"))).ToString().ToUpperInvariant() + '}'
} finally {
  $md5.Dispose()
}

$inputDir = Join-Path $PWD 'output/msi-input'
$binaryPath = Join-Path $inputDir "$appSlug.exe"
$configPath = Join-Path $inputDir "$appSlug.json"
$iconPath = Join-Path $inputDir 'app.ico'
$msiPath = Join-Path $PWD "output/$appSlug.msi"
$templateMsi = if ($env:MASTER_TEMPLATE_MSI) { $env:MASTER_TEMPLATE_MSI } else { 'target/master/void-template.msi' }

New-Item -ItemType Directory -Force $inputDir, (Join-Path $PWD 'output') | Out-Null
Copy-Item -Force $env:MASTER_BINARY_PATH $binaryPath
Copy-Item -Force 'void.json' $configPath
Copy-Item -Force 'branding/app.ico' $iconPath
& (Join-Path $PSScriptRoot 'set-windows-icon.ps1') -Executable $binaryPath -IconFile $iconPath

# If the pre-compiled master template MSI was not downloaded from the release yet, build it once
if (-not (Test-Path -LiteralPath $templateMsi)) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $templateMsi) | Out-Null
  & (Join-Path $PSScriptRoot 'build-windows-template-msi.ps1') -MasterExe $binaryPath -OutputMsi $templateMsi
}

Copy-Item -Force $templateMsi $msiPath

# Pack the icon-stamped executable and void.json into cab1.cab using native makecab.exe (Compress=OFF, MaxDiskSize=0)
$cabDir = Join-Path $inputDir 'cab'
New-Item -ItemType Directory -Force $cabDir | Out-Null
$ddfPath = Join-Path $cabDir 'payload.ddf'
$cabPath = Join-Path $cabDir 'cab1.cab'
$infPath = Join-Path $cabDir 'setup.inf'
$rptPath = Join-Path $cabDir 'setup.rpt'
$ddfContent = @"
.OPTION EXPLICIT
.Set CabinetNameTemplate=cab1.cab
.Set DiskDirectoryTemplate="$cabDir"
.Set DiskDirectory1="$cabDir"
.Set MaxDiskSize=0
.Set Cabinet=ON
.Set Compress=OFF
.Set InfFileName="$infPath"
.Set RptFileName="$rptPath"
"$binaryPath" ApplicationExeFile
"$configPath" ApplicationConfigFile
"@
Set-Content -LiteralPath $ddfPath -Value $ddfContent -Encoding ASCII
$makecabOut = & makecab.exe /F $ddfPath 2>&1
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $cabPath)) {
  throw "makecab.exe failed to build payload cabinet: $($makecabOut -join "`n")"
}

$exeSize = (Get-Item -LiteralPath $binaryPath).Length
$cfgSize = (Get-Item -LiteralPath $configPath).Length
$shortBase = ($appSlug -replace '[^a-zA-Z0-9]', '').ToUpperInvariant()
if ($shortBase.Length -gt 5) { $shortBase = $shortBase.Substring(0, 5) }
if ([string]::IsNullOrWhiteSpace($shortBase)) { $shortBase = 'VOID' }

# Directly patch the pre-compiled Master MSI database via native WindowsInstaller.Installer COM API
$installer = New-Object -ComObject WindowsInstaller.Installer
$db = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @($msiPath, 1))

function Invoke-MsiSql([string]$sql, [object[]]$params = @()) {
  $view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db, @($sql))
  if ($params.Count -gt 0) {
    $rec = $installer.GetType().InvokeMember('CreateRecord', 'InvokeMethod', $null, $installer, @($params.Count))
    for ($i = 0; $i -lt $params.Count; $i++) {
      $val = $params[$i]
      if ($val -is [int] -or $val -is [long]) {
        $null = $rec.GetType().InvokeMember('IntegerData', 'SetProperty', $null, $rec, @($i + 1, [int]$val))
      } else {
        $null = $rec.GetType().InvokeMember('StringData', 'SetProperty', $null, $rec, @($i + 1, [string]$val))
      }
    }
    $null = $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, @($rec))
  } else {
    $null = $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null)
  }
  $null = $view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null)
}

function Set-MsiStream([string]$sql, [string]$filePath) {
  $view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db, @($sql))
  $rec = $installer.GetType().InvokeMember('CreateRecord', 'InvokeMethod', $null, $installer, @(1))
  $null = $rec.GetType().InvokeMember('SetStream', 'InvokeMethod', $null, $rec, @(1, [string]$filePath))
  $null = $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, @($rec))
  $null = $view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null)
}

Invoke-MsiSql 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''ProductName''' @($appName)
Invoke-MsiSql 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''ProductVersion''' @($wixVersion)
Invoke-MsiSql 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''ProductCode''' @($productCode)
Invoke-MsiSql 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''UpgradeCode''' @($upgradeCode)
Invoke-MsiSql 'UPDATE `Upgrade` SET `UpgradeCode` = ?' @($upgradeCode)
Invoke-MsiSql 'UPDATE `Component` SET `ComponentId` = ? WHERE `Component` = ''ApplicationExeComponent''' @($exeCompGuid)
Invoke-MsiSql 'UPDATE `Component` SET `ComponentId` = ? WHERE `Component` = ''ApplicationConfigComponent''' @($cfgCompGuid)
Invoke-MsiSql 'UPDATE `Directory` SET `DefaultDir` = ? WHERE `Directory` = ''INSTALLFOLDER''' @("${shortBase}DIR|$appSlug")
Invoke-MsiSql 'UPDATE `Directory` SET `DefaultDir` = ? WHERE `Directory` = ''StartMenuDir''' @("${shortBase}MNU|$appName")
Invoke-MsiSql 'UPDATE `File` SET `FileName` = ?, `FileSize` = ? WHERE `File` = ''ApplicationExeFile''' @("${shortBase}EXE.EXE|$appSlug.exe", [int]$exeSize)
Invoke-MsiSql 'UPDATE `File` SET `FileName` = ?, `FileSize` = ? WHERE `File` = ''ApplicationConfigFile''' @("${shortBase}CFG.JSN|$appSlug.json", [int]$cfgSize)
Invoke-MsiSql 'UPDATE `Shortcut` SET `Name` = ? WHERE `Shortcut` = ''ApplicationShortcut''' @("${shortBase}LNK|$appName")
Invoke-MsiSql 'UPDATE `Shortcut` SET `Name` = ? WHERE `Shortcut` = ''DesktopShortcut''' @("${shortBase}DSK|$appName")
Invoke-MsiSql 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegExeVal''' @("Software\VOID Engine\$appSlug")
Invoke-MsiSql 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegCfgVal''' @("Software\VOID Engine\$appSlug")
Invoke-MsiSql 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegMenuVal''' @("Software\VOID Engine\$appSlug")
Invoke-MsiSql 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegDeskVal''' @("Software\VOID Engine\$appSlug")

Set-MsiStream 'UPDATE `Icon` SET `Data` = ? WHERE `Name` = ''ProductIcon''' $iconPath
Set-MsiStream 'UPDATE `_Streams` SET `Data` = ? WHERE `Name` = ''cab1.cab''' $cabPath

$summary = $db.GetType().InvokeMember('SummaryInformation', 'GetProperty', $null, $db, @(4))
$null = $summary.GetType().InvokeMember('Property', 'SetProperty', $null, $summary, @(2, $appName))
$null = $summary.GetType().InvokeMember('Property', 'SetProperty', $null, $summary, @(3, $appName))
$null = $summary.GetType().InvokeMember('Property', 'SetProperty', $null, $summary, @(9, $packageCode))
$null = $summary.GetType().InvokeMember('Persist', 'InvokeMethod', $null, $summary, $null)
$null = $db.GetType().InvokeMember('Commit', 'InvokeMethod', $null, $db, $null)

[System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($db) | Out-Null
[System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null

Remove-Item -Recurse -Force $inputDir -ErrorAction SilentlyContinue
Write-Host "Packaged $appName as $msiPath"
