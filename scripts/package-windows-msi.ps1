$ErrorActionPreference = 'Stop'
$appName = if ([string]::IsNullOrWhiteSpace($env:APP_NAME)) { 'VOID Engine' } else { $env:APP_NAME.Trim() }
$appSlug = if ([string]::IsNullOrWhiteSpace($env:APP_SLUG)) { 'void' } else { $env:APP_SLUG.Trim() }
$version = if ([string]::IsNullOrWhiteSpace($env:APP_VERSION)) { '1.0.0' } else { $env:APP_VERSION.Trim() }
if ($appSlug -notmatch '^[a-z0-9][a-z0-9_+-]*$') { throw "Invalid package name: $appSlug" }
$wixVersion = ($version -split '[-+]', 2)[0]
if ($wixVersion -notmatch '^\d{1,5}\.\d{1,5}\.\d{1,5}$') { throw "Invalid MSI version: $version" }
$md5 = [System.Security.Cryptography.MD5]::Create()
try {
  $upgradeCode = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($appSlug))).ToString().ToUpperInvariant()
  $exeComponentGuid = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:exe"))).ToString().ToUpperInvariant()
  $configComponentGuid = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:config"))).ToString().ToUpperInvariant()
} finally {
  $md5.Dispose()
}

$inputDir = Join-Path $PWD 'output/msi-input'
$binaryPath = Join-Path $inputDir "$appSlug.exe"
$configPath = Join-Path $inputDir "$appSlug.json"
$iconPath = Join-Path $inputDir 'app.ico'
$dialogBmp = Join-Path $PWD 'branding/wix-dialog.bmp'
$bannerBmp = Join-Path $PWD 'branding/wix-banner.bmp'
New-Item -ItemType Directory -Force $inputDir, (Join-Path $PWD 'output') | Out-Null
Copy-Item -Force $env:MASTER_BINARY_PATH $binaryPath
Copy-Item -Force 'void.json' $configPath
Copy-Item -Force 'branding/app.ico' $iconPath
& (Join-Path $PSScriptRoot 'set-windows-icon.ps1') -Executable $binaryPath -IconFile $iconPath

$wixBin = if (Get-Command candle.exe -ErrorAction SilentlyContinue) {
  Split-Path (Get-Command candle.exe).Source
} else {
  Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'WiX Toolset v3.14/bin'
}
$candle = Join-Path $wixBin 'candle.exe'
$light = Join-Path $wixBin 'light.exe'
if (-not (Test-Path $candle) -or -not (Test-Path $light)) { throw 'WiX Toolset 3.14 candle.exe/light.exe are required' }

$escapedName = [System.Security.SecurityElement]::Escape($appName)
$wixUiVariables = ''
if ((Test-Path $dialogBmp) -and (Test-Path $bannerBmp)) {
  $wixUiVariables = @'
    <WixVariable Id="WixUIDialogBmp" Value="$(var.DialogBmp)" />
    <WixVariable Id="WixUIBannerBmp" Value="$(var.BannerBmp)" />
'@
}

$wxs = @'
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="@@APP_NAME@@" Language="1033" Version="@@APP_VERSION@@" Manufacturer="VOID Engine" UpgradeCode="@@UPGRADE_CODE@@">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perUser" Platform="x64" />
    <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <Icon Id="ProductIcon" SourceFile="$(var.IconPath)" />
    <Property Id="ARPPRODUCTICON" Value="ProductIcon" />
    <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
