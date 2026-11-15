$ErrorActionPreference = 'Stop'
$appName = if ([string]::IsNullOrWhiteSpace($env:APP_NAME)) { 'VOID Engine' } else { $env:APP_NAME.Trim() }
$appSlug = if ([string]::IsNullOrWhiteSpace($env:APP_SLUG)) { 'void' } else { $env:APP_SLUG.Trim() }
$version = if ([string]::IsNullOrWhiteSpace($env:APP_VERSION)) { '1.0.0' } else { $env:APP_VERSION.Trim() }
if ($appSlug -notmatch '^[a-z0-9][a-z0-9_+-]*$') { throw "Invalid package name: $appSlug" }
$wixVersion = ($version -split '[-+]', 2)[0]
if ($wixVersion -notmatch '^\d{1,5}\.\d{1,5}\.\d{1,5}$') { throw "Invalid MSI version: $version" }

$wixBin = if (Get-Command candle.exe -ErrorAction SilentlyContinue) {
  Split-Path (Get-Command candle.exe).Source
} else {
  Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'WiX Toolset v3.14/bin'
}
$candle = Join-Path $wixBin 'candle.exe'
$light = Join-Path $wixBin 'light.exe'
if (-not (Test-Path -LiteralPath $candle) -or -not (Test-Path -LiteralPath $light)) {
  throw 'WiX Toolset 3.14 candle.exe/light.exe are required to package the Windows MSI'
}

$md5 = [System.Security.Cryptography.MD5]::Create()
try {
  $upgradeCode = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($appSlug))).ToString().ToUpperInvariant()
  $exeCompGuid = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:exe"))).ToString().ToUpperInvariant()
  $cfgCompGuid = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:config"))).ToString().ToUpperInvariant()
} finally {
  $md5.Dispose()
}