@@WIX_UI_VARS@@
    <UIRef Id="WixUI_InstallDir" />
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="INSTALLFOLDER" Name="$(var.AppSlug)" />
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Directory Id="StartMenuDir" Name="@@APP_NAME@@" />
      </Directory>
      <Directory Id="DesktopFolder" Name="Desktop" />
    </Directory>
    <Feature Id="MainFeature" Title="@@APP_NAME@@" Level="1">
      <ComponentRef Id="ApplicationExeComponent" />
      <ComponentRef Id="ApplicationConfigComponent" />
      <ComponentRef Id="StartMenuCleanup" />
      <ComponentRef Id="DesktopShortcutComponent" />
    </Feature>
    <DirectoryRef Id="INSTALLFOLDER">
      <Component Id="ApplicationExeComponent" Guid="@@EXE_GUID@@" Win64="yes">
        <File Id="ApplicationExeFile" Source="$(var.BinaryPath)" Name="$(var.AppSlug).exe" />
        <RegistryValue Root="HKCU" Key="Software\VOID Engine\$(var.AppSlug)" Name="Executable" Type="integer" Value="1" KeyPath="yes" />
      </Component>
      <Component Id="ApplicationConfigComponent" Guid="@@CONFIG_GUID@@" Win64="yes">
        <File Id="ApplicationConfigFile" Source="$(var.ConfigPath)" Name="$(var.AppSlug).json" />
        <RegistryValue Root="HKCU" Key="Software\VOID Engine\$(var.AppSlug)" Name="Config" Type="integer" Value="1" KeyPath="yes" />
        <RemoveFolder Id="RemoveInstallFolder" Directory="INSTALLFOLDER" On="uninstall" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="StartMenuDir">
      <Component Id="StartMenuCleanup" Guid="*" Win64="yes">
        <Shortcut Id="ApplicationShortcut" Name="@@APP_NAME@@" Target="[INSTALLFOLDER]$(var.AppSlug).exe" WorkingDirectory="INSTALLFOLDER" Icon="ProductIcon" IconIndex="0" />
        <RemoveFolder Id="RemoveStartMenuFolder" On="uninstall" />
        <RegistryValue Root="HKCU" Key="Software\VOID Engine\$(var.AppSlug)" Name="Installed" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="DesktopFolder">
      <Component Id="DesktopShortcutComponent" Guid="*" Win64="yes">
        <Shortcut Id="DesktopShortcut" Name="@@APP_NAME@@" Target="[INSTALLFOLDER]$(var.AppSlug).exe" WorkingDirectory="INSTALLFOLDER" Icon="ProductIcon" IconIndex="0" />
        <RegistryValue Root="HKCU" Key="Software\VOID Engine\$(var.AppSlug)" Name="DesktopShortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
  </Product>
</Wix>
'@
$wxs = $wxs.Replace('@@APP_NAME@@', $escapedName).Replace('@@APP_VERSION@@', $wixVersion).Replace('@@UPGRADE_CODE@@', $upgradeCode).Replace('@@EXE_GUID@@', $exeComponentGuid).Replace('@@CONFIG_GUID@@', $configComponentGuid).Replace('@@WIX_UI_VARS@@', $wixUiVariables)
$wxsPath = Join-Path $PWD 'wix/main.wxs'
New-Item -ItemType Directory -Force (Split-Path $wxsPath) | Out-Null
Set-Content -LiteralPath $wxsPath -Value $wxs -Encoding utf8

$wixobj = Join-Path $PWD 'output/main.wixobj'
$msiPath = Join-Path $PWD "output/$appSlug.msi"
$candleArgs = @('-nologo', '-ext', 'WixUIExtension', "-dAppSlug=$appSlug", "-dBinaryPath=$binaryPath", "-dConfigPath=$configPath", "-dIconPath=$iconPath", "-dDialogBmp=$dialogBmp", "-dBannerBmp=$bannerBmp", '-out', $wixobj, $wxsPath)
& $candle @candleArgs
if ($LASTEXITCODE -ne 0) { throw "WiX candle failed with exit code $LASTEXITCODE" }
& $light -nologo -sval -ext WixUIExtension -out $msiPath $wixobj
if ($LASTEXITCODE -ne 0) { throw "WiX light failed with exit code $LASTEXITCODE" }
Write-Host "Packaged $appName as $msiPath"