$outputDir = Join-Path $PWD.Path 'output'
$msiPath = Join-Path $outputDir "$appSlug.msi"
New-Item -ItemType Directory -Force $outputDir | Out-Null

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "void-msi-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force $tempDir | Out-Null
try {
  $binaryPath = Join-Path $tempDir "$appSlug.exe"
  $configPath = Join-Path $tempDir "$appSlug.json"
  $iconPath = Join-Path $tempDir 'app.ico'

  Copy-Item -Force $env:MASTER_BINARY_PATH $binaryPath
  Copy-Item -Force 'void.json' $configPath
  if (-not (Test-Path -LiteralPath 'branding/app.ico')) {
    node scripts/prepare-branding.mjs
  }
  Copy-Item -Force 'branding/app.ico' $iconPath
  & (Join-Path $PSScriptRoot 'set-windows-icon.ps1') -Executable $binaryPath -IconFile $iconPath

  $dialogBmp = Join-Path $PWD.Path 'branding/wix-dialog.bmp'
  $bannerBmp = Join-Path $PWD.Path 'branding/wix-banner.bmp'
  $wixUiVars = ''
  if ((Test-Path -LiteralPath $dialogBmp) -and (Test-Path -LiteralPath $bannerBmp)) {
    $escDialog = [System.Security.SecurityElement]::Escape($dialogBmp)
    $escBanner = [System.Security.SecurityElement]::Escape($bannerBmp)
    $wixUiVars = @"
    <WixVariable Id="WixUIDialogBmp" Value="$escDialog" />
    <WixVariable Id="WixUIBannerBmp" Value="$escBanner" />
"@
  }

  $escAppName = [System.Security.SecurityElement]::Escape($appName)
  $escAppSlug = [System.Security.SecurityElement]::Escape($appSlug)
  $escVersion = [System.Security.SecurityElement]::Escape($wixVersion)
  $escBinary = [System.Security.SecurityElement]::Escape($binaryPath)
  $escConfig = [System.Security.SecurityElement]::Escape($configPath)
  $escIcon = [System.Security.SecurityElement]::Escape($iconPath)

  $wxs = @"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="$escAppName" Language="1033" Version="$escVersion" Manufacturer="$escAppName" UpgradeCode="$upgradeCode">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perUser" Platform="x64" />
    <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
    <Media Id="1" Cabinet="cab1.cab" EmbedCab="yes" CompressionLevel="none" />
    <Icon Id="ProductIcon" SourceFile="$escIcon" />
    <Property Id="ARPPRODUCTICON" Value="ProductIcon" />
    <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
$wixUiVars
    <UIRef Id="WixUI_InstallDir" />
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="INSTALLFOLDER" Name="$escAppSlug" />
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Directory Id="StartMenuDir" Name="$escAppName" />
      </Directory>
      <Directory Id="DesktopFolder" Name="Desktop" />
    </Directory>
    <Feature Id="MainFeature" Title="$escAppName" Level="1">
      <ComponentRef Id="ApplicationExeComponent" />
      <ComponentRef Id="ApplicationConfigComponent" />
      <ComponentRef Id="StartMenuCleanup" />
      <ComponentRef Id="DesktopShortcutComponent" />
    </Feature>
    <DirectoryRef Id="INSTALLFOLDER">
      <Component Id="ApplicationExeComponent" Guid="$exeCompGuid" Win64="yes">
        <File Id="ApplicationExeFile" Source="$escBinary" Name="$escAppSlug.exe" />
        <RegistryValue Id="RegExeVal" Root="HKCU" Key="Software\VOID Engine\$escAppSlug" Name="Executable" Type="integer" Value="1" KeyPath="yes" />
      </Component>
      <Component Id="ApplicationConfigComponent" Guid="$cfgCompGuid" Win64="yes">
        <File Id="ApplicationConfigFile" Source="$escConfig" Name="$escAppSlug.json" />
        <RegistryValue Id="RegCfgVal" Root="HKCU" Key="Software\VOID Engine\$escAppSlug" Name="Config" Type="integer" Value="1" KeyPath="yes" />
        <RemoveFolder Id="RemoveInstallFolder" Directory="INSTALLFOLDER" On="uninstall" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="StartMenuDir">
      <Component Id="StartMenuCleanup" Guid="*" Win64="yes">
        <Shortcut Id="ApplicationShortcut" Name="$escAppName" Target="[#ApplicationExeFile]" WorkingDirectory="INSTALLFOLDER" Icon="ProductIcon" IconIndex="0" />
        <RemoveFolder Id="RemoveStartMenuFolder" On="uninstall" />
        <RegistryValue Id="RegMenuVal" Root="HKCU" Key="Software\VOID Engine\$escAppSlug" Name="Installed" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="DesktopFolder">
      <Component Id="DesktopShortcutComponent" Guid="*" Win64="yes">
        <Shortcut Id="DesktopShortcut" Name="$escAppName" Target="[#ApplicationExeFile]" WorkingDirectory="INSTALLFOLDER" Icon="ProductIcon" IconIndex="0" />
        <RegistryValue Id="RegDeskVal" Root="HKCU" Key="Software\VOID Engine\$escAppSlug" Name="DesktopShortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
  </Product>
</Wix>
"@

  $wxsPath = Join-Path $tempDir 'package.wxs'
  $wixobjPath = Join-Path $tempDir 'package.wixobj'
  Set-Content -LiteralPath $wxsPath -Value $wxs -Encoding utf8

  & $candle -nologo -ext WixUIExtension -out $wixobjPath $wxsPath
  if ($LASTEXITCODE -ne 0) { throw "WiX candle failed with exit code $LASTEXITCODE" }
  & $light -nologo -sval -ext WixUIExtension -out $msiPath $wixobjPath
  if ($LASTEXITCODE -ne 0) { throw "WiX light failed with exit code $LASTEXITCODE" }
  Write-Host "Packaged $appName as $msiPath"
} finally {
  Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
}
